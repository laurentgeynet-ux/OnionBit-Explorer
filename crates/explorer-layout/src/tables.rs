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

use crate::block::{
    border_px, css_px, font_size_px, layout_block, raw_length_px, side_px, BlockCtx, FloatCtx,
};
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
/// `top`/`bottom` posées au-dessus/dessous de la grille,
/// pleine largeur ; `left`/`right` posées en vis-à-vis —
/// au bord latéral de la boîte table, alignées en haut
/// (R-164, ancien align HTML).
#[derive(Default)]
struct Captions {
    /// `caption-side: top` (défaut).
    top: Vec<Vec<usize>>,
    /// `caption-side: bottom`.
    bottom: Vec<Vec<usize>>,
    /// `caption-side: left` (vis-à-vis gauche, R-164).
    left: Vec<Vec<usize>>,
    /// `caption-side: right` (vis-à-vis droit, R-164).
    right: Vec<Vec<usize>>,
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
        && b.children
            .iter()
            .all(|c| explorer_common::deep(|| ignorable_in_table(c)))
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
                    explorer_common::deep(|| collect_col_styles(c, out));
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
                    match c.style.get("caption-side") {
                        Some("bottom") => caps.bottom.push(p),
                        Some("left") => caps.left.push(p),
                        Some("right") => caps.right.push(p),
                        _ => caps.top.push(p),
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
    // §17.6.1 `border-spacing` (R-159) — consommé seulement
    // en `separate` : `sh` entre colonnes + bords externes,
    // `sv` entre rangs + bords externes. Le `collapse`
    // §17.6.2 ignore la propriété (ses arêtes fusionnent).
    let (sh, sv) = if st
        .get("border-collapse")
        .is_some_and(|v| v.trim() == "collapse")
    {
        (0.0_f32, 0.0_f32)
    } else {
        border_spacing(&st, cb_w, fs, ctx)
    };
    let spacing_x = sh * u16::try_from(ncols + 1).map_or(0.0, f32::from);
    let frame = il + ir + spacing_x;
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
    // Budget des colonnes = grid moins les espacements
    // `border-spacing` (`sh` inter-colonnes + bords
    // externes, R-159) — `cols_budget` alimente la distribution,
    // `grid_w` garde la pleine largeur pour les captions.
    let cols_budget = (grid_w - spacing_x).max(0.0);

    // Distribution (§17.5.2.2) : une `width` de cellule cloue
    // sa colonne ; le reste va aux colonnes auto au prorata
    // de `pref` (égalité si tout est vide) ; jamais sous
    // `min` sauf surcontrainte totale (fixes incluses —
    // réduction proportionnelle).
    let fixed_sum: f32 = spec_w.iter().sum();
    let mut col_w = vec![0.0_f32; ncols];
    if fixed_sum >= cols_budget && fixed_sum > 0.0 {
        for (i, w) in col_w.iter_mut().enumerate() {
            *w = if spec_w[i] > 0.0 {
                spec_w[i] * cols_budget / fixed_sum
            } else {
                min_w[i].min(cols_budget)
            };
        }
    } else {
        let mut left = cols_budget;
        for (i, w) in col_w.iter_mut().enumerate() {
            *w = spec_w[i].min(cols_budget);
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
    } else {
        // `separate` (§17.6.1 / R-159) : `sv` entre chaque
        // rang ET aux bords externes — `gap[0]`/`gap[nrows]`
        // couvrent les extrémités comme sous `collapse`.
        gap.fill(sv);
    }

    // R-179 — arêtes VERTICALES (colonnes) sous `collapse` :
    // `cg[c]` précède la colonne `c`, mêmes règles que
    // `gap[k]` — `max` des `border-right` des cellules finissant
    // en `c-1` et des `border-left` de celles commençant en `c`,
    // bords externes soustraits de la bordure propre de la
    // table (déjà dans `il`/`ir`). Sous `separate`, `sh` (R-159).
    let ncols = col_w.len();
    let mut cg = vec![0.0_f32; ncols + 1];
    if collapse {
        let mut left_w = vec![0.0_f32; ncols];
        let mut right_w = vec![0.0_f32; ncols];
        for row in &rows {
            for cell in &row.cells {
                let mut cc = &*bx;
                let mut ok = true;
                for &i in &cell.path {
                    let Some(n) = cc.children.get(i) else {
                        ok = false;
                        break;
                    };
                    cc = n;
                }
                if ok {
                    left_w[cell.col] = left_w[cell.col].max(border_px(
                        &cc.style,
                        "border-left-width",
                        cb_w,
                        fs,
                        ctx,
                    ));
                    let end = cell.col + cell.cols - 1;
                    right_w[end] =
                        right_w[end].max(border_px(&cc.style, "border-right-width", cb_w, fs, ctx));
                }
            }
        }
        let bl = border_px(&st, "border-left-width", cb_w, fs, ctx);
        let br = border_px(&st, "border-right-width", cb_w, fs, ctx);
        cg[0] = (left_w[0] - bl).max(0.0);
        for c in 1..ncols {
            cg[c] = right_w[c - 1].max(left_w[c]);
        }
        cg[ncols] = (right_w[ncols - 1] - br).max(0.0);
    } else {
        cg.fill(sh);
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
    // Hauteurs posées par cellule — consommées par le
    // `vertical-align` de la passe 2 (R-160).
    let mut cell_used: Vec<Vec<f32>> = Vec::with_capacity(nrows);
    for (k, row) in rows.iter().enumerate() {
        let mut row_h = 0.0_f32;
        let mut used_list = Vec::with_capacity(row.cells.len());
        for cell in &row.cells {
            let col_x: f32 = x
                + il
                + cg[..=cell.col].iter().sum::<f32>()
                + col_w[..cell.col].iter().sum::<f32>();
            // `colspan` : la cellule occupe la somme des
            // colonnes couvertes (R-47) + les arêtes internes.
            let cw: f32 = col_w[cell.col..cell.col + cell.cols].iter().sum::<f32>()
                + cg[cell.col + 1..cell.col + cell.cols].iter().sum::<f32>();
            // Cellule positionnée → conteneur pour ses absolus.
            let cell_cb = Rect {
                x: col_x,
                y: cursor_y,
                width: cw,
                height: 0.0,
            };
            let Some(cell_box) = at_path(bx, &cell.path) else {
                used_list.push(0.0);
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
            used_list.push(used);
        }
        cell_used.push(used_list);
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
        // `tr[valign]` (R-160) : lu sur la rangée — la
        // cellule spécifiée prime (vertical-align n'est pas
        // hérité ; le hint tr existe mais n'atteint pas les
        // cellules sinon).
        let tr_va: Option<String> = row
            .path
            .as_ref()
            .and_then(|p| at_path(bx, p))
            .and_then(|t| t.style.get("vertical-align").map(str::to_string));
        // R-179 — baseline de rangée (§17.5.3.1) : max des
        // baselines de 1re ligne des cellules `baseline` —
        // une cellule sans ligne aligne sur son bas.
        let row_baseline = {
            let mut b = 0.0_f32;
            for cell in &row.cells {
                let Some(cb) = at_path(bx, &cell.path) else {
                    continue;
                };
                let va = cb
                    .style
                    .get("vertical-align")
                    .or(tr_va.as_deref())
                    .unwrap_or("baseline");
                if va == "baseline" {
                    // Lignes sur le conteneur ou son premier
                    // bloc anonyme enfant ; sans ligne, le bas
                    // de la boîte fait baseline (§17.5.3.1).
                    let l = cb
                        .lines
                        .first()
                        .or_else(|| cb.children.iter().find_map(|c| c.lines.first()));
                    b = b.max(
                        l.map_or(cb.rect.y + cb.rect.height, |l| l.rect.y + l.baseline) - cb.rect.y,
                    );
                }
            }
            b
        };
        for (ci, cell) in row.cells.iter().enumerate() {
            let Some(cell_box) = at_path(bx, &cell.path) else {
                continue;
            };
            let p1_top = cell_box.rect.y;
            // `rowspan` : la boîte peinte couvre la somme des
            // rangs traversés + les arêtes partagées internes
            // (R-47 — la distribution de la hauteur de
            // contenu entre rangs reste hors périmètre).
            let last = (k + cell.span_rows).min(nrows);
            let span_h: f32 = row_tops[k..last].iter().map(|t| t.1).sum::<f32>()
                + gap[k + 1..last].iter().sum::<f32>();
            cell_box.rect.height = cell_box.rect.height.max(span_h.max(rh));
            // R-179 — sous `collapse`, la bordure de cellule
            // couvre l'arête partagée entière : les filets des
            // voisins coïncident (1 px exact, ≈ une arête plus
            // épaisse au-delà — le vainqueur spec n'est pas
            // départagé). Le contenu reste à sa place posée.
            if collapse {
                cell_box.rect.x -= cg[cell.col];
                cell_box.rect.width += cg[cell.col] + cg[cell.col + cell.cols];
            }
            cell_box.rect.y = ry;
            // `vertical-align` cellule (R-160, §17.5.3.1) :
            // `middle`/`bottom` décalent le contenu posé dans
            // la hauteur finale (fonds/bordures couvrent
            // toute la rangée). `baseline`/`top` = pas de
            // décalage (baseline réel = lot suivant).
            let cell_h = cell_box.rect.height;
            let used = cell_used
                .get(k)
                .and_then(|r| r.get(ci))
                .copied()
                .unwrap_or(0.0);
            let va = cell_box
                .style
                .get("vertical-align")
                .or(tr_va.as_deref())
                .unwrap_or("baseline");
            let dy = match va {
                "middle" => (cell_h - used) / 2.0,
                "bottom" => cell_h - used,
                // R-179 — `baseline` réel : la 1re ligne de la
                // cellule se cale sur la baseline de rangée
                // (le rang ne grandit pas — approximation si
                // la ligne poussée déborde).
                "baseline" => {
                    let l = cell_box
                        .lines
                        .first()
                        .or_else(|| cell_box.children.iter().find_map(|c| c.lines.first()));
                    let bl = l.map_or(used, |l| l.rect.y + l.baseline - p1_top);
                    (row_baseline - bl).max(0.0)
                }
                _ => 0.0,
            };
            if dy > 0.01 {
                shift_subtree_y(cell_box, dy);
            }
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
    // R-164 — `caption-side: left|right` : la légende se
    // pose en vis-à-vis du bord externe de la table, alignée
    // haut (largeur = sa préférée, bornée à 40% de la
    // table). Elle n'entre pas dans `h` (spec : posée à
    // côté de la boîte bordure — ici bord interne).
    for (side, caps_side) in [(-1.0_f32, &caps.left), (1.0_f32, &caps.right)] {
        for p in caps_side {
            let Some(cap) = at_path(bx, p) else {
                continue;
            };
            let cap_w = (table_w * 0.4).min(grid_w);
            let cap_x = if side < 0.0 { x - cap_w } else { x + table_w };
            let cap_cb = Rect {
                x: cap_x,
                y: y + it,
                width: cap_w,
                height: 0.0,
            };
            layout_block(
                cap,
                cap_x,
                y + it,
                cap_w,
                None,
                ctx,
                fs,
                cap_cb,
                &mut FloatCtx::default(),
            );
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

/// `border-spacing` → `(horizontal, vertical)` — deux
/// longueurs ou une (la seconde = la première, §17.6.1).
/// `0`/invalide → `0` — consommé seulement en `separate`
/// (R-159 ; `collapse` fusionne via `gap[]` §17.6.2).
fn border_spacing(st: &ComputedStyle, cb_w: f32, fs: f32, ctx: &BlockCtx) -> (f32, f32) {
    let mut it = st
        .get("border-spacing")
        .into_iter()
        .flat_map(|v| v.split_whitespace());
    let h = it.next().and_then(|t| raw_length_px(t, cb_w, fs, ctx));
    let v = it.next().and_then(|t| raw_length_px(t, cb_w, fs, ctx));
    match (h, v) {
        (Some(h), Some(v)) => (h, v),
        (Some(h), None) => (h, h),
        _ => (0.0, 0.0),
    }
}

/// Décale `dy` tout le contenu posé d'une boîte — lignes
/// inline (fragments) et sous-arbres enfants, récursif.
/// La boîte elle-même garde sa géométrie : fonds et
/// bordures couvrent la rangée entière — utilisé par
/// `vertical-align` de cellule (R-160, §17.5.3.1).
fn shift_subtree_y(bx: &mut LayoutBox, dy: f32) {
    for line in &mut bx.lines {
        line.rect.y += dy;
        for frag in &mut line.fragments {
            frag.rect.y += dy;
        }
    }
    for child in &mut bx.children {
        child.rect.y += dy;
        explorer_common::deep(|| shift_subtree_y(child, dy));
    }
}
