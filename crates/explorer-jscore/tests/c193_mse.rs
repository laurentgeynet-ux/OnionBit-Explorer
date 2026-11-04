// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! JS-C193 — surface MSE honnête sans lecture : `MediaSource`/
//! `SourceBuffer`/`SourceBufferList`/`TimeRanges` réels,
//! `createObjectURL(mediaSource)`, `isTypeSupported` → `""`
//! (aucun conteneur jouable sans transport — ADR-0023).

use explorer_jscore::{Dom, Engine};
use std::cell::RefCell;
use std::rc::Rc;

const PAGE: &str = r#"<html><body><video id="v"></video></body></html>"#;

fn engine() -> Engine {
    let mut e = Engine::new();
    e.install_dom(&Rc::new(RefCell::new(Dom::from_html(PAGE))));
    e
}

fn ev(e: &mut Engine, src: &str) -> String {
    match e.eval(src) {
        Ok(v) => e.describe(v),
        Err(err) => panic!("{src} -> {err}"),
    }
}

#[test]
fn c193_media_source_constructs_and_is_event_target() {
    let mut e = engine();
    assert_eq!(ev(&mut e, "typeof MediaSource"), "function");
    assert_eq!(ev(&mut e, "typeof SourceBuffer"), "function");
    // Feature-detect + instanceof + proto chaîne.
    assert_eq!(
        ev(
            &mut e,
            "var ms=new MediaSource();\
             (ms instanceof MediaSource)+'|'+(typeof ms.addEventListener)+'|'+\
             (typeof ms.addSourceBuffer)+'|'+ms.readyState"
        ),
        "true|function|function|closed"
    );
    // Interfaces non constructibles (spec `Illegal constructor`).
    let r = e.eval("new SourceBuffer()");
    assert!(r.is_err(), "new SourceBuffer() doit jeter");
    let r = e.eval("new SourceBufferList()");
    assert!(r.is_err(), "new SourceBufferList() doit jeter");
    let r = e.eval("new TimeRanges()");
    assert!(r.is_err(), "new TimeRanges() doit jeter");
    // Mais les prototypes sont réels.
    assert_eq!(
        ev(
            &mut e,
            "var sb=ms.addSourceBuffer('video/mp4');\
             (sb instanceof SourceBuffer)+'|'+(typeof SourceBuffer.prototype==='object')"
        ),
        "true|true"
    );
}

#[test]
fn c193_is_type_supported_is_honest_empty() {
    let mut e = engine();
    // `""` pour tout type — jamais de faux succès : le player
    // bascule vers ses fallbacks (`<video src>` → seam hôte).
    assert_eq!(ev(&mut e, "typeof MediaSource.isTypeSupported"), "function");
    assert_eq!(
        ev(
            &mut e,
            "MediaSource.isTypeSupported('video/mp4;codecs=\"avc1.42E01E\"')+'|'+\
             MediaSource.isTypeSupported('video/webm')"
        ),
        "|"
    );
}

#[test]
fn c193_source_buffers_list_and_events() {
    let mut e = engine();
    // Liste vivante indexée + identité cachée + événement
    // `addsourcebuffer` (rail `__et` partagé).
    assert_eq!(
        ev(
            &mut e,
            "var ms=new MediaSource();\
             var seen='';\
             var l0=ms.sourceBuffers;\
             l0.addEventListener('addsourcebuffer',function(ev){seen=ev.type});\
             var sb=ms.addSourceBuffer('video/mp4;codecs=\"avc1\"');\
             (ms.sourceBuffers===l0)+'|'+l0.length+'|'+(l0[0]===sb)+'|'+seen"
        ),
        "true|1|true|addsourcebuffer"
    );
    // `removeSourceBuffer` — splice + `removesourcebuffer`.
    assert_eq!(
        ev(
            &mut e,
            "var seen2='';\
             l0.addEventListener('removesourcebuffer',function(ev){seen2=ev.type});\
             ms.removeSourceBuffer(sb);\
             l0.length+'|'+seen2"
        ),
        "0|removesourcebuffer"
    );
}

#[test]
fn c193_append_buffer_events_and_props() {
    let mut e = engine();
    // `appendBuffer` accepté-borné : séquence updatestart →
    // update → updateend synchrone, `updating` bascule.
    assert_eq!(
        ev(
            &mut e,
            "var ms=new MediaSource();\
             var sb=ms.addSourceBuffer('video/mp4');\
             var log=[];\
             var inU='?';\
             sb.addEventListener('updatestart',function(){log.push('s');inU=sb.updating});\
             sb.addEventListener('update',function(){log.push('u')});\
             sb.addEventListener('updateend',function(){log.push('e')});\
             sb.appendBuffer(new Uint8Array(4));\
             log.join('>')+'|'+inU+'|'+sb.updating+'|'+sb.buffered.length"
        ),
        // JS-C278 — `buffered` estimé : 1 plage après append.
        "s>u>e|true|false|1"
    );
    // Props IDL : mode / timestampOffset / appendWindow /
    // pistes vides.
    assert_eq!(
        ev(
            &mut e,
            "sb.mode='sequence';sb.timestampOffset=1.5;\
             sb.appendWindowStart=0.25;sb.appendWindowEnd=99;\
             sb.mode+'|'+sb.timestampOffset+'|'+sb.appendWindowStart+'|'+\
             sb.appendWindowEnd+'|'+sb.audioTracks.length+'|'+(sb.buffered instanceof TimeRanges)"
        ),
        "sequence|1.5|0.25|99|0|true"
    );
    // abort → `abort`+`updateend`, updating retombé.
    assert_eq!(
        ev(
            &mut e,
            "var alog=[];\
             sb.addEventListener('abort',function(){alog.push('a')});\
             sb.abort();alog.join('')+'|'+sb.updating"
        ),
        "a|false"
    );
}

#[test]
fn js_c278_buffered_estime_offsets_et_remove() {
    let mut e = engine();
    // Estimation ADR-0029 : appends à offsets croissants —
    // le delta réel corrige la plage précédente, la plage
    // courante porte le span observé ; `remove` soustrait.
    assert_eq!(
        ev(
            &mut e,
            "var ms=new MediaSource();\
             var sb=ms.addSourceBuffer('video/mp4');\
             sb.appendBuffer(new Uint8Array(8));\
             var a1=sb.buffered.length+'/'+sb.buffered.start(0)+'/'+sb.buffered.end(0);\
             sb.timestampOffset=6;\
             sb.appendBuffer(new Uint8Array(8));\
             var a2=sb.buffered.start(0)+'/'+sb.buffered.end(0);\
             sb.timestampOffset=12;\
             sb.appendBuffer(new Uint8Array(8));\
             var a3=sb.buffered.end(0)+'/'+sb.buffered.length;\
             sb.remove(3,9);\
             a1+'|'+a2+'|'+a3+'|'+sb.buffered.length+'/'+sb.buffered.start(0)+'/'+sb.buffered.end(0)+'/'+sb.buffered.start(1)+'/'+sb.buffered.end(1)"
        ),
        // 1er append : [0,6) nominal ; offset 6 → delta 6
        // confirmé [0,6)+[6,12) ; offset 12 → [12,18). Après
        // remove(3,9) : [0,3) + [9,18).
        "1/0/6|0/12|18/1|2/0/3/9/18"
    );
}

#[test]
fn js_c278_buffered_append_window_clip_et_abort_conserve() {
    let mut e = engine();
    // Clip append window : `appendWindowEnd` borne la fin
    // estimée. JS-C325 — `abort` est spec : il conserve le
    // média committé (`buffered` survit) et ne reset que
    // l'état parseur — l'ancienne sémantique « abort vide
    // tout » vidait le miroir après un abort de player réel.
    assert_eq!(
        ev(
            &mut e,
            "var ms=new MediaSource();\
             var sb=ms.addSourceBuffer('video/mp4');\
             sb.appendWindowEnd=4;\
             sb.appendBuffer(new Uint8Array(8));\
             var w=sb.buffered.end(0);\
             sb.abort();\
             w+'/'+sb.buffered.length+'/'+sb.buffered.end(0)"
        ),
        "4/1/4"
    );
}

/// JS-C284 — `media.buffered` = union des `SourceBuffer`
/// actifs quand `src` pointe un object-URL `MediaSource`
/// (spec) ; appends à `timestampOffset` constant (timestamps
/// en bande, tfdt fMP4) **étendent** la plage — sinon
/// `media.buffered.end` gèle et un player réel (hls.js
/// `bufferInfo.end`) re-cible un fragment déjà livré.
#[test]
fn js_c284_media_buffered_union_et_sequentiel() {
    let mut e = engine();
    assert_eq!(
        ev(
            &mut e,
            "var ms=new MediaSource();\
             var sb=ms.addSourceBuffer('video/mp4');\
             var sa=ms.addSourceBuffer('audio/mp4');\
             var v=document.getElementById('v');\
             v.src=URL.createObjectURL(ms);\
             var b0=v.buffered.length;\
             sb.appendBuffer(new Uint8Array(8));\
             var b1=v.buffered.length+'/'+v.buffered.end(0);\
             sb.appendBuffer(new Uint8Array(8));\
             var b2=v.buffered.end(0);\
             sa.timestampOffset=20;\
             sa.appendBuffer(new Uint8Array(8));\
             b0+'|'+b1+'|'+b2+'|'+v.buffered.length+'/'+v.buffered.end(0)+'/'+v.buffered.start(1)+'/'+v.buffered.end(1)"
        ),
        // src MSE sans append → 0 ; rail vidéo : [0,6) puis
        // séquentiel [0,12) (offset constant) ; rail audio
        // à offset 20 → [20,26) disjoint ; union = 2 plages.
        "0|1/6|12|2/12/20/26"
    );
}

/// JS-C288 — sous MSE, `video.duration` = `MediaSource
/// .duration` fixée par le site (spec attachement ; hls.js
/// la pose pour la détection VOD) et `video.seekable` =
/// `liveSeekableRange` quand fixée, sinon union `buffered`.
#[test]
fn js_c288_media_duration_et_seekable_sous_mse() {
    let mut e = engine();
    assert_eq!(
        ev(
            &mut e,
            "var ms=new MediaSource();\
             var sb=ms.addSourceBuffer('video/mp4');\
             var v=document.getElementById('v');\
             v.src=URL.createObjectURL(ms);\
             var d0=v.duration;\
             ms.duration=42.5;\
             sb.timestampOffset=3;\
             sb.appendBuffer(new Uint8Array(8));\
             var s0=v.seekable.start(0)+'/'+v.seekable.end(0);\
             ms.setLiveSeekableRange(10,99);\
             d0+'|'+v.duration+'|'+ms.duration+'|'+s0+'|'+v.seekable.start(0)+'/'+v.seekable.end(0)"
        ),
        // duration : NaN avant réglage, 42.5 après —
        // reflet exact côté ms. seekable : plage estimée
        // [3,9) (offset 3, append nominal) tant que live
        // range absente, [10,99) après setLiveSeekableRange.
        "NaN|42.5|42.5|3/9|10/99"
    );
}

/// JS-C285 — `buffered` lu dans les timestamps fMP4 en
/// bande : `mdhd` timescale=1000 dans le `moov` de l'init ;
/// `tfdt`+`trun` des `moof` donnent la vraie plage de
/// chaque segment (1 s chacun). L'init ne produit **pas**
/// de plage (pas de média) et le rail `video.buffered`
/// reflète l'union exacte — un estimateur fixe surestime
/// et fait sauter des fragments au loader hls.js.
#[test]
fn js_c285_buffered_timestamps_fmp4_en_bande() {
    // moov[trak[mdia[mdhd v0 ts=1000]]] — 56 o.
    const INIT: &str = "000000386d6f6f76\
                        000000307472616b\
                        000000286d646961\
                        000000206d646864\
                        000000000000000000000000000003e80000000000000000";
    // moof[traf[tfhd(ddef=500) tfdt(v1,base=0) trun(2×500)]]
    // + mdat — durée totale 1000/1000 = 1 s → [0,1).
    const SEG0: &str = "000000506d6f6f66\
                        0000004874726166\
                        00000014746668640000000800000001000001f4\
                        0000001474666474010000000000000000000000\
                        000000187472756e0000010000000002000001f4000001f4\
                        0000000c6d646174deadbeef";
    // Idem, base tfdt=1000 → [1,2).
    const SEG1: &str = "000000506d6f6f66\
                        0000004874726166\
                        00000014746668640000000800000001000001f4\
                        00000014746664740100000000000000000003e8\
                        000000187472756e0000010000000002000001f4000001f4\
                        0000000c6d646174deadbeef";
    let mut e = engine();
    let src = format!(
        "function H(s){{var a=new Uint8Array(s.length/2);\
         for(var i=0;i<a.length;i++)a[i]=parseInt(s.substr(i*2,2),16);\
         return a}}\
         var ms=new MediaSource();\
         var sb=ms.addSourceBuffer('video/mp4');\
         var v=document.getElementById('v');\
         v.src=URL.createObjectURL(ms);\
         sb.appendBuffer(H('{INIT}').buffer);\
         var b0=sb.buffered.length;\
         sb.appendBuffer(H('{SEG0}').buffer);\
         var b1=sb.buffered.length+'/'+sb.buffered.start(0)+'/'+sb.buffered.end(0);\
         sb.appendBuffer(H('{SEG1}').buffer);\
         b0+'|'+b1+'|'+sb.buffered.end(0)+'|'+v.buffered.end(0)",
    );
    // Init seul → 0 plage ; seg0 → [0,1) ; seg1 → [0,2)
    // fusionné ; union média identique.
    assert_eq!(ev(&mut e, &src), "0|1/0/1|2|2");
}

#[test]
fn c193_create_object_url_registers_media_source() {
    let mut e = engine();
    // `URL.createObjectURL(mediaSource)` — le chemin canonique
    // des players : blob: URL enregistrée, revoke silencieux.
    assert_eq!(
        ev(
            &mut e,
            "var ms=new MediaSource();\
             var u=URL.createObjectURL(ms);\
             (/^blob:/).test(u)+'|'+(typeof u)"
        ),
        "true|string"
    );
    e.eval("URL.revokeObjectURL(u)").unwrap();
    // `Blob` reste sur son rail (aucune régression).
    assert_eq!(
        ev(
            &mut e,
            "var b=new Blob(['x'],{type:'text/plain'});\
             (/^blob:/).test(URL.createObjectURL(b))"
        ),
        "true"
    );
    // TypeError honnête sur le reste.
    let r = e.eval("URL.createObjectURL(42)");
    assert!(r.is_err(), "createObjectURL(42) doit jeter");
}

#[test]
fn c193_end_of_stream_and_duration() {
    let mut e = engine();
    assert_eq!(
        ev(
            &mut e,
            "var ms=new MediaSource();\
             var seen='';\
             ms.addEventListener('sourceended',function(ev){seen=ev.type});\
             ms.duration=42.5;\
             ms.endOfStream();\
             ms.readyState+'|'+ms.duration"
        ),
        "ended|42.5"
    );
    // JS-C324 — `sourceended` est filé en **tâche** (spec :
    // `endOfStream` met le tir en file, jamais synchrone) ;
    // le drain de fin d'eval l'a servi.
    assert_eq!(ev(&mut e, "seen"), "sourceended");
    // `duration = NaN` → TypeError (spec).
    let r = e.eval("ms.duration=NaN");
    assert!(r.is_err(), "duration=NaN doit jeter");
    // Live range retenu → TimeRanges.
    assert_eq!(
        ev(
            &mut e,
            "ms.setLiveSeekableRange(5,30);\
             var r=ms.liveSeekableRange;\
             r.length+'|'+r.start(0)+'|'+r.end(0)"
        ),
        "1|5|30"
    );
}

/// JS-C282 — un listener d'append qui jette ne fige plus le
/// `SourceBuffer` : `updating` retombe, `updateend` part quand
/// même, l'erreur est propagée à l'appelant. Sans ça
/// `blockBuffers` d'hls.js attend `sb.updating===false` à
/// jamais → `endOfStream` jamais appelé (vol hls b-e2e).
#[test]
fn c193_append_listener_throw_keeps_machine() {
    let mut e = engine();
    assert_eq!(
        ev(
            &mut e,
            "var ms=new MediaSource();\
             var sb=ms.addSourceBuffer('video/mp4');\
             var seq='';\
             sb.addEventListener('update',function(){throw new Error('boom')});\
             sb.addEventListener('updateend',function(){seq+='|end;u='+sb.updating});\
             try { sb.appendBuffer(new ArrayBuffer(4)); seq+='|ok'; }\
             catch (err) { seq+='|threw:'+err.message; }\
             seq",
        ),
        "|end;u=false|threw:boom"
    );
}
