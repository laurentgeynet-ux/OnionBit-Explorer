// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Intégration E1 — socket réel : handshake, dispatch, portée,
//! `unsupported` typé, trame binaire PNG, notifications filtrées.
//! Le transport est simulé (HTML statique) — aucune socket
//! réseau moteur, seule la boucle WS du serveur est vraie.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use explorer_core::Session;
use explorer_layout::font::{FontRequest, FontSource};
use explorer_net::error::NetResult;
use explorer_net::http::{HttpTransport, Request, Response};
use explorer_test_support::fixture_ttf;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio_tungstenite::tungstenite::Message;

struct FixtureFont;
impl FontSource for FixtureFont {
    fn font_data(&self, _req: &FontRequest) -> Option<Vec<u8>> {
        Some(fixture_ttf())
    }
}

/// HTML riche en contrôles — le snapshot doit trouver lien,
/// bouton, champ.
const PAGE: &str = "<!doctype html><html><head><title>t</title></head><body>\
    <a href=\"/next\">go</a><button id=\"b\">ok</button>\
    <input type=\"text\" id=\"f\"></body></html>";

struct FakeTransport;
#[async_trait]
impl HttpTransport for FakeTransport {
    async fn fetch(&self, _req: &Request) -> NetResult<Response> {
        Ok(Response {
            status: 200,
            reason: "fake".to_string(),
            headers: vec![],
            body: PAGE.as_bytes().to_vec(),
            reused_connection: false,
            resolved_url: None,
        })
    }
}

/// Socket client de test — enveloppe mince send/recv JSON.
struct Agent {
    sock: tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    next_id: u64,
    /// Notifications intercalées lues pendant `call` — elles
    /// appartiennent au test qui les attend (`next_event` les
    /// consomme avant le socket).
    stash: Vec<Value>,
}

impl Agent {
    /// Requête → réponse JSON (saute les notifications
    /// intercalées — un wait les consomme explicitement).
    async fn call(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        let req = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        self.sock
            .send(Message::Text(req.to_string().into()))
            .await
            .expect("send");
        loop {
            match self.sock.next().await.expect("stream clos").expect("frame") {
                Message::Text(t) => {
                    let v: Value = serde_json::from_str(&t).expect("json");
                    if v.get("id").and_then(Value::as_u64) == Some(id) {
                        return v;
                    }
                    self.stash.push(v); // notification — gardée pour next_event
                }
                Message::Binary(_) => panic!("trame binaire inattendue"),
                _ => {}
            }
        }
    }

    /// Prochaine notification `event.*` — `None` au-delà du
    /// budget (2 s — une notification attendue ne devrait
    /// jamais approcher la borne).
    async fn next_event(&mut self) -> Option<Value> {
        if let Some(i) = self.stash.iter().position(|v| {
            v.get("method")
                .and_then(Value::as_str)
                .is_some_and(|m| m.starts_with("event."))
        }) {
            return Some(self.stash.remove(i));
        }
        let fut = async {
            loop {
                if let Message::Text(t) = self.sock.next().await?.ok()? {
                    let v: Value = serde_json::from_str(&t).ok()?;
                    if v.get("method")
                        .and_then(Value::as_str)
                        .is_some_and(|m| m.starts_with("event."))
                    {
                        return Some(v);
                    }
                }
            }
        };
        tokio::time::timeout(Duration::from_secs(2), fut)
            .await
            .ok()
            .flatten()
    }

    /// Réponse binaire corrélée à `id` — le test `frame.get`.
    async fn next_binary_for(&mut self, id: u64) -> Vec<u8> {
        let fut = async {
            loop {
                if let Message::Binary(b) = self.sock.next().await?.ok()? {
                    if let Some((rid, _, _)) = explorer_agent::proto::binary::decode(&b) {
                        if u64::from(rid) == id {
                            return Some(b.to_vec());
                        }
                    }
                }
            }
        };
        tokio::time::timeout(Duration::from_secs(2), fut)
            .await
            .ok()
            .flatten()
            .expect("trame binaire attendue")
    }
}

/// Spawn le routeur sur un listener éphémère, connecte un
/// client WS — la boucle de test.
async fn connect() -> (Agent, tokio::task::JoinHandle<()>) {
    let session = Arc::new(tokio::sync::Mutex::new(Session::new(
        Arc::new(FakeTransport),
        Arc::new(FixtureFont),
    )));
    let app = explorer_agent::agent_router(session, None);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    let url = format!("ws://127.0.0.1:{port}/agent");
    let (sock, _) = tokio_tungstenite::connect_async(&url)
        .await
        .expect("ws connect");
    (
        Agent {
            sock,
            next_id: 1,
            stash: Vec::new(),
        },
        server,
    )
}

/// `session.open` par défaut — capacités livrées, profil
/// restrictif.
async fn open(a: &mut Agent) -> Value {
    a.call(
        "session.open",
        json!({"protocol": "eap/1", "capabilities": [
            "snapshot", "wait_commit", "input", "eval", "frames",
            "console_log", "dom", "tabs", "events"
        ]}),
    )
    .await
}

#[tokio::test]
async fn handshake_et_porte_session_open() {
    let (mut a, srv) = connect().await;
    // Verbe métier avant open → -32008.
    let r = a.call("tab.list", json!({})).await;
    assert_eq!(r["error"]["code"], -32008);
    let r = open(&mut a).await;
    assert_eq!(r["result"]["protocol"], "eap/1");
    assert!(r["result"]["capabilities"].as_array().expect("caps").len() >= 8);
    assert_eq!(r["result"]["profile"]["input"], false);
    assert_eq!(r["result"]["profile"]["eval"], false);
    // Après open : tab.list passe.
    let r = a.call("tab.list", json!({})).await;
    assert!(r["result"].is_object());
    srv.abort();
}

#[tokio::test]
async fn profil_restrictif_bloque_input_et_eval() {
    let (mut a, srv) = connect().await;
    open(&mut a).await;
    let r = a.call("tab.open", json!({})).await;
    let tab = r["result"]["tab"].as_u64().expect("tab");
    // input sans droit → -32008.
    let r = a
        .call("input.click_at", json!({"tab": tab, "x": 1.0, "y": 1.0}))
        .await;
    assert_eq!(r["error"]["code"], -32008);
    // eval sans droit → -32008.
    let r = a.call("eval.run", json!({"tab": tab, "expr": "1+1"})).await;
    assert_eq!(r["error"]["code"], -32008);
    srv.abort();
}

#[tokio::test]
async fn unsupported_type_pour_phases_futures() {
    let (mut a, srv) = connect().await;
    open(&mut a).await;
    let r = a.call("tab.open", json!({})).await;
    let tab = r["result"]["tab"].as_u64().expect("tab");
    // input.click : la cap `handles` n'est pas dans la liste
    // négociée par open() → -32001 capability (non négociée,
    // sémantique du contrat — le droit input viendrait après).
    let r = a
        .call("input.click", json!({"tab": tab, "ref": "el:1"}))
        .await;
    assert_eq!(r["error"]["code"], -32001);
    assert_eq!(r["error"]["data"]["capability"], "handles");
    // snapshot{since} → -32001 : la cap `snapshot_diff` n'est
    // pas négociée par open() (le diff E3 existe, il est gate).
    let r = a
        .call("snapshot.get", json!({"tab": tab, "since": 0}))
        .await;
    assert_eq!(r["error"]["code"], -32001);
    // wait.for idle → -32001 (hooks réseau E4).
    let r = a
        .call(
            "wait.for",
            json!({"tab": tab, "kind": "idle", "timeout_ms": 10}),
        )
        .await;
    assert_eq!(r["error"]["code"], -32001);
    srv.abort();
}

#[tokio::test]
async fn erreurs_parse_et_methode() {
    let (mut a, srv) = connect().await;
    // Méthode inconnue → -32601.
    let r = a.call("nav.fly", json!({})).await;
    assert_eq!(r["error"]["code"], -32601);
    // Params invalides sur méthode connue → -32602.
    let r = a.call("nav.goto", json!({"tab": "x"})).await;
    assert_eq!(r["error"]["code"], -32602);
    // JSON illisible → -32700.
    a.sock
        .send(Message::Text("{pas json".into()))
        .await
        .expect("send");
    let v = loop {
        if let Some(Ok(Message::Text(t))) = a.sock.next().await {
            break serde_json::from_str::<Value>(&t).expect("json");
        }
    };
    assert_eq!(v["error"]["code"], -32700);
    srv.abort();
}

#[tokio::test]
async fn nav_snapshot_handles() {
    let (mut a, srv) = connect().await;
    open(&mut a).await;
    let r = a.call("tab.open", json!({})).await;
    let tab = r["result"]["tab"].as_u64().expect("tab");
    let r = a
        .call("nav.goto", json!({"tab": tab, "url": "http://t.local/x"}))
        .await;
    assert!(r["error"].is_null(), "{r}");
    assert_eq!(r["result"]["title"], "t");
    // Snapshot interactif : a, button, input attendus.
    let r = a
        .call(
            "snapshot.get",
            json!({"tab": tab, "interactive_only": true}),
        )
        .await;
    let nodes = r["result"]["nodes"].as_array().expect("nodes");
    let roles: Vec<_> = nodes.iter().map(|n| n["role"].as_str()).collect();
    assert!(roles.contains(&Some("link")), "{roles:?}");
    assert!(roles.contains(&Some("button")), "{roles:?}");
    assert!(roles.contains(&Some("textbox")), "{roles:?}");
    // Chaque nœud porte ref + rect.
    for n in nodes {
        assert!(n["ref"].as_str().is_some_and(|s| s.starts_with("el:")));
        assert!(n["rect"]["w"].as_f64().is_some());
    }
    // since → -32001 (diff = E3).
    let r = a
        .call("snapshot.get", json!({"tab": tab, "since": 0}))
        .await;
    assert_eq!(r["error"]["code"], -32001);
    srv.abort();
}

#[tokio::test]
async fn frame_binaire_correlee() {
    let (mut a, srv) = connect().await;
    open(&mut a).await;
    let r = a.call("tab.open", json!({})).await;
    let tab = r["result"]["tab"].as_u64().expect("tab");
    let r = a
        .call("nav.goto", json!({"tab": tab, "url": "http://t.local/x"}))
        .await;
    assert!(r["error"].is_null(), "{r}");
    // frame.get — réponse binaire [id][1][png].
    let id = a.next_id;
    a.next_id += 1;
    let req = json!({"jsonrpc":"2.0","id":id,"method":"frame.get","params":{"tab":tab}});
    a.sock
        .send(Message::Text(req.to_string().into()))
        .await
        .expect("send");
    let frame = a.next_binary_for(id).await;
    let (rid, kind, payload) = explorer_agent::proto::binary::decode(&frame).expect("décodage");
    assert_eq!(u64::from(rid), id);
    assert_eq!(kind, explorer_agent::proto::binary::Kind::PngFrame);
    assert_eq!(&payload[..4], b"\x89PNG");
    srv.abort();
}

#[tokio::test]
async fn notifications_filtrees() {
    let (mut a, srv) = connect().await;
    open(&mut a).await;
    // Souscription nav+snapshot sur la session entière.
    let r = a
        .call("events.subscribe", json!({"kinds": ["nav", "snapshot"]}))
        .await;
    assert!(r["result"]["subscribed"].is_array());
    let r = a.call("tab.open", json!({})).await;
    let tab = r["result"]["tab"].as_u64().expect("tab");
    // Naviguer → nav.started, nav.committed, snapshot.changed
    // arrivent (les événements antérieurs au call peuvent
    // déjà filer — on lit jusqu'au committed).
    let call = a
        .call("nav.goto", json!({"tab": tab, "url": "http://t.local/x"}))
        .await;
    assert!(call["error"].is_null(), "{call}");
    let mut seen = Vec::new();
    for _ in 0..6 {
        match a.next_event().await {
            Some(v) => {
                seen.push(v["method"].as_str().unwrap_or("").to_string());
                if seen.iter().any(|m| m == "event.snapshot.changed") {
                    break;
                }
            }
            None => break,
        }
    }
    assert!(seen.contains(&"event.nav.started".to_string()), "{seen:?}");
    assert!(
        seen.contains(&"event.nav.committed".to_string()),
        "{seen:?}"
    );
    assert!(
        seen.contains(&"event.snapshot.changed".to_string()),
        "{seen:?}"
    );
    srv.abort();
}

#[tokio::test]
async fn portee_onglets_appliquee() {
    let (mut a, srv) = connect().await;
    // Profil borné à l'onglet 42 — tout verbe sur un autre
    // onglet → -32008.
    let r = a
        .call(
            "session.open",
            json!({"protocol":"eap/1","profile":{"input":true,"eval":true,"tabs":[42]}}),
        )
        .await;
    assert_eq!(r["result"]["profile"]["input"], true);
    // tab.open refusé (portée bornée).
    let r = a.call("tab.open", json!({})).await;
    assert_eq!(r["error"]["code"], -32008);
    // Verbe sur l'onglet hors portée → -32008.
    let r = a.call("snapshot.get", json!({"tab": 1})).await;
    assert_eq!(r["error"]["code"], -32008);
    // Onglet dans la portée mais inexistant → -32005.
    let r = a.call("snapshot.get", json!({"tab": 42})).await;
    assert_eq!(r["error"]["code"], -32005);
    srv.abort();
}

#[tokio::test]
async fn wait_for_frame_borne() {
    let (mut a, srv) = connect().await;
    open(&mut a).await;
    let r = a.call("tab.open", json!({})).await;
    let tab = r["result"]["tab"].as_u64().expect("tab");
    // wait.for frame sans activité → -32004 au timeout.
    let r = a
        .call(
            "wait.for",
            json!({"tab": tab, "kind": "frame", "timeout_ms": 50}),
        )
        .await;
    assert_eq!(r["error"]["code"], -32004);
    srv.abort();
}

#[tokio::test]
async fn e2_select_click_type() {
    let (mut a, srv) = connect().await;
    // Profil input requis pour les actions.
    let r = a
        .call(
            "session.open",
            json!({"protocol":"eap/1","capabilities":["snapshot","handles","actionable","wait_commit","input","frames","tabs","events"],"profile":{"input":true}}),
        )
        .await;
    assert!(r["result"]["capabilities"]
        .as_array()
        .expect("caps")
        .contains(&json!("handles")));
    let r = a.call("tab.open", json!({})).await;
    let tab = r["result"]["tab"].as_u64().expect("tab");
    let r = a
        .call("nav.goto", json!({"tab": tab, "url": "http://t.local/x"}))
        .await;
    assert!(r["error"].is_null(), "{r}");

    // query.select — moteur CSS natif.
    let r = a
        .call("query.select", json!({"tab": tab, "selector": "a, input"}))
        .await;
    let refs = r["result"]["refs"].as_array().expect("refs");
    assert_eq!(refs.len(), 2, "{r}");
    assert!(refs[0].as_str().is_some_and(|s| s.starts_with("el:")));
    // Sélecteur invalide → -32006.
    let r = a
        .call("query.select", json!({"tab": tab, "selector": ">>"}))
        .await;
    assert_eq!(r["error"]["code"], -32006);

    // input.click sur le lien → navigated (auto-wait complet).
    let r = a
        .call("query.select", json!({"tab": tab, "selector": "a"}))
        .await;
    let link = r["result"]["refs"][0].as_str().expect("ref");
    let r = a
        .call(
            "input.click",
            json!({"tab": tab, "ref": link, "opts": {"timeout_ms": 3000}}),
        )
        .await;
    assert_eq!(r["result"]["result"], "navigated", "{r}");
    let act = &r["result"]["actionability"];
    assert_eq!(act["visible"], true);
    assert_eq!(act["topmost"], true);

    // Le click a déclenché une nav — attendre le commit avant
    // de re-sélectionner (le DOM est détaché en vol).
    let r = a
        .call(
            "wait.for",
            json!({"tab": tab, "kind": "commit", "timeout_ms": 5000}),
        )
        .await;
    assert_eq!(r["result"]["ok"], true, "{r}");
    // Re-navigation = même page → nouvelles refs pour l'input.
    let r = a
        .call("query.select", json!({"tab": tab, "selector": "#f"}))
        .await;
    let field = r["result"]["refs"][0].as_str().expect("ref");
    let r = a
        .call(
            "input.type",
            json!({"tab": tab, "ref": field, "text": "onion"}),
        )
        .await;
    assert_eq!(r["result"]["result"], "edited", "{r}");

    // Ref expirée après navigation → -32002.
    let r = a
        .call("nav.goto", json!({"tab": tab, "url": "http://t.local/y"}))
        .await;
    assert!(r["error"].is_null(), "{r}");
    let r = a
        .call("input.click", json!({"tab": tab, "ref": field}))
        .await;
    assert_eq!(r["error"]["code"], -32002);
    srv.abort();
}

#[tokio::test]
async fn e2_wait_selector_stable() {
    let (mut a, srv) = connect().await;
    open(&mut a).await;
    let r = a.call("tab.open", json!({})).await;
    let tab = r["result"]["tab"].as_u64().expect("tab");
    let r = a
        .call("nav.goto", json!({"tab": tab, "url": "http://t.local/x"}))
        .await;
    assert!(r["error"].is_null(), "{r}");
    // selector présent → ok rapide.
    let r = a
        .call(
            "wait.for",
            json!({"tab": tab, "kind": "selector", "selector": "button#b", "timeout_ms": 2000}),
        )
        .await;
    assert_eq!(r["result"]["ok"], true, "{r}");
    // selector absent → -32004.
    let r = a
        .call(
            "wait.for",
            json!({"tab": tab, "kind": "selector", "selector": ".absent", "timeout_ms": 120}),
        )
        .await;
    assert_eq!(r["error"]["code"], -32004);
    // stable → ok (page statique).
    let r = a
        .call(
            "wait.for",
            json!({"tab": tab, "kind": "stable", "timeout_ms": 2000}),
        )
        .await;
    assert_eq!(r["result"]["ok"], true, "{r}");
    srv.abort();
}

#[tokio::test]
async fn e3_snapshot_semantique() {
    let (mut a, srv) = connect().await;
    let r = a
        .call(
            "session.open",
            json!({"protocol":"eap/1","capabilities":["snapshot","snapshot_diff","tabs","events"]}),
        )
        .await;
    assert!(r["result"]["capabilities"]
        .as_array()
        .expect("caps")
        .contains(&json!("snapshot_diff")));
    let r = a.call("tab.open", json!({})).await;
    let tab = r["result"]["tab"].as_u64().expect("tab");
    let r = a
        .call("nav.goto", json!({"tab": tab, "url": "http://t.local/x"}))
        .await;
    assert!(r["error"].is_null(), "{r}");

    let r = a
        .call(
            "snapshot.get",
            json!({"tab": tab, "interactive_only": false}),
        )
        .await;
    let nodes = r["result"]["nodes"].as_array().expect("nodes");
    // Noms accessibles = texte visible, plus id/cls/tag.
    let a_node = nodes.iter().find(|n| n["role"] == "link").expect("un lien");
    assert_eq!(a_node["name"], "go", "{a_node}");
    let b_node = nodes
        .iter()
        .find(|n| n["role"] == "button")
        .expect("un bouton");
    assert_eq!(b_node["name"], "ok", "{b_node}");
    let f_node = nodes
        .iter()
        .find(|n| n["role"] == "textbox")
        .expect("un champ");
    assert_eq!(f_node["name"], "f", "{f_node}");
    // Profondeur réelle — le plat E1 (0 partout) est fini.
    assert!(a_node["depth"].as_u64().expect("depth") > 0);
    // États natifs.
    assert!(a_node["states"]
        .as_array()
        .expect("states")
        .contains(&json!("link")));
    assert!(f_node["states"]
        .as_array()
        .expect("states")
        .contains(&json!("editable")));
    srv.abort();
}

#[tokio::test]
async fn e3_diff_since() {
    let (mut a, srv) = connect().await;
    let r = a
        .call(
            "session.open",
            json!({"protocol":"eap/1","capabilities":["snapshot","snapshot_diff","handles","actionable","input","tabs"],"profile":{"input":true}}),
        )
        .await;
    assert!(r["error"].is_null(), "{r}");
    let r = a.call("tab.open", json!({})).await;
    let tab = r["result"]["tab"].as_u64().expect("tab");
    let r = a
        .call("nav.goto", json!({"tab": tab, "url": "http://t.local/x"}))
        .await;
    assert!(r["error"].is_null(), "{r}");

    // Snapshot de base — la ref du champ est mémorisée.
    let r = a.call("snapshot.get", json!({"tab": tab})).await;
    let base_rev = r["result"]["rev"].as_u64().expect("rev");
    let f_ref = r["result"]["nodes"]
        .as_array()
        .expect("nodes")
        .iter()
        .find(|n| n["role"] == "textbox")
        .and_then(|n| n["ref"].as_str())
        .expect("ref champ")
        .to_string();

    // Diff sans mutation : tout identique → trois vides.
    let r = a
        .call("snapshot.get", json!({"tab": tab, "since": base_rev}))
        .await;
    assert_eq!(r["result"]["base_rev"], json!(base_rev), "{r}");
    assert_eq!(r["result"]["added"], json!([]));
    assert_eq!(r["result"]["removed"], json!([]));
    assert_eq!(r["result"]["changed"], json!([]));

    // Mutation dans le même rev : la frappe écrit `value` et
    // pose le focus → `changed` porte la ref STABLE du champ.
    let r = a
        .call("input.type", json!({"tab": tab, "ref": f_ref, "text": "x"}))
        .await;
    assert_eq!(r["result"]["result"], "edited", "{r}");
    let r = a
        .call("snapshot.get", json!({"tab": tab, "since": base_rev}))
        .await;
    assert_eq!(r["result"]["base_rev"], json!(base_rev), "{r}");
    assert_eq!(r["result"]["added"], json!([]), "{r}");
    assert_eq!(r["result"]["removed"], json!([]), "{r}");
    let changed = r["result"]["changed"].as_array().expect("changed");
    let hit = changed
        .iter()
        .find(|n| n["ref"].as_str() == Some(f_ref.as_str()))
        .unwrap_or_else(|| panic!("champ absent du diff: {r}"));
    // Focus posé par la frappe → état visible dans le diff.
    assert!(
        hit["states"]
            .as_array()
            .expect("states")
            .contains(&json!("focus")),
        "{hit}"
    );

    // `since` jamais servi (ou rev périmée) → dégradation
    // honnête : snapshot complet, pas d'erreur.
    let r = a
        .call("snapshot.get", json!({"tab": tab, "since": 999}))
        .await;
    assert!(r["result"]["nodes"].is_array(), "{r}");
    assert!(r["result"]["base_rev"].is_null(), "{r}");
    srv.abort();
}
