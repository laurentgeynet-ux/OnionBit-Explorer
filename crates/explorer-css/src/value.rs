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
    named_color(&lower)
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

/// `background` complet (CSS2 §14.2.1) : le shorthand
/// **réinitialise** tous les longhands — les non mentionnés
/// repassent à leur valeur initiale (sinon `background: none`
/// laisserait une couleur déclarée avant survivre).
fn expand_background(value: &str) -> Option<Vec<(String, String)>> {
    let (mut color, mut image, mut repeat, mut attachment) = (None, None, None, None);
    let mut position = Vec::new();
    let mut recognized = false;
    for part in value.split_whitespace() {
        // La grammaire du raccourci exige une seule valeur par
        // composante ; un doublon ou un token inconnu invalide
        // la déclaration entière (`background: red pink` → drop).
        if parse_color(part).is_some() {
            if color.replace(part.to_string()).is_some() {
                return None;
            }
        } else if part == "none" || part.starts_with("url(") {
            if image.replace(part.to_string()).is_some() {
                return None;
            }
        } else if matches!(part, "repeat" | "repeat-x" | "repeat-y" | "no-repeat") {
            if repeat.replace(part.to_string()).is_some() {
                return None;
            }
        } else if matches!(part, "fixed" | "scroll" | "local") {
            if attachment.replace(part.to_string()).is_some() {
                return None;
            }
        } else if (matches!(part, "left" | "right" | "top" | "bottom" | "center")
            || parse_length(part).is_some()
            || part.ends_with('%'))
            && position.len() < 4
        {
            position.push(part.to_string());
        } else {
            return None;
        }
        recognized = true;
    }
    if !recognized {
        return None;
    }
    Some(vec![
        (
            "background-color".to_string(),
            color.unwrap_or_else(|| "transparent".to_string()),
        ),
        (
            "background-image".to_string(),
            image.unwrap_or_else(|| "none".to_string()),
        ),
        (
            "background-repeat".to_string(),
            repeat.unwrap_or_else(|| "repeat".to_string()),
        ),
        (
            "background-attachment".to_string(),
            attachment.unwrap_or_else(|| "scroll".to_string()),
        ),
        (
            "background-position".to_string(),
            if position.is_empty() {
                "0px 0px".to_string()
            } else {
                position.join(" ")
            },
        ),
    ])
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
