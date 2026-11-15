// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Dispatch des verbes EAP vers l'API publique de `Session` —
//! jamais de couture parallèle vers le pipeline (même règle que
//! l'API UI). Les méthodes hors phase répondent
//! [`ErrorCode::Unsupported`] : réponse légale du contrat, pas
//! un trou masqué.

use std::sync::Arc;
use std::time::Duration;

use explorer_core::{
    drain_applies_shared, drain_navs_shared, eval_shared, handle_input_shared, history_move_shared,
    navigate_shared, CoreError, CoreEvent, EvalReply, InputEvent, InputOutcome,
    KeyMods as CoreKeyMods, Session,
};
use explorer_net::http::{Initiator, Method as NetMethod, Request as NetRequest};
use explorer_net::url::Url;
use serde_json::{json, Value};

use crate::proto::{Capability, ErrorCode, FrameParams, KeyMods, Method, RpcError, WaitKind};
use crate::snapshot::nodes_from_geometry;
use crate::state::{AgentConn, AgentShared, SubFilter};

/// Timeout de navigation par défaut quand `wait` n'en porte
/// pas (règle 13 — nommé, pas en dur).
const NAV_WAIT_DEFAULT_MS: u64 = 30_000;

/// Capacités **implémentées** par cette build — l'intersection
/// avec les demandées fait la liste livrée. Les absents sont
/// les phases futures (E2 `handles`/`actionable`, E3
/// `snapshot_diff`).
const IMPLEMENTED: &[Capability] = &[
    Capability::Snapshot,
    Capability::WaitCommit,
    Capability::Input,
    Capability::Eval,
    Capability::Frames,
    Capability::ConsoleLog,
    Capability::Dom,
    Capability::Tabs,
    Capability::Events,
];

/// Capacité requise par méthode — `None` = toujours servie
/// (cycle de vie `session.*`).
fn cap_of(m: &Method) -> Option<Capability> {
    match m {
        Method::SessionOpen(_) | Method::SessionClose(_) | Method::SessionPing(_) => None,
        Method::TabOpen(_)
        | Method::TabClose(_)
        | Method::TabList(_)
        | Method::NavGoto(_)
        | Method::NavReload(_)
        | Method::NavBack(_)
        | Method::NavForward(_)
        | Method::NavViewport(_) => Some(Capability::Tabs),
        Method::SnapshotGet(_) => Some(Capability::Snapshot),
        Method::QuerySelect(_)
        | Method::InputClick(_)
        | Method::InputType(_)
        | Method::InputHover(_) => Some(Capability::Handles),
        Method::FrameGet(_) => Some(Capability::Frames),
        Method::ConsoleGet(_) => Some(Capability::ConsoleLog),
        Method::DomGet(_) => Some(Capability::Dom),
        Method::InputKey(_) | Method::InputScroll(_) | Method::InputClickAt(_) => {
            Some(Capability::Input)
        }
        Method::EvalRun(_) => Some(Capability::Eval),
        Method::WaitFor(_) => Some(Capability::WaitCommit),
        Method::EventsSubscribe(_) | Method::EventsUnsubscribe(_) => Some(Capability::Events),
    }
}

/// Nom stable de capacité pour `unsupported` — le libellé du
/// contrat, pas `{:?}` d'un enum interne.
fn cap_name(c: Capability) -> &'static str {
    match c {
        Capability::Snapshot => "snapshot",
        Capability::SnapshotDiff => "snapshot_diff",
        Capability::Handles => "handles",
        Capability::Actionable => "actionable",
        Capability::WaitCommit => "wait_commit",
        Capability::Input => "input",
        Capability::Eval => "eval",
        Capability::Frames => "frames",
        Capability::ConsoleLog => "console_log",
        Capability::Dom => "dom",
        Capability::Tabs => "tabs",
        Capability::Events => "events",
    }
}

/// Sortie du dispatch : réponse JSON, réponse binaire (frame),
/// ou fin de connexion demandée.
pub enum Dispatch {
    /// `{result}|{error}` — sérialisé par l'appelant.
    Reply(Result<Value, RpcError>),
    /// Trame binaire prête (`id|kind|payload` encodés).
    Binary(Vec<u8>),
}

fn err(code: ErrorCode, message: impl Into<String>) -> RpcError {
    RpcError {
        code: code.into(),
        message: message.into(),
        data: None,
    }
}
fn err_data(code: ErrorCode, message: impl Into<String>, data: Value) -> RpcError {
    RpcError {
        code: code.into(),
        message: message.into(),
        data: Some(data),
    }
}
fn unsupported(what: &str, cap: &'static str) -> RpcError {
    err_data(
        ErrorCode::Unsupported,
        format!("{what} — capacité non livrée"),
        json!({ "capability": cap }),
    )
}

/// `CoreError` → `RpcError` — les types qui comptent gardent leur
/// code, le reste dégrade en `EngineUnavailable` (jamais de
/// panic sur erreur moteur).
impl From<CoreError> for RpcError {
    fn from(e: CoreError) -> Self {
        match &e {
            CoreError::UnknownTab(t) => {
                err_data(ErrorCode::NoSuchTab, e.to_string(), json!({ "tab": t }))
            }
            CoreError::Url(_) | CoreError::EmptyDocument => {
                err(ErrorCode::InvalidParams, e.to_string())
            }
            CoreError::Timeout(_) => err(ErrorCode::Timeout, e.to_string()),
            _ => err(ErrorCode::EngineUnavailable, e.to_string()),
        }
    }
}

/// `tab` dans la portée du profil ?
fn scope(conn: &AgentConn, tab: u64) -> Result<(), RpcError> {
    if conn.tab_allowed(tab) {
        Ok(())
    } else {
        Err(err_data(
            ErrorCode::Unauthorized,
            format!("onglet {tab} hors portée du profil"),
            json!({ "tab": tab }),
        ))
    }
}

/// L'onglet existe dans la session ?
async fn tab_exists(st: &Arc<AgentShared>, tab: u64) -> Result<(), RpcError> {
    let session = st.session.lock().await;
    if session.tab_info(tab).is_some() {
        Ok(())
    } else {
        Err(err_data(
            ErrorCode::NoSuchTab,
            format!("onglet inconnu : {tab}"),
            json!({ "tab": tab }),
        ))
    }
}

/// Attend un événement de la session satisfaisant `pred`, borné
/// par `dur` — abonnement frais sur le broadcast (buffered : les
/// événements émis entre l'appel et la souscription sont lus si
/// le receveur souscrit avant — l'appelant souscrit **avant**
/// l'action qui déclenche).
async fn await_event(
    session: &Arc<tokio::sync::Mutex<Session>>,
    dur: Duration,
    pred: impl Fn(&CoreEvent) -> bool,
) -> Result<CoreEvent, RpcError> {
    let mut rx = session.lock().await.subscribe();
    let wait = async {
        loop {
            match rx.recv().await {
                Ok(ev) if pred(&ev) => return Ok(ev),
                Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}

                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    return Err(err(ErrorCode::EngineUnavailable, "bus d'événements fermé"));
                }
            }
        }
    };
    tokio::time::timeout(dur, wait)
        .await
        .map_err(|_| err(ErrorCode::Timeout, "attente échue".to_string()))?
}

/// `wait=frame` après commit : la prochaine
/// `frame.invalidated` de l'onglet dans le budget restant.
async fn settle_wait(
    st: &Arc<AgentShared>,
    tab: u64,
    wait: Option<WaitKind>,
    timeout_ms: Option<u32>,
) -> Result<(), RpcError> {
    match wait {
        Some(WaitKind::Frame) => {
            let dur = Duration::from_millis(timeout_ms.map_or(NAV_WAIT_DEFAULT_MS, u64::from));
            await_event(
                &st.session,
                dur,
                |ev| matches!(ev, CoreEvent::FrameInvalidated { tab: t, .. } if *t == tab),
            )
            .await
            .map(|_| ())
        }
        Some(WaitKind::Idle | WaitKind::Selector | WaitKind::Stable) => Err(unsupported(
            "wait.for hors commit/frame — E2",
            "wait_commit",
        )),
        _ => Ok(()),
    }
}

/// Navigation partagée + drain — le pattern des handlers REST
/// (`navigate_shared` hors verrou, `drain_navs_shared` pour les
/// intentions posées en vol).
async fn do_nav(
    st: &Arc<AgentShared>,
    tab: u64,
    req: NetRequest,
    record: bool,
) -> Result<Value, RpcError> {
    Box::pin(navigate_shared(&st.session, tab, req, record))
        .await
        .map_err(RpcError::from)?;
    Box::pin(drain_navs_shared(&st.session, tab)).await;
    let session = st.session.lock().await;
    let t = session.tab_info(tab);
    match t {
        Some(t) => Ok(json!({
            "url": t.url, "title": t.title, "rev": st.rev(tab)
        })),
        None => Err(RpcError::from(CoreError::UnknownTab(tab))),
    }
}

/// Dispatch d'une requête — `id` sert à corréler la réponse
/// binaire éventuelle (`frame.get` exige un `id` entier).
pub async fn dispatch(
    st: &Arc<AgentShared>,
    conn: &mut AgentConn,
    id: &Value,
    method: Method,
) -> Dispatch {
    // session.open est la porte — tout le reste exige l'open.
    if !conn.opened && !matches!(method, Method::SessionOpen(_) | Method::SessionPing(_)) {
        return Dispatch::Reply(Err(err(
            ErrorCode::Unauthorized,
            "session.open requis avant tout verbe métier",
        )));
    }
    if let Some(cap) = cap_of(&method) {
        if !conn.caps.contains(&cap) {
            return Dispatch::Reply(Err(unsupported("verbe", cap_name(cap))));
        }
    }
    // `frame.get` répond en trame binaire, pas en JSON-RPC —
    // le chemin `run` n'est pas emprunté.
    if let Method::FrameGet(p) = &method {
        return match frame_binary(st, conn, id, p).await {
            Ok(bytes) => Dispatch::Binary(bytes),
            Err(e) => Dispatch::Reply(Err(e)),
        };
    }
    Dispatch::Reply(run(st, conn, id, method).await)
}

/// `frame.get` → trame `[u32 id][u32 kind=1][png]` encodée —
/// `id` entier exigé (corrélation côté client).
async fn frame_binary(
    st: &Arc<AgentShared>,
    conn: &AgentConn,
    id: &Value,
    params: &FrameParams,
) -> Result<Vec<u8>, RpcError> {
    scope(conn, params.tab)?;
    tab_exists(st, params.tab).await?;
    let id_num = id
        .as_u64()
        .and_then(|v| u32::try_from(v).ok())
        .ok_or_else(|| {
            err(
                ErrorCode::InvalidParams,
                "frame.get exige un id entier 32 bits — corrélation de la trame binaire",
            )
        })?;
    let png = {
        let mut s = st.session.lock().await;
        let _ = s.tick_animations(params.tab);
        if params.full == Some(true) {
            s.frame_png_full(params.tab)
        } else {
            // Région px CSS → px physiques (`frame_png_region`
            // attend des px physiques, même contrat que REST).
            let scale = s.scale().max(f32::EPSILON);
            if let Some(clip) = params.clip {
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                // px CSS ≥ 0 × scale fini → px physiques.
                let (rx, ry, rw, rh) = (
                    (clip.x * scale).round().max(0.0) as u32,
                    (clip.y * scale).round().max(0.0) as u32,
                    (clip.w * scale).round().max(1.0) as u32,
                    (clip.h * scale).round().max(1.0) as u32,
                );
                s.frame_png_region(params.tab, rx, ry, rw, rh)
            } else {
                let (vw, vh) = s.viewport();
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let (pw, ph) = (
                    (vw * scale).round().max(0.0) as u32,
                    (vh * scale).round().max(0.0) as u32,
                );
                s.frame_png_region(params.tab, 0, 0, pw.max(1), ph.max(1))
                    .or_else(|| s.frame_png(params.tab))
            }
        }
    };
    let png = png.filter(|b| !b.is_empty()).ok_or_else(|| {
        err(
            ErrorCode::EngineUnavailable,
            "pas de frame — rasterisation en vol",
        )
    })?;
    let mut out = Vec::with_capacity(crate::proto::binary::HEADER + png.len());
    crate::proto::binary::encode(id_num, crate::proto::binary::Kind::PngFrame, &png, &mut out);
    Ok(out)
}

/// Corps du dispatch — supposé open + capabilité vérifiés.
/// Corps du dispatch — supposé open + capabilité vérifiés.
/// Route par groupe sémantique ; chaque sous-fonction porte
/// ses bras (découpage pour la lisibilité du lint `too_many_lines`).
async fn run(
    st: &Arc<AgentShared>,
    conn: &mut AgentConn,
    _id: &Value,
    method: Method,
) -> Result<Value, RpcError> {
    match method {
        Method::SessionOpen(_) | Method::SessionPing(_) | Method::SessionClose(_) => {
            session_verbs(conn, method)
        }
        Method::TabOpen(_) | Method::TabClose(_) | Method::TabList(_) => {
            tab_verbs(st, conn, method).await
        }
        Method::NavGoto(_)
        | Method::NavReload(_)
        | Method::NavBack(_)
        | Method::NavForward(_)
        | Method::NavViewport(_) => nav_verbs(st, conn, method).await,
        Method::SnapshotGet(_)
        | Method::QuerySelect(_)
        | Method::FrameGet(_)
        | Method::ConsoleGet(_)
        | Method::DomGet(_) => perceive_verbs(st, conn, method).await,
        Method::InputClick(_)
        | Method::InputType(_)
        | Method::InputHover(_)
        | Method::InputKey(_)
        | Method::InputScroll(_)
        | Method::InputClickAt(_)
        | Method::EvalRun(_) => act_verbs(st, conn, method).await,
        Method::WaitFor(_) | Method::EventsSubscribe(_) | Method::EventsUnsubscribe(_) => {
            wait_verbs(st, conn, method).await
        }
    }
}

/// Cycle de vie — handshake, liveness, close.
fn session_verbs(conn: &mut AgentConn, method: Method) -> Result<Value, RpcError> {
    match method {
        Method::SessionOpen(p) => {
            if let Some(proto) = p.protocol.as_deref() {
                if proto != crate::PROTOCOL {
                    return Err(err(
                        ErrorCode::InvalidParams,
                        format!("protocol {} attendu, reçu {proto}", crate::PROTOCOL),
                    ));
                }
            }
            let caps: std::collections::HashSet<Capability> = p.capabilities.map_or_else(
                || IMPLEMENTED.iter().copied().collect(),
                |v| v.into_iter().filter(|c| IMPLEMENTED.contains(c)).collect(),
            );
            conn.caps = caps.iter().copied().collect();
            conn.profile = p.profile.unwrap_or_default();
            conn.opened = true;
            Ok(json!({
                "protocol": crate::PROTOCOL,
                "capabilities": caps,
                "profile": conn.profile,
            }))
        }
        Method::SessionPing(_) => Ok(json!({
            "ts_ms": std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| {
                    u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
                })
        })),
        Method::SessionClose(_) => {
            conn.closing = true;
            Ok(json!({}))
        }

        // Verbe hors groupe — garde-fou interne (le dispatch
        // en amont route par construction ; si ça arrive, la
        // dégradation honnête est une erreur moteur).
        _ => Err(err(
            ErrorCode::EngineUnavailable,
            "verbe hors groupe — routage interne",
        )),
    }
}

/// Onglets — cycle de vie sous portée du profil.
async fn tab_verbs(
    st: &Arc<AgentShared>,
    conn: &mut AgentConn,
    method: Method,
) -> Result<Value, RpcError> {
    match method {
        Method::TabOpen(_) => {
            if !conn.may_open_tab() {
                return Err(err(
                    ErrorCode::Unauthorized,
                    "portée tabs bornée — ouverture d'onglet refusée",
                ));
            }
            let mut s = st.session.lock().await;
            if let Some(retry) = s.shed_refusal() {
                return Err(RpcError::from(CoreError::Overloaded(retry)));
            }
            let tab = s.open_tab();
            Ok(json!({ "tab": tab }))
        }
        Method::TabClose(p) => {
            scope(conn, p.tab)?;
            let mut s = st.session.lock().await;
            s.close_tab(p.tab).map_err(RpcError::from)?;
            conn.handles.purge_tab(p.tab);
            Ok(json!({}))
        }
        Method::TabList(_) => {
            let s = st.session.lock().await;
            let tabs: Vec<_> = s
                .tab_infos()
                .iter()
                .filter(|t| conn.tab_allowed(t.id))
                .map(|t| {
                    json!({
                        "tab": t.id, "url": t.url, "title": t.title,
                        "scroll_y": t.scroll_y,
                    })
                })
                .collect();
            Ok(json!({ "tabs": tabs }))
        }

        // Verbe hors groupe — garde-fou interne (le dispatch
        // en amont route par construction ; si ça arrive, la
        // dégradation honnête est une erreur moteur).
        _ => Err(err(
            ErrorCode::EngineUnavailable,
            "verbe hors groupe — routage interne",
        )),
    }
}

/// Navigation — goto/reload/historique/viewport.
async fn nav_verbs(
    st: &Arc<AgentShared>,
    conn: &mut AgentConn,
    method: Method,
) -> Result<Value, RpcError> {
    match method {
        Method::NavGoto(p) => {
            scope(conn, p.tab)?;
            tab_exists(st, p.tab).await?;
            // wait hors commit/frame → refus AVANT de naviguer :
            // une action qu'on sait ne pas pouvoir honorer ne
            // doit pas produire d'effet de bord.
            if matches!(
                p.wait,
                Some(WaitKind::Idle | WaitKind::Selector | WaitKind::Stable)
            ) {
                return Err(unsupported("wait hors commit/frame — E2", "wait_commit"));
            }
            let url =
                Url::parse(&p.url).map_err(|e| err(ErrorCode::InvalidParams, e.to_string()))?;
            let mut req = NetRequest::get(url);
            req.initiator = Some(Initiator::TopLevelNav);
            let r = do_nav(st, p.tab, req, true).await?;
            settle_wait(st, p.tab, p.wait, p.timeout_ms).await?;
            Ok(r)
        }
        Method::NavReload(p) => {
            scope(conn, p.tab)?;
            if matches!(
                p.wait,
                Some(WaitKind::Idle | WaitKind::Selector | WaitKind::Stable)
            ) {
                return Err(unsupported("wait hors commit/frame — E2", "wait_commit"));
            }
            let req = {
                let s = st.session.lock().await;
                s.reload_request(p.tab).map_err(RpcError::from)?
            };
            if !matches!(req.method, NetMethod::Get | NetMethod::Head) {
                // Resoumission non sûre — même règle que REST :
                // l'agent doit rejouer une navigation explicite.
                return Err(err_data(
                    ErrorCode::InvalidParams,
                    "resubmit_confirm — méthode non sûre, rejouer nav.goto",
                    json!({ "method": req.method.as_str(), "url": req.url.to_string() }),
                ));
            }
            let r = do_nav(st, p.tab, req, false).await?;
            settle_wait(st, p.tab, p.wait, p.timeout_ms).await?;
            Ok(r)
        }
        Method::NavBack(p) => hist(st, conn, p.tab, -1, p.wait, p.timeout_ms).await,
        Method::NavForward(p) => hist(st, conn, p.tab, 1, p.wait, p.timeout_ms).await,
        Method::NavViewport(p) => {
            scope(conn, p.tab)?;
            tab_exists(st, p.tab).await?;
            let mut s = st.session.lock().await;
            #[allow(clippy::cast_precision_loss)]
            s.set_viewport(p.w as f32, p.h as f32, p.scale.unwrap_or(1.0));
            let _ = s.rerender(p.tab);
            Ok(json!({}))
        }

        // Verbe hors groupe — garde-fou interne (le dispatch
        // en amont route par construction ; si ça arrive, la
        // dégradation honnête est une erreur moteur).
        _ => Err(err(
            ErrorCode::EngineUnavailable,
            "verbe hors groupe — routage interne",
        )),
    }
}

/// Perception — snapshot, DOM, console, frame.
async fn perceive_verbs(
    st: &Arc<AgentShared>,
    conn: &mut AgentConn,
    method: Method,
) -> Result<Value, RpcError> {
    match method {
        Method::SnapshotGet(p) => {
            scope(conn, p.tab)?;
            if p.since.is_some() {
                return Err(unsupported("snapshot diff — E3", "snapshot_diff"));
            }
            tab_exists(st, p.tab).await?;
            let report = {
                let s = st.session.lock().await;
                s.geometry_report(p.tab)
            };
            let Some(report) = report else {
                return Err(err(
                    ErrorCode::EngineUnavailable,
                    "pas de géométrie — arbre détaché ou onglet vierge",
                ));
            };
            let rev = st.rev(p.tab);
            let url = {
                let s = st.session.lock().await;
                s.tab_info(p.tab).and_then(|t| t.url).unwrap_or_default()
            };
            let nodes = nodes_from_geometry(
                &report,
                &mut conn.handles,
                p.tab,
                rev,
                p.interactive_only.unwrap_or(true),
                p.max_nodes.unwrap_or(4096) as usize,
            );
            Ok(json!({ "rev": rev, "url": url, "nodes": nodes }))
        }
        Method::QuerySelect(_) => Err(unsupported("query.select — handles E2", "handles")),
        Method::FrameGet(_) => Err(err(
            ErrorCode::EngineUnavailable,
            "frame.get devait être court-circuité avant run",
        )),
        Method::ConsoleGet(p) => {
            scope(conn, p.tab)?;
            tab_exists(st, p.tab).await?;
            Ok(json!({
                "entries": st.console_entries(p.tab, p.limit.unwrap_or(512) as usize)
            }))
        }
        Method::DomGet(p) => {
            scope(conn, p.tab)?;
            tab_exists(st, p.tab).await?;
            let mut s = st.session.lock().await;
            s.reclaim_orphaned_paint(p.tab);
            match s.dom_dump(p.tab) {
                Some(d) => Ok(json!({ "html": d })),
                None => Err(err(
                    ErrorCode::EngineUnavailable,
                    "pas de DOM — apply en vol, réessayer",
                )),
            }
        }

        // Verbe hors groupe — garde-fou interne (le dispatch
        // en amont route par construction ; si ça arrive, la
        // dégradation honnête est une erreur moteur).
        _ => Err(err(
            ErrorCode::EngineUnavailable,
            "verbe hors groupe — routage interne",
        )),
    }
}

/// Actions — input et eval sous droits du profil.
async fn act_verbs(
    st: &Arc<AgentShared>,
    conn: &mut AgentConn,
    method: Method,
) -> Result<Value, RpcError> {
    match method {
        Method::InputClick(_) | Method::InputType(_) | Method::InputHover(_) => {
            Err(unsupported("action par handle — E2", "handles"))
        }
        Method::InputKey(p) => {
            scope(conn, p.tab)?;
            right_input(conn)?;
            tab_exists(st, p.tab).await?;
            let outcome = send_input(
                st,
                p.tab,
                InputEvent::Key {
                    key: p.key,
                    mods: mods(p.mods),
                },
            )
            .await?;
            Ok(outcome_json(outcome))
        }
        Method::InputScroll(p) => {
            scope(conn, p.tab)?;
            right_input(conn)?;
            if p.elem_ref.is_some() {
                return Err(unsupported("scroll_into_view — handles E2", "handles"));
            }
            tab_exists(st, p.tab).await?;
            let outcome = send_input(
                st,
                p.tab,
                InputEvent::Scroll {
                    dy: p.dy.unwrap_or(0.0),
                },
            )
            .await?;
            Ok(outcome_json(outcome))
        }
        Method::InputClickAt(p) => {
            scope(conn, p.tab)?;
            right_input(conn)?;
            tab_exists(st, p.tab).await?;
            let outcome = send_input(st, p.tab, InputEvent::Click { x: p.x, y: p.y }).await?;
            let mut v = outcome_json(outcome);
            // Coordonnées = échappatoire : toujours marqué
            // degraded (pas de handle, pas de checks — contrat).
            v["result"] = json!("degraded");
            v["degraded"] = json!("click_at — pas de handle ni de checks");
            Ok(v)
        }

        Method::EvalRun(p) => {
            scope(conn, p.tab)?;
            right_eval(conn)?;
            tab_exists(st, p.tab).await?;
            match eval_shared(&st.session, p.tab, p.expr).await {
                EvalReply::Done { value, .. } => {
                    let (mut val, mut er) = match &value {
                        Ok(v) => (Some(v.clone()), None),
                        Err(e) => (None, Some(e.clone())),
                    };
                    let mut truncated = false;
                    if let Some(v) = &val {
                        if v.len() > 64_000 {
                            truncated = true;
                            val = Some(v.chars().take(64_000).collect());
                        }
                    }
                    let _ = &mut er;
                    Ok(json!({
                        "ok": value.is_ok(), "value": val, "error": er,
                        "truncated": truncated,
                    }))
                }
                EvalReply::Unavailable => Err(err(
                    ErrorCode::EngineUnavailable,
                    "moteur JS indisponible — pompe en vol",
                )),
            }
        }

        // Verbe hors groupe — garde-fou interne (le dispatch
        // en amont route par construction ; si ça arrive, la
        // dégradation honnête est une erreur moteur).
        _ => Err(err(
            ErrorCode::EngineUnavailable,
            "verbe hors groupe — routage interne",
        )),
    }
}

/// Attente et souscriptions aux événements.
async fn wait_verbs(
    st: &Arc<AgentShared>,
    conn: &mut AgentConn,
    method: Method,
) -> Result<Value, RpcError> {
    match method {
        Method::WaitFor(p) => {
            scope(conn, p.tab)?;
            tab_exists(st, p.tab).await?;
            let dur = Duration::from_millis(u64::from(p.timeout_ms));
            match p.kind {
                WaitKind::Commit => await_event(
                    &st.session,
                    dur,
                    |ev| matches!(ev, CoreEvent::Navigated { tab: t, .. } if *t == p.tab),
                )
                .await
                .map(|_| json!({ "ok": true })),
                WaitKind::Frame => await_event(
                    &st.session,
                    dur,
                    |ev| matches!(ev, CoreEvent::FrameInvalidated { tab: t, .. } if *t == p.tab),
                )
                .await
                .map(|_| json!({ "ok": true })),
                WaitKind::Idle | WaitKind::Selector | WaitKind::Stable => Err(unsupported(
                    "wait.for hors commit/frame — E2",
                    "wait_commit",
                )),
            }
        }

        Method::EventsSubscribe(p) => {
            if let Some(t) = p.tab {
                scope(conn, t)?;
            }
            let mut subs = conn
                .subs
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            subs.tab = p.tab;
            subs.kinds = p.kinds.iter().copied().collect();
            Ok(json!({ "subscribed": p.kinds }))
        }
        Method::EventsUnsubscribe(p) => {
            if let Some(t) = p.tab {
                scope(conn, t)?;
            }
            let mut subs = conn
                .subs
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            match p.tab {
                Some(_) => subs.kinds.clear(),
                None => *subs = SubFilter::default(),
            }
            Ok(json!({}))
        }

        // Verbe hors groupe — garde-fou interne (le dispatch
        // en amont route par construction ; si ça arrive, la
        // dégradation honnête est une erreur moteur).
        _ => Err(err(
            ErrorCode::EngineUnavailable,
            "verbe hors groupe — routage interne",
        )),
    }
}

/// Historique ±1 — partagé back/forward.
async fn hist(
    st: &Arc<AgentShared>,
    conn: &AgentConn,
    tab: u64,
    delta: isize,
    wait: Option<WaitKind>,
    timeout_ms: Option<u32>,
) -> Result<Value, RpcError> {
    scope(conn, tab)?;
    tab_exists(st, tab).await?;
    if matches!(
        wait,
        Some(WaitKind::Idle | WaitKind::Selector | WaitKind::Stable)
    ) {
        return Err(unsupported("wait hors commit/frame — E2", "wait_commit"));
    }
    Box::pin(history_move_shared(&st.session, tab, delta))
        .await
        .map_err(RpcError::from)?;
    Box::pin(drain_navs_shared(&st.session, tab)).await;
    settle_wait(st, tab, wait, timeout_ms).await?;
    let s = st.session.lock().await;
    match s.tab_info(tab) {
        Some(t) => Ok(json!({ "url": t.url, "title": t.title, "rev": st.rev(tab) })),
        None => Err(RpcError::from(CoreError::UnknownTab(tab))),
    }
}

/// Droit `input` du profil.
fn right_input(conn: &AgentConn) -> Result<(), RpcError> {
    if conn.profile.input {
        Ok(())
    } else {
        Err(err_data(
            ErrorCode::Unauthorized,
            "profil sans droit input",
            json!({ "required": "input" }),
        ))
    }
}
/// Droit `eval` du profil.
fn right_eval(conn: &AgentConn) -> Result<(), RpcError> {
    if conn.profile.eval {
        Ok(())
    } else {
        Err(err_data(
            ErrorCode::Unauthorized,
            "profil sans droit eval",
            json!({ "required": "eval" }),
        ))
    }
}

/// Envoie l'input et lance les drains détachés — même motif que
/// le handler REST `/input` (enfilement synchrone, apply/nav
/// hors attente).
async fn send_input(
    st: &Arc<AgentShared>,
    tab: u64,
    ev: InputEvent,
) -> Result<InputOutcome, RpcError> {
    let outcome = handle_input_shared(&st.session, tab, ev)
        .await
        .map_err(RpcError::from)?;
    let sess = Arc::clone(&st.session);
    tokio::spawn(async move {
        drain_applies_shared(&sess, tab).await;
        Box::pin(drain_navs_shared(&sess, tab)).await;
    });
    Ok(outcome)
}

fn mods(m: Option<KeyMods>) -> CoreKeyMods {
    m.map_or(CoreKeyMods::NONE, |k| CoreKeyMods {
        ctrl: k.ctrl,
        shift: k.shift,
        alt: k.alt,
        meta: k.meta,
    })
}

/// `InputOutcome` → `actionResult` — le résultat brut vit dans
/// `outcome`, `result` est la forme du contrat.
fn outcome_json(o: InputOutcome) -> Value {
    let (result, extra) = match o {
        InputOutcome::Ignored => ("ignored", json!({})),
        InputOutcome::Scrolled { scroll_y } => ("scrolled", json!({ "scroll_y": scroll_y })),
        InputOutcome::Navigated => ("navigated", json!({})),
        InputOutcome::Focused => ("focused", json!({})),
        InputOutcome::Edited => ("edited", json!({})),
        InputOutcome::Unfocused => ("ignored", json!({ "outcome": "unfocused" })),
        InputOutcome::Hover { url, editable } => (
            "done",
            json!({ "outcome": "hover", "url": url, "editable": editable }),
        ),
    };
    let mut v = json!({ "result": result });
    if let Some(obj) = extra.as_object() {
        for (k, x) in obj {
            v[k] = x.clone();
        }
    }
    v
}
