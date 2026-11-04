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
