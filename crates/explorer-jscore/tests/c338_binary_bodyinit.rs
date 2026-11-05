// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! JS-C338 — corps `BodyInit` **binaires** de bout en bout (mur
//! terrain §B-98 du vol 644 : le POST SABR/UMP protobuf de `YouTube`
//! `/watch` mourait en `ToString`/UTF-8 lossy à la frontière
//! transport → `sabr.malformed_config`). `Uint8Array`/`ArrayBuffer`/
//! `DataView`/`Blob` traversent `send`/`fetch`/`Request`/`sendBeacon`
//! en octets bruts ; la déviation JS-C67 (`FormData` → urlencodé) est
//! reconduite.

use explorer_jscore::{Engine, XhrRequest, XhrTransport};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// Transport in-process : enregistre les requêtes, livraison manuelle.
struct StubTransport {
    requests: RefCell<Vec<(u64, XhrRequest)>>,
    next: Cell<u64>,
}

impl XhrTransport for StubTransport {
    fn send(&self, req: &XhrRequest) -> u64 {
        self.next.set(self.next.get() + 1);
        let id = self.next.get();
        self.requests.borrow_mut().push((id, req.clone()));
        id
    }
}

fn engine() -> (Engine, Rc<StubTransport>) {
    let mut e = Engine::new();
    let t = Rc::new(StubTransport {
        requests: RefCell::new(Vec::new()),
        next: Cell::new(0),
    });
    e.install_xhr(t.clone());
    (e, t)
}

fn describe(e: &mut Engine, src: &str) -> String {
    match e.eval(src) {
        Ok(v) => e.describe(v),
        Err(err) => panic!("{src} -> {err}"),
    }
}

/// `xhr.send(Uint8Array)` — octets exacts au transport (avant :
/// join `"0,255,128,127"` par `ToString`).
#[test]
fn xhr_send_typed_array_raw_bytes() {
    let (mut e, t) = engine();
    describe(
        &mut e,
        "var x = new XMLHttpRequest();\
         x.open('POST', '/ump');\
         x.send(new Uint8Array([0x00, 0xFF, 0x80, 0x7F]));",
    );
    let reqs = t.requests.borrow();
    assert_eq!(reqs[0].1.method, "POST");
    assert_eq!(
        reqs[0].1.body.as_deref(),
        Some(&[0x00, 0xFF, 0x80, 0x7F][..])
    );
}

/// `fetch(url, {body: TypedArray})` — même discipline binaire.
#[test]
fn fetch_typed_array_body_raw_bytes() {
    let (mut e, t) = engine();
    describe(
        &mut e,
        "fetch('/sabr', {method: 'POST', body: new Uint8Array([0xDE, 0xAD, 0xBE, 0xEF])});",
    );
    let reqs = t.requests.borrow();
    assert_eq!(
        reqs[0].1.body.as_deref(),
        Some(&[0xDE, 0xAD, 0xBE, 0xEF][..])
    );
}

/// `fetch(url, {body: DataView})` — la **région vue** seule part
/// (`byteOffset`/`byteLength` honorés, pas le buffer entier).
#[test]
fn fetch_dataview_body_region_only() {
    let (mut e, t) = engine();
    describe(
        &mut e,
        "var ab = new ArrayBuffer(8);\
         var u = new Uint8Array(ab);\
         for (var i = 0; i < 8; i++) u[i] = i;\
         fetch('/v', {method: 'POST', body: new DataView(ab, 2, 4)});",
    );
    let reqs = t.requests.borrow();
    assert_eq!(
        reqs[0].1.body.as_deref(),
        Some(&[0x02, 0x03, 0x04, 0x05][..])
    );
}

/// `fetch(url, {body: ArrayBuffer})` — buffer entier brut.
#[test]
fn fetch_arraybuffer_body_raw() {
    let (mut e, t) = engine();
    describe(
        &mut e,
        "var u = new Uint8Array(3); u[0] = 0xF0; u[1] = 0x9F; u[2] = 0x92;\
         fetch('/ab', {method: 'PUT', body: u.buffer});",
    );
    let reqs = t.requests.borrow();
    assert_eq!(reqs[0].1.body.as_deref(), Some(&[0xF0, 0x9F, 0x92][..]));
}

/// `Blob` binaire → corps brut + `Content-Type: blob.type` (avant :
/// UTF-8 lossy — un octet ≥ 0x80 devenait `U+FFFD`).
#[test]
fn xhr_send_binary_blob_raw_and_type() {
    let (mut e, t) = engine();
    describe(
        &mut e,
        "var b = new Blob([new Uint8Array([0xFE, 0x00, 0xBF])],\
                          {type: 'application/x-protobuf'});\
         var x = new XMLHttpRequest();\
         x.open('POST', '/blob');\
         x.send(b);",
    );
    let reqs = t.requests.borrow();
    let r = &reqs[0].1;
    assert_eq!(r.body.as_deref(), Some(&[0xFE, 0x00, 0xBF][..]));
    assert!(r
        .headers
        .iter()
        .any(|(k, v)| k.eq_ignore_ascii_case("content-type") && v == "application/x-protobuf"));
}

/// `fetch(new Request(url, {body: TypedArray}))` — le corps survit
/// à la reconstitution `Request` (avant : `from_utf8_lossy` à la
/// lecture de `req.body`).
#[test]
fn fetch_from_request_preserves_binary_body() {
    let (mut e, t) = engine();
    describe(
        &mut e,
        "var r = new Request('/r', {method: 'POST', body: new Uint8Array([0xC0, 0xFF, 0xEE])});\
         fetch(r);",
    );
    let reqs = t.requests.borrow();
    assert_eq!(reqs[0].1.body.as_deref(), Some(&[0xC0, 0xFF, 0xEE][..]));
}

/// `navigator.sendBeacon(url, TypedArray)` — octets bruts, POST
/// `no-cors`/`include` inchangés.
#[test]
fn send_beacon_typed_array_raw() {
    let (mut e, t) = engine();
    e.install_dom(&std::rc::Rc::new(std::cell::RefCell::new(
        explorer_jscore::Dom::from_html("<html><body></body></html>"),
    )));
    describe(
        &mut e,
        "navigator.sendBeacon('/beacon', new Uint8Array([0x01, 0xFE, 0x80]));",
    );
    let reqs = t.requests.borrow();
    let r = &reqs[0].1;
    assert_eq!(r.method, "POST");
    assert_eq!(r.body.as_deref(), Some(&[0x01, 0xFE, 0x80][..]));
    assert_eq!(r.mode, explorer_jscore::FetchMode::NoCors);
    assert_eq!(r.credentials, explorer_jscore::FetchCredentials::Include);
}

/// Corps chaîne — toujours l'UTF-8 du `ToString` (régression :
/// `'é'` → `0xC3 0xA9`, pas de dérive d'encodage).
#[test]
fn string_body_remains_utf8() {
    let (mut e, t) = engine();
    describe(
        &mut e,
        "var x = new XMLHttpRequest();\
         x.open('POST', '/s');\
         x.send('clé=' + encodeURIComponent('à'));",
    );
    let reqs = t.requests.borrow();
    assert_eq!(reqs[0].1.body.as_deref(), Some("clé=%C3%A0".as_bytes()));
}

/// `FormData`/`URLSearchParams` — urlencodé reconduit (JS-C67) :
/// `URLSearchParams` reste `application/x-www-form-urlencoded;
/// charset=UTF-8`, octets ASCII encodés.
#[test]
fn urlencoded_bodies_still_encode() {
    let (mut e, t) = engine();
    describe(
        &mut e,
        "var fd = new FormData(); fd.append('u', 'lou x'); fd.append('n', '1');\
         var x = new XMLHttpRequest();\
         x.open('POST', '/fd');\
         x.send(fd);\
         var usp = new URLSearchParams(); usp.set('q', 'test me');\
         fetch('/usp', {method: 'POST', body: usp});",
    );
    let reqs = t.requests.borrow();
    assert_eq!(reqs[0].1.body.as_deref(), Some("u=lou+x&n=1".as_bytes()));
    assert_eq!(reqs[1].1.body.as_deref(), Some("q=test+me".as_bytes()));
    assert!(reqs[1]
        .1
        .headers
        .iter()
        .any(|(k, v)| k.eq_ignore_ascii_case("content-type")
            && v == "application/x-www-form-urlencoded;charset=UTF-8"));
}
