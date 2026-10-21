// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Cache mémoire HTTP (étape 7, ADR-0002).
//!
//! - Clé : URL sérialisée (schéma+autorité+path+query).
//! - Fraîcheur : `Cache-Control: max-age` puis `Expires:` ; une
//!   entrée périmée est **revalidée** par `If-None-Match` /
//!   `If-Modified-Since` plutôt que re-téléchargée.
//! - Non caché : non-GET, `Cache-Control: no-store`, `Vary: *`,
//!   statuts hors liste cachable RFC 9111.
//! - `304 Not Modified` → corps + statut de l'entrée servis depuis
//!   le cache, en-têtes fusionnés (les validateurs récents
//!   remplacent).
//! - Budget : `max_entry_bytes` par entrée, `max_total_bytes`
//!   global, éviction FIFO.
//! - Mémoire seule — disque optionnel en phase ultérieure.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use async_trait::async_trait;

use crate::error::NetResult;
use crate::http::{HttpTransport, Method, Request, Response};

/// Statuts cachables sans heuristique explicite (RFC 9111 §3 liste,
/// sous-ensemble usuel).
fn cacheable_status(status: u16) -> bool {
    matches!(
        status,
        200 | 203 | 204 | 300 | 301 | 308 | 404 | 405 | 410 | 414 | 501
    )
}

/// Entrée de cache — publique (contrat [`CacheStore`],
/// étape 56 : une implémentation disque la sérialise, la
/// mémoire la clone). `stored_at`/`fresh_until` en
/// `SystemTime` — l'impl persistante les convertit en
/// secondes unix.
#[derive(Debug, Clone)]
pub struct CacheEntry {
    /// Code de statut (`200`…).
    pub status: u16,
    /// Raison (`OK`…).
    pub reason: String,
    /// En-têtes — paires `(nom, valeur)`, ordre préservé.
    pub headers: Vec<(String, String)>,
    /// Corps déframé.
    pub body: Vec<u8>,
    /// Instant de stockage.
    pub stored_at: SystemTime,
    /// Fraîcheur : `now < fresh_until` → servir sans requête.
    pub fresh_until: Option<SystemTime>,
    /// Validateur `ETag` (revalidation `If-None-Match`).
    pub etag: Option<String>,
    /// Validateur `Last-Modified` (`If-Modified-Since`).
    pub last_modified: Option<String>,
}

/// Port de stockage du cache HTTP (étape 56) — le transport
/// [`CachingTransport`] ne connaît que `get`/`put` ; les
/// implémentations : [`MemoryCache`] (volatil) ou un cache
/// disque borné côté hôte (JsonFileStore-like, daemon).
pub trait CacheStore: Send + Sync {
    /// Entrée pour `key` (URL sérialisée) — `None` si absente
    /// ou illisible (dégradation : on re-télécharge).
    fn cache_get(&self, key: &str) -> Option<CacheEntry>;
    /// Stocke `entry` sous `key` — l'impl applique ses
    /// budgets (taille d'entrée, total, éviction).
    fn cache_put(&self, key: &str, entry: CacheEntry);
}

/// Budgets du cache.
#[derive(Debug, Clone)]
pub struct CacheConfig {
    /// Taille max d'une entrée (octets).
    pub max_entry_bytes: usize,
    /// Taille totale max du cache (octets).
    pub max_total_bytes: usize,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            max_entry_bytes: 8 * 1024 * 1024,
            max_total_bytes: 64 * 1024 * 1024,
        }
    }
}

/// Cache mémoire partagé (thread-safe — les fetches courent en
/// parallèle sur tokio ; la `Mutex` n'est jamais tenue sur `.await`).
pub struct MemoryCache {
    map: Mutex<HashMap<String, CacheEntry>>,
    order: Mutex<VecDeque<String>>,
    total: Mutex<usize>,
    cfg: CacheConfig,
}

impl MemoryCache {
    /// Cache vide, budgets par défaut.
    #[must_use]
    pub fn new() -> Arc<Self> {
        Self::with_config(CacheConfig::default())
    }

    /// Cache vide, budgets explicites.
    #[must_use]
    pub fn with_config(cfg: CacheConfig) -> Arc<Self> {
        Arc::new(Self {
            map: Mutex::new(HashMap::new()),
            order: Mutex::new(VecDeque::new()),
            total: Mutex::new(0),
            cfg,
        })
    }

    /// Nombre d'entrées (tests/debug).
    #[must_use]
    pub fn len(&self) -> usize {
        self.map
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len()
    }

    /// `true` si vide.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl CacheStore for MemoryCache {
    fn cache_get(&self, key: &str) -> Option<CacheEntry> {
        self.map
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(key)
            .cloned()
    }

    fn cache_put(&self, key: &str, entry: CacheEntry) {
        let url = key.to_string();
        let bytes = entry.body.len();
        let mut map = self
            .map
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut order = self
            .order
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut total = self
            .total
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(old) = map.get(&url) {
            *total = total.saturating_sub(old.body.len());
        }
        map.insert(url.clone(), entry);
        order.push_back(url);
        *total += bytes;
        // Éviction FIFO.
        while *total > self.cfg.max_total_bytes {
            let Some(k) = order.pop_front() else { break };
            if let Some(e) = map.remove(&k) {
                *total = total.saturating_sub(e.body.len());
            }
        }
    }
}

/// Transport décorateur : cache + revalidation derrière le
/// port [`CacheStore`] (étape 56 — mémoire ou disque borné,
/// l'hôte choisit). Seul `fetch` est caché ; `fetch_stream`
/// court-circuite (corps en vol non tamponnés — documenté).
pub struct CachingTransport {
    inner: Arc<dyn HttpTransport>,
    cache: Arc<dyn CacheStore>,
    cfg: CacheConfig,
}

impl CachingTransport {
    /// Enveloppe `inner` avec le `cache` partagé et les
    /// budgets `cfg` (borne d'entrée à l'écriture — la
    /// politique d'éviction reste propre au store).
    #[must_use]
    pub fn wrap(
        inner: Arc<dyn HttpTransport>,
        cache: Arc<dyn CacheStore>,
        cfg: CacheConfig,
    ) -> Self {
        Self { inner, cache, cfg }
    }
}

/// `header` sur une liste de paires (utilitaire local — `Response`
/// expose déjà `header`, mais `Entry` stocke la liste brute).
fn hdr<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}

#[async_trait]
impl HttpTransport for CachingTransport {
    async fn fetch(&self, req: &Request) -> NetResult<Response> {
        // On ne cache que les GET.
        if req.method != Method::Get {
            return self.inner.fetch(req).await;
        }
        let key = req.url.as_str();
        let cached = self.cache.cache_get(&key);

        // Frais → sert sans réseau.
        if let Some(e) = &cached {
            if e.fresh_until.is_some_and(|t| SystemTime::now() < t) {
                return Ok(Response {
                    status: e.status,
                    reason: e.reason.clone(),
                    headers: e.headers.clone(),
                    body: e.body.clone(),
                    reused_connection: false,
                    resolved_url: None,
                });
            }
        }

        // Périmé → revalidation conditionnelle si validateurs.
        let mut req = req.clone();
        let mut has_validator = false;
        if let Some(e) = &cached {
            if let Some(et) = &e.etag {
                req.headers.push(("If-None-Match".to_string(), et.clone()));
                has_validator = true;
            }
            if let Some(lm) = &e.last_modified {
                req.headers
                    .push(("If-Modified-Since".to_string(), lm.clone()));
                has_validator = true;
            }
        }

        let resp = self.inner.fetch(&req).await?;

        // 304 → sert l'entrée cachée, met à jour les en-têtes.
        if resp.status == 304 {
            if let Some(mut e) = cached {
                if has_validator {
                    for (n, v) in &resp.headers {
                        if !matches!(
                            n.to_ascii_lowercase().as_str(),
                            // Corps stocké décodé : la taille filaire
                            // et l'encodage filaire du 304 ne
                            // correspondent pas à ce que nous avons.
                            "content-length" | "transfer-encoding" | "content-encoding"
                        ) {
                            e.headers.retain(|(en, _)| !en.eq_ignore_ascii_case(n));
                            e.headers.push((n.clone(), v.clone()));
                        }
                    }
                    e.fresh_until = freshness(&e.headers, e.stored_at);
                    self.cache.cache_put(&key, e.clone());
                    return Ok(Response {
                        status: e.status,
                        reason: e.reason.clone(),
                        headers: e.headers,
                        body: e.body,
                        reused_connection: resp.reused_connection,
                        resolved_url: None,
                    });
                }
            }
            return Ok(resp);
        }

        // Stocke si cachable.
        if cacheable_status(resp.status)
            && resp.body.len() <= self.cfg.max_entry_bytes
            && !resp
                .header("cache-control")
                .is_some_and(|v| v.to_ascii_lowercase().contains("no-store"))
            && resp.header("vary") != Some("*")
        {
            let entry = CacheEntry {
                status: resp.status,
                reason: resp.reason.clone(),
                headers: resp.headers.clone(),
                body: resp.body.clone(),
                stored_at: SystemTime::now(),
                fresh_until: freshness(&resp.headers, SystemTime::now()),
                etag: resp.header("etag").map(str::to_string),
                last_modified: resp.header("last-modified").map(str::to_string),
            };
            self.cache.cache_put(&key, entry);
        }
        Ok(resp)
    }

    /// Streamé : court-circuit franc (étape 101) — le corps en
    /// vol n'est pas tamponné pour le cache (le store travaille
    /// sur corps entiers) ; la navigation streamée ne lit ni
    /// n'écrit le cache. Conforme à l'intention documentée —
    /// avant, le repli `fetch` bufferisait le corps complet
    /// derrière l'interface streamée.
    async fn fetch_stream(&self, req: &Request) -> NetResult<crate::http::StreamedResponse> {
        self.inner.fetch_stream(req).await
    }
}

/// Fraîcheur : `max-age` en priorité, sinon `Expires:` absolu.
fn freshness(headers: &[(String, String)], at: SystemTime) -> Option<SystemTime> {
    if let Some(cc) = hdr(headers, "cache-control") {
        for d in cc.split(',') {
            let d = d.trim();
            if let Some(v) = d.strip_prefix("max-age=") {
                if let Ok(secs) = v.trim_matches('"').trim().parse::<u64>() {
                    return Some(at + Duration::from_secs(secs));
                }
            }
        }
    }
    if let Some(e) = hdr(headers, "expires") {
        return crate::cookies::parse_http_date(e);
    }
    None
}
