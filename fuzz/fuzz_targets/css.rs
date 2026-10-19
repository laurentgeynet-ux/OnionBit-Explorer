// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Cible cargo-fuzz du pipeline CSS : tokenizer + parser +
//! sélecteurs sur entrées arbitraires — contrat « dégrader,
//! jamais paniquer ».
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let sheet = explorer_css::parse_stylesheet(&text);
    // Les sélecteurs des règles reconnues doivent se re-parser
    // sans paniquer.
    for rule in &sheet.rules {
        if let explorer_css::Rule::Qualified(q) = rule {
            let _ = explorer_css::parse_selector_list(&q.prelude);
        }
    }
    // Cascade sur la feuille entière (chemins media/supports).
    let mut engine = explorer_css::StyleEngine::new();
    engine.add_css(&text, explorer_css::Origin::Author);
});
