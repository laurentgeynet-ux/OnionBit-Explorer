// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Builtins JS — section extraite de `builtins.rs` (JS-C40, pur
//! reformatage mécanique : sémantique et tests inchangés).

#[allow(clippy::wildcard_imports)]
use super::*;

// ------------------------------------------------------------------
// Function
// ------------------------------------------------------------------

pub(crate) fn function_call(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    e.call_value(this, arg(args, 0), &args[1.min(args.len())..])
}

pub(crate) fn function_apply(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let this_arg = arg(args, 0);
    let arr = arg(args, 1);
    // `CreateListFromArrayLike` (ES §) : `undefined`/`null` →
    // liste vide ; tout objet passe par `length` + `[[Get]]`
    // par indice (`arguments`, NodeList, `{length:2,…}`) ;
    // non-objet → TypeError. Bornes identiques à
    // `this_array_like` (`{length:1e9}` hostile → tick).
    let arr_args = match arr {
        Value::Undefined | Value::Null => Vec::new(),
        Value::Obj(id) => {
            if let Some(ObjKind::Array(elems)) = e.heap.obj(id).map(|o| &o.kind) {
                elems.clone()
            } else {
                let len = match e.get_prop_value(arr, "length")? {
                    Value::Num(n) => (n.max(0.0) as usize).min(1_000_000),
                    _ => 0,
                };
                let mut out = Vec::with_capacity(len.min(65_536));
                for i in 0..len {
                    out.push(e.get_prop_value(arr, &i.to_string())?);
                    if i % 8192 == 0 {
                        e.tick()?;
                    }
                }
                out
            }
        }
        _ => return Err(e.type_error("CreateListFromArrayLike called on non-object")),
    };
    e.call_value(this, this_arg, &arr_args)
}

pub(crate) fn function_to_string(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let out = match this {
        Value::Obj(id) => match e.heap.obj(id).map(|o| &o.kind) {
            Some(ObjKind::Fun(crate::heap::Fun::Script { lit, .. })) => {
                let name = lit.name.clone().unwrap_or_default();
                // Paramètres reconstruits depuis les patterns
                // (approximation — la spec rend le texte source ;
                // patterns complexes → `…`).
                let params: Vec<String> = lit
                    .params
                    .iter()
                    .map(|p| match &p.pat {
                        crate::ast::Pat::Ident(n) if p.rest => format!("...{n}"),
                        crate::ast::Pat::Ident(n) => n.clone(),
                        crate::ast::Pat::Default(b, _) if !p.rest => match b.as_ref() {
                            crate::ast::Pat::Ident(n) => format!("{n}=…"),
                            _ => "…".to_string(),
                        },
                        _ => "…".to_string(),
                    })
                    .collect();
                if lit.is_arrow {
                    format!("({}) => {{ [code] }}", params.join(", "))
                } else {
                    // Spec : `function*`, `async function` et
                    // `async function*` gardent leur forme source.
                    let kw = match (lit.is_async, lit.is_gen) {
                        (false, false) => "function",
                        (false, true) => "function*",
                        (true, false) => "async function",
                        (true, true) => "async function*",
                    };
                    format!("{kw} {name}({}) {{ [code] }}", params.join(", "))
                }
            }
            // Fonctions liées → `function () { [native code] }`
            // (spec Function.prototype.toString — source absente).
            Some(ObjKind::Fun(crate::heap::Fun::Bound { .. })) => {
                "function () { [native code] }".to_string()
            }
            Some(ObjKind::Fun(
                crate::heap::Fun::Native { name, .. } | crate::heap::Fun::BoundNative { name, .. },
            )) => format!("function {name}() {{ [native code] }}"),
            _ => "function () { [code] }".to_string(),
        },
        _ => "function () { [code] }".to_string(),
    };
    Ok(e.str_val(out))
}
