// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Gate JS-C616 — `for await`/`for-of` : sémantique
//! `AsyncFromSyncIterator` + `IteratorClose`.
//!
//! Tranche 1 : deux déviations de la machine `GExec`
//! (`interp/lazy.rs`) —
//! 1. Repli `@@iterator` (`CreateAsyncFromSyncIterator`) :
//!    `next()` rend `{value,done}` synchrone, c'est
//!    `result.value` qui est awaited (unwrap) — avant, la
//!    boucle armait l'await sur l'objet résultat et livrait
//!    la promesse brute (`result.value` = `[object Promise]`).
//! 2. `IteratorClose` : `break`/`return`/`throw`/`continue`
//!    externe traversant un `for await` doit appeler
//!    `iter.return()` **awaited** (spec `AsyncIteratorClose`)
//!    ; idem `for-of` sync en version non awaited. Avant, le
//!    frame était dépilé tel quel — `return()` jamais appelé.
//!    Un `next()` **rejeté** ne ferme PAS (abrupt hors corps).

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

/// Repli `@@iterator` : `result.value` est awaited — une
/// promesse en valeur est unwrap, pas livrée brute.
#[test]
fn c616_sync_iter_unwrap_value() {
    let out = run("var a=[];\
         async function f(){for await (var x of [Promise.resolve(1),2,Promise.resolve(3)]){a.push(x)}out=a.join('|')}f();");
    assert_eq!(out, "1|2|3");
}

/// `done:true` court-circuite sans unwrap supplémentaire.
#[test]
fn c616_sync_iter_done() {
    let out = run("var n=0;var it={\
         [Symbol.iterator]:function(){return {\
             next:function(){n++;return {value:Promise.resolve(n),done:n>2}},\
             return:function(){out='closed';return {done:true}}}}};\
         async function f(){for await (var x of it){}out='n'+n}f();");
    assert_eq!(out, "n3");
}

/// `break` dans `for await` (itérateur async) → `return()`
/// appelé une fois.
#[test]
fn c616_forawait_break_close() {
    let out = run("var calls=[];var it={\
         [Symbol.asyncIterator]:function(){return {\
             next:function(){return Promise.resolve({value:1,done:false})},\
             return:function(){calls.push('ret');return Promise.resolve({done:true})}}}};\
         async function f(){for await (var x of it){break}out=calls.join('|')}f();");
    assert_eq!(out, "ret");
}

/// `throw` dans le corps → `return()` awaited avant que la
/// throw ne reparte (ordre observable).
#[test]
fn c616_forawait_throw_close_awaited() {
    let out = run("var log=[];var it={\
         [Symbol.asyncIterator]:function(){return {\
             next:function(){return Promise.resolve({value:1,done:false})},\
             return:function(){return Promise.resolve().then(function(){log.push('closed');return {done:true}})}}}};\
         async function f(){try{for await (var x of it){throw 'boom'}}\
         catch(e){log.push('catch:'+e)}out=log.join('|')}f();");
    assert_eq!(out, "closed|catch:boom");
}

/// `return` dans une async fn traversant `for await` → close.
#[test]
fn c616_forawait_return_close() {
    let out = run("var log=[];var it={\
         [Symbol.asyncIterator]:function(){return {\
             next:function(){return Promise.resolve({value:1,done:false})},\
             return:function(){log.push('closed');return {done:true}}}}};\
         async function f(){for await (var x of it){return 42}log.push('jamais')}\
         f().then(function(){out=log.join('|')});var out='';");
    assert_eq!(out, "closed");
}

/// `next()` **rejeté** → pas de `return()` (abrupt hors corps).
#[test]
fn c616_forawait_next_reject_no_close() {
    let out = run("var calls=0;var it={\
         [Symbol.asyncIterator]:function(){return {\
             next:function(){return Promise.reject('boom')},\
             return:function(){calls++;return {done:true}}}}};\
         async function f(){try{for await (var x of it){}}\
         catch(e){out='calls'+calls+'/'+e}}f();");
    assert_eq!(out, "calls0/boom");
}

/// `break` **labelé** vers la boucle externe → les deux
/// itérateurs sont fermés dans l'ordre (interne puis cible).
#[test]
fn c616_forawait_labeled_break_close() {
    let out = run("var log=[];function mk(n){return {\
         [Symbol.asyncIterator]:function(){return {\
             next:function(){return Promise.resolve({value:n,done:false})},\
             return:function(){log.push('ret'+n);return {done:true}}}}}};\
         async function f(){outer:for await (var x of mk(1)){\
         for await (var y of mk(2)){log.push(y);break outer}}out=log.join('|')}f();");
    assert_eq!(out, "2|ret2|ret1");
}

/// `for-of` sync : `break` appelle `iter.return()` — même
/// sur un itérateur déjà drainé (`ForIter` conserve `iter`).
#[test]
fn c616_forof_break_close() {
    let out = run("var calls=0;var it={\
         [Symbol.iterator]:function(){var i=0;return {\
             next:function(){i++;return {value:i,done:i>5}},\
             return:function(){calls++;return {done:true}}}}};\
         for (var x of it){if(x===2){break}}out='calls'+calls;");
    assert_eq!(out, "calls1");
}

/// `return` absent → `break` passe sans erreur (close
/// optionnelle spec `IteratorClose`).
#[test]
fn c616_close_absente() {
    let out = run("var it={\
         [Symbol.asyncIterator]:function(){return {\
             next:function(){return Promise.resolve({value:1,done:false})}}}};\
         async function f(){for await (var x of it){break}out='ok'}f();");
    assert_eq!(out, "ok");
}

/// `return()` rejetée → la nouvelle abrupt remplace le
/// break (spec : abrupt innerResult l'emporte).
#[test]
fn c616_close_reject_remplace() {
    let out = run("var it={\
         [Symbol.asyncIterator]:function(){return {\
             next:function(){return Promise.resolve({value:1,done:false})},\
             return:function(){return Promise.reject('close-boom')}}}};\
         async function f(){try{for await (var x of it){break}}\
         catch(e){out='catch:'+e}}f();");
    assert_eq!(out, "catch:close-boom");
}
