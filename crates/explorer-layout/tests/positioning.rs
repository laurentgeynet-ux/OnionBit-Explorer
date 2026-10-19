// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Tests de l'étape 21 : tables (auto layout borné),
//! `float`/`clear`, `position` (relative/absolute/fixed),
//! `z-index` et `overflow` — métadonnées usées posées sur
//! `LayoutBox` par le layout.

mod common;

use explorer_css::{Origin, StyleEngine, StyleEnv};
use explorer_dom::{Dom, NodeId};
use explorer_html::TreeBuilder;
use explorer_layout::font::{FontRequest, FontSource};
use explorer_layout::{
    build_box_tree, layout_tree, BlockCtx, Float, LayoutBox, Overflow, Position,
};

const VIEW: f32 = 400.0;

/// `FontSource` sur la fixture : avance uniforme 0.5em.
struct Fixture(Vec<u8>);
impl FontSource for Fixture {
    fn font_data(&self, _req: &FontRequest) -> Option<Vec<u8>> {
        Some(self.0.clone())
    }
}

fn ctx() -> BlockCtx<'static> {
    let data = common::fixture_ttf();
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

fn by_id(dom: &Dom, id: &str) -> NodeId {
    let mut queue = vec![dom.document()];
    while let Some(n) = queue.pop() {
        if dom
            .attribute(n, "id")
            .ok()
            .flatten()
            .is_some_and(|v| v == id)
        {
            return n;
        }
        if let Ok(ch) = dom.children(n) {
            queue.extend(ch.iter().rev());
        }
    }
    panic!("#{id} absent")
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

// ---------- tables ----------

#[test]
fn table_colonnes_egales_sans_largeurs() {
    // Deux colonnes sans `width` : la largeur de la table se
    // répartit proportionnellement aux largeurs préférées
    // (contenus égaux → parts égales).
    let (dom, bx) = laid(
        "<table><tr><td id=a>xx</td><td id=b>yy</td></tr></table>",
        "table{width:200px}",
    );
    let a = find(&bx, by_id(&dom, "a"));
    let b = find(&bx, by_id(&dom, "b"));
    assert!(
        (b.rect.x - (a.rect.x + a.rect.width)).abs() < 0.01,
        "cellules adjacentes : a {:?} b {:?}",
        a.rect,
        b.rect
    );
    assert!((a.rect.width - b.rect.width).abs() < 0.01);
    assert!(
        (a.rect.width - 100.0).abs() < 0.5,
        "colonne ≈ moitié de la table : {}",
        a.rect.width
    );
}

#[test]
fn table_rangee_hauteur_max_cellule() {
    let (dom, bx) = laid(
        "<table><tr><td id=a><div class=t></div></td>\
         <td id=b><div class=u></div></td></tr></table>",
        "table{width:200px} .t{height:30px} .u{height:10px}",
    );
    let a = find(&bx, by_id(&dom, "a"));
    let b = find(&bx, by_id(&dom, "b"));
    // La rangée prend la plus grande hauteur ; les cellules
    // auto sont uniformisées à la hauteur de rangée.
    assert!(
        (b.rect.height - a.rect.height).abs() < 0.01,
        "a {} b {}",
        a.rect.height,
        b.rect.height
    );
}

#[test]
fn table_largeur_specifiee_cellule() {
    let (dom, bx) = laid(
        "<table><tr><td id=a>xx</td><td id=b>yy</td></tr></table>",
        "table{width:200px} #a{width:150px}",
    );
    let a = find(&bx, by_id(&dom, "a"));
    let b = find(&bx, by_id(&dom, "b"));
    assert!((a.rect.width - 150.0).abs() < 0.5);
    assert!(b.rect.width > 0.0);
}

// ---------- floats / clear ----------

#[test]
fn float_se_place_au_bord_et_frere_retrécit() {
    let (dom, bx) = laid(
        "<div id=box><div id=f></div><div id=n>contenu</div></div>",
        "#f{float:left;width:100px;height:50px} #box{width:300px}",
    );
    let f = find(&bx, by_id(&dom, "f"));
    let n = find(&bx, by_id(&dom, "n"));
    assert_eq!(f.float, Float::Left);
    // Le flottant colle au bord gauche du contenu.
    assert!((f.rect.x - n.rect.x) <= 100.0);
    // Le bloc suivant voit une bande rétrécie (x décalé).
    assert!(
        n.rect.x + n.rect.width <= f.rect.x + f.rect.width + 200.0,
        "n {:?} après f {:?}",
        n.rect,
        f.rect
    );
}

#[test]
fn clear_descend_sous_le_flottant() {
    let (dom, bx) = laid(
        "<div id=box><div id=f></div><div id=c></div></div>",
        "#f{float:left;width:100px;height:50px} #c{clear:left;height:10px} #box{width:300px}",
    );
    let f = find(&bx, by_id(&dom, "f"));
    let c = find(&bx, by_id(&dom, "c"));
    assert!(
        c.rect.y >= f.rect.y + f.rect.height - 0.01,
        "c {:?} doit passer sous f {:?}",
        c.rect,
        f.rect
    );
}

#[test]
fn float_right_colle_au_bord_droit() {
    let (dom, bx) = laid(
        "<div id=box><div id=f></div></div>",
        "#f{float:right;width:80px;height:40px} #box{width:300px}",
    );
    let f = find(&bx, by_id(&dom, "f"));
    let bxw = find(&bx, by_id(&dom, "box"));
    let right_edge = bxw.rect.x + bxw.rect.width;
    assert!(
        (f.rect.x + f.rect.width - right_edge).abs() < 1.0,
        "flottant droit {:?} vs bord {}",
        f.rect,
        right_edge
    );
}

// ---------- position ----------

#[test]
fn relative_decale_sans_sortir_du_flux() {
    let (dom, bx) = laid(
        "<div id=a></div><div id=r></div><div id=b></div>",
        "#a{height:20px} #r{height:10px;position:relative;top:15px;left:5px} #b{height:10px}",
    );
    let r = find(&bx, by_id(&dom, "r"));
    let a = find(&bx, by_id(&dom, "a"));
    let b = find(&bx, by_id(&dom, "b"));
    assert_eq!(r.position, Position::Relative);
    // Décalé de top:15px relativement à sa place de flux.
    let flux_y = a.rect.y + a.rect.height;
    assert!((r.rect.y - (flux_y + 15.0)).abs() < 0.5, "r.y={}", r.rect.y);
    assert!((r.rect.x - 5.0).abs() < 0.5);
    // Le suivant reste au flux (pas ré-affecté par le décalage).
    assert!(b.rect.y < r.rect.y);
}

#[test]
fn absolute_hors_flux_ancre_au_positionne() {
    let (dom, bx) = laid(
        "<div id=p><div id=q></div></div><div id=n></div>",
        "#p{position:relative;height:100px;width:200px}\
         #q{position:absolute;top:10px;left:20px;width:50px;height:30px}\
         #n{height:5px}",
    );
    let p = find(&bx, by_id(&dom, "p"));
    let q = find(&bx, by_id(&dom, "q"));
    let n = find(&bx, by_id(&dom, "n"));
    assert_eq!(q.position, Position::Absolute);
    // Ancré au padding box du parent positionné.
    assert!(
        (q.rect.x - (p.rect.x + 20.0)).abs() < 1.0,
        "q {:?} p {:?}",
        q.rect,
        p.rect
    );
    assert!((q.rect.y - (p.rect.y + 10.0)).abs() < 1.0);
    // Hors flux : le frère suivant n'est pas repoussé.
    assert!((n.rect.y - (p.rect.y + p.rect.height)).abs() < 0.5);
}

#[test]
fn fixed_ancre_viewport() {
    let (dom, bx) = laid(
        "<div id=q></div>",
        "#q{position:fixed;top:0;right:0;width:60px;height:20px}",
    );
    let q = find(&bx, by_id(&dom, "q"));
    assert_eq!(q.position, Position::Fixed);
    // right:0 → bord droit du viewport (VIEW=400).
    assert!(
        (q.rect.x + q.rect.width - VIEW).abs() < 1.0,
        "q {:?}",
        q.rect
    );
    assert!(q.rect.y.abs() < 1.0);
}

#[test]
fn absolute_bottom_dans_parent_positionne() {
    let (dom, bx) = laid(
        "<div id=p><div id=q></div></div>",
        "#p{position:relative;height:100px;width:200px}\
         #q{position:absolute;bottom:5px;width:40px;height:20px}",
    );
    let p = find(&bx, by_id(&dom, "p"));
    let q = find(&bx, by_id(&dom, "q"));
    // bottom:5 → y = bas du cb - 5 - hauteur.
    let expected = p.rect.y + p.rect.height - 5.0 - 20.0;
    assert!(
        (q.rect.y - expected).abs() < 1.0,
        "q {:?} attendu {}",
        q.rect,
        expected
    );
}

// ---------- métadonnées usées ----------

#[test]
fn zindex_et_overflow_usages() {
    let (dom, bx) = laid(
        "<div id=z></div><div id=a></div>",
        "#z{z-index:7;overflow:hidden} #a{overflow:auto}",
    );
    let z = find(&bx, by_id(&dom, "z"));
    let a = find(&bx, by_id(&dom, "a"));
    assert_eq!(z.z_index, 7);
    assert_eq!(z.overflow, Overflow::Hidden);
    assert_eq!(a.overflow, Overflow::Auto);
}
