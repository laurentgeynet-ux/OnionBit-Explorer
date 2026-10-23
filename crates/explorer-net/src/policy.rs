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
use crate::http::{
    CorsRequest, FetchCredentials, FetchDest, FetchMode, HttpTransport, Initiator, Method,
    ReferrerPolicy, Request, Response, StreamedResponse,
};
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
        let orig = req.url.clone();
        let mut req = req.clone();
        for _ in 0..self.max_redirects {
            let mut resp = self.inner.fetch(&req).await?;
            if !matches!(resp.status, 301 | 302 | 303 | 307 | 308) {
                // Cible finale de la chaîne — `None` si la
                // première réponse n'était pas un 3xx (URL
                // de requête inchangée — étape 67).
                resp.resolved_url = (req.url != orig).then(|| req.url.clone());
                return Ok(resp);
            }
            let Some(loc) = resp.header("location") else {
                resp.resolved_url = (req.url != orig).then(|| req.url.clone());
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

    /// Variante streamée (étape 101) : chaque hop est suivi en
    /// flux — les en-têtes de la cible finale sont rendus dès
    /// réception, sans attendre le corps du hop ni celui de la
    /// réponse terminale. Indispensable pour la navigation :
    /// `NavJob` décide sur le statut (403 → page d'erreur
    /// immédiate) — le repli `fetch` bufferisé retenait le
    /// statut derrière le corps complet (~60 s mesuré terrain
    /// sur fnac.fr : corps anti-bot au goutte-à-goutte,
    /// `io_timeout` par maillon). Le corps d'un hop 3xx n'est
    /// pas drainé : sa tâche de corps meurt avec le canal
    /// (connexion sacrifiée, délai nul — le web réel sert des
    /// corps de redirection parfois sans fin propre).
    async fn fetch_stream(&self, req: &Request) -> NetResult<StreamedResponse> {
        let orig = req.url.clone();
        let mut req = req.clone();
        for _ in 0..self.max_redirects {
            let mut resp = self.inner.fetch_stream(&req).await?;
            if !matches!(resp.status, 301 | 302 | 303 | 307 | 308) {
                // Cible finale — `None` si la première réponse
                // n'était pas un 3xx (URL inchangée).
                resp.resolved_url = (req.url != orig).then(|| req.url.clone());
                return Ok(resp);
            }
            let status = resp.status;
            let Some(loc) = resp.header("location").map(str::to_string) else {
                resp.resolved_url = (req.url != orig).then(|| req.url.clone());
                return Ok(resp); // 3xx sans Location → tel quel.
            };
            let next = Url::resolve(&req.url, &loc)
                .map_err(|e| NetError::InvalidResponse(format!("Location invalide: {e}")))?;
            drop(resp); // corps du hop abandonné — pas d'attente.
            req.method = redirect_method(req.method, status);
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

    /// Streamé (étape 101) : upgrade `http:` → `https:` à
    /// l'émission, capture `Strict-Transport-Security` depuis
    /// les en-têtes streamés.
    async fn fetch_stream(&self, req: &Request) -> NetResult<StreamedResponse> {
        let mut req = req.clone();
        if req.url.scheme() == "http" && self.store.should_upgrade(&req.url.host()) {
            let mut u = req.url.as_str();
            u = u.replacen("http://", "https://", 1);
            if let Ok(upgraded) = Url::parse(&u) {
                req.url = upgraded;
            }
        }
        let resp = self.inner.fetch_stream(&req).await?;
        if req.url.scheme() == "https" {
            for (n, v) in &resp.headers {
                if n.eq_ignore_ascii_case("strict-transport-security") {
                    self.store.record(&req.url.host(), v);
                }
            }
        }
        Ok(resp)
    }
}

// ---------------------------------------------------------------------
// Referer / Origin
// ---------------------------------------------------------------------

/// Transport décorateur : calcule `Referer` et `Origin` par requête
/// sous `req.referrer_policy` — défaut
/// *strict-origin-when-cross-origin* (étape 132).
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

/// `Referer:` calculé sous la `policy` de la requête
/// (étape 132 — spec *Referrer-Policy* § « determine
/// request's referrer »). « URL complète » = URL sans
/// fragment ; « origine » = sérialisation `scheme://host
/// [:port]/` ; « rétrogradation » = initiateur `https` →
/// cible `http`.
#[must_use]
pub fn referer_value_for(initiator: &Url, target: &Url, policy: ReferrerPolicy) -> Option<String> {
    let downgrade = initiator.scheme() == "https" && target.scheme() == "http";
    let same_origin = initiator.origin() == target.origin();
    let full = || {
        let mut s = initiator.as_str();
        if let Some(i) = s.find('#') {
            s.truncate(i);
        }
        s
    };
    let origin = || match initiator.origin() {
        crate::url::Origin::Tuple { scheme, host, port } => Some(match (scheme.as_str(), port) {
            ("http", 80) | ("https", 443) => format!("{scheme}://{host}/"),
            _ => format!("{scheme}://{host}:{port}/"),
        }),
        crate::url::Origin::Opaque => None,
    };
    match policy {
        ReferrerPolicy::NoReferrer => None,
        ReferrerPolicy::NoReferrerWhenDowngrade => (!downgrade).then(full),
        ReferrerPolicy::SameOrigin => same_origin.then(full),
        ReferrerPolicy::Origin => origin(),
        ReferrerPolicy::StrictOrigin => (!downgrade).then(origin).flatten(),
        ReferrerPolicy::OriginWhenCrossOrigin => {
            if same_origin {
                Some(full())
            } else {
                origin()
            }
        }
        ReferrerPolicy::StrictOriginWhenCrossOrigin => {
            if downgrade {
                None
            } else if same_origin {
                Some(full())
            } else {
                origin()
            }
        }
        ReferrerPolicy::UnsafeUrl => Some(full()),
    }
}

/// `Origin:` calculé — toute méthode non-safe (spec Fetch :
/// ni `GET` ni `HEAD`, origine same-origin **incluse** —
/// étape 133 ; les endpoints CSRF/auth l'exigent). Origine
/// opaque sérialisée `null` (spec).
fn origin_value(initiator: &Url, _target: &Url, method: Method) -> Option<String> {
    if matches!(method, Method::Get | Method::Head | Method::Options) {
        return None;
    }
    Some(referer_value_origin(initiator).unwrap_or_else(|| "null".to_string()))
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

/// En-têtes `Referer`/`Origin` d'une sous-requête émise depuis
/// `initiator` — même politique *strict-origin-when-cross-origin*
/// que [`RefererTransport`], calculée **à l'émission** par le
/// scheduler de sous-ressources (étape 50) qui ne passe pas par le
/// décorateur. `GET` n'émet jamais `Origin` (méthode safe).
#[must_use]
pub fn initiator_headers(initiator: &Url, target: &Url, method: Method) -> Vec<(String, String)> {
    initiator_headers_policy(initiator, target, method, ReferrerPolicy::default())
}

/// Variante d'[`initiator_headers`] sous une `policy`
/// explicite (étape 132 — `Referrer-Policy:` de la page /
/// `referrerpolicy=` de l'élément). `Origin:` n'est pas
/// gouverné par la politique (pas une info de référent).
#[must_use]
pub fn initiator_headers_policy(
    initiator: &Url,
    target: &Url,
    method: Method,
    policy: ReferrerPolicy,
) -> Vec<(String, String)> {
    let mut out = Vec::new();
    if let Some(r) = referer_value_for(initiator, target, policy) {
        out.push(("referer".to_string(), r));
    }
    if let Some(o) = origin_value(initiator, target, method) {
        out.push(("origin".to_string(), o));
    }
    out
}

/// En-têtes `Sec-Fetch-*` d'une requête (étape 125, *Fetch
/// Metadata Request Headers*) — calculés **à l'émission** par
/// l'appelant, même patron que [`initiator_headers`] : la
/// plateforme les contrôle, le contenu ne peut pas les poser
/// (`sec-*` est un nom interdit côté script).
///
/// - `Sec-Fetch-Site` : `none` sans initiateur de page ;
///   `same-origin`/`same-site`/`cross-site` sinon — « site »
///   = `(schéma, domaine registrable)`, même calcul que la
///   politique `SameSite` des cookies. `TopLevelNav` rend
///   `none` (pas de site initiateur : saisie, favori).
/// - `Sec-Fetch-Mode` : `navigate` pour les initiateurs de
///   navigation, sinon `req.fetch_mode` (`fetch()`/`XHR`
///   portent `cors`, les sous-ressources `no-cors`).
/// - `Sec-Fetch-Dest` : `document` pour les navigations,
///   sinon `req.fetch_dest`.
/// - `Sec-Fetch-User` : `?1` uniquement sur une navigation
///   marquée `req.fetch_user` (geste utilisateur réel, posé
///   au dispatch — jamais deviné).
///
/// Rien n'est émis hors `http`/`https` (`data:`/`about:`/
/// `file:` ne portent pas de fetch metadata).
#[must_use]
pub fn sec_fetch_headers(req: &Request) -> Vec<(String, String)> {
    if !matches!(req.url.scheme(), "http" | "https") {
        return Vec::new();
    }
    let nav = matches!(
        req.initiator,
        Some(Initiator::TopLevelNav | Initiator::NavFrom(_))
    );
    let site = match req.initiator.as_ref() {
        None | Some(Initiator::TopLevelNav) => "none",
        Some(Initiator::NavFrom(page) | Initiator::Subresource(page)) => fetch_site(page, &req.url),
    };
    let mode = if nav {
        "navigate"
    } else {
        match req.fetch_mode {
            FetchMode::Cors => "cors",
            FetchMode::NoCors => "no-cors",
            FetchMode::SameOrigin => "same-origin",
            FetchMode::Navigate => "navigate",
        }
    };
    let dest = if nav {
        "document"
    } else {
        match req.fetch_dest {
            FetchDest::Document | FetchDest::Empty => "empty",
            FetchDest::Script => "script",
            FetchDest::Style => "style",
            FetchDest::Image => "image",
            FetchDest::Font => "font",
            FetchDest::Iframe => "iframe",
        }
    };
    let mut out = vec![
        ("sec-fetch-site".to_string(), site.to_string()),
        ("sec-fetch-mode".to_string(), mode.to_string()),
        ("sec-fetch-dest".to_string(), dest.to_string()),
    ];
    if nav && req.fetch_user {
        out.push(("sec-fetch-user".to_string(), "?1".to_string()));
    }
    out
}

/// `Sec-Fetch-Site` d'une sous-requête émise depuis `page` :
/// `same-origin` si origines égales, `same-site` si même
/// `(schéma, registrable)`, `cross-site` sinon.
fn fetch_site(page: &Url, target: &Url) -> &'static str {
    if page.origin() == target.origin() {
        "same-origin"
    } else if page.scheme() == target.scheme()
        && crate::cookies::registrable(&page.host()) == crate::cookies::registrable(&target.host())
    {
        "same-site"
    } else {
        "cross-site"
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
            if !req.no_referer
                && !req
                    .headers
                    .iter()
                    .any(|(n, _)| n.eq_ignore_ascii_case("referer"))
            {
                let policy = req.referrer_policy.unwrap_or_default();
                if let Some(r) = referer_value_for(&i, &req.url, policy) {
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

    /// Streamé (étape 101) : mêmes en-têtes `Referer`/`Origin`
    /// injectés, passage en flux.
    async fn fetch_stream(&self, req: &Request) -> NetResult<StreamedResponse> {
        let mut req = req.clone();
        let init = self
            .initiator
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        if let Some(i) = init {
            if !req.no_referer
                && !req
                    .headers
                    .iter()
                    .any(|(n, _)| n.eq_ignore_ascii_case("referer"))
            {
                let policy = req.referrer_policy.unwrap_or_default();
                if let Some(r) = referer_value_for(&i, &req.url, policy) {
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
        self.inner.fetch_stream(&req).await
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

    /// Streamé (étape 101) : même refus mixed-content à
    /// l'émission, passage en flux.
    async fn fetch_stream(&self, req: &Request) -> NetResult<StreamedResponse> {
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
        self.inner.fetch_stream(req).await
    }
}

// ---------------------------------------------------------------------
// CorsTransport — enforcement CORS (étape 126)
// ---------------------------------------------------------------------

/// Transport décorateur : enforce le protocole CORS sur les
/// requêtes `fetch_mode == Cors` portant `req.cors` (étape
/// 126, spec Fetch « HTTP-network-or-cache fetch ») — toute
/// autre requête passe sans examen.
///
/// - **Preflight** : méthode hors `{GET, HEAD, POST}`,
///   `cors.unsafe_headers` non vide ou `Content-Type` hors
///   des trois types « form » (`application/x-www-form-
///   urlencoded`/`multipart/form-data`/`text/plain`) →
///   `OPTIONS` préalable : `Origin:` (origine de la page),
///   `Access-Control-Request-Method:`, `Access-Control-
///   Request-Headers:` quand non vide. Accepté si statut
///   2xx + `ACAO` (`*` admis seulement sans `include`,
///   sinon origine exacte) + `ACAM` couvrant la méthode +
///   `ACAH` couvrant chaque en-tête demandé ; `ACAC: true`
///   exigé quand `credentials == Include`.
/// - **Réponse finale** : lisible si `ACAO: *` (sans
///   `include`) ou `ACAO` = origine exacte ; `ACAC: true`
///   exigé sous `include` — sinon `NetError::CorsBlocked`
///   (l'échec remonte opaque : `status: 0` côté script,
///   comme une erreur réseau).
///
/// Pas de cache de preflight — spec : entrée `(origine, url,
/// credentials)` ; hors MVP : un `OPTIONS` de plus par
/// requête non simple. Cookies sortants/`Set-Cookie` restent
/// à la session (jar) — le transport ne voit que la requête.
pub struct CorsTransport {
    inner: Arc<dyn HttpTransport>,
}

impl CorsTransport {
    /// Enveloppe `inner`.
    #[must_use]
    pub fn wrap(inner: Arc<dyn HttpTransport>) -> Self {
        Self { inner }
    }

    /// Requête soumise au protocole : `mode:cors` + contexte
    /// + `http(s)`. `no-cors`/`navigate` passent.
    fn cors_of(req: &Request) -> Option<&CorsRequest> {
        if req.fetch_mode != FetchMode::Cors {
            return None;
        }
        if !matches!(req.url.scheme(), "http" | "https") {
            return None;
        }
        req.cors.as_ref()
    }

    /// Un `OPTIONS` est-il exigé ? Méthode non safelistée,
    /// en-tête auteur non safelisté, ou `Content-Type` hors
    /// des trois types « form » (spec Fetch §3.2.5).
    fn needs_preflight(req: &Request, cors: &CorsRequest) -> bool {
        if !cors_simple_method(req.method) {
            return true;
        }
        if !cors.unsafe_headers.is_empty() {
            return true;
        }
        req.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case("content-type"))
            .is_some_and(|(_, v)| {
                !cors_safelisted_content_type(v.split(';').next().unwrap_or("").trim())
            })
    }

    /// Requête `OPTIONS` de preflight (spec Fetch §3.2.7) :
    /// sans corps ni credentials, `Origin` + `ACRM` (+ `ACRH`
    /// si des en-têtes auteur non safelistés existent). Le
    /// contexte `cors` n'est pas propagé — le preflight n'est
    /// pas lui-même vérifié par CORS.
    fn preflight(req: &Request, cors: &CorsRequest) -> Request {
        let mut p = Request {
            method: Method::Options,
            url: req.url.clone(),
            headers: vec![
                ("origin".to_string(), cors.origin.clone()),
                (
                    "access-control-request-method".to_string(),
                    req.method.as_str().to_string(),
                ),
            ],
            body: None,
            initiator: req.initiator.clone(),
            fetch_dest: req.fetch_dest,
            fetch_mode: FetchMode::Cors,
            fetch_user: false,
            cors: None,
            // Le preflight suit la politique `Referer` de la
            // requête d'origine (Fetch : même referrer
            // policy sur l'`OPTIONS`).
            no_referer: req.no_referer,
            referrer_policy: req.referrer_policy,
        };
        if !cors.unsafe_headers.is_empty() {
            p.headers.push((
                "access-control-request-headers".to_string(),
                cors.unsafe_headers.join(", "),
            ));
        }
        p.headers.extend(sec_fetch_headers(&p));
        p
    }

    /// `ACAO`/`ACAC` sur une réponse — `Some(raison)` si la
    /// politique refuse la lecture.
    fn acao_violation(cors: &CorsRequest, resp: &Response) -> Option<String> {
        let include = cors.credentials == FetchCredentials::Include;
        let acao_ok = resp.headers_all("access-control-allow-origin").any(|v| {
            let v = v.trim();
            (v == "*" && !include) || v == cors.origin
        });
        if !acao_ok {
            return Some(format!(
                "Access-Control-Allow-Origin absent ou ≠ {}",
                cors.origin
            ));
        }
        if include
            && !resp
                .headers_all("access-control-allow-credentials")
                .any(|v| v.trim().eq_ignore_ascii_case("true"))
        {
            return Some(
                "Access-Control-Allow-Credentials requis (credentials=include)".to_string(),
            );
        }
        None
    }

    /// La réponse `OPTIONS` autorise-t-elle `(méthode,
    /// en-têtes)` ? `ACAM`/`ACAH` = listes de jetons
    /// insensibles à la casse ; `*` admis seulement sans
    /// `include` (spec : `*` ne couvre pas les credentials).
    fn preflight_violation(req: &Request, cors: &CorsRequest, resp: &Response) -> Option<String> {
        if !(200..300).contains(&resp.status) {
            return Some(format!("preflight OPTIONS → statut {}", resp.status));
        }
        if let Some(why) = Self::acao_violation(cors, resp) {
            return Some(format!("preflight : {why}"));
        }
        let include = cors.credentials == FetchCredentials::Include;
        let method = req.method.as_str();
        let method_ok = resp
            .headers_all("access-control-allow-methods")
            .flat_map(|v| v.split(','))
            .any(|t| {
                let t = t.trim();
                t.eq_ignore_ascii_case(method) || (t == "*" && !include)
            });
        if !method_ok {
            return Some(format!("preflight : méthode {method} non autorisée"));
        }
        for h in &cors.unsafe_headers {
            let ok = resp
                .headers_all("access-control-allow-headers")
                .flat_map(|v| v.split(','))
                .any(|t| {
                    let t = t.trim();
                    t.eq_ignore_ascii_case(h) || (t == "*" && !include)
                });
            if !ok {
                return Some(format!("preflight : en-tête {h} non autorisé"));
            }
        }
        None
    }
}

/// Verbe CORS-safelisté (`GET`/`HEAD`/`POST`) — tout autre
/// déclenche le preflight (spec Fetch §3.2.2).
const fn cors_simple_method(m: Method) -> bool {
    matches!(m, Method::Get | Method::Head | Method::Post)
}

/// `Content-Type` CORS-safelisté (spec : les trois types
/// `application/x-www-form-urlencoded`, `multipart/form-data`,
/// `text/plain` — comparé sans paramètres, insensible à la
/// casse). `pub` : la session compose
/// `CorsRequest::unsafe_headers` avec le même filtre.
#[must_use]
pub fn cors_safelisted_content_type(v: &str) -> bool {
    matches!(
        v.to_ascii_lowercase().as_str(),
        "application/x-www-form-urlencoded" | "multipart/form-data" | "text/plain"
    )
}

#[async_trait]
impl HttpTransport for CorsTransport {
    async fn fetch(&self, req: &Request) -> NetResult<Response> {
        let Some(cors) = Self::cors_of(req) else {
            return self.inner.fetch(req).await;
        };
        if Self::needs_preflight(req, cors) {
            let pre = self.inner.fetch(&Self::preflight(req, cors)).await?;
            if let Some(why) = Self::preflight_violation(req, cors, &pre) {
                return Err(NetError::CorsBlocked(why));
            }
        }
        let resp = self.inner.fetch(req).await?;
        if let Some(why) = Self::acao_violation(cors, &resp) {
            return Err(NetError::CorsBlocked(why));
        }
        Ok(resp)
    }

    /// Streamé : délégation directe — les requêtes streamées
    /// (documents) ne sont jamais `mode:cors` ; sans override
    /// le défaut du trait bufferiserait via `fetch` et tuerait
    /// le rendu progressif.
    async fn fetch_stream(&self, req: &Request) -> NetResult<StreamedResponse> {
        self.inner.fetch_stream(req).await
    }
}

// ---------------------------------------------------------------------
// En-têtes de sécurité de réponse (étape 134)
// ---------------------------------------------------------------------

/// MIMEs JavaScript (table WHATWG « JavaScript MIME type » —
/// utilisée par `nosniff` et module scripts).
fn is_js_mime(mime: &str) -> bool {
    matches!(
        mime,
        "application/ecmascript"
            | "application/javascript"
            | "application/x-ecmascript"
            | "application/x-javascript"
            | "text/ecmascript"
            | "text/javascript"
            | "text/javascript1.0"
            | "text/javascript1.1"
            | "text/javascript1.2"
            | "text/javascript1.3"
            | "text/javascript1.4"
            | "text/javascript1.5"
            | "text/jscript"
            | "text/livescript"
            | "text/x-ecmascript"
            | "text/x-javascript"
    )
}

/// `X-Content-Type-Options: nosniff` (étape 134, spec MIME
/// sniffing §5) — la réponse déclare honorer son
/// `Content-Type` : pour une destination `script` la MIME
/// doit être JavaScript, pour `style` `text/css` ; tout autre
/// destination est inchangée (images, fonts…). `true` =
/// admissible. `Content-Type` absent ou invalide sous
/// `nosniff` = échec de MIME (bloqué, spec).
#[must_use]
pub fn nosniff_allows(dest: FetchDest, resp: &Response) -> bool {
    if !resp.nosniff() {
        return true;
    }
    if !matches!(dest, FetchDest::Script | FetchDest::Style) {
        return true;
    }
    let Some(ct) = resp.content_type() else {
        return false;
    };
    let mime = ct
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    match dest {
        FetchDest::Script => is_js_mime(&mime),
        FetchDest::Style => mime == "text/css",
        _ => true,
    }
}

/// `Cross-Origin-Resource-Policy` (étape 134, Fetch §4.7) —
/// sur une sous-ressource `no-cors` : `same-origin` n'admet
/// que l'origine de la page, `same-site` son site
/// (`schéma://registrable`, même notion que `SameSite`) ;
/// `cross-origin`, en-tête absent ou valeur inconnue →
/// admis. `page`/`resp_url` : URL de la page initiatrice et
/// de la ressource. `true` = admissible.
#[must_use]
pub fn corp_allows(page: &Url, resp_url: &Url, resp: &Response) -> bool {
    let Some(v) = resp.header("cross-origin-resource-policy") else {
        return true;
    };
    match v.trim().to_ascii_lowercase().as_str() {
        "same-origin" => page.origin() == resp_url.origin(),
        "same-site" => {
            let a = crate::cookies::partition_key(page);
            let b = crate::cookies::partition_key(resp_url);
            a.is_some() && a == b
        }
        _ => true,
    }
}

/// Peut-on embarquer `resp` en sous-contexte sous `page`
/// (étape 134) — `X-Frame-Options` + CSP `frame-ancestors`
/// sur la réponse de la sous-navigation (`<iframe src>`,
/// `FetchDest::Iframe`). CSP `frame-ancestors` l'emporte dès
/// qu'une directive existe (CSP §frame-ancestors : XFO
/// ignoré en sa présence) ; chaque en-tête `Content-Security-
/// Policy` doit autoriser (un CSP plus restrictif réduit).
/// XFO : `deny` → jamais ; `sameorigin` → origine de la page
/// = origine de la ressource ; `allow-from` obsolète ignoré
/// (navigateurs modernes le passent sous silence) ; valeurs
/// inconnues → pas de restriction. Sans en-tête → admis.
#[must_use]
pub fn frame_embedding_allowed(page: &Url, resp_url: &Url, resp: &Response) -> bool {
    let mut csp_seen = false;
    for csp in resp.headers_all("content-security-policy") {
        for directive in csp.split(';') {
            let mut tokens = directive.split_whitespace();
            let Some(name) = tokens.next() else { continue };
            if !name.eq_ignore_ascii_case("frame-ancestors") {
                continue;
            }
            csp_seen = true;
            // Aucune source ne matche → la directive refuse.
            if !tokens.any(|src| fa_source_matches(src, page, resp_url)) {
                return false;
            }
        }
    }
    if csp_seen {
        return true;
    }
    for v in resp.headers_all("x-frame-options") {
        match v.trim().to_ascii_lowercase().as_str() {
            "deny" => return false,
            "sameorigin" if page.origin() != resp_url.origin() => return false,
            _ => {}
        }
    }
    true
}

/// Une source `frame-ancestors` matche-t-elle `page` —
/// ancêtre candidat (CSP §6.6.2.10 « matching a source
/// expression »), sous-ensemble borné : `'none'` (jamais),
/// `*` (tout hôte http(s)), `'self'` (origine de la
/// **ressource** embarquée), scheme-source `https:`,
/// host-source `[scheme://][*.]host[:port]` (le `*` de tête
/// admet le hôte et ses sous-domaines ; chemin ignoré —
/// la spec ne conserve que l'hôte). Les sources `data:`,
/// `'unsafe-*'`, nonce/hash n'ont pas de sens ici.
fn fa_source_matches(src: &str, page: &Url, resp_url: &Url) -> bool {
    let s = src.trim().to_ascii_lowercase();
    if s == "*" {
        return matches!(page.scheme(), "http" | "https");
    }
    if s == "'self'" {
        return page.origin() == resp_url.origin();
    }
    if s == "'none'" || s.is_empty() {
        return false;
    }
    // Scheme-source : `https:`/`http:` (sans `//`).
    if let Some(scheme) = s.strip_suffix(':') {
        if !scheme.is_empty()
            && scheme
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
            && !scheme.contains('.')
        {
            return page.scheme() == scheme;
        }
    }
    // Host-source : `[scheme://][*.]host[:port][/path…]`.
    let (scheme, rest) = match s.split_once("://") {
        Some((sch, rest)) => (Some(sch), rest),
        None => (None, s.as_str()),
    };
    let host_port = rest.split('/').next().unwrap_or("");
    let (host_pat, port_pat) = match host_port.split_once(':') {
        Some((h, p)) => (h, Some(p)),
        None => (host_port, None),
    };
    let page_host = page.host();
    let host_ok = if let Some(suffix) = host_pat.strip_prefix("*.") {
        page_host == suffix || page_host.ends_with(&format!(".{suffix}"))
    } else {
        page_host == host_pat
    };
    if !host_ok {
        return false;
    }
    if let Some(sch) = scheme {
        if page.scheme() != sch {
            return false;
        }
    }
    if let Some(p) = port_pat {
        let Ok(want) = p.parse::<u16>() else {
            return false;
        };
        if page.effective_port() != Some(want) {
            return false;
        }
    }
    true
}

/// Valeur `Accept` par destination (Fetch §4.4, CAP-141)
/// — posée quand l'émetteur n'a pas fourni la sienne. Les
/// tables WHATWG/Chrome annoncent aussi `avif`/`webp` :
/// le moteur ne les décode pas — la liste reste **honnête**
/// (annonce = formats réellement peints), sinon les CDN à
/// négociation de contenu serviraient des octets inertes.
#[must_use]
pub fn accept_value(dest: FetchDest) -> &'static str {
    match dest {
        FetchDest::Document | FetchDest::Iframe => {
            "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8"
        }
        FetchDest::Image => "image/png,image/svg+xml,image/*;q=0.8,*/*;q=0.5",
        FetchDest::Style => "text/css,*/*;q=0.1",
        FetchDest::Script | FetchDest::Font | FetchDest::Empty => "*/*",
    }
}

/// `Accept` par destination (CAP-141) — un navigateur
/// négocie le contenu par type de ressource : serveurs/CDN
/// réels répondent différemment (ou refusent) sans `Accept`
/// adapté. Posé **seulement si absent** : un `Accept` auteur
/// (`fetch(url, {headers:{Accept:…}})`, callers explicites)
/// n'est jamais écrasé. Les requêtes non-http(s) ne reçoivent
/// rien (jamais de socket — destructeur amont).
pub struct AcceptTransport {
    inner: Arc<dyn HttpTransport>,
}

impl AcceptTransport {
    /// Enveloppe `inner`.
    #[must_use]
    pub fn wrap(inner: Arc<dyn HttpTransport>) -> Self {
        Self { inner }
    }

    /// Stamp idempotent — n'écrase jamais un `Accept` posé
    /// en amont (auteur ou transport plus externe).
    fn stamp(req: &Request) -> Request {
        if !matches!(req.url.scheme(), "http" | "https")
            || req
                .headers
                .iter()
                .any(|(n, _)| n.eq_ignore_ascii_case("accept"))
        {
            return req.clone();
        }
        let mut req = req.clone();
        req.headers.push((
            "Accept".to_string(),
            accept_value(req.fetch_dest).to_string(),
        ));
        req
    }
}

#[async_trait]
impl HttpTransport for AcceptTransport {
    async fn fetch(&self, req: &Request) -> NetResult<Response> {
        self.inner.fetch(&Self::stamp(req)).await
    }

    /// Streamé : même stamp puis délégation — sans override le
    /// défaut du trait bufferiserait via `fetch` (voir
    /// `CorsTransport::fetch_stream`, étape 126).
    async fn fetch_stream(&self, req: &Request) -> NetResult<StreamedResponse> {
        self.inner.fetch_stream(&Self::stamp(req)).await
    }
}
