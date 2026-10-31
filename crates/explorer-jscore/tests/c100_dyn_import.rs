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

/// JS-C101 — le module livré fait `import {x} from 'dep'` :
/// la dep statique manque le préfetch MAIS la session l'a
/// seedée dans `module_sources` → `instantiate` la trouve.
/// Verrou Shoelace réel : le composant livré importe
/// `LitElement` depuis `lit` (dep statique hors préfetch).
#[test]
fn c101_delivered_module_deps_seeded() {
    let mut e = Engine::new();
    e.set_module_loader(Rc::new(RefCell::new(EmptyLoader)));
    e.set_import_drain();
    e.eval_module(
        "globalThis.__r='start'; import('./comp/sl-button.js').then(m=>{globalThis.__r='ns:'+m.answer;}).catch(x=>{globalThis.__r='rej';});",
        "main.js",
    )
    .expect("module eval");
    let reqs = e.drain_dyn_imports();
    assert_eq!(reqs.len(), 1);
    // La session a fetché le graphe — dep seedée sous l'URL
    // résolue (clé `load_source` : `resolve('lit')` → url).
    e.seed_module_source(
        "https://cdn.example/lit".to_string(),
        "export const Lit = 1;".to_string(),
    );
    e.deliver_import(
        reqs[0].seq,
        Ok("import {Lit} from 'lit'; export const answer = Lit + 41;".to_string()),
    );
    let _ = e.eval("0"); // drain microtasks → `.then` tourné.
    let r = e.eval("globalThis.__r").expect("eval __r");
    assert_eq!(e.describe(r), "ns:42");
}

/// Verrou google SERP : `import("data:…")` se résout inline,
/// SANS réseau ni loader (le lazy-loader Closure avalait le
/// rejet « fetch échoué » — bootstrap muet, page vide).
/// Percent + base64 servis ; MIME non-JS → rejet honnête spec.
#[test]
fn data_url_module_resolves_inline() {
    let mut e = Engine::new();
    // Aucun loader posé — le rail `data:` n'en a pas besoin.
    e.eval_module(
        "globalThis.__r='start';\
         import('data:text/javascript,export%20const%20answer%20=%2042;')\
           .then(m=>{globalThis.__r='ns:'+m.answer;})\
           .catch(()=>{globalThis.__r='rej';});",
        "main.js",
    )
    .expect("module eval");
    let _ = e.eval("0"); // drain microtasks → `.then` tourné.
    let r = e.eval("globalThis.__r").expect("eval __r");
    assert_eq!(e.describe(r), "ns:42");
    // Variante base64 + param charset.
    e.eval_module(
        "globalThis.__r='start';\
         import('data:text/javascript;charset=utf-8;base64,ZXhwb3J0IGNvbnN0IGFuc3dlciA9IDk5Ow==')\
           .then(m=>{globalThis.__r='b64:'+m.answer;})\
           .catch(()=>{globalThis.__r='rej';});",
        "m2.js",
    )
    .expect("module eval b64");
    let _ = e.eval("0");
    let r = e.eval("globalThis.__r").expect("eval __r b64");
    assert_eq!(e.describe(r), "b64:99");
    // MIME non-JS → rejet spec (`text/plain` n'est pas JS).
    e.eval_module(
        "globalThis.__r='start';\
         import('data:text/plain,hello')\
           .then(()=>{globalThis.__r='no';})\
           .catch(()=>{globalThis.__r='rej';});",
        "m3.js",
    )
    .expect("module eval mime");
    let _ = e.eval("0");
    let r = e.eval("globalThis.__r").expect("eval __r mime");
    assert_eq!(e.describe(r), "rej");
    // Tout s'est résolu inline — aucune requête drainée.
    assert!(e.drain_dyn_imports().is_empty());
}

/// JS-C245 — chaîne de modules profonde : `instantiate`/
/// `evaluate_module` récursaient sans garde `maybe_grow` là
/// où parser et éval classiques en ont une (vols 380/381 —
/// `thread 'explorer-jscore' has overflowed its stack` en
/// `module eval`, RSS modeste). Le verrou évalue un graphe
/// de 190 modules chaînés (sous le cap `MAX_MODULE_DEPTH`)
/// : sans la garde, les frames natives lourdes débordent la
/// pile du fil de test (2 Mio).
#[test]
fn deep_module_chain_evaluates_under_stacker() {
    /// `mod_N.js` → `import {v as w} from './mod_N+1.js';
    /// export const v = w + 1` — chaîne de N niveaux.
    struct ChainLoader {
        depth: u32,
        seen: std::cell::Cell<u32>,
    }
    impl explorer_jscore::ModuleLoader for ChainLoader {
        fn load(
            &mut self,
            specifier: &str,
            _referrer: &str,
        ) -> std::result::Result<String, String> {
            self.seen.set(self.seen.get() + 1);
            let n: u32 = specifier
                .trim_start_matches("./mod_")
                .trim_end_matches(".js")
                .parse()
                .unwrap_or(0);
            if n + 1 >= self.depth {
                Ok("export const v = 1;".to_string())
            } else {
                Ok(format!(
                    "import {{ v as w }} from './mod_{}.js'; export const v = w + 1;",
                    n + 1
                ))
            }
        }
        fn resolve(&mut self, specifier: &str, _referrer: &str) -> Option<String> {
            Some(specifier.to_string())
        }
    }

    const DEPTH: u32 = 190;
    let mut e = Engine::new();
    let loader = Rc::new(RefCell::new(ChainLoader {
        depth: DEPTH,
        seen: std::cell::Cell::new(0),
    }));
    e.set_module_loader(loader.clone());
    let ns = e
        .eval_module(
            "import { v } from './mod_0.js'; globalThis.__out = v; export const out = v;",
            "entry.js",
        )
        .expect("module eval chaîne profonde");
    // `out` = DEPTH (chaque niveau ajoute 1 à la feuille 1).
    let r = e.eval("globalThis.__out").expect("eval __out");
    assert_eq!(e.describe(r), DEPTH.to_string());
    // §B-3/B-67 : `describe`/`String` sur un namespace
    // (proto=null) ne doit pas déborder — `to_primitive` nomme
    // l'échec via `describe_shallow`, jamais `describe`.
    let _ = e.describe(ns);
    let _ = loader;
}
