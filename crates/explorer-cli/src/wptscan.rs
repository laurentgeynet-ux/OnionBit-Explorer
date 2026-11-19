// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! `wpt-scan` — prospection du `MANIFEST.json` officiel des
//! web-platform-tests (ADR-0010).
//!
//! Le manifest est l'index canonique du WPT : il énumère
//! chaque test avec son type (`reftest`, `testharness`,
//! `crashtest`…), ses références et ses flags — le pendant
//! du frontmatter YAML de test262. Il n'est pas versionné
//! dans le dépôt WPT ; on consomme l'artefact pré-généré
//! publié en asset des releases `merge_pr_*`
//! (téléchargement et mise en cache : `scripts/scan_wpt.ps1`,
//! sous `Explorer-data/wpt/`).
//!
//! Format du JSON (version 9) : `items.<type>` est un arbre
//! dont les clés sont les segments de chemin ; une feuille
//! est `[hash, [url, [[ref, "=="|"!="], …], extras]]` —
//! `url` quasi toujours `null` (le chemin fait foi), les
//! refs sont des URL racine-dépôt (`/css/…`, `url_base`
//! vaut `/`), `extras` porte les drapeaux (`fuzzy`,
//! `timeout`, `testdriver`…).
//!
//! Sortie : une ligne TSV par test sous les préfixes
//! demandés — `chemin<TAB>refs<TAB>extras` — pour curation
//! vers `tests/data/wpt/manifest.txt`.

use std::io::BufReader;
use std::process::ExitCode;

use serde_json::Value;

/// Point d'entrée : `<manifest.json> [--type <t>] <préfixe>…`.
///
/// `--type` vaut `reftest` par défaut (seul type exécutable
/// par notre harnais statique aujourd'hui ; `print-reftest`,
/// `crashtest`… sont sélectionnables pour la prospection).
pub fn run(manifest: &str, args: &[String]) -> ExitCode {
    let mut kind = "reftest".to_string();
    let mut prefixes: Vec<String> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--type" {
            let Some(t) = args.get(i + 1) else {
                eprintln!("wpt-scan : --type attend une valeur");
                return ExitCode::FAILURE;
            };
            kind.clone_from(t);
            i += 2;
        } else {
            prefixes.push(args[i].clone());
            i += 1;
        }
    }
    if prefixes.is_empty() {
        eprintln!("usage : explorer-cli wpt-scan <MANIFEST.json> [--type reftest] <préfixe>…");
        eprintln!("        ex. explorer-cli wpt-scan Explorer-data/wpt/MANIFEST.json css/css-grid");
        return ExitCode::FAILURE;
    }

    let Ok(file) = std::fs::File::open(manifest) else {
        eprintln!("wpt-scan : manifest illisible : {manifest}");
        eprintln!("           (le télécharger via `pwsh -File scripts/scan_wpt.ps1 -Update`)");
        return ExitCode::FAILURE;
    };
    let root: Value = match serde_json::from_reader(BufReader::new(file)) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("wpt-scan : JSON invalide ({e}) — manifest tronqué ? relancer -Update");
            return ExitCode::FAILURE;
        }
    };
    let Some(tree) = root["items"][&kind].as_object() else {
        eprintln!("wpt-scan : pas de section items.{kind} dans ce manifest");
        return ExitCode::FAILURE;
    };

    let mut out = Vec::new();
    let mut parts = Vec::new();
    for (k, v) in tree {
        parts.push(k.clone());
        walk(v, &mut parts, &mut out);
        parts.pop();
    }
    out.retain(|(path, _, _)| {
        prefixes
            .iter()
            .any(|p| path.starts_with(p.trim_end_matches('/')))
    });
    out.sort();
    let count = out.len();
    for (path, refs, extras) in &out {
        println!("{path}\t{refs}\t{extras}");
    }
    eprintln!("{count} {kind} sous {}", prefixes.join(", "));
    ExitCode::SUCCESS
}

/// Descend l'arbre `items.<type>` : objet = nœud de chemin,
/// tableau = feuille `[hash, [url, refs, extras]]`.
fn walk(node: &Value, parts: &mut Vec<String>, out: &mut Vec<(String, String, String)>) {
    match node {
        Value::Object(map) => {
            for (k, v) in map {
                parts.push(k.clone());
                walk(v, parts, out);
                parts.pop();
            }
        }
        Value::Array(leaf) => {
            let Some(body) = leaf.get(1).and_then(Value::as_array) else {
                return;
            };
            let refs = body
                .get(1)
                .and_then(Value::as_array)
                .map(|refs| {
                    refs.iter()
                        .filter_map(|r| {
                            let pair = r.as_array()?;
                            Some(format!(
                                "{} {}",
                                pair.get(1)?.as_str()?,
                                pair.first()?.as_str()?
                            ))
                        })
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            let extras = body
                .get(2)
                .and_then(Value::as_object)
                .map(|ex| {
                    ex.iter()
                        .map(|(k, v)| {
                            let v = if v.is_string() {
                                v.as_str().unwrap_or_default().to_string()
                            } else {
                                v.to_string()
                            };
                            format!("{k}={v}")
                        })
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            out.push((parts.join("/"), refs, extras));
        }
        _ => {}
    }
}
