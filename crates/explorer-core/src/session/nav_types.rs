// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! File des navigations différées émises par les scripts
//! (`location.href`, `history.*`).

use explorer_net::http::Request;

/// Action dépilée par un drain de navigation (étape 53d) —
/// requête prête (input `pending_nav`, `location`, submit
/// de formulaire) ou déplacement relatif d'historique.
pub(crate) enum DrainAct {
    /// `(requête, push historique)` — conduite par
    /// `navigate_request`/`navigate_shared`. `Box` :
    /// `Request` (233 o) domine `Go` (8 o) — la file
    /// n'embarque pas le pire cas par cellule.
    Req(Box<Request>, bool),
    /// Navigation dans un **nouveau** contexte top-level
    /// (étape 131 — `target=_blank`/nommé sur `<a>` ou
    /// `<form>`) : le drain ouvre un onglet puis y conduit
    /// la requête — l'intention vit dans la file de
    /// l'onglet émetteur. Le second membre = nom de cible
    /// (CAP-159) : `""`/`_blank` = toujours neuf ; un nom
    /// réutilise l'onglet porteur s'il existe.
    NewTab(Box<Request>, String),
    /// `history.go(d)` — résolu via `history_move` (ou son
    /// équivalent partagé) ; `0` = reload.
    Go(isize),
    /// `location.replace` (CAP-152) — navigation réelle
    /// (`push` off) puis **réécriture** de l'entrée
    /// courante par la requête commise : `back` saute la
    /// page remplacée (sémantique `historyHandling:
    /// "replace"`, flows auth).
    Replace(Box<Request>),
    /// `location.reload` (CAP-152) — rejeu de la requête
    /// historiée de l'entrée courante (méthode + corps —
    /// un POST re-POSTe), sans entrée nouvelle.
    Reload,
}

// ═══════════════ Étape 53d — drivers partagés ═══════════════
