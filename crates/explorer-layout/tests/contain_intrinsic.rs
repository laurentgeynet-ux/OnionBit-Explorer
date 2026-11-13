// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! `contain-intrinsic-*`/`content-visibility`/`contain: size` (Étape
//! 230) : seul `hidden` saute ses enfants ; `auto` et `contain: size`
//! les gardent boxifiés — la taille déclarée voyage par `intrinsic`.

use explorer_css::{Origin, StyleEngine, StyleEnv};
use explorer_html::TreeBuilder;

/// Nombre de boîtes enfants sous `.x` — le contenu
/// boxifié (ou sauté) se lit directement.
fn children_of(html: &str, css: &str, class: &str) -> (usize, Option<(f32, f32)>) {
    fn find<'a>(
        bx: &'a explorer_layout::LayoutBox,
        dom: &explorer_dom::Dom,
        class: &str,
    ) -> Option<&'a explorer_layout::LayoutBox> {
        if bx
            .node
            .and_then(|n| dom.attribute_str(n, "class").ok().flatten())
            .is_some_and(|c| c == class)
        {
            return Some(bx);
        }
        bx.children.iter().find_map(|c| find(c, dom, class))
    }
    let dom = TreeBuilder::parse(html);
    let mut eng = StyleEngine::new();
    eng.add_css(css, Origin::Author);
    let styles = eng.compute_tree(&dom, dom.document(), &StyleEnv::default());
    let bx = explorer_layout::build_box_tree(&dom, dom.document(), &styles).expect("arbre");
    let b = find(&bx, &dom, class).expect("boîte .x");
    (b.children.len(), b.intrinsic)
}

#[test]
fn content_visibility_auto_keeps_children() {
    // Étape 230 — `auto` ne saute que hors-écran : le skip
    // inconditionnel R-251 figeait le contenu invisible au
    // scroll ; la boxification garde désormais les enfants
    // (et `contain-intrinsic-*` est ignoré — il ne vaut que
    // pour le contenu sauté).
    let (kids, intrinsic) = children_of(
        "<body><div class=x><p>contenu</p></div></body>",
        ".x { content-visibility: auto; contain-intrinsic-size: 200px 80px }",
        "x",
    );
    assert_eq!(kids, 1, "auto doit garder son contenu");
    assert_eq!(intrinsic, None, "auto non sauté ignore l'intrinsèque");
}

#[test]
fn content_visibility_hidden_skips_children() {
    // `hidden` saute sans condition — feuille à taille
    // déclarée, aucun enfant.
    let (kids, intrinsic) = children_of(
        "<body><div class=x><p>contenu</p></div></body>",
        ".x { content-visibility: hidden; contain-intrinsic-size: 200px 80px }",
        "x",
    );
    assert_eq!(kids, 0);
    assert_eq!(intrinsic, Some((200.0, 80.0)));
}

#[test]
fn contain_size_intrinsic() {
    // `contain: size` : taille déclarée forcée **et** contenu
    // boxifié/rendu (Étape 230 — le leaf-skip R-251 le rendait
    // invisible ; le contenu déborde la boîte si plus grand).
    let (kids, intrinsic) = children_of(
        "<body><div class=x><p>contenu</p></div></body>",
        ".x { contain: size; contain-intrinsic-width: 300px; contain-intrinsic-height: 60px }",
        "x",
    );
    assert_eq!(kids, 1, "contain:size doit garder son contenu");
    assert_eq!(intrinsic, Some((300.0, 60.0)));
}
