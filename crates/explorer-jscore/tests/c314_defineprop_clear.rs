// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Verrous JS-C313 — sémantique `[[DefineOwnProperty]]` : dans un
//! descripteur d'accessor, un champ `get`/`set` PRÉSENT avec la
//! valeur `undefined` **vide** ce côté (il ne « conserve » pas
//! l'ancien). C'est le mécanisme du décorateur `@computed` Polymer
//! (`aRc` fait `P.get = void 0` puis `Object.defineProperty`) :
//! si le vieux getter survit, il re-fire sur le prototype sans
//! `__data` et tue le constructeur (mur §B-4(c) `ytd-app`).
//! Cas adjacents : descripteur générique sur accessor (flags seuls,
//! pas de conversion en data), non-configurable (refus spec).

use explorer_jscore::Engine;

fn eval(src: &str) -> String {
    let mut e = Engine::new();
    match e.eval(src) {
        Ok(v) => e.describe(v),
        Err(err) => panic!("eval: {err}\n  src: {src}"),
    }
}

/// Le cas réel : `P.get = void 0` sur le descripteur récupéré,
/// puis `defineProperty` — le getter est supprimé (spec).
#[test]
fn get_undefined_clears_getter() {
    assert_eq!(
        eval(
            "var o={};Object.defineProperty(o,'x',{get:function(){return 7},configurable:!0,enumerable:!0});\
             var d=Object.getOwnPropertyDescriptor(o,'x');d.get=void 0;Object.defineProperty(o,'x',d);\
             var d2=Object.getOwnPropertyDescriptor(o,'x');\
             o.x+'|'+typeof d2.get+'|'+d2.enumerable+'|'+d2.configurable"
        ),
        "undefined|undefined|true|true"
    );
}

/// Même chose côté `set` : `set:undefined` présent vide le setter.
#[test]
fn set_undefined_clears_setter() {
    assert_eq!(
        eval(
            "var o={_v:0};Object.defineProperty(o,'x',{set:function(v){this._v=v},configurable:!0});\
             var d=Object.getOwnPropertyDescriptor(o,'x');d.set=void 0;Object.defineProperty(o,'x',d);\
             o.x=9;o._v"
        ),
        "0"
    );
}

/// `get:undefined` posé par `defineProperty` direct (pas via un
/// descripteur muté) — lecture → `undefined`, jamais d'appel.
#[test]
fn direct_get_undefined_is_inert() {
    assert_eq!(
        eval(
            "var o={};Object.defineProperty(o,'x',{get:undefined,set:function(){},configurable:!0});\
             o.x === undefined && typeof Object.getOwnPropertyDescriptor(o,'x').set"
        ),
        "function"
    );
}

/// Champ `set` PRÉSENT avec une fonction remplace l'ancien setter
/// — le merge côté-absent ne s'applique qu'aux champs absents.
#[test]
fn present_side_replaces() {
    assert_eq!(
        eval(
            "var o={log:[]};Object.defineProperty(o,'x',{get:function(){return 1},set:function(){this.log.push('old')},configurable:!0});\
             Object.defineProperty(o,'x',{set:function(){this.log.push('new')}});\
             o.x=5;o.x+'|'+o.log[0]"
        ),
        "1|new"
    );
}

/// Descripteur générique (flags seuls) sur accessor : le type
/// est préservé — pas de conversion en data `Undefined`.
#[test]
fn generic_descriptor_keeps_accessor() {
    assert_eq!(
        eval(
            "var o={};Object.defineProperty(o,'x',{get:function(){return 7},configurable:!0,enumerable:!0});\
             Object.defineProperty(o,'x',{enumerable:!1});\
             var d=Object.getOwnPropertyDescriptor(o,'x');\
             o.x+'|'+typeof d.get+'|'+d.enumerable"
        ),
        "7|function|false"
    );
}

/// Accessor non-configurable : changer `get` (même via
/// `undefined`) est un `TypeError` — spec `[[DefineOwnProperty]]`.
#[test]
fn nonconfig_get_cannot_change() {
    assert_eq!(
        eval(
            "var o={};Object.defineProperty(o,'x',{get:function(){return 1},configurable:!1});\
             try{Object.defineProperty(o,'x',{get:undefined});'no-throw'}catch(e){'TypeError'}"
        ),
        "TypeError"
    );
    // Mais `get:undefined` sur un accessor sans getter est
    // idempotent — même valeur, pas de throw.
    assert_eq!(
        eval(
            "var o={};Object.defineProperty(o,'x',{set:function(){},configurable:!1});\
             try{Object.defineProperty(o,'x',{get:undefined});'ok'}catch(e){'TypeError'}"
        ),
        "ok"
    );
}

/// Chaîne Polymer verbatim — la séquence exacte du décorateur
/// `aRc`/`_.V` : getter posé, déplacé vers `__compute__h`, puis
/// `get` vidé — la capture `proto[h]` doit lire `undefined`.
#[test]
fn polymer_computed_decorator_sequence() {
    assert_eq!(
        eval(
            "var Y={};\
             Object.defineProperties(Y,{aFG:{configurable:!0,enumerable:!0,get:function(){return 'fired'}}});\
             var W=Object.getOwnPropertyDescriptor(Y,'aFG'),q=W.get;\
             Object.defineProperty(Y,'__compute__aFG',{value:q,enumerable:!0});\
             W.get=void 0;Object.defineProperty(Y,'aFG',W);\
             Y.aFG+'|'+Y.__compute__aFG()"
        ),
        "undefined|fired"
    );
}
