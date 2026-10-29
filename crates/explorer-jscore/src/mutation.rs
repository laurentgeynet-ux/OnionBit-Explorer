// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! `MutationObserver` (JS-C26) — enregistrement sur le DOM
//! miroir et livraison des records au microtask checkpoint.
//!
//! Modèle : le `Dom` produit les `MutationRecord` à chaque
//! mutation instrumentée (`append`/`detach`/`insert_before`/
//! `replace_child`/`replace_children`/`set_attr`/`remove_attr`/
//! `set_text`) et les empile par id d'observer ; le moteur
//! draine la file en fin de checkpoint microtask et appelle
//! chaque callback **une fois** avec son lot, ordre de
//! production conservé (spec : « notify mutation observers »).

use std::collections::HashMap;
use std::rc::Rc;

use crate::dom::{MutObsReg, MutationRecord, NodeId};
use crate::dom_host::{node_for_in, node_value, DomRef, NodeCache};
use crate::heap::{Host, JsResult, ObjKind};
use crate::value::{ObjId, Value};
use crate::Engine;

/// État moteur des `MutationObserver` — les records eux-mêmes
/// vivent dans la file du `Dom` (survivent aux wrappers GC).
#[derive(Default)]
pub(crate) struct MutObsState {
    /// Prochain id d'observer alloué.
    next_id: u64,
    /// Observers vivants : `id → (callback, wrapper JS)`.
    cbs: HashMap<u64, (Value, ObjId)>,
}

impl MutObsState {
    /// Racines GC : callbacks et wrappers de tous les observers
    /// enregistrés — le `MutObsHost` ne référence pas le
    /// callback (corrélé par `id`) et un observer déconnecté
    /// peut être ré-armé par `observe` : son cb doit survivre.
    pub(crate) fn root_ids(&self, ids: &mut Vec<ObjId>) {
        for (cb, wrapper) in self.cbs.values() {
            crate::heap::push_value(ids, *cb);
            ids.push(*wrapper);
        }
    }
}

/// Host object `MutationObserver` — `id` corrèle les
/// registrations du `Dom` et le callback ici.
struct MutObsHost {
    dom: DomRef,
    cache: NodeCache,
    id: u64,
}

impl Engine {
    /// Installe le constructeur `MutationObserver` — appelé par
    /// `install_dom` (le miroir est requis à l'usage, pas à
    /// l'installation).
    pub(crate) fn install_mutation_observer(&mut self) {
        let v = self.native_fn("MutationObserver", mutobs_ctor);
        let g = self.global;
        self.set_own(g, "MutationObserver", v, false);
    }

    /// Livraison des records en file : un appel par observer
    /// notifié, dans l'ordre de production — les callbacks
    /// peuvent muter (nouveaux records) ou enqueuer des
    /// microtasks, le checkpoint reboucle (borné par le
    /// budget ops). `true` = au moins un observer notifié.
    ///
    /// # Errors
    /// Propage la première exception d'un callback (comme un
    /// listener — l'appelant la traite comme un échec de script).
    pub(crate) fn deliver_mutations(&mut self) -> JsResult<bool> {
        let Some((dom, cache)) = self.dom_ctx.clone() else {
            return Ok(false);
        };
        let recs = dom.borrow_mut().drain_mutations();
        if recs.is_empty() {
            return Ok(false);
        }
        // Groupage par observer, ordre de première apparition.
        let mut order: Vec<u64> = Vec::new();
        let mut by_obs: HashMap<u64, Vec<MutationRecord>> = HashMap::new();
        for (id, r) in recs {
            by_obs
                .entry(id)
                .or_insert_with(|| {
                    order.push(id);
                    Vec::new()
                })
                .push(r);
        }
        for id in order {
            let Some(&(cb, wrapper)) = self.mutobs.cbs.get(&id) else {
                continue;
            };
            let Some(recs) = by_obs.get(&id) else {
                continue;
            };
            let arr = records_value(self, &dom, &cache, recs);
            self.call_value(cb, Value::Obj(wrapper), &[arr, Value::Obj(wrapper)])?;
        }
        Ok(true)
    }
}

/// `new MutationObserver(callback)` — callback requis.
fn mutobs_ctor(e: &mut Engine, _this: Value, args: &[Value]) -> JsResult<Value> {
    let cb = args.first().copied().unwrap_or(Value::Undefined);
    if !e.is_callable(cb) {
        return Err(e.type_error("MutationObserver: callback requis"));
    }
    let Some((dom, cache)) = e.dom_ctx.clone() else {
        return Err(e.type_error("MutationObserver: document non installé"));
    };
    e.mutobs.next_id += 1;
    let id = e.mutobs.next_id;
    let v = e.new_host_object(Rc::new(MutObsHost { dom, cache, id }));
    if let Value::Obj(oid) = v {
        e.mutobs.cbs.insert(id, (cb, oid));
    }
    Ok(v)
}

/// Propriété d'options — `undefined` hors objet.
fn opt(e: &mut Engine, opts: Value, key: &str) -> Value {
    match opts {
        Value::Obj(o) => e.get_prop(o, key).unwrap_or(Value::Undefined),
        _ => Value::Undefined,
    }
}

/// `opt` + truthiness en deux temps (`to_boolean` est `&self`,
/// `get_prop` `&mut self` — l'argument est évalué d'abord).
fn optb(e: &mut Engine, opts: Value, key: &str) -> bool {
    let v = opt(e, opts, key);
    e.to_boolean(v)
}

/// `NodeId`s → array JS de wrappers nœud.
fn nodes_value(e: &mut Engine, dom: &DomRef, cache: &NodeCache, nodes: &[NodeId]) -> Value {
    let vals: Vec<Value> = nodes
        .iter()
        .map(|&n| node_value(e, dom, cache, n))
        .collect();
    e.new_array(vals)
}

/// `MutationRecord` (file `Dom`) → objet JS.
fn record_value(e: &mut Engine, dom: &DomRef, cache: &NodeCache, r: &MutationRecord) -> Value {
    let o = e.new_object();
    let ty = e.str_val(r.ty);
    e.set_own(o, "type", ty, true);
    let target = node_value(e, dom, cache, r.target);
    e.set_own(o, "target", target, true);
    let added = nodes_value(e, dom, cache, &r.added);
    e.set_own(o, "addedNodes", added, true);
    let removed = nodes_value(e, dom, cache, &r.removed);
    e.set_own(o, "removedNodes", removed, true);
    for (k, sib) in [
        ("previousSibling", r.prev_sibling),
        ("nextSibling", r.next_sibling),
    ] {
        let v = sib.map_or(Value::Null, |n| node_value(e, dom, cache, n));
        e.set_own(o, k, v, true);
    }
    let name = r
        .attr_name
        .as_ref()
        .map_or(Value::Null, |n| e.str_val(n.clone()));
    e.set_own(o, "attributeName", name, true);
    e.set_own(o, "attributeNamespace", Value::Null, true);
    let old = r
        .old_value
        .as_ref()
        .map_or(Value::Null, |v| e.str_val(v.clone()));
    e.set_own(o, "oldValue", old, true);
    Value::Obj(o)
}

/// Lot de records → `MutationRecord[]` (ordre conservé).
fn records_value(
    e: &mut Engine,
    dom: &DomRef,
    cache: &NodeCache,
    recs: &[MutationRecord],
) -> Value {
    let vals: Vec<Value> = recs
        .iter()
        .map(|r| record_value(e, dom, cache, r))
        .collect();
    e.new_array(vals)
}

impl Host for MutObsHost {
    fn class_name(&self) -> &'static str {
        "MutationObserver"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn trace(&self, out: &mut Vec<ObjId>) {
        crate::dom_host::trace_node_cache(&self.cache, out);
    }

    fn invoke(
        &self,
        e: &mut Engine,
        _this: Value,
        name: &str,
        args: &[Value],
    ) -> JsResult<Option<Value>> {
        match name {
            "observe" => {
                let target = args.first().copied().unwrap_or(Value::Undefined);
                let Some(node) = node_for_in(e, target, &self.dom) else {
                    return Err(e.type_error("observe: nœud requis"));
                };
                let opts = args.get(1).copied().unwrap_or(Value::Undefined);
                let child_list = optb(e, opts, "childList");
                // Spec : `attributeFilter`/`attributeOldValue`
                // impliquent `attributes` ; idem `characterData`.
                let attribute_filter = match opt(e, opts, "attributeFilter") {
                    Value::Obj(o) => match e.heap.obj(o).map(|ob| &ob.kind) {
                        Some(ObjKind::Array(elems)) => {
                            let elems = elems.clone();
                            Some(
                                elems
                                    .iter()
                                    .filter_map(|v| e.to_js_string(*v).ok())
                                    .collect(),
                            )
                        }
                        _ => None,
                    },
                    _ => None,
                };
                let attributes = optb(e, opts, "attributes")
                    || optb(e, opts, "attributeOldValue")
                    || attribute_filter.is_some();
                let character_data =
                    optb(e, opts, "characterData") || optb(e, opts, "characterDataOldValue");
                if !(child_list || attributes || character_data) {
                    return Err(e.type_error("observe: au moins un type à observer requis"));
                }
                let reg = MutObsReg {
                    id: self.id,
                    child_list,
                    attributes,
                    character_data,
                    subtree: optb(e, opts, "subtree"),
                    attribute_filter,
                    attribute_old_value: optb(e, opts, "attributeOldValue"),
                    character_data_old_value: optb(e, opts, "characterDataOldValue"),
                };
                self.dom.borrow_mut().observe_mutations(node, reg);
                Ok(Some(Value::Undefined))
            }
            "disconnect" => {
                self.dom.borrow_mut().disconnect_observer(self.id);
                Ok(Some(Value::Undefined))
            }
            "takeRecords" => {
                let recs = self.dom.borrow_mut().take_mutation_records(self.id);
                Ok(Some(records_value(e, &self.dom, &self.cache, &recs)))
            }
            _ => Ok(None),
        }
    }
}
