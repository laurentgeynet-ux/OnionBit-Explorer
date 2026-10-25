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
    let a = if e.is_callable(this) {
        e.construct(this, &[Value::Num(len as f64)])?
    } else {
        e.new_array(vec![Value::Undefined; len])
    };
    for (i, v) in args.iter().enumerate() {
        e.set_prop_value(a, &i.to_string(), *v)?;
    }
    e.set_prop_value(a, "length", Value::Num(len as f64))?;
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
        Some(m) if e.is_callable(m) => e.call_value(m, obj, args).map(Some),
        Some(_) => Err(e.type_error("symbol method is not callable")),
    }
}

/// `ArraySpeciesCreate(src, len)` (spec) : `new src.constructor
/// [@@species](len)` si la prop existe ; un `undefined`/`null` →
/// tableau ordinaire, autre chose → `TypeError` via `construct`.
pub(crate) fn species_array(e: &mut Engine, src: Value, len: usize) -> JsResult<Value> {
    if let Value::Obj(id) = src {
        if matches!(e.heap.obj(id).map(|o| &o.kind), Some(ObjKind::Array(_))) {
            let sp = e.get_prop(id, "constructor").and_then(|c| match c {
                Value::Obj(cid) => e.get_prop(cid, crate::interp::SYM_SPECIES),
                _ => None,
            });
            match sp {
                None | Some(Value::Undefined | Value::Null) => {}
                Some(ctor_v) => return e.construct(ctor_v, &[Value::Num(len as f64)]),
            }
        }
    }
    Ok(e.new_array(Vec::new()))
}

/// Algorithme `rx.exec`-résultat partagé `str.match`/`@@match`
/// (spec : global → liste de textes + `lastIndex` reset ; sinon
/// résultat d'un seul `exec`).
pub(crate) fn re_match_exec(
    e: &mut Engine,
    re: &std::rc::Rc<crate::regexp::JsRegExp>,
    id: crate::value::ObjId,
    s: &str,
) -> JsResult<Value> {
    if re.global {
        let matches = re.exec_all(s, 0);
        e.note_re(s, matches.last());
        let texts: Vec<Value> = matches.into_iter().map(|m| e.str_val(m.text)).collect();
        let _ = e.set_prop(id, "lastIndex", Value::Num(0.0));
        return Ok(if texts.is_empty() {
            Value::Null
        } else {
            arr_of(e, texts)
        });
    }
    let m = re.exec(s, 0);
    e.note_re(s, m.as_ref());
    match m {
        Some(m) => Ok(match_result(e, &m, re, s)),
        None => Ok(Value::Null),
    }
}

pub(crate) fn re_match_all_impl(
    e: &mut Engine,
    re: &std::rc::Rc<crate::regexp::JsRegExp>,
    s: &str,
) -> JsResult<Value> {
    if !re.global {
        return Err(e.type_error("matchAll needs a global regex"));
    }
    let items: Vec<Value> = re
        .exec_all(s, 0)
        .iter()
        .map(|m| match_result(e, m, re, s))
        .collect();
    Ok(crate::collections::iter_obj(e, items, e.protos.regexp_iter))
}

/// Branche `RegExp` de `replace`/`@@replace` (global → toutes les
/// occurrences ; sinon la première).
pub(crate) fn re_replace_impl(
    e: &mut Engine,
    re: &std::rc::Rc<crate::regexp::JsRegExp>,
    s: &str,
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
    let matches = if re.global {
        re.exec_all(s, 0)
    } else {
        re.exec(s, 0).into_iter().collect()
    };
    if std::env::var("DBG_DRIVE").is_ok() {
        eprintln!("[re_replace] matches={} s.len={}", matches.len(), s.len());
    }
    e.note_re(s, matches.last());
    if matches.is_empty() {
        return Ok(e.str_val(s.to_string()));
    }
    splice_matches(e, s, &matches, rep)
}

/// `rx[Symbol.match](str)` — `RegExp.prototype[@@match]` (spec).
pub(crate) fn re_sym_match(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let s = e.to_js_string(arg(args, 0))?;
    let (re, id) = as_regexp(e, this)?;
    re_match_exec(e, &re, id, &s)
}

/// `rx[Symbol.matchAll](str)` — itérateur de résultats, `g` requis.
pub(crate) fn re_sym_match_all(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let s = e.to_js_string(arg(args, 0))?;
    let (re, _id) = as_regexp(e, this)?;
    re_match_all_impl(e, &re, &s)
}

/// `rx[Symbol.replace](str, rep)` — global ou première occurrence.
pub(crate) fn re_sym_replace(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let s = e.to_js_string(arg(args, 0))?;
    let rep = arg(args, 1);
    let (re, _id) = as_regexp(e, this)?;
    re_replace_impl(e, &re, &s, rep)
}

/// `rx[Symbol.search](str)` — indice du premier match, `-1` sinon.
pub(crate) fn re_sym_search(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let s = e.to_js_string(arg(args, 0))?;
    let (re, _id) = as_regexp(e, this)?;
    let m = re.exec(&s, 0);
    e.note_re(&s, m.as_ref());
    match m {
        Some(m) => Ok(Value::Num(m.index as f64)),
        None => Ok(Value::Num(-1.0)),
    }
}

/// `rx[Symbol.split](str, limit)` — captures insérées entre les
/// morceaux (délègue à `JsRegExp::split`).
pub(crate) fn re_sym_split(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let s = e.to_js_string(arg(args, 0))?;
    let limit = match arg(args, 1) {
        Value::Undefined => usize::MAX,
        v => e.to_number(v)? as usize,
    };
    let (re, _id) = as_regexp(e, this)?;
    if limit == 0 {
        return Ok(arr_of(e, Vec::new()));
    }
    let vals: Vec<Value> = re
        .split(&s)
        .into_iter()
        .take(limit)
        .map(|p| e.str_val(p))
        .collect();
    Ok(arr_of(e, vals))
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
