// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Adaptateur `DilloEngine` : proxy canal → thread `Rc` réel,
//! erreurs mappées, budget, `Send` — in-process, pas de socket.

use explorer_js::{DilloEngine, JsEngine, JsError, NullEngine};

#[test]
fn dillo_engine_is_send() {
    fn assert_send<T: Send>() {}
    assert_send::<DilloEngine>();
    assert_send::<NullEngine>();
}

#[test]
fn eval_returns_described_completion() {
    let mut eng = DilloEngine::new();
    assert_eq!(eng.eval("1 + 2 * 3").unwrap(), "7");
    assert_eq!(eng.eval("'a' + 'b'").unwrap(), "ab");
}

#[test]
fn syntax_error_maps_to_variant() {
    let mut eng = DilloEngine::new();
    match eng.eval("var = =") {
        Err(JsError::Syntax { line, .. }) => assert!(line >= 1),
        other => panic!("attendu Syntax, obtenu {other:?}"),
    }
}

#[test]
fn uncaught_maps_to_variant() {
    let mut eng = DilloEngine::new();
    match eng.eval("throw 'boom'") {
        Err(JsError::Uncaught(m)) => assert!(m.contains("boom")),
        other => panic!("attendu Uncaught, obtenu {other:?}"),
    }
}

#[test]
fn budget_kills_infinite_loop() {
    let mut eng = DilloEngine::new();
    eng.set_budget(10_000);
    let r = eng.eval("while (true) {}");
    assert!(r.is_err(), "la boucle infinie doit tomber sur budget");
    // Le moteur reste utilisable (contexte vivant, budget ré-armé).
    eng.set_budget(5_000_000);
    assert_eq!(eng.eval("40 + 2").unwrap(), "42");
}

#[test]
fn run_pending_drains_microtasks() {
    let mut eng = DilloEngine::new();
    let _ = eng.eval("Promise.resolve(1)");
    let _ = eng.run_pending();
}

#[test]
fn factory_builds_contexts() {
    let f = DilloEngine::factory();
    let mut a = (f)();
    let mut b = (f)();
    // Contextes isolés : la variable de a n'existe pas dans b.
    a.eval("var x = 42").unwrap();
    assert!(b.eval("x").is_err());
    assert_eq!(a.eval("x").unwrap(), "42");
}

#[test]
fn null_engine_noops() {
    let mut eng = NullEngine;
    assert_eq!(eng.eval("anything").unwrap(), "undefined");
    eng.set_budget(1);
    assert_eq!(eng.run_pending(), 0);
    eng.install_dom("<html></html>");
    assert_eq!(eng.sync_dom(), None);
}

#[test]
fn install_dom_exposes_document_global() {
    let mut eng = DilloEngine::new();
    eng.install_dom("<html><body><div id=\"x\">old</div></body></html>");
    // `document` installé → bindings visibles, pas de dirty.
    assert_eq!(eng.eval("typeof document").unwrap(), "object");
    assert_eq!(eng.sync_dom(), None);
}

#[test]
fn dom_mutation_sets_dirty_and_serializes() {
    let mut eng = DilloEngine::new();
    eng.install_dom("<html><body><div id=\"x\">old</div></body></html>");
    eng.eval("document.getElementById('x').textContent = 'NEUF'")
        .unwrap();
    let html = eng.sync_dom().expect("mutation attendue : dirty bit levé");
    assert!(html.contains("NEUF"), "sérialisation = {html}");
    assert!(!html.contains(">old<"), "sérialisation = {html}");
    // Dirty bit consommé : la synchro suivante ne revoit rien.
    assert_eq!(eng.sync_dom(), None);
}

#[test]
fn dom_untouched_stays_clean() {
    let mut eng = DilloEngine::new();
    eng.install_dom("<html><body><p>stable</p></body></html>");
    // Lecture seule (getElementById) → pas de mutation.
    eng.eval("document.getElementById('p')").unwrap();
    assert_eq!(eng.sync_dom(), None);
}
