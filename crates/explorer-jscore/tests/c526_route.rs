// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! JS-C526 — mémo de routes `(maillon,kid) → issue de chaîne`
//! validée par `SHAPE_EPOCH`. La mémo doit relire juste après
//! chaque mutation structurelle (prop ajoutée/retirée sur un
//! maillon, proto réécrit, sweep) — et relire la **valeur**
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
         M.x = 'mid';                       /* push_prop → epoch++ */\
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
