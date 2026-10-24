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
        [_, cmd, url, out] if cmd == "shot" => shot(url, out, None),
        [_, cmd, url, out, size] if cmd == "shot" => shot(url, out, Some(size)),
        [_, cmd, url] if cmd == "jprobe" => jprobe(url),
        [_, cmd, url] if cmd == "dom" => dom_live(url),
        [_, cmd, url] if cmd == "mem" => mem_report(url),
        [_, cmd, url, dir] if cmd == "save" => save_fixture(url, dir),
        [_, cmd] if cmd == "mcp" => mcp_main("9377"),
        [_, cmd, port] if cmd == "mcp" => mcp_main(port),
        [_, cmd, a, b] if cmd == "diff" => diff_pngs(a, b),
        _ => {
            eprintln!("usage : explorer-cli dump <fichier|url>");
            eprintln!("        explorer-cli fetch <url>");
            eprintln!("        explorer-cli layout <fichier|url>");
            eprintln!("        explorer-cli shot <url> <sortie.png> [LxH]");
            eprintln!("        explorer-cli jprobe <url>");
            eprintln!("        explorer-cli dom <url>");
            eprintln!("        explorer-cli mem <url>");
            eprintln!("        explorer-cli save <url> <dossier>");
            eprintln!("        explorer-cli diff <a.png> <b.png>");
            eprintln!("        explorer-cli mcp [port]  (serveur MCP stdio → daemon)");
            ExitCode::FAILURE
        }
    }
}

/// Pile réseau complète (TLS + ALPN h1/h2 + décodage +
/// redirections bornées) — partagée par `read_source` et
/// `shot`.
fn client_stack() -> Arc<dyn HttpTransport> {
    let tls = Arc::new(TlsConnector::web_roots());
    let h1: Arc<dyn HttpTransport> = Arc::new(AutoClient::new(tls, HttpConfig::default()));
    let decoded: Arc<dyn HttpTransport> = Arc::new(DecodingTransport::wrap(h1, 16 * 1024 * 1024));
    Arc::new(RedirectTransport::wrap(decoded, 8))
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

#[tokio::main(flavor = "current_thread")]
async fn shot(url: &str, out: &str, size: Option<&str>) -> ExitCode {
    let fonts = explorer_render::DirFontSource::new(&default_fonts_dir());
    let mut session = session_from_env(fonts);
    session.set_js_factory(explorer_js::JscoreEngine::factory());
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

/// `mcp [port]` — serveur MCP stdio qui wrappe l'API REST du
/// daemon (`--port` attendu : 9377 humain, 9385 agent-flight).
fn mcp_main(port: &str) -> ExitCode {
    let Ok(p) = port.parse::<u16>() else {
        eprintln!("mcp : port invalide '{port}'");
        return ExitCode::FAILURE;
    };
    match mcp::serve(p) {
        Ok(_) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("mcp : {e}");
            ExitCode::FAILURE
        }
    }
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
                .children(dom.document())
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
    let mut queue = vec![dom.document()];
    while let Some(n) = queue.pop() {
        if dom.tag_name(n).is_ok_and(|t| t == "style") {
            if let Ok(t) = dom.text_content(n) {
                css.push_str(&t);
                css.push('\n');
            }
        }
        if let Ok(ch) = dom.children(n) {
            queue.extend(ch.iter().rev());
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
