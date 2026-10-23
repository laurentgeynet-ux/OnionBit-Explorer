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
    /// `(handle, promise ObjId)` en vol dans ce contexte.
    pub pending: Vec<(u64, ObjId)>,
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
                    });
                    e.crypto.pending.push((h, pid));
                }
            }
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
        let owner = if self.crypto.pending.iter().any(|(h, _)| *h == resp.handle) {
            Some(self.cur_ctx)
        } else {
            self.parked
                .iter()
                .find(|(_, c)| c.crypto.pending.iter().any(|(h, _)| *h == resp.handle))
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
                .position(|(h, _)| *h == resp.handle)
            else {
                return Ok(());
            };
            let (_, pid) = self.crypto.pending.remove(pos);
            match resp.result {
                Ok(bytes) => {
                    let ab = crate::typed_array::new_array_buffer(self, bytes);
                    self.resolve_promise(pid, ab);
                }
                Err(msg) => {
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
    let tag = e.str_val("SubtleCrypto");
    e.set_own(subtle, crate::interp::SYM_TO_STRING_TAG, tag, false);
    e.set_own(crypto, "subtle", Value::Obj(subtle), false);
}
