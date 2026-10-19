// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Flexbox — MVP d'une seule ligne (`flex-wrap` ignoré).
//!
//! Couvert : `display: flex`/`inline-flex` (le conteneur est
//! de niveau bloc / atomique en-ligne), `flex-direction`
//! row/column (+`-reverse`), `order`, `flex-grow`,
//! `flex-shrink`, `flex-basis`, shorthand `flex`,
//! `justify-content` (6 valeurs usuelles), `align-items` et
//! `align-self`, `gap`/`row-gap`/`column-gap`.
//!
//! Écarts connus : une base `auto` non résolue (ni
//! `flex-basis`, ni `width`, ni intrinsèque remplacé) mesure
//! le max-content réel du contenu (R-3) ; `stretch` étend le
//! rect cross sans re-fluide le contenu ; `margin: auto` est
//! résolu sur les deux axes (le libre principal avant
//! `justify-content`, le cross en reliquat) ; `flex-wrap`
//! non implémenté ; en `column`, la distribution du libre
//! n'a pas de sens sans hauteur de conteneur — les items
//! s'empilent à leur taille naturelle.

use explorer_css::ComputedStyle;

use crate::block::{
    css_px, font_size_px, layout_anon_block, layout_block, raw_length_px, side_px, BlockCtx,
    FloatCtx,
};
use crate::boxtree::{BoxKind, LayoutBox};
use crate::Rect;

/// Paramètres de l'axe principal lus sur le conteneur.
struct Axis {
    /// `true` = row (main horizontal), `false` = column.
    row: bool,
    /// Sens de lecture inversé (`*-reverse`).
    reverse: bool,
}

/// Facteurs d'un item, résolus une fois (longhands +
/// shorthand `flex` : `N` → grow, longueur → basis,
/// `none`/`auto` → couples documentés).
struct ItemFlex {
    grow: f32,
    shrink: f32,
    /// `flex-basis`/`width`(main) résolue en px, ou mesure
    /// texte — `None` = part égale du libre.
    base: Option<f32>,
    /// `align-self` s'il diffère de `auto`.
    align: Option<String>,
}

/// Nombre flottant d'une propriété (`flex-grow`…), `None`.
fn prop_num(st: &ComputedStyle, prop: &str) -> Option<f32> {
    st.get(prop)?.trim().parse::<f32>().ok()
}

/// `gap`/`column-gap`/`row-gap` en px pour l'axe `row`.
pub(crate) fn gap_px(st: &ComputedStyle, row: bool, basis: f32, fs: f32, ctx: &BlockCtx) -> f32 {
    let axis_prop = if row { "column-gap" } else { "row-gap" };
    css_px(st, axis_prop, basis, fs, ctx)
        .or_else(|| {
            // `gap: <row> [<col>]` — valeur de l'axe concerné.
            st.get("gap").and_then(|v| {
                let mut it = v.split_whitespace();
                let first = it.next()?;
                let second = it.next();
                let pick = if row { second.unwrap_or(first) } else { first };
                raw_length_px(pick, basis, fs, ctx)
            })
        })
        .unwrap_or(0.0)
}

/// Résout les facteurs `flex-*` d'un enfant pour l'axe `row`.
fn item_flex(c: &LayoutBox, row: bool, cb: f32, fs: f32, ctx: &BlockCtx) -> ItemFlex {
    let st = &c.style;
    let (mut grow, mut shrink, mut basis_raw) = (
        prop_num(st, "flex-grow"),
        prop_num(st, "flex-shrink"),
        st.get("flex-basis").map(str::to_string),
    );
    // Shorthand `flex` — n'écrase que ce qui est absent
    // (approximation : le cascade a déjà tranché l'ordre).
    if let Some(flex) = st.get("flex") {
        let t = flex.trim();
        if grow.is_none() && shrink.is_none() && basis_raw.is_none() {
            match t {
                "none" => {
                    grow = Some(0.0);
                    shrink = Some(0.0);
                }
                "auto" => {
                    grow = Some(1.0);
                    shrink = Some(1.0);
                }
                "initial" => {}
                _ => {
                    let mut nums = t.split_whitespace().filter_map(|s| s.parse::<f32>().ok());
                    grow = nums.next().or(grow);
                    shrink = nums.next().or(shrink);
                    if let Some(b) = t
                        .split_whitespace()
                        .find(|s| s.parse::<f32>().is_err() && *s != "auto")
                    {
                        basis_raw = Some(b.to_string());
                    }
                }
            }
        }
    }
    let basis_prop = if row { "width" } else { "height" };
    let base = basis_raw
        .as_deref()
        .filter(|b| !b.eq_ignore_ascii_case("auto") && !b.eq_ignore_ascii_case("content"))
        .and_then(|b| raw_length_px(b, cb, fs, ctx))
        .or_else(|| crate::intrinsic::intrinsic_size_px(st, "flex-basis", c, cb, fs, ctx))
        .or_else(|| css_px(st, basis_prop, cb, fs, ctx))
        .or_else(|| crate::intrinsic::intrinsic_size_px(st, basis_prop, c, cb, fs, ctx))
        .or_else(|| c.intrinsic.map(|i| if row { i.0 } else { i.1 }));
    ItemFlex {
        grow: grow.unwrap_or(0.0).max(0.0),
        shrink: shrink.unwrap_or(1.0).max(0.0),
        base,
        align: st.get("align-self").map(str::to_string),
    }
}

/// Translate tout le sous-arbre posé (`rect`, lignes,
/// fragments, enfants récursifs).
pub(crate) fn translate_subtree(bx: &mut LayoutBox, dx: f32, dy: f32) {
    bx.rect.x += dx;
    bx.rect.y += dy;
    for line in &mut bx.lines {
        line.rect.x += dx;
        line.rect.y += dy;
        for f in &mut line.fragments {
            f.rect.x += dx;
            f.rect.y += dy;
        }
    }
    for c in &mut bx.children {
        translate_subtree(c, dx, dy);
    }
}

/// Pose un item à `(ix, iy)` de piste `iw` — bloc ou bloc
/// anonyme selon sa sorte.
pub(crate) fn layout_item(
    c: &mut LayoutBox,
    ix: f32,
    iy: f32,
    iw: f32,
    ctx: &BlockCtx,
    fs: f32,
    pos_cb: Rect,
) {
    match c.kind {
        BoxKind::AnonymousBlock => {
            layout_anon_block(c, ix, iy, iw, ctx, fs, pos_cb, &FloatCtx::default(), None);
        }
        _ => {
            // Item flex = BFC propre : registre vierge. `cb_h`
            // non propagé : la hauteur de la ligne flex dépend
            // du contenu — % enfants indéfinis (§10.5).
            layout_block(
                c,
                ix,
                iy,
                iw,
                None,
                ctx,
                fs,
                pos_cb,
                &mut FloatCtx::default(),
            );
        }
    }
}

/// `justify-content` → décalage du premier item + espacement
/// inter-items supplémentaire.
#[allow(clippy::cast_precision_loss)]
fn justify_offsets(j: &str, free: f32, count: usize) -> (f32, f32) {
    let n = count.max(1) as f32;
    match j {
        "flex-end" => (free.max(0.0), 0.0),
        "center" => (free.max(0.0) / 2.0, 0.0),
        "space-between" if count > 1 => (0.0, free.max(0.0) / (n - 1.0)),
        "space-around" => (free.max(0.0) / n / 2.0, free.max(0.0) / n),
        "space-evenly" => (free.max(0.0) / (n + 1.0), free.max(0.0) / (n + 1.0)),
        _ => (0.0, 0.0),
    }
}

/// `align-items`/`align-self` → décalage cross de l'item
/// (`stretch`/`baseline` ≈ `flex-start` — MVP).
fn align_offset(mode: &str, line_cross: f32, item_cross: f32) -> f32 {
    match mode {
        "center" => (line_cross - item_cross).max(0.0) / 2.0,
        "flex-end" => (line_cross - item_cross).max(0.0),
        _ => 0.0,
    }
}

/// Flux flex des enfants de `bx` — appelé par `layout_block`
/// après son propre dimensionnement. Retourne le curseur
/// final (bas du contenu).
#[allow(
    clippy::too_many_arguments,
    clippy::too_many_lines,
    clippy::cast_precision_loss
)]
pub(crate) fn flow_flex_children(
    bx: &mut LayoutBox,
    content_x: f32,
    start_cursor: f32,
    content_w: f32,
    ctx: &BlockCtx,
    fs: f32,
    pos_cb: Rect,
) -> f32 {
    let st = bx.style.clone();
    let axis = match st.get("flex-direction") {
        Some("row-reverse") => Axis {
            row: true,
            reverse: true,
        },
        Some("column") => Axis {
            row: false,
            reverse: false,
        },
        Some("column-reverse") => Axis {
            row: false,
            reverse: true,
        },
        _ => Axis {
            row: true,
            reverse: false,
        },
    };
    let main_cb = content_w;
    let gap = gap_px(&st, axis.row, main_cb, fs, ctx);
    let justify = st.get_or("justify-content", "flex-start").to_string();
    let align = st.get_or("align-items", "stretch").to_string();

    // Indices d'items triés par `order` (stable).
    let mut order_idx: Vec<usize> = (0..bx.children.len()).collect();
    let orders: Vec<i32> = bx
        .children
        .iter()
        .map(|c| {
            c.style
                .get("order")
                .and_then(|v| v.trim().parse::<i32>().ok())
                .unwrap_or(0)
        })
        .collect();
    order_idx.sort_by_key(|&i| orders[i]);
    let n = order_idx.len();
    if n == 0 {
        return start_cursor;
    }

    if axis.row {
        // Facteurs + bases résolus (main = horizontal).
        let mut items: Vec<ItemFlex> = Vec::with_capacity(n);
        for &i in &order_idx {
            let c = &bx.children[i];
            let cfs = font_size_px(&c.style, fs, ctx);
            let mut f = item_flex(c, true, main_cb, cfs, ctx);
            if f.base.is_none() {
                // R-3 : mesure max-content réelle du contenu
                // (était : texte direct seulement).
                f.base = Some(crate::intrinsic::max_content_px(c, cfs, ctx));
            }
            items.push(f);
        }
        let gaps = gap * (n as f32 - 1.0).max(0.0);
        // Marges axe principal (R-3) : `auto` absorbé plus bas,
        // le fixe compte dans l'espace libre (boîte de marge).
        let mut ml = Vec::with_capacity(n);
        let mut mr = Vec::with_capacity(n);
        let mut n_auto = 0usize;
        for &i in &order_idx {
            let c = &bx.children[i];
            let cfs = font_size_px(&c.style, fs, ctx);
            let (a_l, a_r) = (
                c.style.get("margin-left") == Some("auto"),
                c.style.get("margin-right") == Some("auto"),
            );
            ml.push(if a_l {
                0.0
            } else {
                side_px(&c.style, "margin-left", main_cb, cfs, ctx, 0.0)
            });
            mr.push(if a_r {
                0.0
            } else {
                side_px(&c.style, "margin-right", main_cb, cfs, ctx, 0.0)
            });
            n_auto += usize::from(a_l) + usize::from(a_r);
        }
        let margins: f32 = ml.iter().sum::<f32>() + mr.iter().sum::<f32>();
        let free =
            main_cb - gaps - margins - items.iter().map(|f| f.base.unwrap_or(0.0)).sum::<f32>();
        // Tailles main finales : grow répartit le libre ; sans
        // grow, les bases `None` se partagent le libre à parts
        // égales (approx max-content) ; dépassement → shrink
        // pondéré `shrink × base`.
        let total_grow: f32 = items.iter().map(|f| f.grow).sum();
        let auto_count = items.iter().filter(|f| f.base.is_none()).count() as f32;
        let mut sizes: Vec<f32> = Vec::with_capacity(n);
        if free > 0.0 && total_grow > 0.0 {
            for f in &items {
                sizes.push(f.base.unwrap_or(0.0) + free * f.grow / total_grow);
            }
        } else if free > 0.0 && auto_count > 0.0 {
            let share = free / auto_count;
            for f in &items {
                sizes.push(f.base.unwrap_or(share));
            }
        } else {
            let denom: f32 = items.iter().map(|f| f.shrink * f.base.unwrap_or(0.0)).sum();
            for f in &items {
                let b = f.base.unwrap_or(0.0);
                sizes.push(if denom > 0.0 && free < 0.0 {
                    (b + free * (f.shrink * b) / denom).max(0.0)
                } else {
                    b
                });
            }
        }
        // `margin: auto` axe principal : chaque marge auto
        // absorbe une part égale du libre restant AVANT
        // `justify-content` (spec §8.1 — le libre consommé
        // neutralise justify). Les marges sont résolues en px
        // sur le style de l'item : `layout_block` les applique
        // une seule fois à l'intérieur de la piste élargie.
        let free_m = main_cb
            - gaps
            - sizes.iter().sum::<f32>()
            - ml.iter().sum::<f32>()
            - mr.iter().sum::<f32>();
        if free_m > 0.0 && n_auto > 0 {
            let share = free_m / n_auto as f32;
            for (k, &i) in order_idx.iter().enumerate() {
                let cst = &bx.children[i].style;
                if cst.get("margin-left") == Some("auto") {
                    ml[k] = share;
                }
                if cst.get("margin-right") == Some("auto") {
                    mr[k] = share;
                }
            }
        }
        for (k, &i) in order_idx.iter().enumerate() {
            let c = &mut bx.children[i];
            c.style.set("margin-left", &format!("{}px", ml[k]));
            c.style.set("margin-right", &format!("{}px", mr[k]));
        }
        let used: f32 = (0..n).map(|k| sizes[k] + ml[k] + mr[k]).sum::<f32>() + gaps;
        let (mut lead, between) = justify_offsets(&justify, main_cb - used, n);
        if axis.reverse {
            lead = main_cb - used - lead;
        }
        // Passe 1 : pose à (lead…, cursor) — cross = hauteurs.
        // La piste de l'item inclut ses marges résolues.
        let mut crosses = Vec::with_capacity(n);
        let mut pen = content_x + lead.max(0.0);
        for (k, &i) in order_idx.iter().enumerate() {
            layout_item(
                &mut bx.children[i],
                pen,
                start_cursor,
                sizes[k] + ml[k] + mr[k],
                ctx,
                fs,
                pos_cb,
            );
            crosses.push(bx.children[i].rect.height);
            pen += sizes[k] + ml[k] + mr[k] + gap + between;
        }
        let line_cross = crosses.iter().copied().fold(0.0f32, f32::max);
        // Passe 2 : décalage cross selon align — `stretch`
        // étire la boîte à la hauteur de ligne quand sa taille
        // cross est `auto` (approximation : le rect s'étend,
        // le contenu n'est pas re-flouté) ; les marges cross
        // `auto` partagent le reliquat (centrage).
        for (k, &i) in order_idx.iter().enumerate() {
            let c = &mut bx.children[i];
            let mode = items[k].align.as_deref().unwrap_or(&align);
            let explicit_h = c.style.get("height").is_some_and(|v| v != "auto");
            let (top_auto, bot_auto) = (
                c.style.get("margin-top") == Some("auto"),
                c.style.get("margin-bottom") == Some("auto"),
            );
            if mode == "stretch" && !explicit_h {
                c.rect.height = line_cross;
                continue;
            }
            let dy = if top_auto || bot_auto {
                let slack = (line_cross - crosses[k]).max(0.0);
                match (top_auto, bot_auto) {
                    (true, true) => slack / 2.0,
                    (true, false) => slack,
                    _ => 0.0,
                }
            } else {
                align_offset(mode, line_cross, crosses[k])
            };
            if dy > 0.0 {
                translate_subtree(c, 0.0, dy);
            }
        }
        start_cursor + line_cross
    } else {
        // Colonne : empilement à taille naturelle, pleine
        // largeur cross ; `align-*` décale horizontalement les
        // items à `width` explicite. `column-reverse` inverse
        // l'ordre (ancrage haut conservé — MVP).
        let mut pen_y = start_cursor;
        let iter: Vec<usize> = if axis.reverse {
            order_idx.iter().rev().copied().collect()
        } else {
            order_idx
        };
        for (k, i) in iter.into_iter().enumerate() {
            if k > 0 {
                pen_y += gap;
            }
            let c = &mut bx.children[i];
            let mode = item_flex(c, false, main_cb, fs, ctx)
                .align
                .unwrap_or_else(|| align.clone());
            layout_item(c, content_x, pen_y, content_w, ctx, fs, pos_cb);
            // Cross = horizontal : marges `auto` → centrage.
            let (l_auto, r_auto) = (
                c.style.get("margin-left") == Some("auto"),
                c.style.get("margin-right") == Some("auto"),
            );
            let slack = (content_w - c.rect.width).max(0.0);
            let dx = match (l_auto, r_auto) {
                (true, true) => slack / 2.0,
                (true, false) => slack,
                (false, true) => 0.0,
                (false, false) => align_offset(&mode, content_w, c.rect.width),
            };
            if dx > 0.0 {
                translate_subtree(c, dx, 0.0);
            }
            pen_y += c.rect.height;
        }
        pen_y
    }
}
