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

use explorer_boa_diff::{concordant, eval_boa, eval_jscore, eval_v8, v8_available, Outcome};

include!("../src/probes.rs");

/// `(sonde, moteur fautif)` — divergences connues admises.
/// `jscore` = jscore s'écarte de la spec ; `boa` = boa s'écarte ;
/// `v8` = V8 s'écarte (rare — arbitre de référence).
const KNOWN: &[(&str, &str)] = &[
    // `.source` d'un `/` déjà échappé : boa ré-échappe la paire
    // `\/` → `a\\/b` non reparseable (`eval('/'+src+'/')` jette
    // SyntaxError chez boa, repasse chez jscore+V8 — bug boa
    // révélé par la triangulation, JS-C586).
    ("re_source_str", "boa"),
];

/// Divergences connues jscore↔V8 — arbitrées, documentées dans
/// `docs/audit-jscore-vs-boa.md` (§ canal V8).
const KNOWN_V8: &[(&str, &str)] = &[
    // `structuredClone(fn)` : jscore jette `TypeError` —
    // approximation assumée (`DataCloneError` est un DOMException
    // hôte, le ctor vit dans `install_dom` absent de l'eval nu) ;
    // V8/Node jette le vrai `DOMException`. Fixe différée —
    // règle de couverture réactive aux sites réels.
    ("clone_err_throws", "jscore"),
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

/// Cliquet triangulaire jscore↔V8 : rejoue le corpus contre Node
/// (`runner.js`, sémantique Script). Désarmé sans node — le canal
/// est facultatif ; quand il est là, il échoue sur toute
/// divergence nouvelle ou résolue (`KNOWN_V8` fait foi).
#[test]
fn corpus_ratchet_v8() {
    if !v8_available() {
        return;
    }
    let mut new_div = Vec::new();
    let mut resolved = Vec::new();
    for (name, src) in PROBES {
        let j = eval_jscore(src);
        let v = eval_v8(src);
        if concordant(&j, &v) {
            if KNOWN_V8.iter().any(|(n, _)| n == name) {
                resolved.push(*name);
            }
            continue;
        }
        if !KNOWN_V8.iter().any(|(n, _)| n == name) {
            new_div.push(format!("{name}: jscore={:?} v8={:?}", short(&j), short(&v)));
        }
    }
    assert!(
        new_div.is_empty() && resolved.is_empty(),
        "divergences nouvelles: {new_div:?} | résolues (mettre à jour KNOWN_V8): {resolved:?}"
    );
}

fn short(o: &Outcome) -> String {
    let s = format!("{o:?}");
    s.chars().take(140).collect()
}
