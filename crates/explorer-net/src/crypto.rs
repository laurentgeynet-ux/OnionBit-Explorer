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

// -- SRI (CAP-163) -------------------------------------------------------

/// `integrity` d'une sous-ressource (SRI — W3C « Subresource
/// Integrity », §sec « does response match metadataList ») :
/// la valeur d'attribut `integrity` est une liste de
/// `algo-digest` (base64), options `?…` ignorées. Règles
/// spec bornées :
///
/// - attribut vide/absent d'algo reconnu → admis (la métadonnée
///   SRI « vide » n'a jamais force de blocage) ;
/// - au moins un algo reconnu → la réponse doit matcher **un**
///   condensat de la famille la **plus forte** présente
///   (`sha512` > `sha384` > `sha256`) ;
/// - base64 indécodable / condensat vide → ce jeton ne peut pas
///   matcher (il ne bloque pas seul si d'autres matchent) ;
/// - `?options` tronqués proprement ; tolérant aux espaces
///   superflus. Pas de CORS-mode requis à cette tranche (borné).
#[must_use]
pub fn integrity_allows(metadata: &str, body: &[u8]) -> bool {
    // Tokens reconnus : `(algo, b64)` — famille la plus forte
    // présente (rang sha512=2 > sha384=1 > sha256=0).
    let mut cands: Vec<(u8, Vec<u8>)> = Vec::new();
    for tok in metadata.split_whitespace() {
        let tok = tok.split('?').next().unwrap_or_default();
        let Some((algo, b64)) = tok.split_once('-') else {
            continue;
        };
        let rank = match algo.to_ascii_lowercase().as_str() {
            "sha256" => 0,
            "sha384" => 1,
            "sha512" => 2,
            _ => continue,
        };
        if let Some(bytes) = base64_decode(b64.as_bytes()) {
            if !bytes.is_empty() {
                cands.push((rank, bytes));
            }
        }
    }
    if cands.is_empty() {
        return true;
    }
    let best = cands.iter().map(|(r, _)| *r).max().unwrap_or(0);
    let algo = match best {
        2 => CryptoAlgo::Sha512,
        1 => CryptoAlgo::Sha384,
        _ => CryptoAlgo::Sha256,
    };
    let got = digest(algo, body);
    cands.iter().any(|(r, want)| *r == best && *want == got)
}

/// Base64 standard — tolérant : espaces ASCII ignorés,
/// `?options` déjà tronqué en amont. `None` sur alphabet
/// invalide ou padding mal placé.
fn base64_decode(s: &[u8]) -> Option<Vec<u8>> {
    fn val(b: u8) -> Option<u8> {
        match b {
            b'A'..=b'Z' => Some(b - b'A'),
            b'a'..=b'z' => Some(b - b'a' + 26),
            b'0'..=b'9' => Some(b - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let mut acc = 0u32;
    let mut nbits = 0u32;
    let mut pad = false;
    for &b in s {
        if b.is_ascii_whitespace() {
            continue;
        }
        if b == b'=' {
            pad = true;
            continue;
        }
        if pad {
            return None; // donnée après `=` — invalide
        }
        let v = val(b)?;
        acc = (acc << 6) | u32::from(v);
        nbits += 6;
        if nbits >= 8 {
            nbits -= 8;
            // `nbits` < 8 → `acc >> nbits` tient dans u8.
            #[allow(clippy::cast_possible_truncation)]
            out.push((acc >> nbits) as u8);
            acc &= (1 << nbits) - 1;
        }
    }
    Some(out)
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

    /// SRI (CAP-163) — « abc » en sha256/384/512 base64 :
    /// match/mismatch, algo inconnu admis, `?options`,
    /// famille la plus forte gouvernante.
    #[test]
    fn sri_match_mismatch() {
        let good256 = "sha256-ungWv48Bz+pBQUDeXa4iI7ADYaOWF3qctBD/YfIAFa0=";
        let good384 = "sha384-ywB1P0WjXou1oD1pmsZQBycsMqsO3tFjGotgWkP/W+2AhgcroefMI1i67KE0yCWn";
        let good512 = "sha512-3a81oZNherrMQXNJriBBMRLm+k6JqX6iCp7u5ktV05ohkpkqJ0/BqDa6PCOj/uu9RU1EI2Q86A4qmslPpUyknw==";
        assert!(integrity_allows(good256, b"abc"));
        assert!(integrity_allows(good384, b"abc"));
        assert!(integrity_allows(good512, b"abc"));
        assert!(!integrity_allows(good256, b"abd"));
        // Aucun algo reconnu → admis (borne spec).
        assert!(integrity_allows("md5-AAAA", b"abc"));
        assert!(integrity_allows("", b"abc"));
        // `?options` ignorées ; plusieurs jetons : un match
        // de la famille la plus forte suffit.
        assert!(integrity_allows(
            "sha256-AAA?x=y sha256-ungWv48Bz+pBQUDeXa4iI7ADYaOWF3qctBD/YfIAFa0=",
            b"abc"
        ));
        // sha384 présent mais mauvais → le sha256 vrai ne
        // sauve pas (la plus forte famille gouverne).
        assert!(!integrity_allows(
            "sha256-ungWv48Bz+pBQUDeXa4iI7ADYaOWF3qctBD/YfIAFa0= sha384-BAD",
            b"abc"
        ));
        // Base64 invalide → jeton ignoré ; admis si rien de
        // reconnu ne reste, refusé sinon.
        assert!(integrity_allows("sha256-!!!", b"abc"));
        assert!(!integrity_allows("sha256-AAAA", b"abc"));
    }
}
