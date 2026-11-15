// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! `explorer-agent` — surface de pilotage IA (**EAP**, ADR-0035).
//!
//! Deuxième client de `Session` (le premier étant l'API UI REST+SSE,
//! ADR-0003 — inchangée). Ce crate porte :
//! - le **protocole** : types de messages JSON-RPC 2.0 + trames
//!   binaires ([`proto`]), miroir du schéma `schema/eap-1.schema.json`
//!   (source de vérité — les types sont tenus en phase, verrouillés
//!   par les vecteurs `schema/golden/`) ;
//! - le **transport** : WebSocket `GET /agent` sur le listener
//!   loopback existant, sous-protocole `eap/1` (E1) ;
//! - les **handles** d'éléments serveur, l'actionabilité et
//!   l'auto-wait sur commits (E2) ;
//! - le snapshot sémantique + diff `{since}` (E3).
//!
//! Contrainte de coexistence : tout accès moteur passe par les API
//! publiques de `Session` sous le même verrou tokio — jamais de
//! couture parallèle vers le pipeline.

#![forbid(unsafe_code)]

pub mod actionable;
pub mod anomalies;
pub mod dispatch;
pub mod proto;
pub mod snapshot;
pub mod state;
pub mod ws;

pub use state::{AgentConn, AgentShared};
pub use ws::agent_router;

/// Version de sous-protocole négociée au handshake WS
/// (`Sec-WebSocket-Protocol`).
pub const PROTOCOL: &str = "eap/1";

/// Le schéma JSON embarqué — servi tel quel par `GET /agent/schema`
/// pour introspection côté client.
pub const SCHEMA: &str = include_str!("../schema/eap-1.schema.json");
