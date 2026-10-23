// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! `Worker` bornés (JS-C29) — un `Engine` par thread (patron
//! `JscoreEngine` : le moteur ne quitte jamais son thread),
//! messages `postMessage` clonés par valeur en forme fil
//! [`WireValue`] (aucun handle de heap ne traverse), `onmessage`
//! des deux côtés, `importScripts` minimal, `terminate`,
//! `close()` côté worker.
//!
//! Déviations documentées :
//! - pas de mémoire partagée réelle : un `TypedArray`/`ArrayBuffer`
//!   posté est **copié** (pas de `SharedArrayBuffer` inter-thread) ;
//! - `addEventListener('message')` n'est pas implémenté —
//!   `onmessage`/`onerror` seuls ;
//! - les timers tournent entre deux messages (un worker qui ne
//!   reçoit rien n'exécute pas ses timers sans borne — la pompe
//!   attend `next_timer_deadline`) ;
//! - un worker ne peut pas spawner de worker (pas de constructeur
//!   `Worker` dans son scope).

use crate::heap::{JsResult, ObjKind, Thrown};
use crate::value::{ObjId, Value};
use crate::Engine;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, RecvTimeoutError, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Instant;

/// Chargeur de sources worker — résolution (`file:`/URL) et fetch
/// appartiennent à l'embeddeur, même couture que [`crate::ModuleLoader`].
/// `Send` : l'instance vit dans le thread du worker
/// (`importScripts` y charge en synchrone).
pub trait WorkerLoader: Send {
    /// Source de `spec` vu depuis `referrer`.
    ///
    /// # Errors
    /// `Err` message propagé en erreur JS (`TypeError`).
    fn load(&mut self, spec: &str, referrer: &str) -> std::result::Result<String, String>;
}

/// Fabrique de loaders — chaque worker reçoit sa propre instance
/// (son thread l'emporte ; la fabrique reste côté moteur parent).
pub type WorkerFactory = Arc<dyn Fn() -> Box<dyn WorkerLoader> + Send + Sync>;

/// Configuration du pool de workers (bornes — contenu hostile).
#[derive(Debug, Clone, Copy)]
pub struct WorkerConfig {
    /// Workers simultanés vivants max (`new Worker` au-delà →
    /// `TypeError` — pas de croissance illimitée).
    pub max_workers: usize,
    /// Capacité de chaque file de messages (main→worker et
    /// worker→main) — pleine = back-pressure naturelle par blocage
    /// de l'émetteur.
    pub queue_len: usize,
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            max_workers: 8,
            queue_len: 64,
        }
    }
}

/// Forme fil d'une valeur clonée entre threads — tout est par
/// valeur, aucun `ObjId`/`Rc` ne sort d'un heap.
#[derive(Debug)]
pub(crate) enum WireValue {
    /// `undefined`.
    Undefined,
    /// `null`.
    Null,
    /// Booléen.
    Bool(bool),
    /// Nombre.
    Num(f64),
    /// Chaîne.
    Str(String),
    /// `[…]` (éléments dans l'ordre).
    Arr(Vec<WireValue>),
    /// Objet ordinaire — props data propres, ordre d'insertion.
    Obj(Vec<(String, WireValue)>),
    /// `ArrayBuffer` (un `TypedArray`/`DataView` posté arrive
    /// comme son buffer — approximation documentée).
    Buf(Vec<u8>),
    /// `RegExp` `(source, flags)`.
    Re(String, String),
    /// `Map` — paires `(clé, valeur)` ordonnées.
    Map(Vec<(WireValue, WireValue)>),
    /// `Set` — valeurs ordonnées.
    Set(Vec<WireValue>),
    /// `BigInt`.
    Big(num_bigint::BigInt),
    /// Référence arrière — index dans l'ordre de création des
    /// conteneurs (cycles préservés, spec `StructuredSerialize`).
    Ref(u32),
}

/// Un événement remonté du worker vers le moteur principal.
pub(crate) enum WorkerEvent {
    /// `postMessage(v)` depuis le worker — valeur fil.
    Msg(WireValue),
    /// Erreur JS non rattrapée dans le worker (corps du script ou
    /// handler `onmessage`) — le wrapper voit `onerror`.
    Fail(String),
}

/// Slot moteur principal ↔ worker : file sortante, événements
/// entrants, handle de thread détachable. `alive` est partagé avec
/// le `WorkerHost` — son `finalize` (wrapper GC'd) coupe le
/// flot entrant et le marque terminé ici.
pub(crate) struct WorkerSlot {
    /// Événements remontés par le worker.
    pub rx: Receiver<WorkerEvent>,
    /// Vivant tant que le fil sortant n'est pas coupé.
    pub alive: Arc<AtomicBool>,
    /// Thread du worker — gardé pour l'information ; détaché (le
    /// drop du `JoinHandle` ne tue pas le thread, il le libère).
    pub join: Option<JoinHandle<()>>,
    /// Wrapper `Worker` côté heap (pour `onmessage`/`onerror`).
    pub obj: Value,
    /// `terminate()` appelé ou worker mort.
    pub terminated: bool,
}

/// Objet hôte du wrapper `Worker` — porte le canal sortant et le
/// flag vivant : le GC de l'emballage (`finalize`) termine le
/// thread sans que l'engine n'ait à surveiller.
struct WorkerHost {
    /// File main→worker ; `None` après `terminate()`/GC.
    tx: Mutex<Option<SyncSender<WireValue>>>,
    /// Partagé avec le slot — `false` = mort/explicitement terminé.
    alive: Arc<AtomicBool>,
    /// Index du slot dans `Engine::workers` (lookup au pump).
    idx: usize,
}

impl crate::heap::Host for WorkerHost {
    fn class_name(&self) -> &'static str {
        "Worker"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn invoke(
        &self,
        e: &mut Engine,
        _this: Value,
        name: &str,
        args: &[Value],
    ) -> JsResult<Option<Value>> {
        match name {
            "postMessage" => {
                let wv = e.wire_encode(arg(args, 0))?;
                let tx = self.tx.lock().ok().and_then(|t| t.clone());
                if let Some(tx) = tx {
                    // File pleine → back-pressure (borne queue_len) ;
                    // mort → silencieux (spec : postMessage sur
                    // worker terminé = noop).
                    let _ = tx.send(wv);
                }
                Ok(Some(Value::Undefined))
            }
            "terminate" => {
                self.alive.store(false, Ordering::Relaxed);
                if let Ok(mut t) = self.tx.lock() {
                    *t = None; // coupe le flot → le worker sort
                }
                e.workers[self.idx].terminated = true;
                Ok(Some(Value::Undefined))
            }
            _ => Ok(None),
        }
    }

    fn finalize(&self) {
        // Wrapper GC'd → worker orphelin : coupe son flot entrant,
        // le thread sort de lui-même (à l'issue de l'eval courant).
        self.alive.store(false, Ordering::Relaxed);
        if let Ok(mut t) = self.tx.lock() {
            *t = None;
        }
    }
}

fn arg(args: &[Value], i: usize) -> Value {
    args.get(i).copied().unwrap_or(Value::Undefined)
}

impl Engine {
    /// Fabrique de loaders — `Some` = `new Worker(spec)` charge
    /// réellement (source demandée à l'embeddeur, synchrone comme
    /// les modules — approximation documentée).
    pub fn set_worker_factory(&mut self, factory: WorkerFactory) {
        self.worker_factory = Some(factory);
    }

    /// Bornes du pool (défaut [`WorkerConfig::default`]).
    pub fn set_worker_config(&mut self, cfg: WorkerConfig) {
        self.worker_cfg = cfg;
    }

    /// Installe le constructeur `Worker` global (appelé par
    /// [`Self::install_dom`]). Sans fabrique, `new Worker` jette
    /// un `TypeError` honnête — `typeof Worker` reste `"function"`
    /// pour la détection de feature.
    pub(crate) fn install_worker_ctor(&mut self) {
        let v = self.native_fn("Worker", worker_ctor);
        let g = self.global;
        self.set_own(g, "Worker", v, false);
    }

    /// Livre les événements remontés par les workers : `onmessage`
    /// pour les valeurs postées, `onerror` pour les erreurs non
    /// rattrapées. Appelée au point de pompe (`run_microtasks`) —
    /// jamais au milieu d'une évaluation.
    ///
    /// # Errors
    /// Propage la première exception d'un handler `onmessage`/
    /// `onerror` (même contrat que `dispatch_event`).
    pub fn pump_workers(&mut self) -> JsResult<usize> {
        let mut delivered = 0usize;
        for i in 0..self.workers.len() {
            loop {
                let ev = match self.workers[i].rx.try_recv() {
                    Ok(ev) => ev,
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        self.workers[i].terminated = true;
                        self.workers[i].join = None;
                        break;
                    }
                };
                let obj = self.workers[i].obj;
                match ev {
                    WorkerEvent::Msg(wv) => {
                        let data = self.wire_decode(&wv)?;
                        let Value::Obj(oid) = obj else { continue };
                        let onmsg = self.get_prop(oid, "onmessage").unwrap_or(Value::Undefined);
                        if self.is_callable(onmsg) {
                            let ev = self.message_event(obj, "message", data);
                            self.call_value(onmsg, obj, &[ev])?;
                            delivered += 1;
                        }
                    }
                    WorkerEvent::Fail(msg) => {
                        let Value::Obj(oid) = obj else { continue };
                        let onerr = self.get_prop(oid, "onerror").unwrap_or(Value::Undefined);
                        if self.is_callable(onerr) {
                            let m = self.str_val(msg);
                            let ev = self.message_event(obj, "error", m);
                            self.call_value(onerr, obj, &[ev])?;
                            delivered += 1;
                        }
                    }
                }
            }
            if !self.workers[i].alive.load(Ordering::Relaxed) {
                self.workers[i].terminated = true;
            }
        }
        Ok(delivered)
    }

    /// Objet événement minimal `{type, data, target}` pour
    /// `message`/`error` (`WorkerGlobalScope` spec-lite).
    fn message_event(&mut self, target: Value, ty: &str, data: Value) -> Value {
        let id = self.new_object();
        let t = self.str_val(ty);
        self.set_own(id, "type", t, true);
        self.set_own(id, "data", data, true);
        self.set_own(id, "target", target, true);
        Value::Obj(id)
    }

    // -- forme fil ---------------------------------------------------

    /// `StructuredSerialize` vers [`WireValue`] : cycles préservés
    /// (`Ref`), non-clonables (`Fun`/`Promise`/`Symbol`/`Proxy`/
    /// `Host`/`Iter`) → `TypeError` (`DataCloneError` — postMessage
    /// jette, spec).
    pub(crate) fn wire_encode(&mut self, v: Value) -> JsResult<WireValue> {
        let mut seen = Vec::new();
        self.wire_enc(v, &mut seen)
    }

    #[allow(clippy::too_many_lines)]
    fn wire_enc(&mut self, v: Value, seen: &mut Vec<ObjId>) -> JsResult<WireValue> {
        match v {
            Value::Undefined | Value::Hole => Ok(WireValue::Undefined),
            Value::Null => Ok(WireValue::Null),
            Value::Bool(b) => Ok(WireValue::Bool(b)),
            Value::Num(n) => Ok(WireValue::Num(n)),
            Value::Str(id) => Ok(WireValue::Str(self.heap.string(id).unwrap_or_default())),
            Value::Big(id) => match self.heap.obj(id).map(|o| &o.kind) {
                Some(ObjKind::BigInt(b)) => Ok(WireValue::Big(b.clone())),
                _ => Err(self.type_error("postMessage: cannot clone bigint")),
            },
            Value::Obj(id) => {
                enum Shape {
                    Arr(Vec<Value>),
                    Obj(Vec<(String, Value)>),
                    Buf(Vec<u8>),
                    Re(String, String),
                    Map(Vec<(Value, Value)>),
                    Set(Vec<Value>),
                    Bad(&'static str),
                }
                if let Some(i) = seen.iter().position(|s| *s == id) {
                    return Ok(WireValue::Ref(i as u32));
                }
                let shape = match self.heap.obj(id).map(|o| &o.kind) {
                    Some(ObjKind::Array(elems)) => Shape::Arr(elems.clone()),
                    Some(ObjKind::ByteBuf(b)) => Shape::Buf(b.clone()),
                    Some(ObjKind::RegExp(re)) => Shape::Re(re.source.clone(), re.flag_string()),
                    Some(ObjKind::Fun(_)) => Shape::Bad("function"),
                    Some(ObjKind::Promise(_)) => Shape::Bad("Promise"),
                    Some(ObjKind::Host(_)) => Shape::Bad("host object"),
                    Some(ObjKind::Symbol { .. }) => Shape::Bad("Symbol"),
                    Some(ObjKind::Proxy { .. }) => Shape::Bad("Proxy"),
                    Some(ObjKind::Iter { .. }) => Shape::Bad("iterator"),
                    Some(ObjKind::FetchBody { .. }) => Shape::Bad("Response"),
                    Some(ObjKind::Map(m)) => Shape::Map(m.clone()),
                    Some(ObjKind::Set(s)) => Shape::Set(s.clone()),
                    _ => {
                        let props: Vec<(String, Value)> = self
                            .heap
                            .obj(id)
                            .map(|o| {
                                o.props
                                    .iter()
                                    .filter_map(|(k, p)| p.data_value().map(|d| (k.clone(), d)))
                                    .collect()
                            })
                            .unwrap_or_default();
                        Shape::Obj(props)
                    }
                };
                match shape {
                    Shape::Bad(w) => Err(self.type_error(format!("postMessage: cannot clone {w}"))),
                    Shape::Buf(b) => Ok(WireValue::Buf(b)),
                    Shape::Re(s, f) => Ok(WireValue::Re(s, f)),
                    Shape::Arr(elems) => {
                        seen.push(id);
                        let mut out = Vec::with_capacity(elems.len());
                        for v in elems {
                            out.push(self.wire_enc(v, seen)?);
                        }
                        Ok(WireValue::Arr(out))
                    }
                    Shape::Obj(props) => {
                        seen.push(id);
                        let mut out = Vec::with_capacity(props.len());
                        for (k, v) in props {
                            out.push((k, self.wire_enc(v, seen)?));
                        }
                        Ok(WireValue::Obj(out))
                    }
                    Shape::Map(entries) => {
                        seen.push(id);
                        let mut out = Vec::with_capacity(entries.len());
                        for (k, v) in entries {
                            out.push((self.wire_enc(k, seen)?, self.wire_enc(v, seen)?));
                        }
                        Ok(WireValue::Map(out))
                    }
                    Shape::Set(items) => {
                        seen.push(id);
                        let mut out = Vec::with_capacity(items.len());
                        for v in items {
                            out.push(self.wire_enc(v, seen)?);
                        }
                        Ok(WireValue::Set(out))
                    }
                }
            }
        }
    }

    /// Reconstruit un `Value` depuis la forme fil — `Ref` pointe
    /// sur un conteneur déjà créé (`memo`, ordre de création).
    pub(crate) fn wire_decode(&mut self, w: &WireValue) -> JsResult<Value> {
        let mut memo: Vec<Value> = Vec::new();
        Ok(self.wire_dec(w, &mut memo))
    }

    fn wire_dec(&mut self, w: &WireValue, memo: &mut Vec<Value>) -> Value {
        match w {
            WireValue::Undefined => Value::Undefined,
            WireValue::Null => Value::Null,
            WireValue::Bool(b) => Value::Bool(*b),
            WireValue::Num(n) => Value::Num(*n),
            WireValue::Str(s) => self.str_val(s.clone()),
            WireValue::Buf(b) => {
                let proto = Value::Obj(self.protos.array_buffer);
                Value::Obj(self.heap.alloc_obj(crate::heap::Object {
                    proto,
                    props: Vec::new(),
                    kind: ObjKind::ByteBuf(b.clone()),
                    extensible: true,
                    elem_integrity: 0,
                    args_map: None,
                }))
            }
            WireValue::Re(src, flags) => {
                let src = src.clone();
                let flags = flags.clone();
                self.make_regexp(&src, &flags).unwrap_or(Value::Null)
            }
            WireValue::Ref(i) => memo.get(*i as usize).copied().unwrap_or(Value::Null),
            WireValue::Big(b) => self.big_val(b.clone()),
            WireValue::Map(entries) => {
                let proto = Value::Obj(self.protos.map);
                let m = Value::Obj(self.heap.alloc_obj(crate::heap::Object {
                    proto,
                    props: Vec::new(),
                    kind: ObjKind::Map(Vec::new()),
                    extensible: true,
                    elem_integrity: 0,
                    args_map: None,
                }));
                memo.push(m);
                let Value::Obj(id) = m else { return m };
                for (k, v) in entries {
                    let dk = self.wire_dec(k, memo);
                    let dv = self.wire_dec(v, memo);
                    if let Some(o) = self.heap.obj_mut(id) {
                        if let ObjKind::Map(m) = &mut o.kind {
                            m.push((dk, dv));
                        }
                    }
                }
                m
            }
            WireValue::Set(items) => {
                let proto = Value::Obj(self.protos.set);
                let s = Value::Obj(self.heap.alloc_obj(crate::heap::Object {
                    proto,
                    props: Vec::new(),
                    kind: ObjKind::Set(Vec::new()),
                    extensible: true,
                    elem_integrity: 0,
                    args_map: None,
                }));
                memo.push(s);
                let Value::Obj(id) = s else { return s };
                for it in items {
                    let v = self.wire_dec(it, memo);
                    if let Some(o) = self.heap.obj_mut(id) {
                        if let ObjKind::Set(elems) = &mut o.kind {
                            elems.push(v);
                        }
                    }
                }
                s
            }
            WireValue::Arr(items) => {
                let arr = self.new_array(Vec::new());
                memo.push(arr);
                let Value::Obj(id) = arr else { return arr };
                for it in items {
                    let v = self.wire_dec(it, memo);
                    if let Some(o) = self.heap.obj_mut(id) {
                        if let ObjKind::Array(elems) = &mut o.kind {
                            elems.push(v);
                        }
                    }
                }
                arr
            }
            WireValue::Obj(props) => {
                let id = self.new_object();
                let o = Value::Obj(id);
                memo.push(o);
                for (k, v) in props {
                    let dv = self.wire_dec(v, memo);
                    self.set_own(id, k, dv, true);
                }
                o
            }
        }
    }
}

/// `new Worker(spec)` — fetch synchrone de la source via la
/// fabrique (même approximation que le `ModuleLoader`), spawn
/// d'un `Engine` dédié sur son thread, pool borné.
fn worker_ctor(e: &mut Engine, _this: Value, args: &[Value]) -> JsResult<Value> {
    let spec = e.to_js_string(arg(args, 0))?;
    if spec.is_empty() {
        return Err(e.type_error("Worker: empty script URL"));
    }
    let Some(factory) = e.worker_factory.clone() else {
        return Err(e.type_error("Worker: no loader installed"));
    };
    if e.workers.iter().filter(|s| !s.terminated).count() >= e.worker_cfg.max_workers {
        return Err(e.type_error("Worker: pool exhausted"));
    }
    let mut loader = factory();
    let referrer = e.hooks.page_url.clone();
    let src = loader
        .load(&spec, &referrer)
        .map_err(|m| e.type_error(format!("Worker: cannot load `{spec}`: {m}")))?;
    let (in_tx, in_rx) = sync_channel::<WireValue>(e.worker_cfg.queue_len);
    let (out_tx, out_rx) = sync_channel::<WorkerEvent>(e.worker_cfg.queue_len);
    let alive = Arc::new(AtomicBool::new(true));
    let idx = e.workers.len();
    let op = e.op_limit;
    let spec_w = spec.clone();
    let alive_w = Arc::clone(&alive);
    let join = std::thread::spawn(move || {
        worker_main(&spec_w, &src, loader, &out_tx, &in_rx, op, alive_w);
    });
    let host = e.new_host_object(Rc::new(WorkerHost {
        tx: Mutex::new(Some(in_tx)),
        alive: Arc::clone(&alive),
        idx,
    }));
    e.workers.push(WorkerSlot {
        rx: out_rx,
        alive,
        join: Some(join),
        obj: host,
        terminated: false,
    });
    Ok(host)
}

/// Corps du thread worker : scope dédié (`self`/`postMessage`/
/// `importScripts`/`close`), eval de la source, puis pompe
/// `recv` ↔ timers jusqu'à déconnexion ou `close()`.
#[allow(clippy::too_many_lines)]
fn worker_main(
    spec: &str,
    src: &str,
    loader: Box<dyn WorkerLoader>,
    out: &SyncSender<WorkerEvent>,
    rx: &Receiver<WireValue>,
    op_limit: u64,
    alive: Arc<AtomicBool>,
) {
    let fail = |e: &mut Engine, t: Thrown| {
        let s = e.describe(t.0);
        let _ = out.send(WorkerEvent::Fail(s));
    };
    let mut e = Engine::new();
    e.set_op_limit(op_limit);
    e.install_timers();
    e.hooks.page_url = spec.to_string();
    e.worker_loader = Some(loader);
    e.worker_out = Some(out.clone());
    e.worker_alive = alive;
    // Scope WorkerGlobalScope minimal.
    let g = e.global;
    let gv = Value::Obj(g);
    e.set_own(g, "self", gv, false);
    e.set_own(g, "globalThis", gv, false);
    let post = e.native_fn("postMessage", wscope_post);
    e.set_own(g, "postMessage", post, false);
    let imp = e.native_fn("importScripts", wscope_import_scripts);
    e.set_own(g, "importScripts", imp, false);
    let close = e.native_fn("close", wscope_close);
    e.set_own(g, "close", close, false);
    e.set_own(g, "onmessage", Value::Null, true);
    e.set_own(g, "onerror", Value::Null, true);

    if let Err(err) = e.eval(src) {
        // Corps du worker mort (syntaxe/exception) — `onerror`
        // côté parent, puis le thread sort.
        let _ = out.send(WorkerEvent::Fail(err.to_string()));
        return;
    }
    if let Err(t) = e.run_microtasks() {
        fail(&mut e, t);
        return;
    }
    loop {
        if e.worker_close || !e.worker_alive.load(Ordering::Relaxed) {
            break;
        }
        // Attente bornée : un timer armé fixe le délai, sinon
        // blocage pur sur la file (le worker dort — borné).
        let wv = match e.next_timer_deadline() {
            Some(at) => {
                let wait = at.saturating_duration_since(Instant::now());
                match rx.recv_timeout(wait) {
                    Ok(w) => Some(w),
                    Err(RecvTimeoutError::Timeout) => None,
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            }
            None => match rx.recv() {
                Ok(w) => Some(w),
                Err(_) => break,
            },
        };
        if let Some(w) = wv {
            // Livraison `onmessage` côté worker — `{data,type}`.
            match e.wire_decode(&w) {
                Ok(data) => {
                    let onmsg = e.get_prop(g, "onmessage").unwrap_or(Value::Undefined);
                    if e.is_callable(onmsg) {
                        let ev = e.message_event(gv, "message", data);
                        if let Err(t) = e.call_value(onmsg, gv, &[ev]) {
                            fail(&mut e, t);
                        }
                    }
                }
                Err(t) => fail(&mut e, t),
            }
        }
        if let Err(t) = e.run_due_timers() {
            fail(&mut e, t);
        }
        if let Err(t) = e.run_microtasks() {
            fail(&mut e, t);
        }
    }
}

/// `postMessage(v)` dans le scope worker — remonte une valeur fil
/// au moteur principal (file bornée : pleine = blocage, morte =
/// silencieux).
fn wscope_post(e: &mut Engine, _this: Value, args: &[Value]) -> JsResult<Value> {
    let wv = e.wire_encode(arg(args, 0))?;
    if let Some(out) = &e.worker_out {
        let _ = out.send(WorkerEvent::Msg(wv));
    }
    Ok(Value::Undefined)
}

/// `importScripts(s1, s2, …)` — charge et évalue chaque source
/// dans l'ordre, scope global worker (spec : synchrone, le
/// premier échec stoppe la séquence).
fn wscope_import_scripts(e: &mut Engine, _this: Value, args: &[Value]) -> JsResult<Value> {
    // Le loader sort de `e` le temps des `eval` (borrow mut).
    let Some(mut loader) = e.worker_loader.take() else {
        return Err(e.type_error("importScripts: no loader"));
    };
    let mut srcs = Vec::new();
    for a in args {
        let spec = e.to_js_string(*a)?;
        let referrer = e.hooks.page_url.clone();
        match loader.load(&spec, &referrer) {
            Ok(s) => srcs.push(s),
            Err(m) => {
                e.worker_loader = Some(loader);
                return Err(e.type_error(format!("importScripts: cannot load `{spec}`: {m}")));
            }
        }
    }
    e.worker_loader = Some(loader);
    for src in srcs {
        if let Err(err) = e.eval(&src) {
            return Err(e.type_error(format!("importScripts: {err}")));
        }
    }
    Ok(Value::Undefined)
}

/// `close()` dans le worker — le fil sort après la pompe en cours
/// (spec : terminaison à la fin de la tâche).
fn wscope_close(e: &mut Engine, _this: Value, _args: &[Value]) -> JsResult<Value> {
    e.worker_close = true;
    Ok(Value::Undefined)
}
