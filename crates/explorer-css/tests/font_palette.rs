// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! R-239 — `@font-palette-values --nom {}` : palette
//! enregistrée, `font-palette: --nom` résolu en base-palette.

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
fn palette_nom_resolue_en_base() {
    let st = style_of(
        "@font-palette-values --custom { font-family: \"Foo\"; base-palette: 2;\
         override-colors: 0 red, 3 blue }\
         p { font-family: \"Foo\"; font-palette: --custom }",
    );
    assert_eq!(st.get("font-palette"), Some("2"));
}

#[test]
fn palette_absente_normal() {
    let st = style_of(
        "@font-palette-values --p { font-family: \"Foo\" }\
         p { font-palette: --p }",
    );
    assert_eq!(st.get("font-palette"), Some("normal"));
}

#[test]
fn registre_peuple() {
    let mut eng = StyleEngine::new();
    eng.add_css(
        "@font-palette-values --p { font-family: \"Foo\"; base-palette: 1; override-colors: 0 red }",
        Origin::Author,
    );
    let fp = eng.font_palettes.get("--p").expect("palette");
    assert_eq!(fp.font_family, "Foo");
    assert_eq!(fp.base_palette, "1");
    assert_eq!(fp.override_colors, vec![(0, "red".to_string())]);
}
