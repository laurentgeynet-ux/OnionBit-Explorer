// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! JS-C630 — modificateurs inline `(?ims-ims:…)` ES2025 :
//!
//! - **Portée `s`** : `(?s:…)`/`-s` basculent l'état effectif
//!   `dotall` — `.` est émis en classes littérales, le flag
//!   backend serait sans effet ; restauration au `)` via
//!   `mod_stack` (parallèle à `open_stack`).
//! - **Portée `i` non-`u`** : bascule le folding manuel
//!   `fold` (simple-upper — `i` fancy folderait Unicode à
//!   tort) ; backrefs enveloppées `(?i:…)` (comparaison
//!   `Canonicalize` — exact ASCII, superset rare).
//! - **Portée `i` `u`/`v`** : `i` émis natif (folding
//!   Unicode correct = spec).
//! - **Portée `m`** : toujours émise native `(?m:`/`(?-m:`,
//!   cohérent avec le flag global.
//! - **Validation** : `i`/`m`/`s` seuls, pas de doublon,
//!   listes disjointes (`(?i-i:x)` → `SyntaxError`), gauche
//!   vide licite avec `-` (`(?-i:x)` = retrait pur).

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

/// `(?s:…)` et `(?-s:…)` — `.` suit l'état effectif, y
/// compris les 4 `LineTerminator`, et la portée se referme.
#[test]
fn dotall_modifier_scope() {
    // `(?s:.)` dans un pattern non-`s` : LT inclus.
    assert_eq!(eval("/(?s:.)/.test('\\n')"), "true");
    assert_eq!(eval("/(?s:.)/.test('\\u2028')"), "true");
    assert_eq!(eval("/(?s:.)/.test('\\u2029')"), "true");
    assert_eq!(eval("/(?s:.)/.test('x')"), "true");
    // Hors portée : `.` redevient sans-LT.
    assert_eq!(eval("/a.(?s:b.b).c/.test('a,b\\nb,c')"), "true");
    assert_eq!(eval("/a.(?s:b.b).c/.test('a\\nb\\nb,c')"), "false");
    assert_eq!(eval("/a.(?s:b.b).c/.test('a,b\\nb\\nc')"), "false");
    // `(?-s:.)` sous `s` global : LT exclus dans la portée.
    assert_eq!(eval("/(?-s:.)/s.test('\\n')"), "false");
    assert_eq!(eval("/(?-s:.)/s.test('x')"), "true");
    assert_eq!(eval("/.(?-s:.)./s.test('\\n\\n\\n')"), "false");
    assert_eq!(eval("/.(?-s:.)./s.test('\\nx\\n')"), "true");
    // `-` avec remove vide : `(?s-:…)`.
    assert_eq!(eval("/(?s-:.)/.test('\\n')"), "true");
}

/// `(?i:…)`/`(?-i:…)` non-`u` : folding simple-upper dans la
/// portée, jamais le folding Unicode du backend.
#[test]
fn icase_modifier_scope_non_u() {
    // Lettres pliées dans la portée.
    assert_eq!(eval("/(?i:k)/.test('K')"), "true");
    assert_eq!(eval("/(?i:k)/.test('k')"), "true");
    // Pas de folding Unicode : la canonicalisation non-`u` est
    // `ToUppercase` simple — 'k'→'K' ≠ KELVIN U+212A.
    assert_eq!(eval("/(?i:k)/.test('\\u212a')"), "false");
    assert_eq!(eval("/(?i:\\u212a)/.test('k')"), "false");
    assert_eq!(eval("/(?i:\\u212a)/.test('\\u212a')"), "true");
    // Classes pliées dans la portée.
    assert_eq!(eval("/(?i:[a-c])/.test('B')"), "true");
    assert_eq!(eval("/(?i:[a-c])/.test('d')"), "false");
    // Hors portée : pas de folding.
    assert_eq!(eval("/(?i:k)l/.test('KL')"), "false");
    // `(?-i:…)` sous `i` global : folding coupé dans la portée.
    assert_eq!(eval("/(?-i:k)/i.test('K')"), "false");
    assert_eq!(eval("/(?-i:k)/i.test('k')"), "true");
    assert_eq!(eval("/x(?-i:y)z/i.test('xYz')"), "false");
    assert_eq!(eval("/x(?-i:y)z/i.test('XyZ')"), "true");
    // Backref insensible à la casse dans la portée.
    assert_eq!(eval("/(a)(?i:\\1)/.test('aA')"), "true");
    assert_eq!(eval("/(a)(?i:\\1)/.test('aa')"), "true");
    assert_eq!(eval("/(a)(?i-:\\1)/.test('aA')"), "true");
    // Et retirée sous `i` global.
    assert_eq!(eval("/(a)(?-i:\\1)/i.test('aA')"), "false");
}

/// `(?i:…)` sous `u` : émis natif — folding Unicode complet
/// (spec `u`), KELVIN inclus.
#[test]
fn icase_modifier_scope_u() {
    assert_eq!(eval("/(?i:k)/u.test('K')"), "true");
    assert_eq!(eval("/(?i:k)/u.test('\\u212a')"), "true");
    assert_eq!(eval("/(?-i:k)/iu.test('K')"), "false");
}

/// `(?m:…)`/`(?-m:…)` : passé natif — `^`/`$` en bord de
/// ligne dans la portée seulement.
#[test]
fn multiline_modifier_scope() {
    assert_eq!(eval("/(?m:^b$)/.test('a\\nb\\nc')"), "true");
    assert_eq!(eval("/^a\\n(?m:^b$)\\nc$/.test('a\\nb\\nc')"), "true");
    assert_eq!(eval("/^a\\n(?m:^b$)\\nc$/.test('\\na\\nb\\nc')"), "false");
    assert_eq!(eval("/(?m:es$)/.test('es\\ns')"), "true");
    // `-m` sous `m` global : bords de ligne coupés.
    assert_eq!(eval("/(?-m:^b$)/m.test('a\\nb\\nc')"), "false");
}

/// Validation : listes disjointes et flags autorisés.
#[test]
fn modifier_syntax_errors() {
    // Listes disjointes — `(?i-i:x)` → SyntaxError ; `i` en
    // `sub` seulement est licite (`(?sm-i:x)` valide).
    for bad in ["/(?i-i:x)/", "/(?im-m:x)/", "/(?sm-s:x)/"] {
        let r = eval(&format!("eval('{bad}')"));
        assert!(
            r.starts_with("ERR"),
            "{bad} devrait lever une erreur, obtenu {r}"
        );
    }
    // `g`/`u`/`v`/`y`/`d` interdits en modifiers.
    for bad in ["/(?g:x)/", "/(?u:x)/", "/(?y:x)/"] {
        let r = eval(&format!("eval('{bad}')"));
        assert!(
            r.starts_with("ERR"),
            "{bad} devrait lever une erreur, obtenu {r}"
        );
    }
    // Gauche vide licite : `(?-i:…)` = retrait pur.
    assert_eq!(eval("/(?-i:k)/i.test('K')"), "false");
    assert_eq!(eval("/(?-i:k)/.test('k')"), "true");
    // Alternatives hors portée non affectées.
    assert_eq!(eval("/(?i:a)|b/.test('B')"), "false");
    assert_eq!(eval("/(?i:a)|b/.test('A')"), "true");
}

/// `GetWordCharacters(modifiers)` : `\w`/`\W`/`\b`/`\B`
/// suivent l'`i` effectif — ASCII63 seul sans `i`, +`ſK`
/// (u) / +`ſıȷ` (non-u) avec.
#[test]
fn word_chars_effective_i() {
    // `\w` JS = ASCII63, jamais les word-chars Unicode —
    // fix préexistant : 'é' ne matchait pas à tort.
    assert_eq!(eval("/\\w/.test('e')"), "true");
    assert_eq!(eval("/\\w/.test('\\u00e9')"), "false");
    // Sous `i` effectif u : + `ſ`/`K` (fold ∈ ASCII).
    assert_eq!(eval("/\\w/u.test('\\u017f')"), "false");
    assert_eq!(eval("/(?i:\\w)/u.test('\\u017f')"), "true");
    assert_eq!(eval("/(?i:\\w)/u.test('\\u212a')"), "true");
    assert_eq!(eval("/\\w/iu.test('\\u212a')"), "true");
    // `(?-i:` retire les extras.
    assert_eq!(eval("/(?-i:\\w)/ui.test('\\u017f')"), "false");
    assert_eq!(eval("/(?-i:\\W)/ui.test('\\u017f')"), "true");
    // `\b`/`\B` bornés sur le même jeu.
    assert_eq!(eval("/(?i:\\b)/u.test('\\u017f')"), "true");
    assert_eq!(eval("/(?-i:\\b)/ui.test('\\u017f')"), "false");
    assert_eq!(eval("/(?-i:Z\\B)/ui.test('Z\\u017f')"), "false");
    assert_eq!(eval("/a\\bb/.test('ab')"), "false");
    // `a\bb` : la frontière après 'a' existe mais 'b' n'est
    // pas à sa position — jamais de match.
    assert_eq!(eval("/a\\bb/.test('a b')"), "false");
    assert_eq!(eval("/a\\b/.test('a ')"), "true");
}

/// `\P{P}` sous `i` effectif : `{c : canon(c) ∈ canon(¬P)}`
/// — `\P{Lu}`+i matche 'a' **et** 'A' (non-`u` : identité
/// Annex B `\P`, pas concerné).
#[test]
fn negated_property_under_i() {
    assert_eq!(eval("/\\P{Lu}/u.test('a')"), "true");
    assert_eq!(eval("/\\P{Lu}/u.test('A')"), "false");
    assert_eq!(eval("/\\P{Lu}/iu.test('a')"), "true");
    assert_eq!(eval("/\\P{Lu}/iu.test('A')"), "true");
    assert_eq!(eval("/(?i:\\P{Lu})/u.test('a')"), "true");
    assert_eq!(eval("/(?i:\\P{Lu})/u.test('A')"), "true");
    assert_eq!(eval("/(?i:\\P{Lu})/u.test('0')"), "true");
}

/// Modifiers imbriqués : l'état interne prime, la sortie
/// restaure l'état englobant.
#[test]
fn nested_modifiers() {
    assert_eq!(eval("/(?i:x(?-i:y)z)/.test('XYZ')"), "false");
    assert_eq!(eval("/(?i:x(?-i:y)z)/.test('XyZ')"), "true");
    assert_eq!(eval("/(?s:a(?-s:.)c)/.test('a\\nc')"), "false");
    assert_eq!(eval("/(?s:a(?-s:.)c)/.test('abc')"), "true");
}
