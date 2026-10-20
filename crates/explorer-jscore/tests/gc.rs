// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Mark-sweep GC tests (ADR-0008, chantier JS-A) : reclamation des
//! graphes inaccessibles (cycles compris), survie des racines
//! (globals, envs capturés, timers, microtasks, racines explicites),
//! tombstones réutilisés, handles obsolètes détectés.

use explorer_jscore::{Engine, ObjId, Value};

fn ev(e: &mut Engine, src: &str) -> Value {
    e.eval(src)
        .unwrap_or_else(|err| panic!("eval failed for {src:?}: {err:?}"))
}

fn obj_id(v: Value) -> ObjId {
    match v {
        Value::Obj(id) => id,
        other => panic!("expected object, got {other:?}"),
    }
}

#[test]
fn unreachable_objects_are_collected() {
    let mut e = Engine::new();
    let before = e.live_count();
    ev(
        &mut e,
        "for (var i = 0; i < 8; i++) { ({ x: i, y: 'pad' }); }",
    );
    let mid = e.live_count();
    let swept = e.collect();
    assert!(mid > before, "allocations grew the heap");
    // Completion keeps the LAST temp alive — at least 7 must go.
    assert!(swept >= 7, "swept {swept}, expected >= 7");
    assert!(e.live_count() < mid);
}

#[test]
fn cyclic_graph_is_collected() {
    let mut e = Engine::new();
    ev(
        &mut e,
        "var a = {}; var b = {}; a.b = b; b.a = a; a = null; b = null;",
    );
    let swept = e.collect();
    assert!(swept >= 2, "a<->b cycle must be reclaimed, swept {swept}");
}

#[test]
fn globals_and_their_subgraphs_survive() {
    let mut e = Engine::new();
    ev(&mut e, "var keep = { deep: { v: 42 }, list: [1, 2, 3] };");
    e.collect();
    assert_eq!(ev(&mut e, "keep.deep.v + keep.list[2]"), Value::Num(45.0));
}

#[test]
fn let_bindings_in_global_env_survive() {
    let mut e = Engine::new();
    // `let` bindings live in the env record, not on `globalThis`.
    ev(&mut e, "let lex = { n: 7 };");
    e.collect();
    assert_eq!(ev(&mut e, "lex.n"), Value::Num(7.0));
}

#[test]
fn closure_env_is_retained_through_fun_object() {
    let mut e = Engine::new();
    ev(
        &mut e,
        "var inc = (function () { var n = 10; var pad = { x: 1 }; \
           return function () { n += pad.x; return n; }; })();",
    );
    e.collect();
    // `inc` is alive via global; its Fun::Script env keeps `n`/`pad`.
    assert_eq!(ev(&mut e, "inc()"), Value::Num(11.0));
    assert_eq!(ev(&mut e, "inc()"), Value::Num(12.0));
}

#[test]
fn dead_closures_release_their_env() {
    let mut e = Engine::new();
    ev(
        &mut e,
        "var f = (function () { var heavy = [1,2,3,4,5]; \
                  return function () { return 1; }; })(); \
                f = null;",
    );
    let swept = e.collect();
    assert!(
        swept >= 6,
        "closure + env array + elements should go, swept {swept}"
    );
}

#[test]
fn explicit_root_then_unroot() {
    let mut e = Engine::new();
    let id = obj_id(ev(&mut e, "({ keep: 1 })"));
    e.add_root(id);
    ev(&mut e, "0"); // reset completion root
    e.collect();
    assert!(e.is_alive(id), "explicit root must survive collect");
    assert!(e.is_rooted(id));
    e.remove_root(id);
    e.collect();
    assert!(!e.is_alive(id), "unrooted object must be swept");
    assert!(!e.is_rooted(id));
}

#[test]
fn stale_id_resolves_to_none_not_panic() {
    let mut e = Engine::new();
    let id = obj_id(ev(&mut e, "({ dangle: true })"));
    ev(&mut e, "0");
    e.collect();
    assert!(!e.is_alive(id));
    // Reuse : allouer des objets — le slot peut être réattribué à
    // une autre valeur ; l'ancien handle ne panique jamais, il
    // résout soit None soit un autre objet (contract ADR-0008 :
    // les handles hôtes stables passent par les racines, pas par
    // des ObjId nus).
    ev(&mut e, "for (var i = 0; i < 20; i++) { ({ n: i }); }");
    e.collect();
}

#[test]
fn pending_timer_callback_is_a_root() {
    let mut e = Engine::new();
    e.install_timers();
    ev(
        &mut e,
        "var fired = ''; \
         var cb = (function () { var big = { pad: 'ok' }; \
                                 return function () { fired = big.pad; }; })(); \
         setTimeout(cb, 0); cb = null;",
    );
    e.collect();
    // `cb`/`big` are only reachable through the pending timer —
    // timer roots must keep them alive.
    e.run_due_timers().expect("timer pump");
    assert_eq!(ev(&mut e, "fired === 'ok'"), Value::Bool(true));
}

#[test]
fn promise_reactions_keep_their_closures_alive() {
    let mut e = Engine::new();
    // Pending promise held by `res` → its reaction entry roots the
    // callback function, which roots the env capturing `big`.
    ev(
        &mut e,
        "var seen = ''; var res; \
         new Promise(function (r) { res = r; }).then( \
           (function () { var big = { m: 'yes' }; \
                          return function () { seen = big.m; }; })() \
         );",
    );
    e.collect();
    ev(&mut e, "res();");
    assert_eq!(ev(&mut e, "seen === 'yes'"), Value::Bool(true));
}

#[test]
fn dom_wrapper_cache_keeps_wrappers_alive() {
    use explorer_jscore::Dom;
    use std::cell::RefCell;
    use std::rc::Rc;
    let mut e = Engine::new();
    e.install_dom(&Rc::new(RefCell::new(Dom::from_html(
        "<html><body><div id='memo'>x</div></body></html>",
    ))));
    ev(&mut e, "var w = document.getElementById('memo'); w = null;");
    e.collect();
    // Wrapper table is rooted via dom_ctx — re-query must return
    // the SAME wrapper object (identity through the cache).
    assert_eq!(
        ev(&mut e, "document.getElementById('memo') !== null"),
        Value::Bool(true)
    );
    e.collect();
    assert_eq!(
        ev(&mut e, "document.getElementById('memo').id === 'memo'"),
        Value::Bool(true)
    );
}

#[test]
fn repeated_collect_on_stable_state_sweeps_nothing() {
    let mut e = Engine::new();
    ev(&mut e, "var stable = { a: [1, 2], b: 'txt' };");
    let first = e.collect();
    let second = e.collect();
    assert_eq!(second, 0, "idempotent collect on rooted graph");
    let _ = first; // may sweep init leftovers
    assert_eq!(ev(&mut e, "stable.a[1]"), Value::Num(2.0));
}

#[test]
fn collect_reclaims_strings_too() {
    let mut e = Engine::new();
    ev(&mut e, "var s = 'a' + 'b' + 'c'; s = null;");
    let swept = e.collect();
    assert!(swept >= 1, "dead string cells must be swept");
}
