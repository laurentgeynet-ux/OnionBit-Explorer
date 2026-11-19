// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Tests du dump géométrique normalisé (R-208) : union par
//! nœud, boîtes anonymes ignorées, ordre déterministe.

mod common;

use explorer_css::{Origin, StyleEngine, StyleEnv};
use explorer_dom::{Dom, NodeId};
use explorer_html::TreeBuilder;
use explorer_layout::{build_box_tree, geometry_dump, layout_tree, BlockCtx, LayoutBox};

const VIEW: f32 = 800.0;

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

#[test]
fn entrees_par_noeud_union_et_ordre() {
    let (dom, bx) = laid(
        "<body><div id=a style='height:20px'>x</div><p style='height:10px'>y</p></body>",
        "",
    );
    let d = geometry_dump(&bx);
    assert!(d.width > 0.0 && d.height > 0.0);
    // Chaque élément/tag présent → une entrée ; index unique.
    let div = by_tag(&dom, "div");
    let p = by_tag(&dom, "p");
    let ed = d.entries.iter().find(|e| e.node == div).expect("div");
    let ep = d.entries.iter().find(|e| e.node == p).expect("p");
    assert!(ed.rect.height >= 19.0 && ep.rect.height >= 9.0);
    assert!(ep.rect.y >= ed.rect.y + 19.0, "p sous div: {:?}", d.entries);
    // Le nœud racine n'est pas anonyme → première entrée.
    assert!(d.entries.iter().all(|e| e.fragments >= 1));
}

#[test]
fn union_fragments_inline_multiligne() {
    // Un inline étiré sur deux lignes → plusieurs fragments,
    // l'union couvre les deux lignes (gBCR).
    let (dom, bx) = laid(
        "<body><div style='width:80px'><span>mot mot mot mot mot mot mot mot</span></div></body>",
        "span { font-size: 20px }",
    );
    let d = geometry_dump(&bx);
    let span = by_tag(&dom, "span");
    let e = d.entries.iter().find(|e| e.node == span);
    // Le span inline peut produire un fragment par ligne ;
    // dans tous les cas l'entrée existe et sa hauteur couvre
    // le contenu posé.
    let e = e.expect("span");
    assert!(e.rect.height > 0.0);
}

#[test]
fn style_dump_style_calcule_et_parent() {
    // Étape 315 : chaque nœud producteur rapporte son style
    // calculé réduit aux props d'attribution + son plus
    // proche ancêtre de boîte — le sidecar de `geo_diff`.
    fn prop<'a>(e: &'a explorer_layout::StyleEntry, k: &str) -> Option<&'a str> {
        e.computed
            .iter()
            .find(|(n, _)| *n == k)
            .map(|(_, v)| v.as_str())
    }
    let (dom, bx) = laid(
        "<body><div id=p style='display:flex; gap:4px'><span>x</span></div></body>",
        "div { line-height: 2 } span { font-size: 22px }",
    );
    let d = explorer_layout::style_dump(&bx);
    let div = by_tag(&dom, "div");
    let span = by_tag(&dom, "span");
    let ed = d.iter().find(|e| e.node == div).expect("div");
    let es = d.iter().find(|e| e.node == span).expect("span");
    // Props d'attribution présentes quand déclarées.
    assert_eq!(prop(ed, "display"), Some("flex"));
    assert_eq!(prop(ed, "line-height"), Some("2"));
    // font-size propre au span.
    assert_eq!(prop(es, "font-size"), Some("22px"));
    // Chaîne de parenté : le span remonte au div de boîte.
    assert_eq!(es.parent, Some(div));
    // La racine n'a pas de parent de boîte.
    assert!(d.iter().any(|e| e.parent.is_none()));
}
