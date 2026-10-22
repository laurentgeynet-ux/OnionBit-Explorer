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

use crate::display::{BorderStyle, DisplayItem, DisplayList, MarkerKind};
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
            return false;
        };
        let ok = (|| {
            let mut stream = writer.stream_writer_with_size(PNG_IDAT_TARGET).ok()?;
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
                stream.write_all(&band).ok()?;
            }
            stream.finish().ok()?;
            Some(())
        })();
        ok.is_some() && writer.finish().is_ok()
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
            pixmap, gc, face, face_key, g, pen, oy, gscale, size_px, cur, &paint, clip,
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
    if let Some(path) = gc
        .glyph_path(face, face_key, g.id)
        .and_then(|p| p.transform(tf))
    {
        pixmap.fill_path(&path, paint, FillRule::Winding, Transform::default(), clip);
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
    replay_items(&mut pixmap, &list.items, dpi, fonts, gc, None);
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
    let y_css = y_css.max(0.0);
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
    let bounds = crate::display::resolved_bounds(&list.items);
    gc.sync(fonts);
    replay_items(
        &mut pixmap,
        &list.items,
        base,
        fonts,
        gc,
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

/// Rejoue les items dans `pixmap` — boucle de peinture
/// partagée par [`rasterize`] et [`rasterize_dirty`].
///
/// `filter` : quand `Some((bounds, window))`, un item de
/// dessin dont la bounds résolue (`resolved_bounds`, même
/// sémantique de pile) n'intersecte pas `window` est sauté ;
/// les items d'état (`Push*/Pop*`) sont toujours rejoués —
/// la pile doit refléter l'emplacement de chaque item.
fn replay_items(
    pixmap: &mut Pixmap,
    items: &[DisplayItem],
    dpi: Transform,
    fonts: &dyn FontSource,
    gc: &mut GlyphCache,
    filter: Option<(&[Option<explorer_layout::Rect>], explorer_layout::Rect)>,
) {
    let mut clips: Vec<Mask> = Vec::new();
    // Pile des `transform` CSS : `cur` = dpi · produit des
    // matrices actives — recomposé à chaque push/pop (peu de
    // niveaux en pratique).
    let mut tfs: Vec<Transform> = Vec::new();
    let mut cur = dpi;
    for (idx, item) in items.iter().enumerate() {
        match item {
            DisplayItem::PushClip(r) => {
                if let Some(mut mask) = Mask::new(pixmap.width(), pixmap.height()) {
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
            _ => {
                // Fenêtre sale : item dont la bounds résolue
                // est hors fenêtre → sauté (son encre ne peut
                // pas y toucher). `None` bounds = conservateur.
                let draw = filter.is_none_or(|(bounds, win)| {
                    bounds
                        .get(idx)
                        .copied()
                        .flatten()
                        .is_none_or(|b| rects_overlap(b, win))
                });
                if draw {
                    paint_item(pixmap, item, cur, fonts, gc, clips.last());
                }
            }
        }
    }
}

/// Intersection non vide de deux rects px CSS — filtre
/// « item × fenêtre » de `replay_items` (R-37).
fn rects_overlap(a: explorer_layout::Rect, b: explorer_layout::Rect) -> bool {
    a.x < b.x + b.width && b.x < a.x + a.width && a.y < b.y + b.height && b.y < a.y + a.height
}

/// Peint un item de dessin (les clips sont gérés par
/// `rasterize` directement).
fn paint_item(
    pixmap: &mut Pixmap,
    item: &DisplayItem,
    dpi: Transform,
    fonts: &dyn FontSource,
    gc: &mut GlyphCache,
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
        } => {
            paint_tiled(pixmap, rect, img, *repeat, *origin, dpi, clip);
        }
        DisplayItem::MaskedFill { rect, color, mask } => {
            paint_masked_fill(pixmap, rect, *color, mask, dpi, clip);
        }
        DisplayItem::Marker { rect, kind, color } => {
            paint_marker(pixmap, rect, *kind, *color, dpi, clip);
        }
        DisplayItem::PushClip(_)
        | DisplayItem::PopClip
        | DisplayItem::PushTransform(_)
        | DisplayItem::PopTransform => {}
    }
}

/// Marqueur `list-item` (issue #9) — forme primitive inscrite
/// dans `rect` : `disc`/`square` pleins, `circle` en anneau.
/// `fill_guarded` couvre le carré ; le disque passe par un
/// chemin ovale (même garde de domaine via `device_bbox`).
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
/// zone device de `rect` — `mask-size`/`mask-position` hors
/// portée (le motif icône dimensionne la boîte à l'icône, la
/// zone est donc déjà la bonne). Un `clip` (`overflow`)
/// actif est composé par `min` des couvertures ≈ intersection
/// (les masks sont quasi binaires dans le motif).
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]
fn paint_masked_fill(
    pixmap: &mut Pixmap,
    rect: &explorer_layout::Rect,
    color: Color,
    img: &crate::DecodedImage,
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
    let (x0, y0) = (zone.left().max(0.0) as i32, zone.top().max(0.0) as i32);
    let (x1, y1) = (zone.right().ceil() as i32, zone.bottom().ceil() as i32);
    let w = pw as usize;
    let data = m.data_mut();
    for py in y0..y1 {
        let fy = (py as f32 - phys.top()) / phys.height();
        let sy = ((fy.max(0.0) * ih as f32) as usize).min(ih as usize - 1);
        for px in x0..x1 {
            let fx = (px as f32 - phys.left()) / phys.width();
            let sx = ((fx.max(0.0) * iw as f32) as usize).min(iw as usize - 1);
            data[py as usize * w + px as usize] = img.rgba[(sy * iw as usize + sx) * 4 + 3];
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
/// documenté en `DisplayItem`). Quand les côtés peints ont
/// des couleurs différentes, chaque côté est le **trapèze**
/// entre l'arête externe et l'arête de la padding box — le
/// coin se partage sur la diagonale coin-externe→coin-interne
/// (comportement CSS des jointures) ; c'est ce qui dessine
/// les vrais triangles `border-*` à `height:0` (Acid2 nose).
/// Couleur uniforme → anneau par rectangles pixel-snapés
/// (arêtes dures, pas de couture AA sur la diagonale).
fn paint_border(
    pixmap: &mut Pixmap,
    rect: &explorer_layout::Rect,
    widths: &[f32; 4],
    colors: &[Color; 4],
    styles: [BorderStyle; 4],
    dpi: Transform,
    clip: Option<&Mask>,
) {
    let painted = |i: usize| styles[i] != BorderStyle::None && widths[i] > 0.0;
    let uniform = (0..4)
        .filter(|&i| painted(i))
        .map(|i| colors[i])
        .collect::<Vec<_>>()
        .windows(2)
        .all(|w| w[0] == w[1]);
    let (x0, y0) = (rect.x, rect.y);
    let (x1, y1) = (rect.x + rect.width, rect.y + rect.height);
    // Coins internes (padding box). `widths` = [t, r, b, l].
    let itl = (x0 + widths[3], y0 + widths[0]);
    let itr = (x1 - widths[1], y0 + widths[0]);
    let ibr = (x1 - widths[1], y1 - widths[2]);
    let ibl = (x0 + widths[3], y1 - widths[2]);
    for (side, (bw, (c, s))) in widths
        .iter()
        .zip(colors.iter().zip(styles.iter()))
        .enumerate()
    {
        if *s == BorderStyle::None || *bw <= 0.0 {
            continue;
        }
        let mut paint = Paint::default();
        paint.set_color(skia_color(*c));
        // Un côté sans voisin peint n'a pas de coin mitré : sa
        // bande est un rectangle — passer par `fill_guarded`
        // (pixel-snap des fonds, comme un background ; sinon un
        // reftest à `border-top/bottom` non uniformes gardait
        // des arêtes AA d'un pixel face aux fills snappés —
        // margin-collapse-003).
        let neighbors_painted = painted((side + 3) % 4) || painted((side + 1) % 4);
        if uniform || !neighbors_painted {
            let r = match side {
                0 => Rect::from_xywh(rect.x, rect.y, rect.width, *bw),
                1 => Rect::from_xywh(rect.x + rect.width - *bw, rect.y, *bw, rect.height),
                2 => Rect::from_xywh(rect.x, rect.y + rect.height - *bw, rect.width, *bw),
                _ => Rect::from_xywh(rect.x, rect.y, *bw, rect.height),
            };
            if let Some(r) = r {
                fill_guarded(pixmap, &r, &paint, dpi, clip);
            }
            continue;
        }
        // Trapèze du côté : arête externe + arête interne de
        // la padding box. Ordre des sommets = sens horaire.
        let quad: [(f32, f32); 4] = match side {
            0 => [(x0, y0), (x1, y0), itr, itl],
            1 => [(x1, y0), (x1, y1), ibr, itr],
            2 => [(x1, y1), (x0, y1), ibl, ibr],
            _ => [(x0, y1), (x0, y0), itl, ibl],
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
}
