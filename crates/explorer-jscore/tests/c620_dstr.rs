// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Gate JS-C620 — destructuration par item dans `for await`
//! (`interp/lazy.rs`) :
//! 1. Protocole **async** pour les cibles `[…]` — frame
//!    `DstrAwait` : un `next()` awaited par élément,
//!    `AsyncIteratorClose` awaited en fin de motif ou sur
//!    abrupt de bind (spec
//!    `AsyncIteratorBindingInitialization`). Avant : le
//!    `bind_pat` sync cherchait `@@iterator` → `TypeError`
//!    sur itérables purement async.
//! 2. Suspend du motif LHS évalué **par itération** : les
//!    `yield`/`await` des défauts/clés (`[x = yield]`,
//!    `{x: x = yield}`, `[...a[yield]]`) sont hissés en tête
//!    de corps — avant : hissés une fois avant la boucle.
//! 3. Cibles `Expr` motifs (`for await ([x] of y)`) :
//!    `resolve_lvalue` rendait `None` → jamais liées ;
//!    `write_lvalue` couvre la destructuration-assignation.

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

/// `for await (var [x] of [it])` — `it` n'a QUE
/// `@@asyncIterator` : tirage par `next()` awaited +
/// `return()` awaited à la fin du motif.
#[test]
fn c620_forawait_var_array_async_protocol() {
    let out = run("var log=[];var it={\
         [Symbol.asyncIterator]:function(){return {\
             next:function(){return Promise.resolve({value:1,done:false})},\
             return:function(){log.push('ret');return Promise.resolve({done:true})}}}};\
         async function f(){for await (var [x] of [it]){log.push('x'+x)}out=log.join('|')}f();");
    assert_eq!(out, "ret|x1"); // close de fin de motif avant le corps (spec)
}

/// Même forme en assignation : `for await ([x] of [it])`.
#[test]
fn c620_forawait_expr_array_assign() {
    let out = run("var x=0;var it={\
         [Symbol.asyncIterator]:function(){return {\
             next:function(){return Promise.resolve({value:7,done:false})},\
             return:function(){return Promise.resolve({done:true})}}}};\
         async function f(){for await ([x] of [it]){}out='x'+x}f();");
    assert_eq!(out, "x7");
}

/// `done` prématuré : l'élément restant lie `undefined`, le
/// `...rest` collecte ce qui a été drainé, pas de close
/// (itérateur épuisé).
#[test]
fn c620_forawait_array_early_done() {
    let out = run(
        "var log=[];var n=0;var it={\
         [Symbol.asyncIterator]:function(){return {\
             next:function(){n++;return Promise.resolve({value:n,done:n>1})},\
             return:function(){log.push('ret');return Promise.resolve({done:true})}}}};\
         async function f(){for await (var [a,b] of [it]){log.push(a+'/'+b)}out=log.join('|')}f();",
    );
    assert_eq!(out, "1/undefined");
}

/// `...rest` draine l'itérateur async jusqu'à `done`.
#[test]
fn c620_forawait_array_rest_drain() {
    let out = run("var n=0;var it={\
         [Symbol.asyncIterator]:function(){return {\
             next:function(){n++;return Promise.resolve({value:n*10,done:n>4})}}}};\
         async function f(){for await (var [a,...r] of [it]){out=a+'|'+r.join(',')}}f();");
    assert_eq!(out, "10|20,30,40");
}

/// Erreur de bind → `AsyncIteratorClose` awaited avant la
/// propagation (`return()` observé) : `{a}` sur `undefined`
/// → `TypeError` → `it.return()`.
#[test]
fn c620_forawait_bind_error_close() {
    let out = run("var log=[];var it={\
         [Symbol.asyncIterator]:function(){return {\
             next:function(){return Promise.resolve({value:undefined,done:false})},\
             return:function(){log.push('ret');return Promise.resolve({done:true})}}}};\
         async function f(){try{for await (var [{a}] of [it]){}}catch(e){log.push('caught')}out=log.join('|')}f();");
    assert_eq!(out, "ret|caught");
}

/// `[x = yield]` — le défaut suspend **par itération** et
/// reçoit la valeur envoyée par `.next(86)`.
#[test]
fn c620_forawait_lhs_yield_default() {
    let out = run("var x=0;\
         async function* g(){for await ([x = yield] of [[]]){out='x'+x}}\
         var it=g();it.next();it.next(86);");
    assert_eq!(out, "x86");
}

/// `{x: x = yield}` — défaut d'un motif objet.
#[test]
fn c620_forawait_lhs_yield_obj_default() {
    let out = run("var x=0;\
         async function* g(){for await ({x: x = yield} of [{}]){out='x'+x}}\
         var it=g();it.next();it.next(86);");
    assert_eq!(out, "x86");
}

/// `[{} = yield]` — défaut motif vide : le `yield` produit
/// la valeur envoyée, le motif `{}` accepte tout non-null.
#[test]
fn c620_forawait_lhs_yield_empty_obj() {
    let out = run("var x=0;\
         async function* g(){for await ([{} = yield] of [[]]){out='ok'}}\
         var it=g();it.next();it.next(86);");
    assert_eq!(out, "ok");
}

/// `[...x[yield]]` — clé calculée suspendue dans le reste :
/// `x[86]` reçoit le drain.
#[test]
fn c620_forawait_lhs_yield_computed_key() {
    let out = run("var x=[];var r;\
         async function* g(){for await ([...x[yield]] of [[33]]){out='r'+x[86]}}\
         var it=g();it.next();it.next(86);");
    assert_eq!(out, "r33");
}

/// `for-of` sync : le suspend de cible est aussi par
/// itération (`for ([x = yield] of [[]])` dans un
/// générateur).
#[test]
fn c620_gen_forof_lhs_yield() {
    let out = run("var x=0;\
         function* g(){for ([x = yield] of [[]]){out='x'+x}}\
         var it=g();it.next();it.next(86);");
    assert_eq!(out, "x86");
}

/// Régression — cible `{…}` en assignation : `for await
/// ({x} of it)` liait rien avant JS-C620.
#[test]
fn c620_forawait_expr_object_assign() {
    let out = run("var x=0;\
         async function f(){for await ({x} of [{x:9}]){out='x'+x}}f();");
    assert_eq!(out, "x9");
}
