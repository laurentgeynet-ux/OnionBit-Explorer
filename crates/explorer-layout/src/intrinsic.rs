// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Tailles intrinsèques (R-3) : `min-content`, `max-content`
//! et `fit-content` mesurées sur le contenu **réel** — texte
//! pesé via la fonte injectée (ADR-0009), sous-arbres
//! récursés. Fini l'approximation « taille du texte direct »
//! pour la mesure ; le layout `width` les résout en px.
//!
//! Convention CSS Sizing : le **max-content** est la plus
//! grande largeur sans coupure souple ; le **min-content**
//! le plus grand morceau insécable (mot le plus long,
//! atome remplacé le plus large). `fit-content` =
//! `min(max-content, max(min-content, disponible))`.
//!
//! Approximations documentées (MVP) :
//! - les enfants de niveau bloc contribuent leur **max**
//!   (flux vertical — les largeurs ne s'additionnent pas) ;
//! - une suite d'enfants en-ligne contribue la **somme** des
//!   max-content, et le **max** des min-content (les
//!   coupures souples autorisées entre eux) ;
//! - `table-row` somme ses cellules (colonnes côte à côte) ;
//! - marges/paddings des enfants ignorés, sauf le padding
//!   propre porté par l'appelant (`width` = content box).

use crate::block::{box_size_to_content, css_px, font_size_px, measure_fn, side_px, BlockCtx};
use crate::boxtree::{BoxKind, LayoutBox};
use explorer_css::ComputedStyle;

/// `width`/`min-width`/`max-width`/`flex-basis` mot-clé → px
/// mesurés, `None` si la valeur n'est pas un mot-clé
/// intrinsèque.
pub(crate) fn intrinsic_size_px(
    st: &ComputedStyle,
    prop: &str,
    bx: &LayoutBox,
    basis: f32,
    fs: f32,
    ctx: &BlockCtx,
) -> Option<f32> {
    match st.get(prop)?.trim() {
        "max-content" => Some(max_content_px(bx, fs, ctx)),
        "min-content" => Some(min_content_px(bx, fs, ctx)),
        "fit-content" => {
            let avail = (basis
                - side_px(st, "margin-left", basis, fs, ctx, 0.0)
                - side_px(st, "margin-right", basis, fs, ctx, 0.0))
            .max(0.0);
            Some(max_content_px(bx, fs, ctx).min(min_content_px(bx, fs, ctx).max(avail)))
        }
        _ => None,
    }
}

/// Largeur max-content du contenu de `bx` (content box).
pub(crate) fn max_content_px(bx: &LayoutBox, fs: f32, ctx: &BlockCtx) -> f32 {
    content_px(bx, fs, ctx, true)
}

/// Largeur min-content du contenu de `bx` (content box).
pub(crate) fn min_content_px(bx: &LayoutBox, fs: f32, ctx: &BlockCtx) -> f32 {
    content_px(bx, fs, ctx, false)
}

/// Mesure mutualisée : `max` = insécable complet, `min` =
/// plus grand morceau insécable. `fs` = font-size **de bx**
/// (résolu par l'appelant).
fn content_px(bx: &LayoutBox, fs: f32, ctx: &BlockCtx, max: bool) -> f32 {
    // Taille directe : `width` explicite (hors %) fait foi
    // AVANT l'intrinsèque remplacé (§5.1 : la contribution
    // d'un remplacé est sa taille utilisée — un
    // `img{width:40px}` mesure 40, pas son intrinsèque.
    // Terrain : icônes sociales rust-lang.org empilées une
    // par ligne, la base flex des `<a>` valant l'intrinsèque
    // 512 > conteneur). Un `%` indéfini retombe sur le
    // contenu (§5.2). `min|max-width` non-% bornent la
    // source retenue. `box-sizing` convertit les tailles
    // SPÉCIFIÉES en contenu (R-43) — l'intrinsèque est déjà
    // du contenu (les marges restent hors — comptées par
    // `own_sides` côté appelant).
    let st = &bx.style;
    let chassis: f32 = [
        "padding-left",
        "padding-right",
        "border-left-width",
        "border-right-width",
    ]
    .iter()
    .map(|p| side_px(st, p, 0.0, fs, ctx, 0.0))
    .sum();
    let conv = |v: f32| box_size_to_content(st, v, chassis);
    let bound = |p: &str| -> Option<f32> {
        let v = st.get(p)?;
        if v.contains('%') {
            return None;
        }
        css_px(st, p, 0.0, fs, ctx).map(conv)
    };
    // §4.1 : un `width:%` en mesure intrinsèque est indéfini
    // (le bloc contenant dépend de cette mesure) — un
    // remplacé n'impose alors **pas** son intrinsèque : sa
    // contribution est nulle, sans quoi un `img{width:100%}`
    // gonflerait tout ancêtre shrink-to-fit à la taille du
    // bitmap (terrain : carte BBC `min-width:fit-content` à
    // 480px dès l'arrivée du bitmap, débordant sa piste de
    // grille 236px). Un non-remplacé retombe sur son contenu
    // (`bx.intrinsic` absent → `None` → mesure des enfants).
    let pct = st.get("width").is_some_and(|v| v.contains('%'));
    let base = if pct && bx.intrinsic.is_some() {
        Some(0.0)
    } else {
        bound("width").or(bx.intrinsic.map(|i| i.0))
    };
    if let Some(mut w) = base {
        if let Some(mx) = bound("max-width") {
            w = w.min(mx);
        }
        if let Some(mn) = bound("min-width") {
            w = w.max(mn);
        }
        return w;
    }
    // Boîte texte : mesure de la fonte réelle.
    if bx.kind == BoxKind::Text {
        if let Some(t) = &bx.text {
            return text_px(&bx.style, t, fs, ctx, max);
        }
        return 0.0;
    }
    // Conteneur flex en axe horizontal (§9.9) : les items
    // s'alignent sur le main axis → max-content = somme d'une
    // ligne ; min-content d'un conteneur `wrap` = plus grand
    // item (chacun peut tenir sa propre ligne), `nowrap` =
    // somme également (aucune coupure possible).
    if matches!(bx.display(), "flex" | "inline-flex") {
        let dir = bx.style.get("flex-direction").unwrap_or("row");
        if matches!(dir.trim(), "row" | "row-reverse") {
            let wrap = bx
                .style
                .get("flex-wrap")
                .is_some_and(|v| v.trim() != "nowrap");
            let items: Vec<f32> = bx
                .children
                .iter()
                .map(|c| {
                    let cfs = font_size_px(&c.style, fs, ctx);
                    explorer_common::deep(|| content_px(c, cfs, ctx, max)) + own_sides(c, fs, ctx)
                })
                .collect();
            if max || !wrap {
                return items.iter().sum();
            }
            return items.iter().fold(0.0_f32, |a, &b| a.max(b));
        }
    }
    // Ligne de table = somme des cellules (colonnes).
    // Chaque arête ∝ profondeur de l'arbre passe par
    // `explorer_common::deep` (R-24).
    if bx.display() == "table-row" {
        return bx
            .children
            .iter()
            .map(|c| {
                explorer_common::deep(|| content_px(c, font_size_px(&c.style, fs, ctx), ctx, max))
                    + own_sides(c, fs, ctx)
            })
            .sum();
    }
    // Conteneur : enfants en-ligne s'additionnent (max) ou se
    // maximisent par morceau (min) ; enfants bloc = max.
    let mut best = 0.0f32;
    let mut run = 0.0f32;
    for c in &bx.children {
        let cfs = font_size_px(&c.style, fs, ctx);
        if inline_level(c) {
            let w = explorer_common::deep(|| content_px(c, cfs, ctx, max));
            if max {
                run += w;
            } else {
                run = run.max(w);
            }
        } else if matches!(c.style.get("float"), Some("left" | "right")) {
            // Enfant flottant : à max-content il se tasse en
            // ligne avec ses voisins flottants — la même
            // contribution de « run » que l'en-ligne (un bloc
            // en-flux clôt la suite), **châssis compris**
            // (§4.2 : la contribution inclut marges, padding
            // et bordures). Le style est lu ici : `c.float`
            // n'est posé que lors de la pose de l'enfant.
            let w = explorer_common::deep(|| content_px(c, cfs, ctx, max)) + own_sides(c, fs, ctx);
            if max {
                run += w;
            } else {
                run = run.max(w);
            }
        } else {
            best = best.max(run);
            run = 0.0;
            best = best.max(
                explorer_common::deep(|| content_px(c, cfs, ctx, max)) + own_sides(c, fs, ctx),
            );
        }
    }
    best.max(run)
}

/// `true` = enfant qui coule en ligne (texte, inline,
/// remplacé) — regroupé en « run » pour la mesure.
fn inline_level(c: &LayoutBox) -> bool {
    c.kind.is_inline_level() || c.display() == "inline-block" || c.intrinsic.is_some()
}

/// Mesure `text` : max = chaîne entière (espaces effondrés,
/// §16.6.1) ; min = le mot le plus long.
fn text_px(st: &ComputedStyle, text: &str, fs: f32, ctx: &BlockCtx, max: bool) -> f32 {
    let measure = measure_fn(st, fs, ctx);
    if max {
        return measure(&crate::inline::collapse_ws(text));
    }
    text.split(crate::text::is_css_ws)
        .filter(|w| !w.is_empty())
        .map(&measure)
        .fold(0.0, f32::max)
}

/// Marges + padding + bordures horizontales propres d'un
/// enfant — comptées dans l'intrinsèque du parent (la boîte
/// de marge déborde la content box du conteneur).
pub(crate) fn own_sides(c: &LayoutBox, fs: f32, ctx: &BlockCtx) -> f32 {
    let st = &c.style;
    let cfs = font_size_px(st, fs, ctx);
    let mut acc = 0.0f32;
    for prop in [
        "margin-left",
        "margin-right",
        "padding-left",
        "padding-right",
        "border-left-width",
        "border-right-width",
    ] {
        acc += side_px(st, prop, 0.0, cfs, ctx, 0.0);
    }
    acc
}
