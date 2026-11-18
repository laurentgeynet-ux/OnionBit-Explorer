// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Gate JS-C601 — `%AsyncGenerator%.next/return/throw` :
//! file `AsyncGeneratorEnqueue` → **promesses FIFO** au lieu
//! du `{value,done}` synchrone hérité de `%IteratorPrototype%`
//! (~3 656 fichiers test262 mouraient `then is not a
//! function (on Object{value,done})`).
//!
//! Spec §27.5 : chaque appel enfile `(kind, completion,
//! promise)` ; la pompe sert la tête tant que le générateur
//! n'est ni `completed` ni suspendu en `await` — la requête
//! reste en tête pendant la suspension, settlée à la reprise
//! (`resume_common` → `agen_pump`).

use explorer_jscore::Engine;

/// Évalue `setup`, draine les microtasks du job, puis lit `out`.
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

/// `it.next()` rend une `Promise` (le verbatim test262 :
/// `{value,done}` nu → `then is not a function`).
#[test]
fn c601_next_rend_une_promesse() {
    let out = run("async function* g(){yield 1};var it=g();var r=it.next();\
         var out=[r instanceof Promise,typeof r.then,r.constructor.name].join('|');");
    assert_eq!(out, "true|function|Promise");
}

/// File FIFO : trois `next()` en vol settlent dans l'ordre
/// `{1,false}|{2,false}|{undefined,true}`.
#[test]
fn c601_file_fifo() {
    let out = run("async function* g(){yield 1;yield 2};var it=g();var a=[];\
         it.next().then(function(r){a.push(r.value+','+r.done)});\
         it.next().then(function(r){a.push(r.value+','+r.done)});\
         it.next().then(function(r){a.push(r.value+','+r.done);out=a.join('|')});var out='';");
    assert_eq!(out, "1,false|2,false|undefined,true");
}

/// Suspension `await` en pleine requête : la promesse de
/// `next()` n'est settlée qu'à la reprise, avec la valeur
/// awaited livrée au corps.
#[test]
fn c601_await_en_pleine_requete() {
    let out = run(
        "async function* g(){yield 1;var x=await Promise.resolve(9);yield x}\
         var it=g();var a=[];\
         it.next().then(function(r){a.push(r.value)});\
         it.next().then(function(r){a.push(r.value);out=a.join('|')});var out='';",
    );
    assert_eq!(out, "1|9");
}

/// `next(v)` livre `v` au site `yield` suspendu.
#[test]
fn c601_next_livre_la_valeur() {
    let out = run("async function* g(){var x=yield 1;yield x};var it=g();\
         it.next().then(function(){it.next(42).then(function(r){out=r.value})});var out=0;");
    assert_eq!(out, "42");
}

/// `return(v)` → `{v,done:true}` puis la file se draine en
/// `completed` (`{undefined,done:true}`).
#[test]
fn c601_return_puis_drain_completed() {
    let out = run("async function* g(){yield 1;yield 2};var it=g();var a=[];\
         it.next().then(function(){\
           it.return('r').then(function(r){a.push(r.value+','+r.done)});\
           it.next().then(function(r){a.push(r.value+','+r.done);out=a.join('|')})});var out='';");
    assert_eq!(out, "r,true|undefined,true");
}

/// `throw(v)` atteint le `try/catch` interne du corps.
#[test]
fn c601_throw_au_site_suspendu() {
    let out = run(
        "async function* g(){try{yield 1}catch(e){yield 'caught:'+e}};var it=g();\
         it.next().then(function(){it.throw('boom').then(function(r){out=r.value+','+r.done})});var out='';",
    );
    assert_eq!(out, "caught:boom,false");
}

/// `throw(v)` sur `suspendedStart` → **reject** (spec
/// `AsyncGeneratorResumeAbrupt` : le corps n'a jamais tourné,
/// son `try` n'est pas engagé).
#[test]
fn c601_throw_avant_start_rejette() {
    let out = run(
        "async function* g(){try{yield 1}catch(e){yield 'caught:'+e}};var it=g();\
         it.throw('boom').then(function(){out='F'},function(e){out='R:'+e});var out='';",
    );
    assert_eq!(out, "R:boom");
}

/// `for await` consome l'itérateur async (le `next` awaited
/// transite par la même file).
#[test]
fn c601_for_await_consomme() {
    let out = run("async function* g(){yield 1;yield 2};\
         var out='';(async function(){var a=[];\
         for await(var x of g())a.push(x);out=a.join('|')})();");
    assert_eq!(out, "1|2");
}

/// Surface proto spec : `it[Symbol.asyncIterator]() === it`
/// (`%AsyncIteratorPrototype%` distinct — la déviation
/// « hérite `%IteratorPrototype%` » est soldée).
#[test]
fn c601_async_iterator_symbole() {
    let out =
        run("async function* g(){yield 1};var it=g();var out=it[Symbol.asyncIterator]()===it;");
    assert_eq!(out, "true");
}

/// Générateur sync intact : `it.next()` reste `{value,done}`
/// nu (le changement ne touche que `async function*`).
#[test]
fn c601_gen_sync_inchange() {
    let out = run("function* g(){yield 1};var it=g();var r=it.next();\
         var out=[typeof r.then,r.value,r.done].join('|');");
    assert_eq!(out, "undefined|1|false");
}

/// `for of` sur un générateur async → `TypeError` (spec :
/// pas sync-itérable — `%AsyncIteratorPrototype%` n'a pas
/// de `@@iterator`).
#[test]
fn c601_pas_sync_iterable() {
    let mut e = Engine::new();
    let r = e.eval("async function* g(){yield 1};for(var x of g())x");
    assert!(r.is_err(), "for of sur agen doit jeter");
}

/// `Iterator.prototype.next.call(agen)` → `TypeError` (la
/// méthode sync garde son receveur).
#[test]
fn c601_methode_sync_sur_agen() {
    let mut e = Engine::new();
    let r = e.eval("async function* g(){yield 1};var it=g();Iterator.prototype.next.call(it)");
    assert!(r.is_err(), "iter_next sur agen doit jeter");
}
