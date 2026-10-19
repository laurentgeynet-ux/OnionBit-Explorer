// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Transitions CSS (étape 45) — sous-ensemble animable :
//! `transition` (shorthand) et les quatre longhands,
//! `linear`/`ease*`/`cubic-bezier()`, interpolation des
//! nombres, longueurs (même unité ou absolues → px) et
//! couleurs RGBA. Déviations MVP : `steps()` et les listes
//! multi-valeurs (`transform`, `box-shadow`, `filter`) ne
//! sont pas interpolées — ces propriétés commutent en fin
//! de transition (comportement « discret » de la spec).
//!
//! Le module est pur : aucune horloge ici — `value_at`
//! reçoit une progression `t ∈ [0,1]` calculée par la
//! session, qui possède l'`Instant`.

use crate::cascade::ComputedStyle;
use crate::value::{parse_color, parse_length};

/// Une déclaration de transition résolue (un item de la
/// liste `transition` / les longhands alignés par index).
#[derive(Debug, Clone, PartialEq)]
pub struct TransitionDecl {
    /// Propriété animée (`"all"` = toute propriété animable,
    /// `"none"` = aucune — déclaration inerte).
    pub property: String,
    /// Durée en secondes (≤ 0 → pas de transition).
    pub duration: f64,
    /// Délai en secondes (négatif = transition déjà en cours
    /// au départ, spec).
    pub delay: f64,
    /// Fonction d'easing.
    pub timing: Timing,
}

/// `transition-timing-function` : mots-clés + `cubic-bezier`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Timing {
    /// `linear`.
    Linear,
    /// `ease` — équivalent `cubic-bezier(0.25, 0.1, 0.25, 1)`.
    Ease,
    /// `ease-in` — `cubic-bezier(0.42, 0, 1, 1)`.
    EaseIn,
    /// `ease-out` — `cubic-bezier(0, 0, 0.58, 1)`.
    EaseOut,
    /// `ease-in-out` — `cubic-bezier(0.42, 0, 0.58, 1)`.
    EaseInOut,
    /// `cubic-bezier(x1, y1, x2, y2)` — x bornés à `[0,1]`.
    CubicBezier(f64, f64, f64, f64),
}

impl Timing {
    /// Progresse `p ∈ [0,1]` → valeur eased ∈ `[0,1]` (peut
    /// dépasser légèrement sur les bezier à y hors `[0,1]` —
    /// la spec autorise le dépassement).
    #[must_use]
    pub fn apply(self, p: f64) -> f64 {
        let (x1, y1, x2, y2) = match self {
            Self::Linear => return p.clamp(0.0, 1.0),
            Self::Ease => (0.25, 0.1, 0.25, 1.0),
            Self::EaseIn => (0.42, 0.0, 1.0, 1.0),
            Self::EaseOut => (0.0, 0.0, 0.58, 1.0),
            Self::EaseInOut => (0.42, 0.0, 0.58, 1.0),
            Self::CubicBezier(x1, y1, x2, y2) => (x1, y1, x2, y2),
        };
        cubic_bezier_y(x1, y1, x2, y2, p.clamp(0.0, 1.0))
    }
}

/// Résout `y(t)` d'une cubique de Bézier — `t` paramétré par
/// `x` : Newton puis bissection en repli (algorithme de
/// référence des navigateurs).
fn cubic_bezier_y(x1: f64, y1: f64, x2: f64, y2: f64, x: f64) -> f64 {
    let sample = |t: f64, a: f64, b: f64| {
        // B(1,t) → 3(1-t)²t·a + 3(1-t)t²·b + t³.
        let u = 1.0 - t;
        (3.0 * u * u * t * a) + (3.0 * u * t * t * b) + (t * t * t)
    };
    if x <= 0.0 {
        return 0.0;
    }
    if x >= 1.0 {
        return 1.0;
    }
    // Newton : ~6 itérations suffisent au-delà de 1e-6.
    let mut t = x;
    for _ in 0..6 {
        let cur = sample(t, x1, x2) - x;
        if cur.abs() < 1e-6 {
            return sample(t, y1, y2);
        }
        // Dérivée de x(t) — garde-fou division nulle.
        let u = 1.0 - t;
        let d = (3.0 * u * u * x1) + (6.0 * u * t * (x2 - x1)) + (3.0 * t * t * (1.0 - x2));
        if d.abs() < 1e-6 {
            break;
        }
        t -= cur / d;
        if !(0.0..=1.0).contains(&t) {
            break;
        }
    }
    // Bissection — converge toujours (x croissant sur `[0,1]`).
    let (mut lo, mut hi) = (0.0f64, 1.0f64);
    let mut t = x;
    for _ in 0..32 {
        let cur = sample(t, x1, x2);
        if (cur - x).abs() < 1e-6 {
            break;
        }
        if cur < x {
            lo = t;
        } else {
            hi = t;
        }
        t = f64::midpoint(lo, hi);
    }
    sample(t, y1, y2)
}

/// Parse une durée `<time>` : `2s`, `150ms`, `0` → secondes.
#[must_use]
pub fn parse_duration(text: &str) -> Option<f64> {
    let t = text.trim();
    let (num, mult) = if let Some(n) = t.strip_suffix("ms") {
        (n, 0.001)
    } else if let Some(n) = t.strip_suffix('s') {
        (n, 1.0)
    } else if t == "0" {
        ("0", 1.0)
    } else {
        return None;
    };
    let v: f64 = num.trim().parse().ok()?;
    v.is_finite().then_some(v * mult)
}

/// Parse `transition-timing-function` — `steps()` non couvert
/// → `None` (la déclaration tombe en `ease`, valeur initiale).
#[must_use]
pub fn parse_timing(text: &str) -> Option<Timing> {
    let t = text.trim().to_ascii_lowercase();
    Some(match t.as_str() {
        "linear" => Timing::Linear,
        "ease" => Timing::Ease,
        "ease-in" => Timing::EaseIn,
        "ease-out" => Timing::EaseOut,
        "ease-in-out" => Timing::EaseInOut,
        _ => {
            let inner = t
                .strip_prefix("cubic-bezier(")
                .and_then(|s| s.strip_suffix(')'))?;
            let nums: Vec<f64> = inner
                .split(',')
                .filter_map(|p| p.trim().parse().ok())
                .collect();
            let [x1, y1, x2, y2] = nums.as_slice() else {
                return None;
            };
            // Spec : x hors `[0,1]` → déclaration invalide.
            if !(0.0..=1.0).contains(x1) || !(0.0..=1.0).contains(x2) {
                return None;
            }
            Timing::CubicBezier(*x1, *y1, *x2, *y2)
        }
    })
}

/// Déclarations `transition-*` d'un style calculé —
/// `transition` (shorthand) puis longhands alignés par
/// index (les longhands l'emportent, spec : les listes
/// s'étendent par répétition). `duration ≤ 0` → inerte,
/// filtrée par l'appelant.
#[must_use]
pub fn transition_decls(style: &ComputedStyle) -> Vec<TransitionDecl> {
    let mut decls = Vec::new();
    // Shorthand : `transition: prop dur timing delay, …`.
    if let Some(raw) = style.get("transition") {
        for item in raw.split(',') {
            decls.push(parse_transition_item(item));
        }
    }
    // Longhands : listes parallèles, index `i` → item `i`.
    let longhand = |name: &str| -> Vec<String> {
        style.get(name).map_or_else(Vec::new, |v| {
            v.split(',').map(|s| s.trim().to_string()).collect()
        })
    };
    let props = longhand("transition-property");
    let durs = longhand("transition-duration");
    let timings = longhand("transition-timing-function");
    let delays = longhand("transition-delay");
    let n = props.len().max(durs.len());
    for i in 0..n {
        // Un longhand explicite écrase l'item shorthand à
        // même index — composition champ par champ.
        let base = decls.get(i).cloned().unwrap_or_default();
        let mut d = base;
        if let Some(p) = props.get(i) {
            d.property.clone_from(p);
        }
        if let Some(t) = durs.get(i).and_then(|s| parse_duration(s)) {
            d.duration = t;
        }
        if let Some(t) = delays.get(i).and_then(|s| parse_duration(s)) {
            d.delay = t;
        }
        if let Some(t) = timings.get(i).and_then(|s| parse_timing(s)) {
            d.timing = t;
        }
        if i < decls.len() {
            decls[i] = d;
        } else {
            decls.push(d);
        }
    }
    decls
}

/// Un item du shorthand : `[prop] [durée] [timing] [délai]`
/// — le premier `<time>` est la durée (ordre spec).
fn parse_transition_item(item: &str) -> TransitionDecl {
    let mut d = TransitionDecl::default();
    let mut saw_time = false;
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
        } else {
            tok.clone_into(&mut d.property);
        }
    }
    d
}

impl Default for TransitionDecl {
    fn default() -> Self {
        Self {
            property: "all".to_string(),
            duration: 0.0,
            delay: 0.0,
            timing: Timing::Ease,
        }
    }
}

/// Interpole `from → to` à la progression `t ∈ [0,1]` —
/// `Some` si la paire est animable (nombre, longueur de même
/// unité ou absolue convertie en px, couleur), `None` sinon
/// (la session commute alors discrètement à `t = 1`).
#[must_use]
pub fn interpolate(from: &str, to: &str, t: f64) -> Option<String> {
    let t = t.clamp(0.0, 1.0);
    let lerp = |a: f64, b: f64| (b - a).mul_add(t, a);
    // Nombre nu (`opacity`, `z-index`…).
    if let (Ok(a), Ok(b)) = (from.trim().parse::<f64>(), to.trim().parse::<f64>()) {
        return Some(fmt_num(lerp(a, b)));
    }
    // Longueurs : même unité → lerp direct ; sinon les
    // absolues passent en px (la valeur calculée l'est).
    if let (Some(a), Some(b)) = (parse_length(from), parse_length(to)) {
        if a.unit == b.unit {
            return Some(format!("{}{}", fmt_num(lerp(a.value, b.value)), a.unit));
        }
        if let (Some(fa), Some(fb)) = (a.unit.px_factor(), b.unit.px_factor()) {
            return Some(format!("{}px", fmt_num(lerp(a.value * fa, b.value * fb))));
        }
        return None;
    }
    // Couleurs : lerp par canal RGBA → `rgba(r,g,b,a)` — le
    // canal alpha repasse en `0..1` (syntaxe `rgba()`).
    if let (Some(a), Some(b)) = (parse_color(from), parse_color(to)) {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let c = |x: u8, y: u8| -> u8 {
            lerp(f64::from(x), f64::from(y)).round().clamp(0.0, 255.0) as u8
        };
        return Some(format!(
            "rgba({}, {}, {}, {})",
            c(a.r, b.r),
            c(a.g, b.g),
            c(a.b, b.b),
            fmt_num(lerp(f64::from(a.a), f64::from(b.a)) / 255.0)
        ));
    }
    None
}

/// `f64` → chaîne courte (4 décimales max, zéros traînants
/// retirés) — le re-parse côté cascade reste exact.
fn fmt_num(v: f64) -> String {
    let s = format!("{v:.4}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" {
        "0".to_string()
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duree_s_ms_zero() {
        assert_eq!(parse_duration("2s"), Some(2.0));
        assert_eq!(parse_duration("150ms"), Some(0.15));
        assert_eq!(parse_duration("0"), Some(0.0));
        assert_eq!(parse_duration(".5s"), Some(0.5));
        assert_eq!(parse_duration("abc"), None);
        assert_eq!(parse_duration(""), None);
    }

    #[test]
    fn timing_mots_cles_et_bezier() {
        assert_eq!(parse_timing("linear"), Some(Timing::Linear));
        assert_eq!(parse_timing("ease-in-out"), Some(Timing::EaseInOut));
        assert_eq!(
            parse_timing("cubic-bezier(0.1, 0.7, 1.0, 0.1)"),
            Some(Timing::CubicBezier(0.1, 0.7, 1.0, 0.1))
        );
        // x hors `[0,1]` → invalide (spec).
        assert_eq!(parse_timing("cubic-bezier(2, 0, 0, 1)"), None);
        assert_eq!(parse_timing("steps(3)"), None);
        assert_eq!(parse_timing("bounce"), None);
    }

    #[test]
    fn easing_bornes() {
        assert_eq!(Timing::Linear.apply(0.5), 0.5);
        assert_eq!(Timing::Ease.apply(0.0), 0.0);
        assert!((Timing::Ease.apply(1.0) - 1.0).abs() < 1e-4);
        // ease-in : départ lent → milieu < 0.5.
        assert!(Timing::EaseIn.apply(0.5) < 0.5);
        assert!(Timing::EaseOut.apply(0.5) > 0.5);
    }

    #[test]
    fn decls_shorthand_et_longhands() {
        let mut s = ComputedStyle::default();
        s.set("transition", "opacity 0.3s ease-in 50ms, width 1s");
        let d = transition_decls(&s);
        assert_eq!(d.len(), 2);
        assert_eq!(d[0].property, "opacity");
        assert_eq!(d[0].duration, 0.3);
        assert_eq!(d[0].delay, 0.05);
        assert_eq!(d[0].timing, Timing::EaseIn);
        assert_eq!(d[1].property, "width");
        assert_eq!(d[1].duration, 1.0);
    }

    #[test]
    fn decls_longhand_ecrase_l_index() {
        let mut s = ComputedStyle::default();
        s.set("transition", "opacity 1s, width 2s");
        s.set("transition-duration", "4s");
        let d = transition_decls(&s);
        // Le longhand à l'index 0 écrase la durée shorthand.
        assert_eq!(d[0].duration, 4.0);
        assert_eq!(d[1].duration, 2.0);
    }

    #[test]
    fn interp_nombres_longueurs_couleurs() {
        assert_eq!(interpolate("0", "1", 0.5), Some("0.5".into()));
        assert_eq!(interpolate("10px", "20px", 0.25), Some("12.5px".into()));
        // unités absolues mixtes → px.
        assert_eq!(
            interpolate("96px", "1in", 0.5).map(|s| s == "96px"),
            Some(true)
        );
        let mid = interpolate("rgb(0,0,0)", "rgb(255,255,255)", 0.5).unwrap();
        assert!(mid.contains("128"), "{mid}");
        // non interpolable.
        assert_eq!(interpolate("auto", "block", 0.5), None);
    }
}
