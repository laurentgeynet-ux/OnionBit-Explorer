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

/// Source deux fontes (R-69) : latin primaire + CJK servie
/// par `fallback_for` — la plage `0x65E0-0x65FF` couvre 日.
struct TwoFonts {
    latin: std::sync::Arc<Vec<u8>>,
    cjk: std::sync::Arc<Vec<u8>>,
    with_fb: bool,
}
impl FontSource for TwoFonts {
    fn font_data(&self, _req: &FontRequest) -> Option<Vec<u8>> {
        Some((*self.latin).clone())
    }
    fn font_face(&self, _req: &FontRequest) -> Option<(std::sync::Arc<Vec<u8>>, u32)> {
        Some((self.latin.clone(), 0))
    }
    fn fallback_for(&self, c: char, _req: &FontRequest) -> Option<(std::sync::Arc<Vec<u8>>, u32)> {
        (self.with_fb && (0x65E0..=0x65FF).contains(&(c as u32))).then(|| (self.cjk.clone(), 0))
    }
}

fn two_fonts(with_fb: bool) -> &'static TwoFonts {
    Box::leak(Box::new(TwoFonts {
        latin: std::sync::Arc::new(explorer_test_support::fixture_ttf()),
        cjk: std::sync::Arc::new(explorer_test_support::fixture_ttf_custom(
            "Cjk", 0x65E0, 0x65FF,
        )),
        with_fb,
    }))
}

fn frame_fonts(html: &str, css: &str, fonts: &'static dyn FontSource) -> Frame {
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
    layout_tree(
        &mut bx,
        VIEW,
        &BlockCtx {
            fonts: Some(fonts),
            ..BlockCtx::default()
        },
    );
    let list = build_display_list(&bx, VIEW, 200.0, 0.0);
    rasterize(&list, fonts, 1.0).expect("frame allouée")
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

// ── R-86 : masks sur pseudos + mask-size/position/repeat ────

/// Mask porté par un `::before` — la boîte pseudo n'a pas de
/// `NodeId` : le lookup passe par `masks_url` (texte `url(...)`
/// du style calculé). Motif `.x::before` des icônes MDN.
#[test]
fn mask_sur_pseudo_via_url() {
    let dom: Dom = TreeBuilder::parse("<span class=x></span>");
    let mut eng = StyleEngine::new();
    eng.add_css("body{margin:0}", Origin::Author);
    // Mask 20×20 moitié gauche opaque, comme le test R-52 —
    // porté par le pseudo : seule la moitié gauche est rouge.
    eng.add_css(
        ".x::before{content:\"\";display:inline-block;width:20px;height:20px;\
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
    assert!(
        list.items
            .iter()
            .any(|i| matches!(i, explorer_render::DisplayItem::MaskedFill { .. })),
        "MaskedFill attendu sur pseudo, got {}",
        list.dump()
    );
    let f = rasterize(&list, font_src(), 1.0).expect("frame allouée");
    assert_eq!(px(&f, 5, 10)[0], 255, "mask pseudo : zone opaque rouge");
    assert_eq!(
        px(&f, 15, 10),
        [255, 255, 255, 255],
        "mask pseudo : hors alpha = canvas"
    );
}

/// `mask-size` + `mask-position` + `mask-repeat:no-repeat`
/// résolus à l'émission : la tuile est placée, pas étirée.
#[test]
fn mask_size_position_repeat_placent_la_tuile() {
    let dom: Dom = TreeBuilder::parse("<div class=x></div>");
    let mut eng = StyleEngine::new();
    eng.add_css("body{margin:0}", Origin::Author);
    // Tuile 10×10 (moitié gauche opaque → 5 px rouges) centrée
    // dans la boîte 20×20, sans répétition : rouge en
    // x∈[5,10) y∈[5,15), canvas partout ailleurs.
    eng.add_css(
        ".x{display:block;width:20px;height:20px;\
         mask-image:url('data:image/svg+xml;utf8,<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"20\" height=\"20\"><rect x=\"0\" y=\"0\" width=\"10\" height=\"20\" fill=\"black\"/></svg>');\
         mask-size:10px;mask-position:center;mask-repeat:no-repeat;\
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
    assert_eq!(px(&f, 7, 10)[0], 255, "tuile centrée : moitié opaque");
    assert_eq!(
        px(&f, 12, 10),
        [255, 255, 255, 255],
        "no-repeat : hors tuile = rien (pas de fuite du fond)"
    );
    assert_eq!(
        px(&f, 2, 10),
        [255, 255, 255, 255],
        "no-repeat : avant la tuile = rien"
    );
}

/// `mask-repeat` défaut = tuilage : `mask-size:50%` sans
/// repeat explicite recouvre la boîte en tuiles.
#[test]
fn mask_repeat_defaut_u_tile() {
    let dom: Dom = TreeBuilder::parse("<div class=x></div>");
    let mut eng = StyleEngine::new();
    eng.add_css("body{margin:0}", Origin::Author);
    // Tuile 10×10 (moitié gauche opaque → 5 px rouges par
    // tuile) répétée sur la boîte 20×20 : x∈{0..5,10..15}
    // rouges, x∈{5..10,15..20} blancs.
    eng.add_css(
        ".x{display:block;width:20px;height:20px;\
         mask-image:url('data:image/svg+xml;utf8,<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"20\" height=\"20\"><rect x=\"0\" y=\"0\" width=\"10\" height=\"20\" fill=\"black\"/></svg>');\
         mask-size:50%;\
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
    assert_eq!(px(&f, 12, 10)[0], 255, "deuxième tuile = opaque aussi");
    assert_eq!(
        px(&f, 17, 10),
        [255, 255, 255, 255],
        "interstice de tuile = canvas"
    );
}

/// `register_mask` — couture fetch : le mask `url(http…)`
/// enregistré par l'appelant borne le fond comme un `data:`.
#[test]
fn mask_fetchem_enregistre_via_register_mask() {
    let dom: Dom = TreeBuilder::parse("<div class=x></div>");
    let mut eng = StyleEngine::new();
    eng.add_css("body{margin:0}", Origin::Author);
    eng.add_css(
        ".x{display:block;width:20px;height:20px;\
         mask-image:url(http://example.test/icon.svg);\
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
    // L'appelant (NAV) enregistre le bitmap sous le texte
    // `url(...)` brut du style calculé.
    let mut imgs = explorer_render::PaintImages::default();
    imgs.register_mask(
        "http://example.test/icon.svg",
        std::sync::Arc::new(explorer_render::DecodedImage {
            width: 20,
            height: 20,
            rgba: (0..20 * 20)
                .flat_map(|i| {
                    let x = i % 20;
                    if x < 10 {
                        [0, 0, 0, 255]
                    } else {
                        [0, 0, 0, 0]
                    }
                })
                .collect(),
        }),
    );
    let list = build_display_list_full(&bx, VIEW, 200.0, 0.0, &imgs);
    assert!(
        list.items
            .iter()
            .any(|i| matches!(i, explorer_render::DisplayItem::MaskedFill { .. })),
        "mask enregistré → MaskedFill, got {}",
        list.dump()
    );
    let f = rasterize(&list, font_src(), 1.0).expect("frame allouée");
    assert_eq!(px(&f, 5, 10)[0], 255, "mask fetché : opaque rouge");
    assert_eq!(
        px(&f, 15, 10),
        [255, 255, 255, 255],
        "mask fetché : transparent = canvas"
    );
}

// ── R-58 : clip-path inset borne les pixels ─────────────────

#[test]
fn clip_path_inset_ne_peint_que_le_centre() {
    // `clip-path:inset(25%)` sur une boîte 20×20 fond rouge →
    // seul le carré central 10×10 peint ; les coins restent au
    // fond de page (blanc). (`inset(50%)` = clip 0×0 — le
    // motif `.visuallyHidden` GitHub masque tout.)
    let f = frame(
        "<div id=s></div>",
        "#s{position:absolute;left:0;top:0;width:20px;height:20px;\
         background:#ff0000;clip-path:inset(25%)}",
    );
    assert_eq!(px(&f, 10, 10), [255, 0, 0, 255], "centre = rouge");
    assert_eq!(px(&f, 2, 2), [255, 255, 255, 255], "coin = blanc");
    assert_eq!(px(&f, 18, 18), [255, 255, 255, 255], "coin opposé");
}

#[test]
fn clip_rect_zero_ne_peint_rien() {
    // `clip:rect(0,0,0,0)` : clip 0×0 → aucun pixel du fond.
    let f = frame(
        "<div id=s></div>",
        "#s{position:absolute;left:0;top:0;width:20px;height:20px;\
         background:#ff0000;clip:rect(0,0,0,0)}",
    );
    for (x, y) in [(0, 0), (10, 10), (19, 19)] {
        assert_eq!(px(&f, x, y), [255, 255, 255, 255], "({x},{y}) blanc");
    }
}

// ── R-62 : rasterize_window — tranche verticale ─────────────

fn list_of(html: &str, css: &str, doc_h: f32) -> explorer_render::DisplayList {
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
    build_display_list(&bx, VIEW, doc_h, 0.0)
}

#[test]
fn fenetre_raster_equivaut_aux_lignes_du_plein_cadre() {
    // Fenêtre [30,50) ≡ lignes 30..50 du plein cadre — même
    // mécanique de filtre/translation que rasterize_dirty.
    let html = "<div id=a></div><div id=b></div>";
    let css = "#a{background:#ff0000;width:200px;height:50px}\
               #b{background:#0000ff;width:200px;height:50px}";
    let list = list_of(html, css, 200.0);
    let full = rasterize(&list, font_src(), 1.0).expect("plein cadre");
    let win =
        explorer_render::rasterize_window(&list, font_src(), 1.0, 30.0, 50.0).expect("fenêtre");
    assert_eq!((win.width, win.height), (full.width, 50));
    for y in 0..50 {
        for x in [0, 100, 199] {
            assert_eq!(px(&win, x, y), px(&full, x, y + 30), "ligne {y}");
        }
    }
}

#[test]
fn fenetre_raster_rasterise_sous_le_cap_document() {
    // Document 20 000 px : `rasterize` refuse (borne), la
    // fenêtre [19 900, +100) peint quand même la tranche.
    let list = list_of(
        "<div id=d></div>",
        "#d{background:#00ff00;width:200px;height:20000px}",
        20_000.0,
    );
    assert!(
        rasterize(&list, font_src(), 1.0).is_none(),
        "document sous cap"
    );
    let win = explorer_render::rasterize_window(&list, font_src(), 1.0, 19_900.0, 100.0)
        .expect("fenêtre sous le cap");
    assert_eq!(px(&win, 10, 10), [0, 255, 0, 255], "tranche peinte");
    // Fenêtre hors document → blanc (scroll de fin de page).
    let off = explorer_render::rasterize_window(&list, font_src(), 1.0, 30_000.0, 50.0)
        .expect("fenêtre hors doc");
    assert_eq!(px(&off, 10, 10), [255, 255, 255, 255], "blanc");
}

// ── R-69 : repli par glyphe jusqu'aux pixels ────────────────

#[test]
fn fallback_peint_le_glyphe_de_repli() {
    // "A日" : '日' absent de la fixture latine → span de
    // repli dans le run → contour de la face CJK → encre à
    // la position du deuxième glyphe (avance 10 px à 20 px).
    let f = frame_fonts(
        "<p>A日</p>",
        "p{margin:0;font-size:20px;color:#000000}",
        two_fonts(true),
    );
    let p = px(&f, 15, 10);
    assert!(p[0] < 128 && p[3] > 128, "repli peint, got {p:?}");
    // Contrôle : le 'A' latin peint pareil en tête.
    let a = px(&f, 5, 10);
    assert!(a[0] < 128 && a[3] > 128, "'A' peint, got {a:?}");
}

#[test]
fn sans_fallback_notdef_ne_peint_rien() {
    // Même scène, `fallback_for` → `None` : '日' reste
    // `.notdef` (contour vide de la fixture) → blanc à la
    // position du glyphe — le premier reste peint.
    let f = frame_fonts(
        "<p>A日</p>",
        "p{margin:0;font-size:20px;color:#000000}",
        two_fonts(false),
    );
    let a = px(&f, 5, 10);
    assert!(a[0] < 128 && a[3] > 128, "'A' peint, got {a:?}");
    assert_eq!(px(&f, 15, 10), [255, 255, 255, 255], "notdef vide");
}

// ── atlas de glyphes rendus (R-85) ────────────────────────────

/// Empreinte des pixels d'encre d'une bande `(y0, y1)` —
/// positions normalisées au min (x, y) de la zone : deux
/// bandes contenant le même texte doivent donner la même
/// empreinte (la couverture snapée est position-
/// indépendante — c'est l'invariant qui rend l'atlas
/// correct).
fn empreinte_encre(f: &Frame, y0: u32, y1: u32) -> Vec<(u32, u32)> {
    let mut pts = Vec::new();
    for y in y0..y1 {
        for x in 0..f.width {
            let p = px(f, x, y);
            if p[0] < 200 && p[3] > 64 {
                pts.push((x, y));
            }
        }
    }
    let (mx, my) = pts.iter().fold((u32::MAX, u32::MAX), |(ax, ay), &(x, y)| {
        (ax.min(x), ay.min(y))
    });
    pts.iter().map(|&(x, y)| (x - mx, y - my)).collect()
}

#[test]
fn atlas_meme_texte_meme_empreinte_a_deux_positions() {
    // Deux paragraphes au texte identique, positions
    // différentes → bitmaps d'encre identiques à la
    // translation près (le blit atlas reproduit exactement
    // la couverture du `fill_path` historique).
    let f = frame(
        "<p>HH</p><p style=\"margin-top:40px\">HH</p>",
        "p{margin:0;font-size:20px;color:#000000}",
    );
    let a = empreinte_encre(&f, 0, 30);
    let b = empreinte_encre(&f, 40, 100);
    assert!(!a.is_empty(), "encre attendue en bande haute");
    assert_eq!(a, b, "même texte → même empreinte d'encre");
}

#[test]
fn atlas_repaint_a_chaud_reproduit_la_frame() {
    // Deux passes avec le même `GlyphCache` : la passe
    // chaude ne fait que des blits depuis l'atlas — le
    // résultat doit être BIT-IDENTIQUE à la passe froide
    // (le cache introduit de la mémoire, pas de la
    // divergence).
    let html = "<p>atlas chaud</p>";
    let css = "p{margin:0;font-size:20px;color:#000000}";
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
    let list = build_display_list(&bx, VIEW, 200.0, 0.0);
    let src = font_src();
    let mut gc = explorer_render::GlyphCache::new();
    let froid = explorer_render::rasterize_cached(&list, src, 1.0, &mut gc).expect("frame froide");
    let chaud = explorer_render::rasterize_cached(&list, src, 1.0, &mut gc).expect("frame chaude");
    assert_eq!(froid.pixels, chaud.pixels, "atlas : repaint divergent");
}

// ── B-32 : <img> redimensionné en bilinéaire ────────────────

#[test]
fn image_redimensionnee_echantillonne_en_bilineaire() {
    // Source 4x1 : noir, blanc, noir, blanc (opaque). Réduite
    // à 2 px de large, le nearest-neighbor historique rendait
    // [noir, noir] (texels 0 et 2) ; le bilinéaire doit
    // produire deux pixels ~127 (moyenne des paires).
    let img = std::sync::Arc::new(explorer_render::DecodedImage {
        width: 4,
        height: 1,
        rgba: vec![
            0, 0, 0, 255, 255, 255, 255, 255, 0, 0, 0, 255, 255, 255, 255, 255,
        ],
    });
    let list = explorer_render::DisplayList {
        items: vec![explorer_render::DisplayItem::Image {
            rect: explorer_layout::Rect {
                x: 0.0,
                y: 0.0,
                width: 2.0,
                height: 1.0,
            },
            img,
        }],
        width: 2.0,
        height: 1.0,
        spans: std::collections::HashMap::default(),
        tree_epoch: 0,
        dropped_emits: 0,
        first_cycle_path: None,
    };
    let f = rasterize(&list, font_src(), 1.0).expect("frame allouée");
    for x in 0..2 {
        let p = px(&f, x, 0);
        assert!(
            (110..=145).contains(&p[0]),
            "pixel {x} attendu ~127 (bilineaire), got {p:?}"
        );
        assert_eq!(p[0], p[1]);
        assert_eq!(p[1], p[2]);
        assert_eq!(p[3], 255);
    }
}
