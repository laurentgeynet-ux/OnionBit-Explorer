// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! `explorer-layout` — arbre de boîtes et algorithmes de
//! dimensionnement (block, inline, tables, flottants, positionnement).
//!
//! Squelette de l'étape 1 — la phase 4 implémente le contenu.
//! Domaine pur : le texte travaille sur des runs itemisés
//! (ADR-0009) et les fontes arrivent par un port `FontProvider`
//! injecté — aucun I/O ici.

#![forbid(unsafe_code)]
