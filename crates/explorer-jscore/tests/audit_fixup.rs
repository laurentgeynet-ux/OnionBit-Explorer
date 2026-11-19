// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Correctifs de l'audit de triangulation V8/JSC(bun)/QuickJS
//! — sondes du corpus oracle où jscore déviait des **trois**
//! moteurs et de la spec : `TypedArray` `set`/`fill` post-coercion,
//! `arguments.caller` (retiré ES2017), `Function.prototype`
//! `caller`/`arguments` configurables, et le pattern `new
//! RegExp(s)` à surrogate isolé (régression C622).

use explorer_jscore::Engine;

#[track_caller]
fn eval(src: &str) -> String {
    let mut e = Engine::new();
    match e.eval(src) {
        Ok(v) => e.describe(v),
        Err(err) => format!("ERR {err}"),
    }
}

/// `set` : `ToIndex(offset)` avant la lecture de la source et
/// borne sur la longueur **vivante** — un `valueOf` qui rétrécit
/// le RAB rend la vue hors-bornes → `RangeError` (unanimité
/// V8/JSC/QJS ; avant : no-op silencieux → `t[0]` = 0).
#[test]
fn ta_set_live_len_range_error() {
    assert_eq!(
        eval("try{let r=new ArrayBuffer(8,{maxByteLength:16}),t=new Int16Array(r);t.set([9,9],{valueOf(){r.resize(2);return 2}});'ok'}catch(e){e.constructor.name}"),
        "RangeError"
    );
    // Chemin sain inchangé.
    assert_eq!(
        eval("let t=new Uint8Array(4);t.set([1,2],1);t[1]+''+t[2]"),
        "12"
    );
}

/// `fill` : `IsTypedArrayOutOfBounds` revérifié après les
/// coercitions des bornes → `TypeError` (unanimité ; avant :
/// écritures silencieusement perdues).
#[test]
fn ta_fill_oob_type_error() {
    assert_eq!(
        eval("try{let r=new ArrayBuffer(8),t=new Int16Array(r);t.fill(7,{valueOf(){r.transfer();return 2}});'ok'}catch(e){e.constructor.name}"),
        "TypeError"
    );
    // Rétréci mais en bornes : la plage vivante re-clampe.
    assert_eq!(
        eval("let r=new ArrayBuffer(8,{maxByteLength:16}),t=new Int16Array(r);t.fill(7,{valueOf(){r.resize(4);return 0}});t.length+':'+t[0]"),
        "2:7"
    );
}

/// `CreateUnmappedArgumentsObject` : `caller` n'existe plus
/// depuis ES2017 — `callee` reste l'accesseur `%ThrowTypeError%`
/// partagé (unanimité moteurs).
#[test]
fn arguments_caller_removed() {
    assert_eq!(
        eval(
            "var a=function(x={}){return arguments}();Object.getOwnPropertyDescriptor(a,'caller')"
        ),
        "undefined"
    );
    assert_eq!(
        eval("var a=function(x={}){return arguments}();try{a.callee;'ok'}catch(e){e.constructor.name}"),
        "TypeError"
    );
}

/// `Function.prototype.{caller,arguments}` : `%ThrowTypeError%`
/// get/set mais `configurable:true` (spec
/// `AddRestrictedFunctionProperties`, unanimité moteurs).
#[test]
fn function_proto_restricted_configurable() {
    assert_eq!(
        eval("var d=Object.getOwnPropertyDescriptor(Function.prototype,'caller');d.enumerable+'|'+d.configurable+'|'+delete Function.prototype.caller"),
        "false|true|true"
    );
    assert_eq!(
        eval(
            "var d=Object.getOwnPropertyDescriptor(Function.prototype,'arguments');d.configurable"
        ),
        "true"
    );
    // Fonction stricte : pas d'accesseur sloppy §B-125 propre —
    // le `%ThrowTypeError%` du proto jette (chemin intact,
    // moteur frais par `eval`).
    assert_eq!(
        eval("try{(function(){'use strict'}).caller;'ok'}catch(e){e.constructor.name}"),
        "TypeError"
    );
}

/// `new RegExp(s)` / `String.prototype.*` avec `s` porteur d'un
/// surrogate isolé : la source rendue en `\uXXXX` textuel donne
/// au traducteur la forme du littéral (régression C622 —
/// `re_encode` produisait une sentinelle relue comme atome
/// astral réel et éclatée en non-`u`).
#[test]
fn regexp_ctor_lone_surrogate() {
    assert_eq!(eval(r#"new RegExp("\uD800").test("x\uD800")"#), "true");
    assert_eq!(eval(r#""x\uD800".search("\uD800")"#), "1");
    assert_eq!(eval(r#"var m="x\uD800".match("\uD800");m.index"#), "1");
    // `u` reste cohérent ; pairs astrals inchangés.
    assert_eq!(eval(r#"new RegExp("\uD800","u").test("x\uD800")"#), "true");
    assert_eq!(eval(r#"new RegExp("𐒤").test("𐒤")"#), "true");
}
