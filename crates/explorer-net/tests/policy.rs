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

/// Réponse streamée scriptée — `stall` retient le canal de corps
/// indéfiniment (corps jamais terminé : anti-bot, `close_delimited`
/// sans fin), pour prouver que l'amont n'attend pas le corps.
struct Canned {
    status: u16,
    headers: Vec<(String, String)>,
    chunks: Vec<Vec<u8>>,
    stall: bool,
}

fn canned(status: u16, headers: &[(&str, &str)], chunks: Vec<Vec<u8>>, stall: bool) -> Canned {
    Canned {
        status,
        headers: headers
            .iter()
            .map(|(n, v)| (n.to_string(), v.to_string()))
            .collect(),
        chunks,
        stall,
    }
}

struct StreamFake {
    responses: std::sync::Mutex<std::collections::VecDeque<Canned>>,
}

impl StreamFake {
    fn new(responses: Vec<Canned>) -> Arc<Self> {
        Arc::new(Self {
            responses: std::sync::Mutex::new(responses.into()),
        })
    }
}

#[async_trait::async_trait]
impl HttpTransport for StreamFake {
    async fn fetch(&self, _req: &Request) -> explorer_net::error::NetResult<Response> {
        unimplemented!("tests streamés uniquement")
    }

    async fn fetch_stream(
        &self,
        _req: &Request,
    ) -> explorer_net::error::NetResult<explorer_net::http::StreamedResponse> {
        let c = self
            .responses
            .lock()
            .unwrap()
            .pop_front()
            .expect("canned response manquante");
        let (tx, rx) = mpsc::channel(4);
        if c.stall {
            // Canal ouvert, jamais alimenté : le corps pend —
            // `fetch_stream` doit rendre les en-têtes sans lui.
            tokio::spawn(async move {
                let _hold = tx;
                std::future::pending::<()>().await;
            });
        } else {
            tokio::spawn(async move {
                for chunk in c.chunks {
                    if tx.send(Ok(chunk)).await.is_err() {
                        return;
                    }
                }
            });
        }
        Ok(explorer_net::http::StreamedResponse {
            status: c.status,
            reason: String::new(),
            headers: c.headers,
            reused_connection: false,
            resolved_url: None,
            body: rx,
        })
    }
}

#[tokio::test]
async fn stream_error_status_headers_early_stalled_body() {
    // Régression fnac.fr (~60 s → page d'erreur immédiate,
    // étape 101) : le hop 301 ET le 403 terminal portent des
    // corps retenus indéfiniment — ni l'un ni l'autre ne doit
    // retenir les en-têtes.
    let inner = StreamFake::new(vec![
        canned(301, &[("Location", "/target")], vec![], true),
        canned(403, &[("Content-Type", "text/html")], vec![], true),
    ]);
    let t = RedirectTransport::wrap(inner, 8);
    let r = tokio::time::timeout(
        Duration::from_secs(5),
        t.fetch_stream(&Request::get(u("http://a.example/"))),
    )
    .await
    .expect("les en-têtes streamés ne doivent pas attendre le corps")
    .unwrap();
    assert_eq!(r.status, 403);
    assert_eq!(r.resolved_url.unwrap().as_str(), "http://a.example/target");
}

#[tokio::test]
async fn stream_follows_redirect_and_delivers_body() {
    let inner = StreamFake::new(vec![
        canned(302, &[("Location", "/b")], vec![b"hop".to_vec()], false),
        canned(200, &[], vec![b"hel".to_vec(), b"lo".to_vec()], false),
    ]);
    let t = RedirectTransport::wrap(inner, 8);
    let mut r = t
        .fetch_stream(&Request::get(u("http://a.example/a")))
        .await
        .unwrap();
    assert_eq!(r.status, 200);
    assert_eq!(r.resolved_url.unwrap().as_str(), "http://a.example/b");
    let mut body = Vec::new();
    while let Some(chunk) = r.body.recv().await {
        body.extend_from_slice(&chunk.unwrap());
    }
    assert_eq!(body, b"hello");
}

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

/// `rel=noreferrer` (étape 131) : `Request::no_referer`
/// supprime l'injection du `Referer` même avec un
/// initiateur suivi — `Origin`, lui, n'est pas une
/// information de référent et reste émis.
#[tokio::test]
async fn no_referer_suppresses_injection() {
    let inner = FakeTransport::new(vec![resp(200, &[], b"x"), resp(200, &[], b"x")]);
    let t = RefererTransport::wrap(inner.clone(), Some(u("https://a.example/page")));
    let mut req = Request::get(u("https://b.example/"));
    req.no_referer = true;
    t.fetch(&req).await.unwrap();
    // Sans le drapeau : injection normale (contrôle).
    t.fetch(&Request::get(u("https://b.example/")))
        .await
        .unwrap();
    let seen = inner.seen();
    assert!(
        !seen[0].headers.iter().any(|(n, _)| n == "Referer"),
        "no_referer : aucun Referer injecté"
    );
    assert!(
        seen[1].headers.iter().any(|(n, _)| n == "Referer"),
        "contrôle : Referer injecté sans le drapeau"
    );
}

// ---- Étape 132 : `Request::referrer_policy` ----

use explorer_net::http::ReferrerPolicy;
use explorer_net::policy::referer_value_for;

/// `ReferrerPolicy::parse` : jetons reconnus, casse
/// tolérée, dernière valeur valide d'une liste virgulée
/// gagne, aucune reconnue → `None`.
#[test]
fn referrer_policy_parse_tokens() {
    use ReferrerPolicy::*;
    assert_eq!(ReferrerPolicy::parse("no-referrer"), Some(NoReferrer));
    assert_eq!(ReferrerPolicy::parse("Origin"), Some(Origin));
    assert_eq!(
        ReferrerPolicy::parse("strict-origin-when-cross-origin"),
        Some(StrictOriginWhenCrossOrigin)
    );
    assert_eq!(ReferrerPolicy::parse("always"), Some(UnsafeUrl));
    assert_eq!(
        ReferrerPolicy::parse("garbage, same-origin"),
        Some(SameOrigin),
        "dernière valide de la liste"
    );
    assert_eq!(ReferrerPolicy::parse("bogus"), None);
    assert_eq!(ReferrerPolicy::parse(""), None);
}

/// Matrice `referer_value_for` : chaque politique ×
/// (same-origin / cross-origin / rétrogradation).
#[test]
fn referrer_policy_matrix() {
    use ReferrerPolicy::*;
    let page = u("https://a.example/dir/page?x=1#frag");
    let same = u("https://a.example/other");
    let cross = u("https://b.example/x");
    let down = u("http://a.example/x");
    let f = |p, t: &Url| referer_value_for(&page, t, p);
    // no-referrer : jamais.
    assert_eq!(f(NoReferrer, &same), None);
    assert_eq!(f(NoReferrer, &cross), None);
    // no-referrer-when-downgrade : complet sauf rétrograd.
    assert_eq!(
        f(NoReferrerWhenDowngrade, &cross).as_deref(),
        Some("https://a.example/dir/page?x=1")
    );
    assert_eq!(f(NoReferrerWhenDowngrade, &down), None);
    // same-origin : complet same-origin seulement.
    assert_eq!(
        f(SameOrigin, &same).as_deref(),
        Some("https://a.example/dir/page?x=1")
    );
    assert_eq!(f(SameOrigin, &cross), None);
    // origin : origine seule même en rétrogradation.
    assert_eq!(f(Origin, &cross).as_deref(), Some("https://a.example/"));
    assert_eq!(f(Origin, &down).as_deref(), Some("https://a.example/"));
    // strict-origin : origine sauf rétrogradation.
    assert_eq!(
        f(StrictOrigin, &cross).as_deref(),
        Some("https://a.example/")
    );
    assert_eq!(f(StrictOrigin, &down), None);
    // origin-when-cross-origin : complet ou origine, même
    // en rétrogradation (`down` est cross — le schéma
    // diffère — donc origine, pas de suppression).
    assert_eq!(
        f(OriginWhenCrossOrigin, &down).as_deref(),
        Some("https://a.example/"),
        "downgrade : cross → origine, pas de coupure"
    );
    assert_eq!(
        f(OriginWhenCrossOrigin, &cross).as_deref(),
        Some("https://a.example/")
    );
    // strict-origin-when-cross-origin : le défaut historique.
    assert_eq!(
        f(StrictOriginWhenCrossOrigin, &same).as_deref(),
        Some("https://a.example/dir/page?x=1")
    );
    assert_eq!(
        f(StrictOriginWhenCrossOrigin, &cross).as_deref(),
        Some("https://a.example/")
    );
    assert_eq!(f(StrictOriginWhenCrossOrigin, &down), None);
    // unsafe-url : complet toujours.
    assert_eq!(
        f(UnsafeUrl, &down).as_deref(),
        Some("https://a.example/dir/page?x=1")
    );
}

/// `RefererTransport` : `req.referrer_policy` gouverne
/// l'en-tête émis ; `no_referer` reste prioritaire.
#[tokio::test]
async fn referrer_policy_governs_injection() {
    let inner = FakeTransport::new(vec![resp(200, &[], b"x"); 4]);
    let t = RefererTransport::wrap(inner.clone(), Some(u("https://a.example/dir/page?x=1")));
    // same-origin : URL complète seulement.
    let mut r1 = Request::get(u("https://b.example/x"));
    r1.referrer_policy = Some(ReferrerPolicy::SameOrigin);
    t.fetch(&r1).await.unwrap();
    // unsafe-url : complet même cross-origin.
    let mut r2 = Request::get(u("https://b.example/x"));
    r2.referrer_policy = Some(ReferrerPolicy::UnsafeUrl);
    t.fetch(&r2).await.unwrap();
    // no_referer l'emporte sur une politique permissive.
    let mut r3 = Request::get(u("https://b.example/x"));
    r3.referrer_policy = Some(ReferrerPolicy::UnsafeUrl);
    r3.no_referer = true;
    t.fetch(&r3).await.unwrap();
    // défaut : cross-origin → origine seule (contrôle).
    t.fetch(&Request::get(u("https://b.example/x")))
        .await
        .unwrap();
    let seen = inner.seen();
    let refv = |r: &Request| {
        r.headers
            .iter()
            .find(|(n, _)| n == "Referer")
            .map(|(_, v)| v.clone())
    };
    assert_eq!(refv(&seen[0]), None, "same-origin cross → rien");
    assert_eq!(
        refv(&seen[1]).as_deref(),
        Some("https://a.example/dir/page?x=1"),
        "unsafe-url cross → complet"
    );
    assert_eq!(refv(&seen[2]), None, "no_referer prioritaire");
    assert_eq!(
        refv(&seen[3]).as_deref(),
        Some("https://a.example/"),
        "défaut cross → origine"
    );
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

// -- Sec-Fetch-* (étape 125) ---------------------------------------------

use explorer_net::http::{FetchDest, FetchMode, Initiator};
use explorer_net::policy::sec_fetch_headers;

fn hdr<'a>(hs: &'a [(String, String)], name: &str) -> Option<&'a str> {
    hs.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str())
}

#[test]
fn sec_fetch_top_level_nav_is_none_document() {
    let mut req = Request::get(u("https://accounts.example/"));
    req.initiator = Some(Initiator::TopLevelNav);
    req.fetch_user = true;
    let h = sec_fetch_headers(&req);
    assert_eq!(hdr(&h, "sec-fetch-site"), Some("none"));
    assert_eq!(hdr(&h, "sec-fetch-mode"), Some("navigate"));
    assert_eq!(hdr(&h, "sec-fetch-dest"), Some("document"));
    assert_eq!(hdr(&h, "sec-fetch-user"), Some("?1"));
}

#[test]
fn sec_fetch_nav_from_same_origin() {
    let mut req = Request::get(u("https://a.example/next"));
    req.initiator = Some(Initiator::NavFrom(u("https://a.example/")));
    let h = sec_fetch_headers(&req);
    assert_eq!(hdr(&h, "sec-fetch-site"), Some("same-origin"));
    assert_eq!(hdr(&h, "sec-fetch-mode"), Some("navigate"));
    // Non marqué par l'émetteur → pas de `?1` (nav scriptée).
    assert!(hdr(&h, "sec-fetch-user").is_none());
}

#[test]
fn sec_fetch_subresource_same_site_vs_cross() {
    // Même registrable + même schéma, hôte différent → same-site.
    let mut req = Request::get(u("https://cdn.a.example/x.css"));
    req.initiator = Some(Initiator::Subresource(u("https://a.example/")));
    req.fetch_dest = FetchDest::Style;
    let h = sec_fetch_headers(&req);
    assert_eq!(hdr(&h, "sec-fetch-site"), Some("same-site"));
    assert_eq!(hdr(&h, "sec-fetch-mode"), Some("no-cors"));
    assert_eq!(hdr(&h, "sec-fetch-dest"), Some("style"));

    // Autre registrable → cross-site.
    let mut req = Request::get(u("https://b.example/x.png"));
    req.initiator = Some(Initiator::Subresource(u("https://a.example/")));
    req.fetch_dest = FetchDest::Image;
    let h = sec_fetch_headers(&req);
    assert_eq!(hdr(&h, "sec-fetch-site"), Some("cross-site"));
    assert_eq!(hdr(&h, "sec-fetch-dest"), Some("image"));
}

#[test]
fn sec_fetch_script_request_is_cors_empty() {
    // `fetch()`/XHR : mode `cors`, dest `empty` — la session pose
    // ces champs à la construction (`script_request`, étape 125).
    let mut req = Request::get(u("https://api.example/v1"));
    req.initiator = Some(Initiator::Subresource(u("https://a.example/")));
    req.fetch_mode = FetchMode::Cors;
    let h = sec_fetch_headers(&req);
    assert_eq!(hdr(&h, "sec-fetch-mode"), Some("cors"));
    assert_eq!(hdr(&h, "sec-fetch-dest"), Some("empty"));
}

#[test]
fn sec_fetch_skipped_off_http() {
    let mut req = Request::get(u("data:text/plain,x"));
    req.initiator = Some(Initiator::Subresource(u("https://a.example/")));
    assert_eq!(sec_fetch_headers(&req), Vec::<(String, String)>::new());
}

#[test]
fn sec_fetch_no_initiator_is_none() {
    // Outil sans page (CLI/fakes) — `none`, mode/dest des champs.
    let req = Request::get(u("https://a.example/x"));
    let h = sec_fetch_headers(&req);
    assert_eq!(hdr(&h, "sec-fetch-site"), Some("none"));
    assert_eq!(hdr(&h, "sec-fetch-mode"), Some("no-cors"));
    assert_eq!(hdr(&h, "sec-fetch-dest"), Some("empty"));
}

// -- CORS : CorsTransport (étape 126) -------------------------------------

use explorer_net::http::{CorsRequest, FetchCredentials};
use explorer_net::policy::CorsTransport;

fn req_hdr<'a>(req: &'a Request, name: &str) -> Option<&'a str> {
    req.headers
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}

fn cors_req(url: &str, method: Method, origin: &str, unsafe_h: &[&str]) -> Request {
    let mut req = Request::get(u(url));
    req.method = method;
    req.initiator = Some(Initiator::Subresource(u("https://page.example/")));
    req.fetch_mode = FetchMode::Cors;
    req.cors = Some(CorsRequest {
        origin: origin.to_string(),
        credentials: FetchCredentials::SameOrigin,
        unsafe_headers: unsafe_h.iter().map(ToString::to_string).collect(),
    });
    req
}

#[tokio::test]
async fn cors_simple_get_acao_star_passes() {
    // Requête simple + `ACAO: *` → pas de preflight, réponse livrée.
    let inner = FakeTransport::new(vec![resp(
        200,
        &[("access-control-allow-origin", "*")],
        b"ok",
    )]);
    let t = CorsTransport::wrap(inner.clone());
    let r = t
        .fetch(&cors_req(
            "https://api.example/x",
            Method::Get,
            "https://page.example",
            &[],
        ))
        .await
        .unwrap();
    assert_eq!(r.body, b"ok");
    // Une seule requête émise — aucun OPTIONS.
    assert_eq!(inner.seen().len(), 1);
}

#[tokio::test]
async fn cors_simple_get_acao_mismatch_blocked() {
    // `ACAO` ≠ origine → CorsBlocked (opaque côté script).
    let inner = FakeTransport::new(vec![resp(
        200,
        &[("access-control-allow-origin", "https://other.example")],
        b"ok",
    )]);
    let t = CorsTransport::wrap(inner.clone());
    let err = t
        .fetch(&cors_req(
            "https://api.example/x",
            Method::Get,
            "https://page.example",
            &[],
        ))
        .await
        .unwrap_err();
    assert!(matches!(err, NetError::CorsBlocked(_)), "{err}");
}

#[tokio::test]
async fn cors_preflight_put_then_request() {
    // PUT cross → OPTIONS d'abord ; `ACAM` couvrant PUT +
    // `ACAO` exact → la vraie requête part ensuite.
    let inner = FakeTransport::new(vec![
        resp(
            200,
            &[
                ("access-control-allow-origin", "https://page.example"),
                ("access-control-allow-methods", "GET, PUT"),
            ],
            b"",
        ),
        resp(
            200,
            &[("access-control-allow-origin", "https://page.example")],
            b"done",
        ),
    ]);
    let t = CorsTransport::wrap(inner.clone());
    let r = t
        .fetch(&cors_req(
            "https://api.example/x",
            Method::Put,
            "https://page.example",
            &[],
        ))
        .await
        .unwrap();
    assert_eq!(r.body, b"done");
    let seen = inner.seen();
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[0].method, Method::Options);
    assert_eq!(
        req_hdr(&seen[0], "access-control-request-method"),
        Some("PUT")
    );
    assert_eq!(seen[1].method, Method::Put);
}

#[tokio::test]
async fn cors_preflight_denied_method_blocks() {
    // `ACAM` ne couvre pas DELETE → bloqué, la vraie requête
    // ne part jamais.
    let inner = FakeTransport::new(vec![resp(
        200,
        &[
            ("access-control-allow-origin", "https://page.example"),
            ("access-control-allow-methods", "GET"),
        ],
        b"",
    )]);
    let t = CorsTransport::wrap(inner.clone());
    let err = t
        .fetch(&cors_req(
            "https://api.example/x",
            Method::Delete,
            "https://page.example",
            &[],
        ))
        .await
        .unwrap_err();
    assert!(matches!(err, NetError::CorsBlocked(_)), "{err}");
    assert_eq!(inner.seen().len(), 1);
}

#[tokio::test]
async fn cors_preflight_bad_status_blocks() {
    let inner = FakeTransport::new(vec![resp(403, &[], b"nope")]);
    let t = CorsTransport::wrap(inner.clone());
    let err = t
        .fetch(&cors_req(
            "https://api.example/x",
            Method::Put,
            "https://page.example",
            &[],
        ))
        .await
        .unwrap_err();
    assert!(matches!(err, NetError::CorsBlocked(_)), "{err}");
    assert_eq!(inner.seen().len(), 1);
}

#[tokio::test]
async fn cors_unsafe_header_checked_by_acah() {
    // `x-token` non safelisté → `ACRH: x-token` au preflight ;
    // `ACAH` qui ne le couvre pas → blocage, puis acceptation
    // quand l'en-tête est listé.
    let inner = FakeTransport::new(vec![resp(
        200,
        &[
            ("access-control-allow-origin", "https://page.example"),
            ("access-control-allow-methods", "*"),
            ("access-control-allow-headers", "content-type"),
        ],
        b"",
    )]);
    let t = CorsTransport::wrap(inner.clone());
    let err = t
        .fetch(&cors_req(
            "https://api.example/x",
            Method::Get,
            "https://page.example",
            &["x-token"],
        ))
        .await
        .unwrap_err();
    assert!(matches!(err, NetError::CorsBlocked(_)), "{err}");
    assert_eq!(
        req_hdr(&inner.seen()[0], "access-control-request-headers"),
        Some("x-token")
    );

    let inner = FakeTransport::new(vec![
        resp(
            200,
            &[
                ("access-control-allow-origin", "https://page.example"),
                ("access-control-allow-methods", "*"),
                ("access-control-allow-headers", "X-TOKEN, content-type"),
            ],
            b"",
        ),
        resp(
            200,
            &[("access-control-allow-origin", "https://page.example")],
            b"ok",
        ),
    ]);
    let t = CorsTransport::wrap(inner.clone());
    t.fetch(&cors_req(
        "https://api.example/x",
        Method::Get,
        "https://page.example",
        &["x-token"],
    ))
    .await
    .unwrap();
}

#[tokio::test]
async fn cors_include_requires_exact_acao_and_acac() {
    // `credentials:include` : `*` refusé, origine exacte sans
    // `ACAC` refusée, exact+`ACAC:true` acceptée.
    let mut req = cors_req(
        "https://api.example/x",
        Method::Get,
        "https://page.example",
        &[],
    );
    req.cors.as_mut().unwrap().credentials = FetchCredentials::Include;

    let inner = FakeTransport::new(vec![resp(
        200,
        &[("access-control-allow-origin", "*")],
        b"x",
    )]);
    let t = CorsTransport::wrap(inner.clone());
    let err = t.fetch(&req.clone()).await.unwrap_err();
    assert!(matches!(err, NetError::CorsBlocked(_)), "{err}");

    let inner = FakeTransport::new(vec![resp(
        200,
        &[("access-control-allow-origin", "https://page.example")],
        b"x",
    )]);
    let t = CorsTransport::wrap(inner.clone());
    let err = t.fetch(&req.clone()).await.unwrap_err();
    assert!(matches!(err, NetError::CorsBlocked(_)), "{err}");

    let inner = FakeTransport::new(vec![resp(
        200,
        &[
            ("access-control-allow-origin", "https://page.example"),
            ("access-control-allow-credentials", "true"),
        ],
        b"ok",
    )]);
    let t = CorsTransport::wrap(inner.clone());
    let r = t.fetch(&req).await.unwrap();
    assert_eq!(r.body, b"ok");
}

#[tokio::test]
async fn cors_json_content_type_triggers_preflight() {
    // `Content-Type: application/json` (safelisté en nom,
    // valeur hors types « form ») → preflight quand même.
    let mut req = cors_req(
        "https://api.example/x",
        Method::Post,
        "https://page.example",
        &[],
    );
    req.headers
        .push(("content-type".to_string(), "application/json".to_string()));
    let inner = FakeTransport::new(vec![
        resp(
            200,
            &[
                ("access-control-allow-origin", "https://page.example"),
                ("access-control-allow-methods", "POST"),
            ],
            b"",
        ),
        resp(
            200,
            &[("access-control-allow-origin", "https://page.example")],
            b"ok",
        ),
    ]);
    let t = CorsTransport::wrap(inner.clone());
    t.fetch(&req).await.unwrap();
    assert_eq!(inner.seen()[0].method, Method::Options);
}

#[tokio::test]
async fn cors_no_context_passthrough() {
    // Requête sans `cors` (sous-ressource `no-cors`, nav…) —
    // aucune vérification, réponse nue.
    let inner = FakeTransport::new(vec![resp(200, &[], b"raw")]);
    let t = CorsTransport::wrap(inner.clone());
    let mut req = Request::get(u("https://api.example/x"));
    req.initiator = Some(Initiator::Subresource(u("https://page.example/")));
    let r = t.fetch(&req).await.unwrap();
    assert_eq!(r.body, b"raw");
}
