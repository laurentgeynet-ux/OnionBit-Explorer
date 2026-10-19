// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Tests de la display list (étape 22) : ordre de peinture,
//! fonds/bordures/texte, clipping `overflow`, tri `z-index`.

use explorer_css::{Origin, StyleEngine, StyleEnv};
use explorer_dom::{Dom, NodeId};
use explorer_html::TreeBuilder;
use explorer_layout::font::{FontRequest, FontSource};
use explorer_layout::{build_box_tree, layout_tree, BlockCtx, LayoutBox};
use explorer_render::{build_display_list, BorderStyle, DisplayItem};

const VIEW: f32 = 400.0;

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

fn dl(html: &str, css: &str) -> (Dom, LayoutBox, Vec<DisplayItem>) {
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
    layout_tree(&mut bx, VIEW, &ctx());
    let list = build_display_list(&bx, VIEW, 200.0, 0.0);
    (dom, bx, list.items)
}

fn by_id(dom: &Dom, id: &str) -> NodeId {
    let mut q = vec![dom.document()];
    while let Some(n) = q.pop() {
        if dom.attribute(n, "id").ok().flatten().as_deref() == Some(id) {
            return n;
        }
        if let Ok(ch) = dom.children(n) {
            q.extend(ch.iter().rev());
        }
    }
    panic!("#{id} absent")
}

fn find(b: &LayoutBox, id: NodeId) -> Option<&LayoutBox> {
    if b.node == Some(id) {
        return Some(b);
    }
    b.children.iter().find_map(|c| find(c, id))
}

#[test]
fn fond_emis_avant_contenu() {
    let (_dom, _bx, items) = dl(
        "<div id=d><p>texte</p></div>",
        "#d{background-color:#ff0000} p{margin:0}",
    );
    // Le fond rouge est émis avant les items du <p>.
    let fill = items
        .iter()
        .position(|i| matches!(i, DisplayItem::FillRect { .. }))
        .expect("fond émis");
    assert!(fill < items.len());
    assert!(matches!(items[fill], DisplayItem::FillRect { .. }));
}

#[test]
fn bordure_un_item_quatre_cotes() {
    let (_dom, _bx, items) = dl("<div id=d></div>", "#d{border:2px solid #00f;height:10px}");
    let border = items
        .iter()
        .find_map(|i| match i {
            DisplayItem::Border { widths, styles, .. } => Some((*widths, *styles)),
            _ => None,
        })
        .expect("item bordure");
    assert!(border.0.iter().all(|w| (*w - 2.0).abs() < 0.01));
    assert!(border.1.iter().all(|s| *s == BorderStyle::Solid));
}

#[test]
fn texte_emis_depuis_fragments_shapes() {
    let (_dom, _bx, items) = dl("<p>bonjour</p>", "p{margin:0}");
    let texts = items
        .iter()
        .filter(|i| matches!(i, DisplayItem::Text { .. }))
        .count();
    assert!(texts > 0, "au moins un item texte");
}

#[test]
fn overflow_clip_encadre_les_enfants() {
    let (_dom, _bx, items) = dl(
        "<div id=o><div id=in></div></div>",
        "#o{overflow:hidden;height:20px} #in{height:50px}",
    );
    let push = items
        .iter()
        .position(|i| matches!(i, DisplayItem::PushClip(_)))
        .expect("PushClip");
    let pop = items
        .iter()
        .rposition(|i| matches!(i, DisplayItem::PopClip))
        .expect("PopClip");
    assert!(push < pop, "clip ouvert puis refermé");
    // L'enfant (fond ou fill) est entre les deux.
    assert!(items.len() > push + 1);
}

#[test]
fn zindex_trie_les_positionnes() {
    // Deux positionnés : z=5 doit passer après z=1.
    let (dom, bx, items) = dl(
        "<div id=a></div><div id=b></div>",
        "#a{position:absolute;z-index:5;background-color:#ff0000;\
         width:10px;height:10px}\
         #b{position:absolute;z-index:1;background-color:#00ff00;\
         width:10px;height:10px}",
    );
    let _ = find(&bx, by_id(&dom, "a"));
    // Les deux fonds : #b (z=1) doit être émis avant #a (z=5)
    // malgré l'ordre DOM.
    let fills: Vec<explorer_css::value::Color> = items
        .iter()
        .filter_map(|i| match i {
            DisplayItem::FillRect { color, .. } => Some(*color),
            _ => None,
        })
        .collect();
    let green = fills
        .iter()
        .position(|c| c.g == 255)
        .expect("fond vert (#b)");
    let red = fills
        .iter()
        .position(|c| c.r == 255)
        .expect("fond rouge (#a)");
    assert!(green < red, "z=1 peint avant z=5 : {fills:?}");
}

#[test]
fn display_none_nemet_rien() {
    let (_dom, _bx, items) = dl(
        "<div id=v></div><div id=h></div>",
        "#v{background-color:#ff0000;height:5px}\
         #h{display:none;background-color:#00ff00;height:5px}",
    );
    let greens = items
        .iter()
        .filter(|i| matches!(i, DisplayItem::FillRect { color, .. } if color.g == 255))
        .count();
    assert_eq!(greens, 0, "display:none ne peint pas");
}

// ── transforms / sticky (étape 42) ────────────────────────────

/// `dl` avec `scroll_y` réglable (sticky dépend du scroll).
fn dl_scrolled(html: &str, css: &str, scroll_y: f32) -> (Dom, LayoutBox, Vec<DisplayItem>) {
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
    layout_tree(&mut bx, VIEW, &ctx());
    let list = build_display_list(&bx, VIEW, 200.0, scroll_y);
    (dom, bx, list.items)
}

#[test]
fn transform_translate_encadre_le_sous_arbre() {
    let (_dom, _bx, items) = dl(
        "<div id=t><span>x</span></div>",
        "#t{transform:translate(10px, 5px);background-color:#ff0000;\
         width:20px;height:20px}",
    );
    let push = items
        .iter()
        .position(|i| matches!(i, DisplayItem::PushTransform(_)))
        .expect("push-transform émis");
    let pop = items
        .iter()
        .rposition(|i| matches!(i, DisplayItem::PopTransform))
        .expect("pop-transform émis");
    let fill = items
        .iter()
        .position(|i| matches!(i, DisplayItem::FillRect { .. }))
        .expect("fond émis");
    assert!(push < fill && fill < pop, "fond encadré par la transform");
    // La matrice porte la translation + origine centre :
    // T(20,15) · translate(10,5) · T(-20,-15) — résultat net
    // = translate(10,5) quel que soit l'origine.
    if let DisplayItem::PushTransform(m) = &items[push] {
        assert!((m.tx - 10.0).abs() < 0.5, "tx={}", m.tx);
        assert!((m.ty - 5.0).abs() < 0.5, "ty={}", m.ty);
    }
}

#[test]
fn transform_none_ou_invalide_nemet_rien() {
    let (_dom, _bx, items) = dl(
        "<div id=t></div>",
        "#t{transform:foo(1);background-color:#ff0000;height:5px}",
    );
    assert!(
        !items
            .iter()
            .any(|i| matches!(i, DisplayItem::PushTransform(_))),
        "transform invalide ignorée"
    );
}

#[test]
fn sticky_colle_au_seuil_sous_le_scroll() {
    // Document 600px, header sticky top:0 — scroll 300 →
    // le header (haut document à y≈0) doit être translaté
    // de ~300 pour rester visible.
    let (_dom, _bx, items) = dl_scrolled(
        "<header id=h></header><main></main>",
        "body{margin:0} main{height:600px} #h{position:sticky;top:0;background-color:#ff0000;height:20px}",
        300.0,
    );
    let push = items
        .iter()
        .position(|i| matches!(i, DisplayItem::PushTransform(_)))
        .expect("sticky → transform émise");
    if let DisplayItem::PushTransform(m) = &items[push] {
        // painted_top = 0 - 300 = -300 → dy = 0 - (-300) = 300.
        assert!((m.ty - 300.0).abs() < 1.0, "ty={}", m.ty);
    }
}

#[test]
fn sticky_sans_scroll_ne_decolle_pas() {
    let (_dom, _bx, items) = dl_scrolled(
        "<header id=h></header><main></main>",
        "body{margin:0} main{height:600px} #h{position:sticky;top:0;background-color:#ff0000;height:20px}",
        0.0,
    );
    assert!(
        !items
            .iter()
            .any(|i| matches!(i, DisplayItem::PushTransform(_))),
        "pas de scroll → pas de translation"
    );
}
