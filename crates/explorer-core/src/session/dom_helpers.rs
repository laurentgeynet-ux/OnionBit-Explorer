// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Helpers DOM : hit-test, chemins d'éléments, titre, champs
//! éditables, collectes de sous-ressources et d'ancres.
use std::collections::HashMap;

use explorer_dom::{Dom, DomError, NodeId};
use explorer_layout::{LayoutBox, Rect};
use explorer_net::http::FetchCredentials;
use explorer_net::url::Url;
use explorer_render::ImageTable;

use super::{CoreError, SandboxFlags};

/// Hit-test profond : la boîte la plus imbriquée contenant
/// `(x, y)` **viewport** gagne — renvoie son `NodeId` source
/// (`None` pour les boîtes anonymes → on remonte au parent
/// peint le plus proche via les ancêtres DOM côté appelant).
/// `scroll_y` est filé pour que les boîtes `position:fixed`
/// (coords viewport) restent atteignables après scroll (i#18).
pub(crate) fn hit_test(bx: &LayoutBox, x: f32, y: f32, scroll_y: f32) -> Option<NodeId> {
    // R-19 : l'élagage sur le border-box tuait les zones dont
    // un enfant déborde le parent (lite.ddg). Le test vit en
    // amont : `explorer_layout::hit_test_scrolled` élague sur
    // l'union du sous-arbre (paint bounds) — même contrat.
    explorer_layout::hit_test_scrolled(bx, x, y, scroll_y)
}

/// Chemin `(tag, index parmi les frères éléments de même tag)`
/// de `node` (ou de son plus proche ancêtre élément) depuis la
/// racine — rejoué côté miroir JS pour localiser la cible d'un
/// événement (les deux arbres peuvent diverger sur les
/// wrappers implicites, voir `mirror_node` d'`explorer-js`).
pub(crate) fn element_path(dom: &Dom, node: NodeId) -> Option<Vec<(String, u32)>> {
    let mut cur = node;
    while dom.element(cur).is_err() {
        cur = dom.parent(cur).ok()??;
    }
    let mut path = Vec::new();
    while let Some(p) = dom.parent(cur).ok().flatten() {
        let tag = dom.tag_name(cur).ok()?.to_string();
        let k = dom
            .children(p)
            .ok()?
            .iter()
            .take_while(|&&c| c != cur)
            .filter(|&&c| dom.tag_name(c).is_ok_and(|t| t == tag))
            .count();
        path.push((tag, u32::try_from(k).ok()?));
        cur = p;
    }
    path.reverse();
    Some(path)
}

/// Marche l'arbre de layout **commis** et remplit la table de
/// géométrie publiée au JS (issue #19, JS-8) : `element_path`
/// du nœud DOM → union de ses boîtes (≙ le rect union des
/// fragments de `getBoundingClientRect` spec). `fixed` se
/// propage aux descendants — leurs rects sont en coords
/// **viewport** (le layout pose `position:fixed` contre le
/// viewport, i#18) ; les boîtes anonymes (`node: None`)
/// n'ont pas de path mais leurs enfants sont marchés.
/// Appelée à chaque commit paint : jamais sur arbre en
/// mutation.
pub(crate) fn collect_geometry(
    dom: &Dom,
    bx: &LayoutBox,
    fixed: bool,
    out: &mut explorer_js::GeometryMap,
) {
    let fixed = fixed || bx.position == explorer_layout::Position::Fixed;
    if let Some(n) = bx.node {
        if let Some(path) = element_path(dom, n) {
            let e = out.rects.entry(path).or_default();
            e.rect = e.rect.union(explorer_js::GeoRect {
                x: bx.rect.x,
                y: bx.rect.y,
                w: bx.rect.width,
                h: bx.rect.height,
            });
            e.fixed |= fixed;
        }
    }
    for c in &bx.children {
        collect_geometry(dom, c, fixed, out);
    }
}

/// Champ éditable le plus proche du nœud (lui-même compris) :
/// `<input>` à type texte ou `<textarea>`. `None` sinon.
pub(crate) fn editable_field(dom: &Dom, mut node: NodeId) -> Option<NodeId> {
    /// Types `input` acceptant du texte libre (absence de
    /// `type` incluse — valeur par défaut HTML).
    const TEXT_TYPES: &[&str] = &[
        "", "text", "search", "url", "password", "email", "number", "tel",
    ];
    loop {
        match dom.tag_name(node) {
            Ok("input") => {
                let ty = dom
                    .attribute(node, "type")
                    .ok()
                    .flatten()
                    .unwrap_or_default();
                return TEXT_TYPES.contains(&ty.as_str()).then_some(node);
            }
            Ok("textarea") => return Some(node),
            _ => {}
        }
        node = match dom.parent(node) {
            Ok(Some(p)) => p,
            _ => return None,
        };
    }
}

/// Valeur textuelle du champ : attribut `value` pour
/// `<input>`, contenu texte pour `<textarea>`.
pub(crate) fn field_value(dom: &Dom, field: NodeId) -> String {
    if dom.tag_name(field).is_ok_and(|t| t == "textarea") {
        dom.text_content(field).unwrap_or_default()
    } else {
        dom.attribute(field, "value")
            .ok()
            .flatten()
            .unwrap_or_default()
    }
}

/// Applique la touche au contenu courant : `Backspace` retire
/// le dernier caractère (frontière UTF-8), un `key` à un seul
/// `char` s'insère à la fin (pas de curseur interne au MVP) ;
/// le reste (Shift, Ctrl…) laisse le texte tel quel.
pub(crate) fn apply_key(cur: &str, key: &str) -> String {
    if key == "Backspace" {
        let mut s = cur.to_string();
        s.pop();
        return s;
    }
    let mut chars = key.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) => {
            let mut s = cur.to_string();
            s.push(c);
            s
        }
        _ => cur.to_string(),
    }
}

/// Édite le champ : `input` → attribut `value`, `textarea` →
/// texte enfant unique (les enfants texte surnuméraires —
/// cas pathologique — sont vidés).
pub(crate) fn edit_field(dom: &mut Dom, field: NodeId, key: &str) -> Result<(), DomError> {
    let next = apply_key(&field_value(dom, field), key);
    if dom.tag_name(field).is_ok_and(|t| t == "textarea") {
        let texts: Vec<NodeId> = dom
            .children(field)?
            .into_iter()
            .filter(|&c| dom.text(c).is_ok())
            .collect();
        if let Some((&first, rest)) = texts.split_first() {
            dom.set_text(first, &next)?;
            for &n in rest {
                dom.set_text(n, "")?;
            }
        } else {
            let t = dom.create_text(&next)?;
            dom.append_child(field, t)?;
        }
    } else {
        dom.set_attribute(field, "value", &next)?;
    }
    Ok(())
}

/// Plus proche ancêtre `<a href>` (le nœud lui-même
/// compris) — `None` si le point n'est pas dans un lien.
/// L'élément est rendu (pas seulement le `href`) : le
/// dispatch lit `target`/`rel` dessus (étape 131).
pub(crate) fn ancestor_link(dom: &Dom, mut node: NodeId) -> Option<NodeId> {
    loop {
        if dom.tag_name(node).is_ok_and(|t| t == "a")
            && matches!(dom.attribute(node, "href"), Ok(Some(_)))
        {
            return Some(node);
        }
        node = match dom.parent(node) {
            Ok(Some(p)) => p,
            _ => return None,
        };
    }
}

/// Un nom de cible (`target`/`formtarget`/`base target`)
/// désigne-t-il un contexte neuf ? — `_self`/`_parent`/
/// `_top` et vide = contexte courant ; `_blank` ou un nom
/// arbitraire = nouveau contexte (étape 131, CAP-150).
pub(crate) fn target_name_new_context(name: &str) -> bool {
    let t = name.trim();
    !t.is_empty()
        && !matches!(
            t.to_ascii_lowercase().as_str(),
            "_self" | "_parent" | "_top" | ""
        )
}

/// Nom de cible **effectif** d'un élément liant (`<a>`,
/// `<form>`) — `target` propre sinon `<base target>`,
/// `""` si aucun (CAP-159 : le drain en a besoin pour la
/// réutilisation des contextes nommés — le booléen
/// `target_new_context` seul perd l'information).
pub(crate) fn resolved_target_name(dom: &Dom, el: NodeId, base_target: Option<&str>) -> String {
    match dom.attribute(el, "target") {
        Ok(Some(t)) => t,
        _ => base_target.map_or_else(String::new, str::to_string),
    }
}

/// `<base>` du document (HTML §4.8.3, étape 135) —
/// `(base_url résolue, base target)` : le **premier**
/// `<base href>` en ordre document fixe l'URL de base de
/// résolution (liens, formulaires, sous-ressources,
/// `fetch` relatif) — résolue contre `doc_url`, ignorée
/// si absente ou irrésolvable (`None` → l'URL du document
/// reste la base). Le premier `<base target>` fixe la
/// cible de navigation par défaut. Les `<base>`
/// suivants sont sans effet (spec).
pub(crate) fn document_base(dom: &Dom, doc_url: &Url) -> (Option<Url>, Option<String>) {
    let mut seen_href = false;
    let mut seen_target = false;
    let mut base_url = None;
    let mut base_target = None;
    let mut queue = vec![dom.document()];
    while let Some(n) = queue.pop() {
        if dom.tag_name(n).is_ok_and(|t| t == "base") {
            if !seen_href {
                // Premier `<base href>` en ordre document —
                // irrésolvable = repli sur l'URL du document,
                // les `<base>` suivants sont sans effet (spec).
                if let Some(href) = dom.attribute(n, "href").ok().flatten() {
                    seen_href = true;
                    base_url = Url::resolve(doc_url, href.trim()).ok();
                }
            }
            if !seen_target {
                if let Some(t) = dom.attribute(n, "target").ok().flatten() {
                    seen_target = true;
                    base_target = Some(t);
                }
            }
            if seen_href && seen_target {
                break;
            }
        }
        if let Ok(ch) = dom.children(n) {
            queue.extend(ch.iter().rev());
        }
    }
    (base_url, base_target)
}

/// `rel` contient `noreferrer` — suppression du
/// `Referer` sur la navigation émise (et `noopener`
/// implicite, sans objet tant que `window.opener` n'a
/// pas de pont JS — étape 130).
pub(crate) fn rel_no_referrer(dom: &Dom, el: NodeId) -> bool {
    dom.attribute(el, "rel").ok().flatten().is_some_and(|r| {
        r.split_whitespace()
            .any(|t| t.eq_ignore_ascii_case("noreferrer"))
    })
}

/// `referrerpolicy=` d'un élément liant (`<a>`, `<form>`,
/// `<iframe>` — étape 132) — `None` si absent ou valeur
/// non reconnue (le défaut document s'applique alors).
/// `rel=noreferrer` reste traité à part (`no_referer`,
/// étape 131) — il l'emporte, spec « noreferrer →
/// no-referrer ».
pub(crate) fn element_referrer_policy(
    dom: &Dom,
    el: NodeId,
) -> Option<explorer_net::http::ReferrerPolicy> {
    dom.attribute(el, "referrerpolicy")
        .ok()
        .flatten()
        .and_then(|v| explorer_net::http::ReferrerPolicy::parse(&v))
}

/// Politique `Referer` du document via `<meta
/// name=referrer>` (étape 132) — la **dernière valide**
/// en ordre document gagne (spec : les méta sont
/// traitées à leur rencontre ; aucune reconnue → `None`).
pub(crate) fn meta_referrer_policy(dom: &Dom) -> Option<explorer_net::http::ReferrerPolicy> {
    let mut out = None;
    let mut queue = vec![dom.document()];
    while let Some(n) = queue.pop() {
        if dom.tag_name(n).is_ok_and(|t| t == "meta")
            && dom
                .attribute(n, "name")
                .ok()
                .flatten()
                .is_some_and(|v| v.trim().eq_ignore_ascii_case("referrer"))
        {
            if let Some(p) = dom
                .attribute(n, "content")
                .ok()
                .flatten()
                .and_then(|v| explorer_net::http::ReferrerPolicy::parse(&v))
            {
                out = Some(p);
            }
        }
        if let Ok(ch) = dom.children(n) {
            queue.extend(ch.iter().rev());
        }
    }
    out
}

/// `<meta http-equiv=refresh>` du document (CAP-143) —
/// la **première valide** en ordre document gagne (spec :
/// les méta sont traitées à leur rencontre — une méta
/// refresh syntaxiquement morte n'occupe pas le slot).
/// Retourne `(délai secondes, URL brute)` ; `None` dans
/// l'URL = rechargement du document courant.
pub(crate) fn meta_refresh(dom: &Dom) -> Option<(f64, Option<String>)> {
    let mut queue = vec![dom.document()];
    while let Some(n) = queue.pop() {
        if dom.tag_name(n).is_ok_and(|t| t == "meta")
            && dom
                .attribute(n, "http-equiv")
                .ok()
                .flatten()
                .is_some_and(|v| v.trim().eq_ignore_ascii_case("refresh"))
        {
            if let Some(r) = dom
                .attribute(n, "content")
                .ok()
                .flatten()
                .and_then(|v| parse_refresh_content(&v))
            {
                return Some(r);
            }
        }
        if let Ok(ch) = dom.children(n) {
            queue.extend(ch.iter().rev());
        }
    }
    None
}

/// Grammaire `content` d'une méta refresh **et de
/// l'en-tête `Refresh:`** (spec HTML « pragmas »,
/// tolérante — CAP-143/144) : `N[;][url=]U` — nombre en
/// tête (int ou décimal, un chiffre exigé), séparateur
/// `;` ou `,`, token `url=` optionnel, URL verbatim
/// (quotes appariées retirées). Sans séparateur, un
/// suffixe non vide rend la valeur inexploitable —
/// jamais d'URL collée au délai. `None` = ignorée.
pub(crate) fn parse_refresh_content(content: &str) -> Option<(f64, Option<String>)> {
    let s = content.trim_start();
    let digits = s.len()
        - s.trim_start_matches(|c: char| c.is_ascii_digit() || c == '.')
            .len();
    if digits == 0 || !s[..digits].bytes().any(|b| b.is_ascii_digit()) {
        return None;
    }
    let secs: f64 = s[..digits].parse().ok().filter(|v: &f64| v.is_finite())?;
    let rest = s[digits..].trim_start();
    if rest.is_empty() {
        return Some((secs, None));
    }
    let rest = match rest.as_bytes()[0] {
        b';' | b',' => rest[1..].trim_start(),
        _ => return None,
    };
    // `url=` optionnel : le token ne compte que suivi
    // d'un `=` (sinon `url…` est le début de l'URL).
    let rest = if rest.len() > 3 && rest[..3].eq_ignore_ascii_case("url") {
        let after = rest[3..].trim_start();
        after.strip_prefix('=').map_or(rest, |r| r.trim_start())
    } else {
        rest
    };
    let url = rest.trim();
    if url.is_empty() {
        return Some((secs, None));
    }
    let q = url.as_bytes()[0] as char;
    let url = if q == '"' || q == '\'' {
        url.strip_prefix(q)
            .and_then(|r| r.strip_suffix(q))
            .map_or(url, |r| r.trim())
    } else {
        url
    };
    if url.is_empty() {
        Some((secs, None))
    } else {
        Some((secs, Some(url.to_string())))
    }
}

/// Extrait le texte du premier `<title>` du document —
/// `None` si absent ou vide.
pub(crate) fn extract_title(dom: &Dom) -> Option<String> {
    let mut queue = vec![dom.document()];
    while let Some(n) = queue.pop() {
        if dom.tag_name(n).is_ok_and(|t| t == "title") {
            if let Ok(t) = dom.text_content(n) {
                let t = t.trim().to_string();
                if !t.is_empty() {
                    return Some(t);
                }
            }
        }
        if let Ok(ch) = dom.children(n) {
            queue.extend(ch.iter().rev());
        }
    }
    None
}

/// Contenu de chaque `<style>` du document, ordre document
/// — une entrée par élément (étape 104 : chaque feuille est
/// parsée isolément — une accolade non fermée ne peut plus
/// absorber la feuille suivante, cf. R-68).
/// `<meta http-equiv=Content-Security-Policy>` du document
/// — valeurs `content` en ordre document (CAP-145) : les
/// politiques méta se cumulent avec celles de l'en-tête
/// (chacune doit autoriser — la plus restrictive réduit).
pub(crate) fn collect_meta_csp(dom: &Dom) -> Vec<String> {
    let mut out = Vec::new();
    let mut queue = vec![dom.document()];
    while let Some(n) = queue.pop() {
        if dom.tag_name(n).is_ok_and(|t| t == "meta")
            && dom
                .attribute(n, "http-equiv")
                .ok()
                .flatten()
                .is_some_and(|v| v.trim().eq_ignore_ascii_case("content-security-policy"))
        {
            if let Some(c) = dom.attribute(n, "content").ok().flatten() {
                out.push(c);
            }
        }
        if let Ok(ch) = dom.children(n) {
            queue.extend(ch.iter().rev());
        }
    }
    out
}

/// Corps des `<style>` du document, ordre document —
/// sous CSP `style-src` (CAP-145) : un `<style>` inline
/// n'est retenu que si `'unsafe-inline'`/un `nonce`
/// d'élément l'autorise (`csp` vide = aucune politique →
/// tout admis, comme avant).
pub(crate) fn collect_style_sheets(dom: &Dom, csp: &[explorer_net::csp::CspPolicy]) -> Vec<String> {
    let mut sheets = Vec::new();
    let mut queue = vec![dom.document()];
    while let Some(n) = queue.pop() {
        if dom.tag_name(n).is_ok_and(|t| t == "style") {
            let nonce = dom.attribute(n, "nonce").ok().flatten();
            if !explorer_net::csp::csp_inline_allowed(
                csp,
                explorer_net::csp::InlineKind::Style,
                nonce.as_deref(),
            ) {
                continue;
            }
            if let Ok(t) = dom.text_content(n) {
                sheets.push(t);
            }
        }
        if let Ok(ch) = dom.children(n) {
            queue.extend(ch.iter().rev());
        }
    }
    sheets
}

/// Attribut `crossorigin` (CAP-167, spec HTML « CORS settings
/// attribute ») → mode credentials : absent → `None`
/// (comportement `no-cors` historique) ; `""`/`anonymous`/
/// valeur inconnue → `SameOrigin` (anonymous — credentials
/// same-origin seulement, spec) ; `use-credentials` →
/// `Include`. La valeur est ASCII case-insensitive.
pub(crate) fn crossorigin_attr(dom: &Dom, n: NodeId) -> Option<FetchCredentials> {
    dom.attribute(n, "crossorigin").ok().flatten().map(|v| {
        match v.trim().to_ascii_lowercase().as_str() {
            "use-credentials" => FetchCredentials::Include,
            _ => FetchCredentials::SameOrigin,
        }
    })
}

/// Élément stylesheet collecté : `(href, media, integrity,
/// crossorigin, el_ord)` — `media` sert au filtre viewport de
/// `plan_subresources` ; `integrity` (SRI, CAP-163) et
/// `crossorigin` (mode CORS, CAP-167) voyagent jusqu'au
/// commit de la réponse. `el_ord` (CAP-185) = ordinal de
/// l'élément parmi **tous** les `<link>` du document
/// (y compris les non-collectés — autre `rel`, `href` vide)
/// : identifie l'élément miroir pour `load`/`error`.
pub(crate) type SheetLink = (
    String,
    Option<String>,
    Option<String>,
    Option<FetchCredentials>,
    u32,
);

/// Feuille résolue prête au fetch : `(url absolue,
/// integrity, crossorigin, el_ord)` — `media` a déjà servi
/// au filtre ; `el_ord` (CAP-185) identifie l'élément miroir
/// pour la livraison `load`/`error` de fin de fetch.
pub(crate) type SheetFetch = (Url, Option<String>, Option<FetchCredentials>, u32);

/// `SheetLink` des `<link rel=stylesheet>` dans l'ordre —
/// `rel` case-insensitive à tokens (la spec admet
/// `rel="stylesheet alternate"` et cie) ; `href` vide sauté.
/// Tous les `<link>` — y compris les non-collectés — sont
/// numérotés pour `el_ord` (CAP-185, même convention que
/// `collect_scripts`).
pub(crate) fn collect_stylesheet_links(dom: &Dom) -> Vec<SheetLink> {
    let mut out = Vec::new();
    let mut els = 0u32;
    let mut queue = vec![dom.document()];
    while let Some(n) = queue.pop() {
        if dom.tag_name(n).is_ok_and(|t| t == "link") {
            let el_ord = els;
            els += 1;
            let rel = dom
                .attribute(n, "rel")
                .ok()
                .flatten()
                .unwrap_or_default()
                .to_ascii_lowercase();
            if rel.split_whitespace().any(|w| w == "stylesheet") {
                if let Ok(Some(href)) = dom.attribute(n, "href") {
                    if !href.is_empty() {
                        let media = dom.attribute(n, "media").ok().flatten();
                        let integrity = dom.attribute(n, "integrity").ok().flatten();
                        out.push((href, media, integrity, crossorigin_attr(dom, n), el_ord));
                    }
                }
            }
        }
        if let Ok(ch) = dom.children(n) {
            queue.extend(ch.iter().rev());
        }
    }
    out
}

/// Descripteur d'un candidat `srcset` (CAP-157) — `X` =
/// densité de pixels (`2x`), `W` = largeur source en px
/// (`640w`). Les descripteurs `h` et les candidats sans
/// descripteur reconnaissable sont bornés : `h` saute le
/// candidat, « rien » vaut `1x` par défaut (spec « parsing
/// a srcset attribute »).
enum SrcsetDesc {
    /// `Nx` — ratio par rapport à la densité du device.
    X(f32),
    /// `Nw` — largeur intrinsèque de la source en px.
    W(u32),
}

/// Token descripteur terminal d'un candidat `srcset` —
/// `None` = pas un descripteur (le token fait partie de
/// l'URL ou le candidat est nu → `1x` par défaut).
fn srcset_desc(tok: &str) -> Option<SrcsetDesc> {
    let (num, kind) = tok.split_at(tok.len().checked_sub(1)?);
    match kind {
        "x" | "X" => num
            .parse::<f32>()
            .ok()
            .filter(|v| *v > 0.0)
            .map(SrcsetDesc::X),
        "w" | "W" => num
            .parse::<u32>()
            .ok()
            .filter(|v| *v > 0)
            .map(SrcsetDesc::W),
        _ => None,
    }
}

/// Candidat `srcset` choisi (CAP-157) — sélection bornée de
/// la spec « select an image source » : descripteurs `x`
/// → plus petit `x ≥ dpr`, sinon le plus grand ; `w` → plus
/// petit `w ≥ eff_w×dpr`, sinon le plus grand (`eff_w` =
/// largeur rendue en CSS px, `sizes` résolu ou viewport).
/// Mélange `x`/`w`, candidat `data:`, `h` ou liste vide →
/// `None` (repli `src` — les candidats `data:` sont bornés :
/// le peintre inline `collect_data_images` indexe par `src`,
/// les ignorer évite la désynchronisation fetch/peinture).
pub(crate) fn pick_srcset(srcset: &str, eff_w: f32, dpr: f32) -> Option<String> {
    let mut cands: Vec<(String, SrcsetDesc)> = Vec::new();
    // Queue d'une URL `data:` découpée sur sa virgule
    // interne — le segment suivant est sa fin (+desc), pas
    // un candidat.
    let mut data_tail = false;
    for part in srcset.split(',') {
        let p = part.trim();
        if p.is_empty() {
            continue;
        }
        if data_tail {
            data_tail = false;
            continue;
        }
        let (url, desc) = match p.rsplit_once(char::is_whitespace) {
            Some((u, d)) if !u.trim().is_empty() => {
                (u.trim(), srcset_desc(d).unwrap_or(SrcsetDesc::X(1.0)))
            }
            _ => (p, SrcsetDesc::X(1.0)),
        };
        if url.starts_with("data:") {
            data_tail = true;
            continue;
        }
        cands.push((url.to_string(), desc));
    }
    if cands.is_empty() {
        return None;
    }
    let w_of = |d: &SrcsetDesc| match d {
        SrcsetDesc::W(w) => Some(*w),
        SrcsetDesc::X(_) => None,
    };
    let x_of = |d: &SrcsetDesc| match d {
        SrcsetDesc::X(x) => Some(*x),
        SrcsetDesc::W(_) => None,
    };
    if cands.iter().all(|(_, d)| w_of(d).is_some()) {
        let need = f64::from(eff_w * dpr);
        cands
            .iter()
            .filter(|(_, d)| w_of(d).is_some_and(|w| f64::from(w) >= need))
            .min_by_key(|(_, d)| w_of(d).unwrap_or(u32::MAX))
            .or_else(|| cands.iter().max_by_key(|(_, d)| w_of(d).unwrap_or(0)))
            .map(|(u, _)| u.clone())
    } else {
        if !cands.iter().all(|(_, d)| x_of(d).is_some()) {
            return None; // mélange x/w — invalide, repli `src`.
        }
        let by_x = |a: &(String, SrcsetDesc), b: &(String, SrcsetDesc)| {
            x_of(&a.1)
                .unwrap_or(f32::MAX)
                .partial_cmp(&x_of(&b.1).unwrap_or(f32::MAX))
                .unwrap_or(std::cmp::Ordering::Equal)
        };
        cands
            .iter()
            .filter(|(_, d)| x_of(d).is_some_and(|x| x >= dpr))
            .min_by(|a, b| by_x(a, b))
            .or_else(|| cands.iter().max_by(|a, b| by_x(a, b)))
            .map(|(u, _)| u.clone())
    }
}

/// `sizes` résolu en CSS px (CAP-157, borné) : la liste est
/// parcourue et le **premier** terme sans condition média
/// (sans parenthèse ouvrante) s'applique — `Npx` → `N`,
/// `Nvw` → `N/100×viewport`, `Nem`/`Nrem` → `N×16` ; les
/// termes conditionnels `(...) Npx` sont sautés (éval
/// média hors portée — le terme final non conditionnel de
/// la spec est ainsi trouvé). Défaut : `viewport` (100vw).
pub(crate) fn resolve_sizes(sizes: Option<String>, vp_w: f32) -> f32 {
    let Some(sizes) = sizes else {
        return vp_w;
    };
    for term in sizes.split(',') {
        let t = term.trim();
        if t.contains('(') {
            continue;
        }
        if let Some(v) = t
            .strip_suffix("px")
            .and_then(|n| n.trim().parse::<f32>().ok())
        {
            return v;
        }
        if let Some(v) = t
            .strip_suffix("vw")
            .and_then(|n| n.trim().parse::<f32>().ok())
        {
            return vp_w * (v / 100.0);
        }
        let v: Option<f32> = t
            .strip_suffix("rem")
            .or_else(|| t.strip_suffix("em"))
            .and_then(|n| n.trim().parse().ok())
            .map(|n: f32| n * 16.0);
        if let Some(v) = v {
            return v;
        }
    }
    vp_w
}

/// `(NodeId, url)` des `<img>` et des affiches `<video
/// poster>` du document dans l'ordre — `src`/`poster`
/// absent ou vide est sauté (étape 58 : l'affiche suit le
/// canal image, se peint dans la boîte vidéo). CAP-157 :
/// `<img srcset>` présent et résoluble **prime** sur `src`
/// — `img_w`/`dpr` = largeur viewport CSS px et densité de
/// pixels de la session (choix `w`/`x` de `pick_srcset`,
/// `sizes` résolu par `resolve_sizes`).
pub(crate) fn collect_images(dom: &Dom, img_w: f32, dpr: f32) -> Vec<(NodeId, String)> {
    let mut out = Vec::new();
    let mut queue = vec![dom.document()];
    while let Some(n) = queue.pop() {
        match dom.tag_name(n) {
            Ok("img") => {
                let srcset = dom
                    .attribute(n, "srcset")
                    .ok()
                    .flatten()
                    .filter(|s| !s.is_empty());
                let picked = srcset.and_then(|ss| {
                    let eff_w = resolve_sizes(dom.attribute(n, "sizes").ok().flatten(), img_w);
                    pick_srcset(&ss, eff_w, dpr)
                });
                let src = picked.or_else(|| {
                    dom.attribute(n, "src")
                        .ok()
                        .flatten()
                        .filter(|s| !s.is_empty())
                });
                if let Some(src) = src {
                    out.push((n, src));
                }
            }
            Ok("video") => {
                if let Ok(Some(src)) = dom.attribute(n, "poster") {
                    if !src.is_empty() {
                        out.push((n, src));
                    }
                }
            }
            _ => {}
        }
        if let Ok(ch) = dom.children(n) {
            queue.extend(ch.iter().rev());
        }
    }
    out
}

/// Source d'un sous-contexte `<iframe>` (CAP-142) :
/// `srcdoc` est un document **inline** (prioritaire sur
/// `src` quand présent, même vide — spec HTML « process
/// the iframe attributes » : l'attribut charge
/// `about:srcdoc` sans aucune requête) ; `Src` est la
/// valeur brute à résoudre/fetcher.
pub(crate) enum IframeKind {
    /// `srcdoc="…"` — HTML du document enfant, verbatim.
    Srcdoc(String),
    /// `src="…"` brut non vide — ignoré si `srcdoc` est
    /// présent (la collecte garantit l'exclusivité).
    Src(String),
}

/// Drapeaux `sandbox` d'un `<iframe>` depuis l'attribut
/// (CAP-149) : séparés par espaces, jetons insensibles à
/// la casse, inconnus ignorés (spec HTML « parsing a
/// sandboxing directive » — un `sandbox` **vide** restreint
/// tout). `None` = attribut absent = contexte plein droit.
pub(crate) fn parse_sandbox(attr: Option<&str>) -> Option<SandboxFlags> {
    let attr = attr?;
    let has = |tok: &str| {
        attr.split_ascii_whitespace()
            .any(|t| t.eq_ignore_ascii_case(tok))
    };
    Some(SandboxFlags {
        scripts: has("allow-scripts"),
        same_origin: has("allow-same-origin"),
        popups: has("allow-popups"),
        forms: has("allow-forms"),
        top_navigation: has("allow-top-navigation"),
    })
}

/// `<iframe>` du document dans l'ordre — `(kind,
/// referrerpolicy, sandbox)` par élément (étape 128 + 132 +
/// CAP-142 + CAP-149 : sans `src` ni `srcdoc` l'élément
/// saute — sa boîte reste un cadre vide ; `referrerpolicy`
/// gouverne le `Referer` de la requête de sous-contexte,
/// `None` = politique du document ; `sandbox` restreint le
/// contexte enfant — `None` = plein droit). La résolution/
/// dédup/mixed-content vit dans `plan_subresources`.
pub(crate) fn collect_iframes(
    dom: &Dom,
) -> Vec<(
    IframeKind,
    Option<explorer_net::http::ReferrerPolicy>,
    Option<SandboxFlags>,
)> {
    let mut out = Vec::new();
    let mut queue = vec![dom.document()];
    while let Some(n) = queue.pop() {
        if dom.tag_name(n).is_ok_and(|t| t == "iframe") {
            let srcdoc = dom.attribute(n, "srcdoc").ok().flatten();
            let src = dom
                .attribute(n, "src")
                .ok()
                .flatten()
                .filter(|s| !s.is_empty());
            let kind = match (srcdoc, src) {
                (Some(h), _) => Some(IframeKind::Srcdoc(h)),
                (None, Some(s)) => Some(IframeKind::Src(s)),
                (None, None) => None,
            };
            if let Some(k) = kind {
                out.push((
                    k,
                    element_referrer_policy(dom, n),
                    parse_sandbox(dom.attribute(n, "sandbox").ok().flatten().as_deref()),
                ));
            }
        }
        if let Ok(ch) = dom.children(n) {
            queue.extend(ch.iter().rev());
        }
    }
    out
}

/// Extrait tous les `url(…)` d'une valeur CSS — même forme
/// en déclaration de feuille et en valeur calculée (la
/// cascade conserve `url("…")` tel quel). Guillemets
/// optionnels, séquences `\` résolues. Doublon assumé de
/// `dataurl::url_arg` (privée à `explorer-render`) : le
/// fetch NAV des fonds HTTP (étape 98) indexe ses corps par
/// le texte brut extrait ici — le même extracteur relu à
/// l'insertion garantit la coïncidence des clés.
pub(crate) fn css_url_args(value: &str, out: &mut Vec<String>) {
    let b = value.as_bytes();
    let mut i = 0usize;
    while i + 4 <= b.len() {
        if !b[i..i + 4].eq_ignore_ascii_case(b"url(") {
            i += 1;
            continue;
        }
        let mut j = i + 4;
        while j < b.len() && b[j] != b')' {
            j += 1;
        }
        let raw = value[i + 4..j]
            .trim()
            .trim_matches(|c| c == '"' || c == '\'');
        if !raw.is_empty() {
            out.push(css_unescape(raw));
        }
        i = j.max(i + 4);
    }
}

/// Premier `url(…)` de la valeur — le raster ne peint qu'un
/// fond par boîte (`backgrounds: NodeId → image`).
pub(crate) fn css_url_arg(value: &str) -> Option<String> {
    let mut urls = Vec::with_capacity(1);
    css_url_args(value, &mut urls);
    urls.into_iter().next()
}

/// Échappements CSS chaîne/url : `\`+autre → l'autre char ;
/// `\`+1-6 hexdigits (+espaces absorbés) → codepoint si
/// valide. Doublon du `css_unescape` privé de
/// `explorer-render::dataurl` — mêmes règles, clés alignées.
fn css_unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        let mut hex = String::new();
        while hex.len() < 6 && it.peek().is_some_and(char::is_ascii_hexdigit) {
            hex.push(it.next().unwrap_or_default());
        }
        if hex.is_empty() {
            if let Some(p) = it.next() {
                out.push(p);
            }
        } else {
            if it.peek().is_some_and(char::is_ascii_whitespace) {
                it.next();
            }
            if let Some(ch) = u32::from_str_radix(&hex, 16)
                .ok()
                .and_then(char::from_u32)
                .filter(|ch| *ch != '\0')
            {
                out.push(ch);
            }
        }
    }
    out
}

/// Taille préférée `(w, h)` par élément remplacé décodé
/// (`<img>`, `<object>`, affiche `<video poster>` — R-34) :
/// attribut `width`/`height` sain s'il existe, sinon
/// dimension intrinsèque — chaque axe indépendamment
/// (§10.3.2).
#[allow(clippy::cast_precision_loss)] // pixels u32 → f32 : < 2^24.
pub(crate) fn image_sizes(dom: &Dom, images: &ImageTable) -> HashMap<NodeId, (f32, f32)> {
    let px_attr = |n: NodeId, name: &str| {
        dom.attribute(n, name)
            .ok()
            .flatten()
            .and_then(|v| v.trim().trim_end_matches("px").parse::<f32>().ok())
            .filter(|v| *v > 0.0)
    };
    images
        .iter()
        .map(|(&n, img)| {
            let w = px_attr(n, "width").unwrap_or(img.width as f32);
            let h = px_attr(n, "height").unwrap_or(img.height as f32);
            (n, (w, h))
        })
        .collect()
}

/// `<form>` ancêtre (le nœud lui-même compris) — `None` hors
/// formulaire. L'association `form=` hors arbre est hors
/// scope MVP (ADR-0014).
pub(crate) fn enclosing_form(dom: &Dom, mut node: NodeId) -> Option<NodeId> {
    loop {
        if dom.tag_name(node).is_ok_and(|t| t == "form") {
            return Some(node);
        }
        node = dom.parent(node).ok()??;
    }
}

/// Bouton par défaut du formulaire (HTML §4.10.22.2) : le
/// premier contrôle submit en ordre d'arbre —
/// `input[type=submit|image]` ou `<button>` non `disabled`
/// (type absent ≙ `submit`).
pub(crate) fn default_button(dom: &Dom, form: NodeId) -> Option<NodeId> {
    let mut queue = vec![form];
    while let Some(n) = queue.pop() {
        let tag = dom.tag_name(n).unwrap_or("");
        if n != form
            && dom.element(n).is_ok()
            && dom.attribute(n, "disabled").ok().flatten().is_none()
        {
            let is_submit = match tag {
                "input" => matches!(attr_lower(dom, n, "type").as_str(), "submit" | "image"),
                "button" => !matches!(attr_lower(dom, n, "type").as_str(), "button" | "reset"),
                _ => false,
            };
            if is_submit {
                return Some(n);
            }
        }
        if n == form || tag != "form" {
            if let Ok(ch) = dom.children(n) {
                queue.extend(ch.iter().rev());
            }
        }
    }
    None
}

pub(crate) fn dom_path_node(dom: &Dom, path: &[(String, u32)]) -> Option<NodeId> {
    let mut cur = dom.document();
    for (tag, k) in path {
        let mut seen = 0u32;
        let mut next = None;
        for c in dom.children(cur).ok()? {
            if dom.tag_name(c).is_ok_and(|t| t == *tag) {
                if seen == *k {
                    next = Some(c);
                    break;
                }
                seen += 1;
            }
        }
        cur = next?;
    }
    Some(cur)
}

/// Diagnostic d'un chemin mort (M-H1 — jonglerie focus
/// google) : niveau du premier segment `(tag, index)`
/// irrésolu + les tags des enfants présents à ce niveau —
/// dit *où* une mutation a déplacé la cible, pas seulement
/// qu'elle est morte. `None` = le chemin résout.
pub(crate) fn dom_path_dead_end(dom: &Dom, path: &[(String, u32)]) -> Option<(usize, Vec<String>)> {
    let mut cur = dom.document();
    for (i, (tag, k)) in path.iter().enumerate() {
        let children = dom.children(cur).ok()?;
        let mut seen = 0u32;
        let mut next = None;
        for &c in &children {
            if dom.tag_name(c).is_ok_and(|t| t == *tag) {
                if seen == *k {
                    next = Some(c);
                    break;
                }
                seen += 1;
            }
        }
        let Some(n) = next else {
            let tags = children
                .iter()
                .filter_map(|&c| dom.tag_name(c).ok().map(str::to_string))
                .collect();
            return Some((i, tags));
        };
        cur = n;
    }
    None
}

pub(crate) fn find_by_id(dom: &Dom, id: &str) -> Option<NodeId> {
    find_by_attr(dom, "id", id)
}

/// Premier élément dont l'attribut `attr` vaut `value`,
/// parcours préfixe — même itératif que `find_by_id`
/// historique. Sert au re-ancrage du focus par identité
/// (`id`/`name`) quand le chemin `(tag, index)` a été
/// déplacé par une mutation (étape 92).
pub(crate) fn find_by_attr(dom: &Dom, attr: &str, value: &str) -> Option<NodeId> {
    let mut queue = vec![dom.document()];
    while let Some(n) = queue.pop() {
        if dom.element(n).is_ok() && dom.attribute(n, attr).ok().flatten().as_deref() == Some(value)
        {
            return Some(n);
        }
        if let Ok(ch) = dom.children(n) {
            queue.extend(ch.iter().rev());
        }
    }
    None
}

/// Repli navigateur pour les fragments (étape 56d) :
/// `<a name="x">` quand aucun `id="x"` n'existe —
/// itératif, même parcours que `find_by_id`.
pub(crate) fn find_named_anchor(dom: &Dom, name: &str) -> Option<NodeId> {
    let mut queue = vec![dom.document()];
    while let Some(n) = queue.pop() {
        if dom.tag_name(n).is_ok_and(|t| t == "a")
            && dom.attribute(n, "name").ok().flatten().as_deref() == Some(name)
        {
            return Some(n);
        }
        if let Ok(ch) = dom.children(n) {
            queue.extend(ch.iter().rev());
        }
    }
    None
}

/// Hauteur `y` (haut) de la première boîte posée portant
/// `target` — `min` sur toutes les boîtes du nœud (un
/// élément peut en couvrir plusieurs — itératif, R-24).
pub(crate) fn box_y_of_node(root: &LayoutBox, target: NodeId) -> Option<f32> {
    let mut stack = vec![root];
    let mut best: Option<f32> = None;
    while let Some(bx) = stack.pop() {
        if bx.node == Some(target) {
            best = Some(best.map_or(bx.rect.y, |acc| acc.min(bx.rect.y)));
        }
        stack.extend(bx.children.iter());
    }
    best
}

/// Boîte posée de `node` — descente récursive de l'arbre de
/// layout (chaque `LayoutBox` garde son `NodeId` source).
pub(crate) fn box_rect(bx: &LayoutBox, node: NodeId) -> Option<Rect> {
    if bx.node == Some(node) {
        return Some(bx.rect);
    }
    bx.children.iter().find_map(|c| box_rect(c, node))
}

/// Attribut en minuscules, `""` si absent.
pub(crate) fn attr_lower(dom: &Dom, n: NodeId, attr: &str) -> String {
    attr_opt(dom, n, attr).unwrap_or_default().to_lowercase()
}

/// Attribut ou `None`.
pub(crate) fn attr_opt(dom: &Dom, n: NodeId, attr: &str) -> Option<String> {
    dom.attribute(n, attr).ok().flatten()
}

/// Éléments descendants portant `tag`, ordre document.
pub(crate) fn descendant_elements(dom: &Dom, root: NodeId, tag: &str) -> Vec<NodeId> {
    let mut out = Vec::new();
    let mut queue = vec![root];
    while let Some(n) = queue.pop() {
        if n != root && dom.tag_name(n).is_ok_and(|t| t == tag) {
            out.push(n);
        }
        if let Ok(ch) = dom.children(n) {
            queue.extend(ch.iter().rev());
        }
    }
    out
}

// -- Page d'erreur navigateur (étape 69) -------------------------------

/// Échappement minimal d'un texte injecté dans le document
/// d'erreur généré (URL, cause) — `&` `<` `>` suffisent, les
/// autres séquences sont inertes en contexte texte/attribut
/// double-quoté.
pub(crate) fn esc_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Document d'erreur **peint par le moteur** (étape 69) :
/// une nav qui échoue (404/5xx, connexion refusée, DNS,
/// document vide) produit une page lisible plutôt qu'un
/// vide — statut, URL tentée, cause. Markup minimal volontaire
/// (UA stylesheet + styles inline, zéro dépendance UI).
pub(crate) fn error_document(url: &Url, err: &CoreError) -> String {
    let cause = match err {
        CoreError::HttpStatus(s) => format!("Le serveur a répondu « HTTP {s} »."),
        CoreError::Fetch(m) => format!("Impossible de joindre le serveur ({m})."),
        CoreError::Url(m) => format!("Adresse invalide ({m})."),
        other => other.to_string(),
    };
    format!(
        "<!doctype html><html><head><title>Erreur de chargement</title></head>\
         <body style=\"font-family:sans-serif;margin:3em auto;max-width:34em\">\
         <h1>Page inaccessible</h1>\
         <p>La page <code>{}</code> n’a pas pu être chargée.</p>\
         <p><strong>{}</strong></p>\
         <p>Réessayez plus tard, ou revenez à la page précédente.</p>\
         </body></html>",
        esc_html(&url.to_string()),
        esc_html(&cause),
    )
}
