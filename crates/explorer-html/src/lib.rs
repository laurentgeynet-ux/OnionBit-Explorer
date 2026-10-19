// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! `explorer-html` — tokenizer HTML5 WHATWG (étape 10) puis
//! tree-builder vers `explorer-dom` (étape 11).
//!
//! Entrée **incrémentale** (ADR-0012) : le flux arrive par chunks
//! via [`Tokenizer::feed`], les tokens se tirent par
//! [`Tokenizer::next_token`]. En amont, [`encoding`] fournit le
//! sniffing BOM/`<meta>`/`windows-1252` et le transcodage vers
//! `String` — newlines normalisés par le DOM à l'insertion.
//!
//! Domaine pur : aucun I/O, aucun async. Toute entrée malformée
//! produit des tokens dégradés, jamais de panic.

#![forbid(unsafe_code)]

pub mod encoding;
pub mod entities;
mod token;
mod tokenizer;
mod tree;

pub use token::{Token, TokenAttribute};
pub use tokenizer::{ContentKind, Tokenizer};
pub use tree::{QuirksMode, TreeBuilder};
