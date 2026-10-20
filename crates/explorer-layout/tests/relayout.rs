// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Tests R-12 : relayout incrémental par sous-arbre sale —
//! reboîtage local, propagation de hauteur, dégradations
//! `NeedsFullLayout`/`NotFound`, équivalence avec un reflow
/// complet.
use std::collections::HashMap;

use explorer_css::{Origin, StyleEngine, StyleEnv, StyleMap};
use explorer_dom::{Dom, NodeId};
use explorer_html::TreeBuilder;
use explorer_layout::font::{FontRequest, FontSource};
use explorer_layout::{
    build_box_tree_sized, layout_tree, relayout_subtree, BlockCtx, LayoutBox, Rect, Relayout,
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

/// Dom + styles + arbre posé, clés `id` → `NodeId` résolues.
fn setup(html: &str, css: &str) -> (Dom, StyleMap, LayoutBox) {
    let dom = TreeBuilder::parse(html);
    let mut eng = StyleEngine::new();
    // R-32 : marge UA `body { margin: 8px }` neutralisée —
    // fixtures écrites pour une géométrie à marge nulle.
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
    if b.node == Some(id) {
        return b;
    }
    for c in &b.children {
        if let Some(f) = try_find(c, id) {
            return f;
        }
    }
    panic!("nœud {id:?} absent de l'arbre")
}

fn try_find(b: &LayoutBox, id: NodeId) -> Option<&LayoutBox> {
    if b.node == Some(id) {
        return Some(b);
    }
    b.children.iter().find_map(|c| try_find(c, id))
}

/// Positions (x, y, w, h) de toutes les boîtes porteuses d'un
/// nœud — comparaison relayout vs reflow complet.
fn positions(bx: &LayoutBox, out: &mut Vec<(NodeId, Rect)>) {
    if let Some(n) = bx.node {
        out.push((n, bx.rect));
    }
    for c in &bx.children {
        positions(c, out);
    }
}

#[test]
fn texte_mute_relayout_done_et_identique_au_reflow() {
    let (mut dom, _styles, mut bx) = setup(
        "<html><body><div id=a><p>xxxx yyyy</p></div><div id=b><p>bas</p></div></body></html>",
        "div { width: 200px }",
    );
    let a = by_id(&dom, "a");
    // Mute le texte de <p> dans #a.
    let p = dom.first_child(a).ok().flatten().expect("p");
    let txt = dom.first_child(p).ok().flatten().expect("texte");
    dom.set_text(txt, "zzzz wwww qqqq").expect("set_text");
    let styles2 = {
        let mut eng = StyleEngine::new();
        // Même neutralisation que `setup` — sinon le reflow
        // de comparaison appliquerait la marge UA `body`.
        eng.add_css("body { margin: 0 }", Origin::Author);
        eng.add_css("div { width: 200px }", Origin::Author);
        eng.compute_tree(&dom, dom.document(), &StyleEnv::default())
    };
    let r = relayout_subtree(&mut bx, &dom, &styles2, &HashMap::new(), a, &ctx());
    assert_eq!(r, Relayout::Done);
    // Équivalence : même arbre qu'un rebuild + layout complet.
    let root = dom.document_element().expect("el");
    let mut fresh = build_box_tree_sized(&dom, root, &styles2, &HashMap::new()).expect("arbre");
    layout_tree(&mut fresh, 400.0, &ctx());
    let mut got = Vec::new();
    let mut want = Vec::new();
    positions(&bx, &mut got);
    positions(&fresh, &mut want);
    assert_eq!(got, want, "rects identiques au reflow complet");
}

#[test]
fn croissance_propagee_aux_ancetres_et_freres() {
    let (mut dom, styles, mut bx) = setup(
        "<html><body><div id=a><p id=t>court</p></div><div id=b>suite</div></body></html>",
        "p { margin: 0 }",
    );
    let b = by_id(&dom, "b");
    let y_b_avant = find(&bx, b).rect.y;
    let a = by_id(&dom, "a");
    let t = by_id(&dom, "t");
    let txt = dom.first_child(t).ok().flatten().expect("texte");
    // Beaucoup plus de texte → <p> plus haut → #a grandit → #b descend.
    let long = "mot ".repeat(80);
    dom.set_text(txt, &long).expect("set_text");
    let r = relayout_subtree(&mut bx, &dom, &styles, &HashMap::new(), a, &ctx());
    assert_eq!(r, Relayout::Done);
    assert!(
        find(&bx, b).rect.y > y_b_avant,
        "le frère suivant descend après croissance"
    );
}

#[test]
fn cible_absente_rend_not_found() {
    let (mut dom, styles, mut bx) = setup("<html><body><p>x</p></body></html>", "");
    // Élément créé mais jamais inséré : absent de l'arbre.
    let ghost = dom.create_element("section").expect("el");
    let r = relayout_subtree(&mut bx, &dom, &styles, &HashMap::new(), ghost, &ctx());
    assert_eq!(r, Relayout::NotFound);
}

#[test]
fn flottant_dans_le_document_impose_needs_full() {
    let (dom, styles, mut bx) = setup(
        "<html><body><img style='float:left'><div id=a><p>txt</p></div></body></html>",
        "",
    );
    let a = by_id(&dom, "a");
    let r = relayout_subtree(&mut bx, &dom, &styles, &HashMap::new(), a, &ctx());
    assert_eq!(r, Relayout::NeedsFullLayout);
}

#[test]
fn chemin_inline_ou_absolu_impose_needs_full() {
    // Cible posée en absolute → son rejeu local ne peut pas
    // recalculer sa position (elle dépend du CB posé).
    let (dom, styles, mut bx) = setup(
        "<html><body><div id=a style='position:absolute;left:10px'><p>x</p></div></body></html>",
        "",
    );
    let a = by_id(&dom, "a");
    let r = relayout_subtree(&mut bx, &dom, &styles, &HashMap::new(), a, &ctx());
    assert_eq!(r, Relayout::NeedsFullLayout);
}

/// R-33 : un fils **non vide** qui grandit décale ses frères
/// par translation au lieu de re-poser tout le parent — le
/// résultat doit rester identique au reflow complet (à un
/// epsilon d'accumulation flottante près).
#[test]
fn croissance_non_vide_translate_les_freres() {
    let (mut dom, styles, mut bx) = setup(
        "<html><body><div id=a><p id=t>court</p></div>\
         <p id=b>un</p><p id=c>deux</p></body></html>",
        "",
    );
    let a = by_id(&dom, "a");
    let t = by_id(&dom, "t");
    let txt = dom.first_child(t).ok().flatten().expect("texte");
    dom.set_text(txt, &"mot ".repeat(80)).expect("set_text");
    let r = relayout_subtree(&mut bx, &dom, &styles, &HashMap::new(), a, &ctx());
    assert_eq!(r, Relayout::Done);
    let root = dom.document_element().expect("el");
    let mut fresh = build_box_tree_sized(&dom, root, &styles, &HashMap::new()).expect("arbre");
    layout_tree(&mut fresh, 400.0, &ctx());
    let mut got = Vec::new();
    let mut want = Vec::new();
    positions(&bx, &mut got);
    positions(&fresh, &mut want);
    assert_eq!(got.len(), want.len());
    for ((ng, rg), (nw, rw)) in got.iter().zip(&want) {
        assert_eq!(ng, nw);
        let eps = 0.01;
        assert!(
            (rg.x - rw.x).abs() < eps
                && (rg.y - rw.y).abs() < eps
                && (rg.width - rw.width).abs() < eps
                && (rg.height - rw.height).abs() < eps,
            "{ng:?} : {rg:?} vs {rw:?}"
        );
    }
}

/// R-33 : la transition **vide ↔ non vide** change la chaîne
/// de margins collapsing (§8.3.1) — repli `relay`, qui doit
/// aussi produire l'arbre du reflow complet.
#[test]
fn vide_vers_non_vide_relaye_et_reste_equivalent() {
    let (mut dom, styles, mut bx) = setup(
        "<html><body><div id=a></div><p id=b>apres</p></body></html>",
        "",
    );
    let a = by_id(&dom, "a");
    let p = dom.create_element("p").expect("p");
    let t = dom.create_text("contenu ajoute").expect("texte");
    dom.append_child(p, t).expect("append");
    dom.append_child(a, p).expect("append");
    let r = relayout_subtree(&mut bx, &dom, &styles, &HashMap::new(), a, &ctx());
    assert_eq!(r, Relayout::Done);
    let root = dom.document_element().expect("el");
    let mut fresh = build_box_tree_sized(&dom, root, &styles, &HashMap::new()).expect("arbre");
    layout_tree(&mut fresh, 400.0, &ctx());
    let mut got = Vec::new();
    let mut want = Vec::new();
    positions(&bx, &mut got);
    positions(&fresh, &mut want);
    assert_eq!(got, want, "relay vide→plein identique au reflow");
}

#[test]
fn texte_mute_via_noeud_texte_normalise_a_lelement() {
    let (mut dom, styles, mut bx) =
        setup("<html><body><div id=a><p>avant</p></div></body></html>", "");
    let a = by_id(&dom, "a");
    let p = dom.first_child(a).ok().flatten().expect("p");
    let txt = dom.first_child(p).ok().flatten().expect("texte");
    dom.set_text(txt, "après mutation").expect("set_text");
    // On passe le nœud TEXTE — doit se normaliser à l'élément.
    let r = relayout_subtree(&mut bx, &dom, &styles, &HashMap::new(), txt, &ctx());
    assert_eq!(r, Relayout::Done);
}
