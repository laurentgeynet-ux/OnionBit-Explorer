// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Repros figés de sites réels — corpus de régression (ADR-0010).
//!
//! `data/repros/lobste/` : page d'accueil lobste.rs (25 stories)
//! et sa `application.css`. `system-system.css` est la cible des
//! `@import` du site — non résolue côté domaine (la couture fetch
//! relève de NAVIGATEUR) : les `var(--color-*)` retombent donc
//! sur leurs replis (§ R-22 — `border-color` → `currentColor`
//! → `color` → noir à défaut).
//!
//! R-22 : cette page exposait trois défauts —
//!   1. `ul.tags` (`display:inline-block`) prenait la largeur du
//!      conteneur (~978 px) au lieu du shrink-to-fit (~87 px) ;
//!   2. le contenu d'un `position:absolute` niché en inline et
//!      les blancs internes d'un `inline-block` contenant un
//!      bloc (`details.caches > summary`) gonflaient la hauteur
//!      de ligne des bylines (23 px → ~93 px) ;
//!   3. fond/bordure des `<a class=tag>` peints sur l'union des
//!      fragments wrappés au lieu de chaque fragment.

use explorer_css::{Origin, StyleEngine, StyleEnv};
use explorer_dom::{Dom, NodeId};
use explorer_html::TreeBuilder;
use explorer_layout::font::{FontRequest, FontSource};
use explorer_layout::{build_box_tree, layout_tree, BlockCtx, BoxKind, LayoutBox};

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

/// Parse le fixture lobste.rs et pose l'arbre à 1024 px de large.
fn lobste() -> (Dom, LayoutBox) {
    let dom = TreeBuilder::parse(include_str!("data/repros/lobste/index.html"));
    let mut eng = StyleEngine::new();
    eng.add_css(
        include_str!("data/repros/lobste/application.css"),
        Origin::Author,
    );
    let styles = eng.compute_tree(&dom, dom.document(), &StyleEnv::default());
    let root = dom
        .children(dom.document())
        .into_iter()
        .flatten()
        .find(|&c| dom.element(c).is_ok())
        .expect("racine");
    let mut bx = build_box_tree(&dom, root, &styles).expect("arbre");
    layout_tree(&mut bx, 1024.0, &ctx());
    (dom, bx)
}

/// `true` si le nœud porte `class` contenant `tok` (mot entier).
fn has_class(dom: &Dom, n: Option<NodeId>, tok: &str) -> bool {
    n.and_then(|n| dom.attribute(n, "class").ok().flatten())
        .is_some_and(|c| c.split_whitespace().any(|w| w == tok))
}

/// Collecte toutes les boîtes dont le nœud porte la classe `tok`.
fn collect<'a>(bx: &'a LayoutBox, dom: &Dom, tok: &str, out: &mut Vec<&'a LayoutBox>) {
    if has_class(dom, bx.node, tok) {
        out.push(bx);
    }
    for c in &bx.children {
        collect(c, dom, tok, out);
    }
}

#[test]
fn lobste_stories_compactes() {
    let (dom, bx) = lobste();
    let mut stories = Vec::new();
    collect(&bx, &dom, "story", &mut stories);
    assert_eq!(stories.len(), 25, "25 stories attendues");
    for s in &stories {
        // Avant R-22 : ~158 px+ (lignes gonflées par les
        // absolus nichés et les blancs des atomes).
        assert!(s.rect.height < 130.0, "story trop haute : {:?}", s.rect);
        assert!(s.rect.height > 30.0, "story sans contenu : {:?}", s.rect);
    }
}

#[test]
fn lobste_tags_shrink_to_fit() {
    let (dom, bx) = lobste();
    let mut tags = Vec::new();
    collect(&bx, &dom, "tags", &mut tags);
    assert_eq!(tags.len(), 25, "25 listes de tags attendues");
    for t in &tags {
        // `display:inline-block` → shrink-to-fit : la liste de
        // tags est compacte (~87 px), pas la largeur conteneur.
        assert!(
            t.rect.width < 300.0,
            "ul.tags non shrink-to-fit : {:?}",
            t.rect
        );
    }
}

#[test]
fn lobste_bylines_une_ligne() {
    let (dom, bx) = lobste();
    let mut bylines = Vec::new();
    collect(&bx, &dom, "byline", &mut bylines);
    assert_eq!(bylines.len(), 25, "25 bylines attendues");
    for b in &bylines {
        // La byline contient un `position:absolute` niché et un
        // `details.caches` inline-block — ni l'un ni l'autre ne
        // doit entrer dans le calcul de la hauteur de ligne.
        let anon = b
            .children
            .iter()
            .find(|c| c.kind == BoxKind::AnonymousBlock);
        let Some(anon) = anon else { continue };
        for line in &anon.lines {
            assert!(
                line.rect.height < 30.0,
                "ligne de byline gonflée : {:?}",
                line.rect
            );
        }
    }
}
