// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Contenu des nœuds — données par type + chaînage d'arbre.

use crate::id::NodeId;

/// Contenu typé d'un nœud de l'arène.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeData {
    /// Racine unique — index 0 de l'arène, toujours vivante.
    Document,
    /// `<!DOCTYPE>` : nom, identifiants public/système.
    Doctype(DoctypeData),
    /// Élément : nom de balise + attributs ordonnés.
    Element(ElementData),
    /// Texte (newlines normalisés `\r\n`/`\r` → `\n` à l'insertion).
    Text(String),
    /// Commentaire `<!-- … -->`.
    Comment(String),
}

/// Données d'un `<!DOCTYPE html …>`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DoctypeData {
    /// Nom (`html`).
    pub name: String,
    /// Identifiant public (souvent vide).
    pub public_id: String,
    /// Identifiant système (souvent vide).
    pub system_id: String,
}

/// Attribut d'élément — l'ordre d'insertion est préservé (la
/// sérialisation DOM l'exige).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attribute {
    /// Nom (casse préservée — la normalisation HTML est au tree-builder).
    pub name: String,
    /// Valeur brute.
    pub value: String,
}

/// Données d'un élément.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ElementData {
    /// Nom de balise.
    pub tag_name: String,
    /// Attributs dans l'ordre d'insertion.
    pub attributes: Vec<Attribute>,
}

impl ElementData {
    /// Crée un élément sans attribut.
    pub fn new(tag_name: impl Into<String>) -> Self {
        Self {
            tag_name: tag_name.into(),
            attributes: Vec::new(),
        }
    }
}

/// Nœud d'arène : chaînage intrusif parent/enfants/siblings +
/// contenu typé. Tous les liens sont des `NodeId` — validité
/// garantie par la génération à chaque accès via `Dom`.
#[derive(Debug)]
pub(crate) struct Node {
    pub(crate) parent: Option<NodeId>,
    pub(crate) first_child: Option<NodeId>,
    pub(crate) last_child: Option<NodeId>,
    pub(crate) prev_sibling: Option<NodeId>,
    pub(crate) next_sibling: Option<NodeId>,
    pub(crate) data: NodeData,
}

impl Node {
    /// Crée un nœud orphelin (aucun lien).
    pub(crate) fn new(data: NodeData) -> Self {
        Self {
            parent: None,
            first_child: None,
            last_child: None,
            prev_sibling: None,
            next_sibling: None,
            data,
        }
    }
}
