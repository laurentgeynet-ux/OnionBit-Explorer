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
}

#[async_trait]
impl HttpTransport for FakeTransport {
    async fn fetch(&self, req: &Request) -> NetResult<Response> {
        let body = if req.url.path() == "/link" {
            "<!doctype html><html><body>\
             <a href=\"/b\">lien</a></body></html>"
        } else {
            self.body
        };
        Ok(Response {
            status: 200,
            reason: "fake".to_string(),
            headers: vec![],
            body: body.as_bytes().to_vec(),
            reused_connection: false,
        })
    }
}

/// Routeur câblé sur une session de test.
fn test_app() -> axum::Router {
    let t: Arc<dyn HttpTransport> = Arc::new(FakeTransport {
        body: "<!doctype html><html><body><div>api</div></body></html>",
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

    // Clic sur le lien → navigated.
    let r = app
        .clone()
        .oneshot(json_post(
            &format!("/api/tabs/{id}/input"),
            r#"{"type":"click","x":5,"y":5}"#,
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
    let t: Arc<dyn HttpTransport> = Arc::new(FakeTransport { body: "x" });
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
