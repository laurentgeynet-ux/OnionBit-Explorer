// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! JS-C639 — `RegExp.prototype[Symbol.matchAll]` : `IsRegExp`
//! n'est appelé qu'une fois, via `Construct` dans le ctor
//! (ordre observable `ToString(arg)` → `Get(flags)` →
//! `ToString(flags)` → `@@match`) et une erreur levée par le
//! getter `@@match` se propage. Comportement livré par
//! JS-C634 (`regexp_ctor` spec) — ce gate verrouille.

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

/// Un getter `@@match` qui lève doit se propager hors de
/// `@@matchAll` (via `Construct` → `IsRegExp`).
#[test]
fn isregexp_getter_throws() {
    assert_eq!(
        eval(
            r"var o={get [Symbol.match](){throw 'BOM'}};try{RegExp.prototype[Symbol.matchAll].call(o,'');'no'}catch(e){e}"
        ),
        "BOM"
    );
}

/// Ordre d'observation spec : `ToString(arg)` → `Get(flags)`
/// → `ToString(flags)` → `IsRegExp` (`@@match` lu en dernier,
/// une seule fois, par le ctor dans `Construct`).
#[test]
fn isregexp_called_once_order() {
    let src = r"var calls=[];
        Object.defineProperty(RegExp.prototype,Symbol.match,{get:function(){calls.push('proto-m');return true}});
        var o={get [Symbol.match](){calls.push('get @@match');return false},
               get flags(){calls.push('get flags');return {toString(){calls.push('flags toString');return ''}}}};
        RegExp.prototype[Symbol.matchAll].call(o,{toString(){calls.push('arg toString')}});
        calls.join(',')";
    assert_eq!(
        eval(src),
        "arg toString,get flags,flags toString,get @@match"
    );
    // Le getter sur le prototype n'est pas lu (`o` a le sien).
    assert_eq!(
        eval(
            r"var c=0;
            Object.defineProperty(RegExp.prototype,Symbol.match,{get:function(){c++;return true}});
            var o={get [Symbol.match](){return false},get flags(){return 'g'}};
            RegExp.prototype[Symbol.matchAll].call(o,'x');c"
        ),
        "0"
    );
}
