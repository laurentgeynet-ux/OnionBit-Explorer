// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Tests TLS — handshake réel loopback, cert auto-signé `rcgen`.

use std::sync::Arc;

use explorer_net::error::NetError;
use explorer_net::h1::{H1Client, HttpConfig};
use explorer_net::http::{HttpTransport, Request};
use explorer_net::tls::TlsConnector;
use explorer_net::url::Url;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::{RootCertStore, ServerConfig};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;

/// Cert + store de confiance pour `localhost`.
fn cert_and_store() -> (ServerConfig, RootCertStore) {
    let rcgen::CertifiedKey { cert, key_pair } =
        rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
    let cert_der = CertificateDer::from(cert.der().to_vec());
    let key_der = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key_pair.serialize_der()));

    let mut store = RootCertStore::empty();
    store.add(cert_der.clone()).unwrap();

    let server = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert_der], key_der)
        .unwrap();
    (server, store)
}

/// Serveur TLS : handshake rustls, lit la requête, répond `resp`.
async fn serve_tls(resp: &'static str) -> (u16, RootCertStore) {
    let (server, store) = cert_and_store();
    let acceptor = TlsAcceptor::from(Arc::new(server));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        loop {
            let Ok((sock, _)) = listener.accept().await else {
                return;
            };
            let acceptor = acceptor.clone();
            tokio::spawn(async move {
                let Ok(mut tls) = acceptor.accept(sock).await else {
                    return;
                };
                let mut buf = [0u8; 4096];
                loop {
                    let Ok(n) = tls.read(&mut buf).await else {
                        return;
                    };
                    if n == 0 || buf[..n].windows(4).any(|w| w == b"\r\n\r\n") {
                        break;
                    }
                }
                let _ = tls.write_all(resp.as_bytes()).await;
            });
        }
    });
    (port, store)
}

#[tokio::test]
async fn https_fetch_trusted_cert() {
    let (port, store) =
        serve_tls("HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\nTLS!").await;
    let client = H1Client::new(
        Arc::new(TlsConnector::with_roots(store)),
        HttpConfig::default(),
    );
    let url = Url::parse(&format!("https://localhost:{port}/secure")).unwrap();
    let resp = client.fetch(&Request::get(url)).await.unwrap();
    assert_eq!(resp.status, 200);
    assert_eq!(resp.body, b"TLS!");
}

#[tokio::test]
async fn https_untrusted_cert_is_tls_error() {
    // Même cert, mais client avec les racines Mozilla (qui ne le
    // connaissent pas) → erreur TLS typée, pas un crash.
    let (port, _store) = serve_tls("HTTP/1.1 200 OK\r\nContent-Length: 1\r\n\r\nx").await;
    let client = H1Client::new(Arc::new(TlsConnector::web_roots()), HttpConfig::default());
    let url = Url::parse(&format!("https://localhost:{port}/")).unwrap();
    let err = client.fetch(&Request::get(url)).await.unwrap_err();
    assert!(matches!(err, NetError::Tls { .. }), "{err}");
}

#[tokio::test]
async fn https_wrong_hostname_is_tls_error() {
    // Cert pour `localhost`, connexion à `127.0.0.1` → SNI/SAN ne
    // correspond pas → erreur TLS typée.
    let (port, store) = serve_tls("HTTP/1.1 200 OK\r\nContent-Length: 1\r\n\r\nx").await;
    let client = H1Client::new(
        Arc::new(TlsConnector::with_roots(store)),
        HttpConfig::default(),
    );
    let url = Url::parse(&format!("https://127.0.0.1:{port}/")).unwrap();
    let err = client.fetch(&Request::get(url)).await.unwrap_err();
    assert!(matches!(err, NetError::Tls { .. }), "{err}");
}
