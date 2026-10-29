// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Builtins JS — section extraite de `builtins.rs` (JS-C40, pur
//! reformatage mécanique : sémantique et tests inchangés).

#[allow(clippy::wildcard_imports)]
use super::*;

// ------------------------------------------------------------------
// constructors
// ------------------------------------------------------------------

pub(crate) fn object_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    match arg(args, 0) {
        v @ Value::Obj(_) => Ok(v),
        _ => Ok(Value::Obj(e.new_object())),
    }
}

pub(crate) fn array_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    if args.len() == 1 {
        if let Value::Num(n) = args[0] {
            if n >= 0.0 && n.fract() == 0.0 {
                // `Array(n)` crée un tableau *épars* (n trous —
                // spec `ArrayCreate`), pas n `undefined`. Garde
                // d'alloc : `n` hostile doit refuser, pas avorter
                // le processus (JS-C55).
                e.alloc_guard((n as usize).saturating_mul(std::mem::size_of::<Value>()))?;
                return Ok(arr_of(e, vec![Value::Hole; n as usize]));
            }
            return Err(e.range_error("invalid array length"));
        }
    }
    Ok(arr_of(e, args.to_vec()))
}

/// Propriété cachée portant `[[PrimitiveValue]]` des wrappers
/// `new String/Number/Boolean` — sans elle, `valueOf`/`toString`
/// retomberaient sur `ToPrimitive(this)` → boucle infinie.
pub(crate) const PRIM: &str = "__prim";

/// `this` est-il l'objet frais d'un `new` sur le proto donné ?
/// Deux signaux : le proto exact (`new String()`…) ou
/// `ctor_fresh` — `construct_nt` marque l'objet alloué pour la
/// construction en cours, ce qui couvre
/// `Reflect.construct(String/Number/Boolean, args, newTarget)`
/// où le proto vient de `newTarget.prototype` (spec
/// `OrdinaryCreateFromConstructor` + slot interne initialisé —
/// le feature-detect `Reflect.construct` des helpers Babel
/// `_callSuper`/`_isNativeReflectConstruct` en dépend).
pub(crate) fn constructed_this(e: &Engine, this: Value, proto: ObjId) -> Option<ObjId> {
    let Value::Obj(id) = this else {
        return None;
    };
    if e.ctor_fresh == Some(id) {
        return Some(id);
    }
    match e.heap.obj(id).map(|o| o.proto) {
        Some(Value::Obj(p)) if p == proto => Some(id),
        _ => None,
    }
}

/// Pose `__prim` sur `this` et retourne `this` si `this` est un
/// wrapper construit sur `proto` ; sinon retourne `prim` (appel
/// fonctionnel — `String(x)` produit une primitive).
pub(crate) fn wrap_or_prim(e: &mut Engine, this: Value, proto: ObjId, prim: Value) -> Value {
    match constructed_this(e, this, proto) {
        Some(id) => {
            e.set_own(id, PRIM, prim, false);
            // `new String(s)` a `length` propre non-writable
            // (spec `StringCreate` — code units UTF-16).
            if proto == e.protos.string {
                if let Value::Str(sid) = prim {
                    let n = e.heap.string(sid).map_or(0, |s| u16_len(&s));
                    e.set_own_attrs(
                        id,
                        "length",
                        Value::Num(n as f64),
                        crate::heap::PropAttrs {
                            writable: false,
                            enumerable: false,
                            configurable: false,
                        },
                    );
                }
            }
            this
        }
        None => prim,
    }
}

pub(crate) fn string_ctor(e: &mut Engine, t: Value, args: &[Value]) -> JsResult<Value> {
    match arg(args, 0) {
        Value::Undefined => {
            let prim = e.str_val("");
            Ok(wrap_or_prim(e, t, e.protos.string, prim))
        }
        v => {
            let s = if let Some(d) = sym_desc(e, v) {
                if d.is_empty() {
                    "Symbol()".to_string()
                } else {
                    format!("Symbol({d})")
                }
            } else {
                e.to_js_string(v)?
            };
            let prim = e.str_val(s);
            Ok(wrap_or_prim(e, t, e.protos.string, prim))
        }
    }
}

pub(crate) fn number_ctor(e: &mut Engine, t: Value, args: &[Value]) -> JsResult<Value> {
    match arg(args, 0) {
        Value::Undefined => Ok(wrap_or_prim(e, t, e.protos.number, Value::Num(0.0))),
        // `Number(1n)` — `ToNumeric` autorise le bigint (troncation
        // `to_f64` documentée pour |b| > 2^53).
        Value::Big(_) => {
            let n = e
                .bigint_of(arg(args, 0))
                .and_then(|b| num_traits::ToPrimitive::to_f64(&b))
                .unwrap_or(f64::NAN);
            Ok(wrap_or_prim(e, t, e.protos.number, Value::Num(n)))
        }
        v => {
            let n = e.to_number(v)?;
            Ok(wrap_or_prim(e, t, e.protos.number, Value::Num(n)))
        }
    }
}

pub(crate) fn boolean_ctor(e: &mut Engine, t: Value, args: &[Value]) -> JsResult<Value> {
    let b = Value::Bool(e.to_boolean(arg(args, 0)));
    Ok(wrap_or_prim(e, t, e.protos.boolean, b))
}

/// `this` en tant que primitive String — wrapper `new String` via
/// `__prim`, primitive directe, sinon `ToString(this)`. Ne boucle
/// jamais : les méthodes strictes (`toString`/`valueOf`) lisent
/// `__prim` ou jettent.
pub(crate) fn this_prim(e: &Engine, this: Value) -> Option<Value> {
    match this {
        Value::Obj(id) => e.heap.obj(id).and_then(|o| o.get_own(PRIM)),
        v => Some(v),
    }
}

/// `this` strict pour `String.prototype.{toString,valueOf}` :
/// primitive string ou wrapper — `TypeError` sinon (spec
/// `thisStringValue`, évite aussi la récursion ToString→toString).
pub(crate) fn this_string_strict(e: &mut Engine, this: Value) -> JsResult<Value> {
    match this_prim(e, this) {
        Some(v @ Value::Str(_)) => Ok(v),
        _ => Err(e.type_error("String.prototype method called on incompatible receiver")),
    }
}

pub(crate) fn make_named_error(
    e: &mut Engine,
    proto: ObjId,
    name: &str,
    args: &[Value],
) -> JsResult<Value> {
    let msg = match arg(args, 0) {
        Value::Undefined => String::new(),
        v => e.to_js_string(v)?,
    };
    let err = e.error_value(proto, name, &msg);
    // `new Error(msg, {cause})` → `err.cause` (own, non-énumérable).
    if let (Value::Obj(eid), Value::Obj(opt)) = (err, arg(args, 1)) {
        if let Some(c) = e.get_prop(opt, "cause") {
            e.set_own(eid, "cause", c, false);
        }
    }
    Ok(err)
}

/// `AggregateError(errors, message?, options?)` — iterable `errors`
/// matérialisé en tableau propre (spec `CreateArrayFromList`-like).
pub(crate) fn aggregate_error_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let errs = match arg(args, 0) {
        Value::Undefined | Value::Null => Vec::new(),
        v => e.iter_values(v)?,
    };
    let list = e.new_array(errs);
    let err = make_named_error(e, e.protos.aggregate_error, "AggregateError", &args[1..])?;
    if let Value::Obj(eid) = err {
        e.set_own(eid, "errors", list, false);
    }
    Ok(err)
}

/// `Error.isError(v)` (ES2026) — spec : slot interne
/// `[[ErrorData]]`. Approximation documentée : vrai si la chaîne de
/// prototypes de `v` atteint `Error.prototype` (couvre `Error`,
/// `TypeError`, `AggregateError`, sous-classes — un objet
/// utilisateur avec `proto = Error.prototype` est indistinguable
/// chez nous).
pub(crate) fn error_is_error(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let mut cur = match arg(args, 0) {
        Value::Obj(id) => match e.heap.obj(id).map(|o| o.proto) {
            Some(Value::Obj(p)) => Some(p),
            _ => return Ok(Value::Bool(false)),
        },
        _ => return Ok(Value::Bool(false)),
    };
    let target = e.protos.error;
    let mut depth = 0;
    while let Some(id) = cur {
        if id == target {
            return Ok(Value::Bool(true));
        }
        cur = match e.heap.obj(id).map(|o| o.proto) {
            Some(Value::Obj(p)) => Some(p),
            _ => None,
        };
        depth += 1;
        if depth > 1000 {
            break;
        }
    }
    Ok(Value::Bool(false))
}

pub(crate) fn error_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let p = e.protos.error;
    make_named_error(e, p, "Error", args)
}

pub(crate) fn type_error_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let p = e.protos.type_error;
    make_named_error(e, p, "TypeError", args)
}

pub(crate) fn reference_error_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let p = e.protos.reference_error;
    make_named_error(e, p, "ReferenceError", args)
}

pub(crate) fn syntax_error_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let p = e.protos.syntax_error;
    make_named_error(e, p, "SyntaxError", args)
}

/// `EvalError`/`RangeError`/`URIError` — dev.to énumère
/// `%EvalError%` dans sa table d'intrinsèques (JS-C56 vague 2).
pub(crate) fn eval_error_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let p = e.protos.eval_error;
    make_named_error(e, p, "EvalError", args)
}

pub(crate) fn range_error_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let p = e.protos.range_error;
    make_named_error(e, p, "RangeError", args)
}

pub(crate) fn uri_error_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let p = e.protos.uri_error;
    make_named_error(e, p, "URIError", args)
}

pub(crate) fn date_now(_e: &mut Engine, _t: Value, _args: &[Value]) -> JsResult<Value> {
    Ok(Value::Num(now_millis()))
}

pub(crate) fn date_get_time(e: &mut Engine, this: Value, _args: &[Value]) -> JsResult<Value> {
    match this {
        Value::Obj(id) => Ok(e.get_prop(id, "__time").unwrap_or(Value::Num(f64::NAN))),
        _ => Err(e.type_error("Date method on non-Date")),
    }
}

pub(crate) fn date_to_string(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let ms = date_ms(e, this)?;
    Ok(e.str_val(if ms.is_finite() {
        let (y, mo, d, wd, h, mi, sec, _m) = ms_to_parts(ms);
        // Format spec `DateString`/`TimeString` (fuseau UTC fixe) :
        // "Thu Jan 01 1970 00:00:00 GMT+0000 (Coordinated Universal Time)".
        format!(
            "{} {} {:02} {:04} {:02}:{:02}:{:02} GMT+0000 (Coordinated Universal Time)",
            WD_NAMES[wd as usize],
            MO_NAMES[(mo - 1) as usize],
            d,
            y,
            h,
            mi,
            sec
        )
    } else {
        "Invalid Date".into()
    }))
}
