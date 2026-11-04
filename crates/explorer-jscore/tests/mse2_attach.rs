// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! MSE-2 — attache `blob:` → `<video>` → `sourceopen`
//! (ADR-0029) : `v.src = URL.createObjectURL(ms)` ou
//! `v.setAttribute("src", u)` attache le `MediaSource`
//! (`readyState "closed"` → `"open"` + `sourceopen`),
//! idempotent au second attachement, sans effet sur un
//! `blob:` non-MSE. La couture offre `mse` côté core est
//! couverte par `media.rs` (marquage scan → `MediaOffered`).

use explorer_jscore::{Dom, Engine, MediaReport, MediaState};
use std::cell::RefCell;
use std::rc::Rc;

const PAGE: &str = r#"<html><body><video id="v"></video></body></html>"#;

fn engine() -> Engine {
    engine_dom().0
}

fn engine_dom() -> (Engine, Rc<RefCell<Dom>>) {
    let mut e = Engine::new();
    let dom = Rc::new(RefCell::new(Dom::from_html(PAGE)));
    e.install_dom(&dom);
    (e, dom)
}

fn ev(e: &mut Engine, src: &str) -> String {
    match e.eval(src) {
        Ok(v) => e.describe(v),
        Err(err) => panic!("{src} -> {err}"),
    }
}

/// Box fMP4 minimale (size + fourcc + payload).
fn bx(tag: [u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut v = u32::try_from(payload.len() + 8)
        .expect("box < 4Gio")
        .to_be_bytes()
        .to_vec();
    v.extend_from_slice(&tag);
    v.extend_from_slice(payload);
    v
}

/// Init fMP4 minimal : `moov` porteur d'un enfant — le
/// walker exige une payload non vide (`end > pos+hdr`).
/// `has_moov` → métadonnées connues → `loadedmetadata`.
fn moov() -> Vec<u8> {
    bx(*b"moov", &bx(*b"free", &[]))
}

#[test]
fn mse2_src_prop_attaches_and_fires_sourceopen() {
    let mut e = engine();
    // Le chemin player réel : `video.src = URL.createObjectURL(ms)`
    // → `readyState open` + `sourceopen` **filé en tâche**
    // (JS-C324) — `n` reste 0 dans l'expression synchrone puis
    // le tir est servi à la frontière de job.
    assert_eq!(
        ev(
            &mut e,
            "var ms=new MediaSource();var n=0;\
             ms.addEventListener('sourceopen',function(){n++});\
             document.getElementById('v').src=URL.createObjectURL(ms);\
             ms.readyState+'|'+n"
        ),
        "open|0"
    );
    assert_eq!(ev(&mut e, "n"), "1");
}

#[test]
fn mse2_attached_media_source_reports_idle_networkstate() {
    let mut e = engine();
    // §B-83/JS-C328 — gate player réel : `v.networkState` lit
    // `NETWORK_IDLE` (1) sur un object URL `MediaSource` — la
    // ressource est locale (appends, pas réseau — spec resource
    // fetch). `3` (`NO_SOURCE`) sur un attach réussi gelait la
    // machine d'état bitmovin au terrain (vol 629 : `net=3`
    // avec `mse:true` posé, `mseBuffers` vide à jamais).
    assert_eq!(
        ev(
            &mut e,
            "var ms=new MediaSource();var v=document.getElementById('v');\
             v.src=URL.createObjectURL(ms);v.networkState"
        ),
        "1"
    );
    // Canaris : `src` non-MSE reste `NO_SOURCE`, absent → `EMPTY`.
    assert_eq!(
        ev(
            &mut e,
            "var v2=document.createElement('video');v2.src='x.mp4';v2.networkState"
        ),
        "3"
    );
    assert_eq!(
        ev(
            &mut e,
            "var v3=document.createElement('video');v3.networkState"
        ),
        "0"
    );
}

#[test]
fn mse2_set_attribute_src_attaches_too() {
    let mut e = engine();
    // Même rail par `setAttribute` — les deux voies de pose
    // de `src` que les players empruntent.
    assert_eq!(
        ev(
            &mut e,
            "var ms=new MediaSource();var n=0;\
             ms.onsourceopen=function(){n++};\
             var v=document.getElementById('v');\
             v.setAttribute('src',URL.createObjectURL(ms));\
             ms.readyState+'|'+n"
        ),
        "open|0"
    );
    assert_eq!(ev(&mut e, "n"), "1");
}

#[test]
fn mse2_attach_is_idempotent() {
    let mut e = engine();
    // Spec : un `MediaSource` attaché ignore un second
    // attachement — pas de double `sourceopen`.
    assert_eq!(
        ev(
            &mut e,
            "var ms=new MediaSource();var n=0;\
             ms.addEventListener('sourceopen',function(){n++});\
             var v=document.getElementById('v');\
             v.src=URL.createObjectURL(ms);\
             v.src=URL.createObjectURL(ms);\
             v.setAttribute('src',URL.createObjectURL(ms));\
             ms.readyState+'|'+n"
        ),
        "open|0"
    );
    assert_eq!(ev(&mut e, "n"), "1");
}

#[test]
fn mse2_sourceopen_reaches_late_listener() {
    let mut e = engine();
    // JS-C324 — pattern canonique MSE (MDN/web.dev/dash.js) :
    // `src=` d'abord, `addEventListener('sourceopen')` ensuite —
    // la spec file l'événement en tâche, il doit survivre à
    // l'enregistrement tardif (un tir synchrone le perdait à
    // jamais — dash.js inerte, terrain §B-83).
    assert_eq!(
        ev(
            &mut e,
            "var ms=new MediaSource();var n=0;\
             document.getElementById('v').src=URL.createObjectURL(ms);\
             ms.addEventListener('sourceopen',function(){n++});\
             ms.readyState"
        ),
        "open"
    );
    // Le listener posé après `src=` a tout de même reçu le tir.
    assert_eq!(ev(&mut e, "n"), "1");
}

#[test]
fn mse2_sourceended_reaches_late_listener() {
    let mut e = engine();
    // JS-C324 — même mise en file pour `sourceended` : le
    // listener posé après `endOfStream()` le reçoit quand même.
    assert_eq!(
        ev(
            &mut e,
            "var ms=new MediaSource();var n=0;\
             document.getElementById('v').src=URL.createObjectURL(ms);\
             ms.endOfStream();\
             ms.onsourceended=function(){n++};\
             ms.readyState"
        ),
        "ended"
    );
    assert_eq!(ev(&mut e, "n"), "1");
}

#[test]
fn mse2_non_mse_blob_url_does_not_attach() {
    let mut e = engine();
    // `blob:` d'un `Blob` ordinaire → aucune attache : le
    // `MediaSource` reste `closed`, pas de `sourceopen`.
    assert_eq!(
        ev(
            &mut e,
            "var ms=new MediaSource();var n=0;\
             ms.addEventListener('sourceopen',function(){n++});\
             document.getElementById('v').src=\
               URL.createObjectURL(new Blob(['x']));\
             ms.readyState+'|'+n"
        ),
        "closed|0"
    );
}

#[test]
fn mse2_end_of_stream_survives_attach() {
    let mut e = engine();
    // `ended` avant attache n'est pas rétrogradé en `open` —
    // le flip ne part que de `closed`.
    assert_eq!(
        ev(
            &mut e,
            "var ms=new MediaSource();ms.endOfStream();\
             document.getElementById('v').src=URL.createObjectURL(ms);\
             ms.readyState"
        ),
        "ended"
    );
}

#[test]
fn mse2_attach_fires_loadstart_then_sourceopen() {
    let mut e = engine();
    // §B-88/JS-C332 — le mur vol 634 : listeners exhaustifs
    // armés par le player puis `src=blob:` → **zéro** event
    // élément mesuré (machine armée à jamais, `mseBuffers:[]`).
    // Spec : la sélection de ressource file `loadstart` sur
    // l'élément — avant `sourceopen` (ordre navigateur), en
    // tâche comme le reste (JS-C324).
    assert_eq!(
        ev(
            &mut e,
            "var L=[];var ms=new MediaSource();var v=document.getElementById('v');\
             v.addEventListener('loadstart',function(){L.push('loadstart')});\
             ms.addEventListener('sourceopen',function(){L.push('sourceopen')});\
             v.src=URL.createObjectURL(ms);L.length"
        ),
        "0"
    );
    assert_eq!(ev(&mut e, "L.join(',')"), "loadstart,sourceopen");
    // Élément sain à l'attache : réseau IDLE (JS-C328),
    // rien encore de bufferisé.
    assert_eq!(ev(&mut e, "v.networkState+'|'+v.readyState"), "1|0");
}

#[test]
fn mse2_ms_duration_fires_element_durationchange() {
    let mut e = engine();
    // §B-88 — `ms.duration` reflétée sur l'élément attaché :
    // le set tire `durationchange` (spec — le player VOD le
    // pose après le manifeste, bitmovin attend ce signal).
    ev(
        &mut e,
        "var L=[];var ms=new MediaSource();var v=document.getElementById('v');\
         v.src=URL.createObjectURL(ms);window.__ms=ms;\
         v.addEventListener('durationchange',function(){L.push('dc:'+v.duration)});'armed'",
    );
    ev(&mut e, "__ms.duration=211.4;'set'");
    assert_eq!(ev(&mut e, "L.join(',')+'|'+v.duration"), "dc:211.4|211.4");
    // Set de même valeur → pas de re-tir ; nouveau set → tir.
    ev(&mut e, "__ms.duration=211.4;__ms.duration=310;'set2'");
    assert_eq!(ev(&mut e, "L.join(',')"), "dc:211.4,dc:310");
}

#[test]
fn mse2_attach_with_preset_duration_fires_durationchange() {
    let mut e = engine();
    // §B-88 — durée connue AVANT l'attache : l'élément la
    // reflète dès le `src=` → `durationchange` filé derrière
    // `loadstart`/`sourceopen` (spec media element attach).
    assert_eq!(
        ev(
            &mut e,
            "var L=[];var ms=new MediaSource();var v=document.getElementById('v');\
             v.addEventListener('loadstart',function(){L.push('loadstart')});\
             v.addEventListener('durationchange',function(){L.push('durationchange')});\
             ms.addEventListener('sourceopen',function(){L.push('sourceopen')});\
             ms.duration=211.4;\
             v.src=URL.createObjectURL(ms);L.length"
        ),
        "0"
    );
    assert_eq!(
        ev(&mut e, "L.join(',')"),
        "loadstart,sourceopen,durationchange"
    );
    assert_eq!(ev(&mut e, "v.duration"), "211.4");
}

#[test]
fn mse2_appends_climb_element_readiness() {
    let mut e = engine();
    // §B-88 — l'échelle `readyState` de l'élément est montée
    // par les appends eux-mêmes (spec MSE : métadonnées =
    // init `moov`, données = plages bufferisées) — sans
    // attendre un claim hôte. C'est le signal `qual>0` que le
    // player attend pour avancer sa machine.
    ev(
        &mut e,
        "var L=[];var ms=new MediaSource();var v=document.getElementById('v');\
         v.src=URL.createObjectURL(ms);window.__sb=ms.addSourceBuffer('video/mp4');\
         ['loadstart','loadedmetadata','loadeddata','canplay',\
          'canplaythrough','progress'].forEach(\
          function(t){v.addEventListener(t,function(){L.push(t)})});'armed'",
    );
    // Init `moov` appendé → `loadedmetadata` (HAVE_METADATA)
    // + `progress` (event élément filé à l'updateend).
    e.eval(&format!(
        "__sb.appendBuffer(new Uint8Array({:?}).buffer);",
        moov()
    ))
    .expect("append init");
    assert_eq!(ev(&mut e, "v.readyState"), "1");
    // Segment média (octets bruts → plage estimée ~[0,6 s],
    // JS-C284) : la tête (pos 0) est couverte →
    // `loadeddata`+`canplay` ; fin 6 s ≥ marge
    // CANPLAYTHROUGH → `canplaythrough`.
    e.eval("__sb.appendBuffer(new ArrayBuffer(64));")
        .expect("append media");
    assert_eq!(ev(&mut e, "v.readyState"), "4");
    assert_eq!(
        ev(&mut e, "L.join(',')"),
        "loadstart,loadedmetadata,progress,loadeddata,canplay,canplaythrough,progress"
    );
    // Appends suivants : `progress` seul re-tire (échelle
    // edge-triggered, jamais polluée — même règle que 182).
    e.eval("__sb.appendBuffer(new ArrayBuffer(64));")
        .expect("append media 2");
    assert_eq!(ev(&mut e, "L.slice(-1)[0]"), "progress");
}

#[test]
fn mse2_play_pends_until_claim_then_resolves() {
    let (mut e, dom) = engine_dom();
    // §B-88 — `play()` sous attache MSE pré-claim : la
    // promesse **pend** (spec : tranche au premier `playing`)
    // au lieu de rejeter `NotSupportedError` — le player lit
    // `paused=false` dès l'intention (flag interne spec).
    ev(
        &mut e,
        "var ms=new MediaSource();var v=document.getElementById('v');\
         v.src=URL.createObjectURL(ms);var R='pending';\
         v.play().then(function(){R='resolved'},function(){R='rejected'});'armed'",
    );
    assert_eq!(ev(&mut e, "R+'|'+v.paused"), "pending|false");
    // Claim hôte → les promesses pendantes migrent et
    // tranchent au premier rapport `playing` (handoff : pas
    // de `loadstart` re-tiré — il a été servi à l'attache).
    let node = dom.borrow().by_id("v").expect("v node");
    e.set_media_claim(node, 7, Some(vec!["avc1".into()]));
    let rep = MediaReport {
        position_ms: 0,
        duration_ms: Some(60_000),
        buffered_ms: 1_000,
        state: MediaState::Playing,
        error: None,
    };
    e.deliver_media_state(node, &rep).expect("playing");
    assert_eq!(ev(&mut e, "R"), "resolved");
    // Canari : hors attache MSE le rejet honnête reste.
    assert_eq!(
        ev(
            &mut e,
            "var v2=document.createElement('video');var R2='pending';\
             v2.play().then(function(){R2='resolved'},function(){R2='rejected'});R2"
        ),
        "pending"
    );
    assert_eq!(ev(&mut e, "R2"), "rejected");
}

#[test]
fn mse2_load_and_pause_preclaim() {
    let mut e = engine();
    // §B-88 — `v.load()` rejoue la sélection de ressource :
    // `loadstart` re-filé (le `unload().then(load+play)` de
    // bitmovin repart sur un cycle de vie propre). `pause()`
    // relâche l'intention `play` (+ event `pause` filé).
    ev(
        &mut e,
        "var L=[];var ms=new MediaSource();var v=document.getElementById('v');\
         ['loadstart','pause'].forEach(\
          function(t){v.addEventListener(t,function(){L.push(t)})});\
         v.src=URL.createObjectURL(ms);'attached'",
    );
    assert_eq!(ev(&mut e, "L.join(',')"), "loadstart");
    ev(&mut e, "v.load();'loaded'");
    assert_eq!(ev(&mut e, "L.join(',')"), "loadstart,loadstart");
    ev(&mut e, "v.play();v.pause();v.paused;'paused'");
    assert_eq!(ev(&mut e, "L.slice(-1)[0]+'|'+v.paused"), "pause|true");
}
