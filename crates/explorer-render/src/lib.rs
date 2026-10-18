// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! `explorer-render` — display list → framebuffer RGBA → PNG
//! (`tiny-skia`, ADR-0001/0004).
//!
//! Squelette de l'étape 1 — la phase 5 implémente le contenu.
//! Une frame = un état cohérent : jamais de rasterisation d'un arbre
//! en cours de mutation.

#![forbid(unsafe_code)]
