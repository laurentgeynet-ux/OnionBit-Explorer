// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Tests du réordonnancement bidi visuel (R-171) : hint
//! `dir` → `direction`, niveau de base du paragraphe dans
//! l'itemisation UAX #9, retournement §L2 des fragments
//! par niveau à la fermeture de ligne.

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

/// Fragment texte de `line` contenant `needle` (exact pour
/// les runs découpés par UAX #14 — les pièces gardent leur
/// texte propre).
fn frag_with<'a>(
    line: &'a explorer_layout::LineBox,
    needle: &str,
) -> &'a explorer_layout::Fragment {
    line.fragments
        .iter()
        .find(|f| f.text.contains(needle))
        .unwrap_or_else(|| panic!("fragment « {needle} » absent : {:?}", line.fragments))
}

#[test]
fn ltr_base_garde_ordre_logique() {
    // Contrôle : paragraphe LTR « abc אבג def » — l'hébreu
    // reste entre ses deux voisins latins (ordre logique =
    // visuel pour les niveaux).
    let (dom, bx) = laid(
        "<!doctype html><p>abc אבג def</p>",
        "p{font-size:10px;width:400px}",
    );
    let p = find(&bx, by_tag(&dom, "p"));
    let line = &p.children[0].lines[0];
    let (fa, fh, fd) = (
        frag_with(line, "abc"),
        frag_with(line, "אבג"),
        frag_with(line, "def"),
    );
    assert!(
        fa.rect.x < fh.rect.x && fh.rect.x < fd.rect.x,
        "LTR : abc < אבג < def — x = {}, {}, {}",
        fa.rect.x,
        fh.rect.x,
        fd.rect.x
    );
    // Les niveaux sont portés par les fragments.
    assert_eq!(fa.level, 0);
    assert_eq!(fh.level, 1);
}

#[test]
fn rtl_base_retourne_et_ancre_a_droite() {
    // Paragraphe RTL « abc אבג def » : la lecture logique
    // part du bord droit — « abc » se peint à droite,
    // « def » à gauche (niveaux [2,…,1,…,2] → §L2).
    let (dom, bx) = laid(
        "<!doctype html><p dir=rtl>abc אבג def</p>",
        "p{font-size:10px;width:400px}",
    );
    let p = find(&bx, by_tag(&dom, "p"));
    let line = &p.children[0].lines[0];
    let (fa, fd) = (frag_with(line, "abc"), frag_with(line, "def"));
    assert!(
        fa.rect.x > fd.rect.x,
        "RTL : abc doit être à droite de def — x = {} vs {}",
        fa.rect.x,
        fd.rect.x
    );
    // Le premier logique est ancré au bord droit du contenu.
    let right = fa.rect.x + fa.rect.width;
    let edge = p.children[0].rect.x + p.children[0].rect.width;
    assert!(
        (right - edge).abs() < 1.0,
        "fin visuelle ~bord droit : {right} vs {edge}"
    );
}

#[test]
fn rtl_tout_hebreu_se_retourne() {
    // « שלום עליכם » tout niveau 1 : la séquence entière est
    // retournée — le premier mot logique part à droite.
    let (dom, bx) = laid(
        "<!doctype html><p dir=rtl>אאא בבב</p>",
        "p{font-size:10px;width:400px}",
    );
    let p = find(&bx, by_tag(&dom, "p"));
    let line = &p.children[0].lines[0];
    let (f1, f2) = (frag_with(line, "אאא"), frag_with(line, "בבב"));
    assert!(
        f1.rect.x > f2.rect.x,
        "premier mot à droite : {} > {}",
        f1.rect.x,
        f2.rect.x
    );
    let right = f1.rect.x + f1.rect.width;
    let edge = p.children[0].rect.x + p.children[0].rect.width;
    assert!(
        (right - edge).abs() < 1.0,
        "ancré droite : {right} vs {edge}"
    );
}

#[test]
fn dir_rtl_heritage_et_align_start() {
    // `dir` sur l'ancêtre : `direction` est héritée — un
    // bloc nu sous un `div[dir=rtl]` s'ancre à droite, et
    // `text-align: start` suit (spec : start = right).
    let (dom, bx) = laid(
        "<!doctype html><div dir=rtl><p>abc</p></div>",
        "p{font-size:10px;width:400px}",
    );
    let p = find(&bx, by_tag(&dom, "p"));
    let line = &p.children[0].lines[0];
    let f = frag_with(line, "abc");
    let edge = p.children[0].rect.x + p.children[0].rect.width;
    let right = f.rect.x + f.rect.width;
    assert!(
        (right - edge).abs() < 1.0,
        "start en RTL = droite : fin {right} vs {edge}"
    );
}

#[test]
fn dir_auto_resout_au_premier_fort() {
    // `dir=auto` : le premier caractère fort du contenu
    // fixe la directionnalité (hébreu → rtl → ancre droite).
    let (dom, bx) = laid(
        "<!doctype html><p dir=auto>שלום</p>",
        "p{font-size:10px;width:400px}",
    );
    let p = find(&bx, by_tag(&dom, "p"));
    let line = &p.children[0].lines[0];
    let f = frag_with(line, "שלום");
    let edge = p.children[0].rect.x + p.children[0].rect.width;
    let right = f.rect.x + f.rect.width;
    assert!(
        (right - edge).abs() < 1.0,
        "dir=auto + hébreu → ancré droite : {right} vs {edge}"
    );
    // Contrôle latin : dir=auto → ltr → ancre gauche.
    let (dom2, bx2) = laid(
        "<!doctype html><p dir=auto>hello</p>",
        "p{font-size:10px;width:400px}",
    );
    let p2 = find(&bx2, by_tag(&dom2, "p"));
    let line2 = &p2.children[0].lines[0];
    let f2 = frag_with(line2, "hello");
    assert!(f2.rect.x - p2.children[0].rect.x < 1.0, "ancré gauche");
}

#[test]
fn direction_css_rtl_equivaut_dir() {
    // `direction: rtl` en CSS (sans attribut `dir`) : même
    // niveau de base et même ancrage que `dir=rtl`.
    let (dom, bx) = laid(
        "<!doctype html><p style=\"direction:rtl\">אאא בבב</p>",
        "p{font-size:10px;width:400px}",
    );
    let p = find(&bx, by_tag(&dom, "p"));
    let line = &p.children[0].lines[0];
    let (f1, f2) = (frag_with(line, "אאא"), frag_with(line, "בבב"));
    assert!(f1.rect.x > f2.rect.x, "premier logique à droite");
    let right = f1.rect.x + f1.rect.width;
    let edge = p.children[0].rect.x + p.children[0].rect.width;
    assert!((right - edge).abs() < 1.0);
}

#[test]
fn rtl_wrap_garde_l_ordre_visuel_par_ligne() {
    // Le réordonnancement se fait ligne par ligne : chaque
    // ligne retournée s'ancre à droite indépendamment.
    let (dom, bx) = laid(
        "<!doctype html><p dir=rtl>אאא בבב גגג דדד</p>",
        "p{font-size:10px;width:60px}",
    );
    let p = find(&bx, by_tag(&dom, "p"));
    let anon = &p.children[0];
    assert!(anon.lines.len() >= 2, "{:?}", anon.lines.len());
    let edge = anon.rect.x + anon.rect.width;
    for line in &anon.lines {
        let right = line
            .fragments
            .iter()
            .map(|f| f.rect.x + f.rect.width)
            .fold(0.0f32, f32::max);
        assert!(
            (right - edge).abs() < 1.0,
            "chaque ligne RTL ancrée droite : {right} vs {edge}"
        );
    }
}
