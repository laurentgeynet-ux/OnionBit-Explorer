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
pub(crate) fn hit_test(
    bx: &LayoutBox,
    x: f32,
    y: f32,
    scroll_y: f32,
    vp: (f32, f32),
) -> Option<NodeId> {
    // R-19 : l'élagage sur le border-box tuait les zones dont
    // un enfant déborde le parent (lite.ddg). Le test vit en
    // amont : `explorer_layout::hit_test_scrolled` élague sur
    // l'union du sous-arbre (paint bounds) — même contrat.
    // `vp` = scrollport — seuils sticky `bottom`/`right` (B-40).
    explorer_layout::hit_test_scrolled_vp(bx, x, y, scroll_y, vp)
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
            .children_iter(p)
            .ok()?
            .take_while(|&c| c != cur)
            .filter(|&c| dom.tag_name(c).is_ok_and(|t| t == tag))
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
            // JS-C97 : la position calculée sert d'
            // `offsetParent` aux descendants — la marche des
            // ancêtres vit côté miroir (`positioned` ≠ static).
            e.positioned |= bx.position != explorer_layout::Position::Static;
            // §B-28 : la valeur `position` calculée est
            // publiée — `getComputedStyle(el).position` la
            // sert quand l'inline ne dit rien (uMap/Leaflet
            // lisent `absolute` pour ne pas écraser le
            // stretch `top/bottom` d'un `relative`).
            e.position = match bx.position {
                explorer_layout::Position::Static => explorer_js::GeomPosition::Static,
                explorer_layout::Position::Relative => explorer_js::GeomPosition::Relative,
                explorer_layout::Position::Absolute => explorer_js::GeomPosition::Absolute,
                explorer_layout::Position::Fixed => explorer_js::GeomPosition::Fixed,
                explorer_layout::Position::Sticky => explorer_js::GeomPosition::Sticky,
            };
        }
    }
    for c in &bx.children {
        explorer_common::deep(|| collect_geometry(dom, c, fixed, out));
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
                    .attribute_str(node, "type")
                    .ok()
                    .flatten()
                    .unwrap_or_default();
                return TEXT_TYPES.contains(&ty).then_some(node);
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
        dom.attribute_str(field, "value")
            .ok()
            .flatten()
            .unwrap_or_default()
            .to_owned()
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
            .children_iter(field)?
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
            && matches!(dom.attribute_str(node, "href"), Ok(Some(_)))
        {
            return Some(node);
        }
        node = match dom.parent(node) {
            Ok(Some(p)) => p,
            _ => return None,
        };
    }
}

/// Plus proche ancêtre `<img src>` (le nœud lui-même
/// compris) — `None` si le point n'est pas sur une image
/// adressable (étape 176, « enregistrer l'image »).
pub(crate) fn ancestor_img(dom: &Dom, mut node: NodeId) -> Option<NodeId> {
    loop {
        if dom.tag_name(node).is_ok_and(|t| t == "img")
            && matches!(dom.attribute_str(node, "src"), Ok(Some(_)))
        {
            return Some(node);
        }
        node = match dom.parent(node) {
            Ok(Some(p)) => p,
            _ => return None,
        };
    }
}

/// Plus proche ancêtre `<video>`/`<audio>` (le nœud
/// lui-même compris) — `None` si le point tombe hors d'un
/// élément média (étape 186, arbitrage pointer : le nœud
/// topmost *est* média = vidéo nue ; un contrôle DOM du
/// site peint par-dessus rend un nœud non-média).
pub(crate) fn ancestor_media(dom: &Dom, mut node: NodeId) -> Option<NodeId> {
    loop {
        if dom
            .tag_name(node)
            .is_ok_and(|t| t == "video" || t == "audio")
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
    match dom.attribute_str(el, "target") {
        Ok(Some(t)) => t.to_owned(),
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
    for n in dom.descendants(dom.document()).into_iter().flatten() {
        if dom.tag_name(n).is_ok_and(|t| t == "base") {
            if !seen_href {
                // Premier `<base href>` en ordre document —
                // irrésolvable = repli sur l'URL du document,
                // les `<base>` suivants sont sans effet (spec).
                if let Some(href) = dom.attribute_str(n, "href").ok().flatten() {
                    seen_href = true;
                    base_url = Url::resolve(doc_url, href.trim()).ok();
                }
            }
            if !seen_target {
                if let Some(t) = dom.attribute_str(n, "target").ok().flatten() {
                    seen_target = true;
                    base_target = Some(t.to_owned());
                }
            }
            if seen_href && seen_target {
                break;
            }
        }
    }
    (base_url, base_target)
}

/// `rel` contient `noreferrer` — suppression du
/// `Referer` sur la navigation émise (et `noopener`
/// implicite, sans objet tant que `window.opener` n'a
/// pas de pont JS — étape 130).
pub(crate) fn rel_no_referrer(dom: &Dom, el: NodeId) -> bool {
    dom.attribute_str(el, "rel")
        .ok()
        .flatten()
        .is_some_and(|r| {
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
    dom.attribute_str(el, "referrerpolicy")
        .ok()
        .flatten()
        .and_then(explorer_net::http::ReferrerPolicy::parse)
}

/// Politique `Referer` du document via `<meta
/// name=referrer>` (étape 132) — la **dernière valide**
/// en ordre document gagne (spec : les méta sont
/// traitées à leur rencontre ; aucune reconnue → `None`).
pub(crate) fn meta_referrer_policy(dom: &Dom) -> Option<explorer_net::http::ReferrerPolicy> {
    let mut out = None;
    for n in dom.descendants(dom.document()).into_iter().flatten() {
        if dom.tag_name(n).is_ok_and(|t| t == "meta")
            && dom
                .attribute_str(n, "name")
                .ok()
                .flatten()
                .is_some_and(|v| v.trim().eq_ignore_ascii_case("referrer"))
        {
            if let Some(p) = dom
                .attribute_str(n, "content")
                .ok()
                .flatten()
                .and_then(explorer_net::http::ReferrerPolicy::parse)
            {
                out = Some(p);
            }
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
    for n in dom.descendants(dom.document()).into_iter().flatten() {
        if dom.tag_name(n).is_ok_and(|t| t == "meta")
            && dom
                .attribute_str(n, "http-equiv")
                .ok()
                .flatten()
                .is_some_and(|v| v.trim().eq_ignore_ascii_case("refresh"))
        {
            if let Some(r) = dom
                .attribute_str(n, "content")
                .ok()
                .flatten()
                .and_then(parse_refresh_content)
            {
                return Some(r);
            }
        }
    }
    None
}

/// `<meta http-equiv=refresh>` enfermée dans un
/// `<noscript>` (vol google SERP, CAP-155) : scripting
/// levé au parse, le contenu de `noscript` est rawtext —
/// la méta n'existe jamais comme élément et
/// [`meta_refresh`] la manque. Spec : ignorée tant que le
/// scripting tourne. Repli produit : quand la page est un
/// **fallback pur** — tous les enfants éléments du `body`
/// sont `noscript`/`script`, aucun texte non blanc hors
/// repli — le rawtext est re-parsé (scripting off) et sa
/// méta refresh honorée : le document n'a rien d'autre à
/// offrir, c'est le sens du repli. Une page riche avec un
/// `noscript` annexe garde la spec : `None`.
/// Re-parse le rawtext d'un `<noscript>` (scripting off)
/// et applique la grammaire `meta_refresh` habituelle.
fn noscript_refresh_of(dom: &Dom, n: NodeId) -> Option<(f64, Option<String>)> {
    let text = dom.text_content(n).ok()?;
    if text.trim().is_empty() {
        return None;
    }
    let mut tb = explorer_html::TreeBuilder::new();
    tb.push(&text);
    tb.finish();
    meta_refresh(&tb.into_dom())
}

/// `<meta http-equiv=refresh>` enfermée dans un
/// `<noscript>` (vol google SERP, CAP-155) : scripting
/// levé au parse, le contenu de `noscript` est rawtext —
/// la méta n'existe jamais comme élément et
/// [`meta_refresh`] la manque. Spec : ignorée tant que le
/// scripting tourne. Repli produit, deux cas honorés :
/// - **`noscript` dans le `head`** (vol 23 DDG `/html` :
///   le méta de repli no-JS du head est un redirect
///   d'entrée de gamme, indépendant du contenu du body) ;
/// - **fallback pur** — tous les enfants éléments du
///   `body` sont `noscript`/`script`, aucun texte non
///   blanc hors repli : le document n'a rien d'autre à
///   offrir.
///
/// Une page riche avec un `noscript` annexe **dans le
/// body** garde la spec : `None`.
pub(crate) fn noscript_meta_refresh(dom: &Dom) -> Option<(f64, Option<String>)> {
    // `head` et `body` en un seul balayage.
    let mut head = None;
    let mut body = None;
    for n in dom.descendants(dom.document()).into_iter().flatten() {
        match dom.tag_name(n) {
            Ok("head") => head = Some(n),
            Ok("body") => body = Some(n),
            _ => {}
        }
        if head.is_some() && body.is_some() {
            break;
        }
    }
    // Cas 1 — `noscript` descendant de `head`, toujours
    // éligible (pattern no-JS de DDG, indépendant du body).
    let mut noscripts = Vec::new();
    if let Some(h) = head {
        for n in dom.descendants(h).into_iter().flatten() {
            if dom.tag_name(n).is_ok_and(|t| t == "noscript") {
                noscripts.push(n);
            }
        }
    }
    for n in &noscripts {
        if let Some(r) = noscript_refresh_of(dom, *n) {
            return Some(r);
        }
    }
    // Cas 2 — fallback pur dans le `body`.
    let body = body?;
    let children = dom.children_iter(body).ok()?;
    let mut body_noscripts = Vec::new();
    for c in children {
        match dom.data(c).ok()? {
            explorer_dom::NodeData::Element(_) => match dom.tag_name(c).ok()? {
                "script" => {}
                "noscript" => body_noscripts.push(c),
                _ => return None, // contenu réel — spec.
            },
            // Texte non blanc hors repli → contenu réel.
            explorer_dom::NodeData::Text(t) if !dom.text_at(*t).trim().is_empty() => {
                return None;
            }
            _ => {}
        }
    }
    for n in body_noscripts {
        if let Some(r) = noscript_refresh_of(dom, n) {
            return Some(r);
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
    for n in dom.descendants(dom.document()).into_iter().flatten() {
        if dom.tag_name(n).is_ok_and(|t| t == "title") {
            if let Ok(t) = dom.text_content(n) {
                let t = t.trim().to_string();
                if !t.is_empty() {
                    return Some(t);
                }
            }
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
    for n in dom.descendants(dom.document()).into_iter().flatten() {
        if dom.tag_name(n).is_ok_and(|t| t == "meta")
            && dom
                .attribute_str(n, "http-equiv")
                .ok()
                .flatten()
                .is_some_and(|v| v.trim().eq_ignore_ascii_case("content-security-policy"))
        {
            if let Some(c) = dom.attribute_str(n, "content").ok().flatten() {
                out.push(c.to_owned());
            }
        }
    }
    out
}

/// Corps des `<style>` du document, ordre document —
/// sous CSP `style-src` (CAP-145) : un `<style>` inline
/// n'est retenu que si `'unsafe-inline'`/un `nonce`
/// d'élément l'autorise (`csp` vide = aucune politique →
/// tout admis, comme avant).
///
/// R-96 — chaque entrée est `(scope, css)` : `scope` = la
/// racine shadow propriétaire de la feuille (`None` =
/// document). La traversée descend dans les racines
/// attachées : un `<style>` en shadow est confiné à sa
/// racine (`add_scoped_css` côté moteur), jamais fusionné
/// dans la feuille document.
pub(crate) fn collect_style_sheets(
    dom: &Dom,
    csp: &[explorer_net::csp::CspPolicy],
) -> Vec<(Option<NodeId>, String)> {
    let mut sheets = Vec::new();
    let mut queue = vec![dom.document()];
    while let Some(n) = queue.pop() {
        if dom.tag_name(n).is_ok_and(|t| t == "style") {
            let nonce = dom.attribute_str(n, "nonce").ok().flatten();
            if explorer_net::csp::csp_inline_allowed(
                csp,
                explorer_net::csp::InlineKind::Style,
                nonce,
            ) {
                if let Ok(t) = dom.text_content(n) {
                    sheets.push((dom.containing_shadow_root(n), t));
                }
            }
        }
        if let Ok(ch) = dom.children(n) {
            queue.extend(ch.iter().rev());
        }
        // Flat tree : la racine shadow se visite à la
        // position de son hôte — avant les enfants light
        // (poussée après eux, donc dépilée avant).
        if let Some(root) = dom.shadow_root(n, true) {
            queue.push(root);
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
    dom.attribute_str(n, "crossorigin").ok().flatten().map(|v| {
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
    for n in dom.descendants(dom.document()).into_iter().flatten() {
        if dom.tag_name(n).is_ok_and(|t| t == "link") {
            let el_ord = els;
            els += 1;
            let rel = dom
                .attribute_str(n, "rel")
                .ok()
                .flatten()
                .unwrap_or_default()
                .to_ascii_lowercase();
            // Repli produit (vol 32) : `rel="preload"
            // as="style"` — le pattern `onload="this.rel =
            // 'stylesheet'"` meurt avec le JS ; la feuille
            // préchargée est traitée comme un stylesheet
            // (spec : un preload ne s'applique qu'au swap —
            // dégradé assumé, un `as=style` existe pour être
            // appliqué). `media=` continue de filtrer en plan.
            let is_sheet = rel.split_whitespace().any(|w| w == "stylesheet")
                || (rel.split_whitespace().any(|w| w == "preload")
                    && dom
                        .attribute_str(n, "as")
                        .ok()
                        .flatten()
                        .is_some_and(|a| a.trim().eq_ignore_ascii_case("style")));
            if is_sheet {
                if let Ok(Some(href)) = dom.attribute_str(n, "href") {
                    if !href.is_empty() {
                        let media = dom.attribute_str(n, "media").ok().flatten();
                        let integrity = dom.attribute_str(n, "integrity").ok().flatten();
                        out.push((
                            href.to_owned(),
                            media.map(str::to_owned),
                            integrity.map(str::to_owned),
                            crossorigin_attr(dom, n),
                            el_ord,
                        ));
                    }
                }
            }
        }
    }
    out
}

/// `href` bruts des `<link rel=modulepreload>` et
/// `<link rel=preload as=script|module>` dans l'ordre
/// document — sèment la frontier initiale de
/// `fetch_module_sources` (B-4 data.gouv : Nuxt importmap
/// déclare ~34 chunks que le graphe statique n'atteint pas
/// encore). Résolution/dédup/CSP en plan
/// (`plan_subresources`) ; un `preload` à autre `as` n'est
/// **pas** un module — jamais préfetché ici.
pub(crate) fn collect_module_preloads(dom: &Dom) -> Vec<String> {
    let mut out = Vec::new();
    for n in dom.descendants(dom.document()).into_iter().flatten() {
        if dom.tag_name(n).is_ok_and(|t| t == "link") {
            let rel = dom
                .attribute_str(n, "rel")
                .ok()
                .flatten()
                .unwrap_or_default()
                .to_ascii_lowercase();
            let is_mod_preload = rel.split_whitespace().any(|w| w == "modulepreload")
                || (rel.split_whitespace().any(|w| w == "preload")
                    && dom.attribute_str(n, "as").ok().flatten().is_some_and(|a| {
                        let a = a.trim();
                        a.eq_ignore_ascii_case("script") || a.eq_ignore_ascii_case("module")
                    }));
            if is_mod_preload {
                if let Ok(Some(href)) = dom.attribute_str(n, "href") {
                    if !href.is_empty() {
                        out.push(href.to_owned());
                    }
                }
            }
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
pub(crate) fn resolve_sizes(sizes: Option<&str>, vp_w: f32) -> f32 {
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
    let probe = tracing::enabled!(target: "media_probe", tracing::Level::INFO);
    let mut seen = 0usize;
    let mut novar = 0usize;
    let mut picture = 0usize;
    let mut source = 0usize;
    let mut novar_attrs: Vec<String> = Vec::new();
    let mut out = Vec::new();
    for n in dom.descendants(dom.document()).into_iter().flatten() {
        match dom.tag_name(n) {
            Ok("picture") => picture += 1,
            Ok("source") => source += 1,
            Ok("img") => {
                seen += 1;
                // `srcset` ET sa variante lazy `data-srcset`
                // (§B-20 — la valeur `url 1x, url2 2x` résolue
                // comme URL brute ne produisait rien : elle
                // passe par `pick_srcset` comme `srcset`, et
                // prime `src`/`data-src` de même préséance).
                let srcset_of = |attr: &str| {
                    dom.attribute_str(n, attr)
                        .ok()
                        .flatten()
                        .filter(|s| !s.is_empty())
                        .and_then(|ss| {
                            let eff_w =
                                resolve_sizes(dom.attribute_str(n, "sizes").ok().flatten(), img_w);
                            pick_srcset(ss, eff_w, dpr)
                        })
                };
                let src = srcset_of("srcset")
                    .or_else(|| {
                        dom.attribute_str(n, "src")
                            .ok()
                            .flatten()
                            .filter(|s| !s.is_empty())
                            .map(str::to_owned)
                    })
                    // Lazy-load (vol 13 lemonde/bbc : ~200
                    // `<img>` à `data-src`, ~0 % peints) : le
                    // deferred `loading="lazy"` est déjà
                    // fetché d'emblée — reste la variante
                    // « src déplacé en data-* » dont le JS
                    // bootstrap est souvent mort. Repli
                    // produit : les attributs lazy usuels
                    // servent de source, dans cet ordre —
                    // mieux que jamais peint.
                    .or_else(|| srcset_of("data-srcset"))
                    .or_else(|| {
                        const LAZY_SRCS: &[&str] =
                            &["data-src", "data-lazy-src", "data-original", "data-url"];
                        LAZY_SRCS.iter().find_map(|a| {
                            dom.attribute_str(n, a)
                                .ok()
                                .flatten()
                                .filter(|s| !s.is_empty())
                                .map(str::to_owned)
                        })
                    });
                if let Some(src) = src {
                    out.push((n, src.clone()));
                } else if probe {
                    novar += 1;
                    // §B-108 — census des attributs portés par
                    // un `<img>` sans variante connue : un lazy
                    // attr hors famille connue y apparaît nommé
                    // (`data-initial-src`, etc.). Borné 16.
                    if novar_attrs.len() < 16 {
                        if let Ok(attrs) = dom.attributes(n) {
                            for a in attrs.iter().take(12) {
                                let name = a.name.as_str();
                                if !novar_attrs.iter().any(|x| x == name) {
                                    novar_attrs.push(name.to_string());
                                }
                            }
                        }
                    }
                }
            }
            Ok("video") => {
                if let Ok(Some(src)) = dom.attribute_str(n, "poster") {
                    if !src.is_empty() {
                        out.push((n, src.to_owned()));
                    }
                }
            }
            _ => {}
        }
    }
    // Sonde §B-108 — `img.collect` : `<img>` vues vs
    // candidates livrées au fetch vs sans variante connue ;
    // `picture`/`source` recensent le gap `<picture>` (non
    // collecté) ; `img.attr` nomme les attrs des sans-
    // variante — un lazy attr inconnu s'y lit directement.
    if probe {
        tracing::info!(
            target: "media_probe",
            site = "img.collect",
            seen,
            cand = out.len(),
            novar,
            picture,
            source
        );
        if !novar_attrs.is_empty() {
            tracing::info!(
                target: "media_probe",
                site = "img.attr",
                names = ?novar_attrs
            );
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
    for n in dom.descendants(dom.document()).into_iter().flatten() {
        if dom.tag_name(n).is_ok_and(|t| t == "iframe") {
            let srcdoc = dom.attribute_str(n, "srcdoc").ok().flatten();
            let src = dom
                .attribute_str(n, "src")
                .ok()
                .flatten()
                .filter(|s| !s.is_empty());
            let kind = match (srcdoc, src) {
                (Some(h), _) => Some(IframeKind::Srcdoc(h.to_owned())),
                (None, Some(s)) => Some(IframeKind::Src(s.to_owned())),
                (None, None) => None,
            };
            if let Some(k) = kind {
                out.push((
                    k,
                    element_referrer_policy(dom, n),
                    parse_sandbox(dom.attribute_str(n, "sandbox").ok().flatten()),
                ));
            }
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
        dom.attribute_str(n, name)
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
            && dom.attribute_str(n, "disabled").ok().flatten().is_none()
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
            // `<form>` imbriqué non descendu (défensif —
            // le tree-builder ne le produit pas, mais le
            // garde-fou reste moins cher que le parcours).
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
        for c in dom.children_iter(cur).ok()? {
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

/// Premier élément dont l'attribut `attr` vaut `value`,
/// parcours préfixe — itératif. Pour `attr == "id"`,
/// préférer [`Dom::by_id`] (O(1) via `id_index`, AR-11).
/// Sert au re-ancrage du focus par identité (`name`)
/// quand le chemin `(tag, index)` a été déplacé par
/// une mutation (étape 92).
pub(crate) fn find_by_attr(dom: &Dom, attr: &str, value: &str) -> Option<NodeId> {
    dom.descendants(dom.document())
        .into_iter()
        .flatten()
        .find(|&n| {
            dom.element(n).is_ok() && dom.attribute_str(n, attr).ok().flatten() == Some(value)
        })
}

/// Repli navigateur pour les fragments (étape 56d) :
/// `<a name="x">` quand aucun `id="x"` n'existe —
/// itératif, même parcours que `find_by_id`.
pub(crate) fn find_named_anchor(dom: &Dom, name: &str) -> Option<NodeId> {
    dom.descendants(dom.document())
        .into_iter()
        .flatten()
        .find(|&n| {
            dom.tag_name(n).is_ok_and(|t| t == "a")
                && dom.attribute_str(n, "name").ok().flatten() == Some(name)
        })
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

/// Attribut ou `None` (emprunté à l'arène — `to_owned`
/// où une `String` est réellement requise).
pub(crate) fn attr_opt<'a>(dom: &'a Dom, n: NodeId, attr: &str) -> Option<&'a str> {
    dom.attribute_str(n, attr).ok().flatten()
}

/// Éléments descendants portant `tag`, ordre document.
pub(crate) fn descendant_elements(dom: &Dom, root: NodeId, tag: &str) -> Vec<NodeId> {
    let mut out = Vec::new();
    for n in dom.descendants(root).into_iter().flatten() {
        if dom.tag_name(n).is_ok_and(|t| t == tag) {
            out.push(n);
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
///
/// Les échecs TLS (PR-8, version réduite) ont droit à une page
/// **sécurité** distincte : détail du refus, **aucun
/// contournement proposé** — un `cert_decisions`/bypass
/// demandera sa propre étape (verifier rustls custom).
pub(crate) fn error_document(url: &Url, err: &CoreError) -> String {
    // `NetError::Tls` est stringifié en amont — son préfixe
    // d'affichage (`TLS vers …`, contrat net) discrimine le
    // bras sécurité sans plomber un variant dédié.
    if let CoreError::Fetch(m) = err {
        if let Some(detail) = m.strip_prefix("TLS vers") {
            return format!(
                "<!doctype html><html><head><title>Connexion non sécurisée</title></head>\
                 <body style=\"font-family:sans-serif;margin:3em auto;max-width:34em\">\
                 <h1>Connexion non sécurisée</h1>\
                 <p>La connexion vers <code>{}</code> a été interrompue : \
                 le certificat du serveur n'a pas pu être validé.</p>\
                 <p><strong>Détail :</strong> TLS{}</p>\
                 <p>Un attaquant pourrait tenter d'intercepter vos données. \
                 <strong>Aucun contournement n'est proposé</strong> — \
                 le certificat doit être corrigé côté serveur.</p>\
                 <p>Revenez à la page précédente.</p>\
                 </body></html>",
                esc_html(&url.to_string()),
                esc_html(detail),
            );
        }
    }
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
