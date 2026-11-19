// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Gate JS-C615 — constructions internes `OrdinaryCreateFrom
//! Constructor` hors `construct_nt` :
//!
//! - `Array.from` : `Construct(this)` quand `IsConstructor`
//!   (chemin itérateur **et** array-like), proto résolu par
//!   `GetPrototypeFromConstructor` (realm de `newTarget`) ;
//!   `GetMethod @@iterator` (non-callable → `TypeError`, plus
//!   de repli array-like) ; `IteratorClose` sur abrupt.
//! - `ArraySpeciesCreate` : lectures `constructor`/`@@species`
//!   observables + démotion cross-realm du `%Array%` distant
//!   (routage `splice` inclus).
//! - `eval-body` gen/async-gen : `f.prototype` non-objet →
//!   `%GeneratorPrototype%`/`%AsyncGeneratorPrototype%` du
//!   realm DE LA FONCTION.

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

/// `Array.from.call(C, it)` construit dans le realm de `C`
/// quand `C.prototype` n'est pas objet (test262
/// `Array/from/proto-from-ctor-realm`).
#[test]
fn from_construct_realm() {
    let out = run(
        "var other=$262.createRealm().global;var C=new other.Function();C.prototype=null;\
         var a=Array.from.call(C,[1,2]);\
         (Object.getPrototypeOf(a)===other.Object.prototype)+'|'+a[0]+'|'+a.length",
    );
    assert_eq!(out, "true|1|2");
}

/// `Array.from` non-ctor → `ArrayCreate` ordinaire ; `@@iterator`
/// non-callable → `TypeError` (plus de repli array-like).
#[test]
fn from_getmethod_iterator() {
    let out = run(
        "var r=[];\
         try{Array.from.call(function(){},[1,2])}catch(e){r.push('no')}r.push('ok');\
         try{Array.from({length:1,0:'x',[Symbol.iterator]:42});r.push('no2')}catch(e){r.push(e instanceof TypeError)}\
         r.join(',')",
    );
    assert_eq!(out, "ok,true");
}

/// `Array.from` array-like : `Construct(this, [len])` — le ctor
/// reçoit la longueur ; ordre `Get`/`map` intercalé (spec).
#[test]
fn from_arraylike_construct() {
    let out = run(
        "var args;function C(n){args=n}C.prototype=null;\
         var a=Array.from.call(C,{length:2,0:'x',1:'y'},function(v){return v+'!'});\
         args+'|'+(a.x===undefined)+'|'+a[0]+'|'+a.length+'|'+(Object.getPrototypeOf(a)===Object.prototype)",
    );
    // `C` retourne `undefined` → l'objet minté (proto
    // `%Object.prototype%` du realm de C — ici le nôtre).
    assert_eq!(out, "2|true|x!|2|true");
}

/// `IteratorClose` sur `mapfn` qui jette — `return` invoqué.
#[test]
fn from_iterator_close_on_abrupt() {
    let out = run(
        "var closed=false;var it={[Symbol.iterator](){return {next(){return {value:1,done:false}},return(){closed=true;return {}}}}};\
         try{Array.from(it,function(){throw 'x'})}catch(e){}closed",
    );
    assert_eq!(out, "true");
}

/// `ArraySpeciesCreate` : `%Array%` d'un autre realm → démoté,
/// `@@species` jamais lu (`callCount` 0) — `splice` routé.
#[test]
fn splice_species_cross_realm_demote() {
    let out = run("var callCount=0;var OA=$262.createRealm().global.Array;\
         var d={get:function(){callCount++}};var a=[1,2,3];a.constructor=OA;\
         Object.defineProperty(Array,Symbol.species,d);Object.defineProperty(OA,Symbol.species,d);\
         var r=a.splice(0,1);\
         (Object.getPrototypeOf(r)===Array.prototype)+'|'+callCount+'|'+r.length");
    assert_eq!(out, "true|0|1");
}

/// `ArraySpeciesCreate` : ctor non-Array d'autre realm conservé
/// — `@@species` lu dessus, résultat = `new C(len)` (proto du
/// ctor custom), getter `Array[@@species]` pas invoqué.
#[test]
fn splice_species_cross_realm_custom() {
    let out = run("var callCount=0;var OO=$262.createRealm().global.Object;\
         function C(){}var a=[1,2,3];a.constructor=OO;OO[Symbol.species]=C;\
         Object.defineProperty(Array,Symbol.species,{get:function(){callCount++}});\
         var r=a.splice(0,2);\
         (Object.getPrototypeOf(r)===C.prototype)+'|'+callCount+'|'+r[0]+'|'+r[1]+'|'+r.length");
    assert_eq!(out, "true|0|1|2|2");
}

/// `@@species` observable : un accesseur posé sur `constructor`
/// est bien invoqué par `splice`/`map` (avant : data-only,
/// invisible).
#[test]
fn species_getter_observable() {
    let out = run(
        "var n=0;function C(){return [9]}var a=[1,2];a.constructor={get [Symbol.species](){n++;return C}};\
         a.splice(0,1);a.map(function(){});n",
    );
    assert_eq!(out, "2");
}

/// `eval-body` : `g.prototype` non-objet → `%GeneratorPrototype%`
/// du realm de la fonction (test262 `generators/eval-body-
/// proto-realm`).
#[test]
fn gen_proto_realm() {
    let out = run("var other=$262.createRealm().global;\
         var g=other.eval('(0,function*(){})');\
         var GP=Object.getPrototypeOf(g.prototype);\
         var r=[];\
         g.prototype=null;r.push(Object.getPrototypeOf(g())===GP);\
         g.prototype=undefined;r.push(Object.getPrototypeOf(g())===GP);\
         g.prototype=Symbol();r.push(Object.getPrototypeOf(g())===GP);\
         g.prototype='s';r.push(Object.getPrototypeOf(g())===GP);\
         r.join(',')");
    assert_eq!(out, "true,true,true,true");
}

/// Idem `async function*` — `%AsyncGeneratorPrototype%`.
#[test]
fn agen_proto_realm() {
    let out = run("var other=$262.createRealm().global;\
         var g=other.eval('(0,async function*(){})');\
         var GP=Object.getPrototypeOf(g.prototype);\
         g.prototype=null;Object.getPrototypeOf(g())===GP");
    assert_eq!(out, "true");
}

/// `[[DefineOwnProperty]]` sur un index hors-borne d'array :
/// l'élément est **stocké** (bug préexistant — refus muet
/// `Ok(true)` sans écriture ; C615 le rend visible via
/// `CreateDataProperty` dans `Array.from`/`splice`).
#[test]
fn define_index_oob_stores() {
    let out = run(
        "var a=[];Object.defineProperty(a,'0',{value:9,writable:true,enumerable:true,configurable:true});\
         a.length+'|'+a[0]+'|'+JSON.stringify(a)",
    );
    assert_eq!(out, "1|9|[9]");
}

/// Régression : `g.prototype` objet → le proto de l'instance
/// est cet objet (comportement historique conservé).
#[test]
fn gen_proto_normal() {
    let out = run(
        "function*g(){}var it=g();\
         (Object.getPrototypeOf(it)===g.prototype)+'|'+(Object.getPrototypeOf(g.prototype)!==Object.prototype)",
    );
    assert_eq!(out, "true|true");
}
