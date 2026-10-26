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
    relay("/movie/user/user/1001.mp4", "http://127.0.0.1:8000").await;
}

/// Vol réseau réel — cible fournie par `MEDIA_PROXY_FIELD_URL`
/// (aucune URL ni identifiant dans le dépôt : serveur privé).
/// Même pile que le daemon (`RedirectTransport` — les panels
/// IPTV redirigent vers le nœud de flux).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "terrain : exige MEDIA_PROXY_FIELD_URL=<url flux>"]
async fn proxy_relaye_flux_env() {
    let url = std::env::var("MEDIA_PROXY_FIELD_URL")
        .expect("MEDIA_PROXY_FIELD_URL requis");
    relay_full(&url).await;
}

async fn relay(path: &str, base: &str) {
    relay_full(&format!("{base}{path}")).await;
}

async fn relay_full(target: &str) {
    let inner: Arc<dyn HttpTransport> = Arc::new(AutoClient::new(
        Arc::new(TlsConnector::web_roots()),
        HttpConfig::default(),
    ));
    // Même pile que le daemon pour les flux : redirections
    // suivies (302 → nœud token), bornes par défaut.
    let transport: Arc<dyn HttpTransport> =
        Arc::new(explorer_net::policy::RedirectTransport::wrap(inner, 5));
    // UA injecté : les panels IPTV filtrent souvent (VLC…) —
    // `MEDIA_PROXY_FIELD_UA` permet de le choisir en vol.
    let ua = std::env::var("MEDIA_PROXY_FIELD_UA")
        .unwrap_or_else(|_| "ExplorerTest".into());
    let proxy = MediaProxy::spawn(transport, ua)
        .await
        .expect("bind loopback");

    let full = proxy.url(target);
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
    // Live `.ts` : le serveur peut ignorer `Range` → flux
    // infini ; on borne à 20 s et on garde le partiel.
    let _ = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        s.read_to_end(&mut buf),
    )
    .await;
    let txt = String::from_utf8_lossy(&buf);
    // FileResponse honore Range → 206 + content-range.
    assert!(
        txt.contains("206") || txt.contains("200"),
        "réponse proxy : {txt}"
    );
    // Corps présent : MP4 (`ftyp`) ou MPEG-TS (sync `0x47`
    // cadencée 188 o) selon le flux servi.
    assert!(buf.len() > 100, "corps vide : {} octets", buf.len());
    let mp4 = buf.windows(4).any(|w| w == b"ftyp");
    let ts = buf.iter().filter(|&&b| b == 0x47).count() > 50;
    assert!(mp4 || ts, "ni signature mp4 ni trames ts relais");
}
