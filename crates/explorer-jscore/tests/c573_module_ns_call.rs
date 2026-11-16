// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! JS-C573 — §B-158 (notion, vol 856) : verbatim
//! `TypeError: object is not a function: [object Module]`.
//! Le site appelle un export lu sur un namespace `import()`
//! dont le binding contient *lui-même* un namespace
//! (`export * as`, re-export d'un `import * as`) — chemin
//! spec-légitime : appeler un Module jette, comme dans
//! Chrome. La sonde `DBG_DRIVE` (`[ns_get] spec::name →
//! Module`) nomme la liaison fautive au prochain vol ; ces
//! tests verrouillent les formes qui produisent/évitent le
//! verbatim, pas un fix (rendre les namespaces appelables
//! violerait `[[Call]]`).

use explorer_jscore::Engine;
use std::cell::RefCell;
use std::rc::Rc;

struct EmptyLoader;

impl explorer_jscore::ModuleLoader for EmptyLoader {
    fn load(&mut self, specifier: &str, _referrer: &str) -> std::result::Result<String, String> {
        Err(format!("not found: {specifier}"))
    }

    fn resolve(&mut self, specifier: &str, _referrer: &str) -> Option<String> {
        Some(format!("https://cdn.example/{specifier}"))
    }
}

/// Importe `src` via `import()` différé, lit `expr` sur le
/// namespace et stocke `String(résultat | erreur)` dans
/// `globalThis.__r`. `m` est en scope dans `expr`.
fn run_import(e: &mut Engine, src: &str, expr: &str) -> String {
    e.eval_module(
        &format!(
            "globalThis.__r='start';\
             import('./m.js').then(m=>{{try{{globalThis.__r=String({expr})}}\
             catch(x){{globalThis.__r=String(x)}}}})\
             .catch(x=>{{globalThis.__r='imp:'+x}});"
        ),
        "main.js",
    )
    .expect("module eval");
    let reqs = e.drain_dyn_imports();
    assert_eq!(reqs.len(), 1);
    e.deliver_import(reqs[0].seq, Ok(src.to_string()));
    let _ = e.eval("0"); // drain microtasks → `.then` tourné.
    let r = e.eval("globalThis.__r").expect("eval __r");
    e.describe(r)
}

/// Appel par *variable* — la forme exacte du verbatim
/// §B-158 : `{removeCookie:n}=cookieService; n(e)` jette
/// « object is not a function: [object Module] » là où
/// `m.f()` jetterait « `f` is not a function ».
fn call_by_var(prop: &str) -> String {
    format!("(()=>{{const n=m.{prop}; return n()}})()")
}

/// `export * as f` — l'export `f` EST le namespace du dep ;
/// `m.f()` jette le verbatim §B-158 (spec : un namespace
/// n'a pas de `[[Call]]`).
#[test]
fn export_star_as_yields_callable_error() {
    let mut e = Engine::new();
    e.set_module_loader(Rc::new(RefCell::new(EmptyLoader)));
    e.set_import_drain();
    e.seed_module_source(
        "https://cdn.example/dep.js".to_string(),
        "export const g = 1;".to_string(),
    );
    let r = run_import(&mut e, "export * as f from './dep.js';", &call_by_var("f"));
    // « [object Module] » au log site (toStringTag lu),
    // « <exception> » si  renonce — les deux
    // prouvent que le callee est un namespace moteur.
    assert!(
        r.contains("[object Module]") || r.contains("<exception>"),
        "verbatim attendu, reçu : {r}"
    );
    assert!(r.contains("TypeError"), "type d'erreur : {r}");
}

/// `import * as ns` re-exporté sous nom — même production :
/// le binding de l'export contient le namespace.
#[test]
fn import_star_reexport_yields_module_value() {
    let mut e = Engine::new();
    e.set_module_loader(Rc::new(RefCell::new(EmptyLoader)));
    e.set_import_drain();
    e.seed_module_source(
        "https://cdn.example/dep.js".to_string(),
        "export const g = 1;".to_string(),
    );
    let r = run_import(
        &mut e,
        "import * as ns from './dep.js'; export {ns};",
        &call_by_var("ns"),
    );
    // « [object Module] » au log site (toStringTag lu),
    // « <exception> » si  renonce — les deux
    // prouvent que le callee est un namespace moteur.
    assert!(
        r.contains("[object Module]") || r.contains("<exception>"),
        "verbatim attendu, reçu : {r}"
    );
    // Mais la lecture non-appelante rend la valeur Module —
    // `typeof` la nomme honnêtement. Moteur frais : le cache
    // `module_urls` court-circuiterait le 2e `import('./m.js')`.
    let mut e2 = Engine::new();
    e2.set_module_loader(Rc::new(RefCell::new(EmptyLoader)));
    e2.set_import_drain();
    e2.seed_module_source(
        "https://cdn.example/dep.js".to_string(),
        "export const g = 1;".to_string(),
    );
    let t = run_import(
        &mut e2,
        "import * as ns from './dep.js'; export {ns};",
        "typeof m.ns",
    );
    assert_eq!(t, "object");
}

/// Re-export transitif d'une *fonction* — le chemin sain :
/// `resolve_export` remonte jusqu'au binding défini dans le
/// dep (pas de Module intermédiaire).
#[test]
fn transitive_reexport_stays_callable() {
    let mut e = Engine::new();
    e.set_module_loader(Rc::new(RefCell::new(EmptyLoader)));
    e.set_import_drain();
    e.seed_module_source(
        "https://cdn.example/dep.js".to_string(),
        "export function f() { return 'ok'; }".to_string(),
    );
    let r = run_import(&mut e, "export {f as g} from './dep.js';", "m.g()");
    assert_eq!(r, "ok");
}

/// `export {ns}` où `ns` vient de `import *` via un
/// intermédiaire : le binding remonte la chaîne et rend
/// toujours le namespace (verbatim préservé à distance).
#[test]
fn star_as_through_reexport_chain() {
    let mut e = Engine::new();
    e.set_module_loader(Rc::new(RefCell::new(EmptyLoader)));
    e.set_import_drain();
    e.seed_module_source(
        "https://cdn.example/leaf.js".to_string(),
        "export const v = 1;".to_string(),
    );
    e.seed_module_source(
        "https://cdn.example/mid.js".to_string(),
        "export * as f from './leaf.js';".to_string(),
    );
    let r = run_import(&mut e, "export {f} from './mid.js';", &call_by_var("f"));
    // « [object Module] » au log site (toStringTag lu),
    // « <exception> » si  renonce — les deux
    // prouvent que le callee est un namespace moteur.
    assert!(
        r.contains("[object Module]") || r.contains("<exception>"),
        "verbatim attendu, reçu : {r}"
    );
}
