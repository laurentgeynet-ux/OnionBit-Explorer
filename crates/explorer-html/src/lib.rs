// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! `explorer-html` — tokenizer et tree-builder HTML5 (états WHATWG,
//! insertion modes, foster parenting) vers `explorer-dom`.
//!
//! Squelette de l'étape 1 — la phase 2 implémente le contenu.
//! Tolérance tag-soup totale : dégrader, jamais planter.

#![forbid(unsafe_code)]
