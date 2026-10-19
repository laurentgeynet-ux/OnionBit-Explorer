// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! `explorer-api` — API REST + SSE (`axum`) exposée sur loopback :
//! routes, DTO, SSE, clé optionnelle (ADR-0003/0004).
//!
//! Étape 24 : `GET /api/tabs/{id}/frame` sert le PNG de
//! l'onglet, `GET /api/events` diffuse `frame_invalidated` et
//! les événements de session en SSE, `POST /api/tabs`,
//! `POST /api/tabs/{id}/navigate`, `POST /api/tabs/{id}/viewport`.
//! Le contrat complet (onglets CRUD, historique, input) est
//! l'étape 25.
//!
//! Les DTO ne fuient jamais dans `explorer-core`. Session
//! sérialisée par `Mutex` (single-writer : le DOM/layout ne
//! sort jamais de ce flux).

#![forbid(unsafe_code)]

use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Path, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use explorer_core::{CoreEvent, Session, TabId};
use futures_util::Stream;

/// État partagé : la session moteur, sérialisée.
#[derive(Clone)]
pub struct ApiState {
    /// Session — `tokio::Mutex` : la garde survit aux `.await`
    /// de `navigate` (future `Send`, single-writer strict).
    pub session: Arc<tokio::sync::Mutex<Session>>,
}

/// Construit le routeur de l'API. `session` est injectée par
/// le daemon — l'API ne crée jamais de moteur.
pub fn router(session: Arc<tokio::sync::Mutex<Session>>) -> Router {
    Router::new()
        .route("/api/tabs", post(open_tab))
        .route("/api/tabs/{id}/frame", get(frame))
        .route("/api/tabs/{id}/navigate", post(navigate))
        .route("/api/tabs/{id}/viewport", post(viewport))
        .route("/api/events", get(events))
        .with_state(ApiState { session })
}

/// `POST /api/tabs` → `{ "id": n }`.
async fn open_tab(State(st): State<ApiState>) -> impl IntoResponse {
    let id = st.session.lock().await.open_tab();
    (StatusCode::CREATED, Json(serde_json::json!({ "id": id }))).into_response()
}

/// `GET /api/tabs/{id}/frame` → `image/png` (pull ADR-0004).
async fn frame(State(st): State<ApiState>, Path(id): Path<TabId>) -> Response {
    let png = st.session.lock().await.frame_png(id);
    match png {
        Some(bytes) => (
            [(header::CONTENT_TYPE, HeaderValue::from_static("image/png"))],
            bytes,
        )
            .into_response(),
        None => StatusCode::NO_CONTENT.into_response(),
    }
}

/// Corps `POST …/navigate` : `{ "url": "https://…" }`.
async fn navigate(
    State(st): State<ApiState>,
    Path(id): Path<TabId>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    let Some(url) = body.get("url").and_then(|u| u.as_str()).map(str::to_string) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    // La garde vit pendant tout l'await : single-writer strict
    // — une navigation en cours sérialise les autres actions.
    let mut session = st.session.lock().await;
    match session.navigate(id, &url).await {
        Ok(()) => StatusCode::OK.into_response(),
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

/// Corps `POST …/viewport` : `{ "w": f32, "h": f32, "scale": f32 }`.
async fn viewport(
    State(st): State<ApiState>,
    Path(id): Path<TabId>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    let f = |k: &str| body.get(k).and_then(serde_json::Value::as_f64);
    let (Some(w), Some(h)) = (f("w"), f("h")) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let scale = f("scale").unwrap_or(1.0);
    let mut session = st.session.lock().await;
    #[allow(clippy::cast_possible_truncation)]
    session.set_viewport(w as f32, h as f32, scale as f32);
    // Re-render immédiat de l'onglet courant (resize live).
    let _ = session.rerender(id);
    StatusCode::OK.into_response()
}

/// `GET /api/events` — flux SSE des événements de session.
async fn events(State(st): State<ApiState>) -> impl IntoResponse {
    let rx = st.session.lock().await.subscribe();
    Sse::new(sse_stream(rx))
        .keep_alive(
            KeepAlive::new()
                .interval(Duration::from_secs(15))
                .text("ka"),
        )
        .into_response()
}

/// Convertit le `broadcast::Receiver` en stream SSE —
/// `CoreEvent` → `event:` + `data:` JSON ; un lag (bus plein)
/// saute les événements intermédiaires (le pull PNG corrige).
fn sse_stream(
    rx: tokio::sync::broadcast::Receiver<CoreEvent>,
) -> impl Stream<Item = Result<Event, Infallible>> {
    futures_util::stream::unfold(rx, |mut rx| async move {
        loop {
            match rx.recv().await {
                Ok(ev) => {
                    let (name, data) = event_json(&ev);
                    return Some((Ok(Event::default().event(name).data(data)), rx));
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return None,
            }
        }
    })
}

/// `CoreEvent` → `(nom SSE, payload JSON)` — le contrat
/// public de l'API (DTO, jamais le type interne).
fn event_json(ev: &CoreEvent) -> (&'static str, String) {
    match ev {
        CoreEvent::TabOpened { tab } => {
            ("tab_opened", serde_json::json!({ "tab": tab }).to_string())
        }
        CoreEvent::Navigated { tab, url } => (
            "navigated",
            serde_json::json!({ "tab": tab, "url": url }).to_string(),
        ),
        CoreEvent::FrameInvalidated { tab } => (
            "frame_invalidated",
            serde_json::json!({ "tab": tab }).to_string(),
        ),
        CoreEvent::PipelineError { tab, message } => (
            "pipeline_error",
            serde_json::json!({ "tab": tab, "message": message }).to_string(),
        ),
    }
}
