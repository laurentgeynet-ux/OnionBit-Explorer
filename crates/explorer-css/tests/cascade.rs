// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Tests de la cascade : origines, spécificité, `!important`,
//! héritage, mots-clés, media queries, feuille UA.

use explorer_css::{ComputedStyle, Origin, StyleEngine, StyleEnv};
use explorer_dom::{DocumentMode, Dom, NodeId};
use explorer_html::TreeBuilder;

/// DOM minimal : `body > div#a > p.b "texte"`.
struct Fixture {
    dom: Dom,
    #[allow(dead_code)]
    body: NodeId,
    div: NodeId,
    p: NodeId,
}

fn fixture() -> Fixture {
    let mut dom = Dom::new();
    let doc = dom.document();
    let body = dom.create_element("body").unwrap();
    dom.append_child(doc, body).unwrap();
    let div = dom.create_element("div").unwrap();
    dom.set_attribute(div, "id", "a").unwrap();
    dom.append_child(body, div).unwrap();
    let p = dom.create_element("p").unwrap();
    dom.set_attribute(p, "class", "b").unwrap();
    dom.append_child(div, p).unwrap();
    let t = dom.create_text("texte").unwrap();
    dom.append_child(p, t).unwrap();
    Fixture { dom, body, div, p }
}

fn env() -> StyleEnv {
    StyleEnv::default()
}

// ── origines et ordre ─────────────────────────────────────────

#[test]
fn feuille_ua_par_defaut() {
    let f = fixture();
    let e = StyleEngine::new();
    let s = e.compute(&f.dom, f.div, &env());
    // div est `display: block` via la feuille UA.
    assert_eq!(s.get("display"), Some("block"));
    // p aussi.
    assert_eq!(e.compute(&f.dom, f.p, &env()).get("display"), Some("block"));
}

#[test]
fn auteur_bat_ua() {
    let f = fixture();
    let mut e = StyleEngine::new();
    e.add_css("div { display: inline }", Origin::Author);
    assert_eq!(
        e.compute(&f.dom, f.div, &env()).get("display"),
        Some("inline")
    );
}

#[test]
fn specificite_bat_ordre() {
    let f = fixture();
    let mut e = StyleEngine::new();
    // La règle la moins spécifique arrive APRÈS — doit perdre.
    e.add_css(
        "#a { display: none } div { display: block }",
        Origin::Author,
    );
    assert_eq!(
        e.compute(&f.dom, f.div, &env()).get("display"),
        Some("none")
    );
}

#[test]
fn ordre_departage_a_specificite_egale() {
    let f = fixture();
    let mut e = StyleEngine::new();
    e.add_css("#a { display: block } #a { display: none }", Origin::Author);
    assert_eq!(
        e.compute(&f.dom, f.div, &env()).get("display"),
        Some("none")
    );
}

#[test]
fn important_bat_normal() {
    let f = fixture();
    let mut e = StyleEngine::new();
    e.add_css(
        "#a { display: none } div { display: block !important }",
        Origin::Author,
    );
    assert_eq!(
        e.compute(&f.dom, f.div, &env()).get("display"),
        Some("block")
    );
}

#[test]
fn important_ua_bat_auteur() {
    let f = fixture();
    let mut e = StyleEngine::new();
    e.add_css("div { display: table !important }", Origin::UserAgent);
    e.add_css("#a { display: none !important }", Origin::Author);
    assert_eq!(
        e.compute(&f.dom, f.div, &env()).get("display"),
        Some("table")
    );
}

// ── héritage et mots-clés ─────────────────────────────────────

#[test]
fn heritage_propriete_heritee() {
    let f = fixture();
    let mut e = StyleEngine::new();
    e.add_css("div { color: red }", Origin::Author);
    // p n'a pas de règle → hérite de color (héritée).
    assert_eq!(e.compute(&f.dom, f.p, &env()).get("color"), Some("red"));
}

#[test]
fn pas_d_heritage_propriete_non_heritee() {
    let f = fixture();
    let mut e = StyleEngine::new();
    e.add_css("div { margin-left: 9px }", Origin::Author);
    // margin-left n'hérite pas et la feuille UA ne le fixe
    // pas sur p → absent.
    assert_eq!(e.compute(&f.dom, f.p, &env()).get("margin-left"), None);
}

#[test]
fn heritage_explicite_mot_cle() {
    let f = fixture();
    let mut e = StyleEngine::new();
    e.add_css(
        "div { margin-top: 9px } p.b { margin-top: inherit }",
        Origin::Author,
    );
    // `inherit` force même une propriété non héritée.
    assert_eq!(
        e.compute(&f.dom, f.p, &env()).get("margin-top"),
        Some("9px")
    );
}

#[test]
fn unset_herite_si_heritee_sinon_initiale() {
    let f = fixture();
    let mut e = StyleEngine::new();
    e.add_css(
        "div { color: green; margin-top: 7px } \
         p.b { color: unset; margin-top: unset }",
        Origin::Author,
    );
    let s = e.compute(&f.dom, f.p, &env());
    assert_eq!(s.get("color"), Some("green")); // héritée → parent
    assert_eq!(s.get("margin-top"), Some("0")); // non héritée → initiale
}

#[test]
fn initial_force_valeur_initiale() {
    let f = fixture();
    let mut e = StyleEngine::new();
    e.add_css("div { color: red } p.b { color: initial }", Origin::Author);
    assert_eq!(
        e.compute(&f.dom, f.p, &env()).get("color"),
        Some("black") // valeur initiale de la table
    );
}

#[test]
fn propriete_custom_heritee() {
    let f = fixture();
    let mut e = StyleEngine::new();
    e.add_css("div { --x: 42px }", Origin::Author);
    assert_eq!(e.compute(&f.dom, f.p, &env()).get("--x"), Some("42px"));
}

// ── media queries ─────────────────────────────────────────────

#[test]
fn media_query_filtrage() {
    let f = fixture();
    let mut e = StyleEngine::new();
    e.add_css(
        "@media (min-width: 600px) { div { display: none } }",
        Origin::Author,
    );
    // env par défaut : 1024px → la règle s'applique.
    assert_eq!(
        e.compute(&f.dom, f.div, &env()).get("display"),
        Some("none")
    );
    // Petit viewport → ignorée.
    let small = StyleEnv {
        width_px: 400,
        ..env()
    };
    assert_eq!(
        e.compute(&f.dom, f.div, &small).get("display"),
        Some("block") // retombe sur la feuille UA
    );
}

#[test]
fn media_query_em_rem_resolus() {
    // `em`/`rem` dans une media query = font-size INITIALE
    // (spec MQ4 — jamais le `:root` calculé). Terrain :
    // rust-lang.org (Tachyons) met tout son CSS desktop
    // derrière `(min-width: 30em)`/`(min-width: 60em)` —
    // non résolu = tout ignoré → rendu mobile empilé.
    let f = fixture();
    let mut e = StyleEngine::new();
    e.add_css(
        "@media (min-width: 30em) { div { color: red } } \
         @media (min-width: 64rem) { p { margin-top: 9px } } \
         @media (min-width: 100em) { div { display: none } }",
        Origin::Author,
    );
    // env : 1024px, base 16px → 30em=480 ✓, 64rem=1024 ✓,
    // 100em=1600 ✗.
    let s = e.compute(&f.dom, f.div, &env());
    assert_eq!(s.get("color"), Some("red"));
    assert_eq!(s.get("display"), Some("block"), "100em > 1024px");
    assert_eq!(
        e.compute(&f.dom, f.p, &env()).get("margin-top"),
        Some("9px"),
        "64rem == 1024px à base 16"
    );
    // Base différente → seuil différent (em suit la base).
    let big = StyleEnv {
        font_size_base: 20.0,
        ..env()
    };
    assert_eq!(
        e.compute(&f.dom, f.div, &big).get("color"),
        Some("red"),
        "30em=600 ≤ 1024"
    );
    assert_eq!(
        e.compute(&f.dom, f.p, &big).get("margin-top"),
        Some("1em"), // retombe sur la feuille UA
        "64rem=1280 > 1024 à base 20"
    );
}

#[test]
fn media_query_range_mq4() {
    // R-56 : syntaxe range MQ4 `(width <= N)` — MDN porte tout
    // son habillage conditionnel en range moderne ; avant, le
    // `(width <= 1044px)` sans `:` devenait un NOM de condition
    // inconnu → query fausse → le `display:none` du popup nav
    // ne s'appliquait pas (menu brut visible) et tout le
    // styling conditionnel tombait.
    let f = fixture();
    let mut e = StyleEngine::new();
    e.add_css(
        "@media (width <= 1044px) { div { display: none } } \
         @media (width > 1044px) { div { margin-top: 9px } }",
        Origin::Author,
    );
    // env par défaut 1024 ≤ 1044 : `none`, pas de marge.
    let s = e.compute(&f.dom, f.div, &env());
    assert_eq!(s.get("display"), Some("none"), "1024 ≤ 1044");
    assert_eq!(s.get("margin-top"), None, "1024 ≯ 1044");
    // À 1200 > 1044 : inverse.
    let wide = StyleEnv {
        width_px: 1200,
        ..env()
    };
    let s = e.compute(&f.dom, f.div, &wide);
    assert_eq!(s.get("display"), Some("block"));
    assert_eq!(s.get("margin-top"), Some("9px"));
    // Bordure stricte : à 1044 exactement — `<=` vrai, `>` faux.
    let edge = StyleEnv {
        width_px: 1044,
        ..env()
    };
    let s = e.compute(&f.dom, f.div, &edge);
    assert_eq!(s.get("display"), Some("none"));
    assert_eq!(s.get("margin-top"), None);
}

#[test]
fn media_query_range_formes_et_collage() {
    let f = fixture();
    let mut e = StyleEngine::new();
    e.add_css(
        // Collé (sans espaces) + double borne + `and` + >=.
        "@media (width<=1044px) { div { color: red } } \
         @media (400px <= width <= 700px) { p { margin-top: 9px } } \
         @media (min-width: 600px) and (width >= 900px) { div { display: none } }",
        Origin::Author,
    );
    // env 1024 : collé ✓, borne haute 700 ✗, `and` ≥900 ✓.
    let s = e.compute(&f.dom, f.div, &env());
    assert_eq!(s.get("color"), Some("red"), "(width<=1044px) collé");
    assert_eq!(s.get("display"), Some("none"), "and + >=");
    assert_eq!(
        e.compute(&f.dom, f.p, &env()).get("margin-top"),
        Some("1em"), // retombe sur la feuille UA
        "1024 > 700 → borne double fausse"
    );
    let mid = StyleEnv {
        width_px: 500,
        ..env()
    };
    assert_eq!(
        e.compute(&f.dom, f.p, &mid).get("margin-top"),
        Some("9px"),
        "500 ∈ [400,700]"
    );
}

#[test]
fn media_negation_et_type() {
    let f = fixture();
    let mut eng = StyleEngine::new();
    eng.add_css(
        "@media not print { div { display: none } } \
         @media print { div { margin-top: 9px } }",
        Origin::Author,
    );
    let s = eng.compute(&f.dom, f.div, &env()); // env = screen
    assert_eq!(s.get("display"), Some("none"));
    assert_eq!(s.get("margin-top"), None);
}

// ── compute_tree ──────────────────────────────────────────────

#[test]
fn compute_tree_propague_l_heritage() {
    let f = fixture();
    let mut e = StyleEngine::new();
    e.add_css("body { color: teal }", Origin::Author);
    let map = e.compute_tree(&f.dom, f.dom.document(), &env());
    assert_eq!(map.get(f.div).and_then(|s| s.get("color")), Some("teal"));
    assert_eq!(map.get(f.p).and_then(|s| s.get("color")), Some("teal"));
    assert_eq!(map.get(f.p).and_then(|s| s.get("display")), Some("block"));
}

#[test]
fn compute_sur_handle_obsolete_ne_panique_pas() {
    let mut f = fixture();
    f.dom.remove(f.div).unwrap();
    let _ = f.dom.collect();
    let e = StyleEngine::new();
    let s: ComputedStyle = e.compute(&f.dom, f.div, &env());
    assert_eq!(s.get("display"), None); // vide, pas de panic
}

// ── propriétés custom / var() (étape 42) ──────────────────────

#[test]
fn var_resout_depuis_le_parent() {
    let f = fixture();
    let mut e = StyleEngine::new();
    e.add_css(
        "div { --x: 30px } p { width: var(--x); color: var(--absent, teal) }",
        Origin::Author,
    );
    let s = e.compute(&f.dom, f.p, &env());
    assert_eq!(s.get("width"), Some("30px"));
    assert_eq!(s.get("color"), Some("teal"), "fallback utilisé");
}

#[test]
fn var_declare_sur_le_meme_element() {
    let f = fixture();
    let mut e = StyleEngine::new();
    e.add_css("p { --c: red; color: var(--c) }", Origin::Author);
    assert_eq!(e.compute(&f.dom, f.p, &env()).get("color"), Some("red"));
}

#[test]
fn var_irresoluble_invalide_la_declaration() {
    let f = fixture();
    let mut e = StyleEngine::new();
    e.add_css("p { width: var(--nope) }", Origin::Author);
    // Référence sans fallback → déclaration invalide : la
    // propriété disparaît (pas de texte `var(` résiduel).
    assert_eq!(e.compute(&f.dom, f.p, &env()).get("width"), None);
}

#[test]
fn var_imbrique_dans_fallback_et_valeur() {
    let f = fixture();
    let mut e = StyleEngine::new();
    e.add_css(
        "div { --a: var(--b) } p { width: var(--a, var(--c, 7px)) }",
        Origin::Author,
    );
    // --a → --b absent → --a invalide → fallback var(--c, 7px)
    // → --c absent → 7px.
    assert_eq!(e.compute(&f.dom, f.p, &env()).get("width"), Some("7px"));
}

#[test]
fn var_ne_se_substitut_pas_aux_proprietes_custom() {
    let f = fixture();
    let mut e = StyleEngine::new();
    // `--x: var(--y)` reste textuel (spec : les customs gardent
    // leur token stream — la substitution se fait à l'usage).
    e.add_css(
        "div { --x: var(--y); --y: 4px } p { width: var(--x) }",
        Origin::Author,
    );
    assert_eq!(
        e.compute(&f.dom, f.div, &env()).get("--x"),
        Some("var(--y)")
    );
    // À l'usage : var(--x) → « var(--y) » → re-substitution → 4px.
    assert_eq!(e.compute(&f.dom, f.p, &env()).get("width"), Some("4px"));
}

// ── feuille UA legacy et hints présentationnels (R-16) ────

/// Crée un élément `tag` avec `attrs` sous `parent`.
fn el(dom: &mut Dom, parent: NodeId, tag: &str, attrs: &[(&str, &str)]) -> NodeId {
    let e = dom.create_element(tag).unwrap();
    for (k, v) in attrs {
        dom.set_attribute(e, k, v).unwrap();
    }
    dom.append_child(parent, e).unwrap();
    e
}

#[test]
fn ua_bloc_legacy() {
    let f = fixture();
    let e = StyleEngine::new();
    let mut dom = f.dom;
    for tag in [
        "center",
        "dir",
        "menu",
        "hgroup",
        "search",
        "listing",
        "xmp",
        "plaintext",
        "details",
        "summary",
        "marquee",
        "frameset",
    ] {
        let n = el(&mut dom, f.body, tag, &[]);
        assert_eq!(
            e.compute(&dom, n, &env()).get("display"),
            Some("block"),
            "<{tag}> doit être block"
        );
    }
}

#[test]
fn ua_center_porte_webkit_center() {
    let f = fixture();
    let e = StyleEngine::new();
    let mut dom = f.dom;
    let c = el(&mut dom, f.body, "center", &[]);
    assert_eq!(
        e.compute(&dom, c, &env()).get("text-align"),
        Some("-webkit-center"),
        "spec : <center> aligne aussi les descendants bloc"
    );
}

#[test]
fn ua_hidden_et_dialog_ferme_masques() {
    let f = fixture();
    let e = StyleEngine::new();
    let mut dom = f.dom;
    let hid = el(&mut dom, f.body, "div", &[("hidden", "")]);
    assert_eq!(e.compute(&dom, hid, &env()).get("display"), Some("none"));
    let closed = el(&mut dom, f.body, "dialog", &[]);
    assert_eq!(e.compute(&dom, closed, &env()).get("display"), Some("none"));
    let opened = el(&mut dom, f.body, "dialog", &[("open", "")]);
    assert_eq!(
        e.compute(&dom, opened, &env()).get("display"),
        Some("block")
    );
}

#[test]
fn ua_details_ferme_ne_montre_que_summary() {
    let f = fixture();
    let e = StyleEngine::new();
    let mut dom = f.dom;
    let det = el(&mut dom, f.body, "details", &[]);
    let sum = el(&mut dom, det, "summary", &[]);
    let p = el(&mut dom, det, "p", &[]);
    assert_eq!(e.compute(&dom, sum, &env()).get("display"), Some("block"));
    assert_eq!(e.compute(&dom, p, &env()).get("display"), Some("none"));
    // details[open] : le p redevient visible.
    let open = el(&mut dom, f.body, "details", &[("open", "")]);
    let p2 = el(&mut dom, open, "p", &[]);
    assert_eq!(e.compute(&dom, p2, &env()).get("display"), Some("block"));
}

#[test]
fn ua_noscript_reste_visible() {
    // Scripting désactivé (domaine pur) : le contenu de
    // <noscript> est un vrai sous-arbre rendu — contrairement
    // à un navigateur scripté qui le masque.
    let f = fixture();
    let e = StyleEngine::new();
    let mut dom = f.dom;
    let n = el(&mut dom, f.body, "noscript", &[]);
    assert_ne!(e.compute(&dom, n, &env()).get("display"), Some("none"));
}

#[test]
fn hint_bgcolor_vers_background_color() {
    let f = fixture();
    let e = StyleEngine::new();
    let mut dom = f.dom;
    for tag in ["table", "tr", "td", "th", "body"] {
        let n = el(&mut dom, f.body, tag, &[("bgcolor", "#ff6600")]);
        assert_eq!(
            e.compute(&dom, n, &env()).get("background-color"),
            Some("#ff6600"),
            "<{tag} bgcolor> → background-color"
        );
    }
}

#[test]
fn hint_align_vers_text_align() {
    let f = fixture();
    let e = StyleEngine::new();
    let mut dom = f.dom;
    let td = el(&mut dom, f.body, "td", &[("align", "right")]);
    assert_eq!(e.compute(&dom, td, &env()).get("text-align"), Some("right"));
    // `middle` vaut `center` en hint (HTML Rendering).
    let d = el(&mut dom, f.body, "div", &[("align", "middle")]);
    assert_eq!(e.compute(&dom, d, &env()).get("text-align"), Some("center"));
    // Valeur invalide → pas de hint (propriété non déclarée).
    let x = el(&mut dom, f.body, "td", &[("align", "nawak")]);
    assert_eq!(e.compute(&dom, x, &env()).get("text-align"), None);
}

#[test]
fn hint_table_spacing_padding_border() {
    let f = fixture();
    let e = StyleEngine::new();
    let mut dom = f.dom;
    let table = el(
        &mut dom,
        f.body,
        "table",
        &[("cellspacing", "7"), ("cellpadding", "4"), ("border", "2")],
    );
    let tr = el(&mut dom, table, "tr", &[]);
    let td = el(&mut dom, tr, "td", &[]);
    let s = e.compute(&dom, table, &env());
    assert_eq!(s.get("border-spacing"), Some("7px"));
    assert_eq!(s.get("border-width"), Some("2px"));
    assert_eq!(s.get("border-style"), Some("outset"));
    let c = e.compute(&dom, td, &env());
    // cellpadding de la table ancêtre → padding de la cellule.
    assert_eq!(c.get("padding"), Some("4px"));
    // border > 0 → filet 1px inset sur les cellules.
    assert_eq!(c.get("border-width"), Some("1px"));
    assert_eq!(c.get("border-style"), Some("inset"));
}

#[test]
fn hint_body_text_et_valign() {
    let f = fixture();
    let e = StyleEngine::new();
    let mut dom = f.dom;
    let body = el(&mut dom, f.body, "body", &[("text", "silver")]);
    assert_eq!(e.compute(&dom, body, &env()).get("color"), Some("silver"));
    let td = el(&mut dom, f.body, "td", &[("valign", "middle")]);
    assert_eq!(
        e.compute(&dom, td, &env()).get("vertical-align"),
        Some("middle")
    );
}

// ── font-size : valeur calculée absolue (R-17) ────────────

#[test]
fn font_size_em_resolu_en_px() {
    // Spec : la valeur calculée de font-size est absolue —
    // « 2em » sous parent 16px → « 32px » stocké, pas le brut.
    let f = fixture();
    let mut e = StyleEngine::new();
    e.add_css("div { font-size: 2em }", Origin::Author);
    assert_eq!(
        e.compute(&f.dom, f.div, &env()).get("font-size"),
        Some("32px")
    );
}

#[test]
fn font_size_heritage_valeur_absolue() {
    // Le fils sans déclaration hérite la valeur CALCULÉE —
    // sinon le « 2em » hérité se re-multipliait à chaque
    // niveau de boîte (le bug terrain : texte ×4/×8).
    let f = fixture();
    let mut e = StyleEngine::new();
    e.add_css("div { font-size: 2em }", Origin::Author);
    assert_eq!(
        e.compute(&f.dom, f.p, &env()).get("font-size"),
        Some("32px")
    );
}

#[test]
fn font_size_mots_cles_et_pourcents() {
    let f = fixture();
    let mut e = StyleEngine::new();
    e.add_css("div { font-size: medium }", Origin::Author);
    assert_eq!(
        e.compute(&f.dom, f.div, &env()).get("font-size"),
        Some("16px")
    );

    let mut dom2 = f.dom;
    let big = el(&mut dom2, f.body, "p", &[("style", "font-size:200%")]);
    assert_eq!(e.compute(&dom2, big, &env()).get("font-size"), Some("32px"));
    let larger = el(&mut dom2, big, "span", &[("style", "font-size:larger")]);
    // `larger` ≈ parent × 1.2 (convention navigateur).
    assert_eq!(
        e.compute(&dom2, larger, &env()).get("font-size"),
        Some("38.4px")
    );
}

#[test]
fn font_size_rem_et_absolus_inchanges() {
    let f = fixture();
    let mut e = StyleEngine::new();
    e.add_css(
        "div { font-size: 2rem } p { font-size: 12pt }",
        Origin::Author,
    );
    // rem → base (16), indépendant du parent.
    assert_eq!(
        e.compute(&f.dom, f.div, &env()).get("font-size"),
        Some("32px")
    );
    // absolu direct (pt → px).
    assert_eq!(
        e.compute(&f.dom, f.p, &env()).get("font-size"),
        Some("16px")
    );
}

// ── R-10 : @keyframes + animation-* ─────────────────────────

/// `<div id=d>` seul — suffit aux tests animation/keyframes.
fn styled(css: &str) -> (Dom, NodeId, StyleEngine, explorer_css::StyleMap) {
    let mut dom = Dom::new();
    let doc = dom.document();
    let div = dom.create_element("div").unwrap();
    dom.set_attribute(div, "id", "d").unwrap();
    dom.append_child(doc, div).unwrap();
    let mut eng = StyleEngine::new();
    eng.add_css(css, Origin::Author);
    let styles = eng.compute_tree(&dom, div, &StyleEnv::default());
    (dom, div, eng, styles)
}

#[test]
fn keyframes_collectees_et_triees() {
    let (_dom, _n, eng, _s) = styled("@keyframes pulse { to { opacity: 1 } 0% { opacity: 0 } }");
    let frames = eng.keyframes("pulse").expect("keyframes collectées");
    assert_eq!(frames.len(), 2);
    assert!(frames[0].0 < frames[1].0, "triées par offset : {frames:?}");
    assert!(frames[0].0.abs() < 1e-6 && (frames[1].0 - 1.0).abs() < 1e-6);
}

#[test]
fn keyframes_multi_offset_eclate() {
    let (_dom, _n, eng, _s) = styled("@keyframes k { 0%, 100% { opacity: 0 } 50% { opacity: 1 } }");
    let frames = eng.keyframes("k").expect("k");
    assert_eq!(frames.len(), 3, "0% + 50% + 100% : {frames:?}");
}

#[test]
fn animation_shorthand_et_phase() {
    use explorer_css::animation::{animation_decls, keyframe_style, Fill};
    let (_dom, node, eng, styles) = styled(
        "@keyframes slide { from { left: 0px } to { left: 100px } }\
         #d { animation: slide 2s linear forwards }",
    );
    let st = styles.get(node).expect("style");
    let decls = animation_decls(st);
    assert_eq!(decls.len(), 1);
    let d = &decls[0];
    assert_eq!(d.name, "slide");
    assert!((d.duration - 2.0).abs() < 1e-9);
    assert_eq!(d.fill, Fill::Forwards);
    // phase : t=1s → 0.5 ; après la fin → forwards figé à 1.
    assert!((d.phase(1.0).unwrap() - 0.5).abs() < 1e-9);
    assert_eq!(d.phase(3.0), Some(1.0));
    // interpolation à mi-chemin : left = 50px.
    let frames = eng.keyframes("slide").unwrap();
    let props = keyframe_style(frames, 0.5, d.timing, st);
    let left = props
        .iter()
        .find(|(n, _)| n == "left")
        .map(|(_, v)| v.as_str());
    assert_eq!(left, Some("50px"), "props={props:?}");
}

#[test]
fn animation_fill_backwards_pendant_le_delai() {
    use explorer_css::animation::animation_decls;
    let (_dom, node, _eng, styles) = styled("#d { animation: x 1s linear 2s backwards }");
    let decls = animation_decls(styles.get(node).unwrap());
    // t=1s : en délai → première frame (phase 0).
    assert_eq!(decls[0].phase(1.0), Some(0.0));
}

#[test]
fn animation_direction_alternate() {
    use explorer_css::animation::{animation_decls, Direction};
    let (_dom, node, _eng, styles) = styled("#d { animation: x 1s linear 0s 2 alternate }");
    let d = &animation_decls(styles.get(node).unwrap())[0];
    assert_eq!(d.direction, Direction::Alternate);
    // Itération 1 (t=1.5) → inversée : frac .5 → phase .5.
    assert!((d.phase(1.5).unwrap() - 0.5).abs() < 1e-9);
}

// ── validation par propriété (R-13e / Acid2) ──────────────────

#[test]
fn longueur_sans_unite_dropee() {
    let f = fixture();
    let mut e = StyleEngine::new();
    // `width: 200` (sans unité, ≠ 0) invalide → la déclaration
    // antérieure valide survit.
    e.add_css("p { width: 2em } p { width: 200 }", Origin::Author);
    assert_eq!(e.compute(&f.dom, f.p, &env()).get("width"), Some("2em"));
}

#[test]
fn background_invalide_drope_pas_remplace() {
    let f = fixture();
    let mut e = StyleEngine::new();
    // `background: red pink` (deux couleurs) invalide → drop ;
    // le `background-color` antérieur survit.
    e.add_css(
        "p { background: yellow } p { background: red pink }",
        Origin::Author,
    );
    assert_eq!(
        e.compute(&f.dom, f.p, &env()).get("background-color"),
        Some("yellow")
    );
}

#[test]
fn echappement_dans_nom_de_propriete() {
    let f = fixture();
    let mut e = StyleEngine::new();
    // `m\argin` ≠ `margin` : `\a` est l'échappement hex U+000A —
    // la propriété résultante est inconnue et ignorée (Acid2).
    e.add_css(r"p { m\argin: 2em; margin-top: 9px }", Origin::Author);
    let s = e.compute(&f.dom, f.p, &env());
    assert_eq!(s.get("margin-top"), Some("9px"));
    assert_eq!(s.get("margin-left"), None, r"m\argin ≠ margin");
}

// ── quirks mode — reset des propriétés héritées sur table ────
// HTML Standard § rendering « In quirks mode » : la table
// réinitialise les propriétés texte héritées (le `<center>`
// autour d'une table ne centre pas les cellules — terrain :
// news.ycombinator.com, R-38). Le mode voyage par
// `Dom::document_mode`, posé par le tree-builder au DOCTYPE.

/// Premier descendant portant `tag` (ordre document).
fn find_tag(dom: &Dom, node: NodeId, tag: &str) -> Option<NodeId> {
    if dom.tag_name(node).ok() == Some(tag) {
        return Some(node);
    }
    for c in dom.children(node).unwrap_or_default() {
        if let Some(f) = find_tag(dom, c, tag) {
            return Some(f);
        }
    }
    None
}

#[test]
fn quirks_table_reset_text_align() {
    // Pas de DOCTYPE → quirks : `<center>` n'atteint pas les
    // cellules (reset `text-align: initial` sur `table`).
    let dom = TreeBuilder::parse("<center><table><tr><td>x</td></tr></table></center>");
    assert_eq!(dom.document_mode(), DocumentMode::Quirks);
    let td = find_tag(&dom, dom.document(), "td").unwrap();
    let s = StyleEngine::new().compute(&dom, td, &env());
    assert_eq!(s.get("text-align"), Some("start"));
}

#[test]
fn no_quirks_table_herite_text_align() {
    // Même markup avec DOCTYPE conforme : héritage normal —
    // la valeur `-webkit-center` de `<center>` traverse.
    let dom =
        TreeBuilder::parse("<!DOCTYPE html><center><table><tr><td>x</td></tr></table></center>");
    assert_eq!(dom.document_mode(), DocumentMode::NoQuirks);
    let td = find_tag(&dom, dom.document(), "td").unwrap();
    let s = StyleEngine::new().compute(&dom, td, &env());
    assert_eq!(s.get("text-align"), Some("-webkit-center"));
}

#[test]
fn quirks_table_reset_font_weight() {
    // `<b>` n'est pas un match du reset (propriété *héritée*
    // seulement) : `font-weight` revient à `normal` sur la
    // table, `th` garde son propre `bold` déclaré en UA.
    let dom = TreeBuilder::parse("<b><table><tr><td>x</td><th>h</th></tr></table></b>");
    assert_eq!(dom.document_mode(), DocumentMode::Quirks);
    let e = StyleEngine::new();
    let td = find_tag(&dom, dom.document(), "td").unwrap();
    assert_eq!(
        e.compute(&dom, td, &env()).get("font-weight"),
        Some("normal")
    );
    let th = find_tag(&dom, dom.document(), "th").unwrap();
    assert_eq!(e.compute(&dom, th, &env()).get("font-weight"), Some("bold"));
}

#[test]
fn quirks_table_font_size_initiale() {
    // `font-size` héritée est réinitialisée à `medium` (base
    // 16px) — un `font-size` sur l'ancêtre ne traverse pas.
    let dom = TreeBuilder::parse(
        "<span style=\"font-size: 30px\"><table><tr><td>x</td></tr></table></span>",
    );
    assert_eq!(dom.document_mode(), DocumentMode::Quirks);
    let td = find_tag(&dom, dom.document(), "td").unwrap();
    let s = StyleEngine::new().compute(&dom, td, &env());
    assert_eq!(s.get("font-size"), Some("16px"));
}

#[test]
fn media_query_unites_viewport() {
    // R-42 : `vw`/`vh`/`vmin`/`vmax` dans les media queries —
    // résolus contre `env.width_px`/`height_px` (MQ4 §6.4).
    // env() : 1024×768 → 50vw=512 ✓, 50vh=384 ✗, 100vmin=768.
    let f = fixture();
    let mut e = StyleEngine::new();
    e.add_css(
        "@media (min-width: 50vw) { div { color: red } } \
         @media (max-width: 50vh) { p { margin-top: 9px } } \
         @media (min-width: 100vmin) { div { display: none } }",
        Origin::Author,
    );
    let s = e.compute(&f.dom, f.div, &env());
    assert_eq!(s.get("color"), Some("red"), "50vw=512 ≤ 1024");
    assert_eq!(
        s.get("display"),
        Some("none"),
        "100vmin=min(1024,768)=768 ≤ 1024"
    );
    assert_eq!(
        e.compute(&f.dom, f.p, &env()).get("margin-top"),
        Some("1em"),
        "50vh=384 < 1024 → query fausse, retombe UA"
    );
}

#[test]
fn font_size_unites_viewport() {
    // R-42 : `font-size:Nvw` est légal — résolu en valeur
    // calculée absolue contre `env` (1024 → 2vw = 20.48px).
    let f = fixture();
    let mut e = StyleEngine::new();
    e.add_css(
        "div { font-size: 2vw } p { font-size: 1vmax }",
        Origin::Author,
    );
    let s = e.compute(&f.dom, f.div, &env());
    assert_eq!(s.get("font-size"), Some("20.48px"), "2vw de 1024");
    assert_eq!(
        e.compute(&f.dom, f.p, &env()).get("font-size"),
        Some("10.24px"),
        "1vmax de max(1024,768)"
    );
}
