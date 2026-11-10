// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! R-233 — `animation-*` longhands : posés séparément
//! (hors shorthand), `animation_decls` les lit par index.

use explorer_css::{Origin, StyleEngine, StyleEnv};
use explorer_dom::Dom;
use explorer_html::TreeBuilder;

fn style_of(cls: &str, css: &str) -> explorer_css::ComputedStyle {
    let dom = TreeBuilder::parse("<body><p class=x>ok</p></body>");
    let mut eng = StyleEngine::new();
    eng.add_css(css, Origin::Author);
    let sm = eng.compute_tree(&dom, dom.document(), &StyleEnv::default());
    fn walk(
        dom: &Dom,
        n: explorer_dom::NodeId,
        cls: &str,
        sm: &explorer_css::StyleMap,
        out: &mut Option<explorer_css::ComputedStyle>,
    ) {
        if dom.attribute_str(n, "class").ok().flatten().is_some_and(|c| c == cls) {
            *out = sm.get(n).cloned();
        }
        if let Ok(ch) = dom.children(n) {
            for c in ch {
                walk(dom, c, cls, sm, out);
            }
        }
    }
    let mut out = None;
    walk(&dom, dom.document(), cls, &sm, &mut out);
    out.expect("élément")
}

#[test]
fn longhands_poses_separement_lus() {
    let st = style_of(
        "x",
        "p { animation-name: spin; animation-duration: 2s;\
             animation-iteration-count: infinite; animation-direction: alternate }",
    );
    let decls = explorer_css::animation::animation_decls(&st);
    assert_eq!(decls.len(), 1);
    assert_eq!(decls[0].name, "spin");
    assert!((decls[0].duration - 2.0).abs() < f64::EPSILON);
    assert!(decls[0].iterations.is_infinite());
}

#[test]
fn longhand_name_seul_reconnu() {
    let st = style_of("x", "p { animation-name: fade }");
    assert_eq!(st.get("animation-name"), Some("fade"));
}
