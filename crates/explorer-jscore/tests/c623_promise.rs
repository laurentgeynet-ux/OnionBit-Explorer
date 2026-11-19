// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Gate JS-C623 — `PromiseResolve`/`Await` spec-complet
//! (`interp/lazy.rs`, `promise.rs`) :
//! 1. `Await(v)` = `PromiseResolve(%Promise%, v)` puis
//!    `PerformPromiseThen` — la reprise est **toujours un
//!    job enfilé** (jamais de livraison synchrone sur
//!    promesse déjà réglée — `ticks-*` test262).
//! 2. `PromiseResolve` lit `v.constructor` par `Get` sur
//!    toute promesse — observable via accesseur.
//! 3. `AsyncFromSyncIterator` enchaîne
//!    `PromiseResolve(value)` → `then(unwrap→cap)` →
//!    `Await(cap)` — 2 gets `constructor` + 2 jobs par
//!    `next()` ; abrupt → `IfAbruptRejectPromise` (rejet
//!    de `cap`, propagé par job, jamais synchrone).
//! 4. Pompe `AsyncGeneratorEnqueue` : `pend` ne bloque
//!    plus quand `resume` est posée (deadlock exposé par
//!    l'await-toujours-job).

use explorer_jscore::Engine;

/// Évalue `setup`, draine les microtasks, puis lit `out`.
#[track_caller]
fn run(setup: &str) -> String {
    let mut e = Engine::new();
    e.eval(setup).unwrap_or_else(|err| panic!("setup -> {err}"));
    e.run_microtasks().expect("microtasks");
    match e.eval("out") {
        Ok(v) => e.describe(v),
        Err(err) => panic!("read out -> {err}"),
    }
}

/// `await` sur promesse résolue : la continuation est un
/// job — un `.then` enregistré **avant** `f()` passe avant
/// la reprise (`[pre, tick, post]`).
#[test]
fn c623_await_resolved_via_job() {
    let out = run("var log=[];\
         async function f(){log.push('pre');await Promise.resolve(0);log.push('post')}\
         Promise.resolve(0).then(()=>log.push('tick'));\
         f().then(()=>out=log.join('|'));");
    assert_eq!(out, "pre|tick|post");
}

/// `await` sur non-promesse : `PromiseResolve` l'enveloppe
/// — la reprise reste un job (`[pre, tick, post]`).
#[test]
fn c623_await_plain_value_via_job() {
    let out = run("var log=[];\
         async function f(){log.push('pre');await 42;log.push('post')}\
         Promise.resolve(0).then(()=>log.push('tick'));\
         f().then(()=>out=log.join('|'));");
    assert_eq!(out, "pre|tick|post");
}

/// `PromiseResolve` observe `v.constructor` : get
/// observable — `AsyncFromSyncIterator` lit
/// `value.constructor` à l'émission (miroir test262
/// `ticks-with-sync-iter-*`) ; la reprise `Await(cap)`
/// déroule ensuite `unwrap` + `resume` en 2 jobs →
/// `loop` après `tick1`/`tick2`.
#[test]
fn c623_afs_constructor_gets_before_ticks() {
    let out = run("var log=[];\
         var p=Promise.resolve(0);\
         Object.defineProperty(p,'constructor',{get(){log.push('ctor');return Promise}});\
         async function f(){\
             log.push('pre');\
             for await (var x of [p]){log.push('loop')}\
             log.push('post')}\
         Promise.resolve(0).then(()=>log.push('tick1'))\
                          .then(()=>log.push('tick2'))\
                          .then(()=>log.push('tick3'))\
                          .then(()=>log.push('tick4'))\
                          .then(()=>out=log.join('|'));\
         f();");
    assert_eq!(out, "pre|ctor|tick1|tick2|loop|tick3|tick4|post");
}

/// Getter `constructor` abrupt : `IfAbruptRejectPromise` —
/// `cap` rejetée, l'`Await` propage via job → `catch` après
/// `tick 2`, jamais de rejet synchrone (miroir test262
/// `continuation-abrupt-completion`).
#[test]
fn c623_afs_abrupt_constructor_rejects_cap() {
    let out = run("var log=[];\
         var p=Promise.resolve(0);\
         Object.defineProperty(p,'constructor',{get(){throw new Error('x')}});\
         async function f(){\
             log.push('start');\
             for await (var x of [p]);\
             log.push('never')}\
         Promise.resolve(0).then(()=>log.push('tick1'))\
                          .then(()=>log.push('tick2'))\
                          .then(()=>log.push('tick3'))\
                          .then(()=>out=log.join('|'));\
         f().catch(()=>log.push('catch'));");
    assert_eq!(out, "start|tick1|tick2|catch|tick3");
}

/// Régression pompe agen (JS-C623) : `await` en pleine
/// requête — `resume` posée débloque `agen_pump` (le bras
/// `resume.is_some()` était inatteignable → deadlock).
#[test]
fn c623_agen_await_resume_pump() {
    let out = run("var log=[];\
         async function* g(){await Promise.resolve(1);yield 9;yield 8}\
         var it=g();\
         it.next().then(v=>{log.push('a'+v.value);return it.next()})\
          .then(v=>{log.push('b'+v.value);out=log.join('|')});");
    assert_eq!(out, "a9|b8");
}

/// `for await` sur itérateur async natif : `next()` rend
/// une promesse — `Await` = `PromiseResolve` (constructor
/// lu) + job par pas.
#[test]
fn c623_async_iter_next_jobs() {
    let out = run("var log=[];\
         var it={[Symbol.asyncIterator](){return {next(){\
             log.push('next');\
             return Promise.resolve(this.i++<1?{value:7,done:false}:{done:true})},i:0}}};\
         async function f(){\
             log.push('pre');\
             for await (var x of it){log.push('loop'+x)}\
             log.push('post')}\
         Promise.resolve(0).then(()=>log.push('tick1')).then(()=>log.push('tick2'));\
         f().then(()=>out=log.join('|'));");
    assert_eq!(out, "pre|next|tick1|loop7|next|tick2|post");
}
