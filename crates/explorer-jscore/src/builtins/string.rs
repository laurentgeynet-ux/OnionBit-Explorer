// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Builtins JS — section extraite de `builtins.rs` (JS-C40, pur
//! reformatage mécanique : sémantique et tests inchangés).

#[allow(clippy::wildcard_imports)]
use super::*;

// ------------------------------------------------------------------
// String
// ------------------------------------------------------------------

pub(crate) fn string_to_string(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    this_string_strict(e, this)
}

pub(crate) fn string_char_at(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let s = this_str(e, this)?;
    let i = match arg(args, 0) {
        Value::Undefined => 0.0,
        v => e.to_number(v)?,
    };
    Ok(
        match if i >= 0.0 {
            u16_at(&s, i as usize)
        } else {
            None
        } {
            Some(c) => e.str_val(c),
            None => e.str_val(""),
        },
    )
}

pub(crate) fn string_char_code_at(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let s = this_str(e, this)?;
    let i = match arg(args, 0) {
        Value::Undefined => 0.0,
        v => e.to_number(v)?,
    };
    Ok(Value::Num(
        if i >= 0.0 {
            s.encode_utf16().nth(i as usize).map(f64::from)
        } else {
            None
        }
        .unwrap_or(f64::NAN),
    ))
}

pub(crate) fn str_index_of(
    e: &mut Engine,
    this: Value,
    args: &[Value],
    last: bool,
) -> JsResult<Value> {
    let s = this_str(e, this)?;
    let needle = e.to_js_string(arg(args, 0))?;
    // Recherche sur la vue code units (spec — `from` et le
    // résultat sont en units).
    let (h, n) = (u16_view(&s), u16_view(&needle));
    let from = match arg(args, 1) {
        // `lastIndexOf` sans position → depuis la fin (spec).
        Value::Undefined if last => usize::MAX,
        Value::Undefined => 0usize,
        v => (e.to_number(v)?.max(0.0)) as usize,
    };
    let pos = if n.len() > h.len() {
        None
    } else if last {
        // `lastIndexOf` : dernière occurrence dont le début est ≤
        // `from` (spec `StringIndexOf` arrière).
        let hi = h.len() - n.len();
        let from = from.min(hi);
        (0..=from).rev().find(|&i| h[i..i + n.len()] == n[..])
    } else {
        (from.min(h.len())..=h.len() - n.len()).find(|&i| h[i..i + n.len()] == n[..])
    };
    Ok(Value::Num(pos.map_or(-1.0, |p| p as f64)))
}

pub(crate) fn string_index_of(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    str_index_of(e, this, args, false)
}

pub(crate) fn string_last_index_of(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    str_index_of(e, this, args, true)
}

pub(crate) fn str_slice(e: &mut Engine, s: &str, start: i64, end: Option<i64>) -> Value {
    let u = u16_view(s);
    let len = u.len() as i64;
    let a = norm_idx(start, len);
    let b = end.map_or(u.len(), |e2| norm_idx(e2, len));
    if a >= b {
        return e.str_val("");
    }
    e.str_val(u16_to_string(&u[a..b]))
}

pub(crate) fn string_slice(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let s = this_str(e, this)?;
    let start = match arg(args, 0) {
        Value::Undefined => 0,
        v => e.to_number(v)? as i64,
    };
    let end = match arg(args, 1) {
        Value::Undefined => None,
        v => Some(e.to_number(v)? as i64),
    };
    Ok(str_slice(e, &s, start, end))
}

pub(crate) fn string_substring(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let s = this_str(e, this)?;
    let u = u16_view(&s);
    let len = u.len() as i64;
    let a = match arg(args, 0) {
        Value::Undefined => 0,
        v => (e.to_number(v)? as i64).clamp(0, len),
    };
    let b = match arg(args, 1) {
        Value::Undefined => len,
        v => (e.to_number(v)? as i64).clamp(0, len),
    };
    let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
    Ok(e.str_val(u16_to_string(&u[lo as usize..hi as usize])))
}

pub(crate) fn string_substr(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let s = this_str(e, this)?;
    let u = u16_view(&s);
    let len = u.len() as i64;
    let mut start = match arg(args, 0) {
        Value::Undefined => 0,
        v => e.to_number(v)? as i64,
    };
    if start < 0 {
        start = (len + start).max(0);
    }
    let count = match arg(args, 1) {
        Value::Undefined => len - start,
        v => (e.to_number(v)? as i64).max(0),
    };
    let end = (start + count).min(len);
    if start >= end {
        return Ok(e.str_val(""));
    }
    Ok(e.str_val(u16_to_string(&u[start as usize..end as usize])))
}

pub(crate) fn string_to_lower(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let s = this_str(e, this)?;
    Ok(e.str_val(s.to_lowercase()))
}

pub(crate) fn string_to_upper(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let s = this_str(e, this)?;
    Ok(e.str_val(s.to_uppercase()))
}

pub(crate) fn string_trim(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let s = this_str(e, this)?;
    Ok(e.str_val(s.trim().to_string()))
}

pub(crate) fn string_split(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let s = this_str(e, this)?;
    // `sep[Symbol.split](this, limit)` (spec `GetMethod`).
    if let Some(v) = sym_method(
        e,
        arg(args, 0),
        crate::interp::SYM_SPLIT,
        &[this, arg(args, 1)],
    )? {
        return Ok(v);
    }
    let limit = match arg(args, 1) {
        Value::Undefined => usize::MAX,
        v => e.to_number(v)? as usize,
    };
    let mut parts: Vec<String> = match arg(args, 0) {
        Value::Undefined => vec![s],
        v => {
            if let Some(re) = re_of(e, v) {
                // Regex split inserts capture groups between pieces.
                if limit == 0 {
                    return Ok(arr_of(e, Vec::new()));
                }
                re.split(&s)
                    .into_iter()
                    .take(limit)
                    .map(str::to_string)
                    .collect()
            } else {
                let sep = e.to_js_string(v)?;
                if sep.is_empty() {
                    // `split('')` → code units (spec) ; surrogate
                    // isolé → U+FFFD (déviation documentée).
                    u16_view(&s).iter().map(|&w| u16_to_string(&[w])).collect()
                } else {
                    s.split(&sep).map(str::to_string).collect()
                }
            }
        }
    };
    parts.truncate(limit);
    let vals: Vec<Value> = parts.iter().map(|p| e.str_val(p.clone())).collect();
    Ok(arr_of(e, vals))
}

pub(crate) fn string_replace(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    replace_impl(e, this, args, false)
}

pub(crate) fn string_replace_all(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    replace_impl(e, this, args, true)
}

pub(crate) fn replace_impl(
    e: &mut Engine,
    this: Value,
    args: &[Value],
    all: bool,
) -> JsResult<Value> {
    let s = this_str(e, this)?;
    let pat = arg(args, 0);
    let rep = arg(args, 1);
    // Spec `replaceAll` : un pattern RegExp sans `g` est rejeté
    // AVANT le dispatch `@@replace` (`IsRegExp` + flags).
    if all {
        if let Some(re) = re_of(e, pat) {
            if !re.global {
                return Err(e.type_error("replaceAll requires a global regex"));
            }
        }
    }
    // `pat[Symbol.replace](this, rep)` (spec `GetMethod`).
    if let Some(v) = sym_method(e, pat, crate::interp::SYM_REPLACE, &[this, rep])? {
        return Ok(v);
    }
    if let Some(re) = re_of(e, pat) {
        return re_replace_impl(e, &re, &s, rep);
    }
    // String pattern — first occurrence (or all for replaceAll).
    let pat = e.to_js_string(pat)?;
    if pat.is_empty() {
        return Ok(e.str_val(s));
    }
    let positions: Vec<usize> = if all {
        s.match_indices(&pat).map(|(i, _)| i).collect()
    } else {
        s.find(&pat).into_iter().collect()
    };
    if positions.is_empty() {
        return Ok(e.str_val(s));
    }
    let mut out = String::with_capacity(s.len());
    let mut cur = 0usize;
    for pos in positions {
        out.push_str(&s[cur..pos]);
        out.push_str(&expand_rep(
            e,
            rep,
            &[],
            &[],
            &s[..pos],
            &s[pos + pat.len()..],
            &pat,
        )?);
        cur = pos + pat.len();
    }
    out.push_str(&s[cur..]);
    Ok(e.str_val(out))
}

/// Coerce a `replace`/`match`/`split`/`search` argument to a
/// `JsRegExp` — `RegExp` objects pass through, strings compile
/// (`str.match("x+")` ≡ `new RegExp("x+")`).
pub(crate) fn re_of(e: &mut Engine, v: Value) -> Option<std::rc::Rc<crate::regexp::JsRegExp>> {
    let Value::Obj(id) = v else { return None };
    match e.heap.obj(id).map(|o| &o.kind) {
        Some(ObjKind::RegExp(r)) => Some(r.clone()),
        _ => None,
    }
}

/// `RegExp` object + the value it must coerce to one.
pub(crate) fn as_regexp(
    e: &mut Engine,
    v: Value,
) -> JsResult<(std::rc::Rc<crate::regexp::JsRegExp>, ObjId)> {
    if let Some(r) = re_of(e, v) {
        let Value::Obj(id) = v else {
            return Err(e.type_error("not a RegExp"));
        };
        return Ok((r, id));
    }
    let pat = e.to_js_string(v)?;
    let obj = e.make_regexp(&pat, "")?;
    let Value::Obj(id) = obj else {
        return Err(e.type_error("not a RegExp"));
    };
    match e.heap.obj(id).map(|o| &o.kind) {
        Some(ObjKind::RegExp(r)) => Ok((r.clone(), id)),
        _ => Err(e.type_error("not a RegExp")),
    }
}

/// Read `rx.lastIndex` (char units).
pub(crate) fn re_last_index(e: &mut Engine, id: ObjId) -> JsResult<usize> {
    let v = e.get_prop(id, "lastIndex").unwrap_or(Value::Num(0.0));
    let n = e.to_number(v)?;
    Ok(if n.is_sign_negative() || n.is_nan() {
        0
    } else {
        n as usize
    })
}

/// Expand `$$`, `$&`, `` $` ``, `$'`, `$n`/`$nn`, `$<name>` in a
/// string replacement. Function replacements are handled by caller.
pub(crate) fn expand_rep(
    e: &mut Engine,
    rep: Value,
    groups: &[Option<String>],
    named: &[(String, String)],
    before: &str,
    after: &str,
    matched: &str,
) -> JsResult<String> {
    let rep = e.to_js_string(rep)?;
    let rc = chars_of(&rep);
    let mut out = String::with_capacity(rep.len());
    let mut i = 0;
    while i < rc.len() {
        if rc[i] == '$' && i + 1 < rc.len() {
            match rc[i + 1] {
                '$' => {
                    out.push('$');
                    i += 2;
                    continue;
                }
                '&' => {
                    out.push_str(matched);
                    i += 2;
                    continue;
                }
                '`' => {
                    out.push_str(before);
                    i += 2;
                    continue;
                }
                '\'' => {
                    out.push_str(after);
                    i += 2;
                    continue;
                }
                '<' => {
                    if let Some(end) = rc[i + 2..].iter().position(|&c| c == '>') {
                        let name: String = rc[i + 2..i + 2 + end].iter().collect();
                        if let Some((_, v)) = named.iter().find(|(k, _)| *k == name) {
                            out.push_str(v);
                        }
                        i += end + 3;
                        continue;
                    }
                }
                c if c.is_ascii_digit() => {
                    let d1 = rc[i + 1].to_digit(10).unwrap_or(0) as usize;
                    let d2 = rc
                        .get(i + 2)
                        .and_then(|c| c.to_digit(10))
                        .map(|d| d as usize);
                    // `$nn` counts only if group nn exists.
                    let (n, step) = match d2 {
                        Some(d2) if d1 * 10 + d2 > 0 && d1 * 10 + d2 < groups.len() => {
                            (d1 * 10 + d2, 3)
                        }
                        _ => (d1, 2),
                    };
                    if n > 0 && n < groups.len() {
                        if let Some(g) = &groups[n] {
                            out.push_str(g);
                        }
                        i += step;
                        continue;
                    }
                }
                _ => {}
            }
        }
        out.push(rc[i]);
        i += 1;
    }
    Ok(out)
}

/// Apply `matches` against `s` with string-or-function replacement.
pub(crate) fn splice_matches(
    e: &mut Engine,
    s: &str,
    matches: &[crate::regexp::ExecMatch],
    rep: Value,
) -> JsResult<Value> {
    let use_fn = e.is_callable(rep);
    if std::env::var("DBG_DRIVE").is_ok() {
        eprintln!(
            "[splice] matches={} use_fn={} s.len={}",
            matches.len(),
            use_fn,
            s.len()
        );
    }
    let mut out = String::with_capacity(s.len() + matches.len() * 8);
    let mut cur = 0usize; // byte offset
    for (mi, m) in matches.iter().enumerate() {
        if std::env::var("DBG_DRIVE").is_ok() && mi % 64 == 0 {
            eprintln!(
                "[splice] i={mi}/{} idx={} end={}",
                matches.len(),
                m.index,
                m.end
            );
        }
        out.push_str(&s[cur..m.start_b]);
        if use_fn {
            // f(match, g1…gn, index, string [, groups])
            let mut call_args = Vec::with_capacity(m.groups.len() + 3);
            call_args.push(e.str_val(m.text.clone()));
            for g in &m.groups[1..] {
                call_args.push(match g {
                    Some(t) => e.str_val(t.clone()),
                    None => Value::Undefined,
                });
            }
            call_args.push(Value::Num(m.index as f64));
            call_args.push(e.str_val(s.to_string()));
            if !m.named.is_empty() {
                let gobj = e.new_object();
                for (k, v) in &m.named {
                    let vv = e.str_val(v.clone());
                    e.set_own(gobj, k, vv, true);
                }
                call_args.push(Value::Obj(gobj));
            }
            let r = e.call_value(rep, Value::Undefined, &call_args)?;
            out.push_str(&e.to_js_string(r)?);
        } else {
            out.push_str(&expand_rep(
                e,
                rep,
                &m.groups,
                &m.named,
                &s[..m.start_b],
                &s[m.end_b..],
                &m.text,
            )?);
        }
        cur = m.end_b;
    }
    out.push_str(&s[cur..]);
    Ok(e.str_val(out))
}

/// `exec`/`test` result array: `[m0, m1…].index/.input/.groups`.
pub(crate) fn match_result(
    e: &mut Engine,
    m: &crate::regexp::ExecMatch,
    re: &crate::regexp::JsRegExp,
    input: &str,
) -> Value {
    let elems: Vec<Value> = m
        .groups
        .iter()
        .map(|g| match g {
            Some(t) => e.str_val(t.clone()),
            None => Value::Undefined,
        })
        .collect();
    let Value::Obj(arr) = e.new_array(elems) else {
        return Value::Null;
    };
    e.set_own(arr, "index", Value::Num(m.index as f64), false);
    let input_v = e.str_val(input.to_string());
    e.set_own(arr, "input", input_v, false);
    let groups_v = if m.named.is_empty() {
        Value::Undefined
    } else {
        let gobj = e.new_object();
        for (k, v) in &m.named {
            let vv = e.str_val(v.clone());
            e.set_own(gobj, k, vv, true);
        }
        Value::Obj(gobj)
    };
    e.set_own(arr, "groups", groups_v, false);
    // `d` flag : `indices` = [[start, end] | undefined] par groupe
    // (+ `indices.groups` miroir des groupes nommés).
    if re.has_indices {
        let idx: Vec<Value> = m
            .group_indices
            .iter()
            .map(|r| match r {
                Some((s, t)) => e.new_array(vec![Value::Num(*s as f64), Value::Num(*t as f64)]),
                None => Value::Undefined,
            })
            .collect();
        let Value::Obj(ia) = e.new_array(idx) else {
            return Value::Obj(arr);
        };
        if !m.named_indices.is_empty() {
            let gobj = e.new_object();
            for (k, r) in &m.named_indices {
                let v = match r {
                    Some((s, t)) => e.new_array(vec![Value::Num(*s as f64), Value::Num(*t as f64)]),
                    None => Value::Undefined,
                };
                e.set_own(gobj, k, v, true);
            }
            e.set_own(ia, "groups", Value::Obj(gobj), false);
        }
        e.set_own(arr, "indices", Value::Obj(ia), false);
    }
    Value::Obj(arr)
}

pub(crate) fn regexp_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    // `new RegExp(re)` clones the pattern; `RegExp(str, flags)`.
    let (pat, flags) = match arg(args, 0) {
        v @ Value::Obj(_) => {
            if let Some(r) = re_of(e, v) {
                let flags = match arg(args, 1) {
                    Value::Undefined => r.flag_string(),
                    f => e.to_js_string(f)?,
                };
                (r.source.clone(), flags)
            } else {
                let p = e.to_js_string(v)?;
                (p, String::new())
            }
        }
        Value::Undefined => (String::new(), String::new()),
        v => {
            let p = e.to_js_string(v)?;
            (p, String::new())
        }
    };
    let flags = if flags.is_empty() {
        match arg(args, 1) {
            Value::Undefined => flags,
            f => e.to_js_string(f)?,
        }
    } else {
        flags
    };
    e.make_regexp(&pat, &flags)
}

pub(crate) fn regexp_exec(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let (re, id) = as_regexp(e, this)?;
    let s = e.to_js_string(arg(args, 0))?;
    let use_last = re.global || re.sticky;
    let last = if use_last { re_last_index(e, id)? } else { 0 };
    if let Some((m, new_last)) = re.exec_from(&s, last) {
        if use_last {
            let _ = e.set_prop(id, "lastIndex", Value::Num(new_last as f64));
        }
        e.note_re(&s, Some(&m));
        Ok(match_result(e, &m, &re, &s))
    } else {
        if use_last {
            let _ = e.set_prop(id, "lastIndex", Value::Num(0.0));
        }
        e.note_re(&s, None);
        Ok(Value::Null)
    }
}

pub(crate) fn regexp_test(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let r = regexp_exec(e, this, args)?;
    Ok(Value::Bool(!matches!(r, Value::Null)))
}

/// `RegExp.escape(s)` — ES2025 : échappe `s` pour insertion
/// littérale dans un pattern. `SyntaxCharacter`s/`/` → `\c` ;
/// en tête, tout autre caractère ASCII non sûr (ponctuation,
/// chiffres, lettres, espace) → `\xNN` ou `\uNNNN`.
pub(crate) fn regexp_escape(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    const SYNTAX: &str = "^$\\.*+?()[]{}|/";
    let src = e.to_js_string(arg(args, 0))?;
    let mut out = String::with_capacity(src.len() * 2);
    for (i, c) in src.chars().enumerate() {
        let esc_hex = i == 0 && (c.is_ascii_alphanumeric() || c.is_ascii_punctuation() || c == ' ');
        if SYNTAX.contains(c) {
            out.push('\\');
            out.push(c);
        } else if esc_hex {
            let cp = c as u32;
            if cp <= 0xff {
                out.push_str(&format!("\\x{cp:02x}"));
            } else {
                out.push_str(&format!("\\u{cp:04x}"));
            }
        } else {
            out.push(c);
        }
    }
    Ok(e.str_val(out))
}

/// `s.isWellFormed()` — nos `char` sont toujours des scalaires
/// valides : `true` (les surrogates orphelins dégradent en
/// U+FFFD au lexing).
pub(crate) fn string_is_well_formed(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    this_str(e, this)?;
    Ok(Value::Bool(true))
}

/// `s.toWellFormed()` — identité (même raison).
pub(crate) fn string_to_well_formed(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let s = this_str(e, this)?;
    Ok(e.str_val(s))
}

pub(crate) fn regexp_to_string(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let (re, _id) = as_regexp(e, this)?;
    let src = if re.source.is_empty() {
        "(?:)"
    } else {
        re.source.as_str()
    };
    Ok(e.str_val(format!("/{src}/{}", re.flag_string())))
}

pub(crate) fn string_match(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let s = this_str(e, this)?;
    // `rx[Symbol.match](this)` (spec `GetMethod`).
    if let Some(v) = sym_method(e, arg(args, 0), crate::interp::SYM_MATCH, &[this])? {
        return Ok(v);
    }
    let (re, id) = as_regexp(e, arg(args, 0))?;
    re_match_exec(e, &re, id, &s)
}

pub(crate) fn string_search(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let s = this_str(e, this)?;
    // `rx[Symbol.search](this)` (spec `GetMethod`).
    if let Some(v) = sym_method(e, arg(args, 0), crate::interp::SYM_SEARCH, &[this])? {
        return Ok(v);
    }
    let (re, _id) = as_regexp(e, arg(args, 0))?;
    match re.exec(&s, 0) {
        Some(m) => Ok(Value::Num(m.index as f64)),
        None => Ok(Value::Num(-1.0)),
    }
}

pub(crate) fn string_concat(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let mut s = this_str(e, this)?;
    for a in args {
        s.push_str(&e.to_js_string(*a)?);
    }
    Ok(e.str_val(s))
}

pub(crate) fn string_from_char_code(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    // Code units → string : une paire surrogate forme l'astral
    // (spec), un surrogate isolé devient U+FFFD (déviation
    // documentée — UTF-8 natif).
    let mut units = Vec::with_capacity(args.len());
    for a in args {
        units.push(e.to_number(*a)? as u16);
    }
    Ok(e.str_val(u16_to_string(&units)))
}
