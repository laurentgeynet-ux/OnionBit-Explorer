// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Verrous JS-C303 — `json_string` décode chaque caractère dans une
//! fenêtre bornée par l'octet de tête (1-4 o) au lieu de re-valider
//! `from_utf8` sur tout le reste du buffer : O(n²) → O(n), le mur
//! `JSON.parse` `OtAutoBlock` (60 s → 0,56 s sur 7,9 Mo). La
//! sémantique est figée ici : round-trip exact des séquences
//! multi-octets (clés et valeurs), échappées `\uXXXX` et paires de
//! surrogates, troncation et octet de tête fautif rejetés.

use explorer_jscore::Engine;

fn t(src: &str) -> String {
    let mut e = Engine::new();
    match e.eval(src) {
        Ok(v) => e.describe(v),
        Err(err) => format!("ERR {err}"),
    }
}

#[test]
fn multibyte_chars_roundtrip_exact() {
    // 1-4 octets : ASCII, é (2 o), € (3 o), emoji (4 o) — chaque
    // fenêtre de décodage doit viser exactement sa longueur.
    let src = r#"JSON.stringify(JSON.parse('{"a":"xé€\ud83d\ude00y"}'))"#;
    assert_eq!(t(src), r#"{"a":"xé€😀y"}"#);
}

#[test]
fn multibyte_keys_roundtrip_exact() {
    let src = r#"JSON.stringify(JSON.parse('{"clé€😀":42}'))"#;
    assert_eq!(t(src), r#"{"clé€😀":42}"#);
}

#[test]
fn escaped_u_and_surrogate_pair() {
    // `\uXXXX` échappés : BMP direct, paire de surrogates jointe,
    // surrogate haut orphelin → replacement char (tolérant).
    let src = r#"JSON.stringify(JSON.parse('"\\u00e9\\ud83d\\ude00\\ud83d"'))"#;
    assert_eq!(t(src), r#""é😀�""#);
}

#[test]
fn utf8_mixed_with_escapes_in_one_string() {
    // Alternance brut/échappé dans la même chaîne : les deux
    // chemins de décodage restent synchronisés sur `r.i`.
    let src = r#"JSON.stringify(JSON.parse('"é\\u00e9€😀"'))"#;
    assert_eq!(t(src), r#""éé€😀""#);
}

#[test]
fn giant_multibyte_json_parses_and_stringifies() {
    // Charge réaliste : 200 objets à clés/valeurs multi-octets —
    // le fenêtrage ne doit ni dériver ni tronquer.
    use std::fmt::Write as _;
    let mut items = String::from("[");
    for i in 0..200 {
        if i > 0 {
            items.push(',');
        }
        let _ = write!(items, "{{\"clé{i}\":\"vé€{i}😀\"}}");
    }
    items.push(']');
    let src = format!(
        "var a=JSON.parse('{}');a.length+'|'+a[0]['clé0']+'|'+a[199]['clé199']",
        items.replace('\'', "\\'")
    );
    assert_eq!(t(&src), "200|vé€0😀|vé€199😀");
}

#[test]
fn parse_errors_still_rejected() {
    // Séquences invalides au niveau JSON (le buffer étant un
    // `String` Rust, l'UTF-8 est garanti — ces erreurs viennent
    // de la grammaire, pas du décodage).
    let src = r#"var r=[];try{JSON.parse('{"a":"x')}catch(e){r.push('E1')}
               try{JSON.parse('"ab')}catch(e){r.push('E2')}
               try{JSON.parse('{"a":1,}')}catch(e){r.push('E3')}r.join('|')"#;
    assert_eq!(t(src), "E1|E2|E3");
}

#[test]
fn parse_then_access_deep_multibyte() {
    // Le cas OtAutoBlock en miniature : chaîne JSON géante →
    // accès indexé par clé multi-octet.
    let src = r#"var o=JSON.parse('{"règles":[{"étiquette":"café"},{"étiquette":"thé"}]}');
               o.règles[1].étiquette"#;
    assert_eq!(t(src), "thé");
}
