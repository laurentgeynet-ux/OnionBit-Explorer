// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Builtins JS — section extraite de `builtins.rs` (JS-C40, pur
//! reformatage mécanique : sémantique et tests inchangés).

#[allow(clippy::wildcard_imports)]
use super::*;

// ------------------------------------------------------------------
// Object
// ------------------------------------------------------------------

pub(crate) fn object_to_string(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let tag = match this {
        Value::Undefined | Value::Hole => "Undefined",
        Value::Null => "Null",
        Value::Num(_) => "Number",
        Value::Bool(_) => "Boolean",
        Value::Str(_) => "String",
        Value::Big(_) => "BigInt",
        Value::Obj(id) => match e.heap.obj(id).map(|o| &o.kind) {
            Some(ObjKind::Array(_)) => "Array",
            Some(ObjKind::Fun(_)) => "Function",
            Some(ObjKind::RegExp(_)) => "RegExp",
            Some(ObjKind::ByteBuf(_)) => "ArrayBuffer",
            Some(ObjKind::DataView { .. }) => "DataView",
            Some(ObjKind::TypedArray { ta, .. }) => ta.name(),
            Some(ObjKind::Host(h)) => return Ok(e.str_val(format!("[object {}]", h.class_name()))),
            _ => "Object",
        },
    };
    // `@@toStringTag` override (spec `Object.prototype.toString` —
    // les instances lisent leur tag propre ou hérité).
    if let Value::Obj(id) = this {
        if let Some(Value::Str(sid)) = e.get_prop(id, crate::interp::SYM_TO_STRING_TAG) {
            let s = e.heap.string(sid).unwrap_or_default();
            return Ok(e.str_val(format!("[object {s}]")));
        }
    }
    Ok(e.str_val(format!("[object {tag}]")))
}

pub(crate) fn has_own_property(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let key = match arg(args, 0) {
        Value::Num(n) => num_to_js_string(n),
        v => e.to_js_string(v)?,
    };
    let has = match this {
        // Proxy : `hasOwnProperty` passe par la trap
        // `getOwnPropertyDescriptor` (spec `[[GetOwnProperty]]`).
        Value::Obj(id)
            if matches!(e.heap.obj(id).map(|o| &o.kind), Some(ObjKind::Proxy { .. })) =>
        {
            e.get_own_prop(id, &key)?.is_some()
        }
        Value::Obj(id) => match e.heap.obj(id) {
            Some(o) => match &o.kind {
                ObjKind::Array(elems) => {
                    key == "length"
                        || crate::interp::array_index(&key)
                            .is_some_and(|i| elems.get(i).is_some_and(|v| !v.is_hole()))
                        || o.get_own(&key).is_some()
                }
                ObjKind::Fun(crate::heap::Fun::Script { .. }) => {
                    key == "length" || o.get_own(&key).is_some()
                }
                _ => o.get_own(&key).is_some(),
            },
            None => false,
        },
        // Primitif string : `length` et les indices sont des
        // props propres du wrapper `ToObject` (spec).
        Value::Str(sid) => {
            key == "length"
                || crate::interp::array_index(&key)
                    .is_some_and(|i| i < e.heap.string(sid).map_or(0, |s| u16_len(&s)))
        }
        _ => false,
    };
    Ok(Value::Bool(has))
}

pub(crate) fn object_value_of(_e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    Ok(this)
}

/// `o.isPrototypeOf(v)` — `this` se trouve-t-il sur la chaîne de
/// prototypes de `v` (bornée à 100 sauts).
pub(crate) fn object_is_prototype_of(
    e: &mut Engine,
    this: Value,
    args: &[Value],
) -> JsResult<Value> {
    let (Value::Obj(pid), Value::Obj(vid)) = (this, arg(args, 0)) else {
        return Ok(Value::Bool(false));
    };
    let mut cur = e.heap.obj(vid).map(|o| o.proto);
    let mut depth = 0;
    while let Some(Value::Obj(c)) = cur {
        if c == pid {
            return Ok(Value::Bool(true));
        }
        cur = e.heap.obj(c).map(|o| o.proto);
        depth += 1;
        if depth > 100 {
            break;
        }
    }
    Ok(Value::Bool(false))
}

pub(crate) fn object_keys(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    // `Object.keys('ab')` → `['0','1']` — indices énumérables de
    // la string-box (spec `ToObject`).
    if let Value::Str(sid) = arg(args, 0) {
        let n = e.heap.string(sid).map_or(0, |s| u16_len(&s));
        let vals: Vec<Value> = (0..n).map(|i| e.str_val(i.to_string())).collect();
        return Ok(arr_of(e, vals));
    }
    let Value::Obj(id) = arg(args, 0) else {
        return Ok(arr_of(e, Vec::new()));
    };
    let keys = e.own_enum_keys_trapped(id)?;
    let vals: Vec<Value> = keys.iter().map(|k| e.str_val(k.clone())).collect();
    Ok(arr_of(e, vals))
}

pub(crate) fn object_get_proto(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    match arg(args, 0) {
        // Proxy : trap `getPrototypeOf` (objet ou null requis),
        // sinon proto de la cible.
        Value::Obj(id) => {
            if let Some((tid, target, handler)) = e.proxy_th(id)? {
                let trap = e.proxy_trap(handler, "getPrototypeOf")?;
                if matches!(trap, Value::Obj(_)) {
                    let r = e.call_value(trap, handler, &[target])?;
                    match r {
                        Value::Obj(_) | Value::Null => {
                            // Spec : cible non-extensible → le trap
                            // doit rendre le proto de la cible.
                            if !e.target_extensible(tid) {
                                let tp = e.heap.obj(tid).map_or(Value::Null, |o| o.proto);
                                if !e.strict_eq(r, tp) {
                                    return Err(e.type_error(
                                        "proxy getPrototypeOf trap must return the target's prototype",
                                    ));
                                }
                            }
                            return Ok(r);
                        }
                        _ => {
                            return Err(e.type_error(
                                "proxy getPrototypeOf trap must return an object or null",
                            ));
                        }
                    }
                }
                return Ok(e.heap.obj(tid).map_or(Value::Null, |o| o.proto));
            }
            Ok(e.heap.obj(id).map_or(Value::Null, |o| o.proto))
        }
        _ => Ok(Value::Null),
    }
}

/// `Object.setPrototypeOf(o, p)` — `p` objet ou `null` ; refus sur
/// cycle de prototype et sur objet non-extensible (spec). Sur un
/// proxy : trap `setPrototypeOf` → `false` = `TypeError`.
pub(crate) fn object_set_proto(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let target = arg(args, 0);
    let proto = match arg(args, 1) {
        v @ Value::Obj(_) => v,
        Value::Null | Value::Undefined => Value::Null,
        _ => return Err(e.type_error("setPrototypeOf: proto must be an object or null")),
    };
    let Value::Obj(id) = target else {
        return Ok(target); // primitives are ignored, per spec
    };
    if let Some((tid, tv, handler)) = e.proxy_th(id)? {
        let trap = e.proxy_trap(handler, "setPrototypeOf")?;
        if matches!(trap, Value::Obj(_)) {
            let ok = e.call_value(trap, handler, &[tv, proto])?;
            if !e.to_boolean(ok) {
                return Err(e.type_error("proxy setPrototypeOf trap returned false"));
            }
            // Spec : cible non-extensible → le proto demandé doit
            // être identique à celui de la cible.
            if !e.target_extensible(tid) {
                let tp = e.heap.obj(tid).map_or(Value::Null, |o| o.proto);
                if !e.strict_eq(proto, tp) {
                    return Err(e.type_error(
                        "proxy setPrototypeOf trap returned true without changing the prototype",
                    ));
                }
            }
            return Ok(target);
        }
        return object_set_proto(e, Value::Undefined, &[Value::Obj(tid), proto]);
    }
    if !e.heap.obj(id).is_some_and(|o| o.extensible) {
        return Err(e.type_error("cannot set prototype of non-extensible object"));
    }
    // Cycle check : remonter la chaîne de `proto` jusqu'à `id`.
    let mut cur = proto;
    while let Value::Obj(cid) = cur {
        if cid == id {
            return Err(e.type_error("cyclic prototype value"));
        }
        cur = e.heap.obj(cid).map_or(Value::Null, |o| o.proto);
    }
    if let Some(o) = e.heap.obj_mut(id) {
        o.proto = proto;
    }
    Ok(target)
}

pub(crate) fn object_values(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let Value::Obj(id) = arg(args, 0) else {
        return Ok(arr_of(e, Vec::new()));
    };
    let keys = e.own_enum_keys_trapped(id)?;
    let mut vals = Vec::with_capacity(keys.len());
    for k in keys {
        // Getter-aware read (class accessors are own props).
        vals.push(e.get_prop_value(Value::Obj(id), &k)?);
    }
    Ok(arr_of(e, vals))
}

/// `Object.assign(t, …srcs)` — enumerable-own copy, accessor-aware
/// read and write (setters on the target fire, per spec).
pub(crate) fn object_assign(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let target = arg(args, 0);
    for src in &args[1.min(args.len())..] {
        if let Value::Obj(sid) = src {
            for k in e.own_enum_keys_trapped(*sid)? {
                let v = e.get_prop_value(*src, &k)?;
                e.set_prop_value(target, &k, v)?;
            }
        }
    }
    Ok(target)
}

/// `Object.create(proto[, propsMap])` — proto must be an object or
/// `null`; the optional map applies `defineProperties` semantics.
pub(crate) fn object_create(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let proto = match arg(args, 0) {
        v @ Value::Obj(_) => v,
        Value::Null | Value::Undefined => Value::Null,
        _ => return Err(e.type_error("Object.create: proto must be an object")),
    };
    let id = e.heap.alloc_obj(Object::ordinary(proto));
    if let Value::Obj(_) = arg(args, 1) {
        define_props_from_map(e, id, arg(args, 1))?;
    }
    Ok(Value::Obj(id))
}

/// `Object.defineProperty(o, k, desc)` — descripteur complet :
/// `value`/`writable`/`enumerable`/`configurable`/`get`/`set`.
/// Règles simplifiées de `[[DefineOwnProperty]]` (heap.rs) :
/// non-configurable → rejet `TypeError`.
pub(crate) fn object_define_property(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let target @ Value::Obj(id) = arg(args, 0) else {
        return Err(e.type_error("Object.defineProperty on non-object"));
    };
    let key = match arg(args, 1) {
        Value::Num(n) => num_to_js_string(n),
        v => e.to_js_string(v)?,
    };
    let Value::Obj(desc_id) = arg(args, 2) else {
        return Err(e.type_error("property descriptor must be an object"));
    };
    define_one_prop(e, id, &key, desc_id)?;
    Ok(target)
}

/// Corps de `[[DefineOwnProperty]]` piloté par un objet descripteur —
/// partagé par `defineProperty`/`defineProperties`/`Object.create`.
/// Sur un proxy : trap `defineProperty` ou délégation à la cible.
pub(crate) fn define_one_prop(
    e: &mut Engine,
    id: ObjId,
    key: &str,
    desc_id: ObjId,
) -> JsResult<()> {
    if let Some((tid, target, handler)) = e.proxy_th(id)? {
        let trap = e.proxy_trap(handler, "defineProperty")?;
        if matches!(trap, Value::Obj(_)) {
            let kv = e.str_val(key);
            let r = e.call_value(trap, handler, &[target, kv, Value::Obj(desc_id)])?;
            return if e.to_boolean(r) {
                // Invariants spec `[[DefineOwnProperty]]` : nouvelle
                // prop sur cible non-extensible → TypeError ; prop
                // non-config ou absente rapportée configurable:false.
                let tgt_exists = e.find_own(tid, key).is_some();
                let tgt_ext = e.target_extensible(tid);
                if !tgt_exists && !tgt_ext {
                    return Err(e.type_error(
                        "proxy defineProperty: cannot add prop on non-extensible target",
                    ));
                }
                Ok(())
            } else {
                Err(e.type_error(format!("cannot define property `{key}`")))
            };
        }
        return define_one_prop(e, tid, key, desc_id);
    }
    let desc = Value::Obj(desc_id);
    let (has_value, has_writable, has_enumerable, has_configurable, has_get, has_set) = {
        let o = e.heap.obj(desc_id);
        let h = |k: &str| o.is_some_and(|o| o.find(k).is_some());
        (
            h("value"),
            h("writable"),
            h("enumerable"),
            h("configurable"),
            h("get"),
            h("set"),
        )
    };
    let cur = e.find_own(id, key);
    // Flags absents du descripteur → valeur de la prop existante,
    // `false` sinon (spec `[[DefineOwnProperty]]`).
    let enumerable = if has_enumerable {
        let v = e.get_prop_value(desc, "enumerable")?;
        e.to_boolean(v)
    } else {
        cur.is_some_and(|p| p.enumerable())
    };
    let configurable = if has_configurable {
        let v = e.get_prop_value(desc, "configurable")?;
        e.to_boolean(v)
    } else {
        cur.is_some_and(|p| p.configurable())
    };
    let writable = if has_writable {
        let v = e.get_prop_value(desc, "writable")?;
        e.to_boolean(v)
    } else {
        cur.is_some_and(|p| matches!(p, crate::heap::Prop::Data { attrs, .. } if attrs.writable))
    };
    let get = e.get_prop_value(desc, "get")?;
    let set = e.get_prop_value(desc, "set")?;
    let has_acc = has_get || has_set;
    let has_data = has_value || has_writable;
    if has_acc && has_data {
        return Err(e.type_error("descriptor cannot be both data and accessor"));
    }
    if has_acc && !matches!(get, Value::Undefined) && !e.is_callable(get)
        || has_acc && !matches!(set, Value::Undefined) && !e.is_callable(set)
    {
        return Err(e.type_error("accessor descriptor `get`/`set` must be callable"));
    }
    let attrs = crate::heap::PropAttrs {
        writable,
        enumerable,
        configurable,
    };
    let v = if has_value {
        e.get_prop_value(desc, "value")?
    } else {
        cur.and_then(|p| p.data_value()).unwrap_or(Value::Undefined)
    };
    // Côté absent : getter/setter existant conservé.
    let (g, s) = match cur {
        Some(crate::heap::Prop::Acc { get: g, set: s, .. }) => {
            (if has_get { get } else { g }, if has_set { set } else { s })
        }
        _ => (get, set),
    };
    // Indice de tableau : l'élément vit dans `ObjKind::Array`,
    // pas dans `props` — route dédiée. Les attributs posés sont
    // contraints par l'intégrité (seal/freeze) : un descripteur
    // incompatible est un `TypeError` (spec [[DefineOwnProperty]]).
    // `defineProperty(arr, 'length', …)` — `length` est synthétisée,
    // pas dans `props` : route dédiée (valeur → resize, verrou
    // `writable:false` → bit 4, le reste est fixe).
    if key == "length" {
        let is_arr = matches!(e.heap.obj(id).map(|o| &o.kind), Some(ObjKind::Array(_)));
        if is_arr {
            let (integ, len) =
                e.heap
                    .obj(id)
                    .map(|o| (o.elem_integrity, 0usize))
                    .map_or((0, 0), |(i, _)| {
                        let l = match e.heap.obj(id).map(|o| &o.kind) {
                            Some(ObjKind::Array(elems)) => elems.len(),
                            _ => 0,
                        };
                        (i, l)
                    });
            if has_enumerable && enumerable || has_configurable && configurable {
                return Err(e.type_error("cannot reconfigure array `length`"));
            }
            if integ & 4 != 0 {
                if has_writable && writable {
                    return Err(e.type_error("cannot reconfigure array `length`"));
                }
                if has_value {
                    let n = match v {
                        Value::Num(x) if x >= 0.0 && x.fract() == 0.0 => x as usize,
                        _ => return Err(e.range_error("invalid array length")),
                    };
                    if n != len {
                        return Err(e.type_error("cannot change locked array `length`"));
                    }
                }
                return Ok(());
            }
            if has_value {
                let n = match v {
                    Value::Num(x) if x >= 0.0 && x.fract() == 0.0 => x as usize,
                    _ => return Err(e.range_error("invalid array length")),
                };
                if integ & 3 != 0 && n < len {
                    return Err(e.type_error("cannot shrink a sealed array"));
                }
                // Garde d'alloc avant densification (JS-C55).
                if n > len {
                    e.alloc_guard((n - len).saturating_mul(std::mem::size_of::<Value>()))?;
                }
                if let Some(ObjKind::Array(elems)) = e.heap.obj_mut(id).map(|o| &mut o.kind) {
                    elems.resize(n, Value::Hole);
                }
            }
            if has_writable && !writable {
                if let Some(o) = e.heap.obj_mut(id) {
                    o.elem_integrity |= 4;
                }
            }
            return Ok(());
        }
    }
    if let Some(i) = crate::interp::array_index(key) {
        let is_arr = matches!(e.heap.obj(id).map(|o| &o.kind), Some(ObjKind::Array(_)));
        if is_arr {
            let (integ, len, cur_v) = e
                .heap
                .obj(id)
                .map(|o| {
                    if let ObjKind::Array(elems) = &o.kind {
                        (o.elem_integrity, elems.len(), elems.get(i).copied())
                    } else {
                        (0, 0, None)
                    }
                })
                .unwrap_or_default();
            let _ = cur_v;
            if has_acc {
                return Err(e.type_error("array index cannot be an accessor"));
            }
            // Descripteur demandant `configurable:true` sur élément
            // sealé/gelé, `writable:true` sur gelé, ou une autre
            // valeur sur gelé → refus spec.
            if integ & 3 != 0 && has_configurable && configurable {
                return Err(e.type_error("cannot reconfigure sealed array element"));
            }
            if integ & 2 != 0 && has_writable && writable {
                return Err(e.type_error("cannot reconfigure frozen array element"));
            }
            if integ & 2 != 0 && has_value && !e.strict_eq(v, cur_v.unwrap_or(Value::Undefined)) {
                return Err(e.type_error("cannot redefine frozen array element"));
            }
            // Hors-bornes → ajout interdit (cible non-extensible
            // sous seal/freeze ; sinon extension normale via set).
            if i >= len && integ & 3 != 0 {
                return Err(e.type_error("cannot add element to sealed array"));
            }
            if i < len && has_value {
                e.set_prop(id, &i.to_string(), v)?;
            }
            return Ok(());
        }
    }
    let ok = e.heap.obj_mut(id).is_some_and(|o| {
        if has_acc {
            o.define_accessor(key, g, s, attrs)
        } else {
            o.define_data(key, v, attrs)
        }
    });
    if !ok {
        return Err(e.type_error(format!("cannot define property `{key}`")));
    }
    Ok(())
}

/// `Object.defineProperties(o, propsMap)` — chaque propre prop
/// énumérable de la map est un descripteur appliqué à `o`.
pub(crate) fn object_define_properties(
    e: &mut Engine,
    _t: Value,
    args: &[Value],
) -> JsResult<Value> {
    let target @ Value::Obj(id) = arg(args, 0) else {
        return Err(e.type_error("Object.defineProperties on non-object"));
    };
    define_props_from_map(e, id, arg(args, 1))?;
    Ok(target)
}

/// Map de descripteurs → définitions (ordre d'insertion conservé).
pub(crate) fn define_props_from_map(e: &mut Engine, id: ObjId, map: Value) -> JsResult<()> {
    let Value::Obj(mid) = map else {
        return Err(e.type_error("property map must be an object"));
    };
    for k in e.own_enum_keys_trapped(mid)? {
        let dv = e.get_prop_value(map, &k)?;
        let Value::Obj(did) = dv else {
            return Err(e.type_error(format!("descriptor for `{k}` must be an object")));
        };
        define_one_prop(e, id, &k, did)?;
    }
    Ok(())
}

/// `Object.getOwnPropertyDescriptor(o, k)` — `{value, writable,
/// enumerable, configurable}` ou `{get, set, enumerable,
/// configurable}`, `undefined` quand la prop est absente.
pub(crate) fn object_get_own_property_descriptor(
    e: &mut Engine,
    _t: Value,
    args: &[Value],
) -> JsResult<Value> {
    let target = arg(args, 0);
    let key = match arg(args, 1) {
        Value::Num(n) => num_to_js_string(n),
        v => e.to_js_string(v)?,
    };
    // Descripteurs propres de la string primitive (spec
    // `ToObject` + `StringGetOwnProperty`).
    if let Value::Str(sid) = target {
        let n = e.heap.string(sid).map_or(0, |s| u16_len(&s));
        let d = e.new_object();
        if key == "length" {
            e.set_own(d, "value", Value::Num(n as f64), true);
            e.set_own(d, "writable", Value::Bool(false), true);
            e.set_own(d, "enumerable", Value::Bool(false), true);
            e.set_own(d, "configurable", Value::Bool(false), true);
            return Ok(Value::Obj(d));
        }
        if let Some(i) = crate::interp::array_index(&key) {
            if i < n {
                let c = e
                    .heap
                    .string(sid)
                    .and_then(|s| u16_at(&s, i))
                    .unwrap_or_default();
                let cv = e.str_val(c);
                e.set_own(d, "value", cv, true);
                e.set_own(d, "writable", Value::Bool(false), true);
                e.set_own(d, "enumerable", Value::Bool(true), true);
                e.set_own(d, "configurable", Value::Bool(false), true);
                return Ok(Value::Obj(d));
            }
        }
        return Ok(Value::Undefined);
    }
    let Value::Obj(id) = target else {
        return Ok(Value::Undefined);
    };
    let Some(p) = e.get_own_prop(id, &key)? else {
        return Ok(Value::Undefined);
    };
    Ok(prop_desc_object(e, p))
}

/// Property descriptor object from a `Prop` — shared by
/// `Object.getOwnPropertyDescriptor` and `Reflect.*`.
pub(crate) fn prop_desc_object(e: &mut Engine, p: crate::heap::Prop) -> Value {
    let d = e.new_object();
    match p {
        crate::heap::Prop::Data { value, attrs } => {
            e.set_own(d, "value", value, true);
            e.set_own(d, "writable", Value::Bool(attrs.writable), true);
            e.set_own(d, "enumerable", Value::Bool(attrs.enumerable), true);
            e.set_own(d, "configurable", Value::Bool(attrs.configurable), true);
        }
        crate::heap::Prop::Acc { get, set, attrs } => {
            e.set_own(d, "get", get, true);
            e.set_own(d, "set", set, true);
            e.set_own(d, "enumerable", Value::Bool(attrs.enumerable), true);
            e.set_own(d, "configurable", Value::Bool(attrs.configurable), true);
        }
    }
    Value::Obj(d)
}

/// `Object.getOwnPropertyDescriptors(o)` — map `key → descriptor`
/// pour chaque propre prop (ordre d'insertion conservé).
pub(crate) fn object_get_own_property_descriptors(
    e: &mut Engine,
    _t: Value,
    args: &[Value],
) -> JsResult<Value> {
    let out = e.new_object();
    let Value::Obj(id) = arg(args, 0) else {
        return Ok(Value::Obj(out));
    };
    let keys: Vec<String> =
        if matches!(e.heap.obj(id).map(|o| &o.kind), Some(ObjKind::Proxy { .. })) {
            e.own_keys_trapped(id)?
        } else {
            match e.heap.obj(id) {
                Some(o) => {
                    let mut ks: Vec<String> = o.props.iter().map(|(k, _)| k.clone()).collect();
                    if let ObjKind::Array(elems) = &o.kind {
                        ks.splice(0..0, (0..elems.len()).map(|i| i.to_string()));
                        ks.insert(elems.len(), "length".into());
                    }
                    ks
                }
                None => Vec::new(),
            }
        };
    for k in keys {
        // Clés internes `\u{f131}…` (privés, métadonnées classe) :
        // jamais exposées comme descripteurs.
        if k.starts_with('\u{f131}') {
            continue;
        }
        let kv = e.str_val(k.clone());
        let d = object_get_own_property_descriptor(e, Value::Undefined, &[arg(args, 0), kv])?;
        e.set_own(out, &k, d, true);
    }
    Ok(Value::Obj(out))
}

/// `Object.freeze` — seal + toutes data props non-writable.
/// Sur un proxy : même approximation que `seal` (trap
/// `preventExtensions` ou cible).
pub(crate) fn object_freeze(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    if let Value::Obj(id) = arg(args, 0) {
        if matches!(e.heap.obj(id).map(|o| &o.kind), Some(ObjKind::Proxy { .. })) {
            return object_prevent_extensions(e, Value::Undefined, &[arg(args, 0)]);
        }
        if let Some(o) = e.heap.obj_mut(id) {
            // `freeze` sur vue typée non vide → TypeError (spec).
            if let ObjKind::TypedArray { len, .. } = &o.kind {
                if *len > 0 {
                    let err = e.type_error("cannot freeze a typed array");
                    return Err(err);
                }
            }
            o.extensible = false;
            o.elem_integrity |= 6;
            for (_, p) in &mut o.props {
                match p {
                    crate::heap::Prop::Data { attrs, .. } => {
                        attrs.configurable = false;
                        attrs.writable = false;
                    }
                    crate::heap::Prop::Acc { attrs, .. } => attrs.configurable = false,
                }
            }
        }
    }
    Ok(arg(args, 0))
}

pub(crate) fn object_is_frozen(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    Ok(Value::Bool(match arg(args, 0) {
        Value::Obj(id) => e.heap.obj(id).is_some_and(|o| {
            !o.extensible
                && o.props
                    .iter()
                    .all(|(_, p)| !p.configurable() && !p.writable())
                && match &o.kind {
                    ObjKind::Array(elems) => elems.is_empty() || o.elem_integrity & 6 == 6,
                    _ => true,
                }
        }),
        _ => true, // primitives are frozen by nature
    }))
}

/// `Object.is` — `SameValue` (`===` plus NaN and ±0 distinction).
pub(crate) fn object_is(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let (a, b) = (arg(args, 0), arg(args, 1));
    if let (Value::Num(x), Value::Num(y)) = (a, b) {
        if x.is_nan() && y.is_nan() {
            return Ok(Value::Bool(true));
        }
        if x == 0.0 && y == 0.0 {
            return Ok(Value::Bool(x.is_sign_negative() == y.is_sign_negative()));
        }
    }
    Ok(Value::Bool(e.strict_eq(a, b)))
}
