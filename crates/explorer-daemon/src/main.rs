// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! `explorer-daemon` — processus hôte : moteur + API REST/SSE sur
//! `127.0.0.1` (ADR-0003). L'app Flutter le supervise (spawn, drain
//! stderr, kill propre).
//!
//! Étape 24 : assemble la pile transport complète (`TLS → H1 →
//! décodage → redirections`), les fontes du répertoire système
//! ([`DirFontSource`]), la [`Session`] moteur et le routeur
//! `explorer-api`. **Loopback strict** — aucun bind externe par
//! défaut (ADR-0003).
//!
//! Configuration (règle 13) : tout par variables
//! d'environnement — `EXPLORER_PORT`, `EXPLORER_FONT_DIR`,
//! `EXPLORER_MAX_BODY`.

#![forbid(unsafe_code)]

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use explorer_core::Session;
use explorer_net::decode::DecodingTransport;
use explorer_net::h1::{H1Client, HttpConfig};
use explorer_net::http::HttpTransport;
use explorer_net::policy::RedirectTransport;
use explorer_net::tls::TlsConnector;
use explorer_render::DirFontSource;
use tracing_subscriber::EnvFilter;

/// Configuration du daemon — seuils en variables
/// d'environnement, jamais de constantes magiques noyées.
struct DaemonConfig {
    /// Adresse d'écoute — **loopback uniquement**.
    bind: SocketAddr,
    /// Répertoire de fontes (`None` = aucune fonte → repli
    /// mesures du moteur, dégradation sans crash).
    fonts_dir: PathBuf,
    /// Cap de décodage du corps (compression hostile).
    max_body: usize,
    /// Redirections max suivies.
    max_redirects: usize,
}

impl DaemonConfig {
    /// Lit l'environnement avec les valeurs par défaut du
    /// contrat (port 9377, cap 16 Mio, 8 redirections).
    fn from_env() -> Self {
        let port = std::env::var("EXPLORER_PORT")
            .ok()
            .and_then(|p| p.parse::<u16>().ok())
            .unwrap_or(9377);
        let fonts_dir =
            std::env::var_os("EXPLORER_FONT_DIR").map_or_else(default_fonts_dir, PathBuf::from);
        let max_body = std::env::var("EXPLORER_MAX_BODY")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(16 * 1024 * 1024);
        Self {
            bind: SocketAddr::from(([127, 0, 0, 1], port)),
            fonts_dir,
            max_body,
            max_redirects: 8,
        }
    }
}

/// Répertoire de fontes par défaut selon la plateforme.
fn default_fonts_dir() -> PathBuf {
    #[cfg(windows)]
    {
        PathBuf::from(r"C:\Windows\Fonts")
    }
    #[cfg(target_os = "macos")]
    {
        PathBuf::from("/System/Library/Fonts")
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        PathBuf::from("/usr/share/fonts")
    }
}

/// Pile transport du navigateur : TLS web-roots → HTTP/1.1 →
/// décodage `gzip`/`chunked` → redirections bornées — la même
/// composition que `explorer-cli`.
fn http_stack(cfg: &DaemonConfig) -> Arc<dyn HttpTransport> {
    let tls = Arc::new(TlsConnector::web_roots());
    let h1: Arc<dyn HttpTransport> = Arc::new(H1Client::new(tls, HttpConfig::default()));
    let decoded: Arc<dyn HttpTransport> = Arc::new(DecodingTransport::wrap(h1, cfg.max_body));
    Arc::new(RedirectTransport::wrap(decoded, cfg.max_redirects))
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();
    let cfg = DaemonConfig::from_env();
    let fonts = DirFontSource::new(&cfg.fonts_dir);
    tracing::info!(dir = %cfg.fonts_dir.display(), "fontes système scannées");

    let session = Arc::new(tokio::sync::Mutex::new(Session::new(
        http_stack(&cfg),
        Arc::new(fonts),
    )));
    let app = explorer_api::router(session);

    let listener = match tokio::net::TcpListener::bind(cfg.bind).await {
        Ok(l) => l,
        Err(e) => {
            tracing::error!(addr = %cfg.bind, error = %e, "bind impossible");
            std::process::exit(1);
        }
    };
    tracing::info!(addr = %cfg.bind, "explorer-daemon prêt");
    if let Err(e) = axum::serve(listener, app).await {
        tracing::error!(error = %e, "serveur arrêté");
        std::process::exit(1);
    }
}
