// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Valeurs CSS typées (étape 16) : longueurs, pourcentages,
//! couleurs, nombres — résolues depuis le texte de déclaration.
//!
//! Les valeurs relatives à la police (`em`, `rem`, `ex`, `ch`)
//! et au viewport (`vw`, `vh`, `vmin`, `vmax`) restent des
//! unités — la conversion en px se fait à la *used value*,
//! qui connaît le contexte de mise en page (font-size hérité,
//! taille du viewport).

use std::fmt;

/// Longueur CSS : valeur + unité.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Length {
    /// Valeur numérique.
    pub value: f64,
    /// Unité (`px`, `em`…).
    pub unit: Unit,
}

/// Unités de longueur.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    /// Pixels CSS (unité de référence).
    Px,
    /// Relatif à `font-size` de l'élément.
    Em,
    /// Relatif à `font-size` de la racine.
    Rem,
    /// Relatif à la hauteur du `x` (≈ 0.5em).
    Ex,
    /// Relatif à la largeur du `0` (≈ 0.5em).
    Ch,
    /// Points, picas, pouces, cm, mm, quart de mm —
    /// absolus (1in = 96px).
    Pt,
    /// Pica (12pt).
    Pc,
    /// Pouce (96px).
    In,
    /// Centimètre (96/2.54 px).
    Cm,
    /// Millimètre.
    Mm,
    /// Quart de millimètre.
    Q,
    /// Pourcents de la largeur/hauteur du viewport.
    Vw,
    /// Pourcents de la hauteur du viewport.
    Vh,
    /// Min/max des deux dimensions du viewport.
    Vmin,
    /// Max des deux dimensions du viewport.
    Vmax,
}

impl Unit {
    /// Unité absolue → facteur en px (relatives → `None`,
    /// conversion à la used value).
    #[must_use]
    pub fn px_factor(self) -> Option<f64> {
        Some(match self {
            Self::Px => 1.0,
            Self::Pt => 96.0 / 72.0,
            Self::Pc => 16.0,
            Self::In => 96.0,
            Self::Cm => 96.0 / 2.54,
            Self::Mm => 96.0 / 25.4,
            Self::Q => 96.0 / 101.6,
            Self::Em
            | Self::Rem
            | Self::Ex
            | Self::Ch
            | Self::Vw
            | Self::Vh
            | Self::Vmin
            | Self::Vmax => return None,
        })
    }

    /// Depuis le texte d'unité (minuscules attendues).
    fn from_str(s: &str) -> Option<Self> {
        Some(match s {
            "px" => Self::Px,
            "em" => Self::Em,
            "rem" => Self::Rem,
            "ex" => Self::Ex,
            "ch" => Self::Ch,
            "pt" => Self::Pt,
            "pc" => Self::Pc,
            "in" => Self::In,
            "cm" => Self::Cm,
            "mm" => Self::Mm,
            "q" => Self::Q,
            "vw" => Self::Vw,
            "vh" => Self::Vh,
            "vmin" => Self::Vmin,
            "vmax" => Self::Vmax,
            _ => return None,
        })
    }
}

impl fmt::Display for Unit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Px => "px",
            Self::Em => "em",
            Self::Rem => "rem",
            Self::Ex => "ex",
            Self::Ch => "ch",
            Self::Pt => "pt",
            Self::Pc => "pc",
            Self::In => "in",
            Self::Cm => "cm",
            Self::Mm => "mm",
            Self::Q => "q",
            Self::Vw => "vw",
            Self::Vh => "vh",
            Self::Vmin => "vmin",
            Self::Vmax => "vmax",
        })
    }
}

/// Couleur RGBA 8 bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Color {
    /// Rouge.
    pub r: u8,
    /// Vert.
    pub g: u8,
    /// Bleu.
    pub b: u8,
    /// Alpha (255 = opaque).
    pub a: u8,
}

impl Color {
    /// Noir opaque — repli de `currentcolor` sur la racine.
    pub const BLACK: Self = Self {
        r: 0,
        g: 0,
        b: 0,
        a: 255,
    };

    /// Transparent.
    pub const TRANSPARENT: Self = Self {
        r: 0,
        g: 0,
        b: 0,
        a: 0,
    };
}

/// Valeur typée générique (résolue d'une déclaration).
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// Mot-clé (minuscules).
    Keyword(String),
    /// Longueur (`10px`, `1.5em`…).
    Length(Length),
    /// Pourcentage (`50%` → 0.5… non : 50.0 — on garde le %).
    Percentage(f64),
    /// Nombre nu (`z-index`, `opacity`…).
    Number(f64),
    /// Couleur.
    Color(Color),
    /// `url(…)` — cible conservée (réseau en amont).
    Url(String),
    /// Texte brut non typé (valeur inconnue ici — conservée
    /// pour les propriétés hors de la table couverte).
    Raw(String),
}

/// Parse une longueur absolue ou relative (`12px`, `1.5em`,
/// `0`). `auto`/`medium` ne sont PAS des longueurs — `None`.
#[must_use]
pub fn parse_length(text: &str) -> Option<Length> {
    let t = text.trim();
    if t == "0" {
        return Some(Length {
            value: 0.0,
            unit: Unit::Px,
        });
    }
    // Découpe nombre / unité : le premier caractère alphabétique
    // ouvre l'unité, SAUF `e`/`E` suivi de chiffre (exposant).
    let b = t.as_bytes();
    let mut split = t.len();
    for (i, &c) in b.iter().enumerate() {
        if c.is_ascii_alphabetic() {
            let exponent = c.eq_ignore_ascii_case(&b'e')
                && matches!(b.get(i + 1), Some(d) if d.is_ascii_digit()
                    || matches!(d, b'+' | b'-')
                        && b.get(i + 2).is_some_and(u8::is_ascii_digit));
            if !exponent {
                split = i;
                break;
            }
        }
    }
    if split == t.len() {
        return None; // nombre seul (déjà traité `0`) ou rien
    }
    let (num, unit) = t.split_at(split);
    let value: f64 = num.parse().ok()?;
    Some(Length {
        value,
        unit: Unit::from_str(&unit.to_ascii_lowercase())?,
    })
}

/// Parse une couleur : `#rgb`, `#rrggbb`, `#rgba`, `#rrggbbaa`,
/// `rgb()`/`rgba()`, noms de base + `transparent`.
/// `currentcolor` est résolu par l'appelant (contexte requis).
#[must_use]
pub fn parse_color(text: &str) -> Option<Color> {
    let t = text.trim();
    if let Some(hex) = t.strip_prefix('#') {
        return parse_hex(hex);
    }
    let lower = t.to_ascii_lowercase();
    if let Some(inner) = lower
        .strip_prefix("rgba(")
        .or_else(|| lower.strip_prefix("rgb("))
        .and_then(|s| s.strip_suffix(')'))
    {
        return parse_rgb(inner);
    }
    // R-115 — `oklab`/`oklch`/`lab`/`lch`/`hsl`/`hsla` (css-color-4).
    for (fn_name, parser) in [
        ("oklab(", parse_oklab as fn(&str) -> Option<Color>),
        ("oklch(", parse_oklch),
        ("lab(", parse_lab),
        ("lch(", parse_lch),
        ("hsl(", parse_hsl),
        ("hsla(", parse_hsl),
    ] {
        if let Some(inner) = lower
            .strip_prefix(fn_name)
            .and_then(|s| s.strip_suffix(')'))
        {
            return parser(inner);
        }
    }
    named_color(&lower)
}

/// « l c h » séparés espaces (+ `/ a`) — les deux styles
/// css-color-4 (pourcentage ou nombre) acceptés.
fn color_3args(inner: &str) -> Option<(String, String, String, f32)> {
    let (body, alpha) = match inner.split_once('/') {
        Some((b, a)) => (
            b,
            a.trim().trim_end_matches('%').parse::<f32>().ok()?
                / if a.trim().ends_with('%') { 100.0 } else { 1.0 },
        ),
        None => (inner, 1.0),
    };
    let mut it = body.split_whitespace();
    Some((
        it.next()?.into(),
        it.next()?.into(),
        it.next()?.into(),
        alpha,
    ))
}

fn num_or_pct(t: &str, max: f32) -> f32 {
    t.trim_end_matches('%').parse::<f32>().map_or(0.0, |v| {
        if t.ends_with('%') {
            v / 100.0 * max
        } else {
            v
        }
    })
}

/// Canal `[0,1]` → octet — arrondi après clamp : la valeur
/// est bornée `0.0..=1.0`, le cast `as u8` ne peut ni
/// tronquer ni perdre de signe.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn unit_to_u8(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

fn srgb_gamma(v: f32) -> f32 {
    if v <= 0.003_130_8 {
        12.92 * v
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    }
}

fn oklab_xyz(l: f32, a: f32, b: f32) -> (f32, f32, f32) {
    let l2 = (l + 0.396_337_78 * a + 0.215_803_76 * b).powi(3);
    let m2 = (l - 0.105_561_346 * a - 0.063_854_17 * b).powi(3);
    let s2 = (l - 0.089_484_18 * a - 1.291_485_5 * b).powi(3);
    (
        1.227_014 * l2 - 0.557_799_8 * m2 + 0.281_256_15 * s2,
        -0.040_580_18 * l2 + 1.112_256_9 * m2 - 0.071_676_6 * s2,
        -0.076_381_28 * l2 - 0.421_481_98 * m2 + 1.586_163_3 * s2,
    )
}

fn xyz_srgb_color(x: f32, y: f32, z: f32, alpha: f32) -> Color {
    let (rl, gl, bl) = (
        x.mul_add(3.240_97, y.mul_add(-1.537_383_2, z * -0.498_610_76)),
        x.mul_add(-0.969_243_64, y.mul_add(1.875_967_5, z * 0.041_555_06)),
        x.mul_add(0.055_630_08, y.mul_add(-0.203_976_96, z * 1.056_971_5)),
    );
    let c = |v: f32| unit_to_u8(srgb_gamma(v));
    Color {
        r: c(rl),
        g: c(gl),
        b: c(bl),
        a: unit_to_u8(alpha),
    }
}

fn parse_oklab(inner: &str) -> Option<Color> {
    let (light, aa, bb, alpha) = color_3args(inner)?;
    let (x_v, y_v, z_v) = oklab_xyz(num_or_pct(&light, 1.0), aa.parse().ok()?, bb.parse().ok()?);
    Some(xyz_srgb_color(x_v, y_v, z_v, alpha))
}

fn parse_oklch(inner: &str) -> Option<Color> {
    let (light, chroma, hue, alpha) = color_3args(inner)?;
    let (cr, hh) = (num_or_pct(&chroma, 0.4), hue.parse::<f32>().ok()?);
    let (x_v, y_v, z_v) = oklab_xyz(
        num_or_pct(&light, 1.0),
        cr * hh.to_radians().cos(),
        cr * hh.to_radians().sin(),
    );
    Some(xyz_srgb_color(x_v, y_v, z_v, alpha))
}

fn parse_lab(inner: &str) -> Option<Color> {
    let (light, aa, bb, alpha) = color_3args(inner)?;
    let fy = (num_or_pct(&light, 100.0) + 16.0) / 116.0;
    let cube = |t: f32| {
        if t > 6.0 / 29.0 {
            t * t * t
        } else {
            (108.0 / 841.0) * (t - 4.0 / 29.0)
        }
    };
    let (aa, bb) = (aa.parse::<f32>().ok()?, bb.parse::<f32>().ok()?);
    let (x_v, y_v, z_v) = (
        cube(fy + aa / 500.0) * 0.95047,
        cube(fy),
        cube(fy - bb / 200.0) * 1.08883,
    );
    Some(xyz_srgb_color(x_v, y_v, z_v, alpha))
}

fn parse_lch(inner: &str) -> Option<Color> {
    let (light, chroma, hue, alpha) = color_3args(inner)?;
    let (cr, hh) = (num_or_pct(&chroma, 150.0), hue.parse::<f32>().ok()?);
    parse_lab(&format!(
        "{} {} {} / {}",
        light,
        cr * hh.to_radians().cos(),
        cr * hh.to_radians().sin(),
        alpha
    ))
}

fn parse_hsl(inner: &str) -> Option<Color> {
    let cleaned = inner.replace(',', " ");
    let (hue, sat, light, alpha) = color_3args(&cleaned)?;
    let (hh, ss, ll) = (
        hue.parse::<f32>().ok()?.rem_euclid(360.0),
        num_or_pct(&sat, 100.0) / 100.0,
        num_or_pct(&light, 100.0) / 100.0,
    );
    // `.min().max()` et non `.clamp()` : une entrée NaN
    // (`hsl(NaN …)`) doit retomber sur les bornes numériques
    // (`f32::min`/`max` ignorent NaN) — `clamp` propagerait
    // NaN et changerait la couleur résultante.
    #[allow(clippy::manual_clamp)]
    let chan = |n: f32| {
        let k = (n + hh / 30.0).rem_euclid(12.0);
        ll - ss * ll.min(1.0 - ll) * (k - 3.0).min(9.0 - k).min(1.0).max(-1.0)
    };
    Some(Color {
        r: unit_to_u8(chan(0.0)),
        g: unit_to_u8(chan(8.0)),
        b: unit_to_u8(chan(4.0)),
        a: unit_to_u8(alpha),
    })
}

fn parse_hex(hex: &str) -> Option<Color> {
    // Nibble à l'index `i` (u8 → chiffre hex).
    let nib = |i: usize| -> Option<u8> {
        u8::try_from(char::to_digit(*hex.as_bytes().get(i)? as char, 16)?).ok()
    };
    match hex.len() {
        3 => Some(Color {
            r: nib(0)? * 17,
            g: nib(1)? * 17,
            b: nib(2)? * 17,
            a: 255,
        }),
        4 => Some(Color {
            r: nib(0)? * 17,
            g: nib(1)? * 17,
            b: nib(2)? * 17,
            a: nib(3)? * 17,
        }),
        6 => Some(Color {
            r: nib(0)? * 16 + nib(1)?,
            g: nib(2)? * 16 + nib(3)?,
            b: nib(4)? * 16 + nib(5)?,
            a: 255,
        }),
        8 => Some(Color {
            r: nib(0)? * 16 + nib(1)?,
            g: nib(2)? * 16 + nib(3)?,
            b: nib(4)? * 16 + nib(5)?,
            a: nib(6)? * 16 + nib(7)?,
        }),
        _ => None,
    }
}

/// `f64` → `u8` arrondi borné `[0, 255]` — le clamp rend le
/// cast sans perte (le lint de troncature ne le voit pas).
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn to_u8(v: f64) -> Option<u8> {
    if v.is_finite() {
        Some(v.round().clamp(0.0, 255.0) as u8)
    } else {
        None
    }
}

fn parse_rgb(inner: &str) -> Option<Color> {
    let parts: Vec<&str> = inner.split([',', '/']).map(str::trim).collect();
    let comp = |s: &str| -> Option<u8> {
        if let Some(p) = s.strip_suffix('%') {
            to_u8(p.parse::<f64>().ok()? * 2.55)
        } else {
            to_u8(s.parse::<f64>().ok()?)
        }
    };
    match parts.as_slice() {
        [red, green, blue] => Some(Color {
            r: comp(red)?,
            g: comp(green)?,
            b: comp(blue)?,
            a: 255,
        }),
        [red, green, blue, alpha] => Some(Color {
            r: comp(red)?,
            g: comp(green)?,
            b: comp(blue)?,
            a: to_u8(alpha.parse::<f64>().ok()? * 255.0)?,
        }),
        _ => None,
    }
}

/// Noms de couleurs de base (niveau 1 + les plus courantes).
fn named_color(name: &str) -> Option<Color> {
    let (r, g, b) = match name {
        "black" => (0, 0, 0),
        "silver" => (192, 192, 192),
        "gray" | "grey" => (128, 128, 128),
        "white" => (255, 255, 255),
        "maroon" => (128, 0, 0),
        "red" => (255, 0, 0),
        "purple" => (128, 0, 128),
        "fuchsia" | "magenta" => (255, 0, 255),
        "green" => (0, 128, 0),
        "lime" => (0, 255, 0),
        "olive" => (128, 128, 0),
        "yellow" => (255, 255, 0),
        "navy" => (0, 0, 128),
        "blue" => (0, 0, 255),
        "teal" => (0, 128, 128),
        "aqua" | "cyan" => (0, 255, 255),
        "orange" => (255, 165, 0),
        "aliceblue" => (240, 248, 255),
        "brown" => (165, 42, 42),
        "pink" => (255, 192, 203),
        "gold" => (255, 215, 0),
        "violet" => (238, 130, 238),
        "indigo" => (75, 0, 130),
        "coral" => (255, 127, 80),
        "salmon" => (250, 128, 114),
        "khaki" => (240, 230, 140),
        "beige" => (245, 245, 220),
        "ivory" => (255, 255, 240),
        "linen" => (250, 240, 230),
        "lavender" => (230, 230, 250),
        "turquoise" => (64, 224, 208),
        "crimson" => (220, 20, 60),
        "tomato" => (255, 99, 71),
        "orchid" => (218, 112, 214),
        "plum" => (221, 160, 221),
        "skyblue" => (135, 206, 235),
        "steelblue" => (70, 130, 180),
        "slategray" | "slategrey" => (112, 128, 144),
        "dimgray" | "dimgrey" => (105, 105, 105),
        "lightgray" | "lightgrey" => (211, 211, 211),
        "darkgray" | "darkgrey" => (169, 169, 169),
        "darkgreen" => (0, 100, 0),
        "darkblue" => (0, 0, 139),
        "darkred" => (139, 0, 0),
        "lightblue" => (173, 216, 230),
        "lightgreen" => (144, 238, 144),
        "transparent" => return Some(Color::TRANSPARENT),
        _ => return None,
    };
    Some(Color { r, g, b, a: 255 })
}

/// Validation par propriété d'une déclaration arrivée au
/// verbatim — `false` → la valeur est hors grammaire, la
/// déclaration est ignorée (spec ; ex. `width: 200` sans
/// unité, `background: red pink`). Les mots-clés CSS-wide
/// passent toujours (résolus plus tard dans la cascade).
/// Périmètre : familles de longueurs et raccourcis dont
/// l'expansion peut refuser une valeur invalide.
#[must_use]
pub fn value_is_valid_for(name: &str, value: &str) -> bool {
    let v = value.trim();
    // `var()`/`calc()`/`attr()` sont résolus plus tard — la
    // validation syntaxique ici les laisse passer.
    if v.contains("var(")
        || v.contains("calc(")
        || v.contains("attr(")
        || matches!(
            v.to_ascii_lowercase().as_str(),
            "inherit" | "initial" | "unset" | "revert" | "revert-layer"
        )
    {
        return true;
    }
    // Longueur ou pourcentage (`50%` — `parse_length` ne
    // couvre que les unités dimensionnelles).
    let len_or_pct = |s: &str| {
        parse_length(s).is_some()
            || s.strip_suffix('%')
                .is_some_and(|n| n.parse::<f64>().is_ok())
    };
    match name {
        "width" | "height" | "min-width" | "min-height" | "max-width" | "max-height" => {
            matches!(
                v,
                "auto" | "none" | "min-content" | "max-content" | "fit-content"
            ) || len_or_pct(v)
        }
        "margin-top" | "margin-right" | "margin-bottom" | "margin-left" | "top" | "right"
        | "bottom" | "left" => v == "auto" || len_or_pct(v),
        "padding-top" | "padding-right" | "padding-bottom" | "padding-left" => len_or_pct(v),
        "border-top-width" | "border-right-width" | "border-bottom-width" | "border-left-width" => {
            matches!(v, "thin" | "medium" | "thick") || len_or_pct(v)
        }
        "font-size" => {
            matches!(
                v,
                "xx-small"
                    | "x-small"
                    | "small"
                    | "medium"
                    | "large"
                    | "x-large"
                    | "xx-large"
                    | "smaller"
                    | "larger"
            ) || len_or_pct(v)
        }
        "line-height" => v == "normal" || v.parse::<f64>().is_ok() || len_or_pct(v),
        "margin" => v.split_whitespace().all(|p| p == "auto" || len_or_pct(p)),
        "padding" => v.split_whitespace().all(len_or_pct),
        "border-width" => v
            .split_whitespace()
            .all(|p| matches!(p, "thin" | "medium" | "thick") || len_or_pct(p)),
        // Raccourcis dont l'expansion refuse les valeurs
        // invalides : arrivés ici sans expansion, c'est qu'elle
        // a échoué — on revalide pour confirmer le drop.
        "background" => expand_background(v).is_some(),
        "font" => expand_font(v).is_some(),
        _ => true,
    }
}

// ─────────────── expansion des shorthands ───────────────

/// Éclate un shorthand en longhands `(nom, valeur)`.
/// `None` = `name` n'est pas un shorthand couvert — la
/// déclaration passe telle quelle. Syntaxe invalide → `None`
/// aussi (la déclaration brute est conservée : la validation
/// par propriété fera le tri).
#[must_use]
pub fn expand_shorthand(name: &str, value: &str) -> Option<Vec<(String, String)>> {
    match name {
        "margin" | "padding" => expand_box(name, value),
        "inset" => {
            // `inset` = `top right bottom left` (CSS Logical
            // §? — boîte 1..4 valeurs, même répétition que
            // margin). Vol 2026-10-26 : les liens overlay
            // `position:absolute; inset:0` du Guardian
            // restaient sans offsets → boîtes `0x0`,
            // articles inouvrables au hit-test.
            let parts: Vec<&str> = value.split_whitespace().collect();
            let [top, right, bottom, left] = four_sides(&parts)?;
            Some(vec![
                ("top".to_string(), top),
                ("right".to_string(), right),
                ("bottom".to_string(), bottom),
                ("left".to_string(), left),
            ])
        }
        "border" => Some(expand_border_parts("border", value)),
        "border-width" | "border-style" | "border-color" => {
            expand_border_uniform(name.strip_prefix("border-")?, value)
        }
        "border-top" | "border-right" | "border-bottom" | "border-left" => {
            Some(expand_border_parts(name, value))
        }
        "overflow" => expand_overflow(value),
        "flex-flow" => Some(expand_flex_flow(value)),
        "flex" => expand_flex(value),
        "gap" => expand_gap(value),
        "columns" => Some(expand_columns(value)),
        "background" => expand_background(value),
        "font" => expand_font(value),
        "list-style" => expand_list_style(value),
        _ => None,
    }
}

/// `flex-flow` : `[direction] [wrap]` ordre libre → longhands.
fn expand_flex_flow(value: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for part in value.split_whitespace() {
        match part {
            "row" | "row-reverse" | "column" | "column-reverse" => {
                out.push(("flex-direction".to_string(), part.to_string()));
            }
            "nowrap" | "wrap" | "wrap-reverse" => {
                out.push(("flex-wrap".to_string(), part.to_string()));
            }
            _ => {}
        }
    }
    out
}

/// `flex` : `none | auto | initial | [<grow> [<shrink>] ||
/// <basis>]` → longhands (le seul `<grow>` implique shrink=1,
/// basis=0 ; `flex-basis` seul non supporté en position 1).
fn expand_flex(value: &str) -> Option<Vec<(String, String)>> {
    let t = value.trim();
    let (grow, shrink, basis) = match t {
        "none" => ("0", "0", "auto"),
        "auto" => ("1", "1", "auto"),
        "initial" => ("0", "1", "auto"),
        _ => {
            let mut grow = "1";
            let mut shrink = "1";
            let mut basis = "0%";
            let mut nums = t.split_whitespace().filter(|s| s.parse::<f32>().is_ok());
            if let Some(g) = nums.next() {
                grow = g;
                if let Some(s) = nums.next() {
                    shrink = s;
                }
                if let Some(b) = t.split_whitespace().find(|s| s.parse::<f32>().is_err()) {
                    basis = b;
                }
            } else {
                // `flex: <basis>` seul (ex. "30px") → `1 1 <basis>`.
                let mut it = t.split_whitespace();
                if let (Some(b), None) = (it.next(), it.next()) {
                    basis = b;
                } else {
                    return None;
                }
            }
            (grow, shrink, basis)
        }
    };
    Some(vec![
        ("flex-grow".to_string(), grow.to_string()),
        ("flex-shrink".to_string(), shrink.to_string()),
        ("flex-basis".to_string(), basis.to_string()),
    ])
}

/// `gap` : `<row> [<col>]` → `row-gap`/`column-gap`.
fn expand_gap(value: &str) -> Option<Vec<(String, String)>> {
    let mut it = value.split_whitespace();
    let row = it.next()?;
    let col = it.next().unwrap_or(row);
    Some(vec![
        ("row-gap".to_string(), row.to_string()),
        ("column-gap".to_string(), col.to_string()),
    ])
}

/// `columns` : `[<width>] [<count>]` ordre libre (CSS
/// Multi-column L1 §3.3) — `auto` couvre le volet absent.
fn expand_columns(value: &str) -> Vec<(String, String)> {
    let mut width = "auto";
    let mut count = "auto";
    for part in value.split_whitespace() {
        match part {
            "auto" => {}
            p if p.parse::<u32>().is_ok() => count = p,
            p => width = p,
        }
    }
    vec![
        ("column-width".to_string(), width.to_string()),
        ("column-count".to_string(), count.to_string()),
    ]
}

/// `margin`/`padding` : 1..4 valeurs → quatre longhands.
fn expand_box(name: &str, value: &str) -> Option<Vec<(String, String)>> {
    let parts: Vec<&str> = value.split_whitespace().collect();
    let [top, right, bottom, left] = four_sides(&parts)?;
    Some(vec![
        (format!("{name}-top"), top),
        (format!("{name}-right"), right),
        (format!("{name}-bottom"), bottom),
        (format!("{name}-left"), left),
    ])
}

/// `border-width/style/color` : 1..4 valeurs → `border-<side>-<kind>`.
fn expand_border_uniform(kind: &str, value: &str) -> Option<Vec<(String, String)>> {
    let parts: Vec<&str> = value.split_whitespace().collect();
    let [top, right, bottom, left] = four_sides(&parts)?;
    Some(vec![
        (format!("border-top-{kind}"), top),
        (format!("border-right-{kind}"), right),
        (format!("border-bottom-{kind}"), bottom),
        (format!("border-left-{kind}"), left),
    ])
}

/// `border`/`border-<side>` : `[width] [style] [color]` ordre
/// libre — `name` est `border` (4 côtés) ou `border-<side>`.
fn expand_border_parts(name: &str, value: &str) -> Vec<(String, String)> {
    let mut width = None;
    let mut style = None;
    let mut color = None;
    for part in value.split_whitespace() {
        if is_border_style(part) {
            style = Some(part);
        } else if parse_color(part).is_some() || part == "currentcolor" {
            color = Some(part);
        } else {
            width = Some(part); // width ou inconnu — validation en aval
        }
    }
    let sides: &[&str] = if name == "border" {
        &["top", "right", "bottom", "left"]
    } else {
        // `border-<side>-<kind>` : le nom complet est préfixe.
        &[""]
    };
    let mut out = Vec::new();
    for side in sides {
        let prefix = if side.is_empty() {
            name.to_string()
        } else {
            format!("border-{side}")
        };
        if let Some(w) = width {
            out.push((format!("{prefix}-width"), w.to_string()));
        }
        if let Some(s) = style {
            out.push((format!("{prefix}-style"), s.to_string()));
        }
        if let Some(c) = color {
            out.push((format!("{prefix}-color"), c.to_string()));
        }
    }
    out
}

/// `overflow` : 1 ou 2 valeurs → `overflow-x`/`-y`.
fn expand_overflow(value: &str) -> Option<Vec<(String, String)>> {
    match value.split_whitespace().collect::<Vec<_>>().as_slice() {
        [v] => Some(vec![
            ("overflow-x".into(), (*v).to_string()),
            ("overflow-y".into(), (*v).to_string()),
        ]),
        [x, y] => Some(vec![
            ("overflow-x".into(), (*x).to_string()),
            ("overflow-y".into(), (*y).to_string()),
        ]),
        _ => None,
    }
}

/// `background` complet (CSS2 §14.2.1, étendu aux composantes
/// CSS3 `<bg-image>` fonctions et `<bg-size>` derrière `/`) :
/// le shorthand **réinitialise** tous les longhands — les non
/// mentionnés repassent à leur valeur initiale (sinon
/// `background: none` laisserait une couleur déclarée avant
/// survivre).
///
/// CSS3 autorise des **couches séparées par des virgules**
/// (`url(image), linear-gradient(décor)`) — chaque couche est
/// une grammaire complète et une couche invalide invalide la
/// déclaration entière (spec). R-107 : les longhands sont
/// **fusionnées par couche** — valeur `, `-jointe par
/// propriété, consommée par `emit_bg_layers` (render) ; la
/// `<bg-color>` n'est légale que sur la **dernière** couche
/// (css-backgrounds-3 §3.10 — une couleur en couche non
/// finale invalide la déclaration). Avant ce split la virgule
/// collait au token `url(` et le second `<bg-image>` faisait
/// tomber la déclaration entière — mesuré : `.votearrow` de
/// news.ycombinator.com.
///
/// La tokenisation est **paren-consciente** : `linear-gradient(
/// to right, red, blue)` reste un seul atome malgré ses
/// espaces — un `split_whitespace` naïf l'éclatait et la
/// déclaration tombait entière (issue #10). `<bg-size>` ne se
/// déclare que derrière un `/` (`url(x) 50% / cover`) ; un `/`
/// collé (`50%/cover`) est accepté comme le font les
/// navigateurs.
fn expand_background(value: &str) -> Option<Vec<(String, String)>> {
    let layers: Vec<Vec<(String, String)>> = split_top_commas(value)
        .into_iter()
        .map(parse_bg_layer)
        .collect::<Option<_>>()?;
    let (last, rest) = layers.split_last()?;
    // §3.10 : `<bg-color>` n'apparaît que dans la couche
    // finale — une couleur non transparente plus haut rend la
    // déclaration invalide.
    if rest.iter().any(|l| {
        l.iter()
            .any(|(n, v)| n == "background-color" && v != "transparent")
    }) {
        return None;
    }
    // Toutes les couches émettent les mêmes longhands dans le
    // même ordre (`into_longhands` est un vecteur fixe) —
    // fusion positionnelle.
    let mut out = Vec::with_capacity(last.len());
    for (i, (name, _)) in last.iter().enumerate() {
        if name == "background-color" {
            out.push((name.clone(), last[i].1.clone()));
        } else {
            let joined = layers
                .iter()
                .map(|l| l[i].1.clone())
                .collect::<Vec<_>>()
                .join(", ");
            out.push((name.clone(), joined));
        }
    }
    Some(out)
}

/// Parse **une** couche du shorthand `background` — même
/// grammaire qu'avant le split multi-couches (R-94) : une
/// valeur par composante, doublon ou token inconnu → `None`.
fn parse_bg_layer(layer: &str) -> Option<Vec<(String, String)>> {
    let mut acc = BgParts::default();
    for raw in bg_tokens(layer) {
        let segs = split_top_slash(raw);
        // `pos / w h` au plus : `a/b/c/d` est malformé.
        if segs.len() > 3 {
            return None;
        }
        for (i, part) in segs.iter().enumerate() {
            if i > 0 {
                acc.in_size = true;
            }
            if part.is_empty() {
                continue; // `/` seul ou `a/` — toléré.
            }
            if !acc.push_part(part) {
                return None;
            }
        }
    }
    if !acc.recognized {
        return None;
    }
    Some(acc.into_longhands())
}

/// Accumulateur du shorthand `background` — un slot par
/// composante ; la grammaire exige une seule valeur par
/// composante (doublon ou token inconnu → déclaration
/// entière droppée, `background: red pink`).
#[derive(Default)]
struct BgParts {
    color: Option<String>,
    image: Option<String>,
    repeat: Option<String>,
    attachment: Option<String>,
    position: Vec<String>,
    size: Vec<String>,
    boxes: Vec<String>,
    /// Les segments qui suivent immédiatement un `/` sont des
    /// valeurs `background-size` — jusqu'au premier token qui
    /// n'en est pas une (`center/cover no-repeat` :
    /// `no-repeat` ressort du mode taille et reclasse normal).
    in_size: bool,
    recognized: bool,
}

impl BgParts {
    /// Classe un atome dans son slot ; `false` = déclaration
    /// invalide.
    fn push_part(&mut self, part: &str) -> bool {
        if self.in_size && self.size.len() < 2 && bg_size_token(part) {
            self.size.push(part.to_string());
            self.recognized = true;
            return true;
        }
        self.in_size = false;
        self.recognized = true;
        if parse_color(part).is_some() {
            return self.color.replace(part.to_string()).is_none();
        }
        if bg_image_token(part) {
            return self.image.replace(part.to_string()).is_none();
        }
        if matches!(part, "repeat" | "repeat-x" | "repeat-y" | "no-repeat") {
            return self.repeat.replace(part.to_string()).is_none();
        }
        if matches!(part, "fixed" | "scroll" | "local") {
            return self.attachment.replace(part.to_string()).is_none();
        }
        if matches!(part, "border-box" | "padding-box" | "content-box") && self.boxes.len() < 2 {
            self.boxes.push(part.to_string());
            return true;
        }
        if (matches!(part, "left" | "right" | "top" | "bottom" | "center")
            || parse_length(part).is_some()
            || part.ends_with('%'))
            && self.position.len() < 4
        {
            self.position.push(part.to_string());
            return true;
        }
        false
    }

    /// Émet les longhands — les absents repassent à leur
    /// valeur initiale (le shorthand réinitialise tout).
    fn into_longhands(self) -> Vec<(String, String)> {
        vec![
            (
                "background-color".to_string(),
                self.color.unwrap_or_else(|| "transparent".to_string()),
            ),
            (
                "background-image".to_string(),
                self.image.unwrap_or_else(|| "none".to_string()),
            ),
            (
                "background-repeat".to_string(),
                self.repeat.unwrap_or_else(|| "repeat".to_string()),
            ),
            (
                "background-attachment".to_string(),
                self.attachment.unwrap_or_else(|| "scroll".to_string()),
            ),
            (
                "background-position".to_string(),
                if self.position.is_empty() {
                    "0px 0px".to_string()
                } else {
                    self.position.join(" ")
                },
            ),
            (
                "background-size".to_string(),
                if self.size.is_empty() {
                    "auto".to_string()
                } else {
                    self.size.join(" ")
                },
            ),
            // Une `<box>` seule couvre origin + clip (spec) ;
            // deux → origin puis clip. La peinture les ignore
            // encore — émis pour complétude de la cascade.
            (
                "background-origin".to_string(),
                self.boxes
                    .first()
                    .cloned()
                    .unwrap_or_else(|| "padding-box".to_string()),
            ),
            (
                "background-clip".to_string(),
                self.boxes
                    .last()
                    .cloned()
                    .unwrap_or_else(|| "border-box".to_string()),
            ),
        ]
    }
}

/// Tokenisation du shorthand `background` : espaces hors
/// parenthèses seulement — `linear-gradient(to right, red,
/// blue)` demeure un atome unique.
fn bg_tokens(value: &str) -> Vec<&str> {
    let mut toks = Vec::new();
    let mut depth = 0u32;
    let mut start = 0usize;
    let mut in_tok = false;
    for (i, ch) in value.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            _ => {}
        }
        if ch.is_whitespace() && depth == 0 {
            if in_tok {
                toks.push(&value[start..i]);
                in_tok = false;
            }
        } else if !in_tok {
            start = i;
            in_tok = true;
        }
    }
    if in_tok {
        toks.push(&value[start..]);
    }
    toks
}

/// Éclate la valeur `background` sur les virgules hors
/// parenthèses — les **couches** CSS3 (`url(a), gradient(…)`).
/// Les virgules *dans* une fonction sont protégées par la
/// profondeur (`linear-gradient(red, blue)` reste un segment).
/// Un segment vide (`url(a),` ou `,url(a)`) produit une couche
/// que [`parse_bg_layer`] rejettera — déclaration invalide,
/// conforme à la spec.
fn split_top_commas(value: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth = 0u32;
    let mut start = 0usize;
    for (i, ch) in value.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                out.push(value[start..i].trim());
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(value[start..].trim());
    out
}

/// Éclate un atome du shorthand sur les `/` hors parenthèses —
/// `50%/cover` → `["50%", "cover"]`, `/` seul → `["", ""]`.
fn split_top_slash(tok: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth = 0u32;
    let mut start = 0usize;
    for (i, ch) in tok.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            '/' if depth == 0 => {
                out.push(&tok[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&tok[start..]);
    out
}

/// Valeur `<bg-image>` du shorthand : `none`, `url(…)` ou une
/// fonction image CSS3 — dégradés (`linear-gradient` et ses
/// variantes `radial-`/`conic-`/`repeating-`/préfixées vendeur,
/// toutes repérées par leur suffixe `gradient(`),
/// `image()`/`image-set()`). Dans une même couche, une seule
/// image — `url(a) url(b)` invalide ; le multi-fonds par
/// virgules est éclaté en amont (`split_top_commas`, R-94).
fn bg_image_token(part: &str) -> bool {
    part == "none"
        || part.starts_with("url(")
        || part.contains("gradient(")
        || part.starts_with("image(")
        || part.starts_with("image-set(")
}

/// Valeur `<bg-size>` après `/` : `auto`, `cover`, `contain`,
/// longueur ou pourcentage (1 ou 2 valeurs).
fn bg_size_token(part: &str) -> bool {
    matches!(part, "auto" | "cover" | "contain")
        || parse_length(part).is_some()
        || part.ends_with('%')
}

/// `font: [style]? [variant]? [weight]? size[/line-height] family`.
fn expand_font(value: &str) -> Option<Vec<(String, String)>> {
    let parts: Vec<&str> = value.split_whitespace().collect();
    // Le premier token à chiffre / mot-clé de taille / `size/lh`
    // marque font-size — tout ce qui suit est la famille.
    let size_pos = parts.iter().position(|p| {
        parse_length(p).is_some() || p.contains('/') || FONT_SIZE_KEYWORDS.contains(p)
    })?;
    let mut out = Vec::new();
    for p in &parts[..size_pos] {
        match *p {
            "italic" | "oblique" => {
                out.push(("font-style".to_string(), (*p).to_string()));
            }
            "small-caps" => {
                out.push(("font-variant".to_string(), (*p).to_string()));
            }
            _ => {
                out.push(("font-weight".to_string(), (*p).to_string()));
            }
        }
    }
    if let Some((size, lh)) = parts[size_pos].split_once('/') {
        out.push(("font-size".to_string(), size.to_string()));
        out.push(("line-height".to_string(), lh.to_string()));
    } else {
        out.push(("font-size".to_string(), parts[size_pos].to_string()));
    }
    let family = parts[size_pos + 1..].join(" ");
    if !family.is_empty() {
        out.push(("font-family".to_string(), family));
    }
    // Le shorthand `font` réinitialise TOUTES les
    // sous-propriétés font-* (CSS Fonts §3) : celles non
    // exprimées retombent à leur initiale — sinon un
    // `font: 12pt serif` laisserait fuiter le `line-height`
    // hérité (margin-collapse-020/021) ou un style/weight
    // hérités. On émet les longhands absents en `normal`.
    for (prop, init) in [
        ("font-style", "normal"),
        ("font-variant", "normal"),
        ("font-weight", "normal"),
        ("line-height", "normal"),
    ] {
        if !out.iter().any(|(p, _)| p == prop) {
            out.push((prop.to_string(), init.to_string()));
        }
    }
    Some(out)
}

/// `list-style` : chaque composante va à son longhand.
fn expand_list_style(value: &str) -> Option<Vec<(String, String)>> {
    let mut out = Vec::new();
    for part in value.split_whitespace() {
        match part {
            "inside" | "outside" => {
                out.push(("list-style-position".to_string(), part.to_string()));
            }
            _ if part.starts_with("url(") => {
                out.push(("list-style-image".to_string(), part.to_string()));
            }
            _ => {
                out.push(("list-style-type".to_string(), part.to_string()));
            }
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

/// `v` est-il un mot-clé de `border-style` ?
fn is_border_style(v: &str) -> bool {
    matches!(
        v,
        "none"
            | "hidden"
            | "dotted"
            | "dashed"
            | "solid"
            | "double"
            | "groove"
            | "ridge"
            | "inset"
            | "outset"
    )
}

/// Mots-clés de `font-size` absolus/relatifs.
const FONT_SIZE_KEYWORDS: &[&str] = &[
    "xx-small", "x-small", "small", "medium", "large", "x-large", "xx-large", "smaller", "larger",
];

/// `1..4` valeurs → `[top, right, bottom, left]` (spec).
fn four_sides(parts: &[&str]) -> Option<[String; 4]> {
    Some(match parts {
        [a] => [
            (*a).to_string(),
            (*a).to_string(),
            (*a).to_string(),
            (*a).to_string(),
        ],
        [a, b] => [
            (*a).to_string(),
            (*b).to_string(),
            (*a).to_string(),
            (*b).to_string(),
        ],
        [a, b, c] => [
            (*a).to_string(),
            (*b).to_string(),
            (*c).to_string(),
            (*b).to_string(),
        ],
        [a, b, c, d] => [
            (*a).to_string(),
            (*b).to_string(),
            (*c).to_string(),
            (*d).to_string(),
        ],
        _ => return None,
    })
}
