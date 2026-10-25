// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! JS-C100 — `import()` dynamique servi à la demande : miss du
//! chargeur → promesse pending + `DynImportReq` drainée +
//! `deliver_import` résout le namespace. Verrou Shoelace
//! mesuré : `MapLoader` ne sert que le préfetch — les specs
//! dynamiques (`${tag}.js`) rejetaient silencieusement.

use explorer_jscore::Engine;
use std::cell::RefCell;
use std::rc::Rc;

/// Loader qui ne connaît rien — chaque `import()` fait miss.
struct EmptyLoader;

impl explorer_jscore::ModuleLoader for EmptyLoader {
    fn load(&mut self, specifier: &str, _referrer: &str) -> std::result::Result<String, String> {
        Err(format!("not found: {specifier}"))
    }

    fn resolve(&mut self, specifier: &str, _referrer: &str) -> Option<String> {
        Some(format!("https://cdn.example/{specifier}"))
    }
}

/// Chaîne `import()` end-to-end : miss → defer → drain →
/// deliver → resolve. Reproduit le chemin Shoelace
/// (`import("./dist/components/...")` non préfetché).
#[test]
fn c100_deferred_import_resolves() {
    let mut e = Engine::new();
    e.set_module_loader(Rc::new(RefCell::new(EmptyLoader)));
    e.set_import_drain();
    e.eval_module(
        "globalThis.__r = 'start';\
         import('./comp/sl-button.js').then(m => { globalThis.__r = 'ns:' + m.answer; });",
        "main.js",
    )
    .expect("module eval");
    // La promesse est pending — `__r` n'a pas bougé.
    let r = e.eval("globalThis.__r").expect("eval __r");
    assert_eq!(e.describe(r), "start");
    // Une requête est enfilée — URL résolue par le loader.
    let reqs = e.drain_dyn_imports();
    assert_eq!(reqs.len(), 1);
    assert_eq!(reqs[0].url, "https://cdn.example/./comp/sl-button.js");
    // Livraison de la source → instancier + évaluer +
    // résoudre la promesse avec le namespace.
    e.deliver_import(reqs[0].seq, Ok("export const answer = 42;".to_string()));
    // `eval` lit AVANT le drain microtasks — no-op d'abord
    // pour voir le résultat du `.then`.
    let _ = e.eval("0");
    let r = e.eval("globalThis.__r").expect("eval __r2");
    assert_eq!(e.describe(r), "ns:42");
}

/// `deliver_import(Err)` → la promesse `import()` rejette.
#[test]
fn c100_deferred_import_rejects() {
    let mut e = Engine::new();
    e.set_module_loader(Rc::new(RefCell::new(EmptyLoader)));
    e.set_import_drain();
    e.eval_module(
        "globalThis.__r = 'start';\
         import('./nope.js').catch(m => { globalThis.__r = 'rej'; });",
        "main.js",
    )
    .expect("module eval");
    let reqs = e.drain_dyn_imports();
    assert_eq!(reqs.len(), 1);
    e.deliver_import(reqs[0].seq, Err("fetch échoué".to_string()));
    let _ = e.eval("0"); // drain microtasks → `.catch` tourné.
    let r = e.eval("globalThis.__r").expect("eval __r");
    assert_eq!(e.describe(r), "rej");
}

/// Sans `set_import_drain`, un miss rejette comme avant
/// (pas de defer fantôme).
#[test]
fn c100_no_drain_rejects_immediately() {
    let mut e = Engine::new();
    e.set_module_loader(Rc::new(RefCell::new(EmptyLoader)));
    e.eval_module(
        "globalThis.__r = 'start';\
         import('./nope.js').catch(m => { globalThis.__r = 'rej'; });",
        "main.js",
    )
    .expect("module eval");
    // Rejet immédiat — le `.catch` a tourné sans drain.
    assert!(e.drain_dyn_imports().is_empty());
    let _ = e.eval("0"); // drain microtasks → `.catch` tourné.
    let r = e.eval("globalThis.__r").expect("eval __r");
    assert_eq!(e.describe(r), "rej");
}
