// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! R-249 — `text-wrap`/`text-wrap-mode`/`text-wrap-style` :
//! shorthand expand. R-264 : `text-wrap-mode` est désormais
//! un vrai longhand consommé au point d'usage (`ws_mode` dans
//! le line-breaker) — il n'est plus **projeté** dans
//! `white-space` (la projection écrasait `white-space:pre`).

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
fn shorthand_mode() {
    let st = style_of("p { text-wrap: nowrap }");
    assert_eq!(st.get("text-wrap-mode"), Some("nowrap"));
}

#[test]
fn shorthand_style() {
    let st = style_of("p { text-wrap: wrap balance }");
    assert_eq!(st.get("text-wrap-style"), Some("balance"));
    assert_eq!(st.get("text-wrap-mode"), Some("wrap"));
}

#[test]
fn mode_seul() {
    let st = style_of("p { text-wrap-mode: nowrap }");
    assert_eq!(st.get("text-wrap-mode"), Some("nowrap"));
    // R-264 : plus de projection — `white-space` reste absent.
    assert_eq!(st.get("white-space"), None);
}

#[test]
fn white_space_shorthand_pose_les_longhands() {
    // R-264 — `white-space` = shorthand de
    // `white-space-collapse` + `text-wrap-mode`
    // (+ `white-space-trim:none`) — css-text-4 §7.1.
    let st = style_of("p { white-space: pre }");
    assert_eq!(st.get("white-space"), Some("pre"));
    assert_eq!(st.get("white-space-collapse"), Some("preserve"));
    assert_eq!(st.get("text-wrap-mode"), Some("nowrap"));
    let st = style_of("p { white-space: break-spaces }");
    assert_eq!(st.get("white-space-collapse"), Some("break-spaces"));
    assert_eq!(st.get("text-wrap-mode"), Some("wrap"));
}

#[test]
fn longhand_apres_le_shorthand_gagne() {
    // Ordre de cascade : le longhand déclaré après le
    // shorthand l'emporte (`pre` + `collapse` → collapse+nowrap).
    let st = style_of("p { white-space: pre; white-space-collapse: collapse }");
    assert_eq!(st.get("white-space-collapse"), Some("collapse"));
    assert_eq!(st.get("text-wrap-mode"), Some("nowrap"));
}

#[test]
fn text_breaking_props_heritent() {
    // R-264 — `word-break`/`overflow-wrap`/`line-break`/
    // `white-space-collapse`/`text-wrap-mode` héritent
    // (css-text-3 §5.x/§6.x + css-text-4 §4/§7).
    let dom = TreeBuilder::parse("<div><p class=x>ok</p></div>");
    let mut eng = StyleEngine::new();
    eng.add_css(
        "div { word-break: break-all; overflow-wrap: anywhere; \
         line-break: anywhere; white-space-collapse: discard; \
         text-wrap-mode: nowrap }",
        Origin::Author,
    );
    let sm = eng.compute_tree(&dom, dom.document(), &StyleEnv::default());
    let mut out = None;
    walk_x(&dom, dom.document(), &sm, &mut out);
    let st = out.expect("élément");
    assert_eq!(st.get("word-break"), Some("break-all"));
    assert_eq!(st.get("overflow-wrap"), Some("anywhere"));
    assert_eq!(st.get("line-break"), Some("anywhere"));
    assert_eq!(st.get("white-space-collapse"), Some("discard"));
    assert_eq!(st.get("text-wrap-mode"), Some("nowrap"));
}
