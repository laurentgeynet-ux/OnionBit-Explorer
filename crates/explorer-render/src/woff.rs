// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Décodeur de fontes web (étape 46) : WOFF 1.0 (RFC
//! 3-2012 / W3C) et WOFF 2.0 (W3C REC 2024) → sfnt
//! (TTF/OTF) prêt pour `ttf-parser`.
//!
//! Périmètre honnête (ADR-0016) :
//! - **WOFF1** : complet — tables zlib ou brutes.
//! - **WOFF2** : tables à transform nulle uniquement
//!   (`glyf`/`loca` en version 3, autres en version 0). Les
//!   glyphes prétraités de la spec §5.1 (reconstruction
//!   `glyf`/`loca`/`hmtx`) ne sont pas décodés : la fonte est
//!   refusée et la page retombe sur la famille suivante.
//! - **Collections** (`ttcf`) : refusées.
//!
//! Bornes hostiles : `numTables ≤ 64`, table ≤ 32 Mio,
//! sortie totale ≤ 64 Mio — la décompression passe par des
//! lecteurs plafonnés, jamais `read_to_end` nu.

use std::io::Read;

/// `numTables` maximum accepté (une fonte réelle en a ≤ ~40).
const MAX_TABLES: usize = 64;
/// Taille originale d'une table, plafond anti-bombe.
const MAX_TABLE: usize = 32 * 1024 * 1024;
/// Sortie sfnt totale plafonnée.
const MAX_OUT: usize = 64 * 1024 * 1024;
/// En-tête WOFF1.
const W1_HEADER: usize = 44;
/// En-tête WOFF2.
const W2_HEADER: usize = 48;

const fn be_u16(d: &[u8], i: usize) -> Option<u16> {
    if i + 2 > d.len() {
        return None;
    }
    Some(u16::from_be_bytes([d[i], d[i + 1]]))
}

const fn be_u32(d: &[u8], i: usize) -> Option<u32> {
    if i + 4 > d.len() {
        return None;
    }
    Some(u32::from_be_bytes([d[i], d[i + 1], d[i + 2], d[i + 3]]))
}

fn push_u32(v: &mut Vec<u8>, x: u32) {
    v.extend_from_slice(&x.to_be_bytes());
}
fn push_u16(v: &mut Vec<u8>, x: u16) {
    v.extend_from_slice(&x.to_be_bytes());
}

/// Somme de contrôle sfnt : u32 BE sur la table paddée à 4.
fn checksum(data: &[u8]) -> u32 {
    data.chunks(4).fold(0u32, |acc, c| {
        let mut w = [0u8; 4];
        w[..c.len()].copy_from_slice(c);
        acc.wrapping_add(u32::from_be_bytes(w))
    })
}

/// En-tête sfnt : searchRange/entrySelector/rangeShift.
/// `n` ≤ 64 garanti par l'appelant (`MAX_TABLES`) — les
/// `as` sont bornés par construction.
#[allow(clippy::cast_possible_truncation)]
fn sfnt_header(out: &mut Vec<u8>, flavor: u32, n: usize) {
    let n = n as u32;
    let pow2 = (u32::BITS - n.leading_zeros() - 1) as u16;
    push_u32(out, flavor);
    push_u16(out, n as u16);
    push_u16(out, 16 << pow2);
    push_u16(out, pow2);
    push_u16(out, 16 * n as u16 - (16 << pow2));
}

/// Une table reconstruite (triable par tag pour le répertoire).
struct Table {
    tag: [u8; 4],
    data: Vec<u8>,
}

/// Assemble le sfnt final : répertoire trié par tag,
/// données alignées sur 4 octets, checksums calculés.
/// Tailles/offsets bornés par `MAX_OUT` (64 Mio ≪ u32) —
/// les `as` sont sûrs par construction.
#[allow(clippy::cast_possible_truncation)]
fn assemble(flavor: u32, mut tables: Vec<Table>) -> Option<Vec<u8>> {
    if tables.is_empty() || tables.len() > MAX_TABLES {
        return None;
    }
    tables.sort_unstable_by_key(|t| t.tag);
    let total: usize = tables.iter().map(|t| t.data.len()).sum();
    if total > MAX_OUT {
        return None;
    }
    let dir_off = 12 + 16 * tables.len();
    let mut out = Vec::with_capacity(dir_off + total + 3 * tables.len());
    sfnt_header(&mut out, flavor, tables.len());
    let mut off = dir_off as u32;
    // Passe 1 : répertoire.
    for t in &tables {
        out.extend_from_slice(&t.tag);
        push_u32(&mut out, checksum(&t.data));
        push_u32(&mut out, off);
        push_u32(&mut out, t.data.len() as u32);
        off += t.data.len() as u32;
        off = (off + 3) & !3;
    }
    // Passe 2 : données paddées.
    for t in &tables {
        out.extend_from_slice(&t.data);
        while out.len() % 4 != 0 {
            out.push(0);
        }
    }
    Some(out)
}

/// Signatures sfnt natives acceptées telles quelles.
fn is_sfnt(d: &[u8]) -> bool {
    matches!(
        d.get(..4),
        Some(b"\x00\x01\x00\x00" | b"OTTO" | b"true" | b"typ1")
    )
}

/// Inflate zlib borné à `expect` octets (`None` si le flot ne
/// s'arrête pas exactement à la taille attendue).
fn inflate(d: &[u8], expect: usize) -> Option<Vec<u8>> {
    if expect > MAX_TABLE {
        return None;
    }
    let mut out = Vec::with_capacity(expect.min(1 << 20));
    flate2::read::ZlibDecoder::new(d)
        .take(expect as u64 + 1)
        .read_to_end(&mut out)
        .ok()
        .filter(|_| out.len() == expect)?;
    Some(out)
}

/// WOFF 1.0 → sfnt. Répertoire `wOFF` + tables zlib/brutes.
fn decode_woff1(d: &[u8]) -> Option<Vec<u8>> {
    if d.len() < W1_HEADER {
        return None;
    }
    let flavor = be_u32(d, 4)?;
    let n = be_u16(d, 12)? as usize;
    if n == 0 || n > MAX_TABLES {
        return None;
    }
    if W1_HEADER + 20 * n > d.len() {
        return None;
    }
    let mut tables = Vec::with_capacity(n);
    for i in 0..n {
        let e = W1_HEADER + 20 * i;
        let tag = d.get(e..e + 4)?.try_into().ok()?;
        let (off, comp, orig) = (
            be_u32(d, e + 4)? as usize,
            be_u32(d, e + 8)? as usize,
            be_u32(d, e + 12)? as usize,
        );
        // Offsets bornés au fichier ; comp > orig = entrée
        // incohérente (spec : comp ≤ orig).
        if comp > orig || off.checked_add(comp)? > d.len() || orig > MAX_TABLE {
            return None;
        }
        let body = &d[off..off + comp];
        let data = if comp < orig {
            inflate(body, orig)?
        } else {
            body.to_vec()
        };
        tables.push(Table { tag, data });
    }
    assemble(flavor, tables)
}

/// Tags connus WOFF2 (flags bits 0–5 → tag, spec §4.1).
const KNOWN_TAGS: [[u8; 4]; 63] = [
    *b"cmap", *b"head", *b"hhea", *b"hmtx", *b"maxp", *b"name", *b"OS/2", *b"post", *b"cvt ",
    *b"fpgm", *b"glyf", *b"loca", *b"prep", *b"CFF ", *b"VORG", *b"EBDT", *b"EBLC", *b"gasp",
    *b"hdmx", *b"kern", *b"LTSH", *b"PCLT", *b"VDMX", *b"vhea", *b"vmtx", *b"BASE", *b"GDEF",
    *b"GPOS", *b"GSUB", *b"EBSC", *b"JSTF", *b"MATH", *b"CBDT", *b"CBLC", *b"COLR", *b"CPAL",
    *b"SVG ", *b"sbix", *b"acnt", *b"avar", *b"bdat", *b"bloc", *b"bsln", *b"cvar", *b"fdsc",
    *b"feat", *b"fmtx", *b"fvar", *b"gvar", *b"hsty", *b"just", *b"lcar", *b"mort", *b"morx",
    *b"opbd", *b"prop", *b"trak", *b"Zapf", *b"Silf", *b"Glat", *b"Gloc", *b"Feat", *b"Sill",
];

/// `UIntBase128` (spec §3.1) — `None` sur flot tronqué ou
/// séquence > 5 octets (borne hostile : valeur ≤ 2³²−1).
fn read_base128(d: &[u8], i: &mut usize) -> Option<u32> {
    let mut v: u64 = 0;
    for _ in 0..5 {
        let b = u64::from(*d.get(*i)?);
        *i += 1;
        v = v * 128 + (b & 0x7f);
        if v > u64::from(u32::MAX) {
            return None;
        }
        if b & 0x80 == 0 {
            return u32::try_from(v).ok();
        }
    }
    None
}

/// WOFF 2.0 → sfnt, transform nulle uniquement. Le flot
/// brotli couvre les données des tables dans l'ordre du
/// répertoire ; la spec impose « glyf transformé puis loca »
/// — le cas transformé est refusé proprement.
fn decode_woff2(d: &[u8]) -> Option<Vec<u8>> {
    if d.len() < W2_HEADER {
        return None;
    }
    let flavor = be_u32(d, 4)?;
    if flavor == u32::from_be_bytes(*b"ttcf") {
        return None; // collections hors périmètre (ADR-0016)
    }
    let n = be_u16(d, 12)? as usize;
    let comp_len = be_u32(d, 20)? as usize;
    if n == 0 || n > MAX_TABLES || comp_len == 0 {
        return None;
    }
    // Répertoire : flags [+tag] origLen(B128) [transformLen].
    let mut i = W2_HEADER;
    let mut entries: Vec<([u8; 4], usize)> = Vec::with_capacity(n);
    for _ in 0..n {
        let flags = *d.get(i)?;
        i += 1;
        let tag_idx = flags & 0x3f;
        let transform = flags >> 6;
        let tag: [u8; 4] = if tag_idx == 63 {
            let t = d.get(i..i + 4)?.try_into().ok()?;
            i += 4;
            t
        } else {
            KNOWN_TAGS[tag_idx as usize]
        };
        let orig = read_base128(d, &mut i)? as usize;
        if orig > MAX_TABLE {
            return None;
        }
        let is_glyf_loca = &tag == b"glyf" || &tag == b"loca";
        let null_transform = if is_glyf_loca {
            transform == 3
        } else {
            transform == 0
        };
        if !null_transform {
            let _tl = read_base128(d, &mut i)?;
            return None; // transform non supportée (ADR-0016)
        }
        entries.push((tag, orig));
    }
    // Flot brotli : le reste du fichier jusqu'aux blocs
    // meta/priv éventuels — `totalCompressedSize` fait foi.
    let comp_end = i.checked_add(comp_len)?.min(d.len());
    let comp = d.get(i..comp_end)?;
    let expected: usize = entries.iter().map(|e| e.1).sum();
    if expected > MAX_OUT {
        return None;
    }
    let mut raw = Vec::with_capacity(expected.min(1 << 20));
    brotli::Decompressor::new(comp, 1 << 16)
        .take(expected as u64 + 1)
        .read_to_end(&mut raw)
        .ok()
        .filter(|_| raw.len() == expected)?;
    let mut off = 0usize;
    let mut tables = Vec::with_capacity(n);
    for (tag, len) in entries {
        tables.push(Table {
            tag,
            data: raw.get(off..off + len)?.to_vec(),
        });
        off += len;
    }
    assemble(flavor, tables)
}

/// Décode une fonte web → sfnt. Détection par signature :
/// `wOFF`, `wOF2`, ou sfnt nu (certains CDN servent le TTF
/// brut). `None` = format refusé → la face est ignorée et
/// la famille retombe sur le fallback.
#[must_use]
pub fn decode_font(data: &[u8]) -> Option<Vec<u8>> {
    if data.len() < 12 {
        return None;
    }
    match data.get(..4) {
        Some(b"wOFF") => decode_woff1(data),
        Some(b"wOF2") => decode_woff2(data),
        _ if is_sfnt(data) => Some(data.to_vec()),
        _ => None,
    }
}

#[cfg(test)]
// Les fixtures écrivent des u32 depuis des `len()` —
// valeurs bornées par construction (octets de test).
#[allow(clippy::cast_possible_truncation)]
mod tests {
    use super::*;

    /// Construit un WOFF1 minimal : une table `test` brute.
    fn woff1(body: &[u8], compressed: bool) -> Vec<u8> {
        let tag = b"test";
        let payload = if compressed {
            use std::io::Write;
            let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
            e.write_all(body).ok();
            e.finish().unwrap_or_default()
        } else {
            body.to_vec()
        };
        let mut w = Vec::new();
        push_u32(&mut w, u32::from_be_bytes(*b"wOFF"));
        push_u32(&mut w, 0x0001_0000);
        push_u32(&mut w, 0);
        push_u16(&mut w, 1); // numTables
        push_u16(&mut w, 0);
        push_u32(&mut w, 0); // totalSfntSize
        push_u32(&mut w, 0); // versions
        push_u32(&mut w, 0); // metaOffset
        push_u32(&mut w, 0); // metaLength
        push_u32(&mut w, 0); // metaOrigLength
        push_u32(&mut w, 0); // privOffset
        push_u32(&mut w, 0); // privLength
        w.extend_from_slice(tag);
        push_u32(&mut w, (W1_HEADER + 20) as u32);
        push_u32(&mut w, payload.len() as u32);
        push_u32(&mut w, body.len() as u32);
        push_u32(&mut w, 0);
        w.extend_from_slice(&payload);
        w
    }

    #[test]
    fn woff1_table_brute() {
        let body = b"le corps de la table";
        let sfnt = decode_font(&woff1(body, false)).unwrap();
        assert_eq!(&sfnt[..4], b"\x00\x01\x00\x00");
        assert_eq!(be_u16(&sfnt, 4), Some(1));
        assert_eq!(&sfnt[12..16], b"test");
        assert_eq!(&sfnt[28..28 + body.len()], body.as_slice());
    }

    #[test]
    fn woff1_table_zlib() {
        let body = vec![7u8; 5000];
        let sfnt = decode_font(&woff1(&body, true)).unwrap();
        assert_eq!(&sfnt[12..16], b"test");
        assert_eq!(&sfnt[28..28 + 5000], body.as_slice());
    }

    #[test]
    fn signatures_et_refus() {
        assert!(decode_font(b"").is_none());
        assert!(decode_font(b"pas une fonte").is_none());
        assert!(decode_font(b"wOF2\x00").is_none());
        let mut ttf = b"\x00\x01\x00\x00xxxx".to_vec();
        ttf.resize(20, 0);
        assert_eq!(decode_font(&ttf), Some(ttf));
    }

    #[test]
    fn woff1_tronque_refuse() {
        let mut w = woff1(b"abc", false);
        w.truncate(W1_HEADER + 10);
        assert!(decode_font(&w).is_none());
        // comp > orig → refus.
        let mut bad = woff1(b"abc", false);
        let comp_len = &mut bad[W1_HEADER + 8..W1_HEADER + 12];
        comp_len.copy_from_slice(&99u32.to_be_bytes());
        assert!(decode_font(&bad).is_none());
    }

    #[test]
    fn base128() {
        let mut i = 0;
        assert_eq!(read_base128(&[0x80, 0x80, 0x7f], &mut i), Some(127));
        assert_eq!(read_base128(&[0xff; 6], &mut 0), None);
    }
}
