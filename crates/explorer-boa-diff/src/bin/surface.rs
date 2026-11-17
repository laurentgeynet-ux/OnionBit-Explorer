// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! `boa-surface` — matrice de surface : exécute `surface.js`
//! (énumération `getOwnPropertyNames`/`Symbols`/`getOwnPropertyDescriptor`
//! sur globalThis → ctors → prototypes) dans jscore puis boa,
//! compare les listes de chemins et les flags de descripteurs.
//! Sortie : `N boa-only | M jscore-only | K flagdiff`.

use std::collections::{BTreeMap, BTreeSet};
use std::process::exit;

use explorer_boa_diff::Outcome;

const SURFACE: &str = include_str!("../../surface.js");

fn enumerate(f: fn(&str) -> Outcome, name: &str) -> BTreeMap<String, String> {
    match f(SURFACE) {
        Outcome::Value(s) => parse(&s),
        other => {
            eprintln!("{name}: énumération impossible: {other:?}");
            exit(2);
        }
    }
}

/// Le script rend `path wec|kind` par ligne, joint par `\n` dans
/// la valeur de complétion.
fn parse(s: &str) -> BTreeMap<String, String> {
    s.lines()
        .filter_map(|l| l.rsplit_once(' '))
        .map(|(p, f)| (p.to_string(), f.to_string()))
        .collect()
}

fn main() {
    let jscore = enumerate(explorer_boa_diff::eval_jscore, "jscore");
    let boa = enumerate(explorer_boa_diff::eval_boa, "boa");
    let jk: BTreeSet<_> = jscore.keys().collect();
    let bk: BTreeSet<_> = boa.keys().collect();
    let boa_only: Vec<_> = bk.difference(&jk).collect();
    let js_only: Vec<_> = jk.difference(&bk).collect();
    let flagdiff: Vec<_> = jk
        .intersection(&bk)
        .filter(|p| jscore[**p] != boa[**p])
        .collect();
    println!("jscore: {} chemins | boa: {}", jscore.len(), boa.len());
    println!(
        "{} boa-only | {} jscore-only | {} flagdiff",
        boa_only.len(),
        js_only.len(),
        flagdiff.len()
    );
    for p in &boa_only {
        println!("boa-only   {p} ={}", boa[p.as_str()]);
    }
    for p in &flagdiff {
        println!(
            "flagdiff   {p} jscore={} boa={}",
            jscore[p.as_str()],
            boa[p.as_str()]
        );
    }
    for p in js_only.iter().take(60) {
        println!("js-only    {p}");
    }
    if js_only.len() > 60 {
        println!("… +{} js-only", js_only.len() - 60);
    }
}
