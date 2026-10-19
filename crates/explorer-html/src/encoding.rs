// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Sniffing d'encodage WHATWG et transcodage vers UTF-8.
//!
//! Ordre WHATWG : BOM d'abord, `<meta charset>` dans les premiers
//! octets ensuite, défaut `windows-1252` (le vrai défaut de la spec
//! HTML pour un document non étiqueté).

/// Encodage reconnu pour un document HTML.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    /// UTF-8 (BOM ou déclaré).
    Utf8,
    /// UTF-16 little-endian (BOM `FF FE`).
    Utf16Le,
    /// UTF-16 big-endian (BOM `FE FF`).
    Utf16Be,
    /// `windows-1252` — défaut WHATWG ; couvre les labels
    /// `latin1`/`iso-8859-1`/`ascii` (la spec les mappe tous ici).
    Windows1252,
}

/// Taille de la fenêtre de sniffing `<meta>` (octets).
const META_SCAN_LEN: usize = 1024;

/// Détecte l'encodage d'un flux d'octets entrant : BOM, puis
/// `<meta charset>` dans les `META_SCAN_LEN` premiers octets,
/// sinon [`Encoding::Windows1252`].
#[must_use]
pub fn sniff_encoding(bytes: &[u8]) -> Encoding {
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return Encoding::Utf8;
    }
    if bytes.starts_with(&[0xFE, 0xFF]) {
        return Encoding::Utf16Be;
    }
    if bytes.starts_with(&[0xFF, 0xFE]) {
        return Encoding::Utf16Le;
    }
    if let Some(label) = sniff_meta_charset(bytes) {
        return label_to_encoding(&label);
    }
    Encoding::Windows1252
}

/// Transcode `bytes` vers `String` selon `encoding` — perte
/// tolérante (`U+FFFD` sur séquences invalides), jamais de panic.
#[must_use]
pub fn decode(bytes: &[u8], encoding: Encoding) -> String {
    match encoding {
        Encoding::Utf8 => String::from_utf8_lossy(bytes).into_owned(),
        Encoding::Utf16Le => decode_utf16(bytes, u16::from_le_bytes),
        Encoding::Utf16Be => decode_utf16(bytes, u16::from_be_bytes),
        Encoding::Windows1252 => decode_windows1252(bytes),
    }
}

/// Sniffing complet : détecte l'encodage puis transcode.
/// Le BOM UTF-8 est consommé (n'apparaît pas dans le texte).
#[must_use]
pub fn decode_sniffed(bytes: &[u8]) -> (String, Encoding) {
    let encoding = sniff_encoding(bytes);
    // Saute le BOM s'il est réellement présent (un encodage UTF-16
    // déclaré par <meta> sans BOM ne perd pas ses 2 premiers octets).
    let body = match encoding {
        Encoding::Utf8 if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) => &bytes[3..],
        Encoding::Utf16Le if bytes.starts_with(&[0xFF, 0xFE]) => &bytes[2..],
        Encoding::Utf16Be if bytes.starts_with(&[0xFE, 0xFF]) => &bytes[2..],
        _ => bytes,
    };
    (decode(body, encoding), encoding)
}

/// Cherche `<meta charset=…>` dans les premiers octets (scan ASCII
/// tolérant : case-insensitive, quotes simples/doubles/nues,
/// `content="…;charset=…"` supporté). WHATWG §prescan.
fn sniff_meta_charset(bytes: &[u8]) -> Option<String> {
    let window = &bytes[..bytes.len().min(META_SCAN_LEN)];
    // ASCII case-insensitive — on travaille en minuscules.
    let lower: Vec<u8> = window.iter().map(u8::to_ascii_lowercase).collect();
    let hay = lower.as_slice();
    let mut pos = 0;
    while let Some(off) = find_subslice(&hay[pos..], b"charset") {
        let start = pos + off;
        // Le mot doit être dans un contexte <meta …> plausible :
        // on exige un '<' non fermé dans les octets précédents.
        if !inside_tag(&hay[..start]) {
            pos = start + 7;
            continue;
        }
        if let Some(label) = read_charset_value(&hay[start + 7..], &window[start + 7..]) {
            return Some(label);
        }
        pos = start + 7;
    }
    None
}

/// `true` si `start` est à l'intérieur d'une balise ouverte
/// (`<meta …>` sans `>` entre les deux) : premier crochet rencontré
/// en remontant — `<` = dedans, `>` = dehors.
fn inside_tag(prefix: &[u8]) -> bool {
    for &b in prefix.iter().rev() {
        match b {
            b'<' => return true,
            b'>' => return false,
            _ => {}
        }
    }
    false
}

/// Après `charset` : `=` optionnellement espacé, puis valeur quotée
/// ou nue — retourne le label ASCII trouvé.
fn read_charset_value(hay: &[u8], raw: &[u8]) -> Option<String> {
    let mut i = 0;
    while i < hay.len() && (hay[i] == b'=' || hay[i].is_ascii_whitespace()) {
        i += 1;
    }
    if i >= hay.len() {
        return None;
    }
    let value = match hay[i] {
        b'"' | b'\'' => {
            let quote = hay[i];
            let start = i + 1;
            let end = hay[start..]
                .iter()
                .position(|&b| b == quote)
                .map_or(hay.len(), |p| start + p);
            &raw[start..end]
        }
        _ => {
            let end = hay[i..]
                .iter()
                .position(|&b| b.is_ascii_whitespace() || b == b';' || b == b'"' || b == b'>')
                .map_or(hay.len(), |p| i + p);
            &raw[i..end]
        }
    };
    let label = String::from_utf8_lossy(value).trim().to_owned();
    if label.is_empty() {
        None
    } else {
        Some(label)
    }
}

/// Mappe un label de charset WHATWG vers l'encodage supporté.
/// Labels inconnus → `Windows1252` (défaut de fait de la spec).
fn label_to_encoding(label: &str) -> Encoding {
    match label.to_ascii_lowercase().as_str() {
        "utf-8" | "utf8" | "unicode-1-1-utf-8" => Encoding::Utf8,
        "utf-16" | "utf-16le" | "utf-16l" => Encoding::Utf16Le,
        "utf-16be" => Encoding::Utf16Be,
        // windows-1252 couvre la famille latin-1/ascii (WHATWG) et
        // sert de repli pour tout label non reconnu.
        _ => Encoding::Windows1252,
    }
}

/// UTF-16 (LE ou BE via `pair`) → String, perte tolérante.
fn decode_utf16(bytes: &[u8], pair: fn([u8; 2]) -> u16) -> String {
    let units: Vec<u16> = bytes.chunks_exact(2).map(|c| pair([c[0], c[1]])).collect();
    String::from_utf16_lossy(&units)
}

/// Table `windows-1252` pour la plage `0x80..=0x9F` (le reste est
/// identique à Latin-1, mappé 1:1 sur les codepoints Unicode).
const WINDOWS_1252_HIGH: [char; 32] = [
    '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8D}', 'Ž', '\u{8F}',
    '\u{90}', '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{9D}', 'ž', 'Ÿ',
];

/// Décodage `windows-1252` — chaque octet est un codepoint
/// (`0x80..=0x9F` via `WINDOWS_1252_HIGH`).
#[must_use]
pub fn decode_windows1252(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|&b| match b {
            0x80..=0x9F => WINDOWS_1252_HIGH[usize::from(b - 0x80)],
            _ => char::from(b),
        })
        .collect()
}

/// Recherche de sous-séquence (naïve — fenêtres ≤ 1 Kio).
fn find_subslice(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}
