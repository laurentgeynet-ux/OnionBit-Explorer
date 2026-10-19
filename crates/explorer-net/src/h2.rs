// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! HTTP/2 (étape 41) — [`AutoClient`], transport `https` qui
//! négocie la version par ALPN (RFC 7301) : `h2` → connexion
//! multiplexée partagée par origine, `http/1.1` (ou absence
//! d'ALPN) → repli sur la pile [`H1Client`] existante, sur la
//! connexion déjà ouverte.
//!
//! Le framing/multiplexage est délégué au crate `h2` (pur Rust,
//! équipe tokio — même catégorie d'infrastructure que `httparse`
//! pour HTTP/1.1) ; la politique au-dessus (pool par origine,
//! budgets, repli) est écrite ici.
//!
//! Périmètre assumé : pas de `h2c` (HTTP/2 en clair — quasi
//! inexistant hors spec), pas de push server (déprécié par les
//! navigateurs), `SETTINGS`/`WINDOW_UPDATE` aux défauts `h2`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use bytes::Bytes;
use tokio::time::timeout;

use crate::error::{NetError, NetResult};
use crate::h1::{pool_key, Conn, Connector, H1Client, HttpConfig, PoolKey};
use crate::http::{HttpTransport, Request, Response, StreamedResponse};

/// Type alias de la poignée d'envoi multiplexée `h2`.
type SendRequest = h2::client::SendRequest<Bytes>;

/// En-têtes interdits sur HTTP/2 (RFC 9113 §8.2.2 — propres à la
/// connexion, remplacés par les pseudo-en-têtes). `te` n'est
/// admis qu'en valeur `trailers` — on retire simplement.
fn forbidden_h2(name: &str) -> bool {
    name.eq_ignore_ascii_case("connection")
        || name.eq_ignore_ascii_case("keep-alive")
        || name.eq_ignore_ascii_case("proxy-connection")
        || name.eq_ignore_ascii_case("transfer-encoding")
        || name.eq_ignore_ascii_case("upgrade")
        || name.eq_ignore_ascii_case("te")
        || name.eq_ignore_ascii_case("host")
}

/// Transport auto-négocié : h2 si ALPN `h2`, h1 sinon. Une
/// connexion h2 par origine, multiplexée (un `SendRequest` cloné
/// par requête) — `http:` passe directement en h1 (pas de `h2c`).
pub struct AutoClient {
    /// Pile HTTP/1.1 complète (repli + `http:`).
    h1: H1Client,
    /// Connecteur négociant ALPN (`TlsConnector` en prod).
    connector: Arc<dyn Connector>,
    cfg: HttpConfig,
    /// Connexions h2 vivantes par origine.
    h2: Mutex<HashMap<PoolKey, SendRequest>>,
}

impl AutoClient {
    /// Client auto sur connecteur injecté.
    #[must_use]
    pub fn new(connector: Arc<dyn Connector>, cfg: HttpConfig) -> Self {
        Self {
            h1: H1Client::new(connector.clone(), cfg.clone()),
            connector,
            cfg,
            h2: Mutex::new(HashMap::new()),
        }
    }

    /// Vide les connexions idle (h1 pool + h2 — les `SendRequest`
    /// tombés ferment leur pilote).
    pub fn close_idle(&self) {
        self.h1.close_idle();
        self.h2
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
    }

    fn h2_conn(&self, key: &PoolKey) -> Option<SendRequest> {
        self.h2
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(key)
            .cloned()
    }

    fn h2_forget(&self, key: &PoolKey) {
        self.h2
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(key);
    }

    /// Ouverture h2 : handshake `h2` sur le flot, spawn du pilote
    /// de connexion, enregistrement par origine.
    async fn h2_handshake(&self, key: &PoolKey, conn: Conn) -> NetResult<SendRequest> {
        let (sr, driver) = timeout(self.cfg.connect_timeout, h2::client::handshake(conn.stream))
            .await
            .map_err(|_| NetError::Timeout {
                phase: "connect",
                ms: u64::try_from(self.cfg.connect_timeout.as_millis()).unwrap_or(u64::MAX),
                target: format!("{}://{}:{}", key.scheme, key.host, key.port),
            })?
            .map_err(|e| NetError::Http2(format!("handshake {e}")))?;
        // Le pilote fait vivre la connexion (frames de contrôle,
        // flow-control) ; sa mort casse les `SendRequest`.
        tokio::spawn(async move {
            if let Err(e) = driver.await {
                tracing::debug!(%e, "connexion h2 terminée");
            }
        });
        self.h2
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(key.clone(), sr.clone());
        Ok(sr)
    }

    /// Envoie la requête sur connexion h2 et attend l'en-tête de
    /// réponse — partagé par [`Self::h2_send`] (corps bufferisé)
    /// et [`Self::h2_send_stream`] (corps streamé, étape 53).
    /// Le corps requête est envoyé par tranches sur le crédit de
    /// flow-control.
    async fn h2_request(
        &self,
        sr: SendRequest,
        req: &Request,
    ) -> NetResult<http::Response<h2::RecvStream>> {
        let mut sr = self
            .io(req, sr.ready())
            .await?
            .map_err(|e| NetError::Http2(format!("ready {e}")))?;

        let mut builder = http::Request::builder()
            .method(req.method.as_str())
            .uri(req.url.as_str())
            .header("user-agent", &self.cfg.user_agent);
        for (n, v) in &req.headers {
            if !forbidden_h2(n) {
                builder = builder.header(n.as_str(), v.as_str());
            }
        }
        let hreq = builder
            .body(())
            .map_err(|e| NetError::Http2(format!("request build {e}")))?;

        let (resp_fut, mut send) = sr
            .send_request(hreq, req.body.is_none())
            .map_err(|e| NetError::Http2(format!("send {e}")))?;
        // Corps : la fenêtre de flow-control h2 peut être
        // inférieure à la taille du corps — on réserve, on attend
        // le crédit, on envoie par tranches jusqu'à `eos`.
        if let Some(body) = &req.body {
            send.reserve_capacity(body.len());
            let mut written = 0;
            while written < body.len() {
                let avail = self
                    .io(
                        req,
                        futures_util::future::poll_fn(|cx| send.poll_capacity(cx)),
                    )
                    .await?
                    .ok_or_else(|| NetError::Http2("stream fermé avant envoi du corps".into()))?
                    .map_err(|e| NetError::Http2(format!("capacity {e}")))?;
                if avail == 0 {
                    continue;
                }
                let n = avail.min(body.len() - written);
                let last = written + n == body.len();
                send.send_data(Bytes::copy_from_slice(&body[written..written + n]), last)
                    .map_err(|e| NetError::Http2(format!("body {e}")))?;
                written += n;
            }
        }

        self.io(req, resp_fut)
            .await?
            .map_err(|e| NetError::Http2(format!("response {e}")))
    }

    /// Une requête sur connexion h2 — corps borné par
    /// `max_body_bytes` (défense anti-mémoire identique à h1).
    async fn h2_send(&self, sr: SendRequest, req: &Request) -> NetResult<Response> {
        let mut resp = self.h2_request(sr, req).await?;

        let headers: Vec<(String, String)> = resp
            .headers()
            .iter()
            .map(|(n, v)| (n.to_string(), v.to_str().unwrap_or_default().to_string()))
            .collect();

        // Corps : collecte bornée — chaque chunk libère sa fenêtre
        // de flow-control pour ne pas étrangler la connexion.
        let mut body = Vec::new();
        while let Some(chunk) = self.io(req, resp.body_mut().data()).await? {
            let chunk = chunk.map_err(|e| NetError::Http2(format!("data {e}")))?;
            body.extend_from_slice(&chunk);
            let _ = resp.body_mut().flow_control().release_capacity(chunk.len());
            if body.len() > self.cfg.max_body_bytes {
                return Err(NetError::BodyTooLarge(self.cfg.max_body_bytes));
            }
        }

        Ok(Response {
            status: resp.status().as_u16(),
            reason: String::new(), // h2 n'a pas de reason phrase.
            headers,
            body,
            reused_connection: true, // multiplexage = toujours partagée.
        })
    }

    /// Variante streamée de [`Self::h2_send`] (étape 53) : la
    /// réception tourne en tâche — chaque `DATA` part sur le
    /// canal et libère sa fenêtre de flow-control aussitôt.
    async fn h2_send_stream(&self, sr: SendRequest, req: &Request) -> NetResult<StreamedResponse> {
        let mut resp = self.h2_request(sr, req).await?;
        let headers: Vec<(String, String)> = resp
            .headers()
            .iter()
            .map(|(n, v)| (n.to_string(), v.to_str().unwrap_or_default().to_string()))
            .collect();
        let status = resp.status().as_u16();
        let (tx, rx) = tokio::sync::mpsc::channel(4);
        let io_timeout = self.cfg.io_timeout;
        let max_body = self.cfg.max_body_bytes;
        let target = target_of(req);
        tokio::spawn(async move {
            let mut total = 0usize;
            loop {
                let Ok(next) = timeout(io_timeout, resp.body_mut().data()).await else {
                    let _ = tx
                        .send(Err(NetError::Timeout {
                            phase: "io",
                            ms: u64::try_from(io_timeout.as_millis()).unwrap_or(u64::MAX),
                            target: target.clone(),
                        }))
                        .await;
                    return;
                };
                let Some(chunk) = next else { return };
                match chunk {
                    Ok(c) => {
                        total += c.len();
                        let _ = resp.body_mut().flow_control().release_capacity(c.len());
                        if total > max_body {
                            let _ = tx.send(Err(NetError::BodyTooLarge(total))).await;
                            return;
                        }
                        if tx.send(Ok(c.to_vec())).await.is_err() {
                            return;
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(Err(NetError::Http2(format!("data {e}")))).await;
                        return;
                    }
                }
            }
        });
        Ok(StreamedResponse {
            status,
            reason: String::new(),
            headers,
            reused_connection: true,
            body: rx,
        })
    }

    /// Timeout par opération — même facteur que `H1Client::io`.
    async fn io<T>(
        &self,
        req: &Request,
        fut: impl std::future::Future<Output = T>,
    ) -> NetResult<T> {
        timeout(self.cfg.io_timeout, fut)
            .await
            .map_err(|_| NetError::Timeout {
                phase: "io",
                ms: u64::try_from(self.cfg.io_timeout.as_millis()).unwrap_or(u64::MAX),
                target: target_of(req),
            })
    }
}

#[async_trait]
impl HttpTransport for AutoClient {
    async fn fetch(&self, req: &Request) -> NetResult<Response> {
        let key = pool_key(req)?;
        // `http:` clair → h1 direct (h2c hors périmètre).
        if key.scheme == "http" {
            return self.h1.fetch(req).await;
        }
        // Connexion h2 vivante ? Une erreur stream-level ne doit
        // pas tuer la connexion ; une erreur connexion-level la
        // déréférence et on retombe sur un reconnect complet.
        if let Some(sr) = self.h2_conn(&key) {
            match self.h2_send(sr, req).await {
                Ok(r) => return Ok(r),
                Err(e) => {
                    tracing::debug!(%e, "échec h2 — connexion abandonnée");
                    self.h2_forget(&key);
                }
            }
        }
        let conn = timeout(
            self.cfg.connect_timeout,
            self.connector
                .connect_negotiated(&key.scheme, &key.host, key.port),
        )
        .await
        .map_err(|_| NetError::Timeout {
            phase: "connect",
            ms: u64::try_from(self.cfg.connect_timeout.as_millis()).unwrap_or(u64::MAX),
            target: target_of(req),
        })??;

        if conn.alpn.as_deref() == Some(b"h2") {
            let sr = self.h2_handshake(&key, conn).await?;
            match self.h2_send(sr, req).await {
                Ok(r) => return Ok(r),
                Err(e) => {
                    self.h2_forget(&key);
                    return Err(e);
                }
            }
        }
        // ALPN `http/1.1` ou absent → h1 sur la connexion ouverte.
        self.h1.fetch_on_stream(&key, conn.stream, req).await
    }

    async fn fetch_stream(&self, req: &Request) -> NetResult<StreamedResponse> {
        let key = pool_key(req)?;
        if key.scheme == "http" {
            return self.h1.fetch_stream(req).await;
        }
        if let Some(sr) = self.h2_conn(&key) {
            match self.h2_send_stream(sr, req).await {
                Ok(r) => return Ok(r),
                Err(e) => {
                    tracing::debug!(%e, "échec h2 — connexion abandonnée");
                    self.h2_forget(&key);
                }
            }
        }
        let conn = timeout(
            self.cfg.connect_timeout,
            self.connector
                .connect_negotiated(&key.scheme, &key.host, key.port),
        )
        .await
        .map_err(|_| NetError::Timeout {
            phase: "connect",
            ms: u64::try_from(self.cfg.connect_timeout.as_millis()).unwrap_or(u64::MAX),
            target: target_of(req),
        })??;

        if conn.alpn.as_deref() == Some(b"h2") {
            let sr = self.h2_handshake(&key, conn).await?;
            return self.h2_send_stream(sr, req).await;
        }
        self.h1.stream_on_conn(&key, conn.stream, req).await
    }
}

fn target_of(req: &Request) -> String {
    format!("{}://{}", req.url.scheme(), req.url.host())
}
