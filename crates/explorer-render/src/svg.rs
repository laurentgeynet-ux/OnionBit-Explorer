// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! `<svg>` **inline** (R-9) : le sous-arbre DOM étranger est
//! re-sérialisé en source SVG autonome puis rasterisé via
//! `usvg`+`resvg` — même machine que le `<img src=*.svg>`
//! d'étape 45, même bornes. Arbitrage documenté dans la
//! roadmap : les display items dédiés (parser `d`, paints,
//! gradients…) auraient dupliqué un moteur SVG entier ; la
//! sérialisation réutilise le parseur conforme déjà
//! embarqué, pour un coût borné par [`SVG_MAX_SOURCE`] et
//! `max_px`.
//!
//! Limites assumées : rendu **statique** (SMIL et scripts
//! SVG ignorés), `<foreignObject>` refusé par `usvg` → zone
//! vide dégradée ; le bitmap est produit à la taille
//! naturelle (bornée `max_px`) puis mis à l'échelle nearest
//! au blit — le re-raster à la taille exacte×DPI est un
//! chantier ultérieur (dirty-rects R-7 le permet déjà).

use std::fmt::Write as _;
use std::sync::Arc;

use explorer_dom::{Dom, Namespace, NodeData, NodeId};

use crate::image::ImageTable;

/// Borne mémoire sur la source SVG re-sérialisée : au-delà le
/// sous-arbre est déclaré invalide (rendu vide) — un DOM
/// hostile peut produire un arbre énorme ; `usvg` borne déjà
/// le *temps* de parse, cette borne couvre la *taille*.
pub const SVG_MAX_SOURCE: usize = 4 * 1024 * 1024;

/// Rasterise les éléments `<svg>` du sous-arbre `root` —
/// `NodeId` → bitmap, à brancher dans [`crate::PaintImages`]
/// (champ `svgs`). La taille de chaque bitmap est sa taille
/// naturelle (`width`/`height`/`viewBox`), bornée à `max_px`
/// pixels — même budget que [`crate::decode_image`].
/// Les sous-arbres invalides sont ignorés (dégradation : la
/// boîte reste remplacée, image absente → fond/bordure
/// peints, comme un `<img>` cassé).
#[must_use]
pub fn collect_svg(dom: &Dom, root: NodeId, max_px: u32) -> ImageTable {
    let mut out = ImageTable::new();
    let mut stack = vec![root];
    while let Some(n) = stack.pop() {
        if let Ok(NodeData::Element(_)) = dom.data(n) {
            if is_svg_root(dom, n) {
                let mut src = String::new();
                serialize_into(dom, n, &mut src);
                // `<use href="#x">`/`url(#x)` cross-îlots (issue
                // cosmétique synthèse — logo reddit aplati) :
                // la cible vit souvent dans un autre îlot
                // (`<symbol>` en début de document) — sans elle
                // usvg rend l'usage vide. On l'injecte dans
                // `<defs>` avant `</svg>` pour la résolution.
                inject_external_refs(dom, root, n, &mut src);
                if src.len() <= SVG_MAX_SOURCE {
                    if let Some(img) = crate::image::decode_svg_raster(src.as_bytes(), max_px) {
                        out.insert(n, Arc::new(img));
                    }
                }
                continue; // sous-arbre SVG : atomique, jamais de HTML dedans
            }
            if let Ok(children) = dom.children(n) {
                stack.extend(children.iter().rev());
            }
        }
    }
    out
}

/// `true` si `n` est l'élément `<svg>` dans le namespace SVG
/// (la racine d'un îlot étranger — les `svg` imbriqués sont
/// rasterisés *par* leur racine, pas collectés seuls).
fn is_svg_root(dom: &Dom, n: NodeId) -> bool {
    dom.tag_name(n).is_ok_and(|t| t == "svg")
        && dom.namespace(n).is_ok_and(|ns| ns == Namespace::Svg)
}

/// Tailles intrinsèques des `<svg>` du sous-arbre, pour
/// `build_box_tree_sized` : attributs `width`/`height` (px
/// ou nombre nu ; `%` sans repère → ignoré), sinon aspect du
/// `viewBox`, sinon défaut CSS des remplacés **300×150**.
/// Les `svg` imbriqués ne sont pas listés (atomiques dans
/// leur racine).
#[must_use]
pub fn svg_sizes(dom: &Dom, root: NodeId) -> std::collections::HashMap<NodeId, (f32, f32)> {
    let mut out = std::collections::HashMap::new();
    let mut stack = vec![root];
    while let Some(n) = stack.pop() {
        if let Ok(NodeData::Element(_)) = dom.data(n) {
            if is_svg_root(dom, n) {
                out.insert(n, svg_intrinsic(dom, n));
                continue;
            }
            if let Ok(children) = dom.children(n) {
                stack.extend(children.iter().rev());
            }
        }
    }
    out
}

/// Taille intrinsèque d'un `<svg>` : attrs > viewBox > 300×150.
fn svg_intrinsic(dom: &Dom, n: NodeId) -> (f32, f32) {
    let attr_px = |name: &str| {
        dom.attribute(n, name)
            .ok()
            .flatten()
            .and_then(|v| parse_svg_length(&v))
    };
    let (w, h) = (attr_px("width"), attr_px("height"));
    let vb = dom
        .attribute(n, "viewBox")
        .ok()
        .flatten()
        .and_then(|v| parse_viewbox(&v));
    match (w, h, vb) {
        (Some(w), Some(h), _) => (w, h),
        (Some(w), None, Some((_, _, vw, vh))) if vw > 0.0 => (w, w * vh / vw),
        (None, Some(h), Some((_, _, vw, vh))) if vh > 0.0 => (h * vw / vh, h),
        (None, None, Some((_, _, vw, vh))) if vw > 0.0 && vh > 0.0 => (vw, vh),
        (Some(w), None, _) => (w, 150.0),
        (None, Some(h), _) => (300.0, h),
        _ => (300.0, 150.0),
    }
}

/// Longueur SVG `42`/`42px` — `%` refusé (pas de repère à
/// ce stade), borné aux valeurs finies positives.
fn parse_svg_length(v: &str) -> Option<f32> {
    let t = v.trim();
    if t.ends_with('%') {
        return None;
    }
    t.trim_end_matches("px")
        .parse::<f32>()
        .ok()
        .filter(|f| f.is_finite() && *f > 0.0)
}

/// `viewBox="minX minY w h"` — séparateurs espace/virgule.
fn parse_viewbox(v: &str) -> Option<(f32, f32, f32, f32)> {
    let nums: Vec<f32> = v
        .split([',', ' ', '\t'])
        .filter(|s| !s.is_empty())
        .filter_map(|s| s.parse().ok())
        .collect();
    (nums.len() == 4).then(|| (nums[0], nums[1], nums[2], nums[3]))
}

/// Sérialise le sous-arbre `node` en source SVG — un seul
/// document autonome : la racine reçoit `xmlns` si absente.
/// Échappement XML des textes et attributs ; commentaires
/// ignorés (non significatifs pour le rendu).
fn serialize_into(dom: &Dom, node: NodeId, out: &mut String) {
    match dom.data(node) {
        Ok(NodeData::Element(e)) => {
            out.push('<');
            out.push_str(&e.tag_name);
            let mut has_xmlns = false;
            for a in &e.attributes {
                let _ = write!(out, " {}=\"", a.name);
                escape_xml(&a.value, out, true);
                out.push('"');
                has_xmlns |= a.name == "xmlns";
            }
            if !has_xmlns && e.namespace == Namespace::Svg && e.tag_name == "svg" {
                out.push_str(" xmlns=\"http://www.w3.org/2000/svg\"");
            }
            let children = dom.children(node).unwrap_or_default();
            if children.is_empty() {
                out.push_str("/>");
            } else {
                out.push('>');
                for c in children {
                    // Arête ∝ profondeur DOM (R-24).
                    explorer_common::deep(|| serialize_into(dom, c, out));
                }
                let _ = write!(out, "</{}>", e.tag_name);
            }
        }
        Ok(NodeData::Text(t)) => escape_xml(t, out, false),
        _ => {}
    }
}

/// Échappement XML — `<`/`&` obligatoires, `>` et `"` en
/// attribut, `'` optionnel (guillemets doubles utilisés).
fn escape_xml(s: &str, out: &mut String, attr: bool) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' if attr => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
}

/// Références `#id`/`url(#id)` d'un îlot résolues depuis le
/// document entier (cross-îlot — les `<symbol>` partagés
/// vivent souvent dans un `<svg>` caché ailleurs). Chaque
/// cible trouvée hors de l'îlot est sérialisée dans un
/// `<defs>` injecté avant `</svg>` ; les refs introuvables
/// ou auto-référencées sont ignorées (rendu dégradé usvg).
fn inject_external_refs(dom: &Dom, doc_root: NodeId, svg: NodeId, src: &mut String) {
    let mut needed = Vec::<String>::new();
    let mut defined = Vec::<String>::new();
    let mut stack = vec![svg];
    while let Some(n) = stack.pop() {
        let Ok(e) = dom.element(n) else { continue };
        for a in &e.attributes {
            if a.name == "id" {
                defined.push(a.value.clone());
            } else if let Some(id) = a.value.strip_prefix('#') {
                if matches!(a.name.as_str(), "href" | "xlink:href") {
                    needed.push(id.to_string());
                }
            } else if let Some(i) = a.value.find("url(#") {
                let rest = &a.value[i + 5..];
                if let Some(end) = rest.find(')') {
                    needed.push(rest[..end].to_string());
                }
            }
        }
        if let Ok(ch) = dom.children(n) {
            stack.extend(ch.iter().rev());
        }
    }
    let mut defs = String::new();
    for id in &needed {
        if defined.contains(id) {
            continue;
        }
        if let Some(target) = find_by_id(dom, doc_root, id) {
            serialize_into(dom, target, &mut defs);
        }
    }
    if defs.is_empty() {
        return;
    }
    let close = "</svg>".to_string();
    if let Some(i) = src.rfind(&close) {
        let defs_tag = format!("<defs>{defs}</defs>");
        src.insert_str(i, &defs_tag);
    }
}

/// Premier élément portant `id` sous `root` (BFS).
fn find_by_id(dom: &Dom, root: NodeId, id: &str) -> Option<NodeId> {
    let mut queue = vec![root];
    while let Some(n) = queue.pop() {
        if let Ok(e) = dom.element(n) {
            if e.attributes.iter().any(|a| a.name == "id" && a.value == id) {
                return Some(n);
            }
            if let Ok(ch) = dom.children(n) {
                queue.extend(ch.iter().rev());
            }
        }
    }
    None
}
