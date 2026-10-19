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
use explorer_core::{CoreError, CoreEvent, InputEvent, InputOutcome, Session, SessionConfig};
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
    routes: Mutex<HashMap<String, (u16, &'static str)>>,
    calls: Mutex<u32>,
}

impl FakeTransport {
    fn new() -> Self {
        Self {
            routes: Mutex::new(HashMap::new()),
            calls: Mutex::new(0),
        }
    }

    fn route(&self, path: &str, status: u16, body: &'static str) {
        if let Ok(mut r) = self.routes.lock() {
            r.insert(path.to_string(), (status, body));
        }
    }
}

#[async_trait]
impl HttpTransport for FakeTransport {
    async fn fetch(&self, req: &Request) -> NetResult<Response> {
        if let Ok(mut c) = self.calls.lock() {
            *c += 1;
        }
        let found = self
            .routes
            .lock()
            .ok()
            .and_then(|r| r.get(req.url.path()).copied());
        match found {
            Some((status, body)) => Ok(Response {
                status,
                reason: "fake".to_string(),
                headers: vec![],
                body: body.as_bytes().to_vec(),
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
            url: Some("http://h.test/b".to_string())
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
