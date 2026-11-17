// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! `boa-diff` — exécute le corpus de sondes dans jscore **et** boa,
//! canonise les issues et rapporte les divergences. Chaque éval
//! tourne dans un processus enfant (`--worker <nom>`) : un crash
//! natif de l'un des moteurs devient `Internal(crash)` au lieu de
//! tuer le runner — observé sur boa 0.22-dev (récursion native
//! sur `for-of`/`break` en fonction).

use std::env;
use std::process::{Command, Stdio};

use explorer_boa_diff::{concordant, eval_boa, eval_jscore, Outcome};

include!("probes.rs");

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
        println!("BOA\t{}", wire(&eval_boa(src)));
        return;
    }
    parent();
}

/// Enfant : évalue la sonde `name` dans les deux moteurs, affiche
/// chaque issue `ENGINE\tkind\ttexte` — protocole ligne lu par le
/// parent. Un seul sonde : si le processus meurt, la perte est
/// confinée à cette sonde.
fn worker(name: &str) {
    let Some((_, src)) = PROBES.iter().find(|(n, _)| *n == name) else {
        eprintln!("sonde inconnue: {name}");
        std::process::exit(2);
    };
    let j = eval_jscore(src);
    println!("JSCORE\t{}", wire(&j));
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

/// Parent : lance un enfant par sonde, compare les deux issues.
fn parent() {
    let exe = env::current_exe().unwrap_or_else(|_| "boa-diff".into());
    let (mut ok, mut div, mut internal) = (0u32, 0u32, 0u32);
    for (name, _) in PROBES {
        let out = Command::new(&exe)
            .args(["--worker", name])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output();
        let (j, b) = match out {
            Ok(o) if o.status.success() => parse_worker(&o.stdout),
            _ => (None, None),
        };
        let (j, b) = (
            j.unwrap_or_else(|| Outcome::Internal("crash".to_string())),
            b.unwrap_or_else(|| Outcome::Internal("crash".to_string())),
        );
        if concordant(&j, &b) {
            if matches!(j, Outcome::Internal(_)) {
                internal += 1;
                println!("{name:<18} INTERNAL  {}", short(&j));
            } else {
                ok += 1;
            }
        } else {
            div += 1;
            println!("{name:<18} DIVERGE  jscore: {}", short(&j));
            println!("{name:<18}          boa   : {}", short(&b));
        }
    }
    println!("---\n{ok} concordants | {div} divergences | {internal} internal");
}

fn parse_worker(stdout: &[u8]) -> (Option<Outcome>, Option<Outcome>) {
    let text = String::from_utf8_lossy(stdout);
    let (mut j, mut b) = (None, None);
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("JSCORE\t") {
            j = unwire(rest);
        } else if let Some(rest) = line.strip_prefix("BOA\t") {
            b = unwire(rest);
        }
    }
    (j, b)
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
