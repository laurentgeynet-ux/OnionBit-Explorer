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
            | u32::from(c.get(2).copied().unwrap_or(0));
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
        // `fromBase64` natif — l'injection par boucle
        // `charCodeAt` (62 040 itérations interprétées)
        // prenait > 60 s en debug et butait la deadline
        // boot wall-clock ; mêmes octets, mêmes assertions.
        "var u8 = Uint8Array.fromBase64({enc:?});\
         var ab = u8.buffer;\
         var w = new Worker('tmx.js');\
         var msgs = [];\
         w.onmessage = function(ev) {{\
             var d = ev.data;\
             msgs.push((d && d.action) || typeof d);\
             if (d && d.action === 'data' && d.segment && d.segment.data) {{\
                 globalThis.__db = d.segment.data.byteLength;\
                 globalThis.__shape = (d.segment.data instanceof ArrayBuffer)\
                     + '/' + (d.segment.initSegment instanceof Uint8Array)\
                     + '/' + (d.segment.initSegment && d.segment.initSegment.buffer instanceof ArrayBuffer);\
             }}\
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
    let r = pump_until(&mut e, "globalThis.__n >= 2", 15);
    let got = e
        .eval("globalThis.__msgs.join('|')")
        .map(|v| e.describe(v))
        .unwrap();
    assert_eq!(r, "true", "reçu seulement: {got}");
    assert!(got.contains("data"), "attendu data: {got}");
    assert!(got.contains("done"), "attendu done: {got}");
    // La charge utile doit survivre au `wire_decode` : le
    // segment muxé porte `data` (init+media) non vide.
    let db = e
        .eval("globalThis.__db || 0")
        .map(|v| e.describe(v))
        .unwrap();
    let db: i64 = db.parse().unwrap_or(0);
    assert!(db > 0, "segment data vide");
    // §B-92 — la forme fil doit garder le type de vue : le
    // player lit `initSegment` comme `Uint8Array` et `data`
    // comme `ArrayBuffer` (re-wrappé `new Uint8Array(data)`
    // côté main) — `initSegment` décodé en objet ordinaire
    // donnait `undefined` en aval et tuait la chaîne d'append
    // en silence.
    let shape = e
        .eval("globalThis.__shape || 'unset'")
        .map(|v| e.describe(v))
        .unwrap();
    assert_eq!(
        shape, "true/true/true",
        "data/initSegment n'ont pas survécu au wire_decode"
    );
}

/// §B-92 — aller-retour des vues : `postMessage` worker→main
/// d'un `Uint8Array`/`DataView` reconstruit le même type de vue
/// (buffer cloné, `byteOffset`/`length` préservés) — avant, la
/// vue tombait dans le bras `Obj` du fil et resortait en `{}`.
#[test]
fn post_message_preserves_typed_views() {
    let mut e = engine_with_workers(&[(
        "v.js",
        "onmessage = function(ev) {\
             var u = ev.data;\
             postMessage({got: (u instanceof Uint8Array), n: u.length, v0: u[0], v3: u[3],\
                          b: (u.buffer instanceof ArrayBuffer)});\
         };\
         var dv = new DataView(new Uint8Array([9,8,7,6]).buffer, 1, 2);\
         postMessage({d: dv, i: new Int16Array([300, -2]), f: new Float64Array([1.5])});"
            .to_string(),
    )]);
    e.eval(
        "var w = new Worker('v.js'); var out = [];\
         w.onmessage = function(ev){ out.push(ev.data); globalThis.__n = out.length; };\
         var u = new Uint8Array([5,6,7,8]);\
         w.postMessage(u, [u.buffer]);\
         globalThis.__out = out;",
    )
    .unwrap();
    assert_eq!(pump_until(&mut e, "globalThis.__n >= 2", 5), "true");
    // Le worker poste `{d,i,f}` dès son eval — il peut arriver
    // avant l'écho de `u` : repérer les deux messages par leur
    // contenu plutôt que par leur indice.
    let r = e
        .eval(
            "var m = globalThis.__out;\
             var views = m[0].d ? m[0] : m[1];\
             var echo = m[0].d ? m[1] : m[0];\
             (echo.got + '|' + echo.n + '|' + echo.v0 + '|' + echo.v3 + '|' + echo.b)\
             + '#' + (views.d instanceof DataView) + '|' + views.d.byteOffset + '|'\
             + views.d.byteLength + '|' + views.d.getUint8(0)\
             + '#' + (views.i instanceof Int16Array) + '|' + views.i[0] + '|' + views.i[1]\
             + '#' + (views.f instanceof Float64Array) + '|' + views.f[0]",
        )
        .map(|v| e.describe(v))
        .unwrap();
    assert_eq!(r, "true|4|5|8|true#true|1|2|8#true|300|-2#true|1.5");
}
