// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! TLS via `rustls` + `webpki-roots` (étape 5, ADR-0002).
//!
//! [`TlsConnector`] implémente la couture [`Connector`] d'étape 4 :
//! TCP en clair pour `http:`, handshake TLS pour `https:`. Les
//! erreurs de certificat/handshake remontent en [`NetError::Tls`]
//! typées — l'UI affichera une page d'erreur, jamais un crash.
//!
//! Crypto provider : `ring` (exception `*-sys*` consignée dans
//! ADR-0002 — aucune pile TLS viable n'est 100 % Rust ; `ring` est
//! le choix audité de-facto et se build sous Windows sans cmake).

use std::io;
use std::sync::Arc;

use async_trait::async_trait;
use rustls::pki_types::ServerName;
use rustls::{ClientConfig, RootCertStore};
use tokio_rustls::TlsConnector as RustlsConnector;

use crate::error::{NetError, NetResult};
use crate::h1::{Connector, IoStream, TcpConnector};

/// Connecteur `http:` (TCP clair) + `https:` (TLS rustls).
pub struct TlsConnector {
    tcp: Arc<dyn Connector>,
    tls: RustlsConnector,
}

impl TlsConnector {
    /// Connecteur TLS avec un magasin de racines explicite, sur
    /// TCP direct.
    #[must_use]
    pub fn with_roots(roots: RootCertStore) -> Self {
        Self::with_inner(roots, Arc::new(TcpConnector))
    }

    /// Connecteur TLS dont la couche TCP passe par `inner` —
    /// composition : [`crate::proxy::ProxyConnector`] pour
    /// `CONNECT`+TLS, un fake en test.
    #[must_use]
    pub fn with_inner(roots: RootCertStore, inner: Arc<dyn Connector>) -> Self {
        let config = ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();
        Self {
            tcp: inner,
            tls: RustlsConnector::from(Arc::new(config)),
        }
    }

    /// Connecteur TLS avec les racines Mozilla (`webpki-roots`).
    #[must_use]
    pub fn web_roots() -> Self {
        Self::with_roots(RootCertStore {
            roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
        })
    }
}

impl Default for TlsConnector {
    fn default() -> Self {
        Self::web_roots()
    }
}

impl std::fmt::Debug for TlsConnector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TlsConnector").finish_non_exhaustive()
    }
}

#[async_trait]
impl Connector for TlsConnector {
    async fn connect(&self, scheme: &str, host: &str, port: u16) -> NetResult<Box<dyn IoStream>> {
        match scheme {
            "http" => self.tcp.connect(scheme, host, port).await,
            "https" => {
                let target = format!("{scheme}://{host}:{port}");
                // La couche sous-jacente peut être un tunnel
                // CONNECT (proxy) — `inner` décide.
                let tcp = self.tcp.connect("https", host, port).await?;

                // SNI : nom DNS ou adresse IP littérale — jamais vide.
                let name = ServerName::try_from(host.to_string()).map_err(|_| NetError::Tls {
                    target: target.clone(),
                    message: format!("nom de serveur invalide : {host}"),
                })?;

                let tls = self
                    .tls
                    .connect(name, tcp)
                    .await
                    .map_err(|e| NetError::Tls {
                        target,
                        message: tls_message(&e),
                    })?;
                Ok(Box::new(tls))
            }
            other => Err(NetError::UnsupportedScheme(other.to_string())),
        }
    }

    /// Délègue à l'inner — le proxy décide de la forme.
    fn absolute_form(&self, scheme: &str, host: &str) -> bool {
        self.tcp.absolute_form(scheme, host)
    }
}

/// Découpe le message d'erreur handshake : erreurs `rustls` portées
/// par `io::Error` (certificat, ALPN, alerte fatale).
fn tls_message(e: &io::Error) -> String {
    if let Some(inner) = e.get_ref() {
        format!("{inner}")
    } else {
        format!("{e}")
    }
}
