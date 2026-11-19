// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Builtins JS — section extraite de `builtins.rs` (JS-C40, pur
//! reformatage mécanique : sémantique et tests inchangés).

#[allow(clippy::wildcard_imports)]
use super::*;

// ------------------------------------------------------------------
// JS-C15 — `Array.of`, dispatch `@@*`, `@@species`, captureStackTrace
// ------------------------------------------------------------------

/// `Array.of(…items)` — spec : si `this` est un ctor → `new this(len)`
/// (sous-classes `Array.of.call(C, …)`), sinon un Array ordinaire.
pub(crate) fn array_of(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let len = args.len();
    // Spec : `IsConstructor(this)` → `A = Construct(this, «len»)`,
    // sinon `ArrayCreate`. Éléments via `CreateDataPropertyOrThrow`
    // — les setters/accessors de la chaîne proto ne sont jamais
    // sollicités (`does-not-use-prototype-properties`).
    let a = if e.is_constructor(this.clone()) {
        e.construct(this, &[Value::Num(len as f64)])?
    } else {
        e.new_array(vec![Value::Undefined; len])
    };
    for (i, v) in args.iter().enumerate() {
        let ok = match a {
            Value::Obj(aid) => e.create_data_prop_spec_on(aid, &i.to_string(), v.clone())?,
            _ => true,
        };
        if !ok {
            return Err(e.type_error("Array.of: element is not creatable"));
        }
    }
    e.set_prop_value(a.clone(), "length", Value::Num(len as f64))?;
    Ok(a)
}

/// `GetMethod(obj, sym)` — dispatch spec `str.x(rx)` → `rx[@@x](…)`.
/// `Ok(None)` → chemin builtin (`sym` absent, `undefined` ou `null`).
pub(crate) fn sym_method(
    e: &mut Engine,
    obj: Value,
    sym: &str,
    args: &[Value],
) -> JsResult<Option<Value>> {
    let Value::Obj(id) = obj else { return Ok(None) };
    match e.get_prop(id, sym) {
        None | Some(Value::Undefined | Value::Null) => Ok(None),
        Some(m) if e.is_callable(m.clone()) => e.call_value(m, obj, args).map(Some),
        Some(_) => Err(e.type_error("symbol method is not callable")),
    }
}

/// `get [Symbol.species]` (spec) — `return this`. Accesseur
/// `{e:false, c:true}` posé par [`install_species`] sur chaque
/// ctor species-aware (audit boa A5).
pub(crate) fn species_get(_e: &mut Engine, this: Value, _args: &[Value]) -> JsResult<Value> {
    Ok(this)
}

/// Pose `get [Symbol.species]` sur le ctor `ctor` (`Array`,
/// `ArrayBuffer`, `Map`, `Set`, `RegExp`, `SharedArrayBuffer`,
/// `TypedArray` + ses concrets — spec).
pub(crate) fn install_species(e: &mut Engine, ctor: &Value) {
    if let Value::Obj(cid) = ctor {
        let sg = e.native_fn("get [Symbol.species]", species_get);
        if let Some(o) = e.heap.obj_mut(*cid) {
            o.set_accessor(crate::interp::SYM_SPECIES, sg, Value::Undefined, false);
        }
    }
}

/// `ArraySpeciesCreate(src, len)` (spec) : `new src.constructor
/// [@@species](len)` ; `undefined`/`null` → tableau ordinaire.
/// Lectures **observables** (`Get`) — `constructor` et `@@species`
/// invoquent leurs accesseurs (JS-C615 ; avant `get_prop`
/// data-only, les getters étaient invisibles).
///
/// Démotion cross-realm (spec étape 6.c) : si `C` est un ctor
/// d'un **autre** realm identique à **son** `%Array%`, `C` est
/// ramené à `undefined` — `@@species` n'est alors jamais lu
/// (`create-proto-from-ctor-realm-array` : `callCount` reste 0).
/// Déviation cataloguée : `IsArray` ne traverse pas les proxies.
pub(crate) fn species_array(e: &mut Engine, src: Value, len: usize) -> JsResult<Value> {
    // `ArrayCreate(len)` — le repli valide la borne spec
    // (`len > 2³²−1` → `RangeError`) et pose `length` ; la
    // représentation sparse ne densifie rien.
    let array_create = |e: &mut Engine| -> JsResult<Value> {
        if len > u32::MAX as usize {
            return Err(e.range_error("Invalid array length"));
        }
        Ok(e.new_array_len(len as u32))
    };
    let Value::Obj(id) = src else {
        return array_create(e);
    };
    if !matches!(e.heap.obj(id).map(|o| &o.kind), Some(ObjKind::Array(_))) {
        return array_create(e);
    }
    let mut c = e.get_prop_value(src.clone(), "constructor")?;
    if e.is_constructor(c.clone()) {
        if let Some(rc) = e.fn_realm(&c) {
            if rc != e.cur_ctx && e.realm_global_own(rc, "Array").is_some_and(|a| a == c) {
                c = Value::Undefined;
            }
        }
    }
    if e.is_spec_object(&c) {
        match e.get_prop_value(c, crate::interp::SYM_SPECIES)? {
            Value::Undefined | Value::Null => {}
            s => return e.construct(s, &[Value::Num(len as f64)]),
        }
    }
    array_create(e)
}

/// Algorithme `rx.exec`-résultat partagé `str.match`/`@@match`
/// (spec : global → liste de textes + `lastIndex` reset ; sinon
/// résultat d'un seul `exec`).
pub(crate) fn re_match_exec(
    e: &mut Engine,
    re: &std::rc::Rc<crate::regexp::JsRegExp>,
    id: crate::value::ObjId,
    s: &crate::value::JsStr,
) -> JsResult<Value> {
    let (hay, dec) = re.enc_hay(s);
    if re.global {
        // Garde §B-59 : `exec_all` matérialise TOUS les matches
        // en eager — un match par char sur géante chaîne →
        // `len` Vec<Match> + cellules sans tick.
        e.alloc_guard(hay.len().saturating_mul(160))?;
        let matches = re.exec_all(&hay, 0);
        e.note_re(&hay, matches.last());
        let texts: Vec<Value> = matches
            .into_iter()
            .map(|m| re_str_val(e, &m.text, dec))
            .collect();
        let _ = e.set_prop(id, "lastIndex", Value::Num(0.0));
        return Ok(if texts.is_empty() {
            Value::Null
        } else {
            arr_of(e, texts)
        });
    }
    let m = re.exec(&hay, 0);
    e.note_re(&hay, m.as_ref());
    match m {
        Some(m) => Ok(match_result(e, &m, re, s, dec)),
        None => Ok(Value::Null),
    }
}

pub(crate) fn re_match_all_impl(
    e: &mut Engine,
    re: &std::rc::Rc<crate::regexp::JsRegExp>,
    s: &crate::value::JsStr,
) -> JsResult<Value> {
    if !re.global {
        return Err(e.type_error("matchAll needs a global regex"));
    }
    let (hay, dec) = re.enc_hay(s);
    e.alloc_guard(hay.len().saturating_mul(160))?;
    let items: Vec<Value> = re
        .exec_all(&hay, 0)
        .iter()
        .map(|m| match_result(e, m, re, s, dec))
        .collect();
    Ok(crate::collections::iter_obj(e, items, e.protos.regexp_iter))
}

/// Branche `RegExp` de `replace`/`@@replace` (global → toutes les
/// occurrences ; sinon la première).
pub(crate) fn re_replace_impl(
    e: &mut Engine,
    re: &std::rc::Rc<crate::regexp::JsRegExp>,
    s: &crate::value::JsStr,
    rep: Value,
) -> JsResult<Value> {
    if std::env::var("DBG_DRIVE").is_ok() {
        eprintln!(
            "[re_replace] src={:?} g={} s.len={}",
            re.source,
            re.global,
            s.len()
        );
    }
    let (hay, dec) = re.enc_hay(s);
    let matches = if re.global {
        e.alloc_guard(hay.len().saturating_mul(160))?;
        re.exec_all(&hay, 0)
    } else {
        re.exec(&hay, 0).into_iter().collect()
    };
    if std::env::var("DBG_DRIVE").is_ok() {
        eprintln!("[re_replace] matches={} s.len={}", matches.len(), hay.len());
    }
    e.note_re(&hay, matches.last());
    if matches.is_empty() {
        return Ok(Value::Str(s.clone()));
    }
    splice_matches(e, &hay, s, &matches, rep, dec)
}

/// `rx[Symbol.match](str)` — `RegExp.prototype[@@match]` (spec).
pub(crate) fn re_sym_match(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let s = e.to_js_jstr(arg(args, 0))?;
    let (re, id) = as_regexp(e, this)?;
    re_match_exec(e, &re, id, &s)
}

/// `rx[Symbol.matchAll](str)` — itérateur de résultats, `g` requis.
pub(crate) fn re_sym_match_all(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let s = e.to_js_jstr(arg(args, 0))?;
    let (re, _id) = as_regexp(e, this)?;
    re_match_all_impl(e, &re, &s)
}

/// `rx[Symbol.replace](str, rep)` — global ou première occurrence.
pub(crate) fn re_sym_replace(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let s = e.to_js_jstr(arg(args, 0))?;
    let rep = arg(args, 1);
    let (re, _id) = as_regexp(e, this)?;
    re_replace_impl(e, &re, &s, rep)
}

/// `rx[Symbol.search](str)` — indice du premier match, `-1` sinon.
pub(crate) fn re_sym_search(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let s = e.to_js_jstr(arg(args, 0))?;
    let (re, _id) = as_regexp(e, this)?;
    let (hay, _dec) = re.enc_hay(&s);
    let m = re.exec(&hay, 0);
    e.note_re(&hay, m.as_ref());
    match m {
        Some(m) => Ok(Value::Num(m.index as f64)),
        None => Ok(Value::Num(-1.0)),
    }
}

/// `RegExpExec(R, S)` (spec 22.2.7.1, JS-C617) : `exec`
/// callable (lu par `Get` — observable) → `Call(exec, R, «S»)`,
/// résultat non-objet non-null → `TypeError` ; non-callable →
/// `RegExpBuiltinExec` — le slot `[[RegExpMatcher]]` est requis
/// (un objet sans slot → `TypeError`).
fn reg_exp_exec(e: &mut Engine, rx: Value, s: Value) -> JsResult<Value> {
    let exec = e.get_prop_value(rx.clone(), "exec")?;
    if e.is_callable(exec.clone()) {
        let r = e.call_value(exec, rx, &[s])?;
        if !e.is_spec_object(&r) && !matches!(r, Value::Null) {
            return Err(e.type_error("RegExp exec result must be an Object or null"));
        }
        return Ok(r);
    }
    if matches!(rx, Value::Obj(id)
        if matches!(e.heap.obj(id).map(|o| &o.kind), Some(ObjKind::RegExp(_))))
    {
        return regexp_exec(e, rx, &[s]);
    }
    Err(e.type_error("not a RegExp"))
}

/// `%RegExp%` du realm courant — défaut `SpeciesConstructor`.
fn regexp_intrinsic(e: &mut Engine) -> JsResult<Value> {
    e.realm_global_own(e.cur_ctx, "RegExp")
        .ok_or_else(|| e.type_error("missing %RegExp% intrinsic"))
}

/// `array[lengthA] = v` (`CreateDataPropertyOrThrow`) puis
/// `lengthA += 1` — `true` quand `lengthA` atteint `lim`.
fn emit_split(
    e: &mut Engine,
    aid: ObjId,
    len_a: &mut usize,
    lim: usize,
    v: Value,
) -> JsResult<bool> {
    if !e.create_data_prop_spec_on(aid, &len_a.to_string(), v)? {
        return Err(e.type_error("split: element is not creatable"));
    }
    *len_a += 1;
    Ok(*len_a >= lim)
}

/// `rx[Symbol.split](str, limit)` — spec 22.2.6.14 (JS-C617) :
/// `SpeciesConstructor` → `Construct(C, «rx, flags+y»)` → boucle
/// `RegExpExec` à `lastIndex` **observable** (le `exec` d'un
/// splitter custom peut le déplacer — `matchEnd` est relu après
/// l'appel, pas inféré du match). Captures intercalées, `lim`
/// borne `lengthA`. Avant : stub natif `JsRegExp::split` — ni
/// species, ni flags, ni `lastIndex`, ni `exec` custom
/// n'étaient observés.
pub(crate) fn re_sym_split(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    if !e.is_spec_object(&this) {
        return Err(e.type_error("Symbol.split receiver is not an object"));
    }
    let s = e.to_js_jstr(arg(args, 0))?;
    // `SpeciesConstructor(rx, %RegExp%)` : `constructor` puis
    // `@@species` lus par `Get` (accesseurs observables).
    let c = e.get_prop_value(this.clone(), "constructor")?;
    let ctor = if matches!(c, Value::Undefined) {
        regexp_intrinsic(e)?
    } else {
        if !e.is_spec_object(&c) {
            return Err(e.type_error("constructor is not an object"));
        }
        match e.get_prop_value(c, crate::interp::SYM_SPECIES)? {
            Value::Undefined | Value::Null => regexp_intrinsic(e)?,
            sc if e.is_constructor(sc.clone()) => sc,
            _ => return Err(e.type_error("@@species is not a constructor")),
        }
    };
    let flags_v = e.get_prop_value(this.clone(), "flags")?;
    let flags = e.to_js_string(flags_v)?;
    // `unicodeMatching` — sert `AdvanceStringIndex` ; +1 char
    // est correct des deux côtés dans notre domaine char.
    let _unicode = flags.contains('u') || flags.contains('v');
    let new_flags = if flags.contains('y') {
        flags
    } else {
        format!("{flags}y")
    };
    let nf_v = e.str_val(new_flags);
    let splitter = e.construct(ctor, &[this, nf_v])?;
    let a = e.new_array(Vec::new());
    let Value::Obj(aid) = a.clone() else {
        return Ok(a);
    };
    let lim = match arg(args, 1) {
        Value::Undefined => u32::MAX as usize,
        v => to_uint32(e.to_number(v)?) as usize,
    };
    if lim == 0 {
        return Ok(a);
    }
    // Domaine encodé quand `s` porte des `units` (lone
    // surrogates) — même convention qu'`enc_hay` : les pièces
    // émises se décodent via `re_str_val(e, _, dec)`.
    let (hay_cow, dec): (std::borrow::Cow<'_, str>, bool) = if s.has_units() {
        (
            std::borrow::Cow::Owned(crate::regexp::re_encode(&s.units_view())),
            true,
        )
    } else {
        (std::borrow::Cow::Borrowed(s.as_str()), false)
    };
    let hay: Vec<char> = hay_cow.chars().collect();
    let s_arg = Value::Str(s);
    let size = hay.len();
    let mut length_a = 0usize;
    // Chaîne vide : un match → `[]`, sinon `[s]`.
    if size == 0 {
        let mr = reg_exp_exec(e, splitter, s_arg)?;
        if !matches!(mr, Value::Null) {
            return Ok(a);
        }
        let empty_v = re_str_val(e, "", dec);
        emit_split(e, aid, &mut length_a, lim, empty_v)?;
        return Ok(a);
    }
    let mut last_match_end = 0usize;
    let mut search_index = 0usize;
    while search_index < size {
        // `Set(splitter, "lastIndex", 𝔽(q), true)` — observable.
        e.set_prop_value(
            splitter.clone(),
            "lastIndex",
            Value::Num(search_index as f64),
        )?;
        let mr = reg_exp_exec(e, splitter.clone(), s_arg.clone())?;
        if matches!(mr, Value::Null) {
            // `AdvanceStringIndex` : +1 char — en unicode les
            // paires de surrogates sont déjà un char unique ;
            // non-unicode sur l'astral avance d'un char au lieu
            // d'un code unit (déviation cataloguée).
            search_index += 1;
            continue;
        }
        // `matchEnd = ToLength(Get(splitter,"lastIndex"))` —
        // relu *après* `exec`, borné à `size`.
        let end_v = e.get_prop_value(splitter.clone(), "lastIndex")?;
        let match_end = (e.to_number(end_v)?.max(0.0) as usize).min(size);
        if match_end == last_match_end {
            search_index += 1;
            continue;
        }
        let piece: String = hay[last_match_end..search_index].iter().collect();
        let piece_v = re_str_val(e, &piece, dec);
        if emit_split(e, aid, &mut length_a, lim, piece_v)? {
            return Ok(a);
        }
        last_match_end = match_end;
        // Captures : `LengthOfArrayLike(matchResult) - 1` puis
        // `Get(matchResult, i)` — observable (un résultat custom
        // peut mentir sur ses propres indexes).
        let n_caps = {
            let lv = e.get_prop_value(mr.clone(), "length")?;
            (e.to_number(lv)?.max(0.0) as usize).saturating_sub(1)
        };
        for ci in 1..=n_caps {
            let cap = e.get_prop_value(mr.clone(), &ci.to_string())?;
            if emit_split(e, aid, &mut length_a, lim, cap)? {
                return Ok(a);
            }
        }
        search_index = last_match_end;
    }
    let piece: String = hay[last_match_end..size].iter().collect();
    let piece_v = re_str_val(e, &piece, dec);
    emit_split(e, aid, &mut length_a, lim, piece_v)?;
    Ok(a)
}

/// `Error.captureStackTrace(obj, ctor?)` — extension V8 : pose la
/// own prop `stack` non-énumérable (en-tête + frames réelles,
/// JS-C47 — l'argument `ctor` d'élision de frames n'est pas
/// honoré : toute la pile est rendue, déviation documentée).
pub(crate) fn error_capture_stack(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let Value::Obj(id) = arg(args, 0) else {
        return Err(e.type_error("captureStackTrace target must be an object"));
    };
    let name = e
        .get_prop(id, "name")
        .and_then(|v| e.to_js_string(v).ok())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "Error".to_string());
    let msg = e
        .get_prop(id, "message")
        .and_then(|v| e.to_js_string(v).ok())
        .unwrap_or_default();
    let s = e.str_val(e.stack_text(&name, &msg));
    e.set_own(id, "stack", s, false);
    Ok(Value::Undefined)
}
