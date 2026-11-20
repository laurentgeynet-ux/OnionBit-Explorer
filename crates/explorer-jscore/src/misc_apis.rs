// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! APIs « fenêtre » ponctuelles (JS-C74).
//!
//! - **`BroadcastChannel`** — **réel**, intra-contexte :
//!   `postMessage` structured-clone la valeur et la livre en
//!   `message` à toutes les *autres* instances ouvertes du
//!   canal (spec : un canal ne reçoit pas ses propres
//!   messages) au checkpoint microtask. `close()` désinscrit
//!   l'instance ; `postMessage` sur canal fermé →
//!   `InvalidStateError`. Inter-documents (multi-onglets)
//!   hors portée — un seul contexte d'exécution existe.
//! - **`Notification`** — stub honnête : `permission` =
//!   `'default'` (aucune UI de grant n'existe),
//!   `requestPermission` résout `'denied'`, le ctor produit
//!   un objet inerte (spec : non-`granted` → jamais affichée,
//!   jamais d'event `show`) avec la surface de props.
//! - **`getSelection`** — `Selection` réel minimal :
//!   `selectAllChildren`/`collapse`/`addRange` modifient une
//!   sélection partagée (spec : singleton par document) ;
//!   `toString` = `textContent` des ranges ; `getRangeAt`
//!   rend un Range-objet `{startContainer,…}` + `cloneRange`.
//! - **`reportError`** — réel : `ErrorEvent` `error` dispatché
//!   sur `window` (listeners `addEventListener('error')`),
//!   `window.onerror` appelé avec la signature legacy
//!   `(message, source, lineno, colno, error)` ; non-annulé →
//!   log `console` embedder (`Uncaught: …`), comme un vrai
//!   uncaught.

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;

use crate::dom::NodeId;
use crate::dom_host::{node_for, node_for_in, node_value};
use crate::heap::{Host, JsResult};
use crate::value::{ObjId, Value};
use crate::Engine;

/// État moteur des APIs JS-C74 — protos paresseux, registre
/// `BroadcastChannel`, messages en file, sélection courante.
#[derive(Default)]
pub(crate) struct MiscState {
    /// `BroadcastChannel.prototype` paresseux.
    bc_proto: Option<ObjId>,
    /// `Notification.prototype` paresseux.
    notif_proto: Option<ObjId>,
    /// `Selection.prototype` paresseux.
    sel_proto: Option<ObjId>,
    /// Canaux nommés → instances ouvertes (`close` retire).
    channels: HashMap<String, Vec<Rc<RefCell<BcData>>>>,
    /// Messages en file `(destinataire, payload cloné)` —
    /// drainés au checkpoint microtask.
    bc_pending: VecDeque<(Rc<RefCell<BcData>>, Value)>,
    /// Sélection courante — partagée entre tous les objets
    /// `Selection` rendus (spec : singleton par document).
    sel: Option<Rc<RefCell<SelData>>>,
    /// Wrapper `Selection` déjà rendu — spec : `getSelection`
    /// retourne **le même objet** à chaque appel.
    sel_wrapper: Option<ObjId>,
    /// `Range.prototype` / `TreeWalker.prototype` /
    /// `NodeIterator.prototype` paresseux (CAP-182/183).
    range_proto: Option<ObjId>,
    tw_proto: Option<ObjId>,
    ni_proto: Option<ObjId>,
    /// `AbstractRange.prototype` / `StaticRange.prototype`
    /// paresseux (JS-C556) — ancêtres proto de `Range`/
    /// `StaticRange` (chaîne `instanceof` spec).
    abstract_range_proto: Option<ObjId>,
    static_range_proto: Option<ObjId>,
    /// `Animation.prototype` paresseux + registre des
    /// animations vivantes (CAP-187) — document-local.
    /// `pub(crate)` : le proto du **ctor** est adopté à
    /// l'install (JS-C461 — identité `instanceof`).
    pub(crate) anim_proto: Option<ObjId>,
    /// `CSSAnimation.prototype`/`CSSTransition.prototype`
    /// (JS-C600, §B-165) — classes filles d'`Animation`
    /// (Web Animations spec) : adoptés à l'install comme
    /// `anim_proto`, héritent `Animation.prototype`.
    pub(crate) css_anim_proto: Option<ObjId>,
    pub(crate) css_trans_proto: Option<ObjId>,
    /// `KeyframeEffect.prototype` paresseux + singleton
    /// `document.timeline` (JS-C461 — WAAPI constructible).
    pub(crate) kfe_proto: Option<ObjId>,
    /// `DOMStringMap.prototype` adopté à l'install
    /// (§B-209) — les objets `el.dataset` (host live
    /// `data-*`) le chaînent : `dataset instanceof
    /// DOMStringMap` réel (whatsapp `_btldrLoadedSrcs`).
    pub(crate) dsm_proto: Option<ObjId>,
    doc_timeline: Option<ObjId>,
    anims: Vec<Rc<RefCell<AnimData>>>,
    /// `ViewTransition.prototype` paresseux (CAP-187).
    vt_proto: Option<ObjId>,
    /// `XPathResult.prototype` paresseux (CAP-188).
    xr_proto: Option<ObjId>,
    /// `__tcfapi` `addEventListener` — compteur de
    /// `listenerId` (JS-C259 : chaque listener reçoit un id
    /// croissant, spec IAB).
    pub(crate) cmp_seq: u32,
    /// Dernière valeur assignée à `window.OptanonWrapper`
    /// (JS-C260 : la prop est un accesseur — le setter lance
    /// le callback ET mémorise la valeur pour le getter).
    pub(crate) cmp_optanon: Option<Value>,
}

impl MiscState {
    /// Racines GC : protos + instances ouvertes (le
    /// `Host::trace` couvre listeners/wrappers) + payloads
    /// en file + ranges de la sélection (wrappers de nœuds).
    pub(crate) fn root_ids(&self, ids: &mut Vec<ObjId>) {
        for p in [
            self.bc_proto,
            self.notif_proto,
            self.sel_proto,
            self.range_proto,
            self.tw_proto,
            self.ni_proto,
            self.abstract_range_proto,
            self.static_range_proto,
            self.anim_proto,
            self.css_anim_proto,
            self.css_trans_proto,
            self.kfe_proto,
            self.dsm_proto,
            self.doc_timeline,
            self.vt_proto,
            self.xr_proto,
        ]
        .into_iter()
        .flatten()
        {
            ids.push(p);
        }
        // Instances ouvertes = racines — un canal reste
        // « entangled » (récepteur) jusqu'à `close`, même
        // sans référence JS restante (spec).
        for chans in self.channels.values() {
            for d in chans {
                if let Some(w) = d.borrow().wrapper {
                    ids.push(w);
                }
            }
        }
        for (_, d) in &self.bc_pending {
            crate::heap::push_value(ids, d);
        }
        if let Some(w) = self.sel_wrapper {
            ids.push(w);
        }
        if let Some(ref v) = self.cmp_optanon {
            crate::heap::push_value(ids, v);
        }
    }
}

// ------------------------------------------------------------------
// BroadcastChannel — réel intra-contexte
// ------------------------------------------------------------------

/// Données partagées d'un `BroadcastChannel`.
struct BcData {
    /// Nom du canal (`readonly`).
    name: String,
    /// `close()` appelé — `postMessage` → `InvalidStateError`.
    closed: bool,
    /// `addEventListener` handlers `(type, fn, capture)`.
    listeners: Vec<(String, Value, bool)>,
    /// Wrapper heap (pour `onmessage`/`target` — renseigné à
    /// la construction).
    wrapper: Option<ObjId>,
}

/// Host `BroadcastChannel`.
struct BcHost {
    data: Rc<RefCell<BcData>>,
}

/// `BroadcastChannel.prototype` paresseux — toStringTag.
fn bc_proto(e: &mut Engine) -> ObjId {
    if let Some(p) = e.misc.bc_proto {
        return p;
    }
    let p = e.new_object();
    let tag = e.str_val("BroadcastChannel");
    e.set_own_attrs(
        p,
        crate::interp::SYM_TO_STRING_TAG,
        tag,
        crate::heap::PropAttrs {
            writable: false,
            enumerable: false,
            configurable: true,
        },
    );
    e.misc.bc_proto = Some(p);
    p
}

/// Erreur shape `DOMException` `{name, message}` (même
/// approximation qu'IDB/`WebSocket`).
fn misc_err(e: &mut Engine, name: &str, msg: &str) -> Value {
    let o = e.new_object();
    let n = e.str_val(name);
    e.set_own(o, "name", n, true);
    let m = e.str_val(msg);
    e.set_own(o, "message", m, true);
    Value::Obj(o)
}

/// `new BroadcastChannel(name)` — inscrit l'instance dans le
/// registre du canal.
fn bc_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let name = e.to_js_string(arg(args, 0))?;
    let data = Rc::new(RefCell::new(BcData {
        name: name.clone(),
        closed: false,
        listeners: Vec::new(),
        wrapper: None,
    }));
    let v = e.new_host_object(Rc::new(BcHost { data: data.clone() }));
    let p = bc_proto(e);
    if let Value::Obj(id) = v {
        if let Some(o) = e.heap.obj_mut(id) {
            o.set_proto(Value::Obj(p));
        }
        data.borrow_mut().wrapper = Some(id);
    }
    e.misc.channels.entry(name).or_default().push(data);
    Ok(v)
}

/// `message` event d'un `BroadcastChannel` — `{type, target,
/// data, origin, lastEventId, source, ports}` (spec
/// `MessageEvent` — `origin`/`source` vides intra-contexte).
fn fire_bc_message(e: &mut Engine, data: &Rc<RefCell<BcData>>, payload: Value) -> JsResult<()> {
    let Some(wid) = data.borrow().wrapper else {
        return Ok(());
    };
    let this = Value::Obj(wid);
    let ev = {
        let o = e.new_object();
        crate::dom_host::event_proto(e, o, "MessageEvent");
        let t = e.str_val("message");
        e.set_own(o, "type", t, true);
        e.set_own(o, "target", this.clone(), true);
        e.set_own(o, "data", payload, true);
        let org = e.str_val("");
        e.set_own(o, "origin", org, true);
        let lid = e.str_val("");
        e.set_own(o, "lastEventId", lid, true);
        e.set_own(o, "source", Value::Null, true);
        let ports = e.new_array(Vec::new());
        e.set_own(o, "ports", ports, true);
        Value::Obj(o)
    };
    if let Some(f) = e.get_prop(wid, "onmessage") {
        if e.is_callable(f.clone()) {
            e.call_value(f, this.clone(), std::slice::from_ref(&ev))?;
        }
    }
    let fns: Vec<Value> = data
        .borrow()
        .listeners
        .iter()
        .filter(|(t, _, _)| t == "message")
        .map(|(_, f, _)| f.clone())
        .collect();
    for f in fns {
        if e.is_callable(f.clone()) {
            e.call_value(f, this.clone(), std::slice::from_ref(&ev))?;
        }
    }
    Ok(())
}

impl Engine {
    /// Livraison des messages `BroadcastChannel` en file
    /// (JS-C74) — ordre d'émission, un `message` event par
    /// destinataire ; instance fermée entre-temps → sautée
    /// (spec : `close` stoppe la réception).
    ///
    /// # Errors
    /// Propage la première exception d'un handler (modèle
    /// listeners DOM).
    pub(crate) fn deliver_bc(&mut self) -> JsResult<usize> {
        let n = self.misc.bc_pending.len();
        while let Some((data, payload)) = self.misc.bc_pending.pop_front() {
            if data.borrow().closed {
                continue;
            }
            fire_bc_message(self, &data, payload)?;
        }
        Ok(n)
    }
}

impl Host for BcHost {
    fn class_name(&self) -> &'static str {
        "BroadcastChannel"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    // Méthodes visibles en lecture (feature-detects) —
    // invoke-only les rendait `undefined` (vols 27-28).
    fn has_method(&self, name: &str) -> bool {
        matches!(
            name,
            "postMessage" | "close" | "addEventListener" | "removeEventListener"
        )
    }

    fn get(&self, e: &mut Engine, _t: Value, key: &str) -> JsResult<Option<Value>> {
        if key != "name" {
            return Ok(None);
        }
        let n = self.data.borrow().name.clone();
        Ok(Some(e.str_val(n)))
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
                if self.data.borrow().closed {
                    return Err(crate::heap::Thrown(misc_err(
                        e,
                        "InvalidStateError",
                        "BroadcastChannel: closed",
                    )));
                }
                let my_name = self.data.borrow().name.clone();
                let datas: Vec<Rc<RefCell<BcData>>> = e
                    .misc
                    .channels
                    .get(&my_name)
                    .map(|v| {
                        v.iter()
                            .filter(|d| !d.borrow().closed && !Rc::ptr_eq(d, &self.data))
                            .cloned()
                            .collect()
                    })
                    .unwrap_or_default();
                // Spec : chaque destinataire reçoit **sa propre
                // copie** structured-clonée — un clone par
                // destinataire, jamais l'objet partagé.
                for d in datas {
                    let payload =
                        crate::builtins::clone::clone_deep(e, arg(args, 0), &mut Vec::new(), 0)?;
                    e.misc.bc_pending.push_back((d, payload));
                }
                Ok(Some(Value::Undefined))
            }
            "close" => {
                {
                    self.data.borrow_mut().closed = true;
                }
                let my_name = self.data.borrow().name.clone();
                if let Some(v) = e.misc.channels.get_mut(&my_name) {
                    v.retain(|d| !Rc::ptr_eq(d, &self.data));
                }
                Ok(Some(Value::Undefined))
            }
            "addEventListener" => {
                let ty = e.to_js_string(arg(args, 0))?;
                let f = arg(args, 1);
                let capture = e.to_boolean(arg(args, 2));
                if e.is_callable(f.clone()) {
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
                    .retain(|(t, g, _)| !(*t == ty && e.strict_eq(g, &f)));
                Ok(Some(Value::Undefined))
            }
            "dispatchEvent" => {
                let ev = arg(args, 0);
                let ty = if let Value::Obj(id) = ev {
                    e.get_prop(id, "type")
                        .as_ref()
                        .and_then(Engine::heap_string_of)
                        .unwrap_or_default()
                } else {
                    String::new()
                };
                if ty == "message" {
                    let payload = if let Value::Obj(id) = ev {
                        e.get_prop(id, "data").unwrap_or(Value::Undefined)
                    } else {
                        Value::Undefined
                    };
                    fire_bc_message(e, &self.data, payload)?;
                }
                let _ = this;
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
        if let Some(w) = d.wrapper {
            out.push(w);
        }
    }
}

// ------------------------------------------------------------------
// Notification — stub honnête
// ------------------------------------------------------------------

/// Host `Notification` — objet inerte (jamais d'events : la
/// permission n'est jamais `granted` sans UI de grant) ;
/// toutes les props sont own-props posées au ctor.
struct NotificationHost;

impl Host for NotificationHost {
    fn class_name(&self) -> &'static str {
        "Notification"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// `Notification.prototype` paresseux.
fn notif_proto(e: &mut Engine) -> ObjId {
    if let Some(p) = e.misc.notif_proto {
        return p;
    }
    let p = e.new_object();
    let tag = e.str_val("Notification");
    e.set_own_attrs(
        p,
        crate::interp::SYM_TO_STRING_TAG,
        tag,
        crate::heap::PropAttrs {
            writable: false,
            enumerable: false,
            configurable: true,
        },
    );
    e.misc.notif_proto = Some(p);
    p
}

/// `new Notification(title, options?)` — l'objet existe
/// (spec : la permission non-`granted` empêche l'affichage,
/// pas la construction) ; props `title`/`body`/`icon`/`tag`/
/// `dir`/`lang`/`data`/`silent`/`requireInteraction` relues
/// de `options` — `on*` assignables, `close()` no-op.
fn notif_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let title = e.to_js_string(arg(args, 0))?;
    let opts = arg(args, 1);
    let v = e.new_host_object(Rc::new(NotificationHost));
    let p = notif_proto(e);
    if let Value::Obj(id) = v {
        if let Some(o) = e.heap.obj_mut(id) {
            o.set_proto(Value::Obj(p));
        }
        let t = e.str_val(title);
        e.set_own(id, "title", t, true);
        // `options` relues — valeurs par défaut spec.
        let get = |e: &mut Engine, k: &str| {
            if let Value::Obj(oid) = opts {
                e.get_prop(oid, k)
            } else {
                None
            }
        };
        for k in ["body", "icon", "tag", "dir", "lang"] {
            let v2 = get(e, k).unwrap_or_else(|| e.str_val(""));
            // `body`/`icon`/`tag`/`dir`/`lang` → string.
            let s = e.to_js_string(v2).unwrap_or_default();
            let sv = e.str_val(s);
            e.set_own(id, k, sv, true);
        }
        let data = get(e, "data").unwrap_or(Value::Null);
        e.set_own(id, "data", data, true);
        for k in ["silent", "requireInteraction", "renotify"] {
            let b = get(e, k).map_or(Value::Bool(false), |x| Value::Bool(e.to_boolean(x)));
            e.set_own(id, k, b, true);
        }
        let ts = e.str_val("");
        e.set_own(id, "badge", ts, true);
        let img = e.str_val("");
        e.set_own(id, "image", img, true);
        let vib = e.new_array(Vec::new());
        e.set_own(id, "vibrate", vib, true);
        let acts = e.new_array(Vec::new());
        e.set_own(id, "actions", acts, true);
        e.set_own(id, "timestamp", Value::Num(0.0), true);
        for k in ["onclick", "onshow", "onerror", "onclose"] {
            e.set_own(id, k, Value::Null, true);
        }
        // `close()` — no-op (jamais affichée).
        e.add_method(id, "close", notif_noop);
    }
    Ok(v)
}

/// `Notification.requestPermission(cb?)` → promesse `'denied'`
/// — pas d'UI de grant dans le miroir : le refus est le
/// résultat honnête (spec : le user-agent décide).
fn notif_request_permission(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let denied = e.str_val("denied");
    if let Some(cb) = args.first() {
        if e.is_callable(cb.clone()) {
            e.call_value(cb.clone(), Value::Undefined, std::slice::from_ref(&denied))?;
        }
    }
    let p = e.new_promise();
    e.resolve_promise(p, denied);
    Ok(Value::Obj(p))
}

/// `close()`/`addEventListener` no-op de `Notification`.
fn notif_noop(_e: &mut Engine, _t: Value, _a: &[Value]) -> JsResult<Value> {
    Ok(Value::Undefined)
}

// ------------------------------------------------------------------
// Selection — réel minimal (spec : singleton par document)
// ------------------------------------------------------------------

/// Un `Range` de la sélection — `(node, offset)` → `(node,
/// offset)` dans l'arbre miroir.
#[derive(Clone, Copy)]
pub(crate) struct SelRange {
    /// Conteneur de début.
    start_node: NodeId,
    /// Offset de début (enfants du conteneur).
    start_off: u32,
    /// Conteneur de fin.
    end_node: NodeId,
    /// Offset de fin.
    end_off: u32,
}

/// Données partagées de la `Selection` courante.
struct SelData {
    /// Ranges — spec : `rangeCount` ≤ 1 sauf Firefox
    /// multi-range (le miroir borne à 1 : `addRange` remplace).
    ranges: Vec<Rc<RefCell<SelRange>>>,
}

/// Host `Selection` — partage `SelData` (singleton).
struct SelectionHost {
    data: Rc<RefCell<SelData>>,
    /// Contexte DOM pour résoudre les nœuds.
    dom: crate::dom_host::DomRef,
    cache: crate::dom_host::NodeCache,
}

/// `SelData` partagé (lazy).
fn sel_data(e: &mut Engine) -> Rc<RefCell<SelData>> {
    if let Some(d) = &e.misc.sel {
        return d.clone();
    }
    let d = Rc::new(RefCell::new(SelData { ranges: Vec::new() }));
    e.misc.sel = Some(d.clone());
    d
}

/// `Selection.prototype` paresseux.
fn sel_proto(e: &mut Engine) -> ObjId {
    if let Some(p) = e.misc.sel_proto {
        return p;
    }
    let p = e.new_object();
    let tag = e.str_val("Selection");
    e.set_own_attrs(
        p,
        crate::interp::SYM_TO_STRING_TAG,
        tag,
        crate::heap::PropAttrs {
            writable: false,
            enumerable: false,
            configurable: true,
        },
    );
    e.misc.sel_proto = Some(p);
    p
}

/// `window.getSelection()`/`document.getSelection()` — **le
/// même objet** `Selection` à chaque appel (spec : singleton
/// par document — `s1 === s2`).
pub(crate) fn get_selection(e: &mut Engine, _t: Value, _a: &[Value]) -> JsResult<Value> {
    let Some((dom, cache)) = e.dom_ctx.clone() else {
        return Ok(Value::Null);
    };
    if let Some(w) = e.misc.sel_wrapper {
        if e.heap.obj(w).is_some() {
            return Ok(Value::Obj(w));
        }
    }
    let data = sel_data(e);
    let v = e.new_host_object(Rc::new(SelectionHost { data, dom, cache }));
    let p = sel_proto(e);
    if let Value::Obj(id) = v {
        if let Some(o) = e.heap.obj_mut(id) {
            o.set_proto(Value::Obj(p));
        }
        e.misc.sel_wrapper = Some(id);
    }
    Ok(v)
}

/// Texte couvert par un `SelRange` — approximation honnête :
/// `start == end` même nœud → `textContent` du nœud borné
/// par les offsets d'enfants ; sinon `textContent` du plus
/// petit ancêtre commun approximé par `start_node` (les
/// ranges réelles du miroir sont quasi-toujours
/// mono-élément via `selectAllChildren`).
fn sel_range_text(dom: &crate::dom_host::DomRef, r: &SelRange) -> String {
    let d = dom.borrow();
    if r.start_node == r.end_node {
        let kids = d.child_nodes(r.start_node);
        if r.start_off == 0 && r.end_off as usize >= kids.len() {
            return d.text_content(r.start_node);
        }
        return kids
            .iter()
            .skip(r.start_off as usize)
            .take((r.end_off.saturating_sub(r.start_off)) as usize)
            .map(|&k| d.text_content(k))
            .collect();
    }
    d.text_content(r.start_node)
}

impl Host for SelectionHost {
    fn class_name(&self) -> &'static str {
        "Selection"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn get(&self, e: &mut Engine, _t: Value, key: &str) -> JsResult<Option<Value>> {
        let d = self.data.borrow();
        Ok(Some(match key {
            "rangeCount" => Value::Num(d.ranges.len() as f64),
            "isCollapsed" => Value::Bool(d.ranges.first().is_none_or(|r| {
                let r = r.borrow();
                r.start_node == r.end_node && r.start_off == r.end_off
            })),
            "type" => {
                return Ok(Some(e.str_val(if d.ranges.is_empty() {
                    "None"
                } else if d.ranges[0].borrow().start_node == d.ranges[0].borrow().end_node
                    && d.ranges[0].borrow().start_off == d.ranges[0].borrow().end_off
                {
                    "Caret"
                } else {
                    "Range"
                })))
            }
            "anchorNode" | "focusNode" => {
                let r = d.ranges.first();
                return Ok(Some(match (key, r) {
                    (_, None) => Value::Null,
                    ("anchorNode", Some(r)) => {
                        let n = r.borrow().start_node;
                        node_value(e, &self.dom, &self.cache, n)
                    }
                    (_, Some(r)) => {
                        let n = r.borrow().end_node;
                        node_value(e, &self.dom, &self.cache, n)
                    }
                }));
            }
            "anchorOffset" => Value::Num(f64::from(
                d.ranges.first().map_or(0, |r| r.borrow().start_off),
            )),
            "focusOffset" => Value::Num(f64::from(
                d.ranges.first().map_or(0, |r| r.borrow().end_off),
            )),
            "direction" => {
                // Spec : `'none'` quand collapsée, sinon le sens
                // ancre→focus — le miroir modélise le range seul :
                // début avant fin = `'forward'`, inversé
                // (`setBaseAndExtent` arrière) = `'backward'`.
                let dir = d.ranges.first().map_or("none", |r| {
                    let r = r.borrow();
                    if r.start_node == r.end_node && r.start_off == r.end_off {
                        "none"
                    } else {
                        match cmp_point(
                            &self.dom,
                            (r.start_node, r.start_off),
                            (r.end_node, r.end_off),
                        ) {
                            x if x < 0 => "forward",
                            _ => "backward",
                        }
                    }
                });
                return Ok(Some(e.str_val(dir)));
            }
            _ => return Ok(None),
        }))
    }

    fn has_method(&self, name: &str) -> bool {
        // Contrat : déclare exactement les noms dispatchés par
        // `invoke` ci-dessous — sans ça `'addRange' in sel` et
        // `typeof sel.addRange` mentaient (invoke-only invisible,
        // JS-C555).
        matches!(
            name,
            "toString"
                | "getRangeAt"
                | "selectAllChildren"
                | "collapse"
                | "collapseToStart"
                | "collapseToEnd"
                | "setPosition"
                | "extend"
                | "addRange"
                | "removeRange"
                | "removeAllRanges"
                | "empty"
                | "setBaseAndExtent"
                | "containsNode"
                | "deleteFromDocument"
                | "getComposedRanges"
                | "modify"
        )
    }

    fn invoke(
        &self,
        e: &mut Engine,
        _t: Value,
        name: &str,
        args: &[Value],
    ) -> JsResult<Option<Value>> {
        match name {
            "toString" => {
                let d = self.data.borrow();
                let text = d
                    .ranges
                    .iter()
                    .map(|r| sel_range_text(&self.dom, &r.borrow()))
                    .collect::<String>();
                Ok(Some(e.str_val(text)))
            }
            "getRangeAt" => {
                let i = e.to_number(arg(args, 0))? as usize;
                let d = self.data.borrow();
                let Some(r) = d.ranges.get(i) else {
                    // Spec : index hors borne → `INDEX_SIZE_ERR`
                    // — TypeError honnête ici.
                    return Err(e.type_error("Selection.getRangeAt: index out of bounds"));
                };
                Ok(Some(range_shared(e, r.clone())))
            }
            "selectAllChildren" => {
                let Some(node) = node_for_in(e, arg(args, 0), &self.dom) else {
                    return Err(e.type_error("Selection.selectAllChildren: not a Node"));
                };
                let count = self.dom.borrow().child_nodes(node).len() as u32;
                self.data.borrow_mut().ranges = vec![Rc::new(RefCell::new(SelRange {
                    start_node: node,
                    start_off: 0,
                    end_node: node,
                    end_off: count,
                }))];
                Ok(Some(Value::Undefined))
            }
            "collapse" | "collapseToStart" | "collapseToEnd" | "setPosition" | "extend" => {
                let d = self.data.borrow();
                // `setPosition(node, off)` = alias webkit de
                // `collapse` — lit les args comme elle, pas le
                // range courant (bug JS-C556 : sinon il recopiait
                // la borne `end` du premier range).
                let node = if matches!(name, "collapse" | "extend" | "setPosition") {
                    node_for_in(e, arg(args, 0), &self.dom)
                } else {
                    d.ranges.first().map(|r| {
                        let r = r.borrow();
                        if name == "collapseToStart" {
                            r.start_node
                        } else {
                            r.end_node
                        }
                    })
                };
                let off = if matches!(name, "collapse" | "extend" | "setPosition") {
                    e.to_number(arg(args, 1)).unwrap_or(0.0) as u32
                } else {
                    d.ranges.first().map_or(0, |r| {
                        let r = r.borrow();
                        if name == "collapseToStart" {
                            r.start_off
                        } else {
                            r.end_off
                        }
                    })
                };
                drop(d);
                // `extend` garde l'ancre — seul le focus bouge.
                if name == "extend" {
                    let mut dd = self.data.borrow_mut();
                    if let (Some(n), Some(r)) = (node, dd.ranges.first_mut()) {
                        let mut r = r.borrow_mut();
                        r.end_node = n;
                        r.end_off = off;
                    }
                } else if let Some(n) = node {
                    self.data.borrow_mut().ranges = vec![Rc::new(RefCell::new(SelRange {
                        start_node: n,
                        start_off: off,
                        end_node: n,
                        end_off: off,
                    }))];
                }
                Ok(Some(Value::Undefined))
            }
            "addRange" => {
                let r = arg(args, 0);
                // Vrai `Range` host (CAP-182) → bornes lues en
                // direct via downcast ; sinon chemin props
                // (snapshot `getRangeAt`/`cloneRange` historique).
                let (sn, so, en, eo) = if let Value::Obj(id) = r {
                    let shared = e.host_of(id).and_then(|h| {
                        h.as_any()
                            .downcast_ref::<RangeHost>()
                            .map(|rh| *rh.data.borrow())
                    });
                    if let Some(rd) = shared {
                        (
                            Some(rd.start_node),
                            rd.start_off,
                            Some(rd.end_node),
                            rd.end_off,
                        )
                    } else {
                        let sn = e
                            .get_prop(id, "startContainer")
                            .and_then(|v| node_for_in(e, v, &self.dom));
                        let en = e
                            .get_prop(id, "endContainer")
                            .and_then(|v| node_for_in(e, v, &self.dom));
                        let so = e
                            .get_prop(id, "startOffset")
                            .and_then(|v| e.to_number(v).ok())
                            .unwrap_or(0.0) as u32;
                        let eo = e
                            .get_prop(id, "endOffset")
                            .and_then(|v| e.to_number(v).ok())
                            .unwrap_or(0.0) as u32;
                        (sn, so, en, eo)
                    }
                } else {
                    (None, 0, None, 0)
                };
                let (Some(sn), Some(en)) = (sn, en) else {
                    return Err(e.type_error("Selection.addRange: not a Range"));
                };
                // Miroir : un seul range (comportement Chrome).
                self.data.borrow_mut().ranges = vec![Rc::new(RefCell::new(SelRange {
                    start_node: sn,
                    start_off: so,
                    end_node: en,
                    end_off: eo,
                }))];
                Ok(Some(Value::Undefined))
            }
            "removeRange" | "removeAllRanges" | "empty" => {
                self.data.borrow_mut().ranges.clear();
                Ok(Some(Value::Undefined))
            }
            "setBaseAndExtent" => {
                let sn = node_for_in(e, arg(args, 0), &self.dom);
                let en = node_for_in(e, arg(args, 2), &self.dom);
                let so = e.to_number(arg(args, 1)).unwrap_or(0.0) as u32;
                let eo = e.to_number(arg(args, 3)).unwrap_or(0.0) as u32;
                let (Some(sn), Some(en)) = (sn, en) else {
                    return Err(e.type_error("Selection.setBaseAndExtent: not a Node"));
                };
                self.data.borrow_mut().ranges = vec![Rc::new(RefCell::new(SelRange {
                    start_node: sn,
                    start_off: so,
                    end_node: en,
                    end_off: eo,
                }))];
                Ok(Some(Value::Undefined))
            }
            "containsNode" => {
                // Approximation honnête : vrai si `node` est le
                // conteneur ou un descendant du conteneur.
                let Some(n) = node_for_in(e, arg(args, 0), &self.dom) else {
                    return Ok(Some(Value::Bool(false)));
                };
                let d = self.data.borrow();
                let hit = d.ranges.iter().any(|r| {
                    let r = r.borrow();
                    if r.start_node == n || r.end_node == n {
                        return true;
                    }
                    let mut cur = self.dom.borrow().parent(n);
                    while let Some(p) = cur {
                        if p == r.start_node || p == r.end_node {
                            return true;
                        }
                        cur = self.dom.borrow().parent(p);
                    }
                    false
                });
                Ok(Some(Value::Bool(hit)))
            }
            "deleteFromDocument" => {
                // Réel pour le cas mono-conteneur : retirer les
                // enfants couverts ; multi-conteneur → no-op
                // (approximation documentée).
                let ranges = self.data.borrow().ranges.clone();
                for r in ranges {
                    let r = r.borrow();
                    if r.start_node == r.end_node {
                        let kids = self.dom.borrow().child_nodes(r.start_node);
                        for &k in kids
                            .iter()
                            .skip(r.start_off as usize)
                            .take((r.end_off.saturating_sub(r.start_off)) as usize)
                        {
                            self.dom.borrow_mut().detach(k);
                        }
                    }
                }
                self.data.borrow_mut().ranges.clear();
                Ok(Some(Value::Undefined))
            }
            "getComposedRanges" => {
                // Spec shadow-aware non portée — approximation
                // honnête documentée : les ranges de la sélection
                // (le miroir n'ouvre qu'un seul arbre composé).
                let ranges = self.data.borrow().ranges.clone();
                let vals = ranges
                    .into_iter()
                    .map(|r| range_shared(e, r))
                    .collect::<Vec<_>>();
                Ok(Some(e.new_array(vals)))
            }
            "modify" => {
                // Borné (JS-C556) : granularité `'character'`
                // seule — `alter='move'` pose un caret, `'extend'`
                // ne bouge que le focus ; `direction` forward/
                // backward/left/right (ltr : right≡forward). Les
                // autres granularités (word/line/…) sont no-op
                // documentées, et la traversée inter-nœuds reste
                // clampée aux bornes du conteneur texte courant.
                let alter = e.to_js_string(arg(args, 0))?;
                let dir = e.to_js_string(arg(args, 1))?;
                let gran = e.to_js_string(arg(args, 2))?;
                if gran != "character" {
                    return Ok(Some(Value::Undefined));
                }
                let step: i64 = match dir.as_str() {
                    "backward" | "left" => -1,
                    _ => 1,
                };
                let rc = self.data.borrow().ranges.first().cloned();
                if let Some(rc) = rc {
                    let mut r = rc.borrow_mut();
                    let len = {
                        let d = self.dom.borrow();
                        match d.nodes.get(r.end_node as usize) {
                            Some(n) if matches!(n.kind, crate::dom::NodeKind::Text) => {
                                d.text_content(r.end_node).chars().count() as i64
                            }
                            Some(_) => d.child_nodes(r.end_node).len() as i64,
                            None => 0,
                        }
                    };
                    let no = (i64::from(r.end_off) + step).clamp(0, len) as u32;
                    r.end_off = no;
                    if alter == "move" {
                        r.start_node = r.end_node;
                        r.start_off = no;
                    }
                }
                Ok(Some(Value::Undefined))
            }
            _ => Ok(None),
        }
    }

    fn trace(&self, out: &mut Vec<ObjId>) {
        crate::dom_host::trace_node_cache(&self.cache, out);
    }
}

// ------------------------------------------------------------------
// reportError — réel
// ------------------------------------------------------------------

/// `globalThis.reportError(v)` — spec : « report the
/// exception » = `ErrorEvent` `error` dispatché sur `window`.
/// `window.onerror` reçoit la signature legacy `(message,
/// source, lineno, colno, error)` (spec HTML) ; un retour
/// truthy ou `preventDefault` sur les listeners `error` de la
/// racine empêche le rapport — sinon `console` embedder log
/// `Uncaught: <v>` (sortie réelle d'un uncaught).
fn report_error(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    report_exception(e, arg(args, 0))?;
    Ok(Value::Undefined)
}

/// « Report the exception » réutilisable (JS listener qui
/// jette pendant `dispatchEvent` — spec : rapporté, jamais
/// propagé à l'appelant, dispatch poursuivi).
pub(crate) fn report_exception(e: &mut Engine, v: Value) -> JsResult<()> {
    // Borne anti-récursion : un listener `error` qui jette
    // redéclencherait ce rail à l'infini.
    if e.report_depth.get() >= 4 {
        return Ok(());
    }
    e.report_depth.set(e.report_depth.get() + 1);
    let r = report_exception_inner(e, v);
    e.report_depth.set(e.report_depth.get() - 1);
    r
}

fn report_exception_inner(e: &mut Engine, v: Value) -> JsResult<()> {
    let message = e.to_js_string(v.clone()).unwrap_or_default();
    // 1. `window.onerror` legacy — 5 arguments (spec HTML) ;
    //    retour truthy = erreur « gérée », pas de rapport.
    let g = e.global;
    if let Some(f) = e.get_prop(g, "onerror") {
        if e.is_callable(f.clone()) {
            let m = e.str_val(message.clone());
            let src = e.str_val(e.hooks.page_url.clone());
            let ret = e.call_value(
                f,
                Value::Obj(g),
                &[m, src, Value::Num(0.0), Value::Num(0.0), v.clone()],
            )?;
            if e.to_boolean(ret) {
                return Ok(());
            }
        }
    }
    // 2. Listeners `error` de la racine — ErrorEvent réel via
    //    la propagation DOM (`preventDefault` = géré).
    let mut prevented = false;
    if let Some((dom, cache)) = e.dom_ctx.clone() {
        let root = dom.borrow().root;
        let o = e.new_object();
        crate::dom_host::event_proto(e, o, "ErrorEvent");
        let t = e.str_val("error");
        e.set_own(o, "type", t, true);
        let m = e.str_val(message.clone());
        e.set_own(o, "message", m, true);
        e.set_own(o, "error", v, true);
        e.set_own(o, "cancelable", Value::Bool(true), true);
        if let Some(state) =
            crate::dom_host::misc_event_state(e, root, &dom, &cache, Value::Obj(o))?
        {
            e.propagate_event(&dom, &cache, &state)?;
            prevented = state.default_prevented();
        }
    }
    // 3. Non-annulé → rapport embedder (`Uncaught: …` — le
    //    canal console, même famille que les uncaught réels).
    if !prevented {
        if let Some(f) = e.hooks.console.clone() {
            f(format!("Uncaught: {message}"));
        }
    }
    Ok(())
}

// ------------------------------------------------------------------
// Installation
// ------------------------------------------------------------------

/// `install_misc` — `Notification`, `BroadcastChannel`,
/// `reportError` sur le global (JS-C74). `getSelection`
/// n'est posé qu'avec un DOM (`install_dom`).
/// Idempotent — `install_fetch`/`install_dom` l'appellent
/// tous deux.
pub(crate) fn install_misc(e: &mut Engine) {
    let g = e.global;
    if e.get_prop(g, "BroadcastChannel").is_some() {
        return;
    }
    // `BroadcastChannel` — réel intra-contexte.
    let bc = e.native_fn("BroadcastChannel", bc_ctor as crate::heap::NativeFn);
    if let Value::Obj(cid) = bc {
        let p = bc_proto(e);
        e.set_own_attrs(
            cid,
            "prototype",
            Value::Obj(p),
            crate::heap::PropAttrs::FIXED,
        );
        e.set_own(p, "constructor", bc.clone(), false);
    }
    e.set_own(g, "BroadcastChannel", bc, true);
    // `Notification` — stub honnête (`permission` 'default',
    // `requestPermission` → 'denied', instances inertes).
    let nf = e.native_fn("Notification", notif_ctor as crate::heap::NativeFn);
    if let Value::Obj(cid) = nf {
        let p = notif_proto(e);
        e.set_own_attrs(
            cid,
            "prototype",
            Value::Obj(p),
            crate::heap::PropAttrs::FIXED,
        );
        e.set_own(p, "constructor", nf.clone(), false);
        // Statiques : `permission` own-prop figée `'default'`
        // (honnête — le grant n'est pas atteignable sans UI,
        // jamais `'granted'` mensonger), `requestPermission`,
        // `maxActions` (0 — jamais d'actions sans UI).
        let d = e.str_val("default");
        e.set_own(cid, "permission", d, true);
        e.add_method(cid, "requestPermission", notif_request_permission);
        e.set_own(cid, "maxActions", Value::Num(0.0), true);
    }
    e.set_own(g, "Notification", nf, true);
    // `reportError` — réel (dispatch + hook).
    let re = e.native_fn("reportError", report_error as crate::heap::NativeFn);
    e.set_own(g, "reportError", re, true);
    // `getSelection` — DOM requis (Selection = nœuds miroir) ;
    // posé par install_dom directement.
    if e.dom_ctx.is_some() {
        let gs = e.native_fn("getSelection", get_selection as crate::heap::NativeFn);
        e.set_own(g, "getSelection", gs, true);
    }
    // `__tcfapi` (+ APIs sœurs sous politique résolue) —
    // délégué à `cmp.rs` (JS-C259) : stub honnête CMP-less
    // par défaut (JS-C213), résolveur formé IAB v2 quand la
    // session porte une décision de consentement.
    crate::cmp::install_cmp(e);
}

/// `args[i]` ou `Undefined` (même convention que `xhr::arg`).
fn arg(args: &[Value], i: usize) -> Value {
    args.get(i).cloned().unwrap_or(Value::Undefined)
}

// ===== CAP-182 — Range / TreeWalker / execCommand (édition) —

/// Host `Range` — frontières mutables (start/end sur des nœuds
/// miroir). `Selection.addRange` lit `startContainer`/`endContainer`/
/// `startOffset`/`endOffset` — exposés ici → intégration directe.
struct RangeHost {
    data: Rc<RefCell<SelRange>>,
    dom: crate::dom_host::DomRef,
    cache: crate::dom_host::NodeCache,
}

/// `(parent, index)` d'un nœud parmi les enfants de son parent.
fn node_index(dom: &crate::dom_host::DomRef, n: NodeId) -> Option<(NodeId, u32)> {
    let d = dom.borrow();
    let p = d.parent(n)?;
    let i = d.child_nodes(p).iter().position(|&c| c == n)? as u32;
    Some((p, i))
}

/// Ancêtre commun de deux nœuds — montée depuis `a` jusqu'à
/// contenir `b` (document-order).
fn common_ancestor(dom: &crate::dom_host::DomRef, a: NodeId, b: NodeId) -> NodeId {
    let d = dom.borrow();
    let mut anc = a;
    loop {
        if d.contains(anc, b) {
            return anc;
        }
        match d.parent(anc) {
            Some(p) => anc = p,
            None => return anc,
        }
    }
}

/// Ordre document de deux boundary points `(node, off)` —
/// `-1`/`0`/`+1` (spec DOM *compare boundary points*).
/// L'offset d'un conteneur-ancêtre se compare à l'index de la
/// branche couvrant l'autre point ; à défaut d'ancêtre direct,
/// les index des deux branches sous l'ancêtre commun tranchent.
fn cmp_point(dom: &crate::dom_host::DomRef, a: (NodeId, u32), b: (NodeId, u32)) -> i32 {
    let (an, ao) = a;
    let (bn, bo) = b;
    if an == bn {
        return match ao.cmp(&bo) {
            std::cmp::Ordering::Less => -1,
            std::cmp::Ordering::Equal => 0,
            std::cmp::Ordering::Greater => 1,
        };
    }
    let d = dom.borrow();
    // Enfant de `anc` porteur de la branche qui contient `n`
    // (remontée bornée : la racine termine toujours la boucle).
    let branch_of = |anc: NodeId, mut n: NodeId| -> NodeId {
        while let Some(p) = d.parent(n) {
            if p == anc {
                return n;
            }
            n = p;
        }
        n
    };
    if d.contains(an, bn) {
        let c = branch_of(an, bn);
        let i = d.child_nodes(an).iter().position(|&k| k == c).unwrap_or(0) as u32;
        // `(an,ao)` précède `b` ssi l'offset n'atteint pas la
        // branche qui contient `b`.
        return if ao <= i { -1 } else { 1 };
    }
    if d.contains(bn, an) {
        let c = branch_of(bn, an);
        let i = d.child_nodes(bn).iter().position(|&k| k == c).unwrap_or(0) as u32;
        // `a` vit dans l'enfant `i` de `bn` : avant `(bn,bo)`
        // ssi `i < bo`.
        return if i < bo { -1 } else { 1 };
    }
    let anc = common_ancestor(dom, an, bn);
    let kids = d.child_nodes(anc);
    let ia = kids
        .iter()
        .position(|&k| k == branch_of(anc, an))
        .unwrap_or(0);
    let ib = kids
        .iter()
        .position(|&k| k == branch_of(anc, bn))
        .unwrap_or(0);
    match ia.cmp(&ib) {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    }
}

/// Enfants top-level de l'ancêtre commun couverts par le range
/// (borné : même-conteneur = tranche `[start_off..end_off)` ;
/// multi-niveaux = enfants de l'ancêtre entre la branche de
/// début et celle de fin — les descendants plus profonds
/// partent avec leur parent).
fn range_covered_top(dom: &crate::dom_host::DomRef, r: &SelRange) -> Vec<NodeId> {
    let d = dom.borrow();
    if r.start_node == r.end_node {
        let kids = d.child_nodes(r.start_node);
        return kids
            .iter()
            .skip(r.start_off as usize)
            .take(r.end_off.saturating_sub(r.start_off) as usize)
            .copied()
            .collect();
    }
    let anc = common_ancestor(dom, r.start_node, r.end_node);
    let kids = d.child_nodes(anc);
    let branch = |mut n: NodeId| {
        while d.parent(n).is_some_and(|p| p != anc) {
            n = d.parent(n).unwrap_or(n);
        }
        n
    };
    let si = kids
        .iter()
        .position(|&k| k == branch(r.start_node))
        .unwrap_or(0);
    let ei = kids
        .iter()
        .position(|&k| k == branch(r.end_node))
        .unwrap_or_else(|| kids.len().saturating_sub(1));
    if kids.is_empty() {
        return Vec::new();
    }
    kids[si.min(kids.len() - 1)..=ei.min(kids.len() - 1)].to_vec()
}

/// Suppression des contenus couverts : texte → sous-chaîne
/// retirée ; sinon les nœuds couverts sont détachés.
fn delete_range_contents(dom: &crate::dom_host::DomRef, r: &SelRange) {
    let (covered, is_text) = {
        let d = dom.borrow();
        (
            range_covered_top(dom, r),
            r.start_node == r.end_node
                && matches!(
                    d.nodes[r.start_node as usize].kind,
                    crate::dom::NodeKind::Text
                ),
        )
    };
    let mut d = dom.borrow_mut();
    if is_text {
        let t = d.text_content(r.start_node);
        let kept: String = t
            .chars()
            .take(r.start_off as usize)
            .chain(t.chars().skip(r.end_off as usize))
            .collect();
        d.set_text(r.start_node, &kept);
        return;
    }
    for n in covered {
        d.detach(n);
    }
}

/// `AbstractRange.prototype` paresseux — ancêtre proto de
/// `Range.prototype`/`StaticRange.prototype` (spec DOM : les
/// accesseurs bornes y vivent en get-only). `pub(crate)` :
/// adopté par le ctor à l'install (identité `instanceof`).
pub(crate) fn abstract_range_proto(e: &mut Engine) -> ObjId {
    if let Some(p) = e.misc.abstract_range_proto {
        return p;
    }
    let p = e.new_object();
    let tag = e.str_val("AbstractRange");
    e.set_own_attrs(
        p,
        crate::interp::SYM_TO_STRING_TAG,
        tag,
        crate::heap::PropAttrs {
            writable: false,
            enumerable: false,
            configurable: true,
        },
    );
    for key in [
        "startContainer",
        "startOffset",
        "endContainer",
        "endOffset",
        "collapsed",
    ] {
        let kv = e.str_val(key);
        let g = e.bound_native("get", abstract_range_get, vec![kv]);
        if let Some(o) = e.heap.obj_mut(p) {
            o.define_accessor(key, g, Value::Undefined, crate::heap::PropAttrs::BUILTIN);
        }
    }
    e.misc.abstract_range_proto = Some(p);
    p
}

/// Getter `AbstractRange.prototype.<borne>` — `this` doit être
/// un range host (`RangeHost`/`StaticRangeHost`), sinon
/// `TypeError` « Illegal invocation » (spec : l'accesseur
/// vérifie la marque interne, pas la forme). La clé est liée
/// dans `bound` (`bound_native` préfixe les args).
fn abstract_range_get(e: &mut Engine, t: Value, args: &[Value]) -> JsResult<Value> {
    let key = e.to_js_string(arg(args, 0))?;
    let Value::Obj(id) = t else {
        return Err(e.type_error("Illegal invocation"));
    };
    let Some((r, dom, cache)) = ({
        let o = e.heap.obj(id);
        o.and_then(|o| match &o.kind {
            crate::heap::ObjKind::Host(h) => {
                if let Some(rh) = h.as_any().downcast_ref::<RangeHost>() {
                    Some((*rh.data.borrow(), rh.dom.clone(), rh.cache.clone()))
                } else {
                    h.as_any()
                        .downcast_ref::<StaticRangeHost>()
                        .map(|sh| (sh.r, sh.dom.clone(), sh.cache.clone()))
                }
            }
            _ => None,
        })
    }) else {
        return Err(e.type_error("Illegal invocation"));
    };
    Ok(match key.as_str() {
        "startContainer" => node_value(e, &dom, &cache, r.start_node),
        "endContainer" => node_value(e, &dom, &cache, r.end_node),
        "startOffset" => Value::Num(f64::from(r.start_off)),
        "endOffset" => Value::Num(f64::from(r.end_off)),
        "collapsed" => Value::Bool(r.start_node == r.end_node && r.start_off == r.end_off),
        _ => Value::Undefined,
    })
}

/// `Range.prototype` paresseux (une fois par engine).
/// `pub(crate)` : adopté par `Range.prototype` du ctor à
/// l'install — `r instanceof Range` exige l'identité
/// (JS-C556 : le `new_object()` dédié la cassait).
pub(crate) fn range_proto(e: &mut Engine) -> ObjId {
    if let Some(p) = e.misc.range_proto {
        return p;
    }
    let p = e.new_object();
    let tag = e.str_val("Range");
    e.set_own_attrs(
        p,
        crate::interp::SYM_TO_STRING_TAG,
        tag,
        crate::heap::PropAttrs {
            writable: false,
            enumerable: false,
            configurable: true,
        },
    );
    // Chaîne spec : `Range.prototype.__proto__ ===
    // AbstractRange.prototype` → `r instanceof AbstractRange`.
    let ar = Value::Obj(abstract_range_proto(e));
    if let Some(o) = e.heap.obj_mut(p) {
        o.set_proto(ar);
    }
    e.misc.range_proto = Some(p);
    p
}

/// Objet `Range` vivant (frontières mutables) — utilisé par
/// `new Range()`/`document.createRange()` et `cloneRange`.
pub(crate) fn range_value(e: &mut Engine, r: SelRange) -> Value {
    range_shared(e, Rc::new(RefCell::new(r)))
}

/// `Range` vivant sur une donnée **partagée** — `getRangeAt`
/// retourne le même contenu que le range ajouté (mutations
/// visibles dans la `Selection`, spec).
fn range_shared(e: &mut Engine, data: Rc<RefCell<SelRange>>) -> Value {
    let Some((dom, cache)) = e.dom_ctx.clone() else {
        return Value::Null;
    };
    let v = e.new_host_object(Rc::new(RangeHost { data, dom, cache }));
    let p = range_proto(e);
    if let Value::Obj(id) = v {
        if let Some(o) = e.heap.obj_mut(id) {
            o.set_proto(Value::Obj(p));
        }
    }
    v
}

/// `new Range()` — spec-constructible (CAP-182) : frontières
/// `(document, 0)`/`(document, 0)`. Sans document → `TypeError`.
pub(crate) fn range_ctor(e: &mut Engine, _t: Value, _a: &[Value]) -> JsResult<Value> {
    let Some((dom, _cache)) = e.dom_ctx.clone() else {
        return Err(e.type_error("Range: no document"));
    };
    let root = dom.borrow().root;
    Ok(range_value(
        e,
        SelRange {
            start_node: root,
            start_off: 0,
            end_node: root,
            end_off: 0,
        },
    ))
}

impl Host for RangeHost {
    fn class_name(&self) -> &'static str {
        "Range"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn trace(&self, out: &mut Vec<ObjId>) {
        crate::dom_host::trace_node_cache(&self.cache, out);
    }

    fn has_method(&self, name: &str) -> bool {
        matches!(
            name,
            "setStart"
                | "setEnd"
                | "setStartBefore"
                | "setStartAfter"
                | "setEndBefore"
                | "setEndAfter"
                | "selectNode"
                | "selectNodeContents"
                | "collapse"
                | "cloneContents"
                | "extractContents"
                | "createContextualFragment"
                | "deleteContents"
                | "insertNode"
                | "cloneRange"
                | "detach"
                | "toString"
                | "getBoundingClientRect"
                | "getClientRects"
                | "compareBoundaryPoints"
                | "comparePoint"
                | "isPointInRange"
                | "intersectsNode"
                | "surroundContents"
        )
    }

    fn get(&self, e: &mut Engine, _t: Value, key: &str) -> JsResult<Option<Value>> {
        let d = self.data.borrow();
        Ok(Some(match key {
            "startContainer" => node_value(e, &self.dom, &self.cache, d.start_node),
            "endContainer" => node_value(e, &self.dom, &self.cache, d.end_node),
            "startOffset" => Value::Num(f64::from(d.start_off)),
            "endOffset" => Value::Num(f64::from(d.end_off)),
            "collapsed" => Value::Bool(d.start_node == d.end_node && d.start_off == d.end_off),
            "commonAncestorContainer" => {
                let anc = common_ancestor(&self.dom, d.start_node, d.end_node);
                node_value(e, &self.dom, &self.cache, anc)
            }
            _ => return Ok(None),
        }))
    }

    fn invoke(
        &self,
        e: &mut Engine,
        _t: Value,
        name: &str,
        args: &[Value],
    ) -> JsResult<Option<Value>> {
        match name {
            "setStart" | "setEnd" => {
                let Some(n) = node_for_in(e, arg(args, 0), &self.dom) else {
                    return Err(e.type_error("Range.set*: not a Node"));
                };
                let off = e.to_number(arg(args, 1)).unwrap_or(0.0).max(0.0) as u32;
                let mut d = self.data.borrow_mut();
                if name == "setStart" {
                    d.start_node = n;
                    d.start_off = off;
                    // Spec : un début posé au-delà de la fin la
                    // rabat dessus (borné : même-conteneur seul —
                    // l'ordre inter-nœuds exige
                    // `compareDocumentPosition`, non porté).
                    if n == d.end_node && off > d.end_off {
                        d.end_off = off;
                    }
                } else {
                    d.end_node = n;
                    d.end_off = off;
                    if n == d.start_node && off < d.start_off {
                        d.start_off = off;
                    }
                }
                Ok(Some(Value::Undefined))
            }
            "setStartBefore" | "setStartAfter" | "setEndBefore" | "setEndAfter" => {
                let Some(n) = node_for_in(e, arg(args, 0), &self.dom) else {
                    return Err(e.type_error("Range.set*: not a Node"));
                };
                let Some((p, i)) = node_index(&self.dom, n) else {
                    return Err(crate::heap::Thrown(crate::dom_host::dom_ex_pub(
                        e,
                        "InvalidNodeTypeError",
                        "node has no parent",
                    )));
                };
                let off = if name.ends_with("After") { i + 1 } else { i };
                let mut d = self.data.borrow_mut();
                if name.starts_with("setStart") {
                    d.start_node = p;
                    d.start_off = off;
                    if p == d.end_node && off > d.end_off {
                        d.end_off = off;
                    }
                } else {
                    d.end_node = p;
                    d.end_off = off;
                    if p == d.start_node && off < d.start_off {
                        d.start_off = off;
                    }
                }
                Ok(Some(Value::Undefined))
            }
            "selectNode" | "selectNodeContents" => {
                let Some(n) = node_for_in(e, arg(args, 0), &self.dom) else {
                    return Err(e.type_error("Range.select*: not a Node"));
                };
                if name == "selectNode" {
                    let Some((p, i)) = node_index(&self.dom, n) else {
                        return Err(crate::heap::Thrown(crate::dom_host::dom_ex_pub(
                            e,
                            "InvalidNodeTypeError",
                            "node has no parent",
                        )));
                    };
                    let mut d = self.data.borrow_mut();
                    d.start_node = p;
                    d.start_off = i;
                    d.end_node = p;
                    d.end_off = i + 1;
                } else {
                    let count = self.dom.borrow().child_nodes(n).len() as u32;
                    let mut d = self.data.borrow_mut();
                    d.start_node = n;
                    d.start_off = 0;
                    d.end_node = n;
                    d.end_off = count;
                }
                Ok(Some(Value::Undefined))
            }
            "collapse" => {
                let to_start = e.to_boolean(arg(args, 0));
                let mut d = self.data.borrow_mut();
                if to_start {
                    d.end_node = d.start_node;
                    d.end_off = d.start_off;
                } else {
                    d.start_node = d.end_node;
                    d.start_off = d.end_off;
                }
                Ok(Some(Value::Undefined))
            }

            "cloneContents" | "extractContents" => {
                let r = *self.data.borrow();
                let covered = range_covered_top(&self.dom, &r);
                let frag = {
                    let mut d = self.dom.borrow_mut();
                    let frag = d.create_fragment();
                    if r.start_node == r.end_node
                        && matches!(
                            d.nodes[r.start_node as usize].kind,
                            crate::dom::NodeKind::Text
                        )
                    {
                        let t = d.text_content(r.start_node);
                        let sub: String = t
                            .chars()
                            .skip(r.start_off as usize)
                            .take(r.end_off.saturating_sub(r.start_off) as usize)
                            .collect();
                        let tn = d.create_text(&sub);
                        d.insert_before(frag, tn, None);
                    } else {
                        for &n in &covered {
                            let c = d.clone_node(n, true);
                            d.insert_before(frag, c, None);
                        }
                    }
                    frag
                };
                if name == "extractContents" {
                    delete_range_contents(&self.dom, &r);
                }
                Ok(Some(node_value(e, &self.dom, &self.cache, frag)))
            }
            "deleteContents" => {
                let r = *self.data.borrow();
                delete_range_contents(&self.dom, &r);
                Ok(Some(Value::Undefined))
            }
            "createContextualFragment" => {
                // `Range.createContextualFragment(html)` — mur
                // maps.google.com (vol 2026-10-26) : première
                // exception de la cascade `sc_*`. Le fragment est
                // parsé dans le même Dom que le range (contexte =
                // startContainer, notre parseur de fragment n'est
                // pas contextualisé — déviation documentée).
                let html = e.to_js_string(arg(args, 0))?;
                let frag = {
                    let mut d = self.dom.borrow_mut();
                    let frag = d.create_fragment();
                    d.parse_fragment_into(frag, &html);
                    frag
                };
                Ok(Some(node_value(e, &self.dom, &self.cache, frag)))
            }
            "insertNode" => {
                let Some(n) = node_for_in(e, arg(args, 0), &self.dom) else {
                    return Err(e.type_error("Range.insertNode: not a Node"));
                };
                let r = *self.data.borrow();
                let mut d = self.dom.borrow_mut();
                let ref_n = d
                    .child_nodes(r.start_node)
                    .get(r.start_off as usize)
                    .copied();
                d.insert_before(r.start_node, n, ref_n);
                Ok(Some(Value::Undefined))
            }
            // — Comparaisons de boundary points (JS-C556) :
            // `cmp_point` donne l'ordre document — approximation
            // `(node,off)` bornée, documentée avec le modèle.
            "compareBoundaryPoints" => {
                // `how` : 0 START_TO_START, 1 START_TO_END,
                // 2 END_TO_END, 3 END_TO_START — la borne
                // correspondante de `this` est comparée à celle
                // du range source (`-1` avant / `0` égal / `+1`
                // après). Constantes posées sur ctor + proto à
                // l'install (spec).
                let how = e.to_number(arg(args, 0)).unwrap_or(f64::NAN);
                if !(0.0..=3.0).contains(&how) {
                    return Err(crate::heap::Thrown(crate::dom_host::dom_ex_pub(
                        e,
                        "NotSupportedError",
                        "compareBoundaryPoints: how",
                    )));
                }
                let (Some(sh), Some(se)) = ({
                    let mut found = (None, None);
                    if let Value::Obj(id) = arg(args, 1) {
                        if let Some(h) = e.host_of(id) {
                            if let Some(rh) = h.as_any().downcast_ref::<RangeHost>() {
                                let r = *rh.data.borrow();
                                found = (
                                    Some((r.start_node, r.start_off)),
                                    Some((r.end_node, r.end_off)),
                                );
                            } else if let Some(sh) = h.as_any().downcast_ref::<StaticRangeHost>() {
                                found = (
                                    Some((sh.r.start_node, sh.r.start_off)),
                                    Some((sh.r.end_node, sh.r.end_off)),
                                );
                            }
                        }
                    }
                    found
                }) else {
                    return Err(crate::heap::Thrown(crate::dom_host::dom_ex_pub(
                        e,
                        "NotSupportedError",
                        "compareBoundaryPoints: not a Range",
                    )));
                };
                let r = *self.data.borrow();
                let (mine, theirs) = match how as u32 {
                    0 => ((r.start_node, r.start_off), sh),
                    1 => ((r.end_node, r.end_off), sh),
                    2 => ((r.end_node, r.end_off), se),
                    _ => ((r.start_node, r.start_off), se),
                };
                Ok(Some(Value::Num(f64::from(cmp_point(
                    &self.dom, mine, theirs,
                )))))
            }
            "comparePoint" => {
                let Some(n) = node_for_in(e, arg(args, 0), &self.dom) else {
                    return Err(e.type_error("Range.comparePoint: not a Node"));
                };
                let off = e.to_number(arg(args, 1)).unwrap_or(0.0).max(0.0) as u32;
                let r = *self.data.borrow();
                let s = cmp_point(&self.dom, (n, off), (r.start_node, r.start_off));
                let en = cmp_point(&self.dom, (n, off), (r.end_node, r.end_off));
                Ok(Some(Value::Num(if s < 0 {
                    -1.0
                } else if en > 0 {
                    1.0
                } else {
                    0.0
                })))
            }
            "isPointInRange" => {
                let Some(n) = node_for_in(e, arg(args, 0), &self.dom) else {
                    return Err(e.type_error("Range.isPointInRange: not a Node"));
                };
                let off = e.to_number(arg(args, 1)).unwrap_or(0.0).max(0.0) as u32;
                let r = *self.data.borrow();
                let s = cmp_point(&self.dom, (n, off), (r.start_node, r.start_off));
                let en = cmp_point(&self.dom, (n, off), (r.end_node, r.end_off));
                Ok(Some(Value::Bool(s >= 0 && en <= 0)))
            }
            "intersectsNode" => {
                // Spec : parent absent → `false` ; sinon le nœud
                // intersecte ssi `(parent, i+1)` est après le début
                // ET `(parent, i)` est avant la fin.
                let Some(n) = node_for_in(e, arg(args, 0), &self.dom) else {
                    return Err(e.type_error("Range.intersectsNode: not a Node"));
                };
                let hit = match node_index(&self.dom, n) {
                    None => false,
                    Some((p, i)) => {
                        let r = *self.data.borrow();
                        cmp_point(&self.dom, (p, i + 1), (r.start_node, r.start_off)) > 0
                            && cmp_point(&self.dom, (p, i), (r.end_node, r.end_off)) < 0
                    }
                };
                Ok(Some(Value::Bool(hit)))
            }
            "surroundContents" => {
                // Spec bornée : `newParent` élément/fragment
                // (InvalidNodeTypeError sinon) ; le contenu couvert
                // est déplacé dedans (clone+delete — approximation
                // d'`extractContents`, identités perdues), ses
                // enfants remplacés, le parent inséré à la borne de
                // début puis sélectionné. La détection « nœud non
                // texte partiellement contenu » (BadStateError)
                // n'est pas portée — même tolérance documentée que
                // `extractContents`.
                let Some(np) = node_for_in(e, arg(args, 0), &self.dom) else {
                    return Err(e.type_error("Range.surroundContents: not a Node"));
                };
                {
                    let d = self.dom.borrow();
                    if !matches!(
                        d.nodes.get(np as usize).map(|n| &n.kind),
                        Some(crate::dom::NodeKind::Element | crate::dom::NodeKind::Fragment)
                    ) {
                        return Err(crate::heap::Thrown(crate::dom_host::dom_ex_pub(
                            e,
                            "InvalidNodeTypeError",
                            "surroundContents: newParent",
                        )));
                    }
                }
                let r = *self.data.borrow();
                // 1. Contenu couvert → fragment (clone) ; 2.
                //    suppression des originaux (≡ extractContents).
                let covered = range_covered_top(&self.dom, &r);
                let frag = {
                    let mut d = self.dom.borrow_mut();
                    let frag = d.create_fragment();
                    for &n in &covered {
                        let c = d.clone_node(n, true);
                        d.insert_before(frag, c, None);
                    }
                    frag
                };
                delete_range_contents(&self.dom, &r);
                {
                    let mut d = self.dom.borrow_mut();
                    // Remplace les enfants du nouveau parent, puis
                    // insère le parent au point de début.
                    for c in d.child_nodes(np) {
                        d.detach(c);
                    }
                    d.insert_before(np, frag, None);
                    let ref_n = d
                        .child_nodes(r.start_node)
                        .get(r.start_off as usize)
                        .copied();
                    d.insert_before(r.start_node, np, ref_n);
                }
                // selectNode(np) — le range épouse le parent posé.
                if let Some((p, i)) = node_index(&self.dom, np) {
                    let mut d = self.data.borrow_mut();
                    d.start_node = p;
                    d.start_off = i;
                    d.end_node = p;
                    d.end_off = i + 1;
                }
                Ok(Some(Value::Undefined))
            }
            "cloneRange" => Ok(Some(range_value(e, *self.data.borrow()))),
            "detach" => Ok(Some(Value::Undefined)),
            "toString" => {
                let text = sel_range_text(&self.dom, &self.data.borrow());
                Ok(Some(e.str_val(text)))
            }
            // Borné : rect = celui de l'ancêtre commun (la
            // géométrie par sous-nœud couvert n'est pas
            // agrégée côté map i#19 — documenté).
            "getBoundingClientRect" | "getClientRects" => {
                let anc = {
                    let d = self.data.borrow();
                    common_ancestor(&self.dom, d.start_node, d.end_node)
                };
                let rect = crate::dom_host::gbrect_shared(e, &self.dom, anc);
                if name == "getBoundingClientRect" {
                    Ok(Some(rect))
                } else {
                    Ok(Some(e.new_array(vec![rect])))
                }
            }
            _ => Ok(None),
        }
    }
}

// ===== JS-C556 — `StaticRange` : `AbstractRange` gelé (bornes
// lues à la construction, jamais mutées — spec DOM).

/// Host `StaticRange` — bornes figées dans `r` (pas de `RefCell`
/// : aucun setter n'existe).
struct StaticRangeHost {
    r: SelRange,
    dom: crate::dom_host::DomRef,
    cache: crate::dom_host::NodeCache,
}

/// `StaticRange.prototype` paresseux — chaîne `__proto__` vers
/// `AbstractRange.prototype` (spec : `s instanceof AbstractRange`).
/// `pub(crate)` : adopté par le ctor à l'install (`instanceof`).
pub(crate) fn static_range_proto(e: &mut Engine) -> ObjId {
    if let Some(p) = e.misc.static_range_proto {
        return p;
    }
    let p = e.new_object();
    let tag = e.str_val("StaticRange");
    e.set_own_attrs(
        p,
        crate::interp::SYM_TO_STRING_TAG,
        tag,
        crate::heap::PropAttrs {
            writable: false,
            enumerable: false,
            configurable: true,
        },
    );
    let ar = Value::Obj(abstract_range_proto(e));
    if let Some(o) = e.heap.obj_mut(p) {
        o.set_proto(ar);
    }
    e.misc.static_range_proto = Some(p);
    p
}

/// `new StaticRange({startContainer, startOffset?, endContainer,
/// endOffset?})` — spec : `init` absent ou bornes non-Node →
/// `TypeError` (« required member »). Le range est gelé : les
/// mutateurs sont absents de la surface (déviation documentée :
/// la spec les expose en *throwing* `InvalidStateError` — ici
/// `s.setStart` est `undefined`, jamais une mutation silencieuse).
pub(crate) fn static_range_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let init = arg(args, 0);
    let Value::Obj(id) = init else {
        return Err(e.type_error("StaticRange: required member startContainer is undefined"));
    };
    let Some((dom, cache)) = e.dom_ctx.clone() else {
        return Err(e.type_error("StaticRange: no document"));
    };
    let sc = e.get_prop(id, "startContainer");
    let ec = e.get_prop(id, "endContainer");
    let so_v = e.get_prop(id, "startOffset");
    let eo_v = e.get_prop(id, "endOffset");
    let (Some(sn), Some(en)) = (
        sc.and_then(|v| node_for_in(e, v, &dom)),
        ec.and_then(|v| node_for_in(e, v, &dom)),
    ) else {
        return Err(e.type_error("StaticRange: startContainer/endContainer must be Nodes"));
    };
    let so = so_v
        .and_then(|v| e.to_number(v).ok())
        .unwrap_or(0.0)
        .max(0.0) as u32;
    let eo = eo_v
        .and_then(|v| e.to_number(v).ok())
        .unwrap_or(0.0)
        .max(0.0) as u32;
    let v = e.new_host_object(Rc::new(StaticRangeHost {
        r: SelRange {
            start_node: sn,
            start_off: so,
            end_node: en,
            end_off: eo,
        },
        dom,
        cache,
    }));
    let p = static_range_proto(e);
    if let Value::Obj(oid) = v {
        if let Some(o) = e.heap.obj_mut(oid) {
            o.set_proto(Value::Obj(p));
        }
    }
    Ok(v)
}

impl Host for StaticRangeHost {
    fn class_name(&self) -> &'static str {
        "StaticRange"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn trace(&self, out: &mut Vec<ObjId>) {
        crate::dom_host::trace_node_cache(&self.cache, out);
    }

    fn get(&self, e: &mut Engine, _t: Value, key: &str) -> JsResult<Option<Value>> {
        let r = &self.r;
        Ok(Some(match key {
            "startContainer" => node_value(e, &self.dom, &self.cache, r.start_node),
            "endContainer" => node_value(e, &self.dom, &self.cache, r.end_node),
            "startOffset" => Value::Num(f64::from(r.start_off)),
            "endOffset" => Value::Num(f64::from(r.end_off)),
            "collapsed" => Value::Bool(r.start_node == r.end_node && r.start_off == r.end_off),
            _ => return Ok(None),
        }))
    }
}

// ===== TreeWalker (CAP-182) — traversée filtrée du miroir.

/// Données `TreeWalker` partagées.
struct TwData {
    root: NodeId,
    /// Masque `NodeFilter.SHOW_*`.
    what: u32,
    /// Callback `filter` JS (`Undefined`/`Null` = absent).
    filter: Value,
    /// `currentNode` spec.
    cur: NodeId,
}

struct TreeWalkerHost {
    data: Rc<RefCell<TwData>>,
    dom: crate::dom_host::DomRef,
    cache: crate::dom_host::NodeCache,
}

/// Masque `SHOW_*` d'un nœud miroir.
fn tw_show_bit(d: &crate::dom::Dom, n: NodeId) -> u32 {
    match d.nodes[n as usize].kind {
        crate::dom::NodeKind::Text => 0x4,
        crate::dom::NodeKind::Comment => 0x80,
        crate::dom::NodeKind::Fragment => 0x400,
        crate::dom::NodeKind::Element => {
            let t = &d.nodes[n as usize].tag;
            if t == "#document" {
                0x100
            } else if t == "#comment" {
                0x80
            } else {
                0x1
            }
        }
    }
}

/// Verdict 0=accept / 1=skip / 2=reject — masque `SHOW_*`
/// d'abord, callback ensuite (`FILTER_ACCEPT`=1 retourne
/// `true`-like dans les scripts réels : on accepte tout
/// verdict truthy≠2/3 — borné, documenté).
fn tw_accept(
    e: &mut Engine,
    dom: &crate::dom_host::DomRef,
    cache: &crate::dom_host::NodeCache,
    n: NodeId,
    what: u32,
    filter: Value,
) -> u8 {
    if tw_show_bit(&dom.borrow(), n) & what == 0 {
        return 1;
    }
    match filter {
        Value::Undefined | Value::Null => 0,
        _ => {
            let nv = node_value(e, dom, cache, n);
            match e
                .call_value(filter, Value::Undefined, &[nv])
                .ok()
                .and_then(|v| e.to_number(v).ok())
            {
                Some(r) if (r - 2.0).abs() < f64::EPSILON => 2,
                Some(r) if (r - 3.0).abs() < f64::EPSILON => 1,
                Some(r) if r != 0.0 => 0,
                _ => 1,
            }
        }
    }
}

/// Frère suivant de `n`, ou — remontée — frère suivant
/// d'un ancêtre, borné à `root` (exclue — le walker ne
/// sort jamais de son arbre).
fn tw_step_out(dom: &crate::dom_host::DomRef, mut n: NodeId, root: NodeId) -> Option<NodeId> {
    while n != root {
        let d = dom.borrow();
        let p = d.parent(n)?;
        let kids = d.child_nodes(p);
        if let Some(&s) = kids
            .iter()
            .position(|&c| c == n)
            .and_then(|i| kids.get(i + 1))
        {
            return Some(s);
        }
        n = p;
    }
    None
}

/// Prochain nœud accepté en ordre document borné à `root`
/// (skip → descend quand même ; reject → saute le sous-arbre).
fn tw_next(
    e: &mut Engine,
    dom: &crate::dom_host::DomRef,
    cache: &crate::dom_host::NodeCache,
    what: u32,
    filter: Value,
    root: NodeId,
    from: NodeId,
) -> Option<NodeId> {
    let mut n = dom
        .borrow()
        .child_nodes(from)
        .into_iter()
        .next()
        .or_else(|| tw_step_out(dom, from, root))?;
    loop {
        match tw_accept(e, dom, cache, n, what, filter.clone()) {
            0 => return Some(n),
            1 => {
                n = dom
                    .borrow()
                    .child_nodes(n)
                    .into_iter()
                    .next()
                    .or_else(|| tw_step_out(dom, n, root))?;
            }
            _ => {
                n = tw_step_out(dom, n, root)?;
            }
        }
    }
}

/// Frère de `n` du côté `forward` (`false` = précédent).
fn tw_sib(dom: &crate::dom_host::DomRef, n: NodeId, forward: bool) -> Option<NodeId> {
    dom.borrow().sibling(n, forward)
}

/// Enfant extrême de `n` du côté `forward` (first / last).
fn tw_child_at(dom: &crate::dom_host::DomRef, n: NodeId, forward: bool) -> Option<NodeId> {
    let d = dom.borrow();
    let kids = d.child_nodes(n);
    if forward { kids.first() } else { kids.last() }.copied()
}

/// Spec "traverse children" — `firstChild()`/`lastChild()` :
/// descend depuis `cur`, `skip` → descente du même côté,
/// `reject` → frère/remontée bornée à `cur` (le receveur) ou
/// `root` — jamais au-dessus (JS-C340 : la borne était `root`,
/// un `firstChild()` receveur vide sautait hors du sous-arbre).
#[allow(clippy::too_many_arguments)]
fn tw_children(
    e: &mut Engine,
    dom: &crate::dom_host::DomRef,
    cache: &crate::dom_host::NodeCache,
    what: u32,
    filter: Value,
    root: NodeId,
    cur: NodeId,
    forward: bool,
) -> Option<NodeId> {
    let mut node = tw_child_at(dom, cur, forward)?;
    loop {
        match tw_accept(e, dom, cache, node, what, filter.clone()) {
            0 => return Some(node),
            1 => {
                if let Some(c) = tw_child_at(dom, node, forward) {
                    node = c;
                    continue;
                }
            }
            _ => {}
        }
        // reject, ou skip sans enfant : frère du même côté,
        // sinon remontée bornée à `cur`/`root` (exclus).
        loop {
            match tw_sib(dom, node, forward) {
                Some(s) => {
                    node = s;
                    break;
                }
                None => match dom.borrow().parent(node) {
                    Some(p) if p != cur && p != root => node = p,
                    _ => return None,
                },
            }
        }
    }
}

/// Spec "traverse siblings" — `nextSibling()`/`previousSibling()` :
/// né de `cur`, jamais au-dessus de `root` ; un frère non
/// accepté (skip **ou** reject — la spec ne distingue pas ici)
/// descend son enfant du même côté avant de tenter le frère
/// suivant (JS-C340).
#[allow(clippy::too_many_arguments)]
fn tw_siblings(
    e: &mut Engine,
    dom: &crate::dom_host::DomRef,
    cache: &crate::dom_host::NodeCache,
    what: u32,
    filter: Value,
    root: NodeId,
    cur: NodeId,
    forward: bool,
) -> Option<NodeId> {
    if cur == root {
        return None;
    }
    let mut node = cur;
    loop {
        let mut sib = tw_sib(dom, node, forward);
        while let Some(s) = sib {
            node = s;
            if tw_accept(e, dom, cache, node, what, filter.clone()) == 0 {
                return Some(node);
            }
            sib = tw_child_at(dom, node, forward).or_else(|| tw_sib(dom, node, forward));
        }
        match dom.borrow().parent(node) {
            Some(p) if p != root => node = p,
            _ => return None,
        }
    }
}

/// Spec `parentNode()` — premier ancêtre accepté, `root`
/// incluse (elle peut être retournée si le filtre l'accepte).
fn tw_parent(
    e: &mut Engine,
    dom: &crate::dom_host::DomRef,
    cache: &crate::dom_host::NodeCache,
    what: u32,
    filter: Value,
    root: NodeId,
    cur: NodeId,
) -> Option<NodeId> {
    let mut node = cur;
    while node != root {
        node = dom.borrow().parent(node)?;
        if tw_accept(e, dom, cache, node, what, filter.clone()) == 0 {
            return Some(node);
        }
    }
    None
}

/// Spec `previousNode()` — reverse ordre document : depuis
/// `cur`, frère précédent → descente dernier enfant (skip),
/// `reject` élague le sous-arbre, parent filtré à la remontée
/// (`root` retournable si acceptée), borné à `root` (JS-C340).
fn tw_prev_node(
    e: &mut Engine,
    dom: &crate::dom_host::DomRef,
    cache: &crate::dom_host::NodeCache,
    what: u32,
    filter: Value,
    root: NodeId,
    cur: NodeId,
) -> Option<NodeId> {
    let mut node = cur;
    while node != root {
        let mut sib = tw_sib(dom, node, false);
        while let Some(s) = sib {
            node = s;
            match tw_accept(e, dom, cache, node, what, filter.clone()) {
                0 => return Some(node),
                1 => {
                    sib = tw_child_at(dom, node, false).or_else(|| tw_sib(dom, node, false));
                }
                _ => sib = tw_sib(dom, node, false),
            }
        }
        node = dom.borrow().parent(node)?;
        if tw_accept(e, dom, cache, node, what, filter.clone()) == 0 {
            return Some(node);
        }
    }
    None
}

/// `TreeWalker.prototype` paresseux.
fn tw_proto(e: &mut Engine) -> ObjId {
    if let Some(p) = e.misc.tw_proto {
        return p;
    }
    let p = e.new_object();
    let tag = e.str_val("TreeWalker");
    e.set_own_attrs(
        p,
        crate::interp::SYM_TO_STRING_TAG,
        tag,
        crate::heap::PropAttrs {
            writable: false,
            enumerable: false,
            configurable: true,
        },
    );
    e.misc.tw_proto = Some(p);
    p
}

/// `document.createTreeWalker(root, whatToShow?, filter?)` —
/// `new TreeWalker` reste non-constructible (spec).
pub(crate) fn create_treewalker(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let Some((dom, cache)) = e.dom_ctx.clone() else {
        return Err(e.type_error("createTreeWalker: no document"));
    };
    let root = node_for_in(e, arg(args, 0), &dom).unwrap_or_else(|| dom.borrow().root);
    let what = e.to_number(arg(args, 1)).unwrap_or(f64::from(u32::MAX)) as u32;
    let filter = arg(args, 2);
    let v = e.new_host_object(Rc::new(TreeWalkerHost {
        data: Rc::new(RefCell::new(TwData {
            root,
            what,
            filter,
            cur: root,
        })),
        dom,
        cache,
    }));
    let p = tw_proto(e);
    if let Value::Obj(id) = v {
        if let Some(o) = e.heap.obj_mut(id) {
            o.set_proto(Value::Obj(p));
        }
    }
    Ok(v)
}

impl Host for TreeWalkerHost {
    fn class_name(&self) -> &'static str {
        "TreeWalker"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn trace(&self, out: &mut Vec<ObjId>) {
        crate::dom_host::trace_node_cache(&self.cache, out);
        crate::heap::push_value(out, &self.data.borrow().filter);
    }

    fn has_method(&self, name: &str) -> bool {
        matches!(
            name,
            "nextNode"
                | "previousNode"
                | "firstChild"
                | "lastChild"
                | "nextSibling"
                | "previousSibling"
                | "parentNode"
        )
    }

    fn get(&self, e: &mut Engine, _t: Value, key: &str) -> JsResult<Option<Value>> {
        let d = self.data.borrow();
        Ok(Some(match key {
            "root" => node_value(e, &self.dom, &self.cache, d.root),
            "whatToShow" => Value::Num(f64::from(d.what)),
            "filter" => d.filter.clone(),
            "currentNode" => node_value(e, &self.dom, &self.cache, d.cur),
            _ => return Ok(None),
        }))
    }

    fn set(&self, e: &mut Engine, _t: Value, key: &str, v: Value) -> JsResult<bool> {
        if key == "currentNode" {
            if let Some(n) = node_for_in(e, v, &self.dom) {
                self.data.borrow_mut().cur = n;
            }
            return Ok(true);
        }
        Ok(false)
    }

    fn invoke(
        &self,
        e: &mut Engine,
        _t: Value,
        name: &str,
        args: &[Value],
    ) -> JsResult<Option<Value>> {
        let _ = args;
        let (root, what, filter, cur) = {
            let d = self.data.borrow();
            (d.root, d.what, d.filter.clone(), d.cur)
        };
        // Toutes les traversées sont relatives à `currentNode`
        // (JS-C340 — §B-97 : ShadyDOM repositionne `currentNode`
        // puis appelle `firstChild`/`nextSibling` ; partir de
        // `root` ou rendre `null` cassait le montage Polymer).
        let found = match name {
            "nextNode" => tw_next(e, &self.dom, &self.cache, what, filter, root, cur),
            "previousNode" => tw_prev_node(e, &self.dom, &self.cache, what, filter, root, cur),
            "firstChild" => tw_children(e, &self.dom, &self.cache, what, filter, root, cur, true),
            "lastChild" => tw_children(e, &self.dom, &self.cache, what, filter, root, cur, false),
            "nextSibling" => tw_siblings(e, &self.dom, &self.cache, what, filter, root, cur, true),
            "previousSibling" => {
                tw_siblings(e, &self.dom, &self.cache, what, filter, root, cur, false)
            }
            "parentNode" => tw_parent(e, &self.dom, &self.cache, what, filter, root, cur),
            _ => return Ok(Some(Value::Null)),
        };
        match found {
            Some(n) => {
                self.data.borrow_mut().cur = n;
                Ok(Some(node_value(e, &self.dom, &self.cache, n)))
            }
            None => Ok(Some(Value::Null)),
        }
    }
}

/// `document.execCommand(cmd, ui?, value?)` (CAP-182 —
/// hérité mais omniprésent : `copy`/`cut` via le hook
/// presse-papiers CAP-181 sur le texte de `Selection`,
/// `selectAll` pose un range `body`-entier ; le reste →
/// `false` honnête — spec : commande inconnue = `false`).
pub(crate) fn exec_command(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let cmd = e.to_js_string(arg(args, 0))?;
    match cmd.as_str() {
        "copy" | "cut" => {
            let Some(hook) = e.hooks.clipboard_write.clone() else {
                return Ok(Value::Bool(false));
            };
            let text = {
                let Some(d) = &e.misc.sel else {
                    return Ok(Value::Bool(false));
                };
                let Some((dom, _c)) = e.dom_ctx.clone() else {
                    return Ok(Value::Bool(false));
                };
                d.borrow()
                    .ranges
                    .iter()
                    .map(|r| sel_range_text(&dom, &r.borrow()))
                    .collect::<String>()
            };
            Ok(Value::Bool(hook(text).is_ok()))
        }
        "selectAll" => {
            let Some((dom, _c)) = e.dom_ctx.clone() else {
                return Ok(Value::Bool(false));
            };
            let d = sel_data(e);
            let (body, count) = {
                let dm = dom.borrow();
                let b = dm.body();
                (b, dm.child_nodes(b).len() as u32)
            };
            d.borrow_mut().ranges = vec![Rc::new(RefCell::new(SelRange {
                start_node: body,
                start_off: 0,
                end_node: body,
                end_off: count,
            }))];
            Ok(Value::Bool(true))
        }
        _ => Ok(Value::Bool(false)),
    }
}

// ===== CAP-183 — NodeIterator / CSSStyleSheet / SharedWorker —

/// `NodeIterator` — itérateur document-order plat (spec :
/// premier `nextNode` **inclut** la racine si acceptée —
/// contrairement au `TreeWalker` qui part d'un niveau plus
/// bas). `new NodeIterator` reste non-constructible (spec).
struct NiHost {
    data: Rc<RefCell<NiData>>,
    dom: crate::dom_host::DomRef,
    cache: crate::dom_host::NodeCache,
}

struct NiData {
    root: NodeId,
    what: u32,
    filter: Value,
    /// `cur` courant + `started` (le premier `nextNode`
    /// part de la racine incluse).
    cur: NodeId,
    started: bool,
}

/// Prédécesseur en ordre document borné à `root` — dernier
/// descendant du frère précédent, sinon le parent (accept
/// décidé par l'appelant ; `reject`/`skip` bouclent).
fn tw_prev(tw_dom: &crate::dom_host::DomRef, n: NodeId, root: NodeId) -> Option<NodeId> {
    if n == root {
        return None;
    }
    let d = tw_dom.borrow();
    let p = d.parent(n)?;
    let kids = d.child_nodes(p);
    if let Some(i) = kids.iter().position(|&c| c == n) {
        if i > 0 {
            let mut m = kids[i - 1];
            while let Some(c) = d.child_nodes(m).last().copied() {
                m = c;
            }
            return Some(m);
        }
    }
    Some(p)
}

/// `NodeIterator.prototype` paresseux.
fn ni_proto(e: &mut Engine) -> ObjId {
    if let Some(p) = e.misc.ni_proto {
        return p;
    }
    let p = e.new_object();
    let tag = e.str_val("NodeIterator");
    e.set_own_attrs(
        p,
        crate::interp::SYM_TO_STRING_TAG,
        tag,
        crate::heap::PropAttrs {
            writable: false,
            enumerable: false,
            configurable: true,
        },
    );
    e.misc.ni_proto = Some(p);
    p
}

/// `document.createNodeIterator(root, whatToShow?, filter?)`
/// (CAP-183) — successeur/prédécesseur document-order via
/// `tw_next`/`tw_prev` ; filter/masque `NodeFilter` comme
/// `createTreeWalker`.
pub(crate) fn create_nodeiterator(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let Some((dom, cache)) = e.dom_ctx.clone() else {
        return Err(e.type_error("createNodeIterator: no document"));
    };
    let root = node_for_in(e, arg(args, 0), &dom).unwrap_or_else(|| dom.borrow().root);
    let what = e.to_number(arg(args, 1)).unwrap_or(f64::from(u32::MAX)) as u32;
    let filter = arg(args, 2);
    let v = e.new_host_object(Rc::new(NiHost {
        data: Rc::new(RefCell::new(NiData {
            root,
            what,
            filter,
            cur: root,
            started: false,
        })),
        dom,
        cache,
    }));
    let p = ni_proto(e);
    if let Value::Obj(id) = v {
        if let Some(o) = e.heap.obj_mut(id) {
            o.set_proto(Value::Obj(p));
        }
    }
    Ok(v)
}

impl Host for NiHost {
    fn class_name(&self) -> &'static str {
        "NodeIterator"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn trace(&self, out: &mut Vec<ObjId>) {
        crate::dom_host::trace_node_cache(&self.cache, out);
        crate::heap::push_value(out, &self.data.borrow().filter);
    }

    fn has_method(&self, name: &str) -> bool {
        matches!(
            name,
            "nextNode" | "previousNode" | "detach" | "referenceNode"
        )
    }

    fn get(&self, e: &mut Engine, _t: Value, key: &str) -> JsResult<Option<Value>> {
        let d = self.data.borrow();
        Ok(Some(match key {
            "root" => node_value(e, &self.dom, &self.cache, d.root),
            "whatToShow" => Value::Num(f64::from(d.what)),
            "filter" => d.filter.clone(),
            "referenceNode" => node_value(e, &self.dom, &self.cache, d.cur),
            "pointerBeforeReferenceNode" => Value::Bool(false),
            _ => return Ok(None),
        }))
    }

    fn invoke(
        &self,
        e: &mut Engine,
        _t: Value,
        name: &str,
        args: &[Value],
    ) -> JsResult<Option<Value>> {
        let _ = args;
        match name {
            "nextNode" => {
                let (root, what, filter, cur, started) = {
                    let d = self.data.borrow();
                    (d.root, d.what, d.filter.clone(), d.cur, d.started)
                };
                let found = if started {
                    tw_next(e, &self.dom, &self.cache, what, filter, root, cur)
                } else {
                    self.data.borrow_mut().started = true;
                    if tw_accept(e, &self.dom, &self.cache, root, what, filter.clone()) == 0 {
                        Some(root)
                    } else {
                        tw_next(e, &self.dom, &self.cache, what, filter, root, root)
                    }
                };
                match found {
                    Some(n) => {
                        self.data.borrow_mut().cur = n;
                        Ok(Some(node_value(e, &self.dom, &self.cache, n)))
                    }
                    None => Ok(Some(Value::Null)),
                }
            }
            "previousNode" => {
                let (root, what, filter, mut n) = {
                    let d = self.data.borrow();
                    (d.root, d.what, d.filter.clone(), d.cur)
                };
                loop {
                    let Some(p) = tw_prev(&self.dom, n, root) else {
                        return Ok(Some(Value::Null));
                    };
                    n = p;
                    if tw_accept(e, &self.dom, &self.cache, n, what, filter.clone()) == 0 {
                        self.data.borrow_mut().cur = n;
                        return Ok(Some(node_value(e, &self.dom, &self.cache, n)));
                    }
                }
            }
            "detach" => Ok(Some(Value::Undefined)),
            "referenceNode" => Ok(Some(node_value(
                e,
                &self.dom,
                &self.cache,
                self.data.borrow().cur,
            ))),
            _ => Ok(None),
        }
    }
}

// ===== CAP-183 (suite) — `CSSStyleSheet` / `SharedWorker` —

/// `new CSSStyleSheet(options?)` (CAP-183) — feuille
/// **constructible** spec : objet miroir `{type:"text/css",
/// href:null, disabled:false, cssRules:[], media:{mediaText:""}}`
/// + `insertRule`/`deleteRule`/`replace`/`replaceSync`.
///
/// Borné : les règles sont stockées telles quelles
/// (`{cssText, type:1}`) — le parseur CSS vit côté
/// `explorer-css`, la cascade/`adoptedStyleSheets` est un
/// chantier session (documenté).
pub(crate) fn css_sheet_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let o = e.new_object();
    let t = e.str_val("text/css");
    e.set_own(o, "type", t, true);
    e.set_own(o, "href", Value::Null, true);
    e.set_own(o, "disabled", Value::Bool(false), true);
    let rules = e.new_array(Vec::new());
    e.set_own(o, "cssRules", rules, true);
    let media = e.new_object();
    let mt = e.str_val("");
    e.set_own(media, "mediaText", mt, true);
    e.set_own(o, "media", Value::Obj(media), true);
    let tag = e.str_val("CSSStyleSheet");
    e.set_own_attrs(
        o,
        crate::interp::SYM_TO_STRING_TAG,
        tag,
        crate::heap::PropAttrs {
            writable: false,
            enumerable: false,
            configurable: true,
        },
    );
    // options `{baseURL, media, disabled}` — `disabled` est
    // le seul honnêtement stocké ici (borné).
    if let Value::Obj(opts) = arg(args, 0) {
        let dis = e.get_prop(opts, "disabled");
        if matches!(dis, Some(Value::Bool(b)) if b) {
            e.set_own(o, "disabled", Value::Bool(true), true);
        }
    }
    e.add_method(o, "insertRule", sheet_insert_rule);
    e.add_method(o, "deleteRule", sheet_delete_rule);
    e.add_method(o, "replace", sheet_replace);
    e.add_method(o, "replaceSync", sheet_replace_sync);
    // `proto = CSSStyleSheet.prototype` (spec + `instanceof`) —
    // le proto porte les méthodes (feature-detect
    // `"replaceSync" in CSSStyleSheet.prototype`, catalyst
    // github) ; les own-methods ci-dessus restent (double
    // rail, lecture directe `sh.insertRule` comme référence).
    if let Ok(Value::Obj(cid)) = e.get_prop_value(Value::Obj(e.global), "CSSStyleSheet") {
        if let Ok(proto @ Value::Obj(_)) = e.get_prop_value(Value::Obj(cid), "prototype") {
            if let Some(obj) = e.heap.obj_mut(o) {
                obj.set_proto(proto);
            }
        }
    }
    Ok(Value::Obj(o))
}

/// `cssRules` de `this` (`ObjId` attendu).
fn sheet_rules(e: &mut Engine, this: Value) -> Option<ObjId> {
    let Value::Obj(id) = this else { return None };
    match e.get_prop(id, "cssRules") {
        Some(Value::Obj(r)) => Some(r),
        _ => None,
    }
}

/// [`sheet_rules`] exposée au module `cssom` (peuplement
/// d'une feuille miroir depuis son texte).
pub(crate) fn sheet_rules_obj(e: &mut Engine, this: Value) -> Option<ObjId> {
    sheet_rules(e, this)
}

/// `insertRule(rule, index?)` → index spec (borné : la règle
/// est stockée `{cssText,type:1}`, pas cascadée).
pub(crate) fn sheet_insert_rule(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let text = e.to_js_string(arg(args, 0))?;
    // Spec : index absent → insertion en fin (`cssRules.length`),
    // pas 0 — l'ordre de cascade dépend de l'ordre d'insertion.
    // La valeur retournée est l'index d'insertion RÉEL (clampé).
    let want = if matches!(arg(args, 1), Value::Undefined) {
        usize::MAX
    } else {
        e.to_number(arg(args, 1)).unwrap_or(0.0).max(0.0) as usize
    };
    let Some(rid) = sheet_rules(e, this.clone()) else {
        return Err(e.type_error("insertRule: no cssRules"));
    };
    let rule = e.new_object();
    let c = e.str_val(&text);
    e.set_own(rule, "cssText", c, true);
    e.set_own(rule, "type", Value::Num(1.0), true);
    // `selectorText` = prélude avant `{` — même shape que les
    // règles peuplées par `cssom::populate_sheet_rules`.
    if let Some(pre) = text
        .split('{')
        .next()
        .map(str::trim)
        .filter(|p| !p.is_empty())
    {
        let sp = e.str_val(pre);
        e.set_own(rule, "selectorText", sp, true);
    }
    let Some(o) = e.heap.obj_mut(rid) else {
        return Err(e.type_error("insertRule: detached"));
    };
    let at = {
        if let crate::heap::ObjKind::Array(elems) = &mut o.kind {
            let at = want.min(elems.len() as usize);
            elems.insert(at, Value::Obj(rule));
            at
        } else {
            0
        }
    };
    reflect_sheet_to_style(e, this, None);
    Ok(Value::Num(at as f64))
}

/// §B-152 — transport cascade vers le `<style>` miroir :
/// `insertRule`/`deleteRule`/`replace*` ne touchent pas le
/// `textContent` de l'élément chez un navigateur réel (CSSOM
/// distinct du DOM) — mais notre pipeline de style se nourrit
/// du texte sérialisé de la feuille. On le régénère depuis
/// `cssRules[*].cssText` (l'ordre des index est préservé,
/// `deleteRule` couvert par la même régénération) ; `text`
/// sert pour `replace*` dont les règles sont vidées par spec.
/// No-op hors miroir DOM ou quand `ownerNode` n'est pas un
/// `<style>` (`<link>`/feuilles construites — celles adoptées
/// transitent déjà par `adopted_sheets_css`).
fn reflect_sheet_to_style(e: &mut Engine, this: Value, text: Option<&str>) {
    let Value::Obj(sid) = this else { return };
    let Ok(own) = e.get_prop_value(Value::Obj(sid), "ownerNode") else {
        return;
    };
    let Some(node) = crate::dom_host::node_for(e, own) else {
        return;
    };
    let Some((dom, _)) = e.dom_ctx.clone() else {
        return;
    };
    {
        let d = dom.borrow();
        if d.nodes.get(node as usize).map(|n| n.tag.as_str()) != Some("style") {
            return;
        }
    }
    let text = if let Some(t) = text {
        t.to_string()
    } else {
        let Some(rid) = sheet_rules(e, Value::Obj(sid)) else {
            return;
        };
        let rule_ids: Vec<ObjId> = match e.heap.obj(rid).map(|o| &o.kind) {
            Some(crate::heap::ObjKind::Array(elems)) => elems
                .stored_vals()
                .filter_map(|el| match el {
                    Value::Obj(id) => Some(*id),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        };
        let mut parts = Vec::new();
        for oid in rule_ids {
            if let Some(t) = e
                .get_prop_value(Value::Obj(oid), "cssText")
                .ok()
                .and_then(|v| e.to_js_string(v).ok())
            {
                parts.push(t);
            }
        }
        parts.join("\n")
    };
    let t = dom.borrow_mut().create_text(&text);
    dom.borrow_mut().replace_children(node, &[t]);
}

/// `deleteRule(index)` — borne `idx < len` sinon `IndexSizeError`.
pub(crate) fn sheet_delete_rule(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let idx = e.to_number(arg(args, 0))? as usize;
    let Some(rid) = sheet_rules(e, this.clone()) else {
        return Err(e.type_error("deleteRule: no cssRules"));
    };
    let Some(o) = e.heap.obj_mut(rid) else {
        return Err(e.type_error("deleteRule: detached"));
    };
    if let crate::heap::ObjKind::Array(elems) = &mut o.kind {
        if idx < elems.len() as usize {
            elems.remove_at(idx);
        }
    }
    reflect_sheet_to_style(e, this, None);
    Ok(Value::Undefined)
}

/// `replace(text)` → promesse résolue `this` (spec :
/// `CSSStyleSheet` — borné : texte stocké `__text`, règles
/// vidées ; pas de parse).
pub(crate) fn sheet_replace(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let text = e.to_js_string(arg(args, 0))?;
    sheet_set_text(e, this.clone(), &text)?;
    let pid = e.new_promise();
    e.resolve_promise(pid, this);
    Ok(Value::Obj(pid))
}

/// `replaceSync(text)` — même, synchrone.
pub(crate) fn sheet_replace_sync(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let text = e.to_js_string(arg(args, 0))?;
    sheet_set_text(e, this, &text)?;
    Ok(Value::Undefined)
}

/// `__text` posé + `cssRules` vidées (spec `replace*`).
fn sheet_set_text(e: &mut Engine, this: Value, text: &str) -> JsResult<()> {
    let Value::Obj(id) = this else {
        return Err(e.type_error("replace: not a sheet"));
    };
    let t = e.str_val(text);
    e.set_own(id, "__text", t, false);
    if let Some(rid) = sheet_rules(e, this.clone()) {
        if let Some(o) = e.heap.obj_mut(rid) {
            if let crate::heap::ObjKind::Array(elems) = &mut o.kind {
                elems.clear();
            }
        }
    }
    reflect_sheet_to_style(e, this, Some(text));
    Ok(())
}

/// `new SharedWorker(url)` (CAP-183) — gate honnête identique
/// à `Worker` : sans loader → `TypeError`. Avec → objet
/// `{port}` qui délègue `postMessage`/`onmessage` au worker
/// (borné : chaque contexte a le sien — le partage spec est
/// un chantier inter-contextes).
pub(crate) fn shared_worker_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    if e.worker_factory.is_none() {
        let spec = e.to_js_string(arg(args, 0))?;
        if spec.is_empty() {
            return Err(e.type_error("SharedWorker: empty script URL"));
        }
        return Err(e.type_error("SharedWorker: no loader installed"));
    }
    let host = crate::worker::worker_ctor_pub(e, Value::Undefined, args)?;
    let o = e.new_object();
    e.set_own(o, "port", host, true);
    // `sw instanceof SharedWorker` — la valeur chaîne
    // `SharedWorker.prototype` (spec) si le ctor global est
    // posé.
    let g = e.global;
    if let Some(Value::Obj(cid)) = e.get_prop(g, "SharedWorker") {
        if let Some(pp @ Value::Obj(_)) = e.get_prop(cid, "prototype") {
            if let Some(ob) = e.heap.obj_mut(o) {
                ob.set_proto(pp);
            }
        }
    }
    let tag = e.str_val("SharedWorker");
    e.set_own_attrs(
        o,
        crate::interp::SYM_TO_STRING_TAG,
        tag,
        crate::heap::PropAttrs {
            writable: false,
            enumerable: false,
            configurable: true,
        },
    );
    Ok(Value::Obj(o))
}

// ===== CAP-187 — Web Animations borné : `animate`, —

/// Données d'une `Animation` vivante — machine à états
/// honnête (pas de compositeur : la timeline n'interpole
/// pas les styles du miroir — borné, documenté).
struct AnimData {
    /// Élément cible (`nodeFor`-résolu à l'appel).
    node: NodeId,
    /// `playState` spec.
    state: &'static str,
    /// `playbackRate`.
    rate: f64,
    /// Promesses `finished`/`ready`.
    finished: ObjId,
    ready: ObjId,
    /// Wrapper `Animation` déjà rendu — `getAnimations`
    /// rend le même objet (identité spec).
    wrapper: Option<ObjId>,
    /// Handlers `onfinish`/`oncancel`/`onremove`.
    onfinish: Value,
    oncancel: Value,
    /// `onremove` (WAAPI `remove` event) — valeur du handler.
    onremove: Value,
    /// `id` spec (chaîne, mutable).
    id: String,
    /// `effect`/`timeline` (JS-C461) — `new Animation(kfe,
    /// tl)` les porte ; `el.animate` pose un `KeyframeEffect`
    /// implicite + `document.timeline`.
    effect: Value,
    timeline: Value,
    /// `animationName` (JS-C475) — `Some` pour une animation
    /// **CSS déclarative** remontée par la session via la
    /// marque `data-explorer-anim` du miroir ; `None` pour
    /// les `Animation` nées du script (`el.animate`, ctor).
    css_name: Option<String>,
    /// Classe `WebIDL` du wrapper (JS-C600, §B-165) —
    /// `"Animation"` née du script, `"CSSAnimation"` pour
    /// les animations CSS déclaratives (les filles n'ont
    /// pas de `[Constructor]` — surfaces globales +
    /// chaîne de protos seulement). Pilote le proto du
    /// wrapper (`instanceof`) et `class_name`
    /// (`toString`).
    class_tag: &'static str,
}

struct AnimHost {
    data: Rc<RefCell<AnimData>>,
}

/// `Animation.prototype` paresseux.
fn anim_proto(e: &mut Engine) -> ObjId {
    if let Some(p) = e.misc.anim_proto {
        return p;
    }
    let p = e.new_object();
    let tag = e.str_val("Animation");
    e.set_own_attrs(
        p,
        crate::interp::SYM_TO_STRING_TAG,
        tag,
        crate::heap::PropAttrs {
            writable: false,
            enumerable: false,
            configurable: true,
        },
    );
    e.misc.anim_proto = Some(p);
    p
}

/// `CSSAnimation.prototype`/`CSSTransition.prototype`
/// paresseux (JS-C600, §B-165) — proto = `Animation.
/// prototype` (chaîne `instanceof` spec : une
/// `CSSAnimation` EST une `Animation`).
fn css_anim_proto(e: &mut Engine) -> ObjId {
    if let Some(p) = e.misc.css_anim_proto {
        return p;
    }
    css_child_proto(e, "CSSAnimation", true)
}
fn css_trans_proto(e: &mut Engine) -> ObjId {
    if let Some(p) = e.misc.css_trans_proto {
        return p;
    }
    css_child_proto(e, "CSSTransition", false)
}
fn css_child_proto(e: &mut Engine, tag: &str, anim: bool) -> ObjId {
    let p = e.new_object();
    let tag = e.str_val(tag);
    e.set_own_attrs(
        p,
        crate::interp::SYM_TO_STRING_TAG,
        tag,
        crate::heap::PropAttrs {
            writable: false,
            enumerable: false,
            configurable: true,
        },
    );
    let parent = anim_proto(e);
    if let Some(o) = e.heap.obj_mut(p) {
        o.set_proto(Value::Obj(parent));
    }
    if anim {
        e.misc.css_anim_proto = Some(p);
    } else {
        e.misc.css_trans_proto = Some(p);
    }
    p
}

/// Wrapper `Animation` pour une donnée (créé une fois —
/// `getAnimations` rend le même objet).
fn anim_wrapper(e: &mut Engine, data: &Rc<RefCell<AnimData>>) -> Value {
    if let Some(w) = data.borrow().wrapper {
        return Value::Obj(w);
    }
    if e.dom_ctx.is_none() {
        return Value::Null;
    }
    let v = e.new_host_object(Rc::new(AnimHost { data: data.clone() }));
    let p = match data.borrow().class_tag {
        "CSSAnimation" => css_anim_proto(e),
        "CSSTransition" => css_trans_proto(e),
        _ => anim_proto(e),
    };
    if let Value::Obj(id) = v {
        if let Some(o) = e.heap.obj_mut(id) {
            o.set_proto(Value::Obj(p));
        }
        data.borrow_mut().wrapper = Some(id);
    }
    v
}

/// `KeyframeEffect.prototype` paresseux (JS-C461).
fn kfe_proto(e: &mut Engine) -> ObjId {
    if let Some(p) = e.misc.kfe_proto {
        return p;
    }
    let p = e.new_object();
    let tag = e.str_val("KeyframeEffect");
    e.set_own_attrs(
        p,
        crate::interp::SYM_TO_STRING_TAG,
        tag,
        crate::heap::PropAttrs {
            writable: false,
            enumerable: false,
            configurable: true,
        },
    );
    e.misc.kfe_proto = Some(p);
    p
}

/// `document.timeline` singleton (JS-C461) —
/// `DocumentTimeline` honnête : `currentTime`/`originTime`
/// = 0 (pas d'horloge compositeur dans le miroir).
/// `pub(crate)` : le getter `document.timeline` (`dom_host`)
/// sert le même singleton.
pub(crate) fn document_timeline_pub(e: &mut Engine) -> ObjId {
    document_timeline(e)
}

fn document_timeline(e: &mut Engine) -> ObjId {
    if let Some(t) = e.misc.doc_timeline {
        return t;
    }
    let t = e.new_object();
    let tag = e.str_val("DocumentTimeline");
    e.set_own_attrs(
        t,
        crate::interp::SYM_TO_STRING_TAG,
        tag,
        crate::heap::PropAttrs {
            writable: false,
            enumerable: false,
            configurable: true,
        },
    );
    e.set_own(t, "currentTime", Value::Num(0.0), false);
    e.set_own(t, "originTime", Value::Num(0.0), false);
    e.misc.doc_timeline = Some(t);
    t
}

/// Host `KeyframeEffect` (JS-C461, mur vol 733 §B-124 —
/// `YouTube` fait `new KeyframeEffect(el, frames, opts)`) :
/// stocke cible/frames/options, lecture honnête, zéro
/// interpolation (pas de compositeur — même borne que
/// `el.animate` CAP-187).
struct KfeHost {
    target: Value,
    keyframes: Value,
    options: Value,
}

impl Host for KfeHost {
    fn class_name(&self) -> &'static str {
        "KeyframeEffect"
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn trace(&self, out: &mut Vec<ObjId>) {
        crate::heap::push_value(out, &self.target);
        crate::heap::push_value(out, &self.keyframes);
        crate::heap::push_value(out, &self.options);
    }
    fn has_method(&self, name: &str) -> bool {
        matches!(name, "getKeyframes" | "getComputedTiming")
    }
    fn get(&self, e: &mut Engine, _t: Value, key: &str) -> JsResult<Option<Value>> {
        Ok(Some(match key {
            "target" => self.target.clone(),
            "pseudoElement" => Value::Null,
            "composite" | "iterationComposite" => e.str_val("replace"),
            _ => return Ok(None),
        }))
    }
    fn invoke(
        &self,
        _e: &mut Engine,
        _t: Value,
        name: &str,
        _args: &[Value],
    ) -> JsResult<Option<Value>> {
        match name {
            "getKeyframes" => Ok(Some(self.keyframes.clone())),
            "getComputedTiming" => Ok(Some(self.options.clone())),
            _ => Ok(None),
        }
    }
}

/// `new KeyframeEffect(target, keyframes, options)`
/// (JS-C461) — ctor spec : les 3 args stockés (le forme
/// copie `new KeyframeEffect(kfe)` délègue aux accesseurs
/// du source via `read_prop` — borné).
pub(crate) fn keyframe_effect_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let target = arg(args, 0);
    let keyframes = arg(args, 1);
    let options = arg(args, 2);
    let v = e.new_host_object(Rc::new(KfeHost {
        target,
        keyframes,
        options,
    }));
    if let Value::Obj(id) = v {
        let p = kfe_proto(e);
        if let Some(o) = e.heap.obj_mut(id) {
            o.set_proto(Value::Obj(p));
        }
    }
    Ok(v)
}

/// `new Animation(effect?, timeline?)` (JS-C461) — même
/// `AnimData` que `el.animate` mais `playState='idle'`
/// (spec : le ctor ne joue pas — `play()` démarre) et
/// `ready` pendante jusqu'au premier `play`.
pub(crate) fn animation_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let effect = arg(args, 0);
    let timeline = arg(args, 1);
    let node = kfe_target_node(e, &effect);
    let tl = if matches!(timeline, Value::Undefined) {
        Value::Obj(document_timeline(e))
    } else {
        timeline
    };
    let finished = e.new_promise();
    let ready = e.new_promise();
    let data = Rc::new(RefCell::new(AnimData {
        node: node.unwrap_or(0),
        state: "idle",
        rate: 1.0,
        finished,
        ready,
        wrapper: None,
        onfinish: Value::Undefined,
        oncancel: Value::Undefined,
        onremove: Value::Undefined,
        id: String::new(),
        effect,
        timeline: tl,
        css_name: None,
        class_tag: "Animation",
    }));
    e.misc.anims.push(data.clone());
    Ok(anim_wrapper(e, &data))
}

/// `NodeId` de la cible d'un `KeyframeEffect` (0 si cible
/// non-élément — la machine à états n'exige pas de nœud).
fn kfe_target_node(e: &mut Engine, effect: &Value) -> Option<NodeId> {
    let Value::Obj(id) = effect else {
        return None;
    };
    let target = e.host_of(*id).and_then(|h| {
        h.as_any()
            .downcast_ref::<KfeHost>()
            .map(|k| k.target.clone())
    })?;
    node_for(e, target)
}

/// `el.animate(keyframes, duration|options)` (CAP-187) —
/// `Animation` machine à états : `playState='running'`,
/// `ready` résolu, `finished` pendante (résolue par
/// `finish()` ou annulée par `cancel()` — pas de timeline
/// réelle, borné).
pub(crate) fn animate(e: &mut Engine, t: Value, args: &[Value]) -> JsResult<Value> {
    let Some(n) = node_for(e, t.clone()) else {
        return Err(e.type_error("animate: not an Element"));
    };
    // JS-C461 : `el.animate` crée un `KeyframeEffect`
    // implicite (spec) — exposé via `animation.effect`.
    let kfe = keyframe_effect_ctor(e, Value::Null, &[t.clone(), arg(args, 0), arg(args, 1)])?;
    let finished = e.new_promise();
    let ready = e.new_promise();
    e.resolve_promise(ready, Value::Undefined);
    let data = Rc::new(RefCell::new(AnimData {
        node: n,
        state: "running",
        rate: 1.0,
        finished,
        ready,
        wrapper: None,
        onfinish: Value::Undefined,
        oncancel: Value::Undefined,
        onremove: Value::Undefined,
        id: String::new(),
        effect: kfe,
        timeline: Value::Obj(document_timeline(e)),
        css_name: None,
        class_tag: "Animation",
    }));
    e.misc.anims.push(data.clone());
    Ok(anim_wrapper(e, &data))
}

/// Marque posée par la session sur le miroir pour chaque
/// animation **CSS** active (JS-C475) — `data-explorer-anim`
/// = `nom` ou `nom|paused`. `get_animations` les matérialise
/// en `Animation` honnêtes : identité `(nœud, nom)` stable
/// entre deux appels, mortes retirées du registre.
const CSS_ANIM_ATTR: &str = "data-explorer-anim";

/// Matérialise les animations CSS marquées du miroir en
/// `AnimData` — crée les entrées manquantes, retire les
/// mortes, met à jour `paused` en place. Retourne les
/// `NodeId` des éléments marqués (pour le filtre de scope).
fn sync_css_anims(e: &mut Engine) -> Vec<(crate::dom::NodeId, String, bool)> {
    let Some((dom, _)) = e.dom_ctx.clone() else {
        return Vec::new();
    };
    let mut marks: Vec<(crate::dom::NodeId, String, bool)> = Vec::new();
    {
        let d = dom.borrow();
        for (id, n) in d.nodes.iter().enumerate() {
            if n.kind != crate::dom::NodeKind::Element {
                continue;
            }
            let Some((_, v)) = n
                .attrs
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(CSS_ANIM_ATTR))
            else {
                continue;
            };
            // `nom` ou `nom|paused` — `|` ne peut pas figurer
            // dans un ident CSS (grammaire des noms @keyframes).
            let (name, paused) = match v.rsplit_once('|') {
                Some((name, "paused")) => (name, true),
                _ => (v.as_str(), false),
            };
            marks.push((id as crate::dom::NodeId, name.to_string(), paused));
        }
    }
    // Mortes : une entrée CSS dont la marque a disparu du
    // miroir quitte le registre (spec : `getAnimations` ne
    // rend que les animations « relevant »).
    e.misc.anims.retain(|a| {
        let d = a.borrow();
        d.css_name
            .as_ref()
            .is_none_or(|cn| marks.iter().any(|(n, name, _)| *n == d.node && name == cn))
    });
    for (node, name, paused) in &marks {
        let state = if *paused { "paused" } else { "running" };
        if let Some(a) = e.misc.anims.iter().find(|a| {
            let d = a.borrow();
            d.node == *node && d.css_name.as_deref() == Some(name.as_str())
        }) {
            let cur = a.borrow().state;
            if cur != "finished" {
                a.borrow_mut().state = state;
            }
            continue;
        }
        let finished = e.new_promise();
        let ready = e.new_promise();
        e.resolve_promise(ready, Value::Undefined);
        let data = Rc::new(RefCell::new(AnimData {
            node: *node,
            state,
            rate: 1.0,
            finished,
            ready,
            wrapper: None,
            onfinish: Value::Undefined,
            oncancel: Value::Undefined,
            onremove: Value::Undefined,
            id: String::new(),
            effect: Value::Null,
            timeline: Value::Obj(document_timeline(e)),
            css_name: Some(name.clone()),
            class_tag: "CSSAnimation",
        }));
        e.misc.anims.push(data);
    }
    marks
}

/// `el.getAnimations()`/`document.getAnimations()`
/// (CAP-187 + JS-C475) — wrappers des animations vivantes :
/// celles nées du script (`el.animate`, `new Animation`)
/// **et** les animations `@keyframes` CSS déclaratives
/// remontées par la session via `data-explorer-anim` (le
/// filtre élément borne aux animations de ce nœud ;
/// document → toutes — borné : pas de sous-arbre).
pub(crate) fn get_animations(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let scope = node_for(e, this);
    sync_css_anims(e);
    let datas: Vec<Rc<RefCell<AnimData>>> = e
        .misc
        .anims
        .iter()
        .filter(|d| {
            let d = d.borrow();
            d.state != "idle" && scope.is_none_or(|s| d.node == s)
        })
        .cloned()
        .collect();
    let anims = datas.iter().map(|d| anim_wrapper(e, d)).collect();
    Ok(e.new_array(anims))
}

impl Host for AnimHost {
    fn class_name(&self) -> &'static str {
        self.data.borrow().class_tag
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn has_method(&self, name: &str) -> bool {
        matches!(
            name,
            "play"
                | "pause"
                | "finish"
                | "cancel"
                | "reverse"
                | "updatePlaybackRate"
                | "persist"
                | "commitStyles"
        )
    }

    fn get(&self, e: &mut Engine, t: Value, key: &str) -> JsResult<Option<Value>> {
        // Méthodes lisibles (Pareto `api` lot 4) — même
        // discipline que les nœuds : `typeof anim.play` et
        // `'play' in anim` exigent une fonction en lecture ;
        // le trampoline redispatche `Host::invoke`.
        if self.has_method(key) {
            let nv = e.str_val(key);
            return Ok(Some(e.bound_native(
                "anim.method",
                anim_method_trampoline,
                vec![nv, t],
            )));
        }
        let d = self.data.borrow();
        Ok(Some(match key {
            "id" => e.str_val(d.id.clone()),
            // `startTime`/`overallProgress` — `null` spec sans
            // horloge de timeline dans le miroir (borne).
            "startTime" | "overallProgress" => Value::Null,
            // `replaceState` — `"active"` honnête : la machine
            // n'a pas de compositeur (pas d'état `removed`
            // sans timeline — même borne CAP-187).
            "replaceState" => return Ok(Some(e.str_val("active"))),
            "onremove" => d.onremove.clone(),
            "playState" => return Ok(Some(e.str_val(d.state))),
            "playbackRate" => Value::Num(d.rate),
            "pending" => Value::Bool(false),
            "currentTime" => Value::Num(0.0),
            // JS-C461 — `effect`/`timeline` servent les
            // valeurs posées au ctor (`new Animation(kfe,tl)`)
            // ou l'implicite `el.animate` (KFE + timeline).
            "effect" => d.effect.clone(),
            "timeline" => d.timeline.clone(),
            // `CSSAnimation.animationName` (JS-C475) —
            // `undefined` sur les `Animation` nées du script
            // (WAAPI : pas de nom déclaratif).
            "animationName" => d
                .css_name
                .as_ref()
                .map_or(Value::Undefined, |n| e.str_val(n.clone())),
            "finished" => Value::Obj(d.finished),
            "ready" => Value::Obj(d.ready),
            "onfinish" => d.onfinish.clone(),
            "oncancel" => d.oncancel.clone(),
            _ => return Ok(None),
        }))
    }

    fn set(&self, e: &mut Engine, _t: Value, key: &str, v: Value) -> JsResult<bool> {
        match key {
            "onfinish" => {
                self.data.borrow_mut().onfinish = v;
                Ok(true)
            }
            "oncancel" => {
                self.data.borrow_mut().oncancel = v;
                Ok(true)
            }
            "onremove" => {
                self.data.borrow_mut().onremove = v;
                Ok(true)
            }
            "id" => {
                self.data.borrow_mut().id = e.to_js_string(v)?;
                Ok(true)
            }
            "playbackRate" => {
                self.data.borrow_mut().rate = e.to_number(v).unwrap_or(1.0);
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    fn invoke(
        &self,
        e: &mut Engine,
        t: Value,
        name: &str,
        args: &[Value],
    ) -> JsResult<Option<Value>> {
        match name {
            "play" => {
                let ready = {
                    let mut d = self.data.borrow_mut();
                    d.state = "running";
                    d.ready
                };
                // JS-C461 — `new Animation()` démarre `idle`
                // avec `ready` pendante : le premier `play`
                // la résout (idempotent sur `el.animate`).
                e.resolve_promise(ready, Value::Undefined);
                Ok(Some(Value::Undefined))
            }
            "pause" => {
                self.data.borrow_mut().state = "paused";
                Ok(Some(Value::Undefined))
            }
            "finish" => {
                let (pid, onf, w) = {
                    let mut d = self.data.borrow_mut();
                    d.state = "finished";
                    (d.finished, d.onfinish.clone(), d.wrapper)
                };
                // `finished` résout avec l'objet `Animation`
                // (spec) — pas la promesse (auto-résolution
                // → rejet TypeError).
                e.resolve_promise(pid, w.map_or(Value::Undefined, Value::Obj));
                if e.is_callable(onf.clone()) {
                    let ev = e.new_object();
                    e.call_value(onf, t, &[Value::Obj(ev)])?;
                }
                Ok(Some(Value::Undefined))
            }
            "cancel" => {
                let (pid, onc) = {
                    let mut d = self.data.borrow_mut();
                    d.state = "idle";
                    (d.finished, d.oncancel.clone())
                };
                let m = e.str_val("AbortError: animation canceled");
                e.reject_promise(pid, m);
                if e.is_callable(onc.clone()) {
                    let ev = e.new_object();
                    e.call_value(onc, t, &[Value::Obj(ev)])?;
                }
                Ok(Some(Value::Undefined))
            }
            "reverse" => {
                let mut d = self.data.borrow_mut();
                d.rate = -d.rate;
                d.state = "running";
                Ok(Some(Value::Undefined))
            }
            "updatePlaybackRate" => {
                self.data.borrow_mut().rate = e.to_number(arg(args, 0)).unwrap_or(1.0);
                Ok(Some(Value::Undefined))
            }
            // `persist`/`commitStyles` — no-op honnête : pas
            // de compositeur dans le miroir (borné).
            "persist" | "commitStyles" => Ok(Some(Value::Undefined)),
            _ => Ok(None),
        }
    }
}

/// `document.startViewTransition(cb)` (CAP-187) —
/// `ViewTransition` machine à états honnête : le callback
/// tourne immédiatement (pas de capture visuelle), les
/// promesses `updateCallbackDone`/`ready`/`finished`
/// résolvent — borné : pas de transition réelle.
pub(crate) fn start_view_transition(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let cb = arg(args, 0);
    let ucd = e.new_promise();
    let ready = e.new_promise();
    let fin = e.new_promise();
    let o = e.new_object();
    e.set_own(o, "updateCallbackDone", Value::Obj(ucd), true);
    e.set_own(o, "ready", Value::Obj(ready), true);
    e.set_own(o, "finished", Value::Obj(fin), true);
    e.add_method(o, "skipTransition", vt_skip);
    let p = vt_proto(e);
    if let Some(obj) = e.heap.obj_mut(o) {
        obj.set_proto(Value::Obj(p));
    }
    if e.is_callable(cb.clone()) {
        e.call_value(cb, Value::Obj(o), &[])?;
    }
    e.resolve_promise(ucd, Value::Undefined);
    e.resolve_promise(ready, Value::Undefined);
    e.resolve_promise(fin, Value::Obj(o));
    Ok(Value::Obj(o))
}

/// `ViewTransition.prototype` paresseux.
fn vt_proto(e: &mut Engine) -> ObjId {
    if let Some(p) = e.misc.vt_proto {
        return p;
    }
    let p = e.new_object();
    let tag = e.str_val("ViewTransition");
    e.set_own_attrs(
        p,
        crate::interp::SYM_TO_STRING_TAG,
        tag,
        crate::heap::PropAttrs {
            writable: false,
            enumerable: false,
            configurable: true,
        },
    );
    e.misc.vt_proto = Some(p);
    p
}

/// `skipTransition()` — no-op honnête (borné : les
/// promesses sont déjà résolues côté miroir).
fn vt_skip(_e: &mut Engine, _t: Value, _a: &[Value]) -> JsResult<Value> {
    Ok(Value::Undefined)
}

// ===== CAP-188 — `document.evaluate` : sous-ensemble XPath —

/// Host `XPathResult` — liste matérialisée à l'évaluation
/// (borné : snapshot même pour les types itérateurs —
/// documenté ; `iterateNext`/`snapshotItem` indexés).
struct XrHost {
    nodes: Vec<NodeId>,
    /// `XPathResult` type code (1 nombre, 2 chaîne, 3 bool,
    /// 4/5 itérateurs, 7 snapshot, 9 premier).
    ty: f64,
    idx: RefCell<usize>,
    /// Scalaires calculés à l'évaluation.
    num: f64,
    text: String,
    flag: bool,
    dom: crate::dom_host::DomRef,
    cache: crate::dom_host::NodeCache,
}

/// Un pas `XPath` borné.
#[derive(Clone)]
enum Step {
    /// Nom d'élément (ou `*`).
    Tag(String),
    /// `text()` — nœuds texte enfants.
    Text,
    /// `node()` — tout type.
    Any,
    /// `.` contexte, `..` parent.
    SelfNode,
    /// `..` parent.
    Parent,
    /// `@a` — test d'attribut (les nœuds attributs ne sont
    /// pas modélisés : l'étape sélectionne l'élément porteur
    /// — déviation bornée documentée).
    Attr(String),
}

/// Axe `XPath` — `child`/`descendant` sont le défaut implicite
/// (`/`/`//`), les autres viennent de `axe::nom` explicite
/// (insee vol 122 : `evaluate: unsupported XPath` sur du
/// `XPath` réel).
#[derive(Clone, Copy, PartialEq)]
enum Axis {
    Child,
    Descendant,
    SelfNode,
    Parent,
    Ancestor,
    AncestorOrSelf,
    DescendantOrSelf,
    FollowingSibling,
    PrecedingSibling,
    Following,
    Preceding,
    /// `attribute::` — déviation bornée : l'étape teste la
    /// présence de l'attribut sur l'élément courant.
    Attribute,
}

/// Prédicat `[…]` — formes courantes du web réel.
enum Pred {
    /// `[@a]` — présence.
    AttrHas(String),
    /// `[@a='v']` — égalité.
    AttrEq(String, String),
    /// `[n]` — n-ième (spec : par axe/parent — borné global).
    Nth(usize),
    /// `[contains(@a,'v')]` / `[starts-with(@a,'v')]` /
    /// `[ends-with(@a,'v')]` — sous-chaîne sur attribut.
    AttrContains(String, String),
    AttrStartsWith(String, String),
    AttrEndsWith(String, String),
    /// `[text()='v']` — texte direct exact.
    TextEq(String),
    /// `[contains(text(),'v')]` — sous-chaîne sur texte.
    TextContains(String),
    /// `[last()]` — dernier de l'ensemble (borné global).
    Last,
    /// `[position()=n]` — index dans l'ensemble.
    Position(usize),
    /// `[not(@a)]` / `[not(@a='v')]` — négation.
    NotAttrHas(String),
    NotAttrEq(String, String),
}

/// Pas parsé : axe + nom + prédicats (tous requis).
struct ParsedStep {
    step: Step,
    axis: Axis,
    /// `true` si l'axe vient de l'expression (`axis::`, `.`,
    /// `..`, `@x`) — un `//` englobant ne doit pas l'écraser
    /// en `Descendant`.
    axis_explicit: bool,
    preds: Vec<Pred>,
}

/// Parse d'un pas — `axis::nom[pred]…`. `None` sur syntaxe non
/// portée (l'évaluatrice dégrade en résultat vide + warn —
/// insee vol 122 : le `TypeError` tuait le script entier).
fn parse_step(raw: &str) -> Option<ParsedStep> {
    // `axis::step` — `child`/`descendant` arrivent déjà fixés
    // par le séparateur (`/`/`//`) ; ici les axes explicites.
    let (axis, raw) = match raw.split_once("::") {
        Some((a, s)) => {
            let ax = match a {
                "ancestor" => Axis::Ancestor,
                "ancestor-or-self" => Axis::AncestorOrSelf,
                "attribute" => Axis::Attribute,
                "child" => Axis::Child,
                "descendant" => Axis::Descendant,
                "descendant-or-self" => Axis::DescendantOrSelf,
                "following" => Axis::Following,
                "following-sibling" => Axis::FollowingSibling,
                "parent" => Axis::Parent,
                "preceding" => Axis::Preceding,
                "preceding-sibling" => Axis::PrecedingSibling,
                "self" => Axis::SelfNode,
                _ => return None,
            };
            (Some(ax), s)
        }
        None => (None, raw),
    };
    let (name, preds) = raw.find('[').map_or((raw, ""), |i| (&raw[..i], &raw[i..]));
    let (step, axis2) = match name {
        "*" => (Step::Tag("*".into()), None),
        "text()" => (Step::Text, None),
        "node()" | "comment()" => (Step::Any, None),
        "." => (Step::SelfNode, Some(Axis::SelfNode)),
        ".." => (Step::Parent, Some(Axis::Parent)),
        _ if name.starts_with('@') => (Step::Attr(name[1..].into()), Some(Axis::Attribute)),
        _ => (Step::Tag(name.to_ascii_lowercase()), None),
    };
    let explicit = axis.is_some() || axis2.is_some();
    let axis = axis.or(axis2).unwrap_or(Axis::Child);
    let mut s = ParsedStep {
        step,
        axis,
        axis_explicit: explicit,
        preds: Vec::new(),
    };
    let mut rest = preds;
    while rest.starts_with('[') {
        let end = rest.find(']')?;
        let p = rest[1..end].trim();
        s.preds.push(parse_pred(p)?);
        rest = &rest[end + 1..];
    }
    if !rest.is_empty() {
        return None;
    }
    Some(s)
}

/// Parse d'un prédicat `[…]` — `None` sur forme non portée.
fn parse_pred(p: &str) -> Option<Pred> {
    let p = p.trim();
    let quoted = |v: &str| v.trim().trim_matches('\'').trim_matches('"').to_string();
    // Fonctions `contains(a,b)`/`starts-with`/`ends-with` —
    // argument attribut `@a` ou `text()`/`.`.
    for fname in ["contains(", "starts-with(", "ends-with("] {
        if let Some(inner) = p.strip_prefix(fname).and_then(|r| r.strip_suffix(')')) {
            let (a, b) = inner.split_once(',')?;
            let a = a.trim();
            let b = quoted(b);
            if let Some(at) = a.strip_prefix('@') {
                let at = at.trim().to_string();
                return Some(match fname {
                    "contains(" => Pred::AttrContains(at, b),
                    "starts-with(" => Pred::AttrStartsWith(at, b),
                    _ => Pred::AttrEndsWith(at, b),
                });
            }
            if a == "text()" || a == "." {
                // `*-with(text(),…)` borné → contient.
                return Some(Pred::TextContains(b));
            }
            return None;
        }
    }
    if let Some(inner) = p.strip_prefix("not(").and_then(|r| r.strip_suffix(')')) {
        let inner = inner.trim();
        if let Some(a) = inner.strip_prefix('@') {
            if let Some((k, v)) = a.split_once('=') {
                return Some(Pred::NotAttrEq(k.trim().into(), quoted(v)));
            }
            return Some(Pred::NotAttrHas(a.trim().into()));
        }
        return None;
    }
    if p == "last()" {
        return Some(Pred::Last);
    }
    if let Some(v) = p.strip_prefix("position()") {
        let n = v.trim().strip_prefix('=')?.trim().parse::<usize>().ok()?;
        return Some(Pred::Position(n.max(1)));
    }
    if let Some(a) = p.strip_prefix('@') {
        if let Some((k, v)) = a.split_once('=') {
            return Some(Pred::AttrEq(k.trim().into(), quoted(v)));
        }
        return Some(Pred::AttrHas(a.trim().into()));
    }
    if let Some(v) = p.strip_prefix("text()") {
        let v = v.trim().strip_prefix('=')?;
        return Some(Pred::TextEq(quoted(v)));
    }
    if let Ok(n) = p.parse::<usize>() {
        return Some(Pred::Nth(n.max(1)));
    }
    None
}

/// `true` si le nœud passe le pas (nom + prédicats).
fn xp_match(d: &crate::dom::Dom, n: NodeId, ps: &ParsedStep) -> bool {
    let node = &d.nodes[n as usize];
    let ok = match &ps.step {
        Step::Tag(t) => t == "*" || (node.kind == crate::dom::NodeKind::Element && node.tag == *t),
        Step::Text => node.kind == crate::dom::NodeKind::Text,
        Step::Any | Step::SelfNode | Step::Parent => true,
        Step::Attr(a) => {
            // Déviation bornée : l'étape `@a` sélectionne
            // l'élément porteur (nœuds attributs non modélisés).
            node.kind == crate::dom::NodeKind::Element && d.attr(n, a).is_some()
        }
    };
    if !ok {
        return false;
    }
    ps.preds.iter().all(|p| xp_pred(d, n, p))
}

/// Un prédicat appliqué à un nœud.
fn xp_pred(d: &crate::dom::Dom, n: NodeId, p: &Pred) -> bool {
    match p {
        Pred::AttrHas(a) => d.attr(n, a).is_some(),
        Pred::AttrEq(a, want) => d.attr(n, a).is_some_and(|v| v == want),
        Pred::AttrContains(a, w) => d.attr(n, a).is_some_and(|v| v.contains(w.as_str())),
        Pred::AttrStartsWith(a, w) => d.attr(n, a).is_some_and(|v| v.starts_with(w.as_str())),
        Pred::AttrEndsWith(a, w) => d.attr(n, a).is_some_and(|v| v.ends_with(w.as_str())),
        Pred::NotAttrHas(a) => d.attr(n, a).is_none(),
        Pred::NotAttrEq(a, w) => d.attr(n, a).is_none_or(|v| v != w),
        Pred::TextEq(w) => d.text_content(n).trim() == w,
        Pred::TextContains(w) => d.text_content(n).contains(w.as_str()),
        // `Nth`/`Last`/`Position` sont appliqués à l'ensemble
        // dans `xp_apply` — ici ils laissent passer.
        Pred::Nth(_) | Pred::Last | Pred::Position(_) => true,
    }
}

/// Tous les descendants en ordre document.
fn xp_descendants(d: &crate::dom::Dom, n: NodeId) -> Vec<NodeId> {
    let mut out = Vec::new();
    let mut stack = d.child_nodes(n);
    stack.reverse();
    while let Some(c) = stack.pop() {
        out.push(c);
        let mut kids = d.child_nodes(c);
        kids.reverse();
        stack.extend(kids);
    }
    out
}

/// `d.child_nodes(n)` sans garder le `Ref` sur le scrutinee.
fn xp_kids(d: &crate::dom::Dom, n: NodeId) -> Vec<NodeId> {
    d.child_nodes(n)
}

/// Un niveau de pas — l'axe dirige le parcours depuis chaque
/// nœud du set (`//` pré-positionne `Descendant`).
fn xp_apply(d: &crate::dom::Dom, set: &[NodeId], ps: &ParsedStep, desc: bool) -> Vec<NodeId> {
    let axis = if desc && !ps.axis_explicit {
        Axis::Descendant
    } else {
        ps.axis
    };
    let mut out: Vec<NodeId> = Vec::new();
    for &n in set {
        let candidates: Vec<NodeId> = match axis {
            Axis::SelfNode => vec![n],
            Axis::Parent => d.parent(n).into_iter().collect(),
            Axis::Child | Axis::Attribute => xp_kids(d, n),
            Axis::Descendant => xp_descendants(d, n),
            Axis::DescendantOrSelf => {
                let mut v = vec![n];
                v.extend(xp_descendants(d, n));
                v
            }
            Axis::Ancestor | Axis::AncestorOrSelf => {
                let mut v = Vec::new();
                let mut cur = d.parent(n);
                while let Some(p) = cur {
                    v.push(p);
                    cur = d.parent(p);
                }
                if axis == Axis::AncestorOrSelf {
                    v.insert(0, n);
                }
                v
            }
            Axis::FollowingSibling | Axis::PrecedingSibling => {
                let Some(p) = d.parent(n) else { continue };
                let kids = xp_kids(d, p);
                let Some(pos) = kids.iter().position(|&k| k == n) else {
                    continue;
                };
                if axis == Axis::FollowingSibling {
                    kids[pos + 1..].to_vec()
                } else {
                    let mut v = kids[..pos].to_vec();
                    v.reverse();
                    v
                }
            }
            Axis::Following | Axis::Preceding => {
                // Ordre document depuis la racine — borné
                // (spec exclut descendants/ancêtres).
                let all = xp_descendants(d, d.root);
                match all.iter().position(|&m| m == n) {
                    Some(pos) if axis == Axis::Following => {
                        let desc: std::collections::HashSet<NodeId> =
                            xp_descendants(d, n).into_iter().collect();
                        all[pos + 1..]
                            .iter()
                            .copied()
                            .filter(|m| !desc.contains(m))
                            .collect()
                    }
                    Some(pos) => {
                        let anc: std::collections::HashSet<NodeId> = {
                            let mut v = std::collections::HashSet::new();
                            let mut cur = d.parent(n);
                            while let Some(p) = cur {
                                v.insert(p);
                                cur = d.parent(p);
                            }
                            v
                        };
                        let mut v: Vec<NodeId> = all[..pos]
                            .iter()
                            .copied()
                            .filter(|m| !anc.contains(m))
                            .collect();
                        v.reverse();
                        v
                    }
                    None => Vec::new(),
                }
            }
        };
        // `attribute::`/`@x` : le candidat est l'élément
        // **courant** testé pour l'attribut (déviation bornée).
        let attr_step = matches!(ps.step, Step::Attr(_));
        if axis == Axis::Attribute || attr_step {
            if xp_match(d, n, ps) {
                out.push(n);
            }
            continue;
        }
        for m in candidates {
            if xp_match(d, m, ps) {
                out.push(m);
            }
        }
    }
    // Prédicats positionnels — appliqués à l'ensemble global
    // (spec : par axe/parent — déviation bornée documentée).
    for p in &ps.preds {
        match p {
            Pred::Nth(k) | Pred::Position(k) => {
                out = out
                    .iter()
                    .copied()
                    .nth(k.saturating_sub(1))
                    .into_iter()
                    .collect();
            }
            Pred::Last => {
                out = out.last().into_iter().copied().collect();
            }
            _ => {}
        }
    }
    out
}

/// `document.evaluate(expr, ctx, resolver, type, result)`
/// (CAP-188) — sous-ensemble `//tag`, `/tag`, `.//`,
/// `.`/`..`, `text()`/`node()`, `[@a]`, `[@a='v']`, `[n]`,
/// `id('x')`, `count(...)` — syntaxe non portée →
/// `TypeError` honnête (spec `InvalidExpressionError` —
/// borné). Types 1/2/3 scalaires + 4/5/7/9 listes.
/// `document.createExpression(expr, resolver)` — retourne un
/// `XPathExpression` `{evaluate(ctx, type, res)}` qui ré-délègue à
/// `xpath_evaluate` (mur htmx/hyperscript : `createExpression is
/// not a function`).
pub(crate) fn xpath_create_expression(
    e: &mut Engine,
    _this: Value,
    args: &[Value],
) -> JsResult<Value> {
    let expr = arg(args, 0);
    let o = e.new_object();
    e.set_own(o, "__expr", expr, false);
    let ev = e.native_fn("evaluate", xpath_expr_evaluate);
    e.set_own(o, "evaluate", ev, true);
    let tag = e.str_val("XPathExpression");
    e.set_own_attrs(
        o,
        crate::interp::SYM_TO_STRING_TAG,
        tag,
        crate::heap::PropAttrs {
            writable: false,
            enumerable: false,
            configurable: true,
        },
    );
    Ok(Value::Obj(o))
}

/// `XPathExpression.prototype.evaluate` — prépose `__expr` aux
/// args puis ré-délègue à `xpath_evaluate` (ctx, type, result).
fn xpath_expr_evaluate(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let expr = match this {
        Value::Obj(id) => e.get_prop(id, "__expr").unwrap_or(Value::Undefined),
        _ => Value::Undefined,
    };
    let mut a = Vec::with_capacity(args.len() + 1);
    a.push(expr);
    a.extend_from_slice(args);
    xpath_evaluate(e, Value::Undefined, &a)
}

/// `document.createNSResolver(node)` — spec : retourne le nœud
/// résolveur (le `documentElement` ici — notre `XPath` ne résout pas
/// de préfixes externes).
pub(crate) fn create_ns_resolver(e: &mut Engine, _this: Value, args: &[Value]) -> JsResult<Value> {
    if matches!(arg(args, 0), Value::Obj(_)) {
        return Ok(arg(args, 0));
    }
    if let Some((dom, cache)) = e.dom_ctx.clone() {
        let root = dom.borrow().root;
        return Ok(node_value(e, &dom, &cache, root));
    }
    Ok(Value::Null)
}

/// Découpe `a|b|c` aux `|` hors `[]`/`()`/guillemets.
fn xpath_union_split(path: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut quote = '\0';
    let mut cur = String::new();
    for c in path.chars() {
        match c {
            '\'' | '"' if quote == '\0' => quote = c,
            _ if c == quote => quote = '\0',
            '[' | '(' if quote == '\0' => depth += 1,
            ']' | ')' if quote == '\0' => depth -= 1,
            '|' if depth == 0 && quote == '\0' => {
                out.push(std::mem::take(&mut cur));
                continue;
            }
            _ => {}
        }
        cur.push(c);
    }
    out.push(cur);
    out
}

/// Séparateur `/` suivant hors `[]`/`()`/guillemets.
fn xpath_step_end(r: &str) -> usize {
    let mut depth = 0i32;
    let mut quote = '\0';
    for (i, c) in r.char_indices() {
        match c {
            '\'' | '"' if quote == '\0' => quote = c,
            _ if c == quote => quote = '\0',
            '[' | '(' if quote == '\0' => depth += 1,
            ']' | ')' if quote == '\0' => depth -= 1,
            '/' if depth == 0 && quote == '\0' => return i,
            _ => {}
        }
    }
    r.len()
}

/// Évalue une branche de chemin (pas d'union) — `None` si un
/// pas sort du sous-ensemble porté.
fn xpath_eval_branch(d: &crate::dom::Dom, ctx: NodeId, path: &str) -> Option<Vec<NodeId>> {
    let mut set = vec![ctx];
    let mut rest = path.trim();
    // Racine absolue `//x` ou `/x` depuis le contexte (borné :
    // `/` initial n'ancre pas au document root — spec stricte,
    // mais le web réel l'emploie comme descendant du ctx).
    while !rest.is_empty() {
        let (step_raw, desc, next) = if let Some(r) = rest.strip_prefix("//") {
            let i = xpath_step_end(r);
            (&r[..i], true, &r[i..])
        } else if let Some(r) = rest.strip_prefix('/') {
            let i = xpath_step_end(r);
            (&r[..i], false, &r[i..])
        } else {
            let i = xpath_step_end(rest);
            (&rest[..i], false, &rest[i..])
        };
        if step_raw.is_empty() {
            rest = next;
            continue;
        }
        let ps = parse_step(step_raw)?;
        set = xp_apply(d, &set, &ps, desc);
        rest = next;
    }
    Some(set)
}

pub(crate) fn xpath_evaluate(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let _ = this;
    let Some((dom, cache)) = e.dom_ctx.clone() else {
        return Err(e.type_error("evaluate: no document"));
    };
    let expr = e.to_js_string(arg(args, 0))?;
    let ctx = node_for_in(e, arg(args, 1), &dom).unwrap_or_else(|| dom.borrow().root);
    let ty = e.to_number(arg(args, 3)).unwrap_or(9.0);
    let mut nodes: Vec<NodeId> = Vec::new();
    let path = expr.trim().to_string();
    let count = path
        .strip_prefix("count(")
        .and_then(|p| p.strip_suffix(')'))
        .map(str::to_string);
    let path = count.as_deref().unwrap_or(&path).to_string();
    {
        let d = dom.borrow();
        if let Some(id) = path
            .strip_prefix("id(")
            .and_then(|p| p.strip_suffix(')'))
            .map(|s| s.trim().trim_matches('\'').trim_matches('"'))
        {
            if let Some(n) = d.by_id(id) {
                nodes.push(n);
            }
        } else {
            // Union `a|b` — découpe aux `|` hors `[]`/`()`/quotes ;
            // chaque branche évaluée, résultats fusionnés en
            // ordre document (index d'arène) dédupé (spec).
            let mut saw_unsupported = false;
            for branch in xpath_union_split(&path) {
                match xpath_eval_branch(&d, ctx, &branch) {
                    Some(mut set) => nodes.append(&mut set),
                    None => saw_unsupported = true,
                }
            }
            if saw_unsupported {
                // XPath valide hors sous-ensemble : résultat
                // partiel/vide + warn console (le `TypeError`
                // tuait le script entier — insee vol 122).
                // Vide = « aucun match » → les sites enchaînent
                // sur leur fallback.
                let w = e.str_val(format!("evaluate: unsupported XPath — {expr}"));
                let _ = crate::dom_host::console_line(e, Value::Undefined, &[w]);
            }
            nodes.sort_unstable();
            nodes.dedup();
        }
    }
    let flag = !nodes.is_empty();
    let num = nodes.len() as f64;
    let text = nodes
        .first()
        .map_or_else(String::new, |&n| dom.borrow().text_content(n));
    let v = e.new_host_object(Rc::new(XrHost {
        nodes,
        ty,
        idx: RefCell::new(0),
        num,
        text,
        flag,
        dom,
        cache,
    }));
    let p = xr_proto(e);
    if let Value::Obj(id) = v {
        if let Some(o) = e.heap.obj_mut(id) {
            o.set_proto(Value::Obj(p));
        }
    }
    Ok(v)
}

/// `XPathResult.prototype` paresseux.
fn xr_proto(e: &mut Engine) -> ObjId {
    if let Some(p) = e.misc.xr_proto {
        return p;
    }
    let p = e.new_object();
    let tag = e.str_val("XPathResult");
    e.set_own_attrs(
        p,
        crate::interp::SYM_TO_STRING_TAG,
        tag,
        crate::heap::PropAttrs {
            writable: false,
            enumerable: false,
            configurable: true,
        },
    );
    e.misc.xr_proto = Some(p);
    p
}

impl Host for XrHost {
    fn class_name(&self) -> &'static str {
        "XPathResult"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn trace(&self, out: &mut Vec<ObjId>) {
        crate::dom_host::trace_node_cache(&self.cache, out);
    }

    fn has_method(&self, name: &str) -> bool {
        matches!(name, "iterateNext" | "snapshotItem")
    }

    fn get(&self, e: &mut Engine, _t: Value, key: &str) -> JsResult<Option<Value>> {
        Ok(Some(match key {
            "resultType" => Value::Num(self.ty),
            "numberValue" => Value::Num(self.num),
            "stringValue" => return Ok(Some(e.str_val(self.text.clone()))),
            "booleanValue" => Value::Bool(self.flag),
            "snapshotLength" => Value::Num(self.nodes.len() as f64),
            "singleNodeValue" => self
                .nodes
                .first()
                .map_or(Value::Null, |&n| node_value(e, &self.dom, &self.cache, n)),
            "invalidIteratorState" => Value::Bool(false),
            _ => return Ok(None),
        }))
    }

    fn invoke(
        &self,
        e: &mut Engine,
        _t: Value,
        name: &str,
        args: &[Value],
    ) -> JsResult<Option<Value>> {
        match name {
            "iterateNext" => {
                let mut i = self.idx.borrow_mut();
                let Some(&n) = self.nodes.get(*i) else {
                    return Ok(Some(Value::Null));
                };
                *i += 1;
                Ok(Some(node_value(e, &self.dom, &self.cache, n)))
            }
            "snapshotItem" => {
                let i = e.to_number(arg(args, 0)).unwrap_or(0.0) as usize;
                let v = self
                    .nodes
                    .get(i)
                    .map_or(Value::Null, |&n| node_value(e, &self.dom, &self.cache, n));
                Ok(Some(v))
            }
            _ => Ok(None),
        }
    }
}

/// Trampoline « méthode `Animation` lisible » (Pareto `api`
/// lot 4) — redispatche `Host::invoke` sur le receveur lié,
/// même motif que `host_method_trampoline` des nœuds.
fn anim_method_trampoline(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let Some((Value::Str(m), rest)) = args.split_first() else {
        return Ok(Value::Undefined);
    };
    let name = m.to_string();
    let Some(Value::Obj(id)) = rest.first().cloned() else {
        return Ok(Value::Undefined);
    };
    let Some(h) = e.host_of(id) else {
        return Ok(Value::Undefined);
    };
    Ok(h.invoke(e, Value::Obj(id), &name, &rest[1..])?
        .unwrap_or(Value::Undefined))
}

// ---- Observable (JS-C560) --------------------------------------------------
//
// `Observable` (proposal TC39, Chrome >=135) — primitive
// reactive reelle, pas un stub : `new Observable(subscribe)`
// memorise la fonction de production dans `__sub` ;
// `subscribe` materialise un `SubscriptionObserver`
// `{next,error,complete,closed}` et un `Subscription`
// `{unsubscribe,closed}` partageant l'etat `sub` ; les
// operateurs retournent des observables derives dont la
// production souscrit la source avec un observer wrappe ;
// les terminateurs (`forEach`/`reduce`/`first`...) rendent
// une promesse. Bornes documentees : `switchMap`/`concatMap`
// se comportent comme `flatMap` (pas d'annulation des
// inners en vol — un seul flux synchrone en pratique), et
// `takeUntil` complete au premier signal du notifieur.

/// Observable neuf, proto `Observable.prototype` global.
fn obs_new(e: &mut Engine) -> ObjId {
    let o = e.new_object();
    if let Some(Value::Obj(cid)) = e.get_prop(e.global, "Observable") {
        if let Some(pp @ Value::Obj(_)) = e.get_prop(cid, "prototype") {
            if let Some(ob) = e.heap.obj_mut(o) {
                ob.set_proto(pp);
            }
        }
    }
    o
}

/// Lit `src.__sub` et l'appelle avec `observer` — la valeur
/// de retour (teardown callable, optionnel) est propagee.
fn obs_run_src(e: &mut Engine, src: Value, observer: Value) -> JsResult<Value> {
    let Value::Obj(sid) = src else {
        return Ok(Value::Undefined);
    };
    let Some(sub) = e.get_prop(sid, "__sub") else {
        return Ok(Value::Undefined);
    };
    if !e.is_callable(sub.clone()) {
        return Ok(Value::Undefined);
    }
    e.call_value(sub, Value::Obj(sid), std::slice::from_ref(&observer))
}

/// `observer.<m>(v)` si callable — point unique d'emission
/// vers un observer (natifs `next`/`error`/`complete` ou
/// wrappers d'operateurs).
fn obs_emit(e: &mut Engine, ob: Value, m: &'static str, v: Value) -> JsResult<Value> {
    let Value::Obj(oid) = ob.clone() else {
        return Ok(Value::Undefined);
    };
    if let Some(f) = e.get_prop(oid, m) {
        if e.is_callable(f.clone()) {
            e.call_value(f, ob, std::slice::from_ref(&v))?;
        }
    }
    Ok(Value::Undefined)
}

/// `obj.<k>` bool — marqueurs `done`/`closed` des etats lies.
fn obs_flag(e: &mut Engine, st: &Value, k: &str) -> bool {
    e.get_prop_value(st.clone(), k)
        .is_ok_and(|v| e.to_boolean(v))
}

/// `new Observable(subscribe)` — `subscribe` callable exige
/// (spec : `TypeError` sinon) ; stocke en `__sub` interne.
fn observable_ctor(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let sub = arg(args, 0);
    if !e.is_callable(sub.clone()) {
        return Err(e.type_error("Observable: subscribe callback is not a function"));
    }
    let Value::Obj(id) = this else {
        return Err(e.type_error("illegal constructor"));
    };
    e.set_own(id, "__sub", sub, false);
    Ok(Value::Undefined)
}

/// `Observable.from(input)` — Observable : identique ;
/// iterable : emission synchrone de chaque element puis
/// `complete` ; Promise : `next`/`error` a resolution ;
/// autre : `TypeError` (spec).
fn observable_from(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let v = arg(args, 0);
    let out = Value::Obj(obs_new(e));
    if let Value::Obj(id) = v {
        if e.get_prop(id, "__sub").is_some() {
            return Ok(v);
        }
    }
    if let Ok(f) = e.get_prop_value(v.clone(), "then") {
        if e.is_callable(f.clone()) {
            let on_ok = e.bound_native("next", obs_from_resolve, vec![out.clone()]);
            let on_err = e.bound_native("error", obs_from_reject, vec![out.clone()]);
            let _ = e.call_value(f, v.clone(), &[on_ok, on_err]);
            return Ok(out);
        }
    }
    let items = e.iter_values(v)?;
    let arr = e.new_array(items);
    let s = e.bound_native("__sub", obs_from_iter_sub, vec![arr]);
    if let Value::Obj(oid) = out {
        e.set_own(oid, "__sub", s, false);
    }
    Ok(out)
}

/// `__sub` de `Observable.from(iterable)` — emission
/// synchrone des elements puis `complete`.
fn obs_from_iter_sub(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let items = arg(args, 0);
    let ob = arg(args, 1);
    let len = match e.get_prop_value(items.clone(), "length") {
        Ok(Value::Num(n)) => n as usize,
        _ => 0,
    };
    for i in 0..len {
        if obs_flag(e, &ob, "closed") {
            return Ok(Value::Undefined);
        }
        let v = e
            .get_prop_value(items.clone(), &i.to_string())
            .unwrap_or(Value::Undefined);
        obs_emit(e, ob.clone(), "next", v)?;
    }
    obs_emit(e, ob, "complete", Value::Undefined)
}

/// `promise.then` de `Observable.from(promise)` — `next`
/// puis `complete` a resolution.
fn obs_from_resolve(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let out = arg(args, 0);
    let v = arg(args, 1);
    obs_emit(e, out.clone(), "next", v)?;
    obs_emit(e, out, "complete", Value::Undefined)
}

/// `promise.catch` de `Observable.from(promise)` — `error`.
fn obs_from_reject(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let out = arg(args, 0);
    obs_emit(e, out, "error", arg(args, 1))
}

/// `obs.subscribe(observer | next?, error?, complete?)` —
/// construit le `SubscriptionObserver`, invoque `__sub`,
/// rend le `Subscription` `{unsubscribe, closed}`.
fn obs_subscribe(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let (next, error, complete) = match arg(args, 0) {
        f if e.is_callable(f.clone()) => (f, arg(args, 1), arg(args, 2)),
        Value::Obj(oid) => (
            e.get_prop(oid, "next").unwrap_or(Value::Undefined),
            e.get_prop(oid, "error").unwrap_or(Value::Undefined),
            e.get_prop(oid, "complete").unwrap_or(Value::Undefined),
        ),
        _ => (Value::Undefined, arg(args, 1), arg(args, 2)),
    };
    // `sub` = etat partage observer/subscription.
    let sub = e.new_object();
    e.set_own(sub, "closed", Value::Bool(false), true);
    e.set_own(sub, "__td", Value::Undefined, false);
    let so = e.new_object();
    let tag = e.str_val("SubscriptionObserver");
    e.set_own_attrs(
        so,
        crate::interp::SYM_TO_STRING_TAG,
        tag,
        crate::heap::PropAttrs {
            writable: false,
            enumerable: false,
            configurable: true,
        },
    );
    for (m, f, user) in [
        ("next", obs_delegate as crate::heap::NativeFn, next),
        ("error", obs_delegate_err, error),
        ("complete", obs_delegate_complete, complete),
    ] {
        let bf = e.bound_native(m, f, vec![Value::Obj(sub), user]);
        e.set_own(so, m, bf, true);
    }
    let cg = e.bound_native("closed", obs_closed_get, vec![Value::Obj(sub)]);
    if let Some(o) = e.heap.obj_mut(so) {
        o.set_accessor("closed", cg.clone(), Value::Undefined, true);
    }
    let teardown = obs_run_src(e, this, Value::Obj(so))?;
    e.set_own(sub, "__td", teardown, true);
    let subscription = e.new_object();
    let tag = e.str_val("Subscription");
    e.set_own_attrs(
        subscription,
        crate::interp::SYM_TO_STRING_TAG,
        tag,
        crate::heap::PropAttrs {
            writable: false,
            enumerable: false,
            configurable: true,
        },
    );
    let uf = e.bound_native("unsubscribe", obs_unsub, vec![Value::Obj(sub)]);
    e.set_own(subscription, "unsubscribe", uf, true);
    if let Some(o) = e.heap.obj_mut(subscription) {
        o.set_accessor("closed", cg, Value::Undefined, true);
    }
    Ok(Value::Obj(subscription))
}

/// `observer.next` reel — no-op si `closed` (spec).
fn obs_delegate(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let sub = arg(args, 0);
    if obs_flag(e, &sub, "closed") {
        return Ok(Value::Undefined);
    }
    let f = arg(args, 1);
    if e.is_callable(f.clone()) {
        e.call_value(f, Value::Undefined, std::slice::from_ref(&arg(args, 2)))?;
    }
    Ok(Value::Undefined)
}

/// `observer.error` — ferme puis delegue (handler absent :
/// l'erreur est absorbee — pas de hostReportErrors ici).
fn obs_delegate_err(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let sub = arg(args, 0);
    if obs_flag(e, &sub, "closed") {
        return Ok(Value::Undefined);
    }
    e.set_prop_value(sub, "closed", Value::Bool(true))?;
    let f = arg(args, 1);
    if e.is_callable(f.clone()) {
        e.call_value(f, Value::Undefined, std::slice::from_ref(&arg(args, 2)))?;
    }
    Ok(Value::Undefined)
}

/// `observer.complete` — idem `error` sans valeur.
fn obs_delegate_complete(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let sub = arg(args, 0);
    if obs_flag(e, &sub, "closed") {
        return Ok(Value::Undefined);
    }
    e.set_prop_value(sub, "closed", Value::Bool(true))?;
    let f = arg(args, 1);
    if e.is_callable(f.clone()) {
        e.call_value(f, Value::Undefined, &[])?;
    }
    Ok(Value::Undefined)
}

/// `subscription.unsubscribe()` — `closed` + teardown.
fn obs_unsub(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let sub = arg(args, 0);
    if obs_flag(e, &sub, "closed") {
        return Ok(Value::Undefined);
    }
    e.set_prop_value(sub.clone(), "closed", Value::Bool(true))?;
    let td = e.get_prop_value(sub, "__td").unwrap_or(Value::Undefined);
    if let Value::Obj(oid) = td {
        if let Some(f) = e.get_prop(oid, "unsubscribe") {
            if e.is_callable(f.clone()) {
                e.call_value(f, td, &[])?;
            }
        }
    } else if e.is_callable(td.clone()) {
        e.call_value(td, Value::Undefined, &[])?;
    }
    Ok(Value::Undefined)
}

/// Accesseur `closed` partage observer/subscription.
fn obs_closed_get(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    Ok(Value::Bool(obs_flag(e, &arg(args, 0), "closed")))
}

/// Fabrique un observable derive : `__sub` = natif lie
/// `[source, extras...]`.
fn obs_derive(
    e: &mut Engine,
    this: Value,
    mut bound_tail: Vec<Value>,
    sub_fn: crate::heap::NativeFn,
) -> Value {
    let o = obs_new(e);
    let mut bound = vec![this];
    bound.append(&mut bound_tail);
    let s = e.bound_native("__sub", sub_fn, bound);
    e.set_own(o, "__sub", s, false);
    Value::Obj(o)
}

/// Observer wrap standard `{next: nf, error/complete:
/// passthrough}` vers `ob`.
fn obs_wrap(e: &mut Engine, ob: Value, next: Value) -> Value {
    let w = e.new_object();
    e.set_own(w, "next", next, true);
    let ef = e.bound_native("error", obs_err_passthru, vec![ob.clone()]);
    e.set_own(w, "error", ef, true);
    let cf = e.bound_native("complete", obs_complete_passthru, vec![ob]);
    e.set_own(w, "complete", cf, true);
    Value::Obj(w)
}

/// Passthrough `error` vers l'observer aval.
fn obs_err_passthru(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    obs_emit(e, arg(args, 0), "error", arg(args, 1))
}

/// Passthrough `complete` vers l'observer aval.
fn obs_complete_passthru(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    obs_emit(e, arg(args, 0), "complete", Value::Undefined)
}

/// `obs.map(f)` — `next` = `f(v,i)`.
fn obs_map(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let f = arg(args, 0);
    if !e.is_callable(f.clone()) {
        return Err(e.type_error("Observable.map: callback is not a function"));
    }
    Ok(obs_derive(e, this, vec![f], obs_map_sub))
}

fn obs_map_sub(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let src = arg(args, 0);
    let f = arg(args, 1);
    let ob = arg(args, 2);
    let st = e.new_object();
    e.set_own(st, "i", Value::Num(0.0), true);
    let nf = e.bound_native("next", obs_map_next, vec![ob, f, Value::Obj(st)]);
    let w = obs_wrap(e, arg(args, 2), nf);
    obs_run_src(e, src, w)
}

fn obs_map_next(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let ob = arg(args, 0);
    let f = arg(args, 1);
    let st = arg(args, 2);
    let v = arg(args, 3);
    let i = e
        .get_prop_value(st.clone(), "i")
        .ok()
        .and_then(|x| e.to_number(x).ok())
        .unwrap_or(0.0);
    e.set_prop_value(st, "i", Value::Num(i + 1.0))?;
    let r = e.call_value(f, Value::Undefined, &[v, Value::Num(i)])?;
    obs_emit(e, ob, "next", r)
}

/// `obs.filter(f)` — `next` si `f(v,i)` truthy.
fn obs_filter(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let f = arg(args, 0);
    if !e.is_callable(f.clone()) {
        return Err(e.type_error("Observable.filter: callback is not a function"));
    }
    Ok(obs_derive(e, this, vec![f], obs_filter_sub))
}

fn obs_filter_sub(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let src = arg(args, 0);
    let f = arg(args, 1);
    let ob = arg(args, 2);
    let st = e.new_object();
    e.set_own(st, "i", Value::Num(0.0), true);
    let nf = e.bound_native("next", obs_filter_next, vec![ob, f, Value::Obj(st)]);
    let w = obs_wrap(e, arg(args, 2), nf);
    obs_run_src(e, src, w)
}

fn obs_filter_next(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let ob = arg(args, 0);
    let f = arg(args, 1);
    let st = arg(args, 2);
    let v = arg(args, 3);
    let i = e
        .get_prop_value(st.clone(), "i")
        .ok()
        .and_then(|x| e.to_number(x).ok())
        .unwrap_or(0.0);
    e.set_prop_value(st, "i", Value::Num(i + 1.0))?;
    let r = e.call_value(f, Value::Undefined, &[v.clone(), Value::Num(i)])?;
    if e.to_boolean(r) {
        obs_emit(e, ob, "next", v)?;
    }
    Ok(Value::Undefined)
}

/// `obs.take(n)` — `n` emissions puis `complete` aval.
fn obs_take(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let n = e.to_number(arg(args, 0))?.max(0.0);
    Ok(obs_derive(e, this, vec![Value::Num(n)], obs_take_sub))
}

fn obs_take_sub(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let src = arg(args, 0);
    let st = e.new_object();
    e.set_own(st, "n", arg(args, 1), true);
    e.set_own(st, "i", Value::Num(0.0), true);
    let nf = e.bound_native("next", obs_take_next, vec![arg(args, 2), Value::Obj(st)]);
    let w = obs_wrap(e, arg(args, 2), nf);
    obs_run_src(e, src, w)
}

fn obs_take_next(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let ob = arg(args, 0);
    let st = arg(args, 1);
    let v = arg(args, 2);
    let i = e
        .get_prop_value(st.clone(), "i")
        .ok()
        .and_then(|x| e.to_number(x).ok())
        .unwrap_or(0.0);
    let n = e
        .get_prop_value(st.clone(), "n")
        .ok()
        .and_then(|x| e.to_number(x).ok())
        .unwrap_or(0.0);
    if i >= n {
        return Ok(Value::Undefined);
    }
    e.set_prop_value(st, "i", Value::Num(i + 1.0))?;
    obs_emit(e, ob.clone(), "next", v)?;
    if i + 1.0 >= n {
        obs_emit(e, ob, "complete", Value::Undefined)?;
    }
    Ok(Value::Undefined)
}

/// `obs.drop(n)` — saute les `n` premieres emissions.
fn obs_drop(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let n = e.to_number(arg(args, 0))?.max(0.0);
    Ok(obs_derive(e, this, vec![Value::Num(n)], obs_drop_sub))
}

fn obs_drop_sub(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let src = arg(args, 0);
    let st = e.new_object();
    e.set_own(st, "n", arg(args, 1), true);
    let nf = e.bound_native("next", obs_drop_next, vec![arg(args, 2), Value::Obj(st)]);
    let w = obs_wrap(e, arg(args, 2), nf);
    obs_run_src(e, src, w)
}

fn obs_drop_next(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let ob = arg(args, 0);
    let st = arg(args, 1);
    let v = arg(args, 2);
    let n = e
        .get_prop_value(st.clone(), "n")
        .ok()
        .and_then(|x| e.to_number(x).ok())
        .unwrap_or(0.0);
    if n > 0.0 {
        e.set_prop_value(st, "n", Value::Num(n - 1.0))?;
        return Ok(Value::Undefined);
    }
    obs_emit(e, ob, "next", v)
}

/// `obs.scan(f, seed?)` — accumulateur emis a chaque pas.
fn obs_scan(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let f = arg(args, 0);
    if !e.is_callable(f.clone()) {
        return Err(e.type_error("Observable.scan: callback is not a function"));
    }
    let seed = arg(args, 1);
    Ok(obs_derive(e, this, vec![f, seed], obs_scan_sub))
}

fn obs_scan_sub(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let src = arg(args, 0);
    let st = e.new_object();
    e.set_own(st, "acc", arg(args, 2), true);
    e.set_own(st, "i", Value::Num(0.0), true);
    let nf = e.bound_native(
        "next",
        obs_scan_next,
        vec![arg(args, 3), arg(args, 1), Value::Obj(st)],
    );
    let w = obs_wrap(e, arg(args, 3), nf);
    obs_run_src(e, src, w)
}

fn obs_scan_next(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let ob = arg(args, 0);
    let f = arg(args, 1);
    let st = arg(args, 2);
    let v = arg(args, 3);
    let i = e
        .get_prop_value(st.clone(), "i")
        .ok()
        .and_then(|x| e.to_number(x).ok())
        .unwrap_or(0.0);
    e.set_prop_value(st.clone(), "i", Value::Num(i + 1.0))?;
    let acc = e
        .get_prop_value(st.clone(), "acc")
        .unwrap_or(Value::Undefined);
    let r = e.call_value(f, Value::Undefined, &[acc, v, Value::Num(i)])?;
    e.set_prop_value(st, "acc", r.clone())?;
    obs_emit(e, ob, "next", r)
}

/// `obs.flatMap(f)`/`concatMap`/`switchMap` — souscrit chaque
/// observable inner produit par `f(v,i)` et reemet ses
/// `next` (borne : sans annulation des inners en vol —
/// `switchMap`/`concatMap` = `flatMap` sur un flux
/// synchrone).
fn obs_flatmap(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let f = arg(args, 0);
    if !e.is_callable(f.clone()) {
        return Err(e.type_error("flatMap: callback is not a function"));
    }
    Ok(obs_derive(e, this, vec![f], obs_flatmap_sub))
}

fn obs_flatmap_sub(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let src = arg(args, 0);
    let st = e.new_object();
    e.set_own(st, "i", Value::Num(0.0), true);
    let nf = e.bound_native(
        "next",
        obs_flatmap_next,
        vec![arg(args, 2), arg(args, 1), Value::Obj(st)],
    );
    let w = obs_wrap(e, arg(args, 2), nf);
    obs_run_src(e, src, w)
}

fn obs_flatmap_next(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let ob = arg(args, 0);
    let f = arg(args, 1);
    let st = arg(args, 2);
    let v = arg(args, 3);
    let i = e
        .get_prop_value(st.clone(), "i")
        .ok()
        .and_then(|x| e.to_number(x).ok())
        .unwrap_or(0.0);
    e.set_prop_value(st, "i", Value::Num(i + 1.0))?;
    let inner = e.call_value(f, Value::Undefined, &[v, Value::Num(i)])?;
    // Observer inner : `next`/`error` reemis, `complete`
    // ignore (la fin globale vient de la source).
    let w = e.new_object();
    let nf = e.bound_native("next", obs_inner_next, vec![ob.clone()]);
    e.set_own(w, "next", nf, true);
    let ef = e.bound_native("error", obs_err_passthru, vec![ob]);
    e.set_own(w, "error", ef, true);
    obs_run_src(e, inner, Value::Obj(w))?;
    Ok(Value::Undefined)
}

fn obs_inner_next(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    obs_emit(e, arg(args, 0), "next", arg(args, 1))
}

/// `obs.catch(f)` — `error` : souscrit `f(err)` et reemet.
fn obs_catch(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let f = arg(args, 0);
    if !e.is_callable(f.clone()) {
        return Err(e.type_error("Observable.catch: callback is not a function"));
    }
    Ok(obs_derive(e, this, vec![f], obs_catch_sub))
}

fn obs_catch_sub(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let src = arg(args, 0);
    let ob = arg(args, 2);
    let w = e.new_object();
    let nf = e.bound_native("next", obs_inner_next, vec![ob.clone()]);
    e.set_own(w, "next", nf, true);
    let ef = e.bound_native("error", obs_catch_err, vec![ob.clone(), arg(args, 1)]);
    e.set_own(w, "error", ef, true);
    let cf = e.bound_native("complete", obs_complete_passthru, vec![ob]);
    e.set_own(w, "complete", cf, true);
    obs_run_src(e, src, Value::Obj(w))
}

fn obs_catch_err(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let ob = arg(args, 0);
    let f = arg(args, 1);
    let err = arg(args, 2);
    let inner = e.call_value(f, Value::Undefined, &[err])?;
    let w = e.new_object();
    let nf = e.bound_native("next", obs_inner_next, vec![ob.clone()]);
    e.set_own(w, "next", nf, true);
    let ef = e.bound_native("error", obs_err_passthru, vec![ob.clone()]);
    e.set_own(w, "error", ef, true);
    let cf = e.bound_native("complete", obs_complete_passthru, vec![ob]);
    e.set_own(w, "complete", cf, true);
    obs_run_src(e, inner, Value::Obj(w))?;
    Ok(Value::Undefined)
}

/// `obs.finally(f)` — `f()` au `complete`/`error` aval.
fn obs_finally(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let f = arg(args, 0);
    if !e.is_callable(f.clone()) {
        return Err(e.type_error("Observable.finally: callback is not a function"));
    }
    Ok(obs_derive(e, this, vec![f], obs_finally_sub))
}

fn obs_finally_sub(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let src = arg(args, 0);
    let ob = arg(args, 2);
    let f = arg(args, 1);
    let w = e.new_object();
    let nf = e.bound_native("next", obs_inner_next, vec![ob.clone()]);
    e.set_own(w, "next", nf, true);
    let ef = e.bound_native("error", obs_finally_err, vec![ob.clone(), f.clone()]);
    e.set_own(w, "error", ef, true);
    let cf = e.bound_native("complete", obs_finally_complete, vec![ob, f]);
    e.set_own(w, "complete", cf, true);
    obs_run_src(e, src, Value::Obj(w))
}

fn obs_finally_err(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let ob = arg(args, 0);
    let f = arg(args, 1);
    let err = arg(args, 2);
    e.call_value(f, Value::Undefined, &[])?;
    obs_emit(e, ob, "error", err)
}

fn obs_finally_complete(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let ob = arg(args, 0);
    let f = arg(args, 1);
    e.call_value(f, Value::Undefined, &[])?;
    obs_emit(e, ob, "complete", Value::Undefined)
}

/// `obs.inspect({next?,error?,complete?,subscribe?,unsubscribe?})`
/// — tap : chaque callback invoque avant le passthrough.
fn obs_inspect(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    Ok(obs_derive(e, this, vec![arg(args, 0)], obs_inspect_sub))
}

fn obs_inspect_sub(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let src = arg(args, 0);
    let ob = arg(args, 2);
    let opts = arg(args, 1);
    let w = e.new_object();
    let nf = e.bound_native("next", obs_inspect_next, vec![ob.clone(), opts.clone()]);
    e.set_own(w, "next", nf, true);
    let ef = e.bound_native("error", obs_inspect_err, vec![ob.clone(), opts.clone()]);
    e.set_own(w, "error", ef, true);
    let cf = e.bound_native("complete", obs_inspect_complete, vec![ob, opts]);
    e.set_own(w, "complete", cf, true);
    obs_run_src(e, src, Value::Obj(w))
}

fn obs_inspect_next(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let ob = arg(args, 0);
    let v = arg(args, 2);
    if let Ok(f) = e.get_prop_value(arg(args, 1), "next") {
        if e.is_callable(f.clone()) {
            e.call_value(f, Value::Undefined, std::slice::from_ref(&v))?;
        }
    }
    obs_emit(e, ob, "next", v)
}

fn obs_inspect_err(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let ob = arg(args, 0);
    let err = arg(args, 2);
    if let Ok(f) = e.get_prop_value(arg(args, 1), "error") {
        if e.is_callable(f.clone()) {
            e.call_value(f, Value::Undefined, std::slice::from_ref(&err))?;
        }
    }
    obs_emit(e, ob, "error", err)
}

fn obs_inspect_complete(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let ob = arg(args, 0);
    if let Ok(f) = e.get_prop_value(arg(args, 1), "complete") {
        if e.is_callable(f.clone()) {
            e.call_value(f, Value::Undefined, &[])?;
        }
    }
    obs_emit(e, ob, "complete", Value::Undefined)
}

/// `obs.takeUntil(notifier)` — `complete` aval au premier
/// `next`/`error`/`complete` du notifieur.
fn obs_take_until(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    Ok(obs_derive(e, this, vec![arg(args, 0)], obs_takeuntil_sub))
}

fn obs_takeuntil_sub(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let src = arg(args, 0);
    let notifier = arg(args, 1);
    let ob = arg(args, 2);
    // Notifieur : premier signal → `complete` aval.
    let w = e.new_object();
    let nf = e.bound_native("next", obs_takeuntil_hit, vec![ob.clone()]);
    e.set_own(w, "next", nf, true);
    let ef = e.bound_native("error", obs_err_passthru, vec![ob.clone()]);
    e.set_own(w, "error", ef, true);
    let cf = e.bound_native("complete", obs_takeuntil_hit, vec![ob.clone()]);
    e.set_own(w, "complete", cf, true);
    obs_run_src(e, notifier, Value::Obj(w))?;
    // Source : passthrough complet.
    let nf = e.bound_native("next", obs_inner_next, vec![ob.clone()]);
    let w2 = obs_wrap(e, ob, nf);
    obs_run_src(e, src, w2)
}

fn obs_takeuntil_hit(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    obs_emit(e, arg(args, 0), "complete", Value::Undefined)
}

/// Montage d'un terminateur : `st` partage `{done, acc...}`,
/// observer `{next: nf, error→rejet, complete: cf}`,
/// retour = la promesse `pid`.
fn obs_term_mount(
    e: &mut Engine,
    this: Value,
    bound_extra: Vec<Value>,
    next_fn: crate::heap::NativeFn,
    complete_fn: crate::heap::NativeFn,
) -> JsResult<Value> {
    let pid = e.new_promise();
    let st = e.new_object();
    e.set_own(st, "done", Value::Bool(false), true);
    let mut bound = vec![Value::Obj(pid), Value::Obj(st)];
    bound.extend(bound_extra.clone());
    let mut bound_c = vec![Value::Obj(pid), Value::Obj(st)];
    bound_c.extend(bound_extra);
    let w = e.new_object();
    let nf = e.bound_native("next", next_fn, bound);
    e.set_own(w, "next", nf, true);
    let ef = e.bound_native("error", obs_term_reject, vec![Value::Obj(pid)]);
    e.set_own(w, "error", ef, true);
    let cf = e.bound_native("complete", complete_fn, bound_c);
    e.set_own(w, "complete", cf, true);
    let td = obs_run_src(e, this, Value::Obj(w))?;
    e.set_own(st, "__td", td, false);
    Ok(Value::Obj(pid))
}

/// `error` d'un terminateur → rejet de la promesse.
fn obs_term_reject(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let Value::Obj(pid) = arg(args, 0) else {
        return Ok(Value::Undefined);
    };
    e.reject_promise(pid, arg(args, 1));
    Ok(Value::Undefined)
}

/// `obs.forEach(f)` → `Promise<undefined>` (f(v,i) par
/// emission).
fn obs_for_each(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let f = arg(args, 0);
    if !e.is_callable(f.clone()) {
        return Err(e.type_error("Observable.forEach: callback is not a function"));
    }
    obs_term_mount(
        e,
        this,
        vec![f],
        obs_foreach_next,
        obs_term_resolve_undefined,
    )
}

fn obs_foreach_next(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let st = arg(args, 1);
    if obs_flag(e, &st, "done") {
        return Ok(Value::Undefined);
    }
    let f = arg(args, 2);
    let v = arg(args, 3);
    let i = e
        .get_prop_value(st.clone(), "i")
        .ok()
        .and_then(|x| e.to_number(x).ok())
        .unwrap_or(0.0);
    e.set_prop_value(st, "i", Value::Num(i + 1.0))?;
    e.call_value(f, Value::Undefined, &[v, Value::Num(i)])?;
    Ok(Value::Undefined)
}

fn obs_term_resolve_undefined(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    if let Value::Obj(pid) = arg(args, 0) {
        e.resolve_promise(pid, Value::Undefined);
    }
    Ok(Value::Undefined)
}

/// `obs.reduce(f, seed?)` → `Promise<acc>` — sans seed, la
/// premiere emission amorce (spec Array#reduce).
fn obs_reduce(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let f = arg(args, 0);
    if !e.is_callable(f.clone()) {
        return Err(e.type_error("Observable.reduce: callback is not a function"));
    }
    obs_term_mount(
        e,
        this,
        vec![f, arg(args, 1), Value::Bool(args.len() > 1)],
        obs_reduce_next,
        obs_reduce_complete,
    )
}

fn obs_reduce_next(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let st = arg(args, 1);
    if obs_flag(e, &st, "done") {
        return Ok(Value::Undefined);
    }
    let f = arg(args, 2);
    let v = arg(args, 5);
    let i = e
        .get_prop_value(st.clone(), "i")
        .ok()
        .and_then(|x| e.to_number(x).ok())
        .unwrap_or(0.0);
    e.set_prop_value(st.clone(), "i", Value::Num(i + 1.0))?;
    let has = obs_flag(e, &st, "has") || matches!(arg(args, 4), Value::Bool(true));
    if !has {
        e.set_prop_value(st.clone(), "acc", v)?;
        e.set_prop_value(st, "has", Value::Bool(true))?;
        return Ok(Value::Undefined);
    }
    let acc = e
        .get_prop_value(st.clone(), "acc")
        .unwrap_or(Value::Undefined);
    let r = e.call_value(f, Value::Undefined, &[acc, v, Value::Num(i)])?;
    e.set_prop_value(st, "acc", r)?;
    Ok(Value::Undefined)
}

fn obs_reduce_complete(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let Value::Obj(pid) = arg(args, 0) else {
        return Ok(Value::Undefined);
    };
    let st = arg(args, 1);
    if !obs_flag(e, &st, "has") && !matches!(arg(args, 4), Value::Bool(true)) {
        let t = e.error_value(e.protos.error, "TypeError", "reduce of empty observable");
        e.reject_promise(pid, t);
        return Ok(Value::Undefined);
    }
    let acc = e.get_prop_value(st, "acc").unwrap_or_else(|_| arg(args, 3));
    e.resolve_promise(pid, acc);
    Ok(Value::Undefined)
}

/// `obs.toArray()` → `Promise<Array>` des emissions.
fn obs_to_array(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    obs_term_mount(e, this, Vec::new(), obs_toarray_next, obs_toarray_complete)
}

fn obs_toarray_next(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let st = arg(args, 1);
    if obs_flag(e, &st, "done") {
        return Ok(Value::Undefined);
    }
    let len = e
        .get_prop_value(st.clone(), "len")
        .ok()
        .and_then(|x| e.to_number(x).ok())
        .unwrap_or(0.0);
    e.set_prop_value(st.clone(), &format!("{}", len as u64), arg(args, 2))?;
    e.set_prop_value(st, "len", Value::Num(len + 1.0))?;
    Ok(Value::Undefined)
}

fn obs_toarray_complete(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let Value::Obj(pid) = arg(args, 0) else {
        return Ok(Value::Undefined);
    };
    let st = arg(args, 1);
    let len = e
        .get_prop_value(st.clone(), "len")
        .ok()
        .and_then(|x| e.to_number(x).ok())
        .unwrap_or(0.0) as usize;
    let mut items = Vec::with_capacity(len);
    for i in 0..len {
        items.push(
            e.get_prop_value(st.clone(), &i.to_string())
                .unwrap_or(Value::Undefined),
        );
    }
    let a = e.new_array(items);
    e.resolve_promise(pid, a);
    Ok(Value::Undefined)
}

/// `obs.first()` → `Promise<v|undefined>` — premiere
/// emission (borne : pas d'unsubscribe amont, marqueur
/// `done` seul — le flux synchrone est court-circuite par
/// le `closed` aval).
fn obs_first(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    obs_term_mount(
        e,
        this,
        Vec::new(),
        obs_first_next,
        obs_term_resolve_undefined,
    )
}

fn obs_first_next(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let st = arg(args, 1);
    if obs_flag(e, &st, "done") {
        return Ok(Value::Undefined);
    }
    e.set_prop_value(st, "done", Value::Bool(true))?;
    if let Value::Obj(pid) = arg(args, 0) {
        e.resolve_promise(pid, arg(args, 2));
    }
    Ok(Value::Undefined)
}

/// `obs.last()` → `Promise<v|undefined>` — derniere
/// emission avant `complete`.
fn obs_last(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    obs_term_mount(e, this, Vec::new(), obs_last_next, obs_last_complete)
}

fn obs_last_next(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let st = arg(args, 1);
    if obs_flag(e, &st, "done") {
        return Ok(Value::Undefined);
    }
    e.set_prop_value(st.clone(), "v", arg(args, 2))?;
    e.set_prop_value(st, "has", Value::Bool(true))?;
    Ok(Value::Undefined)
}

fn obs_last_complete(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    if let Value::Obj(pid) = arg(args, 0) {
        let st = arg(args, 1);
        let v = e.get_prop_value(st, "v").unwrap_or(Value::Undefined);
        e.resolve_promise(pid, v);
    }
    Ok(Value::Undefined)
}

/// `obs.every(f)` → `Promise<bool>` — court-circuite au
/// premier falsy.
fn obs_every(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let f = arg(args, 0);
    if !e.is_callable(f.clone()) {
        return Err(e.type_error("Observable.every: callback is not a function"));
    }
    obs_term_mount(e, this, vec![f], obs_every_next, obs_bool_complete_true)
}

fn obs_every_next(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let st = arg(args, 1);
    if obs_flag(e, &st, "done") {
        return Ok(Value::Undefined);
    }
    let f = arg(args, 2);
    let v = arg(args, 3);
    let i = e
        .get_prop_value(st.clone(), "i")
        .ok()
        .and_then(|x| e.to_number(x).ok())
        .unwrap_or(0.0);
    e.set_prop_value(st.clone(), "i", Value::Num(i + 1.0))?;
    let r = e.call_value(f, Value::Undefined, &[v, Value::Num(i)])?;
    if !e.to_boolean(r) {
        e.set_prop_value(st, "done", Value::Bool(true))?;
        if let Value::Obj(pid) = arg(args, 0) {
            e.resolve_promise(pid, Value::Bool(false));
        }
    }
    Ok(Value::Undefined)
}

fn obs_bool_complete_true(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    if let Value::Obj(pid) = arg(args, 0) {
        let st = arg(args, 1);
        if !obs_flag(e, &st, "done") {
            e.resolve_promise(pid, Value::Bool(true));
        }
    }
    Ok(Value::Undefined)
}

/// `obs.some(f)` → `Promise<bool>` — court-circuite au
/// premier truthy.
fn obs_some(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let f = arg(args, 0);
    if !e.is_callable(f.clone()) {
        return Err(e.type_error("Observable.some: callback is not a function"));
    }
    obs_term_mount(e, this, vec![f], obs_some_next, obs_bool_complete_false)
}

fn obs_some_next(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let st = arg(args, 1);
    if obs_flag(e, &st, "done") {
        return Ok(Value::Undefined);
    }
    let f = arg(args, 2);
    let v = arg(args, 3);
    let i = e
        .get_prop_value(st.clone(), "i")
        .ok()
        .and_then(|x| e.to_number(x).ok())
        .unwrap_or(0.0);
    e.set_prop_value(st.clone(), "i", Value::Num(i + 1.0))?;
    let r = e.call_value(f, Value::Undefined, &[v, Value::Num(i)])?;
    if e.to_boolean(r) {
        e.set_prop_value(st, "done", Value::Bool(true))?;
        if let Value::Obj(pid) = arg(args, 0) {
            e.resolve_promise(pid, Value::Bool(true));
        }
    }
    Ok(Value::Undefined)
}

fn obs_bool_complete_false(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    if let Value::Obj(pid) = arg(args, 0) {
        let st = arg(args, 1);
        if !obs_flag(e, &st, "done") {
            e.resolve_promise(pid, Value::Bool(false));
        }
    }
    Ok(Value::Undefined)
}

/// `obs.find(f)` → `Promise<v|undefined>` — premier match.
fn obs_find(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let f = arg(args, 0);
    if !e.is_callable(f.clone()) {
        return Err(e.type_error("Observable.find: callback is not a function"));
    }
    obs_term_mount(e, this, vec![f], obs_find_next, obs_term_resolve_undefined)
}

fn obs_find_next(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let st = arg(args, 1);
    if obs_flag(e, &st, "done") {
        return Ok(Value::Undefined);
    }
    let f = arg(args, 2);
    let v = arg(args, 3);
    let i = e
        .get_prop_value(st.clone(), "i")
        .ok()
        .and_then(|x| e.to_number(x).ok())
        .unwrap_or(0.0);
    e.set_prop_value(st.clone(), "i", Value::Num(i + 1.0))?;
    let r = e.call_value(f, Value::Undefined, &[v.clone(), Value::Num(i)])?;
    if e.to_boolean(r) {
        e.set_prop_value(st, "done", Value::Bool(true))?;
        if let Value::Obj(pid) = arg(args, 0) {
            e.resolve_promise(pid, v);
        }
    }
    Ok(Value::Undefined)
}

/// Installe `Observable` + `Observable.prototype` (operateurs
/// et terminateurs reels — JS-C560).
pub(crate) fn install_observable(e: &mut Engine) {
    let p = e.new_object();
    let tag = e.str_val("Observable");
    e.set_own_attrs(
        p,
        crate::interp::SYM_TO_STRING_TAG,
        tag,
        crate::heap::PropAttrs {
            writable: false,
            enumerable: false,
            configurable: true,
        },
    );
    for (m, f) in [
        ("subscribe", obs_subscribe as crate::heap::NativeFn),
        ("map", obs_map),
        ("filter", obs_filter),
        ("take", obs_take),
        ("drop", obs_drop),
        ("scan", obs_scan),
        ("flatMap", obs_flatmap),
        ("concatMap", obs_flatmap),
        ("switchMap", obs_flatmap),
        ("catch", obs_catch),
        ("finally", obs_finally),
        ("inspect", obs_inspect),
        ("takeUntil", obs_take_until),
        ("forEach", obs_for_each),
        ("reduce", obs_reduce),
        ("toArray", obs_to_array),
        ("first", obs_first),
        ("last", obs_last),
        ("every", obs_every),
        ("some", obs_some),
        ("find", obs_find),
    ] {
        e.add_method(p, m, f);
    }
    let f = e.native_fn("Observable", observable_ctor);
    if let Value::Obj(cid) = f {
        e.set_own_attrs(
            cid,
            "prototype",
            Value::Obj(p),
            crate::heap::PropAttrs::FIXED,
        );
        e.set_own(p, "constructor", f.clone(), false);
        let from = e.native_fn("from", observable_from);
        e.set_own(cid, "from", from, false);
    }
    let g = e.global;
    e.set_own(g, "Observable", f, true);
}
