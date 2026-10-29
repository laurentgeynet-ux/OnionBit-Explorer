// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! MSE-3 — transport segments (ADR-0029) : `appendBuffer`
//! stocke en file FIFO bornée ; `Engine::mse_pull_segments`
//! draine au plus `max` octets en vrac (`(octets, curseur,
//! eos)`) pour le `MseSource` `ByteSource` hôte ; `eos`
//! quand `endOfStream` est appelé **et** la file vide ;
//! `abort`/`remove` abandonnent la file ; `?buffer=N`
//! indexe le `addSourceBuffer`.

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

/// Crée une `MediaSource`, attache-la au `<video>` et rend
/// l'URL `blob:` (chemin player réel).
fn attached_url(e: &mut Engine) -> String {
    e.eval(
        "var ms=new MediaSource();\
         window.__ms=ms;\
         var u=URL.createObjectURL(ms);\
         document.getElementById('v').src=u;\
         u",
    )
    .map(|v| e.describe(v))
    .expect("attach")
}

#[test]
fn mse3_pull_returns_appended_bytes_fifo() {
    let mut e = engine();
    let u = attached_url(&mut e);
    ev(
        &mut e,
        "var sb=__ms.addSourceBuffer('video/mp4');\
         sb.appendBuffer(new Uint8Array([1,2,3]));\
         sb.appendBuffer(new Uint8Array([4,5]));",
    );
    // Le pull draine la file dans l'ordre d'append, borné
    // par `max`.
    let (data, cursor, eos) = e.mse_pull_segments(&u, 0, 4).expect("pull");
    assert_eq!(data, vec![1, 2, 3, 4]);
    assert_eq!(cursor, 4);
    assert!(!eos);
    let (data, cursor, eos) = e.mse_pull_segments(&u, 0, 16).expect("pull2");
    assert_eq!(data, vec![5]);
    assert_eq!(cursor, 5);
    assert!(!eos);
    // File vide, readyState "open" → pas d'eos (le site peut
    // encore appender — le ByteSource attend).
    let (data, _, eos) = e.mse_pull_segments(&u, 0, 16).expect("pull3");
    assert_eq!(data, Vec::<u8>::new());
    assert!(!eos);
}

#[test]
fn mse3_end_of_stream_sets_eos_when_drained() {
    let mut e = engine();
    let u = attached_url(&mut e);
    ev(
        &mut e,
        "var sb=__ms.addSourceBuffer('video/mp4');\
         sb.appendBuffer(new Uint8Array([9,9]));\
         __ms.endOfStream();",
    );
    // `ended` mais file non vide → octets d'abord, eos au
    // vidage seulement.
    let (data, _, eos) = e.mse_pull_segments(&u, 0, 16).expect("pull");
    assert_eq!(data, vec![9, 9]);
    assert!(eos);
}

#[test]
fn mse3_abort_and_remove_drop_queued_bytes() {
    let mut e = engine();
    let u = attached_url(&mut e);
    ev(
        &mut e,
        "var sb=__ms.addSourceBuffer('video/mp4');\
         sb.appendBuffer(new Uint8Array([1,1]));\
         sb.remove(0,1);",
    );
    let (data, _, _) = e.mse_pull_segments(&u, 0, 16).expect("pull");
    assert_eq!(data, Vec::<u8>::new(), "remove aurait dû vider la file");
    ev(
        &mut e,
        "sb.appendBuffer(new Uint8Array([2,2]));\
         sb.abort();",
    );
    let (data, _, _) = e.mse_pull_segments(&u, 0, 16).expect("pull2");
    assert_eq!(data, Vec::<u8>::new(), "abort aurait dû vider la file");
}

#[test]
fn mse3_buffer_index_selects_source_buffer() {
    let mut e = engine();
    let u = attached_url(&mut e);
    // Cas A+V : deux `SourceBuffer` = deux files
    // indépendantes adressées par `?buffer=N`.
    ev(
        &mut e,
        "var v=__ms.addSourceBuffer('video/mp4');\
         var a=__ms.addSourceBuffer('audio/mp4');\
         v.appendBuffer(new Uint8Array([1]));\
         a.appendBuffer(new Uint8Array([2]));",
    );
    let (dv, _, _) = e.mse_pull_segments(&u, 0, 16).expect("video");
    let (da, _, _) = e.mse_pull_segments(&u, 1, 16).expect("audio");
    assert_eq!(dv, vec![1]);
    assert_eq!(da, vec![2]);
    // Index hors rang → None honnête.
    assert!(e.mse_pull_segments(&u, 2, 16).is_none());
}

#[test]
fn mse3_unknown_url_pulls_none() {
    let mut e = engine();
    // URL non enregistrée → `None` (le handler rendra 410).
    assert!(e.mse_pull_segments("blob:null/999", 0, 16).is_none());
    // `blob:` d'un Blob ordinaire → None (pas une MediaSource).
    let u = e
        .eval("URL.createObjectURL(new Blob(['x']))")
        .map(|v| e.describe(v))
        .expect("blob url");
    assert!(e.mse_pull_segments(&u, 0, 16).is_none());
}
