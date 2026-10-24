// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! `WebSocket` — stub honnête (JS-C73).
//!
//! Le moteur n'ouvre pas de socket (`explorer-net` seul a ce
//! droit, et pas de transport WS à ce jour) : le handshake est
//! refusé **de façon asynchrone** — `error` puis `close`
//! `{code: 1006, wasClean: false}` au checkpoint microtask,
//! exactement la séquence d'une connexion refusée. C'est ce
//! qui arme les fallbacks long-polling des frameworks
//! (Socket.IO, `SockJS`…) : `typeof WebSocket === 'function'`
//! passe leur feature-detect, et l'échec réel les route vers
//! le transport de repli.
//!
//! Surface spec : `readyState`, `url`, `protocol`/`extensions`
//! (`''` — rien de négocié), `bufferedAmount`, `binaryType`
//! (`'blob'`/`'arraybuffer'`), `send` (`InvalidStateError` en
//! `CONNECTING`, silencieux ensuite — jamais `OPEN`), `close`
//! (code/raison validés), `onopen`/`onmessage`/`onerror`/
//! `onclose` + `addEventListener`/`removeEventListener`/
//! `dispatchEvent`, constantes sur le ctor et le prototype.

use std::cell::RefCell;
use std::rc::Rc;

use crate::heap::{Host, JsResult, ObjKind};
use crate::value::{ObjId, Value};
use crate::Engine;

/// État moteur `WebSocket` — proto paresseux + sockets en
/// `CONNECTING` dont le refus reste à livrer.
#[derive(Default)]
pub(crate) struct WsState {
    /// `WebSocket.prototype` paresseux.
    ws_proto: Option<ObjId>,
    /// Wrappers dont le refus de handshake est en file —
    /// drainés au checkpoint et racines GC jusque-là.
    pending: Vec<ObjId>,
}

impl WsState {
    /// Racines GC : proto installé + sockets en attente de
    /// refus (le `Host::trace` du wrapper couvre listeners).
    pub(crate) fn root_ids(&self, ids: &mut Vec<ObjId>) {
        if let Some(p) = self.ws_proto {
            ids.push(p);
        }
        ids.extend(self.pending.iter().copied());
    }
}

/// Données partagées d'un `WebSocket` (slots spec).
struct WsData {
    /// `0` CONNECTING, `1` OPEN, `2` CLOSING, `3` CLOSED —
    /// `OPEN` inatteignable dans le stub (handshake refusé).
    ready: u8,
    /// URL absolue résolue (`ws:`/`wss:` validés au ctor).
    url: String,
    /// Sous-protocole retenu — `''` (aucune négociation).
    protocol: String,
    /// Extensions négociées — `''`.
    extensions: String,
    /// `'blob'` | `'arraybuffer'` (set validé, spec).
    binary_type: String,
    /// Octets « envoyés » non livrés — spec : `send` en
    /// `CLOSING`/`CLOSED` l'incrémente silencieusement.
    buffered: u64,
    /// `close()` demandé en `CONNECTING` (spec : `CLOSING` —
    /// le refus livre quand même `error` + `close`).
    close_requested: bool,
    /// `addEventListener` handlers `(type, fn, capture)`.
    listeners: Vec<(String, Value, bool)>,
}

/// Host `WebSocket`.
struct WebSocketHost {
    data: Rc<RefCell<WsData>>,
}

/// `WsData` d'un `Value` `WebSocket` — `None` sinon.
fn ws_data(e: &Engine, v: Value) -> Option<Rc<RefCell<WsData>>> {
    let Value::Obj(id) = v else { return None };
    match e.heap.obj(id).map(|o| &o.kind) {
        Some(ObjKind::Host(h)) => h
            .clone()
            .as_any()
            .downcast_ref::<WebSocketHost>()
            .map(|w| w.data.clone()),
        _ => None,
    }
}

/// Erreur shape `DOMException` `{name, message}` — même
/// approximation qu'IDB (`.name` est le champ testé).
fn ws_err(e: &mut Engine, name: &str, msg: &str) -> Value {
    let o = e.new_object();
    let n = e.str_val(name);
    e.set_own(o, "name", n, true);
    let m = e.str_val(msg);
    e.set_own(o, "message", m, true);
    Value::Obj(o)
}

/// `WebSocket.prototype` paresseux — constantes + toStringTag
/// (méthodes via `invoke` du host).
fn ws_proto(e: &mut Engine) -> ObjId {
    if let Some(p) = e.ws.ws_proto {
        return p;
    }
    let p = e.new_object();
    for (k, n) in [
        ("CONNECTING", 0.0),
        ("OPEN", 1.0),
        ("CLOSING", 2.0),
        ("CLOSED", 3.0),
    ] {
        e.set_own(p, k, Value::Num(n), true);
    }
    let tag = e.str_val("WebSocket");
    e.set_own(p, crate::interp::SYM_TO_STRING_TAG, tag, false);
    e.ws.ws_proto = Some(p);
    p
}

/// Sous-protocole valide ? Token ASCII sans séparateurs HTTP
/// (spec : chaque élément doit être un token `Sec-WebSocket-
/// Protocol` bien formé — `SyntaxError` sinon).
fn valid_ws_token(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|b| (0x21..=0x7e).contains(&b) && !b"()<>@,;:\\\"/[]?={} \t".contains(&b))
}

/// `new WebSocket(url, protocols?)` — résolution d'URL contre
/// `page_url`, schéma `ws`/`wss` exigé (`SyntaxError` sinon),
/// `protocols` = chaîne ou liste de tokens uniques.
fn websocket_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let raw = arg(args, 0);
    if matches!(raw, Value::Undefined) {
        return Err(e.type_error("WebSocket: url required"));
    }
    let raw = e.to_js_string(raw)?;
    let Some(url) = crate::xhr::resolve_url(e, &raw) else {
        return Err(e.syntax_error("WebSocket: invalid url"));
    };
    let scheme_ok = crate::builtins::url::url_parse_absolute(&url)
        .is_some_and(|p| matches!(crate::builtins::url::url_scheme(&p), "ws" | "wss"));
    if !scheme_ok {
        return Err(e.syntax_error("WebSocket: url scheme must be ws or wss"));
    }
    // `protocols` : string unique ou liste itérable —
    // doublons et tokens mal formés → `SyntaxError` (spec).
    let protos = arg(args, 1);
    if !matches!(protos, Value::Undefined) {
        let mut seen: Vec<String> = Vec::new();
        let mut push = |e: &mut Engine, v: Value| -> JsResult<()> {
            let s = e.to_js_string(v)?;
            if !valid_ws_token(&s) {
                return Err(e.syntax_error("WebSocket: invalid subprotocol"));
            }
            if seen.iter().any(|p| p == &s) {
                return Err(e.syntax_error("WebSocket: duplicate subprotocol"));
            }
            seen.push(s);
            Ok(())
        };
        match protos {
            Value::Obj(_) => {
                for v in e.iter_values(protos)? {
                    push(e, v)?;
                }
            }
            _ => push(e, protos)?,
        }
    }
    let v = e.new_host_object(Rc::new(WebSocketHost {
        data: Rc::new(RefCell::new(WsData {
            ready: 0,
            url,
            protocol: String::new(),
            extensions: String::new(),
            binary_type: "blob".into(),
            buffered: 0,
            close_requested: false,
            listeners: Vec::new(),
        })),
    }));
    let p = ws_proto(e);
    if let Value::Obj(id) = v {
        if let Some(o) = e.heap.obj_mut(id) {
            o.proto = Value::Obj(p);
        }
        // Handshake en file → refus livré au checkpoint.
        e.ws.pending.push(id);
    }
    Ok(v)
}

impl Engine {
    /// Refus de handshake des `WebSocket` en file (JS-C73) —
    /// `error` puis `close` `{code: 1006, wasClean: false}`
    /// par socket, ordre d'ouverture. Asynchrone = la
    /// séquence observable d'un vrai refus réseau ; c'est ce
    /// qui déclenche les fallbacks long-polling des
    /// frameworks.
    ///
    /// Retourne le nombre de sockets notifiées.
    ///
    /// # Errors
    /// Propage la première exception d'un handler (même
    /// modèle que les listeners DOM).
    pub(crate) fn deliver_ws(&mut self) -> JsResult<usize> {
        let pending = std::mem::take(&mut self.ws.pending);
        let mut n = 0usize;
        for wid in pending {
            let Some(data) = ws_data(self, Value::Obj(wid)) else {
                continue;
            };
            n += 1;
            self.fire_ws_event(wid, &data, "error", None)?;
            {
                let mut d = data.borrow_mut();
                d.ready = 3;
            }
            self.fire_ws_event(wid, &data, "close", Some((1006.0, String::new(), false)))?;
        }
        Ok(n)
    }

    /// Event `WebSocket` minimal : `Event` `{type, target}`
    /// pour `error` ; `CloseEvent` `{type, target, code,
    /// reason, wasClean}` pour `close` — `on<ty>` own-prop
    /// puis `addEventListener` (même fan-out que `FileReader`).
    fn fire_ws_event(
        &mut self,
        wrapper: ObjId,
        data: &Rc<RefCell<WsData>>,
        ty: &str,
        close: Option<(f64, String, bool)>,
    ) -> JsResult<()> {
        let this = Value::Obj(wrapper);
        let ev = {
            let o = self.new_object();
            let t = self.str_val(ty);
            self.set_own(o, "type", t, true);
            self.set_own(o, "target", this, true);
            if let Some((code, reason, clean)) = close {
                self.set_own(o, "code", Value::Num(code), true);
                let r = self.str_val(reason);
                self.set_own(o, "reason", r, true);
                self.set_own(o, "wasClean", Value::Bool(clean), true);
            }
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

impl Host for WebSocketHost {
    fn class_name(&self) -> &'static str {
        "WebSocket"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn get(&self, e: &mut Engine, _t: Value, key: &str) -> JsResult<Option<Value>> {
        let d = self.data.borrow();
        Ok(Some(match key {
            "readyState" => Value::Num(f64::from(d.ready)),
            "url" => return Ok(Some(e.str_val(d.url.clone()))),
            "protocol" => return Ok(Some(e.str_val(d.protocol.clone()))),
            "extensions" => return Ok(Some(e.str_val(d.extensions.clone()))),
            "binaryType" => return Ok(Some(e.str_val(d.binary_type.clone()))),
            "bufferedAmount" => Value::Num(d.buffered as f64),
            "CONNECTING" => Value::Num(0.0),
            "OPEN" => Value::Num(1.0),
            "CLOSING" => Value::Num(2.0),
            "CLOSED" => Value::Num(3.0),
            _ => return Ok(None),
        }))
    }

    fn set(&self, e: &mut Engine, this: Value, key: &str, val: Value) -> JsResult<bool> {
        if key == "binaryType" {
            let s = e.to_js_string(val)?;
            if matches!(s.as_str(), "blob" | "arraybuffer") {
                self.data.borrow_mut().binary_type = s;
            }
            // Spec : toute autre valeur ignorée silencieusement.
            return Ok(true);
        }
        // `on<event>` et attributs divers → own prop (défaut).
        if let Value::Obj(id) = this {
            e.set_own(id, key, val, false);
        }
        Ok(true)
    }

    fn invoke(
        &self,
        e: &mut Engine,
        this: Value,
        name: &str,
        args: &[Value],
    ) -> JsResult<Option<Value>> {
        match name {
            "send" => {
                let d = self.data.borrow();
                if d.ready == 0 {
                    // Spec : `send` en CONNECTING →
                    // `InvalidStateError`.
                    return Err(crate::heap::Thrown(ws_err(
                        e,
                        "InvalidStateError",
                        "WebSocket: still CONNECTING",
                    )));
                }
                // CLOSING/CLOSED (OPEN jamais atteint dans le
                // stub) : silencieux, `bufferedAmount` +=
                // taille du message (spec).
                let len = match arg(args, 0) {
                    Value::Undefined => 0,
                    v => e.to_js_string(v).map_or(0, |s| s.len()) as u64,
                };
                drop(d);
                self.data.borrow_mut().buffered += len;
                Ok(Some(Value::Undefined))
            }
            "close" => {
                let code = arg(args, 0);
                if !matches!(code, Value::Undefined) {
                    // IDL `unsigned short` : tronqué — 1000
                    // ou 3000-4999, sinon `InvalidAccessError`.
                    let c = e.to_number(code)? as i64;
                    if !(c == 1000 || (3000..=4999).contains(&c)) {
                        return Err(crate::heap::Thrown(ws_err(
                            e,
                            "InvalidAccessError",
                            "WebSocket: invalid close code",
                        )));
                    }
                }
                let reason = arg(args, 1);
                if !matches!(reason, Value::Undefined) {
                    // Spec : raison ≤ 123 octets UTF-8.
                    if e.to_js_string(reason)?.len() > 123 {
                        return Err(e.syntax_error("WebSocket: close reason too long"));
                    }
                }
                let mut d = self.data.borrow_mut();
                if d.ready == 0 {
                    // CONNECTING → CLOSING ; le refus déjà en
                    // file livrera `error` + `close` (spec :
                    // la connexion échoue quand même).
                    d.ready = 2;
                    d.close_requested = true;
                }
                // CLOSING/CLOSED : no-op (spec).
                Ok(Some(Value::Undefined))
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
                self.data
                    .borrow_mut()
                    .listeners
                    .retain(|(t, g, _)| !(*t == ty && e.strict_eq(*g, f)));
                Ok(Some(Value::Undefined))
            }
            "dispatchEvent" => {
                let ev = arg(args, 0);
                let ty = if let Value::Obj(id) = ev {
                    e.get_prop(id, "type")
                        .and_then(|v| e.heap_string_of(v))
                        .unwrap_or_default()
                } else {
                    String::new()
                };
                if let Value::Obj(wid) = this {
                    if ty == "close" {
                        let code = if let Value::Obj(id) = ev {
                            e.get_prop(id, "code").and_then(|v| e.to_number(v).ok())
                        } else {
                            None
                        };
                        let reason = if let Value::Obj(id) = ev {
                            e.get_prop(id, "reason").and_then(|v| e.heap_string_of(v))
                        } else {
                            None
                        };
                        let clean = if let Value::Obj(id) = ev {
                            e.get_prop(id, "wasClean").map(|v| e.to_boolean(v))
                        } else {
                            None
                        };
                        e.fire_ws_event(
                            wid,
                            &self.data,
                            "close",
                            Some((
                                code.unwrap_or(0.0),
                                reason.unwrap_or_default(),
                                clean.unwrap_or(false),
                            )),
                        )?;
                    } else if !ty.is_empty() {
                        e.fire_ws_event(wid, &self.data, &ty, None)?;
                    }
                }
                Ok(Some(Value::Bool(true)))
            }
            _ => Ok(None),
        }
    }

    fn trace(&self, out: &mut Vec<ObjId>) {
        let d = self.data.borrow();
        for (_, cb, _) in &d.listeners {
            if let Value::Obj(id) = cb {
                out.push(*id);
            }
        }
    }
}

/// `install_websocket` — global `WebSocket` + constantes sur
/// le ctor (JS-C73) — idempotent (`install_fetch` et
/// `install_dom` l'appellent tous deux).
pub(crate) fn install_websocket(e: &mut Engine) {
    let g = e.global;
    if e.get_prop(g, "WebSocket").is_some() {
        return;
    }
    let sc = e.native_fn("WebSocket", websocket_ctor as crate::heap::NativeFn);
    if let Value::Obj(cid) = sc {
        let p = ws_proto(e);
        e.set_own(cid, "prototype", Value::Obj(p), false);
        e.set_own(p, "constructor", sc, false);
        // Constantes statiques `WebSocket.CONNECTING/…`
        // (spec — doublon des props du proto et du host).
        for (k, n) in [
            ("CONNECTING", 0.0),
            ("OPEN", 1.0),
            ("CLOSING", 2.0),
            ("CLOSED", 3.0),
        ] {
            e.set_own(cid, k, Value::Num(n), true);
        }
    }
    e.set_own(g, "WebSocket", sc, false);
}

/// `args[i]` ou `Undefined` (mirroir du helper files.rs).
fn arg(args: &[Value], i: usize) -> Value {
    args.get(i).copied().unwrap_or(Value::Undefined)
}
