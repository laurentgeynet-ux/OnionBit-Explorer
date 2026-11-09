// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! JS-C448 (JT-3) — globals `CommonJS` en sandbox (résidu
//! §B-105) : `module`/`exports`/`require` référencés à nu
//! obtiennent un env enfant semé au lieu d'un
//! `ReferenceError` fatal ; les scripts sains gardés par
//! `typeof` (`debounce`/`chart.umd`) ne basculent jamais
//! en branche CJS — `typeof module` reste `"undefined"`
//! dans `global_env`.

use explorer_jscore::{Dom, Engine};
use std::cell::RefCell;
use std::rc::Rc;

const PAGE: &str = "<html><body><div id=d></div></body></html>";

fn engine() -> Engine {
    let mut e = Engine::new();
    e.install_dom(&Rc::new(RefCell::new(Dom::from_html(PAGE))));
    e
}

fn ev(e: &mut Engine, src: &str) -> String {
    match e.eval(src) {
        Ok(v) => e.describe(v),
        Err(err) => panic!("{src} -> {err}"),
    }
}

/// Référence nue `module.exports` : le script réussit dans
/// le sandbox (sans fix : `ReferenceError: module is not
/// defined`) et `module.exports` est lisible dans le même
/// script.
#[test]
fn c448_bare_module_ref_gets_sandbox() {
    let mut e = engine();
    e.eval_named(
        "module.exports = {v: 41};\n\
         window.__v = module.exports.v + 1;",
        "https://cdn.example/lib.js",
    )
    .expect("sandbox");
    assert_eq!(ev(&mut e, "__v"), "42");
    // `module.id`/`filename` = nom du script — `Node`-parity.
    e.eval_named(
        "module.exports = {};\n\
         window.__id = module.id + '|' + module.filename;",
        "https://cdn.example/dep.js",
    )
    .expect("sandbox");
    assert_eq!(
        ev(&mut e, "__id"),
        "https://cdn.example/dep.js|https://cdn.example/dep.js"
    );
}

/// `exports` alias — écritures sur `exports.x` atterrissent
/// dans `module.exports` (alias `Node`, avant ré-assignation).
#[test]
fn c448_exports_alias_writes_into_module_exports() {
    let mut e = engine();
    e.eval_named(
        "exports.a = 1;\n\
         module.exports.b = 2;\n\
         window.__s = exports.a + '|' + module.exports.b;",
        "https://cdn.example/alias.js",
    )
    .expect("sandbox");
    assert_eq!(ev(&mut e, "__s"), "1|2");
}

/// La garde UMD classique continue de prendre la branche
/// **navigateur** : `typeof module` reste `"undefined"`
/// pour un script sain (jamais replié — debounce/chart.umd
/// intacts).
#[test]
fn c448_umd_guard_keeps_browser_branch() {
    let mut e = engine();
    e.eval(
        "(function(g, f) {\n\
             typeof exports === 'object' && typeof module !== 'undefined'\n\
                 ? (module.exports = f())\n\
                 : (g.Chart = f());\n\
         })(this, function() { return {tag: 'umd'}; });",
    )
    .expect("umd");
    // `window.Chart` posé par la branche navigateur.
    assert_eq!(ev(&mut e, "typeof Chart + '|' + Chart.tag"), "object|umd");
    // Et `module`/`exports`/`require` restent invisibles
    // dans global_env — aucun feature-detect ne bascule.
    assert_eq!(ev(&mut e, "typeof module"), "undefined");
    assert_eq!(ev(&mut e, "typeof exports"), "undefined");
    assert_eq!(ev(&mut e, "typeof require"), "undefined");
}

/// `require` du sandbox sert le `module.exports` des
/// scripts déjà évalués sous sandbox — clé exacte, puis
/// dernier segment de chemin.
#[test]
fn c448_require_resolves_prior_sandbox_exports() {
    let mut e = engine();
    e.eval_named("module.exports = {v: 42};", "https://cdn.example/lib.js")
        .expect("lib sandbox");
    // `require` nu → sandbox du second script ; `'/x/lib.js'`
    // matche par dernier segment.
    e.eval_named(
        "var L = require('./lib.js');\n\
         window.__v = L.v;",
        "https://cdn.example/app.js",
    )
    .expect("app sandbox");
    assert_eq!(ev(&mut e, "__v"), "42");
    // Clé exacte aussi.
    e.eval_named(
        "var L = require('https://cdn.example/lib.js');\n\
         window.__w = L.v;",
        "https://cdn.example/app2.js",
    )
    .expect("app2 sandbox");
    assert_eq!(ev(&mut e, "__w"), "42");
}

/// `require` inconnu → `Error` honnête « Cannot find
/// module » — jamais de fetch ni de silence.
#[test]
fn c448_require_unknown_honest_error() {
    let mut e = engine();
    let err = e
        .eval("require('does-not-exist')")
        .expect_err("module inconnu");
    let msg = format!("{err}");
    assert!(
        msg.contains("Cannot find module 'does-not-exist'"),
        "message inattendu: {msg}"
    );
}

/// Un `ReferenceError` hors CJS remonte tel quel — le
/// repli ne masque jamais une autre erreur.
#[test]
fn c448_non_cjs_ref_error_not_sandboxed() {
    let mut e = engine();
    let err = e.eval("undefinedFn()").expect_err("ref error");
    let msg = format!("{err}");
    assert!(
        msg.contains("undefinedFn is not defined"),
        "message inattendu: {msg}"
    );
}
