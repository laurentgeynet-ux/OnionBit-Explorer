// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Gate JS-C619 — résidus itérateurs mesurés sur test262
//! `language/statements/for-await-of`.
//!
//! Deux corrections :
//! 1. `continue`/`break` labellé vers une boucle externe dans
//!    `GExec` (`interp/lazy.rs`) : le label vivait sur le frame
//!    `Labeled` (Seq) et n'atteignait jamais la cible — les
//!    boucles reçoivent désormais leurs labels via
//!    `e.pending_labels` (convention `exec.rs`).
//! 2. Destructuration `[…]` paresseuse (spec
//!    `IteratorBindingInitialization`) : `bind_pat` tirait tout
//!    l'itérateur eager (`iter_values`) — itérateur non-
//!    finissant → boucle jusqu'au cap heap, `return()` jamais
//!    observé. Désormais un `next()` par élément +
//!    `IteratorClose` en fin de pattern ou sur abrupt.

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

/// `continue outer` dans `for await` : ferme l'itérateur
/// interne puis réitère l'externe — qui termine (`done:true`
/// au 3e `next()` → pas de close sur la complétion normale).
#[test]
fn c619_forawait_labeled_continue_close() {
    let out = run("var log=[];var k=0;var oi={\
         [Symbol.asyncIterator]:function(){return {\
             next:function(){k++;return Promise.resolve({value:k,done:k>2})},\
             return:function(){log.push('retO');return {done:true}}}}};\
         var ii={\
         [Symbol.asyncIterator]:function(){return {\
             next:function(){return Promise.resolve({value:2,done:false})},\
             return:function(){log.push('retI');return {done:true}}}}};\
         async function f(){outer:for await (var x of oi){\
         for await (var y of ii){log.push('x'+x+'y'+y);continue outer}}\
         out=log.join('|')}f();");
    assert_eq!(out, "x1y2|retI|x2y2|retI");
}

/// `continue outer` sur `for-of` sync dans un générateur :
/// cible la boucle externe, pas de close (`ForIter` matérialisé
/// — `iter` absent pour les arrays).
#[test]
fn c619_gen_forof_labeled_continue() {
    let out = run("async function f(){var o=[];outer:for (var a of [1,2,3]){\
         for (var b of [7]){o.push(a+b);continue outer}}out=o.join('|')}\
         f();");
    assert_eq!(out, "8|9|10");
}

/// `continue inner` non labellé en boucle imbriquée — ne doit
/// PAS quitter la boucle externe (garde contre une cible trop
/// large).
#[test]
fn c619_continue_inner_stays() {
    let out = run("async function f(){var o=[];for (var a of [1,2]){\
         for (var b of [3,4]){o.push(a+''+b);continue}}out=o.join('|')}\
         f();");
    assert_eq!(out, "13|14|23|24");
}

/// Destructuration `[x]` d'un itérateur non-finissant : un
/// seul `next()` puis `IteratorClose` — avant, `iter_values`
/// eager bouclait jusqu'au cap heap.
#[test]
fn c619_dstr_array_close() {
    let out = run("var calls=0;var it={\
         [Symbol.iterator]:function(){return {\
             next:function(){return {value:5,done:false}},\
             return:function(){calls++;return {done:true}}}}};\
         var [x]=it;out='x'+x+'c'+calls;");
    assert_eq!(out, "x5c1");
}

/// Même fix dans un `for await` — la cible de binding `[x]`
/// déstructure chaque valeur yielded (spec
/// `IteratorBindingInitialization`).
#[test]
fn c619_forawait_dstr_close() {
    let out = run("var calls=0;var it={\
         [Symbol.iterator]:function(){return {\
             next:function(){return {value:5,done:false}},\
             return:function(){calls++;return {done:true}}}}};\
         async function f(){for await (var [x] of [it]){\
         out='x'+x+'c'+calls}}f();");
    assert_eq!(out, "x5c1");
}

/// `[a,...r]` : `rest` draine l'itérateur jusqu'à `done` —
/// pas de close après épuisement.
#[test]
fn c619_dstr_rest_drains_no_close() {
    let out = run("var calls=0;var n=0;var it={\
         [Symbol.iterator]:function(){return {\
             next:function(){n++;return {value:n,done:n>3}},\
             return:function(){calls++;return {done:true}}}}};\
         var [a,...r]=it;out='a'+a+'r'+r.join('-')+'c'+calls;");
    assert_eq!(out, "a1r2-3c0");
}

/// Trou `[,,]` : consomme un pas sans lier — `x` prend la 3e
/// valeur, puis close.
#[test]
fn c619_dstr_hole_consumes_step() {
    let out = run("var calls=0;var n=0;var it={\
         [Symbol.iterator]:function(){return {\
             next:function(){n++;return {value:n*10,done:false}},\
             return:function(){calls++;return {done:true}}}}};\
         var [,,x]=it;out='x'+x+'c'+calls;");
    assert_eq!(out, "x30c1");
}

/// Erreur pendant le bind → `IteratorClose` et la throw
/// d'origine prime (spec §7.4.6).
#[test]
fn c619_dstr_bind_error_closes() {
    let out = run("var calls=0;var it={\
         [Symbol.iterator]:function(){return {\
             next:function(){return {value:{get x(){throw 'boom'}},done:false}},\
             return:function(){calls++;return {done:true}}}}};\
         try{var [{x}]=it}catch(e){out='c'+calls+'e'+e}");
    assert_eq!(out, "c1eboom");
}
