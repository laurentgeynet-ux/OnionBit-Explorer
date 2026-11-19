// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Gate JS-C621 — soudure du cluster `for-await-of`
//! (`parser.rs`, `lexer.rs`, `interp/lazy.rs`) :
//! 1. Défaut suspendu de motif (`[x = yield]`) lié via une
//!    stmts-frame poussée par `DstrAwait` — l'itérateur du
//!    motif reste **ouvert** pendant la suspension : ordre
//!    `next()` → default observable, `.return()` abrupt →
//!    `AsyncIteratorClose` awaited (`rtrn-close-*`).
//! 2. Validation des cibles `Expr` `[…]`/`{…}` de
//!    `for-in`/`of`/`await` (`for_target_pat_ok`, miroir
//!    `expr_to_pat`) : `[(x,y)]`, `[a + b]`, `{x: f()}` →
//!    `SyntaxError` ; idents réservés strict (`eval`,
//!    `arguments`, `yield`) → `SyntaxError` en strict.
//! 3. `of` contextuel échappé : `o\u0066` (flag `escaped`
//!    du lexer porté par `Token`, `eat_of`/`peek_of`
//!    l'exigent non échappé) → `SyntaxError`.

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

/// `rtrn-close-null` — suspension au `yield` du défaut
/// **après** `it.next()` (nextCount 1), `.return()` abrupt
/// → `return()` du motif observé (returnCount 1), résultat
/// `null` awaited → `TypeError` propagée au rejet.
#[test]
fn c621_forawait_dstr_default_return_null() {
    let out = run("var nc=0;var rc=0;var u=0;\
         var iterator={\
             next:function(){nc++;return {done:false,value:undefined}},\
             return:function(){rc++;return null}};\
         var iterable={[Symbol.iterator]:function(){return iterator}};\
         async function* fn(){for await ([{} = yield] of [iterable]){u++}}\
         var it=fn();\
         it.next().then(function(r){\
             it.return().then(function(){out='fulfilled'},function(e){\
                 out='nc'+nc+'rc'+rc+'u'+u+(e instanceof TypeError?'te':'?')})});");
    assert_eq!(out, "nc1rc1u0te");
}

/// Chemin `done` : `[x = yield]` sur `[]` — l'itérateur
/// est épuisé avant le premier élément, le défaut suspend
/// quand même (`init-yield-expr`).
#[test]
fn c621_forawait_dstr_default_done_path() {
    let out = run("var x=0;\
         async function* g(){for await ([x = yield] of [[]]){out='x'+x}}\
         var it=g();it.next();it.next(4);");
    assert_eq!(out, "x4");
}

/// Ordre observable : `iterator.next()` a tiré l'élément
/// AVANT la suspension du `yield` — `nc` vaut 1 au premier
/// `.then` (spec `IteratorBindingInitialization`).
#[test]
fn c621_forawait_dstr_default_next_first() {
    let out = run("var nc=0;var x=0;\
         var iterator={next:function(){nc++;return {done:false,value:undefined}}};\
         var iterable={[Symbol.iterator]:function(){return iterator}};\
         async function* g(){for await ([x = yield] of [iterable]){out='bad'}}\
         var it=g();\
         it.next().then(function(){out='nc'+nc;it.return()});");
    assert_eq!(out, "nc1");
}

/// `o\u0066` échappé n'est pas le `of` contextuel —
/// `SyntaxError` (spec `for-in`/`for-of` header).
#[test]
fn c621_escaped_of_rejected() {
    let mut e = Engine::new();
    assert!(e.eval("for (var x o\\u0066 [1,2]) {}").is_err());
    // `of` littéral, lui, reste admis :
    assert!(e.eval("var s='';for (var x of [1,2]) {s+=x}").is_ok());
}

/// Cibles `Expr` non assignables → `SyntaxError`.
#[test]
fn c621_for_target_pat_ok() {
    let mut e = Engine::new();
    assert!(e.eval("for ([a + b] of [[]]) {}").is_err());
    assert!(e.eval("for ([f()] of [[]]) {}").is_err());
    assert!(e.eval("for ([(x, y)] of [[]]) {}").is_err());
    assert!(e.eval("for ({x: f()} of [{}]) {}").is_err());
    // et une cible motif classique reste admise :
    assert!(e.eval("for ([a] of [[1]]) {}").is_ok());
}

/// `eval`/`arguments`/`yield` réservés en strict — dans les
/// bindings **et** les exprs des cibles `Expr`.
#[test]
fn c621_strict_reserved_targets() {
    let mut e = Engine::new();
    assert!(e.eval("'use strict';for ([arguments] of [[]]) {}").is_err());
    assert!(e.eval("'use strict';for ([eval] of [[]]) {}").is_err());
    assert!(e
        .eval("'use strict';for (var [x = yield] of [[]]) {}")
        .is_err());
    // sloppy : `arguments` reste une cible d'assignation.
    assert!(e.eval("for ([arguments] of [[]]) {}").is_ok());
}
