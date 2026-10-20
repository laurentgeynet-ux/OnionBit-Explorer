// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Tests de la politique réseau : redirections 3xx, HSTS, proxy
//! (absolute-form + CONNECT), Referer/Origin, mixed-content.

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use explorer_net::error::NetError;
use explorer_net::h1::{H1Client, HttpConfig};
use explorer_net::http::{HttpTransport, Method, Request, Response};
use explorer_net::policy::{
    is_mixed_content, HstsStore, HstsTransport, PolicyTransport, RedirectTransport,
    RefererTransport,
};
use explorer_net::proxy::{ProxyConfig, ProxyConnector};
use explorer_net::url::Url;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::mpsc;

fn u(s: &str) -> Url {
    Url::parse(s).unwrap()
}

/// Transport fake scripté — file d'attente de réponses, log des
/// requêtes reçues (pour assertions sans sockets).
struct FakeTransport {
    responses: std::sync::Mutex<std::collections::VecDeque<Response>>,
    seen: std::sync::Mutex<Vec<Request>>,
}

impl FakeTransport {
    fn new(responses: Vec<Response>) -> Arc<Self> {
        Arc::new(Self {
            responses: std::sync::Mutex::new(responses.into()),
            seen: std::sync::Mutex::new(Vec::new()),
        })
    }
    fn seen(&self) -> Vec<Request> {
        self.seen.lock().unwrap().clone()
    }
}

fn resp(status: u16, headers: &[(&str, &str)], body: &[u8]) -> Response {
    Response {
        status,
        reason: String::new(),
        headers: headers
            .iter()
            .map(|(n, v)| (n.to_string(), v.to_string()))
            .collect(),
        body: body.to_vec(),
        reused_connection: false,
        resolved_url: None,
    }
}

#[async_trait::async_trait]
impl HttpTransport for FakeTransport {
    async fn fetch(&self, req: &Request) -> explorer_net::error::NetResult<Response> {
        self.seen.lock().unwrap().push(req.clone());
        Ok(self
            .responses
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| resp(500, &[], b"no more canned responses")))
    }
}

// -- RedirectTransport ---------------------------------------------------

#[tokio::test]
async fn follows_301_get() {
    let inner = FakeTransport::new(vec![
        resp(301, &[("Location", "/new")], &[]),
        resp(200, &[], b"done"),
    ]);
    let t = RedirectTransport::wrap(inner.clone(), 8);
    let r = t
        .fetch(&Request::get(u("http://a.example/old")))
        .await
        .unwrap();
    assert_eq!(r.status, 200);
    assert_eq!(r.body, b"done");
    // La 2e requête a résolu /new contre la base.
    assert_eq!(inner.seen()[1].url.as_str(), "http://a.example/new");
}

#[tokio::test]
async fn post_303_becomes_get() {
    let inner = FakeTransport::new(vec![
        resp(303, &[("Location", "/done")], &[]),
        resp(200, &[], b"ok"),
    ]);
    let t = RedirectTransport::wrap(inner.clone(), 8);
    let mut req = Request::get(u("http://a.example/submit"));
    req.method = Method::Post;
    req.body = Some(b"x".to_vec());
    t.fetch(&req).await.unwrap();
    let second = &inner.seen()[1];
    assert_eq!(second.method, Method::Get);
    assert!(second.body.is_none());
}

#[tokio::test]
async fn post_307_preserves_method_and_body() {
    let inner = FakeTransport::new(vec![
        resp(307, &[("Location", "/again")], &[]),
        resp(200, &[], b"ok"),
    ]);
    let t = RedirectTransport::wrap(inner.clone(), 8);
    let mut req = Request::get(u("http://a.example/up"));
    req.method = Method::Post;
    req.body = Some(b"data".to_vec());
    t.fetch(&req).await.unwrap();
    let second = &inner.seen()[1];
    assert_eq!(second.method, Method::Post);
    assert_eq!(second.body.as_deref(), Some(b"data".as_slice()));
}

#[tokio::test]
async fn redirect_loop_bounded() {
    let inner = FakeTransport::new(vec![
        resp(301, &[("Location", "/a")], &[]),
        resp(301, &[("Location", "/b")], &[]),
        resp(301, &[("Location", "/a")], &[]),
        resp(301, &[("Location", "/b")], &[]),
        resp(301, &[("Location", "/a")], &[]),
    ]);
    let t = RedirectTransport::wrap(inner, 3);
    let err = t
        .fetch(&Request::get(u("http://a.example/start")))
        .await
        .unwrap_err();
    assert!(matches!(err, NetError::TooManyRedirects(3)), "{err}");
}

#[tokio::test]
async fn three_xx_without_location_passthrough() {
    let inner = FakeTransport::new(vec![resp(304, &[], &[])]);
    let t = RedirectTransport::wrap(inner, 8);
    let r = t
        .fetch(&Request::get(u("http://a.example/x")))
        .await
        .unwrap();
    assert_eq!(r.status, 304);
}

// -- HSTS -----------------------------------------------------------------

#[test]
fn hsts_upgrade_exact_and_subdomain() {
    let store = HstsStore::new(100);
    let t0 = SystemTime::now();
    store.record_at("example.com", "max-age=3600; includeSubDomains", t0);
    assert!(store.should_upgrade_at("example.com", t0));
    assert!(store.should_upgrade_at("a.b.example.com", t0));
    // Expiré → pas d'upgrade.
    assert!(!store.should_upgrade_at("example.com", t0 + Duration::from_secs(3601)));
    // Sans includeSubDomains, le sous-domaine n'est pas couvert.
    let store2 = HstsStore::new(100);
    store2.record_at("example.com", "max-age=3600", t0);
    assert!(!store2.should_upgrade_at("sub.example.com", t0));
    assert!(store2.should_upgrade_at("example.com", t0));
}

#[test]
fn hsts_max_age_zero_revokes() {
    let store = HstsStore::new(100);
    store.record("example.com", "max-age=3600");
    assert!(store.should_upgrade("example.com"));
    store.record("example.com", "max-age=0");
    assert!(!store.should_upgrade("example.com"));
}

#[tokio::test]
async fn hsts_upgrades_url_on_wire() {
    let store = HstsStore::new(100);
    store.record("a.example", "max-age=3600");
    let inner = FakeTransport::new(vec![resp(200, &[], b"x")]);
    let t = HstsTransport::wrap(inner.clone(), store);
    t.fetch(&Request::get(u("http://a.example/p")))
        .await
        .unwrap();
    assert_eq!(inner.seen()[0].url.scheme(), "https");
}

#[tokio::test]
async fn hsts_captures_header_on_https() {
    let store = HstsStore::new(100);
    let inner = FakeTransport::new(vec![resp(
        200,
        &[(
            "Strict-Transport-Security",
            "max-age=3600; includeSubDomains",
        )],
        b"x",
    )]);
    let t = HstsTransport::wrap(inner, store.clone());
    t.fetch(&Request::get(u("https://a.example/")))
        .await
        .unwrap();
    assert!(store.should_upgrade("sub.a.example"));
}

// -- Proxy -----------------------------------------------------------------

#[test]
fn proxy_from_env_bypass() {
    let cfg = ProxyConfig {
        http: Some(u("http://proxy.local:8080")),
        https: None,
        no_proxy: vec![".internal".to_string(), "*.test".to_string()],
    };
    assert!(cfg.is_bypassed("host.internal"));
    assert!(cfg.is_bypassed("deep.sub.internal"));
    assert!(cfg.is_bypassed("x.test"));
    assert!(!cfg.is_bypassed("example.com"));
    assert_eq!(
        cfg.proxy_for("http").map(Url::host),
        Some("proxy.local".to_string())
    );
    assert!(cfg.proxy_for("https").is_none());
}

/// Serveur proxy : répond à la requête, log pour vérifier la
/// forme absolute.
async fn serve_proxy_once() -> (u16, mpsc::UnboundedReceiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (tx, rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        let Ok((mut sock, _)) = listener.accept().await else {
            return;
        };
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
        let _ = sock
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\nprox")
            .await;
    });
    (port, rx)
}

#[tokio::test]
async fn http_via_proxy_absolute_form() {
    let (port, mut rx) = serve_proxy_once().await;
    let cfg = ProxyConfig {
        http: Some(u(&format!("http://127.0.0.1:{port}"))),
        https: None,
        no_proxy: vec![],
    };
    let client = H1Client::new(Arc::new(ProxyConnector::new(cfg)), HttpConfig::default());
    let r = client
        .fetch(&Request::get(u("http://target.example/path")))
        .await
        .unwrap();
    assert_eq!(r.body, b"prox");
    let sent = rx.recv().await.unwrap();
    // Absolute-form sur le fil : `GET http://target.example/path`.
    assert!(
        sent.starts_with("GET http://target.example/path HTTP/1.1"),
        "{sent}"
    );
}

#[tokio::test]
async fn no_proxy_bypasses() {
    let (port, _rx) = serve_proxy_once().await;
    let cfg = ProxyConfig {
        http: Some(u("http://proxy.local:9")), // inaccessible
        https: None,
        no_proxy: vec!["127.0.0.1".to_string()],
    };
    // La cible est bypassée → connexion directe au serveur loopback.
    let client = H1Client::new(Arc::new(ProxyConnector::new(cfg)), HttpConfig::default());
    let r = client
        .fetch(&Request::get(u(&format!("http://127.0.0.1:{port}/direct"))))
        .await
        .unwrap();
    assert_eq!(r.body, b"prox");
}

// -- Referer / Origin -------------------------------------------------------

#[tokio::test]
async fn referer_full_same_origin() {
    let inner = FakeTransport::new(vec![resp(200, &[], b"x")]);
    let t = RefererTransport::wrap(inner.clone(), Some(u("https://a.example/page?x=1#f")));
    t.fetch(&Request::get(u("https://a.example/sub")))
        .await
        .unwrap();
    let seen = inner.seen();
    let referer = seen[0]
        .headers
        .iter()
        .find(|(n, _)| n == "Referer")
        .map(|(_, v)| v.as_str());
    // URL complète, sans fragment.
    assert_eq!(referer, Some("https://a.example/page?x=1"));
}

#[tokio::test]
async fn referer_origin_only_cross_origin() {
    let inner = FakeTransport::new(vec![resp(200, &[], b"x")]);
    let t = RefererTransport::wrap(inner.clone(), Some(u("https://a.example/secret/path")));
    t.fetch(&Request::get(u("https://b.example/cdn")))
        .await
        .unwrap();
    let seen = inner.seen();
    let referer = seen[0]
        .headers
        .iter()
        .find(|(n, _)| n == "Referer")
        .map(|(_, v)| v.as_str());
    // Cross-origin → origine seule, pas le chemin.
    assert_eq!(referer, Some("https://a.example/"));
}

#[tokio::test]
async fn referer_dropped_https_to_http() {
    let inner = FakeTransport::new(vec![resp(200, &[], b"x")]);
    let t = RefererTransport::wrap(inner.clone(), Some(u("https://a.example/")));
    t.fetch(&Request::get(u("http://b.example/")))
        .await
        .unwrap();
    let seen = inner.seen();
    assert!(!seen[0].headers.iter().any(|(n, _)| n == "Referer"));
}

#[tokio::test]
async fn origin_on_cross_origin_post() {
    let inner = FakeTransport::new(vec![resp(200, &[], b"x")]);
    let t = RefererTransport::wrap(inner.clone(), Some(u("https://a.example/")));
    let mut req = Request::get(u("https://b.example/api"));
    req.method = Method::Post;
    t.fetch(&req).await.unwrap();
    let seen = inner.seen();
    let origin = seen[0]
        .headers
        .iter()
        .find(|(n, _)| n == "Origin")
        .map(|(_, v)| v.as_str());
    assert_eq!(origin, Some("https://a.example"));
}

// -- Mixed-content ------------------------------------------------------------

#[test]
fn mixed_content_detection() {
    let secure_page = u("https://a.example/");
    let plain_page = u("http://a.example/");
    assert!(is_mixed_content(
        &secure_page,
        &u("http://cdn.example/x.png")
    ));
    assert!(!is_mixed_content(
        &secure_page,
        &u("https://cdn.example/x.png")
    ));
    assert!(!is_mixed_content(&plain_page, &u("http://x.example/")));
    // localhost toléré (dev).
    assert!(!is_mixed_content(
        &secure_page,
        &u("http://localhost:8080/x")
    ));
    assert!(!is_mixed_content(&secure_page, &u("http://127.0.0.1/x")));
    // data:/about: exempts.
    assert!(!is_mixed_content(&secure_page, &u("data:text/plain,x")));
}

#[tokio::test]
async fn policy_blocks_mixed_content() {
    let inner = FakeTransport::new(vec![resp(200, &[], b"x")]);
    let t = PolicyTransport::wrap(inner.clone(), Some(u("https://a.example/")));
    let err = t
        .fetch(&Request::get(u("http://cdn.example/x.png")))
        .await
        .unwrap_err();
    assert!(matches!(err, NetError::BlockedMixedContent(_)), "{err}");
    // Rien n'est parti sur le fil.
    assert!(inner.seen().is_empty());
    // https → autorisé.
    t.fetch(&Request::get(u("https://cdn.example/x.png")))
        .await
        .unwrap();
    assert_eq!(inner.seen().len(), 1);
}
