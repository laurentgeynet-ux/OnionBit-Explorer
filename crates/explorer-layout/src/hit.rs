// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Hit-test sur l'arbre de boîtes (R-19).
//!
//! L'élagage géométrique `point ∈ rect` est **faux** dès
//! qu'un enfant déborde son parent (repositionnements,
//! marges négatives, `overflow: visible`, centrage bloc) :
//! le sous-arbre peint peut excéder le border-box du parent
//! — c'est le bug terrain lite.ddg (`<center>` pose son
//! `<form>` hors de son rect → zones cliquables mortes).
//!
//! La bonne borne d'élagage est `paint_bounds` : l'union des
//! rects du sous-arbre (ce que la peinture couvre
//! réellement). Elle reste conservative — un point hors de
//! l'union ne peut toucher aucun descendant — mais inclut
//! les enfants échappés. `hit_test` conserve la sémantique
//! historique : la boîte la plus **imbriquée** dont le rect
//! propre contient le point gagne ; son `NodeId` remonte au
//! session (`None` pour les boîtes anonymes — l'appelant
//! remonte aux ancêtres DOM).

use explorer_dom::NodeId;

use crate::boxtree::{LayoutBox, Rect};

/// Union des rects du sous-arbre — la zone que la peinture
/// peut couvrir (bornes de clip `overflow` ignorées à ce
/// stade : l'union est une borne *conservative*, jamais
/// restrictive).
fn paint_bounds(bx: &LayoutBox) -> Rect {
    // Récursion ∝ profondeur de l'arbre → arêtes `deep` (R-24).
    bx.children.iter().fold(bx.rect, |acc, c| {
        union(acc, explorer_common::deep(|| paint_bounds(c)))
    })
}

fn union(a: Rect, b: Rect) -> Rect {
    let (x, y) = (a.x.min(b.x), a.y.min(b.y));
    let (x2, y2) = (
        (a.x + a.width).max(b.x + b.width),
        (a.y + a.height).max(b.y + b.height),
    );
    Rect {
        x,
        y,
        width: x2 - x,
        height: y2 - y,
    }
}

fn contains(r: Rect, x: f32, y: f32) -> bool {
    r.x <= x && x <= r.x + r.width && r.y <= y && y <= r.y + r.height
}

/// Hit-test profond : la boîte la plus imbriquée dont le
/// **rect propre** contient `(x, y)` document gagne — son
/// `NodeId` source (`None` pour les boîtes anonymes, le
/// repli parent étant laissé à l'appelant).
///
/// L'élagage se fait sur `paint_bounds` (union du
/// sous-arbre), pas sur le border-box : un enfant peint hors
/// du parent reste hit-testable (R-19). Les fragments de
/// lignes en-ligne ne sont pas descendus — parité avec la
/// version historique : l'élément inline porteur gagne.
#[must_use]
pub fn hit_test(bx: &LayoutBox, x: f32, y: f32) -> Option<NodeId> {
    if !contains(paint_bounds(bx), x, y) {
        return None;
    }
    for child in &bx.children {
        if let Some(n) = explorer_common::deep(|| hit_test(child, x, y)) {
            return Some(n);
        }
    }
    contains(bx.rect, x, y).then_some(bx.node).flatten()
}
