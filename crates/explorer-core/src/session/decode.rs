// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Décodage de sous-ressources texte : BOM, labels charset,
//! heuristique `css_charset`, flux incrémental.

use explorer_html::encoding::{decode, decode_windows1252, sniff_encoding, Encoding};

/// Fenêtre de sniffing `<meta charset>` — même borne que
/// `explorer-html` (`META_SCAN_LEN`, privée côté domaine) : la
/// spec WHATWG limite la recherche au premier kilo-octet.
pub(crate) const SNIFF_WINDOW: usize = 1024;

/// Décodeur de document progressif (étape 53) : sniffing
/// WHATWG sur les `SNIFF_WINDOW` premiers octets (ou à EOF),
/// puis décodage incrémental — une séquence multi-octets
/// coupée par une frontière de morceau est conservée en queue
/// plutôt que perdue en `U+FFFD` (le décodage one-shot de
/// `decode_sniffed` est préservé à l'assemblage final).
pub(crate) struct StreamDecoder {
    /// Octets non encore décodés (pré-sniffing ou queue
    /// d'une séquence incomplète).
    pub(crate) buf: Vec<u8>,
    /// Encodage décidé — `None` tant que la fenêtre de
    /// sniffing n'est pas pleine (et pas EOF).
    pub(crate) enc: Option<Encoding>,
}

impl StreamDecoder {
    pub(crate) fn new() -> Self {
        Self {
            buf: Vec::new(),
            enc: None,
        }
    }

    /// Morceau de corps → texte décodé (potentiellement vide
    /// pendant la fenêtre de sniffing).
    pub(crate) fn feed(&mut self, chunk: &[u8]) -> String {
        self.buf.extend_from_slice(chunk);
        if self.enc.is_none() {
            if self.buf.len() < SNIFF_WINDOW {
                return String::new();
            }
            self.decide();
        }
        self.decode_tail(false)
    }

    /// Fin de corps — force la décision si la fenêtre n'était
    /// pas pleine, puis vide la queue.
    pub(crate) fn finish(&mut self) -> String {
        if self.enc.is_none() {
            self.decide();
        }
        self.decode_tail(true)
    }

    /// Sniffing + consommation du BOM (même règle que
    /// `decode_sniffed` : BOM réel seulement).
    pub(crate) fn decide(&mut self) {
        let enc = sniff_encoding(&self.buf);
        let bom = match enc {
            Encoding::Utf8 if self.buf.starts_with(&[0xEF, 0xBB, 0xBF]) => 3,
            Encoding::Utf16Le | Encoding::Utf16Be
                if self.buf.starts_with(&[0xFF, 0xFE]) || self.buf.starts_with(&[0xFE, 0xFF]) =>
            {
                2
            }
            _ => 0,
        };
        self.buf.drain(..bom);
        self.enc = Some(enc);
    }

    /// Décode le maximum décodable ; `last` = fin de flux (la
    /// queue incomplète est alors décodée en perte tolérante).
    pub(crate) fn decode_tail(&mut self, last: bool) -> String {
        let Some(enc) = self.enc else {
            return String::new();
        };
        let buf = std::mem::take(&mut self.buf);
        let (text, rest) = decode_prefix(&buf, enc, last);
        self.buf = rest;
        text
    }
}

/// Préfixe décodable de `buf` selon `enc` — renvoie le texte et
/// la queue d'octets non consommés (séquence incomplète en fin,
/// conservée pour le morceau suivant ; vide quand `last`).
pub(crate) fn decode_prefix(buf: &[u8], enc: Encoding, last: bool) -> (String, Vec<u8>) {
    match enc {
        // Un octet = un codepoint — tout est décodable.
        Encoding::Windows1252 => (decode_windows1252(buf), Vec::new()),
        // Multi-octets : seule la queue d'une séquence coupée en
        // frontière de morceau est conservée.
        Encoding::Utf8 => {
            // Fin de flux : décodage lossy complet — identique au
            // chemin one-shot `decode` (U+FFFD par séquence
            // invalide).
            if last {
                return (String::from_utf8_lossy(buf).into_owned(), Vec::new());
            }
            let (text, rest) = match std::str::from_utf8(buf) {
                Ok(_) => (String::from_utf8_lossy(buf).into_owned(), Vec::new()),
                Err(e) => {
                    let valid = e.valid_up_to();
                    let mut text = String::from_utf8_lossy(&buf[..valid]).into_owned();
                    match e.error_len() {
                        // Séquence valide tronquée en queue —
                        // conservée pour le morceau suivant.
                        None => (text, buf[valid..].to_vec()),
                        // Séquence invalide : `U+FFFD` une fois, le
                        // suffixe repart en queue (une séquence
                        // valide coupée y survivra).
                        Some(bad) => {
                            text.push('\u{FFFD}');
                            (text, buf[valid + bad..].to_vec())
                        }
                    }
                }
            };
            (text, rest)
        }
        Encoding::Utf16Le | Encoding::Utf16Be => {
            // Par paires d'octets ; une paire de surrogates ne
            // doit pas être coupée en frontière de morceau.
            let mut n = buf.len() & !1;
            if !last && n >= 2 {
                let unit = match enc {
                    Encoding::Utf16Le => u16::from_le_bytes([buf[n - 2], buf[n - 1]]),
                    _ => u16::from_be_bytes([buf[n - 2], buf[n - 1]]),
                };
                if (0xD800..=0xDBFF).contains(&unit) {
                    n -= 2; // lead surrogate sans trail — attendu.
                }
            }
            let units: Vec<u16> = buf[..n]
                .chunks_exact(2)
                .map(|c| match enc {
                    Encoding::Utf16Le => u16::from_le_bytes([c[0], c[1]]),
                    _ => u16::from_be_bytes([c[0], c[1]]),
                })
                .collect();
            (String::from_utf16_lossy(&units), buf[n..].to_vec())
        }
    }
}

/// BOM d'une sous-ressource (étape 53b) — `(encodage, octets
/// à sauter)`. Contrairement au sniffer HTML, aucun `<meta>` :
/// un script ou une feuille n'est pas un document.
pub(crate) fn sniff_bom(bytes: &[u8]) -> (Encoding, usize) {
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        (Encoding::Utf8, 3)
    } else if bytes.starts_with(&[0xFE, 0xFF]) {
        (Encoding::Utf16Be, 2)
    } else if bytes.starts_with(&[0xFF, 0xFE]) {
        (Encoding::Utf16Le, 2)
    } else {
        (Encoding::Utf8, 0)
    }
}

/// Label de charset WHATWG → encodage supporté — mêmes
/// mappings que `explorer-html::encoding` (famille latin-1 →
/// `Windows1252`). `None` = label absent/inconnu : la
/// priorité retombe sur l'échelon suivant (BOM, UTF-8).
pub(crate) fn label_encoding(label: &str) -> Option<Encoding> {
    match label
        .trim()
        .trim_matches(['"', '\''])
        .to_ascii_lowercase()
        .as_str()
    {
        "utf-8" | "utf8" | "unicode-1-1-utf-8" => Some(Encoding::Utf8),
        "utf-16" | "utf-16le" | "utf-16l" => Some(Encoding::Utf16Le),
        "utf-16be" => Some(Encoding::Utf16Be),
        "windows-1252" | "iso-8859-1" | "latin1" | "latin-1" | "ascii" | "us-ascii"
        | "iso8859-1" | "iso_8859-1" | "cp1252" => Some(Encoding::Windows1252),
        _ => None,
    }
}

/// Corps de sous-ressource texte (étape 53b) : `charset`
/// explicite → BOM → **UTF-8** — jamais le sniffer HTML ni
/// son repli `windows-1252`, qui corrompt les littéraux UTF-8
/// des scripts réels (mojibake « Lâ€™usage » sur
/// example.com). `charset` = attribut `<script charset>`
/// (classiques uniquement — les modules sont toujours UTF-8)
/// ou label de transport/déclaration CSS. Les octets de BOM
/// sont toujours sautés — ils n'appartiennent pas au texte.
pub(crate) fn decode_subresource(bytes: &[u8], charset: Option<&str>) -> String {
    let (bom_enc, skip) = sniff_bom(bytes);
    let enc = charset.and_then(label_encoding).unwrap_or(bom_enc);
    decode(&bytes[skip..], enc)
}

/// `charset=` du `Content-Type` transport (priorité maximale
/// pour les feuilles, WHATWG §4.4 — étape 53b).
pub(crate) fn transport_charset(headers: &[(String, String)]) -> Option<String> {
    let v = headers
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case("content-type"))?
        .1
        .as_str();
    let pos = v.to_ascii_lowercase().find("charset=")? + 8;
    let raw = v[pos..]
        .trim_start_matches(['"', '\''])
        .split([';', '"', '\'', ' '])
        .next()?
        .trim();
    (!raw.is_empty()).then(|| raw.to_string())
}

/// `@charset "…"` en tête de feuille (étape 53b) — la règle
/// ne compte que tout au début (après BOM éventuel) ; ASCII
/// tolérant sur la casse et les espaces.
pub(crate) fn css_charset(bytes: &[u8]) -> Option<String> {
    let (_, skip) = sniff_bom(bytes);
    let head = &bytes[skip..bytes.len().min(skip + 128)];
    let lower: Vec<u8> = head.iter().map(u8::to_ascii_lowercase).collect();
    let mut i = 0;
    while i < lower.len() && lower[i].is_ascii_whitespace() {
        i += 1;
    }
    let rest = lower.get(i..)?;
    rest.strip_prefix(b"@charset")?;
    let mut j = i + "@charset".len();
    while j < lower.len() && lower[j].is_ascii_whitespace() {
        j += 1;
    }
    let quote = *lower.get(j)?;
    if quote != b'"' && quote != b'\'' {
        return None;
    }
    let start = j + 1;
    let end = lower[start..]
        .iter()
        .position(|&b| b == quote)
        .map(|p| start + p)?;
    let label = String::from_utf8_lossy(&head[start..end]).trim().to_owned();
    (!label.is_empty()).then_some(label)
}

/// Corps de feuille liée (étape 53b) : `charset` transport →
/// BOM → `@charset` → UTF-8 (ordre WHATWG §4.4 — le document
/// référent n'entre pas en jeu : nos feuilles externes sont
/// décodées sur leur propre étiquette).
pub(crate) fn decode_stylesheet(body: &[u8], headers: &[(String, String)]) -> String {
    if let Some(c) = transport_charset(headers) {
        return decode_subresource(body, Some(&c));
    }
    let (enc, skip) = sniff_bom(body);
    if skip > 0 {
        return decode(&body[skip..], enc);
    }
    decode_subresource(body, css_charset(body).as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `StreamDecoder` tient une séquence UTF-8 coupée en
    /// frontière de morceau au lieu de perdre l'octet.
    #[test]
    fn stream_decoder_holds_split_utf8() {
        let mut d = StreamDecoder::new();
        // « é » = C3 A9 — le premier morceau s'arrête après C3.
        // BOM UTF-8 pour fixer l'encodage (le sniffer HTML
        // replie sinon sur windows-1252 — byte 0xC3 = 'Ã'
        // décodable immédiatement).
        let mut head = vec![0xEF, 0xBB, 0xBF, 0x41]; // BOM + 'A'
        head.extend(std::iter::repeat_n(0x20, SNIFF_WINDOW - 5));
        head.push(0xC3);
        assert_eq!(
            d.feed(&head),
            "A".to_string() + &" ".repeat(SNIFF_WINDOW - 5)
        );
        assert_eq!(d.feed(&[0xA9]), "é");
        assert_eq!(d.finish(), "");
    }

    /// UTF-16 : une paire de surrogates coupée attend le
    /// trail au morceau suivant.
    #[test]
    fn stream_decoder_holds_split_surrogate() {
        let mut d = StreamDecoder::new();
        // BOM UTF-16LE + « a » + lead surrogate seul.
        let mut head = vec![0xFF, 0xFE, 0x61, 0x00];
        head.extend(std::iter::repeat_n(0x20, SNIFF_WINDOW - 6).flat_map(|b| [b, 0x00]));
        head.extend([0x3D, 0xD8]);
        let out = d.feed(&head);
        assert!(out.contains('a'), "a décodé, surrogate retenu : {out:?}");
        let out = d.feed(&[0x1E, 0xD8]); // trail — U+1E3D dans les faits
        assert!(!out.is_empty(), "paire complétée au chunk suivant");
    }

    #[test]
    fn bom_and_labels() {
        assert_eq!(sniff_bom(&[0xEF, 0xBB, 0xBF]), (Encoding::Utf8, 3));
        assert_eq!(sniff_bom(&[0xFE, 0xFF, 0x00]), (Encoding::Utf16Be, 2));
        assert_eq!(sniff_bom(b"abc"), (Encoding::Utf8, 0));
        assert_eq!(
            label_encoding(" \"ISO-8859-1\" "),
            Some(Encoding::Windows1252)
        );
        assert_eq!(label_encoding("utf-16BE"), Some(Encoding::Utf16Be));
        assert_eq!(label_encoding("koi8-r"), None);
    }

    /// Ordre WHATWG §4.4 : `charset=` transport bat `@charset`.
    #[test]
    fn stylesheet_prefers_transport_charset() {
        // « é » encodé windows-1252 (0xE9) — en UTF-8 ce serait
        // invalide ; le label transport tranche.
        let body = b"a{content:\"\xE9\"}";
        let h = vec![(
            "content-type".to_string(),
            "text/css; charset=iso-8859-1".to_string(),
        )];
        assert!(decode_stylesheet(body, &h).contains('é'));
        // Sans transport : `@charset` mène.
        let h2 = vec![];
        let css = b"@charset \"windows-1252\"; a{content:\"\xE9\"}";
        assert!(decode_stylesheet(css, &h2).contains('é'));
        // Ni l'un ni l'autre : UTF-8.
        let utf8 = "a{content:\"é\"}".as_bytes();
        assert!(decode_stylesheet(utf8, &h2).contains('é'));
    }

    #[test]
    fn transport_and_css_charset_parsing() {
        let h = vec![(
            "Content-Type".to_string(),
            "text/css; Charset=\"UTF-8\"; x=y".to_string(),
        )];
        assert_eq!(transport_charset(&h).as_deref(), Some("UTF-8"));
        assert_eq!(transport_charset(&[]), None);
        assert_eq!(css_charset(b"@charset 'utf-8';"), Some("utf-8".to_string()));
        assert_eq!(
            css_charset(b"  @CHARSET \"latin1\""),
            Some("latin1".to_string())
        );
        assert_eq!(css_charset(b"body{x:1}"), None);
    }
}
