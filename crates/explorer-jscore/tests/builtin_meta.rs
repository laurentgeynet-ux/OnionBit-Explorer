// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Gate JS-C605 — métadonnées spec des fonctions builtin
//! (`fn.length` / `fn.name`).
//!
//! Avant : `native_fn` posait `length = 0` pour **toutes** les
//! natives et `install_arities` ne corrigeait que ~190 chemins
//! (ctors/statics boa, aucun `prototype.*`). Les accesseurs
//! (`get size`…) et les méthodes `@@x` échappaient à la table ;
//! `%GeneratorPrototype%` n'avait pas de `next` propre (arité
//! spec 1 vs 0 pour les protos d'itérateurs concrets).
//!
//! Mécanisme : `arities.rs` régénéré depuis le corpus test262
//! piné (`verifyProperty`/`sameValue` des `*/length.js` et
//! `*/name.js`) — `ARITIES`/`NAMES` par chemin
//! `globalThis`-relatif (segment `Symbol.x` → sentinelle,
//! racines `%XxxProto%` → intrinsèques) + `ACC_*` par
//! `(hôte, clé, get|set)` ; appliqués par `install_arities`
//! après tous les installs aux attrs `{w:false,e:false,c:true}`.

use explorer_jscore::Engine;

fn ev(src: &str) -> String {
    let mut e = Engine::new();
    match e.eval(src) {
        Ok(v) => format!("{v:?}"),
        Err(err) => format!("ERR {err}"),
    }
}

/// `fn.length` — méthodes `prototype`, statiques, ctors, globales.
#[test]
fn arites() {
    assert_eq!(ev("Array.prototype.push.length"), "Num(1.0)");
    assert_eq!(ev("Map.prototype.set.length"), "Num(2.0)");
    assert_eq!(ev("String.prototype.codePointAt.length"), "Num(1.0)");
    assert_eq!(ev("Number.prototype.toString.length"), "Num(1.0)");
    assert_eq!(ev("Array.from.length"), "Num(1.0)");
    assert_eq!(ev("Math.max.length"), "Num(2.0)");
    assert_eq!(ev("JSON.stringify.length"), "Num(3.0)");
    assert_eq!(ev("Promise.prototype.then.length"), "Num(2.0)");
    assert_eq!(ev("parseInt.length"), "Num(2.0)");
    assert_eq!(ev("parseFloat.length"), "Num(1.0)");
    assert_eq!(ev("Function.prototype.bind.length"), "Num(1.0)");
    assert_eq!(ev("Array.prototype[Symbol.iterator].length"), "Num(0.0)");
}

/// `fn.name` — méthodes, `@@x` (notation `[Symbol.x]`) et
/// fonctions partagées (`Array.prototype[Symbol.iterator]` est
/// `%Array.prototype.values%` → `"values"`, spec).
#[test]
fn noms() {
    assert_eq!(ev("Array.prototype.push.name"), "Str(\"push\")");
    assert_eq!(ev("JSON.stringify.name"), "Str(\"stringify\")");
    assert_eq!(
        ev("Iterator.prototype[Symbol.iterator].name"),
        "Str(\"[Symbol.iterator]\")"
    );
    assert_eq!(
        ev("Array.prototype[Symbol.iterator].name"),
        "Str(\"values\")"
    );
    assert_eq!(ev("Array.prototype.values.name"), "Str(\"values\")");
    assert_eq!(
        ev("String.prototype[Symbol.iterator].name"),
        "Str(\"[Symbol.iterator]\")"
    );
}

/// Accesseurs — `name` = `"get x"`/`"set x"`, `length` spec.
#[test]
fn accesseurs() {
    let g = "Object.getOwnPropertyDescriptor(Map.prototype,'size').get";
    assert_eq!(ev(&format!("{g}.length")), "Num(0.0)");
    assert_eq!(ev(&format!("{g}.name")), "Str(\"get size\")");
    let r = "Object.getOwnPropertyDescriptor(RegExp.prototype,'global').get";
    assert_eq!(ev(&format!("{r}.name")), "Str(\"get global\")");
    let s = "Object.getOwnPropertyDescriptor(Array,Symbol.species).get";
    assert_eq!(ev(&format!("{s}.length")), "Num(0.0)");
    assert_eq!(ev(&format!("{s}.name")), "Str(\"get [Symbol.species]\")");
}

/// Descripteurs — `length`/`name` restent
/// `{writable:false, enumerable:false, configurable:true}`.
#[test]
fn descripteurs() {
    assert_eq!(
        ev("Object.getOwnPropertyDescriptor(Array.prototype.push,'length').writable"),
        "Bool(false)"
    );
    assert_eq!(
        ev("Object.getOwnPropertyDescriptor(Array.prototype.push,'length').enumerable"),
        "Bool(false)"
    );
    assert_eq!(
        ev("Object.getOwnPropertyDescriptor(Array.prototype.push,'length').configurable"),
        "Bool(true)"
    );
    assert_eq!(
        ev("Object.getOwnPropertyDescriptor(Array.prototype.push,'name').configurable"),
        "Bool(true)"
    );
    assert_eq!(
        ev("Object.hasOwn(Array.prototype.push,'length')"),
        "Bool(true)"
    );
    assert_eq!(
        ev("Object.hasOwn(Array.prototype.push,'name')"),
        "Bool(true)"
    );
}

/// Intrinsèques `%XxxProto%` — `%GeneratorPrototype%.next` est
/// propre (arité 1) ; les protos d'itérateurs concrets gardent
/// le `next` partagé (arité 0).
#[test]
fn intrinseques() {
    assert_eq!(ev("(function*(){})().next.length"), "Num(1.0)");
    assert_eq!(ev("(function*(){})().throw.length"), "Num(1.0)");
    assert_eq!(ev("(async function*(){})().next.length"), "Num(1.0)");
    assert_eq!(
        ev("Object.hasOwn(Object.getPrototypeOf(function*(){}).prototype,'next')"),
        "Bool(true)"
    );
    assert_eq!(ev("[].keys().next.length"), "Num(0.0)");
}

/// `bind` — `length` = `max(0, cible.length − n)`, `name` =
/// `"bound " + cible.name` — désormais calculé sur les arités
/// **spec** (pas les `0` historiques).
#[test]
fn bind() {
    assert_eq!(ev("Array.prototype.push.bind(null).length"), "Num(1.0)");
    assert_eq!(ev("Array.prototype.push.bind(null,1).length"), "Num(0.0)");
    assert_eq!(
        ev("Array.prototype.push.bind(null).name"),
        "Str(\"bound push\")"
    );
    assert_eq!(ev("parseInt.bind(null).length"), "Num(2.0)");
    assert_eq!(ev("parseInt.bind(null,10).length"), "Num(1.0)");
}

/// Idempotence — les métadonnées survivent à un second
/// `Engine` (passe post-install déterministe).
#[test]
fn second_engine() {
    let mut e = Engine::new();
    assert_eq!(
        format!("{:?}", e.eval("Array.prototype.push.length")),
        "Ok(Num(1.0))"
    );
}
