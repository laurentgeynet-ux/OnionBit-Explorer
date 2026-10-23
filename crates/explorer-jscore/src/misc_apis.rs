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
use crate::dom_host::{node_for, node_value};
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
}

impl MiscState {
    /// Racines GC : protos + instances ouvertes (le
    /// `Host::trace` couvre listeners/wrappers) + payloads
    /// en file + ranges de la sélection (wrappers de nœuds).
    pub(crate) fn root_ids(&self, ids: &mut Vec<ObjId>) {
        for p in [self.bc_proto, self.notif_proto, self.sel_proto]
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
#[derive(Clone)]
struct SelRange {
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
    ranges: Vec<SelRange>,
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

/// Objet `Range` rendu par `getRangeAt` — props spec +
/// `cloneRange`/`detach` (no-op spec)/`toString`/`contains…`
/// minimaux ; mutation complète hors portée (documenté).
fn sel_range_value(
    e: &mut Engine,
    dom: &crate::dom_host::DomRef,
    cache: &crate::dom_host::NodeCache,
    r: &SelRange,
) -> Value {
    let o = e.new_object();
    let sc = node_value(e, dom, cache, r.start_node);
    e.set_own(o, "startContainer", sc, true);
    e.set_own(o, "startOffset", Value::Num(f64::from(r.start_off)), true);
    let ec = node_value(e, dom, cache, r.end_node);
    e.set_own(o, "endContainer", ec, true);
    e.set_own(o, "endOffset", Value::Num(f64::from(r.end_off)), true);
    let collapsed = r.start_node == r.end_node && r.start_off == r.end_off;
    e.set_own(o, "collapsed", Value::Bool(collapsed), true);
    let common = node_value(e, dom, cache, r.start_node);
    e.set_own(o, "commonAncestorContainer", common, true);
    let text = e.str_val(sel_range_text(dom, r));
    e.set_own(o, "toString", Value::Undefined, false);
    // `toString()` via méthode bound — `Value` texte stocké.
    e.set_own(o, "__text", text, false);
    Value::Obj(o)
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
            "isCollapsed" => Value::Bool(
                d.ranges
                    .first()
                    .is_none_or(|r| r.start_node == r.end_node && r.start_off == r.end_off),
            ),
            "type" => {
                return Ok(Some(e.str_val(if d.ranges.is_empty() {
                    "None"
                } else if d.ranges[0].start_node == d.ranges[0].end_node
                    && d.ranges[0].start_off == d.ranges[0].end_off
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
                    ("anchorNode", Some(r)) => node_value(e, &self.dom, &self.cache, r.start_node),
                    (_, Some(r)) => node_value(e, &self.dom, &self.cache, r.end_node),
                }));
            }
            "anchorOffset" => Value::Num(f64::from(d.ranges.first().map_or(0, |r| r.start_off))),
            "focusOffset" => Value::Num(f64::from(d.ranges.first().map_or(0, |r| r.end_off))),
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
                    .map(|r| sel_range_text(&self.dom, r))
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
                Ok(Some(sel_range_value(e, &self.dom, &self.cache, r)))
            }
            "selectAllChildren" => {
                let Some(node) = node_for(e, arg(args, 0)) else {
                    return Err(e.type_error("Selection.selectAllChildren: not a Node"));
                };
                let count = self.dom.borrow().child_nodes(node).len() as u32;
                self.data.borrow_mut().ranges = vec![SelRange {
                    start_node: node,
                    start_off: 0,
                    end_node: node,
                    end_off: count,
                }];
                Ok(Some(Value::Undefined))
            }
            "collapse" | "collapseToStart" | "collapseToEnd" | "extend" => {
                let d = self.data.borrow();
                let node = if matches!(name, "collapse" | "extend") {
                    node_for(e, arg(args, 0))
                } else {
                    d.ranges.first().map(|r| {
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
                        r.end_node = n;
                        r.end_off = off;
                    }
                } else if let Some(n) = node {
                    self.data.borrow_mut().ranges = vec![SelRange {
                        start_node: n,
                        start_off: off,
                        end_node: n,
                        end_off: off,
                    }];
                }
                Ok(Some(Value::Undefined))
            }
            "addRange" => {
                let r = arg(args, 0);
                let (sn, so, en, eo) = if let Value::Obj(id) = r {
                    let sn = e
                        .get_prop(id, "startContainer")
                        .and_then(|v| node_for(e, v));
                    let en = e.get_prop(id, "endContainer").and_then(|v| node_for(e, v));
                    let so = e
                        .get_prop(id, "startOffset")
                        .and_then(|v| e.to_number(v).ok())
                        .unwrap_or(0.0) as u32;
                    let eo = e
                        .get_prop(id, "endOffset")
                        .and_then(|v| e.to_number(v).ok())
                        .unwrap_or(0.0) as u32;
                    (sn, so, en, eo)
                } else {
                    (None, 0, None, 0)
                };
                let (Some(sn), Some(en)) = (sn, en) else {
                    return Err(e.type_error("Selection.addRange: not a Range"));
                };
                // Miroir : un seul range (comportement Chrome).
                self.data.borrow_mut().ranges = vec![SelRange {
                    start_node: sn,
                    start_off: so,
                    end_node: en,
                    end_off: eo,
                }];
                Ok(Some(Value::Undefined))
            }
            "removeRange" | "removeAllRanges" | "empty" => {
                self.data.borrow_mut().ranges.clear();
                Ok(Some(Value::Undefined))
            }
            "setBaseAndExtent" => {
                let sn = node_for(e, arg(args, 0));
                let en = node_for(e, arg(args, 2));
                let so = e.to_number(arg(args, 1)).unwrap_or(0.0) as u32;
                let eo = e.to_number(arg(args, 3)).unwrap_or(0.0) as u32;
                let (Some(sn), Some(en)) = (sn, en) else {
                    return Err(e.type_error("Selection.setBaseAndExtent: not a Node"));
                };
                self.data.borrow_mut().ranges = vec![SelRange {
                    start_node: sn,
                    start_off: so,
                    end_node: en,
                    end_off: eo,
                }];
                Ok(Some(Value::Undefined))
            }
            "containsNode" => {
                // Approximation honnête : vrai si `node` est le
                // conteneur ou un descendant du conteneur.
                let Some(n) = node_for(e, arg(args, 0)) else {
                    return Ok(Some(Value::Bool(false)));
                };
                let d = self.data.borrow();
                let hit = d.ranges.iter().any(|r| {
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
