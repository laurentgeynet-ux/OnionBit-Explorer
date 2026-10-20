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
use crate::font::{self, FontSource};

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
    /// Cache de la pipeline fonte pour la passe (R-33b) :
    /// `font_data`/métriques/shaping ne sont facturés qu'une
    /// fois par requête, pas par run de texte.
    pub font_cache: std::cell::RefCell<font::FontCache>,
}

impl Default for BlockCtx<'_> {
    fn default() -> Self {
        Self {
            fonts: None,
            default_family: "sans-serif",
            base_font_size: 16.0,
            viewport_h: 600.0,
            font_cache: std::cell::RefCell::new(font::FontCache::new()),
        }
    }
}

/// Marges verticales usées d'une boîte (px).
#[derive(Debug, Clone, Copy)]
struct Margins {
    top: f32,
    bottom: f32,
}

/// Marge fusionnée courante (§8.3.1) — paire
/// `(positif max, négatif min)` dont la valeur repliée vaut
/// `pos + neg`. Toutes les marges adjacentes d'une chaîne
/// fusionnent en UNE — un `collapse` pairwise séquentiel
/// perdrait les négatifs internes (Acid2 `.empty` :
/// `[48, 75, 0, −72, 75, 60]` doit valoir 3, pas 60).
#[derive(Clone, Copy, Default)]
struct MarginRun {
    pos: f32,
    neg: f32,
}

impl MarginRun {
    fn add(&mut self, v: f32) {
        if v >= 0.0 {
            self.pos = self.pos.max(v);
        } else {
            self.neg = self.neg.min(v);
        }
    }

    fn fold(&mut self, other: Self) {
        self.pos = self.pos.max(other.pos);
        self.neg = self.neg.min(other.neg);
    }

    fn value(self) -> f32 {
        self.pos + self.neg
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
    // Les marges de la racine s'appliquent à sa border box
    // (§8.3.1 : jamais collapsées — le collapse-through est
    // bloqué par `is_root` plus bas). La marge haute décale
    // la boîte dans l'ICB ; les latérales sont résolues en
    // interne comme pour toute boîte bloc.
    let root_fs = font_size_px(&root.style, ctx.base_font_size, ctx);
    let root_mt = margins(&root.style, viewport_w, root_fs, ctx).top;
    explorer_common::deep(|| {
        layout_block_inner(
            root,
            0.0,
            root_mt,
            viewport_w,
            // ICB : la hauteur du viewport est définie —
            // `height: 100%` sur <html> y résout.
            (ctx.viewport_h > 0.0).then_some(ctx.viewport_h),
            ctx,
            ctx.base_font_size,
            viewport,
            &mut floats,
            true,
        )
    });
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

/// §8.3.1 : une boîte est « transparente aux marges » vers
/// le bas si sa hauteur est nulle, sans bordure ni padding
/// vertical, en flux normal, hors BFC — sa marge basse
/// fusionne alors avec celle de son dernier enfant in-flow.
fn margin_transparent(bx: &LayoutBox, cb_w: f32, fs: f32, ctx: &BlockCtx) -> bool {
    let st = &bx.style;
    bx.rect.height <= 0.0
        && !matches!(bx.position, Position::Absolute | Position::Fixed)
        && bx.float == Float::None
        && !establishes_bfc(st)
        && side_px(st, "padding-top", cb_w, fs, ctx, 0.0) <= 0.0
        && side_px(st, "padding-bottom", cb_w, fs, ctx, 0.0) <= 0.0
        && border_px(st, "border-top-width", cb_w, fs, ctx) <= 0.0
        && border_px(st, "border-bottom-width", cb_w, fs, ctx) <= 0.0
        && !matches!(st.get("clear"), Some("left" | "right" | "both"))
}

/// Candidat « transparent aux marges » avant layout — les
/// conditions purement stylistiques de [`margin_transparent`]
/// (pas de bordure/padding vertical, en flux, hors BFC, pas
/// de `clear`) plus une hauteur qui ne peut pas imposer
/// `> 0` (un `%` ne compte que si `cb_h` est défini — sinon
/// il retombe sur `auto`, repli contenu). La confirmation
/// se fait après layout (`h <= 0`).
fn transparent_candidate(
    bx: &LayoutBox,
    cb_w: f32,
    cb_h: Option<f32>,
    fs: f32,
    ctx: &BlockCtx,
) -> bool {
    let st = &bx.style;
    let h_spec = if st.get("height").is_some_and(|v| v.contains('%')) {
        cb_h.and_then(|h| css_px(st, "height", h, fs, ctx))
    } else {
        css_px(st, "height", 0.0, fs, ctx)
    };
    !matches!(bx.position, Position::Absolute | Position::Fixed)
        && bx.float == Float::None
        && !establishes_bfc(st)
        && side_px(st, "padding-top", cb_w, fs, ctx, 0.0) <= 0.0
        && side_px(st, "padding-bottom", cb_w, fs, ctx, 0.0) <= 0.0
        && border_px(st, "border-top-width", cb_w, fs, ctx) <= 0.0
        && border_px(st, "border-bottom-width", cb_w, fs, ctx) <= 0.0
        && !matches!(st.get("clear"), Some("left" | "right" | "both"))
        && h_spec.is_none_or(|h| h <= 0.0)
}

/// Marge basse échappée d'un enfant posé, en paire
/// `(pos, neg)` — identique à [`escaped_bottom_margin`] mais
/// conserve les deux bornes de la chaîne (§8.3.1). Un
/// dernier enfant transparent contribue sa chaîne complète.
/// La marge basse du dernier fils in-flow s'échappe-t-elle
/// sous la boîte ? (§8.3.1) — pas de padding/bordure bas,
/// pas de BFC, `height:auto` (une hauteur définie cloue la
/// boîte : la marge du fils reste dans son contenu ; un
/// `height:%` non résoluble retombe sur `auto`, repli
/// tolérant). Miroir bas de [`top_adjoining`].
fn bottom_adjoining(bx: &LayoutBox, cb_w: f32, fs: f32, ctx: &BlockCtx) -> bool {
    let st = &bx.style;
    let h_spec = if st.get("height").is_some_and(|v| v.contains('%')) {
        None
    } else {
        css_px(st, "height", 0.0, fs, ctx)
    };
    !matches!(bx.position, Position::Absolute | Position::Fixed)
        && bx.float == Float::None
        && !establishes_bfc(st)
        && side_px(st, "padding-bottom", cb_w, fs, ctx, 0.0) <= 0.0
        && border_px(st, "border-bottom-width", cb_w, fs, ctx) <= 0.0
        && !matches!(st.get("clear"), Some("left" | "right" | "both"))
        && h_spec.is_none_or(|h| h <= 0.0)
}

fn escaped_bottom_run(bx: &LayoutBox, cb_w: f32, parent_fs: f32, ctx: &BlockCtx) -> MarginRun {
    let st = &bx.style;
    let fs = font_size_px(st, parent_fs, ctx);
    let mut run = MarginRun::default();
    run.add(margins(st, cb_w, fs, ctx).bottom);
    // `height_raised` (min-height au-dessus du contenu) coupe
    // la chaîne du dernier fils : sa marge basse est avalée.
    if bottom_adjoining(bx, cb_w, fs, ctx) && !bx.height_raised {
        if let Some(c) = bx.children.iter().rev().find(|c| {
            !matches!(c.kind, BoxKind::Text | BoxKind::AnonymousBlock)
                && !matches!(c.position, Position::Absolute | Position::Fixed)
                && c.float == Float::None
        }) {
            if margin_transparent(c, cb_w, fs, ctx) {
                run.fold(margin_chain(c, cb_w, fs, ctx));
            } else {
                run.fold(escaped_bottom_run(c, cb_w, fs, ctx));
            }
        }
    }
    run
}

/// La marge haute d'une boîte peut-elle fusionner avec
/// celle de son premier fils in-flow ? (§8.3.1) — pas de
/// padding/bordure haut, pas de BFC propre. La hauteur ne
/// joue pas (contrairement au bas, qui exige `height:auto`).
fn top_adjoining(bx: &LayoutBox, cb_w: f32, fs: f32, ctx: &BlockCtx) -> bool {
    let st = &bx.style;
    !establishes_bfc(st)
        && side_px(st, "padding-top", cb_w, fs, ctx, 0.0) <= 0.0
        && border_px(st, "border-top-width", cb_w, fs, ctx) <= 0.0
}

/// Marge haute **effective** d'un enfant de flux : sa
/// `margin-top` fusionne, tant que rien ne la sépare, avec
/// la marge haute de son premier descendant in-flow —
/// récursif (§8.3.1). Une boîte `padding:0; border:0` sans
/// BFC laisse la marge de son premier fils remonter à son
/// propre bord supérieur : c'est ce total qui sert de marge
/// haute face aux voisins (réf float-root : `p` dont la
/// marge traverse `body` dans le `:root` flottant).
///
/// Un enfant de tête « transparent à la chaîne »
/// ([`chain_transparent`] — structurellement vide, ses bords
/// haut et bas coïncident) apporte sa chaîne COMPLÈTE
/// `[mt … mb]` à la fusion, pas seulement `mt` : c'est ce
/// qui remonte la marge basse d'un dernier fils transparent
/// jusqu'au bord haut du parent vide
/// (`margin-collapse-min-height-001.xht`). Le parcours se
/// poursuit alors sur les frères : toute la suite de boîtes
/// vides en tête fusionne dans la même marge.
fn collapsed_top_margin(
    bx: &LayoutBox,
    cb_w: f32,
    cb_h: Option<f32>,
    parent_fs: f32,
    ctx: &BlockCtx,
) -> MarginRun {
    let st = &bx.style;
    let fs = font_size_px(st, parent_fs, ctx);
    let mut run = MarginRun::default();
    run.add(margins(st, cb_w, fs, ctx).top);
    if !top_adjoining(bx, cb_w, fs, ctx) {
        return run;
    }
    for c in &bx.children {
        if matches!(c.kind, BoxKind::Text | BoxKind::AnonymousBlock)
            || out_of_flow_st(&c.style)
            || floated_st(&c.style)
        {
            continue;
        }
        // Un contenu en-ligne (line box) arrête la fusion ;
        // `clear` empêche la marge de rejoindre le haut.
        if c.kind.is_inline_level()
            || matches!(c.style.get("clear"), Some("left" | "right" | "both"))
        {
            break;
        }
        if chain_transparent(c, cb_w, cb_h, fs, ctx) {
            run.fold(candidate_chain(c, cb_w, cb_h, fs, ctx));
            continue;
        }
        run.fold(collapsed_top_margin(c, cb_w, cb_h, fs, ctx));
        break;
    }
    run
}

/// Hors-flux lu au style (`position` absolue/fixe) — sûr
/// avant layout, où `bx.position` n'est pas encore posé.
fn out_of_flow_st(st: &ComputedStyle) -> bool {
    matches!(st.get("position"), Some("absolute" | "fixed"))
}

/// Flottant lu au style — même motif que [`out_of_flow_st`].
fn floated_st(st: &ComputedStyle) -> bool {
    matches!(st.get("float"), Some("left" | "right"))
}

/// « Transparent à la chaîne » **avant layout** : la boîte
/// est candidate à la transparence ([`transparent_candidate`])
/// ET aucun de ses enfants in-flow ne peut contribuer de la
/// hauteur — récursif et purement stylistique, donc sûr sur
/// un sous-arbre pas encore posé (contrairement à
/// [`margin_transparent`], qui lit `rect.height` posé).
/// Flottants et hors-flux ne comptent pas (§10.6.3 : la
/// hauteur auto n'en tient pas compte).
fn chain_transparent(
    bx: &LayoutBox,
    cb_w: f32,
    cb_h: Option<f32>,
    fs: f32,
    ctx: &BlockCtx,
) -> bool {
    transparent_candidate(bx, cb_w, cb_h, fs, ctx)
        && bx.children.iter().all(|c| {
            // Contenu en-ligne ou remplacé → hauteur réelle.
            if matches!(c.kind, BoxKind::Text | BoxKind::AnonymousBlock) || c.kind.is_inline_level()
            {
                return false;
            }
            // Hors-flux et flottants ne contribuent pas — lu
            // au style : `bx.position`/`bx.float` ne sont posés
            // qu'au layout (stale avant).
            if out_of_flow_st(&c.style) || floated_st(&c.style) {
                return true;
            }
            let cfs = font_size_px(&c.style, fs, ctx);
            chain_transparent(c, cb_w, cb_h, cfs, ctx)
        })
}

/// Version pré-layout de [`margin_chain`] : chaîne complète
/// d'une boîte structurellement vide ([`chain_transparent`]),
/// pliable dans la marge haute d'un parent avant que le
/// sous-arbre ne soit posé.
fn candidate_chain(
    bx: &LayoutBox,
    cb_w: f32,
    cb_h: Option<f32>,
    parent_fs: f32,
    ctx: &BlockCtx,
) -> MarginRun {
    let st = &bx.style;
    let fs = font_size_px(st, parent_fs, ctx);
    let m = margins(st, cb_w, fs, ctx);
    let mut run = MarginRun::default();
    run.add(m.top);
    for c in &bx.children {
        if matches!(c.kind, BoxKind::Text | BoxKind::AnonymousBlock)
            || out_of_flow_st(&c.style)
            || floated_st(&c.style)
        {
            continue;
        }
        let cfs = font_size_px(&c.style, fs, ctx);
        if chain_transparent(c, cb_w, cb_h, cfs, ctx) {
            run.fold(candidate_chain(c, cb_w, cb_h, cfs, ctx));
        } else {
            let cm = margins(&c.style, cb_w, cfs, ctx);
            run.add(cm.top);
            run.add(cm.bottom);
        }
    }
    run.add(m.bottom);
    run
}

/// Chaîne de marges complète traversant une boîte
/// transparente aux marges (§8.3.1, « margins collapse
/// through ») : ses marges haute et basse fusionnent avec
/// les chaînes de ses enfants in-flow dans l'ordre — toute
/// la chaîne se replie en une marge.
fn margin_chain(bx: &LayoutBox, cb_w: f32, parent_fs: f32, ctx: &BlockCtx) -> MarginRun {
    let st = &bx.style;
    let fs = font_size_px(st, parent_fs, ctx);
    let m = margins(st, cb_w, fs, ctx);
    let mut run = MarginRun::default();
    run.add(m.top);
    for c in &bx.children {
        if matches!(c.kind, BoxKind::Text | BoxKind::AnonymousBlock)
            || c.out_of_flow()
            || c.float != Float::None
        {
            continue;
        }
        if margin_transparent(c, cb_w, fs, ctx) {
            run.fold(margin_chain(c, cb_w, fs, ctx));
        } else {
            let cm = margins(&c.style, cb_w, font_size_px(&c.style, fs, ctx), ctx);
            run.add(cm.top);
            run.add(cm.bottom);
        }
    }
    run.add(m.bottom);
    run
}

/// Dimensionne `bx` (boîte de niveau bloc) dans le bloc
/// conteneur de largeur `cb_w`, bord haut à `y`. Pose
/// `bx.rect` et descend récursivement. `abs_cb` = bloc
/// conteneur du positionnement absolu (plus proche ancêtre
/// positionné, ou viewport). `fl` = registre des flottants
/// du BFC auquel `bx` appartient (vierge si `bx` établit
/// le sien — fourni par l'appelant).
///
/// Chaque appel passe par [`explorer_common::deep`] (R-24) :
/// la récursion suit la profondeur DOM — un document hostile
/// bascule sur des segments tas au lieu de déborder la pile.
#[allow(clippy::too_many_arguments)]
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
    explorer_common::deep(|| {
        layout_block_inner(bx, x, y, cb_w, cb_h, ctx, parent_fs, abs_cb, fl, false)
    })
}

#[allow(clippy::too_many_lines, clippy::too_many_arguments)]
fn layout_block_inner(
    bx: &mut LayoutBox,
    x: f32,
    y: f32,
    cb_w: f32,
    cb_h: Option<f32>,
    ctx: &BlockCtx,
    parent_fs: f32,
    abs_cb: Rect,
    fl: &mut FloatCtx,
    is_root: bool,
) -> f32 {
    let st = bx.style.clone();
    let fs = font_size_px(&st, parent_fs, ctx);

    // R-12 : contexte de pose mémorisé pour `relayout_subtree`.
    bx.slot = crate::boxtree::BoxSlot {
        x,
        y,
        cb_w,
        cb_h,
        parent_fs,
        abs_cb,
    };

    apply_layout_metadata(bx, &st);

    // §10.5 : `height` en % résout contre la hauteur du bloc
    // conteneur — uniquement si celle-ci est définie
    // (spécifiée, indépendante du contenu) ; sinon la valeur
    // vaut `auto` (repli contenu). Calculée tôt : elle fait
    // office de `cb_h` pour les descendants (base de leurs
    // propres %). `h_css` garde la part purement CSS pour la
    // dérivation de ratio §10.6.2 plus bas.
    let h_css = if st.get("height").is_some_and(|v| v.contains('%')) {
        cb_h.and_then(|h| css_px(&st, "height", h, fs, ctx))
    } else {
        css_px(&st, "height", 0.0, fs, ctx)
    };
    let h_spec = h_css.or(bx.intrinsic.map(|i| i.1));

    // `display: table` → dimensionnement dédié. La marge
    // gauche s'applique à la boîte table (§10.3 : `auto` →
    // 0 pour le MVP) — `cb_w` est amputé d'autant.
    if st.get("display") == Some("table") {
        let ml = side_px(&st, "margin-left", cb_w, fs, ctx, 0.0);
        let mr = side_px(&st, "margin-right", cb_w, fs, ctx, 0.0);
        return crate::tables::layout_table(
            bx,
            x + ml,
            y,
            (cb_w - ml - mr).max(0.0),
            ctx,
            parent_fs,
            h_spec,
        );
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
        .or(bx.intrinsic.map(|i| {
            // §10.6.2 : `width:auto` + `height` spécifiée → la
            // largeur dérive du ratio intrinsèque (h × iw/ih).
            if i.1 > 0.0 {
                h_css.map_or(i.0, |h| h * i.0 / i.1)
            } else {
                i.0
            }
        }))
        .or_else(|| {
            // §10.3.5 : flottant à `width:auto` →
            // shrink-to-fit = clamp(dispo, min-content,
            // max-content). La dispo = bande du flottant
            // moins marges et châssis propres.
            if bx.float == Float::None {
                return None;
            }
            let avail = (cb_w - ml - mr - bl - br - pl - pr).max(0.0);
            Some(
                avail
                    .min(crate::intrinsic::max_content_px(bx, fs, ctx))
                    .max(crate::intrinsic::min_content_px(bx, fs, ctx)),
            )
        });
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
    // §10.4 : `min|max-width` bornent la boîte de CONTENU
    // (`box-sizing: content-box`) — avant la pose des
    // enfants : une valeur « auto » shrink-to-fit trop
    // étroite (flottant) doit être élargie ICI, pas
    // seulement sur le rect final — sinon les descendants
    // se rangent dans la largeur non bornée.
    let content_w = {
        let bound_px = |prop: &str| {
            css_px(&st, prop, cb_w, fs, ctx)
                .or_else(|| crate::intrinsic::intrinsic_size_px(&st, prop, bx, cb_w, fs, ctx))
        };
        let mut w = content_w;
        // §10.4 : `max-width` avant `min-width` — le min
        // gagne les conflits (miroir de `min|max-height`).
        if let Some(mx) = bound_px("max-width") {
            w = w.min(mx);
        }
        if let Some(mn) = bound_px("min-width") {
            w = w.max(mn);
        }
        w
    };

    // §10.6.2 : remplacé à `height:auto` — la hauteur dérive
    // de la largeur utilisée via le ratio intrinsèque, APRES
    // le bridage `min|max-width` (un `img{max-width:100%}`
    // bridé se rétrécit proportionnellement au lieu de
    // s'étirer — terrain : icônes sociales rust-lang.org
    // rendues en bandes blanches ~70×511).
    let h_spec = match (h_css, bx.intrinsic) {
        (None, Some((iw, ih))) if iw > 0.0 => Some(content_w * ih / iw),
        _ => h_spec,
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
        Some("grid" | "inline-grid") => crate::grid::flow_grid_children(
            bx, content_x, cursor, content_w, ctx, fs, pos_cb, h_spec,
        ),
        // Boîte en-ligne posée comme conteneur (`inline-block`,
        // item flex/grid blockifié, repli `<object>`) : son
        // contenu coule dans son PROPRE contexte en-ligne
        // (§9.2.4) — sans cette route, les enfants `Text`
        // d'un inline-block gardaient un rect 0×0 et leur
        // texte n'était jamais peint.
        _ if bx.kind == BoxKind::Inline && bx.children.iter().all(|c| c.kind.is_inline_level()) => {
            let used = crate::inline::layout_inline(
                bx, content_x, cursor, content_w, ctx, fs, pos_cb, fl, cb_h,
            );
            cursor + used
        }
        _ => flow_children(
            bx,
            content_x,
            cursor,
            content_w,
            // §8.3.1 : la marge haute du premier fils ne
            // « traverse » le parent que s'il ne la sépare
            // de rien — pas de padding/bordure haut ET pas
            // de BFC propre (un inline-block garde la marge
            // de son premier fils à l'intérieur). Jamais pour
            // la racine : ses marges ne collapsent pas, la
            // chaîne du premier fils reste à l'intérieur.
            !is_root && pt + bt <= 0.0 && !establishes_bfc(&st),
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
    // §10.7 : `max-height` s'applique AVANT `min-height` —
    // en cas de conflit (`min > max`), le min gagne (Acid2
    // `.picture p` : 8px → cap 2mm → plancher 1em → 12).
    if let Some(mx) = vert_spec("max-height") {
        used_h = used_h.min(mx);
    }
    if let Some(mn) = vert_spec("min-height") {
        used_h = used_h.max(mn);
    }
    // §8.3.1 : hauteur utilisée au-dessus du contenu → la
    // marge basse du dernier fils in-flow est avalée — ni
    // comptée dans le contenu, ni échappée au parent
    // (WPT `margin-collapse-min-height-001` ; si `min-height`
    // ne dépasse pas le contenu, la marge traverse
    // normalement — `-003`, `margin-collapse-037`).
    bx.height_raised = used_h > content_h;
    // La boîte de contenu est déjà bornée par `min|max-width`
    // en amont de la pose des enfants — le rect suit.
    bx.rect.width = bl + pl + content_w + pr + br;
    bx.rect.height = bt + pt + used_h + pb + bb;

    // `position: relative` : décalage après le flux
    // (top pousse vers le bas, bottom vers le haut) — tout
    // le sous-arbre suit la boîte (enfants, lignes,
    // fragments), pas seulement son rect.
    let (off_x, off_y) = rel_offsets(bx, cb_w, fs, ctx);
    if off_x != 0.0 || off_y != 0.0 {
        crate::flex::translate_subtree(bx, off_x, off_y);
    }
    bx.rect.height
}

/// Décalage `position: relative` d'une boîte posée : zéro
/// pour toute autre valeur de `position`. Lit le style
/// calculé (le champ `position` peut ne pas être renseigné
/// selon le point d'appel).
pub(crate) fn rel_offsets(bx: &LayoutBox, cb_w: f32, fs: f32, ctx: &BlockCtx) -> (f32, f32) {
    if bx.position != Position::Relative {
        return (0.0, 0.0);
    }
    (
        css_px(&bx.style, "left", cb_w, fs, ctx).unwrap_or(0.0)
            - css_px(&bx.style, "right", cb_w, fs, ctx).unwrap_or(0.0),
        css_px(&bx.style, "top", cb_w, fs, ctx).unwrap_or(0.0)
            - css_px(&bx.style, "bottom", cb_w, fs, ctx).unwrap_or(0.0),
    )
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

/// Pose le flottant `parent.children[i]` : `flow_y` = bord
/// haut de la **margin box** hypothétique en flux (§9.5.1
/// règle 4), abaissé par `clear` et par les collisions
/// jusqu'à ce que la boîte tienne dans la bande (règles 2,
/// 3, 7 — la bande les encode). La border box est posée à
/// `flow_y + margin-top` : une marge négative la remonte.
/// Le curseur de flux n'est **pas** avancé.
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
    // violer `clear` (dégagement vers le bas, §9.5.2). Ces
    // contraintes s'appliquent au bord de la **margin box** —
    // une marge négative remonte la border box au-dessus de
    // son point de flux (Acid2 `.nose` : `-2em` le glisse
    // sous les yeux).
    let mut my = fl
        .max_top()
        .map_or(flow_y, |t| flow_y.max(t))
        .max(fl.cleared(cst.get("clear"), f32::NEG_INFINITY));
    let mut placed = Rect::default();
    for _ in 0..MAX_FLOAT_STEPS {
        let (bx0, bw) = fl.band(my, content_x, content_w);
        // BFC propre au flottant : registre vierge pour ses
        // propres descendants (§9.4.1).
        let mut inner = FloatCtx::default();
        let child = &mut parent.children[i];
        layout_block(child, bx0, my + mt, bw, cb_h, ctx, fs, pos_cb, &mut inner);
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
        // §9.5.1 règles 3/7 : (a) la margin box ne peut
        // recouvrir un flottant du côté **adverse** dans sa
        // bande verticale — `fl.band` tronque au bloc
        // conteneur et ne voit pas un flottant adverse
        // partiellement dehors ; (b) pas de dépassement du
        // bord du bloc conteneur dans le sens du flottant
        // quand un flottant du **même côté** partage la
        // bande (rule7-outside : déborder n'est toléré que
        // « déjà aussi loin que possible » — un voisin de
        // même côté annule l'exception et force la descente).
        {
            let overlapped = |o: &Rect| {
                placed.y < o.y + o.height - 0.01
                    && placed.y + placed.height > o.y + 0.01
                    && placed.x < o.x + o.width - 0.01
                    && placed.x + placed.width > o.x + 0.01
            };
            let foes = if right_side { &fl.left } else { &fl.right };
            let kin = if right_side { &fl.right } else { &fl.left };
            let mut drop = foes
                .iter()
                .filter(|o| overlapped(o))
                .map(|o| o.y + o.height)
                .fold(f32::NEG_INFINITY, f32::max);
            let overflows = if right_side {
                placed.x < content_x - 0.01
            } else {
                placed.x + placed.width > content_x + content_w + 0.01
            };
            if overflows {
                // « un autre flottant du même côté **à
                // gauche/droite** de lui » : même bande
                // verticale, du côté du flottement (le
                // recouvrement horizontal n'est pas exigé).
                drop = drop.max(
                    kin.iter()
                        .filter(|o| {
                            o.y + o.height > placed.y + 0.01
                                && o.y < placed.y + placed.height - 0.01
                                && if right_side {
                                    o.x >= placed.x + placed.width - 0.01
                                } else {
                                    o.x + o.width <= placed.x + 0.01
                                }
                        })
                        .map(|o| o.y + o.height)
                        .fold(f32::NEG_INFINITY, f32::max),
                );
            }
            if drop.is_finite() && drop > my + 0.01 {
                my = drop;
                continue;
            }
        }
        // Tient dans la bande (ou la bande est déjà pleine
        // largeur : dépassement toléré, règle 7 — « déjà aussi
        // loin que possible »).
        if placed.width <= bw + 0.01 || bw >= content_w - 0.01 {
            break;
        }
        // Descendre sous le prochain flottant mordant.
        match fl.next_band_change(my) {
            Some(next) if next > my + 0.01 => my = next,
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
    // Border-bottom du dernier enfant en flux posé — peut
    // différer de `cursor` après un enfant « transparent »
    // (h0) : sa marge haute a été consommée pour placer sa
    // border box mais ne consomme pas le flux (§8.3.1). Sert
    // d'ancre à la position statique des absolus (§10.6.4 :
    // leur place hypothétique se mesure depuis le bas réel
    // du frère précédent, pas depuis le curseur global).
    let mut flow_bottom = start_cursor;
    let own_bfc = establishes_bfc(&bx.style);
    // `text-align: -webkit-center` (UA : <center>, spec
    // « align descendants to the center ») centre aussi les
    // descendants **bloc** plus étroits que le conteneur —
    // comportement historique que les pages tabulaires (HN,
    // anciens sites) attendent. `center` standard ne touche
    // que les lignes inline.
    let webkit_center = bx.style.get("text-align") == Some("-webkit-center");

    let mut prev_margin: Option<MarginRun> = None;
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
                prev_margin.map_or(0.0, MarginRun::value)
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
            flow_bottom = cursor;
            prev_margin = Some(MarginRun::default());
            first = false;
            continue;
        }

        if kind.is_inline_level() {
            // Enfant en-ligne direct (hors enveloppe) : une
            // ligne de `line_h` — son haut est l'ancre. La
            // marge en attente est un simple écart (les lignes
            // ne collapse pas) et la chaîne est rompue.
            let gap = if first && adjoining_top {
                0.0
            } else {
                prev_margin.map_or(0.0, MarginRun::value)
            };
            inline_anchor = Some(cursor + gap);
            cursor += gap + line_h;
            flow_bottom = cursor;
            prev_margin = Some(MarginRun::default());
            first = false;
            continue;
        }
        let cfs = font_size_px(&cst, fs, ctx);

        // `position: absolute|fixed` : hors flux — rect
        // statique placeholder, résolu en second passage par
        // `resolve_absolutes` (layout_tree). La position
        // statique est celle qu'aurait la border box en flux
        // (§10.6.4) : la marge en attente du frère précédent
        // se replie avec sa chaîne de marges hautes —
        // premier fils inclus : son échappement du parent
        // est *virtuel* (la boîte posée ne bouge plus), le
        // gap vaut alors l'excédent du repli combiné
        // parent+enfant sur la marge propre du parent.
        if matches!(cst.get("position"), Some("absolute" | "fixed")) {
            // Position statique = border-top de la boîte
            // hypothétique en flux (§10.6.4) : le gap replie
            // les marges précédentes ET la chaîne propre de
            // l'absolu — mais PAS d'échappement « premier
            // fils » : sa marge haute ne traverse pas le
            // parent (les marges d'un absolu ne collapsent
            // jamais — `.d` de margin-collapse-114).
            let mut run = prev_margin.unwrap_or_default();
            run.fold(collapsed_top_margin(
                &bx.children[i],
                content_w,
                cb_h,
                fs,
                ctx,
            ));
            let gap = run.value();
            bx.children[i].rect = Rect {
                x: content_x,
                y: flow_bottom + gap,
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
        // pas le bas du bloc anonyme qui précède. Le bord de
        // sa margin box vaut le border-top qu'aurait la boîte
        // en flux normal (chaîne de marges repliée dessus,
        // sans la consommer) moins sa propre marge haute —
        // une marge négative remonte alors la boîte (Acid2
        // `.nose` : `-2em` sous les yeux).
        let cfloat = cst.get("float");
        if matches!(cfloat, Some("left" | "right")) {
            // Les marges d'un flottant ne collapsent jamais
            // (§8.3.1) : le haut de sa margin box s'empile
            // après la marge basse pendante — `prev` et `mt`
            // S'ADDITIONNENT, pas de repli. `cm.top` reste
            // dans la margin box (appliquée par `place_float`).
            let hypo = inline_anchor.unwrap_or(cursor) + prev_margin.unwrap_or_default().value();
            place_float(
                bx,
                i,
                &cst,
                cfloat == Some("right"),
                hypo,
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

        // Élément « transparent aux marges » (§8.3.1) :
        // hauteur nulle et rien qui sépare — toute sa chaîne
        // `[mt … enfants … mb]` rejoint la marge courante au
        // lieu d'être consommée en deux fois. Posé d'emblée
        // à `cursor + gap` : si la boîte se révèle non vide,
        // c'est sa position définitive — ses lignes se sont
        // déjà cassées dans la bande de CETTE ordonnée (un
        // flottant qui commence/finit entre `cursor` et
        // `cursor + gap` change le retrait ligne à ligne ;
        // `floats-placement-005`). Si `h <= 0`, sa place est
        // sans objet : la chaîne entière s'échappe.
        if transparent_candidate(&bx.children[i], content_w, cb_h, cfs, ctx) {
            let mut run = prev_margin.unwrap_or_default();
            run.fold(collapsed_top_margin(
                &bx.children[i],
                content_w,
                cb_h,
                fs,
                ctx,
            ));
            let gap = if first && adjoining_top {
                0.0
            } else {
                run.value()
            };
            let used = layout_block(
                &mut bx.children[i],
                content_x,
                cursor + gap,
                content_w,
                cb_h,
                ctx,
                fs,
                pos_cb,
                fl,
            );
            if bx.children[i].rect.height <= 0.0 {
                // Collapse-through (§8.3.1) : la marge haute
                // a placé sa border-top sans consommer le
                // flux — `flow_bottom` la retient pour la
                // position statique d'un absolu suivant,
                // `prev_margin` garde la chaîne complète
                // {mt,mb} pour le frère en flux suivant.
                flow_bottom = cursor + gap;
                let mut run = prev_margin.unwrap_or_default();
                run.fold(margin_chain(&bx.children[i], content_w, fs, ctx));
                prev_margin = Some(run);
                inline_anchor = None;
                continue;
            }
            if webkit_center {
                let cw = bx.children[i].rect.width;
                if cw < content_w {
                    crate::flex::translate_subtree(
                        &mut bx.children[i],
                        (content_w - cw) / 2.0,
                        0.0,
                    );
                }
            }
            cursor += gap + used;
            flow_bottom = cursor;
            prev_margin = Some(escaped_bottom_run(&bx.children[i], content_w, fs, ctx));
            first = false;
            inline_anchor = None;
            continue;
        }

        // Marge courante + marge haute de l'enfant : la
        // chaîne entière se replie en une valeur (max des
        // positifs + min des négatifs — §8.3.1). La marge
        // haute effective inclut celle qui s'échappe de ses
        // premiers descendants adjacents.
        let mut run = prev_margin.unwrap_or_default();
        run.fold(collapsed_top_margin(
            &bx.children[i],
            content_w,
            cb_h,
            fs,
            ctx,
        ));
        // Premier enfant adjoining : sa marge « traverse » le
        // parent (§8.3.1 — pas d'espace interne au-dessus ;
        // c'est le parent qui ajoute la sienne propre, côté
        // appelant, via `margins()`).
        let gap = if first && adjoining_top {
            0.0
        } else {
            run.value()
        };

        // `clear` (§9.5.1) : le haut de la border box tombe
        // sous les flottants du côté déclaré — l'écart créé
        // joue le rôle de dégagement (les marges au-dessus ne
        // s'ajoutent pas une seconde fois).
        let mut top = cursor + gap;
        if matches!(cst.get("clear"), Some("left" | "right" | "both")) {
            top = fl.cleared(cst.get("clear"), top);
        }
        cursor = top;

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
        flow_bottom = cursor;
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
        prev_margin = Some(escaped_bottom_run(&bx.children[i], content_w, fs, ctx));
        first = false;
    }
    // §8.3.1 : la chaîne de marges basse du dernier fils en
    // flux ne s'échappe par-dessous que si rien ne la sépare
    // de la marge du conteneur (pas de bordure ni padding bas,
    // pas de BFC, hauteur auto — sinon elle reste DANS la
    // hauteur de contenu : la margin box du dernier fils est
    // mesurée (ref `border:1px` des réftests flex-direction).
    // La racine a un `cb_h` défini (viewport) — ses marges ne
    // collapsent de toute façon jamais (§8.3.1).
    let adjoining_bottom = !own_bfc && cb_h.is_none() && bottom_adjoining(bx, content_w, fs, ctx);
    if !adjoining_bottom {
        if let Some(run) = prev_margin {
            cursor = (cursor + run.value()).max(start_cursor);
        }
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
/// Géométrie résolue d'un enfant `absolute`/`fixed` : point
/// d'ancrage `(cx, cy)` (static si le côté est `auto`),
/// largeur de contrainte `cw` et décalages bruts.
struct AbsGeo {
    cx: f32,
    cy: f32,
    cw: f32,
    l: Option<f32>,
    r: Option<f32>,
    t: Option<f32>,
    b: Option<f32>,
    ml: f32,
    mr: f32,
    mb: f32,
}

/// §10.3.7 : `width` non-`auto` → contrainte `l`+`r` ;
/// `width:auto` avec un seul ancrage (ou aucun) →
/// **shrink-to-fit** (`clamp(dispo, min-content,
/// max-content)`) — un conteneur absolu ne remplit jamais
/// tout le CB si son contenu est étroit (Acid2 `.eyes`).
/// Les marges horizontales amputent la largeur disponible.
fn abs_geometry(
    child: &LayoutBox,
    cst: &ComputedStyle,
    target: Rect,
    cfs: f32,
    ctx: &BlockCtx,
) -> AbsGeo {
    let l = css_px(cst, "left", target.width, cfs, ctx);
    let r = css_px(cst, "right", target.width, cfs, ctx);
    let t = css_px(cst, "top", target.height, cfs, ctx);
    let b = css_px(cst, "bottom", target.height, cfs, ctx);
    let ml = side_px(cst, "margin-left", target.width, cfs, ctx, 0.0);
    let mr = side_px(cst, "margin-right", target.width, cfs, ctx, 0.0);
    let mt = side_px(cst, "margin-top", target.width, cfs, ctx, 0.0);
    let mb = side_px(cst, "margin-bottom", target.width, cfs, ctx, 0.0);
    // Shrink-to-fit donne la largeur de CONTENU ; `cb_w`
    // passé à `layout_block` est la border box → ajouter
    // padding+bordures (Acid2 `blockquote` : contenu 48 +
    // 2×24 de bordures = 96 de border box, pas 48).
    let pl = side_px(cst, "padding-left", target.width, cfs, ctx, 0.0);
    let pr = side_px(cst, "padding-right", target.width, cfs, ctx, 0.0);
    let bl = border_px(cst, "border-left-width", target.width, cfs, ctx);
    let br = border_px(cst, "border-right-width", target.width, cfs, ctx);
    let extras = pl + pr + bl + br;
    let w_css = css_px(cst, "width", target.width, cfs, ctx);
    let shrink = |avail: f32| -> f32 {
        let min = crate::intrinsic::min_content_px(child, cfs, ctx);
        let pref = crate::intrinsic::max_content_px(child, cfs, ctx);
        avail.min(pref).max(min).max(0.0) + extras
    };
    let cw = match (l, r) {
        // `left`+`right`+`width:auto` : border box contrainte
        // (§10.3.7). `width` non-auto → sur-contraint, `right`
        // absorbé en LTR : la boîte garde sa largeur à gauche.
        (Some(lv), Some(rv)) => w_css.map_or_else(
            || (target.width - lv - rv - ml - mr).max(0.0),
            |w| w + extras,
        ),
        (Some(lv), None) => w_css.map_or_else(
            || shrink((target.width - lv - ml - mr - extras).max(0.0)),
            |w| w + extras,
        ),
        (None, Some(rv)) => w_css.map_or_else(
            || shrink((target.width - rv - ml - mr - extras).max(0.0)),
            |w| w + extras,
        ),
        (None, None) => w_css.map_or_else(
            || shrink((target.width - ml - mr - extras).max(0.0)),
            |w| w + extras,
        ),
    };
    AbsGeo {
        // Horizontalement, `cx` = bord de la margin box —
        // `layout_block` applique `ml` en interne sur la
        // border box. Verticalement rien n'ajoute `mt` :
        // `top` ancre le bord de marge (§10.6.4) → border
        // top = `top + mt` (Acid2 `blockquote` à `P+36`).
        cx: l.map_or(child.rect.x, |v| target.x + v),
        // `top` ancre la margin box → border top = `top+mt`.
        // `top:auto` → position statique : `rect.y` EST déjà
        // la border box hypothétique (le repli ci-dessus a
        // intégré la marge propre dans le gap).
        cy: t.map_or(child.rect.y, |v| target.y + v + mt),
        cw,
        l,
        r,
        t,
        b,
        ml,
        mr,
        mb,
    }
}

pub(crate) fn resolve_absolutes(bx: &mut LayoutBox, cb: Rect, ctx: &BlockCtx, fs: f32) {
    // Fonte PROPRE de la boîte (R-17 : la cascade stocke la
    // valeur calculée — `font-size:1em` est déjà px absolu)
    // — les `em` des bordures/décalages se résolvent contre
    // elle, pas contre la base 16 du premier appel.
    let own_fs = font_size_px(&bx.style, fs, ctx);
    // Bloc conteneur des enfants absolus de cette boîte :
    // **padding box** de l'ancêtre positionné (§10.1) — la
    // bordure n'en fait pas partie (Acid2 `.picture` a une
    // bordure d'1em : ancrer à la border box décale tout de
    // 12 px). Style lu directement : une boîte inline
    // `position:relative` n'a jamais ses métadonnées posées
    // (`apply_layout_metadata` est réservé aux blocs) — son
    // champ `position` resterait `Static` (R-31).
    // Bloc conteneur des enfants absolus de cette boîte :
    // **padding box** de l'ancêtre positionné (§10.1) — la
    // bordure n'en fait pas partie (Acid2 `.picture` a une
    // bordure d'1em : ancrer à la border box décale tout de
    // 12 px). Champ `position` OU style non-`static` : une
    // boîte inline `position:relative` n'a jamais ses
    // métadonnées posées (`apply_layout_metadata` est réservé
    // aux blocs) — son champ resterait `Static` (R-31). Les
    // boîtes anonymes/texte clonent le style du parent mais
    // ne sont JAMAIS des blocs conteneurs (R-22).
    let own_cb = if (bx.position == Position::Static
        && bx.style.get("position").is_none_or(|p| p == "static"))
        || matches!(bx.kind, BoxKind::Text | BoxKind::AnonymousBlock)
    {
        cb
    } else {
        let bl = border_px(&bx.style, "border-left-width", cb.width, own_fs, ctx);
        let bt = border_px(&bx.style, "border-top-width", cb.width, own_fs, ctx);
        let br = border_px(&bx.style, "border-right-width", cb.width, own_fs, ctx);
        let bb = border_px(&bx.style, "border-bottom-width", cb.width, own_fs, ctx);
        Rect {
            x: bx.rect.x + bl,
            y: bx.rect.y + bt,
            width: (bx.rect.width - bl - br).max(0.0),
            height: (bx.rect.height - bt - bb).max(0.0),
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
        // l'enfant n'a pas été posé — bootstrap du champ) —
        // via `out_of_flow` : une boîte `Text`/`AnonymousBlock`
        // clonant `position:absolute` du parent n'est PAS un
        // élément positionné (R-22 — sinon le texte d'un
        // absolu était re-posé une deuxième fois, décalé des
        // offsets de l'ancêtre à chaque niveau).
        let (pos, cst) = {
            let c = &bx.children[i];
            let p = if c.out_of_flow() {
                match c.style.get("position") {
                    Some("fixed") => Position::Fixed,
                    _ => Position::Absolute,
                }
            } else {
                Position::Static
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
        let geo = abs_geometry(&bx.children[i], &cst, target, cfs, ctx);
        let child = &mut bx.children[i];
        // Boîte absolue = BFC propre : registre vierge. Le CB
        // absolu est défini (padding box de l'ancêtre posé) —
        // ses hauteurs % résolvent (§10.5). `geo.cw` est la
        // **border box** exacte ; `layout_block` attend la
        // largeur conteneur en flux (`w:auto` → `cb_w - fixed
        // - ml - mr`) → on lui rend les marges pour que la
        // soustraction interne retombe sur `cw`.
        layout_block(
            child,
            geo.cx,
            geo.cy,
            geo.cw + geo.ml + geo.mr,
            Some(target.height),
            ctx,
            fs,
            target,
            &mut FloatCtx::default(),
        );
        // right/bottom sans left/top : ancrage au bord opposé
        // — `right`/`bottom` placent le bord de la margin
        // box, la border box recule de `mr`/`mb` (§10.3.7).
        // Le delta translate TOUT le sous-arbre : le contenu a
        // été posé pendant `layout_block` à l'ordonnée
        // initiale, il doit suivre la boîte (réf align-items
        // `bottom: 0` : le texte restait en haut).
        if geo.l.is_none() {
            if let Some(rv) = geo.r {
                let target_x = target.x + target.width - rv - geo.mr - child.rect.width;
                crate::flex::translate_subtree(child, target_x - child.rect.x, 0.0);
            }
        }
        if geo.t.is_none() {
            if let Some(bv) = geo.b {
                let target_y = target.y + target.height - bv - geo.mb - child.rect.height;
                crate::flex::translate_subtree(child, 0.0, target_y - child.rect.y);
            }
        }
    }
    // Descendre dans tous les enfants (cb mis à jour) avec
    // leur contexte de fonte — `own_fs` = fonte de `bx`.
    for child in &mut bx.children {
        // Descente ∝ profondeur de l'arbre → pile garantie (R-24).
        explorer_common::deep(|| resolve_absolutes(child, own_cb, ctx, own_fs));
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
/// directe, `normal` → métriques réelles de la fonte
/// (`natural_line_height`, §10.8.1 — 1.2em en repli).
fn line_height(style: &ComputedStyle, fs: f32, ctx: &BlockCtx) -> f32 {
    let normal = crate::font::natural_line_height(style, fs, ctx);
    let Some(raw) = style.get("line-height") else {
        return normal;
    };
    let raw = raw.trim();
    if raw == "normal" {
        return normal;
    }
    if let Ok(n) = raw.parse::<f32>() {
        return (n * fs).max(1.0);
    }
    css_px(style, "line-height", fs, fs, ctx).unwrap_or(normal)
}

/// Fonction de mesure px d'une chaîne, par la fonte réelle
/// via `FontSource`, sinon repli `0.55em`/caractère (dev
/// headless documenté — jamais la production).
pub(crate) fn measure_fn<'a>(
    style: &'a ComputedStyle,
    fs: f32,
    ctx: &'a BlockCtx,
) -> impl Fn(&str) -> f32 + 'a {
    let font = ctx.fonts.map(|src| (src, font::request_of(style, ctx)));
    #[allow(clippy::cast_precision_loss)]
    move |s: &str| {
        if let Some((src, req)) = &font {
            ctx.font_cache
                .borrow_mut()
                .shape(*src, req, s, fs, false)
                .map_or_else(|| s.chars().count() as f32 * fs * 0.55, |r| r.advance)
        } else {
            s.chars().count() as f32 * fs * 0.55
        }
    }
}
