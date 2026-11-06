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

    /// Sentinelle « pas de lien » pour le chaînage intrusif
    /// interne (AR-15) : `index = u32::MAX` ne peut pas être
    /// une case réelle (l'arène est bornée par `max_nodes`,
    /// très loin de `u32::MAX`), et `generation = 0` ne
    /// correspond à aucune case (les générations démarrent à
    /// 1). Un lien NIL suivi par erreur échoue donc **typé**
    /// (`OutOfBounds` à la résolution) — jamais de mauvais
    /// nœud résolu en silence. Permet de stocker les liens en
    /// `NodeId` nu (8 o) plutôt qu'`Option<NodeId>` (12 o).
    pub(crate) const NIL: Self = Self {
        index: u32::MAX,
        generation: 0,
    };

    /// `true` si ce lien est la sentinelle NIL.
    #[must_use]
    pub(crate) fn is_nil(self) -> bool {
        self.index == u32::MAX
    }

    /// Convertit le lien interne en `Option` pour les
    /// accesseurs publics.
    #[must_use]
    pub(crate) fn as_option(self) -> Option<Self> {
        if self.is_nil() {
            None
        } else {
            Some(self)
        }
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
