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

use crate::heap::{JsResult, ObjKind, Object};
use crate::interp::SYM_ITERATOR;
use crate::value::{ObjId, Value};
use crate::Engine;

fn arg(args: &[Value], i: usize) -> Value {
    args.get(i).copied().unwrap_or(Value::Undefined)
}

/// `SameValueZero` — `===` plus `NaN` equals `NaN`.
fn svz(h: &crate::heap::Heap, a: Value, b: Value) -> bool {
    crate::interp::strict_eq(a, b, h)
        || matches!((a, b), (Value::Num(x), Value::Num(y)) if x.is_nan() && y.is_nan())
}

fn map_entries(e: &Engine, id: ObjId) -> Vec<(Value, Value)> {
    match e.heap.obj(id).map(|o| &o.kind) {
        Some(ObjKind::Map(m)) => m.clone(),
        _ => Vec::new(),
    }
}

fn set_items(e: &Engine, id: ObjId) -> Vec<Value> {
    match e.heap.obj(id).map(|o| &o.kind) {
        Some(ObjKind::Set(s)) => s.clone(),
        _ => Vec::new(),
    }
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
pub(crate) fn iter_obj(e: &mut Engine, items: Vec<Value>) -> Value {
    let proto = Value::Obj(e.protos.iterator);
    Value::Obj(e.heap.alloc_obj(Object {
        proto,
        props: Vec::new(),
        kind: ObjKind::Iter { items, idx: 0 },
        frozen: false,
    }))
}

// ------------------------------------------------------------------
// ctors
// ------------------------------------------------------------------

fn map_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let proto = Value::Obj(e.protos.map);
    let id = e.heap.alloc_obj(Object {
        proto,
        props: Vec::new(),
        kind: ObjKind::Map(Vec::new()),
        frozen: false,
    });
    let m = Value::Obj(id);
    // `new Map([[k, v], …])` — iterate the source, each entry must
    // be a 2-element pair.
    if !matches!(arg(args, 0), Value::Undefined | Value::Null) {
        for pair in e.iter_values(arg(args, 0))? {
            let k = e.get_prop_value(pair, "0")?;
            let v = e.get_prop_value(pair, "1")?;
            map_set_entry(e, id, k, v);
        }
    }
    Ok(m)
}

fn set_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let proto = Value::Obj(e.protos.set);
    let id = e.heap.alloc_obj(Object {
        proto,
        props: Vec::new(),
        kind: ObjKind::Set(Vec::new()),
        frozen: false,
    });
    let s = Value::Obj(id);
    if !matches!(arg(args, 0), Value::Undefined | Value::Null) {
        for v in e.iter_values(arg(args, 0))? {
            set_add(e, id, v);
        }
    }
    Ok(s)
}

fn map_set_entry(e: &mut Engine, id: ObjId, k: Value, v: Value) {
    // Two-phase: find the slot immutably, then mutate — `svz` needs
    // the heap for string comparison.
    let pos = e.heap.obj(id).and_then(|o| match &o.kind {
        ObjKind::Map(m) => m.iter().position(|(ek, _)| svz(&e.heap, *ek, k)),
        _ => None,
    });
    let Some(o) = e.heap.obj_mut(id) else { return };
    let ObjKind::Map(m) = &mut o.kind else { return };
    match pos {
        Some(i) => m[i].1 = v,
        None => m.push((k, v)),
    }
}

fn set_add(e: &mut Engine, id: ObjId, v: Value) {
    let has = e.heap.obj(id).is_some_and(|o| match &o.kind {
        ObjKind::Set(s) => s.iter().any(|&ev| svz(&e.heap, ev, v)),
        _ => true,
    });
    if has {
        return;
    }
    if let Some(o) = e.heap.obj_mut(id) {
        if let ObjKind::Set(s) = &mut o.kind {
            s.push(v);
        }
    }
}

// ------------------------------------------------------------------
// Map methods
// ------------------------------------------------------------------

fn map_set(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let Value::Obj(id) = this else {
        return Err(e.type_error("Map.set on non-Map"));
    };
    map_set_entry(e, id, arg(args, 0), arg(args, 1));
    Ok(this) // chaining
}

fn map_get(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let k = arg(args, 0);
    let v = map_entries(e, c_id(this))
        .into_iter()
        .find(|(ek, _)| svz(&e.heap, *ek, k))
        .map_or(Value::Undefined, |(_, v)| v);
    Ok(v)
}

fn map_has(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let k = arg(args, 0);
    Ok(Value::Bool(
        map_entries(e, c_id(this))
            .iter()
            .any(|(ek, _)| svz(&e.heap, *ek, k)),
    ))
}

fn map_delete(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let Value::Obj(id) = this else {
        return Ok(Value::Bool(false));
    };
    let k = arg(args, 0);
    let pos = e.heap.obj(id).and_then(|o| match &o.kind {
        ObjKind::Map(m) => m.iter().position(|(ek, _)| svz(&e.heap, *ek, k)),
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
    Ok(Value::Num(map_entries(e, c_id(this)).len() as f64))
}

/// Shared `forEach(cb)` for Map (`(v, k, map)`) and Set
/// (`(v, v, set)` per spec — key === value).
fn coll_for_each(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let f = arg(args, 0);
    let this_arg = arg(args, 1);
    if !e.is_callable(f) {
        return Err(e.type_error("forEach: callback is not a function"));
    }
    let Value::Obj(id) = this else {
        return Err(e.type_error("forEach on non-collection"));
    };
    let pairs: Vec<(Value, Value)> = match e.heap.obj(id).map(|o| &o.kind) {
        Some(ObjKind::Map(m)) => m.clone(),
        Some(ObjKind::Set(s)) => s.iter().map(|&v| (v, v)).collect(),
        _ => return Err(e.type_error("forEach on non-collection")),
    };
    for (k, v) in pairs {
        e.call_value(f, this_arg, &[v, k, this])?;
    }
    Ok(Value::Undefined)
}

fn map_keys(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let items: Vec<Value> = map_entries(e, c_id(this))
        .into_iter()
        .map(|(k, _)| k)
        .collect();
    Ok(iter_obj(e, items))
}

fn map_values(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let items: Vec<Value> = map_entries(e, c_id(this))
        .into_iter()
        .map(|(_, v)| v)
        .collect();
    Ok(iter_obj(e, items))
}

/// `map.entries()` — iterator of `[k, v]` arrays.
fn map_iter_entries(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let pairs = map_entries(e, c_id(this));
    let mut items = Vec::with_capacity(pairs.len());
    for (k, v) in pairs {
        items.push(e.new_array(vec![k, v]));
    }
    Ok(iter_obj(e, items))
}

// ------------------------------------------------------------------
// Set methods
// ------------------------------------------------------------------

fn set_add_m(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let Value::Obj(id) = this else {
        return Err(e.type_error("Set.add on non-Set"));
    };
    set_add(e, id, arg(args, 0));
    Ok(this)
}

fn set_has(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let v = arg(args, 0);
    Ok(Value::Bool(
        set_items(e, c_id(this))
            .iter()
            .any(|&ev| svz(&e.heap, ev, v)),
    ))
}

fn set_delete(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let Value::Obj(id) = this else {
        return Ok(Value::Bool(false));
    };
    let v = arg(args, 0);
    let pos = e.heap.obj(id).and_then(|o| match &o.kind {
        ObjKind::Set(s) => s.iter().position(|&ev| svz(&e.heap, ev, v)),
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
    Ok(Value::Num(set_items(e, c_id(this)).len() as f64))
}

/// `set.values()`/`keys()`/`entries()` + `[Symbol.iterator]`.
fn set_values_iter(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    Ok(iter_obj(e, set_items(e, c_id(this))))
}

fn set_entries_iter(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let items = set_items(e, c_id(this));
    let mut out = Vec::with_capacity(items.len());
    for v in items {
        out.push(e.new_array(vec![v, v]));
    }
    Ok(iter_obj(e, out))
}

// ------------------------------------------------------------------
// Iterator prototype — `next()` on ObjKind::Iter
// ------------------------------------------------------------------

fn iter_next(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let Value::Obj(id) = this else {
        return Err(e.type_error("next on non-iterator"));
    };
    let item = {
        let Some(o) = e.heap.obj_mut(id) else {
            return Err(e.type_error("next on non-iterator"));
        };
        match &mut o.kind {
            ObjKind::Iter { items, idx } => {
                if *idx < items.len() {
                    let v = items[*idx];
                    *idx += 1;
                    Some(v)
                } else {
                    None
                }
            }
            _ => return Err(e.type_error("next on non-iterator")),
        }
    };
    Ok(iter_result(
        e,
        item.unwrap_or(Value::Undefined),
        item.is_none(),
    ))
}

/// Iterator proto's `[Symbol.iterator]` returns `this`.
fn iter_self(_e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    Ok(this)
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
    {
        let self_fn = e.native_fn("[Symbol.iterator]", iter_self);
        e.set_own(iterator, SYM_ITERATOR, self_fn, false);
    }

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
    {
        let entries_fn = e.native_fn("entries", map_iter_entries);
        e.set_own(map_proto, SYM_ITERATOR, entries_fn, false);
        let size_fn = e.native_fn("size", map_size);
        if let Some(o) = e.heap.obj_mut(map_proto) {
            o.set_accessor("size", size_fn, Value::Undefined, false);
        }
    }
    let map_ctor_v = e.native_fn("Map", map_ctor);
    e.set_own(c_id(map_ctor_v), "prototype", Value::Obj(map_proto), false);
    let g = e.global;
    e.set_own(g, "Map", map_ctor_v, false);

    let set_proto = e.protos.set;
    e.add_method(set_proto, "add", set_add_m);
    e.add_method(set_proto, "has", set_has);
    e.add_method(set_proto, "delete", set_delete);
    e.add_method(set_proto, "clear", set_clear);
    e.add_method(set_proto, "forEach", coll_for_each);
    e.add_method(set_proto, "keys", set_values_iter);
    e.add_method(set_proto, "values", set_values_iter);
    e.add_method(set_proto, "entries", set_entries_iter);
    {
        let values_fn = e.native_fn("values", set_values_iter);
        e.set_own(set_proto, SYM_ITERATOR, values_fn, false);
        let size_fn = e.native_fn("size", set_size);
        if let Some(o) = e.heap.obj_mut(set_proto) {
            o.set_accessor("size", size_fn, Value::Undefined, false);
        }
    }
    let set_ctor_v = e.native_fn("Set", set_ctor);
    e.set_own(c_id(set_ctor_v), "prototype", Value::Obj(set_proto), false);
    let g = e.global;
    e.set_own(g, "Set", set_ctor_v, false);

    // `WeakMap`/`WeakSet` — alias the Map/Set ctors (no real weak
    // refs — the engine has no GC; deviation in ADR-0007).
    if let Some(wm) = e.get_prop(g, "Map") {
        e.set_own(g, "WeakMap", wm, false);
    }
    if let Some(ws) = e.get_prop(g, "Set") {
        e.set_own(g, "WeakSet", ws, false);
    }
}
