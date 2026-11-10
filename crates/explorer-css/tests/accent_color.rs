// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! R-242 — `accent-color`/`caret-color` + batch dormant
//! (`overflow-anchor`, `overscroll-behavior`, `text-wrap*`…).

use explorer_css::{Origin, StyleEngine, StyleEnv};
use explorer_dom::Dom;
use explorer_html::TreeBuilder;

fn walk_x(
    dom: &Dom,
    n: explorer_dom::NodeId,
    sm: &explorer_css::StyleMap,
    out: &mut Option<explorer_css::ComputedStyle>,
) {
    if dom.attribute_str(n, "class").ok().flatten().is_some_and(|c| c == "x") {
        *out = sm.get(n).cloned();
    }
    if let Ok(ch) = dom.children(n) {
        for c in ch {
            walk_x(dom, c, sm, out);
        }
    }
}

fn style_of(css: &str) -> explorer_css::ComputedStyle {
    let dom = TreeBuilder::parse("<body><input class=x></body>");
    let mut eng = StyleEngine::new();
    eng.add_css(css, Origin::Author);
    let sm = eng.compute_tree(&dom, dom.document(), &StyleEnv::default());
    let mut out = None;
    walk_x(&dom, dom.document(), &sm, &mut out);
    out.expect("élément")
}

#[test]
fn accent_color_reconnu() {
    let st = style_of("input { accent-color: #1f883d }");
    assert_eq!(st.get("accent-color"), Some("#1f883d"));
}

#[test]
fn batch_dormant_props_reconnues() {
    for css in [
        "input { text-wrap: balance }",
        "input { overflow-anchor: none }",
        "input { overscroll-behavior: contain }",
        "input { print-color-adjust: exact }",
        "input { forced-color-adjust: none }",
        "input { field-sizing: content }",
        "input { contain-intrinsic-size: 100px }",
    ] {
        let st = style_of(css);
        let has = ["text-wrap","overflow-anchor","overscroll-behavior",
                   "print-color-adjust","forced-color-adjust","field-sizing",
                   "contain-intrinsic-size"]
            .iter().any(|p| st.get(p).is_some());
        assert!(has, "aucune prop reconnue pour {css:?}");
    }
}
