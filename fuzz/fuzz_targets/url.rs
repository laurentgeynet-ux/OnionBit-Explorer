// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Cible cargo-fuzz du parseur WHATWG URL : `parse` et `resolve`
//! sur entrées arbitraires — contrat « dégrader, jamais
//! paniquer » (erreur typée).
#![no_main]

use explorer_net::url::Url;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let _ = Url::parse(&text);
    // Résolution relative — chemins d'encodage host/query.
    if let Ok(base) = Url::parse("https://user@h.example:8080/a/b?x=1#f") {
        let _ = Url::resolve(&base, &text);
    }
});
