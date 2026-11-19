// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Gate JS-C617 — `RegExp.prototype[Symbol.split]` spec complet :
//!
//! - `SpeciesConstructor` : `constructor` puis `@@species` lus
//!   par `Get` (observables), `Construct(C, «rx, flags+y»)`.
//! - `flags` lu par `Get` sur le RECEIVER ; `y` ajouté si
//!   absent, conservé sinon ; `u`/`v` pilotent
//!   `AdvanceStringIndex`.
//! - `RegExpExec` dynamique : `exec` callable observé,
//!   résultat non-objet non-null → `TypeError` ; sans `exec`
//!   callable → `RegExpBuiltinExec` (slot `[[RegExpMatcher]]`
//!   requis).
//! - `lastIndex` `Set` avant chaque exec, `Get`+`ToLength`
//!   après un match (un exec custom peut le déplacer).
//! - Captures intercalées (`Get` observable), `limit` en
//!   `ToUint32`, `limit === 0` → `[]`, chaîne vide →
//!   `[]`/`[""]`, résultat via `CreateDataPropertyOrThrow`.

use explorer_jscore::Engine;

/// Évalue `src` et décrit la valeur produite.
#[track_caller]
fn run(src: &str) -> String {
    let mut e = Engine::new();
    e.install_t262();
    match e.eval(src) {
        Ok(v) => e.describe(v),
        Err(err) => format!("ERR {err}"),
    }
}

/// Split basique : morceaux, captures intercalées, captures
/// non-appariées → `undefined`, pas de match → `[S]`.
#[test]
fn basic_and_captures() {
    let out = run("var r=[];\
         r.push('a-b-c'.split('-').join());\
         r.push(/([bc])/[Symbol.split]('a-b-c').join());\
         r.push(/(x)?-/[Symbol.split]('a-b').join());\
         r.push(/z/[Symbol.split]('abc').join());\
         r.join('|')");
    // Sticky : la pièce va de lastMatchEnd au point de départ
    // du match — 'a-' puis capture 'b', '-' puis 'c', queue ''.
    assert_eq!(out, "a,b,c|a-,b,-,c,|a,,b|abc");
}

/// Matches vides : pas de boucle infinie, les morceaux
/// adjacents à un match vide sont émis correctement.
#[test]
fn empty_matches() {
    let out = run("var r=[];\
         r.push(/(?:)/[Symbol.split]('ab').join());\
         r.push(new RegExp('')[Symbol.split]('abc').join());\
         r.push(/x*/[Symbol.split]('abxab').join());\
         r.join('|')");
    assert_eq!(out, "a,b|a,b,c|a,b,a,b");
}

/// Chaîne vide : `exec('')` match → `[]`, sinon `['']`.
/// `limit === 0` → `[]` (avant tout exec). `limit` converti
/// par `ToUint32` : -1 → 2³²-1, 2.9 → 2, NaN → 0, 2³² → 0.
#[test]
fn empty_string_and_limits() {
    let out = run("var r=[];\
         r.push(/(?:)/[Symbol.split]('').length);\
         r.push(/z/[Symbol.split]('').join());\
         r.push(/-/[Symbol.split]('a-b-c', 0).length);\
         r.push(/-/[Symbol.split]('a-b-c', 2.9).join());\
         r.push(/-/[Symbol.split]('a-b-c', -1).length);\
         r.push(/-/[Symbol.split]('a-b-c', NaN).length);\
         r.push(/-/[Symbol.split]('a-b-c', 4294967296).length);\
         r.join('|')");
    assert_eq!(out, "0||0|a,b|3|0|0");
}

/// `SpeciesConstructor` : `constructor` et `@@species` sont
/// lus par `Get` — accesseurs observables dans l'ordre —
/// puis `Construct(C, «rx, newFlags»)`.
#[test]
fn species_constructor_observed() {
    let out = run("var log=[];\
         var rx=/x/;\
         function C(a,b){log.push('ctor:'+b);\
           this.exec=function(s){var i=s.indexOf('x',this.lastIndex);\
             if(i<0)return null;this.lastIndex=i+1;var m=['x'];m.length=1;return m;};\
           this.lastIndex=0;}\
         C.prototype={};\
         Object.defineProperty(rx,'constructor',{get:function(){log.push('ctor');return C;}});\
         C[Symbol.species]=C;\
         Object.defineProperty(rx,'flags',{get:function(){log.push('flags');return '';}});\
         rx[Symbol.split]('axbxc').join()+':'+log.join(',')");
    // ctor get → flags get → Construct avec 'y' (flags '').
    // Pièces `substring(lastMatchEnd, q)` : le match custom
    // démarre à index > q → pièces '' + queue 'c'.
    assert_eq!(out, ",,c:ctor,flags,ctor:y");
}

/// `@@species` non-constructor → `TypeError` ;
/// `@@species` null/undefined → défaut `%RegExp%`.
#[test]
fn species_fallbacks() {
    let out = run("var r=[];\
         function tryit(f){try{return f()}catch(e){return e.name}}\
         var a=/x/;a.constructor={};a.constructor[Symbol.species]=42;\
         r.push(tryit(function(){a[Symbol.split]('axb')}));\
         var b=/x/;b.constructor={};b.constructor[Symbol.species]=null;\
         r.push(b[Symbol.split]('axb').join());\
         var c=/x/;c.constructor=undefined;\
         r.push(c[Symbol.split]('axb').join());\
         r.join('|')");
    assert_eq!(out, "TypeError|a,b|a,b");
}

/// `RegExpExec` : un `exec` custom sur le splitter est appelé
/// (pas l'engine natif) ; `lastIndex` écrit avant chaque
/// appel est observable ; `lastIndex` déplacé par exec est
/// relu pour `matchEnd`.
#[test]
fn custom_exec_observed() {
    let out = run("var calls=[];\
         var rx=/x/;\
         function C(){this.exec=function(s){\
           calls.push(this.lastIndex);\
           var i=s.indexOf('x',this.lastIndex);\
           if(i<0)return null;\
           this.lastIndex=i+1;return {index:i,length:1};};this.lastIndex=0;}\
         C[Symbol.species]=C;\
         rx.constructor=C;\
         rx[Symbol.split]('axbxc').join()+':'+calls.join(',')");
    assert_eq!(out, ",,c:0,2,4");
}

/// `exec` retournant une primitive → `TypeError` ; `null` →
/// pas de match à cette position (advance).
#[test]
fn exec_result_contract() {
    let out = run("var r=[];\
         var rx=/x/;var C=function(){};C[Symbol.species]=C;\
         C.prototype={exec:function(){return 5;}};\
         rx.constructor=C;\
         try{rx[Symbol.split]('axb')}catch(e){r.push(e.name)}\
         C.prototype={exec:function(){return undefined;}};\
         try{rx[Symbol.split]('axb')}catch(e){r.push(e.name)}\
         r.join('|')");
    assert_eq!(out, "TypeError|TypeError");
}

/// `y` conservé quand présent : `newFlags` transmis au ctor
/// porte les flags d'origine ; `y` ajouté sinon. Le ctor
/// reçoit bien `(rx, flags)` dans cet ordre.
#[test]
fn flags_to_constructor() {
    let out = run("var seen='';\
         var rx=/x/gi;\
         function C(a,b){seen=b;this.exec=a.exec;this.lastIndex=0;}\
         C[Symbol.species]=C;\
         rx.constructor=C;\
         rx[Symbol.split]('aXb');seen");
    // flags 'gi' + 'y' ajouté.
    assert_eq!(out, "giy");
}

/// `y` présent dans les flags → transmis sans duplication.
#[test]
fn sticky_flag_preserved() {
    let out = run("var seen='';\
         var rx=/x/uy;\
         function C(a,b){seen=b;this.exec=a.exec;this.lastIndex=0;}\
         C[Symbol.species]=C;\
         rx.constructor=C;\
         rx[Symbol.split]('ab');seen");
    assert_eq!(out, "uy");
}

/// `lastIndex` observé **après** exec : un exec qui déplace
/// `lastIndex` change `matchEnd` (et donc les morceaux).
#[test]
fn match_end_reread() {
    let out = run("var rx=/x/;\
         function C(){this.exec=function(s){\
           var i=s.indexOf('x',this.lastIndex);if(i<0)return null;\
           this.lastIndex=i+2;return {index:i,length:1};};this.lastIndex=0;}\
         C[Symbol.species]=C;\
         rx.constructor=C;\
         rx[Symbol.split]('aaxxbb').join()");
    // matchEnd = index+2 (déplacé) : pièce '' puis queue 'bb'.
    assert_eq!(out, ",bb");
}

/// Receiver non-objet → `TypeError` immédiat (avant toute
/// conversion de `string`).
#[test]
fn receiver_not_object() {
    let out = run("var r=[];\
         try{RegExp.prototype[Symbol.split].call('str','a,b')}catch(e){r.push(e.name)}\
         try{RegExp.prototype[Symbol.split].call(4,'a,b')}catch(e){r.push(e.name)}\
         r.join('|')");
    assert_eq!(out, "TypeError|TypeError");
}

/// `ToString(string)` : `undefined` → `"undefined"`.
#[test]
fn string_conversion_order() {
    let out = run("var r=[];\
         r.push(/,/[Symbol.split]('a,b').join());\
         r.push(/e/[Symbol.split]().length);\
         r.join('|')");
    // split(undefined) → split('undefined') → 3 morceaux.
    assert_eq!(out, "a,b|3");
}
