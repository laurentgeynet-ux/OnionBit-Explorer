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

/// Registre des flottants d'un contexte de formatage bloc :
/// les **marge-rects** posés, partagés par toutes les boîtes
/// du même BFC — un enfant qui établit son propre BFC repart
/// d'un registre vierge (§9.4.1 : ses flottants ne mordent
/// pas l'extérieur, et réciproquement).
#[derive(Default)]
pub(crate) struct FloatCtx {
    /// Flottants gauche (marge-rects).
    pub left: Vec<Rect>,
    /// Flottants droit (marge-rects).
    pub right: Vec<Rect>,
}

impl FloatCtx {
    /// Bande libre `cx..cx+cw` rétrécie par les flottants qui
    /// mordent à `top` : retourne `(x, largeur)` effectifs.
    pub(crate) fn band(&self, top: f32, cx: f32, cw: f32) -> (f32, f32) {
        let mut xl = cx;
        let mut xr = cx + cw;
        for f in &self.left {
            if f.y <= top && f.y + f.height > top {
                xl = xl.max(f.x + f.width);
            }
        }
        for f in &self.right {
            if f.y <= top && f.y + f.height > top {
                xr = xr.min(f.x);
            }
        }
        (xl, (xr - xl).max(0.0))
    }

    /// Bas le plus profond des flottants `side` (`None` si
    /// aucun) — pour `clear` et la containment BFC.
    pub(crate) fn bottom_of(list: &[Rect]) -> Option<f32> {
        list.iter().map(|f| f.y + f.height).reduce(f32::max)
    }

    /// Haut le plus élevé des flottants déjà posés (§9.5.1
    /// règle 5 : un nouveau flottant ne peut pas être plus
    /// haut que le HAUT de tout flottant antérieur — les
    /// blocs in-flow antérieurs sont déjà couverts par la
    /// position de flux, qui est sous leurs hauts).
    fn max_top(&self) -> Option<f32> {
        self.left
            .iter()
            .chain(self.right.iter())
            .map(|f| f.y)
            .reduce(f32::max)
    }

    /// `clear` : position minimale sous les flottants du côté.
    pub(crate) fn cleared(&self, side: Option<&str>, y: f32) -> f32 {
        match side {
            Some("left") => Self::bottom_of(&self.left).map_or(y, |b| y.max(b)),
            Some("right") => Self::bottom_of(&self.right).map_or(y, |b| y.max(b)),
            Some("both") => {
                let l = Self::bottom_of(&self.left).unwrap_or(f32::NEG_INFINITY);
                let r = Self::bottom_of(&self.right).unwrap_or(f32::NEG_INFINITY);
                y.max(l.max(r))
            }
            _ => y,
        }
    }

    /// Premier `y > top` où la bande change — bas du flottant
    /// mordant `top` qui finit le plus tôt (`None` si la
    /// bande ne change plus).
    pub(crate) fn next_band_change(&self, top: f32) -> Option<f32> {
        self.left
            .iter()
            .chain(self.right.iter())
            .filter(|f| f.y <= top && f.y + f.height > top)
            .map(|f| f.y + f.height)
            .reduce(f32::min)
    }

    /// Bande effective sur un intervalle `[top, top+h]` :
    /// intersection des bandes — tout flottant dont l'étendue
    /// verticale touche l'intervalle contraint. C'est la
    /// bande qu'une boîte de hauteur `h` doit respecter pour
    /// ne chevaucher aucun flottant (§9.5 : la border box
    /// d'un BFC tient sur TOUTE sa hauteur).
    pub(crate) fn band_over(&self, top: f32, h: f32, cx: f32, cw: f32) -> (f32, f32) {
        if h <= 0.0 {
            return self.band(top, cx, cw);
        }
        let bottom = top + h;
        let mut xl = cx;
        let mut xr = cx + cw;
        for f in &self.left {
            if f.y < bottom && f.y + f.height > top {
                xl = xl.max(f.x + f.width);
            }
        }
        for f in &self.right {
            if f.y < bottom && f.y + f.height > top {
                xr = xr.min(f.x);
            }
        }
        (xl, (xr - xl).max(0.0))
    }

    /// Premier bas de flottant `> y` parmi ceux dont l'étendue
    /// intersecte `[y, y+h]` — descente minimale qui élargit
    /// la bande (un flottant qui finit libère sa morsure).
    pub(crate) fn first_bottom_within(&self, y: f32, h: f32) -> Option<f32> {
        let bottom = y + h;
        self.left
            .iter()
            .chain(self.right.iter())
            .filter(|f| f.y < bottom && f.y + f.height > y)
            .map(|f| f.y + f.height)
            .filter(|b| *b > y)
            .reduce(f32::min)
    }
}

/// `true` si la boîte établit un contexte de formatage bloc
/// propre (CSS 2.1 §9.4.1 + Display L3 `flow-root`) : floats,
/// absolus, inline-blocks, `overflow ≠ visible`, flow-root,
/// cellules/légendes de table, conteneurs flex/grid.
fn establishes_bfc(st: &ComputedStyle) -> bool {
    matches!(
        st.get("display"),
        Some(
            "flow-root"
                | "inline-block"
                | "inline-flex"
                | "inline-grid"
                | "inline-table"
                | "table-cell"
                | "table-caption"
                | "flex"
                | "grid"
        )
    ) || matches!(st.get("overflow"), Some("hidden" | "auto" | "scroll"))
        || matches!(st.get("overflow-x"), Some("hidden" | "auto" | "scroll"))
        || matches!(st.get("overflow-y"), Some("hidden" | "auto" | "scroll"))
        || matches!(st.get("float"), Some("left" | "right"))
        || matches!(st.get("position"), Some("absolute" | "fixed"))
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
    // La racine établit le BFC initial : ses flottants sont
    // contenus dans sa hauteur (§9.4.1/§10.6.7).
    let mut floats = FloatCtx::default();
    layout_block(
        root,
        0.0,
        0.0,
        viewport_w,
        // ICB : la hauteur du viewport est définie —
        // `height: 100%` sur <html> y résout.
        (ctx.viewport_h > 0.0).then_some(ctx.viewport_h),
        ctx,
        ctx.base_font_size,
        viewport,
        &mut floats,
    );
    let deep = FloatCtx::bottom_of(&floats.left)
        .unwrap_or(0.0)
        .max(FloatCtx::bottom_of(&floats.right).unwrap_or(0.0));
    root.rect.height = root.rect.height.max(deep);
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
pub(crate) fn side_px(
    style: &ComputedStyle,
    prop: &str,
    cb_w: f32,
    em: f32,
    ctx: &BlockCtx,
    fallback: f32,
) -> f32 {
    css_px(style, prop, cb_w, em, ctx).unwrap_or(fallback)
}

/// `border-*-width` en px pour le **layout** : les mots-clés
/// `thin`/`medium`/`thick` valent 1/3/5 px — même table que
/// le raster (`display.rs`), sinon la bordure est peinte 3 px
/// mais comptée 0 dans les boîtes (décalage observé sur les
/// reftests CSS2, étape 48). La largeur ne compte QUE si le
/// `border-*-style` correspondant n'est pas `none` — sinon la
/// valeur initiale `medium` donnerait 3 px à chaque boîte.
pub(crate) fn border_px(
    style: &ComputedStyle,
    prop: &str,
    cb_w: f32,
    em: f32,
    ctx: &BlockCtx,
) -> f32 {
    let side = prop
        .strip_prefix("border-")
        .and_then(|s| s.strip_suffix("-width"))
        .unwrap_or_default();
    let style_prop = format!("border-{side}-style");
    if style.get(&style_prop).is_none_or(|s| s.trim() == "none") {
        return 0.0;
    }
    match style.get(prop).map(str::trim) {
        Some("thin") => 1.0,
        Some("thick") => 5.0,
        Some("medium") | None => 3.0,
        Some(_) => css_px(style, prop, cb_w, em, ctx).unwrap_or(0.0),
    }
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
/// positionné, ou viewport). `fl` = registre des flottants
/// du BFC auquel `bx` appartient (vierge si `bx` établit
/// le sien — fourni par l'appelant).
#[allow(clippy::too_many_lines, clippy::too_many_arguments)]
pub(crate) fn layout_block(
    bx: &mut LayoutBox,
    x: f32,
    y: f32,
    cb_w: f32,
    cb_h: Option<f32>,
    ctx: &BlockCtx,
    parent_fs: f32,
    abs_cb: Rect,
    fl: &mut FloatCtx,
) -> f32 {
    let st = bx.style.clone();
    let fs = font_size_px(&st, parent_fs, ctx);

    apply_layout_metadata(bx, &st);

    // §10.5 : `height` en % résout contre la hauteur du bloc
    // conteneur — uniquement si celle-ci est définie
    // (spécifiée, indépendante du contenu) ; sinon la valeur
    // vaut `auto` (repli contenu). Calculée tôt : elle fait
    // office de `cb_h` pour les descendants (base de leurs
    // propres %).
    let h_spec = if st.get("height").is_some_and(|v| v.contains('%')) {
        cb_h.and_then(|h| css_px(&st, "height", h, fs, ctx))
    } else {
        css_px(&st, "height", 0.0, fs, ctx)
    }
    .or(bx.intrinsic.map(|i| i.1));

    // `display: table` → dimensionnement dédié.
    if st.get("display") == Some("table") {
        return crate::tables::layout_table(bx, x, y, cb_w, ctx, parent_fs);
    }

    let ml = side_px(&st, "margin-left", cb_w, fs, ctx, 0.0);
    let mr = side_px(&st, "margin-right", cb_w, fs, ctx, 0.0);
    let bl = border_px(&st, "border-left-width", cb_w, fs, ctx);
    let br = border_px(&st, "border-right-width", cb_w, fs, ctx);
    let pl = side_px(&st, "padding-left", cb_w, fs, ctx, 0.0);
    let pr = side_px(&st, "padding-right", cb_w, fs, ctx, 0.0);
    let bt = border_px(&st, "border-top-width", cb_w, fs, ctx);
    let bb = border_px(&st, "border-bottom-width", cb_w, fs, ctx);
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
    // Mots-clés intrinsèques (R-3) : `min|max-content`,
    // `fit-content` — mesurés sur le contenu réel avant le
    // repli remplacé.
    let w_spec = css_px(&st, "width", cb_w, fs, ctx)
        .or_else(|| crate::intrinsic::intrinsic_size_px(&st, "width", bx, cb_w, fs, ctx))
        .or(bx.intrinsic.map(|i| i.0));
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
            fl,
            h_spec,
        ),
    };

    let content_h = cursor - (y + bt + pt);
    let mut used_h = h_spec.unwrap_or(content_h);
    // `min-height`/`max-height` : mêmes bases % que `height`
    // (hauteur du bloc conteneur définie, sinon ignorées).
    let vert_spec = |prop: &str| {
        if st.get(prop).is_some_and(|v| v.contains('%')) {
            cb_h.and_then(|h| css_px(&st, prop, h, fs, ctx))
        } else {
            css_px(&st, prop, 0.0, fs, ctx)
        }
    };
    if let Some(mn) = vert_spec("min-height") {
        used_h = used_h.max(mn);
    }
    if let Some(mx) = vert_spec("max-height") {
        used_h = used_h.min(mx);
    }
    let w = {
        let mut w = bl + pl + content_w + pr + br;
        let bound_px = |prop: &str| {
            css_px(&st, prop, cb_w, fs, ctx)
                .or_else(|| crate::intrinsic::intrinsic_size_px(&st, prop, bx, cb_w, fs, ctx))
        };
        if let Some(mn) = bound_px("min-width") {
            w = w.max(mn);
        }
        if let Some(mx) = bound_px("max-width") {
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

/// Itérations max de la descente d'un flottant — chaque tour
/// franchit le bas d'un flottant mordant ; la borne n'est
/// qu'une garde contre la pathologie (contenu hostile).
const MAX_FLOAT_STEPS: u32 = 64;

/// Pose le flottant `parent.children[i]` : position de flux
/// = `flow_y` (son « parent anonyme » virtuel, §9.5.1 règle
/// 4), abaissée par `clear` et par les collisions jusqu'à ce
/// que la boîte tienne dans la bande (règles 2, 3, 7 — la
/// bande les encode). Le curseur de flux n'est **pas** avancé.
#[allow(clippy::too_many_arguments)]
fn place_float(
    parent: &mut LayoutBox,
    i: usize,
    cst: &ComputedStyle,
    right_side: bool,
    flow_y: f32,
    content_x: f32,
    content_w: f32,
    ctx: &BlockCtx,
    fs: f32,
    pos_cb: Rect,
    fl: &mut FloatCtx,
    cb_h: Option<f32>,
) {
    let cfs = font_size_px(cst, fs, ctx);
    // Marges résolues sur la largeur du bloc conteneur (§8.3)
    // — le registre mémorise les marge-rects, pas les bordures.
    let ml = side_px(cst, "margin-left", content_w, cfs, ctx, 0.0);
    let mr = side_px(cst, "margin-right", content_w, cfs, ctx, 0.0);
    let mt = side_px(cst, "margin-top", content_w, cfs, ctx, 0.0);
    let mb = side_px(cst, "margin-bottom", content_w, cfs, ctx, 0.0);

    // §9.5.1 : le haut du flottant ne peut être plus haut
    // que le haut de tout flottant antérieur (règle 5) ni
    // violer `clear` (dégagement vers le bas, §9.5.2).
    let mut fy = fl
        .max_top()
        .map_or(flow_y, |t| flow_y.max(t))
        .max(fl.cleared(cst.get("clear"), f32::NEG_INFINITY));
    let mut placed = Rect::default();
    for _ in 0..MAX_FLOAT_STEPS {
        let (bx0, bw) = fl.band(fy, content_x, content_w);
        // BFC propre au flottant : registre vierge pour ses
        // propres descendants (§9.4.1).
        let mut inner = FloatCtx::default();
        let child = &mut parent.children[i];
        layout_block(child, bx0, fy, bw, cb_h, ctx, fs, pos_cb, &mut inner);
        let r = child.rect;
        if right_side {
            let corrected = bx0 + bw - r.width;
            child.rect.x = corrected;
            placed = Rect {
                x: corrected - ml,
                y: r.y - mt,
                width: r.width + ml + mr,
                height: r.height + mt + mb,
            };
        } else {
            placed = Rect {
                x: r.x - ml,
                y: r.y - mt,
                width: r.width + ml + mr,
                height: r.height + mt + mb,
            };
        }
        // Tient dans la bande (ou la bande est déjà pleine
        // largeur : dépassement toléré, règle 7 — « déjà aussi
        // loin que possible »).
        if placed.width <= bw + 0.01 || bw >= content_w - 0.01 {
            break;
        }
        // Descendre sous le prochain flottant mordant.
        match fl.next_band_change(fy) {
            Some(next) if next > fy + 0.01 => fy = next,
            _ => break,
        }
    }
    if right_side {
        fl.right.push(placed);
    } else {
        fl.left.push(placed);
    }
}

/// Empilement vertical des enfants de niveau bloc : marges
/// collapsées, flottants posés « aussi haut que possible »
/// depuis leur position de flux (sans l'avancer — §9.5.1),
/// `clear`, absolus laissés hors flux (rect statique
/// placeholder pour `resolve_absolutes`). Les enfants non-BFC
/// restent pleine largeur : ce sont leurs **lignes** qui se
/// rétrécissent, via le registre `fl` partagé ; les enfants
/// BFC se rangent dans la bande. Retourne le curseur final —
/// étendu au bas des flottants propres si `bx` est un BFC.
#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
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
    fl: &mut FloatCtx,
    cb_h: Option<f32>,
) -> f32 {
    let mut cursor = start_cursor;
    let own_bfc = establishes_bfc(&bx.style);
    // `text-align: -webkit-center` (UA : <center>, spec
    // « align descendants to the center ») centre aussi les
    // descendants **bloc** plus étroits que le conteneur —
    // comportement historique que les pages tabulaires (HN,
    // anciens sites) attendent. `center` standard ne touche
    // que les lignes inline.
    let webkit_center = bx.style.get("text-align") == Some("-webkit-center");

    let mut prev_margin: Option<f32> = None;
    let mut first = true;
    // Haut de la dernière ligne posée par du contenu en-ligne
    // — l'ancre verticale d'un flottant (§9.5.1 : son haut ne
    // peut dépasser le haut de la ligne où tombe sa position
    // source). `None` après un enfant bloc : la position de
    // flux est alors une ligne nouvelle (le curseur).
    let mut inline_anchor: Option<f32> = None;
    let count = bx.children.len();
    for i in 0..count {
        let (kind, cst) = {
            let c = &bx.children[i];
            (c.kind, c.style.clone())
        };

        // Bloc anonyme : boîte sans élément — les propriétés
        // de son style cloné (`float`, `position`, `clear`,
        // `display: flow-root`, marges…) appartiennent au
        // parent et ne s'y appliquent PAS. Pleine largeur,
        // flux pur : seules ses lignes se rétrécissent dans la
        // bande. Marges nulles — le voisin collapse au travers.
        if kind == BoxKind::AnonymousBlock {
            let gap = if first && adjoining_top {
                0.0
            } else {
                prev_margin.unwrap_or(0.0)
            };
            cursor += gap;
            let used = layout_anon_block(
                &mut bx.children[i],
                content_x,
                cursor,
                content_w,
                ctx,
                fs,
                pos_cb,
                fl,
                cb_h,
            );
            inline_anchor = bx.children[i].lines.last().map(|l| l.rect.y);
            cursor += used;
            prev_margin = Some(0.0);
            first = false;
            continue;
        }

        if kind.is_inline_level() {
            // Enfant en-ligne direct (hors enveloppe) : une
            // ligne de `line_h` — son haut est l'ancre.
            inline_anchor = Some(cursor);
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

        // `float` : posé dans la bande à sa position de flux
        // — n'avance ni curseur ni marges (hors flux ; les
        // marges in-flow collapse « à travers » lui, §8.3.1).
        // Ancre = haut de la ligne courante (§9.5.1 : le
        // flottant est « as high as possible » sans dépasser
        // le haut de la ligne où tombe sa position source) —
        // pas le bas du bloc anonyme qui précède.
        let cfloat = cst.get("float");
        if matches!(cfloat, Some("left" | "right")) {
            place_float(
                bx,
                i,
                &cst,
                cfloat == Some("right"),
                inline_anchor.unwrap_or(cursor),
                content_x,
                content_w,
                ctx,
                fs,
                pos_cb,
                fl,
                cb_h,
            );
            continue;
        }

        // `clear` (contenu in-flow) : le curseur descend sous
        // les flottants du côté déclaré (dégagement, §9.5.2).
        if matches!(cst.get("clear"), Some("left" | "right" | "both")) {
            cursor = fl.cleared(cst.get("clear"), cursor);
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

        // BFC : la boîte elle-même se range dans la bande ;
        // non-BFC : pleine largeur — ce sont leurs **lignes**
        // qui se rangent (registre partagé `fl`, ligne par ligne).
        let child_bfc = establishes_bfc(&cst);
        let used = if child_bfc {
            // BFC : registre vierge — les flottants internes ne
            // mordent pas l'extérieur (§9.4.1). Placement
            // complet dans `layout_bfc_child`.
            layout_bfc_child(
                &mut bx.children[i],
                cursor,
                content_x,
                content_w,
                ctx,
                fs,
                pos_cb,
                fl,
                cb_h,
            )
        } else {
            layout_block(
                &mut bx.children[i],
                content_x,
                cursor,
                content_w,
                cb_h,
                ctx,
                fs,
                pos_cb,
                fl,
            )
        };
        cursor += used;
        // `-webkit-center` : enfant bloc in-flow plus étroit
        // que le conteneur → centré horizontalement (comme
        // des marges `auto`). La translation s'applique à
        // tout le sous-arbre (rects posés en absolu).
        if webkit_center {
            let cw = bx.children[i].rect.width;
            if cw < content_w {
                crate::flex::translate_subtree(&mut bx.children[i], (content_w - cw) / 2.0, 0.0);
            }
        }
        // Enfant bloc in-flow : la prochaine ligne est
        // nouvelle — l'ancre des flottants repasse au curseur.
        inline_anchor = None;
        prev_margin = Some(cm.bottom);
        first = false;
    }
    // Containment : un BFC englobe le bas de ses flottants
    // propres (§10.6.7) — le registre `fl` est alors vierge
    // à son entrée et ne contient que les siens.
    if own_bfc {
        let deep = FloatCtx::bottom_of(&fl.left)
            .unwrap_or(f32::NEG_INFINITY)
            .max(FloatCtx::bottom_of(&fl.right).unwrap_or(f32::NEG_INFINITY));
        if deep.is_finite() {
            cursor = cursor.max(deep);
        }
    }
    cursor
}

/// Pose un enfant BFC : sa border box ne doit chevaucher
/// AUCUN flottant du registre `fl` sur toute sa hauteur
/// (§9.5) — la bande retenue est l'intersection des bandes
/// sur `[y, y+h]`. On itère — poser dans la bande, mesurer
/// `h`, ré-intersecter — jusqu'à bande stable ; si la boîte
/// n'y tient pas (largeur fixe > bande), elle descend sous
/// le premier flottant finissant dans son étendue. Borné :
/// chaque descente passe un bas de flottant, jamais deux
/// fois le même. Retourne l'avance verticale consommée
/// (descente éventuelle incluse).
#[allow(clippy::too_many_arguments)]
fn layout_bfc_child(
    child: &mut LayoutBox,
    cursor: f32,
    content_x: f32,
    content_w: f32,
    ctx: &BlockCtx,
    fs: f32,
    pos_cb: Rect,
    fl: &FloatCtx,
    cb_h: Option<f32>,
) -> f32 {
    let mut y = cursor;
    let mut est = 0.0;
    let mut used = 0.0;
    for _ in 0..=fl.left.len() + fl.right.len() + 2 {
        let (bx0, bw) = fl.band_over(y, est, content_x, content_w);
        let mut fresh = FloatCtx::default();
        let h = layout_block(child, bx0, y, bw, cb_h, ctx, fs, pos_cb, &mut fresh);
        let (bx1, bw1) = fl.band_over(y, h, content_x, content_w);
        used = h;
        if (bx1, bw1) == (bx0, bw) {
            // Bande stable — la boîte y tient-elle ?
            let fits =
                child.rect.x >= bx1 - 0.5 && child.rect.x + child.rect.width <= bx1 + bw1 + 0.5;
            if fits {
                break;
            }
            match fl.first_bottom_within(y, h) {
                Some(nb) if nb > y => {
                    y = nb;
                    est = h;
                }
                _ => break,
            }
        } else {
            // Bande resserrée/élargie : reposer dedans.
            est = h;
        }
    }
    // Le curseur rejoint le bas de la boîte — une descente
    // (`y > cursor`) compte dans l'avance.
    y + used - cursor
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
        // Boîte absolue = BFC propre : registre vierge. Le CB
        // absolu est défini (padding box de l'ancêtre posé) —
        // ses hauteurs % résolvent (§10.5).
        layout_block(
            child,
            cx,
            cy,
            cw,
            Some(target.height),
            ctx,
            fs,
            target,
            &mut FloatCtx::default(),
        );
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
/// (étape 20 — vraies line boxes dans `bx.lines`). `fl` est
/// le registre partagé : les lignes se rétrécissent dans la
/// bande de leur propre `y` (§9.5).
#[allow(clippy::too_many_arguments)]
pub(crate) fn layout_anon_block(
    bx: &mut LayoutBox,
    x: f32,
    y: f32,
    w: f32,
    ctx: &BlockCtx,
    parent_fs: f32,
    abs_cb: Rect,
    fl: &FloatCtx,
    cb_h: Option<f32>,
) -> f32 {
    bx.rect = Rect {
        x,
        y,
        width: w,
        height: 0.0,
    };
    crate::inline::layout_inline(bx, x, y, w, ctx, parent_fs, abs_cb, fl, cb_h)
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
