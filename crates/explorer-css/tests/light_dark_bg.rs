// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! R-219 — `background: light-dark(#eee,#222)` : la fonction
//! couleur à résolution retardée passait comme token
//! inconnu dans le shorthand → déclaration entière droppée
//! (mesuré `css.value.dropped.background` sur example.com,
//! fond perdu → page sans fond).

use explorer_css::{Origin, StyleEngine, StyleEnv};
use explorer_dom::Dom;
use explorer_html::TreeBuilder;

fn computed(html: &str, css: &str, tag: &str) -> explorer_css::ComputedStyle {
    let dom: Dom = TreeBuilder::parse(html);
    let mut eng = StyleEngine::new();
    eng.add_css(css, Origin::Author);
    let node = {
        let mut q = vec![dom.document()];
        let mut out = None;
        while let Some(n) = q.pop() {
            if dom.tag_name(n).is_ok_and(|t| t == tag) {
                out = Some(n);
                break;
            }
            if let Ok(c) = dom.children(n) {
                q.extend(c.iter().rev());
            }
        }
        out.unwrap()
    };
    eng.compute(&dom, node, &StyleEnv::default())
}

#[test]
fn background_light_dark_survit_et_resout() {
    // Le shorthand accepte la fonction ; la résolution
    // post-cascade (R-105) la choisit selon color-scheme —
    // schéma clair par défaut → `#eee`.
    let st = computed(
        "<html><body></body></html>",
        "body{background:light-dark(#eee,#222)}",
        "body",
    );
    let bg = st.get("background-color").unwrap_or_default();
    assert!(
        bg.contains("#eee"),
        "light-dark doit résoudre vers #eee, obtenu {bg}"
    );
}

#[test]
fn background_var_passe_aussi() {
    let st = computed(
        "<html><body></body></html>",
        "body{background:var(--bg, #fff)}",
        "body",
    );
    let bg = st.get("background-color").unwrap_or_default();
    assert!(
        bg.contains("var(") || bg.contains("#fff"),
        "var() doit passer ou résoudre, obtenu {bg}"
    );
}
