// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! JS-C558 — audit `invoke` sans `has_method` : 7 hôtes dont les
//! méthodes étaient invisibles à `typeof`/`in` (dispatch `invoke`
//! direct au call-site masquait le trou — `ctx.fillRect(0,0,1,1)`
//! marchait mais `typeof ctx.fillRect === 'undefined'` et
//! `const f = ctx.fillRect` cassait).

use explorer_jscore::{Dom, Engine};
use std::cell::RefCell;
use std::rc::Rc;

const PAGE: &str = r#"<html><body><div id="d" x="1"></div></body></html>"#;

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

/// `CanvasRenderingContext2D` + `CanvasGradient`.
#[test]
fn ctx2d_methods_visible() {
    let mut e = engine();
    assert_eq!(
        ev(
            &mut e,
            "var c = document.createElement('canvas').getContext('2d');\
             (typeof c.fillRect) + '|' + ('fillRect' in c) + '|' +\
             (typeof c.measureText) + '|' + ('getImageData' in c) + '|' +\
             ('fillText' in c) + '|' + (typeof c.fillRect.bind)"
        ),
        "function|true|function|true|true|function"
    );
    assert_eq!(
        ev(
            &mut e,
            "var g = document.createElement('canvas').getContext('2d')\
                .createLinearGradient(0,0,1,1);\
             (typeof g.addColorStop) + '|' + ('addColorStop' in g)"
        ),
        "function|true"
    );
}

/// `DOMImplementation`, `History`, `TimeRanges`, `NamedNodeMap`.
#[test]
fn platform_hosts_methods_visible() {
    let mut e = engine();
    assert_eq!(
        ev(
            &mut e,
            "(typeof document.implementation.hasFeature) + '|' +\
             ('createHTMLDocument' in document.implementation) + '|' +\
             (typeof history.pushState) + '|' + ('replaceState' in history)"
        ),
        "function|true|function|true"
    );
    assert_eq!(
        ev(
            &mut e,
            "var b = document.createElement('video').buffered;\
             (typeof b.start) + '|' + ('end' in b) + '|' + b.length"
        ),
        "function|true|0"
    );
    assert_eq!(
        ev(
            &mut e,
            "var a = document.getElementById('d').attributes;\
             (typeof a.item) + '|' + ('getNamedItem' in a) + '|' +\
             ('removeNamedItemNS' in a)"
        ),
        "function|true|true"
    );
}

/// `Event` — méthodes `invoke`-only visibles + lecture liée.
#[test]
fn event_methods_visible() {
    let mut e = engine();
    assert_eq!(
        ev(
            &mut e,
            "var ev = new Event('click');\
             (typeof ev.preventDefault) + '|' + ('composedPath' in ev) + '|' +\
             (typeof ev.initEvent) + '|' + ('stopImmediatePropagation' in ev) + '|' +\
             Array.isArray(ev.composedPath())"
        ),
        "function|true|function|true|true"
    );
}
