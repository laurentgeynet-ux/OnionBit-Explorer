// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! R-232 — `all: kw` : reset de toutes les longhands au
//! mot-clé (initial/inherit/unset/revert).

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

fn walk_x(dom: &Dom, n: explorer_dom::NodeId, sm: &explorer_css::StyleMap, out: &mut Option<explorer_css::ComputedStyle>) {
    if dom.attribute_str(n, "class").ok().flatten().is_some_and(|c| c == "x") {
        *out = sm.get(n).cloned();
    }
    if let Ok(ch) = dom.children(n) { for c in ch { walk_x(dom, c, sm, out); } }
}

#[test]
fn all_unset_reset_tout() {
    let st = style_of("p { color: red; margin: 10px; all: unset }");
    // color hérité → unset hérite du parent (pas red)
    assert_ne!(st.get("color"), Some("red"));
    // margin non-hérité → initial
    assert!(st.get("margin-top").is_none() || st.get("margin-top") == Some("0"));
}

#[test]
fn all_inherit_force_heritage() {
    let st = style_of("body { color: blue } p { color: red; all: inherit }");
    assert_eq!(st.get("color"), Some("blue"));
}
