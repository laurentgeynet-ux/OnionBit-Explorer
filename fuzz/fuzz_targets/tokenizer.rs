// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Cible cargo-fuzz du tokenizer + tree-builder : entrées
//! arbitraires chunkées, contrat « dégrader, jamais paniquer ».
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let mut t = explorer_html::Tokenizer::new();
    let mut tb = explorer_html::TreeBuilder::new();
    // Chunks de taille variable — force les reprises de coupure.
    for chunk in text.as_bytes().chunks(7) {
        if let Ok(s) = std::str::from_utf8(chunk) {
            t.feed(s);
        }
        while let Some(tok) = t.next_token() {
            tb.process(&mut t, tok);
        }
    }
    t.finish();
    while let Some(tok) = t.next_token() {
        tb.process(&mut t, tok);
    }
    let _ = tb.into_dom();
});
