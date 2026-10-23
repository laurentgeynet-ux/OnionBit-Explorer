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

#[test]
fn table_colspan_cellule_couvre_ses_colonnes() {
    // R-47 — la forme réelle de Hacker News : 3 colonnes
    // (rang, vote, titre) ; le rang subtext est
    // <td colspan=2></td><td class=subtext> — la cellule
    // utile tombe dans la TROISIÈME colonne (sous le titre),
    // pas sous le rang.
    let (dom, bx) = laid(
        "<table><tr><td id=rank>1.</td><td id=vote>^</td>\
         <td id=title>titre long de la dépêche</td></tr>\
         <tr><td colspan=2></td><td id=sub>123 points par x il y a 2 h</td></tr></table>",
        "table{width:400px}",
    );
    let rank = find(&bx, by_id(&dom, "rank"));
    let title = find(&bx, by_id(&dom, "title"));
    let sub = find(&bx, by_id(&dom, "sub"));
    assert!(
        (sub.rect.x - title.rect.x).abs() < 0.01,
        "subtext sous la colonne titre : rank={:?} title={:?} sub={:?}",
        rank.rect,
        title.rect,
        sub.rect
    );
    assert!((sub.rect.width - title.rect.width).abs() < 0.01);
}

#[test]
fn table_colspan_mesure_repartie_sur_les_colonnes() {
    // Une cellule à span impose son contenu aux colonnes
    // couvertes collectivement : sans elle elles seraient
    // étroites, le déficit se répartit (ici la seule source
    // de largeur — la table est élargie par le span).
    let (dom, bx) = laid(
        "<table><tr><td colspan=2 id=w>contenu très large de la cellule enjambée</td></tr>\
         <tr><td id=a>x</td><td id=b>y</td></tr></table>",
        "table{width:400px}",
    );
    let w = find(&bx, by_id(&dom, "w"));
    let a = find(&bx, by_id(&dom, "a"));
    let b = find(&bx, by_id(&dom, "b"));
    // La cellule enjambée couvre exactement les deux colonnes.
    assert!(
        (w.rect.width - (a.rect.width + b.rect.width)).abs() < 0.01,
        "w={:?} a={:?} b={:?}",
        w.rect,
        a.rect,
        b.rect
    );
    assert!((w.rect.x - a.rect.x).abs() < 0.01);
}

#[test]
fn table_rowspan_retient_sa_colonne() {
    // R-47 — `rowspan=2` : la cellule du rang suivant ne peut
    // pas s'installer dans la colonne retenue — elle glisse
    // sous la deuxième colonne.
    let (dom, bx) = laid(
        "<table><tr><td id=sp rowspan=2>a</td><td id=b>b</td></tr>\
         <tr><td id=c>c</td></tr></table>",
        "table{width:200px}",
    );
    let sp = find(&bx, by_id(&dom, "sp"));
    let b = find(&bx, by_id(&dom, "b"));
    let c = find(&bx, by_id(&dom, "c"));
    assert!(
        (c.rect.x - b.rect.x).abs() < 0.01,
        "c sous b (colonne retenue) : sp={:?} b={:?} c={:?}",
        sp.rect,
        b.rect,
        c.rect
    );
    // La boîte du span s'étend sur les deux rangs.
    assert!(
        sp.rect.height > b.rect.height,
        "étendue peinte ≥ 2 rangs : sp={:?} b={:?}",
        sp.rect,
        b.rect
    );
}

#[test]
fn table_spans_malformes_degradent() {
    // R-47 — tolérance : `0`, non-numérique et débordement se
    // comportent comme 1, jamais de panique.
    let (dom, bx) = laid(
        "<table><tr><td id=a colspan=0>x</td><td id=b colspan=abc>y</td>\
         <td id=c colspan=999999999>z</td></tr></table>",
        "table{width:300px}",
    );
    let a = find(&bx, by_id(&dom, "a"));
    let b = find(&bx, by_id(&dom, "b"));
    let c = find(&bx, by_id(&dom, "c"));
    for (n, r) in [("a", a), ("b", b), ("c", c)] {
        assert!(r.rect.width > 0.0, "{n} posée : {r:?}");
    }
    assert!(b.rect.x >= a.rect.x + a.rect.width - 0.01);
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

#[test]
fn absolu_inline_position_statique_du_conteneur() {
    // R-60 — motif `.a11y-menu` de MDN : un `<a>` inline
    // `position:absolute` (sans `top`) seul contenu d'un `ul`
    // absolu placé hors-champ. La position statique de
    // l'enfant doit suivre son bloc conteneur — avant le
    // correctif, le flux inline ne lui écrivait aucun rect
    // → `top:auto` ancrait à y=0 (skip-link peint en haut).
    let (dom, bx) = laid(
        "<ul id=m><li><a id=sk href=#x>Skip</a></li></ul><p id=v>vu</p>",
        "#m{position:absolute;top:-100px;width:100%}
         #sk{position:absolute;left:0;right:0}",
    );
    let m = find(&bx, by_id(&dom, "m"));
    let sk = find(&bx, by_id(&dom, "sk"));
    assert!(m.rect.y < -50.0, "conteneur hors-champ : {:?}", m.rect);
    // `top:auto` → bord haut du contenu du conteneur.
    assert!(
        (sk.rect.y - m.rect.y).abs() < 1.0,
        "l'enfant absolu suit son conteneur : m={:?} sk={:?}",
        m.rect,
        sk.rect
    );
}

#[test]
fn absolu_largeur_pourcent_content_box() {
    // R-91 — terrain github : `.lp-IntroHero` (`position:
    // fixed; width:100%; padding:0 24px`, content-box) rendait
    // 1312 px dans un CB de 1216. `abs_geometry` calculait la
    // border box (1216+48=1264) et la repassait à
    // `layout_block` comme `cb_w` — le % était résolu UNE
    // DEUXIÈME fois sur la base gonflée (1264→1312). Le %
    // d'un absolu doit se résoudre une seule fois, contre le
    // vrai bloc conteneur.
    let (dom, bx) = laid_w(
        "<div id=p><div id=a><div id=i></div></div></div>",
        "#p{position:relative;height:10px}\
         #a{position:absolute;width:100%;padding:0 24px}\
         #i{height:10px}",
        1216.0,
    );
    let a = find(&bx, by_id(&dom, "a"));
    let i = find(&bx, by_id(&dom, "i"));
    // content-box : contenu = 100 % du CB (1216), border =
    // contenu + padding (48) = 1264 — pas 1312.
    assert!(
        (a.rect.width - 1264.0).abs() < 0.01,
        "absolu width:100% content-box : border = CB + châssis, {:?}",
        a.rect
    );
    assert!(
        (i.rect.width - 1216.0).abs() < 0.01,
        "contenu = CB : {:?}",
        i.rect
    );
}

#[test]
fn absolu_largeur_pourcent_border_box() {
    // Variante border-box (le cas github une fois
    // `*{box-sizing:border-box}` appliqué, R-91) : la border
    // box = 100 % du CB, le contenu absorbe le padding.
    let (dom, bx) = laid_w(
        "<div id=p><div id=a><div id=i></div></div></div>",
        "#p{position:relative;height:10px}\
         #a{position:absolute;width:100%;padding:0 24px;box-sizing:border-box}\
         #i{height:10px}",
        1216.0,
    );
    let a = find(&bx, by_id(&dom, "a"));
    let i = find(&bx, by_id(&dom, "i"));
    assert!(
        (a.rect.width - 1216.0).abs() < 0.01,
        "absolu width:100% border-box : border = CB, {:?}",
        a.rect
    );
    assert!(
        (i.rect.width - (1216.0 - 48.0)).abs() < 0.01,
        "contenu = CB - châssis : {:?}",
        i.rect
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
