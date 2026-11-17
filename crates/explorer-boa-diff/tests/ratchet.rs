// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Cliquet différentiel : rejoue tout le corpus en tests. Toute
//! divergence **nouvelle** (non listée) ou résolue échoue — le
//! fichier fait foi de baseline. Pour la mesure détaillée et
//! l'isolation par processus : `cargo run -p explorer-boa-diff`.
//!
//! Divergences connues (`KNOWN`) — chaque entrée est un bug mesuré
//! documenté dans `docs/audit-jscore-vs-boa.md` : retirer l'entrée
//! quand le bug est corrigé, jamais pour faire passer le test.

use explorer_boa_diff::{concordant, eval_boa, eval_jscore, Outcome};

include!("../src/probes.rs");

/// `(sonde, moteur fautif)` — divergences connues admises.
/// `jscore` = jscore s'écarte de la spec ; `boa` = boa s'écarte.
const KNOWN: &[(&str, &str)] = &[
    // Représentation UTF-8 interne : les surrogates isolés se
    // perdent en U+FFFD (gap structurel documenté — JS-C263 a
    // corrigé les positions, pas les valeurs).
    ("str_fromcc", "jscore"),
    ("str_lone_surrogate", "jscore"),
    // boa 0.22-dev : indexation astrale hors BMP (`'😀'[0] !==
    // '\uD800'` côté boa) — bug boa, jscore est spec-correct.
    ("str_astral_idx", "boa"),
];

#[test]
fn corpus_ratchet() {
    let mut new_div = Vec::new();
    let mut resolved = Vec::new();
    for (name, src) in PROBES {
        let j = eval_jscore(src);
        let b = eval_boa(src);
        if concordant(&j, &b) {
            if KNOWN.iter().any(|(n, _)| n == name) {
                resolved.push(*name);
            }
            continue;
        }
        if !KNOWN.iter().any(|(n, _)| n == name) {
            new_div.push(format!(
                "{name}: jscore={:?} boa={:?}",
                short(&j),
                short(&b)
            ));
        }
    }
    assert!(
        new_div.is_empty() && resolved.is_empty(),
        "divergences nouvelles: {new_div:?} | résolues (mettre à jour KNOWN): {resolved:?}"
    );
}

fn short(o: &Outcome) -> String {
    let s = format!("{o:?}");
    s.chars().take(140).collect()
}
