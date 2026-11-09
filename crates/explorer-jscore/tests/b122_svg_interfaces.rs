// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! §B-122 — interfaces SVG non-élément (`SVGAnimated*` et
//! objets-valeur legacy) présentes sur le global. Vol 757
//! explore.org : Swiper fait `e[t] instanceof
//! SVGAnimatedString` (le `className` des éléments SVG est un
//! objet animé, pas une chaîne) — l'interface absente jetait
//! `ReferenceError` depuis les `MutationObservers` à chaque
//! mutation DOM, tuant les insertions (dont l'iframe du
//! lecteur). Le verrou exige l'interface, le `instanceof` à
//! `false` sur une string (branche attendue par Swiper), et
//! le ctor non constructible (`WebIDL`).

use explorer_jscore::{Dom, Engine};
use std::cell::RefCell;
use std::rc::Rc;

const PAGE: &str = r#"<html><body><div id="x"></div></body></html>"#;

fn engine() -> Engine {
    let mut e = Engine::new();
    let dom = Rc::new(RefCell::new(Dom::from_html(PAGE)));
    e.install_dom(&dom);
    e
}

fn ev(e: &mut Engine, src: &str) -> String {
    match e.eval(src) {
        Ok(v) => e.describe(v),
        Err(err) => panic!("{src} -> {err}"),
    }
}

#[test]
fn svg_animated_interfaces_declared() {
    let mut e = engine();
    for name in [
        "SVGAnimatedAngle",
        "SVGAnimatedBoolean",
        "SVGAnimatedEnumeration",
        "SVGAnimatedInteger",
        "SVGAnimatedLength",
        "SVGAnimatedLengthList",
        "SVGAnimatedNumber",
        "SVGAnimatedNumberList",
        "SVGAnimatedPreserveAspectRatio",
        "SVGAnimatedRect",
        "SVGAnimatedString",
        "SVGAnimatedTransformList",
        "SVGLength",
        "SVGLengthList",
        "SVGNumber",
        "SVGNumberList",
        "SVGAngle",
        "SVGPreserveAspectRatio",
        "SVGPoint",
        "SVGPointList",
        "SVGMatrix",
        "SVGRect",
        "SVGStringList",
        "SVGTransform",
        "SVGTransformList",
        "SVGUnitTypes",
    ] {
        let got = ev(&mut e, &format!("typeof {name}"));
        assert_eq!(got, "function", "{name} absent du global");
    }
}

#[test]
fn swiper_pattern_instanceof_no_throw() {
    let mut e = engine();
    // Le pattern exact de Swiper : propriété DOM lue puis
    // `instanceof SVGAnimatedString` — une string rend
    // `false` (notre DOM ne produit jamais d'objet animé),
    // sans lever d'erreur.
    assert_eq!(ev(&mut e, r#""cls" instanceof SVGAnimatedString"#), "false");
    assert_eq!(
        ev(
            &mut e,
            r#"var x=document.getElementById("x"); x.className instanceof SVGAnimatedString"#
        ),
        "false"
    );
    // Même motif sur les longueurs animées (GSAP/d3).
    assert_eq!(ev(&mut e, "1 instanceof SVGAnimatedLength"), "false");
}

#[test]
fn animated_interfaces_not_constructible() {
    let mut e = engine();
    match e.eval("new SVGAnimatedString()") {
        Err(err) => assert!(
            err.to_string().contains("TypeError"),
            "ctor devrait lever TypeError: {err}"
        ),
        Ok(v) => panic!(
            "new SVGAnimatedString() aurait dû lever, a rendu {}",
            e.describe(v)
        ),
    }
}
