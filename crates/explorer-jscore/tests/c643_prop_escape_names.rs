// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! JS-C643 — `\p{…}` mode `u`/`v` : match **exact** des noms et
//! valeurs (`UnicodeMatchProperty`/`Value`), aucun loose
//! matching — propriétés retirées (`Hyphen`, `Other_*`…),
//! variantes de casse/underscores et `\p` nu → `SyntaxError` ;
//! alias courts du tableau normatif (`Hex`, `AHex`, `CI`…)
//! acceptés ; `sc(x)=Unknown`/`Zzzz` compile (`→ \p{Cn}`).

use explorer_jscore::Engine;

#[track_caller]
fn eval(src: &str) -> String {
    let mut e = Engine::new();
    e.install_t262();
    match e.eval(src) {
        Ok(v) => e.describe(v),
        Err(err) => format!("ERR {err}"),
    }
}

/// Loose matching interdit : casse, underscores, espaces.
#[test]
fn loose_matching_rejected() {
    for bad in [
        r"/\p{Ascii}/u",
        r"/\p{ANY}/u",
        r"/\p{any}/u",
        r"/\p{assigned}/u",
        r"/\p{lowercase}/u",
        r"/\p{gC=uppercase_letter}/u",
        r"/\p{gc=uppercaseletter}/u",
        r"/\p{ General_Category=Uppercase_Letter }/u",
        r"/\p{General_Category = Uppercase_Letter}/u",
        r"/\P{ascii}/u",
    ] {
        assert!(
            eval(&format!("(() => {{ {bad}; return 1 }})()")).starts_with("ERR"),
            "{bad} devrait être un SyntaxError"
        );
    }
}

/// Propriétés retirées de la spec → `SyntaxError`.
#[test]
fn removed_props_rejected() {
    for bad in [
        r"/\p{Hyphen}/u",
        r"/\p{Grapheme_Link}/u",
        r"/\p{Other_Lowercase}/u",
        r"/\p{Other_ID_Start}/u",
        r"/\p{Prepended_Concatenation_Mark}/u",
        r"/\p{IsScript=Adlam}/u",
        r"/\p{ASCII=T}/u",
        r"/\pL/u",
    ] {
        assert!(
            eval(&format!("(() => {{ {bad}; return 1 }})()")).starts_with("ERR"),
            "{bad} devrait être un SyntaxError"
        );
    }
}

/// Noms longs + alias normatifs + `Name=Value` acceptés.
#[test]
fn spec_names_accepted() {
    assert_eq!(eval(r"/\p{Hex}/u.test('F')"), "true");
    assert_eq!(eval(r"/\p{AHex}/u.test('g')"), "false");
    assert_eq!(eval(r"/\p{CI}/u.test('x')"), "false");
    assert_eq!(eval(r"/\p{gc=Lu}/u.test('A')"), "true");
    assert_eq!(eval(r"/\p{General_Category=Lu}/u.test('A')"), "true");
    assert_eq!(eval(r"/\p{Script=Latin}/u.test('a')"), "true");
    assert_eq!(eval(r"/\p{scx=Adlam}/u.test('x')"), "false");
    assert_eq!(eval(r"/\p{LC}/u.test('a')"), "true");
}

/// `Script_Extensions`/`sc` `Unknown`/`Zzzz` compilent.
#[test]
fn script_extensions_unknown() {
    for src in [
        r"/\p{Script_Extensions=Unknown}/u.test('x')",
        r"/\p{Script_Extensions=Zzzz}/u.test('x')",
        r"/\p{scx=Unknown}/u.test('x')",
        r"/\p{scx=Zzzz}/u.test('x')",
        r"/\p{sc=Unknown}/u.test('x')",
    ] {
        assert_eq!(eval(src), "false", "{src}");
    }
}

/// `CWKCF` compile (sous-approximation `CW*`, cataloguée C641).
#[test]
fn cwkcf_compiles() {
    assert_eq!(eval(r"/\p{CWKCF}/u.test('x')"), "false");
    assert_eq!(
        eval(r"/\p{Changes_When_NFKC_Casefolded}/u.test('A')"),
        "true"
    );
}
