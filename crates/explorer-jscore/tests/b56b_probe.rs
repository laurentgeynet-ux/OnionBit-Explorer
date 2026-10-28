// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet.
// SPDX-License-Identifier: MIT OR Apache-2.0

//! §B-56 bras 2 (airfrance vol 98) — `\p`/`\P` sloppy en
//! classe = identité littérale Annex B, pas une propriété :
//! `[^\p{Pat_Syn}\p{Pat_WS}]` est le set négatif des lettres
//! `{p,{,P,a,t,_,S,y,n,W,}}` (Chrome). Avant : `\p` nu émis,
//! `{Nom}` traduit en littéraux échappés → backend rejetait
//! `\p\x{7b}…` (« error parsing pattern »).

use explorer_jscore::Engine;

fn eval(src: &str) -> String {
    let mut eng = Engine::new();
    match eng.eval(src) {
        Ok(v) => format!("{v:?}"),
        Err(e) => format!("ERR {e}"),
    }
}

#[test]
fn sloppy_p_in_class_is_identity_literal() {
    // Compile (le vrai bug — SyntaxError avant).
    let out = eval(r"var r=/(?:(?:\{\s*[^\p{Pat_Syn}\p{Pat_WS}]+\s*))/; 'ok'");
    assert!(!out.starts_with("ERR"), "pattern airfrance compilé : {out}");
    // Sémantique set littéral : 'q' matche la niée, 'p' non.
    assert_eq!(
        eval(r"/[^\p{Pat_Syn}]/.test('q')"),
        "Bool(true)",
        "'q' ∈ niée"
    );
    assert_eq!(
        eval(r"/[^\p{Pat_Syn}]/.test('p')"),
        "Bool(false)",
        "'p' ∉ niée (littérale)"
    );
    // Range : `[\p-z]` sloppy = range 'p'-'z' (Chrome).
    assert_eq!(
        eval(r"/[\p-z]/.test('t')"),
        "Bool(true)",
        "range p-z : 't' ∈"
    );
    assert_eq!(
        eval(r"/[\p-z]/.test('o')"),
        "Bool(false)",
        "range p-z : 'o' ∉"
    );
    // `\p` seul en classe : littéral pareil.
    assert_eq!(eval(r"/[\p]/.test('p')"), "Bool(true)", r"[\p] = {{p}}");
    // Octal en contexte (bras rapporté comme rejeté — même
    // classe de cause : `{1,}` + classe) : compile et matche.
    assert_eq!(
        eval(r"/\141\164[a-z]{1,}/.test('atx')"),
        "Bool(true)",
        "octal en contexte"
    );
}

#[test]
fn u_mode_p_in_class_stays_unicode_property() {
    // En `u`, `\p{…}` en classe reste une vraie propriété.
    assert_eq!(
        eval(r"/[\p{Lu}]/u.test('A')"),
        "Bool(true)",
        r"`u` : [\p{{Lu}}] matche 'A'"
    );
    assert_eq!(
        eval(r"/[\p{Lu}]/u.test('a')"),
        "Bool(false)",
        r"`u` : [\p{{Lu}}] ne matche pas 'a'"
    );
}
