// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Relayout incrémental — R-12, tiers `DirtyLayout` d'ADR-0012.
//!
//! [`relayout_subtree`] reboîte puis re-pose **un seul
//! sous-arbre sale** au lieu d'un `layout_tree` complet :
//! la boîte du nœud cible est reconstruite via
//! [`crate::build_box_tree_sized`], re-posée à son
//! [`BoxSlot`] enregistré, et le changement de hauteur est
//! propagé aux ancêtres (re-flow de leur contenu) tant qu'il
//! bouge la boîte.
//!
//! Bornes conservatrices — la fonction répond
//! [`Relayout::NeedsFullLayout`] (l'appelant retombe sur
//! `layout_tree`) quand le sous-arbre ou son contexte dépend
//! d'un état que le rejeu local ne peut pas reconstruire :
//! - **flottants** : le registre `FloatCtx` des voisins hors
//!   sous-arbre est perdu — tout flottant dans le document
//!   impose un reflow complet (scan une fois sur la racine) ;
//! - **`position: absolute|fixed` sur le chemin** : la pose
//!   se fait dans `resolve_absolutes`, pas dans le flux ;
//! - **chemin traversant une boîte anonyme ou inline** : le
//!   flux en-ligne ne se re-pose pas à l'item près.

use std::collections::HashMap;
use std::hash::BuildHasher;

use explorer_css::StyleMap;
use explorer_dom::{Dom, NodeId};

use crate::block::{layout_block, resolve_absolutes, BlockCtx, FloatCtx};
use crate::boxtree::{build_box_tree_sized, BoxKind, LayoutBox};
use crate::{Float, Position};

/// Résultat d'un relayout incrémental.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Relayout {
    /// Le sous-arbre a été reconstruit et re-posé en place.
    Done,
    /// `target` absent de l'arbre de boîtes (élément non
    /// rendu, `display:none`, nœud retiré…).
    NotFound,
    /// Contexte hors de portée du rejeu local (flottants,
    /// positionnés absolus sur le chemin, flux inline…) —
    /// l'appelant doit refaire un `layout_tree` complet.
    NeedsFullLayout,
}

/// Reboîte et re-pose le sous-arbre de `target` dans l'arbre
/// de boîtes déjà posé `root` (DOM/styles/sizes **à jour**).
///
/// `target` peut être un nœud non-élément (texte…) : il est
/// ramené à son ancêtre élément. Retourne
/// [`Relayout::Done`] si le sous-arbre a été re-posé en place
/// — ses rectangles reflètent le nouvel état.
#[must_use]
pub fn relayout_subtree<S: BuildHasher>(
    root: &mut LayoutBox,
    dom: &Dom,
    styles: &StyleMap,
    sizes: &HashMap<NodeId, (f32, f32), S>,
    target: NodeId,
    ctx: &BlockCtx,
) -> Relayout {
    let Some(target) = nearest_element(dom, target) else {
        return Relayout::NotFound;
    };
    // Un flottant n'importe où dans le document peut pousser
    // le sous-arbre (registre partagé non rejouable) — scan
    // conservateur une fois, pas par niveau.
    if has_float(root) {
        return Relayout::NeedsFullLayout;
    }
    match step(root, dom, styles, sizes, target, ctx) {
        Step::NotFound => Relayout::NotFound,
        Step::Full => Relayout::NeedsFullLayout,
        Step::Relaid(_) => Relayout::Done,
    }
}

enum Step {
    NotFound,
    Full,
    /// Re-posé ; le booléen = la hauteur de la boîte a changé.
    Relaid(bool),
}

/// Descente récursive : reboîte à la cible, propage le re-flow
/// aux ancêtres dont la hauteur bouge.
fn step<S: BuildHasher>(
    bx: &mut LayoutBox,
    dom: &Dom,
    styles: &StyleMap,
    sizes: &HashMap<NodeId, (f32, f32), S>,
    target: NodeId,
    ctx: &BlockCtx,
) -> Step {
    if bx.node == Some(target) {
        if !relayable(bx) {
            return Step::Full;
        }
        let h0 = bx.rect.height;
        let slot = bx.slot;
        let Some(mut fresh) = build_box_tree_sized(dom, target, styles, sizes) else {
            return Step::Full;
        };
        fresh.slot = slot;
        *bx = fresh;
        relay(bx, ctx);
        return Step::Relaid(height_moved(bx.rect.height, h0));
    }
    for i in 0..bx.children.len() {
        if contains(&bx.children[i], target) {
            // Le parent re-flow entier : il doit être un
            // conteneur posé par `layout_block`.
            if !relayable(bx) || !relayable(&bx.children[i]) {
                return Step::Full;
            }
            let child_h0 = bx.children[i].rect.height;
            // Arête ∝ profondeur de l'arbre (R-24).
            return match explorer_common::deep(|| {
                step(&mut bx.children[i], dom, styles, sizes, target, ctx)
            }) {
                // contains() vrai → NotFound impossible ; défensif.
                Step::NotFound | Step::Full => Step::Full,
                Step::Relaid(changed) => {
                    let h0 = bx.rect.height;
                    if changed {
                        // Propagation par translation (R-33) :
                        // sans flottant (`has_float` garanti
                        // en amont), une hausse Δ du fils
                        // décale ses frères in-flow suivants
                        // de Δ — leurs layouts internes sont
                        // invariants (marges et dimensions
                        // inchangées). Le coût passe de
                        // `layout_block` sur tout le sous-arbre
                        // à O(frères × nœuds) — storm.html :
                        // ~2× le layout complet → ~ms.
                        // Gardes : un fils hors-flux ne décale
                        // rien, et une transition vide ↔ non
                        // vide change la chaîne de margins
                        // collapsing (§8.3.1 — les marges
                        // voisines collapsent À TRAVERS une
                        // boîte nulle) → `relay` complet.
                        let child = &bx.children[i];
                        let delta = child.rect.height - child_h0;
                        let in_flow = !child.out_of_flow();
                        let thick =
                            child_h0.abs() > f32::EPSILON && child.rect.height.abs() > f32::EPSILON;
                        if shiftable(bx) && in_flow && thick {
                            for c in &mut bx.children[i + 1..] {
                                explorer_common::deep(|| translate_y(c, delta));
                            }
                            if height_auto(bx) {
                                bx.rect.height += delta;
                            }
                            let s = bx.slot;
                            resolve_absolutes(bx, s.abs_cb, ctx, s.parent_fs);
                        } else {
                            relay(bx, ctx);
                        }
                    }
                    Step::Relaid(height_moved(bx.rect.height, h0))
                }
            };
        }
    }
    Step::NotFound
}

/// `bx` peut propager une hausse de fils par translation :
/// conteneur en **flux bloc normal** — les conteneurs à
/// mise en page coordonnée (flex, grid, table, inline)
/// recalculent leurs enfants → repli `relay`. `min|max-
/// height` déclaré ou `height` spécifiée ne sont pas des
/// cas douteux : la hauteur fixe ne propage rien, les
/// bornes seraient à recalculer → `relay` pour la sûreté.
fn shiftable(bx: &LayoutBox) -> bool {
    matches!(
        bx.display(),
        "block" | "flow-root" | "list-item" | "inline-block" | "table-cell"
    ) && bx.style.get("min-height").is_none()
        && bx.style.get("max-height").is_none()
}

/// Hauteur de contenu auto (ni `height` fixe ni bornes —
/// `shiftable` a déjà exclu `min|max-height`).
fn height_auto(bx: &LayoutBox) -> bool {
    matches!(bx.style.get("height"), None | Some("auto"))
}

/// Translation verticale d'un sous-arbre **posé** : rect,
/// lignes en-ligne (rect + fragments), slot enregistré et
/// enfants récursivement. Les descendants `position:absolute`
/// voient leur position statique bouger comme le flux —
/// `resolve_absolutes` (re-jouée par l'appelant) refait le
/// calcul complet à partir des rects translatés.
fn translate_y(bx: &mut LayoutBox, dy: f32) {
    bx.rect.y += dy;
    bx.slot.y += dy;
    bx.slot.abs_cb.y += dy;
    for line in &mut bx.lines {
        line.rect.y += dy;
        for frag in &mut line.fragments {
            frag.rect.y += dy;
        }
    }
    for c in &mut bx.children {
        explorer_common::deep(|| translate_y(c, dy));
    }
}

/// Re-pose `bx` à son slot enregistré : `layout_block` +
/// résolution des descendants positionnés. Registre de
/// flottants neuf — garanti sans flottants par `has_float`.
fn relay(bx: &mut LayoutBox, ctx: &BlockCtx) {
    let s = bx.slot;
    layout_block(
        bx,
        s.x,
        s.y,
        s.cb_w,
        s.cb_h,
        ctx,
        s.parent_fs,
        s.abs_cb,
        &mut FloatCtx::default(),
    );
    resolve_absolutes(bx, s.abs_cb, ctx, s.parent_fs);
}

/// Hauteur réellement modifiée (les micro-différences de
/// représentation f32 ne déclenchent pas de propagation).
fn height_moved(new: f32, old: f32) -> bool {
    (new - old).abs() > 0.01
}

/// `bx` peut être re-posé par [`relay`] : posé par le flux
/// bloc (ni inline, ni anonyme, ni positionné absolu/fixe —
/// ceux-là vivent dans `resolve_absolutes` / le flux inline).
fn relayable(bx: &LayoutBox) -> bool {
    !matches!(
        bx.kind,
        BoxKind::AnonymousBlock | BoxKind::Inline | BoxKind::Text
    ) && !matches!(bx.position, Position::Absolute | Position::Fixed)
}

/// `target` dans le sous-arbre de `bx` (comparaison `NodeId`).
fn contains(bx: &LayoutBox, target: NodeId) -> bool {
    // Arêtes `deep` : récursion ∝ profondeur de l'arbre (R-24).
    bx.node == Some(target)
        || bx
            .children
            .iter()
            .any(|c| explorer_common::deep(|| contains(c, target)))
}

/// Un flottant dans le sous-arbre — style calculé ou usé.
fn has_float(bx: &LayoutBox) -> bool {
    (bx.float != Float::None || matches!(bx.style.get("float"), Some("left" | "right")))
        || bx
            .children
            .iter()
            .any(|c| explorer_common::deep(|| has_float(c)))
}

/// `n` ou son plus proche ancêtre élément (un nœud texte sale
/// se reboîte via l'élément qui le porte).
fn nearest_element(dom: &Dom, mut n: NodeId) -> Option<NodeId> {
    loop {
        if dom.element(n).is_ok() {
            return Some(n);
        }
        n = dom.parent(n).ok().flatten()?;
    }
}
