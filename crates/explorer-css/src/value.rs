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
            // `d*`/`s*`/`l*` (viewport dynamique/petit/large,
            // css-values-4 §6.1.3) : sans chrome mobile les
            // trois bases coïncident → projection sur les
            // unités statiques (motif `100dvh` des éditeurs
            // plein écran — uMap §B-28).
            "vw" | "dvw" | "svw" | "lvw" => Self::Vw,
            "vh" | "dvh" | "svh" | "lvh" => Self::Vh,
            "vmin" | "dvmin" | "svmin" | "lvmin" => Self::Vmin,
            "vmax" | "dvmax" | "svmax" | "lvmax" => Self::Vmax,
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

/// Mode d'écriture (css-writing-modes-3 §3.1 — R-183).
/// `Sideways*` est ramené au mode vertical de base pour le
/// mapping logique (la différence = orientation des glyphes,
/// hors MVP).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WritingMode {
    /// Inline horizontal, bloc top→bottom (défaut).
    #[default]
    HorizontalTb,
    /// Inline top→bottom, bloc right→left.
    VerticalRl,
    /// Inline top→bottom, bloc left→right.
    VerticalLr,
    /// `sideways-rl` — traité comme [`Self::VerticalRl`]
    /// au MVP (glyphes toujours « couchés »).
    SidewaysRl,
    /// `sideways-lr` — traité comme [`Self::VerticalLr`].
    SidewaysLr,
}

impl WritingMode {
    /// Mode vertical (axe inline = vertical) ?
    #[must_use]
    pub fn is_vertical(&self) -> bool {
        !matches!(self, Self::HorizontalTb)
    }
    /// Bloc en x **décroissant** (`vertical-rl`) ?
    #[must_use]
    pub fn block_reversed(&self) -> bool {
        matches!(self, Self::VerticalRl | Self::SidewaysRl)
    }
}

/// `writing-mode` CSS → enum (`horizontal-tb` en repli sur
/// valeur inconnue — dégradation, jamais panic).
#[must_use]
pub fn parse_writing_mode(v: &str) -> WritingMode {
    match v.trim().to_ascii_lowercase().as_str() {
        "vertical-rl" => WritingMode::VerticalRl,
        "vertical-lr" => WritingMode::VerticalLr,
        "sideways-rl" => WritingMode::SidewaysRl,
        "sideways-lr" => WritingMode::SidewaysLr,
        _ => WritingMode::HorizontalTb,
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
    // R-214 — `color-mix(in <espace>, A [p%], B [p%])` (css-color-5)
    // + syntaxe relative `oklch(from <couleur> l c h / a)` — les
    // mélanges passent par sRGB (espaces autres qu'`srgb` admis par
    // approximation documentée). Terrain : `bg-brand-50/50` de
    // plyr.io (Tailwind v4) — `color-mix(in srgb, oklch(from
    // var(--brand) l c h) 50%, transparent)`.
    if let Some(inner) = lower
        .strip_prefix("color-mix(")
        .and_then(|s| s.strip_suffix(')'))
    {
        return parse_color_mix(inner);
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
    // R-214 — syntaxe relative css-color-5 : `oklch(from
    // <couleur> <l> <c> <h> / <a>)` — les canaux `l`/`c`/`h`/
    // `alpha` nomment ceux de la source, seuls ou en `calc()`.
    if let Some(rest) = inner.trim().strip_prefix("from ") {
        return oklch_relative(rest);
    }
    let (light, chroma, hue, alpha) = color_3args(inner)?;
    let (cr, hh) = (num_or_pct(&chroma, 0.4), hue.parse::<f32>().ok()?);
    let (x_v, y_v, z_v) = oklab_xyz(
        num_or_pct(&light, 1.0),
        cr * hh.to_radians().cos(),
        cr * hh.to_radians().sin(),
    );
    Some(xyz_srgb_color(x_v, y_v, z_v, alpha))
}

/// sRGB `[0,255]` → OKLCH `(l, c, h, a)` — inverse des
/// matrices `oklab_xyz`/`xyz_srgb_color` (Ottosson). Sert aux
/// couleurs relatives (`oklch(from …)`) et à `color-mix` en
/// espaces non-sRGB.
fn srgb_to_oklch(c: Color) -> (f32, f32, f32, f32) {
    let lin = |v: u8| {
        let s = f32::from(v) / 255.0;
        if s <= 0.040_45 {
            s / 12.92
        } else {
            ((s + 0.055) / 1.055).powf(2.4)
        }
    };
    let (r, g, b) = (lin(c.r), lin(c.g), lin(c.b));
    let (x_v, y_v, z_v) = (
        r.mul_add(0.412_391, g.mul_add(0.357_584, b * 0.180_481)),
        r.mul_add(0.212_639, g.mul_add(0.715_169, b * 0.072_192)),
        r.mul_add(0.019_331, g.mul_add(0.119_195, b * 0.950_532)),
    );
    let (l_, m_, s_) = (
        (x_v.mul_add(0.818_933, y_v.mul_add(0.361_867, z_v * -0.128_86))).cbrt(),
        (x_v.mul_add(0.032_985, y_v.mul_add(0.929_312, z_v * 0.036_146))).cbrt(),
        (x_v.mul_add(0.048_2, y_v.mul_add(0.264_366, z_v * 0.633_852))).cbrt(),
    );
    let (ol, oa, ob) = (
        0.210_454 * l_ + 0.793_618 * m_ - 0.004_072 * s_,
        1.978 * l_ - 2.428_592 * m_ + 0.450_594 * s_,
        0.025_904 * l_ + 0.782_772 * m_ - 0.808_676 * s_,
    );
    (
        ol,
        oa.hypot(ob),
        ob.atan2(oa).to_degrees().rem_euclid(360.0),
        f32::from(c.a) / 255.0,
    )
}

/// Canal relatif (`l`/`c`/`h`/`alpha`) : mot-clé = valeur de la
/// source ; nombre/% = direct ; `calc()` = expression sur ces
/// variables (`calc(l * 0.5)`, `calc(h + 30)`…). `get` retourne
/// la valeur source du canal nommé ; `scale` la borne des `%`.
fn rel_channel(expr: &str, names: &[&str], src: &[f32], scale: f32) -> Option<f32> {
    let e = expr.trim();
    let get = |k: &str| names.iter().position(|n| *n == k).map(|i| src[i]);
    // Mot-clé seul → canal source ; `none` → 0 (canal absent).
    if e.eq_ignore_ascii_case("none") {
        return Some(0.0);
    }
    if let Some(v) = get(e) {
        return Some(v);
    }
    // `calc(…)` : évalue en substituant les canaux nommés —
    // mini-évaluateur `+ - * /` à priorité `*`/`/` (parenthèses
    // un niveau : le contenu est récursé).
    if let Some(inner) = e.strip_prefix("calc(").and_then(|s| s.strip_suffix(')')) {
        return eval_rel_calc(inner.trim(), &get, scale);
    }
    // Nombre ou pourcentage nu.
    let v = e.strip_suffix('%').map_or_else(
        || e.parse::<f32>().ok(),
        |n| n.trim().parse::<f32>().ok().map(|x| x / 100.0 * scale),
    )?;
    Some(v)
}

/// Évalue un `calc()` restreint : `±` profondeur 0, puis `*/`
/// — les opérandes sont nombres, `%` ou canaux (`l`, `c`, `h`,
/// `alpha`). NaN/échec → `None` (dégradation, jamais panic).
fn eval_rel_calc(e: &str, get: &dyn Fn(&str) -> Option<f32>, scale: f32) -> Option<f32> {
    let atom = |t: &str| -> Option<f32> {
        let t = t.trim();
        if let Some(v) = get(t) {
            return Some(v);
        }
        if let Some(inner) = t.strip_prefix('(').and_then(|s| s.strip_suffix(')')) {
            return eval_rel_calc(inner.trim(), get, scale);
        }
        t.strip_suffix('%').map_or_else(
            || t.parse::<f32>().ok(),
            |n| n.trim().parse::<f32>().ok().map(|x| x / 100.0 * scale),
        )
    };
    // Split ± profondeur 0 (signe en tête toléré).
    let mut terms: Vec<(bool, String)> = Vec::new();
    let (mut depth, mut cur, mut neg) = (0i32, String::new(), false);
    let mut first = true;
    for ch in e.chars() {
        match ch {
            '(' => depth += 1,
            ')' => depth -= 1,
            '+' | '-' if depth == 0 => {
                if first {
                    neg = ch == '-';
                    first = false;
                } else {
                    terms.push((neg, cur.trim().into()));
                    (cur, neg) = (String::new(), ch == '-');
                }
                continue;
            }
            _ => {}
        }
        cur.push(ch);
    }
    terms.push((neg, cur));
    let mul = |t: &str| -> Option<f32> {
        // `*` et `/` dans le terme.
        let it = t.split_inclusive(['*', '/']);
        let mut acc: Option<f32> = None;
        for seg in it {
            let (num, op) = match seg.chars().last() {
                Some('*') => (&seg[..seg.len() - 1], '*'),
                Some('/') => (&seg[..seg.len() - 1], '/'),
                _ => (seg, ' '),
            };
            let v = atom(num)?;
            acc = Some(match (acc, op) {
                (None, _) => v,
                (Some(a), '/') => a / v,
                (Some(a), _) => a * v,
            });
        }
        acc
    };
    let mut sum = 0.0;
    for (neg, t) in &terms {
        let v = mul(t)?;
        sum += if *neg { -v } else { v };
    }
    Some(sum)
}

/// `oklch(from <couleur> <l> <c> <h> / <a>)` — la source est le
/// premier token (hex, nom, `rgb()`… ou fonction imbriquée).
fn oklch_relative(rest: &str) -> Option<Color> {
    // Délimite la couleur source : fonction imbriquée → à la
    // parenthèse fermante de profondeur 0 ; sinon au 1er espace.
    let src_end = if rest
        .trim_start()
        .starts_with(|c: char| c.is_ascii_alphabetic())
        && rest.trim_start().contains('(')
    {
        let mut depth = 0i32;
        let start = rest.trim_start();
        let mut end = start.len();
        for (i, ch) in start.char_indices() {
            match ch {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        end = i + 1;
                        break;
                    }
                }
                _ => {}
            }
        }
        rest.trim_start().len() - rest.len() + end
    } else {
        rest.find(char::is_whitespace)?
    };
    let src = parse_color(&rest[..src_end])?;
    let (l0, c0, h0, a0) = srgb_to_oklch(src);
    let rest = &rest[src_end..];
    // Canaux séparés espaces à profondeur 0 (`calc(l * 0.5)`
    // contient des espaces internes) + `/ alpha` éventuel.
    let (body, alpha) = match rest.split_once('/') {
        Some((b, a)) => (b, a.trim()),
        None => (rest, ""),
    };
    let mut toks: Vec<&str> = Vec::new();
    let (mut depth, mut start) = (0i32, 0usize);
    for (i, ch) in body.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth -= 1,
            ' ' if depth == 0 => {
                let t = body[start..i].trim();
                if !t.is_empty() {
                    toks.push(t);
                }
                start = i + 1;
            }
            _ => {}
        }
    }
    let t = body[start..].trim();
    if !t.is_empty() {
        toks.push(t);
    }
    if toks.len() != 3 {
        return None;
    }
    let (light, chroma, hue) = (toks[0], toks[1], toks[2]);
    let names = ["l", "c", "h", "alpha"];
    let srcv = [l0, c0, h0, a0];
    let l = rel_channel(light, &names, &srcv, 1.0)?;
    let cr = rel_channel(chroma, &names, &srcv, 0.4)?;
    let hh = rel_channel(hue, &names, &srcv, 360.0)?;
    let al = if alpha.is_empty() {
        a0
    } else {
        rel_channel(alpha, &names, &srcv, 1.0)?
    };
    let (x_v, y_v, z_v) = oklab_xyz(l, cr * hh.to_radians().cos(), cr * hh.to_radians().sin());
    Some(xyz_srgb_color(x_v, y_v, z_v, al))
}

/// `color-mix(in <espace>, <couleur> [p%], <couleur> [p%])` —
/// mélange linéaire en sRGB (autres espaces admis : conversion
/// d'abord, interpolation sRGB ensuite — approximation
/// documentée ; `oklch` interpole en `OKLab` serait plus fidèle).
fn parse_color_mix(inner: &str) -> Option<Color> {
    // Split top-level `,` (profondeur 0) : [in srgb, A, B].
    let mut depth = 0i32;
    let mut parts: Vec<&str> = Vec::new();
    let mut start = 0;
    for (i, ch) in inner.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 => {
                parts.push(&inner[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(&inner[start..]);
    if parts.len() != 3 {
        return None;
    }
    // `in <espace>` — admis mais non différencié (approx sRGB).
    let first = parts[0].trim();
    if !first.starts_with("in ") {
        return None;
    }
    let side = |s: &str| -> Option<(Color, f32)> {
        let s = s.trim();
        // Couleur + pourcentage éventuel en fin (`X 50%`).
        let mut depth = 0i32;
        let mut split = s.len();
        for (i, ch) in s.char_indices().rev() {
            match ch {
                ')' => depth += 1,
                '(' => depth -= 1,
                ' ' if depth == 0 => {
                    if s[i + 1..].trim_end().ends_with('%') {
                        split = i;
                    }
                    break;
                }
                _ => {}
            }
        }
        let (col, pct) = if split < s.len() {
            (
                &s[..split],
                s[split..]
                    .trim()
                    .trim_end_matches('%')
                    .parse::<f32>()
                    .ok()?
                    / 100.0,
            )
        } else {
            (s, -1.0)
        };
        Some((parse_color(col)?, pct))
    };
    let (a, mut pa) = side(parts[1])?;
    let (b, mut pb) = side(parts[2])?;
    // Défauts css-color-5 : un seul % → l'autre complète à 100 ;
    // aucun → 50/50. Somme >100 normalisée.
    if pa < 0.0 && pb < 0.0 {
        (pa, pb) = (0.5, 0.5);
    } else if pa < 0.0 {
        pa = (1.0 - pb).max(0.0);
    } else if pb < 0.0 {
        pb = (1.0 - pa).max(0.0);
    }
    let tot = pa + pb;
    if tot <= 0.0 {
        return None;
    }
    let (pa, pb) = (pa / tot, pb / tot);
    let mix = |x: u8, y: u8| -> u8 {
        unit_to_u8((f32::from(x) / 255.0).mul_add(pa, f32::from(y) / 255.0 * pb))
    };
    Some(Color {
        r: mix(a.r, b.r),
        g: mix(a.g, b.g),
        b: mix(a.b, b.b),
        a: unit_to_u8((f32::from(a.a) / 255.0).mul_add(pa, f32::from(b.a) / 255.0 * pb)),
    })
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
        ll - ss * ll.min(1.0 - ll) * (k - 3.0).min(9.0 - k).clamp(-1.0, 1.0)
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
    // `var()`/`calc()`/`attr()`/`min()`/`max()`/`clamp()`
    // sont résolus plus tard — la validation syntaxique ici
    // les laisse passer (vol 11 stripe : `width:min(50%,…)`
    // droppé → items éclatés à la taille de contenu).
    if v.contains("var(")
        || v.contains("calc(")
        || v.contains("attr(")
        || v.contains("min(")
        || v.contains("max(")
        || v.contains("clamp(")
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
    // Propriétés logiques (css-logical-1 — R-124/R-183) :
    // shorthands `*-inline`/`*-block` éclatés en longhands
    // logiques ; la résolution physique attend la cascade.
    if let Some(expanded) = expand_logical(name, value) {
        return Some(expanded);
    }
    // `text-wrap` (css-text-4 — R-249) : `<mode> <style>?`
    // → `text-wrap-mode` + `text-wrap-style`. `nowrap` n'est
    // qu'un mode ; `balance`/`pretty`/`stable`/`auto` ne sont
    // qu'un style — chaque absent retombe sur son initial.
    if name == "text-wrap" {
        let parts: Vec<&str> = split_ws_top(value);
        let mut mode = None;
        let mut style = None;
        for p in parts {
            match p {
                "wrap" | "nowrap" => mode = Some(p),
                "balance" | "pretty" | "stable" | "auto" => style = Some(p),
                _ => return None,
            }
        }
        return Some(vec![
            ("text-wrap-mode".to_string(), mode.unwrap_or("wrap").to_string()),
            ("text-wrap-style".to_string(), style.unwrap_or("auto").to_string()),
        ]);
    }
    // Shorthands `place-*` (css-align-3 — R-125) : 1 valeur
    // répétée ou 2 (`<align> <justify>`).
    if let Some((a, b)) = match name {
        "place-items" => Some(("align-items", "justify-items")),
        "place-content" => Some(("align-content", "justify-content")),
        "place-self" => Some(("align-self", "justify-self")),
        _ => None,
    } {
        let parts: Vec<&str> = split_ws_top(value);
        let (v1, v2) = match parts.as_slice() {
            [v] => (*v, *v),
            [v1, v2] => (*v1, *v2),
            _ => return None,
        };
        return Some(vec![
            (a.to_string(), v1.to_string()),
            (b.to_string(), v2.to_string()),
        ]);
    }
    // `border-radius` (R-167) : 1..4 rayons → coins tl/tr/
    // br/bl ; la forme elliptique `h/v` n'est pas encore
    // supportée — seul le rayon horizontal avant `/` est
    // pris (dégradation documentée).
    if name == "border-radius" {
        let hpart = value.split('/').next().unwrap_or_default();
        let parts: Vec<&str> = split_ws_top(hpart);
        let [tl, tr, br, bl] = four_sides(&parts)?;
        return Some(vec![
            ("border-top-left-radius".to_string(), tl),
            ("border-top-right-radius".to_string(), tr),
            ("border-bottom-right-radius".to_string(), br),
            ("border-bottom-left-radius".to_string(), bl),
        ]);
    }
    match name {
        "margin" | "padding" => expand_box(name, value),
        "inset" => {
            // `inset` = `top right bottom left` (CSS Logical
            // §? — boîte 1..4 valeurs, même répétition que
            // margin). Vol 2026-10-26 : les liens overlay
            // `position:absolute; inset:0` du Guardian
            // restaient sans offsets → boîtes `0x0`,
            // articles inouvrables au hit-test.
            let parts: Vec<&str> = split_ws_top(value);
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
        "gap" | "grid-gap" => expand_gap(value),
        "grid-template" | "grid" => expand_grid_template(value),
        "columns" => Some(expand_columns(value)),
        "column-rule" => Some(expand_column_rule(value)),
        "background" => expand_background(value),
        "font" => expand_font(value),
        "list-style" => expand_list_style(value),
        "outline" => Some(expand_outline(value)),
        "container" => expand_container(value),
        _ => None,
    }
}

/// `container` shorthand → longhands (css-contain-3 — R-190) :
/// `<'container-name'> [ / <'container-type'> ]` — les noms
/// libres avant `/` ; `container-type` absent → `normal`
/// (non émis — c'est l'initiale). Noms vides → `None`
/// (déclaration passthrough, jamais consommée).
fn expand_container(value: &str) -> Option<Vec<(String, String)>> {
    let (names, ty) = match value.split_once('/') {
        Some((n, t)) => (n.trim(), Some(t.trim())),
        None => (value.trim(), None),
    };
    if names.is_empty() {
        return None;
    }
    let mut out = vec![("container-name".to_string(), names.to_string())];
    if let Some(t) = ty {
        out.push(("container-type".to_string(), t.to_string()));
    }
    Some(out)
}

/// `flex-flow` : `[direction] [wrap]` ordre libre → longhands.
fn expand_flex_flow(value: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for part in split_ws_top(value) {
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
            // Tokenisation paren-consciente : `calc(75% - 5px)`
            // reste un atome unique — un split sur espaces brisait
            // la base en `calc(75%` → `flex-basis` invalide →
            // repli layout sur le max-content de l'item (§B-27,
            // ouest-france `su-col-*` : colonne à 13387px).
            let toks = bg_tokens(t);
            let mut nums = toks.iter().filter(|s| s.parse::<f32>().is_ok());
            if let Some(&g) = nums.next() {
                grow = g;
                if let Some(&s) = nums.next() {
                    shrink = s;
                }
                if let Some(&b) = toks.iter().find(|s| s.parse::<f32>().is_err()) {
                    basis = b;
                }
            } else {
                // `flex: <basis>` seul (ex. "30px") → `1 1 <basis>`.
                if let (Some(b), None) = (toks.first().copied(), toks.get(1)) {
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
/// `grid-template`/`grid` : `none | [<rows> / <cols>] |
/// <areas-ascii> [<rows>] / <cols>` — l'ASCII `"a a" 50px
/// "b ." auto / 1fr 2fr` éclate areas/rows/cols ; `grid`
/// traite ici la forme template (`none` + auto-flow hors
/// scope — R-175).
fn expand_grid_template(value: &str) -> Option<Vec<(String, String)>> {
    let v = value.trim();
    if v == "none" {
        return Some(vec![
            ("grid-template-columns".into(), "none".into()),
            ("grid-template-rows".into(), "none".into()),
            ("grid-template-areas".into(), "none".into()),
        ]);
    }
    let (left, right) = v.split_once('/').unwrap_or((v, ""));
    // Côté gauche : `"a a"` = lignes d'areas, les tailles
    // intercalées = `grid-template-rows` ; sans guillemets,
    // le gauche est `rows` quand le droit existe.
    let mut areas = String::new();
    let mut rows: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut in_q = false;
    for c in left.chars() {
        if c == '"' {
            if in_q {
                in_q = false;
                areas.push('"');
                areas.push_str(&cur);
                areas.push('"');
                cur.clear();
            } else {
                rows.extend(cur.split_whitespace().map(str::to_string));
                cur.clear();
                in_q = true;
            }
        } else {
            cur.push(c);
        }
    }
    rows.extend(cur.split_whitespace().map(str::to_string));
    let mut out = Vec::new();
    if !areas.is_empty() {
        out.push(("grid-template-areas".into(), areas));
        if !rows.is_empty() {
            out.push(("grid-template-rows".into(), rows.join(" ")));
        }
        if !right.trim().is_empty() {
            out.push(("grid-template-columns".into(), right.trim().into()));
        }
    } else if !right.trim().is_empty() {
        // `<rows> / <cols>` sans areas.
        if !left.trim().is_empty() {
            out.push(("grid-template-rows".into(), left.trim().into()));
        }
        out.push(("grid-template-columns".into(), right.trim().into()));
    } else {
        // Forme bare sans `/` : invalide hors `none` (spec).
        return None;
    }
    (!out.is_empty()).then_some(out)
}

fn expand_gap(value: &str) -> Option<Vec<(String, String)>> {
    let mut it = value.split_whitespace();
    let row = it.next()?;
    let col = it.next().unwrap_or(row);
    Some(vec![
        ("row-gap".to_string(), row.to_string()),
        ("column-gap".to_string(), col.to_string()),
    ])
}

/// `column-rule` : `[<width>] [<style>] [<color>]` ordre
/// libre (css-multicol §5) — mêmes rôles que `outline`,
/// les mots-clés `thin`/`medium`/`thick` comptent comme
/// largeur (valeurs de `border-width`).
fn expand_column_rule(value: &str) -> Vec<(String, String)> {
    const WIDTH_KW: &[&str] = &["thin", "medium", "thick"];
    let mut out = Vec::new();
    let mut pos = 0usize;
    for tok in split_ws_top(value) {
        let t = tok.to_ascii_lowercase();
        if is_border_style(&t) {
            out.push(("column-rule-style".to_string(), t));
        } else if WIDTH_KW.contains(&t.as_str()) || parse_length(tok).is_some() {
            out.push(("column-rule-width".to_string(), tok.to_string()));
        } else {
            let tail = value[pos..].trim();
            if let Some(c) = parse_color(tail) {
                let _ = c;
                out.push(("column-rule-color".to_string(), tail.to_string()));
            }
            break;
        }
        pos += tok.len() + 1;
    }
    out
}

/// `columns` : `[<width>] [<count>]` ordre libre (CSS
/// Multi-column L1 §3.3) — `auto` couvre le volet absent.
fn expand_columns(value: &str) -> Vec<(String, String)> {
    let mut width = "auto";
    let mut count = "auto";
    for part in split_ws_top(value) {
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

/// Éclate sur les espaces de profondeur 0 — les fonctions
/// (`calc(…)`, `var(…)`, `min(…)`…) gardent leurs espaces
/// internes : un `split_whitespace` naïf cassait
/// `padding:calc(0.25rem * 4)` en trois morceaux invalides et
/// la déclaration entière sautait (Tailwind v4, R-208).
/// Token couleur **déclaré** — `parse_color` strict +
/// `currentcolor` + `light-dark(`/`var(` : ces fonctions ne
/// se résolvent qu'à la valeur calculée (cascade R-105/
/// substitution) — le shorthand doit les laisser passer
/// comme position couleur, sans quoi `background:
/// light-dark(#eee,#222)` est droppé en entier (mesuré
/// example.com — fond `#f0f0f2` perdu, R-219).
fn color_like(part: &str) -> bool {
    parse_color(part).is_some()
        || part == "currentcolor"
        || part.starts_with("light-dark(")
        || part.starts_with("var(")
}

fn split_ws_top(value: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut start = None;
    for (i, c) in value.char_indices() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth -= 1,
            _ => {}
        }
        if !c.is_whitespace() && start.is_none() {
            start = Some(i);
        }
        if c.is_whitespace() && depth <= 0 {
            if let Some(s) = start.take() {
                parts.push(&value[s..i]);
            }
        }
    }
    if let Some(s) = start {
        parts.push(&value[s..]);
    }
    parts
}

/// `margin`/`padding` : 1..4 valeurs → quatre longhands.
fn expand_box(name: &str, value: &str) -> Option<Vec<(String, String)>> {
    let parts: Vec<&str> = split_ws_top(value);
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
    let parts: Vec<&str> = split_ws_top(value);
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
    for part in split_ws_top(value) {
        if is_border_style(part) {
            style = Some(part);
        } else if color_like(part) {
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
    match split_ws_top(value).as_slice() {
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
    /// `background-blend-mode` (R-184) — keyword `<blend-mode>`
    /// de la couche, `normal` par défaut.
    blend: Option<String>,
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
        if color_like(part) {
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
        if BG_BLEND_TOKENS.contains(&part) {
            return self.blend.replace(part.to_string()).is_none();
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
            (
                "background-blend-mode".to_string(),
                self.blend.unwrap_or_else(|| "normal".to_string()),
            ),
        ]
    }
}

/// Keywords `<blend-mode>` du shorthand `background` (R-184)
/// — mêmes valeurs que `background-blend-mode` longhand.
const BG_BLEND_TOKENS: [&str; 16] = [
    "normal",
    "multiply",
    "screen",
    "overlay",
    "darken",
    "lighten",
    "color-dodge",
    "color-burn",
    "hard-light",
    "soft-light",
    "difference",
    "exclusion",
    "hue",
    "saturation",
    "color",
    "luminosity",
];

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
    let parts: Vec<&str> = split_ws_top(value);
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
/// `outline` shorthand → `outline-width`/`-style`/`-color`
/// — tokens indépendants d'ordre (comme `border` : longueur
/// → width, mot-clé de style → style, couleur → color) ;
/// tokens excédentaires ignorés.
fn expand_outline(value: &str) -> Vec<(String, String)> {
    const STYLES: &[&str] = &[
        "solid", "dotted", "dashed", "double", "groove", "ridge", "inset", "outset", "none",
        "hidden",
    ];
    let mut out = Vec::new();
    let mut pos = 0usize;
    for tok in split_ws_top(value) {
        let t = tok.to_ascii_lowercase();
        if STYLES.contains(&t.as_str()) {
            out.push(("outline-style".to_string(), t));
        } else if parse_length(tok).is_some() {
            out.push(("outline-width".to_string(), tok.to_string()));
        } else {
            // `rgb(255, 0, 0)` contient des espaces — la
            // queue complète depuis ce token va au parser
            // couleur (comme `parse_drop_shadow`).
            let tail = value[pos..].trim();
            if let Some(c) = parse_color(tail) {
                let _ = c;
                out.push(("outline-color".to_string(), tail.to_string()));
            }
            break;
        }
        pos += tok.len() + 1;
    }
    out
}

fn expand_list_style(value: &str) -> Option<Vec<(String, String)>> {
    let mut out = Vec::new();
    for part in split_ws_top(value) {
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
/// Propriétés logiques (css-logical-1 — R-124/R-183) : les
/// shorthands éclatent en longhands LOGIQUES ; la résolution
/// vers le repère de flux (mode + direction) se fait à la
/// cascade (`resolve_logical_props`). Retour :
/// `Some(vec![(logique, valeur)…])` ou `None`.
///
/// Couvert : `*-inline[-start|-end]` / `*-block[-start|-end]`
/// de `margin`, `padding`, `inset`, `border-*` ; `inline-size`,
/// `block-size` + `min-`/`max-` ; rayons `border-*-radius`.
fn expand_logical(name: &str, value: &str) -> Option<Vec<(String, String)>> {
    // R-183 : les longhands `*-inline-start`/`end`/`size`…
    // passent VERBATIM (None → insertion brute) — le mapping
    // logique→repère de flux se fait à la cascade
    // (`resolve_logical_props`), où le writing-mode de
    // l'élément ET celui du parent sont connus.
    // Shorthands `*-inline`/`*-block` : 1-2 valeurs répétées,
    // émises sous leurs noms LOGIQUES.
    let (a, b): (&str, &str) = match name {
        "margin-inline" => ("margin-inline-start", "margin-inline-end"),
        "margin-block" => ("margin-block-start", "margin-block-end"),
        "padding-inline" => ("padding-inline-start", "padding-inline-end"),
        "padding-block" => ("padding-block-start", "padding-block-end"),
        "inset-inline" => ("inset-inline-start", "inset-inline-end"),
        "inset-block" => ("inset-block-start", "inset-block-end"),
        "border-inline-width" => ("border-inline-start-width", "border-inline-end-width"),
        "border-inline-style" => ("border-inline-start-style", "border-inline-end-style"),
        "border-inline-color" => ("border-inline-start-color", "border-inline-end-color"),
        "border-block-width" => ("border-block-start-width", "border-block-end-width"),
        "border-block-style" => ("border-block-start-style", "border-block-end-style"),
        "border-block-color" => ("border-block-start-color", "border-block-end-color"),
        _ => {
            // `border-inline`/`border-block` shorthand complet —
            // `expand_border_parts` accepte les noms logiques
            // (le préfixe est recopié tel quel).
            return match name {
                "border-inline" | "border-block" => {
                    let (p1, p2) = if name == "border-inline" {
                        ("border-inline-start", "border-inline-end")
                    } else {
                        ("border-block-start", "border-block-end")
                    };
                    let mut out = expand_border_parts(p1, value);
                    out.extend(expand_border_parts(p2, value));
                    Some(out)
                }
                _ => None,
            };
        }
    };
    let parts: Vec<&str> = split_ws_top(value);
    let (v1, v2) = match parts.as_slice() {
        [v] => (*v, *v),
        [v1, v2] => (*v1, *v2),
        _ => return None,
    };
    Some(vec![
        (a.to_string(), v1.to_string()),
        (b.to_string(), v2.to_string()),
    ])
}

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
