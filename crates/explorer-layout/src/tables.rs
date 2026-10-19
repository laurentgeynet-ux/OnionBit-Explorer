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

/// Une cellule repérée : colonne + chemin d'indices depuis
/// la boîte table jusqu'à elle (le chemin traverse un rang
/// explicite ou désigne un enfant direct = cellule anonyme).
struct Cell {
    col: usize,
    /// Indices `bx → … → table-cell`.
    path: Vec<usize>,
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
/// de rangs) + ses cellules. `path` **vide** = rang anonyme
/// (§17.2.1) : les cellules sont des enfants directs, aucune
/// boîte ligne n'existe (elle ne peint ni fond ni bordure).
struct Row {
    /// Indices `bx → … → table-row` ; `None` = rang anonyme
    /// (§17.2.1) — aucune boîte ligne n'existe : rien à poser.
    path: Option<Vec<usize>>,
    /// Classe visuelle du groupe conteneur.
    class: RowClass,
    cells: Vec<Cell>,
}

/// `true` pour les boîtes qui ne peuvent pas tenir de cellule
/// dans une rangée : texte blanc inter-éléments (l'espace
/// source entre `<li>` n'est PAS du contenu de cellule).
fn ignorable_in_table(b: &LayoutBox) -> bool {
    matches!(
        b.kind,
        crate::BoxKind::Text | crate::BoxKind::AnonymousBlock
    ) && b.text.as_deref().is_none_or(|t| t.trim().is_empty())
        && b.children.iter().all(ignorable_in_table)
}

/// Repère les rangs (`display: table-row` explicites ou
/// anonymes) en traversant les groupes — renvoyés en **ordre
/// visuel** (tête, corps en ordre document, pied), tri stable.
/// §17.2.1 : les `table-cell` nues et tout autre contenu sont
/// emballés dans des cellules/rangs anonymes ; une table
/// **imbriquée** est du contenu (on ne descend pas dedans).
fn collect_grid(bx: &LayoutBox) -> Vec<Row> {
    /// Cellules d'un rang explicite (enfants `table-cell`
    /// directs — le reste est ignoré, rangs réels seuls).
    fn row_cells(b: &LayoutBox, path: &[usize]) -> Vec<Cell> {
        b.children
            .iter()
            .enumerate()
            .filter(|(_, c)| c.display() == "table-cell")
            .enumerate()
            .map(|(col, (idx, _))| {
                let mut p = path.to_vec();
                p.push(idx);
                Cell { col, path: p }
            })
            .collect()
    }
    /// Marche une suite d'enfants (table, ou groupe de rangs) :
    /// rangs explicites + accumulation des cellules nues en
    /// rangs anonymes (`path` = préfixe jusqu'au conteneur).
    fn walk_children(b: &LayoutBox, class: RowClass, prefix: &mut Vec<usize>, rows: &mut Vec<Row>) {
        let mut pending: Vec<Cell> = Vec::new();
        let flush = |pending: &mut Vec<Cell>, rows: &mut Vec<Row>| {
            if !pending.is_empty() {
                rows.push(Row {
                    path: None,
                    class,
                    cells: std::mem::take(pending),
                });
            }
        };
        for (i, c) in b.children.iter().enumerate() {
            match c.display() {
                "table-row" => {
                    flush(&mut pending, rows);
                    prefix.push(i);
                    let cells = row_cells(c, prefix);
                    prefix.pop();
                    rows.push(Row {
                        path: Some({
                            let mut p = prefix.clone();
                            p.push(i);
                            p
                        }),
                        class,
                        cells,
                    });
                }
                "table-header-group" | "table-row-group" | "table-footer-group" => {
                    flush(&mut pending, rows);
                    let g = match c.display() {
                        "table-header-group" => RowClass::Header,
                        "table-footer-group" => RowClass::Footer,
                        _ => RowClass::Body,
                    };
                    prefix.push(i);
                    // Arête ∝ profondeur de l'arbre (R-24).
                    explorer_common::deep(|| walk_children(c, g, prefix, rows));
                    prefix.pop();
                }
                _ => {
                    // Hors rangs/groupes : `table-cell` nue ou
                    // contenu quelconque → cellule du rang
                    // anonyme courant (sauf blanc ignorable).
                    if !ignorable_in_table(c) {
                        let mut p = prefix.clone();
                        p.push(i);
                        pending.push(Cell {
                            col: pending.len(),
                            path: p,
                        });
                    }
                }
            }
        }
        flush(&mut pending, rows);
    }
    let mut rows: Vec<Row> = Vec::new();
    walk_children(bx, RowClass::Body, &mut Vec::new(), &mut rows);
    let rank = |c: RowClass| match c {
        RowClass::Header => 0,
        RowClass::Body => 1,
        RowClass::Footer => 2,
    };
    rows.sort_by_key(|r| rank(r.class));
    // Recolonne : `col` suit l'ordre des cellules dans CHAQUE
    // rang (déjà en ordre doc — rien à faire).
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
    for &i in row.path.as_deref().unwrap_or(&[]) {
        let Some(c) = cur.children.get(i) else {
            return w;
        };
        w = w.max(border_px(&c.style, &prop, cb_w, fs, ctx));
        cur = c;
    }
    for cell in &row.cells {
        let mut cc = bx;
        for &i in &cell.path {
            let Some(n) = cc.children.get(i) else {
                return w;
            };
            cc = n;
        }
        w = w.max(border_px(&cc.style, &prop, cb_w, fs, ctx));
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
        // Table sans aucune cellule : le `width`/`height`
        // spécifiés s'appliquent quand même (une table vide
        // imbriquée garde sa taille — Acid2 `li.second-part`).
        let h = css_px(&st, "height", 0.0, fs, ctx).unwrap_or(0.0);
        bx.rect = Rect {
            x,
            y,
            width: cb_w.min(css_px(&st, "width", cb_w, fs, ctx).unwrap_or(cb_w)),
            height: h,
        };
        return h;
    }

    // Mesures par colonne : min (plus long mot) et pref
    // (texte déroulé) — padding/bordure de cellule incluses.
    let mut min_w = vec![0.0_f32; ncols];
    let mut pref_w = vec![1.0_f32; ncols];
    for row in &rows {
        for cell in &row.cells {
            let (min, pref) = cell_widths(bx, &cell.path, ctx, fs);
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
            let Some(cell_box) = at_path(bx, &cell.path) else {
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
    // Deuxième passe : uniformiser les hauteurs de cellule —
    // §17.5.3 : la `height` d'une cellule est un **minimum**,
    // la rangée l'étire toujours (même spécifiée) ;
    // géométrie réelle pour le rang et ses groupes (leurs
    // fonds/bordures peignent — sous collapse, l'étendue du
    // groupe rejoint ses arêtes partagées pour que ses
    // `border-top|bottom` peignent les lignes fusionnées).
    for (k, row) in rows.iter().enumerate() {
        let (ry, rh) = row_tops[k];
        for cell in &row.cells {
            let Some(cell_box) = at_path(bx, &cell.path) else {
                continue;
            };
            cell_box.rect.height = cell_box.rect.height.max(rh);
            cell_box.rect.y = ry;
        }
        let mut cur = &mut *bx;
        let Some(rpath) = &row.path else {
            continue;
        };
        for (d, &i) in rpath.iter().enumerate() {
            let Some(c) = cur.children.get_mut(i) else {
                break;
            };
            if d + 1 == rpath.len() {
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
                let first = k == 0
                    || rows[k - 1].path.as_deref().and_then(|p| p.get(..=d)) != rpath.get(..=d);
                let last = k + 1 == nrows
                    || rows[k + 1].path.as_deref().and_then(|p| p.get(..=d)) != rpath.get(..=d);
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
fn cell_widths(bx: &LayoutBox, cell_path: &[usize], ctx: &BlockCtx, fs: f32) -> (f32, f32) {
    let mut cur = bx;
    for &i in cell_path {
        let Some(c) = cur.children.get(i) else {
            return (0.0, 0.0);
        };
        cur = c;
    }
    let cell = cur;
    let st = &cell.style;
    // `width` spécifiée domine la mesure de contenu (CSS2
    // §17.5.2.2 : une largeur fixe de cellule borne la
    // colonne — Acid2 pose `width:1em` sur des cellules
    // vides).
    if let Some(w) = css_px(st, "width", 0.0, fs, ctx) {
        return (w, w);
    }
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
