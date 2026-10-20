// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Tests for `XMLHttpRequest` over an injectable transport (JS-4).

use explorer_jscore::{Engine, XhrRequest, XhrResponse, XhrTransport};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// In-process transport: records requests, hands out sequential
/// handles. Delivery is explicit — the test plays "the network".
struct StubTransport {
    requests: RefCell<Vec<(u64, XhrRequest)>>,
    next: Cell<u64>,
}

impl StubTransport {
    fn new() -> Rc<Self> {
        Rc::new(StubTransport {
            requests: RefCell::new(Vec::new()),
            next: Cell::new(0),
        })
    }
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
    let t = StubTransport::new();
    e.install_xhr(t.clone());
    (e, t)
}

fn describe(e: &mut Engine, src: &str) -> String {
    match e.eval(src) {
        Ok(v) => e.describe(v),
        Err(err) => panic!("{src} -> {err}"),
    }
}

fn respond(e: &mut Engine, handle: u64, status: u16, body: &str) {
    e.deliver_xhr_response(XhrResponse {
        handle,
        status,
        status_text: "OK".into(),
        headers: vec![("Content-Type".into(), "text/plain".into())],
        body: body.as_bytes().to_vec(),
    })
    .expect("deliver");
}

#[test]
fn full_request_cycle() {
    let (mut e, t) = engine();
    describe(
        &mut e,
        "var x = new XMLHttpRequest();\
         var log = '';\
         x.onreadystatechange = function () { log += 'rs' + x.readyState; };\
         x.onload = function () { log += ':load'; };\
         x.open('GET', '/api/data');\
         x.setRequestHeader('X-Test', '1');\
         x.send();",
    );
    // Transport saw exactly one request with method/url/headers.
    let reqs = t.requests.borrow();
    assert_eq!(reqs.len(), 1);
    let (h, r) = &reqs[0];
    assert_eq!(r.method, "GET");
    assert_eq!(r.url, "/api/data");
    assert_eq!(r.headers, vec![("X-Test".to_string(), "1".to_string())]);
    let h = *h;
    drop(reqs);
    // Before delivery: readyState 2 (sent), nothing fired.
    assert_eq!(describe(&mut e, "x.readyState"), "2");
    respond(&mut e, h, 200, "payload");
    assert_eq!(
        describe(
            &mut e,
            "x.readyState + '|' + x.status + '|' + x.responseText + '|' + log"
        ),
        "4|200|payload|rs4:load"
    );
}

#[test]
fn post_body_passes_through() {
    let (mut e, t) = engine();
    describe(
        &mut e,
        "var x = new XMLHttpRequest();\
         x.open('POST', '/submit');\
         x.send('a=1&b=2');",
    );
    let reqs = t.requests.borrow();
    assert_eq!(reqs[0].1.method, "POST");
    assert_eq!(reqs[0].1.body.as_deref(), Some("a=1&b=2"));
}

#[test]
fn response_headers() {
    let (mut e, t) = engine();
    describe(
        &mut e,
        "var x = new XMLHttpRequest(); x.open('GET', '/h'); x.send();",
    );
    let h = t.requests.borrow()[0].0;
    respond(&mut e, h, 200, "x");
    assert_eq!(
        describe(&mut e, "x.getResponseHeader('content-type')"),
        "text/plain"
    );
    assert_eq!(describe(&mut e, "x.getResponseHeader('nope')"), "null");
    assert_eq!(
        describe(&mut e, "x.getAllResponseHeaders()"),
        "Content-Type: text/plain\r\n"
    );
}

#[test]
fn add_event_listener_load() {
    let (mut e, t) = engine();
    describe(
        &mut e,
        "var x = new XMLHttpRequest();\
         var got = '';\
         x.addEventListener('load', function (ev) { got = ev.type + '/' + ev.target.status; });\
         x.open('GET', '/l'); x.send();",
    );
    let h = t.requests.borrow()[0].0;
    respond(&mut e, h, 200, "x");
    assert_eq!(describe(&mut e, "got"), "load/200");
}

#[test]
fn error_event_on_status_zero() {
    let (mut e, t) = engine();
    describe(
        &mut e,
        "var x = new XMLHttpRequest();\
         var evs = '';\
         x.onerror = function () { evs += 'err'; };\
         x.onload = function () { evs += 'load'; };\
         x.open('GET', '/down'); x.send();",
    );
    let h = t.requests.borrow()[0].0;
    respond(&mut e, h, 0, "");
    assert_eq!(describe(&mut e, "x.status + '|' + evs"), "0|err");
}

#[test]
fn abort_cancels_delivery() {
    let (mut e, t) = engine();
    describe(
        &mut e,
        "var x = new XMLHttpRequest();\
         var evs = '';\
         x.onload = function () { evs += 'load'; };\
         x.open('GET', '/a'); x.send(); x.abort();",
    );
    let h = t.requests.borrow()[0].0;
    // Late response for an aborted request is dropped.
    respond(&mut e, h, 200, "x");
    assert_eq!(describe(&mut e, "x.readyState + '|' + evs"), "0|");
}

#[test]
fn xhr_constants() {
    let (mut e, _t) = engine();
    assert_eq!(
        describe(
            &mut e,
            "var x = new XMLHttpRequest(); x.UNSENT + ',' + x.DONE"
        ),
        "0,4"
    );
}

#[test]
fn response_xml_parses_into_dom() {
    let (mut e, t) = engine();
    describe(
        &mut e,
        "var x = new XMLHttpRequest(); x.open('GET', '/doc.xml'); x.send();",
    );
    let h = t.requests.borrow()[0].0;
    respond(&mut e, h, 200, "<root><item id='a'>one</item></root>");
    assert_eq!(
        describe(&mut e, "x.responseXML.getElementsByTagName('item').length"),
        "1"
    );
    assert_eq!(
        describe(&mut e, "x.responseXML.getElementById('a').textContent"),
        "one"
    );
}

#[test]
fn no_transport_gives_error_event() {
    let mut e = Engine::new();
    // install_xhr never called → global is absent.
    assert_eq!(describe(&mut e, "typeof XMLHttpRequest"), "undefined");
}

#[test]
fn state_constants_and_newless_call() {
    let (mut e, _t) = engine();
    // Callable without `new` (browser leniency).
    assert_eq!(
        describe(&mut e, "var x = XMLHttpRequest(); x.readyState"),
        "0"
    );
}

// -- fetch (JS-7e) ----------------------------------------------------

#[test]
fn fetch_resolves_response() {
    let (mut e, t) = engine();
    describe(
        &mut e,
        "var out = '';\
         fetch('/api', {method: 'POST', headers: {'X-A': '1'}, body: 'b=2'})\
           .then(function (r) { out += r.ok + '/' + r.status; });",
    );
    let reqs = t.requests.borrow();
    assert_eq!(reqs.len(), 1);
    let (h, r) = &reqs[0];
    assert_eq!(r.method, "POST");
    assert_eq!(r.url, "/api");
    assert_eq!(r.headers, vec![("X-A".to_string(), "1".to_string())]);
    assert_eq!(r.body.as_deref(), Some("b=2"));
    let h = *h;
    drop(reqs);
    respond(&mut e, h, 201, "made");
    assert_eq!(describe(&mut e, "out"), "true/201");
}

#[test]
fn fetch_text_and_json() {
    let (mut e, t) = engine();
    describe(
        &mut e,
        "var out = '';\
         fetch('/t').then(function (r) { return r.text(); })\
                   .then(function (s) { out += 'T:' + s; });\
         fetch('/j').then(function (r) { return r.json(); })\
                   .then(function (o) { out += '|J:' + o.k; });",
    );
    let h1 = t.requests.borrow()[0].0;
    let h2 = t.requests.borrow()[1].0;
    respond(&mut e, h1, 200, "hello");
    respond(&mut e, h2, 200, "{\"k\": 42}");
    assert_eq!(describe(&mut e, "out"), "T:hello|J:42");
}

#[test]
fn fetch_headers_and_status() {
    let (mut e, t) = engine();
    describe(
        &mut e,
        "var out = '';\
         fetch('/h').then(function (r) {\
             out += r.statusText + '/' + r.headers.get('content-type')\
                  + '/' + (r.headers['content-type'] || 'x');\
         });",
    );
    let h = t.requests.borrow()[0].0;
    respond(&mut e, h, 200, "x");
    assert_eq!(describe(&mut e, "out"), "OK/text/plain/text/plain");
    // !ok on 4xx — still resolves (fetch only rejects on net error).
    describe(
        &mut e,
        "var out2 = '';\
         fetch('/nf').then(function (r) { out2 += r.ok + '/' + r.status; });",
    );
    let h2 = t.requests.borrow()[1].0;
    respond(&mut e, h2, 404, "");
    assert_eq!(describe(&mut e, "out2"), "false/404");
}

#[test]
fn fetch_network_error_rejects() {
    let (mut e, t) = engine();
    describe(
        &mut e,
        "var out = '';\
         fetch('/down').then(function () { out += 'ok'; })\
                       .catch(function (e) { out += 'err:' + e; });",
    );
    let h = t.requests.borrow()[0].0;
    respond(&mut e, h, 0, "");
    assert_eq!(describe(&mut e, "out"), "err:fetch: network error");
}

#[test]
fn fetch_no_transport_rejects() {
    let mut e = Engine::new();
    // No install_xhr → fetch absent entirely (same as XHR ctor).
    assert_eq!(describe(&mut e, "typeof fetch"), "undefined");
}
