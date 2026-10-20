// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! R-36 — cache glyphes cross-passe : `rasterize_cached` /
//! `rasterize_dirty_cached` réutilisent faces parsées et
//! contours entre repaints. La garde d'équivalence est la
//! même que `dirty_raster` : pixels strictement identiques
//! entre passe cachée « chaude » et passe à cache neuf —
//! `Path::transform` applique le même `map_point` par point
//! que le sink transformé historique.
//!
//! L'invalidation `FontSource::revision` est exercée de
//! bout en bout : une face `@font-face` inscrite entre deux
//! repaints vide le cache (sinon la famille « inconnue »
//! resterait figée sur le repli).

use std::sync::Arc;

use explorer_css::{Origin, StyleEngine, StyleEnv};
use explorer_html::TreeBuilder;
use explorer_layout::font::{FontRequest, FontSource};
use explorer_layout::{build_box_tree, layout_tree, BlockCtx};
use explorer_render::{
    build_display_list, rasterize, rasterize_cached, rasterize_dirty_cached, DisplayList,
    GlyphCache, WebFontSource,
};

const VIEW: f32 = 400.0;
const DOC_H: f32 = 300.0;

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

/// Display list d'un document posé — mêmes neutralisations
/// que le harnais `dirty_raster`.
fn list_of(html: &str, css: &str) -> DisplayList {
    let dom = TreeBuilder::parse(html);
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
    let mut list = build_display_list(&bx, VIEW, DOC_H, 0.0);
    list.width = VIEW;
    list
}

fn fonts() -> &'static dyn FontSource {
    ctx().fonts.expect("fonte")
}

#[test]
fn cache_chaud_pixels_identiques_a_cache_neuf() {
    // Texte multi-mots : plusieurs glyphes distincts → les
    // contours passent en cache à la 1re passe, réutilisés
    // à la 2e — la frame doit rester strictement identique.
    let list = list_of(
        "<p>lorem ipsum dolor sit amet</p>",
        "body{margin:0} p{font-size:20px}",
    );
    let mut gc = GlyphCache::new();
    let f1 = rasterize_cached(&list, fonts(), 1.0, &mut gc).expect("passe 1");
    assert!(gc.faces_len() > 0, "la face a été retenue");
    assert!(gc.paths_len() > 0, "des contours ont été retenus");
    let f2 = rasterize_cached(&list, fonts(), 1.0, &mut gc).expect("passe chaude");
    let fresh = rasterize(&list, fonts(), 1.0).expect("cache neuf");
    assert_eq!(f1.pixels, fresh.pixels, "passe froide ≠ cache neuf");
    assert_eq!(f2.pixels, fresh.pixels, "passe chaude ≠ cache neuf");
}

#[test]
fn dirty_cache_chaud_identique_au_plein_cadre() {
    // Même garde que `dirty_raster`, en exerçant la variante
    // à cache persistant (fenêtres régionales + cache chaud).
    let l1 = list_of(
        "<div></div><p>alpha beta</p>",
        "body{margin:0} div{height:50px;background-color:#f00} p{font-size:20px}",
    );
    let l2 = list_of(
        "<div></div><p>alpha beta</p>",
        "body{margin:0} div{height:50px;background-color:#00f} p{font-size:20px}",
    );
    let mut gc = GlyphCache::new();
    let f_prev = rasterize_cached(&l1, fonts(), 1.0, &mut gc).expect("prev");
    let f_next_full = rasterize(&l2, fonts(), 1.0).expect("plein");
    let damage = l2.changed_rects(&l1);
    assert!(!damage.is_empty(), "le scénario produit du dirty");
    let f_next_dirty = rasterize_dirty_cached(&l2, fonts(), 1.0, f_prev, &damage, &mut gc)
        .expect("régional caché");
    assert_eq!(
        f_next_dirty.pixels, f_next_full.pixels,
        "régional caché ≠ plein cadre"
    );
}

/// Source vide — le repli de `WebFontSource` ne répond
/// jamais (révision figée `0`).
struct EmptyFont;
impl FontSource for EmptyFont {
    fn font_data(&self, _req: &FontRequest) -> Option<Vec<u8>> {
        None
    }
}

#[test]
fn revision_change_vide_le_cache() {
    // Registre vide → miss figé en cache.
    let mut web = WebFontSource::new(Arc::new(EmptyFont), 4);
    let list = list_of("<p>abcdef</p>", "body{margin:0} p{font-size:20px}");
    let mut gc = GlyphCache::new();
    let miss = rasterize_cached(&list, &web, 1.0, &mut gc).expect("frame sans face");
    assert_eq!(gc.faces_len(), 1, "le miss est retenu (figé)");
    // Une face `@font-face` arrive entre deux repaints :
    // `register` bumpe la révision → le cache est vidé au
    // sync suivant, la face fraîche est résolue.
    assert!(web.register(
        "sans-serif",
        400,
        false,
        explorer_test_support::fixture_ttf()
    ));
    let hit = rasterize_cached(&list, &web, 1.0, &mut gc).expect("frame avec face");
    assert!(gc.paths_len() > 0, "les contours ont re-peuplé");
    assert_ne!(miss.pixels, hit.pixels, "la nouvelle face peint");
}

/// Mesure R-36 à la demande — page dense en texte : la
/// différence fraîche/chaude isole le coût résolution+parse+
/// outline retiré par le cache (le fill scanline, dominé
/// par tiny-skia, reste).
/// `cargo test -p explorer-render --test glyph_cache perf -- --ignored --nocapture`
#[test]
#[ignore = "mesure R-36, à la demande"]
fn perf_cache_chaud_vs_neuf() {
    use std::fmt::Write;
    let mut html = String::new();
    for i in 0..40 {
        let _ = write!(
            html,
            "<p>ligne {i} de texte assez longue pour shaper</p>"
        );
    }
    let list = list_of(&html, "body{margin:0} p{font-size:14px;margin:2px 0}");
    let fonts = fonts();
    let n = 20;
    let t = std::time::Instant::now();
    for _ in 0..n {
        std::hint::black_box(rasterize(&list, fonts, 1.0));
    }
    let fresh = t.elapsed();
    let mut gc = GlyphCache::new();
    std::hint::black_box(rasterize_cached(&list, fonts, 1.0, &mut gc));
    let t = std::time::Instant::now();
    for _ in 0..n {
        std::hint::black_box(rasterize_cached(&list, fonts, 1.0, &mut gc));
    }
    let warm = t.elapsed();
    eprintln!("neuf   : {fresh:?} ({:?}/passe)", fresh / n);
    eprintln!("chaude : {warm:?} ({:?}/passe)", warm / n);
}

#[test]
fn meme_requete_meme_face() {
    // Deux requêtes identiques partagent la `OwnedFace`
    // (Arc) — les contours restent indexés dessus.
    let mut gc = GlyphCache::new();
    let req = FontRequest {
        families: vec!["x".into()],
        weight: 400,
        italic: false,
    };
    let a = gc.face(fonts(), &req).expect("face");
    let b = gc.face(fonts(), &req).expect("face");
    assert!(Arc::ptr_eq(&a, &b), "même face partagée");
    assert_eq!(gc.faces_len(), 1);
}
