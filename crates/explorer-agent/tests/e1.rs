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

struct FakeTransport(&'static str);
#[async_trait]
impl HttpTransport for FakeTransport {
    async fn fetch(&self, _req: &Request) -> NetResult<Response> {
        Ok(Response {
            status: 200,
            reason: "fake".to_string(),
            headers: vec![],
            body: self.0.as_bytes().to_vec(),
            reused_connection: false,
            resolved_url: None,
            from_cache: false,
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
    let (a, srv, _) = connect_with_session().await;
    (a, srv)
}

/// Variante qui rend aussi la `Session` partagée — les
/// tests d'état moteur (orphelin garé, détachement) en ont
/// besoin pour préparer le terrain hors protocole.
async fn connect_with_session() -> (
    Agent,
    tokio::task::JoinHandle<()>,
    Arc<tokio::sync::Mutex<Session>>,
) {
    connect_with_html(PAGE).await
}

/// Variante à HTML libre — les fixtures anormales ne
/// partagent pas `PAGE` (les tests existants comptent ses
/// éléments).
async fn connect_with_html(
    html: &'static str,
) -> (
    Agent,
    tokio::task::JoinHandle<()>,
    Arc<tokio::sync::Mutex<Session>>,
) {
    let session = Arc::new(tokio::sync::Mutex::new(Session::new(
        Arc::new(FakeTransport(html)),
        Arc::new(FixtureFont),
    )));
    let app = explorer_agent::agent_router(Arc::clone(&session), None);
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
        session,
    )
}

/// `session.open` par défaut — capacités livrées, profil
/// restrictif.
async fn open(a: &mut Agent) -> Value {
    a.call(
        "session.open",
        json!({"protocol": "eap/1", "capabilities": [
            "snapshot", "wait_commit", "input", "eval", "frames",
            "console_log", "dom", "tabs", "events", "engine", "net"
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
    // Bloc dirs (étape 265) — annoncé dès l'open : data_root
    // canonique résolu (contexte workspace), `profile` absent
    // quand l'hôte ne l'injecte pas (routeur de test).
    let dirs = &r["result"]["dirs"];
    assert!(dirs.is_object(), "dirs annoncé");
    assert!(
        dirs["data_root"]
            .as_str()
            .is_some_and(|s| s.ends_with("Explorer-data")),
        "data_root canonique : {dirs}"
    );
    assert!(dirs["profile"].is_null(), "profil non injecté");
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
async fn engine_introspection() {
    let (mut a, srv) = connect().await;
    open(&mut a).await;
    let r = a.call("tab.open", json!({})).await;
    let tab = r["result"]["tab"].as_u64().expect("tab");
    let r = a
        .call("nav.goto", json!({"tab": tab, "url": "http://t.local/x"}))
        .await;
    assert!(r["error"].is_null(), "{r}");
    // engine.pipeline — état + compteurs + mémoire.
    let r = a.call("engine.pipeline", json!({"tab": tab})).await;
    let st = &r["result"]["state"];
    assert_eq!(st["has_dom"], true, "{r}");
    assert_eq!(st["has_frame"], true, "{r}");
    assert_eq!(st["orphan_parked"], false);
    assert!(r["result"]["counters"].is_object());
    assert!(r["result"]["mem"]["dom_nodes"]
        .as_u64()
        .is_some_and(|n| n > 0));
    // engine.layout — dump de l'arbre posé.
    let r = a.call("engine.layout", json!({"tab": tab})).await;
    let dump = r["result"]["dump"].as_str().expect("dump");
    assert!(!dump.is_empty(), "{r}");
    // Étapes 250 — livre de phases : la nav a enregistré des
    // bornes `flow.*`, chaque accumulateur porte
    // n/us_total/us_last/us_max/alloc_b/alloc_n.
    let r = a.call("engine.pipeline", json!({"tab": tab})).await;
    let phases = r["result"]["phases"].as_object().expect("phases");
    assert!(
        phases.contains_key("flow.raster"),
        "flow.raster attendu après une nav : {phases:?}"
    );
    let acc = &phases["flow.raster"];
    for k in ["n", "us_total", "us_last", "us_max", "alloc_b", "alloc_n"] {
        assert!(acc[k].is_u64(), "champ {k} : {acc}");
    }
    let n_before = acc["n"].as_u64().expect("n");
    // reset:true = « take » — la réponse porte l'intervalle
    // clos, puis le livre repart à zéro : un take suivant ne
    // revoit plus le compteur clos (les phases nées entre les
    // deux, s'il y en a, repartent d'un n petit).
    let r = a
        .call("engine.pipeline", json!({"tab": tab, "reset": true}))
        .await;
    // `>=` et non `==` : une raster née entre les deux appels
    // (fonte, image tardive) est incluse dans l'intervalle
    // clos — le take capture tout ce qui a couru jusqu'à lui.
    let take_n = r["result"]["phases"]["flow.raster"]["n"]
        .as_u64()
        .expect("n du take");
    assert!(take_n >= n_before, "le take rend l'intervalle clos : {r}");
    let r = a
        .call("engine.pipeline", json!({"tab": tab, "reset": true}))
        .await;
    let n_after = r["result"]["phases"]["flow.raster"]["n"]
        .as_u64()
        .unwrap_or(0);
    assert!(n_after < n_before, "livre reparti après take : {r}");
    // Étape 249 — compteurs du dernier paint commis : `emit`
    // (boîte→item) et `raster` (replay) non nuls après nav,
    // `frame` décrit la liste peinte.
    let r = a.call("engine.paint_stats", json!({"tab": tab})).await;
    assert!(r["error"].is_null(), "{r}");
    let emit = &r["result"]["emit"];
    assert!(
        emit["items"].as_u64().is_some_and(|n| n > 0),
        "items émis attendus : {r}"
    );
    assert!(emit["by_kind"].is_object(), "by_kind : {r}");
    let raster = &r["result"]["raster"];
    assert!(
        raster["draws"].as_u64().is_some_and(|n| n > 0),
        "draws raster attendus : {r}"
    );
    assert!(raster["kind_us"].is_object() && raster["kind_n"].is_object());
    assert!(r["result"]["frame"]["list_items"].is_u64(), "{r}");
    let r = a.call("engine.paint_stats", json!({"tab": 999})).await;
    assert_eq!(r["error"]["code"], -32005);
    // net.events (étape 251) : la navigation commise a
    // produit au moins la complétion `document` — ring
    // monotone, curseur `since_seq` incrémental.
    let r = a.call("net.events", json!({"tab": tab})).await;
    assert!(r["error"].is_null(), "{r}");
    let events = r["result"]["events"].as_array().expect("events");
    assert!(
        events
            .iter()
            .any(|e| e["kind"] == "document" && e["status"] == 200),
        "complétion document attendue : {r}"
    );
    let next_seq = r["result"]["next_seq"].as_u64().expect("next_seq");
    assert!(next_seq >= events.len() as u64, "{r}");
    assert!(
        events
            .iter()
            .all(|e| e["seq"].as_u64().is_some() && e["url"].is_string()),
        "seq/url structurés : {r}"
    );
    let r = a
        .call("net.events", json!({"tab": tab, "since_seq": next_seq}))
        .await;
    assert!(r["error"].is_null(), "{r}");
    assert_eq!(r["result"]["events"].as_array().unwrap().len(), 0, "{r}");
    let r = a.call("net.events", json!({"tab": 999})).await;
    assert_eq!(r["error"]["code"], -32005);
    // frame.diff (étape 252) : le paint commis porte un
    // rapport de dommage — `damage.full` booléen ou rects
    // px CSS ; `windowed` reflète le chemin de raster.
    let r = a.call("frame.diff", json!({"tab": tab})).await;
    assert!(r["error"].is_null(), "{r}");
    assert!(r["result"]["windowed"].is_boolean(), "{r}");
    assert!(r["result"]["damage"]["full"].is_boolean(), "{r}");
    assert!(
        r["result"]["items_cur"].as_u64().is_some_and(|n| n > 0),
        "{r}"
    );
    let r = a.call("frame.diff", json!({"tab": 999})).await;
    assert_eq!(r["error"]["code"], -32005);
    // Onglet inconnu → -32005.
    let r = a.call("engine.pipeline", json!({"tab": 999})).await;
    assert_eq!(r["error"]["code"], -32005);
    srv.abort();
}

/// `engine.cascade` — verdicts par déclaration × longhand
/// sur l'état committé (étape 253) : une ref fraîche de
/// snapshot sert au moins un verdict posé ; ref pourrie →
/// `StaleRef`.
#[tokio::test]
async fn engine_cascade_verdicts() {
    let (mut a, srv) = connect().await;
    // `handles` exigée par `describe` ; le profil `eval`
    // autorise `eval.run{ref}`.
    a.call(
        "session.open",
        json!({"protocol": "eap/1", "capabilities": [
            "snapshot", "wait_commit", "input", "eval", "frames",
            "console_log", "dom", "tabs", "events", "engine", "net",
            "handles"
        ], "profile":{"eval": true}}),
    )
    .await;
    let r = a.call("tab.open", json!({})).await;
    let tab = r["result"]["tab"].as_u64().expect("tab");
    let r = a
        .call("nav.goto", json!({"tab": tab, "url": "http://t.local/x"}))
        .await;
    assert!(r["error"].is_null(), "{r}");
    let r = a.call("snapshot.get", json!({"tab": tab})).await;
    let elem_ref = r["result"]["nodes"]
        .as_array()
        .and_then(|n| n.first())
        .and_then(|n| n["ref"].as_str())
        .expect("une ref dans le snapshot");
    let r = a
        .call("engine.cascade", json!({"tab": tab, "ref": elem_ref}))
        .await;
    assert!(r["error"].is_null(), "{r}");
    let decls = r["result"]["declarations"]
        .as_array()
        .expect("declarations");
    assert!(!decls.is_empty(), "{r}");
    assert!(
        decls.iter().all(
            |d| d["prop"].is_string() && (d["verdict"].is_string() || d["verdict"].is_object())
        ),
        "verdict structuré : {r}"
    );
    assert!(
        decls.iter().any(|d| d["verdict"] == "winner"
            || d["verdict"] == "inherited"
            || d["verdict"] == "initial"),
        "un verdict posé attendu : {r}"
    );
    // `declared_only` filtre le bruit — un verdict posé
    // reste, le tapis de `initial`/`inherited` part.
    let r = a
        .call(
            "engine.cascade",
            json!({"tab": tab, "ref": elem_ref, "declared_only": true}),
        )
        .await;
    assert!(r["error"].is_null(), "{r}");
    let decls2 = r["result"]["declarations"]
        .as_array()
        .expect("declarations");
    assert!(
        decls2
            .iter()
            .all(|d| d["verdict"] != "initial" && d["verdict"] != "inherited"),
        "declared_only : {r}"
    );
    let r = a
        .call("engine.cascade", json!({"tab": tab, "ref": "el:999999"}))
        .await;
    assert_eq!(r["error"]["code"], -32002, "{r}");
    let r = a
        .call("engine.cascade", json!({"tab": 999, "ref": elem_ref}))
        .await;
    assert_eq!(r["error"]["code"], -32005);
    // `describe` — même ref : tag/attrs/rect/texte en un
    // appel, ref pourrie → -32002 (étape 258-bis).
    let r = a
        .call("describe", json!({"tab": tab, "ref": elem_ref}))
        .await;
    assert!(r["error"].is_null(), "{r}");
    assert!(
        r["result"]["tag"].is_string() && r["result"]["node"]["index"].is_u64(),
        "{r}"
    );
    let r = a
        .call("describe", json!({"tab": tab, "ref": "el:999999"}))
        .await;
    assert_eq!(r["error"]["code"], -32002, "{r}");
    // `eval{ref}` — la couture passe le chemin DOM ;
    // verdict honnête : erreur de binding (moteur sans
    // implémenter `eval_element`) ou -32007 si la fixture
    // n'arme pas de moteur JS — jamais de faux succès.
    let r = a
        .call(
            "eval.run",
            json!({"tab": tab, "expr": "el => el.tagName", "ref": elem_ref}),
        )
        .await;
    let honest = r["result"]["error"].is_string()
        || r["result"]["value"].is_string()
        || r["error"]["code"] == -32007;
    assert!(honest, "verdict honnête, pas de faux succès : {r}");
    srv.abort();
}

/// Page avec un interactif caché — `display:none` laisse
/// l'élément dans le DOM mais hors de l'arbre de boîtes.
const ANOMALY_PAGE: &str = r#"<!doctype html><html><body>
    <div style="display:none"><a href="/g">ghost</a>
    <input type="hidden" name="h"></div>
    <a href="/v">visible</a></body></html>"#;

/// Formulaire minimal — `input.submit`/`input.select_option`
/// (étape 259).
const FORM_PAGE: &str = r#"<!doctype html><html><body>
    <a href="/x">hors form</a>
    <form><input type="text"><select><option value="a">AA</option><option value="b">BB</option></select><button>envoyer</button></form>
    </body></html>"#;

#[tokio::test]
async fn c2_submit_select_option() {
    let (mut a, srv, _session) = connect_with_html(FORM_PAGE).await;
    // input + handles requis par les actions ref.
    a.call(
        "session.open",
        json!({"protocol":"eap/1","capabilities":["snapshot","handles","actionable","wait_commit","input","tabs","events"],"profile":{"input":true}}),
    )
    .await;
    let r = a.call("tab.open", json!({})).await;
    let tab = r["result"]["tab"].as_u64().expect("tab");
    let r = a
        .call("nav.goto", json!({"tab": tab, "url": "http://t.local/x"}))
        .await;
    assert!(r["error"].is_null(), "{r}");
    let r = a
        .call("query.select", json!({"tab": tab, "selector": "select"}))
        .await;
    let sel_ref = r["result"]["refs"]
        .as_array()
        .and_then(|a| a.first())
        .and_then(|r| r.as_str())
        .map(str::to_owned)
        .expect("ref select");
    let r = a
        .call("query.select", json!({"tab": tab, "selector": "a"}))
        .await;
    let link_ref = r["result"]["refs"]
        .as_array()
        .and_then(|a| a.first())
        .and_then(|r| r.as_str())
        .map(str::to_owned)
        .expect("ref a");
    // value → choix honnête ; sans value/index → -32602 ;
    // mauvaise value → -32003.
    let r = a
        .call(
            "input.select_option",
            json!({"tab": tab, "ref": sel_ref, "value": "b"}),
        )
        .await;
    assert!(r["error"].is_null(), "{r}");
    let r = a
        .call("input.select_option", json!({"tab": tab, "ref": sel_ref}))
        .await;
    assert_eq!(r["error"]["code"], -32602, "{r}");
    let r = a
        .call(
            "input.select_option",
            json!({"tab": tab, "ref": sel_ref, "value": "zzz"}),
        )
        .await;
    assert_eq!(r["error"]["code"], -32003, "{r}");
    // submit sur le <select> (descendant du form) → séquence
    // complète ; sur le <a> hors form → -32003.
    let r = a
        .call("input.submit", json!({"tab": tab, "ref": sel_ref}))
        .await;
    assert!(r["error"].is_null(), "{r}");
    let r = a
        .call("input.submit", json!({"tab": tab, "ref": link_ref}))
        .await;
    assert_eq!(r["error"]["code"], -32003, "{r}");
    srv.abort();
}

#[tokio::test]
async fn engine_anomalies_flague_interactif_sans_boite() {
    let (mut a, srv, _session) = connect_with_html(ANOMALY_PAGE).await;
    let r = open(&mut a).await;
    // L'index compact est servi au handshake.
    let help = r["result"]["help"].as_object().expect("help");
    assert_eq!(
        help.get("suspects d'affichage / santé ?")
            .and_then(Value::as_str),
        Some("engine.anomalies"),
        "{help:?}"
    );
    let r = a.call("tab.open", json!({})).await;
    let tab = r["result"]["tab"].as_u64().expect("tab");
    let r = a
        .call("nav.goto", json!({"tab": tab, "url": "http://t.local/x"}))
        .await;
    assert!(r["error"].is_null(), "{r}");
    let r = a.call("engine.anomalies", json!({"tab": tab})).await;
    let suspects = r["result"]["suspects"].as_array().expect("suspects");
    // Le lien ghost display:none est flagué no_box, avec ref.
    let ghost = suspects
        .iter()
        .find(|s| s["kind"] == "interactive_no_box")
        .expect("suspect no_box attendu");
    assert_eq!(ghost["tag"], "a");
    assert!(ghost["ref"].as_str().is_some_and(|s| s.starts_with("el:")));
    // La cause nomme le bloqueur — `display:none` sur
    // l'ancêtre <div> (étape 263 : distinguer « caché par
    // design » de « anomalie de boxification »).
    assert_eq!(ghost["cause"]["cause"], "display:none", "{ghost}");
    assert_eq!(ghost["cause"]["on"], "ancestor", "{ghost}");
    assert_eq!(ghost["cause"]["tag"], "div", "{ghost}");
    // L'input hidden n'est PAS flagué (non interactif).
    assert!(!suspects.iter().any(|s| s["tag"] == "input"));
    // La page visible reste un témoin sain.
    assert!(r["result"]["scanned"]["nodes"]
        .as_u64()
        .is_some_and(|n| n > 0));
    srv.abort();
}

#[tokio::test]
async fn not_actionable_nomme_checks_echus() {
    let (mut a, srv, _session) = connect_with_html(ANOMALY_PAGE).await;
    a.call(
        "session.open",
        json!({"protocol":"eap/1","capabilities":["snapshot","handles","actionable","input","tabs"],"profile":{"input":true}}),
    )
    .await;
    let r = a.call("tab.open", json!({})).await;
    let tab = r["result"]["tab"].as_u64().expect("tab");
    let r = a
        .call("nav.goto", json!({"tab": tab, "url": "http://t.local/x"}))
        .await;
    assert!(r["error"].is_null(), "{r}");
    let r = a
        .call(
            "query.select",
            json!({"tab": tab, "selector": "a[href='/g']"}),
        )
        .await;
    let ghost = r["result"]["refs"]
        .as_array()
        .and_then(|a| a.first())
        .and_then(|r| r.as_str())
        .map(str::to_owned)
        .expect("ref ghost");
    // display:none → `visible` jamais satisfait : le -32003
    // nomme les checks échus dans `data.failed` (étape 263 —
    // l'agent n'a plus à rejouer chaque check isolément).
    let r = a
        .call(
            "input.click",
            json!({"tab": tab, "ref": ghost, "opts": {"timeout_ms": 120}}),
        )
        .await;
    assert_eq!(r["error"]["code"], -32003, "{r}");
    let failed = r["error"]["data"]["failed"].as_array().expect("failed");
    assert!(failed.contains(&json!("visible")), "{r}");
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
    // network_idle sur onglet sans trafic → calme dès
    // quiet_ms atteint (étape 258).
    let r = a
        .call(
            "wait.for",
            json!({"tab": tab, "kind": "network_idle", "quiet_ms": 100, "timeout_ms": 2000}),
        )
        .await;
    assert_eq!(r["result"]["ok"], true);
    srv.abort();
}

/// Étape 267 — `session.ping` est servi **hors file** pendant
/// qu'un verbe métier est en vol : la réponse nomme le verbe
/// (`in_flight.method`) et `busy` est vrai — l'agent distingue
/// « vivant mais occupé » de « mort » (cas airfrance : EAP
/// entier muet derrière un verbe wedged).
#[tokio::test]
async fn ping_hors_verrou_pendant_verbe_en_vol() {
    let (mut a, srv) = connect().await;
    open(&mut a).await;
    let r = a.call("tab.open", json!({})).await;
    let tab = r["result"]["tab"].as_u64().expect("tab");
    // Verbe long envoyé brut — `call` attendrait sa fin et le
    // ping n'aurait rien à mesurer. `wait.for frame` sans
    // activité tient ~1500 ms.
    a.sock
        .send(Message::Text(
            json!({
                "jsonrpc": "2.0", "id": 90, "method": "wait.for",
                "params": {"tab": tab, "kind": "frame", "timeout_ms": 1500}
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send wait");
    // Le dispatcher a le temps de prendre le verbe — le ping
    // doit alors voir `in_flight` nommé.
    tokio::time::sleep(Duration::from_millis(100)).await;
    let ping = a.call("session.ping", json!({})).await;
    assert_eq!(ping["result"]["busy"], true, "ping pendant wait : {ping}");
    assert_eq!(
        ping["result"]["in_flight"]["method"], "wait.for",
        "le verbe en vol est nommé : {ping}"
    );
    // Le wait finit sa course — sa réponse arrive après le
    // ping (ordre des réponses, pas des requêtes).
    let fut = async {
        loop {
            if let Some(Ok(Message::Text(t))) = a.sock.next().await {
                let v: Value = serde_json::from_str(&t).expect("json");
                if v.get("id").and_then(Value::as_u64) == Some(90) {
                    return v;
                }
            }
        }
    };
    let wait = tokio::time::timeout(Duration::from_secs(5), fut)
        .await
        .expect("réponse wait.for");
    assert_eq!(wait["error"]["code"], -32004, "wait borne : {wait}");
    // Session libérée → ping rend busy:false.
    let ping = a.call("session.ping", json!({})).await;
    assert_eq!(ping["result"]["busy"], false, "{ping}");
    assert!(ping["result"]["in_flight"].is_null(), "{ping}");
    srv.abort();
}

/// Étape 268 — `session.cancel` arme l'annulation du verbe en
/// vol **hors file** : le `wait.for` wedged est abandonné et
/// répond `-32009`, pendant que le cancel répond
/// `{cancelled:true, method}` immédiatement — le triptyque
/// est complet : détecter (`busy`) → nommer (`in_flight`) →
/// libérer (cancel).
#[tokio::test]
async fn session_cancel_libere_verbe_en_vol() {
    let (mut a, srv) = connect().await;
    open(&mut a).await;
    let r = a.call("tab.open", json!({})).await;
    let tab = r["result"]["tab"].as_u64().expect("tab");
    // Cancel sans verbe en vol → réponse honnête.
    let r = a.call("session.cancel", json!({})).await;
    assert_eq!(r["result"]["cancelled"], false, "{r}");
    // Verbe long en vol (wait.for frame ~30 s sans activité).
    a.sock
        .send(Message::Text(
            json!({
                "jsonrpc": "2.0", "id": 91, "method": "wait.for",
                "params": {"tab": tab, "kind": "frame", "timeout_ms": 30000}
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send wait");
    tokio::time::sleep(Duration::from_millis(100)).await;
    // Le cancel répond en ligne — jamais derrière le verbe.
    let r = a.call("session.cancel", json!({})).await;
    assert_eq!(r["result"]["cancelled"], true, "{r}");
    assert_eq!(r["result"]["method"], "wait.for", "{r}");
    // Le verbe abandonné rend -32009 (pas -32004 : ce n'est
    // pas un timeout, c'est une annulation demandée).
    let fut = async {
        loop {
            if let Some(Ok(Message::Text(t))) = a.sock.next().await {
                let v: Value = serde_json::from_str(&t).expect("json");
                if v.get("id").and_then(Value::as_u64) == Some(91) {
                    return v;
                }
            }
        }
    };
    let wait = tokio::time::timeout(Duration::from_secs(5), fut)
        .await
        .expect("réponse wait.for annulé");
    assert_eq!(wait["error"]["code"], -32009, "wait annulé : {wait}");
    // Session libérée — ping rend busy:false.
    let ping = a.call("session.ping", json!({})).await;
    assert_eq!(ping["result"]["busy"], false, "{ping}");
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
async fn session_close_repond_et_clot() {
    let (mut a, srv) = connect().await;
    open(&mut a).await;
    // La réponse à session.close doit être livrée — le
    // socket se ferme APRÈS le flush (verrou du bug :
    // writer.abort() perdait la réponse en file).
    let r = a.call("session.close", json!({})).await;
    assert_eq!(r["result"], json!({}), "{r}");
    // Le socket se ferme proprement juste après.
    let closed = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            match a.sock.next().await {
                Some(Ok(Message::Close(_)) | Err(_)) | None => return true,
                Some(Ok(_)) => {}
            }
        }
    })
    .await
    .unwrap_or(false);
    assert!(closed, "le socket n'a pas fermé après session.close");
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
    // Étape 257 — la base demandée mais absente est
    // signalée : `stale_since` évite la désync invisible.
    assert_eq!(r["result"]["stale_since"], json!(true), "{r}");

    // Étape 254 — `snap_seq` nomme l'émission exacte : la
    // réponse la porte toujours, `since_seq` diff dessus
    // sans ambiguïté de rev (deux émissions à même rev,
    // params différents).
    let r = a.call("snapshot.get", json!({"tab": tab})).await;
    let seq_a = r["result"]["snap_seq"].as_u64().expect("snap_seq");
    // Émission suivante à params différents, même rev —
    // `since` seul viserait la plus récente ; `since_seq`
    // vise `seq_a` exactement.
    let r = a
        .call(
            "snapshot.get",
            json!({"tab": tab, "interactive_only": false, "since_seq": seq_a}),
        )
        .await;
    assert_eq!(r["result"]["base_seq"], json!(seq_a), "{r}");
    assert!(r["result"]["snap_seq"].as_u64().expect("seq") > seq_a);
    // Historique borné à 8 : après 9+ émissions l'entrée
    // `seq_a` est évincée → dégradation complète honnête.
    for _ in 0..9 {
        a.call("snapshot.get", json!({"tab": tab})).await;
    }
    let r = a
        .call("snapshot.get", json!({"tab": tab, "since_seq": seq_a}))
        .await;
    assert!(r["result"]["nodes"].is_array(), "{r}");
    assert!(r["result"]["base_seq"].is_null(), "{r}");
    srv.abort();
}

#[tokio::test]
async fn wait_frame_reclame_orphelin_gare() {
    // Régression `navigate{wait:frame}` → -32004 intermittent :
    // une payload orpheline garée (détenteur de rendu mort en
    // vol — issue #23) laisse `dom`/`frame` hors de l'onglet ;
    // la sonde nue lisait `None` à vie. Le wait doit armer le
    // pull (`ensure_frame` : reclaim + peinture paresseuse).
    let (mut a, srv, session) = connect_with_session().await;
    open(&mut a).await;
    let r = a.call("tab.open", json!({})).await;
    let tab = r["result"]["tab"].as_u64().expect("tab");
    let r = a
        .call("nav.goto", json!({"tab": tab, "url": "http://t.local/x"}))
        .await;
    assert!(r["error"].is_null(), "{r}");

    // Simule la mort d'un détenteur : payload parquée,
    // `dom`/`frame` absents de l'onglet.
    session.lock().await.debug_drop_detached_paint(tab);
    let r = a
        .call(
            "wait.for",
            json!({"tab": tab, "kind": "frame", "timeout_ms": 2000}),
        )
        .await;
    assert_eq!(r["result"]["ok"], true, "{r}");

    // Même garde côté selector : l'orphelin re-garé rendait
    // `EmptyDocument` jusqu'au timeout.
    session.lock().await.debug_drop_detached_paint(tab);
    let r = a
        .call(
            "wait.for",
            json!({"tab": tab, "kind": "selector", "selector": "button#b", "timeout_ms": 2000}),
        )
        .await;
    assert_eq!(r["result"]["ok"], true, "{r}");
    srv.abort();
}

/// `engine.j17` (étape 255) — le verbe répond `{armed, snapshot}`
/// honnêtement : `armed` reflète `EXPLORER_J17_PROF`/enable,
/// `snapshot` est une chaîne (vide si désarmé, jamais d'erreur).
/// Onglet inconnu → -32005 comme les autres verbes engine.
#[tokio::test]
async fn engine_j17_rend_le_snapshot_arme_ou_non() {
    let (mut a, srv) = connect().await;
    let r = open(&mut a).await;
    let help = r["result"]["help"].as_object().expect("help");
    assert_eq!(
        help.get("compteurs jscore (prop/env/eval…) ?")
            .and_then(Value::as_str),
        Some("engine.j17"),
        "{help:?}"
    );
    let r = a.call("tab.open", json!({})).await;
    let tab = r["result"]["tab"].as_u64().expect("tab");
    let r = a.call("engine.j17", json!({"tab": tab})).await;
    assert!(r["error"].is_null(), "{r}");
    assert!(r["result"]["armed"].is_boolean(), "{r}");
    assert!(r["result"]["snapshot"].is_string(), "{r}");
    let r = a.call("engine.j17", json!({"tab": 999})).await;
    assert_eq!(r["error"]["code"], -32005, "{r}");
    srv.abort();
}
