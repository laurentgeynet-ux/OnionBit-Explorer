// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Gate JS-C611 — early errors de portée (spec `BoundNames` /
//! `LexicallyDeclaredNames` / `VarDeclaredNames`), déclarations
//! hors position statement, `const`/`using` sans initialiser,
//! `return`/`break`/`continue` hors contexte.
//!
//! Contrat : rejeter exactement ce que la spec exige — jamais
//! plus (les tolérances sloppy/annexe B sont des verrous).

use explorer_jscore::Engine;

fn ev(src: &str) -> String {
    let mut e = Engine::new();
    match e.eval(src) {
        Ok(v) => format!("{v:?}"),
        Err(err) => format!("ERR {err}"),
    }
}

/// `src` doit être une erreur de syntaxe.
fn is_syn_err(src: &str) -> bool {
    let s = ev(src);
    s.starts_with("ERR") && s.contains("syntax error")
}

#[test]
fn lexical_dups_and_var_conflicts() {
    for (src, expect_err) in [
        ("let x; let x;", true),
        ("let x; const x = 1;", true),
        ("let x; class x {}", true),
        ("const x = 1; const x = 2;", true),
        ("class x {}; class x {}", true),
        ("var x; let x;", true),
        ("let x; var x;", true),
        ("function f(){}; let f;", true),
        ("let f; function f(){}", true),
        ("var f; function f(){}", false), // var+fn : légal
        ("var a; var a;", false),         // var×2 : légal
        ("var a; var a; var a;", false),
        ("{ let a; let a; }", true),
        ("{ let a; } { let a; }", false), // blocs disjoints : légal
        ("{ var a; let a; }", true),
        ("{ var a; } let a;", true),                // var remonte au top
        ("{ { function g(){} } let g; }", true),    // annexB hoist sloppy
        ("function f(){}; function f(){};", false), // top sloppy : légal
    ] {
        assert_eq!(is_syn_err(src), expect_err, "src: {src}");
    }
}

#[test]
fn declaration_statement_position() {
    for (src, expect_err) in [
        ("if (1) let y = 1;", true),
        ("if (1) const y = 1;", true),
        ("if (1) class C {}", true),
        ("while (0) let y = 1;", true),
        ("while (0) const y = 1;", true),
        ("do class C {} while (0);", true),
        ("do let y = 1; while (0);", true),
        ("for (;;) let y = 1;", true),
        ("for (;;) const y = 1;", true),
        ("with ({}) let y = 1;", true),
        ("lbl: let y = 1;", true),
        ("lbl: class C {}", true),
        // annexe B : `function` sloppy sous if/else/label
        ("if (1) function f() {}", false),
        ("if (1) {} else function f() {}", false),
        ("lbl: function f() {}", false),
        ("a: b: function f() {}", false),
        // IsLabelledFunction : banni en position statement
        // unique même sloppy (annexB couvre `label:` seul).
        ("if (1) l: function f() {}", true),
        ("if (1) l: m: function f() {}", true),
        ("while (0) l: function f() {}", true),
        // `let` + newline = identifiant (ASI), jamais decl.
        ("if (1) let\nx = 1;", false),
        ("if (1) let\n{}", false),
        ("let\nx = 1;", false),
        // …mais jamais en corps de boucle ni sous `with`
        ("while (0) function f() {}", true),
        ("do function f() {} while (0);", true),
        ("for (;;) function f() {}", true),
        ("for (var k in {}) function f() {}", true),
        ("for (var k of []) function f() {}", true),
        ("with ({}) function f() {}", true),
        // génératrices/async : bannies partout
        ("if (1) function* g() {}", true),
        ("if (1) async function f() {}", true),
        ("lbl: function* g() {}", true),
    ] {
        assert_eq!(is_syn_err(src), expect_err, "src: {src}");
    }
    // strict : `function` sous if/else/label devient erreur.
    for src in [
        "'use strict'; if (1) function f() {}",
        "'use strict'; lbl: function f() {}",
        "'use strict'; if (1) {} else function f() {}",
    ] {
        assert!(is_syn_err(src), "src: {src}");
    }
}

#[test]
fn switch_redeclaration() {
    for (src, expect_err) in [
        ("switch (0) { case 1: let a; case 2: let a; }", true),
        ("switch (0) { case 1: let a; case 2: const a = 1; }", true),
        ("switch (0) { case 1: let a; case 2: class a {} }", true),
        ("switch (0) { case 1: var a; case 2: let a; }", true),
        ("switch (0) { case 1: function a(){} case 2: var a; }", true),
        ("switch (0) { case 1: let a; case 2: function a(){} }", true),
        ("switch (0) { case 1: var a; case 2: var a; }", false),
        // fn×2 sloppy : annexB tolère dans le CaseBlock
        (
            "switch (0) { case 1: function a(){} case 2: function a(){} }",
            false,
        ),
        // fn async/gen : jamais exemptées
        (
            "switch (0) { case 1: async function a(){} case 2: var a; }",
            true,
        ),
        (
            "switch (0) { case 1: function* a(){} case 2: var a; }",
            true,
        ),
    ] {
        assert_eq!(is_syn_err(src), expect_err, "src: {src}");
    }
    assert!(is_syn_err(
        "'use strict'; switch (0) { case 1: function a(){} case 2: function a(){} }"
    ));
}

#[test]
fn const_using_init() {
    for (src, expect_err) in [
        ("const x;", true),
        ("const x = 1, y;", true),
        ("const x = 1;", false),
        ("{ using x; }", true),
        ("{ using x = e; }", false),
        ("{ using {a} = e; }", true), // patterns interdits
        ("using x = e;", true),       // top-level script interdit
        ("{ using x = e; }", false),
        ("function f() { using x = e; }", false),
        ("let let;", true),
        ("let [let] = [];", true),
        ("const let = 1;", true),
        ("var let;", false), // `var let` sloppy : légal
        ("for (const x of []) {}", false),
        ("for (let x of []) {}", false),
    ] {
        assert_eq!(is_syn_err(src), expect_err, "src: {src}");
    }
}

#[test]
fn for_head_scope() {
    for (src, expect_err) in [
        // Le bloc du corps ombre la tête — légal (spec :
        // head ∩ VarDeclaredNames seulement).
        ("for (let x of []) { let x; }", false),
        ("for (let x in {}) { let x; }", false),
        ("for (let x = 0; x < 1; x++) { let x; }", false),
        ("for (const x of []) { var x; }", true),
        ("for (let x = 0; x < 1; x++) { { var x; } }", true),
        ("for (let x of []) { var x; }", true),
        ("for (let x of []) { let y; }", false),
        ("for (let x, x;;) {}", true),
        ("for (let [a, a] of []) {}", true),
        ("for (var x of []) { var x; }", false),
    ] {
        assert_eq!(is_syn_err(src), expect_err, "src: {src}");
    }
}

#[test]
fn catch_param_scope() {
    for (src, expect_err) in [
        ("try {} catch (e) { let e; }", true),
        ("try {} catch (e) { const e = 1; }", true),
        ("try {} catch (e) { class e {} }", true),
        ("try {} catch (e) { function e() {} }", true),
        ("try {} catch ([a, a]) {}", true),
        ("try {} catch ({x: a, y: a}) {}", true),
        ("try {} catch ([e]) { var e; }", true), // pattern : var∩param
        ("try {} catch (e) { var e; }", false),  // annexe B sloppy
        ("try {} catch (e) { var f; }", false),
    ] {
        assert_eq!(is_syn_err(src), expect_err, "src: {src}");
    }
    // strict : `catch(e){var e}` devient erreur.
    assert!(is_syn_err("'use strict'; try {} catch (e) { var e; }"));
}

#[test]
fn return_break_continue() {
    for (src, expect_err) in [
        ("return;", true),
        ("return 1;", true),
        ("if (1) return;", true),
        ("{ return; }", true),
        ("function f() { return 1; }", false),
        ("var f = function () { return; };", false),
        ("var f = () => { return 1; };", false),
        ("var f = () => 1;", false),
        ("break;", true),
        ("if (1) break;", true),
        ("for (;;) break;", false),
        ("while (0) break;", false),
        ("do break; while (0);", false),
        ("switch (0) { case 1: break; }", false),
        ("switch (0) { case 1: if (1) break; }", false),
        ("switch (0) { case 1: for (;;) break; }", false),
        ("continue;", true),
        ("switch (0) { case 1: continue; }", true),
        ("for (;;) continue;", false),
        ("while (0) continue;", false),
        ("for (var i = 0; i < 3; i++) { continue; }", false),
        ("lbl: for (var i = 0; i < 3; i++) continue lbl;", false),
        ("lbl: break lbl;", false),
        ("function f() { for (;;) break; }", false),
        ("for (;;) function () { break; }", true), // frontière fn
        ("for (;;) { var g = () => { continue; }; }", true),
        ("class C { static { return; } }", true), // static block
        ("class C { static { for (;;) break; } }", false),
    ] {
        assert_eq!(is_syn_err(src), expect_err, "src: {src}");
    }
}

#[test]
fn params_scope() {
    for (src, expect_err) in [
        ("function f(a) { let a; }", true),
        ("function f(a) { const a = 1; }", true),
        ("function f(a) { class a {} }", true),
        ("function f(a) { var a; }", false), // var∩param : légal
        ("function f(a, a) {}", false),      // simples sloppy : légal
        ("function f(a, a) { 'use strict'; }", true),
        ("function f([a, a]) {}", true), // non-simples : dup toujours
        ("function f(a, ...a) {}", true),
        ("(a, a) => {}", true), // flèches : jamais sloppy
        ("(a) => { let a; }", true),
        ("x => { let x; }", true),
        ("x => { var x; }", false),
        ("var f = function g(g) {};", true), // nom d'expr ∩ param
        ("function g(g) {}", false),         // déclaration : légal
        ("var f = function g(h) { let h; };", true),
    ] {
        assert_eq!(is_syn_err(src), expect_err, "src: {src}");
    }
}
