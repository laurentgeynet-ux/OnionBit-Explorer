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
//! logins `OAuth2`) :
//! - `crypto.subtle.digest(algorithm, data)` → `Promise`
//!   résolvant un `ArrayBuffer` — `SHA-256`/`SHA-384`/`SHA-512`
//!   (noms normalisés casse-insensible, forme `'SHA-256'` ou
//!   `{name}` — `AlgorithmIdentifier` spec).
//! - Algorithme inconnu → `Promise` rejetée `NotSupportedError`
//!   (spec `NormalizeAlgorithm` ; shape `DOMException`
//!   `{name,message}` — approximation documentée, idem IDB).
//! - `data` non-`BufferSource` / `algorithm` non convertible →
//!   `TypeError` **synchrone** (échec de conversion `WebIDL`,
//!   avant la promesse).
//! - Sans transport installé → `Promise` rejetée
//!   `NotSupportedError` (dégradation honnête : la page voit un
//!   backend absent, jamais un faux digest).
//! - Échec plateforme (`Err` côté transport) → `OperationError`.
//!
//! `importKey`/`sign`/`verify`/`encrypt`/`decrypt`/`deriveKey`/
//! `wrapKey`/`generateKey`/`exportKey`/`CryptoKey` : absents en
//! v1 — la détection de feature voit des méthodes manquantes,
//! pas des stubs (HMAC/AES-GCM/RSA-PSS au besoin site).

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
}

/// Format d'`importKey` (JS-C93) — `jwk` n'entre pas dans
/// cette tranche (rejet `NotSupportedError` côté moteur).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportFormat {
    /// `raw` — octets bruts.
    Raw,
    /// `pkcs8` — clé privée PKCS#8.
    Pkcs8,
    /// `spki` — clé publique `SubjectPublicKeyInfo`.
    Spki,
}

/// Algorithme `digest` normalisé (`WebCrypto` — les noms
/// `'SHA-*'` sont validés par le moteur, la plateforme ne
/// re-parse jamais de chaîne).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DigestAlgo {
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
                None => {
                    let err = dom_exc(
                        e,
                        "NotSupportedError",
                        "subtle.digest: no crypto backend installed",
                    );
                    e.reject_promise(pid, err);
                }
                Some(tr) => {
                    let h = tr.submit(CryptoRequest {
                        op: CryptoOp::Digest(algo),
                        data,
                        key: Vec::new(),
                    });
                    e.crypto.pending.push((h, pid, PendingKind::Bytes));
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
            None => e.to_js_string(v)?,
        },
        _ => e.to_js_string(v)?,
    };
    Ok(match name.to_ascii_uppercase().replace('-', "").as_str() {
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
                    .and_then(|c| e.heap_string_of(c))
                    .is_some_and(|c| c.eq_ignore_ascii_case("P-256"))
            } else {
                false
            };
            curve_ok.then_some((KeyAlgo::EcdsaP256, "ECDSA"))
        }
        "ED25519" | "NODEED25519" => Some((KeyAlgo::Ed25519, "Ed25519")),
        _ => None,
    })
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
    let Some(data) = crate::typed_array::bytes_of_value(e, arg(args, 1)) else {
        return Err(e.type_error("subtle.importKey: keyData must be a BufferSource"));
    };
    let algo_v = arg(args, 2);
    let extractable = e.to_boolean(arg(args, 3));
    let usages = key_usages(e, arg(args, 4))?;
    let pid = e.new_promise();
    let format = match format_s.to_ascii_lowercase().as_str() {
        "raw" => Some(ImportFormat::Raw),
        "pkcs8" => Some(ImportFormat::Pkcs8),
        "spki" => Some(ImportFormat::Spki),
        "jwk" => {
            let err = dom_exc(
                e,
                "NotSupportedError",
                "subtle.importKey: jwk not supported",
            );
            e.reject_promise(pid, err);
            return Ok(Value::Obj(pid));
        }
        _ => {
            let err = dom_exc(e, "TypeError", "subtle.importKey: unknown format");
            e.reject_promise(pid, err);
            return Ok(Value::Obj(pid));
        }
    };
    let Some(format) = format else {
        return Ok(Value::Obj(pid));
    };
    match key_algo(e, algo_v)? {
        None => {
            let err = dom_exc(
                e,
                "NotSupportedError",
                "subtle.importKey: unknown algorithm",
            );
            e.reject_promise(pid, err);
        }
        Some((algo, algo_name)) => {
            let key_type: &'static str = match format {
                ImportFormat::Raw => {
                    if matches!(algo, KeyAlgo::Hmac { .. }) {
                        "secret"
                    } else {
                        "public"
                    }
                }
                ImportFormat::Pkcs8 => "private",
                ImportFormat::Spki => "public",
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
    let Some((key_bytes, key_type, algo_name, key_algo, usages, _)) = crypto_key_of(e, key_v)
    else {
        let err = dom_exc(e, "InvalidAccessError", "subtle.sign: not a CryptoKey");
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
    let Some((key_bytes, key_type, algo_name, key_algo, usages, _)) = crypto_key_of(e, key_v)
    else {
        let err = dom_exc(e, "InvalidAccessError", "subtle.verify: not a CryptoKey");
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
        }
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
                    if let (Some(p), Value::Obj(id)) = (self.crypto.ck_proto, ck) {
                        if let Some(o) = self.heap.obj_mut(id) {
                            o.proto = Value::Obj(p);
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

/// Pose `crypto.subtle` (objet `SubtleCrypto`) sur l'objet
/// `crypto` — appelé par `builtins::install` à chaque realm
/// (JS-C78 : chaque contexte a son instance, l'opération
/// `digest` est partagée en native).
pub(crate) fn install_subtle(e: &mut Engine, crypto: ObjId) {
    let subtle = e.new_object();
    e.add_method(subtle, "digest", subtle_digest);
    // JS-C93 — ops à clé (HMAC/ECDSA-P256/Ed25519) via la
    // même couture ; `importKey` rend un `CryptoKey`.
    e.add_method(subtle, "importKey", subtle_import_key);
    e.add_method(subtle, "sign", subtle_sign);
    e.add_method(subtle, "verify", subtle_verify);
    let tag = e.str_val("SubtleCrypto");
    e.set_own(subtle, crate::interp::SYM_TO_STRING_TAG, tag, false);
    e.set_own(crypto, "subtle", Value::Obj(subtle), false);
}
