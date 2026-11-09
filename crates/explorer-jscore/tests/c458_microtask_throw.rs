// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! JS-C458 (JT-14) — throw en microtask n'avorte pas le
//! drain (vol 629 — « throws avalés » : le `?` global
//! sautait les réactions promesse restantes et
//! `fire_unhandled_rejections`). Spec « report the
//! exception » : rapport `uncaught:` + la file continue.

use explorer_jscore::Engine;

/// `queueMicrotask(() => { throw })` suivi d'une réaction
/// promesse : la réaction s'exécute quand même (avant :
/// drain avorté → jamais).
#[test]
fn c458_throw_does_not_abort_pending_reactions() {
    let mut e = Engine::new();
    e.eval(
        "globalThis.__r = [];\
         queueMicrotask(() => { throw new Error('boom'); });\
         Promise.resolve(5).then(v => __r.push('then' + v));",
    )
    .expect("eval");
    e.run_microtasks().expect("drain");
    let v = e.eval("__r.join(',')").map(|x| e.describe(x)).unwrap();
    assert_eq!(v, "then5");
    // Rapport `uncaught:` drainable.
    let reports = e.drain_unhandled_rejections();
    assert!(
        reports.iter().any(|l| l.contains("uncaught:")),
        "reports: {reports:?}"
    );
}

/// Deux jobs qui jettent à la suite : les deux sont
/// rapportés, la file va au bout.
#[test]
fn c458_multiple_throws_all_reported() {
    let mut e = Engine::new();
    e.eval(
        "globalThis.__ok = false;\
         queueMicrotask(() => { throw 'a'; });\
         queueMicrotask(() => { throw 'b'; });\
         queueMicrotask(() => { __ok = true; });",
    )
    .expect("eval");
    e.run_microtasks().expect("drain");
    let v = e.eval("__ok").map(|x| e.describe(x)).unwrap();
    assert_eq!(v, "true");
    let reports = e.drain_unhandled_rejections();
    let n = reports
        .iter()
        .filter(|l| l.starts_with("uncaught:"))
        .count();
    assert_eq!(n, 2, "reports: {reports:?}");
}
