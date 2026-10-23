// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Tests du routeur (étape 24) — `tower::ServiceExt::oneshot`
//! in-process, aucune socket : `POST /api/tabs`, `navigate`,
//! `GET …/frame` (`image/png`), `GET /api/events` (SSE).

use std::sync::Arc;

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{header, Request as HttpRequest, StatusCode};
use explorer_core::Session;
use explorer_layout::font::{FontRequest, FontSource};
use explorer_net::error::NetResult;
use explorer_net::http::{HttpTransport, Request, Response};
use explorer_test_support::fixture_ttf;
use tower::ServiceExt;

struct FixtureFont;

impl FontSource for FixtureFont {
    fn font_data(&self, _req: &FontRequest) -> Option<Vec<u8>> {
        Some(fixture_ttf())
    }
}

struct FakeTransport {
    body: &'static str,
    /// Journal `(méthode chemin)` — la suite reload POST
    /// vérifie qu'aucune resoumission silencieuse n'a fuité.
    log: std::sync::Mutex<Vec<String>>,
}

#[async_trait]
impl HttpTransport for FakeTransport {
    async fn fetch(&self, req: &Request) -> NetResult<Response> {
        self.log
            .lock()
            .unwrap()
            .push(format!("{} {}", req.method.as_str(), req.url.path()));
        let body = match req.url.path() {
            "/form" => {
                "<!doctype html><html><body>\
             <form method=\"post\" action=\"/doit\">\
             <button name=\"b\" value=\"1\">Go</button>\
             </form></body></html>"
            }
            "/link" => {
                "<!doctype html><html><body>\
             <a href=\"/b\">lien</a></body></html>"
            }
            "/media" => {
                "<!doctype html><html><body>\
             <video src=\"v.mp4\" controls></video>\
             <audio src=\"a.mp3\"></audio></body></html>"
            }
            _ => self.body,
        };
        Ok(Response {
            status: 200,
            reason: "fake".to_string(),
            headers: vec![],
            body: body.as_bytes().to_vec(),
            reused_connection: false,
            resolved_url: None,
        })
    }
}

/// Routeur câblé sur une session de test.
fn test_app() -> axum::Router {
    let t: Arc<dyn HttpTransport> = Arc::new(FakeTransport {
        body: "<!doctype html><html><body><div>api</div></body></html>",
        log: std::sync::Mutex::new(Vec::new()),
    });
    let session = Arc::new(tokio::sync::Mutex::new(Session::new(
        t,
        Arc::new(FixtureFont),
    )));
    explorer_api::router(session)
}

fn json_post(uri: &str, body: &str) -> HttpRequest<Body> {
    HttpRequest::builder()
        .method("POST")
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

#[tokio::test]
async fn open_tab_then_frame_is_png() {
    let app = test_app();
    let resp = app
        .clone()
        .oneshot(json_post("/api/tabs", "{}"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let bytes = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let id = v["id"].as_u64().unwrap();
    assert!(id >= 1);

    // Navigation.
    let nav = app
        .clone()
        .oneshot(json_post(
            &format!("/api/tabs/{id}/navigate"),
            r#"{"url":"http://h.test/"}"#,
        ))
        .await
        .unwrap();
    assert_eq!(nav.status(), StatusCode::OK);

    // Frame → PNG.
    let frame = app
        .oneshot(
            HttpRequest::builder()
                .uri(format!("/api/tabs/{id}/frame"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(frame.status(), StatusCode::OK);
    assert_eq!(
        frame.headers().get(header::CONTENT_TYPE).unwrap(),
        "image/png"
    );
    let png = axum::body::to_bytes(frame.into_body(), 8 * 1024 * 1024)
        .await
        .unwrap();
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
}

/// Étape 120 — `GET …/dom` et `GET …/layout` : dumps
/// diagnostiques du document vivant (triage « clic ignoré »
/// — consent google : hit-test `form` vs `<input>` peint).
#[tokio::test]
async fn dom_and_layout_dumps_after_navigate() {
    let app = test_app();
    let resp = app
        .clone()
        .oneshot(json_post("/api/tabs", "{}"))
        .await
        .unwrap();
    let bytes = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
    let id = serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()["id"]
        .as_u64()
        .unwrap();

    // Onglet vierge : ni DOM ni layout → 404 honnête.
    for sub in ["dom", "layout"] {
        let r = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .uri(format!("/api/tabs/{id}/{sub}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::NOT_FOUND, "{sub} vierge");
    }

    app.clone()
        .oneshot(json_post(
            &format!("/api/tabs/{id}/navigate"),
            r#"{"url":"http://h.test/"}"#,
        ))
        .await
        .unwrap();

    let dom = app
        .clone()
        .oneshot(
            HttpRequest::builder()
                .uri(format!("/api/tabs/{id}/dom"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(dom.status(), StatusCode::OK);
    let d = axum::body::to_bytes(dom.into_body(), 1 << 20)
        .await
        .unwrap();
    let d = String::from_utf8_lossy(&d);
    assert!(d.contains("<html>"), "dom dump : {d}");
    assert!(d.contains("api"), "contenu : {d}");

    let lay = app
        .oneshot(
            HttpRequest::builder()
                .uri(format!("/api/tabs/{id}/layout"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(lay.status(), StatusCode::OK);
    let l = axum::body::to_bytes(lay.into_body(), 1 << 20)
        .await
        .unwrap();
    let l = String::from_utf8_lossy(&l);
    assert!(l.contains("<div>"), "layout dump : {l}");
}

/// Étape 97 — `POST …/color-scheme` propage le réglage
/// `dark`/`light` à la session (`prefers-color-scheme` +
/// défauts UA) ; valeur hors contrat → 400.
#[tokio::test]
async fn color_scheme_accepts_dark_rejects_junk() {
    let app = test_app();
    let resp = app
        .clone()
        .oneshot(json_post("/api/tabs", "{}"))
        .await
        .unwrap();
    let bytes = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let id = v["id"].as_u64().unwrap();

    let dark = app
        .clone()
        .oneshot(json_post(
            &format!("/api/tabs/{id}/color-scheme"),
            r#"{"scheme":"dark"}"#,
        ))
        .await
        .unwrap();
    assert_eq!(dark.status(), StatusCode::OK);

    let junk = app
        .oneshot(json_post(
            &format!("/api/tabs/{id}/color-scheme"),
            r#"{"scheme":"solarized"}"#,
        ))
        .await
        .unwrap();
    assert_eq!(junk.status(), StatusCode::BAD_REQUEST);
}

/// Étape 118 — `POST /api/accept-language` règle la valeur
/// session (reflétée dans `/api/status`), chaîne vide lève
/// l'en-tête, corps sans `lang` → 400.
#[tokio::test]
async fn accept_language_roundtrip_and_clear() {
    let app = test_app();

    let ok = app
        .clone()
        .oneshot(json_post(
            "/api/accept-language",
            r#"{"lang":"fr-FR,fr;q=0.9"}"#,
        ))
        .await
        .unwrap();
    assert_eq!(ok.status(), StatusCode::OK);

    let st = app
        .clone()
        .oneshot(
            HttpRequest::builder()
                .uri("/api/status")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = axum::body::to_bytes(st.into_body(), 1024).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(v["accept_language"], "fr-FR,fr;q=0.9");

    let clear = app
        .clone()
        .oneshot(json_post("/api/accept-language", r#"{"lang":""}"#))
        .await
        .unwrap();
    assert_eq!(clear.status(), StatusCode::OK);
    let st = app
        .clone()
        .oneshot(
            HttpRequest::builder()
                .uri("/api/status")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = axum::body::to_bytes(st.into_body(), 1024).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert!(v["accept_language"].is_null());

    let junk = app
        .oneshot(json_post("/api/accept-language", r#"{"lang":42}"#))
        .await
        .unwrap();
    assert_eq!(junk.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn frame_before_navigate_is_no_content() {
    let app = test_app();
    let resp = app
        .clone()
        .oneshot(json_post("/api/tabs", "{}"))
        .await
        .unwrap();
    let bytes = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let id = v["id"].as_u64().unwrap();

    let frame = app
        .oneshot(
            HttpRequest::builder()
                .uri(format!("/api/tabs/{id}/frame"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(frame.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn events_endpoint_is_sse() {
    let app = test_app();
    let resp = app
        .oneshot(
            HttpRequest::builder()
                .uri("/api/events")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        resp.headers()
            .get(header::CONTENT_TYPE)
            .unwrap()
            .to_str()
            .unwrap(),
        "text/event-stream"
    );
    // Le corps est un stream infini — la tête suffit à
    // prouver le contrat ; on ne consomme pas la suite.
}

#[tokio::test]
async fn navigate_bad_body_is_400() {
    let app = test_app();
    let resp = app
        .oneshot(json_post("/api/tabs/1/navigate", "{}"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let bytes = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    // Format d'erreur du contrat ADR-0003.
    assert_eq!(v["error"]["handled"], true);
    assert!(v["error"]["message"].is_string());
}

#[tokio::test]
async fn status_lists_engine_and_tabs() {
    let app = test_app();
    let resp = app
        .clone()
        .oneshot(
            HttpRequest::builder()
                .uri("/api/status")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(v["engine"], "explorer");
    assert_eq!(v["tabs"], serde_json::json!([]));
}

/// `GET /api/egress` (étape 72) — le mode de résolution
/// configuré est observable ; `system` par défaut.
#[tokio::test]
async fn egress_reports_resolver_mode() {
    let app = test_app();
    let resp = app
        .clone()
        .oneshot(
            HttpRequest::builder()
                .uri("/api/egress")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(v["resolver"], "system");
}

#[tokio::test]
async fn tabs_crud_contract() {
    let app = test_app();
    // Crée.
    let resp = app
        .clone()
        .oneshot(json_post("/api/tabs", "{}"))
        .await
        .unwrap();
    let bytes = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let id = v["id"].as_u64().unwrap();

    // Liste.
    let resp = app
        .clone()
        .oneshot(
            HttpRequest::builder()
                .uri("/api/tabs")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(v["tabs"].as_array().unwrap().len(), 1);
    assert_eq!(v["tabs"][0]["id"], id);

    // Supprime.
    let resp = app
        .clone()
        .oneshot(
            HttpRequest::builder()
                .method("DELETE")
                .uri(format!("/api/tabs/{id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    // Inconnu → 404 avec le format d'erreur.
    let resp = app
        .oneshot(
            HttpRequest::builder()
                .method("DELETE")
                .uri(format!("/api/tabs/{id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    let bytes = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(v["error"]["handled"], true);
}

#[tokio::test]
async fn navigate_then_back_forward_reload() {
    let app = test_app();
    let resp = app
        .clone()
        .oneshot(json_post("/api/tabs", "{}"))
        .await
        .unwrap();
    let bytes = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
    let id = serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()["id"]
        .as_u64()
        .unwrap();

    for url in ["http://h.test/a", "http://h.test/b"] {
        let r = app
            .clone()
            .oneshot(json_post(
                &format!("/api/tabs/{id}/navigate"),
                &format!(r#"{{"url":"{url}"}}"#),
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::OK);
    }
    // back → /a, forward → /b, reload → /b.
    for (route, want) in [
        ("back", "http://h.test/a"),
        ("forward", "http://h.test/b"),
        ("reload", "http://h.test/b"),
    ] {
        let r = app
            .clone()
            .oneshot(json_post(&format!("/api/tabs/{id}/{route}"), "{}"))
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::OK, "{route}");
        let bytes = axum::body::to_bytes(r.into_body(), 4096).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["url"], want, "{route}");
    }
}

/// Étape 93 — `reload` réémet la requête d'origine (méthode
/// + corps), pas un `GET` vide ; une méthode non sûre exige
/// `{"confirm":true}` — sinon statut typé `resubmit_confirm`
/// sans qu'aucune resoumission ne parte.
#[tokio::test]
async fn post_reload_requires_confirm() {
    let transport = Arc::new(FakeTransport {
        body: "<!doctype html><html><body><div>api</div></body></html>",
        log: std::sync::Mutex::new(Vec::new()),
    });
    let session = Arc::new(tokio::sync::Mutex::new(Session::new(
        transport.clone(),
        Arc::new(FixtureFont),
    )));
    let app = explorer_api::router(session);

    let resp = app
        .clone()
        .oneshot(json_post("/api/tabs", "{}"))
        .await
        .unwrap();
    let bytes = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
    let id = serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()["id"]
        .as_u64()
        .unwrap();

    // Page formulaire → clic sur le bouton → POST /doit.
    app.clone()
        .oneshot(json_post(
            &format!("/api/tabs/{id}/navigate"),
            r#"{"url":"http://h.test/form"}"#,
        ))
        .await
        .unwrap();
    let r = app
        .clone()
        .oneshot(json_post(
            &format!("/api/tabs/{id}/input"),
            r#"{"type":"click","x":15,"y":15}"#,
        ))
        .await
        .unwrap();
    let bytes = axum::body::to_bytes(r.into_body(), 4096).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(v["result"], "navigated", "{v}");
    // Le endpoint `input` est feu-et-oublie (étape 73) : la
    // nav de soumission tourne en tâche détachée — attendre
    // le POST dans le journal transport.
    for _ in 0..50 {
        if transport
            .log
            .lock()
            .unwrap()
            .iter()
            .any(|l| l == "POST /doit")
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert_eq!(
        transport.log.lock().unwrap().last().unwrap(),
        "POST /doit",
        "le submit émet le POST"
    );

    // reload sans confirm → resubmit_confirm, RIEN ne repart.
    let r = app
        .clone()
        .oneshot(json_post(&format!("/api/tabs/{id}/reload"), "{}"))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(r.into_body(), 4096).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(v["result"], "resubmit_confirm", "{v}");
    assert_eq!(v["method"], "POST");
    assert_eq!(
        transport
            .log
            .lock()
            .unwrap()
            .iter()
            .filter(|l| *l == "POST /doit")
            .count(),
        1,
        "aucune resoumission silencieuse"
    );

    // reload {"confirm":true} → le POST repart.
    let r = app
        .clone()
        .oneshot(json_post(
            &format!("/api/tabs/{id}/reload"),
            r#"{"confirm":true}"#,
        ))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(r.into_body(), 4096).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(v["url"], "http://h.test/doit", "{v}");
    assert_eq!(
        transport
            .log
            .lock()
            .unwrap()
            .iter()
            .filter(|l| *l == "POST /doit")
            .count(),
        2,
        "le POST confirmé repart"
    );
}

#[tokio::test]
async fn input_click_link_navigates() {
    let app = test_app();
    let resp = app
        .clone()
        .oneshot(json_post("/api/tabs", "{}"))
        .await
        .unwrap();
    let bytes = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
    let id = serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()["id"]
        .as_u64()
        .unwrap();
    app.clone()
        .oneshot(json_post(
            &format!("/api/tabs/{id}/navigate"),
            r#"{"url":"http://h.test/link"}"#,
        ))
        .await
        .unwrap();

    // Clic sur le lien → navigated. La feuille UA pose
    // `body { margin: 8px }` — le lien démarre à (8, 9.6) :
    // (15, 15) tombe dans sa boîte, (5, 5) resterait dans la
    // marge et serait ignoré.
    let r = app
        .clone()
        .oneshot(json_post(
            &format!("/api/tabs/{id}/input"),
            r#"{"type":"click","x":15,"y":15}"#,
        ))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(r.into_body(), 4096).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(v["result"], "navigated");

    // Scroll → scrolled + scroll_y.
    let r = app
        .oneshot(json_post(
            &format!("/api/tabs/{id}/input"),
            r#"{"type":"scroll","dy":50}"#,
        ))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(r.into_body(), 4096).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(v["result"], "scrolled");
    assert!(v["scroll_y"].is_number());
}

#[tokio::test]
async fn api_key_gates_routes() {
    let t: Arc<dyn HttpTransport> = Arc::new(FakeTransport {
        body: "x",
        log: std::sync::Mutex::new(Vec::new()),
    });
    let session = Arc::new(tokio::sync::Mutex::new(Session::new(
        t,
        Arc::new(FixtureFont),
    )));
    let app = explorer_api::router_with_key(session, Some("s3cret".to_string()));

    // Sans clé → 401 au format du contrat.
    let r = app
        .clone()
        .oneshot(
            HttpRequest::builder()
                .uri("/api/status")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
    let bytes = axum::body::to_bytes(r.into_body(), 1024).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(v["error"]["handled"], true);

    // En-tête x-api-key → OK.
    let r = app
        .clone()
        .oneshot(
            HttpRequest::builder()
                .uri("/api/status")
                .header("x-api-key", "s3cret")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);

    // ?key= en repli (EventSource).
    let r = app
        .oneshot(
            HttpRequest::builder()
                .uri("/api/status?key=s3cret")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
}

#[tokio::test]
async fn bookmarks_and_history_endpoints() {
    let app = test_app();
    // Signet : ajout → liste → conflit → retrait → 404.
    let r = app
        .clone()
        .oneshot(json_post(
            "/api/bookmarks",
            r#"{"url":"http://h.test/","title":"T"}"#,
        ))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::CREATED);

    let r = app
        .clone()
        .oneshot(
            HttpRequest::builder()
                .uri("/api/bookmarks")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = axum::body::to_bytes(r.into_body(), 4096).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(v["bookmarks"][0]["url"], "http://h.test/");
    assert_eq!(v["bookmarks"][0]["title"], "T");

    let r = app
        .clone()
        .oneshot(json_post("/api/bookmarks", r#"{"url":"http://h.test/"}"#))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::CONFLICT);

    let r = app
        .clone()
        .oneshot(
            HttpRequest::builder()
                .method("DELETE")
                .uri("/api/bookmarks")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"url":"http://h.test/"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::NO_CONTENT);

    let r = app
        .clone()
        .oneshot(
            HttpRequest::builder()
                .method("DELETE")
                .uri("/api/bookmarks")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"url":"http://h.test/"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::NOT_FOUND);

    // Historique : vide sans store de profil.
    let r = app
        .oneshot(
            HttpRequest::builder()
                .uri("/api/history")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = axum::body::to_bytes(r.into_body(), 4096).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(v["history"], serde_json::json!([]));
}

// -- Étape 70 : téléchargements REST ----------------------------------

/// Transport octet-stream — toute URL sert le même blob
/// marqué `attachment`.
struct DlTransport;

#[async_trait]
impl HttpTransport for DlTransport {
    async fn fetch(&self, _req: &Request) -> NetResult<Response> {
        Ok(Response {
            status: 200,
            reason: "fake".to_string(),
            headers: vec![
                (
                    "Content-Type".to_string(),
                    "application/octet-stream".to_string(),
                ),
                (
                    "Content-Disposition".to_string(),
                    "attachment; filename=\"r.bin\"".to_string(),
                ),
            ],
            body: b"\x01\x02\x03blob".to_vec(),
            reused_connection: false,
            resolved_url: None,
        })
    }
}

#[tokio::test]
async fn download_accept_writes_body() {
    let session = Arc::new(tokio::sync::Mutex::new(Session::new(
        Arc::new(DlTransport),
        Arc::new(FixtureFont),
    )));
    let app = explorer_api::router(session);
    let r = app
        .clone()
        .oneshot(json_post("/api/tabs", "{}"))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::CREATED);
    let r = app
        .clone()
        .oneshot(json_post(
            "/api/tabs/1/navigate",
            r#"{"url":"http://h.test/r.bin"}"#,
        ))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    // L'offre est enregistrée.
    let r = app
        .clone()
        .oneshot(
            HttpRequest::builder()
                .uri("/api/tabs/1/downloads")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = axum::body::to_bytes(r.into_body(), 4096).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(v["downloads"][0]["filename"], "r.bin");
    assert_eq!(v["downloads"][0]["size"], 7);
    let dl = v["downloads"][0]["id"].as_u64().unwrap();
    // Acceptation avec chemin explicite → corps écrit.
    let dir = std::env::temp_dir().join("explorer-api-dl");
    let target = dir.join("r.bin");
    // Chemin POSIX : JSON sans échappement backslash.
    let posix = target.display().to_string().replace('\\', "/");
    let body = format!(r#"{{"path":"{posix}"}}"#);
    let r = app
        .clone()
        .oneshot(json_post(
            &format!("/api/tabs/1/downloads/{dl}/accept"),
            &body,
        ))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    assert_eq!(std::fs::read(&target).unwrap(), b"\x01\x02\x03blob");
    // Le pending est consommé.
    let r = app
        .oneshot(
            HttpRequest::builder()
                .uri("/api/tabs/1/downloads")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = axum::body::to_bytes(r.into_body(), 4096).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(v["downloads"], serde_json::json!([]));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Onglet ouvert + page `/media` commise (video controls +
/// audio nu) — helper commun des tests `MediaSurface` 71b.
async fn media_tab() -> (axum::Router, u64) {
    let app = test_app();
    let resp = app
        .clone()
        .oneshot(json_post("/api/tabs", "{}"))
        .await
        .unwrap();
    let bytes = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let id = v["id"].as_u64().unwrap();
    let nav = app
        .clone()
        .oneshot(json_post(
            &format!("/api/tabs/{id}/navigate"),
            r#"{"url":"http://h.test/media"}"#,
        ))
        .await
        .unwrap();
    assert_eq!(nav.status(), StatusCode::OK);
    (app, id)
}

/// `GET …/media` d'un onglet — JSON brut de la liste ou de
/// la fiche (`uri` = `""` ou `"/{mid}"`).
async fn media_get(app: &axum::Router, id: u64, uri: &str) -> serde_json::Value {
    let r = app
        .clone()
        .oneshot(
            HttpRequest::builder()
                .uri(format!("/api/tabs/{id}/media{uri}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = axum::body::to_bytes(r.into_body(), 16 * 1024)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

/// Étape 71b — seam `MediaSurface` REST : les éléments
/// média lisibles sont listés (video + audio 0×0) et un
/// id mort répond `410`.
#[tokio::test]
async fn media_offers_listed_and_gone() {
    let (app, id) = media_tab().await;
    let v = media_get(&app, id, "").await;
    let media = v["media"].as_array().unwrap();
    assert_eq!(media.len(), 2, "video + audio offerts : {v}");
    assert_eq!(media[0]["kind"], "video");
    assert_eq!(media[0]["url"], "http://h.test/v.mp4");
    assert_eq!(media[0]["mime"], "video/mp4");
    assert_eq!(media[0]["controls"], true);
    // audio sans controls → rect 0×0 (pipeline seul).
    assert_eq!(media[1]["kind"], "audio");
    assert_eq!(media[1]["rect"]["w"], 0.0);

    // Id mort → 410 partout.
    let dead = app
        .clone()
        .oneshot(json_post(
            &format!("/api/tabs/{id}/media/999/claim"),
            r#"{"codecs":[]}"#,
        ))
        .await
        .unwrap();
    assert_eq!(dead.status(), StatusCode::GONE);
    let dead = app
        .oneshot(
            HttpRequest::builder()
                .uri(format!("/api/tabs/{id}/media/999"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(dead.status(), StatusCode::GONE);
}

/// Étape 71b — cycle claim/state/release : premier arrivé
/// gagne (`409`), `state` exige le claim, release relâche.
#[tokio::test]
async fn media_claim_state_release() {
    let (app, id) = media_tab().await;
    let v = media_get(&app, id, "").await;
    let mid = v["media"][0]["id"].as_u64().unwrap();

    // `state` sans claim → 409 ; claim → 200 ; second → 409.
    let st = app
        .clone()
        .oneshot(json_post(
            &format!("/api/tabs/{id}/media/{mid}/state"),
            r#"{"state":"playing","position_ms":0,"buffered_ms":0}"#,
        ))
        .await
        .unwrap();
    assert_eq!(st.status(), StatusCode::CONFLICT);
    let cl = app
        .clone()
        .oneshot(json_post(
            &format!("/api/tabs/{id}/media/{mid}/claim"),
            r#"{"codecs":["h264","aac"]}"#,
        ))
        .await
        .unwrap();
    assert_eq!(cl.status(), StatusCode::OK);
    let cl2 = app
        .clone()
        .oneshot(json_post(
            &format!("/api/tabs/{id}/media/{mid}/claim"),
            r#"{"codecs":["vp9"]}"#,
        ))
        .await
        .unwrap();
    assert_eq!(cl2.status(), StatusCode::CONFLICT, "premier arrivé gagne");

    // `state` claimé → 200 et visible dans la fiche GET.
    let st = app
        .clone()
        .oneshot(json_post(
            &format!("/api/tabs/{id}/media/{mid}/state"),
            r#"{"state":"playing","position_ms":1200,"duration_ms":60000,"buffered_ms":4000}"#,
        ))
        .await
        .unwrap();
    assert_eq!(st.status(), StatusCode::OK);
    let v = media_get(&app, id, &format!("/{mid}")).await;
    assert_eq!(v["claimed"], true);
    assert_eq!(v["state"]["position_ms"], 1200);
    assert_eq!(v["state"]["state"], "playing");

    // Release → la surface retombe sur les stubs ADR-0023.
    let rel = app
        .clone()
        .oneshot(json_post(
            &format!("/api/tabs/{id}/media/{mid}/release"),
            "{}",
        ))
        .await
        .unwrap();
    assert_eq!(rel.status(), StatusCode::OK);
    let v = media_get(&app, id, &format!("/{mid}")).await;
    assert_eq!(v["claimed"], false);
}
