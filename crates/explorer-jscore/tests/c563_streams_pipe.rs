// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! JS-C563 / §B-153 — `ReadableStream.prototype.pipeTo` doit
//! être **asynchrone** : la promesse rendue reste pendante tant
//! que la source n'a pas fermé, et les chunks `enqueue`d
//! *après* l'appel continuent d'être écrits dans la
//! destination. Verbatim vol 851/853 (n26, shopify —
//! bootstrap `__reactRouterContext` React Router) :
//! `TypeError: pipeTo: source stalled without pull` — le drain
//! synchrone mourait dès que les chunks arrivaient par un
//! autre job que `pull`.
//! Spec WHATWG Streams §3.4.7 : `pipeTo` verrouille la source,
//! lit en boucle via des requêtes de lecture, écrit dans
//! l'ordre, `close` → `writer.close` + résolution, `error` →
//! `writer.abort` + rejet. Options honorées :
//! `preventClose`/`preventAbort`.

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

/// Fixture commune : source avec controller capturé + sink à
/// journal (`got` reçoit chunks, `CLOSED`, `ABORT:<r>`).
const FIXTURE: &str = r"
    var ctl;
    var rs = new ReadableStream({ start: function (c) { ctl = c; } });
    var got = [];
    var ws = new WritableStream({
        write: function (x) { got.push(x); },
        close: function () { got.push('CLOSED'); },
        abort: function (r) { got.push('ABORT:' + r); }
    });
";

#[test]
fn pipe_to_late_chunks_no_stall() {
    let mut e = engine();
    // Repro §B-153 : `pipeTo` PUIS `enqueue` depuis la suite du
    // script — l'ancien drain synchrone rejetait
    // « source stalled without pull ».
    describe(
        &mut e,
        &format!(
            "{FIXTURE}
             var tag = '';
             var pp = rs.pipeTo(ws);
             pp.then(function () {{ tag += 'ok'; }},
                     function (r) {{ tag += 'rej:' + r; }});
             ctl.enqueue('a');
             ctl.enqueue('b');"
        ),
    );
    // Pipe ouvert — la promesse n'est pas encore résolue.
    assert_eq!(describe(&mut e, "got.join(',')"), "a,b");
    describe(&mut e, "ctl.close();");
    e.run_microtasks().expect("microtasks");
    assert_eq!(describe(&mut e, "got.join(',')"), "a,b,CLOSED");
    assert_eq!(describe(&mut e, "tag"), "ok");
}

#[test]
fn pipe_to_buffered_source_resolves() {
    let mut e = engine();
    // Source déjà alimentée + fermée à l'appel — chemin rapide :
    // tout est écrit, `close` appelé, promesse résolue.
    describe(
        &mut e,
        &format!(
            "{FIXTURE}
             ctl.enqueue('x');
             ctl.enqueue('y');
             ctl.close();
             var tag = '';
             rs.pipeTo(ws).then(function () {{ tag = 'ok'; }},
                                function (r) {{ tag = 'rej'; }});"
        ),
    );
    assert_eq!(describe(&mut e, "got.join(',')"), "x,y,CLOSED");
    e.run_microtasks().expect("microtasks");
    assert_eq!(describe(&mut e, "tag"), "ok");
}

#[test]
fn pipe_to_locks_source() {
    let mut e = engine();
    describe(
        &mut e,
        &format!("{FIXTURE} var before = rs.locked; rs.pipeTo(ws);"),
    );
    assert_eq!(describe(&mut e, "before"), "false");
    assert_eq!(describe(&mut e, "rs.locked"), "true");
    // Spec : un second lecteur est refusé tant que le pipe vit.
    assert_eq!(
        describe(
            &mut e,
            "(function(){ try { rs.getReader(); return 'no'; } catch (x) { return 'threw'; } })()"
        ),
        "threw"
    );
}

#[test]
fn pipe_to_error_aborts_and_rejects() {
    let mut e = engine();
    describe(
        &mut e,
        &format!(
            "{FIXTURE}
             var tag = '';
             rs.pipeTo(ws).then(function () {{ tag = 'ok'; }},
                                function (r) {{ tag = 'rej:' + r; }});
             ctl.enqueue('z');
             ctl.error('boom');"
        ),
    );
    e.run_microtasks().expect("microtasks");
    assert_eq!(describe(&mut e, "got.join(',')"), "z,ABORT:boom");
    assert_eq!(describe(&mut e, "tag"), "rej:boom");
}

#[test]
fn pipe_to_prevent_close_and_prevent_abort() {
    let mut e = engine();
    // `preventClose` : fin de source → pas de `writer.close`,
    // mais la promesse du pipe se résout quand même.
    describe(
        &mut e,
        &format!(
            "{FIXTURE}
             var tag = '';
             rs.pipeTo(ws, {{ preventClose: true }})
               .then(function () {{ tag = 'ok'; }},
                     function (r) {{ tag = 'rej'; }});
             ctl.enqueue('p');
             ctl.close();"
        ),
    );
    e.run_microtasks().expect("microtasks");
    assert_eq!(describe(&mut e, "got.join(',')"), "p");
    assert_eq!(describe(&mut e, "tag"), "ok");
    // `preventAbort` : erreur source → pas d'`abort`, rejet.
    describe(
        &mut e,
        "var ctl2;\
         var rs2 = new ReadableStream({ start: function (c) { ctl2 = c; } });\
         var got2 = [];\
         var ws2 = new WritableStream({\
             write: function (x) { got2.push(x); },\
             close: function () { got2.push('CLOSED'); },\
             abort: function (r) { got2.push('ABORT'); }\
         });\
         var tag2 = '';\
         rs2.pipeTo(ws2, { preventAbort: true })\
            .then(function () { tag2 = 'ok'; },\
                  function (r) { tag2 = 'rej'; });\
         ctl2.error('nope');",
    );
    e.run_microtasks().expect("microtasks");
    assert_eq!(describe(&mut e, "got2.join(',')"), "");
    assert_eq!(describe(&mut e, "tag2"), "rej");
}

#[test]
fn pipe_to_pull_source_drains() {
    let mut e = engine();
    // Source à `pull` : queue vide → pull appelé, chunks
    // enqueued en synchrone → drain complet jusqu'à `close`.
    describe(
        &mut e,
        "var n = 0;\
         var rs = new ReadableStream({\
             pull: function (c) {\
                 n += 1;\
                 if (n < 3) { c.enqueue('p' + n); } else { c.close(); }\
             }\
         });\
         var got = [];\
         var ws = new WritableStream({\
             write: function (x) { got.push(x); },\
             close: function () { got.push('CLOSED'); }\
         });\
         var tag = '';\
         rs.pipeTo(ws).then(function () { tag = 'ok'; });",
    );
    e.run_microtasks().expect("microtasks");
    e.run_microtasks().expect("microtasks");
    assert_eq!(describe(&mut e, "got.join(',')"), "p1,p2,CLOSED");
    assert_eq!(describe(&mut e, "tag"), "ok");
}

#[test]
fn pipe_through_returns_readable_and_pipes() {
    let mut e = engine();
    // `pipeThrough` : `readable` du transform rendu de suite,
    // le pipe continue en tâche de fond.
    describe(
        &mut e,
        &format!(
            "{FIXTURE}
             var ts = new TransformStream({{
                 transform: function (x, c) {{ c.enqueue('T:' + x); }}
             }});
             var out = rs.pipeThrough(ts);
             var rd = out.getReader();
             var seen = '';
             function pump() {{
                 rd.read().then(function (r) {{
                     if (!r.done) {{ seen += r.value + ';'; pump(); }}
                     else {{ seen += 'DONE'; }}
                 }});
             }}
             pump();
             ctl.enqueue('m');
             ctl.close();"
        ),
    );
    e.run_microtasks().expect("microtasks");
    e.run_microtasks().expect("microtasks");
    assert_eq!(describe(&mut e, "seen"), "T:m;DONE");
}
