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

/// CRC32 (PNG) minimal pour forger un IHDR valide en test.
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in bytes {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

/// §B-59 — un en-tête PNG dont chaque côté passe la limite
/// `image` (côté vs côté) mais dont le PRODUIT dépasse le budget
/// pixels : 131072×65536 = 2³³ px → `w*h*4` = alloc 2³⁵ octets
/// dans le décodeur (`allocation of 34359738368 bytes failed`
/// mesuré terrain). Le produit doit être vérifié sur l'en-tête
/// AVANT `decode()` — refus `None`, jamais l'allocation.
#[test]
fn decode_rejects_huge_product_before_decode() {
    let mut forged = red_png();
    // IHDR : sig(8) + len(4) + "IHDR"(4) + w(4) + h(4) + 5 + crc(4).
    forged[16..20].copy_from_slice(&131_072u32.to_be_bytes());
    forged[20..24].copy_from_slice(&65_536u32.to_be_bytes());
    let crc = crc32(&forged[12..29]); // "IHDR" + données
    forged[29..33].copy_from_slice(&crc.to_be_bytes());
    assert!(decode_image(&forged, 16_000_000).is_none());
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
    // Le strut §10.8.1 peut décaler la ligne — on sonde le
    // centre du rect déclaré par l'item, pas (0,0).
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let (cx, cy) = (
        (r.x + r.width / 2.0) as usize,
        (r.y + r.height / 2.0) as usize,
    );
    let px = &frame.pixels[(cy * frame.width as usize + cx) * 4..][..4];
    assert_eq!(px, &[255, 0, 0, 255], "rect = {r:?}");
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

// ─────────────── SVG (étape 45) ───────────────

/// SVG 10×10 : fond transparent + rectangle vert plein cadre.
const GREEN_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10" viewBox="0 0 10 10"><rect x="0" y="0" width="10" height="10" fill="#00ff00"/></svg>"##;

#[test]
fn svg_decode_rect_rgba() {
    let img = decode_image(GREEN_SVG.as_bytes(), 1_000_000).expect("svg décodé");
    assert_eq!((img.width, img.height), (10, 10));
    // Pixel central : vert opaque (RGBA droit, démultiplié).
    let c = (5 * 10 + 5) * 4;
    assert_eq!(
        &img.rgba[c..c + 4],
        &[0, 255, 0, 255],
        "px = {:?}",
        &img.rgba[c..c + 4]
    );
}

#[test]
fn svg_xml_prolog_et_doctype() {
    let svg = format!("<?xml version=\"1.0\"?>\n<!DOCTYPE svg>\n{GREEN_SVG}");
    assert!(decode_image(svg.as_bytes(), 1_000_000).is_some());
}

#[test]
fn svg_viewbox_geant_borne() {
    // Géométrie déclarée 10⁶ × 10⁶ = 10¹² px → réduite au
    // budget, jamais d'allocation démesurée (hostile).
    let big = r##"<svg xmlns="http://www.w3.org/2000/svg" width="1000000" height="1000000" viewBox="0 0 1000000 1000000"><rect width="1000000" height="1000000" fill="#00ff00"/></svg>"##;
    let img = decode_image(big.as_bytes(), 1_000_000).expect("borné pas refusé");
    assert!(u64::from(img.width) * u64::from(img.height) <= 1_000_000);
}

#[test]
fn svg_filtres_regions_divergentes_ne_panique_pas() {
    // §B-55 (vol 94, ici.fr) : `filter="url(#a) url(#b)"` — le
    // calque resvg couvre l'union des régions (200×200) mais
    // `feComposite arithmetic` détruit à la région de f1
    // (100×100) → `assertion src1.width == dest.width` dans la
    // dépendance. Confiné par `catch_unwind` → `None`.
    let hostile = r#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100">
      <filter id="f1" x="0" y="0" width="1" height="1">
        <feComposite in="SourceGraphic" in2="SourceGraphic" operator="arithmetic" k1="1" k2="0" k3="0" k4="0"/>
      </filter>
      <filter id="f2" x="0" y="0" width="2" height="2">
        <feFlood flood-color="blue"/>
      </filter>
      <rect width="100" height="100" fill="red" filter="url(#f1) url(#f2)"/>
    </svg>"#;
    assert!(decode_image(hostile.as_bytes(), 1_000_000).is_none());
    // Un SVG sain au même budget doit toujours décoder.
    assert!(decode_image(GREEN_SVG.as_bytes(), 1_000_000).is_some());
}

#[test]
fn svg_malforme_est_none() {
    assert!(decode_image(b"<svg><rect", 1_000_000).is_none());
    // HTML portant un svg inline : root non-SVG → refus (un
    // `<img>` ne sert pas un document HTML).
    assert!(decode_image(b"<html><body><svg width=10></svg></body></html>", 1_000_000).is_none());
}

/// Fonte absente — le test image n'a pas de texte à shaper.
struct NullFonts;
impl explorer_layout::font::FontSource for NullFonts {
    fn font_data(&self, _req: &explorer_layout::font::FontRequest) -> Option<Vec<u8>> {
        None
    }
}
