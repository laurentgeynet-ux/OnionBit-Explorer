// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! JS-C452 (JT-7/JT-8) — le `TypeError` d'appel sur callee
//! `Undefined`/`Null` **nomme la prop lue** : `x()` →
//! « call of `x` », `(0, o.y)()` (forme indirecte
//! minifiée) → « call of `y` ». Le prochain vol identifie
//! l'API manquante au lieu d'un « undefined » muet.

use explorer_jscore::{Dom, Engine};
use std::cell::RefCell;
use std::rc::Rc;

fn engine() -> Engine {
    let mut e = Engine::new();
    e.install_dom(&Rc::new(RefCell::new(Dom::from_html(
        "<html><body></body></html>",
    ))));
    e
}

fn err(e: &mut Engine, src: &str) -> String {
    match e.eval(src) {
        Err(err) => format!("{err}"),
        Ok(v) => panic!("{src} should fail, got {}", e.describe(v)),
    }
}

#[test]
fn c452_bare_call_names_callee() {
    let mut e = engine();
    // `var x` déclaré-undefined puis `x()` — le message
    // nomme `x`.
    let m = err(&mut e, "var x; x()");
    assert!(m.contains("is not a function"), "{m}");
    assert!(m.contains("call of `x`"), "{m}");
    // Null identique.
    let m = err(&mut e, "var z = null; z()");
    assert!(m.contains("call of `z`"), "{m}");
}

#[test]
fn c452_indirect_call_names_member() {
    let mut e = engine();
    // `(0, o.y)()` — l'appel indirect minifié perd le
    // receveur mais garde le nom du membre.
    let m = err(&mut e, "var o = {}; (0, o.y)()");
    assert!(m.contains("is not a function"), "{m}");
    assert!(m.contains("call of `y`"), "{m}");
}

#[test]
fn c452_member_call_diag_untouched() {
    let mut e = engine();
    // `o.y()` — le diagnostic `Member` existant prime :
    // il nomme déjà `y` + le receveur (pas de suffixe).
    let m = err(&mut e, "var o = {}; o.y()");
    assert!(m.contains("`y` is not a function"), "{m}");
}

#[test]
fn c452_ok_calls_untouched() {
    let mut e = engine();
    // Les appels sains ne paient rien au message.
    let r = e.eval("var f = function () { return 41 + 1; }; f()");
    assert_eq!(r.map(|v| e.describe(v)).unwrap(), "42");
}
