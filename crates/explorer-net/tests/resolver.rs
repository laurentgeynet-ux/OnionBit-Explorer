// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Tests du résolveur `DoH` (étape 72) — endpoint simulé par
//! un transport en mémoire, jamais de socket réelle.

use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use explorer_net::error::{NetError, NetResult};
use explorer_net::http::{HttpTransport, Request, Response};
use explorer_net::resolver::{DohResolver, Resolver, SystemResolver};
use explorer_net::url::Url;

/// Transport en mémoire : une réponse DNS préfabriquée —
/// la requête est capturée pour vérifier le filaire.
struct StubDns {
    /// Réponse `application/dns-message` à renvoyer.
    body: Vec<u8>,
    /// Corps de la requête `DoH` capturé.
    captured: Mutex<Vec<u8>>,
}

#[async_trait]
impl HttpTransport for StubDns {
    async fn fetch(&self, req: &Request) -> NetResult<Response> {
        *self
            .captured
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            req.body.clone().unwrap_or_default();
        Ok(Response {
            status: 200,
            reason: "OK".to_string(),
            headers: vec![(
                "Content-Type".to_string(),
                "application/dns-message".to_string(),
            )],
            body: self.body.clone(),
            reused_connection: false,
            resolved_url: None,
        })
    }
}

/// Réponse DNS `A` pour `example.com` → 93.184.216.34 —
/// nom de RR compressé par pointeur vers la question.
fn dns_response_example() -> Vec<u8> {
    let mut q = vec![
        0xe1, 0x10, // id
        0x81, 0x80, // réponse, pas d'erreur
        0x00, 0x01, // QDCOUNT
        0x00, 0x01, // ANCOUNT
        0x00, 0x00, 0x00, 0x00,
    ];
    q.extend_from_slice(b"\x07example\x03com\x00");
    q.extend_from_slice(&[0x00, 0x01, 0x00, 0x01]); // A IN
    q.extend_from_slice(&[
        0xc0, 0x0c, // name → question
        0x00, 0x01, // type A
        0x00, 0x01, // IN
        0x00, 0x00, 0x01, 0x2c, // TTL 300
        0x00, 0x04, // RDLENGTH
        93, 184, 216, 34,
    ]);
    q
}

fn stub() -> Arc<StubDns> {
    Arc::new(StubDns {
        body: dns_response_example(),
        captured: Mutex::new(Vec::new()),
    })
}

#[tokio::test]
async fn doh_resolves_via_posted_dns_message() {
    let t = stub();
    let r = DohResolver::new(
        Url::parse("https://dns.test/dns-query").unwrap(),
        t.clone(),
        Duration::from_secs(5),
    );
    // Nom ordinaire (`example.com` est désormais d'usage
    // spécial — étape 78 — et court-circuite vers le
    // système).
    let ips = r.resolve("webfix.dev").await.unwrap();
    assert!(ips.contains(&IpAddr::from([93, 184, 216, 34])));
    // La requête est bien un dns-message POSTé (question A).
    let got = t
        .captured
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    assert!(got
        .windows(b"\x06webfix\x03dev\x00".len())
        .any(|w| w == b"\x06webfix\x03dev\x00"));
    assert_eq!(r.describe(), "doh:https://dns.test/dns-query");
}

#[tokio::test]
async fn doh_caches_per_host_within_ttl() {
    let t = stub();
    let r = DohResolver::new(
        Url::parse("https://dns.test/dns-query").unwrap(),
        t.clone(),
        Duration::from_secs(5),
    );
    let a = r.resolve("webfix.dev").await.unwrap();
    let b = r.resolve("webfix.dev").await.unwrap();
    assert_eq!(a, b);
}

/// Étape 78 — les noms d'usage spécial (RFC 6761 + `.local`
/// mDNS) ne partent **jamais** en requête `DoH` :
/// délégation système, comme Chrome/Firefox — un résolveur
/// public n'a aucune réponse légitime pour eux.
#[tokio::test]
async fn doh_bypasses_special_use_names() {
    let t = stub();
    let r = DohResolver::new(
        Url::parse("https://dns.test/dns-query").unwrap(),
        t.clone(),
        Duration::from_secs(5),
    );
    let clear = || {
        t.captured
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_empty()
    };
    // `localhost` : le système répond en loopback — et la
    // sonde n'a été touchée par aucune requête.
    let ips = r.resolve("localhost").await.unwrap();
    assert!(!ips.is_empty() && ips.iter().all(IpAddr::is_loopback));
    assert!(clear(), "localhost est parti en DoH");
    // Le reste de la famille : succès ou échec honnête du
    // système — le transport DoH reste vierge dans tous
    // les cas (casse et point final couverts).
    for h in [
        "svc.localhost",
        "dev.local",
        "app.TEST.",
        "x.invalid",
        "doc.example",
        "example.com",
        "example.net",
        "example.org",
    ] {
        let _ = r.resolve(h).await;
        assert!(clear(), "{h} est parti en DoH");
    }
    // Un nom ordinaire passe bien par le résolveur DoH.
    let _ = r.resolve("webfix.dev").await;
    assert!(!clear(), "un nom ordinaire doit être requêté");
}

/// Un endpoint qui renvoie autre chose que du dns-message
/// → erreur typée, jamais de boucle infinie.
struct BadContent;

#[async_trait]
impl HttpTransport for BadContent {
    async fn fetch(&self, _req: &Request) -> NetResult<Response> {
        Ok(Response {
            status: 200,
            reason: "OK".to_string(),
            headers: vec![("Content-Type".to_string(), "text/html".to_string())],
            body: b"<html>nope</html>".to_vec(),
            reused_connection: false,
            resolved_url: None,
        })
    }
}

#[tokio::test]
async fn doh_rejects_non_dns_message() {
    let r = DohResolver::new(
        Url::parse("https://dns.test/dns-query").unwrap(),
        Arc::new(BadContent),
        Duration::from_secs(5),
    );
    let e = r.resolve("webfix.dev").await.unwrap_err();
    assert!(matches!(e, NetError::Dns { .. }));
}

#[test]
fn system_resolver_is_default() {
    assert_eq!(SystemResolver.describe(), "system");
}
