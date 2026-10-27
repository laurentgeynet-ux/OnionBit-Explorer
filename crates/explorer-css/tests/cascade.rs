// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Tests de la cascade : origines, spécificité, `!important`,
//! héritage, mots-clés, media queries, feuille UA.

use explorer_css::{matches, parse_selector_list, ComputedStyle, Origin, StyleEngine, StyleEnv};
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

// ── @layer (Cascade 5 §6.4 — R-91) ────────────────────────────

#[test]
fn regle_en_layer_sapplique() {
    // Terrain github : tout le reset Primer vit dans
    // `@layer primer-css-base{…}` — le bloc était avalé entier
    // (at-rule inconnu) → `*{box-sizing:border-box}` perdu,
    // les éléments restaient content-box.
    let f = fixture();
    let mut e = StyleEngine::new();
    e.add_css(
        "@layer primer-css-base { * { box-sizing: border-box } }",
        Origin::Author,
    );
    assert_eq!(
        e.compute(&f.dom, f.div, &env()).get("box-sizing"),
        Some("border-box")
    );
}

#[test]
fn hors_layer_bat_tous_les_layers() {
    // Hors-layer gagne même avec une spécificité inférieure et
    // une position plus ancienne (§6.4.4) — la règle en layer
    // s'applique quand même sur ses autres propriétés.
    let f = fixture();
    let mut e = StyleEngine::new();
    e.add_css(
        "div { display: none } \
         @layer base { #a { display: block } div { color: rgb(1, 2, 3) } }",
        Origin::Author,
    );
    let s = e.compute(&f.dom, f.div, &env());
    assert_eq!(s.get("display"), Some("none"));
    assert_eq!(s.get("color"), Some("rgb(1, 2, 3)"));
}

#[test]
fn ordre_des_layers_prime_sur_specificite() {
    // `b` déclaré après `a` → `b` gagne malgré un sélecteur
    // moins spécifique : le rang de layer prime sur (id,class,type).
    let f = fixture();
    let mut e = StyleEngine::new();
    e.add_css(
        "@layer a { #a { color: rgb(1, 2, 3) } } \
         @layer b { div { color: rgb(4, 5, 6) } }",
        Origin::Author,
    );
    assert_eq!(
        e.compute(&f.dom, f.div, &env()).get("color"),
        Some("rgb(4, 5, 6)")
    );
}

#[test]
fn ordre_des_layers_declare_par_statement() {
    // `@layer b, a;` fixe l'ordre d'abord : `a` gagne même si
    // son bloc arrive avant celui de `b` dans la source.
    let f = fixture();
    let mut e = StyleEngine::new();
    e.add_css(
        "@layer b, a; @layer a { div { color: rgb(4, 5, 6) } } \
         @layer b { #a { color: rgb(1, 2, 3) } }",
        Origin::Author,
    );
    assert_eq!(
        e.compute(&f.dom, f.div, &env()).get("color"),
        Some("rgb(4, 5, 6)")
    );
}

#[test]
fn important_inverse_ordre_des_layers() {
    // `!important` renverse l'ordre : le PREMIER layer gagne ;
    // un hors-layer `!important` est le plus faible de tous.
    let f = fixture();
    let mut e = StyleEngine::new();
    e.add_css(
        "@layer a { div { display: none !important } } \
         @layer b { #a { display: block !important } }",
        Origin::Author,
    );
    assert_eq!(
        e.compute(&f.dom, f.div, &env()).get("display"),
        Some("none"),
        "a !important bat b !important (ordre renversé)"
    );
    e.add_css("div { display: inline !important }", Origin::Author);
    assert_eq!(
        e.compute(&f.dom, f.div, &env()).get("display"),
        Some("none"),
        "hors-layer !important < tout layer !important"
    );
    e.add_css("div { display: inline }", Origin::Author);
    assert_eq!(
        e.compute(&f.dom, f.div, &env()).get("display"),
        Some("none"),
        "layer !important bat hors-layer normal"
    );
}

#[test]
fn layer_anonyme_et_imbrication() {
    // Deux `@layer{}` anonymes : slots distincts, le second gagne.
    let f = fixture();
    let mut e = StyleEngine::new();
    e.add_css(
        "@layer { div { display: none } } @layer { div { display: block } }",
        Origin::Author,
    );
    assert_eq!(
        e.compute(&f.dom, f.div, &env()).get("display"),
        Some("block")
    );
    // Sublayer `a.b` : déclaré dans `a` après ses règles
    // propres → il leur est postérieur.
    let mut e2 = StyleEngine::new();
    e2.add_css(
        "@layer a { div { display: inline } @layer b { div { display: block } } }",
        Origin::Author,
    );
    assert_eq!(
        e2.compute(&f.dom, f.div, &env()).get("display"),
        Some("block")
    );
    // Nom pointé direct `@layer a.b{}` ≡ sous-couche de `a`.
    let mut e3 = StyleEngine::new();
    e3.add_css(
        "@layer a { div { display: none } } @layer a.b { div { display: table } }",
        Origin::Author,
    );
    assert_eq!(
        e3.compute(&f.dom, f.div, &env()).get("display"),
        Some("table")
    );
}

#[test]
fn layer_sous_media() {
    // `@layer` imbriqué dans `@media` : layer + requête cumulés —
    // la règle s'applique (color) sans battre le hors-layer
    // (display).
    let f = fixture();
    let mut e = StyleEngine::new();
    e.add_css(
        "div { display: block } \
         @media all { @layer a { div { display: none; color: rgb(1, 2, 3) } } }",
        Origin::Author,
    );
    let s = e.compute(&f.dom, f.div, &env());
    assert_eq!(s.get("display"), Some("block"));
    assert_eq!(s.get("color"), Some("rgb(1, 2, 3)"));
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
fn media_query_prefers_color_scheme() {
    // R-59 — MQ5 `prefers-color-scheme` : `dark`/`light`
    // contre l'env, feature nue (booléen) toujours vraie.
    let f = fixture();
    let mut e = StyleEngine::new();
    e.add_css(
        "@media (prefers-color-scheme: dark) { div { color: red } } \
         @media (prefers-color-scheme: light) { p { margin-top: 9px } } \
         @media (prefers-color-scheme) { div { display: none } }",
        Origin::Author,
    );
    // env Light (défaut) : dark faux, light vrai, nue vraie.
    let s = e.compute(&f.dom, f.div, &env());
    assert_eq!(s.get("color"), None, "env light : dark faux");
    assert_eq!(s.get("display"), Some("none"), "feature nue → vrai");
    assert_eq!(
        e.compute(&f.dom, f.p, &env()).get("margin-top"),
        Some("9px"),
        "env light : light vrai"
    );
    let dark = StyleEnv {
        color_scheme: explorer_css::ColorScheme::Dark,
        ..env()
    };
    let s = e.compute(&f.dom, f.div, &dark);
    assert_eq!(s.get("color"), Some("red"), "env dark : dark vrai");
    assert_eq!(
        e.compute(&f.dom, f.p, &dark).get("margin-top"),
        Some("1em"),
        "env dark : light retombe sur l'UA"
    );
}

#[test]
fn media_query_scripting() {
    // R-93 — MQ5 `scripting` : `enabled`/`none` contre
    // `env.scripting`, feature nue = « pas none », et
    // `initial-only` jamais rapporté par notre moteur.
    let f = fixture();
    let mut e = StyleEngine::new();
    e.add_css(
        "@media (scripting: enabled) { div { color: red } } \
         @media (scripting: none) { p { margin-top: 9px } } \
         @media (scripting) { div { display: none } } \
         @media (scripting: initial-only) { p { margin-bottom: 9px } }",
        Origin::Author,
    );
    // scripting off (défaut) : enabled faux, none vrai,
    // nue fausse, initial-only faux.
    let s = e.compute(&f.dom, f.div, &env());
    assert_eq!(s.get("color"), None, "off : enabled faux");
    assert_eq!(s.get("display"), Some("block"), "off : nue fausse");
    assert_eq!(
        e.compute(&f.dom, f.p, &env()).get("margin-top"),
        Some("9px"),
        "off : none vrai"
    );
    assert_eq!(
        e.compute(&f.dom, f.p, &env()).get("margin-bottom"),
        Some("1em"),
        "off : initial-only faux → retombe sur l'UA"
    );
    let on = StyleEnv {
        scripting: true,
        ..env()
    };
    let s = e.compute(&f.dom, f.div, &on);
    assert_eq!(s.get("color"), Some("red"), "on : enabled vrai");
    assert_eq!(s.get("display"), Some("none"), "on : nue vraie");
    let s = e.compute(&f.dom, f.p, &on);
    assert_eq!(
        s.get("margin-top"),
        Some("1em"),
        "on : none retombe sur l'UA"
    );
    assert_eq!(
        s.get("margin-bottom"),
        Some("1em"),
        "on : initial-only jamais vrai"
    );
}

#[test]
fn noscript_masque_sous_scripting() {
    // R-93 — la feuille UA masque `<noscript>` quand
    // `env.scripting` (fuite de rawtext mesurée sur wikipedia,
    // issue #22) ; sans scripting il reste un sous-arbre
    // rendu (comportement historique).
    let dom = TreeBuilder::parse("<body><noscript><p>x</p></noscript></body>");
    let noscript = {
        let html = dom
            .children(dom.document())
            .unwrap_or_default()
            .into_iter()
            .find(|&c| dom.tag_name(c).is_ok_and(|t| t == "html"))
            .expect("html présent");
        let body = dom
            .children(html)
            .unwrap_or_default()
            .into_iter()
            .find(|&b| dom.tag_name(b).is_ok_and(|t| t == "body"))
            .expect("body présent");
        dom.children(body)
            .unwrap_or_default()
            .into_iter()
            .find(|&n| dom.tag_name(n).is_ok_and(|t| t == "noscript"))
            .expect("noscript présent")
    };
    let e = StyleEngine::new();
    assert_eq!(
        e.compute(&dom, noscript, &env()).get("display"),
        None,
        "sans scripting : noscript reste à sa valeur initiale (UA ne masque pas)"
    );
    let on = StyleEnv {
        scripting: true,
        ..env()
    };
    assert_eq!(
        e.compute(&dom, noscript, &on).get("display"),
        Some("none"),
        "sous scripting : UA masque noscript"
    );
}

/// Vol 2026-10-26 (mastodon) — la règle UA est
/// `display: none !important` comme la spec HTML Rendering :
/// une feuille auteur `noscript { display:flex }` ne doit
/// PAS révéler le fallback rawtext sous scripting.
#[test]
fn noscript_important_coiffe_style_auteur() {
    let dom = TreeBuilder::parse("<body><noscript>fallback</noscript></body>");
    let noscript = {
        let html = dom
            .children(dom.document())
            .unwrap_or_default()
            .into_iter()
            .find(|&c| dom.tag_name(c).is_ok_and(|t| t == "html"))
            .expect("html présent");
        let body = dom
            .children(html)
            .unwrap_or_default()
            .into_iter()
            .find(|&b| dom.tag_name(b).is_ok_and(|t| t == "body"))
            .expect("body présent");
        dom.children(body)
            .unwrap_or_default()
            .into_iter()
            .find(|&n| dom.tag_name(n).is_ok_and(|t| t == "noscript"))
            .expect("noscript présent")
    };
    let mut e = StyleEngine::new();
    e.add_css("noscript { display: flex }", Origin::Author);
    let on = StyleEnv {
        scripting: true,
        ..env()
    };
    assert_eq!(
        e.compute(&dom, noscript, &on).get("display"),
        Some("none"),
        "scripting : l'important UA coiffe le display:flex auteur"
    );
    // Même un `!important` auteur cède devant l'important UA
    // (ordre de cascade : UA important > auteur important).
    let mut e2 = StyleEngine::new();
    e2.add_css("noscript { display: flex !important }", Origin::Author);
    assert_eq!(
        e2.compute(&dom, noscript, &on).get("display"),
        Some("none"),
        "scripting : l'important UA coiffe l'important auteur"
    );
    // Sans scripting : la règle auteur s'applique (UA inerte).
    let mut e3 = StyleEngine::new();
    e3.add_css("noscript { display: flex }", Origin::Author);
    assert_eq!(
        e3.compute(&dom, noscript, &env()).get("display"),
        Some("flex"),
        "sans scripting : le style auteur gagne"
    );
}

#[test]
fn color_scheme_dark_defauts_ua() {
    // R-59 — `color-scheme` sur la racine arbitre les défauts
    // UA sombres (canvas + texte) : `dark` utilisé quand env
    // dark OU qu'aucun `light` n'est déclaré ; `light`
    // explicite = opt-out ; les déclarations auteur gardent
    // la main (rang UA).
    let dom = TreeBuilder::parse("<p>vu</p>");
    let root = dom
        .children(dom.document())
        .into_iter()
        .flatten()
        .find(|&c| dom.element(c).is_ok())
        .unwrap();
    let dark_env = StyleEnv {
        color_scheme: explorer_css::ColorScheme::Dark,
        ..env()
    };
    let styled = |css: &str, e: &StyleEnv| {
        let mut eng = StyleEngine::new();
        eng.add_css(css, Origin::Author);
        eng.compute_tree(&dom, root, e)
    };
    // `dark` déclaré + env dark → canvas/texte inversés.
    let m = styled("html { color-scheme: dark }", &dark_env);
    let html = m.get(root).unwrap();
    assert_eq!(html.get("background-color"), Some("#141414"));
    assert_eq!(html.get("color"), Some("#e6e6e6"));
    // Rang UA : l'auteur garde la main où il déclare.
    let m = styled(
        "html { color-scheme: dark; background-color: white }",
        &dark_env,
    );
    let html = m.get(root).unwrap();
    assert_eq!(html.get("background-color"), Some("white"));
    assert_eq!(html.get("color"), Some("#e6e6e6"));
    // Opt-out : `light` explicite sous env dark → rien.
    let m = styled("html { color-scheme: light }", &dark_env);
    assert_eq!(m.get(root).unwrap().get("background-color"), None);
    // Sans déclaration : rien même en env dark.
    let m = styled("", &dark_env);
    assert_eq!(m.get(root).unwrap().get("background-color"), None);
    // `dark` seul déclaré sous env light → sombre utilisé
    // (la page force le schéma).
    let m = styled("html { color-scheme: dark }", &env());
    assert_eq!(
        m.get(root).unwrap().get("background-color"),
        Some("#141414")
    );
    // `light dark` sous env light → light gagne → rien.
    let m = styled("html { color-scheme: light dark }", &env());
    assert_eq!(m.get(root).unwrap().get("background-color"), None);
    // Le texte des descendants hérite la couleur sombre.
    let m = styled("html { color-scheme: dark }", &dark_env);
    let body = dom
        .children(root)
        .unwrap()
        .into_iter()
        .find(|&c| dom.tag_name(c).is_ok_and(|t| t == "body"))
        .unwrap();
    let p = dom
        .children(body)
        .unwrap()
        .into_iter()
        .find(|&c| dom.tag_name(c).is_ok_and(|t| t == "p"))
        .unwrap();
    assert_eq!(
        m.get(p).unwrap().get("color"),
        Some("#e6e6e6"),
        "color héritée du défaut sombre"
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

// ── R-80 — `initial` sur --* = guaranteed-invalid value ──
// css-variables-1 §3.2 : l'initiale d'une propriété custom est
// la *guaranteed-invalid value* — un `var()` qui la référence
// échoue et prend son fallback (ou invalide la déclaration).
// Le mécanisme « light-dark » de MDN (`--scheme: initial`
// invalidant un toggle `--t: var(--scheme) var(--white)`)
// reposait dessus : la --* réduite à `""` rendait la
// substitution « réussie » avec ` #fff` → texte blanc sur
// blanc (titres de cartes et contributions invisibles,
// issue #14).

#[test]
fn var_initial_sur_custom_est_guaranteed_invalid() {
    let f = fixture();
    let mut e = StyleEngine::new();
    e.add_css(
        "div { --t: var(--scheme) #fff; color: var(--t, #000) } \
         p { --scheme: initial }",
        Origin::Author,
    );
    // --t référence --scheme (GIV sur p) → --t invalide →
    // color prend le fallback #000, pas «  #fff ».
    assert_eq!(e.compute(&f.dom, f.p, &env()).get("color"), Some("#000"));
    // --scheme masquée : get() ne retourne pas la sentinelle.
    assert_eq!(e.compute(&f.dom, f.p, &env()).get("--scheme"), None);
}

#[test]
fn var_initial_sur_custom_masque_heritage() {
    let f = fixture();
    let mut e = StyleEngine::new();
    // div fixe --x ; p la réduit à `initial` → GIV propre qui
    // MASQUE la valeur héritée (spec : initial ≠ absent).
    e.add_css(
        "div { --x: blue } p { --x: initial; color: var(--x, red) }",
        Origin::Author,
    );
    assert_eq!(
        e.compute(&f.dom, f.p, &env()).get("color"),
        Some("red"),
        "la --* propre GIV masque --x hérité → fallback"
    );
}

#[test]
fn var_inherit_unset_sur_custom() {
    let f = fixture();
    // `inherit`/`unset` sur --* → valeur du parent.
    let mut e = StyleEngine::new();
    e.add_css(
        "div { --x: blue } p { --x: unset; color: var(--x, red) }",
        Origin::Author,
    );
    assert_eq!(e.compute(&f.dom, f.p, &env()).get("color"), Some("blue"));
    // Parent sans la --* → `unset` → GIV → fallback.
    let mut e2 = StyleEngine::new();
    e2.add_css("p { --z: unset; width: var(--z, 5px) }", Origin::Author);
    assert_eq!(e2.compute(&f.dom, f.p, &env()).get("width"), Some("5px"));
}

#[test]
fn var_guaranteed_invalid_sans_fallback_invalide() {
    let f = fixture();
    let mut e = StyleEngine::new();
    // Référence à une --* GIV sans fallback → déclaration
    // invalide « at computed-value time » : `width` non
    // héritée → propriété retirée (initiale `auto`).
    e.add_css("div { --x: initial } p { width: var(--x) }", Origin::Author);
    assert_eq!(e.compute(&f.dom, f.p, &env()).get("width"), None);
}

#[test]
fn var_custom_valeur_vide_valide() {
    let f = fixture();
    // `--x: ;` = stream vide valide (css-variables-1 —
    // `<declaration-value>` peut n'être qu'un whitespace) :
    // le toggle `[data-scheme=dark]` de MDN s'en sert pour
    // sélectionner le jeton droit de `var(--scheme) var(--c)`.
    let mut e = StyleEngine::new();
    e.add_css(
        "p { --t: var(--scheme) #fff; --scheme: ; color: var(--t, #000) }",
        Origin::Author,
    );
    let s = e.compute(&f.dom, f.p, &env());
    // --scheme vide → --t = «  #fff » → couleur #fff, pas le
    // fallback #000.
    assert_eq!(
        s.color("color"),
        Some(explorer_css::value::Color {
            r: 255,
            g: 255,
            b: 255,
            a: 255
        })
    );
}

// ── R-92 — les noms `--*` sont SENSIBLES À LA CASSE ───────
// css-variables-1 §2.1 : les propriétés custom sont des
// identifiants sensibles à la casse — `--Foo` et `--foo` sont
// deux propriétés DISTINCTES (contrairement aux propriétés
// ordinaires, ASCII-insensibles). La cascade normalisait tous
// les noms en minuscules à la déclaration : une `--*` à casse
// mixte était stockée minuscule et jamais retrouvée par le
// `var(--CasseExacte)` — tout le fallback s'effondrait au
// littéral terminal. Mesuré sur github.com : `.lp-Spacer` lit
// `var(--Spacer-size1280, var(--Spacer-size1012, …,
// var(--Spacer-size, 156px)))` avec les --Spacer-size* posées
// en `style=` — le spacer `.lp-IntroVisuals` (660px attendu
// à ≥1012px) s'écrasait à 156px, le hero sombre perdait sa
// hauteur et les sections suivantes remontaient dedans.

#[test]
fn var_nom_custom_sensible_a_la_casse() {
    let f = fixture();
    let mut e = StyleEngine::new();
    e.add_css(
        "div { --Foo: 42px } p { width: var(--Foo); color: var(--foo, teal) }",
        Origin::Author,
    );
    let s = e.compute(&f.dom, f.p, &env());
    assert_eq!(
        s.get("width"),
        Some("42px"),
        "var(--Foo) doit joindre --Foo déclarée telle quelle"
    );
    assert_eq!(
        s.get("color"),
        Some("teal"),
        "--foo ≠ --Foo : la référence minuscule ne la voit pas"
    );
}

#[test]
fn var_custom_casse_mixte_dans_style_inline() {
    // Le cas github `.lp-IntroVisuals-spacer` : --* déclarées
    // en `style=` avec casse mixte + digits.
    let f = fixture();
    let mut e = StyleEngine::new();
    let mut dom = f.dom;
    dom.set_attribute(
        f.p,
        "style",
        "--Spacer-size:64px;--Spacer-size768:600px;--Spacer-size1012:660px",
    )
    .unwrap();
    e.add_css(
        "p { height: var(--Spacer-size1280, var(--Spacer-size1012, \
         var(--Spacer-size768, var(--Spacer-size, 156px)))) }",
        Origin::Author,
    );
    assert_eq!(
        e.compute(&dom, f.p, &env()).get("height"),
        Some("660px"),
        "--Spacer-size1280 absente → --Spacer-size1012 (style=)"
    );
}

#[test]
fn var_custom_casse_mixte_heritage_et_lecture() {
    let f = fixture();
    let mut e = StyleEngine::new();
    // --Foo sur div hérite chez p (les --* héritent) ;
    // get() lit à la casse exacte.
    e.add_css("div { --Mixed: 9px }", Origin::Author);
    let s = e.compute(&f.dom, f.p, &env());
    assert_eq!(s.get("--Mixed"), Some("9px"), "héritée à la casse");
    assert_eq!(s.get("--mixed"), None, "la clé minuscule n'existe pas");
    // Et var() hérité : width lit --Mixed via la table parent.
    e.add_css("p { width: var(--Mixed) }", Origin::Author);
    assert_eq!(e.compute(&f.dom, f.p, &env()).get("width"), Some("9px"));
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

/// Vol 2026-10-26 — `[popover]` fermé = `display:none` (UA
/// HTML Rendering §15.3.1) : les tool-tips/mega-menus
/// `popover="manual"` de github étaient peints dépliés.
/// `:popover-open` est toujours faux tant que `showPopover()`
/// n'existe pas → tout `[popover]` est masqué.
#[test]
fn ua_popover_ferme_masque() {
    let f = fixture();
    let e = StyleEngine::new();
    let mut dom = f.dom;
    let tip = el(&mut dom, f.body, "tool-tip", &[("popover", "manual")]);
    assert_eq!(
        e.compute(&dom, tip, &env()).get("display"),
        Some("none"),
        "popover fermé : UA masque"
    );
    // Sans attribut popover : élément normal, règle inerte.
    let plain = el(&mut dom, f.body, "div", &[]);
    assert_ne!(
        e.compute(&dom, plain, &env()).get("display"),
        Some("none"),
        "élément ordinaire non affecté"
    );
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

/// `cols`/`rows`/`size` → dimensions en caractères (HTML
/// Rendering, présentational hints) — trigger terrain : le
/// `<textarea rows=6 cols=60>` de la page commentaire HN
/// rendu ~135×64 px génériques au lieu de ~480×115 px.
#[test]
fn hint_cols_rows_size_en_caracteres() {
    let f = fixture();
    let e = StyleEngine::new();
    let mut dom = f.dom;
    let ta = el(
        &mut dom,
        f.body,
        "textarea",
        &[("rows", "6"), ("cols", "60")],
    );
    let s = e.compute(&dom, ta, &env());
    assert_eq!(s.get("width"), Some("60ch"), "cols=60 → width:60ch");
    assert_eq!(s.get("height"), Some("7.2em"), "rows=6 → height ≈ 6 lignes");
    // Sans attribut : les défauts ex-UA (10em/4em) sont émis
    // en hint — comportement visuel par défaut inchangé.
    let ta2 = el(&mut dom, f.body, "textarea", &[]);
    let s2 = e.compute(&dom, ta2, &env());
    assert_eq!(s2.get("width"), Some("10em"));
    assert_eq!(s2.get("height"), Some("4em"));
    // `size` → ch ; input sans type = texte par défaut.
    let inp = el(&mut dom, f.body, "input", &[("size", "30")]);
    assert_eq!(e.compute(&dom, inp, &env()).get("width"), Some("30ch"));
    // `size` sur checkbox : la règle UA 13px (vraie règle)
    // prime le hint — présentational hints = rang auteur 0.
    let cb = el(
        &mut dom,
        f.body,
        "input",
        &[("type", "checkbox"), ("size", "40")],
    );
    assert_eq!(e.compute(&dom, cb, &env()).get("width"), Some("13px"));
    // La CSS auteur écrase le hint.
    let mut e2 = StyleEngine::new();
    e2.add_css("textarea { width: 500px }", Origin::Author);
    let ta3 = el(&mut dom, f.body, "textarea", &[("cols", "60")]);
    assert_eq!(e2.compute(&dom, ta3, &env()).get("width"), Some("500px"));
}

// ── R-68 : borne MAX_TOKENS vs feuilles concaténées réelles ──

#[test]
fn blob_geant_regle_apres_borne() {
    // Régression R-68 : les feuilles liées arrivent concaténées
    // en un seul `add_css` (session) — un corpus >256 K tokens
    // (github.com ≈ 384 K) perdait silencieusement sa fin :
    // la dernière feuille (et ses règles de masquage) était
    // tronquée → éléments non masqués peints. La règle cible
    // DOIT survivre au-delà de l'ancienne borne.
    let mut css = String::new();
    // `.a{x:y}` = 7 tokens → 45 000 règles ≈ 315 K tokens.
    for _ in 0..45_000 {
        css.push_str(".a{x:y}");
    }
    css.push_str(".probe{visibility:hidden}");
    let mut e = StyleEngine::new();
    e.add_css(&css, Origin::Author);
    let mut dom = Dom::new();
    let doc = dom.document();
    let b = dom.create_element("body").unwrap();
    dom.append_child(doc, b).unwrap();
    let d = dom.create_element("div").unwrap();
    dom.set_attribute(d, "class", "probe").unwrap();
    dom.append_child(b, d).unwrap();
    assert_eq!(e.compute(&dom, d, &env()).get("visibility"), Some("hidden"));
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

#[test]
fn font_size_rem_contre_racine_calculee() {
    // CSS Values §5.2 : `rem` = font-size **calculée** de
    // :root — `html { font-size:62.5% }` (10 px) fait
    // 1rem = 10px, pas 16. Motif terrain (dispatch phase
    // 23/10, « métriques verticales généreuses ») : le portail
    // Wikipedia compose tout en rem → métriques gonflées
    // ×1.6 quand la base fixe servait de référence.
    let mut dom = Dom::new();
    let doc = dom.document();
    let html = dom.create_element("html").unwrap();
    let body = dom.create_element("body").unwrap();
    let p = dom.create_element("p").unwrap();
    dom.append_child(doc, html).unwrap();
    dom.append_child(html, body).unwrap();
    dom.append_child(body, p).unwrap();
    let mut e = StyleEngine::new();
    e.add_css(
        "html { font-size: 62.5% } body { font-size: 1.4rem } p { font-size: 1rem }",
        Origin::Author,
    );
    assert_eq!(e.compute(&dom, html, &env()).get("font-size"), Some("10px"));
    assert_eq!(
        e.compute(&dom, body, &env()).get("font-size"),
        Some("14px"),
        "rem contre la racine calculée (10 px), pas l'initiale"
    );
    assert_eq!(e.compute(&dom, p, &env()).get("font-size"), Some("10px"));
    // Sur `font-size` de la racine elle-même, `rem` vaut
    // l'initiale (16) — :root n'a pas de racine au-dessus.
    let mut e2 = StyleEngine::new();
    e2.add_css("html { font-size: 2rem }", Origin::Author);
    assert_eq!(
        e2.compute(&dom, html, &env()).get("font-size"),
        Some("32px")
    );
    // Même résolution dans le parcours d'arbre complet.
    let styles = e.compute_tree(&dom, doc, &env());
    assert_eq!(styles.get(p).and_then(|s| s.get("font-size")), Some("10px"));
    assert_eq!(
        styles.get(body).and_then(|s| s.get("font-size")),
        Some("14px")
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
fn background_multicouche_fusionne_les_couches() {
    let f = fixture();
    let mut e = StyleEngine::new();
    // R-94 : `url(image), linear-gradient(décor)` = deux
    // couches valides — avant, le second <bg-image> faisait
    // tomber la déclaration entière (votearrow de HN : la
    // flèche `background: url("triangle.svg"),
    // linear-gradient(…)` ne peignait jamais). R-107 : les
    // longhands sortent **jointes par couche** (`, `-séparées)
    // — la peinture les consomme par index de couche.
    e.add_css(
        "p { background: url(\"tri.svg\"), linear-gradient(red, blue) no-repeat }",
        Origin::Author,
    );
    let s = e.compute(&f.dom, f.p, &env());
    assert_eq!(
        s.get("background-image"),
        Some("url(\"tri.svg\"), linear-gradient(red, blue)")
    );
    // Couche 1 = repeat (absent → initiale), couche 2 =
    // no-repeat.
    assert_eq!(s.get("background-repeat"), Some("repeat, no-repeat"));
}

#[test]
fn background_multicouche_invalide_drope() {
    let f = fixture();
    let mut e = StyleEngine::new();
    // Une couche invalide invalide la déclaration entière
    // (spec — même verdict que mono-couche).
    e.add_css(
        "p { background: yellow } p { background: url(\"a.png\"), bogus token }",
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

// ── R-57 : `--*` partagées — sémantique préservée ────────────

/// L'enfant sans déclaration `--*` hérite la table du parent ;
/// sa propre déclaration masque l'héritée du même nom.
#[test]
fn vars_heritees_et_masquees() {
    let f = fixture();
    let mut e = StyleEngine::new();
    e.add_css(
        "div { --accent: red; --gap: 4px } p { --accent: blue; color: var(--accent) }",
        Origin::Author,
    );
    let map = e.compute_tree(&f.dom, f.dom.document(), &env());
    let p = map.get(f.p).expect("style p");
    // Héritage : `--gap` remonte, `--accent` masquée par la
    // déclaration propre.
    assert_eq!(p.get("--gap"), Some("4px"), "héritée du parent");
    assert_eq!(p.get("--accent"), Some("blue"), "propre > héritée");
    // `var()` résolue contre la valeur effective (masquée).
    assert_eq!(p.get("color"), Some("blue"));
    // `iter()` expose les `--*` comme avant.
    let names: Vec<&str> = p.iter().map(|(k, _)| k).collect();
    assert!(names.contains(&"--accent") && names.contains(&"--gap"));
}

/// `set("--x")` sur l'enfant ne fuit pas dans le parent —
/// clone-on-write de la table partagée.
#[test]
fn vars_set_isole_le_parent() {
    let f = fixture();
    let mut e = StyleEngine::new();
    e.add_css("div { --a: 1px }", Origin::Author);
    let map = e.compute_tree(&f.dom, f.dom.document(), &env());
    let mut child = map.get(f.p).expect("style p").clone();
    child.set("--a", "9px");
    child.set("--b", "2px");
    assert_eq!(child.get("--a"), Some("9px"));
    assert_eq!(child.get("--b"), Some("2px"));
    let parent = map.get(f.div).expect("style div");
    assert_eq!(parent.get("--a"), Some("1px"), "parent intact");
    assert_eq!(parent.get("--b"), None, "pas de fuite");
}

/// Trois générations : la petite-fille hérite la table effective
/// de la mère (propres de la mère comprises).
#[test]
fn vars_propagent_les_propres_de_la_mere() {
    let mut dom = Dom::new();
    let doc = dom.document();
    let a = dom.create_element("div").unwrap();
    dom.append_child(doc, a).unwrap();
    let b = dom.create_element("div").unwrap();
    dom.append_child(a, b).unwrap();
    let c = dom.create_element("p").unwrap();
    dom.append_child(b, c).unwrap();
    let mut e = StyleEngine::new();
    e.add_css(
        "div { --theme: dark } div div { --pad: 2px }",
        Origin::Author,
    );
    let map = e.compute_tree(&dom, doc, &env());
    let gc = map.get(c).expect("style p");
    assert_eq!(gc.get("--theme"), Some("dark"), "transmis via la mère");
    assert_eq!(gc.get("--pad"), Some("2px"), "propre de la mère");
}

// ── R-63 : cascade scopée au sous-arbre ──────────────────────

#[test]
fn scoped_equivaut_a_la_map_globale_sur_le_sous_arbre() {
    // compute_scoped(dom, div, env, parent=style(body)) doit
    // produire les mêmes styles que compute_tree pour div et
    // sa descendance — sélecteurs matchent le DOM entier.
    let f = fixture();
    let mut e = StyleEngine::new();
    e.add_css(
        "div#a { color: red } div p { font-weight: bold } \
         body div ~ p { color: blue } p.b { margin-top: 7px }",
        Origin::Author,
    );
    let global = e.compute_tree(&f.dom, f.dom.document(), &env());
    let parent = global.get(f.body);
    let scoped = e.compute_scoped(&f.dom, f.div, &env(), parent);
    for node in [f.div, f.p] {
        let (a, b) = (global.get(node), scoped.get(node));
        assert!(a.is_some() && b.is_some());
        for prop in ["color", "font-weight", "margin-top", "display"] {
            assert_eq!(
                a.unwrap().get(prop),
                b.unwrap().get(prop),
                "{prop} diverge sur {node:?}"
            );
        }
    }
    // Le frère hors sous-arbre n'y figure pas.
    assert!(scoped.get(f.body).is_none());
}

#[test]
fn scoped_herite_custom_properties_et_selecteurs_contextuels() {
    // Héritage --* + sélecteur ancêtre hors sous-arbre : les
    // deux doivent traverser la frontière du scope.
    let mut dom = Dom::new();
    let doc = dom.document();
    let outer = dom.create_element("div").unwrap();
    dom.append_child(doc, outer).unwrap();
    dom.set_attribute(outer, "class", "ctx").unwrap();
    let mid = dom.create_element("section").unwrap();
    dom.append_child(outer, mid).unwrap();
    let leaf = dom.create_element("p").unwrap();
    dom.append_child(mid, leaf).unwrap();
    let mut e = StyleEngine::new();
    e.add_css(
        ".ctx { --theme: dark; color: green } .ctx p { color: red } \
         p { background: var(--theme) }",
        Origin::Author,
    );
    let global = e.compute_tree(&dom, doc, &env());
    let scoped = e.compute_scoped(&dom, mid, &env(), global.get(outer));
    let leaf_style = scoped.get(leaf).expect("style p scopé");
    assert_eq!(
        leaf_style.get("color"),
        Some("red"),
        "sélecteur ancêtre hors scope"
    );
    assert_eq!(
        leaf_style.get("background"),
        Some("dark"),
        "--* héritée via le parent semé"
    );
}

#[test]
fn apply_scoped_remplace_et_purge_les_pseudos() {
    let mut dom = Dom::new();
    let doc = dom.document();
    let div = dom.create_element("div").unwrap();
    dom.append_child(doc, div).unwrap();
    let p = dom.create_element("p").unwrap();
    dom.append_child(div, p).unwrap();
    let mut e = StyleEngine::new();
    e.add_css(
        "div::before { content: 'x' } div { color: red } p { color: blue }",
        Origin::Author,
    );
    let mut map = e.compute_tree(&dom, doc, &env());
    assert!(map
        .pseudo(div, explorer_css::PseudoElement::Before)
        .is_some());
    // La feuille perd ::before → la map scopée fait autorité.
    let mut e2 = StyleEngine::new();
    e2.add_css("div { color: red } p { color: blue }", Origin::Author);
    let scoped = e2.compute_scoped(&dom, div, &env(), map.get(doc));
    map.apply_scoped(scoped);
    assert!(
        map.pseudo(div, explorer_css::PseudoElement::Before)
            .is_none(),
        "pseudo disparu purgé"
    );
    assert_eq!(map.get(p).unwrap().get("color"), Some("blue"));
    // remove : nœud détaché.
    map.remove(p);
    assert!(map.get(p).is_none());
}

// ── R-64 : `(not (f:v))` facteur nié en parens MQ4 ───────────

#[test]
fn media_not_parens_lobste_dark_normal() {
    // Repro terrain Étape 97 : lobste.rs chaîne
    // `(prefers-color-scheme: dark) and
    //  (not (prefers-contrast: more))` sur `dark-normal.css`
    // — l'intérieur `not (…)` devenait un nom inconnu → la
    // query entière échouait même en env Dark.
    let f = fixture();
    let mut e = StyleEngine::new();
    e.add_css(
        "@media (prefers-color-scheme: dark) and (not (prefers-contrast: more)) \
         { div { color: red } }",
        Origin::Author,
    );
    let dark = StyleEnv {
        color_scheme: explorer_css::ColorScheme::Dark,
        ..env()
    };
    assert_eq!(
        e.compute(&f.dom, f.div, &dark).get("color"),
        Some("red"),
        "env dark → dark-normal s'applique"
    );
    assert_eq!(
        e.compute(&f.dom, f.div, &env()).get("color"),
        None,
        "env light → query sautée"
    );
}

#[test]
fn media_not_parens_formes() {
    let f = fixture();
    // Feature connue niée : `(not (min-width:400px))` à
    // 1024 px de large (env()) → faux → rien.
    let mut e = StyleEngine::new();
    e.add_css(
        "@media (not (min-width:400px)) { div { color: red } } \
         @media (not (min-width:2000px)) { div { margin-top: 9px } } \
         @media (not (400px <= width <= 700px)) { div { display: none } }",
        Origin::Author,
    );
    let s = e.compute(&f.dom, f.div, &env());
    assert_eq!(s.get("color"), None, "min-width:400 nié à 1024");
    assert_eq!(
        s.get("margin-top"),
        Some("9px"),
        "min-width:2000 nié à 1024 → vrai"
    );
    // Corps non résoluble (double borne) → « feature
    // absente » → nié vrai (tolérance documentée).
    assert_eq!(s.get("display"), Some("none"), "double borne niée → vrai");
}

// ── R-83 : index, filtre d'ancêtres Bloom, mémo de matching ──
// Le filtre ne produit jamais de faux négatif : chaque kind
// de clé (tag/id/class/attr) reste trouvable quand l'ancêtre
// la porte, les combinators frères et les rangs `:nth-*` sont
// inchangés, et les sélecteurs impossibles restent absents.

#[test]
fn r83_filtre_ancetres_sans_faux_negatif() {
    // body > div#wrap > section.items[data-x] > [span, p, p2]
    // et `em` dans `p` — clés variées sur les ancêtres.
    let mut dom = Dom::new();
    let doc = dom.document();
    let body = dom.create_element("body").unwrap();
    dom.append_child(doc, body).unwrap();
    let div = dom.create_element("div").unwrap();
    dom.set_attribute(div, "id", "wrap").unwrap();
    dom.append_child(body, div).unwrap();
    let sec = dom.create_element("section").unwrap();
    dom.set_attribute(sec, "class", "items").unwrap();
    dom.set_attribute(sec, "data-x", "1").unwrap();
    dom.append_child(div, sec).unwrap();
    let span = dom.create_element("span").unwrap();
    dom.append_child(sec, span).unwrap();
    let p = dom.create_element("p").unwrap();
    dom.append_child(sec, p).unwrap();
    let em = dom.create_element("em").unwrap();
    dom.append_child(p, em).unwrap();
    let p2 = dom.create_element("p").unwrap();
    dom.append_child(sec, p2).unwrap();

    let mut e = StyleEngine::new();
    e.add_css(
        "article p { color: red } \
         div p { color: green } \
         #wrap p { font-weight: bold } \
         .items p { text-align: center } \
         [data-x] p { width: 1px } \
         [data-y] p { z-index: 7 } \
         span + p { border-top-width: 0 } \
         p:nth-child(2) { margin-top: 2px } \
         section p em { top: 0px }",
        Origin::Author,
    );
    let map = e.compute_tree(&dom, doc, &env());
    let sp = map.get(p).unwrap();
    // Tag ancêtre : `article` élagué sans casser `div`.
    assert_eq!(sp.get("color"), Some("green"));
    // Id / classe / nom d'attribut ancêtres.
    assert_eq!(sp.get("font-weight"), Some("bold"));
    assert_eq!(sp.get("text-align"), Some("center"));
    assert_eq!(sp.get("width"), Some("1px"));
    // Clé absente partout → jamais posée (élagage correct).
    assert_eq!(sp.get("z-index"), None);
    // Frère précédent immédiat — non filtré (ce n'est pas
    // un ancêtre).
    assert_eq!(sp.get("border-top-width"), Some("0"));
    // Rang fraternel mémoïsé — `p` est le 2ᵉ élément enfant
    // (`p2` garde la marge UA `1em`, pas le `2px` du nth).
    assert_eq!(sp.get("margin-top"), Some("2px"));
    assert_eq!(map.get(p2).unwrap().get("margin-top"), Some("1em"));
    // Chaîne à trois composés.
    assert_eq!(map.get(em).unwrap().get("top"), Some("0px"));
}

#[test]
fn r83_saut_des_calculs_pseudo() {
    // Sans règle `::before`/`::after`, le calcul pseudo est
    // sauté : pas de pseudo-style produit — et une feuille
    // qui en a un le produit toujours.
    let f = fixture();
    let mut e = StyleEngine::new();
    e.add_css("p { color: green }", Origin::Author);
    let map = e.compute_tree(&f.dom, f.dom.document(), &env());
    assert!(map
        .pseudo(f.p, explorer_css::PseudoElement::Before)
        .is_none());
    e.add_css("p::before { content: 'x' }", Origin::Author);
    let map = e.compute_tree(&f.dom, f.dom.document(), &env());
    assert_eq!(
        map.pseudo(f.p, explorer_css::PseudoElement::Before)
            .and_then(|s| s.get("content")),
        Some("\"x\"")
    );
}

// ── R-96 — scoping shadow DOM ─────────────────────────────────

/// DOM : `body > section.wrap > x-foo.dark#h` ; l'hôte `x-foo`
/// porte une racine shadow ouverte contenant `p.inner` (avec
/// `part=label` sur `span.slot`) :
///   shadowroot: <p class="inner"> / <span part="label">
struct ShadowFixture {
    dom: Dom,
    host: NodeId,
    root: NodeId,
    inner: NodeId,
    label: NodeId,
}

fn shadow_fixture() -> ShadowFixture {
    let mut dom = Dom::new();
    let doc = dom.document();
    let body = dom.create_element("body").unwrap();
    dom.append_child(doc, body).unwrap();
    let wrap = dom.create_element("section").unwrap();
    dom.set_attribute(wrap, "class", "wrap").unwrap();
    dom.append_child(body, wrap).unwrap();
    let host = dom.create_element("x-foo").unwrap();
    dom.set_attribute(host, "class", "dark").unwrap();
    dom.set_attribute(host, "id", "h").unwrap();
    dom.append_child(wrap, host).unwrap();
    let root = dom.create_fragment().unwrap();
    dom.attach_shadow_root(host, root, true);
    let inner = dom.create_element("p").unwrap();
    dom.set_attribute(inner, "class", "inner").unwrap();
    dom.append_child(root, inner).unwrap();
    let label = dom.create_element("span").unwrap();
    dom.set_attribute(label, "part", "label").unwrap();
    dom.append_child(root, label).unwrap();
    ShadowFixture {
        dom,
        host,
        root,
        inner,
        label,
    }
}

#[test]
fn feuille_document_ne_rentre_pas_en_shadow() {
    let f = shadow_fixture();
    let mut e = StyleEngine::new();
    e.add_css("p { color: red }", Origin::Author);
    let map = e.compute_tree(&f.dom, f.dom.document(), &env());
    // `.inner` est calculé (flat tree) mais la règle document
    // `p` ne l'atteint pas — confinement.
    assert_ne!(map.get(f.inner).and_then(|s| s.get("color")), Some("red"));
}

#[test]
fn feuille_scopee_atteint_son_arbre() {
    let f = shadow_fixture();
    let mut e = StyleEngine::new();
    e.add_scoped_css("p { color: green }", Origin::Author, f.root);
    let map = e.compute_tree(&f.dom, f.dom.document(), &env());
    assert_eq!(map.get(f.inner).and_then(|s| s.get("color")), Some("green"));
}

#[test]
fn feuille_scopee_ne_fuit_pas_vers_document() {
    // Deux racines : le scope A n'atteint pas l'arbre B ni le
    // document.
    let mut f = shadow_fixture();
    let host2 = f.dom.create_element("x-bar").unwrap();
    let doc = f.dom.document();
    f.dom.append_child(doc, host2).unwrap();
    let root2 = f.dom.create_fragment().unwrap();
    f.dom.attach_shadow_root(host2, root2, true);
    let inner2 = f.dom.create_element("p").unwrap();
    f.dom.append_child(root2, inner2).unwrap();
    let mut e = StyleEngine::new();
    e.add_scoped_css("p { color: green }", Origin::Author, f.root);
    let map = e.compute_tree(&f.dom, f.dom.document(), &env());
    assert_eq!(map.get(f.inner).and_then(|s| s.get("color")), Some("green"));
    assert_ne!(map.get(inner2).and_then(|s| s.get("color")), Some("green"));
}

#[test]
fn host_sujet_matche_lhote() {
    let f = shadow_fixture();
    let mut e = StyleEngine::new();
    e.add_scoped_css(":host { display: block }", Origin::Author, f.root);
    let map = e.compute_tree(&f.dom, f.dom.document(), &env());
    assert_eq!(
        map.get(f.host).and_then(|s| s.get("display")),
        Some("block")
    );
}

#[test]
fn host_fonctionnel_filtre_lhote() {
    let f = shadow_fixture();
    let mut e = StyleEngine::new();
    // L'hôte a bien `class=dark` → match.
    e.add_scoped_css(":host(.dark) { color: navy }", Origin::Author, f.root);
    // `.light` absent → ne doit pas matcher.
    e.add_scoped_css(":host(.light) { color: pink }", Origin::Author, f.root);
    let map = e.compute_tree(&f.dom, f.dom.document(), &env());
    assert_eq!(map.get(f.host).and_then(|s| s.get("color")), Some("navy"));
}

#[test]
fn host_context_remonte_dans_lenglobant() {
    let f = shadow_fixture();
    let mut e = StyleEngine::new();
    e.add_scoped_css(
        ":host-context(.wrap) { display: grid }",
        Origin::Author,
        f.root,
    );
    e.add_scoped_css(
        ":host-context(.absent) { display: flex }",
        Origin::Author,
        f.root,
    );
    let map = e.compute_tree(&f.dom, f.dom.document(), &env());
    assert_eq!(map.get(f.host).and_then(|s| s.get("display")), Some("grid"));
}

#[test]
fn part_depuis_document_atteint_lelement_expose() {
    let f = shadow_fixture();
    let mut e = StyleEngine::new();
    e.add_css("x-foo::part(label) { color: teal }", Origin::Author);
    let map = e.compute_tree(&f.dom, f.dom.document(), &env());
    assert_eq!(map.get(f.label).and_then(|s| s.get("color")), Some("teal"));
}

#[test]
fn part_sans_attribut_ne_matche_pas() {
    let f = shadow_fixture();
    let mut e = StyleEngine::new();
    // `.inner` n'a pas `part=label`.
    e.add_css("x-foo::part(label) { color: teal }", Origin::Author);
    let map = e.compute_tree(&f.dom, f.dom.document(), &env());
    assert_ne!(map.get(f.inner).and_then(|s| s.get("color")), Some("teal"));
}

#[test]
fn heritage_franchit_la_frontiere() {
    let f = shadow_fixture();
    let mut e = StyleEngine::new();
    // `color` et `--var` héritent de l'hôte vers l'arbre
    // shadow (pas de règle ciblée sur `.inner`).
    e.add_css("x-foo { color: purple; --ton: violet }", Origin::Author);
    let map = e.compute_tree(&f.dom, f.dom.document(), &env());
    assert_eq!(
        map.get(f.inner).and_then(|s| s.get("color")),
        Some("purple")
    );
    // `--*` traverse la frontière : la règle consommatrice
    // vit dans la feuille scopée, la variable sur l'hôte
    // (feuille document) — héritée via le style parent.
    let mut e2 = StyleEngine::new();
    e2.add_css("x-foo { --ton: violet }", Origin::Author);
    e2.add_scoped_css("p { color: var(--ton) }", Origin::Author, f.root);
    let map2 = e2.compute_tree(&f.dom, f.dom.document(), &env());
    assert_eq!(
        map2.get(f.inner).and_then(|s| s.get("color")),
        Some("violet")
    );
}

#[test]
fn descendant_depuis_host_franchit() {
    // `div :host p` — dans la feuille scopée : `div` marche
    // les ancêtres de l'hôte dans l'arbre englobant (section
    // n'est pas div → faux ici) ; `.wrap :host` doit matcher.
    let f = shadow_fixture();
    let mut e = StyleEngine::new();
    e.add_scoped_css("div :host { display: none }", Origin::Author, f.root);
    e.add_scoped_css(".wrap :host { display: block }", Origin::Author, f.root);
    let map = e.compute_tree(&f.dom, f.dom.document(), &env());
    assert_eq!(
        map.get(f.host).and_then(|s| s.get("display")),
        Some("block")
    );
}

// ── R-97 — distribution `<slot>` + `::slotted` ───────────

/// DOM : `body > x-foo` (hôte) ; light DOM : `<em slot="t">`
/// (assigné nommé), `<i>` (défaut), `<u slot="z">` (orphelin).
/// Racine shadow : `<slot name="t">` puis `<slot>` (défaut,
/// avec un `<b>` de repli).
struct SlotFixture {
    dom: Dom,
    root: NodeId,
    em: NodeId,
    i: NodeId,
    u: NodeId,
    fb: NodeId,
}

fn slot_fixture() -> SlotFixture {
    let mut dom = Dom::new();
    let doc = dom.document();
    let body = dom.create_element("body").unwrap();
    dom.append_child(doc, body).unwrap();
    let host = dom.create_element("x-foo").unwrap();
    dom.append_child(body, host).unwrap();
    let em = dom.create_element("em").unwrap();
    dom.set_attribute(em, "slot", "t").unwrap();
    let i = dom.create_element("i").unwrap();
    let u = dom.create_element("u").unwrap();
    dom.set_attribute(u, "slot", "z").unwrap();
    for c in [em, i, u] {
        dom.append_child(host, c).unwrap();
    }
    let root = dom.create_fragment().unwrap();
    let slot_t = dom.create_element("slot").unwrap();
    dom.set_attribute(slot_t, "name", "t").unwrap();
    let slot_def = dom.create_element("slot").unwrap();
    let fb = dom.create_element("b").unwrap();
    dom.append_child(slot_def, fb).unwrap();
    dom.append_child(root, slot_t).unwrap();
    dom.append_child(root, slot_def).unwrap();
    dom.attach_shadow_root(host, root, true);
    SlotFixture {
        dom,
        root,
        em,
        i,
        u,
        fb,
    }
}

#[test]
fn slotted_atteint_lenfant_assigne() {
    let f = slot_fixture();
    let mut e = StyleEngine::new();
    // Feuille scopée : `::slotted` franchit en sortie vers
    // les enfants light assignés à ses slots.
    e.add_scoped_css("::slotted(em) { color: olive }", Origin::Author, f.root);
    let map = e.compute_tree(&f.dom, f.dom.document(), &env());
    assert_eq!(map.get(f.em).and_then(|s| s.get("color")), Some("olive"));
    // `<i>` n'est pas `em` → pas atteint.
    assert_ne!(map.get(f.i).and_then(|s| s.get("color")), Some("olive"));
}

#[test]
fn slotted_non_assigne_intouche() {
    let f = slot_fixture();
    let mut e = StyleEngine::new();
    // `u` a `slot="z"` — aucun `<slot name="z">` → non
    // distribué, `::slotted(*)` ne l'atteint pas.
    e.add_scoped_css("::slotted(*) { color: olive }", Origin::Author, f.root);
    let map = e.compute_tree(&f.dom, f.dom.document(), &env());
    assert_eq!(map.get(f.em).and_then(|s| s.get("color")), Some("olive"));
    assert_eq!(map.get(f.i).and_then(|s| s.get("color")), Some("olive"));
    assert_ne!(map.get(f.u).and_then(|s| s.get("color")), Some("olive"));
}

#[test]
fn slotted_qualifie_le_slot() {
    let f = slot_fixture();
    let mut e = StyleEngine::new();
    // Les simples hors `::slotted` qualifient le SLOT —
    // `slot[name=t]` n'assigne que `em`.
    e.add_scoped_css(
        "slot[name=t]::slotted(*) { color: olive }",
        Origin::Author,
        f.root,
    );
    e.add_scoped_css(
        "slot[name=x]::slotted(*) { color: pink }",
        Origin::Author,
        f.root,
    );
    let map = e.compute_tree(&f.dom, f.dom.document(), &env());
    assert_eq!(map.get(f.em).and_then(|s| s.get("color")), Some("olive"));
    // `<i>` est au slot défaut (sans name) — ni olive ni pink.
    assert_ne!(map.get(f.i).and_then(|s| s.get("color")), Some("olive"));
    assert_ne!(map.get(f.i).and_then(|s| s.get("color")), Some("pink"));
}

#[test]
fn slotted_document_ne_franchit_pas() {
    let f = slot_fixture();
    let mut e = StyleEngine::new();
    // `::slotted` en feuille document : le franchissement
    // exige une feuille scopée — rien n'est atteint.
    e.add_css("::slotted(*) { color: olive }", Origin::Author);
    let map = e.compute_tree(&f.dom, f.dom.document(), &env());
    assert_ne!(map.get(f.em).and_then(|s| s.get("color")), Some("olive"));
    assert_ne!(map.get(f.i).and_then(|s| s.get("color")), Some("olive"));
}

#[test]
fn heritage_assigne_depuis_le_slot() {
    let f = slot_fixture();
    let mut e = StyleEngine::new();
    // Flat tree : l'enfant assigné hérite du style de son
    // `<slot>` (qui hérite lui-même de l'hôte) — pas de
    // l'hôte directement.
    e.add_scoped_css("slot { color: teal }", Origin::Author, f.root);
    e.add_css("x-foo { color: purple }", Origin::Author);
    let map = e.compute_tree(&f.dom, f.dom.document(), &env());
    assert_eq!(
        map.get(f.i).and_then(|s| s.get("color")),
        Some("teal"),
        "assigné → hérite du slot, pas de l'hôte"
    );
    assert_eq!(map.get(f.em).and_then(|s| s.get("color")), Some("teal"));
    // `u` n'est pas assigné → hérite de l'hôte (parent DOM).
    assert_eq!(map.get(f.u).and_then(|s| s.get("color")), Some("purple"));
    // Le contenu de repli vit DANS l'arbre shadow : enfant
    // ordinaire du slot → hérite du slot aussi.
    assert_eq!(map.get(f.fb).and_then(|s| s.get("color")), Some("teal"));
}

// ── R-105 : light-dark() résolu à la valeur calculée ────────

#[test]
fn light_dark_suit_le_schema_use() {
    // css-color-adjust-1 §2.4 : la branche dépend du schéma
    // usé = `color-scheme` de l'élément ∩ préférence env.
    let mk = |env: explorer_css::ColorScheme, css: &str| {
        let mut dom = Dom::new();
        let doc = dom.document();
        let div = dom.create_element("div").unwrap();
        dom.append_child(doc, div).unwrap();
        let mut eng = StyleEngine::new();
        eng.add_css(css, Origin::Author);
        let env = StyleEnv {
            color_scheme: env,
            ..StyleEnv::default()
        };
        let styles = eng.compute_tree(&dom, div, &env);
        styles.get(div).unwrap().get("color").map(str::to_owned)
    };
    // Env clair → branche claire ; env sombre → sombre.
    assert_eq!(
        mk(
            explorer_css::ColorScheme::Light,
            "div{color:light-dark(white, black)}"
        ),
        Some("white".into())
    );
    assert_eq!(
        mk(
            explorer_css::ColorScheme::Dark,
            "div{color:light-dark(white, black)}"
        ),
        Some("black".into())
    );
    // `color-scheme: only light` sur l'élément → clair même
    // sous env sombre (opt-out par construction).
    assert_eq!(
        mk(
            explorer_css::ColorScheme::Dark,
            "div{color-scheme:only light;color:light-dark(white, black)}"
        ),
        Some("white".into())
    );
    // `color-scheme: dark` sous env clair → sombre.
    assert_eq!(
        mk(
            explorer_css::ColorScheme::Light,
            "div{color-scheme:dark;color:light-dark(white, black)}"
        ),
        Some("black".into())
    );
    // Niché dans une valeur composite (var()+fond) : résolu.
    assert_eq!(
        mk(
            explorer_css::ColorScheme::Dark,
            "div{--c:light-dark(#fff,#000);color:var(--c)}"
        ),
        Some("#000".into())
    );
}

#[test]
fn part_chaine_et_exportparts() {
    // R-110 : `::part` chaîné (remontée par frontière) +
    // `exportparts` (alias via l'hôte).
    let mut dom = Dom::new();
    let doc = dom.document();
    let outer = dom.create_element("x-outer").unwrap();
    dom.append_child(doc, outer).unwrap();
    let root_o = dom.create_fragment().unwrap();
    dom.attach_shadow_root(outer, root_o, true);
    let inner = dom.create_element("x-inner").unwrap();
    dom.set_attribute(inner, "part", "mid").unwrap();
    dom.set_attribute(inner, "exportparts", "deep:alias")
        .unwrap();
    dom.append_child(root_o, inner).unwrap();
    let root_i = dom.create_fragment().unwrap();
    dom.attach_shadow_root(inner, root_i, true);
    let deep = dom.create_element("span").unwrap();
    dom.set_attribute(deep, "part", "deep").unwrap();
    dom.append_child(root_i, deep).unwrap();

    let sel = |s: &str| parse_selector_list(s).unwrap().into_iter().next().unwrap();
    // 1. part direct : x-inner::part(deep) atteint deep.
    assert!(matches(&dom, deep, &sel("x-inner::part(deep)")));
    // 2. chaîne : x-outer::part(mid)::part(deep) — mid sur
    //    l'hôte interne, deep sur le sujet.
    assert!(matches(&dom, deep, &sel("x-outer::part(mid)::part(deep)")));
    // 3. exportparts : l'alias est visible — x-inner::part(alias).
    assert!(matches(&dom, deep, &sel("x-inner::part(alias)")));
    // 4. négatif : l'alias n'existe pas sans exportparts,
    //    mid n'est pas sur deep.
    assert!(!matches(&dom, deep, &sel("x-inner::part(mid)")));
    assert!(!matches(&dom, deep, &sel("x-outer::part(deep)")));
}

#[test]
fn important_frontiere_shadow_inverse() {
    // R-111 — css-scoping §6 : pour le normal, l'**interne**
    // (scoped) bat le document ; pour `!important` c'est
    // **l'inverse** — le document gagne. Conflit mesuré via
    // `::part` (le seul canal document → shadow).
    let f = shadow_fixture();
    // Normal : inner gagne.
    let mut e = StyleEngine::new();
    e.add_css("x-foo::part(label) { color: red }", Origin::Author);
    e.add_scoped_css("[part] { color: blue }", Origin::Author, f.root);
    let s = e.compute(&f.dom, f.label, &env());
    assert_eq!(s.get("color"), Some("blue"), "inner normal > outer");
    // !important : outer gagne.
    let mut e = StyleEngine::new();
    e.add_css(
        "x-foo::part(label) { color: red !important }",
        Origin::Author,
    );
    e.add_scoped_css("[part] { color: blue !important }", Origin::Author, f.root);
    let s = e.compute(&f.dom, f.label, &env());
    assert_eq!(s.get("color"), Some("red"), "outer !important > inner");
    // Mixte : inner important bat outer normal (rang domine).
    let mut e = StyleEngine::new();
    e.add_css("x-foo::part(label) { color: red }", Origin::Author);
    e.add_scoped_css("[part] { color: blue !important }", Origin::Author, f.root);
    let s = e.compute(&f.dom, f.label, &env());
    assert_eq!(
        s.get("color"),
        Some("blue"),
        "inner !important > outer normal"
    );
}

#[test]
fn ol_type_attribut_mappe_list_style() {
    // R-113 — `ol[type]` hint → `list-style-type` (HTML
    // §14.3.9) : `type=a` → lower-alpha, `type=I` →
    // upper-roman — datatracker IETF rendait decimal.
    let f = fixture();
    let mut dom = f.dom;
    let ol = dom.create_element("ol").unwrap();
    dom.set_attribute(ol, "type", "a").unwrap();
    dom.append_child(dom.document(), ol).unwrap();
    let li = dom.create_element("li").unwrap();
    dom.append_child(ol, li).unwrap();
    let e = StyleEngine::new();
    let s = e.compute(&dom, li, &env());
    assert_eq!(
        s.get("list-style-type"),
        Some("lower-alpha"),
        "type=a → alpha"
    );
    // `type` inconnu → pas de hint (repli calculé).
    let ol2 = dom.create_element("ol").unwrap();
    dom.set_attribute(ol2, "type", "z").unwrap();
    dom.append_child(dom.document(), ol2).unwrap();
    let li2 = dom.create_element("li").unwrap();
    dom.append_child(ol2, li2).unwrap();
    assert_ne!(
        e.compute(&dom, li2, &env()).get("list-style-type"),
        Some("decimal-z")
    );
}

#[test]
fn color_mix_resout_espaces() {
    // R-114 — `color-mix()` résolu à la valeur calculée
    // (css-color-5 §3 — ×1143 dans la feuille Tailwind).
    let f = fixture();
    let mut dom = f.dom;
    let div = dom.create_element("div").unwrap();
    dom.append_child(dom.document(), div).unwrap();
    let mut e = StyleEngine::new();
    e.add_css(
        "div{color:color-mix(in srgb, red 25%, blue)}",
        Origin::Author,
    );
    let s = e.compute(&dom, div, &env());
    // 25% red + 75% blue en srgb → (63.75,0,191.25) ≈ (64,0,191).
    assert_eq!(s.get("color"), Some("rgb(64,0,191)"));
    // oklab + pourcentage manquant → 50/50.
    let mut e = StyleEngine::new();
    e.add_css(
        "div{color:color-mix(in oklab, black, white)}",
        Origin::Author,
    );
    let s = e.compute(&dom, div, &env());
    assert!(s.get("color").is_some_and(|v| v.starts_with("rgb")));
    // var() imbriqué résolu avant.
    let mut e = StyleEngine::new();
    e.add_css(
        "div{--c:rgb(0,255,0);color:color-mix(in srgb, var(--c) 50%, black)}",
        Origin::Author,
    );
    assert_eq!(
        e.compute(&dom, div, &env()).get("color"),
        Some("rgb(0,128,0)")
    );
}

/// Vol 11 stripe : `min()`/`max()`/`clamp()` dans une
/// déclaration de longueur ne sont plus droppés par la
/// validation (`width:min(50%,300px)` retombait à `auto`).
#[test]
fn min_max_clamp_conserves_en_declaration() {
    let f = fixture();
    let mut e = StyleEngine::new();
    e.add_css(
        "div { width: min(50%, 300px); max-width: clamp(100px, 50%, 300px) }",
        Origin::Author,
    );
    let s = e.compute(&f.dom, f.div, &env());
    assert_eq!(s.get("width"), Some("min(50%, 300px)"));
    assert_eq!(s.get("max-width"), Some("clamp(100px, 50%, 300px)"));
}

#[test]
fn property_registre_inherits_et_initial() {
    // R-118 — @property (css-properties-values-api-1) :
    // inherits:false masque l'héritage, initial-value sert
    // de valeur calculée sur les éléments non déclarés.
    let f = fixture();
    let mut dom = f.dom;
    let parent = dom.create_element("div").unwrap();
    let child = dom.create_element("span").unwrap();
    dom.append_child(dom.document(), parent).unwrap();
    dom.append_child(parent, child).unwrap();
    let mut e = StyleEngine::new();
    e.add_css(
        "@property --brand { syntax: '<color>'; inherits: false; initial-value: rgb(255,0,0) }\
         div{--brand: blue; color: var(--brand)}\
         span{color: var(--brand)}",
        Origin::Author,
    );
    let ps = e.compute(&dom, parent, &env());
    let cs = e.compute(&dom, child, &env());
    // Parent : sa propre valeur (blue).
    assert_eq!(ps.get("color"), Some("blue"));
    // Enfant : inherits:false → pas de blue hérité ; initial
    // du registre sert (red).
    assert_eq!(cs.get("color"), Some("rgb(255,0,0)"));
}

#[test]
fn container_collecte_et_evalue() {
    // R-122 — `@container` : parse + collecte + évaluation
    // contre une taille (2ᵉ passe NAV après layout).
    let mut e = StyleEngine::new();
    e.add_css(
        "@container (min-width: 400px) { .card { width: 100px } }",
        Origin::Author,
    );
    assert_eq!(e.container_queries.len(), 1);
    assert_eq!(e.container_queries[0].name, None);
    // Évaluation : 500≥400 vrai, 300≥400 faux.
    assert!(StyleEngine::matches_container_query(
        "(min-width: 400px)",
        500.0,
        200.0
    ));
    assert!(!StyleEngine::matches_container_query(
        "(min-width: 400px)",
        300.0,
        200.0
    ));
    // `or` disjoint.
    assert!(StyleEngine::matches_container_query(
        "(min-width: 400px) or (min-height: 100px)",
        300.0,
        150.0
    ));
}
