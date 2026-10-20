// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

use explorer_css::{Origin, StyleEngine, StyleEnv};
use explorer_dom::Dom;

/// Le shorthand `font` sans `/lh` réinitialise `line-height`
/// à `normal` (CSS Fonts §3.6) — sinon un `line-height`
/// hérité survivrait à travers le raccourci
/// (WPT `margin-collapse-020`).
#[test]
fn font_shorthand_resets_line_height() {
    let mut dom = Dom::new();
    let doc = dom.document();
    let html = dom.create_element("html").unwrap();
    let p = dom.create_element("p").unwrap();
    dom.append_child(doc, html).unwrap();
    dom.append_child(html, p).unwrap();
    let mut e = StyleEngine::new();
    e.add_css("html { line-height: 1em }", Origin::Author);
    e.add_css("p { font: 12pt serif }", Origin::Author);
    let s = e.compute(&dom, p, &StyleEnv::default());
    assert_eq!(s.get("line-height"), Some("normal"));
    assert_eq!(s.get("font-size"), Some("16px"));
    assert_eq!(s.get("font-family"), Some("serif"));
    assert_eq!(s.get("font-weight"), Some("normal"));
}

/// Avec `/lh` explicite, le shorthand le conserve.
#[test]
fn font_shorthand_keeps_slash_line_height() {
    let mut dom = Dom::new();
    let doc = dom.document();
    let div = dom.create_element("div").unwrap();
    dom.append_child(doc, div).unwrap();
    let mut e = StyleEngine::new();
    e.add_css("div { font: 1em/1.25 serif }", Origin::Author);
    let s = e.compute(&dom, div, &StyleEnv::default());
    assert_eq!(s.get("line-height"), Some("1.25"));
}
