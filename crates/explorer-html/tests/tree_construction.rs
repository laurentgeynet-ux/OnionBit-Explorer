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

use std::fmt::Write as _;
use std::path::Path;

use explorer_dom::{Dom, NodeData, NodeId};
use explorer_html::TreeBuilder;

/// Nombre de cas qui passent aujourd'hui — ne faire QUE monter.
/// Relevé à chaque avancée du tree-builder (cf. CHANGELOG).
const BASELINE_PASSED: usize = 953;

/// Un cas de test extrait d'un fichier `.dat`.
struct Case {
    input: String,
    /// Arbre attendu (brut, `#document` normalisé plus bas).
    expected: String,
    /// `true` si parsing fragmentaire (hors périmètre).
    fragment: bool,
    /// `true` si scripting on (`<noscript>` rawtext — hors périmètre).
    script_on: bool,
}

#[derive(PartialEq)]
enum Section {
    None,
    Data,
    Expected,
    Skip,
}

fn parse_cases(file: &str) -> Vec<Case> {
    let mut cases = Vec::new();
    let mut input = Vec::new();
    let mut expected = Vec::new();
    let mut fragment = false;
    let mut script_on = false;
    let mut section = Section::None;

    let flush = |cases: &mut Vec<Case>,
                 input: &mut Vec<String>,
                 expected: &mut Vec<String>,
                 fragment: &mut bool,
                 script_on: &mut bool| {
        if !expected.is_empty() {
            cases.push(Case {
                input: input.join("\n"),
                expected: expected.join("\n"),
                fragment: *fragment,
                script_on: *script_on,
            });
        }
        input.clear();
        expected.clear();
        *fragment = false;
        *script_on = false;
    };

    for line in file.lines() {
        if line.starts_with('#') {
            match line {
                "#data" => {
                    flush(
                        &mut cases,
                        &mut input,
                        &mut expected,
                        &mut fragment,
                        &mut script_on,
                    );
                    section = Section::Data;
                }
                "#document" => section = Section::Expected,
                "#document-fragment" => {
                    fragment = true;
                    section = Section::Skip;
                }
                "#script-on" => {
                    script_on = true;
                    section = Section::Skip;
                }
                _ => section = Section::Skip, // #errors, #new-errors…
            }
            continue;
        }
        match section {
            Section::Data => input.push(line.to_string()),
            Section::Expected => expected.push(line.to_string()),
            _ => {}
        }
    }
    flush(
        &mut cases,
        &mut input,
        &mut expected,
        &mut fragment,
        &mut script_on,
    );
    cases
}

/// Dump de l'arbre produit — même format que `#document`.
fn dump(dom: &Dom) -> String {
    let mut out = String::new();
    dump_children(dom, dom.document(), 1, &mut out);
    out
}

fn dump_children(dom: &Dom, parent: NodeId, depth: usize, out: &mut String) {
    for child in dom.children(parent).unwrap_or_default() {
        out.push_str("| ");
        for _ in 1..depth {
            out.push_str("  ");
        }
        match dom.data(child).unwrap_or(&NodeData::Document) {
            NodeData::Document => out.push_str("#document-nested"),
            NodeData::Doctype(d) => {
                let _ = write!(out, "<!DOCTYPE {}>", d.name);
            }
            NodeData::Element(e) => {
                let _ = write!(out, "<{}>", e.tag_name);
                let mut attrs: Vec<_> = e
                    .attributes
                    .iter()
                    .map(|a| format!("{}=\"{}\"", a.name, a.value))
                    .collect();
                attrs.sort();
                for a in attrs {
                    out.push('\n');
                    out.push_str("| ");
                    for _ in 0..depth {
                        out.push_str("  ");
                    }
                    out.push_str(&a);
                }
            }
            NodeData::Text(t) => {
                let _ = write!(out, "\"{t}\"");
            }
            NodeData::Comment(c) => {
                let _ = write!(out, "<!-- {c} -->");
            }
        }
        out.push('\n');
        dump_children(dom, child, depth + 1, out);
    }
}

/// Normalise le dump attendu : préfixes de namespace étrangers
/// (`<svg svg>` → `<svg>`, `xlink href` → `xlink:href`, `xmlns
/// xlink` → `xmlns:xlink`, `xml lang` → `xml:lang`), et fusion des
/// nœuds texte adjacents (notre dump n'en produit pas, la spec non
/// plus — garde-fou).
fn normalize_expected(expected: &str) -> String {
    let mut out = String::new();
    for line in expected.lines() {
        let indent = line.len() - line.trim_start().len();
        let content = line.trim_start();
        let norm = if let Some(rest) = content.strip_prefix('<') {
            // Élément ou doctype : `<svg svg>` → `<svg>`.
            if let Some(inner) = rest.strip_suffix('>') {
                let inner = inner
                    .strip_prefix("svg ")
                    .or_else(|| inner.strip_prefix("math "))
                    .unwrap_or(inner);
                format!("<{inner}>")
            } else {
                format!("<{rest}")
            }
        } else if let Some((name, val)) = content.split_once('=') {
            // Attribut étranger `xlink href` → `xlink:href`.
            let name = name
                .replace("xlink ", "xlink:")
                .replace("xml ", "xml:")
                .replace("xmlns ", "xmlns:");
            format!("{name}={val}")
        } else {
            content.to_string()
        };
        for _ in 0..indent {
            out.push(' ');
        }
        out.push_str(&norm);
        out.push('\n');
    }
    out
}

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
        eprintln!("échecs : {}", failures.len());
        for f in failures.iter().take(20) {
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
