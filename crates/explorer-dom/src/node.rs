// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Contenu des nœuds — données par type + chaînage d'arbre.

use crate::atom::Atom;
use crate::id::NodeId;

/// Contenu typé d'un nœud de l'arène.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeData {
    /// Racine unique — index 0 de l'arène, toujours vivante.
    Document,
    /// `<!DOCTYPE>` : nom, identifiants public/système. Boxé —
    /// un seul par document, il ne doit pas plafonner
    /// `NodeData` à 72 o (AR-13 : `Element` devient le variant
    /// maximal → `Node`/`Slot` rétrécissent).
    Doctype(Box<DoctypeData>),
    /// Élément : nom de balise + attributs ordonnés.
    Element(ElementData),
    /// Texte (newlines normalisés `\r\n`/`\r` → `\n` à l'insertion).
    /// Plage dans le buffer partagé `Dom::text_buffer` (AR-14).
    Text(TextRange),
    /// Commentaire `<!-- … -->` — même buffer que `Text`.
    Comment(TextRange),
}

/// Plage `(début, longueur)` en octets dans
/// [`crate::Dom::text_buffer`] — le buffer texte mutualisé de
/// l'arène (AR-14 : des milliers de petites `String` → une
/// allocation, compactée par `collect`). `Copy` — résolution
/// via [`crate::Dom::text_at`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TextRange {
    /// Premier octet dans le buffer.
    pub start: u32,
    /// Longueur en octets.
    pub len: u32,
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
    /// Nom interné — casse préservée (la normalisation HTML est
    /// au tree-builder). `Atom` : dédupliqué process-wide,
    /// `== "x"`/`as_str()`/`Deref<str>` préservent l'usage
    /// `String` (AR-13).
    pub name: Atom,
    /// Valeur brute.
    pub value: String,
}

/// Espace de nommage d'un élément — HTML par défaut ; `SVG` et
/// `MathML` sont posés par le tree-builder en contenu étranger
/// (HTML §13.2.6.5, R-15).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Namespace {
    /// `http://www.w3.org/1999/xhtml`.
    #[default]
    Html,
    /// `http://www.w3.org/2000/svg`.
    Svg,
    /// `http://www.w3.org/1998/Math/MathML`.
    MathMl,
}

/// Données d'un élément.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ElementData {
    /// Nom de balise interné — ajusté en casse pour les balises
    /// étrangères (`foreignObject`, `feGaussianBlur`…, R-15).
    /// `Atom` : dédupliqué process-wide (AR-13).
    pub tag_name: Atom,
    /// Namespace — `Html` sauf contenu étranger.
    pub namespace: Namespace,
    /// Attributs dans l'ordre d'insertion.
    pub attributes: Vec<Attribute>,
    /// Contenu propre d'un `<template>` (fragment document,
    /// R-15) — `None` pour les autres éléments.
    pub template_contents: Option<NodeId>,
}

impl ElementData {
    /// Crée un élément sans attribut — `tag_name` interné à la
    /// volée (`&str`, `String`, `Atom` acceptés).
    pub fn new(tag_name: impl Into<Atom>) -> Self {
        Self {
            tag_name: tag_name.into(),
            namespace: Namespace::Html,
            attributes: Vec::new(),
            template_contents: None,
        }
    }

    /// Crée un élément dans un namespace donné.
    pub fn with_namespace(tag_name: impl Into<Atom>, namespace: Namespace) -> Self {
        Self {
            namespace,
            ..Self::new(tag_name)
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
