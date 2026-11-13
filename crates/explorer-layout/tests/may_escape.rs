// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Tests R-293 : `may_escape` — drapeau « l'encre du
//! sous-arbre peut sortir du `rect` » calculé post-ordre en
//! fin de `layout_tree`/`relayout_subtree`, consommé par la
//! culle de bande de la voie incrémentale côté render.
use std::collections::HashMap;

use explorer_css::{Origin, StyleEngine, StyleEnv, StyleMap};
use explorer_dom::{Dom, NodeId};
use explorer_html::TreeBuilder;
use explorer_layout::font::{FontRequest, FontSource};
use explorer_layout::{
    build_box_tree_sized, layout_tree, relayout_subtree, BlockCtx, LayoutBox, Relayout,
};

struct Fixture(Vec<u8>);
impl FontSource for Fixture {
    fn font_data(&self, _req: &FontRequest) -> Option<Vec<u8>> {
        Some(self.0.clone())
    }
}

fn ctx() -> BlockCtx<'static> {
    let data = explorer_test_support::fixture_ttf();
    let src: &'static Fixture = Box::leak(Box::new(Fixture(data)));
    BlockCtx {
        fonts: Some(src),
        ..BlockCtx::default()
    }
}

fn setup(html: &str, css: &str) -> (Dom, StyleMap, LayoutBox) {
    let dom = TreeBuilder::parse(html);
    let mut eng = StyleEngine::new();
    eng.add_css("body { margin: 0 }", Origin::Author);
    eng.add_css(css, Origin::Author);
    let styles = eng.compute_tree(&dom, dom.document(), &StyleEnv::default());
    let root = dom.document_element().expect("documentElement");
    let mut bx =
        build_box_tree_sized(&dom, root, &styles, &HashMap::new()).expect("arbre de boîtes");
    layout_tree(&mut bx, 400.0, &ctx());
    (dom, styles, bx)
}

fn by_id(dom: &Dom, id: &str) -> NodeId {
    fn walk(dom: &Dom, n: NodeId, id: &str) -> Option<NodeId> {
        if dom.element(n).is_ok() && dom.attribute(n, "id").ok().flatten().as_deref() == Some(id) {
            return Some(n);
        }
        let mut c = dom.first_child(n).ok().flatten();
        while let Some(node) = c {
            if let Some(f) = walk(dom, node, id) {
                return Some(f);
            }
            c = dom.next_sibling(node).ok().flatten();
        }
        None
    }
    walk(dom, dom.document(), id).expect("id présent")
}

fn find(b: &LayoutBox, id: NodeId) -> &LayoutBox {
    fn t(b: &LayoutBox, id: NodeId) -> Option<&LayoutBox> {
        if b.node == Some(id) {
            return Some(b);
        }
        b.children.iter().find_map(|c| t(c, id))
    }
    t(b, id).expect("boîte présente")
}

/// Flux normal statique et contenu : rien ne s'échappe —
/// les sous-arbres sont éligibles à la culle de bande.
#[test]
fn statique_contenu_ne_s_echappe_pas() {
    let (dom, _s, bx) = setup(
        "<div id=a><div id=b><p id=c>texte</p></div></div>",
        "div, p { overflow: hidden } #a { width: 300px } #c { width: 100px }",
    );
    assert!(!find(&bx, by_id(&dom, "c")).may_escape);
    assert!(!find(&bx, by_id(&dom, "b")).may_escape);
    assert!(!find(&bx, by_id(&dom, "a")).may_escape);
}

/// Un descendant positionné s'échappe : tous ses ancêtres
/// portent `may_escape` — la descente d'émission doit les
/// traverser pour atteindre le report `pos_defer`.
#[test]
fn positionne_marque_tous_ses_ancetres() {
    let (dom, _s, bx) = setup(
        "<div id=a><div id=b><div id=c><div id=d></div></div></div></div>",
        "#d { position: absolute; top: 0; left: 0; width: 10px; height: 10px }",
    );
    for id in ["a", "b", "c", "d"] {
        assert!(find(&bx, by_id(&dom, id)).may_escape, "{id}");
    }
}

/// `transform`, `float` et `position:fixed` marquent
/// chacun la boîte (et donc la chaîne d'ancêtres).
#[test]
fn transform_flottant_fixe_echappent() {
    let (dom, _s, bx) = setup(
        "<div id=t></div><div id=f></div><div id=x></div>",
        "#t { transform: translateX(1px); width: 10px; height: 10px } \
         #f { float: left; width: 10px; height: 10px } \
         #x { position: fixed; width: 10px; height: 10px }",
    );
    assert!(find(&bx, by_id(&dom, "t")).may_escape);
    assert!(find(&bx, by_id(&dom, "f")).may_escape);
    assert!(find(&bx, by_id(&dom, "x")).may_escape);
}

/// Un enfant in-flow dont le `rect` déborde un axe
/// `overflow:visible` fait échapper le parent ; sous
/// `overflow:hidden`, l'encre est bornée par le clip — le
/// parent reste cullable.
#[test]
fn debord_visible_echappe_mas_hidden_non() {
    let (dom, _s, bx) = setup(
        "<div id=v><div id=w></div></div><div id=h><div id=z></div></div>",
        "#v, #h { width: 100px; height: 10px } #w, #z { width: 500px; height: 5px } \
         #v { overflow: visible } #h { overflow: hidden }",
    );
    assert!(find(&bx, by_id(&dom, "v")).may_escape);
    assert!(!find(&bx, by_id(&dom, "h")).may_escape);
    // L'enfant débordant lui-même n'est pas une échappée :
    // son encre reste dans son propre rect.
    assert!(!find(&bx, by_id(&dom, "z")).may_escape);
}

/// Une ligne `nowrap` plus large que la boîte sous
/// `overflow:visible` peut déposer de l'encre à droite du
/// `rect` — échappée.
#[test]
fn nowrap_echappe() {
    let (dom, _s, bx) = setup(
        "<div id=n>aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa</div>",
        "#n { width: 50px; white-space: nowrap; overflow: visible }",
    );
    // La boîte #n porte le drapeau via la garde
    // nowrap+visible, et la propagation marque la chaîne.
    assert!(find(&bx, by_id(&dom, "n")).may_escape);
    assert!(bx.may_escape);
}

/// Le relayout incrémental recalcule le drapeau : un
/// `class` ajoutant `position:absolute` doit retourner
/// l'ancêtre à `may_escape = true` après `relayout_subtree`.
#[test]
fn relayout_recalcule_le_drapeau() {
    let (mut dom, _s, mut bx) = setup(
        "<div id=wrap><div id=t></div></div>",
        "#t { width: 10px; height: 10px } .pos { position: absolute; top: 0; left: 0 }",
    );
    let t = by_id(&dom, "t");
    let wrap = by_id(&dom, "wrap");
    assert!(!find(&bx, wrap).may_escape);
    dom.set_attribute(t, "class", "pos").ok();
    let mut eng = StyleEngine::new();
    eng.add_css("body { margin: 0 }", Origin::Author);
    eng.add_css(
        "#t { width: 10px; height: 10px } .pos { position: absolute; top: 0; left: 0 }",
        Origin::Author,
    );
    let styles = eng.compute_tree(&dom, dom.document(), &StyleEnv::default());
    let r = relayout_subtree(&mut bx, &dom, &styles, &HashMap::new(), t, &ctx());
    assert!(matches!(r, Relayout::Done), "{r:?}");
    assert!(find(&bx, t).may_escape);
    assert!(find(&bx, wrap).may_escape);
}
