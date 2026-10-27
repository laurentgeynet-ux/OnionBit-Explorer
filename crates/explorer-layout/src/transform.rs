// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Résolution de `transform`/`transform-origin` en matrice
//! affine 2D — **source unique** pour la peinture
//! (`explorer-render` l'y délègue) et le hit-test
//! (`hit.rs`), qui applique la matrice **inverse** au point
//! sondé (vol : zones cliquables décalées sur éléments
//! transformés).
//!
//! `Affine` suit la convention tiny-skia : `x' = a·x + c·y +
//! e`, `y' = b·x + d·y + f` — soit `Transform::from_row(a,
//! b, c, d, e, f)` côté raster, et les composantes du
//! `matrix(a,b,c,d,e,f)` CSS dans l'ordre.

use explorer_css::value::parse_length;
use explorer_css::ComputedStyle;

use crate::boxtree::{LayoutBox, Rect};

/// Matrice affine 2D `[a c e ; b d f]` — composantes nommées
/// comme la fonction `matrix()` CSS (a·x + c·y + e,
/// b·x + d·y + f).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Affine {
    /// `sx` — coefficient x→x.
    pub a: f32,
    /// `ky` — coefficient x→y.
    pub b: f32,
    /// `kx` — coefficient y→x.
    pub c: f32,
    /// `sy` — coefficient y→y.
    pub d: f32,
    /// `tx` — translation x.
    pub e: f32,
    /// `ty` — translation y.
    pub f: f32,
}

impl Affine {
    /// Matrice identité.
    pub const IDENTITY: Self = Self {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        e: 0.0,
        f: 0.0,
    };

    /// Translation pure.
    #[must_use]
    pub fn translate(tx: f32, ty: f32) -> Self {
        Self {
            e: tx,
            f: ty,
            ..Self::IDENTITY
        }
    }

    /// Échelle `(sx, sy)`.
    #[must_use]
    pub fn scale(sx: f32, sy: f32) -> Self {
        Self {
            a: sx,
            d: sy,
            ..Self::IDENTITY
        }
    }

    /// Rotation d'angle `deg` (degrés, sens horaire — repère
    /// y vers le bas comme tiny-skia `from_rotate`).
    #[must_use]
    #[allow(clippy::cast_possible_truncation)]
    pub fn rotate(deg: f32) -> Self {
        let r = f64::from(deg).to_radians();
        let (sin, cos) = (r.sin() as f32, r.cos() as f32);
        Self {
            a: cos,
            b: sin,
            c: -sin,
            d: cos,
            e: 0.0,
            f: 0.0,
        }
    }

    /// Inclinaison `(tan ax, tan ay)` — `from_skew(kx, ky)`
    /// de tiny-skia : `kx` penche le long de x, `ky` le long
    /// de y.
    #[must_use]
    pub fn skew(kx: f32, ky: f32) -> Self {
        Self {
            b: ky,
            c: kx,
            ..Self::IDENTITY
        }
    }

    /// `rhs ∘ self` — `self` appliquée au point **d'abord**,
    /// `rhs` ensuite : même sémantique que
    /// `Transform::post_concat` de tiny-skia
    /// (`concat(other, self)` = `other·self`, points en
    /// colonne).
    #[must_use]
    pub fn post_concat(self, rhs: Self) -> Self {
        Self {
            a: rhs.a.mul_add(self.a, rhs.c * self.b),
            b: rhs.b.mul_add(self.a, rhs.d * self.b),
            c: rhs.a.mul_add(self.c, rhs.c * self.d),
            d: rhs.b.mul_add(self.c, rhs.d * self.d),
            e: rhs.a.mul_add(self.e, rhs.c * self.f) + rhs.e,
            f: rhs.b.mul_add(self.e, rhs.d * self.f) + rhs.f,
        }
    }

    /// Applique la matrice au point `(x, y)`.
    #[must_use]
    pub fn apply(self, x: f32, y: f32) -> (f32, f32) {
        (
            self.a.mul_add(x, self.c.mul_add(y, self.e)),
            self.b.mul_add(x, self.d.mul_add(y, self.f)),
        )
    }

    /// Inverse — `None` si le déterminant est (quasi) nul :
    /// plan aplati (`scale(0)`), rien de cliquable dedans.
    #[must_use]
    pub fn invert(self) -> Option<Self> {
        let det = self.a * self.d - self.b * self.c;
        if det.abs() <= f32::EPSILON {
            return None;
        }
        let inv = 1.0 / det;
        Some(Self {
            a: self.d * inv,
            b: -self.b * inv,
            c: -self.c * inv,
            d: self.a * inv,
            e: (self.c * self.f - self.d * self.e) * inv,
            f: (self.b * self.e - self.a * self.f) * inv,
        })
    }
}

/// Longueur d'une fonction `transform` → px : absolues via
/// `px_factor`, `%` relatif à `basis` (taille propre de la
/// boîte — spec : reference box = border box).
fn tf_len_px(s: &str, basis: f32) -> Option<f64> {
    let t = s.trim();
    if let Some(p) = t.strip_suffix('%') {
        return p
            .trim()
            .parse::<f64>()
            .ok()
            .map(|v| v * f64::from(basis) / 100.0);
    }
    parse_length(t).and_then(|l| l.unit.px_factor().map(|f| l.value * f))
}

/// Angle `deg`/`rad`/`turn`/`grad` → radians.
fn tf_angle(s: &str) -> Option<f64> {
    let t = s.trim().to_ascii_lowercase();
    let (num, k) = ["deg", "grad", "rad", "turn"]
        .iter()
        .find_map(|u| t.strip_suffix(u).map(|n| (n, *u)))?;
    let v: f64 = num.trim().parse().ok()?;
    Some(match k {
        "deg" => v.to_radians(),
        "rad" => v,
        "turn" => v * std::f64::consts::TAU,
        _ => v * std::f64::consts::PI / 200.0,
    })
}

/// `transform: f1(...) f2(...)…` → matrice brute (sans
/// origine). Fonctions : `matrix`, `translate[X|Y]`,
/// `scale[X|Y]`, `rotate`, `skew[X|Y]` + projections 2D des
/// variantes 3D (`translate3d`, `scale3d`, `matrix3d`,
/// `rotate3d` axe z — composante z ignorée, `translatez`/
/// `scalez` = identité). `%` des `translate` relatif à la
/// border box. `none`/inconnu → `None`.
#[allow(clippy::cast_possible_truncation)]
fn transform_matrix(raw: &str, rect: &Rect) -> Option<Affine> {
    let mut m = Affine::IDENTITY;
    let mut any = false;
    let mut left = raw.trim();
    if left.eq_ignore_ascii_case("none") || left.is_empty() {
        return None;
    }
    while let Some(open) = left.find('(') {
        let name = left[..open].trim().to_ascii_lowercase();
        let close = left.find(')')?;
        let args: Vec<&str> = left[open + 1..close].split(',').collect();
        let num = |i: usize| -> Option<f64> { args.get(i)?.trim().parse().ok() };
        let f = match name.as_str() {
            "matrix" => Affine {
                a: num(0)? as f32,
                b: num(1)? as f32,
                c: num(2)? as f32,
                d: num(3)? as f32,
                e: num(4)? as f32,
                f: num(5)? as f32,
            },
            "translate" => Affine::translate(
                tf_len_px(args.first()?, rect.width)? as f32,
                args.get(1)
                    .map_or(0.0, |v| tf_len_px(v, rect.height).unwrap_or(0.0) as f32),
            ),
            "translatex" => Affine::translate(tf_len_px(args.first()?, rect.width)? as f32, 0.0),
            "translatey" => Affine::translate(0.0, tf_len_px(args.first()?, rect.height)? as f32),
            // Projection 2D des fonctions 3D (moteur planaire —
            // B-53 : popin TarteAuCitron `translate3d(-50%,-50%,0)`
            // non reconnue → transform ignoré → tiroir expulsé
            // hors viewport à `left:50%`). La composante z est
            // ignorée : `translate3d`/`scale3d`/`matrix3d`/
            // `rotate3d` retombent sur leur projection x/y —
            // le décalage principal reste appliqué.
            "translate3d" => Affine::translate(
                tf_len_px(args.first()?, rect.width)? as f32,
                args.get(1)
                    .map_or(0.0, |v| tf_len_px(v, rect.height).unwrap_or(0.0) as f32),
            ),
            "translatez" | "scalez" => Affine::IDENTITY,
            "matrix3d" => {
                // `matrix3d(m11..m44)` : la sous-matrice 2D est
                // (m11 m12 m14, m21 m22 m24, m41 m42) — indices
                // colonne-major CSS : a=m11, b=m12, c=m21,
                // d=m22, e=m41, f=m42 → args[0,1,4,5,12,13].
                Affine {
                    a: num(0)? as f32,
                    b: num(1)? as f32,
                    c: num(4)? as f32,
                    d: num(5)? as f32,
                    e: num(12)? as f32,
                    f: num(13)? as f32,
                }
            }
            "scale3d" => Affine::scale(num(0)? as f32, num(1)? as f32),
            "rotate3d" => {
                // Rotation d'axe `(x,y,z)` : seule la
                // composante z projette en 2D — axe autre
                // qu'(0,0,±1) → bascule hors-plan, ignorée
                // (mieux vaut aucun transform qu'un faux).
                let (ax, ay, az) = (num(0)?, num(1)?, num(2)?);
                if ax.abs() > f64::EPSILON || ay.abs() > f64::EPSILON {
                    return None;
                }
                let sign = if az < 0.0 { -1.0 } else { 1.0 };
                Affine::rotate((sign * tf_angle(args.get(3).copied()?)?.to_degrees()) as f32)
            }
            "scale" => {
                let sx = num(0)? as f32;
                Affine::scale(
                    sx,
                    args.get(1)
                        .map_or(sx, |v| v.trim().parse::<f32>().unwrap_or(sx)),
                )
            }
            "scalex" => Affine::scale(num(0)? as f32, 1.0),
            "scaley" => Affine::scale(1.0, num(0)? as f32),
            "rotate" => {
                // `tf_angle` rend des radians — `rotate` les
                // reconvertit en degrés comme `from_rotate`.
                Affine::rotate(tf_angle(args.first()?)?.to_degrees() as f32)
            }
            "skewx" => Affine::skew(tf_angle(args.first()?)?.tan() as f32, 0.0),
            "skewy" => Affine::skew(0.0, tf_angle(args.first()?)?.tan() as f32),
            "skew" => Affine::skew(
                tf_angle(args.first()?)?.tan() as f32,
                args.get(1).and_then(|v| tf_angle(v)).unwrap_or(0.0).tan() as f32,
            ),
            _ => return None,
        };
        // Composition gauche→droite : chaque fonction s'applique
        // APRÈS les précédentes (post-concaténation).
        m = m.post_concat(f);
        any = true;
        left = left[close + 1..].trim();
    }
    if !any {
        return None;
    }
    Some(m)
}

/// Résout `transform-origin` en point absolu de `rect` —
/// deux valeurs px/`%` (défaut 50 % 50 %).
#[allow(clippy::cast_possible_truncation)]
fn origin_px(st: &ComputedStyle, rect: &Rect) -> (f32, f32) {
    let raw = st.get_or("transform-origin", "50% 50%");
    let mut it = raw.split_whitespace();
    let x = it
        .next()
        .and_then(|v| tf_len_px(v, rect.width))
        .map_or(rect.width / 2.0, |v| v as f32);
    let y = it
        .next()
        .and_then(|v| tf_len_px(v, rect.height))
        .map_or(rect.height / 2.0, |v| v as f32);
    (rect.x + x, rect.y + y)
}

/// `transform` d'une boîte → matrice complète (origine
/// incluse, exprimée dans le repère du `rect` passé —
/// document ou décalé scroll) — `None` si
/// identité/invalidée. Partagée par la peinture (matrice
/// poussée sur la display list) et le hit-test (matrice
/// inverse appliquée au point sondé).
#[must_use]
pub fn box_transform(bx: &LayoutBox, rect: &Rect) -> Option<Affine> {
    let raw = bx.style.get("transform")?;
    let m = transform_matrix(raw, rect)?;
    let (ox, oy) = origin_px(&bx.style, rect);
    // F(p) = o + M(p − o) : post-concaténation dans l'ordre
    // d'application — T(−o) d'abord, M, puis T(+o).
    Some(
        Affine::translate(-ox, -oy)
            .post_concat(m)
            .post_concat(Affine::translate(ox, oy)),
    )
}
