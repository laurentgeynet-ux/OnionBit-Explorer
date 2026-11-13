// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! R-241 — `font-variant-numeric` : mots-clés → tags OT
//! équivalents dans `font-feature-settings` (css-fonts-4 §6.3).

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
    let dom = TreeBuilder::parse("<body><p class=x>ok</p></body>");
    let mut eng = StyleEngine::new();
    eng.add_css(css, Origin::Author);
    let sm = eng.compute_tree(&dom, dom.document(), &StyleEnv::default());
    let mut out = None;
    walk_x(&dom, dom.document(), &sm, &mut out);
    out.expect("élément")
}

#[test]
fn tabular_nums_produit_tnum() {
    let st = style_of("p { font-variant-numeric: tabular-nums }");
    assert!(st.get("font-feature-settings").is_some_and(|v| v.contains("\"tnum\" 1")));
}

#[test]
fn multi_keywords_concat() {
    let st = style_of("p { font-variant-numeric: oldstyle-nums tabular-nums ordinal }");
    let ffs = st.get("font-feature-settings").unwrap_or_default();
    for tag in ["\"onum\" 1", "\"tnum\" 1", "\"ordn\" 1"] {
        assert!(ffs.contains(tag), "{tag} attendu dans {ffs:?}");
    }
}
