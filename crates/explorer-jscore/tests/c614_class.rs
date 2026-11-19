// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Gate JS-C614 — early errors du corps de classe (spec
//! `ClassElement`/`ClassBody`) : `delete` sur référence privée,
//! `constructor` unique, noms privés uniques (paire get/set
//! seule), noms réservés, `super()` sans héritage, `super.#x`,
//! `AllPrivateNamesValid` (`#x` jamais déclaré), `;` exigée
//! après un champ.
//!
//! Contrat : rejeter exactement ce que la spec exige — jamais
//! plus (le code légal des suites est un verrou
//! anti-sur-enforcement).

use explorer_jscore::Engine;

fn ev(src: &str) -> String {
    let mut e = Engine::new();
    match e.eval(src) {
        Ok(v) => format!("{v:?}"),
        Err(err) => format!("ERR {err}"),
    }
}

/// `src` doit être une erreur de syntaxe.
fn is_syn_err(src: &str) -> bool {
    let s = ev(src);
    s.starts_with("ERR") && s.contains("syntax error")
}

#[test]
fn delete_private_refs() {
    for (src, expect_err) in [
        ("class C { #x = 1; m() { delete this.#x } }", true),
        ("class C { #x = 1; m() { delete o.#x } }", true),
        ("class C { #x = 1; m() { delete #x } }", true),
        ("class C { #x(){}; f = delete this.#x }", true),
        ("class C { #x = 1; m() { delete f().#x } }", true),
        ("class C { #x = 1; m() { delete o?.#x } }", true),
        // Légal : prop calculée, membre public, chaîne ordinaire.
        (
            "class C { #x = 1; m() { return delete this['#x'] } }",
            false,
        ),
        ("class C { x = 1; m() { return delete this.x } }", false),
        ("class C { m() { return delete o['#x'] } }", false),
    ] {
        assert_eq!(is_syn_err(src), expect_err, "{src}");
    }
}

#[test]
fn private_name_dups() {
    for (src, expect_err) in [
        ("class C { #x; #x; }", true),
        ("class C { #x(){} #x(){} }", true),
        ("class C { get #x(){} get #x(){} }", true),
        ("class C { set #x(v){} set #x(v){} }", true),
        ("class C { get #x(){} #x; }", true),
        ("class C { #x; set #x(v){} }", true),
        ("class C { #x(){} set #x(v){} }", true),
        // Statique et instance partagent le namespace privé.
        ("class C { #x; static #x; }", true),
        ("class C { static #x; #x(){} }", true),
        // Paire get/set licite — dans les deux ordres.
        ("class C { get #x(){return 1} set #x(v){} }", false),
        ("class C { set #x(v){} get #x(){return 1} }", false),
        (
            "class C { static get #x(){return 1} static set #x(v){} }",
            false,
        ),
        // Le namespace privé ignore `static` : champ + accesseur
        // = doublon même entre statique et instance.
        ("class C { static #x = 1; get #x(){return 0} }", true),
        (
            "class C { static #x = 1
get #x(){return 0} }",
            true,
        ),
        // Des noms publics homonymes sont libres.
        ("class C { #x; x; }", false),
        ("class C { get x(){} get x(){} }", false),
        ("class C { m(){} static m(){} }", false),
    ] {
        assert_eq!(is_syn_err(src), expect_err, "{src}");
    }
}

#[test]
fn ctor_and_reserved_names() {
    for (src, expect_err) in [
        ("class C { constructor(){} constructor(){} }", true),
        // `static constructor(){}` est LÉGAL — seule
        // `FieldDefinition` et `SpecialMethod` sont bannies
        // (`prototype` seul est interdit en statique).
        ("class C { static constructor(){} }", false),
        ("class C { static constructor(){} constructor(){} }", false),
        ("class C { static prototype(){} }", true),
        ("class C { static prototype = 1 }", true),
        ("class C { static constructor = 1 }", true),
        ("class C { get constructor(){} }", true),
        ("class C { set constructor(v){} }", true),
        ("class C { async constructor(){} }", true),
        ("class C { *constructor(){} }", true),
        ("class C { constructor = 1 }", true),
        ("class C { #constructor; }", true),
        ("class C { #constructor(){} }", true),
        // Légal : clés calculées (pas de PropName statique),
        // `prototype` d'instance, un seul ctor.
        ("class C { ['constructor'](){} constructor(){} }", false),
        ("class C { constructor(){} }", false),
        ("class C { prototype(){} }", false),
        // `static` lève toute restriction `constructor` sur les
        // MethodDefinition — même les spéciales sont licites.
        ("class C { static *constructor(){} }", false),
        ("class C { static async constructor(){} }", false),
        ("class C { static async *constructor(){} }", false),
        ("class C { static get constructor(){} }", false),
        ("class C { static set constructor(v){} }", false),
        ("class C { prototype = 1 }", false),
        ("class C { constructor2(){} }", false),
        ("class C { static static(){} }", false),
    ] {
        assert_eq!(is_syn_err(src), expect_err, "{src}");
    }
}

#[test]
fn super_call_requires_heritage() {
    for (src, expect_err) in [
        ("class C { constructor(){ super() } }", true),
        ("class C { constructor(x = super()){} }", true),
        ("class C { constructor(){ let f = () => super() } }", true),
        ("class C { constructor(){ function g(){ super() } } }", true),
        // `super.x` reste légal dans toute méthode.
        ("class C { m(){ return super.x } }", false),
        ("class C { constructor(){ super.x } }", false),
        // `super(` banni aussi hors ctor de classe dérivée :
        // méthodes, accesseurs, inits de champ, blocs static.
        ("class C extends Object { m(){ super() } }", true),
        ("class C extends Object { static m(){ super() } }", true),
        ("class C extends Object { get m(){ super() } }", true),
        ("class C extends Object { #m(){ super() } }", true),
        ("class C extends Object { x = super() }", true),
        ("class C extends Object { static { super() } }", true),
        ("class C extends Object { m(){ let f = () => super() } }", true),
        ("class C extends Object { m(){ function g(){ super() } } }", true),
        ("class C { m(){ super() } }", true),
        // `super.x` dans méthodes/champs/static : toujours légal.
        ("class C extends Object { x = super.x }", false),
        ("class C extends Object { static { super.x } }", false),
        ("class C extends Object { m(){ return () => super.x } }", false),
        // Classe dérivée imbriquée dans une méthode : son super
        // est licite, pas celui du dehors.
        ("class C extends Object { m(){ class D extends Object { constructor(){ super() } } } }", false),
        ("class C extends Object { m(){ class D extends Object { constructor(){ super() } } super() } }", true),
        // Dérivée : légal, y compris via flèche et défauts.
        ("class C extends Object { constructor(){ super() } }", false),
        ("class C extends Object { constructor(){ let f=()=>0; super.x } }", false),
        // Classe dérivée imbriquée dans un ctor non-dérivé :
        // son propre super est licite, pas celui du dehors.
        (
            "class C { constructor(){ class D extends Object { constructor(){ super() } } } }",
            false,
        ),
        (
            "class C { constructor(){ class D extends Object { constructor(){ super() } } super() } }",
            true,
        ),
    ] {
        assert_eq!(is_syn_err(src), expect_err, "{src}");
    }
}

#[test]
fn super_private_access() {
    for (src, expect_err) in [
        ("class C { #m = 1; m(){ return super.#m } }", true),
        ("class C { #m(){} n(){ return super.#m() } }", true),
        ("class C { m(){ return super.x } }", false),
        ("class C { m(){ return super['x'] } }", false),
    ] {
        assert_eq!(is_syn_err(src), expect_err, "{src}");
    }
}

#[test]
fn private_names_valid() {
    for (src, expect_err) in [
        // Référence jamais déclarée → `AllPrivateNamesValid`.
        ("class C { f = this.#x }", true),
        ("class C { f = (() => {})().#x }", true),
        ("class C { m(){ return o.#y } }", true),
        ("class C extends class { x = this.#foo; } { #foo; }", true),
        ("o.#x", true),
        // Déclaré dans l'englobante : légal.
        ("class C { #x = 1; f = this.#x }", false),
        ("class C { #x = 1; m(){ return #x in this } }", false),
        // Les PrivateEnvironments s'emboîtent : `o.#x` dans la
        // classe imbriquée résout le `#x` de l'englobante.
        ("class C { #x; m(){ class D { f = o.#x } } }", false),
    ] {
        assert_eq!(is_syn_err(src), expect_err, "{src}");
    }
}

#[test]
fn private_in_object_syntax() {
    for (src, expect_err) in [
        ("class C { #x = 1; m(){ ({#x: a} = o) } }", true),
        ("class C { #x = 1; m(){ let o = {#x: 1} } }", true),
        // En membre de classe seulement.
        ("class C { #x = 1; }", false),
        ("class C { static #x = 1; get #x(){return 0} }", true),
    ] {
        assert_eq!(is_syn_err(src), expect_err, "{src}");
    }
}

#[test]
fn field_terminator() {
    for (src, expect_err) in [
        ("class C { x = 1 y = 2 }", true),
        ("class C { x y }", true),
        ("class C { x = 1 static m(){} }", true),
        // ASI : saut de ligne, `;`, `}` — tous licites.
        ("class C { x = 1; y = 2 }", false),
        ("class C { x = 1\ny = 2 }", false),
        ("class C { x = 1 }", false),
        ("class C { x = 1; }", false),
        ("class C { x\n#y = 2 }", false),
        ("class C { static #x = 1\nget #x(){return 0} }", true),
    ] {
        assert_eq!(is_syn_err(src), expect_err, "{src}");
    }
}
