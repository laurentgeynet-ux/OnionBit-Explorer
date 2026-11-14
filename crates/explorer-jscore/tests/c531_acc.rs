// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Verrous JS-C531 — `IcKind::Acc` : les sites à accesseur
//! hérité ou own (setter/getter) peuvent apprendre — la chaîne
//! légère de shapes + `props[pos]` est `Acc` est vérifiée à
//! chaque hit, puis le callee **fraîchement relu** est invoqué.
//! La sémantique `[[Get]]`/`[[Set]]` tient intégralement : swap
//! in-place du setter servi frais, swap `Acc→Data` → miss →
//! relearn, setter-less refuse, getter-less → `undefined`,
//! `in` sur accesseur → `true`, `get_prop` data-only → absent.
//! Motivation mesurée (vol N5.35) : ~290 k write-walk par les
//! accesseurs hérités (Polymer `behaviors`) — inapprenables.

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
        .find_map(|p| p.strip_prefix(name).and_then(|v| v.parse::<u64>().ok()))
        .unwrap_or(0)
}

/// Setter hérité servi : écritures répétées à travers un setter
/// de proto → `ic_acc` croît et chaque écriture intercepte le
/// callee (pas de shadow own).
#[test]
fn c531_acc_set_engages() {
    let _g = IC_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    explorer_jscore::j17_enable();
    let before = snap_counter(&explorer_jscore::j17_snapshot(), "ic_acc=");
    assert_eq!(
        eval(
            "var log=[];var p={set x(v){log.push(v)}};\
             var o=Object.create(p);for(var i=0;i<300;i++){o.x=i}\
             log[299]+'/'+o.hasOwnProperty('x')+'/'+p.x"
        ),
        "299/false/undefined"
    );
    let after = snap_counter(&explorer_jscore::j17_snapshot(), "ic_acc=");
    assert!(after > before + 100, "ic_acc attendu : {before} → {after}");
}

/// Getter hérité servi : lectures répétées → `ic_acc` croît,
/// `this` = receveur à chaque appel.
#[test]
fn c531_acc_get_served() {
    let _g = IC_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    explorer_jscore::j17_enable();
    let before = snap_counter(&explorer_jscore::j17_snapshot(), "ic_acc=");
    assert_eq!(
        eval(
            "var p={get x(){return this.tag}};var o=Object.create(p);\
             o.tag=41;var s=0;for(var i=0;i<300;i++){s=o.x}s"
        ),
        "41"
    );
    let after = snap_counter(&explorer_jscore::j17_snapshot(), "ic_acc=");
    assert!(after > before + 100, "ic_acc attendu : {before} → {after}");
}

/// Setter hérité **profond** (2 maillons) : chaîne légère
/// multi-lien vérifiée — le setter intercepte, pas de shadow.
#[test]
fn c531_acc_deep_chain() {
    assert_eq!(
        eval(
            "var log=0;var g={set x(v){log=v}};var p=Object.create(g);\
             var o=Object.create(p);for(var i=0;i<30;i++){o.x=i}\
             log+'/'+o.hasOwnProperty('x')"
        ),
        "29/false"
    );
}

/// Swap in-place du setter : `defineProperty` sur la même clé
/// remplace `props[pos]` (shape stable → le hit vérifie) — le
/// callee **frais** doit être invoqué, jamais un stale.
#[test]
fn c531_setter_swapped_fresh() {
    assert_eq!(
        eval(
            "var log=[];var p={set x(v){log.push('a'+v)}};\
             var o=Object.create(p);o.x=1;\
             Object.defineProperty(p,'x',{set:function(v){log.push('b'+v)}});\
             for(var i=0;i<30;i++){o.x=i}log.slice(-3).join(',')"
        ),
        "b27,b28,b29"
    );
}

/// Swap `Acc → Data` in-place : `props[pos]` change de kind →
/// la vérif `Acc` manque → la marche trouve la Data writable →
/// shadow own créée chez le receveur, proto inchangé.
#[test]
fn c531_acc_to_data_shadows() {
    assert_eq!(
        eval(
            "var log=0;var p={set x(v){log=v}};var o=Object.create(p);\
             o.x=1;\
             Object.defineProperty(p,'x',{value:42,writable:true,\
             configurable:true,enumerable:true});\
             for(var i=0;i<30;i++){o.x=i+100}\
             o.x+'/'+p.x+'/'+log"
        ),
        "129/42/1"
    );
}

/// Accesseur sans setter : l'écriture sloppy est refusée en
/// silence — pas de shadow, la lecture du getter tient.
#[test]
fn c531_setterless_refuses() {
    assert_eq!(
        eval(
            "var p={get x(){return 7}};var o=Object.create(p);\
             for(var i=0;i<30;i++){o.x=i}\
             o.x+'/'+o.hasOwnProperty('x')"
        ),
        "7/false"
    );
}

/// Accesseur sans getter : la lecture rend `undefined`
/// (même sort qu'au bout de la marche).
#[test]
fn c531_getterless_undefined() {
    assert_eq!(
        eval(
            "var log=0;var p={set x(v){log=v}};var o=Object.create(p);\
             var s='u';for(var i=0;i<30;i++){s=o.x}s"
        ),
        "undefined"
    );
}

/// `in` sur accesseur hérité : le hit `Acc` prouve l'existence
/// → `true`, sans invoquer de callee.
#[test]
fn c531_in_operator_acc() {
    assert_eq!(
        eval(
            "var p={get x(){return 1},set x(v){}};var o=Object.create(p);\
             var s=false;for(var i=0;i<30;i++){s=('x' in o)}s"
        ),
        "true"
    );
}

/// Accesseur **own** sur le receveur (chaîne à un maillon) :
/// get/set servis, `this` = receveur.
#[test]
fn c531_own_accessor() {
    assert_eq!(
        eval(
            "var o={_v:0,get x(){return this._v},set x(v){this._v=v}};\
             for(var i=0;i<30;i++){o.x=i}o.x"
        ),
        "29"
    );
}

/// Reprototype en amont : `Object.setPrototypeOf` change la
/// chaîne → la vérif des shapes manque au premier accès →
/// marche complète → le nouveau setter est appris et sert.
#[test]
fn c531_proto_swap_relearns() {
    assert_eq!(
        eval(
            "var log=[];var p1={set x(v){log.push('a'+v)}};\
             var p2={set x(v){log.push('b'+v)}};\
             var o=Object.create(p1);o.x=1;\
             Object.setPrototypeOf(o,p2);for(var i=0;i<30;i++){o.x=i}\
             log.slice(-2).join(',')"
        ),
        "b28,b29"
    );
}
