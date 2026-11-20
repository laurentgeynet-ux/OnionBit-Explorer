// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Builtins JS — section extraite de `builtins.rs` (JS-C40, pur
//! reformatage mécanique : sémantique et tests inchangés).

#[allow(clippy::wildcard_imports)]
use super::*;

// ------------------------------------------------------------------
// JS-7g — coverage extras: Object.entries/fromEntries/hasOwn, Array
// at/flat/fill/findLast/iterators/ES2023 copy variants, String
// at/codePointAt/matchAll/fromCodePoint, WeakMap/WeakSet,
// TextEncoder/TextDecoder, full-ish UTC Date, Math extras,
// Number.toPrecision/toExponential.
// ------------------------------------------------------------------

/// Own-prop test shared by `Object.hasOwn`/`hasOwnProperty` — data
/// prop, accessor or array index.
pub(crate) fn has_own(e: &Engine, id: ObjId, key: &str) -> bool {
    match e.heap.obj(id) {
        Some(o) => match &o.kind {
            ObjKind::Array(elems) => {
                key == "length"
                    || crate::interp::array_index(key).is_some_and(|i| elems.has(i as u32))
                    || o.find(key).is_some()
            }
            // Index canonique d'une vue typée : own ssi index
            // **vivant** — la borne se mesure sur la région
            // réelle du buffer (détaché → vidé, RAB rétréci →
            // indices invalides, spec `IsValidIntegerIndex`,
            // JS-C606). Les clés non-canoniques relèvent des
            // props ordinaires.
            ObjKind::TypedArray {
                ta,
                buffer,
                offset,
                len,
                auto,
            } => match crate::interp::canonical_numeric_index(key) {
                Some(n) => {
                    crate::interp::ta_live_index(&e.heap, *ta, buffer, *offset, *len, *auto, n)
                        .is_some()
                }
                None => o.find(key).is_some(),
            },
            _ => o.find(key).is_some(),
        },
        None => false,
    }
}

/// `Object.entries(o)` → `[[k, v], …]` (own enumerable).
pub(crate) fn object_entries(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let Value::Obj(id) = arg(args, 0) else {
        return Ok(e.new_array(Vec::new()));
    };
    let keys = e.own_enum_keys_trapped(id)?;
    let mut out = Vec::with_capacity(keys.len());
    for k in keys {
        // Spec `Object.entries` : paires des props **string**
        // énumérables — les clés-symboles sont exclues.
        if crate::builtins::bigint::is_sentinel_key(&k) {
            continue;
        }
        // Lecture `[[Get]]` (getters et traps proxy déclenchés).
        let v = e.get_prop_value(Value::Obj(id), &k)?;
        let ks = e.str_val(k);
        let kv = e.new_array(vec![ks, v]);
        out.push(kv);
    }
    Ok(e.new_array(out))
}

/// `Object.fromEntries(iterable)` — `[k, v]` pairs → object.
pub(crate) fn object_from_entries(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let obj = e.new_object();
    let pairs = match arg(args, 0) {
        Value::Obj(id) => e.iter_values(Value::Obj(id))?,
        _ => Vec::new(),
    };
    for p in pairs {
        let pair = match p {
            Value::Obj(id) => e.iter_values(Value::Obj(id))?,
            _ => Vec::new(),
        };
        if pair.len() >= 2 {
            let k = e.prop_key(pair[0].clone())?;
            e.set_prop_value(Value::Obj(obj), &k, pair[1].clone())?;
        }
    }
    Ok(Value::Obj(obj))
}

/// `Object.hasOwn(o, k)` — own prop, no proto walk.
pub(crate) fn object_has_own(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let key = e.prop_key(arg(args, 1))?;
    Ok(Value::Bool(match arg(args, 0) {
        Value::Obj(id)
            if matches!(e.heap.obj(id).map(|o| &o.kind), Some(ObjKind::Proxy { .. })) =>
        {
            e.get_own_prop(id, &key)?.is_some()
        }
        Value::Obj(id) => has_own(e, id, &key),
        _ => false,
    }))
}

// -- Array: index helpers --------------------------------------------

/// `at`/index args → element index (negative wraps from the end).
pub(crate) fn rel_index(e: &mut Engine, v: Value, len: usize) -> JsResult<Option<usize>> {
    let i = e.to_number(v)? as isize;
    let j = if i < 0 { len as isize + i } else { i };
    Ok((j >= 0 && (j as usize) < len).then_some(j as usize))
}

pub(crate) fn array_at(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let id = this_array(e, this.clone())?;
    // `with_elems` — un `rel_index` qui jette (valueOf hostile)
    // ne laisse pas le backing vidé (JS-C319).
    with_elems(e, id, |e, elems| {
        let i = rel_index(e, arg(args, 0), elems.len())?;
        Ok(i.map_or(Value::Undefined, |i| elems[i].clone()).unhole())
    })
}

pub(crate) fn array_fill(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let id = this_array(e, this.clone())?;
    // `with_elems` — `rel_index` faillible + refus `frozen` :
    // le tableau reste intact quoi qu'il arrive (JS-C319).
    with_elems(e, id, |e, elems| {
        let len = elems.len();
        let v = arg(args, 0);
        let start = rel_index(e, arg(args, 1), len)?.unwrap_or(0);
        let end = match arg(args, 2) {
            Value::Undefined => len,
            v => rel_index(e, v, len)?.unwrap_or(len),
        };
        // `fill` n'écrit que sur une plage non vide — tableau gelé
        // avec plage vide → no-op admis (spec `Set(throw)` par slot).
        if end.min(len) > start && e.heap.obj(id).is_some_and(|o| o.elem_integrity & 2 != 0) {
            return Err(e.type_error("cannot fill a frozen array"));
        }
        // `start > end` (ex. `fill(v, 4, 2)`) : plage vide →
        // no-op spec ; le slice Rust `start..end` paniquerait
        // (JS-C594 — panic `extras.rs` interceptée par le
        // harness test262, corpus JS-C30).
        let end = end.min(len);
        if end > start {
            for slot in &mut elems[start..end] {
                *slot = v.clone();
            }
        }
        Ok(this)
    })
}

/// Recursive `flat` — spec depth param, default 1.
pub(crate) fn flat_into(
    e: &mut Engine,
    v: Value,
    depth: u32,
    out: &mut Vec<Value>,
) -> JsResult<()> {
    // Itératif (worklist) : la récursion Rust débordait la pile sur
    // `a.flat(Infinity)` avec ~300 k niveaux d'imbrication — JS-C55.
    // Chaque élément porte la profondeur restante ; les enfants d'un
    // niveau sont empilés en ordre inverse pour préserver l'ordre.
    let mut stack = vec![(v, depth)];
    while let Some((v, depth)) = stack.pop() {
        e.tick()?; // borne ops — un nœud géant ne doit pas boucler.
        if v.is_hole() {
            continue; // les trous disparaissent à l'aplatissement.
        }
        if depth > 0 {
            if let Value::Obj(id) = v {
                if matches!(e.heap.obj(id).map(|o| &o.kind), Some(ObjKind::Array(_))) {
                    // Les enfants sont *lus* (copiés), jamais
                    // `take`n — le Vec vit dans le sous-tableau
                    // JS : le consommer le vidait à jamais
                    // (§B-4(c) : YouTube `behaviors.flat(Infinity)`
                    // vidait `_.ev.YtActionHandlerBehavior` →
                    // proto sans `unregisterActionMap`).
                    let kids = match e.heap.obj(id) {
                        Some(o) => match &o.kind {
                            ObjKind::Array(v) => v.to_vec(),
                            _ => Vec::new(),
                        },
                        None => Vec::new(),
                    };
                    e.alloc_guard(kids.len().saturating_mul(std::mem::size_of::<Value>()))?;
                    for el in kids.into_iter().rev() {
                        stack.push((el, depth - 1));
                    }
                    continue;
                }
            }
        }
        out.push(v);
    }
    Ok(())
}

pub(crate) fn array_flat(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let id = this_array(e, this.clone())?;
    let depth = match arg(args, 0) {
        Value::Undefined => 1,
        v => e.to_number(v)? as u32,
    };
    // Lecture vivante (`iter_get`, JS-C335) : `IsConcatSpreadable`
    // et les `Get` descendants observent `this` peuplé (§B-90).
    let len = iter_len(e, id)?;
    let mut out = Vec::new();
    for i in 0..len {
        if let Some(v) = iter_get(e, id, i)? {
            flat_into(e, v, depth, &mut out)?;
        }
        if i % 8192 == 0 {
            e.tick()?;
        }
    }
    Ok(e.new_array(out))
}

pub(crate) fn array_flat_map(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let (id, cb, this_arg) = iter_cb(e, this.clone(), args, "flatMap")?;
    let len = iter_len(e, id)?;
    let mut out = Vec::new();
    for i in 0..len {
        if let Some(v) = iter_get(e, id, i)? {
            let r = e.call_value(
                cb.clone(),
                this_arg.clone(),
                &[v, Value::Num(i as f64), this.clone()],
            )?;
            flat_into(e, r, 1, &mut out)?;
        }
        if i % 8192 == 0 {
            e.tick()?;
        }
    }
    Ok(e.new_array(out))
}

pub(crate) fn array_find_last(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let (id, cb, this_arg) = iter_cb(e, this.clone(), args, "findLast")?;
    let len = iter_len(e, id)?;
    for i in (0..len).rev() {
        if let Some(v) = iter_get(e, id, i)? {
            let r = e.call_value(
                cb.clone(),
                this_arg.clone(),
                &[v.clone(), Value::Num(i as f64), this.clone()],
            )?;
            if e.to_boolean(r) {
                return Ok(v);
            }
        }
        if i % 8192 == 0 {
            e.tick()?;
        }
    }
    Ok(Value::Undefined)
}

pub(crate) fn array_find_last_index(
    e: &mut Engine,
    this: Value,
    args: &[Value],
) -> JsResult<Value> {
    let (id, cb, this_arg) = iter_cb(e, this.clone(), args, "findLastIndex")?;
    let len = iter_len(e, id)?;
    for i in (0..len).rev() {
        if let Some(v) = iter_get(e, id, i)? {
            let r = e.call_value(
                cb.clone(),
                this_arg.clone(),
                &[v, Value::Num(i as f64), this.clone()],
            )?;
            if e.to_boolean(r) {
                return Ok(Value::Num(i as f64));
            }
        }
        if i % 8192 == 0 {
            e.tick()?;
        }
    }
    Ok(Value::Num(-1.0))
}

// Array iterators — live cursors (JS-C627) : `length` relue et
// `[[Get]]` réel par index à chaque `next()`, spec
// `CreateArrayIterator` — les mutations pendant l'itération sont
// observées (push/pop, écritures d'index, `arguments` mappé).
pub(crate) fn array_values(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    this_array(e, this.clone())?;
    Ok(crate::collections::live_iter_obj(
        e,
        this,
        crate::heap::LiveIterKind::ArrValues,
        e.protos.array_iter,
    ))
}

pub(crate) fn array_keys(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    this_array(e, this.clone())?;
    Ok(crate::collections::live_iter_obj(
        e,
        this,
        crate::heap::LiveIterKind::ArrKeys,
        e.protos.array_iter,
    ))
}

pub(crate) fn array_entries(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    this_array(e, this.clone())?;
    Ok(crate::collections::live_iter_obj(
        e,
        this,
        crate::heap::LiveIterKind::ArrEntries,
        e.protos.array_iter,
    ))
}

// ES2023 non-mutating variants.
pub(crate) fn array_with(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let id = this_array(e, this.clone())?;
    // `with_elems` — `rel_index` faillible ; hors-bornes rend un
    // `None` distinct de l'erreur pour garder le `RangeError`.
    let res = with_elems(e, id, |e, elems| {
        let len = elems.len();
        match rel_index(e, arg(args, 0), len)? {
            Some(i) => {
                // `with` produit un résultat dense (spec `Get` →
                // trou relu `undefined`).
                let mut out_v: Vec<Value> = elems
                    .iter()
                    .map(super::super::value::Value::unhole)
                    .collect();
                out_v[i] = arg(args, 1);
                Ok(Some(e.new_array(out_v)))
            }
            None => Ok(None),
        }
    })?;
    match res {
        Some(out) => Ok(out),
        None => Err(e.range_error("with: index out of range")),
    }
}

pub(crate) fn array_to_reversed(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let id = this_array(e, this.clone())?;
    let elems = take_elems(e, id)?;
    let mut out_v: Vec<Value> = elems
        .iter()
        .map(super::super::value::Value::unhole)
        .collect();
    out_v.reverse();
    let out = e.new_array(out_v);
    put_elems(e, id, elems)?;
    let _ = args;
    Ok(out)
}

/// `splice` (mutable) — supprime `dc` éléments à l'indice `s`,
/// insère les args restants, retourne le tableau des supprimés.
/// `elems` n'est réécrit qu'à la fin : un throw en cours de
/// conversion des args laisse l'array intact.
pub(crate) fn array_splice(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let this_v = this.clone();
    let id = this_array(e, this.clone())?;
    // `with_elems` — `to_number` des args peut jeter : le tableau
    // reste intact (JS-C319). Le refus `sealed` repose aussi le
    // backing inchangé (`elems` n'est réécrit qu'en fin de corps).
    with_elems(e, id, |e, elems| {
        let len = elems.len();
        let start = match arg(args, 0) {
            Value::Undefined => 0,
            v => {
                // Clampe dans [0, len] comme la spec (`rel_index`
                // renvoie None hors bornes — les deux côtés doivent
                // clamper différemment, on calcule à la main).
                let i = e.to_number(v)? as isize;
                if i < 0 {
                    (len as isize + i).max(0) as usize
                } else {
                    (i as usize).min(len)
                }
            }
        };
        let del = if args.is_empty() {
            0
        } else if args.len() == 1 {
            // Un seul arg → supprime jusqu'à la fin (spec).
            len - start
        } else {
            (e.to_number(arg(args, 1))?.max(0.0) as usize).min(len - start)
        };
        let ins: &[Value] = args.get(2..).unwrap_or(&[]);
        let mut out_v = Vec::with_capacity(len - del + ins.len());
        out_v.extend_from_slice(&elems[..start]);
        let deleted: Vec<Value> = elems[start..start + del].to_vec();
        out_v.extend_from_slice(ins);
        out_v.extend_from_slice(&elems[start + del..]);
        // Sous `seal`/`freeze` toute suppression (`DeleteOrThrow`)
        // ou insertion (ajout interdit) lève `TypeError` — y
        // compris quand la longueur finale ne changerait pas.
        if e.heap.obj(id).is_some_and(|o| o.elem_integrity & 3 != 0) && (del > 0 || !ins.is_empty())
        {
            return Err(e.type_error("cannot splice a sealed array"));
        }
        // `A = ? ArraySpeciesCreate(O, actualDeleteCount)` (spec,
        // JS-C615) — `splice` fabriquait `new_array` direct :
        // `@@species`/`constructor` invisibles et le proto du
        // résultat ignorait le ctor résolu.
        let darr = species_array(e, this_v.clone(), del)?;
        if let Value::Obj(did) = darr.clone() {
            for (i, v) in deleted.iter().enumerate() {
                // `CreateDataPropertyOrThrow` — trou → `undefined`
                // (spec `Get`), jamais la cellule `Hole`.
                if !e.create_data_prop_spec_on(did, &i.to_string(), v.unhole())? {
                    return Err(e.type_error("splice: element is not creatable"));
                }
            }
        }
        e.set_prop_value(darr.clone(), "length", Value::Num(del as f64))?;
        *elems = out_v;
        Ok(darr)
    })
}

pub(crate) fn array_to_spliced(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let id = this_array(e, this.clone())?;
    // `with_elems` — `rel_index`/`to_number` faillibles avant le
    // put (JS-C319) ; `this` n'est jamais modifié (non-mutant).
    let out = with_elems(e, id, |e, elems| {
        let len = elems.len();
        let start = match arg(args, 0) {
            Value::Undefined => 0,
            v => rel_index(e, v, len)?.unwrap_or(len),
        };
        let del = match arg(args, 1) {
            Value::Undefined => len - start,
            v => (e.to_number(v)?.max(0.0) as usize).min(len - start),
        };
        let ins: &[Value] = args.get(2..).unwrap_or(&[]);
        let mut out_v = Vec::with_capacity(len - del + ins.len());
        // Résultat dense — `toSpliced` fait `Get` par slot (spec).
        out_v.extend(
            elems[..start]
                .iter()
                .map(super::super::value::Value::unhole),
        );
        out_v.extend_from_slice(ins);
        out_v.extend(
            elems[start + del..]
                .iter()
                .map(super::super::value::Value::unhole),
        );
        Ok(e.new_array(out_v))
    })?;
    Ok(out)
}

pub(crate) fn array_to_sorted(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    // Copy → sort via the shared comparator path → new array
    // (`sort` mutates `this`; `toSorted` must not).
    let id = this_array(e, this.clone())?;
    let elems = take_elems(e, id)?;
    // Dense — `toSorted` relit les slots par `Get` (spec).
    let copy = e.new_array(
        elems
            .iter()
            .map(super::super::value::Value::unhole)
            .collect(),
    );
    put_elems(e, id, elems)?;
    array_sort(e, copy.clone(), args)?;
    Ok(copy)
}

// -- String extras ----------------------------------------------------

pub(crate) fn string_at(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    // §B-59 : lecture en place — `this_str` clonait la chaîne
    // entière par appel (`at`/`charAt` en boucle de décodeur).
    if let Value::Str(id) = this {
        let len = id.u16_len();
        return Ok(match rel_index(e, arg(args, 0), len)? {
            Some(i) => match id.u16_at(i) {
                Some(w) => e.str_val_units(vec![w]),
                None => Value::Undefined,
            },
            None => Value::Undefined,
        });
    }
    let s = this_str(e, this)?;
    match rel_index(e, arg(args, 0), u16_len(&s))? {
        // `u16_at` : O(n) sans matérialiser la vue (§B-59).
        Some(i) => Ok(match u16_at(&s, i) {
            Some(t) => e.str_val(t),
            None => Value::Undefined,
        }),
        None => Ok(Value::Undefined),
    }
}

/// `codePointAt(i)` — `i` est un indice de code unit (spec) :
/// lead surrogate suivi d'un trail → le code point complet ;
/// sinon le code unit lui-même.
pub(crate) fn string_code_point_at(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    // `ToIntegerOrInfinity` : `NaN` → +0 ; **négatif →
    // undefined** (`codePointAt(-1)`, spec) ; troncature.
    let raw_i = {
        let n = e.to_number(arg(args, 0))?;
        if n.is_nan() {
            0.0
        } else {
            n.trunc()
        }
    };
    if raw_i < 0.0 {
        return Ok(Value::Undefined);
    }
    // Str direct : la vue `units` porte les surrogates isolés
    // (le `String` lossy les aurait déjà en U+FFFD).
    if let Value::Str(id) = this {
        let i = raw_i as usize;
        let u = id.units_view();
        let Some(&w) = u.get(i) else {
            return Ok(Value::Undefined);
        };
        let cp = code_point(&u, i, w);
        return Ok(Value::Num(f64::from(cp)));
    }
    let s = this_str(e, this)?;
    let i = raw_i as usize;
    // `encode_utf16().skip(i)` : O(i) sans matérialiser (§B-59).
    let mut it = s.encode_utf16().skip(i);
    let Some(w) = it.next() else {
        return Ok(Value::Undefined);
    };
    let cp = if (0xD800..0xDC00).contains(&w) {
        match it.next() {
            Some(t) if (0xDC00..0xE000).contains(&t) => {
                0x10000 + ((u32::from(w) - 0xD800) << 10) + (u32::from(t) - 0xDC00)
            }
            _ => u32::from(w),
        }
    } else {
        u32::from(w)
    };
    Ok(Value::Num(f64::from(cp)))
}

/// `u[i]` (lead `w` déjà lu) → code point : paire surrogate →
/// astral, sinon le code unit tel quel (spec `CodePointAt`).
fn code_point(u: &[u16], i: usize, w: u16) -> u32 {
    if (0xD800..0xDC00).contains(&w) {
        if let Some(&t) = u.get(i + 1) {
            if (0xDC00..0xE000).contains(&t) {
                return 0x10000 + ((u32::from(w) - 0xD800) << 10) + (u32::from(t) - 0xDC00);
            }
        }
    }
    u32::from(w)
}

/// `String.prototype[Symbol.iterator]` — itérateur de code points
/// (même granularité que `iter_values` : `char` Rust = scalaire).
/// Trigger réel : dev.to `homePage` invoque `str[Symbol.iterator]()`
/// directement — le fast-path `iter_values` ne couvre pas l'appel
/// explicite sur la primitive.
pub(crate) fn string_iterator(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    // Str direct : itérer les **code points** de la vue units —
    // paire surrogate → un item astral (2 units), surrogate
    // isolé → un item d'un unit préservé (spec `@@iterator`).
    if let Value::Str(id) = this {
        let cps = id.iter_code_points();
        e.alloc_guard(cps.len().saturating_mul(160))?;
        let items: Vec<Value> = cps.into_iter().map(|u| e.str_val_units(u)).collect();
        return Ok(crate::collections::iter_obj(e, items, e.protos.string_iter));
    }
    let s = this_str(e, this)?;
    // Garde §B-59 : `chars().collect()` matérialise par
    // caractère une `String` + une cellule `str_val` + une
    // entrée d'internement (~160 o) — mesuré en vol :
    // 20 M chars → 20 M cellules / 3,5 Go live, wedge.
    e.alloc_guard(s.len().saturating_mul(160))?;
    let items: Vec<Value> = s.chars().map(|c| e.str_val(c.to_string())).collect();
    Ok(crate::collections::iter_obj(e, items, e.protos.string_iter))
}

/// `matchAll(re)` — requires the `g` flag (spec `TypeError`); yields
/// an iterator of match arrays.
pub(crate) fn string_match_all(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    // Spec (JS-C618) : `RequireObjectCoercible` ; `IsRegExp` +
    // `flags` requis `g` ; `GetMethod(@@matchAll)` ; `ToString`
    // ; repli `RegExpCreate(rx, "g")` + `Invoke(@@matchAll)`.
    if let Value::Undefined | Value::Null = this {
        return Err(e.type_error("String method called on null/undefined"));
    }
    let pat = arg(args, 0);
    if !matches!(pat, Value::Undefined | Value::Null) {
        if is_regexp_spec(e, pat.clone())? {
            let fv = e.get_prop_value(pat.clone(), "flags")?;
            if !e.to_js_string(fv)?.contains('g') {
                return Err(e.type_error("matchAll requires a global regex"));
            }
        }
        if let Some(v) = sym_method(
            e,
            pat,
            crate::interp::SYM_MATCH_ALL,
            std::slice::from_ref(&this),
        )? {
            return Ok(v);
        }
    }
    let s = e.to_js_jstr(this)?;
    let g = e.str_val("g".to_string());
    let rx = regexp_create(e, arg(args, 0), g)?;
    let m = e.get_prop_value(rx.clone(), crate::interp::SYM_MATCH_ALL)?;
    e.call_value(m, rx, &[Value::Str(s)])
}

/// `String.fromCodePoint(...)` — 0..=0x10FFFF entier (spec :
/// les valeurs surrogate **passent** le `RangeError` et produisent
/// le code unit isolé — `fromCodePoint(0xD800)` ===
/// `fromCharCode(0xD800)`).
pub(crate) fn string_from_code_point(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    // Sortie ≤ 2 units/arg — refuse avant la boucle (§B-59,
    // apply géant).
    e.alloc_guard(args.len().saturating_mul(4))?;
    let mut units = Vec::with_capacity(args.len());
    let mut buf = [0u16; 2];
    for a in args {
        let n = e.to_number(a.clone())?;
        if !(0.0..=1_114_111.0).contains(&n) || n.fract() != 0.0 {
            return Err(e.range_error("fromCodePoint: invalid code point"));
        }
        match char::from_u32(n as u32) {
            Some(c) => units.extend_from_slice(c.encode_utf16(&mut buf)),
            // Seuls les surrogates échouent `from_u32` (n ≤
            // 0x10FFFF) — spec : l'unit isolé est produit tel
            // quel, pas U+FFFD.
            None => units.push(n as u16),
        }
    }
    Ok(e.str_val_units(units))
}

/// `String.raw(strings, …subs)` — join `strings.raw[i]` with the
/// substitutions (spec `GetTemplateObject` inverse — reads `.raw`
/// and `.length`, so array-likes work too).
pub(crate) fn string_raw(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let raw = e.get_prop_value(arg(args, 0), "raw")?;
    // Spec `ToLength` : `ToNumber` (Symbol → `TypeError`
    // propagé), troncature, borné à 2⁵³-1, `NaN`/négatif → +0.
    let n = match e.get_prop_value(raw.clone(), "length")? {
        Value::Undefined | Value::Null => 0,
        v => {
            let n = e.to_number(v)?;
            if n > 0.0 {
                n.min(9_007_199_254_740_991.0) as usize
            } else {
                0
            }
        }
    };
    let mut out = String::new();
    for i in 0..n {
        let seg = e.get_prop_value(raw.clone(), &i.to_string())?;
        out.push_str(&e.to_js_string(seg)?);
        // Spec : la substitution n'entre que si le segment n'est
        // **pas** le dernier (`k+1 < literalSegments`) — le
        // `args.get` inconditionnel collait une substitution en
        // trop (`{raw:['a','b','c']},X,Y,Z` → `aXbYcZ` au lieu
        // de `aXbYc`).
        if i + 1 < n {
            // Substitution manquante → `""` (spec — la boucle de
            // segments **continue**, cf. `return-the-string-value`).
            if let Some(v) = args.get(i + 1) {
                out.push_str(&e.to_js_string(v.clone())?);
            }
        }
    }
    Ok(e.str_val(out))
}

// -- TextEncoder / TextDecoder (UTF-8 only) ----------------------------

/// `new TextEncoder()` — UTF-8 only (labeled encodings are out of
/// scope for the mirror engine; `encode` yields a real `Uint8Array`).
pub(crate) fn text_encoder_ctor(e: &mut Engine, _t: Value, _a: &[Value]) -> JsResult<Value> {
    let id = e.new_object();
    e.add_method(id, "encode", text_encoder_encode);
    let enc = e.str_val("utf-8");
    e.set_own(id, "encoding", enc, true);
    Ok(Value::Obj(id))
}

pub(crate) fn text_encoder_encode(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let s = e.to_js_string(arg(args, 0))?;
    Ok(crate::typed_array::new_u8_array(e, s.into_bytes()))
}

/// `new TextDecoder(label?, options?)` — UTF-8 et UTF-16
/// (labels WHATWG canonisés ; autre label → `RangeError`
/// comme les vrais moteurs). §B-204 : `utf-16le` est le
/// décodeur `UTF16ToString` des runtimes Emscripten/wasm
/// (diagrams.net). `options.fatal` : données invalides →
/// `TypeError` au lieu du remplacement `U+FFFD` (audit 4
/// voix — l'option était ignorée) ; `options.ignoreBOM`
/// : `true` = le BOM est livré dans la sortie (spec — le
/// défaut le retire).
pub(crate) fn text_decoder_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let label = match arg(args, 0) {
        Value::Undefined => "utf-8".to_string(),
        v => e.to_js_string(v)?.to_lowercase(),
    };
    // Labels WHATWG « get an encoding » → nom canonique.
    let enc = match label.as_str() {
        "utf-8" | "utf8" | "unicode-1-1-utf-8" | "unicode11utf8" | "unicode20utf8"
        | "x-unicode20utf8" => "utf-8",
        "utf-16" | "utf-16le" | "csunicode" | "iso-10646-ucs-2" | "ucs-2" | "unicode"
        | "unicodefeffbig" => "utf-16le",
        "utf-16be" | "unicodefffe" => "utf-16be",
        _ => return Err(e.range_error("unsupported encoding")),
    };
    let flag = |e: &mut Engine, key: &str| match arg(args, 1) {
        Value::Obj(id) => {
            let v = e.get_prop(id, key).unwrap_or(Value::Undefined);
            e.to_boolean(v)
        }
        _ => false,
    };
    let fatal = flag(e, "fatal");
    let ignore_bom = flag(e, "ignoreBOM");
    let id = e.new_object();
    e.add_method(id, "decode", text_decoder_decode);
    let enc_v = e.str_val(enc);
    e.set_own(id, "encoding", enc_v, true);
    e.set_own(id, "fatal", Value::Bool(fatal), true);
    e.set_own(id, "ignoreBOM", Value::Bool(ignore_bom), true);
    Ok(Value::Obj(id))
}

pub(crate) fn text_decoder_decode(e: &mut Engine, t: Value, args: &[Value]) -> JsResult<Value> {
    let bytes: Vec<u8> = match arg(args, 0) {
        // Vue typée / `DataView` / `ArrayBuffer` — ses octets bruts.
        Value::Obj(id) => match crate::typed_array::bytes_of_value(e, Value::Obj(id)) {
            Some(b) => b,
            // iterable/array-like de nombres (`decode([72,105])`).
            None => e
                .iter_values(Value::Obj(id))?
                .iter()
                .filter_map(|v| match v {
                    Value::Num(n) => Some(*n as u8),
                    _ => None,
                })
                .collect(),
        },
        _ => Vec::new(),
    };
    let flag = |e: &mut Engine, key: &str| match t {
        Value::Obj(id) => {
            let v = e.get_prop(id, key).unwrap_or(Value::Undefined);
            e.to_boolean(v)
        }
        _ => false,
    };
    let enc = match t {
        Value::Obj(id) => match e.get_prop(id, "encoding") {
            Some(Value::Str(s)) => s.to_string(),
            _ => "utf-8".to_string(),
        },
        _ => "utf-8".to_string(),
    };
    if enc == "utf-16le" || enc == "utf-16be" {
        return decode_utf16(e, &bytes, enc == "utf-16be", &flag);
    }
    let mut slice = &bytes[..];
    // Spec : le BOM UTF-8 est retiré par défaut —
    // `ignoreBOM:true` le conserve (la convention historique
    // était inversée, §B-204).
    if !flag(e, "ignoreBOM") && slice.starts_with(&[0xEF, 0xBB, 0xBF]) {
        slice = &slice[3..];
    }
    match String::from_utf8(slice.to_vec()) {
        Ok(s) => Ok(e.str_val(s)),
        Err(_) if flag(e, "fatal") => {
            Err(e.type_error("The encoded data was not valid for encoding utf-8"))
        }
        Err(_) => Ok(e.str_val(String::from_utf8_lossy(slice).into_owned())),
    }
}

/// `decode` UTF-16LE/BE — paires d'octets → units `u16`
/// livrées telles quelles (un surrogate isolé reste isolé —
/// WHATWG ; le surrogate n'est *jamais* réencodé). BOM :
/// `ignoreBOM:false` → le premier couple `FF FE` force LE,
/// `FE FF` force BE (sniff, spec) ; `ignoreBOM:true` → tout
/// est décodé littéralement dans l'endianness déclarée.
/// Octet impair final → `U+FFFD` (`fatal` → `TypeError`).
fn decode_utf16(
    e: &mut Engine,
    bytes: &[u8],
    declared_be: bool,
    flag: &dyn Fn(&mut Engine, &str) -> bool,
) -> JsResult<Value> {
    let mut be = declared_be;
    let mut i = 0usize;
    if !flag(e, "ignoreBOM") && bytes.len() >= 2 {
        if bytes[0] == 0xFF && bytes[1] == 0xFE {
            be = false;
            i = 2;
        } else if bytes[0] == 0xFE && bytes[1] == 0xFF {
            be = true;
            i = 2;
        }
    }
    let mut units: Vec<u16> = Vec::with_capacity(bytes.len() / 2 + 1);
    let mut odd_tail = false;
    while i + 1 < bytes.len() {
        let (a, b) = (bytes[i], bytes[i + 1]);
        units.push(if be {
            u16::from_be_bytes([a, b])
        } else {
            u16::from_le_bytes([a, b])
        });
        i += 2;
    }
    if i < bytes.len() {
        if flag(e, "fatal") {
            return Err(e.type_error("The encoded data was not valid for encoding utf-16"));
        }
        odd_tail = true;
    }
    if odd_tail {
        units.push(0xFFFD);
    }
    Ok(e.str_val_units(units))
}

// -- Math extras --------------------------------------------------------

macro_rules! math1 {
    ($name:ident, $op:expr) => {
        pub(crate) fn $name(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
            let x = e.to_number(arg(args, 0))?;
            let f: fn(f64) -> f64 = $op;
            Ok(Value::Num(f(x)))
        }
    };
}

math1!(math_trunc, f64::trunc);
// `Math.sign` : spec renvoie x tel quel pour ±0/NaN (signe du
// zéro préservé) — `f64::signum` rend +1 pour -0, d'où le lambda.
math1!(math_sign, |x: f64| {
    if x.is_nan() || x == 0.0 {
        x
    } else {
        x.signum()
    }
});
math1!(math_cbrt, f64::cbrt);
math1!(math_log2, f64::log2);
math1!(math_log10, f64::log10);
math1!(math_log1p, f64::ln_1p);
math1!(math_expm1, f64::exp_m1);
math1!(math_fround, |x| x as f32 as f64);
math1!(math_asin, f64::asin);
math1!(math_acos, f64::acos);
math1!(math_sinh, f64::sinh);
math1!(math_cosh, f64::cosh);
math1!(math_tanh, f64::tanh);

pub(crate) fn math_hypot(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let mut acc = 0.0_f64;
    for a in args {
        let x = e.to_number(a.clone())?;
        acc = acc.hypot(x);
    }
    Ok(Value::Num(acc))
}

pub(crate) fn math_imul(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let a = to_int32(e.to_number(arg(args, 0))?);
    let b = to_int32(e.to_number(arg(args, 1))?);
    Ok(Value::Num(f64::from(a.wrapping_mul(b))))
}

pub(crate) fn math_clz32(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let x = to_uint32(e.to_number(arg(args, 0))?);
    Ok(Value::Num(f64::from(x.leading_zeros())))
}

// -- Number extras --------------------------------------------------------

/// `{:.*e}` gives `d.ddd e±exp` — JS wants the sign always and
/// two-digit minimum exponent is optional; normalize the exponent.
pub(crate) fn js_exp(mut s: String) -> String {
    if let Some(i) = s.rfind('e') {
        let exp = &s[i + 1..];
        if !exp.starts_with(['-', '+']) {
            s.insert(i + 1, '+');
        }
    }
    s
}

pub(crate) fn number_to_precision(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let n = this_num(e, this)?;
    let n = if n == 0.0 { 0.0 } else { n }; // -0 → 0, cf. toFixed
    let p = match arg(args, 0) {
        Value::Undefined => return Ok(e.str_val(num_to_js_string(n))),
        v => e.to_number(v)? as usize,
    };
    if !(1..=100).contains(&p) {
        return Err(e.range_error("toPrecision: digits out of range"));
    }
    if !n.is_finite() || n == 0.0 {
        return Ok(e.str_val(num_to_js_string(n)));
    }
    let exp = n.abs().log10().floor() as i32;
    if exp < -6 || exp >= p as i32 {
        Ok(e.str_val(js_exp(format!("{n:.prec$e}", prec = p - 1))))
    } else {
        let dec = (p as i32 - 1 - exp).max(0) as usize;
        let mut s = format!("{n:.dec$}");
        while s.contains('.') && s.ends_with('0') {
            s.pop();
        }
        if s.ends_with('.') {
            s.pop();
        }
        Ok(e.str_val(s))
    }
}

pub(crate) fn number_to_exponential(
    e: &mut Engine,
    this: Value,
    args: &[Value],
) -> JsResult<Value> {
    let n = this_num(e, this)?;
    let n = if n == 0.0 { 0.0 } else { n }; // -0 → 0, cf. toFixed
    let d = match arg(args, 0) {
        Value::Undefined => {
            // As many digits as needed to represent uniquely — Rust's
            // `{e}` gives the shortest, close enough for our subset.
            return Ok(e.str_val(js_exp(format!("{n:e}"))));
        }
        v => e.to_number(v)? as usize,
    };
    if d > 100 {
        return Err(e.range_error("toExponential: digits out of range"));
    }
    if !n.is_finite() {
        return Ok(e.str_val(num_to_js_string(n)));
    }
    Ok(e.str_val(js_exp(format!("{n:.d$e}"))))
}
