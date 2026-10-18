// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! `explorer-core` — couche application : session, onglets,
//! navigation, orchestration du pipeline (`fetch → parse → style →
//! layout → render`), bus d'événements.
//!
//! Squelette de l'étape 1. Ne connaît que des ports injectables
//! (`HttpTransport`, `JsEngine`) — jamais les sockets ni axum.
//! Single-writer : le DOM et le layout vivent sur un flux unique.

#![forbid(unsafe_code)]
