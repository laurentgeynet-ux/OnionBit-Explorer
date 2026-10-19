// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! `explorer-layout` — arbre de boîtes et algorithmes de
//! dimensionnement (block, inline, tables, flottants, positionnement).
//!
//! L'étape 17 construit l'arbre de boîtes : [`build_box_tree`]
//! transforme DOM + styles calculés en [`LayoutBox`]
//! block/inline/anonymes/texte ; [`LayoutBox::dump`] produit un
//! wireframe texte (feedback visuel avant le raster).
//! Domaine pur : le texte travaille sur des runs itemisés
//! (ADR-0009) et les fontes arrivent par un port `FontProvider`
//! injecté — aucun I/O ici.

#![forbid(unsafe_code)]

mod boxtree;

pub use boxtree::{build_box_tree, BoxKind, LayoutBox, Rect};
