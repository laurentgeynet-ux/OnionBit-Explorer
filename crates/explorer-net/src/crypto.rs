// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Fournisseur crypto de `crypto.subtle` (JS-C79) — côté
//! **plateforme** de la couture embedder : `explorer-jscore`
//! émet `CryptoRequest`, la session route `NetRequest::Crypto`
//! ici, le résultat retourne résoudre la `Promise`.
//!
//! `ring` est le provider déjà consacré pour TLS (ADR-0002,
//! exception `*-sys` documentée) — aucune autre pile crypto
//! n'entre dans l'arbre. Calcul pur, in-process : borné par la
//! taille des données (déjà matérialisées en mémoire côté
//! moteur — aucun I/O, aucune allocation d'entrée ici).

/// Algorithme `digest` — miroir local de
/// `explorer_jscore::DigestAlgo` (`explorer-net` ne connaît
/// pas le moteur : la traduction se fait dans `explorer-core`,
/// sens de dépendance respecté).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CryptoAlgo {
    /// `SHA-256` — PKCE `S256`, SRI.
    Sha256,
    /// `SHA-384`.
    Sha384,
    /// `SHA-512`.
    Sha512,
}

/// `subtle.digest(algo, data)` — les octets du condensat.
/// Fonction pure ; l'appelant décide du contexte d'exécution
/// (`spawn_blocking` pour rester hors de la boucle tokio sur
/// les gros buffers).
#[must_use]
pub fn digest(algo: CryptoAlgo, data: &[u8]) -> Vec<u8> {
    let a = match algo {
        CryptoAlgo::Sha256 => &ring::digest::SHA256,
        CryptoAlgo::Sha384 => &ring::digest::SHA384,
        CryptoAlgo::Sha512 => &ring::digest::SHA512,
    };
    ring::digest::digest(a, data).as_ref().to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Vecteurs FIPS 180-4 — « abc ».
    #[test]
    fn digest_vectors_abc() {
        use std::fmt::Write;
        let to_hex = |b: &[u8]| {
            b.iter().fold(String::new(), |mut s, x| {
                let _ = write!(s, "{x:02x}");
                s
            })
        };
        assert_eq!(
            to_hex(&digest(CryptoAlgo::Sha256, b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            to_hex(&digest(CryptoAlgo::Sha384, b"abc")),
            "cb00753f45a35e8bb5a03d699ac65007272c32ab0eded163\
             1a8b605a43ff5bed8086072ba1e7cc2358baeca134c825a7"
        );
        assert_eq!(
            to_hex(&digest(CryptoAlgo::Sha512, b"abc")),
            "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea2\
             0a9eeee64b55d39a2192992a274fc1a836ba3c23a3feebbd\
             454d4423643ce80e2a9ac94fa54ca49f"
        );
    }

    /// Le condensat vide est valide (spec — `digest("")` ne
    /// jette pas).
    #[test]
    fn digest_empty() {
        assert_eq!(digest(CryptoAlgo::Sha256, b"").len(), 32);
        assert_eq!(digest(CryptoAlgo::Sha384, b"").len(), 48);
        assert_eq!(digest(CryptoAlgo::Sha512, b"").len(), 64);
    }
}
