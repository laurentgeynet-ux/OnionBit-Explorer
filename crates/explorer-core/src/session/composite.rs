// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Rendu composite des sous-contextes `<iframe>` (étape 129).
//!
//! Chaque [`SubFrame`] fetchée est rasterisée dans **sa
//! propre surface** — pipeline complet (styles inline →
//! boîtes → layout → display list → fenêtre raster bornée à
//! la content box) — puis son bitmap est blitté en
//! `DisplayItem::Image` à la boîte de l'élément. Le canal
//! `PaintImages::replaced` (résolution par `NodeId` dans
//! `paint_replaced`, côté PIPE) fait le composite **sans
//! extension du domaine** : z-order, clipping
//! `overflow`/`clip-path` et `object-fit` viennent du
//! peintre générique.
//!
//! Bornes (étape 128 + propres) :
//! - **profondeur 1** : le pipeline enfant ne fetch aucune
//!   sous-ressource — `<link>`/`<img http>`/sous-iframe
//!   restent vides, `<style>` inline et `data:` rendent ;
//! - **budget pixels** : une boîte dont la surface dépasse
//!   `image_max_px` n'est pas rasterisée (cadre vide) ;
//! - aucun contexte JS enfant — `window.parent`/`top` et
//!   `postMessage` cross-contexte relèvent de la piste JS
//!   (étape 130 — un enfant exige son `JsEngine` propre).
//!
//! ## `NodeId`s et re-parse
//!
//! Les bitmaps sont posés dans `images.replaced` sous des
//! `NodeId` du DOM courant — un re-parse réattribue les
//! ids depuis zéro : [`purge_frame_bitmaps`] court donc
//! **avant** tout insert de la passe (une purge tardive
//! pourrait emporter un bitmap neuf dont l'id réutilise la
//! case d'un ancien cadre), et le cache de surfaces
//! ([`SubFrameBitmaps`]) est clé `(url, w, h)` — jamais par
//! `NodeId`, qui n'a aucune stabilité inter-DOM.

use std::collections::HashMap;
use std::sync::Arc;

use explorer_dom::NodeId;
use explorer_layout::{build_box_tree_sized, layout_tree};
use explorer_render::collect_data_images;

use super::apply::{
    layout_ctx, restitute_font_cache, styles_payload, ApplyCtx, PaintPayload, SharedFonts,
};
use super::*;

/// Bornes du cache de surfaces — un cadre de même URL
/// rendu à deux tailles distinctes ne doit pas faire
/// grossir la table sans fin (resize continu) : au-delà
/// de [`MAX_FRAME_BITMAPS`] entrées la table est vidée,
/// elle se régénère à la passe suivante.
const MAX_FRAME_BITMAPS: usize = 32;

/// Cache de surfaces de sous-contextes, clé `(url
/// résolue, w px, h px)` — un bitmap ne dépend que du
/// document enfant (immuable entre navigations) et de la
/// taille de sa fenêtre ; réutilisable sans risque
/// d'alias d'id après re-parse.
pub(crate) type SubFrameBitmaps = HashMap<(String, u32, u32), Arc<DecodedImage>>;

/// Gabarit remplacé d'un `<iframe>` : attributs `width`/
/// `height` en px sains s'ils existent, défaut spec
/// 300×150 par axe (HTML §14.4.2 — même règle que
/// `canvas_intrinsic` côté PIPE).
pub(crate) fn iframe_intrinsic(dom: &Dom, node: NodeId) -> (f32, f32) {
    let attr_px = |name: &str, def: f32| {
        dom.attribute(node, name)
            .ok()
            .flatten()
            .and_then(|v| v.trim().parse::<f32>().ok())
            .filter(|f| f.is_finite() && *f > 0.0)
            .unwrap_or(def)
    };
    (attr_px("width", 300.0), attr_px("height", 150.0))
}

/// `NodeId`s de **tous** les `<iframe>` du document — la
/// boîte remplacée existe même sans contexte fetché
/// (`src` absent, `srcdoc`, fetch échoué → cadre vide).
/// Même parcours que [`collect_iframes`] (file explicite,
/// ordre document).
pub(crate) fn iframe_nodes(dom: &Dom) -> Vec<NodeId> {
    let mut out = Vec::new();
    let mut queue = vec![dom.document()];
    while let Some(n) = queue.pop() {
        if dom.tag_name(n).is_ok_and(|t| t == "iframe") {
            out.push(n);
        }
        if let Ok(ch) = dom.children(n) {
            queue.extend(ch.iter().rev());
        }
    }
    out
}

/// Retire de `images.replaced` les bitmaps posés par la
/// passe composite précédente et vide `frame_nodes`.
/// **À appeler avant tout insert de bitmap de la passe** :
/// après un re-parse, les `NodeId` réattribués peuvent
/// désigner d'autres éléments — purger tard pourrait
/// retirer un bitmap frais placé sous une case recyclée.
pub(crate) fn purge_frame_bitmaps(p: &mut PaintPayload) {
    for n in std::mem::take(&mut p.frame_nodes) {
        p.images.replaced.remove(&n);
    }
}

/// Content box de la boîte (border box − bordures −
/// paddings) — le viewport du sous-contexte est la zone de
/// contenu. Réplique du `content_rect` privé de
/// `explorer-render` (mêmes règles : `border-style` absent
/// ou `none` → épaisseur nulle ; défaut `medium` = 3 px).
fn content_rect(bx: &LayoutBox) -> Rect {
    let side = |name: &str| {
        let st = &bx.style;
        let border = if st
            .get(&format!("border-{name}-style"))
            .is_none_or(|s| s.trim() == "none")
        {
            0.0
        } else {
            st.get(&format!("border-{name}-width"))
                .and_then(|v| v.trim().trim_end_matches("px").parse::<f32>().ok())
                .unwrap_or(3.0)
        };
        let pad = st
            .get(&format!("padding-{name}"))
            .and_then(|v| v.trim().trim_end_matches("px").parse::<f32>().ok())
            .unwrap_or(0.0);
        border + pad
    };
    let (l, t, r, b) = (side("left"), side("top"), side("right"), side("bottom"));
    Rect {
        x: bx.rect.x + l,
        y: bx.rect.y + t,
        width: (bx.rect.width - l - r).max(0.0),
        height: (bx.rect.height - t - b).max(0.0),
    }
}

/// Boîtes `<iframe>` de l'arbre posé → `(node, content
/// box)` — parcours pré-ordre.
fn iframe_boxes(bx: &LayoutBox, dom: &Dom, out: &mut Vec<(NodeId, Rect)>) {
    if let Some(n) = bx.node {
        if dom.tag_name(n).is_ok_and(|t| t == "iframe") {
            out.push((n, content_rect(bx)));
        }
    }
    for c in &bx.children {
        iframe_boxes(c, dom, out);
    }
}

/// Bitmaps des sous-contextes dans `p.images.replaced`, à
/// la clé `NodeId` de chaque boîte `<iframe>` —
/// `paint_replaced`/`is_media` (PIPE) émettent alors
/// l'`Image` clippé à la content box. Appelé après le
/// layout dans `relayout_dom_payload` et
/// `try_incr_payload`, [`purge_frame_bitmaps`] ayant déjà
/// nettoyé la passe précédente.
///
/// Les surfaces vivent dans `p.frame_bitmaps`, clées
/// `(url, w, h)` : un cadre de même adresse sert toutes
/// ses boîtes homonymes (dédup `seen_frames` à la vague),
/// une taille identique ne re-rend jamais, et aucune
/// réutilisation ne peut croiser deux sous-contextes —
/// contrairement à un cache par `NodeId`, instable au
/// re-parse.
pub(crate) fn composite_frames(p: &mut PaintPayload, ctx: &ApplyCtx, dom: &Dom) {
    let Some(tree) = p.tree.as_ref() else { return };
    let mut boxes = Vec::new();
    iframe_boxes(tree, dom, &mut boxes);
    if boxes.is_empty() {
        return;
    }
    let base = p.base_url.clone();
    for (node, rect) in boxes {
        let (w_px, h_px) = (rect.width * ctx.scale, rect.height * ctx.scale);
        if !(w_px.is_finite() && h_px.is_finite())
            || w_px < 1.0
            || h_px < 1.0
            || f64::from(w_px) * f64::from(h_px) > f64::from(ctx.image_max_px)
        {
            continue;
        }
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let (wp, hp) = (w_px.round() as u32, h_px.round() as u32);
        let (Some(base), Ok(Some(src))) = (base.as_ref(), dom.attribute(node, "src")) else {
            continue;
        };
        let Ok(url) = Url::resolve(base, &src) else {
            continue;
        };
        let Some(f) = p.frames.iter().find(|f| f.url == url) else {
            continue;
        };
        if p.frame_bitmaps.len() >= MAX_FRAME_BITMAPS {
            p.frame_bitmaps.clear();
        }
        let key = (url.to_string(), wp, hp);
        let img = p
            .frame_bitmaps
            .entry(key)
            .or_insert_with(|| {
                render_subframe(f, rect.width, rect.height, ctx)
                    .map_or_else(|| Arc::new(blank(wp, hp)), Arc::new)
            })
            .clone();
        p.images.replaced.insert(node, img);
        p.frame_nodes.push(node);
    }
}

/// Bitmap vide de la fenêtre — posé quand le pipeline
/// enfant dégrade (page hostile hors bornes, pas de
/// racine) : le cadre rend une surface blanche opaque
/// (un `<iframe>` en échec peint `about:blank`, pas un
/// trou sur le fond parent), et la défaillance n'est pas
/// re-tentée à chaque passe (le cache clé `(url, w, h)`
/// la mémorise).
fn blank(w: u32, h: u32) -> DecodedImage {
    DecodedImage {
        width: w,
        height: h,
        rgba: vec![255; w.saturating_mul(h) as usize * 4],
    }
}

/// Pipeline enfant bornée : `f.dom` → bitmap de la fenêtre
/// `[0, w×h)` px CSS à l'échelle session. `None` = cadre
/// vide (pas de racine, page hostile hors bornes).
fn render_subframe(f: &SubFrame, w: f32, h: f32, ctx: &ApplyCtx) -> Option<DecodedImage> {
    let mut cp = PaintPayload {
        base_url: Some(f.url.clone()),
        ..PaintPayload::default()
    };
    let child_ctx = ApplyCtx {
        fonts: SharedFonts(Arc::clone(&ctx.fonts.0)),
        viewport: (w, h),
        scale: ctx.scale,
        image_max_px: ctx.image_max_px,
        color_scheme: ctx.color_scheme,
        dom_journal_max_ops: ctx.dom_journal_max_ops,
    };
    let root_el = f.dom.document_element()?;
    let styles = styles_payload(&mut cp, &f.dom, (w, h), ctx.color_scheme);
    let data_imgs = collect_data_images(&f.dom, &styles, ctx.image_max_px);
    cp.images.replaced.extend(data_imgs.replaced);
    cp.images.backgrounds.extend(data_imgs.backgrounds);
    cp.images.masks_url.extend(data_imgs.masks_url);
    let mut sizes = image_sizes(&f.dom, &cp.images.replaced);
    sizes.extend(explorer_render::svg::svg_sizes(&f.dom, root_el));
    // Les `<iframe>` *de l'enfant* gardent leur gabarit
    // atomique mais aucun contexte n'est fetché — cadre
    // vide (profondeur 1, étape 128).
    for n in iframe_nodes(&f.dom) {
        sizes
            .entry(n)
            .or_insert_with(|| iframe_intrinsic(&f.dom, n));
    }
    cp.images.svgs = explorer_render::svg::collect_svg(&f.dom, root_el, ctx.image_max_px);
    // Pas de moteur JS enfant à cette tranche : les
    // `<canvas>` du sous-document n'ont pas d'op-lists.
    let mut bx = build_box_tree_sized(&f.dom, root_el, &styles, &sizes)?;
    let block = layout_ctx(&child_ctx, &mut cp);
    layout_tree(&mut bx, w, &block);
    restitute_font_cache(&mut cp, &block);
    let list =
        explorer_render::build_display_list_incremental(&bx, w, h, (0.0, 0.0), &cp.images, None, 0);
    let frame = explorer_render::rasterize_window_cached(
        &list,
        &child_ctx.fonts,
        ctx.scale,
        0.0,
        h,
        &mut cp.font_cache,
    )?;
    Some(DecodedImage {
        width: frame.width,
        height: frame.height,
        rgba: frame.pixels,
    })
}
