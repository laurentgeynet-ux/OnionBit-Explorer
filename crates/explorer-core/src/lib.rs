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

pub mod paths;
pub mod profile_store;
pub mod session;

/// §B-59 — dernier drain hôte entré (`&'static str`,
/// inconditionnel ~20 ns) : `poll_net`/`net_done`/`timers`/
/// `applies`/`eval`/`commit`. Vide à l'abort = la mort est sur
/// le fil JS (couvert par `last_phase`) ; nommé = la mort est
/// dans un drain core entre deux évals — casse « hors-fil » vs
/// « préambule JS » en un vol.
pub static LAST_DRAIN: std::sync::Mutex<&'static str> = std::sync::Mutex::new("");

pub use explorer_css::ColorScheme;
pub use profile_store::{acquire_profile_lock, JsonDnsStore, JsonFileStore, JsonHstsStore};
pub use session::{
    download_event_json, drain_applies_shared, drain_navs_shared, eval_shared, history_move_shared,
    media_codecs_from_json, media_event_json, media_host_state_str, media_offer_json,
    media_report_from_json, navigate_shared, pump_timers_shared, Bookmark, CoreError, CoreEvent,
    DownloadInfo, DownloadRecord, DownloadState, EvalReply, FrameRegion, FrameTop, GeomBox,
    GeometryReport, HitTargets,
    InputEvent, InputOutcome, KeyMods, MediaAudioSpec, MediaCmd, MediaError, MediaHostState,
    MediaKind, MediaOfferInfo, MediaRect, MediaReport, MsePull, ProfileStore, Session,
    SessionConfig, StoredCookie, TabId, TabInfo, Visit,
};
