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

// -- `crypto.subtle` ops à clé (JS-C93) ------------------------------------

/// Algorithme à clé normalisé côté moteur — la plateforme
/// ne re-parse jamais de chaîne. Première tranche bornée aux
/// algorithmes couverts par `ring` sans nouvelle dépendance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyAlgo {
    /// `HMAC` — clé `raw`, `sign`/`verify` (tag).
    Hmac(CryptoAlgo),
    /// `ECDSA` `P-256`/`SHA-256` — privée `pkcs8`, publique
    /// `raw` SEC1 non compressée ou `spki`.
    EcdsaP256,
    /// `Ed25519` — privée `pkcs8`, publique `raw`/`spki`.
    Ed25519,
    /// `AES-GCM` (§68 couture FIX-1) — clé `raw` de 128 ou
    /// 256 bits ; ops `encrypt`/`decrypt` via `ring::aead`
    /// (tag GCM 128 bits collé au chiffré, convention
    /// `WebCrypto` `tagLength` par défaut).
    AesGcm,
}

/// Format d'`importKey` — `jwk` non supporté à cette
/// tranche (le moteur le rejette `NotSupportedError` avant
/// émission).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyFormat {
    /// Octets bruts (`HMAC`, clés publiques SEC1).
    Raw,
    /// `PKCS#8` — clé privée.
    Pkcs8,
    /// `SubjectPublicKeyInfo` — clé publique.
    Spki,
}

/// `subtle.importKey` — valide la matière et la rend
/// canonique (les octets retournés sont ce que `sign`/
/// `verify` consommeront). Erreur = `DataError`.
///
/// # Errors
/// `Err` (`DataError`) si la clé est invalide pour
/// `(algo, format)` (pkcs8 malformé, raw de mauvaise
/// taille, combinaison non supportée).
pub fn import_key(algo: KeyAlgo, format: KeyFormat, bytes: &[u8]) -> Result<Vec<u8>, String> {
    match (algo, format) {
        (KeyAlgo::Hmac(_), KeyFormat::Pkcs8 | KeyFormat::Spki) => {
            Err("importKey: HMAC accepts raw only".to_string())
        }
        // `HMAC` accepte n'importe quelle longueur (spec —
        // la clé est normalisée à l'usage par le hash).
        // `spki` ECDSA/Ed25519 : passthrough — la matière est
        // validée à l'usage (`verify`), pas de parse ASN.1 ici.
        (KeyAlgo::Hmac(_), KeyFormat::Raw) | (_, KeyFormat::Spki) => Ok(bytes.to_vec()),
        (KeyAlgo::AesGcm, KeyFormat::Raw) => match bytes.len() {
            16 | 32 => Ok(bytes.to_vec()),
            _ => Err("importKey: invalid raw key (AES-GCM veut 128/256 bits)".to_string()),
        },
        (KeyAlgo::AesGcm, _) => Err("importKey: AES-GCM n'accepte que le format raw".to_string()),
        // Privées : `pkcs8` validé par construction ring.
        (KeyAlgo::EcdsaP256, KeyFormat::Pkcs8) => ring::signature::EcdsaKeyPair::from_pkcs8(
            &ring::signature::ECDSA_P256_SHA256_ASN1_SIGNING,
            bytes,
            &ring::rand::SystemRandom::new(),
        )
        .map(|_| bytes.to_vec())
        .map_err(|_| "importKey: invalid pkcs8 (ECDSA P-256)".to_string()),
        (KeyAlgo::Ed25519, KeyFormat::Pkcs8) => ring::signature::Ed25519KeyPair::from_pkcs8(bytes)
            .map(|_| bytes.to_vec())
            .map_err(|_| "importKey: invalid pkcs8 (Ed25519)".to_string()),
        // Publiques `raw` : point SEC1 non compressé P-256
        // (65 octets, préfixe 0x04) / clé Ed25519 (32 octets) ;
        // `spki` est vérifié à l'usage (`verify`) — borne :
        // pas de parse ASN.1 ici.
        (KeyAlgo::EcdsaP256, KeyFormat::Raw) => {
            if bytes.len() == 65 && bytes.first() == Some(&0x04) {
                Ok(bytes.to_vec())
            } else {
                Err("importKey: invalid raw key (ECDSA P-256 SEC1)".to_string())
            }
        }
        (KeyAlgo::Ed25519, KeyFormat::Raw) => {
            if bytes.len() == 32 {
                Ok(bytes.to_vec())
            } else {
                Err("importKey: invalid raw key (Ed25519)".to_string())
            }
        }
    }
}

/// `subtle.sign` — `key` = octets canoniques retournés par
/// [`import_key`]. Erreur = `OperationError`.
///
/// # Errors
/// `Err` (`OperationError`) si l'algo ne peut pas signer
/// avec cette clé (taille/format incompatible).
pub fn sign(algo: KeyAlgo, key: &[u8], data: &[u8]) -> Result<Vec<u8>, String> {
    match algo {
        KeyAlgo::Hmac(hash) => {
            let a = match hash {
                CryptoAlgo::Sha256 => ring::hmac::HMAC_SHA256,
                CryptoAlgo::Sha384 => ring::hmac::HMAC_SHA384,
                CryptoAlgo::Sha512 => ring::hmac::HMAC_SHA512,
            };
            Ok(ring::hmac::sign(&ring::hmac::Key::new(a, key), data)
                .as_ref()
                .to_vec())
        }
        KeyAlgo::EcdsaP256 => {
            let rng = ring::rand::SystemRandom::new();
            let kp = ring::signature::EcdsaKeyPair::from_pkcs8(
                &ring::signature::ECDSA_P256_SHA256_ASN1_SIGNING,
                key,
                &rng,
            )
            .map_err(|_| "sign: invalid pkcs8 (ECDSA P-256)".to_string())?;
            kp.sign(&rng, data)
                .map(|sig| sig.as_ref().to_vec())
                .map_err(|_| "sign: ECDSA P-256 failed".to_string())
        }
        KeyAlgo::Ed25519 => ring::signature::Ed25519KeyPair::from_pkcs8(key)
            .map(|kp| kp.sign(data).as_ref().to_vec())
            .map_err(|_| "sign: Ed25519 failed".to_string()),
        KeyAlgo::AesGcm => Err("sign: AES-GCM ne signe pas (encrypt/decrypt)".to_string()),
    }
}

/// `subtle.encrypt({name:"AES-GCM", iv, additionalData?,
/// tagLength?}, key, data)` — retourne `chiffré ‖ tag` (tag
/// 128 bits, convention GCM/WebCrypto par défaut ; le
/// moteur tronque si `tagLength` < 128). Nonce **96 bits**
/// exigé par `ring` (spec `WebCrypto` accepte 1–64 octets —
/// les longueurs non-96 exigeraient une dérivation ; rejet
/// honnête `OperationError` plutôt que nonce tronqué).
///
/// # Errors
/// `Err` (`OperationError`) si la clé n'est pas 128/256
/// bits, le nonce ≠ 12 octets, ou le sceau échoue.
pub fn aead_seal(
    algo: KeyAlgo,
    key: &[u8],
    nonce: &[u8],
    aad: &[u8],
    plaintext: &[u8],
) -> Result<Vec<u8>, String> {
    let unbound = aead_unbound(algo, key)?;
    let nonce = ring::aead::Nonce::try_assume_unique_for_key(nonce)
        .map_err(|_| "encrypt: AES-GCM veut un nonce de 96 bits".to_string())?;
    let key = ring::aead::LessSafeKey::new(unbound);
    let mut buf = plaintext.to_vec();
    key.seal_in_place_append_tag(nonce, ring::aead::Aad::from(aad), &mut buf)
        .map_err(|_| "encrypt: AES-GCM seal failed".to_string())?;
    Ok(buf)
}

/// `subtle.decrypt({name:"AES-GCM", iv, additionalData?,
/// tagLength?}, key, data)` — `data` = `chiffré ‖ tag` ;
/// retourne le clair ou `OperationError` (tag invalide =
/// échec d'authentification, pas `false`).
///
/// # Errors
/// `Err` (`OperationError`) si la clé/nonce sont invalides
/// ou le tag ne vérifie pas.
pub fn aead_open(
    algo: KeyAlgo,
    key: &[u8],
    nonce: &[u8],
    aad: &[u8],
    ciphertext: &[u8],
) -> Result<Vec<u8>, String> {
    let unbound = aead_unbound(algo, key)?;
    let nonce = ring::aead::Nonce::try_assume_unique_for_key(nonce)
        .map_err(|_| "decrypt: AES-GCM veut un nonce de 96 bits".to_string())?;
    let key = ring::aead::LessSafeKey::new(unbound);
    let mut buf = ciphertext.to_vec();
    let pt = key
        .open_in_place(nonce, ring::aead::Aad::from(aad), &mut buf)
        .map_err(|_| "decrypt: AES-GCM open failed (tag invalide)".to_string())?;
    Ok(pt.to_vec())
}

/// `ring::aead::UnboundKey` depuis les octets canoniques —
/// l'algo sélectionne 128/256 par taille de clé.
fn aead_unbound(algo: KeyAlgo, key: &[u8]) -> Result<ring::aead::UnboundKey, String> {
    let a = match (algo, key.len()) {
        (KeyAlgo::AesGcm, 16) => &ring::aead::AES_128_GCM,
        (KeyAlgo::AesGcm, 32) => &ring::aead::AES_256_GCM,
        (KeyAlgo::AesGcm, _) => return Err("AES-GCM veut une clé de 128/256 bits".to_string()),
        _ => return Err("AEAD : algorithme non supporté".to_string()),
    };
    ring::aead::UnboundKey::new(a, key).map_err(|_| "AES-GCM : clé invalide".to_string())
}

/// `subtle.verify` — `key` = octets canoniques (publique
/// `raw`/`spki`, ou `pkcs8` privée dont on extrait la
/// partie publique pour `HMAC`... non : HMAC signe ET
/// vérifie avec la même clé secrète). `Ok(true/false)` —
/// un échec de vérification n'est **pas** une erreur
/// (spec : `false`, pas `OperationError`).
///
/// # Errors
/// `Err` (`OperationError`) si la clé est inutilisable pour
/// cet algo (format incompatible, extraction impossible).
pub fn verify(algo: KeyAlgo, key: &[u8], sig: &[u8], data: &[u8]) -> Result<Vec<u8>, String> {
    let ok = match algo {
        KeyAlgo::Hmac(hash) => {
            let a = match hash {
                CryptoAlgo::Sha256 => ring::hmac::HMAC_SHA256,
                CryptoAlgo::Sha384 => ring::hmac::HMAC_SHA384,
                CryptoAlgo::Sha512 => ring::hmac::HMAC_SHA512,
            };
            ring::hmac::verify(&ring::hmac::Key::new(a, key), data, sig).is_ok()
        }
        KeyAlgo::EcdsaP256 => {
            ring::signature::UnparsedPublicKey::new(&ring::signature::ECDSA_P256_SHA256_ASN1, key)
                .verify(data, sig)
                .is_ok()
        }
        KeyAlgo::Ed25519 => ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, key)
            .verify(data, sig)
            .is_ok(),
        KeyAlgo::AesGcm => return Err("verify: AES-GCM ne signe pas".to_string()),
    };
    // Booléen encodé sur un octet — le moteur le décode en
    // `true`/`false` à la livraison (la promesse `verify`
    // résout un booléen, pas un ArrayBuffer).
    Ok(vec![u8::from(ok)])
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

    /// AES-GCM — round-trip + vecteur NIST GCM 128
    /// (clé/IV zéros, clair zéro → tag connu).
    #[test]
    fn aes_gcm_roundtrip_et_vecteur() {
        // NIST GCM spec ex. : key=0^128, iv=0^96, pt=0^128
        // → ct = 0388dace60b6a392f328c2b971b2fe78,
        //   tag = ab6e47d42cec13bdf53a67b21257bddf.
        let key = [0u8; 16];
        let iv = [0u8; 12];
        let pt = [0u8; 16];
        let ct = aead_seal(KeyAlgo::AesGcm, &key, &iv, &[], &pt).expect("seal");
        let to_hex = |b: &[u8]| {
            b.iter().fold(String::new(), |mut s, x| {
                use std::fmt::Write;
                let _ = write!(s, "{x:02x}");
                s
            })
        };
        // NIST GCM : ct ‖ tag attendu (vérifié contre la
        // sortie réelle — le vecteur canonique est respecté).
        let want = "0388dace60b6a392f328c2b971b2fe78ab6e47d42cec13bdf53a67b21257bddf";
        let got = to_hex(&ct);
        assert_eq!(got.len(), want.len(), "longueur ct‖tag");
        assert!(got == want, "vecteur GCM : {got}");
        let back = aead_open(KeyAlgo::AesGcm, &key, &iv, &[], &ct).expect("open");
        assert_eq!(back, pt);
        // Tag corrompu → OperationError, jamais de clair.
        let mut bad = ct.clone();
        let n = bad.len() - 1;
        bad[n] ^= 1;
        assert!(aead_open(KeyAlgo::AesGcm, &key, &iv, &[], &bad).is_err());
        // AAD doit vérifier identique.
        let c2 = aead_seal(KeyAlgo::AesGcm, &key, &iv, b"hdr", b"x").expect("seal aad");
        assert!(aead_open(KeyAlgo::AesGcm, &key, &iv, b"autre", &c2).is_err());
        // Clé 256 bits admise, tailles invalides rejetées.
        assert!(aead_seal(KeyAlgo::AesGcm, &[0u8; 32], &iv, &[], b"x").is_ok());
        assert!(import_key(KeyAlgo::AesGcm, KeyFormat::Raw, &[0u8; 16]).is_ok());
        assert!(import_key(KeyAlgo::AesGcm, KeyFormat::Raw, &[0u8; 24]).is_err());
        assert!(import_key(KeyAlgo::AesGcm, KeyFormat::Pkcs8, &[0u8; 16]).is_err());
    }
}
