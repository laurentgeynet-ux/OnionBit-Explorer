// This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
// SPDX-License-Identifier: MIT OR Apache-2.0

//! §B-56 (frandroid vol 96) — `{n,m}` sur grosse classe
//! Unicode : `/^[\w:-]{32,512}$/` rejetée « error building
//! NFA » — le backend déroule ~`m` copies d'états et le
//! `delegate_size_limit` par défaut (10 Mio) était dépassé.
//! Borne relevée à 64 Mio (`DELEGATE_SIZE_LIMIT`).
use explorer_jscore::Engine;

#[test]
fn bounded_repeat_large_unicode_class() {
    let mut e = Engine::new();
    // Littéral (le chemin fautif : validation au parse) et
    // constructeur — les deux doivent compiler.
    for src in [
        r"/^[\w:-]{32,512}$/.test('aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa')",
        r"new RegExp('^[\\w:-]{32,512}$').test('aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa')",
        // Grand max — borne toujours finie, pattern réel.
        r"/\w{1,1024}/.test('abc')",
    ] {
        match e.eval(src) {
            Ok(v) => assert_eq!(format!("{v:?}"), "Bool(true)", "{src}"),
            Err(err) => panic!("{src} -> {err}"),
        }
    }
}

/// Les classes `\p{Pat_Syn}`/`\p{Pat_WS}` (observées airfrance
/// vol 95) compilent et matchent — elles passent au backend
/// qui les connaît ; non-régression sur leur traduction.
#[test]
fn deprecated_unicode_props_match() {
    let mut e = Engine::new();
    for (src, want) in [
        (r"/\p{Pat_Syn}/.test('$')", "Bool(true)"),
        (r"/\p{Pat_Syn}/.test('a')", "Bool(false)"),
        (r"/\p{Pat_WS}/.test(' ')", "Bool(true)"),
        (r"/\141\164/.test('at')", "Bool(true)"),
    ] {
        match e.eval(src) {
            Ok(v) => assert_eq!(format!("{v:?}"), want, "{src}"),
            Err(err) => panic!("{src} -> {err}"),
        }
    }
}
