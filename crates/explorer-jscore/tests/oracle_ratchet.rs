// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Cliquet golden oracle : rejoue tout le corpus différentiel
//! dans `explorer-jscore` **seul** et compare chaque issue
//! canonique à la baseline gravée dans
//! `data/oracle_golden.txt`. La baseline a été validée par
//! triangulation boa+V8 (audit `docs/audit-jscore-vs-boa.md`,
//! JS-C586/587) — le filet anti-régression survit au
//! débranchement de boa et tourne sans Node.
//!
//! Bless (regénérer la baseline après un changement de
//! comportement *intentionnel*) :
//! `EXPLORER_BLESS=1 cargo test -p explorer-jscore --test
//! oracle_ratchet` — le diff git du golden est la revue.

include!("oracle_corpus.rs");

/// Issue canonique — miroir de `explorer_boa_diff::Outcome`
/// (reproduite ici : le filet doit survivre à la suppression du
/// crate oracle ; le format `Debug` échappe les sauts de ligne —
/// une ligne de golden = une sonde).
#[derive(Debug)]
#[allow(dead_code)] // les champs ne sont lus que via `{:?}` (golden)
enum Outcome {
    /// `ToString` de la valeur de complétion.
    Value(String),
    /// Exception levée : `Nom: message`.
    Throw(String),
    /// Erreur de syntaxe (parse).
    Syntax(String),
    /// Échec interne au moteur (limite, deadline).
    Internal(String),
}

fn eval_jscore(src: &str) -> Outcome {
    let mut e = explorer_jscore::Engine::new();
    // `$262` hôte (JS-C607) : opt-in du moteur, posé ici pour que
    // les sondes `c607_*` exercent les realms — aucune sonde
    // existante n'énumère le global ni ne référence `$262`.
    e.install_t262();
    match e.eval(src) {
        Ok(v) => Outcome::Value(e.describe(v)),
        Err(explorer_jscore::Error::Syntax { message, .. }) => Outcome::Syntax(message),
        Err(explorer_jscore::Error::Uncaught(s)) => Outcome::Throw(s),
        Err(explorer_jscore::Error::Deadline) => Outcome::Internal("deadline".to_string()),
    }
}

const GOLDEN: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/oracle_golden.txt");

#[test]
fn oracle_golden_ratchet() {
    // Garde-fou noms uniques : le golden est indexé par nom —
    // un doublon rendrait la comparaison silencieusement fausse.
    let mut seen = std::collections::BTreeSet::new();
    for (name, _) in PROBES {
        assert!(seen.insert(name), "nom de sonde dupliqué: {name}");
    }

    let mut lines = Vec::with_capacity(PROBES.len());
    for (name, src) in PROBES {
        lines.push(format!("{name}\t{:?}", eval_jscore(src)));
    }

    if std::env::var_os("EXPLORER_BLESS").is_some() {
        std::fs::write(GOLDEN, lines.join("\n") + "\n").expect("bless: écrire le golden");
        return;
    }

    let golden = std::fs::read_to_string(GOLDEN).expect("golden manquant — blesser d'abord");
    let expected: Vec<&str> = golden.lines().collect();
    assert_eq!(
        expected.len(),
        lines.len(),
        "taille du golden ({}) ≠ corpus ({}) — blesser ou reverter",
        expected.len(),
        lines.len()
    );
    let mut diffs = Vec::new();
    for (exp, got) in expected.iter().zip(lines.iter()) {
        if exp != got {
            diffs.push(format!("- {exp}\n+ {got}"));
        }
    }
    assert!(
        diffs.is_empty(),
        "{} sonde(s) régressée(s) vs golden:\n{}",
        diffs.len(),
        diffs
            .iter()
            .take(20)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}
