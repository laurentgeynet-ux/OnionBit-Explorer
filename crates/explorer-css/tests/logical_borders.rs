// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! R-236 — longhands `border-*` logiques : routés vers le
//! physique par `resolve_logical_props` (writing-mode+direction).

use explorer_css::{Origin, StyleEngine, StyleEnv};
use explorer_dom::Dom;
use explorer_html::TreeBuilder;

fn walk_x(
    dom: &Dom,
    n: explorer_dom::NodeId,
    sm: &explorer_css::StyleMap,
    out: &mut Option<explorer_css::ComputedStyle>,
) {
    if dom
        .attribute_str(n, "class")
        .ok()
        .flatten()
        .is_some_and(|c| c == "x")
    {
        *out = sm.get(n).cloned();
    }
    if let Ok(ch) = dom.children(n) {
        for c in ch {
            walk_x(dom, c, sm, out);
        }
    }
}

fn style_of(css: &str) -> explorer_css::ComputedStyle {
    let dom = TreeBuilder::parse("<body><p class=x>ok</p></body>");
    let mut eng = StyleEngine::new();
    eng.add_css(css, Origin::Author);
    let sm = eng.compute_tree(&dom, dom.document(), &StyleEnv::default());
    let mut out = None;
    walk_x(&dom, dom.document(), &sm, &mut out);
    out.expect("élément")
}

#[test]
fn inline_start_route_gauche_ltr() {
    let st = style_of("p { border-inline-start-width: 4px }");
    assert_eq!(st.get("border-left-width"), Some("4px"));
}

#[test]
fn inline_start_route_droite_rtl() {
    let st = style_of("body { direction: rtl } p { border-inline-start-width: 4px }");
    assert_eq!(st.get("border-right-width"), Some("4px"));
}

#[test]
fn block_start_route_haut() {
    let st = style_of("p { border-block-start-color: red }");
    assert_eq!(st.get("border-top-color"), Some("red"));
}

#[test]
fn radius_coin_logique_route_physique() {
    let st = style_of("p { border-start-start-radius: 8px }");
    assert_eq!(st.get("border-top-left-radius"), Some("8px"));
}

#[test]
fn bare_border_logical_shorthand_route_physique() {
    // Le shorthand nu `border-inline-end` éclate en longhands
    // logiques, routées vers le physique (LTR → right) —
    // `ComputedStyle.get` ne re-synthétise pas de shorthand.
    let st = style_of("p { border-inline-end: 2px solid blue }");
    assert_eq!(st.get("border-right-width"), Some("2px"));
    assert_eq!(st.get("border-right-style"), Some("solid"));
    assert_eq!(st.get("border-right-color"), Some("blue"));
}

// BCD-2 — overflow flow-relative + valeurs logiques de
// `float`/`clear`/`text-align` (css-logical-1 §4.1,
// css-text-3 §8.1).

#[test]
fn overflow_block_route_y_horizontal() {
    let st = style_of("p { overflow-block: hidden }");
    assert_eq!(st.get("overflow-y"), Some("hidden"));
    assert_eq!(st.get("overflow-x"), None);
}

#[test]
fn overflow_inline_route_x_horizontal() {
    let st = style_of("p { overflow-inline: scroll }");
    assert_eq!(st.get("overflow-x"), Some("scroll"));
}

#[test]
fn overflow_block_route_x_vertical() {
    let st = style_of("p { writing-mode: vertical-rl; overflow-block: clip }");
    assert_eq!(st.get("overflow-x"), Some("clip"));
    assert_eq!(st.get("overflow-y"), None);
}

#[test]
fn float_inline_start_resout_left_ltr() {
    let st = style_of("p { float: inline-start }");
    assert_eq!(st.get("float"), Some("left"));
}

#[test]
fn float_inline_end_resout_right_ltr() {
    let st = style_of("p { float: inline-end }");
    assert_eq!(st.get("float"), Some("right"));
}

#[test]
fn clear_inline_end_resout_left_rtl() {
    let st = style_of("body { direction: rtl } p { clear: inline-end }");
    assert_eq!(st.get("clear"), Some("left"));
}

#[test]
fn text_align_match_parent_resout_direction_element() {
    // Parent `start` (défaut) : `match-parent` résout dans
    // la direction de L'ÉLÉMENT — rtl → right (css-text-3).
    let st = style_of("body { direction: rtl } p { text-align: match-parent }");
    assert_eq!(st.get("text-align"), Some("right"));
}

#[test]
fn text_align_match_parent_ltr_gauche() {
    let st = style_of("p { text-align: match-parent }");
    assert_eq!(st.get("text-align"), Some("left"));
}

