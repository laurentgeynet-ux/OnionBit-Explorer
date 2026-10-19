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
use crate::http::{HttpTransport, Method, Request, Response, StreamedResponse};

/// Un flot bidirectionnel — `TcpStream` en prod, TLS à l'étape 5,
/// duplex simulé en test.
pub trait IoStream: AsyncRead + AsyncWrite + Unpin + Send {}

impl<T> IoStream for T where T: AsyncRead + AsyncWrite + Unpin + Send {}

/// Connexion établie + protocole de couche applicative négocié
/// (ALPN TLS : `b"h2"`, `b"http/1.1"` — RFC 7301). `alpn` est
/// `None` quand rien n'a été négocié (TCP clair, connecteur
/// simulé) — la sémantique de repli HTTP/1.1 est alors celle de
/// la spec : absence d'ALPN ⇒ HTTP/1.1.
pub struct Conn {
    /// Flot bidirectionnel prêt à l'emploi.
    pub stream: Box<dyn IoStream>,
    /// Protocole ALPN négocié, s'il y a lieu.
    pub alpn: Option<Vec<u8>>,
}

/// Ouvre les connexions — injectable pour que les tests tournent
/// in-process (ADR-0002 §11).
#[async_trait]
pub trait Connector: Send + Sync {
    /// Connecte à `(scheme, host, port)`.
    async fn connect(&self, scheme: &str, host: &str, port: u16) -> NetResult<Box<dyn IoStream>>;

    /// Connecte et rapporte le protocole ALPN négocié — repli :
    /// `alpn: None` (TCP clair, connecteurs non négociants).
    /// [`TlsConnector`](crate::tls::TlsConnector) le renseigne.
    async fn connect_negotiated(&self, scheme: &str, host: &str, port: u16) -> NetResult<Conn> {
        Ok(Conn {
            stream: self.connect(scheme, host, port).await?,
            alpn: None,
        })
    }

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
pub(crate) struct PoolKey {
    pub(crate) scheme: String,
    pub(crate) host: String,
    pub(crate) port: u16,
}

/// Connexion réutilisable + tampon résiduel (les octets après la fin
/// des en-têtes appartiennent déjà au corps).
pub(crate) struct Pooled {
    pub(crate) stream: Box<dyn IoStream>,
    pub(crate) buf: Vec<u8>,
    /// Requêtes en *absolute-form* (proxy HTTP).
    pub(crate) absolute_form: bool,
}

/// Client HTTP/1.1 — implémente [`HttpTransport`].
pub struct H1Client {
    connector: Arc<dyn Connector>,
    cfg: HttpConfig,
    /// `Arc` : la tâche de streaming corps (étape 53) remet la
    /// connexion au pool après la fin du corps — elle survit à
    /// l'appel `fetch_stream`.
    pool: Arc<Mutex<HashMap<PoolKey, Vec<Pooled>>>>,
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
            pool: Arc::new(Mutex::new(HashMap::new())),
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

/// Clé de pool d'une requête (`scheme`, `host`, port effectif).
pub(crate) fn pool_key(req: &Request) -> NetResult<PoolKey> {
    Ok(PoolKey {
        scheme: req.url.scheme().to_string(),
        host: req.url.host(),
        port: req
            .url
            .effective_port()
            .ok_or_else(|| NetError::UnsupportedScheme(req.url.scheme().to_string()))?,
    })
}

#[async_trait]
impl HttpTransport for H1Client {
    async fn fetch(&self, req: &Request) -> NetResult<Response> {
        let key = pool_key(req)?;
        let (conn, reused) = self.take_conn(&key, req).await?;
        self.exchange(req, &key, conn, reused).await
    }

    async fn fetch_stream(&self, req: &Request) -> NetResult<StreamedResponse> {
        let key = pool_key(req)?;
        let (conn, reused) = self.take_conn(&key, req).await?;
        self.exchange_stream(req, &key, conn, reused).await
    }
}

impl H1Client {
    /// Sert `req` sur une connexion déjà établie — chemin de repli
    /// ALPN `http/1.1` d'[`crate::h2::AutoClient`], qui conserve la
    /// connexion négociée plutôt que d'en rouvrir une.
    pub(crate) async fn fetch_on_stream(
        &self,
        key: &PoolKey,
        stream: Box<dyn IoStream>,
        req: &Request,
    ) -> NetResult<Response> {
        let conn = Pooled {
            stream,
            buf: Vec::new(),
            absolute_form: self.connector.absolute_form(&key.scheme, &key.host),
        };
        self.exchange(req, key, conn, false).await
    }

    /// Variante streamée de [`Self::fetch_on_stream`] (étape 53) —
    /// même connexion conservée, corps par morceaux.
    pub(crate) async fn stream_on_conn(
        &self,
        key: &PoolKey,
        stream: Box<dyn IoStream>,
        req: &Request,
    ) -> NetResult<StreamedResponse> {
        let conn = Pooled {
            stream,
            buf: Vec::new(),
            absolute_form: self.connector.absolute_form(&key.scheme, &key.host),
        };
        self.exchange_stream(req, key, conn, false).await
    }

    /// Un aller-retour + gestion de pool — corps de `fetch`.
    async fn exchange(
        &self,
        req: &Request,
        key: &PoolKey,
        conn: Pooled,
        reused: bool,
    ) -> NetResult<Response> {
        let (mut conn, mut resp) = self.open_response(req, key, conn, reused).await?;
        resp.body = self.read_body(req, &resp, &mut conn).await?;
        resp.reused_connection = reused;
        if is_reusable(&resp, req) {
            self.put_back(key, conn);
        }
        Ok(resp)
    }

    /// Réponse streamée (étape 53) : en-têtes lues ici, corps
    /// lu en tâche — chaque morceau déframé part sur le canal
    /// et la connexion rejoint le pool à la fin si réutilisable.
    async fn exchange_stream(
        &self,
        req: &Request,
        key: &PoolKey,
        conn: Pooled,
        reused: bool,
    ) -> NetResult<StreamedResponse> {
        let (conn, resp) = self.open_response(req, key, conn, reused).await?;
        let (tx, rx) = tokio::sync::mpsc::channel(4);
        // La tâche possède la connexion jusqu'à fin de corps —
        // remise au pool par `pool` si la réponse est réutilisable.
        let task = BodyTask {
            ctx: IoCtx {
                io_timeout: self.cfg.io_timeout,
                target: target_of(req),
                max_body: self.cfg.max_body_bytes,
            },
            method: req.method,
            status: resp.status,
            headers: resp.headers.clone(),
            conn,
            tx,
            pool: Arc::clone(&self.pool),
            key: key.clone(),
            pool_max: self.cfg.pool_max_per_origin,
        };
        tokio::spawn(task.run());
        Ok(StreamedResponse {
            status: resp.status,
            reason: resp.reason,
            headers: resp.headers,
            reused_connection: reused,
            body: rx,
        })
    }

    /// Écrit la requête et lit l'en-tête — partagé par
    /// [`exchange`] (corps bufferisé) et [`exchange_stream`]
    /// (corps streamé). Une connexion poolée qui échoue à
    /// l'écriture/lecture est réessayée une fois sur une
    /// connexion neuve — méthodes idempotentes seulement.
    async fn open_response(
        &self,
        req: &Request,
        key: &PoolKey,
        mut conn: Pooled,
        reused: bool,
    ) -> NetResult<(Pooled, Response)> {
        let wire = self.write_request(req, conn.absolute_form);
        match self.roundtrip(req, &mut conn, &wire).await {
            Ok(resp) => Ok((conn, resp)),
            Err(e) => {
                if reused && matches!(req.method, Method::Get | Method::Head | Method::Options) {
                    let mut fresh = Pooled {
                        stream: self
                            .connector
                            .connect(&key.scheme, &key.host, key.port)
                            .await?,
                        buf: Vec::new(),
                        absolute_form: self.connector.absolute_form(&key.scheme, &key.host),
                    };
                    let resp = self.roundtrip(req, &mut fresh, &wire).await?;
                    Ok((fresh, resp))
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
    /// Écrit la requête et lit l'en-tête de la réponse — le corps
    /// reste à lire dans `conn.buf`/le flot (bufferisé par
    /// [`Self::read_body`], streamé par [`BodyTask`]). Erreur
    /// typée à chaque étape.
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

        Ok(Response {
            status,
            reason,
            headers,
            body: Vec::new(),
            reused_connection: false,
        })
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

/// Contexte I/O de la tâche de lecture corps — ce que `self.io`
/// fournit au chemin bufferisé, sans `&H1Client` (la tâche
/// streamée est `'static`, étape 53).
struct IoCtx {
    io_timeout: std::time::Duration,
    target: String,
    max_body: usize,
}

impl IoCtx {
    /// Timeout par opération + aplatissement `io::Result` —
    /// même facteur que `H1Client::io`.
    async fn io<T>(
        &self,
        fut: impl std::future::Future<Output = std::io::Result<T>>,
    ) -> NetResult<T> {
        timeout(self.io_timeout, fut)
            .await
            .map_err(|_| NetError::Timeout {
                phase: "io",
                ms: u64::try_from(self.io_timeout.as_millis()).unwrap_or(u64::MAX),
                target: self.target.clone(),
            })?
            .map_err(NetError::Io)
    }
}

/// Tâche de lecture corps streamée (étape 53) — possède la
/// connexion : chaque morceau déframé part sur `tx`, la
/// connexion rejoint le pool en fin de corps si réutilisable,
/// une erreur est livrée en dernier item avant fermeture.
struct BodyTask {
    ctx: IoCtx,
    method: Method,
    status: u16,
    headers: Vec<(String, String)>,
    conn: Pooled,
    tx: tokio::sync::mpsc::Sender<NetResult<Vec<u8>>>,
    pool: Arc<Mutex<HashMap<PoolKey, Vec<Pooled>>>>,
    key: PoolKey,
    pool_max: usize,
}

impl BodyTask {
    /// La future est volumineuse par construction (connexion +
    /// tampon + pool + en-têtes) — elle n'est `.await`ée que par
    /// `tokio::spawn`, allocation unique, pas de pile à grossir.
    #[allow(clippy::large_futures)]
    async fn run(mut self) {
        let reusable = is_reusable_parts(&self.headers, self.method, self.status);
        match self.stream_body().await {
            Ok(()) if reusable => {
                let mut pool = self
                    .pool
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let v = pool.entry(self.key).or_default();
                if v.len() < self.pool_max {
                    v.push(self.conn);
                }
            }
            Ok(()) => {}
            Err(e) => {
                let _ = self.tx.send(Err(e)).await;
            }
        }
    }

    /// Le corps déframé, morceau par morceau — même découpage
    /// que `read_body` (`Content-Length` / `chunked` / EOF),
    /// chaque segment part immédiatement sur le canal.
    async fn stream_body(&mut self) -> NetResult<()> {
        if no_body(self.method, self.status) {
            return Ok(());
        }
        let chunked = self.headers.iter().any(|(n, v)| {
            n.eq_ignore_ascii_case("transfer-encoding") && v.eq_ignore_ascii_case("chunked")
        });
        if chunked {
            return self.stream_chunked().await;
        }
        if let Some(cl) = self
            .headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case("content-length"))
            .map(|(_, v)| v.as_str())
        {
            let len: usize = cl
                .trim()
                .parse()
                .map_err(|_| NetError::InvalidResponse(format!("Content-Length invalide: {cl}")))?;
            if len > self.ctx.max_body {
                return Err(NetError::BodyTooLarge(len));
            }
            let mut sent = 0usize;
            while sent < len {
                if self.conn.buf.is_empty() {
                    let mut chunk = [0u8; 8192];
                    let n = self.ctx.io(self.conn.stream.read(&mut chunk)).await?;
                    if n == 0 {
                        return Err(NetError::TruncatedBody {
                            got: sent,
                            expected: len,
                        });
                    }
                    self.conn.buf.extend_from_slice(&chunk[..n]);
                }
                let take = self.conn.buf.len().min(len - sent);
                let piece: Vec<u8> = self.conn.buf.drain(..take).collect();
                sent += take;
                if self.tx.send(Ok(piece)).await.is_err() {
                    return Ok(()); // receveur parti — connexion consumée.
                }
            }
            return Ok(());
        }
        // Pas de délimiteur → corps jusqu'à EOF (HTTP/1.0) — la
        // connexion n'est pas réutilisable.
        let mut total = self.conn.buf.len();
        if !self.conn.buf.is_empty() {
            let piece = std::mem::take(&mut self.conn.buf);
            if self.tx.send(Ok(piece)).await.is_err() {
                return Ok(());
            }
        }
        let mut chunk = [0u8; 8192];
        loop {
            let n = self.ctx.io(self.conn.stream.read(&mut chunk)).await?;
            if n == 0 {
                return Ok(());
            }
            total += n;
            if total > self.ctx.max_body {
                return Err(NetError::BodyTooLarge(total));
            }
            if self.tx.send(Ok(chunk[..n].to_vec())).await.is_err() {
                return Ok(());
            }
        }
    }

    /// `Transfer-Encoding: chunked` streamé — chaque segment de
    /// données part dès sa réception (jamais assemblé en entier).
    async fn stream_chunked(&mut self) -> NetResult<()> {
        let mut total = 0usize;
        loop {
            let line = self.stream_line().await?;
            let size = usize::from_str_radix(
                line.trim_end_matches(['\r', '\n'])
                    .split(';')
                    .next()
                    .unwrap_or(""),
                16,
            )
            .map_err(|_| NetError::InvalidChunked("taille hex illisible"))?;
            if total + size > self.ctx.max_body {
                return Err(NetError::BodyTooLarge(total + size));
            }
            if size == 0 {
                loop {
                    let l = self.stream_line().await?;
                    if l == "\r\n" || l.is_empty() {
                        return Ok(());
                    }
                }
            }
            total += size;
            // `size` octets de données + `\r\n` terminal : les
            // données partent sur le canal, le CRLF est drainé.
            let mut read = 0usize;
            while read < size + 2 {
                if self.conn.buf.is_empty() {
                    let mut chunk = [0u8; 8192];
                    let n = self.ctx.io(self.conn.stream.read(&mut chunk)).await?;
                    if n == 0 {
                        return Err(NetError::TruncatedBody {
                            got: total,
                            expected: total + size,
                        });
                    }
                    self.conn.buf.extend_from_slice(&chunk[..n]);
                }
                let take = self.conn.buf.len().min(size + 2 - read);
                let data = take.min(size.saturating_sub(read));
                let piece: Vec<u8> = self.conn.buf.drain(..data).collect();
                self.conn.buf.drain(..take - data);
                read += take;
                if !piece.is_empty() && self.tx.send(Ok(piece)).await.is_err() {
                    return Ok(());
                }
            }
        }
    }

    /// Lit une ligne `\r\n` depuis le tampon + flot (version
    /// tâche de `H1Client::read_line`).
    async fn stream_line(&mut self) -> NetResult<String> {
        loop {
            if let Some(i) = find_seq(&self.conn.buf, b"\r\n") {
                let line: Vec<u8> = self.conn.buf.drain(..i + 2).collect();
                return Ok(String::from_utf8_lossy(&line).into_owned());
            }
            let mut chunk = [0u8; 8192];
            let n = self.ctx.io(self.conn.stream.read(&mut chunk)).await?;
            if n == 0 {
                return Err(NetError::TruncatedBody {
                    got: 0,
                    expected: usize::MAX,
                });
            }
            self.conn.buf.extend_from_slice(&chunk[..n]);
        }
    }
}

/// `is_reusable` sur les champs de tête (avant lecture du corps) —
/// même règle : `close` demandé → non, corps absent → oui,
/// délimiteur CL/chunked requis sinon.
fn is_reusable_parts(headers: &[(String, String)], method: Method, status: u16) -> bool {
    if headers
        .iter()
        .any(|(n, v)| n.eq_ignore_ascii_case("connection") && v.eq_ignore_ascii_case("close"))
    {
        return false;
    }
    if no_body(method, status) {
        return true;
    }
    headers
        .iter()
        .any(|(n, _)| n.eq_ignore_ascii_case("content-length"))
        || headers.iter().any(|(n, v)| {
            n.eq_ignore_ascii_case("transfer-encoding") && v.eq_ignore_ascii_case("chunked")
        })
}

/// `memchr` sur séquence — première occurrence de `needle`.
fn find_seq(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}
