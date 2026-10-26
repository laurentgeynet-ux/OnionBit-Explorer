// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! `media_proxy` — proxy local de lecture média (étape 139a,
//! ADR-0024 amendée) : même câblage que la référence
//! `iptv-infra::network::local_proxy` — tout flux de lecture
//! passe par un endpoint loopback streamé, qui réémet via
//! [`HttpTransport`] (politique, cookies, headers custom,
//! `Range` propagé — requis pour le seek vidéo).
//!
//! Le listener écoute sur `127.0.0.1` à port éphémère, partagé
//! par daemon et mode FFI (il vit dans le processus moteur).

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::{Query, Request as AxRequest, State};
use axum::response::Response as AxResponse;
use axum::routing::get;
use axum::Router;
use futures_util::StreamExt;

use crate::http::{HttpTransport, Request};
use crate::url::Url;

/// En-têtes propagés du client vers le flux amont (le lecteur
/// les pose : `Range` pour le seek, `Accept`, `Cookie`).
const FORWARD_IN: &[&str] =
    &["range", "accept", "cookie", "if-range", "user-agent"];

/// En-têtes propagés de la réponse amont vers le client.
const FORWARD_OUT: &[&str] = &[
    "content-type",
    "content-length",
    "content-range",
    "accept-ranges",
    "etag",
    "last-modified",
    "cache-control",
];

/// Proxy média local — un listener loopback par processus.
pub struct MediaProxy {
    addr: SocketAddr,
}

impl MediaProxy {
    /// Démarre le listener sur `127.0.0.1:0` (port éphémère) et
    /// le sert en tâche de fond. `ua` = User-Agent injecté si le
    /// client n'en pose pas.
    ///
    /// # Errors
    /// `std::io::Error` si le bind échoue.
    pub async fn spawn(
        transport: Arc<dyn HttpTransport>,
        ua: String,
    ) -> std::io::Result<Self> {
        let listener =
            tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let app = Router::new().route("/m", get(handle)).with_state(
            ProxyState { transport, ua },
        );
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        Ok(Self { addr })
    }

    /// Adresse d'écoute.
    #[must_use]
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// URL proxyfiée pour `target` — à remettre au lecteur hôte
    /// à la place de l'URL distante.
    #[must_use]
    pub fn url(&self, target: &str) -> String {
        format!("http://{}/m?u={}", self.addr, encode(target))
    }
}

fn encode(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => {
                c.to_string()
            }
            _ => {
                let mut b = String::new();
                let mut tmp = [0u8; 4];
                for byte in c.encode_utf8(&mut tmp).as_bytes() {
                    b.push_str(&format!("%{byte:02X}"));
                }
                b
            }
        })
        .collect()
}

#[derive(Clone)]
struct ProxyState {
    transport: Arc<dyn HttpTransport>,
    ua: String,
}

async fn handle(
    State(st): State<ProxyState>,
    Query(q): Query<HashMap<String, String>>,
    req: AxRequest,
) -> AxResponse {
    let Some(target) = q.get("u") else {
        return AxResponse::builder()
            .status(400)
            .body(Body::from("paramètre `u` requis"))
            .unwrap_or_else(|_| AxResponse::new(Body::empty()));
    };
    let Ok(url) = Url::parse(target) else {
        return AxResponse::builder()
            .status(400)
            .body(Body::from("URL invalide"))
            .unwrap_or_else(|_| AxResponse::new(Body::empty()));
    };
    let mut r = Request::get(url);
    let mut has_ua = false;
    for (name, value) in req.headers() {
        let n = name.as_str().to_lowercase();
        if FORWARD_IN.contains(&n.as_str()) {
            has_ua |= n == "user-agent";
            if let Ok(v) = value.to_str() {
                r.headers.push((n, v.to_string()));
            }
        }
    }
    if !has_ua {
        r.headers.push(("user-agent".into(), st.ua.clone()));
    }
    match st.transport.fetch_stream(&r).await {
        Ok(mut up) => {
            let mut b = AxResponse::builder().status(up.status);
            for (n, v) in &up.headers {
                if FORWARD_OUT.contains(&n.to_lowercase().as_str()) {
                    b = b.header(n.as_str(), v.as_str());
                }
            }
            // Corps streamé chunk-par-chunk (zéro-copie de la
            // référence : jamais de bufferisation pleine).
            let stream = futures_util::stream::poll_fn(move |cx| {
                up.body.poll_recv(cx)
            })
            .map(|r| {
                r.map(axum::body::Bytes::from).map_err(|e| {
                    std::io::Error::new(std::io::ErrorKind::Other, e.to_string())
                })
            });
            match b.body(Body::from_stream(stream)) {
                Ok(resp) => resp,
                Err(_) => AxResponse::new(Body::empty()),
            }
        }
        Err(e) => AxResponse::builder()
            .status(502)
            .body(Body::from(format!("flux amont : {e}")))
            .unwrap_or_else(|_| AxResponse::new(Body::empty())),
    }
}
