// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Verrous JS-C645 — coercion `ToObject(this)` des méthodes génériques,
//! ordre spec `Object.prototype.*`, `@@toStringTag`, `ArraySpeciesCreate`.

use explorer_jscore::Engine;

fn eval(src: &str) -> String {
    let mut e = Engine::new();
    e.install_t262();
    match e.eval(src) {
        Ok(v) => e.describe(v),
        Err(err) => format!("ERR {err}"),
    }
}

#[test]
fn c645_array_receiver_toobject() {
    // Primitives coercées (spec : toute méthode Array.prototype commence
    // par ToObject(this)) — null/undefined lèvent TypeError.
    assert_eq!(eval("Array.prototype.slice.call(true).length"), "0");
    assert_eq!(eval("Array.prototype.slice.call('abc').length"), "3");
    assert_eq!(eval("Array.prototype.slice.call('abc')[0]"), "a");
    assert_eq!(eval("Array.prototype.indexOf.call('abc','b')"), "1");
    assert!(eval("Array.prototype.slice.call(null)").starts_with("ERR"));
    assert!(eval("Array.prototype.forEach.call(undefined,function(){})").starts_with("ERR"));
}

#[test]
fn c645_mutateurs_rendent_o() {
    // copyWithin/fill/reverse/sort rendent O (wrapper coercé), pas `this`.
    assert_eq!(
        eval("Array.prototype.fill.call('x',1) instanceof String"),
        "true"
    );
    assert_eq!(eval("Array.prototype.sort.call('ba') instanceof String"), "true");
    assert_eq!(
        eval("Array.prototype.reverse.call('ab') instanceof String"),
        "true"
    );
    assert_eq!(
        eval("Array.prototype.copyWithin.call('abc',0,1) instanceof String"),
        "true"
    );
}

#[test]
fn c645_is_prototype_of_ordre_et_proxy() {
    // V non-objet → false AVANT ToObject(this) ; Symbol = primitive.
    assert_eq!(eval("Object.prototype.isPrototypeOf.call(null, 5)"), "false");
    assert_eq!(
        eval("Object.prototype.isPrototypeOf.call(undefined, Symbol())"),
        "false"
    );
    // Le walk passe par [[GetPrototypeOf]] (trap proxy observable).
    assert_eq!(
        eval(
            "var hits=0;var p=new Proxy(function(){},{getPrototypeOf(t){hits++;return Array.prototype}}); \
             var f=Object.setPrototypeOf(function(){},p); Array.prototype.isPrototypeOf.call(f, p)===false && hits>=1"
        ),
        "true"
    );
}

#[test]
fn c645_to_locale_string_invoke_receiver() {
    // Invoke(this,"toString") : lookup via wrapper, receiver primitif.
    assert_eq!(
        eval(
            "'use strict'; var seen; var b=true; Object.defineProperty(Object.prototype,'zz',{get(){return 1}}); \
             Boolean.prototype.toString=function(){seen=typeof this;return 'x'}; \
             Object.prototype.toLocaleString.call(b)==='x' && seen==='boolean'"
        ),
        "true"
    );
    assert!(eval("Object.prototype.toLocaleString.call(null)").starts_with("ERR"));
}

#[test]
fn c645_prop_key_symbole_rendu() {
    // ToPropertyKey : un objet dont toString rend un symbole → clé symbole,
    // pas TypeError (ne pas router le symbole par ToString).
    assert_eq!(
        eval(
            "var s=Symbol('k'); var o={}; o[s]=1; var k={toString(){return s}}; \
             Object.prototype.hasOwnProperty.call(o,k) && o.propertyIsEnumerable(k)"
        ),
        "true"
    );
}

#[test]
fn c645_species_flatmap_et_concat() {
    // concat : O (receiver) est l'item 0, puis les args.
    assert_eq!(
        eval("Array.prototype.concat.call('a','b') instanceof Array"),
        "true"
    );
    // flatMap : résultat via ArraySpeciesCreate — constructeur custom honoré.
    assert_eq!(
        eval(
            "var C=function(n){var a=Array(n);a.tag=7;return a}; \
             var src=[1,2]; src.constructor={[Symbol.species]:C}; \
             var r=src.flatMap(function(x){return [x*2]}); r.tag===7 && r.length===2"
        ),
        "true"
    );
}

#[test]
fn c645_array_to_string_fallback() {
    // Array.prototype.toString : join callable → join ; sinon
    // %Object.prototype.toString%.
    assert_eq!(eval("[1,2].toString()"), "1,2");
    assert_eq!(
        eval("var a=[1,2];a.join=5;Object.prototype.toString.call(a)"),
        "[object Array]"
    );
}

#[test]
fn c645_to_string_tags() {
    // builtinTag spec : primitifs brandés, BigInt tag non-string → "Object",
    // @@toStringTag custom string prioritaire.
    assert_eq!(eval("Object.prototype.toString.call(true)"), "[object Boolean]");
    assert_eq!(eval("Object.prototype.toString.call(5)"), "[object Number]");
    assert_eq!(eval("Object.prototype.toString.call('x')"), "[object String]");
    assert_eq!(eval("Object.prototype.toString.call(Symbol())"), "[object Symbol]");
    assert_eq!(
        eval("Object.prototype.toString.call(Object(1n))"),
        "[object BigInt]"
    );
    // Tag non-string → retombe sur le builtinTag ("Object" pour wrapper
    // BigInt sans slot de tag).
    assert_eq!(
        eval(
            "Object.defineProperty(BigInt.prototype,Symbol.toStringTag,{value:86}); \
             Object.prototype.toString.call(Object(1n))"
        ),
        "[object Object]"
    );
    // @@toStringTag custom sur instances brandées → prioritaire.
    assert_eq!(
        eval(
            "var s=new String(); s[Symbol.toStringTag]='t'; Object.prototype.toString.call(s)"
        ),
        "[object t]"
    );
    assert_eq!(
        eval("var a=[]; a[Symbol.toStringTag]='t'; Object.prototype.toString.call(a)"),
        "[object t]"
    );
}
