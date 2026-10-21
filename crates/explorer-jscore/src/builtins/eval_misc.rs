// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Builtins JS — section extraite de `builtins.rs` (JS-C40, pur
//! reformatage mécanique : sémantique et tests inchangés).

#[allow(clippy::wildcard_imports)]
use super::*;

// ------------------------------------------------------------------
// JS-7f — eval/Function/bind, Symbol, Reflect, misc builtins
// ------------------------------------------------------------------

/// `eval(src)` — *indirect* eval semantics: the source runs in the
/// global scope (direct eval reaching the caller's locals is a
/// documented deviation — the tree-walker's env chain isn't
/// re-enterable mid-call). Non-string args return as-is (spec).
pub(crate) fn eval_fn(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    match arg(args, 0) {
        Value::Str(id) => {
            let src = e.heap.string(id).unwrap_or_default();
            e.eval_source(&src)
        }
        v => Ok(v),
    }
}

/// `Function("a","b","body")` — parses `function anonymous(a,b){body}`
/// and binds it to the *global* env (spec: `Function` functions are
/// never closures).
pub(crate) fn function_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let body = if args.is_empty() {
        String::new()
    } else {
        e.to_js_string(args[args.len() - 1])?
    };
    let mut params = String::new();
    for p in &args[..args.len().saturating_sub(1)] {
        if !params.is_empty() {
            params.push(',');
        }
        params.push_str(&e.to_js_string(*p)?);
    }
    let src = format!("function anonymous({params}) {{\n{body}\n}}");
    if let Some(h) = &e.eval_hook {
        h(&src);
    }
    let prog = crate::parser::parse_in(&src, false, e.nest_limit).map_err(|err| {
        crate::heap::Thrown(e.error_value(
            e.protos.syntax_error,
            "SyntaxError",
            &format!("Function: {err}"),
        ))
    })?;
    match prog.body.first() {
        Some(crate::ast::Stmt::FunctionDecl(lit)) => {
            let env = e.global_env.clone();
            Ok(e.make_function(lit, &env, Value::Undefined))
        }
        _ => Err(e.syntax_error("Function: unparseable source")),
    }
}

/// `fn.bind(thisArg, ...bound)` → `Fun::Bound` wrapper.
pub(crate) fn function_bind(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    if !e.is_callable(this) {
        return Err(e.type_error("bind called on non-callable"));
    }
    let bound_this = arg(args, 0);
    let bound = args.get(1..).unwrap_or(&[]).to_vec();
    let proto = Value::Obj(e.protos.function);
    let id = e.heap.alloc_obj(crate::heap::Object::fun(
        crate::heap::Fun::Bound {
            target: this,
            this: bound_this,
            bound: bound.clone(),
        },
        proto,
    ));
    // Spec `BoundFunctionCreate` : `length` =
    // max(0, target.length − nArgs liés), `name` =
    // "bound " + target.name — props `{w:false, c:true}`.
    let tl = match e.get_prop_value(this, "length")? {
        Value::Num(n) if n.is_finite() => n.max(0.0) as usize,
        _ => 0,
    };
    let len = tl.saturating_sub(bound.len());
    let tname = match e.get_prop_value(this, "name")? {
        v @ Value::Str(_) => e.to_js_string(v)?,
        _ => String::new(),
    };
    let nm = e.str_val(format!("bound {tname}"));
    let attrs = crate::heap::PropAttrs {
        writable: false,
        enumerable: false,
        configurable: true,
    };
    e.set_own_attrs(id, "length", Value::Num(len as f64), attrs);
    e.set_own_attrs(id, "name", nm, attrs);
    Ok(Value::Obj(id))
}

/// `Function.prototype[Symbol.hasInstance](v)` —
/// `OrdinaryHasInstance(this, v)` (bound → cible, `prototype` non
/// objet → `TypeError`). Partagé avec `instanceof` via l'engine.
pub(crate) fn function_has_instance(
    e: &mut Engine,
    this: Value,
    args: &[Value],
) -> JsResult<Value> {
    Ok(Value::Bool(e.ordinary_has_instance(this, arg(args, 0))?))
}

/// `Math.sumPrecise(iterable)` (ES2026) — somme compensée
/// (Neumaier/double-double) : la spec demande une précision
/// mathématique infinie arrondie en fin ; la compensation couvre
/// les cas usuels mieux que la somme naïve gauche→droite.
/// Non-itérable → `TypeError` (spec `ToNumber` sur chaque item).
pub(crate) fn math_sum_precise(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let items = match arg(args, 0) {
        Value::Undefined | Value::Null => {
            return Err(e.type_error("sumPrecise requires an iterable"));
        }
        v => e.iter_values(v)?,
    };
    let mut hi = 0.0f64;
    let mut lo = 0.0f64;
    for it in items {
        let x = e.to_number(it)?;
        if x.is_nan() {
            return Ok(Value::Num(f64::NAN));
        }
        let t = hi + x;
        if hi.abs() >= x.abs() {
            lo += (hi - t) + x;
        } else {
            lo += (x - t) + hi;
        }
        hi = t;
    }
    Ok(Value::Num(hi + lo))
}

/// Accesseur poison `caller`/`arguments` (spec
/// `AddRestrictedFunctionProperties`) — get/set → `TypeError`.
pub(crate) fn restricted_prop(e: &mut Engine, _t: Value, _a: &[Value]) -> JsResult<Value> {
    Err(e.type_error("caller/arguments are restricted function properties"))
}

// -- Symbol ---------------------------------------------------------

/// `Symbol(desc?)` — unique `ObjKind::Symbol` per call (identity by
/// `ObjId` ≙ spec identity semantics; prop keys stay strings, so
/// well-known symbols are the existing sentinels).
pub(crate) fn symbol_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let desc = match arg(args, 0) {
        Value::Undefined => None,
        v => Some(e.to_js_string(v)?),
    };
    Ok(e.new_symbol(desc))
}
