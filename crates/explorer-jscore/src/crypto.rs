// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! `crypto.subtle` (JS-C79) — **couture embedder**, jamais de
//! code crypto dans le moteur.
//!
//! `explorer-jscore` est autonome zéro-dépendance (promesse de
//! mission, ADR-0007) : le moteur ne sait pas hacher. Il émet
//! une [`CryptoRequest`] abstraite vers le [`CryptoTransport`]
//! injecté par l'embedder — même forme que [`crate::xhr::XhrTransport`]
//! — et résout la `Promise` à la livraison de la
//! [`CryptoResponse`]. La plateforme exécute (`explorer-net`
//! via `ring`, déjà présent pour TLS — ADR-0002) ; la même
//! couture sert le FFI Flutter.
//!
//! Surface v1 (déclencheur : PKCE `code_challenge` S256 des
//! logins `OAuth2`), étendue par tranches site (JS-C93
//! HMAC/ECDSA/Ed25519, JS-C160 AES-GCM, JS-C214 RSA-PKCS1,
//! JS-C231 RSA-OAEP) :
//! - `crypto.subtle.digest(algorithm, data)` → `Promise`
//!   résolvant un `ArrayBuffer` — `SHA-1`/`SHA-256`/`SHA-384`/
//!   `SHA-512` (noms normalisés casse-insensible, forme
//!   `'SHA-256'` ou `{name}` — `AlgorithmIdentifier` spec).
//! - Algorithme inconnu → `Promise` rejetée `NotSupportedError`
//!   (spec `NormalizeAlgorithm` ; shape `DOMException`
//!   `{name,message}` — approximation documentée, idem IDB).
//! - `data` non-`BufferSource` / `algorithm` non convertible →
//!   `TypeError` **synchrone** (échec de conversion `WebIDL`,
//!   avant la promesse).
//! - Sans transport installé → `digest` calcule en repli natif
//!   (`sha2.rs`, JS-C453/JS-C562) ; les ops à clé rejettent
//!   `NotSupportedError` (dégradation honnête : la page voit un
//!   backend absent, jamais une fausse clé).
//! - Échec plateforme (`Err` côté transport) → `OperationError`.
//!
//! JS-C562 (§B-117 — odysee/aliexpress/cdiscount/deliveroo) :
//! `SHA-1` en `digest`, `AES-CBC` (`importKey`/`encrypt`/
//! `decrypt` PKCS#7), `generateKey` symétrique (AES-GCM/AES-CBC/
//! HMAC — aléa plateforme, jamais le PRNG moteur), `deriveBits`/
//! `deriveKey` (`PBKDF2`/`HKDF` → cibles symétriques) et
//! `exportKey` (`raw`/`jwk` `oct`/`pkcs8`/`spki`). Bornes
//! honnêtes : clés asymétriques `generateKey`, `wrapKey`/
//! `unwrapKey`, `ECDH`/`AES-KW` → `NotSupportedError`.

use std::rc::Rc;

use crate::builtins::arg;
use crate::heap::JsResult;
use crate::value::{ObjId, Value};
use crate::Engine;

/// Opération crypto demandée par le script — transmise à
/// l'embedder via [`CryptoTransport`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CryptoOp {
    /// `crypto.subtle.digest(algorithm, data)` — l'algorithme
    /// est normalisé côté moteur avant émission.
    Digest(DigestAlgo),
    /// `crypto.subtle.importKey(format, keyData, …)` (JS-C93)
    /// — la matière est dans `CryptoRequest::data`, le
    /// résultat = octets canoniques de la clé (le moteur les
    /// remballe en objet `CryptoKey`).
    ImportKey {
        /// Format normalisé (`raw`/`pkcs8`/`spki` — `jwk`
        /// rejeté `NotSupportedError` avant émission, borne).
        format: ImportFormat,
        /// Algorithme à clé normalisé.
        algo: KeyAlgo,
    },
    /// `crypto.subtle.sign(algorithm, key, data)` — octets de
    /// la clé dans `CryptoRequest::key` (copiés hors du
    /// `CryptoKey` à l'appel), données dans `data`.
    Sign {
        /// Algorithme normalisé (doit coller à `key.algorithm`,
        /// validé côté moteur — `InvalidAccessError` sinon).
        algo: KeyAlgo,
    },
    /// `crypto.subtle.verify(algorithm, key, signature, data)`
    /// — clé dans `key`, signature dans `data`, message dans
    /// `CryptoRequest::msg`. Résultat = un octet `0`/`1` que
    /// le moteur décode en booléen (spec : `verify` résout un
    /// booléen, jamais un `ArrayBuffer`).
    Verify {
        /// Algorithme normalisé.
        algo: KeyAlgo,
        /// Signature à vérifier (copiée hors heap).
        sig: Vec<u8>,
    },
    /// `crypto.subtle.encrypt({name:"AES-GCM",iv,additionalData?,
    /// tagLength?}, key, data)` — clair dans `data`, résultat
    /// `chiffré ‖ tag` (128 bits à cette tranche — ring).
    Encrypt {
        /// Algorithme normalisé (AES-GCM uniquement ici).
        algo: KeyAlgo,
        /// Nonce 96 bits copié hors heap.
        iv: Vec<u8>,
        /// AAD optionnelle (`additionalData`).
        aad: Vec<u8>,
    },
    /// `crypto.subtle.decrypt(…)` — mêmes paramètres ; `data`
    /// = `chiffré ‖ tag`, le résultat est le clair.
    Decrypt {
        /// Algorithme normalisé.
        algo: KeyAlgo,
        /// Nonce copié.
        iv: Vec<u8>,
        /// AAD copiée.
        aad: Vec<u8>,
    },
    /// `crypto.subtle.generateKey(algorithm, extractable,
    /// usages)` (JS-C562, deliveroo §B-117) — tranche
    /// symétrique : la plateforme tire `bits` aléatoires
    /// (`ring::rand`, jamais le PRNG moteur) et les valide
    /// comme matière canonique — mêmes octets qu'un
    /// `importKey raw`, remballés en `CryptoKey secret`.
    GenerateKey {
        /// Algorithme cible (AES-GCM/AES-CBC/HMAC à cette
        /// tranche — les paires RSA/EC restent
        /// `NotSupportedError`).
        algo: KeyAlgo,
        /// `length` de l'init en bits (0 = défaut de
        /// l'algo — HMAC = bloc du hash).
        bits: u32,
    },
    /// `crypto.subtle.deriveBits(params, baseKey, length)`
    /// — octets de la clé de base dans `CryptoRequest::key`,
    /// résultat = `ArrayBuffer` des bits dérivés.
    DeriveBits {
        /// KDF normalisée + paramètres copiés.
        params: DeriveParams,
        /// `length` demandée (bits, multiple de 8).
        bits: u32,
    },
    /// `crypto.subtle.deriveKey(params, baseKey,
    /// derivedKeyType, extractable, usages)` — la plateforme
    /// dérive `bits` puis les valide comme clé `target`
    /// (canonique `raw`) ; le moteur remballe en `CryptoKey`
    /// via le rail `ImportKey`.
    DeriveKey {
        /// KDF normalisée + paramètres copiés.
        params: DeriveParams,
        /// Algorithme de la clé dérivée (AES-GCM/AES-CBC/
        /// HMAC à cette tranche).
        target: KeyAlgo,
        /// Longueur dérivée en bits (0 = défaut `target` —
        /// HMAC = bloc du hash).
        bits: u32,
    },
}

/// Algorithme à clé normalisé (JS-C93) — couvert par `ring`
/// côté plateforme : pas de parse de chaîne hors du moteur.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyAlgo {
    /// `HMAC` — `{name:"HMAC", hash, length?}`.
    Hmac {
        /// Algorithme de hachage sous-jacent.
        hash: DigestAlgo,
    },
    /// `ECDSA` `{name:"ECDSA", namedCurve:"P-256"}` — hash fixe
    /// SHA-256 à cette tranche (borné).
    EcdsaP256,
    /// `Ed25519`.
    Ed25519,
    /// `AES-GCM` — `{name:"AES-GCM", length}` ; seal/open
    /// `ring::aead` côté plateforme (JS-C160, marinetraffic).
    AesGcm,
    /// `RSASSA-PKCS1-v1_5` — `{name, hash}` (JS-C214,
    /// poool/paywalls : vérification de licence signée).
    RsaPkcs1 {
        /// Hash du padding PKCS#1 v1.5.
        hash: DigestAlgo,
    },
    /// `RSA-OAEP` — `{name, hash}` (JS-C231, poool wamiz
    /// vol 344 : déchiffrement du jeton de licence —
    /// padding OAEP+MGF1 au hash, via le crate `rsa`
    /// plateforme — `ring` ne chiffre pas RSA).
    RsaOaep {
        /// Hash du padding OAEP (label optionnel porté
        /// par l'op, pas la clé).
        hash: DigestAlgo,
    },
    /// `AES-CBC` — `{name:"AES-CBC", length}` (JS-C562,
    /// aliexpress §B-117) ; padding PKCS#7 spec `WebCrypto` —
    /// crates `aes`+`cbc` côté plateforme (`ring` ne fait
    /// pas CBC).
    AesCbc,
    /// `PBKDF2` — clé de base `raw` pour `deriveBits`/
    /// `deriveKey` (JS-C562) ; aucune autre op.
    Pbkdf2,
    /// `HKDF` — idem (clé de base `raw` = IKM).
    Hkdf,
}

/// Paramètres `deriveBits`/`deriveKey` normalisés côté
/// moteur (JS-C562) — `salt`/`info` copiés hors heap ; la
/// plateforme ignore les champs hors de sa KDF.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeriveParams {
    /// KDF + hash de l'init.
    pub algo: DeriveAlgo,
    /// `salt` (PBKDF2/HKDF).
    pub salt: Vec<u8>,
    /// `info` (HKDF — vide sinon).
    pub info: Vec<u8>,
    /// `iterations` (PBKDF2 — 0 sinon).
    pub iterations: u32,
}

/// KDF normalisée (`PBKDF2`/`HKDF` + `hash` — `ECDH` reste
/// `NotSupportedError` à cette tranche).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeriveAlgo {
    /// `PBKDF2` `{name, salt, iterations, hash}`.
    Pbkdf2 {
        /// Hash de la PRF.
        hash: DigestAlgo,
    },
    /// `HKDF` `{name, salt, info, hash}`.
    Hkdf {
        /// Hash de l'extract/expand.
        hash: DigestAlgo,
    },
}

/// Format d'`importKey` (JS-C93) — `jwk` couvre les clés
/// publiques RSA à cette tranche (JS-C214) : le moteur
/// décode `n`/`e` base64url et les transmet empaquetés.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportFormat {
    /// `raw` — octets bruts.
    Raw,
    /// `pkcs8` — clé privée PKCS#8.
    Pkcs8,
    /// `spki` — clé publique `SubjectPublicKeyInfo`.
    Spki,
    /// `jwk` — `{kty:"RSA", n, e}` (composants pré-découpés).
    Jwk,
}

/// Algorithme `digest` normalisé (`WebCrypto` — les noms
/// `'SHA-*'` sont validés par le moteur, la plateforme ne
/// re-parse jamais de chaîne).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DigestAlgo {
    /// `SHA-1` — héritage web (§B-117, JS-C562) :
    /// `SHA1_FOR_LEGACY_USE_ONLY` côté plateforme.
    Sha1,
    /// `SHA-256` — PKCE `S256`, SRI, `JWT`-adjacent.
    Sha256,
    /// `SHA-384`.
    Sha384,
    /// `SHA-512`.
    Sha512,
}

/// Requête vers le fournisseur crypto de la plateforme.
#[derive(Debug, Clone)]
pub struct CryptoRequest {
    /// Opération normalisée.
    pub op: CryptoOp,
    /// Octets source — **copiés** hors du buffer JS à l'appel
    /// (spec : `CopyBytes` au moment de `digest()` — la
    /// plateforme ne voit jamais d'objet du heap).
    pub data: Vec<u8>,
    /// Octets de la `CryptoKey` pour `sign`/`verify` (JS-C93)
    /// — copiés hors de l'objet à l'appel ; vide sinon.
    pub key: Vec<u8>,
}

/// Résultat — l'embedder le réinjecte par
/// [`Engine::deliver_crypto_response`].
#[derive(Debug, Clone)]
pub struct CryptoResponse {
    /// Handle écho de [`CryptoTransport::submit`].
    pub handle: u64,
    /// `Ok(octets résultat)` ou `Err(message)` → rejet
    /// `OperationError` (shape `DOMException`).
    pub result: Result<Vec<u8>, String>,
}

/// Fournisseur crypto de la plateforme — object-safe ; le
/// moteur appelle `submit` en synchrone à `subtle.digest()`,
/// le transport doit enfiler et rendre un handle
/// immédiatement (non bloquant), la réponse arrivant par
/// [`Engine::deliver_crypto_response`].
pub trait CryptoTransport {
    /// Enfile l'opération ; retourne le handle attendu en
    /// [`CryptoResponse::handle`].
    fn submit(&self, req: CryptoRequest) -> u64;
}

/// État crypto document-local (JS-C78) — les opérations en
/// vol appartiennent au contexte qui les a soumises : la
/// livraison localise le propriétaire par handle et switch,
/// comme `deliver_xhr_response`.
#[derive(Default)]
pub(crate) struct CryptoState {
    /// `(handle, promise ObjId, forme du résultat)` en vol
    /// dans ce contexte — la livraison sait quoi résoudre
    /// (`ArrayBuffer` / booléen / `CryptoKey`).
    pub pending: Vec<(u64, ObjId, PendingKind)>,
    /// `CryptoKey.prototype` paresseux (JS-C93).
    pub ck_proto: Option<ObjId>,
}

/// Forme de résolution d'une opération en vol (JS-C93) —
/// sert à `deliver_crypto_response`.
pub(crate) enum PendingKind {
    /// Octets → `ArrayBuffer` (`digest`, `sign`).
    Bytes,
    /// Un octet `0`/`1` → booléen (`verify`).
    Bool,
    /// Octets canoniques → objet `CryptoKey` (`importKey`).
    ImportKey {
        /// `secret`/`private`/`public` (prop `type`).
        key_type: &'static str,
        /// Nom d'algo normalisé (prop `algorithm.name`).
        algo_name: &'static str,
        /// `usages` normalisés.
        usages: Vec<String>,
        /// `extractable` de l'init.
        extractable: bool,
        /// Algorithme complet de la clé (JS-C93 — `sign`/
        /// `verify` l'utilisent quand l'argument ne porte
        /// que le nom, ex. `sign('HMAC', key)`).
        algo: KeyAlgo,
    },
}

/// `CryptoKey` — objet opaque côté moteur : la matière vit
/// en `key` (octets canoniques validés par la plateforme à
/// l'`importKey`) et ne ressort **que** copiée vers la
/// couture au `sign`/`verify`/`exportKey`.
struct CryptoKeyHost {
    /// Octets canoniques de la clé.
    key: Vec<u8>,
    /// `secret`/`private`/`public`.
    key_type: &'static str,
    /// Nom d'algo normalisé (`HMAC`/`ECDSA`/`Ed25519`).
    algo_name: &'static str,
    /// Algorithme complet de la clé (hash inclus pour HMAC)
    /// — `sign("HMAC", key)` lit l'algo ICI (spec : l'argument
    /// `algorithm` ne porte que `name` pour la vérification).
    algo: KeyAlgo,
    /// Usages normalisés (`sign`/`verify`/…).
    usages: Vec<String>,
    /// `extractable` de l'init.
    extractable: bool,
}

impl crate::heap::Host for CryptoKeyHost {
    fn class_name(&self) -> &'static str {
        "CryptoKey"
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn get(&self, e: &mut Engine, _t: Value, key: &str) -> JsResult<Option<Value>> {
        Ok(Some(match key {
            "type" => return Ok(Some(e.str_val(self.key_type))),
            "extractable" => Value::Bool(self.extractable),
            "algorithm" => {
                let o = e.new_object();
                let n = e.str_val(self.algo_name);
                e.set_own(o, "name", n, true);
                return Ok(Some(Value::Obj(o)));
            }
            "usages" => {
                let items: Vec<Value> = self.usages.iter().map(|u| e.str_val(u.clone())).collect();
                return Ok(Some(e.new_array(items)));
            }
            _ => return Ok(None),
        }))
    }
}

/// Erreur shape `DOMException` `{name, message}` (même
/// approximation qu'IDB/`WebSocket` — `.name` est le champ
/// que les apps testent).
fn dom_exc(e: &mut Engine, name: &str, msg: &str) -> Value {
    let o = e.new_object();
    let n = e.str_val(name);
    e.set_own(o, "name", n, true);
    let m = e.str_val(msg);
    e.set_own(o, "message", m, true);
    Value::Obj(o)
}

/// `algorithm` → [`DigestAlgo`] — `'SHA-256'`/`'sha-256'`/
/// `{name:'SHA-256'}` acceptés (`AlgorithmIdentifier` =
/// `object | DOMString`, noms case-insensitive spec).
/// `None` = algorithme non supporté → `NotSupportedError`.
fn digest_algo(e: &mut Engine, v: Value) -> JsResult<Option<DigestAlgo>> {
    let name = match v {
        Value::Obj(id) => match e.get_prop(id, "name") {
            Some(n) => e.to_js_string(n)?,
            None => e.to_js_string(v)?,
        },
        _ => e.to_js_string(v)?,
    };
    Ok(match name.to_ascii_uppercase().replace('-', "").as_str() {
        "SHA1" => Some(DigestAlgo::Sha1),
        "SHA256" => Some(DigestAlgo::Sha256),
        "SHA384" => Some(DigestAlgo::Sha384),
        "SHA512" => Some(DigestAlgo::Sha512),
        _ => None,
    })
}

/// `crypto.subtle.digest(algorithm, data)` — conversions
/// `WebIDL` synchrones (`TypeError` sur arguments non
/// convertibles), le reste est asynchrone : la `Promise`
/// rejette `NotSupportedError`/`OperationError`.
fn subtle_digest(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let algo_v = arg(args, 0);
    let data_v = arg(args, 1);
    // Conversion BufferSource synchrone : `undefined`/objet
    // non-vu → TypeError *avant* la promesse (WebIDL).
    let Value::Obj(_) = data_v else {
        return Err(e.type_error("subtle.digest: data must be a BufferSource"));
    };
    let Some(data) = crate::typed_array::bytes_of_value(e, data_v) else {
        return Err(e.type_error("subtle.digest: data must be a BufferSource"));
    };
    let pid = e.new_promise();
    match digest_algo(e, algo_v)? {
        None => {
            let err = dom_exc(e, "NotSupportedError", "subtle.digest: unknown algorithm");
            e.reject_promise(pid, err);
        }
        Some(algo) => {
            let transport = e.crypto_transport.clone();
            match transport {
                // JS-C453 — repli SHA-2 natif (zéro-dep,
                // §B-117/odysee : « SubtleCrypto not
                // available » au bootstrap) : sans backend
                // injecté, `digest` calcule en local les trois
                // algos admis au lieu de rejeter.
                None => {
                    let bytes = match algo {
                        DigestAlgo::Sha1 => crate::sha2::sha1(&data).to_vec(),
                        DigestAlgo::Sha256 => crate::sha2::sha256(&data).to_vec(),
                        DigestAlgo::Sha384 => crate::sha2::sha384(&data),
                        DigestAlgo::Sha512 => crate::sha2::sha512(&data),
                    };
                    let ab = crate::typed_array::new_array_buffer(e, bytes);
                    e.resolve_promise(pid, ab);
                }
                Some(tr) => {
                    let h = tr.submit(CryptoRequest {
                        op: CryptoOp::Digest(algo),
                        data,
                        key: Vec::new(),
                    });
                    e.crypto.pending.push((h, pid, PendingKind::Bytes));
                    e.sched_tick("req.crypto");
                }
            }
        }
    }
    Ok(Value::Obj(pid))
}

/// `algorithm` → [`KeyAlgo`] (JS-C93) : `HMAC`
/// (`{name,hash[,length]}` — hash obligatoire à cette
/// tranche), `ECDSA` `{name,namedCurve:"P-256"}` uniquement,
/// `Ed25519` `{name:"Ed25519"|"Ed25519"}` (cf. draft `WebCrypto`
/// `NODE-ED25519`/`Ed25519` — les deux casse-courants admis).
fn key_algo(e: &mut Engine, v: Value) -> JsResult<Option<(KeyAlgo, &'static str)>> {
    let name = match v {
        Value::Obj(id) => match e.get_prop(id, "name") {
            Some(n) => e.to_js_string(n)?,
            None => e.to_js_string(v.clone())?,
        },
        _ => e.to_js_string(v.clone())?,
    };
    // `-` et `_` ignorés (`RSASSA-PKCS1-v1_5` → `RSASSAPKCS1V15`).
    Ok(
        match name.to_ascii_uppercase().replace(['-', '_'], "").as_str() {
            "HMAC" => {
                let hash = if let Value::Obj(id) = v {
                    let hv = e.get_prop(id, "hash").unwrap_or(Value::Undefined);
                    digest_algo(e, hv)?
                } else {
                    None
                };
                hash.map(|h| (KeyAlgo::Hmac { hash: h }, "HMAC"))
            }
            "ECDSA" => {
                let curve_ok = if let Value::Obj(id) = v {
                    e.get_prop(id, "namedCurve")
                        .as_ref()
                        .and_then(Engine::heap_string_of)
                        .is_some_and(|c| c.eq_ignore_ascii_case("P-256"))
                } else {
                    false
                };
                curve_ok.then_some((KeyAlgo::EcdsaP256, "ECDSA"))
            }
            "ED25519" | "NODEED25519" => Some((KeyAlgo::Ed25519, "Ed25519")),
            // `RSASSA-PKCS1-v1_5` (JS-C214) — hash requis à
            // l'`importKey` (spec `RsaHashedImportParams`) ;
            // aux ops `sign`/`verify` l'algo ne porte souvent que
            // le nom → repli SHA-256 (le hash de la CLÉ gouverne
            // de toute façon la signature réelle).
            "RSASSAPKCS1V15" | "RSASSAPKCS1" => {
                let hash = if let Value::Obj(id) = v {
                    let hv = e.get_prop(id, "hash").unwrap_or(Value::Undefined);
                    digest_algo(e, hv)?.unwrap_or(DigestAlgo::Sha256)
                } else {
                    DigestAlgo::Sha256
                };
                Some((KeyAlgo::RsaPkcs1 { hash }, "RSASSA-PKCS1-v1_5"))
            }
            // `RSA-OAEP` (JS-C231) — `{name, hash}` requis à
            // l'`importKey` (même `RsaHashedImportParams`) ;
            // aux `encrypt`/`decrypt` l'algo porte `label`
            // (lu par la branche dédiée, pas ici).
            "RSAOAEP" => {
                let hash = if let Value::Obj(id) = v {
                    let hv = e.get_prop(id, "hash").unwrap_or(Value::Undefined);
                    digest_algo(e, hv)?.unwrap_or(DigestAlgo::Sha256)
                } else {
                    DigestAlgo::Sha256
                };
                Some((KeyAlgo::RsaOaep { hash }, "RSA-OAEP"))
            }
            // `AES-GCM`/`AES-CBC` — `length` acceptée sans
            // validation de borne ici : `generateKey` rejette
            // hors {128,192,256} en `OperationError` spec et
            // l'`importKey` laisse la plateforme valider la
            // taille réelle de la matière.
            "AESGCM" | "AESCBC" => {
                let len_ok = if let Value::Obj(id) = v {
                    match e.get_prop(id, "length") {
                        Some(Value::Num(n)) => n > 0.0 && n.fract() == 0.0,
                        Some(_) => false,
                        None => true,
                    }
                } else {
                    true
                };
                if name.to_ascii_uppercase().replace(['-', '_'], "") == "AESCBC" {
                    len_ok.then_some((KeyAlgo::AesCbc, "AES-CBC"))
                } else {
                    len_ok.then_some((KeyAlgo::AesGcm, "AES-GCM"))
                }
            }
            // `PBKDF2`/`HKDF` (JS-C562) — clés de base `raw`
            // pour `deriveBits`/`deriveKey` ; l'algo complet
            // (hash/salt/iterations) vit dans les params de
            // l'op, pas dans la clé.
            "PBKDF2" => Some((KeyAlgo::Pbkdf2, "PBKDF2")),
            "HKDF" => Some((KeyAlgo::Hkdf, "HKDF")),
            _ => None,
        },
    )
}

/// `usages` de l'init — tableau de chaînes normalisé (spec :
/// la liste permise dépend de l'algo/type ; la vérification
/// fine vit côté plateforme — ici on borne aux noms connus).
fn key_usages(e: &mut Engine, v: Value) -> JsResult<Vec<String>> {
    let mut out = Vec::new();
    if let Ok(iter) = e.iter_values(v) {
        for u in iter {
            let s = e.to_js_string(u)?;
            if matches!(
                s.as_str(),
                "sign"
                    | "verify"
                    | "encrypt"
                    | "decrypt"
                    | "deriveKey"
                    | "deriveBits"
                    | "wrapKey"
                    | "unwrapKey"
            ) {
                out.push(s);
            }
        }
    }
    Ok(out)
}

/// `keyData` JWK `{kty:"RSA", n, e}` → `len(n):u16be ‖
/// n ‖ e` empaqueté pour la couture — la plateforme
/// reconstruit le `SubjectPublicKeyInfo` DER (JS-C214).
/// `None` → `DataError` côté appelant : objet non-RSA,
/// clé privée (`d` présent — hors tranche), composant
/// absent ou base64url invalide.
fn pack_rsa_jwk(e: &mut Engine, v: Value) -> JsResult<Option<Vec<u8>>> {
    let Value::Obj(id) = v else {
        return Ok(None);
    };
    let kty = match e.get_prop(id, "kty") {
        Some(x) => e.to_js_string(x)?,
        None => String::new(),
    };
    if !kty.eq_ignore_ascii_case("rsa") || e.get_prop(id, "d").is_some() {
        return Ok(None);
    }
    let (Some(nv), Some(ev)) = (e.get_prop(id, "n"), e.get_prop(id, "e")) else {
        return Ok(None);
    };
    let Some(n) = b64url_decode(&e.to_js_string(nv)?) else {
        return Ok(None);
    };
    let Some(x) = b64url_decode(&e.to_js_string(ev)?) else {
        return Ok(None);
    };
    if n.is_empty() || n.len() > u16::MAX as usize {
        return Ok(None);
    }
    let mut out = Vec::with_capacity(n.len() + x.len() + 2);
    out.extend_from_slice(&(n.len() as u16).to_be_bytes());
    out.extend_from_slice(&n);
    out.extend_from_slice(&x);
    Ok(Some(out))
}

/// Base64**url** (`-`/`_`, padding `=` toléré, pas
/// d'espace) — alphabet JWK (RFC 7515 §2). `None` sur
/// caractère hors alphabet.
fn b64url_decode(s: &str) -> Option<Vec<u8>> {
    let mut acc = 0u32;
    let mut nbits = 0u32;
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    for &b in s.as_bytes() {
        if b == b'=' {
            break;
        }
        let v = match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'-' => 62,
            b'_' => 63,
            _ => return None,
        };
        acc = (acc << 6) | u32::from(v);
        nbits += 6;
        if nbits >= 8 {
            nbits -= 8;
            out.push((acc >> nbits) as u8);
            acc &= (1 << nbits) - 1;
        }
    }
    Some(out)
}

/// `CryptoKey` → octets + nom d'algo — `None` si l'objet
/// Pièces extraites d'une `CryptoKey` (clé brute, `type`,
/// nom d'algo, algo résolu, usages, `extractable`).
type CryptoKeyParts = (
    Vec<u8>,
    &'static str,
    &'static str,
    KeyAlgo,
    Vec<String>,
    bool,
);

/// n'est pas une `CryptoKey` (`TypeError` `WebIDL`).
fn crypto_key_of(e: &Engine, v: Value) -> Option<CryptoKeyParts> {
    let Value::Obj(id) = v else { return None };
    match e.heap.obj(id).map(|o| &o.kind) {
        Some(crate::heap::ObjKind::Host(h)) => {
            h.clone().as_any().downcast_ref::<CryptoKeyHost>().map(|k| {
                (
                    k.key.clone(),
                    k.key_type,
                    k.algo_name,
                    k.algo,
                    k.usages.clone(),
                    k.extractable,
                )
            })
        }
        _ => None,
    }
}

/// Description courte d'un argument non-`CryptoKey` — même
/// rail que le nommage de receveur JS-C228 : constructeur,
/// `@@toStringTag`, puis propres clés (un JWK brut passé à
/// `decrypt` se lit `Object{kty,n,e}` — vol 344 wamiz).
fn crypto_arg_desc(e: &mut Engine, v: Value) -> String {
    match v {
        Value::Obj(id) => {
            let desc = e
                .get_prop(id, "constructor")
                .and_then(|c| e.get_prop_value(c, "name").ok())
                .as_ref()
                .and_then(Engine::heap_string_of)
                .filter(|n| !n.is_empty())
                .unwrap_or_else(|| "Object".to_string());
            if desc != "Object" {
                return desc;
            }
            if let Some(t) = e
                .get_prop(id, crate::interp::SYM_TO_STRING_TAG)
                .as_ref()
                .and_then(Engine::heap_string_of)
                .filter(|t| !t.is_empty())
            {
                return format!("Object@@{t}");
            }
            let keys = e.own_keys(id);
            let shown: Vec<&str> = keys.iter().take(6).map(String::as_str).collect();
            if shown.is_empty() {
                desc
            } else {
                format!("Object{{{}}}", shown.join(","))
            }
        }
        _ => format!("{v:?}"),
    }
}

/// Nom d'algorithme seul (`"HMAC"` ou `{name:"HMAC"}`) — la
/// vérification `sign`/`verify` se fait sur le NOM : les
/// détails (hash, courbe) viennent de la clé (spec `WebCrypto`
/// — `sign(algorithm, key)` n'exige que `name`).
fn algo_name_of(e: &mut Engine, v: Value) -> JsResult<Option<String>> {
    let name = match v {
        Value::Obj(id) => match e.get_prop(id, "name") {
            Some(n) => e.to_js_string(n)?,
            None => e.to_js_string(v)?,
        },
        _ => e.to_js_string(v)?,
    };
    Ok(Some(name))
}

/// `crypto.subtle.importKey(format, keyData, algorithm,
/// extractable, usages)` (JS-C93) — conversions synchrones
/// `WebIDL` (`TypeError` sur `BufferSource`/format invalide) ;
/// le reste est asynchrone : `NotSupportedError` (format
/// `jwk`, algo inconnu, transport absent), `DataError`
/// (matière refusée par la plateforme), `SyntaxError`
/// (usage hors liste).
fn subtle_import_key(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let format_s = e.to_js_string(arg(args, 0))?;
    let format = match format_s.to_ascii_lowercase().as_str() {
        "raw" => ImportFormat::Raw,
        "pkcs8" => ImportFormat::Pkcs8,
        "spki" => ImportFormat::Spki,
        "jwk" => ImportFormat::Jwk,
        _ => {
            let pid = e.new_promise();
            let err = dom_exc(e, "TypeError", "subtle.importKey: unknown format");
            e.reject_promise(pid, err);
            return Ok(Value::Obj(pid));
        }
    };
    // `keyData` : `BufferSource` sauf `jwk` — objet
    // `{kty,n,e}` (JS-C214). JWK privé (`d` présent) rejeté
    // honnêtement — seules les clés publiques RSA entrent.
    let data = if format == ImportFormat::Jwk {
        if let Some(d) = pack_rsa_jwk(e, arg(args, 1))? {
            d
        } else {
            let pid = e.new_promise();
            let err = dom_exc(
                e,
                "DataError",
                "subtle.importKey: invalid jwk (RSA {kty,n,e} attendu)",
            );
            e.reject_promise(pid, err);
            return Ok(Value::Obj(pid));
        }
    } else {
        let Some(d) = crate::typed_array::bytes_of_value(e, arg(args, 1)) else {
            return Err(e.type_error("subtle.importKey: keyData must be a BufferSource"));
        };
        d
    };
    let algo_v = arg(args, 2);
    let extractable = e.to_boolean(arg(args, 3));
    let usages = key_usages(e, arg(args, 4))?;
    let pid = e.new_promise();
    match key_algo(e, algo_v.clone())? {
        None => {
            // Nom reçu dans le message (même rail que JS-C228)
            // — un `unknown algorithm` sans le nom demandé ne
            // permet pas de nommer le bras en vol (wamiz poool
            // vol 344 : l'algo réel restait inconnu).
            let asked = match algo_v {
                Value::Obj(id) => match e.get_prop(id, "name") {
                    Some(n) => e.to_js_string(n)?,
                    None => e.to_js_string(algo_v)?,
                },
                _ => e.to_js_string(algo_v)?,
            };
            let err = dom_exc(
                e,
                "NotSupportedError",
                &format!("subtle.importKey: unknown algorithm (« {asked} »)"),
            );
            e.reject_promise(pid, err);
        }
        Some((algo, algo_name)) => {
            // `jwk` n'entre que pour RSA publique (JS-C214) —
            // les autres algos/locations sont rejetés
            // `NotSupportedError` sans émission au transport.
            if format == ImportFormat::Jwk
                && !matches!(algo, KeyAlgo::RsaPkcs1 { .. } | KeyAlgo::RsaOaep { .. })
            {
                let err = dom_exc(
                    e,
                    "NotSupportedError",
                    "subtle.importKey: jwk n'est supporté que pour RSASSA-PKCS1-v1_5",
                );
                e.reject_promise(pid, err);
                return Ok(Value::Obj(pid));
            }
            let key_type: &'static str = match format {
                ImportFormat::Raw => {
                    if matches!(
                        algo,
                        KeyAlgo::Hmac { .. }
                            | KeyAlgo::AesGcm
                            | KeyAlgo::AesCbc
                            | KeyAlgo::Pbkdf2
                            | KeyAlgo::Hkdf
                    ) {
                        "secret"
                    } else {
                        "public"
                    }
                }
                ImportFormat::Pkcs8 => "private",
                ImportFormat::Spki | ImportFormat::Jwk => "public",
            };
            match e.crypto_transport.clone() {
                None => {
                    let err = dom_exc(
                        e,
                        "NotSupportedError",
                        "subtle.importKey: no crypto backend installed",
                    );
                    e.reject_promise(pid, err);
                }
                Some(tr) => {
                    let h = tr.submit(CryptoRequest {
                        op: CryptoOp::ImportKey { format, algo },
                        data,
                        key: Vec::new(),
                    });
                    e.crypto.pending.push((
                        h,
                        pid,
                        PendingKind::ImportKey {
                            key_type,
                            algo_name,
                            usages,
                            extractable,
                            algo,
                        },
                    ));
                    e.sched_tick("req.crypto");
                }
            }
        }
    }
    Ok(Value::Obj(pid))
}

/// `crypto.subtle.sign(algorithm, key, data)` (JS-C93) —
/// la clé doit être une `CryptoKey` `private`/`secret`
/// portant `sign` dans ses usages et l'algo demandé ;
/// écarts → `InvalidAccessError` async (spec).
fn subtle_sign(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let algo_v = arg(args, 0);
    let key_v = arg(args, 1);
    let Some(data) = crate::typed_array::bytes_of_value(e, arg(args, 2)) else {
        return Err(e.type_error("subtle.sign: data must be a BufferSource"));
    };
    let pid = e.new_promise();
    let Some((key_bytes, key_type, algo_name, key_algo, usages, _)) =
        crypto_key_of(e, key_v.clone())
    else {
        let desc = crypto_arg_desc(e, key_v);
        let err = dom_exc(
            e,
            "InvalidAccessError",
            &format!("subtle.sign: not a CryptoKey (« {desc} »)"),
        );
        e.reject_promise(pid, err);
        return Ok(Value::Obj(pid));
    };
    // Spec : l'argument `algorithm` n'exige que `name` — le
    // reste (hash/courbe) vient de la clé.
    let name_ok = algo_name_of(e, algo_v)?.is_some_and(|n| {
        n.to_ascii_uppercase().replace('-', "") == algo_name.to_ascii_uppercase().replace('-', "")
    });
    if !(name_ok && matches!(key_type, "private" | "secret") && usages.iter().any(|u| u == "sign"))
    {
        let err = dom_exc(
            e,
            "InvalidAccessError",
            "subtle.sign: key algorithm/usages mismatch",
        );
        e.reject_promise(pid, err);
        return Ok(Value::Obj(pid));
    }
    match e.crypto_transport.clone() {
        None => {
            let err = dom_exc(
                e,
                "NotSupportedError",
                "subtle.sign: no crypto backend installed",
            );
            e.reject_promise(pid, err);
        }
        Some(tr) => {
            let h = tr.submit(CryptoRequest {
                op: CryptoOp::Sign { algo: key_algo },
                data,
                key: key_bytes,
            });
            e.crypto.pending.push((h, pid, PendingKind::Bytes));
            // N1.5 — rail crypto dans le tally : une réponse
            // jamais livrée = promesse pendue (le programme BG
            // signe son attestation juste avant `V`).
            e.sched_tick("req.crypto");
        }
    }
    Ok(Value::Obj(pid))
}

/// `crypto.subtle.verify(algorithm, key, signature, data)`
/// (JS-C93) — `public`/`secret` portant `verify` ; résout
/// un booléen (jamais `OperationError` sur signature fausse).
fn subtle_verify(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let algo_v = arg(args, 0);
    let key_v = arg(args, 1);
    let Some(sig) = crate::typed_array::bytes_of_value(e, arg(args, 2)) else {
        return Err(e.type_error("subtle.verify: signature must be a BufferSource"));
    };
    let Some(data) = crate::typed_array::bytes_of_value(e, arg(args, 3)) else {
        return Err(e.type_error("subtle.verify: data must be a BufferSource"));
    };
    let pid = e.new_promise();
    let Some((key_bytes, key_type, algo_name, key_algo, usages, _)) =
        crypto_key_of(e, key_v.clone())
    else {
        let desc = crypto_arg_desc(e, key_v);
        let err = dom_exc(
            e,
            "InvalidAccessError",
            &format!("subtle.verify: not a CryptoKey (« {desc} »)"),
        );
        e.reject_promise(pid, err);
        return Ok(Value::Obj(pid));
    };
    let name_ok = algo_name_of(e, algo_v)?.is_some_and(|n| {
        n.to_ascii_uppercase().replace('-', "") == algo_name.to_ascii_uppercase().replace('-', "")
    });
    if !(name_ok && matches!(key_type, "public" | "secret") && usages.iter().any(|u| u == "verify"))
    {
        let err = dom_exc(
            e,
            "InvalidAccessError",
            "subtle.verify: key algorithm/usages mismatch",
        );
        e.reject_promise(pid, err);
        return Ok(Value::Obj(pid));
    }
    match e.crypto_transport.clone() {
        None => {
            let err = dom_exc(
                e,
                "NotSupportedError",
                "subtle.verify: no crypto backend installed",
            );
            e.reject_promise(pid, err);
        }
        Some(tr) => {
            let h = tr.submit(CryptoRequest {
                op: CryptoOp::Verify {
                    algo: key_algo,
                    sig,
                },
                data,
                key: key_bytes,
            });
            e.crypto.pending.push((h, pid, PendingKind::Bool));
            e.sched_tick("req.crypto");
        }
    }
    Ok(Value::Obj(pid))
}

// -- JS-C562 — §B-117 : generateKey / derive* / exportKey -----------------

/// Nom lisible de l'`algorithm` demandé — même rail que le
/// nommage `unknown algorithm` de JS-C230 (`« algo »` dans
/// le message, jamais anonyme).
fn algo_asked(e: &mut Engine, v: Value) -> JsResult<String> {
    Ok(match v {
        Value::Obj(id) => match e.get_prop(id, "name") {
            Some(n) => e.to_js_string(n)?,
            None => e.to_js_string(v)?,
        },
        _ => e.to_js_string(v)?,
    })
}

/// Prop numérique `length` (bits) d'un objet params —
/// `None` si absente ou non numérique.
fn length_bits(e: &mut Engine, v: &Value) -> Option<u32> {
    let Value::Obj(id) = v else { return None };
    match e.get_prop(*id, "length") {
        Some(Value::Num(n)) if n > 0.0 && n % 8.0 == 0.0 && n <= f64::from(u32::MAX) =>
        {
            #[allow(clippy::cast_possible_truncation)]
            Some(n as u32)
        }
        _ => None,
    }
}

/// `crypto.subtle.generateKey(algorithm, extractable,
/// usages)` (JS-C562, deliveroo §B-117) — tranche
/// **symétrique** : `AES-GCM`/`AES-CBC` `{name,length}`
/// (`length` ∈ {128,192,256} requis — `OperationError`
/// spec) et `HMAC` `{name,hash,length?}` (absent → bloc du
/// hash côté plateforme). Paires RSA/EC/Ed et KDF →
/// `NotSupportedError` honnête (keygen asymétrique hors
/// tranche). L'aléa est tiré par la plateforme
/// (`ring::rand`) — jamais le PRNG moteur non crypto.
fn subtle_generate_key(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let algo_v = arg(args, 0);
    let extractable = e.to_boolean(arg(args, 1));
    let usages = key_usages(e, arg(args, 2))?;
    let pid = e.new_promise();
    let Some((algo, algo_name)) = key_algo(e, algo_v.clone())? else {
        let asked = algo_asked(e, algo_v)?;
        let err = dom_exc(
            e,
            "NotSupportedError",
            &format!("subtle.generateKey: unknown algorithm (« {asked} »)"),
        );
        e.reject_promise(pid, err);
        return Ok(Value::Obj(pid));
    };
    let bits = match algo {
        KeyAlgo::AesGcm | KeyAlgo::AesCbc => {
            if let Some(b @ (128 | 192 | 256)) = length_bits(e, &algo_v) {
                b
            } else {
                let err = dom_exc(
                    e,
                    "OperationError",
                    "subtle.generateKey: AES requires length ∈ {128,192,256}",
                );
                e.reject_promise(pid, err);
                return Ok(Value::Obj(pid));
            }
        }
        // `length` optionnelle — 0 = bloc du hash (défaut
        // plateforme, spec `HmacKeyGenParams`).
        KeyAlgo::Hmac { .. } => length_bits(e, &algo_v).unwrap_or(0),
        _ => {
            let err = dom_exc(
                e,
                "NotSupportedError",
                "subtle.generateKey: algorithme non couvert à cette tranche (symétrique seul)",
            );
            e.reject_promise(pid, err);
            return Ok(Value::Obj(pid));
        }
    };
    match e.crypto_transport.clone() {
        None => {
            let err = dom_exc(
                e,
                "NotSupportedError",
                "subtle.generateKey: no crypto backend installed",
            );
            e.reject_promise(pid, err);
        }
        Some(tr) => {
            let h = tr.submit(CryptoRequest {
                op: CryptoOp::GenerateKey { algo, bits },
                data: Vec::new(),
                key: Vec::new(),
            });
            e.crypto.pending.push((
                h,
                pid,
                PendingKind::ImportKey {
                    key_type: "secret",
                    algo_name,
                    usages,
                    extractable,
                    algo,
                },
            ));
            e.sched_tick("req.crypto");
        }
    }
    Ok(Value::Obj(pid))
}

/// `{name:"PBKDF2"|"HKDF", salt, info?, iterations?, hash}`
/// → [`DeriveParams`] (JS-C562). Conversions spec :
/// `salt`/`info` non-`BufferSource` et `iterations` absent
/// ou ≤ 0 → `TypeError` **synchrone** ; hash/KDF non
/// supportés → `None` → `NotSupportedError` async.
fn derive_params(e: &mut Engine, v: Value) -> JsResult<Option<DeriveParams>> {
    let name = algo_name_of(e, v.clone())?.unwrap_or_default();
    let norm = name.to_ascii_uppercase().replace('-', "");
    if norm != "PBKDF2" && norm != "HKDF" {
        return Ok(None);
    }
    let Value::Obj(id) = v else {
        return Err(e.type_error("subtle.derive: algorithm must be an object"));
    };
    let hash = match e.get_prop(id, "hash") {
        Some(hv) => digest_algo(e, hv)?,
        None => None,
    };
    let Some(hash) = hash else {
        return Ok(None);
    };
    let Some(salt) = e
        .get_prop(id, "salt")
        .and_then(|sv| crate::typed_array::bytes_of_value(e, sv))
    else {
        return Err(e.type_error("subtle.derive: salt must be a BufferSource"));
    };
    let params = if norm == "PBKDF2" {
        let it = match e.get_prop(id, "iterations") {
            Some(Value::Num(n)) if n >= 1.0 && n <= f64::from(u32::MAX) =>
            {
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                Some(n as u32)
            }
            _ => None,
        };
        let Some(iterations) = it else {
            return Err(e.type_error("subtle.derive: iterations must be a positive number"));
        };
        DeriveParams {
            algo: DeriveAlgo::Pbkdf2 { hash },
            salt,
            info: Vec::new(),
            iterations,
        }
    } else {
        let Some(info) = e
            .get_prop(id, "info")
            .and_then(|iv| crate::typed_array::bytes_of_value(e, iv))
        else {
            return Err(e.type_error("subtle.derive: info must be a BufferSource"));
        };
        DeriveParams {
            algo: DeriveAlgo::Hkdf { hash },
            salt,
            info,
            iterations: 0,
        }
    };
    Ok(Some(params))
}

/// Clé de base KDF validée — octets + accord nom/usage ;
/// l'écart rejette `pid` en `InvalidAccessError` async et
/// rend `None` (spec : jamais de throw synchrone ici).
fn derive_base_key(
    e: &mut Engine,
    key_v: Value,
    kdf_name: &'static str,
    op: &'static str,
    pid: ObjId,
) -> JsResult<Option<Vec<u8>>> {
    let Some((key_bytes, key_type, algo_name, _ka, usages, _)) = crypto_key_of(e, key_v.clone())
    else {
        let desc = crypto_arg_desc(e, key_v);
        let err = dom_exc(
            e,
            "InvalidAccessError",
            &format!("subtle.{op}: not a CryptoKey (« {desc} »)"),
        );
        e.reject_promise(pid, err);
        return Ok(None);
    };
    if !(key_type == "secret"
        && algo_name == kdf_name
        && usages.iter().any(|u| u == "deriveBits" || u == "deriveKey"))
    {
        let err = dom_exc(
            e,
            "InvalidAccessError",
            "subtle.derive: key algorithm/usages mismatch",
        );
        e.reject_promise(pid, err);
        return Ok(None);
    }
    Ok(Some(key_bytes))
}

/// `crypto.subtle.deriveBits(algorithm, baseKey, length)`
/// (JS-C562) — `PBKDF2`/`HKDF` sur clé de base `secret`
/// portant `deriveBits`/`deriveKey` ; résout un
/// `ArrayBuffer` de `length` bits.
fn subtle_derive_bits(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let algo_v = arg(args, 0);
    let key_v = arg(args, 1);
    let length_v = arg(args, 2);
    let params = derive_params(e, algo_v.clone())?;
    let pid = e.new_promise();
    let Some(params) = params else {
        let asked = algo_asked(e, algo_v)?;
        let err = dom_exc(
            e,
            "NotSupportedError",
            &format!("subtle.deriveBits: unknown algorithm (« {asked} »)"),
        );
        e.reject_promise(pid, err);
        return Ok(Value::Obj(pid));
    };
    // Spec : `length` non nul et multiple de 8 → sinon
    // `OperationError` async (la conversion WebIDL a déjà
    // accepté tout `unsigned long`).
    let bits = match length_v {
        Value::Num(n) if n > 0.0 && n % 8.0 == 0.0 && n <= f64::from(u32::MAX) =>
        {
            #[allow(clippy::cast_possible_truncation)]
            Some(n as u32)
        }
        Value::Num(_) => None,
        _ => return Err(e.type_error("subtle.deriveBits: length must be a number")),
    };
    let Some(bits) = bits else {
        let err = dom_exc(
            e,
            "OperationError",
            "subtle.deriveBits: length must be a non-zero multiple of 8",
        );
        e.reject_promise(pid, err);
        return Ok(Value::Obj(pid));
    };
    let kdf_name = match params.algo {
        DeriveAlgo::Pbkdf2 { .. } => "PBKDF2",
        DeriveAlgo::Hkdf { .. } => "HKDF",
    };
    let Some(key_bytes) = derive_base_key(e, key_v, kdf_name, "deriveBits", pid)? else {
        return Ok(Value::Obj(pid));
    };
    match e.crypto_transport.clone() {
        None => {
            let err = dom_exc(
                e,
                "NotSupportedError",
                "subtle.deriveBits: no crypto backend installed",
            );
            e.reject_promise(pid, err);
        }
        Some(tr) => {
            let h = tr.submit(CryptoRequest {
                op: CryptoOp::DeriveBits { params, bits },
                data: Vec::new(),
                key: key_bytes,
            });
            e.crypto.pending.push((h, pid, PendingKind::Bytes));
            e.sched_tick("req.crypto");
        }
    }
    Ok(Value::Obj(pid))
}

/// `crypto.subtle.deriveKey(algorithm, baseKey,
/// derivedKeyType, extractable, usages)` (JS-C562) —
/// cibles symétriques (`AES-GCM`/`AES-CBC` `length` requis,
/// `HMAC` `length` optionnelle) : la plateforme dérive
/// puis valide la matière — résolution `CryptoKey secret`
/// par le rail `ImportKey`.
fn subtle_derive_key(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let algo_v = arg(args, 0);
    let key_v = arg(args, 1);
    let target_v = arg(args, 2);
    let extractable = e.to_boolean(arg(args, 3));
    let usages = key_usages(e, arg(args, 4))?;
    let params = derive_params(e, algo_v.clone())?;
    let pid = e.new_promise();
    let Some(params) = params else {
        let asked = algo_asked(e, algo_v)?;
        let err = dom_exc(
            e,
            "NotSupportedError",
            &format!("subtle.deriveKey: unknown algorithm (« {asked} »)"),
        );
        e.reject_promise(pid, err);
        return Ok(Value::Obj(pid));
    };
    let Some((target, algo_name)) = key_algo(e, target_v.clone())? else {
        let asked = algo_asked(e, target_v)?;
        let err = dom_exc(
            e,
            "NotSupportedError",
            &format!("subtle.deriveKey: unknown derived key type (« {asked} »)"),
        );
        e.reject_promise(pid, err);
        return Ok(Value::Obj(pid));
    };
    let bits = match target {
        KeyAlgo::AesGcm | KeyAlgo::AesCbc => {
            if let Some(b @ (128 | 192 | 256)) = length_bits(e, &target_v) {
                b
            } else {
                let err = dom_exc(
                    e,
                    "OperationError",
                    "subtle.deriveKey: AES requires length ∈ {128,192,256}",
                );
                e.reject_promise(pid, err);
                return Ok(Value::Obj(pid));
            }
        }
        KeyAlgo::Hmac { .. } => length_bits(e, &target_v).unwrap_or(0),
        _ => {
            let err = dom_exc(
                e,
                "NotSupportedError",
                "subtle.deriveKey: cible non couverte à cette tranche (symétrique seule)",
            );
            e.reject_promise(pid, err);
            return Ok(Value::Obj(pid));
        }
    };
    let kdf_name = match params.algo {
        DeriveAlgo::Pbkdf2 { .. } => "PBKDF2",
        DeriveAlgo::Hkdf { .. } => "HKDF",
    };
    let Some(key_bytes) = derive_base_key(e, key_v, kdf_name, "deriveKey", pid)? else {
        return Ok(Value::Obj(pid));
    };
    match e.crypto_transport.clone() {
        None => {
            let err = dom_exc(
                e,
                "NotSupportedError",
                "subtle.deriveKey: no crypto backend installed",
            );
            e.reject_promise(pid, err);
        }
        Some(tr) => {
            let h = tr.submit(CryptoRequest {
                op: CryptoOp::DeriveKey {
                    params,
                    target,
                    bits,
                },
                data: Vec::new(),
                key: key_bytes,
            });
            e.crypto.pending.push((
                h,
                pid,
                PendingKind::ImportKey {
                    key_type: "secret",
                    algo_name,
                    usages,
                    extractable,
                    algo: target,
                },
            ));
            e.sched_tick("req.crypto");
        }
    }
    Ok(Value::Obj(pid))
}

/// Base64**url** encodé (`-`/`_`, sans padding) —
/// complément de [`b64url_decode`] pour `exportKey jwk`.
fn b64url_encode(b: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(b.len().div_ceil(3) * 4);
    for chunk in b.chunks(3) {
        // 3 octets → 4 sextets, tronqué au reste (padding
        // `=` absent en base64url).
        let n24 = u32::from(chunk[0]) << 16
            | u32::from(*chunk.get(1).unwrap_or(&0)) << 8
            | u32::from(*chunk.get(2).unwrap_or(&0));
        let nchars = match chunk.len() {
            1 => 2,
            2 => 3,
            _ => 4,
        };
        for i in 0..nchars {
            let s = 18 - 6 * i;
            out.push(char::from(A[((n24 >> s) & 63) as usize]));
        }
    }
    out
}

/// `RSAPublicKey` DER canonique → `(n, e)` bruts — parseur
/// TLV minimal borné (`exportKey jwk` RSA, JS-C562) ;
/// `None` sur trame non conforme.
fn rsapub_components(der: &[u8]) -> Option<(Vec<u8>, Vec<u8>)> {
    fn tlv(d: &[u8], want: u8) -> Option<(&[u8], &[u8])> {
        if *d.first()? != want {
            return None;
        }
        let b1 = usize::from(*d.get(1)?);
        let (hlen, len) = if b1 & 0x80 == 0 {
            (2usize, b1)
        } else {
            let n = b1 & 0x7f;
            if n == 0 || n > 3 {
                return None;
            }
            let mut v = 0usize;
            for i in 0..n {
                v = (v << 8) | usize::from(*d.get(2 + i)?);
            }
            (2 + n, v)
        };
        Some((d.get(hlen..hlen.checked_add(len)?)?, d.get(hlen + len..)?))
    }
    let (seq, _) = tlv(der, 0x30)?;
    let (n, rest) = tlv(seq, 0x02)?;
    let (e, _) = tlv(rest, 0x02)?;
    // INTEGER signé : l'octet `0x00` de tête (bit de poids
    // fort levé) n'est pas partie de la valeur — JWK veut
    // l'entier brut non signé.
    let strip = |v: &[u8]| -> Vec<u8> {
        if v.is_empty() {
            return Vec::new();
        }
        let p = v.iter().position(|&b| b != 0).unwrap_or(v.len() - 1);
        v[p..].to_vec()
    };
    Some((strip(n), strip(e)))
}

/// Construit l'objet JWK de sortie `exportKey` —
/// `{kty, …, ext, key_ops}` (props spec minimales).
fn jwk_object(e: &mut Engine, kty: &str, fields: &[(&str, Value)]) -> Value {
    let o = e.new_object();
    let k = e.str_val(kty.to_string());
    e.set_own(o, "kty", k, true);
    for (name, v) in fields {
        e.set_own(o, name, v.clone(), true);
    }
    e.set_own(o, "ext", Value::Bool(true), true);
    Value::Obj(o)
}

/// `crypto.subtle.exportKey(format, key)` (JS-C562) —
/// **pure moteur** : les octets canoniques vivent dans la
/// `CryptoKey`, aucune émission plateforme. Formes
/// couvertes : `raw` (clés `secret` + publiques SEC1
/// non compressée), `jwk` (`oct` + `RSA` publique),
/// `pkcs8` (privées — canonique = pkcs8), `spki`
/// (publiques : RSA re-enveloppé `RSAPublicKey`→`spki`,
/// EC/Ed25519 canonique `spki` ou SEC1/`raw` re-enveloppé
/// préfixe fixe). `extractable:false` →
/// `InvalidAccessError` ; format/forme hors tranche →
/// `NotSupportedError` honnête.
fn subtle_export_key(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let format_s = e.to_js_string(arg(args, 0))?;
    let key_v = arg(args, 1);
    let pid = e.new_promise();
    let Some((key_bytes, key_type, _an, key_algo, usages, extractable)) =
        crypto_key_of(e, key_v.clone())
    else {
        let desc = crypto_arg_desc(e, key_v);
        let err = dom_exc(
            e,
            "InvalidAccessError",
            &format!("subtle.exportKey: not a CryptoKey (« {desc} »)"),
        );
        e.reject_promise(pid, err);
        return Ok(Value::Obj(pid));
    };
    if !extractable {
        let err = dom_exc(
            e,
            "InvalidAccessError",
            "subtle.exportKey: key is not extractable",
        );
        e.reject_promise(pid, err);
        return Ok(Value::Obj(pid));
    }
    /// Rejet court — `NotSupportedError` pour les formes
    /// hors tranche, `InvalidAccessError` pour les
    /// combinaisons interdites spec.
    macro_rules! bail {
        ($name:expr, $msg:expr) => {{
            let err = dom_exc(e, $name, $msg);
            e.reject_promise(pid, err);
            return Ok(Value::Obj(pid));
        }};
    }
    match format_s.to_ascii_lowercase().as_str() {
        "raw" => {
            let ok = key_type == "secret"
                || (matches!(key_algo, KeyAlgo::EcdsaP256)
                    && key_bytes.len() == 65
                    && key_bytes.first() == Some(&0x04))
                || (matches!(key_algo, KeyAlgo::Ed25519) && key_bytes.len() == 32);
            if !ok {
                bail!(
                    "NotSupportedError",
                    "subtle.exportKey: raw hors tranche pour cette clé"
                );
            }
            let ab = crate::typed_array::new_array_buffer(e, key_bytes);
            e.resolve_promise(pid, ab);
        }
        "pkcs8" => {
            if key_type != "private" {
                bail!(
                    "InvalidAccessError",
                    "subtle.exportKey: pkcs8 = clé privée seule"
                );
            }
            let ab = crate::typed_array::new_array_buffer(e, key_bytes);
            e.resolve_promise(pid, ab);
        }
        "spki" => {
            if key_type != "public" {
                bail!(
                    "InvalidAccessError",
                    "subtle.exportKey: spki = clé publique seule"
                );
            }
            // Canonique publique : RSA = `RSAPublicKey` nu
            // (re-enveloppe SPKI à préfixe fixe) ; EC/Ed =
            // `spki` si importé ainsi (`0x30`), sinon
            // SEC1/raw → re-enveloppe à préfixe fixe.
            let spki = match key_algo {
                KeyAlgo::RsaPkcs1 { .. } | KeyAlgo::RsaOaep { .. } => {
                    let mut out = vec![
                        0x30, 0x82, 0x00, 0x00, 0x30, 0x0d, 0x06, 0x09, 0x2a, 0x86, 0x48, 0x86,
                        0xf7, 0x0d, 0x01, 0x01, 0x01, 0x05, 0x00, 0x03, 0x82,
                    ];
                    let Ok(bs_len) = u16::try_from(key_bytes.len() + 1) else {
                        bail!(
                            "OperationError",
                            "subtle.exportKey: matière RSA hors bornes"
                        );
                    };
                    out.extend_from_slice(&bs_len.to_be_bytes());
                    out.push(0x00);
                    out.extend_from_slice(&key_bytes);
                    let Ok(total) = u16::try_from(out.len() - 4) else {
                        bail!(
                            "OperationError",
                            "subtle.exportKey: matière RSA hors bornes"
                        );
                    };
                    out[2..4].copy_from_slice(&total.to_be_bytes());
                    out
                }
                KeyAlgo::EcdsaP256 => {
                    if key_bytes.first() == Some(&0x30) {
                        key_bytes.clone()
                    } else if key_bytes.len() == 65 && key_bytes.first() == Some(&0x04) {
                        const P: &[u8] = &[
                            0x30, 0x59, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02,
                            0x01, 0x06, 0x08, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07, 0x03,
                            0x42, 0x00,
                        ];
                        let mut out = P.to_vec();
                        out.extend_from_slice(&key_bytes);
                        out
                    } else {
                        bail!(
                            "NotSupportedError",
                            "subtle.exportKey: spki EC hors tranche"
                        );
                    }
                }
                KeyAlgo::Ed25519 => {
                    if key_bytes.first() == Some(&0x30) {
                        key_bytes.clone()
                    } else if key_bytes.len() == 32 {
                        const P: &[u8] = &[
                            0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00,
                        ];
                        let mut out = P.to_vec();
                        out.extend_from_slice(&key_bytes);
                        out
                    } else {
                        bail!(
                            "NotSupportedError",
                            "subtle.exportKey: spki Ed25519 hors tranche"
                        );
                    }
                }
                _ => bail!("NotSupportedError", "subtle.exportKey: spki hors tranche"),
            };
            let ab = crate::typed_array::new_array_buffer(e, spki);
            e.resolve_promise(pid, ab);
        }
        "jwk" => {
            let key_ops: Vec<Value> = usages.iter().map(|u| e.str_val(u.clone())).collect();
            let key_ops = e.new_array(key_ops);
            match key_algo {
                // `oct` — clé symétrique / base KDF.
                _ if key_type == "secret" => {
                    let k = e.str_val(b64url_encode(&key_bytes));
                    let j = jwk_object(e, "oct", &[("k", k), ("key_ops", key_ops)]);
                    e.resolve_promise(pid, j);
                }
                KeyAlgo::RsaPkcs1 { .. } | KeyAlgo::RsaOaep { .. } if key_type == "public" => {
                    let Some((n, x)) = rsapub_components(&key_bytes) else {
                        bail!("OperationError", "subtle.exportKey: matière RSA corrompue");
                    };
                    let nv = e.str_val(b64url_encode(&n));
                    let ev = e.str_val(b64url_encode(&x));
                    let j = jwk_object(e, "RSA", &[("n", nv), ("e", ev), ("key_ops", key_ops)]);
                    e.resolve_promise(pid, j);
                }
                _ => bail!(
                    "NotSupportedError",
                    "subtle.exportKey: jwk hors tranche pour cette clé"
                ),
            }
        }
        _ => bail!("TypeError", "subtle.exportKey: unknown format"),
    }
    Ok(Value::Obj(pid))
}

impl Engine {
    /// Injecte le fournisseur crypto de la plateforme
    /// (engine-global : partagé par tous les contextes
    /// JS-C78 — la couture est installée une fois et les
    /// sous-contextes en héritent). `crypto.subtle` existe
    /// toujours — sans transport `digest()` rejette
    /// `NotSupportedError` (dégradation honnête).
    pub fn install_crypto(&mut self, transport: Rc<dyn CryptoTransport>) {
        self.crypto_transport = Some(transport);
    }

    /// Livre le résultat d'une opération crypto : résout la
    /// `Promise` avec un `ArrayBuffer` ou rejette
    /// `OperationError`. Le handle localise le contexte
    /// propriétaire (il peut être garé — JS-C78 : switch +
    /// restore, même forme que `deliver_xhr_response`) ;
    /// handle inconnu → silencieux (opération abandonnée).
    ///
    /// # Errors
    /// `JsResult` du microtask checkpoint final.
    pub fn deliver_crypto_response(&mut self, resp: CryptoResponse) -> JsResult<()> {
        let owner = if self
            .crypto
            .pending
            .iter()
            .any(|(h, _, _)| *h == resp.handle)
        {
            Some(self.cur_ctx)
        } else {
            self.parked
                .iter()
                .find(|(_, c)| c.crypto.pending.iter().any(|(h, _, _)| *h == resp.handle))
                .map(|(k, _)| *k)
        };
        let Some(owner) = owner else {
            return Ok(());
        };
        let home = self.cur_ctx;
        self.switch_to(owner);
        let r = (|| {
            let Some(pos) = self
                .crypto
                .pending
                .iter()
                .position(|(h, _, _)| *h == resp.handle)
            else {
                return Ok(());
            };
            let (_, pid, kind) = self.crypto.pending.remove(pos);
            self.sched_tick("dlv.crypto");
            match (kind, resp.result) {
                (PendingKind::Bytes, Ok(bytes)) => {
                    let ab = crate::typed_array::new_array_buffer(self, bytes);
                    self.resolve_promise(pid, ab);
                }
                (PendingKind::Bool, Ok(bytes)) => {
                    // Octet `0`/`1` posé par la plateforme →
                    // booléen spec `verify`.
                    self.resolve_promise(pid, Value::Bool(bytes.first() == Some(&1)));
                }
                (
                    PendingKind::ImportKey {
                        key_type,
                        algo_name,
                        usages,
                        extractable,
                        algo,
                    },
                    Ok(key_bytes),
                ) => {
                    let ck = self.new_host_object(std::rc::Rc::new(CryptoKeyHost {
                        key: key_bytes,
                        key_type,
                        algo_name,
                        algo,
                        usages,
                        extractable,
                    }));
                    if let (Some(p), Value::Obj(id)) = (self.crypto.ck_proto, ck.clone()) {
                        if let Some(o) = self.heap.obj_mut(id) {
                            o.set_proto(Value::Obj(p));
                        }
                    }
                    self.resolve_promise(pid, ck);
                }
                (_, Err(msg)) => {
                    let err = dom_exc(self, "OperationError", &msg);
                    self.reject_promise(pid, err);
                }
            }
            self.run_microtasks()?;
            Ok(())
        })();
        if self.cur_ctx != home {
            self.switch_to(home);
        }
        r
    }
}

/// Params AEAD `{name,iv,additionalData?}` — `iv`/`aad`
/// copiés hors heap ; `tagLength` accepté 128 ou absent
/// (ring scelle toujours 16 octets — autre taille = rejet
/// `NotSupportedError` honnête, jamais tronqué).
fn aead_params(e: &mut Engine, algo_v: Value) -> JsResult<Option<(Vec<u8>, Vec<u8>)>> {
    let Value::Obj(id) = algo_v else {
        return Ok(None);
    };
    let iv_v = e.get_prop(id, "iv").unwrap_or(Value::Undefined);
    let Some(iv) = crate::typed_array::bytes_of_value(e, iv_v) else {
        return Ok(None);
    };
    if let Some(Value::Num(tl)) = e.get_prop(id, "tagLength") {
        if tl as u64 != 128 {
            let err = dom_exc(
                e,
                "NotSupportedError",
                "subtle: AES-GCM tagLength != 128 unsupported",
            );
            return Err(crate::heap::Thrown(err));
        }
    }
    let aad_v = e.get_prop(id, "additionalData").unwrap_or(Value::Undefined);
    let aad = crate::typed_array::bytes_of_value(e, aad_v).unwrap_or_default();
    Ok(Some((iv, aad)))
}

/// Params `{name,iv}` AES-CBC (JS-C562) — `iv` **exactement
/// 128 bits** (spec `AesCbcParams` : tout autre taille →
/// `OperationError` côté appelant) ; pas d'`additionalData`
/// ni de `tagLength` (props ignorées spec).
fn cbc_params(e: &mut Engine, algo_v: Value) -> JsResult<Option<Vec<u8>>> {
    let Value::Obj(id) = algo_v else {
        return Ok(None);
    };
    let iv_v = e.get_prop(id, "iv").unwrap_or(Value::Undefined);
    Ok(crate::typed_array::bytes_of_value(e, iv_v).filter(|iv| iv.len() == 16))
}

/// `crypto.subtle.encrypt`/`decrypt` AES-GCM (JS-C160) et
/// AES-CBC (JS-C562, aliexpress §B-117) — clé `secret` de
/// l'algo demandé portant l'usage ; sinon
/// `InvalidAccessError` async (spec), nom non supporté →
/// `NotSupportedError`. `data` → `chiffré‖tag` (GCM) ou
/// `chiffré` PKCS#7 (CBC) et inverse.
fn subtle_aead(
    e: &mut Engine,
    args: &[Value],
    usage: &str,
    op_kind: fn(KeyAlgo, Vec<u8>, Vec<u8>) -> CryptoOp,
) -> JsResult<Value> {
    let algo_v = arg(args, 0);
    let key_v = arg(args, 1);
    let Some(data) = crate::typed_array::bytes_of_value(e, arg(args, 2)) else {
        return Err(e.type_error("subtle: data must be a BufferSource"));
    };
    let pid = e.new_promise();
    // `algorithm` = `{name:"AES-GCM"}` ou `{name:"AES-CBC"}` —
    // la clé confirme `name` et porte l'usage.
    let want = match algo_name_of(e, algo_v.clone())?
        .map(|n| n.to_ascii_uppercase().replace('-', ""))
        .as_deref()
    {
        Some("AESGCM") => Some(KeyAlgo::AesGcm),
        Some("AESCBC") => Some(KeyAlgo::AesCbc),
        _ => None,
    };
    let Some((key_bytes, key_type, _an, key_algo, usages, _)) = crypto_key_of(e, key_v.clone())
    else {
        let desc = crypto_arg_desc(e, key_v);
        let err = dom_exc(
            e,
            "InvalidAccessError",
            &format!("subtle.{usage}: not a CryptoKey (« {desc} »)"),
        );
        e.reject_promise(pid, err);
        return Ok(Value::Obj(pid));
    };
    let Some(want) = want else {
        let asked = algo_asked(e, algo_v)?;
        let err = dom_exc(
            e,
            "NotSupportedError",
            &format!("subtle.{usage}: unknown algorithm (« {asked} »)"),
        );
        e.reject_promise(pid, err);
        return Ok(Value::Obj(pid));
    };
    if !(want == key_algo && key_type == "secret" && usages.iter().any(|u| u == usage)) {
        let err = dom_exc(
            e,
            "InvalidAccessError",
            "subtle: key algorithm/usages mismatch",
        );
        e.reject_promise(pid, err);
        return Ok(Value::Obj(pid));
    }
    let (iv, aad) = match want {
        KeyAlgo::AesCbc => {
            if let Some(iv) = cbc_params(e, algo_v)? {
                (iv, Vec::new())
            } else {
                let err = dom_exc(
                    e,
                    "OperationError",
                    "subtle: AES-CBC veut un iv de 128 bits",
                );
                e.reject_promise(pid, err);
                return Ok(Value::Obj(pid));
            }
        }
        _ => {
            if let Some((iv, aad)) = aead_params(e, algo_v)? {
                (iv, aad)
            } else {
                let err = dom_exc(e, "OperationError", "subtle: AES-GCM requires an iv");
                e.reject_promise(pid, err);
                return Ok(Value::Obj(pid));
            }
        }
    };
    match e.crypto_transport.clone() {
        None => {
            let err = dom_exc(
                e,
                "NotSupportedError",
                "subtle: no crypto backend installed",
            );
            e.reject_promise(pid, err);
        }
        Some(tr) => {
            let h = tr.submit(CryptoRequest {
                op: op_kind(want, iv, aad),
                data,
                key: key_bytes,
            });
            e.crypto.pending.push((h, pid, PendingKind::Bytes));
        }
    }
    Ok(Value::Obj(pid))
}

/// `subtle.encrypt`/`decrypt` RSA-OAEP (JS-C231) — clé
/// `public`/`private` `RSA-OAEP` portant l'usage demandé ;
/// `label` optionnel (`BufferSource`, vide par défaut) porté
/// dans le champ `aad` de l'op (`iv` inutilisé — borne
/// documentée de la couture).
fn subtle_rsa_oaep(
    e: &mut Engine,
    args: &[Value],
    usage: &str,
    op_kind: fn(KeyAlgo, Vec<u8>, Vec<u8>) -> CryptoOp,
) -> JsResult<Value> {
    let algo_v = arg(args, 0);
    let key_v = arg(args, 1);
    let Some(data) = crate::typed_array::bytes_of_value(e, arg(args, 2)) else {
        return Err(e.type_error("subtle: data must be a BufferSource"));
    };
    let pid = e.new_promise();
    let Some((key_bytes, key_type, _an, key_algo, usages, _)) = crypto_key_of(e, key_v.clone())
    else {
        let desc = crypto_arg_desc(e, key_v);
        let err = dom_exc(
            e,
            "InvalidAccessError",
            &format!("subtle.{usage}: not a CryptoKey (« {desc} »)"),
        );
        e.reject_promise(pid, err);
        return Ok(Value::Obj(pid));
    };
    // Spec : `encrypt` = clé publique, `decrypt` = privée.
    let want_type = if usage == "encrypt" {
        "public"
    } else {
        "private"
    };
    if !(matches!(key_algo, KeyAlgo::RsaOaep { .. })
        && key_type == want_type
        && usages.iter().any(|u| u == usage))
    {
        let err = dom_exc(
            e,
            "InvalidAccessError",
            "subtle: key algorithm/usages mismatch",
        );
        e.reject_promise(pid, err);
        return Ok(Value::Obj(pid));
    }
    let label = if let Value::Obj(id) = algo_v {
        e.get_prop(id, "label")
            .and_then(|lv| crate::typed_array::bytes_of_value(e, lv))
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    match e.crypto_transport.clone() {
        None => {
            let err = dom_exc(
                e,
                "NotSupportedError",
                "subtle: no crypto backend installed",
            );
            e.reject_promise(pid, err);
        }
        Some(tr) => {
            let h = tr.submit(CryptoRequest {
                op: op_kind(key_algo, Vec::new(), label),
                data,
                key: key_bytes,
            });
            e.crypto.pending.push((h, pid, PendingKind::Bytes));
        }
    }
    Ok(Value::Obj(pid))
}

/// `subtle.encrypt` — `CryptoOp::Encrypt`.
fn subtle_encrypt(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    if algo_name_of(e, arg(args, 0))?
        .is_some_and(|n| n.to_ascii_uppercase().replace('-', "") == "RSAOAEP")
    {
        return subtle_rsa_oaep(e, args, "encrypt", |algo, iv, aad| CryptoOp::Encrypt {
            algo,
            iv,
            aad,
        });
    }
    subtle_aead(e, args, "encrypt", |algo, iv, aad| CryptoOp::Encrypt {
        algo,
        iv,
        aad,
    })
}

/// `subtle.decrypt` — `CryptoOp::Decrypt`.
fn subtle_decrypt(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    if algo_name_of(e, arg(args, 0))?
        .is_some_and(|n| n.to_ascii_uppercase().replace('-', "") == "RSAOAEP")
    {
        return subtle_rsa_oaep(e, args, "decrypt", |algo, iv, aad| CryptoOp::Decrypt {
            algo,
            iv,
            aad,
        });
    }
    subtle_aead(e, args, "decrypt", |algo, iv, aad| CryptoOp::Decrypt {
        algo,
        iv,
        aad,
    })
}

/// Pose `crypto.subtle` (objet `SubtleCrypto`) sur l'objet
/// `crypto` — appelé par `builtins::install` à chaque realm
/// (JS-C78 : chaque contexte a son instance, l'opération
/// `subtle` op non implémentée → rejet `NotSupportedError`
/// (JS-C562) — l'API existe et répond honnêtement « non »,
/// comme un navigateur face à un algorithme non supporté.
fn subtle_unsupported(e: &mut Engine, _t: Value, _a: &[Value]) -> JsResult<Value> {
    let pid = e.new_promise();
    let err = dom_exc(e, "NotSupportedError", "subtle: operation not supported");
    e.reject_promise(pid, err);
    Ok(Value::Obj(pid))
}

/// `SubtleCrypto.supports(operation, algorithm)` — feature-
/// detect spec : `true` seulement pour les couples réellement
/// implémentés (mêmes algos que `importKey`/`sign`/`encrypt` —
/// JS-C562 pour `AES-CBC`/`PBKDF2`/`HKDF`/`generateKey`/
/// `derive*`/`exportKey`). L'ordre inversé `(algorithm,
/// operation)`, vu sur des pages réelles, est toléré.
fn subtle_supports(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    const OPS: &[&str] = &[
        "digest",
        "importKey",
        "generateKey",
        "exportKey",
        "deriveBits",
        "deriveKey",
        "sign",
        "verify",
        "encrypt",
        "decrypt",
        "wrapKey",
        "unwrapKey",
    ];
    let first = e.to_js_string(arg(args, 0))?;
    // Spec : `supports(operation, algorithm)` — mais des
    // pages passent l'ordre inverse ; on tolère les deux
    // en regardant laquelle des deux valeurs est une
    // opération connue.
    let (algo, op) = if OPS.contains(&first.as_str()) {
        (algo_name_of(e, arg(args, 1))?.unwrap_or_default(), first)
    } else {
        (
            algo_name_of(e, arg(args, 0))?.unwrap_or_default(),
            e.to_js_string(arg(args, 1))?,
        )
    };
    let a = algo.to_ascii_uppercase().replace(['-', ' '], "");
    Ok(Value::Bool(match op.as_str() {
        "digest" => matches!(a.as_str(), "SHA1" | "SHA256" | "SHA384" | "SHA512"),
        "importKey" => matches!(
            a.as_str(),
            "HMAC"
                | "ECDSA"
                | "ED25519"
                | "AESGCM"
                | "AESCBC"
                | "RSAOAEP"
                | "RSASSAPKCS1V15"
                | "PBKDF2"
                | "HKDF"
        ),
        "generateKey" => matches!(a.as_str(), "AESGCM" | "AESCBC" | "HMAC"),
        "exportKey" => matches!(
            a.as_str(),
            "AESGCM" | "AESCBC" | "HMAC" | "ECDSA" | "ED25519" | "RSAOAEP" | "RSASSAPKCS1V15"
        ),
        "deriveBits" | "deriveKey" => matches!(a.as_str(), "PBKDF2" | "HKDF"),
        "sign" | "verify" => matches!(a.as_str(), "HMAC" | "ECDSA" | "ED25519" | "RSASSAPKCS1V15"),
        "encrypt" | "decrypt" => matches!(a.as_str(), "AESGCM" | "AESCBC" | "RSAOAEP"),
        _ => false,
    }))
}

/// `digest` est partagée en native).
pub(crate) fn install_subtle(e: &mut Engine, crypto: ObjId) {
    // `SubtleCrypto` global (JS-C562) — ctor « Illegal » +
    // proto partageant les natives de l'instance (les ops
    // lisent `args`/la `CryptoKey`, jamais `this`) ; les membres
    // non implémentés répondent par rejet `NotSupportedError`
    // et `supports()` renseigne la matrice réelle.
    let sp = e.new_object();
    for (m, f) in [
        ("digest", subtle_digest as crate::heap::NativeFn),
        ("importKey", subtle_import_key),
        ("sign", subtle_sign),
        ("verify", subtle_verify),
        ("encrypt", subtle_encrypt),
        ("decrypt", subtle_decrypt),
        ("generateKey", subtle_generate_key),
        ("deriveBits", subtle_derive_bits),
        ("deriveKey", subtle_derive_key),
        ("exportKey", subtle_export_key),
        ("supports", subtle_supports),
        ("wrapKey", subtle_unsupported),
        ("unwrapKey", subtle_unsupported),
        ("getPublicKey", subtle_unsupported),
        ("encapsulateBits", subtle_unsupported),
        ("encapsulateKey", subtle_unsupported),
        ("decapsulateBits", subtle_unsupported),
        ("decapsulateKey", subtle_unsupported),
    ] {
        e.add_method(sp, m, f);
    }
    let tag = e.str_val("SubtleCrypto");
    e.set_own_attrs(
        sp,
        crate::interp::SYM_TO_STRING_TAG,
        tag,
        crate::heap::PropAttrs {
            writable: false,
            enumerable: false,
            configurable: true,
        },
    );
    let sc = e.illegal_ctor_bound("SubtleCrypto");
    if let Value::Obj(cid) = sc {
        e.set_own_attrs(
            cid,
            "prototype",
            Value::Obj(sp),
            crate::heap::PropAttrs::FIXED,
        );
        let sup = e.native_fn("supports", subtle_supports);
        e.set_own(cid, "supports", sup, false);
    }
    let g = e.global;
    e.set_own(g, "SubtleCrypto", sc, false);

    let subtle = e.new_object();
    // `subtle instanceof SubtleCrypto` + héritage des membres
    // non implémentés (rejet honnête) via le proto partagé.
    if let Some(o) = e.heap.obj_mut(subtle) {
        o.set_proto(Value::Obj(sp));
    }
    e.add_method(subtle, "digest", subtle_digest);
    // JS-C93 — ops à clé (HMAC/ECDSA-P256/Ed25519) via la
    // même couture ; `importKey` rend un `CryptoKey`.
    e.add_method(subtle, "importKey", subtle_import_key);
    e.add_method(subtle, "sign", subtle_sign);
    e.add_method(subtle, "verify", subtle_verify);
    // JS-C160 — AES-GCM `encrypt`/`decrypt` (marinetraffic :
    // `importKey unknown algorithm` + `Failed to decrypt`) ;
    // JS-C562 étend à `AES-CBC` (aliexpress §B-117).
    e.add_method(subtle, "encrypt", subtle_encrypt);
    e.add_method(subtle, "decrypt", subtle_decrypt);
    // JS-C562 — §B-117 : `generateKey` symétrique (deliveroo),
    // `deriveBits`/`deriveKey` PBKDF2/HKDF, `exportKey`
    // raw/jwk/pkcs8/spki (repli `SubtleCrypto` proto pour
    // `wrapKey`/`unwrapKey`/`getPublicKey`/`encapsulate*`).
    e.add_method(subtle, "generateKey", subtle_generate_key);
    e.add_method(subtle, "deriveBits", subtle_derive_bits);
    e.add_method(subtle, "deriveKey", subtle_derive_key);
    e.add_method(subtle, "exportKey", subtle_export_key);
    e.set_own(crypto, "subtle", Value::Obj(subtle), false);
}
