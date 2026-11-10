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
    let mut stack = vec![root];
    while let Some(b) = stack.pop() {
        if let Some(n) = b.node {
            // R-213 — une boîte en-ligne porte le rect de sa
            // LIGNE allouée ; `gBCR` rend la border box de
            // contenu (~`font-size`, mesuré Chrome : `<a>`
            // sous `line-height:30` → h=17, pas 30).
            add(n, inline_content_rect(b));
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
                    add(n, r);
                }
            }
        }
        stack.extend(b.children.iter().rev());
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
    if matches!(b.kind, crate::boxtree::BoxKind::Inline | crate::boxtree::BoxKind::Text) {
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
