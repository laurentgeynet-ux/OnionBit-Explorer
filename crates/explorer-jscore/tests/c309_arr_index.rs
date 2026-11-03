// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Verrous JS-C309 — fast-path `arr[i]` : quand la clé est un
//! `Num` entier ≥ 0 et la base un `ObjKind::Array` ordinaire,
//! la lecture va directement à `elems[i]` sans allouer la
//! `String` de `prop_key`. Tous les cas qui doivent rester sur
//! le chemin normal (arguments mappé, trou, hors bornes, proto,
//! clés non canoniques, string indexée) sont verrouillés ici.

use explorer_jscore::Engine;

fn eval(src: &str) -> String {
    let mut e = Engine::new();
    match e.eval(src) {
        Ok(v) => e.describe(v),
        Err(err) => panic!("eval: {err}\n  src: {src}"),
    }
}

/// Lecture ordinaire : dans les bornes, trou, hors bornes.
#[test]
fn arr_index_basics() {
    assert_eq!(eval("var a=[10,20,30]; a[0]+'/'+a[1]+'/'+a[2]"), "10/20/30");
    assert_eq!(eval("var a=[1,2]; a[5] === undefined"), "true");
    assert_eq!(eval("var a=[1,,3]; a[1] === undefined && a.length"), "3");
}

/// La clé `Num` non canonique repasse par le chemin props.
#[test]
fn arr_index_non_canonical_key() {
    // `1.5` / `-1` sont des props nommées, pas des éléments.
    assert_eq!(
        eval("var a=[9]; a[1.5]=7; a[-1]=8; a[1.5]+'/'+a[-1]+'/'+a.length"),
        "7/8/1"
    );
    // `-0` lit l'élément 0 (spec : `-0`→`"0"`).
    assert_eq!(eval("var a=[42]; a[-0]"), "42");
}

/// Hors bornes : la chaîne proto doit toujours répondre.
#[test]
fn arr_index_proto_fallback() {
    assert_eq!(
        eval("var a=[1]; Array.prototype[9]='proto'; var r=a[9]; delete Array.prototype[9]; r"),
        "proto"
    );
}

/// `arguments` mappé : le fast-path doit refuser (`args_map`
/// prime) — l'alias param↔arguments[0] reste vivant.
#[test]
fn arr_index_mapped_arguments() {
    assert_eq!(
        eval("function f(x){ arguments[0]=9; return arguments[0]+'/'+x } f(1)"),
        "9/9"
    );
    assert_eq!(eval("function f(x){ x=5; return arguments[0] } f(1)"), "5");
}

/// Une prop numérique définie à côté d'un trou est lue par le
/// chemin normal (le fast-path rate le trou → chemin complet).
#[test]
fn arr_index_hole_with_named_prop() {
    assert_eq!(eval("var a=[1,,3]; a['1']='named'; a[1]"), "named");
}

/// `"abc"[1]` reste l'indexation string (base primitive, pas
/// `ObjKind::Array`).
#[test]
fn arr_index_string_base() {
    assert_eq!(eval("'abc'[1]"), "b");
    assert_eq!(eval("'abc'[1] === 'b' && 'abc'.length"), "3");
}
