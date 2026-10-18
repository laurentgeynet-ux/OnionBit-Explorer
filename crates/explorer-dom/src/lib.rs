// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! `explorer-dom` — arène DOM et `NodeId` générationnels.
//!
//! Étape 9 (phase 2) : arbre de nœuds typés (Document, Doctype,
//! Element, Text, Comment) sur une arène à cases générationnelles.
//!
//! Le modèle mémoire est figé par **ADR-0008** :
//! - `NodeId` = index + génération — un handle périmé échoue typé
//!   ([`DomError::StaleHandle`]), jamais d'accès à une case
//!   réattribuée ;
//! - détacher un sous-arbre ne le détruit pas (sémantique DOM) —
//!   [`Dom::collect`] réclame ce qui est inaccessible depuis le
//!   document **et** non rooté ;
//! - le rooting explicite ([`Dom::add_root`]/[`Dom::remove_root`])
//!   est la couture par laquelle le moteur JS maintient ses handles
//!   vivants — les cycles DOM↔JS passent par cette table, jamais
//!   par des liens forts.
//!
//! Domaine pur : aucun I/O, aucun async. Mutations bornées par le
//! budget `max_nodes` (ADR-0011 — contenu hostile).

#![forbid(unsafe_code)]

mod dom;
mod error;
mod id;
mod node;

pub use dom::{Dom, DEFAULT_MAX_NODES};
pub use error::{DomError, Result};
pub use id::{NodeId, RootId};
pub use node::{Attribute, DoctypeData, ElementData, NodeData};
