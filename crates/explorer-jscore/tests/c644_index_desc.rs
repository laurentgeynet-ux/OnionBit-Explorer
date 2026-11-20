// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! JS-C644 — sémantique `[[DefineOwnProperty]]` : `SameValue` à la
//! redéfinition non-configurable (`NaN` égal, `-0`/`+0` distincts),
//! attributs par index de tableau persistés (`writable`,
//! `enumerable`, `configurable`), création de l'élément absent et
//! `ToObject` de la map `Object.defineProperties`.

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

/// `SameValue` à la re-définition d'une data-prop non-configurable
/// non-writable : `NaN`→`NaN` accepté, `-0`→`+0` refusé
/// (`Object/create/15.2.3.5-4-102`, `defineProperty/…4-85`).
#[test]
fn same_value_redefine() {
    assert_eq!(
        eval(
            r"var o={};Object.defineProperty(o,'x',{value:NaN});try{Object.defineProperty(o,'x',{value:NaN});1}catch(e){0}"
        ),
        "1"
    );
    assert_eq!(
        eval(
            r"var o={};Object.defineProperty(o,'x',{value:-0});try{Object.defineProperty(o,'x',{value:0});1}catch(e){e instanceof TypeError}"
        ),
        "true"
    );
    // Index de tableau figé : `SameValue` aussi (avant :
    // `strict_eq` rejetait `NaN`→`NaN`, acceptait `-0`→`+0`).
    assert_eq!(
        eval(
            r"var a=Object.freeze([NaN]);try{Object.defineProperty(a,'0',{value:NaN});1}catch(e){0}"
        ),
        "1"
    );
    assert_eq!(
        eval(
            r"var a=Object.freeze([-0]);try{Object.defineProperty(a,'0',{value:0});1}catch(e){e instanceof TypeError}"
        ),
        "true"
    );
}

/// Index absent défini par `defineProperty` : la prop est créée
/// avec les attributs demandés — `{value}` seul ⇒ `¬w,¬e,¬c`
/// servis par `getOwnPropertyDescriptor`, `[[Set]]` refuse,
/// `for-in` saute, `delete` refuse
/// (`Object/defineProperty/15.2.3.6-4-215`…).
#[test]
fn array_index_attrs() {
    assert_eq!(
        eval(
            r"var a=[];Object.defineProperty(a,'0',{value:9});var d=Object.getOwnPropertyDescriptor(a,'0');[d.writable,d.enumerable,d.configurable].join()"
        ),
        "false,false,false"
    );
    // `[[Set]]` sur index non-writable → refus (strict : TypeError).
    assert_eq!(
        eval(r"var a=[];Object.defineProperty(a,'0',{value:9});a[0]=5;a[0]"),
        "9"
    );
    assert_eq!(
        eval(
            r"'use strict';var a=[];Object.defineProperty(a,'0',{value:9});try{a[0]=5;0}catch(e){e instanceof TypeError}"
        ),
        "true"
    );
    // `delete` sur index non-configurable → false.
    assert_eq!(
        eval(r"var a=[];Object.defineProperty(a,'0',{value:9});delete a[0]"),
        "false"
    );
    // `enumerable:false` → invisible au `for-in` et `Object.keys`.
    assert_eq!(
        eval(
            r"var a=[];Object.defineProperty(a,'0',{value:9});var k=[];for(var x in a)k.push(x);k.join()+'|'+Object.keys(a).join()"
        ),
        "|"
    );
    // `enumerable:false` → `propertyIsEnumerable` faux,
    // `getOwnPropertyNames` conserve l'index.
    assert_eq!(
        eval(
            r"var a=[];Object.defineProperty(a,'0',{value:9});a.propertyIsEnumerable('0')+'|'+(Object.getOwnPropertyNames(a).indexOf('0')>=0)"
        ),
        "false|true"
    );
}

/// Élément existant : les flags absents du descripteur conservent
/// les valeurs courantes (spec `[[DefineOwnProperty]]`), donc
/// `{value}` seul sur un élément `FULL` reste `w/e/c`.
#[test]
fn array_index_existing_keeps_attrs() {
    assert_eq!(
        eval(
            r"var a=[1];Object.defineProperty(a,'0',{value:9});var d=Object.getOwnPropertyDescriptor(a,'0');[a[0],d.writable,d.enumerable,d.configurable].join()"
        ),
        "9,true,true,true"
    );
    // `{writable:false}` seul sur un configurable → seule l'écriture
    // est verrouillée ; la valeur reste et `delete` passe.
    assert_eq!(
        eval(
            r"var a=[1];Object.defineProperty(a,'0',{writable:false});a[0]=5;a[0]+'|'+(delete a[0])"
        ),
        "1|true"
    );
}

/// Redéfinition d'un index non-configurable : flags figés,
/// `writable:false`+valeur non `SameValue` refusée.
#[test]
fn array_index_redefine() {
    assert_eq!(
        eval(
            r"var a=[];Object.defineProperty(a,'0',{value:9});try{Object.defineProperty(a,'0',{enumerable:true});0}catch(e){e instanceof TypeError}"
        ),
        "true"
    );
    assert_eq!(
        eval(
            r"var a=[];Object.defineProperty(a,'0',{value:9});try{Object.defineProperty(a,'0',{value:8});0}catch(e){e instanceof TypeError}"
        ),
        "true"
    );
    // Valeur identique + flags absents : ré-affirmation acceptée
    // (`15.2.3.6-4-215` — `undefined`→`undefined` est `SameValue`).
    assert_eq!(
        eval(
            r"var a=[];Object.defineProperty(a,'0',{value:undefined});try{Object.defineProperty(a,'0',{value:undefined});1}catch(e){0}"
        ),
        "1"
    );
}

/// `defineProperty` sur index absent : la propriété est **créée**
/// (`undefined` si pas de `value`) — avant : `Ok(true)` sans
/// rien écrire (« 0 should be an own property »).
#[test]
fn array_index_absent_created() {
    assert_eq!(
        eval(
            r"var a=[];Object.defineProperty(a,'0',{});a.hasOwnProperty('0')+'|'+a[0]+'|'+a.length"
        ),
        "true|undefined|1"
    );
    // Hors `length` sur `length` non-writable → TypeError.
    assert_eq!(
        eval(
            r"var a=[1];Object.defineProperty(a,'length',{writable:false});try{Object.defineProperty(a,'5',{value:1});0}catch(e){e instanceof TypeError}"
        ),
        "true"
    );
}

/// `ToObject(Properties)` : `defineProperties(o, false)` wrappe la
/// primitive → aucune clé → no-op (`15.2.3.7-2-3`). `null` reste
/// un `TypeError`.
#[test]
fn define_props_toobject() {
    assert_eq!(
        eval(r"var o={x:1};try{Object.defineProperties(o,false);o.x}catch(e){0}"),
        "1"
    );
    assert_eq!(
        eval(r"var o={};try{Object.defineProperties(o,null);1}catch(e){e instanceof TypeError}"),
        "true"
    );
    // `Object.create` : même conversion sur la map.
    assert_eq!(
        eval(
            r"try{var o=Object.create(null,true);o instanceof Object}catch(e){e instanceof TypeError}"
        ),
        "false"
    );
}

/// `[[Set]]`/`[[Delete]]` sur index attributé après `seal`/`freeze`
/// — le plancher d'intégrité domine toujours la table.
#[test]
fn index_attrs_under_integrity() {
    // `writable:false` posé puis `freeze` : descriptor `¬w,¬c`.
    assert_eq!(
        eval(
            r"var a=[1];Object.defineProperty(a,'0',{value:9,writable:false,enumerable:true,configurable:true});Object.freeze(a);var d=Object.getOwnPropertyDescriptor(a,'0');[d.writable,d.configurable].join()"
        ),
        "false,false"
    );
    // `seal` seul garde `writable:true`.
    assert_eq!(
        eval(
            r"var a=[1];Object.defineProperty(a,'0',{value:9,enumerable:true,writable:true,configurable:true});Object.seal(a);var d=Object.getOwnPropertyDescriptor(a,'0');[d.writable,d.configurable].join()"
        ),
        "true,false"
    );
    // Ré-attributer sous freeze → TypeError (non-configurable).
    assert_eq!(
        eval(
            r"var a=Object.freeze([1]);try{Object.defineProperty(a,'0',{configurable:true});0}catch(e){e instanceof TypeError}"
        ),
        "true"
    );
}

/// Backing `dense` au-delà de `len` : `pop`/`delete` laissent des
/// trous périmés, `set_keep_len` (`arguments[i]=v` pour `i >= len`)
/// une vraie prop — `to_vec`/`reverse`/`apply` restent bornés à
/// `0..len` (panic `indexOf.call(arguments)` JS-C644).
#[test]
fn backing_beyond_len_bounded() {
    // `apply` matérialise via `to_vec` — après `pop` (trou terminal).
    assert_eq!(
        eval(r"var a=[1,2,3];a.pop();(function(x,y){return x+','+y}).apply(null,a)"),
        "1,2"
    );
    // `arguments[2]` posé hors `length` : `indexOf` ne le voit pas.
    assert_eq!(
        eval(
            r"(function(){arguments[2]=false;return Array.prototype.indexOf.call(arguments,true)+','+Array.prototype.indexOf.call(arguments,false)})(0,true)"
        ),
        "1,-1"
    );
    // `reverse` permute `0..len`, épargne la prop au-delà.
    assert_eq!(
        eval(
            r"(function(){arguments[3]=9;Array.prototype.reverse.call(arguments);return arguments[0]+','+arguments[1]+','+arguments[2]+','+arguments[3]})(1,2,3)"
        ),
        "3,2,1,9"
    );
}

/// `CreateDataPropertyOrThrow` des méthodes de tableau : le
/// résultat reçoit `{v, w:t, e:t, c:t}` en `[[DefineOwnProperty]]`
/// — la data-prop non-writable **héritée** ne bloque pas, l'index
/// own configurable est redéfini (`concat`/`slice`/`filter`/`map`).
#[test]
fn result_elems_are_create_data_property() {
    // Proto non-writable ignoré à l'écriture du résultat.
    assert_eq!(
        eval(
            r"Object.defineProperty(Array.prototype,'0',{value:100,writable:false,configurable:true});var r=[101].concat();var d=Object.getOwnPropertyDescriptor(r,'0');delete Array.prototype['0'];[r[0],d.writable].join()"
        ),
        "101,true"
    );
    // Index non-writable configurable du résultat species → redéfini.
    assert_eq!(
        eval(
            r"var a=[1];a.constructor={};a.constructor[Symbol.species]=function(){var q=[];Object.defineProperty(q,0,{value:0,writable:false,configurable:true,enumerable:false});return q};var r=a.filter(function(){return true});var d=Object.getOwnPropertyDescriptor(r,'0');[r[0],d.writable,d.enumerable,d.configurable].join()"
        ),
        "1,true,true,true"
    );
}
