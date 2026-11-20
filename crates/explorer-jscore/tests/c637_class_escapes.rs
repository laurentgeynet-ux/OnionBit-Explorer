// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! JS-C637 — `CharacterClassEscape` exacts : `\d`/`\D` =
//! `0-9` ASCII (le backend couvre `Nd` Unicode), `\s`/`\S` =
//! `White_Space` JS **sans** NEL (U+0085) **+** FEFF — même
//! mécanisme de corps explicite que `\w` (JS-C631).

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

/// `\d` ne matche que `0-9` — chiffres arabes/devanagari/
/// fullwidth exclus, y compris sous `i`/`u`.
#[test]
fn digit_ascii_only() {
    assert_eq!(eval(r"/^\d$/.test('5')"), "true");
    assert_eq!(eval(r"/^\d$/.test('٥')"), "false");
    assert_eq!(eval(r"/^\d$/u.test('５')"), "false");
    assert_eq!(eval(r"/\d/i.test('५')"), "false");
    assert_eq!(eval(r"/[^\d]/.test('٣')"), "true");
    assert_eq!(eval(r"/^\D$/.test('٣')"), "true");
    assert_eq!(eval(r"/^\D$/.test('a')"), "true");
    // Corps de classe : `[\d]` fusionné ASCII.
    assert_eq!(eval(r"/^[\dx]$/.test('7')"), "true");
    assert_eq!(eval(r"/^[\dx]$/.test('٧')"), "false");
    assert_eq!(eval(r"/^[\dx]+$/v.test('٧x9')"), "false");
}

/// `\s` = whitespace JS : TAB..CR, SP, NBSP, Zs Unicode,
/// U+2028/U+2029, FEFF — **sans** NEL (U+0085, `White_Space`
/// Unicode que la spec JS exclut).
#[test]
fn whitespace_js_exact() {
    for cu in [
        "0020", "00a0", "1680", "2000", "200a", "2028", "2029", "202f", "205f", "3000", "feff",
        "0009", "000b",
    ] {
        assert_eq!(
            eval(&format!(r"/^\s$/.test('\u{{{cu}}}')")),
            "true",
            "U+{cu}"
        );
    }
    // NEL (U+0085) et MVS (U+180E) : ni l'un ni l'autre ne
    // sont whitespace JS — `\S` les matche.
    for cu in ["0085", "180e"] {
        assert_eq!(
            eval(&format!(r"/^\s$/.test('\u{{{cu}}}')")),
            "false",
            "U+{cu}"
        );
        assert_eq!(
            eval(&format!(r"/^\S$/.test('\u{{{cu}}}')")),
            "true",
            "U+{cu}"
        );
    }
    assert_eq!(eval(r"/\S/.test('  ')"), "true");
    // Sous `u`/`v` identique.
    assert_eq!(eval(r"/^\s$/u.test('')"), "false");
    assert_eq!(eval(r"/^\s+$/v.test('  ')"), "true");
    // Corps de classe non-`v` : `[\s]` fusionné.
    assert_eq!(eval(r"/^[\s]$/.test('﻿')"), "true");
    assert_eq!(eval(r"/^[\s]$/.test('')"), "false");
}
