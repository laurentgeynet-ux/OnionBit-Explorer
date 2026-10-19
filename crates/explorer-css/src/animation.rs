// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Animations `@keyframes` + `animation-*` (R-10).
//!
//! Le *parsing* des `@keyframes` vit dans le parser
//! ([`crate::parser::AtKind::Keyframes`]) ; ce module couvre
//! le reste du contrat :
//! - [`Keyframes`] — table nommée de frames triées par offset
//!   (`from` = 0, `to` = 1, `N%` / 100), collectée par
//!   `StyleEngine::add_css` ;
//! - [`AnimationDecl`] — un item de la liste `animation` /
//!   des longhands `animation-*` alignés par index (défauts
//!   spec : `0s ease normal none 1`) ;
//! - [`AnimationDecl::phase`] — progression `[0,1]` à
//!   `elapsed` secondes : `delay`, `iteration-count`
//!   (`infinite` inclus), `direction` (`reverse`/`alternate`),
//!   `fill-mode` (forwards = dernière frame figée, backwards =
//!   première frame pendant le délai) ;
//! - [`keyframe_style`] — interpolation des déclarations
//!   entre les deux frames encadrantes, **réutilisant le
//!   moteur d'interpolation des transitions** (étape 45 :
//!   nombres, longueurs même-unité/px, couleurs) — timeline
//!   partagée, cadenceur côté session.
//!
//! Écarts documentés : `steps()` non couvert (comme les
//! transitions), les keyframes multi-sélecteurs (`0%, 50%`)
//! sont dupliquées à chaque offset, une propriété absente
//! d'une frame tombe sur la valeur *calculée* de l'élément
//! (`base` — spec §keyframes implicit 0%/100%).

use crate::transition::{interpolate, parse_duration, parse_timing, Timing};
use crate::{ComputedStyle, Declaration};

/// Table des `@keyframes` d'une feuille/moteur : nom →
/// frames `(offset ∈ [0,1], déclarations)` **triées**, les
/// sélecteurs multi-offset déjà éclatés et `from`/`to`
/// résolus. En collision d'offset, la dernière frame gagne
/// (ordre de feuille — comportement spec).
pub type Keyframes = std::collections::HashMap<String, Vec<(f64, Vec<Declaration>)>>;

/// Sélecteur de frame → offsets fraction : `from` = 0,
/// `to` = 1, `N%` = N/100 ; liste `a, b` éclatée ; offsets
/// hors `[0,100]` ignorés (spec : ils ne s'appliquent pas,
/// la frame survit pour l'ordre de cascade → ici rejetée,
/// écart documenté).
#[must_use]
pub fn keyframe_offsets(sel: &str) -> Vec<f64> {
    sel.split(',')
        .filter_map(|s| {
            let t = s.trim();
            if t.eq_ignore_ascii_case("from") {
                Some(0.0)
            } else if t.eq_ignore_ascii_case("to") {
                Some(1.0)
            } else {
                t.strip_suffix('%')
                    .and_then(|n| n.trim().parse::<f64>().ok())
                    .filter(|n| (0.0..=100.0).contains(n))
                    .map(|n| n / 100.0)
            }
        })
        .collect()
}

/// Enregistre une `@keyframes` dans la table : éclate les
/// multi-offsets, fusionne même-offset (déclarations de la
/// frame la plus tardive écrasent par propriété), trie.
pub fn collect_keyframes(
    table: &mut Keyframes,
    name: String,
    frames: &[(String, Vec<Declaration>)],
) {
    let mut by_offset: Vec<(f64, Vec<Declaration>)> = Vec::new();
    for (sel, decls) in frames {
        for off in keyframe_offsets(sel) {
            match by_offset
                .iter_mut()
                .find(|(o, _)| (*o - off).abs() < f64::EPSILON)
            {
                // Même offset : la frame tardive complète/
                // écrase propriété par propriété (spec cascade).
                Some((_, slot)) => {
                    for d in decls {
                        if let Some(prev) = slot.iter_mut().find(|p| p.name == d.name) {
                            *prev = d.clone();
                        } else {
                            slot.push(d.clone());
                        }
                    }
                }
                None => by_offset.push((off, decls.clone())),
            }
        }
    }
    by_offset.sort_by(|a, b| a.0.total_cmp(&b.0));
    table.insert(name, by_offset);
}

/// Direction `animation-direction`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Direction {
    /// Lecture normale.
    #[default]
    Normal,
    /// Lecture inversée.
    Reverse,
    /// Alterne normal/inversé à chaque itération.
    Alternate,
    /// Alterne inversé/normal à chaque itération.
    AlternateReverse,
}

/// `animation-fill-mode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Fill {
    /// Aucune frame hors de la lecture.
    #[default]
    None,
    /// Dernière frame figée après la fin.
    Forwards,
    /// Première frame pendant le délai.
    Backwards,
    /// Les deux.
    Both,
}

/// Un item `animation` résolu (longhands alignés par index —
/// la liste la plus courte boucle, comme `transition-*`).
#[derive(Debug, Clone)]
pub struct AnimationDecl {
    /// `animation-name` — nom de la `@keyframes` (`"none"`
    /// = inerte, jamais collecté ici).
    pub name: String,
    /// `animation-duration` en secondes (> 0 pour animer).
    pub duration: f64,
    /// `animation-delay` en secondes (négatif = déjà en cours).
    pub delay: f64,
    /// `animation-timing-function` (appliquée entre frames).
    pub timing: Timing,
    /// `animation-iteration-count` — `f64::INFINITY` pour
    /// `infinite`.
    pub iterations: f64,
    /// `animation-direction`.
    pub direction: Direction,
    /// `animation-fill-mode`.
    pub fill: Fill,
}

impl AnimationDecl {
    /// Phase de lecture `[0,1]` à `elapsed` secondes depuis le
    /// démarrage (cadenceur session), `None` si l'animation ne
    /// produit rien : avant le délai sans `backwards`, ou
    /// terminée sans `forwards`.
    #[must_use]
    pub fn phase(&self, elapsed: f64) -> Option<f64> {
        let local = elapsed - self.delay;
        if self.duration <= 0.0 {
            // Durée nulle : figée dès que le délai est passé.
            return (local >= 0.0).then_some(1.0);
        }
        if local < 0.0 {
            // En délai : première frame si fill backwards/both.
            return matches!(self.fill, Fill::Backwards | Fill::Both).then(|| {
                if matches!(
                    self.direction,
                    Direction::Reverse | Direction::AlternateReverse
                ) {
                    1.0
                } else {
                    0.0
                }
            });
        }
        let iter = local / self.duration;
        if iter >= self.iterations {
            // Terminée : dernière frame si fill forwards/both —
            // la lecture figée = fin de la dernière itération
            // (`alternate` à count pair s'achève sur phase 0).
            return match self.fill {
                Fill::Forwards | Fill::Both => {
                    let even = self.iterations.rem_euclid(2.0) < 0.5;
                    Some(match self.direction {
                        Direction::Normal => 1.0,
                        Direction::Reverse => 0.0,
                        Direction::Alternate => f64::from(u8::from(even)),
                        Direction::AlternateReverse => f64::from(u8::from(!even)),
                    })
                }
                Fill::None | Fill::Backwards => None,
            };
        }
        let iter_idx = iter.floor();
        let frac = iter - iter_idx;
        let reversed = match self.direction {
            Direction::Normal => false,
            Direction::Reverse => true,
            Direction::Alternate => iter_idx.rem_euclid(2.0) > 0.5,
            Direction::AlternateReverse => iter_idx.rem_euclid(2.0) < 0.5,
        };
        Some(if reversed { 1.0 - frac } else { frac })
    }
}

impl Default for AnimationDecl {
    fn default() -> Self {
        Self {
            name: "none".to_string(),
            duration: 0.0,
            delay: 0.0,
            timing: Timing::Ease,
            iterations: 1.0,
            direction: Direction::Normal,
            fill: Fill::None,
        }
    }
}

/// Déclarations `animation-*` d'un style calculé : shorthand
/// `animation` (`nom durée timing délai [count] [dir] [fill]`)
/// puis longhands alignés par index. Les items `none` sont
/// conservés (un `none` tardif masque les précédents par
/// index — la session filtre à l'usage).
#[must_use]
pub fn animation_decls(style: &ComputedStyle) -> Vec<AnimationDecl> {
    let decls: Vec<AnimationDecl> = style
        .get("animation")
        .map(|raw| raw.split(',').map(parse_animation_item).collect())
        .unwrap_or_default();
    let longhand = |name: &str| -> Vec<String> {
        style
            .get(name)
            .map(|v| v.split(',').map(|s| s.trim().to_string()).collect())
            .unwrap_or_default()
    };
    // Les longhands alignés par index — la plus courte boucle.
    let lh = Longhands {
        name: longhand("animation-name"),
        dur: longhand("animation-duration"),
        delay: longhand("animation-delay"),
        timing: longhand("animation-timing-function"),
        count: longhand("animation-iteration-count"),
        dir: longhand("animation-direction"),
        fill: longhand("animation-fill-mode"),
    };
    let n = lh
        .all_lens()
        .into_iter()
        .max()
        .unwrap_or(0)
        .max(decls.len());
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let mut d = decls.get(i).cloned().unwrap_or_default();
        if let Some(v) = at(&lh.name, i) {
            d.name.clone_from(v);
        }
        if let Some(v) = at(&lh.dur, i).and_then(|s| parse_duration(s)) {
            d.duration = v;
        }
        if let Some(v) = at(&lh.delay, i).and_then(|s| parse_duration(s)) {
            d.delay = v;
        }
        if let Some(v) = at(&lh.timing, i).and_then(|s| parse_timing(s)) {
            d.timing = v;
        }
        if let Some(v) = at(&lh.count, i) {
            d.iterations = parse_iteration_count(v);
        }
        if let Some(v) = at(&lh.dir, i) {
            d.direction = parse_direction(v);
        }
        if let Some(v) = at(&lh.fill, i) {
            d.fill = parse_fill(v);
        }
        out.push(d);
    }
    out
}

/// `v[i % len]` — la liste boucle (alignement par index spec).
fn at(v: &[String], i: usize) -> Option<&String> {
    v.get(i % v.len().max(1))
}

/// Longhands `animation-*` éclatés en listes — chaque champ
/// boucle sur sa propre longueur à l'alignement par index.
struct Longhands {
    name: Vec<String>,
    dur: Vec<String>,
    delay: Vec<String>,
    timing: Vec<String>,
    count: Vec<String>,
    dir: Vec<String>,
    fill: Vec<String>,
}

impl Longhands {
    fn all_lens(&self) -> [usize; 7] {
        [
            self.name.len(),
            self.dur.len(),
            self.delay.len(),
            self.timing.len(),
            self.count.len(),
            self.dir.len(),
            self.fill.len(),
        ]
    }
}

/// Un item du shorthand : le premier ident non reconnu est
/// le **nom** (ordre spec : `<time>` ×2, mots-clés, puis nom).
fn parse_animation_item(item: &str) -> AnimationDecl {
    let mut d = AnimationDecl::default();
    let mut saw_time = false;
    let mut named = false;
    for tok in item.split_whitespace() {
        if let Some(v) = parse_duration(tok) {
            if saw_time {
                d.delay = v;
            } else {
                d.duration = v;
                saw_time = true;
            }
        } else if let Some(t) = parse_timing(tok) {
            d.timing = t;
        } else if tok.eq_ignore_ascii_case("infinite") {
            d.iterations = f64::INFINITY;
        } else if let Ok(n) = tok.parse::<f64>() {
            d.iterations = n;
        } else {
            match tok.to_ascii_lowercase().as_str() {
                "normal" => d.direction = Direction::Normal,
                "reverse" => d.direction = Direction::Reverse,
                "alternate" => d.direction = Direction::Alternate,
                "alternate-reverse" => d.direction = Direction::AlternateReverse,
                "none" | "forwards" | "backwards" | "both" => {
                    if !named && tok.eq_ignore_ascii_case("none") {
                        // `none` peut être name ou fill — il est
                        // name si rien d'autre ne le prétend.
                        d.name = "none".to_string();
                        named = true;
                    } else {
                        d.fill = parse_fill(tok);
                    }
                }
                _ if !named => {
                    d.name = tok.to_string();
                    named = true;
                }
                _ => {}
            }
        }
    }
    d
}

/// `animation-iteration-count` : `infinite` ou nombre > 0.
fn parse_iteration_count(v: &str) -> f64 {
    if v.trim().eq_ignore_ascii_case("infinite") {
        f64::INFINITY
    } else {
        v.trim()
            .parse::<f64>()
            .ok()
            .filter(|n| *n > 0.0)
            .unwrap_or(1.0)
    }
}

fn parse_direction(v: &str) -> Direction {
    match v.trim().to_ascii_lowercase().as_str() {
        "reverse" => Direction::Reverse,
        "alternate" => Direction::Alternate,
        "alternate-reverse" => Direction::AlternateReverse,
        _ => Direction::Normal,
    }
}

fn parse_fill(v: &str) -> Fill {
    match v.trim().to_ascii_lowercase().as_str() {
        "forwards" => Fill::Forwards,
        "backwards" => Fill::Backwards,
        "both" => Fill::Both,
        _ => Fill::None,
    }
}

/// Propriétés interpolées à la `phase` ∈ [0,1] sur la table
/// de frames (offsets triés) — `timing` appliqué dans chaque
/// segment (spec : `animation-timing-function` de la frame
/// entrante — MVP : la déclaration unique de l'item).
/// Une propriété absente d'une borne part de `base` (frames
/// 0 %/100 % implicites du spec) ; la paire non interpolable
/// commute discrètement à mi-segment (comportement « discrete
/// animation »).
#[must_use]
pub fn keyframe_style(
    frames: &[(f64, Vec<Declaration>)],
    phase: f64,
    timing: Timing,
    base: &ComputedStyle,
) -> Vec<(String, String)> {
    if frames.is_empty() {
        return Vec::new();
    }
    let p = phase.clamp(0.0, 1.0);
    // Bornes encadrantes : dernière frame ≤ p, première ≥ p.
    let mut lo = 0usize;
    for (i, (o, _)) in frames.iter().enumerate() {
        if *o <= p {
            lo = i;
        }
    }
    let hi = frames
        .iter()
        .position(|(o, _)| *o >= p)
        .unwrap_or(frames.len() - 1);
    let (o1, d1) = &frames[lo];
    let (o2, d2) = &frames[hi];
    // t segmentaire : 0 avant, 1 après, interpolé entre.
    let t = if o2 > o1 {
        timing.apply(((p - o1) / (o2 - o1)).clamp(0.0, 1.0))
    } else if p >= *o2 {
        1.0
    } else {
        0.0
    };
    // Union des propriétés déclarées aux deux bornes.
    let mut props: Vec<String> = Vec::new();
    for d in d1.iter().chain(d2.iter()) {
        if !props.contains(&d.name) {
            props.push(d.name.clone());
        }
    }
    let mut out = Vec::new();
    for name in props {
        let v1 = d1.iter().find(|d| d.name == name).map(|d| d.value.as_str());
        let v2 = d2.iter().find(|d| d.name == name).map(|d| d.value.as_str());
        // Borne absente → valeur calculée de l'élément
        // (keyframes implicites 0 %/100 % du spec).
        let b1 = v1.map_or_else(|| base.get(&name), Some);
        let b2 = v2.map_or_else(|| base.get(&name), Some);
        match (b1, b2) {
            (Some(a), Some(b)) => {
                if let Some(v) = interpolate(a, b, t) {
                    out.push((name, v));
                } else {
                    // Non interpolable : flip discret à 50 %.
                    out.push((name, if t < 0.5 { a } else { b }.to_string()));
                }
            }
            (Some(a), None) => out.push((name, a.to_string())),
            (None, Some(b)) if t >= 0.5 => out.push((name, b.to_string())),
            _ => {}
        }
    }
    out
}
