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
    let bounds = crate::display::resolved_bounds(&list.items);
    gc.sync(fonts);
    replay_items(
        &mut pixmap,
        &list.items,
        base,
        fonts,
        gc,
        0,
        Some((&bounds, win_css)),
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
    clippy::cast_sign_loss
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
    let mut clips: Vec<Mask> = Vec::new();
    // R-M5 — groupes hors-écran : `PushFilter`/`PushBlend`/
    // `PushMask` poussent un tampon ; les items suivants s'y
    // peignent (`filtered.last_mut()` = cible courante) et le
    // `Pop*` correspondant le composite selon `GroupKind`.
    let mut filtered: Vec<(Pixmap, GroupKind)> = Vec::new();
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
            // R-M9 — clip arrondi (`overflow` + `border-radius`) :
            // même grille de pixels que `PushClip`, le chemin de
            // masque suit les rayons (`rounded_rect_path`).
            DisplayItem::PushClipRounded { rect: r, radii } => {
                if let Some(mut mask) = Mask::new(pixmap.width(), pixmap.height()) {
                    if let Some(phys) = Rect::from_xywh(r.x, r.y, r.width, r.height)
                        .and_then(|rr| rr.transform(cur))
                    {
                        let dom = Rect::from_ltrb(-SAFE_COORD, -SAFE_COORD, SAFE_COORD, SAFE_COORD);
                        if let Some(clamped) = dom.and_then(|d| phys.intersect(&d)) {
                            if let Some(path) = rounded_rect_path(
                                clamped.left(),
                                clamped.top(),
                                clamped.right(),
                                clamped.bottom(),
                                radii,
                            ) {
                                mask.fill_path(
                                    &path,
                                    FillRule::Winding,
                                    true,
                                    Transform::default(),
                                );
                            }
                        }
                    }
                    clips.push(mask);
                }
            }
            // R-187 — `clip-path:polygon` : mask rempli par
            // `fill_path` sur le chemin du polygone — même
            // pile `clips` que `PushClip`/`PushClipRounded`.
            DisplayItem::PushClipPath { points } => {
                if let Some(mut mask) = Mask::new(pixmap.width(), pixmap.height()) {
                    let mut pb = tiny_skia::PathBuilder::new();
                    let map = |x: f32, y: f32| -> (f32, f32) {
                        let mut pt = tiny_skia::Point::from_xy(x, y);
                        cur.map_point(&mut pt);
                        (pt.x, pt.y)
                    };
                    if let Some((x, y)) = points.first() {
                        let (x0, y0) = map(*x, *y);
                        pb.move_to(x0, y0);
                        for (x, y) in &points[1..] {
                            let (tx, ty) = map(*x, *y);
                            pb.line_to(tx, ty);
                        }
                        pb.close();
                    }
                    if let Some(path) = pb.finish() {
                        mask.fill_path(&path, FillRule::Winding, true, Transform::default());
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
            DisplayItem::PushFilter(ops) => {
                if let Some(buf) = Pixmap::new(pixmap.width(), pixmap.height()) {
                    filtered.push((buf, GroupKind::Ops(ops.clone())));
                }
            }
            // R-185 — `mix-blend-mode`/`isolation` : groupe
            // hors-écran fondu au pop (tag `BLEND_MODES`,
            // 0 = `isolation` isolant).
            DisplayItem::PushBlend(tag) => {
                if let Some(buf) = Pixmap::new(pixmap.width(), pixmap.height()) {
                    filtered.push((buf, GroupKind::Blend(*tag)));
                }
            }
            // R-252 — `mask-image` de groupe : le tampon se
            // composite au pop modulé par l'alpha du bitmap
            // (tuile `dst`/`repeat` résolus à l'émission,
            // aire `rect` = border box).
            DisplayItem::PushMask { rect, layers } => {
                if let Some(buf) = Pixmap::new(pixmap.width(), pixmap.height()) {
                    filtered.push((
                        buf,
                        GroupKind::Mask {
                            rect: *rect,
                            layers: layers.clone(),
                        },
                    ));
                }
            }
            // Pop générique : le kind porté par le groupe fait
            // foi — `PopFilter`/`PopBlend`/`PopMask` sont
            // interchangeables à la raster (l'émission garantit
            // l'appariement ; un interleave fautif dégrade en
            // composite du kind réel, jamais un panic).
            DisplayItem::PopFilter | DisplayItem::PopBlend | DisplayItem::PopMask => {
                if let Some((mut buf, kind)) = filtered.pop() {
                    match kind {
                        GroupKind::Ops(ops) => {
                            apply_filter_ops(&mut buf, &ops);
                            let dst = filtered.last_mut().map_or(&mut *pixmap, |(b, _)| b);
                            dst.draw_pixmap(
                                0,
                                0,
                                buf.as_ref(),
                                &tiny_skia::PixmapPaint::default(),
                                Transform::default(),
                                clips.last(),
                            );
                        }
                        // `blend_channel` par canal quand le
                        // tag > 0, source-over sinon.
                        GroupKind::Blend(tag) => {
                            let dst = filtered.last_mut().map_or(&mut *pixmap, |(b, _)| b);
                            composite_blend(dst, &buf, tag);
                        }
                        GroupKind::Mask { rect, layers } => {
                            let cov = group_mask_coverage(
                                pixmap.width(),
                                pixmap.height(),
                                &rect,
                                &layers,
                                cur,
                                clips.last(),
                            );
                            let dst = filtered.last_mut().map_or(&mut *pixmap, |(b, _)| b);
                            dst.draw_pixmap(
                                0,
                                0,
                                buf.as_ref(),
                                &tiny_skia::PixmapPaint::default(),
                                Transform::default(),
                                cov.as_ref(),
                            );
                        }
                    }
                }
            }
            // R-M9b — `backdrop-filter` : la région déjà
            // peinte sous `rect` reçoit les ops, le contenu
            // se peint ensuite par-dessus.
            DisplayItem::BackdropFilter { rect: r, ops } => {
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let (bw, bh) = (r.width.ceil() as u32, r.height.ceil() as u32);
                if bw > 0 && bh > 0 {
                    if let Some(mut buf) = Pixmap::new(bw, bh) {
                        let src = filtered.last().map_or(pixmap.as_ref(), |(b, _)| b.as_ref());
                        buf.draw_pixmap(
                            0,
                            0,
                            src,
                            &tiny_skia::PixmapPaint::default(),
                            Transform::from_translate(-r.x, -r.y),
                            None,
                        );
                        apply_filter_ops(&mut buf, ops);
                        let dst = filtered.last_mut().map_or(&mut *pixmap, |(b, _)| b);
                        dst.draw_pixmap(
                            r.x.round() as i32,
                            r.y.round() as i32,
                            buf.as_ref(),
                            &tiny_skia::PixmapPaint::default(),
                            Transform::default(),
                            clips.last(),
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
                    let dst = filtered.last_mut().map_or(&mut *pixmap, |(b, _)| b);
                    paint_item(dst, item, cur, fonts, gc, clips.last());
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
        _ => fill_rect_item(pixmap, rect, color, &[0.0; 4], dpi, clip),
    }
}

/// R-167 — un `FillRect` : rayon > 0 → fill du chemin
/// arrondi ; sinon rect pixel-snappé (`fill_guarded`).
fn fill_rect_item(
    pixmap: &mut Pixmap,
    rect: &explorer_layout::Rect,
    color: Color,
    radii: &[f32; 4],
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
        if let Some(path) = rounded_rect_path(x0, y0, x1, y1, &rr) {
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
        DisplayItem::FillRect { rect, color, radii } => {
            fill_rect_item(pixmap, rect, *color, radii, dpi, clip);
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
        } => {
            paint_border(pixmap, rect, widths, colors, *styles, radii, dpi, clip);
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
/// prémultipliés démultipliés au passage.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)]
fn composite_blend(dst: &mut Pixmap, src: &Pixmap, tag: u8) {
    let (w, h) = (src.width().min(dst.width()), src.height().min(dst.height()));
    for y in 0..h as usize {
        for x in 0..w as usize {
            let i = y * dst.width() as usize + x;
            let Some(sp) = src.pixel(x as u32, y as u32) else {
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
fn rounded_rect_path(
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    radii: &[f32; 4],
) -> Option<tiny_skia::Path> {
    const K: f32 = 0.552_284_8;
    let [rtl, rtr, rbr, rbl] = *radii;
    let mut pb = PathBuilder::new();
    pb.move_to(x0 + rtl, y0);
    pb.line_to(x1 - rtr, y0);
    if rtr > 0.0 {
        pb.cubic_to(x1 - rtr + K * rtr, y0, x1, y0 + rtr - K * rtr, x1, y0 + rtr);
    }
    pb.line_to(x1, y1 - rbr);
    if rbr > 0.0 {
        pb.cubic_to(x1, y1 - rbr + K * rbr, x1 - rbr + K * rbr, y1, x1 - rbr, y1);
    }
    pb.line_to(x0 + rbl, y1);
    if rbl > 0.0 {
        pb.cubic_to(x0 + rbl - K * rbl, y1, x0, y1 - rbl + K * rbl, x0, y1 - rbl);
    }
    pb.line_to(x0, y0 + rtl);
    if rtl > 0.0 {
        pb.cubic_to(x0, y0 + rtl - K * rtl, x0 + rtl - K * rtl, y0, x0 + rtl, y0);
    }
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
        if let Some(outer) = rounded_rect_path(ox0, oy0, ox1, oy1, radii) {
            pb.push_path(&outer);
        }
        if let Some(inner) = rounded_rect_path(ix0, iy0, ix1, iy1, &inner_r) {
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
