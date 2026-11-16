// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! JS-C559 — membres IDL fréquents : accesseurs prototype
//! host-first (`dom_acc_get`/`set` délèguent au getter canonique
//! `NodeHost::get`/`set` avant le repli attribut), décomposition
//! URL étendue (`username`/`password`, `host` sans userinfo),
//! formulaires (`form`, `labels`, `control`, validation),
//! collections (`options`, `elements`, `rows`, `cells`,
//! `tBodies`), tables (`cellIndex`/`rowIndex`/`sectionRowIndex`),
//! médias, et la famille `SVGFE*Element` (~194 clés).

use explorer_jscore::{Dom, Engine};
use std::cell::RefCell;
use std::rc::Rc;

const PAGE: &str = r#"<html><body><div id="d"></div></body></html>"#;

fn engine() -> Engine {
    let mut e = Engine::new();
    e.install_dom(&Rc::new(RefCell::new(Dom::from_html(PAGE))));
    e
}

fn ev(e: &mut Engine, src: &str) -> String {
    match e.eval(src) {
        Ok(v) => e.describe(v),
        Err(err) => panic!("{src} -> {err}"),
    }
}

/// `HTMLAnchorElement` : décomposition URL complète, userinfo
/// exclue de `host`/`hostname`/`origin` (spec URL).
#[test]
fn anchor_url_parts() {
    let mut e = engine();
    assert_eq!(
        ev(
            &mut e,
            "var a = document.createElement('a');\
             a.setAttribute('href', 'https://u:p@ex.com:8080/p?q#f');\
             a.username + '|' + a.password + '|' + a.protocol + '|' +\
             a.host + '|' + a.hostname + '|' + a.port + '|' +\
             a.pathname + '|' + a.search + '|' + a.hash + '|' + a.origin"
        ),
        "u|p|https:|ex.com:8080|ex.com|8080|/p|?q|#f|https://ex.com:8080"
    );
    // Membres visibles sur le prototype (`in` + `typeof`).
    assert_eq!(
        ev(
            &mut e,
            "('username' in HTMLAnchorElement.prototype) + '|' +\
             ('password' in HTMLAnchorElement.prototype) + '|' +\
             ('origin' in HTMLAnchorElement.prototype) + '|' +\
             ('host' in HTMLAnchorElement.prototype)"
        ),
        "true|true|true|true"
    );
}

/// Formulaires : `form` remonte par `form=` ou ancêtre,
/// `labels`/`control` par `for=`/`id`, validation + sélection.
#[test]
fn form_control_members() {
    let mut e = engine();
    assert_eq!(
        ev(
            &mut e,
            "var f = document.createElement('form'); f.id = 'F';\
             var i = document.createElement('input');\
             i.setAttribute('required', ''); i.setAttribute('form', 'F');\
             document.body.appendChild(f); document.body.appendChild(i);\
             (i.form === f) + '|' + i.willValidate + '|' +\
             (typeof i.checkValidity) + '|' + i.required + '|' +\
             ('checkValidity' in HTMLInputElement.prototype) + '|' +\
             ('form' in HTMLInputElement.prototype)"
        ),
        "true|true|function|true|true|true"
    );
    // `indeterminate` : setter réfléchi (propriété, pas attribut).
    assert_eq!(
        ev(
            &mut e,
            "var c = document.createElement('input');\
             c.setAttribute('type', 'checkbox'); c.indeterminate = true;\
             c.indeterminate"
        ),
        "true"
    );
    // `label.control` / `input.labels` (for/id croisés).
    assert_eq!(
        ev(
            &mut e,
            "var l = document.createElement('label'); l.htmlFor = 'xx';\
             var i2 = document.createElement('input'); i2.id = 'xx';\
             document.body.appendChild(l); document.body.appendChild(i2);\
             (l.control === i2) + '|' + i2.labels.length"
        ),
        "true|1"
    );
}

/// `select`/`option` : `options`, `length`, `selectedIndex`,
/// `option.index` ; `table` : `rows`/`cells`/`tBodies` + index.
#[test]
fn collections_and_table_index() {
    let mut e = engine();
    assert_eq!(
        ev(
            &mut e,
            "var s = document.createElement('select');\
             var o1 = document.createElement('option');\
             var o2 = document.createElement('option');\
             s.appendChild(o1); s.appendChild(o2);\
             document.body.appendChild(s);\
             s.length + '|' + s.options.length + '|' + o2.index + '|' +\
             ('options' in HTMLSelectElement.prototype) + '|' +\
             ('submit' in HTMLFormElement.prototype)"
        ),
        "2|2|1|true|true"
    );
    assert_eq!(
        ev(
            &mut e,
            "var t = document.createElement('table');\
             var tb = document.createElement('tbody');\
             var tr = document.createElement('tr');\
             var td = document.createElement('td');\
             tr.appendChild(td); tb.appendChild(tr); t.appendChild(tb);\
             document.body.appendChild(t);\
             td.cellIndex + '|' + tr.rowIndex + '|' + tr.sectionRowIndex + '|' +\
             t.rows.length + '|' + tr.cells.length + '|' + t.tBodies.length"
        ),
        "0|0|0|1|1|1"
    );
}

/// `SVGFE*Element` : constructeurs installés (illegal-ctor) +
/// attributs SVG préservant la casse (`stdDeviation`, `in1`).
#[test]
fn svg_filter_interfaces() {
    let mut e = engine();
    assert_eq!(
        ev(
            &mut e,
            "(typeof SVGFEBlendElement) + '|' + (typeof SVGFEGaussianBlurElement) + '|' +\
             ('in1' in SVGFEBlendElement.prototype) + '|' +\
             ('stdDeviationX' in SVGFEGaussianBlurElement.prototype) + '|' +\
             (typeof SVGFECompositeElement) + '|' + (typeof SVGFEDropShadowElement)"
        ),
        "function|function|true|true|function|function"
    );
}

/// Médias : `paused`/`play`/`poster` honnêtes (moteur sans
/// lecture réelle — `paused` reste vrai, `play` est un stub).
#[test]
fn media_members() {
    let mut e = engine();
    assert_eq!(
        ev(
            &mut e,
            "var v = document.createElement('video');\
             ('paused' in HTMLMediaElement.prototype) + '|' +\
             ('play' in HTMLMediaElement.prototype) + '|' +\
             ('poster' in HTMLVideoElement.prototype) + '|' +\
             v.paused + '|' + (typeof v.play) + '|' +\
             ('requestFullscreen' in v)"
        ),
        "true|true|true|true|function|true"
    );
}

/// Scoping : les membres tag-spécifiques ne fuient pas sur les
/// éléments sans rapport (`'value' in div` doit rester faux).
#[test]
fn tag_scoping_preserved() {
    let mut e = engine();
    assert_eq!(
        ev(
            &mut e,
            "var d = document.getElementById('d');\
             ('value' in d) + '|' + ('files' in d) + '|' + ('href' in d) + '|' +\
             ('options' in d) + '|' + ('paused' in d) + '|' + ('className' in d)"
        ),
        "false|false|false|false|false|true"
    );
}
