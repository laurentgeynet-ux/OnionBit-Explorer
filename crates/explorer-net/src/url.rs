// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! URL — parsing et résolution (étape 3, ADR-0002).
//!
//! Sous-ensemble WHATWG *URL Standard* adapté au périmètre :
//!
//! - schémas hiérarchiques `http`, `https`, `file` (autorité
//!   `//[userinfo@]host[:port]`, chemin hiérarchique) ;
//! - schémas opaques `data:`/`about:` (chemin opaque **verbatim** —
//!   pas de normalisation des segments `.`/`..`, pas d'autorité) ;
//! - hôtes : domaines via **IDNA UTS-46** (crate `idna`, pur Rust),
//!   IPv4 aux formes WHATWG (octal `0o…`/`0…`, hex `0x…`, parties
//!   compressées `127.1`), IPv6 `[…]` ;
//! - normalisation : schéma/hôte en minuscules, port par défaut
//!   élidé, segments `.`/`..` résolus, percent-encodage des
//!   caractères interdits, `%XX` valides préservés ;
//! - résolution relative (RFC 3986 §5.2 / WHATWG *basic URL parser*) ;
//! - origine `(scheme, host, port)` pour la politique same-origin —
//!   `file:`/`data:`/`about:` → [`Origin::Opaque`].
//!
//! Déviations documentées (à revoir si un corpus réel en meurt) :
//! `ws`/`wss`/`ftp` absents de la table des ports spéciaux, `file:`
//! Windows ne canonicalise pas `c|` en `C:`, et la validation IDNA
//! est entièrement déléguée à `idna` (UTS-46).

use std::fmt;
use std::fmt::Write as _;
use std::net::Ipv6Addr;

use thiserror::Error;

/// Erreurs de parsing/résolution d'URL.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum UrlError {
    /// Pas de schéma et pas de base pour résoudre.
    #[error("URL relative sans base : {0}")]
    RelativeWithoutBase(String),
    /// Schéma absent ou invalide (`[a-zA-Z][a-zA-Z0-9+.-]*`).
    #[error("schéma invalide : {0}")]
    InvalidScheme(String),
    /// Hôte vide ou invalide pour un schéma hiérarchique.
    #[error("hôte invalide : {0}")]
    InvalidHost(String),
    /// Port non numérique ou hors `0..=65535`.
    #[error("port invalide : {0}")]
    InvalidPort(String),
    /// IDNA/UTS-46 a rejeté le domaine.
    #[error("domaine invalide : {0}")]
    InvalidDomain(String),
}

/// Origine d'une URL pour la politique same-origin (ROADMAP §7/8,
/// ADR-0002). `file:`, `data:`, `about:` sont opaques.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Origin {
    /// `(scheme, host, port)` — schéma hiérarchique à hôte+port.
    Tuple {
        /// Schéma minuscule.
        scheme: String,
        /// Hôte normalisé (ASCII pour les domaines).
        host: String,
        /// Port effectif (défaut du schéma si absent).
        port: u16,
    },
    /// Origine opaque — jamais same-origin avec quoi que ce soit.
    Opaque,
}

impl fmt::Display for Origin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Tuple { scheme, host, port } => write!(f, "{scheme}://{host}:{port}"),
            Self::Opaque => f.write_str("(opaque)"),
        }
    }
}

/// Forme d'hôte normalisée.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Host {
    /// Domaine ASCII (UTS-46 + minuscule).
    Domain(String),
    /// IPv4 en octets.
    Ipv4([u8; 4]),
    /// IPv6 en segments.
    Ipv6([u16; 8]),
    /// Hôte vide — `file:` (`file:///path`).
    Empty,
    /// Schéma opaque (`data:`, `about:`) — pas d'hôte.
    None,
}

/// Ports par défaut des schémas spéciaux (périmètre restreint à ceux
/// qu'on sert réellement — `ws`/`wss`/`ftp` pourront rejoindre la
/// table si un protocole le réclame).
fn default_port(scheme: &str) -> Option<u16> {
    match scheme {
        "http" => Some(80),
        "https" => Some(443),
        _ => None,
    }
}

/// Le schéma a une autorité (`//host`) et un chemin hiérarchique.
fn is_hierarchical(scheme: &str) -> bool {
    matches!(scheme, "http" | "https" | "file")
}

/// Une URL normalisée.
///
/// Invariant : `scheme` minuscule, `host` ASCII/minuscule pour les
/// domaines, `path` commence par `/` pour les schémas hiérarchiques,
/// `port` ne contient jamais le port par défaut du schéma.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Url {
    scheme: String,
    username: String,
    password: Option<String>,
    host: Host,
    port: Option<u16>,
    path: String,
    query: Option<String>,
    fragment: Option<String>,
}

impl Url {
    /// Parse une URL **absolue** (`scheme:…`). Les espaces ASCII
    /// périphériques et les tab/newline internes sont ignorés
    /// (spec — les URLs arrivent souvent sales).
    ///
    /// # Errors
    /// [`UrlError::RelativeWithoutBase`] si l'entrée n'a pas de
    /// schéma, `InvalidHost`/`InvalidPort`/`InvalidDomain` sur les
    /// composants rejetés.
    pub fn parse(input: &str) -> Result<Self, UrlError> {
        Self::parse_inner(input, None)
    }

    /// Résout `input` contre `base` (RFC 3986 §5.2 + WHATWG).
    ///
    /// - `input` avec schéma → URL absolue ;
    /// - `//h/…` → hérite le schéma ;
    /// - `/…` → chemin absolu sur la base ;
    /// - `?q`, `#f`, vide → réutilise le chemin de la base ;
    /// - `rel/path` → merge avec le répertoire de la base.
    ///
    /// # Errors
    /// Mêmes erreurs que [`Url::parse`].
    pub fn resolve(base: &Self, input: &str) -> Result<Self, UrlError> {
        Self::parse_inner(input, Some(base))
    }

    /// Schéma (minuscule).
    #[must_use]
    pub fn scheme(&self) -> &str {
        &self.scheme
    }

    /// Hôte formaté — `"example.com"`, `"127.0.0.1"`, `"[::1]"`,
    /// `""` pour `file:` ou un schéma opaque.
    #[must_use]
    pub fn host(&self) -> String {
        match &self.host {
            Host::Domain(d) => d.clone(),
            Host::Ipv4(o) => o.iter().map(u8::to_string).collect::<Vec<_>>().join("."),
            Host::Ipv6(s) => format!("[{}]", Ipv6Addr::from(*s)),
            Host::Empty | Host::None => String::new(),
        }
    }

    /// `true` si l'URL a un hôte (schéma hiérarchique non-`file`).
    #[must_use]
    pub fn has_host(&self) -> bool {
        matches!(self.host, Host::Domain(_) | Host::Ipv4(_) | Host::Ipv6(_))
    }

    /// Port explicite — `None` si le port par défaut est en vigueur.
    #[must_use]
    pub fn port(&self) -> Option<u16> {
        self.port
    }

    /// Port effectif (défaut du schéma si absent).
    #[must_use]
    pub fn effective_port(&self) -> Option<u16> {
        self.port.or_else(|| default_port(&self.scheme))
    }

    /// Chemin (`/`-préfixé pour les hiérarchiques, opaque sinon).
    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Query sans le `?`.
    #[must_use]
    pub fn query(&self) -> Option<&str> {
        self.query.as_deref()
    }

    /// Fragment sans le `#`.
    #[must_use]
    pub fn fragment(&self) -> Option<&str> {
        self.fragment.as_deref()
    }

    /// `path?query` — *origin-form* pour une requête HTTP.
    #[must_use]
    pub fn path_query(&self) -> String {
        match &self.query {
            Some(q) => format!("{}?{q}", self.path),
            None => self.path.clone(),
        }
    }

    /// Origine pour la politique same-origin.
    #[must_use]
    pub fn origin(&self) -> Origin {
        if is_hierarchical(&self.scheme) && self.scheme != "file" {
            if let Some(port) = self.effective_port() {
                return Origin::Tuple {
                    scheme: self.scheme.clone(),
                    host: self.host(),
                    port,
                };
            }
        }
        Origin::Opaque
    }

    /// Serialise l'URL en forme canonique.
    #[must_use]
    pub fn as_str(&self) -> String {
        let mut s = String::with_capacity(self.path.len() + 32);
        s.push_str(&self.scheme);
        s.push(':');
        if is_hierarchical(&self.scheme) {
            s.push_str("//");
            if !self.username.is_empty() || self.password.is_some() {
                s.push_str(&percent_encode(&self.username, keep_userinfo));
                if let Some(p) = &self.password {
                    s.push(':');
                    s.push_str(&percent_encode(p, keep_userinfo));
                }
                s.push('@');
            }
            s.push_str(&self.host());
            if let Some(p) = self.port {
                s.push(':');
                s.push_str(&p.to_string());
            }
        }
        s.push_str(&self.path);
        if let Some(q) = &self.query {
            s.push('?');
            s.push_str(q);
        }
        if let Some(fr) = &self.fragment {
            s.push('#');
            s.push_str(fr);
        }
        s
    }

    fn parse_inner(input: &str, base: Option<&Self>) -> Result<Self, UrlError> {
        // Spec : trim ASCII whitespace périphérique, supprime
        // tab/newline internes.
        let cleaned: String = input
            .trim_matches(|c: char| c.is_ascii_whitespace() || c == '\t' || c == '\n' || c == '\r')
            .chars()
            .filter(|&c| c != '\t' && c != '\n' && c != '\r')
            .collect();
        let input = cleaned.as_str();

        match scheme_end(input) {
            Some(i) => Self::parse_after_scheme(input[..i].to_ascii_lowercase(), &input[i + 1..]),
            None => match base {
                Some(b) => Self::resolve_relative(b, input),
                None => Err(UrlError::RelativeWithoutBase(input.to_string())),
            },
        }
    }

    /// Parse après `scheme:` — branche hiérarchique vs opaque.
    fn parse_after_scheme(scheme: String, rest: &str) -> Result<Self, UrlError> {
        if is_hierarchical(&scheme) {
            Self::parse_hierarchical(scheme, rest)
        } else {
            // Schéma opaque : chemin verbatim (`data:`/`about:`),
            // query/fragment restent découpables.
            let (path, query, fragment) = split_pqf(rest);
            Ok(Self {
                scheme,
                username: String::new(),
                password: None,
                host: Host::None,
                port: None,
                path,
                query,
                fragment,
            })
        }
    }

    /// `//authority/path?query#fragment`.
    fn parse_hierarchical(scheme: String, rest: &str) -> Result<Self, UrlError> {
        let rest = rest.strip_prefix("//").unwrap_or(rest);
        let auth_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
        let authority = &rest[..auth_end];
        let after_auth = &rest[auth_end..];

        let (username, password, hostport) = match authority.rfind('@') {
            Some(i) => {
                let ui = &authority[..i];
                let (u, p) = match ui.split_once(':') {
                    Some((u, p)) => (u, Some(p)),
                    None => (ui, None),
                };
                (
                    percent_decode(u),
                    p.map(percent_decode),
                    &authority[i + 1..],
                )
            }
            None => (String::new(), None, authority),
        };

        let (host, port) = parse_host_port(&scheme, hostport)?;
        let (raw_path, query, fragment) = split_pqf(after_auth);

        Ok(Self {
            scheme,
            username,
            password,
            host,
            port,
            path: normalize_path(&raw_path),
            query,
            fragment,
        })
    }

    /// Résolution relative ≙ RFC 3986 §5.2.2 / WHATWG *relative* state.
    fn resolve_relative(base: &Self, input: &str) -> Result<Self, UrlError> {
        if input.is_empty() {
            let mut u = base.clone();
            u.fragment = None;
            return Ok(u);
        }

        if !is_hierarchical(&base.scheme) {
            // Schéma opaque : seul `#fragment` est résolvable.
            return if let Some(frag) = input.strip_prefix('#') {
                let mut u = base.clone();
                u.fragment = Some(percent_encode(frag, keep_query));
                Ok(u)
            } else {
                Err(UrlError::RelativeWithoutBase(input.to_string()))
            };
        }

        if input.starts_with("//") {
            return Self::parse_hierarchical(base.scheme.clone(), input);
        }
        if input.starts_with('/') {
            let mut u = base.clone();
            let (p, q, f) = split_pqf(input);
            u.path = normalize_path(&p);
            u.query = q;
            u.fragment = f;
            return Ok(u);
        }
        if let Some(rest) = input.strip_prefix('?') {
            let (q, f) = match rest.split_once('#') {
                Some((a, b)) => (a, Some(b)),
                None => (rest, None),
            };
            let mut u = base.clone();
            u.query = Some(percent_encode(q, keep_query));
            u.fragment = f.map(|f| percent_encode(f, keep_query));
            return Ok(u);
        }
        if let Some(f) = input.strip_prefix('#') {
            let mut u = base.clone();
            u.fragment = Some(percent_encode(f, keep_query));
            return Ok(u);
        }

        // Chemin relatif : merge avec le répertoire de la base.
        let (rel_path, query, fragment) = split_pqf(input);
        let base_dir = base.path.rfind('/').map_or("", |i| &base.path[..=i]);
        let mut u = base.clone();
        u.path = normalize_path(&format!("{base_dir}{rel_path}"));
        u.query = query;
        u.fragment = fragment;
        Ok(u)
    }
}

impl fmt::Display for Url {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.as_str())
    }
}

impl std::str::FromStr for Url {
    type Err = UrlError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

// ---------------------------------------------------------------------
// Helpers de parsing
// ---------------------------------------------------------------------

/// Position du `:` terminant un schéma valide, ou `None`.
fn scheme_end(s: &str) -> Option<usize> {
    let i = s.find(':')?;
    let head = &s[..i];
    if head.is_empty() {
        return None;
    }
    let mut it = head.chars();
    if !it.next().is_some_and(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    it.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-'))
        .then_some(i)
}

/// Coupe `path?query#fragment` et percent-encode chaque partie.
fn split_pqf(s: &str) -> (String, Option<String>, Option<String>) {
    let (pq, fragment) = match s.split_once('#') {
        Some((p, f)) => (p, Some(percent_encode(f, keep_query))),
        None => (s, None),
    };
    let (path, query) = match pq.split_once('?') {
        Some((p, q)) => (p, Some(percent_encode(q, keep_query))),
        None => (pq, None),
    };
    (percent_encode(path, keep_path), query, fragment)
}

/// `host[:port]` — `[v6]:port` géré, `:` simple sinon. Le port par
/// défaut du schéma est élidé (invariant de [`Url`]).
fn parse_host_port(scheme: &str, hostport: &str) -> Result<(Host, Option<u16>), UrlError> {
    let (host_part, port_part) = if let Some(hp) = hostport.strip_prefix('[') {
        match hp.find(']') {
            Some(i) => {
                // Garde les crochets — `parse_host` reconnaît `[…]`.
                let h = &hostport[..i + 2];
                let rest = &hp[i + 1..];
                let p = rest.strip_prefix(':').filter(|s| !s.is_empty());
                (h, p)
            }
            None => return Err(UrlError::InvalidHost(hostport.to_string())),
        }
    } else {
        match hostport.rfind(':') {
            Some(i) if !hostport[i + 1..].contains(':') => {
                (&hostport[..i], Some(&hostport[i + 1..]))
            }
            _ => (hostport, None),
        }
    };

    let port = match port_part {
        Some(p) => {
            let n: u32 = p
                .parse()
                .map_err(|_| UrlError::InvalidPort(p.to_string()))?;
            if n > 65535 {
                return Err(UrlError::InvalidPort(p.to_string()));
            }
            u16::try_from(n).ok()
        }
        None => None,
    }
    .filter(|&p| default_port(scheme) != Some(p));

    Ok((parse_host(scheme, host_part)?, port))
}

/// Hôte → forme normalisée : percent-decode, IPv6, IPv4, IDNA.
fn parse_host(scheme: &str, host: &str) -> Result<Host, UrlError> {
    if host.is_empty() {
        return if scheme == "file" {
            Ok(Host::Empty)
        } else {
            Err(UrlError::InvalidHost(String::new()))
        };
    }
    if let Some(inner) = host.strip_prefix('[').and_then(|h| h.strip_suffix(']')) {
        return inner
            .parse::<Ipv6Addr>()
            .map(|a| Host::Ipv6(a.segments()))
            .map_err(|_| UrlError::InvalidHost(host.to_string()));
    }
    let decoded = percent_decode(host);
    if let Some(ip) = ipv4_parse(&decoded) {
        return Ok(Host::Ipv4(ip));
    }
    let ascii =
        idna::domain_to_ascii(&decoded).map_err(|_| UrlError::InvalidDomain(decoded.clone()))?;
    if ascii.is_empty() {
        return Err(UrlError::InvalidDomain(decoded));
    }
    Ok(Host::Domain(ascii.to_ascii_lowercase()))
}

/// IPv4 WHATWG : `a.b.c.d` ou formes réduites (`127.1` =
/// `127.0.0.1`), octal `0…`, hex `0x…`. Le dernier segment absorbe
/// les bits restants (1 seg = 32 bits, 2 = 8+24, 3 = 8+8+16).
fn ipv4_parse(s: &str) -> Option<[u8; 4]> {
    let parts: Vec<&str> = s.split('.').collect();
    if parts.is_empty() || parts.len() > 4 {
        return None;
    }
    if parts[..parts.len() - 1].iter().any(|p| p.is_empty()) {
        return None;
    }
    let mut nums = Vec::with_capacity(parts.len());
    for (i, p) in parts.iter().enumerate() {
        let n = ipv4_num(p)?;
        if i + 1 < parts.len() && n > 255 {
            return None;
        }
        nums.push(n);
    }
    let last = *nums.last()?;
    let bits_needed = u32::try_from(4 - (nums.len() - 1)).ok()? * 8;
    if bits_needed < 32 && last >= (1u64 << bits_needed) {
        return None;
    }
    let mut out = [0u8; 4];
    for (i, &n) in nums[..nums.len() - 1].iter().enumerate() {
        out[i] = u8::try_from(n).ok()?;
    }
    for (i, slot) in out.iter_mut().enumerate().skip(nums.len() - 1) {
        let shift = u32::try_from(3 - i).ok()? * 8;
        *slot = u8::try_from((last >> shift) & 0xFF).ok()?;
    }
    Some(out)
}

/// Un segment IPv4 : `0x…` hex, `0…` octal, sinon décimal.
fn ipv4_num(s: &str) -> Option<u64> {
    if s.is_empty() {
        return None;
    }
    if let Some(h) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u64::from_str_radix(h, 16).ok()
    } else if s.len() > 1 && s.starts_with('0') {
        u64::from_str_radix(&s[1..], 8)
            .ok()
            .or_else(|| s.parse().ok())
    } else {
        s.parse().ok()
    }
}

/// Normalise un chemin : résout `.`/`..`, préserve les segments
/// vides (pas de collapse `//`), garantit le `/` initial.
fn normalize_path(raw: &str) -> String {
    let segs: Vec<&str> = raw.split('/').collect();
    let last_seg = segs.last().copied().unwrap_or("");
    let mut out: Vec<&str> = Vec::new();
    for seg in segs {
        match seg {
            "." => {}
            ".." => {
                out.pop();
            }
            _ => out.push(seg),
        }
    }
    // Spec : un dernier segment `.`/`..` ajoute un segment vide —
    // `/a/.` → `/a/`, `/a/..` → `/`.
    if matches!(last_seg, "." | "..") {
        out.push("");
    }
    let mut path = out.join("/");
    if !path.starts_with('/') {
        path.insert(0, '/');
    }
    if path.is_empty() {
        path.push('/');
    }
    path
}

// ---------------------------------------------------------------------
// Percent-encoding — jeux de caractères WHATWG simplifiés.
// ---------------------------------------------------------------------

/// Percent-decode `%XX` → octets → UTF-8 lossy (octets invalides →
/// U+FFFD, spec).
///
/// Exposé pour les consommateurs dont la spec exige un décodage
/// du chemin opaque (`javascript:` CAP-154 — le code évalué est
/// le chemin percent-décodé).
#[must_use]
pub fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && hex_val(bytes[i + 1]).is_some()
            && hex_val(bytes[i + 2]).is_some()
        {
            out.push(
                (hex_val(bytes[i + 1]).unwrap_or(0) << 4) | hex_val(bytes[i + 2]).unwrap_or(0),
            );
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

const fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Ré-encode les octets hors du jeu `keep` ; les `%XX` valides sont
/// préservés (normalisés en majuscules).
fn percent_encode(s: &str, keep: impl Fn(u8) -> bool) -> String {
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'%'
            && i + 2 < bytes.len()
            && hex_val(bytes[i + 1]).is_some()
            && hex_val(bytes[i + 2]).is_some()
        {
            out.push('%');
            out.push(char::from(bytes[i + 1].to_ascii_uppercase()));
            out.push(char::from(bytes[i + 2].to_ascii_uppercase()));
            i += 3;
        } else if keep(b) {
            out.push(char::from(b));
            i += 1;
        } else {
            // Multi-octets UTF-8 : encode chaque octet.
            let start = i;
            i += 1;
            while i < bytes.len() && !keep(bytes[i]) {
                i += 1;
            }
            for &c in &bytes[start..i] {
                let _ = write!(out, "%{c:02X}");
            }
        }
    }
    out
}

/// Jeu *path* : alphanum + marks + `/` + ponctuations sûres.
const fn keep_path(b: u8) -> bool {
    b.is_ascii_alphanumeric()
        || matches!(
            b,
            b'-' | b'.'
                | b'_'
                | b'~'
                | b'!'
                | b'$'
                | b'&'
                | b'\''
                | b'('
                | b')'
                | b'*'
                | b'+'
                | b','
                | b';'
                | b'='
                | b':'
                | b'@'
                | b'/'
        )
}

/// Jeu *query*/*fragment* : path + `?`.
const fn keep_query(b: u8) -> bool {
    keep_path(b) || b == b'?'
}

/// Jeu *userinfo* : alphanum + marks (pas `:`/`@`/`/`).
const fn keep_userinfo(b: u8) -> bool {
    b.is_ascii_alphanumeric()
        || matches!(
            b,
            b'-' | b'.'
                | b'_'
                | b'~'
                | b'!'
                | b'$'
                | b'&'
                | b'\''
                | b'('
                | b')'
                | b'*'
                | b'+'
                | b','
                | b';'
                | b'='
        )
}
