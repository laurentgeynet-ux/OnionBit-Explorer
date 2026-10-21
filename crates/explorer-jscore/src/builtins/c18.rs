// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Builtins JS — section extraite de `builtins.rs` (JS-C40, pur
//! reformatage mécanique : sémantique et tests inchangés).

#[allow(clippy::wildcard_imports)]
use super::*;

// ------------------------------------------------------------------
// JS-C18 — Annex B complétion + Number/Atomics
// ------------------------------------------------------------------

/// `Number.isSafeInteger(v)` — entier dans ]-2^53, 2^53[.
pub(crate) fn number_is_safe_integer(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let ok = match arg(args, 0) {
        Value::Num(n) => n.is_finite() && n.fract() == 0.0 && n.abs() < 9_007_199_254_740_992.0,
        _ => false,
    };
    let _ = e;
    Ok(Value::Bool(ok))
}

/// `d.getYear()` (Annex B) — `getFullYear() - 1900`.
pub(crate) fn date_get_year(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let y = date_part(e, this, 0)?;
    Ok(Value::Num(if y.is_nan() { y } else { y - 1900.0 }))
}

/// `d.setYear(y)` (Annex B) — 0-99 → 1900+y ; conserve mois/jour/
/// heure ; retourne le nouveau timestamp (NaN → Invalid Date).
pub(crate) fn date_set_year(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let Value::Obj(id) = this else {
        return Err(e.type_error("Date method on non-Date"));
    };
    if e.get_prop(id, "__time").is_none() {
        return Err(e.type_error("Date method on non-Date"));
    }
    let y = e.to_number(arg(args, 0))?;
    let ms = e.get_prop(id, "__time").map_or(f64::NAN, |v| match v {
        Value::Num(n) => n,
        _ => f64::NAN,
    });
    let new_ms = if y.is_nan() {
        f64::NAN
    } else {
        let (oy, mo, d, _w, h, mi, s, mss) = if ms.is_finite() {
            ms_to_parts(ms)
        } else {
            (1970, 1, 1, 4, 0, 0, 0, 0) // date invalide → base epoch
        };
        let _ = oy;
        let yi = y.trunc() as i64;
        let full = if (0..=99).contains(&yi) {
            yi + 1900
        } else {
            yi
        };
        // `ms_to_parts` rend le mois 1-based ; `parts_to_ms`
        // attend le mois 0-based JS.
        parts_to_ms(
            full,
            i64::from(mo) - 1,
            i64::from(d),
            i64::from(h),
            i64::from(mi),
            i64::from(s),
            i64::from(mss),
        )
    };
    e.set_own(id, "__time", Value::Num(new_ms), false);
    Ok(Value::Num(new_ms))
}

/// `str.m()` Annex B — `<tag>m</tag>` ou `<tag attr="v">m</tag>`
/// (`"` échappé `&quot;` dans la valeur d'attribut, spec `CreateHTML`).
/// Bound args : `[tag, attr]` ; `attr` vide → pas d'attribut.
pub(crate) fn string_html_wrap(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let s = this_str(e, this)?;
    let tag = e.to_js_string(arg(args, 0))?;
    let attr = e.to_js_string(arg(args, 1))?;
    let out = if attr.is_empty() {
        format!("<{tag}>{s}</{tag}>")
    } else {
        let v = e.to_js_string(arg(args, 2))?.replace('"', "&quot;");
        format!("<{tag} {attr}=\"{v}\">{s}</{tag}>")
    };
    Ok(e.str_val(out))
}
