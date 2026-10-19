// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Client HTTP/1.1 (étape 4, ADR-0002).
//!
//! - Connexions TCP via [`Connector`] injectable — `https` arrive à
//!   l'étape 5 en implémentant `Connector` TLS par-dessus `TcpStream`
//!   (la couture `IoStream` est déjà là).
//! - Requêtes **séquentielles** (pas de pipelining — spec ROADMAP).
//! - En-têtes parsées par `httparse`, corps `Content-Length` +
//!   `chunked` + EOF (HTTP/1.0).
//! - Pool keep-alive par `(scheme, host, port)` — connexions rendues
//!   au pool quand la réponse est entièrement délimitée et que le
//!   serveur n'a pas demandé `close`.
//! - Budgets bornés : timeouts connexion/lecture, tailles d'en-tête
//!   et de corps ([`HttpConfig`]).

use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;

use crate::error::{NetError, NetResult};
use crate::http::{HttpTransport, Method, Request, Response};

/// Un flot bidirectionnel — `TcpStream` en prod, TLS à l'étape 5,
/// duplex simulé en test.
pub trait IoStream: AsyncRead + AsyncWrite + Unpin + Send {}

impl<T> IoStream for T where T: AsyncRead + AsyncWrite + Unpin + Send {}

/// Ouvre les connexions — injectable pour que les tests tournent
/// in-process (ADR-0002 §11).
#[async_trait]
pub trait Connector: Send + Sync {
    /// Connecte à `(scheme, host, port)`.
    async fn connect(&self, scheme: &str, host: &str, port: u16) -> NetResult<Box<dyn IoStream>>;

    /// `true` si les requêtes `scheme://host` doivent partir en
    /// *absolute-form* (proxy HTTP : `GET http://h/p HTTP/1.1`).
    /// `false` par défaut — *origin-form*.
    fn absolute_form(&self, scheme: &str, host: &str) -> bool {
        let _ = (scheme, host);
        false
    }
}

/// Connecteur TCP en clair — ouvre une socket brute pour
/// `http:` **et** `https:` (la couche TLS est composée au-dessus
/// par [`TlsConnector`](crate::tls::TlsConnector) — utilisé seul,
/// il n'est valable que pour `http:`).
#[derive(Debug, Default, Clone, Copy)]
pub struct TcpConnector;

#[async_trait]
impl Connector for TcpConnector {
    async fn connect(&self, scheme: &str, host: &str, port: u16) -> NetResult<Box<dyn IoStream>> {
        if !matches!(scheme, "http" | "https") {
            return Err(NetError::UnsupportedScheme(scheme.to_string()));
        }
        let stream = TcpStream::connect((host, port)).await.map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                NetError::Dns {
                    host: host.to_string(),
                    source: e,
                }
            } else {
                NetError::Connect {
                    target: format!("{scheme}://{host}:{port}"),
                    source: e,
                }
            }
        })?;
        Ok(Box::new(stream))
    }
}

/// Budgets et identification du client (règle AGENTS #13 : aucune
/// valeur en dur dans la logique).
#[derive(Debug, Clone)]
pub struct HttpConfig {
    /// Timeout d'établissement de connexion.
    pub connect_timeout: Duration,
    /// Timeout par opération de lecture/écriture.
    pub io_timeout: Duration,
    /// Taille max de l'en-tête de réponse (octets).
    pub max_head_bytes: usize,
    /// Taille max du corps déframé (octets).
    pub max_body_bytes: usize,
    /// Valeur `User-Agent` envoyée.
    pub user_agent: String,
    /// Connexions idle max conservées par origine dans le pool.
    pub pool_max_per_origin: usize,
}

impl Default for HttpConfig {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(10),
            io_timeout: Duration::from_secs(30),
            max_head_bytes: 64 * 1024,
            max_body_bytes: 64 * 1024 * 1024,
            user_agent: "Explorer/0.1".to_string(),
            pool_max_per_origin: 4,
        }
    }
}

/// Clé de pool keep-alive.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct PoolKey {
    scheme: String,
    host: String,
    port: u16,
}

/// Connexion réutilisable + tampon résiduel (les octets après la fin
/// des en-têtes appartiennent déjà au corps).
struct Pooled {
    stream: Box<dyn IoStream>,
    buf: Vec<u8>,
    /// Requêtes en *absolute-form* (proxy HTTP).
    absolute_form: bool,
}

/// Client HTTP/1.1 — implémente [`HttpTransport`].
pub struct H1Client {
    connector: Arc<dyn Connector>,
    cfg: HttpConfig,
    pool: Mutex<HashMap<PoolKey, Vec<Pooled>>>,
}

impl H1Client {
    /// Client TCP en clair + budgets par défaut.
    #[must_use]
    pub fn tcp() -> Self {
        Self::new(Arc::new(TcpConnector), HttpConfig::default())
    }

    /// Client sur connecteur injecté (tests, TLS étape 5).
    #[must_use]
    pub fn new(connector: Arc<dyn Connector>, cfg: HttpConfig) -> Self {
        Self {
            connector,
            cfg,
            pool: Mutex::new(HashMap::new()),
        }
    }

    /// Vide le pool keep-alive (changement de réseau, fermeture
    /// d'onglet…).
    pub fn close_idle(&self) {
        self.pool
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
    }

    /// Sort une connexion du pool ou en ouvre une — renvoie
    /// `(conn, reused)`.
    async fn take_conn(&self, key: &PoolKey, req: &Request) -> NetResult<(Pooled, bool)> {
        let pooled = self
            .pool
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get_mut(key)
            .and_then(Vec::pop);
        if let Some(p) = pooled {
            return Ok((p, true));
        }
        let absolute_form = self.connector.absolute_form(&key.scheme, &key.host);
        let stream = timeout(
            self.cfg.connect_timeout,
            self.connector.connect(&key.scheme, &key.host, key.port),
        )
        .await
        .map_err(|_| NetError::Timeout {
            phase: "connect",
            ms: u64::try_from(self.cfg.connect_timeout.as_millis()).unwrap_or(u64::MAX),
            target: target_of(req),
        })??;
        Ok((
            Pooled {
                stream,
                buf: Vec::new(),
                absolute_form,
            },
            false,
        ))
    }

    async fn io<T>(
        &self,
        req: &Request,
        fut: impl std::future::Future<Output = std::io::Result<T>>,
    ) -> NetResult<T> {
        timeout(self.cfg.io_timeout, fut)
            .await
            .map_err(|_| NetError::Timeout {
                phase: "io",
                ms: u64::try_from(self.cfg.io_timeout.as_millis()).unwrap_or(u64::MAX),
                target: target_of(req),
            })?
            .map_err(NetError::Io)
    }

    fn write_request(&self, req: &Request, absolute_form: bool) -> Vec<u8> {
        let mut s = String::with_capacity(512);
        let target = if absolute_form {
            // Proxy HTTP : *absolute-form* `GET http://h[:p]/path?q`.
            let mut a = format!("{}://{}", req.url.scheme(), req.url.host());
            if let Some(p) = req.url.port() {
                let _ = write!(a, ":{p}");
            }
            format!("{a}{}", req.url.path_query())
        } else {
            req.url.path_query()
        };
        let _ = write!(s, "{} {} HTTP/1.1\r\n", req.method.as_str(), target);
        // Host — inclut le port explicite (pas le port par défaut).
        let mut host = req.url.host();
        if let Some(p) = req.url.port() {
            let _ = write!(host, ":{p}");
        }
        let _ = write!(s, "Host: {host}\r\nUser-Agent: {}\r\n", self.cfg.user_agent);
        for (n, v) in &req.headers {
            let _ = write!(s, "{n}: {v}\r\n");
        }
        if let Some(b) = &req.body {
            let _ = write!(s, "Content-Length: {}\r\n", b.len());
        }
        s.push_str("\r\n");
        s.into_bytes()
    }
}

#[async_trait]
impl HttpTransport for H1Client {
    async fn fetch(&self, req: &Request) -> NetResult<Response> {
        let key = PoolKey {
            scheme: req.url.scheme().to_string(),
            host: req.url.host(),
            port: req
                .url
                .effective_port()
                .ok_or_else(|| NetError::UnsupportedScheme(req.url.scheme().to_string()))?,
        };

        let (mut conn, reused) = self.take_conn(&key, req).await?;
        let wire = self.write_request(req, conn.absolute_form);

        let res = self.roundtrip(req, &mut conn, &wire).await;
        match res {
            Ok(mut resp) => {
                resp.reused_connection = reused;
                if is_reusable(&resp, req) {
                    self.put_back(&key, conn);
                }
                Ok(resp)
            }
            Err(e) => {
                // Une connexion poolée qui échoue à l'écriture/lecture
                // est réessayée une fois sur une connexion neuve —
                // uniquement pour les méthodes idempotentes.
                if reused && matches!(req.method, Method::Get | Method::Head | Method::Options) {
                    let mut fresh = Pooled {
                        stream: self
                            .connector
                            .connect(&key.scheme, &key.host, key.port)
                            .await?,
                        buf: Vec::new(),
                        absolute_form: self.connector.absolute_form(&key.scheme, &key.host),
                    };
                    let mut resp = self.roundtrip(req, &mut fresh, &wire).await?;
                    resp.reused_connection = false;
                    if is_reusable(&resp, req) {
                        self.put_back(&key, fresh);
                    }
                    Ok(resp)
                } else {
                    Err(e)
                }
            }
        }
    }
}

fn target_of(req: &Request) -> String {
    format!("{}://{}", req.url.scheme(), req.url.host())
}

impl H1Client {
    /// Rend une connexion au pool, borné par `pool_max_per_origin`.
    fn put_back(&self, key: &PoolKey, conn: Pooled) {
        let mut pool = self
            .pool
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let v = pool.entry(key.clone()).or_default();
        if v.len() < self.cfg.pool_max_per_origin {
            v.push(conn);
        }
    }
}

/// `Connection: close` ? (nom case-insensitive, valeur « close »).
fn wants_close(resp: &Response) -> bool {
    resp.headers_all("connection")
        .any(|v| v.eq_ignore_ascii_case("close"))
}

/// La connexion survit-elle à cette réponse ?
/// Corps délimité (CL/chunked/none) + pas de `close` demandé.
fn is_reusable(resp: &Response, req: &Request) -> bool {
    if wants_close(resp) {
        return false;
    }
    // Réponses sans corps : réutilisables par définition.
    if no_body(req.method, resp.status) {
        return true;
    }
    // Les corps délimités (CL ou chunked) ont été intégralement lus
    // par `read_body` — le stream est à la frontière propre.
    resp.header("content-length").is_some()
        || resp
            .header("transfer-encoding")
            .is_some_and(|v| v.eq_ignore_ascii_case("chunked"))
}

/// Réponses sans corps (RFC 9110 §9.3 / §15.3) : méthode HEAD,
/// 1xx, 204, 304.
fn no_body(method: Method, status: u16) -> bool {
    method == Method::Head || (100..200).contains(&status) || status == 204 || status == 304
}

impl H1Client {
    /// Un aller-retour complet sur `conn` — écrit, lit l'en-tête,
    /// lit le corps déframé. Erreur typée à chaque étape.
    async fn roundtrip(
        &self,
        req: &Request,
        conn: &mut Pooled,
        wire: &[u8],
    ) -> NetResult<Response> {
        self.io(req, conn.stream.write_all(wire)).await?;
        if let Some(b) = &req.body {
            self.io(req, conn.stream.write_all(b)).await?;
        }

        let head_end = self.read_head(req, conn).await?;
        let head = &conn.buf[..head_end];
        let mut headers_raw = [httparse::EMPTY_HEADER; 96];
        let mut parsed = httparse::Response::new(&mut headers_raw);
        parsed
            .parse(head)
            .map_err(|_| NetError::InvalidResponse("en-têtes illisibles".to_string()))?;
        let status = parsed
            .code
            .ok_or_else(|| NetError::InvalidResponse("pas de statut".to_string()))?;
        let reason = parsed.reason.unwrap_or_default().to_string();
        let headers: Vec<(String, String)> = parsed
            .headers
            .iter()
            .map(|h| {
                (
                    h.name.to_string(),
                    String::from_utf8_lossy(h.value).into_owned(),
                )
            })
            .collect();

        // Le reste du tampon = début du corps.
        let body_start = conn.buf.split_off(head_end);
        conn.buf = body_start;

        let mut resp = Response {
            status,
            reason,
            headers,
            body: Vec::new(),
            reused_connection: false,
        };
        resp.body = self.read_body(req, &resp, conn).await?;
        Ok(resp)
    }

    /// Lit jusqu'à `\r\n\r\n` ou le budget d'en-tête.
    async fn read_head(&self, req: &Request, conn: &mut Pooled) -> NetResult<usize> {
        loop {
            if let Some(i) = find_seq(&conn.buf, b"\r\n\r\n") {
                return Ok(i + 4);
            }
            if conn.buf.len() > self.cfg.max_head_bytes {
                return Err(NetError::HeadTooLarge(self.cfg.max_head_bytes));
            }
            let mut chunk = [0u8; 8192];
            let n = self.io(req, conn.stream.read(&mut chunk)).await?;
            if n == 0 {
                return Err(NetError::InvalidResponse(
                    "connexion fermée avant la fin des en-têtes".to_string(),
                ));
            }
            conn.buf.extend_from_slice(&chunk[..n]);
        }
    }

    /// Lit le corps selon `Content-Length`/`chunked`/EOF, borné par
    /// `max_body_bytes`.
    async fn read_body(
        &self,
        req: &Request,
        resp: &Response,
        conn: &mut Pooled,
    ) -> NetResult<Vec<u8>> {
        if no_body(req.method, resp.status) {
            return Ok(Vec::new());
        }

        let chunked = resp
            .header("transfer-encoding")
            .is_some_and(|v| v.eq_ignore_ascii_case("chunked"));
        if chunked {
            return self.read_chunked(req, conn).await;
        }

        if let Some(cl) = resp.header("content-length") {
            let len: usize = cl
                .trim()
                .parse()
                .map_err(|_| NetError::InvalidResponse(format!("Content-Length invalide: {cl}")))?;
            if len > self.cfg.max_body_bytes {
                return Err(NetError::BodyTooLarge(len));
            }
            while conn.buf.len() < len {
                let mut chunk = vec![0u8; (len - conn.buf.len()).min(8192)];
                let n = self.io(req, conn.stream.read(&mut chunk)).await?;
                if n == 0 {
                    return Err(NetError::TruncatedBody {
                        got: conn.buf.len(),
                        expected: len,
                    });
                }
                conn.buf.extend_from_slice(&chunk[..n]);
            }
            let body = conn.buf.drain(..len).collect();
            return Ok(body);
        }

        // Pas de délimiteur → corps jusqu'à EOF (HTTP/1.0). La
        // connexion ne sera pas réutilisée.
        let mut body = std::mem::take(&mut conn.buf);
        let mut chunk = [0u8; 8192];
        loop {
            let n = self.io(req, conn.stream.read(&mut chunk)).await?;
            if n == 0 {
                break;
            }
            body.extend_from_slice(&chunk[..n]);
            if body.len() > self.cfg.max_body_bytes {
                return Err(NetError::BodyTooLarge(self.cfg.max_body_bytes));
            }
        }
        Ok(body)
    }

    /// `Transfer-Encoding: chunked` — parse `<hex>\r\n<data>\r\n`
    /// jusqu'au chunk `0` + trailers terminés par `\r\n`.
    async fn read_chunked(&self, req: &Request, conn: &mut Pooled) -> NetResult<Vec<u8>> {
        let mut body = Vec::new();
        loop {
            // Ligne de taille.
            let line = self.read_line(req, conn).await?;
            let size = usize::from_str_radix(
                line.trim_end_matches(['\r', '\n'])
                    .split(';')
                    .next()
                    .unwrap_or(""),
                16,
            )
            .map_err(|_| NetError::InvalidChunked("taille hex illisible"))?;
            if body.len() + size > self.cfg.max_body_bytes {
                return Err(NetError::BodyTooLarge(self.cfg.max_body_bytes));
            }
            if size == 0 {
                // Trailers jusqu'à ligne vide.
                loop {
                    let l = self.read_line(req, conn).await?;
                    if l == "\r\n" || l.is_empty() {
                        return Ok(body);
                    }
                }
            }
            while conn.buf.len() < size + 2 {
                let mut chunk = vec![0u8; (size + 2 - conn.buf.len()).min(8192)];
                let n = self.io(req, conn.stream.read(&mut chunk)).await?;
                if n == 0 {
                    return Err(NetError::TruncatedBody {
                        got: body.len() + conn.buf.len(),
                        expected: body.len() + size,
                    });
                }
                conn.buf.extend_from_slice(&chunk[..n]);
            }
            body.extend(conn.buf.drain(..size));
            // `\r\n` final du chunk.
            if &conn.buf[..2] != b"\r\n" {
                return Err(NetError::InvalidChunked("CRLF manquant après chunk"));
            }
            conn.buf.drain(..2);
        }
    }

    /// Lit une ligne terminée par `\r\n` depuis le tampon+stream.
    async fn read_line(&self, req: &Request, conn: &mut Pooled) -> NetResult<String> {
        loop {
            if let Some(i) = find_seq(&conn.buf, b"\r\n") {
                let line: Vec<u8> = conn.buf.drain(..i + 2).collect();
                return Ok(String::from_utf8_lossy(&line).into_owned());
            }
            if conn.buf.len() > self.cfg.max_head_bytes {
                return Err(NetError::InvalidChunked("ligne > budget en-tête"));
            }
            let mut chunk = [0u8; 8192];
            let n = self.io(req, conn.stream.read(&mut chunk)).await?;
            if n == 0 {
                return Err(NetError::TruncatedBody {
                    got: conn.buf.len(),
                    expected: usize::MAX,
                });
            }
            conn.buf.extend_from_slice(&chunk[..n]);
        }
    }
}

/// `memchr` sur séquence — première occurrence de `needle`.
fn find_seq(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}
