// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! JS-C638 — quantificateurs et atomes : entiers de
//! quantificateur non bornés (`DecimalDigits` au-delà de
//! `u32::MAX` est une syntaxe valide qui ne matche jamais),
//! lookahead quantifié enveloppé dans un `(?:…)` (Annex B),
//! collapse des `(?:(?:…))` pour la profondeur du backend
//! et identity escape `\<` émis en `\x{..}` (fancy
//! interprétait `\<` en frontière de mot).

use explorer_jscore::Engine;

#[track_caller]
fn eval(src: &str) -> String {
    let mut e = Engine::new();
    e.install_t262();
    match e.eval(src) {
        Ok(v) => e.describe(v),
        Err(err) => format!("ERR {err}"),
    }
}

/// `DecimalDigits` n'est pas borné par la grammaire :
/// `/b{9007199254740991}/u` compile mais ne peut jamais
/// matcher (aucun input de cette taille). `{n,}` et
/// `{n,m}` avec borne énorme se comportent comme `{n,}`.
#[test]
fn quantifier_integer_limit() {
    assert_eq!(
        eval(r"try{var r=RegExp('b{9007199254740991}','u');'ok'}catch(e){'E'}"),
        "ok"
    );
    assert_eq!(
        eval(r"try{RegExp('b{9007199254740991}','u').test('b')}catch(e){'E'}"),
        "false"
    );
    // `lo` sous la limite reste un vrai quantificateur.
    assert_eq!(eval(r"/ab{2,9007199254740991}/u.test('abb')"), "true");
    // `lo` > `hi` reste une erreur, syntaxe cassée aussi.
    assert_eq!(eval(r"try{RegExp('a{2,1}');0}catch(e){1}"), "1");
    assert_eq!(eval(r"/a{2}/.test('aa')"), "true");
}

/// Annex B : un lookahead est quantifiable hors `u`, y
/// compris quand il est enveloppé dans un groupe non
/// capturant — `(?:(?=(abc)))?`. `u` reste une erreur.
#[test]
fn wrapped_lookahead_quantifier() {
    // `?` et `*` → alternative vide ; `+`/`{1}` → forme nue.
    assert_eq!(eval(r"/(?:(?=(abc)))?abc/.test('abc')"), "true");
    assert_eq!(eval(r"/(?:(?=(abc)))*abc/.test('abc')"), "true");
    assert_eq!(eval(r"/(?:(?=(abc)))+abc/.exec('abc')[0]"), "abc");
    assert_eq!(eval(r"/(?:(?=(abc))){1}abc/.test('abc')"), "true");
    // `u` : SyntaxError comme le cas direct `(?=x)?`.
    assert_eq!(eval(r"try{RegExp('(?:(?=(a)))?a','u');0}catch(e){1}"), "1");
}

/// Groupes non capturants imbriqués redondants aplatis —
/// le backend borne la profondeur à 64 ; 200 `(?:` JS
/// doivent rester utilisables en se réduisant.
#[test]
fn nested_noncapture_depth() {
    let mut open = String::new();
    let mut close = String::new();
    for _ in 0..200 {
        open.push_str("(?:");
        close.push(')');
    }
    let src = format!("var r=RegExp('{open}hello{close}');r.exec('hello')[0]");
    assert_eq!(eval(&src), "hello");
}

/// `\<` hors `u` est un identity escape littéral — fancy
/// regex y voit une frontière de mot ; `\x{..}` émis à la
/// place. Même traitement pour les autres ponctuations
/// non-syntaxe.
#[test]
fn identity_escape_punctuation() {
    assert_eq!(eval(r"/\</.test('<')"), "true");
    assert_eq!(eval(r"/\</.test('a<b')"), "true");
    assert_eq!(eval(r"/\</.exec('a<b')[0]"), "<");
    for s in [
        "!", "#", "%", "&", ",", "-", "/", ":", ";", "<", "=", ">", "@", "`", "~",
    ] {
        let got = eval(&format!(r"/\{s}/.test('{s}')"));
        assert_eq!(got, "true", "\\{s}");
    }
    // `\?`/`\.`/`\(` syntax chars : toujours échappés en
    // verbatim backend.
    assert_eq!(eval(r"/\?\.\(/.test('?.(')"), "true");
}

// Écarts backend catalogués (fancy-regex) — documentés,
// pas gatés ici : itération vide non rejetée
// (`/(a?b??)*/` → `"a"` au lieu de `"ab"`),
// réinitialisation des captures par répétition
// (`S15.10.2.5_A1_T4`, `S15.10.6.2_A1_T6`) et profondeur
// 200 de groupes capturants (`S15.10.2.8_A3_T15`).
