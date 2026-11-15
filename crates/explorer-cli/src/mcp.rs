// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! `explorer-cli mcp` — **adaptateur** MCP stdio ↔ EAP
//! (ADR-0035 §8 : MCP est un adaptateur interchangeable,
//! jamais le protocole). Chaque outil MCP est traduit en
//! verbe EAP sur le WebSocket `/agent` du daemon — handles
//! `el:N`, auto-wait d'actionabilité, snapshot sémantique,
//! trame PNG binaire, tout passe par le contrat unique.
//! Hand-rolled (pas de crate MCP) : le stdio porte la
//! confiance ; la clé `EXPLORER_API_KEY` est transmise en
//! `?key=` quand le daemon en exige une.

use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message;

const PROTOCOL_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];
const FALLBACK_VERSION: &str = "2024-11-05";

/// Borne d'attente d'une réponse EAP — les `wait.for` agents
/// peuvent durer ; le verbe porte sa propre deadline.
const CALL_TIMEOUT: Duration = Duration::from_secs(300);

/// Boucle stdio : une trame JSON-RPC par ligne, jusqu'à EOF.
/// Le runtime tokio est local au pont — stdin bloquant vit
/// dans un thread lecteur, le socket WS dans le runtime.
pub fn serve(port: u16) -> std::io::Result<u8> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(serve_async(port))
}

/// Réponse attendue pour un `id` en vol — le lecteur route
/// le texte JSON-RPC et les trames binaires (PNG `frame.get`)
/// sur la même table.
enum Reply {
    /// Réponse JSON-RPC (résultat ou erreur typée).
    Json(Value),
    /// Charge binaire — en-tête déjà décodé, c'est le PNG.
    Binary(Vec<u8>),
}

/// Client EAP minimal : un socket, une file d'émission, une
/// table `oneshot` par requête en vol, une tâche lectrice.
struct Eap {
    out: mpsc::UnboundedSender<Message>,
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Reply>>>>,
    next: AtomicU64,
}

impl Eap {
    /// Connecte le socket (`eap/1` sous-protocole requis par
    /// le serveur) puis `session.open` : toutes les capacités
    /// v1, profil input+eval — l'adaptateur pilote le
    /// navigateur pour un agent, pas pour observer.
    async fn connect(port: u16) -> Result<Self, String> {
        let mut url = format!("ws://127.0.0.1:{port}/agent");
        if let Ok(key) = std::env::var("EXPLORER_API_KEY") {
            url = format!("{url}?key={key}");
        }
        let mut req =
            tokio_tungstenite::tungstenite::client::IntoClientRequest::into_client_request(&url)
                .map_err(|e| format!("requête ws : {e}"))?;
        req.headers_mut().insert(
            "Sec-WebSocket-Protocol",
            explorer_agent::PROTOCOL
                .parse()
                .map_err(|_| "protocole".to_string())?,
        );
        let (sock, _) = tokio_tungstenite::connect_async(req)
            .await
            .map_err(|e| format!("daemon {url} injoignable : {e}"))?;
        let (mut sink, mut stream) = sock.split();
        let (out, mut out_rx) = mpsc::unbounded_channel::<Message>();
        let pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Reply>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        // Writer — un seul émetteur sur le sink.
        tokio::spawn(async move {
            while let Some(m) = out_rx.recv().await {
                if sink.send(m).await.is_err() {
                    return;
                }
            }
        });
        // Reader — route texte (id) et binaire (request_id)
        // vers la oneshot de l'appelant ; les notifications
        // `event.*` sont ignorées (l'adaptateur n'y souscrit
        // pas — tools/call est du request/response).
        let pend = Arc::clone(&pending);
        tokio::spawn(async move {
            while let Some(msg) = stream.next().await {
                match msg {
                    Ok(Message::Text(t)) => {
                        let Ok(v) = serde_json::from_str::<Value>(&t) else {
                            continue;
                        };
                        let Some(id) = v.get("id").and_then(Value::as_u64) else {
                            continue;
                        };
                        let tx = pend
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .remove(&id);
                        if let Some(tx) = tx {
                            let _ = tx.send(Reply::Json(v));
                        }
                    }
                    Ok(Message::Binary(b)) => {
                        if let Some((id, _, payload)) = explorer_agent::proto::binary::decode(&b) {
                            let tx = pend
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner)
                                .remove(&u64::from(id));
                            if let Some(tx) = tx {
                                let _ = tx.send(Reply::Binary(payload.to_vec()));
                            }
                        }
                    }
                    Ok(Message::Close(_)) | Err(_) => return,
                    Ok(_) => {}
                }
            }
        });
        let eap = Self {
            out,
            pending,
            next: AtomicU64::new(1),
        };
        eap.call(
            "session.open",
            json!({
                "protocol": explorer_agent::PROTOCOL,
                "profile": {"input": true, "eval": true},
            }),
        )
        .await
        .map_err(|e| format!("session.open refusé : {e}"))?;
        Ok(eap)
    }

    /// Envoie une requête et attend sa réponse — JSON ou
    /// binaire selon ce qui arrive pour l'`id`.
    async fn request(&self, method: &str, params: Value) -> Result<Reply, String> {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(id, tx);
        let req = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        if self
            .out
            .send(Message::Text(req.to_string().into()))
            .is_err()
        {
            return Err("socket agent fermé".to_string());
        }
        match tokio::time::timeout(CALL_TIMEOUT, rx).await {
            Ok(Ok(r)) => Ok(r),
            Ok(Err(_)) => Err("lecteur socket mort".to_string()),
            Err(_) => Err(format!("{method} — timeout adaptateur")),
        }
    }

    /// Appel dont la réponse attendue est JSON — l'erreur EAP
    /// typée remonte telle quelle (code + message).
    async fn call(&self, method: &str, params: Value) -> Result<Value, String> {
        match self.request(method, params).await? {
            Reply::Json(v) => match (v.get("result"), v.get("error")) {
                (Some(r), _) => Ok(r.clone()),
                (None, Some(e)) => Err(format!(
                    "eap {} : {}",
                    e["code"].as_i64().unwrap_or(0),
                    e["message"].as_str().unwrap_or("erreur")
                )),
                _ => Err("réponse sans result ni error".to_string()),
            },
            Reply::Binary(_) => Err("trame binaire inattendue".to_string()),
        }
    }

    /// Appel dont la réponse attendue est une trame binaire —
    /// `frame.get` n'émet pas de JSON.
    async fn call_binary(&self, method: &str, params: Value) -> Result<Vec<u8>, String> {
        match self.request(method, params).await? {
            Reply::Binary(b) => Ok(b),
            Reply::Json(v) => Err(format!(
                "eap {} : {}",
                v["error"]["code"].as_i64().unwrap_or(0),
                v["error"]["message"].as_str().unwrap_or("erreur")
            )),
        }
    }
}

async fn serve_async(port: u16) -> std::io::Result<u8> {
    let eap = Eap::connect(port).await.map_err(std::io::Error::other)?;
    let eap = Arc::new(eap);
    // Stdin bloquant → canal tokio.
    let (lines_tx, mut lines) = mpsc::unbounded_channel::<std::io::Result<String>>();
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            let stop = line.is_err();
            if lines_tx.send(line).is_err() || stop {
                return;
            }
        }
    });
    let stdout = std::io::stdout();
    while let Some(line) = lines.recv().await {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let req: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                let err = json!({"jsonrpc":"2.0","id":null,
                    "error":{"code":-32700,"message":format!("parse: {e}")}});
                let mut out = stdout.lock();
                writeln!(out, "{err}")?;
                out.flush()?;
                continue;
            }
        };
        if let Some(resp) = handle(&eap, &req).await {
            let mut out = stdout.lock();
            writeln!(out, "{resp}")?;
            out.flush()?;
        }
    }
    // EOF : close honnête — l'agent part, les onglets
    // survivent (contrat `session.close`).
    let _ = eap.call("session.close", json!({})).await;
    Ok(0)
}

/// Une requête → une réponse ; `None` pour les notifications.
async fn handle(eap: &Arc<Eap>, req: &Value) -> Option<Value> {
    let method = req["method"].as_str()?;
    let id = req.get("id").cloned();
    let result = |r: Value| {
        id.clone()
            .map(|i| json!({"jsonrpc":"2.0","id":i,"result":r}))
    };
    let error = |code: i64, message: String| {
        id.clone()
            .map(|i| json!({"jsonrpc":"2.0","id":i,"error":{"code":code,"message":message}}))
    };
    match method {
        "initialize" => {
            let want = req["params"]["protocolVersion"].as_str().unwrap_or("");
            let v = if PROTOCOL_VERSIONS.contains(&want) {
                want
            } else {
                FALLBACK_VERSION
            };
            result(json!({
                "protocolVersion": v,
                "capabilities": {"tools": {"listChanged": false}},
                "serverInfo": {"name": "explorer-eap", "version": env!("CARGO_PKG_VERSION")},
            }))
        }
        "ping" => result(json!({})),
        m if m.starts_with("notifications/") => None,
        "tools/list" => result(json!({"tools": catalog()})),
        "tools/call" => {
            let name = req["params"]["name"].as_str().unwrap_or("");
            let args = req["params"].get("arguments").cloned().unwrap_or(json!({}));
            Some(call_tool(eap, id.as_ref(), name, &args).await)
        }
        _ => error(-32601, format!("méthode inconnue : {method}")),
    }
}

/// Catalogue d'outils — surface agent EAP, forme
/// Playwright-MCP : les actions passent par `ref` (handle
/// serveur), `click_at` reste l'échappatoire dégradée.
fn tools() -> Value {
    let t = |name: &str, desc: &str, schema: Value| json!({"name": name, "description": desc, "inputSchema": schema});
    let tab =
        || json!({"type":"object","properties":{"tab":{"type":"integer"}},"required":["tab"]});
    let obj =
        |props: Value, req: &[&str]| json!({"type":"object","properties":props,"required":req});
    let nav = json!([
        t("tab_new", "Ouvre un onglet → {tab}", obj(json!({}), &[])),
        t("tab_close", "Ferme l'onglet {tab}", tab()),
        t("tabs", "Onglets ouverts", obj(json!({}), &[])),
        t(
            "navigate",
            "Navigue {tab,url,wait?:commit|frame}",
            obj(
                json!({"tab":{"type":"integer"},"url":{"type":"string"},"wait":{"type":"string"},"timeout_ms":{"type":"integer"}}),
                &["tab", "url"]
            )
        ),
        t("back", "Historique arrière {tab}", tab()),
        t("forward", "Historique avant {tab}", tab()),
        t("reload", "Recharge {tab}", tab()),
        t(
            "viewport",
            "Règle le viewport {tab,w,h}",
            obj(
                json!({"tab":{"type":"integer"},"w":{"type":"integer"},"h":{"type":"integer"}}),
                &["tab", "w", "h"]
            )
        ),
        t(
            "snapshot",
            "Arbre sémantique → {nodes|diff} {tab,since?}",
            obj(
                json!({"tab":{"type":"integer"},"since":{"type":"integer"},"interactive_only":{"type":"boolean"},"max_nodes":{"type":"integer"}}),
                &["tab"]
            )
        ),
        t(
            "select",
            "Sélecteur CSS → refs {tab,selector}",
            obj(
                json!({"tab":{"type":"integer"},"selector":{"type":"string"},"max":{"type":"integer"}}),
                &["tab", "selector"]
            )
        ),
    ]);
    nav
}

/// Catalogue actions + perception lourde — seconde moitié
/// (borne de lignes clippy, aucune sémantique de tri).
fn act_tools() -> Value {
    let t = |name: &str, desc: &str, schema: Value| json!({"name": name, "description": desc, "inputSchema": schema});
    let tab =
        || json!({"type":"object","properties":{"tab":{"type":"integer"}},"required":["tab"]});
    let obj =
        |props: Value, req: &[&str]| json!({"type":"object","properties":props,"required":req});
    json!([
        t(
            "click",
            "Clic par ref, auto-wait {tab,ref}",
            obj(
                json!({"tab":{"type":"integer"},"ref":{"type":"string"},"timeout_ms":{"type":"integer"}}),
                &["tab", "ref"]
            )
        ),
        t(
            "click_at",
            "Clic coordonnées — dégradé {tab,x,y}",
            obj(
                json!({"tab":{"type":"integer"},"x":{"type":"number"},"y":{"type":"number"}}),
                &["tab", "x", "y"]
            )
        ),
        t(
            "type",
            "Frappe dans le champ {tab,ref,text,clear?}",
            obj(
                json!({"tab":{"type":"integer"},"ref":{"type":"string"},"text":{"type":"string"},"clear":{"type":"boolean"},"timeout_ms":{"type":"integer"}}),
                &["tab", "ref", "text"]
            )
        ),
        t(
            "hover",
            "Survol par ref {tab,ref}",
            obj(
                json!({"tab":{"type":"integer"},"ref":{"type":"string"}}),
                &["tab", "ref"]
            )
        ),
        t(
            "scroll",
            "{tab,ref?} amène la ref au viewport, sinon {tab,dy}",
            obj(
                json!({"tab":{"type":"integer"},"ref":{"type":"string"},"dy":{"type":"number"}}),
                &["tab"]
            )
        ),
        t(
            "key",
            "Touche {tab,key} (\"Enter\",\"a\",\"Control+a\"…)",
            obj(
                json!({"tab":{"type":"integer"},"key":{"type":"string"}}),
                &["tab", "key"]
            )
        ),
        t(
            "wait_for",
            "Attend {tab,kind:commit|frame|selector|stable,selector?,timeout_ms?}",
            obj(
                json!({"tab":{"type":"integer"},"kind":{"type":"string"},"selector":{"type":"string"},"timeout_ms":{"type":"integer"}}),
                &["tab", "kind"]
            )
        ),
        t(
            "screenshot",
            "PNG moteur → fichier {tab,path,full?}",
            obj(
                json!({"tab":{"type":"integer"},"path":{"type":"string"},"full":{"type":"boolean"}}),
                &["tab", "path"]
            )
        ),
        t(
            "eval",
            "JS top-level {tab,expr} → valeur JSON (≤64 Ko)",
            obj(
                json!({"tab":{"type":"integer"},"expr":{"type":"string"}}),
                &["tab", "expr"]
            )
        ),
        t("dom", "Dump DOM sérialisé {tab}", tab()),
        t("console", "Erreurs pipeline enregistrées {tab}", tab()),
    ])
}

/// Les deux moitiés du catalogue, concaténées.
fn catalog() -> Value {
    let mut all = tools().as_array().cloned().unwrap_or_default();
    all.extend(act_tools().as_array().cloned().unwrap_or_default());
    Value::Array(all)
}

/// Outil MCP → `(verbe EAP, params)` — la table de
/// correspondance, séparée de l'appel pour la lisibilité
/// (et la borne de taille). `None` = hors table.
fn to_eap(name: &str, args: &Value) -> Option<(&'static str, Value)> {
    let tab = args["tab"].as_u64().unwrap_or(0);
    let elem = args["ref"].as_str().unwrap_or("");
    Some(match name {
        "tab_new" => ("tab.open", json!({})),
        "tab_close" => ("tab.close", json!({"tab": tab})),
        "tabs" => ("tab.list", json!({})),
        "navigate" => (
            "nav.goto",
            json!({"tab": tab, "url": args["url"],
                   "wait": args["wait"], "timeout_ms": args["timeout_ms"]}),
        ),
        "back" => ("nav.back", json!({"tab": tab})),
        "forward" => ("nav.forward", json!({"tab": tab})),
        "reload" => ("nav.reload", json!({"tab": tab})),
        "viewport" => (
            "nav.viewport",
            json!({"tab": tab, "w": args["w"], "h": args["h"]}),
        ),
        "snapshot" => (
            "snapshot.get",
            json!({"tab": tab, "since": args["since"],
                   "interactive_only": args["interactive_only"],
                   "max_nodes": args["max_nodes"]}),
        ),
        "select" => (
            "query.select",
            json!({"tab": tab, "selector": args["selector"], "max": args["max"]}),
        ),
        "click" => (
            "input.click",
            json!({"tab": tab, "ref": elem, "opts": {"timeout_ms": args["timeout_ms"]}}),
        ),
        "click_at" => (
            "input.click_at",
            json!({"tab": tab, "x": args["x"], "y": args["y"]}),
        ),
        "type" => (
            "input.type",
            json!({"tab": tab, "ref": elem, "text": args["text"],
                   "clear": args["clear"].as_bool().unwrap_or(false),
                   "opts": {"timeout_ms": args["timeout_ms"]}}),
        ),
        "hover" => ("input.hover", json!({"tab": tab, "ref": elem})),
        "scroll" => (
            "input.scroll",
            json!({"tab": tab, "ref": args["ref"].as_str(), "dy": args["dy"]}),
        ),
        "key" => ("input.key", json!({"tab": tab, "key": args["key"]})),
        "wait_for" => (
            "wait.for",
            json!({"tab": tab, "kind": args["kind"], "selector": args["selector"],
                   "timeout_ms": args["timeout_ms"].as_u64().unwrap_or(5000)}),
        ),
        "eval" => ("eval.run", json!({"tab": tab, "expr": args["expr"]})),
        "dom" => ("dom.get", json!({"tab": tab})),
        "console" => ("console.get", json!({"tab": tab, "limit": args["limit"]})),
        _ => return None,
    })
}

/// `tools/call` → verbe EAP → `{content:[{type,text}]}` ;
/// l'erreur EAP devient `isError` avec son code typé.
async fn call_tool(eap: &Arc<Eap>, id: Option<&Value>, name: &str, args: &Value) -> Value {
    let resp = |v: Value| match id {
        Some(i) => json!({"jsonrpc":"2.0","id":i,"result":v}),
        None => v,
    };
    if name == "screenshot" {
        return screenshot_tool(
            eap,
            id,
            args["tab"].as_u64().unwrap_or(0),
            args["path"].as_str().unwrap_or("frame.png"),
            args,
        )
        .await;
    }
    let Some((verb, params)) = to_eap(name, args) else {
        return resp(
            json!({"content":[{"type":"text","text":format!("outil inconnu : {name}")}],"isError":true}),
        );
    };
    match eap.call(verb, params).await {
        Ok(r) => resp(json!({"content":[{"type":"text","text":r.to_string()}]})),
        Err(e) => resp(json!({"content":[{"type":"text","text":e}],"isError":true})),
    }
}

/// `screenshot` — `frame.get` rend une trame binaire : le
/// PNG est écrit sur disque, le chemin retourné (l'agent
/// lit le fichier, comme avant en REST).
async fn screenshot_tool(
    eap: &Arc<Eap>,
    id: Option<&Value>,
    tab: u64,
    path: &str,
    args: &Value,
) -> Value {
    let resp = |v: Value| match id {
        Some(i) => json!({"jsonrpc":"2.0","id":i,"result":v}),
        None => v,
    };
    match eap
        .call_binary(
            "frame.get",
            json!({"tab": tab, "full": args["full"].as_bool().unwrap_or(false)}),
        )
        .await
    {
        Ok(png) => match std::fs::write(path, &png) {
            Ok(()) => resp(json!({"content":[{"type":"text",
                "text":format!("frame écrite : {path} ({} octets)", png.len())}]})),
            Err(e) => resp(json!({"content":[{"type":"text",
                "text":format!("écriture {path} : {e}")}],"isError":true})),
        },
        Err(e) => resp(json!({"content":[{"type":"text","text":e}],"isError":true})),
    }
}
