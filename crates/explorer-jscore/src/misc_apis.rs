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
    /// `Animation.prototype` paresseux + registre des
    /// animations vivantes (CAP-187) — document-local.
    anim_proto: Option<ObjId>,
    anims: Vec<Rc<RefCell<AnimData>>>,
    /// `ViewTransition.prototype` paresseux (CAP-187).
    vt_proto: Option<ObjId>,
    /// `XPathResult.prototype` paresseux (CAP-188).
    xr_proto: Option<ObjId>,
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
            self.anim_proto,
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
            crate::heap::push_value(ids, *d);
        }
        if let Some(w) = self.sel_wrapper {
            ids.push(w);
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
    e.set_own(p, crate::interp::SYM_TO_STRING_TAG, tag, false);
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
            o.proto = Value::Obj(p);
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
        let t = e.str_val("message");
        e.set_own(o, "type", t, true);
        e.set_own(o, "target", this, true);
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
        if e.is_callable(f) {
            e.call_value(f, this, &[ev])?;
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
        if e.is_callable(f) {
            e.call_value(f, this, &[ev])?;
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
    e.set_own(p, crate::interp::SYM_TO_STRING_TAG, tag, false);
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
            o.proto = Value::Obj(p);
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
        if e.is_callable(*cb) {
            e.call_value(*cb, Value::Undefined, &[denied])?;
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
    e.set_own(p, crate::interp::SYM_TO_STRING_TAG, tag, false);
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
            o.proto = Value::Obj(p);
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
            "collapse" | "collapseToStart" | "collapseToEnd" | "extend" => {
                let d = self.data.borrow();
                let node = if matches!(name, "collapse" | "extend") {
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
                let off = if matches!(name, "collapse" | "extend") {
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
            "modify" => Ok(Some(Value::Undefined)),
            _ => Ok(None),
        }
    }

    fn trace(&self, _out: &mut Vec<ObjId>) {}
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
    let v = arg(args, 0);
    let message = e.to_js_string(v).unwrap_or_default();
    // 1. `window.onerror` legacy — 5 arguments (spec HTML) ;
    //    retour truthy = erreur « gérée », pas de rapport.
    let g = e.global;
    if let Some(f) = e.get_prop(g, "onerror") {
        if e.is_callable(f) {
            let m = e.str_val(message.clone());
            let src = e.str_val(e.hooks.page_url.clone());
            let ret = e.call_value(
                f,
                Value::Obj(g),
                &[m, src, Value::Num(0.0), Value::Num(0.0), v],
            )?;
            if e.to_boolean(ret) {
                return Ok(Value::Undefined);
            }
        }
    }
    // 2. Listeners `error` de la racine — ErrorEvent réel via
    //    la propagation DOM (`preventDefault` = géré).
    let mut prevented = false;
    if let Some((dom, cache)) = e.dom_ctx.clone() {
        let root = dom.borrow().root;
        let o = e.new_object();
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
    Ok(Value::Undefined)
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
        e.set_own(cid, "prototype", Value::Obj(p), false);
        e.set_own(p, "constructor", bc, false);
    }
    e.set_own(g, "BroadcastChannel", bc, false);
    // `Notification` — stub honnête (`permission` 'default',
    // `requestPermission` → 'denied', instances inertes).
    let nf = e.native_fn("Notification", notif_ctor as crate::heap::NativeFn);
    if let Value::Obj(cid) = nf {
        let p = notif_proto(e);
        e.set_own(cid, "prototype", Value::Obj(p), false);
        e.set_own(p, "constructor", nf, false);
        // Statiques : `permission` own-prop figée `'default'`
        // (honnête — le grant n'est pas atteignable sans UI,
        // jamais `'granted'` mensonger), `requestPermission`,
        // `maxActions` (0 — jamais d'actions sans UI).
        let d = e.str_val("default");
        e.set_own(cid, "permission", d, true);
        e.add_method(cid, "requestPermission", notif_request_permission);
        e.set_own(cid, "maxActions", Value::Num(0.0), true);
    }
    e.set_own(g, "Notification", nf, false);
    // `reportError` — réel (dispatch + hook).
    let re = e.native_fn("reportError", report_error as crate::heap::NativeFn);
    e.set_own(g, "reportError", re, false);
    // `getSelection` — DOM requis (Selection = nœuds miroir) ;
    // posé par install_dom directement.
    if e.dom_ctx.is_some() {
        let gs = e.native_fn("getSelection", get_selection as crate::heap::NativeFn);
        e.set_own(g, "getSelection", gs, false);
    }
}

/// `args[i]` ou `Undefined` (même convention que `xhr::arg`).
fn arg(args: &[Value], i: usize) -> Value {
    args.get(i).copied().unwrap_or(Value::Undefined)
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

/// `Range.prototype` paresseux (une fois par engine).
fn range_proto(e: &mut Engine) -> ObjId {
    if let Some(p) = e.misc.range_proto {
        return p;
    }
    let p = e.new_object();
    let tag = e.str_val("Range");
    e.set_own(p, crate::interp::SYM_TO_STRING_TAG, tag, false);
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
            o.proto = Value::Obj(p);
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
                | "deleteContents"
                | "insertNode"
                | "cloneRange"
                | "detach"
                | "toString"
                | "getBoundingClientRect"
                | "getClientRects"
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
        match tw_accept(e, dom, cache, n, what, filter) {
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

/// `TreeWalker.prototype` paresseux.
fn tw_proto(e: &mut Engine) -> ObjId {
    if let Some(p) = e.misc.tw_proto {
        return p;
    }
    let p = e.new_object();
    let tag = e.str_val("TreeWalker");
    e.set_own(p, crate::interp::SYM_TO_STRING_TAG, tag, false);
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
            o.proto = Value::Obj(p);
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
            "filter" => d.filter,
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
        let found = match name {
            "nextNode" => {
                let (root, what, filter, cur) = {
                    let d = self.data.borrow();
                    (d.root, d.what, d.filter, d.cur)
                };
                tw_next(e, &self.dom, &self.cache, what, filter, root, cur)
            }
            "firstChild" => {
                let (root, what, filter) = {
                    let d = self.data.borrow();
                    (d.root, d.what, d.filter)
                };
                tw_next(e, &self.dom, &self.cache, what, filter, root, root)
            }
            // Borné CAP-182 : navigation inverse/fratrie non
            // portée — `null` honnête plutôt que parcours faux.
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
    e.set_own(p, crate::interp::SYM_TO_STRING_TAG, tag, false);
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
            o.proto = Value::Obj(p);
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
            "filter" => d.filter,
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
                    (d.root, d.what, d.filter, d.cur, d.started)
                };
                let found = if started {
                    tw_next(e, &self.dom, &self.cache, what, filter, root, cur)
                } else {
                    self.data.borrow_mut().started = true;
                    if tw_accept(e, &self.dom, &self.cache, root, what, filter) == 0 {
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
                    (d.root, d.what, d.filter, d.cur)
                };
                loop {
                    let Some(p) = tw_prev(&self.dom, n, root) else {
                        return Ok(Some(Value::Null));
                    };
                    n = p;
                    if tw_accept(e, &self.dom, &self.cache, n, what, filter) == 0 {
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
    e.set_own(o, crate::interp::SYM_TO_STRING_TAG, tag, false);
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
                obj.proto = proto;
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
    let Some(rid) = sheet_rules(e, this) else {
        return Err(e.type_error("insertRule: no cssRules"));
    };
    let rule = e.new_object();
    let c = e.str_val(&text);
    e.set_own(rule, "cssText", c, true);
    e.set_own(rule, "type", Value::Num(1.0), true);
    let Some(o) = e.heap.obj_mut(rid) else {
        return Err(e.type_error("insertRule: detached"));
    };
    let at = {
        if let crate::heap::ObjKind::Array(elems) = &mut o.kind {
            let at = want.min(elems.len());
            elems.insert(at, Value::Obj(rule));
            at
        } else {
            0
        }
    };
    Ok(Value::Num(at as f64))
}

/// `deleteRule(index)` — borne `idx < len` sinon `IndexSizeError`.
pub(crate) fn sheet_delete_rule(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let idx = e.to_number(arg(args, 0))? as usize;
    let Some(rid) = sheet_rules(e, this) else {
        return Err(e.type_error("deleteRule: no cssRules"));
    };
    let Some(o) = e.heap.obj_mut(rid) else {
        return Err(e.type_error("deleteRule: detached"));
    };
    if let crate::heap::ObjKind::Array(elems) = &mut o.kind {
        if idx < elems.len() {
            elems.remove(idx);
        }
    }
    Ok(Value::Undefined)
}

/// `replace(text)` → promesse résolue `this` (spec :
/// `CSSStyleSheet` — borné : texte stocké `__text`, règles
/// vidées ; pas de parse).
pub(crate) fn sheet_replace(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let text = e.to_js_string(arg(args, 0))?;
    sheet_set_text(e, this, &text)?;
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
    if let Some(rid) = sheet_rules(e, this) {
        if let Some(o) = e.heap.obj_mut(rid) {
            if let crate::heap::ObjKind::Array(elems) = &mut o.kind {
                elems.clear();
            }
        }
    }
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
    let tag = e.str_val("SharedWorker");
    e.set_own(o, crate::interp::SYM_TO_STRING_TAG, tag, false);
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
    /// Handlers `onfinish`/`oncancel`.
    onfinish: Value,
    oncancel: Value,
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
    e.set_own(p, crate::interp::SYM_TO_STRING_TAG, tag, false);
    e.misc.anim_proto = Some(p);
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
    let p = anim_proto(e);
    if let Value::Obj(id) = v {
        if let Some(o) = e.heap.obj_mut(id) {
            o.proto = Value::Obj(p);
        }
        data.borrow_mut().wrapper = Some(id);
    }
    v
}

/// `el.animate(keyframes, duration|options)` (CAP-187) —
/// `Animation` machine à états : `playState='running'`,
/// `ready` résolu, `finished` pendante (résolue par
/// `finish()` ou annulée par `cancel()` — pas de timeline
/// réelle, borné).
pub(crate) fn animate(e: &mut Engine, t: Value, args: &[Value]) -> JsResult<Value> {
    let Some(n) = node_for(e, t) else {
        return Err(e.type_error("animate: not an Element"));
    };
    let _ = args; // keyframes/options — non lus (borné : pas de keyframe parsing)
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
    }));
    e.misc.anims.push(data.clone());
    Ok(anim_wrapper(e, &data))
}

/// `el.getAnimations()`/`document.getAnimations()`
/// (CAP-187) — wrappers des animations vivantes (le
/// filtre élément borne aux animations de ce nœud ;
/// document → toutes — borné : pas de sous-arbre).
pub(crate) fn get_animations(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let scope = node_for(e, this);
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
        "Animation"
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

    fn get(&self, e: &mut Engine, _t: Value, key: &str) -> JsResult<Option<Value>> {
        let d = self.data.borrow();
        Ok(Some(match key {
            "playState" => return Ok(Some(e.str_val(d.state))),
            "playbackRate" => Value::Num(d.rate),
            "pending" => Value::Bool(false),
            "currentTime" | "effect" | "timeline" => Value::Null,
            "finished" => Value::Obj(d.finished),
            "ready" => Value::Obj(d.ready),
            "onfinish" => d.onfinish,
            "oncancel" => d.oncancel,
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
                self.data.borrow_mut().state = "running";
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
                    (d.finished, d.onfinish, d.wrapper)
                };
                // `finished` résout avec l'objet `Animation`
                // (spec) — pas la promesse (auto-résolution
                // → rejet TypeError).
                e.resolve_promise(pid, w.map_or(Value::Undefined, Value::Obj));
                if e.is_callable(onf) {
                    let ev = e.new_object();
                    e.call_value(onf, t, &[Value::Obj(ev)])?;
                }
                Ok(Some(Value::Undefined))
            }
            "cancel" => {
                let (pid, onc) = {
                    let mut d = self.data.borrow_mut();
                    d.state = "idle";
                    (d.finished, d.oncancel)
                };
                let m = e.str_val("AbortError: animation canceled");
                e.reject_promise(pid, m);
                if e.is_callable(onc) {
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
        obj.proto = Value::Obj(p);
    }
    if e.is_callable(cb) {
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
    e.set_own(p, crate::interp::SYM_TO_STRING_TAG, tag, false);
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
}

/// Pas parsé : nom + prédicats `[@a]`, `[@a='v']`, `[n]`.
struct ParsedStep {
    step: Step,
    pred_attr: Option<String>,
    pred_val: Option<String>,
    pred_nth: Option<usize>,
}

/// Parse d'un pas — `None` sur syntaxe non portée
/// (honnête : `TypeError` remonté).
fn parse_step(raw: &str) -> Option<ParsedStep> {
    let (name, preds) = raw.find('[').map_or((raw, ""), |i| (&raw[..i], &raw[i..]));
    let step = match name {
        "*" => Step::Tag("*".into()),
        "text()" => Step::Text,
        "node()" => Step::Any,
        "." => Step::SelfNode,
        ".." => Step::Parent,
        _ => Step::Tag(name.to_ascii_lowercase()),
    };
    let mut s = ParsedStep {
        step,
        pred_attr: None,
        pred_val: None,
        pred_nth: None,
    };
    let mut rest = preds;
    while rest.starts_with('[') {
        let end = rest.find(']')?;
        let p = rest[1..end].trim();
        if let Some(a) = p.strip_prefix('@') {
            if let Some((k, v)) = a.split_once('=') {
                s.pred_attr = Some(k.trim().into());
                s.pred_val = Some(v.trim().trim_matches('\'').trim_matches('"').into());
            } else {
                s.pred_attr = Some(a.trim().into());
            }
        } else if let Ok(n) = p.parse::<usize>() {
            s.pred_nth = Some(n.max(1));
        } else {
            return None;
        }
        rest = &rest[end + 1..];
    }
    if !rest.is_empty() {
        return None;
    }
    Some(s)
}

/// `true` si le nœud passe le pas (nom + prédicat attr).
fn xp_match(d: &crate::dom::Dom, n: NodeId, ps: &ParsedStep) -> bool {
    let node = &d.nodes[n as usize];
    let ok = match &ps.step {
        Step::Tag(t) => t == "*" || (node.kind == crate::dom::NodeKind::Element && node.tag == *t),
        Step::Text => node.kind == crate::dom::NodeKind::Text,
        Step::Any | Step::SelfNode | Step::Parent => true,
    };
    if !ok {
        return false;
    }
    if let Some(a) = &ps.pred_attr {
        let Some(v) = d.attr(n, a) else {
            return false;
        };
        if let Some(want) = &ps.pred_val {
            if v != want {
                return false;
            }
        }
    }
    true
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

/// Un niveau de pas — `desc` = `//` (descendants du set
/// courant), sinon enfants directs.
fn xp_apply(d: &crate::dom::Dom, set: &[NodeId], ps: &ParsedStep, desc: bool) -> Vec<NodeId> {
    let mut out = Vec::new();
    for &n in set {
        match ps.step {
            Step::SelfNode => out.push(n),
            Step::Parent => {
                if let Some(p) = d.parent(n) {
                    out.push(p);
                }
            }
            _ => {
                if desc {
                    for m in xp_descendants(d, n) {
                        if xp_match(d, m, ps) {
                            out.push(m);
                        }
                    }
                } else {
                    for m in d.child_nodes(n) {
                        if xp_match(d, m, ps) {
                            out.push(m);
                        }
                    }
                }
            }
        }
    }
    if let Some(k) = ps.pred_nth {
        // Borné : n-ième de l'ensemble global (spec : n-ième
        // par parent — documenté).
        out = out
            .into_iter()
            .nth(k.saturating_sub(1))
            .into_iter()
            .collect();
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
    e.set_own(o, crate::interp::SYM_TO_STRING_TAG, tag, false);
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
/// résolveur (le documentElement ici — notre XPath ne résout pas
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
            let mut set = vec![ctx];
            let mut rest = path.as_str();
            while !rest.is_empty() {
                // `.`/`..` préfixe — `self`/`parent::node()`.
                if let Some(r) = rest.strip_prefix("..") {
                    set = xp_apply(
                        &d,
                        &set,
                        &ParsedStep {
                            step: Step::Parent,
                            pred_attr: None,
                            pred_val: None,
                            pred_nth: None,
                        },
                        false,
                    );
                    rest = r;
                    continue;
                }
                if let Some(r) = rest.strip_prefix('.') {
                    rest = r;
                    continue;
                }
                let (step_raw, desc, next) = if let Some(r) = rest.strip_prefix("//") {
                    let i = r.find('/').unwrap_or(r.len());
                    (&r[..i], true, &r[i..])
                } else if let Some(r) = rest.strip_prefix('/') {
                    let i = r.find('/').unwrap_or(r.len());
                    (&r[..i], false, &r[i..])
                } else {
                    (rest, false, "")
                };
                if step_raw.is_empty() {
                    return Err(e.type_error("evaluate: unsupported XPath"));
                }
                let Some(ps) = parse_step(step_raw) else {
                    return Err(e.type_error("evaluate: unsupported XPath"));
                };
                set = xp_apply(&d, &set, &ps, desc);
                rest = next;
            }
            nodes = set;
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
            o.proto = Value::Obj(p);
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
    e.set_own(p, crate::interp::SYM_TO_STRING_TAG, tag, false);
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
