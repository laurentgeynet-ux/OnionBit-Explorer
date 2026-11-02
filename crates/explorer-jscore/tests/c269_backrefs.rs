// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Régression JS-C269 : `\N` cohabitant avec des groupes nommés.
//! `fancy_regex` refuse les backrefs numérotées dès qu'un nommé
//! existe (« Numbered backref not allowed ») — restriction du
//! backend, pas du JS. Les groupes anonymes sont renommés
//! `(?P<__eN>` à la traduction et `\N` réémis `\k<__eN>` ; les
//! noms synthétiques ne doivent pas fuiter dans `exec().groups`.

use explorer_jscore::Engine;

fn t(src: &str) -> String {
    let mut e = Engine::new();
    match e.eval(src) {
        Ok(v) => e.describe(v),
        Err(err) => format!("ERR {err}"),
    }
}

#[test]
fn numbered_backref_with_named_group() {
    // Cas du terrain (claude.ai — J13) : backref numérotée vers
    // un groupe nommé.
    assert_eq!(t(r"/(?<a>x)\1/.test('xx')"), "true");
    // Backref vers un anonyme en présence d'un nommé.
    assert_eq!(t(r"/(x)(?<b>y)\1/.test('xyx')"), "true");
    // Croisement numérotée + nommée.
    assert_eq!(t(r"/(?<a>x)(y)\2\k<a>/.test('xyyx')"), "true");
    // `\w` + `\s` classes, backref numérotée après nommé.
    assert_eq!(t(r"/(?<word>\w+)\s+\1/.test('aa aa')"), "true");
}

#[test]
fn numbered_backref_no_named_group_unchanged() {
    // Sans nommé : traduction native `\N` (comportement inchangé).
    assert_eq!(t(r"/(x)\1/.test('xx')"), "true");
    assert_eq!(t(r"/(a|b)\1/.test('bb')"), "true");
}

#[test]
fn nested_named_and_anonymous() {
    // Nommé imbriqué dans anonyme, backrefs croisées.
    assert_eq!(t(r"/((?<a>x)y)\1\2/.test('xyxyx')"), "true");
    // Nommé dans lookbehind + backref.
    assert_eq!(t(r"/x(?<=(?<a>y)x)b/.test('yxb')"), "true");
    // Nommés imbriqués : `a`=`b`='c', `\1\2`='cc' → "ccc" requis.
    assert_eq!(t(r"/(?<a>(?<b>c))\1\2/.test('ccc')"), "true");
    assert_eq!(t(r"/(?<a>(?<b>c))\1\2/.test('cdcd')"), "false");
    // Ordre inversé `\2\1`.
    assert_eq!(t(r"/(?<a>.)(?<b>.)\2\1/.test('abba')"), "true");
}

#[test]
fn synthetic_names_do_not_leak() {
    // `groups` ne contient que les noms déclarés — jamais `__eN`,
    // et les nommés non-participants valent `undefined` (spec).
    assert_eq!(t(r"/(?<a>x)|y/.exec('y').groups.a"), "undefined");
    assert_eq!(t(r"/(?<a>x)|y/.exec('y').groups.__e2"), "undefined");
    assert_eq!(t(r"/(x)(?<b>y)\1/.exec('xyx').groups.b"), "y");
    assert_eq!(t(r"/(x)(?<b>y)\1/.exec('xyx').groups.__e1"), "undefined");
    // Captures numérotées inchangées : `__e1` est le groupe 1.
    assert_eq!(t(r"/(x)(?<b>y)\1/.exec('xyx')[1]"), "x");
    assert_eq!(t(r"/(x)(?<b>y)\1/.exec('xyx')[2]"), "y");
    // `groups` existe dès qu'un nommé est déclaré, même absent.
    assert_eq!(t(r"var m=/(?<a>x)|y/.exec('y'); typeof m.groups"), "object");
}

#[test]
fn declared_name_may_be_e_prefixed() {
    // Un nom déclaré `__e1` ne doit pas être filtré ni voler le
    // slot synthétique (collision → `__e1_`).
    assert_eq!(t(r"/(?<__e1>x)(y)\2/.exec('xyy').groups.__e1"), "x");
}
