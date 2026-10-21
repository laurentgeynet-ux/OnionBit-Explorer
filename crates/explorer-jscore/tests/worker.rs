// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! `Worker` bornés (JS-C29) : thread par worker, `postMessage`
//! cloné par valeur, `onmessage` des deux côtés, `importScripts`,
//! `terminate`, pool borné. In-process — jamais de réseau : le
//! `WorkerLoader` sert des sources mémoire.

use explorer_jscore::{Dom, Engine, WorkerConfig, WorkerLoader};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Loader mémoire — map `spec → source`, `Send` (il vit dans le
/// thread du worker).
struct MapWorkerLoader {
    map: HashMap<String, String>,
}

impl WorkerLoader for MapWorkerLoader {
    fn load(&mut self, spec: &str, _referrer: &str) -> Result<String, String> {
        self.map
            .get(spec)
            .cloned()
            .ok_or_else(|| format!("not found: {spec}"))
    }
}

/// Engine + miroir DOM minimal + fabrique de loaders mémoire.
fn engine_with_workers(sources: &[(&str, &str)]) -> Engine {
    let mut e = Engine::new();
    let dom: explorer_jscore::DomRef = Rc::new(std::cell::RefCell::new(Dom::from_html(
        "<html><body></body></html>",
    )));
    e.install_dom(&dom);
    let map: HashMap<String, String> = sources
        .iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect();
    e.set_worker_factory(Arc::new(move || {
        Box::new(MapWorkerLoader { map: map.clone() }) as Box<dyn WorkerLoader>
    }));
    e
}

/// Pompe les messages workers jusqu'à ce que `cond` évalue
/// `true`, ou rend le résultat courant après ~2 s.
fn pump_until(e: &mut Engine, cond: &str) -> String {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if e.pump_workers().is_err() {
            return "PUMP-ERR".to_string();
        }
        let cur = e.eval(cond).map(|v| e.describe(v)).unwrap_or_default();
        if cur == "true" || Instant::now() > deadline {
            return cur;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn worker_echoes_back() {
    let mut e = engine_with_workers(&[(
        "echo.js",
        "self.onmessage = function(ev){ postMessage(ev.data * 2); }",
    )]);
    e.eval("var w = new Worker('echo.js'); w.onmessage = function(ev){ globalThis.__r = ev.data; }; w.postMessage(21)")
        .unwrap();
    assert_eq!(pump_until(&mut e, "globalThis.__r === 42"), "true");
}

#[test]
fn worker_structured_clone_roundtrip() {
    let mut e = engine_with_workers(&[(
        "w.js",
        "self.onmessage = function(ev){ var d = ev.data; postMessage(d.a[1] + ':' + d.b.c + ':' + (d.self === d)); }",
    )]);
    e.eval(
        "var w = new Worker('w.js'); var o = {a:[1,2], b:{c:'x'}}; o.self = o;\
         w.onmessage = function(ev){ globalThis.__r = ev.data; }; w.postMessage(o)",
    )
    .unwrap();
    assert_eq!(pump_until(&mut e, "globalThis.__r === '2:x:true'"), "true");
}

#[test]
fn worker_import_scripts() {
    let mut e = engine_with_workers(&[
        (
            "w.js",
            "importScripts('dep.js'); self.onmessage = function(){ postMessage(depTag()); }",
        ),
        ("dep.js", "function depTag(){ return 'dep-loaded'; }"),
    ]);
    e.eval("var w = new Worker('w.js'); w.onmessage = function(ev){ globalThis.__r = ev.data; }; w.postMessage(0)")
        .unwrap();
    assert_eq!(
        pump_until(&mut e, "globalThis.__r === 'dep-loaded'"),
        "true"
    );
}

#[test]
fn worker_pool_is_bounded() {
    let mut e = engine_with_workers(&[("w.js", "self.onmessage = function(){}")]);
    e.set_worker_config(WorkerConfig {
        max_workers: 1,
        ..WorkerConfig::default()
    });
    e.eval("new Worker('w.js')").unwrap();
    let err = e.eval("new Worker('w.js')").unwrap_err();
    assert!(err.to_string().contains("pool exhausted"), "{err}");
}

#[test]
fn worker_terminate_silences() {
    let mut e =
        engine_with_workers(&[("w.js", "self.onmessage = function(){ postMessage('x'); }")]);
    e.eval(
        "var w = new Worker('w.js'); w.onmessage = function(ev){ globalThis.__r = 1; };\
         w.terminate(); w.postMessage(9)",
    )
    .unwrap();
    // Le message posté après terminate est perdu — le flag reste
    // absent après pompe (`__r` jamais posé → `=== 1` = `false`).
    assert_eq!(pump_until(&mut e, "globalThis.__r === 1"), "false");
}

#[test]
fn worker_uncaught_error_fires_onerror() {
    let mut e = engine_with_workers(&[("w.js", "throw new Error('boom-in-worker')")]);
    e.eval("var w = new Worker('w.js'); w.onerror = function(ev){ globalThis.__r = ev.data; }")
        .unwrap();
    assert_eq!(
        pump_until(
            &mut e,
            "typeof globalThis.__r === 'string' && globalThis.__r.indexOf('boom-in-worker') >= 0"
        ),
        "true"
    );
}

#[test]
fn worker_close_self_terminates() {
    let mut e = engine_with_workers(&[("w.js", "self.onmessage = function(){ close(); };")]);
    e.eval("var w = new Worker('w.js'); w.postMessage(0)")
        .unwrap();
    // Laisse le worker traiter puis mourir — second postMessage
    // absorbé silencieusement.
    std::thread::sleep(Duration::from_millis(50));
    let _ = e.pump_workers();
    e.eval("w.postMessage(1)").unwrap();
    let _ = e.pump_workers();
}

#[test]
fn worker_op_budget_kills_loop() {
    let mut e = engine_with_workers(&[("w.js", "while(true){}")]);
    e.set_op_limit(20_000);
    e.eval("var w = new Worker('w.js'); w.onerror = function(ev){ globalThis.__r = 'dead'; }")
        .unwrap();
    assert_eq!(pump_until(&mut e, "globalThis.__r === 'dead'"), "true");
}

#[test]
fn worker_gc_finalize_terminates() {
    let mut e = engine_with_workers(&[("w.js", "self.onmessage = function(){}")]);
    e.eval("var w = new Worker('w.js');").unwrap();
    e.eval("w = null;").unwrap();
    e.collect();
    // `finalize` a coupé le flot entrant — le thread sort de
    // lui-même ; pompe et nouvelle éval restent saines.
    let _ = e.pump_workers();
    e.eval("globalThis.__still = 1").unwrap();
}

#[test]
fn worker_no_loader_honest_error() {
    let dom: explorer_jscore::DomRef = Rc::new(std::cell::RefCell::new(Dom::from_html(
        "<html><body></body></html>",
    )));
    let mut e = Engine::new();
    e.install_dom(&dom);
    // `Worker` présent pour la feature-detection, jette sans loader.
    let t = e.eval("typeof Worker").unwrap();
    assert_eq!(e.describe(t), "function");
    let err = e.eval("new Worker('x.js')").unwrap_err();
    assert!(err.to_string().contains("no loader"), "{err}");
}
