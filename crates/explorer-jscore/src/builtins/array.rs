// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Builtins JS — section extraite de `builtins.rs` (JS-C40, pur
//! reformatage mécanique : sémantique et tests inchangés).

#[allow(clippy::wildcard_imports)]
use super::*;

// ------------------------------------------------------------------
// Array
// ------------------------------------------------------------------

pub(crate) fn array_push(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let id = this_array(e, this)?;
    // Array-like non-tableau (générique spec, JS-C90 — jQuery
    // `$.prototype.push`) : `take_elems` matérialise via
    // `length`+`Get`, `put_elems` réécrit indices+`length`.
    if !e
        .heap
        .obj(id)
        .is_some_and(|o| matches!(&o.kind, ObjKind::Array(_)))
    {
        if !args.is_empty() && e.heap.obj(id).is_some_and(|o| o.elem_integrity & 3 != 0) {
            return Err(e.type_error("cannot push onto a sealed array"));
        }
        let mut elems = take_elems(e, id);
        elems.extend_from_slice(args);
        let n = elems.len();
        put_elems(e, id, elems);
        return Ok(Value::Num(n as f64));
    }
    // Ajout d'éléments : impossible sous `seal`/`freeze` — la spec
    // passe par `Set(throw)` → `TypeError` même en sloppy.
    if !args.is_empty() && e.heap.obj(id).is_some_and(|o| o.elem_integrity & 3 != 0) {
        return Err(e.type_error("cannot push onto a sealed array"));
    }
    let n = if let Some(o) = e.heap.obj_mut(id) {
        if let ObjKind::Array(v) = &mut o.kind {
            v.extend_from_slice(args);
            v.len()
        } else {
            0
        }
    } else {
        0
    };
    Ok(Value::Num(n as f64))
}

pub(crate) fn array_pop(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let id = this_array(e, this)?;
    let mut elems = take_elems(e, id);
    let orig = elems.clone();
    let v = elems.pop().unwrap_or(Value::Undefined).unhole();
    put_elems_guarded(e, id, elems, &orig)?;
    Ok(v)
}

pub(crate) fn array_shift(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let id = this_array(e, this)?;
    let mut elems = take_elems(e, id);
    let orig = elems.clone();
    let v = if elems.is_empty() {
        Value::Undefined
    } else {
        elems.remove(0)
    }
    .unhole();
    put_elems_guarded(e, id, elems, &orig)?;
    Ok(v)
}

pub(crate) fn array_unshift(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let id = this_array(e, this)?;
    let mut elems = take_elems(e, id);
    let orig = elems.clone();
    let mut new: Vec<Value> = args.to_vec();
    new.append(&mut elems);
    let n = new.len();
    put_elems_guarded(e, id, new, &orig)?;
    Ok(Value::Num(n as f64))
}

pub(crate) fn array_join(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let id = this_array(e, this)?;
    let sep = match arg(args, 0) {
        Value::Undefined => ",".to_string(),
        v => e.to_js_string(v)?,
    };
    let elems = take_elems(e, id);
    let mut parts = Vec::with_capacity(elems.len());
    // `parts` et la sortie `join` sont des String natifs hors
    // compteur du heap — la taille cumulée est gardée en cours
    // de route (JS-C55 : `Array(1e6).fill('x'.repeat(100)).join()`
    // materialiserait ~200 Mio d'un coup).
    let mut total = sep.len().saturating_mul(elems.len().saturating_sub(1));
    for (i, v) in elems.iter().enumerate() {
        parts.push(match v {
            Value::Undefined | Value::Null | Value::Hole => String::new(),
            _ => e.to_js_string(*v)?,
        });
        total = total.saturating_add(parts.last().map_or(0, String::len));
        if i % 4096 == 0 {
            e.alloc_guard(total)?;
        }
    }
    e.alloc_guard(total)?;
    put_elems(e, id, elems);
    Ok(e.str_val(parts.join(&sep)))
}

pub(crate) fn array_index_of(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let id = this_array(e, this)?;
    let needle = arg(args, 0);
    let elems = take_elems(e, id);
    let mut start = match arg(args, 1) {
        Value::Undefined => 0i64,
        v => e.to_number(v)? as i64,
    };
    if start < 0 {
        start = (elems.len() as i64 + start).max(0);
    }
    let mut found = -1i64;
    for (i, v) in elems.iter().enumerate().skip(start as usize) {
        if !v.is_hole() && e.strict_eq(*v, needle) {
            found = i as i64;
            break;
        }
    }
    put_elems(e, id, elems);
    Ok(Value::Num(found as f64))
}

pub(crate) fn array_last_index_of(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let id = this_array(e, this)?;
    let needle = arg(args, 0);
    let elems = take_elems(e, id);
    let mut found = -1i64;
    for (i, v) in elems.iter().enumerate().rev() {
        if !v.is_hole() && e.strict_eq(*v, needle) {
            found = i as i64;
            break;
        }
    }
    put_elems(e, id, elems);
    Ok(Value::Num(found as f64))
}

pub(crate) fn norm_idx(i: i64, len: i64) -> usize {
    if i < 0 {
        (len + i).max(0) as usize
    } else {
        (i.min(len)) as usize
    }
}

pub(crate) fn array_slice(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    // Méthode générique (spec) — array-likes via `length`+`Get`
    // (`Object.create(array)` mesuré github selector-observer).
    let elems = this_array_like(e, this)?;
    let len = elems.len() as i64;
    let start = match arg(args, 0) {
        Value::Undefined => 0,
        v => e.to_number(v)? as i64,
    };
    let end = match arg(args, 1) {
        Value::Undefined => len,
        v => e.to_number(v)? as i64,
    };
    let (a, b) = (norm_idx(start, len), norm_idx(end, len));
    let out: Vec<Value> = if a < b {
        elems[a..b].to_vec()
    } else {
        Vec::new()
    };
    // `ArraySpeciesCreate(this, out.len)` — sous-classes.
    let a_v = species_array(e, this, out.len())?;
    for (i, v) in out.iter().enumerate() {
        e.set_prop_value(a_v, &i.to_string(), *v)?;
    }
    Ok(a_v)
}

pub(crate) fn array_concat(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    // `this` est un élément comme les autres (spec) : spreadable
    // → éclaté via `length`+`Get`, sinon posé tel quel. Couvre
    // les array-likes (`Object.create(array)` github
    // selector-observer : `idx.slice(0).concat(idx.default)`).
    let mut out = Vec::new();
    for a in std::iter::once(&this).chain(args.iter()) {
        // `IsConcatSpreadable` (spec) : `arg[@@isConcatSpreadable]`
        // prime, sinon le défaut est `Array.isArray(arg)`.
        let spread = match a {
            Value::Obj(aid) => match e.get_prop(*aid, crate::interp::SYM_IS_CONCAT_SPREADABLE) {
                Some(v) if !matches!(v, Value::Undefined) => e.to_boolean(v),
                _ => matches!(e.heap.obj(*aid).map(|o| &o.kind), Some(ObjKind::Array(_))),
            },
            _ => false,
        };
        if spread {
            // Lecture via `length` + `Get` par indice (sous-classes,
            // array-likes et proxies traversés, spec). `length`
            // hostile d'un array-like (`{length:1e9}`) : plafond
            // cohérent avec `array_from` + tick périodique — la
            // boucle de `Get` tourne sinon sans borne (JS-C55).
            let len = match e.get_prop_value(*a, "length")? {
                Value::Num(n) => (n.max(0.0) as usize).min(1_000_000),
                _ => 0,
            };
            for i in 0..len {
                let k = i.to_string();
                // `FlattenIntoArray` (spec) : `HasProperty` d'abord
                // — les trous traversent comme des trous, jamais
                // densifiés en `undefined`.
                let v = if let Value::Obj(aid) = a {
                    if e.has_prop(*aid, &k)? {
                        e.get_prop_value(*a, &k)?
                    } else {
                        Value::Hole
                    }
                } else {
                    e.get_prop_value(*a, &k)?
                };
                out.push(v);
                if i % 8192 == 0 {
                    e.tick()?;
                }
            }
        } else {
            out.push(*a);
        }
    }
    let a_v = species_array(e, this, out.len())?;
    for (i, v) in out.iter().enumerate() {
        e.set_prop_value(a_v, &i.to_string(), *v)?;
    }
    Ok(a_v)
}

pub(crate) fn array_reverse(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let id = this_array(e, this)?;
    // `reverse` n'écrit que si len > 1 (spec — sinon aucun `Set`).
    if e.heap.obj(id).is_some_and(|o| {
        o.elem_integrity & 2 != 0 && matches!(&o.kind, ObjKind::Array(v) if v.len() > 1)
    }) {
        return Err(e.type_error("cannot reverse a frozen array"));
    }
    // Array-like (JS-C90) : via `take`/`put_elems` génériques.
    if !e
        .heap
        .obj(id)
        .is_some_and(|o| matches!(&o.kind, ObjKind::Array(_)))
    {
        let mut elems = take_elems(e, id);
        elems.reverse();
        put_elems(e, id, elems);
        return Ok(this);
    }
    if let Some(o) = e.heap.obj_mut(id) {
        if let ObjKind::Array(v) = &mut o.kind {
            v.reverse();
        }
    }
    Ok(this)
}

pub(crate) fn array_sort(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let id = this_array(e, this)?;
    let cmp = arg(args, 0);
    let use_cmp = e.is_callable(cmp);
    let mut elems = take_elems(e, id);
    // `sort` écrit chaque élément via `Set(throw)` → tableau gelé
    // non vide → `TypeError` même si l'ordre ne change pas (spec).
    if e.heap.obj(id).is_some_and(|o| o.elem_integrity & 2 != 0) && !elems.is_empty() {
        put_elems(e, id, elems);
        return Err(e.type_error("cannot sort a frozen array"));
    }
    if use_cmp {
        // Merge sort itératif, stable — comparateur script faillible.
        // L'insertion sort O(n²) gelait le moteur dès ~10^5 éléments
        // hostiles (JS-C55).
        let n = elems.len();
        let mut buf = elems.clone();
        let mut width = 1;
        while width < n {
            let mut i = 0;
            while i < n {
                let m = (i + width).min(n);
                let r = (i + 2 * width).min(n);
                let (mut a, mut b, mut k) = (i, m, i);
                while a < m && b < r {
                    let cr = e.call_value(cmp, Value::Undefined, &[elems[a], elems[b]])?;
                    let take_right = e.to_number(cr)? > 0.0;
                    if take_right {
                        buf[k] = elems[b];
                        b += 1;
                    } else {
                        buf[k] = elems[a];
                        a += 1;
                    }
                    k += 1;
                }
                while a < m {
                    buf[k] = elems[a];
                    a += 1;
                    k += 1;
                }
                while b < r {
                    buf[k] = elems[b];
                    b += 1;
                    k += 1;
                }
                i += 2 * width;
            }
            std::mem::swap(&mut elems, &mut buf);
            width *= 2;
        }
    } else {
        // Clés string matérialisées une fois par élément (spec
        // ToString-une-fois) puis tri Rust O(n log n) — l'insertion
        // sort O(n²) appelait `to_js_string` par comparaison (JS-C55).
        let mut keyed: Vec<(String, Value)> = Vec::with_capacity(elems.len());
        for &v in &elems {
            keyed.push((e.to_js_string(v)?, v));
        }
        keyed.sort_by(|a, b| a.0.cmp(&b.0));
        elems = keyed.into_iter().map(|(_, v)| v).collect();
    }
    put_elems(e, id, elems);
    Ok(this)
}

pub(crate) fn array_for_each(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let id = this_array(e, this)?;
    let cb = arg(args, 0);
    if !e.is_callable(cb) {
        return Err(e.type_error("forEach needs a function"));
    }
    let this_arg = arg(args, 1);
    let elems = take_elems(e, id);
    for (i, v) in elems.iter().enumerate() {
        if v.is_hole() {
            continue; // `forEach` saute les trous (spec).
        }
        e.call_value(cb, this_arg, &[*v, Value::Num(i as f64), this])?;
        if i % 8192 == 0 {
            e.tick()?; // cb native : ops-limit relu ici (JS-C55)
        }
    }
    put_elems(e, id, elems);
    Ok(Value::Undefined)
}

pub(crate) fn array_map(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let id = this_array(e, this)?;
    let cb = arg(args, 0);
    if !e.is_callable(cb) {
        return Err(e.type_error("map needs a function"));
    }
    let this_arg = arg(args, 1);
    let elems = take_elems(e, id);
    let mut out = Vec::with_capacity(elems.len());
    for (i, v) in elems.iter().enumerate() {
        // `map` préserve les trous (spec — `Array(3).map(f)` reste
        // épars, la cb n'est pas appelée).
        out.push(if v.is_hole() {
            Value::Hole
        } else {
            e.call_value(cb, this_arg, &[*v, Value::Num(i as f64), this])?
        });
        if i % 8192 == 0 {
            e.tick()?;
        }
    }
    put_elems(e, id, elems);
    let a_v = species_array(e, this, out.len())?;
    for (i, v) in out.iter().enumerate() {
        e.set_prop_value(a_v, &i.to_string(), *v)?;
    }
    Ok(a_v)
}

pub(crate) fn array_filter(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let id = this_array(e, this)?;
    let cb = arg(args, 0);
    if !e.is_callable(cb) {
        return Err(e.type_error("filter needs a function"));
    }
    let this_arg = arg(args, 1);
    let elems = take_elems(e, id);
    let mut out = Vec::new();
    for (i, v) in elems.iter().enumerate() {
        if v.is_hole() {
            continue;
        }
        let keep = e.call_value(cb, this_arg, &[*v, Value::Num(i as f64), this])?;
        if e.to_boolean(keep) {
            out.push(*v);
        }
        if i % 8192 == 0 {
            e.tick()?;
        }
    }
    put_elems(e, id, elems);
    let a_v = species_array(e, this, out.len())?;
    for (i, v) in out.iter().enumerate() {
        e.set_prop_value(a_v, &i.to_string(), *v)?;
    }
    Ok(a_v)
}

pub(crate) fn array_reduce(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let id = this_array(e, this)?;
    let cb = arg(args, 0);
    if !e.is_callable(cb) {
        return Err(e.type_error("reduce needs a function"));
    }
    let elems = take_elems(e, id);
    let (mut acc, start) = match args.get(1) {
        Some(v) => (*v, 0),
        // Sans init, l'acc est le premier élément *présent* —
        // tout-trous ou vide → TypeError (spec).
        None => {
            if let Some(i) = elems.iter().position(|v| !v.is_hole()) {
                (elems[i], i + 1)
            } else {
                put_elems(e, id, elems);
                return Err(e.type_error("reduce of empty array"));
            }
        }
    };
    for (i, v) in elems.iter().enumerate().skip(start) {
        if v.is_hole() {
            continue;
        }
        acc = e.call_value(cb, Value::Undefined, &[acc, *v, Value::Num(i as f64), this])?;
        if i % 8192 == 0 {
            e.tick()?;
        }
    }
    put_elems(e, id, elems);
    Ok(acc)
}
