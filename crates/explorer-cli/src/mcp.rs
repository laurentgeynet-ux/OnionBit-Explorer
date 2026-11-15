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
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
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
/// `port` = daemon épinglé (défaut de routage) ; `None` =
/// broker pur : la cible se résout par le registre à chaque
/// appel (`daemon`/`nom:onglet`, étape 256).
pub fn serve(port: Option<u16>) -> std::io::Result<u8> {
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

/// Une connexion EAP vivante : un socket, une file
/// d'émission, une table `oneshot` par requête en vol, des
/// tâches lectrice/émettrice. `alive` passe à `false` dès
/// qu'une tâche sort (daemon mort, socket coupée) — le
/// prochain appel reconnecte au lieu de pendre.
struct Live {
    out: mpsc::UnboundedSender<Message>,
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Reply>>>>,
    next: AtomicU64,
    /// Partagé avec les tâches lectrice/émettrice — `false`
    /// dès que l'une sort (socket morte, daemon relancé).
    alive: Arc<AtomicBool>,
}

/// Erreur d'un appel — `dead` = socket prouvée morte
/// (réessayable après reconnexion) ; un timeout n'est **pas**
/// `dead` (un verbe lent n'autorise pas à le rejouer — il
/// aurait pu agir).
struct ReqErr {
    msg: String,
    dead: bool,
}

impl Live {
    /// Envoie une requête et attend sa réponse — JSON ou
    /// binaire selon ce qui arrive pour l'`id`.
    async fn request(&self, method: &str, params: Value) -> Result<Reply, ReqErr> {
        if !self.alive.load(Ordering::Relaxed) {
            return Err(ReqErr {
                msg: "socket agent morte".to_string(),
                dead: true,
            });
        }
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
            self.pending
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .remove(&id);
            return Err(ReqErr {
                msg: "socket agent fermé".to_string(),
                dead: true,
            });
        }
        match tokio::time::timeout(CALL_TIMEOUT, rx).await {
            Ok(Ok(r)) => Ok(r),
            Ok(Err(_)) => Err(ReqErr {
                msg: "lecteur socket mort".to_string(),
                dead: true,
            }),
            Err(_) => Err(ReqErr {
                msg: format!("{method} — timeout adaptateur"),
                dead: false,
            }),
        }
    }
}

/// Client EAP minimal : paresseusement connecté, reconnecte
/// à la demande quand le socket est prouvé mort (daemon
/// relancé — les onglets et leurs ids survivent côté
/// daemon ; les `el:N` sont perdus, `stale_ref` honnête).
struct Eap {
    port: u16,
    live: tokio::sync::Mutex<Option<Arc<Live>>>,
}

impl Eap {
    /// Connecte le socket (`eap/1` sous-protocole requis par
    /// le serveur) puis `session.open` : toutes les capacités
    /// v1, profil input+eval — l'adaptateur pilote le
    /// navigateur pour un agent, pas pour observer.
    async fn connect(port: u16) -> Result<Arc<Live>, String> {
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
        let alive = Arc::new(AtomicBool::new(true));
        // Writer — un seul émetteur sur le sink.
        {
            let alive = Arc::clone(&alive);
            tokio::spawn(async move {
                while let Some(m) = out_rx.recv().await {
                    if sink.send(m).await.is_err() {
                        break;
                    }
                }
                alive.store(false, Ordering::Relaxed);
            });
        }
        // Reader — route texte (id) et binaire (request_id)
        // vers la oneshot de l'appelant ; les notifications
        // `event.*` sont ignorées (l'adaptateur n'y souscrit
        // pas — tools/call est du request/response). À la
        // sortie : vide les requêtes en vol (elles échouent
        // vite au lieu de pendre) et marque la socket morte.
        {
            let pend = Arc::clone(&pending);
            let alive = Arc::clone(&alive);
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
                            if let Some((id, _, payload)) =
                                explorer_agent::proto::binary::decode(&b)
                            {
                                let tx = pend
                                    .lock()
                                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                                    .remove(&u64::from(id));
                                if let Some(tx) = tx {
                                    let _ = tx.send(Reply::Binary(payload.to_vec()));
                                }
                            }
                        }
                        Ok(Message::Close(_)) | Err(_) => break,
                        Ok(_) => {}
                    }
                }
                pend.lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .clear();
                alive.store(false, Ordering::Relaxed);
            });
        }
        let live = Arc::new(Live {
            out,
            pending,
            next: AtomicU64::new(1),
            alive,
        });
        // `session.open` avant de servir — profil pilote.
        let hello = json!({
            "protocol": explorer_agent::PROTOCOL,
            "profile": {"input": true, "eval": true},
        });
        match live.request("session.open", hello).await {
            Ok(Reply::Json(v)) if v.get("error").is_none() => Ok(live),
            Ok(Reply::Json(v)) => Err(format!("session.open refusé : {v}")),
            Ok(Reply::Binary(_)) => Err("trame binaire inattendue à open".to_string()),
            Err(e) => Err(format!("session.open refusé : {}", e.msg)),
        }
    }

    /// La connexion vivante, établie si besoin — le verrou
    /// sérialise les reconnections (un seul handshake par
    /// renaissance, les appels concurrents partagent).
    async fn ensure(&self) -> Result<Arc<Live>, String> {
        let mut g = self.live.lock().await;
        if let Some(l) = &*g {
            if l.alive.load(Ordering::Relaxed) {
                return Ok(Arc::clone(l));
            }
        }
        let l = Self::connect(self.port).await?;
        *g = Some(Arc::clone(&l));
        Ok(l)
    }

    /// Envoie une requête — une reconnexion + un rejeu si la
    /// socket était morte (le verbe n'a jamais atteint le
    /// daemon, rejouer est sûr) ; jamais de rejeu sur timeout.
    async fn request(&self, method: &str, params: Value) -> Result<Reply, String> {
        let mut last = "socket agent morte".to_string();
        for _ in 0..2 {
            let live = self.ensure().await?;
            match live.request(method, params.clone()).await {
                Ok(r) => return Ok(r),
                Err(e) if e.dead => {
                    live.alive.store(false, Ordering::Relaxed);
                    last = e.msg;
                }
                Err(e) => return Err(e.msg),
            }
        }
        Err(last)
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

/// Courtier multi-daemons : une connexion `Eap` paresseuse
/// par port découvert, + le port épinglé éventuel servant de
/// cible par défaut (étape 256). Le multiplexage vit ici —
/// le protocole EAP côté daemon est inchangé : l'adaptateur
/// déroute le préfixe `nom:` de `tab` et réécrit le param.
struct Pool {
    /// Connexions par port — créées au premier appel routé.
    conns: tokio::sync::Mutex<HashMap<u16, Arc<Eap>>>,
    /// Port épinglé au lancement (`mcp <port>`) — cible par
    /// défaut quand l'appel ne nomme pas son daemon.
    default_port: Option<u16>,
}

impl Pool {
    /// Connexion (paresseuse) vers le daemon sur `port` —
    /// partagée entre appels : un `session.open` par daemon.
    async fn conn(&self, port: u16) -> Arc<Eap> {
        let mut g = self.conns.lock().await;
        if let Some(e) = g.get(&port) {
            return Arc::clone(e);
        }
        let e = Arc::new(Eap {
            port,
            live: tokio::sync::Mutex::new(None),
        });
        g.insert(port, Arc::clone(&e));
        e
    }
}

/// Sonde de vivacité loopback : `GET /api/status` minimal
/// sur TCP brut — prouve qu'un *daemon Explorer* écoute
/// (une connexion TCP seule ne distingue pas un port recyclé
/// par un service tiers). ~150 ms de borne, jamais de panic.
async fn probe_daemon(port: u16) -> bool {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let fut = async {
        let mut s = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .ok()?;
        s.write_all(b"GET /api/status HTTP/1.0\r\nHost: 127.0.0.1\r\n\r\n")
            .await
            .ok()?;
        let mut buf = vec![0u8; 512];
        let n = s.read(&mut buf).await.ok()?;
        let head = String::from_utf8_lossy(&buf[..n]);
        // 200 + JSON de statut = notre daemon, pas un port recyclé.
        Some(head.contains(" 200") || head.contains("\"status\""))
    };
    tokio::time::timeout(Duration::from_millis(400), fut)
        .await
        .ok()
        .flatten()
        .unwrap_or(false)
}

/// Annonces du registre **vivantes** — la mort se prouve
/// par PID disparu, jamais par sonde : un daemon saturé qui
/// timeout sur `/api/status` reste vivant et routable
/// (correctif Étape 256 — l'éviction sur sonde supprimait
/// définitivement l'annonce d'un processus pourtant sain).
fn live_announces() -> Vec<explorer_core::daemon_registry::Announce> {
    let mut out = Vec::new();
    for (file, a) in explorer_core::daemon_registry::list() {
        if explorer_core::daemon_registry::announce_stale(&a) {
            explorer_core::daemon_registry::remove(&file);
        } else {
            out.push(a);
        }
    }
    out
}

/// Sélecteur de daemon (`daemon` param ou préfixe `nom:`) →
/// port. Accepte un nom d'annonce ou un port numérique ;
/// l'ambiguïté (homonymes) est une erreur honnête listant
/// les candidats.
fn select_daemon(sel: &str) -> Result<u16, String> {
    if let Ok(p) = sel.parse::<u16>() {
        return Ok(p);
    }
    let matches: Vec<_> = live_announces()
        .into_iter()
        .filter(|a| a.name == sel)
        .collect();
    match matches.len() {
        0 => Err(format!(
            "daemon inconnu : {sel} (daemon_list pour les vivants)"
        )),
        1 => Ok(matches[0].port),
        _ => Err(format!(
            "nom ambigu '{sel}' : {} daemons — vise le port ({})",
            matches.len(),
            matches
                .iter()
                .map(|a| a.port.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

/// Cible par défaut : port épinglé, sinon le daemon unique
/// vivant ; l'absence ou la pluralité est une erreur
/// guidante, jamais un choix silencieux.
fn default_daemon(pool: &Pool) -> Result<u16, String> {
    if let Some(p) = pool.default_port {
        return Ok(p);
    }
    let live = live_announces();
    match live.len() {
        0 => Err("aucun daemon annoncé — daemon_spawn{profile} en démarre un".to_string()),
        1 => Ok(live[0].port),
        _ => Err(format!(
            "plusieurs daemons vivants — précise `daemon` ou `nom:onglet` ({})",
            live.iter()
                .map(|a| format!("{}:{}", a.name, a.port))
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

/// Résout le routage d'un appel → `(connexion, args réécrits,
/// nom de daemon)`. Trois formes de ciblage, par priorité :
/// `daemon` explicite, préfixe `nom:` dans `tab`, défaut.
/// `tab` repart toujours **entier** vers le daemon — EAP
/// reste inchangé, le préfixe est un détail d'adaptateur.
async fn resolve(pool: &Pool, args: &Value) -> Result<(Arc<Eap>, Value, String), String> {
    let mut a = args.clone();
    let mut sel: Option<String> = args["daemon"].as_str().map(str::to_owned);
    if let Some(t) = args["tab"].as_str() {
        if let Some((name, num)) = t.split_once(':') {
            sel = Some(name.to_string());
            if let Ok(n) = num.parse::<u64>() {
                a["tab"] = json!(n);
            }
        }
    }
    let port = match &sel {
        Some(s) => select_daemon(s)?,
        None => default_daemon(pool)?,
    };
    // Le nom du daemon sert au retour qualifié (`nom:N`).
    let name = if let Some(s) = sel {
        s
    } else {
        live_announces()
            .into_iter()
            .find(|x| x.port == port)
            .map_or_else(|| port.to_string(), |x| x.name)
    };
    a.as_object_mut().map(|m| m.remove("daemon"));
    Ok((pool.conn(port).await, a, name))
}

/// Localise `explorer-daemon` pour `daemon_spawn` — à côté
/// de notre exe (même target dir) puis le PATH en repli.
fn daemon_exe() -> Option<std::path::PathBuf> {
    let exe_name = if cfg!(windows) {
        "explorer-daemon.exe"
    } else {
        "explorer-daemon"
    };
    if let Ok(me) = std::env::current_exe() {
        let sib = me.parent().map(|d| d.join(exe_name));
        if let Some(p) = sib.filter(|p| p.is_file()) {
            return Some(p);
        }
    }
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|d| d.join(exe_name))
            .find(|p| p.is_file())
    })
}

/// `daemon_spawn{profile,name?,port?}` — démarre un daemon
/// détaché, attend son annonce (≤6 s), rend `{name,port,pid}`.
/// Le profil est exigé : dupliquer le profil par défaut
/// mourrait sur le verrou — autant le dire avant de fork.
async fn spawn_daemon(args: &Value) -> Result<Value, String> {
    let profile = args["profile"]
        .as_str()
        .ok_or("daemon_spawn : `profile` requis (répertoire dédié)")?;
    let exe = daemon_exe().ok_or("explorer-daemon introuvable (à côté du cli ni PATH)")?;
    let port = args["port"].as_u64().unwrap_or(0);
    let mut cmd = std::process::Command::new(exe);
    cmd.arg("--profile")
        .arg(profile)
        .arg("--port")
        .arg(port.to_string());
    if let Some(n) = args["name"].as_str() {
        cmd.arg("--name").arg(n);
    }
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    let child = cmd.spawn().map_err(|e| format!("spawn : {e}"))?;
    let pid = child.id();
    // Attente de l'annonce — le fichier porte le pid :
    // correspondance exacte, pas de course avec un homonyme.
    for _ in 0..60 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        if let Some((_, a)) = explorer_core::daemon_registry::list()
            .into_iter()
            .find(|(_, a)| a.pid == pid)
        {
            return Ok(json!({"name": a.name, "port": a.port, "pid": a.pid,
                             "profile": a.profile}));
        }
    }
    Err(format!(
        "daemon pid {pid} sans annonce après 6 s — vérifie le profil \
         (verrou d'instance ?) ou lance explorer-daemon à la main"
    ))
}

/// `daemon_stop{name|port}` — arrêt ciblé par pid d'annonce.
/// Windows : `taskkill /PID /T /F` (le handler Ctrl-C n'est
/// pas joignable de l'extérieur) ; unix : SIGTERM (arrêt
/// gracieux géré côté daemon). Jamais de `/IM` — on vise un
/// pid précis, pas un nom d'image.
fn stop_daemon(args: &Value) -> Result<Value, String> {
    let sel = args["daemon"]
        .as_str()
        .or_else(|| args["name"].as_str())
        .map(str::to_owned)
        .or_else(|| args["port"].as_u64().map(|p| p.to_string()))
        .ok_or("daemon_stop : `daemon` (nom ou port) requis")?;
    let port = select_daemon(&sel)?;
    let Some((file, a)) = explorer_core::daemon_registry::list()
        .into_iter()
        .find(|(_, a)| a.port == port)
    else {
        return Err(format!("aucune annonce pour {sel} — déjà arrêté ?"));
    };
    // stdio null : le serveur MCP vit sur stdout — la sortie
    // d'un enfant hérité y corromprait le protocole JSON-RPC.
    #[cfg(windows)]
    let status = std::process::Command::new("taskkill")
        .args(["/PID", &a.pid.to_string(), "/T", "/F"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    #[cfg(not(windows))]
    let status = std::process::Command::new("kill")
        .arg(a.pid.to_string())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    match status {
        Ok(s) if s.success() => {
            explorer_core::daemon_registry::remove(&file);
            Ok(json!({"stopped": true, "name": a.name, "pid": a.pid}))
        }
        Ok(s) => Err(format!("arrêt pid {} : code {}", a.pid, s)),
        Err(e) => Err(format!("arrêt pid {} : {e}", a.pid)),
    }
}

/// `daemon_list` — le registre sondé : vivants (PID) +
/// profils servis + `responsive` (la sonde `/api/status`
/// mesure la réactivité, pas la mort — un daemon saturé
/// apparaît `false` sans être évincé). L'œil de l'agent.
async fn list_daemons() -> Value {
    let live = live_announces();
    let mut out = Vec::new();
    for a in &live {
        out.push(json!({"name": a.name, "port": a.port, "pid": a.pid,
                        "profile": a.profile, "version": a.version,
                        "responsive": probe_daemon(a.port).await}));
    }
    json!(out)
}

async fn serve_async(port: Option<u16>) -> std::io::Result<u8> {
    let pool = Arc::new(Pool {
        conns: tokio::sync::Mutex::new(HashMap::new()),
        default_port: port,
    });
    // Port épinglé : échec franc au démarrage si absent ;
    // mode broker pur : le premier appel résout sa cible.
    if let Some(p) = port {
        pool.conn(p)
            .await
            .ensure()
            .await
            .map_err(std::io::Error::other)?;
    }
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
        if let Some(resp) = handle(&pool, &req).await {
            let mut out = stdout.lock();
            writeln!(out, "{resp}")?;
            out.flush()?;
        }
    }
    // EOF : close honnête sur CHAQUE daemon touché — l'agent
    // part, les onglets survivent (contrat `session.close`).
    for e in pool.conns.lock().await.values() {
        let _ = e.call("session.close", json!({})).await;
    }
    Ok(0)
}

/// Une requête → une réponse ; `None` pour les notifications.
async fn handle(pool: &Arc<Pool>, req: &Value) -> Option<Value> {
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
            Some(call_tool(pool, id.as_ref(), name, &args).await)
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
            "Arbre sémantique → {nodes|diff} {tab,since?|since_seq?}",
            obj(
                json!({"tab":{"type":"integer"},"since":{"type":"integer"},"since_seq":{"type":"integer"},"interactive_only":{"type":"boolean"},"max_nodes":{"type":"integer"}}),
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
            "submit",
            "Soumet le <form> englobant la ref {tab,ref}",
            obj(
                json!({"tab":{"type":"integer"},"ref":{"type":"string"}}),
                &["tab", "ref"]
            )
        ),
        t(
            "select_option",
            "Choisit une option du <select> {tab,ref,value?|index?}",
            obj(
                json!({"tab":{"type":"integer"},"ref":{"type":"string"},"value":{"type":"string"},"index":{"type":"integer"}}),
                &["tab", "ref"]
            )
        ),
        t(
            "click_at",
            "Clic coordonnées — dégradé {tab,x,y,space?}",
            obj(
                json!({"tab":{"type":"integer"},"x":{"type":"number"},"y":{"type":"number"},"space":{"type":"string"}}),
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
            "Attend {tab,kind:commit|frame|selector|stable|network_idle,selector?,quiet_ms?,timeout_ms?}",
            obj(
                json!({"tab":{"type":"integer"},"kind":{"type":"string"},"selector":{"type":"string"},"quiet_ms":{"type":"integer"},"timeout_ms":{"type":"integer"}}),
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
            "JS {tab,expr,ref?,timeout_ms?} → valeur JSON (≤64 Ko)",
            obj(
                json!({"tab":{"type":"integer"},"expr":{"type":"string"},"ref":{"type":"string"},
                       "timeout_ms":{"type":"integer"}}),
                &["tab", "expr"]
            )
        ),
        t("dom", "Dump DOM sérialisé {tab}", tab()),
        t("console", "Erreurs pipeline enregistrées {tab}", tab()),
    ])
}

/// Catalogue introspection `engine.*` — troisième section
/// (borne de lignes clippy, même motif que `act_tools`).
fn engine_tools() -> Value {
    let t = |name: &str, desc: &str, schema: Value| json!({"name": name, "description": desc, "inputSchema": schema});
    let tab =
        || json!({"type":"object","properties":{"tab":{"type":"integer"}},"required":["tab"]});
    let obj =
        |props: Value, req: &[&str]| json!({"type":"object","properties":props,"required":req});
    json!([
        t(
            "engine_layout",
            "Arbre de boîtes posé (dump layout natif) {tab}",
            tab()
        ),
        t(
            "engine_pipeline",
            "État moteur + compteurs + mémoire + phases sync_profile {tab,reset?}",
            obj(
                json!({"tab":{"type":"integer"},"reset":{"type":"boolean"}}),
                &["tab"]
            )
        ),
        t(
            "engine_paint_stats",
            "Compteurs émission + raster du dernier paint {tab}",
            tab()
        ),
        t(
            "engine_cascade",
            "Verdicts de cascade d'un élément {tab,ref,props?|declared_only?}",
            obj(
                json!({"tab":{"type":"integer"},"ref":{"type":"string"},
                       "props":{"type":"array","items":{"type":"string"}},
                       "declared_only":{"type":"boolean"}}),
                &["tab", "ref"]
            )
        ),
        t(
            "describe",
            "Tag/attrs/rect/texte/valeur d'une ref sans re-snapshot {tab,ref,max_text?}",
            obj(
                json!({"tab":{"type":"integer"},"ref":{"type":"string"},"max_text":{"type":"integer"}}),
                &["tab", "ref"]
            )
        ),
        t(
            "engine_j17",
            "Compteurs J17 jscore cumulatifs {tab} → {armed,snapshot}",
            tab()
        ),
        t(
            "engine_anomalies",
            "Suspects d'affichage/santé bornés — indices, pas verdicts {tab,max?}",
            obj(
                json!({"tab":{"type":"integer"},"max":{"type":"integer"}}),
                &["tab"]
            )
        ),
        t(
            "net_events",
            "Complétions réseau (seq, kind, url, status, ms, cache) {tab,since_seq?}",
            obj(
                json!({"tab":{"type":"integer"},"since_seq":{"type":"integer"}}),
                &["tab"]
            )
        ),
        t(
            "frame_diff",
            "Dommage du dernier raster commis {tab}",
            tab()
        ),
    ])
}

/// Les sections du catalogue, concaténées — puis enrichies
/// du multi-daemons : `tab` accepte la forme `"nom:N"`
/// (routage par préfixe) et `daemon` est injecté sur tous
/// les outils (routage explicite), sans retoucher chaque
/// définition. Les outils `daemon_*` sont adapter-locaux —
/// jamais forwardés à EAP (étape 256).
fn catalog() -> Value {
    let mut all = tools().as_array().cloned().unwrap_or_default();
    all.extend(act_tools().as_array().cloned().unwrap_or_default());
    all.extend(engine_tools().as_array().cloned().unwrap_or_default());
    let t = |name: &str, desc: &str, schema: Value| json!({"name": name, "description": desc, "inputSchema": schema});
    all.extend([
        t(
            "daemon_list",
            "Daemons vivants du registre (nom, port, profil, pid)",
            json!({"type":"object","properties":{}}),
        ),
        t(
            "daemon_spawn",
            "Démarre un daemon détaché {profile,name?,port?} → {name,port,pid}",
            json!({"type":"object","properties":{"profile":{"type":"string"},
                   "name":{"type":"string"},"port":{"type":"integer"}},
                   "required":["profile"]}),
        ),
        t(
            "daemon_stop",
            "Arrête un daemon par nom ou port {daemon}",
            json!({"type":"object","properties":{"daemon":{"type":"string"}},
                   "required":["daemon"]}),
        ),
    ]);
    // `tab` : integer ou "nom:N" ; `daemon` : sélecteur global.
    for tool in &mut all {
        let Some(props) = tool["inputSchema"]["properties"].as_object_mut() else {
            continue;
        };
        if let Some(tab) = props.get_mut("tab") {
            tab["type"] = json!(["integer", "string"]);
            tab["description"] = "onglet — N ou \"nom-daemon:N\"".into();
        }
        props.insert(
            "daemon".to_string(),
            json!({"type":"string","description":"nom ou port du daemon cible — défaut : unique vivant ou épinglé"}),
        );
    }
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
            json!({"tab": tab, "since": args["since"], "since_seq": args["since_seq"],
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
            json!({"tab": tab, "x": args["x"], "y": args["y"], "space": args["space"]}),
        ),
        "type" => (
            "input.type",
            json!({"tab": tab, "ref": elem, "text": args["text"],
                   "clear": args["clear"].as_bool().unwrap_or(false),
                   "opts": {"timeout_ms": args["timeout_ms"]}}),
        ),
        "hover" => ("input.hover", json!({"tab": tab, "ref": elem})),
        "submit" => ("input.submit", json!({"tab": tab, "ref": elem})),
        "select_option" => (
            "input.select_option",
            json!({"tab": tab, "ref": elem, "value": args["value"], "index": args["index"]}),
        ),
        "scroll" => (
            "input.scroll",
            json!({"tab": tab, "ref": args["ref"].as_str(), "dy": args["dy"]}),
        ),
        "key" => ("input.key", json!({"tab": tab, "key": args["key"]})),
        "wait_for" => (
            "wait.for",
            json!({"tab": tab, "kind": args["kind"], "selector": args["selector"],
                   "quiet_ms": args["quiet_ms"],
                   "timeout_ms": args["timeout_ms"].as_u64().unwrap_or(5000)}),
        ),
        "eval" => (
            "eval.run",
            json!({"tab": tab, "expr": args["expr"], "ref": args["ref"],
                   "timeout_ms": args["timeout_ms"]}),
        ),
        "dom" => ("dom.get", json!({"tab": tab})),
        "console" => ("console.get", json!({"tab": tab, "limit": args["limit"]})),
        "engine_layout" => ("engine.layout", json!({"tab": tab})),
        "engine_pipeline" => (
            "engine.pipeline",
            json!({"tab": tab, "reset": args["reset"]}),
        ),
        "engine_paint_stats" => ("engine.paint_stats", json!({"tab": tab})),
        "engine_cascade" => (
            "engine.cascade",
            json!({"tab": tab, "ref": args["ref"], "props": args["props"],
                   "declared_only": args["declared_only"]}),
        ),
        "describe" => (
            "describe",
            json!({"tab": tab, "ref": args["ref"], "max_text": args["max_text"]}),
        ),
        "engine_anomalies" => ("engine.anomalies", json!({"tab": tab, "max": args["max"]})),
        "engine_j17" => ("engine.j17", json!({"tab": tab})),
        "net_events" => (
            "net.events",
            json!({"tab": tab, "since_seq": args["since_seq"]}),
        ),
        "frame_diff" => ("frame.diff", json!({"tab": tab})),
        _ => return None,
    })
}

/// `tools/call` → verbe EAP → `{content:[{type,text}]}` ;
/// l'erreur EAP devient `isError` avec son code typé.
/// Le routage multi-daemons précède la table : outils
/// `daemon_*` traités localement, sinon `resolve` déduit la
/// connexion et réécrit `tab`/`daemon` avant `to_eap`.
async fn call_tool(pool: &Arc<Pool>, id: Option<&Value>, name: &str, args: &Value) -> Value {
    let resp = |v: Value| match id {
        Some(i) => json!({"jsonrpc":"2.0","id":i,"result":v}),
        None => v,
    };
    let err = |m: String| resp(json!({"content":[{"type":"text","text":m}],"isError":true}));
    // Outils de cycle de vie — adapter-locaux, jamais EAP.
    match name {
        "daemon_list" => {
            return resp(
                json!({"content":[{"type":"text","text":list_daemons().await.to_string()}]}),
            );
        }
        "daemon_spawn" => {
            return match spawn_daemon(args).await {
                Ok(v) => resp(json!({"content":[{"type":"text","text":v.to_string()}]})),
                Err(e) => err(e),
            };
        }
        "daemon_stop" => {
            return match stop_daemon(args) {
                Ok(v) => resp(json!({"content":[{"type":"text","text":v.to_string()}]})),
                Err(e) => err(e),
            };
        }
        _ => {}
    }
    // Routage : quelle connexion, quels params.
    let (eap, a, dname) = match resolve(pool, args).await {
        Ok(r) => r,
        Err(e) => return err(e),
    };
    if name == "screenshot" {
        return screenshot_tool(
            &eap,
            id,
            a["tab"].as_u64().unwrap_or(0),
            a["path"].as_str().unwrap_or("frame.png"),
            &a,
        )
        .await;
    }
    let Some((verb, params)) = to_eap(name, &a) else {
        return err(format!("outil inconnu : {name}"));
    };
    match eap.call(verb, params).await {
        Ok(mut r) => {
            // `tab.open` : l'id rendu est qualifié `nom:N`
            // pour que l'agent route ses appels suivants
            // sans préciser `daemon` à chaque fois.
            if name == "tab_new" {
                if let Some(n) = r["tab"].as_u64() {
                    r["tab"] = json!(format!("{dname}:{n}"));
                }
                r["daemon"] = json!(dname);
            }
            resp(json!({"content":[{"type":"text","text":r.to_string()}]}))
        }
        Err(e) => err(e),
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
