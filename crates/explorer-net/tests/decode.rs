// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Tests du décodage des corps — `DecodingTransport` sur serveur
//! loopback scripté + assertions charset/nosniff sur `Response`.

use std::io::Write;
use std::sync::Arc;

use explorer_net::decode::{decode_response, DecodingTransport};
use explorer_net::error::NetError;
use explorer_net::h1::{H1Client, HttpConfig, TcpConnector};
use explorer_net::http::{HttpTransport, Request, Response};
use explorer_net::url::Url;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

fn client() -> DecodingTransport {
    DecodingTransport::wrap(
        Arc::new(H1Client::new(Arc::new(TcpConnector), HttpConfig::default())),
        1 << 20,
    )
}

fn url(port: u16, path: &str) -> Url {
    Url::parse(&format!("http://127.0.0.1:{port}{path}")).unwrap()
}

/// Serveur une-réponse : lit la requête, renvoie `resp`.
async fn serve_bytes(resp: Vec<u8>) -> (u16, tokio::sync::mpsc::UnboundedReceiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
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
        let _ = sock.write_all(&resp).await;
    });
    (port, rx)
}

fn gzip(body: &[u8]) -> Vec<u8> {
    let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    e.write_all(body).unwrap();
    e.finish().unwrap()
}

fn zlib(body: &[u8]) -> Vec<u8> {
    let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    e.write_all(body).unwrap();
    e.finish().unwrap()
}

fn raw_deflate(body: &[u8]) -> Vec<u8> {
    let mut e = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
    e.write_all(body).unwrap();
    e.finish().unwrap()
}

fn brotli(body: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut w = brotli::CompressorWriter::new(&mut out, 4096, 5, 22);
        w.write_all(body).unwrap();
    }
    out
}

fn framed(headers: &str, body: &[u8]) -> Vec<u8> {
    let mut v = format!("{headers}Content-Length: {}\r\n\r\n", body.len()).into_bytes();
    v.extend_from_slice(body);
    v
}

#[tokio::test]
async fn gzip_decoded() {
    let body = gzip(b"<html>gzipped</html>");
    let (port, mut rx) = serve_bytes(framed(
        "HTTP/1.1 200 OK\r\nContent-Encoding: gzip\r\n",
        &body,
    ))
    .await;
    let resp = client().fetch(&Request::get(url(port, "/"))).await.unwrap();
    assert_eq!(resp.body, b"<html>gzipped</html>");
    // Content-Length ajusté à la taille décodée.
    assert_eq!(
        resp.header("content-length"),
        Some(resp.body.len().to_string().as_str())
    );
    // Accept-Encoding annoncé sur le fil.
    let sent = rx.recv().await.unwrap();
    assert!(sent.contains("Accept-Encoding"), "{sent}");
}

#[tokio::test]
async fn zlib_deflate_decoded() {
    let body = zlib(b"zlib body");
    let (port, _rx) = serve_bytes(framed(
        "HTTP/1.1 200 OK\r\nContent-Encoding: deflate\r\n",
        &body,
    ))
    .await;
    let resp = client().fetch(&Request::get(url(port, "/"))).await.unwrap();
    assert_eq!(resp.body, b"zlib body");
}

#[tokio::test]
async fn raw_deflate_fallback() {
    // Le web réel envoie souvent du raw deflate sous "deflate".
    let body = raw_deflate(b"raw deflate body");
    let (port, _rx) = serve_bytes(framed(
        "HTTP/1.1 200 OK\r\nContent-Encoding: deflate\r\n",
        &body,
    ))
    .await;
    let resp = client().fetch(&Request::get(url(port, "/"))).await.unwrap();
    assert_eq!(resp.body, b"raw deflate body");
}

#[tokio::test]
async fn brotli_decoded() {
    let body = brotli(b"brotli body");
    let (port, _rx) =
        serve_bytes(framed("HTTP/1.1 200 OK\r\nContent-Encoding: br\r\n", &body)).await;
    let resp = client().fetch(&Request::get(url(port, "/"))).await.unwrap();
    assert_eq!(resp.body, b"brotli body");
}

#[tokio::test]
async fn stacked_codings() {
    // "deflate, gzip" = deflate d'abord, gzip ensuite → décodage
    // en ordre inverse.
    let inner = zlib(b"stacked");
    let body = gzip(&inner);
    let (port, _rx) = serve_bytes(framed(
        "HTTP/1.1 200 OK\r\nContent-Encoding: deflate, gzip\r\n",
        &body,
    ))
    .await;
    let resp = client().fetch(&Request::get(url(port, "/"))).await.unwrap();
    assert_eq!(resp.body, b"stacked");
}

#[test]
fn corrupted_gzip_is_typed_error() {
    let mut resp = Response {
        status: 200,
        reason: "OK".into(),
        headers: vec![("Content-Encoding".into(), "gzip".into())],
        body: b"not gzip at all".to_vec(),
        reused_connection: false,
        resolved_url: None,
    };
    let err = decode_response(&mut resp, 1 << 20).unwrap_err();
    assert!(matches!(err, NetError::ContentDecode(_)), "{err}");
}

#[test]
fn unknown_coding_is_typed_error() {
    let mut resp = Response {
        status: 200,
        reason: "OK".into(),
        headers: vec![("Content-Encoding".into(), "zstd".into())],
        body: b"x".to_vec(),
        reused_connection: false,
        resolved_url: None,
    };
    let err = decode_response(&mut resp, 1 << 20).unwrap_err();
    assert!(matches!(err, NetError::ContentDecode(_)), "{err}");
}

#[test]
fn decode_budget_bombs() {
    // Petite entrée gzip → grande sortie : bornée par le budget.
    let body = gzip(&vec![b'a'; 10_000]);
    let mut resp = Response {
        status: 200,
        reason: "OK".into(),
        headers: vec![("Content-Encoding".into(), "gzip".into())],
        body,
        reused_connection: false,
        resolved_url: None,
    };
    let err = decode_response(&mut resp, 100).unwrap_err();
    assert!(matches!(err, NetError::DecodedTooLarge(_)), "{err}");
}

#[test]
fn charset_parsed() {
    let resp = Response {
        status: 200,
        reason: "OK".into(),
        headers: vec![(
            "Content-Type".into(),
            "text/html; charset=ISO-8859-1".into(),
        )],
        body: vec![],
        reused_connection: false,
        resolved_url: None,
    };
    assert_eq!(resp.charset().as_deref(), Some("iso-8859-1"));
    assert_eq!(resp.content_type(), Some("text/html; charset=ISO-8859-1"));
}

#[test]
fn nosniff_flag() {
    let sniff = Response {
        status: 200,
        reason: "OK".into(),
        headers: vec![("X-Content-Type-Options".into(), "nosniff".into())],
        body: vec![],
        reused_connection: false,
        resolved_url: None,
    };
    assert!(sniff.nosniff());
    let plain = Response {
        status: 200,
        reason: "OK".into(),
        headers: vec![],
        body: vec![],
        reused_connection: false,
        resolved_url: None,
    };
    assert!(!plain.nosniff());
}

#[test]
fn text_transcodes_windows1252() {
    // 0xE9 = 'é' en windows-1252 (latin1), 0x93/94 = guillemets.
    let resp = Response {
        status: 200,
        reason: "OK".into(),
        headers: vec![(
            "Content-Type".into(),
            "text/plain; charset=windows-1252".into(),
        )],
        body: vec![0x63, 0x61, 0x66, 0xE9], // "café"
        reused_connection: false,
        resolved_url: None,
    };
    assert_eq!(resp.text(), "café");
}

#[test]
fn text_utf8() {
    let resp = Response {
        status: 200,
        reason: "OK".into(),
        headers: vec![("Content-Type".into(), "text/plain; charset=utf-8".into())],
        body: "héllo".as_bytes().to_vec(),
        reused_connection: false,
        resolved_url: None,
    };
    assert_eq!(resp.text(), "héllo");
}

#[test]
fn text_defaults_to_1252() {
    // Pas de charset → windows-1252 (défaut WHATWG).
    let resp = Response {
        status: 200,
        reason: "OK".into(),
        headers: vec![],
        body: vec![0xE9],
        reused_connection: false,
        resolved_url: None,
    };
    assert_eq!(resp.text(), "é");
}
