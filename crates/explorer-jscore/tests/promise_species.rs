// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Régressions JS-C280 — `then`/`catch`/`finally` dérivent leur
//! résultat par `SpeciesConstructor(this, %Promise%)` +
//! `NewPromiseCapability(C)` (spec `PerformPromiseThen`). Avant,
//! `promise_then` rendait un `new_promise` inconditionnel : la
//! détection `FORCED_PROMISE_CONSTRUCTOR` de core-js (embarqué
//! dans le CMP Sourcepoint — vol 577 dailymotion) échouait sur
//! `p.then(…) instanceof FakePromise` → `globalThis.Promise`
//! remplacé par un polyfill au prototype `{then, catch}` sans
//! `finally` → `` `finally` is not a function (on Promise) ``.

use explorer_jscore::Engine;

/// Évalue `src` dans un moteur neuf — erreurs → `ERR:…`.
fn ev(src: &str) -> String {
    let mut e = Engine::new();
    match e.eval(src) {
        Ok(v) => e.describe(v),
        Err(err) => format!("ERR:{err}"),
    }
}

/// `setup` puis `check` en second job — les microtasks sont
/// drainées à la fin de chaque `eval` : les réactions `then`
/// enfilées dans `setup` ont couru quand `check` se lit.
fn seq(setup: &str, check: &str) -> String {
    let mut e = Engine::new();
    e.eval(setup)
        .unwrap_or_else(|err| panic!("setup {setup} -> {err}"));
    match e.eval(check) {
        Ok(v) => e.describe(v),
        Err(err) => panic!("check {check} -> {err}"),
    }
}

/// Pilote une `async function f` : résultat/rejet capturés dans
/// `out` après drain microtasks (même helper qu'`await_thenable`).
fn drv(src: &str) -> String {
    let mut e = Engine::new();
    let setup = format!(
        "var out='pending';\
         async function __f(){{ {src} }}\
         __f().then(function(v){{out='ok:'+v}},function(x){{out='err:'+x}});"
    );
    e.eval(&setup)
        .unwrap_or_else(|err| panic!("setup {src} -> {err}"));
    let _ = e.eval("out");
    let v = e.eval("out").unwrap_or_else(|err| panic!("{err}"));
    e.describe(v)
}

// ---------- la détection core-js, verbatim ----------

/// La sonde `FORCED_PROMISE_CONSTRUCTOR` relevée dans
/// `wrapperMessagingWithoutDetection.js` (Sourcepoint CMP) :
/// `e.constructor={}; e.constructor[@@species]=FakePromise;
/// e.then(fn) instanceof FakePromise` doit être `true` sinon
/// core-js remplace `globalThis.Promise`.
#[test]
fn corejs_species_probe() {
    assert_eq!(
        ev("var e=Promise.resolve(1);\
            var t=function(x){x(function(){},function(){})};\
            e.constructor={};e.constructor[Symbol.species]=t;\
            (e.then(function(){}) instanceof t)"),
        "true"
    );
}

/// `Promise[@@species]` accesseur — `get { return this }`.
#[test]
fn species_getter_on_ctor() {
    assert_eq!(ev("Promise[Symbol.species] === Promise"), "true");
    // Propriété d'accesseur, non-énumérable, sur le ctor.
    assert_eq!(
        ev(
            "var d=Object.getOwnPropertyDescriptor(Promise,Symbol.species);\
            typeof d.get + '|' + d.enumerable"
        ),
        "function|false"
    );
}

/// Défaut : `constructor` effacé/absent → `%Promise%` intrinsèque.
#[test]
fn species_default_when_constructor_undefined() {
    assert_eq!(
        ev("var e=Promise.resolve(1);e.constructor=undefined;\
            e.then(function(){}) instanceof Promise"),
        "true"
    );
    assert_eq!(
        ev("var e=Promise.resolve(1);e.constructor={};\
            e.then(function(){}) instanceof Promise"),
        "true"
    );
}

// ---------- sous-classes ----------

/// `class Q extends Promise` — `Q[@@species]` hérité rend `this`
/// (= Q) → `q.then(…)` est une instance de Q. Et
/// `Q.resolve(…)` (spec `PromiseResolve(this, x)`) rend une
/// instance de Q — `NewPromiseCapability(this)`.
#[test]
fn subclass_then_returns_subclass_instance() {
    assert_eq!(
        ev("class Q extends Promise {}\
            var q = new Q(function(r){r(1)});\
            q.then(function(){}) instanceof Q"),
        "true"
    );
    assert_eq!(
        ev("class Q extends Promise {} Q.resolve(1) instanceof Q"),
        "true"
    );
    assert_eq!(
        ev("class Q extends Promise {} Q.reject('x') instanceof Q"),
        "true"
    );
}

/// `@@species` overridé sur une sous-classe → ctor custom.
#[test]
fn subclass_species_override() {
    assert_eq!(
        ev("class Q extends Promise {}\
            var t=function(x){x(function(){},function(){})};\
            Object.defineProperty(Q,Symbol.species,{get:function(){return t}});\
            Q.resolve(1).then(function(){}) instanceof t"),
        "true"
    );
}

// ---------- capability étrangère : settle via callables ----------

/// Ctor étranger qui capture resolve — `p.then(()=>42)` settled
/// → la promesse `F` reçoit 42 via le callable capturé (la
/// réaction court en microtask — lue en second job).
#[test]
fn foreign_capability_settles_via_captured_fn() {
    assert_eq!(
        seq(
            "function F(x){var s=this;s.seen='pending';\
             x(function(v){s.seen=v},function(r){s.seen='rej:'+r});}\
             var e=Promise.resolve(1);e.constructor={};\
             e.constructor[Symbol.species]=F;\
             var f=e.then(function(){return 42});",
            "f.seen"
        ),
        "42"
    );
}

/// Idem côté rejet — `reject` capturé reçoit la raison.
#[test]
fn foreign_capability_reject_via_captured_fn() {
    assert_eq!(
        seq(
            "function F(x){var s=this;s.seen='pending';\
             x(function(v){s.seen=v},function(r){s.seen='rej:'+r});}\
             var e=Promise.reject('boom');e.constructor={};\
             e.constructor[Symbol.species]=F;\
             var f=e.then(undefined,function(r){throw r+'!'});",
            "f.seen"
        ),
        "rej:boom!"
    );
}

/// Pass-through (handler absent) vers capability étrangère.
#[test]
fn foreign_capability_passthrough() {
    assert_eq!(
        seq(
            "function F(x){var s=this;s.seen='pending';\
             x(function(v){s.seen=v},function(r){s.seen='rej:'+r});}\
             var e=Promise.reject('nope');e.constructor={};\
             e.constructor[Symbol.species]=F;\
             var f=e.then(function(){});",
            "f.seen"
        ),
        "rej:nope"
    );
}

// ---------- erreurs spec ----------

/// `constructor` non-objet → `TypeError`.
#[test]
fn species_ctor_non_object() {
    assert_eq!(
        ev("var e=Promise.resolve(1);e.constructor=5;\
            try{e.then(function(){})}catch(x){x.name}"),
        "TypeError"
    );
}

/// `@@species` non-callable → `TypeError`.
#[test]
fn species_non_callable() {
    assert_eq!(
        ev("var e=Promise.resolve(1);e.constructor={};\
            e.constructor[Symbol.species]=42;\
            try{e.then(function(){})}catch(x){x.name}"),
        "TypeError"
    );
}

/// Exécuteur qui ne capture pas les fns → `TypeError`
/// (`PromiseCapability` incomplète — `new C()` sans appel exec).
#[test]
fn capability_not_captured() {
    assert_eq!(
        ev("var e=Promise.resolve(1);e.constructor={};\
            e.constructor[Symbol.species]=function(){};\
            try{e.then(function(){})}catch(x){x.name}"),
        "TypeError"
    );
}

// ---------- les sentiers inchangés ----------

/// Chaîne native ordinaire — fast-path intact.
#[test]
fn native_chain_unchanged() {
    assert_eq!(
        drv(
            "var v = await Promise.resolve(2).then(function(x){return x+1})\
             .then(function(x){return x*10});return v;"
        ),
        "ok:30"
    );
    // `finally`/`catch` passent par `promise_then` — species
    // s'applique aussi (spec) mais le fast-path natif domine.
    assert_eq!(
        drv("var hit='';try{await Promise.reject('x')}\
             catch(e){hit='c:'+e}finally{hit+='|f'}return hit;"),
        "ok:c:x|f"
    );
}

/// Getter `constructor` qui jette → propagé depuis `then`.
#[test]
fn constructor_getter_throws() {
    assert_eq!(
        ev("var e=Promise.resolve(1);\
            Object.defineProperty(e,'constructor',{get:function(){throw 7}});\
            try{e.then(function(){})}catch(x){x}"),
        "7"
    );
}
