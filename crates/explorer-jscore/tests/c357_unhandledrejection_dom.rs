// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! JS-C357 — `unhandledrejection` traverse `propagate_event` :
//! depuis JS-C355, `window.addEventListener` range les
//! listeners dans `node.listeners` (racine DOM), plus dans
//! `__et` — sans propagation, les handlers de la page ne
//! voyaient plus l'event (trou de conformité ; les rejets de
//! la boot-chain restaient invisibles — mur §B-97).

use explorer_jscore::{Dom, Engine};
use std::cell::RefCell;
use std::rc::Rc;

fn engine() -> (Engine, Rc<RefCell<Dom>>) {
    let mut e = Engine::new();
    let dom = Rc::new(RefCell::new(Dom::from_html(
        "<html><body><div></div></body></html>",
    )));
    e.install_dom(&dom);
    (e, dom)
}

/// `window.addEventListener('unhandledrejection')` reçoit
/// l'event (rail DOM post-C355) avec `promise`/`reason`.
#[test]
fn unhandledrejection_reaches_dom_listeners() {
    let (mut e, _dom) = engine();
    e.eval(
        "var got = '';\
         window.addEventListener('unhandledrejection', function(ev){\
           got = ev.type + '/' + ev.reason + '/' + (ev.promise instanceof Promise);\
         });\
         Promise.reject('nope');",
    )
    .expect("eval");
    let got = e.eval("got").expect("got");
    assert_eq!(e.describe(got), "unhandledrejection/nope/true");
}

/// `document.addEventListener` (même file racine) est servi
/// aussi — et un handler qui jette ne casse pas le drain.
#[test]
fn unhandledrejection_document_listener_and_throw_resilience() {
    let (mut e, _dom) = engine();
    e.eval(
        "var n = 0;\
         document.addEventListener('unhandledrejection', function(){ n++; });\
         window.addEventListener('unhandledrejection', function(){ throw 'x'; });\
         Promise.reject('z');",
    )
    .expect("eval");
    let n = e.eval("n").expect("n");
    assert_eq!(
        e.describe(n),
        "1",
        "listener document servi, throw voisin avalé"
    );
}

/// Le rail `__et` (`EventTarget.prototype.addEventListener.
/// call(window,…)`) et `onunhandledrejection` restent servis —
/// pas de double tir pour un listener DOM.
#[test]
fn unhandledrejection_no_double_fire() {
    let (mut e, _dom) = engine();
    e.eval(
        "var hits = 0;\
         window.addEventListener('unhandledrejection', function(){ hits++; });\
         window.onunhandledrejection = function(){ hits += 10; };\
         Promise.reject('q');",
    )
    .expect("eval");
    let hits = e.eval("hits").expect("hits");
    // listener DOM ×1 via propagate + handler DOM0 ×1 via le
    // rail explicite = 11, jamais 12 (pas de double livraison
    // du listener `addEventListener`).
    assert_eq!(e.describe(hits), "11");
}
