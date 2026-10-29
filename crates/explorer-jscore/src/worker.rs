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

    /// Lance la charge **sans attendre** — retourne un
    /// receveur one-shot (spawn différé de `new Worker`,
    /// JS-C132 : le constructeur rend la main avant le
    /// fetch, la source arrive à la pompe). Défaut : `load`
    /// inline — correct mais synchrone (le ctor patiente).
    fn request(
        &mut self,
        spec: &str,
        referrer: &str,
    ) -> Receiver<std::result::Result<String, String>> {
        let (tx, rx) = std::sync::mpsc::channel();
        let _ = tx.send(self.load(spec, referrer));
        rx
    }
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
    /// `Blob`/`File` (JS-C196) — octets + type + props propres
    /// (`File` : `name`/`lastModified` — le proto décodé est
    /// `Blob`, déviation documentée ; avant, un `Blob` posté
    /// sortait en objet vide `{}` — mensonge silencieux).
    Blob(Vec<u8>, String, Vec<(String, WireValue)>),
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
    /// Événements remontés par le worker (canal créé au ctor —
    /// vide tant que le thread n'est pas spawné).
    pub rx: Receiver<WorkerEvent>,
    /// Spawn différé (JS-C132) : receveur de la source en
    /// vol + loader/canaux destinés au thread. `None` une
    /// fois le thread lancé (ou le fetch tombé).
    pub pending: Option<WorkerPending>,
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

/// Matériel du spawn différé — tout est créé au ctor
/// (les `postMessage` pré-démarrage s'enfilent dans `in_tx`,
/// spec : file de messages du port).
pub(crate) struct WorkerPending {
    /// Receveur one-shot de la source fetchée par la session.
    pub src_rx: Receiver<std::result::Result<String, String>>,
    /// Loader emporté par le thread (`importScripts`).
    pub loader: Box<dyn WorkerLoader>,
    /// Spécificateur du script (`page_url` du worker).
    pub spec: String,
    /// File main→worker — passée au thread au spawn.
    pub in_rx: Receiver<WireValue>,
    /// Émetteur worker→main — cloné dans le thread au spawn.
    pub out_tx: SyncSender<WorkerEvent>,
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
        this: Value,
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
            "addEventListener" | "removeEventListener" => {
                // Rails UJS / Sentry instrumentent le wrapper
                // (`w.addEventListener('message', f)`) — sans ça
                // `addEventListener is not a function` (OSM).
                if let Value::Obj(oid) = this {
                    let ty = e.to_js_string(arg(args, 0))?;
                    let f = arg(args, 1);
                    if name == "addEventListener" {
                        if e.is_callable(f) {
                            listener_push(e, oid, &ty, f);
                        }
                    } else {
                        listener_remove(e, oid, &ty, f);
                    }
                }
                Ok(Some(Value::Undefined))
            }
            "dispatchEvent" => Ok(Some(Value::Bool(true))),
            _ => Ok(None),
        }
    }

    fn has_method(&self, name: &str) -> bool {
        matches!(
            name,
            "postMessage"
                | "terminate"
                | "addEventListener"
                | "removeEventListener"
                | "dispatchEvent"
        )
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

// -- listeners `addEventListener` (wrapper + scope worker) -------------
//
// Les écoutes vivent en props ordinaires `__wl{i}` = `{t, f}` +
// compteur `__wln` sur l'objet cible (wrapper `Worker` ou global
// worker) — traçage GC gratuit, pas d'état parallèle.

/// Enregistre `f` comme écoute `type` sur `id`.
fn listener_push(e: &mut Engine, id: crate::value::ObjId, ty: &str, f: Value) {
    let n = match e.get_prop(id, "__wln") {
        Some(Value::Num(n)) => n as usize,
        _ => 0,
    };
    let entry = e.new_object();
    let t = e.str_val(ty);
    e.set_own(entry, "t", t, false);
    e.set_own(entry, "f", f, false);
    e.set_own(id, &format!("__wl{n}"), Value::Obj(entry), false);
    e.set_own(id, "__wln", Value::Num((n + 1) as f64), false);
}

/// Retire les écoutes `ty` égales à `f` (marque `f` à `undefined`,
/// `listeners_of` les ignore — les indices restent stables).
fn listener_remove(e: &mut Engine, id: crate::value::ObjId, ty: &str, f: Value) {
    let n = match e.get_prop(id, "__wln") {
        Some(Value::Num(n)) => n as usize,
        _ => 0,
    };
    for i in 0..n {
        let Some(Value::Obj(ent)) = e.get_prop(id, &format!("__wl{i}")) else {
            continue;
        };
        let same_ty = e
            .get_prop(ent, "t")
            .and_then(|v| e.to_js_string(v).ok())
            .is_some_and(|t| t == ty);
        let same_f = e.get_prop(ent, "f").is_some_and(|ef| e.strict_eq(ef, f));
        if same_ty && same_f {
            e.set_own(ent, "f", Value::Undefined, false);
        }
    }
}

/// Les fonctions d'écoute `type` enregistrées sur `id`.
fn listeners_of(e: &mut Engine, id: crate::value::ObjId, ty: &str) -> Vec<Value> {
    let n = match e.get_prop(id, "__wln") {
        Some(Value::Num(n)) => n as usize,
        _ => 0,
    };
    let mut out = Vec::new();
    for i in 0..n {
        let Some(Value::Obj(ent)) = e.get_prop(id, &format!("__wl{i}")) else {
            continue;
        };
        let same_ty = e
            .get_prop(ent, "t")
            .and_then(|v| e.to_js_string(v).ok())
            .is_some_and(|t| t == ty);
        if same_ty {
            if let Some(f) = e.get_prop(ent, "f") {
                if e.is_callable(f) {
                    out.push(f);
                }
            }
        }
    }
    out
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
        let v = self.native_fn("Worker", worker_ctor_pub);
        let g = self.global;
        // `Worker.prototype` — spec WebIDL : les instruments
        // (Dynatrace ruxitagentjs, axa/laposte vol 131) patchent
        // `Worker.prototype.addEventListener.call(w, …)` ; sans
        // proto, la lecture jetait `of Undefined` et tuait
        // l'agent entier. L'héritage `EventTarget.prototype`
        // est câblé par la boucle de chaînage de `install_dom`
        // (installée après — l'ordre d'installation ne suffit
        // pas à l'exprimer ici).
        let p = self.new_object();
        for name in [
            "postMessage",
            "terminate",
            "addEventListener",
            "removeEventListener",
            "dispatchEvent",
        ] {
            let namev = self.str_val(name);
            let f = self.bound_native("hostMethod", Self::host_method, vec![namev]);
            self.set_own(p, name, f, true);
        }
        if let Value::Obj(cid) = v {
            self.set_own(cid, "prototype", Value::Obj(p), false);
            self.set_own(p, "constructor", v, false);
        }
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
            // Spawn différé (JS-C132) : la source arrive sur le
            // receveur one-shot — `Ok` lance le thread, `Err`
            // (fetch tombé) = event `error` spec sur le wrapper.
            if self.workers[i].pending.is_some() {
                let res = match self.workers[i]
                    .pending
                    .as_ref()
                    .map(|p| p.src_rx.try_recv())
                {
                    Some(Ok(res)) => Some(res),
                    Some(Err(std::sync::mpsc::TryRecvError::Empty)) => None,
                    Some(Err(std::sync::mpsc::TryRecvError::Disconnected)) | None => {
                        self.workers[i].pending = None;
                        self.workers[i].terminated = true;
                        None
                    }
                };
                if let Some(p) = self.workers[i].pending.take() {
                    match res {
                        Some(Ok(src)) => {
                            let alive_w = Arc::clone(&self.workers[i].alive);
                            let op = self.op_limit;
                            let join = std::thread::spawn(move || {
                                let WorkerPending {
                                    spec,
                                    loader,
                                    in_rx,
                                    out_tx,
                                    ..
                                } = p;
                                worker_main(&spec, &src, loader, &out_tx, &in_rx, op, alive_w);
                            });
                            self.workers[i].join = Some(join);
                        }
                        Some(Err(m)) => {
                            self.workers[i].terminated = true;
                            let obj = self.workers[i].obj;
                            if let Value::Obj(oid) = obj {
                                let onerr =
                                    self.get_prop(oid, "onerror").unwrap_or(Value::Undefined);
                                if self.is_callable(onerr) {
                                    let s = self.str_val(m);
                                    let ev = self.message_event(obj, "error", s);
                                    self.call_value(onerr, obj, &[ev])?;
                                    delivered += 1;
                                }
                            }
                        }
                        None => {
                            // Rien livré → repose `pending`.
                            self.workers[i].pending = Some(p);
                        }
                    }
                }
            }
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
                        // `addEventListener('message', f)` du
                        // wrapper — spéc EventTarget.
                        for f in listeners_of(self, oid, "message") {
                            let ev = self.message_event(obj, "message", data);
                            self.call_value(f, obj, &[ev])?;
                            delivered += 1;
                        }
                    }
                    WorkerEvent::Fail(msg) => {
                        let Value::Obj(oid) = obj else { continue };
                        let onerr = self.get_prop(oid, "onerror").unwrap_or(Value::Undefined);
                        if self.is_callable(onerr) {
                            let m = self.str_val(msg.clone());
                            let ev = self.message_event(obj, "error", m);
                            self.call_value(onerr, obj, &[ev])?;
                            delivered += 1;
                        }
                        for f in listeners_of(self, oid, "error") {
                            let m = self.str_val(msg.clone());
                            let ev = self.message_event(obj, "error", m);
                            self.call_value(f, obj, &[ev])?;
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
                    /// `Blob`/`File` — octets `Rc` partagés +
                    /// props propres data (JS-C196).
                    Blb(std::rc::Rc<Vec<u8>>, String, Vec<(String, Value)>),
                    /// Host — passe par `Host::serializable_clone`
                    /// (JS-C196) ; `None` → `Bad(class_name)`.
                    Hst(std::rc::Rc<dyn crate::heap::Host>),
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
                    Some(ObjKind::Host(h)) => Shape::Hst(h.clone()),
                    Some(ObjKind::Blob { data, ty }) => {
                        // Props propres data (`File.name`…), même
                        // extraction que `Shape::Obj`.
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
                        Shape::Blb(data.clone(), ty.clone(), props)
                    }
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
                    // `serializable_clone` rend une valeur
                    // sérialisable (slots en objet ordinaire) —
                    // elle emprunte `self` → le host est passé
                    // par `Rc` hors du match d'extraction.
                    Shape::Hst(h) => match h.serializable_clone(self)? {
                        Some(v) => self.wire_enc(v, seen),
                        None => Err(self
                            .type_error(format!("postMessage: cannot clone {}", h.class_name()))),
                    },
                    Shape::Blb(data, ty, props) => {
                        let mut wprops = Vec::with_capacity(props.len());
                        for (k, v) in props {
                            wprops.push((k, self.wire_enc(v, seen)?));
                        }
                        Ok(WireValue::Blob(data.as_ref().clone(), ty, wprops))
                    }
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
            WireValue::Blob(data, ty, props) => {
                // Vrai `ObjKind::Blob` (JS-C196) — proto `Blob`,
                // props propres restaurées (`File.name`…).
                let b = crate::files::blob_value(self, data.clone(), ty.clone());
                if let Value::Obj(id) = b {
                    for (k, wv) in props {
                        let v = self.wire_dec(wv, memo);
                        self.set_own(id, k, v, true);
                    }
                }
                memo.push(b);
                b
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

/// `new Worker(spec)` — spec : le constructeur rend le
/// wrapper **avant** le fetch (JS-C132) ; la requête part
/// au chargeur (`request` non bloquant), la source arrive
/// à `pump_workers` qui spawn le thread. `onerror` est tiré
/// si le fetch tombe — jamais de blocage de l'éval en cours.
pub(crate) fn worker_ctor_pub(e: &mut Engine, _this: Value, args: &[Value]) -> JsResult<Value> {
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
    let src_rx = loader.request(&spec, &referrer);
    let (in_tx, in_rx) = sync_channel::<WireValue>(e.worker_cfg.queue_len);
    let (out_tx, out_rx) = sync_channel::<WorkerEvent>(e.worker_cfg.queue_len);
    let alive = Arc::new(AtomicBool::new(true));
    let idx = e.workers.len();
    let host = e.new_host_object(Rc::new(WorkerHost {
        tx: Mutex::new(Some(in_tx)),
        alive: Arc::clone(&alive),
        idx,
    }));
    // `w instanceof Worker` — les instances chaînent
    // `Worker.prototype` (spec ; `new_host_object` pose
    // `Object.prototype` par défaut).
    let g = e.global;
    if let (Some(Value::Obj(wc)), Value::Obj(hid)) = (e.get_prop(g, "Worker"), host) {
        if let Some(pp @ Value::Obj(_)) = e.get_prop(wc, "prototype") {
            if let Some(o) = e.heap.obj_mut(hid) {
                o.proto = pp;
            }
        }
    }
    e.workers.push(WorkerSlot {
        rx: out_rx,
        pending: Some(WorkerPending {
            src_rx,
            loader,
            spec,
            in_rx,
            out_tx,
        }),
        alive,
        join: None,
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
    // `self.addEventListener('message', f)` dans le scope worker —
    // même surface que `onmessage` (spec WorkerGlobalScope).
    let ael = e.native_fn("addEventListener", wscope_add_listener);
    e.set_own(g, "addEventListener", ael, false);
    let rel = e.native_fn("removeEventListener", wscope_remove_listener);
    e.set_own(g, "removeEventListener", rel, false);
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
                    for f in listeners_of(&mut e, g, "message") {
                        let ev = e.message_event(gv, "message", data);
                        if let Err(t) = e.call_value(f, gv, &[ev]) {
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

/// `self.addEventListener(type, f)` dans le scope worker — même
/// mécanique de props `__wl*` que le wrapper (cible = global).
fn wscope_add_listener(e: &mut Engine, _this: Value, args: &[Value]) -> JsResult<Value> {
    let ty = e.to_js_string(arg(args, 0))?;
    let f = arg(args, 1);
    if e.is_callable(f) {
        listener_push(e, e.global, &ty, f);
    }
    Ok(Value::Undefined)
}

/// `self.removeEventListener(type, f)` — marque l'écoute à
/// `undefined` (indices stables).
fn wscope_remove_listener(e: &mut Engine, _this: Value, args: &[Value]) -> JsResult<Value> {
    let ty = e.to_js_string(arg(args, 0))?;
    listener_remove(e, e.global, &ty, arg(args, 1));
    Ok(Value::Undefined)
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
