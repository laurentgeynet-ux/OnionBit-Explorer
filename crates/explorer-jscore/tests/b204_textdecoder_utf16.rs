// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! §B-204 — `RangeError: unsupported encoding` sur
//! `new TextDecoder("utf-16le")` (diagrams.net, vol 958) :
//! label WHATWG **supporté** par les navigateurs — le
//! décodeur `UTF16ToString` des runtimes Emscripten/wasm.
//! Verrous : labels canonisés, decode LE/BE, sniff BOM,
//! `ignoreBOM` spec, octet impair, `fatal`, labels encore
//! rejetés.

use explorer_jscore::Engine;

fn ev(js: &str) -> String {
    let mut e = Engine::new();
    match e.eval(js) {
        Ok(v) => e.describe(v),
        Err(err) => panic!("{js} -> {err}"),
    }
}

#[test]
fn utf16_labels_et_decode() {
    // Verbatim Emscripten : `new TextDecoder("utf-16le")`.
    assert_eq!(
        ev("var d=new TextDecoder('utf-16le');d.encoding+'|'+d.decode(new Uint8Array([0x41,0,0x42,0]))"),
        "utf-16le|AB"
    );
    // Alias WHATWG → utf-16le.
    assert_eq!(
        ev("new TextDecoder('utf-16').encoding+'|'+new TextDecoder('UCS-2').encoding"),
        "utf-16le|utf-16le"
    );
    assert_eq!(
        ev("var d=new TextDecoder('utf-16be');d.encoding+'|'+d.decode(new Uint8Array([0,0x41,0,0x42]))"),
        "utf-16be|AB"
    );
    // BOM sniffed : FF FE → LE, FE FF → BE (sur `utf-16le`,
    // un BOM `FE FF` commute l'endianness — spec).
    assert_eq!(
        ev("new TextDecoder('utf-16le').decode(new Uint8Array([0xFF,0xFE,0x41,0,0x42,0]))"),
        "AB"
    );
    assert_eq!(
        ev("new TextDecoder('utf-16le').decode(new Uint8Array([0xFE,0xFF,0,0x41,0,0x42]))"),
        "AB"
    );
    // `ignoreBOM:true` = BOM livré, pas de sniff (spec).
    assert_eq!(
        ev("new TextDecoder('utf-16le',{ignoreBOM:true}).decode(new Uint8Array([0xFF,0xFE,0x41,0])).charCodeAt(0)"),
        "65279"
    );
    // Surrogate isolé livré tel quel (WHATWG — paires
    // disjointes non recombinées, units brutes).
    assert_eq!(
        ev("new TextDecoder('utf-16le').decode(new Uint8Array([0,0xD8,0,0xDC])).length+'|'+new TextDecoder('utf-16le').decode(new Uint8Array([0,0xD8])).charCodeAt(0)"),
        "2|55296"
    );
    // Octet impair final → U+FFFD ; `fatal` → TypeError.
    assert_eq!(
        ev("new TextDecoder('utf-16le').decode(new Uint8Array([0x41,0,0x5A])).length+'|'+new TextDecoder('utf-16le').decode(new Uint8Array([0x41,0,0x5A])).charCodeAt(1)"),
        "2|65533"
    );
    let mut e = Engine::new();
    assert!(e
        .eval("new TextDecoder('utf-16le',{fatal:true}).decode(new Uint8Array([0x41,0,0x5A]))")
        .is_err());
    // Label encore non supporté → RangeError honnête.
    let mut e = Engine::new();
    assert!(e.eval("new TextDecoder('shift_jis')").is_err());
}

#[test]
fn utf8_bom_defaut_retire_ignorebom_conserve() {
    // Spec : BOM UTF-8 retiré par défaut ; `ignoreBOM:true`
    // le conserve (la convention interne était inversée).
    assert_eq!(
        ev("new TextDecoder().decode(new Uint8Array([0xEF,0xBB,0xBF,0x41]))"),
        "A"
    );
    assert_eq!(
        ev("new TextDecoder('utf-8',{ignoreBOM:true}).decode(new Uint8Array([0xEF,0xBB,0xBF,0x41])).charCodeAt(0)"),
        "65279"
    );
}
