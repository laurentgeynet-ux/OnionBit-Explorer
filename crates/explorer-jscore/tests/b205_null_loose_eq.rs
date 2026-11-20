// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! §B-205 — `TypeError: The value of doNotTrack could not be cast to a
//! value that satisfies the schema type: "boolean"` (fr24, vol 958) :
//! le `isType` des schémas yup commence par `null==e` ; notre `==`
//! loose convertissait `Null` en `0` via `to_number` des deux côtés,
//! donc `null == false` était vrai → le cast rejetait le booléen
//! `false` produit par `"1"===navigator.doNotTrack||"yes"===…`.
//! Spec `IsLooselyEqual` : Bool → `ToNumber` puis re-tour récursif.
//! Verrous : `null`/`undefined` vs falsy, famille Bool×{Num,Str,Obj}
//! conservée, reproduction complète du `isType` yup.

use explorer_jscore::Engine;

fn ev(js: &str) -> String {
    let mut e = Engine::new();
    match e.eval(js) {
        Ok(v) => e.describe(v),
        Err(err) => panic!("{js} -> {err}"),
    }
}

#[test]
fn null_n_egale_pas_les_valeurs_falsy() {
    // Le trou : `null == false` rendait vrai.
    assert_eq!(ev("null==false"), "false");
    assert_eq!(ev("false==null"), "false");
    assert_eq!(ev("null==true"), "false");
    assert_eq!(ev("undefined==false"), "false");
    assert_eq!(ev("false==undefined"), "false");
    // Déjà corrects, verrouillés contre régression.
    assert_eq!(ev("null==0"), "false");
    assert_eq!(ev("null==''"), "false");
    assert_eq!(ev("null==NaN"), "false");
    assert_eq!(ev("null==null"), "true");
    assert_eq!(ev("null==undefined"), "true");
    assert_eq!(ev("undefined==null"), "true");
}

#[test]
fn famille_bool_conservee() {
    // Bool → ToNumber puis re-tour : Bool×Num.
    assert_eq!(ev("false==0"), "true");
    assert_eq!(ev("0==false"), "true");
    assert_eq!(ev("true==1"), "true");
    assert_eq!(ev("true==2"), "false");
    assert_eq!(ev("false==true"), "false");
    // Bool×Str : `ToNumber` des deux côtés au re-tour.
    assert_eq!(ev("false==''"), "true");
    assert_eq!(ev("false=='0'"), "true");
    assert_eq!(ev("true=='1'"), "true");
    // `"false"` → NaN, pas `0` : faux même en loose (piège courant).
    assert_eq!(ev("false=='false'"), "false");
    assert_eq!(ev("true=='true'"), "false");
    // Bool×Obj : `ToPrimitive` au re-tour — `[]` → `""` → `0`.
    assert_eq!(ev("false==[]"), "true");
    assert_eq!(ev("false=={}"), "false");
    // `!=` passe par le même chemin.
    assert_eq!(ev("null!=false"), "true");
    assert_eq!(ev("0!=false"), "false");
}

#[test]
fn cast_yup_boolean_sur_false() {
    // Verbatim raven/yup : `check` du schéma `boolean` + `isType` à
    // garde `null==e` + transform de coercion — le `false` produit par
    // `"1"===navigator.doNotTrack||"yes"===navigator.doNotTrack` doit
    // passer le cast.
    // `navigator` n'est pas installé dans `Engine::new()` — on
    // déclare l'exposition réelle (`doNotTrack` = `Null`,
    // dom_host.rs) ; l'expression rend `false` pareil.
    let js = "var navigator={doNotTrack:null};\
        var check=e=>(e instanceof Boolean&&(e=e.valueOf()),'boolean'==typeof e);\
        var isType=e=>null==e?false:check(e);\
        var transform=e=>{if(!isType(e)){\
            if(/^(true|1)$/i.test(String(e)))return!0;\
            if(/^(false|0)$/i.test(String(e)))return!1}return e};\
        var v='1'===navigator.doNotTrack||'yes'===navigator.doNotTrack;\
        var o=transform(v);isType(o)+'|'+v+'|'+o";
    let mut e = Engine::new();
    let out = match e.eval(js) {
        Ok(v) => e.describe(v),
        Err(err) => panic!("yup cast -> {err}"),
    };
    assert_eq!(out, "true|false|false");
}
