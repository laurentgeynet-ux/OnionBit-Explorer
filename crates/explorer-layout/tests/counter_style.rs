// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! R-221 — `@counter-style <nom>` : style de marqueur
//! custom résolu dans `list-style-type: <nom>`
//! (`css.at.unapplied.counter-style` ×9 sur wikipedia).

mod common;

use explorer_css::{Origin, StyleEngine, StyleEnv};
use explorer_dom::Dom;
use explorer_html::TreeBuilder;
use explorer_layout::{build_box_tree, layout_tree, BlockCtx, BoxKind, LayoutBox};

fn laid(html: &str, css: &str) -> (Dom, LayoutBox) {
    let dom = TreeBuilder::parse(html);
    let mut eng = StyleEngine::new();
    eng.add_css(css, Origin::Author);
    let styles = eng.compute_tree(&dom, dom.document(), &StyleEnv::default());
    let root = dom
        .children(dom.document())
        .ok()
        .and_then(|ch| ch.into_iter().find(|&c| dom.element(c).is_ok()))
        .unwrap_or(dom.document());
    let mut bx = build_box_tree(&dom, root, &styles).unwrap();
    layout_tree(&mut bx, 800.0, &BlockCtx::default());
    (dom, bx)
}

fn find_li<'a>(b: &'a LayoutBox, out: &mut Vec<&'a LayoutBox>) {
    if b.kind == BoxKind::ListItem {
        out.push(b);
    }
    for c in &b.children {
        find_li(c, out);
    }
}

#[test]
fn counter_style_nomme_porte_le_style() {
    let (_dom, bx) = laid(
        "<!doctype html><body><ol style=\"list-style-type:roman-alt\">\
         <li>a</li><li>b</li><li>c</li></ol></body>",
        "@counter-style roman-alt { system: cyclic; symbols: \"x\" \"y\" \"z\"; suffix: \")\" }",
    );
    let mut lis = Vec::new();
    find_li(&bx, &mut lis);
    assert_eq!(lis.len(), 3, "trois li attendus");
    assert!(
        lis.iter().all(|li| li.counter.is_some()),
        "le CounterStyle doit voyager sur chaque li"
    );
    // cyclic x/y/z sur ordinals 1/2/3, suffix ")".
    let got: Vec<String> = lis
        .iter()
        .map(|li| li.counter.as_ref().unwrap().label(li.marker_ordinal))
        .collect();
    assert_eq!(got, vec!["x)", "y)", "z)"], "labels cycliques");
}
