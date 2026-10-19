// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Peinture progressive (R-1) : la chaîne
//! `style → boîtes → layout → display list → raster` tolère un
//! DOM **en cours de construction** alimenté par
//! [`TreeBuilder::push`]. Chaque frame reflète un état cohérent
//! à frontière de token — jamais un arbre transitoire — et la
//! dernière frame est pixel-à-pixel identique à celle du
//! document complet. La cadence de peinture relève de la
//! session ; ici on peint à **chaque** push pour couvrir
//! toutes les frontières.

use explorer_css::{Origin, StyleEngine, StyleEnv};
use explorer_dom::Dom;
use explorer_html::TreeBuilder;
use explorer_layout::font::{FontRequest, FontSource};
use explorer_layout::{build_box_tree, layout_tree, BlockCtx, LayoutBox};
use explorer_render::{build_display_list, rasterize, Frame};

const VIEW_W: f32 = 200.0;
const VIEW_H: f32 = 200.0;

struct Fixture(Vec<u8>);
impl FontSource for Fixture {
    fn font_data(&self, _req: &FontRequest) -> Option<Vec<u8>> {
        Some(self.0.clone())
    }
}

const CSS: &str = "body{margin:8px} h1{color:#c00000} \
    .f{float:left;width:40px;height:40px;background:#0000ff} \
    td{border:1px solid #000000} p{margin:4px} .late{color:#00c000}";

/// Page coupée en chunks volontairement hostiles : tag, entité,
/// rawtext `<style>`/`<script>`, table, attribut `alt` —
/// chaque frontière tombe au milieu d'une construction.
const CHUNKS: &[&str] = &[
    "<!DOCTYPE html><htm",
    "l><head><title>Tit",
    "re</title><style>.late{col",
    "or:#00c000}</style><script>var a=1<",
    "2;</script></head><body><h1>Titre</h",
    "1><p>Premier <b>gras <i>ital",
    "ique</i></b> paragraphe.</p><div class=f></d",
    "iv><p>Après le flottant <a href=#x>lien</a>.</p><tab",
    "le><tr><td>cellule</td></tr></table><ul><li>un<li>d",
    "eux</ul><img src=x.png alt=vign",
    "ette><p class=late>dernier &am",
    "p; fin</p></body></html>",
];

/// Peint l'état courant du DOM — même enchaînement que la
/// session : élément racine éventuel → boîtes → layout →
/// display list → raster.
///
/// `None` tant que le document n'a pas d'élément racine
/// (préambule seul — tag `<html>` pas encore complet) : rien à
/// peindre n'est pas une erreur.
fn paint(dom: &Dom) -> Option<Frame> {
    let bx = box_tree_of(dom)?;
    let list = build_display_list(&bx, VIEW_W, VIEW_H, 0.0);
    let fonts = Fixture(explorer_test_support::fixture_ttf());
    rasterize(&list, &fonts, 1.0)
}

/// Arbre de boîtes posé, ou `None` — cf. [`paint`].
fn box_tree_of(dom: &Dom) -> Option<LayoutBox> {
    let mut eng = StyleEngine::new();
    eng.add_css(CSS, Origin::Author);
    let styles = eng.compute_tree(dom, dom.document(), &StyleEnv::default());
    let mut bx = build_box_tree(dom, dom.document_element()?, &styles)?;
    let fonts = Fixture(explorer_test_support::fixture_ttf());
    layout_tree(
        &mut bx,
        VIEW_W,
        &BlockCtx {
            fonts: Some(&fonts),
            ..BlockCtx::default()
        },
    );
    Some(bx)
}

#[test]
fn chaque_push_produit_une_frame_coherente() {
    let mut tb = TreeBuilder::new();
    let mut frames = 0;
    for chunk in CHUNKS {
        tb.push(chunk);
        if let Some(f) = paint(tb.dom()) {
            frames += 1;
            assert!(f.width > 0 && f.height > 0);
            assert_eq!(
                f.pixels.len() as u64,
                u64::from(f.width) * u64::from(f.height) * 4
            );
        }
    }
    tb.finish();
    let _final = paint(tb.dom()).expect("frame finale");
    // Le `<html>` se complète dès le chunk 1 : les frames utiles
    // sont nombreuses bien avant la fin du flux.
    assert!(frames >= 5, "frames intermédiaires : {frames}");
}

#[test]
fn le_contenu_peint_apparait_au_fil_des_chunks() {
    let mut tb = TreeBuilder::new();
    // Préambule seul (tag `<html>` coupé) : rien à peindre.
    tb.push(CHUNKS[0]);
    assert!(paint(tb.dom()).is_none());
    // Après `<h1>Titre` (texte inséré sans attendre `</h1>`) :
    // la boîte texte existe déjà, la table pas encore.
    for chunk in &CHUNKS[1..5] {
        tb.push(chunk);
    }
    let bx = box_tree_of(tb.dom()).expect("boîtes intermédiaires");
    let dump = bx.dump(tb.dom());
    assert!(dump.contains("Titre"), "h1 attendu dans :\n{dump}");
    assert!(!dump.contains("cellule"), "table pas encore arrivée");
    // Fin du flux : tout le contenu est là, `&amp;` décodé.
    for chunk in &CHUNKS[5..] {
        tb.push(chunk);
    }
    tb.finish();
    let bx = box_tree_of(tb.dom()).expect("boîtes finales");
    let dump = bx.dump(tb.dom());
    assert!(dump.contains("cellule"), "cellule attendue dans :\n{dump}");
    assert!(dump.contains("dernier & fin"));
    // `<img>` non chargé → repli `alt` (dégradation documentée).
    assert!(dump.contains("vignette"), "alt attendu dans :\n{dump}");
}

#[test]
fn la_derniere_frame_est_identique_au_parse_complet() {
    let mut tb = TreeBuilder::new();
    for chunk in CHUNKS {
        tb.push(chunk);
    }
    tb.finish();
    let progressif = paint(tb.dom()).expect("frame progressive");
    let page = CHUNKS.concat();
    let one_shot = paint(&TreeBuilder::parse(&page)).expect("frame one-shot");
    assert_eq!(progressif.width, one_shot.width);
    assert_eq!(progressif.height, one_shot.height);
    assert_eq!(
        progressif.pixels, one_shot.pixels,
        "la frame incrémentale diverge du rendu complet"
    );
}

#[test]
fn les_boites_d_une_frame_referencent_des_noeuds_toujours_vivants() {
    let mut tb = TreeBuilder::new();
    for chunk in &CHUNKS[..4] {
        tb.push(chunk);
    }
    let bx = box_tree_of(tb.dom()).expect("boîtes intermédiaires");
    // Le flux se termine ensuite : aucun NodeId référencé par
    // cette frame n'a été collecté pendant la construction —
    // elle reste hit-testable jusqu'à la suivante.
    tb.finish();
    assert_nodes_alive(&bx, tb.dom());
}

fn assert_nodes_alive(b: &LayoutBox, dom: &Dom) {
    if let Some(n) = b.node {
        assert!(dom.is_alive(n), "NodeId {n:?} collecté en cours de route");
    }
    for c in &b.children {
        assert_nodes_alive(c, dom);
    }
}
