// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Fonte fixture embarquée (étape 18) : un TTF minimal
//! **construit en code** — aucune ressource externe, métriques
//! réelles lues par `ttf-parser`/`rustybuzz` (remplace toute
//! mesure fixe approximative, ADR-0009).
//!
//! Caractéristiques : `unitsPerEm` 1000, ascender 800,
//! descender -200, lineGap 200, avance uniforme 500 —
//! glyphes vides (les tests mesurent la géométrie, pas le
//! dessin). `cmap` format 4 : U+0020..U+00FF → glyphe
//! `codepoint - 31`.

// Tous les entiers écrits sont bornés par construction
// (u16/u32 petits garantis) — les `as` sont sûrs mais non
// déductibles par le lint.
#![allow(clippy::cast_possible_truncation)]

const UPEM: u16 = 1000;
const NUM_GLYPHS: u16 = 256;

/// Construit le TTF en mémoire (checksums calculés).
#[must_use]
pub fn fixture_ttf() -> Vec<u8> {
    let tables: Vec<(&'static str, Vec<u8>)> = vec![
        ("cmap", t_cmap()),
        ("glyf", Vec::new()),
        ("head", t_head()),
        ("hhea", t_hhea()),
        ("hmtx", t_hmtx()),
        ("loca", t_loca()),
        ("maxp", t_maxp()),
        ("name", t_name()),
        ("post", t_post()),
    ];
    let n = tables.len() as u16;
    let mut header = Vec::new();
    push_u32(&mut header, 0x0001_0000); // sfnt TrueType
    push_u16(&mut header, n);
    let pow2 = 15 - (n.leading_zeros() as u16);
    push_u16(&mut header, 16 * (1 << pow2)); // searchRange
    push_u16(&mut header, pow2); // entrySelector
    push_u16(&mut header, n * 16 - 16 * (1 << pow2)); // rangeShift

    let dir_len = 12 + tables.len() * 16;
    let mut body = Vec::new();
    let mut records = Vec::new();
    for (tag, data) in &tables {
        let offset = (dir_len + body.len()) as u32;
        records.push((*tag, checksum(data), offset, data.len() as u32));
        body.extend_from_slice(data);
        pad4(&mut body);
    }
    // Répertoire trié par tag (les tables le sont déjà).
    let mut out = header;
    for (tag, sum, off, len) in records {
        out.extend_from_slice(tag.as_bytes());
        push_u32(&mut out, sum);
        push_u32(&mut out, off);
        push_u32(&mut out, len);
    }
    out.extend_from_slice(&body);
    // checkSumAdjustment : somme globale ≡ 0xB1B0AFBA.
    let adj = 0xB1B0_AFBAu32.wrapping_sub(checksum(&out));
    let head_off = find_table_offset(&out, *b"head");
    out[head_off + 8..head_off + 12].copy_from_slice(&adj.to_be_bytes());
    out
}

fn find_table_offset(file: &[u8], tag: [u8; 4]) -> usize {
    let n = u16::from_be_bytes([file[4], file[5]]) as usize;
    for i in 0..n {
        let rec = 12 + i * 16;
        if file[rec..rec + 4] == tag {
            return u32::from_be_bytes([
                file[rec + 8],
                file[rec + 9],
                file[rec + 10],
                file[rec + 11],
            ]) as usize;
        }
    }
    0
}

fn checksum(data: &[u8]) -> u32 {
    let mut sum = 0u32;
    for chunk in data.chunks(4) {
        let mut b = [0u8; 4];
        b[..chunk.len()].copy_from_slice(chunk);
        sum = sum.wrapping_add(u32::from_be_bytes(b));
    }
    sum
}

fn pad4(v: &mut Vec<u8>) {
    while v.len() % 4 != 0 {
        v.push(0);
    }
}

fn push_u16(v: &mut Vec<u8>, x: u16) {
    v.extend_from_slice(&x.to_be_bytes());
}
fn push_i16(v: &mut Vec<u8>, x: i16) {
    v.extend_from_slice(&x.to_be_bytes());
}
fn push_u32(v: &mut Vec<u8>, x: u32) {
    v.extend_from_slice(&x.to_be_bytes());
}
fn push_i64(v: &mut Vec<u8>, x: i64) {
    v.extend_from_slice(&x.to_be_bytes());
}

fn t_head() -> Vec<u8> {
    let mut v = Vec::new();
    push_u32(&mut v, 0x0001_0000); // version
    push_u32(&mut v, 0x0001_0000); // fontRevision
    push_u32(&mut v, 0); // checkSumAdjustment (patché après)
    push_u32(&mut v, 0x5F0F_3CF5); // magic
    push_u16(&mut v, 0); // flags
    push_u16(&mut v, UPEM);
    push_i64(&mut v, 0); // created
    push_i64(&mut v, 0); // modified
    push_i16(&mut v, 0); // xMin
    push_i16(&mut v, 0); // yMin
    push_i16(&mut v, 500); // xMax
    push_i16(&mut v, 800); // yMax
    push_u16(&mut v, 0); // macStyle
    push_u16(&mut v, 8); // lowestRecPPEM
    push_i16(&mut v, 2); // fontDirectionHint
    push_i16(&mut v, 0); // indexToLocFormat : court
    push_i16(&mut v, 0); // glyphDataFormat
    v
}

fn t_hhea() -> Vec<u8> {
    let mut v = Vec::new();
    push_u32(&mut v, 0x0001_0000);
    push_i16(&mut v, 800); // ascender
    push_i16(&mut v, -200); // descender
    push_i16(&mut v, 200); // lineGap
    push_u16(&mut v, 500); // advanceWidthMax
    push_i16(&mut v, 0); // minLSB
    push_i16(&mut v, 0); // minRSB
    push_i16(&mut v, 0); // xMaxExtent
    push_i16(&mut v, 1); // caretSlopeRise
    push_i16(&mut v, 0); // caretSlopeRun
    push_i16(&mut v, 0); // caretOffset
    for _ in 0..4 {
        push_i16(&mut v, 0); // reserved
    }
    push_i16(&mut v, 0); // metricDataFormat
    push_u16(&mut v, NUM_GLYPHS); // numberOfHMetrics
    v
}

fn t_maxp() -> Vec<u8> {
    let mut v = Vec::new();
    push_u32(&mut v, 0x0001_0000);
    push_u16(&mut v, NUM_GLYPHS);
    for _ in 0..13 {
        push_u16(&mut v, 0);
    }
    v
}

fn t_hmtx() -> Vec<u8> {
    let mut v = Vec::new();
    for _ in 0..NUM_GLYPHS {
        push_u16(&mut v, 500); // advance
        push_i16(&mut v, 0); // lsb
    }
    v
}

fn t_loca() -> Vec<u8> {
    // Tous les glyphes vides → offsets tous nuls.
    vec![0u8; (usize::from(NUM_GLYPHS) + 1) * 2]
}

fn t_cmap() -> Vec<u8> {
    let mut v = Vec::new();
    push_u16(&mut v, 0); // version
    push_u16(&mut v, 1); // numTables
    push_u16(&mut v, 3); // platformID Windows
    push_u16(&mut v, 1); // encodingID Unicode BMP
    push_u32(&mut v, 12); // offset du sous-tableau
                          // Format 4 : deux segments — [0x20,0xFF] → cp-31, sentinelle.
    let mut s = Vec::new();
    push_u16(&mut s, 4);
    push_u16(&mut s, 32); // length
    push_u16(&mut s, 0); // language
    push_u16(&mut s, 4); // segCountX2 (2 segments)
    push_u16(&mut s, 4); // searchRange
    push_u16(&mut s, 1); // entrySelector
    push_u16(&mut s, 0); // rangeShift
    push_u16(&mut s, 0x00FF); // endCode[0]
    push_u16(&mut s, 0xFFFF); // endCode[1] sentinelle
    push_u16(&mut s, 0); // reservedPad
    push_u16(&mut s, 0x0020); // startCode[0]
    push_u16(&mut s, 0xFFFF); // startCode[1]
    push_u16(&mut s, 0xFFE1); // idDelta[0] = -31
    push_u16(&mut s, 1); // idDelta[1]
    push_u16(&mut s, 0); // idRangeOffset[0]
    push_u16(&mut s, 0); // idRangeOffset[1]
    v.extend_from_slice(&s);
    v
}

fn t_name() -> Vec<u8> {
    let mut v = Vec::new();
    let name: Vec<u8> = "Fixture"
        .encode_utf16()
        .flat_map(u16::to_be_bytes)
        .collect();
    push_u16(&mut v, 0); // format
    push_u16(&mut v, 1); // count
    push_u16(&mut v, 6 + 12); // stringOffset
    push_u16(&mut v, 3);
    push_u16(&mut v, 1);
    push_u16(&mut v, 0x0409);
    push_u16(&mut v, 1); // family
    push_u16(&mut v, name.len() as u16);
    push_u16(&mut v, 0);
    v.extend_from_slice(&name);
    v
}

fn t_post() -> Vec<u8> {
    let mut v = Vec::new();
    push_u32(&mut v, 0x0003_0000); // format 3.0
    push_u32(&mut v, 0); // italicAngle
    push_i16(&mut v, -100); // underlinePosition
    push_i16(&mut v, 50); // underlineThickness
    push_u32(&mut v, 0); // isFixedPitch
    for _ in 0..4 {
        push_u32(&mut v, 0);
    }
    v
}
