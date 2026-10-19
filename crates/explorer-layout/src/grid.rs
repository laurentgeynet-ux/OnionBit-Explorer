// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! CSS Grid — MVP grille explicite + placement auto.
//!
//! Couvert : `display: grid`/`inline-grid`,
//! `grid-template-columns`/`grid-template-rows` (longueurs,
//! `%`, `fr`, `auto`, `repeat(n, …)`, `minmax(min, max)` à
//! deux bornes — R-5), **noms de lignes `[n]`**,
//! `grid-template-areas` + `grid-area` (nom ou `r/c/r2/c2`),
//! `grid-auto-flow` (`row`/`column`/`dense`), `gap`,
//! `grid-column`/`grid-row` (`a`, `a / b`, `a / span n`,
//! `span n`, noms), placement automatique row-major avec
//! balayage avant, `order`, `justify-items`/`align-items`
//! (décalage dans la cellule, `stretch` = défaut).
//!
//! Écarts connus : `fr` sur l'axe vertical ≈ `auto`
//! (hauteur de conteneur inconnue au flow), `auto` colonne =
//! max des bases mesurées (max-content réel depuis R-3, hors
//! spans multi-pistes), `minmax(min, fr)` borné en une passe
//! (pas de redistribution), indices négatifs et `repeat(
//! auto-fill|fit)` ignorés, `subgrid` hors scope.

use crate::block::{css_px, font_size_px, raw_length_px, BlockCtx};
use crate::flex::{gap_px, layout_item, translate_subtree};
use crate::LayoutBox;

/// Une piste de `grid-template-*`.
#[derive(Clone)]
enum Track {
    /// Longueur brute (px, %, calc…), résolue à l'usage.
    Raw(String),
    /// Fraction de l'espace libre.
    Fr(f32),
    /// `auto` — mesure sur le contenu placé.
    Auto,
    /// `minmax(min, max)` — le min est une longueur brute
    /// (borne basse), le max une piste simple (R-5 : les
    /// `fr` mixtes sont résolus puis bornés par le min).
    MinMax(String, Box<Track>),
}

/// Nom de ligne → indice de ligne (bord de piste, 0-based).
type LineNames = Vec<(String, usize)>;

/// Tokens de piste séparés par espaces de niveau 0
/// (parenthèses de `repeat(`/`minmax(` et crochets de
/// noms de lignes respectés — `[a b]` reste un jeton).
fn track_tokens(raw: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut cur = String::new();
    for c in raw.trim().chars() {
        match c {
            '(' | '[' => {
                depth += 1;
                cur.push(c);
            }
            ')' | ']' => {
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

/// Parse `grid-template-*` en pistes + noms de lignes —
/// `repeat(n, …)` expansé (borné à 512 pistes), `[noms]`
/// nomment la **bordure** de piste qui suit (le dernier
/// `[noms]` nomme la bordure finale), `minmax(min, max)`
/// garde ses deux bornes (R-5).
#[allow(clippy::cast_precision_loss)]
fn parse_tracks(raw: &str, fs: f32, ctx: &BlockCtx, cb: f32) -> (Vec<Track>, LineNames) {
    let mut out = Vec::new();
    let mut names: LineNames = Vec::new();
    let single = |s: &str| -> Track {
        let t = s.trim().to_ascii_lowercase();
        if let Some(f) = t
            .strip_suffix("fr")
            .and_then(|n| n.trim().parse::<f32>().ok())
        {
            Track::Fr(f.max(0.0))
        } else if t == "auto" || raw_length_px(&t, cb, fs, ctx).is_none() {
            Track::Auto
        } else {
            Track::Raw(s.trim().to_string())
        }
    };
    for tok in track_tokens(raw) {
        let t = tok.to_ascii_lowercase();
        if t.starts_with('[') {
            // `[a b]` : chaque nom = la bordure courante.
            for name in t
                .trim_start_matches('[')
                .trim_end_matches(']')
                .split_whitespace()
            {
                names.push((name.to_string(), out.len()));
            }
            continue;
        }
        if let Some(inner) = t.strip_prefix("repeat(").and_then(|s| s.strip_suffix(')')) {
            // `repeat(n, a b c)` ou `repeat(auto-fill…)` —
            // seul le compte numérique est supporté.
            if let Some((n, list)) = inner.split_once(',') {
                if let Ok(n) = n.trim().parse::<usize>() {
                    let (subs, _) = parse_tracks(list.trim(), fs, ctx, cb);
                    for sub in &subs {
                        for _ in 0..n.min(512 / out.len().max(1)) {
                            out.push(match sub {
                                Track::Raw(r) => Track::Raw(r.clone()),
                                Track::Fr(f) => Track::Fr(*f),
                                Track::Auto => Track::Auto,
                                Track::MinMax(mn, mx) => Track::MinMax(mn.clone(), mx.clone()),
                            });
                        }
                    }
                }
            }
            continue;
        }
        if let Some(inner) = t.strip_prefix("minmax(").and_then(|s| s.strip_suffix(')')) {
            // Deux bornes : `minmax(min, max)` — min résolu
            // comme borne basse à la distribution.
            let (mn, mx) = inner.split_once(',').unwrap_or(("auto", "auto"));
            out.push(Track::MinMax(mn.trim().to_string(), Box::new(single(mx))));
            continue;
        }
        out.push(single(&tok));
        if out.len() > 512 {
            break;
        }
    }
    (out, names)
}

/// `grid-column`/`grid-row` → `(début 0-based, span)` —
/// `a`, `a / b`, `a / span n`, `span n`, **noms de lignes**
/// (`mid / end` via `[mid]`… dans le template). `None` =
/// auto.
fn placement(raw: Option<&str>, names: &[(String, usize)]) -> (Option<usize>, usize) {
    let Some(raw) = raw else {
        return (None, 1);
    };
    let parts: Vec<&str> = raw.split('/').map(str::trim).collect();
    let one = |s: &str| -> (Option<usize>, Option<usize>) {
        let s = s.trim();
        if let Some(n) = s.strip_prefix("span ").and_then(|v| v.trim().parse().ok()) {
            return (None, Some(n));
        }
        if let Ok(v) = s.parse::<usize>() {
            return (v.checked_sub(1), None);
        }
        // Nom de ligne déclaré via `[name]` dans le template.
        let by_name = names.iter().find(|(n, _)| n == s).map(|(_, line)| *line);
        (by_name, None)
    };
    let (a0, s0) = one(parts.first().copied().unwrap_or(""));
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

/// `grid-template-areas` → carte nom → `(col, row, span_c,
/// span_r)` (bounding box de la zone). Cellules `.` ou
/// `none` = vides. Zones non rectangulaires : bbox documenté.
fn parse_areas(raw: &str) -> std::collections::HashMap<String, (usize, usize, usize, usize)> {
    let mut areas: std::collections::HashMap<String, (usize, usize, usize, usize)> =
        std::collections::HashMap::new();
    for (ri, row_str) in raw.split('"').skip(1).step_by(2).enumerate() {
        for (ci, cell) in row_str.split_whitespace().enumerate() {
            if cell == "." || cell == "none" {
                continue;
            }
            areas
                .entry(cell.to_string())
                .and_modify(|(c0, r0, w, h)| {
                    let (right, bottom) = (*c0 + *w, *r0 + *h);
                    *c0 = (*c0).min(ci);
                    *r0 = (*r0).min(ri);
                    *w = right.max(ci + 1) - *c0;
                    *h = bottom.max(ri + 1) - *r0;
                })
                .or_insert((ci, ri, 1, 1));
        }
    }
    areas
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
            Track::MinMax(mn, mx) => {
                // Borne basse immédiate ; le max `fr`/`auto`
                // est résolu plus bas puis borné par elle.
                let lo = raw_length_px(mn, cb, fs, ctx).unwrap_or(0.0);
                match &**mx {
                    Track::Raw(r) => lo.max(raw_length_px(r, cb, fs, ctx).unwrap_or(0.0)),
                    _ => lo,
                }
            }
            _ => 0.0,
        })
        .collect();
    // `auto` (y compris `minmax(…, auto)`) : max des bases
    // mesurées des items qui s'y posent.
    for &(col, span, base) in items_base {
        if span == 1 && col < w.len() {
            let sized = match &tracks[col] {
                Track::Auto => true,
                Track::MinMax(_, mx) => matches!(**mx, Track::Auto),
                _ => false,
            };
            if sized {
                w[col] = w[col].max(base);
            }
        }
    }
    let free = (cb - w.iter().sum::<f32>() - gap * w.len().saturating_sub(1) as f32).max(0.0);
    let track_fr = |t: &Track| match t {
        Track::Fr(f) => *f,
        Track::MinMax(_, mx) => match &**mx {
            Track::Fr(f) => *f,
            _ => 0.0,
        },
        _ => 0.0,
    };
    let total_fr: f32 = tracks.iter().map(&track_fr).sum();
    if total_fr > 0.0 {
        for (i, t) in tracks.iter().enumerate() {
            let f = track_fr(t);
            if f > 0.0 {
                // `minmax(min, fr)` : part du libre bornée par
                // le min déjà posé dans `w[i]` (une passe —
                // redistribution complète documentée hors MVP).
                w[i] = w[i].max(free * f / total_fr);
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

    let (mut col_tracks, col_names) = st.get("grid-template-columns").map_or_else(
        || (Vec::new(), Vec::new()),
        |v| parse_tracks(v, fs, ctx, content_w),
    );
    if col_tracks.is_empty() {
        col_tracks.push(Track::Auto); // une colonne implicite
    }
    let (row_tracks, row_names) = st.get("grid-template-rows").map_or_else(
        || (Vec::new(), Vec::new()),
        |v| parse_tracks(v, fs, ctx, content_w),
    );
    // `grid-template-areas` : carte nom → rect (R-5).
    let areas = st
        .get("grid-template-areas")
        .map_or_else(std::collections::HashMap::new, parse_areas);
    // `grid-auto-flow` : `row` (défaut), `column`, `dense`.
    let flow_raw = st.get("grid-auto-flow").unwrap_or("row");
    let flow_col = flow_raw.contains("column");
    let dense = flow_raw.contains("dense");

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
    // `grid-area` nommé via les areas, positions nommées via
    // `[lignes]`, auto = balayage row-major (ou column-major
    // sous `auto-flow: column`), `dense` rescanne depuis (0,0).
    let mut ncols = col_tracks.len();
    let rows_lim = row_tracks.len().max(1); // borne du column flow
    let mut occupancy: Vec<Vec<bool>> = Vec::new();
    let grow_cols = |occ: &mut Vec<Vec<bool>>, ncols: &mut usize, col_tracks: &mut Vec<Track>| {
        col_tracks.push(Track::Auto); // colonne implicite
        *ncols += 1;
        for r in occ.iter_mut() {
            r.push(false);
        }
    };
    let free_at = |occ: &[Vec<bool>],
                   r: usize,
                   c: usize,
                   span_c: usize,
                   span_r: usize,
                   ncols: usize|
     -> bool {
        c + span_c <= ncols
            && (0..span_r.max(1)).all(|dr| {
                (0..span_c).all(|dc| {
                    !occ.get(r + dr)
                        .and_then(|row| row.get(c + dc))
                        .copied()
                        .unwrap_or(false)
                })
            })
    };
    let mut slots: Vec<(usize, usize, usize, usize)> = Vec::with_capacity(order_idx.len());
    let (mut auto_col, mut auto_row) = (0usize, 0usize);
    for &i in &order_idx {
        let cst = &bx.children[i].style;
        // `grid-area` : nom d'area (`grid-template-areas`) ou
        // forme numérique `r / c / r2 / c2` (bords 1-based).
        let area_rect = cst.get("grid-area").and_then(|a| {
            let a = a.trim();
            areas.get(a).copied().or_else(|| {
                let parts: Vec<(Option<usize>, usize)> =
                    a.split('/').map(|p| placement(Some(p), &[])).collect();
                if parts.len() == 4 {
                    // r / c / r2 / c2 → (col, span_c, row, span_r)
                    let (r0, _) = parts[0];
                    let (c0, _) = parts[1];
                    let (r1, s1) = parts[2];
                    let (c1, s2) = parts[3];
                    let row = r0?;
                    let col = c0?;
                    let span_c = if s2 > 1 {
                        s2
                    } else {
                        c1.map_or(1, |e| e.saturating_sub(col).max(1))
                    };
                    let span_r = if s1 > 1 {
                        s1
                    } else {
                        r1.map_or(1, |e| e.saturating_sub(row).max(1))
                    };
                    Some((col, row, span_c, span_r))
                } else {
                    None
                }
            })
        });
        let (mut start_c, mut span_c) = placement(cst.get("grid-column"), &col_names);
        let (mut start_r, mut span_r) = placement(cst.get("grid-row"), &row_names);
        if let Some((c0, r0, sc, sr)) = area_rect {
            start_c = Some(c0);
            span_c = sc;
            start_r = Some(r0);
            span_r = sr;
        }
        while span_c > ncols {
            grow_cols(&mut occupancy, &mut ncols, &mut col_tracks);
        }
        let span_c = span_c.max(1);
        let (mut col, mut row);
        if let Some(c0) = start_c {
            col = c0.min(ncols - span_c);
            row = start_r.unwrap_or(0);
            while !free_at(&occupancy, row, col, span_c, span_r, ncols) {
                row += 1;
                if row > 4096 {
                    break; // borne hostile
                }
            }
        } else if flow_col {
            // `auto-flow: column` : descend les lignes d'une
            // colonne puis avance ; colonnes implicites au-delà.
            col = auto_col;
            row = start_r.unwrap_or(auto_row);
            loop {
                while col + span_c > ncols {
                    grow_cols(&mut occupancy, &mut ncols, &mut col_tracks);
                }
                if row + span_r.max(1) > rows_lim {
                    row = 0;
                    col += 1;
                    continue;
                }
                if col > 4096 || free_at(&occupancy, row, col, span_c, span_r, ncols) {
                    break;
                }
                row += 1;
            }
        } else {
            // Balayage avant depuis le curseur auto — `dense`
            // rescanne depuis l'origine (backfill).
            col = if dense { 0 } else { auto_col };
            row = start_r.unwrap_or(if dense { 0 } else { auto_row });
            loop {
                if col + span_c > ncols {
                    col = 0;
                    row += 1;
                }
                if row > 4096 || free_at(&occupancy, row, col, span_c, span_r, ncols) {
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
        if flow_col {
            auto_row = row + span_r.max(1);
            auto_col = col;
            if auto_row >= rows_lim {
                auto_row = 0;
                auto_col += 1;
            }
        } else {
            auto_col = col + span_c;
            auto_row = row;
            if auto_col >= ncols {
                auto_col = 0;
                auto_row += 1;
            }
        }
    }

    // Base de mesure des items (auto colonne) : `width`
    // explicite, intrinsèque remplacé, max-content réel (R-3).
    let items_base: Vec<(usize, usize, f32)> = slots
        .iter()
        .zip(&order_idx)
        .map(|(&(c, span, _, _), &i)| {
            let ch = &bx.children[i];
            let cfs = font_size_px(&ch.style, fs, ctx);
            let base = css_px(&ch.style, "width", content_w, cfs, ctx)
                .or_else(|| ch.intrinsic.map(|v| v.0))
                .unwrap_or_else(|| crate::intrinsic::max_content_px(ch, cfs, ctx));
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
