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

use std::io::Read;
use std::sync::Arc;

use async_trait::async_trait;

use crate::error::{NetError, NetResult};
use crate::http::{HttpTransport, Request, Response};

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
    let Some(ce) = resp.header("content-encoding").map(str::to_string) else {
        return Ok(());
    };
    let codings: Vec<String> = ce
        .split(',')
        .map(|c| c.trim().to_ascii_lowercase())
        .filter(|c| !c.is_empty() && c != "identity")
        .collect();
    for coding in codings.iter().rev() {
        resp.body = decode_body(&resp.body, coding, max_decoded)?;
    }
    // Le corps a changé de taille : ajuste les en-têtes pour que
    // l'amont voie une réponse cohérente et déjà décodée.
    resp.headers.retain(|(n, _)| {
        !n.eq_ignore_ascii_case("content-encoding") && !n.eq_ignore_ascii_case("content-length")
    });
    resp.headers
        .push(("Content-Length".to_string(), resp.body.len().to_string()));
    Ok(())
}

/// Décode `body` selon un seul `coding`, borné à `max` octets.
fn decode_body(body: &[u8], coding: &str, max: usize) -> NetResult<Vec<u8>> {
    let mut out = Vec::with_capacity(body.len().min(max / 2));
    let too_large = match coding {
        "gzip" | "x-gzip" => read_bounded(&mut flate2::read::GzDecoder::new(body), &mut out, max)
            .map_err(|e| NetError::ContentDecode(format!("gzip: {e}")))?,
        "deflate" => {
            // La spec HTTP dit zlib (RFC 1950), le web réel envoie
            // souvent du raw deflate (RFC 1951) — zlib d'abord,
            // raw en repli (tolérant, jamais crash).
            if let Ok(t) = read_bounded(&mut flate2::read::ZlibDecoder::new(body), &mut out, max) {
                t
            } else {
                out.clear();
                read_bounded(&mut flate2::read::DeflateDecoder::new(body), &mut out, max)
                    .map_err(|e| NetError::ContentDecode(format!("deflate: {e}")))?
            }
        }
        "br" => read_bounded(&mut brotli::Decompressor::new(body, 4096), &mut out, max)
            .map_err(|e| NetError::ContentDecode(format!("br: {e}")))?,
        other => {
            return Err(NetError::ContentDecode(format!("codage inconnu: {other}")));
        }
    };
    if too_large {
        return Err(NetError::DecodedTooLarge(max));
    }
    Ok(out)
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
