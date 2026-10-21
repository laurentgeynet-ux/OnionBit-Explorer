// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Décodage des corps HTTP (étape 6, ADR-0002).
//!
//! [`DecodingTransport`] enveloppe un [`HttpTransport`] : il
//! annonce `Accept-Encoding: gzip, deflate, br`, décompresse le
//! corps selon `Content-Encoding` (plusieurs codages possibles, en
//! ordre inverse d'application), borne le résultat, et met à jour
//! `Content-Length`/`Content-Encoding` pour que le corps vu par
//! l'amont soit toujours **décodé**.
//!
//! Le *charset* (document, pas transport — AGENTS §8) est seulement
//! exposé via [`Response::charset`] ; le transcodage proprement dit
//! reste côté `explorer-html`.

use std::io::{BufRead, BufReader, Read};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use crate::error::{NetError, NetResult};
use crate::http::{HttpTransport, Request, Response, StreamedResponse};

/// Valeur `Accept-Encoding` annoncée — le trio servi par les
/// décodeurs ci-dessous.
pub const ACCEPT_ENCODING: &str = "gzip, deflate, br";

/// Transport décorateur : `Accept-Encoding` + décompression bornée.
pub struct DecodingTransport {
    inner: Arc<dyn HttpTransport>,
    /// Corps décodé max (octets) — le budget de l'amont
    /// (`max_body_bytes`) borne le *fil*, celui-ci borne la
    /// *décompression* (bombe zip = contenu hostile classique).
    max_decoded_bytes: usize,
}

impl DecodingTransport {
    /// Enveloppe `inner` avec un budget de décompression `max_decoded`.
    #[must_use]
    pub fn wrap(inner: Arc<dyn HttpTransport>, max_decoded_bytes: usize) -> Self {
        Self {
            inner,
            max_decoded_bytes,
        }
    }
}

#[async_trait]
impl HttpTransport for DecodingTransport {
    async fn fetch(&self, req: &Request) -> NetResult<Response> {
        let mut req = req.clone();
        if !req
            .headers
            .iter()
            .any(|(n, _)| n.eq_ignore_ascii_case("accept-encoding"))
        {
            req.headers
                .push(("Accept-Encoding".to_string(), ACCEPT_ENCODING.to_string()));
        }

        let mut resp = self.inner.fetch(&req).await?;
        decode_response(&mut resp, self.max_decoded_bytes)?;
        Ok(resp)
    }

    /// Variante streamée (étape 101) : statut et en-têtes rendus
    /// dès réception, corps décodé **en vol** — avant, le repli
    /// `fetch` bufferisait le corps filaire complet avant de
    /// rendre la main, ce qui retenait un statut d'erreur
    /// derrière son corps (403 anti-bot servi au goutte-à-goutte
    /// → `io_timeout` par maillon, ~60 s mesuré terrain fnac.fr)
    /// et tuait la peinture progressive sur toute la chaîne.
    ///
    /// Réponses sans corps (1xx/204/304) ou non encodées :
    /// passage direct. Encodées : tâche `spawn_blocking`
    /// (décodeurs `io::Read` synchrones) alimentée par les
    /// morceaux filaires via `ChanReader`, poussant les
    /// morceaux décodés au fil — `Content-Encoding` /
    /// `Content-Length` retirés (la taille décodée n'est pas
    /// connue d'avance), borne `max_decoded_bytes` conservée.
    async fn fetch_stream(&self, req: &Request) -> NetResult<StreamedResponse> {
        let mut req = req.clone();
        if !req
            .headers
            .iter()
            .any(|(n, _)| n.eq_ignore_ascii_case("accept-encoding"))
        {
            req.headers
                .push(("Accept-Encoding".to_string(), ACCEPT_ENCODING.to_string()));
        }
        let mut resp = self.inner.fetch_stream(&req).await?;
        if resp.status == 204 || resp.status == 304 || (100..200).contains(&resp.status) {
            return Ok(resp);
        }
        let codings: Vec<String> = match resp.header("content-encoding") {
            None => return Ok(resp),
            Some(ce) => ce
                .split(',')
                .map(|c| c.trim().to_ascii_lowercase())
                .filter(|c| !c.is_empty() && c != "identity")
                .collect(),
        };
        if codings.is_empty() {
            return Ok(resp);
        }
        // Le corps décodé n'a plus la taille filaire : les
        // en-têtes reflètent le flux vu par l'amont.
        resp.headers.retain(|(n, _)| {
            !n.eq_ignore_ascii_case("content-encoding") && !n.eq_ignore_ascii_case("content-length")
        });
        let (tx, rx) = tokio::sync::mpsc::channel(4);
        let upstream = std::mem::replace(&mut resp.body, rx);
        let max = self.max_decoded_bytes;
        tokio::task::spawn_blocking(move || {
            decode_stream_task(upstream, &codings, max, &tx);
        });
        Ok(resp)
    }
}

/// Tâche de décodage en vol — `spawn_blocking` car les
/// décodeurs sont synchrones (`io::Read`). Pompe le corps
/// filaire du canal amont, décode, pousse les morceaux décodés.
/// En cas d'échec, l'erreur amont d'origine (timeout, coupure)
/// prévaut sur l'erreur décodeur qu'elle a déclenchée.
fn decode_stream_task(
    upstream: tokio::sync::mpsc::Receiver<NetResult<Vec<u8>>>,
    codings: &[String],
    max: usize,
    tx: &tokio::sync::mpsc::Sender<NetResult<Vec<u8>>>,
) {
    let slot: Arc<Mutex<Option<NetError>>> = Arc::new(Mutex::new(None));
    let reader = ChanReader {
        rx: upstream,
        cur: Vec::new(),
        pos: 0,
        err: Arc::clone(&slot),
    };
    if let Err(e) = pump_decoded(reader, codings, max, tx) {
        let e = slot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
            .unwrap_or(e);
        let _ = tx.blocking_send(Err(e));
    }
}

/// `io::Read` au-dessus du canal de morceaux filaires — légal en
/// `spawn_blocking` (`blocking_recv`). Une erreur amont est
/// conservée typée dans `err` et convertie en `io` pour sortir
/// la pile de décodeurs proprement.
struct ChanReader {
    rx: tokio::sync::mpsc::Receiver<NetResult<Vec<u8>>>,
    cur: Vec<u8>,
    pos: usize,
    err: Arc<Mutex<Option<NetError>>>,
}

impl Read for ChanReader {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        while self.pos == self.cur.len() {
            match self.rx.blocking_recv() {
                Some(Ok(c)) => {
                    self.cur = c;
                    self.pos = 0;
                }
                Some(Err(e)) => {
                    *self
                        .err
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(e);
                    return Err(std::io::Error::other("corps amont en erreur"));
                }
                None => return Ok(0),
            }
        }
        let n = out.len().min(self.cur.len() - self.pos);
        out[..n].copy_from_slice(&self.cur[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

/// Pompe `reader` → chaîne de décodeurs → canal aval, bornée à
/// `max` octets décodés. Les codings s'empilent en ordre inverse
/// d'application — même règle que [`decode_body`]. Fin de flux
/// compressé prématurée : le partiel déjà poussé est conservé
/// (le consommateur streamé est le parseur tag-soup, tolérant par
/// construction — même dégradé que le chemin bufferisé).
fn pump_decoded(
    reader: ChanReader,
    codings: &[String],
    max: usize,
    tx: &tokio::sync::mpsc::Sender<NetResult<Vec<u8>>>,
) -> NetResult<()> {
    let mut r: Box<dyn Read + Send> = Box::new(reader);
    for coding in codings.iter().rev() {
        r = stream_decoder(r, coding)?;
    }
    let mut buf = [0u8; 8192];
    let mut total = 0usize;
    loop {
        match r.read(&mut buf) {
            Ok(0) => return Ok(()),
            Ok(n) => {
                total += n;
                if total > max {
                    return Err(NetError::DecodedTooLarge(max));
                }
                if tx.blocking_send(Ok(buf[..n].to_vec())).is_err() {
                    return Ok(()); // consommateur parti — rien à pousser.
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(()),
            Err(e) => return Err(NetError::ContentDecode(format!("{e}"))),
        }
    }
}

/// Décodeur streamé pour un `coding` — empilable (`Read` →
/// `Read`). `deflate` : choix zlib/raw par inspection des deux
/// premiers octets (signature zlib : `CM=8` en nibble bas +
/// `FCHECK` cohérent) via `BufReader::fill_buf` — l'heuristique
/// remplace le repli « zlib puis raw » du chemin bufferisé,
/// inapplicable en flux non rebobinable.
fn stream_decoder(r: Box<dyn Read + Send>, coding: &str) -> NetResult<Box<dyn Read + Send>> {
    Ok(match coding {
        "gzip" | "x-gzip" => Box::new(flate2::read::GzDecoder::new(r)),
        "br" => Box::new(brotli::Decompressor::new(r, 4096)),
        "deflate" => {
            let mut br = BufReader::new(r);
            let zlib = match br.fill_buf() {
                Ok(h) if h.len() >= 2 => {
                    h[0] & 0x0f == 8 && ((u16::from(h[0]) << 8) | u16::from(h[1])) % 31 == 0
                }
                // Moins de 2 octets visibles : défaut zlib si le
                // premier est cohérent (CM=8), raw sinon.
                Ok(h) => h.first().is_none_or(|b| b & 0x0f == 8),
                Err(_) => true,
            };
            if zlib {
                Box::new(flate2::read::ZlibDecoder::new(br))
            } else {
                Box::new(flate2::read::DeflateDecoder::new(br))
            }
        }
        other => return Err(NetError::ContentDecode(format!("codage inconnu: {other}"))),
    })
}

/// Décode `resp.body` in situ selon ses `Content-Encoding`
/// (séparés par virgule, appliqués en ordre inverse de la liste).
/// Met à jour `Content-Length` et retire `Content-Encoding`.
///
/// # Errors
/// [`NetError::ContentDecode`] sur un codage inconnu ou un flux
/// corrompu, [`NetError::DecodedTooLarge`] si le résultat dépasse
/// `max_decoded`.
pub fn decode_response(resp: &mut Response, max_decoded: usize) -> NetResult<()> {
    // Statuts sans corps (RFC 9110 §15) : un 304 de revalidation
    // porte les en-têtes de la ressource — `Content-Encoding`
    // compris — avec un corps VIDE ; tenter de le décoder cassait
    // la revalidation cache en amont (mesuré terrain : wikipedia
    // 304 gzip → « unexpected end of file » masquant l'entrée).
    if resp.status == 204 || resp.status == 304 || (100..200).contains(&resp.status) {
        return Ok(());
    }
    let Some(ce) = resp.header("content-encoding").map(str::to_string) else {
        return Ok(());
    };
    let codings: Vec<String> = ce
        .split(',')
        .map(|c| c.trim().to_ascii_lowercase())
        .filter(|c| !c.is_empty() && c != "identity")
        .collect();
    let mut truncated = false;
    for coding in codings.iter().rev() {
        let (body, trunc) = decode_body(&resp.body, coding, max_decoded)?;
        resp.body = body;
        truncated |= trunc;
    }
    // Le corps a changé de taille : ajuste les en-têtes pour que
    // l'amont voie une réponse cohérente et déjà décodée.
    resp.headers.retain(|(n, _)| {
        !n.eq_ignore_ascii_case("content-encoding") && !n.eq_ignore_ascii_case("content-length")
    });
    resp.headers
        .push(("Content-Length".to_string(), resp.body.len().to_string()));
    if truncated {
        // Corps incomplet servi en dégradé : interdire sa mise en
        // cache — sinon le cache disque servirait ce partiel comme
        // corps entier jusqu'à expiration (mesuré terrain : le
        // cache stocke le corps DÉCODÉ, sans trace de troncation).
        resp.headers
            .retain(|(n, _)| !n.eq_ignore_ascii_case("cache-control"));
        resp.headers
            .push(("Cache-Control".to_string(), "no-store".to_string()));
    }
    Ok(())
}

/// Décode `body` selon un seul `coding`, borné à `max` octets.
///
/// Troncation en fin de flux (`UnexpectedEof` — connexion coupée
/// avant la fin du flux compressé, mesuré terrain : wikipedia via
/// h2 rend parfois un corps court) : on conserve le **partiel
/// décodé** plutôt que d'échouer — le parseur tag-soup amont est
/// fait pour ça, et Chrome/Firefox rendent pareil. Une corruption
/// en milieu de flux reste une erreur honnête.
fn decode_body(body: &[u8], coding: &str, max: usize) -> NetResult<(Vec<u8>, bool)> {
    let mut out = Vec::with_capacity(body.len().min(max / 2));
    let too_large = match coding {
        "gzip" | "x-gzip" => read_bounded(&mut flate2::read::GzDecoder::new(body), &mut out, max),
        "deflate" => {
            // La spec HTTP dit zlib (RFC 1950), le web réel envoie
            // souvent du raw deflate (RFC 1951) — zlib d'abord,
            // raw en repli (tolérant, jamais crash).
            if let Ok(t) = read_bounded(&mut flate2::read::ZlibDecoder::new(body), &mut out, max) {
                Ok(t)
            } else {
                out.clear();
                read_bounded(&mut flate2::read::DeflateDecoder::new(body), &mut out, max)
            }
        }
        "br" => read_bounded(&mut brotli::Decompressor::new(body, 4096), &mut out, max),
        other => {
            return Err(NetError::ContentDecode(format!("codage inconnu: {other}")));
        }
    };
    match too_large {
        Ok(true) => Err(NetError::DecodedTooLarge(max)),
        Ok(false) => Ok((out, false)),
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof && !out.is_empty() => {
            tracing::warn!(
                coding,
                decoded = out.len(),
                "corps compressé tronqué — partiel décodé conservé"
            );
            Ok((out, true))
        }
        Err(e) => Err(NetError::ContentDecode(format!("{coding}: {e}"))),
    }
}

/// Lit `r` dans `out`, borné à `max` — la décompression peut
/// produire un flux illimité ; on arrête au budget plutôt que
/// d'allouer sans fin. `Ok(true)` = budget dépassé.
fn read_bounded(r: &mut impl Read, out: &mut Vec<u8>, max: usize) -> std::io::Result<bool> {
    let mut chunk = [0u8; 8192];
    loop {
        let n = r.read(&mut chunk)?;
        if n == 0 {
            return Ok(false);
        }
        out.extend_from_slice(&chunk[..n]);
        if out.len() > max {
            return Ok(true);
        }
    }
}
