// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! R-237 — `@namespace` : `ns|tag` résolus par URI,
//! `|tag` sans namespace, `*|tag` tout.

use explorer_css::{Origin, StyleEngine, StyleEnv};
use explorer_dom::Dom;
use explorer_html::TreeBuilder;

fn style_of_tag(html: &str, css: &str, tag: &str) -> explorer_css::ComputedStyle {
    let dom = TreeBuilder::parse(html);
    let mut eng = StyleEngine::new();
    eng.add_css(css, Origin::Author);
    let sm = eng.compute_tree(&dom, dom.document(), &StyleEnv::default());
    let mut out = None;
    walk_tag(&dom, dom.document(), tag, &sm, &mut out);
    out.expect("élément")
}

fn walk_tag(
    dom: &Dom,
    n: explorer_dom::NodeId,
    tag: &str,
    sm: &explorer_css::StyleMap,
    out: &mut Option<explorer_css::ComputedStyle>,
) {
    if out.is_none() && dom.tag_name(n).is_ok_and(|t| t == tag) {
        *out = sm.get(n).cloned();
    }
    if let Ok(ch) = dom.children(n) {
        for c in ch {
            walk_tag(dom, c, tag, sm, out);
        }
    }
}

#[test]
fn svg_prefixed_selector_matches_svg_not_html() {
    let html = "<body><svg><a>x</a></svg><a>y</a></body>";
    let css = "@namespace svg \"http://www.w3.org/2000/svg\";\
         svg|a { color: blue }";
    let st = style_of_tag(html, css, "a");
    // premier <a> trouvé = svg (ordre DOM)
    assert_eq!(st.get("color"), Some("blue"));
}

#[test]
fn html_a_not_matched_by_svg_ns() {
    let html = "<body><a>y</a></body>";
    let css = "@namespace svg \"http://www.w3.org/2000/svg\";\
         svg|a { color: blue }";
    let st = style_of_tag(html, css, "a");
    assert!(st.get("color").is_none() || st.get("color") == Some("black") || st.get("color") != Some("blue"));
}

#[test]
fn star_ns_matches_any() {
    let html = "<body><svg><a>x</a></svg></body>";
    let css = "*|a { color: blue }";
    let st = style_of_tag(html, css, "a");
    assert_eq!(st.get("color"), Some("blue"));
}

