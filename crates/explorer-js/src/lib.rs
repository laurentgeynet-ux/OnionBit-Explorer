// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! `explorer-js` — couture JavaScript : trait `JsEngine` (évaluation,
//! contexte par onglet, budget d'exécution), bindings DOM et table de
//! roots (ADR-0008).
//!
//! Squelette de l'étape 1 — la phase 8 implémente le contenu.
//! `dillo-js` (crate de ce workspace, équipe JS dédiée) est adapté
//! derrière le trait ; `NullEngine` reste le défaut tant que la phase
//! 8 n'est pas livrée.

#![forbid(unsafe_code)]
