// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Test **terrain** du `LocalMediaProxy` (étape 139e) — sockets
//! réelles, ignoré par défaut : à lancer explicitement avec le
//! serveur IPTV de test (`D:\Projet\serveur iptv`, port 8000) :
//! `cargo test -p explorer-net --test media_proxy_field -- --ignored`.
//! Le transport est le vrai `AutoClient` (h1/h2 + TLS) — même
//! pile que le daemon — et le serveur sert `Bear (Demo).mp4`.

use std::sync::Arc;

use explorer_net::h1::HttpConfig;
use explorer_net::h2::AutoClient;
use explorer_net::http::HttpTransport;
use explorer_net::media_proxy::MediaProxy;
use explorer_net::tls::TlsConnector;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "terrain : exige le serveur IPTV local sur :8000"]
async fn proxy_relaye_flux_vod_reel() {
    let transport: Arc<dyn HttpTransport> = Arc::new(AutoClient::new(
        Arc::new(TlsConnector::web_roots()),
        HttpConfig::default(),
    ));
    let proxy = MediaProxy::spawn(transport, "ExplorerTest".into())
        .await
        .expect("bind loopback");

    let full = proxy.url("http://127.0.0.1:8000/movie/user/user/1001.mp4");
    let path = full
        .strip_prefix(&format!("http://{}", proxy.addr()))
        .expect("path");

    let mut s = tokio::net::TcpStream::connect(proxy.addr())
        .await
        .expect("connexion proxy");
    s.write_all(
        format!(
            "GET {path} HTTP/1.1\r\nHost: x\r\nRange: bytes=0-4095\r\nConnection: close\r\n\r\n"
        )
        .as_bytes(),
    )
    .await
    .expect("écriture");
    let mut buf = Vec::new();
    s.read_to_end(&mut buf).await.expect("lecture");
    let txt = String::from_utf8_lossy(&buf);
    // FileResponse honore Range → 206 + content-range.
    assert!(
        txt.contains("206") || txt.contains("200"),
        "réponse proxy : {txt}"
    );
    // Corps MP4 présent (signature `ftyp` visible dans le flux).
    assert!(buf.len() > 100, "corps vide : {} octets", buf.len());
    assert!(buf.windows(4).any(|w| w == b"ftyp"), "pas de signature mp4");
}
