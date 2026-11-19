// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! JS-C625 — RegExp/String tranche suivante :
//!
//! - **`get flags` spec** : tout objet accepté, les huit props
//!   (`hasIndices`, `global`, `ignoreCase`, `multiline`, `dotAll`,
//!   `unicode`, `unicodeSets`, `sticky`) lues par `Get` +
//!   `ToBoolean` — getters observables, throws propagés
//!   (test262 `flags/coercion-*`).
//! - **`\s` inclut U+FEFF** (BOM/ZWNBSP) : le `\s` backend est
//!   `White_Space` Unicode sans FEFF — `\s` émis `[\s\x{feff}]`,
//!   `\S` nu `[^\s\x{feff}]`.
//! - **Props de match énumérables** : `index`/`input`/`groups`/
//!   `indices` posés en `CreateDataPropertyOrThrow`
//!   (enumerable:true) — `Object.keys(m)` les voit.
//! - **`RegExp.escape` ES2025** : `TypeError` sur non-String,
//!   alphanum ASCII en tête → `\xHH`, syntaxchars+`/` → `\c`,
//!   contrôles nommés, `otherPunctuator`/ws/LT/surrogates →
//!   `\xHH`/`\uXXXX`, reste littéral.
//! - **`.` spec** : exclut les 4 `LineTerminator` (`\n \r
//!   \u2028 \u2029`) — le `.` fancy n'excluait que `\n` ; sous
//!   `s`, tout char.
//! - **Classes `v` (`vsets`)** : sous-parseur `v_class` —
//!   `[[…]]` imbriqué, intersection `A&&B` → `(?=[B])[A]`,
//!   différence `A--B` → `(?![B])[A]`, strings `\q{…}` →
//!   alternance longueur-décroissante, ponctuateurs réservés
//!   (singles `(){}|/-`, doubles `&& !! … ~~`) → `SyntaxError`.

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

/// `get flags` : lectures `Get`+`ToBoolean` observables sur un
/// objet quelconque — pas de brand-check `[[RegExpMatcher]]`.
#[test]
fn flags_getter_observable() {
    let g = "Object.getOwnPropertyDescriptor(RegExp.prototype,'flags').get";
    // Tout flag truthy → ordre canonique dgimsuvy.
    assert_eq!(
        eval(&format!(
            "{g}.call({{hasIndices:1,global:1,ignoreCase:1,multiline:1,dotAll:1,unicode:1,unicodeSets:1,sticky:1}})"
        )),
        "dgimsuvy"
    );
    // ToBoolean : `false`/absent omis, `"x"` truthy compte.
    assert_eq!(eval(&format!("{g}.call({{global:'x',ignoreCase:0}})")), "g");
    // Getter qui jette → propagé (pas de lecture brute).
    assert_eq!(
        eval(&format!(
            "try {{ {g}.call({{get hasIndices(){{throw 'THREW'}}}}) }} catch(e) {{ e }}"
        )),
        "THREW"
    );
    // Ordre de lecture : hasIndices puis global (observé par compteur).
    assert_eq!(
        eval(&format!(
            "var o=[]; {g}.call({{get hasIndices(){{o.push('d')}}, get global(){{o.push('g')}}}}); o.join(',')"
        )),
        "d,g"
    );
    // `this` non-objet → TypeError.
    assert!(eval(&format!("{g}.call('x')")).starts_with("ERR"));
}

/// `\s` inclut U+FEFF ; `\S` l'exclut — hors classe et en
/// classe.
#[test]
fn whitespace_feff() {
    assert_eq!(eval(r#"/\s/.test("\ufeff")"#), "true");
    assert_eq!(eval(r#"/\S/.test("\ufeff")"#), "false");
    assert_eq!(eval(r#"/[\s]/.test("\ufeff")"#), "true");
    assert_eq!(eval(r#"/[^\s]/.test("\ufeff")"#), "false");
    // Les autres whitespace Unicode restent couverts.
    assert_eq!(eval(r#"/\s/.test("\u2028")"#), "true");
    assert_eq!(eval(r#"/\s/.test("a")"#), "false");
}

/// Propriétés du résultat de match énumérables
/// (`CreateDataPropertyOrThrow`).
#[test]
fn match_props_enumerable() {
    assert_eq!(
        eval(r#"var m=/(?<g>b)/d.exec("ab"); Object.keys(m).join(',')"#),
        "0,1,index,input,groups,indices"
    );
}

/// `RegExp.escape` — ES2025 `EncodeForRegExpEscape`.
#[test]
fn regexp_escape_spec() {
    // TypeError sur non-String (pas de ToString).
    assert!(eval(r"RegExp.escape(1)").starts_with("ERR"));
    assert!(eval(r"RegExp.escape({})").starts_with("ERR"));
    assert!(eval(r"RegExp.escape(Symbol())").starts_with("ERR"));
    // Alphanum ASCII en tête → `\xHH`.
    assert_eq!(eval(r#"RegExp.escape("aaa")"#), "\\x61aa");
    assert_eq!(eval(r#"RegExp.escape("1111")"#), "\\x31111");
    // Syntaxchars + `/` → `\c` partout.
    assert_eq!(
        eval(r#"RegExp.escape(".*+?^$()[]{}|/\\")"#),
        "\\.\\*\\+\\?\\^\\$\\(\\)\\[\\]\\{\\}\\|\\/\\\\"
    );
    // Contrôles nommés ; ponctu/espace → `\xHH`.
    assert_eq!(eval(r#"RegExp.escape("a b")"#), "\\x61\\x20b");
    assert_eq!(eval(r#"RegExp.escape("\n!")"#), "\\n\\x21");
    assert_eq!(eval(r#"RegExp.escape("a-b")"#), "\\x61\\x2db");
    // Surrogate isolé → `\uXXXX` ; astral littéral.
    assert_eq!(eval(r#"RegExp.escape("\ud800")"#), "\\ud800");
    assert_eq!(eval(r#"RegExp.escape("𐒤x")"#), "𐒤x");
    // Round-trip : l'échappé re-matche la source.
    assert_eq!(
        eval(r#"var s="a.b*c"; new RegExp("^"+RegExp.escape(s)+"$").test(s)"#),
        "true"
    );
    // `_` n'est pas échappé.
    assert_eq!(eval(r#"RegExp.escape("_x")"#), "_x");
}

/// `.` exclut les 4 `LineTerminator` — fancy n'excluait que
/// `\n`. `s` matche tout.
#[test]
fn dot_line_terminators() {
    assert_eq!(eval(r#"/./.test("\r")"#), "false");
    assert_eq!(eval(r#"/./.test("\u2028")"#), "false");
    assert_eq!(eval(r#"/./.test("\u2029")"#), "false");
    assert_eq!(eval(r#"/./.test("\n")"#), "false");
    assert_eq!(eval(r#"/./.test("a")"#), "true");
    assert_eq!(eval(r#"/./s.test("\r")"#), "true");
    assert_eq!(eval(r#"/./s.test("\u2028")"#), "true");
}

/// Classes `v` : `&&`, `--`, `\q{…}`, imbrication, réservés.
#[test]
fn vsets_class_ops() {
    // Intersection.
    assert_eq!(
        eval(r#"new RegExp("[[0-9]&&[0-9]]","v").test("5")"#),
        "true"
    );
    assert_eq!(
        eval(r#"new RegExp("[[0-9]&&[a-z]]","v").test("5")"#),
        "false"
    );
    assert_eq!(
        eval(r#"new RegExp("[\\d&&\\p{ASCII_Hex_Digit}]","v").test("5")"#),
        "true"
    );
    assert_eq!(
        eval(r#"new RegExp("[\\d&&\\p{ASCII_Hex_Digit}]","v").test("f")"#),
        "false"
    );
    assert_eq!(
        eval(r#"new RegExp("[\\d&&\\p{ASCII_Hex_Digit}]","v").test("g")"#),
        "false"
    );
    // Différence.
    assert_eq!(
        eval(r#"new RegExp("[[0-9]--[0-4]]","v").test("5")"#),
        "true"
    );
    assert_eq!(
        eval(r#"new RegExp("[[0-9]--[0-4]]","v").test("3")"#),
        "false"
    );
    assert_eq!(eval(r#"new RegExp("[[0-9]--\\d]","v").test("5")"#), "false");
    // Imbrication simple.
    assert_eq!(eval(r#"new RegExp("[[a-z][0-9]]","v").test("7")"#), "true");
    // `\q{…}` : strings disjointes.
    assert_eq!(eval(r#"new RegExp("[\\q{ab|c}]","v").test("ab")"#), "true");
    assert_eq!(eval(r#"new RegExp("[\\q{ab|c}]","v").test("c")"#), "true");
    assert_eq!(eval(r#"new RegExp("[\\q{ab|c}]","v").test("a")"#), "false");
    // String la plus longue d'abord : `ab` avant `a`.
    assert_eq!(eval(r#"new RegExp("[\\q{a|ab}]","v").exec("ab")[0]"#), "ab");
    // Ponctuateurs réservés → SyntaxError.
    assert!(eval(r#"new RegExp("[()]","v")"#).starts_with("ERR"));
    assert!(eval(r#"new RegExp("[a&&b--c]","v")"#).starts_with("ERR"));
    assert!(eval(r#"new RegExp("[a--b&&c]","v")"#).starts_with("ERR"));
    assert!(eval(r#"new RegExp("[!!]","v")"#).starts_with("ERR"));
    // Range encore fonctionnelle en `v`.
    assert_eq!(eval(r#"new RegExp("[a-z]","v").test("m")"#), "true");
}
