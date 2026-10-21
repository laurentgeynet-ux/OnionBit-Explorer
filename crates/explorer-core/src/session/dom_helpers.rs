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

/// `href` du plus proche ancêtre `<a>` (le nœud lui-même
/// compris) — `None` si le point n'est pas dans un lien.
pub(crate) fn ancestor_href(dom: &Dom, mut node: NodeId) -> Option<String> {
    loop {
        if dom.tag_name(node).is_ok_and(|t| t == "a") {
            if let Ok(Some(href)) = dom.attribute(node, "href") {
                return Some(href);
            }
        }
        node = match dom.parent(node) {
            Ok(Some(p)) => p,
            _ => return None,
        };
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

/// Concatène le contenu des `<style>` du document —
/// identique au helper du CLI (dupliqué tant que le pipeline
/// sous-ressources n'existe pas, étape 12).
pub(crate) fn collect_style_css(dom: &Dom) -> String {
    let mut css = String::new();
    let mut queue = vec![dom.document()];
    while let Some(n) = queue.pop() {
        if dom.tag_name(n).is_ok_and(|t| t == "style") {
            if let Ok(t) = dom.text_content(n) {
                css.push_str(&t);
                css.push('\n');
            }
        }
        if let Ok(ch) = dom.children(n) {
            queue.extend(ch.iter().rev());
        }
    }
    css
}

/// `href` des `<link rel=stylesheet>` dans l'ordre — `rel`
/// case-insensitive à tokens (la spec admet `rel="stylesheet
/// alternate"` et cie) ; `href` vide sauté.
pub(crate) fn collect_stylesheet_links(dom: &Dom) -> Vec<String> {
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
                        out.push(href);
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
