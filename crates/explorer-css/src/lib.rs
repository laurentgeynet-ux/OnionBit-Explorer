// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! `explorer-css` — tokenizer, parser, sélecteurs et cascade CSS.
//!
//! Étape 13 : [`tokenize`] + [`parse_stylesheet`] — tolérant aux
//! erreurs de la spec (déclarations/règles invalides sautées,
//! jamais de panic). Domaine pur : aucun I/O, aucun async.

#![forbid(unsafe_code)]

mod parser;
mod token;
mod tokenizer;

pub use parser::{
    parse_stylesheet, AtKind, AtRule, Declaration, ImportRule, MediaQuery, QualifiedRule, Rule,
    Stylesheet,
};
pub use token::Token;
pub use tokenizer::tokenize;
