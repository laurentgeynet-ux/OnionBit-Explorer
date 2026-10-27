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
//! `justify-content`, le cross en reliquat) ; en `column`,
//! la base main est la hauteur mesurée quand
//! `flex-basis`/`height` sont absents, et `align-content`
//! n'agit que si la taille cross concernée est définie
//! (hauteur px en `row`, largeur toujours définie en
//! `column` ; % sans base = indéfini).

use explorer_css::ComputedStyle;

use crate::block::{
    border_px, box_size_to_content, css_px, font_size_px, layout_anon_block, layout_block,
    raw_length_px, side_px, BlockCtx, FloatCtx,
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
/// `cb_def` = la taille main du conteneur est définie ; une
/// base en `%` ne se résout que si elle l'est (§9.2 : contre
/// une taille indéfinie, la base `%` vaut `content`).
/// `chassis` = padding+bordure main de l'item — `box-sizing`
/// convertit `flex-basis`/`width`/`height` en contenu (R-43).
fn item_flex(
    c: &LayoutBox,
    row: bool,
    cb: f32,
    cb_def: bool,
    chassis: f32,
    fs: f32,
    ctx: &BlockCtx,
) -> ItemFlex {
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
    // §9.2/§10.5 : une longueur en `%` sans base définie
    // résout `auto`/`content`, pas `0`.
    let len_px = |v: &str| {
        if cb_def || !v.trim_end().ends_with('%') {
            raw_length_px(v, cb, fs, ctx)
        } else {
            None
        }
    };
    // Sources CSS (`flex-basis`, `width`/`height`) : tailles
    // spécifiées — `box-sizing` s'applique (R-43). Les mesures
    // intrinsèques (`intrinsic_size_px`, `intrinsic`) sont
    // déjà des tailles de contenu — pas de conversion.
    let css_len = |v: &str| len_px(v).map(|b| box_size_to_content(st, b, chassis));
    let base = basis_raw
        .as_deref()
        .filter(|b| !b.eq_ignore_ascii_case("auto") && !b.eq_ignore_ascii_case("content"))
        .and_then(css_len)
        .or_else(|| crate::intrinsic::intrinsic_size_px(st, "flex-basis", c, cb, fs, ctx))
        .or_else(|| st.get(basis_prop).and_then(css_len))
        .or_else(|| crate::intrinsic::intrinsic_size_px(st, basis_prop, c, cb, fs, ctx))
        .or_else(|| c.intrinsic.map(|i| if row { i.0 } else { i.1 }));
    ItemFlex {
        grow: grow.unwrap_or(0.0).max(0.0),
        shrink: shrink.unwrap_or(1.0).max(0.0),
        base,
        // `align-self:auto` = la valeur calculée **hérite**
        // l'`align-items` du parent (spec §8.3 — `auto` n'est
        // pas un mode à part) : filtré ici, le repli
        // `unwrap_or(&align)` de la passe 2 y retombe.
        align: st
            .get("align-self")
            .filter(|v| v.trim() != "auto")
            .map(str::to_string),
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
#[allow(clippy::too_many_arguments)]
pub(crate) fn layout_item(
    c: &mut LayoutBox,
    ix: f32,
    iy: f32,
    iw: f32,
    ctx: &BlockCtx,
    fs: f32,
    pos_cb: Rect,
    cb_h: Option<f32>,
) {
    if c.kind == BoxKind::AnonymousBlock {
        layout_anon_block(c, ix, iy, iw, ctx, fs, pos_cb, &FloatCtx::default(), cb_h);
    } else {
        // Item flex = contexte de formatage indépendant
        // (§4.1 — idem pour les items grid, même point
        // d'entrée) : registre de flottants vierge ET les
        // marges des descendants restent dans l'item
        // (`fc_root`, §8.3.1). `cb_h` non propagé par
        // défaut (`None` — la hauteur de la ligne flex
        // dépend du contenu, §10.5) ; grid passe la
        // hauteur de sa cellule — l'aire de grille EST
        // le bloc conteneur des % de l'item (§6.5).
        c.fc_root = true;
        layout_block(
            c,
            ix,
            iy,
            iw,
            cb_h,
            ctx,
            fs,
            pos_cb,
            &mut FloatCtx::default(),
        );
    }
}

/// `justify-content` → décalage du premier item + espacement
/// inter-items supplémentaire.
///
/// `free` peut être négatif (items plus larges que le
/// conteneur, `flex-shrink` désactivé) : les alignements
/// bord/centre conservent ce signe — le débordement sort
/// alors par le bord main-start (réfs horiz-003/004) —
/// tandis que les modes distribués (`space-*`) retombent
/// sur `flex-start`, leur alignement de secours.
#[allow(clippy::cast_precision_loss)]
fn justify_offsets(j: &str, free: f32, count: usize) -> (f32, f32) {
    let n = count.max(1) as f32;
    match j {
        // `right`/`end` : alias main-end en écriture
        // horizontale (`direction: rtl` non géré — MVP).
        "flex-end" | "right" | "end" => (free, 0.0),
        "center" => (free / 2.0, 0.0),
        "space-between" if count > 1 && free > 0.0 => (0.0, free / (n - 1.0)),
        "space-around" if free > 0.0 => (free / n / 2.0, free / n),
        "space-evenly" if free > 0.0 => (free / (n + 1.0), free / (n + 1.0)),
        // `flex-start` | `left` | `start` | `normal` |
        // `stretch`, et repli des modes distribués quand le
        // libre est négatif.
        _ => (0.0, 0.0),
    }
}

/// `align-items`/`align-self` → décalage cross de l'item
/// (`stretch`/`baseline` ≈ `flex-start` — MVP).
fn align_offset(mode: &str, line_cross: f32, item_cross: f32) -> f32 {
    match mode {
        // Pas de clamp : un item plus grand que la ligne
        // déborde symétriquement / par main-start (même règle
        // que `justify_offsets`).
        "center" => (line_cross - item_cross) / 2.0,
        "flex-end" => line_cross - item_cross,
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
    h_css: Option<f32>,
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
    // `normal` = comportement `stretch` en flex (spec §8.3).
    let align = {
        let a = st.get_or("align-items", "stretch");
        if a.trim() == "normal" {
            "stretch".to_string()
        } else {
            a.to_string()
        }
    };
    // Hauteur de conteneur définie : `h_css` — résolue par
    // `layout_block` (%, `box-sizing` déjà appliqués, R-43) ;
    // `None` = indéfinie (taille main de l'axe `column` et
    // base des % `row-gap`).
    let h_def = h_css;

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
            let ch = side_px(&c.style, "padding-left", main_cb, cfs, ctx, 0.0)
                + side_px(&c.style, "padding-right", main_cb, cfs, ctx, 0.0)
                + border_px(&c.style, "border-left-width", main_cb, cfs, ctx)
                + border_px(&c.style, "border-right-width", main_cb, cfs, ctx);
            let mut f = item_flex(c, true, main_cb, true, ch, cfs, ctx);
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

        // Padding + bordures axe principal : `flex-basis` et
        // `width` sont des tailles de **contenu** internes —
        // `box-sizing: border-box` y est converti amont (R-43).
        // Espace libre, pistes et retraits se mesurent en
        // border-box (+ marges = margin-box).
        let mut extras: Vec<f32> = Vec::with_capacity(n);
        for &i in &order_idx {
            let c = &bx.children[i];
            let cfs = font_size_px(&c.style, fs, ctx);
            extras.push(
                side_px(&c.style, "padding-left", main_cb, cfs, ctx, 0.0)
                    + side_px(&c.style, "padding-right", main_cb, cfs, ctx, 0.0)
                    + border_px(&c.style, "border-left-width", main_cb, cfs, ctx)
                    + border_px(&c.style, "border-right-width", main_cb, cfs, ctx),
            );
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
                let outer = items[k].base.unwrap_or(0.0) + extras[k] + margins[k].0 + margins[k].1;
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
                used_base += items[k].base.unwrap_or(0.0) + extras[k] + margins[k].0 + margins[k].1;
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
                        // §4.5/§9.9.4 : le shrink est planché par
                        // la taille min automatique de l'item —
                        // `min-width` défini (longueur, %, ou
                        // mot-clé intrinsèque) sinon min-content.
                        // Sans plancher, le seul item shrinkable
                        // absorbait TOUT le dépassement et
                        // s'effondrait à 0 px (vol 32 todoist :
                        // `.details` seul shrinkable face à
                        // `.illustration{flex-shrink:0}` de
                        // 1590 px → colonne héro à 0, texte en
                        // rivière min-content). Gel-itération
                        // complète documentée hors MVP : un clamp
                        // simple suffit au cas courant.
                        let c = &bx.children[order_idx[k]];
                        let cfs = font_size_px(&c.style, fs, ctx);
                        let floor = match c.style.get("min-width").map(|v| v.trim()) {
                            // `auto`/absent : minimum = min-content
                            // du contenu, le `width` propre ignoré.
                            None | Some("") | Some("auto") => {
                                crate::intrinsic::min_content_size_px(c, cfs, ctx)
                            }
                            _ => crate::intrinsic::intrinsic_size_px(
                                &c.style, "min-width", c, main_cb, cfs, ctx,
                            )
                            .or_else(|| css_px(&c.style, "min-width", main_cb, cfs, ctx))
                            .unwrap_or(0.0),
                        };
                        (b + free * (items[k].shrink * b) / denom).max(floor)
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
                    .map(|(&k, s)| s + extras[k] + margins[k].0 + margins[k].1)
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
            // Marges résolues + taille main cible écrites en px
            // sur le style de l'item : `layout_block` les
            // applique une fois — la border box obtenue =
            // cible + padding + bordures (content-box). Sur
            // item `border-box` on écrit la valeur border-box
            // équivalente : `layout_block` reconvertit en
            // contenu et retombe exactement sur la cible (R-43).
            for (lk, &k) in line.iter().enumerate() {
                let c = &mut bx.children[order_idx[k]];
                c.style
                    .set("margin-left", &format!("{}px", line_margins[k].0));
                c.style
                    .set("margin-right", &format!("{}px", line_margins[k].1));
                let w = if c.style.get("box-sizing") == Some("border-box") {
                    sizes_l[lk] + extras[k]
                } else {
                    sizes_l[lk]
                };
                c.style.set("width", &format!("{w}px"));
            }
            let used: f32 = line
                .iter()
                .zip(&sizes_l)
                .map(|(&k, s)| s + extras[k] + line_margins[k].0 + line_margins[k].1)
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
            // comptent dans la taille de ligne). Pas de
            // clamp : un `lead` négatif (débordement)
            // pousse les items hors du bord main-start.
            let mut pen = content_x + lead;
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
                    sizes_l[lk] + extras[k] + line_margins[k].0 + line_margins[k].1,
                    ctx,
                    fs,
                    pos_cb,
                    None,
                );
                let c = &bx.children[i];
                let cfs = font_size_px(&c.style, fs, ctx);
                let mt = side_px(&c.style, "margin-top", main_cb, cfs, ctx, 0.0);
                let mb = side_px(&c.style, "margin-bottom", main_cb, cfs, ctx, 0.0);
                cross_margins[k] = (mt, mb);
                crosses[k] = c.rect.height + mt + mb;
                line_cross = line_cross.max(crosses[k]);
                pen +=
                    sizes_l[lk] + extras[k] + line_margins[k].0 + line_margins[k].1 + gap + between;
            }
            line_crosses.push(line_cross);
        }

        // Cross : empilement des lignes — `align-content`
        // distribue le libre quand la taille cross du
        // conteneur est définie (height px ; % sans base
        // = indéfini, lignes soudées en haut).
        let n_lines = lines.len() as f32;
        let mut total_cross = line_crosses.iter().sum::<f32>() + row_gap * (n_lines - 1.0).max(0.0);
        let ac = st.get("align-content").unwrap_or("stretch");
        let (lead_c, between_c) = if wrap {
            if let Some(h) = h_def {
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
        // §9.4 — conteneur mono-ligne (`nowrap`) à hauteur
        // définie : la ligne prend la taille cross **interne**
        // du conteneur, pas la hauteur max des items. C'est
        // ce qui donne sens à `align-items`/`stretch` dans
        // une boîte à `height` fixe.
        if !wrap {
            if let Some(h) = h_def {
                line_crosses.fill(h);
                total_cross = h;
            }
        }

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
                    // border box = ligne − marges cross,
                    // planchée au `min-height` de l'item
                    // (§9.8 — la taille stretchée respecte
                    // les bornes min/max).
                    let cfs = font_size_px(&c.style, fs, ctx);
                    let mn = side_px(&c.style, "min-height", h_def.unwrap_or(0.0), cfs, ctx, 0.0);
                    c.rect.height = (line_cross - mt - mb).max(mn).max(0.0);
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
        // ---- Colonne : main = vertical, cross = horizontal.
        // Même plan que la branche row, axes permutés :
        // découpe en colonnes (`wrap` borne la hauteur
        // définie), tailles flex + marges `auto` +
        // `justify-content` par colonne, puis `align-content`
        // répartit les colonnes sur la largeur cross.
        let wrap_mode = st.get("flex-wrap").unwrap_or("nowrap");
        let wrap = wrap_mode != "nowrap";
        let wrap_rev = wrap_mode == "wrap-reverse";
        // `row-gap` = écart main entre items d'une colonne
        // (% sur la hauteur) ; `column-gap` = écart cross
        // entre colonnes (% sur la largeur).
        let gap = gap_px(&st, false, h_def.unwrap_or(0.0), fs, ctx);
        let col_gap = gap_px(&st, true, content_w, fs, ctx);
        // Sans hauteur définie : une seule colonne, espace
        // libre nul (le conteneur épouse le contenu).
        let main_h = h_def.unwrap_or(0.0);

        let mut items: Vec<ItemFlex> = Vec::with_capacity(n);
        let mut margins: Vec<(f32, f32)> = Vec::with_capacity(n);
        let mut main_auto = vec![(false, false); n];
        let mut cross_auto = vec![(false, false); n];
        let mut extras: Vec<f32> = Vec::with_capacity(n);
        for (k, &i) in order_idx.iter().enumerate() {
            let c = &mut bx.children[i];
            let cfs = font_size_px(&c.style, fs, ctx);
            let ch = side_px(&c.style, "padding-top", content_w, cfs, ctx, 0.0)
                + side_px(&c.style, "padding-bottom", content_w, cfs, ctx, 0.0)
                + border_px(&c.style, "border-top-width", content_w, cfs, ctx)
                + border_px(&c.style, "border-bottom-width", content_w, cfs, ctx);
            items.push(item_flex(c, false, main_h, h_def.is_some(), ch, cfs, ctx));
            main_auto[k] = (
                c.style.get("margin-top") == Some("auto"),
                c.style.get("margin-bottom") == Some("auto"),
            );
            cross_auto[k] = (
                c.style.get("margin-left") == Some("auto"),
                c.style.get("margin-right") == Some("auto"),
            );
            // % de marges : toujours sur la **largeur** du
            // bloc conteneur (§8.3), pas sur l'axe main.
            margins.push((
                if main_auto[k].0 {
                    0.0
                } else {
                    side_px(&c.style, "margin-top", content_w, cfs, ctx, 0.0)
                },
                if main_auto[k].1 {
                    0.0
                } else {
                    side_px(&c.style, "margin-bottom", content_w, cfs, ctx, 0.0)
                },
            ));
            extras.push(
                side_px(&c.style, "padding-top", content_w, cfs, ctx, 0.0)
                    + side_px(&c.style, "padding-bottom", content_w, cfs, ctx, 0.0)
                    + border_px(&c.style, "border-top-width", content_w, cfs, ctx)
                    + border_px(&c.style, "border-bottom-width", content_w, cfs, ctx),
            );
            // Marges cross `auto` : figées à 0 pendant la
            // mesure — `layout_block` les résoudrait sur la
            // largeur du conteneur au lieu de la colonne.
            if cross_auto[k].0 {
                c.style.set("margin-left", "0px");
            }
            if cross_auto[k].1 {
                c.style.set("margin-right", "0px");
            }
            // Axe cross horizontal (§9.5 taille hypothétique) :
            // hors `stretch`, un item `column` est « fit-content »
            // — sa boîte rétrécit sur le contenu puis
            // `align-items` la place. Sans ça tout item gardait
            // `content_w` de large (un bloc `width:auto` remplit
            // son CB) et le centrage n'avait aucun effet —
            // vol 18 quora : bouton « Réessayer » pleine largeur
            // au lieu d'être centré sous `align-items:center`.
            let mode = items[k].align.as_deref().unwrap_or(&align);
            if mode != "stretch" && c.style.get("width").is_none_or(|v| v == "auto") {
                let ch_h = side_px(&c.style, "padding-left", content_w, cfs, ctx, 0.0)
                    + side_px(&c.style, "padding-right", content_w, cfs, ctx, 0.0)
                    + border_px(&c.style, "border-left-width", content_w, cfs, ctx)
                    + border_px(&c.style, "border-right-width", content_w, cfs, ctx);
                let ml = side_px(&c.style, "margin-left", content_w, cfs, ctx, 0.0);
                let mr = side_px(&c.style, "margin-right", content_w, cfs, ctx, 0.0);
                let avail = (content_w - ml - mr - ch_h).max(0.0);
                let (mc, nc) = (
                    crate::intrinsic::max_content_px(c, cfs, ctx),
                    crate::intrinsic::min_content_px(c, cfs, ctx),
                );
                let fit = avail.min(mc).max(nc);
                let w = if c.style.get("box-sizing") == Some("border-box") {
                    fit + ch_h
                } else {
                    fit
                };
                c.style.set("width", &format!("{w}px"));
            }
        }

        // Passe 0 : pose mesurée — les hauteurs obtenues
        // servent de base quand `flex-basis`/`height` ne sont
        // pas déclarés (contenu à hauteur naturelle).
        for &i in &order_idx {
            layout_item(
                &mut bx.children[i],
                content_x,
                start_cursor,
                content_w,
                ctx,
                fs,
                pos_cb,
                None,
            );
        }
        let mut bases: Vec<f32> = Vec::with_capacity(n);
        // Hauteur de contenu mesurée (passe 0) — taille cible
        // d'un item `grow` quand l'espace libre est indéfini.
        let mut measured: Vec<f32> = Vec::with_capacity(n);
        let mut crosses: Vec<f32> = vec![0.0; n];
        let mut cross_margins: Vec<(f32, f32)> = vec![(0.0, 0.0); n];
        for (k, &i) in order_idx.iter().enumerate() {
            let c = &bx.children[i];
            let cfs = font_size_px(&c.style, fs, ctx);
            let m = (c.rect.height - extras[k]).max(0.0);
            measured.push(m);
            bases.push(items[k].base.unwrap_or(m));
            let ml = side_px(&c.style, "margin-left", content_w, cfs, ctx, 0.0);
            let mr = side_px(&c.style, "margin-right", content_w, cfs, ctx, 0.0);
            cross_margins[k] = (ml, mr);
            crosses[k] = c.rect.width + ml + mr;
        }

        // Découpe en colonnes : `wrap` borne la hauteur
        // définie (`h_def`) — taille hypothétique §9.5.
        let mut lines: Vec<Vec<usize>> = Vec::new();
        {
            let mut cur: Vec<usize> = Vec::new();
            let mut cur_h = 0.0f32;
            for k in 0..n {
                let outer = bases[k] + extras[k] + margins[k].0 + margins[k].1;
                let need = outer + if cur.is_empty() { 0.0 } else { gap };
                if wrap && h_def.is_some() && !cur.is_empty() && cur_h + need > main_h + 0.01 {
                    lines.push(std::mem::take(&mut cur));
                    cur_h = 0.0;
                }
                cur_h += if cur.is_empty() { outer } else { outer + gap };
                cur.push(k);
            }
            if !cur.is_empty() {
                lines.push(cur);
            }
        }

        // Passe 1 : tailles main (grow/shrink sur la
        // hauteur), marges `auto`, `justify-content` — par
        // colonne. `pen_y` marche depuis le haut.
        let mut line_crosses = Vec::with_capacity(lines.len());
        let mut col_used = Vec::with_capacity(lines.len());
        for line in &lines {
            let lgaps = gap * (line.len() as f32 - 1.0).max(0.0);
            let mut used_base = lgaps;
            for &k in line {
                used_base += bases[k] + extras[k] + margins[k].0 + margins[k].1;
            }
            // Hauteur indéfinie → pas d'espace libre à
            // distribuer (0, jamais négatif : le contenu
            // définit la taille).
            let free = if h_def.is_some() {
                main_h - used_base
            } else {
                0.0
            };
            let total_grow: f32 = line.iter().map(|&k| items[k].grow).sum();
            let mut sizes_l: Vec<f32> = Vec::with_capacity(line.len());
            if h_def.is_none() {
                // §9.9.2 : espace libre indéfini — les items à
                // `grow` positif prennent leur taille de
                // contenu mesurée (un `flex:1`/`flex-basis:0px`
                // garde sa hauteur naturelle au lieu de
                // s'effondrer et de recouvrir l'item suivant —
                // terrain : pied de rust-lang.org peint sur le
                // héros). Les autres gardent leur base.
                for &k in line {
                    sizes_l.push(if items[k].grow > 0.0 {
                        measured[k]
                    } else {
                        bases[k]
                    });
                }
            } else if free > 0.0 && total_grow > 0.0 {
                for &k in line {
                    sizes_l.push(bases[k] + free * items[k].grow / total_grow);
                }
            } else {
                let denom: f32 = line.iter().map(|&k| items[k].shrink * bases[k]).sum();
                for &k in line {
                    let b = bases[k];
                    sizes_l.push(if denom > 0.0 && free < 0.0 {
                        (b + free * (items[k].shrink * b) / denom).max(0.0)
                    } else {
                        b
                    });
                }
            }
            // Marges `auto` main : part égale du libre avant
            // `justify-content` (§8.1).
            let free_m = if h_def.is_some() {
                main_h
                    - lgaps
                    - line
                        .iter()
                        .zip(&sizes_l)
                        .map(|(&k, s)| s + extras[k] + margins[k].0 + margins[k].1)
                        .sum::<f32>()
            } else {
                0.0
            };
            let mut line_margins = margins.clone();
            if free_m > 0.0 {
                let n_auto_l: usize = line
                    .iter()
                    .map(|&k| usize::from(main_auto[k].0) + usize::from(main_auto[k].1))
                    .sum();
                if n_auto_l > 0 {
                    let share = free_m / n_auto_l as f32;
                    for &k in line {
                        if main_auto[k].0 {
                            line_margins[k].0 = share;
                        }
                        if main_auto[k].1 {
                            line_margins[k].1 = share;
                        }
                    }
                }
            }
            // Hauteur cible écrite quand elle diffère de la
            // base mesurée — `layout_block` reproduit alors
            // la border box exacte (content-box §9.7 ; valeur
            // border-box équivalente pour `border-box`, R-43).
            for (lk, &k) in line.iter().enumerate() {
                if (sizes_l[lk] - bases[k]).abs() > 0.01 {
                    let i = order_idx[k];
                    let c = &mut bx.children[i];
                    let h = if c.style.get("box-sizing") == Some("border-box") {
                        sizes_l[lk] + extras[k]
                    } else {
                        sizes_l[lk]
                    };
                    c.style.set("height", &format!("{h}px"));
                    layout_item(c, content_x, start_cursor, content_w, ctx, fs, pos_cb, None);
                }
            }
            let used: f32 = line
                .iter()
                .zip(&sizes_l)
                .map(|(&k, s)| s + extras[k] + line_margins[k].0 + line_margins[k].1)
                .sum::<f32>()
                + lgaps;
            // `column-reverse` : l'axe main part du bas —
            // miroir du lead, espacement inclus (cf. row).
            let (lead_j, between) = justify_offsets(
                &justify,
                if h_def.is_some() { main_h - used } else { 0.0 },
                line.len(),
            );
            let between_total = (line.len() as f32 - 1.0).max(0.0) * between;
            let lead = if axis.reverse {
                if h_def.is_some() {
                    main_h - used - between_total - lead_j
                } else {
                    0.0
                }
            } else {
                lead_j
            };
            let mut pen_y = start_cursor + lead;
            let lay_order: Vec<usize> = if axis.reverse {
                (0..line.len()).rev().collect()
            } else {
                (0..line.len()).collect()
            };
            let mut col_cross = 0.0f32;
            for &lk in &lay_order {
                let k = line[lk];
                let i = order_idx[k];
                let c = &mut bx.children[i];
                // La margin box vise `pen_y` — l'item posé à
                // `start_cursor` translate du delta.
                let dy = pen_y + line_margins[k].0 - c.rect.y;
                translate_subtree(c, 0.0, dy);
                col_cross = col_cross.max(crosses[k]);
                pen_y +=
                    sizes_l[lk] + extras[k] + line_margins[k].0 + line_margins[k].1 + gap + between;
            }
            line_crosses.push(col_cross);
            col_used.push(used + lead);
        }

        // Cross : `align-content` répartit les colonnes sur
        // la largeur du conteneur (toujours définie).
        let n_lines = lines.len() as f32;
        let mut total_cross = line_crosses.iter().sum::<f32>() + col_gap * (n_lines - 1.0).max(0.0);
        let ac = st.get("align-content").unwrap_or("stretch");
        let (lead_c, between_c) = if wrap {
            let free_c = (content_w - total_cross).max(0.0);
            if ac == "stretch" && free_c > 0.0 && n_lines > 0.0 {
                let add = free_c / n_lines;
                for lc in &mut line_crosses {
                    *lc += add;
                }
                total_cross += free_c;
            }
            match ac {
                "flex-end" => (free_c, 0.0),
                "center" => (free_c / 2.0, 0.0),
                "space-between" if lines.len() > 1 => (0.0, free_c / (n_lines - 1.0)),
                "space-around" => (free_c / n_lines / 2.0, free_c / n_lines),
                "space-evenly" => (free_c / (n_lines + 1.0), free_c / (n_lines + 1.0)),
                _ => (0.0, 0.0),
            }
        } else {
            (0.0, 0.0)
        };
        // §9.4 : conteneur mono-colonne (`nowrap`) — la
        // colonne prend la largeur cross interne du
        // conteneur (`align-items`/`stretch` y puisent).
        if !wrap {
            line_crosses.fill(content_w);
            total_cross = content_w;
        }

        // Passe 2 : translation cross par colonne — offset
        // (align-content / wrap-reverse) + alignement
        // intra-colonne (align-items/self, stretch, marges
        // cross auto).
        let mut x_off = Vec::with_capacity(lines.len());
        {
            let mut x = content_x + lead_c;
            for lc in &line_crosses {
                x_off.push(x);
                x += *lc + col_gap + between_c;
            }
            if wrap_rev {
                // Miroir cross : la 1re colonne part du bord
                // droit de la zone cross.
                let end = content_x + total_cross + lead_c;
                for (li, xo) in x_off.iter_mut().enumerate() {
                    *xo = end - (*xo - content_x) - line_crosses[li];
                }
            }
        }
        for (li, line) in lines.iter().enumerate() {
            let col_cross = line_crosses[li];
            for &k in line {
                let i = order_idx[k];
                let c = &mut bx.children[i];
                let mode = items[k].align.as_deref().unwrap_or(&align);
                let (l_auto, r_auto) = cross_auto[k];
                let (ml, mr) = cross_margins[k];
                let col_dx = x_off[li] - content_x;
                let outer_w = crosses[k];
                if mode == "stretch" && c.style.get("width").is_none_or(|v| v == "auto") {
                    // La margin box remplit la colonne :
                    // border box = colonne − marges cross.
                    c.rect.width = (col_cross - ml - mr).max(0.0);
                    translate_subtree(c, col_dx, 0.0);
                    continue;
                }
                let inner = if l_auto || r_auto {
                    let slack = (col_cross - outer_w).max(0.0);
                    match (l_auto, r_auto) {
                        (true, true) => slack / 2.0,
                        (true, false) => slack,
                        _ => 0.0,
                    }
                } else {
                    align_offset(mode, col_cross, outer_w)
                };
                translate_subtree(c, col_dx + inner, 0.0);
            }
        }
        // Fond du flux = bas de la plus haute colonne.
        start_cursor + col_used.iter().copied().fold(0.0f32, f32::max)
    }
}
