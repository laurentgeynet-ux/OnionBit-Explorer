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
use explorer_render::{
    build_display_list, build_display_list_full, collect_data_images, rasterize, Frame,
};

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
    // R-32 : marge UA `body { margin: 8px }` neutralisée —
    // fixtures écrites pour une géométrie à marge nulle.
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
    // Au-delà de la boîte : la canvas blanche du document.
    assert_eq!(px(&f, 10, 190), [255, 255, 255, 255]);
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
    // Centre : pas de bordure, pas de fond → canvas blanche.
    assert_eq!(px(&f, 20, 20), [255, 255, 255, 255]);
}

#[test]
fn hairline_fractionnaire_ne_panique_pas() {
    // Régression duckduckgo.com (étape 48) : bordure de 1 px
    // tombant sur une frontière fractionnaire — les bords
    // sub-pixel AA couvrent toute la largeur → bug tiny-skia
    // `hairline_aa/fill_dot8` (`debug_assert`, panic en
    // debug). `fill_guarded` pixel-snappe les arêtes pleines :
    // la hairline devient une colonne nette, jamais un panic.
    let f = frame(
        "<div id=d></div>",
        "#d{border-left:1px solid #00ff00;margin-left:0.4px;\
         width:40px;height:40px}",
    );
    // La bordure ~x=0,4 est snappée sur la colonne x=0 —
    // pleine et nette (comportement des moteurs établis).
    let peinte = (0..f.height.min(40)).all(|y| px(&f, 0, y)[1] == 255);
    assert!(peinte, "bordure hairline non peinte");
}

#[test]
fn coordonnees_astronomiques_ne_paniquent_pas() {
    // Contenu hostile : géométrie au-delà du domaine
    // fixed-point 24.8 (±32 768 px) → clippe/ignorée par
    // `fill_guarded`, jamais panic. Avant le garde, ce cas
    // panic dans `hairline_aa` en build debug. Marge négative
    // géante : une marge positive s'échapperait jusqu'à la
    // racine et produirait une page >16 384 px légitime —
    // refusée en amont (borne `rasterize`), hors du cas visé.
    let f = frame(
        "<div id=d></div>",
        "#d{border:1px solid #f00;margin-top:-100000px;\
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
    assert_eq!(px(&f, 10, 30), [255, 255, 255, 255], "hors clip → canvas");
}

#[test]
fn texte_laisse_des_pixels() {
    let f = frame("<p>AAAA</p>", "p{margin:0;font-size:20px;color:#000000}");
    // De l'encre sombre quelque part dans la zone de texte
    // (la canvas est blanche — on cherche le dessin du
    // glyphe, pas juste de l'opacité).
    let mut found = false;
    'outer: for y in 0..30u32 {
        for x in 0..80u32 {
            let p = px(&f, x, y);
            if p[0] < 128 && p[3] > 128 {
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
    assert_eq!(px(&f, 20, 110), [255, 255, 255, 255]);
}

#[test]
fn png_encode_produit_des_octets() {
    let f = frame("<p>hi</p>", "p{margin:0}");
    let png = f.encode_png().expect("encodage png");
    assert_eq!(&png[..4], &[137, 80, 78, 71], "signature PNG");
}

// ── PNG streamé par bandes (R-37) ───────────────────────────

#[test]
fn png_bande_encode_to_identique_au_vec() {
    // `encode_png_to` (flux `Write` quelconque) et
    // `encode_png` (collecte `Vec`) produisent le même flux
    // d'octets.
    let f = frame(
        "<div id=d></div>",
        "#d{background-color:#123456;width:60px;height:60px}",
    );
    let via_vec = f.encode_png().expect("png");
    let mut direct = Vec::new();
    assert!(f.encode_png_to(&mut direct), "encode_png_to ok");
    assert_eq!(via_vec, direct);
}

#[test]
fn png_bande_multi_bandes_decodent_les_memes_pixels() {
    // Frame plus haute que la bande d'encodage (256 lignes)
    // → plusieurs `write_image_data` ; le décodage doit
    // rendre les pixels exacts de part et d'autre de la
    // frontière de bande.
    let f = frame(
        "<div id=a></div><div id=b></div>",
        "#a{background-color:#ff0000;width:40px;height:200px}\
         #b{background-color:#00ff00;width:40px;height:200px}",
    );
    assert!(f.height > 256, "fixture multi-bandes : {}", f.height);
    let png = f.encode_png().expect("png");
    let img = image::load_from_memory(&png)
        .expect("png décodable")
        .to_rgba8();
    assert_eq!(img.dimensions(), (f.width, f.height));
    assert_eq!(img.get_pixel(10, 100).0, [255, 0, 0, 255], "bande 1");
    assert_eq!(img.get_pixel(10, 300).0, [0, 255, 0, 255], "bande 2");
}

#[test]
fn png_bande_demultiplie_les_pixels() {
    // Prémultiplié → PNG non prémultiplié : alpha 0 → noir,
    // alpha partiel → canal remonté (128/128 → 255).
    let mut f = Frame {
        width: 2,
        height: 1,
        pixels: vec![0; 8],
        scale: 1.0,
    };
    f.pixels[..4].copy_from_slice(&[128, 0, 0, 128]);
    f.pixels[4..8].copy_from_slice(&[9, 9, 9, 0]);
    let png = f.encode_png().expect("png");
    let img = image::load_from_memory(&png)
        .expect("png décodable")
        .to_rgba8();
    assert_eq!(img.get_pixel(0, 0).0, [255, 0, 0, 128]);
    assert_eq!(img.get_pixel(1, 0).0, [0, 0, 0, 0]);
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
    assert_eq!(px(&f, 10, 10), [255, 255, 255, 255], "origine = canvas");
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
    assert_eq!(px(&f, 35, 25), [255, 255, 255, 255], "hors portée = canvas");
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

/// R-52 — motif « icône colorée » : `mask-image` + `background-color`
/// ne peint la couleur que dans l'alpha du mask (sinon carré plein).
#[test]
fn mask_image_borne_le_fond() {
    let dom: Dom = TreeBuilder::parse("<span class=icon></span>");
    let mut eng = StyleEngine::new();
    eng.add_css("body{margin:0}", Origin::Author);
    // Mask = svg 20×20 dont seule la moitié gauche est opaque :
    // le fond rouge ne doit couvrir que x∈[0,10).
    eng.add_css(
        ".icon{display:inline-block;width:20px;height:20px;\
         mask-image:url('data:image/svg+xml;utf8,<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"20\" height=\"20\"><rect x=\"0\" y=\"0\" width=\"10\" height=\"20\" fill=\"black\"/></svg>');\
         background-color:#ff0000}",
        Origin::Author,
    );
    let styles = eng.compute_tree(&dom, dom.document(), &StyleEnv::default());
    let root_el = dom
        .children(dom.document())
        .into_iter()
        .flatten()
        .find(|&c| dom.element(c).is_ok())
        .expect("élément racine");
    let mut bx = build_box_tree(&dom, root_el, &styles).expect("arbre");
    layout_tree(&mut bx, VIEW, &ctx());
    let imgs = collect_data_images(&dom, &styles, 1 << 20);
    let list = build_display_list_full(&bx, VIEW, 200.0, 0.0, &imgs);
    // Le fill plein a disparu de la liste au profit du masked.
    assert!(
        list.items
            .iter()
            .any(|i| matches!(i, explorer_render::DisplayItem::MaskedFill { .. })),
        "MaskedFill attendu, got {}",
        list.dump()
    );
    let f = rasterize(&list, font_src(), 1.0).expect("frame allouée");
    assert_eq!(px(&f, 5, 10)[0], 255, "zone masquée opaque = rouge");
    assert_eq!(
        px(&f, 15, 10),
        [255, 255, 255, 255],
        "hors mask = canvas, pas de fuite du fond"
    );
}

/// Contrôle : `mask-image` déclaré mais bitmap indécodable →
/// le `background-color` se peint plein (repli tolérant).
#[test]
fn mask_image_indecodable_repli_fond_plein() {
    let dom: Dom = TreeBuilder::parse("<span class=icon></span>");
    let mut eng = StyleEngine::new();
    eng.add_css("body{margin:0}", Origin::Author);
    eng.add_css(
        ".icon{display:inline-block;width:20px;height:20px;\
         mask-image:url('data:image/svg+xml;utf8,<not-svg>');\
         background-color:#ff0000}",
        Origin::Author,
    );
    let styles = eng.compute_tree(&dom, dom.document(), &StyleEnv::default());
    let root_el = dom
        .children(dom.document())
        .into_iter()
        .flatten()
        .find(|&c| dom.element(c).is_ok())
        .expect("élément racine");
    let mut bx = build_box_tree(&dom, root_el, &styles).expect("arbre");
    layout_tree(&mut bx, VIEW, &ctx());
    let imgs = collect_data_images(&dom, &styles, 1 << 20);
    let list = build_display_list_full(&bx, VIEW, 200.0, 0.0, &imgs);
    let f = rasterize(&list, font_src(), 1.0).expect("frame allouée");
    assert_eq!(
        px(&f, 15, 10)[0],
        255,
        "mask indécodable → fond plein conservé"
    );
}
