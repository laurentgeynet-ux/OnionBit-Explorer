// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! `EventSource` (JS-C92) — flux Server-Sent Events réel.
//!
//! Le moteur n'ouvre pas de socket : le ctor émet une
//! [`EsRequest`] vers l'[`EsTransport`] injecté par
//! l'embedder (couture — même forme que
//! [`crate::xhr::XhrTransport`]/[`crate::crypto::CryptoTransport`]) ;
//! la session fetch `text/event-stream` en streamé et livre
//! `open`/`message`/`error` via [`Engine::deliver_es`].
//! Sans transport, le refus est **asynchrone** (`error` au
//! checkpoint, même motif que le stub `WebSocket` — le
//! feature-detect `typeof EventSource === 'function'` passe
//! et l'échec arme les fallbacks).
//!
//! Surface spec : `readyState` (0 CONNECTING, 1 OPEN,
//! 2 CLOSED), `url` absolue résolue, `withCredentials`,
//! `close()` (abort du transport), `onopen`/`onmessage`/
//! `onerror` + `addEventListener`/`removeEventListener`/
//! `dispatchEvent`, constantes sur le ctor et le prototype.
//! `MessageEvent` minimal : `type`/`target`/`data`/
//! `lastEventId`/`origin`.
//!
//! Bornes documentées : pas de reconnexion automatique
//! `Last-Event-ID` à cette tranche — une coupure livre
//! `error` et ferme (CLOSED), le script ré-ouvre s'il veut
//! (les frameworks SSE gèrent leur retry) ; champ `retry:`
//! ignoré ; CORS/cookies côté session (même pile que XHR).

use std::cell::RefCell;
use std::rc::Rc;

use crate::heap::{Host, JsResult, ObjKind};
use crate::value::{ObjId, Value};
use crate::Engine;

/// Requête d'ouverture d'un flux SSE — drainée par la
/// session (URL déjà résolue absolue par le ctor, politique
/// réseau côté session).
#[derive(Debug, Clone)]
pub struct EsRequest {
    /// URL absolue (résolution faite au ctor).
    pub url: String,
    /// `withCredentials` de l'init (spec CORS).
    pub with_credentials: bool,
    /// Contexte émetteur — les livraisons visent son realm.
    pub ctx: crate::context::CtxId,
}

/// Événement livré par la session au flux `handle`.
#[derive(Debug, Clone)]
pub enum EsEvent {
    /// Connexion établie : `readyState` → OPEN, event `open`.
    Open,
    /// Champ complet dispatché : `data` jointe `\n`,
    /// `event` nommé (`None` → `message`), `lastEventId`.
    Message {
        /// Charge `data:` jointe par `\n`.
        data: String,
        /// Champ `event:` — `None` → type `message`.
        event: Option<String>,
        /// Champ `id:` — vide = `lastEventId` inchangé.
        id: String,
    },
    /// Échec transport/format — event `error` puis CLOSED
    /// (borne : pas de reconnexion à cette tranche).
    Error,
}

/// Transport SSE injecté — requêtes ouvertes/aborts,
/// réponses via [`Engine::deliver_es`].
pub trait EsTransport {
    /// Ouvre un flux ; le handle retourné identifie les
    /// livraisons [`EsEvent`] et l'`abort`.
    fn open(&self, req: EsRequest) -> u64;
    /// Abandonne le flux `handle` — aucune livraison
    /// ultérieure n'est attendue (dégradation douce si la
    /// tâche est déjà finie).
    fn abort(&self, handle: u64);
}

/// État moteur `EventSource` — document-local comme `ws` :
/// garé/restauré au `switch_to`, les flux meurent avec
/// leur contexte.
#[derive(Default)]
pub(crate) struct EsState {
    /// `EventSource.prototype` paresseux.
    es_proto: Option<ObjId>,
    /// `(handle, wrapper)` des flux vivants — racines GC
    /// tant que le flux n'est pas CLOSED (spec : une
    /// connexion ouverte garde l'objet vivant).
    live: Vec<(u64, ObjId)>,
    /// Wrappers sans transport dont le refus différé reste
    /// à livrer (même motif que `ws.pending`).
    pending_refusal: Vec<ObjId>,
}

impl EsState {
    /// Racines GC : proto + flux vivants + refus en file.
    pub(crate) fn root_ids(&self, ids: &mut Vec<ObjId>) {
        if let Some(p) = self.es_proto {
            ids.push(p);
        }
        ids.extend(self.live.iter().map(|(_, w)| *w));
        ids.extend(self.pending_refusal.iter().copied());
    }
}

/// Données partagées d'un `EventSource` (slots spec).
struct EsData {
    /// 0 CONNECTING, 1 OPEN, 2 CLOSED.
    ready: u8,
    /// URL absolue résolue (`http:`/`https:` au ctor).
    url: String,
    /// `withCredentials` de l'init.
    creds: bool,
    /// Handle alloué par le transport — `None` = sans
    /// transport (refus différé en file).
    handle: Option<u64>,
    /// `Last-Event-ID` courant (spec — propagé sur
    /// `message`, sert à la reconnexion quand elle viendra).
    last_id: String,
    /// `addEventListener` handlers `(type, fn, capture)`.
    listeners: Vec<(String, Value, bool)>,
}

/// Host `EventSource`.
struct EsHost {
    data: Rc<RefCell<EsData>>,
}

/// `EsData` d'un `Value` `EventSource` — `None` sinon.
fn es_data(e: &Engine, v: Value) -> Option<Rc<RefCell<EsData>>> {
    let Value::Obj(id) = v else { return None };
    if let Some(ObjKind::Host(h)) = e.heap.obj(id).map(|o| &o.kind) {
        h.clone()
            .as_any()
            .downcast_ref::<EsHost>()
            .map(|w| w.data.clone())
    } else {
        None
    }
}

/// `EventSource.prototype` paresseux — constantes +
/// toStringTag (méthodes via `invoke` du host).
/// Chaîne `p` → `EventTarget.prototype` (spec `WebIDL`) si le
/// global existe — l'ordre d'installation varie
/// (`install_dom`/xhr) ; sans `EventTarget`, `Object.prototype`
/// reste. Idempotent : retenté à chaque appel — un proto créé
/// tôt (chemin xhr) se chaîne dès que `EventTarget` arrive.
fn chain_et(e: &mut Engine, p: ObjId) {
    let g = e.global;
    if let Some(Value::Obj(etc)) = e.get_prop(g, "EventTarget") {
        if let Some(pp @ Value::Obj(_)) = e.get_prop(etc, "prototype") {
            if let Some(o) = e.heap.obj_mut(p) {
                o.proto = pp;
            }
        }
    }
}

fn es_proto(e: &mut Engine) -> ObjId {
    if let Some(p) = e.es.es_proto {
        chain_et(e, p);
        return p;
    }
    let p = e.new_object();
    for (k, n) in [("CONNECTING", 0.0), ("OPEN", 1.0), ("CLOSED", 2.0)] {
        e.set_own(p, k, Value::Num(n), true);
    }
    // Méthodes IDL — les instruments (ruxit/Dynatrace,
    // mondialrelay vol 140) patchent `EventSource.prototype.
    // addEventListener` ; sans proto exposé la lecture jetait
    // `of Undefined` et tuait l'agent entier.
    for name in [
        "close",
        "addEventListener",
        "removeEventListener",
        "dispatchEvent",
    ] {
        let namev = e.str_val(name);
        let f = e.bound_native("hostMethod", Engine::host_method, vec![namev]);
        e.set_own(p, name, f, true);
    }
    chain_et(e, p);
    let tag = e.str_val("EventSource");
    e.set_own(p, crate::interp::SYM_TO_STRING_TAG, tag, false);
    e.es.es_proto = Some(p);
    p
}

/// `new EventSource(url [, {withCredentials}])` — résolution
/// contre `page_url`, schéma `http`/`https` exigé
/// (`SyntaxError` sinon — spec : hérite de l'URL de page).
fn eventsource_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let raw = arg(args, 0);
    // WebIDL `required` = argument ABSENT ; `EventSource(undefined)`
    // stringify `"undefined"` → résout `<base>/undefined` (spec).
    if args.is_empty() {
        return Err(e.type_error("EventSource: url required"));
    }
    let raw = e.to_js_string(raw)?;
    let Some(url) = crate::xhr::resolve_url(e, &raw) else {
        return Err(e.syntax_error("EventSource: invalid url"));
    };
    let scheme_ok = crate::builtins::url::url_parse_absolute(&url)
        .is_some_and(|p| matches!(crate::builtins::url::url_scheme(&p), "http" | "https"));
    if !scheme_ok {
        return Err(e.syntax_error("EventSource: url scheme must be http or https"));
    }
    let creds = if let Value::Obj(id) = arg(args, 1) {
        e.get_prop(id, "withCredentials")
            .is_some_and(|v| e.to_boolean(v))
    } else {
        false
    };
    let data = Rc::new(RefCell::new(EsData {
        ready: 0,
        url: url.clone(),
        creds,
        handle: None,
        last_id: String::new(),
        listeners: Vec::new(),
    }));
    let v = e.new_host_object(Rc::new(EsHost { data }));
    let p = es_proto(e);
    let Value::Obj(id) = v else { return Ok(v) };
    if let Some(o) = e.heap.obj_mut(id) {
        o.proto = Value::Obj(p);
    }
    if let Some(t) = e.es_transport.clone() {
        let handle = t.open(EsRequest {
            url,
            with_credentials: creds,
            ctx: e.cur_ctx,
        });
        if let Some(d) = es_data(e, v) {
            d.borrow_mut().handle = Some(handle);
        }
        e.es.live.push((handle, id));
    } else {
        // Pas de transport : refus différé au checkpoint —
        // séquence observable d'une connexion refusée.
        e.es.pending_refusal.push(id);
    }
    Ok(v)
}

impl Engine {
    /// Installe le transport SSE — engine-global (même
    /// rangement que `crypto_transport`), appelé par
    /// l'adaptateur session ; le ctor est posé par
    /// `install_eventsource` à `install_dom`.
    pub fn install_es_transport(&mut self, transport: Rc<dyn EsTransport>) {
        self.es_transport = Some(transport);
    }

    /// Livraison d'un [`EsEvent`] au flux `handle` du
    /// contexte `ctx` — l'appelant switche puis restaure
    /// (même forme que `deliver_dyn_*`/`fire_event_listeners`).
    pub fn deliver_es(&mut self, ctx: crate::context::CtxId, handle: u64, ev: &EsEvent) {
        let cur = self.cur_ctx;
        if cur != ctx {
            self.switch_to(ctx);
        }
        if self.cur_ctx == ctx {
            self.deliver_es_here(handle, ev);
        }
        if cur != ctx {
            self.switch_to(cur);
        }
    }

    /// Refus différé (absence de transport) + livraisons en
    /// file — drainé au checkpoint microtask avec `deliver_ws`.
    pub(crate) fn drain_es(&mut self) -> JsResult<usize> {
        let pending = std::mem::take(&mut self.es.pending_refusal);
        let mut n = 0usize;
        for wid in pending {
            let Some(data) = es_data(self, Value::Obj(wid)) else {
                continue;
            };
            n += 1;
            data.borrow_mut().ready = 2;
            self.fire_es_event(wid, &data, "error", None)?;
        }
        Ok(n)
    }

    /// Livraison dans le contexte courant : met à jour les
    /// slots puis tire `open`/`message`/`error`. Un `error`
    /// ferme le flux (borne : pas de reconnexion — le
    /// `readyState` part en CLOSED) et le sort du registre.
    fn deliver_es_here(&mut self, handle: u64, ev: &EsEvent) {
        let Some(pos) = self.es.live.iter().position(|(h, _)| *h == handle) else {
            return;
        };
        let (_, wid) = self.es.live[pos];
        let Some(data) = es_data(self, Value::Obj(wid)) else {
            self.es.live.remove(pos);
            return;
        };
        let (ty, msg): (String, Option<(String, String)>) = match &ev {
            EsEvent::Open => {
                data.borrow_mut().ready = 1;
                ("open".into(), None)
            }
            EsEvent::Error => {
                data.borrow_mut().ready = 2;
                self.es.live.remove(pos);
                ("error".into(), None)
            }
            EsEvent::Message {
                data: body,
                event,
                id,
            } => {
                {
                    let mut d = data.borrow_mut();
                    if !id.is_empty() {
                        d.last_id.clone_from(id);
                    }
                }
                (
                    event.clone().unwrap_or_else(|| "message".into()),
                    Some((body.clone(), id.clone())),
                )
            }
        };
        let _ = self.fire_es_event(wid, &data, &ty, msg);
    }

    /// Event `EventSource` minimal : `Event` `{type,target}`
    /// pour `open`/`error` ; `MessageEvent` `{type,target,
    /// data,lastEventId,origin}` pour les messages — `on<ty>`
    /// own-prop puis `addEventListener` (fan-out `FileReader`).
    fn fire_es_event(
        &mut self,
        wrapper: ObjId,
        data: &Rc<RefCell<EsData>>,
        ty: &str,
        msg: Option<(String, String)>,
    ) -> JsResult<()> {
        let this = Value::Obj(wrapper);
        let ev = {
            let o = self.new_object();
            let t = self.str_val(ty);
            self.set_own(o, "type", t, true);
            self.set_own(o, "target", this, true);
            if let Some((body, id)) = msg {
                let d = self.str_val(body);
                self.set_own(o, "data", d, true);
                let last = if id.is_empty() {
                    data.borrow().last_id.clone()
                } else {
                    id
                };
                let l = self.str_val(last);
                self.set_own(o, "lastEventId", l, true);
                let orig = self.str_val(self.ctx_origin(self.cur_ctx));
                self.set_own(o, "origin", orig, true);
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

impl Host for EsHost {
    fn class_name(&self) -> &'static str {
        "EventSource"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn get(&self, e: &mut Engine, _t: Value, key: &str) -> JsResult<Option<Value>> {
        let d = self.data.borrow();
        Ok(Some(match key {
            "readyState" => Value::Num(f64::from(d.ready)),
            "url" => return Ok(Some(e.str_val(d.url.clone()))),
            "withCredentials" => Value::Bool(d.creds),
            "CONNECTING" => Value::Num(0.0),
            "OPEN" => Value::Num(1.0),
            "CLOSED" => Value::Num(2.0),
            _ => return Ok(None),
        }))
    }

    fn set(&self, e: &mut Engine, this: Value, key: &str, val: Value) -> JsResult<bool> {
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
            "close" => {
                let was_open = {
                    let mut d = self.data.borrow_mut();
                    let was = d.ready != 2;
                    d.ready = 2;
                    was
                };
                if was_open {
                    if let (Some(t), Some(h)) = (e.es_transport.clone(), self.data.borrow().handle)
                    {
                        t.abort(h);
                    }
                }
                // Sort du registre — plus de livraisons ni de
                // racine GC (un flux CLOSED ne garde plus).
                if let Value::Obj(id) = this {
                    let h = self.data.borrow().handle;
                    if let Some(h) = h {
                        e.es.live.retain(|(lh, w)| !(*lh == h && *w == id));
                    }
                }
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
                    e.fire_es_event(wid, &self.data, &ty, None)?;
                }
                Ok(Some(Value::Bool(true)))
            }
            _ => Ok(None),
        }
    }

    fn trace(&self, out: &mut Vec<ObjId>) {
        for (_, f, _) in &self.data.borrow().listeners {
            if let Value::Obj(id) = f {
                out.push(*id);
            }
        }
    }
}

/// `install_eventsource` — global `EventSource` + constantes
/// sur le ctor (méthodes via `invoke` du host). Posé par
/// `install_dom` ; le transport arrive séparément via
/// [`Engine::install_es_transport`] (l'adaptateur l'injecte
/// après) — sans transport le ctor reste fonctionnel mais
/// refuse différé.
pub(crate) fn install_eventsource(e: &mut Engine) {
    let g = e.global;
    let ctor = e.native_fn("EventSource", eventsource_ctor);
    if let Value::Obj(cid) = ctor {
        for (k, n) in [("CONNECTING", 0.0), ("OPEN", 1.0), ("CLOSED", 2.0)] {
            e.set_own(cid, k, Value::Num(n), true);
        }
        // `EventSource.prototype` exposé (spec WebIDL) — posé
        // d'emblée : les instruments le patchent sans jamais
        // instancier (`X.prototype.addEventListener` lu au boot).
        let p = es_proto(e);
        e.set_own(cid, "prototype", Value::Obj(p), false);
        e.set_own(p, "constructor", ctor, false);
    }
    e.set_own(g, "EventSource", ctor, true);
}

fn arg(args: &[Value], i: usize) -> Value {
    args.get(i).copied().unwrap_or(Value::Undefined)
}
