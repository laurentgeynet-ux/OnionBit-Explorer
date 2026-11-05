// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! §B-91 — verrou terrain : un `WorkerGlobalScope` n'a pas de
//! `window`. `builtins::install` posait `window = global` pour
//! tout engine — les bundles worker qui détectent leur
//! environnement par `"undefined"==typeof window` (webworkify,
//! transmuxer bitmovin, hls.js, dash.js) sautaient alors le
//! câblage `onmessage` : messages reçus, dispatch vers un
//! `onmessage` null → silence total côté player (boucle
//! `push`/`flush`/`dispose` infinie, zéro `SourceBuffer`).
//!
//! Le bundle transmuxer réel bitmovin (91 827 o, mux.js
//! embarqué, capturé `blob:bitmovin.com/…`) doit émettre
//! `{action:"data"}` + `{action:"done"}` sur un segment TS
//! authentique — même référence que Node.

use explorer_jscore::{Dom, Engine, WorkerLoader};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

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

fn engine_with_workers(sources: &[(&str, String)]) -> Engine {
    let mut e = Engine::new();
    let dom: explorer_jscore::DomRef = Rc::new(std::cell::RefCell::new(Dom::from_html(
        "<html><body></body></html>",
    )));
    e.install_dom(&dom);
    let map: HashMap<String, String> = sources
        .iter()
        .map(|(k, v)| ((*k).to_string(), v.clone()))
        .collect();
    e.set_worker_factory(Arc::new(move || {
        Box::new(MapWorkerLoader { map: map.clone() }) as Box<dyn WorkerLoader>
    }));
    e
}

fn pump_until(e: &mut Engine, cond: &str, secs: u64) -> String {
    let deadline = Instant::now() + Duration::from_secs(secs);
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

/// Base64 minimal — assez pour injecter le segment dans un
/// `atob` côté JS sans dépendance.
fn b64(data: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(data.len() / 3 * 4 + 4);
    for c in data.chunks(3) {
        let n = (u32::from(c[0]) << 16)
            | (u32::from(c.get(1).copied().unwrap_or(0)) << 8)
            | (u32::from(c.get(2).copied().unwrap_or(0)) << 16);
        s.push(A[(n >> 18) as usize & 63] as char);
        s.push(A[(n >> 12) as usize & 63] as char);
        s.push(if c.len() > 1 {
            A[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        s.push(if c.len() > 2 {
            A[n as usize & 63] as char
        } else {
            '='
        });
    }
    s
}

#[test]
fn worker_scope_has_no_window() {
    // WorkerGlobalScope (spec) : `self`/`globalThis` présents,
    // `window`/`document`/`frames`/`parent`/`top` absents —
    // `typeof x` doit rendre `undefined`, pas `object`.
    let mut e = engine_with_workers(&[(
        "w.js",
        "postMessage((typeof window)+'|'+(typeof document)+'|'+(typeof frames)\
         +'|'+(typeof parent)+'|'+(typeof top)+'|'+(typeof self)\
         +'|'+(typeof globalThis)+'|'+(typeof location)+'|'+(typeof navigator))"
            .to_string(),
    )]);
    e.eval(
        "var w = new Worker('w.js');\
         w.onmessage = function(ev){ globalThis.__r = ''+ev.data; };",
    )
    .unwrap();
    assert_eq!(
        pump_until(&mut e, "typeof globalThis.__r === 'string'", 5),
        "true"
    );
    let r = e.eval("globalThis.__r").map(|v| e.describe(v)).unwrap();
    assert_eq!(
        r,
        "undefined|undefined|undefined|undefined|undefined|object|object|object|object"
    );
}

#[test]
fn transmuxer_bundle_emits_on_real_segment() {
    // Le worker transmuxer réel de bitmovin, nourri d'un
    // segment TS authentique (62 040 o, sync 0x47), doit
    // répondre `data` (segment muxé) puis `done` — référence
    // Node : `[{a:"data",…,db:53880},{a:"done"}]`.
    let bundle = include_str!("data/bitmovin_transmuxer.js").to_string();
    let mut e = engine_with_workers(&[("tmx.js", bundle)]);
    let ts = include_bytes!("data/foreman-h264.ts");
    let enc = b64(ts);
    let script = format!(
        "var bin = atob({enc:?});\
         var ab = new ArrayBuffer(bin.length);\
         var u8 = new Uint8Array(ab);\
         for (var i = 0; i < bin.length; i++) u8[i] = bin.charCodeAt(i);\
         var w = new Worker('tmx.js');\
         var msgs = [];\
         w.onmessage = function(ev) {{\
             var d = ev.data;\
             msgs.push((d && d.action) || typeof d);\
             globalThis.__n = msgs.length;\
         }};\
         w.onerror = function(ev) {{\
             msgs.push('ERR:' + (ev && (ev.message || ev.data)));\
             globalThis.__n = msgs.length;\
         }};\
         w.postMessage({{action:'setTimestampOffset', offset:0}});\
         w.postMessage({{action:'push', data:ab}});\
         w.postMessage({{action:'flush'}});\
         globalThis.__msgs = msgs;",
    );
    e.eval(&script).unwrap();
    // §B-91 : le worker RÉPOND (avant : silence total —
    // `onmessage` jamais câblé). Le message `data` du segment
    // muxé porte des vues typées : sa livraison complète est
    // verrouillée par JS-C337 (`post_message_preserves_typed_views`).
    let r = pump_until(&mut e, "globalThis.__n >= 1", 15);
    let got = e
        .eval("globalThis.__msgs.join('|')")
        .map(|v| e.describe(v))
        .unwrap();
    assert_eq!(r, "true", "reçu seulement: {got}");
    assert!(got.contains("done"), "attendu done: {got}");
}
