// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Builtins JS — section extraite de `builtins.rs` (JS-C40, pur
//! reformatage mécanique : sémantique et tests inchangés).

#[allow(clippy::wildcard_imports)]
use super::*;

// ------------------------------------------------------------------
// JS-C17 — Float16/copyWithin/crypto/Iterator/Annex-B/SuppressedError
// ------------------------------------------------------------------

/// `Array.prototype.copyWithin(target, start?, end?)` — copie
/// interne relative (indices négatifs = depuis la fin, spec).
pub(crate) fn array_copy_within(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let id = this_array(e, this)?;
    let mut elems = take_elems(e, id);
    let len = elems.len() as i64;
    let rel = |v: Value, e: &mut Engine| -> JsResult<i64> { Ok(e.to_number(v)? as i64) };
    let target = match rel(arg(args, 0), e)? {
        i if i < 0 => (len + i).max(0),
        i => i.min(len),
    };
    let start = match arg(args, 1) {
        Value::Undefined => 0,
        v => match rel(v, e)? {
            i if i < 0 => (len + i).max(0),
            i => i.min(len),
        },
    };
    let end = match arg(args, 2) {
        Value::Undefined => len,
        v => match rel(v, e)? {
            i if i < 0 => (len + i).max(0),
            i => i.min(len),
        },
    };
    // Snapshot de la plage source — `copyWithin` a une sémantique
    // memmove : les chevauchements copient les valeurs d'origine,
    // pas les valeurs déjà réécrites (spec `Array.prototype.
    // copyWithin` — copie d'abord, puis pose).
    let count = (end - start).max(0).min(len - target);
    if count > 0 && e.heap.obj(id).is_some_and(|o| o.elem_integrity & 2 != 0) {
        put_elems(e, id, elems);
        return Err(e.type_error("cannot copyWithin a frozen array"));
    }
    if count > 0 {
        let chunk: Vec<Value> = elems[start as usize..(start + count) as usize].to_vec();
        for (k, v) in chunk.into_iter().enumerate() {
            elems[(target as usize) + k] = v;
        }
    }
    put_elems(e, id, elems);
    Ok(this)
}

/// `Math.f16round(v)` — arrondi IEEE-754 binary16 (ES2025).
pub(crate) fn math_f16round(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let v = e.to_number(arg(args, 0))?;
    Ok(Value::Num(crate::heap::f16_to_f64(
        crate::heap::f64_to_f16(v),
    )))
}

/// PRNG partagé `Math.random`/`crypto` — xorshift64 (même schéma
/// que `math_random` ; PAS cryptographique — déviation).
pub(crate) fn next_rand(e: &mut Engine) -> u64 {
    e.rng ^= e.rng >> 12;
    e.rng ^= e.rng << 25;
    e.rng ^= e.rng >> 27;
    e.rng.wrapping_mul(0x2545_f491_4f6c_dd1d)
}

/// `crypto.getRandomValues(ta)` — remplit la vue d'octets
/// pseudo-aléatoires (`TypeError` sur non-TypedArray, spec).
pub(crate) fn crypto_get_random_values(
    e: &mut Engine,
    _t: Value,
    args: &[Value],
) -> JsResult<Value> {
    let v = arg(args, 0);
    let Value::Obj(id) = v else {
        return Err(e.type_error("getRandomValues needs a TypedArray"));
    };
    let Some((ta, bid, offset, len)) = e.heap.obj(id).and_then(|o| match &o.kind {
        ObjKind::TypedArray {
            ta,
            buffer: Value::Obj(bid),
            offset,
            len,
        } => Some((*ta, *bid, *offset, *len)),
        _ => None,
    }) else {
        return Err(e.type_error("getRandomValues needs a TypedArray"));
    };
    // Quota spec : 65 536 octets max.
    if len.saturating_mul(ta.size()) > 65_536 {
        return Err(e.type_error("getRandomValues: quota exceeded"));
    }
    let n = len * ta.size();
    // Tirage hors emprunt du heap (le PRNG vit dans `e`) —
    // borné par le quota ci-dessus (≤ 64 Ko).
    let mut buf = vec![0u8; n];
    for chunk in buf.chunks_mut(8) {
        let r = next_rand(e);
        chunk.copy_from_slice(&r.to_le_bytes()[..chunk.len()]);
    }
    let Some(b) = e.heap.obj_mut(bid) else {
        return Err(e.type_error("detached typed array"));
    };
    let ObjKind::ByteBuf(bytes) = &mut b.kind else {
        return Err(e.type_error("detached typed array"));
    };
    let end = (offset + n).min(bytes.len());
    bytes[offset..end].copy_from_slice(&buf[..end - offset]);
    Ok(v)
}

/// `crypto.randomUUID()` — v4 via le PRNG moteur (déviation :
/// pas cryptographique).
pub(crate) fn crypto_random_uuid(e: &mut Engine, _t: Value, _a: &[Value]) -> JsResult<Value> {
    let mut b = [0u8; 16];
    for chunk in b.chunks_mut(8) {
        let r = next_rand(e);
        chunk.copy_from_slice(&r.to_le_bytes()[..chunk.len()]);
    }
    b[6] = (b[6] & 0x0f) | 0x40; // version 4
    b[8] = (b[8] & 0x3f) | 0x80; // variant RFC 4122
    let hex = |v: u8| format!("{v:02x}");
    let s: String = b.iter().map(|&v| hex(v)).collect();
    Ok(e.str_val(format!(
        "{}-{}-{}-{}-{}",
        &s[0..8],
        &s[8..12],
        &s[12..16],
        &s[16..20],
        &s[20..32]
    )))
}

/// `new Iterator()` — constructeur abstrait (spec `TypeError`).
pub(crate) fn iterator_ctor(e: &mut Engine, _t: Value, _a: &[Value]) -> JsResult<Value> {
    Err(e.type_error("The Iterator constructor is abstract"))
}

/// `Iterator.from(v)` — un itérateur passe tel quel (spec :
/// `GetIteratorFlattenable`), un itérable passe par `@@iterator`.
pub(crate) fn iterator_from(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let v = arg(args, 0);
    if let Value::Obj(id) = v {
        if e.get_prop(id, "next").is_some_and(|n| e.is_callable(n)) {
            return Ok(v);
        }
        if let Some(m) = e.get_prop(id, crate::interp::SYM_ITERATOR) {
            if e.is_callable(m) {
                return e.call_value(m, v, &[]);
            }
        }
        return Err(e.type_error("Iterator.from on non-iterable"));
    }
    if let Value::Str(_) = v {
        let items = e.iter_values(v)?;
        return Ok(crate::collections::iter_obj(e, items, e.protos.iter_helper));
    }
    Err(e.type_error("Iterator.from on non-iterable"))
}

/// `Iterator.concat(...iterables)` (ES2025) — concatène les items
/// de chaque itérable (eager : nos itérateurs sont des snapshots —
/// déviation vs la laziness spec).
pub(crate) fn iterator_concat(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let mut items = Vec::new();
    for a in args {
        items.extend(e.iter_values(*a)?);
    }
    Ok(crate::collections::iter_obj(e, items, e.protos.iter_helper))
}

/// `SuppressedError(error, suppressed, message?)` — ES2026.
pub(crate) fn suppressed_error_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let msg = match arg(args, 2) {
        Value::Undefined => String::new(),
        v => e.to_js_string(v)?,
    };
    let se_p = e
        .get_prop(e.global, "SuppressedError")
        .and_then(|c| {
            e.get_prop(
                match c {
                    Value::Obj(id) => id,
                    _ => e.protos.error,
                },
                "prototype",
            )
        })
        .unwrap_or(Value::Obj(e.protos.error));
    let err = e.error_value(
        match se_p {
            Value::Obj(id) => id,
            _ => e.protos.error,
        },
        "SuppressedError",
        &msg,
    );
    if let Value::Obj(id) = err {
        e.set_own(id, "error", arg(args, 0), true);
        e.set_own(id, "suppressed", arg(args, 1), true);
    }
    Ok(err)
}

/// `Object.prototype.propertyIsEnumerable(k)` — own + flag
/// énumérable (spec ; indices de string wrappers = énumérables).
pub(crate) fn object_property_is_enumerable(
    e: &mut Engine,
    this: Value,
    args: &[Value],
) -> JsResult<Value> {
    let key = e.prop_key(arg(args, 0))?;
    match this {
        Value::Obj(id) => {
            let Some(p) = e.get_own_prop(id, &key)? else {
                return Ok(Value::Bool(false));
            };
            Ok(Value::Bool(p.enumerable()))
        }
        Value::Str(sid) => {
            let s = e.heap.string(sid).unwrap_or_default();
            let ok = crate::interp::array_index(&key).is_some_and(|i| i < s.encode_utf16().count());
            Ok(Value::Bool(ok))
        }
        _ => Ok(Value::Bool(false)),
    }
}

/// `__defineGetter__(k, f)` — accesseur `{get: f}` énumérable +
/// configurable (Annex B).
pub(crate) fn object_define_getter(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    define_accessor_annex(e, this, arg(args, 0), arg(args, 1), true)
}

pub(crate) fn object_define_setter(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    define_accessor_annex(e, this, arg(args, 0), arg(args, 1), false)
}

pub(crate) fn define_accessor_annex(
    e: &mut Engine,
    this: Value,
    key_v: Value,
    f: Value,
    is_get: bool,
) -> JsResult<Value> {
    if !e.is_callable(f) {
        return Err(e.type_error("__defineGetter__/__defineSetter__ needs a function"));
    }
    let Value::Obj(id) = this else {
        return Err(e.type_error("can't define accessor on a primitive"));
    };
    let key = e.prop_key(key_v)?;
    let (get, set) = if is_get {
        (f, Value::Undefined)
    } else {
        (Value::Undefined, f)
    };
    if let Some(o) = e.heap.obj_mut(id) {
        o.define_accessor(
            &key,
            get,
            set,
            crate::heap::PropAttrs {
                writable: false,
                enumerable: true,
                configurable: true,
            },
        );
    }
    Ok(Value::Undefined)
}

/// `__lookupGetter__(k)` — premier `get` trouvé en remontant la
/// chaîne de prototypes (Annex B).
pub(crate) fn object_lookup_getter(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    lookup_accessor_annex(e, this, arg(args, 0), true)
}

pub(crate) fn object_lookup_setter(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    lookup_accessor_annex(e, this, arg(args, 0), false)
}

pub(crate) fn lookup_accessor_annex(
    e: &mut Engine,
    this: Value,
    key_v: Value,
    is_get: bool,
) -> JsResult<Value> {
    let key = e.prop_key(key_v)?;
    let Value::Obj(mut id) = this else {
        return Ok(Value::Undefined);
    };
    let mut hops = 0;
    loop {
        hops += 1;
        if hops > 64 {
            return Ok(Value::Undefined);
        }
        let Some(o) = e.heap.obj(id) else {
            return Ok(Value::Undefined);
        };
        if let Some(crate::heap::Prop::Acc { get, set, .. }) = o.find(&key) {
            return Ok(if is_get { get } else { set });
        }
        match o.proto {
            Value::Obj(next) => id = next,
            _ => return Ok(Value::Undefined),
        }
    }
}

/// `get __proto__` (Annex B) — `[[Prototype]]` du receiver ;
/// primitives → proto de leur wrapper (`(1).__proto__` =
/// `Number.prototype`).
pub(crate) fn proto_getter(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    match this {
        Value::Obj(id) => Ok(e.heap.obj(id).map_or(Value::Null, |o| o.proto)),
        Value::Str(_) => Ok(Value::Obj(e.protos.string)),
        Value::Num(_) => Ok(Value::Obj(e.protos.number)),
        Value::Bool(_) => Ok(Value::Obj(e.protos.boolean)),
        Value::Big(_) => Ok(Value::Obj(e.protos.bigint)),
        _ => Ok(Value::Null),
    }
}

/// `set __proto__` (Annex B) — `o.proto = v` si `v` est objet ou
/// `null` ; autres valeurs ignorées (spec). Délègue à
/// `object_set_proto` : non-extensible → `TypeError`, cycle rejeté,
/// trap proxy honorée.
pub(crate) fn proto_setter(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    if let Value::Obj(id) = this {
        if let v @ (Value::Obj(_) | Value::Null) = arg(args, 0) {
            object_set_proto(e, Value::Undefined, &[Value::Obj(id), v])?;
        }
    }
    Ok(Value::Undefined)
}
