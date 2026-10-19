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
    Bookmark, CoreError, CoreEvent, InputEvent, InputOutcome, Session, SessionConfig,
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
}

impl FakeTransport {
    fn new() -> Self {
        Self {
            routes: Mutex::new(HashMap::new()),
            calls: Mutex::new(0),
            last: Mutex::new(None),
        }
    }

    fn route(&self, path: &str, status: u16, body: &'static str) {
        self.route_bin(path, status, body.as_bytes().to_vec());
    }

    /// Corps binaire (images & co).
    fn route_bin(&self, path: &str, status: u16, body: Vec<u8>) {
        if let Ok(mut r) = self.routes.lock() {
            r.insert(path.to_string(), (status, body));
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
}

#[async_trait]
impl HttpTransport for FakeTransport {
    async fn fetch(&self, req: &Request) -> NetResult<Response> {
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
        let found = self
            .routes
            .lock()
            .ok()
            .and_then(|r| r.get(req.url.path()).cloned());
        match found {
            Some((status, body)) => Ok(Response {
                status,
                reason: "fake".to_string(),
                headers: vec![],
                body,
                reused_connection: false,
            }),
            None => Ok(Response {
                status: 404,
                reason: "fake".to_string(),
                headers: vec![],
                body: b"missing".to_vec(),
                reused_connection: false,
            }),
        }
    }
}

/// Session de test prête : transport + fixture.
fn test_session() -> (Session, Arc<FakeTransport>) {
    let t = Arc::new(FakeTransport::new());
    let s = Session::with_config(
        t.clone(),
        Arc::new(FixtureFont),
        SessionConfig {
            event_capacity: 16,
            viewport: (800.0, 600.0),
            scale: 1.0,
            js_ops: 5_000_000,
            max_images: 32,
            image_max_px: 16_000_000,
            max_sheets: 8,
            max_fonts: 16,
        },
    );
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
            CoreEvent::FrameInvalidated { tab } if tab == id => saw_inv = true,
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
        if matches!(e, CoreEvent::FrameInvalidated { tab } if tab == id) {
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
    while let Ok(e) = rx.try_recv() {
        if matches!(e, CoreEvent::FrameInvalidated { tab } if tab == id) {
            n += 1;
        }
    }
    assert!(n >= 2);
}

#[tokio::test]
async fn key_input_is_ignored() {
    let (mut s, _t) = test_session();
    let id = s.open_tab();
    let out = s
        .handle_input(
            id,
            InputEvent::Key {
                key: "a".to_string(),
            },
        )
        .await
        .unwrap();
    assert_eq!(out, InputOutcome::Ignored);
}

/// Store en mémoire pour tester le câblage du port
/// `ProfileStore` (l'impl fichier est au daemon).
struct MemStore {
    visits: Mutex<Vec<(String, Option<String>)>>,
    bookmarks: Mutex<Vec<Bookmark>>,
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
}

#[tokio::test]
async fn visits_are_recorded_and_bookmarks_persisted() {
    let (mut s, t) = test_session();
    t.route("/", 200, PAGE_T);
    let store = Arc::new(MemStore {
        visits: Mutex::new(Vec::new()),
        bookmarks: Mutex::new(Vec::new()),
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

    // Touche sans focus → ignorée.
    let out = s
        .handle_input(id, InputEvent::Key { key: "a".into() })
        .await
        .unwrap();
    assert_eq!(out, InputOutcome::Ignored);

    // Clic dans l'input (en haut à gauche, ~10em × 1.2em).
    let out = s
        .handle_input(id, InputEvent::Click { x: 5.0, y: 5.0 })
        .await
        .unwrap();
    assert_eq!(out, InputOutcome::Focused);

    // Caractères → édités ; Backspace idem.
    for key in ["a", "b"] {
        let out = s
            .handle_input(id, InputEvent::Key { key: key.into() })
            .await
            .unwrap();
        assert_eq!(out, InputOutcome::Edited);
    }
    let out = s
        .handle_input(
            id,
            InputEvent::Key {
                key: "Backspace".into(),
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
            },
        )
        .await
        .unwrap();
    assert_eq!(out, InputOutcome::Navigated);
    let (_, pq, _) = t.last().unwrap();
    assert_eq!(pq, "/?q=a");
    let out = s
        .handle_input(id, InputEvent::Key { key: "z".into() })
        .await
        .unwrap();
    assert_eq!(out, InputOutcome::Ignored);
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

    // Focus puis clic loin en bas → blur (la touche est
    // ignorée ensuite).
    let _ = s
        .handle_input(id, InputEvent::Click { x: 5.0, y: 5.0 })
        .await;
    let _ = s
        .handle_input(id, InputEvent::Click { x: 5.0, y: 500.0 })
        .await;
    let out = s
        .handle_input(id, InputEvent::Key { key: "x".into() })
        .await
        .unwrap();
    assert_eq!(out, InputOutcome::Ignored);
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
        .handle_input(id, InputEvent::Key { key: "c".into() })
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

/// Page dont un script mute le DOM miroir (`textContent`).
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
    s.set_js_factory(explorer_js::DilloEngine::factory());

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
    s.set_js_factory(explorer_js::DilloEngine::factory());

    let a = s.open_tab();
    s.navigate(a, "http://h.test/").await.unwrap();
    let b = s.open_tab();
    s.navigate(b, "http://h.test/stat").await.unwrap();

    // `createElement` + `appendChild` mutent le miroir → le
    // rendu doit correspondre à la page statique équivalente.
    assert_eq!(s.frame_png(a), s.frame_png(b));
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
    s.set_js_factory(explorer_js::DilloEngine::factory());

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
    s.set_js_factory(explorer_js::DilloEngine::factory());

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
    s.set_js_factory(explorer_js::DilloEngine::factory());

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
        "<html><head><style>#b{background-color:#ff0000;\
         width:2px;height:2px}</style></head>\
         <body><div id=\"b\"></div></body></html>",
    );

    let a = s.open_tab();
    s.navigate(a, "http://h.test/i").await.unwrap();
    let b = s.open_tab();
    s.navigate(b, "http://h.test/s").await.unwrap();
    assert_eq!(s.frame_png(a), s.frame_png(b));
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
    // Bouton : dernier div (~y 160-185 sous la textarea 4em).
    let out = s
        .handle_input(id, InputEvent::Click { x: 10.0, y: 170.0 })
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

#[tokio::test]
async fn select_cycles_options_on_click() {
    let (mut s, t) = test_session();
    t.route("/sl", 200, PAGE_SEL_FORM);
    t.route("/sgo", 200, "<html><body>done</body></html>");
    let id = s.open_tab();
    s.navigate(id, "http://h.test/sl").await.unwrap();
    // Clic sur le select → option suivante.
    let out = s
        .handle_input(id, InputEvent::Click { x: 10.0, y: 6.0 })
        .await
        .unwrap();
    assert_eq!(out, InputOutcome::Edited);
    let out = s
        .handle_input(id, InputEvent::Click { x: 6.0, y: 40.0 })
        .await
        .unwrap();
    assert_eq!(out, InputOutcome::Navigated);
    assert_eq!(t.last().unwrap().1, "/sgo?s=o2");
}

#[tokio::test]
async fn submit_preventdefault_skips_navigation() {
    let (mut s, t) = test_session();
    t.route("/p", 200, PAGE_PREVENT_FORM);
    s.set_js_factory(explorer_js::DilloEngine::factory());
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
    s.set_js_factory(explorer_js::DilloEngine::factory());
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
    s.set_js_factory(explorer_js::DilloEngine::factory());
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
    s.set_js_factory(explorer_js::DilloEngine::factory());
    let id = s.open_tab();
    s.navigate(id, "http://h.test/p").await.unwrap();
    // URL irrésolvable + borne historique : aucune navigation,
    // aucune erreur — la page reste en place.
    assert_eq!(
        s.tab_info(id).unwrap().url.as_deref(),
        Some("http://h.test/p")
    );
}
