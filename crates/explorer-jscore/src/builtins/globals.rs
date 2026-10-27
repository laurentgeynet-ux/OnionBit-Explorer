// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Builtins JS — section extraite de `builtins.rs` (JS-C40, pur
//! reformatage mécanique : sémantique et tests inchangés).

#[allow(clippy::wildcard_imports)]
use super::*;

// ------------------------------------------------------------------
// global functions
// ------------------------------------------------------------------

pub(crate) fn parse_int(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let s = e.to_js_string(arg(args, 0))?;
    let mut radix = match arg(args, 1) {
        Value::Undefined => 0i32,
        v => e.to_number(v)? as i32,
    };
    let t = s.trim_start();
    let (sign, mut rest) = match t.strip_prefix('-') {
        Some(r) => (-1.0, r),
        None => (1.0, t.strip_prefix('+').unwrap_or(t)),
    };
    if radix == 0 || radix == 16 {
        if let Some(h) = rest.strip_prefix("0x").or_else(|| rest.strip_prefix("0X")) {
            rest = h;
            radix = 16;
        }
    }
    if radix == 0 {
        radix = 10;
    }
    if !(2..=36).contains(&radix) {
        return Ok(Value::Num(f64::NAN));
    }
    let mut v = 0f64;
    let mut any = false;
    for c in rest.chars() {
        match c.to_digit(radix as u32) {
            Some(d) => {
                v = v * f64::from(radix) + f64::from(d);
                any = true;
            }
            None => break,
        }
    }
    Ok(Value::Num(if any { sign * v } else { f64::NAN }))
}

pub(crate) fn parse_float(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let s = e.to_js_string(arg(args, 0))?;
    let t = s.trim_start();
    if t.starts_with("Infinity") || t.starts_with("+Infinity") {
        return Ok(Value::Num(f64::INFINITY));
    }
    if t.starts_with("-Infinity") {
        return Ok(Value::Num(f64::NEG_INFINITY));
    }
    let b = t.as_bytes();
    let mut i = 0usize;
    if matches!(b.first(), Some(b'+' | b'-')) {
        i = 1;
    }
    let mut mantissa_digits = 0usize;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
        mantissa_digits += 1;
    }
    if b.get(i) == Some(&b'.') {
        i += 1;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
            mantissa_digits += 1;
        }
    }
    let mut end = i;
    if mantissa_digits > 0 && matches!(b.get(i), Some(b'e' | b'E')) {
        let mut j = i + 1;
        if matches!(b.get(j), Some(b'+' | b'-')) {
            j += 1;
        }
        let ds = j;
        while j < b.len() && b[j].is_ascii_digit() {
            j += 1;
        }
        if j > ds {
            end = j;
        }
    }
    if mantissa_digits == 0 {
        return Ok(Value::Num(f64::NAN));
    }
    Ok(Value::Num(t[..end].parse::<f64>().unwrap_or(f64::NAN)))
}

pub(crate) fn is_nan(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    Ok(Value::Bool(e.to_number(arg(args, 0))?.is_nan()))
}

pub(crate) fn is_finite(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    Ok(Value::Bool(e.to_number(arg(args, 0))?.is_finite()))
}

pub(crate) fn escape(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let s = e.to_js_string(arg(args, 0))?;
    let mut out = String::new();
    for c in s.chars() {
        if c.is_ascii_alphanumeric() || "@*_+-./".contains(c) {
            out.push(c);
        } else if (c as u32) < 256 {
            out.push_str(&format!("%{:02X}", c as u32));
        } else {
            out.push_str(&format!("%u{:04X}", c as u32));
        }
    }
    Ok(e.str_val(out))
}

pub(crate) fn unescape(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let s = e.to_js_string(arg(args, 0))?;
    let cs = chars_of(&s);
    let mut out = String::new();
    let mut i = 0;
    while i < cs.len() {
        if cs[i] == '%' {
            if cs.get(i + 1) == Some(&'u') && i + 6 <= cs.len() {
                if let Some(v) = hex_val(&cs[i + 2..i + 6]) {
                    if let Some(c) = char::from_u32(v) {
                        out.push(c);
                        i += 6;
                        continue;
                    }
                }
            }
            if i + 3 <= cs.len() {
                if let Some(v) = hex_val(&cs[i + 1..i + 3]) {
                    if let Some(c) = char::from_u32(v) {
                        out.push(c);
                        i += 3;
                        continue;
                    }
                }
            }
        }
        out.push(cs[i]);
        i += 1;
    }
    Ok(e.str_val(out))
}

pub(crate) fn hex_val(cs: &[char]) -> Option<u32> {
    if cs.is_empty() {
        return None;
    }
    let mut v = 0u32;
    for c in cs {
        v = v.checked_mul(16)?.checked_add(c.to_digit(16)?)?;
    }
    Some(v)
}

const URI_SAFE: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_.!~*'()";

pub(crate) fn encode_uri_component(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let s = e.to_js_string(arg(args, 0))?;
    let mut out = String::new();
    for b in s.bytes() {
        let c = b as char;
        if c.is_ascii() && URI_SAFE.contains(c) {
            out.push(c);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    Ok(e.str_val(out))
}

pub(crate) fn decode_uri_component(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let s = e.to_js_string(arg(args, 0))?;
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let h = (bytes[i + 1] as char).to_digit(16);
            let l = (bytes[i + 2] as char).to_digit(16);
            if let (Some(h), Some(l)) = (h, l) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    if let Ok(s) = String::from_utf8(out) {
        Ok(e.str_val(s))
    } else {
        let p = e.protos.error;
        Err(Thrown(e.error_value(p, "URIError", "malformed URI")))
    }
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub(crate) fn btoa(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let s = e.to_js_string(arg(args, 0))?;
    if s.chars().any(|c| c as u32 > 0xff) {
        return Err(e.type_error("btoa: non-latin1 input"));
    }
    let b: Vec<u8> = s.chars().map(|c| c as u8).collect();
    let b = b.as_slice();
    let mut out = String::new();
    for chunk in b.chunks(3) {
        let n = chunk.iter().fold(0u32, |a, &c| (a << 8) | u32::from(c));
        let n = n << (8 * (3 - chunk.len()));
        out.push(B64[(n >> 18 & 63) as usize] as char);
        out.push(B64[(n >> 12 & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            B64[(n >> 6 & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            B64[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    Ok(e.str_val(out))
}

pub(crate) fn atob(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let s = e.to_js_string(arg(args, 0))?;
    let mut bits = 0u32;
    let mut nbits = 0u32;
    let mut out = Vec::new();
    for c in s.chars() {
        if c == '=' {
            break;
        }
        // Hors ASCII → invalide (le `c as u8` tronquait
        // `Ā`/`` en alphabet — un octet de plus passait).
        let v = match u8::try_from(c)
            .ok()
            .and_then(|b| B64.iter().position(|&x| x == b))
        {
            Some(v) => v as u32,
            // Spec : ASCII whitespace ignoré avant décodage
            // (espace, \t, \n, \f, \r — pas NBSP).
            None if matches!(c, ' ' | '\t' | '\n' | '\x0c' | '\r') => continue,
            None => return Err(e.type_error("atob: invalid character")),
        };
        bits = (bits << 6) | v;
        nbits += 6;
        if nbits >= 8 {
            nbits -= 8;
            out.push((bits >> nbits) as u8);
        }
    }
    // `atob` rend une **chaîne-octet** (Latin-1, un code point
    // par octet — spec), pas de l'UTF-8 : la validation
    // `from_utf8` jetait `invalid byte sequence` sur toute
    // donnée non-ASCII (bsky bundle Sentry/expo vol22).
    let s: String = out.iter().map(|&b| char::from(b)).collect();
    Ok(e.str_val(s))
}
