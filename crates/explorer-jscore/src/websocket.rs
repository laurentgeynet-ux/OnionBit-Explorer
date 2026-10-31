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
/// `CONNECTING` dont le refus reste à livrer + flux vivants
/// ouverts via la couture [`WsTransport`] (JS-C94).
#[derive(Default)]
pub(crate) struct WsState {
    /// `WebSocket.prototype` paresseux.
    ws_proto: Option<ObjId>,
    /// Wrappers dont le refus de handshake est en file —
    /// drainés au checkpoint et racines GC jusque-là
    /// (chemin « sans transport » du stub).
    pending: Vec<ObjId>,
    /// Flux vivants : `(handle, wrapper ObjId)` — le handle
    /// vient de `WsTransport::open`, les livraisons
    /// `open`/`message`/`error`/`close` arrivent par
    /// [`Engine::deliver_ws_event`]. Racines GC tant que
    /// non `CLOSED`.
    pub live: Vec<(u64, ObjId)>,
}

/// Ouverture demandée par `new WebSocket` (JS-C94) — l'URL
/// est déjà résolue absolue et les sous-protocoles validés
/// (`token` uniques) par le moteur.
#[derive(Debug, Clone)]
pub struct WsRequest {
    /// URL absolue `ws://`/`wss://`.
    pub url: String,
    /// Sous-protocoles demandés (ordre script — le serveur
    /// choisit, le moteur restitue dans `ws.protocol`).
    pub protocols: Vec<String>,
    /// Contexte émetteur (JS-C78/134 — base URL/cookies/
    /// origine du **cadre**, jamais du parent).
    pub ctx: crate::context::CtxId,
}

/// Événement serveur→script — le miroir session
/// (`explorer-net::ws::WsEvent`) traduit côté `explorer-js`.
#[derive(Debug, Clone)]
pub enum WsEventKind {
    /// `101` validé — `protocol` négocié (vide si aucun).
    Open(String),
    /// Message texte (UTF-8 validé côté codec).
    Text(String),
    /// Message binaire → `Blob`-shape `{type:''}` ou
    /// `ArrayBuffer` selon `binaryType` (borne : pas de
    /// `Blob` réel — l'objet porte `size`/`arrayBuffer`).
    Binary(Vec<u8>),
    /// `CloseEvent` : `code` + `reason` + `wasClean`.
    Close {
        /// Code RFC 6455 §7.4.
        code: u16,
        /// Raison UTF-8.
        reason: String,
        /// `true` = close propre (handshake §7.1.2 bouclé),
        /// `false` = coupure (1006).
        clean: bool,
    },
    /// `error` — transport mort ; la session enchaîne un
    /// `close` `{1006, wasClean:false}` (spec « fail the
    /// WebSocket connection »).
    Error,
}

/// Sortant — `send`/`close` du script vers la plateforme.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WsOut {
    /// Trame texte (`send(string)`).
    Text(String),
    /// Trame binaire (`send(ArrayBuffer/Blob)`).
    Binary(Vec<u8>),
    /// `close(code, reason)`.
    Close {
        /// Code validé (1000 ou 3000-4999 — spec).
        code: u16,
        /// Raison ≤ 123 octets (validée).
        reason: String,
    },
}

/// Couture socket de la plateforme (JS-C94) — **engine-
/// global** comme `crypto_transport`/`es_transport` : tous
/// les contextes ouvrent par la même dial. Object-safe,
/// non bloquant : `open` enfile et rend un handle, les
/// événements reviennent par [`Engine::deliver_ws_event`].
pub trait WsTransport {
    /// Ouvre la connexion ; handle écho des livraisons.
    fn open(&self, req: &WsRequest) -> u64;
    /// `send` — texte ou binaire (spec : `send` en `OPEN`).
    fn send(&self, handle: u64, out: WsOut);
    /// `close` — best-effort ; la plateforme répond par
    /// `Close` (propre) ou `Error`→1006 (sale).
    fn close(&self, _handle: u64, _code: u16, _reason: &str) {}
}

impl WsState {
    /// Racines GC : proto installé + sockets en attente de
    /// refus + flux vivants (les `Host::trace` des wrappers
    /// couvrent leurs listeners).
    pub(crate) fn root_ids(&self, ids: &mut Vec<ObjId>) {
        if let Some(p) = self.ws_proto {
            ids.push(p);
        }
        ids.extend(self.pending.iter().copied());
        ids.extend(self.live.iter().map(|(_, w)| *w));
    }
}

/// Données partagées d'un `WebSocket` (slots spec).
struct WsData {
    /// `0` CONNECTING, `1` OPEN, `2` CLOSING, `3` CLOSED.
    ready: u8,
    /// URL absolue résolue (`ws:`/`wss:` validés au ctor).
    url: String,
    /// Handle couture (`Some` = flux réel ouvert via
    /// `WsTransport`, `None` = refus différé sans transport).
    handle: Option<u64>,
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
    // WebIDL `required` = argument ABSENT ; `WebSocket(undefined)`
    // stringify `"undefined"` → résout `<base>/undefined` (spec).
    if args.is_empty() {
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
    let mut seen: Vec<String> = Vec::new();
    let protos = arg(args, 1);
    if !matches!(protos, Value::Undefined) {
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
            url: url.clone(),
            handle: None,
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
        match &e.ws_transport {
            // Couture installée → ouverture réelle ; les
            // événements arrivent par `deliver_ws_event`.
            Some(tr) => {
                let h = tr.open(&WsRequest {
                    url,
                    protocols: seen,
                    ctx: e.cur_ctx,
                });
                if let Some(data) = ws_data(e, v) {
                    data.borrow_mut().handle = Some(h);
                }
                e.ws.live.push((h, id));
            }
            // Sans transport : refus différé (stub JS-C73 —
            // les fallbacks long-polling s'arment).
            None => e.ws.pending.push(id),
        }
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

    /// Livraison d'un événement de flux réel (JS-C94) —
    /// même forme que [`Engine::deliver_es`] : localise le
    /// contexte propriétaire par handle (courant ou garé),
    /// switch, livre, restaure.
    ///
    /// `open` : `readyState`→1 + `protocol` négocié posé.
    /// `message` : `MessageEvent` `{type, target, data}`
    /// — texte = `String`, binaire = `ArrayBuffer` si
    /// `binaryType==='arraybuffer'`, sinon objet
    /// `{size, type:''}` (borne : pas de `Blob` réel).
    /// `error` : event `error` simple. `close` :
    /// `readyState`→3 + `CloseEvent` `{code,reason,wasClean}`
    /// + retrait du registre.
    ///
    /// # Errors
    /// Propage la première exception d'un handler.
    pub fn deliver_ws_event(&mut self, ctx: crate::context::CtxId, handle: u64, ev: WsEventKind) {
        let owner = if self.ws.live.iter().any(|(h, _)| *h == handle) {
            Some(self.cur_ctx)
        } else {
            self.parked
                .iter()
                .find(|(_, c)| c.ws.live.iter().any(|(h, _)| *h == handle))
                .map(|(k, _)| *k)
        };
        if owner != Some(ctx) {
            // Handle orphelin ou mauvais ctx — la session
            // sait normalement router ; on tente le
            // propriétaire trouvé, sinon drop.
            if let Some(o) = owner {
                let home = self.cur_ctx;
                self.switch_to(o);
                self.deliver_ws_here(handle, ev);
                if self.cur_ctx != home {
                    self.switch_to(home);
                }
            }
            return;
        }
        let home = self.cur_ctx;
        self.deliver_ws_here(handle, ev);
        if self.cur_ctx != home {
            self.switch_to(home);
        }
    }

    /// Livraison dans le contexte courant (voir
    /// [`Engine::deliver_ws_event`] pour la forme des
    /// événements).
    fn deliver_ws_here(&mut self, handle: u64, ev: WsEventKind) {
        let Some(pos) = self.ws.live.iter().position(|(h, _)| *h == handle) else {
            return;
        };
        let wid = self.ws.live[pos].1;
        let Some(data) = ws_data(self, Value::Obj(wid)) else {
            return;
        };
        match ev {
            WsEventKind::Open(protocol) => {
                {
                    let mut d = data.borrow_mut();
                    d.ready = 1;
                    d.protocol = protocol;
                    d.buffered = 0;
                }
                let _ = self.fire_ws_event(wid, &data, "open", None);
            }
            WsEventKind::Text(s) => {
                let m = self.str_val(s);
                let _ = self.fire_ws_message(wid, &data, m);
            }
            WsEventKind::Binary(bytes) => {
                let bin_is_ab = data.borrow().binary_type == "arraybuffer";
                let v = if bin_is_ab {
                    crate::typed_array::new_array_buffer(self, bytes)
                } else {
                    // `Blob` approximé : `{size, type:''}`
                    // (borne — pas de `Blob` constructible à
                    // cette tranche ; `arrayBuffer()` absent).
                    let o = self.new_object();
                    self.set_own(o, "size", Value::Num(bytes.len() as f64), true);
                    let t = self.str_val("");
                    self.set_own(o, "type", t, true);
                    Value::Obj(o)
                };
                let _ = self.fire_ws_message(wid, &data, v);
            }
            WsEventKind::Error => {
                let _ = self.fire_ws_event(wid, &data, "error", None);
            }
            WsEventKind::Close {
                code,
                reason,
                clean,
            } => {
                {
                    data.borrow_mut().ready = 3;
                }
                let _ =
                    self.fire_ws_event(wid, &data, "close", Some((f64::from(code), reason, clean)));
                self.ws.live.remove(pos);
            }
        }
    }

    /// `message` : `MessageEvent` `{type, target, data}` —
    /// même fan-out `on*`/`addEventListener` que `fire_ws_event`.
    fn fire_ws_message(
        &mut self,
        wrapper: ObjId,
        data: &Rc<RefCell<WsData>>,
        payload: Value,
    ) -> JsResult<()> {
        let this = Value::Obj(wrapper);
        let ev = {
            let o = self.new_object();
            let t = self.str_val("message");
            self.set_own(o, "type", t, true);
            self.set_own(o, "target", this, true);
            self.set_own(o, "data", payload, true);
            Value::Obj(o)
        };
        if let Some(f) = self.get_prop(wrapper, "onmessage") {
            if self.is_callable(f) {
                self.call_value(f, this, &[ev])?;
            }
        }
        let fns: Vec<Value> = data
            .borrow()
            .listeners
            .iter()
            .filter(|(t, _, _)| t == "message")
            .map(|(_, f, _)| *f)
            .collect();
        for f in fns {
            if self.is_callable(f) {
                self.call_value(f, this, &[ev])?;
            }
        }
        Ok(())
    }

    /// Injecte le dial WebSocket de la plateforme (JS-C94) —
    /// **engine-global** : posé une fois, tous les sous-
    /// contextes ouvrent par la même couture (forme
    /// [`Engine::install_es_transport`]). Sans transport,
    /// `new WebSocket` file un refus différé (stub JS-C73).
    pub fn install_ws_transport(&mut self, transport: Rc<dyn WsTransport>) {
        self.ws_transport = Some(transport);
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

    // Méthodes visibles en lecture — `typeof ws.send` rendait
    // 'undefined' (invoke-only) alors que les feature-detects
    // lisent la surface avant d'appeler (vols 27-28).
    fn has_method(&self, name: &str) -> bool {
        matches!(
            name,
            "send" | "close" | "addEventListener" | "removeEventListener" | "dispatchEvent"
        )
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
                if d.ready != 1 {
                    // CLOSING/CLOSED : silencieux,
                    // `bufferedAmount` += taille (spec).
                    let len = match arg(args, 0) {
                        Value::Undefined => 0,
                        v => e.to_js_string(v).map_or(0, |s| s.len()) as u64,
                    };
                    drop(d);
                    self.data.borrow_mut().buffered += len;
                    return Ok(Some(Value::Undefined));
                }
                // OPEN : émission réelle via la couture —
                // binaire si `ArrayBuffer`/`Uint8Array`,
                // texte sinon (spec `send(data)`).
                let handle = d.handle;
                drop(d);
                if let (Some(tr), Some(h)) = (e.ws_transport.clone(), handle) {
                    let out = match crate::typed_array::bytes_of_value(e, arg(args, 0)) {
                        Some(b) => WsOut::Binary(b),
                        None => WsOut::Text(e.to_js_string(arg(args, 0)).unwrap_or_default()),
                    };
                    tr.send(h, out);
                }
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
                    // la connexion échoue quand même). Flux
                    // réel : on notifie aussi la couture
                    // (abandon du handshake).
                    if let (Some(h), Some(tr)) = (d.handle, &e.ws_transport) {
                        tr.close(h, 1001, "");
                    }
                    d.ready = 2;
                    d.close_requested = true;
                } else if d.ready == 1 {
                    // OPEN → close(code, reason) réel.
                    let code = if matches!(code, Value::Undefined) {
                        1000
                    } else {
                        e.to_number(code)? as u16
                    };
                    let reason = if matches!(reason, Value::Undefined) {
                        String::new()
                    } else {
                        e.to_js_string(reason)?
                    };
                    if let (Some(h), Some(tr)) = (d.handle, &e.ws_transport) {
                        tr.close(h, code, &reason);
                    }
                    d.ready = 2;
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
