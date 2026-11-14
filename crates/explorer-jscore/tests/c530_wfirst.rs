// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Verrous JS-C530 — own-first de l'écriture (`set_prop_walk` +
//! tête de `write_prop_at`) : une own-prop Data writable est
//! écrite sans la mécanique de traversée ni les gardes kinds —
//! mais la sémantique `[[Set]]` tient intégralement : setter
//! hérité intercepte (absent own → boucle), donnée héritée
//! non-writable refuse, own non-writable refuse, proxy head
//! passe par ses traps, `seal` (writable, non-config) écrit
//! encore, `freeze` refuse.
//! Motivation mesurée (vol N5.34) : write-walk ≈ 33 % de
//! `prop_ns` dont les own-writes payaient la mécanique pleine.

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

/// Le raccourci engage : écritures own répétées → `w_own` croît,
/// valeur relue fraîche.
#[test]
fn c530_wown_engages() {
    let _g = IC_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    explorer_jscore::j17_enable();
    let before = snap_counter(&explorer_jscore::j17_snapshot(), "w_own=");
    assert_eq!(eval("var o={x:0};for(var i=0;i<300;i++){o.x=i}o.x"), "299");
    let after = snap_counter(&explorer_jscore::j17_snapshot(), "w_own=");
    assert!(after > before + 100, "w_own attendu : {before} → {after}");
}

/// Setter hérité : absent own → la boucle marche et le setter du
/// proto intercepte — jamais court-circuité par l'own-first.
#[test]
fn c530_inherited_setter_intercepts() {
    assert_eq!(
        eval(
            "var log=0;var p={set x(v){log=v}};var o=Object.create(p);\
             for(var i=0;i<30;i++){o.x=i}log+'/'+o.x"
        ),
        "29/undefined"
    );
}

/// Donnée héritée non-writable : refus sloppy silencieux — aucune
/// own-prop créée, l'héritée reste lue.
#[test]
fn c530_inherited_nonwritable_refuses() {
    assert_eq!(
        eval(
            "var p={};Object.defineProperty(p,'x',{value:1,writable:false});\
             var o=Object.create(p);o.x=9;\
             (o.hasOwnProperty('x')?o.x:'absent')+'/'+o.x"
        ),
        "absent/1"
    );
}

/// Own non-writable : refusé — la valeur ne bouge pas.
#[test]
fn c530_own_nonwritable_refuses() {
    assert_eq!(
        eval(
            "var o={};Object.defineProperty(o,'x',{value:7,writable:false});\
             o.x=9;o.x"
        ),
        "7"
    );
}

/// `seal` (non-configurable mais writable) : l'écriture own passe
/// — `attrs.writable` seul gouverne le raccourci.
#[test]
fn c530_sealed_still_writes() {
    assert_eq!(eval("var o={x:1};Object.seal(o);o.x=5;o.x"), "5");
}

/// Tête proxy : la trap `set` est invoquée — kind inéligible →
/// boucle complète, jamais shuntée.
#[test]
fn c530_proxy_head_traps() {
    assert_eq!(
        eval(
            "var hits=0;var p=new Proxy({},{set:function(t,k,v){hits++;t[k]=v;return true}});\
             for(var i=0;i<30;i++){p.x=i}hits+'/'+p.x"
        ),
        "30/29"
    );
}

/// Prop nommée own sur Array : écrite normalement ; `length` et
/// indices restent sur le chemin éléments (sémantique intacte).
#[test]
fn c530_array_named_prop() {
    assert_eq!(
        eval("var a=[1,2,3];a.tag='t';a[0]=9;a.length=2;a.tag+'/'+a[0]+'/'+a.length"),
        "t/9/2"
    );
}

/// Écriture via `Reflect.set` (receiver ≠ cible possible) et
/// après `setPrototypeOf` : la valeur atterrit sur le receveur.
#[test]
fn c530_reflect_set_and_proto_change() {
    assert_eq!(
        eval(
            "var o={x:1};Reflect.set(o,'x',4);var n=Object.create(null);o.x+'/'+\
             (function(){var a={x:2};Object.setPrototypeOf(a,{});a.x=6;return a.x})()"
        ),
        "4/6"
    );
}
