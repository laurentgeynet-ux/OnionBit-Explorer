// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! R-231 — `@custom-media --nom (q)` : alias de media query,
//! `@media (--nom)` substitué à la collecte.

use explorer_css::{Origin, StyleEngine, StyleEnv};
use explorer_dom::Dom;
use explorer_html::TreeBuilder;

fn walk_x(dom: &Dom, n: explorer_dom::NodeId, sm: &explorer_css::StyleMap, out: &mut String) {
    if dom
        .attribute_str(n, "class")
        .ok()
        .flatten()
        .is_some_and(|c| c == "x")
    {
        *out = sm
            .get(n)
            .and_then(|s| s.get("display").map(str::to_owned))
            .unwrap_or_default();
    }
    if let Ok(ch) = dom.children(n) {
        for c in ch {
            walk_x(dom, c, sm, out);
        }
    }
}

fn display_of(html: &str, css: &str, env: &StyleEnv) -> String {
    let dom = TreeBuilder::parse(html);
    let mut eng = StyleEngine::new();
    eng.add_css(css, Origin::Author);
    let sm = eng.compute_tree(&dom, dom.document(), env);
    let mut out = String::new();
    walk_x(&dom, dom.document(), &sm, &mut out);
    out
}

#[test]
fn custom_media_alias_resolu() {
    let css = "@custom-media --narrow (max-width:640px);\
        @media (--narrow) { p { display:none } }";
    let st = display_of(
        "<body><p class=x>ok</p></body>",
        css,
        &StyleEnv {
            width_px: 1366,
            height_px: 900,
            ..StyleEnv::default()
        },
    );
    assert!(
        st != "none",
        "viewport 1366 > 640 → la règle ne doit pas masquer (st={st:?})"
    );
    let st = display_of(
        "<body><p class=x>ok</p></body>",
        css,
        &StyleEnv {
            width_px: 500,
            height_px: 900,
            ..StyleEnv::default()
        },
    );
    assert_eq!(st, "none", "viewport 500 < 640 → masqué via l'alias");
}
