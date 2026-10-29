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
        "s>u>e|true|false|0"
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
             ms.readyState+'|'+ms.duration+'|'+seen"
        ),
        "ended|42.5|sourceended"
    );
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
