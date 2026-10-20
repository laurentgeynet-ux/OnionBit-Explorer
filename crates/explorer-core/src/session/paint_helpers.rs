// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Helpers du chemin de peinture : table canvas, cull de display
//! list, remap de nœuds, instrumentation de phase.
use super::*;
use std::collections::HashMap;
use std::time::Instant;

use explorer_css::value::Color;
use explorer_dom::{Dom, NodeId};
use explorer_js::CanvasOpRecord;
use explorer_layout::{LayoutBox, Rect};
use explorer_render::canvas::{CanvasOp, CanvasTable};
use explorer_render::{DisplayItem, DisplayList};

/// Plafond de régions invalides accumulées par onglet
/// (étape 55) — au-delà, l'ensemble s'effondre en « toute
/// la frame » : un client qui ne draine jamais retombe sur
/// le comportement plein-format, et une rafale de petites
/// invalidations ne fait pas grossir la charge utile SSE
/// sans borne.
pub(crate) const MAX_DIRTY_RECTS: usize = 32;

/// Dimension max d'une frame rasterisée — borne anti-hostile
/// de `rasterize` (`explorer-render`), déclarée ici pour
/// plafonner la display list avant l'appel : un document
/// plus grand que le cap produit une frame tronquée au
/// sommet plutôt qu'une erreur `FrameTooBig` sèche (la
/// peinture progressive doit émettre pendant le
/// téléchargement, même sur page géante — borne documentée,
/// stratégie fenêtre/tuiles consignée `interface-equipes.md`).
pub(crate) const MAX_FRAME_DIM: f32 = 16_384.0;

/// Seuil du diff de dirty-rects (étape 55g) : au-delà, la
/// fusion O(k²) des régions dans `changed_rects` coûte plus
/// cher que la pleine frame — on la produit directement.
pub(crate) const DIRTY_DIFF_MAX_ITEMS: usize = 2_000;

/// Premier élément portant `id` en ordre document — petit
/// parcours pour `label[for]` (pas d'index d'id exposé).
/// Inverse d'[`element_path`] : nœud désigné par `path`
/// dans `dom` courant — utilisé pour re-résoudre les cibles
/// stockées après un re-parse de synchro (les `NodeId`
/// meurent, les chemins survivent).
/// Chronométrage de phase des chemins de synchro (55g-suite)
/// : émet `phase = <µs>` sur la cible `sync_profile` — coût
/// quasi nul quand le filtre est inactif (`RUST_LOG=
/// sync_profile=info` pour attribuer un gel terrain à ses
/// phases sous verrou : dispatch, sérialisation miroir,
/// re-parse, relayout incrémental/global, repaint). Renvoie
/// l'instant courant pour chaîner les phases.
pub(crate) fn sync_phase(tab: TabId, phase: &'static str, start: Instant) -> Instant {
    let now = Instant::now();
    tracing::info!(
        target: "sync_profile",
        tab,
        phase,
        us = u64::try_from(now.duration_since(start).as_micros()).unwrap_or(u64::MAX),
        "phase sync"
    );
    now
}

/// `<canvas>` 2D (JS-C28, couture R-11) : les op-lists
/// **cumulées** du miroir sont re-résolues par chemin
/// d'élément vers les `NodeId` du DOM **commis** — un chemin
/// qui ne mène nulle part (canvas détaché, arbres divergents)
/// est ignoré : le `<canvas>` reste un remplacé vide, jamais
/// une erreur. Pas de moteur (onglet sans JS, moteur détaché
/// 53f) → table vide, les ops seront relues à la prochaine
/// passe.
#[allow(dead_code)]
pub(crate) fn canvas_table(entry: &mut Tab, dom: &Dom) -> CanvasTable {
    let mut out = CanvasTable::new();
    let Some(eng) = entry.js.as_deref_mut() else {
        return out;
    };
    for (path, recs) in eng.canvas_ops() {
        if recs.is_empty() {
            continue;
        }
        let Some(n) = dom_path_node(dom, &path) else {
            continue;
        };
        out.insert(n, recs.iter().map(to_canvas_op).collect());
    }
    out
}

/// `CanvasOpRecord` (miroir `dillo-js`, sérialisable) →
/// `CanvasOp` (playback R-11) — traduction champ pour champ
/// des deux énumérations figées par la couture.
pub(crate) fn to_canvas_op(rec: &CanvasOpRecord) -> CanvasOp {
    use CanvasOpRecord as R;
    match *rec {
        R::Save => CanvasOp::Save,
        R::Restore => CanvasOp::Restore,
        R::Translate(x, y) => CanvasOp::Translate(x, y),
        R::Scale(x, y) => CanvasOp::Scale(x, y),
        R::Rotate(r) => CanvasOp::Rotate(r),
        R::Transform(m1, m2, m3, m4, m5, m6) => CanvasOp::Transform(m1, m2, m3, m4, m5, m6),
        R::SetTransform(m1, m2, m3, m4, m5, m6) => CanvasOp::SetTransform(m1, m2, m3, m4, m5, m6),
        R::FillColor { r, g, b, a } => CanvasOp::FillColor(Color { r, g, b, a }),
        R::StrokeColor { r, g, b, a } => CanvasOp::StrokeColor(Color { r, g, b, a }),
        R::GlobalAlpha(v) => CanvasOp::GlobalAlpha(v),
        R::LineWidth(v) => CanvasOp::LineWidth(v),
        R::BeginPath => CanvasOp::BeginPath,
        R::MoveTo(x, y) => CanvasOp::MoveTo(x, y),
        R::LineTo(x, y) => CanvasOp::LineTo(x, y),
        R::QuadraticTo(cx, cy, x, y) => CanvasOp::QuadraticTo(cx, cy, x, y),
        R::BezierTo(x1, y1, x2, y2, x3, y3) => CanvasOp::BezierTo(x1, y1, x2, y2, x3, y3),
        R::Arc(x, y, r, sa, ea, ccw) => CanvasOp::Arc(x, y, r, sa, ea, ccw),
        R::Rect(x, y, w, h) => CanvasOp::Rect(x, y, w, h),
        R::ClosePath => CanvasOp::ClosePath,
        R::Fill => CanvasOp::Fill,
        R::Stroke => CanvasOp::Stroke,
        R::Clip => CanvasOp::Clip,
        R::FillRect(x, y, w, h) => CanvasOp::FillRect(x, y, w, h),
        R::StrokeRect(x, y, w, h) => CanvasOp::StrokeRect(x, y, w, h),
        R::ClearRect(x, y, w, h) => CanvasOp::ClearRect(x, y, w, h),
    }
}

/// Couverture brute d'un item de dessin (avant transform/
/// clip) — même géométrie que `item_bounds` côté PIPE,
/// pour le filtre viewport de `repaint` (étape 55g).
pub(crate) fn display_item_bounds(item: &DisplayItem) -> Option<Rect> {
    match item {
        DisplayItem::FillRect { rect, .. }
        | DisplayItem::Border { rect, .. }
        | DisplayItem::Placeholder { rect, .. }
        | DisplayItem::Image { rect, .. }
        | DisplayItem::BackgroundImage { rect, .. } => Some(*rect),
        DisplayItem::Text { origin, run, .. } => Some(Rect {
            x: origin.0,
            y: origin.1 - run.metrics.ascent,
            width: run.advance,
            height: run.metrics.ascent - run.metrics.descent,
        }),
        DisplayItem::PushClip(_)
        | DisplayItem::PopClip
        | DisplayItem::PushTransform(_)
        | DisplayItem::PopTransform => None,
    }
}

/// Filtre hors-cadre avant raster (étape 55g — le goulot
/// de la resynchro n'est pas le parse ni la cascade mais
/// le raster qui re-dessine **tous** les glyphes du
/// document à chaque repaint : ~18 s mesurés sur
/// ~3 000 nœuds). Un item-feuille dont la couverture est
/// entièrement hors de la zone raster
/// `[0,width]×[0,height]` (coordonnées liste — scroll et
/// borne de hauteur déjà cuits dedans) est jeté.
/// Conservateurs : (a) sous `PushTransform` actif on garde
/// tout — les bornes brutes ne sont pas fiables sans
/// composer les matrices ; (b) les `PushClip`/`Pop*` sont
/// conservés pour l'équilibre des piles (un clip ignoré
/// garderait de toute façon plus de contenu, jamais moins).
pub(crate) fn cull_display_list(list: &mut DisplayList) {
    let (vw, vh) = (list.width, list.height);
    let mut in_transform = 0usize;
    list.items.retain(|item| match item {
        DisplayItem::PushTransform(_) => {
            in_transform += 1;
            true
        }
        DisplayItem::PopTransform => {
            in_transform = in_transform.saturating_sub(1);
            true
        }
        DisplayItem::PushClip(_) | DisplayItem::PopClip => true,
        _ => {
            in_transform > 0
                || display_item_bounds(item).is_none_or(|r| {
                    let (rx, ry) = (r.x, r.y);
                    rx < vw && ry < vh && rx + r.width > 0.0 && ry + r.height > 0.0
                })
        }
    });
}

/// Re-mappe les `NodeId` des boîtes conservées **avant** le
/// relayout incrémental (étape 55f) : l'arbre porte des ids
/// de l'**ancien** DOM — chacun est re-résolu dans le
/// nouveau par son chemin d'élément (les non-éléments via
/// l'index dans les enfants du parent, inchangé hors
/// sous-arbre). La racine du sous-arbre sale (`target_old`)
/// est traduite en `target_new` et sa descendance n'est pas
/// visitée — elle sera reconstruite par `relayout_subtree`.
/// `false` = un id irrésolvable → l'appelant fait un reflow
/// global (l'arbre mixte est alors jeté, jamais servi).
pub(crate) fn remap_box_nodes(
    old: &Dom,
    new: &Dom,
    root: &mut LayoutBox,
    target_old: NodeId,
    target_new: NodeId,
) -> bool {
    // Itératif — le parcours suit la profondeur du document
    // (R-24 : ~300 <div> imbriqués réels, pas de récursion).
    // `memo` : les enfants d'un même parent réutilisent sa
    // résolution — sans elle chaque nœud texte re-calculait
    // le `element_path` de son élément père (~3 s sur ~3 000
    // nœuds, profil mission 13).
    let mut stack = vec![root];
    let mut memo: HashMap<NodeId, Option<NodeId>> = HashMap::new();
    while let Some(bx) = stack.pop() {
        if bx.node == Some(target_old) {
            bx.node = Some(target_new);
            continue;
        }
        if let Some(id) = bx.node {
            let Some(fresh) = remap_node(old, new, id, &mut memo) else {
                return false;
            };
            bx.node = Some(fresh);
        }
        stack.extend(bx.children.iter_mut());
    }
    true
}

/// `id` (ancien DOM) → `NodeId` équivalent dans `new` :
/// éléments par `element_path`, non-éléments par index dans
/// les enfants du parent (inchangé hors sous-arbre sale).
/// Les résolutions — succès comme échecs — sont mémoïsées
/// dans `memo` : les frères d'un même parent ne paient sa
/// `element_path` qu'une fois.
pub(crate) fn remap_node(
    old: &Dom,
    new: &Dom,
    id: NodeId,
    memo: &mut HashMap<NodeId, Option<NodeId>>,
) -> Option<NodeId> {
    if let Some(&cached) = memo.get(&id) {
        return cached;
    }
    let out = remap_node_uncached(old, new, id, memo);
    memo.insert(id, out);
    out
}

pub(crate) fn remap_node_uncached(
    old: &Dom,
    new: &Dom,
    id: NodeId,
    memo: &mut HashMap<NodeId, Option<NodeId>>,
) -> Option<NodeId> {
    if old.element(id).is_ok() {
        return dom_path_node(new, &element_path(old, id)?);
    }
    let parent = old.parent(id).ok()??;
    let new_parent = if old.element(parent).is_ok() {
        remap_node(old, new, parent, memo)?
    } else {
        new.document()
    };
    let sibs = old.children(parent).ok()?;
    let idx = sibs.iter().position(|&c| c == id)?;
    let cand = new.children(new_parent).ok()?.get(idx).copied()?;
    // Garde structurelle : élément ↔ élément (même tag),
    // texte/commentaire ↔ non-élément.
    let ok = match (old.element(id).is_ok(), new.element(cand).is_ok()) {
        (true, true) => old.tag_name(id).ok() == new.tag_name(cand).ok(),
        (false, false) => true,
        _ => false,
    };
    ok.then_some(cand)
}
