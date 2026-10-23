// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Tests des valeurs typées, expansion des shorthands et
//! pseudo-classes dynamiques (étape 16).

use explorer_css::value::{expand_shorthand, parse_color, parse_length, Color, Unit};
use explorer_css::{matches_with_state, parse_selector_list, Origin, StyleEngine, StyleEnv};
use explorer_dom::{Dom, NodeId};

// ── longueurs ────────────────────────────────────────────────

#[test]
fn longueurs_absolues_et_relatives() {
    let l = parse_length("12px").unwrap();
    assert_eq!((l.value, l.unit), (12.0, Unit::Px));
    assert_eq!(parse_length("1.5em").unwrap().unit, Unit::Em);
    assert_eq!(parse_length("2rem").unwrap().unit, Unit::Rem);
    assert_eq!(parse_length("-3pt").unwrap().value, -3.0);
    assert_eq!(parse_length("0").unwrap().value, 0.0);
    assert_eq!(parse_length("2in").unwrap().unit, Unit::In);
    // Facteurs absolus → px.
    assert_eq!(Unit::In.px_factor(), Some(96.0));
    assert_eq!(Unit::Pt.px_factor(), Some(96.0 / 72.0));
    assert_eq!(Unit::Pc.px_factor(), Some(16.0));
    assert_eq!(Unit::Em.px_factor(), None); // relative : used value
}

#[test]
fn longueurs_invalides() {
    for bad in ["abc", "10xx", "auto", "medium", "", "px", "--"] {
        assert!(parse_length(bad).is_none(), "attendu None : {bad}");
    }
}

// ── couleurs ─────────────────────────────────────────────────

#[test]
fn couleurs_hex_rgb_nommees() {
    assert_eq!(
        parse_color("#f00"),
        Some(Color {
            r: 255,
            g: 0,
            b: 0,
            a: 255
        })
    );
    assert_eq!(
        parse_color("#00ff00"),
        Some(Color {
            r: 0,
            g: 255,
            b: 0,
            a: 255
        })
    );
    assert_eq!(parse_color("#0000ff80").unwrap().a, 128);
    assert_eq!(parse_color("rgb(255, 0, 0)").unwrap().r, 255);
    // 0.5 * 255 = 127.5 → arrondi à 128.
    assert_eq!(parse_color("rgba(0, 0, 0, 0.5)").unwrap().a, 128);
    assert_eq!(parse_color("rgb(100%, 0%, 0%)").unwrap().r, 255);
    assert_eq!(parse_color("red").unwrap().r, 255);
    assert_eq!(parse_color("transparent"), Some(Color::TRANSPARENT));
    for bad in ["#", "#12", "#xyz", "rgb(1,2)", "mauve?", ""] {
        assert!(parse_color(bad).is_none(), "attendu None : {bad}");
    }
}

// ── shorthands ────────────────────────────────────────────────

#[test]
fn shorthand_margin_quatre_cotes() {
    let e = expand_shorthand("margin", "1px").unwrap();
    assert_eq!(
        e,
        vec![
            ("margin-top".to_string(), "1px".to_string()),
            ("margin-right".to_string(), "1px".to_string()),
            ("margin-bottom".to_string(), "1px".to_string()),
            ("margin-left".to_string(), "1px".to_string()),
        ]
    );
    let e = expand_shorthand("margin", "1px 2px").unwrap();
    assert_eq!(e[0].1, "1px"); // top
    assert_eq!(e[1].1, "2px"); // right
    assert_eq!(e[2].1, "1px"); // bottom
    assert_eq!(e[3].1, "2px"); // left
    let e = expand_shorthand("padding", "1px 2px 3px").unwrap();
    assert_eq!(e[2].1, "3px"); // bottom
    assert_eq!(e[3].1, "2px"); // left = right
    let e = expand_shorthand("margin", "1px 2px 3px 4px").unwrap();
    assert_eq!(e[3].1, "4px");
    assert!(expand_shorthand("margin", "1px 2px 3px 4px 5px").is_none());
}

#[test]
fn shorthand_border() {
    let e = expand_shorthand("border", "2px solid red").unwrap();
    assert!(e.contains(&("border-top-width".into(), "2px".into())));
    assert!(e.contains(&("border-top-style".into(), "solid".into())));
    assert!(e.contains(&("border-top-color".into(), "red".into())));
    assert!(e.contains(&("border-left-color".into(), "red".into())));
    // Ordre libre.
    let e = expand_shorthand("border", "red solid 2px").unwrap();
    assert_eq!(e.len(), 12);
    // Une seule composante.
    let e = expand_shorthand("border", "solid").unwrap();
    assert_eq!(e.len(), 4);
    assert!(e.iter().all(|(n, _)| n.ends_with("-style")));
    // border-width 4 côtés.
    let e = expand_shorthand("border-width", "1px 2px").unwrap();
    assert_eq!(e.len(), 4);
    assert_eq!(e[1].1, "2px");
}

#[test]
fn shorthand_font_et_overflow() {
    let e = expand_shorthand("font", "italic bold 12px/1.5 serif").unwrap();
    assert!(e.contains(&("font-style".into(), "italic".into())));
    assert!(e.contains(&("font-weight".into(), "bold".into())));
    assert!(e.contains(&("font-size".into(), "12px".into())));
    assert!(e.contains(&("line-height".into(), "1.5".into())));
    assert!(e.contains(&("font-family".into(), "serif".into())));
    let e = expand_shorthand("font", "1em monospace").unwrap();
    assert!(e.contains(&("font-size".into(), "1em".into())));
    assert!(e.contains(&("font-family".into(), "monospace".into())));
    let e = expand_shorthand("overflow", "hidden auto").unwrap();
    assert_eq!(
        e,
        vec![
            ("overflow-x".into(), "hidden".into()),
            ("overflow-y".into(), "auto".into())
        ]
    );
}

// ── cascade + shorthands + valeurs typées ────────────────────

fn fixture() -> (Dom, NodeId) {
    let mut dom = Dom::new();
    let doc = dom.document();
    let div = dom.create_element("div").unwrap();
    dom.append_child(doc, div).unwrap();
    (dom, div)
}

#[test]
fn shorthand_en_cascade() {
    let (dom, div) = fixture();
    let mut e = StyleEngine::new();
    e.add_css(
        "div { margin: 5px 10px; border: 1px solid red }",
        Origin::Author,
    );
    let s = e.compute(&dom, div, &StyleEnv::default());
    assert_eq!(s.get("margin-top"), Some("5px"));
    assert_eq!(s.get("margin-right"), Some("10px"));
    assert_eq!(s.get("border-top-width"), Some("1px"));
    assert_eq!(s.get("border-top-style"), Some("solid"));
    assert_eq!(s.get("border-top-color"), Some("red"));
    // Le longhand gagne s'il arrive après (même spécificité,
    // ordre source).
    e.add_css("div { margin-top: 9px }", Origin::Author);
    assert_eq!(
        e.compute(&dom, div, &StyleEnv::default()).get("margin-top"),
        Some("9px")
    );
}

#[test]
fn accesseurs_types() {
    let (dom, div) = fixture();
    let mut e = StyleEngine::new();
    e.add_css(
        "div { margin-top: 2.5em; color: #f00; border-top-color: currentcolor; z-index: 7 }",
        Origin::Author,
    );
    let s = e.compute(&dom, div, &StyleEnv::default());
    let l = s.length("margin-top").unwrap();
    assert_eq!((l.value, l.unit), (2.5, Unit::Em));
    assert_eq!(s.color("color").unwrap().r, 255);
    // currentcolor → la couleur de l'élément.
    assert_eq!(s.color("border-top-color").unwrap().r, 255);
    assert_eq!(s.number("z-index"), Some(7.0));
}

// ── pseudo-classes dynamiques ────────────────────────────────

#[test]
fn dynamiques_selon_etat() {
    let (dom, div) = fixture();
    let mut e = StyleEngine::new();
    e.add_css(
        "div:hover { color: red } div { color: blue }",
        Origin::Author,
    );
    // Sans état → :hover faux → blue.
    let env = StyleEnv::default();
    assert_eq!(e.compute(&dom, div, &env).get("color"), Some("blue"));
    // État hover actif → red.
    let s = e.compute_with_state(&dom, div, &env, &|name| name == "hover");
    assert_eq!(s.get("color"), Some("red"));
    // Autre état → toujours blue.
    let s = e.compute_with_state(&dom, div, &env, &|name| name == "focus");
    assert_eq!(s.get("color"), Some("blue"));
}

#[test]
fn link_structurel() {
    let mut dom = Dom::new();
    let doc = dom.document();
    let a = dom.create_element("a").unwrap();
    dom.set_attribute(a, "href", "x").unwrap();
    dom.append_child(doc, a).unwrap();
    let b = dom.create_element("a").unwrap();
    dom.append_child(doc, b).unwrap();
    let sels = parse_selector_list("a:link").unwrap();
    // :link matche a[href] même sans état (structurel).
    assert!(matches_with_state(&dom, a, &sels[0], &|_| false));
    assert!(!matches_with_state(&dom, b, &sels[0], &|_| false));
    // :visited jamais (pas de tracking).
    let sels = parse_selector_list("a:visited").unwrap();
    assert!(!matches_with_state(&dom, a, &sels[0], &|_| true));
}

#[test]
fn shorthand_background_size_et_gradient() {
    // Issue #10 — `pos / size` ne doit plus dropper la
    // déclaration : couleur, position, repeat ET size émis.
    let e = expand_shorthand("background", "red 50% / contain no-repeat").unwrap();
    let get = |n: &str| e.iter().find(|(k, _)| k == n).map(|(_, v)| v.as_str());
    assert_eq!(get("background-color"), Some("red"));
    assert_eq!(get("background-position"), Some("50%"));
    assert_eq!(get("background-size"), Some("contain"));
    assert_eq!(get("background-repeat"), Some("no-repeat"));
    assert_eq!(get("background-image"), Some("none"));

    // `/` collé ou séparé, size à 2 valeurs.
    let e = expand_shorthand("background", "url(a.png) center/cover").unwrap();
    let get = |n: &str| e.iter().find(|(k, _)| k == n).map(|(_, v)| v.as_str());
    assert_eq!(get("background-image"), Some("url(a.png)"));
    assert_eq!(get("background-position"), Some("center"));
    assert_eq!(get("background-size"), Some("cover"));

    // Fonction image : le gradient, espaces internes compris,
    // est UN atome du shorthand.
    let e = expand_shorthand(
        "background",
        "linear-gradient(to right, red, blue) no-repeat",
    )
    .unwrap();
    let get = |n: &str| e.iter().find(|(k, _)| k == n).map(|(_, v)| v.as_str());
    assert_eq!(
        get("background-image"),
        Some("linear-gradient(to right, red, blue)")
    );
    assert_eq!(get("background-repeat"), Some("no-repeat"));
    assert_eq!(get("background-size"), Some("auto"));

    // Composantes après la taille : `repeat` ressort du mode
    // size, la déclaration survit.
    let e = expand_shorthand("background", "url(a.png) center / cover no-repeat fixed").unwrap();
    let get = |n: &str| e.iter().find(|(k, _)| k == n).map(|(_, v)| v.as_str());
    assert_eq!(get("background-size"), Some("cover"));
    assert_eq!(get("background-repeat"), Some("no-repeat"));
    assert_eq!(get("background-attachment"), Some("fixed"));

    // Rejets préservés : doublon de couleur, token inconnu,
    // taille non déclarée derrière `/`, trop de segments.
    assert!(expand_shorthand("background", "red pink").is_none());
    assert!(expand_shorthand("background", "frobnicate").is_none());
    assert!(expand_shorthand("background", "url(a.png) cover").is_none());
    assert!(expand_shorthand("background", "url(a.png) /cover/x/y").is_none());
}

#[test]
fn shorthand_columns() {
    // R-89 : `columns` = `[width] [count]` ordre libre —
    // l'accueil FR n'utilise que les longhands mais le
    // raccourci couvre la forme compacte.
    let e = expand_shorthand("columns", "10em 3").unwrap();
    assert_eq!(
        e,
        vec![
            ("column-width".to_string(), "10em".to_string()),
            ("column-count".to_string(), "3".to_string()),
        ]
    );
    let e = expand_shorthand("columns", "auto 2").unwrap();
    assert_eq!(e[1].1, "2");
    assert_eq!(e[0].1, "auto");
}
