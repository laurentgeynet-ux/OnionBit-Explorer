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
/// `visibility: hidden|collapse` (style **calculé** — l'héritage
/// est déjà résolu : un descendant explicitement `visible` se
/// relit visible et reste ciblable, §11.2 — R-102). Un élément
/// caché n'est jamais la cible ; on descend quand même dans ses
/// enfants.
fn hidden(bx: &LayoutBox) -> bool {
    matches!(bx.style.get("visibility"), Some("hidden" | "collapse"))
}

/// La boîte ne peut pas être **cible** de hit-test : outre
/// `visibility`, `pointer-events: none` (vol 2026-10-26 —
/// les overlays auteur `pointer-events:none` volaient tous
/// les hits au-dessus des champs : `editable:false` et clic
/// `ignored` sur des `<input>` pourtant peints). Comme pour
/// `visibility`, on descend quand même : un enfant portant
/// `pointer-events: auto` explicite reste ciblable (sa
/// valeur calculée diffère de celle du parent).
fn unhittable(bx: &LayoutBox) -> bool {
    hidden(bx) || bx.style.get("pointer-events") == Some("none")
}

/// Rang de peinture d'une boîte au sein de son parent
/// (CSS 2.1 §9.9.1 / css-position §3.5 — ordre d'empilement
/// simplifié) : les éléments positionnés (`relative`,
/// `absolute`, `fixed`, `sticky`) passent **après** le flux
/// normal, les `z-index` négatifs avant, les positifs après
/// encore. Le hit-test doit frapper la couche la plus haute
/// en premier — un bandeau/overlays `fixed` transparent qui
/// précède le contenu dans le DOM est peint AU-DESSUS de
/// lui : tester les enfants en ordre de document lui
/// volait les clics des liens situés dessous (vol 19 :
/// `{"result":"ignored"}` sur liens valides de
/// gamefaqs/speedtest, « Français » de canada.ca).
fn paint_rank(c: &LayoutBox) -> i64 {
    let positioned = !matches!(
        c.kind,
        crate::BoxKind::Text | crate::BoxKind::AnonymousBlock
    ) && matches!(
        c.style.get("position"),
        Some("relative" | "absolute" | "fixed" | "sticky")
    );
    if !positioned {
        return 0;
    }
    #[allow(clippy::cast_possible_truncation)]
    let z = c
        .style
        .number("z-index")
        .map_or(i64::from(c.z_index), |v| v as i64);
    // Positionnés : z < 0 sous le flux, z = 0/auto au rang
    // central, z > 0 au-dessus de tout.
    1_000_000_000 + z.signum() * 1_000_000_000 + z.clamp(-999_999_999, 999_999_999)
}

/// Indices des enfants dans l'ordre de **peinture inversé**
/// (la dernière couche peinte, la plus haute, testée en
/// premier) — à rang égal, le dernier du document gagne.
fn hit_order(bx: &LayoutBox) -> Vec<usize> {
    let mut idx: Vec<usize> = (0..bx.children.len()).collect();
    idx.sort_by_key(|&i| paint_rank(&bx.children[i]));
    idx.reverse();
    idx
}

#[must_use]
pub fn hit_test(bx: &LayoutBox, x: f32, y: f32) -> Option<NodeId> {
    if !contains(paint_bounds(bx), x, y) {
        return None;
    }
    for i in hit_order(bx) {
        let child = &bx.children[i];
        if let Some(n) = explorer_common::deep(|| hit_test(child, x, y)) {
            return Some(n);
        }
    }
    if unhittable(bx) {
        return None;
    }
    contains(bx.rect, x, y).then_some(bx.node).flatten()
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
    let y = y_vp + delta;
    if !contains(paint_bounds(bx), x, y) {
        return None;
    }
    for i in hit_order(bx) {
        let child = &bx.children[i];
        // `position:fixed` = coords viewport : delta nul pour
        // tout le sous-arbre. Détection par le **style** (comme
        // `resolve_absolutes`) — le champ `position` n'est
        // renseigné que sur les boîtes de bloc.
        let d = if child.style.get("position") == Some("fixed") {
            0.0
        } else {
            delta
        };
        if let Some(n) = explorer_common::deep(|| hit_test_delta(child, x, y_vp, d)) {
            return Some(n);
        }
    }
    if unhittable(bx) {
        return None;
    }
    contains(bx.rect, x, y).then_some(bx.node).flatten()
}
