// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Résolution de noms injectable (étape 72 — ADR-0022
//! prolongé) : le [`Resolver`] par défaut est le système
//! (`tokio::net::lookup_host`) ; [`DohResolver`] interroge
//! un résolveur **DNS-over-HTTPS** configuré
//! (`application/dns-message` en POST — RFC 8484) via une
//! pile `HttpTransport` ordinaire, pour les réseaux dont le
//! DNS est captif ou menteur. Le mode est un choix
//! d'assemblage (daemon `--doh`), jamais un basculement
//! silencieux — et il ne contourne ni le blocage IP ni le
//! filtrage SNI (le proxy/Tor reste la réponse à ce niveau).
//!
//! Le format DNS embarqué est minimal : requêtes `A`/`AAAA`
//! sans recursion côté client, réponses compressées par
//! pointeurs (RFC 1035 §4.1.4) — aucune dépendance.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;

use crate::error::{NetError, NetResult};
use crate::http::{HttpTransport, Method, Request};
use crate::url::Url;

/// Résout un nom d'hôte en adresses IP — injecté dans
/// [`ResolvingConnector`](crate::h1::ResolvingConnector).
#[async_trait]
pub trait Resolver: Send + Sync {
    /// Adresses candidates — vide ou erreur = échec DNS.
    async fn resolve(&self, host: &str) -> NetResult<Vec<IpAddr>>;

    /// Descripteur observable (contrat `GET /api/egress`).
    fn describe(&self) -> String;
}

/// Résolution système — `getaddrinfo` via tokio.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemResolver;

#[async_trait]
impl Resolver for SystemResolver {
    async fn resolve(&self, host: &str) -> NetResult<Vec<IpAddr>> {
        let it = tokio::net::lookup_host((host, 0u16))
            .await
            .map_err(|e| NetError::Dns {
                host: host.to_string(),
                source: e,
            })?;
        Ok(it.map(|a| a.ip()).collect())
    }

    fn describe(&self) -> String {
        "system".to_string()
    }
}

/// Résolveur DNS-over-HTTPS (RFC 8484) : `POST {endpoint}`
/// corps `application/dns-message`, une requête par type
/// (`A` puis `AAAA`), réponses mises en cache par TTL RR
/// (borné : le clamp évite une requête par connexion — une
/// page ouvre des dizaines de sockets sous-ressources).
pub struct DohResolver {
    /// Endpoint `https://…/dns-query` — **aucune valeur en
    /// dur** : fourni par `--doh`/`EXPLORER_DOH`.
    url: Url,
    /// Pile HTTPS ordinaire (résolution **système** de
    /// l'endpoint lui-même — pas de récursion `DoH`).
    transport: std::sync::Arc<dyn HttpTransport>,
    /// Timeout par requête `DoH`.
    timeout: Duration,
    /// Cache `(adresses, expiration)` par hôte — TTL des RR
    /// clamé à [`MIN_TTL_SECS`, `MAX_TTL_SECS`].
    cache: Mutex<HashMap<String, (Instant, Vec<IpAddr>)>>,
}

/// TTL plancher du cache `DoH` (règle 13 : borne de config,
/// pas de magie dans le flux) — un RR à TTL 0 n'interroge
/// pas le résolveur à chaque socket.
const MIN_TTL_SECS: u32 = 10;
/// TTL plafond — une IP qui rote vite n'est pas figée.
const MAX_TTL_SECS: u32 = 300;
/// Identifiant de transaction — fixe : la réponse n'est pas
/// routée par id dans un usage séquentiel (et un id
/// aléatoire exigerait un RNG non présent dans le crate).
const QUERY_ID: u16 = 0xE10;

impl std::fmt::Debug for DohResolver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DohResolver")
            .field("url", &self.url)
            .finish_non_exhaustive()
    }
}

impl DohResolver {
    /// Résolveur `DoH` pointant `url` (typiquement
    /// `https://resolver/dns-query`), requêtes via
    /// `transport` (pile TLS+H1 **sans** `DoH` — la
    /// résolution de l'endpoint reste système).
    #[must_use]
    pub fn new(url: Url, transport: std::sync::Arc<dyn HttpTransport>, timeout: Duration) -> Self {
        Self {
            url,
            transport,
            timeout,
            cache: Mutex::new(HashMap::new()),
        }
    }

    /// Une requête `qtype` (`A`=1, `AAAA`=28) — réponses
    /// décodées en `(adresses, TTL minimal en secondes)`.
    async fn query(&self, host: &str, qtype: u16) -> NetResult<(Vec<IpAddr>, u32)> {
        let body = encode_query(host, qtype)?;
        let mut req = Request::get(self.url.clone());
        req.method = Method::Post;
        req.headers = vec![(
            "Content-Type".to_string(),
            "application/dns-message".to_string(),
        )];
        req.body = Some(body);
        let resp = tokio::time::timeout(self.timeout, self.transport.fetch(&req))
            .await
            .map_err(|_| dns_err(host, "timeout `DoH`"))?
            .map_err(|e| dns_err(host, &e.to_string()))?;
        if resp.status != 200 {
            return Err(dns_err(host, &format!("DoH HTTP {}", resp.status)));
        }
        let ct = resp.header("content-type").unwrap_or_default();
        if !ct.contains("application/dns-message") {
            return Err(dns_err(host, "content-type non dns-message"));
        }
        decode_answers(&resp.body, host)
    }
}

#[async_trait]
impl Resolver for DohResolver {
    async fn resolve(&self, host: &str) -> NetResult<Vec<IpAddr>> {
        // Noms d'usage spécial (étape 78) : jamais par `DoH`
        // — un résolveur public n'a aucune réponse légitime
        // pour eux. Délégation système : `localhost` répond,
        // `.test` échoue honnêtement.
        if is_special_use(host) {
            return SystemResolver.resolve(host).await;
        }
        if let Ok(hit) = self.cache.lock() {
            if let Some((exp, ips)) = hit.get(host) {
                if Instant::now() < *exp {
                    return Ok(ips.clone());
                }
            }
        }
        let (mut out, mut ttl) = self.query(host, QTYPE_A).await.unwrap_or_default();
        let (v6, ttl6) = self.query(host, QTYPE_AAAA).await.unwrap_or_default();
        out.extend(v6);
        ttl = ttl.min(ttl6).clamp(MIN_TTL_SECS, MAX_TTL_SECS);
        if out.is_empty() {
            return Err(dns_err(host, "DoH : aucune adresse"));
        }
        if let Ok(mut hit) = self.cache.lock() {
            hit.insert(
                host.to_string(),
                (
                    Instant::now() + Duration::from_secs(u64::from(ttl)),
                    out.clone(),
                ),
            );
        }
        Ok(out)
    }

    fn describe(&self) -> String {
        format!("doh:{}", self.url.as_str())
    }
}

/// Noms d'usage spécial — RFC 6761 + `.local` (mDNS,
/// RFC 6762). En mode `DoH` ils restent sur la résolution
/// système (étape 78, comme Chrome/Firefox) : un résolveur
/// public ne les connaît pas — `localhost`, les fixtures
/// `.test`, le multicast `.local` et les noms de
/// documentation ne doivent jamais partir en requête `DoH`.
fn is_special_use(host: &str) -> bool {
    let h = host.trim_end_matches('.').to_ascii_lowercase();
    if h == "localhost" {
        return true;
    }
    for suf in [".localhost", ".local", ".test", ".invalid", ".example"] {
        if h.ends_with(suf) {
            return true;
        }
    }
    // Noms de documentation (RFC 6761 §6.5).
    matches!(h.as_str(), "example.com" | "example.net" | "example.org")
}

const QTYPE_A: u16 = 1;
const QTYPE_AAAA: u16 = 28;
const QCLASS_IN: u16 = 1;

fn dns_err(host: &str, msg: &str) -> NetError {
    NetError::Dns {
        host: host.to_string(),
        source: std::io::Error::new(std::io::ErrorKind::NotFound, msg.to_string()),
    }
}

/// Requête DNS filaire : en-tête + question unique
/// `(host, qtype, IN)` — flags `RD`.
fn encode_query(host: &str, qtype: u16) -> NetResult<Vec<u8>> {
    let mut q = Vec::with_capacity(64);
    q.extend_from_slice(&QUERY_ID.to_be_bytes());
    q.extend_from_slice(&0x0100u16.to_be_bytes()); // RD
    q.extend_from_slice(&1u16.to_be_bytes()); // QDCOUNT
    q.extend_from_slice(&[0u8; 6]); // AN/NS/AR = 0
    for label in host.trim_end_matches('.').split('.') {
        if label.len() > 63 {
            return Err(dns_err(host, "label > 63"));
        }
        q.push(u8::try_from(label.len()).unwrap_or(0));
        q.extend_from_slice(label.as_bytes());
    }
    q.push(0);
    q.extend_from_slice(&qtype.to_be_bytes());
    q.extend_from_slice(&QCLASS_IN.to_be_bytes());
    Ok(q)
}

/// Réponse DNS → `(adresses, TTL minimal)` des RR `A`/`AAAA`
/// — compression par pointeurs supportée dans les noms de
/// RR ; le TTL minimal borne le cache appelant.
fn decode_answers(buf: &[u8], host: &str) -> NetResult<(Vec<IpAddr>, u32)> {
    if buf.len() < 12 {
        return Err(dns_err(host, "réponse tronquée"));
    }
    let an = u16::from_be_bytes([buf[6], buf[7]]) as usize;
    let mut pos = 12usize;
    let qd = u16::from_be_bytes([buf[4], buf[5]]) as usize;
    for _ in 0..qd {
        pos = skip_name(buf, pos).ok_or_else(|| dns_err(host, "question malformée"))?;
        pos = pos
            .checked_add(4)
            .ok_or_else(|| dns_err(host, "question tronquée"))?;
    }
    let mut out = Vec::new();
    let mut ttl = u32::MAX;
    for _ in 0..an {
        pos = skip_name(buf, pos).ok_or_else(|| dns_err(host, "réponse malformée"))?;
        let end = pos
            .checked_add(10)
            .ok_or_else(|| dns_err(host, "RR tronqué"))?;
        if end > buf.len() {
            return Err(dns_err(host, "RR tronqué"));
        }
        let rtype = u16::from_be_bytes([buf[pos], buf[pos + 1]]);
        let rr_ttl = u32::from_be_bytes([buf[pos + 4], buf[pos + 5], buf[pos + 6], buf[pos + 7]]);
        let rdlen = u16::from_be_bytes([buf[pos + 8], buf[pos + 9]]) as usize;
        let rdata = end;
        if rdata + rdlen > buf.len() {
            return Err(dns_err(host, "rdata tronqué"));
        }
        match (rtype, rdlen) {
            (QTYPE_A, 4) => {
                ttl = ttl.min(rr_ttl);
                out.push(IpAddr::from([
                    buf[rdata],
                    buf[rdata + 1],
                    buf[rdata + 2],
                    buf[rdata + 3],
                ]));
            }
            (QTYPE_AAAA, 16) => {
                ttl = ttl.min(rr_ttl);
                let mut b = [0u8; 16];
                b.copy_from_slice(&buf[rdata..rdata + 16]);
                out.push(IpAddr::from(b));
            }
            _ => {}
        }
        pos = rdata + rdlen;
    }
    Ok((out, if ttl == u32::MAX { 0 } else { ttl }))
}

/// Saute un nom DNS (labels + pointeurs de compression) —
/// renvoie la position juste après.
fn skip_name(buf: &[u8], mut pos: usize) -> Option<usize> {
    loop {
        let len = *buf.get(pos)? as usize;
        if len == 0 {
            return Some(pos + 1);
        }
        if len & 0xC0 == 0xC0 {
            return pos.checked_add(2); // pointeur : 2 octets
        }
        pos = pos.checked_add(1 + len)?;
    }
}

/// TTL par défaut d'une entrée du cache DNS persisté (PR-9) —
/// `getaddrinfo` ne rapporte pas le TTL RR, on borne à une
/// durée courte et documentée (5 min) plutôt que de retenir
/// des adresses périmées.
pub const DNS_CACHE_TTL: Duration = Duration::from_secs(300);

/// Store de cache DNS persisté — injecté dans
/// [`CachingResolver`] ; la persistance vit chez l'hôte
/// (`state/dns_cache.json` — `JsonDnsStore` côté
/// `explorer-core`, PR-9).
pub trait DnsStore: Send + Sync {
    /// Adresses mémorisées non expirées — `None` = miss.
    fn lookup(&self, host: &str) -> Option<Vec<IpAddr>>;
    /// Mémorise `ips` pour `host` avec la TTL fournie
    /// (bornée côté store).
    fn record(&self, host: &str, ips: &[IpAddr], ttl: Duration);
}

/// Résolveur à cache persisté (PR-9) : hit non expiré → les
/// adresses mémorisées évitent la requête DNS ; miss →
/// résolution amont puis enregistrement. `describe` chaîne le
/// descripteur amont (`system`, `doh:…`).
pub struct CachingResolver {
    inner: Arc<dyn Resolver>,
    store: Arc<dyn DnsStore>,
    ttl: Duration,
}

impl CachingResolver {
    /// Enveloppe `inner` avec le `store` injecté.
    #[must_use]
    pub fn wrap(inner: Arc<dyn Resolver>, store: Arc<dyn DnsStore>, ttl: Duration) -> Self {
        Self { inner, store, ttl }
    }
}

#[async_trait]
impl Resolver for CachingResolver {
    async fn resolve(&self, host: &str) -> NetResult<Vec<IpAddr>> {
        if let Some(ips) = self.store.lookup(host) {
            return Ok(ips);
        }
        let ips = self.inner.resolve(host).await?;
        if !ips.is_empty() {
            self.store.record(host, &ips, self.ttl);
        }
        Ok(ips)
    }

    fn describe(&self) -> String {
        format!("cache({})", self.inner.describe())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Query A `example.com` → 93.184.216.34 — réponse
    /// minimale avec pointeur de compression `c0 0c`.
    #[test]
    fn decodes_compressed_a_record() {
        let q = encode_query("example.com", QTYPE_A).unwrap();
        let mut r = q.clone();
        // Flags : réponse, pas d'erreur.
        r[2] = 0x81;
        r[3] = 0x80;
        r[6] = 0;
        r[7] = 1; // ANCOUNT = 1
        r.extend_from_slice(&[
            0xc0, 0x0c, // name → offset 12 (la question)
            0x00, 0x01, // type A
            0x00, 0x01, // class IN
            0x00, 0x00, 0x01, 0x2c, // TTL 300
            0x00, 0x04, // RDLENGTH
            93, 184, 216, 34,
        ]);
        let (ips, ttl) = decode_answers(&r, "example.com").unwrap();
        assert_eq!(ips, vec![IpAddr::from([93, 184, 216, 34])]);
        assert_eq!(ttl, 300);
    }

    #[test]
    fn encodes_question_wire_format() {
        let q = encode_query("a.b", QTYPE_AAAA).unwrap();
        assert_eq!(&q[..2], &QUERY_ID.to_be_bytes());
        assert_eq!(&q[4..6], &1u16.to_be_bytes());
        assert_eq!(&q[12..], b"\x01a\x01b\x00\x00\x1c\x00\x01");
    }
}
