// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Tests de l'inline layout (étape 20) : line boxes,
//! wrapping UAX #14, `text-align`, inline-block atomique —
//! fonte fixture pour les mesures réelles.

mod common;

use explorer_css::{Origin, StyleEngine, StyleEnv};
use explorer_dom::{Dom, NodeId};
use explorer_html::TreeBuilder;
use explorer_layout::font::{FontRequest, FontSource};
use explorer_layout::{build_box_tree, layout_tree, BlockCtx, LayoutBox};

const VIEW: f32 = 400.0;

/// `FontSource` sur la fixture : avance uniforme 0.5em →
/// `font-size:10px` = 5 px/caractère, métriques réelles.
struct Fixture(Vec<u8>);
impl FontSource for Fixture {
    fn font_data(&self, _req: &FontRequest) -> Option<Vec<u8>> {
        Some(self.0.clone())
    }
}

fn ctx() -> BlockCtx<'static> {
    let data = common::fixture_ttf();
    // Fuite volontaire : fixture statique de test.
    let src: &'static Fixture = Box::leak(Box::new(Fixture(data)));
    BlockCtx {
        fonts: Some(src),
        ..BlockCtx::default()
    }
}

fn laid(html: &str, css: &str) -> (Dom, LayoutBox) {
    laid_w(html, css, VIEW)
}

fn laid_w(html: &str, css: &str, w: f32) -> (Dom, LayoutBox) {
    let dom = TreeBuilder::parse(html);
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
    layout_tree(&mut bx, w, &ctx());
    (dom, bx)
}

fn by_tag(dom: &Dom, tag: &str) -> NodeId {
    let mut queue = vec![dom.document()];
    while let Some(n) = queue.pop() {
        if dom.tag_name(n).is_ok_and(|t| t == tag) {
            return n;
        }
        if let Ok(ch) = dom.children(n) {
            queue.extend(ch.iter().rev());
        }
    }
    panic!("<{tag}> absent")
}

fn find(b: &LayoutBox, id: NodeId) -> &LayoutBox {
    if b.node == Some(id) {
        return b;
    }
    b.children
        .iter()
        .find_map(|c| find_opt(c, id))
        .expect("boîte")
}
fn find_opt(b: &LayoutBox, id: NodeId) -> Option<&LayoutBox> {
    if b.node == Some(id) {
        return Some(b);
    }
    b.children.iter().find_map(|c| find_opt(c, id))
}

#[test]
fn lignes_wrappent_sur_largeur() {
    // p 100px, texte 5px/car → ~20 car/ligne.
    let (dom, bx) = laid(
        "<!doctype html><p>aaaa bbbb cccc dddd eeee ffff</p>",
        "p{font-size:10px;width:100px}",
    );
    let p = find(&bx, by_tag(&dom, "p"));
    let anon = &p.children[0];
    assert!(anon.lines.len() >= 2, "lines={:?}", anon.lines.len());
    // Chaque ligne ≤ 100px de contenu.
    for l in &anon.lines {
        assert!(l.rect.width <= 100.5, "{}", l.rect.width);
    }
    // Hauteur cumulée = rect.height.
    let sum: f32 = anon.lines.iter().map(|l| l.rect.height).sum();
    assert!((sum - anon.rect.height).abs() < 0.01);
}

#[test]
fn fragments_portent_texte_et_position() {
    let (dom, bx) = laid("<!doctype html><p>ab cd</p>", "p{font-size:10px}");
    let p = find(&bx, by_tag(&dom, "p"));
    let line = &p.children[0].lines[0];
    assert!(line.fragments.len() >= 2, "{:?}", line.fragments.len());
    let t: String = line.fragments.iter().map(|f| f.text.as_str()).collect();
    assert_eq!(t, "ab cd");
    // Positions croissantes en x.
    let mut last = f32::MIN;
    for f in &line.fragments {
        assert!(f.rect.x >= last);
        last = f.rect.x;
        assert!(f.shaped.is_some(), "glyphes shapés par la fixture");
    }
}

#[test]
fn text_align_right_et_center() {
    let (dom, bx) = laid(
        "<!doctype html><p>ab</p>",
        "p{font-size:10px;width:200px;text-align:right}",
    );
    let p = find(&bx, by_tag(&dom, "p"));
    let line = &p.children[0].lines[0];
    // « ab » = 10px → slack 190 → fin à 200 dans le contenu.
    let last = line.fragments.last().expect("frag");
    let end = last.rect.x + last.rect.width - p.children[0].rect.x;
    assert!((end - 200.0).abs() < 1.0, "fin={end}");

    let (d2, bx2) = laid(
        "<!doctype html><p>ab</p>",
        "p{font-size:10px;width:200px;text-align:center}",
    );
    let p2 = find(&bx2, by_tag(&d2, "p"));
    let first = &p2.children[0].lines[0].fragments[0];
    let off = first.rect.x - p2.children[0].rect.x;
    assert!((off - 95.0).abs() < 1.0, "centré off={off}");
}

#[test]
fn saut_de_ligne_obligatoire() {
    let (dom, bx) = laid(
        "<!doctype html><p>a\nb</p>",
        "p{font-size:10px;width:400px;white-space:pre-line}",
    );
    let p = find(&bx, by_tag(&dom, "p"));
    assert!(p.children[0].lines.len() >= 2);
}

#[test]
fn inline_block_est_atomique() {
    let (dom, bx) = laid(
        "<!doctype html><p>avant <span>bouton</span> après</p>",
        "p{font-size:10px;width:400px} span{display:inline-block;width:60px;height:20px}",
    );
    let p = find(&bx, by_tag(&dom, "p"));
    let line = &p.children[0].lines[0];
    let atom = line.fragments.iter().find(|f| f.atomic);
    assert!(atom.is_some(), "fragment atomique attendu");
    assert!((atom.expect("a").rect.width - 60.0).abs() < 0.5);
}

#[test]
fn baseline_aligne_les_fragments() {
    let (dom, bx) = laid(
        "<!doctype html><p>a<big>b</big></p>",
        "p{font-size:10px} big{font-size:20px}",
    );
    let p = find(&bx, by_tag(&dom, "p"));
    let line = &p.children[0].lines[0];
    assert!(line.fragments.len() >= 2);
    // Tous les fragments partagent la même baseline absolue.
    let bs: Vec<f32> = line
        .fragments
        .iter()
        .map(|f| f.rect.y + f.baseline)
        .collect();
    for b in &bs {
        assert!((b - bs[0]).abs() < 0.6, "baselines {bs:?}");
    }
}

#[test]
fn absolu_niche_ne_gonfle_pas_la_ligne() {
    // Un `position:absolute` niché dans un inline ne produit
    // aucun item dans la ligne englobante — sinon son
    // encombrement gonflerait la hauteur de ligne (R-22 :
    // lobste.rs, ligne de byline à ~4 lignes de haut).
    // Conteneur `div` : dans un `<p>`, le parseur HTML
    // fermerait le paragraphe avant le <div>.
    let (dom, bx) = laid(
        "<!doctype html><div class=w>av <span class=dp>x<div class=a>absolu</div></span> ap</div>",
        ".w{font-size:10px} .dp{position:relative} .a{position:absolute;top:40px;left:0}",
    );
    let w = find(&bx, by_tag(&dom, "div"));
    let anon = &w.children[0];
    assert_eq!(anon.lines.len(), 1);
    let h = anon.lines[0].rect.height;
    assert!(h < 15.0, "ligne ~12 px attendue, mesuré {h}");
    // Et l'absolu a bien été posé à top:40px sous le span
    // `position:relative` (son bloc conteneur).
    let span = find(&bx, by_tag(&dom, "span"));
    let a_node = {
        let mut q = vec![dom.document()];
        let mut hit = None;
        while let Some(n) = q.pop() {
            if dom.attribute(n, "class").ok().flatten().as_deref() == Some("a") {
                hit = Some(n);
            }
            if let Ok(ch) = dom.children(n) {
                q.extend(ch.iter().rev());
            }
        }
        hit.expect("div.a")
    };
    let abs = find(&bx, a_node);
    assert!(
        (abs.rect.y - span.rect.y - 40.0).abs() < 1.0,
        "abs y={:?} span y={:?}",
        abs.rect,
        span.rect
    );
}

#[test]
fn inline_block_avec_bloc_ne_gonfle_pas() {
    // `details.caches` (lobste.rs) : un atome `inline-block`
    // dont les enfants blocs + blancs doivent compter UNE
    // fois le contenu, pas une ligne par enfant blanc.
    let (dom, bx) = laid(
        "<!doctype html><div class=w>av <span class=c> <b>uni</b> <i>deux</i> </span> ap</div>",
        ".w{font-size:10px} .c{display:inline-block} b,i{display:block}",
    );
    let w = find(&bx, by_tag(&dom, "div"));
    let c = find(&bx, by_tag(&dom, "span"));
    // Deux lignes de contenu (b puis i), pas +3 lignes de blancs.
    assert!(
        c.rect.height < 30.0,
        "atome ~2×12 px attendu, mesuré {}",
        c.rect.height
    );
    // La ligne du conteneur reste proche de la hauteur atome.
    let h = w.children[0].lines[0].rect.height;
    assert!(h < 30.0, "hauteur de ligne {h}");
}

#[test]
fn text_align_mesure_les_margin_boxes() {
    // R-27 : `text-align` mesure la ligne sur les **margin
    // boxes** — les marges d'un atome comptent (§16.2 +
    // §8.3). Réf. WPT `flex-direction-row-reverse-ref`.
    let (dom, bx) = laid(
        "<!doctype html><div><span>x</span></div>",
        "div{text-align:right} span{display:inline-block;width:100px;height:10px;margin:0 20px}",
    );
    let span = find(&bx, by_tag(&dom, "span"));
    // Margin box droite au bord du conteneur (VIEW=400) :
    // border box à 400 - 20(mr) - 100 = 280 (l'ancienne
    // mesure border-box la plaçait à 300).
    assert!((span.rect.x - 280.0).abs() < 0.5, "x={}", span.rect.x);
}
