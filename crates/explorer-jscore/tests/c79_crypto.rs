// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! `crypto.subtle` par couture embedder (JS-C79) — le moteur
//! émet `CryptoRequest` vers un `CryptoTransport`, la
//! plateforme calcule, `deliver_crypto_response` résout la
//! promesse. Le harnais joue « la plateforme » avec un digest
//! jouet : la **plomberie** est testée ici, les vrais
//! condensats `ring` dans `explorer-net`.

use explorer_jscore::{
    CryptoOp, CryptoRequest, CryptoResponse, CryptoTransport, DigestAlgo, Engine,
};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// Transport jouet — digest « honnête mais faux » : les
/// premiers octets du message complétés de `0xAA` jusqu'à la
/// longueur attendue par l'algo. Suffit pour prouver le flux
/// handle → promesse → `ArrayBuffer`.
struct StubCrypto {
    requests: RefCell<Vec<(u64, CryptoRequest)>>,
    next: Cell<u64>,
    /// Taille de condensat par [`DigestAlgo`].
    sizes: (usize, usize, usize),
}

impl StubCrypto {
    fn new() -> Rc<Self> {
        Rc::new(StubCrypto {
            requests: RefCell::new(Vec::new()),
            next: Cell::new(0),
            sizes: (32, 48, 64),
        })
    }

    /// Réponse jouet pour `handle` — replie `requests` en
    /// `CryptoResponse::result`.
    fn response(&self, handle: u64) -> CryptoResponse {
        let (h, req) = self
            .requests
            .borrow()
            .iter()
            .find(|(h, _)| *h == handle)
            .cloned()
            .expect("handle soumis");
        let size = match req.op {
            CryptoOp::Digest(DigestAlgo::Sha256) => self.sizes.0,
            CryptoOp::Digest(DigestAlgo::Sha384) => self.sizes.1,
            CryptoOp::Digest(DigestAlgo::Sha512) => self.sizes.2,
        };
        let mut out = req.data;
        out.resize(size, 0xAA);
        CryptoResponse {
            handle: h,
            result: Ok(out),
        }
    }
}

impl CryptoTransport for StubCrypto {
    fn submit(&self, req: CryptoRequest) -> u64 {
        self.next.set(self.next.get() + 1);
        let id = self.next.get();
        self.requests.borrow_mut().push((id, req));
        id
    }
}

fn engine() -> (Engine, Rc<StubCrypto>) {
    let mut e = Engine::new();
    let t = StubCrypto::new();
    e.install_crypto(t.clone());
    (e, t)
}

fn eval(e: &mut Engine, src: &str) -> explorer_jscore::Value {
    match e.eval(src) {
        Ok(v) => v,
        Err(err) => panic!("{src} -> {err}"),
    }
}

fn describe(e: &mut Engine, src: &str) -> String {
    let v = eval(e, src);
    e.describe(v)
}

/// `crypto.subtle` est présent et `digest` appelable — la
/// détection de feature des pages réelles.
#[test]
fn subtle_surface_present() {
    let (mut e, _t) = engine();
    assert_eq!(describe(&mut e, "typeof crypto.subtle"), "object");
    assert_eq!(describe(&mut e, "typeof crypto.subtle.digest"), "function");
    assert_eq!(
        describe(&mut e, "Object.prototype.toString.call(crypto.subtle)"),
        "[object SubtleCrypto]"
    );
}

/// `digest('SHA-256', ta)` : la promesse résout un
/// `ArrayBuffer` des octets livrés par la plateforme —
/// l'appel ne fait rien (asynchrone spec).
#[test]
fn digest_sha256_resolves_arraybuffer() {
    let (mut e, t) = engine();
    eval(
        &mut e,
        "var got=null; crypto.subtle.digest('SHA-256', new Uint8Array([1,2,3]))\
         .then(v=>{got=v});",
    );
    assert_eq!(describe(&mut e, "got"), "null", "pas de livraison sync");
    let h = t.requests.borrow()[0].0;
    e.deliver_crypto_response(t.response(h)).expect("deliver");
    assert_eq!(describe(&mut e, "got instanceof ArrayBuffer"), "true");
    assert_eq!(describe(&mut e, "got.byteLength"), "32");
    // Les octets source précèdent le padding jouet.
    assert_eq!(
        describe(
            &mut e,
            "Array.from(new Uint8Array(got).slice(0,4)).join(',')"
        ),
        "1,2,3,170"
    );
}

/// `AlgorithmIdentifier` — objet `{name}` et casse
/// insensible sont normalisés par le moteur avant émission.
#[test]
fn digest_algorithm_identifier_forms() {
    let (mut e, t) = engine();
    eval(
        &mut e,
        "crypto.subtle.digest({name:'sha-384'}, new Uint8Array(1));\
         crypto.subtle.digest('SHA512', new Uint8Array(1));",
    );
    let ops: Vec<DigestAlgo> = t
        .requests
        .borrow()
        .iter()
        .map(|(_, r)| match r.op {
            CryptoOp::Digest(a) => a,
        })
        .collect();
    assert_eq!(ops, vec![DigestAlgo::Sha384, DigestAlgo::Sha512]);
}

/// Algorithme inconnu → `Promise` rejetée `NotSupportedError`
/// (normalize spec — la plateforme n'est jamais appelée).
#[test]
fn digest_unknown_algo_rejects_notsupported() {
    let (mut e, t) = engine();
    eval(
        &mut e,
        "var err=null; crypto.subtle.digest('SHA-1', new Uint8Array(1))\
         .catch(ex=>{err=ex.name});",
    );
    // Rejet à l'appel → la réaction `.catch` a tourné au
    // checkpoint microtask de fin d'`eval`.
    assert_eq!(describe(&mut e, "err"), "NotSupportedError");
    assert!(t.requests.borrow().is_empty(), "aucune émission");
}

/// `data` non-`BufferSource` → `TypeError` **synchrone**
/// (échec de conversion `WebIDL` — pas de promesse retournée).
#[test]
fn digest_bad_data_throws_sync() {
    let (mut e, _t) = engine();
    assert_eq!(
        describe(
            &mut e,
            "try{crypto.subtle.digest('SHA-256', 42);'no'}catch(x){x.name}"
        ),
        "TypeError"
    );
    assert_eq!(
        describe(
            &mut e,
            "try{crypto.subtle.digest('SHA-256');'no'}catch(x){x.name}"
        ),
        "TypeError"
    );
}

/// Sans transport installé, `subtle` existe mais `digest`
/// rejette `NotSupportedError` — dégradation honnête, jamais
/// de faux digest.
#[test]
fn digest_without_transport_rejects() {
    let mut e = Engine::new();
    eval(
        &mut e,
        "var err=null; crypto.subtle.digest('SHA-256', new Uint8Array(1))\
         .catch(ex=>{err=ex.name});",
    );
    assert_eq!(describe(&mut e, "err"), "NotSupportedError");
}

/// Échec plateforme (`Err` en réponse) → `OperationError`.
#[test]
fn digest_platform_error_is_operationerror() {
    let (mut e, t) = engine();
    eval(
        &mut e,
        "var err=null; crypto.subtle.digest('SHA-256', new Uint8Array(1))\
         .catch(ex=>{err=ex.name});",
    );
    e.deliver_crypto_response(CryptoResponse {
        handle: t.requests.borrow()[0].0,
        result: Err("backend exploded".into()),
    })
    .expect("deliver");
    assert_eq!(describe(&mut e, "err"), "OperationError");
}

/// `data` est copiée hors du heap à l'appel — muter le buffer
/// après `digest()` ne change pas la requête émise (spec
/// `CopyBytes`).
#[test]
fn digest_copies_data_at_call_time() {
    let (mut e, t) = engine();
    eval(
        &mut e,
        "var b=new Uint8Array([7,7,7]);\
         crypto.subtle.digest('SHA-256', b); b[0]=99;",
    );
    assert_eq!(t.requests.borrow()[0].1.data, vec![7, 7, 7]);
}

/// Une opération soumise dans un contexte garé est livrée à
/// **son** realm (JS-C78 — la promesse appartient au contexte
/// émetteur, pas au courant).
#[test]
fn digest_delivers_to_owning_context() {
    let (mut e, t) = engine();
    let child = e.context_new("https://child.example/f", None);
    eval(
        &mut e,
        "var got=null; crypto.subtle.digest('SHA-256', new Uint8Array(1))\
         .then(v=>{got=v.byteLength});",
    );
    e.context_switch(0);
    assert_eq!(
        describe(&mut e, "typeof got"),
        "undefined",
        "le parent n'a pas de `got` — realm isolé"
    );
    e.deliver_crypto_response(t.response(t.requests.borrow()[0].0))
        .expect("deliver");
    e.context_switch(child);
    assert_eq!(describe(&mut e, "got"), "32", "livré au realm enfant");
}

/// La promesse en vol est racine GC — un `collect` ne la fait
/// pas disparaître avant la livraison.
#[test]
fn pending_digest_is_gc_rooted() {
    let (mut e, t) = engine();
    eval(
        &mut e,
        "crypto.subtle.digest('SHA-256', new Uint8Array(1));",
    );
    e.collect();
    e.deliver_crypto_response(t.response(t.requests.borrow()[0].0))
        .expect("deliver après GC");
}

/// `getRandomValues` — quota spec : plus de 65 536 octets →
/// `QuotaExceededError` (borne vérifiée JS-C79).
#[test]
fn get_random_values_quota() {
    let mut e = Engine::new();
    assert_eq!(
        describe(
            &mut e,
            "try{crypto.getRandomValues(new Uint8Array(65537));'no'}\
             catch(x){x.name}"
        ),
        "QuotaExceededError"
    );
    // Exactement 64 Kio : accepté.
    assert_eq!(
        describe(
            &mut e,
            "crypto.getRandomValues(new Uint8Array(65536)).byteLength"
        ),
        "65536"
    );
}
