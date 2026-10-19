// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Sniffing d'encodage (BOM, `<meta charset>`, défaut
//! windows-1252) et transcodage tolérant — étape 10.

use explorer_html::encoding::{
    decode, decode_sniffed, decode_windows1252, sniff_encoding, Encoding,
};

#[test]
fn bom_utf8() {
    let bytes = b"\xEF\xBB\xBF<p>ok</p>";
    assert_eq!(sniff_encoding(bytes), Encoding::Utf8);
    let (text, enc) = decode_sniffed(bytes);
    assert_eq!(enc, Encoding::Utf8);
    assert_eq!(text, "<p>ok</p>"); // BOM consommé
}

#[test]
fn bom_utf16() {
    assert_eq!(sniff_encoding(&[0xFF, 0xFE, b'a', 0]), Encoding::Utf16Le);
    assert_eq!(sniff_encoding(&[0xFE, 0xFF, 0, b'a']), Encoding::Utf16Be);
    let (text, _) = decode_sniffed(&[0xFF, 0xFE, b'a', 0, b'b', 0]);
    assert_eq!(text, "ab");
}

#[test]
fn meta_charset_simple() {
    let html = br#"<html><head><meta charset="utf-8"></head>"#;
    assert_eq!(sniff_encoding(html), Encoding::Utf8);
}

#[test]
fn meta_charset_variantes() {
    // Sans quotes, case-insensitive, dans content=…
    assert_eq!(sniff_encoding(b"<meta charset=UTF-8>"), Encoding::Utf8);
    assert_eq!(sniff_encoding(b"<meta CHARSET='utf-8'>"), Encoding::Utf8);
    assert_eq!(
        sniff_encoding(br#"<meta http-equiv="Content-Type" content="text/html; charset=utf-8">"#),
        Encoding::Utf8
    );
    // Label inconnu → windows-1252 (repli WHATWG).
    assert_eq!(
        sniff_encoding(b"<meta charset='x-bidule'>"),
        Encoding::Windows1252
    );
}

#[test]
fn meta_hors_fenetre_ou_hors_balise() {
    // `charset` hors d'une balise ouverte → ignoré.
    assert_eq!(sniff_encoding(b"charset=utf-8"), Encoding::Windows1252);
    // Au-delà de la fenêtre de sniffing → ignoré.
    let mut gros = vec![b' '; 2048];
    gros.extend_from_slice(b"<meta charset=utf-8>");
    assert_eq!(sniff_encoding(&gros), Encoding::Windows1252);
}

#[test]
fn defaut_windows1252() {
    assert_eq!(sniff_encoding(b"<p>ascii</p>"), Encoding::Windows1252);
}

#[test]
fn windows1252_table() {
    // 0x93/0x94 = guillemets typographiques, pas des C1.
    assert_eq!(decode_windows1252(&[0x93, 0x94]), "“”");
    assert_eq!(decode_windows1252(&[0xE9]), "é");
    let (text, _) = decode_sniffed(&[0x93]);
    assert_eq!(text, "“");
}

#[test]
fn utf8_malforme_tolerant() {
    // Séquence invalide → U+FFFD, jamais de panic.
    let out = decode(&[b'a', 0xFF, 0xFE, b'b'], Encoding::Utf8);
    assert_eq!(out, "a\u{FFFD}\u{FFFD}b");
}

#[test]
fn utf16_longueur_impaire_tolerant() {
    // Octet orphelin en fin : ignoré sans crash.
    let out = decode(&[b'a', 0, b'b'], Encoding::Utf16Le);
    assert_eq!(out, "a");
}
