// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Tests du block layout (étape 19) : flux normal, marges
//! collapsées, dimensions auto/spécifiées, hauteurs auto —
//! mesures réelles sur la fonte fixture.

mod common;

use explorer_css::{Origin, StyleEngine, StyleEnv};
use explorer_dom::{Dom, NodeId};
use explorer_html::TreeBuilder;
use explorer_layout::{build_box_tree, layout_tree, BlockCtx, BoxKind, LayoutBox};

const VIEW: f32 = 800.0;

/// Pipeline complet : html + css → arbre de boîtes posé.
fn laid(html: &str, css: &str) -> (Dom, LayoutBox) {
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
    layout_tree(&mut bx, VIEW, &BlockCtx::default());
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
        .find_map(|c| {
            if c.node == Some(id) {
                Some(c)
            } else {
                c.children.iter().find_map(|g| find_opt(g, id))
            }
        })
        .expect("boîte du nœud")
}

fn find_opt(b: &LayoutBox, id: NodeId) -> Option<&LayoutBox> {
    if b.node == Some(id) {
        return Some(b);
    }
    b.children.iter().find_map(|c| find_opt(c, id))
}

fn approx(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.51
}

#[test]
fn bloc_remplit_la_largeur() {
    let (dom, bx) = laid("<!doctype html><div>aa</div>", "");
    let div = find(&bx, by_tag(&dom, "div"));
    assert!(approx(div.rect.width, VIEW), "{}", div.rect.width);
    assert!(approx(div.rect.x, 0.0));
}

#[test]
fn width_specifiee_et_centrage_auto() {
    let css = "div{width:200px;margin-left:auto;margin-right:auto}";
    let (dom, bx) = laid("<!doctype html><div>x</div>", css);
    let div = find(&bx, by_tag(&dom, "div"));
    assert!(approx(div.rect.width, 200.0), "{}", div.rect.width);
    // slack (800-200)/2 = 300 → centré.
    assert!(approx(div.rect.x, 300.0), "{}", div.rect.x);
}

#[test]
fn width_specifiee_marges_fixes() {
    let css = "div{width:100px;margin-left:50px}";
    let (dom, bx) = laid("<!doctype html><div>x</div>", css);
    let div = find(&bx, by_tag(&dom, "div"));
    assert!(approx(div.rect.x, 50.0), "{}", div.rect.x);
    assert!(approx(div.rect.width, 100.0));
}

#[test]
fn padding_border_elargissent_la_boite() {
    let css = "div{width:100px;padding:10px;border-width:5px;border-style:solid}";
    let (dom, bx) = laid("<!doctype html><div>x</div>", css);
    let div = find(&bx, by_tag(&dom, "div"));
    // border-box = 5 + 10 + 100 + 10 + 5 = 130.
    assert!(approx(div.rect.width, 130.0), "{}", div.rect.width);
}

#[test]
fn marges_verticales_collapse() {
    // Deux p : 20px/30px → écart 30 (max), pas 50.
    let css = "p{margin-top:0;margin-bottom:20px} p+p{margin-top:30px}";
    let (dom, bx) = laid("<!doctype html><p>a</p><p>b</p>", css);
    // Les deux p : parcours direct des enfants blocs de body.
    let body = find(&bx, by_tag(&dom, "body"));
    let blocks: Vec<&LayoutBox> = body
        .children
        .iter()
        .filter(|c| c.kind == explorer_layout::BoxKind::Block)
        .collect();
    assert!(blocks.len() >= 2);
    let gap = blocks[1].rect.y - (blocks[0].rect.y + blocks[0].rect.height);
    assert!(approx(gap, 30.0), "marges collapsées : {gap} (attendu 30)");
}

#[test]
fn hauteur_auto_somme_des_enfants() {
    let css = "div{height:auto} p{height:40px;margin:0} p+p{height:60px}";
    let (dom, bx) = laid("<!doctype html><div><p>a</p><p>b</p></div>", css);
    let div = find(&bx, by_tag(&dom, "div"));
    assert!(approx(div.rect.height, 100.0), "{}", div.rect.height);
}

#[test]
fn hauteur_specifiee_et_min_max() {
    let (dom, bx) = laid(
        "<!doctype html><div>x</div>",
        "div{height:50px;min-height:80px}",
    );
    let div = find(&bx, by_tag(&dom, "div"));
    assert!(
        approx(div.rect.height, 80.0),
        "min-height gagne : {}",
        div.rect.height
    );
    let (d2, bx2) = laid(
        "<!doctype html><div>x</div>",
        "div{height:200px;max-height:120px}",
    );
    let div2 = find(&bx2, by_tag(&d2, "div"));
    assert!(approx(div2.rect.height, 120.0));
}

#[test]
fn premier_enfant_margin_traverse_le_parent() {
    // div sans padding/bordure : la marge du p enfant « traverse »
    // — pas d'espace interne au-dessus du p.
    let css = "p{margin-top:50px;margin-bottom:0;height:10px}";
    let (dom, bx) = laid("<!doctype html><div><p>x</p></div>", css);
    let div = find(&bx, by_tag(&dom, "div"));
    let p = find(&bx, by_tag(&dom, "p"));
    assert!(
        approx(p.rect.y - div.rect.y, 0.0),
        "{}",
        p.rect.y - div.rect.y
    );
}

#[test]
fn texte_donne_des_lignes() {
    // Fixture : 500 upem × taille → mesure réelle si injectée ;
    // ici headless → repli métrique, mais ≥ 1 ligne × line-height.
    let (dom, bx) = laid(
        "<!doctype html><p>un peu de texte assez long pour plusieurs lignes de test</p>",
        "p{font-size:20px;width:100px}",
    );
    let p = find(&bx, by_tag(&dom, "p"));
    let anon = &p.children[0];
    assert!(anon.rect.height >= 24.0, "{}", anon.rect.height);
    assert!(p.rect.height >= 24.0);
}

// ── legacy HTML : <center>, hints de table (R-16) ─────────

#[test]
fn center_est_bloc_pleine_largeur() {
    // Avant R-16 : <center> absent de la feuille UA → inline,
    // la table enfant perdait son conteneur bloc.
    let (dom, bx) = laid("<!doctype html><center>x</center>", "");
    let c = find(&bx, by_tag(&dom, "center"));
    assert_eq!(c.kind, BoxKind::Block);
    assert!(approx(c.rect.width, VIEW), "{}", c.rect.width);
}

#[test]
fn center_centre_un_descendant_bloc() {
    // `-webkit-center` (UA <center>) : les descendants bloc
    // plus étroits sont centrés — comportement historique
    // des navigateurs que les tables à width% attendent.
    let (dom, bx) = laid(
        "<!doctype html><center><div style=\"width:200px\">x</div></center>",
        "",
    );
    let div = find(&bx, by_tag(&dom, "div"));
    assert!(approx(div.rect.width, 200.0), "{}", div.rect.width);
    assert!(
        approx(div.rect.x, (VIEW - 200.0) / 2.0),
        "div centrée : x={} attendu {}",
        div.rect.x,
        (VIEW - 200.0) / 2.0
    );
}

#[test]
fn center_centre_une_table_etroite() {
    // Le cas terrain : table auto dans <center>.
    let (dom, bx) = laid(
        "<!doctype html><center><table><tr><td>x</td></tr></table></center>",
        "",
    );
    let t = find(&bx, by_tag(&dom, "table"));
    assert!(t.rect.width < VIEW, "{}", t.rect.width);
    assert!(
        approx(t.rect.x, (VIEW - t.rect.width) / 2.0),
        "table centrée : x={}",
        t.rect.x
    );
}

#[test]
fn table_width_pourcent_du_conteneur() {
    // `width="85%"` en attribut → la table mesure 85 % du
    // bloc conteneur (présentational hint → width CSS).
    let (dom, bx) = laid(
        "<!doctype html><table width=\"85%\"><tr><td>x</td></tr></table>",
        "",
    );
    let t = find(&bx, by_tag(&dom, "table"));
    assert!(approx(t.rect.width, VIEW * 0.85), "{}", t.rect.width);
}

#[test]
fn font_size_em_ne_se_remultiplie_pas() {
    // R-17 : `font-size` hérite la valeur calculée absolue —
    // avant, le « 2em » brut du style cloné (anon/texte) se
    // re-résolvait contre le fs parent déjà résolu (×4/×8
    // mesurés sur le terrain). `2em` doit égaler `32px`.
    let (d1, bx1) = laid("<!doctype html><div style=\"font-size:2em\">x</div>", "");
    let (d2, bx2) = laid("<!doctype html><div style=\"font-size:32px\">x</div>", "");
    let h1 = find(&bx1, by_tag(&d1, "div")).rect.height;
    let h2 = find(&bx2, by_tag(&d2, "div")).rect.height;
    assert!(approx(h1, h2), "2em={h1} vs 32px={h2}");
}
