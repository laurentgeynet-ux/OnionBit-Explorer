// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Builtins JS — section extraite de `builtins.rs` (JS-C40, pur
//! reformatage mécanique : sémantique et tests inchangés).

#[allow(clippy::wildcard_imports)]
use super::*;

// ------------------------------------------------------------------
// Number / Boolean / Error prototypes
// ------------------------------------------------------------------

pub(crate) fn number_to_string(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let n = this_num(e, this)?;
    let radix = match arg(args, 0) {
        Value::Undefined => 10,
        v => e.to_number(v)? as u32,
    };
    if radix == 10 || !(2..=36).contains(&radix) {
        return Ok(e.str_val(num_to_js_string(n)));
    }
    if !n.is_finite() {
        return Ok(e.str_val(num_to_js_string(n)));
    }
    let neg = n < 0.0;
    let a = n.abs();
    let mut frac = a.fract();
    let mut int = a.trunc() as u64;
    let mut digits = Vec::new();
    loop {
        digits.push(std::char::from_digit((int % u64::from(radix)) as u32, radix).unwrap_or('0'));
        int /= u64::from(radix);
        if int == 0 {
            break;
        }
    }
    let mut out: String = digits.iter().rev().collect();
    if frac > 0.0 {
        out.push('.');
        let mut i = 0;
        // 53 digits max (mantisse IEEE-754) — termine dès que la
        // fraction tombe à 0 (`(0.5).toString(2)` → "0.1").
        while frac > 0.0 && i < 53 {
            frac *= f64::from(radix);
            let d = frac.trunc() as u32;
            out.push(std::char::from_digit(d, radix).unwrap_or('0'));
            frac = frac.fract();
            i += 1;
        }
    }
    if neg {
        out.insert(0, '-');
    }
    Ok(e.str_val(out))
}

pub(crate) fn number_value_of(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    match this_prim(e, this) {
        Some(v @ Value::Num(_)) => Ok(v),
        _ => Err(e.type_error("Number.prototype.valueOf on incompatible receiver")),
    }
}

pub(crate) fn number_to_fixed(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let n = this_num(e, this)?;
    let d = match arg(args, 0) {
        Value::Undefined => 0usize,
        v => (e.to_number(v)? as i64).clamp(0, 20) as usize,
    };
    if !n.is_finite() {
        return Ok(e.str_val(num_to_js_string(n)));
    }
    // `-0` ne porte pas de signe (spec Number.prototype.toFixed
    // — Acid3 t84) ; Rust formate `-0.0` avec un `-`.
    let n = if n == 0.0 { 0.0 } else { n };
    Ok(e.str_val(format!("{n:.d$}")))
}

pub(crate) fn boolean_to_string(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    match this {
        Value::Bool(b) => Ok(e.str_val(if b { "true" } else { "false" })),
        _ => Err(e.type_error("Boolean method on non-boolean")),
    }
}

pub(crate) fn boolean_value_of(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    match this_prim(e, this) {
        Some(v @ Value::Bool(_)) => Ok(v),
        _ => Err(e.type_error("Boolean.prototype.valueOf on incompatible receiver")),
    }
}

pub(crate) fn error_to_string(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let (name, msg) = match this {
        Value::Obj(id) => {
            let n = e.get_prop(id, "name").map_or_else(
                || "Error".into(),
                |v| e.to_js_string(v).unwrap_or_else(|_| "Error".into()),
            );
            let m = e
                .get_prop(id, "message")
                .map(|v| e.to_js_string(v).unwrap_or_default())
                .unwrap_or_default();
            (n, m)
        }
        _ => ("Error".into(), String::new()),
    };
    Ok(e.str_val(if msg.is_empty() {
        name
    } else {
        format!("{name}: {msg}")
    }))
}
