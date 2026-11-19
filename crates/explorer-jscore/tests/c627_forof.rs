// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Gate JS-C627 — sémantique `for-of` synchrone
//! (`interp/exec.rs`, `interp/mod.rs`, `interp/eval.rs`,
//! `collections.rs`, `builtins/object.rs`, `heap.rs`,
//! `parser.rs`, `module.rs`) :
//! 1. **Record itérateur** — `next` lu une fois
//!    (`iterator-next-reference`) ; abrupts de
//!    `next`/`done`/`value` propagés **sans** `IteratorClose`
//!    (`iterator-next-error`, `iterator-next-result-*`) ;
//!    résultat non-objet → `TypeError` (symboles inclus).
//! 2. **`IteratorClose`** — `break`/`return`/`continue` externe
//!    closent ; complétion `throw` conservée si `return()`
//!    échoue ; `return()` non-objet → `TypeError`.
//! 3. **Itérateurs vivants** — arrays (`length` relue, `Get`
//!    réel par index), set (`contract`), `arguments` (mappé,
//!    `length` non-étendu par `args[len]=v`).
//! 4. **Early errors** — `for (x of [], [])` (comma),
//!    `for (async of …)` (lookahead littéral) ; `\u0061sync`
//!    et `(async)` restent des LHS légales.
//! 5. **TDZ de tête** — `for (let x of [x])` → `ReferenceError`
//!    (la déclaration `Uninit` est directe depuis le motif :
//!    `names_table` clé par pointeur d'AST rate quand le corps
//!    async a été ré-écrit par le lowering).
//! 6. **`defineProperty` accesseur sur index d'array** — vit
//!    dans `props` (bit 8), étend `length` (`array-key-get-error`).
//! 7. **Complétion** — `for-of` sans valeur → `undefined`
//!    (`UpdateEmpty`), valeur de corps préservée.

use explorer_jscore::Engine;

/// Évalue `src`, retourne la description de la complétion.
#[track_caller]
fn run(src: &str) -> String {
    let mut e = Engine::new();
    match e.eval(src) {
        Ok(v) => e.describe(v),
        Err(err) => panic!("{src} -> {err}"),
    }
}

/// `src` doit lever une erreur — rend son `name`.
#[track_caller]
fn thrown(src: &str) -> String {
    run(&format!("try {{ {src} 'NOERR' }} catch (er) {{ er.name }}"))
}

#[test]
fn c627_next_captured_once() {
    // `iterator.next` est lu UNE fois au prologue.
    let out = run(
        "var n = 0, it = { i: 0 }; \
         Object.defineProperty(it, 'next', { get: function() { n++; var self = it; return function() { return self.i++ < 2 ? {value:self.i,done:false} : {done:true}; }; } }); \
         var o = {}; o[Symbol.iterator] = function() { return it; }; \
         for (var x of o) {} n",
    );
    assert_eq!(out, "1");
}

#[test]
fn c627_next_abrupt_no_close() {
    // `next()` qui jette : `return` JAMAIS appelé.
    let out = run("var closed = 0; var o = {}; \
         o[Symbol.iterator] = function() { return { \
           next: function() { throw 'NX'; }, \
           'return': function() { closed++; return {}; } }; }; \
         try { for (var x of o) {} } catch(e) { e }; closed");
    assert_eq!(out, "0");
}

#[test]
fn c627_result_type_and_close() {
    // Résultats primitifs → TypeError ; symbole compris.
    for prim in ["true", "42", "'s'", "null", "undefined", "Symbol()"] {
        let out = run(&format!(
            "var o = {{}}; o[Symbol.iterator] = function() {{ return {{ next: function() {{ return {prim}; }} }}; }}; \
             try {{ for (var x of o) {{}} 'NOERR' }} catch(e) {{ e.name }}"
        ));
        assert_eq!(out, "TypeError", "prim {prim}");
    }
    // `return()` non-objet sur complétion normale → TypeError.
    let out = run("var o = {}; o[Symbol.iterator] = function() { return { \
           next: function() { return {value:1,done:false}; }, \
           'return': function() { return 7; } }; }; \
         try { for (var x of o) { break; } 'NOERR' } catch(e) { e.name }");
    assert_eq!(out, "TypeError");
    // Throw du corps conservé si `return()` échoue.
    let out = run("var o = {}; o[Symbol.iterator] = function() { return { \
           next: function() { return {value:1,done:false}; }, \
           'return': function() { return 7; } }; }; \
         try { for (var x of o) { throw 'BODY'; } } catch(e) { e }");
    assert_eq!(out, "BODY");
}

#[test]
fn c627_live_iterators() {
    // Array vivant : append visité, retrait non visité.
    let out = run("var a = [1], seen = [], n = 0; \
         for (var v of a) { seen.push(v); if (++n < 3) a.push(n + 1); if (n == 3) a.length = 0; } \
         seen.join(',')");
    assert_eq!(out, "1,2,3");
    // Accesseur d'index : `Get` réel → getter invoqué.
    let out = run(
        "var a = []; Object.defineProperty(a, '0', { get: function() { throw 'G'; } }); \
         try { for (var v of a) {} 'NOERR' } catch(e) { e }",
    );
    assert_eq!(out, "G");
    // `length` étendu par le defineProperty d'index.
    assert_eq!(
        run("var a = []; Object.defineProperty(a,'0',{get(){return 9}}); a.length"),
        "1"
    );
    // arguments mappé : mutation visible, `length` non étendu.
    let out = run(
        "var expected = [1, 4, 6], i = 0, ok = true; \
         (function() { for (var v of arguments) { if (v !== expected[i]) ok = false; i++; arguments[i] *= 2; } }(1, 2, 3)); \
         ok + '/' + i",
    );
    assert_eq!(out, "true/3");
    // Set contract : suppression pendant l'itération.
    let out = run("var s = new Set([1,2,3]), seen = []; \
         for (var v of s) { seen.push(v); if (v == 1) s.delete(2); } seen.join(',')");
    assert_eq!(out, "1,3");
}

#[test]
fn c627_parse_early_errors() {
    // Comma en RHS → SyntaxError ; `async of` littéral → SyntaxError ;
    // échappé/parenthésé → légal.
    let mut e = Engine::new();
    for bad in [
        "for (var x of [], []) {}",
        "for (let x of [], []) {}",
        "for (x of [], []) {}",
        "for (async of [1]) ;",
    ] {
        match e.eval(bad) {
            Err(explorer_jscore::Error::Syntax { .. }) => {}
            other => panic!("{bad} should be Syntax, got {other:?}"),
        }
    }
    let out = run("let async; for (\\u0061sync of [7]); async");
    assert_eq!(out, "7");
    let out = run("let async2; for ((async2) of [9]); async2");
    assert_eq!(out, "9");
}

#[test]
fn c627_tdz_and_completion() {
    // TDZ de tête : la tête `let`/`const`/`using` est `Uninit`
    // pendant l'éval de l'itérable ; les AUTRES noms restent lus.
    assert_eq!(thrown("for (let z of [z]) {}"), "ReferenceError");
    assert_eq!(thrown("for (const w of [w]) {}"), "ReferenceError");
    assert_eq!(thrown("for (using u of [u]) {}"), "ReferenceError");
    // Nom externe homonyme : la cellule TDZ ombre → ReferenceError.
    assert_eq!(thrown("let x = 1; for (let x of [x]) {}"), "ReferenceError");
    // Autre nom lu librement dans l'itérable.
    assert_eq!(thrown("let x = [9]; for (let y of [x]) {}"), "NOERR");
    // `typeof` sur un nom TDZ jette aussi.
    assert_eq!(thrown("for (let t of [typeof t]) {}"), "ReferenceError");
    // `typeof` d'un non-déclaré reste "undefined".
    assert_eq!(run("typeof neverDeclared627"), "undefined");
    // Complétion : `UpdateEmpty` — boucle sans valeur → undefined.
    assert_eq!(run("eval('for (var a of [1]) { 42; }')"), "42");
    assert_eq!(run("eval('for (var a of []) { 42; }')"), "undefined");
    // continue vers label externe → IteratorClose.
    let out = run(
        "var closed = 0; var o = {}; o[Symbol.iterator] = function() { var i = 0; return { \
           next: function() { return i++ < 9 ? {value:i,done:false} : {done:true}; }, \
           'return': function() { closed++; return {}; } }; }; \
         L: for (var a of [1]) { for (var x of o) { continue L; } } closed",
    );
    assert_eq!(out, "1");
}

#[test]
fn c627_await_using() {
    // `for (await using x of …)` — itération synchrone,
    // TDZ de tête active, libération `@@asyncDispose`.
    let mut e = Engine::new();
    e.eval(
        "globalThis.__r = 'unset'; \
         (async function(){ let x = 0; for (await using x of [x]) {} })()\
           .then(v => { globalThis.__r = 'RES:' + v }, \
                 er => { globalThis.__r = 'REJ:' + er.name });",
    )
    .expect("eval");
    e.run_microtasks().expect("microtasks");
    let out = match e.eval("globalThis.__r") {
        Ok(v) => e.describe(v),
        Err(err) => panic!("read -> {err}"),
    };
    assert_eq!(out, "REJ:ReferenceError");
}
