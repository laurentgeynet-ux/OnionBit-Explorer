// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Images (étape 37) : `decode_image` borné + blit
//! `DisplayItem::Image` dans la display list — pixels
//! sondés, fallback `alt` quand l'image manque.

use std::collections::HashMap;
use std::sync::Arc;

use explorer_css::{StyleEngine, StyleEnv};
use explorer_dom::{Dom, NodeId};
use explorer_html::TreeBuilder;
use explorer_layout::{build_box_tree_sized, layout_tree, BlockCtx, LayoutBox};
use explorer_render::{
    build_display_list_imaged, decode_image, rasterize, DisplayItem, ImageTable,
};

/// PNG 2×2 rouge opaque, encodé à la volée.
fn red_png() -> Vec<u8> {
    use image::{codecs::png::PngEncoder, ExtendedColorType, ImageEncoder};
    let mut buf = Vec::new();
    PngEncoder::new(&mut buf)
        .write_image(&[255, 0, 0, 255].repeat(4), 2, 2, ExtendedColorType::Rgba8)
        .expect("encode png");
    buf
}

#[test]
fn decode_roundtrip_rgba() {
    let img = decode_image(&red_png(), 1_000_000).expect("png décodé");
    assert_eq!((img.width, img.height), (2, 2));
    assert_eq!(img.rgba.len(), 16);
    assert_eq!(&img.rgba[..4], &[255, 0, 0, 255]);
}

#[test]
fn decode_bounds_rejects_huge() {
    // 2×2 = 4 px > budget 3 → refus (borne hostile).
    assert!(decode_image(&red_png(), 3).is_none());
}

#[test]
fn decode_garbage_is_none() {
    assert!(decode_image(b"pas une image du tout", 1_000_000).is_none());
}

/// Premier nœud `<img>` du document.
fn img_node(dom: &Dom) -> NodeId {
    let mut queue = vec![dom.document()];
    while let Some(n) = queue.pop() {
        if dom.tag_name(n).is_ok_and(|t| t == "img") {
            return n;
        }
        if let Ok(ch) = dom.children(n) {
            queue.extend(ch.iter().rev());
        }
    }
    panic!("pas d'<img> dans la fixture");
}

fn first_image_item(items: &[DisplayItem]) -> Option<&explorer_layout::Rect> {
    items.iter().find_map(|i| match i {
        DisplayItem::Image { rect, .. } => Some(rect),
        _ => None,
    })
}

fn has_alt(b: &LayoutBox) -> bool {
    b.text.as_deref() == Some("alt-ici") || b.children.iter().any(has_alt)
}

#[test]
fn image_item_blits_pixels() {
    let img = Arc::new(decode_image(&red_png(), 1_000_000).expect("png"));
    let dom: Dom = TreeBuilder::parse("<html><body><img src=x></body></html>");
    let node = img_node(&dom);
    let eng = StyleEngine::new();
    let styles = eng.compute_tree(&dom, dom.document(), &StyleEnv::default());
    let root_el = dom
        .children(dom.document())
        .into_iter()
        .flatten()
        .find(|&c| dom.element(c).is_ok())
        .expect("racine");

    // Taille préférée = intrinsèque (aucun attr).
    let mut sizes = HashMap::new();
    sizes.insert(node, (2.0, 2.0));
    let mut bx = build_box_tree_sized(&dom, root_el, &styles, &sizes).expect("arbre");
    layout_tree(&mut bx, 200.0, &BlockCtx::default());

    let mut images = ImageTable::new();
    images.insert(node, img);
    let list = build_display_list_imaged(&bx, 200.0, 200.0, 0.0, &images);
    // La display list porte un item Image couvrant ≥ 2×2.
    let r = first_image_item(&list.items).expect("item Image");
    assert!(r.width >= 2.0 && r.height >= 2.0, "rect = {r:?}");

    let frame = rasterize(&list, &NullFonts, 1.0).expect("frame");
    assert_eq!(&frame.pixels[..4], &[255, 0, 0, 255]);
}

#[test]
fn missing_image_falls_back_to_alt_text() {
    // Aucune image décodée → le `<img>` devient une boîte
    // texte portant `alt` (le bitmap n'est pas attendu).
    let dom: Dom = TreeBuilder::parse("<html><body><img src=x alt=\"alt-ici\"></body></html>");
    let node = img_node(&dom);
    let eng = StyleEngine::new();
    let styles = eng.compute_tree(&dom, dom.document(), &StyleEnv::default());
    let root_el = dom
        .children(dom.document())
        .into_iter()
        .flatten()
        .find(|&c| dom.element(c).is_ok())
        .expect("racine");
    let bx = build_box_tree_sized(&dom, root_el, &styles, &HashMap::new()).expect("arbre");
    // Le sous-arbre contient une boîte texte "alt-ici".
    assert!(has_alt(&bx), "alt non matérialisé : {}", bx.dump(&dom));
    let _ = node;
}

/// Fonte absente — le test image n'a pas de texte à shaper.
struct NullFonts;
impl explorer_layout::font::FontSource for NullFonts {
    fn font_data(&self, _req: &explorer_layout::font::FontRequest) -> Option<Vec<u8>> {
        None
    }
}
