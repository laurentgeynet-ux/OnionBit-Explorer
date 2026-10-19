// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! `explorer-test-support` — transports simulés (`HttpTransport`
//! in-process), fixtures de pages et fonte embarquée pour les golden
//! tests headless (ADR-0009/0010).
//!
//! La fonte fixture [`fixture_ttf`] est la source unique des
//! métriques de test (layout, display list, raster).
//! Volontairement **sans dépendances internes** : les crates de
//! domaine (`layout`, `render`) l'utilisent en dev-dependency
//! sans tirer `tokio`/`rustls` dans leurs tests.

#![forbid(unsafe_code)]

pub mod font_fixture;

pub use font_fixture::fixture_ttf;
