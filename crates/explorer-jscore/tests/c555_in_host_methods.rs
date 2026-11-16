// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! JS-C555 — l'opérateur `in` voit les méthodes du contrat
//! `Host` (`has_method`). `read_prop` les expose déjà comme
//! fonctions liées (`typeof r.setStart === 'function'`) mais
//! `has_prop_walk` ne consultait que `Host::get` → `'setStart'
//! in document.createRange()` rendait `false`, invisible aux
//! feature-detects (`'cloneNode' in n`, sondes BCD `api`).

use explorer_jscore::{Dom, Engine};
use std::cell::RefCell;
use std::rc::Rc;

const PAGE: &str = r#"<html><body><div id="host"><i id="kid">txt</i></div></body></html>"#;

fn engine() -> (Engine, Rc<RefCell<Dom>>) {
    let mut e = Engine::new();
    let dom = Rc::new(RefCell::new(Dom::from_html(PAGE)));
    e.install_dom(&dom);
    (e, dom)
}

fn ev(e: &mut Engine, src: &str) -> String {
    match e.eval(src) {
        Ok(v) => e.describe(v),
        Err(err) => panic!("{src} -> {err}"),
    }
}

/// Méthodes host visibles à `in` — Range, Selection, WebSocket.
#[test]
fn in_operator_sees_host_methods() {
    let (mut e, _dom) = engine();
    assert_eq!(
        ev(
            &mut e,
            "var r = document.createRange();\
             ('setStart' in r) + '|' + ('cloneContents' in r) + '|' +\
             ('startContainer' in r) + '|' + ('collapsed' in r)"
        ),
        "true|true|true|true"
    );
    assert_eq!(
        ev(
            &mut e,
            "var s = window.getSelection();\
             ('addRange' in s) + '|' + ('collapse' in s) + '|' + ('anchorNode' in s)"
        ),
        "true|true|true"
    );
    assert_eq!(
        ev(
            &mut e,
            "var w = new WebSocket('ws://x/');\
             ('send' in w) + '|' + ('close' in w) + '|' + ('readyState' in w)"
        ),
        "true|true|true"
    );
}

/// Le négatif reste honnête : clés inexistantes, et `in` sur
/// les objets ordinaires/proto-inchangés.
#[test]
fn in_operator_negatives_unchanged() {
    let (mut e, _dom) = engine();
    assert_eq!(
        ev(
            &mut e,
            "var r = document.createRange();\
             ('nope' in r) + '|' + ('hasOwnProperty' in r) + '|' +\
             ({a:1}).hasOwnProperty('a') + '|' + ('b' in {a:1})"
        ),
        "false|true|true|false"
    );
    // `hasOwnProperty` reste spec : les méthodes host vivent
    // « sur le prototype » conceptuel — pas own (V8 pareil).
    assert_eq!(
        ev(
            &mut e,
            "var r2 = document.createRange();\
             r2.hasOwnProperty('setStart') + '|' + r2.hasOwnProperty('startContainer')"
        ),
        "false|false"
    );
}

/// Une own-prop scriptée du nom d'une méthode reste `in`-vraie
/// et prime à la lecture (trampoline cédant, JS-C341 intact).
#[test]
fn in_operator_own_prop_and_trampoline() {
    let (mut e, _dom) = engine();
    assert_eq!(
        ev(
            &mut e,
            "var n = document.getElementById('host');\
             n.setStart = 7;\
             ('setStart' in n) + '|' + n.setStart + '|' +\
             ('appendChild' in n) + '|' + typeof n.appendChild"
        ),
        "true|7|true|function"
    );
}
