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
use tiny_skia::{FillRule, Mask, Paint, PathBuilder, Pixmap, Point, Rect, Stroke, Transform};

use crate::display::{BorderStyle, DisplayItem, DisplayList, FilterOp, MarkerKind};
use crate::glyph_cache::GlyphCache;
use owned_ttf_parser::{AsFaceRef, OwnedFace};

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

/// Hauteur d'une bande d'encodage PNG — borne la copie
/// démultipliée (~`width*4*PNG_BAND_ROWS` octets au lieu de
/// toute la frame, R-37).
const PNG_BAND_ROWS: usize = 256;
/// Taille cible des chunks `IDAT` émis par le flux PNG.
const PNG_IDAT_TARGET: usize = 256 * 1024;

impl Frame {
    /// Encode la frame en PNG (buffer complet).
    ///
    /// Même encodage streamé que [`Frame::encode_png_to`],
    /// ici collecté dans un `Vec`.
    #[must_use]
    pub fn encode_png(&self) -> Option<Vec<u8>> {
        let mut out = Vec::new();
        self.encode_png_to(&mut out).then_some(out)
    }

    /// Encode la frame en PNG dans `w`, streamé par bandes.
    ///
    /// Les pixels sont **prémultipliés** (convention
    /// tiny-skia) : démultiplication par bande de
    /// `PNG_BAND_ROWS` lignes avant écriture, comme
    /// `Pixmap::encode_png`. Compression `Fast` (fdeflate —
    /// R-33b : l'encodeur par défaut coûtait ~4 s sur une
    /// frame de page lourde, poste dominant du repaint
    /// progressif). R-37 : fini le clone RGBA monolithique
    /// (~50 Mio sur une page haute) — le scratch démultiplié
    /// reste borné à une bande et les `IDAT` partent par
    /// chunks de ~`PNG_IDAT_TARGET` octets.
    pub fn encode_png_to<W: std::io::Write>(&self, w: W) -> bool {
        use std::io::Write as _;
        let mut enc = png::Encoder::new(w, self.width, self.height);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_compression(png::Compression::Fast);
        enc.set_filter(png::Filter::NoFilter);
        let Ok(mut writer) = enc.write_header() else {
            tracing::warn!(
                w = self.width,
                h = self.height,
                "encode_png_to : write_header échoué"
            );
            return false;
        };
        // Chaque étape est nommée — un `false` muet ne
        // diagnostique rien (`png_b=0` >55 Mo, vols 592/598,
        // Étape 180).
        let ok: Result<(), &'static str> = (|| {
            let mut stream = writer
                .stream_writer_with_size(PNG_IDAT_TARGET)
                .map_err(|_| "stream_writer")?;
            let row_len = self.width as usize * 4;
            let h = self.height as usize;
            let mut band = Vec::with_capacity(row_len * PNG_BAND_ROWS);
            for y0 in (0..h).step_by(PNG_BAND_ROWS) {
                let rows = PNG_BAND_ROWS.min(h - y0);
                band.clear();
                band.extend_from_slice(&self.pixels[y0 * row_len..(y0 + rows) * row_len]);
                for px in band.chunks_exact_mut(4) {
                    let a = u32::from(px[3]);
                    if a == 0 {
                        px[..3].fill(0);
                    } else if a < 255 {
                        for c in &mut px[..3] {
                            *c = ((u32::from(*c) * 255 + a / 2) / a).min(255) as u8;
                        }
                    }
                }
                stream.write_all(&band).map_err(|_| "write_all")?;
            }
            stream.finish().map_err(|_| "stream_finish")?;
            Ok(())
        })();
        if let Err(step) = ok {
            tracing::warn!(
                w = self.width,
                h = self.height,
                step,
                "encode_png_to : échec IDAT"
            );
            return false;
        }
        let done = writer.finish().is_ok();
        if !done {
            tracing::warn!(
                w = self.width,
                h = self.height,
                "encode_png_to : writer.finish échoué"
            );
        }
        done
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

/// Cap du pool de `Mask` de [`replay_items`] — les clips
/// `Real` s'empilent/dépilent en LIFO strict ; au-delà de
/// cette profondeur les buffers sont libérés plutôt que
/// conservés (borné : chaque entrée ≈ w·h octets).
const MASK_POOL_CAP: usize = 8;

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

/// Peint un run texte : contours de chaque glyphe posés à la
/// plume. `ox`/`oy`/`size_px` en **px CSS** ; `cur` = dpi ·
/// transforms actifs — il achève la correspondance vers les
/// pixels physiques (rotation/échelle héritées du CSS).
///
/// R-36 : la face est `gc.face()` (parsée une fois, possédée,
/// partagée entre repaints) et chaque contour vient de
/// `gc.glyph_path()` en unités fonte — plus de `Face::parse`
/// ni de `outline_glyph` par glyphe à chaque frame.
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
    gc: &mut GlyphCache,
    clip: Option<&Mask>,
) {
    let Some(oface) = gc.face(fonts, font) else {
        return;
    };
    let mut paint = Paint {
        anti_alias: true,
        ..Paint::default()
    };
    paint.set_color(color);
    // Faces de repli par span (R-69) — résolues une fois par
    // run ; `span_i` avance à mesure que l'index glyphe entre
    // dans [start, end). `None` = octets illisibles → glyphes
    // sautés (dégradation, le `.notdef` principal ferait
    // double emploi).
    let spans = &run.fallback_spans;
    let mut fb_faces: Vec<Option<std::sync::Arc<OwnedFace>>> = Vec::new();
    if !spans.is_empty() {
        fb_faces = spans.iter().map(|s| gc.face_at(&s.data, s.face)).collect();
    }
    let mut pen = ox;
    let mut span_i = 0usize;
    for (gi, g) in run.glyphs.iter().enumerate() {
        // PUA sans couverture → slot muet (vol 26) : l'avance
        // reste comptée, rien n'est peint.
        if g.invisible {
            pen += g.advance;
            continue;
        }
        let gi32 = u32::try_from(gi).unwrap_or(u32::MAX);
        while span_i < spans.len() && gi32 >= spans[span_i].end {
            span_i += 1;
        }
        let fb = spans
            .get(span_i)
            .filter(|s| gi32 >= s.start)
            .and_then(|_| fb_faces.get(span_i).and_then(Clone::clone));
        let (face, face_key) = match &fb {
            Some(f) => (f.as_face_ref(), std::sync::Arc::as_ptr(f) as usize),
            None => (oface.as_face_ref(), std::sync::Arc::as_ptr(&oface) as usize),
        };
        let gscale = size_px / f32::from(face.units_per_em()).max(1.0);
        draw_glyph(
            pixmap, gc, face, face_key, g, pen, oy, gscale, size_px, cur, &paint, color, clip,
        );
        pen += g.advance;
    }
}

/// Contour d'un glyphe : unités fonte → px CSS (y fonte monte
/// vers le haut → inversion) → translation plume/baseline →
/// `cur` vers les pixels physiques. Offsets/avances en px CSS.
/// Le contour brut sort du cache cross-passe (unités fonte) ;
/// `Path::transform` applique `tf` — même `map_point` par
/// point que le `GlyphSink` historique, résultat identique.
#[allow(clippy::too_many_arguments)]
fn draw_glyph(
    pixmap: &mut Pixmap,
    gc: &mut GlyphCache,
    face: &ttf_parser::Face<'_>,
    face_key: usize,
    g: &Glyph,
    pen: f32,
    baseline: f32,
    gscale: f32,
    size_px: f32,
    cur: Transform,
    paint: &Paint,
    color: tiny_skia::Color,
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
    // R-85 — atlas de glyphes rendus : sous transform
    // axis-aligned (le cas quasi universel), la couverture
    // d'un glyphe snapé est position-indépendante → blit
    // d'un bitmap rendu une fois. Évite le clone+transform
    // du contour par glyphe ET le `DrawTiler` de tiny-skia
    // (clone+transform par tuile ≥ 8192 px, y compris les
    // tuiles que le glyphe ne touche pas). `None` = pas
    // d'encre ou hors gabarit → repli `fill_path`.
    if cur.kx == 0.0 && cur.ky == 0.0 {
        if let Some(rg) = gc.rendered_glyph(
            face,
            face_key,
            g.id,
            gscale * cur.sx,
            gscale * cur.sy,
            color,
        ) {
            #[allow(clippy::cast_possible_truncation)]
            blit_glyph(
                pixmap,
                rg,
                px.round() as i32 + rg.dx,
                py.round() as i32 + rg.dy,
                clip,
            );
            return;
        }
    }
    if let Some(path) = gc
        .glyph_path(face, face_key, g.id)
        .and_then(|p| p.transform(tf))
    {
        pixmap.fill_path(&path, paint, FillRule::Winding, Transform::default(), clip);
    }
}

/// Blit d'un glyphe rendu (R-85) — source-over prémultiplié
/// par pixel, mêmes formules que `paint_image_axis` : la
/// couverture AA est déjà cuite dans `rg.pm` (colorisée au
/// rendu), le `clip` module l'alpha. Équivalence de blend
/// avec `fill_path` : `dst = (couleur·cov·clip) +
/// dst·(1 − alpha·cov·clip)` des deux côtés (±1/255
/// d'arrondi — largement sous la tolérance reftest).
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn blit_glyph(
    pixmap: &mut Pixmap,
    rg: &crate::glyph_cache::RenderedGlyph,
    ox: i32,
    oy: i32,
    clip: Option<&Mask>,
) {
    let (pw, ph) = (
        i32::try_from(pixmap.width()).unwrap_or(i32::MAX),
        i32::try_from(pixmap.height()).unwrap_or(i32::MAX),
    );
    let (sw, sh) = (
        i32::try_from(rg.pm.width()).unwrap_or(i32::MAX),
        i32::try_from(rg.pm.height()).unwrap_or(i32::MAX),
    );
    let (x0, y0) = (ox.max(0), oy.max(0));
    let (x1, y1) = ((ox + sw).min(pw), (oy + sh).min(ph));
    if x0 >= x1 || y0 >= y1 {
        return;
    }
    let clip_data = clip.map(tiny_skia::Mask::data);
    let dst = pixmap.pixels_mut();
    let src = rg.pm.pixels();
    let (pwu, swu) = (pw as usize, sw as usize);
    for yy in y0..y1 {
        let srow = (yy - oy) as usize * swu;
        for xx in x0..x1 {
            let sp = src[srow + (xx - ox) as usize];
            let cov = clip_data.map_or(255, |m| u32::from(m[yy as usize * pwu + xx as usize]));
            let sa = (u32::from(sp.alpha()) * cov + 127) / 255;
            if sa == 0 {
                continue;
            }
            let di = yy as usize * pwu + xx as usize;
            let dp = dst[di];
            let inv = 255 - sa;
            // Premult src-over : dst = src·clip + dst·(1−a·clip)
            let blend =
                |sv: u8, dv: u8| ((u32::from(sv) * cov + u32::from(dv) * inv + 127) / 255) as u8;
            dst[di] = tiny_skia::PremultipliedColorU8::from_rgba(
                blend(sp.red(), dp.red()),
                blend(sp.green(), dp.green()),
                blend(sp.blue(), dp.blue()),
                ((sa * 255 + u32::from(dp.alpha()) * inv + 127) / 255) as u8,
            )
            .unwrap_or(tiny_skia::PremultipliedColorU8::TRANSPARENT);
        }
    }
}

/// Rasterise `list` à `scale` (1.0 = 96 DPI CSS) dans un
/// framebuffer. `fonts` = port `FontSource` injecté par core
/// (même source que le layout pour des glyphes cohérents).
/// `None` si la frame est hors bornes allouables.
/// Équivalent de [`rasterize_cached`] avec un cache jetable
/// — préférer la variante cachée entre repaints (R-36).
#[must_use]
pub fn rasterize(list: &DisplayList, fonts: &dyn FontSource, scale: f32) -> Option<Frame> {
    rasterize_cached(list, fonts, scale, &mut GlyphCache::new())
}

/// [`rasterize`] avec un [`GlyphCache`] persistant (R-36) :
/// faces parsées et contours de glyphes sont réutilisés
/// d'un repaint à l'autre — sync automatique sur
/// `FontSource::revision` (faces `@font-face` inscrites ou
/// purgées → cache vidé).
#[must_use]
pub fn rasterize_cached(
    list: &DisplayList,
    fonts: &dyn FontSource,
    scale: f32,
    gc: &mut GlyphCache,
) -> Option<Frame> {
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

    gc.sync(fonts);
    replay_items(&mut pixmap, &list.items, dpi, fonts, gc, 0, None);
    Some(Frame {
        width: w,
        height: h,
        pixels: pixmap.data().to_vec(),
        scale,
    })
}

/// Rasterise la tranche verticale `[y_css, y_css + height_css[`
/// de `list` dans une frame `list.width × height_css`.
///
/// La hauteur du **document** n'est plus bornée ici : le cap
/// 16 384 px s'applique à la fenêtre demandée, pas à la page —
/// les documents plus hauts que le cap deviennent rasterisables
/// par tranches (demande NAV repriorisée — scroll github/MDN/
/// stallman, `site_smoke`). `y_css` négatif est clampé à 0 ;
/// la portion de fenêtre sous le document reste blanche.
/// Équivalent de [`rasterize_window_cached`] à cache jetable.
#[must_use]
pub fn rasterize_window(
    list: &DisplayList,
    fonts: &dyn FontSource,
    scale: f32,
    y_css: f32,
    height_css: f32,
) -> Option<Frame> {
    rasterize_window_cached(
        list,
        fonts,
        scale,
        y_css,
        height_css,
        &mut GlyphCache::new(),
    )
}

/// [`rasterize_window`] avec un [`GlyphCache`] persistant
/// (R-36) — même sync `FontSource::revision` que
/// [`rasterize_cached`].
#[must_use]
pub fn rasterize_window_cached(
    list: &DisplayList,
    fonts: &dyn FontSource,
    scale: f32,
    y_css: f32,
    height_css: f32,
    gc: &mut GlyphCache,
) -> Option<Frame> {
    // `y_css` négatif accepté : les items sont ancrés
    // `-scroll_y` au build — peindre le document entier
    // dans un pixmap `doc_h`-haut demande la bande
    // `[-scroll_y, doc_h - scroll_y]` (R-N `full=1` à la
    // demande). Le translate `-y_css·scale` fonctionne
    // identiquement en négatif ; `h_px` reste borné à
    // 16 384 ci-dessous.
    let w_px = (list.width * scale).round().max(1.0);
    let h_px = (height_css * scale).round().max(1.0);
    if !(w_px.is_finite() && h_px.is_finite() && y_css.is_finite())
        || w_px > 16384.0
        || h_px > 16384.0
    {
        return None; // borne hostile — fenêtre géante.
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let (w, h) = (w_px as u32, h_px as u32);
    let mut pixmap = Pixmap::new(w, h)?;
    pixmap.fill(tiny_skia::Color::WHITE);
    // p_local = p_css·scale − y0px : la tranche se peint dans
    // son propre repère, pile transform/clip inchangée.
    let base = Transform::from_row(scale, 0.0, 0.0, scale, 0.0, -(y_css * scale));
    let win_css = explorer_layout::Rect {
        x: 0.0,
        y: y_css,
        width: list.width,
        height: h_px / scale,
    };
    // Sondes internes `sync_profile` (audit perf, chantier
    // 10-suite) : `incr.raster` domine le cycle — le split
    // bounds/replay ordonne le chantier (target partagée,
    // tab=0 : PIPE ne connaît pas l'onglet).
    let t = std::time::Instant::now();
    let bounds = crate::display::resolved_bounds(&list.items);
    tracing::info!(
        target: "sync_profile",
        tab = 0u64,
        phase = "raster.bounds",
        us = u64::try_from(t.elapsed().as_micros()).unwrap_or(u64::MAX),
        items = list.items.len(),
        "phase sync"
    );
    gc.sync(fonts);
    let t = std::time::Instant::now();
    replay_items(
        &mut pixmap,
        &list.items,
        base,
        fonts,
        gc,
        0,
        Some((&bounds, win_css)),
    );
    tracing::info!(
        target: "sync_profile",
        tab = 0u64,
        phase = "raster.replay",
        us = u64::try_from(t.elapsed().as_micros()).unwrap_or(u64::MAX),
        "phase sync"
    );
    Some(Frame {
        width: w,
        height: h,
        pixels: pixmap.data().to_vec(),
        scale,
    })
}

/// Rasterise la tranche `win_css` (px CSS, même repère
/// framebuffer que [`rasterize_window`]) en un pixmap
/// **transparent** ne dessinant que les items d'index
/// ≥ `draw_from` — le « top slice » (étape 188,
/// `interface-equipes.md`, demande M2 UX-vidéo) : les items
/// émis après la coupe re-peignent au-dessus des surfaces
/// média natives que l'hôte interpose entre les deux plans.
/// La base servie restant la frame complète, le pixmap
/// borné à `win_css` (session : ∪ des `v∩c` des surfaces)
/// ne perd rien — ce qui tombe hors de la tranche est déjà
/// sous la surface dans la base.
///
/// Les items d'état (`PushClip`/`PushTransform`/`Pop*`) du
/// préfixe `[0..draw_from)` sont rejoués sans dessiner :
/// la pile clip/transform au point de coupe est restituée
/// à l'identique, puis chaque dessin est filtré à `win_css`
/// (même mécanique `resolved_bounds` que la fenêtre).
/// `draw_from == 0` = tranche alpha complète.
/// `None` = borne dépassée (16 384 px par axe) ou pixmap
/// non allouable — comme [`rasterize_window`].
#[must_use]
pub fn rasterize_slice(
    list: &DisplayList,
    fonts: &dyn FontSource,
    scale: f32,
    win_css: explorer_layout::Rect,
    draw_from: usize,
) -> Option<Frame> {
    rasterize_slice_cached(
        list,
        fonts,
        scale,
        win_css,
        draw_from,
        &mut GlyphCache::new(),
    )
}

/// [`rasterize_slice`] avec un [`GlyphCache`] persistant
/// (R-36) — même sync `FontSource::revision` que
/// [`rasterize_cached`].
#[must_use]
pub fn rasterize_slice_cached(
    list: &DisplayList,
    fonts: &dyn FontSource,
    scale: f32,
    win_css: explorer_layout::Rect,
    draw_from: usize,
    gc: &mut GlyphCache,
) -> Option<Frame> {
    let w_px = (win_css.width * scale).round().max(1.0);
    let h_px = (win_css.height * scale).round().max(1.0);
    if !(w_px.is_finite() && h_px.is_finite() && win_css.x.is_finite() && win_css.y.is_finite())
        || w_px > 16384.0
        || h_px > 16384.0
    {
        return None; // borne hostile — fenêtre géante.
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let (w, h) = (w_px as u32, h_px as u32);
    let mut pixmap = Pixmap::new(w, h)?;
    // Pas de fond blanc : le slice se compose en sur-couche
    // alpha au-dessus de base+surfaces — un fond opaque
    // écrirait du blanc par-dessus le média natif.
    let base = Transform::from_row(
        scale,
        0.0,
        0.0,
        scale,
        -(win_css.x * scale),
        -(win_css.y * scale),
    );
    let bounds = crate::display::resolved_bounds(&list.items);
    gc.sync(fonts);
    replay_items(
        &mut pixmap,
        &list.items,
        base,
        fonts,
        gc,
        draw_from,
        Some((&bounds, win_css)),
    );
    Some(Frame {
        width: w,
        height: h,
        pixels: pixmap.data().to_vec(),
        scale,
    })
}

/// Inflation des fenêtres sales (px CSS) — couvre le débord
/// AA (~1 px physique) et l'overhang modeste des glyphes
/// hors de leur boîte de métriques. Le harnais d'équivalence
/// `dirty_raster` est la garde qui la valide.
const DAMAGE_INFLATE_CSS: f32 = 2.0;

/// Borne du nombre de fenêtres — au-delà, le plein cadre
/// est préférable (page hostile, dirty partout).
const DAMAGE_MAX_RECTS: usize = 64;

/// Rasterise `list` en ne repeignant que les fenêtres
/// `damage` (px CSS, espace framebuffer — la sortie de
/// [`DisplayList::changed_rects`]) par-dessus `prev`
/// (consommée : son tampon est muté puis renvoyé).
///
/// `None` = repli [`rasterize`] requis : cible différente
/// (taille, scale), ou borne de fenêtres dépassée.
/// `damage` vide → `prev` renvoyée tel quel (rien à peindre).
///
/// Chaque fenêtre rejoue **tous** les items dont la bounds
/// résolue l'intersecte (ordre de peinture conservé, pile
/// clip/transform identique) — l'équivalence « régional ==
/// plein cadre » est assertée pixel-à-pixel par
/// `tests/dirty_raster.rs`.
/// Équivalent de [`rasterize_dirty_cached`] à cache jetable.
#[must_use]
pub fn rasterize_dirty(
    list: &DisplayList,
    fonts: &dyn FontSource,
    scale: f32,
    prev: Frame,
    damage: &[explorer_layout::Rect],
) -> Option<Frame> {
    rasterize_dirty_cached(list, fonts, scale, prev, damage, &mut GlyphCache::new())
}

/// [`rasterize_dirty`] avec un [`GlyphCache`] persistant
/// (R-36) — même sync `FontSource::revision` que
/// [`rasterize_cached`].
#[must_use]
pub fn rasterize_dirty_cached(
    list: &DisplayList,
    fonts: &dyn FontSource,
    scale: f32,
    mut prev: Frame,
    damage: &[explorer_layout::Rect],
    gc: &mut GlyphCache,
) -> Option<Frame> {
    let w_px = (list.width * scale).round().max(1.0);
    let h_px = (list.height * scale).round().max(1.0);
    if !(w_px.is_finite() && h_px.is_finite()) || w_px > 16384.0 || h_px > 16384.0 {
        return None;
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let (w, h) = (w_px as u32, h_px as u32);
    if prev.width != w || prev.height != h || (prev.scale - scale).abs() > f32::EPSILON {
        return None; // cible différente → plein cadre.
    }
    if damage.is_empty() {
        return Some(prev);
    }
    if damage.len() > DAMAGE_MAX_RECTS {
        return None;
    }

    // Fenêtres physiques : inflation AA (CSS) → scale →
    // agrandissement vers l'extérieur → bornées à la frame.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let mut wins: Vec<(u32, u32, u32, u32)> = damage
        .iter()
        .map(|r| {
            let x0 = ((r.x - DAMAGE_INFLATE_CSS) * scale).floor().max(0.0) as u32;
            let y0 = ((r.y - DAMAGE_INFLATE_CSS) * scale).floor().max(0.0) as u32;
            let x1 = ((r.x + r.width + DAMAGE_INFLATE_CSS) * scale)
                .ceil()
                .min(w_px) as u32;
            let y1 = ((r.y + r.height + DAMAGE_INFLATE_CSS) * scale)
                .ceil()
                .min(h_px) as u32;
            (x0, y0, x1, y1)
        })
        .filter(|(x0, y0, x1, y1)| x1 > x0 && y1 > y0)
        .collect();
    // Fusion des fenêtres qui se recouvrent — jamais deux
    // repeintes sur les mêmes pixels. Triées par (x0,y0) :
    // recouvrement ↔ `x0 <= l.2` (x garanti par le tri) **et**
    // recouvrement vertical `y0 <= l.3 && y1 >= l.1` — un test
    // sur les deux seules bornes max avale des fenêtres
    // disjointes (le tri casse l'ordre de parcours en y).
    let mut merged: Vec<(u32, u32, u32, u32)> = Vec::new();
    wins.sort_unstable();
    for (x0, y0, x1, y1) in wins.drain(..) {
        if let Some(l) = merged.last_mut() {
            if x0 <= l.2 && y0 <= l.3 && y1 >= l.1 {
                l.2 = l.2.max(x1);
                l.3 = l.3.max(y1);
                continue;
            }
        }
        merged.push((x0, y0, x1, y1));
    }

    let bounds = crate::display::resolved_bounds(&list.items);
    gc.sync(fonts);
    for (x0, y0, x1, y1) in merged {
        let (rw, rh) = (x1 - x0, y1 - y0);
        let mut pm = Pixmap::new(rw, rh)?;
        // Même base que `rasterize` : canvas opaque blanche —
        // les trous de la fenêtre retombent sur le fond UA.
        pm.fill(tiny_skia::Color::WHITE);
        // p_local = p_css·scale − (x0,y0) : translation
        // physique composée à l'échelle.
        #[allow(clippy::cast_precision_loss)]
        let base = Transform::from_row(scale, 0.0, 0.0, scale, -(x0 as f32), -(y0 as f32));
        // Fenêtre de filtre en px CSS (bounds résolues y sont).
        #[allow(clippy::cast_precision_loss)]
        let win_css = explorer_layout::Rect {
            x: x0 as f32 / scale,
            y: y0 as f32 / scale,
            width: rw as f32 / scale,
            height: rh as f32 / scale,
        };
        replay_items(
            &mut pm,
            &list.items,
            base,
            fonts,
            gc,
            0,
            Some((&bounds, win_css)),
        );
        // Blit de la fenêtre dans la frame.
        let src = pm.data();
        for row in 0..rh {
            let dst_off = (((y0 + row) * w + x0) * 4) as usize;
            let src_off = (row * rw * 4) as usize;
            let len = (rw * 4) as usize;
            prev.pixels[dst_off..dst_off + len].copy_from_slice(&src[src_off..src_off + len]);
        }
    }
    Some(prev)
}

/// Entrée de la pile `clips` de [`replay_items`] — le
/// `Mask` plein frame (alloc + `fill_path`, ~ms par clip
/// en debug) n'est payé que pour les clips **partiels** :
/// `Empty` = le rect transformé n'intersecte pas le pixmap
/// (le contenu est borné hors écran de toute façon →
/// masque bloquant partagé), `Full` = il le couvre entier
/// (masque identité → `None`, aucun alloc). Mesuré vol
/// dev.to : 4 109 clips/replay dont ~90 % hors fenêtre —
/// le coût dominant du raster fenêtré.
enum ClipEntry {
    /// Hors pixmap — sentinelle `empty_mask` (bloque tout).
    Empty,
    /// Couvre tout le pixmap — aucun masquage.
    Full,
    /// Clip partiel — vrai masque rasterisé.
    Real(Mask),
}

/// Masque actif au sommet de pile : `Real` → son masque,
/// `Empty` → le bloquant partagé, `Full` → aucun.
fn top_clip<'m>(clips: &'m [ClipEntry], empty: Option<&'m Mask>) -> Option<&'m Mask> {
    match clips.last() {
        Some(ClipEntry::Real(m)) => Some(m),
        Some(ClipEntry::Empty) => empty,
        Some(ClipEntry::Full) | None => None,
    }
}

/// Classe un clip rectangulaire (déjà transformé dans le
/// repère pixmap) : hors-pixmap → `Empty`, couvrant →
/// `Full`, partiel → `Real` avec le masque rempli.
/// `None` sur `phys` (rect dégénéré/NaN) → `Empty`, même
/// sémantique qu'avant (masque resté vide = bloque tout).
/// `can_full=false` (clip arrondi : les coins rognent) →
/// un rect couvrant reste `Real`.
/// Réutilise un `Mask` du pool aux dims demandées (sinon
/// alloue) — le buffer repris est remis à zéro, sémantique
/// d'un `Mask::new` frais. `data_mut` n'expose que les
/// octets ; le `fill` reste entier, pas de fuite d'état.
fn take_mask(pool: &mut Vec<Mask>, w: u32, h: u32) -> Option<Mask> {
    let mut m = match pool.iter().position(|p| p.width() == w && p.height() == h) {
        Some(i) => pool.remove(i),
        None => Mask::new(w, h)?,
    };
    m.data_mut().fill(0);
    Some(m)
}

#[allow(clippy::cast_precision_loss)]
fn push_rect_clip(
    clips: &mut Vec<ClipEntry>,
    pool: &mut Vec<Mask>,
    phys: Option<tiny_skia::Rect>,
    can_full: bool,
    w: u32,
    h: u32,
    fill: impl FnOnce(&mut Mask),
) {
    let Some(phys) = phys else {
        clips.push(ClipEntry::Empty);
        return;
    };
    let Some(pix) = tiny_skia::Rect::from_ltrb(0.0, 0.0, w as f32, h as f32) else {
        clips.push(ClipEntry::Empty);
        return;
    };
    if phys.intersect(&pix).is_none() {
        clips.push(ClipEntry::Empty);
        return;
    }
    if can_full
        && phys.left() <= 0.0
        && phys.top() <= 0.0
        && phys.right() >= w as f32
        && phys.bottom() >= h as f32
    {
        clips.push(ClipEntry::Full);
        return;
    }
    // Une entrée est TOUJOURS poussée — sinon le `PopClip`
    // désempile le clip englobant (cas `take_mask` en échec
    // : dégradation bloquante, pile équilibrée).
    match take_mask(pool, w, h) {
        Some(mut mask) => {
            fill(&mut mask);
            clips.push(ClipEntry::Real(mask));
        }
        None => clips.push(ClipEntry::Empty),
    }
}

/// Nature d'un groupe hors-écran sur la pile `filtered` de
/// [`replay_items`] — le kind fait foi au pop (R-252) :
/// `PopFilter`/`PopBlend`/`PopMask` sont interchangeables,
/// l'appariement est garanti par l'émission.
enum GroupKind {
    /// `filter` — ops appliquées avant composite (R-M5).
    Ops(Vec<FilterOp>),
    /// `mix-blend-mode`/`isolation` — fondu au pop (R-185).
    Blend(u8),
    /// `mask-image` de groupe — composite modulé par
    /// l'alpha du bitmap tuilé dans `dst` (R-252).
    Mask {
        /// Aire de peinture (mask-clip, coords document).
        rect: explorer_layout::Rect,
        /// Tuiles des couches — union `add` des couvertures.
        layers: Vec<crate::display::MaskTile>,
    },
}

/// Tampon d'un groupe hors-écran (R-M5), dimensionné à sa
/// **zone d'influence** quand la fenêtre fournit les
/// bounds — audit : `Pixmap::new(w,h)` plein cadre à
/// chaque push + composite plein cadre à chaque pop
/// (4 groupes = 7,1 s de `group_us` sur `YouTube` — même
/// maladie plein-frame que les clips de l'étape 233).
/// La zone = union des bounds peintes restreinte à
/// `parent` ⊕ `portée des ops` (l'encre d'entrée dont un
/// flou/une ombre saigne dans le parent) ∪ sa propre
/// portée en sortie. `void` = aucune influence possible :
/// le composite saute et les contenus peignent dans un
/// puits 1×1 (exact — ops sur transparent = transparent).
struct GroupBuf {
    /// Tampon — taille de la zone, du parent si la zone
    /// n'est pas bornable, ou 1×1 puits quand `void`.
    buf: Pixmap,
    /// Le kind fait foi au pop (R-252).
    kind: GroupKind,
    /// Position du tampon dans l'espace du dst parent —
    /// origine du composite au pop.
    ox: i32,
    oy: i32,
    /// Origine du tampon en coords racine — les items qui
    /// adressent le dst en coords absolues
    /// (`BackdropFilter`) la soustraient.
    ox_root: i32,
    oy_root: i32,
    /// Rien de visible n'en sort — composite sauté au pop.
    void: bool,
    /// `grp_tf` avant le push — le translate
    /// `(-ox,-oy)` qui a fait passer `cur` en espace
    /// tampon est restauré au pop.
    grp_tf_prev: Transform,
}

/// Zone d'influence d'un groupe en coords racine.
enum GroupZone {
    /// Bounds absentes (raster plein cadre sans filtre
    /// fenêtré) ou contenu non bornable — tampon taille
    /// parent, comportement historique.
    Unbounded,
    /// Aucune influence possible sur le dst parent.
    Void,
    /// Rect du tampon (peut déborder du parent côté
    /// entrée : l'encre à portée des ops est conservée).
    Zone { x: i32, y: i32, w: u32, h: u32 },
}

/// Portée spatiale des ops `filter` d'un groupe — les
/// ops de couleur sont à portée nulle ; `blur` étale
/// de `⌈r⌉` px par axe (2 passes `box_blur` rayon r) ;
/// `drop-shadow` décale l'alpha de `(dx,dy)` puis
/// floute — borne `max(|dx|,|dy|) + ⌈blur⌉` conservatrice.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)]
fn ops_reach(ops: &[FilterOp]) -> i32 {
    ops.iter()
        .map(|op| match op {
            FilterOp::Blur(r) => r.max(0.0).ceil() as i32,
            FilterOp::DropShadow { dx, dy, blur, .. } => {
                dx.abs().max(dy.abs()).ceil() as i32 + blur.max(0.0).ceil() as i32
            }
            _ => 0,
        })
        .sum()
}

/// Intersection de deux rects `(x,y,w,h)` flottants.
fn tup_intersect(a: (f32, f32, f32, f32), b: (f32, f32, f32, f32)) -> Option<(f32, f32, f32, f32)> {
    let (x0, y0) = (a.0.max(b.0), a.1.max(b.1));
    let (x1, y1) = ((a.0 + a.2).min(b.0 + b.2), (a.1 + a.3).min(b.1 + b.3));
    (x1 > x0 && y1 > y0).then_some((x0, y0, x1 - x0, y1 - y0))
}

/// Boîte englobante de deux rects `(x,y,w,h)` flottants.
fn tup_union(a: (f32, f32, f32, f32), b: (f32, f32, f32, f32)) -> (f32, f32, f32, f32) {
    let (x0, y0) = (a.0.min(b.0), a.1.min(b.1));
    let (x1, y1) = ((a.0 + a.2).max(b.0 + b.2), (a.1 + a.3).max(b.1 + b.3));
    (x0, y0, x1 - x0, y1 - y0)
}

/// Union des bounds peintes de la portée du groupe qui
/// débute à `start` — lue dans `bounds` (espace
/// `resolved_bounds`, base identité) et ramenée en
/// coords racine par `dpi`. Seuls les items qui
/// peindront comptent (`j >= draw_from` et bounds ∩
/// fenêtre — le même test que la boucle de dessin ;
/// le contenu des sous-groupes imbriqués est couvert
/// par le parcours de portée entier). `parent` = rect
/// du dst en coords racine ; `reach` = [`ops_reach`].
/// `Unbounded` si un item de dessin manque de bounds ou
/// si la portée est déséquilibrée — jamais de zone
/// optimiste.
#[allow(
    clippy::too_many_arguments,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)]
fn group_paint_zone(
    items: &[DisplayItem],
    start: usize,
    draw_from: usize,
    bounds: &[Option<explorer_layout::Rect>],
    win: explorer_layout::Rect,
    dpi: Transform,
    parent: (f32, f32, f32, f32),
    reach: i32,
) -> GroupZone {
    let mut depth = 1i32;
    let mut closed = false;
    let mut painted: Option<(f32, f32, f32, f32)> = None;
    for (j, item) in items.iter().enumerate().skip(start + 1) {
        match item {
            DisplayItem::PushFilter(_)
            | DisplayItem::PushBlend(_)
            | DisplayItem::PushMask { .. } => depth += 1,
            DisplayItem::PopFilter | DisplayItem::PopBlend | DisplayItem::PopMask => {
                depth -= 1;
                if depth == 0 {
                    closed = true;
                    break;
                }
            }
            DisplayItem::PushClip(_)
            | DisplayItem::PushClipRounded { .. }
            | DisplayItem::PushClipPath { .. }
            | DisplayItem::PopClip
            | DisplayItem::PushTransform(_)
            | DisplayItem::PopTransform => {}
            _ => {
                if j >= draw_from {
                    match bounds.get(j).copied().flatten() {
                        // Même test que la boucle de dessin :
                        // un item culé par la fenêtre n'encre
                        // pas le tampon.
                        Some(b) if rects_overlap(b, win) => {
                            let Some(rb) = tiny_skia::Rect::from_xywh(b.x, b.y, b.width, b.height)
                                .and_then(|r| r.transform(dpi))
                            else {
                                return GroupZone::Unbounded;
                            };
                            let rb = (rb.x(), rb.y(), rb.width(), rb.height());
                            painted = Some(painted.map_or(rb, |p| tup_union(p, rb)));
                        }
                        Some(_) => {}
                        None => return GroupZone::Unbounded,
                    }
                }
            }
        }
    }
    if !closed {
        return GroupZone::Unbounded;
    }
    let Some(p) = painted else {
        return GroupZone::Void;
    };
    let r = reach.max(0) as f32;
    let grown = (
        parent.0 - r,
        parent.1 - r,
        parent.2 + 2.0 * r,
        parent.3 + 2.0 * r,
    );
    let out_p = (p.0 - r, p.1 - r, p.2 + 2.0 * r, p.3 + 2.0 * r);
    // Encre d'entrée à portée du parent ∪ pixels de
    // sortie qui retombent dans le parent.
    let z_in = tup_intersect(p, grown);
    let z_out = tup_intersect(out_p, parent);
    let base = match (z_in, z_out) {
        (Some(a), Some(b)) => tup_union(a, b),
        (Some(a), None) | (None, Some(a)) => a,
        (None, None) => return GroupZone::Void,
    };
    // Marge `reach` autour de la SORTIE : les kernels
    // (`box_blur`, ombre) doivent lire le même voisinage
    // qu'en plein cadre — y compris les échantillons
    // transparents au-delà de l'encre, qui comptent dans
    // le diviseur `n` du blur (bord du tampon ≠ bord du
    // frame → sinon divergence d'arrondi aux lisières).
    let zone = match z_out {
        Some(o) => tup_union(base, (o.0 - r, o.1 - r, o.2 + 2.0 * r, o.3 + 2.0 * r)),
        None => base,
    };
    let x = zone.0.floor() as i32;
    let y = zone.1.floor() as i32;
    let x1 = (zone.0 + zone.2).ceil() as i32;
    let y1 = (zone.1 + zone.3).ceil() as i32;
    GroupZone::Zone {
        x,
        y,
        w: (x1 - x).max(1) as u32,
        h: (y1 - y).max(1) as u32,
    }
}

/// Pousse un tampon de groupe dimensionné à sa zone
/// d'influence — factorise les trois `Push*` de
/// [`replay_items`]. Le dst parent = dernier tampon
/// englobant ou la racine ; `void` se propage (un enfant
/// de tampon-vide est vide d'office). `grp_tf`/`cur`
/// basculent en espace tampon quand la zone impose un
/// translate `(-ox,-oy)` ; `grp_tf_prev` restaure au pop.
/// Repli `Unbounded` = tampon taille parent (comportement
/// historique) ; échec d'alloc de la zone → taille parent,
/// puis puits 1×1 — la pile reste appariée.
#[allow(clippy::too_many_arguments, clippy::cast_precision_loss)]
fn push_group(
    filtered: &mut Vec<GroupBuf>,
    kind: GroupKind,
    reach: i32,
    items: &[DisplayItem],
    start: usize,
    draw_from: usize,
    filter: Option<(&[Option<explorer_layout::Rect>], explorer_layout::Rect)>,
    dpi: Transform,
    pw: u32,
    ph: u32,
    grp_tf: &mut Transform,
    tfs: &[Transform],
    cur: &mut Transform,
    n_void: &mut u32,
) {
    let (pox, poy, pdw, pdh) = filtered.last().map_or((0, 0, pw, ph), |p| {
        (p.ox_root, p.oy_root, p.buf.width(), p.buf.height())
    });
    let zone = if filtered.last().is_some_and(|p| p.void) {
        GroupZone::Void
    } else {
        match filter {
            Some((bounds, win)) => group_paint_zone(
                items,
                start,
                draw_from,
                bounds,
                win,
                dpi,
                (pox as f32, poy as f32, pdw as f32, pdh as f32),
                reach,
            ),
            None => GroupZone::Unbounded,
        }
    };
    match zone {
        GroupZone::Void => {
            *n_void += 1;
            if let Some(buf) = Pixmap::new(1, 1) {
                filtered.push(GroupBuf {
                    buf,
                    kind,
                    ox: 0,
                    oy: 0,
                    ox_root: pox,
                    oy_root: poy,
                    void: true,
                    grp_tf_prev: *grp_tf,
                });
            }
        }
        GroupZone::Unbounded => {
            if let Some(buf) = Pixmap::new(pdw, pdh) {
                filtered.push(GroupBuf {
                    buf,
                    kind,
                    ox: 0,
                    oy: 0,
                    ox_root: pox,
                    oy_root: poy,
                    void: false,
                    grp_tf_prev: *grp_tf,
                });
            }
        }
        GroupZone::Zone { x, y, w, h } => {
            if let Some(buf) = Pixmap::new(w, h) {
                let prev = *grp_tf;
                let (ox, oy) = (x - pox, y - poy);
                *grp_tf = grp_tf.post_concat(Transform::from_translate(-ox as f32, -oy as f32));
                *cur = tfs
                    .iter()
                    .fold(dpi, |acc, t| acc.pre_concat(*t))
                    .post_concat(*grp_tf);
                filtered.push(GroupBuf {
                    buf,
                    kind,
                    ox,
                    oy,
                    ox_root: x,
                    oy_root: y,
                    void: false,
                    grp_tf_prev: prev,
                });
            } else if let Some(buf) = Pixmap::new(pdw, pdh) {
                filtered.push(GroupBuf {
                    buf,
                    kind,
                    ox: 0,
                    oy: 0,
                    ox_root: pox,
                    oy_root: poy,
                    void: false,
                    grp_tf_prev: *grp_tf,
                });
            }
        }
    }
}

/// Rejoue les items dans `pixmap` — boucle de peinture
/// partagée par [`rasterize`] et [`rasterize_dirty`].
///
/// `filter` : quand `Some((bounds, window))`, un item de
/// dessin dont la bounds résolue (`resolved_bounds`, même
/// sémantique de pile) n'intersecte pas `window` est sauté ;
/// les items d'état (`Push*/Pop*`) sont toujours rejoués —
/// la pile doit refléter l'emplacement de chaque item.
#[allow(
    clippy::too_many_lines,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)]
fn replay_items(
    pixmap: &mut Pixmap,
    items: &[DisplayItem],
    dpi: Transform,
    fonts: &dyn FontSource,
    gc: &mut GlyphCache,
    draw_from: usize,
    filter: Option<(&[Option<explorer_layout::Rect>], explorer_layout::Rect)>,
) {
    let mut clips: Vec<ClipEntry> = Vec::new();
    // Sentinelle bloquante des clips `Empty` — allouée une
    // fois par replay au lieu d'un `Mask` plein frame par
    // clip hors-fenêtre (audit : ~7,7 ms × 4 109 = 31 s).
    let empty_mask = Mask::new(pixmap.width(), pixmap.height());
    // Pool LIFO des `Mask` `Real` dépiloés — `Mask::new`
    // = alloc + memset `w·h` par clip partiel (~ms en
    // debug ; clip_us résiduel ~1 s/fenêtre typique).
    let mut mask_pool: Vec<Mask> = Vec::new();
    // R-M5 — groupes hors-écran : `PushFilter`/`PushBlend`/
    // `PushMask` poussent un tampon ; les items suivants s'y
    // peignent (`filtered.last_mut()` = cible courante) et le
    // `Pop*` correspondant le composite selon `GroupKind`.
    // Tampons dimensionnés à leur zone d'influence (audit
    // group_us) — `grp_tf` = produit des translates
    // `(-ox,-oy)` de chaque groupe englobant, composé EN
    // TÊTE de `cur` (espace dst, pas doc : `cur =
    // grp_tf · dpi · tfs`) ; restauré à chaque pop via
    // `grp_tf_prev`.
    let mut filtered: Vec<GroupBuf> = Vec::new();
    let mut grp_tf = Transform::default();
    // Pile des `transform` CSS : `cur` = dpi · produit des
    // matrices actives — recomposé à chaque push/pop (peu de
    // niveaux en pratique).
    let mut tfs: Vec<Transform> = Vec::new();
    let mut cur = dpi;
    // Attribution interne (audit perf) : temps par famille —
    // clips (Mask::new + fill_path), composites de groupes
    // (ops/blend/mask-coverage au Pop), dessin effectif ;
    // compteurs par catégorie. Une seule émission en fin.
    let mut us_clip = 0u128;
    let mut us_group = 0u128;
    let mut us_draw = 0u128;
    let (mut n_clip, mut n_group, mut n_draw, mut n_skip) = (0u32, 0u32, 0u32, 0u32);
    // Groupes `void` — aucune influence sur le parent
    // (mesure du taux de skip au re-vol).
    let mut n_group_void = 0u32;
    // `clips_real` = clips ayant payé un `Mask` plein frame —
    // le ratio vs `clips` mesure le taux de culle hors-fenêtre.
    let mut n_clip_real = 0u32;
    // Attribution `draw_us` par kind d'item (P4 perf —
    // texte / remplissages / ombres / images / autre) :
    // `draw_us` ~1 s résiduel dev.to sans décomposition.
    let mut kind_us = [0u64; DRAW_KINDS];
    let mut kind_n = [0u32; DRAW_KINDS];
    for (idx, item) in items.iter().enumerate() {
        match item {
            DisplayItem::PushClip(r) => {
                let t = std::time::Instant::now();
                n_clip += 1;
                let phys =
                    Rect::from_xywh(r.x, r.y, r.width, r.height).and_then(|rr| rr.transform(cur));
                // Sous transform axis-aligned, le clip
                // rectangulaire est arrêté sur la grille de
                // pixels device — cohérent avec les `fill`
                // et blits qui arrêtent leurs bords : un
                // `overflow` fractionnaire ne produit pas
                // de liseré AA que ses voisins n'ont pas.
                let phys = phys.map(|p| {
                    if cur.kx == 0.0 && cur.ky == 0.0 {
                        Rect::from_ltrb(
                            p.left().round(),
                            p.top().round(),
                            p.right().round(),
                            p.bottom().round(),
                        )
                        .unwrap_or(p)
                    } else {
                        p
                    }
                });
                // Clippe au domaine fixed-point : un clip
                // couvrant tout reste plein, un clip
                // lointain reste vide — même sémantique.
                let dom = Rect::from_ltrb(-SAFE_COORD, -SAFE_COORD, SAFE_COORD, SAFE_COORD);
                let clamped = phys.and_then(|p| dom.and_then(|d| p.intersect(&d)));
                // Dims du dst courant — un tampon de groupe
                // borné borne aussi ses clips internes.
                let (cw, ch) = filtered
                    .last()
                    .map_or((pixmap.width(), pixmap.height()), |g| {
                        (g.buf.width(), g.buf.height())
                    });
                push_rect_clip(&mut clips, &mut mask_pool, phys, true, cw, ch, |mask| {
                    if let Some(clamped) = clamped {
                        let path = PathBuilder::from_rect(clamped);
                        mask.fill_path(&path, FillRule::Winding, true, Transform::default());
                    }
                });
                n_clip_real += u32::from(matches!(clips.last(), Some(ClipEntry::Real(_))));
                us_clip += t.elapsed().as_micros();
            }
            // R-M9 — clip arrondi (`overflow` + `border-radius`) :
            // même grille de pixels que `PushClip`, le chemin de
            // masque suit les rayons (`rounded_rect_path`).
            DisplayItem::PushClipRounded { rect: r, radii } => {
                let t = std::time::Instant::now();
                n_clip += 1;
                let phys =
                    Rect::from_xywh(r.x, r.y, r.width, r.height).and_then(|rr| rr.transform(cur));
                let dom = Rect::from_ltrb(-SAFE_COORD, -SAFE_COORD, SAFE_COORD, SAFE_COORD);
                let clamped = phys.and_then(|p| dom.and_then(|d| p.intersect(&d)));
                // `radii` tous nuls ⇒ clip rect strict → `Full`
                // possible ; sinon les coins AA interdisent
                // le sentinelle « couvrant ».
                let rect_only = radii.iter().all(|r| *r == 0.0);
                let (cw, ch) = filtered
                    .last()
                    .map_or((pixmap.width(), pixmap.height()), |g| {
                        (g.buf.width(), g.buf.height())
                    });
                push_rect_clip(
                    &mut clips,
                    &mut mask_pool,
                    phys,
                    rect_only,
                    cw,
                    ch,
                    |mask| {
                        if let Some(clamped) = clamped {
                            if let Some(path) = rounded_rect_path(
                                clamped.left(),
                                clamped.top(),
                                clamped.right(),
                                clamped.bottom(),
                                radii,
                                &ROUND_CORNERS,
                            ) {
                                mask.fill_path(
                                    &path,
                                    FillRule::Winding,
                                    true,
                                    Transform::default(),
                                );
                            }
                        }
                    },
                );
                n_clip_real += u32::from(matches!(clips.last(), Some(ClipEntry::Real(_))));
                us_clip += t.elapsed().as_micros();
            }
            // R-187 — `clip-path:polygon` : mask rempli par
            // `fill_path` sur le chemin du polygone — même
            // pile `clips` que `PushClip`/`PushClipRounded`.
            DisplayItem::PushClipPath { points } => {
                let t = std::time::Instant::now();
                n_clip += 1;
                // Boîte englobante des points transformés : hors
                // pixmap → `Empty`. Jamais `Full` (le polygone
                // peut laisser des trous même à bbox couvrante).
                let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
                let mut pb = tiny_skia::PathBuilder::new();
                let map = |x: f32, y: f32| -> (f32, f32) {
                    let mut pt = tiny_skia::Point::from_xy(x, y);
                    cur.map_point(&mut pt);
                    (pt.x, pt.y)
                };
                if let Some((x, y)) = points.first() {
                    let (mx, my) = map(*x, *y);
                    pb.move_to(mx, my);
                    (x0, y0, x1, y1) = (mx, my, mx, my);
                    for (x, y) in &points[1..] {
                        let (tx, ty) = map(*x, *y);
                        pb.line_to(tx, ty);
                        x0 = x0.min(tx);
                        y0 = y0.min(ty);
                        x1 = x1.max(tx);
                        y1 = y1.max(ty);
                    }
                    pb.close();
                }
                let (cw, ch) = filtered
                    .last()
                    .map_or((pixmap.width(), pixmap.height()), |g| {
                        (g.buf.width(), g.buf.height())
                    });
                let off_pixmap = points.is_empty()
                    || x1 <= 0.0
                    || y1 <= 0.0
                    || x0 >= cw as f32
                    || y0 >= ch as f32;
                if off_pixmap {
                    clips.push(ClipEntry::Empty);
                } else if let Some(mut mask) = take_mask(&mut mask_pool, cw, ch) {
                    if let Some(path) = pb.finish() {
                        mask.fill_path(&path, FillRule::Winding, true, Transform::default());
                    }
                    clips.push(ClipEntry::Real(mask));
                } else {
                    clips.push(ClipEntry::Empty);
                }
                n_clip_real += u32::from(matches!(clips.last(), Some(ClipEntry::Real(_))));
                us_clip += t.elapsed().as_micros();
            }
            DisplayItem::PopClip => {
                // `Real` recyclé dans le pool (LIFO strict) ;
                // `Empty`/`Full`/`None` — rien à recycler.
                if let Some(ClipEntry::Real(m)) = clips.pop() {
                    if mask_pool.len() < MASK_POOL_CAP {
                        mask_pool.push(m);
                    }
                }
            }
            DisplayItem::PushTransform(m) => {
                tfs.push(*m);
                cur = tfs
                    .iter()
                    .fold(dpi, |acc, t| acc.pre_concat(*t))
                    .post_concat(grp_tf);
            }
            DisplayItem::PopTransform => {
                tfs.pop();
                cur = tfs
                    .iter()
                    .fold(dpi, |acc, t| acc.pre_concat(*t))
                    .post_concat(grp_tf);
            }
            DisplayItem::PushFilter(ops) => {
                let t = std::time::Instant::now();
                push_group(
                    &mut filtered,
                    GroupKind::Ops(ops.clone()),
                    ops_reach(ops),
                    items,
                    idx,
                    draw_from,
                    filter,
                    dpi,
                    pixmap.width(),
                    pixmap.height(),
                    &mut grp_tf,
                    &tfs,
                    &mut cur,
                    &mut n_group_void,
                );
                us_group += t.elapsed().as_micros();
            }
            // R-185 — `mix-blend-mode`/`isolation` : groupe
            // hors-écran fondu au pop (tag `BLEND_MODES`,
            // 0 = `isolation` isolant).
            DisplayItem::PushBlend(tag) => {
                let t = std::time::Instant::now();
                push_group(
                    &mut filtered,
                    GroupKind::Blend(*tag),
                    0,
                    items,
                    idx,
                    draw_from,
                    filter,
                    dpi,
                    pixmap.width(),
                    pixmap.height(),
                    &mut grp_tf,
                    &tfs,
                    &mut cur,
                    &mut n_group_void,
                );
                us_group += t.elapsed().as_micros();
            }
            // R-252 — `mask-image` de groupe : le tampon se
            // composite au pop modulé par l'alpha du bitmap
            // (tuile `dst`/`repeat` résolus à l'émission,
            // aire `rect` = border box).
            DisplayItem::PushMask { rect, layers } => {
                let t = std::time::Instant::now();
                push_group(
                    &mut filtered,
                    GroupKind::Mask {
                        rect: *rect,
                        layers: layers.clone(),
                    },
                    0,
                    items,
                    idx,
                    draw_from,
                    filter,
                    dpi,
                    pixmap.width(),
                    pixmap.height(),
                    &mut grp_tf,
                    &tfs,
                    &mut cur,
                    &mut n_group_void,
                );
                us_group += t.elapsed().as_micros();
            }
            // Pop générique : le kind porté par le groupe fait
            // foi — `PopFilter`/`PopBlend`/`PopMask` sont
            // interchangeables à la raster (l'émission garantit
            // l'appariement ; un interleave fautif dégrade en
            // composite du kind réel, jamais un panic).
            DisplayItem::PopFilter | DisplayItem::PopBlend | DisplayItem::PopMask => {
                let t = std::time::Instant::now();
                if let Some(mut g) = filtered.pop() {
                    // Restaure l'espace du dst parent avant le
                    // composite — `cur` redevient doc→parent.
                    grp_tf = g.grp_tf_prev;
                    cur = tfs
                        .iter()
                        .fold(dpi, |acc, tf| acc.pre_concat(*tf))
                        .post_concat(grp_tf);
                    if !g.void {
                        match g.kind {
                            GroupKind::Ops(ops) => {
                                apply_filter_ops(&mut g.buf, &ops);
                                let dst = filtered.last_mut().map_or(&mut *pixmap, |p| &mut p.buf);
                                dst.draw_pixmap(
                                    g.ox,
                                    g.oy,
                                    g.buf.as_ref(),
                                    &tiny_skia::PixmapPaint::default(),
                                    Transform::default(),
                                    top_clip(&clips, empty_mask.as_ref()),
                                );
                            }
                            // `blend_channel` par canal quand le
                            // tag > 0, source-over sinon.
                            GroupKind::Blend(tag) => {
                                let dst = filtered.last_mut().map_or(&mut *pixmap, |p| &mut p.buf);
                                composite_blend(dst, &g.buf, tag, g.ox, g.oy);
                            }
                            GroupKind::Mask { rect, layers } => {
                                // Couverture calculée dans
                                // l'espace du dst parent (`cur`
                                // restauré) à ses dimensions.
                                let (dw, dh) = filtered
                                    .last()
                                    .map_or((pixmap.width(), pixmap.height()), |p| {
                                        (p.buf.width(), p.buf.height())
                                    });
                                let cov = group_mask_coverage(
                                    dw,
                                    dh,
                                    &rect,
                                    &layers,
                                    cur,
                                    top_clip(&clips, empty_mask.as_ref()),
                                );
                                let dst = filtered.last_mut().map_or(&mut *pixmap, |p| &mut p.buf);
                                dst.draw_pixmap(
                                    g.ox,
                                    g.oy,
                                    g.buf.as_ref(),
                                    &tiny_skia::PixmapPaint::default(),
                                    Transform::default(),
                                    cov.as_ref(),
                                );
                            }
                        }
                    }
                }
                us_group += t.elapsed().as_micros();
                n_group += 1;
            }
            // R-M9b — `backdrop-filter` : la région déjà
            // peinte sous `rect` reçoit les ops, le contenu
            // se peint ensuite par-dessus.
            DisplayItem::BackdropFilter { rect: r, ops } => {
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let (bw, bh) = (r.width.ceil() as u32, r.height.ceil() as u32);
                if bw > 0 && bh > 0 {
                    if let Some(mut buf) = Pixmap::new(bw, bh) {
                        // `r` est en coords racine — le dst
                        // englobant peut être un tampon de
                        // groupe décalé : on lit/écrit à
                        // `r - origine_racine` dans son espace.
                        let (gox, goy) = filtered
                            .last()
                            .map_or((0.0, 0.0), |g| (g.ox_root as f32, g.oy_root as f32));
                        let src = filtered.last().map_or(pixmap.as_ref(), |g| g.buf.as_ref());
                        buf.draw_pixmap(
                            0,
                            0,
                            src,
                            &tiny_skia::PixmapPaint::default(),
                            Transform::from_translate(-(r.x - gox), -(r.y - goy)),
                            None,
                        );
                        apply_filter_ops(&mut buf, ops);
                        let dst = filtered.last_mut().map_or(&mut *pixmap, |g| &mut g.buf);
                        dst.draw_pixmap(
                            (r.x - gox).round() as i32,
                            (r.y - goy).round() as i32,
                            buf.as_ref(),
                            &tiny_skia::PixmapPaint::default(),
                            Transform::default(),
                            top_clip(&clips, empty_mask.as_ref()),
                        );
                    }
                }
            }
            _ => {
                // Fenêtre sale : item dont la bounds résolue
                // est hors fenêtre → sauté (son encre ne peut
                // pas y toucher). `None` bounds = conservateur.
                // `idx < draw_from` : sous la coupe top-slice
                // (étape 188) — les Push*/Pop* ci-dessus ont
                // déjà restitué l'état, le dessin est sauté.
                let draw = idx >= draw_from
                    && filter.is_none_or(|(bounds, win)| {
                        bounds
                            .get(idx)
                            .copied()
                            .flatten()
                            .is_none_or(|b| rects_overlap(b, win))
                    });
                if draw {
                    n_draw += 1;
                    let kind = draw_kind(item);
                    let t = std::time::Instant::now();
                    let dst = filtered.last_mut().map_or(&mut *pixmap, |g| &mut g.buf);
                    paint_item(
                        dst,
                        item,
                        cur,
                        fonts,
                        gc,
                        top_clip(&clips, empty_mask.as_ref()),
                    );
                    let us = t.elapsed().as_micros();
                    us_draw += us;
                    kind_us[usize::from(kind)] += u64::try_from(us).unwrap_or(u64::MAX);
                    kind_n[usize::from(kind)] += 1;
                } else {
                    n_skip += 1;
                }
            }
        }
    }
    tracing::info!(
        target: "sync_profile",
        tab = 0u64,
        phase = "raster.parts",
        items = items.len(),
        clips = n_clip,
        clips_real = n_clip_real,
        groups = n_group,
        groups_void = n_group_void,
        draws = n_draw,
        skips = n_skip,
        clip_us = u64::try_from(us_clip).unwrap_or(u64::MAX),
        group_us = u64::try_from(us_group).unwrap_or(u64::MAX),
        draw_us = u64::try_from(us_draw).unwrap_or(u64::MAX),
        draw_text_us = kind_us[0],
        draw_fill_us = kind_us[1],
        draw_shadow_us = kind_us[2],
        draw_img_us = kind_us[3],
        draw_other_us = kind_us[4],
        draw_text_n = kind_n[0],
        "phase sync"
    );
}

/// Intersection non vide de deux rects px CSS — filtre
/// « item × fenêtre » de `replay_items` (R-37).
fn rects_overlap(a: explorer_layout::Rect, b: explorer_layout::Rect) -> bool {
    a.x < b.x + b.width && b.x < a.x + a.width && a.y < b.y + b.height && b.y < a.y + a.height
}

/// Familles de `draw_us` pour l'attribution `raster.parts`
/// (P4 perf) : texte shapé, remplissages/bordures/décor,
/// ombres, images/fond, reste (markers, emphasis, svg…).
const DRAW_KINDS: usize = 5;

/// Bucketing d'un item dessiné — ordre = index de `kind_us`.
fn draw_kind(item: &DisplayItem) -> u8 {
    match item {
        DisplayItem::Text { .. } | DisplayItem::MarkerText { .. } => 0,
        DisplayItem::FillRect { .. }
        | DisplayItem::DecoRect { .. }
        | DisplayItem::Border { .. }
        | DisplayItem::MaskedFill { .. }
        | DisplayItem::BorderImage { .. } => 1,
        DisplayItem::BoxShadow { .. } => 2,
        DisplayItem::Image { .. } | DisplayItem::BackgroundImage { .. } => 3,
        _ => 4,
    }
}

/// Peint un item de dessin (les clips sont gérés par
/// `rasterize` directement).
/// R-178 — trait `text-decoration` stylé : `solid` plein ;
/// `double` = deux tiers fins (haut/bas du slot) ; `dotted`
/// = pas 2·th ; `dashed` = 3·th on / 2·th off ; `wavy` =
/// pseudo-vague en segments alternés (approximation honnête
/// — pas de chemin courbe, mais visuellement ondulée).
fn paint_deco_rect(
    pixmap: &mut Pixmap,
    rect: &explorer_layout::Rect,
    color: Color,
    style: u8,
    dpi: Transform,
    clip: Option<&Mask>,
) {
    use explorer_layout::inline as il;
    let th = rect.height.max(1.0);
    let right = rect.x + rect.width;
    let mut draw = |x: f32, w: f32, y: f32, h: f32| {
        fill_rect_item(
            pixmap,
            &explorer_layout::Rect {
                x,
                y,
                width: w.max(0.0),
                height: h.max(0.0),
            },
            color,
            &[0.0; 4],
            &ROUND_CORNERS,
            dpi,
            clip,
        );
    };
    match style {
        il::DECO_DOUBLE => {
            let t = (th / 3.0).max(1.0);
            draw(rect.x, rect.width, rect.y, t);
            draw(rect.x, rect.width, rect.y + th - t, t);
        }
        il::DECO_DOTTED => {
            let step = (th * 2.0).max(2.0);
            let mut x = rect.x;
            while x < right {
                draw(x, th.min(right - x), rect.y, th);
                x += step;
            }
        }
        il::DECO_DASHED => {
            let (on, off) = ((th * 3.0).max(3.0), (th * 2.0).max(2.0));
            let mut x = rect.x;
            while x < right {
                draw(x, on.min(right - x), rect.y, th);
                x += on + off;
            }
        }
        il::DECO_WAVY => {
            let seg = (th * 1.5).max(2.0);
            let half = (th * 0.5).max(1.0);
            let mut x = rect.x;
            let mut up = true;
            while x < right {
                let y = if up { rect.y } else { rect.y + th - half };
                draw(x, seg.min(right - x), y, half);
                x += seg;
                up = !up;
            }
        }
        _ => fill_rect_item(pixmap, rect, color, &[0.0; 4], &ROUND_CORNERS, dpi, clip),
    }
}

/// Coins `corner-shape` ronds (`DisplayItem::corners` =
/// `[1.0; 4]`) — surfaces bornées : clip overflow,
/// `clip-path:round`, `box-shadow` (R-270).
const ROUND_CORNERS: [f32; 4] = [1.0; 4];

/// R-167 — un `FillRect` : rayon > 0 → fill du chemin
/// arrondi ; sinon rect pixel-snappé (`fill_guarded`).
fn fill_rect_item(
    pixmap: &mut Pixmap,
    rect: &explorer_layout::Rect,
    color: Color,
    radii: &[f32; 4],
    corners: &[f32; 4],
    dpi: Transform,
    clip: Option<&Mask>,
) {
    let mut paint = Paint {
        anti_alias: true,
        ..Paint::default()
    };
    paint.set_color(skia_color(color));
    if radii.iter().any(|r| *r > 0.0) {
        if let Some(path) = rounded_rect_path(
            rect.x,
            rect.y,
            rect.x + rect.width,
            rect.y + rect.height,
            radii,
            corners,
        ) {
            pixmap.fill_path(&path, &paint, FillRule::Winding, dpi, clip);
        }
    } else if let Some(r) = Rect::from_xywh(rect.x, rect.y, rect.width, rect.height) {
        fill_guarded(pixmap, &r, &paint, dpi, clip);
    }
}

/// R-M5 — ops `filter` sur un tampon RGBA premultiplié :
/// chaque op dans l'ordre déclaré (spec) — par-pixel pour
/// les couleurs, passe séparable pour `blur`. Les
/// approximations premultipliées sont documentées.
fn apply_filter_ops(buf: &mut Pixmap, ops: &[FilterOp]) {
    for op in ops {
        match op {
            FilterOp::Blur(r) => box_blur(buf, (*r).max(0.5)),
            FilterOp::DropShadow {
                dx,
                dy,
                blur,
                color,
            } => apply_drop_shadow(buf, *dx, *dy, *blur, *color),
            _ => apply_pixel_op(buf, op),
        }
    }
}

/// R-M11 — `drop-shadow()` : l'ombre est la **forme alpha**
/// du groupe filtré — le canal alpha de `buf` est recopié
/// décalé de `(dx,dy)`, teinté `color` (premultiplié à
/// l'alpha source), flouté `blur` puis le tampon original
/// est recomposé **par-dessus** (src-over) — l'ombre reste
/// sous le contenu quoi qu'il arrive.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_lossless
)]
fn apply_drop_shadow(buf: &mut Pixmap, dx: f32, dy: f32, blur: f32, color: Color) {
    let (w, h) = (buf.width(), buf.height());
    let Some(mut shadow) = Pixmap::new(w, h) else {
        return;
    };
    {
        let (src_px, dst_px) = (buf.data(), shadow.data_mut());
        let (iw, ih) = (
            i32::try_from(w).unwrap_or(i32::MAX),
            i32::try_from(h).unwrap_or(i32::MAX),
        );
        let (odx, ody) = (dx.round() as i32, dy.round() as i32);
        for y in 0..ih {
            let sy = y - ody;
            if sy < 0 || sy >= ih {
                continue;
            }
            for x in 0..iw {
                let sx = x - odx;
                if sx < 0 || sx >= iw {
                    continue;
                }
                let sa = f32::from(src_px[((sy * iw + sx) * 4) as usize + 3]) / 255.0;
                if sa <= 0.0 {
                    continue;
                }
                let d = ((y * iw + x) * 4) as usize;
                dst_px[d] = (f32::from(color.r) * sa).round() as u8;
                dst_px[d + 1] = (f32::from(color.g) * sa).round() as u8;
                dst_px[d + 2] = (f32::from(color.b) * sa).round() as u8;
                dst_px[d + 3] = (f32::from(color.a) * sa).round() as u8;
            }
        }
    }
    if blur > 0.0 {
        box_blur(&mut shadow, blur.max(0.5));
    }
    // src-over : buf peint par-dessus l'ombre.
    let (under, over) = (shadow.take(), buf.data().to_vec());
    buf.data_mut().copy_from_slice(&under);
    // Composite manuel src-over (premultiplié — tiny-skia
    // n'expose pas `draw_pixmap` sur deux Pixmap mutables
    // empruntés ; la boucle est la version sûre).
    {
        let (d, s) = (buf.data_mut(), &over);
        for i in (0..d.len()).step_by(4) {
            let sa = f32::from(s[i + 3]) / 255.0;
            let inv = 1.0 - sa;
            d[i] = s[i].saturating_add((f32::from(d[i]) * inv).round() as u8);
            d[i + 1] = s[i + 1].saturating_add((f32::from(d[i + 1]) * inv).round() as u8);
            d[i + 2] = s[i + 2].saturating_add((f32::from(d[i + 2]) * inv).round() as u8);
            d[i + 3] = s[i + 3].saturating_add((f32::from(d[i + 3]) * inv).round() as u8);
        }
    }
}

/// Op de couleur sur chaque pixel du tampon —
/// `mix(x,y,t)` linéaire ; les matrices agissent sur les
/// canaux RGB premultipliés (approx. : le canal alpha
/// n'entre pas dans le mélange, luminosité alpha-composée).
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::many_single_char_names
)]
fn apply_pixel_op(buf: &mut Pixmap, op: &FilterOp) {
    let px = buf.data_mut();
    for c in px.chunks_exact_mut(4) {
        let (r, g, b, a) = (
            f32::from(c[0]),
            f32::from(c[1]),
            f32::from(c[2]),
            f32::from(c[3]),
        );
        let lum = 0.2126 * r + 0.7152 * g + 0.0722 * b;
        let mix = |x: f32, y: f32, t: f32| x + (y - x) * t;
        let (nr, ng, nb, na) = match op {
            FilterOp::Brightness(f) => (r * f, g * f, b * f, a),
            FilterOp::Contrast(f) => {
                let k = |x: f32| (x - 127.5) * f + 127.5;
                (k(r), k(g), k(b), a)
            }
            FilterOp::Grayscale(t) => (mix(r, lum, *t), mix(g, lum, *t), mix(b, lum, *t), a),
            FilterOp::Invert(t) => (mix(r, a - r, *t), mix(g, a - g, *t), mix(b, a - b, *t), a),
            FilterOp::Opacity(t) => (r * t, g * t, b * t, a * t),
            FilterOp::Sepia(t) => {
                let sr = 0.393 * r + 0.769 * g + 0.189 * b;
                let sg = 0.349 * r + 0.686 * g + 0.168 * b;
                let sb = 0.272 * r + 0.534 * g + 0.131 * b;
                (mix(r, sr, *t), mix(g, sg, *t), mix(b, sb, *t), a)
            }
            FilterOp::Saturate(s) => (
                lum + (r - lum) * s,
                lum + (g - lum) * s,
                lum + (b - lum) * s,
                a,
            ),
            FilterOp::Blur(_) | FilterOp::DropShadow { .. } => (r, g, b, a),
            FilterOp::HueRotate(d) => {
                let rad = d.to_radians();
                let (c1, c2) = (rad.cos(), rad.sin());
                (
                    r * (0.213 + c1 * 0.787 - c2 * 0.213)
                        + g * (0.715 - c1 * 0.715 - c2 * 0.715)
                        + b * (0.072 - c1 * 0.072 + c2 * 0.928),
                    r * (0.213 - c1 * 0.213 + c2 * 0.143)
                        + g * (0.715 + c1 * 0.285 + c2 * 0.140)
                        + b * (0.072 - c1 * 0.072 - c2 * 0.283),
                    r * (0.213 - c1 * 0.213 - c2 * 0.787)
                        + g * (0.715 - c1 * 0.715 + c2 * 0.715)
                        + b * (0.072 + c1 * 0.928 + c2 * 0.072),
                    a,
                )
            }
        };
        c[0] = nr.clamp(0.0, 255.0) as u8;
        c[1] = ng.clamp(0.0, 255.0) as u8;
        c[2] = nb.clamp(0.0, 255.0) as u8;
        c[3] = na.clamp(0.0, 255.0) as u8;
    }
}

/// Box blur séparable (2 passes fenêtre) — approximation
/// du gaussien de `blur()`, documentée (R-M5).
#[allow(
    clippy::cast_possible_truncation,
    clippy::many_single_char_names,
    clippy::cast_sign_loss
)]
fn box_blur(buf: &mut Pixmap, radius: f32) {
    let (w, h) = (buf.width() as usize, buf.height() as usize);
    let r = radius.max(0.0) as usize;
    if w == 0 || h == 0 || r == 0 {
        return;
    }
    let mut tmp = buf.data().to_vec();
    let dst = buf.data_mut();
    for y in 0..h {
        for x in 0..w {
            let mut acc = [0u32; 4];
            let (x0, x1) = (x.saturating_sub(r), (x + r + 1).min(w));
            for xs in x0..x1 {
                let i = (y * w + xs) * 4;
                acc[0] += u32::from(tmp[i]);
                acc[1] += u32::from(tmp[i + 1]);
                acc[2] += u32::from(tmp[i + 2]);
                acc[3] += u32::from(tmp[i + 3]);
            }
            let n = (x1 - x0) as u32;
            let o = (y * w + x) * 4;
            for (ch, dstb) in dst[o..o + 4].iter_mut().enumerate() {
                *dstb = (acc[ch] / n) as u8;
            }
        }
    }
    tmp.copy_from_slice(dst);
    for y in 0..h {
        for x in 0..w {
            let mut acc = [0u32; 4];
            let (y0, y1) = (y.saturating_sub(r), (y + r + 1).min(h));
            for ys in y0..y1 {
                let i = (ys * w + x) * 4;
                acc[0] += u32::from(tmp[i]);
                acc[1] += u32::from(tmp[i + 1]);
                acc[2] += u32::from(tmp[i + 2]);
                acc[3] += u32::from(tmp[i + 3]);
            }
            let n = (y1 - y0) as u32;
            let o = (y * w + x) * 4;
            for (ch, dstb) in dst[o..o + 4].iter_mut().enumerate() {
                *dstb = (acc[ch] / n) as u8;
            }
        }
    }
}

/// R-M2 — `box-shadow` externe : rect décalé + spread,
/// flou approximé en bandes alpha concentriques (rampe
/// lissée, pas de gaussien — documenté). Les rayons
/// suivent `border-radius` (déjà dans `radii`).
#[allow(
    clippy::too_many_arguments,
    clippy::cast_sign_loss,
    clippy::cast_possible_truncation
)]
fn paint_box_shadow(
    pixmap: &mut Pixmap,
    rect: &explorer_layout::Rect,
    offset: (f32, f32),
    blur: f32,
    spread: f32,
    color: Color,
    radii: &[f32; 4],
    dpi: Transform,
    clip: Option<&Mask>,
) {
    let steps: u16 = if blur > 0.0 { 8 } else { 1 };
    for i in (0..steps).rev() {
        let grow = spread + blur * f32::from(i) / f32::from(steps);
        let mut paint = Paint {
            anti_alias: true,
            ..Paint::default()
        };
        let mut c = color;
        c.a = (f32::from(c.a) * (1.0 - f32::from(i) / f32::from(steps)))
            .round()
            .clamp(0.0, 255.0) as u8;
        paint.set_color(skia_color(c));
        let (x0, y0) = (rect.x + offset.0 - grow, rect.y + offset.1 - grow);
        let (x1, y1) = (
            rect.x + offset.0 + rect.width + grow,
            rect.y + offset.1 + rect.height + grow,
        );
        let rr: [f32; 4] = [
            radii[0] + grow,
            radii[1] + grow,
            radii[2] + grow,
            radii[3] + grow,
        ];
        if let Some(path) = rounded_rect_path(x0, y0, x1, y1, &rr, &ROUND_CORNERS) {
            pixmap.fill_path(&path, &paint, FillRule::Winding, dpi, clip);
        }
    }
}

#[allow(clippy::too_many_lines)]
fn paint_item(
    pixmap: &mut Pixmap,
    item: &DisplayItem,
    dpi: Transform,
    fonts: &dyn FontSource,
    gc: &mut GlyphCache,
    clip: Option<&Mask>,
) {
    match item {
        DisplayItem::FillRect {
            rect,
            color,
            radii,
            corners,
        } => {
            fill_rect_item(pixmap, rect, *color, radii, corners, dpi, clip);
        }
        DisplayItem::DecoRect { rect, color, style } => {
            paint_deco_rect(pixmap, rect, *color, *style, dpi, clip);
        }
        DisplayItem::BoxShadow {
            rect,
            offset,
            blur,
            spread,
            color,
            radii,
        } => {
            paint_box_shadow(
                pixmap, rect, *offset, *blur, *spread, *color, radii, dpi, clip,
            );
        }
        DisplayItem::Border {
            rect,
            widths,
            colors,
            styles,
            radii,
            corners,
        } => {
            paint_border(
                pixmap, rect, widths, colors, *styles, radii, corners, dpi, clip,
            );
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
                run.as_ref(),
                font,
                skia_color(*color),
                *size_px,
                dpi,
                fonts,
                gc,
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
            blend,
        } => {
            paint_tiled(pixmap, rect, img, *repeat, *origin, dpi, clip, *blend);
        }
        DisplayItem::MaskedFill {
            rect,
            color,
            mask,
            dst,
            repeat,
            luminance,
        } => {
            paint_masked_fill(
                pixmap, rect, *color, mask, dst, *repeat, *luminance, dpi, clip,
            );
        }
        // R-186 — `text-emphasis` : marque par glyphe au-
        // dessus (over) ou dessous du run.
        DisplayItem::Emphasis {
            origin,
            run,
            size_px,
            kind,
            color,
            over,
        } => {
            paint_emphasis(
                pixmap, *origin, run, *size_px, *kind, *color, *over, dpi, clip,
            );
        }
        DisplayItem::Marker { rect, kind, color } => {
            paint_marker(pixmap, rect, *kind, *color, dpi, clip);
        }
        // R-108 — marqueur textuel : la fonte se résout ici
        // (la display list ne shape pas), run shapé puis peint.
        DisplayItem::MarkerText {
            origin,
            text,
            font,
            color,
            size_px,
        } => paint_marker_text(
            pixmap, *origin, text, font, *color, *size_px, dpi, fonts, gc, clip,
        ),
        DisplayItem::BorderImage {
            rect,
            img,
            src_edges,
            dst_edges,
            fill_center,
            repeat,
        } => {
            paint_border_image(
                pixmap,
                rect,
                img,
                *src_edges,
                *dst_edges,
                *fill_center,
                *repeat,
                dpi,
                clip,
            );
        }
        DisplayItem::PushClip(_)
        | DisplayItem::PushClipRounded { .. }
        | DisplayItem::PushClipPath { .. }
        | DisplayItem::PopClip
        | DisplayItem::PushTransform(_)
        | DisplayItem::PushFilter(_)
        | DisplayItem::PopFilter
        | DisplayItem::PushBlend(_)
        | DisplayItem::PopBlend
        | DisplayItem::PushMask { .. }
        | DisplayItem::PopMask
        | DisplayItem::BackdropFilter { .. }
        | DisplayItem::PopTransform => {}
    }
}

/// R-248 — `border-image` : l'image est découpée en 9
/// régions (`src_edges` = insets `t,r,b,l` en pixels
/// source) et chacune est blittée vers la bordure du
/// border-box (`dst_edges` = épaisseurs résolues). Les
/// coins sont copiés sans recale, les côtés étirés (ou
/// tilés `repeat`/`round`/`space` — approximation
/// stretch-tuile), le centre peint si `fill` (`stretch`).
/// Découpe en deux axes : `src_y`/`dst_y` verticaux ×
/// `src_x`/`dst_x` horizontaux — 9 couples `(src,dst)`.
#[allow(clippy::too_many_arguments)]
fn paint_border_image(
    pixmap: &mut Pixmap,
    rect: &explorer_layout::Rect,
    img: &crate::DecodedImage,
    src_edges: [f32; 4],
    dst_edges: [f32; 4],
    fill_center: bool,
    repeat: (u8, u8),
    dpi: Transform,
    clip: Option<&Mask>,
) {
    // Casts f32↔u32 sûrs : dimensions image < 2^23 (précision
    // f32 suffisante) et bords clampés ≥0 amont (signe garanti).
    #[allow(clippy::cast_precision_loss)]
    let (iw, ih) = (img.width as f32, img.height as f32);
    // Insets source/déstination `(t, r, b, l)` en pixels —
    // convention `<0` = pourcentage de la dimension (t/b
    // contre ih, r/l contre iw) ; `≥0` = px.
    let px = |v: f32, dim: f32| if v < 0.0 { (-v / 100.0) * dim } else { v };
    let st = px(src_edges[0], ih).clamp(0.0, ih);
    let sr = px(src_edges[1], iw).clamp(0.0, iw);
    let sb = px(src_edges[2], ih).clamp(0.0, ih);
    let sl = px(src_edges[3], iw).clamp(0.0, iw);
    let dt = dst_edges[0].clamp(0.0, rect.height);
    let dr = dst_edges[1].clamp(0.0, rect.width);
    let db = dst_edges[2].clamp(0.0, rect.height);
    let dl = dst_edges[3].clamp(0.0, rect.width);
    // 9 régions : (src x/y, dst x/y) — `None` = zéro
    // (région absente, sautée). Index : 0 gauche, 1 centre,
    // 2 droite — mêmes pour top/center/bottom.
    let sx = [(0.0, sl), (sl, iw - sl - sr), (iw - sr, sr)];
    let sy = [(0.0, st), (st, ih - st - sb), (ih - sb, sb)];
    let dx = [(0.0, dl), (dl, rect.width - dl - dr), (rect.width - dr, dr)];
    let dy = [
        (0.0, dt),
        (dt, rect.height - dt - db),
        (rect.height - db, db),
    ];
    for (yi, &(_y0, _h)) in sy.iter().enumerate() {
        for (xi, &(_x0, _w)) in sx.iter().enumerate() {
            // Coin (yi,xi = 0|2) : pas d'étirement ; bord/centre
            // : stretch sur l'axe variable, repeat sur l'axe fixe.
            let corner = xi != 1 && yi != 1;
            let center = xi == 1 && yi == 1;
            if center && !fill_center {
                continue;
            }
            let s0 = sx[xi];
            let s1 = sy[yi];
            let d0 = dx[xi];
            let d1 = dy[yi];
            if s0.1 <= 0.0 || s1.1 <= 0.0 || d0.1 <= 0.0 || d1.1 <= 0.0 {
                continue;
            }
            let _ = (corner, center, repeat);
            // Sous-rect source (s0.x,s0.y → s0.w × s1.h)
            // étiré dans la zone dst — `paint_image`
            // scale l'intégrale de la vue (≡ stretch ;
            // repeat/round ≈ stretch-tuile, approximation
            // documentée R-248).
            // `s0.1`/`s1.1` > 0 vérifié ; `ceil` < iw/ih donc
            // le cast ne peut ni tronquer ni perdre le signe.
            #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
            let sw = s0.1.ceil() as u32;
            #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
            let sh = s1.1.ceil() as u32;
            #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
            let src_x = s0.0.floor() as u32;
            #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
            let src_y = s1.0.floor() as u32;
            if sw == 0 || sh == 0 || src_x >= img.width || src_y >= img.height {
                continue;
            }
            let mut rgba = vec![0u8; (sw * sh * 4) as usize];
            for row in 0..sh.min(img.height - src_y) {
                let src = ((src_y + row) * img.width + src_x) as usize * 4;
                let copy_w = sw.min(img.width - src_x) as usize * 4;
                let dst_o = (row * sw) as usize * 4;
                rgba[dst_o..dst_o + copy_w].copy_from_slice(&img.rgba[src..src + copy_w]);
            }
            let dst = explorer_layout::Rect {
                x: rect.x + d0.0,
                y: rect.y + d1.0,
                width: d0.1,
                height: d1.1,
            };
            let view = crate::DecodedImage {
                width: sw,
                height: sh,
                rgba,
            };
            paint_image(pixmap, &dst, &view, dpi, clip);
        }
    }
}

/// Marqueur `list-item` (issue #9) — forme primitive inscrite
/// dans `rect` : `disc`/`square` pleins, `circle` en anneau.
/// `fill_guarded` couvre le carré ; le disque passe par un
/// chemin ovale (même garde de domaine via `device_bbox`).
/// Marqueur textuel (R-108) — la fonte est résolue via le port
/// `FontSource` (la display list ne shape pas), le libellé est
/// shapé puis peint comme un run ordinaire.
#[allow(clippy::too_many_arguments)]
fn paint_marker_text(
    pixmap: &mut Pixmap,
    origin: (f32, f32),
    text: &str,
    font: &FontRequest,
    color: Color,
    size_px: f32,
    dpi: Transform,
    fonts: &dyn FontSource,
    gc: &mut GlyphCache,
    clip: Option<&Mask>,
) {
    let Some((data, face)) = fonts.font_face(font) else {
        return;
    };
    let Some(run) = explorer_layout::font::shape_face(&data, face, text, size_px, false) else {
        return;
    };
    paint_text(
        pixmap,
        origin.0,
        origin.1,
        &run,
        font,
        skia_color(color),
        size_px,
        dpi,
        fonts,
        gc,
        clip,
    );
}

fn paint_marker(
    pixmap: &mut Pixmap,
    rect: &explorer_layout::Rect,
    kind: crate::display::MarkerKind,
    color: Color,
    dpi: Transform,
    clip: Option<&Mask>,
) {
    let mut paint = Paint {
        anti_alias: true,
        ..Paint::default()
    };
    paint.set_color(skia_color(color));
    let Some(r) = Rect::from_xywh(rect.x, rect.y, rect.width, rect.height) else {
        return;
    };
    match kind {
        MarkerKind::Square => fill_guarded(pixmap, &r, &paint, dpi, clip),
        MarkerKind::Disc | MarkerKind::Circle => {
            // Le disque est un chemin : la garde de domaine
            // des fills rectangulaires est rejouée sur la
            // bbox transformée avant d'entrer dans le raster.
            let domain = Rect::from_ltrb(-SAFE_COORD, -SAFE_COORD, SAFE_COORD, SAFE_COORD);
            let ok = device_bbox(&r, dpi)
                .zip(domain)
                .is_some_and(|(b, d)| b.intersect(&d).is_some());
            if !ok {
                return;
            }
            let mut pb = PathBuilder::new();
            pb.push_oval(r);
            let Some(path) = pb.finish() else {
                return;
            };
            if kind == MarkerKind::Disc {
                pixmap.fill_path(&path, &paint, FillRule::Winding, dpi, clip);
            } else {
                // Anneau ~8 % du diamètre, ≥ 1 px — l'épaisseur
                // est en coords locales (la transform l'échelle).
                let stroke = Stroke {
                    width: (rect.width * 0.08).max(1.0),
                    ..Stroke::default()
                };
                pixmap.stroke_path(&path, &paint, &stroke, dpi, clip);
            }
        }
    }
}

/// `background-color` passé par l'alpha d'un `mask-image`
/// (R-52) : la couleur n'apparaît que là où le mask est
/// opaque. Le bitmap est échantillonné en nearest dans la
/// tuile `dst` (zone de `mask-size`/`mask-position` résolue
/// à l'émission), tuilée selon `repeat` — hors tuile sans
/// repeat = couverture nulle. Un `clip` (`overflow`) actif
/// est composé par `min` des couvertures ≈ intersection
/// (les masks sont quasi binaires dans le motif).
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::too_many_arguments
)]
#[allow(clippy::too_many_lines)]
fn paint_masked_fill(
    pixmap: &mut Pixmap,
    rect: &explorer_layout::Rect,
    color: Color,
    img: &crate::DecodedImage,
    dst: &explorer_layout::Rect,
    repeat: (u8, u8),
    luminance: bool,
    cur: Transform,
    clip: Option<&Mask>,
) {
    let Some(local) = Rect::from_xywh(rect.x, rect.y, rect.width, rect.height) else {
        return;
    };
    let Some(phys) = local.transform(cur) else {
        return;
    };
    // Bords arrêtés sous transform axis-aligned — cohérent
    // avec les fills et clips voisins.
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
    let (iw, ih) = (img.width, img.height);
    if iw == 0 || ih == 0 || phys.width() <= 0.0 || phys.height() <= 0.0 {
        return;
    }
    let (pw, ph) = (pixmap.width(), pixmap.height());
    let dom = Rect::from_ltrb(0.0, 0.0, pw as f32, ph as f32);
    let Some(zone) = dom.and_then(|d| phys.intersect(&d)) else {
        return;
    };
    let Some(mut m) = Mask::new(pw, ph) else {
        return;
    };
    // Tuile du mask en coords device — `dst` transformée et
    // arrêtée comme `phys` (axis-aligned → bords nets).
    let Some(local_dst) = Rect::from_xywh(dst.x, dst.y, dst.width, dst.height) else {
        return;
    };
    let Some(dst_phys) = local_dst.transform(cur) else {
        return;
    };
    let dst_phys = if cur.kx == 0.0 && cur.ky == 0.0 {
        Rect::from_ltrb(
            dst_phys.left().round(),
            dst_phys.top().round(),
            dst_phys.right().round(),
            dst_phys.bottom().round(),
        )
        .unwrap_or(dst_phys)
    } else {
        dst_phys
    };
    let (dx0, dy0) = (
        dst_phys.left().round() as i32,
        dst_phys.top().round() as i32,
    );
    let (dw, dh) = (
        dst_phys.width().round() as i32,
        dst_phys.height().round() as i32,
    );
    if dw <= 0 || dh <= 0 {
        return;
    }
    let (tile_w, tile_h) = (dw as usize, dh as usize);
    let (img_w, img_h) = (iw as usize, ih as usize);
    let (x0, y0) = (zone.left().max(0.0) as i32, zone.top().max(0.0) as i32);
    let (x1, y1) = (zone.right().ceil() as i32, zone.bottom().ceil() as i32);
    let w = pw as usize;
    let data = m.data_mut();
    for py in y0..y1 {
        // Tuilage `mask-repeat` ±∞ (`rem_euclid` gère les
        // rel négatifs — pixels avant l'origine de la tuile) ;
        // sans repeat, hors tuile = couverture nulle (mask
        // absent → pixel masqué, pas peint).
        let Some(ty) = tile_sample((py - dy0) as f32, dh as f32, (y1 - dy0) as f32, repeat.1)
        else {
            continue;
        };
        let sy = (ty as usize * img_h / tile_h).min(img_h - 1);
        for px in x0..x1 {
            let Some(tx) = tile_sample((px - dx0) as f32, dw as f32, (x1 - dx0) as f32, repeat.0)
            else {
                continue;
            };
            let sx = (tx as usize * img_w / tile_w).min(img_w - 1);
            let i4 = (sy * img_w + sx) * 4;
            data[py as usize * w + px as usize] = if luminance {
                let (rc, gc, bc, ac) = (
                    u32::from(img.rgba[i4]),
                    u32::from(img.rgba[i4 + 1]),
                    u32::from(img.rgba[i4 + 2]),
                    u32::from(img.rgba[i4 + 3]),
                );
                ((rc * 2126 + gc * 7152 + bc * 722 + 5000) / 10000 * ac / 255) as u8
            } else {
                img.rgba[i4 + 3]
            };
        }
    }
    // Compose avec le clip actif (`overflow`) — min des
    // couvertures ≈ intersection des zones.
    if let Some(c) = clip {
        for (d, e) in m.data_mut().iter_mut().zip(c.data().iter()) {
            *d = (*d).min(*e);
        }
    }
    let mut paint = Paint {
        anti_alias: true,
        ..Paint::default()
    };
    paint.set_color(skia_color(color));
    pixmap.fill_rect(phys, &paint, Transform::default(), Some(&m));
}

/// Couverture d'un groupe `PushMask` (R-252) : mask
/// canvas-size dont l'alpha = celui du bitmap tuilé dans
/// `tile` (zone `rect` = border box transformée — mask
/// painting area initiale ; hors zone = 0 → groupe
/// invisible, conforme spec : mask hors zone = élément
/// masqué entièrement), composé en `min` avec le clip
/// actif. Même échantillonnage nearest que
/// [`paint_masked_fill`]. `None` = allocation impossible.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::too_many_arguments
)]
fn group_mask_coverage(
    pw: u32,
    ph: u32,
    rect: &explorer_layout::Rect,
    layers: &[crate::display::MaskTile],
    cur: Transform,
    clip: Option<&Mask>,
) -> Option<Mask> {
    // Union `add` des couvertures des couches — chaque tuile
    // est échantillonnée à part puis l'alpha retenu est le
    // max (union). `intersect`/`subtract`/`exclude` = résidus.
    let mut m = Mask::new(pw, ph)?;
    for tile in layers {
        coverage_tile(&mut m, rect, tile, cur);
    }
    if let Some(c) = clip {
        for (d, e) in m.data_mut().iter_mut().zip(c.data().iter()) {
            *d = (*d).min(*e);
        }
    }
    Some(m)
}

/// Accumule la couverture d'une tuile dans `m` (union max) —
/// corps de boucle extrait de la version mono-couche.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]
fn coverage_tile(
    m: &mut Mask,
    rect: &explorer_layout::Rect,
    layer: &crate::display::MaskTile,
    cur: Transform,
) {
    let (img, tile, repeat, luminance) = (&layer.mask, &layer.dst, layer.repeat, layer.luminance);
    let (pw, ph) = (m.width(), m.height());
    let (iw, ih) = (img.width as usize, img.height as usize);
    if iw == 0 || ih == 0 {
        return;
    }
    let Some(phys) =
        Rect::from_xywh(rect.x, rect.y, rect.width, rect.height).and_then(|r| r.transform(cur))
    else {
        return;
    };
    let dom = Rect::from_ltrb(0.0, 0.0, pw as f32, ph as f32);
    let Some(zone) = dom.and_then(|d| phys.intersect(&d)) else {
        return;
    };
    let Some(dst_phys) =
        Rect::from_xywh(tile.x, tile.y, tile.width, tile.height).and_then(|r| r.transform(cur))
    else {
        return;
    };
    let (dx0, dy0) = (
        dst_phys.left().round() as i32,
        dst_phys.top().round() as i32,
    );
    let (dw, dh) = (
        dst_phys.width().round() as i32,
        dst_phys.height().round() as i32,
    );
    if dw <= 0 || dh <= 0 {
        return;
    }
    let (tile_w, tile_h) = (dw as usize, dh as usize);
    let (x0, y0) = (zone.left().max(0.0) as i32, zone.top().max(0.0) as i32);
    let (x1, y1) = (
        zone.right().ceil().min(pw as f32) as i32,
        zone.bottom().ceil().min(ph as f32) as i32,
    );
    let w = pw as usize;
    let data = m.data_mut();
    for py in y0..y1 {
        let Some(ty) = tile_sample((py - dy0) as f32, dh as f32, (y1 - dy0) as f32, repeat.1)
        else {
            continue;
        };
        let sy = (ty as usize * ih / tile_h).min(ih - 1);
        for px in x0..x1 {
            let Some(tx) = tile_sample((px - dx0) as f32, dw as f32, (x1 - dx0) as f32, repeat.0)
            else {
                continue;
            };
            let sx = (tx as usize * iw / tile_w).min(iw - 1);
            let i4 = (sy * iw + sx) * 4;
            let cov = if luminance {
                let (rc, gc, bc, ac) = (
                    u32::from(img.rgba[i4]),
                    u32::from(img.rgba[i4 + 1]),
                    u32::from(img.rgba[i4 + 2]),
                    u32::from(img.rgba[i4 + 3]),
                );
                ((rc * 2126 + gc * 7152 + bc * 722 + 5000) / 10000 * ac / 255) as u8
            } else {
                img.rgba[i4 + 3]
            };
            let d = &mut data[py as usize * w + px as usize];
            *d = match layer.composite {
                1 => (*d).min(cov),                 // intersect
                2 => (*d) * (255 - cov) / 255,      // subtract
                3 => (*d).max(cov) - (*d).min(cov), // exclude
                _ => (*d).max(cov),                 // add (union)
            };
        }
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

/// Blit bilinéaire dans la bbox de `rect` transformée par
/// `cur` (scale/translate/flip — le miroir de contenu n'est
/// pas reproduit : approximation documentée). §B-32 : le
/// nearest-neighbor historique aliassait tout `<img>`
/// redimensionné — les rasters terrain étaient « pixelisés »
/// (un downscale ×4 ne gardait qu'un pixel sur quatre). La
/// filtration se fait en **prémultiplié** : les texels à
/// bord transparent n'injectent pas de couleur fantôme.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)]
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
    // Centre de pixel → source : u = (dx − x0 + ½)·(iw/dw) − ½.
    let du = iw as f32 / (dw.max(1)) as f32;
    let dv = ih as f32 / (dh.max(1)) as f32;
    for dy in dy0..dy1 {
        // Bord clampé : un `v` légèrement hors-borne
        // (arrondi de `phys`) échantillonne la ligne extrême
        // plutôt que de sauter la rangée.
        let vf = ((dy - y0) as f32 + 0.5)
            .mul_add(dv, -0.5)
            .clamp(0.0, ih as f32 - 1.0);
        let vy = vf.floor();
        let wy = vf - vy;
        let (ya, yb) = (vy as usize, ((vy as usize) + 1).min(ih - 1));
        for dx in dx0..dx1 {
            let i = dy as usize * pw_u + dx as usize;
            if clip_data.is_some_and(|m| m[i] == 0) {
                continue;
            }
            let uf = ((dx - x0) as f32 + 0.5)
                .mul_add(du, -0.5)
                .clamp(0.0, iw as f32 - 1.0);
            let ux = uf.floor();
            let wx = uf - ux;
            let (xa, xb) = (ux as usize, ((ux as usize) + 1).min(iw - 1));
            // Bilinear en prémultiplié : p = Σ w·(c·a, a).
            let mut fa = 0.0f32;
            let mut fr = 0.0f32;
            let mut fg = 0.0f32;
            let mut fb = 0.0f32;
            for (yy, wy_) in [(ya, 1.0 - wy), (yb, wy)] {
                for (xx, wx_) in [(xa, 1.0 - wx), (xb, wx)] {
                    let src_i = (yy * iw + xx) * 4;
                    let wgt = wx_ * wy_;
                    let tex_a = f32::from(img.rgba[src_i + 3]);
                    fa += tex_a * wgt;
                    fr += f32::from(img.rgba[src_i]) * tex_a * wgt;
                    fg += f32::from(img.rgba[src_i + 1]) * tex_a * wgt;
                    fb += f32::from(img.rgba[src_i + 2]) * tex_a * wgt;
                }
            }
            // Source-over prémultiplié : src + dst·(1−α) —
            // `fr`… sont des canaux ×α·255 → /255 = canal
            // prémultiplié 0–255 ; `fa` est l'α filtré.
            let inv = 255.0 - fa;
            let prev = dst[i];
            let blend = |sv: f32, pv: u8| {
                (sv + f32::from(pv) * inv / 255.0).round().clamp(0.0, 255.0) as u8
            };
            dst[i] = tiny_skia::PremultipliedColorU8::from_rgba(
                blend(fr / 255.0, prev.red()),
                blend(fg / 255.0, prev.green()),
                blend(fb / 255.0, prev.blue()),
                (fa + f32::from(prev.alpha()) * inv / 255.0)
                    .round()
                    .clamp(0.0, 255.0) as u8,
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
/// R-185 — composite un tampon de groupe (`mix-blend-mode`
/// / `isolation`) sur la destination : `blend_channel` par
/// canal pour tag > 0, copie source-over sinon — pixels
/// prémultipliés démultipliés au passage. Le fondu se fait
/// à la position `(ox,oy)` — le tampon de groupe est
/// dimensionné à sa zone, sa position dans le dst n'est
/// plus `(0,0)` (audit `group_us` : l'itération est bornée
/// à `src`, plus au cadre entier).
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::cast_possible_wrap
)]
fn composite_blend(dst: &mut Pixmap, src: &Pixmap, tag: u8, ox: i32, oy: i32) {
    let (dw, dh) = (dst.width() as i32, dst.height() as i32);
    for sy in 0..src.height() as i32 {
        let y = sy + oy;
        if y < 0 || y >= dh {
            continue;
        }
        for sx in 0..src.width() as i32 {
            let x = sx + ox;
            if x < 0 || x >= dw {
                continue;
            }
            let i = y as usize * dw as usize + x as usize;
            let Some(sp) = src.pixel(sx as u32, sy as u32) else {
                continue;
            };
            let su = sp.demultiply();
            let (sa, sr, sg, sb) = (
                f32::from(su.alpha()) / 255.0,
                f32::from(su.red()) / 255.0,
                f32::from(su.green()) / 255.0,
                f32::from(su.blue()) / 255.0,
            );
            if sa <= 0.0 {
                continue;
            }
            let d = dst.pixels()[i].demultiply();
            let (dr, dg, db) = (
                f32::from(d.red()) / 255.0,
                f32::from(d.green()) / 255.0,
                f32::from(d.blue()) / 255.0,
            );
            let chan = |sv: f32, dv: f32| -> f32 {
                let cb = if tag == 0 {
                    sv
                } else {
                    blend_channel(tag, sv, dv)
                };
                (dv * (1.0 - sa) + cb * sa) * 255.0
            };
            if let Some(c) = tiny_skia::PremultipliedColorU8::from_rgba(
                (chan(sr, dr) * sa).min(255.0) as u8,
                (chan(sg, dg) * sa).min(255.0) as u8,
                (chan(sb, db) * sa).min(255.0) as u8,
                ((sa + f32::from(d.alpha()) / 255.0 * (1.0 - sa)) * 255.0).min(255.0) as u8,
            ) {
                dst.pixels_mut()[i] = c;
            }
        }
    }
}

/// R-186 — `text-emphasis` : une marque par glyphe au
/// milieu de son avance, au-dessus (over) ou dessous de
/// la ligne. Kind : 0 dot, 1 circle, 2 double-circle,
/// 3 triangle, 4 sesame ; bit3 = `open`.
#[allow(
    clippy::too_many_arguments,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
fn paint_emphasis(
    pixmap: &mut Pixmap,
    origin: (f32, f32),
    run: &std::sync::Arc<ShapedRun>,
    size_px: f32,
    kind: u8,
    color: Color,
    over: bool,
    cur: Transform,
    clip: Option<&Mask>,
) {
    let mark = size_px * 0.5;
    let y = if over {
        origin.1 - size_px * 1.15
    } else {
        origin.1 + size_px * 0.35
    };
    let open = kind & 8 != 0;
    let shape = kind & 7;
    let c = tiny_skia::Color::from_rgba8(color.r, color.g, color.b, color.a);
    let mut pen = origin.0;
    for g in &run.glyphs {
        let cx = pen + g.advance / 2.0;
        pen += g.advance;
        let m = mark * 0.5;
        let mut pb = tiny_skia::PathBuilder::new();
        match shape {
            1 | 2 => {
                pb.push_circle(cx, y, m / 2.0);
                if shape == 2 {
                    pb.push_circle(cx, y, m / 4.0);
                }
            }
            3 => {
                pb.move_to(cx, y - m / 2.0);
                pb.line_to(cx + m / 2.0, y + m / 2.0);
                pb.line_to(cx - m / 2.0, y + m / 2.0);
                pb.close();
            }
            4 => {
                pb.move_to(cx, y - m / 2.0);
                pb.line_to(cx + m / 3.0, y);
                pb.line_to(cx, y + m / 2.0);
                pb.line_to(cx - m / 3.0, y);
                pb.close();
            }
            _ => {
                pb.push_circle(cx, y, m / 2.0);
            }
        }
        let Some(path) = pb.finish() else { continue };
        let mut paint = tiny_skia::Paint::default();
        paint.set_color(c);
        if open {
            let stroke = tiny_skia::Stroke {
                width: 1.0,
                ..Default::default()
            };
            pixmap.stroke_path(&path, &paint, &stroke, cur, clip);
        } else {
            pixmap.fill_path(&path, &paint, tiny_skia::FillRule::Winding, cur, clip);
        }
    }
}

/// R-184 — un canal de `background-blend-mode` (formules
/// css-compositing-1 § blending). `s`/`d` ∈ [0,1]. Modes
/// séparables portés ; `hue`/`saturation`/`color`/
/// `luminosity` (non séparables — composés couleur
/// entière) → repli `s` documenté.
fn blend_channel(tag: u8, s: f32, d: f32) -> f32 {
    let clamp01 = |v: f32| v.clamp(0.0, 1.0);
    match tag {
        1 => s * d,         // multiply
        2 => s + d - s * d, // screen
        3 => clamp01(if d <= 0.5 {
            2.0 * s * d
        } else {
            1.0 - 2.0 * (1.0 - s) * (1.0 - d)
        }), // overlay
        4 => s.min(d),      // darken
        5 => s.max(d),      // lighten
        6 => clamp01(if s >= 1.0 {
            1.0
        } else if s <= 0.0 {
            d
        } else {
            d / (1.0 - s)
        }), // color-dodge
        7 => clamp01(if s <= 0.0 { 0.0 } else { 1.0 - (1.0 - d) / s }), // color-burn
        8 => clamp01(if s <= 0.5 {
            2.0 * s * d
        } else {
            1.0 - 2.0 * (1.0 - s) * (1.0 - d)
        }), // hard-light
        9 => {
            // soft-light
            let g = if d <= 0.25 {
                ((16.0 * d - 12.0) * d + 4.0) * d
            } else {
                d.sqrt()
            };
            clamp01(if s <= 0.5 {
                d - (1.0 - 2.0 * s) * d * (1.0 - d)
            } else {
                d + (2.0 * s - 1.0) * (g - d)
            })
        }
        10 => (d - s).abs(),                // difference
        11 => clamp01(d + s - 2.0 * s * d), // exclusion
        _ => s,                             // 12-15 non séparables : repli
    }
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::too_many_arguments,
    clippy::too_many_lines,
    clippy::cast_precision_loss
)]
/// Index source `rel` dans une tuile de `tile` px sur une
/// zone de `zone` px — `space` = N tuiles pleines + gap
/// uniforme (non-peint dans les gaps → `None`), `round` =
/// N tuiles étirées à `zone/N` (échantillonnage à l'échelle).
/// Mode 0 `repeat`, 1 `no-repeat`.
fn tile_sample(rel: f32, tile: f32, zone: f32, mode: u8) -> Option<f32> {
    match mode {
        1 => (0.0..tile).contains(&rel).then_some(rel),
        2 | 3 => {
            if tile <= 0.0 || zone <= 0.0 {
                return None;
            }
            let n = ((zone / tile).floor() as i32).max(1) as f32;
            let pitch = zone / n;
            let local = rel.rem_euclid(pitch);
            if mode == 2 {
                (local < tile).then_some(local)
            } else {
                Some(local * tile / pitch)
            }
        }
        _ => Some(rel.rem_euclid(tile)),
    }
}

#[allow(
    clippy::too_many_arguments,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap
)]
fn paint_tiled(
    pixmap: &mut Pixmap,
    rect: &explorer_layout::Rect,
    img: &crate::DecodedImage,
    repeat: (u8, u8),
    origin: (f32, f32),
    cur: Transform,
    clip: Option<&Mask>,
    blend: u8,
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
        let Some(sy) =
            tile_sample(rel_y as f32, ih as f32, (dy1 - dy0) as f32, repeat.1).map(|v| v as i32)
        else {
            continue;
        };
        for dx in dx0..dx1 {
            let rel_x = dx - ox;
            let Some(sx) = tile_sample(rel_x as f32, iw as f32, (dx1 - dx0) as f32, repeat.0)
                .map(|v| v as i32)
            else {
                continue;
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
            // R-184 — `background-blend-mode` : la couleur
            // mixée `Cb = blend(src, dst)` remplace `src`
            // dans le source-over habituel.
            let blend = |sv: u32, dv: u8| -> u8 {
                if blend == 0 {
                    return ((sv * sa + u32::from(dv) * inv + 127) / 255) as u8;
                }
                let cb = blend_channel(blend, sv as f32 / 255.0, f32::from(dv) / 255.0) * 255.0;
                ((cb * sa as f32 + f32::from(dv) * inv as f32 + 127.5) / 255.0) as u8
            };
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

/// R-167 — chemin d'un rectangle aux 4 coins arrondis
/// (cubiques Bézier κ≈0.5523 par coin, rayons 0 = coin
/// droit ; approximation circulaire — l'elliptique `h/v`
/// n'est pas encore supporté).
/// Un coin de `rounded_rect_path` (R-270) : de l'entrée
/// `S = C + r·u` à la sortie `E = C + r·v` (`u`,`v` =
/// directions depuis le centre `C`, dans l'ordre du
/// parcours). `o` = coin externe (pour `square`) et `k`
/// l'exposant superellipse du coin : 1 = cubique ronde,
/// ≥8 ≈ `square` (sommet), ≤-16 ≈ `notch` (coupure droite
/// en `C`), sinon arc superellipse échantillonné — `e=2^k`,
/// `p=2^{1-k}` : `bevel`(0) trace la corde, `squircle`(2)
/// l'aplatie, `scoop`(-1) la creuse (concave). Le
/// tracé paramétrique `cos^p`/`sin^p` suit exactement la
/// superellipse pour tout `k` réel.
#[allow(clippy::many_single_char_names, clippy::cast_precision_loss)]
fn corner_arc(
    pb: &mut PathBuilder,
    o: (f32, f32),
    c: (f32, f32),
    u: (f32, f32),
    v: (f32, f32),
    r: f32,
    k: f32,
) {
    if r <= 0.0 {
        return;
    }
    if k >= 8.0 {
        // `square` (ou exposant très grand) : sommet franc.
        pb.line_to(o.0, o.1);
        return;
    }
    if k <= -16.0 {
        // `notch` : deux segments par le centre intérieur.
        pb.line_to(c.0, c.1);
        pb.line_to(c.0 + r * v.0, c.1 + r * v.1);
        return;
    }
    if (k - 1.0).abs() < 1e-6 {
        // `round` : cubique Bézier historique (constante
        // classique du quart de cercle).
        const CK: f32 = 0.552_284_8;
        let s = (c.0 + r * u.0, c.1 + r * u.1);
        let e = (c.0 + r * v.0, c.1 + r * v.1);
        pb.cubic_to(
            s.0 + CK * (o.0 - s.0),
            s.1 + CK * (o.1 - s.1),
            e.0 + CK * (o.0 - e.0),
            e.1 + CK * (o.1 - e.1),
            e.0,
            e.1,
        );
        return;
    }
    // Arc superellipse générique — 9 cordes + extrémité
    // exacte (`cos(π/2)` en f32 < 0 → `powf` NaN sinon).
    let p = 2.0_f32.powf(1.0 - k);
    for i in 1..10 {
        let t = i as f32 * std::f32::consts::FRAC_PI_2 / 10.0;
        let cu = t.cos().max(0.0).powf(p) * r;
        let sv = t.sin().max(0.0).powf(p) * r;
        pb.line_to(c.0 + cu * u.0 + sv * v.0, c.1 + cu * u.1 + sv * v.1);
    }
    pb.line_to(c.0 + r * v.0, c.1 + r * v.1);
}

fn rounded_rect_path(
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    radii: &[f32; 4],
    corners: &[f32; 4],
) -> Option<tiny_skia::Path> {
    let [rtl, rtr, rbr, rbl] = *radii;
    let [ktl, ktr, kbr, kbl] = *corners;
    let mut pb = PathBuilder::new();
    pb.move_to(x0 + rtl, y0);
    pb.line_to(x1 - rtr, y0);
    corner_arc(
        &mut pb,
        (x1, y0),
        (x1 - rtr, y0 + rtr),
        (0.0, -1.0),
        (1.0, 0.0),
        rtr,
        ktr,
    );
    pb.line_to(x1, y1 - rbr);
    corner_arc(
        &mut pb,
        (x1, y1),
        (x1 - rbr, y1 - rbr),
        (1.0, 0.0),
        (0.0, 1.0),
        rbr,
        kbr,
    );
    pb.line_to(x0 + rbl, y1);
    corner_arc(
        &mut pb,
        (x0, y1),
        (x0 + rbl, y1 - rbl),
        (-1.0, 0.0),
        (0.0, 1.0),
        rbl,
        kbl,
    );
    pb.line_to(x0, y0 + rtl);
    corner_arc(
        &mut pb,
        (x0, y0),
        (x0 + rtl, y0 + rtl),
        (-1.0, 0.0),
        (0.0, -1.0),
        rtl,
        ktl,
    );
    pb.close();
    pb.finish()
}

/// R-166 — demi-teinte sombre d'une couleur de bordure
/// (moitié vers le noir) pour les styles 3D.
fn shade_dark(c: Color) -> Color {
    Color {
        r: c.r / 2,
        g: c.g / 2,
        b: c.b / 2,
        a: c.a,
    }
}

/// R-166 — demi-teinte claire (moitié vers le blanc) —
/// le mix blanc donne le relief aussi sur teinte foncée.
fn shade_light(c: Color) -> Color {
    Color {
        r: u8::midpoint(c.r, 255),
        g: u8::midpoint(c.g, 255),
        b: u8::midpoint(c.b, 255),
        a: c.a,
    }
}

/// Bandes d'un côté : fractions [f0,f1) le long de
/// l'épaisseur vers l'intérieur + couleur de la bande.
/// `inset` = haut/gauche foncés ; `outset` l'inverse ;
/// `groove`/`ridge` = bande externe/interne échangées ;
/// `double` = deux liserés pleins séparés d'un tiers vide
/// (approximation du bevel Win95, R-166).
fn side_bands(side: usize, c: Color, s: BorderStyle) -> Vec<(f32, f32, Color)> {
    let tl = matches!(side, 0 | 3);
    match s {
        BorderStyle::Inset => {
            vec![(0.0, 1.0, if tl { shade_dark(c) } else { shade_light(c) })]
        }
        BorderStyle::Outset => {
            vec![(0.0, 1.0, if tl { shade_light(c) } else { shade_dark(c) })]
        }
        BorderStyle::Groove => {
            if tl {
                vec![(0.0, 0.5, shade_dark(c)), (0.5, 1.0, shade_light(c))]
            } else {
                vec![(0.0, 0.5, shade_light(c)), (0.5, 1.0, shade_dark(c))]
            }
        }
        BorderStyle::Ridge => {
            if tl {
                vec![(0.0, 0.5, shade_light(c)), (0.5, 1.0, shade_dark(c))]
            } else {
                vec![(0.0, 0.5, shade_dark(c)), (0.5, 1.0, shade_light(c))]
            }
        }
        BorderStyle::Double => vec![(0.0, 1.0 / 3.0, c), (2.0 / 3.0, 1.0, c)],
        _ => vec![(0.0, 1.0, c)],
    }
}

/// Bordure : 4 côtés (pattern dashed/dotted reporté —
/// documenté en `DisplayItem`). Quand les côtés peints ont
/// des couleurs différentes, chaque côté est le **trapèze**
/// entre l'arête externe et l'arête de la padding box — le
/// coin se partage sur la diagonale coin-externe→coin-interne
/// (comportement CSS des jointures) ; c'est ce qui dessine
/// les vrais triangles `border-*` à `height:0` (Acid2 nose).
/// Couleur uniforme → anneau par rectangles pixel-snapés
/// (arêtes dures, pas de couture AA sur la diagonale).
/// R-166 : styles 3D et `double` → bandes `side_bands` —
/// demi-teintes par côté (`inset`/`outset`) ou en moitiés
/// (`groove`/`ridge`) / liserés en tiers (`double`).
#[allow(clippy::too_many_arguments)]
fn paint_border(
    pixmap: &mut Pixmap,
    rect: &explorer_layout::Rect,
    widths: &[f32; 4],
    colors: &[Color; 4],
    styles: [BorderStyle; 4],
    radii: &[f32; 4],
    corners: &[f32; 4],
    dpi: Transform,
    clip: Option<&Mask>,
) {
    // R-167 — coins arrondis : anneau unique externe−interne
    // (EvenOdd) pour une bordure uniforme pleine — le cas
    // commun `border-radius + solid` (boutons, navs). Les
    // variantes de style ou de couleur retombent sur la
    // peinture plate par bandes (approximation documentée).
    if radii.iter().any(|r| *r > 0.0)
        && styles
            .iter()
            .all(|s| *s == BorderStyle::Solid || *s == BorderStyle::None)
        && colors.iter().all(|c| *c == colors[0])
    {
        let (ox0, oy0) = (rect.x, rect.y);
        let (ox1, oy1) = (rect.x + rect.width, rect.y + rect.height);
        let ix0 = ox0 + widths[3];
        let iy0 = oy0 + widths[0];
        let ix1 = ox1 - widths[1];
        let iy1 = oy1 - widths[2];
        let inner_r: [f32; 4] = [
            (radii[0] - widths[0].max(widths[3])).max(0.0),
            (radii[1] - widths[0].max(widths[1])).max(0.0),
            (radii[2] - widths[2].max(widths[1])).max(0.0),
            (radii[3] - widths[2].max(widths[3])).max(0.0),
        ];
        let mut pb = PathBuilder::new();
        if let Some(outer) = rounded_rect_path(ox0, oy0, ox1, oy1, radii, corners) {
            pb.push_path(&outer);
        }
        // R-270 — le contour interne garde l'exposant du
        // coin (css-borders-4 : la forme se propage à la
        // padding box, seul le rayon se réduit).
        if let Some(inner) = rounded_rect_path(ix0, iy0, ix1, iy1, &inner_r, corners) {
            pb.push_path(&inner);
        }
        if let Some(ring) = pb.finish() {
            let mut paint = Paint {
                anti_alias: true,
                ..Paint::default()
            };
            paint.set_color(skia_color(colors[0]));
            pixmap.fill_path(&ring, &paint, FillRule::EvenOdd, dpi, clip);
        }
        return;
    }
    let painted = |i: usize| styles[i] != BorderStyle::None && widths[i] > 0.0;
    // R-166 — styles 3D / multi-bandes : la couleur réelle
    // d'une bande est dérivée (demi-teintes) — le chemin
    // « rect uniforme » ne s'applique qu'aux styles plats.
    let has_bands = styles.iter().any(|s| {
        matches!(
            s,
            BorderStyle::Double
                | BorderStyle::Inset
                | BorderStyle::Outset
                | BorderStyle::Groove
                | BorderStyle::Ridge
        )
    });
    let uniform = !has_bands
        && (0..4)
            .filter(|&i| painted(i))
            .map(|i| colors[i])
            .collect::<Vec<_>>()
            .windows(2)
            .all(|w| w[0] == w[1]);
    for (side, (bw, (c, s))) in widths
        .iter()
        .zip(colors.iter().zip(styles.iter()))
        .enumerate()
    {
        if *s == BorderStyle::None || *bw <= 0.0 {
            continue;
        }
        // Un côté sans voisin peint n'a pas de coin mitré :
        // bande rectangulaire (shortcut `fill_guarded`).
        let rect_shortcut = uniform || !(painted((side + 3) % 4) || painted((side + 1) % 4));
        for (f0, f1, band_color) in side_bands(side, *c, *s) {
            paint_border_band(
                pixmap,
                rect,
                widths,
                side,
                *bw,
                f0,
                f1,
                band_color,
                rect_shortcut,
                dpi,
                clip,
            );
        }
    }
}

/// R-166 — une bande d'un côté de bordure : rect
/// pixel-snappé (`fill_guarded`, margin-collapse-003)
/// quand le côté n'a pas de voisin peint ou que la
/// bordure est uniforme ; sinon trapèze entre les coins
/// externes (fraction f0) et internes (fraction f1) de la
/// padding box — les sommets tournent dans le sens horaire.
#[allow(clippy::too_many_arguments)]
fn paint_border_band(
    pixmap: &mut Pixmap,
    rect: &explorer_layout::Rect,
    widths: &[f32; 4],
    side: usize,
    bw: f32,
    f0: f32,
    f1: f32,
    band_color: Color,
    rect_shortcut: bool,
    dpi: Transform,
    clip: Option<&Mask>,
) {
    let mut paint = Paint::default();
    paint.set_color(skia_color(band_color));
    if rect_shortcut {
        let r = match side {
            0 => Rect::from_xywh(rect.x, rect.y + f0 * bw, rect.width, (f1 - f0) * bw),
            1 => Rect::from_xywh(
                rect.x + rect.width - f1 * bw,
                rect.y,
                (f1 - f0) * bw,
                rect.height,
            ),
            2 => Rect::from_xywh(
                rect.x,
                rect.y + rect.height - f1 * bw,
                rect.width,
                (f1 - f0) * bw,
            ),
            _ => Rect::from_xywh(rect.x + f0 * bw, rect.y, (f1 - f0) * bw, rect.height),
        };
        if let Some(r) = r {
            fill_guarded(pixmap, &r, &paint, dpi, clip);
        }
        return;
    }
    let (x0, y0) = (rect.x, rect.y);
    let (x1, y1) = (rect.x + rect.width, rect.y + rect.height);
    // Coins internes (padding box). `widths` = [t, r, b, l].
    let itl = (x0 + widths[3], y0 + widths[0]);
    let itr = (x1 - widths[1], y0 + widths[0]);
    let ibr = (x1 - widths[1], y1 - widths[2]);
    let ibl = (x0 + widths[3], y1 - widths[2]);
    let lerp_pt = |o: (f32, f32), i: (f32, f32), f: f32| -> (f32, f32) {
        (o.0 + (i.0 - o.0) * f, o.1 + (i.1 - o.1) * f)
    };
    let quad: [(f32, f32); 4] = match side {
        0 => [
            lerp_pt((x0, y0), itl, f0),
            lerp_pt((x1, y0), itr, f0),
            lerp_pt((x1, y0), itr, f1),
            lerp_pt((x0, y0), itl, f1),
        ],
        1 => [
            lerp_pt((x1, y0), itr, f0),
            lerp_pt((x1, y1), ibr, f0),
            lerp_pt((x1, y1), ibr, f1),
            lerp_pt((x1, y0), itr, f1),
        ],
        2 => [
            lerp_pt((x1, y1), ibr, f0),
            lerp_pt((x0, y1), ibl, f0),
            lerp_pt((x0, y1), ibl, f1),
            lerp_pt((x1, y1), ibr, f1),
        ],
        _ => [
            lerp_pt((x0, y1), ibl, f0),
            lerp_pt((x0, y0), itl, f0),
            lerp_pt((x0, y0), itl, f1),
            lerp_pt((x0, y1), ibl, f1),
        ],
    };
    let mut pb = PathBuilder::new();
    pb.move_to(quad[0].0, quad[0].1);
    for p in &quad[1..] {
        pb.line_to(p.0, p.1);
    }
    pb.close();
    if let Some(path) = pb.finish() {
        // La boîte englobante du quad ⊆ `rect` → le garde
        // de domaine device s'y applique tel quel.
        let domain = Rect::from_ltrb(-SAFE_COORD, -SAFE_COORD, SAFE_COORD, SAFE_COORD);
        let outer = Rect::from_xywh(rect.x, rect.y, rect.width, rect.height);
        if let (Some(d), Some(o)) = (domain, outer) {
            if device_bbox(&o, dpi).is_some_and(|b| d.intersect(&b).is_some()) {
                pixmap.fill_path(&path, &paint, FillRule::Winding, dpi, clip);
            }
        }
    }
}
