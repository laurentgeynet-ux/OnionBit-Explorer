// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! JS-C453 (JT-9) — `crypto.subtle.digest` : repli SHA-2
//! natif (zéro-dep) quand aucun `CryptoTransport` n'est
//! injecté — §B-117/odysee abortait sur « `SubtleCrypto` not
//! available ». Vecteurs KAT FIPS 180-4.

use explorer_jscore::{Dom, Engine};
use std::cell::RefCell;
use std::rc::Rc;

fn engine() -> Engine {
    let mut e = Engine::new();
    e.install_dom(&Rc::new(RefCell::new(Dom::from_html(
        "<html><body></body></html>",
    ))));
    e
}

fn digest_hex(e: &mut Engine, algo: &str, src: &str) -> String {
    e.eval(&format!(
        "crypto.subtle.digest('{algo}', new TextEncoder().encode(\"{src}\"))\
             .then(b => {{ window.__h = \
             [...new Uint8Array(b)].map(x => x.toString(16).padStart(2, '0')).join(''); }})"
    ))
    .expect("digest");
    e.run_microtasks().expect("drain");
    e.eval("__h").map(|v| e.describe(v)).unwrap()
}

#[test]
fn c453_digest_sha256_kat_sans_backend() {
    let mut e = engine();
    assert_eq!(
        digest_hex(&mut e, "SHA-256", "abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(
        digest_hex(&mut e, "SHA-256", ""),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
}

#[test]
fn c453_digest_sha384_sha512_kat() {
    let mut e = engine();
    assert_eq!(
        digest_hex(&mut e, "SHA-384", "abc"),
        "cb00753f45a35e8bb5a03d699ac65007272c32ab0eded1631a8b605a43ff5bed\
         8086072ba1e7cc2358baeca134c825a7"
    );
    assert_eq!(
        digest_hex(&mut e, "SHA-512", "abc"),
        "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a\
         2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f"
    );
}

#[test]
fn c453_digest_algo_inconnu_rejette_toujours() {
    let mut e = engine();
    e.eval(
        "crypto.subtle.digest('MD5', new TextEncoder().encode('x'))\
             .catch(er => { window.__e = er.name; })",
    )
    .expect("eval");
    e.run_microtasks().expect("drain");
    let r = e.eval("__e").map(|v| e.describe(v)).unwrap();
    assert_eq!(r, "NotSupportedError");
}
