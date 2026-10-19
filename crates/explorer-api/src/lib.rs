// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! `explorer-api` — API REST + SSE (`axum`) exposée sur loopback :
//! routes, DTO, SSE, clé optionnelle (ADR-0003/0004).
//!
//! Étape 25 — contrat REST :
//! - `GET  /api/status`              → état du moteur ;
//! - `GET  /api/tabs`                → liste des onglets ;
//! - `POST /api/tabs`                → `{ "id": n }` ;
//! - `DELETE /api/tabs/{id}`         → ferme l'onglet ;
//! - `POST /api/tabs/{id}/navigate`  → `{ "url": "…" }` ;
//! - `POST /api/tabs/{id}/reload`    → re-fetch ;
//! - `POST /api/tabs/{id}/back`      → historique −1 ;
//! - `POST /api/tabs/{id}/forward`   → historique +1 ;
//! - `POST /api/tabs/{id}/viewport`  → `{ "w", "h", "scale" }` ;
//! - `GET  /api/tabs/{id}/frame`     → `image/png` (ADR-0004) ;
//! - `GET  /api/events`              → flux SSE.
//!
//! Erreurs : `{"error":{"handled":true,"message":"…"}}` (ADR-0003).
//! Les DTO ne fuient jamais dans `explorer-core`. Session
//! sérialisée par `tokio::Mutex` (single-writer : le DOM/layout
//! ne sort jamais de ce flux).

#![forbid(unsafe_code)]

use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Path, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use explorer_core::{CoreError, CoreEvent, InputEvent, InputOutcome, Session, TabId};
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
    router_with_key(session, None)
}

/// Routeur avec clé d'API optionnelle (ADR-0003) : si `key`
/// est `Some`, chaque requête doit présenter `x-api-key: key`
/// **ou** `?key=key` (SSE `EventSource` ne pose pas d'en-têtes
/// — compromis documenté).
pub fn router_with_key(session: Arc<tokio::sync::Mutex<Session>>, key: Option<String>) -> Router {
    let r = Router::new()
        .route("/api/status", get(status))
        .route("/api/tabs", get(list_tabs).post(open_tab))
        .route("/api/tabs/{id}", delete(close_tab))
        .route("/api/tabs/{id}/frame", get(frame))
        .route("/api/tabs/{id}/navigate", post(navigate))
        .route("/api/tabs/{id}/reload", post(reload))
        .route("/api/tabs/{id}/back", post(back))
        .route("/api/tabs/{id}/forward", post(forward))
        .route("/api/tabs/{id}/viewport", post(viewport))
        .route("/api/tabs/{id}/input", post(input))
        .route(
            "/api/bookmarks",
            get(list_bookmarks).post(add_bookmark).delete(del_bookmark),
        )
        .route("/api/history", get(history))
        .route("/api/events", get(events))
        .with_state(ApiState { session });
    match key {
        Some(k) if !k.is_empty() => r.layer(axum::middleware::from_fn(move |req, next| {
            auth(k.clone(), req, next)
        })),
        _ => r,
    }
}

/// Middleware de clé : `x-api-key` prioritaire, `?key=` en
/// repli — 401 JSON sinon (contrat d'erreur ADR-0003).
async fn auth(want: String, req: axum::extract::Request, next: axum::middleware::Next) -> Response {
    let ok = req
        .headers()
        .get("x-api-key")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v == want)
        || req
            .uri()
            .query()
            .is_some_and(|q| q.split('&').any(|p| p == format!("key={want}")));
    if ok {
        next.run(req).await
    } else {
        (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({
                "error": { "handled": true, "message": "clé d'API invalide ou absente" }
            })),
        )
            .into_response()
    }
}

/// Corps d'erreur du contrat : `{"error":{handled,message}}`.
fn err(status: StatusCode, e: &CoreError) -> Response {
    (
        status,
        Json(serde_json::json!({
            "error": { "handled": true, "message": e.to_string() }
        })),
    )
        .into_response()
}

/// Erreur métier → HTTP : `UnknownTab` → 404, erreurs de
/// pipeline (fetch, HTTP amont, parse) → 502.
fn err_status(e: &CoreError) -> StatusCode {
    match e {
        CoreError::UnknownTab(_) => StatusCode::NOT_FOUND,
        _ => StatusCode::BAD_GATEWAY,
    }
}

/// `GET /api/status` → état du moteur.
async fn status(State(st): State<ApiState>) -> impl IntoResponse {
    let session = st.session.lock().await;
    Json(serde_json::json!({
        "engine": "explorer",
        "version": env!("CARGO_PKG_VERSION"),
        "tabs": session.tabs(),
    }))
}

/// `GET /api/tabs` → `[{id, url, title, can_back, can_forward}]`.
async fn list_tabs(State(st): State<ApiState>) -> impl IntoResponse {
    let session = st.session.lock().await;
    let tabs: Vec<_> = session
        .tab_infos()
        .iter()
        .map(|t| {
            serde_json::json!({
                "id": t.id,
                "url": t.url,
                "title": t.title,
                "can_back": t.can_back,
                "can_forward": t.can_forward,
            })
        })
        .collect();
    Json(serde_json::json!({ "tabs": tabs }))
}

/// `POST /api/tabs` → `{ "id": n }`.
async fn open_tab(State(st): State<ApiState>) -> impl IntoResponse {
    let id = st.session.lock().await.open_tab();
    (StatusCode::CREATED, Json(serde_json::json!({ "id": id }))).into_response()
}

/// `DELETE /api/tabs/{id}`.
async fn close_tab(State(st): State<ApiState>, Path(id): Path<TabId>) -> Response {
    let mut session = st.session.lock().await;
    match session.close_tab(id) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => err(err_status(&e), &e),
    }
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

/// Corps `{ "url": "…" }` → `POST …/navigate`.
async fn navigate(
    State(st): State<ApiState>,
    Path(id): Path<TabId>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    let Some(url) = body.get("url").and_then(|u| u.as_str()).map(str::to_string) else {
        return err(
            StatusCode::BAD_REQUEST,
            &CoreError::Url("champ « url » manquant".to_string()),
        );
    };
    // La garde vit pendant tout l'await : single-writer strict
    // — une navigation en cours sérialise les autres actions.
    let mut session = st.session.lock().await;
    match session.navigate(id, &url).await {
        Ok(()) => tab_json(&session, id),
        Err(e) => err(err_status(&e), &e),
    }
}

/// `POST …/reload` — re-fetch de l'URL courante.
async fn reload(State(st): State<ApiState>, Path(id): Path<TabId>) -> Response {
    let mut session = st.session.lock().await;
    match session.reload(id).await {
        Ok(()) => tab_json(&session, id),
        Err(e) => err(err_status(&e), &e),
    }
}

/// `POST …/back` — historique −1.
async fn back(State(st): State<ApiState>, Path(id): Path<TabId>) -> Response {
    let mut session = st.session.lock().await;
    match session.go_back(id).await {
        Ok(()) => tab_json(&session, id),
        Err(e) => err(err_status(&e), &e),
    }
}

/// `POST …/forward` — historique +1.
async fn forward(State(st): State<ApiState>, Path(id): Path<TabId>) -> Response {
    let mut session = st.session.lock().await;
    match session.go_forward(id).await {
        Ok(()) => tab_json(&session, id),
        Err(e) => err(err_status(&e), &e),
    }
}

/// Réponse de navigation : l'état de l'onglet résultant.
fn tab_json(session: &Session, id: TabId) -> Response {
    match session.tab_info(id) {
        Some(t) => Json(serde_json::json!({
            "id": t.id,
            "url": t.url,
            "title": t.title,
            "can_back": t.can_back,
            "can_forward": t.can_forward,
        }))
        .into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

/// `POST …/viewport` : `{ "w": f32, "h": f32, "scale": f32 }`.
async fn viewport(
    State(st): State<ApiState>,
    Path(id): Path<TabId>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    let f = |k: &str| body.get(k).and_then(serde_json::Value::as_f64);
    let (Some(w), Some(h)) = (f("w"), f("h")) else {
        return err(
            StatusCode::BAD_REQUEST,
            &CoreError::Url("champs « w »/« h » manquants".to_string()),
        );
    };
    let scale = f("scale").unwrap_or(1.0);
    let mut session = st.session.lock().await;
    #[allow(clippy::cast_possible_truncation)]
    session.set_viewport(w as f32, h as f32, scale as f32);
    // Re-render immédiat de l'onglet courant (resize live).
    let _ = session.rerender(id);
    StatusCode::OK.into_response()
}

/// `POST …/input` : `{ "type": "click|mousemove|scroll|key", … }`.
/// Les `mousemove` sont coalescés côté client (dernier état
/// suffit — ADR-0003).
#[allow(clippy::cast_possible_truncation)] // px UI : f32 suffit
async fn input(
    State(st): State<ApiState>,
    Path(id): Path<TabId>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    let f = |k: &str| body.get(k).and_then(serde_json::Value::as_f64);
    let ev = match body.get("type").and_then(|t| t.as_str()) {
        Some("click") => InputEvent::Click {
            x: f("x").unwrap_or(0.0) as f32,
            y: f("y").unwrap_or(0.0) as f32,
        },
        Some("mousemove") => InputEvent::MouseMove {
            x: f("x").unwrap_or(0.0) as f32,
            y: f("y").unwrap_or(0.0) as f32,
        },
        Some("scroll") => InputEvent::Scroll {
            dy: f("dy").unwrap_or(0.0) as f32,
        },
        Some("key") => InputEvent::Key {
            key: body
                .get("key")
                .and_then(|k| k.as_str())
                .unwrap_or("")
                .to_string(),
        },
        _ => {
            return err(
                StatusCode::BAD_REQUEST,
                &CoreError::Url("type d'entrée inconnu".to_string()),
            );
        }
    };
    let outcome = {
        let mut session = st.session.lock().await;
        match session.handle_input(id, ev).await {
            Ok(o) => o,
            Err(e) => return err(err_status(&e), &e),
        }
    };
    Json(match outcome {
        InputOutcome::Ignored => serde_json::json!({ "result": "ignored" }),
        InputOutcome::Scrolled { scroll_y } => {
            serde_json::json!({ "result": "scrolled", "scroll_y": scroll_y })
        }
        InputOutcome::Navigated => serde_json::json!({ "result": "navigated" }),
        InputOutcome::Focused => serde_json::json!({ "result": "focused" }),
        InputOutcome::Edited => serde_json::json!({ "result": "edited" }),
        InputOutcome::Hover { url, editable } => {
            serde_json::json!({ "result": "hover", "url": url, "editable": editable })
        }
    })
    .into_response()
}

/// `GET /api/bookmarks` → `[{url, title}]`.
async fn list_bookmarks(State(st): State<ApiState>) -> impl IntoResponse {
    let session = st.session.lock().await;
    let v: Vec<_> = session
        .bookmarks()
        .iter()
        .map(|b| serde_json::json!({ "url": b.url, "title": b.title }))
        .collect();
    Json(serde_json::json!({ "bookmarks": v }))
}

/// `POST /api/bookmarks` `{ "url": "…", "title": "…"? }` —
/// dédupliqué par URL (409 si déjà présent).
async fn add_bookmark(State(st): State<ApiState>, Json(body): Json<serde_json::Value>) -> Response {
    let Some(url) = body.get("url").and_then(|u| u.as_str()).map(str::to_string) else {
        return err(
            StatusCode::BAD_REQUEST,
            &CoreError::Url("champ « url » manquant".to_string()),
        );
    };
    let title = body
        .get("title")
        .and_then(|t| t.as_str())
        .map(str::to_string);
    let mut session = st.session.lock().await;
    if session.add_bookmark(url, title) {
        StatusCode::CREATED.into_response()
    } else {
        err(
            StatusCode::CONFLICT,
            &CoreError::Url("signet déjà présent".to_string()),
        )
    }
}

/// `DELETE /api/bookmarks` `{ "url": "…" }` — 404 si absent.
async fn del_bookmark(State(st): State<ApiState>, Json(body): Json<serde_json::Value>) -> Response {
    let Some(url) = body.get("url").and_then(|u| u.as_str()).map(str::to_string) else {
        return err(
            StatusCode::BAD_REQUEST,
            &CoreError::Url("champ « url » manquant".to_string()),
        );
    };
    let mut session = st.session.lock().await;
    if session.remove_bookmark(&url) {
        StatusCode::NO_CONTENT.into_response()
    } else {
        err(
            StatusCode::NOT_FOUND,
            &CoreError::Url("signet inconnu".to_string()),
        )
    }
}

/// `GET /api/history` → `[{url, title}]` du store (vide si
/// pas de profil persistant).
async fn history(State(st): State<ApiState>) -> impl IntoResponse {
    let session = st.session.lock().await;
    let v: Vec<_> = session
        .history()
        .iter()
        .map(|e| serde_json::json!({ "url": e.url, "title": e.title }))
        .collect();
    Json(serde_json::json!({ "history": v }))
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
        CoreEvent::TabClosed { tab } => {
            ("tab_closed", serde_json::json!({ "tab": tab }).to_string())
        }
        CoreEvent::Navigated { tab, url, title } => (
            "navigated",
            serde_json::json!({ "tab": tab, "url": url, "title": title }).to_string(),
        ),
        CoreEvent::TitleChanged { tab, title } => (
            "title_changed",
            serde_json::json!({ "tab": tab, "title": title }).to_string(),
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
