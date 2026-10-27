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
use explorer_layout::{
    build_box_tree, build_box_tree_sized, layout_tree, BlockCtx, BoxKind, LayoutBox,
};

const VIEW: f32 = 800.0;

/// Pipeline complet : html + css → arbre de boîtes posé.
fn laid(html: &str, css: &str) -> (Dom, LayoutBox) {
    let dom = TreeBuilder::parse(html);
    let mut eng = StyleEngine::new();
    // R-32 : la feuille UA pose `body { margin: 8px }` —
    // les fixtures de ce fichier supposent une marge nulle
    // (géométrie à coordonnées fixes). Neutralisée ici ;
    // une règle du `css` testé garderait le dernier mot.
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

/// Dernier enfant élément récursif (descente du sous-arbre).
fn deepest(b: &LayoutBox) -> &LayoutBox {
    match b.children.last() {
        Some(c) if c.node.is_some() => deepest(c),
        _ => b,
    }
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

/// Vol 2026-10-26 — `pointer-events: none` : un overlay
/// transparent couvrant un champ ne vole plus le hit — le
/// `<input>` sous-jacent reste `editable` (service-public,
/// bookshop : `editable:false` sur champs peints).
#[test]
fn hit_test_pointer_events_none_transparent() {
    let (dom, bx) = laid(
        "<div id=o style=\"position:absolute;inset:0;width:100px;height:100px\"></div>\
         <input id=i style=\"width:80px;height:20px\">",
        "body{margin:0} #o{pointer-events:none} input{position:absolute;top:0;left:0}",
    );
    let input = by_tag(&dom, "input");
    assert_eq!(
        explorer_layout::hit_test(&bx, 10.0, 10.0),
        Some(input),
        "overlay pointer-events:none : l'input sous-jacent gagne"
    );
}

/// `pointer-events: none` n'interdit que la cible — un
/// enfant portant `auto` explicite reste ciblable (spec :
/// la propriété s'hérite mais se réactive par enfant).
#[test]
fn hit_test_pointer_events_enfant_reactive() {
    let (dom, bx) = laid(
        "<div id=o style=\"position:absolute;inset:0;width:100px;height:100px\">\
         <a id=a href=# style=\"pointer-events:auto;display:block;width:20px;height:20px\">x</a>\
         </div><p id=u style=\"width:50px;height:50px\">y</p>",
        "body{margin:0} #o{pointer-events:none} p{position:absolute;top:0;left:0}",
    );
    let a = by_tag(&dom, "a");
    // (5,5) est dans le <a> ET dans le <p> dessous : le
    // réactivé gagne — hors de lui, le parent `none` est
    // sauté et le <p> sous-jacent encaisse.
    let hit = explorer_layout::hit_test(&bx, 5.0, 5.0);
    // Le nœud gagnant peut être le texte de `<a>` — l'élément
    // ancêtre porteur du `auto` est la cible sémantique.
    let mut cur = hit;
    let elem = loop {
        let Some(n) = cur else { break None };
        if dom.tag_name(n).is_ok() {
            break Some(n);
        }
        cur = dom.parent(n).ok().flatten();
    };
    assert_eq!(
        elem,
        Some(a),
        "enfant pointer-events:auto ciblable : {hit:?}"
    );
    let p = by_tag(&dom, "p");
    let hit = explorer_layout::hit_test(&bx, 40.0, 40.0);
    let node = hit.and_then(|n| {
        let mut cur = n;
        loop {
            match dom.tag_name(cur) {
                Ok(_) => return Some(cur),
                Err(_) => cur = dom.parent(cur).ok().flatten()?,
            }
        }
    });
    assert_eq!(
        node,
        Some(p),
        "zone overlay hors <a> → p sous-jacent : {hit:?}"
    );
}

#[test]
fn hit_test_transform_translate_suit_la_peinture() {
    // Vol : `transform` appliqué à la display list mais
    // ignoré du hit-test → zones cliquables décalées. La
    // matrice inverse du style (`explorer_layout::transform`)
    // ramène le point sondé dans le repère de la boîte.
    let (dom, bx) = laid(
        "<a id=t href=#>x</a>",
        "body{margin:0} a{display:block;position:absolute;left:10px;top:10px;\
         width:40px;height:20px;transform:translate(100px,50px)}",
    );
    let a = by_tag(&dom, "a");
    // Position PEINTE : (110,60) — le rect layout reste
    // (10,10). Le hit doit suivre la peinture.
    let hit = explorer_layout::hit_test(&bx, 115.0, 65.0);
    let el = hit.and_then(|n| {
        if dom.element(n).is_ok() {
            Some(n)
        } else {
            dom.parent(n).ok().flatten()
        }
    });
    assert_eq!(el, Some(a), "hit {hit:?} au point peint");
    // Position LAYOUT (rect brut, avant transform) : le
    // point ne doit plus retomber sur `a`.
    let el2 = explorer_layout::hit_test(&bx, 15.0, 15.0).and_then(|n| {
        if dom.element(n).is_ok() {
            Some(n)
        } else {
            dom.parent(n).ok().flatten()
        }
    });
    assert_ne!(el2, Some(a), "le rect non transformé ne doit plus hit `a`");
}

#[test]
fn hit_test_transform_rotate_90_inverse_les_axes() {
    // Rotation 90° : la zone cliquable suit le quart de
    // tour. Le point sondé est calculé par la **même**
    // `box_transform` que la peinture (source unique
    // `explorer_layout::transform`) — parité garantie par
    // construction, sans supposer la convention d'origine.
    let (dom, bx) = laid(
        "<a id=t href=#>x</a>",
        "body{margin:0} a{display:block;position:absolute;left:0;top:0;\
         width:100px;height:20px;transform:rotate(90deg)}",
    );
    let a = by_tag(&dom, "a");
    let a_bx = find(&bx, a);
    let t = explorer_layout::box_transform(a_bx, &a_bx.rect).expect("matrice");
    // Coins transformés du rect : la sonde vise le centre
    // de la zone PEINTE.
    let (cx, cy) = t.apply(a_bx.rect.x + 25.0, a_bx.rect.y + 10.0);
    let hit = explorer_layout::hit_test(&bx, cx, cy).and_then(|n| {
        if dom.element(n).is_ok() {
            Some(n)
        } else {
            dom.parent(n).ok().flatten()
        }
    });
    assert_eq!(hit, Some(a), "hit ({cx},{cy}) sur la zone peinte");
    // Le centre du rect LAYOUT peut sortir de la surface
    // peinte — là, `a` ne doit pas gagner.
    let (ux, uy) = (
        a_bx.rect.x + a_bx.rect.width / 2.0,
        a_bx.rect.y + a_bx.rect.height / 2.0,
    );
    let inv = t.invert().expect("inversible");
    let (lx, ly) = inv.apply(ux, uy);
    let inside = lx >= a_bx.rect.x
        && lx <= a_bx.rect.x + a_bx.rect.width
        && ly >= a_bx.rect.y
        && ly <= a_bx.rect.y + a_bx.rect.height;
    if !inside {
        let el = explorer_layout::hit_test(&bx, ux, uy).and_then(|n| {
            if dom.element(n).is_ok() {
                Some(n)
            } else {
                dom.parent(n).ok().flatten()
            }
        });
        assert_ne!(el, Some(a), "centre layout hors surface → pas de hit `a`");
    }
}

#[test]
fn hit_test_transforms_imbriquees_accumulent() {
    // Parent `translate(50,0)` + enfant `translate(0,30)` :
    // la zone cliquable cumule les deux matrices — le point
    // peint (rect + (50,30)) hit, le point layout nu non.
    let (dom, bx) = laid(
        "<div id=p><a id=c href=#>x</a></div>",
        "body{margin:0} #p{position:absolute;left:0;top:0;width:60px;height:60px;\
         transform:translate(50px,0)} \
         #c{display:block;position:absolute;left:5px;top:5px;width:30px;\
         height:20px;transform:translate(0,30px)}",
    );
    let c = by_tag(&dom, "a");
    // Rect layout de `c` : (5,5,30,20) — peint à
    // (55,35)-(85,55).
    let hit = explorer_layout::hit_test(&bx, 60.0, 40.0).and_then(|n| {
        if dom.element(n).is_ok() {
            Some(n)
        } else {
            dom.parent(n).ok().flatten()
        }
    });
    assert_eq!(hit, Some(c), "hit cumulé parent+enfant");
}

// ── R-20 : jamais-rendus ne produisent aucune boîte ─────────

#[test]
fn script_ne_produit_pas_de_boite_meme_sans_style() {
    // Filet R-20 : `<script>` injecté sans cascade (style
    // absent de la map, ou UA sans la règle) → aucune boîte,
    // son texte JSON n'apparaît jamais.
    let mut dom = TreeBuilder::parse("<body><div>x</div></body>");
    let eng = StyleEngine::new(); // pas de CSS auteur ; UA inclus
    let styles = eng.compute_tree(&dom, dom.document(), &StyleEnv::default());
    // Injection post-cascade : le `<script>` n'a AUCUN style
    // calculé (le cas wikipedia — nœud posé par apply_dom_sync).
    let body = by_tag(&dom, "body");
    let script = dom.create_element("script").unwrap();
    dom.append_child(body, script).unwrap();
    let text = dom.create_text("var a=1;").unwrap();
    dom.append_child(script, text).unwrap();
    let root_el = by_tag(&dom, "html");
    let bx = build_box_tree(&dom, root_el, &styles).expect("arbre");
    let mut texts = Vec::new();
    let mut stack = vec![&bx];
    while let Some(b) = stack.pop() {
        if let Some(t) = &b.text {
            texts.push(t.clone());
        }
        stack.extend(b.children.iter());
    }
    assert!(
        !texts.iter().any(|t| t.contains("var a")),
        "script peint : {texts:?}"
    );
}

// ── R-13 : pseudo-éléments ::before/::after ──────────────────

#[test]
fn pseudo_elements_generent_des_boites_bordees() {
    // Acid2 « nose » : ::before/::after à contenu vide dont les
    // bordures dessinent — la boîte doit exister même sans texte.
    let dom =
        TreeBuilder::parse("<html><body><div id=n><div><div></div></div></div></body></html>");
    let mut eng = StyleEngine::new();
    eng.add_css(
        "#n div div::before { display:block; content:''; border:1em solid; height:0 }
         #n div div::after { content:' X '; display:inline }
         #n { content:'non' }",
        Origin::Author,
    );
    let styles = eng.compute_tree(&dom, dom.document(), &StyleEnv::default());
    let root = dom.document_element().expect("el");
    let mut bx = build_box_tree(&dom, root, &styles).expect("arbre");
    layout_tree(&mut bx, VIEW, &BlockCtx::default());
    // Le ::before est premier enfant du div le plus profond.
    let leaf = deepest(&bx);
    let before = &leaf.children[0];
    assert_eq!(before.node, None, "pseudo n'a pas de nœud DOM");
    assert!(
        before.rect.height >= 32.0,
        "bordures 2em posées : {:#?}",
        before.rect
    );
    let after = leaf.children.last().expect("after");
    assert_eq!(after.node, None);
    // `content:'non'` sur un élément (pas pseudo) : ignoré —
    // pas de boîte supplémentaire sous bx.
    assert!(bx.children.iter().all(|c| c.node.is_some()));
}

#[test]
fn pseudo_content_alt_text_non_peint() {
    // R-80 — CSS Content-3 : `content: <valeur> / <alt>` — le
    // texte après le `/` est l'alternative d'accessibilité,
    // jamais peinte (`content:"\200b"/" (external)"` de MDN
    // rendait « (external) » en clair, issue #14).
    let dom = TreeBuilder::parse("<html><body><a id=x href=/u>lien</a></body></html>");
    let mut eng = StyleEngine::new();
    eng.add_css("#x::after { content:' Z' / ' (external)' }", Origin::Author);
    let styles = eng.compute_tree(&dom, dom.document(), &StyleEnv::default());
    let root = dom.document_element().expect("el");
    let bx = build_box_tree(&dom, root, &styles).expect("arbre");
    // Cherche le pseudo ::after : sa boîte n'a pas de nœud DOM
    // et son enfant texte = « Z » seul.
    let mut stack = vec![&bx];
    let mut found = false;
    while let Some(b) = stack.pop() {
        if b.node.is_none() && b.children.iter().any(|c| c.text.is_some()) {
            let t = b.children.iter().find_map(|c| c.text.as_ref());
            assert_eq!(t.map(String::as_str), Some(" Z"), "alt jamais peint");
            found = true;
        }
        stack.extend(b.children.iter());
    }
    assert!(found, "boîte ::after produite");
    // Variante : pas de `/` → toutes les chaînes concaténées
    // (comportement historique conservé).
    let mut eng2 = StyleEngine::new();
    eng2.add_css("#x::after { content:' A' ' B' }", Origin::Author);
    let styles = eng2.compute_tree(&dom, dom.document(), &StyleEnv::default());
    let bx2 = build_box_tree(&dom, root, &styles).expect("arbre2");
    let mut stack = vec![&bx2];
    let mut texts = String::new();
    while let Some(b) = stack.pop() {
        if b.node.is_none() {
            for c in &b.children {
                if let Some(t) = &c.text {
                    texts.push_str(t);
                }
            }
        }
        stack.extend(b.children.iter());
    }
    assert_eq!(texts, " A B");
}

// ── R-13c : <object> repli vs remplacé ───────────────────────

#[test]
fn object_resolu_est_remplace_sinon_repli_enfants() {
    let dom = TreeBuilder::parse(
        "<html><body>\
         <object id=ok><span>NON</span></object>\
         <object id=ko><span>REPLI</span></object>\
         </body></html>",
    );
    // `id=ok` simule une ressource résolue via `sizes`.
    let ok = {
        let mut q = vec![dom.document()];
        let mut found = None;
        while let Some(n) = q.pop() {
            if dom.attribute(n, "id").ok().flatten().as_deref() == Some("ok") {
                found = Some(n);
            }
            if let Ok(c) = dom.children(n) {
                q.extend(c);
            }
        }
        found.expect("ok")
    };
    let mut sizes = std::collections::HashMap::new();
    sizes.insert(ok, (7.0, 5.0));
    let styles = StyleEngine::new().compute_tree(&dom, dom.document(), &StyleEnv::default());
    let root = dom.document_element().unwrap();
    let mut bx = build_box_tree_sized(&dom, root, &styles, &sizes).unwrap();
    layout_tree(&mut bx, VIEW, &BlockCtx::default());
    let ok_b = find(&bx, ok);
    assert_eq!(ok_b.intrinsic, Some((7.0, 5.0)));
    assert!(ok_b.children.is_empty(), "remplacé : pas d'enfants");
    // Le repli rend le contenu.
    let ko = {
        let mut q = vec![dom.document()];
        let mut found = None;
        while let Some(n) = q.pop() {
            if dom.attribute(n, "id").ok().flatten().as_deref() == Some("ko") {
                found = Some(n);
            }
            if let Ok(c) = dom.children(n) {
                q.extend(c);
            }
        }
        found.unwrap()
    };
    let ko_b = find(&bx, ko);
    assert!(ko_b.intrinsic.is_none(), "repli : pas d'intrinsèque");
    assert!(!ko_b.children.is_empty(), "repli : enfants rendus");
}

#[test]
fn marge_basse_dernier_fils_reste_dans_conteneur_borde() {
    // R-27 : §8.3.1 — la marge basse du dernier fils en flux
    // ne s'échappe que si rien ne la sépare de celle du
    // conteneur. Avec `border-bottom` (ou padding/BFC) elle
    // compte dans la hauteur de contenu — les réf. WPT
    // `flex-direction-column(-reverse)` en dépendent.
    let (dom, bx) = laid(
        "<!doctype html><div><p>x</p></div>",
        "body{margin:0} div{border:1px solid black} p{height:10px;margin:0 0 20px}",
    );
    let c = find(&bx, by_tag(&dom, "div"));
    let p = find(&bx, by_tag(&dom, "p"));
    // border-box = 1 + 10 + 20(margin inside) + 1 = 32.
    assert!(approx(c.rect.height, 32.0), "h={}", c.rect.height);
    // La margin box du fils aboutit au bas du contenu.
    assert!(
        approx(
            p.rect.y + p.rect.height + 20.0,
            c.rect.y + c.rect.height - 1.0
        ),
        "p={:?} c={:?}",
        p.rect,
        c.rect
    );
}

#[test]
fn marge_basse_dernier_fils_sechappe_sans_separateur() {
    // Cas témoin : sans bordure ni padding, la marge basse
    // du dernier fils collapse par-dessous (hauteur inchangée).
    let (dom, bx) = laid(
        "<!doctype html><div><p>x</p></div>",
        "body{margin:0} p{height:10px;margin:0 0 20px}",
    );
    let c = find(&bx, by_tag(&dom, "div"));
    assert!(approx(c.rect.height, 10.0), "h={}", c.rect.height);
}

#[test]
fn marge_haute_premier_fils_reste_dans_bfc() {
    // R-28 : §8.3.1 — un bloc qui établit un BFC (ici
    // `overflow:hidden`) ne collapse PAS avec la marge haute
    // de son premier fils en flux : la marge compte dans la
    // hauteur de contenu (fils posé à mt sous le bord haut).
    let (dom, bx) = laid(
        "<!doctype html><div><p>x</p></div>",
        "body{margin:0} div{overflow:hidden} p{height:10px;margin:14px 0 0}",
    );
    let c = find(&bx, by_tag(&dom, "div"));
    let p = find(&bx, by_tag(&dom, "p"));
    assert!(approx(c.rect.height, 24.0), "h={}", c.rect.height);
    assert!(approx(p.rect.y, 14.0), "p.y={}", p.rect.y);
}

/// Variante de `laid` avec tailles intrinsèques injectées
/// (`build_box_tree_sized`) pour les éléments remplacés.
fn laid_sized(
    html: &str,
    css: &str,
    sizes: &std::collections::HashMap<explorer_dom::NodeId, (f32, f32)>,
) -> (Dom, LayoutBox) {
    let dom = TreeBuilder::parse(html);
    let mut eng = StyleEngine::new();
    eng.add_css("body { margin: 0 }", Origin::Author);
    eng.add_css(css, Origin::Author);
    let styles = eng.compute_tree(&dom, dom.document(), &StyleEnv::default());
    let root = dom.document_element().expect("racine");
    let mut bx = build_box_tree_sized(&dom, root, &styles, sizes).expect("arbre");
    layout_tree(&mut bx, VIEW, &BlockCtx::default());
    (dom, bx)
}

fn ratio_sizes(
    dom: &Dom,
    tag: &str,
    w: f32,
    h: f32,
) -> std::collections::HashMap<explorer_dom::NodeId, (f32, f32)> {
    let mut m = std::collections::HashMap::new();
    m.insert(by_tag(dom, tag), (w, h));
    m
}

#[test]
fn remplace_max_width_bridant_derive_la_hauteur_par_ratio() {
    // Terrain rust-lang.org : `img{max-width:100%}` bride
    // l'icône à la largeur du conteneur → la hauteur doit
    // suivre le ratio (§10.6.2), pas rester à l'intrinsèque.
    // Sans le fix : 100×480 (étiré vertical) au lieu de ~100.
    let html = "<!doctype html><div style=\"width:100px\"><img style=\"max-width:100%\"></div>";
    let dom_probe = TreeBuilder::parse(html);
    let sizes = ratio_sizes(&dom_probe, "img", 480.0, 480.0);
    let (dom, bx) = {
        // `sizes` indexe les NodeId du dom parsé par le probe —
        // `laid_sized` reparse le même html → ids identiques
        // (parsage déterministe).
        laid_sized(html, "", &sizes)
    };
    let img = find(&bx, by_tag(&dom, "img"));
    assert!(approx(img.rect.width, 100.0), "w={}", img.rect.width);
    assert!(approx(img.rect.height, 100.0), "h={}", img.rect.height);
}

/// Vol 2026-10-26 — `<svg viewBox>` sans `width` : la taille
/// N'EST PAS le viewBox (logo w3.org géant) — défaut remplacé
/// `min(300px, 100%)` (comportement Chrome ; vol 25 kayak :
/// l'icône `⇄` ne doit plus remplir sa colonne de 325 px),
/// `height:auto` dérivé du ratio.
#[test]
fn svg_viewbox_seul_borne_a_300px() {
    let (dom, bx) = laid(
        "<div style=\"width:400px\"><svg viewBox=\"0 0 200 100\"></svg></div>",
        "body{margin:0}",
    );
    let svg = find(&bx, by_tag(&dom, "svg"));
    assert!(approx(svg.rect.width, 300.0), "w={}", svg.rect.width);
    assert!(approx(svg.rect.height, 150.0), "h={}", svg.rect.height);
}

/// Conteneur plus étroit que 300 px : `min(300px, 100%)`
/// laisse le SVG tenir dans son bloc.
#[test]
fn svg_viewbox_seul_conteneur_etroit() {
    let (dom, bx) = laid(
        "<div style=\"width:200px\"><svg viewBox=\"0 0 200 100\"></svg></div>",
        "body{margin:0}",
    );
    let svg = find(&bx, by_tag(&dom, "svg"));
    assert!(approx(svg.rect.width, 200.0), "w={}", svg.rect.width);
    assert!(approx(svg.rect.height, 100.0), "h={}", svg.rect.height);
}

/// Contrôle : `<svg width/height>` explicites gardent la
/// priorité sur le hint (attributs = déclarations UA).
#[test]
fn svg_dimensions_explicites_inchangees() {
    let (dom, bx) = laid(
        "<div style=\"width:400px\"><svg width=\"80\" height=\"40\" viewBox=\"0 0 200 100\"></svg></div>",
        "body{margin:0}",
    );
    let svg = find(&bx, by_tag(&dom, "svg"));
    assert!(approx(svg.rect.width, 80.0), "w={}", svg.rect.width);
    assert!(approx(svg.rect.height, 40.0), "h={}", svg.rect.height);
}

#[test]
fn remplace_height_specifiee_derive_la_largeur_par_ratio() {
    // §10.6.2 symétrique : `height:100px` + `width:auto` →
    // w = h × iw/ih (format paysage 480×240 → w=200).
    let html = "<!doctype html><img style=\"display:block;height:100px\">";
    let dom_probe = TreeBuilder::parse(html);
    let sizes = ratio_sizes(&dom_probe, "img", 480.0, 240.0);
    let (dom, bx) = laid_sized(html, "", &sizes);
    let img = find(&bx, by_tag(&dom, "img"));
    assert!(approx(img.rect.width, 200.0), "w={}", img.rect.width);
    assert!(approx(img.rect.height, 100.0), "h={}", img.rect.height);
}

#[test]
fn remplace_deux_axes_specifies_inchanges() {
    // Carve-out : `width`+`height` explicites gagnent — le
    // ratio ne s'interpose que sur l'axe `auto`.
    let html = "<!doctype html><img style=\"display:block;width:60px;height:40px\">";
    let dom_probe = TreeBuilder::parse(html);
    let sizes = ratio_sizes(&dom_probe, "img", 480.0, 240.0);
    let (dom, bx) = laid_sized(html, "", &sizes);
    let img = find(&bx, by_tag(&dom, "img"));
    assert!(approx(img.rect.width, 60.0), "w={}", img.rect.width);
    assert!(approx(img.rect.height, 40.0), "h={}", img.rect.height);
}

#[test]
fn remplace_inline_atomic_brida_par_max_width() {
    // Forme réelle du bug : <a><img style=max-width:100%></a>
    // dans un conteneur étroit — l'img passe par le chemin
    // atomique inline (size_atomics_walk → layout_block).
    // Intrinsèque portrait 480×511 (mastodon) bridée à 215 :
    // h attendue = 215 × 511/480 ≈ 229, pas 511.
    let html =
        "<!doctype html><div style=\"width:215px\"><a><img style=\"max-width:100%\"></a></div>";
    let dom_probe = TreeBuilder::parse(html);
    let sizes = ratio_sizes(&dom_probe, "img", 480.0, 511.0);
    let (dom, bx) = laid_sized(html, "", &sizes);
    let img = find(&bx, by_tag(&dom, "img"));
    assert!(approx(img.rect.width, 215.0), "w={}", img.rect.width);
    let attendu = 215.0 * 511.0 / 480.0;
    assert!(
        approx(img.rect.height, attendu),
        "h={} vs {}",
        img.rect.height,
        attendu
    );
}

// ── R-43 : `box-sizing` — les tailles spécifiées s'appliquent
// à la border box ; le contenu en est déduit (terrain :
// bouton « Connexion » de google.fr — `min-height:40px` +
// `padding:10px` rendu ~60 px au lieu de 40).

#[test]
fn border_box_width_inclut_padding_et_bordure() {
    // `width:100px` border-box + padding 20 + border 5 →
    // border box 100, contenu 50. Content-box (défaut) → 150.
    let (dom, bx) = laid(
        "<!doctype html><div id=\"t\" style=\"width:100px;padding:0 20px;border:0 solid #000;border-width:0 5px;box-sizing:border-box\">x</div>",
        "",
    );
    let d = find(&bx, by_tag(&dom, "div"));
    assert!(approx(d.rect.width, 100.0), "w={}", d.rect.width);
    let (dom2, bx2) = laid(
        "<!doctype html><div style=\"width:100px;padding:0 20px;border:0 solid #000;border-width:0 5px\">x</div>",
        "",
    );
    let d2 = find(&bx2, by_tag(&dom2, "div"));
    assert!(approx(d2.rect.width, 150.0), "w={}", d2.rect.width);
}

#[test]
fn border_box_min_height_plancher_sur_border_box() {
    // google.fr `.gb_6a` : `min-height:40px` + `padding:10px` +
    // `box-sizing:border-box` → total 40, contenu ≥ 20.
    let (dom, bx) = laid(
        "<!doctype html><div style=\"box-sizing:border-box;min-height:40px;padding:10px 24px;line-height:18px\">x</div>",
        "",
    );
    let d = find(&bx, by_tag(&dom, "div"));
    assert!(approx(d.rect.height, 40.0), "h={}", d.rect.height);
}

#[test]
fn border_box_height_plancher_zero_du_contenu() {
    // `height:30px` border-box + padding 40 → contenu planché
    // à 0 ; css-sizing-3 : la border box résultante DÉPASSE
    // la taille spécifiée (40 — le châssis ne se compresse
    // pas sous la taille demandée).
    let (dom, bx) = laid(
        "<!doctype html><div style=\"box-sizing:border-box;height:30px;padding:20px\">x</div>",
        "",
    );
    let d = find(&bx, by_tag(&dom, "div"));
    assert!(approx(d.rect.height, 40.0), "h={}", d.rect.height);
}

#[test]
fn border_box_max_width_et_max_height_bornent_la_border_box() {
    let (dom, bx) = laid(
        "<!doctype html><div style=\"box-sizing:border-box;width:500px;height:500px;max-width:100px;max-height:80px;padding:10px\">x</div>",
        "",
    );
    let d = find(&bx, by_tag(&dom, "div"));
    assert!(approx(d.rect.width, 100.0), "w={}", d.rect.width);
    assert!(approx(d.rect.height, 80.0), "h={}", d.rect.height);
}

// ── R-44 : `width` explicite d'un remplacé fait foi dans la
// mesure intrinsèque — les `<a>` sociaux de rust-lang.org
// (`footer img{width:40px}`) mesuraient l'intrinsèque 512
// comme base flex → chacun > conteneur → wrap : une icône
// par ligne (§9.5 : la ligne se décide sur la base
// hypothétique, avant le shrink).

#[test]
fn remplace_width_specifiee_mesure_la_base_flex() {
    // 4 × `<a><img style="width:40px">` (intrinsèque 512²)
    // dans un `flex-wrap` de 200 px → tous tiennent sur une
    // seule ligne (~40 px de haut) ; avant : base 512 →
    // 4 lignes de ~40 = ~160.
    let html = concat!(
        "<!doctype html><div style=\"display:flex;flex-wrap:wrap;width:200px\">",
        "<a><img style=\"width:40px\"></a>",
        "<a><img style=\"width:40px\"></a>",
        "<a><img style=\"width:40px\"></a>",
        "<a><img style=\"width:40px\"></a>",
        "</div>"
    );
    let probe = TreeBuilder::parse(html);
    let mut sizes = std::collections::HashMap::new();
    let mut queue = vec![probe.document()];
    while let Some(n) = queue.pop() {
        if probe.tag_name(n).is_ok_and(|t| t == "img") {
            sizes.insert(n, (512.0, 512.0));
        }
        if let Ok(ch) = probe.children(n) {
            queue.extend(ch.iter().rev());
        }
    }
    let (dom, bx) = laid_sized(html, "", &sizes);
    let container = find(&bx, by_tag(&dom, "div"));
    assert!(
        container.rect.height < 60.0,
        "items attendus sur une ligne, h={}",
        container.rect.height
    );
    let a = find(&bx, by_tag(&dom, "a"));
    assert!(approx(a.rect.width, 40.0), "a.w={}", a.rect.width);
}

#[test]
fn remplace_max_width_px_borne_la_mesure_intrinseque() {
    // `img` sans `width` mais `max-width` en px : la borne
    // s'applique à la contribution intrinsèque (§5.1) — un
    // item flex ne dépasse pas la borne par sa base.
    let html = concat!(
        "<!doctype html><div style=\"display:flex;width:200px\">",
        "<a><img style=\"max-width:30px\"></a>",
        "</div>"
    );
    let probe = TreeBuilder::parse(html);
    let sizes = ratio_sizes(&probe, "img", 512.0, 512.0);
    let (dom, bx) = laid_sized(html, "", &sizes);
    let a = find(&bx, by_tag(&dom, "a"));
    assert!(a.rect.width <= 31.0, "a.w={}", a.rect.width);
}

// ── R-45 : `float:right` translate le sous-arbre — la boîte
// du flottant partait au bord droit mais ses descendants
// restaient au bord gauche de la bande (`rect.x` corrigé
// sans translation — même motif que l'ancrage right/bottom
// des absolus). Terrain : thumbs Wikipedia ITN/OTD peints
// à gauche, texte chevauché.

#[test]
fn float_right_translate_tout_le_sous_arbre() {
    let html = concat!(
        "<!doctype html><div style=\"width:400px\">",
        "<aside class=\"itn-img\"><section style=\"width:143px;height:60px\"></section></aside>",
        "<p>alpha beta gamma delta epsilon zeta eta theta</p>",
        "</div>"
    );
    let (dom, bx) = laid(html, ".itn-img{float:right;margin-left:8px;margin-top:3px}");
    let outer = find(&bx, by_tag(&dom, "aside"));
    let inner = find(&bx, by_tag(&dom, "section"));
    // Boîte ET contenu collés au bord droit (400).
    assert!(
        approx(outer.rect.x + outer.rect.width, 400.0),
        "outer.right={}",
        outer.rect.x + outer.rect.width
    );
    assert!(
        approx(inner.rect.x + inner.rect.width, 400.0),
        "inner.right={} (sous-arbre non translaté)",
        inner.rect.x + inner.rect.width
    );
}

#[test]
fn float_left_inchange() {
    // Contrôle : le flottant gauche n'a aucune translation
    // à appliquer — sa boîte et son contenu restent au bord
    // gauche de la bande.
    let html = concat!(
        "<!doctype html><div style=\"width:400px\">",
        "<aside class=\"fl\"><section style=\"width:100px;height:50px\"></section></aside>",
        "</div>"
    );
    let (dom, bx) = laid(html, ".fl{float:left;margin-right:8px}");
    let inner = find(&bx, by_tag(&dom, "section"));
    assert!(approx(inner.rect.x, 0.0), "inner.x={}", inner.rect.x);
}

// ── R-53 : `width:%` d'un flottant se résout sur le bloc
// conteneur, jamais sur la bande libre (§10.3 : la base d'un
// % est la largeur du bloc contenant — la bande ne borne que
// le shrink-to-fit `width:auto`, §10.3.5). Terrain : le
// squelette colonnes de weather.gov (`float:left;width:100%`
// sur `.div-full`/`.full-width-*`) s'effondrait à la bande
// résiduelle — colonne de ~1 caractère/ligne.

#[test]
fn float_width_pourcentage_base_bloc_conteneur() {
    let html = concat!(
        "<!doctype html><div style=\"width:400px\">",
        "<aside class=\"l\"><section style=\"width:100px;height:50px\"></section></aside>",
        "<nav class=\"w\"><section style=\"height:30px\"></section></nav>",
        "</div>"
    );
    let (dom, bx) = laid(html, ".l{float:left} .w{float:left;width:100%}");
    let w = find(&bx, by_tag(&dom, "nav"));
    // 100 % du conteneur (400), pas de la bande (300) — et le
    // flottant descend sous le voisin qu'il ne peut doubler.
    assert!(approx(w.rect.width, 400.0), "w={}", w.rect.width);
    assert!(w.rect.y >= 50.0, "y={} (pas descendu)", w.rect.y);
    assert!(approx(w.rect.x, 0.0), "x={}", w.rect.x);
}

#[test]
fn float_width_pourcentage_partiel_base_bloc_conteneur() {
    // Contrôle partiel : 50 % = 200 du conteneur, pas 150 de
    // la bande ; la bande de 300 peut l'accueillir → reste
    // en ligne à droite du voisin.
    let html = concat!(
        "<!doctype html><div style=\"width:400px\">",
        "<aside class=\"l\"><section style=\"width:100px;height:50px\"></section></aside>",
        "<nav class=\"w\"><section style=\"height:30px\"></section></nav>",
        "</div>"
    );
    let (dom, bx) = laid(html, ".l{float:left} .w{float:left;width:50%}");
    let w = find(&bx, by_tag(&dom, "nav"));
    assert!(approx(w.rect.width, 200.0), "w={}", w.rect.width);
    assert!(approx(w.rect.x, 100.0), "x={}", w.rect.x);
}

// ── R-46 : un item flex/grid établit un contexte de
// formatage indépendant (Flexbox §4.1 / Grid §6) — les
// marges de ses descendants ne s'échappent jamais de
// l'item (ni haut ni bas) et il englobe ses flottants.
// Terrain : la marge `1em` du premier `<p>` d'un item
// s'évaporait dans la mesure flex (item posé sans elle).

#[test]
fn item_flex_contient_les_marges_de_ses_fils() {
    let html = concat!(
        "<!doctype html><div class=\"fc\">",
        "<div class=\"it\"><p>aaa</p></div>",
        "<div class=\"nx\">bbb</div>",
        "</div>"
    );
    let (dom, bx) = laid(
        html,
        ".fc{display:flex;flex-direction:column} p{margin:16px 0}",
    );
    // `.it` = second div du doc (le 1er est `.fc`).
    let mut divs = Vec::new();
    let mut queue = vec![dom.document()];
    while let Some(n) = queue.pop() {
        if dom.tag_name(n).is_ok_and(|t| t == "div") {
            divs.push(n);
        }
        if let Ok(ch) = dom.children(n) {
            queue.extend(ch.iter().rev());
        }
    }
    let it = find(&bx, divs[1]);
    let nx = find(&bx, divs[2]);
    let p = find(&bx, by_tag(&dom, "p"));
    // Marge contenue : le <p> commence à 16 sous le haut de
    // l'item ; l'item englobe la marge basse → h = 51.2.
    assert!(
        approx(p.rect.y - it.rect.y, 16.0),
        "p.y-it.y={}",
        p.rect.y - it.rect.y
    );
    assert!(approx(it.rect.height, 51.2), "it.h={}", it.rect.height);
    // L'item suivant est posé APRÈS la margin box de `.it`.
    assert!(approx(nx.rect.y, it.rect.y + 51.2), "nx.y={}", nx.rect.y);
}

#[test]
fn item_flex_row_marges_dans_le_cross() {
    // En `row`, la hauteur cross de l'item englobe les
    // marges verticales contenues (§4.1 — pas de collapse).
    let html = "<!doctype html><div class=\"fc\"><div class=\"it\"><p>aaa</p></div></div>";
    let (dom, bx) = laid(
        html,
        ".fc{display:flex;flex-direction:row} p{margin:16px 0}",
    );
    let p = find(&bx, by_tag(&dom, "p"));
    assert!(approx(p.rect.y, 16.0), "p.y={}", p.rect.y);
}

// ── Issue #12 : `margin:auto` + `max-width` — §10.4 rejoue
// §10.3.3 avec la largeur bornée : le slack résiduel va aux
// marges `auto` (la boîte se centre). Avant le fix, la
// distribution se faisait sur la largeur tentative AVANT le
// bridage — les marges restaient à 0 (colonne collée à
// gauche ; terrain : colonne `max-width` de text.npr.org).

#[test]
fn margin_auto_max_width_centre_la_boite() {
    let html = concat!(
        "<!doctype html><div style=\"width:800px\">",
        "<article style=\"max-width:600px;margin:0 auto;height:50px\"></article>",
        "</div>"
    );
    let (dom, bx) = laid(html, "");
    let a = find(&bx, by_tag(&dom, "article"));
    assert!(approx(a.rect.width, 600.0), "w={}", a.rect.width);
    assert!(approx(a.rect.x, 100.0), "x={} (pas centré)", a.rect.x);
}

#[test]
fn margin_auto_une_seule_absorbe() {
    // `margin-left:auto` seul : tout le slack à gauche → la
    // boîte est poussée au bord droit.
    let html = concat!(
        "<!doctype html><div style=\"width:800px\">",
        "<article style=\"width:200px;margin-left:auto;height:50px\"></article>",
        "</div>"
    );
    let (dom, bx) = laid(html, "");
    let a = find(&bx, by_tag(&dom, "article"));
    assert!(approx(a.rect.x, 600.0), "x={}", a.rect.x);
}

#[test]
fn margin_auto_width_auto_reste_nulle() {
    // `width:auto` sans borne : les marges `auto` valent 0
    // (§10.3.3) — la boîte remplit le conteneur.
    let html = concat!(
        "<!doctype html><div style=\"width:800px\">",
        "<article style=\"margin:0 auto;height:50px\"></article>",
        "</div>"
    );
    let (dom, bx) = laid(html, "");
    let a = find(&bx, by_tag(&dom, "article"));
    assert!(approx(a.rect.x, 0.0), "x={}", a.rect.x);
    assert!(approx(a.rect.width, 800.0), "w={}", a.rect.width);
}

// ── Issue #12 : mesure intrinsèque bornée par `max-width`
// (Sizing §5.1 : la contribution est bridée par les bornes
// de la boîte). Wikipedia `#mp-tfa-img` : flottant
// shrink-to-fit contenant `thumbinner{max-width:170px}` —
// la légende (~350px max-content) gonflait le flottant à
// sa mesure brute → le texte voisin n'avait plus que la
// bande résiduelle (« étranglé »).

#[test]
fn float_max_width_enfant_borne_le_shrink() {
    let html = concat!(
        "<!doctype html><div style=\"width:800px\">",
        "<aside style=\"float:right;margin-left:20px\">",
        "<section style=\"max-width:170px\">",
        "<div>very long caption text that exceeds one hundred seventy pixels wide by far indeed</div>",
        "</section></aside>",
        "<p>body text beside the float</p></div>"
    );
    let (dom, bx) = laid(html, "");
    let a = find(&bx, by_tag(&dom, "aside"));
    assert!(
        approx(a.rect.width, 170.0),
        "flottant w={} (non borné)",
        a.rect.width
    );
}

#[test]
fn float_min_width_enfant_etend_le_shrink() {
    // Symétrique : `min-width` fait aussi foi en mesure
    // intrinsèque — le flottant ne descend pas sous 200.
    let html = concat!(
        "<!doctype html><div style=\"width:800px\">",
        "<aside style=\"float:left\">",
        "<section style=\"min-width:200px\"><div>tiny</div></section>",
        "</aside><p>x</p></div>"
    );
    let (dom, bx) = laid(html, "");
    let a = find(&bx, by_tag(&dom, "aside"));
    assert!(
        a.rect.width >= 200.0 - 0.01,
        "flottant w={} (min-width ignoré)",
        a.rect.width
    );
}

// ── R-77 : un flottant shrink-to-fit taillé pile à son
// max-content ne doit pas couper son propre texte pour un
// ulp de mesure (issue #13 — nav weather.gov « PAST
// WEATHER »). Le max-content mesure la chaîne entière
// (collapse_ws → un seul appel shaper) alors que le
// line-break somme les pièces : deux ordres d'arrondi f32
// distincts. `font-size:10.001px` rend la divergence
// déterministe sans fonte : `mesure(13c) = 71.507148…`
// < `mesure(6c)+mesure(7c) = 71.507156…` (~7.6e-6).

#[test]
fn float_shrink_to_fit_tolere_ulp_mesure_intrinseque() {
    let html = concat!(
        "<!doctype html><div style=\"width:800px\">",
        "<a style=\"float:left;font-size:10.001px\">AAAAA BBBBBBB</a>",
        "</div>"
    );
    let (dom, bx) = laid(html, "");
    let a = find(&bx, by_tag(&dom, "a"));
    let total: usize = a.lines.len() + a.children.iter().map(|c| c.lines.len()).sum::<usize>();
    assert_eq!(total, 1, "texte coupé en {total} lignes (ulp)");
}

// ── R-77 bis : la bande `FloatCtx` sans morsure rend `cw`
// intact — `(cx+cw)-cx` perd un ulp dès que `cx` a des bits
// fractionnaires (rangée de flottants depuis une origine
// négative, `.content{width:960px;margin:auto}` à 800px →
// x=-80). Le flottant imbriqué `width:auto` shrink-to-fit
// doit recevoir la bande exacte.

#[test]
fn float_origine_negative_bande_rend_cw_exact() {
    let html = concat!(
        "<!doctype html><div style=\"width:960px;margin:0 auto\">",
        "<ul style=\"float:left\">",
        "<li style=\"float:left;padding-right:40.25px\">",
        "<a style=\"float:left\">AAAAA BBBBBBB</a></li>",
        "<li style=\"float:left\"><a style=\"float:left\">CC</a></li>",
        "</ul></div>"
    );
    let (dom, bx) = laid(html, "");
    // Le div .content est centré : (800-960)/2 = -80 →
    // coordonnées fractionnaires en cascade.
    let cont = find(&bx, by_tag(&dom, "div"));
    assert!(approx(cont.rect.x, -80.0), "content x={}", cont.rect.x);
    let a = find(&bx, by_tag(&dom, "a"));
    let total: usize = a.lines.len() + a.children.iter().map(|c| c.lines.len()).sum::<usize>();
    assert_eq!(total, 1, "texte coupé en {total} lignes (bande)");
}

// ── R-78 : `letter-spacing` (§16.4) entre dans la mesure
// — la nav weather.gov cumule ~-45px sur la rangée et
// « ABOUT » tient sur la ligne. Sans fonte : mesure =
// `chars × (fs×0.55 + ls)` — déterministe.

#[test]
fn letter_spacing_negatif_resserre_le_flottant() {
    let html = concat!(
        "<!doctype html><div style=\"width:800px\">",
        "<a style=\"float:left;font-size:10px\">AAAAA</a>",
        "<b style=\"float:left;font-size:10px;letter-spacing:-2px\">AAAAA</b>",
        "</div>"
    );
    let (dom, bx) = laid(html, "");
    let a = find(&bx, by_tag(&dom, "a"));
    let b = find(&bx, by_tag(&dom, "b"));
    // a : 5 × 5.5 = 27.5 ; b : 5 × (5.5 − 2) = 17.5.
    assert!(approx(a.rect.width, 27.5), "a w={}", a.rect.width);
    assert!(approx(b.rect.width, 17.5), "b w={}", b.rect.width);
}

#[test]
fn letter_spacing_positif_elargit_le_flottant() {
    let html = concat!(
        "<!doctype html><div style=\"width:800px\">",
        "<a style=\"float:left;font-size:10px;letter-spacing:1px\">AAAAA</a>",
        "</div>"
    );
    let (dom, bx) = laid(html, "");
    let a = find(&bx, by_tag(&dom, "a"));
    // 5 × (5.5 + 1) = 32.5.
    assert!(approx(a.rect.width, 32.5), "a w={}", a.rect.width);
}

#[test]
fn rem_contre_font_size_racine() {
    // CSS Values §5.2 côté propriétés non-font-size : `rem`
    // = font-size calculée de :root (10 px après `62.5%`),
    // pas l'initiale — marges, tailles, line-height suivent.
    // Terrain (dispatch 23/10, « métriques verticales
    // généreuses ») : le portail Wikipedia compose en rem,
    // tout gonflait ×1.6 quand la base 16 servait encore.
    let (dom, bx) = laid(
        "<div id=d>t</div>",
        "html { font-size: 62.5% } #d { margin-left: 2rem; width: 10rem; height: 0.5rem }",
    );
    let d = find(&bx, by_tag(&dom, "div"));
    assert_eq!(d.rect.x, 20.0, "margin-left:2rem → 20px (root fs 10)");
    assert_eq!(d.rect.width, 100.0, "width:10rem → 100px");
    assert_eq!(d.rect.height, 5.0, "height:0.5rem → 5px");
}

#[test]
fn colonnes_distribuent_les_items() {
    // R-89 / terrain 23/10 — accueil FR `.colonnes` :
    // `column-count` répartit les unités bloc en colonnes
    // équilibrées au lieu de les empiler. La partition se
    // fait au premier niveau multi-enfants (ici `ul>li`,
    // un cran sous le conteneur `.c`).
    let (dom, bx) = laid(
        "<div class=c><ul><li>A</li><li>B</li><li>C</li><li>D</li>\
         <li>E</li><li>F</li></ul></div>",
        ".c { column-count: 3; column-gap: 10px; width: 300px } li { height: 20px }",
    );
    let ul = find(&bx, by_tag(&dom, "ul"));
    assert_eq!(ul.children.len(), 6);
    // col_w = (300 - 20)/3 ≈ 93.33 ; 6×20 px, cible 40 → 2/col.
    // La marge UA `ul{margin-top:1em}` décale tout à y=16,
    // le `padding-left:40px` UA décale les colonnes à x=40+.
    let (a, c, e) = (&ul.children[0], &ul.children[2], &ul.children[4]);
    assert!(approx(a.rect.x, 40.0), "col0 x={}", a.rect.x);
    assert!(approx(c.rect.x, 143.333), "col1 x={}", c.rect.x);
    assert!(approx(e.rect.x, 246.667), "col2 x={}", e.rect.x);
    // Chaque colonne repart du haut du conteneur.
    for (i, u) in ul.children.iter().enumerate() {
        let want = [16.0, 36.0][i % 2];
        assert!(approx(u.rect.y, want), "li{i} y={}", u.rect.y);
    }
    // Le conteneur couvre les trois colonnes.
    let div = find(&bx, by_tag(&dom, "div"));
    assert!(approx(div.rect.height, 40.0), "div h={}", div.rect.height);
    assert!(approx(ul.rect.height, 40.0), "ul h={}", ul.rect.height);
}

#[test]
fn colonnes_degenerent_sans_niveau_multiple() {
    // R-89 : un seul enfant feuille → pas de partition, la
    // mesure mono-colonne est recalée telle quelle (jamais
    // de panique, jamais de re-flow).
    let (dom, bx) = laid(
        "<div class=c><p>seul</p></div>",
        ".c { column-count: 4; width: 300px } p { height: 30px }",
    );
    let p = find(&bx, by_tag(&dom, "p"));
    assert!(approx(p.rect.x, 0.0), "x={}", p.rect.x);
    assert!(approx(p.rect.height, 30.0), "h={}", p.rect.height);
}

// ── R-101 : searchInput Wikipedia — motif .cdx-text-input ──

#[test]
fn hit_test_input_dans_wrapper_cdx() {
    // Motif Codex/Vector-2022 : le wrapper `.cdx-text-input`
    // (position:relative, bordure peinte sur lui) contient
    // l'input en-flux + une icône absolue. Le clic dans la zone
    // peinte doit atteindre l'`<input>`, pas rester sur le
    // wrapper (dispatch 2026-10-22 : 54 sondes → zéro editable).
    let (dom, bx) = laid(
        "<form><div class=\"cdx-text-input\">\
         <input class=\"cdx-text-input__input\" type=\"search\">\
         <span class=\"cdx-text-input__icon\"></span></div></form>",
        "body{margin:0}\
         .cdx-text-input{position:relative;border:1px solid #000;display:flex}\
         .cdx-text-input__input{width:100%;height:32px;border:0;padding:0 32px}\
         .cdx-text-input__icon{position:absolute;left:4px;top:4px;width:16px;height:16px}",
    );
    let input = by_tag(&dom, "input");
    // Centre du champ (hors icône) → l'input doit gagner.
    assert_eq!(
        explorer_layout::hit_test(&bx, 400.0, 17.0),
        Some(input),
        "le centre du champ stylé doit retomber sur <input>"
    );
}

#[test]
fn hit_test_input_absolu_inset_dans_wrapper() {
    // Variante : l'input est `position:absolute;inset:0` dans
    // le wrapper relatif (peint par-dessus toute la zone).
    let (dom, bx) = laid(
        "<form><div class=\"w\">\
         <input id=i type=\"search\">\
         <span class=\"ic\"></span></div></form>",
        "body{margin:0}\
         .w{position:relative;width:400px;height:32px;border:1px solid #000}\
         #i{position:absolute;left:0;top:0;width:100%;height:100%;border:0;padding:0}\
         .ic{position:absolute;left:4px;top:4px;width:16px;height:16px}",
    );
    let input = by_tag(&dom, "input");
    assert_eq!(
        explorer_layout::hit_test(&bx, 300.0, 16.0),
        Some(input),
        "input absolu doit être atteint"
    );
}

#[test]
fn hit_test_input_height_pourcent_dans_wrapper() {
    // Variante : `height:100%` sur l'input en-flux.
    let (dom, bx) = laid(
        "<form><div class=\"w\">\
         <input id=i type=\"search\"></div></form>",
        "body{margin:0}\
         .w{position:relative;width:400px;height:32px;border:1px solid #000}\
         #i{width:100%;height:100%;border:0;padding:0}",
    );
    let input = by_tag(&dom, "input");
    assert_eq!(
        explorer_layout::hit_test(&bx, 300.0, 16.0),
        Some(input),
        "input height:100% doit être atteint"
    );
}

#[test]
fn hit_test_input_height_pourcent_parent_auto() {
    // Le repro exact du dispatch 2026-10-22 (searchInput
    // Wikipedia) : `height:100%` contre un parent à hauteur
    // **auto** → §10.5 le % vaut `auto` → la boîte contrôle
    // sans contenu mesurait **0 px** et ne couvrait pas la
    // zone peinte par la bordure du wrapper → le hit tombait
    // sur le `div` (champ incliquable). La jambe NBSP du
    // contrôle vide lui rend sa hauteur de ligne (~1.2em).
    let (dom, bx) = laid(
        "<form><div class=\"w\">\
         <input id=i type=\"search\"></div></form>",
        "body{margin:0}\
         .w{position:relative;width:400px;border:1px solid #000;padding:4px}\
         #i{width:100%;height:100%;border:0;padding:0}",
    );
    let input = by_tag(&dom, "input");
    assert_eq!(
        explorer_layout::hit_test(&bx, 300.0, 8.0),
        Some(input),
        "la boîte input doit couvrir la zone peinte"
    );
}

// ── R-102 : hit-test filtre visibility:hidden/collapse ─────

#[test]
fn hit_test_visibility_hidden_non_ciblable() {
    // Élément caché : pas de cible (§11.2) ; le descendant
    // explicitement `visible` reste ciblable (la valeur
    // calculée porte l'héritage/la surcharge).
    let (dom, bx) = laid(
        "<div id=v style=\"visibility:hidden\">\
         <a id=h href=\"#\">x</a>\
         <a id=k href=\"#\" style=\"visibility:visible\">y</a>\
         </div>",
        "body{margin:0} #v{height:40px} a{display:block;height:15px}",
    );
    let (h, k) = (by_tag(&dom, "a"), {
        let mut found = None;
        let mut q = vec![dom.document()];
        while let Some(n) = q.pop() {
            if dom.attribute(n, "id").ok().flatten().as_deref() == Some("k") {
                found = Some(n);
            }
            if let Ok(ch) = dom.children(n) {
                q.extend(ch.iter().rev());
            }
        }
        found.expect("a#k")
    });
    let _ = h;
    // (a#h) et son parent #v sont cachés → remonte à body/html.
    let hit_hidden = explorer_layout::hit_test(&bx, 5.0, 5.0);
    assert_ne!(hit_hidden, Some(h), "élément hidden non ciblable");
    let div = {
        let mut q = vec![dom.document()];
        let mut f = None;
        while let Some(n) = q.pop() {
            if dom.attribute(n, "id").ok().flatten().as_deref() == Some("v") {
                f = Some(n);
            }
            if let Ok(ch) = dom.children(n) {
                q.extend(ch.iter().rev());
            }
        }
        f.expect("#v")
    };
    assert_ne!(hit_hidden, Some(div), "parent hidden non ciblable");
    // Descendant `visibility:visible` explicite → ciblable —
    // le hit remonte le nœud **texte** (contrat : parent = `a`).
    let hit_k = explorer_layout::hit_test(&bx, 5.0, 20.0);
    assert_eq!(
        hit_k.and_then(|n| dom.parent(n).ok().flatten()),
        Some(k),
        "descendant visible reste atteint sous un parent caché"
    );
}

// ── R-103 : display:contents — enfants promus items flex ───

#[test]
fn display_contents_enfants_items_flex() {
    // `.f{display:flex} > .w{display:contents} > a+a` : les
    // enfants de l'aplatit deviennent des items flex (côte à
    // côte), pas une seule boîte enveloppe.
    fn find_rects(bx: &LayoutBox, tag: NodeId, out: &mut Vec<explorer_layout::Rect>) {
        if bx.node == Some(tag) {
            out.push(bx.rect);
        }
        for c in &bx.children {
            find_rects(c, tag, out);
        }
    }
    let (dom, bx) = laid(
        "<div class=f><div class=w><i>x</i><i>y</i></div></div>",
        "body{margin:0}.f{display:flex}.w{display:contents}i{width:40px;height:20px}",
    );
    // Les deux items côte à côte : même y, x distincts.
    let mut rects = Vec::new();
    for i in {
        let mut q = vec![dom.document()];
        let mut f = Vec::new();
        while let Some(n) = q.pop() {
            if dom.tag_name(n).is_ok_and(|t| t == "i") {
                f.push(n);
            }
            if let Ok(ch) = dom.children(n) {
                q.extend(ch.iter().rev());
            }
        }
        f
    } {
        find_rects(&bx, i, &mut rects);
    }
    assert_eq!(rects.len(), 2, "les deux <i> ont leur boîte");
    assert!(
        (rects[0].y - rects[1].y).abs() < 1.0 && (rects[1].x - rects[0].x).abs() > 10.0,
        "items côte à côte : {rects:?}"
    );
}

#[test]
fn hit_test_lien_example_com() {
    // Motif réel d'example.com : div centrée marge auto +
    // padding, paragraphe contenant <a href>. La sonde du vol
    // rapportait `lien:absent` — la boîte <a> doit être
    // hittable (sinon trou PIPE réel).
    fn find_rect(bx: &LayoutBox, node: NodeId) -> Option<explorer_layout::Rect> {
        if bx.node == Some(node) {
            return Some(bx.rect);
        }
        bx.children.iter().find_map(|c| find_rect(c, node))
    }
    let (dom, bx) = laid(
        "<div><h1>Example Domain</h1>\
         <p>This domain is for use in examples.\
         <a href=\"https://www.iana.org/domains/example\">Learn more</a></p></div>",
        "body{margin:0} div{width:600px;margin:50px auto;padding:50px}",
    );
    let a = by_tag(&dom, "a");
    let Some(r) = find_rect(&bx, a) else {
        panic!("la boîte <a> n'existe pas — trou réel")
    };
    eprintln!("rect <a> = {r:?}");
    assert!(r.width > 0.0 && r.height > 0.0, "rect non vide : {r:?}");
    // Le hit retourne le nœud texte — son parent est `a`.
    let hit = explorer_layout::hit_test(&bx, r.x + 1.0, r.y + r.height / 2.0);
    let el = hit.and_then(|n| {
        if dom.element(n).is_ok() {
            Some(n)
        } else {
            dom.parent(n).ok().flatten()
        }
    });
    assert_eq!(el, Some(a), "hit sur le lien attendu");
}

fn find_input_h(bx: &LayoutBox, node: NodeId) -> f32 {
    fn rec(b: &LayoutBox, node: NodeId) -> Option<f32> {
        if b.node == Some(node) {
            return Some(b.rect.height);
        }
        b.children.iter().find_map(|c| rec(c, node))
    }
    rec(bx, node).unwrap_or(-1.0)
}

#[test]
fn flex_stretch_input_remplit_wrapper() {
    // R-109 — motif FormControl github : conteneur flex à
    // hauteur définie, item `align-self:auto` (= hérite
    // `align-items`, spec §8.3) et `height:auto` →
    // `align-items:stretch` étire la boîte à la ligne. Avant :
    // le `height:1.2em` UA + `mode:"auto"` bloquaient — la
    // zone basse du wrapper était une zone morte au hit-test.
    let (dom, bx) = laid(
        "<div class=f><input></div>",
        ".f{display:flex;width:256px;height:42px} input{width:256px}",
    );
    let input = by_tag(&dom, "input");
    let h = find_input_h(&bx, input);
    assert_eq!(h, 42.0, "item étiré à la ligne : h = {h}");
    // La zone basse du wrapper (y ≈ 35) désigne l'input, pas
    // le wrapper — c'est ce que le clic github n'atteignait
    // pas (form:clic:ignored).
    assert_eq!(
        explorer_layout::hit_test(&bx, 128.0, 35.0).map(|n| {
            if dom.element(n).is_ok() {
                n
            } else {
                dom.parent(n).ok().flatten().unwrap_or(n)
            }
        }),
        Some(input),
        "hit-test dans la zone étendue"
    );
    // Un `min-height` parent ne change rien — % indéterminé
    // reste `auto` (spec) : le test v3 du diag le confirmait.
}

/// Vol 19 (FIX-3) — un bandeau/overlay apparaissant AVANT le
/// contenu dans le DOM mais couvrant la zone volait les hits
/// des liens posés après lui (ordre de document ≠ ordre de
/// peinture : le dernier peint est au-dessus). Le hit-test
/// suit désormais l'ordre d'empilement inversé.
#[test]
fn hit_test_ordre_empilement_dernier_peint_gagne() {
    let (dom, bx) = laid(
        "<div id=o style=\"width:100px;height:100px\"></div>\
         <a id=a href=# style=\"position:absolute;top:0;left:0;display:block;width:60px;height:60px\">x</a>",
        "body{margin:0}",
    );
    let a = by_tag(&dom, "a");
    // (10,10) est dans les deux : le <a> positionné, peint
    // après le div en flux, gagne.
    let hit = explorer_layout::hit_test(&bx, 10.0, 10.0);
    let elem = hit.and_then(|n| {
        let mut cur = n;
        loop {
            match dom.tag_name(cur) {
                Ok(_) => return Some(cur),
                Err(_) => cur = dom.parent(cur).ok().flatten()?,
            }
        }
    });
    assert_eq!(elem, Some(a), "couche supérieure attendue : {hit:?}");
}

/// Même rang d'empilement : le dernier du document est peint
/// au-dessus — il encaisse en premier.
#[test]
fn hit_test_ordre_document_inverse_a_rang_egal() {
    let (dom, bx) = laid(
        "<div id=o style=\"position:absolute;top:0;left:0;width:100px;height:100px\">\
         </div><a id=a href=# style=\"position:absolute;top:0;left:0;display:block;width:50px;height:50px\">x</a>",
        "body{margin:0}",
    );
    let a = by_tag(&dom, "a");
    let hit = explorer_layout::hit_test(&bx, 10.0, 10.0);
    let elem = hit.and_then(|n| {
        let mut cur = n;
        loop {
            match dom.tag_name(cur) {
                Ok(_) => return Some(cur),
                Err(_) => cur = dom.parent(cur).ok().flatten()?,
            }
        }
    });
    assert_eq!(elem, Some(a), "dernier peint attendu : {hit:?}");
}

/// Vol 28 wordle NYT : `<iframe>` sans dimensions = objet
/// remplacé 300×150 (HTML Rendering) — la boîte ne doit
/// pas retomber à 0×0.
#[test]
fn iframe_sans_dimensions_gabarit_300x150() {
    let (dom, bx) = laid(
        "<div><iframe src=\"https://example.com\"></iframe></div>",
        "body{margin:0} div{width:800px}",
    );
    let f = find(&bx, by_tag(&dom, "iframe"));
    assert!(approx(f.rect.width, 300.0), "w={}", f.rect.width);
    assert!(approx(f.rect.height, 150.0), "h={}", f.rect.height);
}

#[test]
fn hit_test_transform_sur_ancetre_atteint_les_enfants() {
    // Non-régression vol 26 : un `transform` sur un ancêtre
    // commun ne doit pas rendre tout le sous-arbre inerte —
    // la matrice cumulative ramène le point sondé dans le
    // repère de chaque boîte.
    let (dom, bx) = laid(
        "<div id=w><a id=c href=#>lien</a><input id=i></div>",
        "body{margin:0} #w{transform:translate(50px,0)}          a{display:block;width:100px;height:20px}          input{display:block;width:100px;height:20px}",
    );
    let a = by_tag(&dom, "a");
    let inp = by_tag(&dom, "input");
    let el = |n: Option<NodeId>| {
        n.and_then(|n| {
            if dom.element(n).is_ok() {
                Some(n)
            } else {
                dom.parent(n).ok().flatten()
            }
        })
    };
    // (55,5) = position peinte du <a> ; (55,25) = input.
    assert_eq!(el(explorer_layout::hit_test(&bx, 55.0, 5.0)), Some(a));
    assert_eq!(el(explorer_layout::hit_test(&bx, 55.0, 25.0)), Some(inp));
    // Variante scrollée (chemin de la session).
    assert_eq!(
        el(explorer_layout::hit_test_scrolled(&bx, 55.0, 5.0, 0.0)),
        Some(a)
    );
}

#[test]
fn overflow_x_seul_ne_clippe_pas_y() {
    // R-120 : `overflow-x:hidden` seul → y calcule `auto`
    // (css-overflow-3 §3), l'étendue verticale reste
    // scrollable (guardian : doc coupé).
    let dom = TreeBuilder::parse("<div><p>bas</p></div>");
    let mut e = StyleEngine::new();
    e.add_css("div{overflow-x:hidden}", Origin::Author);
    let styles = e.compute_tree(&dom, dom.document(), &StyleEnv::default());
    let root_el = dom
        .children(dom.document())
        .into_iter()
        .flatten()
        .find(|&c| dom.element(c).is_ok())
        .unwrap();
    let mut bx = build_box_tree(&dom, root_el, &styles).unwrap();
    layout_tree(&mut bx, VIEW, &BlockCtx::default());
    let d = find_overflow_x(&bx).expect("div overflow-x");
    assert_eq!(d.overflow, explorer_layout::Overflow::Auto);
}

/// Cherche la boîte `overflow-x:hidden` dans l'arbre (R-120).
fn find_overflow_x(bx: &explorer_layout::LayoutBox) -> Option<&explorer_layout::LayoutBox> {
    if bx.overflow_x == explorer_layout::Overflow::Hidden {
        return Some(bx);
    }
    bx.children.iter().find_map(find_overflow_x)
}

#[test]
fn abs_top_bottom_height_auto_setire() {
    // R-121 / B-23 — stretched-link vinted : `<a>`
    // `position:absolute; inset:0` dans un parent `relative`
    // doit couvrir TOUT le CB (§10.6.4), pas hauteur 0.
    let dom = TreeBuilder::parse("<div id=p><p>carte</p><a id=l>lien</a></div>");
    let mut e = StyleEngine::new();
    e.add_css(
        "body{margin:0} #p{position:relative;height:120px} #l{position:absolute;inset:0}",
        Origin::Author,
    );
    let styles = e.compute_tree(&dom, dom.document(), &StyleEnv::default());
    let root_el = dom
        .children(dom.document())
        .into_iter()
        .flatten()
        .find(|&c| dom.element(c).is_ok())
        .unwrap();
    let mut bx = build_box_tree(&dom, root_el, &styles).unwrap();
    layout_tree(&mut bx, VIEW, &BlockCtx::default());
    let link = find(&bx, by_id(&dom, "l"));
    assert!(
        link.rect.height >= 119.0,
        "stretched-link devrait couvrir ~120px, obtenu {}",
        link.rect.height
    );
}

/// Cherche l'élément `id` dans le DOM (miroir du helper de
/// modern.rs — `dom.attribute(n, "id")`).
fn by_id(dom: &Dom, id: &str) -> NodeId {
    let mut queue = vec![dom.document()];
    while let Some(n) = queue.pop() {
        if dom
            .attribute(n, "id")
            .is_ok_and(|v| v.as_deref() == Some(id))
        {
            return n;
        }
        if let Ok(ch) = dom.children(n) {
            queue.extend(ch.iter().rev());
        }
    }
    panic!("#{id} absent")
}

#[test]
fn hit_positionne_gagne_sur_statique() {
    // B-24/qwant (variante §B-13) : un `<input>` positionné
    // doit gagner le hit-test au même point qu'une <img>
    // statique frère — la couche la plus haute frappe d'abord.
    let dom = TreeBuilder::parse("<div id=p><img id=i><input id=s></div>");
    let mut e = StyleEngine::new();
    e.add_css(
        "body{margin:0} #p{position:relative;height:300px}\
         #i{width:400px;height:300px}\
         #s{position:absolute;top:10px;left:10px;width:200px;height:40px}",
        Origin::Author,
    );
    let styles = e.compute_tree(&dom, dom.document(), &StyleEnv::default());
    let root_el = dom
        .children(dom.document())
        .into_iter()
        .flatten()
        .find(|&c| dom.element(c).is_ok())
        .unwrap();
    let mut bx = build_box_tree(&dom, root_el, &styles).unwrap();
    layout_tree(&mut bx, VIEW, &BlockCtx::default());
    // Point dans l'input (20,20) — img le couvre aussi.
    let hit = explorer_layout::hit_test(&bx, 20.0, 20.0);
    assert_eq!(
        hit.and_then(|n| dom.attribute(n, "id").ok().flatten()),
        Some("s".to_string()),
        "l'input positionné doit gagner sur l'img statique"
    );
}

#[test]
fn aspect_ratio_auto_height() {
    // R-123 : `aspect-ratio` dimensionne la hauteur auto —
    // `aspect-video` (16/9) Tailwind.
    let dom = TreeBuilder::parse("<div id=v></div>");
    let mut e = StyleEngine::new();
    e.add_css(
        "body{margin:0} #v{width:320px;aspect-ratio:16/9}",
        Origin::Author,
    );
    let styles = e.compute_tree(&dom, dom.document(), &StyleEnv::default());
    let root_el = dom
        .children(dom.document())
        .into_iter()
        .flatten()
        .find(|&c| dom.element(c).is_ok())
        .unwrap();
    let mut bx = build_box_tree(&dom, root_el, &styles).unwrap();
    layout_tree(&mut bx, VIEW, &BlockCtx::default());
    let v = find(&bx, by_id(&dom, "v"));
    // 320 / (16/9) = 180 — hauteur dérivée du ratio.
    assert!(
        (v.rect.height - 180.0).abs() < 1.0,
        "aspect-ratio 16/9 sur 320px → ~180, obtenu {}",
        v.rect.height
    );
}

#[test]
fn overflow_wrap_break_word_coupe() {
    // R-126 : `overflow-wrap:break-word` (Tailwind
    // `break-words`) — un mot plus large que la ligne se
    // coupe au lieu de déborder.
    let dom = TreeBuilder::parse("<div id=d>treslongmotquidebordepartout</div>");
    let mut e = StyleEngine::new();
    e.add_css(
        "body{margin:0} #d{width:60px;overflow-wrap:break-word;font-size:10px}",
        Origin::Author,
    );
    let styles = e.compute_tree(&dom, dom.document(), &StyleEnv::default());
    let root_el = dom
        .children(dom.document())
        .into_iter()
        .flatten()
        .find(|&c| dom.element(c).is_ok())
        .unwrap();
    let mut bx = build_box_tree(&dom, root_el, &styles).unwrap();
    layout_tree(&mut bx, VIEW, &BlockCtx::default());
    let d = find(&bx, by_id(&dom, "d"));
    // Le mot de ~30 chars doit produire ≥2 lignes (coupé).
    let total: usize = d.lines.len() + d.children.iter().map(|c| c.lines.len()).sum::<usize>();
    assert!(total >= 2, "mot long non coupé — {total} ligne(s)");
}

#[test]
fn height_pct_chain_html_body_map() {
    // B-28 (uMap/Leaflet vol50) : `html`/`body`/`#map` à
    // `height:100%` — la chaîne doit donner la hauteur
    // du viewport à chaque étape, pas 0.
    let dom = TreeBuilder::parse("<body><div id=map></div></body>");
    let mut e = StyleEngine::new();
    e.add_css(
        "html,body{height:100%;margin:0} #map{height:100%}",
        Origin::Author,
    );
    let styles = e.compute_tree(&dom, dom.document(), &StyleEnv::default());
    let root_el = dom
        .children(dom.document())
        .into_iter()
        .flatten()
        .find(|&c| dom.element(c).is_ok())
        .unwrap();
    let mut bx = build_box_tree(&dom, root_el, &styles).unwrap();
    layout_tree(&mut bx, VIEW, &BlockCtx::default());
    let map = find(&bx, by_id(&dom, "map"));
    assert!(
        (map.rect.height - 600.0).abs() < 1.0,
        "#map height:100% → ~600px viewport, obtenu {}",
        map.rect.height
    );
}

#[test]
fn height_pct_chain_absolute_map() {
    // B-28 variant : mount Leaflet `#map{position:absolute;
    // top:0;bottom:0}` dans `body{position:relative}` —
    // la chaîne % doit donner le viewport au stretch abspos.
    let dom = TreeBuilder::parse("<body><div id=map></div></body>");
    let mut e = StyleEngine::new();
    e.add_css(
        "html,body{height:100%;margin:0} body{position:relative}\
         #map{position:absolute;top:0;left:0;right:0;bottom:0}",
        Origin::Author,
    );
    let styles = e.compute_tree(&dom, dom.document(), &StyleEnv::default());
    let root_el = dom
        .children(dom.document())
        .into_iter()
        .flatten()
        .find(|&c| dom.element(c).is_ok())
        .unwrap();
    let mut bx = build_box_tree(&dom, root_el, &styles).unwrap();
    layout_tree(&mut bx, VIEW, &BlockCtx::default());
    let map = find(&bx, by_id(&dom, "map"));
    assert!(
        (map.rect.height - 600.0).abs() < 1.0,
        "#map abspos stretch → ~600px, obtenu {}",
        map.rect.height
    );
}

#[test]
fn remplace_max_height_pct_bridant_la_hauteur() {
    // B-34 (cjoint vol56) : `<img>` intrinsèque 703×994 dans
    // `div{height:192px}` avec `max-height:100%` → ≤192 px.
    // Le cb_h des atomes inline = h_spec du conteneur (avant
    // le fix : cb_h brut du grand-parent → souvent None →
    // pas de clamp → 994 px débordaient).
    let html = "<!doctype html><div style=\"height:192px\"><img style=\"max-height:100%\"></div>";
    let dom_probe = TreeBuilder::parse(html);
    let sizes = ratio_sizes(&dom_probe, "img", 703.0, 994.0);
    let (dom, bx) = {
        let (dom, bx) = laid_sized(html, "", &sizes);
        (dom, bx)
    };
    let img = find(&bx, by_tag(&dom, "img"));
    assert!(
        img.rect.height <= 192.5,
        "img max-height:100% → ≤192, obtenu {}",
        img.rect.height
    );
}

#[test]
fn hit_sticky_epingle_gagne_au_scroll() {
    // B-40 (trainline/culture.gouv vols 60/62) : un header
    // `position:sticky;top:0` épinglé au viewport est peint
    // décalé par `sticky_offset` — le hit-test doit frapper
    // sa position **visuelle**, pas son doc-y naturel.
    // Sans le fix : le clic sur le header épinglé retombait
    // sur la couche dessous (ancre pleine-largeur → nav
    // erronée) ou `ignored`.
    let dom = TreeBuilder::parse(
        "<body><div id=h><a id=l>nav</a></div>\
         <div id=below style=\"height:2000px\">x</div></body>",
    );
    let mut e = StyleEngine::new();
    e.add_css(
        "body{margin:0} #h{position:sticky;top:0;height:50px}\
         #l{display:block;height:50px}",
        Origin::Author,
    );
    let styles = e.compute_tree(&dom, dom.document(), &StyleEnv::default());
    let root_el = dom
        .children(dom.document())
        .into_iter()
        .flatten()
        .find(|&c| dom.element(c).is_ok())
        .unwrap();
    let mut bx = build_box_tree(&dom, root_el, &styles).unwrap();
    layout_tree(&mut bx, VIEW, &BlockCtx::default());
    // Clic à viewport-y 25 avec scroll=1000 : le header est
    // épinglé en haut — le hit doit tomber sur lui/son lien,
    // PAS sur `below` (doc-y 1025).
    let hit = explorer_layout::hit_test_scrolled_vp(&bx, 10.0, 25.0, 1000.0, (VIEW, 600.0));
    let id = hit
        .and_then(|n| dom.attribute(n, "id").ok().flatten())
        .unwrap_or_default();
    assert!(
        id == "h" || id == "l",
        "clic sur header sticky épinglé → h/l attendu, obtenu {id}"
    );
}
