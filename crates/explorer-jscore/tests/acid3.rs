// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Acid3 — sous-ensemble ECMAScript pur (JS-C36, harnais
//! informatif) : les 16 sous-tests 81-96 extraits verbatim de
//! `wpt/acid/acid3/test.html` (web-platform-tests). Les tests
//! 0-80 et 97-99 exigent DOM/Range/HTTP — hors périmètre moteur.
//!
//! Score tracké dans `CHANGELOG.md` (ADR-0010) : l'assert sert
//! de **cliquet** (régression interdite), pas de couverture
//! globale — 16/16 exécutés, 0 skip.

use explorer_jscore::Engine;

/// Nombre de sous-tests dans la fixture (dénominateur du score).
const SUBTESTS: usize = 16;
/// Cliquet : score minimal mesuré à l'introduction du harnais.
const FLOOR: usize = 16;

#[test]
fn acid3_ecmascript_subset_score() {
    let src = std::fs::read_to_string("tests/conformance/acid3-js-subset.js")
        .expect("fixture acid3-js-subset.js");
    let mut e = Engine::new();
    let report = e.eval(&src).map_or_else(
        |err| format!("harness parse/eval failed: {err}"),
        |v| e.describe(v),
    );
    eprintln!("{report}");
    // Le rapport textuel finit par `passed` — le score est le
    // premier entier du motif `acid3: N/16 passed`.
    let score = report
        .strip_prefix("acid3: ")
        .and_then(|r| r.split('/').next())
        .and_then(|n| n.parse::<usize>().ok())
        .unwrap_or(0);
    assert!(
        score >= FLOOR,
        "régression Acid3 JS : {score}/{SUBTESTS} < cliquet {FLOOR} — {report}"
    );
    assert_eq!(score, SUBTESTS, "Acid3 JS : {score}/{SUBTESTS} — {report}");
}
