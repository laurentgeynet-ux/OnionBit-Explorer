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
//! `colspan`/`rowspan` (R-47) : placement par occupation de
//! grille — un `rowspan` retient ses colonnes sur les rangs
//! suivants, un `colspan` réserve sa plage ; la mesure d'une
//! cellule à span se répartit en déficit égal sur les
//! colonnes couvertes (§17.5.2.2), sa boîte s'étend sur la
//! somme des rangs/colonnes couverts.
//!
//! Hors périmètre documenté : distribution de la hauteur
//! d'un `rowspan` entre ses rangs (toute la hauteur reste
//! dans le rang d'origine), `colspan=0` (= 1), `span` des
//! `col`, `border-spacing`, `display: table-column`,
//! `caption`.

use crate::block::{border_px, css_px, font_size_px, layout_block, side_px, BlockCtx, FloatCtx};
use crate::boxtree::{LayoutBox, Rect};
use explorer_css::ComputedStyle;

/// Une cellule repérée : colonne de départ (assignée par le
/// placement, R-47), étendue `cols`×`rows` + chemin d'indices
/// depuis la boîte table jusqu'à elle (le chemin traverse un
/// rang explicite ou désigne un enfant direct = cellule
/// anonyme).
struct Cell {
    col: usize,
    /// `colspan` utilisé (≥ 1).
    cols: usize,
    /// `rowspan` utilisé (≥ 1).
    span_rows: usize,
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

/// Légendes `table-caption` repérées dans l'arbre (§17.4 :
/// posées au-dessus ou au-dessous de la grille selon
/// `caption-side`, pleine largeur de table). Hors périmètre
/// documenté : `caption-side: left|right`.
#[derive(Default)]
struct Captions {
    /// `caption-side: top` (défaut).
    top: Vec<Vec<usize>>,
    /// `caption-side: bottom`.
    bottom: Vec<Vec<usize>>,
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
pub(crate) fn ignorable_in_table(b: &LayoutBox) -> bool {
    // Colonnes (`col`/`colgroup`) : des repères de grille,
    // pas du contenu — jamais de cellule anonyme.
    if matches!(b.display(), "table-column" | "table-column-group") {
        return true;
    }
    matches!(
        b.kind,
        crate::BoxKind::Text | crate::BoxKind::AnonymousBlock
    ) && b.text.as_deref().is_none_or(|t| t.trim().is_empty())
        && b.children.iter().all(ignorable_in_table)
}

/// Styles des `col` explicites en ordre document — sources de
/// largeur du `table-layout:fixed` (§17.5.2.1). Descend dans
/// les anonymes et les `colgroup` (un `col` est compté pour
/// une colonne — `span` hors périmètre documenté).
fn collect_col_styles<'a>(bx: &'a LayoutBox, out: &mut Vec<&'a ComputedStyle>) {
    for c in &bx.children {
        match c.display() {
            "table-column" => out.push(&c.style),
            "table-column-group" => {
                for g in &c.children {
                    if g.display() == "table-column" {
                        out.push(&g.style);
                    }
                }
            }
            _ => {
                if c.kind == crate::BoxKind::AnonymousBlock {
                    collect_col_styles(c, out);
                }
            }
        }
    }
}

/// Span d'une cellule (hints `-x-colspan`/`-x-rowspan`
/// livrés par la cascade, R-47) — ≥ 1, plafond défensif
/// 1000 (borne HTML table model).
fn span_of(st: &ComputedStyle, prop: &str) -> usize {
    st.get(prop)
        .and_then(|v| v.trim().parse::<u32>().ok())
        .map_or(1, |n| usize::try_from(n).unwrap_or(1000).clamp(1, 1000))
}

/// Repère les rangs (`display: table-row` explicites ou
/// anonymes) en traversant les groupes — renvoyés en **ordre
/// visuel** (tête, corps en ordre document, pied), tri stable.
/// §17.2.1 : les `table-cell` nues et tout autre contenu sont
/// emballés dans des cellules/rangs anonymes ; une table
/// **imbriquée** est du contenu (on ne descend pas dedans).
/// Les `col` sont assignés par [`place_cells`] après le tri.
/// Renvoie aussi les légendes (`table-caption`, §17.4).
fn collect_grid(bx: &LayoutBox) -> (Vec<Row>, Captions) {
    /// Cellules d'un rang explicite (enfants `table-cell`
    /// directs — le reste est ignoré, rangs réels seuls).
    fn row_cells(b: &LayoutBox, path: &[usize]) -> Vec<Cell> {
        b.children
            .iter()
            .enumerate()
            .filter(|(_, c)| c.display() == "table-cell")
            .map(|(idx, c)| {
                let mut p = path.to_vec();
                p.push(idx);
                Cell {
                    col: 0,
                    cols: span_of(&c.style, "-x-colspan"),
                    span_rows: span_of(&c.style, "-x-rowspan"),
                    path: p,
                }
            })
            .collect()
    }
    /// Marche une suite d'enfants (table, ou groupe de rangs) :
    /// rangs explicites + accumulation des cellules nues en
    /// rangs anonymes (`path` = préfixe jusqu'au conteneur).
    fn walk_children(
        b: &LayoutBox,
        class: RowClass,
        prefix: &mut Vec<usize>,
        rows: &mut Vec<Row>,
        caps: &mut Captions,
    ) {
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
                // §17.4 : la légende sort de la grille —
                // posée au-dessus/dessous par `layout_table`.
                "table-caption" => {
                    let mut p = prefix.clone();
                    p.push(i);
                    if c.style.get("caption-side") == Some("bottom") {
                        caps.bottom.push(p);
                    } else {
                        caps.top.push(p);
                    }
                }
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
                    explorer_common::deep(|| walk_children(c, g, prefix, rows, caps));
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
                            col: 0,
                            cols: span_of(&c.style, "-x-colspan"),
                            span_rows: span_of(&c.style, "-x-rowspan"),
                            path: p,
                        });
                    }
                }
            }
        }
        flush(&mut pending, rows);
    }
    let mut rows: Vec<Row> = Vec::new();
    let mut caps = Captions::default();
    walk_children(bx, RowClass::Body, &mut Vec::new(), &mut rows, &mut caps);
    let rank = |c: RowClass| match c {
        RowClass::Header => 0,
        RowClass::Body => 1,
        RowClass::Footer => 2,
    };
    rows.sort_by_key(|r| rank(r.class));
    place_cells(&mut rows);
    (rows, caps)
}

/// Assigne `col` à chaque cellule en **ordre visuel**
/// (R-47 — HTML table model §forming a table, simplifié) :
/// curseur de colonne qui saute les colonnes retenues par les
/// `rowspan` des rangs précédents ; un `colspan` réserve sa
/// plage entière (la plage ne peut pas enjamber une colonne
/// retenue — le curseur avance au-delà). `held[c]` = rangs
/// futurs où la colonne `c` reste occupée.
fn place_cells(rows: &mut [Row]) {
    let mut held: Vec<u32> = Vec::new();
    for row in rows.iter_mut() {
        let mut col = 0usize;
        for cell in &mut row.cells {
            loop {
                while held.get(col).copied().unwrap_or(0) > 0 {
                    col += 1;
                }
                if (col..col + cell.cols).any(|c| held.get(c).copied().unwrap_or(0) > 0) {
                    col += 1;
                } else {
                    break;
                }
            }
            cell.col = col;
            let end = col + cell.cols;
            if held.len() < end {
                held.resize(end, 0);
            }
            for h in &mut held[col..end] {
                *h = (*h).max(u32::try_from(cell.span_rows).unwrap_or(u32::MAX));
            }
            col = end;
        }
        for h in &mut held {
            *h = h.saturating_sub(1);
        }
    }
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

/// Les colonnes couvertes par un `colspan` doivent atteindre
/// **collectivement** la mesure de la cellule — le déficit se
/// répartit à parts égales (§17.5.2.2 ; aucune colonne ne
/// redescend sous sa valeur courante).
fn spread_deficit(cols: &mut [f32], need: f32) {
    if cols.is_empty() {
        return;
    }
    let cur: f32 = cols.iter().sum();
    if need > cur {
        let add = (need - cur) / u16::try_from(cols.len()).map_or(1.0, f32::from);
        for w in cols.iter_mut() {
            *w += add;
        }
    }
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
/// retourne la hauteur utilisée. `h_spec` = `height` résolue
/// du conteneur (content-box) — §17.5.3 : elle borne le bas
/// de la table et l'excédent est distribué aux rangs auto.
#[allow(clippy::too_many_lines)]
pub(crate) fn layout_table(
    bx: &mut LayoutBox,
    x: f32,
    y: f32,
    cb_w: f32,
    ctx: &BlockCtx,
    parent_fs: f32,
    h_spec: Option<f32>,
) -> f32 {
    let st = bx.style.clone();
    let fs = font_size_px(&st, parent_fs, ctx);
    let (rows, caps) = collect_grid(bx);
    let ncols = rows
        .iter()
        .flat_map(|r| r.cells.iter())
        .map(|c| c.col + c.cols)
        .max()
        .unwrap_or(0);
    if ncols == 0 {
        // Table sans aucune cellule : le `width`/`height`
        // spécifiés s'appliquent quand même (une table vide
        // imbriquée garde sa taille — Acid2 `li.second-part`).
        let h = h_spec.unwrap_or(0.0);
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
    // `spec` = `width` spécifiée qui cloue sa colonne.
    let mut min_w = vec![0.0_f32; ncols];
    let mut pref_w = vec![1.0_f32; ncols];
    let mut spec_w = vec![0.0_f32; ncols];
    // `table-layout: fixed` : seules les `width` des `col` et
    // des cellules du PREMIER rang fixent les colonnes
    // (§17.5.2.1) — mesurées séparément.
    let fixed_layout = st.get("table-layout").is_some_and(|v| v.trim() == "fixed");
    let mut first_spec = vec![0.0_f32; ncols];
    let mut fixed_auto_pref = 0.0_f32;
    let mut fixed_sum_est = 0.0_f32;
    let mut col_styles: Vec<&ComputedStyle> = Vec::new();
    if fixed_layout {
        collect_col_styles(bx, &mut col_styles);
    }
    // Cellules mono-colonne : max direct sur leur colonne
    // (comportement historique). Puis cellules à `colspan` :
    // les colonnes couvertes doivent **collectivement**
    // atteindre la mesure de la cellule — le déficit se
    // répartit à parts égales (§17.5.2.2).
    for (ri, row) in rows.iter().enumerate() {
        for cell in &row.cells {
            if cell.cols != 1 {
                continue;
            }
            let (min, pref, spec) = cell_widths(bx, &cell.path, ctx, fs);
            min_w[cell.col] = min_w[cell.col].max(min);
            pref_w[cell.col] = pref_w[cell.col].max(pref.max(min));
            spec_w[cell.col] = spec_w[cell.col].max(spec);
            if ri == 0 {
                first_spec[cell.col] = first_spec[cell.col].max(spec);
            }
        }
    }
    for (ri, row) in rows.iter().enumerate() {
        for cell in &row.cells {
            if cell.cols <= 1 {
                continue;
            }
            let (min, pref, spec) = cell_widths(bx, &cell.path, ctx, fs);
            spread_deficit(&mut min_w[cell.col..cell.col + cell.cols], min);
            spread_deficit(&mut pref_w[cell.col..cell.col + cell.cols], pref.max(min));
            spread_deficit(&mut spec_w[cell.col..cell.col + cell.cols], spec);
            if ri == 0 {
                spread_deficit(&mut first_spec[cell.col..cell.col + cell.cols], spec);
            }
        }
    }
    if fixed_layout {
        // Priorité `col` > cellule de premier rang — un `col`
        // sans `width` laisse la cellule décider.
        for (i, s) in spec_w.iter_mut().enumerate() {
            *s = col_styles
                .get(i)
                .and_then(|cs| css_px(cs, "width", cb_w, fs, ctx))
                .unwrap_or(0.0)
                .max(first_spec[i]);
        }
        // Colonnes sans spécification : `pref` nulle → parts
        // égales du reliquat (voir `auto_pref` nul ci-dessous)
        // — leur estimation de contenu est conservée pour le
        // calcul de la largeur `auto` de la table.
        for (i, p) in pref_w.iter_mut().enumerate() {
            if spec_w[i] <= 0.0 {
                fixed_auto_pref += *p;
                *p = 0.0;
            }
        }
        fixed_sum_est = spec_w.iter().sum();
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

    // Largeur de la table : `pref`/`min` sont des sommes de
    // colonnes (content-box) — en `auto` la border-box inclut
    // en sus bordures et padding propres (§17.5.2), sinon le
    // cadre rognerait les colonnes.
    let total_pref: f32 = pref_w.iter().sum();
    let total_min: f32 = min_w.iter().sum();
    let frame = il + ir;
    // En `table-layout:fixed`, une largeur `auto` s'évalue
    // sur les colonnes spécifiées (`col` + premier rang),
    // jamais sur le contenu (§17.5.2.1) — les colonnes
    // restantes conservent une estimation de contenu.
    let auto_w = if fixed_layout {
        fixed_sum_est + fixed_auto_pref + frame
    } else {
        total_pref + frame
    };
    let table_w = css_px(&st, "width", cb_w, fs, ctx)
        .unwrap_or_else(|| auto_w.min(cb_w))
        .min(cb_w);
    let table_w = table_w.max((total_min + frame).min(cb_w));
    let grid_w = (table_w - il - ir).max(0.0);

    // Distribution (§17.5.2.2) : une `width` de cellule cloue
    // sa colonne ; le reste va aux colonnes auto au prorata
    // de `pref` (égalité si tout est vide) ; jamais sous
    // `min` sauf surcontrainte totale (fixes incluses —
    // réduction proportionnelle).
    let fixed_sum: f32 = spec_w.iter().sum();
    let mut col_w = vec![0.0_f32; ncols];
    if fixed_sum >= grid_w && fixed_sum > 0.0 {
        for (i, w) in col_w.iter_mut().enumerate() {
            *w = if spec_w[i] > 0.0 {
                spec_w[i] * grid_w / fixed_sum
            } else {
                min_w[i].min(grid_w)
            };
        }
    } else {
        let mut left = grid_w;
        for (i, w) in col_w.iter_mut().enumerate() {
            *w = spec_w[i].min(grid_w);
            left -= *w;
        }
        let auto_pref: f32 = (0..ncols)
            .map(|i| if spec_w[i] <= 0.0 { pref_w[i] } else { 0.0 })
            .sum();
        let nauto = spec_w.iter().filter(|&&s| s <= 0.0).count();
        if auto_pref <= f32::EPSILON {
            let each = if nauto > 0 {
                left / u16::try_from(nauto).map_or(1.0, f32::from)
            } else {
                0.0
            };
            // Toutes colonnes spécifiées mais table plus large
            // (`table-layout:fixed` typiquement) : le reliquat
            // se répartit également sur toutes (§17.5.2).
            let bonus = if nauto == 0 && left > 0.0 {
                left / u16::try_from(ncols).map_or(1.0, f32::from)
            } else {
                0.0
            };
            for w in &mut col_w {
                *w += bonus;
            }
            for (i, w) in col_w.iter_mut().enumerate() {
                if spec_w[i] <= 0.0 {
                    *w = each.max(min_w[i].min(left));
                }
            }
        } else {
            let mut slack = left;
            for (i, w) in col_w.iter_mut().enumerate() {
                if spec_w[i] > 0.0 {
                    continue;
                }
                *w = (left * pref_w[i] / auto_pref).max(min_w[i].min(left));
                slack -= *w;
            }
            // Reste/retirer : ajuste sur la plus large colonne
            // auto — le retrait ne descend JAMAIS sous `min`
            // (R-47 : un span massif qui gonfle la somme au-
            // delà de la table ne peut pas annuler une
            // colonne voisine ; la table déborde plutôt).
            if slack > 0.0 {
                if let Some(max) = col_w
                    .iter_mut()
                    .enumerate()
                    .filter(|(i, _)| spec_w[*i] <= 0.0)
                    .max_by(|a, b| a.1.total_cmp(b.1))
                    .map(|(_, w)| w)
                {
                    *max += slack;
                }
            } else if slack < -0.01 {
                let mut rem = -slack;
                for _ in 0..ncols {
                    let Some((i, max)) = col_w
                        .iter_mut()
                        .enumerate()
                        .filter(|(i, _)| spec_w[*i] <= 0.0)
                        .max_by(|a, b| a.1.total_cmp(b.1))
                    else {
                        break;
                    };
                    let take = (*max - min_w[i].min(*max)).max(0.0).min(rem);
                    *max -= take;
                    rem -= take;
                    if take <= 0.0 {
                        break;
                    }
                }
            }
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
    // `row_fixed` : une cellule à `height` spécifiée (non-%)
    // rend le rang fixe pour la distribution §17.5.3.
    let mut cursor_y = y + it + gap[0];
    // §17.4 : légendes `caption-side: top` avant la grille.
    for p in &caps.top {
        let cap_cb = Rect {
            x: x + il,
            y: cursor_y,
            width: grid_w,
            height: 0.0,
        };
        let Some(cap) = at_path(bx, p) else {
            continue;
        };
        cursor_y += layout_block(
            cap,
            x + il,
            cursor_y,
            grid_w,
            None,
            ctx,
            fs,
            cap_cb,
            &mut FloatCtx::default(),
        );
    }
    let mut row_tops = Vec::new();
    let mut row_fixed = vec![false; nrows];
    for (k, row) in rows.iter().enumerate() {
        let mut row_h = 0.0_f32;
        for cell in &row.cells {
            let col_x: f32 = x + il + col_w[..cell.col].iter().sum::<f32>();
            // `colspan` : la cellule occupe la somme des
            // colonnes couvertes (R-47).
            let cw: f32 = col_w[cell.col..cell.col + cell.cols].iter().sum();
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
            if cell_box
                .style
                .get("height")
                .is_some_and(|h| h != "auto" && !h.contains('%'))
            {
                row_fixed[k] = true;
            }
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
    // §17.5.3 : `height` spécifiée plus grande que le contenu
    // → l'excédent se répartit sur les rangs **auto**, au
    // prorata de leur hauteur naturelle (les rangs fixés par
    // une `height` de cellule ne bougent pas).
    let natural = cursor_y - y + ib;
    if let Some(hs) = h_spec {
        let extra = hs + it + ib - natural;
        if extra > 0.01 {
            let free_h: f32 = (0..nrows)
                .map(|k| if row_fixed[k] { 0.0 } else { row_tops[k].1 })
                .sum();
            if free_h > 0.0 {
                let mut dy = 0.0;
                for k in 0..nrows {
                    row_tops[k].0 += dy;
                    if !row_fixed[k] {
                        let add = extra * row_tops[k].1 / free_h;
                        row_tops[k].1 += add;
                        dy += add;
                    }
                }
                cursor_y += extra;
            }
        }
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
            // `rowspan` : la boîte peinte couvre la somme des
            // rangs traversés + les arêtes partagées internes
            // (R-47 — la distribution de la hauteur de
            // contenu entre rangs reste hors périmètre).
            let last = (k + cell.span_rows).min(nrows);
            let span_h: f32 = row_tops[k..last].iter().map(|t| t.1).sum::<f32>()
                + gap[k + 1..last].iter().sum::<f32>();
            cell_box.rect.height = cell_box.rect.height.max(span_h.max(rh));
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
    // §17.4 : légendes `caption-side: bottom` après la
    // grille (elles n'entrent pas dans la distribution
    // §17.5.3 de `height`).
    for p in &caps.bottom {
        let cap_cb = Rect {
            x: x + il,
            y: cursor_y,
            width: grid_w,
            height: 0.0,
        };
        let Some(cap) = at_path(bx, p) else {
            continue;
        };
        cursor_y += layout_block(
            cap,
            x + il,
            cursor_y,
            grid_w,
            None,
            ctx,
            fs,
            cap_cb,
            &mut FloatCtx::default(),
        );
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
/// Largeurs min/pref/spec d'une cellule : plus long mot /
/// contenu déroulé + padding/bordure horizontaux, et `width`
/// spécifiée qui cloue la colonne (0.0 si absente).
fn cell_widths(bx: &LayoutBox, cell_path: &[usize], ctx: &BlockCtx, fs: f32) -> (f32, f32, f32) {
    let mut cur = bx;
    for &i in cell_path {
        let Some(c) = cur.children.get(i) else {
            return (0.0, 0.0, 0.0);
        };
        cur = c;
    }
    let cell = cur;
    let st = &cell.style;
    // `width` spécifiée domine la mesure de contenu (CSS2
    // §17.5.2.2 : une largeur fixe de cellule borne la
    // colonne — Acid2 pose `width:1em` sur des cellules
    // vides).
    let spec = css_px(st, "width", 0.0, fs, ctx).unwrap_or(0.0);
    if spec > 0.0 {
        return (spec, spec, spec);
    }
    // Mesure intrinsèque générale (R-32) : les enfants bloc
    // comptent (`width` explicite, sous-arbres récursés) —
    // avant, seul le texte aplati pesait : une cellule à
    // `div{width:1em}` mesurait 0 et sa colonne s'effondrait.
    let cfs = font_size_px(st, fs, ctx);
    let longest = crate::intrinsic::min_content_px(cell, cfs, ctx);
    let pref = crate::intrinsic::max_content_px(cell, cfs, ctx);
    let extras = css_px(st, "padding-left", 0.0, fs, ctx).unwrap_or(0.0)
        + css_px(st, "padding-right", 0.0, fs, ctx).unwrap_or(0.0)
        + border_px(st, "border-left-width", 0.0, fs, ctx)
        + border_px(st, "border-right-width", 0.0, fs, ctx);
    (longest + extras, pref + extras, 0.0)
}
