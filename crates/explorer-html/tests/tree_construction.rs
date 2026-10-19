// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Conformance html5lib-tests **tree-construction** (ADR-0010).
//!
//! Le corpus `.dat` vit dans `tests/data/tree-construction/`,
//! extrait du repo `html5lib-tests` @ `9329e64694` (dernier commit
//! avant migration vers WPT) — jamais édité à la main.
//!
//! Format `.dat` : cas séparés par lignes `#data` (entrée),
//! `#errors`, `#document-fragment` (contexte), `#script-on`/`off`,
//! `#document` (arbre attendu, indentation `| `).
//!
//! Politique : les cas fragment et `#script-on` sont hors
//! périmètre (pas de parsing fragmentaire ni de scripting).
//! Le test est un **cliquet** : il échoue si le nombre de cas
//! réussis descend sous `BASELINE_PASSED` — toute régression
//! est bloquée, toute progression est relevée à l'exécution.

use std::path::Path;

use explorer_html::TreeBuilder;

mod common;
use common::{dump, normalize_expected, parse_cases};

/// Nombre de cas qui passent aujourd'hui — ne faire QUE monter.
/// Relevé à chaque avancée du tree-builder (cf. CHANGELOG).
const BASELINE_PASSED: usize = 953;

#[test]
fn html5lib_tree_construction() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/tree-construction");
    let mut total = 0usize;
    let mut skipped = 0usize;
    let mut passed = 0usize;
    let mut failures: Vec<String> = Vec::new();

    let mut entries: Vec<_> = std::fs::read_dir(&dir)
        .expect("corpus dir")
        .filter_map(std::result::Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "dat"))
        .collect();
    entries.sort();

    for path in &entries {
        let content = std::fs::read_to_string(path).expect("read .dat");
        for (i, case) in parse_cases(&content).into_iter().enumerate() {
            if case.fragment || case.script_on {
                skipped += 1;
                continue;
            }
            total += 1;
            let id = format!("{}:{i}", path.file_name().unwrap().to_string_lossy());
            let dom = TreeBuilder::parse(&case.input);
            if dump(&dom) == normalize_expected(&case.expected) {
                passed += 1;
            } else {
                failures.push(id);
            }
        }
    }

    let rate = passed * 100 / total.max(1);
    eprintln!("\n=== html5lib tree-construction ===");
    eprintln!("{passed}/{total} passent ({rate}%), {skipped} ignorés (fragment/script-on)");
    if !failures.is_empty() {
        // Triage (R-14) : comptage par fichier `.dat` —
        // regrouper les échecs par algorithme fautif.
        let mut per_file: std::collections::BTreeMap<String, usize> =
            std::collections::BTreeMap::new();
        for f in &failures {
            let file = f.rsplit_once(':').map_or(f.as_str(), |(n, _)| n);
            *per_file.entry(file.to_string()).or_default() += 1;
        }
        let mut sorted: Vec<_> = per_file.into_iter().collect();
        sorted.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
        eprintln!("échecs : {} (par fichier .dat)", failures.len());
        for (file, n) in &sorted {
            eprintln!("  {n:4} {file}");
        }
        for f in &failures {
            eprintln!("  {f}");
        }
    }
    assert!(
        passed >= BASELINE_PASSED,
        "régression : {passed} < baseline {BASELINE_PASSED}"
    );
    // Le score est exposé pour mise à jour manuelle du cliquet.
    eprintln!("BASELINE_PASSED courant = {BASELINE_PASSED}, mesuré = {passed}");
}
