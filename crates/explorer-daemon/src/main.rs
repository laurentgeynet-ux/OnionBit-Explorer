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

mod store;

use explorer_core::Session;
use explorer_net::cookies::{CookieJar, CookieTransport};
use explorer_net::decode::DecodingTransport;
use explorer_net::h1::HttpConfig;
use explorer_net::h2::AutoClient;
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
    /// Clé d'API optionnelle (`EXPLORER_API_KEY` — ADR-0003).
    api_key: Option<String>,
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
            api_key: std::env::var("EXPLORER_API_KEY")
                .ok()
                .filter(|k| !k.is_empty()),
        }
    }
}

/// Période de la pompe réseau script-initiée (étape 51 —
/// règle 13 : borne de config, pas de magie).
const NET_PUMP_MS: std::time::Duration = std::time::Duration::from_millis(50);

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
/// décodage `gzip`/`chunked` → jar à cookies → redirections
/// bornées — la même composition que `explorer-cli`. Le
/// `CookieJar` partagé avec la session (étape 51) est
/// **sous** la couche redirection : chaque hop re-reçoit
/// les cookies de sa propre URL.
fn http_stack(cfg: &DaemonConfig, jar: Arc<CookieJar>) -> Arc<dyn HttpTransport> {
    let tls = Arc::new(TlsConnector::web_roots());
    // ALPN : h2 si le serveur le propose, h1 sinon (etape 41).
    let h1: Arc<dyn HttpTransport> = Arc::new(AutoClient::new(tls, HttpConfig::default()));
    let decoded: Arc<dyn HttpTransport> = Arc::new(DecodingTransport::wrap(h1, cfg.max_body));
    let with_cookies: Arc<dyn HttpTransport> = Arc::new(CookieTransport::wrap(decoded, jar));
    Arc::new(RedirectTransport::wrap(with_cookies, cfg.max_redirects))
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();
    let cfg = DaemonConfig::from_env();
    let fonts = DirFontSource::new(&cfg.fonts_dir);
    tracing::info!(dir = %cfg.fonts_dir.display(), "fontes système scannées");

    let store = Arc::new(store::JsonFileStore::open(
        &store::JsonFileStore::default_dir(),
    ));
    tracing::info!(dir = %store::JsonFileStore::default_dir().display(), "profil chargé");
    // Jar partagé transport ↔ session (étape 51) :
    // `document.cookie`, les XHR same-origin et les
    // navigations voient le même état.
    let jar = Arc::new(CookieJar::new());
    let mut session = Session::new(http_stack(&cfg, jar.clone()), Arc::new(fonts));
    session.set_cookie_jar(jar);
    session.set_store(store);
    session.set_js_factory(explorer_js::DilloEngine::factory());
    let session = Arc::new(tokio::sync::Mutex::new(session));

    // Battement des réponses réseau script-initiées (étape
    // 51) : la session n'a pas d'horloge — un XHR asynchrone
    // est livré à la prochaine pompe, bornée par
    // `NET_PUMP_MS`.
    {
        let s = session.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(NET_PUMP_MS);
            loop {
                tick.tick().await;
                s.lock().await.pump_net().await;
            }
        });
    }
    let app = explorer_api::router_with_key(session, cfg.api_key.clone());
    if cfg.api_key.is_some() {
        tracing::info!("clé d'API requise (x-api-key / ?key=)");
    }

    let listener = match tokio::net::TcpListener::bind(cfg.bind).await {
        Ok(l) => l,
        Err(e) => {
            tracing::error!(addr = %cfg.bind, error = %e, "bind impossible");
            std::process::exit(1);
        }
    };
    tracing::info!(addr = %cfg.bind, "explorer-daemon prêt");
    let r = axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await;
    if let Err(e) = r {
        tracing::error!(error = %e, "serveur arrêté");
        std::process::exit(1);
    }
    tracing::info!("arrêt propre");
}

/// Ctrl-C (SIGINT) ou SIGTERM unix → fin du `serve` sans
/// couper les requêtes en vol.
async fn shutdown_signal() {
    let ctrl_c = tokio::signal::ctrl_c();
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        if let Ok(mut term) = signal(SignalKind::terminate()) {
            tokio::select! {
                () = async { let _ = ctrl_c.await; } => {}
                () = async { let _ = term.recv().await; } => {}
            }
            return;
        }
    }
    let _ = ctrl_c.await;
}
