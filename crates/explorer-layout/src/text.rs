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

use unicode_bidi::{BidiInfo, Level};
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

/// Blanc CSS au sens de §16.6.1 (` ` `\t` `\n` `\r` `\x0B`
/// `\x0C`). Les autres espaces Unicode — `\u{A0}` en tête —
/// NE sont PAS collapsibles : ils portent une avance et une
/// encre propres (`&nbsp;` tient une ligne à lui seul).
/// `char::is_whitespace`/`str::trim` avalent `\u{A0}` —
/// interdits pour toute décision de visibilité.
/// R-281 — lettre CJK pour `word-break:keep-all` (Han +
/// extensions, Hiragana, Katakana, Hangul, compat, pleine
/// largeur — mêmes plages que la classification `mixed` du
/// rendu vertical).
#[must_use]
pub fn is_cjk_letter(c: char) -> bool {
    matches!(c as u32,
        0x3000..=0x30FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF
        | 0xAC00..=0xD7AF | 0xF900..=0xFAFF | 0xFF00..=0xFFEF)
}

#[must_use]
pub fn is_css_ws(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r' | '\u{B}' | '\u{C}')
}

/// `trim` restreint aux blancs CSS — préserve `\u{A0}` & co.
#[must_use]
pub fn trim_css_ws(s: &str) -> &str {
    s.trim_matches(is_css_ws)
}

/// Itemise `text` en runs de direction homogène.
///
/// - Segmentation par niveau bidi (UAX #9 via `unicode-bidi`,
///   `para_level` = niveau d'imbrication du paragraphe —
///   `direction: rtl` → [`Level::rtl()`], `ltr` →
///   [`Level::ltr()`] ; `None` = déduction au premier
///   caractère fort, repli `dir=auto` non résolu — R-171) ;
/// - opportunités UAX #14 recalées en offsets relatifs au run ;
/// - texte vide → `Vec` vide ; les sauts de ligne produisent
///   leurs propres coupures obligatoires.
#[must_use]
pub fn itemize(text: &str, para_level: Option<Level>) -> Vec<TextRun> {
    if text.is_empty() {
        return Vec::new();
    }
    // Coupures UAX #14 sur le texte entier (offsets octets).
    let breaks: Vec<(usize, bool)> = linebreaks(text)
        .map(|(off, kind)| (off, kind == BreakOpportunity::Mandatory))
        .collect();

    let bidi = BidiInfo::new(text, para_level);
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

/// R-275 — points de césure automatiques d'un mot
/// (`hyphens:auto`, css-text-3 §6.1) : **splitter
/// syllabique borné** latin (`en`/`fr`/fallback — pas de
/// lexique Liang embarqué, ~80-90 % de points corrects vs
/// dictionnaire — borne documentée). Règles :
/// - `v-cv` : voyelle–consonne–voyelle → coupure AVANT la
///   consonne seule (`ba-na`, `fa-ther`) ;
/// - `vc-cv` : groupe de ≥2 consonnes → coupure après la
///   première (`but-ton`, `par-ler`) **sauf** si le groupe
///   débute par un cluster inséparable — digraphes
///   `ch ph th sh qu gu gn sch` + liquides
///   `bl br cl cr dr fl fr gl gr pl pr tr vr` → coupure
///   avant le groupe (`ta-ble`, `li-bre`) ;
/// - les lettres doublées se coupent (`tt`, `ll`, `ss`…) —
///   `ch`/`ph`/etc. ne sont pas des doublées ;
/// - jamais avant le 3ᵉ caractère ni après l'avant-dernier
///   (`hyphenate-limit-chars` borne davantage en amont) ;
/// - `y` vocalique entre consonnes, `e` muet final non
///   vocalique (FR), diphtongues voyelle+voyelle non
///   coupées. `lang` vide/absent → règles latines génériques.
///
/// Voyelle « pure » pour la césure — latines + non-ASCII
/// (formes accentuées) ; le `y` contextuel est traité à part
/// par [`hyphen_points`].
fn is_vowel_pure(c: char) -> bool {
    matches!(c.to_ascii_lowercase(), 'a' | 'e' | 'i' | 'o' | 'u') || c > '\u{7F}'
}

/// Clusters consonantiques inséparables en début de syllabe
/// (digraphes + groupes à liaison) — `ch`, `ph`, liquides…
const GLUED_CLUSTERS: &[&str] = &[
    "ch", "ph", "th", "sh", "qu", "gu", "gn", "sch", "bl", "br", "cl", "cr", "dr", "fl", "fr",
    "gl", "gr", "pl", "pr", "tr", "vr", "sp", "st", "sk", "sm", "sn", "sw", "sc", "sl", "ps", "pn",
    "pt", "ct",
];

/// Retourne les offsets octets internes de `word` où une
/// coupure avec tiret est admise.
#[must_use]
pub fn hyphen_points(word: &str, lang: &str) -> Vec<usize> {
    // Voyelles latines + `y` contextuel (entre consonnes
    // pour `en` — `mystery` ; toujours voyelle pour `fr` —
    // `style`) + formes accentuées fréquentes.
    let fr = lang.starts_with("fr");
    let is_vowel = |c: char, prev_c: Option<char>, next_c: Option<char>| -> bool {
        let c = c.to_ascii_lowercase();
        if matches!(c, 'a' | 'e' | 'i' | 'o' | 'u') || c > '\u{7F}' {
            return true; // lettre non-ASCII ≈ voyelle/liquide — borne
        }
        if c == 'y' {
            return fr
                || !prev_c.is_some_and(is_vowel_pure)
                    && !next_c.is_some_and(|n| n.is_ascii_alphabetic() && is_vowel_pure(n));
        }
        false
    };
    let chars: Vec<(usize, char)> = word.char_indices().collect();
    let n = chars.len();
    if n < 5 {
        return Vec::new();
    }
    // `e` final muet (FR) : non vocalique s'il termine le mot
    // après consonne (`table` → t-a-b-l-e muet).
    let vowel_at = |i: usize| -> bool {
        let (prev, next) = (
            i.checked_sub(1).map(|j| chars[j].1),
            chars.get(i + 1).map(|&(_, c)| c),
        );
        let c = chars[i].1;
        if fr && i + 1 == n && c == 'e' && prev.is_some_and(|p| !is_vowel_pure(p)) {
            return false;
        }
        is_vowel(c, prev, next)
    };
    let mut out = Vec::new();
    let mut i = 0;
    while i < n {
        if !vowel_at(i) {
            i += 1;
            continue;
        }
        // Prochaine voyelle.
        let mut j = i + 1;
        while j < n && !vowel_at(j) {
            j += 1;
        }
        if j >= n || j == i + 1 {
            i = j.max(i + 1);
            continue;
        }
        // Run consonantique chars[i+1..j].
        let run_start = i + 1;
        let run = &chars[run_start..j];
        if run.len() == 1 {
            // `v-cv` : coupure avant la consonne.
            out.push((run[0].0, run_start));
        } else {
            let run_str: String = run.iter().map(|&(_, c)| c.to_ascii_lowercase()).collect();
            // Doublée (`tt`, `ll`…) → coupure entre les deux ;
            // cluster inséparable en tête → avant le groupe
            // (`ta-ble`, `my-stery`) ; sinon `vc-cv` après la
            // 1ʳᵉ consonne (`but-ton`, `mon-stre`, `par-ler`).
            let doubled = run[0].1.eq_ignore_ascii_case(&run[1].1);
            let glued = GLUED_CLUSTERS.iter().any(|g| run_str.starts_with(g));
            let k = usize::from(!glued || doubled);
            out.push((run[k].0, run_start + k));
        }
        i = j;
    }
    out.into_iter()
        // Pas de coupure avant le 3ᵉ caractère ni après
        // l'avant-dernier (limit-chars affine en aval).
        .filter(|&(_, ci)| ci >= 2 && ci < n - 1)
        .map(|(off, _)| off)
        .collect()
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
            .is_some_and(|s| measure(s.trim_end_matches(is_css_ws)) <= max_width)
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
