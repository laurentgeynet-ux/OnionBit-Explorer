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

pub mod profile_store;
pub mod session;

pub use explorer_css::ColorScheme;
pub use profile_store::JsonFileStore;
pub use session::{
    drain_applies_shared, drain_navs_shared, history_move_shared, navigate_shared,
    pump_timers_shared, Bookmark, CoreError, CoreEvent, FrameRegion, InputEvent, InputOutcome,
    KeyMods, MediaCmd, MediaError, MediaHostState, MediaKind, MediaOfferInfo, MediaRect,
    MediaReport, ProfileStore, Session, SessionConfig, StoredCookie, TabId, TabInfo, Visit,
};
