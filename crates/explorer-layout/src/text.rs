// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Itemisation du texte (ADR-0009) : un nœud texte ne descend
//! jamais au shaping comme chaîne plate — il est segmenté en
//! **runs** par direction bidi (UAX #9), chacun portant ses
//! opportunités de coupure de ligne (UAX #14).
//!
//! Ce module est pur : ni fonte ni I/O ici. Le shaping propre
//! est fait dans [`crate::font`] via `rustybuzz`.

use std::ops::Range;

use unicode_bidi::BidiInfo;
use unicode_linebreak::{linebreaks, BreakOpportunity};

/// Direction d'un run de texte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Gauche-à-droite.
    Ltr,
    /// Droite-à-gauche.
    Rtl,
}

/// Opportunité de coupure à un offset octet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Break {
    /// Offset octet dans le texte source.
    pub offset: usize,
    /// `true` = coupure obligatoire (`\n`, séparateur dur).
    pub mandatory: bool,
}

/// Run de texte itemisé : direction homogène + coupures.
#[derive(Debug, Clone, PartialEq)]
pub struct TextRun {
    /// Plage d'octets dans le texte source.
    pub range: Range<usize>,
    /// Tranche de texte (copie — les runs survivent à leur
    /// source dans la boîte de ligne).
    pub text: String,
    /// Direction du run.
    pub direction: Direction,
    /// Niveau bidi (pair = LTR).
    pub level: u8,
    /// Opportunités de coupure UAX #14 **relatives au run**.
    pub breaks: Vec<Break>,
}

/// Itemise `text` en runs de direction homogène.
///
/// - Segmentation par niveau bidi (UAX #9 via `unicode-bidi`,
///   niveau de base déduit du premier caractère fort) ;
/// - opportunités UAX #14 recalées en offsets relatifs au run ;
/// - texte vide → `Vec` vide ; les sauts de ligne produisent
///   leurs propres coupures obligatoires.
#[must_use]
pub fn itemize(text: &str) -> Vec<TextRun> {
    if text.is_empty() {
        return Vec::new();
    }
    // Coupures UAX #14 sur le texte entier (offsets octets).
    let breaks: Vec<(usize, bool)> = linebreaks(text)
        .map(|(off, kind)| (off, kind == BreakOpportunity::Mandatory))
        .collect();

    let bidi = BidiInfo::new(text, None);
    let mut runs = Vec::new();
    for para in &bidi.paragraphs {
        // `levels` est indexé par offset octet — on découpe en
        // runs contigus de même niveau, dans l'ordre logique.
        let mut start = para.range.start;
        let mut level = bidi.levels.get(start).copied();
        for off in para.range.start + 1..=para.range.end {
            let cur = if off == para.range.end {
                None
            } else {
                bidi.levels.get(off).copied()
            };
            if cur != level {
                let Some(lv) = level else { continue };
                push_run(&mut runs, text, start..off, lv, &breaks);
                start = off;
                level = cur;
            }
        }
    }
    // Dernier filet : bidi vide (texte tout neutre) → run LTR.
    if runs.is_empty() {
        runs.push(TextRun {
            range: 0..text.len(),
            text: text.to_string(),
            direction: Direction::Ltr,
            level: 0,
            breaks: breaks
                .iter()
                .map(|(off, m)| Break {
                    offset: *off,
                    mandatory: *m,
                })
                .collect(),
        });
    }
    runs
}

/// Empile un run si la tranche est valide et non vide.
fn push_run(
    runs: &mut Vec<TextRun>,
    text: &str,
    range: Range<usize>,
    level: unicode_bidi::Level,
    breaks: &[(usize, bool)],
) {
    let Some(slice) = text.get(range.clone()) else {
        return;
    };
    if slice.is_empty() {
        return;
    }
    runs.push(TextRun {
        range: range.clone(),
        text: slice.to_string(),
        direction: if level.is_rtl() {
            Direction::Rtl
        } else {
            Direction::Ltr
        },
        level: level.number(),
        breaks: breaks
            .iter()
            .filter(|(off, _)| *off > range.start && *off <= range.end)
            .map(|(off, m)| Break {
                offset: off - range.start,
                mandatory: *m,
            })
            .collect(),
    });
}

/// Coupure d'un texte en **lignes** selon `max_width`, en
/// greedy sur les opportunités UAX #14 : la ligne est fermée
/// à la dernière opportunité où elle tenait. Les blancs en
/// fin de ligne ne comptent pas dans la mesure (CSS — la
/// tranche est mesurée tronquée à droite). Un mot sans
/// opportunité interne **déborde** plutôt que d'être coupé
/// (`overflow-wrap: normal`) — sémantique par défaut ;
/// garantie de progression stricte sur contenu hostile.
///
/// `measure` renvoie la largeur px d'une tranche — injectée
/// pour rester pur (fonte réelle ou fixture de test).
///
/// Retourne les offsets octets de début de chaque ligne
/// (le premier est toujours 0 ; la fin du texte n'est pas
/// un début de ligne).
#[must_use]
pub fn break_lines(text: &str, max_width: f32, measure: impl Fn(&str) -> f32) -> Vec<usize> {
    let fits = |a: usize, b: usize| {
        // Blancs de fin : la coupure rend cet espace invisible
        // — il ne doit pas décider du reflow.
        text.get(a..b)
            .is_some_and(|s| measure(s.trim_end()) <= max_width)
    };
    let mut starts = vec![0usize];
    let mut line_start = 0usize;
    // Dernière opportunité où [line_start..off] tenait.
    let mut candidate = 0usize;
    for (off, kind) in linebreaks(text) {
        if off <= line_start {
            continue;
        }
        if kind == BreakOpportunity::Mandatory {
            // D'abord résoudre un débordement en cours : la
            // fin de texte est elle-même marquée mandatory
            // par UAX #14 — couper à la dernière opportunité
            // tenable avant de fermer la ligne.
            if !fits(line_start, off) && candidate > line_start {
                starts.push(candidate);
            }
            starts.push(off);
            line_start = off;
            candidate = off;
            continue;
        }
        if fits(line_start, off) {
            candidate = off;
            continue;
        }
        // Déborde : couper à la dernière opportunité tenable.
        if candidate > line_start {
            starts.push(candidate);
            line_start = candidate;
        }
        if fits(line_start, off) {
            candidate = off;
        } else if off > line_start {
            // Aucune opportunité tenable dans le segment :
            // coupure à l'opportunité courante — le mot qui
            // déborde reste entier et overflow (normal).
            starts.push(off);
            line_start = off;
            candidate = off;
        }
    }
    starts.dedup();
    if starts.last().copied() == Some(text.len()) && !text.is_empty() {
        starts.pop();
    }
    starts
}
