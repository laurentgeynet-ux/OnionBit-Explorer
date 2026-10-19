// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Tests du layout moderne (étape 42) : `calc()`, flexbox,
//! grid — mesures sur l'arbre de boîtes posé.

mod common;

use explorer_css::{Origin, StyleEngine, StyleEnv};
use explorer_dom::{Dom, NodeId};
use explorer_html::TreeBuilder;
use explorer_layout::{build_box_tree, layout_tree, BlockCtx, LayoutBox};

const VIEW: f32 = 800.0;

/// Pipeline complet : html + css → arbre de boîtes posé.
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

fn find_opt(b: &LayoutBox, id: NodeId) -> Option<&LayoutBox> {
    if b.node == Some(id) {
        return Some(b);
    }
    b.children.iter().find_map(|c| find_opt(c, id))
}

fn find(b: &LayoutBox, id: NodeId) -> &LayoutBox {
    find_opt(b, id).expect("boîte du nœud")
}

fn approx(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.51
}

// ── calc() ────────────────────────────────────────────────────

#[test]
fn calc_pourcent_moins_px() {
    let (dom, bx) = laid(
        "<!doctype html><div>x</div>",
        "body { margin: 0 } div { width: calc(100% - 20px) }",
    );
    let d = find(&bx, by_tag(&dom, "div"));
    assert!(approx(d.rect.width, VIEW - 20.0), "{}", d.rect.width);
}

#[test]
fn calc_multiplication_et_imbrication() {
    let (dom, bx) = laid(
        "<!doctype html><div>x</div>",
        "body { margin: 0 } div { width: calc(calc(100px * 2) + 50px / 2) }",
    );
    let d = find(&bx, by_tag(&dom, "div"));
    assert!(approx(d.rect.width, 225.0), "{}", d.rect.width);
}

#[test]
fn calc_invalide_degrade() {
    let (dom, bx) = laid(
        "<!doctype html><div style=\"height: 10px\">x</div>",
        "body { margin: 0 } div { width: calc(1px + 2); height: 10px }",
    );
    let d = find(&bx, by_tag(&dom, "div"));
    // `1px + 2` invalide (dim + nombre nu) → width ignorée →
    // le bloc remplit le conteneur.
    assert!(approx(d.rect.width, VIEW), "{}", d.rect.width);
}

#[test]
fn calc_dans_var() {
    let (dom, bx) = laid(
        "<!doctype html><div>x</div>",
        "body { margin: 0; --w: calc(25% + 10px) } div { width: var(--w) }",
    );
    let d = find(&bx, by_tag(&dom, "div"));
    assert!(approx(d.rect.width, 210.0), "{}", d.rect.width);
}

// ── flex ──────────────────────────────────────────────────────

#[test]
fn flex_grow_partage_l_espace() {
    let (dom, bx) = laid(
        "<!doctype html><section><div id=\"a\">A</div><div id=\"b\">B</div></section>",
        "body { margin: 0 } section { display: flex } section > div { flex: 1 }",
    );
    let section = find(&bx, by_tag(&dom, "section"));
    assert_eq!(section.children.len(), 2);
    let (ia, ib) = (&section.children[0], &section.children[1]);
    assert!(approx(ia.rect.width, VIEW / 2.0), "{}", ia.rect.width);
    assert!(approx(ib.rect.width, VIEW / 2.0));
    assert!(approx(ia.rect.x, 0.0));
    assert!(approx(ib.rect.x, VIEW / 2.0), "{}", ib.rect.x);
}

#[test]
fn flex_justify_center_et_gap() {
    let (dom, bx) = laid(
        "<!doctype html><section><div>A</div><div>B</div></section>",
        "body { margin: 0 } section { display: flex; justify-content: center; gap: 40px } \
         section > div { width: 100px; height: 10px }",
    );
    let section = find(&bx, by_tag(&dom, "section"));
    let (a, b) = (&section.children[0], &section.children[1]);
    // 240 px d'items → lead = (800 - 240) / 2 = 280.
    assert!(approx(a.rect.x, 280.0), "{}", a.rect.x);
    assert!(approx(b.rect.x, 280.0 + 100.0 + 40.0), "{}", b.rect.x);
}

#[test]
fn flex_direction_column() {
    let (dom, bx) = laid(
        "<!doctype html><section><div>A</div><div>B</div></section>",
        "body { margin: 0 } section { display: flex; flex-direction: column; gap: 10px } \
         section > div { height: 20px }",
    );
    let section = find(&bx, by_tag(&dom, "section"));
    let (a, b) = (&section.children[0], &section.children[1]);
    assert!(approx(a.rect.height, 20.0));
    assert!(approx(b.rect.y - a.rect.y, 30.0), "{}", b.rect.y - a.rect.y);
}

#[test]
fn flex_align_center() {
    let (dom, bx) = laid(
        "<!doctype html><section><div>A</div><div id=\"t\">B</div></section>",
        "body { margin: 0 } section { display: flex; align-items: center } \
         section > div { height: 10px; width: 50px } #t { height: 30px }",
    );
    let section = find(&bx, by_tag(&dom, "section"));
    let t = section
        .children
        .iter()
        .find(|c| approx(c.rect.height, 30.0));
    let t = t.expect("item 30px");
    // Ligne cross = 30, item 10px centré → dy = 10.
    let small = section
        .children
        .iter()
        .find(|c| approx(c.rect.height, 10.0))
        .expect("item 10px");
    assert!(approx(small.rect.y - t.rect.y, 10.0), "{}", small.rect.y);
}

// ── grid ──────────────────────────────────────────────────────

#[test]
fn grid_template_columns_fr() {
    let (dom, bx) = laid(
        "<!doctype html><section><div>A</div><div>B</div></section>",
        "body { margin: 0 } section { display: grid; grid-template-columns: 1fr 1fr } \
         section > div { height: 10px }",
    );
    let section = find(&bx, by_tag(&dom, "section"));
    let (a, b) = (&section.children[0], &section.children[1]);
    assert!(approx(a.rect.width, VIEW / 2.0), "{}", a.rect.width);
    assert!(approx(b.rect.x, VIEW / 2.0), "{}", b.rect.x);
}

#[test]
fn grid_fixed_et_gap_et_span() {
    let (dom, bx) = laid(
        "<!doctype html><section><div id=\"w\">W</div><div>N</div></section>",
        "body { margin: 0 } section { display: grid; grid-template-columns: 200px 1fr; \
         column-gap: 20px } #w { grid-column: 1 / span 2; height: 10px } \
         section > div { height: 10px }",
    );
    let section = find(&bx, by_tag(&dom, "section"));
    let (w, n) = (&section.children[0], &section.children[1]);
    assert!(approx(w.rect.width, VIEW), "{}", w.rect.width);
    // Deuxième item sur la ligne suivante, colonne 1.
    assert!(approx(n.rect.x, 0.0), "{}", n.rect.x);
    assert!(n.rect.y > w.rect.y, "{} vs {}", n.rect.y, w.rect.y);
}

#[test]
fn grid_repeat() {
    let (dom, bx) = laid(
        "<!doctype html><section><div>A</div><div>B</div><div>C</div></section>",
        "body { margin: 0 } section { display: grid; \
         grid-template-columns: repeat(3, 100px) } section > div { height: 10px }",
    );
    let section = find(&bx, by_tag(&dom, "section"));
    let c = &section.children[2];
    assert!(approx(c.rect.x, 200.0), "{}", c.rect.x);
    assert!(approx(c.rect.width, 100.0), "{}", c.rect.width);
}
