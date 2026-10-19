// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Tests du raster (étape 23) : framebuffer RGBA réel —
//! pixels sondés pour fonds/bordures/clips/DPI, glyphes
//! dessinés depuis les contours TTF de la fixture.

use explorer_css::{Origin, StyleEngine, StyleEnv};
use explorer_dom::Dom;
use explorer_html::TreeBuilder;
use explorer_layout::font::{FontRequest, FontSource};
use explorer_layout::{build_box_tree, layout_tree, BlockCtx};
use explorer_render::{build_display_list, rasterize, Frame};

const VIEW: f32 = 200.0;

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

fn font_src() -> &'static Fixture {
    let data = explorer_test_support::fixture_ttf();
    Box::leak(Box::new(Fixture(data)))
}

fn frame(html: &str, css: &str) -> Frame {
    frame_at(html, css, 1.0)
}

fn frame_at(html: &str, css: &str, scale: f32) -> Frame {
    let dom: Dom = TreeBuilder::parse(html);
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
    let list = build_display_list(&bx, VIEW, 200.0);
    rasterize(&list, font_src(), scale).expect("frame allouée")
}

/// Pixel RGBA à (x, y) — prémultiplié.
fn px(f: &Frame, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * f.width + x) * 4) as usize;
    [
        f.pixels[i],
        f.pixels[i + 1],
        f.pixels[i + 2],
        f.pixels[i + 3],
    ]
}

#[test]
fn fond_remplit_le_framebuffer() {
    let f = frame(
        "<div id=d></div>",
        "#d{background-color:#ff0000;width:200px;height:60px}",
    );
    assert_eq!(px(&f, 10, 10), [255, 0, 0, 255]);
    // Au-delà de la boîte : transparent (fond du document).
    assert_eq!(px(&f, 10, 190)[3], 0);
}

#[test]
fn bordure_peint_les_cotes() {
    let f = frame(
        "<div id=d></div>",
        "#d{border:4px solid #00ff00;width:40px;height:40px}",
    );
    // Bord supérieur.
    assert_eq!(px(&f, 20, 1)[1], 255);
    // Bord gauche.
    assert_eq!(px(&f, 1, 20)[1], 255);
    // Centre : pas de bordure, pas de fond → transparent.
    assert_eq!(px(&f, 20, 20)[3], 0);
}

#[test]
fn clip_borne_le_dessin_enfant() {
    // overflow:hidden 20px ; enfant rouge de 60px → les pixels
    // sous 20px restent transparents.
    let f = frame(
        "<div id=o><div id=in></div></div>",
        "#o{overflow:hidden;width:100px;height:20px}\
         #in{background-color:#ff0000;width:100px;height:60px}",
    );
    assert_eq!(px(&f, 10, 10), [255, 0, 0, 255]);
    assert_eq!(px(&f, 10, 30)[3], 0, "hors clip → non peint");
}

#[test]
fn texte_laisse_des_pixels() {
    let f = frame("<p>AAAA</p>", "p{margin:0;font-size:20px;color:#000000}");
    // Des pixels noirs quelque part dans la zone de texte.
    let mut found = false;
    'outer: for y in 0..30u32 {
        for x in 0..80u32 {
            if px(&f, x, y)[3] > 0 {
                found = true;
                break 'outer;
            }
        }
    }
    assert!(found, "glyphes rasterisés attendus");
}

#[test]
fn dpi_double_la_taille_physique() {
    let f = frame_at(
        "<div id=d></div>",
        "#d{background-color:#0000ff;width:200px;height:50px}",
        2.0,
    );
    assert_eq!(f.width, 400);
    // Le fond bleu couvre 100px physiques (50 CSS × 2).
    assert_eq!(px(&f, 20, 90)[2], 255);
    assert_eq!(px(&f, 20, 110)[3], 0);
}

#[test]
fn png_encode_produit_des_octets() {
    let f = frame("<p>hi</p>", "p{margin:0}");
    let png = f.encode_png().expect("encodage png");
    assert_eq!(&png[..4], &[137, 80, 78, 71], "signature PNG");
}
