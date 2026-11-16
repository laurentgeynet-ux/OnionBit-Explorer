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
    /// étape 124) — `None` = [`DEFAULT_DAEMON_JS_OPS`] (20 M,
    /// relève le défaut `SessionConfig` 5 M : wordpress et
    /// bootstraps légitimes dépassaient 5 M — `RangeError:
    /// script execution limit` au boot, vol 25) ;
    /// `0` = illimité explicite (mappé `u64::MAX` — un `while(1)`
    /// gèle alors le contexte : réglage diagnostic uniquement).
    js_ops: Option<u64>,
    /// Viewport initial `WxH` (`EXPLORER_VIEWPORT`, étape 129 —
    /// ex. `1366x900` ; `None` = défaut `SessionConfig`
    /// 1024×768). Même format que l'argument `LxH` de
    /// `explorer-cli shot`.
    viewport: Option<(f32, f32)>,
    /// Politique de consentement CMP (`EXPLORER_CONSENT_POLICY`,
    /// étape 171 — ADR-0031) : `honest` (aucun état consenti),
    /// `reject` (« Tout refuser », défaut) ou `accept` (outil
    /// de diagnostic). Réglage de vol autant que produit : les
    /// agents comparent les murs CMP sous chaque politique.
    consent_policy: explorer_net::consent::ConsentPolicy,
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
            consent_policy: match std::env::var("EXPLORER_CONSENT_POLICY").as_deref() {
                Ok("honest") => explorer_net::consent::ConsentPolicy::Honest,
                Ok("accept" | "acceptall") => explorer_net::consent::ConsentPolicy::AcceptAll,
                _ => explorer_net::consent::ConsentPolicy::RejectAll,
            },
        }
    }
}

/// `SessionConfig` dérivée du `DaemonConfig` — schéma de
/// couleurs + surcharge de la borne heap JS
/// (`EXPLORER_JS_MEM_MIB`, étape 123 : outillage diagnostic
/// de vol, pas une promesse produit ; la valeur active est
/// tracée pour que le rapport de terrain cite la borne en
/// vigueur, jamais supposée).
/// Budget d'ops JS par évaluation côté **daemon** (vol 25) :
/// 20 M — le défaut `SessionConfig` (5 M, pensé pour le CLI
/// one-shot) tuait des bootstraps légitimes (`RangeError:
/// script execution limit exceeded` sur wordpress) ; la
/// borne reste finie — un script hostile est toujours
/// interrompu, juste plus tard (le disjoncteur « moteur
/// figé » couvre le worker saturé, vague 8).
const DEFAULT_DAEMON_JS_OPS: u64 = 20_000_000;

fn session_config(cfg: &DaemonConfig) -> explorer_core::SessionConfig {
    let mut sc = explorer_core::SessionConfig {
        color_scheme: cfg.color_scheme,
        consent_policy: cfg.consent_policy,
        ..explorer_core::SessionConfig::default()
    };
    if cfg.consent_policy != explorer_net::consent::ConsentPolicy::RejectAll {
        tracing::info!(
            "EXPLORER_CONSENT_POLICY={:?} — politique de consentement CMP",
            cfg.consent_policy
        );
    }
    if let Some(mib) = cfg.js_mem_mib {
        tracing::info!("EXPLORER_JS_MEM_MIB={mib} — borne heap JS par contexte");
        sc.js_mem_limit_mib = mib;
    }
    // `0` = illimité explicite (étape 124) — `op_limit=0`
    // tuerait le premier `tick` ; `u64::MAX` en tient lieu.
    let ops = cfg.js_ops.unwrap_or(DEFAULT_DAEMON_JS_OPS);
    sc.js_ops = if ops == 0 { u64::MAX } else { ops };
    if cfg.js_ops.is_some() {
        tracing::info!("EXPLORER_JS_OPS={ops} — budget d'ops JS par évaluation");
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
    /// `--name NOM` — nom publié dans le registre de daemons
    /// (étape 256 ; défaut : base du répertoire de profil).
    name: Option<String>,
}

/// Usage affiché sur `--help` ou argument invalide.
fn print_usage() {
    eprintln!(
        "explorer-daemon — hôte moteur + API REST/SSE loopback (ADR-0003)\n\
         usage : explorer-daemon [--port N] [--profile DIR] [--download-dir DIR] [--help]\n\
         \x20 --port N            port d'écoute (surcharge EXPLORER_PORT ;\n\
         \x20                     0 = port éphémère, publié au registre)\n\
         \x20 --name NOM          nom d'annonce au registre de daemons\n\
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
         \x20            EXPLORER_JS_OPS (budget ops JS ; 0=illimité),\n\
         \x20            EXPLORER_DEATH_LOG (sentinelle de mort),\n\
         \x20            EXPLORER_FLIGHT_LOG/EXPLORER_FLIGHT_MS/\n\
         \x20            EXPLORER_FLIGHT_MAX_KB (enregistreur de vol),\n\
         \x20            EXPLORER_RSS_LIMIT_MB/EXPLORER_COMMIT_LIMIT_MB\n\
         \x20            (garde mémoire §B-59 ; 0/absent=désarmé)"
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
        name: None,
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
            "--name" => out.name = Some(value("--name")),
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
    // Rejeu d'un vol enregistré (explorer-net `replay`, N0.1) —
    // priorité absolue : aucun socket ouvert, la session rejoue
    // les réponses du dossier (même convention qu'`explorer-cli`).
    if let Ok(dir) = std::env::var("EXPLORER_NET_REPLAY") {
        if !dir.is_empty() {
            // Nom nu → `<Explorer-data>/record/<nom>` (étape
            // 262) ; chemin explicite → honoré.
            let dir = explorer_core::paths::resolve_record_arg(&dir);
            tracing::info!(dir = %dir.display(), "EXPLORER_NET_REPLAY — rejeu offline");
            return Arc::new(explorer_net::replay::DirTransport::new(&dir));
        }
    }
    // Résolution injectable (étape 72) : `system` (défaut)
    // ou `doh` — le connecteur tente chaque IP résolue. Le
    // cache DNS persisté (PR-9) enveloppe le résolveur choisi.
    let base: Arc<dyn explorer_net::resolver::Resolver> =
        resolver.unwrap_or_else(|| Arc::new(explorer_net::resolver::SystemResolver));
    let tls = Arc::new(TlsConnector::web_roots_over(Arc::new(
        explorer_net::h1::ResolvingConnector::new(Arc::new(
            explorer_net::resolver::CachingResolver::wrap(
                base,
                Arc::new(explorer_core::JsonDnsStore::open(profile)),
                explorer_net::resolver::DNS_CACHE_TTL,
            ),
        )),
    )));
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
    // HSTS (PR-3) : upgrade http→https + mémorise STS — sous la
    // couche redirection, au-dessus du cache (les upgrades
    // réutilisent les entrées existantes).
    let hsts: Arc<dyn HttpTransport> = Arc::new(explorer_net::hsts::HstsTransport::wrap(
        cached,
        Arc::new(explorer_core::JsonHstsStore::open(profile)),
    ));
    let stack: Arc<dyn HttpTransport> = Arc::new(RedirectTransport::wrap(hsts, cfg.max_redirects));
    // Enregistrement de vol (N0.1) — par présence, comme
    // `EXPLORER_DOM_KILL` : chaque réponse servie est écrite
    // (`<clé>-<seq>.resp` + sidecar `.req` pour les corps) —
    // rejouable via `EXPLORER_NET_REPLAY`. Couche externe :
    // les redirections sont déjà résolues (même placement
    // qu'`explorer-cli`, rejeu cohérent entre les deux binaires).
    match std::env::var("EXPLORER_NET_RECORD") {
        Ok(dir) if !dir.is_empty() => {
            // Nom nu → `<Explorer-data>/record/<nom>` (étape
            // 262 — la capture reste sous le projet).
            let dir = explorer_core::paths::resolve_record_arg(&dir);
            tracing::info!(dir = %dir.display(), "EXPLORER_NET_RECORD — capture des réponses");
            Arc::new(explorer_net::replay::RecordingTransport::wrap(stack, &dir))
        }
        _ => stack,
    }
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
    // Étape 3b — hôte à session partagée : les dispatches
    // input tournent hors verrou (`handle_input_shared`) et
    // le replay des entrées différées n'est jamais exécuté
    // sous verrou dans un commit — il sort par
    // `drain_deferred_inputs_shared` en fin de drain.
    session.set_events_offlock(true);
    // Capacités média de l'hôte déclarées au spawn (étape
    // 178 — b-e2e) : `EXPLORER_MEDIA_CODECS` liste
    // séparée par `;` (MIME `codecs="a,b"` ou codecs nus —
    // la liste `mediaCodecs` de l'app ; la `,` interne à
    // `codecs=` reste dans l'item, `note_codec_prefixes`
    // l'éclate). Sans elle, `MediaSource.isTypeSupported`
    // répond `""` sur toute page fraîche et aucun player
    // MSE réel ne démarre.
    if let Ok(v) = std::env::var("EXPLORER_MEDIA_CODECS") {
        let codecs: Vec<String> = v
            .split(';')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect();
        if !codecs.is_empty() {
            session.declare_media_codecs(&codecs);
        }
    }
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
    // `EXPLORER_DEATH_LOG` surchage le chemin ; défaut
    // `<Explorer-data>/logs/` (étape 262 — rien hors du
    // projet), `$TEMP` seulement hors contexte projet.
    let death_log = std::env::var("EXPLORER_DEATH_LOG").map_or_else(
        |_| {
            explorer_core::paths::logs_dir().map_or_else(
                || std::env::temp_dir().join("explorer_daemon_death.log"),
                |d| {
                    let _ = std::fs::create_dir_all(&d);
                    d.join("explorer_daemon_death.log")
                },
            )
        },
        PathBuf::from,
    );
    let _ = std::fs::write(&death_log, "alive\n");
    {
        let p = death_log.clone();
        std::panic::set_hook(Box::new(move |info| {
            let loc = info
                .location()
                .map_or_else(|| "?".to_string(), |l| format!("{l}"));
            let _ = std::fs::write(&p, format!("panic: {info} @ {loc}\n"));
            eprintln!("explorer-daemon : panic {info} @ {loc}");
        }));
    }
    // Garde-fou RSS interne (§B-59 burst ~3-5 Go/s — sudouest
    // monte 8-12 Go en 2-3 s : aucun sondage externe ne gagne,
    // la station encaissait avant chaque kill). Sondage du
    // WorkingSet propre toutes les 50 ms — à 5 Go/s le
    // dépassement plafonne à ~250 Mo au lieu de plusieurs Go.
    // Armé par `EXPLORER_RSS_LIMIT_MB` (vols §B-59 seulement ;
    // absent = désarmé, aucun coût en production). Au seuil :
    // marqueur `rss_guard:` au death_log puis abort — la mort
    // est nommée et reste distincte des kills externes.
    let rss_mb = std::env::var("EXPLORER_RSS_LIMIT_MB")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|m| *m > 0);
    // Garde complémentaire sur la **charge commit**
    // (`virtual_mem` = PagefileUsage) : le burst §B-59 monte
    // `virt` à ~13 Go pendant que `rss` reste ~5 Go — une
    // réserve/commit seule épuise le pagefile et couche la
    // station **avant** que le seuil RSS ne soit atteint.
    // Armé par `EXPLORER_COMMIT_LIMIT_MB` (vols §B-59).
    let commit_mb = std::env::var("EXPLORER_COMMIT_LIMIT_MB")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|m| *m > 0);
    if rss_mb.is_some() || commit_mb.is_some() {
        let p = death_log.clone();
        std::thread::spawn(move || loop {
            let m = memory_stats::memory_stats();
            let rss = m.map_or(0, |s| s.physical_mem) / (1024 * 1024);
            let virt = m.map_or(0, |s| s.virtual_mem) / (1024 * 1024);
            let hit = rss_mb
                .filter(|mb| rss > *mb)
                .map(|mb| ("rss", mb, rss))
                .or_else(|| {
                    commit_mb
                        .filter(|mb| virt > *mb)
                        .map(|mb| ("commit", mb, virt))
                });
            if let Some((kind, mb, cur)) = hit {
                let nat = explorer_jscore::LAST_NAT.lock().map_or("?", |g| *g);
                let expr = explorer_jscore::LAST_EXPR.lock().map_or("?", |g| *g);
                let stmt = explorer_jscore::LAST_STMT.lock().map_or("?", |g| *g);
                let areq =
                    explorer_jscore::LAST_ALLOC_REQ.load(std::sync::atomic::Ordering::Relaxed);
                let stats = explorer_jscore::LAST_STATS
                    .lock()
                    .map_or_else(|_| String::new(), |g| g.clone());
                let frame = explorer_jscore::LAST_FRAME
                    .lock()
                    .map_or_else(|_| String::new(), |g| g.clone());
                // §B-59 — marqueurs de zone de mort : phase du
                // fil JS (`call`/`body`, inconditionnel) et
                // drain hôte (`poll_net`/`timers`/`commit`…) —
                // « préambule JS vs drain core vs hors-fil »
                // tranché en un seul abort.
                let phase = explorer_jscore::last_phase_str();
                let drain = explorer_core::LAST_DRAIN.lock().map_or("?", |g| *g);
                let _ = std::fs::write(
                    &p,
                    format!("rss_guard[{kind}]: {cur} Mo > {mb} Mo | last_nat={nat} | last_stmt={stmt} | last_expr={expr} | last_areq={areq} | last_frame={frame} | last_phase={phase} | last_drain={drain} | last_stats={stats}\n"),
                );
                eprintln!(
                    "explorer-daemon : rss_guard[{kind}] {cur} Mo > {mb} Mo — abort | last_nat={nat} | last_stmt={stmt} | last_expr={expr} | last_areq={areq} | last_frame={frame} | last_phase={phase} | last_drain={drain} | last_stats={stats}"
                );
                std::process::abort();
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        });
    }
    spawn_flight_recorder(&death_log);
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
        .block_on(amain(death_log.clone()));
    // Arrêt propre (CTRL-C / shutdown) : la sentinelle est
    // effacée — un `alive` résiduel après une mort signifie
    // « aucun crochet n'a couru » (mort externe). Étape 276 :
    // un marqueur `console:<kind>` (fermeture console/session
    // Windows — kill externe nommé) est conservé, pas effacé.
    // `console:*` (kill console) et `shutdown:*` (arrêt
    // demandé) sont des fins nommées — on les garde pour la
    // post-mortem ; `alive` résiduel = arrêt ordinaire, nettoyé.
    let keep = std::fs::read_to_string(&death_log)
        .is_ok_and(|s| s.starts_with("console:") || s.starts_with("shutdown:"));
    if !keep {
        let _ = std::fs::remove_file(&death_log);
    }
}

/// Enregistreur de vol unifié (étape 276 — morts externes
/// KAYAK ×3 : `alive` résiduel, zéro panic, zéro WER). La
/// sentinelle distingue déjà « crochet couru » de « mort
/// externe » mais ne nomme pas le site — ce thread
/// vidange les balises §B-59 du fil JS (`LAST_*`) +
/// `LAST_DRAIN` (drains core) + RSS/commit vers
/// `logs/explorer_daemon_flight.log` toutes les
/// `EXPLORER_FLIGHT_MS` ms (défaut 400, 0 = désarmé).
/// Lecture postmortem : dernier beat figé sur une balise
/// = spin à ce site ; beats vivants mais `drain` fixe =
/// wedge ; montée `commit` avant l'arrêt = OOM ; arrêt
/// franc avec `alive` = kill externe. Rotation `.prev`
/// bornée par `EXPLORER_FLIGHT_MAX_KB` (défaut 4096) ;
/// `EXPLORER_FLIGHT_LOG` surcharge le chemin.
fn spawn_flight_recorder(death_log: &Path) {
    let flight_log = std::env::var("EXPLORER_FLIGHT_LOG").map_or_else(
        |_| {
            death_log
                .parent()
                .map(|d| d.join("explorer_daemon_flight.log"))
        },
        |v| Some(PathBuf::from(v)),
    );
    let flight_ms = std::env::var("EXPLORER_FLIGHT_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(400);
    let flight_max_kb = std::env::var("EXPLORER_FLIGHT_MAX_KB")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(4096);
    let Some(flog) = flight_log else { return };
    if flight_ms == 0 {
        return;
    }
    // JS-C566 — arme le breadcrumb `host=` : l'écriture
    // `Mutex<String>` côté `host_mark` ne court que si un
    // lecteur est abonné (ce recorder).
    explorer_jscore::host_mark_enable();
    let get = |m: &std::sync::Mutex<&'static str>| m.lock().map_or("?", |g| *g);
    let gets =
        |m: &std::sync::Mutex<String>| m.lock().map(|g| g.replace('\n', "\\n")).unwrap_or_default();
    std::thread::spawn(move || {
        let t0 = std::time::Instant::now();
        let mut header_done = false;
        loop {
            if !header_done {
                append_flight(
                    &flog,
                    flight_max_kb,
                    &format!("=== daemon start pid={} ===\n", std::process::id()),
                );
                header_done = true;
            }
            let (mut rss, mut com) = (0usize, 0usize);
            if let Some(m) = memory_stats::memory_stats() {
                rss = m.physical_mem / (1024 * 1024);
                com = m.virtual_mem / (1024 * 1024);
            }
            let drain = explorer_core::LAST_DRAIN.lock().map_or("?", |g| *g);
            let areq = explorer_jscore::LAST_ALLOC_REQ.load(std::sync::atomic::Ordering::Relaxed);
            let calls = explorer_jscore::CALL_N.load(std::sync::atomic::Ordering::Relaxed);
            // Étape 279 — balise GC en vol : `calls` figé +
            // `gc` > 0 = wedge mark/sweep nommé ; `gc`=0 + `calls`
            // figé = vrai spin dans le site `host`/`phase`.
            let gc = explorer_jscore::GC_ACTIVE.load(std::sync::atomic::Ordering::Relaxed);
            let gcp = explorer_jscore::GC_PASSES.load(std::sync::atomic::Ordering::Relaxed);
            let gcms = explorer_jscore::GC_LAST_MS.load(std::sync::atomic::Ordering::Relaxed);
            // Étape 281 — lectures/décodages `responseText` :
            // `xrt` reads ≫ decodes = boucle de lecture nommée.
            let xrr = explorer_jscore::XHR_TEXT_READS.load(std::sync::atomic::Ordering::Relaxed);
            let xrd = explorer_jscore::XHR_TEXT_DECODES.load(std::sync::atomic::Ordering::Relaxed);
            let xrm =
                explorer_jscore::XHR_TEXT_DECODE_MS.load(std::sync::atomic::Ordering::Relaxed);
            let line = format!(
                "t={}ms rss={}MiB commit={}MiB | drain={} | calls={} | gc={gc}/{gcp}@{gcms}ms | xrt={xrr}/{xrd}@{xrm}ms | frame={} | fun={} | stmt={} | expr={} | nat={} | host={} | phase={} | areq={} | stats={}\n",
                t0.elapsed().as_millis(),
                rss,
                com,
                drain,
                calls,
                gets(&explorer_jscore::LAST_FRAME),
                gets(&explorer_jscore::LAST_FUN),
                get(&explorer_jscore::LAST_STMT),
                get(&explorer_jscore::LAST_EXPR),
                get(&explorer_jscore::LAST_NAT),
                gets(&explorer_jscore::LAST_HOST),
                explorer_jscore::last_phase_str(),
                areq,
                gets(&explorer_jscore::LAST_STATS),
            );
            append_flight(&flog, flight_max_kb, &line);
            std::thread::sleep(std::time::Duration::from_millis(flight_ms));
        }
    });
}

/// Append borné du flight log (étape 276) : rotation
/// `.prev` quand le fichier dépasse `max_kb` — deux
/// générations conservées, taille totale ≤ ~2×max. Les
/// beats sont mono-lignes (chaîne de la morgue §B-59) —
/// postmortem : dernier beat = site vivant.
fn append_flight(path: &Path, max_kb: u64, line: &str) {
    use std::io::Write as _;
    if let Ok(meta) = std::fs::metadata(path) {
        if meta.len() > max_kb.saturating_mul(1024) {
            let prev = path.with_extension("prev");
            let _ = std::fs::remove_file(&prev);
            let _ = std::fs::rename(path, &prev);
        }
    }
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = f.write_all(line.as_bytes());
    }
}

/// Writer stderr + fichier optionnel (`EXPLORER_LOG` →
/// `<Explorer-data>/logs/daemon-<pid>.log`, étape 262 —
/// les journaux de vol restent sous le projet).
/// `MakeWriter` attend une fabrique par appel.
struct TeeFile {
    f: Option<Arc<std::sync::Mutex<std::fs::File>>>,
}

impl<'a> tracing_subscriber::fmt::writer::MakeWriter<'a> for TeeFile {
    type Writer = TeeWriter;
    fn make_writer(&'a self) -> Self::Writer {
        TeeWriter(self.f.clone())
    }
}

struct TeeWriter(Option<Arc<std::sync::Mutex<std::fs::File>>>);

impl std::io::Write for TeeWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let n = std::io::stderr().write(buf)?;
        if let Some(f) = &self.0 {
            if let Ok(mut g) = f.lock() {
                let _ = g.write_all(buf);
            }
        }
        Ok(n)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        std::io::stderr().flush()
    }
}

#[allow(clippy::too_many_lines)] // setup séquentiel — chaque bloc est documenté in situ
async fn amain(death_log: PathBuf) {
    // Défaut `info` sans `RUST_LOG` (vol 18 : les `warn`
    // iframe étaient perdus — le filtre vide ne passe que
    // `error` ; le daemon est un hôte de diagnostic, les
    // refus d'embarquement/sous-ressources doivent sortir).
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    // stderr non bufferisé (comme explorer-cli) : stdout sous
    // redirection Windows est block-bufferisé — les journaux de
    // vol restaient à 0 octet jusqu'à la mort du daemon.
    // `EXPLORER_LOG` (présence) → tee stderr + fichier sous
    // `<Explorer-data>/logs/` (étape 262).
    let log_file = std::env::var_os("EXPLORER_LOG")
        .is_some()
        .then(|| {
            explorer_core::paths::logs_dir().map(|d| {
                let _ = std::fs::create_dir_all(&d);
                d.join(format!("daemon-{}.log", std::process::id()))
            })
        })
        .flatten()
        .and_then(|p| std::fs::File::create(p).ok());
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(TeeFile {
            f: log_file.map(|f| Arc::new(std::sync::Mutex::new(f))),
        })
        .init();
    let mut cfg = DaemonConfig::from_env();
    let cli = parse_args();
    if let Some(p) = cli.port {
        cfg.bind.set_port(p);
    }
    let fonts = DirFontSource::new(&cfg.fonts_dir);
    tracing::info!(dir = %cfg.fonts_dir.display(), "fontes système scannées");

    let profile_dir = cli
        .profile
        // Nom nu (`--profile drive`, `vol:<nom>`) →
        // `profiles/<nom>`/`vols/<nom>/profile` sous
        // `Explorer-data` (étape 262) ; chemin explicite →
        // honoré (override volontaire, pas du hardcodage).
        .map_or_else(store::JsonFileStore::default_dir, |p| {
            p.to_str()
                .and_then(explorer_core::paths::resolve_profile_name)
                .unwrap_or(p)
        });
    // Verrou d'instance unique sur le profil — partagé avec le
    // mode FFI (`acquire_profile_lock`, même sémantique :
    // deux processus sur le même state/ = deux writers JSON).
    // Gardé en vie jusqu'à la fin du processus.
    let _profile_lock = match explorer_core::acquire_profile_lock(&profile_dir) {
        Ok(l) => l,
        Err(msg) => {
            eprintln!("explorer-daemon : {msg}");
            std::process::exit(2);
        }
    };
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

    // §B-59 — échantillonneur `mem_diag` (EXPLORER_MEM_DIAG=1) :
    // `mem_report` complet par onglet toutes les 500 ms — la
    // pompe ne suffit pas : la fuite constatée sur le terrain
    // (~1 Go/s) se joue hors fenêtre de commit (run détaché,
    // tâches réseau/décode d'arrière-plan). Le compteur qui
    // suit la RSS nomme le réservoir.
    if std::env::var_os("EXPLORER_MEM_DIAG").is_some_and(|v| v == "1") {
        let s = session.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                let mut sess = s.lock().await;
                for tab in sess.tabs() {
                    if let Some(r) = sess.mem_report(tab) {
                        tracing::info!(
                            target: "mem_diag",
                            tab,
                            dom = r.dom_nodes,
                            boxes = r.layout_boxes,
                            dl = r.display_items,
                            frame_b = r.frame_bytes,
                            png_b = r.png_bytes,
                            img_b = r.images_bytes,
                            bodies_b = r.bg_bodies_bytes + r.img_bodies_bytes,
                            heap_b = r.js_heap_bytes,
                            bf_dom = r.bfcache_dom_nodes,
                            bf_img_b = r.bfcache_images_bytes,
                            total_b = r.total_bytes,
                            "mem_diag"
                        );
                    }
                }
            }
        });
    }

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
                let cycle_start = std::time::Instant::now();
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
                // Yield entre onglets (vol 15 cdiscount :
                // famine du flux API sous JS continu — les
                // phases `commit` de chaque pompe tiennent
                // le Mutex en livrées moteur ; céder le
                // tour laisse les handlers HTTP passer
                // entre deux onglets).
                for tab in &tabs {
                    explorer_core::pump_timers_shared(&s, *tab).await;
                    tokio::task::yield_now().await;
                }
                // Un timer peut émettre `location.href` —
                // les intentions sont honorées ici, hors
                // verrou (la pompe reste à sens unique).
                for tab in tabs {
                    Box::pin(explorer_core::drain_navs_shared(&s, tab)).await;
                }
                // Plancher de sieste proportionnel au cycle
                // écoulé (JS-C293, mur YouTube) : deux
                // `setInterval` 25 ms dont chaque callback
                // coûte ~50 ms d'interpréteur sont échus en
                // continu → boucle chaude, moteur ~100 %
                // occupé, évals et livrées affamées. Un
                // cycle lourd impose un repos équivalent
                // (duty ≤ ~50 % — les navigateurs sautent
                // aussi les ticks en retard) ; un cycle
                // léger garde la cadence `deadline`-driven.
                let elapsed = cycle_start.elapsed();
                tokio::time::sleep(wait.max(elapsed.min(NET_PUMP_MS * 4))).await;
            }
        });
    }
    // Surface agent EAP (ADR-0035) — `/agent` WS sur le même
    // listener : deuxième client de `Session`, même verrou,
    // même clé. Routeur disjoint du REST pour garder la
    // couture UI intacte.
    // Canal d'arrêt graceful (étape 296) : le handler
    // `POST /api/shutdown` pousse `true` — `shutdown_signal`
    // sélectionne dessus au même titre que Ctrl-C/console.
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let app = explorer_api::router_with_shutdown(
        session.clone(),
        cfg.api_key.clone(),
        egress,
        shutdown_tx,
    )
    .merge(explorer_agent::agent_router_with_profile(
        session,
        cfg.api_key.clone(),
        Some(profile_dir.clone()),
    ));
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
    // Registre de daemons (étape 256) : l'annonce publie le
    // port RÉEL (`local_addr` — `--port 0` devient ainsi
    // découvrable). La garde retire le fichier à l'arrêt
    // propre ; un crash laisse une entrée périmée que les
    // lecteurs éliminent par sonde.
    let bound_port = listener
        .local_addr()
        .map_or_else(|_| cfg.bind.port(), |a| a.port());
    let daemon_name =
        explorer_core::daemon_registry::default_name(cli.name.as_deref(), &profile_dir);
    let _announce_guard = match explorer_core::daemon_registry::announce(
        &daemon_name,
        bound_port,
        &profile_dir,
    ) {
        Ok(g) => {
            tracing::info!(name = %daemon_name, port = bound_port, "daemon annoncé au registre");
            Some(g)
        }
        Err(e) => {
            // Non fatal : le daemon sert sans annonce — la
            // découverte échoue, pas la navigation.
            tracing::warn!(error = %e, "annonce registre impossible");
            None
        }
    };
    tracing::info!(addr = %cfg.bind, "explorer-daemon prêt");
    let r = axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal(death_log, shutdown_rx))
        .await;
    if let Err(e) = r {
        tracing::error!(error = %e, "serveur arrêté");
        eprintln!("explorer-daemon : boucle serveur terminée en erreur : {e}");
        std::process::exit(1);
    }
    tracing::info!("arrêt propre");
}

/// Ctrl-C (SIGINT), SIGTERM unix ou **événement console
/// Windows** (`close`/`logoff`/`shutdown`) → fin du `serve`
/// sans couper les requêtes en vol.
///
/// Étape 276 : les événements console sont des kills externes
/// — une fermeture de console, de session ou un arrêt système
/// terminait le processus sans qu'aucun crochet ne coure
/// (`alive` résiduel, zéro panic, zéro WER — signature des
/// morts KAYAK field266/272/273). Ils sont désormais
/// interceptés et **nommés** dans la sentinelle
/// (`console:close`/`console:logoff`/`console:shutdown`,
/// conservée à l'arrêt) : un `alive` résiduel après cette
/// étape exclut le kill console et resserre le diagnostic
/// sur stack overflow, alloc-abort ou kill par PID.
async fn shutdown_signal(death_log: PathBuf, api: tokio::sync::watch::Receiver<bool>) {
    let ctrl_c = tokio::signal::ctrl_c();
    let api_stop = api_stop(api);
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        if let Ok(mut term) = signal(SignalKind::terminate()) {
            tokio::select! {
                () = async { let _ = ctrl_c.await; } => {}
                () = async { let _ = term.recv().await; } => {}
                () = api_stop => write_api_stop(&death_log),
            }
            return;
        }
    }
    #[cfg(windows)]
    {
        tokio::select! {
            k = windows_console_kind(ctrl_c) => {
                if let Some(k) = k {
                    // ~5 s avant le kill forcé après CTRL_CLOSE —
                    // suffisant pour nommer la mort.
                    let _ = std::fs::write(&death_log, format!("console:{k}\n"));
                    eprintln!("explorer-daemon : événement console {k} — arrêt nommé");
                }
            }
            () = api_stop => write_api_stop(&death_log),
        }
    }
    #[cfg(not(windows))]
    let _ = ctrl_c.await;
}

/// Le récepteur devient prêt quand `POST /api/shutdown` a
/// poussé `true` (étape 296) — canal `watch` : l'armement
/// survit à un ordre d'évaluation défavorable, pas de course
/// handler/sélecteur.
async fn api_stop(mut api: tokio::sync::watch::Receiver<bool>) {
    while api.changed().await.is_ok() {
        if *api.borrow() {
            return;
        }
    }
}

/// Arrêt propre demandé via l'API : nommé `shutdown:api` à
/// la sentinelle — la post-mortem distingue « demandé par le
/// canal de contrôle » d'un kill externe et des événements
/// console (`console:*`, étape 276).
fn write_api_stop(death_log: &Path) {
    let _ = std::fs::write(death_log, "shutdown:api\n");
    eprintln!("explorer-daemon : arrêt demandé via /api/shutdown");
}

/// Attente des événements console Windows (étape 276) :
/// `Some("close"|"logoff"|"shutdown")` à l'événement,
/// `None` sur Ctrl-C/Ctrl-Break. Chaque `CtrlX` est un flux
/// borné : `recv()` rend à l'événement ; l'enregistrement
/// échoué (`None`) retombe sur un `pending` — la branche ne
/// se déclenche jamais.
#[cfg(windows)]
async fn windows_console_kind(
    ctrl_c: impl std::future::Future<Output = std::io::Result<()>>,
) -> Option<&'static str> {
    use tokio::signal::windows::{ctrl_close, ctrl_logoff, ctrl_shutdown};
    let mut close = ctrl_close().ok();
    let mut logoff = ctrl_logoff().ok();
    let mut shutdown = ctrl_shutdown().ok();
    tokio::select! {
        _ = ctrl_c => None,
        () = async {
            match close.as_mut() {
                Some(s) => {
                    let _ = s.recv().await;
                }
                None => std::future::pending::<()>().await,
            }
        } => Some("close"),
        () = async {
            match logoff.as_mut() {
                Some(s) => {
                    let _ = s.recv().await;
                }
                None => std::future::pending::<()>().await,
            }
        } => Some("logoff"),
        () = async {
            match shutdown.as_mut() {
                Some(s) => {
                    let _ = s.recv().await;
                }
                None => std::future::pending::<()>().await,
            }
        } => Some("shutdown"),
    }
}
