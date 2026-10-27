// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Repro : `import()` defer → `deliver_import` → `.then` doit
//! courir (mesuré github : résolution OK, réaction jamais tirée).

use explorer_jscore::Engine;

#[test]
fn import_then_fires_after_deliver() {
    let mut e = Engine::new();
    e.set_import_drain();
    e.eval("window.__ir='pending';import('https://h/chunk.js').then(function(m){window.__ir='THEN '+m.a;},function(r){window.__ir='ERR '+r;});")
        .unwrap();
    let reqs = e.drain_dyn_imports();
    assert_eq!(reqs.len(), 1, "un import deferred attendu");
    e.deliver_import(reqs[0].seq, Ok("export const a=41".to_string()));
    let ran = e.run_microtasks().unwrap();
    eprintln!("microtasks ran={ran}");
    let out = e.eval("window.__ir").unwrap();
    assert_eq!(e.describe(out), "THEN 41", "réaction .then");
}
