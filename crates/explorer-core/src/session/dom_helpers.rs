// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Helpers DOM : hit-test, chemins d'éléments, titre, champs
//! éditables, collectes de sous-ressources et d'ancres.
use std::collections::HashMap;

use explorer_dom::{Dom, DomError, NodeId};
use explorer_layout::{LayoutBox, Rect};
use explorer_net::url::Url;
use explorer_render::ImageTable;

use super::CoreError;

/// Hit-test profond : la boîte la plus imbriquée contenant
/// `(x, y)` document gagne — renvoie son `NodeId` source
/// (`None` pour les boîtes anonymes → on remonte au parent
/// peint le plus proche via les ancêtres DOM côté appelant).
pub(crate) fn hit_test(bx: &LayoutBox, x: f32, y: f32) -> Option<NodeId> {
    // R-19 : l'élagage sur le border-box tuait les zones dont
    // un enfant déborde le parent (lite.ddg). Le test vit en
    // amont : `explorer_layout::hit_test` élague sur l'union
    // du sous-arbre (paint bounds) — même contrat de retour.
    explorer_layout::hit_test(bx, x, y)
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

/// Cible de navigation d'un élément liant (`<a>`,
/// `<form>`) — HTML « rules for choosing a navigable » :
/// `true` = **nouveau** contexte de navigation
/// (`target=_blank`, ou nom non réservé — les contextes
/// nommés ne sont pas suivis : chaque dispatch ouvre un
/// onglet frais), `false` = contexte courant (`_self`,
/// `_parent`, `_top`, attribut absent/vide). La casse
/// est insensible (spec). `base_target` = `<base target>`
/// du document (étape 135) — consulté quand l'élément
/// n'a pas de `target` propre.
pub(crate) fn target_new_context(dom: &Dom, el: NodeId, base_target: Option<&str>) -> bool {
    let t = match dom.attribute(el, "target") {
        Ok(Some(t)) => t,
        _ => match base_target {
            Some(b) => b.to_string(),
            None => return false,
        },
    };
    let t = t.trim();
    !t.is_empty()
        && !matches!(
            t.to_ascii_lowercase().as_str(),
            "_self" | "_parent" | "_top" | ""
        )
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

/// Grammaire `content` d'une méta refresh (spec HTML
/// « pragmas », tolérante) : `N[;][url=]U` — nombre en
/// tête (int ou décimal, un chiffre exigé), séparateur
/// `;` ou `,`, token `url=` optionnel, URL verbatim
/// (quotes appariées retirées). Sans séparateur, un
/// suffixe non vide rend la méta inexploitable — jamais
/// d'URL collée au délai. `None` = méta ignorée.
fn parse_refresh_content(content: &str) -> Option<(f64, Option<String>)> {
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
pub(crate) fn collect_style_sheets(dom: &Dom) -> Vec<String> {
    let mut sheets = Vec::new();
    let mut queue = vec![dom.document()];
    while let Some(n) = queue.pop() {
        if dom.tag_name(n).is_ok_and(|t| t == "style") {
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

/// `(href, media)` des `<link rel=stylesheet>` dans l'ordre
/// — `rel` case-insensitive à tokens (la spec admet
/// `rel="stylesheet alternate"` et cie) ; `href` vide sauté.
/// `media` est conservé pour le filtre viewport de
/// `plan_subresources` (étape 96 : un `<link media="print">`
/// ne consomme pas la borne `max_sheets`).
pub(crate) fn collect_stylesheet_links(dom: &Dom) -> Vec<(String, Option<String>)> {
    let mut out = Vec::new();
    let mut queue = vec![dom.document()];
    while let Some(n) = queue.pop() {
        if dom.tag_name(n).is_ok_and(|t| t == "link") {
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
                        out.push((href, media));
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

/// `(NodeId, src)` des `<img>` et des affiches `<video
/// poster>` du document dans l'ordre — `src`/`poster`
/// absent ou vide est sauté (étape 58 : l'affiche suit le
/// canal image, se peint dans la boîte vidéo).
pub(crate) fn collect_images(dom: &Dom) -> Vec<(NodeId, String)> {
    let mut out = Vec::new();
    let mut queue = vec![dom.document()];
    while let Some(n) = queue.pop() {
        let attr = match dom.tag_name(n) {
            Ok("img") => "src",
            Ok("video") => "poster",
            _ => "",
        };
        if !attr.is_empty() {
            if let Ok(Some(src)) = dom.attribute(n, attr) {
                if !src.is_empty() {
                    out.push((n, src));
                }
            }
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

/// `<iframe>` du document dans l'ordre — `(kind,
/// referrerpolicy)` par élément (étape 128 + 132 +
/// CAP-142 : sans `src` ni `srcdoc` l'élément saute —
/// sa boîte reste un cadre vide ; l'attribut gouverne le
/// `Referer` de la requête de sous-contexte, `None` =
/// politique du document). La résolution/dédup/mixed-
/// content vit dans `plan_subresources`.
pub(crate) fn collect_iframes(
    dom: &Dom,
) -> Vec<(IframeKind, Option<explorer_net::http::ReferrerPolicy>)> {
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
                out.push((k, element_referrer_policy(dom, n)));
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
