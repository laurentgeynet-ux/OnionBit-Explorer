// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! `explorer-html` — tokenizer HTML5 WHATWG (étape 10) puis
//! tree-builder vers `explorer-dom` (étape 11).
//!
//! Entrée **incrémentale** (ADR-0012) : le flux arrive par chunks
//! — [`TreeBuilder::push`] + [`TreeBuilder::finish`] est la
//! conduite de haut niveau (tokenizer interne, DOM observable
//! entre les chunks via [`TreeBuilder::dom`]) ; le couple bas
//! niveau [`Tokenizer::feed`]/[`Tokenizer::next_token`] reste
//! exposé pour une conduite externe via [`TreeBuilder::process`].
//! En amont, [`encoding`] fournit le sniffing BOM/`<meta>`/
//! `windows-1252` et le transcodage vers `String` — newlines
//! normalisés par le DOM à l'insertion.
//!
//! Domaine pur : aucun I/O, aucun async. Toute entrée malformée
//! produit des tokens dégradés, jamais de panic.

#![forbid(unsafe_code)]

pub mod debug;
pub mod encoding;
pub mod entities;
mod token;
mod tokenizer;
mod tree;

pub use token::{Token, TokenAttribute};
pub use tokenizer::{ContentKind, Tokenizer};
pub use tree::{QuirksMode, TreeBuilder};
