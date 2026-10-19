// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! `explorer-net` — pile réseau : URL, DNS, HTTP/1.1, TLS (`rustls`),
//! décodage de transfert, cookies, cache, MIME.
//!
//! Squelette de l'étape 1 — la phase 1 implémente le contenu
//! (`ROADMAP.md`, ADR-0002). Seul crate du workspace autorisé à
//! ouvrir des sockets ; `HttpTransport` est injectable pour que les
//! tests tournent in-process.

#![forbid(unsafe_code)]

pub mod cache;
pub mod cookies;
pub mod decode;
pub mod error;
pub mod h1;
pub mod h2;
pub mod http;
pub mod policy;
pub mod proxy;
pub mod tls;
pub mod url;
