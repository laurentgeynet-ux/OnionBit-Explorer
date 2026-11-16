// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Image maps (`<map>`/`<area>` — tranche legacy du journal
//! inter-équipes 2026-11-15, point b) : un `<img
//! usemap="#nom">` expose des zones cliquables portées par
//! les `<area>` du `<map>` nommé. Hit-map de session pure —
//! aucun changement de contrat PIPE : `area`/`coords` vivent
//! dans les attributs, la géométrie vient du boxtree posé.
//!
//! Sémantique (HTML « image map » + « hit testing ») :
//! - les `<area>` sont sondées dans l'ordre de l'arbre, la
//!   première contenant le point gagne ; `shape="default"`
//!   ne matche qu'en dernier recours (zone entière) ;
//! - `href` absent (ou `nohref`, attribut obsolète) = zone
//!   non sélectionnable : elle est **sautée**, elle ne
//!   bloque pas les suivantes ;
//! - `shape` absent ou invalide = `rect` ; `coords`
//!   tronqués = zone invalide (sautée, jamais de panic) ;
//! - les coords vivent dans l'espace de l'image
//!   **naturelle** : mises à l'échelle quand l'élément est
//!   rendu à une autre taille (intrinsèque inconnu =
//!   brut — le `sizes` porte déjà `width`/`height` attrs).

use explorer_dom::{Dom, NodeId};
use explorer_layout::LayoutBox;
use explorer_render::ImageTable;

use super::{attr_lower, attr_opt, box_rect};

/// `<img>` ancêtre du nœud touché porteur d'un `usemap` non
/// vide — premier `<img>` remonté, `None` s'il n'a pas
/// d'attribut (la map ne s'applique qu'à l'élément qui la
/// déclare). La spec HTML actuelle ne définit `usemap` que
/// sur `img` — `input[type=image]` (HTML4) soumet le
/// formulaire au clic, `object` rend son contenu de repli.
fn usemap_element(dom: &Dom, mut node: NodeId) -> Option<NodeId> {
    loop {
        if dom.tag_name(node).is_ok_and(|t| t == "img") {
            return attr_opt(dom, node, "usemap")
                .filter(|u| !u.is_empty())
                .map(|_| node);
        }
        node = dom.parent(node).ok()??;
    }
}

/// `<map>` désignée par le fragment `name` (sans `#`) —
/// l'attribut `name` prime (spec « hash-name reference »),
/// `id` accepté en repli historique.
fn map_element(dom: &Dom, name: &str) -> Option<NodeId> {
    let mut by_id = None;
    for n in dom.descendants(dom.document()).into_iter().flatten() {
        if !dom.tag_name(n).is_ok_and(|t| t == "map") {
            continue;
        }
        if attr_opt(dom, n, "name") == Some(name) {
            return Some(n);
        }
        if by_id.is_none() && attr_opt(dom, n, "id") == Some(name) {
            by_id = Some(n);
        }
    }
    by_id
}

/// Forme canonique d'une `<area>` — valeur absente ou
/// inintelligible = `rect` (missing/invalid state default
/// de la spec).
#[derive(Clone, Copy, PartialEq, Eq)]
enum AreaShape {
    Rect,
    Circle,
    Poly,
    Default,
}

fn area_shape(dom: &Dom, area: NodeId) -> AreaShape {
    match attr_lower(dom, area, "shape").as_str() {
        "circle" | "circ" => AreaShape::Circle,
        "poly" | "polygon" => AreaShape::Poly,
        "default" => AreaShape::Default,
        _ => AreaShape::Rect,
    }
}

/// `coords` splitté en floats (virgules — tolérant aux
/// espaces et aux restes non numériques, ignorés).
fn area_coords(dom: &Dom, area: NodeId) -> Vec<f32> {
    attr_opt(dom, area, "coords").map_or_else(Vec::new, |raw| {
        raw.split(',')
            .filter_map(|t| t.trim().parse::<f32>().ok())
            .collect()
    })
}

/// Point-in-polygon, lancer de rayon horizontal — pair
/// croisé = dedans (tolérant, jamais de panic sur <6
/// coords : filtré par l'appelant).
fn poly_contains(cs: &[f32], px: f32, py: f32) -> bool {
    let n = cs.len() / 2;
    let mut inside = false;
    let mut j = n - 1;
    for i in 0..n {
        let (xi, yi) = (cs[2 * i], cs[2 * i + 1]);
        let (xj, yj) = (cs[2 * j], cs[2 * j + 1]);
        if (yi > py) != (yj > py) && px < (xj - xi) * (py - yi) / (yj - yi) + xi {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// Le point `(px, py)` — espace coords de l'aire — est-il
/// couvert ? Bornes `rect` normalisées (ordre des coins
/// toléré), rayon négatif = rien.
fn shape_contains(shape: AreaShape, cs: &[f32], px: f32, py: f32) -> bool {
    match shape {
        AreaShape::Default => true,
        AreaShape::Rect if cs.len() >= 4 => {
            px >= cs[0].min(cs[2])
                && px <= cs[0].max(cs[2])
                && py >= cs[1].min(cs[3])
                && py <= cs[1].max(cs[3])
        }
        AreaShape::Circle if cs.len() >= 3 => {
            cs[2] >= 0.0
                && (px - cs[0]).mul_add(px - cs[0], (py - cs[1]) * (py - cs[1])) <= cs[2] * cs[2]
        }
        AreaShape::Poly if cs.len() >= 6 => poly_contains(cs, px, py),
        _ => false,
    }
}

/// `<area>` de la map de `usemap` couvrant le point
/// **document** `(doc_x, doc_y)` — le nœud retourné porte
/// `href` (zones non sélectionnables sautées en amont).
/// `None` = pas de porteur usemap, map absente, aire hors
/// point ou toutes non sélectionnables.
#[allow(clippy::cast_precision_loss)] // px naturels < 2^24 — f32 suffit.
pub(crate) fn area_hit(
    dom: &Dom,
    tree: &LayoutBox,
    images: &ImageTable,
    node: NodeId,
    doc_x: f32,
    doc_y: f32,
) -> Option<NodeId> {
    let el = usemap_element(dom, node)?;
    let name = attr_opt(dom, el, "usemap")?
        .strip_prefix('#')
        .filter(|n| !n.is_empty())?;
    let map = map_element(dom, name)?;
    let rect = box_rect(tree, el)?;
    // Espace naturel : le point doc relatif à la boîte est
    // remis à l'échelle de l'image source quand elle est
    // connue (élément agrandi/réduit par CSS ou attrs).
    let (rx, ry) = (doc_x - rect.x, doc_y - rect.y);
    let (px, py) = images.get(&el).map_or((rx, ry), |img| {
        let sx = if rect.width > 0.0 {
            img.width as f32 / rect.width
        } else {
            1.0
        };
        let sy = if rect.height > 0.0 {
            img.height as f32 / rect.height
        } else {
            1.0
        };
        (rx * sx, ry * sy)
    });
    let mut default_area = None;
    for area in dom.descendants(map).into_iter().flatten() {
        if !dom.tag_name(area).is_ok_and(|t| t == "area") {
            continue;
        }
        // Non sélectionnable : `href` absent ou `nohref`
        // obsolète — sautée (ne bloque pas la suivante).
        if attr_opt(dom, area, "href").is_none() || attr_opt(dom, area, "nohref").is_some() {
            continue;
        }
        let shape = area_shape(dom, area);
        if shape == AreaShape::Default {
            if default_area.is_none() {
                default_area = Some(area);
            }
            continue;
        }
        let cs = area_coords(dom, area);
        if shape_contains(shape, &cs, px, py) {
            return Some(area);
        }
    }
    // `default` : dernier recours, la zone entière.
    default_area
}
