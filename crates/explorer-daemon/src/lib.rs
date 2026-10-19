// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! `explorer-daemon` — binaire hôte : moteur + API loopback
//! supervisée par l'app Flutter (ADR-0003). La racine du
//! binaire vit dans `main.rs` ; cette bibliothèque expose
//! les modules d'adaptation testables : persistance du
//! profil (`store`) et cache HTTP disque (`cache`,
//! étape 56).

#![forbid(unsafe_code)]

pub mod cache;
pub mod store;
