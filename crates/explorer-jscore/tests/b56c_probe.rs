// This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
// SPDX-License-Identifier: MIT OR Apache-2.0

//! §B-56 bras 3 (airfrance vol 101 — script anti-bot
//! jscrambler) — octales Annex B : `\N` chiffre EN CLASSE est
//! toujours une octale/identité, jamais une backref
//! (`[\r\100]` rendait `\1(?:00)` → « Invalid character
//! class ») ; hors classe le nombre **entier** tranche
//! (`\141` > groupes → `\x{61}` = 'a' — le « plus long
//! préfixe » matchait la chaîne « 41 »).

use explorer_jscore::Engine;

fn eval(src: &str) -> String {
    let mut eng = Engine::new();
    match eng.eval(src) {
        Ok(v) => format!("{v:?}"),
        Err(e) => format!("ERR {e}"),
    }
}

#[test]
fn octal_in_class_is_char_not_backref() {
    // `[\100]` = '@' — avec ou sans groupes ailleurs.
    assert_eq!(eval(r"/[\100]/.test('@')"), "Bool(true)");
    assert_eq!(eval(r"/(x)[\100]/.test('x@')"), "Bool(true)");
    // `\18` = `\x{1}` puis `8`.
    assert_eq!(eval(r"/[\18]/.test('8')"), "Bool(true)");
    assert_eq!(eval(r"/[\18]/.test('\x01')"), "Bool(true)");
    // Le verbatim jscrambler compile entier (avant : « Parsing
    // error at position 256: Invalid character class »).
    let out = eval(
        r"/(?:^|\141\164[\f\u1680-\u2000\u200a\t\u2029 \ufeff\r\u202f\u00a0\u205f\v\u2028\n\u3000]{1,})(?:([^\n\r\u2028\u2029]{0,}?)[\v\u3000\u00a0\f\u200a\ufeff\u202f\u2028 \u1680-\u2000\n\u2029\u205f\t\r\100]\x28){0,1}([^\u2028\r\n\u2029]{1,}?)\u003a([0-9]{1,})\u003a([0-9]{1,})\051{0,1}$/.test('x')",
    );
    assert_eq!(out, "Bool(false)", "motif jscrambler compilé : {out}");
}

#[test]
fn octal_outside_class_whole_number_first() {
    // `\141` avec des groupes = octale 'a' (pas `\1`+`41`).
    assert_eq!(eval(r"/(x)\141/.test('xa')"), "Bool(true)");
    assert_eq!(eval(r"/(x)\141/.test('x41')"), "Bool(false)");
    // `\12` avec 3 groupes = octale `\x0a`, pas `\1`+`2`.
    assert_eq!(eval(r"/(a)(b)(c)\12/.test('abc\n')"), "Bool(true)");
    // Backref valide inchangée.
    assert_eq!(eval(r"/(a)\1/.test('aa')"), "Bool(true)");
    // Backref « vers l'avant » (groupe non clos) = match vide
    // — Acid3 : `/(\3)(\1)(a)/` sur « cat » matche « a ».
    assert_eq!(eval(r"/(\3)(\1)(a)/.test('a')"), "Bool(true)");
    // `\8`/`\9` identité hors classe.
    assert_eq!(eval(r"/\8/.test('8')"), "Bool(true)");
    // Mode `u` : `\1` hors groupes reste SyntaxError.
    assert_eq!(
        eval(r"try { eval('/\\1/u'); 'no' } catch (e) { e instanceof SyntaxError }"),
        "Bool(true)"
    );
}
