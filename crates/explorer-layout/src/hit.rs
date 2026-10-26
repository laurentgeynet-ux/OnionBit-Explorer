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
use crate::transform::{box_transform, Affine};

/// Union des rects du sous-arbre **dans le repère de `bx`**
/// (avant sa propre transform, les transforms des
/// descendants déjà appliquées) — la zone que la peinture
/// peut couvrir (bornes de clip `overflow` ignorées :
/// l'union est une borne *conservative*, jamais
/// restrictive). Un enfant transformé peut déborder très
/// loin de son rect layout : son bound suit la matrice.
fn paint_bounds(bx: &LayoutBox) -> Rect {
    // Récursion ∝ profondeur de l'arbre → arêtes `deep` (R-24).
    bx.children.iter().fold(bx.rect, |acc, c| {
        let b = explorer_common::deep(|| paint_bounds(c));
        // Bounds de l'enfant exprimés dans le repère de `bx` :
        // sa propre `transform` s'applique à tout son
        // sous-arbre (même portée que `PushTransform`).
        let b = box_transform(c, &c.rect).map_or(b, |t| map_rect(t, b));
        union(acc, b)
    })
}

/// Bounding-box du rect `r` sous la matrice `t` (les quatre
/// coins transformés — la rotation élargit, comme le
/// raster).
fn map_rect(t: Affine, r: Rect) -> Rect {
    let (x2, y2) = (r.x + r.width, r.y + r.height);
    let pts = [
        t.apply(r.x, r.y),
        t.apply(x2, r.y),
        t.apply(r.x, y2),
        t.apply(x2, y2),
    ];
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for (px, py) in pts {
        x0 = x0.min(px);
        y0 = y0.min(py);
        x1 = x1.max(px);
        y1 = y1.max(py);
    }
    Rect {
        x: x0,
        y: y0,
        width: x1 - x0,
        height: y1 - y0,
    }
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
/// `visibility: hidden|collapse` (style **calculé** — l'héritage
/// est déjà résolu : un descendant explicitement `visible` se
/// relit visible et reste ciblable, §11.2 — R-102). Un élément
/// caché n'est jamais la cible ; on descend quand même dans ses
/// enfants.
fn hidden(bx: &LayoutBox) -> bool {
    matches!(bx.style.get("visibility"), Some("hidden" | "collapse"))
}

#[must_use]
pub fn hit_test(bx: &LayoutBox, x: f32, y: f32) -> Option<NodeId> {
    hit_test_m(bx, x, y, Affine::IDENTITY)
}

/// Descente sous une matrice cumulative `m` (produit des
/// `transform` des ancêtres, origines résolues) : le point
/// sondé est ramené dans le repère **non transformé** de la
/// boîte par `m⁻¹` — les rects du layout restent exprimés
/// avant transform, comme la display list qui les émet sous
/// `PushTransform`. Une matrice non inversible (`scale(0)`)
/// aplatit le sous-arbre : rien n'y est cliquable.
fn hit_test_m(bx: &LayoutBox, x: f32, y: f32, m: Affine) -> Option<NodeId> {
    // La transform propre de la boîte s'applique à elle-même
    // ET à ses descendants (même portée que `PushTransform`).
    let m = box_transform(bx, &bx.rect).map_or(m, |t| t.post_concat(m));
    let (px, py) = m.invert()?.apply(x, y);
    if !contains(paint_bounds(bx), px, py) {
        return None;
    }
    for child in &bx.children {
        if let Some(n) = explorer_common::deep(|| hit_test_m(child, x, y, m)) {
            return Some(n);
        }
    }
    if hidden(bx) {
        return None;
    }
    contains(bx.rect, px, py).then_some(bx.node).flatten()
}

/// Hit-test conscient du scroll (issue #18) : `y` est en
/// coordonnées **viewport** — `scroll_y` s'ajoute en
/// descendant, sauf sous `position:fixed` dont les boîtes
/// sont posées **contre le viewport** (leurs rects sont
/// déjà en coords viewport : le delta retombe à 0 pour tout
/// le sous-arbre — parité avec `build_display_list_scrolled`
/// qui « ré-initialise l'offset » sous `fixed`).
///
/// Sans ce filage, un élément fixed visuellement épinglé
/// n'encaissait plus rien dès `scroll_y > 0` (sonde `y+scroll`
/// sur un rect resté à `y≈0`) — nav github morte au hover.
/// `hit_test` reste la variante coords document (delta nul).
#[must_use]
pub fn hit_test_scrolled(bx: &LayoutBox, x: f32, y: f32, scroll_y: f32) -> Option<NodeId> {
    hit_test_delta(bx, x, y, scroll_y)
}

fn hit_test_delta(bx: &LayoutBox, x: f32, y_vp: f32, delta: f32) -> Option<NodeId> {
    hit_test_delta_m(bx, x, y_vp, delta, Affine::IDENTITY)
}

/// Variante scrollée de [`hit_test_m`] : `m` cumule les
/// `transform` d'ancêtres (inverse appliquée au point, même
/// parité que la display list — vol : hit décalé sur
/// élément transformé).
fn hit_test_delta_m(bx: &LayoutBox, x: f32, y_vp: f32, delta: f32, m: Affine) -> Option<NodeId> {
    let m = box_transform(bx, &bx.rect).map_or(m, |t| t.post_concat(m));
    let (px, py) = m.invert()?.apply(x, y_vp + delta);
    if !contains(paint_bounds(bx), px, py) {
        return None;
    }
    for child in &bx.children {
        // `position:fixed` = coords viewport : delta nul pour
        // tout le sous-arbre. Détection par le **style** (comme
        // `resolve_absolutes`) — le champ `position` n'est
        // renseigné que sur les boîtes de bloc.
        let d = if child.style.get("position") == Some("fixed") {
            0.0
        } else {
            delta
        };
        if let Some(n) = explorer_common::deep(|| hit_test_delta_m(child, x, y_vp, d, m)) {
            return Some(n);
        }
    }
    if hidden(bx) {
        return None;
    }
    contains(bx.rect, px, py).then_some(bx.node).flatten()
}
