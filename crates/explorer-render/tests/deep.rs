// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! R-24 : un document profond ne doit **jamais** déborder la
//! pile (`repros/host-nest*.html` tuait le worker tokio vers
//! ~300 `<div>` imbriqués). Chaque traversée du pipeline
//! (cascade → boxtree → layout → display list → hit-test)
//! passe ses arêtes récursives par `explorer_common::deep` —
//! bascule sur segment tas en zone rouge — et `LayoutBox`
//! se détruit itérativement.
//!
//! Le test tourne sur un thread à pile volontairement
//! réduite : sans la protection, ~500 niveaux × ~5 Kio de
//! frame de layout dépassent très largement la réserve.

use std::collections::HashMap;

use explorer_css::{Origin, StyleEngine, StyleEnv};
use explorer_dom::Dom;
use explorer_html::TreeBuilder;
use explorer_layout::font::{FontRequest, FontSource};
use explorer_layout::{build_box_tree_sized, hit_test, layout_tree, BlockCtx};
use explorer_render::{build_display_list_full, PaintImages};

struct Fixture(Vec<u8>);
impl FontSource for Fixture {
    fn font_data(&self, _req: &FontRequest) -> Option<Vec<u8>> {
        Some(self.0.clone())
    }
}

fn ctx() -> BlockCtx<'static> {
    let src: &'static Fixture = Box::leak(Box::new(Fixture(explorer_test_support::fixture_ttf())));
    BlockCtx {
        fonts: Some(src),
        ..BlockCtx::default()
    }
}

/// `<div>` imbriqués `n` niveaux — la structure du repro
/// terrain `host-nest.html`.
fn nested(n: usize) -> String {
    let mut s = String::with_capacity(n * 12 + 64);
    s.push_str("<!doctype html><html><body>");
    for _ in 0..n {
        s.push_str("<div>");
    }
    s.push_str("fond");
    for _ in 0..n {
        s.push_str("</div>");
    }
    s.push_str("</body></html>");
    s
}

/// Nombre de `<div>` effectivement dans le DOM (le parser
/// borne déjà la profondeur — c'est son budget, pas le nôtre).
fn count_divs(dom: &Dom) -> usize {
    let mut queue = vec![dom.document()];
    let mut n = 0;
    while let Some(x) = queue.pop() {
        if dom.tag_name(x).is_ok_and(|t| t == "div") {
            n += 1;
        }
        if let Ok(c) = dom.children(x) {
            queue.extend(c);
        }
    }
    n
}

#[test]
fn document_profond_ne_deborde_pas_la_pile() {
    // 4000 demandés ; le tree-builder tronque bien plus tôt
    // (borne propre) — la traversée aval doit suivre tout ce
    // que le DOM contient, sans crash.
    const STACK: usize = 512 * 1024;
    std::thread::Builder::new()
        .stack_size(STACK)
        .spawn(|| {
            let dom = TreeBuilder::parse(&nested(4000));
            let divs = count_divs(&dom);
            assert!(divs >= 100, "le repro réel nicherait ~500");
            let mut eng = StyleEngine::new();
            eng.add_css("div { margin: 1px; padding: 1px }", Origin::Author);
            // Cascade récursive sur la profondeur DOM.
            let styles = eng.compute_tree(&dom, dom.document(), &StyleEnv::default());
            let root = dom.document_element().expect("racine");
            // Boxtree récursif.
            let mut bx = build_box_tree_sized(&dom, root, &styles, &HashMap::new()).expect("arbre");
            // Layout récursif (flow + absolus + inline).
            layout_tree(&mut bx, 800.0, &ctx());
            // Display list récursive.
            let list = build_display_list_full(&bx, 800.0, 600.0, 0.0, &PaintImages::default());
            // Hit-test + text_content récursifs.
            assert!(hit_test(&bx, 2.0, 2.0).is_some());
            assert!(dom.text_content(root).is_ok_and(|t| t.contains("fond")));
            assert!(!list.items.is_empty(), "le texte « fond » peint");
            // `bx`/`dom` droppés ici — drop itératif, pas de
            // descente récursive sur des centaines de niveaux.
        })
        .expect("spawn")
        .join()
        .expect("thread : débordement de pile");
}

#[test]
fn dump_profond_ne_deborde_pas() {
    const STACK: usize = 384 * 1024;
    std::thread::Builder::new()
        .stack_size(STACK)
        .spawn(|| {
            let dom = TreeBuilder::parse(&nested(3000));
            let mut eng = StyleEngine::new();
            eng.add_css("", Origin::Author);
            let styles = eng.compute_tree(&dom, dom.document(), &StyleEnv::default());
            let root = dom.document_element().expect("racine");
            let bx = build_box_tree_sized(&dom, root, &styles, &HashMap::new()).expect("arbre");
            // `dump` marche tout le sous-arbre récursivement.
            let d = bx.dump(&dom);
            assert!(d.lines().count() >= count_divs(&dom));
        })
        .expect("spawn")
        .join()
        .expect("thread : débordement de pile");
}
