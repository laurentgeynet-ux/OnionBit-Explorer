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
    // APIs hôte/navigation installées côté jscore, absentes de
    // boa 0.22-dev (build path-dep sans `intl`) — déviation
    // intentionnelle documentée (JS-C581, audit) : boa lève
    // ReferenceError sur le global manquant, jscore exécute.
    ("clone_deep", "boa"),
    ("clone_circular", "boa"),
    ("clone_map_set", "boa"),
    ("clone_err_throws", "boa"),
    ("intl_collator", "boa"),
    ("intl_numfmt", "boa"),
    // `Error.captureStackTrace` — extension V8 non-spec gardée
    // pour la compat web (feature-detect) ; boa ne l'installe pas.
    ("err_capture", "boa"),
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
