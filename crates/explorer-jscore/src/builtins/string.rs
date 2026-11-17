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
    let i = match arg(args, 0) {
        Value::Undefined => 0.0,
        v => e.to_number(v)?,
    };
    // §B-59 : lecture en place — `charAt` en boucle clonait la
    // chaîne entière par appel via `this_str` (churn commit).
    if let Value::Str(id) = this {
        let w = if i >= 0.0 {
            id.u16_at(i as usize)
        } else {
            None
        };
        return Ok(match w {
            Some(w) => e.str_val_units(vec![w]),
            None => e.str_val(""),
        });
    }
    let s = this_str(e, this)?;
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
    let i = match arg(args, 0) {
        Value::Undefined => 0.0,
        v => e.to_number(v)?,
    };
    // §B-59 : lecture en place (voir `string_char_at`).
    if let Value::Str(id) = this {
        return Ok(Value::Num(
            if i >= 0.0 {
                id.u16_at(i as usize).map(u32::from)
            } else {
                None
            }
            .map_or(f64::NAN, f64::from),
        ));
    }
    let s = this_str(e, this)?;
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
    // `CheckObjectCoercible(this)` avant toute conversion (spec).
    if matches!(this, Value::Undefined | Value::Null) {
        return Err(e.type_error("String method called on null/undefined"));
    }
    let needle = e.to_js_jstr(arg(args, 0))?;
    let from = match arg(args, 1) {
        // `lastIndexOf` sans position → depuis la fin (spec).
        Value::Undefined if last => usize::MAX,
        Value::Undefined => 0usize,
        v => (e.to_number(v)?.max(0.0)) as usize,
    };
    // §B-59 : recherche **en place** sur la cellule — le chemin
    // `this_str` clonait toute la chaîne par appel (décodeur
    // Confiant `indexOf` × N = ratchet de commit géant).
    if let Value::Str(id) = this {
        // Match **exact** en units dès qu'un côté porte des
        // surrogates — `U+FFFD` n'égale pas un surrogate (spec).
        if id.has_units() || needle.has_units() {
            let n = needle.units_view();
            return Ok(Value::Num(id.index_of_units(&n, from, last) as f64));
        }
        return Ok(Value::Num(id.index_of(needle.as_str(), from, last) as f64));
    }
    if needle.has_units() {
        // Haystack sans units (déjà ToStringé) : aucun surrogate
        // réel → un needle à surrogate ne peut pas matcher.
        return Ok(Value::Num(-1.0));
    }
    let needle = needle.as_str();
    let s = this_str(e, this)?;
    let byte_from = JsStr::u16_pos_to_byte(&s, from);
    let pos = if needle.len() > s.len() {
        None
    } else if last {
        let limit = byte_from.min(s.len() - needle.len());
        s.match_indices(&needle)
            .take_while(|(p, _)| *p <= limit)
            .last()
            .map(|(p, _)| p)
    } else if byte_from > s.len() - needle.len() {
        None
    } else {
        s[byte_from..].find(needle).map(|p| p + byte_from)
    };
    Ok(Value::Num(
        pos.map_or(-1.0, |b| s[..b].encode_utf16().count() as f64),
    ))
}

pub(crate) fn string_index_of(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    str_index_of(e, this, args, false)
}

pub(crate) fn string_last_index_of(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    str_index_of(e, this, args, true)
}

pub(crate) fn str_slice(e: &mut Engine, s: &str, start: i64, end: Option<i64>) -> Value {
    // Tranche en units → offsets octets : zéro matérialisation de
    // la vue u16 (§B-59) — borne astrale → char entier (déviation
    // documentée `u16_to_string`).
    let len = u16_len(s) as i64;
    let a = norm_idx(start, len);
    let b = end.map_or(len as usize, |e2| norm_idx(e2, len));
    if a >= b {
        return e.str_val("");
    }
    let (ba, bb) = (JsStr::u16_pos_to_byte(s, a), JsStr::u16_pos_to_byte(s, b));
    e.str_val(s[ba..bb].to_string())
}

pub(crate) fn string_slice(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let start = match arg(args, 0) {
        Value::Undefined => 0,
        v => e.to_number(v)? as i64,
    };
    let end = match arg(args, 1) {
        Value::Undefined => None,
        v => Some(e.to_number(v)? as i64),
    };
    // §B-59 : tranche lue en place — `this_str` clonait la
    // chaîne entière avant de n'en garder qu'un segment.
    if let Value::Str(id) = this {
        let len = id.u16_len() as i64;
        let a = norm_idx(start, len);
        let b = end.map_or(len as usize, |e2| norm_idx(e2, len));
        if a >= b {
            return Ok(e.str_val(""));
        }
        return Ok(Value::Str(id.slice_js_u16(a, b, &e.str_acct)));
    }
    let s = this_str(e, this)?;
    Ok(str_slice(e, &s, start, end))
}

pub(crate) fn string_substring(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    // §B-59 : tranche lue en place (voir `string_slice`).
    if let Value::Str(id) = this {
        let len = id.u16_len() as i64;
        let a = match arg(args, 0) {
            Value::Undefined => 0,
            v => (e.to_number(v)? as i64).clamp(0, len),
        };
        let b = match arg(args, 1) {
            Value::Undefined => len,
            v => (e.to_number(v)? as i64).clamp(0, len),
        };
        let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
        return Ok(Value::Str(id.slice_js_u16(
            lo as usize,
            hi as usize,
            &e.str_acct,
        )));
    }
    let s = this_str(e, this)?;
    let len = u16_len(&s) as i64;
    let a = match arg(args, 0) {
        Value::Undefined => 0,
        v => (e.to_number(v)? as i64).clamp(0, len),
    };
    let b = match arg(args, 1) {
        Value::Undefined => len,
        v => (e.to_number(v)? as i64).clamp(0, len),
    };
    let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
    let (ba, bb) = (
        JsStr::u16_pos_to_byte(&s, lo as usize),
        JsStr::u16_pos_to_byte(&s, hi as usize),
    );
    Ok(e.str_val(s[ba..bb].to_string()))
}

pub(crate) fn string_substr(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    // §B-59 : tranche lue en place (voir `string_slice`).
    if let Value::Str(id) = this {
        let len = id.u16_len() as i64;
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
        return Ok(Value::Str(id.slice_js_u16(
            start as usize,
            end as usize,
            &e.str_acct,
        )));
    }
    let s = this_str(e, this)?;
    let len = u16_len(&s) as i64;
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
    let (ba, bb) = (
        JsStr::u16_pos_to_byte(&s, start as usize),
        JsStr::u16_pos_to_byte(&s, end as usize),
    );
    Ok(e.str_val(s[ba..bb].to_string()))
}

/// Mapping de casse **par code point** (spec `toUpper/LowerCase`
/// = Unicode Default Case Conversion, sans contexte — le mapping
/// complet `char::to_upper/lower` de Rust, expansions `ß→SS`
/// incluses). Un surrogate isolé n'est pas un scalaire : il
/// traverse tel quel (spec — pas de mapping, jamais U+FFFD).
fn case_map(e: &mut Engine, s: &JsStr, upper: bool) -> Value {
    let mut out = Vec::with_capacity(s.u16_len());
    let mut buf = [0u16; 2];
    for cp in s.iter_code_points() {
        let mapped = if cp.len() == 1 && (0xD800..0xE000).contains(&cp[0]) {
            None
        } else {
            String::from_utf16(&cp).ok().and_then(|t| t.chars().next())
        };
        match mapped {
            Some(c) => {
                let m: Vec<char> = if upper {
                    c.to_uppercase().collect()
                } else {
                    c.to_lowercase().collect()
                };
                for mc in m {
                    out.extend_from_slice(mc.encode_utf16(&mut buf));
                }
            }
            None => out.extend_from_slice(&cp),
        }
    }
    e.str_val_units(out)
}

pub(crate) fn string_to_lower(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let s = this_jstr(e, this)?;
    Ok(case_map(e, &s, false))
}

pub(crate) fn string_to_upper(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let s = this_jstr(e, this)?;
    Ok(case_map(e, &s, true))
}

/// Whitespace JS au niveau char — `White_Space` Unicode +
/// `U+FEFF` (ZWNBSP, Cf hors `is_whitespace` Rust) — pendant
/// de `unit_is_ws` pour les fast-paths sans `units`.
pub(crate) fn js_ws(c: char) -> bool {
    c.is_whitespace() || c == '\u{feff}'
}

pub(crate) fn string_trim(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    if let Value::Str(id) = this {
        // Fast-path : sans `units`, trim par chars (whitespace
        // JS — FEFF inclus, spec `TrimString`).
        return Ok(if id.has_units() {
            Value::Str(id.trim_js(true, true, &e.str_acct))
        } else {
            e.str_val(id.as_str().trim_matches(js_ws).to_string())
        });
    }
    let s = this_str(e, this)?;
    Ok(e.str_val(s.trim_matches(js_ws).to_string()))
}

pub(crate) fn string_split(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let s = this_str(e, this.clone())?;
    // `this` est consommé par `sym_method` — `Rc` cloné à bon
    // marché pour le split `''` en units (surrogates préservés).
    let this_id = if let Value::Str(id) = &this {
        Some(id.clone())
    } else {
        None
    };
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
            if let Some(re) = re_of(e, v.clone()) {
                // Regex split inserts capture groups between pieces.
                if limit == 0 {
                    return Ok(arr_of(e, Vec::new()));
                }
                // Domaine sentinelle si le pattern/le sujet porte
                // des surrogates — pièces décodées en units.
                let sjs = this_id
                    .clone()
                    .unwrap_or_else(|| JsStr::new(s.clone(), &e.str_acct));
                let (hay, dec) = re.enc_hay(&sjs);
                let vals: Vec<Value> = re
                    .split(&hay)
                    .into_iter()
                    .take(limit)
                    .map(|p| re_str_val(e, p, dec))
                    .collect();
                return Ok(arr_of(e, vals));
            }
            let sep = e.to_js_string(v)?;
            // Garde §B-59 : `split` matérialise ~160 o par
            // partie (String + cellule `str_val` + intern)
            // — `''` sur 20 M unités ≈ 3 Go de cellules
            // sans tick (signature `rss_guard[commit]`).
            let worst = s.len().min(limit);
            e.alloc_guard(worst.saturating_mul(160))?;
            if sep.is_empty() {
                // `split('')` → un item par code unit (spec) ;
                // la vue `units` préserve les surrogates
                // isolés — `Vec<String>` ne pourrait pas les
                // porter, on produit les `Value` directement.
                let units = match &this_id {
                    Some(id) => id.units_view(),
                    _ => u16_view(e, &s)?,
                };
                let vals: Vec<Value> = units
                    .iter()
                    .take(limit)
                    .map(|&w| e.str_val_units(vec![w]))
                    .collect();
                return Ok(arr_of(e, vals));
            }
            s.split(&sep).take(limit).map(str::to_string).collect()
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
    // `Rc` cloné pour le chemin units (match exact surrogate).
    let this_id = if let Value::Str(id) = &this {
        Some(id.clone())
    } else {
        None
    };
    let s = this_str(e, this.clone())?;
    let pat = arg(args, 0);
    let rep = arg(args, 1);
    // Spec `replaceAll` : un pattern RegExp sans `g` est rejeté
    // AVANT le dispatch `@@replace` (`IsRegExp` + flags).
    if all {
        if let Some(re) = re_of(e, pat.clone()) {
            if !re.global {
                return Err(e.type_error("replaceAll requires a global regex"));
            }
        }
    }
    // `pat[Symbol.replace](this, rep)` (spec `GetMethod`).
    if let Some(v) = sym_method(
        e,
        pat.clone(),
        crate::interp::SYM_REPLACE,
        &[this, rep.clone()],
    )? {
        return Ok(v);
    }
    if let Some(re) = re_of(e, pat.clone()) {
        let sjs = this_id
            .clone()
            .unwrap_or_else(|| JsStr::new(s.clone(), &e.str_acct));
        return re_replace_impl(e, &re, &sjs, rep);
    }
    // String pattern — first occurrence (or all for replaceAll).
    let pat = e.to_js_jstr(pat)?;
    let unchanged = |e: &mut Engine| match &this_id {
        Some(id) => Value::Str(id.clone()),
        None => e.str_val(s.clone()),
    };
    if pat.is_empty() {
        return Ok(unchanged(e));
    }
    // Match **exact** en units dès qu'un côté porte des
    // surrogates — `U+FFFD` ne remplace pas un surrogate (spec).
    if this_id.as_ref().is_some_and(JsStr::has_units) || pat.has_units() {
        let hu = match &this_id {
            Some(id) => id.units_view(),
            None => u16_view(e, &s)?,
        };
        let pu = pat.units_view();
        let mut positions = Vec::new();
        if pu.len() <= hu.len() {
            let max = hu.len() - pu.len();
            let mut i = 0;
            while i <= max {
                if hu[i..i + pu.len()] == pu[..] {
                    positions.push(i);
                    if !all {
                        break;
                    }
                    i += pu.len();
                } else {
                    i += 1;
                }
            }
        }
        if positions.is_empty() {
            return Ok(unchanged(e));
        }
        let mut out = Vec::with_capacity(hu.len());
        let mut cur = 0usize;
        for pos in positions {
            e.alloc_guard(out.len().saturating_mul(2).max(16))?;
            out.extend_from_slice(&hu[cur..pos]);
            out.extend_from_slice(&expand_rep_units(
                e,
                rep.clone(),
                &pu,
                &hu[..pos],
                &hu[pos + pu.len()..],
            )?);
            cur = pos + pu.len();
        }
        out.extend_from_slice(&hu[cur..]);
        return Ok(e.str_val_units(out));
    }
    let pat = pat.as_str();
    let positions: Vec<usize> = if all {
        let n = s.matches(&pat).count();
        e.alloc_guard(n.saturating_mul(std::mem::size_of::<usize>()))?;
        s.match_indices(&pat).map(|(i, _)| i).collect()
    } else {
        s.find(pat).into_iter().collect()
    };
    if positions.is_empty() {
        return Ok(e.str_val(s));
    }
    // §B-59 : la sortie peut dépasser l'entrée × le remplacement
    // — vérifier la croissance pendant la boucle (RangeError,
    // pas abort de l'allocateur).
    let mut out = String::with_capacity(s.len());
    let mut cur = 0usize;
    for pos in positions {
        e.alloc_guard(out.len())?;
        out.push_str(&s[cur..pos]);
        out.push_str(&expand_rep(
            e,
            rep.clone(),
            &[],
            &[],
            &s[..pos],
            &s[pos + pat.len()..],
            pat,
        )?);
        cur = pos + pat.len();
    }
    out.push_str(&s[cur..]);
    Ok(e.str_val(out))
}

/// `expand_rep` en **code units** (chemin pattern String avec
/// surrogates — pas de groupes `()` : pattern litéral). `$$`,
/// `$&`, `` $` ``, `$'` reconnus ; le texte de remplacement peut
/// lui-même porter des surrogates.
fn expand_rep_units(
    e: &mut Engine,
    rep: Value,
    matched: &[u16],
    before: &[u16],
    after: &[u16],
) -> JsResult<Vec<u16>> {
    let rep = e.to_js_jstr(rep)?;
    let ru = rep.units_view();
    e.alloc_guard(ru.len().saturating_mul(2))?;
    let mut out = Vec::with_capacity(ru.len() + matched.len());
    let mut i = 0;
    while i < ru.len() {
        if ru[i] == 0x24 && i + 1 < ru.len() {
            match ru[i + 1] {
                0x24 => {
                    out.push(0x24);
                    i += 2;
                }
                0x26 => {
                    out.extend_from_slice(matched);
                    i += 2;
                }
                0x60 => {
                    out.extend_from_slice(before);
                    i += 2;
                }
                0x27 => {
                    out.extend_from_slice(after);
                    i += 2;
                }
                _ => {
                    out.push(ru[i]);
                    i += 1;
                }
            }
        } else {
            out.push(ru[i]);
            i += 1;
        }
    }
    Ok(out)
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
    if let Some(r) = re_of(e, v.clone()) {
        let Value::Obj(id) = v else {
            return Err(e.type_error("not a RegExp"));
        };
        return Ok((r, id));
    }
    let pat = e.to_js_jstr(v)?;
    let obj = e.make_regexp_jstr(&pat, "")?;
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
    named: &[(String, Option<String>)],
    before: &str,
    after: &str,
    matched: &str,
) -> JsResult<String> {
    let rep = e.to_js_string(rep)?;
    e.alloc_guard(rep.len().saturating_mul(4))?; // §B-59 Vec<char>
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
                        if let Some((_, Some(t))) = named.iter().find(|(k, _)| *k == name) {
                            out.push_str(t);
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
/// Valeur string depuis un texte du domaine regex — quand `dec`
/// le texte est encodé (`JsRegExp::enc_hay`) et les sentinelles
/// redeviennent des code units exacts (`re_decode`).
pub(crate) fn re_str_val(e: &mut Engine, s: &str, dec: bool) -> Value {
    if dec {
        e.str_val_units(crate::regexp::re_decode(s))
    } else {
        e.str_val(s.to_string())
    }
}

/// Splice `replace` : `hay` est le haystack **du domaine regex**
/// (encodé si `dec`) — les offsets `ExecMatch` s'y appliquent ;
/// `orig` est la chaîne JS d'origine pour l'argument `string`
/// d'un remplacement par fonction. Le résultat est décodé quand
/// `dec` (sentinelles → code units exacts).
pub(crate) fn splice_matches(
    e: &mut Engine,
    hay: &str,
    orig: &JsStr,
    matches: &[crate::regexp::ExecMatch],
    rep: Value,
    dec: bool,
) -> JsResult<Value> {
    let use_fn = e.is_callable(rep.clone());
    if std::env::var("DBG_DRIVE").is_ok() {
        eprintln!(
            "[splice] matches={} use_fn={} s.len={}",
            matches.len(),
            use_fn,
            hay.len()
        );
    }
    let mut out = String::with_capacity(hay.len() + matches.len() * 8);
    let mut cur = 0usize; // byte offset
    for (mi, m) in matches.iter().enumerate() {
        // §B-59 : borne de croissance de la sortie — une regex à
        // beaucoup de matches × remplacement long gonflerait `out`
        // jusqu'à l'abort d'alloc (même signature 2³⁵).
        e.alloc_guard(out.len())?;
        if std::env::var("DBG_DRIVE").is_ok() && mi % 64 == 0 {
            eprintln!(
                "[splice] i={mi}/{} idx={} end={}",
                matches.len(),
                m.index,
                m.end
            );
        }
        out.push_str(&hay[cur..m.start_b]);
        if use_fn {
            // f(match, g1…gn, index, string [, groups])
            let mut call_args = Vec::with_capacity(m.groups.len() + 3);
            call_args.push(re_str_val(e, &m.text, dec));
            for g in &m.groups[1..] {
                call_args.push(match g {
                    Some(t) => re_str_val(e, t, dec),
                    None => Value::Undefined,
                });
            }
            call_args.push(Value::Num(m.index as f64));
            call_args.push(Value::Str(orig.clone()));
            if !m.named.is_empty() {
                let gobj = e.new_object();
                for (k, v) in &m.named {
                    let vv = match v {
                        Some(t) => re_str_val(e, t, dec),
                        None => Value::Undefined,
                    };
                    e.set_own(gobj, k, vv, true);
                }
                call_args.push(Value::Obj(gobj));
            }
            let r = e.call_value(rep.clone(), Value::Undefined, &call_args)?;
            out.push_str(&e.to_js_string(r)?);
        } else {
            out.push_str(&expand_rep(
                e,
                rep.clone(),
                &m.groups,
                &m.named,
                &hay[..m.start_b],
                &hay[m.end_b..],
                &m.text,
            )?);
        }
        cur = m.end_b;
    }
    out.push_str(&hay[cur..]);
    Ok(if dec {
        e.str_val_units(crate::regexp::re_decode(&out))
    } else {
        e.str_val(out)
    })
}

/// `exec`/`test` result array: `[m0, m1…].index/.input/.groups`.
/// `dec` signale que les textes du match sont dans le domaine
/// regex encodé (`enc_hay`) — décodés via `re_decode` ; `input`
/// reste le `JsStr` **d'origine** (spec : chaîne matchée).
pub(crate) fn match_result(
    e: &mut Engine,
    m: &crate::regexp::ExecMatch,
    re: &crate::regexp::JsRegExp,
    input: &JsStr,
    dec: bool,
) -> Value {
    let elems: Vec<Value> = m
        .groups
        .iter()
        .map(|g| match g {
            Some(t) => re_str_val(e, t, dec),
            None => Value::Undefined,
        })
        .collect();
    let Value::Obj(arr) = e.new_array(elems) else {
        return Value::Null;
    };
    e.set_own(arr, "index", Value::Num(m.index as f64), false);
    e.set_own(arr, "input", Value::Str(input.clone()), false);
    let groups_v = if m.named.is_empty() {
        Value::Undefined
    } else {
        let gobj = e.new_object();
        for (k, v) in &m.named {
            let vv = match v {
                Some(t) => re_str_val(e, t, dec),
                None => Value::Undefined,
            };
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
            if let Some(r) = re_of(e, v.clone()) {
                let flags = match arg(args, 1) {
                    Value::Undefined => r.flag_string(),
                    f => e.to_js_string(f)?,
                };
                (JsStr::new(r.source.clone(), &e.str_acct), flags)
            } else {
                let p = e.to_js_jstr(v)?;
                (p, String::new())
            }
        }
        Value::Undefined => (JsStr::new("", &e.str_acct), String::new()),
        v => {
            let p = e.to_js_jstr(v)?;
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
    e.make_regexp_jstr(&pat, &flags)
}

/// `RegExp.prototype.compile(pattern, flags)` — Annex B.2.4
/// (dépréciée, sites legacy) : **réinitialise** `this` avec un
/// nouveau pattern + flags (`[[RegExpMatcher]]` requis — `this`
/// non-RegExp → `TypeError`) et `lastIndex` → 0.
pub(crate) fn regexp_compile(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let Value::Obj(id) = this else {
        return Err(e.type_error("compile requires a RegExp receiver"));
    };
    if !matches!(e.heap.obj(id).map(|o| &o.kind), Some(ObjKind::RegExp(_))) {
        return Err(e.type_error("compile requires a RegExp receiver"));
    }
    // Mêmes règles de résolution source/flags que le ctor.
    let (pat, flags) = match arg(args, 0) {
        v @ Value::Obj(_) => {
            if let Some(r) = re_of(e, v.clone()) {
                let flags = match arg(args, 1) {
                    Value::Undefined => r.flag_string(),
                    f => e.to_js_string(f)?,
                };
                (JsStr::new(r.source.clone(), &e.str_acct), flags)
            } else {
                (e.to_js_jstr(v)?, String::new())
            }
        }
        Value::Undefined => (JsStr::new("", &e.str_acct), String::new()),
        v => (e.to_js_jstr(v)?, String::new()),
    };
    let flags = if flags.is_empty() {
        match arg(args, 1) {
            Value::Undefined => flags,
            f => e.to_js_string(f)?,
        }
    } else {
        flags
    };
    // Pattern à `units` (surrogate isolé) → domaine sentinelle.
    let src = if pat.has_units() {
        crate::regexp::re_encode(&pat.units_view())
    } else {
        pat.as_str().to_string()
    };
    let re = match crate::regexp::JsRegExp::compile(&src, &flags) {
        Ok(r) => r,
        Err(msg) => return Err(e.syntax_error(&msg)),
    };
    if let Some(o) = e.heap.obj_mut(id) {
        o.kind = ObjKind::RegExp(std::rc::Rc::new(re));
    }
    let _ = e.set_prop(id, "lastIndex", Value::Num(0.0));
    Ok(Value::Undefined)
}

pub(crate) fn regexp_exec(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let (re, id) = as_regexp(e, this)?;
    let s = e.to_js_jstr(arg(args, 0))?;
    let use_last = re.global || re.sticky;
    let last = if use_last { re_last_index(e, id)? } else { 0 };
    let (hay, dec) = re.enc_hay(&s);
    if let Some((m, new_last)) = re.exec_from(&hay, last) {
        if use_last {
            let _ = e.set_prop(id, "lastIndex", Value::Num(new_last as f64));
        }
        e.note_re(&hay, Some(&m));
        Ok(match_result(e, &m, &re, &s, dec))
    } else {
        if use_last {
            let _ = e.set_prop(id, "lastIndex", Value::Num(0.0));
        }
        e.note_re(&hay, None);
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
        "(?:)".to_string()
    } else {
        re.escaped_source()
    };
    // `str_val_lit` : `re.source` est en domaine sentinelle —
    // restitue les units UTF-16 exacts (surrogate isolé
    // verbatim — test262 `literals/regexp` A2).
    Ok(e.str_val_lit(&format!("/{src}/{}", re.flag_string())))
}

// -- Accesseurs `RegExp.prototype` (spec) ---------------------------------
// Getters en proto : `TypeError` sur `this` non-RegExp ; les
// instances ne portent que `lastIndex` en prop propre.

fn re_get(e: &mut Engine, this: Value) -> JsResult<std::rc::Rc<crate::regexp::JsRegExp>> {
    match re_of(e, this) {
        Some(re) => Ok(re),
        None => Err(e.type_error("not a RegExp")),
    }
}

/// `get source` — `"(?:)"` quand le pattern est vide (spec).
/// `str_val_lit` décode le domaine sentinelle : `.source` rend
/// les units exacts du littéral (surrogate isolé verbatim —
/// `EscapeRegExpPattern` n'y touche pas — test262 A2).
pub(crate) fn re_source_get(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let re = re_get(e, this)?;
    Ok(e.str_val_lit(&if re.source.is_empty() {
        "(?:)".to_string()
    } else {
        re.escaped_source()
    }))
}
/// `get flags` — ordre canonique `dgimsuvy`.
pub(crate) fn re_flags_get(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let re = re_get(e, this)?;
    Ok(e.str_val(re.flag_string()))
}
pub(crate) fn re_global_get(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    Ok(Value::Bool(re_get(e, this)?.global))
}
pub(crate) fn re_ignore_case_get(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    Ok(Value::Bool(re_get(e, this)?.ignore_case))
}
pub(crate) fn re_multiline_get(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    Ok(Value::Bool(re_get(e, this)?.multiline))
}
pub(crate) fn re_dot_all_get(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    Ok(Value::Bool(re_get(e, this)?.dot_all))
}
pub(crate) fn re_sticky_get(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    Ok(Value::Bool(re_get(e, this)?.sticky))
}
pub(crate) fn re_unicode_get(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    Ok(Value::Bool(re_get(e, this)?.unicode))
}
pub(crate) fn re_unicode_sets_get(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    Ok(Value::Bool(re_get(e, this)?.unicode_sets))
}
pub(crate) fn re_has_indices_get(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    Ok(Value::Bool(re_get(e, this)?.has_indices))
}

pub(crate) fn string_match(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let s = this_jstr(e, this.clone())?;
    // `rx[Symbol.match](this)` (spec `GetMethod`).
    if let Some(v) = sym_method(e, arg(args, 0), crate::interp::SYM_MATCH, &[this])? {
        return Ok(v);
    }
    let (re, id) = as_regexp(e, arg(args, 0))?;
    re_match_exec(e, &re, id, &s)
}

pub(crate) fn string_search(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let s = this_jstr(e, this.clone())?;
    // `rx[Symbol.search](this)` (spec `GetMethod`).
    if let Some(v) = sym_method(e, arg(args, 0), crate::interp::SYM_SEARCH, &[this])? {
        return Ok(v);
    }
    let (re, _id) = as_regexp(e, arg(args, 0))?;
    let (hay, _dec) = re.enc_hay(&s);
    match re.exec(&hay, 0) {
        Some(m) => Ok(Value::Num(m.index as f64)),
        None => Ok(Value::Num(-1.0)),
    }
}

pub(crate) fn string_concat(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    // `JsStr` de bout en bout — les surrogates isolés traversent
    // `concat` comme `+` (`try_append_js`/`concat_js`, spec).
    let mut acc = this_jstr(e, this)?;
    for a in args {
        let b = e.to_js_jstr(a.clone())?;
        e.alloc_guard(acc.len().saturating_add(b.len()))?;
        if !acc.try_append_js(&b) {
            acc = crate::value::JsStr::concat_js(&acc, &b, &e.str_acct);
        }
    }
    Ok(Value::Str(acc))
}

pub(crate) fn string_from_char_code(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    // Code units → string : une paire surrogate forme l'astral
    // (spec), un surrogate isolé devient U+FFFD (déviation
    // documentée — UTF-8 natif).
    // Garde d'alloc : `String.fromCharCode.apply(null, énorme
    // tableau)` — `units` + la String de sortie grossissent avec
    // `args.len()`, sans tick entre les éléments (§B-59).
    e.alloc_guard(args.len().saturating_mul(4))?;
    let mut units = Vec::with_capacity(args.len());
    for a in args {
        units.push(e.to_number(a.clone())? as u16);
    }
    // Spec : code units directs — surrogate isolé préservé
    // (`str_val_units`), pas de perte en U+FFFD.
    Ok(e.str_val_units(units))
}
