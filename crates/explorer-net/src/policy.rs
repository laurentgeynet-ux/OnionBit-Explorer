// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Politique réseau (étape 8, ADR-0002).
//!
//! Décorateurs composables sur [`HttpTransport`] :
//!
//! - [`RedirectTransport`] : suivi 3xx borné (`301/302/303/307/308`),
//!   réécriture de méthode conforme (303 → GET sauf HEAD ;
//!   301/302 POST → GET, pratique navigateur ; 307/308 préservent
//!   méthode+corps), `Location` résolu contre l'URL courante ;
//! - [`HstsStore`] + [`HstsTransport`] : RFC 6797 — upgrade
//!   `http→https` avant le fil (jamais de première requête en clair
//!   sur un hôte connu), capture `Strict-Transport-Security`
//!   (max-age, `includeSubDomains`), store borné ;
//! - [`RefererTransport`] : `Referer`/`Origin` calculés par requête
//!   — politique *strict-origin-when-cross-origin* (défaut
//!   navigateur) : URL complète same-origin, origine seule
//!   cross-origin, rien en rétrogradation https→http ; `Origin` sur
//!   les requêtes non-safe cross-origin (POST…) ;
//! - [`PolicyTransport`] : blocage **mixed-content** — sous-
//!   ressource `http:` d'une page `https:` refusée (localhost
//!   toléré), `NetError::BlockedMixedContent`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use async_trait::async_trait;

use crate::error::{NetError, NetResult};
use crate::http::{HttpTransport, Method, Request, Response};
use crate::url::Url;

// ---------------------------------------------------------------------
// RedirectTransport
// ---------------------------------------------------------------------

/// Suivi 3xx borné.
pub struct RedirectTransport {
    inner: Arc<dyn HttpTransport>,
    /// Nombre max de redirections suivies (boucle → erreur).
    pub max_redirects: usize,
}

impl RedirectTransport {
    /// Enveloppe `inner` avec `max_redirects`.
    #[must_use]
    pub fn wrap(inner: Arc<dyn HttpTransport>, max_redirects: usize) -> Self {
        Self {
            inner,
            max_redirects,
        }
    }
}

/// La méthode survit-elle à la redirection ? Renvoie la nouvelle
/// méthode, `None` = conservée. RFC 9110 §15.4 + pratique navigateur.
fn redirect_method(method: Method, status: u16) -> Method {
    match (status, method) {
        // 303 See Other : tout devient GET sauf HEAD.
        (303, Method::Head) => Method::Head,
        // 303 → GET ; 301/302 POST → GET (pratique navigateur
        // héritée, pas la lettre de la spec). 307/308 : préservés.
        (303, _) | (301 | 302, Method::Post) => Method::Get,
        // 307/308 : méthode et corps préservés intégralement.
        _ => method,
    }
}

/// Méthode à corps — la réécriture vers GET droppe le corps.
fn method_has_body(m: Method) -> bool {
    matches!(m, Method::Post | Method::Put)
}

#[async_trait]
impl HttpTransport for RedirectTransport {
    async fn fetch(&self, req: &Request) -> NetResult<Response> {
        let mut req = req.clone();
        for _ in 0..self.max_redirects {
            let resp = self.inner.fetch(&req).await?;
            if !matches!(resp.status, 301 | 302 | 303 | 307 | 308) {
                return Ok(resp);
            }
            let Some(loc) = resp.header("location") else {
                return Ok(resp); // 3xx sans Location → tel quel.
            };
            let next = Url::resolve(&req.url, loc)
                .map_err(|e| NetError::InvalidResponse(format!("Location invalide: {e}")))?;
            req.method = redirect_method(req.method, resp.status);
            if req.method == Method::Get && method_has_body(Request::get(next.clone()).method) {
                // unreachable — garde-fou si la table grandit.
            }
            if req.method == Method::Get {
                req.body = None;
            }
            req.url = next;
        }
        Err(NetError::TooManyRedirects(self.max_redirects))
    }
}

// ---------------------------------------------------------------------
// HSTS — RFC 6797
// ---------------------------------------------------------------------

/// Entrée du store HSTS.
#[derive(Debug, Clone)]
struct HstsEntry {
    expire_at: SystemTime,
    include_subdomains: bool,
}

/// Store HSTS mémoire borné — `max_entries`, éviction des expirés
/// puis FIFO approximatif.
pub struct HstsStore {
    map: Mutex<HashMap<String, HstsEntry>>,
    /// Entrées max (le store est mémoire — borné par politique).
    pub max_entries: usize,
}

impl HstsStore {
    /// Store vide.
    #[must_use]
    pub fn new(max_entries: usize) -> Arc<Self> {
        Arc::new(Self {
            map: Mutex::new(HashMap::new()),
            max_entries,
        })
    }

    /// L'hôte doit-il être passé en `https:` ? Match exact ou
    /// sous-domaine d'une entrée `includeSubDomains`.
    #[must_use]
    pub fn should_upgrade(&self, host: &str) -> bool {
        self.should_upgrade_at(host, SystemTime::now())
    }

    /// Variant datée de [`Self::should_upgrade`].
    #[must_use]
    pub fn should_upgrade_at(&self, host: &str, now: SystemTime) -> bool {
        let map = self
            .map
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(e) = map.get(host) {
            return e.expire_at > now;
        }
        // includeSubDomains : remonte la chaîne de suffixes.
        let mut h = host;
        while let Some(i) = h.find('.') {
            h = &h[i + 1..];
            if let Some(e) = map.get(h) {
                return e.include_subdomains && e.expire_at > now;
            }
        }
        false
    }

    /// Enregistre un en-tête `Strict-Transport-Security` capturé
    /// sur une réponse `https:` pour `host`.
    pub fn record(&self, host: &str, header: &str) {
        self.record_at(host, header, SystemTime::now());
    }

    /// Variant datée de [`Self::record`].
    pub fn record_at(&self, host: &str, header: &str, now: SystemTime) {
        let mut max_age: Option<u64> = None;
        let mut include_sub = false;
        for d in header.split(';') {
            let d = d.trim();
            if let Some(v) = d.to_ascii_lowercase().strip_prefix("max-age=") {
                max_age = v.trim().trim_matches('"').parse().ok();
            } else if d.eq_ignore_ascii_case("includeSubDomains") {
                include_sub = true;
            }
        }
        let Some(secs) = max_age else { return };
        let mut map = self
            .map
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if secs == 0 {
            map.remove(host); // max-age=0 = révocation.
            return;
        }
        map.insert(
            host.to_string(),
            HstsEntry {
                expire_at: now + Duration::from_secs(secs),
                include_subdomains: include_sub,
            },
        );
        while map.len() > self.max_entries {
            map.retain(|_, e| e.expire_at > now);
            if map.len() <= self.max_entries {
                break;
            }
            // FIFO approximatif : retire une clé quelconque.
            if let Some(k) = map.keys().next().cloned() {
                map.remove(&k);
            }
        }
    }
}

/// Transport décorateur HSTS : upgrade `http:` → `https:` sur les
/// hôtes connus avant d'émettre la requête ; capture l'en-tête sur
/// les réponses `https:`.
pub struct HstsTransport {
    inner: Arc<dyn HttpTransport>,
    store: Arc<HstsStore>,
}

impl HstsTransport {
    /// Enveloppe `inner` avec le `store` partagé.
    #[must_use]
    pub fn wrap(inner: Arc<dyn HttpTransport>, store: Arc<HstsStore>) -> Self {
        Self { inner, store }
    }
}

#[async_trait]
impl HttpTransport for HstsTransport {
    async fn fetch(&self, req: &Request) -> NetResult<Response> {
        let mut req = req.clone();
        if req.url.scheme() == "http" && self.store.should_upgrade(&req.url.host()) {
            let mut u = req.url.as_str();
            u = u.replacen("http://", "https://", 1);
            if let Ok(upgraded) = Url::parse(&u) {
                req.url = upgraded;
            }
        }
        let resp = self.inner.fetch(&req).await?;
        if req.url.scheme() == "https" {
            for v in resp.headers_all("strict-transport-security") {
                self.store.record(&req.url.host(), v);
            }
        }
        Ok(resp)
    }
}

// ---------------------------------------------------------------------
// Referer / Origin
// ---------------------------------------------------------------------

/// Transport décorateur : calcule `Referer` et `Origin` par requête
/// selon la politique *strict-origin-when-cross-origin*.
pub struct RefererTransport {
    inner: Arc<dyn HttpTransport>,
    /// URL de la page initiatrice — `None` = pas de Referer.
    initiator: Mutex<Option<Url>>,
}

impl RefererTransport {
    /// Enveloppe `inner` — `initiator` = URL de la page d'où
    /// partent les requêtes (changée par [`Self::set_initiator`]).
    #[must_use]
    pub fn wrap(inner: Arc<dyn HttpTransport>, initiator: Option<Url>) -> Self {
        Self {
            inner,
            initiator: Mutex::new(initiator),
        }
    }

    /// Change la page initiatrice (navigation).
    pub fn set_initiator(&self, url: Option<Url>) {
        *self
            .initiator
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = url;
    }
}

/// `Referer:` calculé — strict-origin-when-cross-origin.
fn referer_value(initiator: &Url, target: &Url) -> Option<String> {
    // Pas de rétrogradation : page https → cible http = rien.
    if initiator.scheme() == "https" && target.scheme() == "http" {
        return None;
    }
    // Même origine → URL complète (sans fragment ni userinfo).
    if initiator.origin() == target.origin() {
        let mut s = initiator.as_str();
        if let Some(i) = s.find('#') {
            s.truncate(i);
        }
        return Some(s);
    }
    // Cross-origin → origine seule.
    match initiator.origin() {
        crate::url::Origin::Tuple { scheme, host, port } => Some(match (scheme.as_str(), port) {
            ("http", 80) | ("https", 443) => format!("{scheme}://{host}/"),
            _ => format!("{scheme}://{host}:{port}/"),
        }),
        crate::url::Origin::Opaque => None,
    }
}

/// `Origin:` calculé — non-safe cross-origin seulement.
fn origin_value(initiator: &Url, target: &Url, method: Method) -> Option<String> {
    if matches!(method, Method::Get | Method::Head | Method::Options) {
        return None;
    }
    match initiator.origin() {
        crate::url::Origin::Tuple { .. } if initiator.origin() != target.origin() => {
            referer_value_origin(initiator)
        }
        _ => None,
    }
}

fn referer_value_origin(u: &Url) -> Option<String> {
    match u.origin() {
        crate::url::Origin::Tuple { scheme, host, port } => Some(match (scheme.as_str(), port) {
            ("http", 80) | ("https", 443) => format!("{scheme}://{host}"),
            _ => format!("{scheme}://{host}:{port}"),
        }),
        crate::url::Origin::Opaque => None,
    }
}

#[async_trait]
impl HttpTransport for RefererTransport {
    async fn fetch(&self, req: &Request) -> NetResult<Response> {
        let mut req = req.clone();
        let init = self
            .initiator
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        if let Some(i) = init {
            if !req
                .headers
                .iter()
                .any(|(n, _)| n.eq_ignore_ascii_case("referer"))
            {
                if let Some(r) = referer_value(&i, &req.url) {
                    req.headers.push(("Referer".to_string(), r));
                }
            }
            if !req
                .headers
                .iter()
                .any(|(n, _)| n.eq_ignore_ascii_case("origin"))
            {
                if let Some(o) = origin_value(&i, &req.url, req.method) {
                    req.headers.push(("Origin".to_string(), o));
                }
            }
        }
        self.inner.fetch(&req).await
    }
}

// ---------------------------------------------------------------------
// Mixed-content
// ---------------------------------------------------------------------

/// `true` si `resource` est du contenu mixte à bloquer pour `page` :
/// page `https:` chargeant une sous-ressource `http:`.
///
/// Exceptions : `localhost`/`127.0.0.1`/`[::1]` (dev), `data:`/
/// `about:`/`file:` (pas du HTTP).
#[must_use]
pub fn is_mixed_content(page: &Url, resource: &Url) -> bool {
    page.scheme() == "https"
        && resource.scheme() == "http"
        && !matches!(
            resource.host().as_str(),
            "localhost" | "127.0.0.1" | "[::1]"
        )
}

/// Transport décorateur : refuse les sous-requêtes `http:` d'une
/// page `https:` — [`NetError::BlockedMixedContent`] typée.
pub struct PolicyTransport {
    inner: Arc<dyn HttpTransport>,
    /// URL de la page — fixée à la navigation.
    page: Mutex<Option<Url>>,
}

impl PolicyTransport {
    /// Enveloppe `inner` — `page` = URL de la page active.
    #[must_use]
    pub fn wrap(inner: Arc<dyn HttpTransport>, page: Option<Url>) -> Self {
        Self {
            inner,
            page: Mutex::new(page),
        }
    }

    /// Change la page active (navigation).
    pub fn set_page(&self, url: Option<Url>) {
        *self
            .page
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = url;
    }
}

#[async_trait]
impl HttpTransport for PolicyTransport {
    async fn fetch(&self, req: &Request) -> NetResult<Response> {
        let page = self
            .page
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        if let Some(p) = page {
            if is_mixed_content(&p, &req.url) {
                return Err(NetError::BlockedMixedContent(format!(
                    "{} → {}",
                    p.as_str(),
                    req.url.as_str()
                )));
            }
        }
        self.inner.fetch(req).await
    }
}
