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
    let list = build_display_list(&bx, VIEW, 200.0, 0.0);
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
fn hairline_fractionnaire_ne_panique_pas() {
    // Régression duckduckgo.com (étape 48) : bordure de 1 px
    // tombant sur une frontière fractionnaire — les bords
    // sub-pixel AA couvrent toute la largeur → bug tiny-skia
    // `hairline_aa/fill_dot8` (`debug_assert`, panic en
    // debug). Le garde `fill_guarded` doit router par
    // `fill_path` : rasterise sans panic et peint l'arête.
    let f = frame(
        "<div id=d></div>",
        "#d{border-left:1px solid #00ff00;margin-left:0.4px;\
         width:40px;height:40px}",
    );
    // La bordure verticale ~x=0,4 a laissé des pixels verts
    // (deux arêtes AA : x=0 couvert ~0,6, x=1 ~0,4).
    let verts = (0..f.height.min(40)).any(|y| px(&f, 0, y)[1] > 100)
        && (0..f.height.min(40)).any(|y| px(&f, 1, y)[1] > 60);
    assert!(verts, "bordure hairline non peinte");
}

#[test]
fn coordonnees_astronomiques_ne_paniquent_pas() {
    // Contenu hostile : géométrie au-delà du domaine
    // fixed-point 24.8 (±32 768 px) → clippe/ignorée par
    // `fill_guarded`, jamais panic. Avant le garde, ce cas
    // panic dans `hairline_aa` en build debug.
    let f = frame(
        "<div id=d></div>",
        "#d{border:1px solid #f00;margin-top:100000px;\
         width:40px;height:40px}",
    );
    // L'assertion est l'absence de panic — la frame est
    // produite quoi qu'il arrive.
    assert!(f.width > 0);
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

// ── transform (étape 42) ──────────────────────────────────────

#[test]
fn transform_translate_deplace_les_pixels() {
    // Carré rouge 20×20 à (0,0), translaté de (60,30) — le
    // pixel rouge doit apparaître dans la zone translatée et
    // l'emplacement d'origine rester blanc.
    let f = frame(
        "<div id=t></div>",
        "body{margin:0} #t{background-color:#ff0000;width:20px;height:20px;\
         transform:translate(60px, 30px)}",
    );
    assert_eq!(px(&f, 70, 40)[0], 255, "rouge translaté");
    assert_eq!(px(&f, 10, 10)[0], 0, "origine vide");
}

#[test]
fn transform_scale_grossit_la_zone_peinte() {
    // 20×20 scalé ×2 depuis le centre (10,10) : couvre
    // [−10,30) sur les deux axes → (25,25) rouge, (35,25)
    // hors portée reste blanc.
    let f = frame(
        "<div id=t></div>",
        "body{margin:0} #t{background-color:#ff0000;width:20px;height:20px;\
         transform:scale(2)}",
    );
    assert_eq!(px(&f, 25, 25)[0], 255, "scalé couvre");
    assert_eq!(px(&f, 35, 25)[0], 0, "hors portée");
}

#[test]
fn transform_rotate_peint_hors_bbox_alignee() {
    // 40×10 tourné de 90° autour du centre (20,5) → couvre
    // la colonne x∈[15,25) y∈[-10,20) : (20,15) rouge alors
    // qu'une bbox alignée ne l'atteindrait pas.
    let f = frame(
        "<div id=t></div>",
        "body{margin:0} #t{background-color:#ff0000;width:40px;height:10px;\
         transform:rotate(90deg)}",
    );
    assert_eq!(px(&f, 20, 15)[0], 255, "rotation peinte");
}
