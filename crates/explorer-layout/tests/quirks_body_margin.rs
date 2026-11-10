// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! R-210 — quirk « marges absorbées au bord de `body` »
//! (HTML Standard §quirks-rendering, mesuré Chrome quirks :
//! `<body><h1>` sans doctype → h1 au bord de contenu de
//! body — la chaîne de marges qui traverse body est avalée).

mod common;

use explorer_css::{Origin, StyleEngine, StyleEnv};
use explorer_dom::{Dom, NodeId};
use explorer_html::TreeBuilder;
use explorer_layout::{build_box_tree, layout_tree, BlockCtx, LayoutBox};

const VIEW: f32 = 800.0;

fn laid(html: &str, css: &str) -> (Dom, LayoutBox) {
    let dom = TreeBuilder::parse(html);
    let mut eng = StyleEngine::new();
    eng.add_css(css, Origin::Author);
    let styles = eng.compute_tree(&dom, dom.document(), &StyleEnv::default());
    let root_el = dom
        .children(dom.document())
        .into_iter()
        .flatten()
        .find(|&c| dom.element(c).is_ok())
        .expect("élément racine");
    let mut bx = build_box_tree(&dom, root_el, &styles).expect("arbre");
    layout_tree(&mut bx, VIEW, &BlockCtx::default());
    (dom, bx)
}

fn by_tag(dom: &Dom, tag: &str) -> NodeId {
    let mut queue = vec![dom.document()];
    while let Some(n) = queue.pop() {
        if dom.tag_name(n).is_ok_and(|t| t == tag) {
            return n;
        }
        if let Ok(ch) = dom.children(n) {
            queue.extend(ch.iter().rev());
        }
    }
    panic!("<{tag}> absent")
}

fn find(b: &LayoutBox, id: NodeId) -> &LayoutBox {
    if b.node == Some(id) {
        return b;
    }
    for c in &b.children {
        let mut stack = vec![c];
        while let Some(x) = stack.pop() {
            if x.node == Some(id) {
                return x;
            }
            stack.extend(&x.children);
        }
    }
    panic!("introuvable")
}

#[test]
fn quirks_marge_premier_fils_avalee_au_bord_body() {
    // Sans doctype → quirks. Mesuré Chrome : h1.y == 8
    // (bord de contenu de body, marge avalée) ; p garde le
    // collapse entre frères : 8 + 37 + 21,44 ≈ 66,4.
    let (dom, bx) = laid("<body><h1>H</h1><p>P", "");
    assert!(bx.quirks_mode, "fixture devrait être quirks");
    let body = find(&bx, by_tag(&dom, "body"));
    let h1 = find(&bx, by_tag(&dom, "h1"));
    assert!(
        (h1.rect.y - body.rect.y).abs() < 1.0,
        "h1 devrait coller au bord de contenu de body : body={} h1={}",
        body.rect.y,
        h1.rect.y
    );
    let p = find(&bx, by_tag(&dom, "p"));
    // Collapse h1↔p inchangé (marge basse h1 ≥ marge haute p).
    assert!(
        p.rect.y > h1.rect.y + h1.rect.height,
        "collapse frères cassé"
    );
}

#[test]
fn quirks_collapse_a_travers_vide_jusqu_a_body() {
    // Chaîne transparente : l'élément vide ne casse pas la
    // quirk — la marge remonte jusqu'à body et s'y avalée.
    let (dom, bx) = laid("<body><div></div><h1>H</h1>", "");
    let h1 = find(&bx, by_tag(&dom, "h1"));
    let body = find(&bx, by_tag(&dom, "body"));
    assert!(
        (h1.rect.y - body.rect.y).abs() < 1.0,
        "h1 à travers div vide : body={} h1={}",
        body.rect.y,
        h1.rect.y
    );
}

#[test]
fn standards_inchanges_marge_protrude() {
    // Garde-fou : même page avec doctype → comportement
    // standard conservé (marge h1 protrude au-dessus de body).
    let (dom, bx) = laid("<!doctype html><body><h1>H</h1>", "");
    let h1 = find(&bx, by_tag(&dom, "h1"));
    assert!(
        h1.rect.y > 12.0,
        "h1 sans marge en standards : y={}",
        h1.rect.y
    );
}
