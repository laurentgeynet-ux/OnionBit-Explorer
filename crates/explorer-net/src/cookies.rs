// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Jar à cookies (étape 7, ADR-0002).
//!
//! Politique réelle dès le départ — RFC 6265 + durcissements web
//! réel :
//!
//! - `Domain=` validé par la **Public Suffix List** (`psl`) — sans
//!   elle, `Domain=co.uk` serait un supercookie ;
//! - `Secure` n'est posé/honoré que sur `https:` (localhost
//!   `127.0.0.1`/`[::1]` toléré pour le dev) ;
//! - `HttpOnly` empêche l'accès via `document.cookie` (le jar
//!   le filtre pour les appelants non-HTTP) ;
//! - `SameSite` exposé sur le cookie pour la politique d'envoi
//!   cross-site de l'étape 8 ;
//! - préfixes `__Host-` (https + host-only + `Path=/`) et
//!   `__Secure-` (https) requis ;
//! - jar borné : `max_cookies` total, `max_per_domain`, purge des
//!   expirés puis des plus anciens.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use crate::url::Url;

/// Politique `SameSite`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SameSite {
    /// `Strict` — jamais envoyé en cross-site.
    Strict,
    /// `Lax` — défaut navigateur moderne.
    #[default]
    Lax,
    /// `None` — requiert `Secure` pour être honoré.
    None,
}

/// Un cookie stocké.
#[derive(Debug, Clone)]
pub struct Cookie {
    /// Nom.
    pub name: String,
    /// Valeur.
    pub value: String,
    /// Domaine de pose (host si host-only, suffixe si `Domain=`).
    pub domain: String,
    /// `true` = cookie « host-only » (pas de `Domain=`).
    pub host_only: bool,
    /// Chemin.
    pub path: String,
    /// Expiration absolue (`Max-Age`/`Expires`), `None` = session.
    pub expires: Option<SystemTime>,
    /// `Secure`.
    pub secure: bool,
    /// `HttpOnly`.
    pub http_only: bool,
    /// `SameSite`.
    pub same_site: SameSite,
    /// Clé de partition CHIPS (étape 127) — `"{scheme}://
    /// {registrable}"` du site top-level au moment de la pose ;
    /// `None` = cookie non partitionné. Un cookie partitionné
    /// n'est renvoyé que dans son seau ([`request_partition`]
    /// égal), quelle que soit sa valeur `SameSite` — la
    /// partition remplace la politique `SameSite` (spec CHIPS).
    pub partition: Option<String>,
}

impl Cookie {
    fn is_expired(&self, now: SystemTime) -> bool {
        self.expires.is_some_and(|t| t <= now)
    }
}

/// Budgets du jar.
#[derive(Debug, Clone)]
pub struct CookieConfig {
    /// Cookies max au total (eviction des plus anciens expirés
    /// d'abord, puis FIFO).
    pub max_cookies: usize,
    /// Cookies max par domaine registrable.
    pub max_per_domain: usize,
}

impl Default for CookieConfig {
    fn default() -> Self {
        Self {
            max_cookies: 3000,
            max_per_domain: 50,
        }
    }
}

/// Clé de stockage : `(partition, domain, path, name)` — les
/// homonymes cohabitent sur des chemins différents **et dans
/// des partitions différentes** (CHIPS, étape 127 : le même
/// nom existe une fois par site top-level, plus la forme
/// non partitionnée).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Key {
    partition: Option<String>,
    domain: String,
    path: String,
    name: String,
}

/// Jar à cookies thread-safe (les fetches courent sur des tâches
/// tokio parallèles — `Mutex` court, jamais tenu sur un `.await`).
pub struct CookieJar {
    map: Mutex<HashMap<Key, Cookie>>,
    cfg: CookieConfig,
}

impl CookieJar {
    /// Jar vide, budgets par défaut.
    #[must_use]
    pub fn new() -> Self {
        Self::with_config(CookieConfig::default())
    }

    /// Jar vide, budgets explicites.
    #[must_use]
    pub fn with_config(cfg: CookieConfig) -> Self {
        Self {
            map: Mutex::new(HashMap::new()),
            cfg,
        }
    }

    /// Traite un en-tête `Set-Cookie` pour `url`. Renvoie `true` si
    /// le cookie a été accepté — rejet silencieux sinon (le web est
    /// hostile : un cookie invalide se droppe, ne crashe pas).
    ///
    /// Sans contexte de page : un attribut `Partitioned` se range
    /// sous le site de `url` elle-même (outil = top-level).
    pub fn set_from_header(&self, url: &Url, header: &str) -> bool {
        self.set_from_header_ctx(url, header, partition_key(url))
    }

    /// Variante contextuelle (CHIPS, étape 127) : `partition` =
    /// clé du site top-level ([`request_partition`]) sous lequel
    /// le cookie `Partitioned` est rangé — `None` pour un appelant
    /// sans page (la pose se range alors sous le site de `url`,
    /// comme [`Self::set_from_header`]).
    pub fn set_from_header_ctx(&self, url: &Url, header: &str, partition: Option<String>) -> bool {
        self.set_from_header_ctx_at(url, header, partition, SystemTime::now())
    }

    /// Variant datée — les tests passent un `now` fixe.
    pub fn set_from_header_at(&self, url: &Url, header: &str, now: SystemTime) -> bool {
        self.set_from_header_ctx_at(url, header, partition_key(url), now)
    }

    /// Variante contextuelle datée de [`Self::set_from_header_ctx`].
    pub fn set_from_header_ctx_at(
        &self,
        url: &Url,
        header: &str,
        partition: Option<String>,
        now: SystemTime,
    ) -> bool {
        let Some(cookie) = Self::parse(url, header, partition, now) else {
            return false;
        };
        let key = Key {
            partition: cookie.partition.clone(),
            domain: cookie.domain.clone(),
            path: cookie.path.clone(),
            name: cookie.name.clone(),
        };
        let mut map = self
            .map
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // Purge opportuniste des expirés.
        map.retain(|_, c| !c.is_expired(now));
        if cookie.is_expired(now) {
            // Cookie déjà expiré à la pose (`Max-Age=0`) = ordre de
            // suppression, pas insertion.
            map.remove(&key);
        } else {
            map.insert(key, cookie);
        }
        self.evict_over_budget(&mut map);
        true
    }

    /// `Cookie:` pour `url` — nom=valeur joints par `; `, les plus
    /// spécifiques (chemins longs) d'abord. `for_http` = inclure les
    /// `HttpOnly` (le fil réseau) ; `false` = vue `document.cookie`.
    ///
    /// Sans contexte d'initiateur (`SameSite` non appliqué —
    /// compat historique, requêtes hors page) ; la variante
    /// politique est [`Self::header_for_ctx`].
    #[must_use]
    pub fn header_for(&self, url: &Url, for_http: bool) -> String {
        self.header_for_at(url, for_http, SystemTime::now())
    }

    /// `Cookie:` pour `req` avec politique `SameSite` appliquée
    /// à l'envoi (étape 66, RFC 6265bis) : `Strict` exige
    /// same-site, `Lax` ajoute l'exception navigation top-level
    /// sûre (`GET`/`HEAD`), `None` reste libre. Un cookie
    /// **partitionné** (étape 127) n'entre que si sa clé égale
    /// [`request_partition`]`(req)` — la partition remplace la
    /// règle `SameSite`. Sans initiateur (`req.initiator`
    /// vide) = envoi permissif — les requêtes sans page (CLI,
    /// fakes) gardent le comportement historique.
    #[must_use]
    pub fn header_for_ctx(&self, req: &Request, for_http: bool) -> String {
        self.header_for_ctx_at(req, for_http, SystemTime::now())
    }

    /// Variant datée de [`Self::header_for_ctx`].
    #[must_use]
    pub fn header_for_ctx_at(&self, req: &Request, for_http: bool, now: SystemTime) -> String {
        self.header_for_sel(&req.url, for_http, now, |c| send_allows(c, req))
    }

    /// Variant datée de [`Self::header_for`].
    #[must_use]
    pub fn header_for_at(&self, url: &Url, for_http: bool, now: SystemTime) -> String {
        self.header_for_sel(url, for_http, now, |_| true)
    }

    /// Corps partagé de `header_for*` — `extra` affine
    /// l'admission de chaque cookie (politique `SameSite`
    /// / partition CHIPS).
    fn header_for_sel(
        &self,
        url: &Url,
        for_http: bool,
        now: SystemTime,
        extra: impl Fn(&Cookie) -> bool,
    ) -> String {
        let host = url.host();
        let https = url.scheme() == "https";
        let mut pairs: Vec<(usize, String)> = self
            .map
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .values()
            .filter(|c| {
                !c.is_expired(now)
                    && domain_match(&host, c)
                    && path_match(url.path(), &c.path)
                    && (!c.secure || https || is_localhost(&host))
                    && (for_http || !c.http_only)
                    && extra(c)
            })
            .map(|c| (c.path.len(), format!("{}={}", c.name, c.value)))
            .collect();
        pairs.sort_by_key(|a| std::cmp::Reverse(a.0));
        pairs
            .into_iter()
            .map(|(_, p)| p)
            .collect::<Vec<_>>()
            .join("; ")
    }

    /// Nombre de cookies stockés (debug/tests).
    #[must_use]
    pub fn len(&self) -> usize {
        self.map
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len()
    }

    /// `true` si le jar est vide.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Instantané des cookies vivants — persistance de profil
    /// (étape 56) : l'hôte sérialise la liste puis la rend via
    /// [`Self::insert_stored`]. Les expirés sont purgés à
    /// l'export (un cookie mort ne voyage pas).
    #[must_use]
    pub fn export(&self) -> Vec<Cookie> {
        let now = SystemTime::now();
        self.map
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .values()
            .filter(|c| !c.is_expired(now))
            .cloned()
            .collect()
    }

    /// Vide tous les cookies du **site** de `host` (domaine
    /// registrable — `Clear-Site-Data: "cookies"`/`"*"`,
    /// CAP-147) : cookies host/domaine **et** seaux CHIPS —
    /// le site perd ses identifiants quel que soit le
    /// contexte top-level de la pose (WICG clear-site-data
    /// §cookies). Retourne le nombre d'entrées supprimées.
    pub fn clear_site(&self, host: &str) -> usize {
        let target = registrable(&host.to_ascii_lowercase());
        let mut map = self
            .map
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let before = map.len();
        map.retain(|_, c| registrable(&c.domain) != target);
        before - map.len()
    }

    /// Réinsère un cookie restauré (étape 56) — même traitement
    /// qu'une pose `Set-Cookie` : expiré ignoré, budgets
    /// `CookieConfig` honorés. `false` si déjà expiré.
    pub fn insert_stored(&self, cookie: Cookie) -> bool {
        let now = SystemTime::now();
        if cookie.is_expired(now) {
            return false;
        }
        let key = Key {
            partition: cookie.partition.clone(),
            domain: cookie.domain.clone(),
            path: cookie.path.clone(),
            name: cookie.name.clone(),
        };
        let mut map = self
            .map
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        map.retain(|_, c| !c.is_expired(now));
        map.insert(key, cookie);
        self.evict_over_budget(&mut map);
        true
    }

    /// Parse `Set-Cookie` → `Cookie` si la politique l'accepte.
    fn parse(
        url: &Url,
        header: &str,
        partition: Option<String>,
        now: SystemTime,
    ) -> Option<Cookie> {
        let mut parts = header.split(';');
        let (name, value) = parts.next()?.split_once('=')?;
        let name = name.trim();
        if name.is_empty() {
            return None;
        }
        let https = url.scheme() == "https";
        let host = url.host();

        let mut c = Cookie {
            name: name.to_string(),
            value: value.trim().to_string(),
            domain: host.clone(),
            host_only: true,
            path: default_path(url.path()),
            expires: None,
            secure: false,
            http_only: false,
            same_site: SameSite::default(),
            partition: None,
        };
        let mut partitioned = false;

        for attr in parts {
            let (k, v) = attr.trim().split_once('=').unwrap_or((attr.trim(), ""));
            if k.eq_ignore_ascii_case("domain") {
                let d = v.trim().trim_start_matches('.').to_ascii_lowercase();
                // Le domaine déclaré doit suffixer l'hôte et ne pas
                // être un suffixe public (supercookie `.co.uk`) —
                // RFC 6265 §5.2.3 + PSL.
                if !is_suffix(&host, &d) || is_public_suffix(&d) {
                    return None;
                }
                c.domain = d;
                c.host_only = false;
            } else if k.eq_ignore_ascii_case("path") {
                c.path = if v.starts_with('/') {
                    v.trim().to_string()
                } else {
                    default_path(url.path())
                };
            } else if k.eq_ignore_ascii_case("max-age") {
                if let Ok(secs) = v.trim().parse::<i64>() {
                    if secs <= 0 {
                        c.expires = Some(now); // suppression immédiate
                    } else {
                        c.expires = Some(
                            now + Duration::from_secs(u64::try_from(secs).unwrap_or(u64::MAX)),
                        );
                    }
                }
            } else if k.eq_ignore_ascii_case("expires") {
                if let Some(t) = parse_http_date(v) {
                    c.expires = Some(t);
                }
            } else if k.eq_ignore_ascii_case("secure") {
                c.secure = true;
            } else if k.eq_ignore_ascii_case("httponly") {
                c.http_only = true;
            } else if k.eq_ignore_ascii_case("samesite") {
                c.same_site = match v.trim().to_ascii_lowercase().as_str() {
                    "strict" => SameSite::Strict,
                    "none" => SameSite::None,
                    _ => SameSite::Lax,
                };
            } else if k.eq_ignore_ascii_case("partitioned") {
                partitioned = true;
            }
        }

        // CHIPS (étape 127) : `Partitioned` exige `Secure` —
        // un cookie partitionné non sécurisé est rejeté, pas
        // dégradé (le seau d'un cookie invalide serait flou).
        if partitioned {
            if !c.secure {
                return None;
            }
            // Sans contexte explicite, la pose se range sous
            // le site de l'URL elle-même (top-level = soi).
            c.partition = Some(
                partition.unwrap_or_else(|| format!("{}://{}", url.scheme(), registrable(&host))),
            );
        }

        // Durcissements.
        if c.secure && !https && !is_localhost(&host) {
            return None; // Secure posé hors https → rejeté.
        }
        if c.name.starts_with("__Host-") {
            // __Host- : https, host-only, Path=/ — tout écart rejette.
            if !https && !is_localhost(&host) {
                return None;
            }
            if !c.host_only || c.path != "/" || !c.secure {
                return None;
            }
        }
        if c.name.starts_with("__Secure-") && !c.secure {
            return None;
        }
        if c.same_site == SameSite::None && !c.secure {
            // Spec modern : SameSite=None exige Secure — tolérant :
            // on retombe sur Lax plutôt que rejeter (les sites
            // hérités en abusent).
            c.same_site = SameSite::Lax;
        }
        Some(c)
    }

    /// Évince au-delà des budgets : expirés d'abord, puis FIFO par
    /// domaine, puis global.
    fn evict_over_budget(&self, map: &mut HashMap<Key, Cookie>) {
        // Par domaine registrable.
        let mut per_domain: HashMap<String, usize> = HashMap::new();
        for k in map.keys() {
            *per_domain.entry(registrable(&k.domain)).or_default() += 1;
        }
        if per_domain.values().all(|&n| n <= self.cfg.max_per_domain)
            && map.len() <= self.cfg.max_cookies
        {
            return;
        }
        // FIFO approximatif : on jette les cookies sans expiration
        // (session) en priorité — heuristique simple, à raffiner
        // avec un timestamp d'insertion si le corpus le réclame.
        let mut keys: Vec<Key> = map.keys().cloned().collect();
        keys.sort_by_key(|k| map.get(k).map_or(usize::MAX, |c| c.path.len()));
        for k in keys {
            let over_domain = {
                let d = registrable(&k.domain);
                per_domain.get(&d).copied().unwrap_or(0) > self.cfg.max_per_domain
            };
            if over_domain || map.len() > self.cfg.max_cookies {
                if let Some(c) = map.remove(&k) {
                    *per_domain.entry(registrable(&c.domain)).or_default() -= 1;
                }
            }
            if map.len() <= self.cfg.max_cookies
                && per_domain.values().all(|&n| n <= self.cfg.max_per_domain)
            {
                break;
            }
        }
    }
}

impl Default for CookieJar {
    fn default() -> Self {
        Self::new()
    }
}

/// `request-host` suffixe `cookie-domain` (RFC 6265 §5.1.3).
fn is_suffix(host: &str, domain: &str) -> bool {
    host == domain || host.ends_with(&format!(".{domain}"))
}

/// Le domaine est-il un suffixe public ? (`co.uk`, `github.io`…)
/// `psl::domain` renvoie `None` quand le nom **est** un suffixe
/// public → interdit en `Domain=` (supercookie).
fn is_public_suffix(domain: &str) -> bool {
    psl::domain(domain.as_bytes()).is_none()
}

/// Domaine registrable (`example.co.uk`) pour `max_per_domain`.
/// `pub(crate)` : le calcul « site » de `Sec-Fetch-Site`
/// (étape 125, `policy::sec_fetch_headers`) partage la même
/// notion que « site pour cookies ».
pub(crate) fn registrable(domain: &str) -> String {
    psl::domain(domain.as_bytes()).map_or_else(
        || domain.to_string(),
        |d| String::from_utf8_lossy(d.as_bytes()).into_owned(),
    )
}

/// Correspondance hôte/cookie : host-only = égalité exacte,
/// `Domain=` = suffixe (RFC 6265 §5.1.3).
fn domain_match(host: &str, c: &Cookie) -> bool {
    if c.host_only {
        host == c.domain
    } else {
        is_suffix(host, &c.domain)
    }
}

/// Correspondance chemin (RFC 6265 §5.1.4) : égalité, ou préfixe si
/// le cookie-path finit par `/` ou le suivant est `/`.
fn path_match(req_path: &str, cookie_path: &str) -> bool {
    if req_path == cookie_path {
        return true;
    }
    req_path.starts_with(cookie_path)
        && (cookie_path.ends_with('/') || req_path.as_bytes().get(cookie_path.len()) == Some(&b'/'))
}

/// Politique `SameSite` à l'envoi (étape 66, RFC 6265bis) :
/// « site » = `(schéma, domaine registrable)` — `Strict`
/// exige same-site ; `Lax` = same-site ou navigation
/// top-level sûre (`GET`/`HEAD`) ; `None` = libre
/// (`Secure` exigé à la pose). Sans initiateur = permissif
/// (compat — requêtes sans page initiatrice).
/// Clé de partition CHIPS d'une URL = site top-level
/// `"{scheme}://{registrable}"` (étape 127). `None` hors
/// `http(s)` — `data:`/`about:`/`file:` ne partitionnent pas.
#[must_use]
pub fn partition_key(url: &Url) -> Option<String> {
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    Some(format!("{}://{}", url.scheme(), registrable(&url.host())))
}

/// Seau CHIPS d'une requête (étape 127) — site top-level sous
/// lequel un cookie `Partitioned` est rangé/comparé :
/// navigation (`TopLevelNav`/`NavFrom`) → site de la **cible**
/// (le nouveau document devient le top-level) ; sous-ressource
/// → site de la page ; sans initiateur (outils) → site de la
/// cible elle-même. `None` hors `http(s)`.
#[must_use]
pub fn request_partition(req: &Request) -> Option<String> {
    match req.initiator.as_ref() {
        Some(Initiator::Subresource(page) | Initiator::Opaque(page)) => partition_key(page),
        Some(Initiator::TopLevelNav | Initiator::NavFrom(_)) | None => partition_key(&req.url),
    }
}

/// Admission d'un cookie à l'envoi : un cookie **partitionné**
/// n'entre que dans son seau ([`request_partition`] égal — la
/// partition remplace la politique `SameSite`, spec CHIPS) ;
/// un cookie ordinaire suit [`same_site_allows`].
fn send_allows(c: &Cookie, req: &Request) -> bool {
    match &c.partition {
        Some(p) => Some(p.as_str()) == request_partition(req).as_deref(),
        None => same_site_allows(c, req.initiator.as_ref(), req),
    }
}

fn same_site_allows(c: &Cookie, ini: Option<&Initiator>, req: &Request) -> bool {
    let Some(ini) = ini else { return true };
    let host = req.url.host();
    let same_site = |page: &Url| {
        page.scheme() == req.url.scheme() && registrable(&page.host()) == registrable(&host)
    };
    match (c.same_site, ini) {
        (SameSite::None, _) | (_, Initiator::TopLevelNav) => true,
        // Origine opaque (`<iframe sandbox>` sans
        // `allow-same-origin`) : same-site avec rien —
        // `Strict`/`Lax` retenus (spec Fetch/cookies).
        (SameSite::Strict | SameSite::Lax, Initiator::Opaque(_)) => false,
        (SameSite::Strict, Initiator::NavFrom(p) | Initiator::Subresource(p))
        | (SameSite::Lax, Initiator::Subresource(p)) => same_site(p),
        (SameSite::Lax, Initiator::NavFrom(p)) => {
            same_site(p) || matches!(req.method, Method::Get | Method::Head)
        }
    }
}

/// Chemin par défaut : le répertoire de l'URL (RFC 6265 §5.1.4).
fn default_path(url_path: &str) -> String {
    match url_path.rfind('/') {
        Some(0) | None => "/".to_string(),
        Some(i) => url_path[..i].to_string(),
    }
}

/// `localhost`/`127.0.0.1`/`[::1]` — les dev serveurs ont des
/// droits de cookie assouplis (pratique des navigateurs).
fn is_localhost(host: &str) -> bool {
    matches!(host, "localhost" | "127.0.0.1" | "[::1]")
}

/// `Expires=Wed, 21 Oct 2015 07:28:00 GMT` → `SystemTime`.
/// Parseur minimal IMF-fixdate RFC 7231 — tolère l'absence de
/// parsing (retour `None`, attr ignoré). `pub(crate)` : le cache
/// l'utilise pour `Expires:`.
pub(crate) fn parse_http_date(s: &str) -> Option<SystemTime> {
    let s = s.trim();
    // "Wd, DD Mon YYYY HH:MM:SS GMT" — on cherche les champs sans
    // exiger le jour de semaine.
    let comma = s.find(',')?;
    let rest = s[comma + 1..].trim();
    let mut it = rest.split_whitespace();
    let day: u64 = it
        .next()?
        .trim_matches(|c: char| !c.is_ascii_digit())
        .parse()
        .ok()?;
    let month = match it.next()?.get(..3)? {
        "Jan" => 1u64,
        "Feb" => 2,
        "Mar" => 3,
        "Apr" => 4,
        "May" => 5,
        "Jun" => 6,
        "Jul" => 7,
        "Aug" => 8,
        "Sep" => 9,
        "Oct" => 10,
        "Nov" => 11,
        "Dec" => 12,
        _ => return None,
    };
    let year: u64 = it.next()?.parse().ok()?;
    let year = if year < 100 { year + 1900 } else { year };
    let time = it.next()?;
    let mut t = time.split(':');
    let hh: u64 = t.next()?.parse().ok()?;
    let mm: u64 = t.next()?.parse().ok()?;
    let ss: u64 = t.next()?.parse().ok()?;
    // jours depuis epoch — civil_from_days (algorithme de Hinnant).
    let days = days_from_civil(year, month, day)?;
    let secs = days.checked_mul(86400)? + hh * 3600 + mm * 60 + ss;
    Some(SystemTime::UNIX_EPOCH + Duration::from_secs(secs))
}

/// `y-m-d` → jours depuis 1970-01-01 (algorithme `civil_from_days`).
fn days_from_civil(y: u64, m: u64, d: u64) -> Option<u64> {
    if !(1..=12).contains(&m) || d == 0 || d > 31 {
        return None;
    }
    let y = if m <= 2 { y.checked_sub(1)? } else { y };
    let era = y / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era.checked_mul(146_097)?
        .checked_add(doe)?
        .checked_sub(719_468)
}

// ---------------------------------------------------------------------
// CookieTransport — injection Cookie: + capture Set-Cookie.
// ---------------------------------------------------------------------

use std::sync::Arc;

use async_trait::async_trait;

use crate::error::NetResult;
use crate::http::{HttpTransport, Initiator, Method, Request, Response};

/// Transport décorateur : envoie `Cookie:` calculé par le jar et
/// capture les `Set-Cookie` de la réponse (toutes les lignes —
/// les doublons sont préservés par `headers_all`).
pub struct CookieTransport {
    inner: Arc<dyn HttpTransport>,
    jar: Arc<CookieJar>,
}

impl CookieTransport {
    /// Enveloppe `inner` avec le `jar` partagé.
    #[must_use]
    pub fn wrap(inner: Arc<dyn HttpTransport>, jar: Arc<CookieJar>) -> Self {
        Self { inner, jar }
    }
}

#[async_trait]
impl HttpTransport for CookieTransport {
    async fn fetch(&self, req: &Request) -> NetResult<Response> {
        let mut req = req.clone();
        let cookie = self.jar.header_for_ctx(&req, true);
        if !cookie.is_empty()
            && !req
                .headers
                .iter()
                .any(|(n, _)| n.eq_ignore_ascii_case("cookie"))
        {
            req.headers.push(("Cookie".to_string(), cookie));
        }
        let resp = self.inner.fetch(&req).await?;
        let partition = request_partition(&req);
        for v in resp.headers_all("set-cookie") {
            let _ = self.jar.set_from_header_ctx(&req.url, v, partition.clone());
        }
        Ok(resp)
    }

    /// Streamé (étape 101) : `Cookie:` injecté à l'émission,
    /// `Set-Cookie` capturé depuis les en-têtes streamés —
    /// disponibles dès réception, le corps n'est pas attendu.
    async fn fetch_stream(&self, req: &Request) -> NetResult<crate::http::StreamedResponse> {
        let mut req = req.clone();
        let cookie = self.jar.header_for_ctx(&req, true);
        if !cookie.is_empty()
            && !req
                .headers
                .iter()
                .any(|(n, _)| n.eq_ignore_ascii_case("cookie"))
        {
            req.headers.push(("Cookie".to_string(), cookie));
        }
        let resp = self.inner.fetch_stream(&req).await?;
        let partition = request_partition(&req);
        for (n, v) in &resp.headers {
            if n.eq_ignore_ascii_case("set-cookie") {
                let _ = self.jar.set_from_header_ctx(&req.url, v, partition.clone());
            }
        }
        Ok(resp)
    }
}
