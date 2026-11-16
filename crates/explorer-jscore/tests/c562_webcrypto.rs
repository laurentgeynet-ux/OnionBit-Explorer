// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! `WebCrypto` §B-117 (JS-C562) — `SHA-1`, `AES-CBC`,
//! `generateKey`, `deriveBits`/`deriveKey` (PBKDF2/HKDF) et
//! `exportKey`. La plomberie est prouvée par un transport
//! jouet qui enregistre les `CryptoOp` ; les vrais vecteurs
//! cryptographiques vivent dans `explorer-net` (NIST CBC,
//! RFC 6070, RFC 5869) et dans le repli natif SHA-1.

use explorer_jscore::{
    CryptoOp, CryptoRequest, CryptoResponse, CryptoTransport, DeriveAlgo, DeriveParams, DigestAlgo,
    Engine, ImportFormat, KeyAlgo,
};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// Transport jouet — enregistre chaque requête ; le test la
/// complète à la main (`CryptoResponse`). Suffit pour
/// vérifier le **dispatch** : algo normalisé, paramètres
/// copiés hors heap, `CryptoKey` remballé à la livraison.
struct StubCrypto {
    requests: RefCell<Vec<(u64, CryptoRequest)>>,
    done: RefCell<std::collections::HashSet<u64>>,
    next: Cell<u64>,
}

impl StubCrypto {
    fn new() -> Rc<Self> {
        Rc::new(StubCrypto {
            requests: RefCell::new(Vec::new()),
            done: RefCell::new(std::collections::HashSet::new()),
            next: Cell::new(0),
        })
    }

    /// Complète la première requête **non livrée** avec
    /// `bytes` (FIFO — plusieurs ops peuvent être en vol).
    fn deliver_last(&self, e: &mut Engine, bytes: Vec<u8>) {
        let (h, _) = self
            .requests
            .borrow()
            .iter()
            .find(|(h, _)| !self.done.borrow().contains(h))
            .cloned()
            .expect("op soumise");
        self.done.borrow_mut().insert(h);
        e.deliver_crypto_response(CryptoResponse {
            handle: h,
            result: Ok(bytes),
        })
        .expect("deliver");
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

/// `digest('SHA-1')` sans transport → SHA-1 natif en repli
/// (même rail que le repli SHA-2 JS-C453 — odysee §B-117).
#[test]
fn sha1_digest_native_fallback() {
    let mut e = Engine::new();
    eval(
        &mut e,
        "var h=null; crypto.subtle.digest('SHA-1',\
             new Uint8Array([97,98,99]))\
         .then(b=>{h=[...new Uint8Array(b)]\
         .map(x=>x.toString(16).padStart(2,'0')).join('')});",
    );
    e.run_microtasks().unwrap();
    // KAT FIPS 180-4 : SHA-1("abc").
    assert_eq!(
        describe(&mut e, "h"),
        "a9993e364706816aba3e25717850c26c9cd0d89d"
    );
}

/// Avec transport, `SHA-1` est routé en `Digest(Sha1)` —
/// la normalisation `'sha-1'`/`{name}` est insensible à la
/// casse et aux tirets.
#[test]
fn sha1_digest_dispatch() {
    let (mut e, t) = engine();
    eval(
        &mut e,
        "crypto.subtle.digest({name:'sha-1'}, new Uint8Array(1));",
    );
    assert!(matches!(
        t.requests.borrow()[0].1.op,
        CryptoOp::Digest(DigestAlgo::Sha1)
    ));
}

/// `SubtleCrypto.supports` — matrice JS-C562 : les couples
/// réellement couverts répondent `true`, l'ordre spec
/// `(operation, algorithm)` et l'ordre inversé sont admis.
#[test]
fn supports_matrix() {
    let (mut e, _t) = engine();
    for (op, algo) in [
        ("digest", "SHA-1"),
        ("encrypt", "AES-CBC"),
        ("decrypt", "AES-CBC"),
        ("importKey", "AES-CBC"),
        ("importKey", "PBKDF2"),
        ("importKey", "HKDF"),
        ("generateKey", "AES-CBC"),
        ("generateKey", "HMAC"),
        ("deriveBits", "PBKDF2"),
        ("deriveKey", "HKDF"),
        ("exportKey", "AES-CBC"),
    ] {
        assert_eq!(
            describe(&mut e, &format!("crypto.subtle.supports('{op}','{algo}')")),
            "true",
            "{op}/{algo}"
        );
        // Ordre inversé toléré.
        assert_eq!(
            describe(&mut e, &format!("crypto.subtle.supports('{algo}','{op}')")),
            "true",
            "inversé {algo}/{op}"
        );
    }
    // Combinaisons hors tranche → `false` honnête.
    for (op, algo) in [
        ("digest", "AES-CBC"),
        ("sign", "AES-CBC"),
        ("generateKey", "PBKDF2"),
        ("deriveBits", "AES-GCM"),
        ("wrapKey", "AES-KW"),
        ("digest", "MD5"),
    ] {
        assert_eq!(
            describe(&mut e, &format!("crypto.subtle.supports('{op}','{algo}')")),
            "false",
            "{op}/{algo}"
        );
    }
}

/// `importKey`/`encrypt`/`decrypt` `AES-CBC` — le `CryptoOp`
/// porte `AesCbc` et un `iv` de 16 octets ; `iv` d'une
/// autre taille → `OperationError` avant émission.
#[test]
fn aes_cbc_dispatch() {
    let (mut e, t) = engine();
    eval(
        &mut e,
        "crypto.subtle.importKey('raw', new Uint8Array(16),\
             {name:'AES-CBC',length:128}, true, ['encrypt','decrypt'])\
         .then(k=>{globalThis.__k=k});",
    );
    assert!(matches!(
        t.requests.borrow()[0].1.op,
        CryptoOp::ImportKey {
            format: ImportFormat::Raw,
            algo: KeyAlgo::AesCbc,
        }
    ));
    t.deliver_last(&mut e, vec![0x42; 16]);
    assert_eq!(describe(&mut e, "__k.algorithm.name"), "AES-CBC");
    assert_eq!(describe(&mut e, "__k.type"), "secret");
    // `encrypt`/`decrypt` émettent `AesCbc` + `iv` copié.
    eval(
        &mut e,
        "var iv=new Uint8Array([1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16]);\
         crypto.subtle.encrypt({name:'AES-CBC',iv},__k,new Uint8Array([9]));\
         crypto.subtle.decrypt({name:'AES-CBC',iv},__k,new Uint8Array(32));",
    );
    {
        let reqs = t.requests.borrow();
        for r in &reqs[1..=2] {
            let (CryptoOp::Encrypt { algo, iv, .. } | CryptoOp::Decrypt { algo, iv, .. }) = &r.1.op
            else {
                panic!("encrypt/decrypt attendu");
            };
            assert!(matches!(algo, KeyAlgo::AesCbc));
            assert_eq!(iv.len(), 16);
        }
        let CryptoOp::Encrypt { iv, .. } = &reqs[1].1.op else {
            panic!("encrypt op");
        };
        assert_eq!(iv.as_slice(), &(1u8..=16).collect::<Vec<_>>());
    }
    // `iv` de 8 octets → `OperationError` asynchrone, aucune
    // émission (spec `AesCbcParams` : 128 bits exacts).
    eval(
        &mut e,
        "globalThis.__e='';\
         crypto.subtle.encrypt({name:'AES-CBC',iv:new Uint8Array(8)},__k,new Uint8Array(1))\
             .catch(x=>{__e=x.name});",
    );
    e.run_microtasks().expect("pump");
    assert_eq!(describe(&mut e, "__e"), "OperationError");
    assert_eq!(t.requests.borrow().len(), 3, "aucune émission");
}

/// `generateKey({name:'AES-CBC',length:256}, true,
/// ['encrypt'])` → `CryptoOp::GenerateKey{AesCbc,256}` ; la
/// livraison remballe un `CryptoKey secret` extractable
/// avec les usages demandés.
#[test]
fn generate_key_dispatch() {
    let (mut e, t) = engine();
    eval(
        &mut e,
        "crypto.subtle.generateKey({name:'AES-CBC',length:256},true,\
             ['encrypt','decrypt']).then(k=>{globalThis.__k=k});",
    );
    assert!(matches!(
        t.requests.borrow()[0].1.op,
        CryptoOp::GenerateKey {
            algo: KeyAlgo::AesCbc,
            bits: 256
        }
    ));
    t.deliver_last(&mut e, vec![7; 32]);
    assert_eq!(describe(&mut e, "__k.type"), "secret");
    assert_eq!(describe(&mut e, "__k.algorithm.name"), "AES-CBC");
    assert_eq!(describe(&mut e, "__k.extractable"), "true");
    assert_eq!(describe(&mut e, "__k.usages.join(',')"), "encrypt,decrypt");
    // `length` invalide → `OperationError` sans émission.
    eval(
        &mut e,
        "globalThis.__e='';\
         crypto.subtle.generateKey({name:'AES-CBC',length:100},true,['encrypt'])\
             .catch(x=>{__e=x.name});",
    );
    e.run_microtasks().expect("pump");
    assert_eq!(describe(&mut e, "__e"), "OperationError");
    // HMAC : `length` absente → `bits`=0 (défaut plateforme).
    eval(
        &mut e,
        "crypto.subtle.generateKey({name:'HMAC',hash:'SHA-256'},true,['sign']);",
    );
    assert!(matches!(
        t.requests.borrow().last().expect("hmac gen").1.op,
        CryptoOp::GenerateKey {
            algo: KeyAlgo::Hmac {
                hash: DigestAlgo::Sha256
            },
            bits: 0
        }
    ));
    // Paires/KDF hors tranche → `NotSupportedError`.
    eval(
        &mut e,
        "globalThis.__n='';globalThis.__m='';\
         crypto.subtle.generateKey({name:'PBKDF2'},true,['deriveBits'])\
             .catch(x=>{__n=x.name});\
         crypto.subtle.generateKey({name:'AES-KW',length:128},true,['wrapKey'])\
             .catch(x=>{__m=x.message});",
    );
    e.run_microtasks().expect("pump");
    assert_eq!(describe(&mut e, "__n"), "NotSupportedError");
    assert!(describe(&mut e, "__m").contains("« AES-KW »"));
}

/// `deriveBits({name:'PBKDF2',salt,iterations,hash}, base,
/// 128)` → `DeriveBits` avec params copiés ; livraison →
/// `ArrayBuffer` de 16 octets.
#[test]
fn derive_bits_pbkdf2() {
    let (mut e, t) = engine();
    // Clé de base `raw` PBKDF2.
    eval(
        &mut e,
        "crypto.subtle.importKey('raw', new Uint8Array([1,2,3,4]), 'PBKDF2',\
             false, ['deriveBits','deriveKey']).then(k=>{globalThis.__base=k});",
    );
    assert!(matches!(
        t.requests.borrow()[0].1.op,
        CryptoOp::ImportKey {
            format: ImportFormat::Raw,
            algo: KeyAlgo::Pbkdf2,
        }
    ));
    t.deliver_last(&mut e, vec![1, 2, 3, 4]);
    eval(
        &mut e,
        "var got=null;\
         crypto.subtle.deriveBits({name:'PBKDF2',\
             salt:new Uint8Array([9,9]), iterations:1000, hash:'SHA-256'},\
             __base, 128).then(b=>{got=b.byteLength});",
    );
    {
        let reqs = t.requests.borrow();
        let CryptoOp::DeriveBits { params, bits } = &reqs[1].1.op else {
            panic!("DeriveBits attendu");
        };
        assert_eq!(*bits, 128);
        assert_eq!(
            *params,
            DeriveParams {
                algo: DeriveAlgo::Pbkdf2 {
                    hash: DigestAlgo::Sha256
                },
                salt: vec![9, 9],
                info: Vec::new(),
                iterations: 1000,
            }
        );
        // La matière de la clé de base voyage dans `key`.
        assert_eq!(reqs[1].1.key, vec![1, 2, 3, 4]);
    }
    t.deliver_last(&mut e, vec![0; 16]);
    assert_eq!(describe(&mut e, "got"), "16");
    // Paramètres hostiles → rejets avant émission :
    // `salt` absent → TypeError **sync**, `iterations` 0
    // → TypeError sync, KDF inconnue → NotSupportedError.
    assert_eq!(
        describe(
            &mut e,
            "try{crypto.subtle.deriveBits({name:'PBKDF2',iterations:1,hash:'SHA-256'},__base,128);'no'}catch(x){x.name}"
        ),
        "TypeError"
    );
    assert_eq!(
        describe(
            &mut e,
            "try{crypto.subtle.deriveBits({name:'PBKDF2',salt:new Uint8Array(1),iterations:0,hash:'SHA-256'},__base,128);'no'}catch(x){x.name}"
        ),
        "TypeError"
    );
    eval(
        &mut e,
        "globalThis.__n='';\
         crypto.subtle.deriveBits({name:'ECDH'},__base,128).catch(x=>{__n=x.name});",
    );
    e.run_microtasks().expect("pump");
    assert_eq!(describe(&mut e, "__n"), "NotSupportedError");
    // Usage manquant → InvalidAccessError.
    eval(
        &mut e,
        "crypto.subtle.importKey('raw', new Uint8Array(4), 'PBKDF2',\
             false, []).then(k=>{globalThis.__b2=k});",
    );
    t.deliver_last(&mut e, vec![1, 2, 3, 4]);
    eval(
        &mut e,
        "globalThis.__u='';\
         crypto.subtle.deriveBits({name:'PBKDF2',salt:new Uint8Array(1),iterations:1,hash:'SHA-256'},__b2,128)\
             .catch(x=>{__u=x.name});",
    );
    e.run_microtasks().expect("pump");
    assert_eq!(describe(&mut e, "__u"), "InvalidAccessError");
}

/// `deriveKey(HKDF…, base, {name:'AES-CBC',length:128})` →
/// `DeriveKey{params, target:AesCbc, bits:128}` ; livraison
/// → `CryptoKey secret` de l'algo cible.
#[test]
fn derive_key_hkdf_to_aes_cbc() {
    let (mut e, t) = engine();
    eval(
        &mut e,
        "crypto.subtle.importKey('raw', new Uint8Array(22), 'HKDF',\
             false, ['deriveKey']).then(k=>{globalThis.__ikm=k});",
    );
    t.deliver_last(&mut e, vec![0x0b; 22]);
    eval(
        &mut e,
        "crypto.subtle.deriveKey({name:'HKDF',salt:new Uint8Array(13),\
             info:new Uint8Array([1,2]),hash:'SHA-256'}, __ikm,\
             {name:'AES-CBC',length:128}, true, ['encrypt'])\
             .then(k=>{globalThis.__k=k});",
    );
    {
        let reqs = t.requests.borrow();
        let CryptoOp::DeriveKey {
            params,
            target,
            bits,
        } = &reqs[1].1.op
        else {
            panic!("DeriveKey attendu");
        };
        assert_eq!(*bits, 128);
        assert!(matches!(target, KeyAlgo::AesCbc));
        assert!(matches!(
            params.algo,
            DeriveAlgo::Hkdf {
                hash: DigestAlgo::Sha256
            }
        ));
        assert_eq!(params.salt, vec![0u8; 13]);
        assert_eq!(params.info, vec![1, 2]);
    }
    t.deliver_last(&mut e, vec![5; 16]);
    assert_eq!(describe(&mut e, "__k.algorithm.name"), "AES-CBC");
    assert_eq!(describe(&mut e, "__k.type"), "secret");
    // Cible non symétrique → NotSupportedError async.
    eval(
        &mut e,
        "globalThis.__n='';\
         crypto.subtle.deriveKey({name:'HKDF',salt:new Uint8Array(1),\
             info:new Uint8Array(0),hash:'SHA-256'}, __ikm,\
             {name:'ECDSA'}, false, ['sign']).catch(x=>{__n=x.name});",
    );
    e.run_microtasks().expect("pump");
    assert_eq!(describe(&mut e, "__n"), "NotSupportedError");
}

/// `exportKey('raw', secret)` — résout la matière brute,
/// moteur-seul (aucune émission) ; non-extractable →
/// `InvalidAccessError` ; format hors tranche →
/// `NotSupportedError`.
#[test]
fn export_key_raw_roundtrip() {
    let (mut e, t) = engine();
    eval(
        &mut e,
        "crypto.subtle.importKey('raw', new Uint8Array([3,1,4,1,5,9,2,6]),\
             'AES-CBC', true, ['encrypt'])\
         .then(k=>{globalThis.__k=k});\
         crypto.subtle.importKey('raw', new Uint8Array([8]),\
             'AES-CBC', false, ['encrypt']).then(k=>{globalThis.__x=k});",
    );
    t.deliver_last(&mut e, vec![3, 1, 4, 1, 5, 9, 2, 6]);
    t.deliver_last(&mut e, vec![8]);
    let before = t.requests.borrow().len();
    eval(
        &mut e,
        "var raw=null;\
         crypto.subtle.exportKey('raw',__k).then(b=>{raw=[...new Uint8Array(b)].join(',')});",
    );
    e.run_microtasks().expect("pump");
    assert_eq!(describe(&mut e, "raw"), "3,1,4,1,5,9,2,6");
    assert_eq!(t.requests.borrow().len(), before, "exportKey = moteur seul");
    eval(
        &mut e,
        "globalThis.__x1='';globalThis.__j='';\
         crypto.subtle.exportKey('raw',__x).catch(x=>{__x1=x.name});\
         crypto.subtle.exportKey('jwk',__k).then(j=>{__j=j.kty+':'+j.k});",
    );
    e.run_microtasks().expect("pump");
    assert_eq!(describe(&mut e, "__x1"), "InvalidAccessError");
    // JWK `oct` — `k` = base64url de la matière brute.
    assert_eq!(describe(&mut e, "__j"), "oct:AwEEAQUJAgY");
}
