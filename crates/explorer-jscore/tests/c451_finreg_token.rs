// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! JS-C451 (JT-6) — `FinalizationRegistry.register` :
//! `unregisterToken` présent et non-objet → `TypeError`
//! (spec `CanBeHeldWeakly`, parité V8) ; `undefined` = pas
//! de jeton, objet/symbole légaux ; `SameValue(target,
//! token)` toujours refusé.

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

fn ev(e: &mut Engine, src: &str) -> String {
    match e.eval(src) {
        Ok(v) => e.describe(v),
        Err(err) => format!("ERR:{err}"),
    }
}

#[test]
fn c451_unregister_token_must_be_object_or_undefined() {
    let mut e = engine();
    // Token non-objet → TypeError (avant : accepté, spec V8 jette).
    for bad in ["'str'", "42", "true", "null"] {
        let r = ev(
            &mut e,
            &format!(
                "var f=new FinalizationRegistry(function(){{}});f.register({{}},0,{bad});'ok'"
            ),
        );
        assert!(r.starts_with("ERR:"), "{bad} -> {r}");
        assert!(r.contains("must be an object or undefined"), "{bad} -> {r}");
    }
    // `undefined` = pas de jeton ; objet = légal.
    assert_eq!(
        ev(
            &mut e,
            "var f=new FinalizationRegistry(function(){});var t={};\
             f.register({},0);f.register({},0,undefined);f.register({},0,t);\
             f.unregister(t)"
        ),
        "true"
    );
}

#[test]
fn c451_unregister_non_object_returns_false() {
    let mut e = engine();
    // `unregister` sur token non-objet → `false` silencieux
    // (spec : aucun jeton ne peut matcher).
    assert_eq!(
        ev(
            &mut e,
            "var f=new FinalizationRegistry(function(){});\
             f.register({},0,{});f.unregister(42)+'|'+f.unregister('x')+'|'+f.unregister(undefined)"
        ),
        "false|false|false"
    );
}
