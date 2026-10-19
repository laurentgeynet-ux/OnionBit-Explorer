// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Tests d'intégration du client HTTP/1.1 — serveur loopback
//! `TcpListener` scripté (in-process, pas d'hôte externe en CI).

use std::sync::Arc;
use std::time::Duration;

use explorer_net::error::NetError;
use explorer_net::h1::{H1Client, HttpConfig, TcpConnector};
use explorer_net::http::{HttpTransport, Method, Request};
use explorer_net::url::Url;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::mpsc;

/// Serveur scripté : lit une requête jusqu'à `\r\n\r\n`, répond
/// `response`, puis (selon `keep`) garde la connexion pour la
/// requête suivante ou la ferme. Renvoie l'adresse liée.
struct Server {
    port: u16,
    /// Requêtes reçues (têtes), pour assertions.
    rx: mpsc::UnboundedReceiver<String>,
}

async fn serve(responses: Vec<&'static str>, keep: bool) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (tx, rx) = mpsc::unbounded_channel();
    let responses = Arc::new(responses);
    tokio::spawn(async move {
        let mut i = 0;
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                return;
            };
            let tx = tx.clone();
            let responses = responses.clone();
            tokio::spawn(async move {
                loop {
                    // Lit jusqu'à la fin des en-têtes + éventuel
                    // corps Content-Length.
                    let mut buf = Vec::new();
                    let mut chunk = [0u8; 4096];
                    let head_end = loop {
                        let Ok(n) = sock.read(&mut chunk).await else {
                            return;
                        };
                        if n == 0 {
                            return;
                        }
                        buf.extend_from_slice(&chunk[..n]);
                        if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                            break p + 4;
                        }
                    };
                    let head = String::from_utf8_lossy(&buf[..head_end]).into_owned();
                    let _ = tx.send(head.clone());
                    // Consomme un éventuel corps.
                    let cl = head
                        .lines()
                        .find(|l| l.to_ascii_lowercase().starts_with("content-length:"))
                        .and_then(|l| l.split(':').nth(1)?.trim().parse::<usize>().ok())
                        .unwrap_or(0);
                    while buf.len() < head_end + cl {
                        let Ok(n) = sock.read(&mut chunk).await else {
                            return;
                        };
                        if n == 0 {
                            return;
                        }
                        buf.extend_from_slice(&chunk[..n]);
                    }
                    if i < responses.len() {
                        let _ = sock.write_all(responses[i].as_bytes()).await;
                        i += 1;
                    }
                    if !keep || head.to_ascii_lowercase().contains("connection: close") {
                        return;
                    }
                }
            });
        }
    });
    Server { port, rx }
}

fn url(port: u16, path: &str) -> Url {
    Url::parse(&format!("http://127.0.0.1:{port}{path}")).unwrap()
}

fn client() -> H1Client {
    H1Client::new(Arc::new(TcpConnector), HttpConfig::default())
}

#[tokio::test]
async fn simple_get() {
    let mut srv = serve(
        vec!["HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello"],
        false,
    )
    .await;
    let resp = client()
        .fetch(&Request::get(url(srv.port, "/p")))
        .await
        .unwrap();
    assert_eq!(resp.status, 200);
    assert_eq!(resp.reason, "OK");
    assert_eq!(resp.body, b"hello");
    assert_eq!(resp.header("content-length"), Some("5"));
    // La requête émise est bien formée.
    let sent = srv.rx.recv().await.unwrap();
    assert!(sent.starts_with("GET /p HTTP/1.1\r\n"), "{sent}");
    assert!(sent.contains("Host: 127.0.0.1:"), "{sent}");
}

#[tokio::test]
async fn post_with_body() {
    let mut srv = serve(
        vec!["HTTP/1.1 201 Created\r\nContent-Length: 2\r\n\r\nok"],
        false,
    )
    .await;
    let mut req = Request::get(url(srv.port, "/submit"));
    req.method = Method::Post;
    req.body = Some(b"data=1".to_vec());
    let resp = client().fetch(&req).await.unwrap();
    assert_eq!(resp.status, 201);
    let sent = srv.rx.recv().await.unwrap();
    assert!(sent.contains("Content-Length: 6"), "{sent}");
    assert!(sent.starts_with("POST "), "{sent}");
}

#[tokio::test]
async fn chunked_body() {
    let srv = serve(
        vec![
            "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n\
              4\r\nwiki\r\n5\r\npedia\r\n0\r\n\r\n",
        ],
        false,
    )
    .await;
    let resp = client()
        .fetch(&Request::get(url(srv.port, "/")))
        .await
        .unwrap();
    assert_eq!(resp.body, b"wikipedia");
    let _ = srv;
}

#[tokio::test]
async fn chunked_with_trailers() {
    let srv = serve(
        vec![
            "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n\
              3\r\nabc\r\n0\r\nX-T: 1\r\n\r\n",
        ],
        false,
    )
    .await;
    let resp = client()
        .fetch(&Request::get(url(srv.port, "/")))
        .await
        .unwrap();
    assert_eq!(resp.body, b"abc");
    let _ = srv;
}

#[tokio::test]
async fn head_has_no_body() {
    let srv = serve(vec!["HTTP/1.1 200 OK\r\nContent-Length: 99\r\n\r\n"], false).await;
    let mut req = Request::get(url(srv.port, "/"));
    req.method = Method::Head;
    let resp = client().fetch(&req).await.unwrap();
    assert_eq!(resp.body.len(), 0);
    let _ = srv;
}

#[tokio::test]
async fn status_204_no_body() {
    let srv = serve(vec!["HTTP/1.1 204 No Content\r\n\r\n"], false).await;
    let resp = client()
        .fetch(&Request::get(url(srv.port, "/")))
        .await
        .unwrap();
    assert_eq!(resp.status, 204);
    assert_eq!(resp.body, b"");
    let _ = srv;
}

#[tokio::test]
async fn eof_delimited_body() {
    // Pas de Content-Length ni chunked → corps = jusqu'à EOF.
    let srv = serve(vec!["HTTP/1.1 200 OK\r\n\r\nbody-until-close"], false).await;
    let resp = client()
        .fetch(&Request::get(url(srv.port, "/")))
        .await
        .unwrap();
    assert_eq!(resp.body, b"body-until-close");
    let _ = srv;
}

#[tokio::test]
async fn keep_alive_reuse() {
    // Deux réponses keep-alive → une seule connexion.
    let srv = serve(
        vec![
            "HTTP/1.1 200 OK\r\nContent-Length: 1\r\n\r\na",
            "HTTP/1.1 200 OK\r\nContent-Length: 1\r\n\r\nb",
        ],
        true,
    )
    .await;
    let c = client();
    let u = url(srv.port, "/");
    let r1 = c.fetch(&Request::get(u.clone())).await.unwrap();
    let r2 = c.fetch(&Request::get(u)).await.unwrap();
    assert_eq!(r1.body, b"a");
    assert_eq!(r2.body, b"b");
    assert!(!r1.reused_connection);
    assert!(r2.reused_connection);
    let _ = srv;
}

#[tokio::test]
async fn connection_close_not_reused() {
    let srv = serve(
        vec!["HTTP/1.1 200 OK\r\nContent-Length: 1\r\nConnection: close\r\n\r\nx"],
        true,
    )
    .await;
    let resp = client()
        .fetch(&Request::get(url(srv.port, "/")))
        .await
        .unwrap();
    assert_eq!(resp.body, b"x");
    let _ = srv;
}

#[tokio::test]
async fn truncated_body_errors() {
    let srv = serve(
        vec!["HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\nabc"], // 3/10 puis close
        false,
    )
    .await;
    let err = client()
        .fetch(&Request::get(url(srv.port, "/")))
        .await
        .unwrap_err();
    assert!(matches!(err, NetError::TruncatedBody { .. }), "{err}");
    let _ = srv;
}

#[tokio::test]
async fn head_too_large() {
    let srv = serve(vec!["HTTP/1.1 200 OK\r\n"], false).await;
    let cfg = HttpConfig {
        max_head_bytes: 16,
        ..HttpConfig::default()
    };
    let c = H1Client::new(Arc::new(TcpConnector), cfg);
    let err = c
        .fetch(&Request::get(url(srv.port, "/")))
        .await
        .unwrap_err();
    // Le serveur envoie plus d'en-têtes que le budget → HeadTooLarge
    // ou connexion fermée — les deux sont des erreurs typées.
    assert!(
        matches!(
            err,
            NetError::HeadTooLarge(_) | NetError::InvalidResponse(_)
        ),
        "{err}"
    );
    let _ = srv;
}

#[tokio::test]
async fn io_timeout() {
    // Serveur qui accepte mais ne répond jamais.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        loop {
            // Lit la requête mais ne répond jamais — le socket reste
            // ouvert tant que la tâche vit.
            if let Ok((mut sock, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let mut sink = [0u8; 4096];
                    loop {
                        match sock.read(&mut sink).await {
                            Ok(0) | Err(_) => return,
                            Ok(_) => {}
                        }
                    }
                });
            }
        }
    });
    let cfg = HttpConfig {
        io_timeout: Duration::from_millis(100),
        ..HttpConfig::default()
    };
    let c = H1Client::new(Arc::new(TcpConnector), cfg);
    let err = c.fetch(&Request::get(url(port, "/"))).await.unwrap_err();
    assert!(matches!(err, NetError::Timeout { .. }), "{err}");
}

#[tokio::test]
async fn unsupported_scheme() {
    let req = Request::get(Url::parse("data:text/plain,x").unwrap());
    let err = client().fetch(&req).await.unwrap_err();
    assert!(matches!(err, NetError::UnsupportedScheme(_)), "{err}");
}

#[tokio::test]
async fn status_and_headers_preserved() {
    let srv = serve(
        vec!["HTTP/1.1 404 Not Found\r\nX-A: 1\r\nX-A: 2\r\nContent-Length: 0\r\n\r\n"],
        false,
    )
    .await;
    let resp = client()
        .fetch(&Request::get(url(srv.port, "/missing")))
        .await
        .unwrap();
    assert_eq!(resp.status, 404);
    assert_eq!(resp.reason, "Not Found");
    // Doublons préservés.
    let vals: Vec<&str> = resp.headers_all("x-a").collect();
    assert_eq!(vals, ["1", "2"]);
    let _ = srv;
}

/// Serveur lent : écrit la réponse en morceaux espacés — force
/// plusieurs lectures TCP côté client (streaming observable,
/// étape 53). Chaque élément de `pieces` est un `write_all`
/// séparé par `gap`.
async fn serve_slow(pieces: Vec<Vec<u8>>, gap: Duration) -> Server {
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
        for p in pieces {
            if sock.write_all(&p).await.is_err() {
                return;
            }
            tokio::time::sleep(gap).await;
        }
    });
    Server { port, rx }
}

/// Collecte le canal corps d'une `StreamedResponse`.
async fn collect(
    mut rx: tokio::sync::mpsc::Receiver<explorer_net::error::NetResult<Vec<u8>>>,
) -> (Vec<Vec<u8>>, Vec<u8>) {
    let mut pieces = Vec::new();
    let mut all = Vec::new();
    while let Some(item) = rx.recv().await {
        let piece = item.unwrap();
        all.extend_from_slice(&piece);
        pieces.push(piece);
    }
    (pieces, all)
}

#[tokio::test]
async fn stream_cl_body_arrives_in_pieces() {
    let srv = serve_slow(
        vec![
            b"HTTP/1.1 200 OK\r\nContent-Length: 34\r\n\r\npremier ".to_vec(),
            b"deuxieme ".to_vec(),
            b"troisieme morceau".to_vec(),
        ],
        Duration::from_millis(25),
    )
    .await;
    let resp = client()
        .fetch_stream(&Request::get(url(srv.port, "/")))
        .await
        .unwrap();
    assert_eq!(resp.status, 200);
    let (pieces, all) = collect(resp.body).await;
    assert_eq!(all, b"premier deuxieme troisieme morceau");
    // Le corps a réellement été streamé — pas assemblé en amont.
    assert!(pieces.len() >= 2, "pieces={:?}", pieces.len());
    let _ = srv;
}

#[tokio::test]
async fn stream_chunked_body_sends_per_chunk() {
    let srv = serve_slow(
        vec![
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4\r\nwiki\r\n".to_vec(),
            b"5\r\npedia\r\n".to_vec(),
            b"0\r\n\r\n".to_vec(),
        ],
        Duration::from_millis(25),
    )
    .await;
    let resp = client()
        .fetch_stream(&Request::get(url(srv.port, "/")))
        .await
        .unwrap();
    let (pieces, all) = collect(resp.body).await;
    assert_eq!(all, b"wikipedia");
    assert_eq!(pieces.len(), 2, "un envoi par chunk de données");
    let _ = srv;
}

#[tokio::test]
async fn stream_reusable_conn_returns_to_pool() {
    let srv = serve(
        vec![
            "HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello",
            "HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nworld",
        ],
        true,
    )
    .await;
    let c = client();
    let resp = c
        .fetch_stream(&Request::get(url(srv.port, "/a")))
        .await
        .unwrap();
    let (_pieces, all) = collect(resp.body).await;
    assert_eq!(all, b"hello");
    // La connexion a rejoint le pool à la fin du corps — la
    // requête suivante la réutilise.
    let resp2 = c.fetch(&Request::get(url(srv.port, "/b"))).await.unwrap();
    assert_eq!(resp2.body, b"world");
    assert!(resp2.reused_connection);
    let _ = srv;
}
