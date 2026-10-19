// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Support proxy (étape 8, ADR-0002).
//!
//! - [`ProxyConfig::from_env`] : `http_proxy`, `https_proxy`,
//!   `no_proxy` (suffixes, `*` = tout, `.` = sous-domaines) —
//!   fonctionnalité standard des navigateurs ; la couture
//!   [`Connector`] reste ouverte à tout mandataire futur
//!   (proxy d'entreprise, Tor…).
//! - [`ProxyConnector`] : `http:` → TCP direct vers le proxy,
//!   requêtes en *absolute-form* (`GET http://h/p HTTP/1.1`) via
//!   [`Connector::absolute_form`] ; `https:` → tunnel `CONNECT`
//!   puis TLS par-dessus (composition : passer le connecteur
//!   comme `inner` de [`crate::tls::TlsConnector`]).

use std::env;

use async_trait::async_trait;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::error::{NetError, NetResult};
use crate::h1::{Connector, IoStream, TcpConnector};
use crate::url::Url;

/// Configuration proxy issue de l'environnement.
#[derive(Debug, Clone, Default)]
pub struct ProxyConfig {
    /// Proxy pour les URLs `http:`.
    pub http: Option<Url>,
    /// Proxy pour les URLs `https:` (tunnel `CONNECT`).
    pub https: Option<Url>,
    /// Suffixes `no_proxy` — `*` = tout bypass, `.dom` = suffixe.
    pub no_proxy: Vec<String>,
}

impl ProxyConfig {
    /// Lit `http_proxy`/`https_proxy`/`no_proxy` (+ variantes
    /// majuscules) de l'environnement.
    #[must_use]
    pub fn from_env() -> Self {
        let get = |k: &str| env::var(k).or_else(|_| env::var(k.to_uppercase())).ok();
        Self {
            http: get("http_proxy").and_then(|v| Url::parse(&v).ok()),
            https: get("https_proxy").and_then(|v| Url::parse(&v).ok()),
            no_proxy: get("no_proxy")
                .map(|v| {
                    v.split(',')
                        .map(|s| s.trim().to_ascii_lowercase())
                        .filter(|s| !s.is_empty())
                        .collect()
                })
                .unwrap_or_default(),
        }
    }

    /// `true` si `host` bypass le proxy (`no_proxy`).
    #[must_use]
    pub fn is_bypassed(&self, host: &str) -> bool {
        let h = host
            .trim_start_matches('[')
            .trim_end_matches(']')
            .to_ascii_lowercase();
        self.no_proxy.iter().any(|p| {
            // Normalise `*.dom`/`.dom`/`dom` → suffixe de domaine.
            let p = p.trim_start_matches('*');
            p == "*"
                || p == h
                || (p.starts_with('.') && h.ends_with(p))
                || h.ends_with(&format!(".{p}"))
        })
    }

    /// Proxy effectif pour `scheme` (ou `None` → direct).
    #[must_use]
    pub fn proxy_for(&self, scheme: &str) -> Option<&Url> {
        match scheme {
            "http" => self.http.as_ref(),
            "https" => self.https.as_ref(),
            _ => None,
        }
    }
}

/// Connecteur via proxy : `http:` → connexion directe au proxy +
/// absolute-form ; `https:` → tunnel `CONNECT`.
pub struct ProxyConnector {
    cfg: ProxyConfig,
    tcp: TcpConnector,
}

impl ProxyConnector {
    /// Connecteur proxy depuis `cfg`.
    #[must_use]
    pub fn new(cfg: ProxyConfig) -> Self {
        Self {
            cfg,
            tcp: TcpConnector,
        }
    }

    /// Lit la réponse `CONNECT` jusqu'à `\r\n\r\n`, vérifie 2xx.
    async fn read_connect_response(stream: &mut TcpStream, target: &str) -> NetResult<()> {
        let mut buf = Vec::with_capacity(512);
        let mut chunk = [0u8; 1024];
        loop {
            let n = stream
                .read(&mut chunk)
                .await
                .map_err(|e| NetError::Connect {
                    target: target.to_string(),
                    source: e,
                })?;
            if n == 0 {
                return Err(NetError::InvalidResponse(
                    "proxy a fermé pendant CONNECT".to_string(),
                ));
            }
            buf.extend_from_slice(&chunk[..n]);
            if buf.windows(4).any(|w| w == b"\r\n\r\n") || buf.len() > 8192 {
                break;
            }
        }
        let head = String::from_utf8_lossy(&buf);
        let status = head
            .split_whitespace()
            .nth(1)
            .and_then(|s| s.parse::<u16>().ok())
            .unwrap_or(0);
        if (200..300).contains(&status) {
            Ok(())
        } else {
            Err(NetError::InvalidResponse(format!(
                "proxy CONNECT → {status}"
            )))
        }
    }
}

#[async_trait]
impl Connector for ProxyConnector {
    async fn connect(&self, scheme: &str, host: &str, port: u16) -> NetResult<Box<dyn IoStream>> {
        let Some(proxy) = self.cfg.proxy_for(scheme).cloned() else {
            return self.tcp.connect(scheme, host, port).await;
        };
        if self.cfg.is_bypassed(host) {
            return self.tcp.connect(scheme, host, port).await;
        }
        let proxy_port = proxy.effective_port().unwrap_or(8080);
        let proxy_host = proxy.host();
        match scheme {
            "http" => {
                // TCP direct au proxy ; la requête partira en
                // absolute-form (signalée par `absolute_form`).
                self.tcp.connect("http", &proxy_host, proxy_port).await
            }
            "https" => {
                let mut stream = TcpStream::connect((proxy_host.as_str(), proxy_port))
                    .await
                    .map_err(|e| NetError::Connect {
                        target: format!("proxy {proxy_host}:{proxy_port}"),
                        source: e,
                    })?;
                let target = format!("{host}:{port}");
                stream
                    .write_all(
                        format!("CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n\r\n").as_bytes(),
                    )
                    .await
                    .map_err(|e| NetError::Connect {
                        target: target.clone(),
                        source: e,
                    })?;
                Self::read_connect_response(&mut stream, &target).await?;
                Ok(Box::new(stream))
            }
            other => Err(NetError::UnsupportedScheme(other.to_string())),
        }
    }

    /// `http:` via proxy → requête en *absolute-form*
    /// (`GET http://h/p HTTP/1.1`).
    fn absolute_form(&self, scheme: &str, host: &str) -> bool {
        scheme == "http" && self.cfg.proxy_for(scheme).is_some() && !self.cfg.is_bypassed(host)
    }
}
