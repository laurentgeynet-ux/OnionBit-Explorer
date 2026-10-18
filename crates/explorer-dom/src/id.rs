// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Identifiants de nœuds — `NodeId` générationnels (ADR-0008).

use std::fmt;

/// Handle opaque vers un nœud de l'arène.
///
/// `index` désigne la case de l'arène, `generation` invalide les
/// handles obsolètes : quand un nœud est collecté, la génération de
/// sa case est incrémentée et tout `NodeId` ancien échoue avec
/// [`crate::DomError::StaleHandle`] — jamais d'accès à un objet
/// réaffecté (ADR-0008).
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct NodeId {
    pub(crate) index: u32,
    pub(crate) generation: u32,
}

impl NodeId {
    /// Construit un handle — usage interne.
    #[must_use]
    pub(crate) fn new(index: u32, generation: u32) -> Self {
        Self { index, generation }
    }

    /// Construit un handle arbitraire — réservé aux fixtures de
    /// test (spike ADR-0008 à faux handles, hors de ce crate).
    /// Ne jamais utiliser dans le pipeline.
    #[doc(hidden)]
    #[must_use]
    pub fn from_parts(index: u32, generation: u32) -> Self {
        Self { index, generation }
    }

    /// Index dans l'arène (diagnostic, logs).
    #[must_use]
    pub fn index(self) -> u32 {
        self.index
    }

    /// Génération capturée à la création du handle.
    #[must_use]
    pub fn generation(self) -> u32 {
        self.generation
    }
}

impl fmt::Debug for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "NodeId({}v{})", self.index, self.generation)
    }
}

/// Clé de la table de rooting — une entrée maintient un nœud
/// (et son sous-arbre) vivant même détaché du document (ADR-0008).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct RootId(pub(crate) u64);
