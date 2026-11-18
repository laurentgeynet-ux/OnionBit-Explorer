// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Objet hôte `$262` du harness test262 (ADR-0010, JS-C607) —
//! installé **uniquement** à la demande ([`Engine::install_t262`]) :
//! les pages réelles ne doivent jamais voir ce global.
//!
//! Surface : `global`, `evalScript` (évaluation **script** dans
//! le realm courant), `createRealm` (realm nu à intrinsics frais
//! — [`Engine::context_new_bare`]), `detachArrayBuffer`.

use crate::heap::{JsResult, ObjKind};
use crate::value::Value;
use crate::Engine;

fn arg(args: &[Value], i: usize) -> Value {
    args.get(i).cloned().unwrap_or(Value::Undefined)
}

impl Engine {
    /// Installe l'objet hôte `$262` sur le global courant. Les
    /// realms créés par `createRealm` ne le reçoivent **pas**
    /// (host-defined du realm principal seulement, sémantique
    /// officielle du runner).
    pub fn install_t262(&mut self) {
        let o = self.new_object();
        let g = self.global;
        self.set_own(o, "global", Value::Obj(g), true);
        let es = self.native_fn("evalScript", t262_eval_script);
        self.set_own(o, "evalScript", es, true);
        let cr = self.native_fn("createRealm", t262_create_realm);
        self.set_own(o, "createRealm", cr, true);
        let d = self.native_fn("detachArrayBuffer", t262_detach_ab);
        self.set_own(o, "detachArrayBuffer", d, true);
        self.set_own(g, "$262", Value::Obj(o), true);
    }
}

/// Évalue `src_v` comme un **script** dans le contexte `ctx`
/// (sémantique `$262.evalScript`/`realm.evalScript`, pas un eval
/// direct : le strict de l'appelant ne s'hérite pas — seule la
/// directive de la source décide). Restore **inconditionnel** du
/// contexte appelant : une exception realm ne laisse jamais le
/// moteur dans le mauvais contexte pour le reste du test.
fn eval_in_ctx(e: &mut Engine, ctx: crate::context::CtxId, src_v: Value) -> JsResult<Value> {
    let (src, enc) = match src_v {
        // Chaîne à surrogates isolés — même chemin que `eval()`
        // (les units survivent jusqu'aux littéraux produits).
        Value::Str(id) if id.has_units() => (crate::regexp::re_encode(&id.units_view()), true),
        Value::Str(id) => (id.to_string(), false),
        _ => return Err(e.type_error("evalScript expects a string source")),
    };
    // `switch_to` no-op sur contexte mort évaluerait dans le
    // realm courant — refus honnête à la place.
    let alive = ctx == e.cur_ctx || e.ctxs.get(ctx as usize).is_some_and(|m| m.alive);
    if !alive {
        return Err(e.type_error("evalScript on a destroyed realm"));
    }
    let prev = e.cur_ctx;
    if ctx != prev {
        e.switch_to(ctx);
    }
    // `strict` appartient au contexte courant : forcé à false le
    // temps de l'éval puis rendu **avant** le switch retour —
    // chaque contexte repart avec son propre drapeau.
    let saved_strict = e.strict;
    e.strict = false;
    let r = if enc {
        e.eval_source_enc(&src, true)
    } else {
        e.eval_source(&src)
    };
    e.strict = saved_strict;
    if ctx != prev {
        e.switch_to(prev);
    }
    r
}

/// `$262.evalScript(src)` — script dans le realm **courant**.
fn t262_eval_script(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let ctx = e.cur_ctx;
    eval_in_ctx(e, ctx, arg(args, 0))
}

/// `$262.createRealm()` — realm nu à intrinsics frais (même heap :
/// les objets traversent, `Array` de l'un n'est pas `Array` de
/// l'autre). Le handle `{global, evalScript, destroy}` est alloué
/// dans le realm appelant — `context_new_bare` laisse le nouveau
/// contexte courant, on restaure avant de construire l'objet.
fn t262_create_realm(e: &mut Engine, _t: Value, _args: &[Value]) -> JsResult<Value> {
    let caller = e.cur_ctx;
    let id = e.context_new_bare("about:blank");
    let global = e.ctxs[id as usize]
        .global
        .map_or(Value::Undefined, Value::Obj);
    e.switch_to(caller);
    let o = e.new_object();
    e.set_own(o, "global", global, true);
    let es = e.bound_native(
        "evalScript",
        realm_eval_script,
        vec![Value::Num(f64::from(id))],
    );
    e.set_own(o, "evalScript", es, true);
    let d = e.bound_native("destroy", realm_destroy, vec![Value::Num(f64::from(id))]);
    e.set_own(o, "destroy", d, true);
    Ok(Value::Obj(o))
}

/// `realm.evalScript(src)` — arg bound `0` = `CtxId` cible.
fn realm_eval_script(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let ctx = match arg(args, 0) {
        Value::Num(n) => n as crate::context::CtxId,
        _ => 0,
    };
    eval_in_ctx(e, ctx, arg(args, 1))
}

/// `realm.destroy()` — arg bound `0` = `CtxId` cible.
fn realm_destroy(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    if let Value::Num(n) = arg(args, 0) {
        e.context_destroy(n as crate::context::CtxId);
    }
    Ok(Value::Undefined)
}

/// `$262.detachArrayBuffer(buf)` — détache in-place : même
/// modèle que `buf.transfer(0)` (la prop `detached` est le seul
/// témoin lu par les vues), mais idempotent — spec
/// `DetachArrayBuffer` : un buffer déjà détaché est un no-op,
/// contrairement à `transfer` qui jette.
fn t262_detach_ab(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let Value::Obj(id) = arg(args, 0) else {
        return Err(e.type_error("detachArrayBuffer expects an ArrayBuffer"));
    };
    if !matches!(e.heap.obj(id).map(|o| &o.kind), Some(ObjKind::ByteBuf(_))) {
        return Err(e.type_error("detachArrayBuffer expects an ArrayBuffer"));
    }
    if let Some(b) = e.heap.obj_mut(id) {
        if let ObjKind::ByteBuf(bytes) = &mut b.kind {
            bytes.clear();
        }
    }
    e.set_own(id, "detached", Value::Bool(true), false);
    Ok(Value::Undefined)
}
