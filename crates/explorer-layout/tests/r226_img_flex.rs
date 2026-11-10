// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! R-226 — élément remplacé (`<img>`) en flex item : sa
//! largeur s'effondre à 0 alors que `style="width:140px"`
//! est explicite (mesuré : `r.html` `quirk_srv` — Chrome
//! 140, nous 0). Même collapse pour l'img *contenu* d'un
//! item (`q.html`). Consomme la dernière moitié du
//! `dx +1032` wikipedia (logo).

use explorer_css::{Origin, StyleEngine, StyleEnv};
use explorer_dom::Dom;
use explorer_html::TreeBuilder;
use explorer_layout::{build_box_tree, layout_tree, BlockCtx, LayoutBox};

fn laid(html: &str, css: &str) -> (Dom, LayoutBox) {
    let dom = TreeBuilder::parse(html);
    let mut eng = StyleEngine::new();
    eng.add_css(css, Origin::Author);
    let styles = eng.compute_tree(&dom, dom.document(), &StyleEnv::default());
    let root = dom
        .children(dom.document())
        .ok()
        .and_then(|ch| ch.into_iter().find(|&c| dom.element(c).is_ok()))
        .unwrap_or(dom.document());
    let mut bx = build_box_tree(&dom, root, &styles).unwrap();
    layout_tree(&mut bx, 1366.0, &BlockCtx::default());
    (dom, bx)
}

fn by_id<'a>(b: &'a LayoutBox, id: &str, dom: &Dom) -> Option<&'a LayoutBox> {
    if b.node.and_then(|n| dom.attribute_str(n, "id").ok().flatten()) == Some(id) {
        return Some(b);
    }
    b.children.iter().find_map(|c| by_id(c, id, dom))
}

#[test]

fn img_flex_item_garde_sa_largeur_explicite() {
    let (dom, bx) = laid(
        "<!doctype html><body><a id=a><img id=i1 src=\"/i.png\" \
         style=\"width:140px;height:20px;display:block\"></a>",
        "#a{display:flex}",
    );
    let img = by_id(&bx, "i1", &dom).expect("img flex item");
    assert!(
        img.rect.width > 100.0,
        "img flex item doit garder width:140px, pas 0 : w={}",
        img.rect.width
    );
}
