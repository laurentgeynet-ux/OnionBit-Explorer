// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Rasterisation (étape 23) : [`rasterize`] peint une
//! [`DisplayList`] dans un framebuffer RGBA via `tiny-skia`
//! (pur Rust, ADR-0001).
//!
//! - `FillRect`/`Border` : rectangles pleins (`dotted`/
//!   `double` rendus pleins pour l'instant — pattern reporté).
//! - `Text` : octets de fonte résolus via le port `FontSource`
//!   (même famille qu'au layout), contours `ttf-parser`
//!   (`Face::outline_glyph`) → path tiny-skia rempli à la
//!   baseline.
//! - `PushClip`/`PopClip` : `Mask` empilées — le `overflow`
//!   borne réellement le dessin des descendants.
//! - `scale` : facteur DPI HiDPI — toute la géométrie est
//!   multipliée, le framebuffer sort en pixels physiques.

use explorer_css::value::Color;
use explorer_layout::font::{FontRequest, FontSource, Glyph, ShapedRun};
use tiny_skia::{FillRule, Mask, Paint, PathBuilder, Pixmap, Point, Rect, Transform};

use crate::display::{BorderStyle, DisplayItem, DisplayList};

/// Framebuffer RGBA prémultiplié (format natif tiny-skia).
#[derive(Debug, Clone)]
pub struct Frame {
    /// Largeur en pixels physiques.
    pub width: u32,
    /// Hauteur en pixels physiques.
    pub height: u32,
    /// Pixels RGBA prémultipliés, `width*height*4` octets.
    pub pixels: Vec<u8>,
    /// Facteur DPI appliqué (pixels physiques = CSS × scale).
    pub scale: f32,
}

impl Frame {
    /// Encode la frame en PNG (`tiny-skia` fournit l'encodeur).
    #[must_use]
    pub fn encode_png(&self) -> Option<Vec<u8>> {
        let size = tiny_skia::IntSize::from_wh(self.width, self.height)?;
        let pm = Pixmap::from_vec(self.pixels.clone(), size)?;
        pm.encode_png().ok()
    }
}

/// `explorer-css` `Color` → `tiny_skia::Color`.
fn skia_color(c: Color) -> tiny_skia::Color {
    tiny_skia::Color::from_rgba8(c.r, c.g, c.b, c.a)
}

/// Point transformé : `tf` applique et retourne `(x, y)`.
fn map(tf: Transform, x: f32, y: f32) -> (f32, f32) {
    let mut p = Point::from_xy(x, y);
    tf.map_point(&mut p);
    (p.x, p.y)
}

/// Adaptateur `ttf_parser::OutlineBuilder` → `PathBuilder`
/// tiny-skia : les coordonnées arrivent en unités fonte
/// (upem) et sont transformées par `tf` à la volée.
struct GlyphSink<'a> {
    pb: &'a mut PathBuilder,
    tf: Transform,
}

impl ttf_parser::OutlineBuilder for GlyphSink<'_> {
    fn move_to(&mut self, x: f32, y: f32) {
        let (x, y) = map(self.tf, x, y);
        self.pb.move_to(x, y);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        let (x, y) = map(self.tf, x, y);
        self.pb.line_to(x, y);
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let (x1, y1) = map(self.tf, x1, y1);
        let (x, y) = map(self.tf, x, y);
        self.pb.quad_to(x1, y1, x, y);
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let (x1, y1) = map(self.tf, x1, y1);
        let (x2, y2) = map(self.tf, x2, y2);
        let (x, y) = map(self.tf, x, y);
        self.pb.cubic_to(x1, y1, x2, y2, x, y);
    }
    fn close(&mut self) {
        self.pb.close();
    }
}

/// Peint un run texte : résolution de la fonte puis contours
/// de chaque glyphe posés à la plume. `ox`/`oy` et
/// `size_px` sont déjà en pixels physiques (dpi appliqué).
#[allow(clippy::too_many_arguments)]
fn paint_text(
    pixmap: &mut Pixmap,
    ox: f32,
    oy: f32,
    run: &ShapedRun,
    font: &FontRequest,
    color: tiny_skia::Color,
    size_px: f32,
    scale: f32,
    fonts: &dyn FontSource,
    clip: Option<&Mask>,
) {
    let Some(data) = fonts.font_data(font) else {
        return;
    };
    let Ok(face) = ttf_parser::Face::parse(&data, 0) else {
        return;
    };
    let upem = f32::from(face.units_per_em()).max(1.0);
    let gscale = size_px / upem;
    let mut paint = Paint {
        anti_alias: true,
        ..Paint::default()
    };
    paint.set_color(color);
    let mut pen = ox;
    for g in &run.glyphs {
        draw_glyph(pixmap, &face, g, pen, oy, gscale, scale, &paint, clip);
        pen += g.advance * scale;
    }
}

/// Contour d'un glyphe : unités fonte → px (y fonte monte
/// vers le haut → inversion) → translation plume/baseline.
/// Offsets/avances en px CSS → multipliés par `scale`.
#[allow(clippy::too_many_arguments)]
fn draw_glyph(
    pixmap: &mut Pixmap,
    face: &ttf_parser::Face,
    g: &Glyph,
    pen: f32,
    baseline: f32,
    gscale: f32,
    scale: f32,
    paint: &Paint,
    clip: Option<&Mask>,
) {
    let tf = Transform::from_translate(pen + g.offset_x * scale, baseline - g.offset_y * scale)
        .post_concat(Transform::from_scale(gscale, -gscale));
    let mut pb = PathBuilder::new();
    face.outline_glyph(
        ttf_parser::GlyphId(g.id),
        &mut GlyphSink { pb: &mut pb, tf },
    );
    if let Some(path) = pb.finish() {
        pixmap.fill_path(&path, paint, FillRule::Winding, Transform::default(), clip);
    }
}

/// Rasterise `list` à `scale` (1.0 = 96 DPI CSS) dans un
/// framebuffer. `fonts` = port `FontSource` injecté par core
/// (même source que le layout pour des glyphes cohérents).
/// `None` si la frame est hors bornes allouables.
#[must_use]
pub fn rasterize(list: &DisplayList, fonts: &dyn FontSource, scale: f32) -> Option<Frame> {
    let w_px = (list.width * scale).round().max(1.0);
    let h_px = (list.height * scale).round().max(1.0);
    if !(w_px.is_finite() && h_px.is_finite()) || w_px > 16384.0 || h_px > 16384.0 {
        return None; // borne hostile — page géante.
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let (w, h) = (w_px as u32, h_px as u32);
    let mut pixmap = Pixmap::new(w, h)?;
    let dpi = Transform::from_scale(scale, scale);

    let mut clips: Vec<Mask> = Vec::new();
    for item in &list.items {
        match item {
            DisplayItem::PushClip(r) => {
                if let Some(mut mask) = Mask::new(w, h) {
                    if let Some(phys) = Rect::from_xywh(r.x, r.y, r.width, r.height)
                        .and_then(|rr| rr.transform(dpi))
                    {
                        let path = PathBuilder::from_rect(phys);
                        mask.fill_path(&path, FillRule::Winding, true, Transform::default());
                    }
                    clips.push(mask);
                }
            }
            DisplayItem::PopClip => {
                clips.pop();
            }
            _ => paint_item(&mut pixmap, item, dpi, scale, fonts, clips.last()),
        }
    }
    Some(Frame {
        width: w,
        height: h,
        pixels: pixmap.data().to_vec(),
        scale,
    })
}

/// Peint un item de dessin (les clips sont gérés par
/// `rasterize` directement).
fn paint_item(
    pixmap: &mut Pixmap,
    item: &DisplayItem,
    dpi: Transform,
    scale: f32,
    fonts: &dyn FontSource,
    clip: Option<&Mask>,
) {
    match item {
        DisplayItem::FillRect { rect, color } => {
            let mut paint = Paint {
                anti_alias: true,
                ..Paint::default()
            };
            paint.set_color(skia_color(*color));
            if let Some(r) = Rect::from_xywh(rect.x, rect.y, rect.width, rect.height) {
                pixmap.fill_rect(r, &paint, dpi, clip);
            }
        }
        DisplayItem::Border {
            rect,
            widths,
            colors,
            styles,
        } => {
            paint_border(pixmap, rect, widths, colors, *styles, dpi, clip);
        }
        DisplayItem::Text {
            origin,
            run,
            font,
            color,
            size_px,
        } => {
            let (ox, oy) = map(dpi, origin.0, origin.1);
            paint_text(
                pixmap,
                ox,
                oy,
                run,
                font,
                skia_color(*color),
                size_px * scale,
                scale,
                fonts,
                clip,
            );
        }
        DisplayItem::Placeholder { rect, .. } => {
            let mut paint = Paint::default();
            paint.set_color(tiny_skia::Color::from_rgba8(0x99, 0x99, 0x99, 0x60));
            if let Some(r) = Rect::from_xywh(rect.x, rect.y, rect.width, rect.height) {
                pixmap.fill_rect(r, &paint, dpi, clip);
            }
        }
        DisplayItem::PushClip(_) | DisplayItem::PopClip => {}
    }
}

/// Bordure : 4 côtés pleins (pattern dashed/dotted reporté —
/// documenté en `DisplayItem`).
fn paint_border(
    pixmap: &mut Pixmap,
    rect: &explorer_layout::Rect,
    widths: &[f32; 4],
    colors: &[Color; 4],
    styles: [BorderStyle; 4],
    dpi: Transform,
    clip: Option<&Mask>,
) {
    for (side, (bw, (c, s))) in widths
        .iter()
        .zip(colors.iter().zip(styles.iter()))
        .enumerate()
    {
        if *s == BorderStyle::None || *bw <= 0.0 {
            continue;
        }
        let r = match side {
            0 => Rect::from_xywh(rect.x, rect.y, rect.width, *bw),
            1 => Rect::from_xywh(rect.x + rect.width - *bw, rect.y, *bw, rect.height),
            2 => Rect::from_xywh(rect.x, rect.y + rect.height - *bw, rect.width, *bw),
            _ => Rect::from_xywh(rect.x, rect.y, *bw, rect.height),
        };
        if let Some(r) = r {
            let mut paint = Paint::default();
            paint.set_color(skia_color(*c));
            pixmap.fill_rect(r, &paint, dpi, clip);
        }
    }
}
