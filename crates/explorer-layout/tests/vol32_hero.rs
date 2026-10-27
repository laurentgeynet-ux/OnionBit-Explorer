// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Repro vol 32 : hero notion effondre en colonne etroite
//! (grid-template-columns a pistes var() + lignes nommees,
//! items grid-column: var(--col)).

mod common;

use explorer_css::{Origin, StyleEngine, StyleEnv};
use explorer_dom::{Dom, NodeId};
use explorer_html::TreeBuilder;
use explorer_layout::{build_box_tree_sized, layout_tree, BlockCtx, LayoutBox};

const VIEW: f32 = 1366.0;

fn laid(html: &str, css: &str) -> (Dom, LayoutBox) {
    let dom = TreeBuilder::parse(html);
    let mut eng = StyleEngine::new();
    eng.add_css("body { margin: 0 }", Origin::Author);
    eng.add_css(css, Origin::Author);
    let styles = eng.compute_tree(&dom, dom.document(), &StyleEnv::default());
    let root_el = dom
        .children(dom.document())
        .into_iter()
        .flatten()
        .find(|&c| dom.element(c).is_ok())
        .expect("element racine");
    let mut bx = build_box_tree_sized(&dom, root_el, &styles, &std::collections::HashMap::new())
        .expect("arbre");
    layout_tree(&mut bx, VIEW, &BlockCtx::default());
    (dom, bx)
}

fn by_id(dom: &Dom, id: &str) -> NodeId {
    let mut queue = vec![dom.document()];
    while let Some(n) = queue.pop() {
        if dom
            .attribute(n, "id")
            .is_ok_and(|v| v.as_deref() == Some(id))
        {
            return n;
        }
        if let Ok(ch) = dom.children(n) {
            queue.extend(ch.iter().rev());
        }
    }
    panic!("#{id} absent")
}

fn find(b: &LayoutBox, id: NodeId) -> &LayoutBox {
    if b.node == Some(id) {
        return b;
    }
    b.children.iter().find_map(|c| find_opt(c, id)).expect("boite")
}

fn find_opt(b: &LayoutBox, id: NodeId) -> Option<&LayoutBox> {
    if b.node == Some(id) {
        return Some(b);
    }
    b.children.iter().find_map(|c| find_opt(c, id))
}

fn approx(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.51
}

/// Vol 32 notion : `grid-template-columns` à pistes issues de
/// `var()` profondes — `calc()` contenant `min()`/`calc()`
/// imbriqués. Sans l'évaluation des fonctions math en position
/// de facteur `calc`, `var(--rail)`/`minmax(0,var(--half))`
/// retombaient en pistes `auto` à 0 px : l'item content ne
/// couvrait que les gouttières (36 px au centre) et l'item
/// media 12 px — la signature « héro effondré ~72 px centré »
/// (les marges/paddings réels élargissent un peu la colonne).
#[test]
fn grid_pistes_var_calc_imbrique() {
    let (dom, bx) = laid(
        "<!doctype html><section class=\"heroGrid\">\
         <div class=\"item regionContent\" id=\"c\"><header><h1>Where teams and agents Think together.</h1></header></div>\
         <div class=\"item regionCenter\" id=\"m\">media</div></section>",
        "body { margin: 0 } :root { --tatami-inset: 16px; --tatami-gutter: 12px; \
         --tatami-center-max: 960px; --tatami-page-max: 1252px } \
         .heroGrid { \
         --inset: var(--tatami-inset); \
         --gutter: var(--tatami-gutter); \
         --avail: calc(100% - (2 * var(--inset))); \
         --page: min(var(--tatami-page-max), var(--avail)); \
         --center-area: min(calc(var(--tatami-center-max) + (2 * var(--gutter))), var(--page)); \
         --half: calc((var(--center-area) - (3 * var(--gutter))) / 2); \
         --rail: calc((var(--page) - var(--center-area)) / 2); \
         grid-template-columns: [full-bleed-start] minmax(0, 1fr) [full-start] \
         var(--inset) [content-start] var(--rail) var(--gutter) [center-start] \
         minmax(0, var(--half)) [center-mid-start] var(--gutter) [center-mid-end] \
         minmax(0, var(--half)) [center-end] var(--gutter) var(--rail) \
         [content-end] var(--inset) [full-end] minmax(0, 1fr) [full-bleed-end]; \
         display: grid } \
         .item { grid-column: var(--col, content-start / content-end) } \
         .regionContent { --col: content-start / content-end } \
         .regionCenter { --col: center-start / center-end }",
    );
    let c = find(&bx, by_id(&dom, "c"));
    let m = find(&bx, by_id(&dom, "m"));
    // page = min(1252, 1366-32) = 1252 ; center = min(984, 1252) = 984 ;
    // rail = (1252-984)/2 = 134 ; half = (984-36)/2 = 474.
    // content-start..content-end = pistes 2..8 = 134+12+474+12+474+12+134.
    assert!(approx(c.rect.width, 1252.0), "c.w={}", c.rect.width);
    assert!(approx(c.rect.x, 57.0), "c.x={}", c.rect.x);
    // center-start..center-end = pistes 4..6 = 474+12+474 = 960.
    assert!(approx(m.rect.width, 960.0), "m.w={}", m.rect.width);
    assert!(approx(m.rect.x, 203.0), "m.x={}", m.rect.x);
}

/// Vol 32 todoist : `.details` seul item shrinkable d'un flex
/// row face à `.illustration{flex-shrink:0}` trop large —
/// le shrink doit être planché par la taille min automatique
/// (min-content, §4.5), jamais 0.
#[test]
fn flex_shrink_plancher_min_content() {
    let (dom, bx) = laid(
        "<!doctype html><section><div id=\"d\"><h1>Enfin plus de clarte.</h1></div>\
         <div id=\"i\"><div style=\"width:900px;height:10px\"></div></div></section>",
        "body { margin: 0 } section { display: flex; column-gap: 48px; width: 800px } \
         #i { flex-shrink: 0 }",
    );
    let d = find(&bx, by_id(&dom, "d"));
    let i = find(&bx, by_id(&dom, "i"));
    // Sans plancher : d = max(0, base - |free|) = 0 (free = -548).
    assert!(d.rect.width > 20.0, "d.w={} (>0, plancher min-content)", d.rect.width);
    assert!(approx(i.rect.width, 900.0), "i.w={}", i.rect.width);
    // `min-width:0` explicite autorise l'effondrement (spec §4.5).
    let (dom2, bx2) = laid(
        "<!doctype html><section><div id=\"d\"><h1>Enfin plus de clarte.</h1></div>\
         <div id=\"i\"><div style=\"width:900px;height:10px\"></div></div></section>",
        "body { margin: 0 } section { display: flex; column-gap: 48px; width: 800px } \
         #i { flex-shrink: 0 } #d { min-width: 0 }",
    );
    let d0 = find(&bx2, by_id(&dom2, "d"));
    assert!(approx(d0.rect.width, 0.0), "min-width:0 -> d.w={}", d0.rect.width);
}
