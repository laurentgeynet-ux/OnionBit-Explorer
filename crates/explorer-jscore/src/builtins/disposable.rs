// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Builtins JS — section extraite de `builtins.rs` (JS-C40, pur
//! reformatage mécanique : sémantique et tests inchangés).

#[allow(clippy::wildcard_imports)]
use super::*;

// ------------------------------------------------------------------
// JS-C17 — `DisposableStack` (ES2026)
// ------------------------------------------------------------------

/// Extrait la pile interne (vérifie `ObjKind::DispStack`).
pub(crate) fn ds_state(e: &mut Engine, this: Value) -> JsResult<ObjId> {
    match this {
        Value::Obj(id) => match e.heap.obj(id).map(|o| &o.kind) {
            Some(ObjKind::DispStack { .. }) => Ok(id),
            _ => Err(e.type_error("not a DisposableStack")),
        },
        _ => Err(e.type_error("not a DisposableStack")),
    }
}

/// `ReferenceError` sur pile déjà déposée (spec `dispose`/`use`).
pub(crate) fn ds_live(e: &mut Engine, id: ObjId) -> JsResult<()> {
    let disposed = match e.heap.obj(id).map(|o| &o.kind) {
        Some(ObjKind::DispStack { disposed, .. }) => *disposed,
        _ => true,
    };
    if disposed {
        Err(e.ref_error("DisposableStack is already disposed"))
    } else {
        Ok(())
    }
}

pub(crate) fn ds_push(e: &mut Engine, id: ObjId, f: Value, t: Value, v: Value) {
    if let Some(o) = e.heap.obj_mut(id) {
        if let ObjKind::DispStack { stack, .. } = &mut o.kind {
            stack.push((f, t, v));
        }
    }
}

pub(crate) fn disposable_stack_ctor(e: &mut Engine, _t: Value, _a: &[Value]) -> JsResult<Value> {
    let proto = e
        .get_prop(e.global, "DisposableStack")
        .and_then(|c| match c {
            Value::Obj(cid) => e.get_prop(cid, "prototype"),
            _ => None,
        })
        .unwrap_or(Value::Obj(e.protos.object));
    let mut o = crate::heap::Object::ordinary(proto);
    o.kind = ObjKind::DispStack {
        disposed: false,
        stack: Vec::new(),
    };
    Ok(Value::Obj(e.heap.alloc_obj(o)))
}

/// `get disposed` — `true` après `dispose()`/`move()`.
pub(crate) fn ds_disposed_get(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let id = ds_state(e, this)?;
    Ok(Value::Bool(ds_live(e, id).is_err()))
}

/// `use(v)` — enregistre `v[@@dispose]` ; `null`/`undefined`
/// passent sans enregistrement (spec).
pub(crate) fn ds_use(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let id = ds_state(e, this)?;
    ds_live(e, id)?;
    let v = arg(args, 0);
    if matches!(v, Value::Null | Value::Undefined) {
        return Ok(v);
    }
    let m = match v {
        Value::Obj(vid) => e.get_prop(vid, crate::interp::SYM_DISPOSE),
        _ => None,
    };
    match m {
        Some(f) if e.is_callable(f) => {
            // `v[@@dispose]()` — `this` = la ressource, pas d'arg.
            ds_push(e, id, f, v, Value::Undefined);
            Ok(v)
        }
        _ => Err(e.type_error("value has no [Symbol.dispose] method")),
    }
}

/// `adopt(v, f)` — disposeur arbitraire `f(v)`.
pub(crate) fn ds_adopt(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let id = ds_state(e, this)?;
    ds_live(e, id)?;
    let f = arg(args, 1);
    if !e.is_callable(f) {
        return Err(e.type_error("adopt onDispose is not callable"));
    }
    ds_push(e, id, f, Value::Undefined, arg(args, 0));
    Ok(arg(args, 0))
}

/// `defer(f)` — disposeur sans argument.
pub(crate) fn ds_defer(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let id = ds_state(e, this)?;
    ds_live(e, id)?;
    let f = arg(args, 0);
    if !e.is_callable(f) {
        return Err(e.type_error("defer callback is not callable"));
    }
    ds_push(e, id, f, Value::Undefined, Value::Undefined);
    Ok(Value::Undefined)
}

/// `dispose()` — LIFO ; première erreur propagée, les erreurs
/// suivantes enroulées en `SuppressedError(err, prev)` (spec).
pub(crate) fn ds_dispose(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let id = ds_state(e, this)?;
    ds_live(e, id)?;
    if let Some(o) = e.heap.obj_mut(id) {
        if let ObjKind::DispStack { disposed, .. } = &mut o.kind {
            *disposed = true;
        }
    }
    let mut pending: Option<Value> = None;
    loop {
        let next = match e.heap.obj_mut(id).map(|o| &mut o.kind) {
            Some(ObjKind::DispStack { stack, .. }) => stack.pop(),
            _ => None,
        };
        let Some((f, t, v)) = next else {
            break;
        };
        if let Err(Thrown(err)) = e.call_value(f, t, &[v]) {
            pending = Some(match pending {
                None => err,
                Some(prev) => make_suppressed(e, err, prev),
            });
        }
    }
    match pending {
        None => Ok(Value::Undefined),
        Some(err) => Err(Thrown(err)),
    }
}

/// `move()` — transfère la pile dans une nouvelle instance ;
/// l'originale est marquée déposée (vide).
pub(crate) fn ds_move(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let id = ds_state(e, this)?;
    ds_live(e, id)?;
    let fresh = disposable_stack_ctor(e, Value::Undefined, &[])?;
    let mut moved = Vec::new();
    if let Some(o) = e.heap.obj_mut(id) {
        if let ObjKind::DispStack { disposed, stack } = &mut o.kind {
            *disposed = true;
            moved = std::mem::take(stack);
        }
    }
    if let Value::Obj(nid) = fresh {
        if let Some(o) = e.heap.obj_mut(nid) {
            if let ObjKind::DispStack { stack, .. } = &mut o.kind {
                *stack = moved;
            }
        }
    }
    Ok(fresh)
}

// JS-C20 — `AsyncDisposableStack` (ES2026)
// ------------------------------------------------------------------

pub(crate) fn async_disposable_stack_ctor(
    e: &mut Engine,
    _t: Value,
    _a: &[Value],
) -> JsResult<Value> {
    let proto = e
        .get_prop(e.global, "AsyncDisposableStack")
        .and_then(|c| match c {
            Value::Obj(cid) => e.get_prop(cid, "prototype"),
            _ => None,
        })
        .unwrap_or(Value::Obj(e.protos.object));
    let mut o = crate::heap::Object::ordinary(proto);
    o.kind = ObjKind::DispStack {
        disposed: false,
        stack: Vec::new(),
    };
    Ok(Value::Obj(e.heap.alloc_obj(o)))
}

/// `use(v)` — enregistre `v[@@asyncDispose] ?? v[@@dispose]`
/// (spec `GetDisposeMethod(async)`).
pub(crate) fn ads_use(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let id = ds_state(e, this)?;
    ds_live(e, id)?;
    let v = arg(args, 0);
    if matches!(v, Value::Null | Value::Undefined) {
        return Ok(v);
    }
    let m = match v {
        Value::Obj(vid) => e
            .get_prop(vid, crate::interp::SYM_ASYNC_DISPOSE)
            .or_else(|| e.get_prop(vid, crate::interp::SYM_DISPOSE)),
        _ => None,
    };
    match m {
        Some(f) if e.is_callable(f) => {
            ds_push(e, id, f, v, Value::Undefined);
            Ok(v)
        }
        _ => Err(e.type_error("value has no [Symbol.asyncDispose] method")),
    }
}

/// `adopt(v, onDisposeAsync)` — disposeur `f(v)` awaité au dépile.
pub(crate) fn ads_adopt(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let id = ds_state(e, this)?;
    ds_live(e, id)?;
    let f = arg(args, 1);
    if !e.is_callable(f) {
        return Err(e.type_error("adopt onDisposeAsync is not callable"));
    }
    ds_push(e, id, f, Value::Undefined, arg(args, 0));
    Ok(arg(args, 0))
}

/// `defer(onDisposeAsync)` — disposeur sans argument.
pub(crate) fn ads_defer(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let id = ds_state(e, this)?;
    ds_live(e, id)?;
    let f = arg(args, 0);
    if !e.is_callable(f) {
        return Err(e.type_error("defer callback is not callable"));
    }
    ds_push(e, id, f, Value::Undefined, Value::Undefined);
    Ok(Value::Undefined)
}

/// `disposeAsync()` — dépile LIFO en awaitant chaque retour
/// (`await` eager — une promesse pending jette, déviation
/// documentée). Résultat : promesse résolue `undefined` ou
/// rejetée (`SuppressedError` enroulé, spec).
pub(crate) fn ads_dispose_async(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let id = ds_state(e, this)?;
    let pid = e.new_promise();
    let run = (|| -> JsResult<()> {
        ds_live(e, id)?;
        if let Some(o) = e.heap.obj_mut(id) {
            if let ObjKind::DispStack { disposed, .. } = &mut o.kind {
                *disposed = true;
            }
        }
        let mut pending: Option<Value> = None;
        loop {
            let next = match e.heap.obj_mut(id).map(|o| &mut o.kind) {
                Some(ObjKind::DispStack { stack, .. }) => stack.pop(),
                _ => None,
            };
            let Some((f, t, v)) = next else {
                break;
            };
            let r = e
                .call_value(f, t, &[v])
                .and_then(|out| e.await_value(out).map(|_| ()));
            if let Err(Thrown(err)) = r {
                pending = Some(match pending {
                    None => err,
                    Some(prev) => make_suppressed(e, err, prev),
                });
            }
        }
        match pending {
            None => Ok(()),
            Some(err) => Err(Thrown(err)),
        }
    })();
    match run {
        Ok(()) => e.resolve_promise(pid, Value::Undefined),
        Err(Thrown(err)) => e.reject_promise(pid, err),
    }
    Ok(Value::Obj(pid))
}

/// `move()` — nouvelle `AsyncDisposableStack` recevant la pile.
pub(crate) fn ads_move(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let id = ds_state(e, this)?;
    ds_live(e, id)?;
    let fresh = async_disposable_stack_ctor(e, Value::Undefined, &[])?;
    let mut moved = Vec::new();
    if let Some(o) = e.heap.obj_mut(id) {
        if let ObjKind::DispStack { disposed, stack } = &mut o.kind {
            *disposed = true;
            moved = std::mem::take(stack);
        }
    }
    if let Value::Obj(nid) = fresh {
        if let Some(o) = e.heap.obj_mut(nid) {
            if let ObjKind::DispStack { stack, .. } = &mut o.kind {
                *stack = moved;
            }
        }
    }
    Ok(fresh)
}

/// Construit un `SuppressedError` (partagé `dispose` + ctor).
pub(crate) fn make_suppressed(e: &mut Engine, err: Value, prev: Value) -> Value {
    let proto = e
        .get_prop(e.global, "SuppressedError")
        .and_then(|c| match c {
            Value::Obj(cid) => e.get_prop(cid, "prototype"),
            _ => None,
        })
        .unwrap_or(Value::Obj(e.protos.error));
    let pid = match proto {
        Value::Obj(p) => p,
        _ => e.protos.error,
    };
    let v = e.error_value(pid, "SuppressedError", "");
    if let Value::Obj(id) = v {
        e.set_own(id, "error", err, true);
        e.set_own(id, "suppressed", prev, true);
    }
    v
}
