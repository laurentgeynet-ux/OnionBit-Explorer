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
    eng.install_dom("<html></html>", "http://h.test/p", 1);
    assert_eq!(eng.sync_dom(), None);
}

#[test]
fn install_dom_exposes_document_global() {
    let mut eng = DilloEngine::new();
    eng.install_dom(
        "<html><body><div id=\"x\">old</div></body></html>",
        "http://h.test/p",
        1,
    );
    // `document` installé → bindings visibles, pas de dirty.
    assert_eq!(eng.eval("typeof document").unwrap(), "object");
    assert_eq!(eng.sync_dom(), None);
}

#[test]
fn dom_mutation_sets_dirty_and_serializes() {
    let mut eng = DilloEngine::new();
    eng.install_dom(
        "<html><body><div id=\"x\">old</div></body></html>",
        "http://h.test/p",
        1,
    );
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
    eng.install_dom(
        "<html><body><p>stable</p></body></html>",
        "http://h.test/p",
        1,
    );
    // Lecture seule (getElementById) → pas de mutation.
    eng.eval("document.getElementById('p')").unwrap();
    assert_eq!(eng.sync_dom(), None);
}

/// Chemin type produit par `element_path` (core) pour un div
/// dans body — wrappers `html`/`body` inclus.
fn path_to_div() -> Vec<(String, u32)> {
    vec![("html".into(), 0), ("body".into(), 0), ("div".into(), 0)]
}

#[test]
fn dispatch_reaches_mirror_listener() {
    let mut eng = DilloEngine::new();
    eng.install_dom(
        "<html><body><div id=\"b\">x</div></body></html>",
        "http://h.test/p",
        1,
    );
    eng.eval(
        "document.getElementById('b').addEventListener('click',\
         function(){this.textContent='HIT'})",
    )
    .unwrap();
    let prevented = eng.dispatch_event("click", &path_to_div()).unwrap();
    assert!(!prevented);
    let html = eng.sync_dom().expect("listener a muté le miroir");
    assert!(html.contains("HIT"), "sérialisation = {html}");
}

#[test]
fn dispatch_skips_implicit_wrappers() {
    let mut eng = DilloEngine::new();
    // Source SANS <html>/<body> : le miroir n'a que le div —
    // le chemin core (avec wrappers) doit quand même l'atteindre.
    eng.install_dom("<div id=\"b\">x</div>", "http://h.test/p", 1);
    eng.eval(
        "document.getElementById('b').addEventListener('click',\
         function(){this.textContent='HIT'})",
    )
    .unwrap();
    assert!(eng.dispatch_event("click", &path_to_div()).is_ok());
    assert!(eng.sync_dom().is_some_and(|h| h.contains("HIT")));
}

#[test]
fn dispatch_prevent_default_propagates() {
    let mut eng = DilloEngine::new();
    eng.install_dom(
        "<html><body><div id=\"b\">x</div></body></html>",
        "http://h.test/p",
        1,
    );
    eng.eval(
        "document.getElementById('b').addEventListener('click',\
         function(e){e.preventDefault()})",
    )
    .unwrap();
    assert!(eng.dispatch_event("click", &path_to_div()).unwrap());
}

#[test]
fn dispatch_bad_path_is_noop() {
    let mut eng = DilloEngine::new();
    eng.install_dom(
        "<html><body><div id=\"b\">x</div></body></html>",
        "http://h.test/p",
        1,
    );
    let bogus = vec![("html".into(), 0), ("body".into(), 0), ("table".into(), 0)];
    assert!(!eng.dispatch_event("click", &bogus).unwrap());
    assert_eq!(eng.sync_dom(), None);
}

#[test]
fn dispatch_without_mirror_is_noop() {
    let mut eng = DilloEngine::new();
    assert!(!eng.dispatch_event("click", &path_to_div()).unwrap());
}

#[test]
fn nav_requests_drained() {
    let mut eng = DilloEngine::new();
    eng.install_dom("<html></html>", "http://h.test/p", 3);
    eng.eval("location.href = '/next'; history.back(); history.go(2);")
        .unwrap();
    assert_eq!(
        eng.drain_nav_requests(),
        vec![
            explorer_js::NavRequest::Navigate("/next".to_string()),
            explorer_js::NavRequest::HistoryGo(-1),
            explorer_js::NavRequest::HistoryGo(2),
        ]
    );
    // File vidée ; un second drain ne re-rend rien.
    assert_eq!(eng.drain_nav_requests().len(), 0);
}

#[test]
fn location_reads_page_url() {
    let mut eng = DilloEngine::new();
    eng.install_dom("<html></html>", "http://h.test/dir/p", 1);
    let out = eng.eval("location.pathname").unwrap();
    assert!(out.contains("/dir/p"), "got {out}");
    let out = eng.eval("history.length").unwrap();
    assert!(out.contains('1'), "got {out}");
}
