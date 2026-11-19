// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! `explorer-cli` — binaire de développement et debug.
//!
//! Commandes :
//! - `dump <fichier|url>` : parse le HTML et affiche le DOM en
//!   dump indenté (format html5lib-tests) — jalon J2.
//! - `fetch <url>` : télécharge et affiche le corps décodé.
//! - `layout <fichier|url>` : pipeline complet → wireframe de
//!   l'arbre de boîtes (dump wireframe, étape 17).
//! - `shot <url> <sortie.png>` : pipeline Session complet
//!   (réseau réel, JS, images) → PNG — smoke interop étape 48.

#![forbid(unsafe_code)]

use std::process::ExitCode;
use std::sync::Arc;

mod mcp;

use explorer_html::TreeBuilder;
use explorer_net::decode::DecodingTransport;
use explorer_net::h1::HttpConfig;
use explorer_net::h2::AutoClient;
use explorer_net::http::{HttpTransport, Request};
use explorer_net::policy::RedirectTransport;
use explorer_net::tls::TlsConnector;
use explorer_net::url::Url;

fn main() -> ExitCode {
    // Journal `tracing` actif — binaire de debug : `RUST_LOG`
    // pilote le niveau (défaut `warn`), les `debug!` session
    // (scripts en échec, etc.) deviennent visibles.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .with_writer(std::io::stderr)
        .init();
    let args: Vec<String> = std::env::args().collect();
    match args.as_slice() {
        [_, cmd, arg] if cmd == "dump" => dump(arg),
        [_, cmd, arg] if cmd == "fetch" => fetch(arg),
        [_, cmd, arg] if cmd == "layout" => layout(arg, None),
        [_, cmd, arg, size] if cmd == "layout" => layout(arg, Some(size)),
        [_, cmd, url, out, rest @ ..] if cmd == "shot" => shot(url, out, rest),
        [_, cmd, url] if cmd == "jprobe" => jprobe(url),
        [_, cmd, url] if cmd == "dom" => dom_live(url),
        [_, cmd, url] if cmd == "mem" => mem_report(url),
        [_, cmd, url, dir] if cmd == "save" => save_fixture(url, dir),
        [_, cmd, file] if cmd == "eval" => eval_js(file, false),
        [_, cmd, file] if cmd == "api" => eval_js(file, true),
        [_, cmd] if cmd == "props" => check_names("prop"),
        [_, cmd, kind] if cmd == "check" => check_names(kind),
        [_, cmd, url, out] if cmd == "download" => download(url, out),
        [_, cmd] if cmd == "mcp" => mcp_main(None),
        [_, cmd, port] if cmd == "mcp" => mcp_main(Some(port)),
        [_, cmd] if cmd == "daemons" => daemons_list(),
        [_, cmd, a, b] if cmd == "diff" => diff_pngs(a, b),
        _ => {
            eprintln!("usage : explorer-cli dump <fichier|url>");
            eprintln!("        explorer-cli fetch <url>");
            eprintln!("        explorer-cli layout <fichier|url>");
            eprintln!(
                "        explorer-cli shot <url> <sortie.png> [LxH] [--report out.json] [--geometry out.json]"
            );
            eprintln!("        explorer-cli jprobe <url>");
            eprintln!("        explorer-cli dom <url>");
            eprintln!("        explorer-cli mem <url>");
            eprintln!("        explorer-cli save <url> <dossier>");
            eprintln!("        explorer-cli eval <fichier.js>");
            eprintln!(
                "        explorer-cli api <fichier.js>   (eval + surface navigateur installée)"
            );
            eprintln!(
                "        explorer-cli props            (noms de props sur stdin → `nom\\tok|miss`)"
            );
            eprintln!("        explorer-cli check prop|sel|at|routed|affects|affectsv|paintv|html|decl|tables");
            eprintln!("        explorer-cli download <url> <sortie>");
            eprintln!("        explorer-cli diff <a.png> <b.png>");
            eprintln!(
                "        explorer-cli mcp [port]  (broker MCP stdio → daemons du registre ;\n\
                       \x20                           port = daemon épinglé par défaut)"
            );
            eprintln!("        explorer-cli daemons   (registre des daemons vivants)");
            ExitCode::FAILURE
        }
    }
}

/// Pile réseau complète (TLS + ALPN h1/h2 + décodage +
/// redirections bornées) — partagée par `read_source` et
/// `shot`.
fn client_stack() -> Arc<dyn HttpTransport> {
    // Rejeu d'un vol enregistré (explorer-net `replay`) —
    // priorité absolue : aucun socket ouvert, la session rejoue
    // les réponses du dossier dans l'ordre d'enregistrement.
    if let Ok(dir) = std::env::var("EXPLORER_NET_REPLAY") {
        if !dir.is_empty() {
            eprintln!("[EXPLORER_NET_REPLAY={dir} — rejeu offline]");
            let replay: Arc<dyn HttpTransport> = Arc::new(explorer_net::replay::DirTransport::new(
                std::path::Path::new(&dir),
            ));
            // Étape 313 — RECORD enveloppant REPLAY : distille
            // un vol complet en fixture bornée (seules les
            // réponses réellement servies au shot sont
            // ré-écrites) — corpus versionné `fuzz/replays/`.
            return match std::env::var("EXPLORER_NET_RECORD") {
                Ok(out) if !out.is_empty() => {
                    eprintln!("[EXPLORER_NET_RECORD={out} — distillation du rejeu]");
                    Arc::new(explorer_net::replay::RecordingTransport::wrap(
                        replay,
                        std::path::Path::new(&out),
                    ))
                }
                _ => replay,
            };
        }
    }
    let tls = Arc::new(TlsConnector::web_roots());
    let h1: Arc<dyn HttpTransport> = Arc::new(AutoClient::new(tls, HttpConfig::default()));
    let decoded: Arc<dyn HttpTransport> = Arc::new(DecodingTransport::wrap(h1, 16 * 1024 * 1024));
    let stack: Arc<dyn HttpTransport> = Arc::new(RedirectTransport::wrap(decoded, 8));
    // Enregistrement de vol : chaque réponse décodée est écrite
    // dans le dossier (`<clé>-<seq>.resp`) — rejouable ensuite
    // via `EXPLORER_NET_REPLAY`.
    match std::env::var("EXPLORER_NET_RECORD") {
        Ok(dir) if !dir.is_empty() => {
            eprintln!("[EXPLORER_NET_RECORD={dir} — capture des réponses]");
            Arc::new(explorer_net::replay::RecordingTransport::wrap(
                stack,
                std::path::Path::new(&dir),
            ))
        }
        _ => stack,
    }
}

/// `download <url> <sortie>` — enregistre `url` sous le
/// chemin explicite `sortie` via la pile transport
/// complète (étape 176 : job streamé `.part` → rename,
/// hors verrou — `download_url` n'attend que par events ;
/// affiche le chemin final sur stdout).
#[tokio::main(flavor = "current_thread")]
async fn download(url: &str, out: &str) -> ExitCode {
    let fonts = explorer_render::DirFontSource::new(&default_fonts_dir());
    let mut session = session_from_env(fonts);
    let tab = session.open_tab();
    match session
        .download_url(tab, url, Some(std::path::PathBuf::from(out)), None)
        .await
    {
        Ok(p) => {
            println!("{}", p.display());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("erreur : {e}");
            ExitCode::FAILURE
        }
    }
}

/// Répertoire de fontes — `EXPLORER_FONT_DIR` puis défaut
/// plateforme (même logique que `explorer-daemon`/`explorer-ffi`).
fn default_fonts_dir() -> std::path::PathBuf {
    if let Ok(dir) = std::env::var("EXPLORER_FONT_DIR") {
        return std::path::PathBuf::from(dir);
    }
    if cfg!(windows) {
        std::path::PathBuf::from("C:\\Windows\\Fonts")
    } else if cfg!(target_os = "macos") {
        std::path::PathBuf::from("/System/Library/Fonts")
    } else {
        std::path::PathBuf::from("/usr/share/fonts")
    }
}

/// Session de vol CLI — `SessionConfig` par défaut surchargée
/// de `EXPLORER_JS_MEM_MIB` (étape 123 : borne heap JS en Mio),
/// `EXPLORER_JS_OPS` (étape 124 : budget d'ops par éval —
/// `0` = illimité explicite) et `EXPLORER_VIEWPORT` (étape 129 :
/// `WxH`, ex. `1366x900` — viewport initial) ; outillage
/// diagnostic, tracé à l'usage.
fn session_from_env(fonts: explorer_render::DirFontSource) -> explorer_core::Session {
    let mut cfg = explorer_core::SessionConfig::default();
    if let Some(mib) = std::env::var("EXPLORER_JS_MEM_MIB")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
    {
        eprintln!("[EXPLORER_JS_MEM_MIB={mib} — borne heap JS par contexte]");
        cfg.js_mem_limit_mib = mib;
    }
    if let Some(ops) = std::env::var("EXPLORER_JS_OPS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
    {
        eprintln!("[EXPLORER_JS_OPS={ops} — budget d'ops JS par évaluation]");
        cfg.js_ops = if ops == 0 { u64::MAX } else { ops };
    }
    if let Some((w, h)) = std::env::var("EXPLORER_VIEWPORT")
        .ok()
        .and_then(|v| parse_viewport(&v))
    {
        eprintln!("[EXPLORER_VIEWPORT={w}x{h} — viewport initial]");
        cfg.viewport = (w, h);
    }
    explorer_core::Session::with_config(client_stack(), std::sync::Arc::new(fonts), cfg)
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

/// `shot <url> <sortie.png>` : navigation réelle complète
/// (réseau, JS, images, webfonts) puis PNG du rendu —
/// mesure interop sur sites réels (étape 48).
/// Grâce post-navigation pour les XHR émis au chargement
/// (étape 51 — bornée, puis rendu quoi qu'il arrive).
const XHR_GRACE_MS: u64 = 1_000;

/// Queue d'arguments de `shot` : `[LxH]` positionnel
/// optionnel, puis paires de flags `--report <json>` /
/// `--geometry <json>` (étapes 215/216). Retourne
/// `(size, report, geometry)` — `Err` = usage.
/// Flags optionnels de `shot` (étapes 215/216).
#[derive(Default)]
struct ShotTail<'a> {
    size: Option<&'a str>,
    report: Option<&'a str>,
    geometry: Option<&'a str>,
}

fn shot_tail(rest: &[String]) -> Result<ShotTail<'_>, ()> {
    let mut out = ShotTail::default();
    let mut idx = 0;
    if let Some(first) = rest.first() {
        if !first.starts_with("--") {
            out.size = Some(first.as_str());
            idx = 1;
        }
    }
    while idx < rest.len() {
        match rest[idx].as_str() {
            "--report" => {
                out.report = Some(rest.get(idx + 1).ok_or(())?.as_str());
                idx += 2;
            }
            "--geometry" => {
                out.geometry = Some(rest.get(idx + 1).ok_or(())?.as_str());
                idx += 2;
            }
            _ => return Err(()),
        }
    }
    Ok(out)
}

#[tokio::main(flavor = "current_thread")]
async fn shot(url: &str, out: &str, rest: &[String]) -> ExitCode {
    let Ok(ShotTail {
        size,
        report,
        geometry,
    }) = shot_tail(rest)
    else {
        eprintln!("usage : explorer-cli shot <url> <sortie.png> [LxH] [--report out.json] [--geometry out.json]");
        return ExitCode::FAILURE;
    };
    if geometry.is_some() {
        // Étape 216 — émis post-navigation (l'arbre n'existe
        // qu'après le commit), voir bloc plus bas.
    }
    let fonts = explorer_render::DirFontSource::new(&default_fonts_dir());
    let mut session = session_from_env(fonts);
    session.set_js_factory(explorer_js::JscoreEngine::factory());
    // `--report` (étape 215) : armer le census moteur (R-207)
    // avant la navigation — `reset` au cas où un processus
    // hôte aurait déjà compté (binaire one-shot = table vide
    // par construction, la garde coûte rien).
    if report.is_some() {
        explorer_common::census::reset();
        explorer_common::census::set_armed(true);
    }
    if let Some(s) = size {
        // `LxH` optionnel — triage terrain à la taille de
        // la fenêtre app (défaut session : 1024×768).
        let mut it = s.split('x');
        if let (Some(w), Some(h)) = (it.next(), it.next()) {
            if let (Ok(w), Ok(h)) = (w.parse::<f32>(), h.parse::<f32>()) {
                session.set_viewport(w, h, 1.0);
            }
        }
    }
    let tab = session.open_tab();
    if let Err(e) = session.navigate(tab, url).await {
        eprintln!("erreur : {e}");
        return ExitCode::FAILURE;
    }
    // Étapes 51-52 : les XHR/fetch émis au chargement et les
    // `setTimeout` courts doivent se résoudre avant le shot —
    // grâce bornée (`XHR_GRACE_MS`) puis rendu quoi qu'il
    // arrive. La sieste suit la prochaine échéance timer.
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(XHR_GRACE_MS);
    loop {
        session.pump_net().await;
        session.pump_timers(tab);
        if session.drain_script_nav(tab).await > 0 {
            continue;
        }
        let idle = session.net_pending() == 0 && session.next_timer_deadline().is_none();
        if idle || std::time::Instant::now() >= deadline {
            break;
        }
        let nap = session
            .next_timer_deadline()
            .map_or(std::time::Duration::from_millis(20), |d| {
                d.min(std::time::Duration::from_millis(20))
            });
        tokio::time::sleep(nap).await;
    }
    // ADR-0025 : mesure du pilote — le taux de repli du
    // journal est le verdict fiabilité de la voie
    // différentielle (`warn!` de chaque rejet est déjà
    // visible via RUST_LOG).
    eprintln!(
        "[journal : {} appliqués / {} replis / {} classées]",
        session.journal_applies(),
        session.journal_fallbacks(),
        session.journal_classified()
    );
    emit_shot_reports(&mut session, tab, url, report, geometry);
    if let Some(png) = session.frame_png(tab) {
        match std::fs::write(out, &png) {
            Ok(()) => {
                println!("{out} — {} octets", png.len());
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("écriture {out} : {e}");
                ExitCode::FAILURE
            }
        }
    } else {
        eprintln!("pas de frame produite");
        ExitCode::FAILURE
    }
}

/// Émission `--report`/`--geometry` post-navigation (étapes
/// 215/216). `--report` (contrat consommé par `site_harvest`) :
/// census moteur verbatim + `doc_h` mesurée — émis même si la
/// frame échoue, un shot sans PNG reste un signal pour le
/// Pareto. `--geometry` (R-208 consommé) : boîtes `NodeId→rect`
/// jointes `(tag,id,cls)`, même forme que `ref_capture`
/// `.ref.json` — consommée par `geo_diff.mjs`.
fn emit_shot_reports(
    session: &mut explorer_core::Session,
    tab: explorer_core::TabId,
    url: &str,
    report: Option<&str>,
    geometry: Option<&str>,
) {
    if let Some(rep) = report {
        let mut doc_h = serde_json::Value::Null;
        let mut doc_h_clamped = serde_json::Value::Null;
        if let Some(info) = session.tab_info(tab) {
            doc_h = serde_json::json!(info.doc_h);
            doc_h_clamped = serde_json::json!(info.doc_h_clamped);
        }
        let payload = serde_json::json!({
            "url": url,
            "final_url": session.tab_url(tab).unwrap_or_default(),
            "doc_h": doc_h,
            "doc_h_clamped": doc_h_clamped,
            "census": explorer_common::census::snapshot(),
        });
        match serde_json::to_string_pretty(&payload).map(|j| std::fs::write(rep, j)) {
            Ok(Ok(())) => {}
            _ => eprintln!("--report : écriture {rep} impossible"),
        }
    }
    if let Some(gpath) = geometry {
        match session.geometry_report(tab) {
            Some(g) => match serde_json::to_string_pretty(&g).map(|j| std::fs::write(gpath, j)) {
                Ok(Ok(())) => {}
                _ => eprintln!("--geometry : écriture {gpath} impossible"),
            },
            None => eprintln!("--geometry : pas d'arbre posé"),
        }
    }
}

/// `jprobe <url>` : sonde de vol terrain — navigation
/// réelle puis **interactions synthétiques** (scrolls,
/// balayage `mousemove` en grille, clics) pour déclencher
/// les points de synchro DOM. Rapporte :
/// - `journal` : fiabilité de la voie différentielle
///   ADR-0025 (appliqués / replis / classées) ;
/// - `editable` : hits `Hover.editable` du balayage
///   (mesure `form:absent` — un champ réel et peint doit
///   être désigné) ;
/// - `media` : offres `MediaSurface` (`media_list`) +
///   claim d'essai sur la première (cycle 71e en
///   conditions réelles — la fixture couvre la mécanique,
///   pas le terrain).
/// `dom <url>` : navigation réelle complète (JS + redirections
/// + hydratation bornée) puis dump du DOM **vivant** — outil
/// de triage : le mur de consentement Google, par exemple,
/// n'existe que post-script (étape 120).
#[tokio::main(flavor = "current_thread")]
async fn dom_live(url: &str) -> ExitCode {
    let fonts = explorer_render::DirFontSource::new(&default_fonts_dir());
    let mut session = session_from_env(fonts);
    session.set_js_factory(explorer_js::JscoreEngine::factory());
    let tab = session.open_tab();
    if let Err(e) = session.navigate(tab, url).await {
        eprintln!("erreur : {e}");
        return ExitCode::FAILURE;
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(1_500);
    while std::time::Instant::now() < deadline {
        session.pump_net().await;
        session.pump_timers(tab);
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    eprintln!("[url : {}]", session.tab_url(tab).unwrap_or_default());
    if let Some(d) = session.dom_dump(tab) {
        print!("{d}");
        ExitCode::SUCCESS
    } else {
        eprintln!("pas de DOM (détaché ?)");
        ExitCode::FAILURE
    }
}

/// `mcp [port]` — broker MCP stdio ↔ EAP : chaque outil est
/// routé vers un daemon du registre (étape 256) puis traduit
/// en verbe EAP sur `/agent`. `port` épingle le daemon par
/// défaut ; absent = broker pur (`daemon`/`nom:onglet` à
/// chaque appel, ou daemon unique).
fn mcp_main(port: Option<&str>) -> ExitCode {
    let mut p = None;
    if let Some(s) = port {
        if let Ok(v) = s.parse::<u16>() {
            p = Some(v);
        } else {
            eprintln!("mcp : port invalide '{s}'");
            return ExitCode::FAILURE;
        }
    }
    match mcp::serve(p) {
        Ok(_) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("mcp : {e}");
            ExitCode::FAILURE
        }
    }
}

/// `daemons` — liste le registre des daemons vivants
/// (étape 256) : l'œil humain du broker, même source.
fn daemons_list() -> ExitCode {
    use std::io::Write;
    let entries = explorer_core::daemon_registry::list();
    if entries.is_empty() {
        println!(
            "aucun daemon annoncé ({})",
            explorer_core::daemon_registry::registry_dir().display()
        );
        return ExitCode::SUCCESS;
    }
    // Une seule passe `netstat` : la vivacité est
    // (pid, image, port lié) — un pid réattribué n'emporte
    // pas l'annonce du mort (même règle que le broker :
    // jamais d'éviction sur sonde — cf. Étape 256b).
    let ports = explorer_core::daemon_registry::listening_ports();
    for (file, a) in &entries {
        if explorer_core::daemon_registry::announce_stale_with(a, ports.as_ref()) {
            explorer_core::daemon_registry::remove(file);
            continue;
        }
        let _ = writeln!(
            std::io::stdout(),
            "{:<16} port={:<6} pid={:<7} v{}  {}",
            a.name,
            a.port,
            a.pid,
            a.version,
            a.profile.display()
        );
    }
    ExitCode::SUCCESS
}

/// `diff <a.png> <b.png>` — diff pixel entre deux rendus :
/// % de pixels différents, boîte englobante des deltas,
/// grille 4×4 des zones chaudes. Exit 0 = identiques,
/// 2 = diffèrent (scriptable), 1 = erreur.
fn diff_pngs(file_a: &str, file_b: &str) -> ExitCode {
    let read = |path: &str| -> Option<explorer_render::image::DecodedImage> {
        let bytes = std::fs::read(path).ok()?;
        explorer_render::image::decode_image(&bytes, 200_000_000)
    };
    let (Some(ia), Some(ib)) = (read(file_a), read(file_b)) else {
        eprintln!("diff : PNG indécodable ou absent ({file_a} / {file_b})");
        return ExitCode::FAILURE;
    };
    if ia.width != ib.width || ia.height != ib.height {
        println!(
            "diff : dimensions differentes {}x{} vs {}x{} — 100%",
            ia.width, ia.height, ib.width, ib.height
        );
        return ExitCode::from(2);
    }
    let wpx = ia.width as usize;
    let hpx = ia.height as usize;
    let (mut ndiff, mut x0, mut y0, mut x1, mut y1) = (0usize, wpx, hpx, 0usize, 0usize);
    let mut grid = [0u32; 16];
    for row in 0..hpx {
        for col in 0..wpx {
            let idx = (row * wpx + col) * 4;
            if (0..3).any(|chan| ia.rgba[idx + chan] != ib.rgba[idx + chan]) {
                ndiff += 1;
                x0 = x0.min(col);
                y0 = y0.min(row);
                x1 = x1.max(col);
                y1 = y1.max(row);
                grid[(row * 4 / hpx).min(3) * 4 + (col * 4 / wpx).min(3)] += 1;
            }
        }
    }
    let total = wpx * hpx;
    if ndiff == 0 {
        println!("diff : identiques ({total} px)");
        return ExitCode::SUCCESS;
    }
    // Pourcentage entier ×100 pour éviter les casts f64 —
    // suffisant pour un diff pixel (clippy pedantic).
    println!(
        "diff : {}.{}% ({ndiff} / {total} px) — boite [{x0},{y0} → {x1},{y1}]",
        ndiff * 10_000 / total / 100,
        (ndiff * 10_000 / total) % 100
    );
    print!("       grille4x4 :");
    for cell in grid {
        print!(" {cell}");
    }
    println!();
    ExitCode::from(2)
}

/// `save <url> <dossier>` : nav réelle complète puis fixture
/// terrain — frame.png, dom.txt, layout.txt, manifest.txt
/// (url+date) pour repro offline et bissection.
#[tokio::main(flavor = "current_thread")]
async fn save_fixture(url: &str, dir: &str) -> ExitCode {
    let fonts = explorer_render::DirFontSource::new(&default_fonts_dir());
    let mut session = session_from_env(fonts);
    session.set_js_factory(explorer_js::JscoreEngine::factory());
    let tab = session.open_tab();
    if let Err(e) = session.navigate(tab, url).await {
        eprintln!("erreur : {e}");
        return ExitCode::FAILURE;
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(XHR_GRACE_MS);
    while std::time::Instant::now() < deadline {
        session.pump_net().await;
        session.pump_timers(tab);
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    if std::fs::create_dir_all(dir).is_err() {
        eprintln!("save : impossible de créer {dir}");
        return ExitCode::FAILURE;
    }
    let put = |name: &str, bytes: &[u8]| {
        let p = format!("{dir}/{name}");
        std::fs::write(&p, bytes).map(|()| p)
    };
    let mut wrote = Vec::new();
    if let Some(png) = session.frame_png(tab) {
        if let Ok(p) = put("frame.png", &png) {
            wrote.push(p);
        }
    }
    if let Some(d) = session.dom_dump(tab) {
        if let Ok(p) = put("dom.txt", d.as_bytes()) {
            wrote.push(p);
        }
    }
    if let Some(l) = session.layout_dump(tab) {
        if let Ok(p) = put("layout.txt", l.as_bytes()) {
            wrote.push(p);
        }
    }
    let url_now = session.tab_url(tab).unwrap_or_default();
    let manifest = format!(
        "url     : {url}\nurl_fin : {url_now}\nversion : {}\n",
        env!("CARGO_PKG_VERSION")
    );
    let _ = put("manifest.txt", manifest.as_bytes());
    if wrote.is_empty() {
        eprintln!("save : rien à écrire (page vide ?)");
        return ExitCode::FAILURE;
    }
    println!("save : {} artefacts dans {dir}", wrote.len());
    ExitCode::SUCCESS
}

/// `eval <fichier.js>` : exécute un script sous `explorer-jscore`
/// — `console.*` vers stdout, throws et rejets drainés vers
/// stderr. Brique du différentiel `scripts/js_diff.mjs`
/// (confrontation à Node/V8 sur corpus). Bornes optionnelles :
/// `EXPLORER_JS_OPS` (ops) — le défaut suit le moteur.
/// `api <fichier.js>` : identique à `eval` mais avec la surface
/// navigateur installée (`install_dom` — `document`,
/// `navigator`, `fetch`, `MutationObserver`, etc.). Sondage de
/// couverture `bcd.api.*` : les globals session sont testés en
/// contexte réel, pas dans le moteur nu.
fn eval_js(path: &str, with_dom: bool) -> ExitCode {
    let src = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("eval : lecture impossible ({path}) : {e}");
            return ExitCode::FAILURE;
        }
    };
    let mut e = explorer_jscore::Engine::new();
    if with_dom {
        let dom = std::rc::Rc::new(std::cell::RefCell::new(explorer_jscore::Dom::from_html(
            "<!doctype html><body></body>",
        )));
        e.install_dom(&dom);
        // Surface « session réelle » (JS-C560) — les globals
        // que les embedders posent hors `install_dom` :
        // timers (`setTimeout`/`requestAnimationFrame`…),
        // `localStorage`/`sessionStorage` (backing neuf
        // page-locale) et `XMLHttpRequest`/`fetch`/`Headers`/
        // `Request`/`Response` sur transport mort — requêtes
        // pendantes, jamais de réponse synthétisée.
        e.install_timers();
        e.install_storage(&explorer_jscore::Storage::default());
        e.install_xhr(std::rc::Rc::new(explorer_jscore::NullTransport));
    }
    if let Some(ops) = std::env::var("EXPLORER_JS_OPS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
    {
        e.set_op_limit(if ops == 0 { u64::MAX } else { ops });
    }
    e.set_console_hook(std::rc::Rc::new(|l| println!("{l}")));
    let r = e.eval_named(&src, path);
    let _ = e.run_microtasks();
    if let Err(err) = &r {
        eprintln!("uncaught: {err}");
    }
    for m in e.drain_unhandled_rejections() {
        eprintln!("{m}");
    }
    if r.is_err() {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

/// `props` : lit des noms de propriétés CSS sur stdin (un par
/// ligne) et émet `nom\tok` ou `nom\tmiss` selon
/// [`explorer_css::prop_supported`] — critère `@supports`
/// (nom seul). Brique de `scripts/bcd_coverage.mjs` : le
/// script alimente les ~650 clés BCD et compte la couverture.
/// `check <prop|sel|at>` (`props` = `check prop`) : lit des
/// noms sur stdin (un par ligne) et émet `nom\tok` ou
/// `nom\tmiss` selon le critère de reconnaissance du moteur —
/// `prop_supported` / `selector_supported` /
/// `at_rule_supported`. Brique de `scripts/bcd_coverage.mjs` :
/// le script alimente les clés BCD et compte la couverture.
/// R-246 — la prop change-t-elle la **geometrie/kind** de
/// `.x` ? `x{name:v}` vs `x{}` — vrai pour `margin`/`all`/
/// `block-size`, faux pour les noms stockés indifféremment
/// (`xyz-bogus`, `mask-*` dormants). Valeurs de sonde par
/// famille (longueur/couleur/mot-clé layout/keyword).
fn prop_affects(name: &str) -> bool {
    let dom = explorer_html::TreeBuilder::parse("<body><div><p class=x>ok</p></div></body>");
    let root = {
        let mut r = None;
        walk_any(dom.document(), &dom, &mut r);
        r.unwrap_or(dom.document())
    };
    let sig = |css: &str| -> String {
        let mut eng = explorer_css::StyleEngine::new();
        eng.add_css(css, explorer_css::Origin::Author);
        let sm = eng.compute_tree(&dom, root, &explorer_css::StyleEnv::default());
        match explorer_layout::build_box_tree(&dom, root, &sm) {
            Some(mut bx) => {
                explorer_layout::layout_tree(&mut bx, 800.0, &explorer_layout::BlockCtx::default());
                bx.dump(&dom)
            }
            None => String::new(),
        }
    };
    let base = sig("p {}");
    for v in [
        "10px",
        "red",
        "block",
        "flex",
        "none",
        "nowrap",
        "column",
        "bold",
        "italic",
        "absolute",
        "inherit",
        "unset",
        "initial",
        // R-246 — axes d'écriture : `writing-mode`/`direction`/
        // `text-orientation`/`unicode-bidi`/`dir`/`bdo`.
        "vertical-rl",
        "vertical-lr",
        "sideways-rl",
        "rtl",
        "ltr",
        "mixed",
    ] {
        if sig(&format!("p {{ {name}: {v} }}")) != base {
            return true;
        }
    }
    // Contexte flex — les props flex-*/grid-*/gap n'ont
    // d'effet que dans un conteneur dédié.
    let base_f = sig("div { display: flex } p {}");
    for v in [
        "row-reverse",
        "wrap",
        "10px",
        "flex-start",
        "column",
        "stretch",
    ] {
        if sig(&format!("div {{ display: flex }} p {{ {name}: {v} }}")) != base_f {
            return true;
        }
    }
    false
}

/// `affectsv` : entrée `name:value` — la déclaration **exacte**
/// `p { name:value }` change-t-elle le box tree vs `p {}` ?
/// Alimente `bcd.css.values` (conformité par feature, pas par
/// nom — `display:ruby` vs `display:grid`).
/// Fixture par famille (R-247 — le `<p>` nu rendait 91 % des
/// valeurs inertes par construction) : chaque prop est
/// déclarée sur l'élément `.x` dans le contexte où sa valeur
/// *peut* produire de la géométrie. `.x` = l'élément porteur
/// de la déclaration (conteneur ou item selon la famille).
#[allow(clippy::too_many_lines)]
fn fixture_for(name: &str) -> &'static str {
    let n = name.trim_start_matches('-').trim_start_matches("webkit-");
    match () {
        // Props de conteneur flex **mono-ligne** (étape 270) —
        // `justify-content`/`column-gap`/`flex-direction`/
        // `flex-wrap`/`flex-flow` n'ont d'oracle qu'avec des
        // items sur une même ligne : le wrap et `width:100%`
        // de l'arme multi-lignes les rendaient inertes.
        () if matches!(
            n,
            "justify-content" | "column-gap" | "flex-direction"
        ) =>
        {
            "<body><div class=x style=\"display:flex;width:400px\"><p>a</p><p>b</p><p>c</p></div></body>"
        }
        // `flex-wrap`/`flex-flow` — le wrap n'a d'oracle
        // qu'avec un item qui DÉBORDE : `a` à 100 % force
        // `b` sur la ligne 2 (étape 297 — la fixture
        // mono-ligne ci-dessus rendait toute la famille
        // inerte, 0/3). Pas de `flex-wrap` en `style=` ni
        // en ctx : la base `nowrap` est l'initiale — `wrap`
        // et `wrap-reverse` divergent, `nowrap` est `na`
        // par `INITIAL_EXTRA`.
        () if n == "flex-wrap" || n == "flex-flow" => {
            "<body><div class=x style=\"display:flex;height:200px\"><p style=\"width:100%\">a</p><p>b</p></div></body>"
        }
        // Props de conteneur flex/grid — `.x` = le conteneur.
        () if matches!(
            n,
            "align-content" | "align-items" | "place-content" | "place-items"
                | "gap" | "row-gap"
        ) =>
        {
            // Multi-lignes + hauteur fixe : `align-content` a
            // de l'espace libre à distribuer (conteneur auto
            // → effet nul par construction).
            "<body><div class=x style=\"display:flex;flex-wrap:wrap;height:200px\"><p style=\"width:100%\">a</p><p>b</p></div></body>"
        }
        // Conteneur **grid** (étape 270) — les props
        // `grid-template-*`/`grid-auto-*` étaient sondées
        // sous `display:flex` → inertes par construction.
        // `.x` = le conteneur grid + items pour le placement.
        () if matches!(
            n,
            "grid-template-columns" | "grid-template-rows" | "grid-auto-flow"
                | "grid-auto-columns" | "grid-auto-rows" | "grid-template"
                | "grid-template-areas" | "grid"
        ) =>
        {
            "<body><div class=x style=\"display:grid;width:200px\"><p>a</p><p>b</p><p>c</p><p>d</p></div></body>"
        }
        // Items **grid** — `grid-column*`/`grid-row*`/`grid-area`
        // étaient sondés sous `display:flex` → inertes.
        () if matches!(
            n,
            "grid-column" | "grid-column-start" | "grid-column-end" | "grid-row"
                | "grid-row-start" | "grid-row-end" | "grid-area"
        ) =>
        {
            "<body><div style=\"display:grid;grid-template-columns:60px 60px 60px;grid-template-rows:40px 40px;width:200px\"><p class=x>a</p><p>b</p><p>c</p></div></body>"
        }
        // `display` (étape 270) — `.x` inline (`<span>`) :
        // `block`/`grid`/`flow-root`/`table-*` convertissent
        // → delta de boîte ; sur un élément déjà `block`
        // ils étaient inertes.
        () if n == "display" => {
            "<body><div><span class=x>ok</span><span>fin</span></div></body>"
        }
        // `z-index` (étape 270) — l'ordre d'empilement n'a
        // d'oracle que sur des boîtes **positionnées**
        // superposées : `.x` chevauche le voisin rouge.
        () if n == "z-index" => {
            "<body><div style=\"position:relative;width:100px;height:60px;background:red\"></div><div class=x style=\"position:relative;margin-top:-30px;width:60px;height:60px;background:blue\"></div></body>"
        }
        // `column-span` (étape 270) — s'applique à l'enfant
        // du conteneur multicol, pas au conteneur : `.x` =
        // le spanner candidat.
        () if n == "column-span" => {
            "<body><div style=\"columns:2;column-gap:10px;width:300px\"><p>a<br>a<br>a</p><h2 class=x style=\"margin:0\">s</h2><p>b<br>b<br>b</p></div></body>"
        }
        // `row-rule*`/`rule-overlap` (étape 270) — les
        // décorations de **rangée** (css-gaps-1) n'existent
        // que sur un conteneur à rangées : grid deux rangées
        // + `row-gap` (multicol n'a que des gaps de colonne).
        () if n.starts_with("row-rule") || n == "rule-overlap" => {
            "<body><div class=x style=\"display:grid;grid-template-rows:50px 50px;row-gap:20px;width:100px\"><p>a</p><p>b</p></div></body>"
        }
        // `unicode-bidi`/`direction` (étape 270) — la
        // réordonnance n'a d'oracle qu'avec du texte
        // bidirectionnel (hébreu incrusté dans du latin).
        () if n == "unicode-bidi" || n == "direction" => {
            "<body><div class=x><p>abc \u{05d0}\u{05d1}\u{05d2} def</p></div></body>"
        }
        // `justify-self` est **ignoré** dans un conteneur
        // flex (css-align §8.3) — conteneur grid requis pour
        // mesurer le vrai support.
        () if n == "justify-self" => {
            "<body><div style=\"display:grid;width:400px\"><p class=x>a</p><p>b</p></div></body>"
        }
        // `justify-items`/`place-items` — ignorés en flex
        // (css-align §8.3) ; conteneur grid requis.
        () if n == "justify-items" => {
            "<body><div class=x style=\"display:grid;width:400px\"><p>a</p><p>b</p></div></body>"
        }
        // `place-self` — item grid (les deux axes self).
        () if n == "place-self" => {
            "<body><div style=\"display:grid;width:400px;height:200px\"><p class=x>a</p></div></body>"
        }
        // `flex` shorthand — item dans conteneur flex
        // (`flex:1` grandit l'item → delta de largeur).
        () if n == "flex" => {
            "<body><div style=\"display:flex;width:400px\"><p class=x>a</p><p>b</p></div></body>"
        }
        // `list-style*` — contexte `<li>` requis (marqueur
        // émis que sur un item de liste, §marker).
        () if n.starts_with("list-style") || n == "list-style-type" => {
            "<body><ul><li class=x>a</li><li>b</li></ul></body>"
        }
        // Props de **table** — `.x` = la table (caption
        // présente pour `caption-side`, cellule vide pour
        // `empty-cells`, bordures pour `border-collapse`).
        () if matches!(n, "empty-cells" | "caption-side" | "border-collapse")
        =>
        {
            "<body><table class=x><caption>c</caption><tr><td style=\"border:1px solid black\"> </td><td style=\"border:1px solid black;width:100px;height:40px\">a</td></tr></table></body>"
        }
        // `table-layout` (R-275, artefact oracle) : `fixed`
        // n'est discriminant que si l'**auto** élargirait la
        // colonne au contenu — une cellule à texte large
        // sans `width` (auto = contenu, fixed = largeurs du
        // premier rang uniquement §17.5.2.1).
        () if n == "table-layout" => {
            "<body><table class=x><tr><td>a</td><td>bbbbbbbbbbbbbbbbbbbbbbbbbbbb</td></tr></table></body>"
        }
        // `break-*`/`page-break-*` — s'appliquent à
        // l'**unité enfant** du conteneur multicol, pas au
        // conteneur : `.x` = 2ᵉ unité, `break-before:column`
        // la pousse en colonne 2 (l'équilibre naturel
        // l'y mettrait aussi → ctx 3 unités, `.x` au milieu
        // serait ambigu — 2 unités + hauteur forçant
        // l'équilibre en col 1 pour `avoid`).
        () if n.starts_with("break-") || n.starts_with("page-break") => {
            "<body><div style=\"columns:2;column-gap:10px;width:210px;height:60px;column-fill:auto\"><p style=\"margin:0\">a</p><p class=x style=\"margin:0\">b</p><p style=\"margin:0\">c</p></div></body>"
        }
        // `vertical-align`/`baseline-shift` — `.x` = cellule
        // table (cellule soeur plus haute → `top`/`bottom`/
        // `middle` déplacent le contenu).
        () if matches!(n, "vertical-align" | "baseline-shift") => {
            "<body><table><tr><td class=x>a</td><td style=\"height:60px\">b</td></tr></table></body>"
        }
        // `alignment-baseline`/`dominant-baseline` — prop
        // de format **inline** : `.x` = atome inline-block
        // dans une ligne de 40 px (bords de ligne
        // observables).
        () if matches!(n, "alignment-baseline" | "dominant-baseline") => {
            "<body><p style=\"font-size:40px\">x<span class=x style=\"display:inline-block;width:10px;height:10px\"></span></p></body>"
        }
        // `quotes` — n'a d'effet que sur le contenu généré
        // `<q>` (paires de guillemets par langue).
        () if n == "quotes" => {
            "<body><p class=x><q>a<q>b</q></q></p></body>"
        }
        // `initial-letter` — ne s'applique qu'au pseudo
        // `::first-letter` (la sonde cible `.x::first-letter`,
        // voir `prop_affects_decl`) et exige ≥3 lignes pour
        // que la morsure de la lettrine soit observable.
        () if n == "initial-letter" => {
            "<body><p class=x style=\"width:120px;font-size:8px\">alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu nu xi omicron pi rho</p></body>"
        }
        // Coupure de ligne — `overflow-wrap`/`word-break`/
        // `line-break`/`hyphens`/`tab-size` n'ont d'effet
        // qu'avec un mot plus long que la ligne (conteneur
        // étroit + mot long/tabulation).
        () if matches!(
            n,
            "overflow-wrap" | "line-break" | "tab-size"
                | "overflow" | "overflow-x" | "overflow-y"
        ) =>
        {
            "<body><div class=x style=\"width:40px\"><p>averyveryveryverylongword\tb</p></div></body>"
        }
        // `word-break` (R-281) : `keep-all` n'a d'effet que
        // sur des lettres CJK — le run latin sert `break-all`,
        // le run  sert `keep-all`/`normal`.
        () if n == "word-break" => {
            "<body><div class=x style=\"width:40px\"><p>averyveryveryverylongword のテキストです</p></div></body>"
        }
        // `text-autospace` (R-282) — n'a d'effet qu'à une
        // frontière CJK ↔ non-CJK adjacente.
        () if n == "text-autospace" => {
            "<body><div style=\"width:200px\"><p class=x>中文abc 中文def</p></div></body>"
        }
        // `text-justify`/`text-wrap-style`/`text-wrap` (R-282,
        // étape 272) — plusieurs lignes de longueurs inégales :
        // `justify` redistribue l'espace, `balance`/`pretty`
        // rééquilibrent la fin (la fixture mot-unique laissait
        // le shorthand `text-wrap` sans rien à rééquilibrer).
        () if n == "text-justify" || n == "text-wrap-style" || n == "text-wrap" => {
            "<body><div><p class=x style=\"width:200px\">un deux trois quatre cinq six sept huit neuf dix</p></div></body>"
        }
        // `text-decoration-skip*` (R-282) — la coupure de la
        // deco ne se voit que sur un blanc **interne** à la
        // ligne (les blancs de bord tombent à la cassure) :
        // inline souligné avec espaces intérieurs, ligne large.
        () if n.starts_with("text-decoration-skip") => {
            "<body><p>zéro <u class=x> un deux trois quatre </u> cinq six</p></body>"
        }
        // `hyphens`/`hyphenate-*` (R-275) : la césure n'a
        // d'oracle que s'il y a une **opportunité** — le mot
        // porte un `&shy;` (manuel) ET assez de lettres pour
        // les points `auto` ; `none` supprime le shy →
        // discriminant vs la baseline `manual`.
        () if n == "hyphens" || n.starts_with("hyphenate") => {
            "<body><div style=\"width:40px\"><p class=x style=\"font-size:8px\">aver&shy;yverylong wordlong</p></div></body>"
        }
        // `ruby-*` — contexte `<ruby>` requis (rb/rt
        // n'existent que sous ruby).
        // `ruby-overhang` — annotation **plus large** que la
        // base requise : sans débordement possible `none` et
        // `auto` coïncident (neutre de contexte).
        () if n == "ruby-overhang" => {
            "<body><p>av <ruby>b<rt class=x>xxxxxxxxxxxx</rt></ruby> ap</p></body>"
        }
        () if n.starts_with("ruby") => {
            "<body><ruby class=x>base<rt class=x>x</rt></ruby>"
        }
        // Props de transformée — n'ont d'effet que sur une
        // boîte **transformée** (`transform-origin` déplace
        // le pivot, `backface`/`perspective`/`transform-box`
        // `transform-style` le référentiel). Insets
        // **asymétriques** (étape 272) : `transform-box`
        // change le centre de référence — uniformes,
        // `border-box`/`content-box` partagent le même
        // centre et tout repli est invisible.
        () if matches!(
            n,
            "transform-origin" | "transform-box" | "perspective"
                | "backface-visibility" | "transform-style"
        ) =>
        {
            "<body><div class=x style=\"width:100px;height:80px;background:blue;transform:rotate(30deg);border:10px solid black;border-right-width:0;padding:5px;padding-left:20px\"><p>a</p></div></body>"
        }
        // Props d'item dans un conteneur flex — `.x` = l'item.
        () if matches!(
            n,
            "align-self" | "order" | "flex-grow" | "flex-shrink" | "flex-basis"
                | "grid-column" | "grid-row" | "grid-column-start"
                | "grid-column-end" | "grid-row-start" | "grid-row-end" | "grid-area"
        ) =>
        {
            // 2 items de hauteurs variables : sans ça
            // `align-self` = stretch (line_cross) → offset 0
            // quel que soit le mot-clé (artefact mesuré).
            "<body><div style=\"display:flex;height:200px\"><p class=x>a</p><p style=\"height:150px\">b</p></div></body>"
        }
        // Ancrage css-anchor-1 — `.x` = l'élément positionné
        // contre l'ancre `#a` (`position-area`, `position-try*`,
        // `anchor()`/`anchor-size()` en insets,
        // `position-visibility`, `anchor-scope`). Sans ancre
        // référencée toute la famille est inerte par
        // construction.
        // `position-try*` — fallbacks évalués **uniquement
        // en débordement** (css-anchor-1 §3.5) : l'ancre est
        // plaquée en haut à gauche et l'item trop grand
        // (`position-area:top` → déborde en y, 900 px → en x)
        // pour que `flip-block`/`flip-inline`/`flip-start`
        // s'activent. Sans débordement toute la famille est
        // inerte.
        // `position-try-order` — l'ordre ne se voit que si
        // AUCUN candidat ne rentre (la dernière essayée est
        // appliquée, css-anchor-1 §3.5) : ancre en haut à
        // droite — `bottom` déborde en x, `left` déborde en
        // y ; `most-width` trie `left` en tête → le repli
        // final diffère de l'ordre `normal`.
        () if n == "position-try-order" => {
            "<body><div id=a style=\"anchor-name:--a;position:absolute;top:5px;left:750px;width:20px;height:20px\"></div><p class=x style=\"position:absolute;position-anchor:--a;position-area:top;width:200px;height:100px\">b</p></body>"
        }
        // `anchor()`/`anchor-size()` en inset/margin/size
        // (étape 270) — les sous-features `anchor`/
        // `anchor-size` sondent ces fonctions : `.x` est
        // positionné contre l'ancre `#a`. Les valeurs
        // ordinaires (`top:10px`, `width:200px`…) restent
        // discriminables dans ce contexte.
        // (inset/margin uniquement : sur les props de sizing,
        // le contexte positionné rendrait `width:auto`
        // shrink-to-fit et tuerait l'oracle intrinsèque —
        // mesuré étape 270.)
        () if matches!(
            n,
            "top" | "right" | "bottom" | "left" | "inset" | "inset-block"
                | "inset-block-end" | "inset-block-start" | "inset-inline"
                | "inset-inline-end" | "inset-inline-start" | "margin"
                | "margin-top" | "margin-right" | "margin-bottom" | "margin-left"
                | "margin-block" | "margin-block-start" | "margin-block-end"
                | "margin-inline" | "margin-inline-start" | "margin-inline-end"
        ) =>
        {
            "<body><div id=a style=\"anchor-name:--a;position:absolute;top:10px;left:10px;width:20px;height:20px\"></div><p class=x style=\"position:absolute;position-anchor:--a\">b</p></body>"
        }
        () if matches!(n, "position-try" | "position-try-fallbacks") => {
            // Étape 272 — `.x` ramené à 300 px : à 900 px
            // aucun candidat ne rentrait dans le viewport,
            // `flip-inline`/`flip-x` restaient inapplicables
            // (le moteur n'applique qu'un candidat qui
            // rentre) — faux misses sur valeurs supportées.
            "<body><div id=a style=\"anchor-name:--a;position:absolute;top:5px;left:5px;width:20px;height:20px\"></div><p class=x style=\"position:absolute;position-anchor:--a;position-area:top;width:300px;height:100px\">b</p></body>"
        }
        () if matches!(n, "position-area" | "position-visibility" | "anchor-scope") => {
            "<body><div id=a style=\"anchor-name:--a;position:absolute;top:10px;left:10px;width:20px;height:20px\"></div><p class=x style=\"position:absolute;position-anchor:--a\">b</p></body>"
        }
        // `anchor-name` — `.x` = l'**ancre** elle-même ; un
        // consommateur `position-anchor:--a` existe pour que
        // le nom résolve (ancre non référencée = effet nul).
        () if n == "anchor-name" => {
            "<body><div class=x style=\"position:absolute;top:10px;left:10px;width:20px;height:20px\"></div><p style=\"position:absolute;position-anchor:--a\">b</p></body>"
        }
        // Bordures — `border*[-color|-style|-width]` n'ont
        // d'effet que sur une boîte **déjà bordée** (une
        // couleur sans `border-style` ne peint rien). La
        // baseline porte `2px solid black` : chaque variante
        // (couleur, style, largeur, côté logique, rayon,
        // `none` = suppression) produit un delta de dump.
        () if n.starts_with("border") => {
            // Le contexte (bordure existante) passe par
            // `context_css` — pas de `style=` inline (il
            // écraserait la déclaration testée).
            "<body><div class=x><p>a</p></div></body>"
        }
        // Contours — même classe : `outline*` observable si
        // une outline existe (`display.rs` peint l'item).
        () if n.starts_with("outline") => {
            "<body><div class=x><p>a</p></div></body>"
        }
        // `clear` — n'a d'effet qu'avec un flottant amont :
        // `.x` se cale sous les flottants (gauche ET droit
        // pour que `clear:right` discrimine, étape 270).
        () if n == "clear" => {
            "<body><div style=\"float:left;width:50px;height:50px\">a</div><div style=\"float:right;width:50px;height:60px\">b</div><p class=x>x</p><p>tail</p></body>"
        }
        // `float` — le flottant `.x` doit avoir du contenu
        // qui le contourne pour que `left`/`right`/
        // `inline-*` se mesurent.
        () if n == "float" => {
            "<body><div><p class=x>a</p><p>un deux trois quatre cinq six sept huit neuf dix</p></div></body>"
        }
        // Formulaires — `accent-color`, `caret-color`, `appearance`,
        // `cursor`, `field-sizing`… n'ont d'effet que sur contrôle.
        // `field-sizing:content` dimensionne le contrôle à
        // son contenu — un contrôle textuel (pas un
        // checkbox sans texte).
        () if n == "field-sizing" => {
            "<body><div><textarea class=x>plusieurs lignes de texte\net encore une</textarea></div></body>"
        }
        () if matches!(
            n,
            "accent-color" | "caret-color" | "appearance"
                | "user-select" | "pointer-events"
        ) =>
        {
            "<body><div><input class=x type=checkbox checked></div></body>"
        }
        // Listes — `list-style-*`, `counter-*`.
        () if n.starts_with("list-style") || n.starts_with("counter") => {
            "<body><ol><li class=x>a</li><li>b</li></ol></body>"
        }
        // Multi-colonnes — le conteneur porte `columns:2` +
        // une `column-rule` : `column-rule-*`, `break-*`,
        // `column-fill`, `column-span` produisent un delta
        // (sans colonnes posées la famille est inerte).
        () if n.starts_with("column-") || n.starts_with("break-") || n == "columns" => {
            "<body><div class=x><p>a</p><p>b</p><p>c</p><p>d</p></div></body>"
        }
        // `rule`/`rule-*`/`row-rule*` (css-gaps-1 — R-271) :
        // décorations de gap — `.x` = le conteneur multicol ;
        // un spanner `column-span:all` au milieu coupe le
        // trait en segments (défaut §5.1) pour que
        // `rule-break:none` soit discriminant.
        () if n == "rule" || n.starts_with("rule-") || n.starts_with("row-rule") => {
            "<body><div class=x><p>a<br>a<br>a<br>a</p><h2 style=\"column-span:all;margin:0\">s</h2><p>b<br>b<br>b<br>b</p></div></body>"
        }
        // `offset-*` (css-motion-1 — R-272) : le chemin se
        // trace dans le **bloc conteneur** — `.x` = enfant
        // `position:absolute` d'un parent `relative` dimensionné
        // (200×200), sinon aucun cb mesurable.
        () if n == "offset" || n.starts_with("offset-") => {
            "<body><div style=\"position:relative;width:200px;height:200px\"><div class=x style=\"position:absolute;width:10px;height:10px\"></div></div></body>"
        }
        // `shape-outside`/`shape-margin`/`shape-image-
        // threshold` (css-shapes-1 — R-273) : la forme n'a
        // d'effet que sur un **flottant** entouré de texte
        // (les line boxes contournent la forme). `.x` = le
        // flottant dimensionné ; le `<p>` apporte les lignes.
        () if n == "shape-outside" || n == "shape-margin" || n == "shape-image-threshold" => {
            "<body><div><div class=x style=\"float:left;width:80px;height:80px\"></div><p>un deux trois quatre cinq six sept huit neuf dix onze douze treize quatorze quinze seize</p></div></body>"
        }
        // `container-type`/`container` (css-contain-3 — R-274) :
        // le `contain:size` implicite ne se voit que sur une
        // boîte **shrink-to-fit** dont la taille dépend de la
        // contribution intrinsèque des enfants — `.x` absolu
        // `width/height:auto` avec un enfant 100×40 :
        // `size` → 0×0, `inline-size` → 0×40, `normal` → 100×40.
        () if n == "container-type" || n == "container" => {
            "<body><div style=\"position:relative;width:400px;height:200px\"><div class=x style=\"position:absolute\"><p style=\"width:100px;height:40px;margin:0\">a</p></div></div></body>"
        }
        // Conteneur **débordant** (étape 219) — `overflow-*`
        // et `overscroll-*` n'ont d'oracle que si le contenu
        // excède la boîte : `hidden`/`clip` coupent vs
        // `scroll`/`auto` posent la gouttière vs `visible`.
        () if matches!(
            n,
            "overflow" | "overflow-x" | "overflow-y" | "overflow-block"
                | "overflow-inline" | "overscroll-behavior" | "overscroll-behavior-x"
                | "overscroll-behavior-y" | "overscroll-behavior-block"
                | "overscroll-behavior-inline"
        ) =>
        {
            "<body><div class=x style=\"width:100px;height:80px\"><p style=\"height:200px\">a</p></div></body>"
        }
        // Conteneur **déjà scrollable** — `scroll-snap-*`/
        // `scroll-margin`/`scroll-padding` (étape 219) : la
        // valeur n'a d'effet possible que sur un scroller.
        () if matches!(
            n,
            "scroll-snap-type" | "scroll-snap-align" | "scroll-snap-stop" | "scroll-margin"
                | "scroll-margin-top" | "scroll-margin-right" | "scroll-margin-bottom"
                | "scroll-margin-left" | "scroll-margin-block" | "scroll-margin-inline"
                | "scroll-margin-block-start" | "scroll-margin-block-end"
                | "scroll-margin-inline-start" | "scroll-margin-inline-end" | "scroll-padding"
                | "scroll-padding-top" | "scroll-padding-right" | "scroll-padding-bottom"
                | "scroll-padding-left" | "scroll-padding-block" | "scroll-padding-inline"
                | "scroll-padding-block-start" | "scroll-padding-block-end"
                | "scroll-padding-inline-start" | "scroll-padding-inline-end"
        ) =>
        {
            "<body><div class=x style=\"width:100px;height:80px;overflow:scroll\"><p style=\"height:200px\">a</p></div></body>"
        }
        // `background*`/`mask*` — pas de `background` inline
        // (il écraserait la déclaration testée) : l'image de
        // contexte est posée par `context_css` (gradient
        // peint sans ressource externe).
        () if n.starts_with("background") || n.starts_with("mask") => {
            "<body><div class=x><p>a</p></div></body>"
        }
        // Boîte à dimensions fixes — `position`, `inset`,
        // `transform`, `overflow`, `scroll-snap`, `contain`,
        // `clip-*` : un box 100×80 suffit (les effets
        // position/sizing/visibilité sont mesurables).
        () if matches!(
            n,
            "position" | "top" | "right" | "bottom" | "left" | "inset"
                | "z-index" | "transform"
                | "contain" | "contain-intrinsic-size"
                | "contain-intrinsic-width" | "contain-intrinsic-height" | "content-visibility"
                | "clip" | "clip-path" | "visibility" | "opacity" | "filter"
                | "mix-blend-mode" | "isolation" | "box-shadow" | "box-sizing"
                | "object-fit" | "object-position"
        ) =>
        {
            "<body><div class=x style=\"width:100px;height:80px;background:blue\"><p>a</p></div></body>"
        }
        // `text-box*` (R-279) — le trim retire le demi-
        // leading des bords : il faut un `line-height` qui
        // en crée un + plusieurs lignes pour discriminer
        // `trim-start`/`trim-end` de `trim-both`.
        () if n == "text-box" || n.starts_with("text-box-") => {
            "<body><p class=x style=\"width:60px;line-height:3\">alpha beta gamma delta epsilon zeta</p></body>"
        }
        // Texte multi-mots — `text-*`, `white-space`,
        // `word-*`, `line-*`, `font-*`, `text-wrap`,
        // `hyphens`, `text-transform`, `letter/word-spacing` :
        // la ligne wrappe ou se mesure différemment.
        () if n.starts_with("text-")
            || n.starts_with("font-")
            || n.starts_with("letter-")
            || n.starts_with("line-")
            || n.starts_with("word-")
            || n.starts_with("white-")
            || n.starts_with("tab-")
            || matches!(
                n,
                "direction" | "unicode-bidi" | "writing-mode" | "text-orientation"
                    | "hyphens" | "text-transform" | "font" | "quotes" | "text-indent"
            ) =>
        {
            // Texte riche (étape 270) : majuscule pour
            // `lowercase`, double-espace + saut de ligne
            // pour `pre-wrap`/`pre-line`/préservation, mot
            // long suffisant pour le wrap à 80 px.
            "<body><div><p class=x style=\"width:80px\">un Deux  trois\nquatre cinq six</p></div></body>"
        }
        // `overflow-clip-margin` (étape 302) — la marge de
        // clip n'existe que si un enfant DÉBORDE de la
        // boîte `.x` : le ctx pose `overflow:hidden` + la
        // taille bornée, l'enfant 200 px dépasse à droite.
        () if n == "overflow-clip-margin" => {
            "<body><div class=x><div style=\"width:200px;height:40px;background:red\"></div></div></body>"
        }
        // `line-clamp` (étape 302) — le clamp n'a d'oracle
        // que sur un texte qui DÉPASSE le nombre de lignes
        // (ctx borne la largeur → multi-lignes) ; un texte
        // court rend toute valeur inerte par construction.
        () if n == "line-clamp" => {
            "<body><div><p class=x>un deux trois quatre cinq six sept huit neuf dix onze</p></div></body>"
        }
        // `margin-trim` (étape 302) — s'applique au
        // conteneur et coupe les marges de l'enfant aux
        // bords : `.x` = le conteneur, l'enfant porte les
        // marges à trimer (sans ça, rien à couper).
        () if n == "margin-trim" => {
            "<body><div class=x><p style=\"margin-top:30px;margin-bottom:30px\">a</p></div></body>"
        }
        // `hanging-punctuation` (étape 302) — `first` ne
        // pend que si un guillemet ouvre la première ligne
        // (ctx borne la largeur → wrap déterministe).
        () if n == "hanging-punctuation" => {
            "<body><div><p class=x>\"abcdefgh ijklmno pqrst uvw</p></div></body>"
        }
        // `text-emphasis-position` (étape 302) — la
        // position n'a d'effet que sur des marques posées
        // (ctx `text-emphasis:dot`) — texte CJK pour
        // `over`/`under`.
        () if n == "text-emphasis-position" => {
            "<body><div><p class=x>かなかな</p></div></body>"
        }
        // `white-space-trim` (étape 302) — `discard-*` ne
        // trime que de l'espace PRÉSERVÉ : ctx `pre-wrap` +
        // espaces en bord de boîte — sous `normal` ils
        // sont déjà collapsés, rien n'est observable.
        () if n == "white-space-trim" => {
            "<body><div><p class=x>  abc def  </p></div></body>"
        }
        // `text-spacing-trim` (étape 302) — la compression
        // CJK n'existe que sur une ponctuation pleine-
        // chasse en bord de ligne : ctx borne la largeur
        // pour placer `、` en tête de seconde ligne.
        () if n == "text-spacing-trim" => {
            "<body><div><p class=x>abcde、かなかな</p></div></body>"
        }
        // Défaut — `<p>` dans `<div>` (texte, box, décorations).
        () => "<body><div><p class=x>ok</p></div></body>",
    }
}

/// Fonte du harnais `affectsv` : Ahem (WPT) embarquée par
/// `include_bytes!`, servie à **toute** requête — sans
/// `FontSource` les fragments restent `shaped: None` et
/// aucun item `Text` n'est émis (`color`, `font-*`,
/// `text-decoration` invisibles à la jauge). Les octets
/// identiques à toute requête n'importent pas : l'oracle
/// mesure le `FontRequest` et la couleur de l'item, pas les
/// contours.
struct BcdAhem;
impl explorer_layout::font::FontSource for BcdAhem {
    fn font_data(&self, _req: &explorer_layout::font::FontRequest) -> Option<Vec<u8>> {
        static AHEM: &[u8] = include_bytes!("../../explorer-render/tests/data/wpt/ahem.ttf");
        Some(AHEM.to_vec())
    }
}

/// Bitmap du harnais `paintv` (étape 274) : SVG 8×8 servi en
/// `data:` URI — décodé par `collect_data_images`, **aucune
/// I/O** (la borne « `PaintImages` vide » des familles
/// `mask-*`/`border-image*`/`object-*`/`background-image`
/// disparaît). Quatre **quadrants** 2D — une bitmap
/// verticalement uniforme rend `object-fit:cover`,
/// `object-position` et `mask-position` invisibles par
/// construction (mesuré au harnais) ; chaque quadrant a
/// couleur **et** alpha propres : les deux interprétations
/// de masque (alpha/luminance), les positions, les
/// répétitions et les découpes `border-image-slice` sont
/// discriminables sur la même image.
const HARNESS_IMG: &str = concat!(
    "data:image/svg+xml,",
    "%3Csvg xmlns='http://www.w3.org/2000/svg' width='8' height='8'%3E",
    "%3Crect width='4' height='4' fill='%230040c8'/%3E",
    "%3Crect x='4' width='4' height='4' fill='%23c84000' fill-opacity='0.5'/%3E",
    "%3Crect y='4' width='4' height='4' fill='%2300a040'/%3E",
    "%3Crect x='4' y='4' width='4' height='4' fill='%23e0c000' fill-opacity='0.6'/%3E",
    "%3C/svg%3E"
);

/// Borne pixels passée à `collect_data_images`/
/// `collect_svg_styled` au harnais `paintv` (même ordre que
/// la session : une fixture hostile ne doit pas rasteriser
/// des giga-pixels).
const HARNESS_MAX_PX: u32 = 1 << 22;

/// Déclarations de **contexte** injectées dans la signature
/// `.x` (`affectsv`) : la prop testée n'a d'effet observable
/// que sur une boîte préparée — `border-color` sans style ne
/// peint rien, `column-rule-*` sans colonnes non plus. La
/// signature compare `.x { ctx }` à `.x { ctx; name:value }`
/// — même règle, l'ordre fait gagner la déclaration testée.
/// À distinguer du `<style>` embarqué (jamais collecté par
/// `compute_tree` dans ce chemin) et de `style=` inline
/// (rang inline > auteur — écraserait la déclaration).
fn context_css(name: &str) -> &'static str {
    let n = name.trim_start_matches('-').trim_start_matches("webkit-");
    if n == "position-try-order" {
        // `bottom` en tête : l'ordre `normal` finit sur
        // `left` (aucun ne rentre) ; `most-width`/`most-size`
        // trient différemment → repli final différent.
        return "position-try-fallbacks:bottom,left";
    }
    if n.starts_with("border") {
        // Logique (`border-block-*`/`border-inline-*`) : le
        // mapping physique arrive APRÈS la cascade — un ctx
        // uniforme poserait la même prop physique au même
        // rang et gagnerait à égalité. `-color`/`-width`
        // exigent juste un `border-style` existant (autre
        // nom de prop — pas de conflit) ; `-style` et le
        // shorthand nu créent la bordure seuls → pas de ctx.
        if n.contains("-block") || n.contains("-inline") {
            if n.ends_with("-style") || (!n.ends_with("-color") && !n.ends_with("-width")) {
                ""
            } else {
                "border-style:solid"
            }
        } else {
            "border:2px solid black;width:100px;height:80px;background:blue"
        }
    } else if n.starts_with("mask") {
        // Les longhands `mask-*` n'ont d'effet que sur une
        // source `mask-image` (même astuce gradient).
        "mask-image:linear-gradient(red,blue);width:100px;height:80px"
    } else if n.starts_with("background-position") {
        // Les positions ne sont discriminables que si
        // l'image ne remplit pas la zone : gradient réduit
        // à 40 % sans répétition (étape 270 — l'image
        // calibrée à la zone rendait tout offset nul).
        "background-image:linear-gradient(red,blue);background-repeat:no-repeat;background-size:40% 40%;border:10px solid black;padding:10px;width:100px;height:80px"
    } else if n.starts_with("border-image") || n == "border-image" {
        // `border-image-*` n'ont d'effet qu'avec une source
        // et un découpage posés — le gradient sert de
        // bitmap sans ressource externe.
        "border-image-source:linear-gradient(red,blue);border-image-slice:30;border:10px solid black;width:100px;height:80px"
    } else if n.starts_with("background") {
        // `position`/`repeat`/`size`/`attachment`/`blend` n'ont
        // d'effet que sur une image de fond — gradient
        // `linear-gradient` (peint sans ressource externe,
        // `PaintImages` vide au harnais). La bordure distingue
        // `content-box`/`padding-box`/`border-box` (sans elle
        // les trois aires coïncident → `clip`/`origin` inertes).
        "background-image:linear-gradient(red,blue);border:10px solid black;padding:10px;width:100px;height:80px"
    } else if n.starts_with("text-decoration-") || n.starts_with("text-underline-") {
        // `text-decoration-color`/`-line`/`-style`/`-thickness`,
        // `text-underline-position`/`-offset` n'ont d'effet
        // qu'avec un trait posé.
        "text-decoration:underline"
    } else if n.starts_with("outline") {
        "outline:2px solid black;width:100px;height:80px"
    } else if n.starts_with("corner") {
        // `corner-*-shape` ne ploie que les coins posés par
        // `border-radius` (css-borders-4 §3.7 — sans rayon
        // le coin est franc quelle que soit la forme) ;
        // le fond porte les `corners` visibles du dump.
        "border-radius:15px;background:#ccc;width:80px;height:60px"
    } else if n == "rule" || n.starts_with("rule-") || n.starts_with("row-rule") {
        // css-gaps-1 : le décor de gap n'existe qu'avec les
        // colonnes posées ; `column-rule` en ctx rend les
        // longhands `rule-*` discriminants (couleur/largeur/
        // style/inset/break) — le shorthand `rule` l'écrase.
        "columns:2;column-gap:20px;column-rule:2px solid black;height:100px"
    } else if n == "offset" || n.starts_with("offset-") {
        // css-motion-1 : le transform n'existe que si le
        // chemin est tracé ET la distance non nulle — les
        // longhands seuls (`offset-distance`, `-rotate`…) sont
        // inertes sans `offset-path` en ctx ; `offset-path`
        // seul est inerte à distance 0 (point de départ =
        // ancre). Le shorthand `offset` réinitialise la
        // distance → inerte s'il n'en porte pas (borne
        // honnête de la spec : `offset:<path>` = distance 0).
        if matches!(n, "offset" | "offset-path") {
            "offset-distance:20px"
        } else if n == "offset-anchor" {
            // L'ancre n'a d'effet que si la position du
            // chemin est **fixe** — `offset-position:normal`
            // (défaut) suit l'ancre et `T − A` est invariant.
            "offset-path:ray(90deg);offset-distance:20px;offset-position:10px 10px"
        } else {
            "offset-path:ray(90deg);offset-distance:20px"
        }
    } else if n == "box-sizing" {
        // border-box vs content-box ne diffèrent qu'avec
        // padding ET bordure (étape 270).
        "padding:10px;border:5px solid black;width:100px;height:80px"
    } else if n == "text-overflow" {
        // L'ellipse/le clip n'existent que sur une ligne
        // qui déborde ET est coupée (étape 270).
        "white-space:nowrap;overflow:hidden;width:60px"
    } else if n == "text-indent" || n == "text-align" || n == "text-align-last" {
        // `each-line`/`hanging`/`justify` ne se voient que
        // sur plusieurs lignes — largeur bornée (étape 270).
        "width:60px"
    } else if n == "text-orientation" {
        // `sideways`/`upright` n'ont d'oracle qu'en écriture
        // verticale (étape 270).
        "writing-mode:vertical-rl"
    } else if n == "overflow-clip-margin" {
        // La marge de clip n'a d'effet qu'avec un
        // `overflow` non-`visible` (étape 270).
        "overflow:hidden;width:80px;height:60px"
    } else if n == "content" {
        // Les paires de guillemets pour `open-quote`/
        // `close-quote` (la sonde cible `.x::before`).
        "quotes:\"«\" \"»\""
    } else if n == "text-justify" {
        // `text-justify` ne règle que la méthode de la
        // justification — inerte sans `text-align:justify`.
        "text-align:justify"
    } else if n == "text-box-edge" {
        // L'edge ne produit d'effet que sous un trim actif
        // (css-inline-3 §10) — `trim-both` en ctx rend chaque
        // valeur discriminable.
        "text-box-trim:trim-both"
    } else if n.starts_with("hyphenate") {
        // `hyphenate-character`/`hyphenate-limit-*` n'ont
        // d'effet que sous césure automatique.
        "hyphens:auto"
    } else if n == "shape-margin" || n == "shape-image-threshold" {
        // `shape-margin` dilate la forme, `shape-image-
        // threshold` n'a d'effet que contre une forme posée.
        "shape-outside:circle(40%)"
    } else if n.starts_with("column-") || n == "columns" {
        "columns:2;column-rule:2px solid black;height:200px"
    } else if n == "line-clamp" {
        // Le clamp n'a d'oracle que si le texte wrappe sur
        // plus de lignes que la valeur — largeur bornée
        // (étape 302, fixture texte long).
        "width:80px"
    } else if n == "hanging-punctuation" {
        // Le pendage en bord de ligne exige des lignes
        // bornées (étape 302).
        "width:80px"
    } else if n == "text-emphasis-position" {
        // La position n'est observable que sur des marques
        // réellement posées (étape 302).
        "text-emphasis:dot"
    } else if n == "white-space-trim" {
        // `discard-*` ne trime que l'espace **préservé** —
        // sans `pre-wrap` les espaces de bord sont déjà
        // collapsés par `white-space:normal` (étape 302).
        "white-space:pre-wrap"
    } else if n == "text-spacing-trim" {
        // La compression CJK n'est visible que si la
        // ponctuation tombe en bord de ligne — largeur
        // bornée (étape 302).
        "width:80px"
    } else {
        ""
    }
}

fn prop_affects_decl(decl: &str) -> bool {
    static SRC: BcdAhem = BcdAhem;
    let Some((name, value)) = decl.split_once(':') else {
        return false;
    };
    let name = name.trim();
    let ctx = explorer_layout::BlockCtx {
        fonts: Some(&SRC),
        ..explorer_layout::BlockCtx::default()
    };
    let dom = explorer_html::TreeBuilder::parse(fixture_for(name));
    let root = {
        let mut r = None;
        walk_any(dom.document(), &dom, &mut r);
        r.unwrap_or(dom.document())
    };
    let sig = |css: &str| -> String {
        let mut eng = explorer_css::StyleEngine::new();
        eng.add_css(css, explorer_css::Origin::Author);
        let sm = eng.compute_tree(&dom, root, &explorer_css::StyleEnv::default());
        match explorer_layout::build_box_tree(&dom, root, &sm) {
            Some(mut bx) => {
                explorer_layout::layout_tree(&mut bx, 800.0, &ctx);
                // Box tree + display list : la géométrie seule
                // rate les effets paint (`accent-color`,
                // `caret-color`, `text-shadow`…). Limite
                // connue : la couleur du run texte n'est pas
                // dans `dl.dump` — `color:*`/`caret-*` purs
                // sur texte restent invisibles à la jauge.
                let dl = explorer_render::build_display_list(&bx, 800.0, 600.0, 0.0);
                format!("{}\n{}", bx.dump(&dom), dl.dump())
            }
            None => String::new(),
        }
    };
    let cctx = context_css(name);
    // `initial-letter` n'existe que sur `::first-letter`
    // (css-inline-3 §4 — non hérité, inerte ailleurs) : la
    // déclaration sondée porte sur le pseudo, le ctx reste
    // sur `.x`.
    let sel = match name {
        // `initial-letter` n'existe que sur `::first-letter`
        // (css-inline-3 §4 — non hérité, inerte ailleurs).
        "initial-letter" => ".x::first-letter",
        // `content` n'a d'effet que sur un pseudo-élément
        // généré (étape 270 — sur l'élément normal seuls
        // les remplacés compteraient, hors harnais).
        "content" => ".x::before",
        _ => ".x",
    };
    // La signature de base `.x { ctx }` ne dépend que de la
    // prop (fixture + ctx + sel sont des fonctions pures de
    // `name`) — une entrée par prop suffit (étape 274 : la
    // re-computait pour chaque valeur → moitié du temps de
    // passe gaspillée sur ~10-15 lignes par prop).
    let a = sig(&format!("{sel} {{ {cctx}; {}: {} }}", name, value.trim()));
    let b = BASE_GEO.with(|c| {
        c.borrow_mut()
            .entry(name.to_string())
            .or_insert_with(|| sig(&format!("{sel} {{ {cctx} }}")))
            .clone()
    });
    if std::env::var_os("EXPLORER_BCD_DEBUG_SIG").is_some() {
        eprintln!("=== {name}:{value}\n--- with\n{a}\n--- base\n{b}");
    }
    a != b
}

thread_local! {
    /// Signatures de **base** mises en cache par prop
    /// (étape 274) — `prop_affects_decl` (dump) et
    /// `prop_paints_decl` (framebuffer). La signature
    /// `.x { ctx }` sans déclaration est invariante pour
    /// toutes les valeurs d'une même prop : le cache divise
    /// par ~2 le coût de la passe `check` (parse DOM +
    /// cascade + layout + dump/raster refaits à chaque ligne
    /// sinon). `Vec<u8>` cloné par appel — ~1,9 Mio,
    /// amorti par le raster évité.
    static BASE_GEO: std::cell::RefCell<std::collections::HashMap<String, String>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
    static BASE_PIX: std::cell::RefCell<
        std::collections::HashMap<String, Option<Vec<u8>>>,
    > = std::cell::RefCell::new(std::collections::HashMap::new());
    /// Idem pour `check computedv` (étape 298) — la
    /// `StyleMap` `.x { ctx }` sérialisée est invariante
    /// par prop.
    static BASE_CMP: std::cell::RefCell<std::collections::HashMap<String, String>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

/// Contexte **additionnel** de l'oracle pixel (étape 274) —
/// concaténé après [`context_css`] (l'ordre fait gagner le
/// ctx pixel en cas de doublon). Un seul objet : poser la
/// bitmap du harnais ([`HARNESS_IMG`]) en source `url()`
/// pour les familles dont l'effet n'existe que sur une
/// image décodée — `PaintImages` était vide au harnais
/// `affectsv` (borne `mask-*`/`border-image*`/`object-*`
/// documentée dans [`NO_ORACLE`]), ici `collect_data_images`
/// la sert réellement.
fn paint_ctx_extra(name: &str) -> String {
    let n = name.trim_start_matches('-').trim_start_matches("webkit-");
    if n == "mask-image" || n == "mask" {
        // La prop **est** la source : pas de mask en ctx —
        // la déclaration la pose (base = boîte non masquée).
        ";background-color:#c04000;width:100px;height:80px".into()
    } else if n.starts_with("mask") {
        // Longhands `mask-*` : sans source la valeur est
        // inerte — bitmap réelle posée + remplissage à
        // masquer (la moitié semi-transparente rend le
        // masquage visible dans les pixels).
        format!(
            ";mask-image:url(\"{HARNESS_IMG}\");background-color:#c04000;width:100px;height:80px"
        )
    } else if n == "border-image-slice" {
        // Source `url()` réelle (`borders_url` l'exige,
        // R-248) + slice ctx `40%` : les pixels de coupe
        // saturent sur une bitmap 8 px (`30` > 8 = image
        // entière = `100%` → miss par identité, mesuré) —
        // `40%` discrimine nombres et pourcentages.
        format!(";border-image-source:url(\"{HARNESS_IMG}\");border-image-slice:40%")
    } else if n == "object-position" {
        // La position n'existe que si le bitmap ne remplit
        // pas la boîte — `fill` (défaut) l'étire et rend
        // tout offset invisible.
        ";object-fit:none".into()
    } else if n.starts_with("border-image-") && n != "border-image-source" {
        // Longhands `border-image-*` : `borders_url` exige
        // une source `url()` — le gradient du ctx
        // géométrique n'y vit pas (R-248). `border-image` et
        // `-source` exclus : leur déclaration **est** la
        // source — un ctx bitmap ferait compter `ok` une
        // valeur qui produit moins de peinture que la base
        // (ex : gradient non servi), faux positif.
        format!(";border-image-source:url(\"{HARNESS_IMG}\")")
    } else if n == "shape-image-threshold" {
        // Le seuil n'existe que contre une forme **image**
        // (le ctx géométrique pose `circle()` — la url le
        // remplace ; borne alpha analysée ou non côté moteur,
        // `miss` honnête si absente).
        format!(";shape-outside:url(\"{HARNESS_IMG}\")")
    } else if n == "offset" || n.starts_with("offset-") {
        // Le `.x` de la fixture motion-path est une boîte
        // vide (10×10 transparente) : déplacée sur le chemin
        // elle reste invisible — fond posé pour que
        // distance/rotate/ancre produisent des pixels.
        ";background:#c04000".into()
    } else if n == "image-rendering" {
        // Lissage/net ne se voient qu'en bitmap mis à
        // l'échelle (8×8 → 80 px).
        format!(
            ";background-image:url(\"{HARNESS_IMG}\");background-size:80px 80px;background-repeat:no-repeat;width:100px;height:80px"
        )
    } else if n.starts_with("background") && n != "background-image" {
        // `position`/`size`/`repeat`/`clip`/`origin`/`blend`
        // discriminent sur une vraie bitmap (le gradient ctx
        // est remplacé — dernier mot gagne).
        format!(";background-image:url(\"{HARNESS_IMG}\")")
    } else {
        String::new()
    }
}

/// Fixture `paintv` (étape 274) : [`fixture_for`] par défaut ;
/// les familles qui exigent un porteur précis reçoivent la
/// leur — remplacé `<img>` pour `object-*`, sous-arbre `<svg>`
/// pour les props à portée SVG2 (le `collect_svg_styled` du
/// harnais projette la déclaration calculée dans l'îlot
/// rasterisé — `fill`/`stroke`/`cx`… restent inertes sur un
/// élément HTML par spec, pas par trou moteur).
fn paint_fixture_for(name: &str) -> String {
    let n = name.trim_start_matches('-').trim_start_matches("webkit-");
    // `object-*` dimensionne/positionne la bitmap d'un
    // remplacé — `.x` = `<img>` servie par la data-URI.
    if matches!(n, "object-fit" | "object-position" | "object-view-box") {
        return format!(
            "<body><img class=x width=\"80\" height=\"40\" src=\"{HARNESS_IMG}\"></body>"
        );
    }
    match () {
        () if matches!(n, "cx" | "cy" | "r") => concat!(
            "<body><svg width=\"160\" height=\"120\" viewBox=\"0 0 160 120\">",
            "<circle class=x cx=\"80\" cy=\"60\" r=\"20\" fill=\"#c04000\"/></svg></body>"
        ),
        () if matches!(n, "x" | "y" | "rx" | "ry") => concat!(
            "<body><svg width=\"160\" height=\"120\" viewBox=\"0 0 160 120\">",
            "<rect class=x x=\"20\" y=\"20\" width=\"80\" height=\"60\" fill=\"#c04000\"/></svg></body>"
        ),
        () if n == "d" => concat!(
            "<body><svg width=\"160\" height=\"120\" viewBox=\"0 0 160 120\">",
            "<path class=x d=\"M20 20h100v60h-100z\" fill=\"#c04000\"/></svg></body>"
        ),
        () if n == "fill-rule" => concat!(
            "<body><svg width=\"160\" height=\"120\" viewBox=\"0 0 160 120\">",
            "<path class=x d=\"M10 10h140v100h-140zM40 40h80v40h-80z\"",
            " fill=\"#c04000\" fill-rule=\"nonzero\"/></svg></body>"
        ),
        // `fill`/`stroke`/`-opacity`/`-width` + refs
        // `url(#g)` : le dégradé est défini dans la fixture,
        // la sonde `url` le vise.
        () if matches!(
            n,
            "fill" | "fill-opacity" | "stroke" | "stroke-opacity" | "stroke-width"
        ) => concat!(
            "<body><svg width=\"160\" height=\"120\" viewBox=\"0 0 160 120\">",
            "<defs><linearGradient id=\"g\">",
            "<stop offset=\"0\" stop-color=\"#c04000\"/><stop offset=\"1\" stop-color=\"#0040c8\"/>",
            "</linearGradient></defs>",
            "<rect class=x x=\"20\" y=\"20\" width=\"100\" height=\"70\"",
            " fill=\"#c04000\" stroke=\"#0040c8\" stroke-width=\"8\"/></svg></body>"
        ),
        () if n.starts_with("stroke-") || n == "paint-order" => concat!(
            "<body><svg width=\"160\" height=\"120\" viewBox=\"0 0 160 120\">",
            "<rect class=x x=\"20\" y=\"20\" width=\"100\" height=\"70\"",
            " fill=\"#c04000\" stroke=\"#0040c8\" stroke-width=\"8\"/></svg></body>"
        ),
        // `vector-effect:non-scaling-stroke` n'existe que
        // sous viewport mis à l'échelle (viewBox 4× plus
        // petit que la surface → le tracé est agrandi ×4).
        () if n == "vector-effect" => concat!(
            "<body><svg width=\"160\" height=\"120\" viewBox=\"0 0 40 30\">",
            "<rect class=x x=\"5\" y=\"5\" width=\"25\" height=\"18\"",
            " fill=\"#c04000\" stroke=\"#0040c8\" stroke-width=\"2\"/></svg></body>"
        ),
        () if matches!(n, "stop-color" | "stop-opacity") => concat!(
            "<body><svg width=\"160\" height=\"120\" viewBox=\"0 0 160 120\">",
            "<defs><linearGradient id=\"g\">",
            "<stop class=x offset=\"0\" stop-color=\"#c04000\"/><stop offset=\"1\" stop-color=\"#0040c8\"/>",
            "</linearGradient></defs>",
            "<rect width=\"160\" height=\"120\" fill=\"url(#g)\"/></svg></body>"
        ),
        () => fixture_for(name),
    }
    .into()
}

/// Oracle **`paintv`** (étape 274) — la déclaration
/// `.x { ctx; name:value }` change-t-elle le **framebuffer** ?
/// Complément d'`affectsv` : les effets de peinture ne vivent
/// ni dans le box tree ni dans le dump display-list
/// (`filter`, `clip-path`, `transform-origin`, `outline`,
/// `mask-*`…) — ils vivent dans les pixels. Pipeline réel :
/// `collect_data_images` (la bitmap `data:` du harnais est
/// décodée pour de vrai) + `collect_svg_styled` (îlots
/// `<svg>` style-aware) + `build_display_list_full` +
/// `rasterize`. Comparaison **exacte** — moteur et fonte
/// Ahem sont déterministes : un pixel différent = la valeur
/// a un effet visible, zéro = inerte ici (`miss` honnête si
/// la valeur devait bouger).
///
/// Même biais assumé qu'`affectsv`, documenté : on mesure
/// « la déclaration change quelque chose », pas « le rendu
/// est conforme » — un rendu faux mais différent compte `ok`
/// (la conformité visuelle reste l'affaire des goldens WPT).
fn prop_paints_decl(decl: &str) -> bool {
    static SRC: BcdAhem = BcdAhem;
    let Some((name, value)) = decl.split_once(':') else {
        return false;
    };
    let name = name.trim();
    let ctx = explorer_layout::BlockCtx {
        fonts: Some(&SRC),
        ..explorer_layout::BlockCtx::default()
    };
    let fixture = paint_fixture_for(name);
    let dom = explorer_html::TreeBuilder::parse(&fixture);
    let root = {
        let mut r = None;
        walk_any(dom.document(), &dom, &mut r);
        r.unwrap_or(dom.document())
    };
    let has_svg = fixture.contains("<svg");
    let sig = |css: &str| -> Option<Vec<u8>> {
        let mut eng = explorer_css::StyleEngine::new();
        eng.add_css(css, explorer_css::Origin::Author);
        let sm = eng.compute_tree(&dom, root, &explorer_css::StyleEnv::default());
        let mut bx = explorer_layout::build_box_tree(&dom, root, &sm)?;
        explorer_layout::layout_tree(&mut bx, 800.0, &ctx);
        let mut images = explorer_render::collect_data_images(&dom, &sm, HARNESS_MAX_PX);
        if has_svg {
            let (svgs, srcs) = explorer_render::collect_svg_styled(&dom, root, &sm, HARNESS_MAX_PX);
            images.svgs = svgs;
            images.svg_sources = srcs;
        }
        let dl = explorer_render::build_display_list_full(&bx, 800.0, 600.0, 0.0, &images);
        explorer_render::rasterize(&dl, &SRC, 1.0).map(|f| f.pixels)
    };
    let cctx = format!("{}{}", context_css(name), paint_ctx_extra(name));
    // Mêmes pseudo-cibles qu'`affectsv` (`initial-letter`,
    // `content`) — le pseudo élément généré est peignable.
    let sel = match name {
        "initial-letter" => ".x::first-letter",
        "content" => ".x::before",
        _ => ".x",
    };
    // Même cache de base que `affectsv` ([`BASE_PIX`]) —
    // la frame `.x { ctx }` est invariante par prop.
    let a = sig(&format!("{sel} {{ {cctx}; {}: {} }}", name, value.trim()));
    let b = BASE_PIX.with(|c| {
        c.borrow_mut()
            .entry(name.to_string())
            .or_insert_with(|| sig(&format!("{sel} {{ {cctx} }}")))
            .clone()
    });
    // Les deux frames font 800×600×4 (rasterize dimensionne
    // à la display list = viewport) : `!=` sur `Option` gère
    // l'échec de raster d'un côté comme un signal de diff.
    if std::env::var_os("EXPLORER_BCD_DEBUG_SIG").is_some() {
        let diff = match (&a, &b) {
            (Some(x), Some(y)) => format!("{} o != {} o", x.len(), y.len()),
            _ => "raster échoué d'un côté".into(),
        };
        eprintln!("=== paintv {name}:{value} — base {diff}");
    }
    a.is_some() && b.is_some() && a != b
}

/// `check computedv` (étape 298) — troisième oracle de la
/// jauge, niveau **style calculé** : la `StyleMap` du
/// sous-arbre `.x` sérialisée (props ∪ vars, ordre DOM,
/// pseudos inclus), avec et sans la déclaration — la prop
/// sondée **exclue** de la signature (la cascade stocke les
/// déclarations vainqueurs verbatim : inclure la prop
/// saturerait l'oracle comme `routed`). Ce qui produit un
/// delta honnête : expansion shorthand (`inset` → 4
/// longhands, `border` → ~12), résolution logique
/// (`block-size` → `height`, `margin-inline` → côtés
/// physiques — `resolve_logical_props`), effets croisés
/// (paire `overflow` qui force l'autre axe, héritage sur
/// les descendants).
///
/// Borne assumée — **diagnostic, hors union ok** :
/// « cascade applique » ≠ « moteur pose/peint »
/// (`overflow-block` s'y stocke verbatim sans effet aval
/// → miss des deux côtés ; `border-image` expanserait
/// tout en restant non-peint — `computed` seul ne
/// retourne pas une paire en `ok`).
fn prop_computes_decl(decl: &str) -> bool {
    let Some((name, value)) = decl.split_once(':') else {
        return false;
    };
    let name = name.trim();
    let dom = explorer_html::TreeBuilder::parse(fixture_for(name));
    let root = {
        let mut r = None;
        walk_any(dom.document(), &dom, &mut r);
        r.unwrap_or(dom.document())
    };
    // Cible `.x` — la signature couvre son sous-arbre :
    // l'effet d'un héritage sur les descendants fait partie
    // du delta calculé.
    let xnode = {
        let mut x = None;
        find_class_x(dom.document(), &dom, &mut x);
        x
    };
    let sig = |css: &str| -> String {
        let mut eng = explorer_css::StyleEngine::new();
        eng.add_css(css, explorer_css::Origin::Author);
        let sm = eng.compute_tree(&dom, root, &explorer_css::StyleEnv::default());
        let mut out = String::new();
        if let Some(x) = xnode {
            dump_computed_subtree(x, &dom, &sm, name, &mut out);
        }
        out
    };
    let cctx = context_css(name);
    // Mêmes pseudo-cibles que les oracles rendus.
    let sel = match name {
        "initial-letter" => ".x::first-letter",
        "content" => ".x::before",
        _ => ".x",
    };
    // Même cache de base que `affectsv`/`paintv` — la
    // `StyleMap` `.x { ctx }` est invariante par prop.
    let a = sig(&format!("{sel} {{ {cctx}; {}: {} }}", name, value.trim()));
    let b = BASE_CMP.with(|c| {
        c.borrow_mut()
            .entry(name.to_string())
            .or_insert_with(|| sig(&format!("{sel} {{ {cctx} }}")))
            .clone()
    });
    if std::env::var_os("EXPLORER_BCD_DEBUG_SIG").is_some() {
        eprintln!("=== computedv {name}:{value}\n--- with\n{a}\n--- base\n{b}");
    }
    a != b
}

/// Premier élément porteur d'un token `x` dans `class` —
/// cible des fixtures de la jauge.
fn find_class_x(
    n: explorer_dom::NodeId,
    dom: &explorer_dom::Dom,
    out: &mut Option<explorer_dom::NodeId>,
) {
    if out.is_none()
        && dom
            .attribute_str(n, "class")
            .ok()
            .flatten()
            .is_some_and(|c| c.split_whitespace().any(|w| w == "x"))
    {
        *out = Some(n);
    }
    if let Ok(ch) = dom.children(n) {
        for c in ch {
            find_class_x(c, dom, out);
        }
    }
}

/// Sérialise le style calculé de `n` (+ pseudos routés +
/// descendants, ordre DOM) dans `out` — `skip` = prop
/// sondée, ignorée partout (anti-saturation).
fn dump_computed_subtree(
    n: explorer_dom::NodeId,
    dom: &explorer_dom::Dom,
    sm: &explorer_css::StyleMap,
    skip: &str,
    out: &mut String,
) {
    use std::fmt::Write;
    if let Some(st) = sm.get(n) {
        ser_computed(st, skip, out);
    }
    for which in [
        explorer_css::PseudoElement::Before,
        explorer_css::PseudoElement::After,
        explorer_css::PseudoElement::FirstLine,
        explorer_css::PseudoElement::FirstLetter,
        explorer_css::PseudoElement::Marker,
    ] {
        if let Some(ps) = sm.pseudo(n, which) {
            let _ = write!(out, "|@{which:?}");
            ser_computed(ps, skip, out);
        }
    }
    if let Ok(ch) = dom.children(n) {
        for c in ch {
            dump_computed_subtree(c, dom, sm, skip, out);
        }
    }
}

/// `k=v` triés d'un `ComputedStyle`, `skip` exclu —
/// `iter()` livre props ∪ vars (les vars restent : une
/// déclaration qui résout à travers elles doit se voir).
fn ser_computed(st: &explorer_css::ComputedStyle, skip: &str, out: &mut String) {
    use std::fmt::Write;
    let mut kv: Vec<(&str, &str)> = st.iter().filter(|(k, _)| *k != skip).collect();
    kv.sort_unstable();
    let _ = write!(out, "|{}", kv.len());
    for (k, v) in kv {
        let _ = write!(out, ";{k}={v}");
    }
}

/// `affectsv` tri-état (étape 219) : `None` = **neutre par
/// construction** — la valeur est l'initiale spec de la prop
/// ou un mot-clé CSS-wide ; sa déclaration a pour effet spec
/// « identique à l'absence de déclaration » → aucun oracle
/// géométrique n'existe. Hors dénominateur (`na`), documenté
/// en note du rapport — sinon le score punissait les valeurs
/// sans effet possible.
///
/// `initial_value` (css) ne couvre qu'une partie du
/// catalogue — `INITIAL_EXTRA` complète par les initiales
/// spec des familles les plus visibles du lot BCD (oracle
/// vérifiable à la spec ; tout ce qui n'est pas initial
/// reste un miss honnête).
const INITIAL_EXTRA: &[(&str, &str)] = &[
    ("accent-color", "auto"),
    ("align-content", "normal"),
    ("align-items", "normal"),
    ("align-self", "auto"),
    ("alignment-baseline", "baseline"),
    ("anchor-name", "none"),
    ("anchor-scope", "none"),
    ("appearance", "none"),
    ("aspect-ratio", "auto"),
    ("backface-visibility", "visible"),
    ("baseline-shift", "baseline"),
    ("caret-color", "auto"),
    ("dominant-baseline", "alphabetic"),
    ("align-self", "normal"),
    ("align-self", "stretch"),
    ("justify-self", "normal"),
    ("justify-self", "stretch"),
    ("justify-items", "normal"),
    ("justify-items", "legacy"),
    ("clear", "none"),
    ("column-fill", "balance"),
    ("contain", "none"),
    ("content", "normal"),
    ("content-visibility", "visible"),
    ("direction", "ltr"),
    ("empty-cells", "show"),
    ("field-sizing", "fixed"),
    ("flex-wrap", "nowrap"),
    ("float", "none"),
    ("hyphens", "manual"),
    ("initial-letter", "normal"),
    ("text-underline-position", "auto"),
    ("text-decoration-inset", "auto"),
    ("text-underline-offset", "auto"),
    ("text-decoration-thickness", "auto"),
    ("text-box-trim", "none"),
    // `text-box` shorthand — un bord `<text-box-edge>` seul
    // (sans token `trim-*`) = trim `none` → inerte ; `none`/
    // `normal` = initiales spec.
    ("text-box", "none"),
    ("text-box", "normal"),
    ("text-box", "cap"),
    ("text-box", "text"),
    ("text-box", "ex"),
    ("text-box", "alphabetic"),
    ("text-box", "ideographic"),
    ("text-box", "ideographic-ink"),
    // `text-box-edge` sans trim actif = inerte (son ctx pose
    // `trim-both`, donc seule l'initiale `auto` reste `na`).
    ("text-box-edge", "auto"),
    // R-282 — `text-justify` : `auto` est l'initiale spec ;
    // `inter-word`/`inter-character`/`distribute` utilisent
    // tous la distribution inter-mots (borne PIPE : pas de
    // glu intra-mot) → déclaration ≡ baseline justifiée.
    ("text-justify", "auto"),
    ("text-justify", "inter-word"),
    ("text-justify", "inter-character"),
    ("text-justify", "distribute"),
    // `text-wrap-style` : `auto` initiale ; `stable` = garder
    // les césures stables au ré-écoulement — pas d'oracle
    // statique.
    ("text-wrap-style", "auto"),
    ("text-wrap-style", "stable"),
    // `text-decoration-skip-spaces` : l'implémentation ne
    // saute aucun blanc par défaut → `none` ≡ absent.
    ("text-decoration-skip-spaces", "none"),
    // `text-autospace` : opt-in — `auto`/`normal`/`no-autospace`
    // ≡ absent par construction.
    ("text-autospace", "auto"),
    ("text-autospace", "normal"),
    ("text-autospace", "no-autospace"),
    // R-286 — `text-decoration-skip-ink` : `auto` est
    // l'initiale spec (le moteur coupe déjà par défaut) ;
    // `all` produit les mêmes cuts que `auto` ici →
    // contextuellement neutre. `text-decoration-skip:auto`
    // = shorthand sur le même comportement par défaut.
    ("text-decoration-skip-ink", "auto"),
    ("text-decoration-skip-ink", "all"),
    ("text-decoration-skip", "auto"),
    // `text` ≡ `auto` (css-inline-3 : `auto` se résout en
    // bords texte — identique par construction).
    ("text-box-edge", "text"),
    // `hyphenate-*` : initiales spec css-text-4 (`auto` =
    // tiret par défaut / limite auto ; `no-limit` = pas de
    // borne de lignes consécutives).
    ("hyphenate-character", "auto"),
    ("hyphenate-limit-chars", "auto"),
    ("hyphenate-limit-lines", "no-limit"),
    ("quotes", "auto"),
    ("image-rendering", "auto"),
    ("isolation", "auto"),
    ("justify-content", "normal"),
    ("justify-items", "legacy"),
    ("justify-self", "auto"),
    ("letter-spacing", "normal"),
    ("line-break", "auto"),
    ("list-style-position", "outside"),
    ("mix-blend-mode", "normal"),
    ("object-fit", "fill"),
    ("order", "0"),
    ("overflow-wrap", "normal"),
    ("overscroll-behavior", "auto"),
    ("pointer-events", "auto"),
    ("position-try", "none"),
    ("position-try-fallbacks", "none"),
    // R-287 — `position-try-order` : `normal` = l'ordre
    // déclaré des fallbacks, initiale spec.
    ("position-try-order", "normal"),
    // R-289 — `ruby-overhang:auto` = initiale spec
    // (débordement autorisé — le défaut de l'impl).
    ("ruby-overhang", "auto"),
    ("resize", "none"),
    ("scroll-behavior", "auto"),
    ("table-layout", "auto"),
    ("text-align", "start"),
    ("text-orientation", "mixed"),
    ("text-wrap", "wrap"),
    ("transform", "none"),
    ("transform-style", "flat"),
    ("unicode-bidi", "normal"),
    ("user-select", "auto"),
    ("vertical-align", "baseline"),
    ("white-space", "normal"),
    ("word-break", "normal"),
    ("word-spacing", "normal"),
    ("writing-mode", "horizontal-tb"),
    // Cran « oracle » : équivalents-initiale confirmés par
    // mesure (`stretch` ≡ `normal` sur les alignements —
    // css-align : `normal` se comporte en `stretch`) et
    // contexte de pagination inexistant (valeurs `page`/
    // `left`/`right`/`paged_context` des `break-*` = no-op
    // hors média paginé → équivalent initiale ici).
    ("appearance", "auto"),
    ("align-content", "stretch"),
    ("align-items", "stretch"),
    ("backdrop-filter", "none"),
    ("break-after", "auto"),
    ("break-after", "page"),
    ("break-after", "left"),
    ("break-after", "right"),
    ("break-after", "paged_context"),
    ("break-before", "auto"),
    ("break-before", "page"),
    ("break-before", "left"),
    ("break-before", "right"),
    ("break-before", "paged_context"),
    ("break-inside", "auto"),
    ("break-inside", "avoid"),
    ("break-inside", "avoid-column"),
    ("break-inside", "avoid-page"),
    ("break-inside", "paged_context"),
    // Initiales `background-*` que `initial_value` ne
    // connaît pas + neutres de l'oracle gradient (`local`
    // ≡ `scroll` sans défilement, `cover`/`center` neutres
    // sur une image déjà calibrée à la zone, `currentColor`
    // ≡ `color` du contexte).
    ("background-attachment", "scroll"),
    ("background-attachment", "local"),
    ("background-blend-mode", "normal"),
    ("background-clip", "border-box"),
    ("background-color", "currentColor"),
    ("background-image", "none"),
    ("background-origin", "padding-box"),
    ("background-position", "0% 0%"),
    ("background-repeat", "repeat"),
    ("background-size", "auto"),
    ("background-size", "cover"),
    // `background-position-x`/`-y` : `left`/`top` = 0 % =
    // la position par défaut — neutre même avec une image
    // réduite ; `center`/`right`/`bottom` deviennent
    // discriminables dès que le ctx réduit l'image
    // (`no-repeat`+`size:40%`, étape 270) → retirés de `na`.
    ("background-position-x", "left"),
    ("background-position-y", "top"),
    // `flex-basis:auto` = initiale spec ; `justify-content:
    // flex-start` ≡ `normal` dans le contexte flex du
    // harnais (aucun espace à redistribuer différemment).
    ("flex-basis", "auto"),
    ("justify-content", "flex-start"),
    // Cellule : `text-top`/`text-bottom`/`sub`/`super`/
    // `<length>` retombent spec sur `baseline` (R-266
    // §17.5.3.1) = le défaut de la fixture cellule → neutre.
    ("vertical-align", "text-top"),
    ("vertical-align", "text-bottom"),
    ("vertical-align", "sub"),
    ("vertical-align", "super"),
    // `rule-break` (css-gaps-1 §4.2) : `intersection` est
    // l'initiale spec ; `spanning-item` produit le même
    // découpage au spanner dans la fixture multicol (aucun
    // oracle ne les distingue ici).
    ("rule-break", "intersection"),
    ("rule-break", "spanning-item"),
    // css-motion-1 : initiales spec + équivalents moteur
    // (`normal`/`auto` = repli ancre/position, `50% 50%` =
    // le centre implicite de la boîte 10×10 de la fixture).
    ("offset", "none"),
    ("offset-path", "none"),
    ("offset-rotate", "auto"),
    ("offset-rotate", "normal"),
    ("offset-anchor", "auto"),
    ("offset-anchor", "normal"),
    ("offset-anchor", "50% 50%"),
    ("offset-position", "normal"),
    ("offset-position", "auto"),
    // css-shapes-1 : `none` est l'initiale ; `margin-box` seul
    // = la boîte de marge du flottant — exactement le
    // contournement par défaut, neutre par construction.
    ("shape-outside", "none"),
    ("shape-outside", "margin-box"),
    // css-contain-3 : `normal`/`none` = initiales.
    ("container-type", "normal"),
    ("container-name", "none"),
    // Étape 270 — passe de qualification `css.values` :
    // `currentColor` résout à la couleur du texte, soit le
    // noir du texte ET des bordures/règles du ctx → inerte
    // par construction ici (même borne que `transparent`
    // sur une règle sans rangée).
    ("border-color", "currentColor"),
    ("border-top-color", "currentColor"),
    ("border-right-color", "currentColor"),
    ("border-bottom-color", "currentColor"),
    ("border-left-color", "currentColor"),
    ("border-block-color", "currentColor"),
    ("border-block-end-color", "currentColor"),
    ("border-block-start-color", "currentColor"),
    ("border-inline-color", "currentColor"),
    ("border-inline-end-color", "currentColor"),
    ("border-inline-start-color", "currentColor"),
    ("column-rule-color", "currentColor"),
    ("rule-color", "currentColor"),
    ("outline-color", "currentColor"),
    ("text-decoration-color", "currentColor"),
    ("text-emphasis-color", "currentColor"),
    ("text-emphasis-color", "transparent"),
    ("color", "currentColor"),
    // Styles `solid`/`underline` déjà posés par le ctx de
    // leur famille → la déclaration ne change rien.
    ("border-style", "solid"),
    ("border-top-style", "solid"),
    ("border-right-style", "solid"),
    ("border-bottom-style", "solid"),
    ("border-left-style", "solid"),
    ("outline-style", "solid"),
    ("column-rule-style", "solid"),
    ("rule-style", "solid"),
    ("text-decoration-style", "solid"),
    // `text-decoration-line` : initiale spec = `none`
    // (étape 302 — `underline` y était par erreur : la
    // valeur la plus courante du web partait en `na`,
    // jamais mesurée, et `none` était mesurée en vain).
    ("text-decoration-line", "none"),
    // Initiales spec absentes de `initial_value` (couvre
    // une partie du catalogue seulement) — chaque ligne
    // vérifiée à la spec de sa propriété.
    ("baseline-source", "auto"),
    ("block-size", "auto"),
    ("caption-side", "top"),
    ("color-interpolation", "auto"),
    ("color-interpolation-filters", "auto"),
    ("color-scheme", "normal"),
    ("column-height", "auto"),
    ("column-width", "auto"),
    ("column-gap", "normal"),
    ("row-gap", "normal"),
    ("gap", "normal"),
    ("content", "none"),
    ("counter-increment", "none"),
    ("counter-reset", "none"),
    ("counter-set", "none"),
    ("contain-intrinsic-size", "none"),
    ("contain-intrinsic-width", "none"),
    ("contain-intrinsic-height", "none"),
    ("contain-intrinsic-block-size", "none"),
    ("contain-intrinsic-inline-size", "none"),
    ("continue", "normal"),
    ("filter", "none"),
    ("flex-direction", "row"),
    ("flex-flow", "row"),
    ("flex-flow", "nowrap"),
    ("flex-wrap", "nowrap"),
    ("font-feature-settings", "normal"),
    ("font-kerning", "auto"),
    ("font-language-override", "normal"),
    ("font-optical-sizing", "auto"),
    ("font-palette", "normal"),
    ("font-size-adjust", "none"),
    ("font-stretch", "normal"),
    ("font-width", "normal"),
    ("font-synthesis", "none"),
    ("font-synthesis-position", "auto"),
    ("font-synthesis-small-caps", "auto"),
    ("font-synthesis-style", "auto"),
    ("font-synthesis-weight", "auto"),
    ("font-variant-alternates", "normal"),
    ("font-variant-caps", "normal"),
    ("font-variant-east-asian", "normal"),
    ("font-variant-emoji", "normal"),
    ("font-variant-ligatures", "normal"),
    ("font-variant-numeric", "normal"),
    ("font-variant-position", "normal"),
    ("font-variation-settings", "normal"),
    ("grid", "none"),
    ("grid-area", "auto"),
    ("grid-auto-columns", "auto"),
    ("grid-auto-flow", "row"),
    ("grid-auto-rows", "auto"),
    ("grid-column", "auto"),
    ("grid-column-end", "auto"),
    ("grid-column-start", "auto"),
    ("grid-row", "auto"),
    ("grid-row-end", "auto"),
    ("grid-row-start", "auto"),
    ("grid-template", "none"),
    ("grid-template-areas", "none"),
    ("grid-template-columns", "none"),
    ("grid-template-rows", "none"),
    ("hanging-punctuation", "none"),
    ("inline-size", "auto"),
    ("inset", "auto"),
    ("inset-block", "auto"),
    ("inset-block-end", "auto"),
    ("inset-block-start", "auto"),
    ("inset-inline", "auto"),
    ("inset-inline-end", "auto"),
    ("inset-inline-start", "auto"),
    ("line-clamp", "none"),
    ("list-style", "outside"),
    ("margin-trim", "none"),
    ("min-block-size", "auto"),
    ("min-inline-size", "auto"),
    ("object-view-box", "none"),
    // `offset-anchor:center` — `auto` (initiale) se
    // comporte comme `center` pour un chemin (css-motion-1
    // §6.3) : même résolution, inerte.
    ("offset-anchor", "center"),
    ("perspective", "none"),
    ("place-content", "normal"),
    ("place-items", "normal"),
    ("place-self", "normal"),
    ("position-anchor", "auto"),
    ("position-area", "normal"),
    ("rotate", "none"),
    ("scale", "none"),
    ("translate", "none"),
    ("rule-break", "none"),
    ("column-rule-break", "none"),
    ("scroll-padding", "auto"),
    ("scroll-snap-align", "none"),
    ("scroll-snap-stop", "normal"),
    ("scroll-snap-type", "none"),
    ("scrollbar-color", "auto"),
    ("scrollbar-gutter", "auto"),
    ("scrollbar-width", "auto"),
    ("text-align", "left"),
    ("text-align-last", "auto"),
    ("text-box", "auto"),
    ("text-decoration", "none"),
    ("text-emphasis", "none"),
    ("text-emphasis-style", "none"),
    ("text-shadow", "none"),
    ("text-wrap-mode", "wrap"),
    ("transform-box", "view-box"),
    ("transform-origin", "center"),
    ("white-space-collapse", "collapse"),
    ("white-space-trim", "none"),
    ("window-drag", "none"),
    // Inertes par construction (pas par initiale) :
    // `background-size:contain` sur un dégradé sans taille
    // intrinsèque ≡ `auto` ; `clip-path:<box>` seul est un
    // clip à la boîte — `border-box`/`view-box`/`margin-box`
    // ⊇ la boîte de bordure → aucun clip ; `fill-box`/
    // `stroke-box` = référentiels SVG ; `position:relative`/
    // `sticky` sans offsets ni scroll = géométrie identique ;
    // `position-anchor:none` = pas d'ancre implicite ;
    // `text-decoration:<style|color>` seul ne pose aucune
    // ligne → rien ne peint.
    ("background-size", "contain"),
    ("clip-path", "none"),
    ("clip-path", "border-box"),
    ("clip-path", "view-box"),
    ("clip-path", "margin-box"),
    ("clip-path", "fill-box"),
    ("clip-path", "stroke-box"),
    ("position", "relative"),
    ("position", "sticky"),
    ("position-anchor", "none"),
    ("position-anchor", "normal"),
    ("position-area", "none"),
    ("position-visibility", "always"),
    ("interpolate-size", "numeric-only"),
    ("text-decoration", "auto"),
    // `vertical-align` à portée SVG (lignes de base de la
    // spec SVG `central`/`hanging`/`mathematical`/
    // `ideographic`/`alphabetic`, et noms BCD non lexèmes
    // `first`/`last`/`center`) — inertes sur inline HTML,
    // même classe que `dominant-baseline`.
    ("vertical-align", "alphabetic"),
    ("vertical-align", "central"),
    ("vertical-align", "hanging"),
    ("vertical-align", "ideographic"),
    ("vertical-align", "mathematical"),
    ("vertical-align", "center"),
    ("vertical-align", "first"),
    ("vertical-align", "last"),
    // `row-rule-style:hidden`/`none` — css-gaps : `hidden`
    // peint comme `none` (rien de visible), `none` est
    // l'initiale — inertes même avec des rangées.
    ("row-rule-style", "hidden"),
    ("row-rule-style", "none"),
    ("row-rule-width", "medium"),
    ("row-rule-break", "none"),
    ("row-rule-break", "normal"),
    // Initiales spec diverses du reliquat mesuré.
    ("max-lines", "none"),
    ("block-ellipsis", "no-ellipsis"),
    ("text-combine-upright", "none"),
    ("text-spacing-trim", "normal"),
    ("border-image-source", "none"),
    ("border-image-width", "auto"),
    ("border-shape", "none"),
    ("box-decoration-break", "slice"),
    ("white-space", "none"),
    // Étape 272 — passe de qualification `css.values`,
    // tranche 2 : paires **inertes par spec** ou
    // **équivalentes au contexte** mesurées au re-probe.
    //
    // `offset` shorthand : toute valeur hors `<offset-path>`
    // remet le chemin à `none` — `offset-position`/
    // `offset-anchor`/`offset-rotate` n'ont pas d'effet
    // sans chemin posé (css-motion-1 §3) → inerte.
    ("offset", "auto"),
    ("offset", "normal"),
    ("offset", "center"),
    ("offset", "top"),
    ("offset", "right"),
    ("offset", "bottom"),
    ("offset", "left"),
    ("offset", "x-start"),
    ("offset", "x-end"),
    ("offset", "y-start"),
    ("offset", "y-end"),
    ("offset", "border-box"),
    ("offset", "content-box"),
    ("offset", "fill-box"),
    ("offset", "margin-box"),
    ("offset", "padding-box"),
    ("offset", "stroke-box"),
    ("offset", "view-box"),
    // `offset-path:<coord-box>` seul = référentiel de
    // coordonnées **sans chemin** — la boîte ne part sur
    // aucun trajet (inerte par spec).
    ("offset-path", "border-box"),
    ("offset-path", "content-box"),
    ("offset-path", "fill-box"),
    ("offset-path", "margin-box"),
    ("offset-path", "padding-box"),
    ("offset-path", "stroke-box"),
    ("offset-path", "view-box"),
    // `transform-box` `fill-box`/`stroke-box` : référentiels
    // d'objet **SVG** — sur un élément HTML ≈ `border-box`
    // (`view-box` = initiale, `border-box` = le référentiel
    // par défaut sur non-SVG → même classe ; `content-box`/
    // `padding-box` deviennent mesurables avec la fixture
    // asymétrique).
    ("transform-box", "fill-box"),
    ("transform-box", "stroke-box"),
    ("transform-box", "border-box"),
    // `vertical-align:top` = l'alignement par défaut du
    // moteur en cellule (`middle`/`bottom` diffèrent, `top`
    // coïncide — même cran que `text-top`/`sub`/`super`).
    ("vertical-align", "top"),
    // `font:<système>` — famille et métriques choisies par
    // l'UA, aucun oracle statique (même classe que
    // `appearance`).
    ("font", "caption"),
    ("font", "icon"),
    ("font", "menu"),
    ("font", "message-box"),
    ("font", "small-caption"),
    ("font", "status-bar"),
    // `font-weight:lighter` → 400 = le défaut
    // (approximation documentée — la pondération relative
    // n'est pas lue, `bolder` → 700 reste mesuré).
    ("font-weight", "lighter"),
    // `justify-content:left`/`start`/`stretch` ≡
    // `flex-start` en flex LTR (css-align §8.1 — même
    // repli que le défaut du contexte). `safe`/`unsafe` :
    // le préfixe ne change le repli qu'en débordement —
    // la fixture flex mono-ligne n'en produit pas →
    // équivalent à la position nue (la mesure reste
    // possible sur `align-content`, ctx multi-lignes).
    ("justify-content", "left"),
    ("justify-content", "start"),
    ("justify-content", "stretch"),
    ("justify-content", "safe"),
    ("justify-content", "unsafe"),
    // `text-wrap` : `auto` initiale ; `stable` ne sert
    // qu'en région éditable (stabilité des césures à
    // l'édition) — hors harnais.
    ("text-wrap", "auto"),
    ("text-wrap", "stable"),
    // Initiales spec `math-*` (famille parsée, effet non
    // porté — les valeurs non initiales restent `miss`
    // honnêtes = trou moteur au journal).
    ("math-style", "normal"),
    ("math-shift", "normal"),
    ("math-depth", "auto-add"),
    // `position-try{,-fallbacks}` : l'aire `top` ≡ l'aire
    // de base `top` de la fixture — le fallback reproduit
    // la position courante (horizontal-tb :
    // `block-start`/`y-start` = `top`).
    ("position-try", "top"),
    ("position-try", "block-start"),
    ("position-try", "y-start"),
    ("position-try", "self-block-start"),
    ("position-try", "span-block-start"),
    ("position-try", "span-top"),
    ("position-try", "span-y-start"),
    ("position-try", "span-self-block-start"),
    // `flip-inline`/`flip-x` = miroir autour du centre de
    // l'ancre — la boîte `position-area:top` y est déjà
    // centrée → delta nul par construction (no-op spec,
    // pas un gap : `flip-start` qui mire l'autre axe est
    // mesuré `ok`).
    ("position-try", "flip-inline"),
    ("position-try", "flip-x"),
    ("position-try-fallbacks", "top"),
    ("position-try-fallbacks", "block-start"),
    ("position-try-fallbacks", "y-start"),
    ("position-try-fallbacks", "self-block-start"),
    ("position-try-fallbacks", "span-block-start"),
    ("position-try-fallbacks", "span-top"),
    ("position-try-fallbacks", "span-y-start"),
    ("position-try-fallbacks", "span-self-block-start"),
    ("position-try-fallbacks", "flip-inline"),
    ("position-try-fallbacks", "flip-x"),
    // `grid-template-*:auto` ≡ la piste implicite `auto`
    // de la fixture mono-axe (même placement).
    ("grid-template-columns", "auto"),
    ("grid-template-rows", "auto"),
    // `shape-outside:gradient` — la forme dépend de
    // l'alpha d'une **image** ; aucune bitmap au harnais
    // (même borne que `mask-*`).
    ("shape-outside", "gradient"),
    // `rule` shorthand : `currentColor`/`solid` ≡ les
    // valeurs déjà posées par le ctx `column-rule`
    // (la remise à zéro spec des volets omis est une
    // borne moteur — journal équipe).
    ("rule", "currentColor"),
    ("rule", "solid"),
    // `row-rule`/`rule-color`/`row-rule-color` : valeurs
    // **sans trait visible** même implémentées — `none`/
    // `hidden` initiales, `transparent` peint invisible,
    // `currentColor` = la couleur par défaut. Les
    // valeurs qui peindraient un trait restent `miss`
    // (expansion sans émission — trou moteur, journal).
    ("row-rule", "none"),
    ("row-rule", "hidden"),
    ("row-rule", "transparent"),
    ("row-rule", "currentColor"),
    ("rule", "transparent"),
    ("rule-color", "transparent"),
    ("row-rule-color", "transparent"),
    ("row-rule-color", "currentColor"),
    // `rule-break:normal` = le comportement par défaut ;
    // `row-rule-break` : initiale `intersection` et
    // équivalent `spanning-item` (même cran que
    // `rule-break` — `none` reste un miss honnête :
    // traverser les spanners est l'effet réel manquant).
    ("rule-break", "normal"),
    ("row-rule-break", "intersection"),
    ("row-rule-break", "spanning-item"),
    // `transform:3d` (sous-feature BCD) — les transformées
    // 3D sont non-affines : aucun oracle dans le dump
    // affine (borne `perspective`, NO_ORACLE).
    ("transform", "3d"),
];

/// Propriétés **non observables** par l'oracle géométrique
/// statique (`affectsv`) — l'effet vit hors du box tree et
/// du dump display-list (curseur, caret, hit-test, scroll
/// dynamique, widget UA, pagination). `na` honnête, pas un
/// « miss » : aucun dump ne peut discriminer la valeur.
const NO_ORACLE: &[&str] = &[
    "cursor",
    "caret-color",
    "accent-color",
    "appearance",
    "pointer-events",
    "resize",
    "touch-action",
    "user-select",
    "touch-callout",
    "tap-highlight-color",
    "scroll-behavior",
    "overscroll-behavior",
    "overscroll-behavior-x",
    "overscroll-behavior-y",
    "overscroll-behavior-block",
    "overscroll-behavior-inline",
    "overflow-anchor",
    "image-rendering",
    "shape-rendering",
    "text-rendering",
    "color-rendering",
    "vector-effect",
    "will-change",
    "forced-color-adjust",
    "print-color-adjust",
    "color-adjust",
    "orphans",
    "widows",
    "page",
    "speak-as",
    "backface-visibility",
    "anchor-scope",
    // `dominant-baseline` : prop à portée SVG (texte SVG) —
    // aucun effet en layout bloc/inline HTML, pas d'oracle.
    "dominant-baseline",
    // `mask-*` : la couche mask n'est résolue que depuis une
    // bitmap `url()` fetchée (`resolve_mask` — `PaintImages`
    // vide au harnais, les gradients ne passent pas par ce
    // chemin) — aucune valeur observable sans ressource
    // externe.
    "mask",
    "mask-image",
    "mask-mode",
    "mask-repeat",
    "mask-position",
    "mask-position-x",
    "mask-position-y",
    "mask-size",
    "mask-clip",
    "mask-origin",
    "mask-composite",
    "mask-border",
    "mask-border-repeat",
    "mask-border-slice",
    "mask-border-source",
    "mask-border-width",
    "mask-border-outset",
    "mask-type",
    // `shape-image-threshold` : seuil alpha des formes image
    // (`url()`/`gradient`) — sources non analysées au harnais
    // (même borne que `mask-*`), aucun oracle statique.
    "shape-image-threshold",
    // `container-name` : métadonnée pure de nommage pour les
    // queries `@container` — aucun effet géométrique propre
    // (le prédicat s'évalue dans la règle, pas la déclaration).
    "container-name",
    // Peinture/géométrie **à portée SVG** (svg2 §geometry /
    // svg11 §painting) : l'effet vit dans l'îlot rasterisé
    // `<svg>` (atome remplacé — son bitmap n'entre pas dans
    // le dump display-list) ; déclarée sur un élément HTML
    // elle est inerte par spec. Même classe que
    // `dominant-baseline` : conformité prouvée par les
    // verrous PIPE (R-283 `collect_svg_styled`), aucun
    // oracle statique ici.
    "fill",
    "fill-opacity",
    "fill-rule",
    "stroke",
    "stroke-width",
    "stroke-opacity",
    "stroke-dasharray",
    "stroke-dashoffset",
    "stroke-linecap",
    "stroke-linejoin",
    "stroke-miterlimit",
    "paint-order",
    "d",
    "cx",
    "cy",
    "r",
    "rx",
    "ry",
    "x",
    "y",
    "stop-color",
    "stop-opacity",
    "flood-color",
    "flood-opacity",
    "lighting-color",
    "marker",
    "marker-start",
    "marker-mid",
    "marker-end",
    // `interactivity` (R-285) : exclusion du hit-test —
    // aucun effet géométrique ni display-list, même classe
    // que `pointer-events` (NO_ORACLE).
    "interactivity",
    // Étape 270 — familles mesurées **hors oracle** par la
    // passe de qualification `css.values` :
    // Contenu remplacé sans bitmap — `PaintImages` vide au
    // harnais, aucune image intrinsèque ne peut être servie
    // (`object-*` dimensionnent/positionnent le bitmap
    // remplacé ; même borne que `mask-*`).
    "object-fit",
    "object-position",
    "object-view-box",
    // `size` — boîte de page `@page` (css-page) : la
    // pagination n'entre pas dans le dump.
    "size",
    // Props de texte à portée SVG — même classe que
    // `dominant-baseline` (îlot `<svg>` rasterisé à part).
    "alignment-baseline",
    "baseline-shift",
    "clip-rule",
    "stroke-color",
    "text-anchor",
    "path-length",
    // Curseur/saisie — présentation interactive, aucun
    // effet de layout statique.
    "caret-animation",
    "caret-shape",
    "window-drag",
    // Délai d'intérêt (`interest-target`) — temporisation
    // d'interaction, hors géométrie.
    "interest-delay",
    "interest-delay-start",
    "interest-delay-end",
    // Méta de navigation — aucun effet de rendu.
    "link-parameters",
    // Snap/scroll dynamiques — l'effet ne se joue qu'en
    // défilement, jamais dans un dump statique.
    "scroll-snap-align",
    "scroll-snap-stop",
    "scroll-snap-type",
    "scroll-padding",
    "scroll-padding-top",
    "scroll-padding-right",
    "scroll-padding-bottom",
    "scroll-padding-left",
    "scroll-padding-block",
    "scroll-padding-block-start",
    "scroll-padding-block-end",
    "scroll-padding-inline",
    "scroll-padding-inline-start",
    "scroll-padding-inline-end",
    "scroll-axis-lock",
    "scroll-initial-target",
    "scroll-marker-group",
    "scroll-target-group",
    // Barres de défilement — chrome UA, jamais peintes
    // dans le framebuffer.
    "scrollbar-color",
    "scrollbar-gutter",
    "scrollbar-width",
    // Timelines/déclencheurs/transitions de vue —
    // famille temporelle (horloge + état animé requis).
    "scroll-timeline",
    "scroll-timeline-axis",
    "scroll-timeline-name",
    "timeline-scope",
    "timeline-trigger",
    "timeline-trigger-activation-range",
    "timeline-trigger-activation-range-end",
    "timeline-trigger-activation-range-start",
    "timeline-trigger-active-range",
    "timeline-trigger-active-range-end",
    "timeline-trigger-active-range-start",
    "timeline-trigger-name",
    "timeline-trigger-source",
    "trigger-scope",
    "view-timeline",
    "view-timeline-axis",
    "view-timeline-inset",
    "view-timeline-name",
    "view-transition-class",
    "view-transition-group",
    "view-transition-name",
    "view-transition-scope",
    // `overlay` — top-layer popover, hors arbre de boîtes
    // statique.
    "overlay",
    // `text-size-adjust` — le moteur n'a pas de passe
    // d'ajustement automatique (mobile text autosize) :
    // toute valeur est inerte ici.
    "text-size-adjust",
    // `image-orientation` — redressement EXIF : sans
    // bitmap décodé aucune orientation n'est observable.
    "image-orientation",
    // `dynamic-range-limit` — gamme HDR, hors colorimétrie
    // du dump.
    "dynamic-range-limit",
    // Famille **sélection/métriques de glyphes** : la fonte
    // du harnais (Ahem unique) n'a ni variantes, ni tables
    // OpenType, ni palettes — même une implémentation
    // complète de ces props ne changerait aucune mesure
    // ici (borne de fixture, pas verdict moteur — la
    // conformité se mesure sur les verrous texte).
    "font-kerning",
    "font-optical-sizing",
    "font-palette",
    "font-synthesis",
    "font-synthesis-position",
    "font-synthesis-small-caps",
    "font-synthesis-style",
    "font-synthesis-weight",
    "font-variant-alternates",
    "font-variant-emoji",
    "font-feature-settings",
    "font-variation-settings",
    "font-language-override",
    "font-width",
    "font-stretch",
    // Étape 272 — `perspective`/`perspective-origin` : la
    // projection est **non-affine** (perspective différée
    // documentée, `transform.rs`) — aucun oracle possible
    // dans un dump affine, même implémentée.
    "perspective",
    "perspective-origin",
    // `interpolate-size` : opt-in d'interpolation vers les
    // mots-clés de sizing (`height:auto` transitable) —
    // l'effet n'existe que sous horloge d'animation, même
    // classe que la famille `animation-*`/`transition-*`.
    "interpolate-size",
    // `border-image*` : le `DisplayItem::BorderImage`
    // (R-248) n'est émis que depuis une bitmap
    // `borders_url` décodée — vide au harnais (`PaintImages`
    // vide), aucune déclaration de la famille n'est
    // observable (même borne que `mask-*`).
    "border-image",
    "border-image-slice",
    "border-image-width",
    "border-image-outset",
    "border-image-repeat",
    "border-image-source",
];

/// Sous-ensemble de [`NO_ORACLE`] encore **inobservable en
/// pixels** (étape 274) : l'oracle `paintv` voit ce que le
/// dump ne voyait pas — les familles ci-dessous y restent
/// aveugles par construction (interaction/OS, défilement
/// dynamique, horloge, pagination, métadonnées, métriques de
/// fontes avec Ahem unique) ou faute de fixture dédiée
/// (texte SVG, markers, primitives de filtre — requalifiable
/// au fil des fixtures).
const PAINT_NA: &[&str] = &[
    // Chrome OS / interaction — jamais dans le framebuffer.
    "cursor",
    "caret-color",
    "caret-animation",
    "caret-shape",
    "accent-color",
    "appearance",
    "pointer-events",
    "resize",
    "touch-action",
    "user-select",
    "touch-callout",
    "tap-highlight-color",
    "window-drag",
    "interactivity",
    "interest-delay",
    "interest-delay-start",
    "interest-delay-end",
    "link-parameters",
    // Défilement dynamique — l'effet n'existe qu'en scroll.
    "scroll-behavior",
    "overscroll-behavior",
    "overscroll-behavior-x",
    "overscroll-behavior-y",
    "overscroll-behavior-block",
    "overscroll-behavior-inline",
    "overflow-anchor",
    "scroll-snap-align",
    "scroll-snap-stop",
    "scroll-snap-type",
    "scroll-padding",
    "scroll-padding-top",
    "scroll-padding-right",
    "scroll-padding-bottom",
    "scroll-padding-left",
    "scroll-padding-block",
    "scroll-padding-block-start",
    "scroll-padding-block-end",
    "scroll-padding-inline",
    "scroll-padding-inline-start",
    "scroll-padding-inline-end",
    "scroll-axis-lock",
    "scroll-initial-target",
    "scroll-marker-group",
    "scroll-target-group",
    // Barres de défilement — chrome UA.
    "scrollbar-color",
    "scrollbar-gutter",
    "scrollbar-width",
    // Timelines/déclencheurs/transitions de vue —
    // famille temporelle (horloge requise).
    "scroll-timeline",
    "scroll-timeline-axis",
    "scroll-timeline-name",
    "timeline-scope",
    "timeline-trigger",
    "timeline-trigger-activation-range",
    "timeline-trigger-activation-range-end",
    "timeline-trigger-activation-range-start",
    "timeline-trigger-active-range",
    "timeline-trigger-active-range-end",
    "timeline-trigger-active-range-start",
    "timeline-trigger-name",
    "timeline-trigger-source",
    "trigger-scope",
    "view-timeline",
    "view-timeline-axis",
    "view-timeline-inset",
    "view-timeline-name",
    "view-transition-class",
    "view-transition-group",
    "view-transition-name",
    "view-transition-scope",
    "interpolate-size",
    "overlay",
    // Pagination / modes forcés / métadonnées / hints.
    "size",
    "orphans",
    "widows",
    "page",
    "speak-as",
    "forced-color-adjust",
    "print-color-adjust",
    "color-adjust",
    "will-change",
    "dynamic-range-limit",
    "text-size-adjust",
    "image-orientation",
    "container-name",
    "anchor-scope",
    // Raster hints sans discriminant au harnais — le lissage
    // ne distingue pas les modes sur nos formes de test
    // (`text-rendering` exige des tables OT absentes d'Ahem,
    // `color-rendering` un chemin ICC).
    "text-rendering",
    "color-rendering",
    // Famille **sélection/métriques de glyphes** — Ahem
    // unique n'a ni variantes, ni tables OpenType, ni
    // palettes : même implémentée, aucune valeur ne change
    // le framebuffer ici (borne de fixture — verrous texte).
    "font-kerning",
    "font-optical-sizing",
    "font-palette",
    "font-synthesis",
    "font-synthesis-position",
    "font-synthesis-small-caps",
    "font-synthesis-style",
    "font-synthesis-weight",
    "font-variant-alternates",
    "font-variant-emoji",
    "font-feature-settings",
    "font-variation-settings",
    "font-language-override",
    "font-width",
    "font-stretch",
    // Portée SVG sans fixture dédiée en v1 — texte SVG
    // (baselines/ancre = métriques usvg), markers et
    // primitives de filtre (`flood`/`lighting` vivent dans
    // des nœuds de filtre), `clip-rule` exige un
    // `<clipPath>` contextuel. `na` honnête — pas un trou
    // mesuré (requalifiable quand la fixture arrive).
    "dominant-baseline",
    "alignment-baseline",
    "baseline-shift",
    "clip-rule",
    "text-anchor",
    "path-length",
    "stroke-color",
    "marker",
    "marker-start",
    "marker-mid",
    "marker-end",
    "flood-color",
    "flood-opacity",
    "lighting-color",
    // Étape 302 — `perspective`/`perspective-origin` : la
    // borne « non-affine » du dump vaut aussi en pixels
    // (étape 272), et la perspective n'existe que sur des
    // enfants en transformée **3D** — eux-mêmes hors-
    // oracle (`transform:3d`, INITIAL_EXTRA) : aucune
    // fixture sondable ne peut la rendre visible. Les 10
    // paires mesuraient la borne, pas un trou (triage
    // fixtures-inertes 2026-11-16).
    "perspective",
    "perspective-origin",
];

/// Verdict de [`subfeature_probe`] (étape 270).
enum Subfeature {
    /// Sonder `name:lit` à la place de la valeur brute.
    Probe(&'static str),
    /// `na` honnête : aucun littéral mesurable au harnais
    /// (portée SVG, horloge, ressource externe ou variante
    /// d'élément).
    Na,
}

/// `(prop, valeur BCD) → partie valeur de la déclaration
/// sonde` — trié par prop pour la lecture.
const SUBFEATURE_PROBE: &[(&str, &str, &str)] = &[
    (
        "background",
        "background-clip",
        "linear-gradient(red,blue) padding-box",
    ),
    (
        "background",
        "background-origin",
        "linear-gradient(red,blue) content-box",
    ),
    (
        "background",
        "background-size",
        "linear-gradient(red,blue) 0 0/40%",
    ),
    (
        "background",
        "border-area",
        "linear-gradient(red,blue) border-area",
    ),
    (
        "background",
        "multiple_backgrounds",
        "linear-gradient(red,blue),linear-gradient(green,yellow)",
    ),
    ("background", "round", "linear-gradient(red,blue) round"),
    ("background", "space", "linear-gradient(red,blue) space"),
    ("background", "text", "linear-gradient(red,blue) text"),
    ("background", "x-end", "linear-gradient(red,blue) x-end"),
    ("background", "x-start", "linear-gradient(red,blue) x-start"),
    ("background", "y-end", "linear-gradient(red,blue) y-end"),
    ("background", "y-start", "linear-gradient(red,blue) y-start"),
    (
        "background-attachment",
        "multiple_backgrounds",
        "scroll,fixed",
    ),
    ("background-position", "multiple_backgrounds", "top,bottom"),
    (
        "background-position",
        "side-relative_values",
        "left 10px top",
    ),
    ("background-position-x", "side-relative_values", "x-start"),
    ("background-position-y", "side-relative_values", "y-start"),
    ("background-repeat", "2-value", "repeat space"),
    (
        "background-repeat",
        "multiple_backgrounds",
        "repeat,no-repeat",
    ),
    (
        "border-bottom-left-radius",
        "elliptical_corners",
        "10px 20px",
    ),
    (
        "border-bottom-right-radius",
        "elliptical_corners",
        "10px 20px",
    ),
    ("border-image", "gradient", "linear-gradient(red,blue) 30"),
    (
        "border-image",
        "optional_border_image_slice",
        "linear-gradient(red,blue) 30 fill",
    ),
    ("border-image", "fill", "linear-gradient(red,blue) 30 fill"),
    ("border-radius", "4_values_for_4_corners", "1px 2px 3px 4px"),
    ("border-radius", "elliptical_borders", "10px 20px"),
    ("border-top-left-radius", "elliptical_corners", "10px 20px"),
    ("border-top-right-radius", "elliptical_corners", "10px 20px"),
    ("box-shadow", "inset", "inset 2px 2px red"),
    ("box-shadow", "multiple_shadows", "2px 2px red,4px 4px blue"),
    ("box-shadow", "spread_radius", "2px 2px 2px 6px red"),
    ("break-inside", "multicol_context", "avoid"),
    ("clip-path", "basic_shape", "circle(30%)"),
    ("clip-path", "path", "path(\"M0 0h20v20z\")"),
    ("column-gap", "flex_context", "20px"),
    ("column-rule-color", "multiple_colors", "red blue"),
    ("column-rule-color", "repeat", "repeat(2,red)"),
    ("contain-intrinsic-size", "auto_none", "auto 40px"),
    ("content", "alt_text", "\"x\" / \"alt\""),
    ("content", "gradient", "linear-gradient(red,blue)"),
    ("counter-increment", "list-item", "list-item 1"),
    ("counter-reset", "list-item", "list-item 1"),
    ("counter-set", "list-item", "list-item 1"),
    ("display", "multi-keyword_values", "inline flow-root"),
    ("filter", "blur", "blur(2px)"),
    ("filter", "brightness", "brightness(1.5)"),
    ("filter", "contrast", "contrast(1.5)"),
    ("filter", "drop-shadow", "drop-shadow(2px 2px red)"),
    ("filter", "grayscale", "grayscale(1)"),
    ("filter", "hue-rotate", "hue-rotate(90deg)"),
    ("filter", "invert", "invert(1)"),
    ("filter", "opacity", "opacity(0.5)"),
    ("filter", "saturate", "saturate(2)"),
    ("filter", "sepia", "sepia(1)"),
    ("font", "font-width_keyword_values", "condensed 16px serif"),
    ("font-size", "rem_values", "2rem"),
    ("font-size-adjust", "two-values", "0.5 0.4"),
    ("font-style", "oblique-angle", "oblique 20deg"),
    ("font-variant", "css_fonts_shorthand", "small-caps"),
    ("font-variant-alternates", "annotation", "annotation(x)"),
    (
        "font-variant-alternates",
        "character_variant",
        "character-variant(x)",
    ),
    ("font-variant-alternates", "ornaments", "ornaments(1)"),
    ("font-variant-alternates", "styleset", "styleset(1)"),
    ("font-variant-alternates", "stylistic", "stylistic(1)"),
    ("font-variant-alternates", "swash", "swash(1)"),
    ("font-weight", "number", "500"),
    ("gap", "flex_context", "30px"),
    // `repeat`/`minmax`/`fit-content` ne sont pas des
    // littéraux seuls dans les pistes grid — sondes
    // fonctionnelles (`fit-content` nu n'est valide que
    // hors `grid-template-*`).
    ("grid-template-columns", "repeat", "repeat(2,50px)"),
    ("grid-template-rows", "repeat", "repeat(2,50px)"),
    ("grid-template-columns", "minmax", "minmax(20px,60px)"),
    ("grid-template-rows", "minmax", "minmax(20px,60px)"),
    ("grid-template-columns", "fit-content", "fit-content(60px)"),
    ("grid-template-rows", "fit-content", "fit-content(60px)"),
    ("justify-content", "context_flex", "center"),
    ("position-try-fallbacks", "position-area", "bottom"),
    ("letter-spacing", "percentages", "10%"),
    // `line-clamp:ellipsis` (sous-feature BCD) = « le
    // clamp produit une ellipse » — la sonde honnête est
    // le clamp entier, dont l'effet est la troncature
    // ellipsée mesurée ok (étape 302).
    ("line-clamp", "ellipsis", "2"),
    ("line-clamp", "string", "\"x\""),
    ("offset-path", "basic_shape", "circle(30%)"),
    ("offset-path", "path", "path(\"M0 0h10v10z\")"),
    ("offset-path", "ray", "ray(45deg)"),
    ("opacity", "percentages", "50%"),
    ("overflow", "multiple_keywords", "hidden scroll"),
    ("rotate", "x_y_z_angle", "x 30deg"),
    ("row-gap", "flex_context", "15px"),
    ("shape-outside", "circle", "circle(40%)"),
    ("shape-outside", "inset", "inset(10px)"),
    ("shape-outside", "path", "path(\"M0 0h10v10z\")"),
    ("shape-outside", "polygon", "polygon(0 0,100% 0,0 100%)"),
    ("shape-outside", "rect", "rect(0 10px 10px 0)"),
    ("shape-outside", "xywh", "xywh(0 0 50% 50%)"),
    ("tab-size", "length", "12px"),
    (
        "text-decoration",
        "includes_color-and-style",
        "underline red dashed",
    ),
    ("text-decoration", "includes_thickness", "underline 2px"),
    ("text-overflow", "string", "\"…\""),
    ("text-overflow", "two_value_syntax", "clip ellipsis"),
    ("transform", "3d", "rotateX(30deg)"),
    ("transform-origin", "three_value_syntax", "10px 20px 5px"),
    ("word-spacing", "percentages", "20%"),
    ("z-index", "negative_values", "-1"),
];

/// Noms génériques → `na` honnête : portée SVG/élément,
/// horloge, ressource externe, ou variante de fixture —
/// aucun littéral mesurable n'existe ici.
const SUBFEATURE_NA: &[&str] = &[
    "-webkit-legacy",
    "absolutely_positioned_flex_children",
    "anchor-valid",
    "animation",
    "animation_computed",
    "dynamic-range-limit-mix_values",
    "from-font",
    "from-image",
    "html_elements",
    "image",
    "image-set",
    "inline_elements",
    "is_animatable",
    "name-only_queries",
    "palette-mix_function",
    "position_after_layout",
    "position_sticky_table_elements",
    "replaced_elements",
    "reset_does_not_affect_siblings",
    "scroll-state",
    "SVG_image_as_background",
    "svg_elements",
    "textarea_support",
    "url",
    "weak_containment",
];

/// Sous-feature BCD **nommée** → littéral de sonde ou `na`
/// (étape 270). Les noms BCD comme `basic_shape`,
/// `multiple_backgrounds` ou `is_animatable` désignent un
/// *cas de support*, pas un lexème CSS : sondés en brut ils
/// mesurent une déclaration invalide — faux miss.
/// `None` = pas une sous-feature connue — le brut est sondé
/// normalement.
fn subfeature_probe(name: &str, v: &str) -> Option<Subfeature> {
    if let Some(&(_, _, lit)) = SUBFEATURE_PROBE
        .iter()
        .find(|&&(p, vv, _)| p == name && vv == v)
    {
        return Some(Subfeature::Probe(lit));
    }
    // `anchor()`/`anchor-size()` — même sous-feature sur
    // toute la famille inset/margin : la fixture ancre
    // est posée pour ces props (voir `fixture_for`).
    // `margin` (shorthand) complète ses longhands
    // (étape 297 — la paire BCD `margin:anchor-size`
    // tombait en `decl:` et mesurait le mot-clé nu).
    if matches!(
        name,
        "top"
            | "right"
            | "bottom"
            | "left"
            | "inset"
            | "inset-block"
            | "inset-block-end"
            | "inset-block-start"
            | "inset-inline"
            | "inset-inline-end"
            | "inset-inline-start"
            | "margin"
            | "margin-top"
            | "margin-right"
            | "margin-bottom"
            | "margin-left"
            | "margin-block"
            | "margin-block-start"
            | "margin-block-end"
            | "margin-inline"
            | "margin-inline-start"
            | "margin-inline-end"
    ) {
        if v == "anchor" {
            return Some(Subfeature::Probe("anchor(--a)"));
        }
        if v == "anchor-size" {
            return Some(Subfeature::Probe("anchor-size(--a)"));
        }
    }
    // … et sur la famille sizing : `anchor()`/`anchor-size()`
    // sans ancre résout en repli/invalidité par spec — la
    // fixture positionnée qui poserait l'ancre tue
    // l'oracle de sizing intrinsèque (`width:auto`
    // shrink-to-fit, mesuré étape 270). Impropre à la
    // mesure sous ce harnais → `na`, pas `miss`
    // (étape 297 : la paire ne mesurait rien de la
    // fonction — seulement « pas d'ancre → rien »).
    if matches!(
        name,
        "width"
            | "height"
            | "block-size"
            | "inline-size"
            | "min-width"
            | "min-height"
            | "min-block-size"
            | "min-inline-size"
            | "max-width"
            | "max-height"
            | "max-block-size"
            | "max-inline-size"
    ) && matches!(v, "anchor" | "anchor-size")
    {
        return Some(Subfeature::Na);
    }
    // `fit-content(<len>)` — même sous-feature sur toute la
    // famille de sizing.
    if v == "fit-content_function"
        && matches!(
            name,
            "width"
                | "height"
                | "block-size"
                | "inline-size"
                | "min-width"
                | "min-height"
                | "min-block-size"
                | "min-inline-size"
                | "max-width"
                | "max-height"
                | "max-block-size"
                | "max-inline-size"
                | "min-size"
                | "max-size"
                | "size"
                | "column-width"
        )
    {
        return Some(Subfeature::Probe("fit-content(60px)"));
    }
    if SUBFEATURE_NA.contains(&v) {
        return Some(Subfeature::Na);
    }
    // `*_context` : la fixture de la prop doit porter le
    // bon conteneur — `flex_context`/`multicol_context`
    // n'ont de sonde honnête que si la fixture matche ;
    // les contextes absents (grid pour `column-gap`…) ne
    // sont pas sondables → `na`.
    if v.ends_with("_context") || v.starts_with("context_") {
        return Some(Subfeature::Na);
    }
    // Toute autre clé avec `_` non listée : `na` au niveau
    // de l'appelant (le brut n'est pas un littéral).
    None
}

/// Verdict des gardes partagées `affectsv`/`paintv`
/// (étape 274) — tout ce qui tranche **avant** l'oracle.
enum DeclGate {
    /// Hors dénominateur (`na` — initial spec, CSS-wide,
    /// temporel, hors-oracle, méta non littéral).
    Na,
    /// Verdict tranché sans oracle (`miss` garanti :
    /// capacité absente mesurée — `hyphens` hors latin).
    Known(bool),
    /// Déclaration à passer à l'oracle (le brut, ou le
    /// littéral qu'une sonde de sous-feature substitue).
    Probe(String),
}

/// Sondes de sous-features **propres à `paintv`** (étape
/// 274) — consultées AVANT [`SUBFEATURE_PROBE`] : les
/// valeurs `url`/`image`/`image-set` sont `na` en oracle
/// géométrique (`PaintImages` vide), ici [`HARNESS_IMG`]
/// sert la ressource réelle via `collect_data_images`.
/// `url(#x)`/`url(#g)` = refs d'îlot — la valeur part en
/// `miss` honnête si le moteur ne résout pas (capacité
/// mesurée, pas borne de jauge).
fn paint_subfeature(name: &str, v: &str) -> Option<String> {
    let url = || format!("url(\"{HARNESS_IMG}\")");
    let lit = match (name, v) {
        ("background" | "border-image-source" | "list-style" | "list-style-image", "url")
        | ("background-image" | "mask" | "mask-image" | "shape-outside", "url" | "image")
        | ("content", "url" | "image" | "content_replacement") => url(),
        ("background-image" | "content" | "shape-outside", "image-set") => {
            format!("image-set({} 1x)", url())
        }
        ("border-image", "url") => format!("{} 30", url()),
        ("clip-path" | "filter" | "offset-path", "url") => "url(#x)".into(),
        ("d", "path") => "path(\"M20 90h100v-50h-100z\")".into(),
        ("fill" | "stroke", "url" | "gradient" | "paint-server") => "url(#g)".into(),
        _ => return None,
    };
    Some(lit)
}

/// Gardes communes aux oracles `affectsv`/`paintv`
/// (étape 274) : mêmes règles — initiales spec, CSS-wide,
/// familles temporelles, table hors-oracle, sous-features
/// méta BCD. `na_props` = la table hors-oracle de l'oracle
/// courant ([`NO_ORACLE`] pour le dump géométrique,
/// [`PAINT_NA`] pour le framebuffer) ; `paint` active les
/// sondes [`paint_subfeature`] prioritaires.
fn decl_gate(decl: &str, na_props: &[&str], paint: bool) -> DeclGate {
    let Some((name, value)) = decl.split_once(':') else {
        return DeclGate::Na;
    };
    let (name, value) = (name.trim(), value.trim());
    let v = value.to_ascii_lowercase();
    if matches!(
        v.as_str(),
        "inherit" | "initial" | "unset" | "revert" | "revert-layer"
    ) || explorer_css::initial_value(name).is_some_and(|iv| iv == value)
        // `eq_ignore_ascii_case` : `v` est déjà lowercasé —
        // les entrées camelCase (`currentColor`…) de la table
        // restaient mortes sans lui (étape 272 —
        // `rule-color:currentColor` comptait `miss`).
        || INITIAL_EXTRA
            .iter()
            .any(|&(n, iv)| n == name && iv.eq_ignore_ascii_case(&v))
        // `corner-*-shape:round` = initiale spec uniforme
        // (css-borders-4 §3.7 — exposant 1 = le rond
        // historique) — 17 noms, un seul repli.
        || (name.starts_with("corner-") && v == "round")
        // `container:<nom>` sans `/` = nommage pur
        // (css-contain-3 — `name / type` ; un token seul
        // est le nom, pas le type) — métadonnée sans
        // effet géométrique, même classe que
        // `container-name` (NO_ORACLE).
        || (name == "container" && !v.contains('/'))
    {
        return DeclGate::Na;
    }
    // Familles **temporelles** : `animation-*`/`transition-*`
    // ne produisent de valeur calculée qu'avec une horloge +
    // `@keyframes` — aucun oracle statique n'existe par
    // construction (même classe que les initiales spec : hors
    // dénominateur). La conformité se mesure sur les verrous
    // session (R-255/256/257), pas ici.
    if name.starts_with("animation-")
        || name.starts_with("transition-")
        || matches!(name, "animation" | "transition")
    {
        return DeclGate::Na;
    }
    // Familles **sans oracle** : l'effet de la prop vit hors
    // du référentiel mesuré — curseur (rendu OS), caret,
    // hit-testing (`pointer-events`), scroll dynamique
    // (`scroll-behavior`, snap), interaction (`resize`,
    // `touch-action`, `user-select`), widget UA (`appearance`,
    // `accent-color`), modes forcés/impression, ancrage
    // dynamique (`overflow-anchor`), pagination (`orphans`/
    // `widows`/`page*`, `speak`), horloges (timelines).
    // `NO_ORACLE` = borne du dump géométrique ; `PAINT_NA` =
    // son sous-ensemble encore aveugle en pixels. Un miss
    // serait un artefact de jauge, pas un trou moteur.
    let nn = name.trim_start_matches('-').trim_start_matches("webkit-");
    if std::env::var_os("EXPLORER_BCD_NO_NOORACLE").is_none() && na_props.contains(&nn) {
        return DeclGate::Na;
    }
    // Sous-features BCD `percentage` = « les valeurs
    // `<percent>` sont supportées » — pas une déclaration en
    // soi : sonder avec `40%` mesure la capacité réelle
    // (10% retomberait pile sur le repli `auto` ~10%em de
    // l'offset).
    if v == "percentage" || v == "percentages" {
        return DeclGate::Probe(format!("{name}:40%"));
    }
    // `hyphens:language_<X>` (sous-features BCD, ~60 clés) =
    // « la césure fonctionne pour la langue X » — pas une
    // déclaration. Le splitter borné (R-275) couvre
    // l'écriture **latine** via les règles génériques (`lang`
    // vide ou latin) : pour ces langues la sonde honnête est
    // `hyphens:auto` ; pour les autres scripts (cyrillique,
    // arabe, indic…) le moteur ne césure réellement pas →
    // `miss` mesuré, pas `na` (la capacité manque, ce n'est
    // pas une borne d'oracle).
    if name == "hyphens" && v.starts_with("language_") {
        const LATIN: &[&str] = &[
            "afrikaans",
            "albanian",
            "basque",
            "bosnian",
            "catalan",
            "croatian",
            "czech",
            "danish",
            "dutch",
            "english",
            "esperanto",
            "estonian",
            "finnish",
            "french",
            "galician",
            "german_reformed_orthography",
            "german_swiss_orthography",
            "german_traditional_orthography",
            "hungarian",
            "icelandic",
            "interlingua",
            "irish",
            "italian",
            "kurmanji",
            "latin",
            "latvian",
            "lithuanian",
            "norwegian_nn",
            "norwegian_no",
            "polish",
            "portuguese",
            "slovak",
            "slovenian",
            "spanish",
            "swedish",
            "turkish",
            "turkmen",
            "upper_sorbian",
            "welsh",
        ];
        let lang = &v["language_".len()..];
        return if LATIN.contains(&lang) {
            DeclGate::Probe("hyphens:auto".into())
        } else {
            DeclGate::Known(false)
        };
    }
    // Sondes `paintv` prioritaires : `url`/`image`/`image-set`
    // sont `na` en géométrie (table générique) — la bitmap
    // du harnais les rend mesurables en pixels.
    if paint {
        if let Some(lit) = paint_subfeature(name, &v) {
            return DeclGate::Probe(format!("{name}:{lit}"));
        }
    }
    // Sous-features BCD nommées (étape 270) — la valeur
    // n'est pas un littéral, elle désigne un cas de
    // support : `subfeature_probe` fournit la déclaration
    // qui le mesure réellement, ou `na`. Une valeur
    // contenant `_` non listée n'est jamais un lexème CSS
    // (aucun mot-clé de spec n'en porte) — `na` honnête
    // plutôt qu'un miss mesuré sur déclaration invalide.
    match subfeature_probe(name, &v) {
        Some(Subfeature::Probe(lit)) => {
            return DeclGate::Probe(format!("{name}:{lit}"));
        }
        Some(Subfeature::Na) => return DeclGate::Na,
        None => {}
    }
    if v.contains('_') {
        return DeclGate::Na;
    }
    DeclGate::Probe(decl.trim().into())
}

/// `check affectsv` — verdict tri-état sur le dump
/// géométrique + display-list (oracle historique).
fn affects_decl_verdict(decl: &str) -> Option<bool> {
    match decl_gate(decl, NO_ORACLE, false) {
        DeclGate::Na => None,
        DeclGate::Known(b) => Some(b),
        DeclGate::Probe(d) => Some(prop_affects_decl(&d)),
    }
}

/// `check paintv` — même verdict tri-état sur le
/// **framebuffer** ([`prop_paints_decl`], étape 274) :
/// [`PAINT_NA`] remplace [`NO_ORACLE`], les sondes
/// [`paint_subfeature`] servent les `url()`.
fn paint_decl_verdict(decl: &str) -> Option<bool> {
    match decl_gate(decl, PAINT_NA, true) {
        DeclGate::Na => None,
        DeclGate::Known(b) => Some(b),
        DeclGate::Probe(d) => Some(prop_paints_decl(&d)),
    }
}

/// `check atrulev` (étape 300) — oracle des **descripteurs
/// d'at-rules** (`bcd.css['at-rules'].<r>.<d>`) : une sonde
/// par descripteur mesurable en-process — (fixture, css de
/// base, css sondé, env). Le verdict = delta géométrique +
/// display-list entre la feuille de base et la feuille qui
/// pose le descripteur — même honnêteté qu'`affectsv` : la
/// feuille sondée doit changer la sortie, pas seulement
/// parser.
///
/// Tout descripteur sans sonde → `na` (hors dénominateur) :
/// `@font-face` (comportement de chargement), `@page`
/// (print), `@keyframes` (temps), `@import` (fetch),
/// `@container`, `@font-feature-values`/`@font-palette-
/// values` (shaping/palette), `speak-as` (a11y).
struct AtProbe {
    fixture: &'static str,
    base: &'static str,
    probe: String,
    /// Contre-sonde optionnelle : règle qui doit **ne pas**
    /// s'appliquer. Nécessaire quand la forme directe peut
    /// être avalée par la tolérance `unknown → vrai` (MQ `or`
    /// interne, `calc()` MQ, imbrication) — le vrai/littéral
    /// applique et le faux/littéral ne doit pas : `ok` exige
    /// `delta(probe) ∧ ¬delta(false_probe)`.
    false_probe: Option<String>,
    scripting: bool,
    reduced_motion: bool,
}

/// `atrule:descripteur` → sonde, `None` = pas d'oracle
/// in-process (`na`).
#[allow(clippy::too_many_lines)]
fn atrulev_probe(rule: &str, desc: &str) -> Option<AtProbe> {
    const P: &str = "<body><p class=x>ok</p></body>";
    const OL: &str = "<body><ol class=x><li>a<li>b<li>c<li>d</ol></body>";
    const PP: &str = "<body><div class=p><p class=x>ok</p></div></body>";
    let mk = |fixture: &'static str, base: &'static str, probe: String| AtProbe {
        fixture,
        base,
        probe,
        false_probe: None,
        scripting: false,
        reduced_motion: false,
    };
    // Sonde média : `@media <cond>` applique `.x{width:400px}`
    // vs base 50 — le littéral est choisi **vrai sous
    // `StyleEnv::default`** (1024×768, screen, dpr 1,
    // scripting none, reduced-motion off, scheme light) :
    // une feature inconnue rend la condition fausse
    // (`_ => false`) → `miss` honnête. Pour les méta-syntaxes
    // (`range`, `or`, `calc`), la tolérance `unknown → vrai`
    // saturerait la forme directe : la sonde est niée sur un
    // littéral faux — `not (faux)` applique ssi le moteur
    // évalue réellement la syntaxe.
    let media = |cond: &str| {
        mk(
            P,
            ".x{width:50px}",
            format!("@media {cond} {{ .x{{width:400px}} }}"),
        )
    };
    // `match_same_arms` : les bras `=> return None`
    // nommés (`counter-style`, `property`) documentent les
    // sous-features volontairement non sondées — le
    // wildcard suffirait mais perdrait l'intention.
    #[allow(clippy::match_same_arms)]
    let probe = match (rule, desc) {
        // ── @counter-style — la signature = le texte des
        // marqueurs de l'<ol> (display list). La base pose
        // le même style nommé sans le descripteur sondé.
        ("counter-style", "system") => mk(
            OL,
            "@counter-style t{symbols:\"α\" \"β\" \"γ\" \"δ\"}.x{list-style-type:t;list-style-position:inside}",
            "@counter-style t{system:numeric;symbols:\"α\" \"β\" \"γ\" \"δ\"}\
             .x{list-style-type:t;list-style-position:inside}"
                .into(),
        ),
        ("counter-style", "symbols") => mk(
            OL,
            "@counter-style t{system:cyclic}.x{list-style-type:t;list-style-position:inside}",
            "@counter-style t{system:cyclic;symbols:\"◆\" \"■\" \"▲\" \"●\"}\
             .x{list-style-type:t;list-style-position:inside}"
                .into(),
        ),
        ("counter-style", "additive-symbols") => mk(
            OL,
            "@counter-style t{system:additive}.x{list-style-type:t;list-style-position:inside}",
            "@counter-style t{system:additive;additive-symbols:3 \"C\",2 \"B\",1 \"A\"}\
             .x{list-style-type:t;list-style-position:inside}"
                .into(),
        ),
        ("counter-style", "fallback") => mk(
            OL,
            "@counter-style t{system:fixed;symbols:\"A\"}.x{list-style-type:t;list-style-position:inside}",
            "@counter-style t{system:fixed;symbols:\"A\";fallback:lower-roman}\
             .x{list-style-type:t;list-style-position:inside}"
                .into(),
        ),
        // `negative` n'agit que sur valeurs négatives —
        // `start="-1"` produit les ordinaux -1,0,1,2 : le
        // premier est enrobé `!n` vs `-n` par défaut.
        ("counter-style", "negative") => mk(
            "<body><ol class=x start=\"-1\"><li>a<li>b<li>c<li>d</ol></body>",
            "@counter-style t{system:numeric}.x{list-style-type:t;list-style-position:inside}",
            "@counter-style t{system:numeric;negative:\"!\"}\
             .x{list-style-type:t;list-style-position:inside}"
                .into(),
        ),
        ("counter-style", "pad") => mk(
            OL,
            "@counter-style t{system:numeric}.x{list-style-type:t;list-style-position:inside}",
            "@counter-style t{system:numeric;pad:2 \"0\"}.x{list-style-type:t;list-style-position:inside}".into(),
        ),
        ("counter-style", "prefix") => mk(
            OL,
            "@counter-style t{system:numeric}.x{list-style-type:t;list-style-position:inside}",
            "@counter-style t{system:numeric;prefix:\"§\"}.x{list-style-type:t;list-style-position:inside}".into(),
        ),
        // `range` hors portée → repli : il faut des symboles
        // pour distinguer l'en-portée (base-4 → γ/δ) du
        // repli décimal (1/4) — sans eux tout est décimal
        // dans les deux feuilles (sonde aveugle).
        ("counter-style", "range") => mk(
            OL,
            "@counter-style t{system:numeric;symbols:\"α\" \"β\" \"γ\" \"δ\"}.x{list-style-type:t;list-style-position:inside}",
            "@counter-style t{system:numeric;symbols:\"α\" \"β\" \"γ\" \"δ\";range:2 3}.x{list-style-type:t;list-style-position:inside}".into(),
        ),
        ("counter-style", "suffix") => mk(
            OL,
            "@counter-style t{system:numeric}.x{list-style-type:t;list-style-position:inside}",
            "@counter-style t{system:numeric;suffix:\"!\"}.x{list-style-type:t;list-style-position:inside}".into(),
        ),
        // `speak-as` — aural/a11y, aucun effet géométrique.
        ("counter-style", _) => return None,
        // ── @supports — la condition évaluée applique la
        // déclaration sondée. `selector()`/`font-format()`…
        // se parsent en prop inconnue → `decl_supported`
        // faux → `miss` honnête (fonction non évaluée).
        ("supports", d) => {
            let cond = match d {
                // `selector(div)` n'a pas de `:` → condition
                // ignorée → prélude vide → `unknown` toléré
                // (applique toujours) — indiscernable d'un
                // vrai ok. `selector(div:hover)` contient `:` →
                // parsé en prop `selector(div` → miss tant que
                // `selector()` n'est pas évaluée, ok dès que la
                // fonction s'ouvre. `font-format`/`font-tech`/
                // `at-rule` n'ont aucune forme à `:` → `na`.
                "selector" => "selector(div:hover)",
                // `named_feature` BCD = la condition
                // propriété-custom `(--x: v)` (css-conditional).
                "named_feature" | "named-feature" => "(--x: 1)",
                _ => return None,
            };
            mk(
                P,
                ".x{width:50px}",
                format!("@supports ({cond}) {{ .x{{width:400px}} }}"),
            )
        }
        // ── @property — la registration est consommée par
        // `var()` : chaque descripteur a une sonde
        // discriminante (fallback `50px` si la règle est
        // ignorée/invalide).
        ("property", "initial-value") => mk(
            PP,
            ".x{width:var(--w,50px)}",
            "@property --w{syntax:\"<length>\";initial-value:400px;inherits:false}\
             .x{width:var(--w,50px)}"
                .into(),
        ),
        ("property", "syntax") => mk(
            PP,
            "@property --w{initial-value:400px;inherits:false}\
             .x{width:var(--w,50px)}",
            // `initial-value` non conforme à `syntax` → la
            // registration est invalide par spec : si le
            // moteur valide, `var(--w)` retombe sur 50.
            "@property --w{syntax:\"<color>\";initial-value:400px;inherits:false}\
             .x{width:var(--w,50px)}"
                .into(),
        ),
        ("property", "inherits") => mk(
            PP,
            ".p{--w:200px}.x{width:var(--w,50px)}",
            // `inherits:false` honoré → `.x` voit l'initial
            // (9 px) et non le `--w` parental (200 px).
            "@property --w{inherits:false;initial-value:9px}\
             .p{--w:200px}.x{width:var(--w,50px)}"
                .into(),
        ),
        ("property", _) => return None,
        // ── @media — littéral vrai sous l'env par défaut.
        ("media", d) => {
            match d {
                "any-hover" => media("(any-hover:hover)"),
                "any-pointer" => media("(any-pointer:fine)"),
                "aspect-ratio" => media("(aspect-ratio:4/3)"),
                "calc" => {
                    let mut m2 = media("(min-width:calc(500px + 1px))");
                    // `calc()` MQ irrésoluble → `condition_matches`
                    // faux, indiscernable d'un vrai calcul sur la
                    // forme niée — paire vrai/faux requise.
                    m2.false_probe = Some(
                        "@media (min-width:calc(9000px + 9000px)){.x{height:777px}}".into(),
                    );
                    m2
                }
                "color" => media("(color)"),
                "color-gamut" => media("(color-gamut:srgb)"),
                "color-index" => media("(color-index)"),
                "device-aspect-ratio" => media("(device-aspect-ratio:4/3)"),
                "device-height" => media("(device-height:768px)"),
                "device-posture" => media("(device-posture)"),
                "device-width" => media("(device-width:1024px)"),
                "display-mode" => media("(display-mode:browser)"),
                "display-state" => media("(display-state)"),
                "dynamic-range" => media("(dynamic-range)"),
                "forced-colors" => media("(forced-colors:none)"),
                "grid" => media("(grid:0)"),
                "height" => media("(min-height:100px)"),
                "horizontal-viewport-segments" => media("(horizontal-viewport-segments:1)"),
                "hover" => media("(hover:hover)"),
                "inverted-colors" => media("(inverted-colors:none)"),
                // Méta-noms BCD : la forme feature /
                // les valeurs fractionnaires.
                "media_features" => media("(min-width:1px)"),
                "media_query_values" => media("(min-width:100.5px)"),
                "monochrome" => media("(monochrome:0)"),
                "nested-queries" => {
                    let mut m2 = mk(
                        P,
                        ".x{width:50px}",
                        "@media screen{@media (min-width:1px){.x{width:400px}}}".into(),
                    );
                    // `@media` imbriqué non évalué → tolérance
                    // → applique : indiscernable du vrai — la
                    // contre-sonde (inner faux) ne doit pas
                    // appliquer.
                    m2.false_probe = Some(
                        "@media screen{@media (min-width:9999px){.x{height:777px}}}".into(),
                    );
                    m2
                }
                "or_syntax" => {
                    let mut m2 = media("(min-width:9000px) or (min-width:100px)");
                    // `(a) or (b)` non parsé → `unknown` toléré →
                    // applique : indiscernable d'un vrai `or`
                    // évalué. La contre-sonde (deux membres faux)
                    // ne doit PAS appliquer.
                    m2.false_probe = Some(
                        "@media (min-width:9000px) or (min-height:9000px){.x{height:777px}}"
                            .into(),
                    );
                    m2
                }
                "orientation" => media("(orientation:landscape)"),
                "overflow-block" => media("(overflow-block:scroll)"),
                "overflow-inline" => media("(overflow-inline:scroll)"),
                "pointer" => media("(pointer:fine)"),
                "prefers-color-scheme" => media("(prefers-color-scheme:light)"),
                "prefers-contrast" => media("(prefers-contrast:no-preference)"),
                "prefers-reduced-data" => media("(prefers-reduced-data:no-preference)"),
                "prefers-reduced-motion" => {
                    let mut m2 = media("(prefers-reduced-motion:reduce)");
                    m2.reduced_motion = true;
                    m2
                }
                "prefers-reduced-transparency" => {
                    media("(prefers-reduced-transparency:no-preference)")
                }
                "range_syntax" => media("not (width > 5000px)"),
                "resizable" => media("(resizable)"),
                "resolution" => media("(resolution:1dppx)"),
                "scan" => media("(scan:progressive)"),
                "scripting" => media("(scripting:none)"),
                "update" => media("(update:fast)"),
                "vertical-viewport-segments" => media("(vertical-viewport-segments:1)"),
                "video-dynamic-range" => media("(video-dynamic-range)"),
                "width" => media("(min-width:500px)"),
                "-moz-device-pixel-ratio" => media("(-moz-device-pixel-ratio:1)"),
                "-webkit-animation" => media("(-webkit-animation)"),
                "-webkit-device-pixel-ratio" => media("(-webkit-device-pixel-ratio:1)"),
                "-webkit-max-device-pixel-ratio" => media("(-webkit-max-device-pixel-ratio:1)"),
                "-webkit-min-device-pixel-ratio" => media("(-webkit-min-device-pixel-ratio:1)"),
                "-webkit-transform-2d" => media("(-webkit-transform-2d)"),
                "-webkit-transform-3d" => media("(-webkit-transform-3d)"),
                "-webkit-transition" => media("(-webkit-transition)"),
                _ => return None,
            }
        }
        // Tout le reste : pas d'oracle in-process — `na`.
        _ => return None,
    };
    Some(probe)
}

/// Verdict `check atrulev` — `na` si aucun oracle pour le
/// couple (règle, descripteur).
fn atrulev_verdict(line: &str) -> Option<bool> {
    let (rule, desc) = line.split_once(':')?;
    let p = atrulev_probe(rule.trim(), desc.trim())?;
    Some(atrule_delta(&p))
}

/// Mesure la sonde : feuille base + sondée vs base seule,
/// même signature qu'`affectsv` (box tree + display list).
fn atrule_delta(p: &AtProbe) -> bool {
    static SRC: BcdAhem = BcdAhem;
    let ctx = explorer_layout::BlockCtx {
        fonts: Some(&SRC),
        ..explorer_layout::BlockCtx::default()
    };
    let dom = explorer_html::TreeBuilder::parse(p.fixture);
    let root = {
        let mut r = None;
        walk_any(dom.document(), &dom, &mut r);
        r.unwrap_or(dom.document())
    };
    let env = explorer_css::StyleEnv {
        scripting: p.scripting,
        reduced_motion: p.reduced_motion,
        ..explorer_css::StyleEnv::default()
    };
    let sig = |css: &str| -> String {
        let mut eng = explorer_css::StyleEngine::new();
        eng.add_css(css, explorer_css::Origin::Author);
        let sm = eng.compute_tree(&dom, root, &env);
        match explorer_layout::build_box_tree(&dom, root, &sm) {
            Some(mut bx) => {
                explorer_layout::layout_tree(&mut bx, 800.0, &ctx);
                let dl = explorer_render::build_display_list(&bx, 800.0, 600.0, 0.0);
                format!("{}\n{}", bx.dump(&dom), dl.dump())
            }
            None => String::new(),
        }
    };
    let b = sig(p.base);
    let a = sig(&format!("{}\n{}", p.base, p.probe));
    // Contre-sonde : le littéral **faux** ne doit produire
    // aucun delta — sinon la forme directe était avalée par
    // la tolérance `unknown → vrai` et l'« ok » serait faux.
    let neg_ok = p
        .false_probe
        .as_ref()
        .is_none_or(|fp| sig(&format!("{}\n{}", p.base, fp)) == b);
    if std::env::var_os("EXPLORER_BCD_DEBUG_SIG").is_some() {
        eprintln!("=== atrulev\n--- with\n{a}\n--- base\n{b}\n--- neg_ok={neg_ok}");
    }
    a != b && neg_ok
}

/// `check computedv` (étape 298) — verdict tri-état sur le
/// style calculé : mêmes `na` de valeur (initiales spec,
/// CSS-wide, méta non sondé) que le chemin union, mais
/// **aucune** table prop-level — `NO_ORACLE`/`PAINT_NA`
/// bornent le *rendu* de familles, pas la cascade (un
/// shorthand `mask` expansé reste un delta calculé
/// mesurable même si aucun oracle rendu ne le voit).
/// `ok` = « la cascade applique », verdict de triage
/// hors union.
fn computed_decl_verdict(decl: &str) -> Option<bool> {
    match decl_gate(decl, &[], true) {
        DeclGate::Na => None,
        DeclGate::Known(b) => Some(b),
        DeclGate::Probe(d) => Some(prop_computes_decl(&d)),
    }
}

/// `check tables` (étape 277) — dump TSV des tables de
/// qualification de la jauge pour `bcd_audit.mjs` :
/// lignes `initial_extra\t<prop>\t<valeur>`,
/// `no_oracle\t<prop>`, `paint_na\t<prop>`.
/// [`subfeature_probe`]/[`paint_subfeature`] sont des
/// `match` non énumérables — leur correction se sonde
/// comportementalement via `check decl` (méta-nom →
/// `na`/`probe:`).
fn check_tables() -> ExitCode {
    for (n, v) in INITIAL_EXTRA {
        println!("initial_extra\t{n}\t{v}");
    }
    for n in NO_ORACLE {
        println!("no_oracle\t{n}");
    }
    for n in PAINT_NA {
        println!("paint_na\t{n}");
    }
    ExitCode::SUCCESS
}

/// `check decl` (étape 277) — verdict de la **porte** de la
/// jauge sur `p:v`, sans oracle : `na`, `known:false`
/// (capacité tranchée absente — `hyphens` hors latin),
/// `probe:<lit>` (sous-feature BCD substituée par un
/// littéral), `decl:<lit>` (mesuré tel quel) ou
/// `invalid:<lit>` (le littéral résolu échoue à la
/// validation explorer-css — suspect « valeur BCD non
/// littérale » ou sonde interne cassée). Chemin de
/// l'union : [`PAINT_NA`] + [`paint_subfeature`] — `na`
/// ici = neutre sous les deux oracles (paint-na ⊆
/// geo-na par construction).
fn check_decls() -> ExitCode {
    use std::io::BufRead;
    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        let Ok(decl) = line else { break };
        let decl = decl.trim();
        if decl.is_empty() {
            continue;
        }
        let out = match decl_gate(decl, PAINT_NA, true) {
            DeclGate::Na => "na".to_string(),
            DeclGate::Known(b) => format!("known:{b}"),
            DeclGate::Probe(d) => {
                let kind = if d == decl { "decl" } else { "probe" };
                if decl_valid(&d) {
                    format!("{kind}:{d}")
                } else {
                    format!("invalid:{d}")
                }
            }
        };
        println!("{decl}\t{out}");
    }
    ExitCode::SUCCESS
}

/// Validation syntaxique du littéral résolu (étape 277) :
/// prop reconnue, déclaration qui survit au parse et
/// valeur dans la grammaire (périmètre
/// [`explorer_css::value::value_is_valid_for`]). Un
/// `invalid` sur
/// une sonde interne = ligne de table cassée (ex. prop
/// retirée de `SUPPORTED_PROPS`) ; sur une valeur BCD
/// littérale = non-littéral passé à l'oracle.
fn decl_valid(d: &str) -> bool {
    let Some((n, v)) = d.split_once(':') else {
        return false;
    };
    let (n, v) = (n.trim(), v.trim());
    explorer_css::prop_supported(n)
        && explorer_css::parse_declarations(d)
            .iter()
            .any(|dd| dd.name.eq_ignore_ascii_case(n))
        && explorer_css::value::value_is_valid_for(n, v)
}

/// `check html` — oracle de comportement par élément
/// (`bcd.html.elements`, étape 219). « Le tag parse » est
/// trivial (toute balise produit un élément) : la jauge
/// combine trois mesures par tag —
///   1. **interface JS** : `document.createElement(tag)`
///      est `instanceof` de l'interface spec (`HTMLDialog-
///      Element`, `HTMLTemplateElement`…, défaut
///      `HTMLElement`) ;
///   2. **membres IDL clés** : `showModal`, `content`,
///      `value`… présents sur l'élément ;
///   3. **géométrie UA** : le box tree (feuille `UA_CSS`
///      incluse par `StyleEngine::new`) montre l'effet
///      attendu — boîte produite, `list-item`, `display:none`
///      des éléments inertes, contenu replié (`details` sans
///      `open`) ou promu (`slot` → `display:contents`).
///
/// Table `tag → assertion` artisanale : BCD fournit le
/// dénominateur, chaque ligne mesure un vrai comportement.
#[derive(Clone, Copy)]
enum HtmlWant {
    /// Une boîte nommée `<tag>` existe dans le dump.
    Renders,
    /// L'élément produit une boîte `list-item`.
    ListItem,
    /// Aucune boîte `<tag>` — `display:none` UA (script,
    /// style, title, meta, link, source, template…).
    Hidden,
    /// Boîte `<tag>` présente mais le texte fils replié
    /// (`details` sans `open`).
    Collapsed(&'static str),
    /// Pas de boîte `<tag>` mais le texte enfant promu dans
    /// le flux (`slot` → `display:contents` UA).
    Contents(&'static str),
}

/// Assertion d'un élément : interface JS attendue,
/// expression IDL supplémentaire (sur `el`, conjointe),
/// fixture HTML et attente géométrique.
struct HtmlSpec {
    ctor: &'static str,
    extra: &'static str,
    fixture: Option<&'static str>,
    want: HtmlWant,
}

const fn hs(
    ctor: &'static str,
    extra: &'static str,
    fixture: Option<&'static str>,
    want: HtmlWant,
) -> HtmlSpec {
    HtmlSpec {
        ctor,
        extra,
        fixture,
        want,
    }
}

/// Table `tag → assertion` — les interfaces viennent de la
/// spec HTML (oracle indépendant de `tag_interface`), les
/// membres IDL des propriétés qui distinguent un élément
/// réel d'un conteneur générique. Les éléments non listés
/// tombent sur le défaut « `HTMLElement` + boîte rendue »
/// (conteneurs sans sémantique propre : `div`, `section`,
/// `article`… — inclus au dénominateur, triviaux).
#[allow(clippy::too_many_lines)]
fn html_spec(tag: &str) -> HtmlSpec {
    use HtmlWant::{Collapsed, Contents, Hidden, ListItem, Renders};
    match tag {
        "a" => hs("HTMLAnchorElement", "'href' in el", None, Renders),
        "area" => hs("HTMLAreaElement", "true",
            Some("<body><map name=m><area href=x></map></body>"), Hidden),
        "audio" => hs("HTMLAudioElement", "true",
            Some("<body><audio></audio></body>"), Hidden),
        "base" => hs("HTMLBaseElement", "true", None, Hidden),
        "blockquote" | "q" => hs("HTMLQuoteElement", "true", None, Renders),
        // Parenthèse de repli pour les UAs sans ruby — ne
        // génère jamais de boîte propre dans un contexte
        // ruby (le texte `<rt>` est posé par le moteur).
        "rp" => hs(
            "HTMLElement",
            "true",
            Some("<body><ruby>base<rp>(</rp><rt>x</rt><rp>)</rp></ruby></body>"),
            Hidden,
        ),
        "body" => hs("HTMLBodyElement", "true",
            Some("<body><p>ok</p></body>"), Renders),
        "br" => hs("HTMLBRElement", "true",
            Some("<body><p>a<br>b</p></body>"), Renders),
        "button" => hs("HTMLButtonElement", "'disabled' in el && 'value' in el",
            None, Renders),
        "canvas" => hs("HTMLCanvasElement", "true", None, Renders),
        "caption" => hs("HTMLTableCaptionElement", "true",
            Some("<body><table><caption>ok</caption><tr><td>c</td></tr></table></body>"),
            Renders),
        "col" | "colgroup" => hs("HTMLTableColElement", "true",
            Some("<body><table><colgroup><col></colgroup><tr><td>c</td></tr></table></body>"),
            Renders),
        "data" => hs("HTMLDataElement", "'value' in el", None, Renders),
        "datalist" => hs("HTMLDataListElement", "true", None, Hidden),
        "del" | "ins" => hs("HTMLModElement", "true", None, Renders),
        // Replié sans `open` — UA `details:not([open]) >
        // :not(summary) { display: none }`.
        "details" => hs("HTMLDetailsElement", "true",
            Some("<body><details><summary>voir</summary><p>HIDEME</p></details></body>"),
            Collapsed("HIDEME")),
        "dialog" => hs("HTMLDialogElement",
            "typeof el.showModal === 'function' && typeof el.close === 'function'",
            Some("<body><dialog><p>HIDEME</p></dialog></body>"), Hidden),
        "dir" => hs("HTMLDirectoryElement", "true", None, Renders),
        "embed" => hs("HTMLEmbedElement", "true", None, Renders),
        "fencedframe" => hs("HTMLFencedFrameElement", "true", None, Renders),
        "fieldset" => hs("HTMLFieldSetElement", "true", None, Renders),
        "font" => hs("HTMLFontElement", "true", None, Renders),
        "form" => hs("HTMLFormElement", "'method' in el", None, Renders),
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" =>
            hs("HTMLHeadingElement", "true", None, Renders),
        "head" => hs("HTMLHeadElement", "true",
            Some("<body><p>ok</p></body>"), Hidden),
        "hr" => hs("HTMLHRElement", "true", None, Renders),
        "html" => hs("HTMLHtmlElement", "true",
            Some("<body><p>ok</p></body>"), Renders),
        "iframe" => hs("HTMLIFrameElement", "'src' in el", None, Renders),
        "img" => hs("HTMLImageElement", "'src' in el && 'alt' in el",
            Some("<body><img alt=x></body>"), Renders),
        "input" => hs("HTMLInputElement", "'value' in el && 'checked' in el",
            Some("<body><input type=checkbox></body>"), Renders),
        "label" => hs("HTMLLabelElement", "true", None, Renders),
        "legend" => hs("HTMLLegendElement", "true",
            Some("<body><fieldset><legend>ok</legend></fieldset></body>"), Renders),
        "li" => hs("HTMLLIElement", "true",
            Some("<body><ul><li>ok</li></ul></body>"), ListItem),
        "link" => hs("HTMLLinkElement", "true", None, Hidden),
        "map" => hs("HTMLMapElement", "true", None, Renders),
        "marquee" => hs("HTMLMarqueeElement", "true", None, Renders),
        "menu" => hs("HTMLMenuElement", "true",
            Some("<body><menu><li>ok</li></menu></body>"), Renders),
        "meta" => hs("HTMLMetaElement", "true", None, Hidden),
        "meter" => hs("HTMLMeterElement", "true", None, Renders),
        "object" => hs("HTMLObjectElement", "true", None, Renders),
        "ol" => hs("HTMLOListElement", "true",
            Some("<body><ol><li>ok</li></ol></body>"), Renders),
        // `option`/`optgroup` ne génèrent pas de boîte
        // propre — le `<select>` peint le libellé de
        // l'option courante dans son contrôle.
        "optgroup" => hs("HTMLOptGroupElement", "true",
            Some("<body><select><optgroup><option>OPTME</option></optgroup></select></body>"),
            Contents("OPTME")),
        "option" => hs("HTMLOptionElement", "'value' in el",
            Some("<body><select><option>OPTME</option></select></body>"), Contents("OPTME")),
        "output" => hs("HTMLOutputElement", "true", None, Renders),
        "p" => hs("HTMLParagraphElement", "true", None, Renders),
        "param" => hs("HTMLParamElement", "true", None, Hidden),
        "picture" => hs("HTMLPictureElement", "true",
            Some("<body><picture><img alt=x></picture></body>"), Renders),
        "pre" => hs("HTMLPreElement", "true", None, Renders),
        "progress" => hs("HTMLProgressElement", "'value' in el", None, Renders),
        "script" => hs("HTMLScriptElement", "true", None, Hidden),
        "select" => hs("HTMLSelectElement", "'value' in el",
            Some("<body><select><option>o</option></select></body>"), Renders),
        // `display:contents` UA — pas de boîte propre, le
        // repli est promu dans le flux.
        "slot" => hs("HTMLSlotElement", "true", None, Contents("ok")),
        "source" => hs("HTMLSourceElement", "true", None, Hidden),
        "span" => hs("HTMLSpanElement", "true", None, Renders),
        "style" => hs("HTMLStyleElement", "true", None, Hidden),
        "table" => hs("HTMLTableElement", "true",
            Some("<body><table><tr><td>ok</td></tr></table></body>"), Renders),
        "tbody" => hs("HTMLTableSectionElement", "true",
            Some("<body><table><tbody><tr><td>ok</td></tr></tbody></table></body>"),
            Renders),
        "thead" => hs("HTMLTableSectionElement", "true",
            Some("<body><table><thead><tr><th>h</th></tr></thead><tr><td>ok</td></tr></table></body>"),
            Renders),
        "tfoot" => hs("HTMLTableSectionElement", "true",
            Some("<body><table><tfoot><tr><td>f</td></tr></tfoot><tr><td>ok</td></tr></table></body>"),
            Renders),
        "td" => hs("HTMLTableCellElement", "true",
            Some("<body><table><tr><td>ok</td></tr></table></body>"), Renders),
        "th" => hs("HTMLTableCellElement", "true",
            Some("<body><table><tr><th>ok</th></tr></table></body>"), Renders),
        // `.content` = DocumentFragment distinct ; les
        // enfants du template n'appartiennent pas au DOM
        // rendu (`display:none` UA de toute façon).
        "template" => hs("HTMLTemplateElement",
            "el.content instanceof DocumentFragment",
            Some("<body><template><p>HIDEME</p></template></body>"), Hidden),
        "textarea" => hs("HTMLTextAreaElement", "'value' in el", None, Renders),
        "time" => hs("HTMLTimeElement", "'dateTime' in el", None, Renders),
        "title" => hs("HTMLTitleElement", "true", None, Hidden),
        "tr" => hs("HTMLTableRowElement", "true",
            Some("<body><table><tr><td>ok</td></tr></table></body>"), Renders),
        "track" => hs("HTMLTrackElement", "true", None, Hidden),
        "ul" => hs("HTMLUListElement", "true",
            Some("<body><ul><li>ok</li></ul></body>"), Renders),
        "video" => hs("HTMLVideoElement", "true", None, Renders),
        "wbr" => hs("HTMLElement", "true",
            Some("<body><p>a<wbr>b</p></body>"), Renders),
        // Conteneur générique — seule jauge honnête :
        // interface `HTMLElement` + boîte produite.
        _ => hs("HTMLElement", "true", None, Renders),
    }
}

/// Dump du box tree d'une fixture (feuille UA comprise) —
/// sondage géométrique partagé avec `prop_affects_decl`.
fn html_box_dump(fixture: &str) -> String {
    let dom = explorer_html::TreeBuilder::parse(fixture);
    let root = {
        let mut r = None;
        walk_any(dom.document(), &dom, &mut r);
        r.unwrap_or(dom.document())
    };
    let eng = explorer_css::StyleEngine::new();
    let sm = eng.compute_tree(&dom, root, &explorer_css::StyleEnv::default());
    match explorer_layout::build_box_tree(&dom, root, &sm) {
        Some(mut bx) => {
            explorer_layout::layout_tree(&mut bx, 800.0, &explorer_layout::BlockCtx::default());
            bx.dump(&dom)
        }
        None => String::new(),
    }
}

fn html_want_ok(dump: &str, tag: &str, want: HtmlWant) -> bool {
    let pat = format!("<{tag}");
    match want {
        HtmlWant::Renders => dump.contains(&pat),
        HtmlWant::Hidden => !dump.contains(&pat),
        HtmlWant::ListItem => dump
            .lines()
            .any(|l| l.contains("list-item") && l.contains(&pat)),
        HtmlWant::Collapsed(text) => dump.contains(&pat) && !dump.contains(text),
        HtmlWant::Contents(text) => !dump.contains(&pat) && dump.contains(text),
    }
}

/// Canal JS de `check html` — un moteur avec la surface
/// navigateur (`install_dom`), une expression de sonde par
/// tag passant par `console.log` (canal capturé, pas de
/// parsing de `Value`).
struct HtmlJsProbe {
    eng: explorer_jscore::Engine,
    out: std::rc::Rc<std::cell::RefCell<Vec<String>>>,
}

impl HtmlJsProbe {
    fn new() -> Self {
        let mut eng = explorer_jscore::Engine::new();
        let dom = std::rc::Rc::new(std::cell::RefCell::new(explorer_jscore::Dom::from_html(
            "<!doctype html><body></body>",
        )));
        eng.install_dom(&dom);
        let out = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let o = out.clone();
        eng.set_console_hook(std::rc::Rc::new(move |l| o.borrow_mut().push(l)));
        Self { eng, out }
    }

    /// `createElement(tag) instanceof ctor && extra` — un
    /// ctor absent du global (`HTMLFencedFrameElement`…)
    /// jette `ReferenceError` = miss honnête.
    fn ok(&mut self, tag: &str, spec: &HtmlSpec) -> bool {
        self.out.borrow_mut().clear();
        let src = format!(
            "console.log(function(){{var el=document.createElement({tag:?});\
             return el instanceof {} && ({})}}()?'ok':'miss')",
            spec.ctor, spec.extra
        );
        if self.eng.eval_named(&src, "check-html").is_err() {
            return false;
        }
        self.out.borrow().last().is_some_and(|l| l.trim() == "ok")
    }
}

fn html_check(tag: &str, cell: &std::cell::RefCell<Option<HtmlJsProbe>>) -> bool {
    let tag = tag.trim().to_ascii_lowercase();
    let spec = html_spec(&tag);
    let js_ok = {
        let mut g = cell.borrow_mut();
        g.get_or_insert_with(HtmlJsProbe::new).ok(&tag, &spec)
    };
    if !js_ok {
        return false;
    }
    let fixture = spec
        .fixture
        .map_or_else(|| format!("<body><{tag}>ok</{tag}></body>"), str::to_string);
    html_want_ok(&html_box_dump(&fixture), &tag, spec.want)
}

fn walk_any(
    n: explorer_dom::NodeId,
    dom: &explorer_dom::Dom,
    out: &mut Option<explorer_dom::NodeId>,
) {
    if dom.element(n).is_ok() && out.is_none() {
        *out = Some(n);
    }
    if let Ok(ch) = dom.children(n) {
        for c in ch {
            walk_any(c, dom, out);
        }
    }
}

fn check_names(kind: &str) -> ExitCode {
    use std::io::BufRead;
    // Modes audit (étape 277) — `tables`/`decl` exposent les
    // structures internes de la jauge à `bcd_audit.mjs`
    // (lints de qualification : lignes mortes, méta-noms,
    // littéraux invalides) — pas des sondes ok/miss.
    match kind {
        "tables" => return check_tables(),
        "decl" => return check_decls(),
        _ => {}
    }
    let routed_dom;
    let test: Box<dyn Fn(&str) -> bool> = match kind {
        "prop" => Box::new(explorer_css::prop_supported),
        "sel" => Box::new(explorer_css::selector_supported),
        "at" => Box::new(explorer_css::at_rule_supported),
        // R-245/246 — `routed` : la déclaration atteint-elle
        // `ComputedStyle` (saturante tant que la cascade
        // stocke les inconnus) ; `affects` : la géométrie
        // change-t-elle (jauge non-saturante).
        "routed" => {
            // Parent à `display:inline` : `display:inherit` sur
            // `.x` hérite `inline` ≠ `block` UA — sans ça la
            // sonde est aveugle (valeur héritée = initiale).
            routed_dom = explorer_html::TreeBuilder::parse(
                "<body style=display:inline><p class=x>ok</p></body>",
            );
            Box::new(move |name| explorer_css::prop_routed(&routed_dom, name))
        }
        "affects" => Box::new(prop_affects),
        // `affectsv`/`paintv` sont tri-état (`na` = valeur
        // initiale spec, CSS-wide ou famille hors-oracle —
        // neutre par construction, hors dénominateur) :
        // boucle dédiée plus bas. `paintv` = oracle pixel
        // (étape 274).
        "affectsv" => Box::new(prop_affects_decl),
        "paintv" => Box::new(prop_paints_decl),
        "computedv" => Box::new(prop_computes_decl),
        "atrulev" => Box::new(|_s| false),
        // Étape 219 — `html` : oracle de comportement par
        // élément (`bcd.html.elements`) — binding JS
        // (interface + membres IDL) ET géométrie UA (box
        // tree) ; un moteur jscore partagé, créé à la
        // première ligne sondée.
        "html" => {
            let cell = std::cell::RefCell::new(None);
            Box::new(move |tag| html_check(tag, &cell))
        }
        _ => {
            eprintln!(
                "check : kind inconnu {kind:?} — attendu prop|sel|at|routed|affects|affectsv|paintv|computedv|atrulev|html|decl|tables"
            );
            return ExitCode::FAILURE;
        }
    };
    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        let Ok(name) = line else { break };
        let name = name.trim();
        if name.is_empty() {
            continue;
        }
        if kind == "affectsv" || kind == "paintv" || kind == "computedv" || kind == "atrulev" {
            let verdict = match kind {
                "affectsv" => affects_decl_verdict(name),
                "paintv" => paint_decl_verdict(name),
                "atrulev" => atrulev_verdict(name),
                _ => computed_decl_verdict(name),
            };
            let verdict = verdict.map_or("na", |ok| if ok { "ok" } else { "miss" });
            println!("{name}\t{verdict}");
        } else {
            println!("{name}\t{}", if test(name) { "ok" } else { "miss" });
        }
    }
    ExitCode::SUCCESS
}

/// `mem <url>` : attribution mémoire par réservoir après
/// navigation + grâce (étape 130) — la page vivante ET les
/// pages gelées bfcache (heaps JS gelés = premier réservoir
/// suspecté du constat « station ~1,4 Gio »).
/// Mio en décimale tronquée (`"12.3"`) — arithmétique
/// entière, aucune conversion `usize`→`f64` (clippy).
fn mib(b: usize) -> String {
    let k = b / 1024;
    format!("{}.{}", k / 1024, (k % 1024) * 10 / 1024)
}

#[tokio::main(flavor = "current_thread")]
async fn mem_report(url: &str) -> ExitCode {
    let fonts = explorer_render::DirFontSource::new(&default_fonts_dir());
    let mut session = session_from_env(fonts);
    session.set_js_factory(explorer_js::JscoreEngine::factory());
    let tab = session.open_tab();
    if let Err(e) = session.navigate(tab, url).await {
        eprintln!("erreur : {e}");
        return ExitCode::FAILURE;
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(1_500);
    while std::time::Instant::now() < deadline {
        session.pump_net().await;
        session.pump_timers(tab);
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    eprintln!("[url : {}]", session.tab_url(tab).unwrap_or_default());
    let Some(r) = session.mem_report(tab) else {
        eprintln!("onglet introuvable");
        return ExitCode::FAILURE;
    };
    println!("page vivante :");
    println!("  dom        : {} nœuds", r.dom_nodes);
    println!("  layout     : {} boîtes", r.layout_boxes);
    println!("  display    : {} items", r.display_items);
    println!("  frame      : {} Mio", mib(r.frame_bytes));
    println!("  png        : {} Mio", mib(r.png_bytes));
    println!("  images     : {} Mio (dédup vivant)", mib(r.images_bytes));
    println!("  bg_bodies  : {} Mio", mib(r.bg_bodies_bytes));
    println!(
        "  img_bodies : {} Mio (compressé retenu)",
        mib(r.img_bodies_bytes)
    );
    println!("  js heap    : {} Mio", mib(r.js_heap_bytes));
    println!("bfcache ({} pages gelées) :", r.bfcache_pages);
    println!(
        "  dom        : {} nœuds · layout {} · display {}",
        r.bfcache_dom_nodes, r.bfcache_layout_boxes, r.bfcache_display_items
    );
    println!(
        "  frame+png  : {} Mio",
        mib(r.bfcache_frame_bytes + r.bfcache_png_bytes)
    );
    println!(
        "  images     : {} Mio (résiduel — attendu 0 post-étape 133)",
        mib(r.bfcache_images_bytes)
    );
    println!(
        "  img_bodies : {} Mio (compressé retenu)",
        mib(r.bfcache_img_bodies_bytes)
    );
    println!("  js heap    : {} Mio", mib(r.bfcache_js_heap_bytes));
    println!("total octets attribués : {} Mio", mib(r.total_bytes));
    println!("(non mesuré : caches fontes, prev_styles, miroir DOM jscore, registres idb/caches)");
    ExitCode::SUCCESS
}

#[tokio::main(flavor = "current_thread")]
async fn jprobe(url: &str) -> ExitCode {
    let fonts = explorer_render::DirFontSource::new(&default_fonts_dir());
    let mut session = session_from_env(fonts);
    session.set_js_factory(explorer_js::JscoreEngine::factory());
    let mut events = session.subscribe();
    let tab = session.open_tab();
    if let Err(e) = session.navigate(tab, url).await {
        eprintln!("erreur : {e}");
        return ExitCode::FAILURE;
    }
    // Grâce post-nav élargie (hydratation réelle : timers +
    // XHR tardifs) puis des entrées pour déclencher
    // dispatch→synchro→journal.
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(2_000);
    while std::time::Instant::now() < deadline {
        session.pump_net().await;
        session.pump_timers(tab);
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    eprintln!(
        "[media post-nav : {} offres]",
        session.media_list(tab).len()
    );
    let mut editable_hits = 0u32;
    for round in 0..3 {
        let _ = session
            .handle_input(tab, explorer_core::InputEvent::Scroll { dy: 120.0 })
            .await;
        // Grille de survol (4×6) — couvre le header
        // (form:absent searchInput y≈52) et la colonne
        // principale ; chaque `Hover.editable` est un hit.
        for x in [100.0, 400.0, 700.0, 950.0] {
            for y in [50.0, 120.0, 200.0, 300.0, 420.0, 560.0] {
                let out = session
                    .handle_input(tab, explorer_core::InputEvent::MouseMove { x, y })
                    .await;
                if matches!(
                    out,
                    Ok(explorer_core::InputOutcome::Hover { editable: true, .. })
                ) {
                    editable_hits += 1;
                }
            }
        }
        let _ = session
            .handle_input(tab, explorer_core::InputEvent::Click { x: 100.0, y: 200.0 })
            .await;
        session.pump_net().await;
        session.pump_timers(tab);
        let nap = session
            .next_timer_deadline()
            .unwrap_or(std::time::Duration::from_millis(80));
        tokio::time::sleep(nap.min(std::time::Duration::from_millis(200))).await;
        session.pump_timers(tab);
        eprintln!("[round {round} media={}]", session.media_list(tab).len());
    }
    eprintln!(
        "[journal : {} appliqués / {} replis / {} classées]",
        session.journal_applies(),
        session.journal_fallbacks(),
        session.journal_classified()
    );
    eprintln!("[editable : {editable_hits} points sur la grille]");
    // Comptage des événements média émis pendant la sonde
    // (offered vs gone tranche « jamais scanné » vs « tué ») —
    // et des `PipelineError` JS/module : c'est le compteur de
    // terrain des erreurs de script (issue #6 — les bare
    // specifiers React y apparaissaient comme « cannot load »).
    let mut n_off = 0u32;
    let mut n_gone = 0u32;
    let mut n_mov = 0u32;
    let mut n_cmd = 0u32;
    let mut pipe_errors: Vec<String> = Vec::new();
    while let Ok(ev) = events.try_recv() {
        match ev {
            explorer_core::CoreEvent::MediaOffered { .. } => n_off += 1,
            explorer_core::CoreEvent::MediaGone { .. } => n_gone += 1,
            explorer_core::CoreEvent::MediaMoved { .. } => n_mov += 1,
            explorer_core::CoreEvent::MediaCommand { .. } => n_cmd += 1,
            explorer_core::CoreEvent::PipelineError { message, .. } => {
                pipe_errors.push(message);
            }
            _ => {}
        }
    }
    eprintln!("[media events : {n_off} offered / {n_mov} moved / {n_gone} gone / {n_cmd} cmd]");
    eprintln!("[erreurs pipeline : {}]", pipe_errors.len());
    for e in pipe_errors.iter().take(12) {
        eprintln!("  - {}", &e[..e.len().min(2000)]);
    }
    // Cycle MediaSurface réel : offres + claim d'essai.
    let offers = session.media_list(tab);
    eprintln!("[media : {} offres]", offers.len());
    for m in &offers {
        eprintln!(
            "  - #{} {:?} {}×{} {} ({:?})",
            m.id, m.kind, m.rect.w, m.rect.h, m.url, m.mime
        );
    }
    if let Some(first) = offers.first() {
        let claim = session.media_claim(tab, first.id, vec!["video/mp4".to_string()]);
        eprintln!("  claim #{} → {:?}", first.id, claim.is_ok());
        if claim.is_ok() {
            let _ = session.media_release(tab, first.id);
        }
    }
    ExitCode::SUCCESS
}

/// Lit une source : fichier local (chemin ou `file:`) ou URL
/// `http(s)` via la pile `explorer-net` complète.
async fn read_source(src: &str) -> Result<Vec<u8>, String> {
    if src.starts_with("http://") || src.starts_with("https://") {
        let client = client_stack();
        let url = Url::parse(src).map_err(|e| format!("url : {e}"))?;
        let resp = client
            .fetch(&Request::get(url))
            .await
            .map_err(|e| format!("fetch : {e}"))?;
        if !(200..300).contains(&resp.status) {
            return Err(format!("HTTP {}", resp.status));
        }
        Ok(resp.body)
    } else {
        let path = src.strip_prefix("file://").unwrap_or(src);
        std::fs::read(path).map_err(|e| format!("{path} : {e}"))
    }
}

#[tokio::main(flavor = "current_thread")]
async fn dump(src: &str) -> ExitCode {
    match read_source(src).await {
        Ok(bytes) => {
            let (html, enc) = explorer_html::encoding::decode_sniffed(&bytes);
            eprintln!("[encodage : {enc:?}]");
            let dom = TreeBuilder::parse(&html);
            print!("{}", explorer_html::debug::dump(&dom));
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("erreur : {e}");
            ExitCode::FAILURE
        }
    }
}

/// Pipeline fetch → parse → style → boîtes → wireframe.
/// `size` optionnel `LxH` : largeur de layout + env médias
/// (défaut 800 — triage terrain à la largeur fenêtre).
#[tokio::main(flavor = "current_thread")]
async fn layout(src: &str, size: Option<&str>) -> ExitCode {
    let width = size
        .and_then(|s| s.split('x').next())
        .and_then(|s| s.parse::<f32>().ok())
        .unwrap_or(800.0);
    match read_source(src).await {
        Ok(bytes) => {
            let (html, enc) = explorer_html::encoding::decode_sniffed(&bytes);
            eprintln!("[encodage : {enc:?}]");
            let dom = TreeBuilder::parse(&html);
            let mut eng = explorer_css::StyleEngine::new();
            eng.add_css(&collect_style_css(&dom), explorer_css::Origin::Author);
            // Media queries évaluées contre le viewport réel
            // du layout (800px par défaut, paramétrable).
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let env = explorer_css::StyleEnv {
                width_px: width as u32,
                ..explorer_css::StyleEnv::default()
            };
            let styles = eng.compute_tree(&dom, dom.document(), &env);
            let Some(root_el) = dom
                .children_iter(dom.document())
                .into_iter()
                .flatten()
                .find(|&c| dom.element(c).is_ok())
            else {
                eprintln!("aucun élément sous le document");
                return ExitCode::FAILURE;
            };
            if let Some(mut bx) = explorer_layout::build_box_tree(&dom, root_el, &styles) {
                explorer_layout::layout_tree(&mut bx, width, &explorer_layout::BlockCtx::default());
                print!("{}", bx.dump(&dom));
                ExitCode::SUCCESS
            } else {
                eprintln!("racine display:none — rien à peindre");
                ExitCode::FAILURE
            }
        }
        Err(e) => {
            eprintln!("erreur : {e}");
            ExitCode::FAILURE
        }
    }
}

/// Concatène le contenu des `<style>` du document (feuilles
/// liées : hors périmètre de l'étape 17).
fn collect_style_css(dom: &explorer_dom::Dom) -> String {
    let mut css = String::new();
    for n in dom.descendants(dom.document()).into_iter().flatten() {
        if dom.tag_name(n).is_ok_and(|t| t == "style") {
            if let Ok(t) = dom.text_content(n) {
                css.push_str(&t);
                css.push('\n');
            }
        }
    }
    css
}

#[tokio::main(flavor = "current_thread")]
async fn fetch(src: &str) -> ExitCode {
    match read_source(src).await {
        Ok(bytes) => {
            use std::io::Write as _;
            let _ = std::io::stdout().write_all(&bytes);
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("erreur : {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    /// `affectsv` : les familles temporelles (`animation-*`/
    /// `transition-*`) sont `na` — neutres par construction,
    /// aucun oracle géométrique statique n'existe (verrous
    /// session R-255/256/257 portent la conformité réelle).
    #[test]
    fn affectsv_familles_temporelles_neutres() {
        for d in [
            "animation-fill-mode:forwards",
            "animation-play-state:paused",
            "animation-timing-function:steps(4,end)",
            "animation-composition:add",
            "animation:spin 1s infinite",
            "transition-property:opacity",
            "transition-timing-function:steps(2)",
            "transition:all .3s",
        ] {
            assert!(super::affects_decl_verdict(d).is_none(), "{d} doit être na");
        }
        // Garde-fou : une prop géométrique reste mesurée.
        assert!(super::affects_decl_verdict("display:grid").is_some());
    }

    /// `paintv` : verrous de l'oracle pixel (étape 274) —
    /// les effets peints hors du dump géométrique deviennent
    /// `ok`, les familles hors-framebuffer restent `na`, les
    /// valeurs initiales spec/CSS-wide restent hors
    /// dénominateur.
    #[test]
    fn paintv_oracle_pixel() {
        for (d, want) in [
            // Effets peints réels — `miss` en dump, `ok` en pixels.
            ("filter:blur(2px)", Some(true)),
            ("clip-path:circle(30%)", Some(true)),
            ("background-color:#00ff00", Some(true)),
            // Familles hors framebuffer.
            ("cursor:pointer", None),
            ("scroll-snap-type:x mandatory", None),
            ("will-change:transform", None),
            ("font-kerning:normal", None),
            // Neutres par construction.
            ("color:initial", None),
            ("color:inherit", None),
        ] {
            assert_eq!(super::paint_decl_verdict(d), want, "{d}");
        }
    }

    /// `check decl` (étape 277) : la porte de la jauge sert
    /// les lints de `bcd_audit.mjs` — verrous sur
    /// `decl_gate` (chemin union) et `decl_valid`.
    #[test]
    fn decl_gate_verdicts_audit() {
        use super::{decl_gate, decl_valid, DeclGate, PAINT_NA};
        // `na` : initiales spec, CSS-wide, méta non sondé.
        for d in ["color:initial", "color:inherit", "display:bogus_meta_name"] {
            assert!(matches!(decl_gate(d, PAINT_NA, true), DeclGate::Na), "{d}");
        }
        // `probe` : sous-feature BCD substituée — latin
        // couvert → `hyphens:auto`, hors latin → miss tranché.
        match decl_gate("hyphens:language_french", PAINT_NA, true) {
            DeclGate::Probe(d) => assert_eq!(d, "hyphens:auto"),
            _ => panic!("hyphens:language_french doit sonder auto"),
        }
        match decl_gate("hyphens:language_arabic", PAINT_NA, true) {
            DeclGate::Known(false) => {}
            _ => panic!("hyphens hors latin = miss tranché"),
        }
        // `decl` : le brut passe tel quel.
        match decl_gate("color:red", PAINT_NA, true) {
            DeclGate::Probe(d) => assert_eq!(d, "color:red"),
            _ => panic!("color:red doit mesurer le brut"),
        }
        // `invalid` : le littéral échoue à la validation —
        // `200` sans unité est hors grammaire `width`.
        assert!(!decl_valid("width:200"));
        assert!(decl_valid("width:200px"));
        assert!(decl_valid("color:red"));
    }

    #[test]
    fn computedv_expansion_vs_inertie_verbatim() {
        use super::computed_decl_verdict;
        // Expansion shorthand : `inset` → 4 longhands
        // physiques → delta calculé hors prop sondée.
        assert_eq!(computed_decl_verdict("inset:10px"), Some(true));
        // Résolution logique : `block-size` → `height`
        // (`resolve_logical_props`, writing-mode horizontal).
        assert_eq!(computed_decl_verdict("block-size:44px"), Some(true));
        // Expansion + résolution composées : `margin-inline`
        // → `margin-left`/`margin-right`.
        assert_eq!(computed_decl_verdict("margin-inline:5px"), Some(true));
        // Longhand nu : stockée verbatim, exclue de la
        // signature → « cascade n'applique rien d'autre »
        // (le rendu relève des oracles geo/pixel).
        assert_eq!(computed_decl_verdict("letter-spacing:2px"), Some(false));
        // Prop logique hors `resolve_logical_props` :
        // stockée verbatim sans effet aval → miss calculé,
        // cohérent avec le routé-inerte mesuré en geo.
        assert_eq!(computed_decl_verdict("overflow-block:hidden"), Some(false));
        // `na` de valeur conservé (CSS-wide).
        assert_eq!(computed_decl_verdict("color:inherit"), None);
    }

    #[test]
    fn atrulev_descripteurs_verdicts() {
        use super::atrulev_verdict;
        // Descripteurs `CounterStyle` câblés dans `label()`
        // (R-303 — additive/negative/pad/range/fallback +
        // marqueur `outside`) → delta marqueurs réel.
        for d in [
            "symbols",
            "prefix",
            "additive-symbols",
            "fallback",
            "negative",
            "pad",
            "range",
        ] {
            assert_eq!(
                atrulev_verdict(&format!("counter-style:{d}")),
                Some(true),
                "{d}"
            );
        }
        // Feature MQ évaluée sous `StyleEnv::default`.
        assert_eq!(atrulev_verdict("media:width"), Some(true));
        // `(faux) or (faux)` : la contre-sonde écarte la
        // tolérance `unknown` — l'`ok` exige l'évaluation.
        assert_eq!(atrulev_verdict("media:or_syntax"), Some(true));
        // `@supports` hors `(prop:val)` : `selector()` sans `:`
        // serait toléré-`unknown` → sonde à `:` → miss honnête.
        assert_eq!(atrulev_verdict("supports:selector"), Some(false));
        // `@property --w{inherits:false}` honoré → `.x` voit
        // l'initial (9 px) et non le `--w` parental.
        assert_eq!(atrulev_verdict("property:inherits"), Some(true));
        // Hors portée in-process → `na`.
        assert_eq!(atrulev_verdict("font-face:font-display"), None);
        assert_eq!(atrulev_verdict("counter-style:speak-as"), None);
    }
}
