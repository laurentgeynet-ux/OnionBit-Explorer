// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Rejouabilité réseau : enregistrement (`RecordingTransport`)
//! et rejeu (`DirTransport`) des réponses HTTP sur disque.
//!
//! Usage : un vol réel coûteux (`YouTube`, `pluto.tv`…) est capturé
//! une fois (`EXPLORER_NET_RECORD=<dir>` côté `explorer-cli`),
//! puis rejoué in-process à l'identique
//! (`EXPLORER_NET_REPLAY=<dir>`) — bissections et tests sans
//! réseau, sans site vivant.
//!
//! ## Format sur disque
//!
//! `<dir>/<clé>-<seq>.resp` où `clé` = hash stable de
//! `(méthode, url)` (`SipHash` à clés fixes — déterministe entre
//! processus) préfixé d'un extrait d'URL assaini pour la
//! lisibilité ; `seq` = occurrence N-ième de la même clé
//! (les continuations `youtubei` répètent la même URL — l'ordre
//! des requêtes est reproduit). Un `.resp` =
//! `status`\n`reason`\n`resolved_url`\n en-têtes `nom\tvaleur`\n
//! ligne vide, puis le corps brut (décodé — l'enregistrement se
//! fait après `DecodingTransport`).
//!
//! Limites assumées : le rejeu exige le même ordre de requêtes
//! que l'enregistrement ; épuisé, le dernier `seq` d'une clé se
//! répète (le site en boucle infinie de polling reste servi).

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use async_trait::async_trait;

use crate::error::NetError;
use crate::http::{HttpTransport, Request, Response};

/// Clé de fichier : extrait d'URL lisible + hash stable de
/// `(méthode, url)` — `DefaultHasher::new()` est déterministe
/// (clés fixes), le même fichier est retrouvé au rejeu.
fn key_of(req: &Request) -> String {
    let mut h = DefaultHasher::new();
    req.method.as_str().hash(&mut h);
    req.url.as_str().hash(&mut h);
    let u = req.url.as_str();
    let slug: String = u
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '.' || *c == '-')
        .take(40)
        .collect();
    format!("{slug}-{:016x}", h.finish())
}

/// Pseudo-en-tête porteur de `resolved_url` (l'URL résolue n'a
/// pas de ligne propre — une valeur vide casserait le repère
/// `\n\n` de fin d'en-tête).
const RESOLVED_KEY: &str = "x-explorer-resolved";

/// Sérialise une réponse : en-tête texte, ligne vide, corps brut.
fn encode(resp: &Response) -> Vec<u8> {
    let mut out = format!("{}\n{}\n", resp.status, resp.reason).into_bytes();
    if let Some(u) = &resp.resolved_url {
        out.extend_from_slice(RESOLVED_KEY.as_bytes());
        out.push(b'\t');
        out.extend_from_slice(u.as_str().as_bytes());
        out.push(b'\n');
    }
    for (n, v) in &resp.headers {
        // Les valeurs d'en-tête ne contiennent pas de '\n'
        // (rejetées au parse HTTP) — le format `\t` est sûr.
        out.extend_from_slice(n.as_bytes());
        out.push(b'\t');
        out.extend_from_slice(v.as_bytes());
        out.push(b'\n');
    }
    out.push(b'\n');
    out.extend_from_slice(&resp.body);
    out
}

/// Désérialise le format [`encode`]. Corps = tout après la
/// première ligne vide.
fn decode(bytes: &[u8]) -> Option<Response> {
    let mut head_end = None;
    for (i, w) in bytes.windows(2).enumerate() {
        if w == b"\n\n" {
            head_end = Some(i);
            break;
        }
    }
    let head_end = head_end?;
    let head = std::str::from_utf8(&bytes[..head_end]).ok()?;
    let mut lines = head.lines();
    let status: u16 = lines.next()?.parse().ok()?;
    let reason = lines.next()?.to_string();
    let mut resolved = None;
    let headers = lines
        .filter_map(|l| l.split_once('\t'))
        .filter(|(n, v)| {
            if *n == RESOLVED_KEY {
                resolved = crate::url::Url::parse(v).ok();
                false
            } else {
                true
            }
        })
        .map(|(n, v)| (n.to_string(), v.to_string()))
        .collect();
    Some(Response {
        status,
        reason,
        headers,
        body: bytes[head_end + 2..].to_vec(),
        reused_connection: false,
        resolved_url: resolved,
    })
}

/// Transport enregistreur : délègue au transport interne et
/// écrit chaque réponse réussie dans `dir` sous
/// `<clé>-<seq>.resp` (compteur par clé en mémoire).
///
/// Écriture best-effort : un échec disque n'interrompt pas la
/// navigation (`tracing::warn`), seul le fichier manque.
pub struct RecordingTransport {
    inner: std::sync::Arc<dyn HttpTransport>,
    dir: PathBuf,
    seqs: Mutex<HashMap<String, usize>>,
}

impl RecordingTransport {
    /// Enrobe `inner` ; crée `dir` si absent.
    pub fn wrap(inner: std::sync::Arc<dyn HttpTransport>, dir: &Path) -> Self {
        let _ = std::fs::create_dir_all(dir);
        Self {
            inner,
            dir: dir.to_path_buf(),
            seqs: Mutex::new(HashMap::new()),
        }
    }
}

#[async_trait]
impl HttpTransport for RecordingTransport {
    async fn fetch(&self, req: &Request) -> crate::error::NetResult<Response> {
        let resp = self.inner.fetch(req).await?;
        let key = key_of(req);
        let seq = {
            let mut m = self
                .seqs
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let n = m.entry(key.clone()).or_insert(0);
            *n += 1;
            *n - 1
        };
        let path = self.dir.join(format!("{key}-{seq}.resp"));
        if let Err(e) = std::fs::write(&path, encode(&resp)) {
            tracing::warn!(url = %req.url, error = %e, "net.record : écriture échouée");
        }
        Ok(resp)
    }
}

/// Transport de rejeu : sert les `<clé>-<seq>.resp` de `dir`
/// dans l'ordre d'enregistrement ; un `seq` épuisé répète le
/// dernier fichier connu de la clé. Clé inconnue →
/// `NetError::Io(NotFound)` — l'appelant voit un échec réseau
/// honnête, jamais un panic.
pub struct DirTransport {
    dir: PathBuf,
    seqs: Mutex<HashMap<String, usize>>,
    hits: AtomicUsize,
    misses: AtomicUsize,
}

impl DirTransport {
    /// Rejoue le contenu de `dir` (produit par
    /// [`RecordingTransport`]).
    #[must_use]
    pub fn new(dir: &Path) -> Self {
        Self {
            dir: dir.to_path_buf(),
            seqs: Mutex::new(HashMap::new()),
            hits: AtomicUsize::new(0),
            misses: AtomicUsize::new(0),
        }
    }

    /// Compteurs `(servies, manquantes)` — diagnostic de
    /// couverture du rejeu en fin de vol.
    #[must_use]
    pub fn stats(&self) -> (usize, usize) {
        (
            self.hits.load(Ordering::Relaxed),
            self.misses.load(Ordering::Relaxed),
        )
    }
}

#[async_trait]
impl HttpTransport for DirTransport {
    async fn fetch(&self, req: &Request) -> crate::error::NetResult<Response> {
        let key = key_of(req);
        let seq = {
            let mut m = self
                .seqs
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let n = m.entry(key.clone()).or_insert(0);
            *n += 1;
            *n - 1
        };
        let path = self.dir.join(format!("{key}-{seq}.resp"));
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) if e.kind() == io::ErrorKind::NotFound && seq > 0 => {
                // Polling : répète la dernière réponse enregistrée
                // de cette clé plutôt que de casser la nav.
                std::fs::read(self.dir.join(format!("{key}-{}.resp", seq - 1)))
                    .map_err(NetError::Io)?
            }
            Err(e) => {
                self.misses.fetch_add(1, Ordering::Relaxed);
                tracing::debug!(url = %req.url, "net.replay : clé absente du vol enregistré");
                return Err(NetError::Io(e));
            }
        };
        self.hits.fetch_add(1, Ordering::Relaxed);
        decode(&bytes).ok_or_else(|| {
            NetError::InvalidResponse(format!("fixture illisible : {}", path.display()))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::url::Url;

    fn req(url: &str) -> Request {
        Request::get(Url::parse(url).expect("url"))
    }

    fn resp(status: u16, body: &str) -> Response {
        Response {
            status,
            reason: "OK".into(),
            headers: vec![("content-type".into(), "text/plain".into())],
            body: body.as_bytes().to_vec(),
            reused_connection: false,
            resolved_url: None,
        }
    }

    struct Stub;
    #[async_trait]
    impl HttpTransport for Stub {
        async fn fetch(&self, _r: &Request) -> crate::error::NetResult<Response> {
            Ok(resp(200, "corps réel"))
        }
    }

    #[test]
    fn encode_decode_aller_retour() {
        let r = Response {
            resolved_url: Url::parse("https://a.b/final").ok(),
            ..resp(404, "b\u{0}inaire\u{7f}")
        };
        let back = decode(&encode(&r)).expect("decode");
        assert_eq!(back.status, 404);
        assert_eq!(back.body, b"b\x00inaire\x7f");
        assert_eq!(back.headers[0].0, "content-type");
        assert_eq!(
            back.resolved_url.as_ref().map(Url::as_str).as_deref(),
            Some("https://a.b/final")
        );
    }

    #[tokio::test]
    async fn record_puis_replay_identique_et_sequentiel() {
        let dir = std::env::temp_dir().join(format!("replay-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let rec = RecordingTransport::wrap(std::sync::Arc::new(Stub), &dir);
        rec.fetch(&req("https://a.b/x")).await.expect("fetch1");
        rec.fetch(&req("https://a.b/x")).await.expect("fetch2");
        // Deux occurrences enregistrées.
        let n = std::fs::read_dir(&dir).expect("dir").count();
        assert_eq!(n, 2);

        let replay = DirTransport::new(&dir);
        let r1 = replay.fetch(&req("https://a.b/x")).await.expect("r1");
        let r2 = replay.fetch(&req("https://a.b/x")).await.expect("r2");
        let r3 = replay.fetch(&req("https://a.b/x")).await.expect("r3");
        assert_eq!(r1.status, 200);
        assert_eq!(r2.body, b"corps r\xC3\xA9el".as_slice());
        assert_eq!(r3.body, r2.body, "seq épuisé → répète le dernier");
        // Clé inconnue → échec typé, pas de panic.
        assert!(replay.fetch(&req("https://a.b/absent")).await.is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
