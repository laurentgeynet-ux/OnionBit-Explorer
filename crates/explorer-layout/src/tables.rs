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
//! `border-collapse: collapse` — MVP horizontal (§17.6.2) :
//! l'arête partagée entre deux rangs adjacents a pour largeur
//! le `max` des bordures candidates (groupe, rang, cellules) ;
//! elle occupe son propre espace entre les rangs. La
//! géométrie des groupes de rangs est étendue jusqu'à leurs
//! arêtes partagées, de sorte que leurs `border-top|bottom`
//! propres peignent les lignes — aucun item de display list
//! dédié n'est requis. Limites documentées : seules les
//! arêtes horizontales fusionnent (les bordures verticales et
//! celles des `tr`/`td` restent peintes à leurs propres
//! bords), `hidden` n'écrase pas, pas de résolution de
//! couleur gagnante.
//!
//! Hors périmètre documenté : `colspan`/`rowspan` (cellules
//! comptées 1×1), `border-spacing`, `display: table-column`,
//! `caption`.

use crate::block::{
    border_px, css_px, font_size_px, layout_block, measure_fn, side_px, BlockCtx, FloatCtx,
};
use crate::boxtree::{LayoutBox, Rect};

/// Une cellule repérée : (rangée, colonne, indice dans
/// `row_box.children`).
struct Cell {
    col: usize,
    /// Indice de l'enfant dans sa boîte ligne.
    idx: usize,
}

/// Classe visuelle d'un rang (§17.5 : header, corps, footer
/// — `tfoot` se rend APRÈS `tbody` quel que soit l'ordre du
/// document).
#[derive(Clone, Copy, PartialEq, Eq)]
enum RowClass {
    /// `table-header-group`.
    Header,
    /// `table-row-group` ou rang hors groupe.
    Body,
    /// `table-footer-group`.
    Footer,
}

/// Un rang `table-row` repéré : chemin d'indices depuis la
/// boîte table (les éléments intermédiaires sont les groupes
/// de rangs) + ses cellules.
struct Row {
    /// Indices enfants `bx → … → table-row`.
    path: Vec<usize>,
    /// Classe visuelle du groupe conteneur.
    class: RowClass,
    cells: Vec<Cell>,
}

/// Repère les boîtes ligne (`display: table-row`) en
/// traversant les groupes — renvoyés en **ordre visuel**
/// (tête, corps en ordre document, pied), tri stable.
fn collect_grid(bx: &LayoutBox) -> Vec<Row> {
    fn walk(b: &LayoutBox, class: RowClass, path: &mut Vec<usize>, rows: &mut Vec<Row>) {
        if b.display() == "table-row" {
            let cells = b
                .children
                .iter()
                .enumerate()
                .filter(|(_, c)| c.display() == "table-cell")
                .enumerate()
                .map(|(col, (idx, _))| Cell { col, idx })
                .collect();
            rows.push(Row {
                path: path.clone(),
                class,
                cells,
            });
            return;
        }
        let class = match b.display() {
            "table-header-group" => RowClass::Header,
            "table-footer-group" => RowClass::Footer,
            _ => class,
        };
        for (i, c) in b.children.iter().enumerate() {
            path.push(i);
            walk(c, class, path, rows);
            path.pop();
        }
    }
    let mut rows: Vec<Row> = Vec::new();
    walk(bx, RowClass::Body, &mut Vec::new(), &mut rows);
    let rank = |c: RowClass| match c {
        RowClass::Header => 0,
        RowClass::Body => 1,
        RowClass::Footer => 2,
    };
    rows.sort_by_key(|r| rank(r.class));
    rows
}

/// Descend le chemin d'indices — `None` si l'arbre a été
/// muté entre-temps (dégradation, jamais de panique).
fn at_path<'a>(bx: &'a mut LayoutBox, path: &[usize]) -> Option<&'a mut LayoutBox> {
    let mut cur = bx;
    for &i in path {
        cur = cur.children.get_mut(i)?;
    }
    Some(cur)
}

/// Largeur d'arête horizontale fusionnée candidate pour un
/// rang : `max` des `border-{side}-width` de chaque boîte du
/// chemin (groupes + rang) et de chaque cellule.
fn chain_edge(bx: &LayoutBox, row: &Row, side: &str, cb_w: f32, fs: f32, ctx: &BlockCtx) -> f32 {
    let prop = format!("border-{side}-width");
    let mut w = 0.0_f32;
    let mut cur = bx;
    for &i in &row.path {
        let Some(c) = cur.children.get(i) else {
            return w;
        };
        w = w.max(border_px(&c.style, &prop, cb_w, fs, ctx));
        cur = c;
    }
    for cell in &row.cells {
        if let Some(c) = cur.children.get(cell.idx) {
            w = w.max(border_px(&c.style, &prop, cb_w, fs, ctx));
        }
    }
    w
}

/// Boîte `display: table` : dimensionne colonnes et lignes,
/// retourne la hauteur utilisée.
#[allow(clippy::too_many_lines)]
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
    let rows = collect_grid(bx);
    let ncols = rows.iter().map(|r| r.cells.len()).max().unwrap_or(0);
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
    for row in &rows {
        for cell in &row.cells {
            let (min, pref) = cell_widths(bx, &row.path, cell.idx, ctx, fs);
            min_w[cell.col] = min_w[cell.col].max(min);
            pref_w[cell.col] = pref_w[cell.col].max(pref.max(min));
        }
    }

    // Bordures/padding PROPRES de la table : la grille de
    // cellules occupe le content box — `table_w` est la
    // border-box (le `width` spécifié s'y applique, à la
    // manière des navigateurs pour `display: table`).
    let il = border_px(&st, "border-left-width", cb_w, fs, ctx)
        + side_px(&st, "padding-left", cb_w, fs, ctx, 0.0);
    let ir = border_px(&st, "border-right-width", cb_w, fs, ctx)
        + side_px(&st, "padding-right", cb_w, fs, ctx, 0.0);
    let it = border_px(&st, "border-top-width", cb_w, fs, ctx)
        + side_px(&st, "padding-top", cb_w, fs, ctx, 0.0);
    let ib = border_px(&st, "border-bottom-width", cb_w, fs, ctx)
        + side_px(&st, "padding-bottom", cb_w, fs, ctx, 0.0);

    // Largeur de la table.
    let total_pref: f32 = pref_w.iter().sum();
    let total_min: f32 = min_w.iter().sum();
    let table_w = css_px(&st, "width", cb_w, fs, ctx)
        .unwrap_or_else(|| total_pref.min(cb_w))
        .min(cb_w);
    let table_w = table_w.max(total_min.min(cb_w));
    let grid_w = (table_w - il - ir).max(0.0);

    // Distribution : surplus au prorata de pref ; déficit
    // jamais sous min (sauf si min total > table → écrêté).
    let mut col_w = vec![0.0_f32; ncols];
    if total_pref <= f32::EPSILON {
        let ncols_f = u16::try_from(ncols).map_or(0.0, f32::from);
        let each = if ncols_f > 0.0 { grid_w / ncols_f } else { 0.0 };
        col_w.fill(each);
    } else {
        let mut left = grid_w;
        for (i, w) in col_w.iter_mut().enumerate() {
            *w = (grid_w * pref_w[i] / total_pref).max(min_w[i].min(grid_w));
            left -= *w;
        }
        // Reste/retirer : ajuste sur la plus large colonne.
        if let Some(max) = col_w.iter_mut().max_by(|a, b| a.total_cmp(b)) {
            *max = (*max + left).max(0.0);
        }
    }

    // §17.6.2 `border-collapse` (MVP horizontal) : l'arête
    // partagée `gap[k]` précède le rang `k` — `gap[0]` et
    // `gap[n]` fusionnent avec la bordure propre de la table
    // (déjà réservée par `it`/`ib` — seul l'excédent compte).
    let nrows = rows.len();
    let collapse = st
        .get("border-collapse")
        .is_some_and(|v| v.trim() == "collapse");
    let mut gap = vec![0.0_f32; nrows + 1];
    if collapse {
        let top_w: Vec<f32> = rows
            .iter()
            .map(|r| chain_edge(bx, r, "top", cb_w, fs, ctx))
            .collect();
        let bot_w: Vec<f32> = rows
            .iter()
            .map(|r| chain_edge(bx, r, "bottom", cb_w, fs, ctx))
            .collect();
        let bt = border_px(&st, "border-top-width", cb_w, fs, ctx);
        let bb = border_px(&st, "border-bottom-width", cb_w, fs, ctx);
        gap[0] = (top_w[0] - bt).max(0.0);
        for k in 1..nrows {
            gap[k] = bot_w[k - 1].max(top_w[k]);
        }
        gap[nrows] = (bot_w[nrows - 1] - bb).max(0.0);
    }

    // Pose : chaque cellule est un bloc dans sa colonne ;
    // hauteur de rangée = max des hauteurs posées. La grille
    // démarre au bord interne (bordure+padding de la table).
    let mut cursor_y = y + it + gap[0];
    let mut row_tops = Vec::new();
    for (k, row) in rows.iter().enumerate() {
        let mut row_h = 0.0_f32;
        for cell in &row.cells {
            let col_x: f32 = x + il + col_w[..cell.col].iter().sum::<f32>();
            let cw = col_w[cell.col];
            // Cellule positionnée → conteneur pour ses absolus.
            let cell_cb = Rect {
                x: col_x,
                y: cursor_y,
                width: cw,
                height: 0.0,
            };
            let Some(cell_box) = at_path(bx, &row.path).and_then(|t| t.children.get_mut(cell.idx))
            else {
                continue;
            };
            // Cellule = BFC propre : registre vierge (§9.4.1).
            // `cb_h` indéfini — la hauteur de rangée dépend du
            // contenu (§10.5 : % enfants → auto).
            let used = layout_block(
                cell_box,
                col_x,
                cursor_y,
                cw,
                None,
                ctx,
                fs,
                cell_cb,
                &mut FloatCtx::default(),
            );
            row_h = row_h.max(used);
        }
        row_tops.push((cursor_y, row_h));
        cursor_y += row_h + gap[k + 1];
    }
    // Deuxième passe : uniformiser les hauteurs de cellule
    // (border-box = hauteur de rangée, si height auto) ;
    // géométrie réelle pour le rang et ses groupes (leurs
    // fonds/bordures peignent — sous collapse, l'étendue du
    // groupe rejoint ses arêtes partagées pour que ses
    // `border-top|bottom` peignent les lignes fusionnées).
    for (k, row) in rows.iter().enumerate() {
        let (ry, rh) = row_tops[k];
        for cell in &row.cells {
            let Some(cell_box) = at_path(bx, &row.path).and_then(|t| t.children.get_mut(cell.idx))
            else {
                continue;
            };
            if css_px(&cell_box.style.clone(), "height", cb_w, fs, ctx).is_none()
                && cell_box.rect.height < rh
            {
                cell_box.rect.height = rh;
            }
            cell_box.rect.y = ry;
        }
        let mut cur = &mut *bx;
        for (d, &i) in row.path.iter().enumerate() {
            let Some(c) = cur.children.get_mut(i) else {
                break;
            };
            if d + 1 == row.path.len() {
                // `table-row` : étendue réelle de la rangée.
                c.rect = Rect {
                    x: x + il,
                    y: ry,
                    width: grid_w,
                    height: rh,
                };
            } else {
                // Groupe intermédiaire : union des rangs qu'il
                // contient ± ses arêtes partagées propres
                // (premier/dernier rang de sa suite).
                let first = k == 0 || rows[k - 1].path.get(..=d) != row.path.get(..=d);
                let last = k + 1 == nrows || rows[k + 1].path.get(..=d) != row.path.get(..=d);
                let gtop =
                    first && border_px(&c.style.clone(), "border-top-width", cb_w, fs, ctx) > 0.0;
                let gbot =
                    last && border_px(&c.style.clone(), "border-bottom-width", cb_w, fs, ctx) > 0.0;
                let top = ry - if gtop { gap[k] } else { 0.0 };
                let bot = ry + rh + if gbot { gap[k + 1] } else { 0.0 };
                if c.rect.height <= 0.0 && c.rect.width <= 0.0 {
                    c.rect = Rect {
                        x: x + il,
                        y: top,
                        width: grid_w,
                        height: bot - top,
                    };
                } else {
                    let ny = c.rect.y.min(top);
                    c.rect.height = (c.rect.y + c.rect.height).max(bot) - ny;
                    c.rect.y = ny;
                }
            }
            cur = c;
        }
    }
    let h = cursor_y - y + ib;
    bx.rect = Rect {
        x,
        y,
        width: table_w,
        height: h,
    };
    h
}

/// Largeurs min/pref d'une cellule : plus long mot /
/// contenu déroulé + padding/bordure horizontaux.
fn cell_widths(bx: &LayoutBox, path: &[usize], idx: usize, ctx: &BlockCtx, fs: f32) -> (f32, f32) {
    let mut cur = bx;
    for &i in path {
        let Some(c) = cur.children.get(i) else {
            return (0.0, 0.0);
        };
        cur = c;
    }
    let Some(cell) = cur.children.get(idx) else {
        return (0.0, 0.0);
    };
    let st = &cell.style;
    let measure = measure_fn(st, font_size_px(st, fs, ctx), ctx);
    let txt = flatten_text(cell);
    // `\u{A0}` n'est PAS un blanc CSS : il porte l'avance du
    // min-content comme n'importe quel glyphe (§16.6.1).
    let longest = txt
        .split(crate::text::is_css_ws)
        .map(&measure)
        .fold(0.0_f32, f32::max);
    let pref = measure(crate::text::trim_css_ws(&txt));
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
