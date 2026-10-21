// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! JS-B — harness test262 (ADR-0010) : extrait versionné de la suite
//! officielle ECMAScript, exécuté fichier par fichier sur un
//! `Engine` neuf. Le cliquet `BASELINE_PASSED`/`BASELINE_SKIPPED`
//! verrouille le score — la CI échoue si la couverture régresse.
//!
//! Sémantique exécutée (sous-ensemble test262 minimal viable) :
//! - frontmatter `/*--- … ---*/` : `flags`, `features`, `includes`,
//!   `negative.{phase,type}` parsés par balayage (le corpus n'a
//!   besoin ni de YAML complet ni d'expressions) ;
//! - `flags: [module, async]` → skip (hors périmètre moteur) ;
//!   `raw` → aucun prélude ; sinon `sta.js` + `assert.js` + les
//!   `includes:` demandés sont injectés ;
//! - `onlyStrict` → préfixe `"use strict"` ; `noStrict`/absent →
//!   mode sloppy (une seule passe — test262 en fait deux, le delta
//!   strict/sloppy est documenté comme écart) ;
//! - `negative.phase` ∈ {`parse`, `early`, `resolution`} → une
//!   erreur de syntaxe est attendue ; `runtime` → un throw est
//!   attendu (le `type` exact n'est pas vérifié en v0 — écart
//!   documenté, à resserrer quand le typage d'erreurs JS suivra) ;
//! - `features` : tout feature hors whitelist → skip. La whitelist
//!   reflète le moteur réel (classes, flèches, templates, let/const,
//!   Symbol, Map/Set, Promise…) ; les skips sont comptés, jamais
//!   silencieux.

use explorer_jscore::{Engine, Error};
use std::fs;
use std::path::{Path, PathBuf};

/// Cliquet de régression — **re-mesuré après la vague regexp
/// (JS-C43/C48/C36) puis trois fixes de conformité**. Score @
/// extraction d53ebc8 : **1046/1046 exécutés, 31 skips, 0 échec**.
/// Les 39 échecs de la re-mesure regexp (1007/1046, 31 skips)
/// ont tous été soldés : négatifs de séparateurs numériques à
/// zéro en tête (`08`/`0_8`/`0_0` → `SyntaxError`, 8 cas),
/// `put-value-prop-base-primitive` (proxy/setter amarré sur la
/// chaîne proto d'un receiver primitif, 1 cas), et noms privés
/// `#\uXXXX` — le lexer privé ignorait les échappes Unicode
/// (30 cas, un seul mécanisme).
const BASELINE_PASSED: usize = 1046;
/// Plafond de skips toléré — monte seulement si le corpus grossit.
const BASELINE_SKIPPED_MAX: usize = 31;

/// Features présents dans le corpus mais absents du moteur —
/// vérifiés contre `builtins.rs`/`lexer.rs`/`regexp.rs`, pas
/// supposés : skip explicite et compté. Tout le reste TOURNE et
/// échoue honnêtement si le moteur ne suit pas.
///
/// Liste re-vérifiée à la vague JS-8+ (re-mesure) : generators,
/// async (functions/iteration/`for await`), `BigInt`, `TypedArray`/
/// `ArrayBuffer`/`DataView`/SAB/`Atomics`, `Intl`, `Temporal`,
/// `WeakRef`/`FinalizationRegistry`, `Proxy`, `import.meta`,
/// dynamic `import()`, regexp lookbehind/`d`-flag, séparateurs
/// `1_0`, champs et méthodes privés de classe, `static {}`,
/// `Symbol.asyncIterator`/`matchAll`/`toStringTag`/`toPrimitive`/
/// `species` sont tous implémentés — ils tournent donc désormais au
/// lieu d'être skippés.
const UNSUPPORTED: &[&str] = &[
    // Jamais implémentés (hors périmètre ou inobservables).
    "hashbang",               // pas de commentaire `#!` ligne 1
    "IsHTMLDDA",              // `document.all` (Annex B host)
    "cross-realm",            // un seul realm par Engine
    "tail-call-optimization", // tree-walker sans PTC
];

/// Frontmatter `/*--- … ---*/` minimal — les champs qui pilotent
/// l'exécution.
#[derive(Default)]
struct Meta {
    flags: Vec<String>,
    features: Vec<String>,
    includes: Vec<String>,
    negative_phase: Option<String>,
    #[allow(dead_code)]
    negative_type: Option<String>,
}

fn meta(src: &str) -> Meta {
    let mut m = Meta::default();
    let Some(start) = src.find("/*---") else {
        return m;
    };
    let Some(end) = src[start..].find("---*/") else {
        return m;
    };
    let block = &src[start + 5..start + end];
    let mut key = "";
    for raw in block.lines() {
        let line = raw.trim_end();
        let t = line.trim_start();
        if t.is_empty() {
            continue;
        }
        if let Some(item) = t.strip_prefix("- ") {
            // Élément de liste YAML — appartient à la clé courante.
            let item = item.trim().to_string();
            match key {
                "flags" => m.flags.push(item),
                "features" => m.features.push(item),
                "includes" => m.includes.push(item),
                _ => {}
            }
            continue;
        }
        let Some((k, v)) = t.split_once(':') else {
            continue;
        };
        let k = k.trim();
        let v = v.trim();
        match (key, k) {
            ("negative", "phase") => m.negative_phase = Some(v.to_string()),
            ("negative", "type") => m.negative_type = Some(v.to_string()),
            _ => {}
        }
        // Clés de premier niveau (indentation nulle dans le bloc).
        if line.starts_with(char::is_alphabetic) {
            key = "";
            match k {
                "flags" | "features" | "includes" => {
                    key = match k {
                        "flags" => "flags",
                        "features" => "features",
                        _ => "includes",
                    };
                    let list = match key {
                        "flags" => &mut m.flags,
                        "features" => &mut m.features,
                        _ => &mut m.includes,
                    };
                    if v.starts_with('[') {
                        let inner = v.trim_start_matches('[').trim_end_matches(']');
                        for it in inner.split(',') {
                            let it = it.trim();
                            if !it.is_empty() {
                                list.push(it.to_string());
                            }
                        }
                    } else if !v.is_empty() {
                        list.push(v.to_string());
                    }
                }
                "negative" => key = "negative",
                _ => {}
            }
        }
    }
    m
}

fn corpus_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/conformance/test262")
}

fn harness_src(name: &str) -> Option<String> {
    fs::read_to_string(corpus_root().join("_harness").join(name)).ok()
}

/// Construit la source exécutable : prélude (sauf `raw`) +
/// includes + corps du test (frontmatter laissé — c'est un
/// commentaire).
fn build_source(m: &Meta, body: &str) -> Option<String> {
    let mut src = String::new();
    // `onlyStrict` : la directive doit ouvrir le script entier
    // (le harnais tourne alors en strict — c'est le modèle du
    // runner officiel test262).
    if m.flags.iter().any(|f| f == "onlyStrict") {
        src.push_str("\"use strict\";\n");
    }
    if !m.flags.iter().any(|f| f == "raw") {
        src.push_str(&harness_src("sta.js")?);
        src.push('\n');
        src.push_str(&harness_src("assert.js")?);
        src.push('\n');
    }
    for inc in &m.includes {
        src.push_str(&harness_src(inc)?);
        src.push('\n');
    }
    src.push_str(body);
    Some(src)
}

fn skip_reason(m: &Meta) -> Option<String> {
    for f in &m.flags {
        if f == "module" || f == "async" {
            return Some(format!("flag:{f}"));
        }
    }
    for feat in &m.features {
        if UNSUPPORTED.contains(&feat.as_str()) {
            return Some(format!("feature:{feat}"));
        }
    }
    for inc in &m.includes {
        if harness_src(inc).is_none() {
            return Some(format!("include:{inc}"));
        }
    }
    None
}

/// Verdict attendu d'un fichier.
fn run_one(_path: &Path, body: &str) -> Result<(), String> {
    let m = meta(body);
    if let Some(reason) = skip_reason(&m) {
        return Err(format!("SKIP {reason}"));
    }
    let Some(src) = build_source(&m, body) else {
        return Err("SKIP harness-missing".to_string());
    };
    let mut e = Engine::new();
    let outcome = e.eval(&src);
    match m.negative_phase.as_deref() {
        Some("parse" | "early" | "resolution") => match outcome {
            Err(Error::Syntax { .. }) => Ok(()),
            other => Err(format!("expected syntax error, got {other:?}")),
        },
        Some("runtime") => match outcome {
            Err(_) => Ok(()),
            Ok(v) => Err(format!("expected throw, completed {v:?}")),
        },
        Some(_) | None => match outcome {
            Ok(_) => Ok(()),
            Err(err) => Err(format!("{err:?}")),
        },
    }
}

fn collect_js(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(std::result::Result::ok)
        .map(|e| e.path())
        .collect();
    paths.sort();
    for p in paths {
        if p.is_dir() {
            if p.file_name() != Some(std::ffi::OsStr::new("_harness")) {
                collect_js(&p, out);
            }
        } else if p.extension() == Some(std::ffi::OsStr::new("js")) {
            out.push(p);
        }
    }
}

/// Certains tests test262 nichent profondément (`(((((…)))))`,
/// `a = a = a = …`) — le parseur récursif a besoin d'une pile
/// large. Le moteur reste responsable de ses budgets ; le harness
/// travaille dans un thread dimensionné pour la suite.
const HARNESS_STACK: usize = 512 * 1024 * 1024;

#[test]
fn test262_baseline() {
    std::thread::Builder::new()
        .stack_size(HARNESS_STACK)
        .spawn(baseline_inner)
        .expect("spawn harness thread")
        .join()
        .expect("harness thread panicked");
}

fn baseline_inner() {
    let root = corpus_root();
    let mut files = Vec::new();
    collect_js(&root, &mut files);
    assert_eq!(files.len(), 1077, "corpus drifted — re-extract");

    let mut passed = 0usize;
    let mut skipped = 0usize;
    let mut failures: Vec<(PathBuf, String)> = Vec::new();
    let mut skip_reasons: std::collections::BTreeMap<String, usize> =
        std::collections::BTreeMap::new();

    for f in &files {
        let body = match fs::read_to_string(f) {
            Ok(b) => b,
            Err(err) => {
                failures.push((f.clone(), format!("read: {err}")));
                continue;
            }
        };
        match run_one(f, &body) {
            Ok(()) => passed += 1,
            Err(msg) if msg.starts_with("SKIP ") => {
                skipped += 1;
                *skip_reasons
                    .entry(msg.trim_start_matches("SKIP ").to_string())
                    .or_default() += 1;
            }
            Err(msg) => failures.push((f.clone(), msg)),
        }
    }

    eprintln!("\n=== test262 (subset langage, extraction d53ebc8) ===");
    eprintln!("passed {passed} / {}", files.len() - skipped);
    eprintln!("skipped {skipped}");
    for (r, n) in &skip_reasons {
        eprintln!("  {r}: {n}");
    }
    eprintln!("failed {}", failures.len());
    for (f, msg) in &failures {
        eprintln!("  {} — {msg}", f.display());
    }

    assert!(
        skipped <= BASELINE_SKIPPED_MAX,
        "skipped {skipped} > ratchet {BASELINE_SKIPPED_MAX}"
    );
    // BASELINE_PASSED vaut 0 tant que la première mesure n'a
    // pas été gravée — la comparaison est alors volontairement
    // tautologique (cliquet non encore armé).
    #[allow(clippy::absurd_extreme_comparisons)]
    {
        assert!(
            passed >= BASELINE_PASSED,
            "coverage regression: {passed} < {BASELINE_PASSED}"
        );
    }
    // Le cliquet `passed >= BASELINE` verrouille ; la liste des
    // échecs reste lisible via `--nocapture`.
}
