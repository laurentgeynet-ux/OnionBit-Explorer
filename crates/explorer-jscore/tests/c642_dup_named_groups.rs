// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! JS-C642 — `regexp-duplicate-named-groups` : un nom de groupe
//! peut se répéter dans des **alternatives disjointes** ; chaque
//! déclaration a son slot, `groups.x`/`indices.groups.x` =
//! l'alternative participante, `\k<x>` = la participante (vide
//! si aucune). Hors alternatives disjointes → `SyntaxError`.

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

/// Slots distincts, `groups.x` = la participante.
#[test]
fn disjoint_alternatives() {
    assert_eq!(eval(r"/(?<x>a)|(?<x>b)/.exec('bab').join('/')"), "b//b");
    assert_eq!(eval(r"/(?<x>a)|(?<x>b)/.exec('bab').groups.x"), "b");
    assert_eq!(eval(r"'abab'.match(/(?<x>a)|(?<x>b)/).groups.x"), "a");
    assert_eq!(
        eval(r"/(?<x>a)|(?<x>b)|z/.exec('zab').groups.x"),
        "undefined"
    );
}

/// `\k<x>` = la participante des alternatives ; aucune → vide.
#[test]
fn dup_backreference() {
    assert_eq!(
        eval(r"/(?:(?<x>a)|(?<x>b))\k<x>/.exec('aa').join('/')"),
        "aa/a/"
    );
    assert_eq!(
        eval(r"/(?:(?<x>a)|(?<x>b))\k<x>/.exec('bb').join('/')"),
        "bb//b"
    );
    // Aucune participante (branche `z` choisie) → backref vide.
    assert_eq!(
        eval(r"/(?:z|(?:(?<x>a)|(?<x>b)))c\k<x>/.test('zc')"),
        "true"
    );
}

/// `indices.groups.x` suit la participante (`d` flag).
#[test]
fn dup_indices() {
    assert_eq!(
        eval(r"/(?<x>a)|(?<x>b)/d.exec('zb').indices.groups.x[0]"),
        "1"
    );
}

/// Réutilisation du même nom dans deux alternatives imbriquées
/// d'une même disjonction — reste disjoint.
#[test]
fn nested_disjoint() {
    assert_eq!(
        eval(r"/(?:(?<x>a)|(?:(?<x>b)|(?<x>c)))\k<x>/.exec('cc').groups.x"),
        "c"
    );
    assert_eq!(
        eval(r"/(?:(?<x>a)|(?:(?<x>b)|(?<x>c)))\k<x>/.exec('aa').join('/')"),
        "aa/a//"
    );
}

/// Hors alternatives disjointes → `SyntaxError`.
#[test]
fn non_disjoint_rejected() {
    // Consécutifs dans la même alternative.
    assert_eq!(
        eval(r"try{RegExp('(?<x>a)(?<x>b)');0}catch(e){e.name}"),
        "SyntaxError"
    );
    // Préfixe de chemin (niché dans la même branche).
    assert_eq!(
        eval(r"try{RegExp('(?<x>a)(?:(?<x>b)|c)');0}catch(e){e.name}"),
        "SyntaxError"
    );
    // Encore licite : alternants disjoints.
    assert_eq!(eval(r"RegExp('(?<x>a)|(?<x>b)').source"), "(?<x>a)|(?<x>b)");
}
