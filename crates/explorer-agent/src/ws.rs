// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Transport WebSocket `GET /agent` — la porte d'entrée EAP
//! (ADR-0035). Un socket = une connexion [`AgentConn`] :
//! texte JSON-RPC 2.0 pour le contrôle, trames binaires pour
//! les charges (PNG), notifications `event.*` poussées par un
//! forwarder dédié filtré par `events.subscribe`.

use std::sync::{Arc, RwLock};

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use explorer_core::{CoreEvent, Session};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::Value;
use tokio::sync::mpsc;

use crate::dispatch::{dispatch, Dispatch};
use crate::proto::{ErrorCode, Event, EventKind, Incoming, Notification, Response as RpcResponse};
use crate::state::{AgentConn, AgentShared, SubFilter};

/// Capacité de la file d'émission par connexion — au-delà le
/// socket est considéré mort (un client lent ne doit pas faire
/// grossir la mémoire du daemon sans borne).
const OUT_QUEUE: usize = 64;

/// Clé API en query (`?key=`) — même convention que l'API REST.
#[derive(Debug, Deserialize)]
struct AgentQuery {
    /// Clé optionnelle — alternative à `x-api-key` (un client WS
    /// navigateur ne peut pas poser d'en-tête custom).
    key: Option<String>,
}

/// Routeur de la surface agent — à fusionner dans le routeur du
/// daemon. `/agent` monte le socket, `/agent/schema` sert le
/// contrat JSON pour introspection (même contenu que
/// [`crate::SCHEMA`]).
pub fn agent_router(session: Arc<tokio::sync::Mutex<Session>>, key: Option<String>) -> Router {
    agent_router_with_profile(session, key, None)
}

/// [`agent_router`] + profil servi injecté (`--profile`/env/
/// défaut résolu par l'hôte) — annoncé dans le bloc `dirs` de
/// `session.open` pour que le client connaisse les dossiers
/// applicatifs du daemon (étape 265).
pub fn agent_router_with_profile(
    session: Arc<tokio::sync::Mutex<Session>>,
    key: Option<String>,
    served_profile: Option<std::path::PathBuf>,
) -> Router {
    let st = AgentShared::with_profile(session, key, served_profile);
    Router::new()
        .route("/agent", get(ws_upgrade))
        .route("/agent/schema", get(schema))
        .with_state(st)
}

/// `GET /agent/schema` — le contrat, verbatim.
async fn schema(State(_): State<Arc<AgentShared>>) -> Response {
    (
        [(axum::http::header::CONTENT_TYPE, "application/schema+json")],
        crate::SCHEMA,
    )
        .into_response()
}

/// `GET /agent` — upgrade. La clé est vérifiée ici (avant le
/// handshake : un socket ouvert non autorisé est une fuite de
/// surface), le sous-protocole `eap/1` est exigé.
async fn ws_upgrade(
    ws: WebSocketUpgrade,
    State(st): State<Arc<AgentShared>>,
    headers: HeaderMap,
    Query(q): Query<AgentQuery>,
) -> Response {
    if let Some(expected) = &st.key {
        let given = headers
            .get("x-api-key")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned)
            .or(q.key);
        if given.as_deref() != Some(expected.as_str()) {
            return StatusCode::UNAUTHORIZED.into_response();
        }
    }
    ws.protocols([crate::PROTOCOL])
        .on_upgrade(move |sock| serve(sock, st))
}

/// Boucle d'une connexion : réception → dispatch → émission ;
/// le forwarder tourne en tâche sœur sur le bus `CoreEvent`.
async fn serve(sock: WebSocket, st: Arc<AgentShared>) {
    let (sink, mut stream) = sock.split();
    let (tx, rx) = mpsc::channel::<Message>(OUT_QUEUE);
    let writer = tokio::spawn(write_loop(sink, rx));
    let subs = Arc::new(RwLock::new(SubFilter::default()));
    let fwd = tokio::spawn(forwarder(Arc::clone(&st), Arc::clone(&subs), tx.clone()));

    let mut conn = AgentConn {
        subs: Arc::clone(&subs),
        ..AgentConn::default()
    };

    while let Some(msg) = stream.next().await {
        match msg {
            Ok(Message::Text(t)) => {
                handle(&st, &mut conn, t.as_str(), &tx).await;
            }
            Ok(Message::Close(_)) | Err(_) => break,
            Ok(_) => {} // ping/pong gérés par tungstenite ; binaire entrant non utilisé en v1
        }
        if conn.closing {
            break;
        }
    }
    fwd.abort();
    // `session.close` doit livrer sa réponse : plus de
    // producteur (`fwd` mort), on lâche `tx` — le writer
    // draine la file (réponse close incluse) puis termine
    // et le socket se ferme proprement. Un `abort` ici
    // perdrait la réponse et pendrait le client.
    drop(tx);
    let _ = writer.await;
}

/// Écrit tout ce que la file reçoit — une seule tâche touche
/// le sink (les réponses, les notifications et les binaires
/// s'y entrelacent dans l'ordre de production).
async fn write_loop(
    mut sink: futures_util::stream::SplitSink<WebSocket, Message>,
    mut rx: mpsc::Receiver<Message>,
) {
    while let Some(m) = rx.recv().await {
        if sink.send(m).await.is_err() {
            return;
        }
    }
}

/// Traite un message texte : parse JSON-RPC → dispatch →
/// émission de la réponse (JSON ou trame binaire). Les
/// notifications (`id` absent) sont dispatchées sans réponse —
/// feu et oubli du protocole.
async fn handle(
    st: &Arc<AgentShared>,
    conn: &mut AgentConn,
    text: &str,
    tx: &mpsc::Sender<Message>,
) {
    let incoming = match serde_json::from_str::<Incoming>(text) {
        Ok(v) => v,
        Err(e) => {
            // Méthode connue mais params invalides → -32602 ;
            // nom inconnu → -32601 ; JSON illisible → -32700.
            let code = classify_parse_error(text);
            // L'id est recopié quand il est lisible — un client
            // n'attend jamais `null` sauf JSON vraiment illisible.
            let id = serde_json::from_str::<Value>(text)
                .ok()
                .and_then(|v| v.get("id").cloned())
                .unwrap_or(Value::Null);
            send_json(tx, &RpcResponse::err(id, code, format!("{e}"), None));
            return;
        }
    };
    let (id, method, is_request) = match incoming {
        Incoming::Request { id, method } => (id, method, true),
        Incoming::Notification { method } => (Value::Null, method, false),
    };
    match dispatch(st, conn, &id, method).await {
        Dispatch::Reply(Ok(result)) if is_request => {
            send_json(tx, &RpcResponse::ok(id, result));
        }
        Dispatch::Reply(Err(e)) if is_request => {
            let code = ErrorCode::try_from(e.code).unwrap_or(ErrorCode::EngineUnavailable);
            send_json(tx, &RpcResponse::err(id, code, e.message, e.data));
        }
        Dispatch::Binary(bytes) => {
            let _ = tx.try_send(Message::Binary(bytes.into()));
        }
        Dispatch::Reply(_) => {} // notification : rien à répondre
    }
}

/// Erreur de parse → code — la distinction tient au JSON brut :
/// `method` est-elle un nom du contrat ?
fn classify_parse_error(text: &str) -> ErrorCode {
    let Ok(v) = serde_json::from_str::<Value>(text) else {
        return ErrorCode::Parse;
    };
    // `method` absent ou inconnu → -32601 (le serveur n'a pas
    // de variante -32600 : l'erreur la plus parlante l'emporte,
    // le client corrige son catalogue).
    let Some(m) = v.get("method").and_then(Value::as_str) else {
        return ErrorCode::MethodNotFound;
    };
    if KNOWN_METHODS.contains(&m) {
        ErrorCode::InvalidParams
    } else {
        ErrorCode::MethodNotFound
    }
}

/// Noms de méthodes du contrat — double du `rename` de
/// [`Method`], vérifié par test (drift = cassure de parse).
const KNOWN_METHODS: &[&str] = &[
    "session.open",
    "session.close",
    "session.ping",
    "tab.open",
    "tab.close",
    "tab.list",
    "nav.goto",
    "nav.reload",
    "nav.back",
    "nav.forward",
    "nav.viewport",
    "snapshot.get",
    "query.select",
    "frame.get",
    "console.get",
    "dom.get",
    "input.click",
    "input.type",
    "input.hover",
    "input.key",
    "input.scroll",
    "input.click_at",
    "eval.run",
    "wait.for",
    "events.subscribe",
    "events.unsubscribe",
    "engine.j17",
];

/// Émet une réponse JSON — file bornée, `try_send` : un client
/// qui ne lit pas n'a pas droit à la mémoire du daemon.
fn send_json(tx: &mpsc::Sender<Message>, r: &RpcResponse) {
    if let Ok(s) = serde_json::to_string(r) {
        let _ = tx.try_send(Message::Text(s.into()));
    }
}

/// Forwarder : bus `CoreEvent` → notifications filtrées. Le
/// `rev` `nav.committed`/`snapshot.changed` est lu depuis
/// `AgentShared` (writer unique = recorder — le forwarder ne
/// fait que relayer, jamais incrémenter).
async fn forwarder(st: Arc<AgentShared>, subs: Arc<RwLock<SubFilter>>, tx: mpsc::Sender<Message>) {
    let mut rx = st.session.lock().await.subscribe();
    loop {
        let ev = match rx.recv().await {
            Ok(ev) => ev,
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
            Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
        };
        for ev in map_event(&st, &ev) {
            let (kind, tab) = kind_of(&ev);
            let pass = {
                let f = subs
                    .read()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                f.kinds.contains(&kind) && f.tab.is_none_or(|t| t == tab)
            };
            if !pass {
                continue;
            }
            if let Ok(s) = serde_json::to_string(&Notification::new(ev)) {
                if tx.try_send(Message::Text(s.into())).is_err() {
                    return; // file pleine ou close — le client est mort
                }
            }
        }
    }
}

/// `CoreEvent` → liste d'[`Event`] — un événement moteur peut
/// produire deux notifications (commit = nav + révision de
/// snapshot).
fn map_event(st: &AgentShared, ev: &CoreEvent) -> Vec<Event> {
    match ev {
        CoreEvent::Navigating { tab, url } => {
            vec![Event::NavStarted {
                tab: *tab,
                url: url.clone(),
            }]
        }
        CoreEvent::Navigated { tab, url, title } => {
            let rev = st.rev(*tab);
            vec![
                Event::NavCommitted {
                    tab: *tab,
                    url: url.clone(),
                    title: title.clone(),
                    rev,
                },
                Event::SnapshotChanged { tab: *tab, rev },
            ]
        }
        CoreEvent::FrameInvalidated {
            tab,
            rects,
            scroll_y,
        } => {
            vec![Event::FrameInvalidated {
                tab: *tab,
                rects: rects
                    .iter()
                    .map(|r| crate::proto::Rect {
                        x: r.x,
                        y: r.y,
                        w: r.w,
                        h: r.h,
                    })
                    .collect(),
                scroll_y: *scroll_y,
            }]
        }
        CoreEvent::PipelineError {
            tab: Some(tab),
            message,
        } => {
            vec![Event::ConsoleEntry {
                tab: *tab,
                message: message.clone(),
            }]
        }
        CoreEvent::TabOpened { tab } => vec![Event::TabOpened { tab: *tab }],
        CoreEvent::TabClosed { tab } => vec![Event::TabClosed { tab: *tab }],
        _ => Vec::new(),
    }
}

/// Famille + onglet d'un événement (filtre de souscription).
fn kind_of(ev: &Event) -> (EventKind, u64) {
    match ev {
        Event::NavStarted { tab, .. } | Event::NavCommitted { tab, .. } => (EventKind::Nav, *tab),
        Event::FrameInvalidated { tab, .. } => (EventKind::Frame, *tab),
        Event::ConsoleEntry { tab, .. } => (EventKind::ConsoleLog, *tab),
        Event::SnapshotChanged { tab, .. } => (EventKind::Snapshot, *tab),
        Event::TabOpened { tab } | Event::TabClosed { tab } => (EventKind::Tabs, *tab),
    }
}
