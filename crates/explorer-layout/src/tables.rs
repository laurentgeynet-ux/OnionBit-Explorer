// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Tables (étape 21) : dimensionnement `display: table`.
//!
//! Algorithme **auto** simplifié (CSS 2.1 §17.5.2.5) :
//! - lignes = descendants `display: table-row` (les groupes
//!   `table-row-group|header|footer` sont transparents) ;
//! - cellules = `display: table-cell` directs de la ligne ;
//! - par colonne : `min` = plus long mot des cellules,
//!   `pref` = contenu déroulé — la table distribue sa
//!   largeur au prorata de `pref` (bornée par `min`) ;
//! - chaque cellule est un bloc conteneur standard
//!   (`layout_block`) ; hauteur de ligne = max des cellules.
//!
//! Hors périmètre documenté : `colspan`/`rowspan` (cellules
//! comptées 1×1), `border-collapse` (modèle `separate` avec
//! `border-spacing` horizontal 0 — l'espace vient des marges
//! des cellules), `display: table-column`, `caption`.

use crate::block::{css_px, font_size_px, layout_block, measure_fn, BlockCtx};
use crate::boxtree::{LayoutBox, Rect};

/// Une cellule repérée : (rangée, colonne, indice dans
/// `row_box.children`).
struct Cell {
    col: usize,
    /// Indice de l'enfant dans sa boîte ligne.
    idx: usize,
}

/// Repère les boîtes ligne (`display: table-row`) en
/// traversant les groupes, puis les cellules de chaque.
fn collect_grid(bx: &LayoutBox) -> Vec<(usize, Vec<Cell>)> {
    fn walk(b: &LayoutBox, rows: &mut Vec<(usize, Vec<Cell>)>, ri: &mut usize) {
        if b.display() == "table-row" {
            let mut cells = Vec::new();
            for (i, c) in b.children.iter().enumerate() {
                if c.display() == "table-cell" {
                    cells.push(Cell {
                        col: cells.len(),
                        idx: i,
                    });
                }
            }
            rows.push((*ri, cells));
            *ri += 1;
            return;
        }
        for c in &b.children {
            walk(c, rows, ri);
        }
    }
    let mut rows: Vec<(usize, Vec<Cell>)> = Vec::new();
    let mut ri = 0usize;
    walk(bx, &mut rows, &mut ri);
    rows
}

/// Boîte `display: table` : dimensionne colonnes et lignes,
/// retourne la hauteur utilisée.
pub(crate) fn layout_table(
    bx: &mut LayoutBox,
    x: f32,
    y: f32,
    cb_w: f32,
    ctx: &BlockCtx,
    parent_fs: f32,
) -> f32 {
    let st = bx.style.clone();
    let fs = font_size_px(&st, parent_fs, ctx);
    let grid = collect_grid(bx);
    let ncols = grid.iter().map(|(_, cells)| cells.len()).max().unwrap_or(0);
    if ncols == 0 {
        bx.rect = Rect {
            x,
            y,
            width: cb_w.min(css_px(&st, "width", cb_w, fs, ctx).unwrap_or(cb_w)),
            height: 0.0,
        };
        return 0.0;
    }

    // Mesures par colonne : min (plus long mot) et pref
    // (texte déroulé) — padding/bordure de cellule incluses.
    let mut min_w = vec![0.0_f32; ncols];
    let mut pref_w = vec![1.0_f32; ncols];
    for (r, cells) in &grid {
        for cell in cells {
            let (min, pref) = cell_widths(bx, *r, cell.idx, ctx, fs);
            min_w[cell.col] = min_w[cell.col].max(min);
            pref_w[cell.col] = pref_w[cell.col].max(pref.max(min));
        }
    }

    // Largeur de la table.
    let total_pref: f32 = pref_w.iter().sum();
    let total_min: f32 = min_w.iter().sum();
    let table_w = css_px(&st, "width", cb_w, fs, ctx)
        .unwrap_or_else(|| total_pref.min(cb_w))
        .min(cb_w);
    let table_w = table_w.max(total_min.min(cb_w));

    // Distribution : surplus au prorata de pref ; déficit
    // jamais sous min (sauf si min total > table → écrêté).
    let mut col_w = vec![0.0_f32; ncols];
    if total_pref <= f32::EPSILON {
        let ncols_f = u16::try_from(ncols).map_or(0.0, f32::from);
        let each = if ncols_f > 0.0 {
            table_w / ncols_f
        } else {
            0.0
        };
        col_w.fill(each);
    } else {
        let mut left = table_w;
        for (i, w) in col_w.iter_mut().enumerate() {
            *w = (table_w * pref_w[i] / total_pref).max(min_w[i].min(table_w));
            left -= *w;
        }
        // Reste/retirer : ajuste sur la plus large colonne.
        if let Some(max) = col_w.iter_mut().max_by(|a, b| a.total_cmp(b)) {
            *max = (*max + left).max(0.0);
        }
    }

    // Pose : chaque cellule est un bloc dans sa colonne ;
    // hauteur de rangée = max des hauteurs posées.
    let mut cursor_y = y;
    let mut row_tops = Vec::new();
    for (r, cells) in &grid {
        let mut row_h = 0.0_f32;
        for cell in cells {
            let col_x: f32 = x + col_w[..cell.col].iter().sum::<f32>();
            let cw = col_w[cell.col];
            // Cellule positionnée → conteneur pour ses absolus.
            let cell_cb = Rect {
                x: col_x,
                y: cursor_y,
                width: cw,
                height: 0.0,
            };
            let Some(cell_box) = cell_ref_mut(bx, *r, cell.idx) else {
                continue;
            };
            let used = layout_block(cell_box, col_x, cursor_y, cw, ctx, fs, cell_cb);
            row_h = row_h.max(used);
        }
        row_tops.push((cursor_y, row_h));
        cursor_y += row_h;
    }
    // Deuxième passe : uniformiser les hauteurs de cellule
    // (border-box = hauteur de rangée, si height auto).
    for (r, cells) in &grid {
        let (ry, rh) = row_tops[*r];
        for cell in cells {
            let Some(cell_box) = cell_ref_mut(bx, *r, cell.idx) else {
                continue;
            };
            if css_px(&cell_box.style.clone(), "height", cb_w, fs, ctx).is_none()
                && cell_box.rect.height < rh
            {
                cell_box.rect.height = rh;
            }
            cell_box.rect.y = ry;
        }
    }
    let h = cursor_y - y;
    bx.rect = Rect {
        x,
        y,
        width: table_w,
        height: h,
    };
    h
}

/// Référence mutable à la cellule (ligne `row`, enfant
/// `idx` de la boîte ligne `row`).
fn cell_ref_mut(bx: &mut LayoutBox, row: usize, idx: usize) -> Option<&mut LayoutBox> {
    // Rejouer le parcours de `collect_grid` jusqu'à la ligne
    // `row` — le compteur d'ordre est stable ; `None` si la
    // grille a été mutée entre-temps (dégradation, pas panic).
    fn find<'a>(b: &'a mut LayoutBox, row: &mut usize, idx: usize) -> Option<&'a mut LayoutBox> {
        if b.display() == "table-row" {
            if *row == 0 {
                return b.children.get_mut(idx);
            }
            *row -= 1;
            return None;
        }
        for c in &mut b.children {
            if let Some(r) = find(c, row, idx) {
                return Some(r);
            }
        }
        None
    }
    let mut r = row;
    find(bx, &mut r, idx)
}

/// Largeurs min/pref d'une cellule : plus long mot /
/// contenu déroulé + padding/bordure horizontaux.
fn cell_widths(bx: &LayoutBox, row: usize, idx: usize, ctx: &BlockCtx, fs: f32) -> (f32, f32) {
    // Lecture immuable via le même parcours.
    fn find<'a>(b: &'a LayoutBox, row: &mut usize, idx: usize) -> Option<&'a LayoutBox> {
        if b.display() == "table-row" {
            if *row == 0 {
                return b.children.get(idx);
            }
            *row -= 1;
            return None;
        }
        for c in &b.children {
            if let Some(r) = find(c, row, idx) {
                return Some(r);
            }
        }
        None
    }
    let mut r = row;
    let Some(cell) = find(bx, &mut r, idx) else {
        return (0.0, 0.0);
    };
    let st = &cell.style;
    let measure = measure_fn(st, font_size_px(st, fs, ctx), ctx);
    let txt = flatten_text(cell);
    let longest = txt.split_whitespace().map(&measure).fold(0.0_f32, f32::max);
    let pref = measure(txt.trim());
    let extras = css_px(st, "padding-left", 0.0, fs, ctx).unwrap_or(0.0)
        + css_px(st, "padding-right", 0.0, fs, ctx).unwrap_or(0.0)
        + css_px(st, "border-left-width", 0.0, fs, ctx).unwrap_or(0.0)
        + css_px(st, "border-right-width", 0.0, fs, ctx).unwrap_or(0.0);
    (longest + extras, pref + extras)
}

/// Texte aplati d'une boîte (tous descendants).
fn flatten_text(b: &LayoutBox) -> String {
    let mut s = String::new();
    if let Some(t) = &b.text {
        s.push_str(t);
    }
    for c in &b.children {
        s.push_str(&flatten_text(c));
        s.push(' ');
    }
    s
}
