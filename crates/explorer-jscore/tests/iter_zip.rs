// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Gate JS-C608 — `Iterator.zip`/`zipKeyed` **joint-iteration**
//! paresseux (proposal `joint-iteration`, ~126 fichiers test262).
//!
//! Avant : drain eager — `zip_iters` matérialisait
//! `Vec<Vec<Value>>` à la construction (le pas n'était pas
//! paresseux, `padding` était une *valeur* indexée au lieu
//! d'un itérateur, `strict` levait un `RangeError` a priori,
//! `zipKeyed` rendait des objets `Object.prototype` et
//! ignorait mode/clés énumérables/fermetures).
//!
//! Mécanisme : `IterOp::Zip { iters, keys, mode, pads }` —
//! records `(source, next, done)` ouverts à la construction
//! (spec), pas tiré un à un dans `it_helper_step`,
//! `IteratorCloseAll` en ordre inverse sur abrupt et
//! `helper.return()`/`throw()`.

use explorer_jscore::Engine;

/// Évalue `src` et décrit la valeur produite.
#[track_caller]
fn run(src: &str) -> String {
    let mut e = Engine::new();
    match e.eval(src) {
        Ok(v) => e.describe(v),
        Err(err) => panic!("eval -> {err}"),
    }
}

/// `shortest` (défaut) : lignes jusqu'au premier épuisé —
/// fraîches à chaque `next()` (pas de partage).
#[test]
fn shortest_lignes_fraiches() {
    let out = run("let z=Iterator.zip([[1,2],[10,20,30]]);\
         let a=z.next();a.value.push(99);\
         let b=z.next();\
         a.value+'|'+b.value+'|'+z.next().done+'|'+(a.value!==b.value)");
    assert_eq!(out, "1,10,99|2,20|true|true");
}

/// Paresse observable : rien n'est tiré du sous-jacent avant
/// `next()` — mais les records sont **ouverts** à la
/// construction (spec).
#[test]
fn ouverture_construction_pas_paresseux() {
    let out = run("let log=[];\
         function mk(n){return{[Symbol.iterator](){log.push('it'+n);\
         let i=0;return{next(){log.push('nx'+n);return i++<n?{value:i*10}:{done:true}}}}}};\
         let z=Iterator.zip([mk(2),mk(3)]);\
         let a=log.join(',');log=[];\
         let r=z.next().value+'|'+z.next().value+'|'+z.next().done;\
         a+'|'+log.join(',')+'|'+r");
    assert_eq!(out, "it2,it3|nx2,nx3,nx2,nx3,nx2|10,10|20,20|true");
}

/// Ordre des options : `mode` lu **avant** `padding`, et
/// `padding` jamais lu hors `longest` (get observable).
#[test]
fn options_mode_puis_padding() {
    let out = run("let log=[];\
         let o={get mode(){log.push('mode');return'longest'},\
                get padding(){log.push('pad');return[0]}};\
         Iterator.zip([[1],[2,3]],o).next();log.join(',')");
    assert_eq!(out, "mode,pad");
    let out2 = run("let n=0;\
         let o={get mode(){return'shortest'},get padding(){n++}};\
         Iterator.zip([[1]],o).next();n");
    assert_eq!(out2, "0");
}

/// Mode invalide → `TypeError` (pas `RangeError`) ; `strict`
/// sur longueurs différentes → `TypeError` au **pull**, pas
/// à la construction.
#[test]
fn modes_invalides_et_strict() {
    let out = run("try{Iterator.zip([[1]],{mode:'x'})}catch(e){e.name}");
    assert_eq!(out, "TypeError");
    let out2 = run("let z=Iterator.zip([[1,2],[3]],{mode:'strict'});\
         try{z.next().value+'/'+z.next().value}catch(e){e.name}");
    assert_eq!(out2, "TypeError");
    // Strict conforme : mêmes longueurs → itération normale.
    let out3 = run("let z=Iterator.zip([[1,2],[3,4]],{mode:'strict'});\
         z.next().value+'|'+z.next().value+'|'+z.next().done");
    assert_eq!(out3, "1,3|2,4|true");
}

/// `longest` : record fini → padding (itérateur, tiré au plus
/// `N` fois à la construction), `undefined` au-delà — et
/// continue tant qu'un record produit.
#[test]
fn longest_padding_iterateur() {
    let out = run("let z=Iterator.zip([[1,2],[10,20,30,40]],\
         {mode:'longest',padding:[0,1]});\
         let r='';for(let x=z.next();!x.done;x=z.next())r+=x.value+';';r");
    // Padding = N valeurs tirées **une fois** à la
    // construction — `pads[0]=0` fixe pour le record 0.
    assert_eq!(out, "1,10;2,20;0,30;0,40;");
    // Sans padding → `undefined`.
    let out2 = run("let z=Iterator.zip([[1],[9,8]],{mode:'longest'});\
         let a=z.next().value;let b=z.next().value;\
         a+'|'+b[0]+','+b[1]+'|'+z.next().done");
    assert_eq!(out2, "1,9|undefined,8|true");
}

/// `GetIteratorFlattenable` : record `{next}` nu accepté,
/// chaîne primitive acceptée, scalaire non-itérable rejeté.
#[test]
fn flattenable_entrees() {
    let out = run("let it={next(){return{done:true}}};\
         let z=Iterator.zip([[1,2],it]);\
         z.next().value+'|'+z.next().done");
    // Record déjà fini → `{done:true}` dès le 1er pas (spec
    // `shortest` : la ligne partielle est écartée).
    assert_eq!(out, "undefined|true");
    // `reject-primitives` : la chaîne primitive est REJETÉE
    // (seuls les objets — dont `new String` — passent).
    let out2 = run("try{Iterator.zip([[1],'ab'])}catch(e){e.name}");
    assert_eq!(out2, "TypeError");
    let out3 = run("try{Iterator.zip([42])}catch(e){e.name}");
    assert_eq!(out3, "TypeError");
}

/// `iterables` non-objet → `TypeError` avant la lecture des
/// options ; `iterables` non-itérable → `TypeError`.
#[test]
fn iterables_validation() {
    let out = run("try{Iterator.zip(42,{get mode(){throw 9}})}catch(e){e.name}");
    assert_eq!(out, "TypeError");
    let out2 = run("try{Iterator.zip({})}catch(e){e.name}");
    assert_eq!(out2, "TypeError");
}

/// `zipKeyed` : clés propres énumérables (+ symbols), objet
/// résultat à proto `null`, clé `undefined` omise, non-
/// énumérable ignorée.
#[test]
fn zip_keyed_resultats() {
    let out = run("let s=Symbol('s');\
         let src={a:[1,2],[s]:[9]};\
         Object.defineProperty(src,'c',{value:[7],enumerable:false});\
         let z=Iterator.zipKeyed(src);\
         let a=z.next().value;\
         (a.a+'|'+a[s]+'|'+('c'in a)+'|'+(Object.getPrototypeOf(a)===null))");
    assert_eq!(out, "1|9|false|true");
    // `undefined` → clé absente des résultats.
    let out2 = run("let z=Iterator.zipKeyed({a:[1],b:undefined,c:[3]});\
         let r=z.next().value;Object.keys(r)+'|'+('b'in r)");
    assert_eq!(out2, "a,c|false");
    // Mode `longest` — le padding est indexé **par clé**
    // (`Get(paddingOption, key)`, pas un itérateur).
    let out3 = run("let z=Iterator.zipKeyed({x:[1],y:[7,8]},\
         {mode:'longest',padding:{x:0}});\
         let a=z.next().value;let b=z.next().value;\
         a.x+','+a.y+'|'+b.x+','+b.y+'|'+z.next().done");
    assert_eq!(out3, "1,7|0,8|true");
}

/// `return()` sur le helper → `IteratorCloseAll` en ordre
/// **inverse** ; fin naturelle `shortest` sans close.
#[test]
fn fermetures_ordre_inverse() {
    let out = run("let log=[];\
         function mk(n){return{[Symbol.iterator](){return{\
         next(){return{value:0}},return(){log.push('r'+n);return{}}}}}};\
         let z=Iterator.zip([mk(1),mk(2),mk(3)]);\
         z.next();z.return();log.join(',')");
    assert_eq!(out, "r3,r2,r1");
    // Fin `shortest` : les records encore ouverts SONT fermés
    // (`IteratorCloseAll`, ordre inverse — spec joint-iteration).
    let out2 = run("let log=[];\
         function mk(n){return{[Symbol.iterator](){let i=0;return{\
         next(){return i++<n?{value:i}:{done:true}},\
         return(){log.push('r'+n);return{}}}}}};\
         let z=Iterator.zip([mk(1),mk(9)]);z.next();z.next();\
         log.join(',')+'/'+z.next().done");
    assert_eq!(out2, "r9/true");
}

/// Abrupt sur `next` du sous-jacent → records ouverts fermés
/// en ordre inverse puis propagation.
#[test]
fn abrupt_ferme_tout() {
    let out = run("let log=[];\
         function mk(id,boom){return{[Symbol.iterator](){let i=0;return{\
         next(){if(boom&&i++)throw'boom';return{value:i}},\
         return(){log.push('r'+id);return{}}}}}};\
         let z=Iterator.zip([mk(1,false),mk(2,true),mk(3,false)]);\
         z.next();try{z.next()}catch(e){log.push('thrown:'+e)};log.join(',')");
    // Le record fautif (2) sort d'openIters AVANT le close —
    // il n'est PAS refermé (spec `IteratorZip`).
    assert_eq!(out, "r3,r1,thrown:boom");
}
