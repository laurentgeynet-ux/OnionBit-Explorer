// This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
// SPDX-License-Identifier: MIT OR Apache-2.0

//! JS-C326 — verrou §B-83 branche B : le checkpoint microtasks
//! tourne même quand la rafale timers rend `Err`.
//! Avant le fix, `Cmd::RunPending` enchaînait
//! `run_due_timers_bounded().and_then(run_microtasks)` : un callback
//! qui explose son budget ops/deadline rendait `Err` → la file de
//! promesses était sautée à chaque pompe. Un timer qui se ré-arme
//! *avant* le coup fatal rend la famine permanente : la file croît
//! sans jamais se vider (machine à états async des players gelée —
//! le « drain qui ne tick pas » du vol 625).

use explorer_js::{JsEngine, JscoreEngine};

#[test]
fn timer_err_starves_microtasks() {
    let mut eng = JscoreEngine::new();
    eng.install_dom("<html><body></body></html>", "https://t/", 1);
    eng.set_budget(50_000);
    eng.eval(
        r"
        var done = 0;
        function tick(){
            Promise.resolve().then(function(){ done++; });
            setTimeout(tick, 0);   // ré-arme AVANT le coup fatal
            while (true) {}        // explose le budget ops → Err batch
        }
        setTimeout(tick, 0);
        ",
    )
    .unwrap();

    for _ in 0..4 {
        let r = eng.run_pending_bounded(10);
        assert!(r.is_err(), "pompe attendue en erreur (budget tué)");
    }
    let (timers, microtasks, _rs) = eng.boot_diag();
    assert_eq!(timers, 1, "la chaîne setTimeout reste armée");
    assert_eq!(
        microtasks, 0,
        "le checkpoint drainait malgré l'Err timer : la file doit se vider"
    );
}
