// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Builtins JS — section extraite de `builtins.rs` (JS-C40, pur
//! reformatage mécanique : sémantique et tests inchangés).

#[allow(clippy::wildcard_imports)]
use super::*;

// ------------------------------------------------------------------
// JS-C8 — structuredClone / groupBy / fromAsync
// ------------------------------------------------------------------

/// `structuredClone(v)` — copie profonde spec `StructuredSerialize` :
/// cycles préservés (table id→clone), `Fun`/`Promise`/`Proxy`/`Host`/
/// `Symbol`/`Iter` non-clonables → `TypeError` (`DataCloneError`).
pub(crate) fn structured_clone(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let mut seen: Vec<(ObjId, ObjId)> = Vec::new();
    clone_deep(e, arg(args, 0), &mut seen)
}

/// Forme extraite du `ObjKind` (champs Copy — le borrow du heap
/// s'arrête avant toute allocation).
enum CloneShape {
    Arr,
    Map,
    Set,
    Buf(Vec<u8>),
    Dv(ObjId, usize, usize),
    Ta(crate::heap::TaKind, ObjId, usize, usize),
    Re(String, String),
    Ordinary,
    Err(&'static str),
}

pub(crate) fn clone_deep(
    e: &mut Engine,
    v: Value,
    seen: &mut Vec<(ObjId, ObjId)>,
) -> JsResult<Value> {
    let Value::Obj(id) = v else { return Ok(v) };
    if let Some((_, c)) = seen.iter().find(|(s, _)| *s == id) {
        return Ok(Value::Obj(*c));
    }
    let shape = match e.heap.obj(id).map(|o| &o.kind) {
        Some(ObjKind::Array(_)) => CloneShape::Arr,
        Some(ObjKind::Map(_)) => CloneShape::Map,
        Some(ObjKind::Set(_)) => CloneShape::Set,
        Some(ObjKind::ByteBuf(b)) => CloneShape::Buf(b.clone()),
        Some(ObjKind::DataView {
            buffer: Value::Obj(bid),
            offset,
            len,
        }) => CloneShape::Dv(*bid, *offset, *len),
        Some(ObjKind::TypedArray {
            ta,
            buffer: Value::Obj(bid),
            offset,
            len,
        }) => CloneShape::Ta(*ta, *bid, *offset, *len),
        Some(ObjKind::RegExp(re)) => CloneShape::Re(re.source.clone(), re.flag_string()),
        Some(ObjKind::Fun(_)) => CloneShape::Err("function"),
        Some(ObjKind::Promise(_)) => CloneShape::Err("Promise"),
        Some(ObjKind::Host(_)) => CloneShape::Err("host object"),
        Some(ObjKind::Symbol { .. }) => CloneShape::Err("Symbol"),
        Some(ObjKind::Proxy { .. }) => CloneShape::Err("Proxy"),
        Some(ObjKind::Iter { .. }) => CloneShape::Err("iterator"),
        Some(ObjKind::FetchResponse { .. }) => CloneShape::Err("Response"),
        _ => CloneShape::Ordinary,
    };
    match shape {
        CloneShape::Err(w) => Err(e.type_error(format!("structuredClone: cannot clone {w}"))),
        CloneShape::Buf(b) => Ok(Value::Obj(e.heap.alloc_obj(Object {
            proto: Value::Obj(e.protos.array_buffer),
            props: Vec::new(),
            kind: ObjKind::ByteBuf(b),
            extensible: true,
            elem_integrity: 0,
            args_map: None,
        }))),
        CloneShape::Dv(bid, offset, len) => {
            let nb = clone_deep(e, Value::Obj(bid), seen)?;
            Ok(Value::Obj(e.heap.alloc_obj(Object {
                proto: Value::Obj(e.protos.data_view),
                props: Vec::new(),
                kind: ObjKind::DataView {
                    buffer: nb,
                    offset,
                    len,
                },
                extensible: true,
                elem_integrity: 0,
                args_map: None,
            })))
        }
        CloneShape::Ta(ta, bid, offset, len) => {
            let nb = clone_deep(e, Value::Obj(bid), seen)?;
            let Value::Obj(nbid) = nb else {
                return Err(e.type_error("structuredClone: bad buffer"));
            };
            Ok(crate::typed_array::new_ta(e, ta, nbid, offset, len))
        }
        CloneShape::Re(src, flags) => e.make_regexp(&src, &flags),
        CloneShape::Arr => {
            let arr = e.new_array(Vec::new());
            let Value::Obj(cid) = arr else {
                return Err(e.type_error("structuredClone: bad array"));
            };
            seen.push((id, cid));
            let elems = match e.heap.obj(id).map(|o| &o.kind) {
                Some(ObjKind::Array(elems)) => elems.clone(),
                _ => Vec::new(),
            };
            let mut out = Vec::with_capacity(elems.len());
            for v in elems {
                out.push(clone_deep(e, v, seen)?);
            }
            if let Some(o) = e.heap.obj_mut(cid) {
                if let ObjKind::Array(el) = &mut o.kind {
                    *el = out;
                }
            }
            Ok(arr)
        }
        CloneShape::Map => {
            let cid = e.heap.alloc_obj(Object {
                proto: Value::Obj(e.protos.map),
                props: Vec::new(),
                kind: ObjKind::Map(Vec::new()),
                extensible: true,
                elem_integrity: 0,
                args_map: None,
            });
            seen.push((id, cid));
            let pairs = match e.heap.obj(id).map(|o| &o.kind) {
                Some(ObjKind::Map(m)) => m.clone(),
                _ => Vec::new(),
            };
            for (k, v) in pairs {
                let kc = clone_deep(e, k, seen)?;
                let vc = clone_deep(e, v, seen)?;
                crate::collections::map_set_entry(e, cid, kc, vc);
            }
            Ok(Value::Obj(cid))
        }
        CloneShape::Set => {
            let cid = e.heap.alloc_obj(Object {
                proto: Value::Obj(e.protos.set),
                props: Vec::new(),
                kind: ObjKind::Set(Vec::new()),
                extensible: true,
                elem_integrity: 0,
                args_map: None,
            });
            seen.push((id, cid));
            let items = match e.heap.obj(id).map(|o| &o.kind) {
                Some(ObjKind::Set(s)) => s.clone(),
                _ => Vec::new(),
            };
            for v in items {
                let vc = clone_deep(e, v, seen)?;
                crate::collections::set_add(e, cid, vc);
            }
            Ok(Value::Obj(cid))
        }
        CloneShape::Ordinary => {
            let (proto, props) = match e.heap.obj(id) {
                Some(o) => (o.proto, o.props.clone()),
                _ => (Value::Obj(e.protos.object), Vec::new()),
            };
            let cid = e.heap.alloc_obj(Object::ordinary(proto));
            seen.push((id, cid));
            for (key, prop) in props {
                // Spec : props énumérables propres, clonées en data.
                let crate::heap::Prop::Data { value, attrs } = prop else {
                    continue;
                };
                if !attrs.enumerable {
                    continue;
                }
                let cv = clone_deep(e, value, seen)?;
                e.set_own(cid, &key, cv, true);
            }
            Ok(Value::Obj(cid))
        }
    }
}

/// `Object.groupBy(items, cb)` — objet à proto nul `key → [items]`
/// (ES2024 ; les clés passent par `ToString` comme `groupBy` spec).
pub(crate) fn object_group_by(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let items = e.iter_values(arg(args, 0))?;
    let f = arg(args, 1);
    if !e.is_callable(f) {
        return Err(e.type_error("groupBy callback is not callable"));
    }
    let out = e.heap.alloc_obj(Object::ordinary(Value::Null));
    for (i, v) in items.into_iter().enumerate() {
        let k = e.call_value(f, Value::Undefined, &[v, Value::Num(i as f64)])?;
        let key = e.to_js_string(k)?;
        let arr = if let Some(v @ Value::Obj(_)) = e.get_prop(out, &key) {
            v
        } else {
            let a = e.new_array(Vec::new());
            e.set_own(out, &key, a, true);
            a
        };
        if let Value::Obj(aid) = arr {
            if let Some(o) = e.heap.obj_mut(aid) {
                if let ObjKind::Array(elems) = &mut o.kind {
                    elems.push(v);
                }
            }
        }
    }
    Ok(Value::Obj(out))
}

/// `Array.fromAsync(src, mapfn?, thisArg?)` — `await` de chaque
/// élément puis du résultat de mapfn, promesse de tableau
/// (même déviation settled-only qu'`await` : pending → `TypeError`).
pub(crate) fn array_from_async(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let src = arg(args, 0);
    let f = arg(args, 1);
    let has_map = !matches!(f, Value::Undefined);
    if has_map && !e.is_callable(f) {
        return Err(e.type_error("fromAsync mapfn is not callable"));
    }
    let t = arg(args, 2);
    // Comme toute fonction async, `fromAsync` ne jette jamais
    // synchrone : les erreurs (itération, mapfn, rejet d'élément)
    // rejettent la promesse retournée.
    let mut collect = || -> JsResult<Vec<Value>> {
        let vals = crate::typed_array::src_vals(e, src)?;
        let mut out = Vec::with_capacity(vals.len());
        for (i, v) in vals.iter().enumerate() {
            let mut x = await_settled(e, *v)?;
            if has_map {
                x = e.call_value(f, t, &[x, Value::Num(i as f64)])?;
                x = await_settled(e, x)?;
            }
            out.push(x);
        }
        Ok(out)
    };
    let result = collect();
    let pid = e.new_promise();
    match result {
        Ok(out) => {
            let arr = e.new_array(out);
            e.resolve_promise(pid, arr);
        }
        Err(Thrown(v)) => e.reject_promise(pid, v),
    }
    Ok(Value::Obj(pid))
}

/// `await` limité aux promesses settled — déviation documentée
/// partagée avec `Expr::Await` (pending → `TypeError`).
pub(crate) fn await_settled(e: &mut Engine, v: Value) -> JsResult<Value> {
    e.await_value(v)
}

// -- WeakRef / FinalizationRegistry (JS-C13) --------------------------

/// `new WeakRef(target)` — la cible doit être un objet (spec).
/// `ObjKind::WeakRef` n'est pas tracé → `deref()` observe la
/// collecte réelle.
pub(crate) fn weakref_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let Value::Obj(tid) = arg(args, 0) else {
        return Err(e.type_error("WeakRef target must be an object"));
    };
    let proto = match e.get_prop(e.global, "WeakRef") {
        Some(c) => e
            .get_prop_value(c, "prototype")
            .unwrap_or(Value::Obj(e.protos.object)),
        None => Value::Obj(e.protos.object),
    };
    Ok(Value::Obj(e.heap.alloc_obj(Object {
        proto,
        props: Vec::new(),
        kind: ObjKind::WeakRef(tid),
        extensible: true,
        elem_integrity: 0,
        args_map: None,
    })))
}

/// `wr.deref()` — `undefined` si la cible a été collectée.
pub(crate) fn weakref_deref(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let Value::Obj(id) = this else {
        return Err(e.type_error("WeakRef.prototype.deref on non-WeakRef"));
    };
    match e.heap.obj(id).map(|o| &o.kind) {
        Some(ObjKind::WeakRef(tid)) => Ok(if e.heap.obj(*tid).is_some() {
            Value::Obj(*tid)
        } else {
            Value::Undefined
        }),
        _ => Err(e.type_error("WeakRef.prototype.deref on non-WeakRef")),
    }
}

/// `new FinalizationRegistry(cleanup)` — `cleanup` callable
/// requis (spec `TypeError` sinon).
pub(crate) fn finreg_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let cb = arg(args, 0);
    if !e.is_callable(cb) {
        return Err(e.type_error("FinalizationRegistry callback must be callable"));
    }
    let proto = match e.get_prop(e.global, "FinalizationRegistry") {
        Some(c) => e
            .get_prop_value(c, "prototype")
            .unwrap_or(Value::Obj(e.protos.object)),
        None => Value::Obj(e.protos.object),
    };
    let id = e.heap.alloc_obj(Object {
        proto,
        props: Vec::new(),
        kind: ObjKind::FinReg {
            cb,
            entries: Vec::new(),
        },
        extensible: true,
        elem_integrity: 0,
        args_map: None,
    });
    e.finregs.push(id);
    Ok(Value::Obj(id))
}

/// `fr.register(target, held, unregisterToken?)` — `target` objet
/// requis, `token !== held` (spec).
pub(crate) fn finreg_register(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let Value::Obj(id) = this else {
        return Err(e.type_error("FinalizationRegistry.register on non-registry"));
    };
    let Value::Obj(tid) = arg(args, 0) else {
        return Err(e.type_error("FinalizationRegistry target must be an object"));
    };
    let held = arg(args, 1);
    let token = arg(args, 2);
    if e.strict_eq(token, held) {
        return Err(e.type_error("unregister token must not equal held value"));
    }
    match e.heap.obj_mut(id).map(|o| &mut o.kind) {
        Some(ObjKind::FinReg { entries, .. }) => {
            entries.push(crate::heap::FinRegEntry {
                target: tid,
                held,
                token,
            });
            Ok(Value::Undefined)
        }
        _ => Err(e.type_error("FinalizationRegistry.register on non-registry")),
    }
}

/// `fr.unregister(token)` — retire les entrées à ce jeton, rend
/// `true` si au moins une a été trouvée (spec).
pub(crate) fn finreg_unregister(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let Value::Obj(id) = this else {
        return Err(e.type_error("FinalizationRegistry.unregister on non-registry"));
    };
    let token = arg(args, 0);
    let Some(entries) = (match e.heap.obj_mut(id).map(|o| &mut o.kind) {
        Some(ObjKind::FinReg { entries, .. }) => Some(std::mem::take(entries)),
        _ => None,
    }) else {
        return Err(e.type_error("FinalizationRegistry.unregister on non-registry"));
    };
    let before = entries.len();
    let mut keep = Vec::with_capacity(before);
    for en in entries {
        if !e.strict_eq(en.token, token) {
            keep.push(en);
        }
    }
    let removed = before - keep.len();
    if let Some(o) = e.heap.obj_mut(id) {
        if let ObjKind::FinReg { entries: es, .. } = &mut o.kind {
            *es = keep;
        }
    }
    Ok(Value::Bool(removed > 0))
}

/// Job microtask d'un cleanup `FinalizationRegistry` — bound =
/// `[cb, held]`, exécute `cb(held)` (`this` = undefined).
pub(crate) fn finreg_cleanup(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let cb = arg(args, 0);
    let held = arg(args, 1);
    e.call_value(cb, Value::Undefined, &[held])
}

/// `String.prototype.normalize(form?)` — NFC par défaut, NFD/NFKC/
/// NFKD explicites (unicode-normalization, tables Unicode réelles).
pub(crate) fn string_normalize(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    use unicode_normalization::UnicodeNormalization;
    let s = e.to_js_string(this)?;
    let form = match arg(args, 0) {
        Value::Undefined => "NFC".to_string(),
        v => e.to_js_string(v)?,
    };
    let out: String = match form.as_str() {
        "NFC" => s.nfc().collect(),
        "NFD" => s.nfd().collect(),
        "NFKC" => s.nfkc().collect(),
        "NFKD" => s.nfkd().collect(),
        _ => return Err(e.range_error("invalid normalization form")),
    };
    Ok(e.str_val(out))
}
