// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! R-249 — `text-wrap`/`text-wrap-mode`/`text-wrap-style` :
//! shorthand expand, `mode:nowrap` → `white-space:nowrap`.

use explorer_css::{Origin, StyleEngine, StyleEnv};
use explorer_dom::Dom;
use explorer_html::TreeBuilder;

fn walk_x(dom: &Dom, n: explorer_dom::NodeId, sm: &explorer_css::StyleMap, out: &mut Option<explorer_css::ComputedStyle>) {
    if dom.attribute_str(n, "class").ok().flatten().is_some_and(|c| c == "x") {
        *out = sm.get(n).cloned();
    }
    if let Ok(ch) = dom.children(n) {
        for c in ch { walk_x(dom, c, sm, out); }
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
fn shorthand_mode() {
    let st = style_of("p { text-wrap: nowrap }");
    assert_eq!(st.get("white-space"), Some("nowrap"));
}

#[test]
fn shorthand_style() {
    let st = style_of("p { text-wrap: wrap balance }");
    assert_eq!(st.get("text-wrap-style"), Some("balance"));
    assert_eq!(st.get("white-space"), Some("normal"));
}

#[test]
fn mode_seul() {
    let st = style_of("p { text-wrap-mode: nowrap }");
    assert_eq!(st.get("white-space"), Some("nowrap"));
}
