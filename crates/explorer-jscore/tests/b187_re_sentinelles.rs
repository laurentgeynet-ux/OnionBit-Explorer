// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! §B-187 — sonde : classes Unicode multi-plages verbatim des
//! vols 949-951 (doordash/zendesk/fedex). Les verbatims rendus
//! `\U000f0000-\U000f07ff` sont le domaine sentinelle affiché
//! brut = plages de **code units surrogates** `\ud800-\udfff`.

use explorer_jscore::Engine;

fn eval(src: &str) -> String {
    let mut e = Engine::new();
    e.install_t262();
    match e.eval(src) {
        Ok(v) => e.describe(v),
        Err(err) => format!("ERR {err}"),
    }
}

/// Classe Next.js grapheme **verbatim** (chunk 76805) :
/// plage de surrogates `\ud800-\udfff` en non-`u`.
#[test]
fn next_grapheme_class_surrogate_range() {
    let out = eval(
        "try { var r = RegExp('[\\\\u200d\\\\ud800-\\\\udfff\\\\u0300-\\\\u036f\\\\ufe20-\\\\ufe2f\\\\u20d0-\\\\u20ff\\\\ufe0e\\\\ufe0f]');\
             r.test('\\ud83d\\ude00') + ':' + r.test('a') } catch(e) { 'ERR '+e.message }",
    );
    println!("next_class = {out}");
    assert_eq!(out, "true:false");
}

/// Classe gmaps bidi/combining (fedex) — verbatim :
/// plages BMP + atomes surrogates `\ud802-\ud803` etc.
#[test]
fn gmaps_combining_class() {
    let out = eval(
        "try { var r = RegExp('[\\\\u0591-\\\\u06af\\\\u06ba-\\\\u08ff\\\\u200f\\\\ud802-\\\\ud803\\\\ud83a-\\\\ud83b\\\\ufb1d-\\\\ufdff\\\\ufe70-\\\\ufefc]');\
             r.test('\\u05d0') + ':' + r.test('x') } catch(e) { 'ERR '+e.message }",
    );
    println!("gmaps = {out}");
    assert_eq!(out, "true:false");
}

/// Lookahead grapheme doordash : `\ud83c[\udffb-\udfff]` —
/// littéral surrogate isolé + classe de surrogates +
/// lookahead.
#[test]
fn grapheme_lookahead_surrogates() {
    let out = eval(
        "try { var r = RegExp('\\\\ud83c[\\\\udffb-\\\\udfff](?=\\\\ud83c[\\\\udffb-\\\\udfff])');\
             r.test('\\ud83c\\udffc\\ud83c\\udffd') } catch(e) { 'ERR '+e.message }",
    );
    println!("lookahead = {out}");
    assert_eq!(out, "true");
}

/// Plage de surrogates complète `\ud800-\udfff` seule en
/// classe — cas minimal.
#[test]
fn bare_surrogate_range_class() {
    let out = eval(
        "try { var r = RegExp('[\\\\ud800-\\\\udfff]');\
             r.test('\\ud800') + ':' + r.test('z') } catch(e) { 'ERR '+e.message }",
    );
    println!("bare = {out}");
    assert_eq!(out, "true:false");
}

/// §B-187 — grapheme splitter lodash **assemblé** (chunk
/// doordash 31887 verbatim) : `U = RegExp(E+"(?="+E+")|"+L+C,"g")`.
#[test]
fn lodash_words_assembled() {
    let out = eval(
        r#"var l="\\ud800-\\udfff",u="\\u0300-\\u036f\\ufe20-\\ufe23",c="\\u20d0-\\u20f0",
            d="\\u2700-\\u27bf",f="a-z\\xdf-\\xf6\\xf8-\\xff",p="A-Z\\xc0-\\xd6\\xd8-\\xde",
            g="\\ufe0e\\ufe0f",h="\\xac\\xb1\\xd7\\xf7\\x00-\\x2f\\x3a-\\x40\\x5b-\\x60\\x7b-\\xbf\\u2000-\\u206f \\t\\x0b\\f\\xa0\\ufeff\\n\\r\\u2028\\u2029\\u1680\\u180e\\u2000\\u2001\\u2002\\u2003\\u2004\\u2005\\u2006\\u2007\\u2008\\u2009\\u200a\\u202f\\u205f\\u3000",
            v="['’]",m="["+u+c+"]",b="["+f+"]",x="[^"+l+h+"\\d+"+d+f+p+"]",
            E="\\ud83c[\\udffb-\\udfff]",_="[^"+l+"]",S="(?:\\ud83c[\\udde6-\\uddff]){2}",
            O="[\\ud800-\\udbff][\\udc00-\\udfff]",A="\\\\u200d",
            I="(?:"+m+"|"+E+")?",P="["+g+"]?",
            D="(?:"+A+"(?:"+[_,S,O].join("|")+")"+P+I+")*",
            C=P+I+D,L="(?:"+[_+m+"?",m,S,O,"["+l+"]"].join("|")+")";
        try { var U = RegExp(E+"(?="+E+")|"+L+C,"g"); 'ok:'+U.test('\ud83d\ude00') }
        catch(e) { 'ERR '+e.message }"#,
    );
    println!("lodash_U = {out}");
    assert_eq!(out, "ok:true");
}

/// Sonde : la string `"\\ud800"` doit faire 6 units (texte
/// `\ud800`) — si le lexer en produisait le unit 0xD800, la
/// source `RegExp` porterait des sentinelle bruts.
#[test]
fn escape_double_backslash_u_len() {
    let out = eval(r"var s='\\ud800'; s.length + ':' + s");
    println!("len = {out}");
    assert_eq!(out, "6:\\ud800");
}

/// Sous-classes adjacentes du ticket — propriété `\p{Cs}`
/// (surrogates, claude) et classe `\uXXXX` (kwcdn/temu) :
/// le chemin diffère (property escape, pas sentinelle).
#[test]
fn adjacent_subclasses() {
    let out = eval(
        "var r = '';\
         try { r += 'cs:' + new RegExp('\\\\p{Cs}','u').test('\\ud800'); } catch(e) { r += 'cs:ERR'; }\
         try { r += ' pcs:' + new RegExp('\\\\p{Cs}','u').test('\\u{1f600}'); } catch(e) { r += ' pcs:ERR'; }\
         try { r += ' neg:' + new RegExp('\\\\P{Cs}','u').test('a'); } catch(e) { r += ' neg:ERR'; }\
         try { r += ' u4:' + /[\\u4e00-\\u9fff]/.test('\\u4e2d'); } catch(e) { r += ' u4:ERR'; }\
         try { r += ' q:' + new RegExp('x{2562}').test('x'.repeat(2562)); } catch(e) { r += ' q:ERR'; }\
         r",
    );
    println!("adj = {out}");
    assert_eq!(out, "cs:true pcs:false neg:true u4:true q:true");
}

/// §B-187 — **vrai chemin verbatim** : le fichier doordash a
/// `\ud800` en **simple backslash** → units surrogates réels
/// dans la chaîne → `re_encode` → sentinelle bruts dans le
/// pattern traduit.
#[test]
fn real_surrogate_units_in_class() {
    let out = eval(
        "try { var r = RegExp('[\\\\u200d\\ud800-\\udfff\\\\u0300-\\\\u036f\\\\ufe20-\\\\ufe2f\\\\u20d0-\\\\u20ff\\\\ufe0e\\\\ufe0f]');\
             r.test('\\ud83d\\ude00') + ':' + r.test('a') } catch(e) { 'ERR '+e.message }",
    );
    println!("real_units = {out}");
    assert_eq!(out, "true:false");
}
