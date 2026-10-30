// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Verrous §B-36 — SVG déplacé hors de son bouton parent
//! (wttj `locale-dropdown-trigger` vol 57/88/102).
mod common;

use explorer_css::{Origin, StyleEngine, StyleEnv};
use explorer_dom::Dom;
use explorer_html::TreeBuilder;
use explorer_layout::{build_box_tree, layout_tree, BlockCtx, LayoutBox};

const VIEW: f32 = 1366.0;

fn laid(html: &str, css: &str) -> (Dom, LayoutBox) {
    let dom = TreeBuilder::parse(html);
    let mut eng = StyleEngine::new();
    eng.add_css("body { margin: 0 }", Origin::Author);
    eng.add_css(css, Origin::Author);
    let styles = eng.compute_tree(
        &dom,
        dom.document(),
        &StyleEnv {
            width_px: 1366,
            height_px: 900,
            ..StyleEnv::default()
        },
    );
    let root_el = dom
        .children(dom.document())
        .into_iter()
        .flatten()
        .find(|&c| dom.element(c).is_ok())
        .expect("racine");
    let mut bx = build_box_tree(&dom, root_el, &styles).expect("arbre");
    layout_tree(&mut bx, VIEW, &BlockCtx::default());
    (dom, bx)
}

fn find_svg<'a>(b: &'a LayoutBox, dom: &'a Dom) -> &'a LayoutBox {
    find_svg_opt(b, dom).expect("svg")
}

fn find_svg_opt<'a>(b: &'a LayoutBox, dom: &'a Dom) -> Option<&'a LayoutBox> {
    if b.node
        .is_some_and(|n| dom.tag_name(n).is_ok_and(|t| t == "svg"))
    {
        return Some(b);
    }
    b.children.iter().find_map(|c| find_svg_opt(c, dom))
}

#[test]
fn svg_flex_min_width_auto_ne_depasse_pas_width_specifie() {
    // §B-36 (wttj vols 57/88/102) : icône `<svg>` à
    // `width:16px;height:16px` dans un bouton flex —
    // l'intrinsèque 300×150 fait que `min-content` mesure
    // `h × iw/ih` = 32 ; la suggestion de taille SPÉCIFIÉE
    // (16) doit plafonner le plancher `min-width:auto`
    // (flexbox §9.9.2). Avant le fix : item utilisé 32 px et
    // débordement `justify-content:center` → svg peint à
    // gauche du bouton.
    let (dom, bx) = laid(
        r#"<button><svg></svg><div id="w"></div></button>"#,
        "button{display:flex;justify-content:center;width:120px}\
         svg{width:16px;height:16px;display:block}#w{width:200px}",
    );
    let svg = find_svg(&bx, &dom);
    assert!(
        (svg.rect.width - 16.0).abs() < 0.5,
        "svg min-width:auto planché à 16 (spec §9.9.2), obtenu {}",
        svg.rect.width
    );
}

#[test]
fn svg_flex_min_width_auto_sans_width_garde_contenu() {
    // Contrôle : `width:auto` — le plancher reste la mesure
    // de contenu (ratio intrinsèque), pas 0.
    let (dom, bx) = laid(
        r#"<button><svg></svg><div id="w"></div></button>"#,
        "button{display:flex;justify-content:center;width:120px}\
         svg{height:16px;display:block}#w{width:200px}",
    );
    let svg = find_svg(&bx, &dom);
    assert!(
        svg.rect.width > 16.0,
        "svg width:auto → plancher contenu (h×ratio=32), obtenu {}",
        svg.rect.width
    );
}
