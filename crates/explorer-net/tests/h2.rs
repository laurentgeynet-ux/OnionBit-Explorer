// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Tests `AutoClient` (étape 41) : ALPN `h2` → multiplexage sur
//! une vraie connexion `h2` (serveur in-process sur duplex),
//! ALPN `http/1.1`/absent → repli h1 sur la connexion négociée.

use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use explorer_net::error::NetResult;
use explorer_net::h1::{Conn, Connector, HttpConfig, IoStream};
use explorer_net::h2::AutoClient;
use explorer_net::http::{FetchDest, FetchMode, HttpTransport, Method, Request};
use explorer_net::url::Url;
use tokio::io::{duplex, DuplexStream};

/// File de flots serveur : chaque `connect` produit une paire
/// `duplex` — le client côté connecteur, le serveur côté test.
type Ends = Arc<Mutex<Vec<DuplexStream>>>;

/// Connecteur simulé : file de paires duplex + ALPN fixé.
struct FakeConnector {
    ends: Ends,
    alpn: Option<Vec<u8>>,
}

#[async_trait]
impl Connector for FakeConnector {
    async fn connect(
        &self,
        _scheme: &str,
        _host: &str,
        _port: u16,
    ) -> NetResult<Box<dyn IoStream>> {
        let (client, server) = duplex(64 * 1024);
        self.ends
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(server);
        Ok(Box::new(client))
    }

    async fn connect_negotiated(&self, scheme: &str, host: &str, port: u16) -> NetResult<Conn> {
        Ok(Conn {
            stream: self.connect(scheme, host, port).await?,
            alpn: self.alpn.clone(),
        })
    }
}

fn client(ends: Ends, alpn: Option<Vec<u8>>) -> AutoClient {
    AutoClient::new(
        Arc::new(FakeConnector { ends, alpn }),
        HttpConfig::default(),
    )
}

fn req(url: &str) -> Request {
    Request {
        method: Method::Get,
        url: Url::parse(url).expect("url"),
        headers: vec![],
        body: None,
        initiator: None,
        fetch_dest: FetchDest::Empty,
        fetch_mode: FetchMode::NoCors,
        fetch_user: false,
        cors: None,
        no_referer: false,
    }
}

/// Défile un flot serveur et lance la tâche `h2::server` dessus
/// — sert `body` avec `status` sur chaque stream reçu.
fn serve_h2(ends: &Ends, status: u16, body: &'static str) {
    let server = ends
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .pop()
        .expect("connexion serveur");
    tokio::spawn(async move {
        let mut conn = h2::server::handshake(server).await.expect("handshake");
        while let Some(accepted) = conn.accept().await {
            let (request, mut respond) = accepted.expect("stream");
            let _method = request.method().clone();
            tokio::spawn(async move {
                let resp = http::Response::builder()
                    .status(status)
                    .header("x-test", "yes")
                    .body(())
                    .expect("resp");
                let mut send = respond.send_response(resp, false).expect("send");
                send.send_data(bytes::Bytes::from_static(body.as_bytes()), true)
                    .expect("data");
            });
        }
    });
}

#[tokio::test]
async fn alpn_h2_serves_body_and_headers() {
    let ends: Ends = Arc::new(Mutex::new(Vec::new()));
    let c = client(ends.clone(), Some(b"h2".to_vec()));
    let r = req("https://h.test/");
    // Le connect doit avoir eu lieu avant qu'on serve le flot —
    // `fetch` ouvre puis handshake : on lance le serveur en tâche
    // concurrente, il attendra la paire duplex.
    let e2 = ends.clone();
    let server = tokio::spawn(async move {
        for _ in 0..200 {
            if !e2.lock().unwrap_or_else(PoisonError::into_inner).is_empty() {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        panic!("pas de connexion");
    });
    let fetch = tokio::spawn(async move { c.fetch(&r).await });
    // Attendre la connexion puis servir.
    server.await.expect("wait");
    serve_h2(&ends, 200, "bonjour h2");
    let resp = fetch.await.expect("join").expect("fetch");
    assert_eq!(resp.status, 200);
    assert_eq!(String::from_utf8_lossy(&resp.body), "bonjour h2");
    assert_eq!(
        resp.headers
            .iter()
            .find(|(n, _)| n == "x-test")
            .map(|x| &x.1),
        Some(&"yes".to_string())
    );
    assert!(resp.reused_connection);
}

#[tokio::test]
async fn alpn_h2_post_body_roundtrip() {
    let ends: Ends = Arc::new(Mutex::new(Vec::new()));
    let c = client(ends.clone(), Some(b"h2".to_vec()));
    let mut r = req("https://h.test/post");
    r.method = Method::Post;
    r.body = Some(b"a=1&b=2".to_vec());

    let e2 = ends.clone();
    let server = tokio::spawn(async move {
        for _ in 0..200 {
            if !e2.lock().unwrap_or_else(PoisonError::into_inner).is_empty() {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        panic!("pas de connexion");
    });
    let fetch = tokio::spawn(async move { c.fetch(&r).await });
    server.await.expect("wait");
    // Serveur écho : renvoie le corps reçu.
    let s = ends
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .pop()
        .expect("flot");
    tokio::spawn(async move {
        let mut conn = h2::server::handshake(s).await.expect("handshake");
        while let Some(accepted) = conn.accept().await {
            let (request, mut respond) = accepted.expect("stream");
            tokio::spawn(async move {
                let mut body = Vec::new();
                let mut rs = request.into_parts().1;
                while let Some(chunk) = rs.data().await {
                    let c = chunk.expect("chunk");
                    body.extend_from_slice(&c);
                    let _ = rs.flow_control().release_capacity(c.len());
                }
                let resp = http::Response::builder()
                    .status(200)
                    .body(())
                    .expect("resp");
                let mut send = respond.send_response(resp, false).expect("send");
                send.send_data(bytes::Bytes::from(body), true)
                    .expect("data");
            });
        }
    });
    let resp = fetch.await.expect("join").expect("fetch");
    assert_eq!(resp.status, 200);
    assert_eq!(String::from_utf8_lossy(&resp.body), "a=1&b=2");
}

#[tokio::test]
async fn alpn_absent_falls_back_to_h1_on_same_conn() {
    let ends: Ends = Arc::new(Mutex::new(Vec::new()));
    let c = client(ends.clone(), None);
    let r = req("https://h.test/");

    let e2 = ends.clone();
    let server = tokio::spawn(async move {
        for _ in 0..200 {
            if !e2.lock().unwrap_or_else(PoisonError::into_inner).is_empty() {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        panic!("pas de connexion");
    });
    let fetch = tokio::spawn(async move { c.fetch(&r).await });
    server.await.expect("wait");
    let mut s = ends
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .pop()
        .expect("flot");
    tokio::spawn(async move {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut buf = vec![0u8; 4096];
        let n = s.read(&mut buf).await.expect("read");
        let head = String::from_utf8_lossy(&buf[..n]);
        assert!(head.starts_with("GET / HTTP/1.1"), "reçu : {head}");
        let page = "<html>repli h1</html>";
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/html\r\n\r\n{page}",
            page.len()
        );
        s.write_all(resp.as_bytes()).await.expect("write");
    });
    let resp = fetch.await.expect("join").expect("fetch");
    assert_eq!(resp.status, 200);
    assert_eq!(String::from_utf8_lossy(&resp.body), "<html>repli h1</html>");
    // Pas de multiplexage → la connexion repart au pool h1.
    assert!(!resp.reused_connection);
}

#[tokio::test]
async fn http_scheme_stays_h1() {
    let ends: Ends = Arc::new(Mutex::new(Vec::new()));
    let c = client(ends.clone(), Some(b"h2".to_vec()));
    let r = req("http://h.test/");

    let e2 = ends.clone();
    let server = tokio::spawn(async move {
        for _ in 0..200 {
            if !e2.lock().unwrap_or_else(PoisonError::into_inner).is_empty() {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        panic!("pas de connexion");
    });
    let fetch = tokio::spawn(async move { c.fetch(&r).await });
    server.await.expect("wait");
    let mut s = ends
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .pop()
        .expect("flot");
    tokio::spawn(async move {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut buf = vec![0u8; 4096];
        let n = s.read(&mut buf).await.expect("read");
        let head = String::from_utf8_lossy(&buf[..n]);
        assert!(head.starts_with("GET / HTTP/1.1"), "reçu : {head}");
        let page = "clair";
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{page}",
            page.len()
        );
        s.write_all(resp.as_bytes()).await.expect("write");
    });
    let resp = fetch.await.expect("join").expect("fetch");
    assert_eq!(String::from_utf8_lossy(&resp.body), "clair");
}
