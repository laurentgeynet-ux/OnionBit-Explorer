// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! `explorer-core` — couche application : session, onglets,
//! navigation, orchestration du pipeline (`fetch → parse → style →
//! layout → render`), bus d'événements.
//!
//! Étape 24 : [`Session`] pilote le pipeline complet derrière les
//! ports injectés (`HttpTransport`, `FontSource`) et diffuse
//! `frame_invalidated` pour le pull PNG (ADR-0004).
//! Ne connaît que des ports — jamais les sockets ni axum.
//! Single-writer : le DOM et le layout vivent sur un flux unique.

#![forbid(unsafe_code)]

pub mod session;

pub use session::{
    drain_navs_shared, history_move_shared, navigate_shared, Bookmark, CoreError, CoreEvent,
    InputEvent, InputOutcome, ProfileStore, Session, SessionConfig, TabId, TabInfo, Visit,
};
