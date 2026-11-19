// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Dump géométrique normalisé (R-208) — `NodeId → rect` pour
//! le diff boîte-à-boîte contre la référence Chrome
//! (`ref_capture` → `getBoundingClientRect`).
//!
//! Un élément dont les boîtes se fragmentent (inline
//! multi-lignes, continuations absolues/flottantes) rapporte
//! l'**union** de tous ses fragments — la sémantique
//! `getBoundingClientRect` (union des `getClientRects`).
//! Les boîtes anonymes (`node: None`) sont ignorées : leur
//! géométrie est déjà couverte par les fragments de leurs
//! descendants porteurs de nœud.
//!
//! Les entrées couvrent tout nœud ayant produit de la
//! géométrie — éléments **et** nœuds texte (la distinction
//! relève du DOM chez le consommateur NAV, qui joint par
//! `NodeId`). Ordre : première apparition en pré-ordre de
//! boîtes — déterministe et stable pour le diff.

use std::collections::HashMap;

use explorer_dom::NodeId;

use crate::boxtree::{LayoutBox, Rect};
use crate::transform::Affine;

/// Une entrée `NodeId → rect` du dump.
#[derive(Debug, Clone, Copy)]
pub struct GeomEntry {
    /// Nœud DOM producteur de la géométrie.
    pub node: NodeId,
    /// Union des rects de tous ses fragments (border-box au
    /// sens `getBoundingClientRect`).
    pub rect: Rect,
    /// Nombre de fragments fusionnés (> 1 = élément split).
    pub fragments: u32,
}

/// Dump complet d'une frame posée.
#[derive(Debug, Clone)]
pub struct GeometryDump {
    /// Largeur du document posé (rect de la boîte racine).
    pub width: f32,
    /// Hauteur du document posé.
    pub height: f32,
    /// Entrées en ordre de première apparition (pré-ordre
    /// boîtes, puis fragments de ligne).
    pub entries: Vec<GeomEntry>,
}

/// Parcourt l'arbre posé et regroupe les rects par nœud
/// (pré-ordre pile, enfants empilés inversés — déterministe).
#[must_use]
pub fn geometry_dump(root: &LayoutBox) -> GeometryDump {
    let mut order: Vec<NodeId> = Vec::new();
    let mut map: HashMap<NodeId, (Rect, u32)> = HashMap::new();
    let mut add = |node: NodeId, rect: Rect| {
        if let Some((r, c)) = map.get_mut(&node) {
            *r = union(*r, rect);
            *c += 1;
        } else {
            map.insert(node, (rect, 1));
            order.push(node);
        }
    };
    // R-223 — coordonnées post-transform : chaque boîte
    // émet son rect composé par la chaîne des `Affine` de
    // ses ancêtres + la sienne (`gBCR` inclut les
    // transforms — les menus `vector-*` wikipedia vivaient
    // dans un ancêtre translaté : dx +1032 px sans ça).
    let mut stack = vec![(root, Affine::IDENTITY)];
    while let Some((b, acc)) = stack.pop() {
        let t_own = crate::transform::box_transform(b, &b.rect);
        let emit_acc = t_own.map_or(acc, |t| acc.post_concat(t));
        let xform = |r: Rect| {
            if emit_acc == Affine::IDENTITY {
                r
            } else {
                rect_of_points(&[
                    emit_acc.apply(r.x, r.y),
                    emit_acc.apply(r.x + r.width, r.y),
                    emit_acc.apply(r.x, r.y + r.height),
                    emit_acc.apply(r.x + r.width, r.y + r.height),
                ])
            }
        };
        if let Some(n) = b.node {
            // R-213 — une boîte en-ligne porte le rect de sa
            // LIGNE allouée ; `gBCR` rend la border box de
            // contenu (~`font-size`, mesuré Chrome : `<a>`
            // sous `line-height:30` → h=17, pas 30).
            add(n, xform(inline_content_rect(b)));
        }
        for line in &b.lines {
            for frag in &line.fragments {
                if let Some(n) = frag.node {
                    let r = if frag.atomic {
                        // Inline-block/remplacé : le rect EST
                        // la border box — pas de resserrement.
                        frag.rect
                    } else {
                        inline_content_rect_st(frag.rect, &frag.style)
                    };
                    add(n, xform(r));
                }
            }
        }
        stack.extend(b.children.iter().rev().map(|c| (c, emit_acc)));
    }
    GeometryDump {
        width: root.rect.width,
        height: root.rect.height,
        entries: order
            .into_iter()
            .map(|node| {
                let (rect, fragments) = map[&node];
                GeomEntry {
                    node,
                    rect,
                    fragments,
                }
            })
            .collect(),
    }
}

/// Rect « contenu » d'une boîte `Inline`/`Text` : la ligne
/// allouée (line-height) resserrée à ~`font-size` et
/// recentrée verticalement — approximation honnête de la
/// border box `gBCR` (±1 px mesuré : a=17 pour fs=16).
/// Si la font-size n'est pas résolue en px ou dépasse la
/// hauteur, le rect brut est conservé (jamais agrandi).
fn inline_content_rect(b: &LayoutBox) -> Rect {
    if matches!(
        b.kind,
        crate::boxtree::BoxKind::Inline | crate::boxtree::BoxKind::Text
    ) {
        inline_content_rect_st(b.rect, &b.style)
    } else {
        b.rect
    }
}

/// Idem [`inline_content_rect`] sur un rect + style (cas
/// fragment de ligne non-atomique).
fn inline_content_rect_st(rect: Rect, style: &explorer_css::ComputedStyle) -> Rect {
    let Some(fs) = style
        .get("font-size")
        .and_then(|v| v.trim_end_matches("px").trim().parse::<f32>().ok())
    else {
        return rect;
    };
    if fs >= rect.height || fs <= 0.0 {
        return rect;
    }
    Rect {
        x: rect.x,
        y: rect.y + (rect.height - fs) / 2.0,
        width: rect.width,
        height: fs,
    }
}

/// Bounding box des points transformés (`gBCR` = bbox du
/// quadrilatère transformé, R-223).
fn rect_of_points(pts: &[(f32, f32)]) -> Rect {
    let (mut x0, mut y0, mut x1, mut y1) = (
        f32::INFINITY,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::NEG_INFINITY,
    );
    for &(x, y) in pts {
        x0 = x0.min(x);
        y0 = y0.min(y);
        x1 = x1.max(x);
        y1 = y1.max(y);
    }
    Rect {
        x: x0,
        y: y0,
        width: (x1 - x0).max(0.0),
        height: (y1 - y0).max(0.0),
    }
}

/// Une entrée `NodeId → style calculé` pour l'attribution
/// terrain (geodiff → feature soupçonnée).
#[derive(Debug, Clone)]
pub struct StyleEntry {
    /// Nœud DOM porteur de la boîte.
    pub node: NodeId,
    /// Plus proche ancêtre de boîte porteur d'un `NodeId`
    /// (chaîne de contexte — la cause d'un décalage vit
    /// presque toujours chez le parent : sa marge, sa
    /// largeur, son `display`).
    pub parent: Option<NodeId>,
    /// Props d'attribution déclarées/résolues, dans l'ordre
    /// fixe d'`ATTRIB_PROPS` (déterministe pour le diff).
    pub computed: Vec<(&'static str, String)>,
}

/// Props retenues pour l'attribution : celles qui font la
/// géométrie (boîte, flux, texte, alignement) — pas les
/// purement visuelles (`color`, `background`…). Un delta
/// `dx/dy/dw/dh` n'a de coupables que dans cet ensemble.
const ATTRIB_PROPS: &[&str] = &[
    "display",
    "position",
    "float",
    "clear",
    "z-index",
    "visibility",
    "box-sizing",
    "width",
    "height",
    "min-width",
    "min-height",
    "max-width",
    "max-height",
    "margin-top",
    "margin-right",
    "margin-bottom",
    "margin-left",
    "margin-inline-start",
    "margin-inline-end",
    "margin-block-start",
    "margin-block-end",
    "padding-top",
    "padding-right",
    "padding-bottom",
    "padding-left",
    "padding-inline-start",
    "padding-inline-end",
    "padding-block-start",
    "padding-block-end",
    "border-top-width",
    "border-right-width",
    "border-bottom-width",
    "border-left-width",
    "line-height",
    "font-size",
    "font-family",
    "font-weight",
    "text-align",
    "text-indent",
    "white-space",
    "vertical-align",
    "direction",
    "writing-mode",
    "overflow",
    "overflow-x",
    "overflow-y",
    "list-style-type",
    "list-style-position",
    "list-style-image",
    "align-content",
    "align-items",
    "align-self",
    "justify-content",
    "justify-items",
    "justify-self",
    "gap",
    "row-gap",
    "column-gap",
    "flex-direction",
    "flex-wrap",
    "flex-grow",
    "flex-shrink",
    "flex-basis",
    "order",
    "grid-template-columns",
    "grid-template-rows",
    "grid-column",
    "grid-row",
    "column-count",
    "column-width",
    "top",
    "right",
    "bottom",
    "left",
    "transform",
    "contain",
    "aspect-ratio",
    "object-fit",
    "content",
    "table-layout",
    "border-collapse",
    "border-spacing",
    "caption-side",
];

/// Parcourt l'arbre posé et rapporte, pour chaque nœud
/// producteur de géométrie, son style calculé réduit aux
/// props d'attribution + le nœud de son plus proche ancêtre
/// de boîte (même marche que [`geometry_dump`] — pile,
/// pré-ordre, déterministe). Consommé par `geo_diff` pour
/// transformer un delta en hypothèse de feature.
#[must_use]
pub fn style_dump(root: &LayoutBox) -> Vec<StyleEntry> {
    let mut out: Vec<StyleEntry> = Vec::new();
    let mut stack = vec![(root, Option::<NodeId>::None)];
    while let Some((b, parent)) = stack.pop() {
        if let Some(n) = b.node {
            out.push(StyleEntry {
                node: n,
                parent,
                computed: ATTRIB_PROPS
                    .iter()
                    .filter_map(|p| b.style.get(p).map(|v| (*p, v.to_string())))
                    .collect(),
            });
        }
        let next_parent = b.node.or(parent);
        stack.extend(b.children.iter().rev().map(|c| (c, next_parent)));
    }
    out
}

/// Union de deux rects (bord le plus externe par axe).
fn union(a: Rect, b: Rect) -> Rect {
    let x = a.x.min(b.x);
    let y = a.y.min(b.y);
    Rect {
        x,
        y,
        width: (a.x + a.width).max(b.x + b.width) - x,
        height: (a.y + a.height).max(b.y + b.height) - y,
    }
}
