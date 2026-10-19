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
//! - `scale` : facteur DPI `HiDPI` — toute la géométrie est
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

/// Domaine sûr tiny-skia : le scan convertit en virgule fixe
/// 24.8 — au-delà d'environ ±32 768 px une coordonnée
/// déborde et panique dans `hairline_aa` (site réel :
/// duckduckgo.com, étape 48 — `debug_assert` en build de
/// debug). Une géométrie aussi lointaine est hors écran de
/// toute façon : on la clippe au domaine (les fonds qui
/// couvrent tout restent peints) ou on l'ignore (cas
/// non-axis-aligned).
const SAFE_COORD: f32 = 30_000.0;

/// `tf` est-il axis-aligned (translate/scale/mirror sans
/// rotation ni skew) ?
fn axis_aligned(tf: Transform) -> bool {
    tf.kx == 0.0 && tf.ky == 0.0
}

/// Bbox device des 4 coins de `r` transformés par `tf` —
/// `None` si un coin n'est pas fini.
fn device_bbox(r: &Rect, tf: Transform) -> Option<Rect> {
    let pts = [
        (r.left(), r.top()),
        (r.right(), r.top()),
        (r.left(), r.bottom()),
        (r.right(), r.bottom()),
    ];
    let mut l = f32::MAX;
    let mut t = f32::MAX;
    let mut rr = f32::MIN;
    let mut b = f32::MIN;
    for (x, y) in pts {
        let (mx, my) = map(tf, x, y);
        l = l.min(mx);
        t = t.min(my);
        rr = rr.max(mx);
        b = b.max(my);
    }
    // `from_ltrb` exige finitude + l<r — un coin non fini ou
    // une bbox inversée retourne `None` (hors domaine).
    Rect::from_ltrb(l, t, rr, b)
}

/// `fill_rect` gardé : la géométrie est ramenée dans le
/// domaine fixed-point sûr AVANT d'entrer dans tiny-skia.
fn fill_guarded(pixmap: &mut Pixmap, r: &Rect, paint: &Paint, tf: Transform, clip: Option<&Mask>) {
    let domain = Rect::from_ltrb(-SAFE_COORD, -SAFE_COORD, SAFE_COORD, SAFE_COORD);
    let (Some(domain), Some(b)) = (domain, device_bbox(r, tf)) else {
        return;
    };
    if axis_aligned(tf) {
        // Clippe en espace device : un fond couvrant tout le
        // domaine reste peint, un rect lointain disparaît.
        if let Some(c) = b.intersect(&domain) {
            // Pixel-snap des arêtes sur la grille device, comme
            // les moteurs établis pour les remplissages pleins.
            // Sinon deux rectangles abutés à une coordonnée
            // fractionnaire composent deux arêtes AA au lieu
            // d'une (couture sombre — reftests WPT), et un rect
            // dont les bords sub-pixel couvrent toute la
            // largeur déclenchait le `debug_assert!(false)` de
            // tiny-skia 0.11 (hairline_aa/fill_dot8 — bordure
            // 1 px à position fractionnaire, duckduckgo.com).
            // Des arêtes entières rendent ce bug impossible.
            let snapped = Rect::from_ltrb(
                c.left().round(),
                c.top().round(),
                c.right().round(),
                c.bottom().round(),
            );
            if let Some(s) = snapped {
                pixmap.fill_rect(s, paint, Transform::default(), clip);
            }
        }
    } else if b.left().abs() <= SAFE_COORD
        && b.top().abs() <= SAFE_COORD
        && b.right().abs() <= SAFE_COORD
        && b.bottom().abs() <= SAFE_COORD
    {
        pixmap.fill_rect(*r, paint, tf, clip);
    }
    // sinon : géométrie hors domaine — ignorée, jamais crash.
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

/// Peint un run texte : contours de chaque glyphe posés à la
/// plume. `ox`/`oy`/`size_px` en **px CSS** ; `cur` = dpi ·
/// transforms actifs — il achève la correspondance vers les
/// pixels physiques (rotation/échelle héritées du CSS).
#[allow(clippy::too_many_arguments)]
fn paint_text(
    pixmap: &mut Pixmap,
    ox: f32,
    oy: f32,
    run: &ShapedRun,
    font: &FontRequest,
    color: tiny_skia::Color,
    size_px: f32,
    cur: Transform,
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
        draw_glyph(
            pixmap, &face, g, pen, oy, gscale, size_px, cur, &paint, clip,
        );
        pen += g.advance;
    }
}

/// Contour d'un glyphe : unités fonte → px CSS (y fonte monte
/// vers le haut → inversion) → translation plume/baseline →
/// `cur` vers les pixels physiques. Offsets/avances en px CSS.
#[allow(clippy::too_many_arguments)]
fn draw_glyph(
    pixmap: &mut Pixmap,
    face: &ttf_parser::Face,
    g: &Glyph,
    pen: f32,
    baseline: f32,
    gscale: f32,
    size_px: f32,
    cur: Transform,
    paint: &Paint,
    clip: Option<&Mask>,
) {
    // La position de plume (espace device) doit rester dans le
    // domaine fixed-point — marge de débord des contours :
    // 4× la taille de fonte device couvre tout glyphe plausible.
    let (px, py) = map(cur, pen + g.offset_x, baseline - g.offset_y);
    let s = cur
        .sx
        .abs()
        .max(cur.sy.abs())
        .max(cur.kx.abs())
        .max(cur.ky.abs())
        .max(1.0);
    let extent = 4.0 * size_px * s;
    if !(px.is_finite() && py.is_finite())
        || px.abs() + extent > SAFE_COORD
        || py.abs() + extent > SAFE_COORD
    {
        return;
    }
    let tf = if cur.kx == 0.0 && cur.ky == 0.0 {
        // Pixel snapping : le point de plume est arrêté sur la
        // grille device (comportement des moteurs — lignes à
        // offsets fractionnaires ×,2 sans liseré AA incohérent
        // avec les fill/blit snapés). L'échelle fonte et celle
        // de `cur` (dpi) se composent en une seule.
        Transform::from_scale(gscale * cur.sx, -gscale * cur.sy)
            .post_concat(Transform::from_translate(px.round(), py.round()))
    } else {
        // Ordre de composition : les unités fonte sont mises à
        // l'échelle AVANT la translation plume/baseline —
        // `post_concat` applique `self` puis `other`.
        Transform::from_scale(gscale, -gscale)
            .post_concat(Transform::from_translate(
                pen + g.offset_x,
                baseline - g.offset_y,
            ))
            .post_concat(cur)
    };
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
    // Canvas : opaque blanche — le fond de canvas par défaut
    // de l'UA (CSS2 §14.2 : le document n'y dépose son fond
    // que si html/body en déclarent un). Sans elle, les zones
    // non couvertes resteraient transparentes.
    pixmap.fill(tiny_skia::Color::WHITE);
    let dpi = Transform::from_scale(scale, scale);

    let mut clips: Vec<Mask> = Vec::new();
    // Pile des `transform` CSS : `cur` = dpi · produit des
    // matrices actives — recomposé à chaque push/pop (peu de
    // niveaux en pratique).
    let mut tfs: Vec<Transform> = Vec::new();
    let mut cur = dpi;
    for item in &list.items {
        match item {
            DisplayItem::PushClip(r) => {
                if let Some(mut mask) = Mask::new(w, h) {
                    if let Some(phys) = Rect::from_xywh(r.x, r.y, r.width, r.height)
                        .and_then(|rr| rr.transform(cur))
                    {
                        // Sous transform axis-aligned, le clip
                        // rectangulaire est arrêté sur la grille de
                        // pixels device — cohérent avec les `fill`
                        // et blits qui arrêtent leurs bords : un
                        // `overflow` fractionnaire ne produit pas
                        // de liseré AA que ses voisins n'ont pas.
                        let phys = if cur.kx == 0.0 && cur.ky == 0.0 {
                            Rect::from_ltrb(
                                phys.left().round(),
                                phys.top().round(),
                                phys.right().round(),
                                phys.bottom().round(),
                            )
                            .unwrap_or(phys)
                        } else {
                            phys
                        };
                        // Clippe au domaine fixed-point : un clip
                        // couvrant tout reste plein, un clip
                        // lointain reste vide — même sémantique.
                        let dom = Rect::from_ltrb(-SAFE_COORD, -SAFE_COORD, SAFE_COORD, SAFE_COORD);
                        if let Some(clamped) = dom.and_then(|d| phys.intersect(&d)) {
                            let path = PathBuilder::from_rect(clamped);
                            mask.fill_path(&path, FillRule::Winding, true, Transform::default());
                        }
                    }
                    clips.push(mask);
                }
            }
            DisplayItem::PopClip => {
                clips.pop();
            }
            DisplayItem::PushTransform(m) => {
                tfs.push(*m);
                cur = tfs.iter().fold(dpi, |acc, t| acc.pre_concat(*t));
            }
            DisplayItem::PopTransform => {
                tfs.pop();
                cur = tfs.iter().fold(dpi, |acc, t| acc.pre_concat(*t));
            }
            _ => paint_item(&mut pixmap, item, cur, fonts, clips.last()),
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
                fill_guarded(pixmap, &r, &paint, dpi, clip);
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
            paint_text(
                pixmap,
                origin.0,
                origin.1,
                run,
                font,
                skia_color(*color),
                *size_px,
                dpi,
                fonts,
                clip,
            );
        }
        DisplayItem::Placeholder { rect, .. } => {
            let mut paint = Paint::default();
            paint.set_color(tiny_skia::Color::from_rgba8(0x99, 0x99, 0x99, 0x60));
            if let Some(r) = Rect::from_xywh(rect.x, rect.y, rect.width, rect.height) {
                fill_guarded(pixmap, &r, &paint, dpi, clip);
            }
        }
        DisplayItem::Image { rect, img } => {
            paint_image(pixmap, rect, img, dpi, clip);
        }
        DisplayItem::BackgroundImage {
            rect,
            img,
            repeat,
            origin,
        } => {
            paint_tiled(pixmap, rect, img, *repeat, *origin, dpi, clip);
        }
        DisplayItem::PushClip(_)
        | DisplayItem::PopClip
        | DisplayItem::PushTransform(_)
        | DisplayItem::PopTransform => {}
    }
}

/// Bitmap `img` blittée dans `rect` sous la transform `cur`.
/// `cur` axis-aligned (translate/scale/flip) → boucle
/// nearest-neighbor historique ; rotation/skew →
/// `draw_pixmap` affine (buffer prémultiplié temporaire).
fn paint_image(
    pixmap: &mut Pixmap,
    rect: &explorer_layout::Rect,
    img: &crate::DecodedImage,
    cur: Transform,
    clip: Option<&Mask>,
) {
    if cur.kx == 0.0 && cur.ky == 0.0 && cur.sx != 0.0 && cur.sy != 0.0 {
        paint_image_axis(pixmap, rect, img, cur, clip);
        return;
    }
    // Affine générale : la bbox device doit rester dans le
    // domaine fixed-point sûr (draw_pixmap scanne en 24.8).
    let Some(local) = Rect::from_xywh(rect.x, rect.y, rect.width, rect.height) else {
        return;
    };
    let Some(b) = device_bbox(&local, cur) else {
        return;
    };
    if !(b.left().abs() <= SAFE_COORD
        && b.top().abs() <= SAFE_COORD
        && b.right().abs() <= SAFE_COORD
        && b.bottom().abs() <= SAFE_COORD)
    {
        return;
    }
    // Affine générale : source prémultipliée → draw_pixmap.
    let (iw, ih) = (img.width, img.height);
    let Some(mut src) = Pixmap::new(iw, ih) else {
        return;
    };
    for (i, px) in img.rgba.chunks_exact(4).enumerate() {
        let a = u32::from(px[3]);
        // Prémultiplication bornée [0,255] par construction.
        #[allow(clippy::cast_possible_truncation)]
        let m = |v: u8| ((u32::from(v) * a + 127) / 255) as u8;
        if let Some(c) =
            tiny_skia::PremultipliedColorU8::from_rgba(m(px[0]), m(px[1]), m(px[2]), px[3])
        {
            src.pixels_mut()[i] = c;
        }
    }
    #[allow(clippy::cast_precision_loss)]
    let tf = Transform::from_scale(
        rect.width.max(0.0) / iw as f32,
        rect.height.max(0.0) / ih as f32,
    )
    .post_concat(Transform::from_translate(rect.x, rect.y))
    .post_concat(cur);
    pixmap.draw_pixmap(
        0,
        0,
        src.as_ref(),
        &tiny_skia::PixmapPaint::default(),
        tf,
        clip,
    );
}

/// Blit nearest-neighbor dans la bbox de `rect` transformée
/// par `cur` (scale/translate/flip — le miroir de contenu
/// n'est pas reproduit : approximation documentée).
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn paint_image_axis(
    pixmap: &mut Pixmap,
    rect: &explorer_layout::Rect,
    img: &crate::DecodedImage,
    cur: Transform,
    clip: Option<&Mask>,
) {
    let (pw, ph) = (
        i32::try_from(pixmap.width()).unwrap_or(i32::MAX),
        i32::try_from(pixmap.height()).unwrap_or(i32::MAX),
    );
    let Some(phys) =
        Rect::from_xywh(rect.x, rect.y, rect.width, rect.height).and_then(|r| r.transform(cur))
    else {
        return;
    };
    let (x0, y0) = (phys.x().round() as i32, phys.y().round() as i32);
    let (x1, y1) = (
        (phys.x() + phys.width()).round() as i32,
        (phys.y() + phys.height()).round() as i32,
    );
    let (dx0, dy0) = (x0.max(0), y0.max(0));
    let (dx1, dy1) = (x1.min(pw), y1.min(ph));
    if dx0 >= dx1 || dy0 >= dy1 || rect.width <= 0.0 || rect.height <= 0.0 {
        return;
    }
    let iw = img.width as usize;
    let ih = img.height as usize;
    let clip_data = clip.map(tiny_skia::Mask::data);
    let dst = pixmap.pixels_mut();
    let (pw_u, dw, dh) = (pw as usize, x1 - x0, y1 - y0);
    for dy in dy0..dy1 {
        let sy = ((dy - y0) as usize * ih) / dh.max(1) as usize;
        for dx in dx0..dx1 {
            let i = dy as usize * pw_u + dx as usize;
            if clip_data.is_some_and(|m| m[i] == 0) {
                continue;
            }
            let sx = ((dx - x0) as usize * iw) / dw.max(1) as usize;
            let s = (sy.min(ih - 1) * iw + sx.min(iw - 1)) * 4;
            let (sa, sr, sg, sb) = (
                u32::from(img.rgba[s + 3]),
                u32::from(img.rgba[s]),
                u32::from(img.rgba[s + 1]),
                u32::from(img.rgba[s + 2]),
            );
            // Source-over prémultiplié : src + dst·(1−α).
            let inv = 255 - sa;
            let d = dst[i];
            let blend = |sv: u32, dv: u8| ((sv * sa + u32::from(dv) * inv + 127) / 255) as u8;
            dst[i] = tiny_skia::PremultipliedColorU8::from_rgba(
                blend(sr, d.red()),
                blend(sg, d.green()),
                blend(sb, d.blue()),
                (sa + u32::from(d.alpha()) * inv / 255) as u8,
            )
            .unwrap_or(tiny_skia::PremultipliedColorU8::TRANSPARENT);
        }
    }
}

/// `background-image` : tuilée à taille naturelle depuis le
/// coin `origin` (coords document, inclut `background-position`
/// et l'ancrage `fixed` = viewport), borné à l'aire `rect`,
/// selon `repeat` `(x, y)` — les tuiles couvrent l'aire y
/// compris **avant** l'origine (repeat ±∞). Axis-aligned →
/// boucle directe ; le cas général tombe sur la boucle avec
/// `cur` appliqué par point — approximation bornée documentée.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap
)]
fn paint_tiled(
    pixmap: &mut Pixmap,
    rect: &explorer_layout::Rect,
    img: &crate::DecodedImage,
    repeat: (bool, bool),
    origin: (f32, f32),
    cur: Transform,
    clip: Option<&Mask>,
) {
    let (pw, ph) = (
        i32::try_from(pixmap.width()).unwrap_or(i32::MAX),
        i32::try_from(pixmap.height()).unwrap_or(i32::MAX),
    );
    let Some(phys) =
        Rect::from_xywh(rect.x, rect.y, rect.width, rect.height).and_then(|r| r.transform(cur))
    else {
        return;
    };
    // Origine transformée par point (axis-aligned OK —
    // translate/scale/flip pures).
    let ptx = origin.0 * cur.sx + origin.1 * cur.kx + cur.tx;
    let pty = origin.0 * cur.ky + origin.1 * cur.sy + cur.ty;
    let (ox, oy) = (ptx.round() as i32, pty.round() as i32);
    let (dx0, dy0) = (
        (phys.x().round() as i32).max(0),
        (phys.y().round() as i32).max(0),
    );
    let (dx1, dy1) = (
        ((phys.x() + phys.width()).round() as i32).min(pw),
        ((phys.y() + phys.height()).round() as i32).min(ph),
    );
    if dx0 >= dx1 || dy0 >= dy1 {
        return;
    }
    let iw = img.width as i32;
    let ih = img.height as i32;
    let clip_data = clip.map(tiny_skia::Mask::data);
    let dst = pixmap.pixels_mut();
    for dy in dy0..dy1 {
        let rel_y = dy - oy;
        let sy = if repeat.1 {
            rel_y.rem_euclid(ih)
        } else {
            if rel_y < 0 || rel_y >= ih {
                continue;
            }
            rel_y
        };
        for dx in dx0..dx1 {
            let rel_x = dx - ox;
            let sx = if repeat.0 {
                rel_x.rem_euclid(iw)
            } else {
                if rel_x < 0 || rel_x >= iw {
                    continue;
                }
                rel_x
            };
            let i = dy as usize * pw as usize + dx as usize;
            if clip_data.is_some_and(|m| m[i] == 0) {
                continue;
            }
            let s = (sy as usize * iw as usize + sx as usize) * 4;
            let (sa, sr, sg, sb) = (
                u32::from(img.rgba[s + 3]),
                u32::from(img.rgba[s]),
                u32::from(img.rgba[s + 1]),
                u32::from(img.rgba[s + 2]),
            );
            let inv = 255 - sa;
            let d = dst[i];
            let blend = |sv: u32, dv: u8| ((sv * sa + u32::from(dv) * inv + 127) / 255) as u8;
            dst[i] = tiny_skia::PremultipliedColorU8::from_rgba(
                blend(sr, d.red()),
                blend(sg, d.green()),
                blend(sb, d.blue()),
                (sa + u32::from(d.alpha()) * inv / 255) as u8,
            )
            .unwrap_or(tiny_skia::PremultipliedColorU8::TRANSPARENT);
        }
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
            fill_guarded(pixmap, &r, &paint, dpi, clip);
        }
    }
}
