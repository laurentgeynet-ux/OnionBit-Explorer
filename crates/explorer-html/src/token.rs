// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Tokens émis par le tokenizer HTML5.

/// Attribut d'un tag, ordre d'insertion préservé (doublons : le
/// premier gagne, conformément à la spec).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenAttribute {
    /// Nom (déjà en minuscules pour les balises ASCII).
    pub name: String,
    /// Valeur, entités résolues.
    pub value: String,
}

/// Token de sortie du tokenizer — les `Character` sont coalescés en
/// séquences (licite : la spec html5lib-tests accepte le batching).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Token {
    /// Série de caractères texte (newlines normalisés en amont).
    Character(String),
    /// Balise ouvrante.
    StartTag {
        /// Nom en minuscules.
        name: String,
        /// Attributs dans l'ordre du source.
        attributes: Vec<TokenAttribute>,
        /// `<foo />` — flag syntaxique (le tree-builder décide).
        self_closing: bool,
    },
    /// Balise fermante.
    EndTag {
        /// Nom en minuscules.
        name: String,
    },
    /// `<!-- … -->` (bogus comments inclus : `<?…>`, `<!x…>`).
    Comment(String),
    /// `<!DOCTYPE …>` ; `force_quirks` = parsing dégradé → quirks mode.
    Doctype {
        /// Nom (ex. `html`).
        name: Option<String>,
        /// Identifiant public.
        public_id: Option<String>,
        /// Identifiant système.
        system_id: Option<String>,
        /// `true` → le document passe en quirks mode.
        force_quirks: bool,
    },
    /// Fin du flux.
    Eof,
}
