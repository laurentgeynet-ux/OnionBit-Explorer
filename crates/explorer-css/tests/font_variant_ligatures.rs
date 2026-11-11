// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! `font-variant` shorthand → longhands (css-fonts-4 §6.7) +
//! `font-variant-ligatures` → tags OT (`liga`/`clig`/`dlig`/
//! `hlig`/`calt`, §6.4) dans `font-feature-settings`.

use explorer_css::{Origin, StyleEngine, StyleEnv};
use explorer_dom::Dom;
use explorer_html::TreeBuilder;

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
fn ligatures_communes_produisent_liga_clig() {
    let st = style_of("p { font-variant-ligatures: common-ligatures contextual }");
    let ffs = st.get("font-feature-settings").unwrap_or_default();
    for tag in ["\"liga\" 1", "\"clig\" 1", "\"calt\" 1"] {
        assert!(ffs.contains(tag), "{tag} attendu dans {ffs:?}");
    }
}

#[test]
fn no_common_coupe_les_features() {
    let st = style_of("p { font-variant-ligatures: no-common-ligatures }");
    let ffs = st.get("font-feature-settings").unwrap_or_default();
    for tag in ["\"liga\" 0", "\"clig\" 0"] {
        assert!(ffs.contains(tag), "{tag} attendu dans {ffs:?}");
    }
}

#[test]
fn none_coupe_tout() {
    let st = style_of("p { font-variant-ligatures: none }");
    let ffs = st.get("font-feature-settings").unwrap_or_default();
    for tag in ["\"liga\" 0", "\"clig\" 0", "\"dlig\" 0", "\"hlig\" 0"] {
        assert!(ffs.contains(tag), "{tag} attendu dans {ffs:?}");
    }
}

#[test]
fn shorthand_distribue_les_longhands() {
    let st = style_of("p { font-variant: small-caps tabular-nums jis04 }");
    assert_eq!(
        st.get("font-variant-caps").map(str::trim),
        Some("small-caps"),
        "caps routé vers font-variant-caps"
    );
    let ffs = st.get("font-feature-settings").unwrap_or_default();
    for tag in ["\"tnum\" 1", "\"jp04\" 1"] {
        assert!(ffs.contains(tag), "{tag} attendu dans {ffs:?}");
    }
}

#[test]
fn shorthand_position_et_ligatures() {
    let st = style_of("p { font-variant: super no-common-ligatures }");
    assert_eq!(
        st.get("font-variant-position").map(str::trim),
        Some("super")
    );
    let ffs = st.get("font-feature-settings").unwrap_or_default();
    assert!(ffs.contains("\"liga\" 0"), "liga coupé dans {ffs:?}");
}

#[test]
fn shorthand_none_coupe_les_ligatures() {
    let st = style_of("p { font-variant: none }");
    let ffs = st.get("font-feature-settings").unwrap_or_default();
    assert!(ffs.contains("\"liga\" 0"), "liga coupé dans {ffs:?}");
}
