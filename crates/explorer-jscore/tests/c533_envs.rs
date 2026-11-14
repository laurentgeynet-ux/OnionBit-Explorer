// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Verrous JS-C533 — `Bindings` slot-indexé : les scopes câblés
//! vivent en `canon` partagé (`Rc<[Rc<str>]>` du resolver) +
//! `cells` denses ; les liaisons dynamiques (eval-injection,
//! scopes non modélisés) tombent dans `extra`. La sémantique
//! est identique au `BindingMap` : `Uninit` = absent vivant
//! (TDZ/remontée), première liaison d'un nom gagne, `with`/
//! closures/`arguments`/modules inchangés, fast-path `(d,slot)`
//! opportuniste préservé sur les scopes non câblés (ordre
//! d'insertion `extra` + vérif clé).
//! Motivation mesurée (vol N5.37) : `ident` = 34 % des nœuds
//! évalués, `env_lvl` 2,63 M — chaque niveau payait un scan de
//! paires `(Rc<str>, EnvCell)` non partagées par instance.

use explorer_jscore::Engine;

/// Sérialise les tests sensibles aux compteurs J17 partagés.
static IC_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn eval(src: &str) -> String {
    let mut e = Engine::new();
    match e.eval(src) {
        Ok(v) => e.describe(v),
        Err(err) => panic!("eval: {err}\n  src: {src}"),
    }
}

fn snap_counter(snap: &str, name: &str) -> u64 {
    snap.split(' ')
        .find_map(|p| {
            p.strip_prefix(name)
                .and_then(|v| v.strip_prefix('='))
                .and_then(|v| v.parse::<u64>().ok())
        })
        .unwrap_or(0)
}

/// Le canon sert les liaisons ordinaires : paramètres, `var`,
/// `let` locaux — `bnd_canon` croît pendant l'évaluation.
#[test]
fn c533_canon_serves_locals() {
    let _g = IC_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    explorer_jscore::j17_enable();
    let mut e = Engine::new();
    let r = e
        .eval(
            "function f(a,b){var c=a+b;let d=c*2;return c+d}\
             var s=0;for(var i=0;i<80;i++){s+=f(i,2)} s",
        )
        .expect("eval");
    assert_eq!(e.describe(r), "9960");
    let snap = explorer_jscore::j17_snapshot();
    assert!(
        snap_counter(&snap, "bnd_canon") > 0,
        "le canon doit servir des lookups : {snap}"
    );
}

/// Injection `eval` : `eval("var z=…")` ajoute une liaison hors
/// canon dans le scope courant — `extra` la porte, les lectures
/// suivantes la trouvent (remontée incluse).
#[test]
fn c533_eval_injection_lands_in_extra() {
    assert_eq!(
        eval(
            "function f(){eval('var z=41');return z+1}\
             var r=f();var z2=0;function g(){eval('var z2=9');return z2}\
             r+'/'+g()+'/'+z2"
        ),
        "42/9/0"
    );
}

/// TDZ préservé : `let` lu avant sa déclaration dans le même
/// scope — `Uninit` compte absent → remontée/`ReferenceError`
/// selon le cas (ici : le `var` externe gagne, comportement
/// historique du moteur conservé).
#[test]
fn c533_tdz_uninit_counts_absent() {
    assert_eq!(
        eval("var q=7;function g(){return typeof q;let q=5}g()"),
        "number"
    );
    // Vrai TDZ : aucune liaison externe ne masque.
    let mut e = Engine::new();
    let r = e.eval("function h(){return w;let w=5}h()");
    assert!(r.is_err(), "let lu avant déclaration doit jeter");
}

/// Closures : chaque appel a SES cellules — `make` incrémente un
/// compteur capturé, deux compteurs indépendants ne se
/// contaminent pas.
#[test]
fn c533_closures_own_their_cells() {
    assert_eq!(
        eval(
            "function make(){var c=0;return function(){return ++c}}\
             var a=make(),b=make();a();a();a();b();a()+'/'+b()"
        ),
        "4/2"
    );
}

/// Gros scope dynamique : >16 liaisons posées par `eval` — la
/// promotion `Map` d'`extra` reste correcte (lookups, écritures,
/// remontée).
#[test]
fn c533_large_dynamic_scope() {
    assert_eq!(
        eval(
            "function f(){var src='';for(var i=0;i<40;i++){src+='var v'+i+'='+i+';'}\
             eval(src);return v0+v10+v20+v39}f()"
        ),
        "69"
    );
}

/// `arguments` paresseux : liaison canonique matérialisée à la
/// première lecture, écriture possible ensuite.
#[test]
fn c533_lazy_arguments_cell() {
    assert_eq!(
        eval(
            "function f(){var a=arguments;return a.length+','+a[0]}\
             function g(){arguments='x';return arguments}\
             f(1,2,3)+'/'+g(9)"
        ),
        "3,1/x"
    );
}

/// `with` : la résolution passe par l'objet puis remonte — le
/// stockage slot-indexé des scopes englobants n'y change rien.
#[test]
fn c533_with_still_consults_object() {
    assert_eq!(eval("var o={x:5};var x=1;with(o){var y=x+1}y+','+x"), "6,1");
}

/// Réécritures répétées : la cellule canonique est écrite
/// in-place — le `slot_hit` continue de servir après mutation
/// (pas de recopie par instance).
#[test]
fn c533_inplace_writes_keep_serving() {
    let _g = IC_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    explorer_jscore::j17_enable();
    let mut e = Engine::new();
    let r = e
        .eval("function f(){var n=0;for(var i=0;i<100;i++){n+=i}return n}f()")
        .expect("eval");
    assert_eq!(e.describe(r), "4950");
    let snap = explorer_jscore::j17_snapshot();
    assert!(
        snap_counter(&snap, "slot_hit") > 0,
        "les slots résolus doivent servir : {snap}"
    );
}

/// `const` : écriture refusée sur cellule canonique — la garde
/// `mutable` survit au split (strict → `TypeError`).
#[test]
fn c533_const_binding_refuses() {
    let mut e = Engine::new();
    let r = e.eval("'use strict';const c=1;c=2");
    assert!(r.is_err(), "écriture const doit jeter");
}

/// Portées de bloc `for`-tête : `let` par itération vit dans son
/// env câblé — chaque tour capture sa cellule.
#[test]
fn c533_for_head_per_iteration_cells() {
    assert_eq!(
        eval(
            "var fs=[];for(let i=0;i<3;i++){fs.push(function(){return i})}\
             fs[0]()*100+fs[1]()*10+fs[2]()"
        ),
        "12"
    );
}
