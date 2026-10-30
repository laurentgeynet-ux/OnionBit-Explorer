// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! HSTS (RFC 6797) — décorateur [`HstsTransport`] : upgrade
//! `http→https` pour les hôtes connus et mémorise
//! `Strict-Transport-Security` sur les réponses `https`.
//!
//! Le store est **injecté** (port [`HstsStore`]) — la
//! persistance vit chez l'hôte (`state/hsts.json` —
//! `JsonHstsStore` côté `explorer-core`, PR-3) ; ce module
//! reste pur transport, aucun I/O fichier.

use std::sync::Arc;

use async_trait::async_trait;

use crate::error::NetResult;
use crate::http::{HttpTransport, Request};

/// Store d'hôtes HSTS — injecté au montage de la pile.
pub trait HstsStore: Send + Sync {
    /// `true` si `host` est couvert par une politique active
    /// (hôte exact ou domaine parent `includeSubDomains`,
    /// non expirée) — upgrade `http→https` requis.
    fn is_secure(&self, host: &str) -> bool;
    /// Mémorise `host` pour `max_age` secondes ; `0` = retrait
    /// (RFC 6797 §8.1 — le serveur retire sa politique).
    fn record(&self, host: &str, max_age: u64, include_subdomains: bool);
}

/// Parse `Strict-Transport-Security` → `(max_age_secs,
/// include_subdomains)`. Directives case-insensitive ;
/// `max-age` absent/invalide → `None` (en-tête ignoré).
/// `max-age=0` renvoie `Some((0, _))` — retrait géré par le
/// store, pas par le parse.
#[must_use]
pub fn parse_sts(value: &str) -> Option<(u64, bool)> {
    let mut max_age = None;
    let mut subs = false;
    for dir in value.split(';') {
        let dir = dir.trim();
        if dir.eq_ignore_ascii_case("includesubdomains") {
            subs = true;
        } else if let Some((k, v)) = dir.split_once('=') {
            if k.trim().eq_ignore_ascii_case("max-age") {
                max_age = v.trim().parse::<u64>().ok();
            }
        }
    }
    max_age.map(|a| (a, subs))
}

/// Réécrit `http://…` en `https://…` (schéma seul — le
/// `Url` opaque/garde sa forme).
fn upgrade(url: &crate::url::Url) -> Option<crate::url::Url> {
    let s = url.to_string();
    s.strip_prefix("http:")
        .and_then(|rest| crate::url::Url::parse(&format!("https:{rest}")).ok())
}

/// Décorateur : **avant** fetch, `http` + hôte connu → URL
/// réécrite `https` (upgrade interne — pas de round-trip de
/// redirection) ; **après** réponse `https`, un en-tête STS
/// valide est mémorisé. Placé sous la couche cache : les
/// upgrades réutilisent les entrées existantes.
pub struct HstsTransport {
    inner: Arc<dyn HttpTransport>,
    store: Arc<dyn HstsStore>,
}

impl HstsTransport {
    /// Enveloppe `inner` avec le `store` injecté.
    #[must_use]
    pub fn wrap(inner: Arc<dyn HttpTransport>, store: Arc<dyn HstsStore>) -> Self {
        Self { inner, store }
    }
}

#[async_trait]
impl HttpTransport for HstsTransport {
    async fn fetch(&self, req: &Request) -> NetResult<crate::http::Response> {
        let mut upgraded = None;
        if req.url.scheme() == "http" && self.store.is_secure(req.url.host().as_str()) {
            let mut r = req.clone();
            if let Some(u) = upgrade(&req.url) {
                r.url = u;
                upgraded = Some(r);
            }
        }
        let req = upgraded.as_ref().unwrap_or(req);
        let resp = self.inner.fetch(req).await?;
        // STS n'a d'effet que servi en https (RFC 6797 §7.1 —
        // jamais honoré en clair).
        if req.url.scheme() == "https" {
            if let Some(v) = resp.header("strict-transport-security") {
                if let Some((age, subs)) = parse_sts(v) {
                    self.store.record(req.url.host().as_str(), age, subs);
                }
            }
        }
        Ok(resp)
    }
}
