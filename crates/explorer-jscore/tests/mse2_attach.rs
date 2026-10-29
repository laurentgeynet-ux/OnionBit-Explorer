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
    // → `readyState open` + `sourceopen` vu par le listener.
    assert_eq!(
        ev(
            &mut e,
            "var ms=new MediaSource();var n=0;\
             ms.addEventListener('sourceopen',function(){n++});\
             document.getElementById('v').src=URL.createObjectURL(ms);\
             ms.readyState+'|'+n"
        ),
        "open|1"
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
        "open|1"
    );
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
        "open|1"
    );
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
