// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! JS-B — harness test262 (ADR-0010) : extrait versionné de la suite
//! officielle ECMAScript, exécuté fichier par fichier sur un
//! `Engine` neuf. Le cliquet `BASELINE_PASSED`/`BASELINE_SKIPPED`
//! verrouille le score — la CI échoue si la couverture régresse.
//!
//! Sémantique exécutée :
//! - frontmatter `/*--- … ---*/` : `flags`, `features`, `includes`,
//!   `negative.{phase,type}` — mini-YAML complet (listes `- item`
//!   et `[a, b]`, flow-map `negative: {phase: …, type: …}`,
//!   scalaires, sous-clés indentées) ;
//! - **double passe strict + sloppy** (modèle du runner officiel) :
//!   `raw`/`noStrict` → sloppy seul, `onlyStrict` → strict seul,
//!   défaut → les deux — chaque passe-mode compte dans le score ;
//! - `flags: [module]` → `eval_module` (imports → skip `loader`) ;
//! - `flags: [async]` → shim `$DONE` + `run_microtasks` : le test
//!   échoue si `$DONE` reste non réglé ou reçoit une erreur ;
//! - `negative.phase` ∈ {`parse`, `early`, `resolution`} → erreur
//!   de syntaxe attendue ; `runtime` → throw attendu **et `type`
//!   vérifié** (`Uncaught` porte `Name: message` — le préfixe fait
//!   foi de classe) ;
//! - `features` : tout feature hors whitelist → skip. La whitelist
//!   reflète le moteur réel ; les skips sont comptés, jamais
//!   silencieux, et le score est rapporté **par répertoire**.

use explorer_jscore::{Engine, Error, Value};
use std::fs;
use std::path::{Path, PathBuf};

/// Cliquet de régression — **gravé en passes-mode après la refonte
/// BOA-10..14** (frontmatter complet, double passe strict/sloppy,
/// `async`/`module`, `negative.type`). Score @ extraction d53ebc8 :
/// **2045/2045 passes-mode exécutées, 31 skips fichier, 0 échec**
/// — strict identique à sloppy sur toute la sous-extraction lexer
/// (comments 46, identifiers 535, line-terminators 82, literals
/// 1041, types 207, white-space 134 — passes par répertoire).
const BASELINE_PASSED: usize = 2045;
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

/// Frontmatter `/*--- … ---*/` — les champs qui pilotent
/// l'exécution.
#[derive(Default)]
struct Meta {
    flags: Vec<String>,
    features: Vec<String>,
    includes: Vec<String>,
    negative_phase: Option<String>,
    negative_type: Option<String>,
}

/// Découpe `[a, b]` ou `{k: v, …}` en items `a`/`k: v` — le
/// mini-YAML test262 n'imbrique jamais au-delà d'un niveau.
fn split_flow(inner: &str) -> Vec<String> {
    inner
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

fn push_list<'a>(m: &'a mut Meta, k: &str) -> Option<&'a mut Vec<String>> {
    match k {
        "flags" => Some(&mut m.flags),
        "features" => Some(&mut m.features),
        "includes" => Some(&mut m.includes),
        _ => None,
    }
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
            // Élément de liste — appartient à la clé courante.
            if let Some(list) = push_list(&mut m, key) {
                list.push(item.trim().to_string());
            }
            continue;
        }
        let Some((k, v)) = t.split_once(':') else {
            continue;
        };
        let k = k.trim();
        let v = v.trim();
        // Sous-clés indentées du bloc `negative:`.
        match (key, k) {
            ("negative", "phase") => m.negative_phase = Some(v.to_string()),
            ("negative", "type") => m.negative_type = Some(v.to_string()),
            _ => {}
        }
        // Clés de premier niveau (indentation nulle dans le bloc).
        if line.starts_with(char::is_alphabetic) {
            key = "";
            if k == "negative" {
                key = "negative";
                // Flow-map `negative: {phase: …, type: …}`.
                if let Some(inner) = v.strip_prefix('{').and_then(|s| s.strip_suffix('}')) {
                    for pair in split_flow(inner) {
                        if let Some((sk, sv)) = pair.split_once(':') {
                            match sk.trim() {
                                "phase" => m.negative_phase = Some(sv.trim().to_string()),
                                "type" => m.negative_type = Some(sv.trim().to_string()),
                                _ => {}
                            }
                        }
                    }
                }
            } else if let Some(list) = push_list(&mut m, k) {
                key = match k {
                    "flags" => "flags",
                    "features" => "features",
                    _ => "includes",
                };
                if let Some(inner) = v.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
                    list.extend(split_flow(inner));
                } else if !v.is_empty() {
                    list.push(v.to_string());
                }
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

/// Mode d'une passe — la sémantique officielle test262 exécute
/// strict ET sloppy sauf restriction de flag ; `module` est un
/// mode à part (toujours strict, `eval_module`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Mode {
    Sloppy,
    Strict,
    Module,
}

/// Passes demandées par les flags : `module` → Module seul ;
/// `onlyStrict` → Strict seul ; `raw`/`noStrict` → Sloppy seul ;
/// sinon les deux.
fn modes(m: &Meta) -> &'static [Mode] {
    if m.flags.iter().any(|f| f == "module") {
        &[Mode::Module]
    } else if m.flags.iter().any(|f| f == "onlyStrict") {
        &[Mode::Strict]
    } else if m.flags.iter().any(|f| f == "noStrict" || f == "raw") {
        &[Mode::Sloppy]
    } else {
        &[Mode::Sloppy, Mode::Strict]
    }
}

/// Shim `$DONE` (les tests `flags: async`) — le `doneprintHandle`
/// officiel dépend d'un `print` hôte ; la sentinelle porte
/// `globalThis.__t262_done` relue après `run_microtasks`.
const ASYNC_SHIM: &str = "globalThis.__t262_done = \"pending\";\n\
     globalThis.__t262_err = undefined;\n\
     function $DONE(err) { globalThis.__t262_done = (err === undefined) ? \"ok\" : \"err\"; \
     globalThis.__t262_err = err; }\n";

/// Construit la source exécutable pour une passe : directive
/// strict éventuelle, prélude (sauf `raw`), includes, shim async,
/// puis le corps du test (frontmatter laissé — commentaire).
fn build_source(m: &Meta, body: &str, mode: Mode) -> Option<String> {
    let mut src = String::new();
    if mode == Mode::Strict {
        src.push_str("\"use strict\";\n");
    }
    if !m.flags.iter().any(|f| f == "raw") {
        src.push_str(&harness_src("sta.js")?);
        src.push('\n');
        src.push_str(&harness_src("assert.js")?);
        src.push('\n');
    }
    for inc in &m.includes {
        // `doneprintHandle.js` suppose un `print` hôte — notre
        // shim `$DONE` le remplace entièrement.
        if inc == "doneprintHandle.js" {
            continue;
        }
        src.push_str(&harness_src(inc)?);
        src.push('\n');
    }
    if m.flags.iter().any(|f| f == "async") {
        src.push_str(ASYNC_SHIM);
    }
    src.push_str(body);
    Some(src)
}

/// Le test charge-t-il des modules externes ? `import`/`export
/// … from` avec spécifieur ou `import(…)` dynamique → il faudrait
/// un loader test262 — skip compté `loader`.
fn needs_loader(body: &str) -> bool {
    let mut rest = body;
    while let Some(i) = rest.find(['i', 'e']) {
        let w = &rest[i..];
        let word = if w.starts_with("import") {
            "import"
        } else if w.starts_with("export") {
            "export"
        } else {
            rest = &rest[1..];
            continue;
        };
        let after = rest[i + word.len()..].chars().next();
        let bounded = !after.is_some_and(|c| c == '_' || c == '$' || c.is_alphanumeric());
        if bounded {
            let stmt = &rest[i + word.len()..];
            let head = stmt[..stmt.len().min(300)]
                .split(';')
                .next()
                .unwrap_or(stmt);
            let h = head.trim_start();
            if h.starts_with('"')
                || h.starts_with('\'')
                || h.starts_with('(')
                || head.contains("from")
            {
                return true;
            }
        }
        rest = &rest[i + word.len()..];
    }
    false
}

fn skip_reason(m: &Meta) -> Option<String> {
    for feat in &m.features {
        if UNSUPPORTED.contains(&feat.as_str()) {
            return Some(format!("feature:{feat}"));
        }
    }
    for inc in &m.includes {
        if inc != "doneprintHandle.js" && harness_src(inc).is_none() {
            return Some(format!("include:{inc}"));
        }
    }
    None
}

/// `negative.type` vs erreur levée — `thrown` porte le rendu
/// `Name: message` (`describe`/`Uncaught`) : le préfixe avant
/// `:` ou la fin fait foi de classe. `phase:parse` est rapporté
/// comme `"SyntaxError"`.
fn check_thrown_type(m: &Meta, thrown: &str) -> Result<(), String> {
    match &m.negative_type {
        None => Ok(()),
        Some(t)
            if thrown == t
                || thrown.starts_with(&format!("{t}:"))
                || thrown.starts_with(&format!("{t} ")) =>
        {
            Ok(())
        }
        Some(t) => Err(format!("negative type {t} vs thrown {thrown}")),
    }
}

/// Verdict d'une passe, factorisé (script/module) : `phase`
/// `parse|early|resolution` → syntaxe attendue ; `runtime` → throw
/// typé ; sinon complétion normale.
fn verdict(m: &Meta, out: Result<Value, Error>) -> Result<(), String> {
    match m.negative_phase.as_deref() {
        Some("parse" | "early" | "resolution") => match out {
            Err(Error::Syntax { .. }) => check_thrown_type(m, "SyntaxError"),
            other => Err(format!("expected syntax error, got {other:?}")),
        },
        Some("runtime") => match out {
            Err(Error::Uncaught(s)) => check_thrown_type(m, &s),
            Err(Error::Deadline) => Err("expected throw, got deadline".to_string()),
            Err(Error::Syntax { message, .. }) => {
                Err(format!("expected runtime throw, got syntax: {message}"))
            }
            Ok(_) => Err("expected throw, completed".to_string()),
        },
        Some(_) | None => match out {
            Ok(_) => Ok(()),
            Err(err) => Err(format!("{err:?}")),
        },
    }
}

/// Verdict `flags: async` : l'éval doit compléter, puis la file
/// de microtasks est drainée (bornée en interne) et la sentinelle
/// `$DONE` relue — `"ok"` seul passe.
fn finish_async(e: &mut Engine, out: Result<Value, Error>) -> Result<(), String> {
    if let Err(err) = out {
        Err(format!("{err:?}"))
    } else {
        if let Err(err) = e.run_microtasks() {
            return Err(format!("microtasks: {err:?}"));
        }
        match e.eval("globalThis.__t262_done") {
            Ok(v) => match e.describe(v).as_str() {
                "ok" => Ok(()),
                "err" => {
                    let msg = e
                        .eval("String(globalThis.__t262_err)")
                        .ok()
                        .map(|v| e.describe(v))
                        .unwrap_or_default();
                    Err(format!("$DONE error: {msg}"))
                }
                _ => Err("$DONE jamais réglé".to_string()),
            },
            Err(err) => Err(format!("done-check: {err:?}")),
        }
    }
}

/// Une passe d'un fichier dans un mode donné.
fn run_pass(path: &Path, m: &Meta, body: &str, mode: Mode) -> Result<(), String> {
    if mode == Mode::Module && needs_loader(body) {
        return Err("SKIP loader".to_string());
    }
    let Some(src) = build_source(m, body, mode) else {
        return Err("SKIP harness-missing".to_string());
    };
    let mut e = Engine::new();
    let out = match mode {
        Mode::Module => e.eval_module(&src, &path.to_string_lossy()),
        Mode::Sloppy | Mode::Strict => e.eval(&src),
    };
    if m.flags.iter().any(|f| f == "async") && m.negative_phase.is_none() {
        return finish_async(&mut e, out);
    }
    verdict(m, out)
}

/// Toutes les passes d'un fichier (skip compté une fois).
fn run_one(path: &Path, body: &str) -> Vec<(Mode, Result<(), String>)> {
    let m = meta(body);
    if let Some(reason) = skip_reason(&m) {
        return vec![(Mode::Sloppy, Err(format!("SKIP {reason}")))];
    }
    modes(&m)
        .iter()
        .map(|mode| (*mode, run_pass(path, &m, body, *mode)))
        .collect()
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

    let mut passed = 0usize; // passes-mode réussies
    let mut skipped = 0usize; // fichiers skippés (une fois)
    let mut executed = 0usize; // passes-mode jouées
    let mut failures: Vec<(PathBuf, Mode, String)> = Vec::new();
    let mut skip_reasons: std::collections::BTreeMap<String, usize> =
        std::collections::BTreeMap::new();
    let mut per_dir: std::collections::BTreeMap<String, (usize, usize)> =
        std::collections::BTreeMap::new();

    for f in &files {
        let body = match fs::read_to_string(f) {
            Ok(b) => b,
            Err(err) => {
                failures.push((f.clone(), Mode::Sloppy, format!("read: {err}")));
                continue;
            }
        };
        let dir = f
            .strip_prefix(&root)
            .ok()
            .and_then(|p| p.components().next())
            .map_or_else(
                || ".".to_string(),
                |c| c.as_os_str().to_string_lossy().into_owned(),
            );
        for (mode, r) in run_one(f, &body) {
            match r {
                Ok(()) => {
                    passed += 1;
                    executed += 1;
                    per_dir.entry(dir.clone()).or_default().0 += 1;
                }
                Err(msg) if msg.starts_with("SKIP ") => {
                    skipped += 1;
                    *skip_reasons
                        .entry(msg.trim_start_matches("SKIP ").to_string())
                        .or_default() += 1;
                }
                Err(msg) => {
                    executed += 1;
                    failures.push((f.clone(), mode, msg));
                }
            }
        }
        per_dir.entry(dir).or_default().1 += 1;
    }

    eprintln!("\n=== test262 (subset langage, extraction d53ebc8) ===");
    eprintln!(
        "passed {passed} / {executed} passes-mode (fichiers: {}, skips fichiers: {skipped})",
        files.len()
    );
    for (d, (p, n)) in &per_dir {
        eprintln!("  {d}: {p} passes / {n} fichiers");
    }
    for (r, n) in &skip_reasons {
        eprintln!("  skip {r}: {n}");
    }
    eprintln!("failed {}", failures.len());
    for (f, mode, msg) in &failures {
        eprintln!("  {mode:?} {} — {msg}", f.display());
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

// ------------------------------------------------------------------
// Verrous unitaires (mécanique du harness, pas le moteur).
// ------------------------------------------------------------------

#[test]
fn meta_flux_bloc_scalaire() {
    let m = meta(
        "/*---\nflags: [noStrict]\nnegative:\n  phase: runtime\n  type: TypeError\n---*/\nthrow 0;",
    );
    assert_eq!(m.flags, ["noStrict"]);
    assert_eq!(m.negative_phase.as_deref(), Some("runtime"));
    assert_eq!(m.negative_type.as_deref(), Some("TypeError"));
}

#[test]
fn meta_flow_map_negative() {
    let m = meta("/*---\nnegative: {phase: parse, type: SyntaxError}\nflags: [onlyStrict]\n---*/");
    assert_eq!(m.negative_phase.as_deref(), Some("parse"));
    assert_eq!(m.negative_type.as_deref(), Some("SyntaxError"));
    assert_eq!(m.flags, ["onlyStrict"]);
}

#[test]
fn meta_dash_listes() {
    let m = meta("/*---\nfeatures:\n  - Symbol\n  - BigInt\nincludes:\n  - sta.js\n---*/");
    assert_eq!(m.features, ["Symbol", "BigInt"]);
    assert_eq!(m.includes, ["sta.js"]);
}

#[test]
fn modes_selection() {
    let d = meta("/*---\nflags: [module]\n---*/");
    assert_eq!(modes(&d), &[Mode::Module]);
    let s = meta("/*---\nflags: [onlyStrict]\n---*/");
    assert_eq!(modes(&s), &[Mode::Strict]);
    let n = meta("/*---\nflags: [noStrict]\n---*/");
    assert_eq!(modes(&n), &[Mode::Sloppy]);
    let dd = meta("/*---\ninfo: x\n---*/");
    assert_eq!(modes(&dd), &[Mode::Sloppy, Mode::Strict]);
}

#[test]
fn loader_detecte() {
    assert!(needs_loader("import \"x.js\";"));
    assert!(needs_loader("import {a} from \"y.js\";"));
    assert!(needs_loader("export * from \"z.js\";"));
    assert!(needs_loader("let p = import(\"m.js\");"));
    assert!(!needs_loader("import.meta.url;"));
    assert!(!needs_loader("let x = 1; x.imported = 2;"));
}

#[test]
fn negative_type_verrouille() {
    let m = meta("/*---\nnegative: {phase: runtime, type: TypeError}\n---*/");
    let mut e = Engine::new();
    let out = e.eval("throw new TypeError('x');");
    assert!(verdict(&m, out).is_ok(), "TypeError attendu");
    let out = e.eval("throw new RangeError('x');");
    assert!(verdict(&m, out).is_err(), "RangeError ne doit pas passer");
}

#[test]
fn async_done_ok() {
    let m = meta("/*---\nflags: [async]\n---*/");
    let p = Path::new("t.js");
    let r = run_pass(
        p,
        &m,
        "Promise.resolve().then(function(){ $DONE(); });",
        Mode::Sloppy,
    );
    assert!(r.is_ok(), "{r:?}");
}

#[test]
fn async_done_err() {
    let m = meta("/*---\nflags: [async]\n---*/");
    let p = Path::new("t.js");
    let r = run_pass(
        p,
        &m,
        "Promise.resolve().then(function(){ $DONE('boom'); });",
        Mode::Sloppy,
    );
    assert!(r.is_err(), "doit échouer : {r:?}");
}

#[test]
fn async_done_jamais_regle() {
    let m = meta("/*---\nflags: [async]\n---*/");
    let p = Path::new("t.js");
    let r = run_pass(p, &m, "Promise.resolve().then(function(){});", Mode::Sloppy);
    assert!(r.is_err(), "doit échouer : {r:?}");
}

#[test]
fn module_eval() {
    let m = meta("/*---\nflags: [module]\n---*/");
    let p = Path::new("t.js");
    let r = run_pass(
        p,
        &m,
        "export const x = 1; globalThis.__m = x;",
        Mode::Module,
    );
    assert!(r.is_ok(), "{r:?}");
}
