// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Sonde §B-110 — repro distillé du challenge `SG_SS`
//! (Search Guard google.com, vols 681-684) : la boucle de
//! dispatch émet
//! **1 `Array` + 1 `Str` par op** de sa boucle de dispatch —
//! mesuré terrain par la sonde top-kinds du message dur :
//! `Array:8 009 102/2 749 Mio Str:8 010 200/1 344 Mio`
//! (~500 B/op, cap dur 2 Gio atteint en ~45 s sous
//! `mem_limit` 512 Mio). Le sweep mid-eval étant impossible
//! (pile d'appel non racine — JS-C206), le cap tue l'eval
//! avant la fin du challenge → SERP bloquée.
//!
//! JS-C420 : la moitié `Str` du churn meurt désormais au
//! refcount (hors arène) — le census dur ne nomme plus que
//! les kinds encore cellulaires, `Array` en tête.
//!
//! Ici : la même signature en 20 lignes — `for` qui alloue
//! `[i]` + `String(i)` par tour, `mem_limit` réduit pour un
//! verdict rapide. `cargo test --test b110_probe`.

use explorer_jscore::Engine;

/// Signature §B-110 : le churn `[x]`+`str` par op doit mourir
/// en `RangeError` « deferred GC unreachable » avec les kinds
/// `Array`/`Str` dominant le message — pas en kill aveugle.
#[test]
fn array_str_churn_hits_deferred_gc_cap() {
    let mut e = Engine::new();
    e.set_mem_limit(64 * 1024 * 1024); // 64 Mio → cap dur 256 Mio
    let r = e.eval("for (var i = 0; i < 4e6; i++) { var a = [i]; var s = String(i); } 1");
    let msg = format!("{r:?}");
    assert!(
        msg.contains("deferred GC unreachable"),
        "le churn doit tomber sous le cap dur GC différée : {msg}"
    );
    // La forme morte nomme ses kinds (top-kinds §B-110) —
    // `Array` domine ; `Str` n'est plus un kind de cellule
    // depuis JS-C420 (les temporaries meurent au refcount).
    assert!(
        msg.contains("Array:") && !msg.contains("Str:"),
        "le message doit nommer les kinds dominants : {msg}"
    );
}
