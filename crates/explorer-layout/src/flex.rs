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
//! `justify-content`, le cross en reliquat) ; `flex-wrap` en
//! `column` n'est pas implémenté (colonnes empilées sans
//! hauteur de conteneur) ; `align-content` n'agit que si la
//! hauteur du conteneur est définie en px (% sans base =
//! indéfini).

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
        // Arête ∝ profondeur de l'arbre → pile garantie (R-24).
        explorer_common::deep(|| translate_subtree(c, dx, dy));
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
        // Marges axe principal (R-3) : `auto` absorbé par ligne
        // plus bas, le fixe compte dans l'espace libre.
        let mut margins: Vec<(f32, f32)> = Vec::with_capacity(n);
        let mut main_auto = vec![(false, false); n];
        for (k, &i) in order_idx.iter().enumerate() {
            let c = &bx.children[i];
            let cfs = font_size_px(&c.style, fs, ctx);
            main_auto[k] = (
                c.style.get("margin-left") == Some("auto"),
                c.style.get("margin-right") == Some("auto"),
            );
            margins.push((
                if main_auto[k].0 {
                    0.0
                } else {
                    side_px(&c.style, "margin-left", main_cb, cfs, ctx, 0.0)
                },
                if main_auto[k].1 {
                    0.0
                } else {
                    side_px(&c.style, "margin-right", main_cb, cfs, ctx, 0.0)
                },
            ));
        }

        // `flex-wrap` : découpe des items en lignes par taille
        // hypothétique (base + marges, §9.5) — `nowrap` = une
        // seule ligne ; `wrap-reverse` inverse l'axe cross.
        let wrap_mode = st.get("flex-wrap").unwrap_or("nowrap");
        let wrap = wrap_mode != "nowrap";
        let wrap_rev = wrap_mode == "wrap-reverse";
        let row_gap = gap_px(&st, false, main_cb, fs, ctx);
        let mut lines: Vec<Vec<usize>> = Vec::new();
        {
            let mut cur: Vec<usize> = Vec::new();
            let mut cur_w = 0.0f32;
            for k in 0..n {
                let outer = items[k].base.unwrap_or(0.0) + margins[k].0 + margins[k].1;
                let need = outer + if cur.is_empty() { 0.0 } else { gap };
                if wrap && !cur.is_empty() && cur_w + need > main_cb + 0.01 {
                    lines.push(std::mem::take(&mut cur));
                    cur_w = 0.0;
                }
                cur_w += if cur.is_empty() { outer } else { outer + gap };
                cur.push(k);
            }
            if !cur.is_empty() {
                lines.push(cur);
            }
        }

        // Pose chaque ligne à y=0 relatif (translation cross
        // finale plus bas) : tailles flex, marges auto,
        // justify — tout se calcule **par ligne** (§9.6/9.7).
        let mut crosses: Vec<f32> = vec![0.0; n];
        // Marges cross (haut/bas) par item — ajoutées à la
        // taille de ligne ET à l'offset de pose (passe 2).
        let mut cross_margins: Vec<(f32, f32)> = vec![(0.0, 0.0); n];
        let mut line_crosses = Vec::with_capacity(lines.len());
        for line in &lines {
            let lgaps = gap * (line.len() as f32 - 1.0).max(0.0);
            let mut used_base = lgaps;
            for &k in line {
                used_base += items[k].base.unwrap_or(0.0) + margins[k].0 + margins[k].1;
            }
            let free = main_cb - used_base;
            let total_grow: f32 = line.iter().map(|&k| items[k].grow).sum();
            // Tailles main finales de la ligne : grow répartit
            // le libre ; dépassement → shrink pondéré.
            let mut sizes_l: Vec<f32> = Vec::with_capacity(line.len());
            if free > 0.0 && total_grow > 0.0 {
                for &k in line {
                    sizes_l.push(items[k].base.unwrap_or(0.0) + free * items[k].grow / total_grow);
                }
            } else {
                let denom: f32 = line
                    .iter()
                    .map(|&k| items[k].shrink * items[k].base.unwrap_or(0.0))
                    .sum();
                for &k in line {
                    let b = items[k].base.unwrap_or(0.0);
                    sizes_l.push(if denom > 0.0 && free < 0.0 {
                        (b + free * (items[k].shrink * b) / denom).max(0.0)
                    } else {
                        b
                    });
                }
            }
            // Marges `auto` : part égale du libre restant avant
            // justify (spec §8.1).
            let free_m = main_cb
                - lgaps
                - line
                    .iter()
                    .zip(&sizes_l)
                    .map(|(&k, s)| s + margins[k].0 + margins[k].1)
                    .sum::<f32>();
            let n_auto_l = line
                .iter()
                .filter(|&&k| main_auto[k].0 || main_auto[k].1)
                .count();
            let mut line_margins = margins.clone();
            if free_m > 0.0 && n_auto_l > 0 {
                let share = free_m
                    / (line
                        .iter()
                        .map(|&k| usize::from(main_auto[k].0) + usize::from(main_auto[k].1))
                        .sum::<usize>() as f32);
                for &k in line {
                    if main_auto[k].0 {
                        line_margins[k].0 = share;
                    }
                    if main_auto[k].1 {
                        line_margins[k].1 = share;
                    }
                }
            }
            // Marges résolues en px sur le style de l'item —
            // `layout_block` les applique une fois, dans la
            // piste qui les inclut.
            for &k in line {
                let c = &mut bx.children[order_idx[k]];
                c.style
                    .set("margin-left", &format!("{}px", line_margins[k].0));
                c.style
                    .set("margin-right", &format!("{}px", line_margins[k].1));
            }
            let used: f32 = line
                .iter()
                .zip(&sizes_l)
                .map(|(&k, s)| s + line_margins[k].0 + line_margins[k].1)
                .sum::<f32>()
                + lgaps;
            // `*-reverse` : l'axe principal part du bord
            // opposé — les items se posent dans l'ordre
            // logique inversé et le groupe est mesuré depuis
            // main-start (bord droit en row-reverse) : le lead
            // se reflète, espacement `between` inclus.
            let (lead_j, between) = justify_offsets(&justify, main_cb - used, line.len());
            let between_total = (line.len() as f32 - 1.0).max(0.0) * between;
            let lead = if axis.reverse {
                main_cb - used - between_total - lead_j
            } else {
                lead_j
            };
            // Passe 1 : pose à (lead…, 0) — cross = hauteurs
            // **extérieures** (§9.4 : les marges cross
            // comptent dans la taille de ligne).
            let mut pen = content_x + lead.max(0.0);
            let mut line_cross = 0.0f32;
            let lay_order: Vec<usize> = if axis.reverse {
                (0..line.len()).rev().collect()
            } else {
                (0..line.len()).collect()
            };
            for &lk in &lay_order {
                let k = line[lk];
                let i = order_idx[k];
                layout_item(
                    &mut bx.children[i],
                    pen,
                    start_cursor,
                    sizes_l[lk] + line_margins[k].0 + line_margins[k].1,
                    ctx,
                    fs,
                    pos_cb,
                );
                let c = &bx.children[i];
                let cfs = font_size_px(&c.style, fs, ctx);
                let mt = side_px(&c.style, "margin-top", main_cb, cfs, ctx, 0.0);
                let mb = side_px(&c.style, "margin-bottom", main_cb, cfs, ctx, 0.0);
                cross_margins[k] = (mt, mb);
                crosses[k] = c.rect.height + mt + mb;
                line_cross = line_cross.max(crosses[k]);
                pen += sizes_l[lk] + line_margins[k].0 + line_margins[k].1 + gap + between;
            }
            line_crosses.push(line_cross);
        }

        // Cross : empilement des lignes — `align-content`
        // distribue le libre quand la taille cross du
        // conteneur est définie (height px ; % sans base
        // = indéfini, lignes soudées en haut).
        let n_lines = lines.len() as f32;
        let mut total_cross = line_crosses.iter().sum::<f32>() + row_gap * (n_lines - 1.0).max(0.0);
        let content_h = css_px(&st, "height", 0.0, fs, ctx);
        let ac = st.get("align-content").unwrap_or("stretch");
        let (lead_c, between_c) = if wrap {
            if let Some(h) = content_h {
                let free_c = (h - total_cross).max(0.0);
                // `stretch` : le libre est réparti sur les
                // lignes (leurs crosses grandissent).
                if ac == "stretch" && free_c > 0.0 && n_lines > 0.0 {
                    let add = free_c / n_lines;
                    for lc in &mut line_crosses {
                        *lc += add;
                    }
                    total_cross += free_c;
                }
                let m = n_lines;
                match ac {
                    "flex-end" => (free_c, 0.0),
                    "center" => (free_c / 2.0, 0.0),
                    "space-between" if lines.len() > 1 => (0.0, free_c / (m - 1.0)),
                    "space-around" => (free_c / m / 2.0, free_c / m),
                    "space-evenly" => (free_c / (m + 1.0), free_c / (m + 1.0)),
                    _ => (0.0, 0.0),
                }
            } else {
                (0.0, 0.0)
            }
        } else {
            (0.0, 0.0)
        };

        // Passe 2 : translation cross par ligne — offset de
        // ligne (align-content / empilement / wrap-reverse)
        // + alignement intra-ligne (align-items/self,
        // stretch, marges cross auto).
        let mut y_off = Vec::with_capacity(lines.len());
        {
            let mut y = start_cursor + lead_c;
            for lc in &line_crosses {
                y_off.push(y);
                y += *lc + row_gap + between_c;
            }
            if wrap_rev {
                // Miroir sur l'axe cross : la 1re ligne part
                // du bas de la zone cross.
                let end = start_cursor + total_cross + lead_c;
                for (li, yo) in y_off.iter_mut().enumerate() {
                    *yo = end - (*yo - start_cursor) - line_crosses[li];
                }
            }
        }
        for (li, line) in lines.iter().enumerate() {
            let line_cross = line_crosses[li];
            for &k in line {
                let i = order_idx[k];
                let c = &mut bx.children[i];
                let mode = items[k].align.as_deref().unwrap_or(&align);
                let explicit_h = c.style.get("height").is_some_and(|v| v != "auto");
                let (top_auto, bot_auto) = (
                    c.style.get("margin-top") == Some("auto"),
                    c.style.get("margin-bottom") == Some("auto"),
                );
                let line_dy = y_off[li] - start_cursor;
                let (mt, mb) = cross_margins[k];
                if mode == "stretch" && !explicit_h {
                    // La margin box remplit la ligne : la
                    // border box = ligne − marges cross.
                    c.rect.height = (line_cross - mt - mb).max(0.0);
                    translate_subtree(c, 0.0, line_dy + mt);
                    continue;
                }
                let inner = if top_auto || bot_auto {
                    let slack = (line_cross - crosses[k]).max(0.0);
                    match (top_auto, bot_auto) {
                        (true, true) => slack / 2.0,
                        (true, false) => slack,
                        _ => 0.0,
                    }
                } else {
                    align_offset(mode, line_cross, crosses[k])
                };
                translate_subtree(c, 0.0, line_dy + inner + mt);
            }
        }
        start_cursor + total_cross
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
            let cfs = font_size_px(&c.style, fs, ctx);
            // Marges de l'axe principal (vertical) : à la
            // charge du conteneur flex — `layout_block` ne
            // gère pas sa propre marge haute (rôle du parent
            // en flux normal).
            let mt = if c.style.get("margin-top") == Some("auto") {
                0.0
            } else {
                side_px(&c.style, "margin-top", main_cb, cfs, ctx, 0.0)
            };
            let mb = if c.style.get("margin-bottom") == Some("auto") {
                0.0
            } else {
                side_px(&c.style, "margin-bottom", main_cb, cfs, ctx, 0.0)
            };
            pen_y += mt;
            let mode = item_flex(c, false, main_cb, fs, ctx)
                .align
                .unwrap_or_else(|| align.clone());
            layout_item(c, content_x, pen_y, content_w, ctx, fs, pos_cb);
            // Cross = horizontal : marges `auto` → centrage.
            let (l_auto, r_auto) = (
                c.style.get("margin-left") == Some("auto"),
                c.style.get("margin-right") == Some("auto"),
            );
            let ml = if l_auto {
                0.0
            } else {
                side_px(&c.style, "margin-left", content_w, cfs, ctx, 0.0)
            };
            let mr = if r_auto {
                0.0
            } else {
                side_px(&c.style, "margin-right", content_w, cfs, ctx, 0.0)
            };
            // `layout_block` a déjà décalé la border box de
            // ml — la margin box est donc à content_x ; on
            // translate du delta d'alignement seulement.
            let outer_w = c.rect.width + ml + mr;
            let slack = (content_w - outer_w).max(0.0);
            let dx = match (l_auto, r_auto) {
                (true, true) => slack / 2.0,
                (true, false) => slack,
                (false, true) => 0.0,
                (false, false) => align_offset(&mode, content_w, outer_w),
            };
            if dx > 0.0 {
                translate_subtree(c, dx, 0.0);
            }
            pen_y += c.rect.height + mb;
        }
        pen_y
    }
}
