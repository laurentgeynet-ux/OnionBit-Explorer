// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

#[allow(clippy::wildcard_imports)]
use super::*;
use crate::ast::{AssignOp, Expr, ObjProp, Pat};
use crate::heap::{Heap, ObjKind};
use crate::value::{JsStr, Value};

/// `bigint == number` — égalité mathématique (spec `IsLooselyEqual`
/// étape : le `number` doit être fini et entier).
pub(crate) fn big_eq_num(b: &num_bigint::BigInt, n: f64) -> bool {
    if !n.is_finite() || n.fract() != 0.0 {
        return false;
    }
    num_traits::FromPrimitive::from_f64(n).is_some_and(|nb| b == &nb)
}

/// `bigint <op> number` — comparaison mathématique exacte :
/// `None` si `n` est `NaN`. Pour `n` fractionnaire, le bigint
/// (entier) est `Less` ssi `b <= floor(n)`, `Greater` ssi
/// `b >= ceil(n)` — il ne peut jamais égaler.
pub(crate) fn big_cmp_num(b: &num_bigint::BigInt, n: f64) -> Option<std::cmp::Ordering> {
    if n.is_nan() {
        return None;
    }
    if n.is_infinite() {
        return Some(if n < 0.0 {
            std::cmp::Ordering::Greater
        } else {
            std::cmp::Ordering::Less
        });
    }
    let f = n.floor();
    let lo: num_bigint::BigInt = num_traits::FromPrimitive::from_f64(f)?;
    match b.cmp(&lo) {
        std::cmp::Ordering::Less => Some(std::cmp::Ordering::Less),
        std::cmp::Ordering::Greater => Some(std::cmp::Ordering::Greater),
        std::cmp::Ordering::Equal => {
            if n.fract() == 0.0 {
                Some(std::cmp::Ordering::Equal)
            } else {
                Some(std::cmp::Ordering::Greater) // b == floor(n) < n
            }
        }
    }
}

/// `===` (no user code runs).
pub(crate) fn strict_eq(l: &Value, r: &Value, heap: &Heap) -> bool {
    let (l, r) = (l.unhole(), r.unhole());
    match (&l, &r) {
        (Value::Num(a), Value::Num(b)) => a == b, // NaN != NaN via f64 ==
        (Value::Str(a), Value::Str(b)) => {
            // `JsStr` : égalité de contenu — fast-path même
            // boîte `Rc` d'abord (cache `num_strs`, aliasing).
            JsStr::ptr_eq(a, b) || a == b
        }
        (Value::Big(a), Value::Big(b)) => {
            if a == b {
                true
            } else {
                matches!(
                    (heap.obj(*a).map(|o| &o.kind), heap.obj(*b).map(|o| &o.kind)),
                    (Some(ObjKind::BigInt(x)), Some(ObjKind::BigInt(y))) if x == y
                )
            }
        }
        _ => l == r,
    }
}

/// `[[OwnPropertyKeys]]` order : indices-entiers croissants, puis
/// strings par ordre d'insertion, puis symboles (clés sentinelle
/// privée) par insertion. Les `props` du heap gardant l'ordre
/// d'insertion brut, on partitionne ici.
pub(crate) fn spec_key_order(keys: Vec<String>) -> Vec<String> {
    let mut ints: Vec<(usize, String)> = Vec::new();
    let mut strs = Vec::new();
    let mut syms = Vec::new();
    for k in keys {
        if let Some(i) = array_index(&k) {
            ints.push((i, k));
        } else if k.starts_with('\u{f130}') {
            // Toutes les clés-symboles (`Symbol.*` well-knowns ET
            // anonymes `sym{N}`) après les strings (ordre spec
            // `[[OwnPropertyKeys]]`, §B-57).
            syms.push(k);
        } else {
            strs.push(k);
        }
    }
    ints.sort_by_key(|(i, _)| *i);
    ints.into_iter()
        .map(|(_, k)| k)
        .chain(strs)
        .chain(syms)
        .collect()
}

pub(crate) fn typeof_str(v: Value, heap: &Heap) -> &'static str {
    match v {
        Value::Undefined | Value::Hole => "undefined",
        Value::Null => "object",
        Value::Bool(_) => "boolean",
        Value::Num(_) => "number",
        Value::Str(_) => "string",
        Value::Big(_) => "bigint",
        Value::Obj(mut id) => loop {
            match heap.obj(id).map(|o| &o.kind) {
                Some(ObjKind::Fun(_)) => break "function",
                Some(ObjKind::Symbol { .. }) => break "symbol",
                // `typeof proxy` = `typeof target` (révoqué → object :
                // la spec lève TypeError — approximation documentée).
                Some(ObjKind::Proxy { target, handler }) => match (handler.is_some(), target) {
                    (true, Value::Obj(tid)) => id = *tid,
                    _ => break "object",
                },
                _ => break "object",
            }
        },
    }
}

/// `ToNumber` on strings: trim, hex, `Infinity`, decimal.
pub(crate) fn parse_js_number(s: &str) -> f64 {
    let t = s.trim();
    if t.is_empty() {
        return 0.0;
    }
    let (sign, rest) = match t.strip_prefix('-') {
        Some(r) => (-1.0, r),
        None => (1.0, t.strip_prefix('+').unwrap_or(t)),
    };
    if rest.eq_ignore_ascii_case("infinity") {
        return sign * f64::INFINITY;
    }
    for (p, base) in [
        ("0x", 16),
        ("0X", 16),
        ("0o", 8),
        ("0O", 8),
        ("0b", 2),
        ("0B", 2),
    ] {
        if let Some(d) = rest.strip_prefix(p) {
            return u64::from_str_radix(d, base).map_or(f64::NAN, |v| sign * v as f64);
        }
    }
    rest.parse::<f64>().map_or(f64::NAN, |v| sign * v)
}

/// Is `key` a canonical array index (`"0"`, `"1"`, … no leading zeros)?
/// JS-C539 — `#[track_caller]` : `j17_arridx_probe` attribue
/// l'essai au callsite (`arridx:` du snapshot).
#[track_caller]
pub(crate) fn array_index(key: &str) -> Option<usize> {
    crate::j17_arridx_probe(std::panic::Location::caller());
    if key.is_empty() || !key.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    if key.len() > 1 && key.starts_with('0') {
        return None;
    }
    // Spec : un index de tableau est < 2**32-1 — `"4294967295"`
    // et au-delà sont des props nommées ordinaires (JS-C613).
    key.parse::<usize>()
        .ok()
        .filter(|&i| i < u32::MAX as usize)
}

/// Spec `CanonicalNumericIndexString` (JS-C606) : `key` est la
/// forme canonique de sa valeur numérique — `ToString(ToNumber(P))
/// == P` — `"-0"` étant canonique (index −0, jamais valide).
/// `Some` pour `"0"`, `"1.5"`, `"NaN"`, `"Infinity"`, `"-1"` ;
/// `None` pour `"01"`, `"+1"`, `" 1"`, `"1e1"`, `"0x10"` — ces
/// clés relèvent des props ordinaires d'une vue typée.
#[track_caller]
pub(crate) fn canonical_numeric_index(key: &str) -> Option<f64> {
    crate::j17_arridx_probe(std::panic::Location::caller());
    // Gate premier octet : la forme canonique d'un nombre ne peut
    // commencer que par chiffre | '-' | '+' | '.' | 'I' | 'N'.
    let b = *key.as_bytes().first()?;
    if !(b.is_ascii_digit() || matches!(b, b'-' | b'+' | b'.' | b'I' | b'N')) {
        return None;
    }
    if key == "-0" {
        return Some(-0.0);
    }
    let n = parse_js_number(key);
    if crate::value::num_to_js_string(n) == key {
        Some(n)
    } else {
        None
    }
}

/// `TypedArrayLength` spec (JS-C606) — nombre d'éléments
/// **visibles** d'une vue, recalculé sur la longueur courante
/// du buffer : vue `auto` → `(bufLen − offset)/size` ; vue
/// fixed → `len`, sauf si la région déclarée déborde le buffer
/// (rétréci ou détaché) → vue hors-bornes → `0`.
pub(crate) fn ta_live_len(
    heap: &Heap,
    ta: crate::heap::TaKind,
    buffer: &Value,
    offset: usize,
    len: usize,
    auto: bool,
) -> usize {
    let Value::Obj(bid) = buffer else { return 0 };
    let Some(ObjKind::ByteBuf(bytes)) = heap.obj(*bid).map(|b| &b.kind) else {
        return 0;
    };
    let avail = bytes.len().saturating_sub(offset);
    if auto {
        avail / ta.size()
    } else if len.saturating_mul(ta.size()) <= avail {
        len
    } else {
        0
    }
}

/// `IsValidIntegerIndex` **vivant** (spec — buffer détaché ou
/// RAB rétréci rend l'index invalide même s'il reste `< len`) :
/// entier canonique ≥ +0 strictement sous [`ta_live_len`].
pub(crate) fn ta_live_index(
    heap: &Heap,
    ta: crate::heap::TaKind,
    buffer: &Value,
    offset: usize,
    len: usize,
    auto: bool,
    n: f64,
) -> Option<usize> {
    if n.fract() != 0.0 || n.is_sign_negative() {
        return None;
    }
    let i = n as usize;
    (i < ta_live_len(heap, ta, buffer, offset, len, auto)).then_some(i)
}

/// Convert an lhs expression to a [`Pat`] for destructuring
/// assignments (`[a] = x`, `({p: q} = o)`). `None` when the
/// expression isn't a valid pattern — the caller falls back to a
/// plain-lvalue error.
pub(crate) fn expr_to_pat(e: &Expr) -> Option<Pat> {
    // JS-C400 — arête ∝ imbrication destructurante : l'appel
    // arrive depuis `eval_expr_inner`/`exec_stmt_inner`, déjà
    // dans un segment — `[[[[…]]]]` hostile débordait le reste
    // de segment sans jamais pouvoir re-basculer (§B-111).
    stacker::maybe_grow(
        crate::parser::STACK_RED_ZONE,
        crate::parser::STACK_GROW,
        || expr_to_pat_inner(e),
    )
}

fn expr_to_pat_inner(e: &Expr) -> Option<Pat> {
    match e {
        Expr::Ident(name) => Some(Pat::Ident(name.clone())),
        // `o.p`/`a[i]` destructuring targets assign through the
        // existing lvalue path.
        Expr::Member(..) | Expr::Index(..) => Some(Pat::Lvalue(e.clone())),
        Expr::Spread(inner) => expr_to_pat(inner).map(|p| Pat::Rest(Box::new(p))),
        Expr::Assign(l, AssignOp::Set, d) => {
            expr_to_pat(l).map(|p| Pat::Default(Box::new(p), d.clone()))
        }
        Expr::Array(elems) => {
            let mut out = Vec::with_capacity(elems.len());
            for el in elems {
                match el {
                    None => out.push(None),
                    Some(e) => out.push(Some(expr_to_pat(e)?)),
                }
            }
            Some(Pat::Array(out))
        }
        Expr::Object(props) => {
            let mut pats = Vec::with_capacity(props.len());
            let mut rest = None;
            for p in props {
                match p {
                    ObjProp::Prop(k, ve) => pats.push((k.clone(), expr_to_pat(ve)?)),
                    ObjProp::Spread(e) => rest = Some(Box::new(expr_to_pat(e)?)),
                    ObjProp::Getter(..) | ObjProp::Setter(..) => return None,
                }
            }
            Some(Pat::Object { props: pats, rest })
        }
        _ => None,
    }
}

/// `obj[Symbol.iterator]()` partagé des host objects créés par
/// `Engine::new_iterable_host` (JS-C76) : relit
/// `Host::iter_items` à chaque appel (collection vivante) et rend
/// un `%ArrayIterator%` sur le snapshot. `undefined` si `this`
/// n'est pas un host itérable — ne devrait pas arriver (la native
/// n'est posée qu'en own-prop des hôtes itérables).
pub(crate) fn host_sym_iterator(e: &mut Engine, t: Value, _args: &[Value]) -> JsResult<Value> {
    if let Value::Obj(id) = t {
        if let Some(h) = e.host_of(id) {
            crate::host_mark(h.class_name(), "iter_items");
            if let Some(items) = h.iter_items(e)? {
                let proto = e.protos.array_iter;
                return Ok(crate::collections::iter_obj(e, items, proto));
            }
        }
    }
    Ok(Value::Undefined)
}
