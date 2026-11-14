// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! JS-C526 — mémo de routes `(maillon,kid) → issue de chaîne`
//! revalidée **par maillon stocké** (`shape` + `proto` — C526b,
//! après que l'epoch globale s'est mesurée mortelle au vol
//! N5.29 : chaque `push_prop` du tas tuait tout le cache).
//! La mémo doit relire juste après chaque mutation structurelle
//! d'un maillon de la route (prop ajoutée/retirée, proto réécrit),
//! ignorer les mutations hors route — et relire la **valeur**
//! fraîche à position constante (écriture non-structurelle).

use explorer_jscore::{j17_enable, j17_snapshot, Engine};

fn ev(src: &str) -> String {
    let mut e = Engine::new();
    j17_enable();
    match e.eval(src) {
        Ok(v) => format!("{v:?}"),
        Err(x) => format!("ERR {x:?}"),
    }
}

/// Shadowing par ajout sur un maillon intermédiaire —
/// `push_prop` sur le proto du milieu doit invalider la route.
#[test]
fn shadowing_on_middle_link_invalidates() {
    let r = ev(
        "var P = {x: 'proto'}; var M = Object.create(P); var o = Object.create(M);\
         var a = o.x; var b = o.x;          /* marche + mémo */\
         M.x = 'mid';                       /* push_prop → shape M change */\
         var c = o.x; var d = o.x;          /* re-marche + mémo */\
         [a, b, c, d].join('|')",
    );
    assert_eq!(r, "Str(\"proto|proto|mid|mid\")");
}

/// Suppression sur le holder — `remove_own` invalide.
#[test]
fn delete_on_holder_invalidates() {
    let r = ev("var P = {x: 'proto'}; var o = Object.create(P);\
         var a = o.x; var b = o.x;\
         delete P.x;\
         var c = o.x; var d = o.x;\
         [a, b, c===undefined, d===undefined].join('|')");
    assert_eq!(r, "Str(\"proto|proto|true|true\")");
}

/// Écriture de valeur à position constante — pas de bump,
/// la mémo relit `props[pos]` frais (jamais de stale value).
#[test]
fn value_write_at_same_pos_stays_fresh() {
    let r = ev("var P = {x: 1}; var o = Object.create(P);\
         var a = o.x; var b = o.x;\
         P.x = 42;                          /* props.get_mut — pas de bump */\
         var c = o.x; var d = o.x;\
         [a, b, c, d].join('|')");
    assert_eq!(r, "Str(\"1|1|42|42\")");
}

/// Réécriture du lien proto — `set_proto` invalide la chaîne.
#[test]
fn setprototypeof_invalidates() {
    let r = ev(
        "var P = {x: 'proto'}; var Q = {x: 'q'}; var o = Object.create(P);\
         var a = o.x; var b = o.x;\
         Object.setPrototypeOf(o, Q);\
         var c = o.x; var d = o.x;\
         [a, b, c, d].join('|')",
    );
    assert_eq!(r, "Str(\"proto|proto|q|q\")");
}

/// JS-C526b — une mutation structurelle **hors route** ne doit
/// PAS invalider : l'epoch globale tuait les 16 384 entrées à
/// chaque `push_prop` du tas (N5.29, `rt_st` > `rt_hit`).
#[test]
fn unrelated_mutation_keeps_route() {
    let mut e = Engine::new();
    j17_enable();
    e.eval(
        "var P = {m: function(){return 1}}; var o = Object.create(P);\
         var u = {}; var s = 0;\
         for (var i = 0; i < 50; i++) { s += o.m(); u['k'+i] = i; } s",
    )
    .expect("eval");
    let snap = j17_snapshot();
    let get = |n: &str| {
        snap.split_whitespace()
            .find_map(|p| p.strip_prefix(&format!("{n}=")))
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(0)
    };
    // ~50 hits attendus (1er lookup = miss + pose) : les
    // mutations de `u` ne sont pas dans la route o→P.
    assert!(get("rt_hit") >= 40, "rt_hit doit dominer — {snap}");
    assert!(
        get("rt_st") < get("rt_hit"),
        "hors route ne doit pas staler — {snap}"
    );
}

/// Le compteur `rt_hit` prouve que le 2e accès saute la marche
/// (O(1) sur chaîne répétée).
#[test]
fn memo_counters_tick() {
    let mut e = Engine::new();
    j17_enable();
    e.eval(
        "var P = {m: function(){return 1}}; var o = Object.create(P);\
         var s = 0; for (var i = 0; i < 100; i++) { s += o.m(); } s",
    )
    .expect("eval");
    let snap = j17_snapshot();
    let get = |n: &str| {
        snap.split_whitespace()
            .find_map(|p| p.strip_prefix(&format!("{n}=")))
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(0)
    };
    assert!(get("rt_hit") > 0, "rt_hit doit compter — {snap}");
}
