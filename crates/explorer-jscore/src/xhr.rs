// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! `XMLHttpRequest` over an injectable transport (JS-4).
//!
//! explorer-jscore **never touches the network itself** (§4.11): the
//! embedder installs an [`XhrTransport`] (backed by
//! `explorer-net`'s `HttpTransport` in the browser, by an
//! in-process stub in tests). The flow mirrors
//! the timer pump:
//!
//! 1. `xhr.open(method, url)` + `xhr.send()` → the engine calls
//!    `transport.send(req)` and registers the wrapper under the
//!    returned handle.
//! 2. The embedder performs the request on its own tasks, then calls
//!    [`Engine::deliver_xhr_response`] — which fills `status`/
//!    `responseText`, moves `readyState` to 4 and fires
//!    `readystatechange`/`load` (or `error`) handlers.
//!
//! Deliberate subset: everything is async (`open(m, u, false)` sync
//! send can't block our model), `response`/`responseType`/`upload`/
//! progress events are out — `responseText`, `responseXML` (parsed
//! into the mini-DOM), headers and the classic callback props cover
//! the old-school scripts we target.

use crate::heap::{Host, JsResult, ObjKind};
use crate::value::Value;
use crate::Engine;
use std::cell::RefCell;
use std::rc::Rc;

/// What the script asked for — handed to the embedder's transport.
#[derive(Debug, Clone)]
pub struct XhrRequest {
    /// HTTP method as written (`GET`, `POST`…).
    pub method: String,
    /// URL as written — resolution to absolute is the embedder's job
    /// (it owns the base URL context).
    pub url: String,
    /// `setRequestHeader` calls, in order.
    pub headers: Vec<(String, String)>,
    /// `send(body)` payload, if any.
    pub body: Option<String>,
}

/// What the embedder got back — handed to `deliver_xhr_response`.
#[derive(Debug, Clone)]
pub struct XhrResponse {
    /// The handle id returned by `XhrTransport::send`.
    pub handle: u64,
    /// HTTP status (`0` for network-level failure → `error` event).
    pub status: u16,
    /// Reason phrase.
    pub status_text: String,
    /// Response headers.
    pub headers: Vec<(String, String)>,
    /// Raw body (lossy-decoded to `responseText`).
    pub body: Vec<u8>,
}

/// The embedder's network connector. Object-safe; the engine calls
/// `send` synchronously at `xhr.send()` time — the transport should
/// enqueue work and return a handle immediately.
pub trait XhrTransport {
    /// Queue the request; returns a handle the embedder will echo in
    /// `deliver_xhr_response`.
    fn send(&self, req: &XhrRequest) -> u64;
    /// `xhr.abort()` — best-effort cancellation.
    fn abort(&self, _handle: u64) {}
}

/// Engine-side XHR bookkeeping: live handle → wrapper + shared data.
#[derive(Default)]
pub(crate) struct XhrEngineState {
    pub transport: Option<Rc<dyn XhrTransport>>,
    /// In-flight requests: `(handle, wrapper ObjId, shared data)`.
    pub live: Vec<(u64, crate::value::ObjId, Rc<RefCell<XhrData>>)>,
    /// In-flight `fetch` calls: `(handle, promise ObjId, request
    /// url)` — l'URL est re-servie en `response.url` (JS-C66).
    pub fetches: Vec<(u64, crate::value::ObjId, String)>,
    /// `Headers.prototype`/`Request.prototype`/`Response.prototype`
    /// — posés par `install_fetch` (JS-C66), semés en racines GC
    /// dans `Engine::collect`.
    pub headers_proto: Option<crate::value::ObjId>,
    /// `Request.prototype` — voir `headers_proto`.
    pub req_proto: Option<crate::value::ObjId>,
    /// `Response.prototype` — voir `headers_proto`.
    pub res_proto: Option<crate::value::ObjId>,
    /// `FormData.prototype` (JS-C67) — paresseux (naît de
    /// `res.formData()`/`new FormData` même sans DOM).
    pub fd_proto: Option<crate::value::ObjId>,
}

/// Per-XHR-instance state, shared between the JS wrapper and the
/// engine's `live` registry.
#[derive(Default)]
pub(crate) struct XhrData {
    ready_state: u8,
    status: u16,
    status_text: String,
    response_text: String,
    response_headers: Vec<(String, String)>,
    method: String,
    url: String,
    request_headers: Vec<(String, String)>,
    handle: Option<u64>,
    /// `addEventListener` handlers: `(type, fn, capture)`.
    listeners: Vec<(String, Value, bool)>,
}

impl XhrData {
    /// `Value` retenues par les listeners — racines GC : un XHR
    /// vivant garde ses callbacks `readystatechange`/`load`/…
    /// en vie.
    pub(crate) fn listener_values(&self, out: &mut Vec<Value>) {
        for (_, cb, _) in &self.listeners {
            out.push(*cb);
        }
    }
}

/// The `XMLHttpRequest` host object.
struct XhrHost {
    data: Rc<RefCell<XhrData>>,
}

impl Engine {
    /// Install the `XMLHttpRequest` constructor on the global object
    /// and route `send()` through `transport`.
    pub fn install_xhr(&mut self, transport: Rc<dyn XhrTransport>) {
        self.xhr.transport = Some(transport);
        let v = self.native_fn("XMLHttpRequest", xhr_ctor);
        let g = self.global;
        self.set_own(g, "XMLHttpRequest", v, false);
        self.install_fetch();
        self.install_abort();
    }

    /// Deliver a completed response: fills the XHR, moves it to
    /// `readyState` 4 and fires `readystatechange` then `load`
    /// (`error` when `status == 0`). Unknown handles are dropped —
    /// the XHR may have been aborted meanwhile.
    ///
    /// # Errors
    /// Propagates the first uncaught exception from a handler.
    pub fn deliver_xhr_response(&mut self, resp: XhrResponse) -> JsResult<()> {
        // `fetch` handles resolve a promise instead of filling an XHR.
        if let Some(pos) = self
            .xhr
            .fetches
            .iter()
            .position(|(h, _, _)| *h == resp.handle)
        {
            let (_, pid, url) = self.xhr.fetches.remove(pos);
            if resp.status == 0 {
                let msg = self.str_val("fetch: network error");
                self.reject_promise(pid, msg);
            } else {
                let r = self.make_fetch_response(resp, url);
                self.resolve_promise(pid, r);
            }
            self.run_microtasks()?;
            return Ok(());
        }
        let Some(pos) = self.xhr.live.iter().position(|(h, _, _)| *h == resp.handle) else {
            return Ok(());
        };
        let (_, wrapper, data) = self.xhr.live.remove(pos);
        {
            let mut d = data.borrow_mut();
            d.status = resp.status;
            d.status_text = resp.status_text;
            d.response_headers = resp.headers;
            d.response_text = String::from_utf8_lossy(&resp.body).into_owned();
            d.ready_state = 4;
            d.handle = None;
        }
        self.fire_xhr_event(wrapper, &data, "readystatechange")?;
        self.fire_xhr_event(
            wrapper,
            &data,
            if resp.status == 0 { "error" } else { "load" },
        )?;
        // Handlers may have queued promise reactions — drain now that
        // the XHR job is complete.
        self.run_microtasks()?;
        Ok(())
    }

    /// `readystatechange`/`load`/`error` fan-out: `on<ty>` own prop,
    /// then `addEventListener` registrations.
    fn fire_xhr_event(
        &mut self,
        wrapper: crate::value::ObjId,
        data: &Rc<RefCell<XhrData>>,
        ty: &str,
    ) -> JsResult<()> {
        let this = Value::Obj(wrapper);
        let ev = {
            let o = self.new_object();
            let t = self.str_val(ty);
            self.set_own(o, "type", t, true);
            self.set_own(o, "target", this, true);
            Value::Obj(o)
        };
        let key = format!("on{ty}");
        if let Some(f) = self.get_prop(wrapper, &key) {
            if self.is_callable(f) {
                self.call_value(f, this, &[ev])?;
            }
        }
        let fns: Vec<Value> = data
            .borrow()
            .listeners
            .iter()
            .filter(|(t, _, _)| t == ty)
            .map(|(_, f, _)| *f)
            .collect();
        for f in fns {
            if self.is_callable(f) {
                self.call_value(f, this, &[ev])?;
            }
        }
        Ok(())
    }
}

/// `new XMLHttpRequest()` (also callable without `new` — returns a
/// fresh XHR either way, matching browser leniency).
fn xhr_ctor(e: &mut Engine, _this: Value, _args: &[Value]) -> JsResult<Value> {
    Ok(e.new_host_object(Rc::new(XhrHost {
        data: Rc::new(RefCell::new(XhrData::default())),
    })))
}

fn arg(args: &[Value], i: usize) -> Value {
    args.get(i).copied().unwrap_or(Value::Undefined)
}

impl Host for XhrHost {
    fn class_name(&self) -> &'static str {
        "XMLHttpRequest"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn get(&self, e: &mut Engine, _t: Value, key: &str) -> JsResult<Option<Value>> {
        let d = self.data.borrow();
        Ok(Some(match key {
            "readyState" => Value::Num(f64::from(d.ready_state)),
            "status" => Value::Num(f64::from(d.status)),
            "statusText" => e.str_val(d.status_text.clone()),
            "responseText" | "response" => e.str_val(d.response_text.clone()),
            "responseXML" => {
                if d.ready_state < 4 || d.status == 0 {
                    Value::Null
                } else {
                    let body = d.response_text.clone();
                    drop(d);
                    let domref: crate::dom_host::DomRef =
                        Rc::new(RefCell::new(crate::dom::Dom::from_html(&body)));
                    crate::dom_host::document_value(e, &domref)
                }
            }
            "UNSENT" | "OPENED" | "HEADERS_RECEIVED" | "LOADING" | "DONE" => {
                // Constants per spec (also exposed on the constructor
                // — old scripts use `xhr.DONE` on the instance).
                Value::Num(match key {
                    "OPENED" => 1.0,
                    "HEADERS_RECEIVED" => 2.0,
                    "LOADING" => 3.0,
                    "DONE" => 4.0,
                    _ => 0.0,
                })
            }
            _ => return Ok(None),
        }))
    }

    fn invoke(
        &self,
        e: &mut Engine,
        this: Value,
        name: &str,
        args: &[Value],
    ) -> JsResult<Option<Value>> {
        match name {
            "open" => {
                let method = e.to_js_string(arg(args, 0))?;
                let url = e.to_js_string(arg(args, 1))?;
                let mut d = self.data.borrow_mut();
                // `open` resets the request per spec.
                *d = XhrData {
                    ready_state: 1,
                    method,
                    url,
                    listeners: std::mem::take(&mut d.listeners),
                    ..XhrData::default()
                };
                Ok(Some(Value::Undefined))
            }
            "setRequestHeader" => {
                let k = e.to_js_string(arg(args, 0))?;
                let v = e.to_js_string(arg(args, 1))?;
                self.data.borrow_mut().request_headers.push((k, v));
                Ok(Some(Value::Undefined))
            }
            "send" => {
                let mut ct: Option<String> = None;
                let body = match arg(args, 0) {
                    Value::Undefined | Value::Null => None,
                    v => {
                        // `FormData`/`URLSearchParams` → urlencoded +
                        // CT (même déviation que `fetch`, JS-C67).
                        let (text, c) = body_init_text(e, v)?;
                        ct = c;
                        Some(text)
                    }
                };
                if let Some(ct) = ct {
                    let mut d = self.data.borrow_mut();
                    if !d
                        .request_headers
                        .iter()
                        .any(|(k, _)| k.eq_ignore_ascii_case("content-type"))
                    {
                        d.request_headers.push(("Content-Type".into(), ct));
                    }
                }
                let (transport, wrapper) = {
                    let t = e.xhr.transport.clone();
                    let Value::Obj(w) = this else {
                        return Err(e.type_error("send: bad receiver"));
                    };
                    (t, w)
                };
                let Some(transport) = transport else {
                    // No connector installed → behave like network down.
                    let mut d = self.data.borrow_mut();
                    d.ready_state = 4;
                    d.status = 0;
                    drop(d);
                    e.fire_xhr_event(wrapper, &self.data, "readystatechange")?;
                    e.fire_xhr_event(wrapper, &self.data, "error")?;
                    return Ok(Some(Value::Undefined));
                };
                let req = {
                    let mut d = self.data.borrow_mut();
                    d.ready_state = 2; // headers "sent" — embedder reports the rest
                    XhrRequest {
                        method: d.method.clone(),
                        url: d.url.clone(),
                        headers: d.request_headers.clone(),
                        body,
                    }
                };
                let handle = transport.send(&req);
                self.data.borrow_mut().handle = Some(handle);
                e.xhr.live.push((handle, wrapper, self.data.clone()));
                Ok(Some(Value::Undefined))
            }
            "abort" => {
                if let Some(h) = self.data.borrow_mut().handle.take() {
                    if let Some(t) = &e.xhr.transport {
                        t.abort(h);
                    }
                    e.xhr.live.retain(|(lh, _, _)| *lh != h);
                }
                self.data.borrow_mut().ready_state = 0;
                Ok(Some(Value::Undefined))
            }
            "getResponseHeader" => {
                let k = e.to_js_string(arg(args, 0))?;
                let v = self
                    .data
                    .borrow()
                    .response_headers
                    .iter()
                    .find(|(hk, _)| hk.eq_ignore_ascii_case(&k))
                    .map(|(_, v)| v.clone());
                Ok(Some(match v {
                    Some(v) => e.str_val(v),
                    None => Value::Null,
                }))
            }
            "getAllResponseHeaders" => {
                let s = self.data.borrow().response_headers.iter().fold(
                    String::new(),
                    |mut acc, (k, v)| {
                        acc.push_str(k);
                        acc.push_str(": ");
                        acc.push_str(v);
                        acc.push_str("\r\n");
                        acc
                    },
                );
                Ok(Some(e.str_val(s)))
            }
            "addEventListener" => {
                let ty = e.to_js_string(arg(args, 0))?;
                let f = arg(args, 1);
                let capture = e.to_boolean(arg(args, 2));
                if e.is_callable(f) {
                    self.data.borrow_mut().listeners.push((ty, f, capture));
                }
                Ok(Some(Value::Undefined))
            }
            "removeEventListener" => {
                let ty = e.to_js_string(arg(args, 0))?;
                let f = arg(args, 1);
                let capture = e.to_boolean(arg(args, 2));
                self.data
                    .borrow_mut()
                    .listeners
                    .retain(|(t, lf, c)| !(t == &ty && *c == capture && e.strict_eq(*lf, f)));
                Ok(Some(Value::Undefined))
            }
            _ => Ok(None),
        }
    }
}

// ------------------------------------------------------------------
// fetch + `Headers`/`Request`/`Response` (JS-7e → surface JS-C66)
// ------------------------------------------------------------------
//
// `Headers`/`Request`/`Response` sont des objets à prototype réel
// (`instanceof`, `for-of` et `Object.keys` observables conformes) ;
// le corps vit dans `ObjKind::FetchBody` — `body` = `null` honnête
// tant que `ReadableStream` n'est pas livré (JS-C71). Un objet
// `Headers` porte ses entrées en own-props **non énumérables**
// (nom minuscule → valeur jointe : le `get`/l'itération spec
// combinent de toute façon) ; `set-cookie` fait exception — jamais
// combiné, il est rangé dans la prop sentinelle [`SET_COOKIE_KEY`]
// et re-servi par `getSetCookie()`. `blob()`/`formData()` rendent
// des objets minimaux honnêtes — les constructeurs `Blob`/
// `FormData` complets arrivent à JS-C70/JS-C67.

/// Prop cachée portant la liste brute `set-cookie` (tableau de
/// chaînes — jamais combinée, spec `no-combine`).
const SET_COOKIE_KEY: &str = "\u{f130}headers.set-cookie";

impl Engine {
    /// Install `fetch`, `Headers`, `Request`, `Response` — ctors +
    /// prototypes réels (idempotent : les protos survivent à une
    /// ré-installation).
    pub fn install_fetch(&mut self) {
        if self.xhr.headers_proto.is_none() {
            let objp = Value::Obj(self.protos.object);
            let hp = self.heap.alloc_obj(crate::heap::Object::ordinary(objp));
            self.add_method(hp, "append", h_append);
            self.add_method(hp, "set", h_set);
            self.add_method(hp, "delete", h_delete);
            self.add_method(hp, "has", h_has);
            self.add_method(hp, "get", h_get);
            self.add_method(hp, "getSetCookie", h_get_set_cookie);
            self.add_method(hp, "forEach", h_for_each);
            self.add_method(hp, "entries", h_entries);
            self.add_method(hp, "keys", h_keys);
            self.add_method(hp, "values", h_values);
            // `for-of`/`a[Symbol.iterator]` : sur le prototype (lu
            // par `iter_values` via `get_prop` — chaîne complète).
            let it = self.native_fn("[Symbol.iterator]", h_entries);
            self.set_own(hp, crate::interp::SYM_ITERATOR, it, false);
            let tag = self.str_val("Headers");
            self.set_own(hp, crate::interp::SYM_TO_STRING_TAG, tag, false);
            let qp = self.heap.alloc_obj(crate::heap::Object::ordinary(objp));
            let sp = self.heap.alloc_obj(crate::heap::Object::ordinary(objp));
            // Mixin `Body` + `clone()` partagé Request/Response.
            for p in [qp, sp] {
                self.add_method(p, "text", body_text);
                self.add_method(p, "json", body_json);
                self.add_method(p, "arrayBuffer", body_arraybuf);
                self.add_method(p, "formData", body_formdata);
                self.add_method(p, "blob", body_blob);
                self.add_method(p, "clone", fetch_clone);
            }
            let tag = self.str_val("Request");
            self.set_own(qp, crate::interp::SYM_TO_STRING_TAG, tag, false);
            let tag = self.str_val("Response");
            self.set_own(sp, crate::interp::SYM_TO_STRING_TAG, tag, false);
            self.xhr.headers_proto = Some(hp);
            self.xhr.req_proto = Some(qp);
            self.xhr.res_proto = Some(sp);
        }
        let g = self.global;
        let v = self.native_fn("fetch", fetch_fn);
        self.set_own(g, "fetch", v, false);
        let hc = self.native_fn("Headers", headers_ctor);
        self.link_ctor(hc, self.xhr.headers_proto);
        self.set_own(g, "Headers", hc, false);
        let rc = self.native_fn("Request", request_ctor);
        self.link_ctor(rc, self.xhr.req_proto);
        self.set_own(g, "Request", rc, false);
        let sc = self.native_fn("Response", response_ctor);
        self.link_ctor(sc, self.xhr.res_proto);
        if let Value::Obj(cid) = sc {
            self.add_method(cid, "error", response_error);
            self.add_method(cid, "redirect", response_redirect);
            self.add_method(cid, "json", response_json);
        }
        self.set_own(g, "Response", sc, false);
        // `FormData` (JS-C67) — `new FormData()` valide sans DOM ;
        // l'argument `form` exige le miroir (`install_dom`).
        crate::dom_host::install_formdata_global(self);
    }

    /// `ctor.prototype = proto` + `proto.constructor = ctor` —
    /// liaison `instanceof` (`OrdinaryHasInstance`) des classes
    /// fetch (JS-C66).
    fn link_ctor(&mut self, ctor: Value, proto: Option<crate::value::ObjId>) {
        let Some(pid) = proto else { return };
        if let Value::Obj(cid) = ctor {
            self.set_own(cid, "prototype", Value::Obj(pid), false);
        }
        self.set_own(pid, "constructor", ctor, false);
    }

    /// `Response` livrée par le transport : même forme que
    /// `new Response` — `type: "basic"`, `url` = URL de la requête.
    fn make_fetch_response(&mut self, resp: XhrResponse, url: String) -> Value {
        self.response_obj(RespSpec {
            status: resp.status,
            status_text: resp.status_text,
            headers: resp.headers,
            body: String::from_utf8_lossy(&resp.body).into_owned(),
            ty: "basic",
            url,
            redirected: false,
        })
    }

    /// Objet `Response` complet : `ObjKind::FetchBody` (corps
    /// caché) + own-props publiques — source unique pour
    /// `new Response`, `Response.error/redirect/json` et la
    /// livraison réseau.
    fn response_obj(&mut self, s: RespSpec) -> Value {
        let proto = self.xhr.res_proto.unwrap_or(self.protos.object);
        let id = self.heap.alloc_obj(crate::heap::Object {
            proto: Value::Obj(proto),
            props: Vec::new(),
            kind: ObjKind::FetchBody { body: s.body },
            extensible: true,
            elem_integrity: 0,
            args_map: None,
        });
        let hdrs = self.headers_obj(s.headers);
        let st = self.str_val(s.status_text);
        let u = self.str_val(s.url);
        let ty = self.str_val(s.ty);
        self.set_own(id, "type", ty, true);
        self.set_own(id, "url", u, true);
        self.set_own(id, "redirected", Value::Bool(s.redirected), true);
        self.set_own(id, "status", Value::Num(f64::from(s.status)), true);
        self.set_own(id, "statusText", st, true);
        self.set_own(id, "ok", Value::Bool((200..300).contains(&s.status)), true);
        self.set_own(id, "headers", hdrs, true);
        self.set_own(id, "bodyUsed", Value::Bool(false), true);
        // `ReadableStream` = JS-C71 — `null` honnête (le texte reste
        // lisible via `text()`/`json()`/…).
        self.set_own(id, "body", Value::Null, true);
        Value::Obj(id)
    }

    /// Objet `Headers` (proto `Headers.prototype`) dont les entrées
    /// sont posées en own-props non énumérables — `set-cookie`
    /// isolé en prop sentinelle (jamais combiné).
    fn headers_obj(&mut self, pairs: Vec<(String, String)>) -> Value {
        let proto = self.xhr.headers_proto.unwrap_or(self.protos.object);
        let id = self
            .heap
            .alloc_obj(crate::heap::Object::ordinary(Value::Obj(proto)));
        for (k, v) in pairs {
            self.headers_append_raw(id, &k, v);
        }
        Value::Obj(id)
    }

    /// `append` brut (validation déjà faite par l'appelant) :
    /// combine par `", "` — `set-cookie` pousse dans la liste
    /// sentinelle (spec `no-combine`).
    fn headers_append_raw(&mut self, id: crate::value::ObjId, name: &str, val: String) {
        let key = name.to_lowercase();
        if key == "set-cookie" {
            let cur = match self.get_prop(id, SET_COOKIE_KEY) {
                Some(v) => v,
                None => Value::Undefined,
            };
            let vv = self.str_val(val);
            if let Value::Obj(aid) = cur {
                if let Some(o) = self.heap.obj_mut(aid) {
                    if let ObjKind::Array(elems) = &mut o.kind {
                        elems.push(vv);
                    }
                }
            } else {
                let a = self.new_array(vec![vv]);
                self.set_own(id, SET_COOKIE_KEY, a, false);
            }
            return;
        }
        let joined = match self.get_prop(id, &key) {
            Some(v) => format!("{}, {}", self.heap_string_of(v).unwrap_or_default(), val),
            None => val,
        };
        let s = self.str_val(joined);
        self.set_own(id, &key, s, false);
    }
}

/// Spec interne d'un `Response` (livraison réseau ou ctors).
struct RespSpec {
    status: u16,
    status_text: String,
    headers: Vec<(String, String)>,
    body: String,
    ty: &'static str,
    url: String,
    redirected: bool,
}

// ---- Headers -----------------------------------------------------

/// Nom d'en-tête RFC 7230 `token` valide (`append`/`set`/`delete`/
/// `get`/`has` l'exigent — `TypeError` sinon, spec `name/value`).
fn valid_header_name(n: &str) -> bool {
    !n.is_empty()
        && n.bytes().all(|b| {
            matches!(b,
                b'!' | b'#' | b'$' | b'%' | b'&' | b'\'' | b'*' | b'+'
                | b'-' | b'.' | b'^' | b'_' | b'`' | b'|' | b'~'
                | b'0'..=b'9' | b'a'..=b'z' | b'A'..=b'Z')
        })
}

/// `name` validé puis passé en minuscules (en-têtes insensibles
/// à la casse) — `TypeError` si le nom est malformé.
fn h_name(e: &mut Engine, v: Value) -> JsResult<String> {
    let n = e.to_js_string(v)?;
    if !valid_header_name(&n) {
        return Err(e.type_error(format!("invalid header name {n:?}")));
    }
    Ok(n.to_lowercase())
}

/// Valeur normalisée (spec `normalize`) : blancs HTTP de bord
/// rognés, NUL/CR/LF refusés — `TypeError` sinon.
fn h_value(e: &mut Engine, v: Value) -> JsResult<String> {
    let s = e.to_js_string(v)?;
    if s.contains(['\0', '\r', '\n']) {
        return Err(e.type_error("invalid header value"));
    }
    Ok(s.trim_matches(|c| c == ' ' || c == '\t').to_string())
}

/// Entrées `(nom, valeur)` d'un `HeadersInit` : objet `Headers` ou
/// record `{name: v}` (scan des own-props hors sentinelles — les
/// props non énumérables des `Headers` incluses), ou séquence
/// `[[n, v], …]` (noms validés, spec).
fn headers_pairs(e: &mut Engine, v: Value) -> JsResult<Vec<(String, String)>> {
    let Value::Obj(id) = v else {
        return Ok(Vec::new());
    };
    let arr = match e.heap.obj(id).map(|o| &o.kind) {
        Some(ObjKind::Array(elems)) => Some(elems.clone()),
        _ => None,
    };
    if let Some(elems) = arr {
        let mut out = Vec::new();
        for el in elems {
            let k = e.get_prop_value(el, "0")?;
            let v2 = e.get_prop_value(el, "1")?;
            let k = e.to_js_string(k)?;
            let v2 = h_value(e, v2)?;
            if !valid_header_name(&k) {
                return Err(e.type_error(format!("invalid header name {k:?}")));
            }
            out.push((k.to_lowercase(), v2));
        }
        return Ok(out);
    }
    let keys: Vec<String> = match e.heap.obj(id) {
        Some(o) => o
            .props
            .iter()
            .filter(|(k, _)| !k.starts_with('\u{f130}'))
            .map(|(k, _)| k.clone())
            .collect(),
        None => Vec::new(),
    };
    let mut out = Vec::new();
    for k in keys {
        let vv = e.get_prop_value(v, &k)?;
        out.push((k, h_value(e, vv)?));
    }
    Ok(out)
}

/// `new Headers(init?)` — `init` = `Headers` | `[[n,v],…]` | record.
fn headers_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let pairs = match arg(args, 0) {
        Value::Undefined | Value::Null => Vec::new(),
        v => headers_pairs(e, v)?,
    };
    Ok(e.headers_obj(pairs))
}

/// `headers.append(name, v)` — combine `", "` (`set-cookie` exclu).
fn h_append(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let name = h_name(e, arg(args, 0))?;
    let val = h_value(e, arg(args, 1))?;
    if let Value::Obj(id) = this {
        e.headers_append_raw(id, &name, val);
    }
    Ok(Value::Undefined)
}

/// `headers.set(name, v)` — remplace toutes les entrées du nom.
fn h_set(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let name = h_name(e, arg(args, 0))?;
    let val = h_value(e, arg(args, 1))?;
    if let Value::Obj(id) = this {
        if let Some(o) = e.heap.obj_mut(id) {
            o.remove_own(&name);
            if name == "set-cookie" {
                o.remove_own(SET_COOKIE_KEY);
            }
        }
        e.headers_append_raw(id, &name, val);
    }
    Ok(Value::Undefined)
}

/// `headers.delete(name)`.
fn h_delete(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let name = h_name(e, arg(args, 0))?;
    if let Value::Obj(id) = this {
        if let Some(o) = e.heap.obj_mut(id) {
            o.remove_own(&name);
            if name == "set-cookie" {
                o.remove_own(SET_COOKIE_KEY);
            }
        }
    }
    Ok(Value::Undefined)
}

/// `headers.has(name)` → bool.
fn h_has(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let name = h_name(e, arg(args, 0))?;
    let hit = matches!(this, Value::Obj(id)
        if e.get_prop(id, &name).is_some()
            || (name == "set-cookie" && e.get_prop(id, SET_COOKIE_KEY).is_some()));
    Ok(Value::Bool(hit))
}

/// `headers.get(name)` → valeur jointe | `null` (`set-cookie` :
/// première valeur brute — spec interdit la combinaison).
fn h_get(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let name = h_name(e, arg(args, 0))?;
    let Value::Obj(id) = this else {
        return Ok(Value::Null);
    };
    if name == "set-cookie" {
        let first = set_cookie_list(e, id).into_iter().next();
        return Ok(match first {
            Some(s) => e.str_val(s),
            None => Value::Null,
        });
    }
    Ok(match e.get_prop(id, &name) {
        Some(v) => v,
        None => Value::Null,
    })
}

/// Valeurs brutes `set-cookie` de l'objet (prop sentinelle).
fn set_cookie_list(e: &Engine, id: crate::value::ObjId) -> Vec<String> {
    // `get_prop` pour lire la prop cachée (non-énumérable).
    let Some(Value::Obj(aid)) = ({
        // `Engine::get_prop` veut `&mut self` — on passe par une
        // lecture directe du heap (props sont data).
        match e.heap.obj(id) {
            Some(o) => o
                .props
                .iter()
                .find(|(k, _)| k == SET_COOKIE_KEY)
                .and_then(|(_, p)| p.data_value()),
            None => None,
        }
    }) else {
        return Vec::new();
    };
    match e.heap.obj(aid).map(|o| &o.kind) {
        Some(ObjKind::Array(elems)) => elems.iter().filter_map(|v| e.heap_string_of(*v)).collect(),
        _ => Vec::new(),
    }
}

/// `headers.getSetCookie()` → `[]` des valeurs `set-cookie` brutes
/// (les `Expires=…, …` y restent entiers — jamais recombinés).
fn h_get_set_cookie(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let out = match this {
        Value::Obj(id) => set_cookie_list(e, id)
            .into_iter()
            .map(|s| e.str_val(s))
            .collect(),
        _ => Vec::new(),
    };
    Ok(e.new_array(out))
}

/// Entrées `(nom, valeur)` triées par nom — ordre d'itération
/// spec (`lexicographical, combined`) ; `set-cookie` rapporte ses
/// valeurs brutes sans combinaison.
fn h_sorted(e: &mut Engine, this: Value) -> Vec<(String, String)> {
    let Value::Obj(id) = this else {
        return Vec::new();
    };
    let mut out = headers_pairs(e, Value::Obj(id)).unwrap_or_default();
    out.retain(|(k, _)| k != "set-cookie");
    for sc in set_cookie_list(e, id) {
        out.push(("set-cookie".into(), sc));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// `headers.forEach(cb, thisArg?)` — `(valeur, nom, headers)`.
fn h_for_each(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let cb = arg(args, 0);
    let ta = arg(args, 1);
    if !e.is_callable(cb) {
        return Err(e.type_error("forEach: callback required"));
    }
    for (k, v) in h_sorted(e, this) {
        let vv = e.str_val(v);
        let kk = e.str_val(k);
        e.call_value(cb, ta, &[vv, kk, this])?;
    }
    Ok(Value::Undefined)
}

/// `headers.entries()` / `@@iterator` — itérateur de `[nom, val]`.
fn h_entries(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let mut items = Vec::new();
    for (k, v) in h_sorted(e, this) {
        let kk = e.str_val(k);
        let vv = e.str_val(v);
        items.push(e.new_array(vec![kk, vv]));
    }
    let proto = e.protos.array_iter;
    Ok(crate::collections::iter_obj(e, items, proto))
}

/// `headers.keys()` — itérateur des noms (triés, dédupliqués).
fn h_keys(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let items = h_sorted(e, this)
        .into_iter()
        .map(|(k, _)| e.str_val(k))
        .collect();
    let proto = e.protos.array_iter;
    Ok(crate::collections::iter_obj(e, items, proto))
}

/// `headers.values()` — itérateur des valeurs (tri par nom).
fn h_values(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let items = h_sorted(e, this)
        .into_iter()
        .map(|(_, v)| e.str_val(v))
        .collect();
    let proto = e.protos.array_iter;
    Ok(crate::collections::iter_obj(e, items, proto))
}

// ---- mixin Body + détection --------------------------------------

/// `ObjKind::FetchBody` de `this` → `(ObjId, corps)`.
fn fetch_body_of(e: &Engine, this: Value) -> Option<(crate::value::ObjId, String)> {
    let Value::Obj(id) = this else { return None };
    match e.heap.obj(id).map(|o| &o.kind) {
        Some(ObjKind::FetchBody { body }) => Some((id, body.clone())),
        _ => None,
    }
}

/// Consommation unique du corps (spec `Body`) : `bodyUsed` armé à
/// la première lecture, second appel → `Err(valeur TypeError)` à
/// rejeter en promesse (pas de throw synchrone).
fn body_take(e: &mut Engine, this: Value) -> JsResult<Result<String, Value>> {
    let Some((id, body)) = fetch_body_of(e, this) else {
        let v = e.error_value(
            e.protos.type_error,
            "TypeError",
            "Body: not a Request/Response",
        );
        return Ok(Err(v));
    };
    if matches!(e.get_prop(id, "bodyUsed"), Some(Value::Bool(true))) {
        let v = e.error_value(
            e.protos.type_error,
            "TypeError",
            "Body has already been consumed",
        );
        return Ok(Err(v));
    }
    e.set_own(id, "bodyUsed", Value::Bool(true), true);
    Ok(Ok(body))
}

/// `body.text()` → promesse du corps texte.
fn body_text(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let pid = e.new_promise();
    match body_take(e, this)? {
        Ok(b) => {
            let v = e.str_val(b);
            e.resolve_promise(pid, v);
        }
        Err(err) => e.reject_promise(pid, err),
    }
    Ok(Value::Obj(pid))
}

/// `body.json()` → `JSON.parse` du corps (rejet `SyntaxError`).
fn body_json(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let pid = e.new_promise();
    match body_take(e, this)? {
        Ok(b) => {
            let txt = e.str_val(b);
            match crate::json::parse(e, txt) {
                Ok(v) => e.resolve_promise(pid, v),
                Err(crate::heap::Thrown(t)) => e.reject_promise(pid, t),
            }
        }
        Err(err) => e.reject_promise(pid, err),
    }
    Ok(Value::Obj(pid))
}

/// `body.arrayBuffer()` → `ArrayBuffer` des octets du corps.
fn body_arraybuf(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let pid = e.new_promise();
    match body_take(e, this)? {
        Ok(b) => {
            let id = e.heap.alloc_obj(crate::heap::Object {
                proto: Value::Obj(e.protos.array_buffer),
                props: Vec::new(),
                kind: ObjKind::ByteBuf(b.into_bytes()),
                extensible: true,
                elem_integrity: 0,
                args_map: None,
            });
            e.resolve_promise(pid, Value::Obj(id));
        }
        Err(err) => e.reject_promise(pid, err),
    }
    Ok(Value::Obj(pid))
}

/// `body.formData()` — décodage `application/x-www-form-urlencoded`
/// en vrai `FormData` (JS-C67).
fn body_formdata(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let pid = e.new_promise();
    match body_take(e, this)? {
        Ok(b) => {
            let pairs: Vec<(String, Value)> = crate::builtins::url::url_decode_pairs(&b)
                .into_iter()
                .map(|(k, v)| (k, e.str_val(v)))
                .collect();
            let fd = crate::dom_host::formdata_new(e, pairs);
            e.resolve_promise(pid, fd);
        }
        Err(err) => e.reject_promise(pid, err),
    }
    Ok(Value::Obj(pid))
}

/// `body.blob()` — `Blob` minimal `{size, type, text(),
/// arrayBuffer()}` (`type` = `Content-Type` ; ctor JS-C70).
fn body_blob(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let pid = e.new_promise();
    match body_take(e, this)? {
        Ok(b) => {
            let bl = blob_value(e, &b, this);
            e.resolve_promise(pid, bl);
        }
        Err(err) => e.reject_promise(pid, err),
    }
    Ok(Value::Obj(pid))
}

/// `request.clone()`/`response.clone()` — copie des own-props +
/// `Headers` frais (la liste est copiée), `bodyUsed` relu à
/// `false` ; `TypeError` si le corps est déjà consommé (spec).
fn fetch_clone(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let Value::Obj(id) = this else {
        return Err(e.type_error("clone: bad receiver"));
    };
    if matches!(e.get_prop(id, "bodyUsed"), Some(Value::Bool(true))) {
        return Err(e.type_error("cannot clone a Request/Response whose body is used"));
    }
    let (proto, body, props) = match e.heap.obj(id) {
        Some(o) => (
            o.proto,
            match &o.kind {
                ObjKind::FetchBody { body } => body.clone(),
                _ => String::new(),
            },
            o.props.clone(),
        ),
        None => return Err(e.type_error("clone: bad receiver")),
    };
    let nid = e.heap.alloc_obj(crate::heap::Object {
        proto,
        props: Vec::new(),
        kind: ObjKind::FetchBody { body },
        extensible: true,
        elem_integrity: 0,
        args_map: None,
    });
    for (k, p) in &props {
        if matches!(k.as_str(), "headers" | "bodyUsed" | "body") || k.starts_with('\u{f130}') {
            continue;
        }
        if let crate::heap::Prop::Data { value, attrs } = p {
            e.set_own(nid, k, *value, attrs.enumerable);
        }
    }
    // `headers` frais — liste recopiée (spec `clone`).
    let hpairs = match e.get_prop(id, "headers") {
        Some(h) => headers_pairs(e, h)?,
        None => Vec::new(),
    };
    // `set-cookie` brut — pas passé par `headers_pairs` (prop
    // sentinelle filtrée) : recopié explicitement.
    let sc = match e.get_prop(id, "headers") {
        Some(Value::Obj(hid)) => set_cookie_list(e, hid),
        _ => Vec::new(),
    };
    let mut hpairs = hpairs;
    for v in sc {
        hpairs.push(("set-cookie".into(), v));
    }
    let h = e.headers_obj(hpairs);
    e.set_own(nid, "headers", h, true);
    e.set_own(nid, "bodyUsed", Value::Bool(false), true);
    e.set_own(nid, "body", Value::Null, true);
    Ok(Value::Obj(nid))
}

// ---- Request -----------------------------------------------------

/// `this`/`v` est-il un `Request` (proto `Request.prototype`) ?
fn is_request(e: &Engine, v: Value) -> bool {
    let (Value::Obj(id), Some(rp)) = (v, e.xhr.req_proto) else {
        return false;
    };
    matches!(e.heap.obj(id).map(|o| o.proto), Some(Value::Obj(p)) if p == rp)
}

/// Méthode HTTP normalisée (spec : la liste canonique passe en
/// majuscules, le reste garde sa casse) — `TypeError` si le token
/// est malformé.
fn norm_method(e: &mut Engine, m: String) -> JsResult<String> {
    if !valid_header_name(&m) {
        return Err(e.type_error(format!("invalid method {m:?}")));
    }
    Ok(match m.to_lowercase().as_str() {
        "delete" | "get" | "head" | "options" | "post" | "put" => m.to_uppercase(),
        _ => m,
    })
}

/// Résolution d'URL contre `hooks.page_url` (base document) —
/// brut si la base est absente (contexte de test) ; `None` si le
/// résultat est invalide.
fn resolve_url(e: &Engine, raw: &str) -> Option<String> {
    let base = e.hooks.page_url.clone();
    if base.is_empty() {
        return Some(raw.to_string());
    }
    crate::builtins::url::url_parse_absolute(&base)
        .and_then(|b| crate::builtins::url::url_parse(raw, Some(&b)))
        .map(|p| crate::builtins::url::url_href(&p))
}

/// `new Request(input, init?)` — `input` = chaîne URL ou `Request`
/// (dont les champs servent de base avant `init`) ; `GET`/`HEAD` +
/// corps → `TypeError` (spec).
fn request_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let input = arg(args, 0);
    let init = arg(args, 1);
    if matches!(input, Value::Undefined) {
        return Err(e.type_error("Request: url required"));
    }
    let url: String;
    let (mut method, mut headers, mut body) = ("GET".to_string(), Vec::new(), String::new());
    let mut input_signal = Value::Undefined;
    if is_request(e, input) {
        let Value::Obj(rid) = input else {
            return Err(e.type_error("Request: bad input"));
        };
        if matches!(e.get_prop(rid, "bodyUsed"), Some(Value::Bool(true))) {
            return Err(e.type_error("Request: input's body is already used"));
        }
        input_signal = e.get_prop(rid, "signal").unwrap_or(Value::Undefined);
        url = e
            .get_prop(rid, "url")
            .and_then(|v| e.heap_string_of(v))
            .unwrap_or_default();
        method = e
            .get_prop(rid, "method")
            .and_then(|v| e.heap_string_of(v))
            .unwrap_or_else(|| "GET".into());
        headers = match e.get_prop(rid, "headers") {
            Some(h) => headers_pairs(e, h)?,
            None => Vec::new(),
        };
        if let Some(Value::Obj(hid)) = e.get_prop(rid, "headers") {
            for sc in set_cookie_list(e, hid) {
                headers.push(("set-cookie".into(), sc));
            }
        }
        if let Some((_, b)) = fetch_body_of(e, input) {
            body = b;
        }
    } else {
        url = e.to_js_string(input)?;
    }
    if let Value::Obj(oid) = init {
        if let Some(m) = e.get_prop(oid, "method") {
            if !matches!(m, Value::Undefined | Value::Null) {
                let ms = e.to_js_string(m)?;
                method = norm_method(e, ms)?;
            }
        }
        if let Some(hs) = e.get_prop(oid, "headers") {
            if !matches!(hs, Value::Undefined | Value::Null) {
                headers = headers_pairs(e, hs)?;
            }
        }
        if let Some(b) = e.get_prop(oid, "body") {
            if !matches!(b, Value::Undefined | Value::Null) {
                let (text, ct) = body_init_text(e, b)?;
                body = text;
                if let Some(ct) = ct {
                    if !headers
                        .iter()
                        .any(|(k, _)| k.eq_ignore_ascii_case("content-type"))
                    {
                        headers.push(("Content-Type".into(), ct));
                    }
                }
            }
        }
    }
    if matches!(method.as_str(), "GET" | "HEAD") && !body.is_empty() {
        return Err(e.type_error("Request: GET/HEAD cannot have a body"));
    }
    let Some(url) = resolve_url(e, &url) else {
        return Err(e.type_error(format!("Request: invalid URL {url:?}")));
    };
    let proto = e.xhr.req_proto.unwrap_or(e.protos.object);
    let id = e.heap.alloc_obj(crate::heap::Object {
        proto: Value::Obj(proto),
        props: Vec::new(),
        kind: ObjKind::FetchBody { body },
        extensible: true,
        elem_integrity: 0,
        args_map: None,
    });
    // `signal` : `init.signal` > signal du Request d'entrée >
    // signal frais non-aborté.
    let mut signal = match init {
        Value::Obj(oid) => e.get_prop(oid, "signal").unwrap_or(Value::Undefined),
        _ => Value::Undefined,
    };
    if matches!(signal, Value::Undefined) {
        signal = input_signal;
    }
    if matches!(signal, Value::Undefined | Value::Null) {
        signal = abort_signal_value(e);
    }
    let h = e.headers_obj(headers);
    let u = e.str_val(url);
    let m = e.str_val(method);
    let get_str = |e: &mut Engine, key: &str, default: &str| -> JsResult<Value> {
        let v = match init {
            Value::Obj(oid) => e.get_prop(oid, key),
            _ => None,
        };
        match v {
            Some(v) if !matches!(v, Value::Undefined | Value::Null) => {
                let s = e.to_js_string(v)?;
                Ok(e.str_val(s))
            }
            _ => Ok(e.str_val(default)),
        }
    };
    let keepalive = match init {
        Value::Obj(oid) => e.get_prop(oid, "keepalive"),
        _ => None,
    }
    .is_some_and(|v| e.to_boolean(v));
    let mode = get_str(e, "mode", "cors")?;
    let credentials = get_str(e, "credentials", "same-origin")?;
    let cache = get_str(e, "cache", "default")?;
    let redirect = get_str(e, "redirect", "follow")?;
    let referrer = get_str(e, "referrer", "about:client")?;
    let referrer_policy = get_str(e, "referrerPolicy", "")?;
    let integrity = get_str(e, "integrity", "")?;
    let dest = e.str_val("");
    e.set_own(id, "url", u, true);
    e.set_own(id, "method", m, true);
    e.set_own(id, "headers", h, true);
    e.set_own(id, "signal", signal, true);
    e.set_own(id, "mode", mode, true);
    e.set_own(id, "credentials", credentials, true);
    e.set_own(id, "cache", cache, true);
    e.set_own(id, "redirect", redirect, true);
    e.set_own(id, "referrer", referrer, true);
    e.set_own(id, "referrerPolicy", referrer_policy, true);
    e.set_own(id, "integrity", integrity, true);
    e.set_own(id, "keepalive", Value::Bool(keepalive), true);
    e.set_own(id, "destination", dest, true);
    e.set_own(id, "bodyUsed", Value::Bool(false), true);
    e.set_own(id, "body", Value::Null, true);
    Ok(Value::Obj(id))
}

// ---- Response ----------------------------------------------------

/// `new Response(body?, init?)` — `init.status` ∈ [200, 599] entier
/// (`RangeError` sinon, spec), `init.headers` = `HeadersInit`.
fn response_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let (body, body_ct) = match arg(args, 0) {
        Value::Undefined | Value::Null => (String::new(), None),
        v => body_init_text(e, v)?,
    };
    let init = arg(args, 1);
    let (status, status_text, mut headers) = response_init(e, init, 200)?;
    if let Some(ct) = body_ct {
        if !headers
            .iter()
            .any(|(k, _)| k.eq_ignore_ascii_case("content-type"))
        {
            headers.push(("Content-Type".into(), ct));
        }
    }
    Ok(e.response_obj(RespSpec {
        status,
        status_text,
        headers,
        body,
        ty: "default",
        url: String::new(),
        redirected: false,
    }))
}

/// Triple `(status, statusText, headers)` validé extrait d'un
/// `init` `Response` (`status` ∈ [200,599] entier sinon
/// `RangeError`).
type RespInit = (u16, String, Vec<(String, String)>);

/// `init` `{status, statusText, headers}` → [`RespInit`].
fn response_init(e: &mut Engine, init: Value, default_status: u16) -> JsResult<RespInit> {
    let Value::Obj(oid) = init else {
        return Ok((default_status, String::new(), Vec::new()));
    };
    let status = match e.get_prop(oid, "status") {
        Some(v) if !matches!(v, Value::Undefined | Value::Null) => {
            let n = e.to_number(v)?;
            if !(n.fract() == 0.0 && (200.0..=599.0).contains(&n)) {
                return Err(e.range_error("Response: status must be an integer in [200, 599]"));
            }
            n as u16
        }
        _ => default_status,
    };
    let status_text = match e.get_prop(oid, "statusText") {
        Some(v) if !matches!(v, Value::Undefined | Value::Null) => e.to_js_string(v)?,
        _ => String::new(),
    };
    let headers = match e.get_prop(oid, "headers") {
        Some(v) if !matches!(v, Value::Undefined | Value::Null) => headers_pairs(e, v)?,
        _ => Vec::new(),
    };
    Ok((status, status_text, headers))
}

/// `Response.error()` — réponse réseau-mort (`type: "error"`).
fn response_error(e: &mut Engine, _t: Value, _a: &[Value]) -> JsResult<Value> {
    Ok(e.response_obj(RespSpec {
        status: 0,
        status_text: String::new(),
        headers: Vec::new(),
        body: String::new(),
        ty: "error",
        url: String::new(),
        redirected: false,
    }))
}

/// `Response.redirect(url, status=302)` — `status` ∈ redirection
/// 3xx (`RangeError` sinon), `Location` posé.
fn response_redirect(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let url = e.to_js_string(arg(args, 0))?;
    let status = match arg(args, 1) {
        Value::Undefined => 302,
        v => e.to_number(v)? as u16,
    };
    if !matches!(status, 301 | 302 | 303 | 307 | 308) {
        return Err(e.range_error("Response.redirect: status must be a redirect status"));
    }
    let Some(resolved) = resolve_url(e, &url) else {
        return Err(e.type_error(format!("Response.redirect: invalid URL {url:?}")));
    };
    Ok(e.response_obj(RespSpec {
        status,
        status_text: String::new(),
        headers: vec![("location".into(), resolved)],
        body: String::new(),
        ty: "default",
        url: String::new(),
        redirected: false,
    }))
}

/// `Response.json(data, init?)` — corps `JSON.stringify(data)` +
/// `Content-Type: application/json`.
fn response_json(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let data = arg(args, 0);
    let init = arg(args, 1);
    let (status, status_text, mut headers) = response_init(e, init, 200)?;
    let body = {
        let jv = e.get_prop(e.global, "JSON").unwrap_or(Value::Undefined);
        let f = e.get_prop_value(jv, "stringify")?;
        match e.call_value(f, jv, &[data])? {
            Value::Undefined => String::new(),
            v => e.heap_string_of(v).unwrap_or_default(),
        }
    };
    if !headers.iter().any(|(k, _)| k == "content-type") {
        headers.push(("content-type".into(), "application/json".into()));
    }
    Ok(e.response_obj(RespSpec {
        status,
        status_text,
        headers,
        body,
        ty: "default",
        url: String::new(),
        redirected: false,
    }))
}

// ---- objets minimaux `blob()`/`formData()` -----------------------

/// `Blob` minimal de `res.blob()` (constructeur complet : JS-C70).
fn blob_value(e: &mut Engine, body: &str, this: Value) -> Value {
    let ty = match this {
        Value::Obj(id) => match e.get_prop(id, "headers") {
            Some(h) => e
                .get_prop_value(h, "content-type")
                .ok()
                .and_then(|v| e.heap_string_of(v))
                .unwrap_or_default(),
            None => String::new(),
        },
        _ => String::new(),
    };
    let o = e.new_object();
    let sz = Value::Num(body.len() as f64);
    e.set_own(o, "size", sz, true);
    let tv = e.str_val(ty);
    e.set_own(o, "type", tv, true);
    let bv = e.str_val(body.to_string());
    let t = e.bound_native("text", blob_text_fulfill, vec![bv]);
    e.set_own(o, "text", t, false);
    let bv = e.str_val(body.to_string());
    let t = e.bound_native("arrayBuffer", blob_ab_fulfill, vec![bv]);
    e.set_own(o, "arrayBuffer", t, false);
    Value::Obj(o)
}

/// `blob.text()` → promesse du texte (bound arg = corps).
fn blob_text_fulfill(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let pid = e.new_promise();
    e.resolve_promise(pid, arg(args, 0));
    Ok(Value::Obj(pid))
}

/// `blob.arrayBuffer()` → promesse `ArrayBuffer` (bound arg = corps).
fn blob_ab_fulfill(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let pid = e.new_promise();
    let bytes = e
        .heap_string_of(arg(args, 0))
        .unwrap_or_default()
        .into_bytes();
    let id = e.heap.alloc_obj(crate::heap::Object {
        proto: Value::Obj(e.protos.array_buffer),
        props: Vec::new(),
        kind: ObjKind::ByteBuf(bytes),
        extensible: true,
        elem_integrity: 0,
        args_map: None,
    });
    e.resolve_promise(pid, Value::Obj(id));
    Ok(Value::Obj(pid))
}

/// `BodyInit` → `(texte, Content-Type suggéré)` — `FormData` et
/// `URLSearchParams` sérialisent en urlencodé (déviation
/// documentée JS-C67 : la spec exige `multipart/form-data` pour
/// `FormData`, hors scope du transport `String`) ; toute autre
/// valeur passe par `ToString` sans type suggéré.
fn body_init_text(e: &mut Engine, v: Value) -> JsResult<(String, Option<String>)> {
    let encode = |pairs: &[(String, String)]| {
        pairs
            .iter()
            .map(|(k, val)| {
                format!(
                    "{}={}",
                    crate::builtins::url::form_encode(k),
                    crate::builtins::url::form_encode(val)
                )
            })
            .collect::<Vec<_>>()
            .join("&")
    };
    if let Some(pairs) = crate::dom_host::formdata_pairs_of(e, v) {
        return Ok((
            encode(&pairs),
            Some("application/x-www-form-urlencoded".into()),
        ));
    }
    if let Some(pairs) = crate::builtins::url::usp_pairs_of(e, v) {
        return Ok((
            encode(&pairs),
            Some("application/x-www-form-urlencoded;charset=UTF-8".into()),
        ));
    }
    Ok((e.to_js_string(v)?, None))
}

// ---- fetch --------------------------------------------------------

/// `fetch(url | Request, opts?)` — `input` `Request` fournit
/// url/method/headers/body par défaut, `opts` surcharge (spec
/// `fetch` = `new Request(input, init)` puis transport).
fn fetch_fn(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let input = arg(args, 0);
    let opts = arg(args, 1);
    let mut req = XhrRequest {
        method: "GET".into(),
        url: String::new(),
        headers: Vec::new(),
        body: None,
    };
    let mut signal = Value::Undefined;
    if is_request(e, input) {
        let Value::Obj(rid) = input else {
            return Err(e.type_error("fetch: bad Request"));
        };
        if matches!(e.get_prop(rid, "bodyUsed"), Some(Value::Bool(true))) {
            let pid = e.new_promise();
            let err = e.error_value(
                e.protos.type_error,
                "TypeError",
                "Request body is already used",
            );
            e.reject_promise(pid, err);
            return Ok(Value::Obj(pid));
        }
        req.url = e
            .get_prop(rid, "url")
            .and_then(|v| e.heap_string_of(v))
            .unwrap_or_default();
        req.method = e
            .get_prop(rid, "method")
            .and_then(|v| e.heap_string_of(v))
            .unwrap_or_else(|| "GET".into());
        if let Some(h) = e.get_prop(rid, "headers") {
            req.headers = headers_pairs(e, h)?;
            if let Some(Value::Obj(hid)) = e.get_prop(rid, "headers") {
                for sc in set_cookie_list(e, hid) {
                    req.headers.push(("set-cookie".into(), sc));
                }
            }
        }
        if let Some((_, b)) = fetch_body_of(e, input) {
            if !b.is_empty() {
                req.body = Some(b);
            }
        }
        signal = e.get_prop(rid, "signal").unwrap_or(Value::Undefined);
    } else {
        req.url = match input {
            Value::Undefined => return Err(e.type_error("fetch: url required")),
            v => e.to_js_string(v)?,
        };
    }
    if let Value::Obj(oid) = opts {
        if let Some(m) = e.get_prop(oid, "method") {
            if !matches!(m, Value::Undefined | Value::Null) {
                let ms = e.to_js_string(m)?;
                req.method = norm_method(e, ms)?;
            }
        }
        if let Some(hs) = e.get_prop(oid, "headers") {
            if !matches!(hs, Value::Undefined | Value::Null) {
                req.headers = headers_pairs(e, hs)?;
                if let Value::Obj(hid) = hs {
                    for sc in set_cookie_list(e, hid) {
                        req.headers.push(("set-cookie".into(), sc));
                    }
                }
            }
        }
        if let Some(b) = e.get_prop(oid, "body") {
            if !matches!(b, Value::Undefined | Value::Null) {
                let (text, ct) = body_init_text(e, b)?;
                req.body = Some(text);
                if let Some(ct) = ct {
                    if !req
                        .headers
                        .iter()
                        .any(|(k, _)| k.eq_ignore_ascii_case("content-type"))
                    {
                        req.headers.push(("Content-Type".into(), ct));
                    }
                }
            }
        }
        if let Some(s) = e.get_prop(oid, "signal") {
            signal = s;
        }
    }
    let pid = e.new_promise();
    // `{signal}` déjà aborted → rejet `AbortError` immédiat (l'abord
    // d'un transfert en vol n'est pas câblé — stub honnête).
    if let Some(d) = abort_data(e, signal) {
        if d.borrow().aborted {
            let msg = e.str_val("AbortError: signal is aborted");
            e.reject_promise(pid, msg);
            return Ok(Value::Obj(pid));
        }
    }
    if let Some(t) = &e.xhr.transport {
        let h = t.send(&req);
        e.xhr.fetches.push((h, pid, req.url.clone()));
    } else {
        // No connector — reject like a network failure.
        let msg = e.str_val("fetch: no transport");
        e.reject_promise(pid, msg);
    }
    Ok(Value::Obj(pid))
}

// ------------------------------------------------------------------
// AbortController / AbortSignal (JS-7f) — `abort()` marque le signal
// et dispatche `abort` ; `fetch` rejette un signal déjà aborted.
// ------------------------------------------------------------------

/// État partagé d'un `AbortSignal` — `(callback, once)`.
struct AbortData {
    aborted: bool,
    reason: Value,
    listeners: Vec<(Value, bool)>,
}

/// The `AbortSignal` host object.
struct AbortSignalHost {
    data: Rc<RefCell<AbortData>>,
}

/// `Value` → son `AbortData` si c'est un `AbortSignal` à nous.
fn abort_data(e: &Engine, v: Value) -> Option<Rc<RefCell<AbortData>>> {
    let Value::Obj(id) = v else {
        return None;
    };
    match e.heap.obj(id).map(|o| &o.kind) {
        Some(ObjKind::Host(h)) => h
            .clone()
            .as_any()
            .downcast_ref::<AbortSignalHost>()
            .map(|s| s.data.clone()),
        _ => None,
    }
}

impl Engine {
    /// Installe `AbortController` + `AbortSignal` sur le global —
    /// complément honnête de `fetch` (stub : pas d'abord d'un
    /// transfert en vol, seul le rejet si déjà aborted est câblé).
    fn install_abort(&mut self) {
        let g = self.global;
        let ac = self.native_fn("AbortController", abort_controller_ctor);
        self.set_own(g, "AbortController", ac, false);
        let asig = self.native_fn("AbortSignal", abort_signal_ctor);
        if let Value::Obj(cid) = asig {
            // `AbortSignal.abort(reason?)` — signal déjà aborted.
            let f = self.native_fn("abort", abort_signal_static_abort);
            self.set_own(cid, "abort", f, false);
        }
        self.set_own(g, "AbortSignal", asig, false);
    }
}

impl Host for AbortSignalHost {
    fn class_name(&self) -> &'static str {
        "AbortSignal"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn get(&self, _e: &mut Engine, _t: Value, key: &str) -> JsResult<Option<Value>> {
        Ok(match key {
            "aborted" => Some(Value::Bool(self.data.borrow().aborted)),
            "reason" => {
                let d = self.data.borrow();
                if d.aborted {
                    Some(d.reason)
                } else {
                    Some(Value::Undefined)
                }
            }
            _ => None,
        })
    }

    fn invoke(
        &self,
        e: &mut Engine,
        _this: Value,
        name: &str,
        args: &[Value],
    ) -> JsResult<Option<Value>> {
        match name {
            "addEventListener" => {
                let ty = e.to_js_string(arg(args, 0))?;
                let f = arg(args, 1);
                if ty == "abort" && e.is_callable(f) {
                    let once = matches!(arg(args, 2), Value::Obj(o)
                        if matches!(e.get_prop(o, "once"), Some(Value::Bool(true))));
                    self.data.borrow_mut().listeners.push((f, once));
                }
                Ok(Some(Value::Undefined))
            }
            "removeEventListener" => {
                let ty = e.to_js_string(arg(args, 0))?;
                let f = arg(args, 1);
                self.data
                    .borrow_mut()
                    .listeners
                    .retain(|(g, _)| ty != "abort" || *g != f);
                Ok(Some(Value::Undefined))
            }
            "throwIfAborted" => {
                let r = {
                    let d = self.data.borrow();
                    d.aborted.then_some(d.reason)
                };
                if let Some(r) = r {
                    return Err(crate::heap::Thrown(if matches!(r, Value::Undefined) {
                        e.str_val("AbortError")
                    } else {
                        r
                    }));
                }
                Ok(Some(Value::Undefined))
            }
            _ => Ok(None),
        }
    }

    fn trace(&self, out: &mut Vec<crate::value::ObjId>) {
        for (f, _) in &self.data.borrow().listeners {
            if let Value::Obj(id) = f {
                out.push(*id);
            }
        }
    }

    fn keys(&self, _e: &mut Engine, _t: Value) -> Vec<String> {
        vec!["aborted".into()]
    }
}

/// `new AbortSignal()` — `TypeError` per spec ("illegal
/// constructor" : un signal ne vient que d'un contrôleur ou d'un
/// constructeur statique).
fn abort_signal_ctor(e: &mut Engine, _t: Value, _a: &[Value]) -> JsResult<Value> {
    Err(e.type_error("AbortSignal: illegal constructor"))
}

/// `AbortSignal.abort(reason?)` — signal déjà aborted.
fn abort_signal_static_abort(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let reason = arg(args, 0);
    Ok(e.new_host_object(Rc::new(AbortSignalHost {
        data: Rc::new(RefCell::new(AbortData {
            aborted: true,
            reason,
            listeners: Vec::new(),
        })),
    })))
}

/// `AbortSignal` neuf non-aborté — signal par défaut de
/// `new Request` quand `init.signal` est absent (JS-C66).
fn abort_signal_value(e: &mut Engine) -> Value {
    e.new_host_object(Rc::new(AbortSignalHost {
        data: Rc::new(RefCell::new(AbortData {
            aborted: false,
            reason: Value::Undefined,
            listeners: Vec::new(),
        })),
    }))
}

/// `new AbortController()` — `{signal, abort()}` : le signal est un
/// host, `abort` un natif bound sur sa `Value`.
fn abort_controller_ctor(e: &mut Engine, _t: Value, _a: &[Value]) -> JsResult<Value> {
    let signal = abort_signal_value(e);
    let ctrl = e.new_object();
    e.set_own(ctrl, "signal", signal, true);
    let f = e.bound_native("abort", controller_abort, vec![signal]);
    e.set_own(ctrl, "abort", f, false);
    Ok(Value::Obj(ctrl))
}

/// `controller.abort(reason?)` — marque le signal (idempotent) et
/// dispatche `abort` : `onabort` puis les listeners.
fn controller_abort(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let sig = arg(args, 0);
    let reason = arg(args, 1);
    let Some(data) = abort_data(e, sig) else {
        return Ok(Value::Undefined);
    };
    {
        let mut d = data.borrow_mut();
        if d.aborted {
            return Ok(Value::Undefined);
        }
        d.aborted = true;
        d.reason = reason;
    }
    let ev = {
        let o = e.new_object();
        let t = e.str_val("abort");
        e.set_own(o, "type", t, true);
        e.set_own(o, "target", sig, true);
        Value::Obj(o)
    };
    if let Value::Obj(sid) = sig {
        if let Some(f) = e.get_prop(sid, "onabort") {
            if e.is_callable(f) {
                e.call_value(f, sig, &[ev])?;
            }
        }
    }
    let listeners = data.borrow().listeners.clone();
    for (f, once) in listeners {
        if e.is_callable(f) {
            e.call_value(f, sig, &[ev])?;
        }
        if once {
            data.borrow_mut().listeners.retain(|(g, _)| *g != f);
        }
    }
    Ok(Value::Undefined)
}
