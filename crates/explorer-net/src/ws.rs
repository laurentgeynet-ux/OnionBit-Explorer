// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Client WebSocket (RFC 6455) — connexion **dédiée**, hors
//! pool HTTP : après le `101 Switching Protocols`, la socket
//! devient exclusive au flux (une par WebSocket, comme un
//! navigateur).
//!
//! - [`WsDial`] : couture injectable — prod = [`RealWsDial`]
//!   (connecteur `h1` + TLS existants), tests = dial simulé
//!   in-process (aucune socket, §10 respecté).
//! - [`WsEvent`]/`WsSend` : messages canalisés — la tâche
//!   driver lit les trames serveur et écrit les trames client.
//! - Codec : masquage client obligatoire, fragmentation
//!   (continuation), ping/pong automatique, handshake close,
//!   trames de contrôle ≤ 125 octets et `FIN` (RFC 6455 §5.5),
//!   `RSV` refusés (aucune extension négociée — `permessage-
//!   deflate` jamais annoncé), taille de message bornée.
//! - Handshake : `Sec-WebSocket-Key` aléatoire,
//!   `Accept` = SHA-1(base64(key)+GUID) vérifié, sous-protocole
//!   négocié (premier de la liste serveur acceptée — spec :
//!   le serveur choisit), redirections **jamais** suivies
//!   (spec : l'ouverture échoue net), `Upgrade`/`Connection`
//!   insensibles à la casse.

use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc::{Receiver, Sender};

use crate::error::{NetError, NetResult};
use crate::h1::Connector;

/// Événement serveur→script — la session le re-route vers
/// `deliver_ws` (multi-coups, même discipline que `EsDone`).
#[derive(Debug, Clone)]
pub enum WsEvent {
    /// `101` validé — `protocol` = sous-protocole négocié
    /// (vide si aucun).
    Open {
        /// `Sec-WebSocket-Protocol` de la réponse.
        protocol: String,
    },
    /// Trame texte (UTF-8 validé côté codec — perte en
    /// décodage → `Error`, spec).
    Text(String),
    /// Trame binaire.
    Binary(Vec<u8>),
    /// Close propre : code + raison du pair (ou le nôtre en
    /// écho — `1000`/`1006` normalisés côté session).
    Close {
        /// Code RFC 6455 §7.4.
        code: u16,
        /// Raison UTF-8.
        reason: String,
    },
    /// Erreur transport/trame — le flux est mort (close
    /// implicite côté livraison).
    Error(String),
}

/// Message script→serveur, enfilé par la session.
#[derive(Debug, Clone)]
pub enum WsSend {
    /// `send(string)` — trame texte.
    Text(String),
    /// `send(ArrayBuffer/Blob)` — trame binaire.
    Binary(Vec<u8>),
    /// `close(code, reason)`.
    Close {
        /// Code RFC 6455 §7.4 (défaut `1000`).
        code: u16,
        /// Raison ≤ 123 octets (bornée à l'émission).
        reason: String,
    },
}

/// Spec d'ouverture — la session a résolu l'URL et assemblé
/// les en-têtes (Origin, Cookie, `Sec-WebSocket-Protocol`).
#[derive(Debug, Clone)]
pub struct WsSpec {
    /// `ws://` ou `wss://`.
    pub secure: bool,
    /// Hôte d'origine.
    pub host: String,
    /// Port effectif.
    pub port: u16,
    /// `resource` (`path?query`, `"/"` par défaut).
    pub resource: String,
    /// En-têtes additionnels (`Origin`, `Cookie`,
    /// `Sec-WebSocket-Protocol` si demandés).
    pub headers: Vec<(String, String)>,
}

/// Couture dialable — `open` rend immédiatement le couple de
/// canaux ; le `WsEvent::Open`/erreur arrive **sur le canal
/// d'événements** (asynchrone comme la spec).
#[async_trait::async_trait]
pub trait WsDial: Send + Sync {
    /// Ouvre un flux WS ; les événements arrivent sur `events`,
    /// les messages sortants partent par `sends`.
    async fn open(
        &self,
        spec: WsSpec,
        events: Sender<WsEvent>,
        sends: Receiver<WsSend>,
    ) -> NetResult<()>;
}

/// Tailles bornées — les trames géantes/hostiles restent
/// sous quota (`SessionConfig` côté session).
#[derive(Debug, Clone)]
pub struct WsLimits {
    /// Charge utile max d'un message assemblé (fragmentation
    /// réunie) — au-delà : `Error` + close 1009.
    pub max_message: usize,
}

impl Default for WsLimits {
    fn default() -> Self {
        Self {
            max_message: 16 * 1024 * 1024,
        }
    }
}

/// Dial réel : connecteur `h1` (TCP + `TlsConnector` composé
/// au-dessus par l'appelant) + handshake + driver de trames.
pub struct RealWsDial {
    connector: Arc<dyn Connector>,
    limits: WsLimits,
}

impl RealWsDial {
    /// `connector` = celui du transport HTTP (même TLS/proxy)
    /// — la socket n'entre **jamais** dans son pool.
    #[must_use]
    pub fn new(connector: Arc<dyn Connector>, limits: WsLimits) -> Self {
        Self { connector, limits }
    }
}

/// Base64 standard — dupliqué du décodeur SRI (petit, stable,
/// pas de crate externe pour 40 lignes).
fn b64_encode(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (u32::from(c[0]) << 16)
            | (u32::from(c.get(1).copied().unwrap_or(0)) << 8)
            | u32::from(c.get(2).copied().unwrap_or(0));
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 {
            T[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if c.len() > 2 {
            T[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

/// `Sec-WebSocket-Accept` attendu — SHA-1(key + GUID) en
/// base64 (RFC 6455 §1.3).
fn accept_for(key_b64: &str) -> String {
    const GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";
    let mut h = ring::digest::Context::new(&ring::digest::SHA1_FOR_LEGACY_USE_ONLY);
    h.update(key_b64.as_bytes());
    h.update(GUID.as_bytes());
    b64_encode(h.finish().as_ref())
}

/// Trame parsée côté serveur (jamais masquée — RFC : le
/// serveur ne masque pas ; recevoir masqué = trame fautive).
#[derive(Debug)]
struct Frame {
    fin: bool,
    opcode: u8,
    payload: Vec<u8>,
}

/// Opcodes (§5.2).
const OP_CONT: u8 = 0;
const OP_TEXT: u8 = 1;
const OP_BIN: u8 = 2;
const OP_CLOSE: u8 = 8;
const OP_PING: u8 = 9;
const OP_PONG: u8 = 10;

/// Parse incrémental serveur→client : `Some((consommé,
/// frame))` ou `None` (données insuffisantes). Erreurs de
/// protocole → `Err` (la session normalise en close 1002).
fn parse_frame(buf: &[u8]) -> Result<Option<(usize, Frame)>, String> {
    if buf.len() < 2 {
        return Ok(None);
    }
    let fin = buf[0] & 0x80 != 0;
    let rsv = buf[0] & 0x70;
    if rsv != 0 {
        return Err("ws: RSV set without negotiated extension".into());
    }
    let opcode = buf[0] & 0x0f;
    let masked = buf[1] & 0x80 != 0;
    if masked {
        return Err("ws: masked server frame".into());
    }
    let mut len = u64::from(buf[1] & 0x7f);
    let mut off = 2;
    if len == 126 {
        if buf.len() < 4 {
            return Ok(None);
        }
        len = u64::from(u16::from_be_bytes([buf[2], buf[3]]));
        off = 4;
    } else if len == 127 {
        if buf.len() < 10 {
            return Ok(None);
        }
        len = u64::from_be_bytes(buf[2..10].try_into().unwrap_or([0; 8]));
        off = 10;
        if len >> 63 != 0 {
            return Err("ws: 64-bit length MSB set".into());
        }
    }
    if opcode >= 8 {
        // Trames de contrôle : FIN obligatoire, ≤ 125 (§5.5).
        if !fin {
            return Err("ws: fragmented control frame".into());
        }
        if len > 125 {
            return Err("ws: control frame > 125 bytes".into());
        }
    } else if !matches!(opcode, OP_CONT | OP_TEXT | OP_BIN) {
        return Err(format!("ws: unknown opcode {opcode}"));
    }
    let plen = usize::try_from(len).map_err(|_| "ws: frame too large".to_string())?;
    if buf.len() < off + plen {
        return Ok(None);
    }
    Ok(Some((
        off + plen,
        Frame {
            fin,
            opcode,
            payload: buf[off..off + plen].to_vec(),
        },
    )))
}

/// Encodage client→serveur : **masqué** (RFC §5.3 — clé
/// 4 octets aléatoire par trame, XOR du payload).
fn encode_frame(opcode: u8, payload: &[u8], fin: bool) -> Vec<u8> {
    let mut mask = [0u8; 4];
    let _ = ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut mask);
    let mut out = Vec::with_capacity(14 + payload.len());
    out.push(if fin { 0x80 | opcode } else { opcode });
    let l = payload.len();
    if l < 126 {
        out.push(0x80 | u8::try_from(l).unwrap_or(0));
    } else if u16::try_from(l).is_ok() {
        out.push(0x80 | 0x7e);
        out.extend_from_slice(&u16::try_from(l).unwrap_or(u16::MAX).to_be_bytes());
    } else {
        out.push(0x80 | 0x7f);
        out.extend_from_slice(&(l as u64).to_be_bytes());
    }
    out.extend_from_slice(&mask);
    for (i, b) in payload.iter().enumerate() {
        out.push(b ^ mask[i % 4]);
    }
    out
}

fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}

#[allow(clippy::too_many_lines)]
#[async_trait::async_trait]
impl WsDial for RealWsDial {
    async fn open(
        &self,
        spec: WsSpec,
        events: Sender<WsEvent>,
        mut sends: Receiver<WsSend>,
    ) -> NetResult<()> {
        let scheme = if spec.secure { "https" } else { "http" };
        let stream = self
            .connector
            .connect(scheme, &spec.host, spec.port)
            .await?;

        // -- handshake HTTP/1.1 Upgrade -------------------------------
        let mut key_raw = [0u8; 16];
        let _ = ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut key_raw);
        let key_b64 = b64_encode(&key_raw);
        let mut req = format!(
            "GET {} HTTP/1.1\r\nHost: {}:{}\r\nUpgrade: websocket\r\n\
             Connection: Upgrade\r\nSec-WebSocket-Key: {}\r\n\
             Sec-WebSocket-Version: 13\r\n",
            spec.resource, spec.host, spec.port, key_b64
        );
        for (k, v) in &spec.headers {
            use std::fmt::Write as _;
            let _ = write!(req, "{k}: {v}\r\n");
        }
        req.push_str("\r\n");

        let (mut rd, mut wr) = tokio::io::split(stream);
        wr.write_all(req.as_bytes()).await.map_err(NetError::Io)?;

        // Lecture des en-têtes sur la moitié read via un
        // adaptateur — `IoStream` est splitté : on lit via
        // `rd` directement.
        let mut buf = Vec::with_capacity(1024);
        let mut chunk = [0u8; 1024];
        let (status, resp_headers, _rest) = loop {
            let n = rd.read(&mut chunk).await.map_err(NetError::Io)?;
            if n == 0 {
                return Err(NetError::InvalidResponse(
                    "ws: closed during handshake".into(),
                ));
            }
            buf.extend_from_slice(&chunk[..n]);
            if buf.len() > 16 * 1024 {
                return Err(NetError::HeadTooLarge(buf.len()));
            }
            if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                let head = String::from_utf8_lossy(&buf[..pos]).to_string();
                let rest = buf.split_off(pos + 4);
                let mut lines = head.split("\r\n");
                let status: u16 = lines
                    .next()
                    .and_then(|l| l.split_whitespace().nth(1))
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
                let headers: Vec<(String, String)> = lines
                    .filter_map(|l| {
                        l.split_once(':')
                            .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
                    })
                    .collect();
                break (status, headers, rest);
            }
        };
        if status != 101 {
            return Err(NetError::InvalidResponse(format!(
                "ws: handshake status {status}"
            )));
        }
        let upgrade_ok =
            header(&resp_headers, "upgrade").is_some_and(|v| v.eq_ignore_ascii_case("websocket"));
        let conn_ok = header(&resp_headers, "connection").is_some_and(|v| {
            v.split(',')
                .any(|t| t.trim().eq_ignore_ascii_case("upgrade"))
        });
        let accept_ok = header(&resp_headers, "sec-websocket-accept")
            .is_some_and(|v| v == accept_for(&key_b64));
        if !(upgrade_ok && conn_ok && accept_ok) {
            return Err(NetError::InvalidResponse("ws: invalid 101 response".into()));
        }
        let protocol = header(&resp_headers, "sec-websocket-protocol")
            .unwrap_or("")
            .to_string();
        let _ = events.send(WsEvent::Open { protocol }).await;

        // -- driver de trames -----------------------------------------
        let limits = self.limits.clone();
        let mut pending_msg: Vec<u8> = Vec::new();
        let mut msg_is_text = false;
        let mut rbuf = Vec::with_capacity(4096);
        loop {
            tokio::select! {
                // Envoi client → trame masquée.
                send = sends.recv() => {
                    let Some(m) = send else { break };
                    let (op, payload) = match m {
                        WsSend::Text(t) => (OP_TEXT, t.into_bytes()),
                        WsSend::Binary(b) => (OP_BIN, b),
                        WsSend::Close { code, reason } => {
                            let mut p = Vec::with_capacity(125);
                            p.extend_from_slice(&code.to_be_bytes());
                            p.extend_from_slice(reason.as_bytes().iter().take(123).copied().collect::<Vec<_>>().as_slice());
                            let frame = encode_frame(OP_CLOSE, &p, true);
                            if wr.write_all(&frame).await.is_err() { break; }
                            // RFC : on attend le close du pair
                            // (timeout court), puis on termine.
                            let _ = tokio::time::timeout(
                                std::time::Duration::from_secs(2),
                                rd.read(&mut chunk),
                            ).await;
                            break;
                        }
                    };
                    let frame = encode_frame(op, &payload, true);
                    if wr.write_all(&frame).await.is_err() { break; }
                }
                // Réception serveur → trames démasquées.
                n = rd.read(&mut chunk) => {
                    match n {
                        Ok(0) => {
                            let _ = events.send(WsEvent::Error(
                                "ws: socket closed".into(),
                            )).await;
                            break;
                        }
                        Err(e) => {
                            let _ = events.send(WsEvent::Error(
                                format!("ws: io {e}"),
                            )).await;
                            break;
                        }
                        Ok(n) => {
                            rbuf.extend_from_slice(&chunk[..n]);
                            loop {
                                match parse_frame(&rbuf) {
                                    Err(e) => {
                                        let _ = events.send(WsEvent::Error(e)).await;
                                        return Ok(());
                                    }
                                    Ok(None) => break,
                                    Ok(Some((used, f))) => {
                                        rbuf.drain(..used);
                                        match f.opcode {
                                            OP_PING => {
                                                let pong = encode_frame(OP_PONG, &f.payload, true);
                                                if wr.write_all(&pong).await.is_err() {
                                                    return Ok(());
                                                }
                                            }
                                            OP_CLOSE => {
                                                let (code, reason) = if f.payload.len() >= 2 {
                                                    (
                                                        u16::from_be_bytes([f.payload[0], f.payload[1]]),
                                                        String::from_utf8_lossy(&f.payload[2..]).to_string(),
                                                    )
                                                } else {
                                                    (1005, String::new())
                                                };
                                                // Écho close (RFC §5.5.1 —
                                                // le client répond au close
                                                // du pair par un close
                                                // identique).
                                                let echo = encode_frame(OP_CLOSE, &f.payload, true);
                                                let _ = wr.write_all(&echo).await;
                                                let _ = events.send(WsEvent::Close { code, reason }).await;
                                                return Ok(());
                                            }
                                            OP_TEXT | OP_BIN => {
                                                if pending_msg.is_empty() {
                                                    msg_is_text = f.opcode == OP_TEXT;
                                                }
                                                pending_msg.extend_from_slice(&f.payload);
                                                if pending_msg.len() > limits.max_message {
                                                    let _ = events.send(WsEvent::Error(
                                                        "ws: message too large".into(),
                                                    )).await;
                                                    return Ok(());
                                                }
                                                if f.fin {
                                                    let payload = std::mem::take(&mut pending_msg);
                                                    let ev = if msg_is_text {
                                                        match String::from_utf8(payload) {
                                                            Ok(s) => WsEvent::Text(s),
                                                            Err(_) => WsEvent::Error(
                                                                "ws: invalid utf-8 text".into(),
                                                            ),
                                                        }
                                                    } else {
                                                        WsEvent::Binary(payload)
                                                    };
                                                    if events.send(ev).await.is_err() {
                                                        return Ok(());
                                                    }
                                                }
                                            }
                                            _ => {
                                                // Continuation (ou opcode
                                                // inconnu toléré) : fragment
                                                // si un message est ouvert —
                                                // `parse_frame` a déjà rejeté
                                                // les opcodes hors §5.2.
                                                if pending_msg.is_empty() && f.opcode == OP_CONT {
                                                    let _ = events.send(WsEvent::Error(
                                                        "ws: stray continuation".into(),
                                                    )).await;
                                                    return Ok(());
                                                }
                                                if f.opcode != OP_CONT { continue; }
                                                pending_msg.extend_from_slice(&f.payload);
                                                if pending_msg.len() > limits.max_message {
                                                    let _ = events.send(WsEvent::Error(
                                                        "ws: message too large".into(),
                                                    )).await;
                                                    return Ok(());
                                                }
                                                if f.fin {
                                                    let payload = std::mem::take(&mut pending_msg);
                                                    let ev = if msg_is_text {
                                                        match String::from_utf8(payload) {
                                                            Ok(s) => WsEvent::Text(s),
                                                            Err(_) => WsEvent::Error(
                                                                "ws: invalid utf-8 text".into(),
                                                            ),
                                                        }
                                                    } else {
                                                        WsEvent::Binary(payload)
                                                    };
                                                    if events.send(ev).await.is_err() {
                                                        return Ok(());
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        // Fin de socket : la session normalise — si le pair
        // n'a pas clos proprement, `Error` rendra 1006.
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn b64_roundtrip_shape() {
        assert_eq!(b64_encode(b""), "");
        assert_eq!(b64_encode(b"abc"), "YWJj");
        assert_eq!(b64_encode(b"ab"), "YWI=");
        // `Sec-WebSocket-Accept` du vecteur RFC 6455.
        assert_eq!(
            accept_for("dGhlIHNhbXBsZSBub25jZQ=="),
            "s3pPLMBiTxaQ9kYGzzhZRbK+xOo="
        );
    }

    #[test]
    fn parse_simple_frame() {
        // Texte "hi" non masqué.
        let buf = [0x81, 0x02, b'h', b'i'];
        let (used, f) = parse_frame(&buf).unwrap().unwrap();
        assert_eq!(used, 4);
        assert!(f.fin && f.opcode == OP_TEXT && f.payload == b"hi");
    }

    #[test]
    fn parse_needs_more() {
        assert!(parse_frame(&[0x81]).unwrap().is_none());
        // Longueur 126 → header incomplet.
        assert!(parse_frame(&[0x81, 126, 0]).unwrap().is_none());
    }

    #[test]
    fn masked_server_frame_rejected() {
        assert!(parse_frame(&[0x81, 0x82, 0, 0, 0, 0, 0, 0]).is_err());
    }

    #[test]
    fn control_frame_rules() {
        // Ping fragmenté → erreur.
        assert!(parse_frame(&[0x09, 0x00]).is_err());
        // Ping > 125 → erreur.
        let mut buf = vec![0x89, 126];
        buf.extend_from_slice(&126u16.to_be_bytes());
        buf.extend(std::iter::repeat_n(0, 126));
        assert!(parse_frame(&buf).is_err());
    }

    #[test]
    fn encode_masks_payload() {
        let f = encode_frame(OP_TEXT, b"hi", true);
        assert_eq!(f[0], 0x81);
        assert_eq!(f[1] & 0x80, 0x80); // masqué
        assert_eq!(f[1] & 0x7f, 2);
        // Démasquage → payload.
        let mask = [f[2], f[3], f[4], f[5]];
        assert_eq!([f[6] ^ mask[0], f[7] ^ mask[1]], *b"hi");
    }

    // ── Fil réel (JS-C94) — loopback TCP ─────────────────────
    //
    // Le codec est couvert par les unitaires ci-dessus et le
    // contrat JS par les tests session (dial simulé) ; ce test
    // est le seul à exercer `RealWsDial` **contre une socket
    // réelle** : handshake HTTP/1.1 Upgrade → `101` validé,
    // trames masquées client ↔ serveur en clair, echo
    // texte/binaire, `ping` → `pong` auto (constaté côté
    // serveur), close serveur → écho + `WsEvent::Close`.

    /// Trame **client** (masquée) lue côté serveur de test —
    /// `parse_frame` refuse les trames masquées (côté client),
    /// le serveur a besoin de son propre décodeur.
    fn srv_frame(buf: &[u8]) -> Option<(usize, u8, Vec<u8>)> {
        if buf.len() < 2 {
            return None;
        }
        let op = buf[0] & 0x0f;
        let masked = buf[1] & 0x80 != 0;
        let mut len = usize::from(buf[1] & 0x7f);
        let mut off = 2;
        if len == 126 {
            if buf.len() < 4 {
                return None;
            }
            len = usize::from(u16::from_be_bytes([buf[2], buf[3]]));
            off = 4;
        } else if len == 127 {
            if buf.len() < 10 {
                return None;
            }
            len = usize::try_from(u64::from_be_bytes(buf[2..10].try_into().unwrap_or([0; 8])))
                .ok()?;
            off = 10;
        }
        let mask = if masked {
            if buf.len() < off + 4 {
                return None;
            }
            let m = [buf[off], buf[off + 1], buf[off + 2], buf[off + 3]];
            off += 4;
            m
        } else {
            [0u8; 4]
        };
        if buf.len() < off + len {
            return None;
        }
        let mut p = buf[off..off + len].to_vec();
        if masked {
            for (i, b) in p.iter_mut().enumerate() {
                *b ^= mask[i % 4];
            }
        }
        Some((off + len, op, p))
    }

    /// Trame **serveur** (jamais masquée) vers le client.
    fn srv_encode(op: u8, p: &[u8]) -> Vec<u8> {
        let mut out = vec![0x80 | op];
        if p.len() < 126 {
            out.push(u8::try_from(p.len()).unwrap_or(0));
        } else if let Ok(l) = u16::try_from(p.len()) {
            out.push(126);
            out.extend_from_slice(&l.to_be_bytes());
        } else {
            out.push(127);
            out.extend_from_slice(&(p.len() as u64).to_be_bytes());
        }
        out.extend_from_slice(p);
        out
    }

    /// Marqueur de la validation du handshake côté serveur
    /// (pas un opcode — `seen` mélange opcodes et sentinelle).
    const SEEN_HANDSHAKE: u8 = 0xEE;

    /// Lit la requête `Upgrade` et répond `101` (Accept
    /// recalculé, sous-protocole renvoyé si demandé) —
    /// retourne `false` si la socket est à abandonner.
    async fn srv_handshake(
        sock: &mut tokio::net::TcpStream,
        seen: &std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
    ) -> bool {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut req = Vec::new();
        let mut b = [0u8; 512];
        loop {
            let Ok(n) = sock.read(&mut b).await else {
                return false;
            };
            if n == 0 {
                return false;
            }
            req.extend_from_slice(&b[..n]);
            if req.windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
        }
        let head_orig = String::from_utf8_lossy(&req).to_string();
        let head = head_orig.to_lowercase();
        // Forme RFC : upgrade + version + clé présents.
        if head.contains("upgrade: websocket")
            && head.contains("sec-websocket-version: 13")
            && head.contains("connection: upgrade")
        {
            seen.lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(SEEN_HANDSHAKE);
        }
        // La clé est extraite de la tête **originale** — la
        // valeur base64 est sensible à la casse.
        let Some(key) = head_orig.lines().find_map(|l| {
            l.split_once(':')
                .filter(|(k, _)| k.trim().eq_ignore_ascii_case("sec-websocket-key"))
                .map(|(_, v)| v.trim().to_string())
        }) else {
            return false;
        };
        let proto_hdr = if head.contains("sec-websocket-protocol: p1") {
            "Sec-WebSocket-Protocol: p1\r\n"
        } else {
            ""
        };
        let resp = format!(
            "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\n\
             Connection: Upgrade\r\nSec-WebSocket-Accept: {}\r\n{}\r\n",
            accept_for(&key),
            proto_hdr
        );
        sock.write_all(resp.as_bytes()).await.is_ok()
    }

    /// Serveur d'écho minimal : [`srv_handshake`], puis echo
    /// texte/binaire ; après le 2ᵉ écho il émet un `ping`,
    /// attend le `pong` client (constaté dans `seen`) puis
    /// envoie `close{1000,"sv"}` — le client échoit le close
    /// et termine.
    async fn echo_server(
        listener: tokio::net::TcpListener,
        seen: std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
    ) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mark = |op: u8| {
            seen.lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(op);
        };
        let Ok((mut sock, _)) = listener.accept().await else {
            return;
        };
        if !srv_handshake(&mut sock, &seen).await {
            return;
        }
        let mut b = [0u8; 512];
        let mut buf = Vec::new();
        let mut echoes = 0u8;
        let mut pinged = false;
        loop {
            let Ok(n) = sock.read(&mut b).await else {
                return;
            };
            if n == 0 {
                return;
            }
            buf.extend_from_slice(&b[..n]);
            while let Some((used, op, p)) = srv_frame(&buf) {
                buf.drain(..used);
                match op {
                    OP_TEXT | OP_BIN => {
                        if sock.write_all(&srv_encode(op, &p)).await.is_err() {
                            return;
                        }
                        echoes += 1;
                        if echoes == 2 && !pinged {
                            pinged = true;
                            if sock.write_all(&srv_encode(OP_PING, b"pc")).await.is_err() {
                                return;
                            }
                        }
                    }
                    OP_PONG => {
                        mark(OP_PONG);
                        let mut pl = Vec::with_capacity(4);
                        pl.extend_from_slice(&1000u16.to_be_bytes());
                        pl.extend_from_slice(b"sv");
                        let _ = sock.write_all(&srv_encode(OP_CLOSE, &pl)).await;
                        return;
                    }
                    OP_PING => {
                        mark(OP_PING);
                        if sock.write_all(&srv_encode(OP_PONG, &p)).await.is_err() {
                            return;
                        }
                    }
                    OP_CLOSE => {
                        mark(OP_CLOSE);
                        let _ = sock.write_all(&srv_encode(OP_CLOSE, &p)).await;
                        return;
                    }
                    _ => {}
                }
            }
        }
    }

    /// `open` → `Open{p1}` → echo `Text`/`Binary` → le serveur
    /// émet `ping` (le `pong` client est constaté côté
    /// serveur) puis `close{1000,"sv"}` → `WsEvent::Close` +
    /// `open` retourne `Ok`. Le handshake validé (upgrade,
    /// version, clé) est marqué `SEEN_HANDSHAKE`.
    #[tokio::test]
    async fn real_dial_loopback_lifecycle() {
        use crate::h1::TcpConnector;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        tokio::spawn(echo_server(listener, std::sync::Arc::clone(&seen)));

        let dial = RealWsDial::new(std::sync::Arc::new(TcpConnector), WsLimits::default());
        let (ev_tx, mut ev_rx) = tokio::sync::mpsc::channel::<WsEvent>(16);
        let (send_tx, send_rx) = tokio::sync::mpsc::channel::<WsSend>(16);
        let spec = WsSpec {
            secure: false,
            host: "127.0.0.1".into(),
            port,
            resource: "/x".into(),
            headers: vec![("Sec-WebSocket-Protocol".into(), "p1".into())],
        };
        let h = tokio::spawn(async move { dial.open(spec, ev_tx, send_rx).await });

        // Handshake `101` + sous-protocole négocié.
        match ev_rx.recv().await {
            Some(WsEvent::Open { protocol }) => assert_eq!(protocol, "p1"),
            other => panic!("open attendu : {other:?}"),
        }
        // Echo texte puis binaire (les deux opcodes données).
        send_tx.send(WsSend::Text("hi".into())).await.expect("send");
        match ev_rx.recv().await {
            Some(WsEvent::Text(t)) => assert_eq!(t, "hi"),
            other => panic!("echo texte : {other:?}"),
        }
        send_tx
            .send(WsSend::Binary(vec![1, 2, 3]))
            .await
            .expect("send");
        match ev_rx.recv().await {
            Some(WsEvent::Binary(b)) => assert_eq!(b, [1, 2, 3]),
            other => panic!("echo binaire : {other:?}"),
        }
        // Le serveur a clos après son ping : `close` livré.
        match ev_rx.recv().await {
            Some(WsEvent::Close { code, reason }) => {
                assert_eq!((code, reason.as_str()), (1000, "sv"));
            }
            other => panic!("close attendu : {other:?}"),
        }
        h.await.expect("join").expect("open ok");
        let seen = seen
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert!(
            seen.contains(&SEEN_HANDSHAKE),
            "handshake RFC vu : {seen:?}"
        );
        assert!(seen.contains(&OP_PONG), "pong auto du client vu : {seen:?}");
    }

    /// Close **initié par le client** (chemin `ws.close()`) :
    /// `open` écrit le close masqué, le serveur l'échoit, le
    /// flux termine `Ok` — la livraison `Close{1000}` reste
    /// la normalisation côté session (pas d'événement pour
    /// notre propre close — comportement visé).
    #[tokio::test]
    async fn real_dial_loopback_client_close() {
        use crate::h1::TcpConnector;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        tokio::spawn(echo_server(listener, std::sync::Arc::clone(&seen)));

        let dial = RealWsDial::new(std::sync::Arc::new(TcpConnector), WsLimits::default());
        let (ev_tx, mut ev_rx) = tokio::sync::mpsc::channel::<WsEvent>(16);
        let (send_tx, send_rx) = tokio::sync::mpsc::channel::<WsSend>(16);
        let spec = WsSpec {
            secure: false,
            host: "127.0.0.1".into(),
            port,
            resource: "/x".into(),
            headers: Vec::new(),
        };
        let h = tokio::spawn(async move { dial.open(spec, ev_tx, send_rx).await });
        assert!(matches!(ev_rx.recv().await, Some(WsEvent::Open { .. })));
        send_tx
            .send(WsSend::Close {
                code: 1000,
                reason: "bye".into(),
            })
            .await
            .expect("send close");
        // Le serveur échoit : `open` termine sans erreur —
        // aucun `WsEvent` post-close n'est attendu.
        h.await.expect("join").expect("open ok");
        let seen = seen
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert!(
            seen.contains(&OP_CLOSE),
            "close client masqué reçu côté serveur : {seen:?}"
        );
    }
}
