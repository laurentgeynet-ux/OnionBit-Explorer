// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Builtins JS — section extraite de `builtins.rs` (JS-C40, pur
//! reformatage mécanique : sémantique et tests inchangés).

#[allow(clippy::wildcard_imports)]
use super::*;

// ---- BigInt (JS-C7) --------------------------------------------------

/// `this` d'une méthode `BigInt.prototype` : primitive `Value::Big`
/// ou wrapper `Object(1n)`/`new` portant `__prim`.
pub(crate) fn bigint_this(e: &mut Engine, this: Value) -> JsResult<num_bigint::BigInt> {
    let prim = match this {
        v @ Value::Big(_) => v,
        Value::Obj(id) => {
            let own = e.heap.obj(id).and_then(|o| o.get_own(PRIM));
            match own {
                Some(p @ Value::Big(_)) => p,
                _ => {
                    return Err(
                        e.type_error("BigInt.prototype method called on incompatible receiver")
                    );
                }
            }
        }
        _ => {
            return Err(e.type_error("BigInt.prototype method called on incompatible receiver"));
        }
    };
    match e.bigint_of(prim) {
        Some(b) => Ok(b),
        None => Err(e.type_error("invalid BigInt value")),
    }
}

/// `BigInt(x)` — fonctionnel uniquement (`new` → `TypeError`, spec
/// `isConstructor` interdit). `x` numérique doit être entier.
pub(crate) fn bigint_ctor(e: &mut Engine, t: Value, args: &[Value]) -> JsResult<Value> {
    if constructed_this(e, t, e.protos.bigint).is_some() {
        return Err(e.type_error("BigInt is not a constructor"));
    }
    let v = arg(args, 0);
    if let Value::Num(n) = v {
        if !n.is_finite() || n.fract() != 0.0 {
            return Err(e.range_error("the number is not a safe integer"));
        }
        let b = num_traits::FromPrimitive::from_f64(n).unwrap_or_else(num_bigint::BigInt::default);
        return Ok(e.big_val(b));
    }
    let prim = if let Value::Obj(_) = v {
        e.to_primitive(v, crate::interp::PrimHint::Number)?
    } else {
        v
    };
    // `to_bigint` produit le SyntaxError spec pour les chaînes
    // non-parsables (`StringToBigInt`).
    let b = e.to_bigint(prim)?;
    Ok(e.big_val(b))
}

pub(crate) fn bigint_to_string(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let b = bigint_this(e, this)?;
    let radix = match arg(args, 0) {
        Value::Undefined => 10,
        v => {
            let n = e.to_number(v)?;
            if !(2.0..=36.0).contains(&n) || n.fract() != 0.0 {
                return Err(e.range_error("toString() radix must be between 2 and 36"));
            }
            n as u32
        }
    };
    Ok(e.str_val(b.to_str_radix(radix)))
}

pub(crate) fn bigint_value_of(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let b = bigint_this(e, this)?;
    Ok(e.big_val(b))
}

/// `ToIndex` simplifié : entier ≥ 0 (borné à 2^53-1 implicite f64).
pub(crate) fn to_bits(e: &mut Engine, v: Value) -> JsResult<u32> {
    let n = e.to_number(v)?;
    if !n.is_finite() || n < 0.0 || n.fract() != 0.0 {
        return Err(e.range_error("invalid bigint bit width"));
    }
    Ok(n as u32)
}

pub(crate) fn bigint_as_int_n(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let bits = to_bits(e, arg(args, 0))?;
    let b = e.to_bigint(arg(args, 1))?;
    if bits == 0 {
        return Ok(e.big_val(num_bigint::BigInt::from(0)));
    }
    let m = num_bigint::BigInt::from(1) << bits;
    let mut r = &b % &m;
    if r.sign() == num_bigint::Sign::Minus {
        r += &m;
    }
    let half = &m >> 1u32;
    if r >= half {
        r -= &m;
    }
    Ok(e.big_val(r))
}

pub(crate) fn bigint_as_uint_n(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let bits = to_bits(e, arg(args, 0))?;
    let b = e.to_bigint(arg(args, 1))?;
    if bits == 0 {
        return Ok(e.big_val(num_bigint::BigInt::from(0)));
    }
    let m = num_bigint::BigInt::from(1) << bits;
    let mut r = &b % &m;
    if r.sign() == num_bigint::Sign::Minus {
        r += &m;
    }
    Ok(e.big_val(r))
}

/// `Symbol.for(key)` — global registry on the engine (survives
/// heap traffic; `===` on a `for` symbol is identity-stable).
pub(crate) fn symbol_for(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let key = e.to_js_string(arg(args, 0))?;
    if let Some(v) = e.symbol_reg.get(&key) {
        return Ok(*v);
    }
    // `Symbol.for` symbols carry their key as description (spec).
    let sym = symbol_ctor(e, Value::Undefined, args)?;
    e.symbol_reg.insert(key, sym);
    Ok(sym)
}

/// `Symbol.keyFor(sym)` — reverse lookup over the registry.
pub(crate) fn symbol_key_for(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let sym = arg(args, 0);
    let hit = e
        .symbol_reg
        .iter()
        .find(|(_, v)| e.strict_eq(**v, sym))
        .map(|(k, _)| k.clone());
    Ok(hit.map_or(Value::Undefined, |k| e.str_val(k)))
}

pub(crate) fn sym_desc(e: &Engine, this: Value) -> Option<String> {
    let Value::Obj(id) = this else { return None };
    match e.heap.obj(id).map(|o| &o.kind) {
        Some(ObjKind::Symbol { desc, .. }) => Some(desc.clone().unwrap_or_default()),
        _ => None,
    }
}

pub(crate) fn symbol_to_string(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let d =
        sym_desc(e, this).ok_or_else(|| e.type_error("Symbol.prototype.toString on non-symbol"))?;
    Ok(e.str_val(if d.is_empty() {
        "Symbol()".to_string()
    } else {
        format!("Symbol({d})")
    }))
}

pub(crate) fn symbol_value_of(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    if sym_desc(e, this).is_none() {
        return Err(e.type_error("Symbol.prototype.valueOf on non-symbol"));
    }
    Ok(this)
}

/// `Symbol.prototype.description` getter.
pub(crate) fn symbol_description(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    match sym_desc(e, this) {
        Some(d) if !d.is_empty() => Ok(e.str_val(d)),
        Some(_) => Ok(Value::Undefined),
        None => Err(e.type_error("description on non-symbol")),
    }
}

// -- Reflect ---------------------------------------------------------

pub(crate) fn reflect_get(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let Value::Obj(id) = arg(args, 0) else {
        return Err(e.type_error("Reflect.get on non-object"));
    };
    let k = e.prop_key(arg(args, 1))?;
    e.get_prop_value(Value::Obj(id), &k)
}

pub(crate) fn reflect_set(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let Value::Obj(id) = arg(args, 0) else {
        return Err(e.type_error("Reflect.set on non-object"));
    };
    let k = e.prop_key(arg(args, 1))?;
    e.set_prop_value(Value::Obj(id), &k, arg(args, 2))?;
    Ok(Value::Bool(true))
}

pub(crate) fn reflect_has(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let Value::Obj(id) = arg(args, 0) else {
        return Ok(Value::Bool(false));
    };
    let k = e.prop_key(arg(args, 1))?;
    Ok(Value::Bool(e.has_prop(id, &k)?))
}

pub(crate) fn reflect_own_keys(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let Value::Obj(id) = arg(args, 0) else {
        return Ok(e.new_array(Vec::new()));
    };
    let keys = e.own_keys_trapped(id)?;
    // Clés-symboles `\u{f130}…` rendues ; internes `\u{f131}…` filtrées.
    let vals = keys
        .iter()
        .filter(|k| !k.starts_with('\u{f131}'))
        .map(|k| e.str_val(k.clone()))
        .collect();
    Ok(e.new_array(vals))
}

pub(crate) fn reflect_apply(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let f = arg(args, 0);
    let this = arg(args, 1);
    let list = match arg(args, 2) {
        Value::Obj(id) => e.iter_values(Value::Obj(id))?,
        _ => Vec::new(),
    };
    e.call_value(f, this, &list)
}

pub(crate) fn reflect_construct(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let f = arg(args, 0);
    let list = match arg(args, 1) {
        Value::Obj(id) => e.iter_values(Value::Obj(id))?,
        _ => Vec::new(),
    };
    // `newTarget` optionnel — `new.target` vu par `f` (spec :
    // défaut = la cible).
    let nt = match arg(args, 2) {
        Value::Undefined | Value::Null => f,
        v => v,
    };
    e.construct_nt(f, &list, nt)
}

/// `Reflect.defineProperty(t, k, desc)` — mêmes règles que
/// `Object.defineProperty` mais rend `false` sur échec au lieu de
/// jeter (spec `Reflect.defineProperty` — les erreurs internes de
/// `[[DefineOwnProperty]]` surface en `false`, déviation documentée :
/// un getter de descripteur qui jette est aussi rendu `false`).
pub(crate) fn reflect_define_property(
    e: &mut Engine,
    _t: Value,
    args: &[Value],
) -> JsResult<Value> {
    let Value::Obj(id) = arg(args, 0) else {
        return Err(e.type_error("Reflect.defineProperty on non-object"));
    };
    let k = e.prop_key(arg(args, 1))?;
    let Value::Obj(desc) = arg(args, 2) else {
        return Err(e.type_error("Reflect.defineProperty: descriptor is not an object"));
    };
    Ok(Value::Bool(define_one_prop(e, id, &k, desc).is_ok()))
}

/// `Reflect.deleteProperty(t, k)` — `false` si la prop refuse,
/// jamais de `TypeError` de suppression (contrairement au `delete`
/// strict).
pub(crate) fn reflect_delete_property(
    e: &mut Engine,
    _t: Value,
    args: &[Value],
) -> JsResult<Value> {
    let Value::Obj(id) = arg(args, 0) else {
        return Err(e.type_error("Reflect.deleteProperty on non-object"));
    };
    let k = e.prop_key(arg(args, 1))?;
    // `delete_result` jette en mode strict sur refus — pour
    // `Reflect` tout échec se rabat sur `false` (déviation
    // documentée : une trap `deleteProperty` qui jette est aussi
    // rendue `false`).
    Ok(Value::Bool(
        e.delete_result(id, &k).is_ok_and(|v| e.to_boolean(v)),
    ))
}

/// `Reflect.getOwnPropertyDescriptor(t, k)` — objet descripteur ou
/// `undefined` ; passe par la trap `getOwnPropertyDescriptor`.
pub(crate) fn reflect_get_own_property_descriptor(
    e: &mut Engine,
    _t: Value,
    args: &[Value],
) -> JsResult<Value> {
    let Value::Obj(id) = arg(args, 0) else {
        return Err(e.type_error("Reflect.getOwnPropertyDescriptor on non-object"));
    };
    let k = e.prop_key(arg(args, 1))?;
    let Some(p) = e.get_own_prop(id, &k)? else {
        return Ok(Value::Undefined);
    };
    Ok(prop_desc_object(e, p))
}

/// `Reflect.getPrototypeOf(t)` — `TypeError` sur primitive
/// (contrairement à `Object.getPrototypeOf` qui ignore).
pub(crate) fn reflect_get_prototype_of(
    e: &mut Engine,
    _t: Value,
    args: &[Value],
) -> JsResult<Value> {
    match arg(args, 0) {
        v @ Value::Obj(_) => object_get_proto(e, Value::Undefined, &[v]),
        _ => Err(e.type_error("Reflect.getPrototypeOf on non-object")),
    }
}

/// `Reflect.setPrototypeOf(t, p)` — `false` sur refus (cycle,
/// non-extensible, trap `false`), `TypeError` sur args invalides.
pub(crate) fn reflect_set_prototype_of(
    e: &mut Engine,
    _t: Value,
    args: &[Value],
) -> JsResult<Value> {
    let target = arg(args, 0);
    if !matches!(target, Value::Obj(_)) {
        return Err(e.type_error("Reflect.setPrototypeOf on non-object"));
    }
    if !matches!(arg(args, 1), Value::Obj(_) | Value::Null) {
        return Err(e.type_error("Reflect.setPrototypeOf: proto must be an object or null"));
    }
    match object_set_proto(e, Value::Undefined, &[target, arg(args, 1)]) {
        Ok(_) => Ok(Value::Bool(true)),
        Err(_) => Ok(Value::Bool(false)),
    }
}

/// `Reflect.isExtensible(t)` — `TypeError` sur primitive.
pub(crate) fn reflect_is_extensible(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    match arg(args, 0) {
        v @ Value::Obj(_) => object_is_extensible(e, Value::Undefined, &[v]),
        _ => Err(e.type_error("Reflect.isExtensible on non-object")),
    }
}

/// `Reflect.preventExtensions(t)` — `true` si l'opération a réussi,
/// `false` sur refus (trap `false` ou cible déjà scellée).
pub(crate) fn reflect_prevent_extensions(
    e: &mut Engine,
    _t: Value,
    args: &[Value],
) -> JsResult<Value> {
    match arg(args, 0) {
        v @ Value::Obj(_) => Ok(Value::Bool(
            object_prevent_extensions(e, Value::Undefined, &[v]).is_ok(),
        )),
        _ => Err(e.type_error("Reflect.preventExtensions on non-object")),
    }
}

// -- Proxy ------------------------------------------------------------

/// Alloue `new Proxy(target, handler)` — les deux arguments doivent
/// être des objets (spec `Proxy` step 1-2).
pub(crate) fn new_proxy(e: &mut Engine, target: Value, handler: Value) -> JsResult<Value> {
    if !matches!(target, Value::Obj(_)) || !matches!(handler, Value::Obj(_)) {
        return Err(e.type_error("Proxy target and handler must be objects"));
    }
    // Proto apparent du proxy = celui de la cible (approximation
    // pour les marches `for-in`/`instanceof` qui ne passent pas
    // par la trap `getPrototypeOf`).
    let proto = match target {
        Value::Obj(tid) => e.heap.obj(tid).map_or(Value::Null, |o| o.proto),
        _ => Value::Null,
    };
    Ok(Value::Obj(e.heap.alloc_obj(Object {
        proto,
        props: Vec::new(),
        kind: ObjKind::Proxy {
            target,
            handler: Some(handler),
        },
        extensible: true,
        elem_integrity: 0,
        args_map: None,
    })))
}

/// `new Proxy(t, h)` — construct-only : l'appel fonctionnel lève
/// `TypeError` (heuristique `this` = objet frais sur
/// `Proxy.prototype`, même technique que `String`/`Number`).
pub(crate) fn proxy_ctor(e: &mut Engine, t: Value, args: &[Value]) -> JsResult<Value> {
    if constructed_this(e, t, e.protos.proxy).is_none() {
        return Err(e.type_error("Proxy constructor must be called with `new`"));
    }
    new_proxy(e, arg(args, 0), arg(args, 1))
}

/// `Proxy.revocable(t, h)` → `{proxy, revoke}` — `revoke` est une
/// native liée au proxy (`BoundNative`) qui nulle son handler.
pub(crate) fn proxy_revocable(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let p = new_proxy(e, arg(args, 0), arg(args, 1))?;
    let revoke = e.bound_native("revoke", proxy_revoke, vec![p]);
    let out = e.new_object();
    e.set_own(out, "proxy", p, true);
    e.set_own(out, "revoke", revoke, true);
    Ok(Value::Obj(out))
}

/// `revoke()` — l'objet proxy est passé en arg lié (`bound[0]`) ;
/// après appel, toute opération sur le proxy lève `TypeError`.
pub(crate) fn proxy_revoke(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    if let Some(Value::Obj(id)) = args.first() {
        if let Some(o) = e.heap.obj_mut(*id) {
            if let ObjKind::Proxy { handler, .. } = &mut o.kind {
                *handler = None;
            }
        }
    }
    Ok(Value::Undefined)
}

// -- performance ------------------------------------------------------

/// `performance.now()` — ms since engine creation (monotonic).
pub(crate) fn performance_now(e: &mut Engine, _t: Value, _a: &[Value]) -> JsResult<Value> {
    Ok(Value::Num(e.t0.elapsed().as_secs_f64() * 1000.0))
}

/// `performance.mark/measure/clear*` — pas de timeline : no-op.
pub(crate) fn performance_noop(_e: &mut Engine, _t: Value, _a: &[Value]) -> JsResult<Value> {
    Ok(Value::Undefined)
}

/// `performance.getEntries*` — rien n'est collecté → `[]`.
pub(crate) fn performance_empty_entries(
    e: &mut Engine,
    _t: Value,
    _a: &[Value],
) -> JsResult<Value> {
    Ok(e.new_array(Vec::new()))
}

// -- encodeURI / decodeURI -------------------------------------------

/// RFC 3986 reserved + URI-safe set kept verbatim by `encodeURI`
/// (component version escapes `; / ? : @ & = + $ , #` too).
const URI_SAFE_FULL: &str =
    "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_.!~*'();/?:@&=+$,#";

pub(crate) fn uri_escape(s: &str, safe: &str) -> String {
    let mut out = String::new();
    for &b in s.as_bytes() {
        let c = b as char;
        if c.is_ascii() && safe.contains(c) {
            out.push(c);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

pub(crate) fn uri_unescape(e: &mut Engine, s: &str) -> JsResult<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            if i + 2 >= b.len() {
                return Err(e.uri_error("malformed URI escape"));
            }
            let hex = std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or("");
            let Ok(v) = u8::from_str_radix(hex, 16) else {
                return Err(e.uri_error("malformed URI escape"));
            };
            out.push(v);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).map_err(|_| e.uri_error("invalid utf-8 in URI"))
}

pub(crate) fn encode_uri(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let s = e.to_js_string(arg(args, 0))?;
    Ok(e.str_val(uri_escape(&s, URI_SAFE_FULL)))
}

pub(crate) fn decode_uri(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let s = e.to_js_string(arg(args, 0))?;
    let out = uri_unescape(e, &s)?;
    Ok(e.str_val(out))
}

// -- Array extras ------------------------------------------------------

/// `Array.isArray(v)`.
pub(crate) fn array_is_array(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    Ok(Value::Bool(matches!(arg(args, 0), Value::Obj(id)
        if matches!(e.heap.obj(id).map(|o| &o.kind), Some(ObjKind::Array(_))))))
}

/// `Array.from(iterable|arrayLike)` — iterator protocol, then a
/// `length`-indexed fallback; strings iterate by char.
pub(crate) fn array_from(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let src = arg(args, 0);
    let mapfn = arg(args, 1);
    let has_map = !matches!(mapfn, Value::Undefined);
    if has_map && !e.is_callable(mapfn) {
        return Err(e.type_error("Array.from mapfn is not callable"));
    }
    let t = arg(args, 2);
    let mut items = if let Ok(v) = e.iter_values(src) {
        v
    } else {
        // Spec : pas de @@iterator → array-like `length`-driven.
        // `iter_values` jette aussi sur itérateur défaillant —
        // distinction impossible ici (deviation : le repli
        // array-like est tenté dans les deux cas).
        {
            let n = match e.get_prop_value(src, "length") {
                Ok(l) => {
                    let n = e.to_number(l)?;
                    if n.is_finite() && n > 0.0 {
                        n.min(4_294_967_295.0) as usize
                    } else {
                        0
                    }
                }
                Err(_) => 0,
            };
            let mut out = Vec::with_capacity(n.min(1_000_000));
            for i in 0..n {
                e.tick()?;
                out.push(e.get_prop_value(src, &i.to_string())?);
            }
            out
        }
    };
    if has_map {
        for (i, v) in items.iter_mut().enumerate() {
            *v = e.call_value(mapfn, t, &[*v, Value::Num(i as f64)])?;
        }
    }
    Ok(e.new_array(items))
}

pub(crate) fn array_includes(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let id = this_array(e, this)?;
    let needle = arg(args, 0);
    let elems = take_elems(e, id);
    // SameValueZero (NaN matches itself here).
    let found = elems.iter().any(|&v| {
        e.strict_eq(v, needle)
            || matches!((v, needle), (Value::Num(a), Value::Num(b)) if a.is_nan() && b.is_nan())
    });
    put_elems(e, id, elems);
    Ok(Value::Bool(found))
}

/// Shared `find`/`findIndex`/`some`/`every` loop — `cb(v, i, arr)`.
pub(crate) fn array_iter_find(
    e: &mut Engine,
    this: Value,
    args: &[Value],
) -> JsResult<(Vec<Value>, Vec<Option<bool>>, ObjId)> {
    let id = this_array(e, this)?;
    let cb = arg(args, 0);
    if !e.is_callable(cb) {
        return Err(e.type_error("callback is not a function"));
    }
    let elems = take_elems(e, id);
    let mut hits = Vec::with_capacity(elems.len());
    for (i, &v) in elems.iter().enumerate() {
        if v.is_hole() {
            hits.push(None); // trou → cb non appelée (spec `HasProperty`).
            continue;
        }
        let r = e.call_value(cb, arg(args, 1), &[v, Value::Num(i as f64), this])?;
        hits.push(Some(e.to_boolean(r)));
    }
    Ok((elems, hits, id))
}

pub(crate) fn array_find(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let (elems, hits, id) = array_iter_find(e, this, args)?;
    let out = elems
        .iter()
        .zip(&hits)
        .find_map(|(&v, &h)| (h == Some(true)).then_some(v))
        .unwrap_or(Value::Undefined);
    put_elems(e, id, elems);
    Ok(out)
}

pub(crate) fn array_find_index(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let (elems, hits, id) = array_iter_find(e, this, args)?;
    let out = hits
        .iter()
        .position(|&h| h == Some(true))
        .map_or(-1.0, |i| i as f64);
    put_elems(e, id, elems);
    Ok(Value::Num(out))
}

pub(crate) fn array_some(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let (elems, hits, id) = array_iter_find(e, this, args)?;
    let out = hits.contains(&Some(true));
    put_elems(e, id, elems);
    Ok(Value::Bool(out))
}

pub(crate) fn array_every(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let (elems, hits, id) = array_iter_find(e, this, args)?;
    // `every` : les trous sont ignorés — `all` porte sur les
    // éléments présents seulement (spec).
    let out = hits.iter().flatten().all(|&h| h);
    put_elems(e, id, elems);
    Ok(Value::Bool(out))
}

// -- String extras -----------------------------------------------------

pub(crate) fn string_includes(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let s = this_str(e, this)?;
    let needle = e.to_js_string(arg(args, 0))?;
    // `from` en code units (spec) → préfixe units puis recherche.
    let from = match arg(args, 1) {
        Value::Undefined => 0usize,
        v => (e.to_number(v)?.max(0.0)) as usize,
    };
    let u = u16_view(&s);
    let hay = u16_to_string(u.get(from.min(u.len())..).unwrap_or(&[]));
    Ok(Value::Bool(hay.contains(&needle)))
}

pub(crate) fn string_starts_with(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let s = this_str(e, this)?;
    let needle = e.to_js_string(arg(args, 0))?;
    let start = match arg(args, 1) {
        Value::Undefined => 0usize,
        v => (e.to_number(v)? as usize).min(u16_len(&s)),
    };
    // Recherche directe sur la vue units — plus de tranche `&str`
    // (un index dans un astral ne panique plus).
    let (u, n) = (u16_view(&s), u16_view(&needle));
    Ok(Value::Bool(
        start + n.len() <= u.len() && u[start..start + n.len()] == n[..],
    ))
}

pub(crate) fn string_ends_with(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let s = this_str(e, this)?;
    let needle = e.to_js_string(arg(args, 0))?;
    let end = match arg(args, 1) {
        Value::Undefined => u16_len(&s),
        v => (e.to_number(v)? as usize).min(u16_len(&s)),
    };
    let (u, n) = (u16_view(&s), u16_view(&needle));
    Ok(Value::Bool(
        n.len() <= end && u[end - n.len()..end] == n[..],
    ))
}

pub(crate) fn string_repeat(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let s = this_str(e, this)?;
    let n = e.to_number(arg(args, 0))?;
    if n < 0.0 || !n.is_finite() {
        return Err(e.range_error("repeat count out of range"));
    }
    Ok(e.str_val(s.repeat(n as usize)))
}

pub(crate) fn pad(e: &mut Engine, s: &str, args: &[Value], right: bool) -> JsResult<String> {
    let target = e.to_number(arg(args, 0))?.max(0.0) as usize;
    let fill = match arg(args, 1) {
        Value::Undefined => " ".to_string(),
        v => e.to_js_string(v)?,
    };
    // `target` et le compte courant sont en code units (spec).
    let cur = u16_len(s);
    if cur >= target || fill.is_empty() {
        return Ok(s.to_string());
    }
    let need = target - cur;
    let fill_u = u16_view(&fill);
    let mut pad_units: Vec<u16> = Vec::with_capacity(need);
    while pad_units.len() < need {
        let take = (need - pad_units.len()).min(fill_u.len());
        pad_units.extend_from_slice(&fill_u[..take]);
    }
    let p = u16_to_string(&pad_units);
    Ok(if right {
        format!("{s}{p}")
    } else {
        format!("{p}{s}")
    })
}

pub(crate) fn string_pad_start(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let s = this_str(e, this)?;
    let out = pad(e, &s, args, false)?;
    Ok(e.str_val(out))
}

pub(crate) fn string_pad_end(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let s = this_str(e, this)?;
    let out = pad(e, &s, args, true)?;
    Ok(e.str_val(out))
}

pub(crate) fn string_trim_start(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let s = this_str(e, this)?;
    Ok(e.str_val(s.trim_start().to_string()))
}

pub(crate) fn string_trim_end(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let s = this_str(e, this)?;
    Ok(e.str_val(s.trim_end().to_string()))
}

// -- Number statics ------------------------------------------------------

pub(crate) fn number_is_nan(_e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    Ok(Value::Bool(
        matches!(arg(args, 0), Value::Num(n) if n.is_nan()),
    ))
}

pub(crate) fn number_is_finite(_e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    Ok(Value::Bool(
        matches!(arg(args, 0), Value::Num(n) if n.is_finite()),
    ))
}

pub(crate) fn number_is_integer(_e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    Ok(Value::Bool(
        matches!(arg(args, 0), Value::Num(n) if n.is_finite() && n.fract() == 0.0),
    ))
}

// -- Object extras -------------------------------------------------------

/// `Object.getOwnPropertyNames` — all own props (enum + non-enum).
/// Préfixe sentinelle des clés-symboles (`\u{f130}Symbol.*`).
pub(crate) fn object_get_own_property_names(
    e: &mut Engine,
    _t: Value,
    args: &[Value],
) -> JsResult<Value> {
    let keys = match arg(args, 0) {
        Value::Obj(id) => e.own_keys_trapped(id)?,
        // String-box : indices + `length` (spec `ToObject`).
        Value::Str(sid) => {
            let n = e.heap.string(sid).map_or(0, |s| u16_len(&s));
            (0..n)
                .map(|i| i.to_string())
                .chain(["length".to_string()])
                .collect()
        }
        _ => Vec::new(),
    };
    // Spec : `getOwnPropertyNames` ne rend que les clés string —
    // les clés-symboles sentinelles `\u{f130}…` et les internes
    // `\u{f131}…` (noms privés, métadonnées de classe) sont exclues.
    let vals = keys
        .iter()
        .filter(|k| !is_sentinel_key(k))
        .map(|k| e.str_val(k.clone()))
        .collect();
    Ok(e.new_array(vals))
}

/// `true` pour les clés sentinelles internes `\u{f120}…\u{f13f}`
/// (symboles, noms privés, métadonnées de classe) — invisibles
/// aux API d'énumération de noms.
pub(crate) fn is_sentinel_key(k: &str) -> bool {
    k.chars()
        .next()
        .is_some_and(|c| ('\u{f120}'..='\u{f13f}').contains(&c))
}

/// `Object.getOwnPropertySymbols` — les `ObjKind::Symbol`
/// correspondant aux clés sentinelles `\u{f130}…` (reverse-lookup
/// `sym_index` ; une entrée périmée par GC est ignorée — le slot
/// ne peut pas être réutilisé par un symbole à même clé).
pub(crate) fn object_get_own_property_symbols(
    e: &mut Engine,
    _t: Value,
    args: &[Value],
) -> JsResult<Value> {
    let keys = match arg(args, 0) {
        Value::Obj(id) => e.own_keys_trapped(id)?,
        _ => Vec::new(),
    };
    let mut vals = Vec::new();
    for k in &keys {
        // Toute clé sentinelle `\u{f130}…` — `Symbol.<name>` des
        // well-knowns ET `sym{N}`/`sym.for.<k>` des symboles
        // utilisateurs.
        if !k.starts_with('\u{f130}') {
            continue;
        }
        let Some(sid) = e.sym_index.get(k).copied() else {
            continue;
        };
        let live = matches!(
            e.heap.obj(sid).map(|o| &o.kind),
            Some(ObjKind::Symbol { key, .. }) if key == k
        );
        if live {
            vals.push(Value::Obj(sid));
        }
    }
    Ok(e.new_array(vals))
}

/// `Object.preventExtensions` — `[[Extensible]] = false`. Sur un
/// proxy : trap `preventExtensions` ou délégation à la cible.
pub(crate) fn object_prevent_extensions(
    e: &mut Engine,
    _t: Value,
    args: &[Value],
) -> JsResult<Value> {
    if let Value::Obj(id) = arg(args, 0) {
        if let Some((tid, target, handler)) = e.proxy_th(id)? {
            let trap = e.proxy_trap(handler, "preventExtensions")?;
            if matches!(trap, Value::Obj(_)) {
                let ok = e.call_value(trap, handler, &[target])?;
                if !e.to_boolean(ok) {
                    return Err(e.type_error("proxy preventExtensions trap returned false"));
                }
                if e.target_extensible(tid) {
                    return Err(e.type_error(
                        "proxy preventExtensions trap returned true on an extensible target",
                    ));
                }
                return Ok(arg(args, 0));
            }
            return object_prevent_extensions(e, Value::Undefined, &[Value::Obj(tid)]);
        }
        if let Some(o) = e.heap.obj_mut(id) {
            o.extensible = false;
        }
    }
    Ok(arg(args, 0))
}

pub(crate) fn object_is_extensible(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    if let Value::Obj(id) = arg(args, 0) {
        if let Some((tid, target, handler)) = e.proxy_th(id)? {
            let trap = e.proxy_trap(handler, "isExtensible")?;
            if matches!(trap, Value::Obj(_)) {
                let r = e.call_value(trap, handler, &[target])?;
                let rb = e.to_boolean(r);
                if rb != e.target_extensible(tid) {
                    return Err(e.type_error(
                        "proxy isExtensible trap must report the target's extensibility",
                    ));
                }
                return Ok(Value::Bool(rb));
            }
            return object_is_extensible(e, Value::Undefined, &[Value::Obj(tid)]);
        }
        return Ok(Value::Bool(e.heap.obj(id).is_some_and(|o| o.extensible)));
    }
    Ok(Value::Bool(false))
}

/// `Object.seal` — non-extensible + toutes props non-configurables.
/// Sur un proxy : la trap `preventExtensions` si présente, sinon la
/// cible (approximation — la spec définit chaque prop via la trap
/// `defineProperty`, surface jugée trop lourde ici).
pub(crate) fn object_seal(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    if let Value::Obj(id) = arg(args, 0) {
        if matches!(e.heap.obj(id).map(|o| &o.kind), Some(ObjKind::Proxy { .. })) {
            return object_prevent_extensions(e, Value::Undefined, &[arg(args, 0)]);
        }
        if let Some(o) = e.heap.obj_mut(id) {
            // `seal` sur vue typée non vide → TypeError (spec —
            // les éléments restent configurable:true).
            if let ObjKind::TypedArray { len, .. } = &o.kind {
                if *len > 0 {
                    let err = e.type_error("cannot seal a typed array");
                    return Err(err);
                }
            }
            o.extensible = false;
            o.elem_integrity |= 1;
            for (_, p) in &mut o.props {
                match p {
                    crate::heap::Prop::Data { attrs, .. }
                    | crate::heap::Prop::Acc { attrs, .. } => {
                        attrs.configurable = false;
                    }
                }
            }
        }
    }
    Ok(arg(args, 0))
}

pub(crate) fn object_is_sealed(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    Ok(Value::Bool(match arg(args, 0) {
        Value::Obj(id) => e.heap.obj(id).is_some_and(|o| {
            !o.extensible
                && o.props.iter().all(|(_, p)| !p.configurable())
                && match &o.kind {
                    ObjKind::Array(elems) => elems.is_empty() || o.elem_integrity & 3 != 0,
                    _ => true,
                }
        }),
        _ => true,
    }))
}
