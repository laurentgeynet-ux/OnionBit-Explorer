// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Builtins JS — section extraite de `builtins.rs` (JS-C40, pur
//! reformatage mécanique : sémantique et tests inchangés).

#[allow(clippy::wildcard_imports)]
use super::*;

// ------------------------------------------------------------------
// Math
// ------------------------------------------------------------------

macro_rules! math1 {
    ($name:ident, $f:expr) => {
        pub(crate) fn $name(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
            let f: fn(f64) -> f64 = $f;
            Ok(Value::Num(f(e.to_number(arg(args, 0))?)))
        }
    };
}

math1!(math_abs, f64::abs);
math1!(math_floor, f64::floor);
math1!(math_ceil, f64::ceil);
math1!(math_sqrt, f64::sqrt);
math1!(math_exp, f64::exp);
math1!(math_log, f64::ln);
math1!(math_sin, f64::sin);
math1!(math_cos, f64::cos);
math1!(math_tan, f64::tan);
math1!(math_atan, f64::atan);

pub(crate) fn math_round(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    // JS rounds half up (toward +∞).
    Ok(Value::Num((e.to_number(arg(args, 0))? + 0.5).floor()))
}

pub(crate) fn math_max(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let mut m = f64::NEG_INFINITY;
    for a in args {
        let n = e.to_number(*a)?;
        if n.is_nan() {
            return Ok(Value::Num(f64::NAN));
        }
        m = m.max(n);
    }
    Ok(Value::Num(if args.is_empty() {
        f64::NEG_INFINITY
    } else {
        m
    }))
}

pub(crate) fn math_min(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let mut m = f64::INFINITY;
    for a in args {
        let n = e.to_number(*a)?;
        if n.is_nan() {
            return Ok(Value::Num(f64::NAN));
        }
        m = m.min(n);
    }
    Ok(Value::Num(if args.is_empty() { f64::INFINITY } else { m }))
}

pub(crate) fn math_pow(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let b = e.to_number(arg(args, 0))?;
    let x = e.to_number(arg(args, 1))?;
    Ok(Value::Num(b.powf(x)))
}

pub(crate) fn math_atan2(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let y = e.to_number(arg(args, 0))?;
    let x = e.to_number(arg(args, 1))?;
    Ok(Value::Num(y.atan2(x)))
}

pub(crate) fn math_random(e: &mut Engine, _t: Value, _a: &[Value]) -> JsResult<Value> {
    // xorshift64* — fine for page scripts, not crypto.
    e.rng ^= e.rng >> 12;
    e.rng ^= e.rng << 25;
    e.rng ^= e.rng >> 27;
    let v = e.rng.wrapping_mul(0x2545_f491_4f6c_dd1d);
    Ok(Value::Num((v >> 11) as f64 / 9_007_199_254_740_992.0))
}
