// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Gate JS-C606 — sémantiques indexées des `TypedArray`s
//! (internal methods `IntegerIndexedExotic`).
//!
//! Avant : l'index était reconnu par `array_index` (décimaux
//! canoniques seulement) — pas de `CanonicalNumericIndexString`,
//! pas de bras TA dans `defineProperty`/`hasOwn`/`in`, et
//! `Reflect.set`/`Reflect.defineProperty` ne respectaient ni le
//! receiver ni le retour booléen spec.
//!
//! Mécanisme : `canonical_numeric_index` (ops.rs) distingue
//! index canonique valide / canonique invalide (`-0`, négatifs,
//! fractionnaires, `NaN`…) / non-numérique (clé ordinaire) ;
//! tous les points d'entrée (`find_own`, `set`, `delete`,
//! `hasOwn`, `defineProperty`, `Reflect.*`) consomment la même
//! classification. `TypedArraySetElement` convertit la valeur
//! **avant** la borne — les coercitions hostiles lèvent même
//! sur clé avalée.

use explorer_jscore::Engine;

fn ev(src: &str) -> String {
    let mut e = Engine::new();
    match e.eval(src) {
        Ok(v) => format!("{v:?}"),
        Err(err) => format!("ERR {err}"),
    }
}

/// `hasOwn`/`hasOwnProperty`/`in` — seuls les indices valides
/// in-range sont des props own.
#[test]
fn has_own_et_in() {
    assert_eq!(
        ev("var t=new Uint8Array(2);Object.hasOwn(t,'0')+'|'+Object.hasOwn(t,'5')+'|'+Object.hasOwn(t,'-1')"),
        "Str(\"true|false|false\")"
    );
    assert_eq!(
        ev("var t=new Uint8Array(2);t.hasOwnProperty('0')+'|'+t.hasOwnProperty('-0')+'|'+t.hasOwnProperty('1.5')"),
        "Str(\"true|false|false\")"
    );
    assert_eq!(
        ev("var t=new Uint8Array(2);('0' in t)+'|'+('1' in t)+'|'+('5' in t)+'|'+('-1' in t)"),
        "Str(\"true|true|false|false\")"
    );
}

/// `defineProperty` sur index valide : écrit l'élément ; descripteur
/// incompatible ou index invalide → `TypeError` / false.
#[test]
fn define_property_index() {
    assert_eq!(
        ev("var t=new Uint8Array(2);Object.defineProperty(t,'0',{value:5});t[0]"),
        "Num(5.0)"
    );
    assert_eq!(
        ev("var t=new Uint8Array(2);Object.defineProperty(t,'0',{value:5,writable:true,enumerable:true,configurable:true});t[0]"),
        "Num(5.0)"
    );
    for desc in [
        "{configurable:false}",
        "{enumerable:false}",
        "{writable:false}",
        "{get(){return 1}}",
    ] {
        assert_eq!(
            ev(&format!(
                "var t=new Uint8Array(2);try{{Object.defineProperty(t,'0',{desc})}}catch(e){{e.constructor.name}}"
            )),
            "Str(\"TypeError\")",
            "desc {desc}"
        );
    }
    // Index canonique invalide (OOB, -0, fractionnaire) → refus.
    for key in ["'9'", "'-0'", "'-1'", "'1.5'"] {
        assert_eq!(
            ev(&format!(
                "var t=new Uint8Array(2);try{{Object.defineProperty(t,{key},{{value:5}})}}catch(e){{e.constructor.name}}"
            )),
            "Str(\"TypeError\")",
            "key {key}"
        );
    }
    assert_eq!(
        ev("var t=new Uint8Array(2);Reflect.defineProperty(t,'9',{value:5})+'|'+Reflect.defineProperty(t,'0',{value:7})+'|'+t[0]"),
        "Str(\"false|true|7\")"
    );
}

/// `[[Set]]` : index canonique invalide avalé (succès, pas de prop)
/// mais la coercition de valeur se fait avant la borne.
#[test]
fn set_canonique_avale() {
    for key in ["'-0'", "'-1'", "'1.5'", "'NaN'", "'Infinity'"] {
        assert_eq!(
            ev(&format!(
                "var t=new Uint8Array(2);t[{key}]=5;Object.hasOwn(t,{key})+'|'+t[{key}]"
            )),
            "Str(\"false|undefined\")",
            "key {key}"
        );
    }
    // ToNumber se produit même sur clé avalée.
    for key in ["'-0'", "'-1'", "'9'"] {
        assert_eq!(
            ev(&format!(
                "var t=new Uint8Array(2);try{{t[{key}]={{valueOf(){{throw 'x'}}}}}}catch(e){{'threw:'+e}}"
            )),
            "Str(\"threw:x\")",
            "key {key}"
        );
    }
}

/// Clés non-canoniques (`01`, `1.0`, `+1`) → prop ordinaire.
#[test]
fn cles_non_canoniques_ordinaires() {
    for key in ["'01'", "'1.0'", "'+1'", "'0.0000001'"] {
        assert_eq!(
            ev(&format!(
                "var t=new Uint8Array(2);t[{key}]=5;Object.hasOwn(t,{key})+'|'+t[{key}]"
            )),
            "Str(\"true|5\")",
            "key {key}"
        );
    }
    // La valeur n'est pas coercée pour une prop ordinaire.
    assert_eq!(
        ev("var t=new Uint8Array(2);var o={valueOf(){return 5}};t['1.0']=o;t['1.0']===o"),
        "Bool(true)"
    );
}

/// `delete` : index valide → false ; invalide/non-canonique → true.
#[test]
fn delete_index() {
    assert_eq!(
        ev("var t=new Uint8Array(2);delete t[0]+'|'+delete t['5']+'|'+delete t['-0']+'|'+delete t['-1']+'|'+delete t['1.0']"),
        "Str(\"false|true|true|true|true\")"
    );
}

/// `Reflect.set` : booléen spec + receiver `SameValue(O,R)`.
#[test]
fn reflect_set_receiver() {
    assert_eq!(
        ev("var t=new Uint8Array(2);Reflect.set(t,'0',5)+'|'+t[0]"),
        "Str(\"true|5\")"
    );
    // Canonique invalide → true sans écriture.
    assert_eq!(
        ev("var t=new Uint8Array(2);Reflect.set(t,'9',5)"),
        "Bool(true)"
    );
    assert_eq!(
        ev("var t=new Uint8Array(2);Reflect.set(t,'-1',5)"),
        "Bool(true)"
    );
    // Receiver ≠ O : pas d'écriture indexée sur O — prop ordinaire
    // sur le receiver.
    assert_eq!(
        ev("var t=new Uint8Array(2);var r={};Reflect.set(t,'0',9,r)+'|'+t[0]+'|'+r[0]"),
        "Str(\"true|0|9\")"
    );
    assert_eq!(
        ev("var t=new Uint8Array(2);var r={};Reflect.set(t,'9',9,r)+'|'+('9' in r)"),
        "Str(\"true|false\")"
    );
    // Écriture via proto : prop own créée sur le receiver.
    assert_eq!(
        ev("var t=new Uint8Array(2);var r=Object.create(t);r[0]=77;t[0]+'|'+r[0]+'|'+Object.hasOwn(r,'0')"),
        "Str(\"0|77|true\")"
    );
    // Refus ordinaire propagé.
    assert_eq!(
        ev("var t=new Uint8Array(2);Object.defineProperty(t,'x',{writable:false,value:1});Reflect.set(t,'x',42)"),
        "Bool(false)"
    );
}

/// `BigInt` TA : `ToBigInt` avant borne, mêmes règles indexées.
#[test]
fn bigint_ta() {
    assert_eq!(
        ev("var t=new BigInt64Array(2);t[0]=5n;t['-1']=9n;t[0]+'|'+Object.hasOwn(t,'-1')"),
        "Str(\"5|false\")"
    );
    assert_eq!(
        ev("var t=new BigInt64Array(2);try{t[0]=1}catch(e){e.constructor.name}"),
        "Str(\"TypeError\")"
    );
    assert_eq!(
        ev("var t=new BigInt64Array(2);try{t['-1']=1}catch(e){e.constructor.name}"),
        "Str(\"TypeError\")"
    );
}

/// Lien TA dans une chaîne proto : les clés canoniques
/// invalides shadowent le proto — get→undefined, has→false,
/// set→succès avalé, jamais d'accesseur `%TypedArray%.prototype`.
#[test]
fn shadow_proto_canonique() {
    // `TypedArray.prototype[k]` posé sciemment n'apparaît pas.
    assert_eq!(
        ev("Uint8Array.prototype[1]='x';var t=new Uint8Array(1);Reflect.has(t,'1')+'|'+Reflect.has(t,'-1')+'|'+Reflect.has(t,'42')"),
        "Str(\"false|false|false\")"
    );
    // Getter/setter proto inatteignables (canonique invalide).
    assert_eq!(
        ev("var t=new Float64Array(1);var hit=0;Object.defineProperty(Float64Array.prototype,1,{get(){hit++;return 9},set(v){hit++},configurable:true});var r=Object.create(t);r[1]=3;t[1]+'|'+hit+'|'+Reflect.get(t,1)+'|'+Reflect.has(t,1)"),
        "Str(\"undefined|0|undefined|false\")"
    );
    // Héritage non-numérique intact.
    assert_eq!(
        ev("Uint8Array.prototype.foo=42;var t=new Uint8Array(1);Reflect.has(t,'foo')+'|'+Reflect.has(t,'subarray')"),
        "Str(\"true|true\")"
    );
}

/// Receiver exotique : proxy (trap `defineProperty` ×1), TA
/// court (refus), non-extensible, receiver primitif.
#[test]
fn receivers_exotiques() {
    assert_eq!(
        ev("var t=new Uint8Array(1);var calls=0;var r=new Proxy(Object.create(t),{defineProperty(tg,k,d){calls++;Object.defineProperty(tg,k,d);return true}});r[0]=5;calls+'|'+r[0]+'|'+t[0]"),
        "Str(\"1|5|0\")"
    );
    // Canonique invalide : la trap n'est même pas appelée.
    assert_eq!(
        ev("var t=new Uint8Array(1);var calls=0;var r=new Proxy(Object.create(t),{defineProperty(tg,k,d){calls++;return true}});r['-0']=5;calls"),
        "Num(0.0)"
    );
    // Receiver TA trop court : `[[DefineOwnProperty]]` refuse.
    assert_eq!(
        ev("var t=new Uint8Array(1);var s=new Uint8Array(0);Reflect.set(t,'0',9,s)+'|'+t[0]+'|'+s[0]"),
        "Str(\"false|0|undefined\")"
    );
    // Receiver primitif : canonique invalide → succès, pas de
    // coercition de la valeur.
    assert_eq!(
        ev("var t=new Int32Array(10);Reflect.set(t,100,{valueOf(){throw 'x'}},'notobj')"),
        "Bool(true)"
    );
    // Receiver Array : `[[DefineOwnProperty]]` d'Array crée
    // l'élément et pousse `length`.
    assert_eq!(
        ev("var t=new Uint8Array(1);var r=Object.setPrototypeOf([],t);r[0]=5;r[0]+'|'+r.length+'|'+t[0]"),
        "Str(\"5|1|0\")"
    );
}

/// Buffer détaché : toute validité indexée devient `false`.
#[test]
fn detached_live_bounds() {
    assert_eq!(
        ev("var b=new ArrayBuffer(8);var t=new Uint8Array(b);b.transfer(0);t[0]+'|'+Object.hasOwn(t,'0')+'|'+Reflect.has(t,'0')"),
        "Str(\"undefined|false|false\")"
    );
    // Écriture avalée (ElementSet → invalide → succès muet).
    assert_eq!(
        ev("var b=new ArrayBuffer(8);var t=new Uint8Array(b);b.transfer(0);t[0]=5;t[0]"),
        "Undefined"
    );
}

/// Vues sur buffer resizable : `auto` suit les resize, `fixed`
/// devient hors-bornes (spec `TypedArrayLength`, JS-C606).
#[test]
fn rab_live_length() {
    assert_eq!(
        ev("var r=new ArrayBuffer(24,{maxByteLength:64});var a=new Float64Array(r,8);r.resize(48);a.length"),
        "Num(5.0)"
    );
    // Vue fixed : shrink sous la fin déclarée → hors-bornes totale.
    assert_eq!(
        ev("var r=new ArrayBuffer(24,{maxByteLength:64});var f=new Float64Array(r,0,2);r.resize(8);f.length+'|'+(0 in f)+'|'+Object.keys(f).join(',')"),
        "Str(\"0|false|\")"
    );
    // Auto : index re-entrant après re-grow → écriture vivante.
    assert_eq!(
        ev("var r=new ArrayBuffer(24,{maxByteLength:64});var a=new Float64Array(r,8);r.resize(16);a[0]=7;r.resize(48);a[0]+'|'+a.length"),
        "Str(\"7|5\")"
    );
}

/// `getOwnPropertyDescriptor`, `-0`, freeze/seal.
#[test]
fn descripteurs_et_gel() {
    assert_eq!(
        ev("var t=new Uint8Array(2);Object.getOwnPropertyDescriptor(t,'-0')===undefined"),
        "Bool(true)"
    );
    assert_eq!(
        ev("var t=new Uint8Array(2);var d=Object.getOwnPropertyDescriptor(t,'0');d.writable+'|'+d.enumerable+'|'+d.configurable"),
        "Str(\"true|true|true\")"
    );
    assert_eq!(
        ev("var t=new Uint8Array(2);Object.getOwnPropertyNames(t).join(',')"),
        "Str(\"0,1\")"
    );
    assert_eq!(
        ev("var t=new Uint8Array(2);Object.preventExtensions(t);Object.isFrozen(t)+'|'+Object.isSealed(t)+'|'+Object.isExtensible(t)"),
        "Str(\"false|true|false\")"
    );
    assert_eq!(
        ev("var t=new Uint8Array(0);Object.freeze(t);Object.isFrozen(t)"),
        "Bool(true)"
    );
    assert_eq!(
        ev("var t=new Uint8Array(2);try{Object.freeze(t)}catch(e){e.constructor.name}"),
        "Str(\"TypeError\")"
    );
}
