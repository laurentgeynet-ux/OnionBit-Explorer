// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! `boa-diff` — exécute le corpus de sondes dans jscore, boa **et**
//! V8 (Node.js, sous-processus `runner.js`), canonise les issues et
//! rapporte les divergences — trois moteurs, vote majoritaire pour
//! l'arbitrage. Chaque éval tourne dans un processus enfant
//! (`--worker <nom>`) : un crash natif de l'un des moteurs devient
//! `Internal(crash)` au lieu de tuer le runner — observé sur boa
//! 0.22-dev (récursion native sur `for-of`/`break` en fonction).

use std::env;
use std::process::{Command, Stdio};

use explorer_boa_diff::{concordant, eval_boa, eval_jscore, eval_v8, v8_available, Outcome};

include!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../explorer-jscore/tests/oracle_corpus.rs"
));

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.first().is_some_and(|a| a == "--worker") {
        let name = args.get(1).map_or("", String::as_str);
        worker(name);
        return;
    }
    if args.first().is_some_and(|a| a == "--src") {
        let src = args.get(1).map_or("", String::as_str);
        println!("JSCORE\t{}", wire(&eval_jscore(src)));
        println!("V8\t{}", wire(&eval_v8(src)));
        println!("BOA\t{}", wire(&eval_boa(src)));
        return;
    }
    parent();
}

/// Enfant : évalue la sonde `name` dans les trois moteurs, affiche
/// chaque issue `ENGINE\tkind\ttexte` — protocole ligne lu par le
/// parent. Un seul sonde : si le processus meurt, la perte est
/// confinée à cette sonde. V8 avant boa : un crash natif de boa
/// (observé) ne doit pas masquer la voix de l'arbitre.
fn worker(name: &str) {
    let Some((_, src)) = PROBES.iter().find(|(n, _)| *n == name) else {
        eprintln!("sonde inconnue: {name}");
        std::process::exit(2);
    };
    let j = eval_jscore(src);
    println!("JSCORE\t{}", wire(&j));
    let v = if v8_available() {
        eval_v8(src)
    } else {
        Outcome::Internal("v8 absent".to_string())
    };
    println!("V8\t{}", wire(&v));
    let b = eval_boa(src);
    println!("BOA\t{}", wire(&b));
}

fn wire(o: &Outcome) -> String {
    match o {
        Outcome::Value(s) => format!("Value\t{s}"),
        Outcome::Throw(s) => format!("Throw\t{s}"),
        Outcome::Syntax(s) => format!("Syntax\t{s}"),
        Outcome::Internal(s) => format!("Internal\t{s}"),
    }
}

fn unwire(line: &str) -> Option<Outcome> {
    let (kind, rest) = line.split_once('\t')?;
    Some(match kind {
        "Value" => Outcome::Value(rest.to_string()),
        "Throw" => Outcome::Throw(rest.to_string()),
        "Syntax" => Outcome::Syntax(rest.to_string()),
        _ => Outcome::Internal(rest.to_string()),
    })
}

/// Parent : lance un enfant par sonde, compare les trois issues —
/// une divergence sur **n'importe quelle paire** est rapportée
/// (jscore est la voix mesurée ; boa et V8 sont les deux arbitres).
fn parent() {
    let exe = env::current_exe().unwrap_or_else(|_| "boa-diff".into());
    let (mut ok, mut div, mut internal) = (0u32, 0u32, 0u32);
    for (name, _) in PROBES {
        let out = Command::new(&exe)
            .args(["--worker", name])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output();
        let (j, v, b) = match out {
            Ok(o) if o.status.success() => parse_worker(&o.stdout),
            _ => (None, None, None),
        };
        let (j, v, b) = (
            j.unwrap_or_else(|| Outcome::Internal("crash".to_string())),
            v.unwrap_or_else(|| Outcome::Internal("crash".to_string())),
            b.unwrap_or_else(|| Outcome::Internal("crash".to_string())),
        );
        if concordant(&j, &b) && concordant(&j, &v) {
            if matches!(j, Outcome::Internal(_)) {
                internal += 1;
                println!("{name:<18} INTERNAL  {}", short(&j));
            } else {
                ok += 1;
            }
        } else {
            div += 1;
            println!("{name:<18} DIVERGE  jscore: {}", short(&j));
            println!("{name:<18}          v8    : {}", short(&v));
            println!("{name:<18}          boa   : {}", short(&b));
        }
    }
    println!("---\n{ok} concordants | {div} divergences | {internal} internal");
}

fn parse_worker(stdout: &[u8]) -> (Option<Outcome>, Option<Outcome>, Option<Outcome>) {
    let text = String::from_utf8_lossy(stdout);
    let (mut j, mut v, mut b) = (None, None, None);
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("JSCORE\t") {
            j = unwire(rest);
        } else if let Some(rest) = line.strip_prefix("V8\t") {
            v = unwire(rest);
        } else if let Some(rest) = line.strip_prefix("BOA\t") {
            b = unwire(rest);
        }
    }
    (j, v, b)
}

fn short(o: &Outcome) -> String {
    let s = match o {
        Outcome::Value(s) => format!("Value({s})"),
        Outcome::Throw(s) => format!("Throw({s})"),
        Outcome::Syntax(s) => format!("Syntax({s})"),
        Outcome::Internal(s) => format!("Internal({s})"),
    };
    s.chars().take(120).collect()
}
