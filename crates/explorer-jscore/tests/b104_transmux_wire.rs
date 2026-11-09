// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! §B-104 — rail wire du transmuxer bitmovin.
//!
//! En vol (725), la chaîne complète `post→onmsg→out→in` a été
//! observée saine, puis le handler de réponse a jeté
//! `TypeError: cannot read 'buffer' of Null` à
//! `new Segment(e.segment.data.buffer, …)` : `e.segment.data`
//! est `Null` parce que `ArrayHelper.append(initSegment, data)`
//! rend `null` quand `getTypedArrayType` (`instanceof
//! Uint8Array|16|32`) échoue sur un des deux arguments.
//! Ce test rejoue le round-trip exact des deux côtés du rail :
//! `push` parent→worker avec `Uint8Array` (+ liste de transfert),
//! réponse worker→parent `{data:{initSegment,data,info}}` — et
//! vérifie que les typed arrays arrivent en vrais `TypedArray`
//! (`instanceof` + `.buffer` exploitables).

use explorer_jscore::{Dom, Engine, WorkerLoader};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

struct MapLoader {
    map: HashMap<String, String>,
}

impl WorkerLoader for MapLoader {
    fn load(&mut self, spec: &str, _referrer: &str) -> Result<String, String> {
        self.map
            .get(spec)
            .cloned()
            .ok_or_else(|| format!("not found: {spec}"))
    }
}

fn engine_with(src: &str) -> Engine {
    let mut e = Engine::new();
    let dom: explorer_jscore::DomRef = Rc::new(std::cell::RefCell::new(Dom::from_html(
        "<html><body></body></html>",
    )));
    e.install_dom(&dom);
    let mut map = HashMap::new();
    map.insert("tmx.js".to_string(), src.to_string());
    e.set_worker_factory(Arc::new(move || {
        Box::new(MapLoader { map: map.clone() }) as Box<dyn WorkerLoader>
    }));
    e
}

fn pump_until(e: &mut Engine, cond: &str) -> String {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if e.pump_workers().is_err() {
            return "PUMP-ERR".to_string();
        }
        let cur = e.eval(cond).map(|v| e.describe(v)).unwrap_or_default();
        if cur != "undefined" || Instant::now() > deadline {
            return cur;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// Le worker reçoit `{action:"push",data:<Uint8Array>}` — il
/// rapporte la forme fil reçue (`instanceof` côté worker),
/// transmux factice, et reposte le segment façon mux.js :
/// `{action:"data", data:{initSegment,data,info,captions}}`.
#[test]
fn b104_segment_typed_arrays_survivent_le_rail() {
    let mut e = engine_with(
        "self.onmessage = function(ev){\
           var d = ev.data;\
           self.__recv = (d && d.data instanceof Uint8Array) + ':' + (d && d.data && d.data.length);\
           var seg = { initSegment: new Uint8Array([9,8,7]),\
                       data: d.data,\
                       info: { type: 'video', trackId: 1, baseMediaDecodeTime: 90000, startsWithKeyframe: true, timelineStartInfo: {minSegmentPts: 0} },\
                       captions: [] };\
           postMessage({ action: 'data', data: seg, originalSegment: null });\
           postMessage({ action: 'done', originalSegment: null });\
         }",
    );
    e.eval(
        "var w = new Worker('tmx.js');\
         w.onmessage = function(ev){\
           var t = ev.data;\
           if (t.action === 'data') {\
             var seg = t.data;\
             /* rejeu Ey : makeSegmentSelfInitializing = append(init,data) */\
             function gt(x){ return x instanceof Uint8Array ? Uint8Array : x instanceof Uint16Array ? Uint16Array : x instanceof Uint32Array ? Uint32Array : null }\
             function append(a,b){ var n=gt(a),r=gt(b); if(!n&&!r||n!==r) return null; var o=new n((a.byteLength+b.byteLength)/n.BYTES_PER_ELEMENT); o.set(new n(a),0); o.set(new n(b), a.byteLength/n.BYTES_PER_ELEMENT); return o }\
             seg.data = append(seg.initSegment, seg.data);\
             /* rejeu Rz : e.segment.data.buffer */\
             globalThis.__res = 'u8init:'+(seg.initSegment instanceof Uint8Array)+' u8data:'+(seg.data instanceof Uint8Array)+' buf:'+(seg.data && seg.data.buffer !== undefined)+' len:'+(seg.data && seg.data.length);\
           }\
         };\
         w.postMessage({action:'push', data: new Uint8Array([1,2,3,4])}, [new Uint8Array([1,2,3,4])]);",
    )
    .unwrap();
    let r = pump_until(&mut e, "globalThis.__res");
    assert_eq!(r, "u8init:true u8data:true buf:true len:7");
}

/// Le payload `push` vu côté worker — `ev.data.data` doit être
/// un vrai `Uint8Array` là-bas aussi (le transmuxer lit ses
/// octets ; une forme dégradée produirait une sortie non-TA).
#[test]
fn b104_push_arrive_en_typed_array_cote_worker() {
    let mut e = engine_with(
        "self.onmessage = function(ev){\
           var d = ev.data && ev.data.data;\
           postMessage({ u8: d instanceof Uint8Array, len: d && d.length, first: d && d[0] });\
         }",
    );
    e.eval(
        "var w = new Worker('tmx.js');\
         w.onmessage = function(ev){ globalThis.__res = 'u8:'+ev.data.u8+' len:'+ev.data.len+' first:'+ev.data.first };\
         w.postMessage({action:'push', data: new Uint8Array([7,8,9])});",
    )
    .unwrap();
    let r = pump_until(&mut e, "globalThis.__res");
    assert_eq!(r, "u8:true len:3 first:7");
}
