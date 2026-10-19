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

use crate::boxtree::{BoxKind, LayoutBox, Rect};
use crate::font::{self, FontRequest, FontSource};
use crate::text;

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
}

impl Default for BlockCtx<'_> {
    fn default() -> Self {
        Self {
            fonts: None,
            default_family: "sans-serif",
            base_font_size: 16.0,
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
    layout_block(root, 0.0, 0.0, viewport_w, ctx, ctx.base_font_size);
}

/// Résout `font-size` en px : `em`/`rem` relatifs au parent
/// (ou à la base), `%` idem, absolus directs.
fn font_size_px(style: &ComputedStyle, parent_fs: f32, ctx: &BlockCtx) -> f32 {
    match css_px(style, "font-size", parent_fs, parent_fs, ctx) {
        Some(v) if v > 0.0 => v,
        _ => parent_fs.max(1.0),
    }
}

/// `prop` → px. `pct_basis` = base des pourcents (largeur du
/// bloc conteneur ou font-size selon la propriété) ;
/// `em_basis` = base des unités `em`.
/// `auto`/`medium`/inconnu → `None`.
fn css_px(
    style: &ComputedStyle,
    prop: &str,
    pct_basis: f32,
    em_basis: f32,
    ctx: &BlockCtx,
) -> Option<f32> {
    let raw = style.get(prop)?;
    let raw = raw.trim();
    if let Some(pct) = raw.strip_suffix('%') {
        return pct
            .trim()
            .parse::<f32>()
            .ok()
            .map(|v| v * pct_basis / 100.0);
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
/// `bx.rect` et descend récursivement.
fn layout_block(
    bx: &mut LayoutBox,
    x: f32,
    y: f32,
    cb_w: f32,
    ctx: &BlockCtx,
    parent_fs: f32,
) -> f32 {
    let st = bx.style.clone();
    let fs = font_size_px(&st, parent_fs, ctx);

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
    let w_spec = css_px(&st, "width", cb_w, fs, ctx);
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
    let mut cursor = y + bt + pt;

    // Empilement vertical avec marges collapsées.
    let adjoining_top = pt + bt <= 0.0;
    let mut prev_margin: Option<f32> = None;
    let mut first = true;
    let count = bx.children.len();
    for i in 0..count {
        let (kind, cst) = {
            let c = &bx.children[i];
            (c.kind, c.style.clone())
        };
        if kind.is_inline_level() {
            // Enfant en-ligne orphelin (modèle assoupli des
            // boîtes inline contenant du bloc) — hauteur
            // d'une ligne.
            cursor += line_height(&st, fs, ctx);
            continue;
        }
        let cfs = font_size_px(&cst, fs, ctx);
        let cm = margins(&cst, content_w, cfs, ctx);
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
        let child = &mut bx.children[i];
        let used = match kind {
            BoxKind::AnonymousBlock => {
                layout_anon_block(child, content_x, cursor, content_w, ctx, fs)
            }
            _ => layout_block(child, content_x, cursor, content_w, ctx, fs),
        };
        cursor += used;
        prev_margin = Some(cm.bottom);
        first = false;
    }
    // Dernier enfant adjoining : sa marge traverse aussi —
    // le contenu s'arrête à cursor ; l'effet complet viendra
    // avec la propagation des marges en used value (le parent
    // devra combiner sa marge basse avec celle du dernier
    // enfant : conservé simple ici, l'écart max est une marge
    // de bloc — étape 21 affinera floats/clearance).

    let content_h = cursor - (y + bt + pt);
    let h_spec = css_px(&st, "height", ctx.base_font_size, fs, ctx);
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
    bx.rect.height
}

/// Hauteur d'un bloc anonyme : empilement de ses lignes
/// (estimation — les vraies line boxes arrivent à l'étape 20).
fn layout_anon_block(
    bx: &mut LayoutBox,
    x: f32,
    y: f32,
    w: f32,
    ctx: &BlockCtx,
    parent_fs: f32,
) -> f32 {
    let st = bx.style.clone();
    let fs = font_size_px(&st, parent_fs, ctx);
    let lh = line_height(&st, fs, ctx);
    let measure = measure_fn(&st, fs, ctx);
    let mut cursor = y;
    let count = bx.children.len();
    for i in 0..count {
        // Texte du fragment (l'en-ligne porte son texte dans
        // ses propres descendants : on concatène).
        let txt = collect_text(&bx.children[i]);
        if txt.trim().is_empty() {
            continue;
        }
        let lines = text::break_lines(&txt, w, &measure).len().max(1);
        #[allow(clippy::cast_precision_loss)]
        let h = lines as f32 * lh;
        let child = &mut bx.children[i];
        child.rect = Rect {
            x,
            y: cursor,
            width: w,
            height: h,
        };
        cursor += h;
    }
    bx.rect = Rect {
        x,
        y,
        width: w,
        height: cursor - y,
    };
    cursor - y
}

/// Texte aplati d'un fragment en-ligne (texte direct ou
/// descendants des boîtes inline).
fn collect_text(b: &LayoutBox) -> String {
    if let Some(t) = &b.text {
        t.clone()
    } else {
        let mut s = String::new();
        for c in &b.children {
            s.push_str(&collect_text(c));
        }
        s
    }
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
fn measure_fn<'a>(
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
