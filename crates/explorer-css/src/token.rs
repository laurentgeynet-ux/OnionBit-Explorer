// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Tokens CSS (spec CSS Syntax Level 3) — sortie du tokenizer.

/// Un token CSS. Les valeurs numériques gardent la représentation
/// source (`repr`) pour préserver `+`/`.5`/`1e3` — la valeur
/// flottante sert au tri/comparaisons.
#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    /// Identifiant (`color`, `--var`, `-webkit-x`).
    Ident(String),
    /// Nom de fonction suivi de `(` (`rgb`, `var`, `calc`).
    Function(String),
    /// `@name` (at-rule).
    AtKeyword(String),
    /// `#abc` — `is_id` vrai si la forme est un id valide.
    Hash {
        value: String,
        is_id: bool,
    },
    /// Chaîne entre quotes — contenu décodé (échappements résolus).
    Str(String),
    /// `bad-string` : newline non échappé dans une chaîne (erreur
    /// de tokenization tolérée — la règle sera rejetée).
    BadString,
    /// `url(...)` non quoté décodé.
    Url(String),
    /// `bad-url` : caractère invalide dans `url(...)` non quoté.
    BadUrl,
    /// Nombre nu (`12`, `+4.5`, `-1e3`).
    Number {
        repr: String,
        value: f64,
    },
    /// Pourcentage (`50%`).
    Percentage {
        repr: String,
        value: f64,
    },
    /// Dimension (`10px`, `2.5em`, `1e2ms`).
    Dimension {
        repr: String,
        value: f64,
        unit: String,
    },
    /// Un ou plusieurs espaces/tab/newlines (normalisés).
    Whitespace,
    /// `:`.
    Colon,
    /// `;`.
    Semicolon,
    /// `,`.
    Comma,
    /// `[` `]` `{` `}` `(` `)` — ponctuation de bloc.
    OpenBracket,
    CloseBracket,
    OpenBrace,
    CloseBrace,
    OpenParen,
    CloseParen,
    /// `<!--` (toléré en tête de feuille, legacy).
    Cdo,
    /// `-->` (idem).
    Cdc,
    /// Tout autre caractère isolé.
    Delim(char),
    /// Fin du flux.
    Eof,
}
