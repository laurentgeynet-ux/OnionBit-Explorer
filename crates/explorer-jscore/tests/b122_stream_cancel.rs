// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! §B-122(e) — `ReadableStream.prototype.cancel()` doit rendre
//! une **promesse** (spec §3.4.4), pas `undefined`. Vol 755
//! explore.org : ky appelle `body.cancel().catch(()=>{})` dans
//! le `finally` de chaque requête — `undefined.catch` jetait
//! `TypeError`, la réponse 200 était rejetée, `TanStack` Query
//! passait en erreur et la page affichait « Camera not found ».
//! Le verrou exige un thenable qui se résout, qui adopte le
//! retour de `source.cancel`, et qui rejette si la source jette.

use explorer_jscore::{Dom, Engine};
use std::cell::RefCell;
use std::rc::Rc;

const PAGE: &str = r"<html><body></body></html>";

fn engine() -> Engine {
    let mut e = Engine::new();
    let dom = Rc::new(RefCell::new(Dom::from_html(PAGE)));
    e.install_dom(&dom);
    e
}

fn describe(e: &mut Engine, src: &str) -> String {
    match e.eval(src) {
        Ok(v) => e.describe(v),
        Err(err) => panic!("{src} -> {err}"),
    }
}

#[test]
fn cancel_returns_thenable_that_resolves() {
    let mut e = engine();
    // La forme exacte du `finally` ky : `.cancel().catch(...)`.
    describe(
        &mut e,
        "var rs = new ReadableStream({});\
         var tag = '';\
         rs.cancel().then(function () { tag += 'ok'; })\
                    .catch(function () { tag += 'rej'; });",
    );
    assert_eq!(describe(&mut e, "typeof rs.cancel()"), "object");
    e.run_microtasks().expect("microtasks");
    assert_eq!(describe(&mut e, "tag"), "ok");
}

#[test]
fn cancel_invokes_source_cancel_and_adopts() {
    let mut e = engine();
    describe(
        &mut e,
        "var called = '';\
         var rs = new ReadableStream({\
           cancel: function (reason) { called = 'c:' + reason; return Promise.resolve(7); }\
         });\
         var tag = '';\
         rs.cancel('bye').then(function (v) { tag = 'r:' + v; });",
    );
    e.run_microtasks().expect("microtasks");
    // `source.cancel` appelé avec la raison ; la promesse
    // retournée se résout (undefined — la valeur du thenable
    // source n'est pas propagée par spec pour cancel).
    assert_eq!(describe(&mut e, "called"), "c:bye");
    assert_eq!(describe(&mut e, "tag"), "r:undefined");
}

#[test]
fn cancel_rejects_when_source_throws() {
    let mut e = engine();
    describe(
        &mut e,
        "var rs = new ReadableStream({\
           cancel: function () { throw new Error('boom'); }\
         });\
         var tag = '';\
         rs.cancel().then(function () { tag = 'ok'; })\
                    .catch(function (r) { tag = 'rej:' + r.message; });",
    );
    e.run_microtasks().expect("microtasks");
    assert_eq!(describe(&mut e, "tag"), "rej:boom");
}

#[test]
fn reader_cancel_still_thenable() {
    let mut e = engine();
    describe(
        &mut e,
        "var rs = new ReadableStream({});\
         var rd = rs.getReader();\
         var tag = '';\
         rd.cancel().then(function () { tag = 'ok'; });",
    );
    e.run_microtasks().expect("microtasks");
    assert_eq!(describe(&mut e, "tag"), "ok");
}
