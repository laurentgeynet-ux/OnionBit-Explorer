// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Verrous JS-C308 — `arguments` paresseux : la cellule
//! `EnvCell::LazyArgs` ne matérialise l'objet qu'à la première
//! lecture ; l'écriture `arguments = v` remplace sans payer la
//! construction. Tous les comportements observables du chemin
//! eager sont verrouillés ici.

use explorer_jscore::Engine;

fn eval(src: &str) -> String {
    let mut e = Engine::new();
    match e.eval(src) {
        Ok(v) => e.describe(v),
        Err(err) => panic!("eval: {err}\n  src: {src}"),
    }
}

/// Lecture simple : contenu + length — matérialisation à la
/// demande, éléments identiques aux args passés.
#[test]
fn arguments_basic_content() {
    assert_eq!(
        eval("function f(){ return arguments.length+'/'+arguments[1] } f(7,8,9)"),
        "3/8"
    );
    assert_eq!(
        eval("function f(){ var s=0; for(var i=0;i<arguments.length;i++) s+=arguments[i]; return s } f(1,2,3,4)"),
        "10"
    );
}

/// `arguments === arguments` — la matérialisation remplace la
/// cellule par le Bind : l'identité de l'objet est stable entre
/// deux lectures (spécification du chemin eager).
#[test]
fn arguments_identity_stable() {
    assert_eq!(
        eval("function f(){ return arguments===arguments } f(1)"),
        "true"
    );
    assert_eq!(
        eval("function f(){ var a=arguments; var b=arguments; a.x=42; return b.x } f(1)"),
        "42"
    );
}

/// Map sloppy : `arguments[i]` et le paramètre `i` partagent le
/// slot env (spec `CreateMappedArgumentsObject`) — écrire l'un
/// change l'autre, dans les deux sens.
#[test]
fn arguments_mapped_aliasing_sloppy() {
    assert_eq!(
        eval("function f(a){ arguments[0]=99; return a } f(1)"),
        "99"
    );
    assert_eq!(
        eval("function f(a){ a=77; return arguments[0] } f(1)"),
        "77"
    );
    // Le mapping ne couvre que les indices < params.len() —
    // `arguments[1]` n'aliasse pas `a` (paramètre 0 seul).
    assert_eq!(
        eval("function f(a){ arguments[1]=5; return typeof arguments[1]+'/'+a } f(1,2)"),
        "number/1"
    );
}

/// `arguments.callee` sloppy = la fonction (Annex B) ; strict =
/// prop absente (`undefined`, tolérance documentée vs poisoned
/// accessor spec).
#[test]
fn arguments_callee() {
    assert_eq!(
        eval("function f(){ return arguments.callee===f } f()"),
        "true"
    );
    assert_eq!(
        eval("'use strict'; function f(){ return arguments.callee } f()"),
        "undefined"
    );
}

/// Strict : `arguments` existe et énumère les args, mais sans
/// aliasing map ni `callee` — `arguments[0]=9` ne mute pas le
/// paramètre.
#[test]
fn arguments_strict_unmapped() {
    assert_eq!(
        eval("'use strict'; function f(a){ arguments[0]=9; return a+'/'+arguments[0] } f(1)"),
        "1/9"
    );
}

/// Écriture `arguments = v` : la cellule paresseuse est
/// remplacée sans matérialiser — la valeur assignée se lit
/// ensuite normalement.
#[test]
fn arguments_write_replaces() {
    assert_eq!(
        eval("function f(){ arguments=42; return arguments } f(1,2)"),
        "42"
    );
}

/// Closure échappée : la liaison paresseuse survit au retour —
/// la lecture post-call matérialise avec les mêmes args.
#[test]
fn arguments_escaped_closure() {
    assert_eq!(
        eval("var g=function f(){ return function(){ return arguments[0]+arguments.length } }(); g(5,6)"),
        "7"
    );
}

/// Arrow : pas de `arguments` propre — la lecture remonte au
/// scope englobant (spec).
#[test]
fn arguments_arrow_falls_through() {
    assert_eq!(
        eval("function f(){ var g=()=>arguments.length; return g() } f(1,2,3)"),
        "3"
    );
}

/// `eval` dans le corps lit la même liaison matérialisée.
#[test]
fn arguments_via_eval() {
    assert_eq!(
        eval("function f(){ return eval('arguments.length+arguments[0]') } f(4,5)"),
        "6"
    );
}

/// `Array.prototype.slice` et copie : `arguments` se comporte
/// comme un array-like ordinaire après matérialisation.
#[test]
fn arguments_array_like() {
    assert_eq!(
        eval("function f(){ return Array.prototype.slice.call(arguments,1).join(',') } f(1,2,3)"),
        "2,3"
    );
}

/// Une fonction qui ne lit jamais `arguments` n'a pas d'effet
/// observable du lazy (retour, `this`, paramètres inchangés).
#[test]
fn arguments_unread_no_effect() {
    assert_eq!(
        eval("function f(a,b){ return a*b+this.k } var o={k:10,f:f}; o.f(3,4)"),
        "22"
    );
}
