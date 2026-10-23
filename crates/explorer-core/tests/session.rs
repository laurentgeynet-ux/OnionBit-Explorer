// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Tests de la session (étape 24) — `FakeTransport`
//! in-process, aucune socket (ADR-0002) : fetch → parse →
//! style → layout → raster → PNG vérifiable, événements du
//! bus observés via `subscribe()`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use explorer_core::{
    Bookmark, CoreError, CoreEvent, InputEvent, InputOutcome, KeyMods, Session, SessionConfig,
};
use explorer_layout::font::{FontRequest, FontSource};
use explorer_net::error::NetResult;
use explorer_net::http::{HttpTransport, Request, Response};
use explorer_test_support::fixture_ttf;

/// Fonte unique : la fixture TTF synthétique partagée.
struct FixtureFont;

impl FontSource for FixtureFont {
    fn font_data(&self, _req: &FontRequest) -> Option<Vec<u8>> {
        Some(fixture_ttf())
    }
}

/// Transport simulé : table `chemin → (status, corps)`,
/// compte les requêtes pour prouver l'absence de re-fetch.
struct FakeTransport {
    routes: Mutex<HashMap<String, (u16, Vec<u8>)>>,
    calls: Mutex<u32>,
    /// Dernière requête : `(méthode, path?query, corps)` —
    /// pour vérifier les soumissions de formulaire.
    last: Mutex<Option<(String, String, Vec<u8>)>>,
    /// En-têtes de la dernière requête (asserts `Referer`,
    /// `Origin`, `Cookie` — étape 51).
    last_headers: Mutex<Vec<(String, String)>>,
    /// En-têtes par route — `path → en-têtes de réponse`
    /// (ACAO pour les tests CORS).
    resp_headers: Mutex<HashMap<String, Vec<(String, String)>>>,
    /// Corps streamés par route — `path → morceaux` (étape 53 ;
    /// les routes ordinaires passent par le corps unique de
    /// `routes` via le défaut du trait).
    chunks: Mutex<HashMap<String, Vec<Vec<u8>>>>,
    /// Journal des `path?query` requis (étape 96 — prouver
    /// qu'une feuille hors-media/`@import` n'a *pas* été
    /// fetchée, pas seulement qu'elle n'est pas appliquée).
    paths: Mutex<Vec<String>>,
    /// Journal `(path, en-têtes)` par requête (étape 128 —
    /// les métadonnées `sec-fetch-*`/`cookie` d'un fetch
    /// `<iframe>` ne sont pas forcément la *dernière*
    /// requête : `last_headers` ne suffit pas).
    req_log: Mutex<ReqLog>,
}

/// `(path, en-têtes de requête)` par requête émise (étape 128).
type ReqLog = Vec<(String, Vec<(String, String)>)>;

/// R-32 : la feuille UA pose `body { margin: 8px }` (compat
/// navigateurs — les refs WPT la supposent). Les fixtures
/// historiques ont été écrites pour une marge nulle : elle
/// est neutralisée ici pour préserver la géométrie attendue
/// (clics à coordonnées fixes, comparaisons de frames). La
/// règle est injectée dans `<head>` — créé au besoin — sans
/// toucher aux enfants de `<body>` ; une page qui pose sa
/// propre marge garde la sienne (son `<style>` vient après).
fn zero_body_margin(body: &str) -> String {
    const STYLE: &str = "<style>body{margin:0}</style>";
    if let Some(i) = body.find("<head>") {
        let mut s = String::with_capacity(body.len() + STYLE.len());
        s.push_str(&body[..i + 6]);
        s.push_str(STYLE);
        s.push_str(&body[i + 6..]);
        s
    } else if let Some(i) = body.find("<body") {
        let mut s = String::with_capacity(body.len() + 13 + STYLE.len());
        s.push_str(&body[..i]);
        s.push_str("<head>");
        s.push_str(STYLE);
        s.push_str("</head>");
        s.push_str(&body[i..]);
        s
    } else {
        body.to_string()
    }
}

impl FakeTransport {
    fn new() -> Self {
        Self {
            routes: Mutex::new(HashMap::new()),
            calls: Mutex::new(0),
            last: Mutex::new(None),
            last_headers: Mutex::new(Vec::new()),
            resp_headers: Mutex::new(HashMap::new()),
            chunks: Mutex::new(HashMap::new()),
            paths: Mutex::new(Vec::new()),
            req_log: Mutex::new(Vec::new()),
        }
    }

    fn route(&self, path: &str, status: u16, body: &'static str) {
        self.route_bin(path, status, zero_body_margin(body).into_bytes());
    }

    /// Corps binaire (images & co).
    fn route_bin(&self, path: &str, status: u16, body: Vec<u8>) {
        if let Ok(mut r) = self.routes.lock() {
            r.insert(path.to_string(), (status, body));
        }
    }

    /// Route avec en-têtes de réponse (CORS, Set-Cookie).
    fn route_headers(&self, path: &str, status: u16, body: &'static str, headers: &[(&str, &str)]) {
        self.route(path, status, body);
        if let Ok(mut h) = self.resp_headers.lock() {
            h.insert(
                path.to_string(),
                headers
                    .iter()
                    .map(|(n, v)| (n.to_string(), v.to_string()))
                    .collect(),
            );
        }
    }

    /// Route à corps streamé — les morceaux arrivent tels quels
    /// sur le canal (frontières de chunk arbitraires, y compris
    /// au milieu d'une balise ou d'un caractère multi-octets).
    fn route_chunks(&self, path: &str, status: u16, parts: Vec<Vec<u8>>) {
        if let Ok(mut r) = self.routes.lock() {
            r.insert(path.to_string(), (status, Vec::new()));
        }
        if let Ok(mut c) = self.chunks.lock() {
            c.insert(path.to_string(), parts);
        }
    }

    /// Requêtes servies.
    fn calls(&self) -> u32 {
        self.calls.lock().map_or(0, |c| *c)
    }

    /// Dernière requête enregistrée.
    fn last(&self) -> Option<(String, String, Vec<u8>)> {
        self.last.lock().ok().and_then(|l| l.clone())
    }

    /// `path?query` de toutes les requêtes, dans l'ordre.
    fn paths(&self) -> Vec<String> {
        self.paths.lock().map_or_else(|_| Vec::new(), |p| p.clone())
    }

    /// Valeur d'un en-tête de la dernière requête.
    fn last_header(&self, name: &str) -> Option<String> {
        self.last_headers.lock().ok().and_then(|h| {
            h.iter()
                .find(|(n, _)| n.eq_ignore_ascii_case(name))
                .map(|(_, v)| v.clone())
        })
    }

    /// Comptabilisation commune `fetch`/`fetch_stream`.
    fn record(&self, req: &Request) {
        if let Ok(mut c) = self.calls.lock() {
            *c += 1;
        }
        if let Ok(mut l) = self.last.lock() {
            *l = Some((
                format!("{:?}", req.method),
                req.url.path_query(),
                req.body.clone().unwrap_or_default(),
            ));
        }
        if let Ok(mut h) = self.last_headers.lock() {
            h.clone_from(&req.headers);
        }
        if let Ok(mut p) = self.paths.lock() {
            p.push(req.url.path_query());
        }
        if let Ok(mut l) = self.req_log.lock() {
            l.push((req.url.path().to_string(), req.headers.clone()));
        }
    }

    /// En-têtes de la requête émise vers `path` (premier
    /// match) — étape 128 : asserts `sec-fetch-*`/`cookie`
    /// d'un fetch `<iframe>` ou d'une sous-ressource non
    /// terminale.
    fn req_headers(&self, path: &str) -> Option<Vec<(String, String)>> {
        self.req_log
            .lock()
            .ok()
            .and_then(|l| l.iter().find(|(p, _)| p == path).map(|(_, h)| h.clone()))
    }

    /// Valeur d'en-tête de la requête vers `path`.
    fn req_header(&self, path: &str, name: &str) -> Option<String> {
        self.req_headers(path).and_then(|h| {
            h.iter()
                .find(|(n, _)| n.eq_ignore_ascii_case(name))
                .map(|(_, v)| v.clone())
        })
    }

    /// Réponse simulée — la même pour `fetch` et `fetch_stream`.
    fn serve(&self, req: &Request) -> (u16, Vec<(String, String)>, Vec<u8>) {
        let found = self
            .routes
            .lock()
            .ok()
            .and_then(|r| r.get(req.url.path()).cloned());
        let headers = self
            .resp_headers
            .lock()
            .ok()
            .and_then(|h| h.get(req.url.path()).cloned())
            .unwrap_or_default();
        match found {
            Some((status, body)) => (status, headers, body),
            None => (404, headers, b"missing".to_vec()),
        }
    }
}

#[async_trait]
impl HttpTransport for FakeTransport {
    async fn fetch(&self, req: &Request) -> NetResult<Response> {
        self.record(req);
        let (status, headers, body) = self.serve(req);
        Ok(Response {
            status,
            reason: "fake".to_string(),
            headers,
            body,
            reused_connection: false,
            resolved_url: None,
        })
    }

    /// Sert les routes `route_chunks` morceau par morceau ;
    /// les autres envoient leur corps en un seul morceau.
    async fn fetch_stream(&self, req: &Request) -> NetResult<explorer_net::http::StreamedResponse> {
        self.record(req);
        let parts = self
            .chunks
            .lock()
            .ok()
            .and_then(|c| c.get(req.url.path()).cloned());
        let (status, headers, body) = self.serve(req);
        let parts = parts.unwrap_or_else(|| vec![body]);
        let (tx, rx) = tokio::sync::mpsc::channel(parts.len().max(1));
        for p in parts {
            let _ = tx.try_send(Ok(p));
        }
        Ok(explorer_net::http::StreamedResponse {
            status,
            reason: "fake".to_string(),
            headers,
            reused_connection: false,
            resolved_url: None,
            body: rx,
        })
    }
}

/// Session de test prête : transport + fixture.
fn test_session() -> (Session, Arc<FakeTransport>) {
    test_session_with(SessionConfig {
        event_capacity: 16,
        viewport: (800.0, 600.0),
        scale: 1.0,
        js_ops: 5_000_000,
        max_images: 32,
        image_max_px: 16_000_000,
        max_sheets: 8,
        max_imports: 8,
        max_fonts: 16,
        max_scripts: 32,
        max_frames: 8,
        max_modules: 64,
        max_inflight: 6,
        max_script_net: 16,
        max_timer_jobs: 64,
        paint_chunk_bytes: 32 * 1024,
        paint_interval_ms: 100,
        resync_gap_max_ms: 4_000,
        max_download_bytes: 64 * 1024 * 1024,
        bfcache_pages: 4,
        dom_journal_max_ops: 512,
        js_mem_limit_mib: 512,
        color_scheme: explorer_css::ColorScheme::default(),
    })
}

/// `test_session` paramétrable — les tests de réglage
/// (bornes heap…) surchargent la config sans dupliquer le
/// transport/fixture (étape 123).
fn test_session_with(cfg: SessionConfig) -> (Session, Arc<FakeTransport>) {
    let t = Arc::new(FakeTransport::new());
    let s = Session::with_config(t.clone(), Arc::new(FixtureFont), cfg);
    (s, t)
}

/// HTML minimal peinture garantie (fond UA + bloc).
const PAGE: &str = "<!doctype html><html><body>\
     <div>bonjour monde</div></body></html>";

/// Page avec `<title>`.
const PAGE_T: &str = "<!doctype html><html><head>\
     <title>Page A</title></head><body><div>a</div></body></html>";

/// Page avec un lien en haut à gauche.
const PAGE_LINK: &str = "<!doctype html><html><body>\
     <a href=\"/b\">lien vers b</a></body></html>";

/// Page haute (déborde le viewport 800×600 → scrollable).
fn tall_page() -> &'static str {
    "<!doctype html><html><head><style>#t{height:2000px}</style></head>\
     <body><div id=\"t\">haut</div></body></html>"
}

#[tokio::test]
async fn open_tab_emits_event() {
    let (mut s, _t) = test_session();
    let mut rx = s.subscribe();
    let id = s.open_tab();
    assert!(matches!(
        rx.try_recv(),
        Ok(CoreEvent::TabOpened { tab }) if tab == id
    ));
    assert_eq!(s.tabs(), vec![id]);
}

#[tokio::test]
async fn navigate_produces_png_and_events() {
    let (mut s, t) = test_session();
    t.route("/", 200, PAGE);
    let mut rx = s.subscribe();
    let id = s.open_tab();
    s.navigate(id, "http://h.test/").await.unwrap();

    let png = s.frame_png(id).unwrap();
    // Signature PNG.
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
    assert_eq!(s.tab_url(id).as_deref(), Some("http://h.test/"));

    // Événements observés dans l'ordre.
    let mut saw_nav = false;
    let mut saw_inv = false;
    while let Ok(ev) = rx.try_recv() {
        match ev {
            CoreEvent::Navigated { tab, .. } if tab == id => saw_nav = true,
            CoreEvent::FrameInvalidated { tab, .. } if tab == id => saw_inv = true,
            _ => {}
        }
    }
    assert!(saw_nav && saw_inv);
}

#[tokio::test]
async fn frame_absent_before_navigate() {
    let (mut s, _t) = test_session();
    let id = s.open_tab();
    assert!(s.frame_png(id).is_none());
}

#[tokio::test]
async fn bad_url_is_typed_error() {
    let (mut s, _t) = test_session();
    let id = s.open_tab();
    assert!(matches!(
        s.navigate(id, "pas une url").await,
        Err(CoreError::Url(_))
    ));
}

#[tokio::test]
async fn http_error_is_typed_and_emits_pipeline_error() {
    let (mut s, t) = test_session();
    t.route("/", 500, "boom");
    let mut rx = s.subscribe();
    let id = s.open_tab();
    assert!(matches!(
        s.navigate(id, "http://h.test/").await,
        Err(CoreError::HttpStatus(500))
    ));
    let mut saw_err = false;
    while let Ok(ev) = rx.try_recv() {
        if matches!(ev, CoreEvent::PipelineError { tab: Some(t), .. } if t == id) {
            saw_err = true;
        }
    }
    assert!(saw_err);
}

/// Étape 69 — une nav en échec commet une page d'erreur
/// peinte par le moteur : URL tentée en barre, cause
/// lisible dans le document, `can_back` préservé (la
/// page précédente reste restaurée par l'historique).
#[tokio::test]
async fn failed_nav_commits_error_page() {
    let (mut s, t) = test_session();
    t.route("/ok", 200, PAGE_T);
    t.route("/ok2", 200, PAGE);
    t.route("/ko", 404, "nope");
    let id = s.open_tab();
    s.navigate(id, "http://h.test/ok").await.unwrap();
    s.navigate(id, "http://h.test/ok2").await.unwrap();
    assert!(s.tab_info(id).is_some_and(|i| i.can_back));
    assert_eq!(s.find_in_page(id, "Page inaccessible"), 0);
    assert!(matches!(
        s.navigate(id, "http://h.test/ko").await,
        Err(CoreError::HttpStatus(404))
    ));
    let info = s.tab_info(id).unwrap();
    assert_eq!(info.url.as_deref(), Some("http://h.test/ko"));
    assert_eq!(info.title.as_deref(), Some("Erreur de chargement"));
    assert!(info.can_back, "l'historique ne doit pas être consumé");
    // Le document commis est la page d'erreur, pas la page
    // précédente ni un vide.
    assert!(s.find_in_page(id, "Page inaccessible") > 0);
    assert!(s.find_in_page(id, "HTTP 404") > 0);
    // Retour arrière (étape 76) : la page précédente `ok2` est
    // restaurée — la nav échouée a sa propre entrée, la
    // cible n'est pas sautée.
    s.go_back(id).await.unwrap();
    assert_eq!(
        s.tab_info(id).and_then(|i| i.url).as_deref(),
        Some("http://h.test/ok2")
    );
}

/// Étape 75 — le surlignage `find` de la page précédente ne
/// cuit pas dans la frame d'erreur : la purge interactive a
/// lieu AVANT le relayout (avant : `find` n'était effacé
/// qu'après `relayout_dom` → bande orange peinte sur le 404).
#[tokio::test]
async fn failed_nav_error_page_has_no_stale_find_highlight() {
    let (mut s, t) = test_session();
    t.route("/ok", 200, PAGE);
    t.route("/ko", 404, "nope");
    let id = s.open_tab();
    s.navigate(id, "http://h.test/ok").await.unwrap();
    assert!(s.find_in_page(id, "bonjour") > 0);
    // Surbrillance find ≈ orange (255,152,0,α) / jaune
    // (255,235,59,α) blendée — pixels chauds r≈255, g 120-250,
    // b < 150.
    let warm = |px: &[u8]| {
        px.chunks_exact(4)
            .filter(|p| p[0] > 230 && p[1] > 110 && p[1] < 250 && p[2] < 150)
            .count()
    };
    {
        let (_, _, px) = s.frame_pixels(id).unwrap();
        assert!(warm(px) > 0, "le find doit peindre son surlignage");
    }
    assert!(matches!(
        s.navigate(id, "http://h.test/ko").await,
        Err(CoreError::HttpStatus(404))
    ));
    assert!(s.find_state(id).is_none(), "le find est purgé");
    let (_, _, px) = s.frame_pixels(id).unwrap();
    assert_eq!(warm(px), 0, "aucun reste de find sur la page d'erreur");
}

/// Étape 76 — la navigation en échec pousse son entrée
/// d'historique (sémantique Chrome/Firefox) : `back` depuis
/// la page d'erreur revient à la page précédente (la cible
/// n'est pas sautée), `forward` retente la cible — jamais
/// de page d'erreur gelée via bfcache.
#[tokio::test]
async fn failed_nav_pushes_history_entry_and_retries() {
    let (mut s, t) = test_session();
    t.route("/a", 200, PAGE);
    t.route("/b", 200, PAGE_T);
    t.route("/ko", 404, "nope");
    let id = s.open_tab();
    s.navigate(id, "http://h.test/a").await.unwrap();
    s.navigate(id, "http://h.test/b").await.unwrap();
    assert!(matches!(
        s.navigate(id, "http://h.test/ko").await,
        Err(CoreError::HttpStatus(404))
    ));
    // Position = l'entrée d'erreur : can_back ET can_forward
    // faux — l'entrée est la dernière.
    let info = s.tab_info(id).unwrap();
    assert!(info.can_back, "B doit être accessible par back");
    assert!(!info.can_forward, "l'entrée d'erreur est terminale");
    // `back` revient à B — la cible échouée n'est pas sautée.
    s.go_back(id).await.unwrap();
    assert_eq!(
        s.tab_info(id).and_then(|i| i.url).as_deref(),
        Some("http://h.test/b")
    );
    // `forward` retente la cible (pas de snapshot d'erreur) —
    // le 404 renvoie la page d'erreur à la position de
    // l'entrée.
    assert!(matches!(
        s.go_forward(id).await,
        Err(CoreError::HttpStatus(404))
    ));
    let info = s.tab_info(id).unwrap();
    assert_eq!(info.url.as_deref(), Some("http://h.test/ko"));
    assert_eq!(info.title.as_deref(), Some("Erreur de chargement"));
    assert!(info.can_back && !info.can_forward);
    // Et `back` re-revient à B (pas à A — la position est
    // bien l'entrée en échec).
    s.go_back(id).await.unwrap();
    assert_eq!(
        s.tab_info(id).and_then(|i| i.url).as_deref(),
        Some("http://h.test/b")
    );
}

/// Étape 69 — le chemin hors-verrou (`navigate_shared`)
/// commet la même page d'erreur.
#[tokio::test]
async fn failed_nav_shared_commits_error_page() {
    use explorer_net::url::Url;
    let (s, t) = test_session();
    t.route("/ko", 500, "boom");
    let sess = std::sync::Arc::new(tokio::sync::Mutex::new(s));
    let id = sess.lock().await.open_tab();
    let req = Request::get(Url::parse("http://h.test/ko").unwrap());
    assert!(matches!(
        Box::pin(explorer_core::navigate_shared(&sess, id, req, true)).await,
        Err(CoreError::HttpStatus(500))
    ));
    let mut g = sess.lock().await;
    assert_eq!(
        g.tab_info(id).and_then(|i| i.url).as_deref(),
        Some("http://h.test/ko")
    );
    assert!(g.find_in_page(id, "Page inaccessible") > 0);
}

/// Étape 70 — un octet-stream top-level est bufferisé et
/// proposé (`DownloadOffered`) : la page courante reste,
/// rien n'est committé ; `accept` écrit le corps.
#[tokio::test]
async fn download_offered_buffers_and_keeps_page() {
    let (mut s, t) = test_session();
    t.route("/page", 200, PAGE_T);
    t.route_headers(
        "/file.bin",
        200,
        "\u{0}PK\u{3}\u{4}binary",
        &[
            ("Content-Type", "application/octet-stream"),
            ("Content-Disposition", "attachment; filename=\"x.bin\""),
        ],
    );
    let id = s.open_tab();
    s.navigate(id, "http://h.test/page").await.unwrap();
    let mut rx = s.subscribe();
    // La nav vers un attachment aboutit sans commettre :
    // pas d'URL, pas d'historique — la page reste.
    s.navigate(id, "http://h.test/file.bin").await.unwrap();
    assert_eq!(
        s.tab_info(id).unwrap().url.as_deref(),
        Some("http://h.test/page")
    );
    let pending = s.pending_downloads(id);
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].filename, "x.bin");
    assert_eq!(pending[0].mime, "application/octet-stream");
    assert_eq!(pending[0].size, 11);
    let mut offered = false;
    while let Ok(ev) = rx.try_recv() {
        if matches!(ev, CoreEvent::DownloadOffered { filename, .. } if filename == "x.bin") {
            offered = true;
        }
    }
    assert!(offered, "DownloadOffered attendu");
    // Acceptation → fichier écrit + DownloadDone.
    let dir = std::env::temp_dir().join("explorer-dl-test");
    let target = s
        .accept_download(id, pending[0].id, Some(dir.join("out.bin")))
        .unwrap();
    assert_eq!(std::fs::read(&target).unwrap().len(), 11);
    assert!(s.pending_downloads(id).is_empty());
    let mut done = false;
    while let Ok(ev) = rx.try_recv() {
        if matches!(ev, CoreEvent::DownloadDone { .. }) {
            done = true;
        }
    }
    assert!(done, "DownloadDone attendu");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Étape 70 — le rejet rejoue le corps comme document
/// (dégradation « dump texte » historique) : l'URL est
/// committée et le contenu peint.
#[tokio::test]
async fn decline_download_replays_as_document() {
    let (mut s, t) = test_session();
    t.route("/page", 200, PAGE_T);
    t.route_headers(
        "/data.bin",
        200,
        "<b>blob texte</b>",
        &[("Content-Type", "application/octet-stream")],
    );
    let id = s.open_tab();
    s.navigate(id, "http://h.test/page").await.unwrap();
    s.navigate(id, "http://h.test/data.bin").await.unwrap();
    let dl = s.pending_downloads(id)[0].id;
    s.decline_download(id, dl).await.unwrap();
    assert_eq!(
        s.tab_info(id).unwrap().url.as_deref(),
        Some("http://h.test/data.bin")
    );
    assert!(s.find_in_page(id, "blob texte") > 0);
    assert!(s.pending_downloads(id).is_empty());
}

/// Étape 70 — un type documentaire (`text/plain`, HTML
/// sans type déclaré) ne part jamais en téléchargement.
#[tokio::test]
async fn document_types_never_download() {
    let (mut s, t) = test_session();
    t.route_headers(
        "/notes",
        200,
        "plain body",
        &[("Content-Type", "text/plain")],
    );
    let id = s.open_tab();
    s.navigate(id, "http://h.test/notes").await.unwrap();
    assert!(s.pending_downloads(id).is_empty());
    assert_eq!(
        s.tab_info(id).unwrap().url.as_deref(),
        Some("http://h.test/notes")
    );
}

#[tokio::test]
async fn unknown_tab_is_typed_error() {
    let (mut s, _t) = test_session();
    assert!(matches!(
        s.navigate(99, "http://h.test/").await,
        Err(CoreError::UnknownTab(99))
    ));
    assert!(matches!(s.rerender(99), Err(CoreError::UnknownTab(99))));
}

#[tokio::test]
async fn rerender_uses_cached_dom() {
    let (mut s, t) = test_session();
    t.route("/", 200, PAGE);
    let mut rx = s.subscribe();
    let id = s.open_tab();
    s.navigate(id, "http://h.test/").await.unwrap();
    let before = s.frame_png(id).unwrap();

    s.set_viewport(400.0, 300.0, 1.0);
    s.rerender(id).unwrap();
    let after = s.frame_png(id).unwrap();

    // Re-render à viewport réduit : PNG différent, AUCUN
    // nouveau fetch (le DOM est l'état conservé).
    assert_ne!(before, after);
    let calls = *t.calls.lock().unwrap();
    assert_eq!(calls, 1);
    let mut saw_inv = false;
    while let Ok(e) = rx.try_recv() {
        if matches!(e, CoreEvent::FrameInvalidated { tab, .. } if tab == id) {
            saw_inv = true;
        }
    }
    assert!(saw_inv);
}

#[tokio::test]
async fn invalid_viewport_is_ignored() {
    let (mut s, _t) = test_session();
    s.set_viewport(0.0, -5.0, 1.0);
    assert_eq!(s.viewport(), (800.0, 600.0));
}

/// Étape 94 (demande PIPE→NAV) — les media queries sont
/// évaluées contre le viewport RÉEL de la session (800×600
/// ici), pas le `StyleEnv::default()` 1024×768 :
/// `@media (max-width:900px)` doit matcher à 800px — le bloc
/// rouge `display:none`'d n'apparaît nulle part au raster.
#[tokio::test]
async fn media_query_uses_session_viewport() {
    let (mut s, t) = test_session();
    t.route(
        "/",
        200,
        "<!doctype html><html><head><style>\
         div { height: 300px; }\
         #gone { background: #ff0000; }\
         #stay { background: #00ff00; }\
         @media (max-width: 900px) { #gone { display: none; } }\
         </style></head><body>\
         <div id=gone></div><div id=stay></div>\
         </body></html>",
    );
    let id = s.open_tab();
    s.navigate(id, "http://h.test/").await.unwrap();
    let (fw, fh, px) = s.frame_pixels(id).unwrap();
    let (fw, fh) = (fw as usize, fh as usize);
    let mut red = 0usize;
    let mut green = 0usize;
    for i in 0..fw * fh {
        let p = &px[i * 4..][..4];
        if p[0] > 200 && p[1] < 60 && p[2] < 60 {
            red += 1;
        }
        if p[1] > 200 && p[0] < 60 && p[2] < 60 {
            green += 1;
        }
    }
    // Viewport 800 ≤ 900 : `#gone` masqué, `#stay` visible.
    assert_eq!(
        red, 0,
        "viewport réel 800px : la règle max-width:900 s'applique"
    );
    assert!(green > 0, "le bloc hors media query reste peint");
}

/// Étape 97 — `SessionConfig.color_scheme` propagé à
/// `StyleEnv` : `@media (prefers-color-scheme: dark)` est
/// inerte sous l'env `light` par défaut, active après
/// `set_color_scheme(Dark)` + `rerender` (réglage OS/app
/// basculé en vol — même mécanisme que le resize).
#[tokio::test]
async fn color_scheme_dark_active_prefers_dark() {
    let (mut s, t) = test_session();
    t.route(
        "/",
        200,
        "<!doctype html><html><head><style>\
         div { height: 300px; }\
         @media (prefers-color-scheme: dark) {\
         #d { background: #00ff00; }\
         }\
         </style></head><body><div id=d></div></body></html>",
    );
    let id = s.open_tab();
    s.navigate(id, "http://h.test/").await.unwrap();
    // Env `light` (défaut) : la variante sombre est absente.
    let (fw, fh, px) = s.frame_pixels(id).unwrap();
    assert_eq!(
        count_color(fw as usize, fh as usize, px, [0, 255, 0]),
        0,
        "env light : la règle prefers-color-scheme:dark reste inactive"
    );
    // Bascule runtime (réglage app) → le re-render applique.
    s.set_color_scheme(explorer_core::ColorScheme::Dark);
    s.rerender(id).unwrap();
    let (fw, fh, px) = s.frame_pixels(id).unwrap();
    assert!(
        count_color(fw as usize, fh as usize, px, [0, 255, 0]) > 0,
        "env dark : la règle prefers-color-scheme:dark s'applique"
    );
}

/// Fenêtre raster (consommation R-62) : un document plus haut
/// que le cap 16 384 px reste scrollable — la frame peint la
/// tranche viewport-ancrée, pas seulement le haut du document
/// (github/MDN/stallman : le scroll n'y montrait rien).
#[tokio::test]
async fn tall_document_scrolls_past_raster_cap() {
    let (mut s, t) = test_session();
    t.route(
        "/",
        200,
        "<!doctype html><html><head><style>\
         body{margin:0} #a{height:17000px;background:#00ff00}\
         #b{height:3000px;background:#ff0000}\
         </style></head><body><div id=a></div><div id=b></div></body></html>",
    );
    let id = s.open_tab();
    s.navigate(id, "http://h.test/").await.unwrap();
    // Scroll profond : la bande rouge (y∈[17000,20000]) doit
    // remplir la fenêtre — doc 20 000 px > cap 16 384.
    s.handle_input(id, InputEvent::Scroll { dy: 18000.0 })
        .await
        .unwrap();
    let (fw, fh, px) = s.frame_pixels(id).unwrap();
    // Fenêtrée : la frame couvre le viewport (800×600), pas la
    // tranche document de 16 384 px — le cap est sur la fenêtre.
    assert_eq!((fw, fh), (800, 600), "frame fenêtrée au viewport");
    assert!(
        count_color(fw as usize, fh as usize, px, [255, 0, 0]) > 0,
        "le contenu au-delà du cap 16 384 est peint"
    );
    // Fond du document (scroll max = 20 000 − 600 = 19 400) —
    // tout rouge.
    s.handle_input(id, InputEvent::Scroll { dy: 99_999.0 })
        .await
        .unwrap();
    let (fw, fh, px) = s.frame_pixels(id).unwrap();
    assert_eq!(
        count_color(fw as usize, fh as usize, px, [0, 255, 0]),
        0,
        "fond de doc : aucun pixel vert de la bande du haut"
    );
    // Retour en haut : la fenêtre suit — bande verte visible.
    s.handle_input(id, InputEvent::Scroll { dy: -99_999.0 })
        .await
        .unwrap();
    let (fw, fh, px) = s.frame_pixels(id).unwrap();
    assert!(
        count_color(fw as usize, fh as usize, px, [0, 255, 0]) > 0,
        "retour en haut : bande verte repeinte"
    );
}

/// Compte les pixels d'une teinte dans la frame `px`.
fn count_color(fw: usize, fh: usize, px: &[u8], target: [u8; 3]) -> usize {
    let mut n = 0;
    for i in 0..fw * fh {
        let p = &px[i * 4..][..3];
        let [tr, tg, tb] = target.map(i32::from);
        let d = |c: u8, t: i32| (i32::from(c) - t).abs() < 30;
        if d(p[0], tr) && d(p[1], tg) && d(p[2], tb) {
            n += 1;
        }
    }
    n
}

/// Étape 96 : `<link media>` est évalué contre le viewport
/// réel **avant** la borne `max_sheets` — une feuille
/// hors-media n'est ni fetchée ni appliquée, une feuille à
/// media conforme l'est.
#[tokio::test]
async fn link_media_filtered_against_viewport() {
    let (mut s, t) = test_session();
    t.route(
        "/",
        200,
        "<!doctype html><html><head>\
         <link rel=stylesheet href=/keep.css>\
         <link rel=stylesheet href=/print.css media=print>\
         <link rel=stylesheet href=\"/wide.css\" media=\"(min-width:2000px)\">\
         <link rel=stylesheet href=\"/mq.css\" media=\"screen and (max-width:900px)\">\
         </head><body>\
         <div id=k style=\"width:50px;height:50px\"></div>\
         <div id=p style=\"width:50px;height:50px\"></div>\
         <div id=w style=\"width:50px;height:50px\"></div>\
         <div id=m style=\"width:50px;height:50px\"></div>\
         </body></html>",
    );
    t.route_bin("/keep.css", 200, b"#k{background:#00ff00}".to_vec());
    t.route_bin("/print.css", 200, b"#p{background:#ff0000}".to_vec());
    t.route_bin("/wide.css", 200, b"#w{background:#ff0000}".to_vec());
    t.route_bin("/mq.css", 200, b"#m{background:#0000ff}".to_vec());
    let id = s.open_tab();
    s.navigate(id, "http://h.test/").await.unwrap();
    let (fw, fh, px) = s.frame_pixels(id).unwrap();
    let (fw, fh) = (fw as usize, fh as usize);
    assert!(
        count_color(fw, fh, px, [0, 255, 0]) > 0,
        "keep.css appliquée"
    );
    assert!(
        count_color(fw, fh, px, [0, 0, 255]) > 0,
        "mq.css (max-width:900) appliquée à 800px"
    );
    assert_eq!(
        count_color(fw, fh, px, [255, 0, 0]),
        0,
        "feuilles hors-media non appliquées"
    );
    let paths = t.paths();
    assert!(
        paths.iter().any(|p| p == "/keep.css"),
        "keep fetchée : {paths:?}"
    );
    assert!(
        paths.iter().any(|p| p == "/mq.css"),
        "mq fetchée : {paths:?}"
    );
    assert!(
        !paths.iter().any(|p| p == "/print.css"),
        "print non fetchée : {paths:?}"
    );
    assert!(
        !paths.iter().any(|p| p == "/wide.css"),
        "wide non fetchée : {paths:?}"
    );
}

/// Étape 97 — variante lobste.rs : `<link media=
/// "(prefers-color-scheme: dark)">` est fetchée **uniquement**
/// en env sombre (le shim `media_attr_matches` de l'étape 96
/// hérite `color_scheme` via `StyleEnv` — R-59 gratuite
/// côté NAV). En `dark`, c'est `dark-normal` qui arrive,
/// pas `light-normal`.
#[tokio::test]
async fn link_prefers_color_scheme_dark_selects_variant() {
    let (mut s, t) = test_session();
    t.route(
        "/",
        200,
        "<!doctype html><html><head>\
         <link rel=stylesheet href=/light-normal.css \
         media=\"(prefers-color-scheme: light)\">\
         <link rel=stylesheet href=/dark-normal.css \
         media=\"(prefers-color-scheme: dark)\">\
         </head><body>\
         <div id=v style=\"width:50px;height:50px\"></div>\
         </body></html>",
    );
    t.route_bin("/light-normal.css", 200, b"#v{background:#ff0000}".to_vec());
    t.route_bin("/dark-normal.css", 200, b"#v{background:#00ff00}".to_vec());
    let id = s.open_tab();
    // Env sombre avant la navigation : la variante dark est
    // planifiée, la light filtrée.
    s.set_color_scheme(explorer_core::ColorScheme::Dark);
    s.navigate(id, "http://h.test/").await.unwrap();
    let paths = t.paths();
    assert!(
        paths.iter().any(|p| p == "/dark-normal.css"),
        "dark-normal fetchée en env dark : {paths:?}"
    );
    assert!(
        !paths.iter().any(|p| p == "/light-normal.css"),
        "light-normal filtrée en env dark : {paths:?}"
    );
    let (fw, fh, px) = s.frame_pixels(id).unwrap();
    assert!(
        count_color(fw as usize, fh as usize, px, [0, 255, 0]) > 0,
        "dark-normal appliquée"
    );
    assert_eq!(
        count_color(fw as usize, fh as usize, px, [255, 0, 0]),
        0,
        "light-normal absente"
    );
}

/// Étape 96 : media inclassable/malformée → tolérance — la
/// feuille est conservée (on ne masque jamais par doute).
#[tokio::test]
async fn link_media_malformed_is_tolerant() {
    let (mut s, t) = test_session();
    t.route(
        "/",
        200,
        "<!doctype html><html><head>\
         <link rel=stylesheet href=\"/odd.css\" media=\"(((\">\
         </head><body><div id=d style=\"width:50px;height:50px\"></div></body></html>",
    );
    t.route_bin("/odd.css", 200, b"#d{background:#00ff00}".to_vec());
    let id = s.open_tab();
    s.navigate(id, "http://h.test/").await.unwrap();
    let (fw, fh, px) = s.frame_pixels(id).unwrap();
    assert!(
        count_color(fw as usize, fh as usize, px, [0, 255, 0]) > 0,
        "media malformée → feuille gardée et appliquée"
    );
}

/// Étape 96 : la borne `max_sheets` s'applique APRÈS le
/// filtre media — les liens `print` en tête ne peuvent plus
/// affamer les feuilles effectives.
#[tokio::test]
async fn max_sheets_counts_after_media_filter() {
    let t = Arc::new(FakeTransport::new());
    let mut s = Session::with_config(
        t.clone(),
        Arc::new(FixtureFont),
        SessionConfig {
            viewport: (800.0, 600.0),
            max_sheets: 2,
            ..SessionConfig::default()
        },
    );
    t.route(
        "/",
        200,
        "<!doctype html><html><head>\
         <link rel=stylesheet href=/p1.css media=print><link rel=stylesheet href=/p2.css media=print>\
         <link rel=stylesheet href=/a.css><link rel=stylesheet href=/b.css>\
         </head><body>\
         <div id=a style=\"width:50px;height:50px\"></div>\
         <div id=b style=\"width:50px;height:50px\"></div>\
         </body></html>",
    );
    t.route_bin("/a.css", 200, b"#a{background:#00ff00}".to_vec());
    t.route_bin("/b.css", 200, b"#b{background:#0000ff}".to_vec());
    t.route_bin("/p1.css", 200, b"#a{background:#ff0000}".to_vec());
    t.route_bin("/p2.css", 200, b"#b{background:#ff0000}".to_vec());
    let id = s.open_tab();
    s.navigate(id, "http://h.test/").await.unwrap();
    let (fw, fh, px) = s.frame_pixels(id).unwrap();
    let (fw, fh) = (fw as usize, fh as usize);
    assert!(count_color(fw, fh, px, [0, 255, 0]) > 0);
    assert!(count_color(fw, fh, px, [0, 0, 255]) > 0);
    let paths = t.paths();
    assert!(
        !paths.iter().any(|p| p == "/p1.css" || p == "/p2.css"),
        "{paths:?}"
    );
}

/// Étape 96 : `<link>` en doublon (même URL résolue) → un
/// seul fetch et un seul slot — les pages à liens générés
/// (GitHub ~7 doublons) ne brûlent plus ni quota ni
/// requête.
#[tokio::test]
async fn stylesheet_href_dedup() {
    let (mut s, t) = test_session();
    t.route(
        "/",
        200,
        "<!doctype html><html><head>\
         <link rel=stylesheet href=/a.css>\
         <link rel=stylesheet href=/a.css>\
         <link rel=stylesheet href=\"./a.css\">\
         <link rel=stylesheet href=/b.css>\
         </head><body><div id=d style=\"width:50px;height:50px\"></div></body></html>",
    );
    t.route_bin("/a.css", 200, b"#d{background:#00ff00}".to_vec());
    t.route_bin("/b.css", 200, b"".to_vec());
    let id = s.open_tab();
    s.navigate(id, "http://h.test/").await.unwrap();
    let paths = t.paths();
    let a = paths.iter().filter(|p| *p == "/a.css").count();
    assert_eq!(a, 1, "même URL résolue → un seul fetch : {paths:?}");
    assert!(paths.iter().any(|p| p == "/b.css"), "{paths:?}");
}

/// Étape 96 : `@import` fetché et expansé — le contenu de la
/// feuille importée participe à la cascade ; l'import à media
/// hors env est sauté (non fetché).
#[tokio::test]
async fn import_expands_and_filters_media() {
    let (mut s, t) = test_session();
    t.route(
        "/",
        200,
        "<!doctype html><html><head><link rel=stylesheet href=/a.css></head><body>\
         <div id=x style=\"width:50px;height:50px\"></div>\
         <div id=y style=\"width:50px;height:50px\"></div>\
         </body></html>",
    );
    t.route_bin(
        "/a.css",
        200,
        b"@import \"/b.css\"; @import \"/dead.css\" print; #x{background:#00ff00}".to_vec(),
    );
    t.route_bin("/b.css", 200, b"#y{background:#0000ff}".to_vec());
    t.route_bin("/dead.css", 200, b"#x,#y{background:#ff0000}".to_vec());
    let id = s.open_tab();
    s.navigate(id, "http://h.test/").await.unwrap();
    let (fw, fh, px) = s.frame_pixels(id).unwrap();
    let (fw, fh) = (fw as usize, fh as usize);
    assert!(
        count_color(fw, fh, px, [0, 255, 0]) > 0,
        "règle propre de a.css"
    );
    assert!(
        count_color(fw, fh, px, [0, 0, 255]) > 0,
        "règle de b.css importée"
    );
    let paths = t.paths();
    assert!(
        paths.iter().any(|p| p == "/b.css"),
        "b importée : {paths:?}"
    );
    assert!(
        !paths.iter().any(|p| p == "/dead.css"),
        "import print sauté : {paths:?}"
    );
}

/// Étape 96 : chaîne `@import` bornée — `max_imports` feuilles
/// total, le surplus est sauté ; les boucles ne bouclent pas
/// (dédup par URL).
#[tokio::test]
async fn import_bounded_and_acyclic() {
    let t = Arc::new(FakeTransport::new());
    let mut s = Session::with_config(
        t.clone(),
        Arc::new(FixtureFont),
        SessionConfig {
            max_imports: 1,
            ..SessionConfig::default()
        },
    );
    t.route(
        "/",
        200,
        "<!doctype html><html><head><link rel=stylesheet href=/a.css></head><body>\
         <div id=x style=\"width:50px;height:50px\"></div></body></html>",
    );
    // b s'importe elle-même (boucle) puis c — borne + dédup.
    t.route_bin(
        "/a.css",
        200,
        b"@import \"/b.css\"; @import \"/c.css\"; #x{background:#00ff00}".to_vec(),
    );
    t.route_bin(
        "/b.css",
        200,
        b"@import \"/b.css\"; @import \"/a.css\";".to_vec(),
    );
    t.route_bin("/c.css", 200, b"#x{background:#ff0000}".to_vec());
    let id = s.open_tab();
    s.navigate(id, "http://h.test/").await.unwrap();
    let paths = t.paths();
    assert!(paths.iter().any(|p| p == "/b.css"), "{paths:?}");
    assert!(
        !paths.iter().any(|p| p == "/c.css"),
        "borne 1 → c jamais fetchée : {paths:?}"
    );
    let b_fetches = paths.iter().filter(|p| *p == "/b.css").count();
    assert_eq!(
        b_fetches, 1,
        "boucle b→b neutralisée par la dédup : {paths:?}"
    );
}

#[tokio::test]
async fn title_is_extracted_and_title_changed_emitted() {
    let (mut s, t) = test_session();
    t.route("/", 200, PAGE_T);
    let mut rx = s.subscribe();
    let id = s.open_tab();
    s.navigate(id, "http://h.test/").await.unwrap();

    let info = s.tab_info(id).unwrap();
    assert_eq!(info.title.as_deref(), Some("Page A"));

    let mut saw_title = false;
    while let Ok(e) = rx.try_recv() {
        if matches!(e, CoreEvent::TitleChanged { title, .. } if title == "Page A") {
            saw_title = true;
        }
    }
    assert!(saw_title);
}

#[tokio::test]
async fn history_back_forward_reload() {
    let (mut s, t) = test_session();
    t.route("/a", 200, PAGE_T);
    t.route(
        "/b",
        200,
        "<!doctype html><html><head>\
         <title>Page B</title></head><body><div>b</div></body></html>",
    );
    let id = s.open_tab();
    s.navigate(id, "http://h.test/a").await.unwrap();
    s.navigate(id, "http://h.test/b").await.unwrap();

    let info = s.tab_info(id).unwrap();
    assert!(info.can_back && !info.can_forward);

    // back → /a
    s.go_back(id).await.unwrap();
    assert_eq!(s.tab_url(id).as_deref(), Some("http://h.test/a"));
    let info = s.tab_info(id).unwrap();
    assert!(!info.can_back && info.can_forward);

    // forward → /b
    s.go_forward(id).await.unwrap();
    assert_eq!(s.tab_url(id).as_deref(), Some("http://h.test/b"));

    // Nouvelle navigation : tronque le futur.
    s.go_back(id).await.unwrap();
    s.navigate(id, "http://h.test/b").await.unwrap();
    let info = s.tab_info(id).unwrap();
    assert!(!info.can_forward, "futur tronqué");

    // reload : re-fetch, historique inchangé.
    let calls = *t.calls.lock().unwrap();
    s.reload(id).await.unwrap();
    assert_eq!(*t.calls.lock().unwrap(), calls + 1);
    assert_eq!(s.tab_url(id).as_deref(), Some("http://h.test/b"));
    assert!(s.tab_info(id).unwrap().can_back);

    // back sans page précédente → erreur typée, pas de panic.
    s.go_back(id).await.unwrap();
    assert!(matches!(s.go_back(id).await, Err(CoreError::EmptyDocument)));
}

#[tokio::test]
async fn close_tab_emits_and_removes() {
    let (mut s, _t) = test_session();
    let mut rx = s.subscribe();
    let id = s.open_tab();
    s.close_tab(id).unwrap();
    assert_eq!(s.tabs(), Vec::<u64>::new());
    assert!(s.tab_info(id).is_none());
    let mut saw = false;
    while let Ok(e) = rx.try_recv() {
        if matches!(e, CoreEvent::TabClosed { tab } if tab == id) {
            saw = true;
        }
    }
    assert!(saw);
    assert!(matches!(s.close_tab(id), Err(CoreError::UnknownTab(_))));
}

#[tokio::test]
async fn click_on_link_navigates() {
    let (mut s, t) = test_session();
    t.route("/", 200, PAGE_LINK);
    t.route("/b", 200, PAGE_T);
    let id = s.open_tab();
    s.navigate(id, "http://h.test/").await.unwrap();

    let out = s
        .handle_input(id, InputEvent::Click { x: 5.0, y: 5.0 })
        .await
        .unwrap();
    assert_eq!(out, InputOutcome::Navigated);
    assert_eq!(s.tab_url(id).as_deref(), Some("http://h.test/b"));
}

/// Terrain 23/10 — « reload d'un onglet wikipedia rechargeait
/// google.com » : une intention de navigation enfilée sur le
/// document A (clic lien via `handle_input_now`, reste au-delà
/// de `MAX_NAV_HOPS`, drain hôte interrompu) survivait au
/// commit du document B — `pending_nav` n'était pas purgé —
/// et tirait au drain suivant (nav, reload, input). Le commit
/// doit tuer les intentions du document mort, comme
/// `restore()` le fait déjà (tab.rs).
#[tokio::test]
async fn commit_kills_pending_nav_of_previous_document() {
    let (mut s, t) = test_session();
    t.route("/a", 200, PAGE_LINK);
    t.route("/b", 200, PAGE);
    t.route("/c", 200, PAGE);
    let id = s.open_tab();
    s.navigate(id, "http://h.test/a").await.unwrap();

    // Clic lien enfilé mais jamais drainé (l'utilisateur a
    // tapé une URL omnibox avant que l'hôte ne draine).
    let out = s
        .handle_input_now(id, InputEvent::Click { x: 5.0, y: 5.0 })
        .unwrap();
    assert_eq!(out, InputOutcome::Navigated);

    // La navigation omnibox gagne : l'intention /b du
    // document mort ne doit pas tirer après le commit.
    s.navigate(id, "http://h.test/c").await.unwrap();
    assert_eq!(s.tab_url(id).as_deref(), Some("http://h.test/c"));
    assert_eq!(t.last().unwrap().1, "/c");

    // Idem au reload : aucun reliquat ne doit naviguer ailleurs.
    s.reload(id).await.unwrap();
    assert_eq!(s.tab_url(id).as_deref(), Some("http://h.test/c"));
    assert_eq!(t.last().unwrap().1, "/c");
}

/// Étape 118 — `Accept-Language` piloté à chaud : absent par
/// défaut, la valeur posée par `set_accept_language` part sur
/// les requêtes suivantes, chaîne vide lève l'en-tête (le
/// `LangTransport` qui coiffe la pile la relit à chaque appel).
#[tokio::test]
async fn accept_language_reaches_requests_and_updates_live() {
    let (mut s, t) = test_session();
    t.route("/a", 200, PAGE);
    t.route("/b", 200, PAGE);
    let id = s.open_tab();

    s.navigate(id, "http://h.test/a").await.unwrap();
    assert_eq!(t.last_header("accept-language"), None);

    s.set_accept_language(Some("fr-FR,fr;q=0.9,en;q=0.8".into()));
    s.navigate(id, "http://h.test/b").await.unwrap();
    assert_eq!(
        t.last_header("accept-language").as_deref(),
        Some("fr-FR,fr;q=0.9,en;q=0.8")
    );

    s.set_accept_language(Some("   ".into()));
    s.navigate(id, "http://h.test/a").await.unwrap();
    assert_eq!(t.last_header("accept-language"), None);
}

#[tokio::test]
async fn click_off_link_is_ignored() {
    let (mut s, t) = test_session();
    t.route("/", 200, PAGE_LINK);
    let id = s.open_tab();
    s.navigate(id, "http://h.test/").await.unwrap();

    // Le lien fait ~20px de haut ; un clic à y=400 tombe dans
    // le body, hors lien.
    let out = s
        .handle_input(id, InputEvent::Click { x: 5.0, y: 400.0 })
        .await
        .unwrap();
    assert_eq!(out, InputOutcome::Ignored);
    assert_eq!(s.tab_url(id).as_deref(), Some("http://h.test/"));
}

#[tokio::test]
async fn mousemove_reports_link_href() {
    let (mut s, t) = test_session();
    t.route("/", 200, PAGE_LINK);
    let id = s.open_tab();
    s.navigate(id, "http://h.test/").await.unwrap();

    let out = s
        .handle_input(id, InputEvent::MouseMove { x: 5.0, y: 5.0 })
        .await
        .unwrap();
    assert_eq!(
        out,
        InputOutcome::Hover {
            url: Some("http://h.test/b".to_string()),
            editable: false
        }
    );
}

#[tokio::test]
async fn scroll_is_clamped_and_invalidates() {
    let (mut s, t) = test_session();
    t.route("/", 200, tall_page());
    let mut rx = s.subscribe();
    let id = s.open_tab();
    s.navigate(id, "http://h.test/").await.unwrap();

    // Scroll dans la borne : doc 2000+ → max = doc - 600.
    let out = s
        .handle_input(id, InputEvent::Scroll { dy: 300.0 })
        .await
        .unwrap();
    match out {
        InputOutcome::Scrolled { scroll_y } => {
            assert!((scroll_y - 300.0).abs() < 0.01);
        }
        o => panic!("inattendu : {o:?}"),
    }

    // Dépassement → clampé à doc_h - viewport.
    let out = s
        .handle_input(id, InputEvent::Scroll { dy: 99999.0 })
        .await
        .unwrap();
    let max = match out {
        InputOutcome::Scrolled { scroll_y } => scroll_y,
        o => panic!("inattendu : {o:?}"),
    };
    assert!(max > 300.0 && max <= 2000.0);

    // Scroll négatif au-delà → clampé à 0.
    let out = s
        .handle_input(id, InputEvent::Scroll { dy: -99999.0 })
        .await
        .unwrap();
    match out {
        InputOutcome::Scrolled { scroll_y } => assert!(scroll_y.abs() < 0.01),
        o => panic!("inattendu : {o:?}"),
    }

    // Les scrolls ont émis des frame_invalidated.
    let mut n = 0;
    let mut last_scroll = -1.0f32;
    while let Ok(e) = rx.try_recv() {
        // Étape 71d : l'event porte le scroll de la frame —
        // l'hôte positionne ses overlays à `rect − scroll_y`.
        if let CoreEvent::FrameInvalidated { tab, scroll_y, .. } = e {
            if tab == id {
                n += 1;
                last_scroll = scroll_y;
            }
        }
    }
    assert!(n >= 2);
    assert!((last_scroll - 0.0).abs() < f32::EPSILON);
}

#[tokio::test]
async fn key_input_is_unfocused() {
    let (mut s, _t) = test_session();
    let id = s.open_tab();
    let out = s
        .handle_input(
            id,
            InputEvent::Key {
                key: "a".to_string(),
                mods: KeyMods::NONE,
            },
        )
        .await
        .unwrap();
    // Étape 86 : aucun champ focalisé → `Unfocused` (le
    // client resynchronise son drapeau dessus).
    assert_eq!(out, InputOutcome::Unfocused);
}

/// Store en mémoire pour tester le câblage du port
/// `ProfileStore` (l'impl fichier est au daemon).
struct MemStore {
    visits: Mutex<Vec<(String, Option<String>)>>,
    bookmarks: Mutex<Vec<Bookmark>>,
    /// Jar persisté (étape 56) — `cookies.json` équivalent.
    cookies: Mutex<Vec<explorer_core::StoredCookie>>,
}

impl explorer_core::ProfileStore for MemStore {
    fn record_visit(&self, url: &str, title: Option<&str>) {
        if let Ok(mut v) = self.visits.lock() {
            v.push((url.to_string(), title.map(str::to_string)));
        }
    }
    fn load_bookmarks(&self) -> Vec<Bookmark> {
        self.bookmarks
            .lock()
            .map_or_else(|_| Vec::new(), |b| b.clone())
    }
    fn save_bookmarks(&self, b: &[explorer_core::Bookmark]) {
        if let Ok(mut m) = self.bookmarks.lock() {
            *m = b.to_vec();
        }
    }
    fn history(&self) -> Vec<explorer_core::Visit> {
        self.visits.lock().map_or_else(
            |_| Vec::new(),
            |v| {
                v.iter()
                    .map(|(url, title)| explorer_core::Visit {
                        url: url.clone(),
                        title: title.clone(),
                    })
                    .collect()
            },
        )
    }
    fn load_cookies(&self) -> Vec<explorer_core::StoredCookie> {
        self.cookies
            .lock()
            .map_or_else(|_| Vec::new(), |c| c.clone())
    }
    fn save_cookies(&self, cookies: &[explorer_core::StoredCookie]) {
        if let Ok(mut c) = self.cookies.lock() {
            *c = cookies.to_vec();
        }
    }
}

#[tokio::test]
async fn visits_are_recorded_and_bookmarks_persisted() {
    let (mut s, t) = test_session();
    t.route("/", 200, PAGE_T);
    let store = Arc::new(MemStore {
        visits: Mutex::new(Vec::new()),
        bookmarks: Mutex::new(Vec::new()),
        cookies: Mutex::new(Vec::new()),
    });
    s.set_store(store.clone());
    let id = s.open_tab();
    s.navigate(id, "http://h.test/").await.unwrap();

    // La visite est enregistrée (url + titre).
    let h = s.history();
    assert_eq!(h.len(), 1);
    assert_eq!(h[0].url, "http://h.test/");
    assert_eq!(h[0].title.as_deref(), Some("Page A"));

    // Signet : ajout, dédup, persistance, retrait.
    assert!(s.add_bookmark("http://h.test/".to_string(), None));
    assert!(!s.add_bookmark("http://h.test/".to_string(), None));
    assert_eq!(store.bookmarks.lock().unwrap().len(), 1);
    assert!(s.remove_bookmark("http://h.test/"));
    assert!(!s.remove_bookmark("http://h.test/"));
    assert_eq!(s.bookmarks().len(), 0);
}

/// Page avec un formulaire : `input` en haut à gauche,
/// `textarea` en dessous.
const PAGE_FORM: &str = "<!doctype html><html><body><form>\
     <input name=\"q\"><br><textarea>ab</textarea></form></body></html>";

#[tokio::test]
async fn click_input_focuses_then_keys_edit() {
    let (mut s, t) = test_session();
    t.route("/", 200, PAGE_FORM);
    let id = s.open_tab();
    s.navigate(id, "http://h.test/").await.unwrap();

    // Touche sans focus → « unfocused » (étape 86 — le
    // client resynchronise son drapeau de focus dessus).
    let out = s
        .handle_input(
            id,
            InputEvent::Key {
                key: "a".into(),
                mods: KeyMods::NONE,
            },
        )
        .await
        .unwrap();
    assert_eq!(out, InputOutcome::Unfocused);

    // Clic dans l'input (en haut à gauche, ~10em × 1.2em).
    let out = s
        .handle_input(id, InputEvent::Click { x: 5.0, y: 5.0 })
        .await
        .unwrap();
    assert_eq!(out, InputOutcome::Focused);

    // Caractères → édités ; Backspace idem.
    for key in ["a", "b"] {
        let out = s
            .handle_input(
                id,
                InputEvent::Key {
                    key: key.into(),
                    mods: KeyMods::NONE,
                },
            )
            .await
            .unwrap();
        assert_eq!(out, InputOutcome::Edited);
    }
    let out = s
        .handle_input(
            id,
            InputEvent::Key {
                key: "Backspace".into(),
                mods: KeyMods::NONE,
            },
        )
        .await
        .unwrap();
    assert_eq!(out, InputOutcome::Edited);

    // Enter dans un `<form>` = soumission implicite (étape
    // 38) : GET sans action → re-fetch de la page avec la
    // query des champs (`q=a` après le Backspace).
    let out = s
        .handle_input(
            id,
            InputEvent::Key {
                key: "Enter".into(),
                mods: KeyMods::NONE,
            },
        )
        .await
        .unwrap();
    assert_eq!(out, InputOutcome::Navigated);
    let (_, pq, _) = t.last().unwrap();
    assert_eq!(pq, "/?q=a");
    // La navigation a emporté le focus — la frappe suivante
    // rapporte « unfocused », pas `Ignored` muet (étape 86).
    let out = s
        .handle_input(
            id,
            InputEvent::Key {
                key: "z".into(),
                mods: KeyMods::NONE,
            },
        )
        .await
        .unwrap();
    assert_eq!(out, InputOutcome::Unfocused);
}

/// Étape 103 — `keydown` porte `key`/`code`/`keyCode`/`which`
/// (table UI Events côté session) : le listener lit les props
/// et mute le miroir → le rendu joint la page statique
/// équivalente.
const PAGE_KEYLOG: &str = "<!doctype html><html><body>\
     <div id=\"k\"></div>\
     <script>document.body.addEventListener('keydown',function(e){\
     document.getElementById('k').textContent=e.key+'/'+e.code+'/'+e.keyCode+'/'+e.which})</script>\
     </body></html>";
const PAGE_KEYLOG_AFTER: &str = "<!doctype html><html><body>\
     <div id=\"k\">a/KeyA/65/65</div></body></html>";

#[tokio::test]
async fn keydown_event_carries_key_code_props() {
    let (mut s, t) = test_session();
    t.route("/ev", 200, PAGE_KEYLOG);
    t.route("/stat", 200, PAGE_KEYLOG_AFTER);
    s.set_js_factory(explorer_js::JscoreEngine::factory());

    let a = s.open_tab();
    s.navigate(a, "http://h.test/ev").await.unwrap();
    let b = s.open_tab();
    s.navigate(b, "http://h.test/stat").await.unwrap();

    // Pas de champ focalisé : `keydown` document-level vers
    // `<body>` — les props sont posées par la session.
    let out = s
        .handle_input(
            a,
            InputEvent::Key {
                key: "a".into(),
                mods: KeyMods::NONE,
            },
        )
        .await
        .unwrap();
    assert_eq!(out, InputOutcome::Unfocused);
    assert_eq!(s.frame_png(a), s.frame_png(b));
}

#[tokio::test]
async fn keydown_prevent_default_suppresses_edit() {
    let (mut s, t) = test_session();
    t.route(
        "/",
        200,
        "<!doctype html><html><body><input name=q>\
         <script>document.addEventListener('keydown',\
         function(e){e.preventDefault()})</script>\
         </body></html>",
    );
    s.set_js_factory(explorer_js::JscoreEngine::factory());
    let id = s.open_tab();
    s.navigate(id, "http://h.test/").await.unwrap();
    let out = s
        .handle_input(id, InputEvent::Click { x: 5.0, y: 5.0 })
        .await
        .unwrap();
    assert_eq!(out, InputOutcome::Focused);
    // `preventDefault` remonte par le même canal que les props
    // (étape 103) — l'édition est supprimée, `Ignored` rapporté.
    let out = s
        .handle_input(
            id,
            InputEvent::Key {
                key: "a".into(),
                mods: KeyMods::NONE,
            },
        )
        .await
        .unwrap();
    assert_eq!(out, InputOutcome::Ignored);
}

/// Étape 105 — `keydown` porte aussi `ctrlKey`/`shiftKey`/
/// `altKey`/`metaKey` (booleans DOM) depuis
/// `InputEvent::Key.mods`. L'accord est livré à la page mais
/// n'édite pas : `Ignored` après dispatch, frame = page
/// statique équivalente.
const PAGE_MODLOG: &str = "<!doctype html><html><body>\
     <div id=\"k\"></div>\
     <script>document.body.addEventListener('keydown',function(e){\
     document.getElementById('k').textContent=e.ctrlKey+'/'+e.shiftKey+'/'+e.altKey+'/'+e.metaKey})</script>\
     </body></html>";
const PAGE_MODLOG_AFTER: &str = "<!doctype html><html><body>\
     <div id=\"k\">true/true/false/false</div></body></html>";

#[tokio::test]
async fn keydown_event_carries_modifier_props() {
    let (mut s, t) = test_session();
    t.route("/ev", 200, PAGE_MODLOG);
    t.route("/stat", 200, PAGE_MODLOG_AFTER);
    s.set_js_factory(explorer_js::JscoreEngine::factory());

    let a = s.open_tab();
    s.navigate(a, "http://h.test/ev").await.unwrap();
    let b = s.open_tab();
    s.navigate(b, "http://h.test/stat").await.unwrap();

    // Ctrl+Shift+a : l'event est dispatché avec les mods,
    // puis l'accord saute l'action par défaut (pas de champ
    // focalisé de toute façon → `Ignored`).
    let out = s
        .handle_input(
            a,
            InputEvent::Key {
                key: "a".into(),
                mods: KeyMods {
                    ctrl: true,
                    shift: true,
                    ..KeyMods::NONE
                },
            },
        )
        .await
        .unwrap();
    assert_eq!(out, InputOutcome::Ignored);
    assert_eq!(s.frame_png(a), s.frame_png(b));
}

/// Étape 105 — un accord Ctrl/Alt/Meta n'édite pas le champ
/// (l'event est dispatché, l'action par défaut est sautée) ;
/// `Shift` seul reste de la frappe — la soumission `Enter`
/// prouve le contenu final (« q=Ab », jamais « q=aAb »).
#[tokio::test]
async fn keydown_chord_suppresses_default_edit() {
    let (mut s, t) = test_session();
    t.route("/", 200, PAGE_FORM);
    let id = s.open_tab();
    s.navigate(id, "http://h.test/").await.unwrap();
    let out = s
        .handle_input(id, InputEvent::Click { x: 6.0, y: 6.0 })
        .await
        .unwrap();
    assert_eq!(out, InputOutcome::Focused);

    // Ctrl+a : raccourci — livré à la page, pas inséré.
    let out = s
        .handle_input(
            id,
            InputEvent::Key {
                key: "a".into(),
                mods: KeyMods {
                    ctrl: true,
                    ..KeyMods::NONE
                },
            },
        )
        .await
        .unwrap();
    assert_eq!(out, InputOutcome::Ignored);

    // Shift+a : frappe normale — le `key` livré porte « A »,
    // `shift` n'est pas un accord → édition.
    let out = s
        .handle_input(
            id,
            InputEvent::Key {
                key: "A".into(),
                mods: KeyMods {
                    shift: true,
                    ..KeyMods::NONE
                },
            },
        )
        .await
        .unwrap();
    assert_eq!(out, InputOutcome::Edited);
    let out = s
        .handle_input(
            id,
            InputEvent::Key {
                key: "b".into(),
                mods: KeyMods::NONE,
            },
        )
        .await
        .unwrap();
    assert_eq!(out, InputOutcome::Edited);

    let out = s
        .handle_input(
            id,
            InputEvent::Key {
                key: "Enter".into(),
                mods: KeyMods::NONE,
            },
        )
        .await
        .unwrap();
    assert_eq!(out, InputOutcome::Navigated);
    let (_, pq, _) = t.last().unwrap();
    assert_eq!(pq, "/?q=Ab");
}

/// M-H1 #1 — caret du champ focalisé : le clic dans un
/// `input` peint une barre verticale pleine (≈1 px ×
/// interligne) à l'origine du contenu ; une frappe la décale
/// à la fin du texte tapé ; `Escape` la retire avec le
/// focus. La barre se distingue des glyphes : ≥10 px sombres
/// **contigus** sur une même colonne (la bordure UA
/// `#767676`=118 reste au-dessus du seuil).
#[tokio::test]
async fn caret_paints_moves_and_clears_with_focus() {
    let (mut s, t) = test_session();
    t.route("/", 200, PAGE_FORM);
    let id = s.open_tab();
    s.navigate(id, "http://h.test/").await.unwrap();

    // Première colonne portant ≥16 px sombres contigus dans
    // la bande haute de la page (x <200) : discriminant du
    // caret = hauteur d'interligne (~18 px) — un fût de
    // glyphe à fs16 plafonne ~13 px, la bordure UA
    // `#767676`=118 reste au-dessus du seuil d'encre.
    let bar_col = |s: &Session, id: u64| -> Option<usize> {
        let (fw, fh, px) = s.frame_pixels(id).unwrap();
        let (fw, fh) = (fw as usize, fh as usize);
        for col in 0..fw.min(200) {
            let mut run = 0usize;
            for row in 0..fh.min(80) {
                let p = &px[(row * fw + col) * 4..][..4];
                if p[0] < 100 && p[1] < 100 && p[2] < 100 && p[3] > 200 {
                    run += 1;
                    if run >= 16 {
                        return Some(col);
                    }
                } else {
                    run = 0;
                }
            }
        }
        None
    };

    // Avant le focus : aucune barre pleine.
    assert!(
        bar_col(&s, id).is_none(),
        "aucun caret avant que le champ soit focalisé"
    );

    // Clic dans l'input → focus → caret à l'origine du
    // contenu (champ vide).
    let out = s
        .handle_input(id, InputEvent::Click { x: 5.0, y: 5.0 })
        .await
        .unwrap();
    assert_eq!(out, InputOutcome::Focused);
    let col0 = bar_col(&s, id).expect("caret peint au focus");
    assert!(col0 < 30, "caret au début du contenu, col={col0}");

    // Frappe → le caret suit la fin du texte (colonne > au
    // départ — les glyphes tapés ne sont pas des barres
    // pleines, `bar_col` retrouve bien le caret).
    for k in ["a", "b", "c", "d"] {
        s.handle_input(
            id,
            InputEvent::Key {
                key: k.into(),
                mods: KeyMods::NONE,
            },
        )
        .await
        .unwrap();
    }
    let col1 = bar_col(&s, id).expect("caret après frappe");
    assert!(col1 > col0 + 4, "caret avancé : {col0} → {col1}");

    // `Escape` → focus perdu → plus de barre.
    s.handle_input(
        id,
        InputEvent::Key {
            key: "Escape".into(),
            mods: KeyMods::NONE,
        },
    )
    .await
    .unwrap();
    assert!(
        bar_col(&s, id).is_none(),
        "le caret disparaît avec le focus"
    );
}

/// Étape 86 — `Enter`/`Escape` hors `<form>` perdent le
/// focus : l'outcome `Unfocused` signale la perte au client
/// (avant : `Ignored` indistinct du `keydown` consommé par
/// JS — l'app gardait son flag `fieldFocused` et avalait
/// chaque frappe suivante dans le vide).
#[tokio::test]
async fn enter_escape_unfocus_reports_unfocused() {
    const PAGE_INPUT_SEUL: &str = "<!doctype html><html><body><input></body></html>";
    let (mut s, t) = test_session();
    t.route("/", 200, PAGE_INPUT_SEUL);
    let id = s.open_tab();
    s.navigate(id, "http://h.test/").await.unwrap();

    let out = s
        .handle_input(id, InputEvent::Click { x: 5.0, y: 5.0 })
        .await
        .unwrap();
    assert_eq!(out, InputOutcome::Focused);
    let out = s
        .handle_input(
            id,
            InputEvent::Key {
                key: "a".into(),
                mods: KeyMods::NONE,
            },
        )
        .await
        .unwrap();
    assert_eq!(out, InputOutcome::Edited);

    for key in ["Enter", "Escape"] {
        // Ré-focus pour chaque tour (Enter a tout lâché).
        let out = s
            .handle_input(id, InputEvent::Click { x: 5.0, y: 5.0 })
            .await
            .unwrap();
        assert_eq!(out, InputOutcome::Focused);
        let out = s
            .handle_input(
                id,
                InputEvent::Key {
                    key: key.into(),
                    mods: KeyMods::NONE,
                },
            )
            .await
            .unwrap();
        assert_eq!(out, InputOutcome::Unfocused, "{key}");
        // La frappe suivante confirme : plus de champ.
        let out = s
            .handle_input(
                id,
                InputEvent::Key {
                    key: "b".into(),
                    mods: KeyMods::NONE,
                },
            )
            .await
            .unwrap();
        assert_eq!(out, InputOutcome::Unfocused, "après {key}");
    }
}

#[tokio::test]
async fn click_outside_blurs_and_mousemove_reports_editable() {
    let (mut s, t) = test_session();
    t.route("/", 200, PAGE_FORM);
    let id = s.open_tab();
    s.navigate(id, "http://h.test/").await.unwrap();

    // Survol de l'input → éditable, pas de lien.
    let out = s
        .handle_input(id, InputEvent::MouseMove { x: 5.0, y: 5.0 })
        .await
        .unwrap();
    assert_eq!(
        out,
        InputOutcome::Hover {
            url: None,
            editable: true
        }
    );

    // Focus puis clic loin en bas → blur (la touche rapporte
    // « unfocused » ensuite — étape 86).
    let _ = s
        .handle_input(id, InputEvent::Click { x: 5.0, y: 5.0 })
        .await;
    let _ = s
        .handle_input(id, InputEvent::Click { x: 5.0, y: 500.0 })
        .await;
    let out = s
        .handle_input(
            id,
            InputEvent::Key {
                key: "x".into(),
                mods: KeyMods::NONE,
            },
        )
        .await
        .unwrap();
    assert_eq!(out, InputOutcome::Unfocused);
}

#[tokio::test]
async fn textarea_is_editable() {
    let (mut s, t) = test_session();
    t.route("/", 200, PAGE_FORM);
    let id = s.open_tab();
    s.navigate(id, "http://h.test/").await.unwrap();

    // La textarea suit l'input + <br> : ~y 30..90.
    let out = s
        .handle_input(id, InputEvent::Click { x: 5.0, y: 50.0 })
        .await
        .unwrap();
    assert_eq!(out, InputOutcome::Focused);
    let out = s
        .handle_input(
            id,
            InputEvent::Key {
                key: "c".into(),
                mods: KeyMods::NONE,
            },
        )
        .await
        .unwrap();
    assert_eq!(out, InputOutcome::Edited);
}

/// Page avec deux `<script>` : le premier jette, le second est
/// syntaxiquement sain — la navigation réussit quand même.
const PAGE_SCRIPT: &str = "<!doctype html><html><body><div>x</div>\
     <script>throw 'boom'</script><script>var ok = 1</script>\
     </body></html>";

#[tokio::test]
async fn script_error_emits_pipeline_error_but_page_loads() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    let (mut s, t) = test_session();
    t.route("/", 200, PAGE_SCRIPT);
    // Fabrique espionne : compte les contextes créés.
    let calls = Arc::new(AtomicUsize::new(0));
    let calls2 = calls.clone();
    s.set_js_factory(Arc::new(move || {
        calls2.fetch_add(1, Ordering::SeqCst);
        Box::new(explorer_js::NullEngine)
    }));
    let mut rx = s.subscribe();
    let id = s.open_tab();
    s.navigate(id, "http://h.test/").await.unwrap();

    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(s.tab_url(id).as_deref(), Some("http://h.test/"));
    // Le PNG est produit (pipeline complet malgré le script).
    assert!(s.frame_png(id).is_some());

    // `NullEngine` ne jette pas — pas de PipelineError attendu
    // ici ; l'événement d'échec est testé côté adaptateur.
    let _ = rx.try_recv();
}

#[tokio::test]
async fn no_factory_means_scripts_ignored() {
    let (mut s, t) = test_session();
    t.route("/", 200, PAGE_SCRIPT);
    let id = s.open_tab();
    // Pas de fabrique → scripts ignorés, navigation propre.
    s.navigate(id, "http://h.test/").await.unwrap();
    assert_eq!(s.tab_url(id).as_deref(), Some("http://h.test/"));
}

/// Étape 123 — allocation ~4 Mio tentée puis titre posé :
/// « ok » si la borne laisse passer, « err:… » sinon.
const PAGE_HEAP_PROBE: &str = "<!doctype html><html><body>\
     <script>try{var s='x'.repeat(4000000);document.title='ok'}\
     catch(e){document.title='err:'+e.message}</script>\
     </body></html>";

/// La borne `js_mem_limit_mib` arrive jusqu'au contexte JS :
/// à 1 Mio, l'allocation de ~4 Mio est refusée (`RangeError`)
/// et le titre rendu « err » — la couture
/// `JsEngine::set_mem_limit` est bien appliquée à la
/// création du contexte (réglage diagnostic, pas produit).
#[tokio::test]
async fn js_heap_limit_reaches_context() {
    let (mut s, t) = test_session_with(SessionConfig {
        js_mem_limit_mib: 1,
        ..SessionConfig::default()
    });
    t.route("/", 200, PAGE_HEAP_PROBE);
    s.set_js_factory(explorer_js::JscoreEngine::factory());
    let mut rx = s.subscribe();
    let id = s.open_tab();
    s.navigate(id, "http://h.test/").await.unwrap();
    let mut title = None;
    while let Ok(ev) = rx.try_recv() {
        if let CoreEvent::TitleChanged { tab, title: t2 } = ev {
            if tab == id {
                title = Some(t2);
            }
        }
    }
    assert!(
        title.as_deref().is_some_and(|t| t.starts_with("err:")),
        "borne attendue : titre {title:?}"
    );
}

/// `js_mem_limit_mib: 0` = illimité explicite : la même
/// allocation passe et le titre rendu « ok » — le réglage
/// est bien bidirectionnel (pas une borne câblée en dur).
#[tokio::test]
async fn js_heap_limit_zero_disarms() {
    let (mut s, t) = test_session_with(SessionConfig {
        js_mem_limit_mib: 0,
        ..SessionConfig::default()
    });
    t.route("/", 200, PAGE_HEAP_PROBE);
    s.set_js_factory(explorer_js::JscoreEngine::factory());
    let mut rx = s.subscribe();
    let id = s.open_tab();
    s.navigate(id, "http://h.test/").await.unwrap();
    let mut title = None;
    while let Ok(ev) = rx.try_recv() {
        if let CoreEvent::TitleChanged { tab, title: t2 } = ev {
            if tab == id {
                title = Some(t2);
            }
        }
    }
    assert_eq!(title.as_deref(), Some("ok"));
}

/// Étape 130 — `mem_report` attribue la page vivante et,
/// après une seconde navigation, la page gelée bfcache
/// (heap JS gelé compté — premier réservoir suspecté du
/// constat « station ~1,4 Gio »).
#[tokio::test]
async fn mem_report_attributes_live_and_bfcache() {
    let (mut s, t) = test_session();
    // Page scriptée AVEC contenu peint : un moteur JS vivant
    // + des items de display list à attribuer (puis à geler).
    t.route(
        "/a",
        200,
        "<!doctype html><html><body><div>contenu /a</div>\
         <script>document.title='a'</script></body></html>",
    );
    t.route("/b", 200, PAGE);
    s.set_js_factory(explorer_js::JscoreEngine::factory());
    let id = s.open_tab();
    s.navigate(id, "http://h.test/a").await.unwrap();
    // Peinture paresseuse : `frame_png` déclenche
    // layout/display/raster si la frame n'existe pas.
    let _ = s.frame_png(id);
    let r1 = s.mem_report(id).expect("rapport après nav /a");
    assert!(r1.dom_nodes > 0, "dom peuplé");
    assert!(r1.layout_boxes > 0 && r1.display_items > 0);
    assert!(r1.frame_bytes > 0, "frame produite");
    assert!(r1.js_heap_bytes > 0, "contexte JS vivant mesuré");
    assert_eq!(r1.bfcache_pages, 0);
    // Seconde navigation → /a gelée : son heap JS doit
    // apparaître dans `bfcache_js_heap_bytes`.
    s.navigate(id, "http://h.test/b").await.unwrap();
    let _ = s.frame_png(id);
    let r2 = s.mem_report(id).expect("rapport après nav /b");
    assert_eq!(r2.bfcache_pages, 1, "/a gelée");
    assert!(r2.bfcache_dom_nodes > 0 && r2.bfcache_frame_bytes > 0);
    assert!(
        r2.bfcache_js_heap_bytes > 0,
        "le moteur JS gelé est un réservoir mesuré"
    );
    // Onglet inconnu → None (dégradation, pas de panique).
    assert!(s.mem_report(999).is_none());
}

/// Étape 133 — le gel bfcache libère les bitmaps
/// `DecodedImage` (multiplicateur ×`bfcache_pages` mesuré
/// par l'étape 130 : ~144 Mio de bitmaps github × 4 pages
/// gelées) mais retient les corps compressés : le retour
/// re-décode depuis `img_bodies` **sans réseau** et la
/// page restaurée peint à l'identique.
#[tokio::test]
async fn bfcache_snapshot_drops_bitmaps_keeps_bodies() {
    let (mut s, t) = test_session();
    t.route("/i", 200, "<html><body><img src=\"/i.png\"></body></html>");
    t.route_bin("/i.png", 200, red_png());
    t.route("/b", 200, "<html><body><div>b</div></body></html>");

    let id = s.open_tab();
    s.navigate(id, "http://h.test/i").await.unwrap();
    let before = s.frame_png(id).expect("frame /i");
    let live = s.mem_report(id).expect("rapport vivant /i");
    assert!(live.images_bytes > 0, "bitmap <img> vivante");
    assert!(live.img_bodies_bytes > 0, "corps compressé retenu");

    // Navigation → /i gèle : plus aucun `Arc` de bitmap ne
    // doit survivre dans le snapshot, mais le corps
    // compressé reste.
    s.navigate(id, "http://h.test/b").await.unwrap();
    let r = s.mem_report(id).expect("rapport après nav /b");
    assert_eq!(r.bfcache_pages, 1, "/i gelée");
    assert_eq!(
        r.bfcache_images_bytes, 0,
        "le gelé ne retient plus de bitmap"
    );
    assert!(
        r.bfcache_img_bodies_bytes > 0,
        "le gelé retient le corps compressé"
    );

    // Retour bfcache : re-décodage sans réseau (le
    // transport n'a pas été re-sollicité), rendu identique.
    let calls = *t.calls.lock().unwrap();
    s.go_back(id).await.unwrap();
    assert_eq!(
        *t.calls.lock().unwrap(),
        calls,
        "restauration sans re-fetch"
    );
    let r2 = s.mem_report(id).expect("rapport après back");
    assert!(r2.images_bytes > 0, "bitmap re-décodée au retour");
    assert_eq!(
        s.frame_png(id).expect("frame restaurée"),
        before,
        "la page restaurée peint à l'identique"
    );
}
const PAGE_MUT: &str = "<!doctype html><html><body>\
     <div id=\"x\">avant</div>\
     <script>document.getElementById('x').textContent='APRES'</script>\
     </body></html>";

/// Page statique équivalente au résultat de la mutation.
const PAGE_MUT_STATIC: &str = "<!doctype html><html><body>\
     <div id=\"x\">APRES</div></body></html>";

#[tokio::test]
async fn script_dom_mutation_is_rendered() {
    let (mut s, t) = test_session();
    t.route("/mut", 200, PAGE_MUT);
    t.route("/stat", 200, PAGE_MUT_STATIC);
    s.set_js_factory(explorer_js::JscoreEngine::factory());

    let a = s.open_tab();
    s.navigate(a, "http://h.test/mut").await.unwrap();
    let b = s.open_tab();
    s.navigate(b, "http://h.test/stat").await.unwrap();

    // Le PNG de la page mutée par le script doit être
    // identique à celui de la page statique équivalente.
    let (pa, pb) = (s.frame_png(a), s.frame_png(b));
    assert!(pa.is_some() && pb.is_some());
    assert_eq!(pa, pb, "la mutation JS du DOM doit être rendue à l'écran");
}

#[tokio::test]
async fn script_dom_append_is_rendered() {
    let (mut s, t) = test_session();
    t.route(
        "/",
        200,
        "<!doctype html><html><body><div id=\"h\">x</div>\
         <script>var b=document.createElement('div');\
         b.textContent='AJOUT';document.body.appendChild(b)</script>\
         </body></html>",
    );
    t.route(
        "/stat",
        200,
        "<!doctype html><html><body><div id=\"h\">x</div>\
         <div>AJOUT</div></body></html>",
    );
    s.set_js_factory(explorer_js::JscoreEngine::factory());

    let a = s.open_tab();
    s.navigate(a, "http://h.test/").await.unwrap();
    let b = s.open_tab();
    s.navigate(b, "http://h.test/stat").await.unwrap();

    // `createElement` + `appendChild` mutent le miroir → le
    // rendu doit correspondre à la page statique équivalente.
    assert_eq!(s.frame_png(a), s.frame_png(b));
}

/// Page `<canvas>` dessinée par script (JS-C28, couture R-11).
const PAGE_CANVAS: &str = "<!doctype html><html><body>\
<canvas id=\"c\" width=\"40\" height=\"30\"></canvas>\
<script>var x=document.getElementById('c').getContext('2d');\
x.fillStyle='#ff0000';x.fillRect(0,0,40,30);\
x.strokeStyle='#0000ff';x.strokeRect(2,2,10,10)</script></body></html>";

/// `<canvas>` 2D : `getContext`/`fillRect` enregistrent dans
/// le miroir → la session rejoue la op-list (R-11) et la
/// frame contient les pixels peints par le script.
#[tokio::test]
async fn canvas_2d_script_paints_bitmap() {
    let (mut sess, tr) = test_session();
    tr.route("/c", 200, PAGE_CANVAS);
    tr.route(
        "/plain",
        200,
        "<!doctype html><html><body><canvas width=\"40\" height=\"30\"></canvas></body></html>",
    );
    sess.set_js_factory(explorer_js::JscoreEngine::factory());

    let drawn = sess.open_tab();
    sess.navigate(drawn, "http://h.test/c").await.unwrap();
    let blank = sess.open_tab();
    sess.navigate(blank, "http://h.test/plain").await.unwrap();

    // Le fillRect couvre toute la surface 40×30 → la frame
    // contient des pixels rouges ; la page sans script n'a
    // rien dessiné.
    let red = {
        let (fw, fh, px) = sess.frame_pixels(drawn).unwrap();
        (0..fh).any(|row| {
            (0..fw).any(|col| {
                let i = ((row * fw + col) * 4) as usize;
                px[i] > 200 && px[i + 1] < 80 && px[i + 2] < 80
            })
        })
    };
    assert!(red, "le fillRect rouge du script doit être peint");
    assert_ne!(
        sess.frame_png(drawn),
        sess.frame_png(blank),
        "canvas dessiné vs canvas vierge"
    );
}

/// Page avec listener `click` qui mute le DOM miroir.
const PAGE_ONCLICK: &str = "<!doctype html><html><body>\
     <div id=\"b\">avant</div>\
     <script>document.getElementById('b').addEventListener('click',\
     function(){document.getElementById('b').textContent='CLIQUE'})</script>\
     </body></html>";

/// Statique équivalente au résultat du click.
const PAGE_ONCLICK_AFTER: &str = "<!doctype html><html><body>\
     <div id=\"b\">CLIQUE</div></body></html>";

#[tokio::test]
async fn click_dispatches_dom_event_and_rerenders() {
    let (mut s, t) = test_session();
    t.route("/ev", 200, PAGE_ONCLICK);
    t.route("/stat", 200, PAGE_ONCLICK_AFTER);
    s.set_js_factory(explorer_js::JscoreEngine::factory());

    let a = s.open_tab();
    s.navigate(a, "http://h.test/ev").await.unwrap();
    let b = s.open_tab();
    s.navigate(b, "http://h.test/stat").await.unwrap();

    // Clic sur le div : le listener mute le miroir → resynchro
    // → rendu identique à la page statique cible.
    let out = s
        .handle_input(a, InputEvent::Click { x: 5.0, y: 5.0 })
        .await
        .unwrap();
    assert!(matches!(out, InputOutcome::Ignored));
    assert_eq!(s.frame_png(a), s.frame_png(b));
}

/// JS-C45 — le handler inline `onclick=` (attribut, pas
/// `addEventListener`) est compilé par le moteur : clic →
/// mutation → même rendu que la page statique cible.
const PAGE_ONCLICK_ATTR: &str = "<!doctype html><html><body>\
     <div id=\"b\" onclick=\"this.textContent='CLIQUE'\">avant</div>\
     </body></html>";

#[tokio::test]
async fn click_runs_inline_onclick_attribute() {
    let (mut s, t) = test_session();
    t.route("/ev", 200, PAGE_ONCLICK_ATTR);
    t.route("/stat", 200, PAGE_ONCLICK_AFTER);
    s.set_js_factory(explorer_js::JscoreEngine::factory());

    let a = s.open_tab();
    s.navigate(a, "http://h.test/ev").await.unwrap();
    let b = s.open_tab();
    s.navigate(b, "http://h.test/stat").await.unwrap();

    let out = s
        .handle_input(a, InputEvent::Click { x: 5.0, y: 5.0 })
        .await
        .unwrap();
    assert!(matches!(out, InputOutcome::Ignored));
    assert_eq!(s.frame_png(a), s.frame_png(b));
}

/// Lien avec `preventDefault` — l'action par défaut est
/// supprimée : pas de navigation.
const PAGE_PREVENT: &str = "<!doctype html><html><body>\
     <a id=\"l\" href=\"/b\">lien</a>\
     <script>document.getElementById('l').addEventListener('click',\
     function(e){e.preventDefault()})</script></body></html>";

#[tokio::test]
async fn click_prevented_link_does_not_navigate() {
    let (mut s, t) = test_session();
    t.route("/", 200, PAGE_PREVENT);
    t.route("/b", 200, "<html><body><p>page b</p></body></html>");
    s.set_js_factory(explorer_js::JscoreEngine::factory());

    let id = s.open_tab();
    s.navigate(id, "http://h.test/").await.unwrap();
    let out = s
        .handle_input(id, InputEvent::Click { x: 5.0, y: 5.0 })
        .await
        .unwrap();

    // `preventDefault` remonte → le lien ne navigue pas.
    assert!(matches!(out, InputOutcome::Ignored));
    assert_eq!(s.tab_url(id).as_deref(), Some("http://h.test/"));
}

/// `setTimeout(…, 0)` pendant le chargement — le timer dû est
/// drainé par `run_pending` et sa mutation est rendue.
const PAGE_TIMER: &str = "<!doctype html><html><body>\
     <div id=\"b\">avant</div>\
     <script>setTimeout(function(){\
     document.getElementById('b').textContent='TIMER'},0)</script>\
     </body></html>";

/// Statique équivalente au résultat du timer.
const PAGE_TIMER_AFTER: &str = "<!doctype html><html><body>\
     <div id=\"b\">TIMER</div></body></html>";

#[tokio::test]
async fn timer_zero_fires_and_rerenders() {
    let (mut s, t) = test_session();
    t.route("/t", 200, PAGE_TIMER);
    t.route("/stat", 200, PAGE_TIMER_AFTER);
    s.set_js_factory(explorer_js::JscoreEngine::factory());

    let a = s.open_tab();
    s.navigate(a, "http://h.test/t").await.unwrap();
    let b = s.open_tab();
    s.navigate(b, "http://h.test/stat").await.unwrap();

    assert_eq!(s.frame_png(a), s.frame_png(b));
}

/// PNG 2×2 rouge opaque, encodé à la volée.
fn red_png() -> Vec<u8> {
    use image::{codecs::png::PngEncoder, ExtendedColorType, ImageEncoder};
    let mut buf = Vec::new();
    PngEncoder::new(&mut buf)
        .write_image(&[255, 0, 0, 255].repeat(4), 2, 2, ExtendedColorType::Rgba8)
        .expect("encode png");
    buf
}

#[tokio::test]
async fn img_renders_decoded_bitmap() {
    let (mut s, t) = test_session();
    t.route("/i", 200, "<html><body><img src=\"/i.png\"></body></html>");
    t.route_bin("/i.png", 200, red_png());
    // Page équivalente : un bloc 2×2 à fond rouge — la
    // comparaison PNG prouve le blit bout en bout.
    t.route(
        "/s",
        200,
        "<html><head><style>#b{display:inline-block;background-color:#ff0000;\
         width:2px;height:2px}</style></head>\
         <body><div id=\"b\"></div></body></html>",
    );

    let a = s.open_tab();
    s.navigate(a, "http://h.test/i").await.unwrap();
    let b = s.open_tab();
    s.navigate(b, "http://h.test/s").await.unwrap();
    assert_eq!(s.frame_png(a), s.frame_png(b));
}

/// Étape 108 — un apply de synchro JS re-parse le document :
/// les `<img>` fetchés doivent survivre (le cache persistant
/// `img_decoded` re-matche par `src` — avant, la table
/// `images.replaced` était RAZ à `PaintImages::default()` et
/// le bitmap mourait à la première mutation post-fetch —
/// trigger : logo central de www.wikipedia.org, vol 22/10).
#[tokio::test]
async fn img_survives_js_dom_resync() {
    let (mut s, t) = test_session();
    // Le flottant 0×0 force `Relayout::NeedsFullLayout` à la
    // synchro JS — le reflow complet reconstruit le boxtree
    // entier (l'incrémental retiendrait la boîte image et
    // masquerait la perte de `images.replaced`).
    // La mutation est différée par `setTimeout` à délai réel :
    // les timers à 0 ms tirent pendant `navigate` (synchro
    // `relayout:false` avant `insert_images` — ne peut pas
    // perdre les images). 80 ms + `sleep` le fait tirer
    // post-nav → apply `relayout:true` — le vrai chemin du
    // wipe (wikipedia : `jsl10n` mute après le fetch).
    t.route(
        "/m",
        200,
        "<html><body><div style=\"float:left;width:0;height:0\"></div>\
         <img src=\"/i.png\"><div id=\"x\">avant</div>\
         <script>setTimeout(function(){\
         document.getElementById('x').textContent='APRES'},80)</script>\
         </body></html>",
    );
    t.route_bin("/i.png", 200, red_png());
    // DOM final équivalent, statique.
    t.route(
        "/s",
        200,
        "<html><body><div style=\"float:left;width:0;height:0\"></div>\
         <img src=\"/i.png\"><div id=\"x\">APRES</div></body></html>",
    );
    s.set_js_factory(explorer_js::JscoreEngine::factory());

    let a = s.open_tab();
    s.navigate(a, "http://h.test/m").await.unwrap();
    std::thread::sleep(std::time::Duration::from_millis(120));
    s.pump_timers(a);
    let b = s.open_tab();
    s.navigate(b, "http://h.test/s").await.unwrap();
    let (pa, pb) = (s.frame_png(a), s.frame_png(b));
    assert!(pa.is_some() && pb.is_some());
    assert_eq!(pa, pb, "l'<img> fetché doit survivre à la synchro JS");
}

/// Étape 108 — même classe de bug côté `background-image:
/// url(http…)` : `bg_bodies` était consommé une fois puis
/// `images.backgrounds` RAZ à l'apply suivant. Le cache
/// `bg_decoded` rend l'insertion idempotente.
#[tokio::test]
async fn fetched_background_survives_js_dom_resync() {
    let (mut s, t) = test_session();
    // Flottant 0×0 : reflow complet à la synchro (voir
    // `img_survives_js_dom_resync`).
    t.route(
        "/m",
        200,
        "<html><head><style>#b{background-image:url(/i.png);width:2px;height:2px}\
         </style></head><body><div style=\"float:left;width:0;height:0\"></div>\
         <div id=\"b\"></div><div id=\"x\">avant</div>\
         <script>setTimeout(function(){\
         document.getElementById('x').textContent='APRES'},80)</script>\
         </body></html>",
    );
    t.route_bin("/i.png", 200, red_png());
    // Équivalent statique : même box, fond rouge uni.
    t.route(
        "/s",
        200,
        "<html><head><style>#b{background-color:#ff0000;width:2px;height:2px}\
         </style></head><body><div style=\"float:left;width:0;height:0\"></div>\
         <div id=\"b\"></div><div id=\"x\">APRES</div></body></html>",
    );
    s.set_js_factory(explorer_js::JscoreEngine::factory());

    let a = s.open_tab();
    s.navigate(a, "http://h.test/m").await.unwrap();
    std::thread::sleep(std::time::Duration::from_millis(120));
    s.pump_timers(a);
    let b = s.open_tab();
    s.navigate(b, "http://h.test/s").await.unwrap();
    let (pa, pb) = (s.frame_png(a), s.frame_png(b));
    assert!(pa.is_some() && pb.is_some());
    assert_eq!(pa, pb, "le fond fetché doit survivre à la synchro JS");
}

#[tokio::test]
#[allow(clippy::many_single_char_names)]
async fn broken_img_falls_back_to_alt() {
    let (mut s, t) = test_session();
    // `/i.png` n'est pas routée → 404 → le `<img>` tombe
    // sur sa boîte texte `alt`. On prouve que c'est bien
    // l'alt qui se peint : PNG différent selon le texte
    // `alt`, identique pour un autre `src` cassé.
    t.route(
        "/a",
        200,
        "<html><body><img src=\"/i.png\" alt=\"fallback\"></body></html>",
    );
    t.route(
        "/b",
        200,
        "<html><body><img src=\"/autre.png\" alt=\"fallback\"></body></html>",
    );
    t.route(
        "/c",
        200,
        "<html><body><img src=\"/i.png\" alt=\"changed\"></body></html>",
    );

    let a = s.open_tab();
    s.navigate(a, "http://h.test/a").await.unwrap();
    let b = s.open_tab();
    s.navigate(b, "http://h.test/b").await.unwrap();
    let c = s.open_tab();
    s.navigate(c, "http://h.test/c").await.unwrap();

    // Même `alt`, `src` cassé différent → rendu identique.
    assert_eq!(s.frame_png(a), s.frame_png(b));
    // `alt` différent → le texte peint change.
    assert_ne!(s.frame_png(a), s.frame_png(c));
}

#[tokio::test]
async fn img_count_is_bounded() {
    let (mut s, t) = test_session();
    // 40 images déclarées, borne `max_images` = 32 par
    // défaut — les requêtes d'image restent bornées.
    let mut page = String::from("<html><body>");
    for i in 0..40 {
        let _ = std::fmt::Write::write_fmt(&mut page, format_args!("<img src=\"/i.png?id={i}\">"));
    }
    page.push_str("</body></html>");
    // `route` prend &'static — la page fuit volontairement
    // (test borné).
    t.route("/", 200, Box::leak(page.into_boxed_str()));
    t.route_bin("/i.png", 200, red_png());
    let id = s.open_tab();
    s.navigate(id, "http://h.test/").await.unwrap();
    // 1 document + ≤ 32 images (`max_images`) — jamais 41.
    assert!(t.calls() <= 33, "fetches non bornés : {}", t.calls());
    assert!(s.frame_png(id).is_some());
}

// ------------------------------------------------------------------
// Étape 98 — background-image HTTP
// ------------------------------------------------------------------

#[tokio::test]
async fn background_image_fetched_and_rendered() {
    let (mut s, t) = test_session();
    t.route(
        "/i",
        200,
        "<html><head><link rel=\"stylesheet\" href=\"/css/s.css\"></head>\
        <body><div id=\"b\"></div></body></html>",
    );
    // `url("bg.png")` relative à la FEUILLE → `/css/bg.png` :
    // la résolution contre la base feuille est le test.
    t.route(
        "/css/s.css",
        200,
        "#b{display:inline-block;background-image:url(\"bg.png\");\
         width:2px;height:2px}",
    );
    t.route_bin("/css/bg.png", 200, red_png());

    let a = s.open_tab();
    s.navigate(a, "http://h.test/i").await.unwrap();
    // Le bitmap fetché s'est blitté — ≥ 1 pixel rouge plein
    // peint (l'image 2×2 couvre sa boîte en tile).
    let (_, _, px) = s.frame_pixels(a).unwrap();
    let red = px
        .chunks_exact(4)
        .filter(|p| p[0] > 230 && p[1] < 40 && p[2] < 40)
        .count();
    assert!(red > 0, "fond rouge attendu — paths={:?}", t.paths());
}

#[tokio::test]
async fn background_image_missed_fetch_is_silent() {
    let (mut s, t) = test_session();
    // Corps absent (404) : dégradation gracieuse — pas de
    // fond, pas d'échec, la page se peint.
    t.route(
        "/i",
        200,
        "<html><head><link rel=\"stylesheet\" href=\"/s.css\"></head>\
        <body><div id=\"b\"></div></body></html>",
    );
    t.route(
        "/s.css",
        200,
        "#b{background-image:url(/gone.png);width:2px;height:2px}",
    );
    let id = s.open_tab();
    s.navigate(id, "http://h.test/i").await.unwrap();
    assert!(s.frame_png(id).is_some());
}

#[tokio::test]
async fn background_image_count_is_bounded() {
    let (mut s, t) = test_session();
    // 40 fonds distincts — le budget `max_images` (32) est
    // partagé avec `<img>` : fetchs bg bornés à 32.
    let mut css = String::new();
    for i in 0..40 {
        let _ = std::fmt::Write::write_fmt(
            &mut css,
            format_args!("#b{i}{{background-image:url(/bg.png?id={i})}}"),
        );
    }
    let mut page =
        String::from("<html><head><link rel=\"stylesheet\" href=\"/s.css\"></head><body>");
    for i in 0..40 {
        let _ = std::fmt::Write::write_fmt(&mut page, format_args!("<div id=\"b{i}\"></div>"));
    }
    page.push_str("</body></html>");
    t.route("/", 200, Box::leak(page.into_boxed_str()));
    t.route("/s.css", 200, Box::leak(css.into_boxed_str()));
    t.route_bin("/bg.png", 200, red_png());
    let id = s.open_tab();
    s.navigate(id, "http://h.test/").await.unwrap();
    // 1 document + 1 feuille + ≤ 32 fonds — jamais 42.
    assert!(t.calls() <= 34, "fetchs bg non bornés : {}", t.calls());
    assert!(s.frame_png(id).is_some());
}

// ------------------------------------------------------------------
// Étape 121 — mask-image HTTP (demande PIPE R-86)
// ------------------------------------------------------------------

/// PNG 4×4 : moitié gauche opaque, moitié droite
/// transparente — posé sur une boîte 4×4, un `mask-image`
/// borne le `background-color` aux 8 px de gauche (16 = le
/// repli « masque absent → fond plein »).
fn half_mask_png() -> Vec<u8> {
    use image::{codecs::png::PngEncoder, ExtendedColorType, ImageEncoder};
    let mut rgba = Vec::new();
    for _y in 0..4 {
        for x in 0..4 {
            rgba.extend_from_slice(&[0, 0, 0, if x < 2 { 255 } else { 0 }]);
        }
    }
    let mut buf = Vec::new();
    PngEncoder::new(&mut buf)
        .write_image(&rgba, 4, 4, ExtendedColorType::Rgba8)
        .expect("encode png");
    buf
}

/// Compte les pixels rouges pleins de la frame — l'oracle
/// pixel qui distingue masque appliqué (8) du repli fond
/// plein (16) sur une boîte 4×4.
fn red_px_count(s: &Session, id: explorer_core::TabId) -> usize {
    let (_, _, px) = s.frame_pixels(id).unwrap();
    px.chunks_exact(4)
        .filter(|p| p[0] > 230 && p[1] < 40 && p[2] < 40)
        .count()
}

/// Bout en bout : `mask-image: url(…)` relative à la feuille
/// liée est fetchée, décodée (`image_max_px`) et enregistrée
/// dans `masks_url` sous le texte `url(...)` brut — le
/// `MaskedFill` borne le fond à la zone opaque.
#[tokio::test]
async fn mask_image_fetched_and_rendered() {
    let (mut s, t) = test_session();
    t.route(
        "/i",
        200,
        "<html><head><link rel=\"stylesheet\" href=\"/css/s.css\"></head>\
        <body><div id=\"b\"></div></body></html>",
    );
    // `url("m.png")` relative à la FEUILLE → `/css/m.png` :
    // la résolution contre la base feuille est le test.
    t.route(
        "/css/s.css",
        200,
        "#b{display:inline-block;mask-image:url(\"m.png\");\
         background-color:#ff0000;width:4px;height:4px}",
    );
    t.route_bin("/css/m.png", 200, half_mask_png());

    let a = s.open_tab();
    s.navigate(a, "http://h.test/i").await.unwrap();
    assert_eq!(
        red_px_count(&s, a),
        8,
        "masque fetché → moitié gauche seulement — paths={:?}",
        t.paths()
    );
}

/// Même rail côté `<style>` inline (base = document) et
/// propriété préfixée `-webkit-mask-image` — le motif des
/// icônes MDN.
#[tokio::test]
async fn webkit_mask_image_fetched_and_rendered() {
    let (mut s, t) = test_session();
    t.route(
        "/i",
        200,
        "<html><head><style>#b{display:inline-block;\
         -webkit-mask-image:url(/m.png);background-color:#ff0000;\
         width:4px;height:4px}</style></head>\
        <body><div id=\"b\"></div></body></html>",
    );
    t.route_bin("/m.png", 200, half_mask_png());
    let a = s.open_tab();
    s.navigate(a, "http://h.test/i").await.unwrap();
    assert_eq!(
        red_px_count(&s, a),
        8,
        "masque -webkit fetché attendu — paths={:?}",
        t.paths()
    );
}

/// Le motif MDN `.x::before` : la boîte pseudo n'a pas de
/// `NodeId` — son masque ne passe QUE par `masks_url` cléée
/// sur le texte `url(...)` brut. Le wiring fetch doit donc
/// enregistrer sous cette clé (pas de re-scan par styles).
#[tokio::test]
async fn mask_on_pseudo_fetched() {
    let (mut s, t) = test_session();
    t.route(
        "/i",
        200,
        "<html><head><style>.x::before{content:\"\";\
         display:inline-block;width:4px;height:4px;\
         mask-image:url(/m.png);background-color:#ff0000}</style></head>\
        <body><span class=\"x\"></span></body></html>",
    );
    t.route_bin("/m.png", 200, half_mask_png());
    let a = s.open_tab();
    s.navigate(a, "http://h.test/i").await.unwrap();
    assert_eq!(
        red_px_count(&s, a),
        8,
        "masque pseudo fetché attendu — paths={:?}",
        t.paths()
    );
}

/// Corps absent (404) : dégradation gracieuse — pas de
/// `MaskedFill`, le repli `background-color` plein peint
/// les 16 px, la page vit.
#[tokio::test]
async fn mask_image_missed_fetch_is_silent() {
    let (mut s, t) = test_session();
    t.route(
        "/i",
        200,
        "<html><head><style>#b{display:inline-block;\
         mask-image:url(/gone.png);background-color:#ff0000;\
         width:4px;height:4px}</style></head>\
        <body><div id=\"b\"></div></body></html>",
    );
    let id = s.open_tab();
    s.navigate(id, "http://h.test/i").await.unwrap();
    assert_eq!(
        red_px_count(&s, id),
        16,
        "repli fond plein attendu (masque 404)"
    );
}

/// Trou colateral mesuré : `data_imgs.masks_url` n'était
/// jamais étendu à l'apply — un masque `data:` était aveugle
/// en session dès la première frame (le merge n'existait
/// pas, contrairement aux tests PIPE qui injectent le
/// `PaintImages` complet). Le fetch http ne couvre que la
/// moitié du contrat.
#[tokio::test]
async fn data_mask_paints_in_session() {
    use std::fmt::Write;
    let (mut s, t) = test_session();
    let enc = half_mask_png().iter().fold(String::new(), |mut out, byte| {
        let _ = write!(out, "%{byte:02X}");
        out
    });
    t.route(
        "/i",
        200,
        Box::leak(
            format!(
                "<html><head><style>#b{{display:inline-block;\
                 mask-image:url(\"data:image/png,{enc}\");\
                 background-color:#ff0000;width:4px;height:4px}}</style></head>\
                <body><div id=\"b\"></div></body></html>"
            )
            .into_boxed_str(),
        ),
    );
    let a = s.open_tab();
    s.navigate(a, "http://h.test/i").await.unwrap();
    assert_eq!(
        red_px_count(&s, a),
        8,
        "masque data: peint — le merge masks_url est requis"
    );
}

/// Régression RAZ : `apply_payload` repose
/// `PaintImages::default()` à chaque re-parse — `masks_url`
/// meurt avec. `mask_aliases` + `bg_decoded` doivent le
/// ré-enregistrer à chaque cascade (symétrique de
/// `fetched_background_survives_js_dom_resync`).
#[tokio::test]
async fn fetched_mask_survives_js_dom_resync() {
    let (mut s, t) = test_session();
    // Flottant 0×0 : reflow complet à la synchro (voir
    // `img_survives_js_dom_resync`).
    t.route(
        "/m",
        200,
        "<html><head><style>#b{display:inline-block;mask-image:url(/i.png);\
         background-color:#ff0000;width:4px;height:4px}\
         </style></head><body><div style=\"float:left;width:0;height:0\"></div>\
         <div id=\"b\"></div><div id=\"x\">avant</div>\
         <script>setTimeout(function(){\
         document.getElementById('x').textContent='APRES'},80)</script>\
         </body></html>",
    );
    t.route_bin("/i.png", 200, half_mask_png());
    // Équivalent statique : la moitié gauche (2×4) rouge
    // pleine, le reste du motif transparent.
    t.route(
        "/s",
        200,
        "<html><head><style>#b{display:inline-block;background-color:#ff0000;\
         width:2px;height:4px}\
         </style></head><body><div style=\"float:left;width:0;height:0\"></div>\
         <div id=\"b\"></div><div id=\"x\">APRES</div></body></html>",
    );
    s.set_js_factory(explorer_js::JscoreEngine::factory());

    let a = s.open_tab();
    s.navigate(a, "http://h.test/m").await.unwrap();
    std::thread::sleep(std::time::Duration::from_millis(120));
    s.pump_timers(a);
    let b = s.open_tab();
    s.navigate(b, "http://h.test/s").await.unwrap();
    let (pa, pb) = (s.frame_png(a), s.frame_png(b));
    assert!(pa.is_some() && pb.is_some());
    assert_eq!(pa, pb, "le masque fetché doit survivre à la synchro JS");
}

// ------------------------------------------------------------------
// Étape 38 — formulaires
// ------------------------------------------------------------------

/// GET : input en div 1, submit en div 2 (y ≈ 24-47).
const PAGE_GET_FORM: &str = "<html><body><form action=\"/search\" method=\"get\">\
    <div><input name=\"q\" value=\"hi there\"></div>\
    <div><input type=\"submit\" value=\"Go\"></div></form></body></html>";

/// POST : chaque contrôle dans son `div` (empilement bloc
/// déterministe), bouton tout en bas (~y 160+).
const PAGE_POST_FORM: &str = "<html><body><form method=\"post\" action=\"/doit\">\
    <div><input name=\"a\" value=\"1\"></div>\
    <div><input type=\"checkbox\" name=\"c\" checked></div>\
    <div><input type=\"checkbox\" name=\"d\"></div>\
    <div><input type=\"radio\" name=\"r\" value=\"x\" checked>\
    <input type=\"radio\" name=\"r\" value=\"y\"></div>\
    <div><select name=\"s\"><option value=\"o1\">One</option>\
    <option value=\"o2\" selected>Two</option></select></div>\
    <div><textarea name=\"t\">zz</textarea></div>\
    <div><button name=\"b\" value=\"ok\">Send</button></div></form></body></html>";

/// Checkbox + submit, chacun sur sa ligne.
const PAGE_CB_FORM: &str = "<html><body><form action=\"/cbgo\">\
    <div><input type=\"checkbox\" name=\"c\" value=\"1\"></div>\
    <div><input type=\"submit\" value=\"Go\"></div></form></body></html>";

/// Deux radios homonymes + submit.
const PAGE_RADIO_FORM: &str = "<html><body><form action=\"/rgo\"><div>\
    <input type=\"radio\" name=\"r\" value=\"x\" checked>\
    <input type=\"radio\" name=\"r\" value=\"y\"></div>\
    <div><input type=\"submit\" value=\"Go\"></div></form></body></html>";

/// Select à deux options + submit.
const PAGE_SEL_FORM: &str = "<html><body><form action=\"/sgo\"><div>\
    <select name=\"s\"><option value=\"o1\">One</option>\
    <option value=\"o2\">Two</option></select></div>\
    <div><input type=\"submit\" value=\"Go\"></div></form></body></html>";

/// `submit` JS preventDefault → pas de navigation.
const PAGE_PREVENT_FORM: &str = "<html><body><form action=\"/go\">\
    <div><input type=\"submit\" value=\"Go\"></div></form>\
    <script>document.querySelector('form').addEventListener('submit',\
    function(e){e.preventDefault()});</script></body></html>";

#[tokio::test]
async fn form_get_submit_sends_query() {
    let (mut s, t) = test_session();
    t.route("/f", 200, PAGE_GET_FORM);
    t.route("/search", 200, "<html><body>results</body></html>");
    let id = s.open_tab();
    s.navigate(id, "http://h.test/f").await.unwrap();
    let out = s
        .handle_input(id, InputEvent::Click { x: 6.0, y: 36.0 })
        .await
        .unwrap();
    assert_eq!(out, InputOutcome::Navigated);
    let (m, pq, _) = t.last().unwrap();
    assert_eq!(m, "Get");
    assert_eq!(pq, "/search?q=hi+there");
    assert_eq!(
        s.tab_info(id).unwrap().url.as_deref(),
        Some("http://h.test/search?q=hi+there")
    );
}

#[tokio::test]
async fn form_post_submit_sends_urlencoded_body() {
    let (mut s, t) = test_session();
    t.route("/f", 200, PAGE_POST_FORM);
    t.route("/doit", 200, "<html><body>done</body></html>");
    let id = s.open_tab();
    s.navigate(id, "http://h.test/f").await.unwrap();
    // Bouton : dernier div (~y 172-195 sous la textarea 4em
    // — le strut §10.8.1 élargit chaque line box).
    let out = s
        .handle_input(id, InputEvent::Click { x: 10.0, y: 183.0 })
        .await
        .unwrap();
    assert_eq!(out, InputOutcome::Navigated);
    let (m, pq, body) = t.last().unwrap();
    assert_eq!(m, "Post");
    assert_eq!(pq, "/doit");
    assert_eq!(
        String::from_utf8(body).unwrap(),
        "a=1&c=on&r=x&s=o2&t=zz&b=ok"
    );
}

#[tokio::test]
async fn checkbox_toggles_then_submits() {
    let (mut s, t) = test_session();
    t.route("/cb", 200, PAGE_CB_FORM);
    t.route("/cbgo", 200, "<html><body>done</body></html>");
    let id = s.open_tab();
    s.navigate(id, "http://h.test/cb").await.unwrap();
    let before = s.frame_png(id);
    // Coche la case (marque peinte) → Edited.
    let out = s
        .handle_input(id, InputEvent::Click { x: 6.0, y: 6.0 })
        .await
        .unwrap();
    assert_eq!(out, InputOutcome::Edited);
    assert_ne!(s.frame_png(id), before);
    // Submit → `c=1` part dans la query.
    let out = s
        .handle_input(id, InputEvent::Click { x: 6.0, y: 40.0 })
        .await
        .unwrap();
    assert_eq!(out, InputOutcome::Navigated);
    assert_eq!(t.last().unwrap().1, "/cbgo?c=1");
}

#[tokio::test]
async fn radio_group_is_exclusive() {
    let (mut s, t) = test_session();
    t.route("/r", 200, PAGE_RADIO_FORM);
    t.route("/rgo", 200, "<html><body>done</body></html>");
    let id = s.open_tab();
    s.navigate(id, "http://h.test/r").await.unwrap();
    // 2e radio (après la 1re de 13px + gap).
    let out = s
        .handle_input(id, InputEvent::Click { x: 20.0, y: 6.0 })
        .await
        .unwrap();
    assert_eq!(out, InputOutcome::Edited);
    let out = s
        .handle_input(id, InputEvent::Click { x: 6.0, y: 40.0 })
        .await
        .unwrap();
    assert_eq!(out, InputOutcome::Navigated);
    // `r=y` seul — le `checked` de x a été retiré.
    assert_eq!(t.last().unwrap().1, "/rgo?r=y");
}

/// Clic sur le `<select>` → popup moteur ; clic sur la 2e
/// ligne → `o2` choisie ; submit → `s=o2` (étape 54 — le
/// popup a remplacé le cycle MVP d'ADR-0014).
#[tokio::test]
async fn select_popup_opens_and_chooses_option() {
    let (mut s, t) = test_session();
    t.route("/sl", 200, PAGE_SEL_FORM);
    t.route("/sgo", 200, "<html><body>done</body></html>");
    let id = s.open_tab();
    s.navigate(id, "http://h.test/sl").await.unwrap();
    let closed = s.frame_png(id);
    // Clic sur le select → le popup s'ouvre (repeint).
    let out = s
        .handle_input(id, InputEvent::Click { x: 10.0, y: 6.0 })
        .await
        .unwrap();
    assert_eq!(out, InputOutcome::Edited);
    assert_ne!(s.frame_png(id), closed);
    // 2e ligne du popup (sous le contrôle, ~y 36-54).
    let out = s
        .handle_input(id, InputEvent::Click { x: 10.0, y: 45.0 })
        .await
        .unwrap();
    assert_eq!(out, InputOutcome::Edited);
    // Submit → `s=o2` (l'option cliquée, pas la 1re).
    let out = s
        .handle_input(id, InputEvent::Click { x: 6.0, y: 40.0 })
        .await
        .unwrap();
    assert_eq!(out, InputOutcome::Navigated);
    assert_eq!(t.last().unwrap().1, "/sgo?s=o2");
}

/// Clic hors du popup ouvert → il se referme sans choix —
/// le clic suivant soumet `s=o1` (1re option par défaut).
#[tokio::test]
async fn select_popup_dismisses_on_outside_click() {
    let (mut s, t) = test_session();
    t.route("/sl", 200, PAGE_SEL_FORM);
    t.route("/sgo", 200, "<html><body>done</body></html>");
    let id = s.open_tab();
    s.navigate(id, "http://h.test/sl").await.unwrap();
    let out = s
        .handle_input(id, InputEvent::Click { x: 10.0, y: 6.0 })
        .await
        .unwrap();
    assert_eq!(out, InputOutcome::Edited);
    // Clic en plein vide — consommé pour refermer.
    let out = s
        .handle_input(id, InputEvent::Click { x: 300.0, y: 300.0 })
        .await
        .unwrap();
    assert_eq!(out, InputOutcome::Edited);
    let out = s
        .handle_input(id, InputEvent::Click { x: 6.0, y: 40.0 })
        .await
        .unwrap();
    assert_eq!(out, InputOutcome::Navigated);
    assert_eq!(t.last().unwrap().1, "/sgo?s=o1");
}

/// Popup ouvert : `ArrowDown` déplace le surlignage,
/// `Enter` choisit la ligne, `Escape` referme sans choix.
#[tokio::test]
async fn select_popup_keyboard_navigates() {
    let (mut s, t) = test_session();
    t.route("/sl", 200, PAGE_SEL_FORM);
    t.route("/sgo", 200, "<html><body>done</body></html>");
    let id = s.open_tab();
    s.navigate(id, "http://h.test/sl").await.unwrap();
    s.handle_input(id, InputEvent::Click { x: 10.0, y: 6.0 })
        .await
        .unwrap();
    // Deux `ArrowDown` → 2e option, `Enter` la choisit.
    for k in ["ArrowDown", "ArrowDown", "Enter"] {
        s.handle_input(
            id,
            InputEvent::Key {
                key: k.to_string(),
                mods: KeyMods::NONE,
            },
        )
        .await
        .unwrap();
    }
    let out = s
        .handle_input(id, InputEvent::Click { x: 6.0, y: 40.0 })
        .await
        .unwrap();
    assert_eq!(out, InputOutcome::Navigated);
    assert_eq!(t.last().unwrap().1, "/sgo?s=o2");
}

/// Étape 63 — le popup peint les **étiquettes** des options
/// en vrai texte (avant : `Placeholder` = boîte grise sans
/// glyphe — zéro pixel d'encre). Des pixels sombres doivent
/// apparaître dans les lignes du popup ouvert.
#[tokio::test]
async fn select_popup_paints_option_text() {
    let (mut s, t) = test_session();
    t.route("/sl", 200, PAGE_SEL_FORM);
    let id = s.open_tab();
    s.navigate(id, "http://h.test/sl").await.unwrap();
    // Clic sur le select → popup ouvert.
    s.handle_input(id, InputEvent::Click { x: 10.0, y: 6.0 })
        .await
        .unwrap();
    let (fw, fh, px) = s.frame_pixels(id).unwrap();
    // Le popup vit sous le contrôle (≈y 15..110, x 0..160) —
    // encre #202020 sur fond blanc : compter les pixels
    // franchement sombres (border 118 gris et placeholder
    // ~0xcc clair sont exclus par <80).
    let (fw, fh) = (fw as usize, fh as usize);
    let mut dark = 0usize;
    for row in 15..fh.min(110) {
        for col in 0..fw.min(160) {
            let p = &px[(row * fw + col) * 4..][..4];
            if p[0] < 80 && p[1] < 80 && p[2] < 80 {
                dark += 1;
            }
        }
    }
    assert!(
        dark > 50,
        "les étiquettes du popup doivent peindre de l'encre (pixels sombres : {dark})"
    );
    // `ArrowDown` déplace le surlignage — la frame change
    // (retour visuel du clavier, aussi vérifié en valeur
    // par `select_popup_keyboard_navigates`).
    let before = s.frame_png(id).unwrap();
    s.handle_input(
        id,
        InputEvent::Key {
            key: "ArrowDown".to_string(),
            mods: KeyMods::NONE,
        },
    )
    .await
    .unwrap();
    assert_ne!(s.frame_png(id).unwrap(), before);
}

/// Étape 64 — `Navigated` n'est rapporté qu'au **commit
/// effectif** : une soumission dont le fetch meurt (404)
/// laisse l'URL inchangée — l'outcome dégrade en `Edited`
/// (avant : l'intention était rapportée comme un état
/// commis — l'UI affichait une URL jamais atteinte).
#[tokio::test]
async fn failed_submit_reports_edited_not_navigated() {
    let (mut s, t) = test_session();
    t.route("/sl", 200, PAGE_SEL_FORM);
    // `/sgo` non routé → 404 → la nav échoue.
    let id = s.open_tab();
    s.navigate(id, "http://h.test/sl").await.unwrap();
    // Focus le champ texte puis Enter → soumission implicite.
    s.handle_input(id, InputEvent::Click { x: 6.0, y: 6.0 })
        .await
        .unwrap();
    let out = s
        .handle_input(
            id,
            InputEvent::Key {
                key: "Enter".to_string(),
                mods: KeyMods::NONE,
            },
        )
        .await
        .unwrap();
    assert_eq!(out, InputOutcome::Edited, "{out:?}");
    assert_eq!(
        s.tab_info(id).and_then(|t| t.url).as_deref(),
        Some("http://h.test/sl")
    );
}

/// Étape 53e — le `charset=` du `Content-Type` transport
/// l'emporte sur le prescan `<meta>` absent : un titre
/// UTF-8 servi `text/html; charset=utf-8` n'est plus relu
/// en windows-1252 (repro HN `â€“`).
#[tokio::test]
async fn document_charset_honors_transport_header() {
    let (mut s, t) = test_session();
    // UTF-8 réel (« Actualités – quotidien »), aucun <meta>
    // charset — seul l'en-tête transport le déclare.
    t.route_headers(
        "/hn",
        200,
        "<html><head><title>Actualités – quotidien</title></head>\
         <body><p>été déjà</p></body></html>",
        &[("content-type", "text/html; charset=utf-8")],
    );
    let id = s.open_tab();
    s.navigate(id, "http://h.test/hn").await.unwrap();
    let title = s.tab_info(id).and_then(|t| t.title).unwrap_or_default();
    assert!(
        title.contains("Actualités"),
        "titre relu en 1252 ? {title:?}"
    );
    assert!(title.contains('–'), "titre : {title:?}");
}

/// Étape 65 — `tab_info.doc_h` rapporte la hauteur
/// **mesurée** (pas le plafond raster 2¹⁴) et
/// `doc_h_clamped` signale que la frame ne couvre pas le
/// document entier.
#[tokio::test]
async fn doc_h_reports_measured_height_not_frame_cap() {
    let (mut s, t) = test_session();
    t.route(
        "/tall",
        200,
        "<html><body><div style=\"height:20000px\">deep</div></body></html>",
    );
    t.route("/short", 200, "<html><body><p>x</p></body></html>");
    let id = s.open_tab();
    s.navigate(id, "http://h.test/tall").await.unwrap();
    let info = s.tab_info(id).unwrap();
    assert!(info.doc_h > 16_384.0, "doc_h={}", info.doc_h);
    assert!(info.doc_h_clamped);
    s.navigate(id, "http://h.test/short").await.unwrap();
    let info = s.tab_info(id).unwrap();
    assert!(!info.doc_h_clamped, "doc_h={}", info.doc_h);
}

/// R-87 — `html,body{height:100%}` (vector-2022) : le contenu
/// qui déborde le body à hauteur spécifiée reste **défilable** —
/// `doc_h` suit le bas de la région défilante, pas la boîte
/// `body`. Terrain : fr.wikipedia ≥1120 px rendait un document
/// d'un viewport — scroll mort, bas de page clippé.
#[tokio::test]
async fn doc_h_covers_overflow_past_fixed_height_body() {
    let (mut s, t) = test_session();
    t.route(
        "/ovf",
        200,
        "<!doctype html><html><head><style>html,body{height:100%}</style></head>\
         <body><div style=\"height:2000px\">haut</div></body></html>",
    );
    let id = s.open_tab();
    s.navigate(id, "http://h.test/ovf").await.unwrap();
    let info = s.tab_info(id).unwrap();
    assert!(info.doc_h >= 2000.0, "doc_h={}", info.doc_h);
    // Le scroll avance réellement — avant le fix, le clamp à
    // `doc_h − viewport = 0` clouait la page.
    let out = s
        .handle_input(id, InputEvent::Scroll { dy: 300.0 })
        .await
        .unwrap();
    match out {
        InputOutcome::Scrolled { scroll_y } => {
            assert!((scroll_y - 300.0).abs() < 0.01, "scroll_y={scroll_y}");
        }
        o => panic!("inattendu : {o:?}"),
    }
}

/// `Escape` referme sans choix — `s=o1` à la soumission.
#[tokio::test]
async fn select_popup_escape_keeps_default() {
    let (mut s, t) = test_session();
    t.route("/sl", 200, PAGE_SEL_FORM);
    t.route("/sgo", 200, "<html><body>done</body></html>");
    let id = s.open_tab();
    s.navigate(id, "http://h.test/sl").await.unwrap();
    s.handle_input(id, InputEvent::Click { x: 10.0, y: 6.0 })
        .await
        .unwrap();
    s.handle_input(
        id,
        InputEvent::Key {
            key: "Escape".to_string(),
            mods: KeyMods::NONE,
        },
    )
    .await
    .unwrap();
    let out = s
        .handle_input(id, InputEvent::Click { x: 6.0, y: 40.0 })
        .await
        .unwrap();
    assert_eq!(out, InputOutcome::Navigated);
    assert_eq!(t.last().unwrap().1, "/sgo?s=o1");
}

#[tokio::test]
async fn submit_preventdefault_skips_navigation() {
    let (mut s, t) = test_session();
    t.route("/p", 200, PAGE_PREVENT_FORM);
    s.set_js_factory(explorer_js::JscoreEngine::factory());
    let id = s.open_tab();
    s.navigate(id, "http://h.test/p").await.unwrap();
    let calls = t.calls();
    let out = s
        .handle_input(id, InputEvent::Click { x: 6.0, y: 6.0 })
        .await
        .unwrap();
    assert_eq!(out, InputOutcome::Ignored);
    // Aucun fetch supplémentaire, URL inchangée.
    assert_eq!(t.calls(), calls);
    assert_eq!(
        s.tab_info(id).unwrap().url.as_deref(),
        Some("http://h.test/p")
    );
}

// ---- Étape 46 : feuilles liées + fontes web ----

/// TTF de la fixture dont `hmtx` est patchée (avance 800 au
/// lieu de 500) — texte plus large → PNG discriminants.
fn fat_fixture_ttf() -> Vec<u8> {
    let mut ttf = fixture_ttf();
    let n = u16::from_be_bytes([ttf[4], ttf[5]]) as usize;
    for i in 0..n {
        let rec = 12 + i * 16;
        if ttf[rec..rec + 4] == *b"hmtx" {
            let off = u32::from_be_bytes([ttf[rec + 8], ttf[rec + 9], ttf[rec + 10], ttf[rec + 11]])
                as usize;
            for g in 0..256 {
                ttf[off + g * 4..off + g * 4 + 2].copy_from_slice(&800u16.to_be_bytes());
            }
            return ttf;
        }
    }
    ttf
}

/// Enveloppe WOFF1 sans compression (compLen == origLen)
/// autour d'un sfnt — reconstruit le répertoire table par
/// table depuis le sfnt source.
// Tailles/offsets issus de `len()` de fixtures — bornés.
#[allow(clippy::cast_possible_truncation)]
fn wrap_woff1(sfnt: &[u8]) -> Vec<u8> {
    let n = u16::from_be_bytes([sfnt[4], sfnt[5]]) as usize;
    let mut tables: Vec<([u8; 4], &[u8])> = Vec::with_capacity(n);
    for i in 0..n {
        let rec = 12 + i * 16;
        let tag: [u8; 4] = sfnt[rec..rec + 4].try_into().unwrap();
        let off = u32::from_be_bytes([sfnt[rec + 8], sfnt[rec + 9], sfnt[rec + 10], sfnt[rec + 11]])
            as usize;
        let len = u32::from_be_bytes([
            sfnt[rec + 12],
            sfnt[rec + 13],
            sfnt[rec + 14],
            sfnt[rec + 15],
        ]) as usize;
        tables.push((tag, &sfnt[off..off + len]));
    }
    let mut w = Vec::new();
    let push = |w: &mut Vec<u8>, x: u32| w.extend_from_slice(&x.to_be_bytes());
    w.extend_from_slice(b"wOFF");
    push(&mut w, 0x0001_0000);
    push(&mut w, 0); // length (ignoré par le décodeur)
    w.extend_from_slice(&(n as u16).to_be_bytes());
    w.extend_from_slice(&[0; 2]); // reserved
    for _ in 0..7 {
        push(&mut w, 0); // totalSfntSize, versions, meta, priv
    }
    let data_start = 44 + 20 * n;
    let mut body = Vec::new();
    for (tag, data) in &tables {
        w.extend_from_slice(tag);
        push(&mut w, (data_start + body.len()) as u32);
        push(&mut w, data.len() as u32);
        push(&mut w, data.len() as u32);
        push(&mut w, 0);
        body.extend_from_slice(data);
        while body.len() % 4 != 0 {
            body.push(0);
        }
    }
    w.extend_from_slice(&body);
    w
}

const PAGE_WEBFONT: &str = r#"<!doctype html><html><head><style>
@font-face { font-family: "WF"; src: url(/w.ttf); }
p { font-family: WF; font-size: 20px; margin: 0; }
</style></head><body><p>HELLO WORLD</p></body></html>"#;

const PAGE_LINKED: &str = r#"<!doctype html><html><head>
<link rel="stylesheet" href="/s.css">
</head><body><p>LINKED TEXT</p></body></html>"#;

const LINKED_CSS: &str = r#"
@font-face { font-family: "WF"; src: url(/w.woff) format("woff"); font-weight: 700; }
p { font-family: WF; font-size: 20px; margin: 0; }
"#;

#[tokio::test]
async fn webfont_ttf_fetched_and_used() {
    let (mut s, t) = test_session();
    t.route("/p", 200, PAGE_WEBFONT);
    t.route_bin("/w.ttf", 200, fat_fixture_ttf());
    let id = s.open_tab();
    s.navigate(id, "http://h.test/p").await.unwrap();
    // La fonte est fetchée et ses avances (800) produisent un
    // rendu différent du repli fixture (500).
    let with_font = s.frame_png(id).unwrap();
    let (mut s2, t2) = test_session();
    t2.route("/p", 200, PAGE_WEBFONT);
    t2.route_bin("/w.ttf", 404, Vec::new());
    let id2 = s2.open_tab();
    s2.navigate(id2, "http://h.test/p").await.unwrap();
    let fallback = s2.frame_png(id2).unwrap();
    assert_ne!(with_font, fallback);
}

#[tokio::test]
async fn webfont_woff_via_linked_stylesheet() {
    let (mut s, t) = test_session();
    t.route("/p", 200, PAGE_LINKED);
    t.route("/s.css", 200, LINKED_CSS);
    t.route_bin("/w.woff", 200, wrap_woff1(&fat_fixture_ttf()));
    let id = s.open_tab();
    s.navigate(id, "http://h.test/p").await.unwrap();
    // La feuille liée est fetchée, sa `@font-face` aussi ;
    // le texte utilise les avances 800 (≠ repli 500).
    let with_font = s.frame_png(id).unwrap();
    let (mut s2, t2) = test_session();
    t2.route("/p", 200, PAGE_LINKED);
    t2.route("/s.css", 200, LINKED_CSS);
    let id2 = s2.open_tab();
    s2.navigate(id2, "http://h.test/p").await.unwrap();
    assert_ne!(with_font, s2.frame_png(id2).unwrap());
}

#[tokio::test]
async fn webfont_register_bounded() {
    let (mut s, t) = {
        let t = Arc::new(FakeTransport::new());
        let s = Session::with_config(
            t.clone(),
            Arc::new(FixtureFont),
            SessionConfig {
                max_fonts: 1,
                ..SessionConfig::default()
            },
        );
        (s, t)
    };
    t.route(
        "/p",
        200,
        r"<!doctype html><html><head><style>
@font-face { font-family: A; src: url(/a.ttf); }
@font-face { font-family: B; src: url(/b.ttf); }
</style></head><body><p>x</p></body></html>",
    );
    t.route_bin("/a.ttf", 200, fixture_ttf());
    t.route_bin("/b.ttf", 200, fixture_ttf());
    let id = s.open_tab();
    s.navigate(id, "http://h.test/p").await.unwrap();
    // Page + a.ttf fetchés ; b.ttf jamais (registre plein).
    assert!(t.last().is_some_and(|(_, q, _)| q == "/a.ttf"));
    assert_eq!(t.calls(), 2);
}

// ---- Étape 44 (moteur) : navigation pilotée par script ----

#[tokio::test]
async fn script_location_href_navigates() {
    let (mut s, t) = test_session();
    t.route(
        "/p",
        200,
        r#"<!doctype html><html><body><script>
location.href = "/cible";
</script></body></html>"#,
    );
    t.route("/cible", 200, "<!doctype html><title>CI</title><p>dest</p>");
    s.set_js_factory(explorer_js::JscoreEngine::factory());
    let id = s.open_tab();
    s.navigate(id, "http://h.test/p").await.unwrap();
    // La navigation script s'applique après le chargement :
    // l'onglet est arrivé sur /cible, entrée d'historique
    // poussée par `navigate`.
    assert_eq!(
        s.tab_info(id).unwrap().url.as_deref(),
        Some("http://h.test/cible")
    );
}

#[tokio::test]
async fn script_history_back_moves() {
    let (mut s, t) = test_session();
    t.route("/a", 200, "<!doctype html><title>A</title><p>un</p>");
    t.route(
        "/b",
        200,
        "<!doctype html><html><body><script>history.back();</script></body></html>",
    );
    s.set_js_factory(explorer_js::JscoreEngine::factory());
    let id = s.open_tab();
    s.navigate(id, "http://h.test/a").await.unwrap();
    s.navigate(id, "http://h.test/b").await.unwrap();
    // `history.back()` pendant le chargement de /b → retour /a.
    assert_eq!(
        s.tab_info(id).unwrap().url.as_deref(),
        Some("http://h.test/a")
    );
}

#[tokio::test]
async fn script_location_invalid_url_ignored() {
    let (mut s, t) = test_session();
    t.route(
        "/p",
        200,
        r#"<!doctype html><html><body><script>
location.href = "ht!tp://[::1]bad";
history.go(99);
</script></body></html>"#,
    );
    s.set_js_factory(explorer_js::JscoreEngine::factory());
    let id = s.open_tab();
    s.navigate(id, "http://h.test/p").await.unwrap();
    // Le href irrégulier se résout en relatif contre la base
    // → la nav part quand même et échoue en 404 : depuis
    // l'étape 69 la page d'erreur moteur est commise sous
    // l'URL tentée (borne historique : toujours ignorée).
    assert_eq!(
        s.tab_info(id).unwrap().url.as_deref(),
        Some("http://h.test/ht!tp://%5B::1%5Dbad")
    );
    assert!(s.find_in_page(id, "Page inaccessible") > 0);
}

/// Transition (étape 45) : un handler `click` change `class`
/// → la resynchro relayout voit le diff `opacity` et
/// enregistre la transition — `tick_animations` avance
/// alors la frame.
#[tokio::test]
async fn transition_declenchee_sur_mutation_classe() {
    let (mut s, t) = test_session();
    t.route(
        "/tr",
        200,
        r#"<!doctype html><html><head><style>
        div { transition: opacity 60s linear; }
        .f { opacity: 0; }
        </style></head><body><div id="d" style="width:80px;height:80px">hello</div>
        <script>
        document.getElementById('d').addEventListener('click', function(){
          document.getElementById('d').setAttribute('class','f');
        });
        </script></body></html>"#,
    );
    s.set_js_factory(explorer_js::JscoreEngine::factory());
    let id = s.open_tab();
    s.navigate(id, "http://h.test/tr").await.unwrap();
    // Clic dans la div : la mutation synchronisée relance le
    // layout → diff opacity 1→0 avec `transition` → en vol.
    s.handle_input(id, InputEvent::Click { x: 5.0, y: 5.0 })
        .await
        .unwrap();
    assert!(s.tick_animations(id).unwrap());
    assert!(s.frame_png(id).is_some());
}

/// Sans transition déclarée, `tick_animations` est inerte.
#[tokio::test]
async fn tick_animations_sans_transition_inerte() {
    let (mut s, t) = test_session();
    t.route(
        "/n",
        200,
        "<!doctype html><html><body><p>x</p></body></html>",
    );
    let id = s.open_tab();
    s.navigate(id, "http://h.test/n").await.unwrap();
    assert!(!s.tick_animations(id).unwrap());
}

// ---- Étape 50 : `<script src>` + scheduler de sous-ressources ----

/// `<script src>` fetché via `HttpTransport` puis évalué :
/// la mutation du miroir doit être rendue (étape 50).
#[tokio::test]
async fn external_script_fetched_and_executed() {
    let (mut s, t) = test_session();
    t.route(
        "/p",
        200,
        "<!doctype html><html><body><div id=\"x\">avant</div>\
         <script src=\"/a.js\"></script></body></html>",
    );
    t.route(
        "/a.js",
        200,
        "document.getElementById('x').textContent='EXT'",
    );
    t.route(
        "/stat",
        200,
        "<!doctype html><html><body><div id=\"x\">EXT</div></body></html>",
    );
    s.set_js_factory(explorer_js::JscoreEngine::factory());

    let a = s.open_tab();
    s.navigate(a, "http://h.test/p").await.unwrap();
    let b = s.open_tab();
    s.navigate(b, "http://h.test/stat").await.unwrap();

    // Page + script externe + page statique = 3 fetches.
    assert_eq!(t.calls(), 3);
    assert_eq!(s.frame_png(a), s.frame_png(b));
}

/// Deux `<script src>` classiques : ordre document strict —
/// le second voit l'état laissé par le premier.
#[tokio::test]
async fn external_scripts_run_in_document_order() {
    let (mut s, t) = test_session();
    t.route(
        "/p",
        200,
        "<!doctype html><html><body><div id=\"x\">avant</div>\
         <script src=\"/a.js\"></script><script src=\"/b.js\"></script>\
         <script>document.getElementById('x').textContent=o</script>\
         </body></html>",
    );
    t.route("/a.js", 200, "var o='A'");
    t.route("/b.js", 200, "o=o+'B'");
    t.route(
        "/stat",
        200,
        "<!doctype html><html><body><div id=\"x\">AB</div></body></html>",
    );
    s.set_js_factory(explorer_js::JscoreEngine::factory());

    let a = s.open_tab();
    s.navigate(a, "http://h.test/p").await.unwrap();
    let b = s.open_tab();
    s.navigate(b, "http://h.test/stat").await.unwrap();

    // Ordre inversé (b avant a) : `o` indéfini → erreur →
    // le texte resterait « avant ».
    assert_eq!(s.frame_png(a), s.frame_png(b));
}

/// `defer` : exécuté après les scripts synchrones (post-parse)
/// même déclaré avant, et dans l'ordre document entre eux.
#[tokio::test]
async fn defer_scripts_run_after_parse_in_order() {
    let (mut s, t) = test_session();
    t.route(
        "/p",
        200,
        "<!doctype html><html><body><div id=\"x\">avant</div>\
         <script defer src=\"/d1.js\"></script>\
         <script src=\"/p.js\"></script>\
         <script defer src=\"/d2.js\"></script>\
         </body></html>",
    );
    t.route("/p.js", 200, "var o='P'");
    t.route("/d1.js", 200, "o=o+'-D1'");
    t.route(
        "/d2.js",
        200,
        "o=o+'-D2';document.getElementById('x').textContent=o",
    );
    t.route(
        "/stat",
        200,
        "<!doctype html><html><body><div id=\"x\">P-D1-D2</div></body></html>",
    );
    s.set_js_factory(explorer_js::JscoreEngine::factory());

    let a = s.open_tab();
    s.navigate(a, "http://h.test/p").await.unwrap();
    let b = s.open_tab();
    s.navigate(b, "http://h.test/stat").await.unwrap();

    assert_eq!(s.frame_png(a), s.frame_png(b));
}

/// `async` : évalué à réception, donc avant la file
/// post-parse (`defer`) même déclaré après dans le document.
#[tokio::test]
async fn async_script_runs_before_defer() {
    let (mut s, t) = test_session();
    t.route(
        "/p",
        200,
        "<!doctype html><html><body><div id=\"x\">avant</div>\
         <script defer src=\"/d.js\"></script>\
         <script async src=\"/x.js\"></script>\
         </body></html>",
    );
    t.route("/x.js", 200, "var o='X'");
    t.route(
        "/d.js",
        200,
        "o=o+'D';document.getElementById('x').textContent=o",
    );
    t.route(
        "/stat",
        200,
        "<!doctype html><html><body><div id=\"x\">XD</div></body></html>",
    );
    s.set_js_factory(explorer_js::JscoreEngine::factory());

    let a = s.open_tab();
    s.navigate(a, "http://h.test/p").await.unwrap();
    let b = s.open_tab();
    s.navigate(b, "http://h.test/stat").await.unwrap();

    // `d` avant `x` : `o` indéfini → erreur → « avant ».
    assert_eq!(s.frame_png(a), s.frame_png(b));
}

/// `<script type="module">` externe avec import statique :
/// le graphe est prefetché via le transport puis instancié
/// par le `ModuleLoader` (JS-C5) — la dépendance s'exécute.
#[tokio::test]
async fn module_script_fetches_graph_and_evaluates() {
    let (mut s, t) = test_session();
    t.route(
        "/p",
        200,
        "<!doctype html><html><body><div id=\"x\">avant</div>\
         <script type=\"module\" src=\"/m.js\"></script></body></html>",
    );
    t.route("/m.js", 200, "import './dep.js'");
    t.route(
        "/dep.js",
        200,
        "document.getElementById('x').textContent='DEP'",
    );
    t.route(
        "/stat",
        200,
        "<!doctype html><html><body><div id=\"x\">DEP</div></body></html>",
    );
    s.set_js_factory(explorer_js::JscoreEngine::factory());

    let a = s.open_tab();
    s.navigate(a, "http://h.test/p").await.unwrap();
    let b = s.open_tab();
    s.navigate(b, "http://h.test/stat").await.unwrap();

    // Page + module + dépendance + page statique = 4 fetches.
    assert_eq!(t.calls(), 4);
    assert_eq!(s.frame_png(a), s.frame_png(b));
}

/// Module inline : nom synthétique, dépendances résolues
/// contre l'URL de la page.
#[tokio::test]
async fn inline_module_resolves_against_page() {
    let (mut s, t) = test_session();
    t.route(
        "/p",
        200,
        "<!doctype html><html><body><div id=\"x\">avant</div>\
         <script type=\"module\">import './d.js'</script></body></html>",
    );
    t.route(
        "/d.js",
        200,
        "document.getElementById('x').textContent='IM'",
    );
    t.route(
        "/stat",
        200,
        "<!doctype html><html><body><div id=\"x\">IM</div></body></html>",
    );
    s.set_js_factory(explorer_js::JscoreEngine::factory());

    let a = s.open_tab();
    s.navigate(a, "http://h.test/p").await.unwrap();
    let b = s.open_tab();
    s.navigate(b, "http://h.test/stat").await.unwrap();

    assert_eq!(t.calls(), 3);
    assert_eq!(s.frame_png(a), s.frame_png(b));
}

/// Un `<script src>` en 404 est sauté : les scripts suivants
/// s'exécutent, la page charge (dégradation, règle 3).
#[tokio::test]
async fn external_script_404_is_skipped() {
    let (mut s, t) = test_session();
    t.route(
        "/p",
        200,
        "<!doctype html><html><body><div id=\"x\">avant</div>\
         <script src=\"/missing.js\"></script>\
         <script>document.getElementById('x').textContent='OK'</script>\
         </body></html>",
    );
    s.set_js_factory(explorer_js::JscoreEngine::factory());
    let a = s.open_tab();
    s.navigate(a, "http://h.test/p").await.unwrap();
    // La page charge et le script inline s'est exécuté.
    assert_eq!(s.tab_url(a).as_deref(), Some("http://h.test/p"));
    assert!(s.frame_png(a).is_some());
}

/// Sans fabrique JS, aucun `<script src>` n'est fetché
/// (scripting désactivé — spec HTML).
#[tokio::test]
async fn no_factory_means_script_src_not_fetched() {
    let (mut s, t) = test_session();
    t.route(
        "/p",
        200,
        "<!doctype html><html><body><div>x</div>\
         <script src=\"/a.js\"></script></body></html>",
    );
    t.route("/a.js", 200, "var o=1");
    let id = s.open_tab();
    s.navigate(id, "http://h.test/p").await.unwrap();
    assert_eq!(t.calls(), 1);
}

/// Transport qui mesure le pic de requêtes simultanées —
/// prouve le parallélisme ET sa borne (`max_inflight`).
struct ConcurrencyTransport {
    routes: Mutex<HashMap<String, (u16, Vec<u8>)>>,
    current: std::sync::atomic::AtomicUsize,
    peak: std::sync::atomic::AtomicUsize,
}

#[async_trait]
impl HttpTransport for ConcurrencyTransport {
    async fn fetch(&self, req: &Request) -> NetResult<Response> {
        use std::sync::atomic::Ordering;
        let n = self.current.fetch_add(1, Ordering::SeqCst) + 1;
        self.peak.fetch_max(n, Ordering::SeqCst);
        // Cède le tour : une 2e requête en vol s'observe.
        tokio::task::yield_now().await;
        let found = self
            .routes
            .lock()
            .ok()
            .and_then(|r| r.get(req.url.path()).cloned());
        self.current.fetch_sub(1, Ordering::SeqCst);
        match found {
            Some((status, body)) => Ok(Response {
                status,
                reason: "fake".to_string(),
                headers: vec![],
                body,
                reused_connection: false,
                resolved_url: None,
            }),
            None => Ok(Response {
                status: 404,
                reason: "fake".to_string(),
                headers: vec![],
                body: b"missing".to_vec(),
                reused_connection: false,
                resolved_url: None,
            }),
        }
    }
}

/// Scheduler borné : 4 `<img>` sous `max_inflight = 2` →
/// pic exactement 2 (jamais 1 = séquentiel, jamais 4 = libre).
#[tokio::test]
async fn subresource_fetches_bounded_parallel() {
    let t = Arc::new(ConcurrencyTransport {
        routes: Mutex::new(HashMap::new()),
        current: std::sync::atomic::AtomicUsize::new(0),
        peak: std::sync::atomic::AtomicUsize::new(0),
    });
    if let Ok(mut r) = t.routes.lock() {
        r.insert(
            "/p".to_string(),
            (
                200,
                b"<!doctype html><html><body>\
                  <img src=\"/i1\"><img src=\"/i2\">\
                  <img src=\"/i3\"><img src=\"/i4\">\
                  </body></html>"
                    .to_vec(),
            ),
        );
        for p in ["/i1", "/i2", "/i3", "/i4"] {
            r.insert(p.to_string(), (200, b"img".to_vec()));
        }
    }
    let mut s = Session::with_config(
        t.clone(),
        Arc::new(FixtureFont),
        SessionConfig {
            max_inflight: 2,
            ..SessionConfig::default()
        },
    );
    let id = s.open_tab();
    s.navigate(id, "http://h.test/p").await.unwrap();
    assert_eq!(t.peak.load(std::sync::atomic::Ordering::SeqCst), 2);
}

// ===== Étape 51 — XHR/fetch/storage/cookie (ADR-0019) =====
//
// Vrai `JscoreEngine` : les scripts émettent de vrais
// `NetRequest` drainés par la session ; les observables sont
// le PNG (mutation rendue) et les en-têtes capturés par le
// `FakeTransport`.

/// XHR `GET /api` : la réponse remplit le div cible.
const PAGE_XHR: &str = "<!doctype html><html><body>\
    <div id=\"x\">vide</div>\
    <script>var q=new XMLHttpRequest();q.open('GET','/api');\
    q.onload=function(){document.getElementById('x').textContent=q.responseText};\
    q.send()</script></body></html>";

/// Équivalent statique après mutation XHR.
const PAGE_XHR_DATA: &str = "<!doctype html><html><body><div id=\"x\">DATA</div></body></html>";

#[tokio::test]
async fn xhr_same_origin_fetches_and_renders() {
    let (mut s, t) = test_session();
    t.route("/", 200, PAGE_XHR);
    t.route("/api", 200, "DATA");
    t.route("/stat", 200, PAGE_XHR_DATA);
    s.set_js_factory(explorer_js::JscoreEngine::factory());
    let a = s.open_tab();
    s.navigate(a, "http://h.test/").await.unwrap();
    // `Referer` posé à l'émission de la requête script.
    let referer = t.last_header("referer");
    assert!(
        referer
            .as_ref()
            .is_some_and(|r| r.starts_with("http://h.test")),
        "referer attendu sur la XHR, reçu {referer:?}"
    );
    let b = s.open_tab();
    s.navigate(b, "http://h.test/stat").await.unwrap();
    assert_eq!(
        s.frame_png(a),
        s.frame_png(b),
        "XHR same-origin livrée et rendue"
    );
}

/// XHR cross-origin : `onerror` → `ERR`, `onload` → `OK`.
const PAGE_XHR_CROSS: &str = "<!doctype html><html><body>\
    <div id=\"x\">?</div>\
    <script>var q=new XMLHttpRequest();q.open('GET','http://b.test/api');\
    q.onerror=function(){document.getElementById('x').textContent='ERR'};\
    q.onload=function(){document.getElementById('x').textContent='OK'};\
    q.send()</script></body></html>";

#[tokio::test]
async fn xhr_cross_origin_opaque_without_acao() {
    let (mut s, t) = test_session();
    t.route("/", 200, PAGE_XHR_CROSS);
    // La route est servie par path (hôte ignoré) — la réponse
    // arrive SANS `Access-Control-Allow-Origin` → opaque.
    t.route("/api", 200, "DATA");
    t.route(
        "/stat",
        200,
        "<!doctype html><html><body><div id=\"x\">ERR</div></body></html>",
    );
    s.set_js_factory(explorer_js::JscoreEngine::factory());
    let a = s.open_tab();
    s.navigate(a, "http://a.test/").await.unwrap();
    // Émission OK : `Origin` canonique posé (spec Fetch —
    // toute requête CORS le porte, même GET).
    assert_eq!(
        t.last_header("origin").as_deref(),
        Some("http://a.test"),
        "Origin canonique attendu sur la requête CORS"
    );
    let b = s.open_tab();
    s.navigate(b, "http://a.test/stat").await.unwrap();
    assert_eq!(
        s.frame_png(a),
        s.frame_png(b),
        "réponse sans ACAO = opaque → événement error"
    );
}

#[tokio::test]
async fn xhr_cross_origin_readable_with_acao() {
    let (mut s, t) = test_session();
    t.route("/", 200, PAGE_XHR_CROSS);
    t.route_headers("/api", 200, "DATA", &[("access-control-allow-origin", "*")]);
    t.route(
        "/stat",
        200,
        "<!doctype html><html><body><div id=\"x\">OK</div></body></html>",
    );
    s.set_js_factory(explorer_js::JscoreEngine::factory());
    let a = s.open_tab();
    s.navigate(a, "http://a.test/").await.unwrap();
    let b = s.open_tab();
    s.navigate(b, "http://a.test/stat").await.unwrap();
    assert_eq!(
        s.frame_png(a),
        s.frame_png(b),
        "ACAO `*` → réponse lisible par le script"
    );
}

/// XHR cross-origin avec en-tête non safelisté — preflight
/// `OPTIONS` émis par `CorsTransport` (étape 126) ; la
/// réponse sans `Access-Control-Allow-Headers` refuse la
/// requête (`error` côté script, jamais la vraie `GET`).
const PAGE_XHR_PREFLIGHT: &str = "<!doctype html><html><body>\
    <div id=\"x\">?</div>\
    <script>var q=new XMLHttpRequest();q.open('GET','http://b.test/api');\
    q.setRequestHeader('X-Token','t');\
    q.onerror=function(){document.getElementById('x').textContent='ERR'};\
    q.send()</script></body></html>";

#[tokio::test]
async fn xhr_cross_origin_custom_header_blocked() {
    let (mut s, t) = test_session();
    t.route("/", 200, PAGE_XHR_PREFLIGHT);
    t.route("/api", 200, "DATA");
    t.route(
        "/stat",
        200,
        "<!doctype html><html><body><div id=\"x\">ERR</div></body></html>",
    );
    s.set_js_factory(explorer_js::JscoreEngine::factory());
    let a = s.open_tab();
    s.navigate(a, "http://a.test/").await.unwrap();
    // Deux appels : le document + le preflight `OPTIONS`
    // (refusé : `ACAH`/`ACAO` absents) — la vraie requête ne
    // part jamais.
    assert_eq!(t.calls(), 2, "document + preflight OPTIONS refusé");
    let b = s.open_tab();
    s.navigate(b, "http://a.test/stat").await.unwrap();
    assert_eq!(
        s.frame_png(a),
        s.frame_png(b),
        "en-tête non safelisté cross-origin → error"
    );
}

/// Deux XHR, quota 1 : la seconde échoue d'office (`error`),
/// la première complète.
const PAGE_XHR_QUOTA: &str = "<!doctype html><html><body>\
    <div id=\"a\">?</div><div id=\"b\">?</div>\
    <script>\
    var q=new XMLHttpRequest();q.open('GET','/api');\
    q.onload=function(){document.getElementById('a').textContent='OK'};\
    var r=new XMLHttpRequest();r.open('GET','/api');\
    r.onerror=function(){document.getElementById('b').textContent='KO'};\
    q.send();r.send()</script></body></html>";

#[tokio::test]
async fn xhr_quota_exceeded_fails_fast() {
    let t = Arc::new(FakeTransport::new());
    t.route("/", 200, PAGE_XHR_QUOTA);
    t.route("/api", 200, "DATA");
    t.route(
        "/stat",
        200,
        "<!doctype html><html><body><div id=\"a\">OK</div>\
         <div id=\"b\">KO</div></body></html>",
    );
    let mut s = Session::with_config(
        t.clone(),
        Arc::new(FixtureFont),
        SessionConfig {
            max_script_net: 1,
            ..SessionConfig::default()
        },
    );
    s.set_js_factory(explorer_js::JscoreEngine::factory());
    let a = s.open_tab();
    s.navigate(a, "http://h.test/").await.unwrap();
    let b = s.open_tab();
    s.navigate(b, "http://h.test/stat").await.unwrap();
    assert_eq!(
        s.frame_png(a),
        s.frame_png(b),
        "quota atteint → 2e requête en erreur immédiate"
    );
}

/// `document.cookie` posé par le script → jar de session,
/// lu par le document suivant (même origine).
const PAGE_COOKIE_SET: &str = "<!doctype html><html><body><p>x</p>\
    <script>document.cookie='k=v;path=/'</script></body></html>";

/// Page écho : `document.cookie` lu → div.
const PAGE_COOKIE_READ: &str = "<!doctype html><html><body>\
    <div id=\"x\">?</div>\
    <script>document.getElementById('x').textContent=document.cookie</script>\
    </body></html>";

const PAGE_COOKIE_KV: &str = "<!doctype html><html><body><div id=\"x\">k=v</div></body></html>";

#[tokio::test]
async fn document_cookie_reaches_shared_jar() {
    use explorer_net::url::Url;

    let (mut s, t) = test_session();
    t.route("/set", 200, PAGE_COOKIE_SET);
    t.route("/read", 200, PAGE_COOKIE_READ);
    t.route("/stat", 200, PAGE_COOKIE_KV);
    s.set_js_factory(explorer_js::JscoreEngine::factory());
    let tab_a = s.open_tab();
    s.navigate(tab_a, "http://h.test/set").await.unwrap();
    // L'écriture script a rejoint le jar partagé.
    let jar = s.cookie_jar();
    let page = Url::parse("http://h.test/").unwrap();
    assert_eq!(jar.header_for(&page, false), "k=v");
    // Le document suivant le lit via `document.cookie`.
    let tab_b = s.open_tab();
    s.navigate(tab_b, "http://h.test/read").await.unwrap();
    let tab_c = s.open_tab();
    s.navigate(tab_c, "http://h.test/stat").await.unwrap();
    assert_eq!(
        s.frame_png(tab_b),
        s.frame_png(tab_c),
        "cookie relu dans le doc suivant"
    );
}

/// `Set-Cookie` d'une réponse XHR same-origin → jar partagé.
#[tokio::test]
async fn xhr_set_cookie_same_origin_captured() {
    use explorer_net::url::Url;

    let (mut s, t) = test_session();
    t.route("/", 200, PAGE_XHR);
    t.route_headers("/api", 200, "DATA", &[("set-cookie", "s=1;path=/")]);
    s.set_js_factory(explorer_js::JscoreEngine::factory());
    let id = s.open_tab();
    s.navigate(id, "http://h.test/").await.unwrap();
    let jar = s.cookie_jar();
    let u = Url::parse("http://h.test/").unwrap();
    assert_eq!(jar.header_for(&u, false), "s=1");
    // …et l'en-tête Cookie repart sur la XHR same-origin
    // suivante (cookie injecté à l'émission).
    let id2 = s.open_tab();
    s.navigate(id2, "http://h.test/").await.unwrap();
    assert_eq!(t.last_header("cookie").as_deref(), Some("s=1"));
}

// ===== JS-C77 — `fetch(mode, credentials)` bout-en-bout =====

/// `mode:'no-cors'` cross : la réponse se livre opaque sans
/// `ACAO` — promesse résolue, `res.type === 'opaque'` (en
/// `mode:'cors'` la même cible rendait `error`).
const PAGE_NO_CORS: &str = "<!doctype html><html><body>\
    <div id=\"x\">?</div>\
    <script>fetch('http://b.test/api',{mode:'no-cors'})\
    .then(function(r){document.getElementById('x').textContent=r.type})\
    .catch(function(){document.getElementById('x').textContent='ERR'})</script>\
    </body></html>";

#[tokio::test]
async fn fetch_no_cors_opaque_without_acao() {
    let (mut s, t) = test_session();
    t.route("/", 200, PAGE_NO_CORS);
    // Aucun `ACAO` — `mode:cors` rendrait `error` ; `no-cors`
    // doit résoudre `opaque`.
    t.route("/api", 200, "DATA");
    t.route(
        "/stat",
        200,
        "<!doctype html><html><body><div id=\"x\">opaque</div></body></html>",
    );
    s.set_js_factory(explorer_js::JscoreEngine::factory());
    let a = s.open_tab();
    s.navigate(a, "http://a.test/").await.unwrap();
    // `Sec-Fetch-Mode: no-cors` posé à l'émission (étape 125).
    assert_eq!(t.last_header("sec-fetch-mode").as_deref(), Some("no-cors"));
    let b = s.open_tab();
    s.navigate(b, "http://a.test/stat").await.unwrap();
    assert_eq!(s.frame_png(a), s.frame_png(b), "no-cors → type opaque");
}

/// `mode:'same-origin'` sur cible cross : rejet `TypeError`
/// avant émission — aucune requête réseau ne part.
const PAGE_SAME_ORIGIN: &str = "<!doctype html><html><body>\
    <div id=\"x\">?</div>\
    <script>fetch('http://b.test/api',{mode:'same-origin'})\
    .then(function(){document.getElementById('x').textContent='OK'})\
    .catch(function(e){document.getElementById('x').textContent=e.name})</script>\
    </body></html>";

#[tokio::test]
async fn fetch_same_origin_cross_never_emitted() {
    let (mut s, t) = test_session();
    t.route("/", 200, PAGE_SAME_ORIGIN);
    t.route("/api", 200, "DATA");
    t.route(
        "/stat",
        200,
        "<!doctype html><html><body><div id=\"x\">TypeError</div></body></html>",
    );
    s.set_js_factory(explorer_js::JscoreEngine::factory());
    let a = s.open_tab();
    s.navigate(a, "http://a.test/").await.unwrap();
    assert!(
        !t.paths().iter().any(|p| p == "/api"),
        "aucune émission attendue : rejet same-origin avant transport"
    );
    let b = s.open_tab();
    s.navigate(b, "http://a.test/stat").await.unwrap();
    assert_eq!(
        s.frame_png(a),
        s.frame_png(b),
        "same-origin cross → TypeError catch"
    );
}

/// `credentials:'include'` cross : `Set-Cookie` de la cible
/// capturé sous `include`, puis `Cookie` émis sur le fetch
/// `include` suivant (`SameSite=None;Secure` + https).
const PAGE_CREDS: &str = "<!doctype html><html><body>\
    <div id=\"x\">?</div>\
    <script>fetch('https://b.test/set',{credentials:'include'})\
    .then(function(){return fetch('https://b.test/get',{credentials:'include'})})\
    .then(function(){document.getElementById('x').textContent='OK'})\
    .catch(function(){document.getElementById('x').textContent='ERR'})</script>\
    </body></html>";

#[tokio::test]
async fn fetch_include_cross_sends_cookie() {
    let (mut s, t) = test_session();
    t.route("/", 200, PAGE_CREDS);
    // `/set` : lisible sous `include` — `ACAO` explicite +
    // `ACAC: true` requis (spec : `*` refusé sous include) —
    // + pose `k=v` (capturé sous `include`) ; `/get` : idem,
    // doit recevoir `Cookie`.
    t.route_headers(
        "/set",
        200,
        "S",
        &[
            ("access-control-allow-origin", "https://a.test"),
            ("access-control-allow-credentials", "true"),
            ("set-cookie", "k=v; SameSite=None; Secure"),
        ],
    );
    t.route_headers(
        "/get",
        200,
        "G",
        &[
            ("access-control-allow-origin", "https://a.test"),
            ("access-control-allow-credentials", "true"),
        ],
    );
    t.route(
        "/stat",
        200,
        "<!doctype html><html><body><div id=\"x\">OK</div></body></html>",
    );
    s.set_js_factory(explorer_js::JscoreEngine::factory());
    let a = s.open_tab();
    s.navigate(a, "https://a.test/").await.unwrap();
    // La chaîne `.then` émet `/get` à la livraison de `/set` —
    // une pompe de plus le draine (idem hôte réel).
    s.pump_net().await;
    let paths = t.paths();
    assert!(
        paths.iter().any(|p| p == "/get"),
        "le fetch /get enchaîné doit être émis : {paths:?}"
    );
    assert_eq!(
        t.last_header("cookie").as_deref(),
        Some("k=v"),
        "credentials:include → Cookie émis cross sous SameSite=None;Secure"
    );
    let b = s.open_tab();
    s.navigate(b, "https://a.test/stat").await.unwrap();
    assert_eq!(s.frame_png(a), s.frame_png(b), "chaîne fetch include → OK");
}

/// `credentials:'omit'` same-origin : `Cookie` absent même
/// avec un cookie en jar (l'omit gouverne émission ET capture).
const PAGE_OMIT: &str = "<!doctype html><html><body>\
    <div id=\"x\">?</div>\
    <script>fetch('/api',{credentials:'omit'})\
    .then(function(){document.getElementById('x').textContent='OK'})\
    .catch(function(){document.getElementById('x').textContent='ERR'})</script>\
    </body></html>";

#[tokio::test]
async fn fetch_omit_sends_no_cookie() {
    let (mut s, t) = test_session();
    t.route("/", 200, PAGE_OMIT);
    t.route_headers("/seed", 200, "S", &[("set-cookie", "k=v;path=/")]);
    t.route("/api", 200, "DATA");
    s.set_js_factory(explorer_js::JscoreEngine::factory());
    let seed = s.open_tab();
    s.navigate(seed, "http://h.test/seed").await.unwrap();
    let a = s.open_tab();
    s.navigate(a, "http://h.test/").await.unwrap();
    assert_eq!(
        t.last_header("cookie"),
        None,
        "credentials:omit → aucun Cookie émis malgré le jar"
    );
}

/// `localStorage` : écrit dans un document, relu dans le
/// suivant (même origine — map de session drainée/installée).
const PAGE_LS_SET: &str = "<!doctype html><html><body><p>x</p>\
    <script>localStorage.setItem('k','LS')</script></body></html>";

const PAGE_LS_READ: &str = "<!doctype html><html><body>\
    <div id=\"x\">?</div>\
    <script>document.getElementById('x').textContent=localStorage.getItem('k')</script>\
    </body></html>";

const PAGE_LS_VAL: &str = "<!doctype html><html><body><div id=\"x\">LS</div></body></html>";

#[tokio::test]
async fn localstorage_persists_across_documents() {
    let (mut s, t) = test_session();
    t.route("/set", 200, PAGE_LS_SET);
    t.route("/read", 200, PAGE_LS_READ);
    t.route("/stat", 200, PAGE_LS_VAL);
    s.set_js_factory(explorer_js::JscoreEngine::factory());
    let tab_a = s.open_tab();
    s.navigate(tab_a, "http://h.test/set").await.unwrap();
    let tab_b = s.open_tab();
    s.navigate(tab_b, "http://h.test/read").await.unwrap();
    let tab_c = s.open_tab();
    s.navigate(tab_c, "http://h.test/stat").await.unwrap();
    assert_eq!(
        s.frame_png(tab_b),
        s.frame_png(tab_c),
        "localStorage conservé entre documents same-origin"
    );
}

// ===== Étape 52 — événements v2 + tick timers (ADR-0019) =====

/// Survol suivi : `mouseover`, `mousemove` coalescé,
/// `mouseout` à la sortie — accumulateur dans le texte.
const PAGE_MOUSEV: &str = "<!doctype html><html><body>\
    <div id=\"b\">avant</div>\
    <script>var d=document.getElementById('b');\
    d.addEventListener('mouseover',function(){d.textContent=d.textContent+'O'});\
    d.addEventListener('mousemove',function(){d.textContent=d.textContent+'M'});\
    d.addEventListener('mouseout',function(){d.textContent=d.textContent+'F'})</script>\
    </body></html>";

/// Statique équivalente après `over + move + move + out`.
const PAGE_MOUSEV_AFTER: &str = "<!doctype html><html><body>\
    <div id=\"b\">avantOMMF</div></body></html>";

#[tokio::test]
async fn mouse_hover_dispatches_over_move_out() {
    let (mut s, t) = test_session();
    t.route("/ev", 200, PAGE_MOUSEV);
    t.route("/stat", 200, PAGE_MOUSEV_AFTER);
    s.set_js_factory(explorer_js::JscoreEngine::factory());
    let a = s.open_tab();
    s.navigate(a, "http://h.test/ev").await.unwrap();
    let b = s.open_tab();
    s.navigate(b, "http://h.test/stat").await.unwrap();

    // Entrée : over + move ; second move même nœud : move
    // seul ; sortie : out — ordre 'OMMF'.
    s.handle_input(a, InputEvent::MouseMove { x: 5.0, y: 5.0 })
        .await
        .unwrap();
    s.handle_input(a, InputEvent::MouseMove { x: 6.0, y: 5.0 })
        .await
        .unwrap();
    s.handle_input(a, InputEvent::MouseMove { x: 5.0, y: 400.0 })
        .await
        .unwrap();
    assert_eq!(s.frame_png(a), s.frame_png(b));
}

/// Clavier + focus : `keydown` avant édition, `input` après,
/// `keyup` au relâchement ; `blur`/`focus` à la transition.
const PAGE_KEYS: &str = "<!doctype html><html><body>\
    <input id=\"a\"><br><input id=\"c\"><div id=\"r\">-</div>\
    <script>var r=document.getElementById('r');r.textContent='';\
    var a=document.getElementById('a');var c=document.getElementById('c');\
    a.addEventListener('focus',function(){r.textContent=r.textContent+'F'});\
    a.addEventListener('blur',function(){r.textContent=r.textContent+'B'});\
    a.addEventListener('keydown',function(){r.textContent=r.textContent+'D'});\
    a.addEventListener('input',function(){r.textContent=r.textContent+'I'});\
    a.addEventListener('keyup',function(){r.textContent=r.textContent+'U'});\
    c.addEventListener('focus',function(){r.textContent=r.textContent+'G'})</script>\
    </body></html>";

/// Statique équivalente après `focus + down/input + up +
/// blur/focus` : 'FDIUBG' + la valeur 'x' tapée (peinte
/// depuis R-21 — le `value` du contrôle est rendu).
const PAGE_KEYS_AFTER: &str = "<!doctype html><html><body>\
    <input id=\"a\" value=\"x\"><br><input id=\"c\">\
    <div id=\"r\">FDIUBG</div></body></html>";

#[tokio::test]
async fn key_and_focus_events_dispatch_in_order() {
    let (mut s, t) = test_session();
    t.route("/ev", 200, PAGE_KEYS);
    t.route("/stat", 200, PAGE_KEYS_AFTER);
    s.set_js_factory(explorer_js::JscoreEngine::factory());
    let a = s.open_tab();
    s.navigate(a, "http://h.test/ev").await.unwrap();
    let b = s.open_tab();
    s.navigate(b, "http://h.test/stat").await.unwrap();

    // Focus sur `a` → 'F'.
    let out = s
        .handle_input(a, InputEvent::Click { x: 5.0, y: 5.0 })
        .await
        .unwrap();
    assert_eq!(out, InputOutcome::Focused);
    // 'x' → keydown 'D' puis édition + input 'I'.
    let out = s
        .handle_input(
            a,
            InputEvent::Key {
                key: "x".into(),
                mods: KeyMods::NONE,
            },
        )
        .await
        .unwrap();
    assert_eq!(out, InputOutcome::Edited);
    // Relâchement → keyup 'U'.
    s.handle_input(
        a,
        InputEvent::KeyUp {
            key: "x".into(),
            mods: KeyMods::NONE,
        },
    )
    .await
    .unwrap();
    // Clic sur `c` → blur 'B' sur `a`, focus 'G' sur `c`.
    let out = s
        .handle_input(a, InputEvent::Click { x: 5.0, y: 35.0 })
        .await
        .unwrap();
    assert_eq!(out, InputOutcome::Focused);
    // M-H1 #1 : `a` peint le caret de `c` focalisé — le
    // défocaliser (Escape) le retire pour rester
    // pixel-comparable à la page statique sans focus.
    let out = s
        .handle_input(
            a,
            InputEvent::Key {
                key: "Escape".into(),
                mods: KeyMods::NONE,
            },
        )
        .await
        .unwrap();
    assert_eq!(out, InputOutcome::Unfocused);
    assert_eq!(s.frame_png(a), s.frame_png(b));
}

/// `wheel` (annulable) cible le survol/document, `scroll`
/// niveau document suit le défilement effectif.
const PAGE_SCROLL: &str = "<!doctype html><html><body>\
    <div style=\"height:500px\"></div><div id=\"r\">avant</div>\
    <div style=\"height:1500px\"></div>\
    <script>var r=document.getElementById('r');var b=document.body;\
    b.addEventListener('wheel',function(){r.textContent=r.textContent+'W'});\
    b.addEventListener('scroll',function(){r.textContent=r.textContent+'S'})</script>\
    </body></html>";

/// Équivalente au DOM synchronisé **scrollé** : `scroll_y`
/// est cuit dans la display list — le frame de `a` montre le
/// doc décalé de 100px (`#r` à frame-400). Contrôle :
/// `#r` à doc-400, doc-height identique 2019.
const PAGE_SCROLL_AFTER: &str = "<html><body>\
    <div style=\"height:400px\"></div><div id=\"r\">avantWS</div>\
    <div style=\"height:1600px\"></div></body></html>";

#[tokio::test]
async fn scroll_dispatches_wheel_then_scroll() {
    let (mut sess, tr) = test_session();
    tr.route("/ev", 200, PAGE_SCROLL);
    tr.route("/stat", 200, PAGE_SCROLL_AFTER);
    sess.set_js_factory(explorer_js::JscoreEngine::factory());
    let ta = sess.open_tab();
    sess.navigate(ta, "http://h.test/ev").await.unwrap();
    let tb = sess.open_tab();
    sess.navigate(tb, "http://h.test/stat").await.unwrap();

    let out = sess
        .handle_input(ta, InputEvent::Scroll { dy: 100.0 })
        .await
        .unwrap();
    match out {
        InputOutcome::Scrolled { scroll_y } => {
            assert!((scroll_y - 100.0).abs() < 0.01);
        }
        o => panic!("inattendu : {o:?}"),
    }
    assert_eq!(sess.frame_png(ta), sess.frame_png(tb));
}

/// `wheel` `preventDefault` : le scroll est supprimé, l'événe-
/// ment `scroll` n'est pas émis.
const PAGE_WHEEL_PREV: &str = "<!doctype html><html><body>\
    <div style=\"height:500px\"></div><div id=\"r\">avant</div>\
    <div style=\"height:1500px\"></div>\
    <script>var r=document.getElementById('r');var b=document.body;\
    b.addEventListener('wheel',function(e){e.preventDefault();\
    r.textContent=r.textContent+'W'});\
    b.addEventListener('scroll',function(){r.textContent=r.textContent+'S'})</script>\
    </body></html>";

/// Équivalente au DOM synchronisé : 'W' émis, page non
/// scrollée, même doc-height.
const PAGE_WHEEL_PREV_AFTER: &str = "<html><body>\
    <div style=\"height:500px\"></div><div id=\"r\">avantW</div>\
    <div style=\"height:1500px\"></div></body></html>";

#[tokio::test]
async fn wheel_prevented_suppresses_scroll() {
    let (mut sess, tr) = test_session();
    tr.route("/ev", 200, PAGE_WHEEL_PREV);
    tr.route("/stat", 200, PAGE_WHEEL_PREV_AFTER);
    sess.set_js_factory(explorer_js::JscoreEngine::factory());
    let ta = sess.open_tab();
    sess.navigate(ta, "http://h.test/ev").await.unwrap();
    let tb = sess.open_tab();
    sess.navigate(tb, "http://h.test/stat").await.unwrap();

    let out = sess
        .handle_input(ta, InputEvent::Scroll { dy: 100.0 })
        .await
        .unwrap();
    match out {
        // `preventDefault` a bloqué le scroll — y reste à 0.
        InputOutcome::Scrolled { scroll_y } => assert!(scroll_y.abs() < 0.01),
        o => panic!("inattendu : {o:?}"),
    }
    assert_eq!(sess.frame_png(ta), sess.frame_png(tb));
}

/// `setTimeout`/`setInterval`/`requestAnimationFrame` tirent
/// par la pompe, sans input — l'échéance est exposée à
/// l'hôte (`next_timer_deadline`).
const PAGE_PUMP: &str = "<!doctype html><html><body>\
    <div id=\"r\">avant</div>\
    <script>var r=document.getElementById('r');\
    setTimeout(function(){r.textContent=r.textContent+'T'},0);\
    var i=setInterval(function(){r.textContent=r.textContent+'I';\
    clearInterval(i)},0);\
    requestAnimationFrame(function(){r.textContent=r.textContent+'R'})</script>\
    </body></html>";

/// Statique équivalente après `T + I + R`.
const PAGE_PUMP_AFTER: &str = "<!doctype html><html><body>\
    <div id=\"r\">avantTIR</div></body></html>";

#[tokio::test]
async fn timers_fire_via_pump_without_input() {
    let (mut s, t) = test_session();
    t.route("/ev", 200, PAGE_PUMP);
    t.route("/stat", 200, PAGE_PUMP_AFTER);
    s.set_js_factory(explorer_js::JscoreEngine::factory());
    let a = s.open_tab();
    s.navigate(a, "http://h.test/ev").await.unwrap();
    let b = s.open_tab();
    s.navigate(b, "http://h.test/stat").await.unwrap();

    // Les callbacks à délai 0 tirent dès le `run_pending` de
    // fin de navigation ; sous charge parallèle le rAF
    // (~16 ms) peut aussi être échu. L'assertion porte sur
    // l'état final — pomper sans input jusqu'à épuisement
    // des timers (borne : un timer lent n'est pas un bug).
    for _ in 0..10 {
        s.pump_timers(a);
        if s.next_timer_deadline().is_none() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    // Une dernière pompe synchronise le miroir → le rendu.
    s.pump_timers(a);
    assert_eq!(s.frame_png(a), s.frame_png(b));
    // Timeout consommé, intervalle auto-annulé, rAF tiré.
    assert_eq!(s.next_timer_deadline(), None);
}

/// `max_timer_jobs` (ADR-0019) : borne par pompe — les
/// timers restants attendent la pompe suivante.
const PAGE_BOUND: &str = "<!doctype html><html><body>\
    <div id=\"r\">avant</div>\
    <script>var r=document.getElementById('r');\
    setTimeout(function(){r.textContent=r.textContent+'A'},0);\
    setTimeout(function(){r.textContent=r.textContent+'B'},0);\
    setTimeout(function(){r.textContent=r.textContent+'C'},0)</script>\
    </body></html>";

/// Statique équivalente après les trois callbacks.
const PAGE_BOUND_AFTER: &str = "<!doctype html><html><body>\
    <div id=\"r\">avantABC</div></body></html>";

#[tokio::test]
async fn timer_jobs_bounded_per_pump() {
    let t = Arc::new(FakeTransport::new());
    t.route("/ev", 200, PAGE_BOUND);
    t.route("/stat", 200, PAGE_BOUND_AFTER);
    let mut s = Session::with_config(
        t.clone(),
        Arc::new(FixtureFont),
        SessionConfig {
            max_timer_jobs: 1,
            ..SessionConfig::default()
        },
    );
    s.set_js_factory(explorer_js::JscoreEngine::factory());
    let a = s.open_tab();
    s.navigate(a, "http://h.test/ev").await.unwrap();
    let b = s.open_tab();
    s.navigate(b, "http://h.test/stat").await.unwrap();

    // Un callback par pompe — borne respectée ; 'ABC' au
    // bout de trois pompes.
    for _ in 0..3 {
        s.pump_timers(a);
    }
    assert_eq!(s.frame_png(a), s.frame_png(b));
    assert_eq!(s.next_timer_deadline(), None);
}

/// `MutationObserver` (JS-C26) livré au même point de
/// synchro que les timers : mutation faite par un callback
/// `setTimeout`, record livré dans la même pompe.
const PAGE_MUT_OBS: &str = "<!doctype html><html><body>\
    <div id=\"t\">n0</div><div id=\"r\">avant</div>\
    <script>var r=document.getElementById('r');\
    var t=document.getElementById('t');\
    var o=new MutationObserver(function(recs){\
    r.textContent='OBS'+recs.length});\
    o.observe(t,{childList:true});\
    setTimeout(function(){t.textContent='n1'},0)</script>\
    </body></html>";

/// Statique équivalente : `t` muté + record livré.
const PAGE_MUT_OBS_AFTER: &str = "<!doctype html><html><body>\
    <div id=\"t\">n1</div><div id=\"r\">OBS1</div></body></html>";

#[tokio::test]
async fn mutation_observer_delivered_at_pump_checkpoint() {
    let (mut s, t) = test_session();
    t.route("/ev", 200, PAGE_MUT_OBS);
    t.route("/stat", 200, PAGE_MUT_OBS_AFTER);
    s.set_js_factory(explorer_js::JscoreEngine::factory());
    let a = s.open_tab();
    s.navigate(a, "http://h.test/ev").await.unwrap();
    let b = s.open_tab();
    s.navigate(b, "http://h.test/stat").await.unwrap();

    s.pump_timers(a);
    assert_eq!(s.frame_png(a), s.frame_png(b));
}

/// Checkbox : activation avant `click` (le handler lit
/// `checked` post-bascule), puis `input` et `change` dans
/// l'ordre DOM.
const PAGE_CHECK: &str = "<!doctype html><html><body>\
    <input id=\"k\" type=\"checkbox\"><div id=\"r\">avant</div>\
    <script>var r=document.getElementById('r');\
    var k=document.getElementById('k');\
    k.addEventListener('click',function(){\
    r.textContent=r.textContent+'C'+(k.checked?'1':'0')});\
    k.addEventListener('input',function(){r.textContent=r.textContent+'I'});\
    k.addEventListener('change',function(){r.textContent=r.textContent+'N'})</script>\
    </body></html>";

/// Équivalente au DOM synchronisé : case cochée
/// (`checked=""` sérialisé par le miroir) + 'C1' 'I' 'N'.
const PAGE_CHECK_AFTER: &str = "<html><body>\
    <input id=\"k\" type=\"checkbox\" checked=\"\"><div id=\"r\">avantC1IN</div>\
    </body></html>";

#[tokio::test]
async fn checkbox_click_dispatches_input_then_change() {
    let (mut s, t) = test_session();
    t.route("/ev", 200, PAGE_CHECK);
    t.route("/stat", 200, PAGE_CHECK_AFTER);
    s.set_js_factory(explorer_js::JscoreEngine::factory());
    let a = s.open_tab();
    s.navigate(a, "http://h.test/ev").await.unwrap();
    let b = s.open_tab();
    s.navigate(b, "http://h.test/stat").await.unwrap();

    let out = s
        .handle_input(a, InputEvent::Click { x: 5.0, y: 5.0 })
        .await
        .unwrap();
    assert_eq!(out, InputOutcome::Edited);
    assert_eq!(s.frame_png(a), s.frame_png(b));
}

/// `preventDefault` sur le `click` d'une case : la bascule
/// est restaurée, `input`/`change` ne partent pas.
const PAGE_CHECK_PREV: &str = "<!doctype html><html><body>\
    <input id=\"k\" type=\"checkbox\"><div id=\"r\">avant</div>\
    <script>var r=document.getElementById('r');\
    var k=document.getElementById('k');\
    k.addEventListener('click',function(e){e.preventDefault();\
    r.textContent=r.textContent+'C'+(k.checked?'1':'0')});\
    k.addEventListener('change',function(){r.textContent=r.textContent+'N'})</script>\
    </body></html>";

/// Équivalente au DOM synchronisé : 'C1' (bascule faite
/// puis restaurée — case décochée), pas de 'N'.
const PAGE_CHECK_PREV_AFTER: &str = "<html><body>\
    <input id=\"k\" type=\"checkbox\"><div id=\"r\">avantC1</div>\
    </body></html>";

#[tokio::test]
async fn checkbox_prevented_click_restores_state() {
    let (mut s, t) = test_session();
    t.route("/ev", 200, PAGE_CHECK_PREV);
    t.route("/stat", 200, PAGE_CHECK_PREV_AFTER);
    s.set_js_factory(explorer_js::JscoreEngine::factory());
    let a = s.open_tab();
    s.navigate(a, "http://h.test/ev").await.unwrap();
    let b = s.open_tab();
    s.navigate(b, "http://h.test/stat").await.unwrap();

    let out = s
        .handle_input(a, InputEvent::Click { x: 5.0, y: 5.0 })
        .await
        .unwrap();
    assert_eq!(out, InputOutcome::Ignored);
    assert_eq!(s.frame_png(a), s.frame_png(b));
}

// ===== Étape 53 — session incrémentale (peinture pendant le
// téléchargement) =====

/// Session à cadence de peinture réglable — `usize::MAX`/`u64::MAX`
/// désactive chaque seuil indépendamment.
fn paint_session(chunk_bytes: usize, interval_ms: u64) -> (Session, Arc<FakeTransport>) {
    let t = Arc::new(FakeTransport::new());
    let s = Session::with_config(
        t.clone(),
        Arc::new(FixtureFont),
        SessionConfig {
            paint_chunk_bytes: chunk_bytes,
            paint_interval_ms: interval_ms,
            ..SessionConfig::default()
        },
    );
    (s, t)
}

/// `FrameInvalidated` reçus sur l'abonnement (drain non
/// bloquant — les événements sont déjà tous émis).
fn invalidated(rx: &mut tokio::sync::broadcast::Receiver<CoreEvent>) -> usize {
    let mut n = 0;
    while let Ok(ev) = rx.try_recv() {
        if matches!(ev, CoreEvent::FrameInvalidated { .. }) {
            n += 1;
        }
    }
    n
}

/// Page assez grande pour plusieurs peintures (> fenêtre de
/// sniffing de 1024 o, sinon tout reste tamponné chez le
/// décodeur) : le contenu croît au fil des morceaux.
fn page_stream() -> String {
    use std::fmt::Write as _;
    let mut p = String::from("<!doctype html><html><body>");
    for i in 0..40 {
        let _ = write!(
            p,
            "<div style=\"height:24px\">ligne {i} — contenu progressif</div>"
        );
    }
    p.push_str("</body></html>");
    p
}

/// Découpe `s` en morceaux de `n` octets (frontières de chunk
/// arbitraires — y compris au milieu d'une balise).
fn byte_parts(s: &str, n: usize) -> Vec<Vec<u8>> {
    s.as_bytes().chunks(n).map(<[u8]>::to_vec).collect()
}

#[tokio::test]
async fn progressive_paint_per_chunk() {
    let (mut s, t) = paint_session(1, 0);
    let mut rx = s.subscribe();
    let parts = byte_parts(&page_stream(), 512);
    t.route_chunks("/ev", 200, parts);
    let a = s.open_tab();
    s.navigate(a, "http://h.test/ev").await.unwrap();

    // Cadence « chaque morceau » : des peintures intermédiaires
    // s'ajoutent à l'invalidation finale (les tous premiers
    // chunks ne peignent pas — l'arbre n'a pas encore de
    // contenu, et la fenêtre de sniffing tamponne le début).
    assert!(invalidated(&mut rx) > 1, "peintures progressives attendues");
}

#[tokio::test]
async fn progressive_paint_cadence_respected() {
    // Seuils hors de portée → zéro peinture intermédiaire ;
    // seule l'invalidation finale subsiste.
    let (mut s, t) = paint_session(usize::MAX, u64::MAX);
    let mut rx = s.subscribe();
    let parts = byte_parts(&page_stream(), 512);
    t.route_chunks("/ev", 200, parts);
    let a = s.open_tab();
    s.navigate(a, "http://h.test/ev").await.unwrap();
    assert_eq!(invalidated(&mut rx), 1);
}

#[tokio::test]
async fn progressive_final_equals_one_shot() {
    // Même document, livré en morceaux ou en un coup → même
    // frame finale.
    let (mut s, t) = paint_session(usize::MAX, u64::MAX);
    let page = page_stream();
    // `route_bin` direct : la page servie en un coup doit
    // être *identique* aux morceaux (route() neutralise la
    // marge body des fixtures interactives — hors propos ici).
    t.route_bin("/one", 200, page.clone().into_bytes());
    t.route_chunks("/many", 200, byte_parts(&page, 137));
    let a = s.open_tab();
    s.navigate(a, "http://h.test/one").await.unwrap();
    let b = s.open_tab();
    s.navigate(b, "http://h.test/many").await.unwrap();
    assert_eq!(s.frame_png(a), s.frame_png(b));
}

#[tokio::test]
async fn progressive_utf8_boundary_no_replacement() {
    // Document UTF-8 (BOM) dont `é` (C3 A9) est coupé en
    // frontière de morceau : la séquence est conservée en
    // queue plutôt que perdue en U+FFFD — la frame égale
    // celle de la livraison en un coup.
    let (mut s, t) = paint_session(usize::MAX, u64::MAX);
    let mut page = b"\xEF\xBB\xBF<!doctype html><html><body><p>caf".to_vec();
    page.extend_from_slice(&[0xC3, 0xA9]);
    page.extend_from_slice(b"</p></body></html>");
    // Le découpage coupe pile au milieu du C3 A9.
    let mut parts: Vec<Vec<u8>> = page[..page.len() - 16]
        .chunks(11)
        .map(<[u8]>::to_vec)
        .collect();
    parts.extend(page[page.len() - 16..].chunks(3).map(<[u8]>::to_vec));
    t.route_chunks("/split", 200, parts);
    let b = s.open_tab();
    t.route_bin("/whole", 200, page);
    s.navigate(b, "http://h.test/whole").await.unwrap();
    let a = s.open_tab();
    s.navigate(a, "http://h.test/split").await.unwrap();
    assert_eq!(s.frame_png(a), s.frame_png(b));
}

#[tokio::test]
async fn progressive_meta_sniff_windows1252() {
    // `<meta charset>` dans la fenêtre de sniffing puis octet
    // 0xE9 en 1252 → « é » — le décodage décide après la
    // fenêtre, le texte suivant en profite.
    let (mut s, t) = paint_session(usize::MAX, u64::MAX);
    let mut page = b"<!doctype html><html><head>\
        <meta charset=\"windows-1252\"></head><body><p>caf"
        .to_vec();
    page.push(0xE9);
    page.extend_from_slice(b"</p></body></html>");
    let parts: Vec<Vec<u8>> = page.chunks(13).map(<[u8]>::to_vec).collect();
    t.route_chunks("/sniff", 200, parts);
    let a = s.open_tab();
    s.navigate(a, "http://h.test/sniff").await.unwrap();
    let b = s.open_tab();
    // Référence en vrais octets 1252 (0xE9 = « é ») — un `\u{E9}`
    // dans la source serait deux octets UTF-8, sniffés « Ã© ».
    let mut refp = b"<!doctype html><html><head>\
        <meta charset=\"windows-1252\"></head><body><p>caf"
        .to_vec();
    refp.push(0xE9);
    refp.extend_from_slice(b"</p></body></html>");
    t.route_bin("/ref", 200, refp);
    s.navigate(b, "http://h.test/ref").await.unwrap();
    assert_eq!(s.frame_png(a), s.frame_png(b));
}

#[tokio::test]
async fn progressive_stream_error_is_typed() {
    // Corps non-2xx streamé → `HttpStatus` comme le chemin
    // bufferisé (le canal est abandonné, la tâche se termine).
    let (mut s, t) = paint_session(1, 0);
    t.route_chunks("/err", 404, vec![b"nope".to_vec()]);
    let a = s.open_tab();
    let err = s.navigate(a, "http://h.test/err").await.unwrap_err();
    assert!(matches!(err, CoreError::HttpStatus(404)));
}

// ===== Étape 53b — charset des sous-ressources =====

/// Script externe UTF-8 sans `charset` : les littéraux
/// multi-octets restent intacts — le sniffer HTML (repli
/// Latin-1) ne s'applique plus aux sous-ressources
/// (mojibake « Lâ€™usage » constaté sur example.com).
#[tokio::test]
async fn script_utf8_literals_not_mojibake() {
    let (mut s, t) = test_session();
    t.route(
        "/p",
        200,
        "<!doctype html><html><body><div id=\"x\">avant</div>\
         <script src=\"/s.js\"></script></body></html>",
    );
    // « L'usage » — apostrophe U+2019 codée E2 80 99 : sous
    // l'ancien décodage elle devenait « â€™ ».
    t.route_bin(
        "/s.js",
        200,
        b"document.getElementById('x').textContent='L\xE2\x80\x99usage'".to_vec(),
    );
    t.route(
        "/stat",
        200,
        "<!doctype html><html><head><meta charset=\"utf-8\"></head>\
         <body><div id=\"x\">L\u{2019}usage</div></body></html>",
    );
    s.set_js_factory(explorer_js::JscoreEngine::factory());

    let a = s.open_tab();
    s.navigate(a, "http://h.test/p").await.unwrap();
    let b = s.open_tab();
    s.navigate(b, "http://h.test/stat").await.unwrap();
    assert_eq!(s.frame_png(a), s.frame_png(b));
}

/// `<script charset>` honoré : l'octet 0xE9 se décode « é »
/// sous `iso-8859-1` — le label déclaré l'emporte sur UTF-8.
#[tokio::test]
async fn script_charset_attr_decodes_latin1() {
    let (mut s, t) = test_session();
    t.route(
        "/p",
        200,
        "<!doctype html><html><body><div id=\"x\">avant</div>\
         <script src=\"/s.js\" charset=\"iso-8859-1\"></script></body></html>",
    );
    t.route_bin(
        "/s.js",
        200,
        b"document.getElementById('x').textContent='caf\xE9'".to_vec(),
    );
    t.route(
        "/stat",
        200,
        "<!doctype html><html><head><meta charset=\"utf-8\"></head>\
         <body><div id=\"x\">caf\u{E9}</div></body></html>",
    );
    s.set_js_factory(explorer_js::JscoreEngine::factory());

    let a = s.open_tab();
    s.navigate(a, "http://h.test/p").await.unwrap();
    let b = s.open_tab();
    s.navigate(b, "http://h.test/stat").await.unwrap();
    assert_eq!(s.frame_png(a), s.frame_png(b));
}

/// Module externe : toujours UTF-8 par spec — même un
/// `charset` déclaré sur la balise est ignoré.
#[tokio::test]
async fn module_script_utf8_despite_charset_attr() {
    let (mut s, t) = test_session();
    t.route(
        "/p",
        200,
        "<!doctype html><html><body><div id=\"x\">avant</div>\
         <script type=\"module\" src=\"/m.js\" charset=\"iso-8859-1\"></script></body></html>",
    );
    t.route_bin(
        "/m.js",
        200,
        b"document.getElementById('x').textContent='caf\xC3\xA9'".to_vec(),
    );
    t.route(
        "/stat",
        200,
        "<!doctype html><html><head><meta charset=\"utf-8\"></head>\
         <body><div id=\"x\">caf\u{E9}</div></body></html>",
    );
    s.set_js_factory(explorer_js::JscoreEngine::factory());

    let a = s.open_tab();
    s.navigate(a, "http://h.test/p").await.unwrap();
    let b = s.open_tab();
    s.navigate(b, "http://h.test/stat").await.unwrap();
    assert_eq!(s.frame_png(a), s.frame_png(b));
}

// ------------------------------------------------------------------
// Étape 54 — validation de contraintes + multipart +
// `form.submit()` script
// ------------------------------------------------------------------

/// Champ `required` vide + submit.
const PAGE_REQUIRED_FORM: &str = "<html><body><form action=\"/vgo\">\
    <div><input name=\"q\" required></div>\
    <div><input type=\"submit\" value=\"Go\"></div></form></body></html>";

/// `novalidate` saute la validation.
const PAGE_NOVALIDATE_FORM: &str = "<html><body><form action=\"/vgo\" novalidate>\
    <div><input name=\"q\" required></div>\
    <div><input type=\"submit\" value=\"Go\"></div></form></body></html>";

/// `minlength` non satisfait.
const PAGE_MINLENGTH_FORM: &str = "<html><body><form action=\"/vgo\">\
    <div><input name=\"q\" value=\"x\" minlength=\"3\"></div>\
    <div><input type=\"submit\" value=\"Go\"></div></form></body></html>";

/// `pattern` non satisfait.
const PAGE_PATTERN_FORM: &str = "<html><body><form action=\"/vgo\">\
    <div><input name=\"q\" value=\"abc\" pattern=\"[0-9]+\"></div>\
    <div><input type=\"submit\" value=\"Go\"></div></form></body></html>";

/// `type=number` hors `max`.
const PAGE_RANGE_FORM: &str = "<html><body><form action=\"/vgo\">\
    <div><input type=\"number\" name=\"q\" value=\"99\" min=\"1\" max=\"10\"></div>\
    <div><input type=\"submit\" value=\"Go\"></div></form></body></html>";

/// `step` non respecté.
const PAGE_STEP_FORM: &str = "<html><body><form action=\"/vgo\">\
    <div><input type=\"number\" name=\"q\" value=\"3\" min=\"0\" step=\"2\"></div>\
    <div><input type=\"submit\" value=\"Go\"></div></form></body></html>";

/// `enctype=multipart/form-data` + champ fichier.
const PAGE_MULTIPART_FORM: &str = "<html><body><form method=\"post\" action=\"/mp\"\
    enctype=\"multipart/form-data\">\
    <div><input name=\"a\" value=\"1\"></div>\
    <div><input type=\"file\" name=\"f\" value=\"x.txt\"></div>\
    <div><input type=\"submit\" value=\"Go\"></div></form></body></html>";

/// `form.submit()` au chargement — navigation script.
const PAGE_JS_SUBMIT: &str = "<html><body><form action=\"/jgo\" id=\"f\">\
    <input name=\"x\" value=\"1\"></form>\
    <script>document.getElementById('f').submit();</script></body></html>";

/// `setCustomValidity` script → attribut miroir → blocage
/// moteur à la soumission.
const PAGE_CUSTOM_VALIDITY: &str = "<html><body><form action=\"/vgo\">\
    <div><input name=\"q\" value=\"ok\"></div>\
    <div><input type=\"submit\" value=\"Go\"></div></form>\
    <script>document.querySelector('input').setCustomValidity('nope');</script>\
    </body></html>";

/// `requestSubmit()` sur un formulaire valide navigue.
const PAGE_REQUEST_SUBMIT: &str = "<html><body><form action=\"/jgo\" id=\"f\">\
    <input name=\"x\" value=\"1\"></form>\
    <script>document.getElementById('f').requestSubmit();</script></body></html>";

/// Un submit sur `page` doit rester `Ignored` sans requête
/// réseau supplémentaire (validation bloquée).
async fn assert_submit_blocked(page: &'static str) {
    let (mut s, t) = test_session();
    t.route("/f", 200, page);
    t.route("/vgo", 200, "<html><body>done</body></html>");
    let id = s.open_tab();
    s.navigate(id, "http://h.test/f").await.unwrap();
    let calls = t.calls();
    let out = s
        .handle_input(id, InputEvent::Click { x: 6.0, y: 40.0 })
        .await
        .unwrap();
    assert_eq!(out, InputOutcome::Ignored);
    assert_eq!(t.calls(), calls);
    assert_eq!(
        s.tab_info(id).unwrap().url.as_deref(),
        Some("http://h.test/f")
    );
}

#[tokio::test]
async fn required_field_blocks_submission() {
    assert_submit_blocked(PAGE_REQUIRED_FORM).await;
}

#[tokio::test]
async fn minlength_violation_blocks_submission() {
    assert_submit_blocked(PAGE_MINLENGTH_FORM).await;
}

#[tokio::test]
async fn pattern_violation_blocks_submission() {
    assert_submit_blocked(PAGE_PATTERN_FORM).await;
}

#[tokio::test]
async fn number_out_of_range_blocks_submission() {
    assert_submit_blocked(PAGE_RANGE_FORM).await;
}

#[tokio::test]
async fn step_violation_blocks_submission() {
    assert_submit_blocked(PAGE_STEP_FORM).await;
}

/// `required` rempli → `Enter` soumet (soumission implicite,
/// validation passée — le bouton par défaut est `submitter`).
#[tokio::test]
async fn valid_form_submits_on_enter() {
    let (mut s, t) = test_session();
    t.route("/f", 200, PAGE_REQUIRED_FORM);
    t.route("/vgo", 200, "<html><body>done</body></html>");
    let id = s.open_tab();
    s.navigate(id, "http://h.test/f").await.unwrap();
    // Focus le champ puis tape `hi` — chaque touche = `input`.
    s.handle_input(id, InputEvent::Click { x: 6.0, y: 6.0 })
        .await
        .unwrap();
    for k in ["h", "i"] {
        s.handle_input(
            id,
            InputEvent::Key {
                key: k.to_string(),
                mods: KeyMods::NONE,
            },
        )
        .await
        .unwrap();
    }
    let out = s
        .handle_input(
            id,
            InputEvent::Key {
                key: "Enter".to_string(),
                mods: KeyMods::NONE,
            },
        )
        .await
        .unwrap();
    assert_eq!(out, InputOutcome::Navigated);
    assert_eq!(t.last().unwrap().1, "/vgo?q=hi");
}

/// `novalidate` : le formulaire `required` vide part quand même.
#[tokio::test]
async fn novalidate_form_submits() {
    let (mut s, t) = test_session();
    t.route("/f", 200, PAGE_NOVALIDATE_FORM);
    t.route("/vgo", 200, "<html><body>done</body></html>");
    let id = s.open_tab();
    s.navigate(id, "http://h.test/f").await.unwrap();
    let out = s
        .handle_input(id, InputEvent::Click { x: 6.0, y: 40.0 })
        .await
        .unwrap();
    assert_eq!(out, InputOutcome::Navigated);
    assert_eq!(t.last().unwrap().1, "/vgo?q=");
}

/// `setCustomValidity` côté script bloque la soumission —
/// l'attribut `data-explorer-cv` a transité miroir→DOM.
#[tokio::test]
async fn custom_validity_blocks_submission() {
    let (mut s, t) = test_session();
    s.set_js_factory(explorer_js::JscoreEngine::factory());
    t.route("/f", 200, PAGE_CUSTOM_VALIDITY);
    t.route("/vgo", 200, "<html><body>done</body></html>");
    let id = s.open_tab();
    s.navigate(id, "http://h.test/f").await.unwrap();
    let calls = t.calls();
    let out = s
        .handle_input(id, InputEvent::Click { x: 6.0, y: 40.0 })
        .await
        .unwrap();
    assert_eq!(out, InputOutcome::Ignored);
    assert_eq!(t.calls(), calls);
}

/// `multipart/form-data` : parties `Content-Disposition`,
/// fichier vide avec `filename`, frontière refermée.
#[tokio::test]
async fn multipart_post_sends_parts() {
    let (mut s, t) = test_session();
    t.route("/f", 200, PAGE_MULTIPART_FORM);
    t.route("/mp", 200, "<html><body>done</body></html>");
    let id = s.open_tab();
    s.navigate(id, "http://h.test/f").await.unwrap();
    let out = s
        .handle_input(id, InputEvent::Click { x: 6.0, y: 60.0 })
        .await
        .unwrap();
    assert_eq!(out, InputOutcome::Navigated);
    let (m, pq, body) = t.last().unwrap();
    assert_eq!(m, "Post");
    assert_eq!(pq, "/mp");
    let ct = t.last_header("content-type").unwrap();
    assert!(ct.starts_with("multipart/form-data; boundary=----ExplorerFormBoundary"));
    let text = String::from_utf8(body).unwrap();
    assert!(text.contains("Content-Disposition: form-data; name=\"a\"\r\n\r\n1\r\n"));
    assert!(text.contains(
        "Content-Disposition: form-data; name=\"f\"; filename=\"x.txt\"\r\n\
         Content-Type: application/octet-stream\r\n\r\n"
    ));
    assert!(text.trim_end().ends_with("--"));
}

/// `form.submit()` au chargement → `NavRequest::FormSubmit`
/// drainée → navigation GET avec les champs collectés.
#[tokio::test]
async fn js_form_submit_navigates() {
    let (mut s, t) = test_session();
    s.set_js_factory(explorer_js::JscoreEngine::factory());
    t.route("/f", 200, PAGE_JS_SUBMIT);
    t.route("/jgo", 200, "<html><body>done</body></html>");
    let id = s.open_tab();
    s.navigate(id, "http://h.test/f").await.unwrap();
    s.drain_script_nav(id).await;
    assert_eq!(
        s.tab_info(id).unwrap().url.as_deref(),
        Some("http://h.test/jgo?x=1")
    );
}

/// `requestSubmit()` sur un formulaire valide soumet aussi.
#[tokio::test]
async fn js_request_submit_navigates() {
    let (mut s, t) = test_session();
    s.set_js_factory(explorer_js::JscoreEngine::factory());
    t.route("/f", 200, PAGE_REQUEST_SUBMIT);
    t.route("/jgo", 200, "<html><body>done</body></html>");
    let id = s.open_tab();
    s.navigate(id, "http://h.test/f").await.unwrap();
    s.drain_script_nav(id).await;
    assert_eq!(
        s.tab_info(id).unwrap().url.as_deref(),
        Some("http://h.test/jgo?x=1")
    );
}

/// Document plus haut que le cap raster (16 384 px) : la
/// navigation réussit et la frame couvre le sommet du
/// document plutôt que d'échouer en `FrameTooBig`.
#[tokio::test]
async fn giant_document_clamps_frame_instead_of_error() {
    let (mut s, t) = test_session();
    t.route(
        "/",
        200,
        "<!doctype html><html><body>\
         <div style=\"height:20000px\">haut</div>\
         <div>loin en bas</div></body></html>",
    );
    let id = s.open_tab();
    s.navigate(id, "http://h.test/").await.unwrap();
    let png = s.frame_png(id).expect("frame plafonnée");
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
}

/// Repro QA 52b — texte tapé dans un `<input>` : la frame
/// doit refléter la frappe (le run texte suit la valeur,
/// pas le layout figé du chargement).
/// Fix livré côté PIPE en R-21 (`Text` sous
/// `AnonymousBlock`) — le test reste comme régression du
/// défaut « texte jamais peint ».
#[tokio::test]
async fn typed_text_repaints_new_value() {
    let (mut s, t) = test_session();
    t.route("/", 200, PAGE_FORM);
    let id = s.open_tab();
    s.navigate(id, "http://h.test/").await.unwrap();
    s.handle_input(id, InputEvent::Click { x: 5.0, y: 5.0 })
        .await
        .unwrap();
    let before = s.frame_png(id).unwrap();
    for key in ["x", "y", "z"] {
        s.handle_input(
            id,
            InputEvent::Key {
                key: key.into(),
                mods: KeyMods::NONE,
            },
        )
        .await
        .unwrap();
    }
    let after = s.frame_png(id).unwrap();
    assert_ne!(before, after, "le texte tapé doit repeindre");
}

/// Repro QA 52b (chemin daemon) : un `setTimeout` **différé**
/// (comme la pompe de fond l'appelle sans input) mute le DOM,
/// émet `FrameInvalidated`, et un `document.title` émis par
/// script produit `TitleChanged`.
const PAGE_DELAYED: &str = "<!doctype html><html><head><title>A</title></head><body>\
    <div id=\"r\">avant</div>\
    <script>document.title='B';\
    setTimeout(function(){var d=document.createElement('h2');\
    d.textContent='TIMER FIRED';document.body.appendChild(d);\
    document.title='C'},150)</script></body></html>";

const PAGE_DELAYED_AFTER: &str = "<!doctype html><html><head><title>C</title></head><body>\
    <div id=\"r\">avant</div><h2>TIMER FIRED</h2></body></html>";

#[tokio::test]
async fn delayed_timer_pumps_and_emits_events() {
    let (mut s, t) = test_session();
    t.route("/ev", 200, PAGE_DELAYED);
    t.route("/stat", 200, PAGE_DELAYED_AFTER);
    s.set_js_factory(explorer_js::JscoreEngine::factory());
    let mut rx = s.subscribe();
    let a = s.open_tab();
    s.navigate(a, "http://h.test/ev").await.unwrap();
    let b = s.open_tab();
    s.navigate(b, "http://h.test/stat").await.unwrap();

    // `document.title='B'` synchrone → TitleChanged B ; le
    // `Navigated` final annonce le titre vivant (jamais la
    // valeur périmée « A » du parse — régression 52b).
    let titles: Vec<String> = std::iter::from_fn(|| rx.try_recv().ok())
        .filter_map(|e| match e {
            CoreEvent::TitleChanged { tab, title } if tab == a => Some(title),
            _ => None,
        })
        .collect();
    assert!(!titles.is_empty(), "title_changed attendu");
    assert!(
        titles.iter().all(|t| t == "B"),
        "titre vivant attendu, jamais « A » : {titles:?}"
    );

    // La pompe de fond (≙ daemon) — sans aucun input : le
    // timer différé tire, mute le DOM, émet ses événements.
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    for _ in 0..4 {
        s.pump_timers_all();
    }

    // Mutation rendue (frame == témoin) + événements émis :
    // TitleChanged C puis FrameInvalidated.
    assert_eq!(s.frame_png(a), s.frame_png(b));
    let mut got_title = None;
    let mut got_frame = false;
    while let Ok(e) = rx.try_recv() {
        match e {
            CoreEvent::TitleChanged { tab, title } if tab == a => got_title = Some(title),
            CoreEvent::FrameInvalidated { tab, .. } if tab == a => got_frame = true,
            _ => {}
        }
    }
    assert_eq!(got_title.as_deref(), Some("C"));
    assert!(got_frame, "FrameInvalidated après timer");
}

/// Transport à goulot (étape 53d) : le premier morceau du
/// document part, puis le corps attend `released` — la
/// navigation reste « en vol » le temps de prouver que la
/// session partagée répond aux autres appels.
struct GatedTransport {
    /// Libère le dernier morceau du corps.
    released: Arc<tokio::sync::Notify>,
    /// Documents émis — le test attend le départ de la nav.
    calls: std::sync::atomic::AtomicU32,
}

#[async_trait]
impl HttpTransport for GatedTransport {
    async fn fetch(&self, _req: &Request) -> NetResult<Response> {
        // Sous-ressources : absentes — 404 rapide (la vague
        // finit vite, le test vise le gel du document).
        Ok(Response {
            status: 404,
            reason: "gated".to_string(),
            headers: vec![],
            body: b"missing".to_vec(),
            reused_connection: false,
            resolved_url: None,
        })
    }

    async fn fetch_stream(
        &self,
        _req: &Request,
    ) -> NetResult<explorer_net::http::StreamedResponse> {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let (tx, rx) = tokio::sync::mpsc::channel(2);
        let released = self.released.clone();
        tokio::spawn(async move {
            let _ = tx.send(Ok(b"<html><title>slow".to_vec())).await;
            released.notified().await;
            let _ = tx
                .send(Ok(b"</title><body>fin</body></html>".to_vec()))
                .await;
        });
        Ok(explorer_net::http::StreamedResponse {
            status: 200,
            reason: "gated".to_string(),
            headers: vec![],
            reused_connection: false,
            resolved_url: None,
            body: rx,
        })
    }
}

/// Étape 53d — le défaut produit : `Mutex<Session>` tenu
/// pendant toute la navigation gelait `/status`, `/tabs`,
/// `/frame` et l'input pendant la charge (nav > 8 min sur
/// terrain). Une session partagée doit rester serviable
/// pendant le téléchargement : le verrou s'acquiert en
/// quelques millisecondes puis la nav aboutit au signal.
#[tokio::test]
async fn shared_session_stays_responsive_during_navigation() {
    use explorer_net::url::Url;

    let released = Arc::new(tokio::sync::Notify::new());
    let t = Arc::new(GatedTransport {
        released: released.clone(),
        calls: std::sync::atomic::AtomicU32::new(0),
    });
    let sess = Arc::new(tokio::sync::Mutex::new(Session::with_config(
        t.clone(),
        Arc::new(FixtureFont),
        SessionConfig::default(),
    )));
    let tab = sess.lock().await.open_tab();
    let s2 = sess.clone();
    let nav = tokio::spawn(async move {
        Box::pin(explorer_core::navigate_shared(
            &s2,
            tab,
            Request::get(Url::parse("https://a/b").unwrap()),
            true,
        ))
        .await
    });
    // Attends que le fetch soit réellement parti.
    for _ in 0..100 {
        if t.calls.load(std::sync::atomic::Ordering::SeqCst) > 0 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(
        t.calls.load(std::sync::atomic::Ordering::SeqCst) > 0,
        "la navigation n'a jamais démarré"
    );
    // Le cœur de la régression : le verrou doit être libre
    // PENDANT que le corps est en vol — avant, toute la
    // durée du fetch bloquait les autres opérations.
    let guard = tokio::time::timeout(std::time::Duration::from_millis(500), sess.lock()).await;
    let Ok(guard) = guard else {
        panic!("session gelée pendant le téléchargement (53d)");
    };
    // `tabs`/`tab_info`/`frame` serviables sous le même lock.
    assert_eq!(guard.tabs(), vec![tab]);
    drop(guard);
    // Libère le corps → la navigation aboutit et commit.
    released.notify_waiters();
    let out = nav.await.unwrap_or_else(|e| panic!("jointure nav : {e}"));
    assert!(out.is_ok(), "nav en échec : {out:?}");
    let s = sess.lock().await;
    assert_eq!(
        s.tab_info(tab).and_then(|i| i.title),
        Some("slow".to_string())
    );
}

/// Dirty-rects transportés (étape 55, contrat R-7
/// `changed_rects`) : une mutation localisée produit des
/// régions bornées dans `FrameInvalidated`, servies en
/// pull partiel (`frame_region`/`frame_png_region`/
/// `take_dirty`).
#[tokio::test]
#[allow(clippy::cast_possible_truncation)]
#[allow(clippy::cast_sign_loss)]
#[allow(clippy::cast_precision_loss)]
async fn dirty_rects_reported_and_region_served() {
    let (mut s, t) = test_session();
    t.route(
        "/ck",
        200,
        "<!doctype html><html><body><form>         <input type=checkbox></form></body></html>",
    );
    let a = s.open_tab();
    let mut rx = s.subscribe();
    s.navigate(a, "http://h.test/ck").await.unwrap();
    while rx.try_recv().is_ok() {}
    // Remise à zéro : la nav a déjà poussé « pleine frame ».
    s.take_dirty(a);

    // Clic sur la case → repaint → invalidation bornée.
    s.handle_input(a, InputEvent::Click { x: 5.0, y: 5.0 })
        .await
        .unwrap();
    let (fw, fh, _) = s.frame_pixels(a).unwrap();
    let mut rects = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        if let CoreEvent::FrameInvalidated { tab, rects: r, .. } = ev {
            if tab == a {
                rects = r;
            }
        }
    }
    assert_ne!(rects, [], "régions invalides attendues");
    for r in &rects {
        assert!(r.x >= 0.0 && r.y >= 0.0, "région {r:?}");
        assert!(
            r.x + r.w <= fw as f32 + 1.0 && r.y + r.h <= fh as f32 + 1.0,
            "région {r:?} hors frame {fw}x{fh}"
        );
    }
    // Une case à cocher ne doit pas salir toute la frame —
    // au moins une région strictement plus petite.
    assert!(
        rects.iter().any(|r| r.w < fw as f32 || r.h < fh as f32),
        "invalidation plein cadre inattendue : {rects:?}"
    );

    // Pull partiel : la région est servie en RGBA et en PNG.
    let r = rects[0];
    let (rw, rh, px) = s
        .frame_region(
            a,
            r.x as u32,
            r.y as u32,
            r.w.ceil() as u32,
            r.h.ceil() as u32,
        )
        .unwrap();
    assert_eq!(px.len(), (rw * rh * 4) as usize);
    let png = s
        .frame_png_region(
            a,
            r.x as u32,
            r.y as u32,
            r.w.ceil() as u32,
            r.h.ceil() as u32,
        )
        .unwrap();
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
    // Hors bornes → clip à vide → None.
    assert!(s.frame_region(a, fw, fh, 10, 10).is_none());

    // Canal de pull sans événements (FFI) : take_dirty
    // draine l'accumulation — second appel vide.
    assert_ne!(s.take_dirty(a), []);
    assert_eq!(s.take_dirty(a), []);
}

// ===== Étape 56 — profil persisté : cookies + bfcache =====

/// Aller-retour dans l'historique : les pages gelées du
/// bfcache sont restaurées sans aucun re-fetch — le compteur
/// du transport reste figé et la frame revient telle quelle.
#[tokio::test]
async fn bfcache_restores_back_forward_without_refetch() {
    let (mut s, t) = test_session();
    t.route("/a", 200, PAGE_T);
    t.route("/b", 200, PAGE);
    let id = s.open_tab();
    s.navigate(id, "http://h.test/a").await.unwrap();
    s.navigate(id, "http://h.test/b").await.unwrap();
    let calls = t.calls();

    // back → /a restaurée depuis le gel : zéro réseau.
    s.go_back(id).await.unwrap();
    assert_eq!(s.tab_url(id).as_deref(), Some("http://h.test/a"));
    assert_eq!(t.calls(), calls, "hit bfcache — pas de re-fetch");
    assert!(s.frame_png(id).is_some(), "frame restaurée servie");

    // forward → /b re-gelée au back : zéro réseau aussi.
    s.go_forward(id).await.unwrap();
    assert_eq!(s.tab_url(id).as_deref(), Some("http://h.test/b"));
    assert_eq!(t.calls(), calls);

    // reload reste un vrai re-fetch (bfcache ≠ rechargement).
    s.reload(id).await.unwrap();
    assert_eq!(t.calls(), calls + 1);
}

/// Terrain 23/10 : `back` restauré par bfcache puis `reload`
/// rechargeait la page **quittée** — `try_bfcache` met à jour
/// `url`/`hist_idx` mais laissait `last_req` sur la requête
/// de /b ; `reload_request` la rejouait sur l'affichage /a.
/// Le reload post-restauration doit réémettre la requête de
/// la position restaurée.
#[tokio::test]
async fn reload_after_bfcache_restore_refetches_restored_page() {
    let (mut s, t) = test_session();
    t.route("/a", 200, PAGE_T);
    t.route("/b", 200, PAGE);
    let id = s.open_tab();
    s.navigate(id, "http://h.test/a").await.unwrap();
    s.navigate(id, "http://h.test/b").await.unwrap();

    // back → /a restaurée sans réseau ; reload → re-fetch de /a.
    s.go_back(id).await.unwrap();
    assert_eq!(s.tab_url(id).as_deref(), Some("http://h.test/a"));
    s.reload(id).await.unwrap();
    assert_eq!(
        t.last().unwrap().1,
        "/a",
        "reload post-bfcache doit rejouer la requête de la page restaurée"
    );

    // Aller-retour symétrique : forward restaure /b, reload /b.
    s.go_forward(id).await.unwrap();
    s.reload(id).await.unwrap();
    assert_eq!(t.last().unwrap().1, "/b");
}

/// Le bfcache est borné (`bfcache_pages` = 4) : la page la
/// plus anciennement gelée est évincée — y revenir dégrade
/// proprement en fetch normal.
#[tokio::test]
async fn bfcache_evicts_oldest_page() {
    let (mut s, t) = test_session();
    for i in 0..6 {
        t.route(&format!("/{i}"), 200, PAGE);
    }
    let id = s.open_tab();
    for i in 0..6 {
        s.navigate(id, &format!("http://h.test/{i}")).await.unwrap();
    }
    let calls = t.calls();

    // Les 4 dernières positions sont gelées : 4 retours sans
    // réseau.
    for idx in (1..5).rev() {
        s.go_back(id).await.unwrap();
        assert_eq!(
            s.tab_url(id).as_deref(),
            Some(format!("http://h.test/{idx}").as_str())
        );
    }
    assert_eq!(t.calls(), calls, "positions bornées restaurées");

    // La position 0 a été évincée (FIFO) → re-fetch normal.
    s.go_back(id).await.unwrap();
    assert_eq!(s.tab_url(id).as_deref(), Some("http://h.test/0"));
    assert_eq!(t.calls(), calls + 1, "snapshot évincé → fetch");
}

/// Cookies persistés (étape 56) : `set_store` recharge le
/// jar depuis le store — redémarrage simulé, le cookie
/// survit ; la pose d'un cookie persistant atterrit dans le
/// store ; un cookie de session, jamais.
#[tokio::test]
async fn cookies_persist_across_restart() {
    use explorer_core::StoredCookie;
    use explorer_net::url::Url;

    // Redémarrage : le store contient déjà un cookie — le
    // nouveau jar le sert dès la première navigation.
    let store = Arc::new(MemStore {
        visits: Mutex::new(Vec::new()),
        bookmarks: Mutex::new(Vec::new()),
        cookies: Mutex::new(vec![StoredCookie {
            name: "p".to_string(),
            value: "1".to_string(),
            domain: "h.test".to_string(),
            host_only: true,
            path: "/".to_string(),
            expires_unix: Some(4_000_000_000),
            secure: false,
            http_only: false,
            same_site: "lax".to_string(),
            partition: None,
        }]),
    });
    let (mut s, t) = test_session();
    s.set_store(store.clone());
    let url = Url::parse("http://h.test/").unwrap();
    assert_eq!(s.cookie_jar().header_for(&url, true), "p=1");

    // Le serveur pose un cookie persistant + un cookie de
    // session via le transport (pose directe sur le jar
    // partagé — rôle de `CookieTransport` en production).
    s.cookie_jar()
        .set_from_header(&url, "keep=2;path=/;max-age=3600");
    s.cookie_jar().set_from_header(&url, "tmp=9;path=/");
    t.route("/", 200, PAGE);
    let id = s.open_tab();
    s.navigate(id, "http://h.test/").await.unwrap();

    // Fin de navigation = point de synchro → `save_cookies`.
    let saved = store.cookies.lock().unwrap().clone();
    let names: Vec<&str> = saved.iter().map(|c| c.name.as_str()).collect();
    assert!(names.contains(&"p"), "cookie seed re-persisté");
    assert!(names.contains(&"keep"), "cookie persistant sauvé");
    assert!(!names.contains(&"tmp"), "cookie de session non persisté");
}

/// Sémantique POST de l'historique (étape 56) : bfcache
/// désactivé, le déplacement rejoue la requête d'origine —
/// méthode + corps, pas un GET (limite « reload/back d'un
/// POST refait GET » levée).
#[tokio::test]
async fn post_history_entry_replays_request() {
    let t = Arc::new(FakeTransport::new());
    let mut s = Session::with_config(
        t.clone(),
        Arc::new(FixtureFont),
        SessionConfig {
            bfcache_pages: 0,
            ..SessionConfig::default()
        },
    );
    t.route("/f", 200, PAGE_POST_FORM);
    t.route("/doit", 200, "<html><body>done</body></html>");
    let id = s.open_tab();
    s.navigate(id, "http://h.test/f").await.unwrap();
    // Bouton submit → POST /doit (corps urlencodé).
    let out = s
        .handle_input(id, InputEvent::Click { x: 10.0, y: 183.0 })
        .await
        .unwrap();
    assert_eq!(out, InputOutcome::Navigated);
    let (m, pq, body) = t.last().unwrap();
    assert_eq!(m, "Post");
    assert_eq!(pq, "/doit");
    assert_ne!(body.len(), 0);

    // back → rejoue le GET /f.
    s.go_back(id).await.unwrap();
    assert_eq!(s.tab_url(id).as_deref(), Some("http://h.test/f"));
    assert_eq!(t.last().unwrap().0, "Get");

    // forward → rejoue le POST /doit, corps compris.
    s.go_forward(id).await.unwrap();
    let (m, pq, body) = t.last().unwrap();
    assert_eq!(m, "Post", "forward rejoue le POST historié");
    assert_eq!(pq, "/doit");
    assert_ne!(body.len(), 0, "corps urlencodé rejoué");

    // reload sur la page servie par POST → re-POST.
    s.reload(id).await.unwrap();
    assert_eq!(t.last().unwrap().0, "Post", "reload réémet le POST");
}

/// Étape 93 — `reload_request` sur une page d'erreur retente
/// la requête **tombée** (méthode + corps de l'entrée
/// `failed`) : un POST échoué se re-émet en POST, pas en
/// `GET` vide. La confirmation de resoumission est une
/// décision du client — l'API l'exige (`resubmit_confirm`),
/// la session rejoue.
#[tokio::test]
async fn reload_request_on_failed_post_replays_request() {
    use explorer_net::url::Url;
    let (s, t) = test_session();
    t.route("/doit", 500, "boom");
    let sess = std::sync::Arc::new(tokio::sync::Mutex::new(s));
    let id = sess.lock().await.open_tab();
    let mut req = Request::get(Url::parse("http://h.test/doit").unwrap());
    req.method = explorer_net::http::Method::Post;
    req.body = Some(b"a=1".to_vec());
    assert!(matches!(
        Box::pin(explorer_core::navigate_shared(&sess, id, req, true)).await,
        Err(CoreError::HttpStatus(500))
    ));
    // Page d'erreur commise : `last_req` est purgé, la
    // requête `failed` de l'entrée est la source de rejeu.
    let req = sess.lock().await.reload_request(id).unwrap();
    assert_eq!(req.method.as_str(), "POST", "retente la méthode tombée");
    assert_eq!(req.body.as_deref(), Some(b"a=1".as_slice()));
    assert_eq!(req.url.to_string(), "http://h.test/doit");
    // Le rejeu émet le POST (il retombe 500 — c'est la
    // retente qui compte, pas le résultat).
    assert!(matches!(
        Box::pin(explorer_core::navigate_shared(&sess, id, req, true)).await,
        Err(CoreError::HttpStatus(500))
    ));
    assert_eq!(t.last().unwrap().0, "Post", "le rejeu est un POST");
}

/// Étape 55b — `<svg>` inline câblé en session : tailles
/// intrinsèques (`svg_sizes` → `build_box_tree_sized`) et
/// bitmap rasterisé (`collect_svg` → `PaintImages.svgs`)
/// participent au rendu — un rect rouge SVG laisse des
/// pixels rouges dans la frame (avant : espace réservé).
#[tokio::test]
async fn inline_svg_is_rasterized_into_frame() {
    const SVG_PAGE: &str = "<!doctype html><html><body>\
         <svg width=\"40\" height=\"40\" viewBox=\"0 0 40 40\">\
         <rect x=\"0\" y=\"0\" width=\"40\" height=\"40\" fill=\"red\"/></svg>\
         </body></html>";
    let (mut s, t) = test_session();
    t.route("/", 200, SVG_PAGE);
    let id = s.open_tab();
    s.navigate(id, "http://h.test/").await.unwrap();

    let (w, h, px) = s.frame_pixels(id).unwrap();
    assert!(w > 0 && h > 0);
    let red = px
        .chunks_exact(4)
        .filter(|p| p[0] > 200 && p[1] < 60 && p[2] < 60)
        .count();
    assert!(
        red > 100,
        "le rect SVG rouge doit peindre (pixels rouges : {red})"
    );
}

/// Étape 55d — les événements rapportent le **delta**, pas
/// l'historique : deux invalidations successives produisent
/// deux événements portant chacun leurs régions propres
/// (avant : chaque émission re-sérialisait l'accumulation à
/// vie → collapse « plein cadre » au-delà de 32 repaints).
/// `take_dirty` reste le canal pull indépendant.
#[tokio::test]
async fn frame_invalidated_reports_delta_not_history() {
    let (mut s, t) = test_session();
    t.route(
        "/ck",
        200,
        "<!doctype html><html><body><form>\
         <input type=checkbox></form></body></html>",
    );
    let a = s.open_tab();
    let mut rx = s.subscribe();
    s.navigate(a, "http://h.test/ck").await.unwrap();

    let grab = |rx: &mut tokio::sync::broadcast::Receiver<CoreEvent>| {
        let mut rects = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            if let CoreEvent::FrameInvalidated { tab, rects: r, .. } = ev {
                if tab == a {
                    rects = r;
                }
            }
        }
        rects
    };

    // Premier bascule → delta #1 (la nav a déjà émis la sienne).
    s.handle_input(a, InputEvent::Click { x: 5.0, y: 5.0 })
        .await
        .unwrap();
    let r1 = grab(&mut rx);
    assert_ne!(r1, [], "delta attendu au premier clic");

    // Second bascule → delta #2 : les régions du #1 ne sont
    // PAS re-rapportées (un clic à même endroit produit le
    // même nombre de régions — l'historique cumulé serait
    // strictement plus grand).
    s.handle_input(a, InputEvent::Click { x: 5.0, y: 5.0 })
        .await
        .unwrap();
    let r2 = grab(&mut rx);
    assert_ne!(r2, [], "delta attendu au second clic");
    assert_eq!(
        r2.len(),
        r1.len(),
        "le second événement re-rapporte l'historique : {r1:?} puis {r2:?}"
    );

    // Le pull FFI est indépendant : l'accumulation complète
    // survit aux émissions.
    let pulled = s.take_dirty(a);
    assert!(
        pulled.len() >= r1.len() + r2.len(),
        "take_dirty doit accumuler indépendamment : {pulled:?}"
    );
    assert_eq!(s.take_dirty(a), []);
}

/// Étape 55f — la synchro DOM vise le sous-arbre sale :
/// un `appendChild` déclenché par événement doit passer par
/// `relayout_subtree` (compteur `incremental_syncs`) plutôt
/// qu'un reflow global, tout en produisant la même frame
/// que la page statique équivalente.
#[tokio::test]
async fn dom_sync_uses_incremental_relayout() {
    let (mut s, t) = test_session();
    t.route(
        "/",
        200,
        "<!doctype html><html><body><div id=\"h\">x</div>\
         <script>document.body.onclick=function(){\
         var b=document.createElement('div');b.textContent='AJOUT';\
         document.body.appendChild(b)}</script></body></html>",
    );
    t.route(
        "/stat",
        200,
        "<!doctype html><html><body><div id=\"h\">x</div>\
         <div>AJOUT</div><div>AJOUT</div></body></html>",
    );
    s.set_js_factory(explorer_js::JscoreEngine::factory());
    let a = s.open_tab();
    s.navigate(a, "http://h.test/").await.unwrap();

    // ADR-0025 (étape 111) : le journal de mutations supplante
    // le relayout incrémental 55f — la première mutation
    // bootstrape la table d'identité via un repli cléé, la
    // seconde est rejouée **en place** (NodeId stables).
    s.handle_input(a, InputEvent::Click { x: 10.0, y: 10.0 })
        .await
        .unwrap();
    s.handle_input(a, InputEvent::Click { x: 10.0, y: 10.0 })
        .await
        .unwrap();
    assert!(
        s.journal_applies() >= 1,
        "la seconde mutation doit rejouer par journal (in-place)"
    );

    let b = s.open_tab();
    s.navigate(b, "http://h.test/stat").await.unwrap();
    assert_eq!(
        s.frame_png(a),
        s.frame_png(b),
        "le rendu du journal doit égaler la page statique"
    );
}

/// Étape 55f — borne conservatrice : un flottant dans le
/// document rend le rejeu local impossible (`FloatCtx`
/// perdu) — la synchro retombe sur le reflow global et le
/// rendu reste correct (le compteur incrémental ne bouge
/// pas).
#[tokio::test]
async fn dom_sync_falls_back_when_floats_present() {
    let (mut s, t) = test_session();
    t.route(
        "/",
        200,
        "<!doctype html><html><body><div style=\"float:left\">f</div>\
         <script>document.body.onclick=function(){\
         var b=document.createElement('div');b.textContent='AJOUT';\
         document.body.appendChild(b)}</script></body></html>",
    );
    t.route(
        "/stat",
        200,
        "<!doctype html><html><body><div style=\"float:left\">f</div>\
         <div>AJOUT</div></body></html>",
    );
    s.set_js_factory(explorer_js::JscoreEngine::factory());
    let a = s.open_tab();
    s.navigate(a, "http://h.test/").await.unwrap();
    let before = s.incremental_syncs();

    // Clic dans le flottant (shrink-to-fit R-29 : ~8 px de
    // large — le body a hauteur nulle, un clic hors du
    // flottant tombe sur <html>, hors de la chaîne de
    // propagation vers body).
    s.handle_input(a, InputEvent::Click { x: 4.0, y: 4.0 })
        .await
        .unwrap();
    assert_eq!(
        s.incremental_syncs(),
        before,
        "flottant → repli conservateur sur reflow global"
    );

    let b = s.open_tab();
    s.navigate(b, "http://h.test/stat").await.unwrap();
    assert_eq!(
        s.frame_png(a),
        s.frame_png(b),
        "le repli doit produire la même frame que la page statique"
    );
}

/// Reliquat R-88 — la voie incrémentale (`relayout_subtree`)
/// ne posait pas `root_font_size` : tout `rem` du sous-arbre
/// retombait sur l'initiale (16 px) dès la synchro cléée.
/// `html{font-size:20px}` + `table{width:2rem}` : un
/// `setAttribute` sur `<table>` est classé « re-parse » par
/// le journal (ADR-0025 — `RISKY_PARENT`), le `dirty` reste
/// intact → synchro cléée → `relayout_subtree` sur la table.
/// 40 px attendus (32 avant le fix) ; le compteur
/// `incremental_syncs` prouve la voie empruntée.
#[tokio::test]
async fn rem_basis_survives_incremental_relayout() {
    let (mut s, t) = test_session();
    t.route(
        "/m",
        200,
        "<html><head><style>html{font-size:20px}\
         #t{width:2rem;height:2rem;background:#ff0000}\
         </style></head><body><table id=\"t\"><tr><td>x</td></tr></table>\
         <script>document.body.onclick=function(){\
         document.getElementById('t').setAttribute('data-k','1')}</script>\
         </body></html>",
    );
    // Équivalent statique : 2rem sur racine 20px = 40px.
    t.route(
        "/s",
        200,
        "<html><head><style>#t{width:40px;height:40px;background:#ff0000}\
         </style></head><body>\
         <table id=\"t\" data-k=\"1\"><tr><td>x</td></tr></table>\
         </body></html>",
    );
    s.set_js_factory(explorer_js::JscoreEngine::factory());

    let a = s.open_tab();
    s.navigate(a, "http://h.test/m").await.unwrap();
    let before = s.incremental_syncs();
    s.handle_input(a, InputEvent::Click { x: 10.0, y: 10.0 })
        .await
        .unwrap();
    assert!(
        s.incremental_syncs() > before,
        "la synchro classée re-parse doit passer par relayout_subtree"
    );

    let b = s.open_tab();
    s.navigate(b, "http://h.test/s").await.unwrap();
    let (pa, pb) = (s.frame_png(a), s.frame_png(b));
    assert!(pa.is_some() && pb.is_some());
    assert_eq!(
        pa, pb,
        "le rem doit tenir la font-size racine (20px) après relayout incrémental"
    );
}

/// Étape 56e — métrique mission 11 : `<img src>` déclenche
/// bien une requête de sous-ressource (document + image =
/// 2 hits transport). La panne observée terrain venait du
/// harness `cache_server.py` (variable `PAGE` morte — la
/// page servie ne contenait pas d'`<img>`).
#[tokio::test]
async fn img_src_triggers_subresource_fetch() {
    let (mut s, t) = test_session();
    t.route("/", 200, "<html><body><img src=\"/i.png\"></body></html>");
    t.route_bin("/i.png", 200, red_png());
    let id = s.open_tab();
    let before = t.calls();
    s.navigate(id, "http://h.test/").await.unwrap();
    assert_eq!(t.calls(), before + 2, "document + <img> attendus (56e)");
}

/// Transport à goulot par route (régression 56c) : `/slow`
/// livre un premier morceau (peinture progressive) puis
/// attend `released` pour la fin — reproduit la fenêtre
/// « peint mais jamais commis » d'une nav supplantée.
struct SlowThenGate {
    released: Arc<tokio::sync::Notify>,
    painted: Arc<tokio::sync::Notify>,
}

#[async_trait]
impl HttpTransport for SlowThenGate {
    async fn fetch(&self, _req: &Request) -> NetResult<Response> {
        Ok(Response {
            status: 404,
            reason: "gated".to_string(),
            headers: vec![],
            body: b"missing".to_vec(),
            reused_connection: false,
            resolved_url: None,
        })
    }

    async fn fetch_stream(&self, req: &Request) -> NetResult<explorer_net::http::StreamedResponse> {
        let (tx, rx) = tokio::sync::mpsc::channel(4);
        let (released, painted) = (self.released.clone(), self.painted.clone());
        let path = req.url.path().to_string();
        tokio::spawn(async move {
            if path == "/slow" {
                // Premier morceau > fenêtre de sniffing du
                // décodeur (1 024 octets) : sans ça `feed`
                // tamponne et la peinture progressive verrait
                // un DOM vide (no-op `EmptyDocument`).
                let mut first = b"<html><title>slow</title><body>SLOW ".to_vec();
                first.extend(std::iter::repeat_n(b'S', 1100));
                let _ = tx.send(Ok(first)).await;
                painted.notify_one();
                released.notified().await;
                let _ = tx.send(Ok(b" fin</body></html>".to_vec())).await;
            } else {
                let body = match path.as_str() {
                    "/a" => "<html><title>a</title><body>AAA</body></html>",
                    _ => "<html><title>c</title><body>CCC</body></html>",
                };
                let _ = tx.send(Ok(body.as_bytes().to_vec())).await;
            }
        });
        Ok(explorer_net::http::StreamedResponse {
            status: 200,
            reason: "gated".to_string(),
            headers: vec![],
            reused_connection: false,
            resolved_url: None,
            body: rx,
        })
    }
}

/// Étape 56c — la peinture progressive d'une navigation
/// **supplantée** ne doit pas laisser l'onglet hybride
/// (frame du document en vol + DOM/URL de l'ancienne page)
/// ni empoisonner le snapshot bfcache : A → nav /slow
/// (peint, en vol) → nav /c commit → /slow est jetée, le
/// repeint depuis le DOM commis doit ramener la frame sur C.
#[tokio::test]
async fn superseded_nav_reverts_speculative_paint() {
    use explorer_net::url::Url;

    let released = Arc::new(tokio::sync::Notify::new());
    let painted = Arc::new(tokio::sync::Notify::new());
    let t = Arc::new(SlowThenGate {
        released: released.clone(),
        painted: painted.clone(),
    });
    let sess = Arc::new(tokio::sync::Mutex::new(Session::with_config(
        t,
        Arc::new(FixtureFont),
        SessionConfig {
            paint_chunk_bytes: 1,
            paint_interval_ms: 0,
            ..SessionConfig::default()
        },
    )));
    let tab = sess.lock().await.open_tab();

    // A commit sa page.
    Box::pin(explorer_core::navigate_shared(
        &sess,
        tab,
        Request::get(Url::parse("https://h/a").unwrap()),
        true,
    ))
    .await
    .unwrap();
    let a_frame = sess.lock().await.frame_png(tab);

    // /slow démarre, peint son document en vol, puis attend.
    let s2 = sess.clone();
    let slow = tokio::spawn(async move {
        Box::pin(explorer_core::navigate_shared(
            &s2,
            tab,
            Request::get(Url::parse("https://h/slow").unwrap()),
            true,
        ))
        .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(2), painted.notified())
        .await
        .expect("/slow n'a jamais peint");
    // La peinture spéculative doit s'appliquer : la frame
    // montre /slow alors que l'URL commise reste /a — on
    // attend le changement effectif (pas un délai fixe).
    let mut spec_seen = false;
    for _ in 0..100 {
        {
            let mut s = sess.lock().await;
            if s.frame_png(tab) != a_frame {
                spec_seen = true;
                break;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(
        spec_seen,
        "sanité : la peinture progressive n'a jamais changé la frame"
    );

    // /c commit — /slow est supplantée : sa nav_apply sera
    // jetée, la peinture spéculative doit être révertée.
    Box::pin(explorer_core::navigate_shared(
        &sess,
        tab,
        Request::get(Url::parse("https://h/c").unwrap()),
        true,
    ))
    .await
    .unwrap();
    released.notify_waiters();
    slow.await.unwrap().unwrap();

    // Référence : la page /c rendue dans un onglet neuf.
    let ref_tab = sess.lock().await.open_tab();
    Box::pin(explorer_core::navigate_shared(
        &sess,
        ref_tab,
        Request::get(Url::parse("https://h/c").unwrap()),
        true,
    ))
    .await
    .unwrap();

    let mut s = sess.lock().await;
    assert_eq!(s.tab_url(tab).as_deref(), Some("https://h/c"));
    assert_eq!(
        s.frame_png(tab),
        s.frame_png(ref_tab),
        "l'état hybride (frame /slow + DOM /c) a survécu (56c)"
    );
}

/// Étape 56c — `navigate` vers l'URL de l'entrée **forward**
/// (`can_forward: true`) restaure le snapshot bfcache comme
/// un `go_forward` — commit complet de l'entrée — au lieu
/// d'un fetch neuf dont la peinture progressive afficherait
/// B pendant que l'entrée commise reste A (état hybride du
/// repro terrain mission 11 : A→B→back→`navigate B`).
#[tokio::test]
async fn navigate_to_forward_url_restores_bfcache_entry() {
    let (mut s, t) = test_session();
    t.route("/a", 200, PAGE_T);
    t.route("/b", 200, PAGE);
    let id = s.open_tab();
    s.navigate(id, "http://h.test/a").await.unwrap();
    s.navigate(id, "http://h.test/b").await.unwrap();
    s.go_back(id).await.unwrap();
    assert_eq!(s.tab_url(id).as_deref(), Some("http://h.test/a"));
    let calls = t.calls();

    // navigate(B) pendant can_forward → restauration gelée :
    // zéro réseau, et l'entrée forward est réellement commise.
    s.navigate(id, "http://h.test/b").await.unwrap();
    assert_eq!(s.tab_url(id).as_deref(), Some("http://h.test/b"));
    assert_eq!(t.calls(), calls, "restauration bfcache — pas de re-fetch");
    assert!(
        matches!(s.go_forward(id).await, Err(CoreError::EmptyDocument)),
        "l'entrée forward doit être consommée par la restauration"
    );

    // Un navigate vers une URL absente de l'historique reste
    // un fetch normal (pas de faux positif sur le raccourci).
    t.route("/d", 200, PAGE);
    s.navigate(id, "http://h.test/d").await.unwrap();
    assert_eq!(t.calls(), calls + 1);
}

/// Étape 56d — `href="#id"` / `url#frag` : navigation
/// **même-document** = entrée d'historique + scroll vers la
/// cible (`id` puis `<a name>`), jamais de re-fetch ;
/// fragment cross-document appliqué après le commit.
#[tokio::test]
async fn fragment_nav_scrolls_without_refetch() {
    let (mut s, t) = test_session();
    // Cible à ~2000px — déborde le viewport 800×600.
    let page = "<!doctype html><html><head><style>.sp{height:2000px}</style></head>\
        <body><div class=\"sp\">haut</div><p id=\"sec\">cible</p>\
        <a name=\"named\">nommée</a></body></html>";
    t.route("/a", 200, page);
    t.route("/b", 200, page);
    let id = s.open_tab();
    s.navigate(id, "http://h.test/a").await.unwrap();
    let calls = t.calls();

    // Même document : `#sec` → historique + scroll, zéro fetch.
    s.navigate(id, "http://h.test/a#sec").await.unwrap();
    assert_eq!(t.calls(), calls, "même document : pas de re-fetch (56d)");
    assert_eq!(s.tab_url(id).as_deref(), Some("http://h.test/a#sec"));
    let scroll = match s
        .handle_input(id, InputEvent::Scroll { dy: 0.0 })
        .await
        .unwrap()
    {
        InputOutcome::Scrolled { scroll_y } => scroll_y,
        o => panic!("inattendu : {o:?}"),
    };
    assert!(
        scroll > 0.0,
        "le fragment doit scroller vers la cible (56d)"
    );

    // Repli `<a name>` : même résultat.
    s.navigate(id, "http://h.test/a#named").await.unwrap();
    let scroll = match s
        .handle_input(id, InputEvent::Scroll { dy: 0.0 })
        .await
        .unwrap()
    {
        InputOutcome::Scrolled { scroll_y } => scroll_y,
        o => panic!("inattendu : {o:?}"),
    };
    assert!(scroll > 0.0, "`<a name>` est une cible valide (56d)");

    // Cross-document : /b#sec fetch puis scroll post-commit.
    s.navigate(id, "http://h.test/b#sec").await.unwrap();
    assert_eq!(t.calls(), calls + 1, "URL différente : fetch normal");
    let scroll = match s
        .handle_input(id, InputEvent::Scroll { dy: 0.0 })
        .await
        .unwrap()
    {
        InputOutcome::Scrolled { scroll_y } => scroll_y,
        o => panic!("inattendu : {o:?}"),
    };
    assert!(
        scroll > 0.0,
        "fragment cross-document appliqué post-commit (56d)"
    );
}

/// Étape 56f — les images `data:` peignent leur bitmap
/// inline en session (zéro requête réseau) au lieu de 0
/// pixel : `<img src=data:>` et `background-image:
/// url(data:)` — payload percent-encodé (le décodeur
/// `data_url_bytes` le traite sans base64).
#[tokio::test]
async fn data_url_images_paint_inline() {
    use std::fmt::Write;
    let (mut sess, tr) = test_session();
    let enc = red_png().iter().fold(String::new(), |mut out, byte| {
        let _ = write!(out, "%{byte:02X}");
        out
    });
    tr.route(
        "/i",
        200,
        Box::leak(
            format!("<html><body><img src=\"data:image/png,{enc}\"></body></html>")
                .into_boxed_str(),
        ),
    );
    tr.route(
        "/g",
        200,
        Box::leak(
            format!(
                "<html><head><style>#b{{display:inline-block;background-image:url(\"data:image/png,{enc}\");\
                 width:2px;height:2px}}</style></head><body><div id=\"b\"></div></body></html>"
            )
            .into_boxed_str(),
        ),
    );
    // Référence : bloc 2×2 à fond rouge (même oracle que
    // `img_renders_decoded_bitmap`).
    tr.route(
        "/s",
        200,
        "<html><head><style>#b{display:inline-block;background-color:#ff0000;\
         width:2px;height:2px}</style></head>\
         <body><div id=\"b\"></div></body></html>",
    );

    let img_tab = sess.open_tab();
    sess.navigate(img_tab, "http://h.test/i").await.unwrap();
    let ref_tab = sess.open_tab();
    sess.navigate(ref_tab, "http://h.test/s").await.unwrap();
    let bg_tab = sess.open_tab();
    sess.navigate(bg_tab, "http://h.test/g").await.unwrap();

    assert_eq!(
        sess.frame_png(img_tab),
        sess.frame_png(ref_tab),
        "img data: = bitmap peinte (56f)"
    );
    assert_eq!(
        sess.frame_png(bg_tab),
        sess.frame_png(ref_tab),
        "background-image data: = bitmap peinte (56f)"
    );
    // Aucune sous-ressource `data:` ne part en fetch.
    assert_eq!(tr.calls(), 3, "data: = zéro requête réseau (56f)");
}

/// Étape 55e — `@keyframes` cadencé en session : le défaut
/// terrain (`anim.html` — deux frames à 400 ms d'écart
/// étaient identiques). La lecture doit produire des frames
/// différentes entre deux pulls, être pilotée aussi par le
/// battement hôte (`pump_timers_all`), puis se replier sur
/// le style de base une fois terminée (`fill` absent).
#[tokio::test]
async fn keyframes_animation_advances_and_finishes() {
    let (mut sess, tr) = test_session();
    tr.route(
        "/k",
        200,
        "<html><head><style>\
         @keyframes k { from { background-color: #ff0000 }\
         to { background-color: #0000ff } }\
         #b { width:2px; height:2px; animation: k 500ms linear }\
         </style></head><body><div id=\"b\"></div></body></html>",
    );
    let tab = sess.open_tab();
    sess.navigate(tab, "http://h.test/k").await.unwrap();
    let start = sess.frame_png(tab).unwrap();
    // Mi-lecture (~40 % du segment) : la couleur interpolée
    // diffère du `from` rouge — pull UI (tick_animations).
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    assert!(
        sess.tick_animations(tab).unwrap(),
        "une animation en lecture doit produire une frame (55e)"
    );
    let mid = sess.frame_png(tab).unwrap();
    assert_ne!(
        start, mid,
        "frames identiques en cours d'animation — le bug anim.html (55e)"
    );
    // Identité stable : un tick immédiat conserve la frame
    // (le start n'est pas réarmé par le re-style).
    let mid2 = sess.frame_png(tab).unwrap();
    assert_eq!(mid, mid2, "un re-style immédiat ne doit pas redémarrer");
    // Fin de lecture via le battement hôte (pump_timers_all)
    // — `fill` absent → repli sur le style calculé brut.
    tokio::time::sleep(std::time::Duration::from_millis(450)).await;
    sess.pump_timers_all();
    let end = sess.frame_png(tab).unwrap();
    assert_ne!(
        mid, end,
        "l'animation terminée replie sur le style de base (55e)"
    );
}

/// Étape 55e (rouvert, mission 15) — la pompe **partagée**
/// doit avancer les animations même sans moteur JS : une
/// page `@keyframes` purement CSS a `js: None` → la branche
/// « pas de moteur » de `pump_timers_shared` doit quand
/// même battre `advance_animations`, sinon le daemon
/// produit des frames identiques (anim.html, 6 pulls).
#[tokio::test]
async fn shared_pump_advances_scriptless_animations() {
    let (sess0, tr) = test_session();
    tr.route(
        "/a",
        200,
        "<html><head><style>\
         @keyframes k { from { background-color: #ff0000 }\
         to { background-color: #0000ff } }\
         #b { width:2px; height:2px; animation: k 500ms linear }\
         </style></head><body><div id=\"b\"></div></body></html>",
    );
    let sess = std::sync::Arc::new(tokio::sync::Mutex::new(sess0));
    let tab = sess.lock().await.open_tab();
    sess.lock()
        .await
        .navigate(tab, "http://h.test/a")
        .await
        .unwrap();
    let start = sess.lock().await.frame_png(tab).unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(120)).await;
    // Page sans <script> → `take_js_engine` rend None : le
    // battement doit tout de même relayout+repaint l'anim.
    explorer_core::pump_timers_shared(&sess, tab).await;
    let mid = sess.lock().await.frame_png(tab).unwrap();
    assert_ne!(
        start, mid,
        "pompe partagée inerte sans moteur JS — anim figée (55e)"
    );
}

/// Étape 55e — garde : un élément sans `animation` (ou
/// `animation-name: none`) ne déclenche aucun tick de
/// relayout — `tick_animations` reste inerte.
#[tokio::test]
async fn keyframes_absent_means_no_animation_ticks() {
    let (mut sess, tr) = test_session();
    tr.route(
        "/s",
        200,
        "<html><head><style>\
         #b { width:2px; height:2px; background-color: #ff0000;\
         animation-name: none }\
         </style></head><body><div id=\"b\"></div></body></html>",
    );
    let tab = sess.open_tab();
    sess.navigate(tab, "http://h.test/s").await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert!(
        !sess.tick_animations(tab).unwrap(),
        "aucun relayout sans animation en lecture (55e)"
    );
}

/// Moteur de test : `run_pending` signale son entrée puis
/// bloque sur un canal — sert à prouver que la pompe partagée
/// laisse le verrou session libre pendant l'exécution JS
/// (étape 53f : `repros/relayout.html` figeait `/status`
/// ~20 s en tenant `&mut Session`).
struct BarrierEngine {
    entered: std::sync::mpsc::Sender<()>,
    release: std::sync::mpsc::Receiver<()>,
}

impl explorer_js::JsEngine for BarrierEngine {
    fn eval(&mut self, _src: &str) -> Result<String, explorer_js::JsError> {
        Ok("undefined".to_string())
    }

    fn run_pending_bounded(&mut self, _max: usize) -> usize {
        let _ = self.entered.send(());
        let _ = self.release.recv();
        1
    }

    fn set_budget(&mut self, _ops: u64) {}
}

/// Étape 53f — pendant que `run_pending` est bloqué (moteur
/// extrait dans `spawn_blocking`), le verrou session doit
/// rester acquisissable : `/status` ne gèle plus.
#[tokio::test(flavor = "current_thread")]
async fn pump_shared_keeps_lock_free_during_js_burst() {
    let (mut s, t) = test_session();
    t.route("/", 200, PAGE_SCRIPT);
    // Fabrique one-shot : les canaux de la barrière sont
    // posés avant la navigation qui installe le moteur.
    let slot = Arc::new(Mutex::new(
        None::<(std::sync::mpsc::Sender<()>, std::sync::mpsc::Receiver<()>)>,
    ));
    let slot2 = slot.clone();
    s.set_js_factory(Arc::new(move || {
        let (entered, release) = slot2
            .lock()
            .unwrap()
            .take()
            .expect("barrière posée avant la navigation");
        Box::new(BarrierEngine { entered, release })
    }));
    let (etx, erx) = std::sync::mpsc::channel::<()>();
    let (rtx, rrx) = std::sync::mpsc::channel::<()>();
    *slot.lock().unwrap() = Some((etx, rrx));
    let id = s.open_tab();
    s.navigate(id, "http://h.test/").await.unwrap();

    let sess = Arc::new(tokio::sync::Mutex::new(s));
    let s2 = sess.clone();
    let pump = tokio::spawn(async move { explorer_core::pump_timers_shared(&s2, id).await });

    // Le moteur est entré dans `run_pending` — attente sans
    // bloquer le worker (runtime current_thread).
    tokio::task::spawn_blocking(move || erx.recv())
        .await
        .unwrap()
        .unwrap();

    // Le verrou session reste acquisissable pendant le burst.
    let guard = tokio::time::timeout(std::time::Duration::from_secs(5), sess.lock())
        .await
        .expect("le verrou session reste libre pendant le burst JS");
    drop(guard);

    let _ = rtx.send(());
    assert_eq!(pump.await.unwrap(), 1, "le job bloqué est bien rapporté");
}

/// Étape 53f — la pompe partagée applique les mutations DOM
/// comme la pompe synchrone : même résultat rendu que la
/// page statique équivalente.
#[tokio::test(flavor = "current_thread")]
async fn pump_shared_applies_dom_mutations() {
    let (mut s, t) = test_session();
    t.route("/ev", 200, PAGE_PUMP);
    t.route("/stat", 200, PAGE_PUMP_AFTER);
    s.set_js_factory(explorer_js::JscoreEngine::factory());
    let a = s.open_tab();
    s.navigate(a, "http://h.test/ev").await.unwrap();
    let b = s.open_tab();
    s.navigate(b, "http://h.test/stat").await.unwrap();

    let sess = Arc::new(tokio::sync::Mutex::new(s));
    // Pomper jusqu'à épuisement des timers — chaque phase
    // take/commit ne tient le verrou que quelques µs.
    for _ in 0..10 {
        explorer_core::pump_timers_shared(&sess, a).await;
        let done = sess.lock().await.next_timer_deadline().is_none();
        if done {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    explorer_core::pump_timers_shared(&sess, a).await;

    let mut s = sess.lock().await;
    assert_eq!(s.frame_png(a), s.frame_png(b));
    assert_eq!(s.next_timer_deadline(), None);
}

/// Étape 58 — `<video poster>` : l'affiche suit le canal
/// image (fetch borné + décodage) et se peint étirée dans
/// la boîte vidéo 300×150 de la feuille UA complémentaire
/// (ADR-0023 — aucun décodeur embarqué).
#[tokio::test]
async fn video_poster_paints_in_media_box() {
    let (mut s, t) = test_session();
    // `width`/`height` présentationnels (R-34) : boîte
    // 300×150, l'affiche 2×2 est étirée (`object-fit:fill`)
    // sur toute la content-box.
    t.route(
        "/v",
        200,
        "<html><body><video poster=\"/p.png\" width=\"300\" height=\"150\"></video></body></html>",
    );
    t.route_bin("/p.png", 200, red_png());
    // Référence : inline-block 300×150 uniformément rouge.
    t.route(
        "/s",
        200,
        "<html><body><div style=\"display:inline-block;width:300px;\
         height:150px;background-color:#ff0000\"></div></body></html>",
    );

    let a = s.open_tab();
    s.navigate(a, "http://h.test/v").await.unwrap();
    let b = s.open_tab();
    s.navigate(b, "http://h.test/s").await.unwrap();
    assert_eq!(s.frame_png(a), s.frame_png(b));
}

/// Étape 58 — la boîte de repli `<video>` sans affiche :
/// fond noir 300×150 (feuille UA complémentaire), comme
/// l'élément remplacé standard.
#[tokio::test]
async fn video_without_poster_renders_black_box() {
    let (mut s, t) = test_session();
    t.route("/v", 200, "<html><body><video></video></body></html>");
    t.route(
        "/s",
        200,
        "<html><body><div style=\"display:inline-block;width:300px;\
         height:150px;background-color:#000000\"></div></body></html>",
    );

    let a = s.open_tab();
    s.navigate(a, "http://h.test/v").await.unwrap();
    let b = s.open_tab();
    s.navigate(b, "http://h.test/s").await.unwrap();
    assert_eq!(s.frame_png(a), s.frame_png(b));
}

/// Étape 58 — `<audio>` sans `controls` est masqué (UA
/// complement) : même rendu qu'une page vide.
#[tokio::test]
async fn audio_without_controls_is_hidden() {
    let (mut s, t) = test_session();
    t.route(
        "/a",
        200,
        "<html><body><audio src=\"/a.mp3\" autoplay></audio></body></html>",
    );
    t.route("/b", 200, "<html><body></body></html>");

    let a = s.open_tab();
    s.navigate(a, "http://h.test/a").await.unwrap();
    let b = s.open_tab();
    s.navigate(b, "http://h.test/b").await.unwrap();
    assert_eq!(s.frame_png(a), s.frame_png(b));
}

/// R-34 — taille **intrinsèque** de l'affiche : un `<video
/// poster>` sans `width`/`height` prend les dimensions du
/// bitmap décodé (2×2), comme un `<img>` — fini le 300×150
/// forcé. La comparaison PNG couvre collecte + sizing +
/// peinture bout en bout.
#[tokio::test]
async fn video_poster_intrinsic_size_matches_img() {
    let (mut s, t) = test_session();
    t.route(
        "/v",
        200,
        "<html><body><video poster=\"/p.png\"></video></body></html>",
    );
    t.route("/i", 200, "<html><body><img src=\"/p.png\"></body></html>");
    t.route_bin("/p.png", 200, red_png());

    let a = s.open_tab();
    s.navigate(a, "http://h.test/v").await.unwrap();
    let b = s.open_tab();
    s.navigate(b, "http://h.test/i").await.unwrap();
    assert_eq!(s.frame_png(a), s.frame_png(b));
}

/// Étape 58 — affiche `poster="data:…"` : même canal
/// inline que `img` (percent-encodé, zéro requête réseau).
#[tokio::test]
async fn video_poster_data_url_paints() {
    use std::fmt::Write;
    let (mut s, t) = test_session();
    let enc = red_png().iter().fold(String::new(), |mut out, byte| {
        let _ = write!(out, "%{byte:02X}");
        out
    });
    t.route(
        "/v",
        200,
        Box::leak(
            format!(
                "<html><body><video poster=\"data:image/png,{enc}\" \
                 width=\"300\" height=\"150\"></video></body></html>"
            )
            .into_boxed_str(),
        ),
    );
    t.route(
        "/s",
        200,
        "<html><body><div style=\"display:inline-block;width:300px;\
         height:150px;background-color:#ff0000\"></div></body></html>",
    );

    let a = s.open_tab();
    s.navigate(a, "http://h.test/v").await.unwrap();
    let b = s.open_tab();
    s.navigate(b, "http://h.test/s").await.unwrap();
    assert_eq!(s.frame_png(a), s.frame_png(b));
}

/// Étape 68 — occurrence déjà dans le viewport : le scroll
/// ne bouge pas mais la surbrillance doit quand même être
/// rasterisée (avant : `FillRect` calculés, jamais peints).
#[tokio::test]
async fn find_highlight_repaints_without_scroll() {
    let (mut s, t) = test_session();
    t.route("/f", 200, "<html><body><p>lorem ipsum</p></body></html>");
    let a = s.open_tab();
    s.navigate(a, "http://h.test/f").await.unwrap();
    let before = s.frame_png(a);
    assert_eq!(s.find_in_page(a, "lorem"), 1);
    let after = s.frame_png(a);
    assert_ne!(before, after, "surbrillance rasterisée sans déplacement");
}

/// R-35 — le surlignage find suit les métriques glyphe, pas
/// la boîte texte : « rem » dans « loremipsum » couvre ≈ 3
/// glyphes sur 10 (bande étroite), positionnée sur le mot.
#[allow(clippy::cast_precision_loss)]
#[tokio::test]
async fn find_highlight_follows_glyph_metrics() {
    let (mut s, t) = test_session();
    // Fixture Ahem : avance uniforme 0.5em → 20 px/car à 40px.
    t.route(
        "/f",
        200,
        "<html><body><p style=\"font-size:40px\">loremipsum</p></body></html>",
    );
    let a = s.open_tab();
    s.navigate(a, "http://h.test/f").await.unwrap();
    assert_eq!(s.find_in_page(a, "rem"), 1);
    let (w, _h, px) = s.frame_pixels(a).unwrap();
    // Colonnes couvertes par la surbrillance chaude (orange
    // occurrence courante — même seuil que le test étape 75).
    let mut xs = Vec::new();
    for (i, p) in px.chunks_exact(4).enumerate() {
        if p[0] > 230 && p[1] > 110 && p[1] < 250 && p[2] < 150 {
            xs.push(i % w as usize);
        }
    }
    assert!(!xs.is_empty(), "surbrillance peinte");
    let (lo, hi) = (
        *xs.iter().min().unwrap() as f32,
        *xs.iter().max().unwrap() as f32,
    );
    // « rem » = caractères 2..5 : bande ≈ [40, 100] px (avec
    // l'anti-aliasing du blend, tolérance généreuse) — l'ancien
    // repli boîte-entière peignait ≈ [0, 200].
    assert!(lo >= 25.0, "début {lo} : les 2 premiers glyphes exclus");
    assert!(hi <= 140.0, "fin {hi} : le reste du mot exclu");
}

/// R-35 — granularité occurrence : un match dans un élément
/// niché (<b>) n'est compté qu'une fois (le repli boîte ne
/// double pas la couverture fragments) ; un match qui
/// enjambe deux fragments reste UNE occurrence navigable.
#[tokio::test]
async fn find_groups_cross_fragment_occurrence() {
    let (mut s, t) = test_session();
    t.route(
        "/f",
        200,
        "<html><body><p style=\"font-size:40px\">aa<b>bb</b>cc</p></body></html>",
    );
    let a = s.open_tab();
    s.navigate(a, "http://h.test/f").await.unwrap();
    // « bb » vit dans un fragment niché — une occurrence, pas
    // de doublon boîte-entière.
    assert_eq!(s.find_in_page(a, "bb"), 1, "match niché dédupliqué");
    // « abb » enjambe les fragments « aa » et « bb » : une
    // occurrence (deux sous-rects peints), navigation stable.
    assert_eq!(s.find_in_page(a, "abb"), 1, "match transversal groupé");
    assert_eq!(s.find_step(a, 1), (0, 1), "une seule occurrence navigable");
}

/// Étape 67 — l'URL commise est la cible **finale** de la
/// chaîne de redirections (barre d'adresse + historique),
/// et le fragment du `Location` est appliqué au commit
/// (`/landing#sec` scrollait à 0 avant le fix).
#[tokio::test]
async fn redirect_commits_final_url_and_fragment() {
    let inner = Arc::new(FakeTransport::new());
    inner.route_headers("/r301", 301, "", &[("Location", "/landing#sec")]);
    inner.route(
        "/landing",
        200,
        "<html><body>\
         <div style=\"height:2000px\">haut</div>\
         <div id=\"sec\">cible</div></body></html>",
    );
    let t: Arc<dyn explorer_net::http::HttpTransport> =
        Arc::new(explorer_net::policy::RedirectTransport::wrap(inner, 8));
    let mut s = Session::with_config(
        t,
        Arc::new(FixtureFont),
        SessionConfig {
            event_capacity: 16,
            viewport: (800.0, 600.0),
            scale: 1.0,
            js_ops: 5_000_000,
            max_images: 32,
            image_max_px: 16_000_000,
            max_sheets: 8,
            max_imports: 8,
            max_fonts: 16,
            max_scripts: 32,
            max_frames: 8,
            max_modules: 64,
            max_inflight: 6,
            max_script_net: 16,
            max_timer_jobs: 64,
            paint_chunk_bytes: 32 * 1024,
            paint_interval_ms: 100,
            resync_gap_max_ms: 4_000,
            max_download_bytes: 64 * 1024 * 1024,
            bfcache_pages: 4,
            dom_journal_max_ops: 512,
            js_mem_limit_mib: 512,
            color_scheme: explorer_css::ColorScheme::default(),
        },
    );

    let a = s.open_tab();
    s.navigate(a, "http://h.test/r301").await.unwrap();
    // URL commise = cible finale + son fragment.
    assert_eq!(s.tab_url(a).as_deref(), Some("http://h.test/landing#sec"));
    // Le fragment du Location a scrollé vers `#sec`.
    let scroll = match s
        .handle_input(a, InputEvent::Scroll { dy: 0.0 })
        .await
        .unwrap()
    {
        InputOutcome::Scrolled { scroll_y } => scroll_y,
        o => panic!("inattendu : {o:?}"),
    };
    assert!(scroll > 0.0, "fragment du Location appliqué");
}

/// Étape 88 — `about:blank` : le document vide canonique se
/// navigue **sans fetch** (bouton Home, rejeu omnibox) — la
/// preuve est le compteur du transport : aucune route
/// `about:` n'existe, un fetch partirait en échec.
#[tokio::test]
async fn about_blank_navigates_sans_fetch() {
    let (mut s, t) = test_session();
    t.route("/", 200, PAGE_T);
    let id = s.open_tab();
    s.navigate(id, "http://h.test/").await.unwrap();
    let fetched = t.calls();

    s.navigate(id, "about:blank").await.unwrap();
    assert_eq!(s.tab_url(id).as_deref(), Some("about:blank"));
    assert_eq!(t.calls(), fetched, "about:blank ne doit pas fetcher");
    // Document commis : frame blanche peinte, titre vide
    // (l'UI retombe sur l'URL), historique utilisable.
    let png = s.frame_png(id).unwrap();
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
    s.go_back(id).await.unwrap();
    assert_eq!(s.tab_url(id).as_deref(), Some("http://h.test/"));
}

/// Étape 111 / ADR-0025 — le journal de mutations rejoue la
/// synchro **en place** (`NodeId` stables : images, focus, find
/// survivent sans re-key) au lieu du re-parse intégral. La
/// première mutation tombe en `NoBaseline` (bootstrap : la
/// table d'identité se monte à la baseline cléée servie en
/// repli), les suivantes rejouent — `journal_applies` le
/// mesure.
#[tokio::test]
async fn journal_apply_replays_in_place() {
    let (mut s, t) = test_session();
    t.route(
        "/m",
        200,
        "<html><body><div id=\"x\">avant</div>\
         <script>setTimeout(function(){\
         document.getElementById('x').textContent='APRES'},40);\
         setTimeout(function(){\
         document.getElementById('x').setAttribute('data-ok','1')},500)</script>\
         </body></html>",
    );
    // DOM final équivalent, statique.
    t.route(
        "/s",
        200,
        "<html><body><div id=\"x\" data-ok=\"1\">APRES</div></body></html>",
    );
    s.set_js_factory(explorer_js::JscoreEngine::factory());

    let a = s.open_tab();
    s.navigate(a, "http://h.test/m").await.unwrap();
    // Rafale 1 : le timer 40 ms seul — journal → NoBaseline
    // (pas encore de table d'identité) → repli cléé qui la
    // monte.
    std::thread::sleep(std::time::Duration::from_millis(90));
    s.pump_timers(a);
    assert_eq!(s.journal_fallbacks(), 1, "bootstrap NoBaseline attendu");
    // Rafale 2 : le timer 500 ms — la baseline existe, le
    // journal rejoue en place.
    std::thread::sleep(std::time::Duration::from_millis(500));
    s.pump_timers(a);
    assert!(
        s.journal_applies() >= 1,
        "le second timer doit rejouer par journal (in-place)"
    );
    let b = s.open_tab();
    s.navigate(b, "http://h.test/s").await.unwrap();
    let (pa, pb) = (s.frame_png(a), s.frame_png(b));
    assert!(pa.is_some() && pb.is_some());
    assert_eq!(pa, pb, "le rejeu en place doit égaler le DOM statique");
}

/// `dom_journal_max_ops` dépassé → repli re-parse mesuré
/// (`journal_fallbacks`) — la page reste correcte.
#[tokio::test]
async fn journal_ops_limit_falls_back() {
    let t = Arc::new(FakeTransport::new());
    let mut s = Session::with_config(
        t.clone(),
        Arc::new(FixtureFont),
        SessionConfig {
            dom_journal_max_ops: 1, // tout journal > 1 op → repli
            ..SessionConfig::default()
        },
    );
    t.route(
        "/m",
        200,
        "<html><body><div id=\"x\">avant</div><div id=\"y\">0</div>\
         <script>setTimeout(function(){\
         document.getElementById('x').textContent='APRES';\
         document.getElementById('y').textContent='1'},60)</script>\
         </body></html>",
    );
    t.route(
        "/s",
        200,
        "<html><body><div id=\"x\">APRES</div><div id=\"y\">1</div></body></html>",
    );
    s.set_js_factory(explorer_js::JscoreEngine::factory());

    let a = s.open_tab();
    s.navigate(a, "http://h.test/m").await.unwrap();
    std::thread::sleep(std::time::Duration::from_millis(120));
    s.pump_timers(a);
    assert!(
        s.journal_fallbacks() >= 1,
        "borne d'ops → repli : {}",
        s.journal_fallbacks()
    );
    let b = s.open_tab();
    s.navigate(b, "http://h.test/s").await.unwrap();
    let (pa, pb) = (s.frame_png(a), s.frame_png(b));
    assert!(pa.is_some() && pb.is_some());
    assert_eq!(pa, pb, "le repli re-parse garde le DOM correct");
}

/// Régression mesurée sur dev.to : le moteur garde la casse
/// SVG (`clipPath`) là où le miroir sérialise `clippath` —
/// avant la normalisation, l'empreinte divergeait à la
/// baseline et **tout** journal tombait en `Fingerprint`.
/// Une page à SVG inline + une mutation banale doit rejouer.
#[tokio::test]
async fn journal_survives_svg_case_divergence() {
    let (mut s, t) = test_session();
    t.route(
        "/m",
        200,
        "<html><body><div id=\"x\">avant</div>\
         <svg><clipPath id=\"c\"><path d=\"M0 0\"/></clipPath>\
         <linearGradient id=\"g\"/></svg>\
         <script>setTimeout(function(){\
         document.getElementById('x').textContent='UN'},40);\
         setTimeout(function(){\
         document.getElementById('x').setAttribute('data-ok','1')},500)</script>\
         </body></html>",
    );
    s.set_js_factory(explorer_js::JscoreEngine::factory());

    let a = s.open_tab();
    s.navigate(a, "http://h.test/m").await.unwrap();
    std::thread::sleep(std::time::Duration::from_millis(90));
    s.pump_timers(a);
    // Bootstrap : NoBaseline → baseline cléée (repli normal).
    std::thread::sleep(std::time::Duration::from_millis(500));
    s.pump_timers(a);
    assert!(
        s.journal_applies() >= 1,
        "le journal doit rejouer malgré clipPath/linearGradient : \
         applies={} replis={}",
        s.journal_applies(),
        s.journal_fallbacks()
    );
}

/// Mesure du périmètre du pilote (ADR-0025) — fixture des
/// classes classées « re-parse » : `<table>`/`select` (foster
/// parenting), `<script>` dans un fragment inséré. Chaque
/// classe doit être **comptée** par `journal_classified`
/// (repli cléé pré-drain — invisible à `journal_fallbacks`),
/// la page reste correcte (le repli est transparent), et la
/// mutation banale suivante rejoue par journal — auto-
/// réparation de la voie différentielle après chaque repli.
#[tokio::test]
async fn journal_classified_measures_exotic_classes() {
    let (mut s, t) = test_session();
    t.route(
        "/m",
        200,
        "<html><body><div id=\"x\">avant</div>\
         <table id=\"tb\"><tr><td>1</td></tr></table>\
         <select id=\"sel\"></select>\
         <script>\
         setTimeout(function(){\
         document.getElementById('x').setAttribute('data-b','1')},40);\
         setTimeout(function(){\
         var tr=document.createElement('tr');\
         tr.appendChild(document.createElement('td'));\
         document.getElementById('tb').appendChild(tr)},250);\
         setTimeout(function(){\
         var o=document.createElement('option');o.text='o2';\
         document.getElementById('sel').appendChild(o)},400);\
         setTimeout(function(){\
         document.getElementById('x').innerHTML='<i>s<\\/i>'},550);\
         setTimeout(function(){\
         document.getElementById('x').setAttribute('data-c','1')},700)\
         </script></body></html>",
    );
    s.set_js_factory(explorer_js::JscoreEngine::factory());

    let a = s.open_tab();
    s.navigate(a, "http://h.test/m").await.unwrap();
    // Rafale 1 (bénigne) : NoBaseline → baseline cléée montée.
    std::thread::sleep(std::time::Duration::from_millis(90));
    s.pump_timers(a);
    let classified_baseline = s.journal_classified();

    // Rafale 2 : insert <tr> sous <table> — parent risqué.
    std::thread::sleep(std::time::Duration::from_millis(200));
    s.pump_timers(a);
    let c_table = s.journal_classified();
    assert!(
        c_table > classified_baseline,
        "insert sous <table> classé re-parse : {classified_baseline} -> {c_table}"
    );

    // Rafale 3 : <option> sous <select>.
    std::thread::sleep(std::time::Duration::from_millis(200));
    s.pump_timers(a);
    let c_select = s.journal_classified();
    assert!(
        c_select > c_table,
        "insert sous <select> classé re-parse : {c_table} -> {c_select}"
    );

    // Rafale 4 : innerHTML banal — NON classé (contrôle :
    // seules les classes risquées tombent).
    std::thread::sleep(std::time::Duration::from_millis(200));
    s.pump_timers(a);
    let c_inner = s.journal_classified();
    assert_eq!(
        c_inner, c_select,
        "innerHTML banal ne doit pas être classé : {c_select} -> {c_inner}"
    );

    // Rafale 5 : SetAttr bénigne — le journal rejoue à
    // nouveau (auto-réparation post-repli).
    std::thread::sleep(std::time::Duration::from_millis(200));
    s.pump_timers(a);
    assert!(
        s.journal_applies() >= 1,
        "la voie différentielle doit se réparer après les replis : \
         applies={} classées={} replis={}",
        s.journal_applies(),
        s.journal_classified(),
        s.journal_fallbacks()
    );
    // Et la page commise reflète tout : le <tr> greffé.
    let png = s.frame_png(a);
    assert!(png.is_some(), "frame produite après les replis");

    // `document.write` post-load — peu importe la voie
    // (classée/repli/appliquée), la page reste cohérente.
    t.route(
        "/w",
        200,
        "<html><body><div>vieux</div><script>\
         setTimeout(function(){\
         document.write('<div>neuf<\\/div>')},60)</script></body></html>",
    );
    let b = s.open_tab();
    s.navigate(b, "http://h.test/w").await.unwrap();
    std::thread::sleep(std::time::Duration::from_millis(120));
    s.pump_timers(b);
    assert!(s.frame_png(b).is_some(), "document.write sans crash");
}

/// Issue #11 — le cycle de vie document est dispatché au
/// commit de navigation : `DOMContentLoaded` puis `load`.
/// Avant le fix, les listeners s'enregistraient mais ne
/// couraient jamais (lobste.rs : timestamps ISO bruts,
/// journal 0/0/0 — le script `application.js` s'évaluait
/// proprement, seul le déclenchement manquait).
#[tokio::test]
async fn lifecycle_domcontentloaded_and_load_dispatch() {
    let (mut s, t) = test_session();
    s.set_js_factory(explorer_js::JscoreEngine::factory());
    // Le listener `load` RELIT le titre posé par
    // `DOMContentLoaded` : la mutation DCL est coalescée
    // (un seul apply en attente par onglet — seul l'état
    // final s'applique), le chaînage rend le passage de
    // DCL observable dans le titre commis.
    t.route(
        "/p",
        200,
        "<html><head><title>orig</title></head><body><script>\
         document.addEventListener('DOMContentLoaded',\
         function(){document.title='dcl'});\
         window.addEventListener('load',\
         function(){document.title=document.title+'+loaded'});\
         </script><div>x</div></body></html>",
    );
    let mut rx = s.subscribe();
    let a = s.open_tab();
    s.navigate(a, "http://h.test/p").await.unwrap();
    // Les mutations des listeners sont enfilées — un drain
    // les applique (voie identique aux mutations script).
    s.pump_net().await;
    let mut changed = Vec::new();
    let mut nav_title = None;
    while let Ok(ev) = rx.try_recv() {
        match ev {
            CoreEvent::TitleChanged { tab, title } if tab == a => changed.push(title),
            CoreEvent::Navigated { tab, title, .. } if tab == a => nav_title = title,
            _ => {}
        }
    }
    assert!(
        changed.iter().any(|t| t == "dcl+loaded"),
        "le titre chaîné prouve DCL puis load : {changed:?}"
    );
    assert_eq!(nav_title.as_deref(), Some("orig"));
    assert_eq!(
        s.tab_info(a).and_then(|i| i.title).as_deref(),
        Some("dcl+loaded")
    );
}

/// ADR-0025 (issue #3) — une mutation **post-baseline** doit
/// être rejouée par le journal, pas repliée en re-parse :
/// le DCL produit le premier journal (repli `NoBaseline`
/// attendu → baseline cléée), puis un timer tardif mute —
/// `journal_applies` passe à 1, les compteurs le prouvent.
/// Le délai du timer doit excéder la durée de `navigate`
/// (sinon la mutation fond dans le journal DCL — mesuré au
/// diagnostic de la sonde d'origine).
#[tokio::test]
async fn journal_post_baseline_rejoue_en_place() {
    let (mut s, t) = test_session();
    s.set_js_factory(explorer_js::JscoreEngine::factory());
    t.route_bin(
        "/",
        200,
        concat!(
            "<!doctype html><html><body><p id=x>avant</p>",
            "<script>document.addEventListener('DOMContentLoaded',function(){",
            "document.getElementById('x').innerText='dcl';})</script>",
            "<script>setTimeout(function(){",
            "document.getElementById('x').innerText='REJ'+'OUE';},2500)</script>",
            "</body></html>"
        )
        .into(),
    );
    let a = s.open_tab();
    s.navigate(a, "http://h.test/").await.unwrap();
    for _ in 0..70 {
        s.pump_net().await;
        std::thread::sleep(std::time::Duration::from_millis(60));
        s.pump_timers(a);
    }
    assert_eq!(
        s.journal_fallbacks(),
        1,
        "un seul repli attendu — le premier journal (NoBaseline)"
    );
    assert_eq!(
        s.journal_applies(),
        1,
        "le journal post-baseline doit rejouer en place"
    );
    assert_eq!(s.journal_classified(), 0, "rien de classé re-parse");
    assert_eq!(s.find_in_page(a, "rejoue"), 1, "mutation commise");
}

/// Issue #6 — l'import map de la page réécrit les bare
/// specifiers : `import 'pkg'` avec `{"pkg":"./real-pkg.js"}`
/// doit fetcher et exécuter `/real-pkg.js`, jamais `/pkg`.
/// Les deux routes sont servies avec des marqueurs distincts :
/// `MAPDEP` = map honorée, `RELDEP` = résolution relative
/// accidentelle (le trou mesuré avant le fix — la map était
/// collectée nulle part : `set_import_map` jamais appelé).
/// Marqueurs éclatés pour que `find` ne matche pas le source.
#[tokio::test]
async fn importmap_reecrit_bare_au_prefetch_et_eval() {
    let (mut s, t) = test_session();
    s.set_js_factory(explorer_js::JscoreEngine::factory());
    t.route(
        "/",
        200,
        concat!(
            "<!doctype html><html><body><p id=m>-</p><p id=d>-</p>",
            "<script type=\"importmap\">{\"imports\":{\"pkg\":\"./real-pkg.js\"}}</script>",
            "<script type=\"module\">import 'pkg';",
            "document.getElementById('m').innerText='ENT'+'RYOK';</script>",
            "</body></html>"
        ),
    );
    t.route_bin(
        "/pkg",
        200,
        b"document.getElementById('d').innerText='REL'+'DEP';".to_vec(),
    );
    t.route_bin(
        "/real-pkg.js",
        200,
        b"document.getElementById('d').innerText='MAP'+'DEP';".to_vec(),
    );
    let a = s.open_tab();
    s.navigate(a, "http://h.test/").await.unwrap();
    for _ in 0..5 {
        s.pump_net().await;
        s.pump_timers(a);
    }
    assert_eq!(
        s.find_in_page(a, "mapdep"),
        1,
        "la cible de la map doit être exécutée"
    );
    assert_eq!(
        s.find_in_page(a, "reldep"),
        0,
        "/pkg ne doit pas être exécuté à la place"
    );
    assert_eq!(s.find_in_page(a, "entryok"), 1, "module d'entrée commis");
    assert!(
        t.paths().iter().all(|p| p != "/pkg"),
        "/pkg ne doit même pas être fetché — chemins : {:?}",
        t.paths()
    );
}

/// Issue #6 bis — un module **importé via bare specifier**
/// s'enregistre chez jscore sous son nom brut (`'pkg'`) : ses
/// propres imports doivent résoudre contre **son** URL
/// résolue, pas planter en « irrésolvable » (mesuré sur
/// github.com : les chunks React importent `react` depuis un
/// module bare-enregistré → `spécificateur irrésolvable` sur
/// l'URL absolue). La clé `alias` est le spec brut.
#[tokio::test]
async fn importmap_dep_bare_resout_ses_propres_imports() {
    let (mut s, t) = test_session();
    s.set_js_factory(explorer_js::JscoreEngine::factory());
    t.route(
        "/",
        200,
        concat!(
            "<!doctype html><html><body><p id=m>-</p><p id=d>-</p>",
            "<script type=\"importmap\">",
            "{\"imports\":{\"pkg\":\"./pkg-real.js\",\"deep\":\"./deep-real.js\"}}",
            "</script>",
            "<script type=\"module\">import 'pkg';",
            "document.getElementById('m').innerText='ENT'+'RYOK';</script>",
            "</body></html>"
        ),
    );
    // `pkg` importe à la fois un relatif (`./sub.js` — base =
    // l'URL résolue de pkg) et un autre bare (`deep`).
    t.route_bin(
        "/pkg-real.js",
        200,
        concat!(
            "import './sub.js';import 'deep';",
            "document.getElementById('d').innerText='PKG'+'OK';"
        )
        .into(),
    );
    t.route_bin(
        "/sub.js",
        200,
        b"document.getElementById('d').innerText='SUB'+'OK';".to_vec(),
    );
    t.route_bin(
        "/deep-real.js",
        200,
        b"document.getElementById('d').innerText='DEP'+'OK';".to_vec(),
    );
    let a = s.open_tab();
    s.navigate(a, "http://h.test/").await.unwrap();
    for _ in 0..5 {
        s.pump_net().await;
        s.pump_timers(a);
    }
    let paths = t.paths();
    assert!(
        paths.iter().any(|p| p == "/sub.js"),
        "le relatif dans un module bare doit résoudre sur sa base — fetches : {paths:?}"
    );
    assert!(
        paths.iter().any(|p| p == "/deep-real.js"),
        "un bare imbriqué suit la map — fetches : {paths:?}"
    );
    assert!(
        paths.iter().all(|p| p != "/pkg" && p != "/deep"),
        "les bare ne doivent jamais se dégénérer en relatif — fetches : {paths:?}"
    );
    assert_eq!(
        s.find_in_page(a, "pkgok"),
        1,
        "deps évaluées avant le parent — #d finit à PKGOK"
    );
    assert_eq!(s.find_in_page(a, "entryok"), 1, "module d'entrée commis");
}

/// ADR-0025 — un `Insert` sous parent « risqué » (table —
/// parsing contextuel) classe le journal non rejouable au
/// drain : `classified` +1 et repli keyed qui livre quand
/// même la mutation (issue #3 — la voie `classified` mesurée
/// en terrain pour la première fois). Le scénario exerce les
/// trois compteurs : `fallbacks` (bootstrap `NoBaseline`),
/// `classified` (insert tabulaire) puis `applies` (`SetText`
/// rejeué en place une fois la baseline posée). Les
/// marqueurs JS sont éclatés pour que `find` ne matche pas
/// la source du `<script>` lui-même.
#[tokio::test]
async fn journal_insert_sous_table_classe_repli_keyed() {
    let (mut s, t) = test_session();
    s.set_js_factory(explorer_js::JscoreEngine::factory());
    let page = concat!(
        "<!doctype html><html><body>",
        "<table><tbody id=\"tb\"><tr><td>r0</td></tr></tbody></table>",
        "<p id=\"p\">txt</p>",
        "<script>",
        "setTimeout(function(){",
        "document.getElementById('p').innerText='JRN'+'AL';",
        "},300);",
        "setTimeout(function(){",
        "var tr=document.createElement('tr');",
        "tr.innerHTML='<td>'+'JR'+'OW'+'</td>';",
        "document.getElementById('tb').appendChild(tr);",
        "},900);",
        "setTimeout(function(){",
        "document.getElementById('p').innerText='JRN'+'AL2';",
        "},1500);",
        "</script></body></html>"
    );
    t.route("/", 200, page);
    let a = s.open_tab();
    s.navigate(a, "http://h.test/").await.unwrap();
    s.pump_net().await;
    // Timer 300 ms — premier journal : bootstrap NoBaseline,
    // le repli keyed livre le SetText et pose la baseline.
    std::thread::sleep(std::time::Duration::from_millis(400));
    s.pump_timers(a);
    assert_eq!(s.journal_fallbacks(), 1, "bootstrap NoBaseline attendu");
    assert_eq!(s.journal_classified(), 0, "SetText traduisible");
    assert_eq!(
        s.find_in_page(a, "jrnal"),
        1,
        "SetText livrée par le repli keyed"
    );
    // Timer 900 ms — `Insert` sous <tbody> : classé non
    // rejouable au drain, la sérialisation keyed subsume.
    std::thread::sleep(std::time::Duration::from_millis(700));
    s.pump_timers(a);
    assert_eq!(
        s.journal_classified(),
        1,
        "insert sous parent risqué → classé"
    );
    assert_eq!(
        s.find_in_page(a, "jrow"),
        1,
        "ligne insérée livrée par le repli keyed"
    );
    // Timer 1500 ms — baseline posée : le journal rejoue.
    std::thread::sleep(std::time::Duration::from_millis(700));
    s.pump_timers(a);
    assert_eq!(s.journal_applies(), 1, "journal post-baseline rejeué");
    assert_eq!(s.find_in_page(a, "jrnal2"), 1, "rejeu commis");
}

// ===== Étape 128 — contextes `<iframe>` (cap « web réel
// sécurisé ») : fetch sous-initiateur + sous-DOM parsé =====

/// Page à un `<iframe src>`.
const PAGE_IFRAME: &str = "<!doctype html><html><body>\
     <iframe src=\"/f\"></iframe><div>parent</div></body></html>";

/// Document enfant servi à l'iframe.
const FRAME_DOC: &str = "<!doctype html><html><head>\
     <title>Widget</title></head><body><p>enfant</p></body></html>";

/// `<iframe src>` = fetch d'un sous-contexte : requête émise,
/// document enfant parsé en son DOM propre, titre extrait.
#[tokio::test]
async fn iframe_src_fetches_subcontext() {
    let (mut s, t) = test_session();
    t.route("/", 200, PAGE_IFRAME);
    t.route("/f", 200, FRAME_DOC);
    let id = s.open_tab();
    s.navigate(id, "http://h.test/").await.unwrap();
    assert!(t.paths().contains(&"/f".to_string()), "iframe fetchée");
    let frames = s.tab_frames(id);
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].0, "http://h.test/f");
    assert_eq!(frames[0].1.as_deref(), Some("Widget"));
    let dump = s.frame_dom_dump(id, 0).unwrap();
    assert!(dump.contains("enfant"), "{dump}");
}

/// La requête iframe porte les métadonnées d'une navigation
/// imbriquée : `sec-fetch-dest: iframe`, `sec-fetch-mode:
/// navigate`, `sec-fetch-site` calculé sur la page parente,
/// `referer` = page.
#[tokio::test]
async fn iframe_fetch_emits_navigate_metadata() {
    let (mut s, t) = test_session();
    t.route("/", 200, PAGE_IFRAME);
    t.route("/f", 200, FRAME_DOC);
    let id = s.open_tab();
    s.navigate(id, "http://h.test/").await.unwrap();
    assert_eq!(
        t.req_header("/f", "sec-fetch-dest").as_deref(),
        Some("iframe")
    );
    assert_eq!(
        t.req_header("/f", "sec-fetch-mode").as_deref(),
        Some("navigate")
    );
    assert_eq!(
        t.req_header("/f", "sec-fetch-site").as_deref(),
        Some("same-origin")
    );
    assert_eq!(
        t.req_header("/f", "referer").as_deref(),
        Some("http://h.test/")
    );
    // Pas de `sec-fetch-user` — pas un geste top-level.
    assert_eq!(t.req_header("/f", "sec-fetch-user"), None);
}

/// `<iframe>` cross-site : `sec-fetch-site: cross-site`,
/// et la politique `SameSite` retient les cookies `Lax` —
/// seuls les `Partitioned` du seau parent voyagent (CHIPS).
#[tokio::test]
async fn iframe_cross_site_samesite_and_partition() {
    use explorer_net::cookies::CookieJar;
    use explorer_net::http::{Initiator, Request};
    use explorer_net::url::Url;

    let (mut s, t) = test_session();
    t.route(
        "/",
        200,
        "<!doctype html><html><body>\
        <iframe src=\"https://cdn.test/f\"></iframe></body></html>",
    );
    // L'enfant cross-site pose un cookie partitionné (le
    // FakeTransport sert par `path`, l'hôte importe peu).
    t.route_headers(
        "/f",
        200,
        FRAME_DOC,
        &[("set-cookie", "chip=1; Partitioned; Secure")],
    );
    let id = s.open_tab();
    s.navigate(id, "https://site-a.test/").await.unwrap();
    assert_eq!(
        t.req_header("/f", "sec-fetch-site").as_deref(),
        Some("cross-site"),
        "site-a.test vs cdn.test"
    );

    // Le cookie `Partitioned` est rangé sous le seau de la
    // page top-level (site-a.test) : il part sur une sous-
    // ressource ultérieure de cdn.test sous site-a, jamais
    // en top-level direct ni sous un autre site.
    let jar = s.cookie_jar();
    let jar: &CookieJar = &jar;
    let cdn = Url::parse("https://cdn.test/f").unwrap();
    let mut req = Request::get(cdn.clone());
    req.initiator = Some(Initiator::Subresource(
        Url::parse("https://site-a.test/").unwrap(),
    ));
    assert!(
        jar.header_for_ctx(&req, true).contains("chip=1"),
        "partition site-a"
    );
    let mut other = Request::get(cdn);
    other.initiator = Some(Initiator::Subresource(
        Url::parse("https://site-b.test/").unwrap(),
    ));
    assert!(
        !jar.header_for_ctx(&other, true).contains("chip=1"),
        "seau site-b vide"
    );
}

/// Un `<iframe src>` en échec (404) ne casse pas la
/// navigation — aucun sous-contexte commis, page vivante.
#[tokio::test]
async fn iframe_404_is_tolerated() {
    let (mut s, t) = test_session();
    t.route("/", 200, PAGE_IFRAME);
    // `/f` non routé → 404 du FakeTransport.
    let id = s.open_tab();
    assert!(s.navigate(id, "http://h.test/").await.is_ok());
    assert!(t.paths().contains(&"/f".to_string()));
    assert_eq!(s.tab_frames(id), Vec::new());
}

/// `max_frames` borne la vague : 2 iframes déclarés, borne
/// à 1 → un seul fetch part.
#[tokio::test]
async fn iframe_bounded_by_max_frames() {
    let (mut s, t) = test_session_with(SessionConfig {
        max_frames: 1,
        ..SessionConfig::default()
    });
    t.route(
        "/",
        200,
        "<!doctype html><html><body>\
         <iframe src=\"/a\"></iframe><iframe src=\"/b\"></iframe>\
         </body></html>",
    );
    t.route("/a", 200, FRAME_DOC);
    t.route("/b", 200, FRAME_DOC);
    let id = s.open_tab();
    s.navigate(id, "http://h.test/").await.unwrap();
    let paths = t.paths();
    assert!(paths.contains(&"/a".to_string()));
    assert!(!paths.contains(&"/b".to_string()), "borne max_frames=1");
    assert_eq!(s.tab_frames(id).len(), 1);
}

/// Les sous-contextes voyagent dans le snapshot bfcache :
/// back/forward restaure les DOM enfants sans re-fetch.
#[tokio::test]
async fn iframe_frames_ride_bfcache() {
    let (mut s, t) = test_session();
    t.route("/", 200, PAGE_IFRAME);
    t.route("/f", 200, FRAME_DOC);
    t.route("/b", 200, PAGE_T);
    let id = s.open_tab();
    s.navigate(id, "http://h.test/").await.unwrap();
    assert_eq!(s.tab_frames(id).len(), 1);
    s.navigate(id, "http://h.test/b").await.unwrap();
    assert_eq!(s.tab_frames(id), Vec::new(), "page sans iframe");
    let calls = t.calls();
    s.go_back(id).await.unwrap();
    assert_eq!(t.calls(), calls, "bfcache : aucun re-fetch");
    assert_eq!(s.tab_frames(id).len(), 1, "frames restaurés");
}
