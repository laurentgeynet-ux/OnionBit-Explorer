// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! `XMLHttpRequest` over an injectable transport (JS-4).
//!
//! dillo-js **never touches the network itself** (§4.11): the
//! embedder installs an [`XhrTransport`] (backed by `dillo-net` in
//! the browser, by an in-process stub in tests). The flow mirrors
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
    /// In-flight `fetch` calls: `(handle, promise ObjId)`.
    pub fetches: Vec<(u64, crate::value::ObjId)>,
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
        if let Some(pos) = self.xhr.fetches.iter().position(|(h, _)| *h == resp.handle) {
            let (_, pid) = self.xhr.fetches.remove(pos);
            if resp.status == 0 {
                let msg = self.str_val("fetch: network error");
                self.reject_promise(pid, msg);
            } else {
                let r = self.make_fetch_response(resp);
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
                let body = match arg(args, 0) {
                    Value::Undefined | Value::Null => None,
                    v => Some(e.to_js_string(v)?),
                };
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
// fetch (JS-7e) — same transport, promise result
// ------------------------------------------------------------------

impl Engine {
    /// Install `fetch(url, opts?)` — returns a `Promise` resolved
    /// with a `Response`-ish object once the transport completes.
    pub fn install_fetch(&mut self) {
        let v = self.native_fn("fetch", fetch_fn);
        let g = self.global;
        self.set_own(g, "fetch", v, false);
    }

    /// Build the `Response` object for a completed fetch — plain
    /// object whose `ObjKind::FetchResponse` carries the payload;
    /// `text()`/`json()` are own-prop natives, `headers.get` a bound
    /// native over the response object.
    fn make_fetch_response(&mut self, resp: XhrResponse) -> Value {
        let id = self.heap.alloc_obj(crate::heap::Object {
            proto: Value::Obj(self.protos.object),
            props: Vec::new(),
            kind: ObjKind::FetchResponse {
                headers: resp.headers,
                body: String::from_utf8_lossy(&resp.body).into_owned(),
            },
            frozen: false,
        });
        let st = self.str_val(resp.status_text);
        let ok = Value::Bool((200..300).contains(&resp.status));
        self.set_own(id, "ok", ok, true);
        self.set_own(id, "status", Value::Num(f64::from(resp.status)), true);
        self.set_own(id, "statusText", st, true);
        // `headers` — own keys + `get(name)` (bound to the response).
        let hdrs = self.new_object();
        let headers_list = match self.heap.obj(id).map(|o| &o.kind) {
            Some(ObjKind::FetchResponse { headers, .. }) => headers.clone(),
            _ => Vec::new(),
        };
        for (k, v) in &headers_list {
            let vv = self.str_val(v.clone());
            self.set_own(hdrs, &k.to_lowercase(), vv, true);
        }
        let get_fn = self.bound_native("get", resp_header_get, vec![Value::Obj(id)]);
        self.set_own(hdrs, "get", get_fn, false);
        self.set_own(id, "headers", Value::Obj(hdrs), true);
        let text_fn = self.native_fn("text", resp_text);
        let json_fn = self.native_fn("json", resp_json);
        self.set_own(id, "text", text_fn, false);
        self.set_own(id, "json", json_fn, false);
        Value::Obj(id)
    }
}

fn resp_body(e: &Engine, this: Value) -> String {
    match this {
        Value::Obj(id) => match e.heap.obj(id).map(|o| &o.kind) {
            Some(ObjKind::FetchResponse { body, .. }) => body.clone(),
            _ => String::new(),
        },
        _ => String::new(),
    }
}

/// `response.text()` → resolved promise of the body.
fn resp_text(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let body = resp_body(e, this);
    let pid = e.new_promise();
    let v = e.str_val(body);
    e.resolve_promise(pid, v);
    Ok(Value::Obj(pid))
}

/// `response.json()` → `JSON.parse` of the body as a promise.
fn resp_json(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let body = resp_body(e, this);
    let pid = e.new_promise();
    let txt = e.str_val(body);
    match crate::json::parse(e, txt) {
        Ok(v) => e.resolve_promise(pid, v),
        Err(crate::heap::Thrown(t)) => e.reject_promise(pid, t),
    }
    Ok(Value::Obj(pid))
}

/// `headers.get(name)` — case-insensitive; the bound arg is the
/// parent response object (`BoundNative` prepends it to `args`).
fn resp_header_get(e: &mut Engine, _this: Value, args: &[Value]) -> JsResult<Value> {
    let resp = arg(args, 0);
    let name = e.to_js_string(arg(args, 1))?.to_lowercase();
    let Value::Obj(id) = resp else {
        return Ok(Value::Null);
    };
    let v = match e.heap.obj(id).map(|o| &o.kind) {
        Some(ObjKind::FetchResponse { headers, .. }) => headers
            .iter()
            .find(|(k, _)| k.to_lowercase() == name)
            .map(|(_, v)| v.clone()),
        _ => None,
    };
    Ok(match v {
        Some(s) => e.str_val(s),
        None => Value::Null,
    })
}

/// `fetch(url, opts?)` — `{method, headers, body}` subset.
fn fetch_fn(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let url = match arg(args, 0) {
        Value::Undefined => return Err(e.type_error("fetch: url required")),
        v => e.to_js_string(v)?,
    };
    let opts = arg(args, 1);
    let mut req = XhrRequest {
        method: "GET".into(),
        url,
        headers: Vec::new(),
        body: None,
    };
    if let Value::Obj(oid) = opts {
        if let Some(m) = e.get_prop(oid, "method") {
            req.method = e.to_js_string(m)?;
        }
        if let Some(hs) = e.get_prop(oid, "headers") {
            if let Value::Obj(hid) = hs {
                for k in e.own_enum_keys(hid) {
                    let hv = e.get_prop_value(hs, &k)?;
                    req.headers.push((k, e.to_js_string(hv)?));
                }
            }
        }
        if let Some(b) = e.get_prop(oid, "body") {
            if !matches!(b, Value::Undefined | Value::Null) {
                req.body = Some(e.to_js_string(b)?);
            }
        }
    }
    let pid = e.new_promise();
    if let Some(t) = &e.xhr.transport {
        let h = t.send(&req);
        e.xhr.fetches.push((h, pid));
    } else {
        // No connector — reject like a network failure.
        let msg = e.str_val("fetch: no transport");
        e.reject_promise(pid, msg);
    }
    Ok(Value::Obj(pid))
}
