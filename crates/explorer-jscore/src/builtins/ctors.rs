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
        // `Object()`/`Object(undefined)`/`Object(null)` → objet
        // ordinaire vide (spec `Object` function, §19.1.1.1).
        Value::Undefined | Value::Null => Ok(Value::Obj(e.new_object())),
        // §B-72 bras 2 (fifa davinci vol 504) : `Object(t)` sur
        // primitive = `ToObject` → wrapper du bon proto —
        // `Object('NA').constructor === String`, attendu par les
        // `assert`s proto2 des bundles Closure/libphonenumber.
        v => Ok(e.box_primitive(v)),
    }
}

pub(crate) fn array_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    if args.len() == 1 {
        if let Value::Num(n) = args[0] {
            if n >= 0.0 && n.fract() == 0.0 && n <= u32::MAX as f64 {
                // `Array(n)` crée un tableau *épars* (n trous —
                // spec `ArrayCreate`), pas n `undefined` : `len`
                // posée, zéro élément stocké (JS-C613 — plus de
                // `vec![Hole; n]` ni de garde d'alloc : rien ne
                // se matérialise).
                return Ok(e.new_array_len(n as u32));
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
    match e.heap.obj(id).map(|o| o.proto.clone()) {
        Some(Value::Obj(p)) if p == proto => Some(id),
        _ => None,
    }
}

/// Pose `__prim` sur `this` et retourne `this` si `this` est un
/// wrapper construit sur `proto` ; sinon retourne `prim` (appel
/// fonctionnel — `String(x)` produit une primitive).
pub(crate) fn wrap_or_prim(e: &mut Engine, this: Value, proto: ObjId, prim: Value) -> Value {
    match constructed_this(e, this.clone(), proto) {
        Some(id) => {
            e.set_own(id, PRIM, prim.clone(), false);
            // `new String(s)` a `length` propre non-writable
            // (spec `StringCreate` — code units UTF-16).
            if proto == e.protos.string {
                if let Value::Str(sid) = prim {
                    let n = sid.u16_len();
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
    // Spec : `String()` **sans argument** → `""`, mais
    // `String(undefined)` → `"undefined"` — la distinction
    // passe par l'arité (comme `Number(undefined)` → `NaN`,
    // JS-C618).
    if args.is_empty() {
        let prim = e.str_val("");
        return Ok(wrap_or_prim(e, t, e.protos.string, prim));
    }
    let v = arg(args, 0);
    let prim = if let Some(d) = sym_desc(e, v.clone()) {
        e.str_val(if d.is_empty() {
            "Symbol()".to_string()
        } else {
            format!("Symbol({d})")
        })
    } else {
        e.to_str_val(v)?
    };
    Ok(wrap_or_prim(e, t, e.protos.string, prim))
}

pub(crate) fn number_ctor(e: &mut Engine, t: Value, args: &[Value]) -> JsResult<Value> {
    // Spec : `Number()` **sans argument** → `+0`, mais
    // `Number(undefined)` → `ToNumber(undefined)` = `NaN` — la
    // distinction passe par l'arité, pas par la valeur de `arg`.
    if args.is_empty() {
        return Ok(wrap_or_prim(e, t, e.protos.number, Value::Num(0.0)));
    }
    match arg(args, 0) {
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
    if let (Value::Obj(eid), Value::Obj(opt)) = (err.clone(), arg(args, 1)) {
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
    let err = make_named_error(
        e,
        e.protos.aggregate_error,
        "AggregateError",
        args.get(1..).unwrap_or(&[]),
    )?;
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
        Value::Obj(id) => match e.heap.obj(id).map(|o| o.proto.clone()) {
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
        cur = match e.heap.obj(id).map(|o| o.proto.clone()) {
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

/// Table legacy `WebIDL` `name → code` (`DOMException`) : les codes
/// numériques historiques survivent via `e.code`/`DOMException.
/// DATA_CLONE_ERR` — les noms modernes hors table rendent 0.
pub(crate) fn dom_exception_code(name: &str) -> u32 {
    match name {
        "IndexSizeError" => 1,
        "DOMStringSizeError" => 2,
        "HierarchyRequestError" => 3,
        "WrongDocumentError" => 4,
        "InvalidCharacterError" => 5,
        "NoDataAllowedError" => 6,
        "NoModificationAllowedError" => 7,
        "NotFoundError" => 8,
        "NotSupportedError" => 9,
        "InUseAttributeError" => 10,
        "InvalidStateError" => 11,
        "SyntaxError" => 12,
        "InvalidModificationError" => 13,
        "NamespaceError" => 14,
        "InvalidAccessError" => 15,
        "ValidationError" => 16,
        "TypeMismatchError" => 17,
        "SecurityError" => 18,
        "NetworkError" => 19,
        "AbortError" => 20,
        "URLMismatchError" => 21,
        "QuotaExceededError" => 22,
        "TimeoutError" => 23,
        "InvalidNodeTypeError" => 24,
        "DataCloneError" => 25,
        _ => 0,
    }
}

/// `new DOMException(message?, name?)` — `{name, message, code,
/// stack}` own (`WebIDL` §`DOMException` ; `name` défaut `"Error"`).
/// Constructeur-only : appel sans `new` → `TypeError` spec.
pub(crate) fn dom_exception_ctor(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let id = match this {
        Value::Obj(id)
            if e.heap
                .obj(id)
                .is_some_and(|o| o.proto == Value::Obj(e.protos.dom_exception)) =>
        {
            id
        }
        _ => return Err(e.type_error("DOMException constructor requires 'new'")),
    };
    let message = match arg(args, 0) {
        Value::Undefined => String::new(),
        v => e.to_js_string(v)?,
    };
    let name = match arg(args, 1) {
        Value::Undefined => "Error".to_string(),
        v => e.to_js_string(v)?,
    };
    let m = e.str_val(message.clone());
    e.set_own(id, "message", m, false);
    let n = e.str_val(name.clone());
    e.set_own(id, "name", n, false);
    let code = dom_exception_code(&name);
    e.set_own(id, "code", Value::Num(f64::from(code)), false);
    // `stack` own non-énumérable capturé à la construction —
    // même convention que `error_value` (JS-C47).
    let s = e.str_val(e.stack_text(&name, &message));
    e.set_own(id, "stack", s, false);
    Ok(Value::Undefined)
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

/// `Date.prototype[@@toPrimitive](hint)` (spec 21.4.4.45) —
/// **exception Date** : `hint === "default"` est traité comme
/// `"string"` (→ `toString` puis `valueOf`) à l'inverse de
/// `OrdinaryToPrimitive` ; `"number"` → `valueOf` puis
/// `toString`. Autre hint → `TypeError`.
pub(crate) fn date_to_primitive(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let hint = e.to_js_string(arg(args, 0))?;
    let order: [&str; 2] = match hint.as_str() {
        "number" => ["valueOf", "toString"],
        "string" | "default" => ["toString", "valueOf"],
        _ => return Err(e.type_error("invalid hint")),
    };
    if let Value::Obj(id) = this.clone() {
        for n in order {
            if let Some(m) = e.get_prop(id, n) {
                if e.is_callable(m.clone()) {
                    let r = e.call_value(m, this.clone(), &[])?;
                    if !matches!(r, Value::Obj(_)) {
                        return Ok(r);
                    }
                }
            }
        }
    }
    Err(e.type_error("cannot convert Date to primitive"))
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
