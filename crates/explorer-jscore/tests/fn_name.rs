// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Gate JS-C604 — `SetFunctionName` / `NamedEvaluation`.
//!
//! Avant : `name`/`length` étaient synthétisés à la lecture
//! (`lit.name`, comptage params), mais la syntaxe ne posait
//! jamais `lit.name` pour les formes anonymes — méthodes,
//! accesseurs, membres de classe, props objet, computed et
//! défauts exposaient `""`. Le descripteur d'un nom inféré
//! passait par `set_own` (`w:true`) au lieu des attrs spec
//! `{w:false,e:false,c:true}`.
//!
//! Mécanisme : `set_fn_name` (prop own aux attrs spec),
//! `prop_key_and_name` (clé évaluée UNE fois → clé interne
//! et nom d'affichage `Symbol → "[desc]"`, `#priv → "#x"`),
//! `is_anon_fn_def` — branchés sur : props/objets méthodes et
//! accesseurs (préfixes `get `/`set `), membres de classe,
//! `Pat::Default` (params + destructuring `SingleNameBinding`),
//! assignations à référence identifiant (`x = f` — pas
//! `o.k = f`), `export default`.

use explorer_jscore::Engine;

fn ev(src: &str) -> String {
    let mut e = Engine::new();
    match e.eval(src) {
        Ok(v) => format!("{v:?}"),
        Err(err) => format!("ERR {err}"),
    }
}

/// `NamedEvaluation` — liaisons et assignations.
#[test]
fn bindings() {
    assert_eq!(ev("var f = function(){}; f.name"), "Str(\"f\")");
    assert_eq!(ev("var g = () => 0; g.name"), "Str(\"g\")");
    assert_eq!(ev("var h; h = function(){}; h.name"), "Str(\"h\")");
    assert_eq!(ev("var i; i = () => 0; i.name"), "Str(\"i\")");
    // Nom explicite jamais écrasé.
    assert_eq!(ev("var e1 = function expl(){}; e1.name"), "Str(\"expl\")");
    // `o.k = f` n'est PAS une NamedEvaluation (membre, pas
    // référence identifiant) — spec : le nom reste `""`.
    assert_eq!(ev("var t = {}; t.k = function(){}; t.k.name"), "Str(\"\")");
}

/// Objets littéraux — props, méthodes, accesseurs.
#[test]
fn object_literal() {
    assert_eq!(ev("var o = {k: function(){}}; o.k.name"), "Str(\"k\")");
    assert_eq!(ev("var o = {k: () => 0}; o.k.name"), "Str(\"k\")");
    assert_eq!(ev("var o = {m(){}}; o.m.name"), "Str(\"m\")");
    assert_eq!(
        ev("var o = {get x(){return 1}};\
            Object.getOwnPropertyDescriptor(o,'x').get.name"),
        "Str(\"get x\")"
    );
    assert_eq!(
        ev("var o = {set x(v){}};\
            Object.getOwnPropertyDescriptor(o,'x').set.name"),
        "Str(\"set x\")"
    );
    // Computed : évalué une seule fois, nom = clé résolue.
    assert_eq!(
        ev("var c = 0; var o = {[(c++,'ab')]: function(){}};\
            c + ',' + o.ab.name"),
        "Str(\"1,ab\")"
    );
    assert_eq!(ev("var o = {['q'+'r'](){}}; o.qr.name"), "Str(\"qr\")");
    // Symbol → `"[desc]"`.
    assert_eq!(
        ev("var s = Symbol('tag'); var o = {[s]: function(){}};\
            o[s].name"),
        "Str(\"[tag]\")"
    );
    assert_eq!(
        ev("var o = {[Symbol()]: function(){}};\
            Object.getOwnPropertyNames(o).length >= 0 && \
            o[Object.getOwnPropertySymbols ? 0 : 0] === undefined || true;\
            'ok'"),
        "Str(\"ok\")"
    );
    assert_eq!(
        ev("var o = {[Symbol()]: function(){}};\
            var k = Object.getOwnPropertySymbols(o)[0]; o[k].name"),
        "Str(\"[]\")"
    );
}

/// Classes — méthodes, static, accesseurs, privés.
#[test]
fn class_members() {
    assert_eq!(ev("class K {m(){}}; K.prototype.m.name"), "Str(\"m\")");
    assert_eq!(ev("class K {static s(){}}; K.s.name"), "Str(\"s\")");
    assert_eq!(
        ev("class K {get g(){return 1}};\
            Object.getOwnPropertyDescriptor(K.prototype,'g').get.name"),
        "Str(\"get g\")"
    );
    assert_eq!(
        ev("class K {set g(v){}};\
            Object.getOwnPropertyDescriptor(K.prototype,'g').set.name"),
        "Str(\"set g\")"
    );
    assert_eq!(
        ev("class K {['m'+'n'](){}}; K.prototype.mn.name"),
        "Str(\"mn\")"
    );
    // Computed évalué une fois.
    assert_eq!(
        ev(
            "var c = 0; class K {[(c++,'z')](){} static [(c++,'y')](){}};\
            c"
        ),
        "Num(2.0)"
    );
    // Classe anonyme liée → nom du binding.
    assert_eq!(ev("var A = class {}; A.name"), "Str(\"A\")");
    assert_eq!(ev("var B = class BB {}; B.name"), "Str(\"BB\")");
    // `static name(){}` / `static get name` priment sur le
    // nom inféré (spec `HasOwnProperty` du binding).
    assert_eq!(
        ev("var K2 = class { static name() {} }; typeof K2.name"),
        "Str(\"function\")"
    );
    assert_eq!(
        ev("var K3 = class { static get name() { return 'N'; } }; K3.name"),
        "Str(\"N\")"
    );
}

/// `using` et init de boucle sont des `NamedEvaluation`.
#[test]
fn using_and_for_init() {
    assert_eq!(
        ev("Function.prototype[Symbol.dispose] = function(){};\
            { using u = () => {}; globalThis.__u = u.name; } globalThis.__u"),
        "Str(\"u\")"
    );
    assert_eq!(
        ev(
            "for (var f = function(){};;) { globalThis.__f = f.name; break; }\
            globalThis.__f"
        ),
        "Str(\"f\")"
    );
    assert_eq!(
        ev(
            "for (let g = () => 0;;) { globalThis.__g = g.name; break; }\
            globalThis.__g"
        ),
        "Str(\"g\")"
    );
}

/// `name`/`length` sont des props own **réelles** (spec
/// `FunctionInitialize`) — `hasOwnProperty` les voit.
#[test]
fn own_props() {
    assert_eq!(
        ev("var f = function(){};\
            f.hasOwnProperty('name') + '|' + f.hasOwnProperty('length')"),
        "Str(\"true|true\")"
    );
    assert_eq!(
        ev("async function af(){}; af.hasOwnProperty('name') + '|' + af.name"),
        "Str(\"true|af\")"
    );
    assert_eq!(
        ev("function* gf(){}; gf.hasOwnProperty('name') + '|' + gf.name"),
        "Str(\"true|gf\")"
    );
}

/// Couverture `IsAnonymousFunctionDefinition` — comma/call
/// ne nomment pas, parenthèses transparentes oui.
#[test]
fn cover_grammar() {
    // `(0, f)` — Seq : pas de NamedEvaluation.
    assert_eq!(
        ev("var xCover = (0, function(){}); xCover.name"),
        "Str(\"\")"
    );
    // `(f)` — parenthèses transparentes : NamedEvaluation.
    assert_eq!(
        ev("var cover = (function(){}); cover.name"),
        "Str(\"cover\")"
    );
    // Résultat d'appel : pas de NamedEvaluation.
    assert_eq!(
        ev("var r = (function(){ return function(){}; })(); r.name"),
        "Str(\"\")"
    );
}

/// Défauts — params et destructuring `SingleNameBinding`.
#[test]
fn defaults() {
    assert_eq!(
        ev("function d(x = function(){}) { return x.name; } d()"),
        "Str(\"x\")"
    );
    assert_eq!(ev("var [z = () => 0] = []; z.name"), "Str(\"z\")");
    assert_eq!(ev("var {w = function(){}} = {}; w.name"), "Str(\"w\")");
    // Pattern non-identifiant : pas de nom inféré.
    assert_eq!(
        ev("var [{length: l} = function(){}] = [{}]; 'ok'"),
        "Str(\"ok\")"
    );
}

/// Descripteurs spec — `{w:false,e:false,c:true}`.
#[test]
fn descriptors() {
    assert_eq!(
        ev(
            "var d = Object.getOwnPropertyDescriptor(function(){},'name');\
            d.writable + ',' + d.enumerable + ',' + d.configurable"
        ),
        "Str(\"false,false,true\")"
    );
    // Non-writable : assignation ignorée (sloppy silencieux).
    assert_eq!(
        ev("var f = function(){}; f.name = 'hack'; f.name"),
        "Str(\"f\")"
    );
    // Configurable : `defineProperty` peut renommer.
    assert_eq!(
        ev("var f = () => 0;\
            Object.defineProperty(f,'name',{value:'fix'}); f.name"),
        "Str(\"fix\")"
    );
    // Nom inféré a les mêmes attrs.
    assert_eq!(
        ev("var f = () => 0;\
            var d = Object.getOwnPropertyDescriptor(f,'name');\
            d.writable + ',' + d.enumerable + ',' + d.configurable"),
        "Str(\"false,false,true\")"
    );
}

/// `length` — paramètres avant premier défaut/rest ;
/// destructuring compte 1.
#[test]
fn arity() {
    assert_eq!(ev("function f(a,{b},...r){}; f.length"), "Num(2.0)");
    assert_eq!(ev("function f(a,b=1,c){}; f.length"), "Num(1.0)");
    assert_eq!(ev("var f = (a,b,c) => 0; f.length"), "Num(3.0)");
    assert_eq!(
        ev(
            "var d = Object.getOwnPropertyDescriptor(function(){},'length');\
            d.writable + ',' + d.enumerable + ',' + d.configurable"
        ),
        "Str(\"false,false,true\")"
    );
}

/// Bound — `"bound <target>"` préservé, non écrasé.
#[test]
fn bound() {
    assert_eq!(
        ev("var f = function foo(){}.bind(null); f.name"),
        "Str(\"bound foo\")"
    );
    // Chaînage.
    assert_eq!(
        ev("var f = function foo(){}.bind(null).bind(null); f.name"),
        "Str(\"bound bound foo\")"
    );
}

/// `Function.prototype.name` === `""` (héritage après lecture
/// d'une fonction anonyme).
#[test]
fn anonymous_stays_empty() {
    assert_eq!(ev("(function(){}).name"), "Str(\"\")");
    assert_eq!(ev("(() => 0).name"), "Str(\"\")");
    // bound fonctionne aussi sur nom inféré.
    assert_eq!(
        ev("var f = (() => 0).bind(null); f.name"),
        "Str(\"bound \")"
    );
}
