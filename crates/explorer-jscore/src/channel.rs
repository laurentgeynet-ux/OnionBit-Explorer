// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! `MessageChannel`/`MessagePort` (JS-C81) — canal de messages
//! intra-contexte, la voie rapide d'ordonnancement des
//! frameworks : le scheduler React fait `port2.postMessage` /
//! `port1.onmessage` quand `MessageChannel` est présent (sinon
//! repli `setTimeout` — ordres de grandeur plus lent et sans
//! garantie de priorité).
//!
//! Modèle : `postMessage` sur un port enfile un message cloné
//! ([`WireValue`] — structured clone spec, `DataCloneError`
//! sur non-clonable) destiné au **port pair** ; la livraison
//! se fait au checkpoint de pompe (`run_microtasks`, même
//! frontière que `drain_pending_msgs`/`pump_workers`) en
//! tirant `onmessage` + les listeners `message`.
//!
//! Déviations documentées :
//! - `start()` no-op : les ports sont « entangled » et
//!   démarrés dès création (le démarrage paresseux spec n'a
//!   pas d'observable ici — `onmessage` posé démarre tout de
//!   suite, hors évaluation) ;
//! - pas de `transfer` de ports ni de `postMessage` cross-
//!   contexte (le canal est mono-realm) ;
//! - `MessageEvent` minimal : `type`/`data`/`target`/
//!   `currentTarget`/`ports` (vide) — pas de `source`/`origin`
//!   (intra-contexte, pas de fenêtre).

use crate::heap::{Host, JsResult};
use crate::value::Value;
use crate::Engine;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

fn arg(args: &[Value], i: usize) -> Value {
    args.get(i).copied().unwrap_or(Value::Undefined)
}

/// Message en file pour un port — le `WireValue` est décodé à
/// la livraison dans le contexte enregistreur.
pub(crate) struct PortMsg {
    /// Port destinataire (le pair du port posté).
    pub dest: Value,
    /// Donnée clonée (structured clone).
    pub data: crate::worker::WireValue,
    /// Realm enregistreur — livraison dans son contexte.
    pub ctx: crate::context::CtxId,
}

/// Méthodes lisibles du port (`typeof p.postMessage` =
/// `"function"`) — même trampoline que `dom_host` : bound =
/// `(nom, receveur)` → `Host::invoke`.
fn port_method_trampoline(e: &mut Engine, _this: Value, args: &[Value]) -> JsResult<Value> {
    let Some((Value::Str(m), rest)) = args.split_first() else {
        return Ok(Value::Undefined);
    };
    let Some(name) = e.heap.string(*m) else {
        return Ok(Value::Undefined);
    };
    let Some(Value::Obj(id)) = rest.first().copied() else {
        return Ok(Value::Undefined);
    };
    let Some(h) = e.host_of(id) else {
        return Ok(Value::Undefined);
    };
    Ok(h.invoke(e, Value::Obj(id), &name, &rest[1..])?
        .unwrap_or(Value::Undefined))
}

/// `MessagePort` — `onmessage`/`addEventListener('message')`,
/// `postMessage` clone-route vers le pair, `close` fige.
struct PortHost {
    /// L'autre port du canal (`Value` traquée pour le GC).
    peer: Cell<Value>,
    /// `close()` appelé — poste et réception deviennent no-op
    /// (spec : `postMessage` sur port désentrelacé est
    /// silencieux ; les messages en file vers un port fermé
    /// sont défaussés à la livraison).
    closed: Cell<bool>,
    /// Listeners `addEventListener` : `(type, fonction)` —
    /// capture ignoré (un port n'a pas de phase de capture).
    listeners: RefCell<Vec<(String, Value)>>,
}

impl PortHost {
    /// Snapshot des listeners `message` — `None` si le port
    /// est fermé (livraison défaussée, spec).
    fn msg_listeners(&self) -> Option<Vec<Value>> {
        if self.closed.get() {
            return None;
        }
        Some(
            self.listeners
                .borrow()
                .iter()
                .filter(|(t, _)| t == "message")
                .map(|(_, f)| *f)
                .collect(),
        )
    }
}

impl Host for PortHost {
    fn class_name(&self) -> &'static str {
        "MessagePort"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn trace(&self, out: &mut Vec<crate::value::ObjId>) {
        if let Value::Obj(id) = self.peer.get() {
            out.push(id);
        }
        for (_, f) in self.listeners.borrow().iter() {
            if let Value::Obj(id) = f {
                out.push(*id);
            }
        }
    }

    fn get(&self, e: &mut Engine, this: Value, key: &str) -> JsResult<Option<Value>> {
        match key {
            // `port.onmessage` : propre scriptée sinon `null`
            // (spec — même convention que les handlers `on*`).
            "onmessage" | "onmessageerror" => {
                if let Value::Obj(id) = this {
                    if let Some(v) = e.find_prop(id, key).and_then(|p| p.data_value()) {
                        return Ok(Some(v));
                    }
                }
                Ok(Some(Value::Null))
            }
            k
            @ ("postMessage" | "close" | "start" | "addEventListener" | "removeEventListener") => {
                let nv = e.str_val(k);
                Ok(Some(e.bound_native(
                    "port.method",
                    port_method_trampoline,
                    vec![nv, this],
                )))
            }
            _ => Ok(None),
        }
    }

    fn invoke(
        &self,
        e: &mut Engine,
        _this: Value,
        name: &str,
        args: &[Value],
    ) -> JsResult<Option<Value>> {
        match name {
            // Messages postés ici sont livrés au port pair.
            "postMessage" => {
                let data = e.wire_encode(arg(args, 0))?;
                if self.closed.get() {
                    return Ok(Some(Value::Undefined));
                }
                e.port_msgs.push_back(PortMsg {
                    dest: self.peer.get(),
                    data,
                    ctx: e.current_context(),
                });
                Ok(Some(Value::Undefined))
            }
            "close" => {
                self.closed.set(true);
                Ok(Some(Value::Undefined))
            }
            // Démarrage implicite à la création — no-op spec.
            "start" => Ok(Some(Value::Undefined)),
            "addEventListener" => {
                let ty = e.to_js_string(arg(args, 0))?;
                let f = arg(args, 1);
                if e.is_callable(f) {
                    self.listeners.borrow_mut().push((ty, f));
                    // Un listener posé démarre le port (spec) —
                    // déjà vrai (démarrés à la création).
                }
                Ok(Some(Value::Undefined))
            }
            "removeEventListener" => {
                let ty = e.to_js_string(arg(args, 0))?;
                let f = arg(args, 1);
                self.listeners
                    .borrow_mut()
                    .retain(|(t, g)| !(t == &ty && *g == f));
                Ok(Some(Value::Undefined))
            }
            _ => Ok(None),
        }
    }

    fn keys(&self, _e: &mut Engine, _this: Value) -> Vec<String> {
        vec![
            "postMessage".into(),
            "close".into(),
            "start".into(),
            "addEventListener".into(),
            "removeEventListener".into(),
            "onmessage".into(),
            "onmessageerror".into(),
        ]
    }
}

/// `MessageChannel` — coquille porteuse des deux ports.
struct ChannelHost;

impl Host for ChannelHost {
    fn class_name(&self) -> &'static str {
        "MessageChannel"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl Engine {
    /// File des messages de ports — vidée au checkpoint de
    /// pompe (même frontière que `postMessage` fenêtre).
    /// Champ déclaré dans `interp/mod.rs` à côté de
    /// `pending_msgs`.
    fn new_port_pair(&mut self) -> (Value, Value) {
        let h1 = Rc::new(PortHost {
            peer: Cell::new(Value::Undefined),
            closed: Cell::new(false),
            listeners: RefCell::new(Vec::new()),
        });
        let h2 = Rc::new(PortHost {
            peer: Cell::new(Value::Undefined),
            closed: Cell::new(false),
            listeners: RefCell::new(Vec::new()),
        });
        let p1 = self.new_host_object(h1.clone());
        let p2 = self.new_host_object(h2.clone());
        h1.peer.set(p2);
        h2.peer.set(p1);
        (p1, p2)
    }

    /// `new MessageChannel()` — `{port1, port2}` entrelacés.
    fn channel_ctor(e: &mut Engine, _this: Value, _args: &[Value]) -> JsResult<Value> {
        let (p1, p2) = e.new_port_pair();
        let ch = e.new_host_object(Rc::new(ChannelHost));
        let Value::Obj(cid) = ch else {
            return Ok(Value::Undefined);
        };
        e.set_own(cid, "port1", p1, true);
        e.set_own(cid, "port2", p2, true);
        Ok(ch)
    }

    /// Installe le constructeur `MessageChannel` global
    /// (appelé par `install_dom`, à côté de `Worker`).
    pub(crate) fn install_channel_ctor(&mut self) {
        let v = self.native_fn("MessageChannel", Self::channel_ctor);
        let g = self.global;
        self.set_own(g, "MessageChannel", v, false);
    }

    /// Livre les messages en file vers leurs ports — événement
    /// `message` sur `onmessage` puis listeners. Tourne au
    /// checkpoint de pompe ; les callbacks peuvent reposter —
    /// la boucle vidange jusqu'à épuisement (bornée par
    /// l'op-budget global).
    ///
    /// # Errors
    /// Propage la première exception d'un handler (même
    /// contrat que `dispatch_event`).
    pub(crate) fn drain_port_msgs(&mut self) -> JsResult<usize> {
        let mut n = 0usize;
        while let Some(m) = self.port_msgs.pop_front() {
            self.switch_to(m.ctx);
            if self.cur_ctx != m.ctx {
                continue;
            }
            let Value::Obj(id) = m.dest else {
                continue;
            };
            if !self.is_alive(id) {
                continue;
            }
            let Some(listeners) = self.host_of(id).and_then(|h| {
                h.as_any()
                    .downcast_ref::<PortHost>()
                    .and_then(PortHost::msg_listeners)
            }) else {
                continue;
            };
            let data = self.wire_decode(&m.data)?;
            let ev = self.new_object();
            let ty = self.str_val("message");
            self.set_own(ev, "type", ty, true);
            self.set_own(ev, "data", data, true);
            self.set_own(ev, "target", m.dest, true);
            self.set_own(ev, "currentTarget", m.dest, true);
            self.set_own(ev, "bubbles", Value::Bool(false), true);
            self.set_own(ev, "cancelable", Value::Bool(false), true);
            let ports = self.new_array(Vec::new());
            self.set_own(ev, "ports", ports, true);
            let evv = Value::Obj(ev);
            // `onmessage` d'abord (ordre spec : handler IDL +
            // listeners du même type forment une file unique —
            // ici l'IDL précède, déviation mineure).
            if let Some(cb) = self.get_prop(id, "onmessage") {
                if self.is_callable(cb) {
                    self.call_value(cb, m.dest, &[evv])?;
                }
            }
            for f in listeners {
                if self.is_callable(f) {
                    self.call_value(f, m.dest, &[evv])?;
                }
            }
            n += 1;
        }
        Ok(n)
    }
}
