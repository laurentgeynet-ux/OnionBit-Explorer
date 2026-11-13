// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! R-229 — `@font-feature-values` : valeurs nommées
//! résolues par `font-variant-alternates` en réglages
//! `font-feature-settings` équivalents (css-fonts-4 §6).

use explorer_css::{Origin, StyleEngine, StyleEnv};
use explorer_dom::Dom;
use explorer_html::TreeBuilder;

fn walk_cls(
    dom: &Dom,
    n: explorer_dom::NodeId,
    cls: &str,
    sm: &explorer_css::StyleMap,
    out: &mut Option<explorer_css::ComputedStyle>,
) {
    if dom
        .attribute_str(n, "class")
        .ok()
        .flatten()
        .is_some_and(|c| c == cls)
    {
        *out = sm.get(n).cloned();
    }
    if let Ok(ch) = dom.children(n) {
        for c in ch {
            walk_cls(dom, c, cls, sm, out);
        }
    }
}

fn style_of(html: &str, css: &str, cls: &str) -> explorer_css::ComputedStyle {
    let dom = TreeBuilder::parse(html);
    let mut eng = StyleEngine::new();
    eng.add_css(css, Origin::Author);
    let sm = eng.compute_tree(&dom, dom.document(), &StyleEnv::default());
    let mut out = None;
    walk_cls(&dom, dom.document(), cls, &sm, &mut out);
    out.expect("élément trouvé")
}

#[test]
fn annotation_nom_resolu_en_nalt() {
    let st = style_of(
        "<body><p class=x>ok</p></body>",
        "@font-feature-values \"Foo\" { @annotation { circled: 1 } }\
         p { font-family: \"Foo\"; font-variant-alternates: annotation(circled) }",
        "x",
    );
    let ffs = st.get("font-feature-settings").unwrap_or_default();
    assert!(
        ffs.contains("\"nalt\" 1"),
        "annotation(circled) doit résoudre \"nalt\" 1, pas {ffs:?}"
    );
}

#[test]
fn styleset_et_swash_multi_indices() {
    let st = style_of(
        "<body><p class=x>ok</p></body>",
        "@font-feature-values \"Bar\" {\
            @styleset { wide: 1 4 7 }\
            @swash { fancy: 2 }\
         }\
         p { font-family: \"Bar\"; font-variant-alternates: styleset(wide) swash(fancy) }",
        "x",
    );
    let ffs = st.get("font-feature-settings").unwrap_or_default();
    for want in ["\"ss01\"", "\"ss04\"", "\"ss07\"", "\"swsh\" 2"] {
        assert!(ffs.contains(want), "{want} attendu dans {ffs:?}");
    }
}

#[test]
fn sans_alternates_rien_nest_touche() {
    let st = style_of(
        "<body><p class=x>ok</p></body>",
        "@font-feature-values \"Foo\" { @annotation { c: 3 } }\
         p { font-family: \"Foo\" }",
        "x",
    );
    assert!(st.get("font-feature-settings").is_none());
}
