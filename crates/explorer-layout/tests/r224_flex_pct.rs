// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! R-224 — `width:100%` dans un flex item à basis contenu :
//! Chrome résout le 100 % au contenu intrinsèque (le parent
//! shrink-wrap = max-content du descendant), nous l'étirons
//! au conteneur — mesuré wikipedia (`vector-collapsible`
//! 1088 vs 297 → menus abs `right:0` décalés dx +1032 px).
//! FIXTURE minimale reproduite (`quirk_srv/m.html`) :
//! `coll` 1082 eng vs 208 ref.

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
    if b.node
        .and_then(|n| dom.attribute_str(n, "id").ok().flatten())
        == Some(id)
    {
        return Some(b);
    }
    b.children.iter().find_map(|c| by_id(c, id, dom))
}

#[test]

fn flex_item_pct_child_shrink_wrap_au_contenu() {
    let (dom, bx) = laid(
        "<!doctype html><body><div id=hend><div id=start>L</div>\
         <div id=coll><nav><div id=m><div id=mc><ul id=ul>\
         <li>Read<li>View source<li>History</ul></div></div></nav></div></div>",
        "#hend{display:flex;width:1082px;margin-left:44px}#start{width:180px}\
         #mc{width:100%}#ul{display:flex;justify-content:end;list-style:none;margin:0;padding:0}\
         li{padding:0 8px;white-space:nowrap}",
    );
    let coll = by_id(&bx, "coll", &dom).expect("collapsible");
    // Chrome : ~208 (somme des li). Nous : 1082 (étiré au conteneur).
    assert!(
        coll.rect.width < 400.0,
        "width:100% dans shrink-wrap doit donner le contenu (~208), pas le conteneur : w={}",
        coll.rect.width
    );
}
