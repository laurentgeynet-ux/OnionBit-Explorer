// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Gate JS-C628 — statements `language/statements` élargis
//! (`interp/exec.rs`, `interp/mod.rs`, `interp/eval.rs`,
//! `parser.rs`) :
//! 1. **Complétions `UpdateEmpty`** — `V`/`empty` propagés
//!    par `if` (sans `else` → `undefined`), `while`/`do`/`for*`,
//!    `try/finally`, `switch` (`V=undefined`, jamais `empty`),
//!    `labeled`, `with`.
//! 2. **`with`** — env objet : `@@unscopables` observé par
//!    `Get` (getters/traps propagés), ordre spec
//!    `has → unscop → has → get/set`, `delete ident` →
//!    `[[Delete]]` sur l'objet, prop disparue → `undefined`
//!    sloppy / `ReferenceError` strict.
//! 3. **Déstructuration for-of (LHS expr)** — référence cible
//!    évaluée AVANT `IteratorStep` ; abrupts **produits par
//!    l'itération** (`next`/`done`/`value`) sans `return()` ;
//!    abrupts de cible/liaison → `IteratorClose` même `done`.
//! 4. **`@@iterator` via `Get`** — primitives wrappées ;
//!    `Pat::Array` sans `@@iterator` callable → `TypeError`.
//! 5. **Parse** — `for (let in o)` sloppy (LHS ident), `async
//!    of =>` arrow légale, rest dernier/sans-init (`[...x = 1]`,
//!    `[...r, y]`, `{...r, p}`).

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

/// `src` doit être une erreur de syntaxe.
#[track_caller]
fn syntax(src: &str) {
    let mut e = Engine::new();
    match e.eval(src) {
        Err(explorer_jscore::Error::Syntax { .. }) => {}
        other => panic!("{src} -> expected Syntax, got {other:?}"),
    }
}

#[test]
fn c628_completion_values() {
    // `if` sans `else`, cond fausse → `undefined` (non-vide).
    assert_eq!(run("eval('1; if (false) { }')"), "undefined");
    // `if` abrupt : `break` complète `undefined` (UpdateEmpty).
    assert_eq!(
        run("eval('2; do { 3; if (true) { break } else {} } while (false)')"),
        "undefined"
    );
    // `switch` : `V = undefined` — jamais `empty`.
    assert_eq!(run("eval('1; switch (null) { }')"), "undefined");
    assert_eq!(run("eval('1; switch (\"a\") { case null: }')"), "undefined");
    assert_eq!(run("eval('2; switch (\"a\") { case \"a\": 3; }')"), "3");
    // Boucles : `V = undefined`, valeur de corps préservée.
    assert_eq!(run("eval('1; while (true) { break; }')"), "undefined");
    assert_eq!(run("eval('2; while (true) { 3; break; }')"), "3");
    // `try`/`finally` : `F` normal → `F = C` (la valeur du
    // `finally` est écartée même non-vide) ; `F` abrupt écrase.
    assert_eq!(run("eval('1; try { 5 } finally {}')"), "5");
    assert_eq!(run("eval('1; try { 5 } finally { 7 }')"), "5");
    assert_eq!(
        run("eval('2; try { throw null } catch (e) { 7 } finally { 8 }')"),
        "7"
    );
    // `with`/`labeled` : complétion du corps.
    assert_eq!(run("eval('1; L: { 6; break L }')"), "6");
}

#[test]
fn c628_with_object_env() {
    // `delete ident` → `[[Delete]]` sur l'objet `with` (le
    // global survit).
    assert_eq!(
        run("var p3 = 1; var o = { p3: 'c' }; var d; \
              with (o) { d = delete p3 } d + '|' + o.p3 + '|' + p3"),
        "true|undefined|1"
    );
    // `@@unscopables` — `{p1:true}` masque `p1` → scope externe.
    assert_eq!(
        thrown(
            "var o = { p1: 'a' }; o[Symbol.unscopables] = { p1: true }; \
                with (o) { p1 }"
        ),
        "ReferenceError"
    );
    // Traps proxy : erreurs propagées (has / unscopables / get).
    assert_eq!(
        thrown(
            "var p = new Proxy({}, { has() { throw new RangeError() } }); \
                with (p) { q }"
        ),
        "RangeError"
    );
    assert_eq!(
        thrown(
            "var o = { q: 1 }; o[Symbol.unscopables] = new Proxy({}, \
                { get() { throw new EvalError() } }); with (o) { q }"
        ),
        "EvalError"
    );
    // Prop supprimée dans le getter `@@unscopables` → `undefined`
    // (sloppy `GetBindingValue`).
    assert_eq!(
        run("var o = { q: 42 }; o[Symbol.unscopables] = \
             { get q() { delete o.q; return false } }; var r; \
             with (o) { r = q } typeof r"),
        "undefined"
    );
    // Ordre spec des traps : `has → unscop → has → get`.
    assert_eq!(
        run("var log = []; var p = new Proxy({ x: 9 }, { \
             has(t, k) { log.push('h'); return k in t }, \
             get(t, k) { log.push('g'); return t[k] } }); \
             var r; with (p) { r = x } r + '|' + log.join('')"),
        "9|hghgh"
    );
}

#[test]
fn c628_dstr_close_and_lref() {
    // Référence cible évaluée AVANT `next` : le throw ferme
    // l'itérateur interne (`return` appelé, `next` jamais).
    assert_eq!(
        run("var nc = 0, rc = 0; var it = { \
             next() { nc++; return { done: true } }, \
             return() { rc++ } }; var itb = {}; \
             itb[Symbol.iterator] = () => it; \
             try { for ([ {}[(function(){ throw 7 })()] ] of [itb]) {} } \
             catch (e) {} nc + '|' + rc"),
        "0|1"
    );
    // Abrupt produit par l'itération (`next` jette) → PAS de
    // `return()` (`thrw-close-skip`).
    assert_eq!(
        run("var nc = 0, rc = 0; var it = { \
             next() { nc++; throw new EvalError() }, \
             return() { rc++ } }; var itb = {}; \
             itb[Symbol.iterator] = () => it; \
             try { for ([ x ] of [itb]) {} } catch (e) {} nc + '|' + rc"),
        "1|0"
    );
    // `rest` épuise l'itérateur → pas de `return()`.
    assert_eq!(
        run("var rc = 0; var it = { n: 0, \
             next() { this.n++; return this.n > 2 ? { done: true } \
                  : { done: false, value: this.n } }, \
             return() { rc++ } }; var itb = {}; \
             itb[Symbol.iterator] = () => it; var r; \
             for ([a, ...r] of [itb]) {} r.length + '|' + rc"),
        "1|0"
    );
    // `@@iterator` via `Get` — `delete Array.prototype[…]` →
    // `TypeError` à la déstructuration d'un tableau.
    assert_eq!(
        thrown("delete Array.prototype[Symbol.iterator]; var [x] = [1];"),
        "TypeError"
    );
    // `{...rest}` sur primitive string — index énumérables.
    assert_eq!(run("var r; ({...r} = 'f'); r[0]"), "f");
}

#[test]
fn c628_parse_targets() {
    // `let` LHS sloppy dans `for-in`.
    assert_eq!(
        run("var obj = { k: 1 }; var let; for ( let in obj ) ; let"),
        "k"
    );
    // `async of => …` — arrow légale en tête `for(;;)`.
    assert_eq!(run("var c = 0; for (async of => {}; c < 2; ++c) {} c"), "2");
    // Rest : dernier seulement, sans initialiseur.
    syntax("for ([...x = 1] of [[]]) ;");
    syntax("for ([...r, y] of [[]]) ;");
    syntax("for ({...r, p} of [{}]) ;");
    syntax("for ([...[(x, y)]] of [[[]]]) ;");
}
