// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! URLs `data:` (RFC 2397) — décodage **pur** : aucune I/O,
//! le corps voyage dans l'URL elle-même. Utilisé par
//! `background-image: url(data:…)`, `<img src="data:…">`,
//! `<object data="data:…">` (R-13 — Acid2 s'appuie dessus
//! pour les yeux et le front du visage) et
//! `<video poster="data:…">` (R-34).
//!
//! [`collect_data_images`] parcourt le DOM + la `StyleMap`
//! et renvoie un [`PaintImages`] dont `replaced`/`backgrounds`
//! sont remplis des `data:` décodés — l'appelant les fusionne
//! avec les images réseau.

use std::sync::Arc;

use explorer_css::StyleMap;
use explorer_dom::Dom;

use crate::image::{decode_image, DecodedImage, PaintImages};

/// `data:[<mediatype>][;base64],<payload>` → octets.
/// Tolérant : espaces ASCII ignorés dans le base64, padding
/// `=` optionnel, et la charge peut être **percent-encodée
/// même en mode base64** (Acid2 écrit `%2F`/`%3D`) — on la
/// décode avant d'attaquer les sextets. `None` si la
/// syntaxe ou le payload est cassé — jamais de panic sur
/// une URL hostile.
#[must_use]
pub fn data_url_bytes(url: &str) -> Option<Vec<u8>> {
    let rest = url.strip_prefix("data:")?;
    let comma = rest.find(',')?;
    let meta = &rest[..comma];
    let payload = &rest[comma + 1..];
    let raw = percent_decode(payload.as_bytes());
    if meta.split(';').any(|f| f.eq_ignore_ascii_case("base64")) {
        base64_decode(&raw)
    } else {
        Some(raw)
    }
}

/// `data:` d'image → bitmap RGBA (mêmes bornes que
/// [`decode_image`]). `None` si refusé/cassé.
#[must_use]
pub fn decode_data_url(url: &str, max_px: u32) -> Option<DecodedImage> {
    decode_image(&data_url_bytes(url)?, max_px)
}

/// Collecte les images inline `data:` de la page :
///
/// - `<img src="data:…">` / `<object data="data:…">` /
///   `<video poster="data:…">` → `replaced` (sert aussi le
///   sizing intrinsèque — l'affiche dimensionne la boîte
///   média, R-34) ;
/// - `background-image: url("data:…")` en cascade →
///   `backgrounds`, clés par `NodeId` de l'élément porteur.
///
/// Fonction pure — bornée par `max_px`. Les URLs non-`data:`
/// et les payloads refusés sont ignorés (l'appelant les
/// résout par le réseau).
#[must_use]
pub fn collect_data_images(dom: &Dom, styles: &StyleMap, max_px: u32) -> PaintImages {
    let mut out = PaintImages::default();
    // Marche itérative — un document hostile profond ne
    // déborde pas la pile.
    let mut stack = vec![dom.document()];
    while let Some(node) = stack.pop() {
        let attr = match dom.tag_name(node) {
            Ok("img") => "src",
            Ok("object") => "data",
            Ok("video") => "poster",
            _ => "",
        };
        if !attr.is_empty() {
            if let Ok(Some(src)) = dom.attribute(node, attr) {
                if let Some(img) = decode_data_url(src.trim(), max_px) {
                    out.replaced.insert(node, Arc::new(img));
                }
            }
        }
        if let Ok(children) = dom.children(node) {
            stack.extend(children);
        }
    }
    for (node, style) in styles.iter() {
        if let Some(url) = style
            .get("background-image")
            .and_then(url_arg)
            .filter(|u| u.starts_with("data:"))
        {
            if let Some(img) = decode_data_url(&url, max_px) {
                out.backgrounds.insert(node, Arc::new(img));
            }
        }
        // R-52 — motif « icône colorée » : `mask-image` +
        // `background-color` (Codex/Primer/Material). Le mask
        // est quasi-systématiquement un `data:` SVG inline ;
        // `-webkit-mask-image` est l'orthographe historique,
        // encore fréquente dans le CSS minifié. La même image
        // est aussi indexée **par url** (`masks_url`) — les
        // boîtes pseudo (`::before`/`::after`) n'ont pas de
        // `NodeId` : leur lookup passe par le texte `url(...)`
        // du style calculé.
        for prop in ["mask-image", "-webkit-mask-image"] {
            if let Some(url) = style
                .get(prop)
                .and_then(url_arg)
                .filter(|u| u.starts_with("data:"))
            {
                if let Some(img) = decode_data_url(&url, max_px) {
                    let img = Arc::new(img);
                    out.masks_url.insert(url, img.clone());
                    out.masks.insert(node, img);
                    break;
                }
            }
        }
        // Masks portés par les `::before`/`::after` de `node` —
        // les styles pseudo sont clés sur l'élément origine
        // dans `StyleMap` ; la boîte pseudo n'a pas de
        // `NodeId` propre → index `masks_url` uniquement.
        for which in [
            explorer_css::PseudoElement::Before,
            explorer_css::PseudoElement::After,
        ] {
            if let Some(ps) = styles.pseudo(node, which) {
                if let Some(url) = mask_url(ps).filter(|u| u.starts_with("data:")) {
                    if let Some(img) = decode_data_url(&url, max_px) {
                        out.masks_url.insert(url, Arc::new(img));
                    }
                }
            }
        }
    }
    out
}

/// `mask-image` / `-webkit-mask-image` d'un style calculé →
/// texte `url(...)` déballé (première propriété renseignée) —
/// clé de [`crate::PaintImages::masks_url`], utilisée pour les
/// porteurs sans `NodeId` (pseudos) et les masks fetchés
/// enregistrés par l'appelant.
pub(crate) fn mask_url(style: &explorer_css::ComputedStyle) -> Option<String> {
    for prop in ["mask-image", "-webkit-mask-image"] {
        if let Some(url) = style.get(prop).and_then(url_arg) {
            return Some(url);
        }
    }
    None
}

/// `url(<x>)` → `<x>` déballé : guillemets optionnels ôtés,
/// puis échappements CSS résolus (`\"`→`"`, `\'`→`'`,
/// `\\`→`\`, `\XXXX`→codepoint) — la valeur calculée
/// sérialise `url("…")` en échappant les `"` internes, et
/// sans dééchappement la payload arrive avec des `\`
/// littéraux (un `data:` SVG devient du XML invalide —
/// R-52, motif icône Wikipedia).
pub(crate) fn url_arg(value: &str) -> Option<String> {
    let inner = value.trim().strip_prefix("url(")?.strip_suffix(')')?.trim();
    let inner = inner.trim_matches(|c| c == '"' || c == '\'');
    Some(css_unescape(inner))
}

/// Échappements CSS chaîne/url : `\`+autre → l'autre char ;
/// `\`+1-6 hexdigits (+espaces absorbés) → codepoint si
/// valide. Tolérant : un `\` orphelin en fin disparaît.
fn css_unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        let mut hex = String::new();
        while hex.len() < 6 && it.peek().is_some_and(char::is_ascii_hexdigit) {
            hex.push(it.next().unwrap_or_default());
        }
        if hex.is_empty() {
            if let Some(p) = it.next() {
                out.push(p);
            }
        } else {
            // Espaces absorbés après une séquence hex.
            if it.peek().is_some_and(char::is_ascii_whitespace) {
                it.next();
            }
            if let Some(ch) = u32::from_str_radix(&hex, 16)
                .ok()
                .and_then(char::from_u32)
                .filter(|ch| *ch != '\0')
            {
                out.push(ch);
            }
        }
    }
    out
}

/// `%XX` → octet, in-place sur le reste.
fn percent_decode(s: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        let pair = if s[i] == b'%' && i + 2 < s.len() {
            match (hex(s[i + 1]), hex(s[i + 2])) {
                (Some(h), Some(l)) => Some(h * 16 + l),
                _ => None,
            }
        } else {
            None
        };
        if let Some(byte) = pair {
            out.push(byte);
            i += 3;
        } else {
            out.push(s[i]);
            i += 1;
        }
    }
    out
}

/// Base64 permissif : ignore tout octet hors alphabet
/// (espaces inclus — RFC 4648 tolérance WHATWG), `=` clos
/// la lecture, padding absent toléré.
#[allow(clippy::cast_possible_truncation)]
fn base64_decode(s: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() / 4 * 3);
    let mut acc: u32 = 0;
    let mut bits = 0u32;
    let mut seen = false;
    for &b in s {
        if b == b'=' {
            break;
        }
        let Some(v) = b64_val(b) else { continue };
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        seen = true;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    seen.then_some(out)
}

fn hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn b64_val(b: u8) -> Option<u8> {
    match b {
        b'A'..=b'Z' => Some(b - b'A'),
        b'a'..=b'z' => Some(b - b'a' + 26),
        b'0'..=b'9' => Some(b - b'0' + 52),
        b'+' | b'-' => Some(62),
        b'/' | b'_' => Some(63),
        _ => None,
    }
}
