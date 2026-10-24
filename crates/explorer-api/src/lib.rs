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

use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use explorer_core::{
    drain_applies_shared, drain_navs_shared, history_move_shared, navigate_shared, CoreError,
    CoreEvent, InputEvent, InputOutcome, KeyMods, Session, TabId,
};
use explorer_net::http::{Initiator, Method, Request};
use explorer_net::url::Url;
use futures_util::Stream;

/// État partagé : la session moteur, sérialisée.
#[derive(Clone)]
pub struct ApiState {
    /// Session — `tokio::Mutex` : la garde survit aux `.await`
    /// de `navigate` (future `Send`, single-writer strict).
    pub session: Arc<tokio::sync::Mutex<Session>>,
    /// Descripteur du résolveur en sortie (`system`,
    /// `doh:<endpoint>` — étape 72) — observabilité
    /// `GET /api/egress`, jamais de basculement silencieux.
    pub egress: String,
    /// Journal console par onglet — `PipelineError` enregistrés
    /// (cap [`CONSOLE_RING`]) : les erreurs JS/fetch survivent
    /// au flux SSE éphémère (`GET /api/tabs/{id}/console`).
    pub console: ConsoleRing,
}

/// Ring borné des erreurs pipeline par onglet — la tâche
/// `recorder` (`router_with_egress`) le remplit depuis le
/// broadcast ; la sonde le lit. `std::sync::Mutex` : tenue
/// brève, jamais d'`.await` sous verrou.
pub type ConsoleRing = std::sync::Arc<
    std::sync::Mutex<std::collections::HashMap<TabId, std::collections::VecDeque<String>>>,
>;

/// Capacité par onglet — assez pour un boot de bundle.
const CONSOLE_RING: usize = 512;

/// Enregistre les `PipelineError` par onglet dans `ring` —
/// la sonde `/console` survit à l'éphémère SSE (vol github :
/// les erreurs de boot mouraient avec la connexion).
fn spawn_console_recorder(session: &Arc<tokio::sync::Mutex<Session>>, ring: &ConsoleRing) {
    let mut rx = {
        // Subscribe sans garder le verrou : subscribe() est
        // sync, le lock ne sert qu'à atteindre la session.
        let Ok(s) = session.try_lock() else {
            return; // session occupée : pas de recorder (dégradé, jamais bloquant)
        };
        s.subscribe()
    };
    let ring = std::sync::Arc::clone(ring);
    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(CoreEvent::PipelineError { tab, message }) => {
                    if let Some(t) = tab {
                        let mut g = ring
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                        let q = g.entry(t).or_default();
                        if q.len() >= CONSOLE_RING {
                            q.pop_front();
                        }
                        q.push_back(message);
                    }
                }
                Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
            }
        }
    });
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
    router_with_egress(session, key, "system".to_string())
}

/// Routeur complet : clé optionnelle + descripteur de
/// résolveur exposé par `GET /api/egress` (étape 72).
pub fn router_with_egress(
    session: Arc<tokio::sync::Mutex<Session>>,
    key: Option<String>,
    egress: String,
) -> Router {
    let console = ConsoleRing::default();
    spawn_console_recorder(&session, &console);
    let r = Router::new()
        .route("/api/status", get(status))
        .route("/api/egress", get(egress_info))
        .route("/api/accept-language", post(accept_language))
        .route("/api/tabs", get(list_tabs).post(open_tab))
        .route("/api/tabs/{id}", delete(close_tab))
        .route("/api/tabs/{id}/frame", get(frame))
        .route("/api/tabs/{id}/dom", get(dom_dump))
        .route("/api/tabs/{id}/layout", get(layout_dump))
        .route("/api/tabs/{id}/mem", get(mem_report))
        .route("/api/tabs/{id}/navigate", post(navigate))
        .route("/api/tabs/{id}/reload", post(reload))
        .route("/api/tabs/{id}/back", post(back))
        .route("/api/tabs/{id}/forward", post(forward))
        .route("/api/tabs/{id}/viewport", post(viewport))
        .route("/api/tabs/{id}/color-scheme", post(color_scheme))
        .route("/api/tabs/{id}/input", post(input))
        .route("/api/tabs/{id}/find", post(find))
        .route("/api/tabs/{id}/media", get(list_media))
        .route("/api/tabs/{id}/media/{mid}", get(get_media))
        .route("/api/tabs/{id}/media/{mid}/claim", post(media_claim))
        .route("/api/tabs/{id}/media/{mid}/release", post(media_release))
        .route("/api/tabs/{id}/media/{mid}/state", post(media_state))
        .route("/api/tabs/{id}/downloads", get(list_downloads))
        .route(
            "/api/tabs/{id}/downloads/{dl}/accept",
            post(accept_download),
        )
        .route(
            "/api/tabs/{id}/downloads/{dl}/decline",
            post(decline_download),
        )
        .route(
            "/api/bookmarks",
            get(list_bookmarks).post(add_bookmark).delete(del_bookmark),
        )
        .route("/api/history", get(history))
        .route("/api/events", get(events))
        .route("/api/tabs/{id}/console", get(console_log))
        .with_state(ApiState {
            session,
            egress,
            console,
        });
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
fn err(status: StatusCode, e: &impl std::fmt::Display) -> Response {
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
        "accept_language": session.accept_language(),
    }))
}

/// `POST /api/accept-language` — `{"lang": "fr-FR,fr;q=0.9"}`
/// (étape 118) : la session annonce la valeur en
/// `Accept-Language` sur toutes les requêtes suivantes ;
/// `""`/`null` lève l'en-tête.
async fn accept_language(
    State(st): State<ApiState>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    let lang = body.get("lang").and_then(|v| {
        if v.is_null() {
            Some(String::new())
        } else {
            v.as_str().map(str::to_string)
        }
    });
    let Some(lang) = lang else {
        return err(
            StatusCode::BAD_REQUEST,
            &CoreError::Url("champ « lang » chaîne attendu".to_string()),
        );
    };
    st.session
        .lock()
        .await
        .set_accept_language(if lang.trim().is_empty() {
            None
        } else {
            Some(lang)
        });
    StatusCode::OK.into_response()
}

/// `GET /api/egress` → `{resolver}` — mode de résolution DNS
/// configuré à l'assemblage (`system` ou `doh:<endpoint>`,
/// étape 72). Observabilité uniquement : le mode ne bascule
/// jamais en cours de route.
async fn egress_info(State(st): State<ApiState>) -> impl IntoResponse {
    Json(serde_json::json!({ "resolver": st.egress }))
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

/// Paramètres de `GET /frame` (étape 55) — région en
/// pixels physiques, comme les `rects` de
/// `frame_invalidated`. Incomplet (une coordonnée manque) =
/// frame entière, pour rester tolérant côté client.
#[derive(serde::Deserialize)]
struct FrameRegionQuery {
    x: Option<u32>,
    y: Option<u32>,
    w: Option<u32>,
    h: Option<u32>,
}

/// `GET /api/tabs/{id}/frame` → `image/png` (pull ADR-0004).
/// Le pull fait avancer les transitions CSS (étape 45) :
/// l'UI cadence les frames, le moteur échantillonne le
/// progrès à chaque appel. `?x=&y=&w=&h=` (px physiques,
/// étape 55) ne sert que la région demandée.
async fn frame(
    State(st): State<ApiState>,
    Path(id): Path<TabId>,
    Query(q): Query<FrameRegionQuery>,
) -> Response {
    let mut session = st.session.lock().await;
    let _ = session.tick_animations(id);
    let png = match (q.x, q.y, q.w, q.h) {
        (Some(rx), Some(ry), Some(rw), Some(rh)) => session.frame_png_region(id, rx, ry, rw, rh),
        _ => session.frame_png(id),
    };
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
    // Ordre des erreurs préservé : onglet inconnu avant URL
    // invalide (comme `Session::navigate`).
    {
        let session = st.session.lock().await;
        if session.tab_info(id).is_none() {
            return err(StatusCode::NOT_FOUND, &CoreError::UnknownTab(id));
        }
    }
    let url = match Url::parse(&url) {
        Ok(u) => u,
        Err(e) => return err(StatusCode::BAD_REQUEST, &CoreError::Url(e.to_string())),
    };
    // Étape 53d — la navigation est conduite en phases hors
    // verrou (`navigate_shared`) : le `Mutex<Session>` n'est
    // plus tenu pendant le téléchargement, `status`, `tabs`,
    // `frame` et les autres onglets restent serviables.
    // Navigation explicite (requête API) = top-level —
    // `SameSite` : tout passe (étape 66).
    let mut req = Request::get(url);
    req.initiator = Some(Initiator::TopLevelNav);
    match Box::pin(navigate_shared(&st.session, id, req, true)).await {
        Ok(()) => {
            Box::pin(drain_navs_shared(&st.session, id)).await;
            let session = st.session.lock().await;
            tab_json(&session, id)
        }
        Err(e) => err(err_status(&e), &e),
    }
}

/// `POST …/reload` — réémet la requête courante (méthode
/// d'origine, corps compris — étape 93 : l'endpoint émettait
/// un `GET` vide, court-circuitant `Session::reload` et son
/// `last_req`). Corps optionnel `{ "confirm": true }` :
/// requis pour rejouer une méthode non sûre — sinon statut
/// typé `resubmit_confirm` (l'app affiche le dialogue
/// « renvoyer le formulaire ? » des navigateurs).
async fn reload(
    State(st): State<ApiState>,
    Path(id): Path<TabId>,
    body: axum::body::Bytes,
) -> Response {
    let confirm = serde_json::from_slice::<serde_json::Value>(&body)
        .ok()
        .and_then(|v| v.get("confirm")?.as_bool())
        .unwrap_or_default();
    let req = {
        let session = st.session.lock().await;
        match session.reload_request(id) {
            Ok(r) => r,
            Err(e) => return err(err_status(&e), &e),
        }
    };
    if !confirm && !matches!(req.method, Method::Get | Method::Head) {
        // Page servie par une méthode non sûre (POST…) : la
        // resoumission silencieuse peut doubler un effet de
        // bord (commande, paiement). Le client rappelle avec
        // `{"confirm":true}` après accord explicite.
        return Json(serde_json::json!({
            "result": "resubmit_confirm",
            "method": req.method.as_str(),
            "url": req.url.to_string(),
        }))
        .into_response();
    }
    match Box::pin(navigate_shared(&st.session, id, req, false)).await {
        Ok(()) => {
            Box::pin(drain_navs_shared(&st.session, id)).await;
            let session = st.session.lock().await;
            tab_json(&session, id)
        }
        Err(e) => err(err_status(&e), &e),
    }
}

/// `POST …/back` — historique −1.
async fn back(State(st): State<ApiState>, Path(id): Path<TabId>) -> Response {
    match Box::pin(history_move_shared(&st.session, id, -1)).await {
        Ok(()) => {
            Box::pin(drain_navs_shared(&st.session, id)).await;
            let session = st.session.lock().await;
            tab_json(&session, id)
        }
        Err(e) => err(err_status(&e), &e),
    }
}

/// `POST …/forward` — historique +1.
async fn forward(State(st): State<ApiState>, Path(id): Path<TabId>) -> Response {
    match Box::pin(history_move_shared(&st.session, id, 1)).await {
        Ok(()) => {
            Box::pin(drain_navs_shared(&st.session, id)).await;
            let session = st.session.lock().await;
            tab_json(&session, id)
        }
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
            // Hauteur document réelle vs raster plafonné
            // (étape 65) — `doc_h` peut dépasser la frame.
            "doc_h": t.doc_h,
            "doc_h_clamped": t.doc_h_clamped,
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

/// `POST …/color-scheme` : `{ "scheme": "dark"|"light" }` —
/// bascule `prefers-color-scheme` + défauts UA (étape 97).
/// Le réglage est session-level (comme `set_viewport`, la
/// route par-onglet ne fait que nommer l'onglet à repeindre).
async fn color_scheme(
    State(st): State<ApiState>,
    Path(id): Path<TabId>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    let scheme = match body.get("scheme").and_then(serde_json::Value::as_str) {
        Some("dark") => explorer_core::ColorScheme::Dark,
        Some("light") => explorer_core::ColorScheme::Light,
        _ => {
            return err(
                StatusCode::BAD_REQUEST,
                &CoreError::Url("champ « scheme » « dark »|« light » attendu".to_string()),
            );
        }
    };
    let mut session = st.session.lock().await;
    session.set_color_scheme(scheme);
    // Re-render immédiat : la cascade réévalue les media
    // queries et les défauts UA sur le nouvel env.
    let _ = session.rerender(id);
    StatusCode::OK.into_response()
}

/// `ctrl`/`shift`/`alt`/`meta` optionnels d'une entrée clavier
/// (étape 105) — absent = `false`.
fn key_mods(body: &serde_json::Value) -> KeyMods {
    let b = |k: &str| {
        body.get(k)
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
    };
    KeyMods {
        ctrl: b("ctrl"),
        shift: b("shift"),
        alt: b("alt"),
        meta: b("meta"),
    }
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
            mods: key_mods(&body),
        },
        Some("keyup") => InputEvent::KeyUp {
            key: body
                .get("key")
                .and_then(|k| k.as_str())
                .unwrap_or("")
                .to_string(),
            mods: key_mods(&body),
        },
        _ => {
            return err(
                StatusCode::BAD_REQUEST,
                &CoreError::Url("type d'entrée inconnu".to_string()),
            );
        }
    };
    // `handle_input_now` pose les intentions (étape 53d) et
    // la réponse rend l'outcome d'enfilement **immédiatement**
    // (étape 73) : les drains — apply en `spawn_blocking`
    // (>30 s sur heavy.html) et navs réseau — tournent en
    // tâche détachée, pas dans l'attente HTTP. La vérité du
    // commit arrive par SSE : `navigated`/`pipeline_error`
    // pour les navs, `frame_invalidated` pour la peinture ;
    // un apply supplanté par epoch est jeté par la garde
    // (observable via les events du gagnant). Les appelants
    // `&mut` (FFI/CLI) gardent `handle_input` qui draine en
    // synchrone et rapporte le commit réel.
    let outcome = {
        let mut session = st.session.lock().await;
        match session.handle_input_now(id, ev) {
            Ok(o) => o,
            Err(e) => return err(err_status(&e), &e),
        }
    };
    {
        let sess = st.session.clone();
        tokio::spawn(async move {
            drain_applies_shared(&sess, id).await;
            Box::pin(drain_navs_shared(&sess, id)).await;
        });
    }
    Json(match outcome {
        InputOutcome::Ignored => serde_json::json!({ "result": "ignored" }),
        InputOutcome::Scrolled { scroll_y } => {
            serde_json::json!({ "result": "scrolled", "scroll_y": scroll_y })
        }
        InputOutcome::Navigated => serde_json::json!({ "result": "navigated" }),
        InputOutcome::Focused => serde_json::json!({ "result": "focused" }),
        InputOutcome::Edited => serde_json::json!({ "result": "edited" }),
        InputOutcome::Unfocused => serde_json::json!({ "result": "unfocused" }),
        InputOutcome::Hover { url, editable } => {
            serde_json::json!({ "result": "hover", "url": url, "editable": editable })
        }
    })
    .into_response()
}

/// `GET /api/tabs/{id}/dom` → `text/plain` : dump du DOM
/// **vivant** (post-JS — étape 120). Outil de triage : les
/// contrôles construits par script (mur de consentement
/// Google…) n'apparaissent qu'ici, pas dans le HTML servi.
/// `404` onglet inconnu ou DOM détaché (apply en vol).
async fn dom_dump(State(st): State<ApiState>, Path(id): Path<TabId>) -> Response {
    let mut session = st.session.lock().await;
    // Réinsère d'abord une éventuelle payload détachée
    // orpheline (issue #23 — tâche de rendu tombée en vol).
    session.reclaim_orphaned_paint(id);
    match session.dom_dump(id) {
        Some(d) => (StatusCode::OK, d).into_response(),
        None => (StatusCode::NOT_FOUND, "pas de DOM").into_response(),
    }
}

/// `GET /api/tabs/{id}/layout` → `text/plain` : dump de
/// l'arbre de boîtes posé (étape 120) — le hit-test s'y
/// résout ; un « clic ignoré » = zone peinte sans boîte
/// couvrante. `404` onglet inconnu ou arbre détaché.
async fn layout_dump(State(st): State<ApiState>, Path(id): Path<TabId>) -> Response {
    let mut session = st.session.lock().await;
    session.reclaim_orphaned_paint(id);
    match session.layout_dump(id) {
        Some(d) => (StatusCode::OK, d).into_response(),
        None => (StatusCode::NOT_FOUND, "pas de layout").into_response(),
    }
}

/// `GET /api/tabs/{id}/mem` → `MemReport` JSON (étape 130) :
/// attribution mémoire par réservoir — page vivante + pages
/// gelées bfcache (heaps JS gelés inclus). `404` onglet
/// inconnu. Verrou mutable : lire `heap_stats` d'un contexte
/// JS passe par le canal de son worker.
async fn mem_report(State(st): State<ApiState>, Path(id): Path<TabId>) -> Response {
    let mut session = st.session.lock().await;
    session.reclaim_orphaned_paint(id);
    match session.mem_report(id) {
        Some(r) => Json(r).into_response(),
        None => (StatusCode::NOT_FOUND, "onglet inconnu").into_response(),
    }
}

/// `POST /api/tabs/{id}/find` `{ "query": "…", "dir": 0|±1 }`
/// — `dir = 0` : nouvelle recherche ; ±1 : occurrence
/// suivante/précédente. Réponse `{ count, index, query }`.
/// Le surlignage est peint côté moteur (étape 57bis) — la
/// prochaine frame le montre via le pull habituel.
async fn find(
    State(st): State<ApiState>,
    Path(id): Path<TabId>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    let query = body
        .get("query")
        .and_then(|q| q.as_str())
        .unwrap_or("")
        .to_string();
    let dir = body
        .get("dir")
        .and_then(serde_json::Value::as_i64)
        .unwrap_or(0);
    let mut session = st.session.lock().await;
    let (count, index) = if dir == 0 {
        (session.find_in_page(id, &query), 0)
    } else {
        let (i, n) = session.find_step(id, dir);
        (n, i)
    };
    Json(serde_json::json!({
        "count": count,
        "index": index,
        "query": query,
    }))
    .into_response()
}

/// `GET /api/tabs/{id}/downloads` → téléchargements en
/// attente de l'onglet (étape 70).
async fn list_downloads(State(st): State<ApiState>, Path(id): Path<TabId>) -> impl IntoResponse {
    let session = st.session.lock().await;
    let v: Vec<_> = session
        .pending_downloads(id)
        .iter()
        .map(|d| {
            serde_json::json!({
                "id": d.id, "tab": d.tab, "url": d.url,
                "filename": d.filename, "mime": d.mime, "size": d.size,
            })
        })
        .collect();
    Json(serde_json::json!({ "downloads": v }))
}

/// `POST /api/tabs/{id}/downloads/{dl}/accept`
/// `{ "path": "…"? }` — écrit le corps bufferisé au chemin
/// donné ou sous le répertoire de téléchargement de la
/// session avec le nom suggéré (étape 70).
async fn accept_download(
    State(st): State<ApiState>,
    Path((id, dl)): Path<(TabId, u64)>,
    body: Option<Json<serde_json::Value>>,
) -> Response {
    let path = body.and_then(|Json(v)| {
        v.get("path")
            .and_then(|p| p.as_str())
            .map(std::path::PathBuf::from)
    });
    let mut session = st.session.lock().await;
    match session.accept_download(id, dl, path) {
        Ok(target) => Json(serde_json::json!({
            "path": target.display().to_string()
        }))
        .into_response(),
        Err(e) => err(StatusCode::BAD_REQUEST, &e),
    }
}

/// `POST /api/tabs/{id}/downloads/{dl}/decline` — le corps
/// est rejoué comme document (dégradation « dump texte »).
async fn decline_download(
    State(st): State<ApiState>,
    Path((id, dl)): Path<(TabId, u64)>,
) -> Response {
    let mut session = st.session.lock().await;
    match session.decline_download(id, dl).await {
        Ok(_) => Json(serde_json::json!({ "declined": true })).into_response(),
        Err(e) => err(StatusCode::BAD_REQUEST, &e),
    }
}

/// Sérialise une surface média (DTO du contrat
/// `MediaSurface` — rect en px CSS document). La forme
/// appartient au contrat : déléguée à `explorer-core` pour
/// rester identique au drain FFI (étape 71e).
fn media_json(m: &explorer_core::MediaOfferInfo) -> serde_json::Value {
    explorer_core::media_offer_json(m)
}

/// `MediaError` → statut HTTP du contrat (`410` id mort,
/// `409` déjà claimée / non claimée).
fn media_err(e: explorer_core::MediaError) -> Response {
    let code = match e {
        explorer_core::MediaError::Dead => StatusCode::GONE,
        explorer_core::MediaError::Taken | explorer_core::MediaError::NotClaimed => {
            StatusCode::CONFLICT
        }
    };
    err(code, &e)
}

/// `GET /api/tabs/{id}/media` → surfaces média vivantes de
/// l'onglet (étape 71b, contrat `MediaSurface`).
async fn list_media(State(st): State<ApiState>, Path(id): Path<TabId>) -> impl IntoResponse {
    let session = st.session.lock().await;
    let v: Vec<_> = session.media_list(id).iter().map(media_json).collect();
    Json(serde_json::json!({ "media": v }))
}

/// `GET /api/tabs/{id}/media/{mid}` → fiche d'une surface
/// (diagnostics) — `410` si l'id est mort.
async fn get_media(State(st): State<ApiState>, Path((id, mid)): Path<(TabId, u64)>) -> Response {
    let session = st.session.lock().await;
    match session.media_list(id).into_iter().find(|m| m.id == mid) {
        Some(m) => Json(media_json(&m)).into_response(),
        None => media_err(explorer_core::MediaError::Dead),
    }
}

/// `POST /api/tabs/{id}/media/{mid}/claim` `{codecs: []}` —
/// premier arrivé gagne : `200` claim acquis, `409` déjà
/// claimée, `410` élément mort (étape 71b).
async fn media_claim(
    State(st): State<ApiState>,
    Path((id, mid)): Path<(TabId, u64)>,
    body: Option<Json<serde_json::Value>>,
) -> Response {
    let codecs: Vec<String> = body
        .map(|Json(v)| explorer_core::media_codecs_from_json(&v))
        .unwrap_or_default();
    let mut session = st.session.lock().await;
    match session.media_claim(id, mid, codecs) {
        Ok(()) => Json(serde_json::json!({ "claimed": true })).into_response(),
        Err(e) => media_err(e),
    }
}

/// `POST /api/tabs/{id}/media/{mid}/release` — l'hôte se
/// détache ; la surface retombe sur les stubs ADR-0023.
async fn media_release(
    State(st): State<ApiState>,
    Path((id, mid)): Path<(TabId, u64)>,
) -> Response {
    let mut session = st.session.lock().await;
    match session.media_release(id, mid) {
        Ok(()) => Json(serde_json::json!({ "released": true })).into_response(),
        Err(e) => media_err(e),
    }
}

/// `POST /api/tabs/{id}/media/{mid}/state`
/// `{position_ms, duration_ms, buffered_ms, state, error?}`
/// — rapport de lecture hôte (battement ≤ 4 Hz côté hôte,
/// étape 71b ; reflet JS à 71c).
async fn media_state(
    State(st): State<ApiState>,
    Path((id, mid)): Path<(TabId, u64)>,
    body: Option<Json<serde_json::Value>>,
) -> Response {
    let Some(Json(v)) = body else {
        return err(
            StatusCode::BAD_REQUEST,
            &std::io::Error::other("corps JSON requis"),
        );
    };
    let Some(report) = explorer_core::media_report_from_json(&v) else {
        return err(
            StatusCode::BAD_REQUEST,
            &std::io::Error::other("state ∈ playing|paused|ended|error"),
        );
    };
    let mut session = st.session.lock().await;
    match session.media_report(id, mid, report) {
        Ok(()) => Json(serde_json::json!({ "reported": true })).into_response(),
        Err(e) => media_err(e),
    }
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

/// `GET /api/tabs/{id}/console` → `[{ts_ms, message}]` :
/// erreurs pipeline (scripts en échec, fetch refusés)
/// enregistrées par onglet — la sonde persistante là où le
/// flux SSE est éphémère (vol github : les erreurs de boot
/// mouraient avec la connexion). `404` onglet sans journal.
async fn console_log(State(st): State<ApiState>, Path(id): Path<TabId>) -> Response {
    let g = st
        .console
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    match g.get(&id) {
        Some(q) if !q.is_empty() => Json(serde_json::json!({
            "console": q.iter().collect::<Vec<_>>()
        }))
        .into_response(),
        _ => (StatusCode::NOT_FOUND, "pas de journal").into_response(),
    }
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
        CoreEvent::FrameInvalidated {
            tab,
            rects,
            scroll_y,
        } => (
            "frame_invalidated",
            serde_json::json!({
                "tab": tab,
                // Étape 55 : régions invalides (px physiques)
                // — le client peut ne re-tirer que
                // `frame?x=&y=&w=&h=` par région ; vide =
                // repeindre entièrement.
                "rects": rects.iter().map(|r| serde_json::json!({
                    "x": r.x, "y": r.y, "w": r.w, "h": r.h,
                })).collect::<Vec<_>>(),
                // Étape 71d : scroll de la frame (px CSS) —
                // overlays média positionnés à `rect − scroll_y`.
                "scroll_y": scroll_y,
            })
            .to_string(),
        ),
        CoreEvent::PipelineError { tab, message } => (
            "pipeline_error",
            serde_json::json!({ "tab": tab, "message": message }).to_string(),
        ),
        CoreEvent::DownloadOffered {
            tab,
            id,
            url,
            filename,
            mime,
            size,
        } => (
            "download_offered",
            serde_json::json!({
                "tab": tab, "id": id, "url": url,
                "filename": filename, "mime": mime, "size": size,
            })
            .to_string(),
        ),
        CoreEvent::DownloadProgress {
            tab,
            id,
            received,
            total,
        } => (
            "download_progress",
            serde_json::json!({
                "tab": tab, "id": id,
                "received": received, "total": total,
            })
            .to_string(),
        ),
        CoreEvent::DownloadDone { tab, id, path } => (
            "download_done",
            serde_json::json!({ "tab": tab, "id": id, "path": path }).to_string(),
        ),
        CoreEvent::MediaOffered { .. }
        | CoreEvent::MediaMoved { .. }
        | CoreEvent::MediaGone { .. }
        | CoreEvent::MediaCommand { .. } => media_event_json(ev),
    }
}

/// Bras `MediaSurface` de [`event_json`] (étape 71b) —
/// délégué au contrat `explorer-core` (étape 71e) : la
/// charge JSON est la même en SSE et en drain FFI.
fn media_event_json(ev: &CoreEvent) -> (&'static str, String) {
    // Le motif d'appel ne retient que les variantes média ;
    // un appel direct hors bras dégrade en event muet.
    explorer_core::media_event_json(ev).unwrap_or(("media", String::new()))
}
