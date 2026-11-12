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

use crate::boxtree::{BoxKind, LayoutBox, Rect};

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
            "translatez" | "scalez" | "perspective" => Affine::IDENTITY,
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
            // R-188 — 3D : `rotateZ`/`rotate3d` = rotation
            // plan ; `rotateX`/`rotateY` = compression du
            // côté orthogonal (projection affine — le warp
            // perspective non-affine est différé documenté) ;
            // `perspective(d)` = matrice de projection —
            // approximation identité (affine) documentée.
            "rotatex" => Affine {
                a: 1.0,
                b: 0.0,
                c: 0.0,
                d: tf_angle(args.first()?)?.to_degrees().to_radians().cos() as f32,
                e: 0.0,
                f: 0.0,
            },
            "rotatey" => Affine {
                a: tf_angle(args.first()?)?.to_degrees().to_radians().cos() as f32,
                b: 0.0,
                c: 0.0,
                d: 1.0,
                e: 0.0,
                f: 0.0,
            },
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
            "rotate" | "rotatez" => {
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
/// deux valeurs px/`%` (défaut 50 % 50 %) ; une troisième
/// valeur `z` est tolérée au parse et ignorée (aplatissement
/// 2D — R-188, documenté).
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

/// Transform du repère de flux propre → celui du parent
/// (R-183, css-writing-modes) : les enfants d'une boîte
/// dont le `writing-mode` croise le flux contenant sont
/// posés en coordonnées transposées (`x'` = axe inline,
/// `y'` = axe bloc) ; cette matrice les remet dans le
/// repère parent. `content` = content box de la boîte **en
/// coords parent**. `None` = identité (même repère).
///
/// `sideways-*` suit le vertical de base ; `text-orientation`
/// ne change pas la géométrie des boîtes (glyphes seuls —
/// hors MVP).
#[must_use]
#[allow(clippy::many_single_char_names)]
pub fn flow_transform(
    parent: explorer_css::value::WritingMode,
    own: explorer_css::value::WritingMode,
    content: &Rect,
) -> Option<Affine> {
    use explorer_css::value::WritingMode as W;
    let (a, b, c, d, e, f) = match (parent, own) {
        // O=vrl dans flux horizontal : inline→bas, bloc→gauche.
        (W::HorizontalTb, W::VerticalRl | W::SidewaysRl) => {
            (0., -1., 1., 0., content.x + content.width, content.y)
        }
        // O=vlr dans flux horizontal, OU O=horizontal dans
        // flux vlr : même transposée directe.
        (W::HorizontalTb, W::VerticalLr | W::SidewaysLr)
        | (W::VerticalLr | W::SidewaysLr, W::HorizontalTb) => {
            (0., 1., 1., 0., content.x, content.y)
        }
        // O=horizontal dans flux vrl : inverse de la transposée.
        (W::VerticalRl | W::SidewaysRl, W::HorizontalTb) => {
            (0., 1., -1., 0., content.x, content.y + content.height)
        }
        // Flux verticaux de sens opposés : miroir sur l'axe bloc.
        (W::VerticalRl | W::SidewaysRl, W::VerticalLr | W::SidewaysLr)
        | (W::VerticalLr | W::SidewaysLr, W::VerticalRl | W::SidewaysRl) => {
            (1., 0., 0., -1., content.x, content.y + content.height)
        }
        _ => return None,
    };
    Some(Affine { a, b, c, d, e, f })
}

/// `rotate` individuel (CSS Transforms 2, R-188) →
/// matrice. Formes : `<angle>` (axe z implicite),
/// `x|y|z <angle>`, `<x> <y> <z> <angle>`. Projection
/// affine documentée (mêmes conventions que les fonctions
/// `transform`) : axe z → rotation plan, x → `sy`=cosθ,
/// y → `sx`=cosθ, axe arbitraire hors-plan → `None`.
#[allow(clippy::cast_possible_truncation)]
fn rotate_prop(v: &str) -> Option<Affine> {
    let mut letter = "";
    let mut axis = [0.0f64; 3];
    let mut naxis = 0usize;
    let mut angle = None;
    for t in v.split_whitespace() {
        if let Ok(n) = t.parse::<f64>() {
            if naxis < 3 {
                axis[naxis] = n;
                naxis += 1;
            }
        } else if let Some(a) = tf_angle(t) {
            angle = Some(a);
        } else {
            letter = t;
        }
    }
    let ang = angle?;
    let (x, y, z) = match (letter, naxis) {
        ("", 0) | ("z", _) => (0.0, 0.0, 1.0),
        ("x", _) => (1.0, 0.0, 0.0),
        ("y", _) => (0.0, 1.0, 0.0),
        ("", 3) => (axis[0], axis[1], axis[2]),
        _ => return None,
    };
    if x.abs() < f64::EPSILON && y.abs() < f64::EPSILON && z.abs() > f64::EPSILON {
        Some(Affine::rotate((z.signum() * ang.to_degrees()) as f32))
    } else if x.abs() > f64::EPSILON && y.abs() < f64::EPSILON && z.abs() < f64::EPSILON {
        Some(Affine {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: ang.cos() as f32,
            e: 0.0,
            f: 0.0,
        })
    } else if x.abs() < f64::EPSILON && y.abs() > f64::EPSILON && z.abs() < f64::EPSILON {
        Some(Affine {
            a: ang.cos() as f32,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            e: 0.0,
            f: 0.0,
        })
    } else {
        // Axe arbitraire hors-plan — pas de projection
        // affine honnête → propriété ignorée.
        None
    }
}

/// `translate`/`rotate`/`scale` individuelles (CSS
/// Transforms 2, R-188) → matrice partielle, composée dans
/// l'ordre spec `translate → rotate → scale` **avant**
/// `transform`. `none`/invalide → portion ignorée
/// (tolérance, jamais de matrice fausse).
#[allow(clippy::cast_possible_truncation)]
fn individual_transform(st: &ComputedStyle, rect: &Rect) -> Option<Affine> {
    let mut m = Affine::IDENTITY;
    let mut any = false;
    if let Some(v) = st.get("translate") {
        let mut it = v.split_whitespace();
        // Le z d'une 3e valeur est projeté à 0 (moteur
        // planaire) ; `none` ne parse pas → ignoré.
        if let Some(tx) = it.next().and_then(|t| tf_len_px(t, rect.width)) {
            let ty = it
                .next()
                .and_then(|t| tf_len_px(t, rect.height))
                .unwrap_or(0.0);
            m = m.post_concat(Affine::translate(tx as f32, ty as f32));
            any = true;
        }
    }
    if let Some(v) = st.get("rotate") {
        if let Some(r) = rotate_prop(v.trim()) {
            m = m.post_concat(r);
            any = true;
        }
    }
    if let Some(v) = st.get("scale") {
        let mut it = v.split_whitespace();
        if let Some(sx) = it.next().and_then(|t| t.parse::<f64>().ok()) {
            let sy = it.next().and_then(|t| t.parse::<f64>().ok()).unwrap_or(sx);
            m = m.post_concat(Affine::scale(sx as f32, sy as f32));
            any = true;
        }
    }
    any.then_some(m)
}

/// `transform-box` → boîte de référence des `%` et de
/// `transform-origin` (css-transforms §transform-box,
/// R-188 suite) : `border-box` (défaut) ; `padding-box` =
/// moins les bordures ; `content-box` = moins bordures +
/// padding. `fill-box`/`stroke-box`/`view-box` (boîte
/// objet SVG) → repli `border-box` documenté — pas de
/// boîte objet distincte pour nos éléments.
#[allow(clippy::cast_possible_truncation)]
fn transform_box_rect(st: &ComputedStyle, rect: &Rect) -> Rect {
    let len = |p: &str, base: f32| -> f32 {
        st.get(p).and_then(|v| tf_len_px(v, base)).unwrap_or(0.0) as f32
    };
    let bl = len("border-left-width", rect.width);
    let br = len("border-right-width", rect.width);
    let bt = len("border-top-width", rect.height);
    let bb = len("border-bottom-width", rect.height);
    let mut r = *rect;
    let tb = st.get("transform-box").map(str::trim);
    if matches!(tb, Some("padding-box" | "content-box")) {
        r.x += bl;
        r.y += bt;
        r.width = (r.width - bl - br).max(0.0);
        r.height = (r.height - bt - bb).max(0.0);
        if tb == Some("content-box") {
            let pl = len("padding-left", rect.width);
            let pr = len("padding-right", rect.width);
            let pt = len("padding-top", rect.height);
            let pb = len("padding-bottom", rect.height);
            r.x += pl;
            r.y += pt;
            r.width = (r.width - pl - pr).max(0.0);
            r.height = (r.height - pt - pb).max(0.0);
        }
    }
    r
}

/// `transform` d'une boîte → matrice complète (origine
/// incluse, exprimée dans le repère du `rect` passé —
/// document ou décalé scroll) — `None` si
/// identité/invalidée. Partagée par la peinture (matrice
/// poussée sur la display list) et le hit-test (matrice
/// inverse appliquée au point sondé).
#[must_use]
pub fn box_transform(bx: &LayoutBox, rect: &Rect) -> Option<Affine> {
    // Boîtes anonymes / runs texte : jamais « transformable »
    // (CSS Transforms — transformable = boîte générée par un
    // élément ou pseudo-élément). Leur style clone celui du
    // parent (`wrap_anonymous`/feuilles texte) : `transform`
    // s'y rejouerait à chaque niveau d'enveloppe — matrice
    // re-poussée M²/M³, texte peint vrillé/miré (§B-58,
    // letterboxd `.backdrop-metadata` `rotate(-90deg)` lu
    // comme `rotate(-180deg)` = miroir).
    if matches!(bx.kind, BoxKind::AnonymousBlock | BoxKind::Text) {
        return None;
    }
    // Boîte de référence `transform-box` (R-188 suite) —
    // `%` des fonctions et `transform-origin` s'y résolvent.
    let trect = transform_box_rect(&bx.style, rect);
    // CSS Transforms 2 (R-188) : `translate`/`rotate`/
    // `scale` individuelles s'appliquent AVANT `transform`.
    let mut any = false;
    let mut m = Affine::IDENTITY;
    if let Some(t) = individual_transform(&bx.style, &trect) {
        m = t;
        any = true;
    }
    if let Some(t) = bx
        .style
        .get("transform")
        .and_then(|raw| transform_matrix(raw, &trect))
    {
        m = m.post_concat(t);
        any = true;
    }
    if !any {
        return None;
    }
    let (ox, oy) = origin_px(&bx.style, &trect);
    // F(p) = o + M(p − o) : post-concaténation dans l'ordre
    // d'application — T(−o) d'abord, M, puis T(+o).
    Some(
        Affine::translate(-ox, -oy)
            .post_concat(m)
            .post_concat(Affine::translate(ox, oy)),
    )
}

// ---- R-272 — css-motion-1 `offset-*` --------------------

/// Une paire `<position>` (`x y`, mots-clés ou longueurs/`%`)
/// résolue en coords absolues de `r` — `%` sur les deux axes.
#[allow(clippy::cast_possible_truncation)]
fn pos_in_rect(tok1: &str, tok2: &str, r: &Rect) -> (f32, f32) {
    let axis = |t: &str, lo: f32, len: f32, start: &str, end: &str| -> f32 {
        let t = t.trim();
        if t == start {
            return lo;
        }
        if t == end {
            return lo + len;
        }
        if t == "center" {
            return lo + len / 2.0;
        }
        tf_len_px(t, len).map_or(lo, |v| lo + v as f32)
    };
    (
        axis(tok1, r.x, r.width, "left", "right"),
        axis(tok2, r.y, r.height, "top", "bottom"),
    )
}

/// `offset-anchor`/`offset-position` → point : `auto`/`normal`
/// → `fallback` ; sinon 1-2 tokens (x [y], `center` seul
/// → centre). `r` = boîte de référence (`%`/mots-clés).
#[allow(clippy::cast_possible_truncation)]
fn offset_pos(raw: &str, r: &Rect, fallback: (f32, f32)) -> (f32, f32) {
    let raw = raw.trim();
    if raw.is_empty() || matches!(raw, "auto" | "normal" | "none") {
        return fallback;
    }
    let mut it = raw.split_whitespace();
    let Some(x) = it.next() else {
        return fallback;
    };
    let y = it.next().unwrap_or("center");
    if matches!(x, "top" | "bottom") {
        // Ordre inversé (`bottom left`) : le token vertical
        // en tête, l'autre pilote l'axe x (spec `<position>`).
        return pos_in_rect(y, x, r);
    }
    pos_in_rect(x, y, r)
}

/// Angle de trajectoire + distance → point et tangente d'un
/// chemin paramétré. `perim` = longueur totale (0 = rayon
/// infini — `%` non résolvable → traité en px).
enum OffPath {
    /// `ray(θ)` — origine propre, direction θ (0deg = haut).
    Ray { dir: (f32, f32) },
    /// `circle(r)`/`ellipse(rx ry)` — centre + param t.
    Ellipse { c: (f32, f32), rx: f32, ry: f32 },
    /// `inset`/`rect`/`xywh` — rectangle dans le cb.
    Rect { x: f32, y: f32, w: f32, h: f32 },
    /// `polygon(p1, p2, …)` — sommets dans le cb.
    Poly { pts: Vec<(f32, f32)> },
}

/// `point at (x, y, tangent_deg)` à distance `d` le long du
/// chemin — départ : gauche pour ellipse (spec), coin haut-
/// gauche pour rect, premier sommet pour polygon (sens
/// horaire, coords écran y-down = angle croissant).
#[allow(clippy::many_single_char_names, clippy::cast_possible_truncation)]
fn path_point(p: &OffPath, d: f32) -> (f32, f32, f32) {
    use std::f32::consts::{PI, TAU};
    match p {
        OffPath::Ray { dir } => (dir.0 * d, dir.1 * d, dir.1.atan2(dir.0).to_degrees()),
        OffPath::Ellipse { c, rx, ry } => {
            // Périmètre Ramanujan II ; départ au point
            // gauche (t=π), sens horaire (t croissant en
            // coords écran y-down).
            let (a, b) = (rx.max(f32::EPSILON), ry.max(f32::EPSILON));
            let h = ((a - b) / (a + b)).powi(2);
            let perim = PI * (a + b) * (1.0 + 3.0 * h / (10.0 + (4.0 - 3.0 * h).sqrt()));
            let t = PI + TAU * (d / perim.max(f32::EPSILON));
            // Tangente = dérivée paramétrique (x′, y′).
            let tan = (b * t.cos()).atan2(-a * t.sin()).to_degrees();
            (c.0 + a * t.cos(), c.1 + b * t.sin(), tan)
        }
        OffPath::Rect { x, y, w, h } => {
            let segs = [(*w, 0.0), (0.0, *h), (-*w, 0.0), (0.0, -*h)];
            let mut at = (*x, *y);
            let mut d = d.rem_euclid((2.0 * (w + h)).max(f32::EPSILON));
            for (dx, dy) in segs {
                let len = (dx * dx + dy * dy).sqrt();
                if d <= len.max(f32::EPSILON) {
                    let f = if len > 0.0 { d / len } else { 0.0 };
                    return (at.0 + f * dx, at.1 + f * dy, dy.atan2(dx).to_degrees());
                }
                d -= len;
                at = (at.0 + dx, at.1 + dy);
            }
            (*x, *y, 0.0)
        }
        OffPath::Poly { pts } => {
            let perim = path_perim(p);
            if pts.len() < 2 {
                return (pts.first().map_or(0.0, |p| p.0), 0.0, 0.0);
            }
            let mut d = d.rem_euclid(perim.max(f32::EPSILON));
            for i in 0..pts.len() {
                let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
                let (dx, dy) = (b.0 - a.0, b.1 - a.1);
                let len = (dx * dx + dy * dy).sqrt();
                if d <= len.max(f32::EPSILON) {
                    let f = if len > 0.0 { d / len } else { 0.0 };
                    return (a.0 + f * dx, a.1 + f * dy, dy.atan2(dx).to_degrees());
                }
                d -= len;
            }
            (pts[0].0, pts[0].1, 0.0)
        }
    }
}

/// css-motion-1 (R-272) : `offset-path` positionne la boîte
/// le long d'un chemin tracé dans son bloc conteneur (`cb` —
/// coords du même repère que `rect`). Transform résultant :
/// `F(p) = T + R·(p − A)` — l'ancre `A` (offset-anchor,
/// défaut = `transform-origin`/centre) atterrit sur le point
/// `T` du chemin à `offset-distance`, la boîte tournée de
/// `offset-rotate` (`auto` = tangente, `reverse` = +180°).
///
/// Chemins supportés : `ray(θ)`, `circle(r [at p])`,
/// `ellipse(rx ry [at p])`, `inset`/`rect`/`xywh`,
/// `polygon(...)`. Dormants : `path('…')`, `url()`, les
/// `coord-box` nus, `offset-position` sur les formes à `at`.
/// `offset` est expansé en longhands par la cascade.
#[must_use]
#[allow(
    clippy::too_many_lines,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::many_single_char_names
)]
pub fn offset_transform(st: &ComputedStyle, rect: &Rect, cb: &Rect) -> Option<Affine> {
    let raw_path = st.get("offset-path")?;
    let raw_path = raw_path.trim();
    if raw_path.is_empty() || raw_path == "none" {
        return None;
    }
    // Ancre de la boîte : `offset-anchor` (défaut
    // `transform-origin`, sinon centre — spec §5.3).
    let anchor = offset_pos(
        st.get("offset-anchor").unwrap_or(""),
        rect,
        origin_px(st, rect),
    );
    // Position d'ancrage du chemin : `offset-position`,
    // défaut `normal` = position propre de l'ancre.
    let origin = offset_pos(st.get("offset-position").unwrap_or(""), cb, anchor);
    // Corps du chemin.
    let path = parse_off_path(raw_path, cb, origin)?;
    // Distance : `<length>` ou `%` du périmètre (ray → px).
    let raw_d = st.get("offset-distance").unwrap_or("0");
    let d = if raw_d.trim().ends_with('%') {
        let pct = raw_d
            .trim_end_matches('%')
            .trim()
            .parse::<f32>()
            .unwrap_or(0.0);
        match &path {
            OffPath::Ray { .. } => pct,
            _ => pct / 100.0 * path_perim(&path),
        }
    } else {
        tf_len_px(raw_d, cb.width).map_or(0.0, |v| v as f32)
    };
    let (px, py, tangent) = path_point(&path, d);
    // `ray` part de l'origine résolue ; les formes donnent
    // déjà des points absolus du cb (`at`/offsets inclus).
    let target = if matches!(path, OffPath::Ray { .. }) {
        (origin.0 + px, origin.1 + py)
    } else {
        (px, py)
    };
    // `offset-rotate` : `auto` = tangente ; `reverse` =
    // tangente+180° ; `auto <a>` = tangente+a ; `<a>` seul.
    let raw_rot = st.get("offset-rotate").unwrap_or("auto");
    let mut it = raw_rot.split_whitespace();
    let rot = match (it.next(), it.next()) {
        (Some("reverse"), _) => tangent + 180.0,
        (Some("auto"), extra) => {
            tangent
                + extra
                    .and_then(tf_angle)
                    .map_or(0.0, |v| v.to_degrees() as f32)
        }
        (Some(a), _) => tf_angle(a).map_or(0.0, |v| v.to_degrees() as f32),
        _ => 0.0,
    };
    // F(p) = T + R(p − A).
    let (s, c) = rot.to_radians().sin_cos();
    Some(
        Affine::translate(-anchor.0, -anchor.1)
            .post_concat(Affine {
                a: c,
                b: s,
                c: -s,
                d: c,
                e: 0.0,
                f: 0.0,
            })
            .post_concat(Affine::translate(target.0, target.1)),
    )
}

/// Périmètre du chemin — `%` de `offset-distance` s'y
/// résout (ray = 0, non résolvable par construction).
#[allow(clippy::cast_possible_truncation)]
fn path_perim(p: &OffPath) -> f32 {
    use std::f32::consts::PI;
    match p {
        OffPath::Ray { .. } => 0.0,
        OffPath::Ellipse { rx, ry, .. } => {
            let (a, b) = (rx.max(f32::EPSILON), ry.max(f32::EPSILON));
            let h = ((a - b) / (a + b)).powi(2);
            PI * (a + b) * (1.0 + 3.0 * h / (10.0 + (4.0 - 3.0 * h).sqrt()))
        }
        OffPath::Rect { w, h, .. } => 2.0 * (w + h).max(0.0),
        OffPath::Poly { pts } => pts
            .iter()
            .zip(pts.iter().cycle().skip(1))
            .take(pts.len())
            .map(|(a, b)| ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt())
            .sum(),
    }
}

/// Parse le corps de `offset-path` en [`OffPath`] — origine
/// `o` résolue en amont (`offset-position`, défaut = ancre
/// de la boîte). `None` = dormant (`path()`, `url()`,
/// `coord-box`) ou invalide.
#[allow(clippy::cast_possible_truncation, clippy::many_single_char_names)]
fn parse_off_path(raw: &str, cb: &Rect, o: (f32, f32)) -> Option<OffPath> {
    let (name, args) = raw.split_once('(')?;
    let args = args.trim_end_matches(')').trim();
    match name.trim() {
        "ray" => {
            let deg = args
                .split_whitespace()
                .find_map(tf_angle)
                .map_or(0.0, |v| v.to_degrees() as f32);
            // Convention CSS : 0deg = vers le haut.
            let r = deg.to_radians();
            Some(OffPath::Ray {
                dir: (r.sin(), -r.cos()),
            })
        }
        "circle" | "ellipse" => {
            let (dims, at) = args.split_once(" at ").map_or((args, ""), |(d, a)| (d, a));
            let mut it = dims.split_whitespace();
            let (rx, ry) = if name.trim() == "circle" {
                let r = tf_len_px(it.next().unwrap_or("50%"), cb.width)? as f32;
                (r, r)
            } else {
                let rx = tf_len_px(it.next()?, cb.width)? as f32;
                let ry = tf_len_px(it.next()?, cb.height)? as f32;
                (rx, ry)
            };
            let c = if at.is_empty() {
                o
            } else {
                let mut ait = at.split_whitespace();
                pos_in_rect(
                    ait.next().unwrap_or("center"),
                    ait.next().unwrap_or("center"),
                    cb,
                )
            };
            Some(OffPath::Ellipse { c, rx, ry })
        }
        "inset" | "rect" | "xywh" => {
            let v: Vec<f32> = args
                .split_whitespace()
                .filter_map(|t| tf_len_px(t, cb.width).map(|v| v as f32))
                .collect();
            let (x, y, w, h) = if name.trim() == "xywh" {
                (*v.first()?, *v.get(1)?, *v.get(2)?, *v.get(3)?)
            } else {
                // inset/rect : t r b l (clockwise).
                let g = |i: usize| v.get(i).or_else(|| v.last()).copied().unwrap_or(0.0);
                let (t, r, b, l) = (g(0), g(1), g(2), g(3));
                (
                    l,
                    t,
                    (cb.width - l - r).max(0.0),
                    (cb.height - t - b).max(0.0),
                )
            };
            Some(OffPath::Rect {
                x: cb.x + x,
                y: cb.y + y,
                w,
                h,
            })
        }
        "polygon" => {
            let pts: Vec<(f32, f32)> = args
                .split(',')
                .filter_map(|p| {
                    let mut it = p.split_whitespace();
                    let x = tf_len_px(it.next()?, cb.width)? as f32;
                    let y = tf_len_px(it.next()?, cb.height)? as f32;
                    Some((cb.x + x, cb.y + y))
                })
                .collect();
            (pts.len() >= 2).then_some(OffPath::Poly { pts })
        }
        _ => None,
    }
}
