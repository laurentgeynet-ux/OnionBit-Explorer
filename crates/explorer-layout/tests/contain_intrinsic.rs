// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! R-251 — `contain-intrinsic-*`/`content-visibility`/`contain: size` :
//! la boîte saute ses enfants et prend la taille déclarée.

use explorer_css::{Origin, StyleEngine, StyleEnv};
use explorer_html::TreeBuilder;

fn find(
    bx: &explorer_layout::LayoutBox,
    dom: &explorer_dom::Dom,
    class: &str,
) -> Option<(f32, f32)> {
    if bx
        .node
        .and_then(|n| dom.attribute_str(n, "class").ok().flatten())
        .is_some_and(|c| c == class)
    {
        return Some((
            bx.intrinsic.map_or(-1.0, |i| i.0),
            bx.intrinsic.map_or(-1.0, |i| i.1),
        ));
    }
    for c in &bx.children {
        if let Some(r) = find(c, dom, class) {
            return Some(r);
        }
    }
    None
}

fn rect_of(html: &str, css: &str, class: &str) -> (f32, f32) {
    let dom = TreeBuilder::parse(html);
    let mut eng = StyleEngine::new();
    eng.add_css(css, Origin::Author);
    let styles = eng.compute_tree(&dom, dom.document(), &StyleEnv::default());
    let bx = explorer_layout::build_box_tree(&dom, dom.document(), &styles).expect("arbre");
    find(&bx, &dom, class).unwrap_or((-1.0, -1.0))
}

#[test]
fn content_visibility_intrinsic() {
    let (w, h) = rect_of(
        "<body><div class=x><p>contenu</p></div></body>",
        ".x { content-visibility: auto; contain-intrinsic-size: 200px 80px }",
        "x",
    );
    assert_eq!((w, h), (200.0, 80.0));
}

#[test]
fn contain_size_intrinsic() {
    let (w, h) = rect_of(
        "<body><div class=x><p>contenu</p></div></body>",
        ".x { contain: size; contain-intrinsic-width: 300px; contain-intrinsic-height: 60px }",
        "x",
    );
    assert_eq!((w, h), (300.0, 60.0));
}
