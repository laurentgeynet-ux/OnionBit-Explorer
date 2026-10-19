// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Block layout (étape 19) : flux normal CSS 2.1.
//!
//! - **Horizontal** : `width` + marges/padding/bordures
//!   résolus contre la largeur du bloc conteneur ; marges
//!   `auto` absorbent le slack ; surcontrainte → `margin-right`
//!   ajustée (flux LTR, §10.3.3).
//! - **Vertical** : enfants de niveau bloc empilés, marges
//!   adjacentes **collapsées** (§8.3.1) — entre frères, et
//!   parent/premier|dernier enfant quand rien ne sépare.
//! - **Hauteur** : `height` spécifiée, sinon somme du flux ;
//!   `min|max-width|height` bornent (`clamp`).
//! - **Contenu en-ligne** (blocs anonymes) : hauteur estimée
//!   par `break_lines` — la pose des glyphes/lignes fines est
//!   l'étape 20 ; ici on ne besoin que des hauteurs de ligne.
//!
//! Fonte : [`BlockCtx::fonts`] — le port `FontSource` est
//! injecté par `core` ; sans source, repli métrique
//! documenté (`0.55em`/caractère) qui ne sert qu'au dev
//! headless, jamais en production.

use explorer_css::value::{parse_length, Unit};
use explorer_css::ComputedStyle;

use crate::boxtree::{BoxKind, Float, LayoutBox, Overflow, Position, Rect};
use crate::font::{self, FontRequest, FontSource};

/// Contexte de dimensionnement (une passe de layout).
pub struct BlockCtx<'a> {
    /// Source de fontes injectée (ADR-0009) — `None` en dev
    /// headless sans fontes (repli métrique documenté).
    pub fonts: Option<&'a dyn FontSource>,
    /// Famille de repli quand `font-family` n'en nomme aucune
    /// connue de la source.
    pub default_family: &'a str,
    /// Font-size de base (px) pour `rem` et la racine.
    pub base_font_size: f32,
    /// Hauteur du viewport — bloc conteneur des `fixed` et
    /// base des `vh` (étape 21).
    pub viewport_h: f32,
}

impl Default for BlockCtx<'_> {
    fn default() -> Self {
        Self {
            fonts: None,
            default_family: "sans-serif",
            base_font_size: 16.0,
            viewport_h: 600.0,
        }
    }
}

/// Marges verticales usées d'une boîte (px).
#[derive(Debug, Clone, Copy)]
struct Margins {
    top: f32,
    bottom: f32,
}

/// Effondrement de deux marges adjacentes (§8.3.1).
fn collapse(a: f32, b: f32) -> f32 {
    match (a >= 0.0, b >= 0.0) {
        (true, true) => a.max(b),
        (false, false) => a.min(b),
        _ => a + b,
    }
}

/// Dimensionne l'arbre de boîtes dans un viewport de largeur
/// `viewport_w` : chaque `rect` est rempli (x, y, border-box
/// width/height). Le `y` de départ est 0.
pub fn layout_tree(root: &mut LayoutBox, viewport_w: f32, ctx: &BlockCtx) {
    let viewport = Rect {
        x: 0.0,
        y: 0.0,
        width: viewport_w,
        height: ctx.viewport_h,
    };
    layout_block(
        root,
        0.0,
        0.0,
        viewport_w,
        ctx,
        ctx.base_font_size,
        viewport,
    );
    resolve_absolutes(root, viewport, ctx, ctx.base_font_size);
}

/// Résout `font-size` en px : `em`/`rem` relatifs au parent
/// (ou à la base), `%` idem, absolus directs.
pub(crate) fn font_size_px(style: &ComputedStyle, parent_fs: f32, ctx: &BlockCtx) -> f32 {
    match css_px(style, "font-size", parent_fs, parent_fs, ctx) {
        Some(v) if v > 0.0 => v,
        _ => parent_fs.max(1.0),
    }
}

/// `prop` → px. `pct_basis` = base des pourcents (largeur du
/// bloc conteneur ou font-size selon la propriété) ;
/// `em_basis` = base des unités `em`.
/// `auto`/`medium`/inconnu → `None`.
pub(crate) fn css_px(
    style: &ComputedStyle,
    prop: &str,
    pct_basis: f32,
    em_basis: f32,
    ctx: &BlockCtx,
) -> Option<f32> {
    raw_length_px(style.get(prop)?, pct_basis, em_basis, ctx)
}

/// Même résolution que [`css_px`] mais sur la valeur brute —
/// pour les syntaxes multi-valeurs (`flex`, `gap`…) qui ne
/// sont pas une propriété unique.
pub(crate) fn raw_length_px(
    raw: &str,
    pct_basis: f32,
    em_basis: f32,
    ctx: &BlockCtx,
) -> Option<f32> {
    let raw = raw.trim();
    if let Some(pct) = raw.strip_suffix('%') {
        return pct
            .trim()
            .parse::<f32>()
            .ok()
            .map(|v| v * pct_basis / 100.0);
    }
    // `calc(...)` : mini-évaluateur — termes résolus en px
    // avec les mêmes bases que ci-dessous (`%` → `pct_basis`,
    // `em` → `em_basis`…), opérateurs `+ - * /` et parenthèses.
    if raw.len() > 5 && raw[..5].eq_ignore_ascii_case("calc(") && raw.ends_with(')') {
        return eval_calc(&raw[5..raw.len() - 1], pct_basis, em_basis, ctx);
    }
    let len = parse_length(raw)?;
    #[allow(clippy::cast_possible_truncation)]
    Some(match len.unit {
        Unit::Em => len.value as f32 * em_basis,
        Unit::Rem => len.value as f32 * ctx.base_font_size,
        Unit::Ex | Unit::Ch => len.value as f32 * em_basis * 0.5,
        u => (len.value * u.px_factor()?) as f32,
    })
}

/// Évalue l'intérieur d'un `calc(...)` en px : `term ((+|-) term)*`,
/// `term = factor ((*|/) factor)*`, `factor = '(' expr ')' | dim`.
/// `N%` → base `pct_basis` ; unités comme `css_px` ; nombre nu
/// toléré comme px (spec : invalide hors `*`/`/` — on dégrade).
/// `None` sur toute irrégularité (jamais de panique).
#[allow(clippy::too_many_lines)]
fn eval_calc(expr: &str, pct_basis: f32, em_basis: f32, ctx: &BlockCtx) -> Option<f32> {
    // Un nombre nu n'est pas une dimension : on le note pour
    // interdire `1px + 2` et `a * b` entre deux dimensions.
    #[derive(Clone, Copy)]
    struct Term {
        px: f64,
        dim: bool,
    }
    fn dim_px(text: &str, pct: f32, em: f32, ctx: &BlockCtx) -> Option<Term> {
        let t = text.trim();
        if let Some(p) = t.strip_suffix('%') {
            return p.trim().parse::<f64>().ok().map(|v| Term {
                px: v * f64::from(pct) / 100.0,
                dim: true,
            });
        }
        if let Some(l) = parse_length(t) {
            let px = match l.unit {
                Unit::Em => l.value * f64::from(em),
                Unit::Rem => l.value * f64::from(ctx.base_font_size),
                Unit::Ex | Unit::Ch => l.value * f64::from(em) * 0.5,
                u => l.value * u.px_factor()?,
            };
            return Some(Term { px, dim: true });
        }
        t.parse::<f64>().ok().map(|px| Term { px, dim: false })
    }
    fn factor(s: &str, i: &mut usize, pct: f32, em: f32, ctx: &BlockCtx) -> Option<Term> {
        let b = s.as_bytes();
        while *i < b.len() && b[*i].is_ascii_whitespace() {
            *i += 1;
        }
        if *i >= b.len() {
            return None;
        }
        // `calc(` imbriqué ≡ parenthèse (le préfixe est sauté).
        let calc_open =
            s[*i..].len() >= 5 && s[*i..*i + 4].eq_ignore_ascii_case("calc") && b[*i + 4] == b'(';
        if calc_open {
            *i += 4;
        }
        if b[*i] == b'(' {
            *i += 1;
            let t = expr_in(s, i, pct, em, ctx)?;
            while *i < b.len() && b[*i].is_ascii_whitespace() {
                *i += 1;
            }
            if *i >= b.len() || b[*i] != b')' {
                return None;
            }
            *i += 1;
            return Some(t);
        }
        let start = *i;
        // `-` n'est PAS un délimiteur : le signe colle au nombre
        // (« -5px ») — « 10px-5px » sans espaces forme un seul
        // token invalide (spec : espaces requis autour de ±).
        while *i < b.len() && !matches!(b[*i], b'+' | b'*' | b'/' | b')' | b' ' | b'\t') {
            *i += 1;
        }
        dim_px(&s[start..*i], pct, em, ctx)
    }
    fn term(s: &str, i: &mut usize, pct: f32, em: f32, ctx: &BlockCtx) -> Option<Term> {
        let mut acc = factor(s, i, pct, em, ctx)?;
        loop {
            let b = s.as_bytes();
            while *i < b.len() && b[*i].is_ascii_whitespace() {
                *i += 1;
            }
            if *i >= b.len() || !matches!(b[*i], b'*' | b'/') {
                return Some(acc);
            }
            let div = b[*i] == b'/';
            *i += 1;
            let rhs = factor(s, i, pct, em, ctx)?;
            acc = match (div, acc.dim, rhs.dim) {
                // `*` : un seul côté dimensionné ; `/` : diviseur nu.
                (false, true, false) | (false, false, _) => Term {
                    px: acc.px * rhs.px,
                    dim: acc.dim || rhs.dim,
                },
                (true, _, false) if rhs.px != 0.0 => Term {
                    px: acc.px / rhs.px,
                    dim: acc.dim,
                },
                _ => return None,
            };
        }
    }
    fn expr_in(s: &str, i: &mut usize, pct: f32, em: f32, ctx: &BlockCtx) -> Option<Term> {
        let mut acc = term(s, i, pct, em, ctx)?;
        loop {
            let b = s.as_bytes();
            while *i < b.len() && b[*i].is_ascii_whitespace() {
                *i += 1;
            }
            if *i >= b.len() || !matches!(b[*i], b'+' | b'-') {
                return Some(acc);
            }
            let neg = b[*i] == b'-';
            *i += 1;
            let rhs = term(s, i, pct, em, ctx)?;
            if acc.dim != rhs.dim {
                return None; // dim ± nombre nu : invalide
            }
            acc.px = if neg {
                acc.px - rhs.px
            } else {
                acc.px + rhs.px
            };
        }
    }
    let mut i = 0;
    let out = expr_in(expr, &mut i, pct_basis, em_basis, ctx)?;
    while i < expr.len() && expr.as_bytes()[i].is_ascii_whitespace() {
        i += 1;
    }
    if i == expr.len() && out.dim {
        #[allow(clippy::cast_possible_truncation)]
        return Some(out.px as f32);
    }
    None
}

/// Côté de marge/bordure/padding en px (`auto` et manquant →
/// `fallback`).
fn side_px(
    style: &ComputedStyle,
    prop: &str,
    cb_w: f32,
    em: f32,
    ctx: &BlockCtx,
    fallback: f32,
) -> f32 {
    css_px(style, prop, cb_w, em, ctx).unwrap_or(fallback)
}

/// Marges verticales d'une boîte (pour l'effondrement).
fn margins(style: &ComputedStyle, cb_w: f32, em: f32, ctx: &BlockCtx) -> Margins {
    Margins {
        top: side_px(style, "margin-top", cb_w, em, ctx, 0.0),
        bottom: side_px(style, "margin-bottom", cb_w, em, ctx, 0.0),
    }
}

/// Dimensionne `bx` (boîte de niveau bloc) dans le bloc
/// conteneur de largeur `cb_w`, bord haut à `y`. Pose
/// `bx.rect` et descend récursivement. `abs_cb` = bloc
/// conteneur du positionnement absolu (plus proche ancêtre
/// positionné, ou viewport).
#[allow(clippy::too_many_lines)]
pub(crate) fn layout_block(
    bx: &mut LayoutBox,
    x: f32,
    y: f32,
    cb_w: f32,
    ctx: &BlockCtx,
    parent_fs: f32,
    abs_cb: Rect,
) -> f32 {
    let st = bx.style.clone();
    let fs = font_size_px(&st, parent_fs, ctx);

    apply_layout_metadata(bx, &st);

    // `display: table` → dimensionnement dédié.
    if st.get("display") == Some("table") {
        return crate::tables::layout_table(bx, x, y, cb_w, ctx, parent_fs);
    }

    let ml = side_px(&st, "margin-left", cb_w, fs, ctx, 0.0);
    let mr = side_px(&st, "margin-right", cb_w, fs, ctx, 0.0);
    let bl = side_px(&st, "border-left-width", cb_w, fs, ctx, 0.0);
    let br = side_px(&st, "border-right-width", cb_w, fs, ctx, 0.0);
    let pl = side_px(&st, "padding-left", cb_w, fs, ctx, 0.0);
    let pr = side_px(&st, "padding-right", cb_w, fs, ctx, 0.0);
    let bt = side_px(&st, "border-top-width", cb_w, fs, ctx, 0.0);
    let bb = side_px(&st, "border-bottom-width", cb_w, fs, ctx, 0.0);
    let pt = side_px(&st, "padding-top", cb_w, fs, ctx, 0.0);
    let pb = side_px(&st, "padding-bottom", cb_w, fs, ctx, 0.0);

    // §10.3.3 : width spécifiée → le slack (éventuellement
    // négatif → débordement) va aux marges `auto` ; tout
    // spécifié → `margin-right` reprend le reste (flux LTR).
    // `auto` hors width → marge 0 (déjà le repli `side_px`).
    // Élément remplacé (`img`) : la taille préférée fait
    // office de `width`/`height` quand le CSS est `auto`
    // (§10.3.2 — attr/dimensions intrinsèques déjà résolus
    // en amont, chaque axe indépendamment).
    let w_spec = css_px(&st, "width", cb_w, fs, ctx).or(bx.intrinsic.map(|i| i.0));
    let auto_left = st.get("margin-left") == Some("auto");
    let auto_right = st.get("margin-right") == Some("auto");
    let fixed = bl + br + pl + pr;
    let (ml_used, _mr_used, content_w) = match w_spec {
        Some(w) => {
            let slack = cb_w - fixed - w;
            match (auto_left, auto_right) {
                (true, true) => {
                    let each = slack / 2.0;
                    (each, slack - each, w)
                }
                (true, false) => (slack - mr, mr, w),
                (false, true) => (ml, slack - ml, w),
                (false, false) => (ml, slack - ml - mr, w),
            }
        }
        None => (ml, mr, (cb_w - fixed - ml - mr).max(0.0)),
    };

    let rect_x = x + ml_used;
    bx.rect = Rect {
        x: rect_x,
        y,
        width: bl + pl + content_w + pr + br,
        height: 0.0,
    };
    let content_x = rect_x + bl + pl;
    let cursor = y + bt + pt;

    // Conteneur des descendants positionnés : padding box de
    // cette boîte si elle est positionnée, sinon hérité.
    let pos_cb = if bx.position == Position::Static {
        abs_cb
    } else {
        Rect {
            x: content_x - pl,
            y: y + bt,
            width: pl + content_w + pr,
            height: 0.0,
        }
    };

    let line_h = line_height(&st, fs, ctx);
    let cursor = match st.get("display") {
        Some("flex" | "inline-flex") => {
            crate::flex::flow_flex_children(bx, content_x, cursor, content_w, ctx, fs, pos_cb)
        }
        Some("grid" | "inline-grid") => {
            crate::grid::flow_grid_children(bx, content_x, cursor, content_w, ctx, fs, pos_cb)
        }
        _ => flow_children(
            bx,
            content_x,
            cursor,
            content_w,
            pt + bt <= 0.0,
            line_h,
            ctx,
            fs,
            pos_cb,
        ),
    };

    let content_h = cursor - (y + bt + pt);
    let h_spec = css_px(&st, "height", ctx.base_font_size, fs, ctx).or(bx.intrinsic.map(|i| i.1));
    let mut used_h = h_spec.unwrap_or(content_h);
    if let Some(mn) = css_px(&st, "min-height", cb_w, fs, ctx) {
        used_h = used_h.max(mn);
    }
    if let Some(mx) = css_px(&st, "max-height", cb_w, fs, ctx) {
        used_h = used_h.min(mx);
    }
    let w = {
        let mut w = bl + pl + content_w + pr + br;
        if let Some(mn) = css_px(&st, "min-width", cb_w, fs, ctx) {
            w = w.max(mn);
        }
        if let Some(mx) = css_px(&st, "max-width", cb_w, fs, ctx) {
            w = w.min(mx);
        }
        w
    };
    bx.rect.width = w;
    bx.rect.height = bt + pt + used_h + pb + bb;

    // `position: relative` : décalage après le flux
    // (top pousse vers le bas, bottom vers le haut).
    if bx.position == Position::Relative {
        let off_x = css_px(&st, "left", cb_w, fs, ctx).unwrap_or(0.0)
            - css_px(&st, "right", cb_w, fs, ctx).unwrap_or(0.0);
        let off_y = css_px(&st, "top", cb_w, fs, ctx).unwrap_or(0.0)
            - css_px(&st, "bottom", cb_w, fs, ctx).unwrap_or(0.0);
        bx.rect.x += off_x;
        bx.rect.y += off_y;
    }
    bx.rect.height
}

/// Métadonnées usées étape 21 : `position`, `float`,
/// `overflow` (axes x/y combinés — la valeur la plus
/// restrictive gagne) et `z-index`, lus du style calculé.
fn apply_layout_metadata(bx: &mut LayoutBox, st: &ComputedStyle) {
    bx.position = match st.get("position") {
        Some("relative") => Position::Relative,
        Some("absolute") => Position::Absolute,
        Some("fixed") => Position::Fixed,
        Some("sticky") => Position::Sticky,
        _ => Position::Static,
    };
    bx.float = match st.get("float") {
        Some("left") => Float::Left,
        Some("right") => Float::Right,
        _ => Float::None,
    };
    let ov = |axis: &str| match st.get(axis) {
        Some("hidden") => 3,
        Some("auto") => 2,
        Some("scroll") => 1,
        _ => 0,
    };
    bx.overflow = match ov("overflow-x").max(ov("overflow-y")) {
        3 => Overflow::Hidden,
        2 => Overflow::Auto,
        1 => Overflow::Scroll,
        _ => Overflow::Visible,
    };
    bx.z_index = st
        .get("z-index")
        .and_then(|z| z.trim().parse::<i32>().ok())
        .unwrap_or(0);
}

/// Bande libre `cx..cx+cw` rétrécie par les flottants qui
/// mordent à `top` : retourne `(x, largeur)` effectifs.
fn float_band(fl: &[Rect], fr: &[Rect], top: f32, cx: f32, cw: f32) -> (f32, f32) {
    let mut xl = cx;
    let mut xr = cx + cw;
    for f in fl {
        if f.y <= top && f.y + f.height > top {
            xl = xl.max(f.x + f.width);
        }
    }
    for f in fr {
        if f.y <= top && f.y + f.height > top {
            xr = xr.min(f.x);
        }
    }
    (xl, (xr - xl).max(0.0))
}

/// `clear` : descend `cursor` sous le bas des flottants.
fn float_clear(floats: &[Rect], cursor: &mut f32) {
    if let Some(b) = floats.iter().map(|f| f.y + f.height).reduce(f32::max) {
        *cursor = (*cursor).max(b);
    }
}

/// Empilement vertical des enfants de niveau bloc : marges
/// collapsées, flottants posés au bord (bandes réservées),
/// `clear`, absolus laissés hors flux (rect statique
/// placeholder pour `resolve_absolutes`). Retourne le curseur
/// final (bas du contenu).
#[allow(clippy::too_many_arguments)]
fn flow_children(
    bx: &mut LayoutBox,
    content_x: f32,
    start_cursor: f32,
    content_w: f32,
    adjoining_top: bool,
    line_h: f32,
    ctx: &BlockCtx,
    fs: f32,
    pos_cb: Rect,
) -> f32 {
    let mut cursor = start_cursor;
    // Bandes réservées (rect margin-compris) par côté — le
    // contenu suivant se rétrécit où ils mordent
    // (approximation : bande prise au bord haut de l'enfant —
    // affinage ligne par ligne reporté, documenté).
    let mut floats_l: Vec<Rect> = Vec::new();
    let mut floats_r: Vec<Rect> = Vec::new();

    let mut prev_margin: Option<f32> = None;
    let mut first = true;
    let count = bx.children.len();
    for i in 0..count {
        let (kind, cst) = {
            let c = &bx.children[i];
            (c.kind, c.style.clone())
        };
        if kind.is_inline_level() {
            cursor += line_h;
            continue;
        }
        let cfs = font_size_px(&cst, fs, ctx);
        let cm = margins(&cst, content_w, cfs, ctx);

        // `position: absolute|fixed` : hors flux — rect
        // statique placeholder, résolu en second passage par
        // `resolve_absolutes` (layout_tree).
        if matches!(cst.get("position"), Some("absolute" | "fixed")) {
            bx.children[i].rect = Rect {
                x: content_x,
                y: cursor,
                width: content_w,
                height: 0.0,
            };
            continue;
        }

        // `clear` : le flux descend sous les flottants du côté.
        match cst.get("clear") {
            Some("left") => float_clear(&floats_l, &mut cursor),
            Some("right") => float_clear(&floats_r, &mut cursor),
            Some("both") => {
                float_clear(&floats_l, &mut cursor);
                float_clear(&floats_r, &mut cursor);
            }
            _ => {}
        }

        // `float` : posé au bord de la bande courante,
        // enregistré — n'avance pas le curseur.
        let cfloat = cst.get("float");
        if kind != BoxKind::AnonymousBlock && matches!(cfloat, Some("left" | "right")) {
            let (fx0, fw) = float_band(&floats_l, &floats_r, cursor, content_x, content_w);
            let child = &mut bx.children[i];
            layout_block(child, fx0, cursor, fw, ctx, fs, pos_cb);
            let frect = child.rect;
            if cfloat == Some("right") {
                child.rect.x = fx0 + fw - frect.width;
                floats_r.push(frect);
            } else {
                floats_l.push(frect);
            }
            continue;
        }

        // Premier enfant adjoining : sa marge « traverse » le
        // parent (§8.3.1 — pas d'espace interne au-dessus ;
        // c'est le parent qui ajoute la sienne propre, côté
        // appelant, via `margins()`).
        let gap = if first && adjoining_top {
            0.0
        } else {
            prev_margin.map_or(cm.top, |p| collapse(p, cm.top))
        };
        cursor += gap;
        // Bande effective au bord haut de l'enfant.
        let (ex, ew) = float_band(&floats_l, &floats_r, cursor, content_x, content_w);
        let child = &mut bx.children[i];
        let used = match kind {
            BoxKind::AnonymousBlock => layout_anon_block(child, ex, cursor, ew, ctx, fs, pos_cb),
            _ => layout_block(child, ex, cursor, ew, ctx, fs, pos_cb),
        };
        cursor += used;
        prev_margin = Some(cm.bottom);
        first = false;
    }
    cursor
}

/// Second passage (étape 21) : pose les boîtes
/// `position: absolute|fixed` laissées hors flux, contre leur
/// bloc conteneur — padding box du plus proche ancêtre
/// positionné (hauteurs désormais connues), viewport pour
/// `fixed`. Appelée par `layout_tree` après le flux.
pub(crate) fn resolve_absolutes(bx: &mut LayoutBox, cb: Rect, ctx: &BlockCtx, fs: f32) {
    // Bloc conteneur des enfants absolus de cette boîte.
    let own_cb = if bx.position == Position::Static {
        cb
    } else {
        Rect {
            x: bx.rect.x,
            y: bx.rect.y,
            width: bx.rect.width,
            height: bx.rect.height,
        }
    };
    let viewport = Rect {
        x: 0.0,
        y: 0.0,
        width: own_cb.width.max(cb.width),
        height: ctx.viewport_h,
    };
    let count = bx.children.len();
    for i in 0..count {
        // Lire le STYLE (`c.position` reste `Static` tant que
        // l'enfant n'a pas été posé — bootstrap du champ).
        let (pos, cst) = {
            let c = &bx.children[i];
            let p = match c.style.get("position") {
                Some("absolute") => Position::Absolute,
                Some("fixed") => Position::Fixed,
                _ => Position::Static,
            };
            (p, c.style.clone())
        };
        if !matches!(pos, Position::Absolute | Position::Fixed) {
            continue;
        }
        let cfs = font_size_px(&cst, fs, ctx);
        let target = if pos == Position::Fixed {
            viewport
        } else {
            own_cb
        };
        let static_x = bx.children[i].rect.x;
        let static_y = bx.children[i].rect.y;
        let l = css_px(&cst, "left", target.width, cfs, ctx);
        let r = css_px(&cst, "right", target.width, cfs, ctx);
        let t = css_px(&cst, "top", target.height, cfs, ctx);
        let b = css_px(&cst, "bottom", target.height, cfs, ctx);
        let cw = match (l, r) {
            (Some(l), Some(r)) => (target.width - l - r).max(0.0),
            (Some(l), None) => (target.width - l).max(0.0),
            _ => target.width,
        };
        let cx = l.map_or(static_x, |v| target.x + v);
        let cy = t.map_or(static_y, |v| target.y + v);
        let child = &mut bx.children[i];
        layout_block(child, cx, cy, cw, ctx, fs, target);
        // right/bottom sans left/top : ancrage au bord opposé.
        if l.is_none() {
            if let Some(rv) = r {
                child.rect.x = target.x + target.width - rv - child.rect.width;
            }
        }
        if t.is_none() {
            if let Some(bv) = b {
                child.rect.y = target.y + target.height - bv - child.rect.height;
            }
        }
    }
    // Descendre dans tous les enfants (cb mis à jour).
    for child in &mut bx.children {
        resolve_absolutes(child, own_cb, ctx, fs);
    }
}

/// Hauteur d'un bloc anonyme : délégué au layout en-ligne
/// (étape 20 — vraies line boxes dans `bx.lines`).
pub(crate) fn layout_anon_block(
    bx: &mut LayoutBox,
    x: f32,
    y: f32,
    w: f32,
    ctx: &BlockCtx,
    parent_fs: f32,
    abs_cb: Rect,
) -> f32 {
    bx.rect = Rect {
        x,
        y,
        width: w,
        height: 0.0,
    };
    crate::inline::layout_inline(bx, x, y, w, ctx, parent_fs, abs_cb)
}

/// `line-height` en px : nombre → ×font-size, longueur →
/// directe, `normal` → 1.2em (arrêté commun des UA).
fn line_height(style: &ComputedStyle, fs: f32, ctx: &BlockCtx) -> f32 {
    let Some(raw) = style.get("line-height") else {
        return fs * 1.2;
    };
    let raw = raw.trim();
    if raw == "normal" {
        return fs * 1.2;
    }
    if let Ok(n) = raw.parse::<f32>() {
        return (n * fs).max(1.0);
    }
    css_px(style, "line-height", fs, fs, ctx).unwrap_or(fs * 1.2)
}

/// Fonction de mesure px d'une chaîne, par la fonte réelle
/// via `FontSource`, sinon repli `0.55em`/caractère (dev
/// headless documenté — jamais la production).
pub(crate) fn measure_fn<'a>(
    style: &'a ComputedStyle,
    fs: f32,
    ctx: &'a BlockCtx,
) -> impl Fn(&str) -> f32 + 'a {
    let font_data = ctx.fonts.and_then(|src| {
        let families: Vec<String> = style.get("font-family").map_or_else(
            || vec![ctx.default_family.to_string()],
            |f| {
                f.split(',')
                    .map(|s| s.trim().trim_matches(|c| c == '"' || c == '\'').to_string())
                    .collect()
            },
        );
        let weight = style
            .get("font-weight")
            .and_then(|w| match w.trim() {
                "bold" | "bolder" => Some(700u16),
                "normal" | "lighter" => Some(400),
                v => v.parse().ok(),
            })
            .unwrap_or(400);
        let italic = matches!(style.get("font-style"), Some("italic" | "oblique"));
        let req = FontRequest {
            families,
            weight,
            italic,
        };
        src.font_data(&req)
    });
    #[allow(clippy::cast_precision_loss)]
    move |s: &str| {
        if let Some(d) = &font_data {
            font::measure(d, s, fs)
        } else {
            s.chars().count() as f32 * fs * 0.55
        }
    }
}
