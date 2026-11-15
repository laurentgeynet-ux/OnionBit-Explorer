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

use crate::actionable::{auto_wait, resolve, POLL_MS};
use crate::proto::{
    Actionability, Capability, ErrorCode, FrameParams, KeyMods, Method, RpcError, WaitKind,
};
use crate::state::{AgentConn, AgentShared, StoredSnap, SubFilter};

/// Timeout de navigation par défaut quand `wait` n'en porte
/// pas (règle 13 — nommé, pas en dur).
const NAV_WAIT_DEFAULT_MS: u64 = 30_000;

/// Capacités **implémentées** par cette build — l'intersection
/// avec les demandées fait la liste livrée. Surface v1
/// E1–E3 servie ; les futures extensions s'ajoutent ici.
const IMPLEMENTED: &[Capability] = &[
    Capability::Snapshot,
    Capability::SnapshotDiff,
    Capability::Handles,
    Capability::Actionable,
    Capability::WaitCommit,
    Capability::Input,
    Capability::Eval,
    Capability::Frames,
    Capability::ConsoleLog,
    Capability::Dom,
    Capability::Tabs,
    Capability::Events,
    Capability::Engine,
    Capability::Net,
];

/// Index compact servi à `session.open` (étape 248) —
/// question d'agent → verbe(s), filtré par les capacités
/// livrées. Un contexte LLM tient ~15 lignes de plan au
/// lieu d'explorer le catalogue ; la réponse « où est le
/// bug » reste aux verbes, l'index ne fait que guider.
const HELP: &[(&str, Option<Capability>, &str)] = &[
    (
        "que voit l'agent ?",
        Some(Capability::Snapshot),
        "snapshot.get",
    ),
    (
        "diff depuis le dernier snapshot ?",
        Some(Capability::SnapshotDiff),
        "snapshot.get{since|since_seq}",
    ),
    (
        "suspects d'affichage / santé ?",
        Some(Capability::Engine),
        "engine.anomalies",
    ),
    (
        "boîte posée, absente ou 0x0 ?",
        Some(Capability::Engine),
        "engine.layout",
    ),
    (
        "où partent octets et état ?",
        Some(Capability::Engine),
        "engine.pipeline",
    ),
    (
        "où part le temps par phase ?",
        Some(Capability::Engine),
        "engine.pipeline{phases,reset}",
    ),
    (
        "combien d'items émis/dessinés ?",
        Some(Capability::Engine),
        "engine.paint_stats",
    ),
    (
        "d'où vient ce style ?",
        Some(Capability::Engine),
        "engine.cascade",
    ),
    (
        "quelles requêtes réseau ?",
        Some(Capability::Net),
        "net.events{since_seq}",
    ),
    (
        "quoi de repeint à la frame ?",
        Some(Capability::Engine),
        "frame.diff",
    ),
    (
        "erreurs pipeline ?",
        Some(Capability::ConsoleLog),
        "console.get",
    ),
    ("DOM brut ?", Some(Capability::Dom), "dom.get"),
    ("image peinte ?", Some(Capability::Frames), "frame.get"),
    (
        "onglets ?",
        Some(Capability::Tabs),
        "tab.list|tab.open|tab.close",
    ),
    (
        "naviguer ?",
        Some(Capability::Tabs),
        "nav.goto|nav.back|nav.forward|nav.reload",
    ),
    (
        "taille du viewport ?",
        Some(Capability::Tabs),
        "nav.viewport",
    ),
    (
        "trouver un élément ?",
        Some(Capability::Handles),
        "query.select",
    ),
    (
        "cliquer, taper, survoler ?",
        Some(Capability::Input),
        "input.click|input.type|input.hover",
    ),
    (
        "touches, scroll, coordonnées ?",
        Some(Capability::Input),
        "input.key|input.scroll|input.click_at",
    ),
    ("évaluer du JS ?", Some(Capability::Eval), "eval.run"),
    (
        "attendre un état ?",
        Some(Capability::WaitCommit),
        "wait.for",
    ),
    (
        "s'abonner aux événements ?",
        Some(Capability::Events),
        "events.subscribe|events.unsubscribe",
    ),
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
        Method::EngineLayout(_)
        | Method::EnginePipeline(_)
        | Method::EnginePaintStats(_)
        | Method::EngineCascade(_)
        | Method::EngineAnomalies(_)
        | Method::FrameDiff(_) => Some(Capability::Engine),
        Method::NetEvents(_) => Some(Capability::Net),
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
        Capability::Engine => "engine",
        Capability::Net => "net",
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

/// `wait:frame` — le moteur peint en **pull** (Étape 222) :
/// headless, personne ne consomme les frames, et les
/// `FrameInvalidated` de la nav (peinture spéculative +
/// repeinture de commit) partent *pendant* `navigate_shared`
/// — attendre un événement futur serait un timeout garanti.
/// Sémantique honnête : « le document commis a une frame
/// peinte » — sonde `frame_pixels` bornée, puis
/// `tick_animations` pour armer le pull consommateur (la
/// pompe anim se dégate pour les clients qui lisent).
async fn wait_frame(st: &Arc<AgentShared>, tab: u64, dur: Duration) -> Result<(), RpcError> {
    let deadline = std::time::Instant::now() + dur;
    loop {
        let ready = {
            let mut s = st.session.lock().await;
            // Le wait arme le pull au lieu de sonder à
            // l'aveugle : orphelin garé → réinséré ; DOM
            // commis non peint → produit paresseusement.
            // Sans ça `frame_pixels` reste `None` à vie sur
            // des états réparables (timeout garanti).
            s.ensure_frame(tab);
            if s.frame_pixels(tab).is_some() {
                let _ = s.tick_animations(tab);
                true
            } else {
                false
            }
        };
        if ready {
            return Ok(());
        }
        if std::time::Instant::now() >= deadline {
            return Err(err(ErrorCode::Timeout, "attente échue".to_string()));
        }
        tokio::time::sleep(Duration::from_millis(POLL_MS)).await;
    }
}

/// `wait=frame` après commit : la prochaine frame peinte du
/// document, dans le budget restant.
async fn settle_wait(
    st: &Arc<AgentShared>,
    tab: u64,
    wait: Option<WaitKind>,
    timeout_ms: Option<u32>,
) -> Result<(), RpcError> {
    match wait {
        Some(WaitKind::Frame) => {
            let dur = Duration::from_millis(timeout_ms.map_or(NAV_WAIT_DEFAULT_MS, u64::from));
            wait_frame(st, tab, dur).await
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
        // Orphelin garé → réinséré + DOM commis non peint →
        // produit (même garde que `wait_frame`) : sans ça
        // `frame.get` répond « pas de frame » sur un état
        // réparable.
        s.ensure_frame(params.tab);
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
        | Method::DomGet(_)
        | Method::EngineLayout(_)
        | Method::EnginePipeline(_)
        | Method::EnginePaintStats(_)
        | Method::EngineCascade(_)
        | Method::EngineAnomalies(_)
        | Method::NetEvents(_)
        | Method::FrameDiff(_) => perceive_verbs(st, conn, method).await,
        Method::InputClick(_) | Method::InputType(_) | Method::InputHover(_) => {
            handle_verbs(st, conn, method).await
        }
        Method::InputKey(_)
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
            // Index compact (étape 248) — question→verbes,
            // filtré par les capacités livrées : un agent
            // tient ~15 lignes en contexte au lieu d'explorer.
            let help: serde_json::Map<String, Value> = HELP
                .iter()
                .filter(|(_, cap, _)| cap.is_none_or(|c| caps.contains(&c)))
                .map(|(q, _, v)| ((*q).to_string(), json!(v)))
                .collect();
            Ok(json!({
                "protocol": crate::PROTOCOL,
                "capabilities": caps,
                "profile": conn.profile,
                "help": help,
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
            conn.snaps.remove(&p.tab);
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

/// `snapshot.get` — arbre sémantique E3 (rôle/nom/états/
/// depth réels) ou diff `{since}` contre la dernière
/// émission à cette `rev`. Base absente ou périmée →
/// snapshot complet honnête, jamais d'erreur.
async fn snapshot_get(
    st: &Arc<AgentShared>,
    conn: &mut AgentConn,
    p: crate::proto::SnapshotParams,
) -> Result<Value, RpcError> {
    scope(conn, p.tab)?;
    // Le diff exige la capacité dédiée — négociée au
    // `session.open`, sinon `-32001` honnête.
    if (p.since.is_some() || p.since_seq.is_some())
        && !conn.caps.contains(&Capability::SnapshotDiff)
    {
        return Err(unsupported("snapshot diff", "snapshot_diff"));
    }
    tab_exists(st, p.tab).await?;
    // Un commit nav n'implique pas un DOM attaché : l'apply
    // qui suit détache l'arbre un instant. On attend la
    // re-attach bornée (~500 ms) plutôt que de rendre un
    // `-32007` transitoire à l'agent — au-delà, erreur honnête.
    let mut attempts = 0u8;
    let (entries, report, url) = loop {
        let mut s = st.session.lock().await;
        // Orphelin garé → réinséré avant la sonde : sinon
        // `EmptyDocument` persiste et le retry ne sert à
        // rien (même garde que les waiters).
        s.reclaim_orphaned_paint(p.tab);
        match s.snapshot_entries(p.tab) {
            Ok(entries) => {
                let report = s.geometry_report(p.tab);
                let url = s.tab_info(p.tab).and_then(|t| t.url).unwrap_or_default();
                break (entries, report, url);
            }
            Err(CoreError::UnknownTab(_)) => {
                return Err(err(ErrorCode::NoSuchTab, "onglet inconnu"));
            }
            Err(e) => {
                drop(s);
                attempts += 1;
                if attempts >= 10 {
                    return Err(err(ErrorCode::EngineUnavailable, e.to_string()));
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
    };
    let rev = st.rev(p.tab);
    let emitted = crate::snapshot::emit(
        &entries,
        report.as_ref(),
        &mut conn.handles,
        p.tab,
        rev,
        p.interactive_only.unwrap_or(true),
        p.max_nodes.unwrap_or(4096) as usize,
    );
    // Base du diff (étape 254 — ring borné d'émissions) :
    // `since_seq` vise l'émission exacte ; `since` garde sa
    // sémantique « dernière émission à cette rev » — on
    // cherche de la plus récente à la plus ancienne.
    let ring = conn.snaps.get(&p.tab);
    let base = p
        .since_seq
        .and_then(|seq| ring.and_then(|r| r.iter().find(|s| s.snap_seq == seq)))
        .or_else(|| {
            p.since
                .and_then(|since| ring.and_then(|r| r.iter().rev().find(|s| s.rev == since)))
        });
    // Diff servi — base absente ou élaguée (jamais snapshotée,
    // nav entre-temps, historique plein) dégrade en snapshot
    // complet, jamais d'erreur.
    let snap_seq = conn.snap_seq;
    let result = if let Some(snap) = base {
        let (added, removed, changed) = crate::snapshot::diff(&snap.entries, &emitted);
        json!({
            "rev": rev, "base_rev": snap.rev, "base_seq": snap.snap_seq,
            "snap_seq": snap_seq, "url": url,
            "added": added, "removed": removed,
            "changed": changed,
        })
    } else {
        let nodes: Vec<_> = emitted.iter().map(|e| e.node.clone()).collect();
        json!({ "rev": rev, "snap_seq": snap_seq, "url": url, "nodes": nodes })
    };
    conn.snap_seq += 1;
    let ring = conn.snaps.entry(p.tab).or_default();
    ring.push_back(StoredSnap {
        snap_seq,
        rev,
        entries: emitted
            .into_iter()
            .map(|e| crate::snapshot::SnapEntry {
                index: e.index,
                generation: e.generation,
                elem_ref: e.node.elem_ref,
                sig: e.sig,
            })
            .collect(),
    });
    while ring.len() > crate::state::SNAP_HISTORY {
        ring.pop_front();
    }
    Ok(result)
}

/// Perception — snapshot, DOM, console, frame.
async fn perceive_verbs(
    st: &Arc<AgentShared>,
    conn: &mut AgentConn,
    method: Method,
) -> Result<Value, RpcError> {
    match method {
        Method::SnapshotGet(p) => snapshot_get(st, conn, p).await,
        Method::QuerySelect(p) => {
            scope(conn, p.tab)?;
            tab_exists(st, p.tab).await?;
            let indices = {
                let s = st.session.lock().await;
                s.select_nodes(p.tab, &p.selector).map_err(|e| match e {
                    CoreError::Selector(_) => err(ErrorCode::BadSelector, e.to_string()),
                    other => RpcError::from(other),
                })?
            };
            let rev = st.rev(p.tab);
            let refs: Vec<_> = indices
                .iter()
                .take(p.max.unwrap_or(64) as usize)
                .map(|&i| conn.handles.alloc(p.tab, i, rev))
                .collect();
            Ok(json!({ "refs": refs }))
        }
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
        Method::EngineLayout(p) => {
            scope(conn, p.tab)?;
            tab_exists(st, p.tab).await?;
            let mut s = st.session.lock().await;
            s.reclaim_orphaned_paint(p.tab);
            match s.layout_dump(p.tab) {
                Some(d) => Ok(json!({ "dump": d })),
                None => Err(err(
                    ErrorCode::EngineUnavailable,
                    "pas d'arbre de layout — apply en vol, réessayer",
                )),
            }
        }
        Method::EnginePipeline(p) => {
            scope(conn, p.tab)?;
            tab_exists(st, p.tab).await?;
            let mut s = st.session.lock().await;
            match s.pipeline_report(p.tab, p.reset.unwrap_or(false)) {
                Some(r) => Ok(r),
                None => Err(err(ErrorCode::NoSuchTab, "onglet inconnu")),
            }
        }
        Method::EnginePaintStats(p) => {
            scope(conn, p.tab)?;
            tab_exists(st, p.tab).await?;
            let mut s = st.session.lock().await;
            s.reclaim_orphaned_paint(p.tab);
            match s.paint_stats(p.tab) {
                Some(r) => Ok(r),
                None => Err(err(ErrorCode::NoSuchTab, "onglet inconnu")),
            }
        }
        Method::EngineAnomalies(p) => {
            scope(conn, p.tab)?;
            tab_exists(st, p.tab).await?;
            let max = p
                .max
                .map_or_else(crate::anomalies::default_max, |m| m as usize);
            crate::anomalies::run(st, conn, p.tab, max).await
        }
        Method::EngineCascade(p) => engine_cascade_verb(st, conn, p).await,
        Method::NetEvents(_) | Method::FrameDiff(_) => net_frame_verbs(st, conn, method).await,
        // Verbe hors groupe — garde-fou interne (le dispatch
        // en amont route par construction ; si ça arrive, la
        // dégradation honnête est une erreur moteur).
        _ => Err(err(
            ErrorCode::EngineUnavailable,
            "verbe hors groupe — routage interne",
        )),
    }
}

/// `net.events` + `frame.diff` (étapes 251/252) — sortis de
/// `perceive_verbs` pour la borne de taille ; même discipline
/// (scope onglet + existence + lecture sous verrou).
async fn net_frame_verbs(
    st: &Arc<AgentShared>,
    conn: &mut AgentConn,
    method: Method,
) -> Result<Value, RpcError> {
    match method {
        Method::NetEvents(p) => {
            scope(conn, p.tab)?;
            tab_exists(st, p.tab).await?;
            let s = st.session.lock().await;
            match s.net_events(p.tab, p.since_seq) {
                Some(r) => Ok(r),
                None => Err(err(ErrorCode::NoSuchTab, "onglet inconnu")),
            }
        }
        Method::FrameDiff(p) => {
            scope(conn, p.tab)?;
            tab_exists(st, p.tab).await?;
            let mut s = st.session.lock().await;
            s.reclaim_orphaned_paint(p.tab);
            match s.frame_diff(p.tab) {
                Some(r) => Ok(r),
                None => Err(err(ErrorCode::NoSuchTab, "onglet inconnu")),
            }
        }
        _ => Err(err(
            ErrorCode::EngineUnavailable,
            "verbe hors groupe — routage interne",
        )),
    }
}

/// `engine.cascade` — verdicts de cascade d'un élément par
/// sa ref (étape 253). `StaleRef` comme les autres verbes
/// `ref` ; `EngineUnavailable` si l'état n'est pas encore
/// committé (première nav en vol ou nœud mort entre la
/// résolution de la ref et la lecture — la génération est
/// re-validée côté session).
async fn engine_cascade_verb(
    st: &Arc<AgentShared>,
    conn: &mut AgentConn,
    p: crate::proto::CascadeParams,
) -> Result<Value, RpcError> {
    scope(conn, p.tab)?;
    tab_exists(st, p.tab).await?;
    let handle = resolve(conn, st, p.tab, &p.elem_ref)?;
    let s = st.session.lock().await;
    match s.engine_cascade(p.tab, handle.node) {
        Some(r) => Ok(r),
        None => Err(err(
            ErrorCode::EngineUnavailable,
            "pas d'état committé — navigation en vol, réessayer",
        )),
    }
}

/// Actions par handle — auto-wait d'actionabilité avant
/// d'agir (ADR-0035 §6) : les verbes `ref` vivent ici,
/// séparés des actions directes par souci de taille.
async fn handle_verbs(
    st: &Arc<AgentShared>,
    conn: &mut AgentConn,
    method: Method,
) -> Result<Value, RpcError> {
    match method {
        Method::InputClick(p) => {
            scope(conn, p.tab)?;
            right_input(conn)?;
            tab_exists(st, p.tab).await?;
            if p.button.as_deref().is_some_and(|b| b != "left") {
                return Err(unsupported("bouton non-left — E3", "handles"));
            }
            // `mods` non transmis : `InputEvent::Click` n'en
            // porte pas (v1 — le modifieur clavier vit dans
            // input.key, pas dans le click atomique).
            let handle = resolve(conn, st, p.tab, &p.elem_ref)?;
            let opts = p.opts.unwrap_or_default();
            let (probe, action, degraded) = auto_wait(st, p.tab, handle, &opts).await?;
            let scroll_y = scroll_y(st, p.tab).await;
            let Some((cx, cy)) = probe.center_viewport(scroll_y) else {
                return Err(not_actionable(
                    "pas de centre cliquable — élément non posé",
                    action,
                ));
            };
            let outcome = send_input(st, p.tab, InputEvent::Click { x: cx, y: cy }).await?;
            Ok(action_json(outcome, action, degraded))
        }
        Method::InputType(p) => {
            scope(conn, p.tab)?;
            right_input(conn)?;
            tab_exists(st, p.tab).await?;
            type_into_field(st, conn, p).await
        }
        Method::InputHover(p) => {
            scope(conn, p.tab)?;
            right_input(conn)?;
            tab_exists(st, p.tab).await?;
            let handle = resolve(conn, st, p.tab, &p.elem_ref)?;
            let opts = p.opts.unwrap_or_default();
            let (probe, action, degraded) = auto_wait(st, p.tab, handle, &opts).await?;
            let scroll_y = scroll_y(st, p.tab).await;
            let Some((cx, cy)) = probe.center_viewport(scroll_y) else {
                return Err(not_actionable("pas de centre — non posé", action));
            };
            let outcome = send_input(st, p.tab, InputEvent::MouseMove { x: cx, y: cy }).await?;
            // `input.hover` rend `{url, editable}` du contrat.
            let (url, editable) = match &outcome {
                InputOutcome::Hover { url, editable } => (url.clone(), *editable),
                _ => (None, false),
            };
            let mut v = action_json(outcome, action, degraded);
            v["url"] = json!(url);
            v["editable"] = json!(editable);
            Ok(v)
        }
        // Garde-fou interne — le routage en amont est exhaustif.
        _ => Err(err(
            ErrorCode::EngineUnavailable,
            "verbe hors groupe — routage interne",
        )),
    }
}

/// `input.type` — focus par clic natif puis frappe
/// caractère par caractère (les handlers JS voient une
/// vraie frappe, pas une injection de valeur).
async fn type_into_field(
    st: &Arc<AgentShared>,
    conn: &mut AgentConn,
    p: crate::proto::TypeParams,
) -> Result<Value, RpcError> {
    let handle = resolve(conn, st, p.tab, &p.elem_ref)?;
    let opts = p.opts.unwrap_or_default();
    let (probe, action, degraded) = auto_wait(st, p.tab, handle, &opts).await?;
    // `Editable` : input/textarea, ou `contenteditable`
    // explicite — le check `enabled` est déjà dans
    // l'auto-wait.
    let editable = matches!(probe.tag.as_str(), "input" | "textarea")
        || probe.attr("contenteditable") == Some("true")
        || probe.attr("contenteditable") == Some("");
    if !editable {
        return Err(not_actionable("élément non éditable", action));
    }
    let scroll_y = scroll_y(st, p.tab).await;
    let Some((cx, cy)) = probe.center_viewport(scroll_y) else {
        return Err(not_actionable("pas de centre — non posé", action));
    };
    // Focus par le clic natif, puis frappe caractère
    // par caractère (Key+KeyUp — pas d'injection de
    // valeur, les handlers JS voient une vraie frappe).
    send_input(st, p.tab, InputEvent::Click { x: cx, y: cy }).await?;
    if p.clear {
        send_input(
            st,
            p.tab,
            InputEvent::Key {
                key: "a".into(),
                mods: CoreKeyMods {
                    ctrl: true,
                    ..CoreKeyMods::NONE
                },
            },
        )
        .await?;
        send_input(
            st,
            p.tab,
            InputEvent::Key {
                key: "Delete".into(),
                mods: CoreKeyMods::NONE,
            },
        )
        .await?;
    }
    for ch in p.text.chars() {
        let key = ch.to_string();
        send_input(
            st,
            p.tab,
            InputEvent::Key {
                key: key.clone(),
                mods: CoreKeyMods::NONE,
            },
        )
        .await?;
        send_input(
            st,
            p.tab,
            InputEvent::KeyUp {
                key,
                mods: CoreKeyMods::NONE,
            },
        )
        .await?;
    }
    Ok(action_json(InputOutcome::Edited, action, degraded))
}

/// Actions directes — input par coordonnées/focus et eval
/// sous droits du profil.
async fn act_verbs(
    st: &Arc<AgentShared>,
    conn: &mut AgentConn,
    method: Method,
) -> Result<Value, RpcError> {
    match method {
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
            tab_exists(st, p.tab).await?;
            if let Some(r) = &p.elem_ref {
                let handle = resolve(conn, st, p.tab, r)?;
                let probe = crate::actionable::probe(st, p.tab, handle.node).await?;
                let Some(rect) = probe.rect else {
                    return Err(not_actionable(
                        "élément non posé — scroll_into_view impossible",
                        Actionability::default(),
                    ));
                };
                let (scroll_y, vh) = {
                    let s = st.session.lock().await;
                    (
                        s.tab_info(p.tab).map_or(0.0, |t| t.scroll_y),
                        s.viewport().1,
                    )
                };
                // Centrer la boîte dans le viewport.
                let dy = (rect.y + rect.h / 2.0) - scroll_y - vh / 2.0;
                let outcome = send_input(st, p.tab, InputEvent::Scroll { dy }).await?;
                return Ok(outcome_json(outcome));
            }
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
                // Pull-model : `wait_frame`, pas un event —
                // le repaint post-action est synchrone sous
                // le verrou, son `FrameInvalidated` est déjà
                // parti quand l'appel arrive.
                WaitKind::Frame => wait_frame(st, p.tab, dur)
                    .await
                    .map(|()| json!({ "ok": true })),
                WaitKind::Selector => {
                    let Some(sel) = p.selector.as_deref() else {
                        return Err(err(
                            ErrorCode::InvalidParams,
                            "kind=selector exige params.selector",
                        ));
                    };
                    wait_selector(st, p.tab, sel, dur).await
                }
                WaitKind::Stable => wait_stable(st, p.tab, dur).await,
                WaitKind::Idle => Err(unsupported("wait.idle — hooks réseau E4", "wait_commit")),
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

/// Scroll courant de l'onglet (0.0 hors info — l'action
/// dégrade en coordonnées document, jamais de panic).
async fn scroll_y(st: &Arc<AgentShared>, tab: u64) -> f32 {
    st.session
        .lock()
        .await
        .tab_info(tab)
        .map_or(0.0, |t| t.scroll_y)
}

/// `NotActionable` typé avec le constat dans `data`.
fn not_actionable(message: impl Into<String>, a: Actionability) -> RpcError {
    err_data(
        ErrorCode::NotActionable,
        message,
        json!({ "actionability": a }),
    )
}

/// `actionResult` complet — outcome brut + actionabilité
/// constatée + marqueur `degraded` le cas échéant.
fn action_json(o: InputOutcome, a: Actionability, degraded: bool) -> Value {
    let mut v = outcome_json(o);
    v["actionability"] = json!(a);
    if degraded {
        v["degraded"] = json!("force — checks jamais atteints, action au dernier rect");
    }
    v
}

/// `wait.for{kind:selector}` — polling borné sur la sélection
/// native : non-vide → ok ; `EmptyDocument`/`UnknownTab`
/// comptent comme « pas encore » (la page peut être en vol).
async fn wait_selector(
    st: &Arc<AgentShared>,
    tab: u64,
    selector: &str,
    dur: Duration,
) -> Result<Value, RpcError> {
    // Le sélecteur est validé une fois — un `BadSelector` est
    // une erreur d'appel, pas une condition d'attente. Un
    // document en vol/détaché n'est pas une erreur d'appel
    // non plus : le poll attendra sa pose (orphelin garé
    // réclamé d'abord — même garde que la boucle).
    {
        let mut s = st.session.lock().await;
        s.reclaim_orphaned_paint(tab);
        if let Err(e) = s.select_nodes(tab, selector) {
            match e {
                CoreError::Selector(_) => {
                    return Err(err(ErrorCode::BadSelector, e.to_string()));
                }
                CoreError::EmptyDocument | CoreError::UnknownTab(_) => {}
                other => return Err(RpcError::from(other)),
            }
        }
    }
    let deadline = tokio::time::Instant::now() + dur;
    loop {
        {
            let mut s = st.session.lock().await;
            // Orphelin garé → réinséré avant la sonde : sans
            // ça `select_nodes` rend `EmptyDocument` à vie et
            // l'attente timeout sur un état réparable.
            s.reclaim_orphaned_paint(tab);
            if let Ok(indices) = s.select_nodes(tab, selector) {
                if !indices.is_empty() {
                    return Ok(json!({ "ok": true }));
                }
            }
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(err(ErrorCode::Timeout, "selector jamais satisfait"));
        }
        tokio::time::sleep(Duration::from_millis(POLL_MS)).await;
    }
}

/// `wait.for{kind:stable}` — deux lectures de géométrie
/// consécutives identiques (signature `(node,x,y,w,h)`).
async fn wait_stable(st: &Arc<AgentShared>, tab: u64, dur: Duration) -> Result<Value, RpcError> {
    let deadline = tokio::time::Instant::now() + dur;
    let mut prev = None::<u64>;
    loop {
        let sig = {
            let mut s = st.session.lock().await;
            // Même garde que `wait_selector` — orphelin garé
            // réinséré avant de lire la géométrie.
            s.reclaim_orphaned_paint(tab);
            s.geometry_report(tab).map(|r| {
                // FNV-1a sur les boîtes — signature compacte,
                // pas de clone du rapport.
                let mut h = 0xcbf2_9ce4_8422_2325u64;
                for b in &r.boxes {
                    for f in [b.x, b.y, b.w, b.h] {
                        for byte in f.to_le_bytes() {
                            h = (h ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3);
                        }
                    }
                    h = (h ^ u64::from(b.node)).wrapping_mul(0x0000_0100_0000_01b3);
                }
                h
            })
        };
        if let (Some(a), Some(b)) = (prev, sig) {
            if a == b {
                return Ok(json!({ "ok": true }));
            }
        }
        if sig.is_some() {
            prev = sig;
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(err(ErrorCode::Timeout, "layout jamais stable"));
        }
        tokio::time::sleep(Duration::from_millis(POLL_MS)).await;
    }
}
