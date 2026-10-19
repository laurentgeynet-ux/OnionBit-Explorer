// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Dump de diagnostic — arbre de boîtes après layout + display
//! list, pour inspecter un fixture WPT ou n'importe quel HTML.
//!
//! ```text
//! cargo test -p explorer-render --test debug_dump -- --nocapture
//! set WPT_FILE=tests/data/wpt/css/CSS2/floats/floats-placement-001.html
//! ```

use std::path::PathBuf;

use explorer_css::{Origin, StyleEngine, StyleEnv};
use explorer_dom::Dom;
use explorer_html::TreeBuilder;
use explorer_layout::{build_box_tree, layout_tree, BlockCtx, LayoutBox};

fn collect_css(dom: &Dom) -> String {
    let mut css = String::new();
    let mut queue = vec![dom.document()];
    while let Some(n) = queue.pop() {
        if dom.tag_name(n).is_ok_and(|t| t == "style") {
            if let Ok(t) = dom.text_content(n) {
                css.push_str(&t);
                css.push('\n');
            }
        }
        if let Ok(ch) = dom.children(n) {
            queue.extend(ch.iter().rev());
        }
    }
    css
}

fn dump(b: &LayoutBox, d: usize) {
    let r = b.rect;
    eprintln!(
        "{:w$}{:?} flt={:?} [{:.1} {:.1} {:.1}x{:.1}]",
        "",
        b.kind,
        b.float,
        r.x,
        r.y,
        r.width,
        r.height,
        w = d * 2
    );
    for c in &b.children {
        dump(c, d + 1);
    }
}

#[test]
fn dump_layout() {
    let p =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(std::env::var("WPT_FILE").unwrap_or_else(
            |_| "tests/data/wpt/css/CSS2/floats/floats-placement-001.html".to_string(),
        ));
    let src = std::fs::read_to_string(p).expect("fixture");
    let dom = TreeBuilder::parse(&src);
    let mut eng = StyleEngine::new();
    eng.add_css(&collect_css(&dom), Origin::Author);
    let styles = eng.compute_tree(&dom, dom.document(), &StyleEnv::default());
    let root = dom
        .children(dom.document())
        .into_iter()
        .flatten()
        .find(|&c| dom.element(c).is_ok())
        .expect("root");
    let mut bx = build_box_tree(&dom, root, &styles).expect("arbre");
    layout_tree(&mut bx, 800.0, &BlockCtx::default());
    dump(&bx, 0);
    // Display items pour inspection.
    let dl = explorer_render::build_display_list(&bx, 800.0, 600.0, 0.0);
    for it in &dl.items {
        eprintln!("  ITEM {it:?}");
    }
}
