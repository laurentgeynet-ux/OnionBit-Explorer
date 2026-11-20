// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! JS-C632 — reliquats named-groups / match-indices :
//! échappements `\uXXXX`/`\u{…}` dans les noms de groupes,
//! backrefs vers groupe non clos = vide, `groups` à proto
//! nul, `indices.groups` propre own même sans groupes.

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

/// `UnicodeEscapeSequence` décodée dans `RegExpIdentifierName`
/// — `(?<\u{03C0}>…)` ≡ `(?<π>…)`, astral via `\u{…}` ou
/// paire de surrogates `\uD8\uDC`.
#[test]
fn name_escapes_decoded() {
    assert_eq!(eval(r"/(?<\u{03C0}>a)/u.exec('bab').groups.π"), "a");
    assert_eq!(eval(r"/(?<\u{1d4d1}>a)/u.test('ba')"), "true");
    assert_eq!(eval(r"/(?<a\uD801\uDCA4>.)/u.test('a')"), "true");
    assert_eq!(eval(r"/(?<_\u200C>a)/u.test('a')"), "true");
    // Malformés : escape non-`\u`, surrogate orphelin, `>` absent.
    assert_eq!(eval(r"try{RegExp('(?<\\x41>a)','u');0}catch(e){1}"), "1");
    assert_eq!(eval(r"try{RegExp('(?<\uD801>a)','u');0}catch(e){1}"), "1");
    // `\k<…>` décode pareil.
    assert_eq!(eval(r"/(?<π>a)(?<_>b)\k<\u{03C0}>/u.test('aba')"), "true");
}

/// Une backref vers un groupe **ouvert** (pas encore fermé à la
/// position de la référence, ou déclaré plus loin) matche la
/// chaîne vide — spec 21.2.2.9.
#[test]
fn backref_to_open_group_matches_empty() {
    assert_eq!(
        eval(r"JSON.stringify('bab'.match(/(?<a>\k<a>\w)../u))"),
        "[\"bab\",\"b\"]"
    );
    assert_eq!(eval(r"'bab'.match(/(?<a>\k<a>\w)../u).groups.a"), "b");
    assert_eq!(
        eval(r"JSON.stringify('bab'.match(/\k<a>(?<a>b)\w\k<a>/u))"),
        "[\"bab\",\"b\"]"
    );
    assert_eq!(
        eval(r"'bab'.match(/(?<b>b)\k<a>(?<a>a)\k<b>/u).length"),
        "3"
    );
    assert_eq!(
        eval(r"JSON.stringify('bab'.match(/(?<b>b)\k<a>(?<a>a)\k<b>/))"),
        "[\"bab\",\"b\",\"a\"]"
    );
}

/// `groups` = `ObjectCreate(null)` ; `__proto__` y est une
/// entrée ordinaire.
#[test]
fn groups_null_prototype() {
    assert_eq!(
        eval("Object.getPrototypeOf(/(?<x>.)/.exec('a').groups) === null"),
        "true"
    );
    assert_eq!(eval(r"/(?<__proto__>.)/.exec('a').groups.__proto__"), "a");
    assert_eq!(eval("/./.exec('a').groups === undefined"), "true");
}

/// `indices.groups` : toujours prop own (`undefined` sans
/// groupes), proto nul avec groupes.
#[test]
fn indices_groups_own_property() {
    assert_eq!(
        eval("/./d.exec('a').indices.hasOwnProperty('groups')"),
        "true"
    );
    assert_eq!(eval("/./d.exec('a').indices.groups === undefined"), "true");
    assert_eq!(
        eval("Object.getPrototypeOf(/(?<x>.)/d.exec('a').indices.groups) === null"),
        "true"
    );
    assert_eq!(eval(r"/(?<x>.)/d.exec('a').indices.groups.x.join()"), "0,1");
    assert_eq!(
        eval(r"/(?<__proto__>.)/d.exec('a').indices.groups.__proto__.join()"),
        "0,1"
    );
}
