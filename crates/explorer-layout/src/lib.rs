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
//! L'étape 18 pose la stack texte (ADR-0009) : [`text`]
//! itemise bidi/UAX #14, [`font`] shape via `rustybuzz` derrière
//! le port [`FontSource`] (implémentation `fontdb` en infra,
//! injectée par `explorer-core`).
//! Domaine pur : aucun I/O ici.

#![forbid(unsafe_code)]

pub mod block;
mod boxtree;
mod flex;
pub mod font;
mod grid;
mod hit;
pub mod inline;
mod intrinsic;
mod relayout;
mod tables;
pub mod text;

pub use block::{layout_tree, BlockCtx};
pub use boxtree::{
    build_box_tree, build_box_tree_sized, BoxKind, BoxSlot, Float, LayoutBox, Overflow, Position,
    Rect,
};
pub use font::{FontMetrics, FontRequest, FontSource, Glyph, ShapedRun};
pub use hit::hit_test;
pub use inline::{Fragment, LineBox};
pub use relayout::{relayout_subtree, Relayout};
pub use text::{Break, Direction, TextRun};
