// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Tests du layout moderne (étape 42) : `calc()`, flexbox,
//! grid — mesures sur l'arbre de boîtes posé.

mod common;

use explorer_css::{Origin, StyleEngine, StyleEnv};
use explorer_dom::{Dom, NodeId};
use explorer_html::TreeBuilder;
use explorer_layout::{build_box_tree, layout_tree, BlockCtx, LayoutBox};

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

fn find_opt(b: &LayoutBox, id: NodeId) -> Option<&LayoutBox> {
    if b.node == Some(id) {
        return Some(b);
    }
    b.children.iter().find_map(|c| find_opt(c, id))
}

fn find(b: &LayoutBox, id: NodeId) -> &LayoutBox {
    find_opt(b, id).expect("boîte du nœud")
}

fn approx(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.51
}

// ── calc() ────────────────────────────────────────────────────

#[test]
fn calc_pourcent_moins_px() {
    let (dom, bx) = laid(
        "<!doctype html><div>x</div>",
        "body { margin: 0 } div { width: calc(100% - 20px) }",
    );
    let d = find(&bx, by_tag(&dom, "div"));
    assert!(approx(d.rect.width, VIEW - 20.0), "{}", d.rect.width);
}

#[test]
fn calc_multiplication_et_imbrication() {
    let (dom, bx) = laid(
        "<!doctype html><div>x</div>",
        "body { margin: 0 } div { width: calc(calc(100px * 2) + 50px / 2) }",
    );
    let d = find(&bx, by_tag(&dom, "div"));
    assert!(approx(d.rect.width, 225.0), "{}", d.rect.width);
}

#[test]
fn calc_invalide_degrade() {
    let (dom, bx) = laid(
        "<!doctype html><div style=\"height: 10px\">x</div>",
        "body { margin: 0 } div { width: calc(1px + 2); height: 10px }",
    );
    let d = find(&bx, by_tag(&dom, "div"));
    // `1px + 2` invalide (dim + nombre nu) → width ignorée →
    // le bloc remplit le conteneur.
    assert!(approx(d.rect.width, VIEW), "{}", d.rect.width);
}

#[test]
fn calc_dans_var() {
    let (dom, bx) = laid(
        "<!doctype html><div>x</div>",
        "body { margin: 0; --w: calc(25% + 10px) } div { width: var(--w) }",
    );
    let d = find(&bx, by_tag(&dom, "div"));
    assert!(approx(d.rect.width, 210.0), "{}", d.rect.width);
}

// ── flex ──────────────────────────────────────────────────────

#[test]
fn flex_grow_partage_l_espace() {
    let (dom, bx) = laid(
        "<!doctype html><section><div id=\"a\">A</div><div id=\"b\">B</div></section>",
        "body { margin: 0 } section { display: flex } section > div { flex: 1 }",
    );
    let section = find(&bx, by_tag(&dom, "section"));
    assert_eq!(section.children.len(), 2);
    let (ia, ib) = (&section.children[0], &section.children[1]);
    assert!(approx(ia.rect.width, VIEW / 2.0), "{}", ia.rect.width);
    assert!(approx(ib.rect.width, VIEW / 2.0));
    assert!(approx(ia.rect.x, 0.0));
    assert!(approx(ib.rect.x, VIEW / 2.0), "{}", ib.rect.x);
}

#[test]
fn flex_justify_center_et_gap() {
    let (dom, bx) = laid(
        "<!doctype html><section><div>A</div><div>B</div></section>",
        "body { margin: 0 } section { display: flex; justify-content: center; gap: 40px } \
         section > div { width: 100px; height: 10px }",
    );
    let section = find(&bx, by_tag(&dom, "section"));
    let (a, b) = (&section.children[0], &section.children[1]);
    // 240 px d'items → lead = (800 - 240) / 2 = 280.
    assert!(approx(a.rect.x, 280.0), "{}", a.rect.x);
    assert!(approx(b.rect.x, 280.0 + 100.0 + 40.0), "{}", b.rect.x);
}

#[test]
fn flex_direction_column() {
    let (dom, bx) = laid(
        "<!doctype html><section><div>A</div><div>B</div></section>",
        "body { margin: 0 } section { display: flex; flex-direction: column; gap: 10px } \
         section > div { height: 20px }",
    );
    let section = find(&bx, by_tag(&dom, "section"));
    let (a, b) = (&section.children[0], &section.children[1]);
    assert!(approx(a.rect.height, 20.0));
    assert!(approx(b.rect.y - a.rect.y, 30.0), "{}", b.rect.y - a.rect.y);
}

#[test]
fn flex_row_reverse_ordre_et_ancrage() {
    // R-27 : `row-reverse` pose les items dans l'ordre
    // logique inversé, le groupe packé vers main-start
    // (bord droit) — réf. WPT `flex-direction-row-reverse`.
    let (dom, bx) = laid(
        "<!doctype html><section><div id=\"a\">A</div><div id=\"b\">B</div></section>",
        "body { margin: 0 } section { display: flex; flex-direction: row-reverse } \
         section > div { width: 100px; height: 10px }",
    );
    let section = find(&bx, by_tag(&dom, "section"));
    let (a, b) = (&section.children[0], &section.children[1]);
    // Premier fils source à DROITE : B à 600, A à 700.
    assert!(approx(b.rect.x, 600.0), "b.x={}", b.rect.x);
    assert!(approx(a.rect.x, 700.0), "a.x={}", a.rect.x);
}

#[test]
fn flex_align_center() {
    let (dom, bx) = laid(
        "<!doctype html><section><div>A</div><div id=\"t\">B</div></section>",
        "body { margin: 0 } section { display: flex; align-items: center } \
         section > div { height: 10px; width: 50px } #t { height: 30px }",
    );
    let section = find(&bx, by_tag(&dom, "section"));
    let t = section
        .children
        .iter()
        .find(|c| approx(c.rect.height, 30.0));
    let t = t.expect("item 30px");
    // Ligne cross = 30, item 10px centré → dy = 10.
    let small = section
        .children
        .iter()
        .find(|c| approx(c.rect.height, 10.0))
        .expect("item 10px");
    assert!(approx(small.rect.y - t.rect.y, 10.0), "{}", small.rect.y);
}

#[test]
fn flex_justify_right_end_alias() {
    // R-28 : `right`/`end` ≡ `flex-end` sur un conteneur
    // row LTR ; `left` ≡ `flex-start` (css-align-3 §4.1).
    for (val, want_x) in [
        ("left", 0.0),
        ("right", VIEW - 100.0),
        ("end", VIEW - 100.0),
    ] {
        let (dom, bx) = laid(
            "<!doctype html><section><div>A</div></section>",
            &format!(
                "body {{ margin: 0 }} section {{ display: flex; justify-content: {val} }} \
                 section > div {{ width: 100px; height: 10px }}"
            ),
        );
        let section = find(&bx, by_tag(&dom, "section"));
        assert!(
            approx(section.children[0].rect.x, want_x),
            "{val}: x={}",
            section.children[0].rect.x
        );
    }
}

#[test]
fn flex_justify_overflow_lead_negatif() {
    // R-28 : `flex-end` sur un conteneur plus étroit que les
    // items → lead négatif (débord vers main-start), §9.5.
    let (dom, bx) = laid(
        "<!doctype html><section><div>A</div></section>",
        "body { margin: 0 } section { display: flex; justify-content: flex-end; width: 30px } \
         section > div { flex: 0 0 45px; height: 10px }",
    );
    let section = find(&bx, by_tag(&dom, "section"));
    assert!(
        approx(section.children[0].rect.x, -15.0),
        "{}",
        section.children[0].rect.x
    );
}

#[test]
fn flex_align_hauteur_conteneur() {
    // R-28 / §9.4 : conteneur mono-ligne à hauteur définie →
    // la ligne cross = taille cross interne du conteneur,
    // pas la hauteur max des items.
    let (dom, bx) = laid(
        "<!doctype html><section><div>A</div></section>",
        "body { margin: 0 } section { display: flex; align-items: center; height: 100px } \
         section > div { width: 50px; height: 10px }",
    );
    let section = find(&bx, by_tag(&dom, "section"));
    assert!(
        approx(section.children[0].rect.y, 45.0),
        "{}",
        section.children[0].rect.y
    );
}

#[test]
fn flex_basis_content_box() {
    // R-28 : `flex-basis`/`width` = content-box — padding et
    // bordures s'ajoutent à la taille externe de l'item.
    let (dom, bx) = laid(
        "<!doctype html><section><div>A</div></section>",
        "body { margin: 0 } section { display: flex } \
         section > div { flex: 0 0 35px; padding: 2px; border: 4px solid; height: 10px }",
    );
    let section = find(&bx, by_tag(&dom, "section"));
    // border-box = 35 + 4 + 8 = 47.
    assert!(
        approx(section.children[0].rect.width, 47.0),
        "{}",
        section.children[0].rect.width
    );
}

#[test]
fn flex_shrink_cible_appliquee() {
    // R-28 : la cible shrink calculée remplace le `width`
    // déclaré quand l'item est posé (§9.7).
    let (dom, bx) = laid(
        "<!doctype html><section><div>A</div><div>B</div></section>",
        "body { margin: 0 } section { display: flex; width: 100px } \
         section > div { width: 80px; flex-shrink: 1; height: 10px }",
    );
    let section = find(&bx, by_tag(&dom, "section"));
    // 160 px d'items dans 100 px → chacun rétrécit de 30 → 50.
    assert!(
        approx(section.children[0].rect.width, 50.0),
        "{}",
        section.children[0].rect.width
    );
    assert!(approx(section.children[1].rect.width, 50.0));
}

#[test]
fn flex_row_wrap_gap() {
    // R-29 : `row wrap` — découpe en lignes sur la largeur,
    // `column-gap` entre items, `row-gap` entre lignes.
    let (dom, bx) = laid(
        "<!doctype html><section><div>A</div><div>B</div><div>C</div></section>",
        "body { margin: 0 } \
         section { display: flex; flex-flow: row wrap; width: 100px; \
                   column-gap: 10px; row-gap: 5px } \
         section > div { width: 40px; height: 10px }",
    );
    let section = find(&bx, by_tag(&dom, "section"));
    // Ligne 1 : 40+10+40 = 90 ≤ 100 ; C déborderait (140).
    let (a, b, c) = (
        &section.children[0],
        &section.children[1],
        &section.children[2],
    );
    assert!(approx(b.rect.x, 50.0), "b.x={}", b.rect.x);
    assert!(approx(c.rect.x, 0.0), "c.x={}", c.rect.x);
    assert!(approx(c.rect.y, 15.0), "c.y={}", c.rect.y);
    let _ = a;
}

#[test]
fn flex_column_wrap_gap_align_content() {
    // R-29 : `column wrap` — découpe en colonnes sur la
    // hauteur définie ; `justify-content` distribue le libre
    // main dans chaque colonne, `align-content` répartit les
    // colonnes sur le cross, la marge `auto` main absorbe le
    // libre avant justify. Géométrie de la fixture WPT
    // `flexbox-column-row-gap-001`.
    let (dom, bx) = laid(
        "<!doctype html><section>\
         <div>A</div><div>B</div><div>C</div><div>D</div><div>E</div><div id=\"f\">F</div></section>",
        "body { margin: 0 } \
         section { display: flex; flex-flow: column wrap; width: 200px; height: 220px; \
                   column-gap: 20px; row-gap: 40px; justify-content: space-around; \
                   align-content: space-around } \
         section > div { width: 28px; height: 28px; border: 1px solid } \
         #f { margin-bottom: auto }",
    );
    let section = find(&bx, by_tag(&dom, "section"));
    let (a, d) = (&section.children[0], &section.children[3]);
    // 3 items par colonne (170 px) ; le 4e déborderait.
    // Cross : 30+20+30 = 80 → libre 120 → space-around
    // lead 30, entre 60 — ajouté AU column-gap → colonnes
    // à x=30 et x=140 (espacement 20+60 = 80).
    assert!(approx(a.rect.x, 30.0), "col1.x={}", a.rect.x);
    assert!(approx(d.rect.x, 140.0), "col2.x={}", d.rect.x);
    // Col1 : libre main 50, justify space-around →
    // lead 50/6 ≈ 8.33, entre 50/3 ≈ 16.67.
    let b = &section.children[1];
    assert!(approx(a.rect.y, 25.0 / 3.0), "a.y={}", a.rect.y);
    assert!(
        approx(b.rect.y, 25.0 / 3.0 + 30.0 + 40.0 + 50.0 / 3.0),
        "b.y={}",
        b.rect.y
    );
    // Col2 : `margin-bottom:auto` de F absorbe les 50 px de
    // libre → empilement serré depuis le haut (row-gap seul).
    assert!(approx(d.rect.y, 0.0), "d.y={}", d.rect.y);
    let f = find(&bx, by_id(&dom, "f"));
    assert!(approx(f.rect.y, 140.0), "f.y={}", f.rect.y);
}

#[test]
fn inline_block_vertical_align_top() {
    // R-28 : `vertical-align: top` ancre la margin box de
    // l'atome au sommet de la ligne (§10.8), pas baseline.
    let (dom, bx) = laid(
        "<!doctype html><div id=\"l\"><span id=\"s\"></span><span id=\"t\"></span></div>",
        "body { margin: 0 } #l { line-height: 0; font-size: 0 } \
         #l > span { display: inline-block; vertical-align: top } \
         #s { width: 20px; height: 18px } #t { width: 20px; height: 22px }",
    );
    let s = find(&bx, by_id(&dom, "s"));
    let t = find(&bx, by_id(&dom, "t"));
    assert!(
        approx(s.rect.y, t.rect.y),
        "s.y={} t.y={}",
        s.rect.y,
        t.rect.y
    );
}

// ── grid ──────────────────────────────────────────────────────

#[test]
fn grid_template_columns_fr() {
    let (dom, bx) = laid(
        "<!doctype html><section><div>A</div><div>B</div></section>",
        "body { margin: 0 } section { display: grid; grid-template-columns: 1fr 1fr } \
         section > div { height: 10px }",
    );
    let section = find(&bx, by_tag(&dom, "section"));
    let (a, b) = (&section.children[0], &section.children[1]);
    assert!(approx(a.rect.width, VIEW / 2.0), "{}", a.rect.width);
    assert!(approx(b.rect.x, VIEW / 2.0), "{}", b.rect.x);
}

#[test]
fn grid_fixed_et_gap_et_span() {
    let (dom, bx) = laid(
        "<!doctype html><section><div id=\"w\">W</div><div>N</div></section>",
        "body { margin: 0 } section { display: grid; grid-template-columns: 200px 1fr; \
         column-gap: 20px } #w { grid-column: 1 / span 2; height: 10px } \
         section > div { height: 10px }",
    );
    let section = find(&bx, by_tag(&dom, "section"));
    let (w, n) = (&section.children[0], &section.children[1]);
    assert!(approx(w.rect.width, VIEW), "{}", w.rect.width);
    // Deuxième item sur la ligne suivante, colonne 1.
    assert!(approx(n.rect.x, 0.0), "{}", n.rect.x);
    assert!(n.rect.y > w.rect.y, "{} vs {}", n.rect.y, w.rect.y);
}

#[test]
fn grid_repeat() {
    let (dom, bx) = laid(
        "<!doctype html><section><div>A</div><div>B</div><div>C</div></section>",
        "body { margin: 0 } section { display: grid; \
         grid-template-columns: repeat(3, 100px) } section > div { height: 10px }",
    );
    let section = find(&bx, by_tag(&dom, "section"));
    let c = &section.children[2];
    assert!(approx(c.rect.x, 200.0), "{}", c.rect.x);
    assert!(approx(c.rect.width, 100.0), "{}", c.rect.width);
}

// ── R-5 : grid MVP+ ──────────────────────────────────────────

#[test]
fn grid_template_areas_place_par_nom() {
    // `grid-template-areas` + `grid-area: nav|main` : pose par
    // bounding box de la zone nommée.
    let (dom, bx) = laid(
        "<!doctype html><section><div id=\"m\">M</div><div id=\"n\">N</div></section>",
        "body { margin: 0 } section { display: grid; \
         grid-template-columns: 200px 1fr; \
         grid-template-areas: \"nav main\" } \
         #n { grid-area: nav; height: 10px } #m { grid-area: main; height: 10px }",
    );
    let section = find(&bx, by_tag(&dom, "section"));
    // #m déclaré en premier dans le DOM mais posé en col 2.
    let (m, n) = (&section.children[0], &section.children[1]);
    assert!(
        approx(n.rect.x, 0.0) && approx(n.rect.width, 200.0),
        "nav {:?}",
        n.rect
    );
    assert!(
        approx(m.rect.x, 200.0) && approx(m.rect.width, 600.0),
        "main {:?}",
        m.rect
    );
}

#[test]
fn grid_area_span_multi_cellules() {
    // Area 2×2 : l'item occupe la bbox complète.
    let (dom, bx) = laid(
        "<!doctype html><section><div id=\"h\">H</div><div>x</div><div>y</div></section>",
        "body { margin: 0 } section { display: grid; \
         grid-template-columns: 100px 1fr; grid-template-rows: 20px 20px; \
         grid-template-areas: \"head head\" \"l r\" } \
         #h { grid-area: head; height: 10px }",
    );
    let section = find(&bx, by_tag(&dom, "section"));
    let h = &section.children[0];
    assert!(approx(h.rect.width, 800.0), "{}", h.rect.width);
    assert!(approx(h.rect.x, 0.0) && approx(h.rect.y, 0.0));
}

#[test]
fn grid_lignes_nommees() {
    // `[mid]` dans le template nomme la bordure —
    // `grid-column: mid / end` place entre les lignes nommées.
    let (dom, bx) = laid(
        "<!doctype html><section><div>X</div></section>",
        "body { margin: 0 } section { display: grid; \
         grid-template-columns: [start] 100px [mid] 200px [end] } \
         section > div { grid-column: mid / end; height: 10px }",
    );
    let d = find(&bx, by_tag(&dom, "section")).children[0].clone();
    assert!(approx(d.rect.x, 100.0), "x={}", d.rect.x);
    assert!(approx(d.rect.width, 200.0), "w={}", d.rect.width);
}

#[test]
fn grid_auto_flow_column() {
    // `column` : remplit les lignes d'une colonne avant de
    // passer à la suivante (ici 2 lignes → items 0,1 en col 0).
    let (dom, bx) = laid(
        "<!doctype html><section><div>a</div><div>b</div><div>c</div></section>",
        "body { margin: 0 } section { display: grid; grid-auto-flow: column; \
         grid-template-rows: 10px 10px; grid-template-columns: 300px 1fr } \
         section > div { height: 10px }",
    );
    let section = find(&bx, by_tag(&dom, "section"));
    let (a, b, c) = (
        &section.children[0],
        &section.children[1],
        &section.children[2],
    );
    assert!(approx(a.rect.x, 0.0) && approx(a.rect.y, 0.0));
    assert!(
        approx(b.rect.x, 0.0) && approx(b.rect.y, 10.0),
        "b {:?}",
        b.rect
    );
    assert!(
        approx(c.rect.x, 300.0) && approx(c.rect.y, 0.0),
        "c {:?}",
        c.rect
    );
}

#[test]
fn grid_auto_flow_dense_rebouche() {
    // `dense` : l'item auto rescanne depuis (0,0) et rebouche
    // le trou laissé par l'item explicitement placé en col 2.
    let (dom, bx) = laid(
        "<!doctype html><section>\
           <div id=\"p\">P</div><div>A</div><div>B</div></section>",
        "body { margin: 0 } section { display: grid; grid-auto-flow: dense; \
         grid-template-columns: 100px 100px } #p { grid-column: 2; height: 10px } \
         section > div { height: 10px }",
    );
    let section = find(&bx, by_tag(&dom, "section"));
    let a = &section.children[1];
    assert!(
        approx(a.rect.x, 0.0) && approx(a.rect.y, 0.0),
        "dense {:?}",
        a.rect
    );
    // Sans dense : le trou (0,0) reste vide, A part en (1,0).
    let (dom2, bx2) = laid(
        "<!doctype html><section>\
           <div id=\"p\">P</div><div>A</div></section>",
        "body { margin: 0 } section { display: grid; \
         grid-template-columns: 100px 100px } #p { grid-column: 2; height: 10px } \
         section > div { height: 10px }",
    );
    let a2 = &find(&bx2, by_tag(&dom2, "section")).children[1];
    assert!(a2.rect.y > 0.0, "sparse laisse le trou : {:?}", a2.rect);
}

#[test]
fn grid_minmax_fr_borne() {
    // `minmax(400px, 1fr)` : la part fr est bornée par le min
    // quand le libre est insuffisant.
    let (dom, bx) = laid(
        "<!doctype html><section><div>a</div><div>b</div></section>",
        "body { margin: 0 } section { display: grid; \
         grid-template-columns: minmax(400px, 1fr) 1fr } \
         section > div { height: 10px }",
    );
    let section = find(&bx, by_tag(&dom, "section"));
    let (a, b) = (&section.children[0], &section.children[1]);
    // free = 800-400 = 400 → part fr = 200 < min 400 → col0=400.
    assert!(approx(a.rect.width, 400.0), "{}", a.rect.width);
    assert!(approx(b.rect.width, 200.0), "{}", b.rect.width);
}

// ── R-30 : fr vertical grid + résiduels ───────────────────────

#[test]
fn grid_fr_vertical_distribue() {
    // `grid-template-rows: 30px 1fr` sur conteneur à hauteur
    // définie : le fr prend le libre (70), pas le contenu —
    // les pistes flexibles sont exclues du calcul de l'espace
    // (§12.7).
    let (dom, bx) = laid(
        "<!doctype html><section><div id=\"a\">a</div><div id=\"b\">b</div></section>",
        "body { margin: 0 } section { display: grid; height: 100px; \
         grid-template-rows: 30px 1fr } #a { grid-row: 1 } #b { grid-row: 2 }",
    );
    let section = find(&bx, by_tag(&dom, "section"));
    let b = &section.children[1];
    assert!(approx(b.rect.y, section.rect.y + 30.0), "{}", b.rect.y);
    // `stretch` (défaut) : l'item remplit la cellule de 70.
    assert!(approx(b.rect.height, 70.0), "{}", b.rect.height);
}

#[test]
fn grid_item_height_pct_resout_sur_cellule() {
    // `height:100%` d'un item se résout contre son aire de
    // grille (§6.5), pas contre le conteneur englobant.
    let (dom, bx) = laid(
        "<!doctype html><section><div id=\"b\">b</div></section>",
        "body { margin: 0 } section { display: grid; height: 100px; \
         grid-template-rows: 30px 1fr } #b { grid-row: 2; height: 100% }",
    );
    let b = find(&bx, by_id(&dom, "b"));
    assert!(approx(b.rect.height, 70.0), "{}", b.rect.height);
}

#[test]
fn abspos_position_statique_apres_marge() {
    // Placeholder absolu : la marge en attente du frère
    // précédent est consommée — la position statique inclut
    // la marge collapsée (§10.6.4).
    let (dom, bx) = laid(
        "<!doctype html><p>x</p><div id=\"a\"></div>",
        "body { margin: 0 } p { margin-bottom: 16px; height: 10px } \
         #a { position: absolute; width: 20px; height: 20px }",
    );
    let a = find(&bx, by_id(&dom, "a"));
    // border-top = bas du <p> (10) + marge 16 = 26.
    assert!(approx(a.rect.y, 26.0), "{}", a.rect.y);
}

#[test]
fn table_width_cellule_cloue_colonne() {
    // `width:100px` sur un td cloue sa colonne ; le reste de
    // la table va aux colonnes auto (§17.5.2.2) — avant, la
    // colonne spécifiée prenait une part proportionnelle.
    let (dom, bx) = laid(
        "<!doctype html><table><tr><td id=\"a\">a</td><td id=\"b\">b</td></tr></table>",
        "body { margin: 0 } table { width: 400px } #a { width: 100px }",
    );
    let a = find(&bx, by_id(&dom, "a"));
    let b = find(&bx, by_id(&dom, "b"));
    assert!(approx(a.rect.width, 100.0), "{}", a.rect.width);
    assert!(approx(b.rect.x, 100.0), "{}", b.rect.x);
    assert!(approx(b.rect.width, 300.0), "{}", b.rect.width);
}

#[test]
fn table_height_excedent_distribue() {
    // `height` de la table > contenu : l'excédent va aux rangs
    // sans `height` de cellule spécifiée (§17.5.3).
    let (dom, bx) = laid(
        "<!doctype html><table><tr><td>a</td></tr><tr><td id=\"b\">b</td></tr></table>",
        "body { margin: 0 } table { width: 400px; height: 100px } \
         tr:first-child td { height: 30px }",
    );
    let b = find(&bx, by_id(&dom, "b"));
    // rang2 = 100 - 30 = 70 (rang1 fixé à 30).
    assert!(approx(b.rect.height, 70.0), "{}", b.rect.height);
}
