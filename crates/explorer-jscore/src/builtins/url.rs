// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Builtins JS — section extraite de `builtins.rs` (JS-C40, pur
//! reformatage mécanique : sémantique et tests inchangés).

#[allow(clippy::wildcard_imports)]
use super::*;

// ------------------------------------------------------------------
// JS-C16 — reduceRight, locale-fallbacks, URL/URLSearchParams
// ------------------------------------------------------------------

pub(crate) fn array_reduce_right(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let id = this_array(e, this)?;
    let cb = arg(args, 0);
    if !e.is_callable(cb) {
        return Err(e.type_error("reduceRight needs a function"));
    }
    let elems = take_elems(e, id);
    let (mut acc, start) = match args.get(1) {
        Some(v) => (*v, elems.len() as i64 - 1),
        // Sans init, acc = dernier élément *présent* (spec).
        None => {
            if let Some(i) = elems.iter().rposition(|v| !v.is_hole()) {
                (elems[i], i as i64 - 1)
            } else {
                put_elems(e, id, elems);
                return Err(e.type_error("reduceRight of empty array"));
            }
        }
    };
    for i in (0..=start).rev() {
        let v = elems[i as usize];
        if v.is_hole() {
            continue;
        }
        acc = e.call_value(cb, Value::Undefined, &[acc, v, Value::Num(i as f64), this])?;
    }
    put_elems(e, id, elems);
    Ok(acc)
}

/// `localeCompare` (déviation documentée : ordre par point de
/// code Unicode, pas de collation CLDR) — `-1/0/1`.
pub(crate) fn string_locale_compare(
    e: &mut Engine,
    this: Value,
    args: &[Value],
) -> JsResult<Value> {
    let s = this_str(e, this)?;
    let o = e.to_js_string(arg(args, 0))?;
    let r = match s.cmp(&o) {
        std::cmp::Ordering::Less => -1.0,
        std::cmp::Ordering::Equal => 0.0,
        std::cmp::Ordering::Greater => 1.0,
    };
    Ok(Value::Num(r))
}

/// `Number.prototype.toLocaleString` — repli `toString` (pas de
/// locale, déviation documentée).
pub(crate) fn number_to_locale(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    number_to_string(e, this, args)
}

// -- URL (WHATWG subset) ---------------------------------------------

/// Parts décomposées d'une URL — pas d'IDNA ni de validation
/// d'hôte stricte (subset navigateur : schéma/authority/path/
/// query/fragment + résolution relative basique).
pub(crate) struct UrlParts {
    scheme: String,
    user: String,
    pass: String,
    host: String,
    port: String,
    path: String,
    query: String,
    frag: String,
}

const URL_DEFAULT_PORTS: &[(&str, &str)] = &[
    ("http", "80"),
    ("https", "443"),
    ("ws", "80"),
    ("wss", "443"),
    ("ftp", "21"),
];

pub(crate) fn url_has_origin(scheme: &str) -> bool {
    matches!(scheme, "http" | "https" | "ws" | "wss" | "ftp" | "file")
}

/// Découpe `user:pass@host:port` (authority après `//`).
pub(crate) fn url_split_authority(auth: &str, p: &mut UrlParts) {
    let (up, hp) = match auth.rsplit_once('@') {
        Some((u, h)) => (u, h),
        None => ("", auth),
    };
    if let Some((u, pw)) = up.split_once(':') {
        p.user = u.to_string();
        p.pass = pw.to_string();
    } else {
        p.user = up.to_string();
    }
    // Port = `:digits` en fin d'authority (host nu IPv6 `[…]` ok).
    if let Some(brk) = hp.rsplit_once(':') {
        if !brk.1.is_empty() && brk.1.chars().all(|c| c.is_ascii_digit()) && !brk.0.ends_with(']') {
            let default = URL_DEFAULT_PORTS
                .iter()
                .find(|(s, _)| s == &p.scheme.as_str())
                .map_or("", |(_, d)| *d);
            p.host = brk.0.to_string();
            p.port = if brk.1 == default {
                String::new()
            } else {
                brk.1.to_string()
            };
            return;
        }
    }
    p.host = hp.to_string();
}

/// Normalise les segments `.`/`..` d'un chemin.
pub(crate) fn url_norm_path(path: &str) -> String {
    let mut segs: Vec<&str> = Vec::new();
    for seg in path.split('/') {
        match seg {
            "." => {}
            ".." => {
                segs.pop();
            }
            _ => segs.push(seg),
        }
    }
    let out = segs.join("/");
    if path.starts_with('/') && !out.starts_with('/') {
        return format!("/{out}");
    }
    out
}

/// Parse absolu `scheme:…` — `None` si pas de schéma.
pub(crate) fn url_parse_absolute(s: &str) -> Option<UrlParts> {
    let colon = s.find(':')?;
    let scheme = &s[..colon];
    if scheme.is_empty()
        || !scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
        || !scheme
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic())
    {
        return None;
    }
    let mut p = UrlParts {
        scheme: scheme.to_ascii_lowercase(),
        user: String::new(),
        pass: String::new(),
        host: String::new(),
        port: String::new(),
        path: String::new(),
        query: String::new(),
        frag: String::new(),
    };
    let mut rest = &s[colon + 1..];
    if let Some(a) = rest.strip_prefix("//") {
        rest = a;
        let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
        url_split_authority(&rest[..end], &mut p);
        rest = &rest[end..];
    }
    let path_end = rest.find(['?', '#']).unwrap_or(rest.len());
    p.path = url_norm_path(&rest[..path_end]);
    rest = &rest[path_end..];
    if let Some(r) = rest.strip_prefix('?') {
        let qend = r.find('#').unwrap_or(r.len());
        p.query = r[..qend].to_string();
        rest = &r[qend..];
    }
    if let Some(f) = rest.strip_prefix('#') {
        p.frag = f.to_string();
    }
    Some(p)
}

/// Résolution relative basique (WHATWG `basic URL parser` subset) :
/// `//h`, `/p`, `?q`, `#f`, chemin relatif depuis le dossier de base.
pub(crate) fn url_parse(href: &str, base: Option<&UrlParts>) -> Option<UrlParts> {
    let s = href.trim();
    if let Some(p) = url_parse_absolute(s) {
        return Some(p);
    }
    let b = base?;
    let mut p = UrlParts {
        scheme: b.scheme.clone(),
        user: b.user.clone(),
        pass: b.pass.clone(),
        host: b.host.clone(),
        port: b.port.clone(),
        path: b.path.clone(),
        query: b.query.clone(),
        frag: String::new(),
    };
    if s.is_empty() {
        p.frag.clone_from(&b.frag);
        return Some(p);
    }
    if let Some(f) = s.strip_prefix('#') {
        p.frag = f.to_string();
        return Some(p);
    }
    p.frag.clear();
    if let Some(q) = s.strip_prefix('?') {
        p.query = q.to_string();
        return Some(p);
    }
    p.query.clear();
    if let Some(a) = s.strip_prefix("//") {
        let end = a.find(['/', '?', '#']).unwrap_or(a.len());
        url_split_authority(&a[..end], &mut p);
        let rest = &a[end..];
        let path_end = rest.find(['?', '#']).unwrap_or(rest.len());
        p.path = url_norm_path(&rest[..path_end]);
        let rest = &rest[path_end..];
        if let Some(r) = rest.strip_prefix('?') {
            let qend = r.find('#').unwrap_or(r.len());
            p.query = r[..qend].to_string();
            if let Some(f) = r[qend..].strip_prefix('#') {
                p.frag = f.to_string();
            }
        } else if let Some(f) = rest.strip_prefix('#') {
            p.frag = f.to_string();
        }
        return Some(p);
    }
    // Chemin : absolu (`/p`) ou relatif au dossier de base.
    let path_end = s.find(['?', '#']).unwrap_or(s.len());
    let raw = &s[..path_end];
    p.path = if raw.starts_with('/') {
        url_norm_path(raw)
    } else {
        let dir = match b.path.rfind('/') {
            Some(i) => &b.path[..=i],
            None => "/",
        };
        url_norm_path(&format!("{dir}{raw}"))
    };
    let rest = &s[path_end..];
    if let Some(r) = rest.strip_prefix('?') {
        let qend = r.find('#').unwrap_or(r.len());
        p.query = r[..qend].to_string();
        if let Some(f) = r[qend..].strip_prefix('#') {
            p.frag = f.to_string();
        }
    } else if let Some(f) = rest.strip_prefix('#') {
        p.frag = f.to_string();
    }
    Some(p)
}

pub(crate) fn url_href(p: &UrlParts) -> String {
    let mut out = format!("{}:", p.scheme);
    // `file:` garde son authority vide (`file:///x`) — spec.
    if !p.host.is_empty() || !p.user.is_empty() || p.scheme == "file" {
        out.push_str("//");
        if !p.user.is_empty() {
            out.push_str(&p.user);
            if !p.pass.is_empty() {
                out.push(':');
                out.push_str(&p.pass);
            }
            out.push('@');
        }
        out.push_str(&p.host);
        if !p.port.is_empty() {
            out.push(':');
            out.push_str(&p.port);
        }
    }
    if p.path.is_empty() && url_has_origin(&p.scheme) {
        out.push('/');
    } else {
        out.push_str(&p.path);
    }
    if !p.query.is_empty() {
        out.push('?');
        out.push_str(&p.query);
    }
    if !p.frag.is_empty() {
        out.push('#');
        out.push_str(&p.frag);
    }
    out
}

pub(crate) fn url_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let href = e.to_js_string(arg(args, 0))?;
    // `base` peut être une `URL` (lit son `href`) ou une string.
    let base_s = match arg(args, 1) {
        Value::Undefined => None,
        v => Some(e.to_js_string(v)?),
    };
    let base = base_s.as_deref().and_then(|b| url_parse_absolute(b.trim()));
    if base_s.is_some() && base.is_none() {
        return Err(e.type_error("Invalid base URL"));
    }
    let Some(p) = url_parse(&href, base.as_ref()) else {
        return Err(e.type_error("Invalid URL"));
    };
    let proto = Value::Obj(e.protos.url);
    let id = e.heap.alloc_obj(crate::heap::Object::ordinary(proto));
    let host = if p.port.is_empty() {
        p.host.clone()
    } else {
        format!("{}:{}", p.host, p.port)
    };
    let origin = if url_has_origin(&p.scheme) && p.scheme != "file" {
        let mut o = format!("{}://{}", p.scheme, p.host);
        if !p.port.is_empty() {
            o.push(':');
            o.push_str(&p.port);
        }
        o
    } else {
        "null".to_string()
    };
    let fields = [
        ("href", url_href(&p)),
        ("protocol", format!("{}:", p.scheme)),
        ("origin", origin),
        ("username", p.user.clone()),
        ("password", p.pass.clone()),
        ("host", host),
        ("hostname", p.host.clone()),
        ("port", p.port.clone()),
        (
            "pathname",
            if p.path.is_empty() && url_has_origin(&p.scheme) {
                "/".to_string()
            } else {
                p.path.clone()
            },
        ),
        (
            "search",
            if p.query.is_empty() {
                String::new()
            } else {
                format!("?{}", p.query)
            },
        ),
        (
            "hash",
            if p.frag.is_empty() {
                String::new()
            } else {
                format!("#{}", p.frag)
            },
        ),
    ];
    for (k, v) in fields {
        let sv = e.str_val(v);
        e.set_own(id, k, sv, true);
    }
    // `searchParams` lié à la query.
    let pairs = url_decode_pairs(&p.query);
    let sp = usp_new(e, pairs);
    e.set_own(id, "searchParams", sp, true);
    Ok(Value::Obj(id))
}

pub(crate) fn url_href_get(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let Value::Obj(id) = this else {
        return Err(e.type_error("URL method on non-URL"));
    };
    Ok(e.get_prop(id, "href").unwrap_or(Value::Undefined))
}

// -- URLSearchParams (x-www-form-urlencoded) --------------------------

/// Paire `(name, value)` encodée formulaire — `+` = espace.
pub(crate) fn url_decode_pairs(q: &str) -> Vec<(String, String)> {
    if q.is_empty() {
        return Vec::new();
    }
    q.split('&')
        .filter(|seg| !seg.is_empty())
        .map(|seg| {
            let (k, v) = seg.split_once('=').unwrap_or((seg, ""));
            (form_decode(k), form_decode(v))
        })
        .collect()
}

pub(crate) fn form_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < bytes.len() => {
                let hex = |b: u8| (b as char).to_digit(16);
                match (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                    (Some(h), Some(l)) => {
                        out.push((h * 16 + l) as u8);
                        i += 2;
                    }
                    _ => out.push(b'%'),
                }
            }
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub(crate) fn form_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'*' | b'-' | b'.' | b'_' => {
                out.push(b as char);
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

const USP_PAIRS: &str = "\\u{f131}urlp";

/// Lit les paires cachées `\u{f131}urlp` (array `[ [k,v],… ]`).
pub(crate) fn usp_pairs(e: &mut Engine, id: crate::value::ObjId) -> Vec<(String, String)> {
    let Some(Value::Obj(arr)) = e.get_prop(id, USP_PAIRS) else {
        return Vec::new();
    };
    let n = match e.get_prop(arr, "length") {
        Some(Value::Num(n)) => n.max(0.0) as usize,
        _ => 0,
    };
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        if let Some(Value::Obj(pair)) = e.get_prop(arr, &i.to_string()) {
            let k = e
                .get_prop(pair, "0")
                .map(|v| e.to_js_string(v).unwrap_or_default())
                .unwrap_or_default();
            let v = e
                .get_prop(pair, "1")
                .map(|v| e.to_js_string(v).unwrap_or_default())
                .unwrap_or_default();
            out.push((k, v));
        }
    }
    out
}

pub(crate) fn usp_write(e: &mut Engine, id: crate::value::ObjId, pairs: Vec<(String, String)>) {
    let vals: Vec<Value> = pairs
        .into_iter()
        .map(|(k, v)| {
            let k = e.str_val(k);
            let v = e.str_val(v);
            e.new_array(vec![k, v])
        })
        .collect();
    let arr = e.new_array(vals);
    e.set_own(id, USP_PAIRS, arr, false);
}

/// Nouvel objet `URLSearchParams` (proto `protos.url_params`).
pub(crate) fn usp_new(e: &mut Engine, pairs: Vec<(String, String)>) -> Value {
    let proto = Value::Obj(e.protos.url_params);
    let id = e.heap.alloc_obj(crate::heap::Object::ordinary(proto));
    usp_write(e, id, pairs);
    Value::Obj(id)
}

pub(crate) fn usp_this(
    e: &mut Engine,
    this: Value,
) -> JsResult<(crate::value::ObjId, Vec<(String, String)>)> {
    let Value::Obj(id) = this else {
        return Err(e.type_error("URLSearchParams method on non-object"));
    };
    if !matches!(e.get_prop(id, USP_PAIRS), Some(Value::Obj(_))) {
        return Err(e.type_error("URLSearchParams method on incompatible receiver"));
    }
    Ok((id, usp_pairs(e, id)))
}

pub(crate) fn usp_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let mut pairs = Vec::new();
    match arg(args, 0) {
        Value::Undefined | Value::Null => {}
        v @ Value::Obj(oid) => {
            // Iterable (`[[k,v],…]`, autre URLSearchParams via son
            // `[Symbol.iterator]`) → paires ; non-itérable → record
            // `{k: v}` dans l'ordre des props énumérables.
            match e.iter_values(v) {
                Ok(items) => {
                    for it in items {
                        let k = e.get_prop_value(it, "0")?;
                        let val = e.get_prop_value(it, "1")?;
                        pairs.push((e.to_js_string(k)?, e.to_js_string(val)?));
                    }
                }
                Err(_) => {
                    if matches!(e.get_prop(oid, USP_PAIRS), Some(Value::Obj(_))) {
                        // Copie directe si l'itérateur a été cassé.
                        pairs = usp_pairs(e, oid);
                    } else {
                        for k in e.own_enum_keys(oid) {
                            let val = e.get_prop_value(v, &k)?;
                            pairs.push((k, e.to_js_string(val)?));
                        }
                    }
                }
            }
        }
        v => {
            let s = e.to_js_string(v)?;
            let q = s.strip_prefix('?').unwrap_or(&s);
            pairs = url_decode_pairs(q);
        }
    }
    Ok(usp_new(e, pairs))
}

pub(crate) fn usp_get(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let (_id, pairs) = usp_this(e, this)?;
    let name = e.to_js_string(arg(args, 0))?;
    match pairs.iter().find(|(k, _)| *k == name) {
        Some((_, v)) => Ok(e.str_val(v.clone())),
        None => Ok(Value::Null),
    }
}

pub(crate) fn usp_get_all(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let (_id, pairs) = usp_this(e, this)?;
    let name = e.to_js_string(arg(args, 0))?;
    let vals: Vec<Value> = pairs
        .iter()
        .filter(|(k, _)| *k == name)
        .map(|(_, v)| e.str_val(v.clone()))
        .collect();
    Ok(e.new_array(vals))
}

pub(crate) fn usp_has(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let (_id, pairs) = usp_this(e, this)?;
    let name = e.to_js_string(arg(args, 0))?;
    Ok(Value::Bool(pairs.iter().any(|(k, _)| *k == name)))
}

pub(crate) fn usp_set(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let (id, mut pairs) = usp_this(e, this)?;
    let name = e.to_js_string(arg(args, 0))?;
    let val = e.to_js_string(arg(args, 1))?;
    let mut out = Vec::with_capacity(pairs.len());
    let mut inserted = false;
    for (k, v) in pairs.drain(..) {
        if k == name {
            if !inserted {
                out.push((k, val.clone()));
                inserted = true;
            }
        } else {
            out.push((k, v));
        }
    }
    if !inserted {
        out.push((name, val));
    }
    usp_write(e, id, out);
    Ok(Value::Undefined)
}

pub(crate) fn usp_append(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let (id, mut pairs) = usp_this(e, this)?;
    let name = e.to_js_string(arg(args, 0))?;
    let val = e.to_js_string(arg(args, 1))?;
    pairs.push((name, val));
    usp_write(e, id, pairs);
    Ok(Value::Undefined)
}

pub(crate) fn usp_delete(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let (id, pairs) = usp_this(e, this)?;
    let name = e.to_js_string(arg(args, 0))?;
    let val_filter = match arg(args, 1) {
        Value::Undefined => None,
        v => Some(e.to_js_string(v)?),
    };
    let keep: Vec<(String, String)> = pairs
        .into_iter()
        .filter(|(k, v)| !(k == &name && val_filter.as_ref().is_none_or(|f| f == v)))
        .collect();
    usp_write(e, id, keep);
    Ok(Value::Undefined)
}

pub(crate) fn usp_sort(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let (id, mut pairs) = usp_this(e, this)?;
    // Tri stable sur la clé (spec : code points).
    pairs.sort_by(|a, b| a.0.cmp(&b.0));
    usp_write(e, id, pairs);
    Ok(Value::Undefined)
}

pub(crate) fn usp_for_each(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let (_id, pairs) = usp_this(e, this)?;
    let cb = arg(args, 0);
    if !e.is_callable(cb) {
        return Err(e.type_error("forEach needs a function"));
    }
    let this_arg = arg(args, 1);
    for (k, v) in pairs {
        let kv = e.str_val(k);
        let vv = e.str_val(v);
        e.call_value(cb, this_arg, &[vv, kv, this])?;
    }
    Ok(Value::Undefined)
}

pub(crate) fn usp_to_string(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let (_id, pairs) = usp_this(e, this)?;
    let out: Vec<String> = pairs
        .iter()
        .map(|(k, v)| format!("{}={}", form_encode(k), form_encode(v)))
        .collect();
    Ok(e.str_val(out.join("&")))
}

pub(crate) fn usp_keys(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let (_id, pairs) = usp_this(e, this)?;
    let vals: Vec<Value> = pairs.into_iter().map(|(k, _)| e.str_val(k)).collect();
    Ok(crate::collections::iter_obj(e, vals, e.protos.usp_iter))
}

pub(crate) fn usp_values(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let (_id, pairs) = usp_this(e, this)?;
    let vals: Vec<Value> = pairs.into_iter().map(|(_, v)| e.str_val(v)).collect();
    Ok(crate::collections::iter_obj(e, vals, e.protos.usp_iter))
}

pub(crate) fn usp_entries(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let (_id, pairs) = usp_this(e, this)?;
    let vals: Vec<Value> = pairs
        .into_iter()
        .map(|(k, v)| {
            let k = e.str_val(k);
            let v = e.str_val(v);
            e.new_array(vec![k, v])
        })
        .collect();
    Ok(crate::collections::iter_obj(e, vals, e.protos.usp_iter))
}
