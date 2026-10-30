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

// ------------------------------------------------------------------
// JS-C66 — `Headers`/`Request`/`Response` constructibles + mixin Body
// ------------------------------------------------------------------

#[test]
fn c66_headers_ctor_methods_iter() {
    let (mut e, _t) = engine();
    // Record init + lecture insensible à la casse.
    assert_eq!(
        describe(
            &mut e,
            "var h = new Headers({'X-A': '1', 'Content-Type': 'text/plain'});\
             h.get('x-a') + '|' + h.has('CONTENT-TYPE') + '|' + h.get('missing')"
        ),
        "1|true|null"
    );
    // append combine par ", ", set remplace, delete supprime.
    assert_eq!(
        describe(&mut e, "h.append('x-a', '2'); h.get('x-a')"),
        "1, 2"
    );
    assert_eq!(
        describe(
            &mut e,
            "h.set('x-a', '3'); h.get('x-a') + '|' + h.has('x-a')"
        ),
        "3|true"
    );
    describe(&mut e, "h.delete('x-a'); var gone = h.has('x-a');");
    assert_eq!(describe(&mut e, "gone"), "false");
    // Init séquence de paires + normalisation des valeurs.
    assert_eq!(
        describe(&mut e, "new Headers([['a','1'],['b',' 2 ']]).get('b')"),
        "2"
    );
    // Nom malformé → TypeError.
    assert!(e.eval("new Headers().append('bad name', 'x')").is_err());
    assert!(e.eval("new Headers().set('ok', 'a\nb')").is_err());
    // `instanceof` + `Object.keys` vides (props non énumérables).
    assert_eq!(describe(&mut e, "h instanceof Headers"), "true");
    assert_eq!(describe(&mut e, "Object.keys(h).length"), "0");
    // Itération triée par nom (spec) + forEach(v, n, h).
    assert_eq!(
        describe(
            &mut e,
            "var s = ''; var h2 = new Headers({b:'2', a:'1'});\
             for (var kv of h2) s += kv[0] + '=' + kv[1] + ';';\
             h2.forEach(function (v, n) { s += n; }); s"
        ),
        "a=1;b=2;ab"
    );
    // `new Headers(autre)` copie.
    assert_eq!(describe(&mut e, "new Headers(h2).get('a')"), "1");
}

#[test]
fn c66_headers_set_cookie_never_combined() {
    let (mut e, _t) = engine();
    describe(
        &mut e,
        "var h = new Headers();\
         h.append('Set-Cookie', 'a=1; Expires=Wed, 21 Oct 2026');\
         h.append('set-cookie', 'b=2');",
    );
    // `getSetCookie` = valeurs brutes (la virgule d'Expires est
    // préservée — jamais recombinée), `get` rend la première.
    assert_eq!(
        describe(
            &mut e,
            "h.getSetCookie().join(' / ') + '|' + h.get('set-cookie')"
        ),
        "a=1; Expires=Wed, 21 Oct 2026 / b=2|a=1; Expires=Wed, 21 Oct 2026"
    );
}

#[test]
fn c66_request_ctor_and_clone() {
    let (mut e, _t) = engine();
    // Surface minimale — pas de `page_url` → URL brute conservée.
    assert_eq!(
        describe(
            &mut e,
            "var r = new Request('/api', {method: 'post', headers: {'X-A': '1'}, body: 'p=1'});\
             r.url + '|' + r.method + '|' + r.headers.get('x-a') + '|'\
             + r.mode + '|' + r.credentials + '|' + r.redirect + '|'\
             + (r instanceof Request) + '|' + r.bodyUsed + '|' + (r.body instanceof ReadableStream)"
        ),
        "/api|POST|1|cors|same-origin|follow|true|false|true"
    );
    // URL résolue contre la base de la page.
    e.set_page_url("https://ex.test/dir/page");
    assert_eq!(
        describe(&mut e, "new Request('sub/x').url"),
        "https://ex.test/dir/sub/x"
    );
    // GET/HEAD + corps → TypeError (spec).
    assert!(e.eval("new Request('/g', {body: 'x'})").is_err());
    // `new Request(r)` reprend les champs (headers copiés).
    assert_eq!(
        describe(
            &mut e,
            "var r2 = new Request(r); r2.method + '|' + r2.headers.get('x-a')"
        ),
        "POST|1"
    );
    // `clone()` — headers copiés (mutation de l'originale n'y
    // touche pas), `bodyUsed` relu à false.
    assert_eq!(
        describe(
            &mut e,
            "var c = r.clone(); r.headers.set('x-a', '9');\
             c.headers.get('x-a') + '|' + (c instanceof Request)"
        ),
        "1|true"
    );
    // Signal frais par défaut.
    assert_eq!(describe(&mut e, "r.signal.aborted"), "false");
}

#[test]
fn c66_response_ctor_and_statics() {
    let (mut e, _t) = engine();
    assert_eq!(
        describe(
            &mut e,
            "var res = new Response('hi', {status: 404, statusText: 'Nope', headers: {'X-B': '2'}});\
             res.status + '|' + res.statusText + '|' + res.ok + '|'\
             + res.type + '|' + res.headers.get('x-b') + '|'\
             + (res instanceof Response)"
        ),
        "404|Nope|false|default|2|true"
    );
    // status hors [200,599] → RangeError.
    assert!(e.eval("new Response('x', {status: 99})").is_err());
    assert!(e.eval("new Response('x', {status: 600})").is_err());
    // Statics : error / redirect / json.
    assert_eq!(
        describe(
            &mut e,
            "var er = Response.error(); er.type + '|' + er.status + '|' + er.ok"
        ),
        "error|0|false"
    );
    assert_eq!(
        describe(
            &mut e,
            "var rd = Response.redirect('/dest', 307);\
             rd.status + '|' + rd.headers.get('location')"
        ),
        "307|/dest"
    );
    assert!(e.eval("Response.redirect('/x', 200)").is_err());
    assert_eq!(
        describe(
            &mut e,
            "var rj = Response.json({a: 1});\
             rj.headers.get('content-type') + '|' + rj.status"
        ),
        "application/json|200"
    );
}

#[test]
fn c66_body_mixin() {
    let (mut e, t) = engine();
    // Mixin complet sur la réponse livrée.
    describe(
        &mut e,
        "var out = '';\
         fetch('/x').then(function (r) {\
             out += (r.body instanceof ReadableStream) + '|' + r.bodyUsed;\
             return r.text();\
         }).then(function (s) { out += '|' + s; });",
    );
    let h = t.requests.borrow()[0].0;
    respond(&mut e, h, 200, "payload");
    assert_eq!(describe(&mut e, "out"), "true|false|payload");
    // Seconde consommation → TypeError rejeté (promesse).
    describe(
        &mut e,
        "var out2 = '';\
         var rr;\
         fetch('/y').then(function (r) { rr = r; return r.text(); })\
                   .then(function () { return rr.text(); })\
                   .catch(function (er) { out2 += er.name + ':' + rr.bodyUsed; });",
    );
    let h2 = t.requests.borrow()[1].0;
    respond(&mut e, h2, 200, "x");
    assert_eq!(describe(&mut e, "out2"), "TypeError:true");
    // arrayBuffer / blob / formData.
    describe(
        &mut e,
        "var out3 = '';\
         fetch('/ab').then(function (r) { return r.arrayBuffer(); })\
                     .then(function (ab) { out3 += 'ab' + ab.byteLength; });\
         fetch('/bl').then(function (r) { return r.blob(); })\
                     .then(function (b) { out3 += '|bl' + b.size + '/' + b.type; });\
         fetch('/fd').then(function (r) { return r.formData(); })\
                     .then(function (f) { out3 += '|fd' + f.get('a') + f.getAll('b').length + f.has('c'); });",
    );
    let h3 = t.requests.borrow()[2].0;
    let h4 = t.requests.borrow()[3].0;
    let h5 = t.requests.borrow()[4].0;
    respond(&mut e, h3, 200, "abc");
    respond(&mut e, h4, 200, "abcd");
    respond(&mut e, h5, 200, "a=1&b=2&b=3");
    assert_eq!(describe(&mut e, "out3"), "ab3|bl4/text/plain|fd12false");
}

#[test]
fn c66_fetch_request_input() {
    let (mut e, t) = engine();
    // `fetch(new Request(...))` — url/method/headers/body repris.
    describe(
        &mut e,
        "var out = '';\
         fetch(new Request('/rq', {method: 'put', headers: {'X-R': '7'}, body: 'q=8'}))\
           .then(function (r) { out += (r instanceof Response) + '/' + r.status + '/' + r.url; });",
    );
    {
        let reqs = t.requests.borrow();
        let (h, r) = &reqs[0];
        assert_eq!(r.method, "PUT");
        assert_eq!(r.url, "/rq");
        assert_eq!(r.headers, vec![("x-r".to_string(), "7".to_string())]);
        assert_eq!(r.body.as_deref(), Some("q=8"));
        let h = *h;
        drop(reqs);
        respond(&mut e, h, 200, "ok");
    }
    assert_eq!(describe(&mut e, "out"), "true/200//rq");
    // `opts` surcharge le Request d'entrée (méthode normalisée).
    describe(
        &mut e,
        "fetch(new Request('/rq2', {method: 'get'}), {method: 'post', body: 'z'});",
    );
    let reqs = t.requests.borrow();
    assert_eq!(reqs[1].1.method, "POST");
    assert_eq!(reqs[1].1.body.as_deref(), Some("z"));
    // `{headers: new Headers(...)}` accepté en opts.
    drop(reqs);
    describe(
        &mut e,
        "fetch('/hh', {headers: new Headers({'X-H': '9'})});",
    );
    let reqs = t.requests.borrow();
    assert_eq!(
        reqs[2].1.headers,
        vec![("x-h".to_string(), "9".to_string())]
    );
}

// ------------------------------------------------------------------
// JS-C67 — `FormData` dans fetch/XHR + bidirectionnel USP
// ------------------------------------------------------------------

#[test]
fn c67_fetch_body_formdata_urlencoded() {
    let (mut e, t) = engine();
    // `fetch(url, {body: FormData})` → corps urlencodé +
    // Content-Type posé (déviation documentée vs multipart).
    describe(
        &mut e,
        "var fd = new FormData();\
         fd.append('u', 'lou x'); fd.append('n', '1');\
         fetch('/login', {method: 'post', body: fd});",
    );
    let reqs = t.requests.borrow();
    let r = &reqs[0].1;
    assert_eq!(r.body.as_deref(), Some("u=lou+x&n=1"));
    assert!(
        r.headers
            .iter()
            .any(|(k, v)| k == "Content-Type" && v == "application/x-www-form-urlencoded"),
        "headers: {:?}",
        r.headers
    );
    drop(reqs);
    // Content-Type fourni → non écrasé.
    describe(
        &mut e,
        "var fd2 = new FormData(); fd2.append('a', '1');\
         fetch('/x', {body: fd2, headers: {'Content-Type': 'text/custom'}});",
    );
    let reqs = t.requests.borrow();
    let r = &reqs[1].1;
    assert!(
        r.headers
            .iter()
            .any(|(k, v)| k == "Content-Type" && v == "text/custom"),
        "headers: {:?}",
        r.headers
    );
    drop(reqs);
}

#[test]
fn c67_xhr_send_formdata_urlencoded() {
    let (mut e, t) = engine();
    describe(
        &mut e,
        "var fd = new FormData(); fd.append('k', 'v v');\
         var x = new XMLHttpRequest();\
         x.open('POST', '/f'); x.send(fd);",
    );
    let reqs = t.requests.borrow();
    let r = &reqs[0].1;
    assert_eq!(r.body.as_deref(), Some("k=v+v"));
    assert!(
        r.headers
            .iter()
            .any(|(k, v)| k == "Content-Type" && v == "application/x-www-form-urlencoded"),
        "headers: {:?}",
        r.headers
    );
}

#[test]
fn c67_response_formdata_is_real() {
    let (mut e, t) = engine();
    describe(
        &mut e,
        "var out = '';\
         fetch('/f').then(function (r) { return r.formData(); })\
                   .then(function (fd) {\
                       out += (fd instanceof FormData) + '|' + fd.get('a')\
                            + '|' + fd.getAll('b').join(',') + '|' + fd.has('zz');\
                   });",
    );
    let h = t.requests.borrow()[0].0;
    respond(&mut e, h, 200, "a=1&b=2&b=3");
    assert_eq!(describe(&mut e, "out"), "true|1|2,3|false");
    // `new FormData()` sans DOM marche aussi côté fetch-only.
    assert_eq!(
        describe(&mut e, "typeof FormData + '|' + new FormData().has('x')"),
        "function|false"
    );
}

#[test]
fn c67_request_response_bodyinit() {
    let (mut e, _t) = engine();
    // `new Request(url, {body: FormData})` → corps urlencodé + CT
    // sur les `Headers` du Request (extract spec).
    assert_eq!(
        describe(
            &mut e,
            "var fd = new FormData(); fd.append('a', '1 2');\
             var rq = new Request('/p', {method: 'POST', body: fd});\
             rq.headers.get('content-type')"
        ),
        "application/x-www-form-urlencoded"
    );
    // Le corps est consommé par `fetch(rq)` → urlencodé.
    describe(&mut e, "fetch(rq);");
    // `new Response(fd)` → corps texte lisible + CT.
    assert_eq!(
        describe(
            &mut e,
            "var rs = new Response(fd);\
             rs.headers.get('content-type') + '|' + rs.bodyUsed"
        ),
        "application/x-www-form-urlencoded|false"
    );
    describe(
        &mut e,
        "var rtxt = ''; rs.text().then(function (t) { rtxt = t; });",
    );
    assert_eq!(describe(&mut e, "rtxt"), "a=1+2");
    // `new Response(usp)` → urlencoded + CT `charset=UTF-8` (spec).
    assert_eq!(
        describe(
            &mut e,
            "new Response(new URLSearchParams('x=1')).headers.get('content-type')"
        ),
        "application/x-www-form-urlencoded;charset=UTF-8"
    );
}

#[test]
fn c67_fetch_xhr_body_usp() {
    let (mut e, t) = engine();
    describe(
        &mut e,
        "fetch('/u', {method: 'PUT', body: new URLSearchParams('a=1&b=x y')});",
    );
    describe(
        &mut e,
        "var x = new XMLHttpRequest(); x.open('POST', '/u');\
         x.send(new URLSearchParams('q=test me'));",
    );
    let reqs = t.requests.borrow();
    let (r0, r1) = (&reqs[0].1, &reqs[1].1);
    assert_eq!(r0.body.as_deref(), Some("a=1&b=x+y"));
    assert!(
        r0.headers
            .iter()
            .any(|(k, v)| k == "Content-Type"
                && v == "application/x-www-form-urlencoded;charset=UTF-8"),
        "headers: {:?}",
        r0.headers
    );
    assert_eq!(r1.body.as_deref(), Some("q=test+me"));
    assert!(
        r1.headers
            .iter()
            .any(|(k, v)| k == "Content-Type"
                && v == "application/x-www-form-urlencoded;charset=UTF-8"),
        "headers: {:?}",
        r1.headers
    );
}

// ---------------------------------------------------------------
// JS-C70 — Blob/File/FileReader + URL.createObjectURL
// ---------------------------------------------------------------

#[test]
fn c70_blob_ctor_parts_meta() {
    let (mut e, _t) = engine();
    // Parts : string, ArrayBuffer, typed array, Blob imbriqué.
    assert_eq!(
        describe(
            &mut e,
            "var inner = new Blob(['ZZ']);\
             var b = new Blob(['ab', new Uint8Array([99, 100]), inner], {type: 'Text/CSV'});\
             b.size + '|' + b.type + '|' + (b instanceof Blob) + '|' +\
             Object.prototype.toString.call(b)"
        ),
        "6|text/csv|true|[object Blob]"
    );
    // Slice : indices relatifs spec + contentType.
    assert_eq!(
        describe(
            &mut e,
            "var s = b.slice(1, -1, 'TEXT/PLAIN'); s.size + '|' + s.type"
        ),
        "4|text/plain"
    );
    // blob.text()/arrayBuffer() → promesses.
    describe(
        &mut e,
        "var out = '';\
         s.text().then(function (t) { out += t; });\
         s.arrayBuffer().then(function (ab) { out += ':' + ab.byteLength; });",
    );
    assert_eq!(describe(&mut e, "out"), "bcdZ:4");
    // Type invalide (hors ASCII printable) → "".
    assert_eq!(
        describe(&mut e, "new Blob([], {type: 'x\ty'}).type === ''"),
        "true"
    );
}

#[test]
fn c70_file_ctor_meta() {
    let (mut e, _t) = engine();
    assert_eq!(
        describe(
            &mut e,
            "var f = new File(['hi'], 'a.txt', {type: 'TEXT/PLAIN', lastModified: 42});\
             f.name + '|' + f.size + '|' + f.type + '|' + f.lastModified + '|' +\
             f.webkitRelativePath + '|' + (f instanceof File) + '|' + (f instanceof Blob)"
        ),
        "a.txt|2|text/plain|42||true|true"
    );
    // lastModified par défaut ≈ maintenant (> 0).
    describe(&mut e, "var g = new File([], 'n');");
    assert_eq!(describe(&mut e, "g.lastModified > 0"), "true");
    // `new File(['x'])` → TypeError (2 args requis).
    assert_eq!(
        describe(
            &mut e,
            "try { new File(['x']); 'no' } catch (err) { 'threw' }"
        ),
        "threw"
    );
}

#[test]
fn c70_filereader_events_result() {
    let (mut e, _t) = engine();
    describe(
        &mut e,
        "var f = new File(['hello'], 'h.txt', {type: 'text/plain'});\
         var r = new FileReader();\
         var log = 'rs' + r.readyState;\
         r.addEventListener('progress', function (ev) { log += ':p' + ev.loaded + '/' + ev.total; });\
         r.onload = function (ev) { log += ':load:' + (ev.target === r) + ':' + r.result; };\
         r.onloadend = function () { log += ':end:rs' + r.readyState; };\
         r.readAsText(f); log += ':rs' + r.readyState;",
    );
    // Livré au checkpoint : events en ordre + result + DONE.
    assert_eq!(
        describe(&mut e, "log"),
        "rs0:rs1:p5/5:load:true:hello:end:rs2"
    );
    // readAsDataURL → `data:<type>;base64,…`.
    describe(
        &mut e,
        "var r2 = new FileReader(); var du = '';\
         r2.onload = function () { du = r2.result; };\
         r2.readAsDataURL(f);",
    );
    assert_eq!(describe(&mut e, "du"), "data:text/plain;base64,aGVsbG8=");
    // readAsArrayBuffer → ArrayBuffer des octets.
    describe(
        &mut e,
        "var r3 = new FileReader(); var ab = 0;\
         r3.onload = function () { ab = r3.result.byteLength; };\
         r3.readAsArrayBuffer(f);",
    );
    assert_eq!(describe(&mut e, "ab"), "5");
    // readAsBinaryString → byte → code unit.
    describe(
        &mut e,
        "var r4 = new FileReader(); var bs = '';\
         r4.onload = function () { bs = r4.result; };\
         r4.readAsBinaryString(new Blob([new Uint8Array([65, 66])]));",
    );
    assert_eq!(describe(&mut e, "bs"), "AB");
    // Constantes statiques + instance.
    assert_eq!(
        describe(
            &mut e,
            "FileReader.EMPTY + ',' + FileReader.DONE + ',' + r.DONE + ',' +\
             (r instanceof FileReader)"
        ),
        "0,2,2,true"
    );
    // abort() pendant une lecture → abort + loadend, result null.
    describe(
        &mut e,
        "var r5 = new FileReader(); var al = '';\
         r5.onabort = function () { al += 'abort'; };\
         r5.onloadend = function () { al += ':end:' + (r5.result === null); };\
         r5.readAsText(f); r5.abort();",
    );
    assert_eq!(describe(&mut e, "al"), "abort:end:true");
}

#[test]
fn c70_object_url_fetch() {
    let (mut e, _t) = engine();
    describe(
        &mut e,
        "var b = new Blob(['payload'], {type: 'text/x-test'});\
         var u = URL.createObjectURL(b);",
    );
    // `blob:<origin>/<n>` — harnais sans page → origine "null".
    assert_eq!(describe(&mut e, "u.indexOf('blob:null/')"), "0");
    // `fetch(blob:)` résout en local — sans transport.
    describe(
        &mut e,
        "var st = 0; var ct = ''; var txt = '';\
         fetch(u).then(function (r) {\
             st = r.status; ct = r.headers.get('content-type');\
             return r.text();\
         }).then(function (t) { txt = t; });",
    );
    assert_eq!(
        describe(&mut e, "st + '|' + ct + '|' + txt"),
        "200|text/x-test|payload"
    );
    // revoke → l'URL ne résout plus (promise rejetée).
    describe(
        &mut e,
        "URL.revokeObjectURL(u); var rej = '';\
         fetch(u).then(function () { rej = 'ok'; }, function (m) { rej = 'err'; });",
    );
    assert_eq!(describe(&mut e, "rej"), "err");
    // createObjectURL(non-blob) → TypeError.
    assert_eq!(
        describe(
            &mut e,
            "try { URL.createObjectURL({}); 'no' } catch (err) { 'threw' }"
        ),
        "threw"
    );
}

#[test]
fn c70_body_blob_and_res_blob() {
    let (mut e, t) = engine();
    // `body: Blob` → octets UTF-8 + CT = blob.type.
    describe(
        &mut e,
        "fetch('/u', {method: 'POST', body: new Blob(['contenu'], {type: 'text/plain'})});",
    );
    // `xhr.send(File)` → même conversion.
    describe(
        &mut e,
        "var x = new XMLHttpRequest(); x.open('POST', '/u2');\
         x.send(new File(['fdata'], 'f.txt', {type: 'text/csv'}));",
    );
    {
        let reqs = t.requests.borrow();
        let (r0, r1) = (&reqs[0].1, &reqs[1].1);
        assert_eq!(r0.body.as_deref(), Some("contenu"));
        assert!(r0
            .headers
            .iter()
            .any(|(k, v)| k == "Content-Type" && v == "text/plain"));
        assert_eq!(r1.body.as_deref(), Some("fdata"));
        assert!(r1
            .headers
            .iter()
            .any(|(k, v)| k == "Content-Type" && v == "text/csv"));
    }
    // `new Request`/`new Response` avec corps Blob.
    assert_eq!(
        describe(
            &mut e,
            "new Response(new Blob(['x'], {type: 'text/a'})).headers.get('content-type')"
        ),
        "text/a"
    );
    // `res.blob()` → vrai Blob (proto, slice, instanceof).
    describe(
        &mut e,
        "fetch('/dl').then(function (r) { globalThis.rb = r.blob(); });",
    );
    respond(&mut e, 3, 200, "streamed");
    describe(
        &mut e,
        "var rok = ''; rb.then(function (bl) {\
             rok = (bl instanceof Blob) + '|' + bl.size + '|' + bl.type + '|' +\
             Object.prototype.toString.call(bl); });",
    );
    assert_eq!(describe(&mut e, "rok"), "true|8|text/plain|[object Blob]");
}

#[test]
fn c70_formdata_file_filename() {
    let (mut e, t) = engine();
    describe(
        &mut e,
        "var fd = new FormData();\
         fd.append('plain', 'v');\
         fd.append('file', new File(['x'], 'doc.txt', {type: 'text/plain'}));\
         fd.append('raw', new Blob(['y']));\
         fd.append('named', new Blob(['z']), 'renamed.bin');\
         fd.set('s', new Blob(['w']), 'set.bin');\
         fetch('/up', {method: 'POST', body: fd});",
    );
    // Corps urlencodé (limite documentée) — File → filename,
    // Blob nu → "blob", 3e arg → File renommé.
    {
        let reqs = t.requests.borrow();
        let body = reqs[0].1.body.clone().unwrap_or_default();
        assert_eq!(
            body,
            "plain=v&file=doc.txt&raw=blob&named=renamed.bin&s=set.bin"
        );
    }
    // `fd.get` garde la valeur File (proto conservé).
    assert_eq!(
        describe(
            &mut e,
            "(fd.get('file') instanceof File) + '|' + fd.get('file').name + '|' +\
             (fd.get('named') instanceof File) + '|' + fd.get('named').name + '|' +\
             fd.get('named').size"
        ),
        "true|doc.txt|true|renamed.bin|1"
    );
}

#[test]
fn c70_structured_clone_blob_file() {
    let (mut e, _t) = engine();
    assert_eq!(
        describe(
            &mut e,
            "var f = new File(['data'], 'n.txt', {type: 'text/plain', lastModified: 7});\
             var c = structuredClone(f);\
             (c instanceof File) + '|' + c.name + '|' + c.lastModified + '|' +\
             c.size + '|' + (c !== f)"
        ),
        "true|n.txt|7|4|true"
    );
}

// ---------------------------------------------------------------
// JS-C71 — ReadableStream/WritableStream/TransformStream
// ---------------------------------------------------------------

#[test]
fn c71_readable_start_enqueue_read() {
    let (mut e, _t) = engine();
    // `start` enqueué au construct ; `read()` en promesses
    // `{value, done}` spec.
    assert_eq!(
        describe(
            &mut e,
            "var rs = new ReadableStream({start: function (c) {\
                 c.enqueue('a'); c.enqueue('b'); c.close(); }});\
             (rs instanceof ReadableStream) + '|' + rs.locked"
        ),
        "true|false"
    );
    describe(
        &mut e,
        "var rd = rs.getReader();\
         var log = (rs.locked ? 'L' : '-') + '|';\
         rd.read().then(function (r) { log += r.value + r.done + ';'; });\
         rd.read().then(function (r) { log += r.value + r.done + ';'; });\
         rd.read().then(function (r) { log += r.done + ';'; });",
    );
    assert_eq!(describe(&mut e, "log"), "L|afalse;bfalse;true;");
    // `releaseLock` → nouveau reader possible.
    assert_eq!(
        describe(
            &mut e,
            "rd.releaseLock(); rs.locked + '|' + typeof rs.getReader"
        ),
        "false|function"
    );
}

#[test]
fn c71_readable_pull_and_pending() {
    let (mut e, _t) = engine();
    // Queue vide → `pull(controller)` invoqué ; encore vide →
    // la promesse `read()` attend le prochain enqueue/close.
    describe(
        &mut e,
        "var i = 0; var seq = '';\
         var rs2 = new ReadableStream({pull: function (c) {\
             seq += 'p';\
             c.enqueue(++i);\
             if (i >= 2) c.close(); }});\
         var r2 = rs2.getReader();\
         var got = '';\
         r2.read().then(function (r) { got += r.value; });\
         r2.read().then(function (r) { got += ',' + r.value; });\
         r2.read().then(function (r) { got += ',' + r.done; });",
    );
    // `pull` appelé pour chaque read (backpressure mémoire).
    assert_eq!(describe(&mut e, "got"), "1,2,true");
    // `Symbol.asyncIterator` → `for await`-compatible : chaque
    // `next()` = promesse `{value,done}` (stream frais — un
    // stream locked refuse l'itérateur, spec).
    describe(
        &mut e,
        "var rs3 = new ReadableStream({start: function (c) {\
             c.enqueue('z'); c.close(); }});\
         var it = rs3[Symbol.asyncIterator](); var av = '';\
         it.next().then(function (r) { av = r.value + r.done; });",
    );
    assert_eq!(
        describe(&mut e, "typeof it !== 'undefined' ? av : '?'"),
        "zfalse"
    );
}

#[test]
fn c71_writable_sink_write_close() {
    let (mut e, _t) = engine();
    describe(
        &mut e,
        "var sunk = '';\
         var ws = new WritableStream({\
             write: function (c) { sunk += c; },\
             close: function () { sunk += '|closed'; }});\
         var w = ws.getWriter();\
         var st = ws.locked + '|';\
         w.write('x').then(function () { st += 'w'; });\
         w.close().then(function () { st += 'c'; });\
         w.closed.then(function () { st += 'C'; });",
    );
    assert_eq!(describe(&mut e, "st + '|' + sunk"), "true|wcC|x|closed");
    // `w.abort(r)` → `sink.abort` + `closed` résolu.
    describe(
        &mut e,
        "var ar = '';\
         var ws2 = new WritableStream({abort: function (r) { ar = 'a' + r; }});\
         var w2 = ws2.getWriter();\
         w2.abort(7).then(function () { ar += '|ok'; });",
    );
    assert_eq!(describe(&mut e, "ar"), "a7|ok");
    // Double getWriter → TypeError (lock spec).
    assert_eq!(
        describe(
            &mut e,
            "try { ws.getWriter(); 'no' } catch (err) { 'threw' }"
        ),
        "threw"
    );
}

#[test]
fn c71_transform_and_pipethrough() {
    let (mut e, _t) = engine();
    // `transform(chunk, ctrl)` → `ctrl.enqueue` transformé ;
    // `pipeThrough` = pipeTo(writable) puis readable.
    describe(
        &mut e,
        "var src = new ReadableStream({start: function (c) {\
             c.enqueue('a'); c.enqueue('b'); c.close(); }});\
         var t = new TransformStream({\
             transform: function (c, ctrl) { ctrl.enqueue(c + '!'); },\
             flush: function (ctrl) { ctrl.enqueue('end'); }});\
         var out = src.pipeThrough(t);\
         var acc = '';\
         var rd = out.getReader();\
         rd.read().then(function (r) { acc += r.value + ';'; });\
         rd.read().then(function (r) { acc += r.value + ';'; });\
         rd.read().then(function (r) { acc += r.value + ';'; });\
         rd.read().then(function (r) { acc += 'd' + r.done; });",
    );
    assert_eq!(describe(&mut e, "acc"), "a!;b!;end;dtrue");
    // Transform identité (`new TransformStream()` nu).
    describe(
        &mut e,
        "var s2 = new ReadableStream({start: function (c) {\
             c.enqueue(1); c.close(); }});\
         var o2 = s2.pipeThrough(new TransformStream());\
         var id = '';\
         o2.getReader().read().then(function (r) { id = r.value; });",
    );
    assert_eq!(describe(&mut e, "id"), "1");
    // `ts.readable`/`ts.writable` exposés + instanceof.
    assert_eq!(
        describe(
            &mut e,
            "var t2 = new TransformStream();\
             (t2 instanceof TransformStream) + '|' +\
             (t2.readable instanceof ReadableStream) + '|' +\
             (t2.writable instanceof WritableStream)"
        ),
        "true|true|true"
    );
}

#[test]
fn c158_text_codec_streams() {
    // JS-C158 — `TextEncoderStream`/`TextDecoderStream`
    // (medium.com : `TextEncoderStream is not defined`) —
    // TransformStream à transformer natif UTF-8.
    let (mut e, _t) = engine();
    assert_eq!(
        describe(
            &mut e,
            "typeof TextEncoderStream + '|' + typeof TextDecoderStream + '|' +\
             (new TextEncoderStream().writable instanceof WritableStream)"
        ),
        "function|function|true"
    );
    // Round-trip : string → bytes → string.
    describe(
        &mut e,
        "var acc = '';\
         var te = new TextEncoderStream();\
         var w = te.writable.getWriter(), r = te.readable.getReader();\
         var td = new TextDecoderStream();\
         var wd = td.writable.getWriter(), rd2 = td.readable.getReader();\
         w.write('héllo').then(function () {\
             r.read().then(function (v) { acc += v.value.length + ';'; });\
         });\
         wd.write(new TextEncoder().encode('monde')).then(function () {\
             rd2.read().then(function (v) { acc += v.value; });\
         });",
    );
    assert_eq!(describe(&mut e, "acc"), "6;monde");
}

#[test]
fn c71_response_body_stream() {
    let (mut e, _t) = engine();
    // `res.body` = ReadableStream — un chunk Uint8Array + done.
    describe(
        &mut e,
        "fetch('/dl').then(function (r) {\
             globalThis.rb = r.body; globalThis.bu = r.bodyUsed; });",
    );
    respond(&mut e, 1, 200, "streamed");
    describe(
        &mut e,
        "var bl = (rb instanceof ReadableStream) + '|';\
         var rd = rb.getReader();\
         rd.read().then(function (r) {\
             bl += (r.value instanceof Uint8Array) + ':' + r.value.byteLength + ';'; });\
         rd.read().then(function (r) { bl += 'd' + r.done; });",
    );
    assert_eq!(describe(&mut e, "bl"), "true|true:8;dtrue");
    // `req.body` — stream pour POST, null pour GET (spec).
    assert_eq!(
        describe(
            &mut e,
            "var rq = new Request('/u', {method: 'POST', body: 'xy'});\
             (rq.body instanceof ReadableStream) + '|' +\
             (new Request('/g').body === null)"
        ),
        "true|true"
    );
    // `new Response('abc').body` → stream du corps.
    describe(
        &mut e,
        "var rr = new Response('abc'); var n = 0;\
         rr.body.getReader().read().then(function (r) { n = r.value.byteLength; });",
    );
    assert_eq!(describe(&mut e, "n"), "3");
    // `blob.stream()` → ReadableStream des octets (JS-C71).
    describe(
        &mut e,
        "var b = new Blob(['xy']); var bs = 0;\
         b.stream().getReader().read().then(function (r) {\
             bs = r.value.byteLength; });",
    );
    assert_eq!(describe(&mut e, "bs"), "2");
}

#[test]
fn c71_tee_and_cancel() {
    let (mut e, _t) = engine();
    // `tee()` → deux branches sur la queue restante ; source locked.
    describe(
        &mut e,
        "var rs = new ReadableStream({start: function (c) {\
             c.enqueue('x'); c.enqueue('y'); c.close(); }});\
         var pair = rs.tee();\
         var ta = ''; var tb = '';\
         pair[0].getReader().read().then(function (r) { ta = r.value; });\
         pair[1].getReader().read().then(function (r) { tb = r.value; });",
    );
    assert_eq!(
        describe(&mut e, "rs.locked + '|' + ta + '|' + tb"),
        "true|x|x"
    );
    // `cancel` → `source.cancel(reason)` + done.
    describe(
        &mut e,
        "var cr = '';\
         var rs2 = new ReadableStream({\
             start: function (c) { c.enqueue(1); },\
             cancel: function (r) { cr = 'c' + r; }});\
         rs2.cancel(9); var rd2 = rs2.getReader();\
         rd2.read().then(function (r) { cr += '|' + r.done; });",
    );
    assert_eq!(describe(&mut e, "cr"), "c9|true");
    // `reader.cancel` en promesse.
    describe(
        &mut e,
        "var cr2 = '';\
         var rs3 = new ReadableStream({cancel: function (r) { cr2 = 'k' + r; }});\
         var rd3 = rs3.getReader();\
         rd3.cancel(4).then(function () { cr2 += '|p'; });",
    );
    assert_eq!(describe(&mut e, "cr2"), "k4|p");
}

// ---- JS-C72 : IndexedDB + Cache API -------------------------------

#[test]
fn c72_idb_open_upgrade_create_store() {
    let (mut e, _t) = engine();
    // `indexedDB.open` : `upgradeneeded` (oldVersion=0) puis
    // `success` — `createObjectStore` légal dans l'upgrade.
    describe(
        &mut e,
        "var log = '';         var rq = indexedDB.open('appdb', 1);         rq.onupgradeneeded = function (ev) {           log += 'up' + ev.oldVersion + '>' + ev.newVersion;           var db = ev.target.result;           db.createObjectStore('kv', { keyPath: 'id' });         };         rq.onsuccess = function (ev) {           var db = ev.target.result;           log += '|ok:' + db.name + '@' + db.version                + '/' + db.objectStoreNames.join(',');         };",
    );
    assert_eq!(describe(&mut e, "log"), "up0>1|ok:appdb@1/kv");
}

#[test]
fn c187_idb_request_is_event_target() {
    let (mut e, _t) = engine();
    // mastodon vol 102 : `addEventListener is not a function` sur
    // `IDBRequest` tuait la boot idb (dexie/idb-keyval passent par
    // des listeners, pas des `on*`). Spec : `IDBRequest` est un
    // `EventTarget` — rail `__et` partagé, créé paresseusement.
    describe(
        &mut e,
        "var log = ''; var rq = indexedDB.open('etdb', 1);\
         log += typeof rq.addEventListener + ':';\
         rq.addEventListener('upgradeneeded', function (ev) {\
           log += 'up' + ev.oldVersion + ':';\
           ev.target.result.createObjectStore('kv');\
         });\
         rq.addEventListener('success', function (ev) {\
           log += 'ok:' + ev.target.result.name;\
         });\
         rq.onsuccess = function () { log += '|ons'; };",
    );
    // Ordre : `on*` d'abord puis les listeners (approximation
    // documentée — spec = ordre d'enregistrement).
    assert_eq!(describe(&mut e, "log"), "function:up0:|onsok:etdb");
    // `removeEventListener` retire ; `dispatchEvent` appelle.
    describe(
        &mut e,
        "var r2 = indexedDB.open('etdb2', 1), seen = '';\
         var cb = function () { seen += 'F'; };\
         r2.addEventListener('success', cb);\
         r2.removeEventListener('success', cb);\
         r2.onsuccess = function () { seen += '|clean'; };",
    );
    assert_eq!(describe(&mut e, "seen"), "|clean");
}

#[test]
fn c190_idb_request_thenable() {
    let (mut e, _t) = engine();
    // mastodon vol 116 : `then is not a function (on IDBRequest)`
    // — les wrappers idb `await`-ent la request directement.
    // `req.then(f)` = promesse du RESULT (déviation spec assumée :
    // `await req` attendu = résultat par les apps réelles).
    describe(
        &mut e,
        "var out = ''; var rq = indexedDB.open('thdb', 1);\
         rq.onupgradeneeded = function (ev) {\
           ev.target.result.createObjectStore('kv');\
         };\
         rq.then(function (db) { out += 'then:' + db.name + '@' + db.version; });",
    );
    e.run_microtasks().expect("microtasks");
    assert_eq!(describe(&mut e, "out"), "then:thdb@1");
    // `await req` sur une sous-request (get) — adopte le thenable.
    describe(
        &mut e,
        "var out2 = '';\
         var r0 = indexedDB.open('thdb2', 1);\
         r0.onupgradeneeded = function (ev) {\
           ev.target.result.createObjectStore('s');\
         };\
         r0.onsuccess = function (ev) {\
           var st = ev.target.result.transaction(['s'],'readwrite').objectStore('s');\
           st.put(42, 'k');\
           st.get('k').then(function (v) { out2 = 'got:' + v; });\
         };",
    );
    e.run_microtasks().expect("microtasks");
    assert_eq!(describe(&mut e, "out2"), "got:42");
}

#[test]
fn c72_idb_put_get_delete() {
    let (mut e, _t) = engine();
    describe(
        &mut e,
        "var out = '';         var r0 = indexedDB.open('shop', 1);         r0.onupgradeneeded = function (ev) {           ev.target.result.createObjectStore('items');         };         r0.onsuccess = function (ev) {           var db = ev.target.result;           var tx = db.transaction(['items'], 'readwrite');           var st = tx.objectStore('items');           st.put({ n: 5 }, 'a');           var g = st.get('a');           g.onsuccess = function (e2) { out += 'g:' + e2.target.result.n; };           var c = st.count();           c.onsuccess = function (e2) { out += '|c:' + e2.target.result; };           tx.oncomplete = function () { out += '|done'; };         };",
    );
    assert_eq!(describe(&mut e, "out"), "g:5|c:1|done");
    // deleteDatabase.
    describe(
        &mut e,
        "var dlog = '';         var rd = indexedDB.deleteDatabase('shop');         rd.onsuccess = function () { dlog = 'del'; };",
    );
    assert_eq!(describe(&mut e, "dlog"), "del");
}

#[test]
fn c72_idb_keypath_autoinc_cursor() {
    let (mut e, _t) = engine();
    describe(
        &mut e,
        "var log = '';         var r = indexedDB.open('lib', 1);         r.onupgradeneeded = function (ev) {           var db = ev.target.result;           var s = db.createObjectStore('books', { keyPath: 'isbn' });           s.createIndex('byYear', 'year', { unique: false });         };         r.onsuccess = function (ev) {           var st = ev.target.result.transaction(['books'], 'readwrite')             .objectStore('books');           st.put({ isbn: 'b2', year: 2001 });           st.put({ isbn: 'b1', year: 1999 });           var cur = st.openCursor();           var seen = [];           cur.onsuccess = function (e2) {             var c = e2.target.result;             if (c) { seen.push(c.key + '=' + c.value.year); c.continue(); }             else { log = seen.join(','); }           };         };",
    );
    // Curseur `next` : ordre de clés `b1` puis `b2`.
    assert_eq!(describe(&mut e, "log"), "b1=1999,b2=2001");
    // Requête sur l'index.
    describe(
        &mut e,
        "var ilog = '';         var r2 = indexedDB.open('lib');         r2.onsuccess = function (ev) {           var st = ev.target.result.transaction(['books'])             .objectStore('books');           var g = st.index('byYear').get(1999);           g.onsuccess = function (e2) {             ilog = e2.target.result.isbn;           };         };",
    );
    assert_eq!(describe(&mut e, "ilog"), "b1");
}

#[test]
fn c72_idb_keyrange_and_cmp() {
    let (mut e, _t) = engine();
    assert_eq!(
        describe(
            &mut e,
            "[indexedDB.cmp(1, 2), indexedDB.cmp('a', 'a'),              IDBKeyRange.bound(1, 5).includes(3),              IDBKeyRange.bound(1, 5).includes(9),              IDBKeyRange.lowerBound(2, true).includes(2)].join(',')"
        ),
        "-1,0,true,false,false"
    );
}

#[test]
fn c72_idb_errors() {
    let (mut e, _t) = engine();
    describe(
        &mut e,
        "var elog = '';         var r = indexedDB.open('errs', 1);         r.onupgradeneeded = function (ev) {           ev.target.result.createObjectStore('s');         };         r.onsuccess = function (ev) {           var st = ev.target.result.transaction(['s'], 'readwrite')             .objectStore('s');           try { st.put({ x: 1 }); } catch (er) {             elog = 'err:' + er.name;           }           var dup = st.put('v', 'k');           var dup2 = st.add('v', 'k');           dup.onsuccess = function () { elog += '|put-ok'; };           dup2.onerror = function (e2) {             elog += '|add:' + e2.target.error.name;           };         };",
    );
    // `put` sans clé ni keyPath → TypeError synchrone (spec
    // DataError, mappé TypeError) ; `add` doublon → request
    // `error` ConstraintError.
    assert_eq!(
        describe(&mut e, "elog"),
        "err:TypeError|put-ok|add:ConstraintError"
    );
}

#[test]
fn c72_caches_roundtrip() {
    let (mut e, _t) = engine();
    describe(
        &mut e,
        "var log = '';         var res = new Response('BODY1', { status: 201 });         caches.open('v1').then(function (c) {           return c.put('/api/x', res).then(function () { return c; });         }).then(function (c) {           return c.match('/api/x');         }).then(function (r) {           return r.text();         }).then(function (t) {           log = 'hit:' + t;         });",
    );
    assert_eq!(describe(&mut e, "log"), "hit:BODY1");
    // `has`/`keys`/`delete` + match global.
    describe(
        &mut e,
        "var log2 = '';         caches.has('v1').then(function (h) { log2 += 'has:' + h; });         caches.keys().then(function (ks) { log2 += '|' + ks.join('+'); });         caches.match('/api/x').then(function (r) {           log2 += '|m:' + (r !== undefined);         });         caches.delete('v1').then(function (d) {           log2 += '|del:' + d;           return caches.has('v1');         }).then(function (h) { log2 += '|gone:' + h; });",
    );
    assert_eq!(
        describe(&mut e, "log2"),
        "has:true|v1|m:true|del:true|gone:false"
    );
}

#[test]
fn c72_caches_add_uses_transport() {
    let (mut e, t) = engine();
    describe(
        &mut e,
        "var alog = '';         caches.open('v2').then(function (c) {           return c.add('/net-res');         }).then(function () { alog = 'added'; });",
    );
    // `add` → fetch via le transport stub — on joue le réseau.
    let reqs = t.requests.borrow();
    assert_eq!(reqs.len(), 1);
    let (h, r) = &reqs[0];
    assert_eq!(r.url, "/net-res");
    let h = *h;
    drop(reqs);
    respond(&mut e, h, 200, "NET!");
    assert_eq!(describe(&mut e, "alog"), "added");
    // Le corps est caché.
    describe(
        &mut e,
        "var blog = '';         caches.match('/net-res').then(function (r) {           return r.text();         }).then(function (t) { blog = t; });",
    );
    assert_eq!(describe(&mut e, "blog"), "NET!");
}

#[test]
fn c72_idb_request_shape() {
    let (mut e, _t) = engine();
    // `IDBRequest` : readyState pending → done, typeof global.
    assert_eq!(
        describe(
            &mut e,
            "var s = [];             var rq = indexedDB.open('shape', 1);             s.push(typeof indexedDB);             s.push(rq.readyState);             s.push(typeof rq.onsuccess);             rq.onsuccess = function () { s.push('done'); };             s.join(',')"
        ),
        "object,pending,undefined"
    );
    assert_eq!(describe(&mut e, "s[s.length-1]"), "done");
}

// ------------------------------------------------------------------
// JS-C73 — WebSocket stub honnête
// ------------------------------------------------------------------

#[test]
fn c73_websocket_shape() {
    let (mut e, _t) = engine();
    // `typeof WebSocket` → feature-detect des frameworks ;
    // constantes + props initiales spec.
    assert_eq!(
        describe(
            &mut e,
            "var s = [];             s.push(typeof WebSocket);             s.push(WebSocket.CONNECTING + '/' + WebSocket.OPEN + '/' + WebSocket.CLOSING + '/' + WebSocket.CLOSED);             var ws = new WebSocket('ws://h.test/sock');             s.push(ws instanceof WebSocket);             s.push(ws.readyState);             s.push(ws.url);             s.push(ws.protocol + '|' + ws.extensions + '|' + ws.binaryType);             s.push(ws.bufferedAmount);             s.join(',')"
        ),
        "function,0/1/2/3,true,0,ws://h.test/sock,||blob,0"
    );
}

#[test]
fn c73_websocket_refusal() {
    let (mut e, _t) = engine();
    // Handshake refusé en async : error puis close 1006 —
    // la séquence observable d'un vrai refus (arme les
    // fallbacks long-polling).
    assert_eq!(
        describe(
            &mut e,
            "var log = [];             var ws = new WebSocket('wss://h.test/x');             ws.onerror = function (ev) { log.push('E:' + ev.type); };             ws.onclose = function (ev) { log.push('C:' + ev.code + ':' + ev.wasClean + ':' + ws.readyState); };             log.push('pre:' + ws.readyState);             'armed'"
        ),
        "armed"
    );
    // Livré au checkpoint de fin d'eval.
    assert_eq!(
        describe(&mut e, "log.join(',')"),
        "pre:0,E:error,C:1006:false:3"
    );
}

#[test]
fn c73_websocket_url_and_protocols() {
    let (mut e, _t) = engine();
    // Schéma ws/wss exigé + arg requis + protocols validés.
    assert_eq!(
        describe(
            &mut e,
            "var out = [];             try { new WebSocket('http://h.test/'); } catch (err) { out.push('http:' + (err instanceof SyntaxError)); }             try { new WebSocket(); } catch (err) { out.push('noarg:' + (err instanceof TypeError)); }             try { new WebSocket('ws://h/', 'not a proto!'); } catch (err) { out.push('badtok:' + (err instanceof SyntaxError)); }             try { new WebSocket('ws://h/', ['a', 'a']); } catch (err) { out.push('dup:' + (err instanceof SyntaxError)); }             var ok = new WebSocket('ws://h/', ['chat', 'super']);             out.push('ok:' + ok.readyState);             out.join(',')"
        ),
        "http:true,noarg:true,badtok:true,dup:true,ok:0"
    );
}

#[test]
fn c73_websocket_send_close_binarytype() {
    let (mut e, _t) = engine();
    // `send` en CONNECTING → InvalidStateError (name spec) ;
    // `close()` valide → CLOSING ; codes invalides refusés ;
    // `binaryType` validé.
    assert_eq!(
        describe(
            &mut e,
            "var out = [];             var ws = new WebSocket('ws://h.test/');             try { ws.send('x'); } catch (err) { out.push('send:' + err.name); }             try { ws.close(2000); } catch (err) { out.push('code:' + err.name); }             try { ws.close(1000, 'x'.repeat(200)); } catch (err) { out.push('reason:' + (err instanceof SyntaxError)); }             ws.binaryType = 'arraybuffer';             out.push('bt:' + ws.binaryType);             ws.binaryType = 'bogus';             out.push('bt2:' + ws.binaryType);             ws.close();             out.push('rs:' + ws.readyState);             ws.close();             out.push('rs2:' + ws.readyState);             out.join(',')"
        ),
        "send:InvalidStateError,code:InvalidAccessError,reason:true,bt:arraybuffer,bt2:arraybuffer,rs:2,rs2:2"
    );
    // Le refus livre quand même error + close (spec : la
    // connexion échoue même après close() en CONNECTING).
    assert_eq!(describe(&mut e, "ws.readyState"), "3");
}

#[test]
fn c73_websocket_event_target() {
    let (mut e, _t) = engine();
    // addEventListener/removeEventListener + close() en
    // CONNECTING → readyState 2 jusqu'au refus.
    assert_eq!(
        describe(
            &mut e,
            "var log = [];             var ws = new WebSocket('ws://h.test/');             var a = function () { log.push('a'); };             var b = function () { log.push('b'); };             ws.addEventListener('close', a);             ws.addEventListener('close', b);             ws.removeEventListener('close', b);             ws.addEventListener('error', function () { log.push('err!'); });             'armed'"
        ),
        "armed"
    );
    assert_eq!(describe(&mut e, "log.join(',')"), "err!,a");
}

// ------------------------------------------------------------------
// JS-C74 — BroadcastChannel (contexte sans DOM)
// ------------------------------------------------------------------

#[test]
fn c74_broadcast_channel() {
    let (mut e, _t) = engine();
    // Réel intra-contexte : post → les autres instances du
    // canal reçoivent `message` au checkpoint ; pas soi-même.
    assert_eq!(
        describe(
            &mut e,
            "var log = [];             var a = new BroadcastChannel('chan');             var b = new BroadcastChannel('chan');             var c = new BroadcastChannel('autre');             a.onmessage = function (ev) { log.push('a:' + ev.data.n); };             b.addEventListener('message', function (ev) { log.push('b:' + ev.data.n); });             c.onmessage = function () { log.push('c:!'); };             b.postMessage({n: 42});             log.push(b.name + '|' + (a instanceof BroadcastChannel));             'armed'"
        ),
        "armed"
    );
    // a reçoit (pas b — jamais soi-même), c non (autre canal).
    assert_eq!(describe(&mut e, "log.join(',')"), "chan|true,a:42");
}

#[test]
fn c74_broadcast_close_and_clone() {
    let (mut e, _t) = engine();
    // close() → InvalidStateError + désinscription ; payload
    // cloné par destinataire (le receveur mute sans effet).
    describe(
        &mut e,
        "var log = [];         var a = new BroadcastChannel('x');         var b = new BroadcastChannel('x');         b.onmessage = function (ev) { ev.data.mut = true; log.push('got'); };         a.postMessage({mut: false});         var sent = a.postMessage ? 'ok' : 'no';         a.close();         try { a.postMessage(1); } catch (err) { log.push('closed:' + err.name); }",
    );
    // `closed:` posé dans l'eval, `got` livré au checkpoint.
    assert_eq!(
        describe(&mut e, "log.join(',')"),
        "closed:InvalidStateError,got"
    );
}

// ------------------------------------------------------------------
// JS-C77 — `fetch(mode, credentials)` → transport (couture 125/126).
// ------------------------------------------------------------------

#[test]
fn c77_fetch_defaults_and_props() {
    let (mut e, t) = engine();
    describe(&mut e, "fetch('/x');");
    let reqs = t.requests.borrow();
    assert_eq!(reqs.len(), 1);
    let r = &reqs[0].1;
    assert_eq!(r.mode, explorer_jscore::FetchMode::Cors);
    assert_eq!(r.credentials, explorer_jscore::FetchCredentials::SameOrigin);
    drop(reqs);
    // `Request` expose `mode`/`credentials` (défauts spec).
    assert_eq!(
        describe(
            &mut e,
            "var r = new Request('/y'); r.mode + '|' + r.credentials"
        ),
        "cors|same-origin"
    );
}

#[test]
fn c77_fetch_init_overrides() {
    let (mut e, t) = engine();
    describe(
        &mut e,
        "fetch('/x', {mode: 'no-cors', credentials: 'include'});",
    );
    let reqs = t.requests.borrow();
    let r = &reqs[0].1;
    assert_eq!(r.mode, explorer_jscore::FetchMode::NoCors);
    assert_eq!(r.credentials, explorer_jscore::FetchCredentials::Include);
}

#[test]
fn c77_no_cors_strips_author_headers() {
    let (mut e, t) = engine();
    describe(
        &mut e,
        "fetch('/x', {mode: 'no-cors', headers: {'X-Key': '1', 'Accept': 'a',                        'Content-Type': 'text/plain', 'Content-Language': 'fr'}});",
    );
    let reqs = t.requests.borrow();
    let names: Vec<String> = reqs[0]
        .1
        .headers
        .iter()
        .map(|(n, _)| n.to_ascii_lowercase())
        .collect();
    // Safelist seulement — `X-Key` strippé à l'émission.
    assert_eq!(names, vec!["accept", "content-type", "content-language"]);
}

#[test]
fn c77_same_origin_cross_rejects_before_transport() {
    let (mut e, t) = engine();
    e.set_page_url("https://a.test/dir/page");
    describe(
        &mut e,
        "var out = '';         fetch('https://b.test/x', {mode: 'same-origin'})           .catch(function (er) { out = er.name; });",
    );
    assert_eq!(describe(&mut e, "out"), "TypeError");
    // Rejet avant émission — le transport n'est pas sollicité.
    assert_eq!(t.requests.borrow().len(), 0);
    // `same-origin` sur cible same-origin : émise.
    describe(
        &mut e,
        "fetch('https://a.test/dir/y', {mode: 'same-origin'});",
    );
    assert_eq!(t.requests.borrow().len(), 1);
}

#[test]
fn c77_cross_cors_types_response() {
    let (mut e, t) = engine();
    e.set_page_url("https://a.test/dir/page");
    describe(
        &mut e,
        "var t1 = '', t2 = '';         fetch('https://a.test/dir/y').then(function (r) { t1 = r.type; });         fetch('https://b.test/z').then(function (r) { t2 = r.type; });",
    );
    let h1 = t.requests.borrow()[0].0;
    let h2 = t.requests.borrow()[1].0;
    respond(&mut e, h1, 200, "x");
    respond(&mut e, h2, 200, "x");
    assert_eq!(describe(&mut e, "t1 + '|' + t2"), "basic|cors");
}

#[test]
fn c77_no_cors_opaque_response() {
    let (mut e, t) = engine();
    e.set_page_url("https://a.test/dir/page");
    describe(
        &mut e,
        "var log = '';         fetch('https://b.test/z', {mode: 'no-cors'})           .then(function (r) {              r.text().then(function (t) {                log = r.type + '|' + r.status + '|' + r.ok + '|' +                       r.headers.get('x') + '|' + t.length;              });            });",
    );
    let h = t.requests.borrow()[0].0;
    respond(&mut e, h, 200, "secret");
    // Réponse opaque : `type:'opaque'`, status 0, `ok` false,
    // headers vides, corps vide — la promesse résout tout de même.
    assert_eq!(describe(&mut e, "log"), "opaque|0|false|null|0");
}

#[test]
fn c77_mode_and_credentials_validation() {
    let (mut e, t) = engine();
    describe(
        &mut e,
        "var out = [];         fetch('/a', {mode: 'navigate'})           .catch(function (er) { out.push(er.name); });         fetch('/b', {mode: 'bogus'})           .catch(function (er) { out.push(er.name); });         fetch('/c', {credentials: 'bogus'})           .catch(function (er) { out.push(er.name); });         fetch('/d', {mode: 'no-cors', method: 'PUT'})           .catch(function (er) { out.push(er.name); });",
    );
    assert_eq!(
        describe(&mut e, "out.join(',')"),
        "TypeError,TypeError,TypeError,TypeError"
    );
    assert_eq!(t.requests.borrow().len(), 0);
    // `new Request` : `TypeError` synchrone (spec).
    assert!(e.eval("new Request('/x', {mode: 'navigate'})").is_err());
    assert!(e
        .eval("new Request('/x', {mode: 'no-cors', method: 'PUT'})")
        .is_err());
    assert!(e.eval("new Request('/x', {credentials: 'bogus'})").is_err());
}

#[test]
fn c77_request_inherits_mode_credentials() {
    let (mut e, t) = engine();
    describe(
        &mut e,
        "var rq = new Request('/x', {mode: 'no-cors', credentials: 'omit'});         fetch(rq);",
    );
    let reqs = t.requests.borrow();
    let r = &reqs[0].1;
    assert_eq!(r.mode, explorer_jscore::FetchMode::NoCors);
    assert_eq!(r.credentials, explorer_jscore::FetchCredentials::Omit);
    drop(reqs);
    // `init` surcharge l'héritage.
    describe(&mut e, "fetch(rq, {credentials: 'include'});");
    let reqs = t.requests.borrow();
    assert_eq!(
        reqs[1].1.credentials,
        explorer_jscore::FetchCredentials::Include
    );
    assert_eq!(reqs[1].1.mode, explorer_jscore::FetchMode::NoCors);
}

#[test]
fn c77_xhr_with_credentials() {
    let (mut e, t) = engine();
    assert_eq!(
        describe(&mut e, "var x = new XMLHttpRequest(); x.withCredentials"),
        "false"
    );
    describe(
        &mut e,
        "x.open('POST', '/a'); x.withCredentials = true; x.send();",
    );
    let reqs = t.requests.borrow();
    let r = &reqs[0].1;
    assert_eq!(r.mode, explorer_jscore::FetchMode::Cors);
    assert_eq!(r.credentials, explorer_jscore::FetchCredentials::Include);
    drop(reqs);
    // `withCredentials` hors `opened` non envoyé →
    // `InvalidStateError` (spec — DOMException shape `{name}`).
    assert_eq!(
        describe(
            &mut e,
            "var n = ''; try { x.withCredentials = false; } catch (er) { n = er.name; } n"
        ),
        "InvalidStateError"
    );
    // `open()` reset l'état, pas le drapeau — toujours include.
    describe(&mut e, "x.open('GET', '/b'); x.send();");
    assert_eq!(
        t.requests.borrow()[1].1.credentials,
        explorer_jscore::FetchCredentials::Include
    );
}

#[test]
fn c77_send_beacon_no_cors_include() {
    // `sendBeacon` (navigator, DOM installé) : spec `mode:
    // no-cors` + `credentials: include` — fire-and-forget.
    let mut e = Engine::new();
    let t = StubTransport::new();
    e.install_xhr(t.clone());
    e.install_dom(&std::rc::Rc::new(std::cell::RefCell::new(
        explorer_jscore::Dom::from_html("<html><body></body></html>"),
    )));
    describe(&mut e, "navigator.sendBeacon('/beacon', 'd=1')");
    let reqs = t.requests.borrow();
    assert_eq!(reqs.len(), 1);
    let r = &reqs[0].1;
    assert_eq!(r.method, "POST");
    assert_eq!(r.mode, explorer_jscore::FetchMode::NoCors);
    assert_eq!(r.credentials, explorer_jscore::FetchCredentials::Include);
}

/// `xhr.upload` (JS-C91) — objet `EventTarget` stable, avale les
/// registrations : htmx enchaîne `xhr.upload.addEventListener
/// ('progress',…)` sur chaque requête — `undefined` tuait son
/// pipeline ajax.
#[test]
fn xhr_upload_stub_stable() {
    let (mut e, _t) = engine();
    assert_eq!(
        describe(
            &mut e,
            "var x=new XMLHttpRequest();var u=x.upload;\
             [typeof u, u===x.upload, u.onprogress,\
             (u.addEventListener('progress',function(){}),'ok')].join('|')"
        ),
        "object|true||ok"
    );
}

/// `AbortSignal.timeout(ms)` (JS-C194, maif vol 122) — statique
/// spec : signal qui aborte après `ms` via la file de timers,
/// raison `TimeoutError`, `onabort` + listeners dispatchés.
#[test]
fn c194_abort_signal_timeout() {
    let (mut e, _t) = engine();
    assert_eq!(describe(&mut e, "typeof AbortSignal.timeout"), "function");
    assert_eq!(
        describe(
            &mut e,
            "var s=AbortSignal.timeout(0);var f='';\
             s.addEventListener('abort',function(){f='l'});\
             s.aborted+'|'+(s instanceof AbortSignal)"
        ),
        "false|true"
    );
    e.run_due_timers().expect("timers");
    assert_eq!(
        describe(
            &mut e,
            "f+'|'+s.aborted+'|'+s.reason.name+'|'+(function(){try{s.throwIfAborted();return 'no'}catch(x){return x.name}})()"
        ),
        "l|true|TimeoutError|TimeoutError"
    );
    // Délai > 0 : le signal reste vivant tant que le timer
    // n'est pas dû.
    assert_eq!(
        describe(&mut e, "var s2=AbortSignal.timeout(50);s2.aborted"),
        "false"
    );
}

/// §B-14 — coinbase vol 148 : `get is not a function (on
/// IDBRequest)` + JS-C215 : les interfaces IDB partagent les
/// protos **réels** des instances — `instanceof`,
/// `constructor.name`, héritage `EventTarget` et
/// `IDBOpenDBRequest < IDBRequest`. Verrou surface +
/// chaînage.
#[test]
fn b14_idb_request_surfaces_spec() {
    let (mut e, _t) = engine();
    e.eval(
        "var out='';\
         var rq=indexedDB.open('pdb',1);\
         rq.onupgradeneeded=function(ev){\
           ev.target.result.createObjectStore('kv',{keyPath:'k'});\
         };\
         rq.onsuccess=function(ev){\
           var db=ev.target.result;\
           var tx=db.transaction('kv','readwrite');\
           var st=tx.objectStore('kv');\
           var g=st.get('x');\
           out=[typeof g.get,typeof g.source.get,typeof st.get,\
                typeof tx.get,typeof db.get,\
                rq instanceof IDBRequest,\
                rq instanceof IDBOpenDBRequest,\
                g instanceof IDBRequest,\
                !(g instanceof IDBOpenDBRequest),\
                rq.constructor.name,g.constructor.name,\
                st.constructor.name,tx.constructor.name,db.constructor.name,\
                indexedDB instanceof IDBFactory].join('|');\
         };",
    )
    .expect("eval");
    assert_eq!(
        describe(&mut e, "out"),
        "undefined|function|function|undefined|undefined\
         |true|true|true|true\
         |IDBOpenDBRequest|IDBRequest|IDBObjectStore|IDBTransaction|IDBDatabase|true"
    );
}

/// JS-C215 — greffe sur le proto réel : `IDBRequest.prototype`
/// modifiée par un script (ruxit/workbox patchent les protos
/// au boot) doit être visible par les vraies requests —
/// l'ancien proto nu distinct absorbait la greffe.
#[test]
fn c215_idb_proto_graft_reaches_instances() {
    let (mut e, _t) = engine();
    e.eval(
        "globalThis.__hit=0;\
         IDBRequest.prototype.__probe=function(){__hit++};",
    )
    .expect("graft");
    e.eval(
        "var rq=indexedDB.open('gdb',1);\
         rq.onupgradeneeded=function(ev){\
           ev.target.result.createObjectStore('kv');\
         };\
         rq.onsuccess=function(){\
           var st=rq.result.transaction('kv').objectStore('kv');\
           var g=st.get('x');\
           g.__probe();rq.__probe();\
         };",
    )
    .expect("eval");
    assert_eq!(describe(&mut e, "__hit"), "2");
    // `onupgradeneeded`/`onblocked` restent des props
    // `IDBOpenDBRequest` : une request `store.get` ne doit
    // pas être `instanceof IDBOpenDBRequest` (vérifié plus
    // haut) mais `open` l'est.
}
