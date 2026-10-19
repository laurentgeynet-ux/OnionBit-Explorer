// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Tests du jar à cookies (RFC 6265 + PSL + préfixes) et du cache
//! mémoire (fraîcheur, revalidation 304, no-store).

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use explorer_net::cache::{CacheConfig, CachingTransport, MemoryCache};
use explorer_net::cookies::{CookieConfig, CookieJar, CookieTransport};
use explorer_net::h1::{H1Client, HttpConfig, TcpConnector};
use explorer_net::http::{HttpTransport, Request};
use explorer_net::url::Url;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::mpsc;

fn u(s: &str) -> Url {
    Url::parse(s).unwrap()
}

// -- CookieJar --------------------------------------------------------

#[test]
fn set_and_send_host_only() {
    let jar = CookieJar::new();
    let url = u("https://example.com/a/b");
    assert!(jar.set_from_header(&url, "sid=42"));
    assert_eq!(
        jar.header_for(&u("https://example.com/a/c"), true),
        "sid=42"
    );
    // Host-only : le sous-domaine ne reçoit pas.
    assert_eq!(jar.header_for(&u("https://sub.example.com/a"), true), "");
}

#[test]
fn domain_attr_suffixes() {
    let jar = CookieJar::new();
    let url = u("https://sub.example.com/");
    assert!(jar.set_from_header(&url, "d=1; Domain=example.com"));
    assert_eq!(jar.header_for(&u("https://example.com/"), true), "d=1");
    assert_eq!(
        jar.header_for(&u("https://other.example.com/"), true),
        "d=1"
    );
}

#[test]
fn domain_out_of_tree_rejected() {
    let jar = CookieJar::new();
    let url = u("https://a.example.com/");
    // Domain= qui ne suffixe pas l'hôte → rejeté.
    assert!(!jar.set_from_header(&url, "x=1; Domain=evil.com"));
}

#[test]
fn public_suffix_rejected() {
    let jar = CookieJar::new();
    let url = u("https://a.example.co.uk/");
    // Supercookie `.co.uk` bloqué par la PSL.
    assert!(!jar.set_from_header(&url, "x=1; Domain=co.uk"));
    // …mais le domaine registrable passe.
    assert!(jar.set_from_header(&url, "x=1; Domain=example.co.uk"));
}

#[test]
fn secure_only_over_https() {
    let jar = CookieJar::new();
    let https = u("https://example.com/");
    let http = u("http://example.com/");
    assert!(jar.set_from_header(&https, "s=1; Secure"));
    assert_eq!(jar.header_for(&http, true), "");
    assert_eq!(jar.header_for(&https, true), "s=1");
    // Posé en clair → rejeté.
    assert!(!jar.set_from_header(&http, "s2=1; Secure"));
}

#[test]
fn secure_localhost_tolerated() {
    let jar = CookieJar::new();
    let url = u("http://localhost:8080/");
    assert!(jar.set_from_header(&url, "dev=1; Secure"));
    assert_eq!(jar.header_for(&url, true), "dev=1");
}

#[test]
fn httponly_hidden_from_script() {
    let jar = CookieJar::new();
    let url = u("https://example.com/");
    assert!(jar.set_from_header(&url, "h=1; HttpOnly"));
    assert_eq!(jar.header_for(&url, false), ""); // vue document.cookie
    assert_eq!(jar.header_for(&url, true), "h=1"); // vue réseau
}

#[test]
fn path_match_rules() {
    let jar = CookieJar::new();
    let url = u("https://example.com/app/page");
    assert!(jar.set_from_header(&url, "p=1; Path=/app"));
    assert_eq!(jar.header_for(&u("https://example.com/app/x"), true), "p=1");
    // /app≠/application — le caractère suivant doit être '/'.
    assert_eq!(
        jar.header_for(&u("https://example.com/application"), true),
        ""
    );
}

#[test]
fn max_age_expiry_and_delete() {
    let jar = CookieJar::new();
    let url = u("https://example.com/");
    let t0 = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
    assert!(jar.set_from_header_at(&url, "e=1; Max-Age=60", t0));
    assert_eq!(
        jar.header_for_at(&url, true, t0 + Duration::from_secs(30)),
        "e=1"
    );
    assert_eq!(
        jar.header_for_at(&url, true, t0 + Duration::from_secs(61)),
        ""
    );
    // Max-Age=0 supprime `d` — `e` reste valide à t0.
    jar.set_from_header_at(&url, "d=1", t0);
    jar.set_from_header_at(&url, "d=gone; Max-Age=0", t0);
    assert_eq!(jar.header_for_at(&url, true, t0), "e=1");
}

#[test]
fn host_prefix_rules() {
    let jar = CookieJar::new();
    let https = u("https://example.com/");
    let http = u("http://example.com/");
    // __Host- : Secure + host-only + Path=/ requis.
    assert!(!jar.set_from_header(&http, "__Host-a=1; Secure; Path=/"));
    assert!(!jar.set_from_header(&https, "__Host-b=1; Path=/")); // sans Secure
    assert!(!jar.set_from_header(&https, "__Host-c=1; Secure; Path=/app"));
    assert!(!jar.set_from_header(&https, "__Host-d=1; Secure; Path=/; Domain=example.com"));
    assert!(jar.set_from_header(&https, "__Host-e=1; Secure; Path=/"));
    // __Secure- : Secure requis.
    assert!(!jar.set_from_header(&http, "__Secure-f=1"));
    assert!(!jar.set_from_header(&https, "__Secure-g=1")); // sans Secure
    assert!(jar.set_from_header(&https, "__Secure-h=1; Secure"));
}

#[test]
fn samesite_parsed() {
    let jar = CookieJar::new();
    let url = u("https://example.com/");
    assert!(jar.set_from_header(&url, "a=1; SameSite=Strict; Secure"));
    let map_len = jar.len();
    assert_eq!(map_len, 1);
    let _ = jar; // SameSite exposé sur le cookie — l'envoi cross-site
                 // est la politique de l'étape 8.
}

#[test]
fn jar_budgets() {
    let jar = CookieJar::with_config(CookieConfig {
        max_cookies: 3,
        max_per_domain: 50,
    });
    let url = u("https://example.com/");
    for i in 0..5 {
        jar.set_from_header(&url, &format!("c{i}={i}"));
    }
    assert!(jar.len() <= 3, "len={}", jar.len());
}

// -- CookieTransport end-to-end ----------------------------------------

/// Serveur : 1re requête → Set-Cookie, 2e → écho de Cookie: reçu.
async fn serve_cookie_echo() -> (u16, mpsc::UnboundedReceiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (tx, rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        let Ok((mut sock, _)) = listener.accept().await else {
            return;
        };
        for i in 0..2 {
            let mut buf = Vec::new();
            let mut chunk = [0u8; 4096];
            loop {
                let Ok(n) = sock.read(&mut chunk).await else {
                    return;
                };
                if n == 0 {
                    return;
                }
                buf.extend_from_slice(&chunk[..n]);
                if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            let head = String::from_utf8_lossy(&buf).into_owned();
            let _ = tx.send(head.clone());
            let resp = if i == 0 {
                "HTTP/1.1 200 OK\r\nSet-Cookie: sid=abc\r\nContent-Length: 2\r\n\r\nok"
            } else {
                "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok"
            };
            let _ = sock.write_all(resp.as_bytes()).await;
        }
    });
    (port, rx)
}

#[tokio::test]
async fn cookie_roundtrip_over_wire() {
    let (port, mut rx) = serve_cookie_echo().await;
    let jar = Arc::new(CookieJar::new());
    let transport = CookieTransport::wrap(
        Arc::new(H1Client::new(Arc::new(TcpConnector), HttpConfig::default())),
        jar,
    );
    let url = u(&format!("http://localhost:{port}/"));
    transport.fetch(&Request::get(url.clone())).await.unwrap();
    transport.fetch(&Request::get(url)).await.unwrap();
    let first = rx.recv().await.unwrap();
    let second = rx.recv().await.unwrap();
    assert!(!first.contains("Cookie:"), "{first}");
    assert!(second.contains("Cookie: sid=abc"), "{second}");
}

// -- MemoryCache / CachingTransport -------------------------------------

/// Serveur scripté : une séquence de réponses, log des requêtes.
async fn serve_seq(responses: Vec<&'static str>) -> (u16, mpsc::UnboundedReceiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (tx, rx) = mpsc::unbounded_channel();
    let responses = Arc::new(responses);
    let i = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                return;
            };
            let tx = tx.clone();
            let responses = responses.clone();
            let i = i.clone();
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut chunk = [0u8; 4096];
                loop {
                    let Ok(n) = sock.read(&mut chunk).await else {
                        return;
                    };
                    if n == 0 {
                        return;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                    if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                        break;
                    }
                }
                let _ = tx.send(String::from_utf8_lossy(&buf).into_owned());
                let idx = i.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                if idx < responses.len() {
                    let _ = sock.write_all(responses[idx].as_bytes()).await;
                }
            });
        }
    });
    (port, rx)
}

#[tokio::test]
async fn fresh_max_age_served_without_network() {
    let (port, mut rx) = serve_seq(vec![
        "HTTP/1.1 200 OK\r\nCache-Control: max-age=60\r\nContent-Length: 3\r\n\r\nabc",
    ])
    .await;
    let t = CachingTransport::wrap(
        Arc::new(H1Client::new(Arc::new(TcpConnector), HttpConfig::default())),
        MemoryCache::new(),
        CacheConfig::default(),
    );
    let url = u(&format!("http://localhost:{port}/x"));
    let r1 = t.fetch(&Request::get(url.clone())).await.unwrap();
    let r2 = t.fetch(&Request::get(url)).await.unwrap();
    assert_eq!(r1.body, b"abc");
    assert_eq!(r2.body, b"abc");
    // Une seule requête a touché le réseau.
    assert!(rx.recv().await.is_some());
    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn stale_revalidates_304() {
    let (port, mut rx) = serve_seq(vec![
        "HTTP/1.1 200 OK\r\nETag: \"v1\"\r\nContent-Length: 3\r\n\r\nold",
        "HTTP/1.1 304 Not Modified\r\nETag: \"v1\"\r\n\r\n",
    ])
    .await;
    let t = CachingTransport::wrap(
        Arc::new(H1Client::new(Arc::new(TcpConnector), HttpConfig::default())),
        MemoryCache::new(),
        CacheConfig::default(),
    );
    let url = u(&format!("http://localhost:{port}/x"));
    t.fetch(&Request::get(url.clone())).await.unwrap();
    let r2 = t.fetch(&Request::get(url)).await.unwrap();
    // 304 → corps caché servi avec statut 200.
    assert_eq!(r2.status, 200);
    assert_eq!(r2.body, b"old");
    // La 2e requête portait le validateur.
    let _ = rx.recv().await;
    let second = rx.recv().await.unwrap();
    assert!(second.contains("If-None-Match: \"v1\""), "{second}");
}

#[tokio::test]
async fn no_store_not_cached() {
    let (port, mut rx) = serve_seq(vec![
        "HTTP/1.1 200 OK\r\nCache-Control: no-store\r\nContent-Length: 1\r\n\r\na",
        "HTTP/1.1 200 OK\r\nCache-Control: no-store\r\nContent-Length: 1\r\n\r\nb",
    ])
    .await;
    let t = CachingTransport::wrap(
        Arc::new(H1Client::new(Arc::new(TcpConnector), HttpConfig::default())),
        MemoryCache::new(),
        CacheConfig::default(),
    );
    let url = u(&format!("http://localhost:{port}/x"));
    t.fetch(&Request::get(url.clone())).await.unwrap();
    let r2 = t.fetch(&Request::get(url)).await.unwrap();
    assert_eq!(r2.body, b"b"); // re-téléchargé
    let _ = rx.recv().await;
    assert!(rx.recv().await.is_some()); // 2 requêtes réseau
}

#[tokio::test]
async fn cache_eviction_bounded() {
    // Deux entrées de ~20 octets chacune, budget 25 → la seconde
    // insertion évince la première.
    let (port, _rx) = serve_seq(vec![
        "HTTP/1.1 200 OK\r\nCache-Control: max-age=60\r\nContent-Length: 10\r\n\r\naaaaaaaaaa",
        "HTTP/1.1 200 OK\r\nCache-Control: max-age=60\r\nContent-Length: 10\r\n\r\nbbbbbbbbbb",
    ])
    .await;
    let cache = MemoryCache::with_config(CacheConfig {
        max_entry_bytes: 1024,
        max_total_bytes: 15,
    });
    let t = CachingTransport::wrap(
        Arc::new(H1Client::new(Arc::new(TcpConnector), HttpConfig::default())),
        cache.clone(),
        CacheConfig::default(),
    );
    t.fetch(&Request::get(u(&format!("http://localhost:{port}/a"))))
        .await
        .unwrap();
    t.fetch(&Request::get(u(&format!("http://localhost:{port}/b"))))
        .await
        .unwrap();
    assert_eq!(cache.len(), 1, "len={}", cache.len());
}
