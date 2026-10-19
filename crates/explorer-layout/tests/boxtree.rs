// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Tests de l'arbre de boîtes (étape 17).

use explorer_css::{Origin, StyleEngine, StyleEnv};
use explorer_dom::{Dom, NodeId};
use explorer_html::TreeBuilder;
use explorer_layout::{build_box_tree, BoxKind, LayoutBox};

/// Concatène le contenu des éléments `<style>` du DOM
/// (feuilles liées : hors périmètre de l'étape 17).
fn collect_style_css(dom: &Dom) -> String {
    let mut css = String::new();
    let mut queue = vec![dom.document()];
    while let Some(n) = queue.pop() {
        if dom.tag_name(n).is_ok_and(|t| t == "style") {
            if let Ok(t) = dom.text_content(n) {
                css.push_str(&t);
                css.push('\n');
            }
        }
        if let Ok(ch) = dom.children(n) {
            queue.extend(ch.iter().rev());
        }
    }
    css
}

/// Parse `html`, calcule les styles, construit l'arbre de
/// boîtes sous le premier élément du document.
fn tree(html: &str) -> (Dom, LayoutBox) {
    let dom = TreeBuilder::parse(html);
    let mut eng = StyleEngine::new();
    eng.add_css(&collect_style_css(&dom), Origin::Author);
    let styles = eng.compute_tree(&dom, dom.document(), &StyleEnv::default());
    // La racine des boîtes = premier élément sous le document.
    let root_el = dom
        .children(dom.document())
        .into_iter()
        .flatten()
        .find(|&c| dom.element(c).is_ok())
        .expect("un élément sous le document");
    let bx = build_box_tree(&dom, root_el, &styles).expect("arbre de boîtes");
    (dom, bx)
}

/// Cherche la première boîte portant un nœud `id` donné.
fn find(b: &LayoutBox, id: NodeId) -> Option<&LayoutBox> {
    if b.node == Some(id) {
        return Some(b);
    }
    b.children.iter().find_map(|c| find(c, id))
}

/// Trouve un nœud par tag dans le DOM (parcours en largeur).
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
    panic!("tag <{tag}> absent")
}

#[test]
fn arbre_simple() {
    let (dom, bx) = tree("<!doctype html><p>bonjour</p>");
    // html → block ; head/body blocks ; p block ; texte.
    assert_eq!(bx.kind, BoxKind::Block);
    let p = find(&bx, by_tag(&dom, "p")).expect("boîte p");
    assert_eq!(p.kind, BoxKind::Block);
    // Le texte du p est enveloppé dans un bloc anonyme.
    assert_eq!(p.children.len(), 1);
    assert_eq!(p.children[0].kind, BoxKind::AnonymousBlock);
    assert_eq!(p.children[0].children[0].kind, BoxKind::Text);
    assert_eq!(p.children[0].children[0].text.as_deref(), Some("bonjour"));
}

#[test]
fn display_none_saute_le_sous_arbre() {
    let (dom, bx) =
        tree("<!doctype html><style>p{display:none}</style><p>invisible</p><div>vu</div>");
    let p_id = by_tag(&dom, "p");
    assert!(find(&bx, p_id).is_none(), "p display:none → pas de boîte");
    assert!(find(&bx, by_tag(&dom, "div")).is_some());
}

#[test]
fn inline_et_anonyme() {
    let (dom, bx) = tree("<!doctype html><div>a <b>x</b> c</div>");
    let div = find(&bx, by_tag(&dom, "div")).expect("boîte div");
    assert_eq!(div.kind, BoxKind::Block);
    // Tout en-ligne → un seul anonyme ; blancs internes gardés.
    assert_eq!(div.children.len(), 1);
    let anon = &div.children[0];
    assert_eq!(anon.kind, BoxKind::AnonymousBlock);
    assert!(anon.node.is_none());
    assert_eq!(anon.children.len(), 3, "texte, b, texte");
    let b = find(anon, by_tag(&dom, "b")).expect("b");
    assert_eq!(b.kind, BoxKind::Inline);
}

#[test]
fn mixte_block_inline_enveloppes() {
    let (dom, bx) = tree("<!doctype html><div>tête<p>bloc</p>queue<span>fin</span></div>");
    let div = find(&bx, by_tag(&dom, "div")).expect("boîte div");
    assert_eq!(div.children.len(), 3, "anon, p, anon");
    assert_eq!(div.children[0].kind, BoxKind::AnonymousBlock);
    assert_eq!(div.children[1].kind, BoxKind::Block);
    assert_eq!(div.children[2].kind, BoxKind::AnonymousBlock);
    // Le span finit dans le second anonyme (texte + inline).
    assert_eq!(div.children[2].children.len(), 2);
    assert_eq!(div.children[2].children[1].kind, BoxKind::Inline);
}

#[test]
fn blancs_purs_en_bord_tronques() {
    let (dom, bx) = tree("<!doctype html><div>\n  <p>x</p>\n</div>");
    let div = find(&bx, by_tag(&dom, "div")).expect("div");
    // Un seul enfant : le p — pas d'anonyme pour les blancs.
    assert_eq!(div.children.len(), 1);
    assert_eq!(div.children[0].kind, BoxKind::Block);
}

#[test]
fn espace_entre_inlines_preserve() {
    let (dom, bx) = tree("<!doctype html><div><b>x</b> <i>y</i></div>");
    let div = find(&bx, by_tag(&dom, "div")).expect("div");
    let anon = &div.children[0];
    assert_eq!(anon.kind, BoxKind::AnonymousBlock);
    // b, " ", i → l'espace survit entre les deux en-lignes.
    assert_eq!(anon.children.len(), 3);
    assert_eq!(anon.children[1].kind, BoxKind::Text);
}

#[test]
fn list_item_est_bloc_avec_puce() {
    let (dom, bx) = tree("<!doctype html><ul><li>un</li><li>deux</li></ul>");
    let li = find(&bx, by_tag(&dom, "li")).expect("li");
    assert_eq!(li.kind, BoxKind::ListItem);
}

#[test]
fn dump_wireframe_lisible() {
    let (dom, bx) = tree("<!doctype html><p>salut</p>");
    let dump = bx.dump(&dom);
    assert!(dump.contains("block"), "{dump}");
    assert!(dump.contains("anon"), "{dump}");
    assert!(dump.contains("\"salut\""), "{dump}");
    assert!(dump.contains("<p>") || dump.contains('p'), "{dump}");
}
