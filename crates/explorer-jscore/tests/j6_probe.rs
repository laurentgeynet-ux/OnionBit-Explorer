// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Sonde J6 temporaire — évalue le chunk `_app` BBC Good Food
//! verbatim (vol 34) pour localiser le `` `...` outside a call or
//! array literal `` résiduel.

use explorer_jscore::Engine;

#[test]
fn j6_bbc_app_eval() {
    let src = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/data/vol34_bbc_app.js"
    ))
    .unwrap_or_else(|e| panic!("fixture: {e}"));
    let mut e = Engine::new();
    e.install_timers();
    match e.eval(&src) {
        Ok(v) => println!("EVAL OK → {}", e.describe(v)),
        Err(x) => println!("EVAL ERR → {x:?}"),
    }
    for i in 0..8 {
        match e.run_microtasks() {
            Ok(n) => println!("microtasks[{i}] ok n={n}"),
            Err(x) => println!("microtasks[{i}] ERR → {x:?}"),
        }
        match e.run_due_timers_bounded(200) {
            Ok(n) => println!("timers[{i}] ok n={n}"),
            Err(x) => println!("timers[{i}] ERR → {x:?}"),
        }
        for r in e.drain_unhandled_rejections() {
            println!("REJECTION → {r}");
        }
    }
}
