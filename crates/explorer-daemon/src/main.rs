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
//! Configuration (règle 13) : variables d'environnement —
//! `EXPLORER_PORT`, `EXPLORER_FONT_DIR`, `EXPLORER_MAX_BODY`,
//! `EXPLORER_PROFILE` — surchargées par les flags
//! `--port`/`--profile` (étape 62 : les arguments inconnus
//! sont refusés, plus d'ignorance silencieuse).

#![forbid(unsafe_code)]

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use explorer_daemon::{cache, store};

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
    /// Timeout d'une requête `DoH` (`EXPLORER_DOH_TIMEOUT_MS`,
    /// étape 72 — règle 13).
    doh_timeout: std::time::Duration,
    /// Schéma de couleurs propagé à `StyleEnv` (étape 97 —
    /// `EXPLORER_COLOR_SCHEME` `dark`/`light`, défaut
    /// `light`).
    color_scheme: explorer_core::ColorScheme,
    /// Borne heap JS par contexte en Mio (`EXPLORER_JS_MEM_MIB`,
    /// étape 123) — `None` = défaut `SessionConfig` (512) ;
    /// `0` = illimité explicite. Outillage diagnostic de vol,
    /// pas une promesse produit.
    js_mem_mib: Option<usize>,
    /// Budget d'ops JS par évaluation (`EXPLORER_JS_OPS`,
    /// étape 124) — `None` = défaut `SessionConfig` (5 M) ;
    /// `0` = illimité explicite (mappé `u64::MAX` — un `while(1)`
    /// gèle alors le contexte : réglage diagnostic uniquement).
    js_ops: Option<u64>,
    /// Viewport initial `WxH` (`EXPLORER_VIEWPORT`, étape 129 —
    /// ex. `1366x900` ; `None` = défaut `SessionConfig`
    /// 1024×768). Même format que l'argument `LxH` de
    /// `explorer-cli shot`.
    viewport: Option<(f32, f32)>,
}

/// Parse `WxH` (`1366x900`) → `(w, h)` — vide/invalide → `None`
/// (réglage d'env : on dégrade au défaut, jamais de panne).
fn parse_viewport(v: &str) -> Option<(f32, f32)> {
    let mut it = v.split('x');
    match (it.next(), it.next()) {
        (Some(w), Some(h)) => match (w.parse::<f32>(), h.parse::<f32>()) {
            (Ok(w), Ok(h)) if w > 0.0 && h > 0.0 => Some((w, h)),
            _ => None,
        },
        _ => None,
    }
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
            doh_timeout: std::env::var("EXPLORER_DOH_TIMEOUT_MS")
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                .map_or(
                    std::time::Duration::from_secs(5),
                    std::time::Duration::from_millis,
                ),
            color_scheme: match std::env::var("EXPLORER_COLOR_SCHEME").as_deref() {
                Ok("dark") => explorer_core::ColorScheme::Dark,
                _ => explorer_core::ColorScheme::Light,
            },
            js_mem_mib: std::env::var("EXPLORER_JS_MEM_MIB")
                .ok()
                .and_then(|v| v.parse::<usize>().ok()),
            js_ops: std::env::var("EXPLORER_JS_OPS")
                .ok()
                .and_then(|v| v.parse::<u64>().ok()),
            viewport: std::env::var("EXPLORER_VIEWPORT")
                .ok()
                .and_then(|v| parse_viewport(&v)),
        }
    }
}

/// `SessionConfig` dérivée du `DaemonConfig` — schéma de
/// couleurs + surcharge de la borne heap JS
/// (`EXPLORER_JS_MEM_MIB`, étape 123 : outillage diagnostic
/// de vol, pas une promesse produit ; la valeur active est
/// tracée pour que le rapport de terrain cite la borne en
/// vigueur, jamais supposée).
fn session_config(cfg: &DaemonConfig) -> explorer_core::SessionConfig {
    let mut sc = explorer_core::SessionConfig {
        color_scheme: cfg.color_scheme,
        ..explorer_core::SessionConfig::default()
    };
    if let Some(mib) = cfg.js_mem_mib {
        tracing::info!("EXPLORER_JS_MEM_MIB={mib} — borne heap JS par contexte");
        sc.js_mem_limit_mib = mib;
    }
    if let Some(ops) = cfg.js_ops {
        // `0` = illimité explicite (étape 124) — `op_limit=0`
        // tuerait le premier `tick` ; `u64::MAX` en tient lieu.
        let v = if ops == 0 { u64::MAX } else { ops };
        tracing::info!("EXPLORER_JS_OPS={ops} — budget d'ops JS par évaluation");
        sc.js_ops = v;
    }
    if let Some((w, h)) = cfg.viewport {
        tracing::info!("EXPLORER_VIEWPORT={w}x{h} — viewport initial des onglets");
        sc.viewport = (w, h);
    }
    sc
}

/// Période de la pompe réseau script-initiée (étape 51 —
/// règle 13 : borne de config, pas de magie).
const NET_PUMP_MS: std::time::Duration = std::time::Duration::from_millis(50);

/// Arguments CLI (étape 62) — surcharges des `EXPLORER_*`.
struct CliArgs {
    /// `--port N` → surcharge `EXPLORER_PORT`.
    port: Option<u16>,
    /// `--profile DIR` → surcharge `EXPLORER_PROFILE`.
    profile: Option<PathBuf>,
    /// `--download-dir DIR` → surcharge `EXPLORER_DOWNLOAD_DIR`
    /// (étape 70 ; défaut `<profil>/downloads`).
    download_dir: Option<PathBuf>,
    /// `--doh URL` → surcharge `EXPLORER_DOH` (étape 72) —
    /// endpoint DNS-over-HTTPS (`https://…/dns-query`) ;
    /// absent = résolution système.
    doh: Option<String>,
}

/// Usage affiché sur `--help` ou argument invalide.
fn print_usage() {
    eprintln!(
        "explorer-daemon — hôte moteur + API REST/SSE loopback (ADR-0003)\n\
         usage : explorer-daemon [--port N] [--profile DIR] [--download-dir DIR] [--help]\n\
         \x20 --port N            port d'écoute (surcharge EXPLORER_PORT)\n\
         \x20 --profile DIR       répertoire de profil persistant\n\
         \x20                     (surcharge EXPLORER_PROFILE)\n\
         \x20 --download-dir DIR  répertoire des téléchargements acceptés\n\
         \x20                     (surcharge EXPLORER_DOWNLOAD_DIR ;\n\
         \x20                     défaut <profil>/downloads)\n\
         \x20 --doh URL           endpoint DNS-over-HTTPS\n\
         \x20                     (surcharge EXPLORER_DOH ;\n\
         \x20                     ex. https://r/dns-query)\n\
         \x20 --help              affiche cette aide\n\
         variables : EXPLORER_PORT, EXPLORER_PROFILE, EXPLORER_FONT_DIR,\n\
         \x20            EXPLORER_MAX_BODY, EXPLORER_API_KEY, EXPLORER_DOWNLOAD_DIR,\n\
         \x20            EXPLORER_DOH, EXPLORER_DOH_TIMEOUT_MS,\n\
         \x20            EXPLORER_COLOR_SCHEME (dark|light),\n\
         \x20            EXPLORER_JS_MEM_MIB (borne heap JS ; 0=illimité),\n\
         \x20            EXPLORER_JS_OPS (budget ops JS ; 0=illimité)"
    );
}

/// Sort sur erreur d'argument (exit 2 + usage).
fn arg_fail(msg: &str) -> ! {
    eprintln!("{msg}");
    print_usage();
    std::process::exit(2);
}

/// Parse les arguments — tout flag inconnu ou valeur
/// invalide est refusé (exit 2 + usage), `--help` affiche
/// et sort 0 (étape 62 : les missions passaient `--port`
/// en croyant qu'il était honoré).
fn parse_args() -> CliArgs {
    let mut out = CliArgs {
        port: None,
        profile: None,
        download_dir: std::env::var_os("EXPLORER_DOWNLOAD_DIR").map(PathBuf::from),
        doh: std::env::var("EXPLORER_DOH").ok().filter(|v| !v.is_empty()),
    };
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        let (flag, inline) = a
            .split_once('=')
            .map_or((a.as_str(), None), |(f, v)| (f, Some(v)));
        let mut value = |name: &str| -> String {
            inline
                .map(str::to_owned)
                .or_else(|| it.next())
                .unwrap_or_else(|| arg_fail(&format!("{name} : valeur absente")))
        };
        match flag {
            "--help" | "-h" => {
                print_usage();
                std::process::exit(0);
            }
            "--port" => {
                let v = value("--port");
                let Ok(p) = v.parse::<u16>() else {
                    arg_fail(&format!("--port : {v} n'est pas un port valide"));
                };
                out.port = Some(p);
            }
            "--profile" => out.profile = Some(PathBuf::from(value("--profile"))),
            "--download-dir" => {
                out.download_dir = Some(PathBuf::from(value("--download-dir")));
            }
            "--doh" => out.doh = Some(value("--doh")),
            _ => arg_fail(&format!("argument inconnu : {a}")),
        }
    }
    out
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
/// décodage `gzip`/`chunked` → jar à cookies → redirections
/// bornées — la même composition que `explorer-cli`. Le
/// `CookieJar` partagé avec la session (étape 51) est
/// **sous** la couche redirection : chaque hop re-reçoit
/// les cookies de sa propre URL.
fn http_stack(
    cfg: &DaemonConfig,
    jar: Arc<CookieJar>,
    profile: &Path,
    resolver: Option<Arc<dyn explorer_net::resolver::Resolver>>,
) -> Arc<dyn HttpTransport> {
    // Résolution injectable (étape 72) : `system` (défaut)
    // ou `doh` — le connecteur tente chaque IP résolue.
    let tls = Arc::new(match resolver {
        Some(r) => {
            TlsConnector::web_roots_over(Arc::new(explorer_net::h1::ResolvingConnector::new(r)))
        }
        None => TlsConnector::web_roots(),
    });
    // ALPN : h2 si le serveur le propose, h1 sinon (etape 41).
    let h1: Arc<dyn HttpTransport> = Arc::new(AutoClient::new(tls, HttpConfig::default()));
    let decoded: Arc<dyn HttpTransport> = Arc::new(DecodingTransport::wrap(h1, cfg.max_body));
    let with_cookies: Arc<dyn HttpTransport> = Arc::new(CookieTransport::wrap(decoded, jar));
    // Cache disque borné par URL (étape 56) — à l'intérieur
    // de la couche redirection : chaque hop est caché sous
    // sa propre URL (les 301 sont eux-mêmes cachables).
    let disk = Arc::new(cache::DiskCache::open(
        &profile.join("cache"),
        explorer_net::cache::CacheConfig::default(),
    ));
    let cached: Arc<dyn HttpTransport> = Arc::new(explorer_net::cache::CachingTransport::wrap(
        with_cookies,
        disk,
        explorer_net::cache::CacheConfig::default(),
    ));
    Arc::new(RedirectTransport::wrap(cached, cfg.max_redirects))
}

/// Construction de la `Session` — pile HTTP + dial WebSocket
/// (JS-C94, sockets dédiées hors pool `AutoClient`) + store +
/// répertoire de téléchargements + factory JS. Extraite de
/// `main` (périmètre de câblage, pas de logique).
fn build_session(
    cfg: &DaemonConfig,
    fonts: DirFontSource,
    store: Arc<store::JsonFileStore>,
    jar: Arc<CookieJar>,
    profile_dir: &Path,
    download_dir: Option<PathBuf>,
    resolver: Option<Arc<dyn explorer_net::resolver::Resolver>>,
) -> Session {
    // Dial `WebSocket` (JS-C94) — même connecteur TLS +
    // résolution que la pile HTTP, mais **sockets dédiées**
    // (une par flux, hors pool `AutoClient`). Construit
    // avant `http_stack` qui consomme `resolver`.
    let ws_conn: Arc<dyn explorer_net::h1::Connector> = match resolver.clone() {
        Some(r) => Arc::new(TlsConnector::web_roots_over(Arc::new(
            explorer_net::h1::ResolvingConnector::new(r),
        ))),
        None => Arc::new(TlsConnector::web_roots()),
    };
    let mut session = Session::with_config(
        http_stack(cfg, jar.clone(), profile_dir, resolver),
        Arc::new(fonts),
        session_config(cfg),
    );
    session.set_cookie_jar(jar);
    session.set_ws_dial(Arc::new(explorer_net::ws::RealWsDial::new(
        ws_conn,
        explorer_net::ws::WsLimits::default(),
    )));
    session.set_store(store);
    // Répertoire des téléchargements acceptés sans chemin
    // explicite (étape 70) — `<profil>/downloads` par défaut.
    session.set_download_dir(Some(
        download_dir.unwrap_or_else(|| profile_dir.join("downloads")),
    ));
    session.set_js_factory(explorer_js::JscoreEngine::factory());
    session
}

fn main() {
    // Sentinelle de mort (vol PIPE : code 1 + stderr vide à
    // ~6e-7e site, deux vols de suite). Principe : `alive`
    // écrit au départ, effacé à l'arrêt propre — un vol qui
    // laisse `alive` = mort externe (kill, stack overflow,
    // alloc-abort : aucun de ces chemins ne passe par un
    // crochet Rust) ; une entrée `panic:` = crochet exécuté
    // → le panic est nommé. `EXPLORER_DEATH_LOG` surchage le
    // chemin (défaut `$TEMP/explorer_daemon_death.log`).
    let death_log = std::env::var("EXPLORER_DEATH_LOG")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("explorer_daemon_death.log"));
    let _ = std::fs::write(&death_log, "alive\n");
    {
        let p = death_log.clone();
        std::panic::set_hook(Box::new(move |info| {
            let loc = info
                .location()
                .map(|l| format!("{l}"))
                .unwrap_or_else(|| "?".to_string());
            let _ = std::fs::write(&p, format!("panic: {info} @ {loc}\n"));
            eprintln!("explorer-daemon : panic {info} @ {loc}");
        }));
    }
    // Pile large pour **tous** les threads du runtime (workers
    // + pool `spawn_blocking`) : le thread moteur jscore porte
    // déjà 32 Mio (JS-C41) ; un overflow sur un worker —
    // apply/layout récursif sur DOM profond, dispatch
    // ré-entrant — tue le processus sans unwind (signature du
    // crash x.com intermittent mesuré au vol). 32 Mio ici =
    // hypothèse fix+diagnostic : si la mort revient, ce n'est
    // pas la pile ; si elle disparaît, mécanisme confirmé.
    tokio::runtime::Builder::new_multi_thread()
        .thread_stack_size(32 * 1024 * 1024)
        .enable_all()
        .build()
        .expect("runtime tokio")
        .block_on(amain());
    // Arrêt propre (CTRL-C / shutdown) : la sentinelle est
    // effacée — un `alive` résiduel après une mort signifie
    // « aucun crochet n'a couru » (mort externe).
    let _ = std::fs::remove_file(&death_log);
}

#[allow(clippy::too_many_lines)] // setup séquentiel — chaque bloc est documenté in situ
async fn amain() {
    // Défaut `info` sans `RUST_LOG` (vol 18 : les `warn`
    // iframe étaient perdus — le filtre vide ne passe que
    // `error` ; le daemon est un hôte de diagnostic, les
    // refus d'embarquement/sous-ressources doivent sortir).
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();
    let mut cfg = DaemonConfig::from_env();
    let cli = parse_args();
    if let Some(p) = cli.port {
        cfg.bind.set_port(p);
    }
    let fonts = DirFontSource::new(&cfg.fonts_dir);
    tracing::info!(dir = %cfg.fonts_dir.display(), "fontes système scannées");

    let profile_dir = cli
        .profile
        .unwrap_or_else(store::JsonFileStore::default_dir);
    let store = Arc::new(store::JsonFileStore::open(&profile_dir));
    tracing::info!(dir = %profile_dir.display(), "profil chargé");
    // Jar partagé transport ↔ session (étape 51) :
    // `document.cookie`, les XHR same-origin et les
    // navigations voient le même état.
    let jar = Arc::new(CookieJar::new());
    // Résolution DoH optionnelle (étape 72) : l'endpoint est
    // résolu par le système via une pile TLS ordinaire — pas
    // de récursion DoH, aucun endpoint en dur.
    let (resolver, egress) = match cli.doh.as_deref() {
        Some(raw) => match explorer_net::url::Url::parse(raw) {
            Ok(u) if u.scheme() == "https" => {
                let doh_transport: Arc<dyn HttpTransport> = Arc::new(AutoClient::new(
                    Arc::new(TlsConnector::web_roots()),
                    HttpConfig::default(),
                ));
                let r: Arc<dyn explorer_net::resolver::Resolver> = Arc::new(
                    explorer_net::resolver::DohResolver::new(u, doh_transport, cfg.doh_timeout),
                );
                let egress = r.describe();
                tracing::info!(egress = %egress, "résolution DoH configurée");
                (Some(r), egress)
            }
            Ok(_) => arg_fail("--doh : l'endpoint doit être en https://"),
            Err(e) => arg_fail(&format!("--doh : URL invalide : {e}")),
        },
        None => (None, "system".to_string()),
    };
    let session = Arc::new(tokio::sync::Mutex::new(build_session(
        &cfg,
        fonts,
        store,
        jar,
        &profile_dir,
        cli.download_dir,
        resolver,
    )));

    // Battement des réponses réseau script-initiées (étape
    // 51) + timers JS (étape 52) : la session n'a pas
    // d'horloge — un XHR asynchrone est livré à la prochaine
    // pompe, un `setTimeout` tire sans attendre un input.
    // La sieste est `min(échéance timer, NET_PUMP_MS)` : un
    // timer à 10 ms n'attend pas le battement complet.
    // Étapes 53d/53f : drains de navigation et pompe JS
    // hors verrou — ni une `location.href=` tirée par un
    // timer ni un burst `run_pending`/`sync_dom` ne gèlent
    // l'API pendant leur exécution.
    {
        let s = session.clone();
        tokio::spawn(async move {
            loop {
                let (wait, tabs) = {
                    let mut sess = s.lock().await;
                    tokio::task::yield_now().await;
                    sess.poll_net();
                    let w = sess
                        .next_timer_deadline()
                        .map_or(NET_PUMP_MS, |d| d.min(NET_PUMP_MS));
                    (w, sess.tabs())
                };
                // Livraisons réseau → resynchros enfilées,
                // appliquées hors verrou (étape 55h).
                for tab in &tabs {
                    explorer_core::drain_applies_shared(&s, *tab).await;
                }
                // Pompe JS par onglet : `run_pending` +
                // `sync_dom` tournent en `spawn_blocking`
                // avec le moteur extrait — le verrou n'est
                // tenu qu'au take/commit (étape 53f).
                for tab in &tabs {
                    explorer_core::pump_timers_shared(&s, *tab).await;
                }
                // Un timer peut émettre `location.href` —
                // les intentions sont honorées ici, hors
                // verrou (la pompe reste à sens unique).
                for tab in tabs {
                    Box::pin(explorer_core::drain_navs_shared(&s, tab)).await;
                }
                tokio::time::sleep(wait).await;
            }
        });
    }
    let app = explorer_api::router_with_egress(session, cfg.api_key.clone(), egress);
    if cfg.api_key.is_some() {
        tracing::info!("clé d'API requise (x-api-key / ?key=)");
    }

    let listener = match tokio::net::TcpListener::bind(cfg.bind).await {
        Ok(l) => l,
        Err(e) => {
            // Fatal : stderr direct — un filtre RUST_LOG
            // ciblé (ex. sync_profile) ne doit pas avaler la
            // raison de mort (étape 77, mission 19).
            tracing::error!(addr = %cfg.bind, error = %e, "bind impossible");
            eprintln!("explorer-daemon : bind {} impossible : {e}", cfg.bind);
            std::process::exit(1);
        }
    };
    tracing::info!(addr = %cfg.bind, "explorer-daemon prêt");
    let r = axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await;
    if let Err(e) = r {
        tracing::error!(error = %e, "serveur arrêté");
        eprintln!("explorer-daemon : boucle serveur terminée en erreur : {e}");
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
