// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Verrous de calibrage de l'oracle : chaque moteur répond la
//! forme canonique attendue, et `concordant` discrimine.

use explorer_boa_diff::{concordant, eval_boa, eval_jscore, Outcome};

include!("../src/probes.rs");

#[test]
fn jscore_value() {
    assert_eq!(eval_jscore("1 + 1"), Outcome::Value("2".into()));
    assert_eq!(eval_jscore("typeof 0"), Outcome::Value("number".into()));
}

#[test]
fn boa_value() {
    assert_eq!(eval_boa("1 + 1"), Outcome::Value("2".into()));
    assert_eq!(eval_boa("typeof 0"), Outcome::Value("number".into()));
}

#[test]
fn jscore_throw_class() {
    let o = eval_jscore("null.x");
    assert_eq!(o.thrown_name().as_deref(), Some("TypeError"));
}

#[test]
fn boa_throw_class() {
    let o = eval_boa("null.x");
    assert_eq!(o.thrown_name().as_deref(), Some("TypeError"));
}

#[test]
fn syntax_both() {
    assert!(matches!(eval_jscore("1+"), Outcome::Syntax(_)));
    // boa rapporte l'échec de parse via un SyntaxError → canon Syntax.
    assert!(matches!(
        eval_boa("1+"),
        Outcome::Syntax(_) | Outcome::Throw(_)
    ));
}

#[test]
fn concordance() {
    assert!(concordant(
        &Outcome::Value("2".into()),
        &Outcome::Value("2".into())
    ));
    // Classe égale, messages différents : concordant (message non spec).
    assert!(concordant(
        &Outcome::Throw("TypeError: a".into()),
        &Outcome::Throw("TypeError: b".into())
    ));
    assert!(!concordant(
        &Outcome::Throw("TypeError: a".into()),
        &Outcome::Throw("RangeError: a".into())
    ));
    assert!(!concordant(
        &Outcome::Value("1".into()),
        &Outcome::Value("2".into())
    ));
    assert!(!concordant(
        &Outcome::Syntax("x".into()),
        &Outcome::Value("x".into())
    ));
}

/// Les noms de sondes sont la clé du lookup `--worker` : un
/// doublon fait exécuter deux fois la même source et masque la
/// seconde dans le binaire (observé : `json_indent` ×2 — la
/// variante multi-lignes n'était jamais exécutée).
#[test]
fn probe_names_unique() {
    let mut seen = std::collections::BTreeSet::new();
    for (name, _) in PROBES {
        assert!(seen.insert(name), "sonde en double : {name}");
    }
}
