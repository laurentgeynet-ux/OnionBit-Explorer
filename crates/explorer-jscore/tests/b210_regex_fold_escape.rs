// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! §B-210 — `SyntaxError: invalid regex …: Parsing error at position 593:
//! Invalid character class` (fr24, sonde release Étape 331,
//! `app-CYaLwPhd.js`) : la regex `extractLocation` d'error-stack-parser
//! contient `\[native` — sous le flag `i` (pliage de casse manuel, chemin
//! non-`u`), `push_folded` émettait les caractères littéraux **bruts** :
//! `[` nu ouvrait une classe backend invalide. Le même trou rendait
//! `\.`/`\(`/`\|`/`\*`/`\?`/`\{`… des métacaractères au lieu de littéraux
//! sous `i` (sémantique fausse silencieuse). Fix : émission via
//! `class_char` (échappement `\x{…}` du backend) sur la branche sans
//! variante de casse. Verrous : verbatim fr24 compile sous `i` + match
//! identique à la sémantique attendue, `\.` littéral sous `i`, cas
//! `u`/`m`/`s`/`g` inchangés, classes source préservées.

use explorer_jscore::Engine;

fn ev(js: &str) -> String {
    let mut e = Engine::new();
    match e.eval(js) {
        Ok(v) => e.describe(v),
        Err(err) => panic!("{js} -> {err}"),
    }
}

/// Source verbatim de la regex `extractLocation` d'error-stack-parser
/// embarquée par fr24 (160 chars).
const FR24_SRC: &str = "^\\s*(.*?)(?:\\((.*?)\\))?(?:(?:(?:^|@)((?:file|https?|blob|chrome|webpack|resource|capacitor|\\[native).*?|[^@]*bundle|\\[wasm code\\])(?::(\\d+))?(?::(\\d+))?)|@)\\s*$";

#[test]
fn regex_i_fold_metachars_echappes() {
    // Le verbatim fr24 compile sous tous les flags — `i` était la seule
    // voie fautive (« Invalid character class » @593 sur le pattern
    // traduit).
    assert_eq!(
        ev(&format!(
            "var s={FR24_SRC:?}; ['','i','u','m','s','g'].map(f=>{{try{{new RegExp(s,f);return 1}}catch(e){{return 0}}}}).join('')"
        )),
        "111111"
    );
    // Match réel : cadres de pile aux formats Firefox/Chrome/webkit.
    assert_eq!(
        ev(&format!(
            "var rx=new RegExp({FR24_SRC:?},'i');\
             [\"foo@http://x/app.js:11:22\",\"[native code]\",\"webpack://./src/a.js:7:8\"]\
             .map(s=>{{var m=rx.exec(s);return m?m[3]+':'+m[4]+':'+m[5]:'null'}}).join('|')"
        )),
        "http://x/app.js:11:22|[native code]:undefined:undefined|webpack://./src/a.js:7:8"
    );
}

#[test]
fn regex_i_litteraux_echappes_conservent_sens() {
    // `\.` sous `i` : point littéral, PAS « n'importe quel char » —
    // l'émission brute rendait `.` métacaractère (match tout).
    assert_eq!(
        ev("var r=/\\./i; r.test('a.b')+'|'+r.test('ab')"),
        "true|false"
    );
    // `\[` sous `i` : crochet littéral + pliage sur les lettres.
    assert_eq!(
        ev("var r=/\\[native/i; r.test('[NATIVE]')+'|'+r.test('xnad')"),
        "true|false"
    );
    // Autres métacaractères échappés sous `i` : `\(`, `\*`, `\+`, `\?`.
    assert_eq!(
        ev("var r=/a\\(b\\*c\\+d\\?e\\)/i; r.test('A(B*C+D?E)')+'|'+r.test('abcdefg')"),
        "true|false"
    );
    // Classes source et intervalles sous `i` inchangés.
    assert_eq!(
        ev("var r=/[a-z0-9\\-\\.]+\\.js$/i; r.test('APP-1.JS')+'|'+r.test('a/js')"),
        "true|false"
    );
}
