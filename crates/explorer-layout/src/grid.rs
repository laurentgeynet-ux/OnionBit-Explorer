// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! CSS Grid — MVP grille explicite + placement auto.
//!
//! Couvert : `display: grid`/`inline-grid`,
//! `grid-template-columns`/`grid-template-rows` (longueurs,
//! `%`, `fr`, `auto`, `repeat(n, …)`, `minmax(a, b)` → `b`),
//! `gap`/`row-gap`/`column-gap`, `grid-column`/`grid-row`
//! (`a`, `a / b`, `a / span n`, `span n`), placement
//! automatique row-major avec balayage avant, `order`,
//! `justify-items`/`align-items` (décalage dans la cellule,
//! `stretch` = défaut pleine cellule).
//!
//! Écarts connus : pas de `grid-template-areas` ni de lignes
//! nommées, pas de placement dense/backfill, `fr` sur l'axe
//! vertical ≈ `auto` (hauteur de conteneur inconnue au flow),
//! `auto` colonne = max des bases mesurées (pas max-content
//! intégral), indices négatifs ignorés.

use crate::block::{css_px, font_size_px, raw_length_px, BlockCtx};
use crate::flex::{gap_px, layout_item, translate_subtree};
use crate::LayoutBox;

/// Une piste de `grid-template-*`.
enum Track {
    /// Longueur brute (px, %, calc…), résolue à l'usage.
    Raw(String),
    /// Fraction de l'espace libre.
    Fr(f32),
    /// `auto` — mesure sur le contenu placé.
    Auto,
}

/// Tokens de piste séparés par espaces de niveau 0
/// (parenthèses de `repeat(`/`minmax(` respectées).
fn track_tokens(raw: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut cur = String::new();
    for c in raw.trim().chars() {
        match c {
            '(' => {
                depth += 1;
                cur.push(c);
            }
            ')' => {
                depth = depth.saturating_sub(1);
                cur.push(c);
            }
            c if c.is_whitespace() && depth == 0 => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            _ => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Parse `grid-template-*` en pistes — `repeat(n, …)`
/// expansé (borné à 512 pistes), `minmax` → borne max.
#[allow(clippy::cast_precision_loss)]
fn parse_tracks(raw: &str, fs: f32, ctx: &BlockCtx, cb: f32) -> Vec<Track> {
    let mut out = Vec::new();
    for tok in track_tokens(raw) {
        let t = tok.to_ascii_lowercase();
        if let Some(inner) = t.strip_prefix("repeat(").and_then(|s| s.strip_suffix(')')) {
            // `repeat(n, a b c)` ou `repeat(auto-fill…)` —
            // seul le compte numérique est supporté.
            if let Some((n, list)) = inner.split_once(',') {
                if let Ok(n) = n.trim().parse::<usize>() {
                    for sub in parse_tracks(list.trim(), fs, ctx, cb) {
                        for _ in 0..n.min(512 / out.len().max(1)) {
                            out.push(match &sub {
                                Track::Raw(r) => Track::Raw(r.clone()),
                                Track::Fr(f) => Track::Fr(*f),
                                Track::Auto => Track::Auto,
                            });
                        }
                    }
                }
            }
            continue;
        }
        if let Some(inner) = t.strip_prefix("minmax(").and_then(|s| s.strip_suffix(')')) {
            // Borne supérieure — le côté droit de la virgule.
            let max = inner.rsplit(',').next().unwrap_or("auto").trim();
            out.push(if max == "auto" {
                Track::Auto
            } else {
                parse_tracks(max, fs, ctx, cb)
                    .into_iter()
                    .next()
                    .unwrap_or(Track::Auto)
            });
            continue;
        }
        if let Some(f) = t
            .strip_suffix("fr")
            .and_then(|n| n.trim().parse::<f32>().ok())
        {
            out.push(Track::Fr(f.max(0.0)));
        } else if t == "auto" {
            out.push(Track::Auto);
        } else {
            // Vérifiable ? sinon la piste tombe `auto`.
            out.push(if raw_length_px(&t, cb, fs, ctx).is_some() {
                Track::Raw(tok)
            } else {
                Track::Auto
            });
        }
        if out.len() > 512 {
            break;
        }
    }
    out
}

/// `grid-column`/`grid-row` → `(début 0-based, span)` —
/// `a`, `a / b`, `a / span n`, `span n`. `None` = auto.
fn placement(raw: Option<&str>) -> (Option<usize>, usize) {
    let Some(raw) = raw else {
        return (None, 1);
    };
    let parts: Vec<&str> = raw.split('/').map(str::trim).collect();
    let one = |s: &str| -> (Option<usize>, Option<usize>) {
        if let Some(n) = s.strip_prefix("span ").and_then(|v| v.trim().parse().ok()) {
            return (None, Some(n));
        }
        (
            s.parse::<usize>().ok().filter(|v| *v > 0).map(|v| v - 1),
            None,
        )
    };
    let (a0, s0) = one(parts.first().unwrap_or(&""));
    if parts.len() == 1 {
        return (a0, s0.unwrap_or(1));
    }
    let (b0, s1) = one(parts[1]);
    let span = s1.or(s0).unwrap_or_else(|| match (a0, b0) {
        (Some(a), Some(b)) if b > a => b - a,
        _ => 1,
    });
    (a0.or(b0.map(|b| b.saturating_sub(span))), span.max(1))
}

/// Largeur px des pistes colonnes : fixes résolues, `auto` =
/// max des bases d'items placés, `fr` = part du libre.
#[allow(clippy::cast_precision_loss)]
fn col_widths(
    tracks: &[Track],
    items_base: &[(usize, usize, f32)],
    gap: f32,
    cb: f32,
    fs: f32,
    ctx: &BlockCtx,
) -> Vec<f32> {
    let mut w: Vec<f32> = tracks
        .iter()
        .map(|t| match t {
            Track::Raw(r) => raw_length_px(r, cb, fs, ctx).unwrap_or(0.0),
            _ => 0.0,
        })
        .collect();
    // `auto` : max des bases mesurées des items qui s'y posent.
    for &(col, span, base) in items_base {
        if span == 1 && col < w.len() && matches!(tracks[col], Track::Auto) {
            w[col] = w[col].max(base);
        }
    }
    let free = (cb - w.iter().sum::<f32>() - gap * w.len().saturating_sub(1) as f32).max(0.0);
    let total_fr: f32 = tracks
        .iter()
        .map(|t| match t {
            Track::Fr(f) => *f,
            _ => 0.0,
        })
        .sum();
    if total_fr > 0.0 {
        for (i, t) in tracks.iter().enumerate() {
            if let Track::Fr(f) = t {
                w[i] = free * f / total_fr;
            }
        }
    }
    w
}

/// Flux grid des enfants de `bx` — appelé par `layout_block`.
/// Retourne le curseur final (bas du contenu).
#[allow(
    clippy::too_many_arguments,
    clippy::too_many_lines,
    clippy::cast_precision_loss
)]
pub(crate) fn flow_grid_children(
    bx: &mut LayoutBox,
    content_x: f32,
    start_cursor: f32,
    content_w: f32,
    ctx: &BlockCtx,
    fs: f32,
    pos_cb: crate::Rect,
) -> f32 {
    let st = bx.style.clone();
    let gap_c = gap_px(&st, true, content_w, fs, ctx);
    let gap_r = gap_px(&st, false, content_w, fs, ctx);

    let mut col_tracks = st
        .get("grid-template-columns")
        .map_or_else(Vec::new, |v| parse_tracks(v, fs, ctx, content_w));
    if col_tracks.is_empty() {
        col_tracks.push(Track::Auto); // une colonne implicite
    }
    let row_tracks = st
        .get("grid-template-rows")
        .map_or_else(Vec::new, |v| parse_tracks(v, fs, ctx, content_w));

    // Ordre : `order` puis arbre (comme flex).
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

    // Placement : `(col, span_c, row, span_r)` par item —
    // auto = premier slot libre en balayage row-major.
    let ncols = col_tracks.len();
    let mut occupancy: Vec<Vec<bool>> = Vec::new();
    let free_at = |occ: &[Vec<bool>], r: usize, c: usize, span: usize| -> bool {
        c + span <= ncols
            && (0..span).all(|k| {
                occ.get(r)
                    .is_none_or(|row| !row.get(c + k).copied().unwrap_or(false))
            })
    };
    let mut slots: Vec<(usize, usize, usize, usize)> = Vec::with_capacity(order_idx.len());
    let (mut auto_col, mut auto_row) = (0usize, 0usize);
    for &i in &order_idx {
        let cst = &bx.children[i].style;
        let (start_c, span_c) = placement(cst.get("grid-column"));
        let (start_r, span_r) = placement(cst.get("grid-row"));
        let span_c = span_c.min(ncols).max(1);
        let (mut col, mut row);
        if let Some(c0) = start_c {
            col = c0.min(ncols - span_c.max(1));
            row = start_r.unwrap_or(0);
            while !free_at(&occupancy, row, col, span_c) {
                row += 1;
                if row > 4096 {
                    break; // borne hostile
                }
            }
        } else {
            // Balayage avant depuis le curseur auto.
            col = auto_col;
            row = start_r.unwrap_or(auto_row);
            loop {
                if col + span_c > ncols {
                    col = 0;
                    row += 1;
                }
                if row > 4096 || free_at(&occupancy, row, col, span_c) {
                    break;
                }
                col += 1;
            }
        }
        while occupancy.len() <= row + span_r {
            occupancy.push(vec![false; ncols]);
        }
        for dr in 0..span_r.max(1) {
            for dc in 0..span_c {
                occupancy[row + dr][col + dc] = true;
            }
        }
        slots.push((col, span_c, row, span_r));
        auto_col = col + span_c;
        auto_row = row;
        if auto_col >= ncols {
            auto_col = 0;
            auto_row += 1;
        }
    }

    // Base de mesure des items (auto colonne) : `width`
    // explicite, intrinsèque, texte direct.
    let items_base: Vec<(usize, usize, f32)> = slots
        .iter()
        .zip(&order_idx)
        .map(|(&(c, span, _, _), &i)| {
            let ch = &bx.children[i];
            let cfs = font_size_px(&ch.style, fs, ctx);
            let base = css_px(&ch.style, "width", content_w, cfs, ctx)
                .or_else(|| ch.intrinsic.map(|v| v.0))
                .or_else(|| crate::flex::text_content_px(ch, cfs, ctx))
                .unwrap_or(0.0);
            (c, span, base)
        })
        .collect();
    let widths = col_widths(&col_tracks, &items_base, gap_c, content_w, fs, ctx);
    let mut x_off = Vec::with_capacity(widths.len());
    let mut acc = content_x;
    for w in &widths {
        x_off.push(acc);
        acc += w + gap_c;
    }

    // Passe 1 : pose à y=0 pour mesurer la hauteur naturelle.
    let mut heights: Vec<f32> = Vec::with_capacity(slots.len());
    for (k, &i) in order_idx.iter().enumerate() {
        let (c, span, _, _) = slots[k];
        let cell_w = widths[c..(c + span).min(widths.len())].iter().sum::<f32>()
            + gap_c * (span.saturating_sub(1)) as f32;
        layout_item(
            &mut bx.children[i],
            x_off[c],
            0.0,
            cell_w.max(0.0),
            ctx,
            fs,
            pos_cb,
        );
        heights.push(bx.children[i].rect.height);
    }

    // Hauteurs de lignes : piste fixe → sa valeur ; `fr`/`auto`
    // → max des items de la ligne (fr vertical ≈ auto, MVP).
    let nrows = slots
        .iter()
        .map(|&(_, _, r, s)| r + s)
        .max()
        .unwrap_or(0)
        .max(row_tracks.len());
    let mut row_h = vec![0.0f32; nrows];
    for (ri, h) in row_h.iter_mut().enumerate() {
        if let Some(Track::Raw(raw)) = row_tracks.get(ri) {
            *h = raw_length_px(raw, content_w, fs, ctx).unwrap_or(0.0);
        }
    }
    for (k, &(_, _, r, span_r)) in slots.iter().enumerate() {
        if span_r == 1 && row_h.get(r).copied().unwrap_or(0.0) < heights[k] {
            let dominated = row_tracks
                .get(r)
                .is_some_and(|t| matches!(t, Track::Raw(_)));
            if !dominated {
                row_h[r] = row_h[r].max(heights[k]);
            }
        }
    }
    let mut y_off = Vec::with_capacity(nrows);
    let mut y = start_cursor;
    for h in &row_h {
        y_off.push(y);
        y += h + gap_r;
    }

    // Passe 2 : translation à la ligne + ajustements
    // justify-items/align-items dans la cellule.
    let ji = st.get_or("justify-items", "stretch").to_string();
    let ai = st.get_or("align-items", "stretch").to_string();
    for (k, &i) in order_idx.iter().enumerate() {
        let (c, span_cols, r, span_rows) = slots[k];
        let cell_w = widths[c..(c + span_cols).min(widths.len())]
            .iter()
            .sum::<f32>()
            + gap_c * (span_cols.saturating_sub(1)) as f32;
        let cell_h = row_h[r..(r + span_rows).min(row_h.len())]
            .iter()
            .sum::<f32>()
            + gap_r * (span_rows.saturating_sub(1)) as f32;
        let jmode = bx.children[i]
            .style
            .get("justify-self")
            .map_or(ji.as_str(), |v| v);
        let amode = bx.children[i]
            .style
            .get("align-self")
            .map_or(ai.as_str(), |v| v);
        let dx = match jmode {
            "center" => (cell_w - bx.children[i].rect.width).max(0.0) / 2.0,
            "end" | "flex-end" => (cell_w - bx.children[i].rect.width).max(0.0),
            _ => 0.0,
        };
        let dy = match amode {
            "center" => (cell_h - bx.children[i].rect.height).max(0.0) / 2.0,
            "end" | "flex-end" => (cell_h - bx.children[i].rect.height).max(0.0),
            _ => 0.0,
        };
        translate_subtree(&mut bx.children[i], dx, y_off[r] + dy);
    }
    y_off.last().copied().unwrap_or(start_cursor) + row_h.last().copied().unwrap_or(0.0)
}
