// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Erreurs typées du DOM — jamais de panic sur contenu hostile.

use thiserror::Error;

use crate::id::NodeId;

/// Erreurs des opérations sur l'arène DOM.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum DomError {
    /// Le handle pointe une case dont la génération a avancé —
    /// le nœud visé a été collecté (ADR-0008).
    #[error("handle obsolète {id:?} : génération attendue {expected}, case en {found}")]
    StaleHandle {
        /// Handle fautif.
        id: NodeId,
        /// Génération portée par le handle.
        expected: u32,
        /// Génération actuelle de la case.
        found: u32,
    },

    /// L'index du handle déborde l'arène (handle fabriqué).
    #[error("handle hors arène {0:?}")]
    OutOfBounds(NodeId),

    /// Opération réservée à un autre type de nœud.
    #[error("mauvais type de nœud {id:?} : attendu {expected}")]
    WrongKind {
        /// Nœud fautif.
        id: NodeId,
        /// Type attendu.
        expected: &'static str,
    },

    /// Requête de hiérarchie invalide (cycle, insertion de soi…).
    #[error("hiérarchie invalide : {0}")]
    HierarchyRequest(&'static str),

    /// Clé de rooting inconnue.
    #[error("root inconnu {0:?}")]
    UnknownRoot(crate::id::RootId),

    /// Budget de nœuds de l'arène atteint (ADR-0011 — le contenu
    /// hostile ne doit pas pouvoir grossir le DOM sans borne).
    #[error("budget de nœuds atteint ({limit})")]
    BudgetExceeded {
        /// Budget configuré.
        limit: usize,
    },
}

/// Alias de résultat du crate.
pub type Result<T> = std::result::Result<T, DomError>;
