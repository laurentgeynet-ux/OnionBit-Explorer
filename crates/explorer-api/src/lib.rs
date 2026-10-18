// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! `explorer-api` — API REST + SSE (`axum`) exposée sur loopback :
//! routes, DTO, SSE, clé optionnelle (ADR-0003/0004).
//!
//! Squelette de l'étape 1 — la phase 6 implémente le contenu.
//! Les DTO ne fuient jamais dans `explorer-core`.

#![forbid(unsafe_code)]
