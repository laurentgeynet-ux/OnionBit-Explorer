// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! `JSON.parse` / `JSON.stringify` (JS-7e).
//!
//! `parse` is a small recursive-descent reader (not the script
//! parser — JSON is stricter: strings double-quoted, no trailing
//! commas). `stringify` handles primitives, arrays, objects,
//! `toJSON` and the `replacer`/`space` arguments in their common
//! shapes.

use crate::heap::{JsResult, ObjKind};
use crate::value::Value;
use crate::Engine;

fn arg(args: &[Value], i: usize) -> Value {
    args.get(i).copied().unwrap_or(Value::Undefined)
}

// ------------------------------------------------------------------
// parse
// ------------------------------------------------------------------

struct Reader<'a> {
    s: &'a [u8],
    i: usize,
}

impl Reader<'_> {
    fn ws(&mut self) {
        while matches!(self.s.get(self.i), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.i += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }

    fn eat(&mut self, b: u8) -> bool {
        self.ws();
        if self.peek() == Some(b) {
            self.i += 1;
            true
        } else {
            false
        }
    }

    fn expect(&mut self, b: u8) -> Result<(), &'static str> {
        if self.eat(b) {
            Ok(())
        } else {
            Err("unexpected character")
        }
    }
}

fn json_string(r: &mut Reader) -> Result<String, &'static str> {
    if !r.eat(b'"') {
        return Err("expected string");
    }
    let mut out = String::new();
    loop {
        match r.peek() {
            None => return Err("unterminated string"),
            Some(b'"') => {
                r.i += 1;
                return Ok(out);
            }
            Some(b'\\') => {
                r.i += 1;
                match r.peek() {
                    Some(b'"') => out.push('"'),
                    Some(b'\\') => out.push('\\'),
                    Some(b'/') => out.push('/'),
                    Some(b'b') => out.push('\u{8}'),
                    Some(b'f') => out.push('\u{c}'),
                    Some(b'n') => out.push('\n'),
                    Some(b'r') => out.push('\r'),
                    Some(b't') => out.push('\t'),
                    Some(b'u') => {
                        r.i += 1;
                        let hex = r.s.get(r.i..r.i + 4).ok_or("bad \\u escape")?;
                        let n = u16::from_str_radix(
                            std::str::from_utf8(hex).map_err(|_| "bad \\u escape")?,
                            16,
                        )
                        .map_err(|_| "bad \\u escape")?;
                        r.i += 3; // +1 from the shared advance below
                                  // BMP char; surrogate pairs collapse to the
                                  // replacement char if unpaired (tolerant).
                        if (0xD800..0xDC00).contains(&n) {
                            // High surrogate — try `\uXXXX` low half.
                            let save = r.i;
                            r.i += 1;
                            if r.peek() == Some(b'\\') && r.s.get(r.i + 1) == Some(&b'u') {
                                if let Some(h2) = r.s.get(r.i + 2..r.i + 6) {
                                    if let Ok(lo) = u16::from_str_radix(
                                        std::str::from_utf8(h2).unwrap_or(""),
                                        16,
                                    ) {
                                        if (0xDC00..0xE000).contains(&lo) {
                                            let c = 0x10000u32
                                                + (((n as u32) - 0xD800) << 10)
                                                + (lo as u32 - 0xDC00);
                                            out.push(char::from_u32(c).unwrap_or('\u{fffd}'));
                                            r.i += 5;
                                        } else {
                                            out.push('\u{fffd}');
                                            r.i = save;
                                        }
                                    } else {
                                        out.push('\u{fffd}');
                                        r.i = save;
                                    }
                                } else {
                                    out.push('\u{fffd}');
                                    r.i = save;
                                }
                            } else {
                                out.push('\u{fffd}');
                                r.i = save;
                            }
                        } else if (0xDC00..0xE000).contains(&n) {
                            out.push('\u{fffd}');
                        } else {
                            out.push(char::from_u32(n as u32).unwrap_or('\u{fffd}'));
                        }
                    }
                    _ => return Err("bad escape"),
                }
                r.i += 1;
            }
            Some(_) => {
                // UTF-8 passthrough — decode one char.
                let rest = &r.s[r.i..];
                let s = std::str::from_utf8(rest).unwrap_or("");
                match s.chars().next() {
                    Some(c) => {
                        out.push(c);
                        r.i += c.len_utf8();
                    }
                    None => return Err("invalid utf-8"),
                }
            }
        }
    }
}

fn json_value(e: &mut Engine, r: &mut Reader, depth: u32) -> JsResult<Value> {
    if depth > 200 {
        return Err(e.range_error("JSON too deep"));
    }
    e.tick()?;
    r.ws();
    let v = match r.peek() {
        Some(b'f') if r.s[r.i..].starts_with(b"false") => {
            r.i += 5;
            Value::Bool(false)
        }
        Some(b't') if r.s[r.i..].starts_with(b"true") => {
            r.i += 4;
            Value::Bool(true)
        }
        Some(b'n') if r.s[r.i..].starts_with(b"null") => {
            r.i += 4;
            Value::Null
        }
        Some(b'"') => {
            let s = json_string(r).map_err(|m| e.syntax_error(m))?;
            e.str_val(s)
        }
        Some(b'[') => {
            r.i += 1;
            let mut elems = Vec::new();
            if !r.eat(b']') {
                loop {
                    elems.push(json_value(e, r, depth + 1)?);
                    if r.eat(b']') {
                        break;
                    }
                    r.expect(b',').map_err(|m| e.syntax_error(m))?;
                }
            }
            e.new_array(elems)
        }
        Some(b'{') => {
            r.i += 1;
            let id = e.new_object();
            if !r.eat(b'}') {
                loop {
                    r.ws();
                    let k = json_string(r).map_err(|m| e.syntax_error(m))?;
                    r.expect(b':').map_err(|m| e.syntax_error(m))?;
                    let v = json_value(e, r, depth + 1)?;
                    e.set_own(id, &k, v, true);
                    if r.eat(b'}') {
                        break;
                    }
                    r.expect(b',').map_err(|m| e.syntax_error(m))?;
                }
            }
            Value::Obj(id)
        }
        Some(b'-' | b'0'..=b'9') => {
            // Scan the JSON number grammar.
            let start = r.i;
            if r.peek() == Some(b'-') {
                r.i += 1;
            }
            while matches!(r.peek(), Some(b'0'..=b'9')) {
                r.i += 1;
            }
            if r.peek() == Some(b'.') {
                r.i += 1;
                while matches!(r.peek(), Some(b'0'..=b'9')) {
                    r.i += 1;
                }
            }
            if matches!(r.peek(), Some(b'e' | b'E')) {
                r.i += 1;
                if matches!(r.peek(), Some(b'+' | b'-')) {
                    r.i += 1;
                }
                while matches!(r.peek(), Some(b'0'..=b'9')) {
                    r.i += 1;
                }
            }
            let txt = std::str::from_utf8(&r.s[start..r.i]).unwrap_or("0");
            match txt.parse::<f64>() {
                Ok(n) if txt != "-" => Value::Num(n),
                _ => return Err(e.syntax_error("bad JSON number")),
            }
        }
        _ => return Err(e.syntax_error("unexpected token in JSON")),
    };
    Ok(v)
}

/// `JSON.parse` of an already-stringified value — shared with
/// `Response.json()`.
pub(crate) fn parse(e: &mut Engine, v: Value) -> JsResult<Value> {
    let text = e.to_js_string(v)?;
    let mut r = Reader {
        s: text.as_bytes(),
        i: 0,
    };
    let v = json_value(e, &mut r, 0)?;
    r.ws();
    if r.i != r.s.len() {
        return Err(e.syntax_error("trailing data in JSON"));
    }
    Ok(v)
}

fn json_parse(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let v = parse(e, arg(args, 0))?;
    let reviver = arg(args, 1);
    if !e.is_callable(reviver) {
        return Ok(v);
    }
    // Reviver (spec `InternalizeJSONProperty`) : parcours
    // post-ordre, `reviver.call(holder, key, value)` — retour
    // `undefined` → suppression de la prop.
    let holder = e.new_object();
    let root_key = e.str_val("");
    e.set_own(holder, "", v, true);
    internalize(e, holder, root_key, reviver)
}

/// `InternalizeJSONProperty(holder, name)` — descend dans les
/// objets/tableaux puis appelle le reviver sur `holder[name]`.
fn internalize(
    e: &mut Engine,
    holder: crate::value::ObjId,
    name: Value,
    reviver: Value,
) -> JsResult<Value> {
    let key = match name {
        Value::Str(sid) => e.heap.string(sid).unwrap_or_default(),
        Value::Num(n) => crate::value::num_to_js_string(n),
        _ => String::new(),
    };
    let val = e
        .get_prop_value(Value::Obj(holder), &key)
        .unwrap_or(Value::Undefined);
    if let Value::Obj(oid) = val {
        let keys: Vec<String> = match e.heap.obj(oid).map(|o| &o.kind) {
            Some(crate::heap::ObjKind::Array(elems)) => {
                (0..elems.len()).map(|i| i.to_string()).collect()
            }
            Some(_) => e.own_enum_keys(oid),
            None => Vec::new(),
        };
        for k in keys {
            let kval = e.str_val(k.clone());
            let sub = internalize(e, oid, kval, reviver)?;
            if matches!(sub, Value::Undefined) {
                e.delete_result(oid, &k)?;
            } else {
                // `set_prop_value` : les indices tombent dans les
                // éléments (`set_own` ne touche pas `ObjKind::Array`).
                e.set_prop_value(Value::Obj(oid), &k, sub)?;
            }
        }
    }
    e.call_value(reviver, Value::Obj(holder), &[name, val])
}

// ------------------------------------------------------------------
// stringify
// ------------------------------------------------------------------

fn esc_str(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// État de sérialisation — `replacer` (fonction appelée
/// `(key, value)` avec `holder` en `this`) et `plist`
/// (`PropertyList` — whitelist de clés, ordre de la liste).
struct Ctx {
    replacer: Option<Value>,
    plist: Option<Vec<String>>,
}

fn json_write(
    e: &mut Engine,
    out: &mut String,
    v: Value,
    space: &str,
    indent: u32,
    depth: u32,
    cx: &Ctx,
) -> JsResult<bool> {
    if depth > 200 {
        // Spec: TypeError ("Converting circular structure to JSON").
        return Err(e.type_error("JSON.stringify cycle/too deep"));
    }
    e.tick()?;
    match v {
        Value::Undefined => return Ok(false), // elided
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if b { "true" } else { "false" }),
        Value::Num(n) => {
            if n.is_finite() {
                out.push_str(&crate::value::num_to_js_string(n));
            } else {
                out.push_str("null");
            }
        }
        Value::Str(id) => {
            let s = e.heap.string(id).unwrap_or_default();
            esc_str(out, &s);
        }
        // Spec `SerializeJSONProperty` : BigInt → TypeError.
        Value::Big(_) => return Err(e.type_error("do not know how to serialize a BigInt")),
        Value::Obj(id) => {
            // `JSON.rawJSON` : le texte est émis tel quel (spec
            // `SerializeJSONProperty` reconnaît le slot interne).
            let raw = e.heap.obj(id).and_then(|o| {
                if !o.extensible
                    && matches!(o.proto, Value::Null)
                    && matches!(o.get_own("rawJSON"), Some(Value::Str(_)))
                {
                    o.get_own("rawJSON")
                } else {
                    None
                }
            });
            if let Some(Value::Str(rid)) = raw {
                let s = e.heap.string(rid).unwrap_or_default();
                out.push_str(&s);
                return Ok(true);
            }
            // Functions AND Symbols are not JSON values — elided
            // like undefined.
            if matches!(
                e.heap.obj(id).map(|o| &o.kind),
                Some(ObjKind::Fun(_) | ObjKind::Symbol { .. })
            ) {
                return Ok(false);
            }
            // `toJSON` hook.
            if let Some(f) = e.get_prop(id, "toJSON") {
                if e.is_callable(f) {
                    let v2 = e.call_value(f, v, &[])?;
                    return json_write(e, out, v2, space, indent, depth + 1, cx);
                }
            }
            let kind_is_arr = matches!(e.heap.obj(id).map(|o| &o.kind), Some(ObjKind::Array(_)));
            let pad = " ".repeat((indent + 1) as usize * space.len());
            let close_pad = " ".repeat(indent as usize * space.len());
            let nl = if space.is_empty() { "" } else { "\n" };
            if kind_is_arr {
                let n = match e.heap.obj(id).map(|o| &o.kind) {
                    Some(ObjKind::Array(elems)) => elems.len(),
                    _ => 0,
                };
                out.push('[');
                for i in 0..n {
                    if i > 0 {
                        out.push(',');
                    }
                    out.push_str(nl);
                    out.push_str(&pad);
                    let mut el = e
                        .find_own(id, &i.to_string())
                        .and_then(|p| p.data_value())
                        .unwrap_or(Value::Undefined);
                    if let Some(rp) = cx.replacer {
                        let kv = e.str_val(i.to_string());
                        el = e.call_value(rp, v, &[kv, el])?;
                    }
                    if !json_write(e, out, el, space, indent + 1, depth + 1, cx)? {
                        out.push_str("null"); // undefined → null in arrays
                    }
                }
                if n > 0 {
                    out.push_str(nl);
                    out.push_str(&close_pad);
                }
                out.push(']');
            } else {
                // Ordinary objects (incl. Map/Set → `{}` — spec).
                // `PropertyList` : seules les clés listées, dans
                // l'ordre de la liste ; sinon toutes les clés propres
                // énumérables.
                let keys = match &cx.plist {
                    Some(pl) => pl.clone(),
                    None => e.own_enum_keys(id),
                };
                out.push('{');
                let mut first = true;
                for k in keys {
                    let Some(mut val) = e.find_own(id, &k).and_then(|p| p.data_value()) else {
                        continue;
                    };
                    if let Some(rp) = cx.replacer {
                        let kv = e.str_val(k.clone());
                        val = e.call_value(rp, v, &[kv, val])?;
                    }
                    // Skip undefined/functions/symbols in values.
                    if matches!(val, Value::Undefined) {
                        continue;
                    }
                    if let Value::Obj(fid) = val {
                        if matches!(
                            e.heap.obj(fid).map(|o| &o.kind),
                            Some(ObjKind::Fun(_) | ObjKind::Symbol { .. })
                        ) {
                            continue;
                        }
                    }
                    if !first {
                        out.push(',');
                    }
                    first = false;
                    out.push_str(nl);
                    out.push_str(&pad);
                    esc_str(out, &k);
                    out.push(':');
                    if !space.is_empty() {
                        out.push(' ');
                    }
                    json_write(e, out, val, space, indent + 1, depth + 1, cx)?;
                }
                if !first {
                    out.push_str(nl);
                    out.push_str(&close_pad);
                }
                out.push('}');
            }
        }
    }
    Ok(true)
}

/// `JSON.rawJSON(text)` (ES2026) — objet gelé à proto nul portant
/// `rawJSON` (le texte émis tel quel par `stringify`). Spec : le
/// texte doit être un JSON valide qui ne produit pas un
/// conteneur — on le re-parse ; un JSON invalide ou un
/// objet/tableau → `SyntaxError`.
fn json_raw_json(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let text = match arg(args, 0) {
        Value::Str(id) => e.heap.string(id).unwrap_or_default(),
        _ => {
            return Err(e.type_error("JSON.rawJSON expects a string"));
        }
    };
    let tin = e.str_val(text.clone());
    match json_parse(e, Value::Undefined, &[tin]) {
        Ok(Value::Obj(_)) => {
            return Err(e.syntax_error("JSON.rawJSON must not produce an object"));
        }
        Ok(_) => {}
        Err(t) => return Err(t),
    }
    let obj = e.new_object();
    let raw = e.str_val(text);
    e.set_own(obj, "rawJSON", raw, false);
    // Spec : `Object.freeze(Object.create(null))`.
    if let Some(o) = e.heap.obj_mut(obj) {
        o.proto = Value::Null;
        o.extensible = false;
    }
    Ok(Value::Obj(obj))
}

/// `JSON.isRawJSON(v)` — objet gelé à proto nul portant une
/// `rawJSON` string (marqueur posé par `rawJSON` uniquement —
/// approximation documentée du slot interne `[[RawJSON]]`).
fn json_is_raw_json(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let ok = match arg(args, 0) {
        Value::Obj(id) => {
            let marked = e.heap.obj(id).is_some_and(|o| {
                !o.extensible
                    && matches!(o.proto, Value::Null)
                    && matches!(o.get_own("rawJSON"), Some(Value::Str(_)))
            });
            marked
        }
        _ => false,
    };
    Ok(Value::Bool(ok))
}

fn json_stringify(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let v = arg(args, 0);
    // `replacer` — fonction (spec `Call` par prop) ou tableau de
    // clés (`PropertyList` : strings/numbers → ToString, dédup).
    let rep = arg(args, 1);
    let mut plist: Option<Vec<String>> = None;
    let replacer = if e.is_callable(rep) {
        Some(rep)
    } else {
        if let Value::Obj(pid) = rep {
            if matches!(e.heap.obj(pid).map(|o| &o.kind), Some(ObjKind::Array(_))) {
                let n = e
                    .get_prop_value(rep, "length")
                    .ok()
                    .and_then(|v| match v {
                        Value::Num(n) => Some(n as usize),
                        _ => None,
                    })
                    .unwrap_or(0);
                let mut list: Vec<String> = Vec::new();
                for i in 0..n {
                    let ev = e.get_prop_value(rep, &i.to_string())?;
                    let k = match ev {
                        Value::Str(sid) => e.heap.string(sid).unwrap_or_default(),
                        Value::Num(nv) => crate::value::num_to_js_string(nv),
                        // Wrapper String/Number → ToString.
                        Value::Obj(_) => match e.to_js_string(ev) {
                            Ok(s) => s,
                            Err(_) => continue,
                        },
                        _ => continue,
                    };
                    if !list.contains(&k) {
                        list.push(k);
                    }
                }
                plist = Some(list);
            }
        }
        None
    };
    // `space` — number → that many spaces (cap 10), string → literal
    // prefix (cap 10 chars). Wrapper Number/String → ToNumber/
    // ToString (spec).
    let space = match arg(args, 2) {
        Value::Num(n) => " ".repeat((n as usize).min(10)),
        Value::Str(id) => e
            .heap
            .string(id)
            .unwrap_or_default()
            .chars()
            .take(10)
            .collect(),
        Value::Obj(_) => {
            let v = arg(args, 2);
            if let Ok(n) = e.to_number(v) {
                " ".repeat((n as usize).min(10))
            } else {
                e.to_js_string(v)
                    .unwrap_or_default()
                    .chars()
                    .take(10)
                    .collect()
            }
        }
        _ => String::new(),
    };
    let cx = Ctx { replacer, plist };
    // Racine : holder `{"": v}` — le replacer y est appelé avec
    // la clé `""` (spec `SerializeJSONProperty` sur la racine).
    let mut root = v;
    if let Some(rp) = cx.replacer {
        let holder = e.new_object();
        e.set_own(holder, "", v, true);
        let kv = e.str_val("");
        root = e.call_value(rp, Value::Obj(holder), &[kv, v])?;
    }
    let mut out = String::new();
    if !json_write(e, &mut out, root, &space, 0, 0, &cx)? {
        return Ok(Value::Undefined);
    }
    Ok(e.str_val(out))
}

/// Install the `JSON` global object.
pub(crate) fn install(e: &mut Engine) {
    let j = e.new_object();
    e.add_method(j, "parse", json_parse);
    e.add_method(j, "stringify", json_stringify);
    e.add_method(j, "rawJSON", json_raw_json);
    e.add_method(j, "isRawJSON", json_is_raw_json);
    let tag = e.str_val("JSON");
    e.set_own(j, crate::interp::SYM_TO_STRING_TAG, tag, false);
    let g = e.global;
    e.set_own(g, "JSON", Value::Obj(j), false);
}
