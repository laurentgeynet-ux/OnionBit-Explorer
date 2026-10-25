// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Références de caractères nommées (`&amp;`, `&nbsp;`…).
//!
//! La table complète WHATWG (~2 100 noms, `entities_data.rs`,
//! générée depuis `entities.json` officiel — R-15) couvre le jeu
//! entier de la spec ; `legacy` marque les ~106 noms admis sans
//! point-virgule dans le contenu (jamais dans les attributs
//! suivis de `=`/alphanumérique — règle de l'esperluette ambiguë).

mod entities_data;
use entities_data::NAMED_REFS;

/// Une entité nommée : nom sans `&` ni `;`, rendu, et `legacy` =
/// autorisée sans `;` dans le contenu (jamais dans les attributs
/// suivis de `=`/alphanumérique — règle de l'esperluette ambiguë).
struct NamedRef {
    name: &'static str,
    value: &'static str,
    legacy: bool,
}

/// Résultat d'une résolution d'entité : rendu + longueur consommée
/// dans l'entrée (`&` non compris — inclut le `;` s'il est présent).
///
/// Exposé pour le miroir DOM (`explorer-jscore` décode le texte
/// injecté via `innerHTML`/`document.write` avec la même table —
/// demande journalisée 2026-10-21, R-98).
pub struct EntityMatch {
    /// Rendu de la référence (chaîne de remplacement).
    pub value: &'static str,
    /// Longueur consommée dans l'entrée, `&` non compris —
    /// inclut le `;` s'il est présent.
    pub consumed: usize,
}

/// Résout une référence nommée commençant à `input` (juste après
/// `&`). Matching du plus long préfixe, `;` d'abord puis legacy
/// sans `;` (`in_attribute` applique la règle de l'esperluette
/// ambiguë : jamais devant `=`/alphanumérique).
#[must_use]
pub fn lookup_named(input: &str, in_attribute: bool) -> Option<EntityMatch> {
    // `;` d'abord — matching exact le plus long.
    let mut best: Option<&NamedRef> = None;
    let mut best_len = 0;
    for r in NAMED_REFS {
        if r.name.len() > best_len
            && input.starts_with(r.name)
            && input.as_bytes().get(r.name.len()) == Some(&b';')
        {
            best = Some(r);
            best_len = r.name.len();
        }
    }
    if let Some(r) = best {
        return Some(EntityMatch {
            value: r.value,
            consumed: r.name.len() + 1,
        });
    }
    // Legacy sans `;` — uniquement les entités marquées.
    let after_blocked = |len: usize| -> bool {
        in_attribute
            && input[len..]
                .chars()
                .next()
                .is_some_and(|c| c == '=' || c.is_ascii_alphanumeric())
    };
    let mut best: Option<&NamedRef> = None;
    let mut best_len = 0;
    for r in NAMED_REFS {
        if r.legacy && r.name.len() > best_len && input.starts_with(r.name) {
            // Règle de l'esperluette ambiguë : en attribut, jamais
            // devant `=` ou alphanumérique (`?a=1&amp=x` littéral).
            // Dans le contenu, `&notit` = `¬it` (spec) — pas de blocage.
            if in_attribute && after_blocked(r.name.len()) {
                continue;
            }
            best = Some(r);
            best_len = r.name.len();
        }
    }
    best.map(|r| EntityMatch {
        value: r.value,
        consumed: r.name.len(),
    })
}

/// Remap `windows-1252` pour les références numériques
/// `0x80..=0x9F` (spec : `&#150;` = `–`, pas U+0096).
const NUMERIC_1252: [char; 32] = [
    '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8D}', 'Ž', '\u{8F}',
    '\u{90}', '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{9D}', 'ž', 'Ÿ',
];

/// Convertit un codepoint de référence numérique selon la spec :
/// `0x00` → `U+FFFD`, surrogates/sup → `U+FFFD`, `0x80..=0x9F` →
/// remap `windows-1252`.
#[must_use]
pub fn map_numeric(codepoint: u32) -> char {
    match codepoint {
        0x00 => '\u{FFFD}',
        0x80..=0x9F => NUMERIC_1252[(codepoint - 0x80) as usize],
        cp => char::from_u32(cp).unwrap_or('\u{FFFD}'),
    }
}

/// Décode les références de caractères d'un texte complet
/// (version « string entière » du chemin `charref` du
/// tokenizer — R-98, pour les producteurs hors tokenizer comme
/// le miroir DOM de `explorer-jscore`).
///
/// Mêmes règles : `&name;`, nommées legacy sans `;`
/// (esperluette ambiguë honorée si `in_attribute`), `&#123;`
/// et `&#x1F;` avec remap `windows-1252` via [`map_numeric`],
/// `&#` non suivi de chiffres et `&` orphelins restent
/// littéraux. Entrée supposée complète (pas de mode
/// streaming).
#[must_use]
pub fn decode_str(input: &str, in_attribute: bool) -> String {
    if !input.contains('&') {
        return input.to_owned();
    }
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let after = &rest[amp + 1..];
        let consumed = match after.chars().next() {
            Some('#') => {
                if let Some((text, n)) = numeric_ref(after) {
                    out.push_str(&text);
                    n
                } else {
                    out.push_str("&#");
                    1
                }
            }
            Some(c) if c.is_ascii_alphanumeric() => {
                if let Some(m) = lookup_named(after, in_attribute) {
                    out.push_str(m.value);
                    m.consumed
                } else {
                    out.push('&');
                    0
                }
            }
            _ => {
                out.push('&');
                0
            }
        };
        rest = &after[consumed..];
    }
    out.push_str(rest);
    out
}

/// Référence numérique `&#…` — `after` pointe juste après `&`,
/// donc commence par `#`. Retourne `(rendu, consommé dans
/// after)` ; `None` = `&#` non suivi de chiffres (littéral).
fn numeric_ref(after: &str) -> Option<(String, usize)> {
    let rem = &after[1..]; // après '#'
    let hex = matches!(rem.chars().next(), Some('x' | 'X'));
    let digit_pred = if hex {
        |c: char| c.is_ascii_hexdigit()
    } else {
        |c: char| c.is_ascii_digit()
    };
    let mut i = usize::from(hex);
    while rem[i..].chars().next().is_some_and(digit_pred) {
        i += rem[i..].chars().next().map_or(0, char::len_utf8);
    }
    if i == usize::from(hex) {
        return None;
    }
    let cp = u32::from_str_radix(&rem[usize::from(hex)..i], if hex { 16 } else { 10 })
        .unwrap_or(u32::MAX);
    Some((
        map_numeric(cp).to_string(),
        1 + i + usize::from(rem[i..].starts_with(';')),
    ))
}
