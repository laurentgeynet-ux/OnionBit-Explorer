// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Core JS value representation and number formatting.

/// Handle into the `Heap` arena (ADR-0005-style:
/// objects reference each other by id, never by pointer).
pub type ObjId = u32;

/// A JavaScript value. `Copy` because every indirection is an `ObjId`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Value {
    /// `undefined`
    Undefined,
    /// `null`
    Null,
    /// Boolean.
    Bool(bool),
    /// IEEE-754 double.
    Num(f64),
    /// Heap-allocated string.
    Str(ObjId),
    /// `bigint` primitive — `ObjId` of an `ObjKind::BigInt` cell
    /// (primitive semantics : `1n === 1n`, never a `[[Prototype]]`
    /// chain — the id only carries the digits).
    Big(ObjId),
    /// Heap-allocated object.
    Obj(ObjId),
    /// Élément-trou de tableau épars (`Array(3)`, `delete a[i]`,
    /// `[1,,3]`). Jamais observable directement — `find_own` rend
    /// `None` (spec `HasProperty`) et les chemins génériques le
    /// traitent comme `undefined` ; les méthodes qui *sautent*
    /// les trous (`forEach`/`map`/…) testent `is_hole`.
    Hole,
}

impl Value {
    /// Whether the value is object-like (heap `Obj`).
    #[must_use]
    pub fn is_object(self) -> bool {
        matches!(self, Value::Obj(_))
    }

    /// Trou de tableau épars.
    #[must_use]
    pub fn is_hole(self) -> bool {
        matches!(self, Value::Hole)
    }

    /// `Hole → Undefined` — lecture d'un trou (itérateurs,
    /// valeurs retournées par `pop`/`at`/spread…).
    #[must_use]
    pub fn unhole(self) -> Value {
        if self.is_hole() {
            Value::Undefined
        } else {
            self
        }
    }
}

/// `ToString(number)` — mirrors the JS rules: `NaN`, `Infinity`,
/// plain decimals, exponential form outside `[1e-6, 1e21)`.
pub fn num_to_js_string(n: f64) -> String {
    if n.is_nan() {
        return "NaN".into();
    }
    if n.is_infinite() {
        return if n < 0.0 { "-Infinity" } else { "Infinity" }.into();
    }
    if n == 0.0 {
        return "0".into();
    }
    let a = n.abs();
    if !(1e-6..1e21).contains(&a) {
        let s = format!("{n:e}");
        // Rust prints "1e21"/"1.5e-7"; JS wants "1e+21"/"1.5e-7".
        if let Some(i) = s.find('e') {
            let (mant, exp) = s.split_at(i + 1);
            let exp = &exp[0..];
            if exp.starts_with('-') {
                return format!("{mant}{exp}");
            }
            return format!("{mant}+{exp}");
        }
        return s;
    }
    if n.fract() == 0.0 {
        return format!("{n:.0}");
    }
    format!("{n}")
}

/// `ToInt32` — truncate then reduce modulo 2³².
pub fn to_int32(n: f64) -> i32 {
    if !n.is_finite() || n == 0.0 {
        return 0;
    }
    let m = n.trunc() % 4_294_967_296.0;
    let m = if m < 0.0 { m + 4_294_967_296.0 } else { m };
    // m ∈ [0, 2³²)
    if m >= 2_147_483_648.0 {
        (m - 4_294_967_296.0) as i64 as i32
    } else {
        m as i64 as i32
    }
}

/// `ToUint32`.
pub fn to_uint32(n: f64) -> u32 {
    to_int32(n) as u32
}
