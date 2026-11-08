// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Repro §B-105 — `cannot read 'prototype' of Null` dans
//! `chart.umd` `TypedRegistry.isForType` : `{__proto__:null}`
//! dans un `registerables` créait une prop own **énumérable**
//! `null` que `_each`/`for..in` livrait à `register` —
//! spec `PropertyDefinition` : `__proto__:` littéral règle
//! `[[Prototype]]`, jamais de prop propre (JS-C429).

use explorer_jscore::{Dom, Engine};
use std::cell::RefCell;
use std::rc::Rc;

fn eng() -> Engine {
    let mut e = Engine::new();
    e.install_dom(&Rc::new(RefCell::new(Dom::from_html(
        "<html><body><canvas id=c></canvas></body></html>",
    ))));
    e
}

fn load() -> String {
    std::fs::read_to_string("/c/OnionBit-vol/logs/b105/chart.umd.js")
        .or_else(|_| std::fs::read_to_string("C:/OnionBit-vol/logs/b105/chart.umd.js"))
        .expect("chart.umd.js absent — télécharger jsdelivr chart.js@4.4.0")
}

/// Verrou §B-105 — la racine moteur : `__proto__:` littéral règle
/// [[Prototype]], ne crée jamais une prop own énumérable.
#[test]
fn b105_proto_literal_n_est_pas_prop_own() {
    let mut e = eng();
    let r = e
        .eval(
            "(function(){\
                var o = {__proto__: null, a: 1};\
                var keys = [];\
                for (var k in o) keys.push(k);\
                var own = Object.prototype.hasOwnProperty.call(o, '__proto__');\
                var computed = {['__proto__']: null};\
                var cown = Object.getOwnPropertyNames(computed).join(',');\
                var cproto = Object.getPrototypeOf(computed) === Object.prototype;\
                var ignored = {__proto__: 7};\
                var ign_proto = Object.getPrototypeOf(ignored) === Object.prototype;\
                return keys.join(',')+'|'+own+'|'+cown+','+cproto+'|'+ign_proto;\
            })()",
        )
        .unwrap();
    // `a` seul énumérable ; `__proto__` littéral ni own ni énuméré ;
    // `['__proto__']` calculé = prop own normale (proto intact) ;
    // valeur non-obj ignorée (proto par défaut conservé).
    assert_eq!(format!("{r:?}"), r#"Str("a|false|__proto__,true|true")"#);
}

#[test]
fn b105_bundle_eval_sans_crash() {
    let mut e = eng();
    let src = load();
    match e.eval(&src) {
        Ok(_) => {
            let r = e.eval("typeof Chart");
            assert_eq!(format!("{r:?}"), "Ok(Str(\"function\"))", "Chart posé ?");
        }
        Err(err) => panic!("bundle eval a jeté : {err:?}"),
    }
}
