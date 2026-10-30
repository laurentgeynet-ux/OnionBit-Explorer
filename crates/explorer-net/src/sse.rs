// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Parseur `text/event-stream` (Server-Sent Events, JS-C92) —
//! protocole texte au-dessus d'HTTP : le corps arrive par
//! morceaux, chaque champ `data:`/`event:`/`id:`/`retry:` est
//! accumulé, une ligne vide dispatche l'événement. Tolérant
//! par construction : ligne inconnue → commentaire ignoré,
//! fin de flux en plein champ → événement non dispatché
//! (spec : un événement incomplet est abandonné).

/// Événement SSE complet — un par bloc terminé par une
/// ligne vide.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseEvent {
    /// Charge `data:` — lignes jointes par `\n`, `\n` final
    /// retiré (spec).
    pub data: String,
    /// Champ `event:` — `None` → type `message`.
    pub event: Option<String>,
    /// Champ `id:` — vide = `Last-Event-ID` inchangé.
    pub id: String,
}

/// Parseur incrémental — `feed` reçoit les morceaux du corps
/// streamé et rend les événements complets.
#[derive(Default)]
pub struct SseParser {
    /// Octets en attente d'une ligne complète.
    buf: Vec<u8>,
    /// `data` accumulée pour l'événement courant.
    data: String,
    /// `event` courant.
    event: Option<String>,
    /// `id` courant.
    id: String,
    /// `\r` vu en fin de morceau — `\r\n` coupé entre deux
    /// chunks : le `\n` suivant sera absorbé sans produire
    /// de ligne vide (`\r\n` = un seul terminateur).
    cr_pending: bool,
    /// BOM UTF-8 en tête de flux absorbé une fois (spec) —
    /// `false` tant que le premier octet n'est pas lu.
    started: bool,
}

/// §B-59 — budgets des accumulateurs : un flux qui pousse des
/// octets sans terminateur de ligne gonflerait `buf` à vitesse
/// fil (même signature que la fuite terrain ~1 Go/s), et des
/// champs `data:` sans ligne vide gonfleraient `data`. Au-delà
/// du budget on jette l'état partiel et on repart — jamais
/// d'allocation sans fin.
const MAX_SSE_LINE_BYTES: usize = 1 << 20; // 1 Mio par ligne non terminée
const MAX_SSE_DATA_BYTES: usize = 4 << 20; // 4 Mio de data par événement

impl SseParser {
    /// Nouveau parseur vide.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Alimente le parseur ; rend les événements complets
    /// (zéro ou plusieurs). Décodage UTF-8 lossy (spec :
    /// le stream est UTF-8 — `from_utf8_lossy` borne la
    /// mauvaise donnée au repli, jamais d'échec).
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<SseEvent> {
        if self.started {
            self.buf.extend_from_slice(bytes);
        } else {
            self.started = true;
            // BOM UTF-8 en tête de flux (spec : ignoré).
            let bytes = if bytes.starts_with(&[0xef, 0xbb, 0xbf]) {
                &bytes[3..]
            } else {
                bytes
            };
            self.buf.extend_from_slice(bytes);
        }
        let mut out = Vec::new();
        while let Some(line) = self.take_line() {
            if line.is_empty() {
                out.extend(self.dispatch());
            } else {
                self.field(&line);
            }
        }
        // Reliquat de ligne jamais terminée au-delà du budget →
        // la jeter (dégradé : l'événement en cours est perdu,
        // le flux reprend au prochain terminateur — §B-59).
        if self.buf.len() > MAX_SSE_LINE_BYTES {
            self.buf.clear();
            self.cr_pending = false;
        }
        out
    }

    /// Fin de flux : un éventement sans ligne vide finale
    /// est **abandonné** (spec — dispatch uniquement à la
    /// ligne vide). Rend les événements complets restants
    /// si le flux finit sur `\n\n` exactement à la coupe.
    pub fn finish(&mut self) -> Vec<SseEvent> {
        // Une ligne résiduelle sans terminateur n'est pas
        // un champ complet — on la jette. Il ne peut pas
        // rester d'événement complet non dispatché (le
        // dispatch se fait à la ligne vide, toujours lue).
        self.buf.clear();
        self.data.clear();
        self.event = None;
        self.id.clear();
        Vec::new()
    }

    /// Prend la ligne suivante du tampon (`\n`, `\r\n` ou
    /// `\r` seul comme terminateur — spec) ou `None`.
    fn take_line(&mut self) -> Option<String> {
        let mut i = 0usize;
        while i < self.buf.len() {
            match self.buf[i] {
                b'\n' => {
                    if self.cr_pending && i == 0 {
                        // `\r\n` coupé entre deux chunks : le
                        // `\n` est absorbé, pas une ligne vide.
                        self.cr_pending = false;
                        self.buf.drain(..1);
                        i = 0;
                        continue;
                    }
                    let line = String::from_utf8_lossy(&self.buf[..i]).into_owned();
                    self.buf.drain(..=i);
                    self.cr_pending = false;
                    return Some(line);
                }
                b'\r' => {
                    if i + 1 == self.buf.len() {
                        // Fin de chunk pile après `\r` — le
                        // `\r` est le terminateur (consommé) ;
                        // `cr_pending` fera absorber un `\n`
                        // en tête du prochain morceau.
                        self.cr_pending = true;
                        let line = String::from_utf8_lossy(&self.buf[..i]).into_owned();
                        self.buf.drain(..=i);
                        return Some(line);
                    }
                    let line = String::from_utf8_lossy(&self.buf[..i]).into_owned();
                    // `\r\n` → saute les deux octets ; `\rX`
                    // → `\r` seul est le terminateur.
                    let skip = if self.buf[i + 1] == b'\n' {
                        i + 2
                    } else {
                        i + 1
                    };
                    self.buf.drain(..skip);
                    self.cr_pending = false;
                    return Some(line);
                }
                _ => i += 1,
            }
        }
        None
    }

    /// Traite une ligne-champ : `name: value` (espace
    /// unique après `:` ignoré — spec) ; `:` en tête =
    /// commentaire ; sans `:` = champ à valeur vide.
    fn field(&mut self, line: &str) {
        let (name, value) = match line.split_once(':') {
            Some((n, v)) => (n, v.strip_prefix(' ').unwrap_or(v)),
            None => (line, ""),
        };
        match name {
            "data" => {
                // §B-59 : `data:` accumulé sans ligne vide est
                // borné — une ligne qui ferait dépasser le budget
                // est ignorée (événement tronqué plutôt que flux
                // sans fin).
                if self.data.len() + value.len() < MAX_SSE_DATA_BYTES {
                    self.data.push_str(value);
                    self.data.push('\n');
                }
            }
            "event" => self.event = Some(value.to_string()),
            "id" => self.id = value.to_string(),
            // `retry:` ignoré à cette tranche (pas de
            // reconnexion — borne documentée) ; tout champ
            // inconnu est un commentaire (spec).
            _ => {}
        }
    }

    /// Dispatch à la ligne vide — l'événement n'est émis que
    /// si `data` non vide (spec : sans `data:`, l'événement
    /// est ignoré mais `id:` met quand même à jour).
    fn dispatch(&mut self) -> Option<SseEvent> {
        let ev = if self.data.is_empty() {
            None
        } else {
            let mut data = std::mem::take(&mut self.data);
            data.pop(); // `\n` final de jointure — spec.
            Some(SseEvent {
                data,
                event: self.event.clone(),
                id: self.id.clone(),
            })
        };
        self.event = None;
        ev
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_events() {
        let mut p = SseParser::new();
        let evs = p.feed(b"data: hello\n\ndata: two\ndata: lines\n\n");
        assert_eq!(
            evs,
            vec![
                SseEvent {
                    data: "hello".into(),
                    event: None,
                    id: String::new(),
                },
                SseEvent {
                    data: "two\nlines".into(),
                    event: None,
                    id: String::new(),
                },
            ]
        );
    }

    #[test]
    fn named_and_id() {
        let mut p = SseParser::new();
        let evs = p.feed(b"event: ping\nid: 42\ndata: {}\n\n");
        assert_eq!(
            evs,
            vec![SseEvent {
                data: "{}".into(),
                event: Some("ping".into()),
                id: "42".into(),
            }]
        );
    }

    #[test]
    fn split_across_chunks() {
        let mut p = SseParser::new();
        assert_eq!(p.feed(b"data: hel").len(), 0);
        assert_eq!(p.feed(b"lo\r").len(), 0);
        // `\r\n` coupé : la ligne vide se décode quand `\n`
        // arrive au morceau suivant.
        let evs = p.feed(b"\n\n");
        assert_eq!(evs.len(), 1);
        assert_eq!(evs[0].data, "hello");
    }

    /// §B-59 — un flot sans terminateur de ligne est jeté au
    /// budget au lieu de gonfler `buf` à vitesse fil.
    #[test]
    fn unterminated_line_is_dropped() {
        let mut p = SseParser::new();
        let flood = vec![b'x'; MAX_SSE_LINE_BYTES + 1];
        assert_eq!(p.feed(&flood).len(), 0);
        // Le parseur repart proprement au morceau suivant.
        let evs = p.feed(b"data: ok\n\n");
        assert_eq!(evs.len(), 1);
        assert_eq!(evs[0].data, "ok");
    }

    /// §B-59 — `data:` accumulé sans ligne vide est borné :
    /// la ligne qui dépasserait le budget est ignorée.
    #[test]
    fn giant_event_data_is_bounded() {
        let mut p = SseParser::new();
        let big = format!("data: {}\n", "y".repeat(MAX_SSE_DATA_BYTES + 1024));
        let evs = p.feed(format!("{big}data: tail\n\n").as_bytes());
        assert_eq!(evs.len(), 1);
        assert_eq!(evs[0].data, "tail");
    }

    #[test]
    fn comments_and_retry_ignored() {
        let mut p = SseParser::new();
        let evs = p.feed(b": comment\nretry: 3000\ndata: x\n\n");
        assert_eq!(evs.len(), 1);
        assert_eq!(evs[0].data, "x");
    }

    #[test]
    fn event_without_data_dropped_but_id_kept() {
        let mut p = SseParser::new();
        assert_eq!(p.feed(b"id: 7\n\ndata: y\n\n").len(), 1);
        let mut p2 = SseParser::new();
        let evs = p2.feed(b"id: 7\n\ndata: y\n\n");
        assert_eq!(evs[0].id, "7");
    }

    #[test]
    fn incomplete_tail_abandoned() {
        let mut p = SseParser::new();
        assert_eq!(p.feed(b"data: pending").len(), 0);
        assert_eq!(p.finish().len(), 0);
    }
}
