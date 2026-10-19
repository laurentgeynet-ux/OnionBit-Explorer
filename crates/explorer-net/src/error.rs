// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Erreurs réseau typées — la politique UI (page d'erreur, jamais
//! de crash) consomme ces variantes (ADR-0002, ADR-0011).

use std::io;

use thiserror::Error;

/// Erreur réseau d'une requête.
#[derive(Debug, Error)]
pub enum NetError {
    /// Échec DNS ou adresse invalide.
    #[error("résolution DNS échouée pour {host} : {source}")]
    Dns {
        /// Hôte demandé.
        host: String,
        /// Erreur `std::io` sous-jacente.
        #[source]
        source: io::Error,
    },
    /// Échec de connexion TCP/TLS.
    #[error("connexion à {target} impossible : {source}")]
    Connect {
        /// `scheme://host:port`.
        target: String,
        /// Erreur sous-jacente.
        #[source]
        source: io::Error,
    },
    /// Erreur TLS (certificat, handshake…) — remontée typée pour
    /// que l'UI affiche une page d'erreur, jamais un crash.
    #[error("TLS vers {target} : {message}")]
    Tls {
        /// `scheme://host:port`.
        target: String,
        /// Description de l'échec TLS.
        message: String,
    },
    /// Timeout (connexion, lecture ou total).
    #[error("timeout {phase} après {ms} ms vers {target}")]
    Timeout {
        /// Phase : `connect`, `read`, `total`.
        phase: &'static str,
        /// Budget consommé.
        ms: u64,
        /// `scheme://host:port`.
        target: String,
    },
    /// Schéma non supporté par ce transport (`ftp:`…).
    #[error("schéma non supporté par le transport : {0}")]
    UnsupportedScheme(String),
    /// Réponse malformée (statut/en-têtes illisibles).
    #[error("réponse HTTP invalide : {0}")]
    InvalidResponse(String),
    /// Taille de l'en-tête au-delà du budget.
    #[error("en-tête de réponse > {0} octets")]
    HeadTooLarge(usize),
    /// Corps au-delà du budget mémoire.
    #[error("corps de réponse > {0} octets")]
    BodyTooLarge(usize),
    /// Corps tronqué (connexion coupée avant `Content-Length` ou
    /// chunk final).
    #[error("corps tronqué : {got} octets reçus, {expected} attendus")]
    TruncatedBody {
        /// Octets lus avant la coupure.
        got: usize,
        /// Taille attendue.
        expected: usize,
    },
    /// Chunked invalide (taille hex illisible, CR/LF manquant…).
    #[error("chunked invalide : {0}")]
    InvalidChunked(&'static str),
    /// Erreur I/O générique pendant le transfert.
    #[error("I/O : {0}")]
    Io(#[from] io::Error),
}

/// Alias de résultat réseau.
pub type NetResult<T> = Result<T, NetError>;
