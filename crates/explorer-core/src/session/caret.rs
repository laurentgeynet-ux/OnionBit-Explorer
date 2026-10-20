// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Caret du champ éditable focalisé (constat terrain M-H1 #1) :
//! un champ focalisé ne montrait **rien** avant la première
//! frappe — la barre verticale est peinte en overlay de la
//! display list, sur le même mécanisme que le find et le popup
//! `<select>`.
//!
//! Position : fin du texte courant — dernier fragment posé
//! dans le sous-arbre du champ (`rect.x + width + trail_w`,
//! l'avance de l'espace final retirée du `width` à la
//! fermeture de ligne est rendue au caret) ; origine du
//! contenu quand la valeur est vide — le `placeholder` peint
//! n'est pas du texte édité. Point d'insertion interne
//! (flèches, clic en milieu de texte) : pas de MVP — limite
//! héritée d'`edit_field` qui n'insère qu'en fin.
//!
//! Coordonnées **frame** : `scroll_y` est retiré (les items
//! réguliers sont émis scroll-soustrait ; le find/popup en
//! coords document conservent leur convention existante).
//! Pas de clignotement au MVP : le caret est statique, un
//! toggle rythmé par `tick_animations` pourra l'animer.

use explorer_css::value::{parse_color, Color, Length};
use explorer_layout::font::{self, FontRequest, FontSource};
use explorer_layout::{Fragment, LayoutBox, LineBox, Rect};

use super::apply::{ApplyCtx, PaintPayload};
use super::dom_helpers::{dom_path_node, field_value};
use explorer_dom::Dom;

/// Largeur du caret en px CSS (~Chrome : 1 px).
const CARET_W: f32 = 1.0;

/// Rect (coords frame) + encre du caret du champ focalisé —
/// `None` quand rien ne doit peindre : pas de focus, chemin
/// mort (nœud re-parsé), boîte absente (champ `display:none`),
/// ou fonte illisible pour la mesure du repli.
pub(crate) fn caret_rect(
    p: &PaintPayload,
    ctx: &ApplyCtx,
    dom: Option<&Dom>,
) -> Option<(Rect, Color)> {
    let path = p.focused.as_deref()?;
    let dom = dom?;
    let tree = p.tree.as_ref()?;
    let node = dom_path_node(dom, path)?;
    let bx = box_of(tree, node)?;
    // Valeur éditable non vide → fin du dernier fragment posé
    // (texte réel) ; sinon origine du contenu — un
    // `placeholder` affiché sur valeur vide n'avance pas le
    // point d'insertion.
    let (x, y, h, ink) = if field_value(dom, node).is_empty() {
        caret_origin(bx, ctx)
    } else {
        last_text_frag(bx).map_or_else(
            || caret_origin(bx, ctx),
            |(f, line)| {
                (
                    f.rect.x + f.rect.width + f.trail_w,
                    line.rect.y,
                    line.rect.height.max(1.0),
                    f.style
                        .get("color")
                        .and_then(parse_color)
                        .unwrap_or(Color::BLACK),
                )
            },
        )
    };
    Some((
        Rect {
            x,
            y: y - p.scroll_y,
            width: CARET_W,
            height: h,
        },
        ink,
    ))
}

/// Première boîte posée pour `node` dans le sous-arbre —
/// parcours pré-ordre (le nœud du contrôle est porté par sa
/// propre boîte atomique, ses enfants synthétiques — bloc
/// anonyme + `Text` — partagent le même `node`).
fn box_of(bx: &LayoutBox, node: explorer_dom::NodeId) -> Option<&LayoutBox> {
    if bx.node == Some(node) {
        return Some(bx);
    }
    bx.children.iter().find_map(|c| box_of(c, node))
}

/// Dernier fragment de texte posé dans le sous-arbre, avec
/// sa ligne — `frag.rect` est la hauteur du glyphe posé, le
/// caret enjambe l'interligne (`line.rect`). Parcours
/// inversé (enfants d'abord, lignes ensuite : le texte d'un
/// contrôle vit dans les lignes de son bloc anonyme interne,
/// celui d'un `textarea` dans ses propres lignes ou un bloc
/// anonyme enfant).
fn last_text_frag(bx: &LayoutBox) -> Option<(&Fragment, &LineBox)> {
    for c in bx.children.iter().rev() {
        if let Some(t) = last_text_frag(c) {
            return Some(t);
        }
    }
    for line in bx.lines.iter().rev() {
        if let Some(f) = line
            .fragments
            .iter()
            .rev()
            .find(|f| !f.atomic && !f.text.is_empty())
        {
            return Some((f, line));
        }
    }
    None
}

/// Repli « valeur vide » : origine de la boîte de contenu
/// (padding + bordure) et hauteur d'interligne de la fonte
/// du champ — repli `font-size × 1.2` si les octets de fonte
/// sont illisibles.
fn caret_origin(bx: &LayoutBox, ctx: &ApplyCtx) -> (f32, f32, f32, Color) {
    let st = &bx.style;
    let fs = font_size_px(st);
    let x = bx.rect.x + side_px(st, "border-left-width", fs) + side_px(st, "padding-left", fs);
    let y = bx.rect.y + side_px(st, "border-top-width", fs) + side_px(st, "padding-top", fs);
    let h = font_line_h(st, fs, &ctx.fonts).unwrap_or(fs * 1.2).max(1.0);
    let ink = st
        .get("color")
        .and_then(parse_color)
        .unwrap_or(Color::BLACK);
    (x, y, h, ink)
}

/// `font-size` calculée en px (la cascade la stocke résolue,
/// R-17 — les unités relatives retombent sur ×fs comme dans
/// le display).
#[allow(clippy::cast_possible_truncation)] // px CSS ≪ 2^24
fn font_size_px(st: &explorer_css::ComputedStyle) -> f32 {
    let l = st.get("font-size").and_then(parse_len);
    l.map_or(16.0, |l| {
        l.unit
            .px_factor()
            .map_or_else(|| l.value * 16.0, |f| l.value * f) as f32
    })
}

/// Épaisseur d'un côté (`padding-*`/`border-*-width`) en px —
/// approx `em`/`rem` → ×`font-size` (même approximation que
/// `prop_px` du rendu).
#[allow(clippy::cast_possible_truncation)] // px CSS ≪ 2^24
fn side_px(st: &explorer_css::ComputedStyle, name: &str, fs: f32) -> f32 {
    let Some(l) = st.get(name).and_then(parse_len) else {
        return 0.0;
    };
    l.unit
        .px_factor()
        .map_or_else(|| l.value * f64::from(fs), |f| l.value * f) as f32
}

fn parse_len(raw: &str) -> Option<Length> {
    explorer_css::value::parse_length(raw)
}

/// Interligne de la fonte du champ à `fs` — requête
/// `font-family` du style, `sans-serif` en repli (même
/// convention que le popup `<select>`).
fn font_line_h(st: &explorer_css::ComputedStyle, fs: f32, fonts: &dyn FontSource) -> Option<f32> {
    let families: Vec<String> = st.get("font-family").map_or_else(
        || vec!["sans-serif".to_string()],
        |f| {
            f.split(',')
                .map(|s| s.trim().trim_matches(|c| c == '"' || c == '\'').to_string())
                .collect()
        },
    );
    let req = FontRequest {
        families,
        weight: st
            .get("font-weight")
            .and_then(|w| match w.trim() {
                "bold" | "bolder" => Some(700u16),
                "normal" | "lighter" => Some(400),
                v => v.parse().ok(),
            })
            .unwrap_or(400),
        italic: matches!(st.get("font-style"), Some("italic" | "oblique")),
    };
    let data = fonts.font_data(&req)?;
    Some(font::metrics(&data, fs)?.line_height())
}
