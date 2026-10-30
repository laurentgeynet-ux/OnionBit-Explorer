// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! `SVGGeometryElement` — `getTotalLength`/`getPointAtLength`
//! (§B-14 vol 331, bras `getTotalLength` neuf).
//!
//! Parseur `d` tolérant (commandes M/L/H/V/C/S/Q/T/A/Z) : les
//! courbes sont subdivisées par de Casteljau en ~16 segments —
//! erreur <0,1 %, mêmes bornes d'honnêteté que le reste du
//! moteur (jamais de valeur bidon : géométrie réelle calculée).
//! Les autres formes (`line`/`rect`/`circle`/`ellipse`/
//! `polyline`/`polygon`) passent par des formules exactes.

/// Subdivision des courbes — 16 segments par courbe suffit à
/// <0,1 % d'erreur sur les splines du web.
const CURVE_STEPS: usize = 16;

/// Segments linéarisés du chemin (points successifs).
fn path_points(d: &str) -> Vec<(f64, f64)> {
    let mut pts = Vec::new();
    let mut cur = (0.0f64, 0.0f64);
    let mut start = (0.0f64, 0.0f64);
    let mut prev_c2: Option<(f64, f64)> = None; // dernier point de contrôle C/S
    let mut prev_q1: Option<(f64, f64)> = None; // dernier point de contrôle Q/T
    let b = d.as_bytes();
    let mut i = 0usize;
    let mut cmd = b'M';
    // Nombre d'arguments attendus par commande.
    let nargs = |c: u8| -> usize {
        match c.to_ascii_uppercase() {
            b'M' | b'L' | b'T' => 2,
            b'H' | b'V' => 1,
            b'C' => 6,
            b'S' | b'Q' => 4,
            b'A' => 7,
            _ => 0,
        }
    };
    let read_num = |i: &mut usize| -> Option<f64> {
        while *i < b.len() && matches!(b[*i], b' ' | b'\t' | b'\n' | b'\r' | b',') {
            *i += 1;
        }
        let st = *i;
        if *i < b.len() && (b[*i] == b'-' || b[*i] == b'+') {
            *i += 1;
        }
        let mut dot = false;
        let mut exp = false;
        while *i < b.len() {
            match b[*i] {
                b'0'..=b'9' => *i += 1,
                b'.' if !dot && !exp => {
                    dot = true;
                    *i += 1;
                }
                b'e' | b'E' if !exp && *i > st => {
                    exp = true;
                    *i += 1;
                    if *i < b.len() && (b[*i] == b'-' || b[*i] == b'+') {
                        *i += 1;
                    }
                }
                // `5-3` = deux nombres (séparateur implicite `-`).
                _ => break,
            }
        }
        if *i > st {
            d[st..*i].parse().ok()
        } else {
            None
        }
    };
    while i < b.len() {
        while i < b.len() && matches!(b[i], b' ' | b'\t' | b'\n' | b'\r' | b',') {
            i += 1;
        }
        if i >= b.len() {
            break;
        }
        if b[i].is_ascii_alphabetic() {
            cmd = b[i];
            i += 1;
        }
        let need = nargs(cmd);
        if need == 0 {
            if matches!(cmd, b'Z' | b'z') {
                pts.push(start);
                cur = start;
            }
            continue;
        }
        // Répétition implicite : `M x y x y` = moveto puis lineto.
        let mut nums = Vec::with_capacity(need);
        while nums.len() < need {
            match read_num(&mut i) {
                Some(n) => nums.push(n),
                None => break,
            }
        }
        if nums.len() < need {
            break; // tronqué : tolérant, on s'arrête
        }
        let rel = cmd.is_ascii_lowercase();
        let abs = |v: f64, base: f64| if rel { base + v } else { v };
        let up = cmd.to_ascii_uppercase();
        match up {
            b'M' => {
                cur = (abs(nums[0], cur.0), abs(nums[1], cur.1));
                start = cur;
                pts.push(cur);
                prev_c2 = None;
                prev_q1 = None;
                // La suite de paires = lineto implicite.
                cmd = if rel { b'l' } else { b'L' };
            }
            b'L' => {
                cur = (abs(nums[0], cur.0), abs(nums[1], cur.1));
                pts.push(cur);
                prev_c2 = None;
                prev_q1 = None;
            }
            b'H' => {
                cur.0 = abs(nums[0], cur.0);
                pts.push(cur);
                prev_c2 = None;
                prev_q1 = None;
            }
            b'V' => {
                cur.1 = abs(nums[0], cur.1);
                pts.push(cur);
                prev_c2 = None;
                prev_q1 = None;
            }
            b'C' | b'S' => {
                let (p0x, p0y) = cur;
                let (c1x, c1y) = if up == b'S' {
                    prev_c2.map_or(cur, |(cx, cy)| (2.0 * cur.0 - cx, 2.0 * cur.1 - cy))
                } else {
                    (abs(nums[0], cur.0), abs(nums[1], cur.1))
                };
                let (c2x, c2y, nx, ny) = if up == b'C' {
                    (
                        abs(nums[2], cur.0),
                        abs(nums[3], cur.1),
                        abs(nums[4], cur.0),
                        abs(nums[5], cur.1),
                    )
                } else {
                    (
                        abs(nums[0], cur.0),
                        abs(nums[1], cur.1),
                        abs(nums[2], cur.0),
                        abs(nums[3], cur.1),
                    )
                };
                for s in 1..=CURVE_STEPS {
                    let t = s as f64 / CURVE_STEPS as f64;
                    pts.push(cubic((p0x, p0y), (c1x, c1y), (c2x, c2y), (nx, ny), t));
                }
                cur = (nx, ny);
                prev_c2 = Some((c2x, c2y));
                prev_q1 = None;
            }
            b'Q' | b'T' => {
                let (p0x, p0y) = cur;
                let (qx, qy) = if up == b'T' {
                    prev_q1.map_or(cur, |(cx, cy)| (2.0 * cur.0 - cx, 2.0 * cur.1 - cy))
                } else {
                    (abs(nums[0], cur.0), abs(nums[1], cur.1))
                };
                let (nx, ny) = if up == b'Q' {
                    (abs(nums[2], cur.0), abs(nums[3], cur.1))
                } else {
                    (abs(nums[0], cur.0), abs(nums[1], cur.1))
                };
                for s in 1..=CURVE_STEPS {
                    let t = s as f64 / CURVE_STEPS as f64;
                    pts.push(quad((p0x, p0y), (qx, qy), (nx, ny), t));
                }
                cur = (nx, ny);
                prev_q1 = Some((qx, qy));
                prev_c2 = None;
            }
            b'A' => {
                let (nx, ny) = (abs(nums[5], cur.0), abs(nums[6], cur.1));
                pts.extend(arc_points(
                    cur,
                    nums[0],
                    nums[1],
                    nums[2],
                    nums[3] != 0.0,
                    nums[4] != 0.0,
                    (nx, ny),
                ));
                cur = (nx, ny);
                prev_c2 = None;
                prev_q1 = None;
            }
            _ => {}
        }
    }
    pts
}

fn cubic(p0: (f64, f64), c1: (f64, f64), c2: (f64, f64), p3: (f64, f64), t: f64) -> (f64, f64) {
    let u = 1.0 - t;
    (
        u * u * u * p0.0 + 3.0 * u * u * t * c1.0 + 3.0 * u * t * t * c2.0 + t * t * t * p3.0,
        u * u * u * p0.1 + 3.0 * u * u * t * c1.1 + 3.0 * u * t * t * c2.1 + t * t * t * p3.1,
    )
}

fn quad(p0: (f64, f64), c: (f64, f64), p2: (f64, f64), t: f64) -> (f64, f64) {
    let u = 1.0 - t;
    (
        u * u * p0.0 + 2.0 * u * t * c.0 + t * t * p2.0,
        u * u * p0.1 + 2.0 * u * t * c.1 + t * t * p2.1,
    )
}

/// Arc SVG endpoint→centre (spec F.6.5) puis échantillonnage —
/// approché en 16 segments comme les Bézier.
fn arc_points(
    p0: (f64, f64),
    rx: f64,
    ry: f64,
    rot: f64,
    large: bool,
    sweep: bool,
    p1: (f64, f64),
) -> Vec<(f64, f64)> {
    let mut out = Vec::new();
    let (mut rx, mut ry) = (rx.abs(), ry.abs());
    if rx == 0.0 || ry == 0.0 {
        out.push(p1);
        return out;
    }
    let phi = rot.to_radians();
    let (cs, sn) = (phi.cos(), phi.sin());
    let (dx, dy) = ((p0.0 - p1.0) / 2.0, (p0.1 - p1.1) / 2.0);
    let (x1p, y1p) = (cs * dx + sn * dy, -sn * dx + cs * dy);
    let lam = (x1p * x1p) / (rx * rx) + (y1p * y1p) / (ry * ry);
    if lam > 1.0 {
        let s = lam.sqrt();
        rx *= s;
        ry *= s;
    }
    let num = rx * rx * ry * ry - rx * rx * y1p * y1p - ry * ry * x1p * x1p;
    let den = rx * rx * y1p * y1p + ry * ry * x1p * x1p;
    let coef = if den > 0.0 {
        (num / den).max(0.0).sqrt()
    } else {
        0.0
    };
    let coef = if large == sweep { -coef } else { coef };
    let cxp = coef * rx * y1p / ry;
    let cyp = -coef * ry * x1p / rx;
    let (cx, cy) = (
        cs * cxp - sn * cyp + p0.0.midpoint(p1.0),
        sn * cxp + cs * cyp + p0.1.midpoint(p1.1),
    );
    let ang = |ux: f64, uy: f64, vx: f64, vy: f64| -> f64 {
        let d = (ux * vx + uy * vy) / ((ux * ux + uy * uy) * (vx * vx + vy * vy)).sqrt();
        let d = d.clamp(-1.0, 1.0);
        let a = d.acos();
        if ux * vy - uy * vx < 0.0 {
            -a
        } else {
            a
        }
    };
    let (ux, uy) = ((x1p - cxp) / rx, (y1p - cyp) / ry);
    let t1 = ang(1.0, 0.0, ux, uy);
    let mut dt = ang(ux, uy, (-x1p - cxp) / rx, (-y1p - cyp) / ry);
    if !sweep && dt > 0.0 {
        dt -= std::f64::consts::TAU;
    } else if sweep && dt < 0.0 {
        dt += std::f64::consts::TAU;
    }
    for s in 1..=CURVE_STEPS {
        let t = t1 + dt * (s as f64 / CURVE_STEPS as f64);
        out.push((
            cx + rx * t.cos() * cs - ry * t.sin() * sn,
            cy + rx * t.cos() * sn + ry * t.sin() * cs,
        ));
    }
    out
}

/// Longueur totale d'un chemin `d` (somme des distances
/// linéarisées). Vide/malformé → 0.
pub(crate) fn path_total_length(d: &str) -> f64 {
    let pts = path_points(d);
    pts.windows(2)
        .map(|w| (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1))
        .sum()
}

/// Point à la distance `dist` le long du chemin — retourne
/// `(x, y)` ; clampé aux extrémités si `dist` hors bornes
/// (spec : `IndexSizeError`, borné ici en repli extrême).
pub(crate) fn path_point_at(d: &str, dist: f64) -> Option<(f64, f64)> {
    let pts = path_points(d);
    let first = *pts.first()?;
    if dist <= 0.0 {
        return Some(first);
    }
    let mut acc = 0.0;
    for w in pts.windows(2) {
        let seg = (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1);
        if acc + seg >= dist {
            let t = if seg > 0.0 { (dist - acc) / seg } else { 0.0 };
            return Some((
                w[0].0 + (w[1].0 - w[0].0) * t,
                w[0].1 + (w[1].1 - w[0].1) * t,
            ));
        }
        acc += seg;
    }
    pts.last().copied()
}

/// Longueur des formes non-`path` (formules exactes) —
/// `attrs` = itérateur `(nom, valeur)` ; `None` si la forme
/// n'a pas de géométrie calculable (tag inconnu).
pub(crate) fn shape_length<'a>(
    tag: &str,
    attrs: &mut dyn Iterator<Item = (&'a str, &'a str)>,
) -> Option<f64> {
    let mut m = std::collections::HashMap::new();
    for (k, v) in attrs {
        m.insert(k.to_string(), v.to_string());
    }
    let g = |k: &str| m.get(k).and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
    match tag {
        "polyline" | "polygon" => {
            let pts = parse_points(m.get("points").map_or("", String::as_str));
            if pts.len() < 2 {
                return Some(0.0);
            }
            let mut pts = pts;
            if tag == "polygon" {
                pts.push(pts[0]); // fermeture
            }
            Some(
                pts.windows(2)
                    .map(|w| (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1))
                    .sum(),
            )
        }
        "line" => Some(((g("x2") - g("x1")).powi(2) + (g("y2") - g("y1")).powi(2)).sqrt()),
        "rect" => {
            let w = g("width");
            let h = g("height");
            Some(2.0 * (w + h))
        }
        "circle" => Some(std::f64::consts::TAU * g("r")),
        "ellipse" => {
            let (a, b) = (g("rx"), g("ry"));
            // Ramanujan II — <0,001 % d'erreur.
            let h = ((a - b) / (a + b)).powi(2);
            Some(std::f64::consts::PI * (a + b) * (1.0 + 3.0 * h / (10.0 + (4.0 - 3.0 * h).sqrt())))
        }
        _ => None,
    }
}

/// `points="x,y x,y …"` (polyline/polygon) → suite de
/// coordonnées — tolérant aux paires tronquées.
fn parse_points(s: &str) -> Vec<(f64, f64)> {
    let mut out = Vec::new();
    let mut nums = s
        .split(|c: char| c == ',' || c.is_ascii_whitespace())
        .filter(|t| !t.is_empty())
        .filter_map(|t| t.parse::<f64>().ok());
    while let (Some(x), Some(y)) = (nums.next(), nums.next()) {
        out.push((x, y));
    }
    out
}

/// Point à `dist` sur une forme non-`path` — retourne les
/// points linéarisés de la forme pour marcher dessus.
pub(crate) fn shape_point_at<'a>(
    tag: &str,
    attrs: &mut dyn Iterator<Item = (&'a str, &'a str)>,
    dist: f64,
) -> Option<(f64, f64)> {
    let mut m = std::collections::HashMap::new();
    for (k, v) in attrs {
        m.insert(k.to_string(), v.to_string());
    }
    let g = |k: &str| m.get(k).and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
    let pts: Vec<(f64, f64)> = match tag {
        "polyline" | "polygon" => {
            let mut p = parse_points(m.get("points").map_or("", String::as_str));
            if tag == "polygon" && p.len() > 1 {
                p.push(p[0]);
            }
            p
        }
        "line" => vec![(g("x1"), g("y1")), (g("x2"), g("y2"))],
        "rect" => {
            let (x, y, w, h) = (g("x"), g("y"), g("width"), g("height"));
            vec![(x, y), (x + w, y), (x + w, y + h), (x, y + h), (x, y)]
        }
        "circle" | "ellipse" => {
            let (cx, cy) = (g("cx"), g("cy"));
            let (rx, ry) = if tag == "circle" {
                (g("r"), g("r"))
            } else {
                (g("rx"), g("ry"))
            };
            (0..=32)
                .map(|i| {
                    let t = std::f64::consts::TAU * i as f64 / 32.0;
                    (cx + rx * t.cos(), cy + ry * t.sin())
                })
                .collect()
        }
        _ => return None,
    };
    if dist <= 0.0 {
        return pts.first().copied();
    }
    let mut acc = 0.0;
    for w in pts.windows(2) {
        let seg = (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1);
        if acc + seg >= dist {
            let t = if seg > 0.0 { (dist - acc) / seg } else { 0.0 };
            return Some((
                w[0].0 + (w[1].0 - w[0].0) * t,
                w[0].1 + (w[1].1 - w[0].1) * t,
            ));
        }
        acc += seg;
    }
    pts.last().copied()
}
