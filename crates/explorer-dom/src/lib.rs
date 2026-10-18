// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! `explorer-dom` — arène DOM et `NodeId` générationnels.
//!
//! Squelette de l'étape 1 — la phase 2 implémente le contenu.
//! Le modèle mémoire (rooting explicite, cycles DOM↔JS par table de
//! callbacks, invalidation de génération documentée sur chaque
//! mutation) est figé par ADR-0008.

#![forbid(unsafe_code)]
