// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! §B-104 — vrai transmuxer bitmovin, vrais octets TS.
//!
//! Le worker `transmux_worker.js` est la source exacte servie en
//! vol 725 (extraite de `bitmovinplayer.js` — 91 827 o), le
//! `segment0.ts` un vrai segment HLS téléchargé par le daemon
//! (cache prof725, 153 784 o, sync `0x47`). En vol, la réponse
//! `{action:"data",segment}` a mené à
//! `TypeError: cannot read 'buffer' of Null` dans `Rz`
//! (`e.segment.data` `null` → `ArrayHelper.append` a rendu
//! `null` : `getTypedArrayType`/`instanceof` a refusé une des
//! deux entrées). Ce test rejoue la chaîne parent exacte :
//! `ez` → `postProcessSegment` (`new Uint8Array(e.data)`) →
//! `Ey` (`append(initSegment,data)`) → `Rz` (`data.buffer`).

use explorer_jscore::{Dom, Engine, WorkerLoader};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

const WORKER_JS: &str = "C:/OnionBit-vol/logs/b104/transmux_worker.js";
const SEG_B64: &str = "C:/OnionBit-vol/logs/b104/segment0.b64";
const SEG_AUDIO_B64: &str = "C:/OnionBit-vol/logs/b104/segment0_audio.b64";

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

fn pump_until(e: &mut Engine, cond: &str, secs: u64) -> String {
    let deadline = Instant::now() + Duration::from_secs(secs);
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

#[test]
fn b104_vrai_muxjs_segment_exploitable() {
    let Ok(src) = std::fs::read_to_string(WORKER_JS) else {
        eprintln!("skip: {WORKER_JS} absent");
        return;
    };
    let Ok(b64) = std::fs::read_to_string(SEG_B64) else {
        eprintln!("skip: {SEG_B64} absent");
        return;
    };
    let mut e = Engine::new();
    let dom: explorer_jscore::DomRef = Rc::new(std::cell::RefCell::new(Dom::from_html(
        "<html><body></body></html>",
    )));
    e.install_dom(&dom);
    let mut map = HashMap::new();
    map.insert("tmx.js".to_string(), src);
    e.set_worker_factory(Arc::new(move || {
        Box::new(MapLoader { map: map.clone() }) as Box<dyn WorkerLoader>
    }));

    // Rejeu fidèle du rail bitmovin : `bz`/`cz` envoient
    // `push`/`flush` ; `onmessage` rejoue `ez` (dispatch par
    // `action`) → `postProcessSegment` (`new Uint8Array(e.data)`)
    // → `Ey` (`append(init,data)`) → `Rz` (`data.buffer`).
    let script = format!(
        r"var w = new Worker('tmx.js');
         var msgs = [];
         w.onmessage = function(ev){{
           var t = ev.data;
           if (t.action === 'data') {{
             var seg = t.segment;
             msgs.push('data segkeys:' + Object.keys(seg).join('|'));
             function gt(x){{ return x instanceof Uint8Array ? 'u8' : x instanceof Uint16Array ? 'u16' : x instanceof Uint32Array ? 'u32' : (x===null?'null':typeof x) }}
             globalThis.__pre = 'seg.data brut:' + gt(seg.data) + ' init brut:' + gt(seg.initSegment);
             /* postProcessSegment : e.data = new Uint8Array(e.data) */
             seg.data = new Uint8Array(seg.data);
             globalThis.__post = 'seg.data wrap:' + gt(seg.data);
             /* Ey → makeSegmentSelfInitializing → append */
             function app(a,b){{ var n=(a instanceof Uint8Array&&Uint8Array)||(a instanceof Uint16Array&&Uint16Array)||(a instanceof Uint32Array&&Uint32Array)||null, r=(b instanceof Uint8Array&&Uint8Array)||(b instanceof Uint16Array&&Uint16Array)||(b instanceof Uint32Array&&Uint32Array)||null; if(!n&&!r||n!==r) return null; var o=new n((a.byteLength+b.byteLength)/n.BYTES_PER_ELEMENT); o.set(new n(a),0); o.set(new n(b),a.byteLength/n.BYTES_PER_ELEMENT); return o }}
             seg.data = app(seg.initSegment, seg.data);
             /* Rz → e.segment.data.buffer */
             try {{ globalThis.__res = 'buf:' + (seg.data !== null && seg.data.buffer !== undefined) + ' len:' + (seg.data && seg.data.length) }}
             catch (err) {{ globalThis.__res = 'THROW:' + err.message }}
           }} else if (t.action === 'done') {{
             msgs.push('done');
           }} else {{
             msgs.push(t.action);
           }}
           globalThis.__msgs = msgs.join(' ;; ');
         }};
         var ts = Uint8Array.fromBase64('{b64}');
         w.postMessage({{action:'push', data: ts}}, [ts]);
         w.postMessage({{action:'flush'}});",
        b64 = b64.trim()
    );
    e.eval(&script).unwrap();

    let res = pump_until(&mut e, "globalThis.__res", 20);
    eprintln!("__pre={}", pump_read(&mut e, "globalThis.__pre"));
    eprintln!("__post={}", pump_read(&mut e, "globalThis.__post"));
    eprintln!("__msgs={}", pump_read(&mut e, "globalThis.__msgs"));
    assert!(
        res.starts_with("buf:true"),
        "chaîne Ey/Rz cassée : __res={res} (voir __pre/__post/__msgs ci-dessus)"
    );
}

/// Segment **audio** `segment_0.ts` du même vol : mux.js émet un
/// `data` par piste — en vol `out×2` puis crash. La piste audio
/// est la candidate pour une forme différente (`initSegment`
/// absent → `append→null` → `Rz` sur `data.buffer` de `null`).
#[test]
fn b104_vrai_muxjs_segment_audio() {
    let Ok(src) = std::fs::read_to_string(WORKER_JS) else {
        eprintln!("skip: {WORKER_JS} absent");
        return;
    };
    let Ok(b64) = std::fs::read_to_string(SEG_AUDIO_B64) else {
        eprintln!("skip: {SEG_AUDIO_B64} absent");
        return;
    };
    let mut e = Engine::new();
    let dom: explorer_jscore::DomRef = Rc::new(std::cell::RefCell::new(Dom::from_html(
        "<html><body></body></html>",
    )));
    e.install_dom(&dom);
    let mut map = HashMap::new();
    map.insert("tmx.js".to_string(), src);
    e.set_worker_factory(Arc::new(move || {
        Box::new(MapLoader { map: map.clone() }) as Box<dyn WorkerLoader>
    }));

    let script = format!(
        r"var w = new Worker('tmx.js');
         var n = 0;
         w.onmessage = function(ev){{
           var t = ev.data;
           if (t.action === 'data') {{
             n++;
             var seg = t.segment;
             function gt(x){{ return x instanceof Uint8Array ? 'u8' : x instanceof Uint16Array ? 'u16' : x instanceof Uint32Array ? 'u32' : (x===null?'null':typeof x) }}
             var rep = '#' + n + ' type:' + seg.type + ' init:' + gt(seg.initSegment) + ' data:' + gt(seg.data) + ' keys:' + Object.keys(seg).join('|');
             /* chaîne parent exacte */
             seg.data = new Uint8Array(seg.data);
             function app(a,b){{ var n=(a instanceof Uint8Array&&Uint8Array)||(a instanceof Uint16Array&&Uint16Array)||(a instanceof Uint32Array&&Uint32Array)||null, r=(b instanceof Uint8Array&&Uint8Array)||(b instanceof Uint16Array&&Uint16Array)||(b instanceof Uint32Array&&Uint32Array)||null; if(!n&&!r||n!==r) return null; var o=new n((a.byteLength+b.byteLength)/n.BYTES_PER_ELEMENT); o.set(new n(a),0); o.set(new n(b),a.byteLength/n.BYTES_PER_ELEMENT); return o }}
             seg.data = app(seg.initSegment, seg.data);
             try {{ rep += ' -> ' + (seg.data === null ? 'APPEND-NULL' : 'buf ok len:' + seg.data.length) }}
             catch (err) {{ rep += ' -> THROW:' + err.message }}
             globalThis['__seg' + n] = rep;
             globalThis.__count = n;
           }}
         }};
         var ts = Uint8Array.fromBase64('{b64}');
         w.postMessage({{action:'push', data: ts}}, [ts]);
         w.postMessage({{action:'flush'}});",
        b64 = b64.trim()
    );
    e.eval(&script).unwrap();

    let _ = pump_until(&mut e, "globalThis.__count", 20);
    std::thread::sleep(Duration::from_millis(300));
    let _ = pump_until(
        &mut e,
        "globalThis.__seg2 === undefined ? undefined : 'x'",
        2,
    );
    eprintln!("count={}", pump_read(&mut e, "globalThis.__count"));
    eprintln!("seg1={}", pump_read(&mut e, "globalThis.__seg1"));
    eprintln!("seg2={}", pump_read(&mut e, "globalThis.__seg2"));
    eprintln!("seg3={}", pump_read(&mut e, "globalThis.__seg3"));
}

/// Séquence live exacte du vol 725 : `post×3` puis `out×2`.
/// Deux lectures possibles : `setTimestampOffset+push+flush`
/// (1 data + 1 done) ou `push+push+flush` (2 data). Dans
/// mux.js, `initSegment` n'est émis qu'**une fois par piste
/// et par instance de transmuxer** — un 2ᵉ `data` de la même
/// piste arrive **sans** `initSegment` → `append→null` →
/// `Rz` lit `data.buffer` sur `null` : le crash observé.
/// Ce test pousse 2 segments dans UN worker pour vérifier.
#[test]
fn b104_deux_pushes_meme_worker() {
    let Ok(src) = std::fs::read_to_string(WORKER_JS) else {
        eprintln!("skip: {WORKER_JS} absent");
        return;
    };
    let Ok(seg_vid) = std::fs::read_to_string(SEG_B64) else {
        eprintln!("skip: {SEG_B64} absent");
        return;
    };
    let Ok(seg_audio) = std::fs::read_to_string(SEG_AUDIO_B64) else {
        eprintln!("skip: {SEG_AUDIO_B64} absent");
        return;
    };
    let mut e = Engine::new();
    let dom: explorer_jscore::DomRef = Rc::new(std::cell::RefCell::new(Dom::from_html(
        "<html><body></body></html>",
    )));
    e.install_dom(&dom);
    let mut map = HashMap::new();
    map.insert("tmx.js".to_string(), src);
    e.set_worker_factory(Arc::new(move || {
        Box::new(MapLoader { map: map.clone() }) as Box<dyn WorkerLoader>
    }));

    // Même observation qu'en vol : chaque `data` loguée avec
    // sa forme ; la chaîne `Ey`/`Rz` complète rejouée.
    let script = format!(
        r"var w = new Worker('tmx.js');
         var n = 0;
         w.onmessage = function(ev){{
           var t = ev.data;
           function gt(x){{ return x instanceof Uint8Array ? 'u8' : x instanceof Uint16Array ? 'u16' : x instanceof Uint32Array ? 'u32' : (x===null?'null':typeof x) }}
           if (t.action === 'data') {{
             n++;
             var seg = t.segment;
             var rep = '#' + n + ' type:' + seg.type + ' init:' + gt(seg.initSegment) + ' data:' + gt(seg.data);
             /* postProcessSegment + Ey */
             seg.data = new Uint8Array(seg.data);
             function app(a,b){{ var n=(a instanceof Uint8Array&&Uint8Array)||(a instanceof Uint16Array&&Uint16Array)||(a instanceof Uint32Array&&Uint32Array)||null, r=(b instanceof Uint8Array&&Uint8Array)||(b instanceof Uint16Array&&Uint16Array)||(b instanceof Uint32Array&&Uint32Array)||null; if(!n&&!r||n!==r) return null; var o=new n((a.byteLength+b.byteLength)/n.BYTES_PER_ELEMENT); o.set(new n(a),0); o.set(new n(b),a.byteLength/n.BYTES_PER_ELEMENT); return o }}
             seg.data = app(seg.initSegment, seg.data);
             rep += seg.data === null ? ' -> APPEND-NULL' : ' -> ok len:' + seg.data.length;
             globalThis['__seg' + n] = rep;
             globalThis.__count = n;
           }} else {{
             globalThis.__done = (globalThis.__done||'') + t.action + ' ';
           }}
         }};
         var v = Uint8Array.fromBase64('{bv}');
         var a = Uint8Array.fromBase64('{ba}');
         w.postMessage({{action:'push', data: v}}, [v]);
         w.postMessage({{action:'push', data: a}}, [a]);
         w.postMessage({{action:'flush'}});",
        bv = seg_vid.trim(),
        ba = seg_audio.trim()
    );
    e.eval(&script).unwrap();

    let _ = pump_until(&mut e, "globalThis.__count", 20);
    std::thread::sleep(Duration::from_millis(400));
    eprintln!("count={}", pump_read(&mut e, "globalThis.__count"));
    eprintln!("done={}", pump_read(&mut e, "globalThis.__done"));
    eprintln!("seg1={}", pump_read(&mut e, "globalThis.__seg1"));
    eprintln!("seg2={}", pump_read(&mut e, "globalThis.__seg2"));
    eprintln!("seg3={}", pump_read(&mut e, "globalThis.__seg3"));
}

fn pump_read(e: &mut Engine, expr: &str) -> String {
    e.eval(expr).map(|v| e.describe(v)).unwrap_or_default()
}

/// §B-104 — régression realm : la pompe `run_microtasks` itère
/// tous les ctxs vivants puis appelait `pump_workers` **sous le
/// dernier itéré**. Chez bitmovin, le dernier = iframe ads
/// (sodar/ima) : `wire_decode`/`new_ta` reconstruisaient alors
/// `initSegment`/`data` avec les protos de l'iframe, et
/// `getTypedArrayType` (résolu dans le realm principal via
/// `instanceof`) les refusait → `ArrayHelper.append` → `null`
/// → `TypeError: cannot read 'buffer' of Null` dans `Rz`.
///
/// Repro fidèle : `Worker` créé en ctx principal, un ctx iframe
/// ensuite (comme les ads), puis drain — la chaîne
/// `postProcessSegment` (`new Uint8Array(e.data)`) → `append`
/// → `data.buffer` doit rester dans le realm du Worker.
#[test]
fn b104_realm_worker_livre_sous_ctx_createur() {
    // Worker jouet : forme fil bitmovin exacte (`initSegment`
    // typedarray, `data` arraybuffer, `originalSegment`
    // arraybuffer).
    let wsrc = r"
        self.onmessage = function(ev) {
          if (ev.data.action === 'go') {
            var seg = {type:'video', info:{}, captions:[],
                       captionStreams:{}, metadata:[],
                       initSegment:new Uint8Array(8),
                       data:new ArrayBuffer(16)};
            self.postMessage({action:'data', segment:seg,
                              byteOffset:0, byteLength:16});
            self.postMessage({action:'done', what:'transmuxing',
                              originalSegment:new ArrayBuffer(4)});
          }
        }";
    let mut e = Engine::new();
    let dom: explorer_jscore::DomRef = Rc::new(std::cell::RefCell::new(Dom::from_html(
        "<html><body></body></html>",
    )));
    e.install_dom(&dom);
    let mut map = HashMap::new();
    map.insert("tmx2.js".to_string(), wsrc.to_string());
    e.set_worker_factory(Arc::new(move || {
        Box::new(MapLoader { map: map.clone() }) as Box<dyn WorkerLoader>
    }));

    // Chaîne parent verbatim : `postProcessSegment` → `Ey`
    // (`append`) → `Rz` (`e.segment.data.buffer`).
    e.eval(
        r"var qy = [];
           function getTA(x){ return x instanceof Uint8Array ? Uint8Array : x instanceof Uint16Array ? Uint16Array : x instanceof Uint32Array ? Uint32Array : null }
           function app(t,i){ var n=getTA(t), r=getTA(i); if(!n&&!r||n!==r) return null; var o=new n((t.byteLength+i.byteLength)/n.BYTES_PER_ELEMENT); o.set(new n(t),0); o.set(new n(i),t.byteLength/n.BYTES_PER_ELEMENT); return o }
           var w = new Worker('tmx2.js');
           w.onmessage = function(ev){
             var t = ev.data;
             if (t.action === 'data') {
               var seg = t.segment;
               seg.data = new Uint8Array(seg.data);
               seg.data = app(seg.initSegment, seg.data);
               qy.push({segment: seg});
             } else if (t.action === 'done') {
               try { globalThis.__res = 'buf:' + (qy[0].segment.data.buffer !== undefined) + ':' + qy[0].segment.data.length }
               catch (err) { globalThis.__res = 'THROW:' + err.message }
             }
           };
           w.postMessage({action:'go'});",
    )
    .unwrap();

    // iframe « ads » — la pompe la visite après le principal et
    // livrait les messages worker sous SON realm (le bug).
    let _ads = e.context_new("https://ads.example/x", None);

    let deadline = Instant::now() + Duration::from_secs(10);
    let res = loop {
        let _ = e.run_microtasks();
        // Lecture sous le realm principal (`__res` y vit : le
        // handler s'y résout quel que soit le ctx de livraison).
        e.context_switch(0);
        let cur = pump_read(&mut e, "String(globalThis.__res)");
        if !matches!(cur.as_str(), "" | "undefined" | "null") {
            break cur;
        }
        if Instant::now() > deadline {
            break "TIMEOUT".to_string();
        }
        std::thread::sleep(Duration::from_millis(3));
    };
    assert!(
        res.starts_with("buf:true"),
        "realm mismatch : le handler worker a vu les intrinsics \
         de l'iframe au lieu du principal — {res}"
    );
}
