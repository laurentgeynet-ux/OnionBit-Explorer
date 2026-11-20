// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! JS-C633 — contrôles syntaxiques restreints : groupe de
//! modificateurs `(?-:…)` à listes vides rejeté (les deux
//! `RegularExpressionFlags` ne peuvent être vides ensemble)
//! et `IdentityEscape` restreint en mode `u` — seuls les
//! `SyntaxCharacter` + `/` sont des escapes d'identité en
//! position atome, `+ -` en corps de classe.

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

/// `(?-:x)` — les deux listes de flags vides : la grammaire
/// ES2025 exige au moins une lettre `i`/`m`/`s` d'un côté.
/// `(?i:x)` (add seul) et `(?-i:x)` (remove seul) restent
/// valides.
#[test]
fn modifiers_both_empty() {
    assert_eq!(eval(r"try{RegExp('(?-:x)');0}catch(e){1}"), "1");
    // Littéral : early SyntaxError → le script ne s'exécute pas.
    assert!(eval(r"try{/(?-:x)/;0}catch(e){1}").starts_with("ERR"));
    assert_eq!(eval(r"/(?i:i)/.test('I')"), "true");
    assert_eq!(eval(r"/(?m-i:i)/i.test('I')"), "false");
    assert_eq!(eval(r"/(?-i:i)/i.test('I')"), "false");
    // Même lettre des deux côtés → SyntaxError.
    assert_eq!(eval(r"try{RegExp('(?i-i:x)');0}catch(e){1}"), "1");
}

/// En mode `u`, un `IdentityEscape` n'est autorisé que sur
/// `SyntaxCharacter` (`^ $ \ . * + ? ( ) [ ] { } |`) et `/`
/// en position atome ; `-` seulement dans une classe.
/// Tout autre ASCII non reconnu → `SyntaxError`.
#[test]
fn restricted_identity_escape_u() {
    // Acceptés en atome : syntax chars + `/`.
    for s in [
        "^", "$", "\\", ".", "*", "+", "?", "(", ")", "[", "]", "{", "}", "|", "/",
    ] {
        let esc = s.replace('\\', "\\\\");
        assert_eq!(
            eval(&format!(
                r"try{{RegExp('\\{esc}','u');'ok'}}catch(e){{'E'}}"
            )),
            "ok",
            "\\{s}"
        );
    }
    // Refusés : espace, lettres non-escape, digits >0,
    // ponctuation non-syntax.
    for s in [
        " ", "!", "~", "%", "<", "a", "z", "A", "Z", "8", "9", "_", ",", ":",
    ] {
        assert_eq!(
            eval(&format!(r"try{{RegExp('\\{s}','u');'ok'}}catch(e){{'E'}}")),
            "E",
            "\\{s}"
        );
    }
    // `-` : refusé en atome, admis en classe.
    assert_eq!(eval(r"try{RegExp('\\-','u');0}catch(e){1}"), "1");
    assert_eq!(eval(r"/[a\-z]/u.test('-')"), "true");
    // `\b` en classe = backspace ; `\B` n'existe pas en classe.
    assert_eq!(eval(r"/[\b]/u.test('\b')"), "true");
    assert_eq!(eval(r"try{RegExp('[\\B]','u');0}catch(e){1}"), "1");
    assert_eq!(eval(r"/[\B]/.test('B')"), "true");
    // Les escapes structurés restent licites.
    assert_eq!(eval(r"/\x41/u.test('A')"), "true");
    assert_eq!(eval(r"/\cA/u.test('\x01')"), "true");
    assert_eq!(eval(r"/\0/u.test('\0')"), "true");
    assert_eq!(eval(r"/\*/u.test('*')"), "true");
    // Non-`u` : Annex B — identité libre.
    assert_eq!(eval(r"/\ /.test(' ')"), "true");
    assert_eq!(eval(r"/\!/.test('!')"), "true");
}

/// Le fichier `_alpha` couvre aussi les identités sur
/// lettres Unicode (`\π`, `\ſ`…) : sous `u`, tout char qui
/// n'est ni escape reconnu ni `SyntaxCharacter` est rejeté.
#[test]
fn restricted_identity_escape_alpha() {
    for ch in ["\u{3c0}", "\u{17f}", "\u{e9}", "\u{212a}"] {
        assert_eq!(
            eval(&format!(r"try{{RegExp('\\{ch}','u');'ok'}}catch(e){{'E'}}")),
            "E",
            "\\{ch}"
        );
        // Littéral non échappé : OK.
        assert_eq!(
            eval(&format!(r"RegExp('{ch}','u').test('{ch}')")),
            "true",
            "{ch}"
        );
    }
}
