// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! JS-C557 — factories numériques `CSS.<unit>(v)` (css-typed-om
//! §4.3) + `CSS.highlights` : objet plat `{value, unit}` honnête,
//! `toString()` → `"<v><unit>"`.

use explorer_jscore::{Dom, Engine};
use std::cell::RefCell;
use std::rc::Rc;

const PAGE: &str = "<html><body></body></html>";

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

/// Chaque unité BCD présente + appelable, valeur et unité lues.
#[test]
fn css_unit_factories() {
    let mut e = engine();
    assert_eq!(
        ev(
            &mut e,
            "var u = CSS.px(42);\
             (typeof CSS.px) + '|' + (typeof CSS.vw) + '|' + (typeof CSS.deg) + '|' +\
             (typeof CSS.ms) + '|' + (typeof CSS.fr) + '|' + u.value + '|' + u.unit"
        ),
        "function|function|function|function|function|42|px"
    );
    // toString spec-like : "<value><unit>".
    assert_eq!(ev(&mut e, "String(CSS.em(1.5))"), "1.5em");
    assert_eq!(ev(&mut e, "String(CSS.percent(50))"), "50%");
    assert_eq!(ev(&mut e, "String(CSS.number(3))"), "3");
    // Coercition ToNumber sur l'argument.
    assert_eq!(ev(&mut e, "CSS.s('2').value"), "2");
}

/// `CSS.highlights` — `HighlightRegistry` maplike (Map honnête).
#[test]
fn css_highlights_registry() {
    let mut e = engine();
    assert_eq!(
        ev(
            &mut e,
            "CSS.highlights.set('x', 1);\
             (typeof CSS.highlights.set) + '|' + CSS.highlights.get('x') + '|' +\
             CSS.highlights.has('x')"
        ),
        "function|1|true"
    );
}
