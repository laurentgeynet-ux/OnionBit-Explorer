// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! §B-58 — `transform` ne doit pas se rejouer sur les boîtes
//! anonymes/texte (leur style clone celui du parent) : une
//! matrice repoussée à chaque niveau d'enveloppe peint le
//! texte sous M²/M³ (letterboxd `rotate(-90)` → miré).

use explorer_css::{Origin, StyleEngine, StyleEnv};
use explorer_dom::Dom;
use explorer_html::TreeBuilder;
use explorer_layout::font::{FontRequest, FontSource};
use explorer_layout::{build_box_tree, layout_tree, BlockCtx};
use explorer_render::{build_display_list, DisplayItem};

const VIEW: f32 = 800.0;

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

fn items(html: &str, css: &str) -> Vec<DisplayItem> {
    let dom: Dom = TreeBuilder::parse(html);
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
    layout_tree(&mut bx, VIEW, &ctx());
    build_display_list(&bx, VIEW, 500.0, 0.0).items
}

/// Nombre de `PushTransform` actifs quand un item `Text` est
/// émis — chacun doit être exactement 1 (la matrice de
/// l'élément, pas celle des enveloppes anonymes).
fn depth_at_text(items: &[DisplayItem]) -> Vec<usize> {
    let mut depth = 0usize;
    let mut out = Vec::new();
    for it in items {
        match it {
            DisplayItem::PushTransform(_) => depth += 1,
            DisplayItem::PopTransform => depth = depth.saturating_sub(1),
            DisplayItem::Text { .. } => out.push(depth),
            _ => {}
        }
    }
    out
}

#[test]
fn transform_element_pousse_une_seule_fois_flux() {
    let items = items(
        "<p id=t>F</p>",
        "#t{margin:0;margin-top:60px;margin-left:80px;font-size:60px;transform:rotate(45deg)}",
    );
    let depths = depth_at_text(&items);
    assert_eq!(depths, vec![1], "texte sous M¹ uniquement");
}

#[test]
fn transform_element_pousse_une_seule_fois_absolu() {
    // `position:absolute` → chemin différé — même exigence.
    let items = items(
        "<p id=t>F</p>",
        "#t{position:absolute;top:60px;left:80px;font-size:60px;transform:rotate(45deg)}",
    );
    let depths = depth_at_text(&items);
    assert_eq!(depths, vec![1], "texte sous M¹ uniquement");
}

#[test]
fn transform_element_inline_dans_bloc_transforme() {
    // Motif letterboxd `.backdrop-metadata > a` : l'enfant
    // inline `<a>` (anonymisé) ne repousse pas la matrice
    // du <p> — le texte reste sous une seule rotation.
    let items = items(
        "<p id=t><a>x</a></p>",
        "#t{position:absolute;top:0;right:0;font-size:20px;\
         transform-origin:0 0;transform:translate(100%) rotate(-90deg)}",
    );
    let depths = depth_at_text(&items);
    assert_eq!(depths, vec![1], "texte sous M¹ uniquement");
}
