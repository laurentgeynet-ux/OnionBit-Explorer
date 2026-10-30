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
    /// `AES-GCM` (JS-C160) — clé `raw` de 128 ou
    /// 256 bits ; ops `encrypt`/`decrypt` via `ring::aead`
    /// (tag GCM 128 bits collé au chiffré, convention
    /// `WebCrypto` `tagLength` par défaut).
    AesGcm,
    /// `RSASSA-PKCS1-v1_5` (JS-C214, poool/paywalls) — hash
    /// `{name:"SHA-*"}` obligatoire ; publique `spki`/`jwk`,
    /// privée `pkcs8` ; ops `sign`/`verify` via
    /// `ring::signature` RSA.
    RsaPkcs1(CryptoAlgo),
}

/// Format d'`importKey` — `jwk` couvre `kty:"RSA"` public
/// à cette tranche (JS-C214) ; le moteur a déjà extrait
/// `n`/`e` et les paque `len(n):u16be ‖ n ‖ e` dans `bytes`
/// (l'objet JSON JWK ne traverse pas la couture).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyFormat {
    /// Octets bruts (`HMAC`, clés publiques SEC1).
    Raw,
    /// `PKCS#8` — clé privée.
    Pkcs8,
    /// `SubjectPublicKeyInfo` — clé publique.
    Spki,
    /// `JSON Web Key` — matière prédécoupée côté moteur.
    Jwk,
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
        // RSA `spki` (JS-C214) — déballé **avant** le
        // passthrough générique : la forme canonique est
        // `RSAPublicKey` nu, ce que `RsaParameters` de ring
        // parse à `verify`.
        (KeyAlgo::RsaPkcs1(_), KeyFormat::Spki) => unwrap_spki_rsapub(bytes),
        // `HMAC` accepte n'importe quelle longueur (spec —
        // la clé est normalisée à l'usage par le hash).
        // RSA `pkcs8` — conservé tel quel pour `sign` (un
        // `verify` sur clé privée est rejeté en amont par
        // les usages WebCrypto).
        // `spki` ECDSA/Ed25519 : passthrough — la matière est
        // validée à l'usage (`verify`), pas de parse ASN.1 ici.
        (KeyAlgo::Hmac(_), KeyFormat::Raw)
        | (KeyAlgo::RsaPkcs1(_), KeyFormat::Pkcs8)
        | (_, KeyFormat::Spki) => Ok(bytes.to_vec()),
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
        (KeyAlgo::RsaPkcs1(_), KeyFormat::Jwk) => rsa_pub_der_from_packed_jwk(bytes),
        (KeyAlgo::RsaPkcs1(_), KeyFormat::Raw) => {
            Err("importKey: RSASSA-PKCS1-v1_5 n'accepte que jwk/spki/pkcs8".to_string())
        }
        (_, KeyFormat::Jwk) => {
            Err("importKey: jwk n'est supporté que pour RSA à cette tranche".to_string())
        }
    }
}

// -- RSA : canonique `RSAPublicKey` (JS-C214) ------------------------------
//
// `RsaParameters` de ring parse à `verify` le seul
// `RSAPublicKey ::= SEQUENCE { modulus INTEGER,
// publicExponent INTEGER }` (RFC 8017) — pas l'enveloppe
// `SubjectPublicKeyInfo` (RFC 5280). Forme canonique
// retenue : le `RSAPublicKey` DER nu. `jwk` le reconstruit,
// `spki` le déballe — parseur DER minimal et borné.

/// `n‖e` JWK empaquetés côté moteur (`len(n):u16be ‖ n ‖ e`)
/// → `RSAPublicKey` DER.
fn rsa_pub_der_from_packed_jwk(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let n_len = u16::from_be_bytes([
        *bytes.first().ok_or("jwk: trame vide")?,
        *bytes.get(1).ok_or("jwk: trame tronquée")?,
    ]) as usize;
    let n = bytes.get(2..2 + n_len).ok_or("jwk: modulus tronqué")?;
    let e = bytes.get(2 + n_len..).ok_or("jwk: exposant absent")?;
    if n.is_empty() || e.is_empty() || n.len() > 1024 || e.len() > 8 {
        // Borne : modulus ≤ 8192 bits (ring) ; `e` tient dans
        // 64 bits en pratique (3/65537 couvre le web).
        return Err("jwk: composants RSA hors bornes".to_string());
    }
    let mut out = Vec::with_capacity(n.len() + e.len() + 8);
    der_seq(&[der_int(n), der_int(e)].concat(), &mut out);
    Ok(out)
}

/// `SubjectPublicKeyInfo` DER → contenu du `BIT STRING`
/// (`RSAPublicKey`). Tolérant au parse minimal : enveloppe
/// `SEQ{ SEQ{…}, BITSTRING{…} }`, longueurs courtes/longues,
/// rien d'autre — pas de validation d'OID (un `rsaEncryption`
/// ou paramètres absents passent ; la clé est revérifiée à
/// l'usage par `RsaParameters`).
fn unwrap_spki_rsapub(spki: &[u8]) -> Result<Vec<u8>, String> {
    fn bad() -> String {
        "importKey: invalid spki (RSA)".to_string()
    }
    let (tag, outer, rest) = der_tlv(spki).ok_or_else(bad)?;
    if tag != 0x30 || !rest.is_empty() {
        return Err(bad());
    }
    let (tag, _alg, rest2) = der_tlv(outer).ok_or_else(bad)?;
    if tag != 0x30 {
        return Err(bad());
    }
    let (tag, bitstr, _r) = der_tlv(rest2).ok_or_else(bad)?;
    if tag != 0x03 || bitstr.first() != Some(&0x00) {
        return Err(bad());
    }
    let inner = bitstr.get(1..).ok_or_else(bad)?;
    Ok(inner.to_vec())
}

/// Un TLV DER → `(tag, contenu, reste)` ; `None` sur trame
/// tronquée ou longueur malformée.
fn der_tlv(d: &[u8]) -> Option<(u8, &[u8], &[u8])> {
    let tag = *d.first()?;
    let b1 = usize::from(*d.get(1)?);
    let (hlen, len) = if b1 & 0x80 == 0 {
        (2usize, b1)
    } else {
        let n = b1 & 0x7f;
        if n == 0 || n > 3 {
            return None; // forme indéfinie / >16 Mio : hors propos
        }
        let mut v = 0usize;
        for i in 0..n {
            v = (v << 8) | usize::from(*d.get(2 + i)?);
        }
        (2 + n, v)
    };
    let end = hlen.checked_add(len)?;
    Some((tag, d.get(hlen..end)?, d.get(end..)?))
}

/// Écrit `SEQUENCE { content }` en DER dans `out`.
fn der_seq(content: &[u8], out: &mut Vec<u8>) {
    out.push(0x30);
    der_len(content.len(), out);
    out.extend_from_slice(content);
}

/// Longueur DER (forme courte < 128, longue sinon — un
/// modulus 2048 bits produit ~256 octets → 2 octets de
/// longueur, jamais plus ici).
fn der_len(len: usize, out: &mut Vec<u8>) {
    // `len < 0x80` tient dans `u8`, `len ≤ 0xffff` dans
    // `u16` — les branches garantissent le domaine.
    #[allow(clippy::cast_possible_truncation)]
    if len < 0x80 {
        out.push(len as u8);
    } else if len <= 0xffff {
        out.push(0x82);
        out.extend_from_slice(&(len as u16).to_be_bytes());
    } else {
        out.push(0x83);
        out.extend_from_slice(&len.to_be_bytes()[5..]);
    }
}

/// `INTEGER` DER — l'entier non signé est préfixé d'un
/// octet `0x00` quand son bit de poids fort est levé
/// (sinon lu négatif — un modulus RSA lève presque
/// toujours le sien).
fn der_int(be: &[u8]) -> Vec<u8> {
    let first_nonzero = be.iter().position(|&b| b != 0).unwrap_or(be.len());
    let v = &be[first_nonzero.min(be.len().saturating_sub(1))..];
    let pad = usize::from(v.first().is_some_and(|b| b & 0x80 != 0));
    let mut out = Vec::with_capacity(v.len() + 3);
    out.push(0x02);
    der_len(v.len() + pad, &mut out);
    out.extend(std::iter::repeat_n(0, pad));
    out.extend_from_slice(v);
    out
}

/// `subtle.encrypt` AES-GCM — `nonce` 96 bits, `aad` libre ;
/// retourne `chiffré ‖ tag` (128 bits — ring).
///
/// # Errors
/// `Err` (`OperationError`) si clé ≠ 128/256, nonce ≠ 12
/// octets, ou sceau échoué.
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

/// `subtle.decrypt` AES-GCM — `data` = `chiffré ‖ tag` ;
/// retourne le clair ou `OperationError` (tag invalide =
/// échec d'authentification).
///
/// # Errors
/// `Err` (`OperationError`) si clé/nonce invalide ou tag faux.
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
/// 128/256 sélectionnés par taille de clé.
fn aead_unbound(algo: KeyAlgo, key: &[u8]) -> Result<ring::aead::UnboundKey, String> {
    let a = match (algo, key.len()) {
        (KeyAlgo::AesGcm, 16) => &ring::aead::AES_128_GCM,
        (KeyAlgo::AesGcm, 32) => &ring::aead::AES_256_GCM,
        (KeyAlgo::AesGcm, _) => return Err("AES-GCM veut une clé de 128/256 bits".to_string()),
        _ => return Err("AEAD : algorithme non supporté".to_string()),
    };
    ring::aead::UnboundKey::new(a, key).map_err(|_| "AES-GCM : clé invalide".to_string())
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
        KeyAlgo::RsaPkcs1(hash) => {
            let kp = ring::signature::RsaKeyPair::from_pkcs8(key)
                .map_err(|_| "sign: invalid pkcs8 (RSA)".to_string())?;
            let pad = match hash {
                CryptoAlgo::Sha256 => &ring::signature::RSA_PKCS1_SHA256,
                CryptoAlgo::Sha384 => &ring::signature::RSA_PKCS1_SHA384,
                CryptoAlgo::Sha512 => &ring::signature::RSA_PKCS1_SHA512,
            };
            let mut sig = vec![0u8; kp.public().modulus_len()];
            kp.sign(pad, &ring::rand::SystemRandom::new(), data, &mut sig)
                .map(|()| sig)
                .map_err(|_| "sign: RSASSA-PKCS1-v1_5 failed".to_string())
        }
    }
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
        KeyAlgo::RsaPkcs1(hash) => {
            let params = match hash {
                CryptoAlgo::Sha256 => &ring::signature::RSA_PKCS1_2048_8192_SHA256,
                CryptoAlgo::Sha384 => &ring::signature::RSA_PKCS1_2048_8192_SHA384,
                CryptoAlgo::Sha512 => &ring::signature::RSA_PKCS1_2048_8192_SHA512,
            };
            ring::signature::UnparsedPublicKey::new(params, key)
                .verify(data, sig)
                .is_ok()
        }
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

    /// RSA-PKCS1 (JS-C214) — fixture réelle générée par
    /// `System.Security.Cryptography.RSA` (2048, SHA-256) :
    /// le JWK empaqueté doit rendre le SPKI **octet-exact**
    /// et `verify` accepter la vraie signature, refuser une
    /// signature corrompue ou un message altéré.
    #[test]
    #[allow(clippy::too_many_lines, clippy::cast_possible_truncation)]
    fn rsa_jwk_spki_canonical_et_verify() {
        fn b64url(s: &str) -> Vec<u8> {
            let mut acc = 0u32;
            let mut nb = 0u32;
            let mut out = Vec::new();
            for &b in s.as_bytes() {
                let v = match b {
                    b'A'..=b'Z' => b - b'A',
                    b'a'..=b'z' => b - b'a' + 26,
                    b'0'..=b'9' => b - b'0' + 52,
                    b'-' => 62,
                    b'_' => 63,
                    _ => panic!("hors alphabet"),
                };
                acc = (acc << 6) | u32::from(v);
                nb += 6;
                if nb >= 8 {
                    nb -= 8;
                    out.push((acc >> nb) as u8);
                    acc &= (1 << nb) - 1;
                }
            }
            out
        }
        fn unhex(s: &str) -> Vec<u8> {
            (0..s.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("hex"))
                .collect()
        }
        let n = b64url(
            "2iewfMZgOuTGqnPqzW6Lzh5iJyh056_kRbdlp-I8MdirTxmOlGKAhlDm3lsr4wENIA7388\
             yfE7kYpJ32eScorS11lIowIIfdosDHpajaJkqsopENr6tffDJAegYI8gwBqreHnAkDGhpucRW\
             vto-5Ig2V9jKafiMeKpNGR0Cf9XhBj52nFJ_wRz-iNkvhqLPIU09abfjGUPvVb1BaISGA7hJB\
             u97Bj1R7q1Kx5WQRpiR7Io2YW1aa_d4xsZhnuh783MC7WJKGNLHgjFvOm_Rjy35VbzMCdasp4\
             yBKUYQuGf2SzM6WPYJnDGPso1Eqxo7Rw7nFqz8SwFzr-wY1nLaHDQ",
        );
        let e = b64url("AQAB");
        assert_eq!(n.len(), 256);
        let mut packed = Vec::new();
        packed.extend_from_slice(&(n.len() as u16).to_be_bytes());
        packed.extend_from_slice(&n);
        packed.extend_from_slice(&e);
        let key = import_key(
            KeyAlgo::RsaPkcs1(CryptoAlgo::Sha256),
            KeyFormat::Jwk,
            &packed,
        )
        .expect("import jwk");
        // Canonique = `RSAPublicKey` DER nu (= le contenu du
        // BIT STRING du SPKI .NET — ce que `RsaParameters`
        // de ring parse à `verify`).
        let rsapub_want = unhex(
            "3082010A0282010100DA27B07CC6603AE4C6AA73EACD6E8BCE1E62272874E7AFE445B765A7E23C31\
             D8AB4F198E9462808650E6DE5B2BE3010D200EF7F3CC9F13B918A49DF6792728AD2D75948A302087DD\
             A2C0C7A5A8DA264AACA2910DAFAB5F7C32407A0608F20C01AAB7879C09031A1A6E7115AFB68FB9220D\
             95F6329A7E231E2A934647409FF578418F9DA7149FF0473FA2364BE1A8B3C8534F5A6DF8C650FBD56F\
             505A212180EE1241BBDEC18F547BAB52B1E56411A6247B228D985B569AFDDE31B19867BA1EFCDCC0BB\
             58928634B1E08C5BCE9BF463CB7E556F330275AB29E3204A51842E19FD92CCCE963D82670C63ECA351\
             2AC68ED1C3B9C5AB3F12C05CEBFB06359CB6870D0203010001",
        );
        assert_eq!(key, rsapub_want, "RSAPublicKey DER canonique");
        // `spki` → même canonique (enveloppe déballée).
        let spki_dotnet = unhex(
            "30820122300D06092A864886F70D01010105000382010F003082010A0282010100DA27B07CC660\
             3AE4C6AA73EACD6E8BCE1E62272874E7AFE445B765A7E23C31D8AB4F198E9462808650E6DE5B2BE3\
             010D200EF7F3CC9F13B918A49DF6792728AD2D75948A302087DDA2C0C7A5A8DA264AACA2910DAFAB\
             5F7C32407A0608F20C01AAB7879C09031A1A6E7115AFB68FB9220D95F6329A7E231E2A934647409F\
             F578418F9DA7149FF0473FA2364BE1A8B3C8534F5A6DF8C650FBD56F505A212180EE1241BBDEC18F\
             547BAB52B1E56411A6247B228D985B569AFDDE31B19867BA1EFCDCC0BB58928634B1E08C5BCE9BF4\
             63CB7E556F330275AB29E3204A51842E19FD92CCCE963D82670C63ECA3512AC68ED1C3B9C5AB3F12\
             C05CEBFB06359CB6870D0203010001",
        );
        let key2 = import_key(
            KeyAlgo::RsaPkcs1(CryptoAlgo::Sha256),
            KeyFormat::Spki,
            &spki_dotnet,
        )
        .expect("import spki");
        assert_eq!(key2, rsapub_want, "spki déballé → RSAPublicKey");
        let sig = unhex(
            "2D30623EC7B23DC6A6C97D8269B08C8AC7E000CC0D58B34D5D71701C9AC86198C07C66B2FF727B\
             FC96321B8DC29AA7700D9E3929C44A1B293ED1E32D5EF0F4D7A5FF5AA4AC94385FDBAA5DF02040AE\
             D385E2F2A85212A32ED444ACEEFEEE264E6B6439741F4993166A65DD83BA99B90E47D8AAEEBD4AFE\
             1B2FACDD42DC3AD3E0021D6F119CEC7B920BC5CA96DC371D05EE08E4A22645027A94F9B17820B467\
             0A48B3227A6AA1B4B7EAAD3089A166AA7AC86A5AD7A1BC1959CF4FF653B6C4268C5942748A402975\
             02346A278CDCBF34AB9A0C5B27D40D83BF5D4B112DEEEF5DE45FFC56BA77851C812B6E4FE799D21E\
             5C8B8F011BB68DAC08BBE54D0443572AFC",
        );
        let msg = b"poool-license-test";
        assert_eq!(
            verify(KeyAlgo::RsaPkcs1(CryptoAlgo::Sha256), &key, &sig, msg).expect("verify"),
            vec![1]
        );
        // Signature corrompue → `false`, jamais d'erreur.
        let mut bad = sig.clone();
        bad[10] ^= 1;
        assert_eq!(
            verify(KeyAlgo::RsaPkcs1(CryptoAlgo::Sha256), &key, &bad, msg).expect("verify bad"),
            vec![0]
        );
        assert_eq!(
            verify(KeyAlgo::RsaPkcs1(CryptoAlgo::Sha256), &key, &sig, b"autre").expect("verify"),
            vec![0]
        );
        // Rejets honnêtes : trame tronquée, modulus borné.
        assert!(import_key(
            KeyAlgo::RsaPkcs1(CryptoAlgo::Sha256),
            KeyFormat::Jwk,
            &[0, 4, 1]
        )
        .is_err());
        assert!(import_key(KeyAlgo::RsaPkcs1(CryptoAlgo::Sha256), KeyFormat::Raw, &n).is_err());
        assert!(import_key(KeyAlgo::Hmac(CryptoAlgo::Sha256), KeyFormat::Jwk, &packed).is_err());
    }
}
