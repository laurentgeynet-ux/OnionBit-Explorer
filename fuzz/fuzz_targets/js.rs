// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Cible cargo-fuzz du moteur JS : lexer → parser → évaluateur sur
//! entrées arbitraires — contrat « dégrader, jamais paniquer ».
//! Les budgets (ops, profondeur, mémoire heap) sont bornés pour
//! garder l'itération rapide et prouver que les garde-fous
//! tiennent sous contenu hostile (ADR-0019).
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let mut e = explorer_jscore::Engine::new();
    // Bornes serrées : 50 K ops et ~32 Mio de heap suffisent à
    // explorer le parseur/l'évaluateur sans traîner.
    e.set_op_limit(50_000);
    e.set_mem_limit(32 * 1024 * 1024);
    // Script classique : parse complet + exécution bornée.
    let _ = e.eval(&text);
    // Module : chemin parseur distinct (imports/exports stricts).
    let _ = e.eval_module(&text, "fuzz.js");
    // Scan des spécificateurs — parse + drop immédiat (chemin
    // exercé par la résolution de modules sans évaluation).
    let _ = explorer_jscore::module_specifiers(&text);
});
