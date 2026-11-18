// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Gate JS-C613 — arrays sparse.
//!
//! Tranche livrée (règles de validation, sans changement de
//! représentation) :
//! - `ArraySetLength` : `length` = `ToNumber` puis `ToUint32` —
//!   divergence → `RangeError` (`2**32`, négatif, fraction, NaN).
//! - `array_index` borné à 2**32-2 : `"4294967295"` et au-delà
//!   deviennent des props nommées ordinaires.
//!
//! Les tests `#[ignore]` forment le contrat d'acceptation de la
//! tranche représentation (`ObjKind::Array` dense+sparse, `len`
//! découplée du `Vec`) — à activer à l'atterrissage.

use explorer_jscore::Engine;

/// Évalue `src` et décrit la valeur produite.
#[track_caller]
fn run(src: &str) -> String {
    let mut e = Engine::new();
    match e.eval(src) {
        Ok(v) => e.describe(v),
        Err(err) => format!("ERR {err}"),
    }
}

/// `length` invalide → `RangeError` (spec `ArraySetLength`).
#[test]
fn length_invalide_range_error() {
    let out = run(
        "let r=[];\
         let a=[1,2,3];try{a.length=4294967296;r.push('no')}catch(e){r.push(e instanceof RangeError)}\
         let b=[1,2,3];try{b.length=4294967297;r.push('no')}catch(e){r.push(e instanceof RangeError)}\
         let c=[1,2,3];try{c.length=-1;r.push('no')}catch(e){r.push(e instanceof RangeError)}\
         let d=[1,2,3];try{d.length=2.5;r.push('no')}catch(e){r.push(e instanceof RangeError)}\
         let f=[1,2,3];try{f.length=NaN;r.push('no')}catch(e){r.push(e instanceof RangeError)}\
         r.join(',')",
    );
    assert_eq!(out, "true,true,true,true,true");
}

/// `length` non-numérique passe `ToNumber` : `"2"` tronque,
/// `null` vide, `true` → 1, objet sans `valueOf` → `RangeError`
/// (`ToNumber` → NaN ≠ `ToUint32` → 0).
#[test]
fn length_to_number() {
    let out = run(
        "let a=[1,2,3];a.length='2';let r=[a.length,a[0],a[1],a[2]];\
         let b=[1,2,3];b.length=null;r.push(b.length);\
         let c=[1,2,3];c.length=true;r.push(c.length);\
         let e=[1,2,3];try{e.length={};r.push('no')}catch(x){r.push(x instanceof RangeError)}\
         r.join(',')",
    );
    // `a[2]` tronqué → `undefined`, que `join` rend en chaîne vide.
    assert_eq!(out, "2,1,2,,0,1,true");
}

/// `"4294967295"` n'est pas un index : prop nommée ordinaire,
/// `length` inchangée, la clé existe bien et s'énumère.
#[test]
fn index_au_dela_du_max_nomme() {
    let out = run(
        "let a=[];a['4294967295']=1;\
         [a.length,a['4294967295'],'4294967295' in a,Object.keys(a).join('.'),a.hasOwnProperty('4294967295')].join(',')",
    );
    assert_eq!(out, "0,1,true,4294967295,true");
}

/// `4294967294` reste un index valide (2**32-2) — écriture sur
/// un array existant à petite capacité via prop nommée impossible ;
/// on vérifie la borne via `array_index` indirectement : la clé
/// doit entrer dans le chemin index (length s'ajuste) ou rester
/// nommée — ici la garde d'alloc refuse la densification.
#[test]
fn index_max_valide() {
    // `a[4294967294]=x` exige length=4294967295 : sparse à
    // l'atterrissage C613 — aujourd'hui `alloc_guard` RangeError.
    let out = run("let a=[];try{a[4294967294]=1;'no:'+a.length}catch(e){e instanceof RangeError}");
    assert_eq!(out, "true");
}

/// La valeur `length` négative en string passe aussi par
/// `ToNumber` — `"-1"` est rejetée comme `−1` numérique.
#[test]
fn length_string_negative() {
    let out = run("let a=[1,2,3];try{a.length='-1';'no'}catch(e){e instanceof RangeError}");
    assert_eq!(out, "true");
}

// ---- Contrat C613-représentation (à activer) ----

/// `a.length = 4294967295` réussit sans allocation : length
/// découplée du backing, relue à l'identique.
#[test]
#[ignore = "C613 représentation : len découplée du Vec"]
fn length_max_sparse() {
    let out = run("let a=[];a.length=4294967295;a.length");
    assert_eq!(out, "4294967295");
}

/// `new Array(4294967295)` : length géante, zéro élément, tout
/// trou — `'0' in a` faux, `a[0]` undefined.
#[test]
#[ignore = "C613 représentation : ctor sparse sans densification"]
fn ctor_geant_trous() {
    let out = run(
        "let a=new Array(4294967295);[a.length,'0' in a,a[0]===undefined,Object.keys(a).length].join(',')",
    );
    assert_eq!(out, "4294967295,false,true,0");
}

/// Écriture d'un index élevé : `a[2**30]=x` → `length = 2**30+1`
/// sans densifier — `keys`/`for-in`/JSON ne parcourent que le
/// seul élément réel.
#[test]
#[ignore = "C613 représentation : write haut ne densifie pas"]
fn index_haut_sans_densifier() {
    let out = run("let a=[];a[1073741824]='x';let n=0;for(let k in a)n++;\
         [a.length,a[1073741824],n,Object.keys(a).join('.'),'0' in a,a[0]===undefined].join(',')");
    assert_eq!(out, "1073741825,x,1,1073741824,false,true");
}

/// Réduction sparse : `length` descend sous les éléments réels
/// → ceux-ci disparaissent, y compris dans la map sparse.
#[test]
#[ignore = "C613 représentation : shrink traverse dense+sparse"]
fn shrink_traverse_sparse() {
    let out = run("let a=[1,2];a[100000]='x';a.length=3;\
         [a.length,a[0],a[1],'100000' in a,Object.keys(a).join('.')].join(',')");
    assert_eq!(out, "3,1,2,false,0.1");
}

/// `delete` sur index sparse + `in` / lecture traversent la map.
#[test]
#[ignore = "C613 représentation : delete sur index sparse"]
fn delete_index_sparse() {
    let out = run("let a=[];a[2**30]='x';delete a[2**30];\
         ['x'===a[2**30]?'bad':'ok',2**30 in a,a.length].join(',')");
    assert_eq!(out, "ok,false,1073741825");
}
