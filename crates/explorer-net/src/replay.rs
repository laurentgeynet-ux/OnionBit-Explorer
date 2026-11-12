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
//! Discriminant POST (N0.1) : une requête à corps non vide
//! laisse aussi `<clé>-<seq>.req` (corps brut). Plusieurs POST
//! vers la même URL (continuations `youtubei`, `next`…) se
//! distinguent par leur corps — au rejeu, l'entrée dont le
//! `.req` égale le corps entrant est servie **quel que soit
//! l'ordre** ; sans correspondance exacte (corps volatil :
//! token/nonce régénéré) ou sans `.req` enregistré, repli à
//! l'ordre d'enregistrement.
//!
//! Limites assumées : sans discriminant de corps le rejeu exige
//! le même ordre de requêtes que l'enregistrement ; épuisé, le
//! dernier `seq` d'une clé se répète (le site en boucle
//! infinie de polling reste servi).

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

/// Désérialise le format [`encode`] : `status\n` + `reason\n`
/// + en-têtes `k\tv` + ligne vide + corps.
///
/// La `reason` peut être **vide** (HTTP/2 n'a pas de
/// phrase) — le premier `\n\n` n'est donc PAS une borne
/// tête/corps fiable : on lit ligne par ligne depuis le
/// début.
fn decode(bytes: &[u8]) -> Option<Response> {
    fn line<'a>(bytes: &'a [u8], pos: &mut usize) -> Option<&'a [u8]> {
        let rel = bytes[*pos..].iter().position(|&b| b == b'\n')?;
        let l = &bytes[*pos..*pos + rel];
        *pos += rel + 1;
        Some(l)
    }
    let mut pos = 0usize;
    let status: u16 = std::str::from_utf8(line(bytes, &mut pos)?)
        .ok()?
        .parse()
        .ok()?;
    let reason = std::str::from_utf8(line(bytes, &mut pos)?)
        .ok()?
        .to_string();
    let mut resolved = None;
    let mut headers = Vec::new();
    loop {
        let l = line(bytes, &mut pos)?;
        if l.is_empty() {
            break;
        }
        let Some((n, v)) = l
            .iter()
            .position(|&b| b == b'\t')
            .map(|i| (&l[..i], &l[i + 1..]))
        else {
            continue;
        };
        let (n, v) = (std::str::from_utf8(n).ok()?, std::str::from_utf8(v).ok()?);
        if n == RESOLVED_KEY {
            resolved = crate::url::Url::parse(v).ok();
        } else {
            headers.push((n.to_string(), v.to_string()));
        }
    }
    Some(Response {
        status,
        reason,
        headers,
        body: bytes[pos..].to_vec(),
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
        // Sidecar `.req` : le corps POST discrimine les réponses
        // partageant `(méthode, url)` au rejeu (cf. doc module).
        if let Some(body) = req.body.as_deref().filter(|b| !b.is_empty()) {
            if let Err(e) = std::fs::write(self.dir.join(format!("{key}-{seq}.req")), body) {
                tracing::warn!(url = %req.url, error = %e, "net.record : corps requête non écrit");
            }
        }
        Ok(resp)
    }
}

/// Une entrée de rejeu : un `.resp` et le `.req` éventuel
/// (corps de la requête enregistrée — discriminant POST).
struct ReplayEntry {
    seq: usize,
    resp: PathBuf,
    req: Option<PathBuf>,
    claimed: bool,
}

/// Transport de rejeu : sert les `<clé>-<seq>.resp` de `dir`.
/// Le dossier est indexé une fois à l'ouverture — un listing
/// par requête serait un I/O par fetch et rayerait le profil
/// des vols rejoués. À l'arrivée d'une requête : match exact
/// par corps `.req` enregistré si la requête porte un corps,
/// sinon premier `seq` non réclamé ; tout réclamé → le dernier
/// se répète (le site en boucle infinie de polling reste
/// servi). Clé inconnue → `NetError::Io(NotFound)` — l'appelant
/// voit un échec réseau honnête, jamais un panic.
pub struct DirTransport {
    /// `préfixe de fichier` (`{slug}-{hash}`) → entrées par
    /// `seq` croissant.
    families: Mutex<HashMap<String, Vec<ReplayEntry>>>,
    hits: AtomicUsize,
    misses: AtomicUsize,
}

impl DirTransport {
    /// Rejoue le contenu de `dir` (produit par
    /// [`RecordingTransport`]) — indexe les `.resp`/`.req` une
    /// fois (ordre `seq` lu sur le suffixe du nom de fichier).
    #[must_use]
    pub fn new(dir: &Path) -> Self {
        let mut families: HashMap<String, Vec<ReplayEntry>> = HashMap::new();
        if let Ok(rd) = std::fs::read_dir(dir) {
            for e in rd.flatten() {
                let name = e.file_name();
                let Some(name) = name.to_str() else { continue };
                let Some(stem) = name.strip_suffix(".resp") else {
                    continue;
                };
                // `{clé}-{seq}` : le dernier tiret sépare le seq
                // (la clé peut en contenir — slug d'URL).
                let Some((prefix, seq)) = stem
                    .rsplit_once('-')
                    .and_then(|(p, s)| s.parse::<usize>().ok().map(|n| (p, n)))
                else {
                    continue;
                };
                let req = {
                    let p = dir.join(format!("{stem}.req"));
                    p.is_file().then_some(p)
                };
                families
                    .entry(prefix.to_string())
                    .or_default()
                    .push(ReplayEntry {
                        seq,
                        resp: e.path(),
                        req,
                        claimed: false,
                    });
            }
        }
        for entries in families.values_mut() {
            entries.sort_by_key(|e| e.seq);
        }
        Self {
            families: Mutex::new(families),
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

    /// Lit + décode le `.resp` d'une entrée.
    fn serve(entry: &ReplayEntry) -> crate::error::NetResult<Response> {
        let bytes = std::fs::read(&entry.resp).map_err(NetError::Io)?;
        decode(&bytes).ok_or_else(|| {
            NetError::InvalidResponse(format!("fixture illisible : {}", entry.resp.display()))
        })
    }
}

#[async_trait]
impl HttpTransport for DirTransport {
    async fn fetch(&self, req: &Request) -> crate::error::NetResult<Response> {
        let key = key_of(req);
        let mut m = self
            .families
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(entries) = m.get_mut(&key) else {
            self.misses.fetch_add(1, Ordering::Relaxed);
            tracing::debug!(url = %req.url, "net.replay : clé absente du vol enregistré");
            return Err(NetError::Io(io::Error::from(io::ErrorKind::NotFound)));
        };
        // 1. Match exact par corps enregistré — seuls les POST
        //    à corps le discriminent (un GET n'a pas de `.req`).
        // 2. Repli : premier seq non réclamé (ordre
        //    d'enregistrement — corps volatil ou absent).
        // 3. Tout réclamé → dernier (polling infini servi).
        let pick = req
            .body
            .as_deref()
            .filter(|b| !b.is_empty())
            .and_then(|b| {
                entries.iter().position(|e| {
                    !e.claimed
                        && e.req
                            .as_ref()
                            .is_some_and(|p| std::fs::read(p).is_ok_and(|rec| rec == b))
                })
            })
            .or_else(|| entries.iter().position(|e| !e.claimed))
            .unwrap_or(entries.len() - 1);
        let e = &mut entries[pick];
        e.claimed = true;
        self.hits.fetch_add(1, Ordering::Relaxed);
        Self::serve(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::url::Url;

    fn req(url: &str) -> Request {
        Request::get(Url::parse(url).expect("url"))
    }

    fn req_post(url: &str, body: &str) -> Request {
        let mut r = Request::get(Url::parse(url).expect("url"));
        r.method = crate::http::Method::Post;
        r.body = Some(body.as_bytes().to_vec());
        r
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

    /// Répond selon le corps de la requête — sert à vérifier
    /// que le rejeu discrimine les POST par leur `.req`.
    struct Stub2;
    #[async_trait]
    impl HttpTransport for Stub2 {
        async fn fetch(&self, r: &Request) -> crate::error::NetResult<Response> {
            let body = match r.body.as_deref() {
                Some(b) if b == b"a" => b"resp-A".to_vec(),
                _ => b"resp-B".to_vec(),
            };
            Ok(Response {
                body,
                ..resp(200, "x")
            })
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

    /// Réponse HTTP/2 : `reason` vide — le fichier commence
    /// `200\n\n` et l'ancien décodeur confondait cette ligne
    /// vide avec la borne tête/corps (`fixture illisible`).
    #[test]
    fn decode_reason_vide_http2() {
        let fixture = b"200\n\ncontent-type\ttext/html\n\n<html/>";
        let r = decode(fixture).expect("decode h2");
        assert_eq!(r.status, 200);
        assert_eq!(r.reason, "");
        assert_eq!(r.body, b"<html/>".as_slice());
        // Aller-retour : `reason: ""` encode le même motif.
        let back = decode(&encode(&r)).expect("round-trip");
        assert_eq!(back.body, r.body);
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

    #[tokio::test]
    async fn replay_post_discrimine_par_corps() {
        // Deux POST même `(méthode, url)` avec corps distincts
        // (continuations `youtubei`) — le rejeu sert la réponse
        // dont le `.req` égale le corps entrant, quel que soit
        // l'ordre des requêtes.
        let dir = std::env::temp_dir().join(format!("replay-post-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let rec = RecordingTransport::wrap(std::sync::Arc::new(Stub2), &dir);
        rec.fetch(&req_post("https://a.b/yt", "a"))
            .await
            .expect("fA");
        rec.fetch(&req_post("https://a.b/yt", "b"))
            .await
            .expect("fB");

        let replay = DirTransport::new(&dir);
        // Ordre inversé de l'enregistrement : B d'abord →
        // discrimination par corps, pas par seq.
        let rb = replay
            .fetch(&req_post("https://a.b/yt", "b"))
            .await
            .expect("rb");
        let ra = replay
            .fetch(&req_post("https://a.b/yt", "a"))
            .await
            .expect("ra");
        assert_eq!(rb.body, b"resp-B");
        assert_eq!(ra.body, b"resp-A");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn replay_corps_volatil_repli_ordre() {
        // Corps jamais revu deux fois (token régénéré au
        // rejeu) : pas de match exact → l'ordre
        // d'enregistrement fait foi, sans casser la nav.
        let dir = std::env::temp_dir().join(format!("replay-vol-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let rec = RecordingTransport::wrap(std::sync::Arc::new(Stub), &dir);
        rec.fetch(&req_post("https://a.b/yt", "tok-1"))
            .await
            .expect("f1");
        rec.fetch(&req_post("https://a.b/yt", "tok-2"))
            .await
            .expect("f2");

        let replay = DirTransport::new(&dir);
        let r1 = replay
            .fetch(&req_post("https://a.b/yt", "tok-3"))
            .await
            .expect("r1");
        let r2 = replay
            .fetch(&req_post("https://a.b/yt", "tok-4"))
            .await
            .expect("r2");
        assert_eq!(r1.body, b"corps r\xC3\xA9el".as_slice());
        assert_eq!(r2.body, r1.body, "seq suivants servis dans l'ordre");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
