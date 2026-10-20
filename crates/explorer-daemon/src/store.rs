// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Ré-export — l implémentation `JsonFileStore` a migré
//! vers `explorer-core::profile_store` (étape 61) pour être
//! partagée par les deux hôtes (daemon + FFI) sans
//! dépendance latérale. Le module est conservé pour
//! compatibilité des chemins `explorer_daemon::store::*`.

pub use explorer_core::JsonFileStore;
