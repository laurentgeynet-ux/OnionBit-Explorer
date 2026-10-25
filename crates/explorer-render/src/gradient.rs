// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Rasterisation des dégradés CSS (`linear-gradient`) — issue
//! #10 : la valeur n'était ni parsée ni peinte, la déclaration
//! `background` tombait entière (barre tricolore NPR, bandeau
//! lobste.rs absents).
//!
//! Un dégradé n'a pas de taille intrinsèque : il est rendu en
//! un bitmap aux dimensions de la **zone de positionnement**
//! (padding box) qui emprunte ensuite toute la machinerie
//! `BackgroundImage` existante — position, `repeat`, clip sur
//! la border box. `repeating-linear-gradient` est couvert par
//! repliement du paramètre ; `radial-`/`conic-gradient` sont
//! acceptés par le parseur mais non peints (résidu documenté).

use explorer_css::value::{parse_color, parse_length, Color};

use crate::image::DecodedImage;

/// Borne d'hospitalité sur le bitmap produit (contenu hostile
/// possible : `width:1e6px` — §8 « budgets »). Le dégradé
/// reste correct sur la partie peinte ; au-delà la tuile
/// répète, dégradation acceptée.
const MAX_GRADIENT_DIM: f32 = 4096.0;

/// `linear-gradient(…)` d'une couche `background-image` →
/// bitmap `area_w × area_h` (l'aire de positionnement), `None`
/// si la couche n'est pas un dégradé linéaire ou est malformée.
/// `fs` = `font-size` calculée pour les stops en unités
/// font-relatives.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)]
pub(crate) fn decode_linear_gradient(
    layer: &str,
    area_w: f32,
    area_h: f32,
    fs: f64,
) -> Option<DecodedImage> {
    let low = layer.to_ascii_lowercase();
    let repeating = low.contains("repeating-linear-gradient(");
    if !repeating && !low.contains("linear-gradient(") {
        return None;
    }
    let open = low.find("gradient(")? + "gradient(".len() - 1;
    // Parenthèse fermante assortie à l'appel `gradient(`.
    let mut depth = 0i32;
    let mut close = layer.len();
    for (i, ch) in layer.char_indices().skip_while(|(i, _)| *i <= open) {
        match ch {
            '(' => depth += 1,
            ')' => {
                if depth == 0 {
                    close = i;
                    break;
                }
                depth -= 1;
            }
            _ => {}
        }
    }
    let args = split_top_commas(&layer[open + 1..close]);
    if args.is_empty() {
        return None;
    }
    // Premier argument : direction (`to <side>` / `<angle>`)
    // ou directement le premier stop couleur.
    let mut axis = (0.0f32, 1.0f32); // défaut spec : `to bottom`.
    let mut stop_args: &[&str] = &args;
    if let Some(dir) = gradient_direction(args[0]).or_else(|| gradient_angle(args[0])) {
        axis = dir;
        stop_args = &args[1..];
    }
    if stop_args.is_empty() {
        return None;
    }

    // Projection des 4 coins de la zone sur l'axe : la ligne de
    // dégradé parcourt exactement la zone (spec : le segment
    // est dimensionné pour couvrir la boîte).
    let width = area_w.clamp(1.0, MAX_GRADIENT_DIM);
    let height = area_h.clamp(1.0, MAX_GRADIENT_DIM);
    let (dx, dy) = axis;
    let proj = |px: f32, py: f32| px * dx + py * dy;
    let pmin = proj(0.0, 0.0)
        .min(proj(width, 0.0))
        .min(proj(0.0, height))
        .min(proj(width, height));
    let pmax = proj(0.0, 0.0)
        .max(proj(width, 0.0))
        .max(proj(0.0, height))
        .max(proj(width, height));
    let span = (pmax - pmin).max(1.0);

    let stops = resolve_stops(stop_args, span, fs)?;
    let (iw, ih) = (width as u32, height as u32);
    let mut rgba = vec![0u8; iw as usize * ih as usize * 4];
    for row in 0..ih {
        for col in 0..iw {
            // Échantillon au centre du pixel.
            let t0 = (proj(col as f32 + 0.5, row as f32 + 0.5) - pmin) / span;
            let frac = if repeating {
                t0.rem_euclid(1.0)
            } else {
                t0.clamp(0.0, 1.0)
            };
            let color = sample_stops(&stops, frac);
            let off = (row as usize * iw as usize + col as usize) * 4;
            rgba[off] = color.r;
            rgba[off + 1] = color.g;
            rgba[off + 2] = color.b;
            rgba[off + 3] = color.a;
        }
    }
    Some(DecodedImage {
        width: iw,
        height: ih,
        rgba,
    })
}

/// Éclate sur les virgules hors parenthèses (les couleurs
/// `rgb(…,…,…)`/`rgba(…)` et fonctions internes sont
/// protégées).
fn split_top_commas(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth = 0u32;
    let mut start = 0usize;
    for (i, ch) in s.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                out.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&s[start..]);
    out
}

/// Éclate sur les espaces hors parenthèses (un stop
/// `rgba(255, 0, 0, .5) 30%` reste deux atomes : couleur puis
/// position).
fn split_top_ws(s: &str) -> Vec<&str> {
    let mut toks = Vec::new();
    let mut depth = 0u32;
    let mut start = 0usize;
    let mut in_tok = false;
    for (i, ch) in s.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            _ => {}
        }
        if ch.is_whitespace() && depth == 0 {
            if in_tok {
                toks.push(&s[start..i]);
                in_tok = false;
            }
        } else if !in_tok {
            start = i;
            in_tok = true;
        }
    }
    if in_tok {
        toks.push(&s[start..]);
    }
    toks
}

/// `to <side>` → vecteur d'axe du dégradé en coords écran
/// (y vers le bas). `None` si l'argument n'est pas une
/// direction keyword.
fn gradient_direction(arg: &str) -> Option<(f32, f32)> {
    let mut toks = split_top_ws(arg);
    if toks.first().copied() != Some("to") {
        return None;
    }
    toks.remove(0);
    let (mut dx, mut dy) = (0.0f32, 0.0f32);
    for t in toks {
        match t {
            "top" => dy -= 1.0,
            "bottom" => dy += 1.0,
            "left" => dx -= 1.0,
            "right" => dx += 1.0,
            _ => return None,
        }
    }
    if dx == 0.0 && dy == 0.0 {
        return None;
    }
    let n = (dx * dx + dy * dy).sqrt();
    Some((dx / n, dy / n))
}

/// `<angle>` → vecteur d'axe : la spec mesure l'angle dans le
/// sens horaire depuis « up » (`0deg` = `to top`, `90deg` = `to
/// right`) → `(sin θ, −cos θ)`. `deg`/`grad`/`rad`/`turn`.
fn gradient_angle(arg: &str) -> Option<(f32, f32)> {
    let a = arg.trim();
    let deg = if let Some(v) = a.strip_suffix("deg") {
        v.parse::<f32>().ok()?
    } else if let Some(v) = a.strip_suffix("grad") {
        v.parse::<f32>().ok()? * 0.9
    } else if let Some(v) = a.strip_suffix("turn") {
        v.parse::<f32>().ok()? * 360.0
    } else {
        let v = a.strip_suffix("rad")?;
        v.parse::<f32>().ok()?.to_degrees()
    };
    let rad = deg.to_radians();
    Some((rad.sin(), -rad.cos()))
}

/// Stops résolus `(fraction 0..=1, couleur)` triés — la spec
/// distribue les positions absentes régulièrement entre les
/// positions explicites voisines et impose la monotonie (un
/// stop ne précède jamais son prédécesseur).
#[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
fn resolve_stops(args: &[&str], span_px: f32, fs: f64) -> Option<Vec<(f32, Color)>> {
    let mut parsed: Vec<(Color, Option<f32>)> = Vec::with_capacity(args.len());
    for a in args {
        let toks = split_top_ws(a.trim());
        let color = toks.first().and_then(|t| parse_color(t))?;
        let pos = toks.get(1).and_then(|t| stop_position(t, span_px, fs));
        parsed.push((color, pos));
    }
    if parsed.is_empty() {
        return None;
    }
    let n = parsed.len();
    let mut pos: Vec<Option<f32>> = parsed.iter().map(|(_, p)| *p).collect();
    // Bornes implicites : premier stop = 0, dernier = 1 (spec
    // §images-3 « default position »).
    if pos[0].is_none() {
        pos[0] = Some(0.0);
    }
    if pos[n - 1].is_none() {
        pos[n - 1] = Some(1.0);
    }
    // Répartition linéaire des trous entre positions explicites.
    let mut i = 0;
    while i < n {
        if pos[i].is_some() {
            i += 1;
            continue;
        }
        let mut j = i;
        while j < n && pos[j].is_none() {
            j += 1;
        }
        let lo = pos[i - 1]?;
        let hi = if j < n { pos[j]? } else { 1.0 };
        let step = (hi - lo) / (j - i + 1) as f32;
        for (k, slot) in pos[i..j.min(n)].iter_mut().enumerate() {
            *slot = Some(lo + step * (k + 1) as f32);
        }
        i = j;
    }
    let mut out: Vec<(f32, Color)> = Vec::with_capacity(n);
    let mut prev = f32::MIN;
    for (k, (color, _)) in parsed.iter().enumerate() {
        let p = pos[k].unwrap_or(0.0).max(prev);
        prev = p;
        out.push((p, *color));
    }
    Some(out)
}

/// Position de stop : `<%>` → fraction de la ligne ;
/// `<angle>` (`deg`/`grad`/`rad`/`turn`, stops coniques) →
/// fraction de tour ; `<length>` absolue → px/étendue ;
/// font-relative → `fs`.
#[allow(clippy::cast_possible_truncation)]
fn stop_position(t: &str, span_px: f32, fs: f64) -> Option<f32> {
    if let Some(p) = t.strip_suffix('%') {
        return p.parse::<f32>().ok().map(|v| v / 100.0);
    }
    if let Some(deg) = angle_deg(t) {
        return Some(deg / 360.0);
    }
    let l = parse_length(t)?;
    let px = match l.unit.px_factor() {
        Some(f) => l.value * f,
        None => l.value * fs * 0.5, // ex/ch ≈ 0.5em, approximation.
    };
    Some(px as f32 / span_px)
}

/// Couleur du dégradé au paramètre `t ∈ [0,1]` — interpolation
/// linéaire par canal entre stops adjacents.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn sample_stops(stops: &[(f32, Color)], t: f32) -> Color {
    if stops.is_empty() {
        return Color::BLACK;
    }
    if t <= stops[0].0 {
        return stops[0].1;
    }
    if let Some(last) = stops.last() {
        if t >= last.0 {
            return last.1;
        }
    }
    for w in stops.windows(2) {
        let (p0, c0) = w[0];
        let (p1, c1) = w[1];
        if t >= p0 && t <= p1 {
            let f = if p1 > p0 { (t - p0) / (p1 - p0) } else { 0.0 };
            let lerp = |a: u8, b: u8| {
                (f64::from(a) + (f64::from(b) - f64::from(a)) * f64::from(f))
                    .round()
                    .clamp(0.0, 255.0) as u8
            };
            return Color {
                r: lerp(c0.r, c1.r),
                g: lerp(c0.g, c1.g),
                b: lerp(c0.b, c1.b),
                a: lerp(c0.a, c1.a),
            };
        }
    }
    stops.last().map_or(Color::BLACK, |(_, c)| *c)
}

// ── R-104 : radial-gradient / conic-gradient ────────────────

/// Point d'entrée unique des couches dégradé : `linear-`,
/// `radial-`, `conic-gradient` (+ variantes `repeating-*`) →
/// bitmap `area_w × area_h`, `None` si non dégradé/malformé.
#[allow(clippy::cast_precision_loss)]
pub(crate) fn decode_gradient(
    layer: &str,
    area_w: f32,
    area_h: f32,
    fs: f64,
) -> Option<DecodedImage> {
    let low = layer.to_ascii_lowercase();
    if low.contains("conic-gradient(") {
        return decode_conic(layer, &low, area_w, area_h, fs);
    }
    if low.contains("radial-gradient(") {
        return decode_radial(layer, &low, area_w, area_h, fs);
    }
    decode_linear_gradient(layer, area_w, area_h, fs)
}

/// `args[0]` après `gradient(` : la virgule de tête sépare le
/// prologue (`shape/size/at`/`from`) des stops.
fn gradient_args<'a>(layer: &'a str, low: &str) -> Option<(&'a str, Vec<&'a str>)> {
    let open = low.find("gradient(")? + "gradient(".len() - 1;
    let mut depth = 0i32;
    let mut close = layer.len();
    for (i, ch) in layer.char_indices().skip_while(|(i, _)| *i <= open) {
        match ch {
            '(' => depth += 1,
            ')' if depth == 0 => {
                close = i;
                break;
            }
            ')' => depth -= 1,
            _ => {}
        }
    }
    let args = split_top_commas(&layer[open + 1..close]);
    (!args.is_empty()).then(|| (args[0], args))
}

/// `at <x> [<y>]` → centre `(px, py)` dans l'aire — mots-clés
/// `left|right|top|bottom|center`, `<%>` et `<length>` (le
/// premier token = x, le second = y ; un seul `top`/`bottom`
/// seul compte pour y, l'autre axe centre).
#[allow(clippy::cast_possible_truncation)]
fn at_position(arg: &str, w: f32, h: f32, fs: f64) -> (f32, f32) {
    let toks = split_top_ws(arg.trim());
    let mut cx = w / 2.0;
    let mut cy = h / 2.0;
    let mut seen_x = false;
    let mut seen_y = false;
    for t in toks {
        match t {
            "left" if !seen_x => {
                cx = 0.0;
                seen_x = true;
            }
            "right" if !seen_x => {
                cx = w;
                seen_x = true;
            }
            "top" if !seen_y => {
                cy = 0.0;
                seen_y = true;
            }
            "bottom" if !seen_y => {
                cy = h;
                seen_y = true;
            }
            "center" => {}
            _ => {
                let px = if let Some(p) = t.strip_suffix('%') {
                    p.parse::<f32>().ok().map(|v| v / 100.0)
                } else {
                    parse_length(t).map(|l| {
                        (match l.unit.px_factor() {
                            Some(f) => l.value * f,
                            None => l.value * fs * 0.5,
                        }) as f32
                    })
                };
                if let Some(v) = px {
                    if !seen_x {
                        cx = if t.ends_with('%') { v * w } else { v };
                        seen_x = true;
                    } else if !seen_y {
                        cy = if t.ends_with('%') { v * h } else { v };
                        seen_y = true;
                    }
                }
            }
        }
    }
    (cx, cy)
}

/// Rayons de l'ellipse finale du dégradé radial — keywords
/// spec (`*-side`/`*-corner`) ou longueurs explicites ; le
/// `√2` de `*-corner` ellipse est la formule littérale de
/// css-images-3 §3.2.
#[allow(clippy::similar_names)] // dx_min/dy_min : axes jumeaux
fn radial_radii(
    shape_circle: bool,
    size_kw: &str,
    explicit: Option<(f32, f32)>,
    cx: f32,
    cy: f32,
    w: f32,
    h: f32,
) -> (f32, f32) {
    if let Some(r) = explicit {
        return r;
    }
    let dx_min = cx.min(w - cx).max(0.0);
    let dx_max = cx.max(w - cx).max(0.0);
    let dy_min = cy.min(h - cy).max(0.0);
    let dy_max = cy.max(h - cy).max(0.0);
    let corner_dist = |dx: f32, dy: f32| (dx * dx + dy * dy).sqrt();
    match size_kw {
        "closest-side" if shape_circle => (dx_min.min(dy_min), dx_min.min(dy_min)),
        "farthest-side" if shape_circle => (dx_max.max(dy_max), dx_max.max(dy_max)),
        "closest-corner" if shape_circle => {
            let r = [
                corner_dist(cx, cy),
                corner_dist(w - cx, cy),
                corner_dist(cx, h - cy),
                corner_dist(w - cx, h - cy),
            ]
            .into_iter()
            .fold(f32::MAX, f32::min);
            (r, r)
        }
        "closest-side" => (dx_min, dy_min),
        "farthest-side" => (dx_max, dy_max),
        "closest-corner" => (
            dx_min * std::f32::consts::SQRT_2,
            dy_min * std::f32::consts::SQRT_2,
        ),
        // `farthest-corner` — défaut spec.
        _ if shape_circle => {
            let r = [
                corner_dist(cx, cy),
                corner_dist(w - cx, cy),
                corner_dist(cx, h - cy),
                corner_dist(w - cx, h - cy),
            ]
            .into_iter()
            .fold(0.0f32, f32::max);
            (r, r)
        }
        _ => (
            dx_max * std::f32::consts::SQRT_2,
            dy_max * std::f32::consts::SQRT_2,
        ),
    }
}

/// `radial-gradient([shape size at pos,] stops…)` → bitmap.
/// Prologue MVP : `circle`/`ellipse`, `closest|farthest-*
/// side|corner`, `at <pos>`, rayons explicites (1 pour
/// `circle`, 2 pour `ellipse`, `<%>` ellipse seulement,
/// rapportés aux axes de l'aire).
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)]
fn decode_radial(
    layer: &str,
    low: &str,
    area_w: f32,
    area_h: f32,
    fs: f64,
) -> Option<DecodedImage> {
    let repeating = low.contains("repeating-radial-gradient(");
    let (prologue, args) = gradient_args(layer, low)?;
    // Le prologue est-il un prologue de forme (vs un stop) ?
    // Heuristique : le premier token n'est pas une couleur.
    let toks = split_top_ws(prologue.trim());
    let prologue_is_shape = parse_color(toks.first().copied().unwrap_or("")).is_none();
    let stop_args: &[&str] = if prologue_is_shape && args.len() > 1 {
        &args[1..]
    } else {
        &args[..]
    };
    if stop_args.is_empty() {
        return None;
    }
    let mut circle = false;
    let mut size_kw = "farthest-corner";
    // Rayons explicites (`None` = px, `Some` = fraction %).
    let mut radii: Vec<Result<f32, f32>> = Vec::new();
    let mut past_at = false;
    for t in &toks {
        if *t == "at" {
            past_at = true;
            continue;
        }
        if past_at || !prologue_is_shape {
            break;
        }
        match *t {
            "circle" => circle = true,
            "ellipse" => circle = false,
            "closest-side" | "farthest-side" | "closest-corner" | "farthest-corner" => {
                size_kw = t;
            }
            _ => {
                if let Some(p) = t.strip_suffix('%') {
                    if let Ok(v) = p.parse::<f32>() {
                        radii.push(Ok(v / 100.0));
                        continue;
                    }
                }
                if let Some(l) = parse_length(t) {
                    radii.push(Err(match l.unit.px_factor() {
                        Some(f) => (l.value * f) as f32,
                        None => (l.value * fs * 0.5) as f32,
                    }));
                }
            }
        }
    }
    let (mut cx, mut cy) = (area_w / 2.0, area_h / 2.0);
    if prologue_is_shape {
        if let Some(i) = prologue.trim().find(" at ") {
            let (x, y) = at_position(&prologue.trim()[i + 4..], area_w, area_h, fs);
            cx = x;
            cy = y;
        }
    }
    let width = area_w.clamp(1.0, MAX_GRADIENT_DIM);
    let height = area_h.clamp(1.0, MAX_GRADIENT_DIM);
    // `Ok(f)` = fraction d'axe, `Err(px)` = absolu.
    let px_of = |r: Result<f32, f32>, axis: f32| match r {
        Ok(f) => f * axis,
        Err(px) => px,
    };
    let explicit = if radii.len() >= 2 && !circle {
        Some((px_of(radii[0], width), px_of(radii[1], height)))
    } else {
        radii.first().map(|&r| (px_of(r, width), px_of(r, width)))
    };
    let (rx, ry) = radial_radii(circle, size_kw, explicit, cx, cy, width, height);
    let (rx, ry) = (rx.max(0.01), ry.max(0.01));
    let stops = resolve_stops(stop_args, 1.0, fs)?;
    let (iw, ih) = (width as u32, height as u32);
    let mut rgba = vec![0u8; iw as usize * ih as usize * 4];
    for row in 0..ih {
        for col in 0..iw {
            let dx = (col as f32 + 0.5 - cx) / rx;
            let dy = (row as f32 + 0.5 - cy) / ry;
            let t0 = (dx * dx + dy * dy).sqrt();
            let frac = if repeating {
                t0.rem_euclid(1.0)
            } else {
                t0.clamp(0.0, 1.0)
            };
            let color = sample_stops(&stops, frac);
            let off = (row as usize * iw as usize + col as usize) * 4;
            rgba[off] = color.r;
            rgba[off + 1] = color.g;
            rgba[off + 2] = color.b;
            rgba[off + 3] = color.a;
        }
    }
    Some(DecodedImage {
        width: iw,
        height: ih,
        rgba,
    })
}

/// `conic-gradient([from θ] [at pos,] stops…)` → bitmap :
/// fraction = angle horaire depuis le haut (+ `from`), les
/// stops en `<%>` ou `<angle>` (`deg`/`grad`/`rad`/`turn`).
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)]
fn decode_conic(layer: &str, low: &str, area_w: f32, area_h: f32, fs: f64) -> Option<DecodedImage> {
    let repeating = low.contains("repeating-conic-gradient(");
    let (prologue, args) = gradient_args(layer, low)?;
    let toks = split_top_ws(prologue.trim());
    let prologue_is_shape = parse_color(toks.first().copied().unwrap_or("")).is_none();
    let stop_args: &[&str] = if prologue_is_shape && args.len() > 1 {
        &args[1..]
    } else {
        &args[..]
    };
    if stop_args.is_empty() {
        return None;
    }
    let mut from_deg = 0.0f32;
    let mut cx = area_w / 2.0;
    let mut cy = area_h / 2.0;
    if prologue_is_shape {
        let mut i = 0;
        while i < toks.len() {
            match toks[i] {
                "from" => {
                    if let Some(d) = toks.get(i + 1).and_then(|a| angle_deg(a)) {
                        from_deg = d;
                    }
                    i += 1;
                }
                "at" => {
                    let rest = toks[i + 1..].join(" ");
                    let (x, y) = at_position(&rest, area_w, area_h, fs);
                    cx = x;
                    cy = y;
                    break;
                }
                _ => {}
            }
            i += 1;
        }
    }
    let stops = resolve_stops(stop_args, 1.0, fs)?;
    let width = area_w.clamp(1.0, MAX_GRADIENT_DIM);
    let height = area_h.clamp(1.0, MAX_GRADIENT_DIM);
    let (iw, ih) = (width as u32, height as u32);
    let mut rgba = vec![0u8; iw as usize * ih as usize * 4];
    for row in 0..ih {
        for col in 0..iw {
            let dx = col as f32 + 0.5 - cx;
            let dy = row as f32 + 0.5 - cy;
            // 0deg = haut, sens horaire : atan2(dx, -dy).
            let ang = dx.atan2(-dy).to_degrees();
            let t0 = (ang + from_deg).rem_euclid(360.0) / 360.0;
            let frac = if repeating {
                t0.rem_euclid(1.0)
            } else {
                t0.clamp(0.0, 1.0)
            };
            let color = sample_stops(&stops, frac);
            let off = (row as usize * iw as usize + col as usize) * 4;
            rgba[off] = color.r;
            rgba[off + 1] = color.g;
            rgba[off + 2] = color.b;
            rgba[off + 3] = color.a;
        }
    }
    Some(DecodedImage {
        width: iw,
        height: ih,
        rgba,
    })
}

/// `<angle>` → degrés (`deg`/`grad`/`rad`/`turn`).
fn angle_deg(a: &str) -> Option<f32> {
    let v = a.trim();
    if let Some(d) = v.strip_suffix("deg") {
        d.parse().ok()
    } else if let Some(d) = v.strip_suffix("grad") {
        d.parse::<f32>().ok().map(|x| x * 0.9)
    } else if let Some(d) = v.strip_suffix("turn") {
        d.parse::<f32>().ok().map(|x| x * 360.0)
    } else {
        v.strip_suffix("rad")?
            .parse::<f32>()
            .ok()
            .map(f32::to_degrees)
    }
}
