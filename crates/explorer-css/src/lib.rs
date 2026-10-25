// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! `explorer-css` — tokenizer, parser, sélecteurs et cascade CSS.
//!
//! Étape 13 : [`tokenize`] + [`parse_stylesheet`] — tolérant aux
//! erreurs de la spec (déclarations/règles invalides sautées,
//! jamais de panic). Domaine pur : aucun I/O, aucun async.
//!
//! Étape 14 : [`parse_selector_list`] + [`crate::matches`]/[`matches_any`]
//! — type/classe/id/attributs, combinateurs, pseudo-classes
//! structurelles, spécificité. Le matching interroge
//! [`explorer_dom::Dom`] par ses accesseurs `Result` — un handle
//! obsolète dégrade en non-match.
//!
//! Étape 15 : [`StyleEngine`] — cascade (UA < auteur <
//! `!important`), ordre source, héritage, mots-clés
//! `inherit`/`initial`/`unset`, media queries évaluées sur
//! [`StyleEnv`], feuille UA intégrée.
//!
//! Étape 16 : [`value`] — longueurs/couleurs/nombres typés,
//! expansion des shorthands (`margin`, `border`, `font`…),
//! pseudo-classes dynamiques pilotées par un état externe
//! (`compute_with_state`, restyle ADR-0012).

#![forbid(unsafe_code)]

pub mod animation;
mod cascade;
mod fontface;
mod parser;
mod selector;
mod state;
mod token;
mod tokenizer;
pub mod transition;
pub mod value;

pub use cascade::{
    initial_value, is_inherited, media_matches, ColorScheme, ComputedStyle, Origin, StyleEngine,
    StyleEnv, StyleMap,
};
pub use fontface::{FontFace, FontSrc};
pub use parser::{
    parse_stylesheet, AtKind, AtRule, Declaration, ImportRule, MediaQuery, QualifiedRule, Rule,
    Stylesheet,
};
pub use selector::{
    matches, matches_any, matches_any_with_state, matches_with_state, parse_selector_list, AttrOp,
    Combinator, ComplexSelector, CompoundSelector, DynamicPseudo, PseudoClass, PseudoElement,
    SimpleSelector, Specificity,
};
pub use token::Token;
pub use tokenizer::tokenize;
