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

#[test]
fn width_max_content_mesure_le_texte() {
    // R-3 : `width: max-content` — la boîte se rétracte à sa
    // plus longue ligne insécable (texte entier effondré : « aaa bbbb » =
    // 8 caractères × 8.8px en repli métrique headless).
    let (dom, bx) = laid(
        "<!doctype html><div style=\"width:max-content\">aaa bbbb</div>",
        "",
    );
    let d = find(&bx, by_tag(&dom, "div"));
    assert!(approx(d.rect.width, 8.0 * 8.8), "{}", d.rect.width);
}

#[test]
fn width_min_content_mot_le_plus_long() {
    // `min-content` = le plus grand morceau insécable : le
    // mot le plus long (« bbbb », 4 chars → 35.2px), pas le
    // texte entier ni le conteneur.
    let (dom, bx) = laid(
        "<!doctype html><div style=\"width:min-content\">aaa bbbb</div>",
        "",
    );
    let d = find(&bx, by_tag(&dom, "div"));
    assert!(approx(d.rect.width, 4.0 * 8.8), "{}", d.rect.width);
}

#[test]
fn width_fit_content_borne_par_disponible() {
    // `fit-content` = min(max-content, max(min-content,
    // dispo)) : texte court → max-content ; texte long →
    // borné à la largeur disponible.
    let (dom, bx) = laid(
        "<!doctype html><div style=\"width:fit-content\">aaa bbbb</div>",
        "",
    );
    let d = find(&bx, by_tag(&dom, "div"));
    assert!(approx(d.rect.width, 8.0 * 8.8), "court {}", d.rect.width);
    // Mots séparés : max-content ~1760 > dispo, min-content
    // = un mot (35.2) < dispo → borné à 800.
    let long = "aaaa ".repeat(50);
    let (dom2, bx2) = laid(
        &format!("<!doctype html><div style=\"width:fit-content\">{long}</div>"),
        "",
    );
    let d2 = find(&bx2, by_tag(&dom2, "div"));
    assert!(approx(d2.rect.width, VIEW), "long {}", d2.rect.width);
}

#[test]
fn max_content_via_enfant_bloc_a_largeur_fixe() {
    // Le max-content d'un conteneur inclut ses enfants bloc :
    // un fils à `width: 120px` impose 120px même si le texte
    // est plus étroit ou plus court.
    let (dom, bx) = laid(
        "<!doctype html><div style=\"width:max-content\">\
           <div style=\"width:120px\">x</div>yy</div>",
        "",
    );
    // `by_tag` visite en ordre document : le premier div est
    // le conteneur à `max-content` (le fils 120px + « yy »
    // inline → contribution max = 120).
    let mcx = find(&bx, by_tag(&dom, "div"));
    assert!(approx(mcx.rect.width, 120.0), "{}", mcx.rect.width);
}

#[test]
fn max_width_max_content_borne() {
    // `max-width: max-content` borne la boîte à sa ligne
    // insécable (le texte aurait droit à 800px en `auto`).
    let (dom, bx) = laid(
        "<!doctype html><div style=\"max-width:max-content\">aaa bbbb</div>",
        "",
    );
    let d = find(&bx, by_tag(&dom, "div"));
    assert!(approx(d.rect.width, 8.0 * 8.8), "{}", d.rect.width);
}

#[test]
fn flex_margin_auto_pousse_a_droite() {
    // R-3 : `margin-left: auto` sur un item flex absorbe le
    // libre de l'axe principal — le second item est poussé
    // contre le bord droit du conteneur.
    let (dom, bx) = laid(
        "<!doctype html><div id=\"f\" style=\"display:flex\">\
           <div style=\"width:100px\">a</div>\
           <div style=\"width:100px;margin-left:auto\">b</div></div>",
        "",
    );
    let f = find(&bx, by_tag(&dom, "div"));
    assert_eq!(f.children.len(), 2);
    let second = &f.children[1];
    assert!(
        approx(second.rect.x + second.rect.width, VIEW),
        "b fin x={}",
        second.rect.x
    );
}

#[test]
fn flex_margin_auto_des_deux_cotes_centre() {
    // `margin: 0 auto` sur l'item : les deux marges absorbent
    // → centrage dans l'axe principal.
    let (dom, bx) = laid(
        "<!doctype html><div style=\"display:flex\">\
           <div style=\"width:100px;margin:0 auto\">a</div></div>",
        "",
    );
    let f = find(&bx, by_tag(&dom, "div"));
    let item = &f.children[0];
    assert!(
        approx(item.rect.x, (VIEW - 100.0) / 2.0),
        "centré x={}",
        item.rect.x
    );
}

#[test]
fn flex_align_stretch_etire_la_boite() {
    // `align-items: stretch` (défaut) : l'item sans `height`
    // s'étire à la hauteur de la ligne (= le plus grand
    // frère). Avant : traité comme flex-start.
    let (dom, bx) = laid(
        "<!doctype html><div style=\"display:flex\">\
           <div style=\"width:100px;height:80px\">a</div>\
           <div style=\"width:100px\">b</div></div>",
        "",
    );
    let f = find(&bx, by_tag(&dom, "div"));
    assert!(
        approx(f.children[1].rect.height, 80.0),
        "{}",
        f.children[1].rect.height
    );
    // `height` explicite : pas d'étirement (spec).
    let (dom2, bx2) = laid(
        "<!doctype html><div style=\"display:flex\">\
           <div style=\"width:100px;height:80px\">a</div>\
           <div style=\"width:100px;height:30px\">b</div></div>",
        "",
    );
    let f2 = find(&bx2, by_tag(&dom2, "div"));
    assert!(
        approx(f2.children[1].rect.height, 30.0),
        "{}",
        f2.children[1].rect.height
    );
}

#[test]
fn flex_wrap_coupe_en_lignes() {
    // R-4 : `flex-wrap: wrap` — les items qui dépassent la
    // piste principale passent à la ligne suivante (3×300px
    // dans 800 → 2 sur la 1re ligne, 1 sur la 2e).
    let (dom, bx) = laid(
        "<!doctype html><div style=\"display:flex;flex-wrap:wrap\">\
           <div style=\"width:300px\">a</div>\
           <div style=\"width:300px\">b</div>\
           <div style=\"width:300px\">c</div></div>",
        "",
    );
    let f = find(&bx, by_tag(&dom, "div"));
    let (a, b, c) = (&f.children[0], &f.children[1], &f.children[2]);
    assert!(approx(a.rect.y, 0.0) && approx(b.rect.y, 0.0), "ligne 1");
    assert!(approx(b.rect.x, 300.0), "b x={}", b.rect.x);
    assert!(
        c.rect.y > a.rect.height - 0.5,
        "c sur ligne 2 y={}",
        c.rect.y
    );
    assert!(approx(c.rect.x, 0.0), "c x={}", c.rect.x);
    // Hauteur du conteneur = 2 lignes.
    assert!(
        approx(f.rect.height, 2.0 * a.rect.height),
        "{}",
        f.rect.height
    );
}

#[test]
fn flex_wrap_nowrap_par_defaut() {
    // Sans wrap : débordement reste sur une ligne.
    let (dom, bx) = laid(
        "<!doctype html><div style=\"display:flex\">\
           <div style=\"width:500px\">a</div>\
           <div style=\"width:500px\">b</div></div>",
        "",
    );
    let f = find(&bx, by_tag(&dom, "div"));
    assert!(approx(f.children[1].rect.y, 0.0), "même ligne");
    assert!(f.children[1].rect.x > 0.0, "b déborde à droite");
}

#[test]
fn flex_wrap_row_gap_entre_lignes() {
    let (dom, bx) = laid(
        "<!doctype html><div style=\"display:flex;flex-wrap:wrap;row-gap:10px\">\
           <div style=\"width:600px\">a</div>\
           <div style=\"width:600px\">b</div></div>",
        "",
    );
    let f = find(&bx, by_tag(&dom, "div"));
    let (a, b) = (&f.children[0], &f.children[1]);
    assert!(
        approx(b.rect.y - a.rect.y - a.rect.height, 10.0),
        "row-gap dy={}",
        b.rect.y - a.rect.y
    );
}

#[test]
fn flex_wrap_align_content_center() {
    // `align-content: center` sur conteneur à hauteur fixe :
    // le bloc de lignes est centré dans l'axe cross.
    let (dom, bx) = laid(
        "<!doctype html><div style=\"display:flex;flex-wrap:wrap;\
           height:200px;align-content:center\">\
           <div style=\"width:600px\">a</div>\
           <div style=\"width:600px\">b</div></div>",
        "",
    );
    let f = find(&bx, by_tag(&dom, "div"));
    let a = &f.children[0];
    let expected = (200.0 - 2.0 * a.rect.height) / 2.0;
    assert!(
        approx(a.rect.y, expected),
        "y={} attendu {expected}",
        a.rect.y
    );
}

#[test]
fn flex_wrap_reverse_inverse_le_cross() {
    // `wrap-reverse` : la première ligne part du BAS de la
    // zone cross (hauteur définie requise pour voir l'effet).
    let (dom, bx) = laid(
        "<!doctype html><div style=\"display:flex;flex-wrap:wrap-reverse;\
           height:100px\">\
           <div style=\"width:600px\">a</div>\
           <div style=\"width:600px\">b</div></div>",
        "",
    );
    let f = find(&bx, by_tag(&dom, "div"));
    let (a, b) = (&f.children[0], &f.children[1]);
    // a (1re ligne) se colle en bas, b au-dessus.
    assert!(
        approx(a.rect.y + a.rect.height, 100.0),
        "a en bas y={} h={}",
        a.rect.y,
        a.rect.height
    );
    assert!(
        approx(b.rect.y, 100.0 - 2.0 * a.rect.height),
        "b y={}",
        b.rect.y
    );
}

#[test]
fn flex_wrap_justify_par_ligne() {
    // `justify-content: flex-end` s'applique à chaque ligne.
    let (dom, bx) = laid(
        "<!doctype html><div style=\"display:flex;flex-wrap:wrap;\
           justify-content:flex-end\">\
           <div style=\"width:300px\">a</div>\
           <div style=\"width:300px\">b</div>\
           <div style=\"width:300px\">c</div></div>",
        "",
    );
    let f = find(&bx, by_tag(&dom, "div"));
    let c = &f.children[2];
    assert!(
        approx(c.rect.x + c.rect.width, VIEW),
        "c fin x={}",
        c.rect.x
    );
    assert!(c.rect.y > 0.0, "c ligne 2");
}

// ── R-19 : hit-test sur paint_bounds ────────────────────────

#[test]
fn hit_test_enfant_echappe_du_parent() {
    // Enfant `position:absolute` POSÉ HORS du border-box du
    // parent (lite.ddg : `<center>`+`<form>`) — l'élagage sur
    // le rect propre tuait la zone. `paint_bounds` (union du
    // sous-arbre) garde la branche testable.
    let (dom, bx) = laid(
        "<div id=p style=\"position:relative;height:50px\">\
         <a id=c style=\"position:absolute;top:80px;left:10px;width:20px;height:20px\"></a>\
         </div>",
        "body{margin:0}",
    );
    let want = by_tag(&dom, "a");
    assert_eq!(
        explorer_layout::hit_test(&bx, 15.0, 90.0),
        Some(want),
        "point (15,90) hors du parent mais dans l'enfant → hit"
    );
}

#[test]
fn hit_test_profond_normal_inchange() {
    // Point dans un enfant en flux : la boîte la plus
    // imbriquée gagne (contrat inchangé).
    let (dom, bx) = laid(
        "<div id=o><p id=i>x</p></div>",
        "body{margin:0} #o{height:60px} p{height:20px;margin:0}",
    );
    // Le nœud gagnant = le texte de `p` (le plus profond) —
    // son parent DOM est `p` (contrat inchangé).
    let p = by_tag(&dom, "p");
    let hit = explorer_layout::hit_test(&bx, 5.0, 5.0).expect("hit");
    assert_eq!(dom.parent(hit).ok().flatten(), Some(p), "hit {hit:?}");
}

#[test]
fn hit_test_point_hors_document() {
    let (_dom, bx) = laid("<div></div>", "body{margin:0} div{height:20px}");
    assert_eq!(explorer_layout::hit_test(&bx, 5.0, 5000.0), None);
}
