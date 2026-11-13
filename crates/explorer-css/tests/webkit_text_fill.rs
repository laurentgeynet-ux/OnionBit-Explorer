// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! R-235 — `-webkit-text-fill-color` remplace `color` pour
//! le remplissage du texte ; `-webkit-text-stroke*` reconnus.

use explorer_css::{Origin, StyleEngine, StyleEnv};
use explorer_dom::Dom;
use explorer_html::TreeBuilder;

fn style_of(css: &str) -> explorer_css::ComputedStyle {
    let dom = TreeBuilder::parse("<body><p class=x>ok</p></body>");
    let mut eng = StyleEngine::new();
    eng.add_css(css, Origin::Author);
    let sm = eng.compute_tree(&dom, dom.document(), &StyleEnv::default());
    let mut out = None;
    walk_x(&dom, dom.document(), &sm, &mut out);
    out.expect("élément")
}

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

#[test]
fn text_fill_remplace_color() {
    let st = style_of("p { color: red; -webkit-text-fill-color: blue }");
    assert_eq!(st.get("color"), Some("blue"));
}

#[test]
fn transparent_garde_color() {
    let st = style_of("p { color: red; -webkit-text-fill-color: transparent }");
    assert_eq!(st.get("color"), Some("red"));
}

#[test]
fn stroke_props_reconnus() {
    // Shorthand `<width> <color>` expansé en longhands
    // (spec — avant, l'alias vendor rabattait tout sur
    // `-webkit-text-stroke-width` et la couleur était perdue).
    let st = style_of("p { -webkit-text-stroke: 1px black }");
    assert_eq!(st.get("-webkit-text-stroke-width"), Some("1px"));
    assert_eq!(st.get("-webkit-text-stroke-color"), Some("black"));
}
