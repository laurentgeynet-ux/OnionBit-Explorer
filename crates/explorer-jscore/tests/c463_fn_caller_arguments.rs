// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! JS-C463 (§B-125, vol 734) — `caller`/`arguments` sur
//! `Function.prototype` : V8 rend `null` pour les fonctions
//! sloppy (`YouTube` lit `fn.caller`/`fn.arguments` dans sa
//! cascade — le `TypeError` inconditionnel remontait
//! `uncaught:` ×8+ et arrêtait la chaîne), les fonctions
//! strictes/génératrices/async gardent leur poison propre
//! (`restricted_prop` — propre prop > proto, spec
//! `AddRestrictedFunctionProperties` préservée).

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

/// Fonction sloppy : `caller`/`arguments` = `null` (V8),
/// pas de throw.
#[test]
fn c463_sloppy_fn_caller_arguments_null() {
    let mut e = engine();
    assert_eq!(
        ev(
            &mut e,
            "function f(){}; (f.caller===null)+'|'+(f.arguments===null)+'|'+(typeof f)"
        ),
        "true|true|function"
    );
    // En lecture héritée aussi : arrow sloppy → proto.
    assert_eq!(
        ev(
            &mut e,
            "var g=()=>1; (g.caller===null)+'|'+(g.arguments===null)"
        ),
        "true|true"
    );
}

/// Fonction stricte : le poison propre prime sur le proto —
/// `TypeError` inchangé (spec).
#[test]
fn c463_strict_fn_caller_still_throws() {
    let mut e = engine();
    assert_eq!(
        ev(
            &mut e,
            "var s=(function(){'use strict'; var sf=function(){};\
             try{var c=sf.caller; return 'nothrow:'+c}\
             catch(e){return e instanceof TypeError?'typeerror':'other'}})();\
             var a=(function(){'use strict'; var sf=function(){};\
             try{var c=sf.arguments; return 'nothrow:'+c}\
             catch(e){return e instanceof TypeError?'typeerror':'other'}})();\
             s+'|'+a"
        ),
        "typeerror|typeerror"
    );
}

/// Lecture répétée stable (la chaîne `YouTube` itère) + un
/// call-site via variable/property path.
#[test]
fn c463_repeated_access_and_bound() {
    let mut e = engine();
    assert_eq!(
        ev(
            &mut e,
            "function h(){}; var out=[]; for(var i=0;i<20;i++) out.push(h.caller===null);\
             out.every(Boolean)+'|'+out.length"
        ),
        "true|20"
    );
}
