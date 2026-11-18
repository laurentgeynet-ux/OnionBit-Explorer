// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! `Map`/`Set` and lazy iterators (JS-7e).
//!
//! Storage is an insertion-ordered `Vec` — spec iteration order is
//! observable and key lookup is `SameValueZero` (`===` plus NaN).
//! Iterators are `ObjKind::Iter` snapshots consumed by `next()`
//! (spec semantics: live iterators — ours freeze a snapshot, a
//! documented deviation).

use crate::heap::{IterHelperState, IterOp, JsResult, ObjKind, Object, Thrown};
use crate::interp::SYM_ITERATOR;
use crate::value::{ObjId, Value};
use crate::Engine;
use std::collections::VecDeque;

fn arg(args: &[Value], i: usize) -> Value {
    args.get(i).cloned().unwrap_or(Value::Undefined)
}

/// `SameValueZero` — `===` plus `NaN` equals `NaN`.
fn svz(h: &crate::heap::Heap, a: &Value, b: &Value) -> bool {
    crate::interp::strict_eq(a, b, h)
        || matches!((a, b), (Value::Num(x), Value::Num(y)) if x.is_nan() && y.is_nan())
}

/// Snapshot des entrées d'une `Map` — clone du Vec backing
/// (sémantique d'itérateur gelé, déviation documentée). Gardé
/// sur la taille totale : le clone commet `len×16` en UNE alloc
/// (§B-59 — un `keys()`/`entries()`/`forEach` sur Map géante).
fn map_entries(e: &mut Engine, id: ObjId) -> JsResult<Vec<(Value, Value)>> {
    let len = e.heap.obj(id).map_or(0, |o| match &o.kind {
        ObjKind::Map(m) => m.len(),
        _ => 0,
    });
    e.alloc_guard(len.saturating_mul(std::mem::size_of::<(Value, Value)>()))?;
    Ok(match e.heap.obj(id).map(|o| &o.kind) {
        Some(ObjKind::Map(m)) => m.clone(),
        _ => Vec::new(),
    })
}

fn set_items(e: &mut Engine, id: ObjId) -> JsResult<Vec<Value>> {
    let len = e.heap.obj(id).map_or(0, |o| match &o.kind {
        ObjKind::Set(s) => s.len(),
        _ => 0,
    });
    e.alloc_guard(len.saturating_mul(std::mem::size_of::<Value>()))?;
    Ok(match e.heap.obj(id).map(|o| &o.kind) {
        Some(ObjKind::Set(s)) => s.clone(),
        _ => Vec::new(),
    })
}

/// `{value, done}` iterator result.
fn iter_result(e: &mut Engine, value: Value, done: bool) -> Value {
    let o = e.new_object();
    let v = value;
    let d = Value::Bool(done);
    e.set_own(o, "value", v, true);
    e.set_own(o, "done", d, true);
    Value::Obj(o)
}

/// New lazy iterator object over a snapshot.
/// `next()`/`[Symbol.iterator]` iterator over a snapshot — shared
/// by Map/Set and now `Array.prototype.entries/keys/values`.
/// `proto` = le prototype concret spec de la famille
/// (`%ArrayIteratorPrototype%`, `%MapIteratorPrototype%`…), lui-
/// même chaîné sur `%IteratorPrototype%`.
pub(crate) fn iter_obj(e: &mut Engine, items: Vec<Value>, proto: ObjId) -> Value {
    let proto = Value::Obj(proto);
    Value::Obj(e.heap.alloc_obj(Object {
        proto,
        props: Vec::new(),
        idx: std::cell::OnceCell::new(),
        shape: std::cell::Cell::new(0),
        kind: ObjKind::Iter {
            items,
            idx: 0,
            ret: Value::Undefined,
        },
        extensible: true,
        elem_integrity: 0,
        lookups: std::cell::Cell::new(0),
        flags: std::cell::Cell::new(0),
        args_map: None,
    }))
}

// ------------------------------------------------------------------
// ctors
// ------------------------------------------------------------------

fn map_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    map_new(e, args, e.protos.map)
}

/// `new WeakMap(...)` — proto dédié (`@@toStringTag` `WeakMap`),
/// internals `ObjKind::Map` identiques (pas de refs faibles —
/// déviation documentée, le moteur ne GC pas les clés).
fn weak_map_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    map_new(e, args, e.protos.weak_map)
}

fn map_new(e: &mut Engine, args: &[Value], proto_id: crate::value::ObjId) -> JsResult<Value> {
    let proto = Value::Obj(proto_id);
    let id = e.heap.alloc_obj(Object {
        proto,
        props: Vec::new(),
        idx: std::cell::OnceCell::new(),
        shape: std::cell::Cell::new(0),
        kind: ObjKind::Map(Vec::new()),
        extensible: true,
        elem_integrity: 0,
        lookups: std::cell::Cell::new(0),
        flags: std::cell::Cell::new(0),
        args_map: None,
    });
    let m = Value::Obj(id);
    // `new Map([[k, v], …])` — iterate the source, each entry must
    // be a 2-element pair.
    if !matches!(arg(args, 0), Value::Undefined | Value::Null) {
        for (i, pair) in e.iter_values(arg(args, 0))?.into_iter().enumerate() {
            // Tické : `new Map(géant)` boucle sans borne sinon —
            // kill RSS/deadline/ops morts dans le span (§B-59).
            if i % 4096 == 0 {
                e.tick()?;
            }
            let k = e.get_prop_value(pair.clone(), "0")?;
            let v = e.get_prop_value(pair, "1")?;
            weak_key_check(e, id, &k)?;
            map_set_entry(e, id, k, v)?;
        }
    }
    Ok(m)
}

fn set_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    set_new(e, args, e.protos.set)
}

/// `new WeakSet(...)` — voir `weak_map_ctor`.
fn weak_set_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    set_new(e, args, e.protos.weak_set)
}

fn set_new(e: &mut Engine, args: &[Value], proto_id: crate::value::ObjId) -> JsResult<Value> {
    let proto = Value::Obj(proto_id);
    let id = e.heap.alloc_obj(Object {
        proto,
        props: Vec::new(),
        idx: std::cell::OnceCell::new(),
        shape: std::cell::Cell::new(0),
        kind: ObjKind::Set(Vec::new()),
        extensible: true,
        elem_integrity: 0,
        lookups: std::cell::Cell::new(0),
        flags: std::cell::Cell::new(0),
        args_map: None,
    });
    let s = Value::Obj(id);
    if !matches!(arg(args, 0), Value::Undefined | Value::Null) {
        for (i, v) in e.iter_values(arg(args, 0))?.into_iter().enumerate() {
            if i % 4096 == 0 {
                e.tick()?;
            }
            weak_key_check(e, id, &v)?;
            set_add(e, id, v)?;
        }
    }
    Ok(s)
}

pub(crate) fn map_set_entry(e: &mut Engine, id: ObjId, k: Value, v: Value) -> JsResult<()> {
    // Two-phase: find the slot immutably, then mutate — `svz` needs
    // the heap for string comparison.
    let (pos, len) = e.heap.obj(id).map_or((None, 0), |o| match &o.kind {
        ObjKind::Map(m) => (m.iter().position(|(ek, _)| svz(&e.heap, ek, &k)), m.len()),
        _ => (None, 0),
    });
    if pos.is_none() {
        // Garde : `m.push` fait doubler le Vec backing dans UN
        // insert — un drain `m.set(k,v)` hostile commet ~6 Go de
        // reserve entre deux ticks (§B-59, même classe que les
        // `props`/`elems` bornés).
        e.alloc_guard((len + 1).saturating_mul(std::mem::size_of::<(Value, Value)>()))?;
    }
    let Some(o) = e.heap.obj_mut(id) else {
        return Ok(());
    };
    let ObjKind::Map(m) = &mut o.kind else {
        return Ok(());
    };
    match pos {
        Some(i) => m[i].1 = v,
        None => m.push((k, v)),
    }
    Ok(())
}

pub(crate) fn set_add(e: &mut Engine, id: ObjId, v: Value) -> JsResult<()> {
    let (has, len) = e.heap.obj(id).map_or((true, 0), |o| match &o.kind {
        ObjKind::Set(s) => (s.iter().any(|ev| svz(&e.heap, ev, &v)), s.len()),
        _ => (true, 0),
    });
    if has {
        return Ok(());
    }
    e.alloc_guard((len + 1).saturating_mul(std::mem::size_of::<Value>()))?;
    if let Some(o) = e.heap.obj_mut(id) {
        if let ObjKind::Set(s) = &mut o.kind {
            s.push(v);
        }
    }
    Ok(())
}

// ------------------------------------------------------------------
// Map methods
// ------------------------------------------------------------------

/// `CanBeHeldWeakly` (spec) — les protos `WeakMap`/`WeakSet`
/// exigent une clé objet ; `Map`/`Set` acceptent tout.
fn weak_key_check(e: &mut Engine, id: ObjId, k: &Value) -> JsResult<()> {
    let is_weak = e.heap.obj(id).is_some_and(
        |o| matches!(&o.proto, Value::Obj(p) if *p == e.protos.weak_map || *p == e.protos.weak_set),
    );
    if is_weak && !matches!(k, Value::Obj(_)) {
        return Err(e.type_error("invalid value used as weak map/set key"));
    }
    Ok(())
}

fn map_set(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let Value::Obj(id) = this else {
        return Err(e.type_error("Map.set on non-Map"));
    };
    let k = arg(args, 0);
    weak_key_check(e, id, &k)?;
    map_set_entry(e, id, k, arg(args, 1))?;
    Ok(this) // chaining
}

fn map_get(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let k = arg(args, 0);
    // Lookup sans clone : `svz` n'a besoin que de `&e.heap` —
    // `map_entries` clonait TOUT le Vec par `get` (O(n) mémoire
    // par lookup — bras §B-59, deep-clone gtag `f` : `d.get(g)`
    // par nœud sur une mémo à millions d'entrées).
    let v = e
        .heap
        .obj(c_id(this))
        .and_then(|o| match &o.kind {
            ObjKind::Map(m) => m
                .iter()
                .find(|(ek, _)| svz(&e.heap, ek, &k))
                .map(|(_, v)| v.clone()),
            _ => None,
        })
        .unwrap_or(Value::Undefined);
    Ok(v)
}

fn map_has(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let k = arg(args, 0);
    Ok(Value::Bool(e.heap.obj(c_id(this)).is_some_and(
        |o| match &o.kind {
            ObjKind::Map(m) => m.iter().any(|(ek, _)| svz(&e.heap, ek, &k)),
            _ => false,
        },
    )))
}

fn map_delete(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let Value::Obj(id) = this else {
        return Ok(Value::Bool(false));
    };
    let k = arg(args, 0);
    let pos = e.heap.obj(id).and_then(|o| match &o.kind {
        ObjKind::Map(m) => m.iter().position(|(ek, _)| svz(&e.heap, ek, &k)),
        _ => None,
    });
    let Some(i) = pos else {
        return Ok(Value::Bool(false));
    };
    if let Some(o) = e.heap.obj_mut(id) {
        if let ObjKind::Map(m) = &mut o.kind {
            m.remove(i);
        }
    }
    Ok(Value::Bool(true))
}

/// `getOrInsert(k, v)` (ES2026) — `get(k)` si présent, sinon
/// `set(k, v)` et rend `v`. Un seul lookup (spec).
fn map_get_or_insert(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let Value::Obj(id) = this else {
        return Err(e.type_error("Map.getOrInsert on non-Map"));
    };
    let (k, v) = (arg(args, 0), arg(args, 1));
    weak_key_check(e, id, &k)?;
    if let Some(ev) = e.heap.obj(id).and_then(|o| match &o.kind {
        ObjKind::Map(m) => m
            .iter()
            .find(|(ek, _)| svz(&e.heap, ek, &k))
            .map(|(_, ev)| ev.clone()),
        _ => None,
    }) {
        return Ok(ev);
    }
    map_set_entry(e, id, k, v.clone())?;
    Ok(v)
}

/// `getOrInsertComputed(k, cb)` (ES2026) — `cb(k)` n'est appelée
/// qu'en cas d'absence ; le résultat est inséré puis rendu.
fn map_get_or_insert_computed(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let Value::Obj(id) = this else {
        return Err(e.type_error("Map.getOrInsertComputed on non-Map"));
    };
    let (k, cb) = (arg(args, 0), arg(args, 1));
    weak_key_check(e, id, &k)?;
    if !e.is_callable(cb.clone()) {
        return Err(e.type_error("Map.getOrInsertComputed: callback is not callable"));
    }
    if let Some(ev) = e.heap.obj(id).and_then(|o| match &o.kind {
        ObjKind::Map(m) => m
            .iter()
            .find(|(ek, _)| svz(&e.heap, ek, &k))
            .map(|(_, ev)| ev.clone()),
        _ => None,
    }) {
        return Ok(ev);
    }
    let v = e.call_value(cb, Value::Undefined, std::slice::from_ref(&k))?;
    map_set_entry(e, id, k, v.clone())?;
    Ok(v)
}

fn map_clear(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    if let Value::Obj(id) = this {
        if let Some(o) = e.heap.obj_mut(id) {
            if let ObjKind::Map(m) = &mut o.kind {
                m.clear();
            }
        }
    }
    Ok(Value::Undefined)
}

/// `map.size` — accessor on the prototype.
fn map_size(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let n = e.heap.obj(c_id(this)).map_or(0, |o| match &o.kind {
        ObjKind::Map(m) => m.len(),
        _ => 0,
    });
    Ok(Value::Num(n as f64))
}

/// Shared `forEach(cb)` for Map (`(v, k, map)`) and Set
/// (`(v, v, set)` per spec — key === value).
fn coll_for_each(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let f = arg(args, 0);
    let this_arg = arg(args, 1);
    if !e.is_callable(f.clone()) {
        return Err(e.type_error("forEach: callback is not a function"));
    }
    let Value::Obj(id) = this else {
        return Err(e.type_error("forEach on non-collection"));
    };
    // Garde sur le snapshot : `forEach` sur collection géante
    // commet `len×16` d'un coup (§B-59 — même classe que
    // `map_entries`).
    let n = e.heap.obj(id).map_or(0, |o| match &o.kind {
        ObjKind::Map(m) => m.len(),
        ObjKind::Set(s) => s.len(),
        _ => 0,
    });
    e.alloc_guard(n.saturating_mul(std::mem::size_of::<(Value, Value)>()))?;
    let pairs: Vec<(Value, Value)> = match e.heap.obj(id).map(|o| &o.kind) {
        Some(ObjKind::Map(m)) => m.clone(),
        Some(ObjKind::Set(s)) => s.iter().map(|v| (v.clone(), v.clone())).collect(),
        _ => return Err(e.type_error("forEach on non-collection")),
    };
    for (k, v) in pairs {
        e.call_value(f.clone(), this_arg.clone(), &[v, k, this.clone()])?;
    }
    Ok(Value::Undefined)
}

fn map_keys(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let items: Vec<Value> = map_entries(e, c_id(this))?
        .into_iter()
        .map(|(k, _)| k)
        .collect();
    Ok(iter_obj(e, items, e.protos.map_iter))
}

fn map_values(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let items: Vec<Value> = map_entries(e, c_id(this))?
        .into_iter()
        .map(|(_, v)| v)
        .collect();
    Ok(iter_obj(e, items, e.protos.map_iter))
}

/// `map.entries()` — iterator of `[k, v]` arrays.
fn map_iter_entries(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let pairs = map_entries(e, c_id(this))?;
    let mut items = Vec::with_capacity(pairs.len());
    for (k, v) in pairs {
        items.push(e.new_array(vec![k, v]));
    }
    Ok(iter_obj(e, items, e.protos.map_iter))
}

// ------------------------------------------------------------------
// Set methods
// ------------------------------------------------------------------

fn set_add_m(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let Value::Obj(id) = this else {
        return Err(e.type_error("Set.add on non-Set"));
    };
    let v = arg(args, 0);
    weak_key_check(e, id, &v)?;
    set_add(e, id, v)?;
    Ok(this)
}

fn set_has(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let v = arg(args, 0);
    Ok(Value::Bool(e.heap.obj(c_id(this)).is_some_and(
        |o| match &o.kind {
            ObjKind::Set(s) => s.iter().any(|ev| svz(&e.heap, ev, &v)),
            _ => false,
        },
    )))
}

fn set_delete(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let Value::Obj(id) = this else {
        return Ok(Value::Bool(false));
    };
    let v = arg(args, 0);
    let pos = e.heap.obj(id).and_then(|o| match &o.kind {
        ObjKind::Set(s) => s.iter().position(|ev| svz(&e.heap, ev, &v)),
        _ => None,
    });
    let Some(i) = pos else {
        return Ok(Value::Bool(false));
    };
    if let Some(o) = e.heap.obj_mut(id) {
        if let ObjKind::Set(s) = &mut o.kind {
            s.remove(i);
        }
    }
    Ok(Value::Bool(true))
}

fn set_clear(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    if let Value::Obj(id) = this {
        if let Some(o) = e.heap.obj_mut(id) {
            if let ObjKind::Set(s) = &mut o.kind {
                s.clear();
            }
        }
    }
    Ok(Value::Undefined)
}

fn set_size(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let n = e.heap.obj(c_id(this)).map_or(0, |o| match &o.kind {
        ObjKind::Set(s) => s.len(),
        _ => 0,
    });
    Ok(Value::Num(n as f64))
}

/// `set.values()`/`keys()`/`entries()` + `[Symbol.iterator]`.
fn set_values_iter(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let items = set_items(e, c_id(this))?;
    Ok(iter_obj(e, items, e.protos.set_iter))
}

fn set_entries_iter(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let items = set_items(e, c_id(this))?;
    let mut out = Vec::with_capacity(items.len());
    for v in items {
        out.push(e.new_array(vec![v.clone(), v]));
    }
    Ok(iter_obj(e, out, e.protos.set_iter))
}

// -- ES2025 Set composition ------------------------------------------------

/// Alloue un `Set` pré-rempli (dédup via `set_add`, ordre
/// d'insertion préservé).
fn new_set_from(e: &mut Engine, items: Vec<Value>) -> JsResult<Value> {
    let proto = Value::Obj(e.protos.set);
    let id = e.heap.alloc_obj(Object {
        proto,
        props: Vec::new(),
        idx: std::cell::OnceCell::new(),
        shape: std::cell::Cell::new(0),
        kind: ObjKind::Set(Vec::new()),
        extensible: true,
        elem_integrity: 0,
        lookups: std::cell::Cell::new(0),
        flags: std::cell::Cell::new(0),
        args_map: None,
    });
    for v in items {
        set_add(e, id, v)?;
    }
    Ok(Value::Obj(id))
}

/// `this` et l'argument doivent être des `Set` — retourne les
/// deux listes d'éléments (ordre d'insertion).
fn two_sets(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<(Vec<Value>, Vec<Value>)> {
    let mine = set_items(e, c_id(this))?;
    let other = match arg(args, 0) {
        Value::Obj(oid) if matches!(e.heap.obj(oid).map(|o| &o.kind), Some(ObjKind::Set(_))) => {
            set_items(e, oid)?
        }
        _ => return Err(e.type_error("expected a Set argument")),
    };
    Ok((mine, other))
}

fn contains(e: &Engine, hay: &[Value], v: &Value) -> bool {
    hay.iter().any(|x| svz(&e.heap, x, v))
}

fn set_union(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let (mut a, b) = two_sets(e, this, args)?;
    a.extend(b);
    new_set_from(e, a)
}

fn set_intersection(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let (a, b) = two_sets(e, this, args)?;
    let keep: Vec<Value> = a.iter().filter(|v| contains(e, &b, v)).cloned().collect();
    new_set_from(e, keep)
}

fn set_difference(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let (a, b) = two_sets(e, this, args)?;
    let keep: Vec<Value> = a.iter().filter(|v| !contains(e, &b, v)).cloned().collect();
    new_set_from(e, keep)
}

fn set_sym_diff(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let (a, b) = two_sets(e, this, args)?;
    let mut out: Vec<Value> = a.iter().filter(|v| !contains(e, &b, v)).cloned().collect();
    out.extend(b.iter().filter(|v| !contains(e, &a, v)).cloned());
    new_set_from(e, out)
}

fn set_is_subset(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let (a, b) = two_sets(e, this, args)?;
    Ok(Value::Bool(a.iter().all(|v| contains(e, &b, v))))
}

fn set_is_superset(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let (a, b) = two_sets(e, this, args)?;
    Ok(Value::Bool(b.iter().all(|v| contains(e, &a, v))))
}

fn set_is_disjoint(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let (a, b) = two_sets(e, this, args)?;
    Ok(Value::Bool(!a.iter().any(|v| contains(e, &b, v))))
}

// ------------------------------------------------------------------
// Iterator prototype — `next()` on ObjKind::Iter
// ------------------------------------------------------------------

fn iter_next(e: &mut Engine, this: Value, a: &[Value]) -> JsResult<Value> {
    let Value::Obj(id) = this else {
        return Err(e.type_error("next on non-iterator"));
    };
    // Générateur lazy (JS-C34) — `next(v)` reprend le corps.
    {
        let Some(o) = e.heap.obj(id) else {
            return Err(e.type_error("next on non-iterator"));
        };
        if let ObjKind::GenLive { exec } = &o.kind {
            let exec = exec.clone();
            // Ré-entrance (générateur qui s'appelle lui-même
            // pendant son exécution) : l'emprunt est déjà pris —
            // spec `GeneratorResume` → TypeError abrupte, pas
            // de panic `BorrowMutError` (JS-C593).
            let Ok(mut exec) = exec.try_borrow_mut() else {
                return Err(e.type_error("generator is already running"));
            };
            // JS-C601 — la méthode sync de `%IteratorPrototype%`
            // appliquée à un `async function*` → TypeError spec
            // (la voie normale passe par `%AsyncGenerator%.next`,
            // la file `AsyncGeneratorEnqueue` à promesses).
            if exec.is_agen() {
                return Err(e.type_error("sync iterator method on async generator"));
            }
            return exec.next(e, arg(a, 0));
        }
    }
    // Helper paresseux (spec `%IteratorHelperPrototype%`) —
    // l'étape tire du sous-jacent via `call_value` : hors de
    // tout emprunt `obj_mut`.
    if matches!(
        e.heap.obj(id).map(|o| &o.kind),
        Some(ObjKind::IterHelper(_))
    ) {
        return it_helper_next(e, id);
    }
    let (item, ret) = {
        let Some(o) = e.heap.obj_mut(id) else {
            return Err(e.type_error("next on non-iterator"));
        };
        match &mut o.kind {
            ObjKind::Iter { items, idx, ret } => {
                if *idx < items.len() {
                    let v = items[*idx].clone();
                    *idx += 1;
                    (Some(v), Value::Undefined)
                } else {
                    (None, ret.clone())
                }
            }
            _ => return Err(e.type_error("next on non-iterator")),
        }
    };
    let done = item.is_none();
    Ok(iter_result(e, item.unwrap_or(ret), done))
}

/// Iterator proto's `[Symbol.iterator]` returns `this` —
/// sert aussi `[Symbol.asyncIterator]` de
/// `%AsyncIteratorPrototype%` (JS-C601 : `this` nu dans les
/// deux cas).
pub(crate) fn iter_self(_e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    Ok(this)
}

/// `gen.return(v)` — termine l'itérateur (le corps a déjà tourné
/// en mode eager : pas de `finally` à rejouer) → `{value: v,
/// done: true}`.
fn iter_return(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let Value::Obj(id) = this else {
        return Err(e.type_error("return on non-iterator"));
    };
    // Générateur lazy — `return(v)` termine le corps.
    if let Some(o) = e.heap.obj(id) {
        if let ObjKind::GenLive { exec } = &o.kind {
            let exec = exec.clone();
            let Ok(mut exec) = exec.try_borrow_mut() else {
                return Err(e.type_error("generator is already running"));
            };
            if exec.is_agen() {
                return Err(e.type_error("sync iterator method on async generator"));
            }
            return exec.ret_at(e, arg(args, 0));
        }
    }
    // Helper paresseux : `return` = `IteratorClose` du
    // sous-jacent **pendant que la sentinelle `Running` est en
    // place** (ré-entrance — un `return` du sous-jacent qui
    // rappelle `helper.return()`/`next()`/`throw()` →
    // `TypeError` « already running », sémantique générateur),
    // puis `{value: undefined, done: true}` — spec
    // `%IteratorHelperPrototype%.return`, l'argument n'est PAS
    // propagé. Helper complété → pas de forward (verrou
    // `done`), même résultat.
    let st = take_helper(e, id)?;
    if let Some(mut st) = st {
        if !st.done {
            st.done = true;
            let src = st.source.clone();
            let r = it_close(e, &src);
            if let Some(o) = e.heap.obj_mut(id) {
                o.kind = ObjKind::IterHelper(st);
            }
            r?;
        } else if let Some(o) = e.heap.obj_mut(id) {
            o.kind = ObjKind::IterHelper(st);
        }
        return Ok(iter_result(e, Value::Undefined, true));
    }
    if let Some(o) = e.heap.obj_mut(id) {
        if let ObjKind::Iter { items, idx, .. } = &mut o.kind {
            *idx = items.len();
        }
    }
    Ok(iter_result(e, arg(args, 0), true))
}

/// `gen.throw(e)` — lazy (JS-C34) : jeté *au site* suspendu
/// (un `try/catch` interne peut rattraper) ; eager : levée
/// directe à l'appelant.
fn iter_throw(e: &mut Engine, t: Value, args: &[Value]) -> JsResult<Value> {
    if let Value::Obj(id) = t {
        if let Some(o) = e.heap.obj(id) {
            if let ObjKind::GenLive { exec } = &o.kind {
                let exec = exec.clone();
                let Ok(mut exec) = exec.try_borrow_mut() else {
                    return Err(e.type_error("generator is already running"));
                };
                if exec.is_agen() {
                    return Err(e.type_error("sync iterator method on async generator"));
                }
                return exec.throw_at(e, arg(args, 0));
            }
        }
        // Helper paresseux : fermer le sous-jacent sous
        // sentinelle `Running` (`IteratorClose` sur complétion
        // abrupte — son échec est masqué par la valeur jetée ;
        // ré-entrance pendant le close → `TypeError`) puis
        // lever. Helper complété → lever sans fermer.
        let st = take_helper(e, id)?;
        if let Some(mut st) = st {
            if !st.done {
                st.done = true;
                let src = st.source.clone();
                let _ = it_close(e, &src);
            }
            if let Some(o) = e.heap.obj_mut(id) {
                o.kind = ObjKind::IterHelper(st);
            }
        }
    }
    Err(Thrown(arg(args, 0)))
}

fn c_id(v: Value) -> ObjId {
    match v {
        Value::Obj(id) => id,
        _ => u32::MAX as ObjId,
    }
}

/// Install `Map`, `Set`, the shared iterator prototype.
pub(crate) fn install(e: &mut Engine) {
    let iterator = e.protos.iterator;
    e.add_method(iterator, "next", iter_next);
    e.add_method(iterator, "return", iter_return);
    e.add_method(iterator, "throw", iter_throw);
    {
        let self_fn = e.native_fn("[Symbol.iterator]", iter_self);
        e.set_own(iterator, SYM_ITERATOR, self_fn, false);
        // PAS de `[Symbol.asyncIterator]` : un `Iter` est un
        // itérable *sync* — `for await` doit passer par le fallback
        // `AsyncFromSyncIterator` qui awaite chaque valeur.
    }
    // JS-C8 — Iterator helpers (ES2025) : snapshot eager, les
    // méthodes paresseuses spec sont matérialisées (déviation).
    e.add_method(iterator, "map", it_map);
    e.add_method(iterator, "filter", it_filter);
    e.add_method(iterator, "take", it_take);
    e.add_method(iterator, "drop", it_drop);
    e.add_method(iterator, "flatMap", it_flat_map);
    e.add_method(iterator, "toArray", it_to_array);
    e.add_method(iterator, "forEach", it_for_each);
    e.add_method(iterator, "reduce", it_reduce);
    e.add_method(iterator, "some", it_some);
    e.add_method(iterator, "every", it_every);
    e.add_method(iterator, "find", it_find);
    // ES2026 chunking/sequencing — snapshot eager (déviation).
    e.add_method(iterator, "includes", it_includes);
    e.add_method(iterator, "join", it_join);
    e.add_method(iterator, "chunks", it_chunks);
    e.add_method(iterator, "windows", it_windows);

    let map_proto = e.protos.map;
    e.add_method(map_proto, "set", map_set);
    e.add_method(map_proto, "get", map_get);
    e.add_method(map_proto, "has", map_has);
    e.add_method(map_proto, "delete", map_delete);
    e.add_method(map_proto, "clear", map_clear);
    e.add_method(map_proto, "forEach", coll_for_each);
    e.add_method(map_proto, "keys", map_keys);
    e.add_method(map_proto, "values", map_values);
    e.add_method(map_proto, "entries", map_iter_entries);
    // ES2026 upsert — hérité par WeakMap via la chaîne proto.
    e.add_method(map_proto, "getOrInsert", map_get_or_insert);
    e.add_method(map_proto, "getOrInsertComputed", map_get_or_insert_computed);
    {
        let entries_fn = e.native_fn("entries", map_iter_entries);
        e.set_own(map_proto, SYM_ITERATOR, entries_fn, false);
        let size_fn = e.native_fn("size", map_size);
        if let Some(o) = e.heap.obj_mut(map_proto) {
            o.set_accessor("size", size_fn, Value::Undefined, false);
        }
    }
    let map_ctor_v = e.native_fn("Map", map_ctor);
    e.set_own_attrs(
        c_id(map_ctor_v.clone()),
        "prototype",
        Value::Obj(map_proto),
        crate::heap::PropAttrs::FIXED,
    );
    e.set_own(map_proto, "constructor", map_ctor_v.clone(), false);
    e.add_method(c_id(map_ctor_v.clone()), "groupBy", map_group_by);
    let g = e.global;
    e.set_own(g, "Map", map_ctor_v.clone(), false);
    crate::builtins::install_species(e, &map_ctor_v);

    let set_proto = e.protos.set;
    e.add_method(set_proto, "add", set_add_m);
    e.add_method(set_proto, "has", set_has);
    e.add_method(set_proto, "delete", set_delete);
    e.add_method(set_proto, "clear", set_clear);
    e.add_method(set_proto, "forEach", coll_for_each);
    e.add_method(set_proto, "keys", set_values_iter);
    e.add_method(set_proto, "values", set_values_iter);
    e.add_method(set_proto, "entries", set_entries_iter);
    // ES2025 `Set` composition methods.
    e.add_method(set_proto, "union", set_union);
    e.add_method(set_proto, "intersection", set_intersection);
    e.add_method(set_proto, "difference", set_difference);
    e.add_method(set_proto, "symmetricDifference", set_sym_diff);
    e.add_method(set_proto, "isSubsetOf", set_is_subset);
    e.add_method(set_proto, "isSupersetOf", set_is_superset);
    e.add_method(set_proto, "isDisjointFrom", set_is_disjoint);
    {
        let values_fn = e.native_fn("values", set_values_iter);
        e.set_own(set_proto, SYM_ITERATOR, values_fn, false);
        let size_fn = e.native_fn("size", set_size);
        if let Some(o) = e.heap.obj_mut(set_proto) {
            o.set_accessor("size", size_fn, Value::Undefined, false);
        }
    }
    let set_ctor_v = e.native_fn("Set", set_ctor);
    e.set_own_attrs(
        c_id(set_ctor_v.clone()),
        "prototype",
        Value::Obj(set_proto),
        crate::heap::PropAttrs::FIXED,
    );
    e.set_own(set_proto, "constructor", set_ctor_v.clone(), false);
    let g = e.global;
    e.set_own(g, "Set", set_ctor_v.clone(), false);
    crate::builtins::install_species(e, &set_ctor_v);

    // `WeakMap`/`WeakSet` — ctors + protos dédiés (JS-C19) :
    // `WeakMap !== Map` et `Object.prototype.toString` distingue
    // "WeakMap" (spec `@@toStringTag`). Protos parentés à
    // `Object.prototype` (spec JS-C24) : seuls `get`/`set`/`has`/
    // `delete` (et `add` pour WeakSet) sont exposés — pas
    // d'itérateur, pas d'accès aux entrées. Pas de refs faibles —
    // déviation ADR-0007.
    {
        let wmp = e.protos.weak_map;
        let tag = e.str_val("WeakMap");
        e.set_own_attrs(
            wmp,
            crate::interp::SYM_TO_STRING_TAG,
            tag,
            crate::heap::PropAttrs {
                writable: false,
                enumerable: false,
                configurable: true,
            },
        );
        e.add_method(wmp, "get", map_get);
        e.add_method(wmp, "set", map_set);
        e.add_method(wmp, "has", map_has);
        e.add_method(wmp, "delete", map_delete);
        // ES2026 upsert — présent sur WeakMap aussi (spec).
        e.add_method(wmp, "getOrInsert", map_get_or_insert);
        e.add_method(wmp, "getOrInsertComputed", map_get_or_insert_computed);
        let wm = e.native_fn("WeakMap", weak_map_ctor);
        e.set_own_attrs(
            c_id(wm.clone()),
            "prototype",
            Value::Obj(wmp),
            crate::heap::PropAttrs::FIXED,
        );
        e.set_own(wmp, "constructor", wm.clone(), false);
        e.set_own(g, "WeakMap", wm, false);
    }
    {
        let wsp = e.protos.weak_set;
        let tag = e.str_val("WeakSet");
        e.set_own_attrs(
            wsp,
            crate::interp::SYM_TO_STRING_TAG,
            tag,
            crate::heap::PropAttrs {
                writable: false,
                enumerable: false,
                configurable: true,
            },
        );
        e.add_method(wsp, "add", set_add_m);
        e.add_method(wsp, "has", set_has);
        e.add_method(wsp, "delete", set_delete);
        let ws = e.native_fn("WeakSet", weak_set_ctor);
        e.set_own_attrs(
            c_id(ws.clone()),
            "prototype",
            Value::Obj(wsp),
            crate::heap::PropAttrs::FIXED,
        );
        e.set_own(wsp, "constructor", ws.clone(), false);
        e.set_own(g, "WeakSet", ws, false);
    }
}

// ------------------------------------------------------------------
// JS-C8 — Map.groupBy (ES2024)
// ------------------------------------------------------------------

/// `Map.groupBy(items, cb)` — `Map` groupé par clé callback
/// (`SameValueZero` — clés `Value` brutes, pas `ToString`).
fn map_group_by(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let items = e.iter_values(arg(args, 0))?;
    let f = arg(args, 1);
    if !e.is_callable(f.clone()) {
        return Err(e.type_error("groupBy callback is not callable"));
    }
    let out = e.heap.alloc_obj(Object {
        proto: Value::Obj(e.protos.map),
        props: Vec::new(),
        idx: std::cell::OnceCell::new(),
        shape: std::cell::Cell::new(0),
        kind: ObjKind::Map(Vec::new()),
        extensible: true,
        elem_integrity: 0,
        lookups: std::cell::Cell::new(0),
        flags: std::cell::Cell::new(0),
        args_map: None,
    });
    for (i, v) in items.into_iter().enumerate() {
        let k = e.call_value(
            f.clone(),
            Value::Undefined,
            &[v.clone(), Value::Num(i as f64)],
        )?;
        let existing = e.heap.obj(out).and_then(|o| match &o.kind {
            ObjKind::Map(m) => m
                .iter()
                .find(|(ek, _)| svz(&e.heap, ek, &k))
                .and_then(|(_, ev)| {
                    if let Value::Obj(id) = ev {
                        Some(*id)
                    } else {
                        None
                    }
                }),
            _ => None,
        });
        let aid = if let Some(id) = existing {
            id
        } else {
            let a = e.new_array(Vec::new());
            let Value::Obj(aid) = a else { continue };
            map_set_entry(e, out, k, a)?;
            aid
        };
        if let Some(o) = e.heap.obj_mut(aid) {
            if let ObjKind::Array(elems) = &mut o.kind {
                elems.push(v);
            }
        }
    }
    Ok(Value::Obj(out))
}

// ------------------------------------------------------------------
// Iterator helpers (ES2025) — **paresseux** (`ObjKind::IterHelper`,
// spec `%IteratorHelperPrototype%`). La paresse est observable :
// `map(f)` ne tire pas avant `next()`, les getters `done`/`value`
// qui jettent propagent au pull, `take(n)` fonctionne sur source
// infinie, `return` est forwardé (`IteratorClose`). L'ancien
// snapshot eager drainait via `get_prop` data-only : un
// `get done() { throw }` n'était jamais observé → boucle jusqu'au
// cap heap (cluster test262 #1, ~178 fichiers — profil §B-114).
// ------------------------------------------------------------------

/// Étape d'un record itérateur — spec `IteratorStep` +
/// `IteratorValue` : résultat non-objet → `TypeError`, getters
/// `done`/`value` propagés (`get_prop` data-only les avalait).
fn it_step(e: &mut Engine, src: &Value, next: &Value) -> JsResult<Option<Value>> {
    e.tick()?;
    let r = e.call_value(next.clone(), src.clone(), &[])?;
    let Value::Obj(rid) = r else {
        return Err(e.type_error("iterator result is not an object"));
    };
    let done = e.get_prop_value(Value::Obj(rid), "done")?;
    if e.to_boolean(done) {
        return Ok(None);
    }
    Ok(Some(e.get_prop_value(Value::Obj(rid), "value")?))
}

/// `IteratorClose` (spec §7.4) — `GetMethod(src, "return")` :
/// nullish → rien, **non-callable → `TypeError`** (`GetMethod`),
/// sinon appelé avec zéro arg (complétion abrupte) ; résultat
/// non-objet → `TypeError`, sinon ignoré.
fn it_close(e: &mut Engine, src: &Value) -> JsResult<()> {
    if !matches!(src, Value::Obj(_)) {
        return Ok(());
    }
    let rf = e.get_prop_value(src.clone(), "return")?;
    if matches!(rf, Value::Undefined | Value::Null) {
        return Ok(());
    }
    if !e.is_callable(rf.clone()) {
        return Err(e.type_error("iterator return is not callable"));
    }
    let r = e.call_value(rf, src.clone(), &[])?;
    if matches!(r, Value::Obj(_)) {
        Ok(())
    } else {
        Err(e.type_error("iterator return result is not an object"))
    }
}

/// `GetIteratorDirect`/`Flattenable` — record `(obj, next)` :
/// `{next}` callable direct ou via `@@iterator` (la spec
/// accepte les deux — le fallback manquant était le cluster
/// #4 « not iterable »).
pub(crate) fn it_record(e: &mut Engine, v: Value) -> JsResult<(Value, Value)> {
    if matches!(v, Value::Obj(_)) {
        let nf = e.get_prop_value(v.clone(), "next")?;
        if e.is_callable(nf.clone()) {
            return Ok((v, nf));
        }
        let m = e.get_prop_value(v.clone(), SYM_ITERATOR)?;
        if e.is_callable(m.clone()) {
            let it = e.call_value(m, v.clone(), &[])?;
            if matches!(it, Value::Obj(_)) {
                let nf = e.get_prop_value(it.clone(), "next")?;
                if e.is_callable(nf.clone()) {
                    return Ok((it, nf));
                }
            }
        }
    }
    // Primitif itérable (chaîne — spec `ToObject` + `@@iterator`)
    // : snapshot matérialisé, comme nos itérateurs de chaîne.
    if matches!(v, Value::Str(_)) {
        let items = e.iter_values(v)?;
        let it = iter_obj(e, items, e.protos.string_iter);
        let nf = e.get_prop_value(it.clone(), "next")?;
        if e.is_callable(nf.clone()) {
            return Ok((it, nf));
        }
    }
    Err(e.type_error("value is not iterable"))
}

/// `%WrapForValidIteratorPrototype%` (spec `Iterator.from`) —
/// le record est forwardé **verbatim** par `next`.
pub(crate) fn it_wrap(e: &mut Engine, source: Value, next: Value) -> Value {
    let id = e
        .heap
        .alloc_obj(Object::ordinary(Value::Obj(e.protos.iter_helper)));
    if let Some(o) = e.heap.obj_mut(id) {
        o.kind = ObjKind::IterHelper(Box::new(IterHelperState {
            source,
            next,
            inner: None,
            done: false,
            counter: 0,
            op: IterOp::Wrap,
        }));
    }
    Value::Obj(id)
}

/// `Iterator.concat(...)` — les args ont été validés à la
/// construction (objet + `@@iterator` callable lu une fois —
/// spec `GetMethod`, jamais appelé ici). La file porte des
/// paires `(itérable, méthode)` **non ouvertes** : chaque
/// record est créé paresseusement quand l'itération l'atteint
/// (« the remaining iterables are never opened »). `st.next`
/// `Undefined` = pas de record courant. `return`/`throw`
/// ferment le record courant (verrou `done` partagé).
pub(crate) fn it_concat(e: &mut Engine, recs: VecDeque<(Value, Value)>) -> Value {
    let done = recs.is_empty();
    let id = e
        .heap
        .alloc_obj(Object::ordinary(Value::Obj(e.protos.iter_helper)));
    if let Some(o) = e.heap.obj_mut(id) {
        o.kind = ObjKind::IterHelper(Box::new(IterHelperState {
            done,
            source: Value::Undefined,
            next: Value::Undefined,
            inner: None,
            counter: 0,
            op: IterOp::Concat(recs),
        }));
    }
    Value::Obj(id)
}

/// Crée le helper paresseux — valide le receveur
/// (`GetIteratorDirect`) puis pose le record + `op`.
fn it_helper(e: &mut Engine, this: Value, op: IterOp) -> JsResult<Value> {
    let (source, next) = it_record(e, this)?;
    let id = e
        .heap
        .alloc_obj(Object::ordinary(Value::Obj(e.protos.iter_helper)));
    if let Some(o) = e.heap.obj_mut(id) {
        o.kind = ObjKind::IterHelper(Box::new(IterHelperState {
            source,
            next,
            inner: None,
            done: false,
            counter: 0,
            op,
        }));
    }
    Ok(Value::Obj(id))
}

/// État sentinelle — posé dans le tas pendant qu'une méthode
/// de helper exécute du code appelable (`next` du sous-jacent,
/// callback, `return` de `it_close`). Une ré-entrance la lit
/// → `TypeError` « generator is already running » (sémantique
/// générateur spec — les helpers SONT des générateurs).
fn running_sentinel() -> ObjKind {
    ObjKind::IterHelper(Box::new(IterHelperState {
        source: Value::Undefined,
        next: Value::Undefined,
        inner: None,
        done: false,
        counter: 0,
        op: IterOp::Running,
    }))
}

/// Extrait l'état `IterHelper` en posant la sentinelle
/// [`running_sentinel`]. `Ok(None)` = pas un helper (kind
/// restauré tel quel). Sentinelle déjà en place → `TypeError`
/// (ré-entrance — la sentinelle est reposée, la course
/// continue).
fn take_helper(e: &mut Engine, id: ObjId) -> JsResult<Option<Box<IterHelperState>>> {
    let st = match e
        .heap
        .obj_mut(id)
        .map(|o| std::mem::replace(&mut o.kind, running_sentinel()))
    {
        Some(ObjKind::IterHelper(st)) => st,
        Some(k) => {
            if let Some(o) = e.heap.obj_mut(id) {
                o.kind = k;
            }
            return Ok(None);
        }
        None => return Ok(None),
    };
    if matches!(st.op, IterOp::Running) {
        if let Some(o) = e.heap.obj_mut(id) {
            o.kind = ObjKind::IterHelper(st);
        }
        return Err(e.type_error("generator is already running"));
    }
    Ok(Some(st))
}

/// `next()` d'un helper paresseux — l'état est extrait
/// ([`take_helper`], sentinelle `Running` en place), une étape
/// appliquée, reposé. `%WrapForValidIteratorPrototype%` est
/// traité **sans** extraction : appel brut, la ré-entrance y
/// est légale.
fn it_helper_next(e: &mut Engine, id: ObjId) -> JsResult<Value> {
    // `Wrap` — appel brut : `next` cloné, invoqué, résultat
    // forwardé verbatim (non-objet → `TypeError`).
    if matches!(
        e.heap.obj(id).map(|o| &o.kind),
        Some(ObjKind::IterHelper(st)) if matches!(st.op, IterOp::Wrap)
    ) {
        let (src, next) = match e.heap.obj(id).map(|o| &o.kind) {
            Some(ObjKind::IterHelper(st)) => (st.source.clone(), st.next.clone()),
            _ => (Value::Undefined, Value::Undefined),
        };
        let r = e.call_value(next, src, &[])?;
        if matches!(r, Value::Obj(_)) {
            return Ok(r);
        }
        return Err(e.type_error("iterator result is not an object"));
    }
    let Some(mut st) = take_helper(e, id)? else {
        return Err(e.type_error("next on non-iterator"));
    };
    let r = it_helper_step(e, &mut st);
    // Abrupt : le helper est **complété** (sémantique
    // générateur — `next` ultérieur → `{done:true}` sans
    // re-step, `return` non forwardé).
    // Le close n'est PAS fait ici : spec, un abrupt du *step*
    // (`next`/getters) se propage tel quel sans fermer — seul
    // l'abrupt du *callback* passe par `IfAbruptCloseIterator`
    // (traité dans `it_helper_step`/`step_or_close`).
    if r.is_err() {
        st.done = true;
    }
    if let Some(o) = e.heap.obj_mut(id) {
        o.kind = ObjKind::IterHelper(st);
    }
    match r? {
        ItOut::Val(v) => Ok(iter_result(e, v, false)),
        ItOut::Done => Ok(iter_result(e, Value::Undefined, true)),
    }
}

/// `IfAbruptCloseIterator` (spec) — l'erreur du *callback*
/// (mapper/predicate, ou `GetIteratorFlattenable` du résultat)
/// ferme le sous-jacent puis se propage ; l'échec du close est
/// masqué. Un abrupt du *step* lui-même (`next`, getters
/// `done`/`value`) se propage au contraire **sans** fermer.
fn step_or_close<T>(e: &mut Engine, src: &Value, r: JsResult<T>) -> JsResult<T> {
    match r {
        Err(t) => {
            let _ = it_close(e, src);
            Err(t)
        }
        ok => ok,
    }
}

/// Sortie d'une étape de helper paresseux.
enum ItOut {
    /// `{value: v, done: false}` reconstruit.
    Val(Value),
    /// `{value: undefined, done: true}`.
    Done,
}

/// Une étape paresseuse — [`ItOut::Val`] = `{value:v,
/// done:false}` reconstruit, [`ItOut::Done`] = `done` (l'état
/// passe `done`).
fn it_helper_step(e: &mut Engine, st: &mut IterHelperState) -> JsResult<ItOut> {
    if st.done {
        return Ok(ItOut::Done);
    }
    e.tick()?;
    let cb = |e: &mut Engine, f: &Value, v: Value, i: u64| {
        e.call_value(f.clone(), Value::Undefined, &[v, Value::Num(i as f64)])
    };
    let out = match &mut st.op {
        IterOp::Map(f) => match it_step(e, &st.source, &st.next)? {
            Some(v) => {
                let i = st.counter;
                st.counter += 1;
                let m = cb(e, f, v, i);
                let m = step_or_close(e, &st.source, m)?;
                ItOut::Val(m)
            }
            None => ItOut::Done,
        },
        IterOp::Filter(f) => {
            let mut hit = ItOut::Done;
            while let Some(v) = it_step(e, &st.source, &st.next)? {
                let i = st.counter;
                st.counter += 1;
                let k = cb(e, f, v.clone(), i);
                let k = step_or_close(e, &st.source, k)?;
                if e.to_boolean(k) {
                    hit = ItOut::Val(v);
                    break;
                }
            }
            hit
        }
        IterOp::Take(rem) => {
            if *rem <= 0.0 {
                it_close(e, &st.source)?;
                ItOut::Done
            } else {
                *rem -= 1.0;
                match it_step(e, &st.source, &st.next)? {
                    Some(v) => ItOut::Val(v),
                    None => ItOut::Done,
                }
            }
        }
        IterOp::Drop(rem) => {
            let mut alive = true;
            while *rem > 0.0 {
                *rem -= 1.0;
                if it_step(e, &st.source, &st.next)?.is_none() {
                    alive = false;
                    break;
                }
            }
            if alive {
                match it_step(e, &st.source, &st.next)? {
                    Some(v) => ItOut::Val(v),
                    None => ItOut::Done,
                }
            } else {
                ItOut::Done
            }
        }
        IterOp::FlatMap(f) => {
            let mut out = ItOut::Done;
            while matches!(out, ItOut::Done) {
                if let Some((isrc, inext)) = st.inner.clone() {
                    match it_step(e, &isrc, &inext)? {
                        Some(v) => {
                            out = ItOut::Val(v);
                            break;
                        }
                        None => st.inner = None,
                    }
                }
                match it_step(e, &st.source, &st.next)? {
                    None => break,
                    Some(v) => {
                        let i = st.counter;
                        st.counter += 1;
                        let m = cb(e, f, v, i);
                        let m = step_or_close(e, &st.source, m)?;
                        let r = it_record(e, m);
                        st.inner = Some(step_or_close(e, &st.source, r)?);
                    }
                }
            }
            out
        }
        IterOp::Concat(rest) => {
            let mut out = ItOut::Done;
            loop {
                if e.is_callable(st.next.clone()) {
                    match it_step(e, &st.source, &st.next)? {
                        Some(v) => {
                            out = ItOut::Val(v);
                            break;
                        }
                        None => st.next = Value::Undefined,
                    }
                } else {
                    // Le record s'ouvre **ici** — `Call(method,
                    // item)` puis `GetIteratorDirect` sur le
                    // résultat (spec ; les itérables jamais
                    // atteints ne sont jamais ouverts).
                    match rest.pop_front() {
                        Some((item, method)) => {
                            let it = e.call_value(method, item, &[])?;
                            if !matches!(it, Value::Obj(_)) {
                                return Err(e.type_error("iterator is not an object"));
                            }
                            let (s, n) = it_record(e, it)?;
                            st.source = s;
                            st.next = n;
                        }
                        None => break,
                    }
                }
            }
            out
        }
        IterOp::Chunks(n) => {
            let mut buf = Vec::with_capacity(*n);
            for _ in 0..*n {
                match it_step(e, &st.source, &st.next)? {
                    Some(v) => buf.push(v),
                    None => break,
                }
            }
            if buf.is_empty() {
                ItOut::Done
            } else {
                ItOut::Val(e.new_array(buf))
            }
        }
        IterOp::Windows { n, buf } => {
            while buf.len() < *n {
                match it_step(e, &st.source, &st.next)? {
                    Some(v) => buf.push_back(v),
                    None => break,
                }
            }
            if buf.len() < *n {
                ItOut::Done
            } else {
                let out = e.new_array(buf.iter().cloned().collect());
                buf.pop_front();
                ItOut::Val(out)
            }
        }
        // `Wrap` est traité en amont (`it_helper_next`, appel
        // brut sans extraction) ; `Running` = sentinelle de
        // course — les deux sont interceptées avant ce point,
        // filet de sécurité.
        IterOp::Wrap | IterOp::Running => {
            return Err(e.type_error("generator is already running"));
        }
    };
    if matches!(out, ItOut::Done) {
        st.done = true;
    }
    Ok(out)
}

/// Drain `this` — fast path `ObjKind::Iter`, sinon le protocole
/// `next()` partagé (getters propagés). Utilisé par les
/// terminaux sans early-exit ni callback-fermeture.
fn it_pull_all(e: &mut Engine, this: Value) -> JsResult<Vec<Value>> {
    if let Value::Obj(id) = this {
        if let Some(o) = e.heap.obj_mut(id) {
            if let ObjKind::Iter { items, idx, .. } = &mut o.kind {
                let rest = items.split_off(*idx);
                *idx = 0;
                return Ok(rest);
            }
        }
        let (src, next) = it_record(e, this)?;
        let mut out = Vec::new();
        while let Some(v) = it_step(e, &src, &next)? {
            out.push(v);
        }
        return Ok(out);
    }
    Err(e.type_error("iterator helper on non-iterator"))
}

fn it_map(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let f = arg(args, 0);
    if !e.is_callable(f.clone()) {
        return Err(e.type_error("mapper is not callable"));
    }
    it_helper(e, this, IterOp::Map(f))
}

fn it_filter(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let f = arg(args, 0);
    if !e.is_callable(f.clone()) {
        return Err(e.type_error("predicate is not callable"));
    }
    it_helper(e, this, IterOp::Filter(f))
}

/// `limit` spec `ToIntegerOrInfinity` — NaN → +0, négatif →
/// `RangeError` (le `.max(0.0)` clampait silencieusement).
fn it_limit(e: &mut Engine, v: Value, what: &str) -> JsResult<f64> {
    let n = e.to_number(v)?;
    if n.is_nan() {
        return Ok(0.0);
    }
    if n < 0.0 {
        return Err(e.range_error(format!("{what} must be non-negative")));
    }
    Ok(n)
}

fn it_take(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let n = it_limit(e, arg(args, 0), "take limit")?;
    it_helper(e, this, IterOp::Take(n))
}

fn it_drop(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let n = it_limit(e, arg(args, 0), "drop limit")?;
    it_helper(e, this, IterOp::Drop(n))
}

fn it_flat_map(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let f = arg(args, 0);
    if !e.is_callable(f.clone()) {
        return Err(e.type_error("mapper is not callable"));
    }
    it_helper(e, this, IterOp::FlatMap(f))
}

fn it_to_array(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let vals = it_pull_all(e, this)?;
    Ok(e.new_array(vals))
}

/// Terminal à callbacks — boucle par étape (spec) : l'erreur
/// d'un callback ferme le sous-jacent (`IfAbruptCloseIterator`)
/// au lieu de l'avoir déjà drainé en entier.
fn it_for_each(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let f = arg(args, 0);
    if !e.is_callable(f.clone()) {
        return Err(e.type_error("callback is not callable"));
    }
    let (src, next) = it_record(e, this)?;
    let mut i = 0u64;
    while let Some(v) = it_step(e, &src, &next)? {
        if let Err(t) = e.call_value(f.clone(), Value::Undefined, &[v, Value::Num(i as f64)]) {
            let _ = it_close(e, &src);
            return Err(t);
        }
        i += 1;
    }
    Ok(Value::Undefined)
}

fn it_reduce(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let f = arg(args, 0);
    if !e.is_callable(f.clone()) {
        return Err(e.type_error("reducer is not callable"));
    }
    let (src, next) = it_record(e, this)?;
    let mut acc = match arg(args, 1) {
        Value::Undefined => match it_step(e, &src, &next)? {
            Some(v) => v,
            None => return Err(e.type_error("reduce of empty iterator with no initial value")),
        },
        v => v,
    };
    let mut i = match arg(args, 1) {
        Value::Undefined => 1u64,
        _ => 0u64,
    };
    while let Some(v) = it_step(e, &src, &next)? {
        match e.call_value(f.clone(), Value::Undefined, &[acc, v, Value::Num(i as f64)]) {
            Ok(a) => acc = a,
            Err(t) => {
                let _ = it_close(e, &src);
                return Err(t);
            }
        }
        i += 1;
    }
    Ok(acc)
}

fn it_some(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let f = arg(args, 0);
    if !e.is_callable(f.clone()) {
        return Err(e.type_error("predicate is not callable"));
    }
    let (src, next) = it_record(e, this)?;
    let mut i = 0u64;
    while let Some(v) = it_step(e, &src, &next)? {
        match e.call_value(f.clone(), Value::Undefined, &[v, Value::Num(i as f64)]) {
            Err(t) => {
                let _ = it_close(e, &src);
                return Err(t);
            }
            Ok(k) if e.to_boolean(k.clone()) => {
                it_close(e, &src)?;
                return Ok(Value::Bool(true));
            }
            _ => i += 1,
        }
    }
    Ok(Value::Bool(false))
}

fn it_every(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let f = arg(args, 0);
    if !e.is_callable(f.clone()) {
        return Err(e.type_error("predicate is not callable"));
    }
    let (src, next) = it_record(e, this)?;
    let mut i = 0u64;
    while let Some(v) = it_step(e, &src, &next)? {
        match e.call_value(f.clone(), Value::Undefined, &[v, Value::Num(i as f64)]) {
            Err(t) => {
                let _ = it_close(e, &src);
                return Err(t);
            }
            Ok(k) if !e.to_boolean(k.clone()) => {
                it_close(e, &src)?;
                return Ok(Value::Bool(false));
            }
            _ => i += 1,
        }
    }
    Ok(Value::Bool(true))
}

fn it_find(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let f = arg(args, 0);
    if !e.is_callable(f.clone()) {
        return Err(e.type_error("predicate is not callable"));
    }
    let (src, next) = it_record(e, this)?;
    let mut i = 0u64;
    while let Some(v) = it_step(e, &src, &next)? {
        match e.call_value(
            f.clone(),
            Value::Undefined,
            &[v.clone(), Value::Num(i as f64)],
        ) {
            Err(t) => {
                let _ = it_close(e, &src);
                return Err(t);
            }
            Ok(k) if e.to_boolean(k.clone()) => {
                it_close(e, &src)?;
                return Ok(v);
            }
            _ => i += 1,
        }
    }
    Ok(Value::Undefined)
}

// -- Iterator helpers ES2026 (chunking/sequencing proposals) --------------
// Même mécanisme paresseux que les helpers ES2025.

/// `iter.includes(v)` — `SameValueZero` (`===` + NaN), early-
/// exit fermé (`IteratorClose`).
fn it_includes(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let needle = arg(args, 0);
    let (src, next) = it_record(e, this)?;
    while let Some(v) = it_step(e, &src, &next)? {
        let nan = matches!((v.clone(), needle.clone()), (Value::Num(a), Value::Num(b)) if a.is_nan() && b.is_nan());
        if nan || e.strict_eq(&v, &needle) {
            it_close(e, &src)?;
            return Ok(Value::Bool(true));
        }
    }
    Ok(Value::Bool(false))
}

/// `iter.join(sep?)` — joint les valeurs (défaut `,`).
fn it_join(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let sep = match arg(args, 0) {
        Value::Undefined => ",".to_string(),
        v => e.to_js_string(v)?,
    };
    let mut out = String::new();
    for (i, v) in it_pull_all(e, this)?.into_iter().enumerate() {
        if i > 0 {
            out.push_str(&sep);
        }
        out.push_str(&e.to_js_string(v)?);
    }
    Ok(e.str_val(out))
}

/// Taille de lot `n` : entier ≥ 1, `RangeError` sinon (proposal).
fn it_chunk_size(e: &mut Engine, v: Value) -> JsResult<usize> {
    let n = e.to_number(v)?;
    if n.is_nan() || n < 1.0 {
        return Err(e.range_error("chunk size must be a positive integer"));
    }
    Ok(n as usize)
}

/// `iter.chunks(n)` — lots consécutifs de `n` éléments (le
/// dernier peut être plus court) → itérateur d'arrays paresseux.
fn it_chunks(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let n = it_chunk_size(e, arg(args, 0))?;
    it_helper(e, this, IterOp::Chunks(n))
}

/// `iter.windows(n)` — fenêtres glissantes de `n` (rien si
/// l'itérateur est plus court que `n`) → itérateur d'arrays
/// paresseux.
fn it_windows(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let n = it_chunk_size(e, arg(args, 0))?;
    it_helper(
        e,
        this,
        IterOp::Windows {
            n,
            buf: std::collections::VecDeque::new(),
        },
    )
}

/// `Iterator.zip(iterables, {mode?})` — proposal : itérateur
/// d'arrays `[it1[i], it2[i], …]` ; mode `shortest` (défaut),
/// `longest` (avec `padding`) ou `strict` (longueurs égales
/// imposées → `RangeError`).
pub(crate) fn iterator_zip(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let iters = zip_iters(e, arg(args, 0))?;
    let opts = arg(args, 1);
    let mode = match &opts {
        Value::Obj(id) => e.get_prop(*id, "mode").map_or_else(
            || "shortest".to_string(),
            |v| e.to_js_string(v).unwrap_or_default(),
        ),
        _ => "shortest".to_string(),
    };
    let max = iters.iter().map(Vec::len).max().unwrap_or(0);
    let min = iters.iter().map(Vec::len).min().unwrap_or(0);
    let len = match mode.as_str() {
        "longest" => max,
        "strict" => {
            if max != min {
                return Err(e.range_error("zip strict requires same-length iterators"));
            }
            max
        }
        "shortest" => min,
        _ => return Err(e.type_error("unknown zip mode")),
    };
    let pad = match &opts {
        Value::Obj(id) => e.get_prop(*id, "padding").unwrap_or(Value::Undefined),
        _ => Value::Undefined,
    };
    let mut out = Vec::with_capacity(len);
    for i in 0..len {
        let row: Vec<Value> = iters
            .iter()
            .map(|it| it.get(i).cloned().unwrap_or_else(|| pad.clone()))
            .collect();
        out.push(e.new_array(row));
    }
    Ok(iter_obj(e, out, e.protos.iter_helper))
}

/// `Iterator.zipKeyed(obj, {mode?})` — proposal : chaque clé de
/// `obj` est un itérable → itérateur d'objets `{k: it_k[i]}`.
pub(crate) fn iterator_zip_keyed(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let Value::Obj(id) = arg(args, 0) else {
        return Err(e.type_error("zipKeyed expects an object of iterables"));
    };
    let keys = e.own_keys(id);
    let mut iters = Vec::with_capacity(keys.len());
    for k in &keys {
        let v = e.get_prop(id, k).unwrap_or(Value::Undefined);
        iters.push(e.iter_values(v)?);
    }
    let len = iters.iter().map(Vec::len).min().unwrap_or(0);
    let mut out = Vec::with_capacity(len);
    for i in 0..len {
        let obj = e.new_object();
        for (k, it) in keys.iter().zip(iters.iter()) {
            e.set_own(obj, k, it[i].clone(), true);
        }
        out.push(Value::Obj(obj));
    }
    Ok(iter_obj(e, out, e.protos.iter_helper))
}

/// Déplie l'itérable d'itérables en `Vec<Vec<Value>>`.
fn zip_iters(e: &mut Engine, v: Value) -> JsResult<Vec<Vec<Value>>> {
    let outer = e.iter_values(v)?;
    let mut iters = Vec::with_capacity(outer.len());
    for it in outer {
        iters.push(e.iter_values(it)?);
    }
    Ok(iters)
}
