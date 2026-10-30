// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Playback d'op-list `<canvas>` 2D (R-11, ex-62 moitié rendu).
//!
//! Couture NAV↔PIPELINE figée dans [`CanvasOp`] : `explorer-jscore`
//! (JS-C28) enregistre les appels `CanvasRenderingContext2D`
//! en une `Vec<CanvasOp>` par nœud `<canvas>` ; ce module la
//! rejoue vers `tiny-skia` → bitmap de l'élément, consommé
//! comme un remplacé (`DisplayItem::Image`).
//!
//! MVP couvert : rectangles rapides, chemins (move/line/quad/
//! cubic/arc/rect/close), `fill`/`stroke`/`clip`, transforms,
//! `globalAlpha`, `lineWidth`, save/restore. Volontairement
//! hors MVP (variantes additives ultérieures) : texte,
//! `drawImage`, dégradés/motifs, composite ops, dash, ellipse.

use tiny_skia::{FillRule, Mask, Paint, Path, PathBuilder, Pixmap, Rect, Stroke, Transform};

use crate::DecodedImage;
use explorer_css::value::Color;

/// Une opération `CanvasRenderingContext2D` enregistrée,
/// rejouée dans l'ordre d'appel JS.
#[derive(Debug, Clone, PartialEq)]
pub enum CanvasOp {
    /// `save()` — empile l'état (transform, styles, clip).
    Save,
    /// `restore()` — dépile l'état.
    Restore,
    /// `translate(x, y)`.
    Translate(f32, f32),
    /// `scale(x, y)`.
    Scale(f32, f32),
    /// `rotate(rad)` — radians, sens horaire.
    Rotate(f32),
    /// `transform(a, b, c, d, e, f)` — post-multiplie la CTM.
    Transform(f32, f32, f32, f32, f32, f32),
    /// `setTransform(a, b, c, d, e, f)` — remplace la CTM.
    SetTransform(f32, f32, f32, f32, f32, f32),
    /// `fillStyle = color` (couleur pleine — dégradés hors MVP).
    FillColor(Color),
    /// `strokeStyle = color`.
    StrokeColor(Color),
    /// `globalAlpha = v` (multiplicateur d'alpha, borné 0..=1).
    GlobalAlpha(f32),
    /// `lineWidth = px`.
    LineWidth(f32),
    /// `beginPath()`.
    BeginPath,
    /// `moveTo(x, y)`.
    MoveTo(f32, f32),
    /// `lineTo(x, y)`.
    LineTo(f32, f32),
    /// `quadraticCurveTo(cx, cy, x, y)`.
    QuadraticTo(f32, f32, f32, f32),
    /// `bezierCurveTo(c1x, c1y, c2x, c2y, x, y)`.
    BezierTo(f32, f32, f32, f32, f32, f32),
    /// `arc(cx, cy, r, start_rad, end_rad, anticlockwise)`.
    Arc(f32, f32, f32, f32, f32, bool),
    /// `rect(x, y, w, h)` — ajoute au chemin courant.
    Rect(f32, f32, f32, f32),
    /// `closePath()`.
    ClosePath,
    /// `fill()` — remplit le chemin courant (sans l'effacer).
    Fill,
    /// `stroke()` — trace le chemin courant (sans l'effacer).
    Stroke,
    /// `clip()` — intersecte le clip courant avec le chemin.
    Clip,
    /// `fillRect(x, y, w, h)` — remplissage direct.
    FillRect(f32, f32, f32, f32),
    /// `strokeRect(x, y, w, h)` — contour direct.
    StrokeRect(f32, f32, f32, f32),
    /// `clearRect(x, y, w, h)` — efface vers transparent.
    ClearRect(f32, f32, f32, f32),
}

/// Table « nœud `<canvas>` → op-list enregistrée » — produite
/// par NAV (JS-C28), consommée à la génération de display list.
pub type CanvasTable = std::collections::HashMap<explorer_dom::NodeId, Vec<CanvasOp>>;

/// État graphique canvas — sous-ensemble sauvegardé par `save`.
#[derive(Debug, Clone)]
struct Ctx {
    tf: Transform,
    fill: Color,
    stroke: Color,
    alpha: f32,
    line_w: f32,
    /// Profondeur de la pile de clips à ce point.
    clip_depth: usize,
}

impl Default for Ctx {
    fn default() -> Self {
        Self {
            tf: Transform::default(),
            fill: Color::BLACK,
            stroke: Color::BLACK,
            alpha: 1.0,
            line_w: 1.0,
            clip_depth: 0,
        }
    }
}

/// Budget pixels d'une surface canvas — même ordre que
/// `image_max_px` (16 Mpx ≈ 64 Mio RGBA). §B-59 : `w`/`h`
/// viennent des attributs `width`/`height` du `<canvas>`
/// (contenu non fiable) — un `width="131072" height="65536"`
/// demandait `Pixmap::new` = alloc unique 2³⁵ octets → abort
/// (`allocation of 34359738368 bytes failed` mesuré terrain).
const MAX_CANVAS_PX: u64 = 16_000_000;

/// Rejoue `ops` dans un pixmap `w`×`h` (dimensions intrinsèques
/// de l'élément — attributs `width`/`height`). Retourne le
/// bitmap RGBA (alpha non prémultiplié, comme `decode_image`)
/// — `None` si la surface est invalide/vide ou hors budget
/// pixels (dégradé, jamais crash).
#[must_use]
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub fn raster_ops(ops: &[CanvasOp], w: u32, h: u32) -> Option<DecodedImage> {
    if ops.is_empty() || w == 0 || h == 0 || u64::from(w) * u64::from(h) > MAX_CANVAS_PX {
        return None;
    }
    let mut pm = Pixmap::new(w, h)?;
    let mut cx = Ctx::default();
    let mut states: Vec<Ctx> = Vec::new();
    // Pile de clips : `clip()` empile le chemin intersecté ;
    // `restore()` revient à la profondeur sauvegardée (Mask non
    // clonable — profondeur + empilement, jamais de pop direct).
    let mut clips: Vec<Mask> = Vec::new();
    let mut path = PathBuilder::new();
    for op in ops {
        if ctx_op(&mut cx, &mut states, &mut clips, op) {
            continue;
        }
        if path_op(&mut path, op) {
            continue;
        }
        match *op {
            CanvasOp::Fill => fill(&mut pm, &path, &cx, &clips),
            CanvasOp::Stroke => stroke(&mut pm, &path, &cx, &clips),
            CanvasOp::FillRect(fx, fy, fw, fh) => {
                if let Some(r) = Rect::from_xywh(fx, fy, fw, fh) {
                    let p = PathBuilder::from_rect(r);
                    fill_path(&mut pm, &p, &paint(cx.fill, cx.alpha), cx.tf, clips.last());
                }
            }
            CanvasOp::StrokeRect(fx, fy, fw, fh) => {
                if let Some(r) = Rect::from_xywh(fx, fy, fw, fh) {
                    let p = PathBuilder::from_rect(r);
                    stroke_path(&mut pm, &p, &cx, clips.last());
                }
            }
            CanvasOp::ClearRect(fx, fy, fw, fh) => {
                if let Some(r) = Rect::from_xywh(fx, fy, fw, fh) {
                    let p = PathBuilder::from_rect(r);
                    let mut pt = paint(Color::BLACK, cx.alpha);
                    pt.blend_mode = tiny_skia::BlendMode::Clear;
                    fill_path(&mut pm, &p, &pt, cx.tf, clips.last());
                }
            }
            CanvasOp::Clip => {
                if let Some(p) = path.clone().finish() {
                    if let Some(mut m) = Mask::new(w, h) {
                        m.fill_path(&p, FillRule::Winding, true, cx.tf);
                        if let Some(top) = clips.last() {
                            // Intersection clip-courant × chemin :
                            // min des alphas pixel à pixel.
                            let (dst, src) = (m.data_mut(), top.data());
                            for (da, sa) in dst.iter_mut().zip(src.iter()) {
                                *da = (*da).min(*sa);
                            }
                        }
                        clips.push(m);
                    }
                }
            }
            _ => {}
        }
    }
    let rgba = unpremultiply(pm.take());
    Some(DecodedImage {
        width: w,
        height: h,
        rgba,
    })
}

/// Bras « état du contexte » (save/restore, transforms, styles,
/// clip) — `true` si `op` a été consommée.
fn ctx_op(cx: &mut Ctx, states: &mut Vec<Ctx>, clips: &mut Vec<Mask>, op: &CanvasOp) -> bool {
    match *op {
        CanvasOp::Save => {
            states.push(Ctx {
                clip_depth: clips.len(),
                ..cx.clone()
            });
        }
        CanvasOp::Restore => {
            if let Some(prev) = states.pop() {
                clips.truncate(prev.clip_depth);
                *cx = prev;
            }
        }
        CanvasOp::Translate(tx, ty) => {
            cx.tf = cx.tf.post_concat(Transform::from_translate(tx, ty));
        }
        CanvasOp::Scale(sx, sy) => {
            cx.tf = cx.tf.post_concat(Transform::from_scale(sx, sy));
        }
        CanvasOp::Rotate(rad) => {
            cx.tf = cx.tf.post_concat(Transform::from_rotate(rad.to_degrees()));
        }
        CanvasOp::Transform(ma, mb, mc, md, me, mf) => {
            cx.tf = cx
                .tf
                .post_concat(Transform::from_row(ma, mb, mc, md, me, mf));
        }
        CanvasOp::SetTransform(ma, mb, mc, md, me, mf) => {
            cx.tf = Transform::from_row(ma, mb, mc, md, me, mf);
        }
        CanvasOp::FillColor(c) => cx.fill = c,
        CanvasOp::StrokeColor(c) => cx.stroke = c,
        CanvasOp::GlobalAlpha(v) => cx.alpha = v.clamp(0.0, 1.0),
        CanvasOp::LineWidth(v) => cx.line_w = v.max(0.0),
        _ => return false,
    }
    true
}

/// Bras « construction de chemin » — `true` si `op` a été
/// consommée.
fn path_op(path: &mut PathBuilder, op: &CanvasOp) -> bool {
    match *op {
        CanvasOp::BeginPath => *path = PathBuilder::new(),
        CanvasOp::MoveTo(mx, my) => {
            path.move_to(mx, my);
        }
        CanvasOp::LineTo(lx, ly) => {
            path.line_to(lx, ly);
        }
        CanvasOp::QuadraticTo(qx, qy, ex, ey) => {
            path.quad_to(qx, qy, ex, ey);
        }
        CanvasOp::BezierTo(c1x, c1y, c2x, c2y, ex, ey) => {
            path.cubic_to(c1x, c1y, c2x, c2y, ex, ey);
        }
        CanvasOp::Arc(ax, ay, rad, sa, ea, ccw) => arc(path, ax, ay, rad, sa, ea, ccw),
        CanvasOp::Rect(rx, ry, rw, rh) => {
            if let Some(r) = Rect::from_xywh(rx, ry, rw, rh) {
                path.push_rect(r);
            }
        }
        CanvasOp::ClosePath => {
            path.close();
        }
        _ => return false,
    }
    true
}

/// `paint` `color` × `alpha` — tiny-skia travaille en alpha
/// prémultiplié, le canal alpha du `Paint` porte le produit.
fn paint(c: Color, alpha: f32) -> Paint<'static> {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let a = (f32::from(c.a) * alpha).clamp(0.0, 255.0) as u8;
    Paint {
        shader: tiny_skia::Shader::SolidColor(tiny_skia::Color::from_rgba8(c.r, c.g, c.b, a)),
        anti_alias: true,
        ..Paint::default()
    }
}

/// `fill()` : remplit le chemin courant sous la CTM.
fn fill(pm: &mut Pixmap, path: &PathBuilder, cx: &Ctx, clips: &[Mask]) {
    if let Some(p) = path.clone().finish() {
        fill_path(pm, &p, &paint(cx.fill, cx.alpha), cx.tf, clips.last());
    }
}

/// `stroke()` : trace le chemin courant, épaisseur `lineWidth`.
fn stroke(pm: &mut Pixmap, path: &PathBuilder, cx: &Ctx, clips: &[Mask]) {
    if let Some(p) = path.clone().finish() {
        stroke_path(pm, &p, cx, clips.last());
    }
}

fn stroke_path(pm: &mut Pixmap, p: &Path, cx: &Ctx, clip: Option<&Mask>) {
    let s = Stroke {
        width: cx.line_w.max(f32::EPSILON),
        ..Stroke::default()
    };
    pm.stroke_path(p, &paint(cx.stroke, cx.alpha), &s, cx.tf, clip);
}

fn fill_path(pm: &mut Pixmap, p: &Path, pt: &Paint, tf: Transform, clip: Option<&Mask>) {
    pm.fill_path(p, pt, FillRule::Winding, tf, clip);
}

/// `arc()` — approximation en segments cubiques (≤ 90° par
/// segment, kappa = 4/3·tan(Δ/4)). `sa`/`ea` = radians ;
/// `ccw` inverse le sens de balayage.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn arc(path: &mut PathBuilder, ax: f32, ay: f32, rad: f32, sa: f32, ea: f32, ccw: bool) {
    if rad <= 0.0 || !rad.is_finite() {
        return;
    }
    let sweep = if ccw {
        let mut d = sa - ea;
        while d <= 0.0 {
            d += std::f32::consts::TAU;
        }
        -d
    } else {
        let mut d = ea - sa;
        while d < 0.0 {
            d += std::f32::consts::TAU;
        }
        d
    };
    if sweep.abs() < f32::EPSILON {
        return;
    }
    let steps = ((sweep.abs() / std::f32::consts::FRAC_PI_2).ceil() as usize).max(1);
    #[allow(clippy::cast_precision_loss)]
    let step = sweep / steps as f32;
    let mut a = sa;
    let mut cur = (ax + rad * a.cos(), ay + rad * a.sin());
    // Spec : arc sur chemin non vide joint par une ligne.
    if path.is_empty() {
        path.move_to(cur.0, cur.1);
    } else {
        path.line_to(cur.0, cur.1);
    }
    for _ in 0..steps {
        let na = a + step;
        let next = (ax + rad * na.cos(), ay + rad * na.sin());
        let k = 4.0 / 3.0 * (step / 4.0).tan();
        path.cubic_to(
            cur.0 + k * rad * (-a.sin()),
            cur.1 + k * rad * a.cos(),
            next.0 - k * rad * (-na.sin()),
            next.1 - k * rad * na.cos(),
            next.0,
            next.1,
        );
        a = na;
        cur = next;
    }
}

/// tiny-skia stocke l'alpha prémultiplié ; `DecodedImage` suit
/// la convention `image` (RGBA droit) — conversion exacte.
fn unpremultiply(data: Vec<u8>) -> Vec<u8> {
    let mut out = data;
    for px in out.chunks_exact_mut(4) {
        let a = px[3];
        if a != 0 && a != 255 {
            let f = 255.0 / f32::from(a);
            for c in &mut px[..3] {
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                {
                    *c = (f32::from(*c) * f).min(255.0) as u8;
                }
            }
        }
    }
    out
}
