// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! JS-C460 (JT-F) — harnais de mesure : churn sous éval,
//! résidus §B-114 (overshoot `a.ads.rmbl.ws`) et §B-110
//! (sweep sous éval `SG_SS` ~500 B/op).
//!
//! Diagnostic : sous éval un sweep ne part qu'au `safepoint_gc`
//! entre statements — la borne d'âge `young_floor` vaut le seq
//! du **plus ancien statement actif** : une boucle `for`
//! épingle le churn de toutes ses itérations (« jeunes »),
//! chaque passe balaie sans rien réclamer, le backoff borne la
//! cadence et le cap dur `dead_hard` fait le kill. Le fix
//! (borne d'âge par frame, masquée pendant la sous-exécution)
//! doit rendre les morts du tour précédent réclamables au
//! safepoint du tour courant.
//!
//! `cargo test --test c460_gc_churn` — mesures via
//! `heap_stats().gc_runs` / `estimated_bytes` /
//! `last_eval_gc_us` (JS-C423).

use explorer_jscore::{Engine, Value};

const MIO: usize = 1024 * 1024;

/// Churn `SG_SS` distillé : ~500 B de cellules mortes par
/// itération d'un `for` top-level (signature `b110_probe`).
/// Sans borne fine : `dead_hard` tue à ~256 Mio accumulés ;
/// avec : le safepoint réclame les morts du tour précédent —
/// l'éval complète sous une borne bien inférieure.
#[test]
fn c460_pow_loop_churn_reclaimed_between_iterations() {
    let mut e = Engine::new();
    e.set_mem_limit(32 * MIO); // cap dur 256 Mio (plancher)
    e.set_op_limit(200_000_000);
    let runs0 = e.heap_stats().gc_runs;
    let r = e.eval(
        "var acc = 0;\
         for (var i = 0; i < 6e5; i++) {\
           var a = [i, i + 1, i + 2]; var b = [i + 3, i + 4, i + 5];\
           var s = String(i); acc += a.length + b.length;\
         }\
         acc",
    );
    let runs = e.heap_stats().gc_runs - runs0;
    let (gc_us, sp_us) = (e.last_eval_gc_us(), e.last_eval_sp_us());
    match r {
        Ok(Value::Num(n)) => assert_eq!(n, 6.0 * 6e5),
        other => panic!(
            "le churn doit être réclamé mid-span (runs={runs}, \
             gc_us={gc_us}, sp_us={sp_us}, bytes={}): {other:?}",
            e.heap_stats().estimated_bytes
        ),
    }
    assert!(
        e.heap_stats().estimated_bytes < 64 * MIO,
        "le tas doit rester borné par la dette, pas grimper au cap : {}",
        e.heap_stats().estimated_bytes
    );
    assert!(
        sp_us <= gc_us,
        "sp ⊆ gc : sp={sp_us} gc={gc_us} runs={runs}"
    );
}

/// Même churn sous **un corps de fonction appelé en position
/// « call-only »** (`var x = f()`) : le statement appelant ne
/// tient rien pendant le dispatch — sa borne peut être masquée,
/// les morts de la callee réclamés. Forme exacte `x = SG_SS()`.
#[test]
fn c460_call_only_stmt_frees_callee_churn() {
    let mut e = Engine::new();
    e.set_mem_limit(32 * MIO);
    e.set_op_limit(200_000_000);
    let r = e.eval(
        "function churn() {\
           var acc = 0;\
           for (var i = 0; i < 6e5; i++) {\
             var a = [i, i + 1, i + 2]; var b = [i + 3, i + 4, i + 5];\
             var s = String(i); acc += a.length + b.length;\
           }\
           return acc;\
         }\
         var x = churn();\
         x",
    );
    match r {
        Ok(Value::Num(n)) => assert_eq!(n, 6.0 * 6e5),
        other => panic!(
            "le churn de la callee doit être réclamé (bytes={}): {other:?}",
            e.heap_stats().estimated_bytes
        ),
    }
    assert!(
        e.heap_stats().estimated_bytes < 64 * MIO,
        "borne masquée : tas borné attendu, {}",
        e.heap_stats().estimated_bytes
    );
}

/// Frontière connue (chantier suivant — table de racines
/// d'expressions) : un appel imbriqué dans une **expression
/// composée** (`a + f()`) garde la borne du statement — les
/// temporaires non racinés du bras gauche l'exigent. Verrouille
/// le diagnostic honnête : kill différé, kinds nommés — pas de
/// sweep sauvage de temporaires vivants.
#[test]
fn c460_compound_expr_call_stays_bounded() {
    let mut e = Engine::new();
    e.set_mem_limit(32 * MIO);
    e.set_op_limit(200_000_000);
    // `keep` tient un temporaire vivant pendant l'appel qui
    // produit le churn : la borne du statement le protège.
    let r = e.eval(
        "var keep = {v: 1};\
         function churn() {\
           for (var i = 0; i < 8e5; i++) { var a = [i, i + 1]; var s = String(i); }\
           return 0;\
         }\
         var x = keep.v + churn();\
         x",
    );
    let msg = format!("{r:?}");
    assert!(
        r.is_err() && msg.contains("deferred GC unreachable"),
        "churn épinglé sous expression composée → cap dur nommé, \
         jamais de corruption : {msg}"
    );
    assert!(
        msg.contains("Array:"),
        "les kinds nomment leur matière : {msg}"
    );
}

/// Correctness — raison d'être de `young_floor` : un temporaire
/// déjà évalué d'une expression composée survit aux safepoints
/// de la callee (c'est exactement ce que la borne protège).
#[test]
fn c460_mid_expr_temps_survive_safepoints() {
    let mut e = Engine::new();
    e.set_mem_limit(32 * MIO);
    e.set_op_limit(200_000_000);
    let r = e.eval(
        "function g() {\
           for (var i = 0; i < 5e5; i++) { var t = [i, i + 1]; }\
           return {v: 2};\
         }\
         function deep() { return {v: 40}; }\
         var r = deep().v + g().v;\
         r",
    );
    match r {
        Ok(Value::Num(n)) => assert_eq!(n, 42.0),
        other => panic!("temporaire mid-expression balayé : {other:?}"),
    }
}

/// `for (let i = obj; side(); i.n++)` — la tête lexicale est un
/// objet vivant pendant les évals test/update : un safepoint
/// dans une callee n'y passant pas par les args ne doit pas
/// balayer `i` (env non semée hors corps — racinée pour la
/// boucle sous borne fine).
#[test]
fn c460_for_let_head_env_rooted() {
    let mut e = Engine::new();
    e.set_mem_limit(64 * MIO);
    e.set_op_limit(200_000_000);
    let r = e.eval(
        "function side() { for (var j = 0; j < 6e4; j++) { var t = [j, j + 1]; } return true; }\
         var seen = 0;\
         for (let i = {n: 0}; side() && i.n < 3; i.n++) { seen++; }\
         seen",
    );
    match r {
        Ok(Value::Num(n)) => assert_eq!(n, 3.0),
        other => panic!("tête `let` balayée pendant test/update : {other:?}"),
    }
}

/// `for..of` paresseux (JS-C456) : l'itérateur vit à travers
/// tous les corps — sous borne fine il n'est plus « jeune »
/// après le premier tour, il doit être raciné pour la boucle.
#[test]
fn c460_forof_lazy_iterator_rooted() {
    let mut e = Engine::new();
    e.set_mem_limit(64 * MIO);
    e.set_op_limit(200_000_000);
    let r = e.eval(
        "var it = {\
           [Symbol.iterator]: function () {\
             return { n: 0, next: function () {\
               return this.n < 4 ? {done: false, value: this.n++} : {done: true};\
             } };\
           }\
         };\
         var acc = 0;\
         for (const x of it) {\
           for (var j = 0; j < 2e5; j++) { var t = [j]; }\
           acc += x;\
         }\
         acc",
    );
    match r {
        Ok(Value::Num(n)) => assert_eq!(n, 6.0),
        other => panic!("itérateur lazy balayé en cours de boucle : {other:?}"),
    }
}

/// `try { return v } finally { … }` — la valeur de `return`
/// vit dans `r` pendant le corps `finally` : racine temporaire
/// requise sous borne fine (parité du jeton `throw`, JS-C421).
#[test]
fn c460_try_finally_return_value_rooted() {
    let mut e = Engine::new();
    e.set_mem_limit(64 * MIO);
    e.set_op_limit(200_000_000);
    let r = e.eval(
        "function f() {\
           try { return {v: 41}; }\
           finally { for (var j = 0; j < 3e5; j++) { var t = [j]; } }\
         }\
         f().v + 1",
    );
    match r {
        Ok(Value::Num(n)) => assert_eq!(n, 42.0),
        other => panic!("valeur de return balayée pendant finally : {other:?}"),
    }
}
