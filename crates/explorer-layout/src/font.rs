// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Fontes et shaping (ADR-0009).
//!
//! Frontière : `explorer-layout` définit le **port**
//! [`FontSource`] (données de fontes) et fait tout le calcul —
//! métriques via `ttf-parser`, shaping via `rustybuzz`, pur,
//! aucun I/O. L'implémentation `fontdb` (registre/filesystem)
//! vit en couche infrastructure et est injectée par
//! `explorer-core` — jamais l'inverse.
//!
//! Les fonctions de mesure prennent les **octets de la fonte**
//! (`&[u8]`) pour rester sans état côté appelant ; le cache de
//! [`ttf_parser::Face`] est une optimisation d'infra, pas un
//! invariant du domaine.

use std::collections::HashMap;
use std::sync::Arc;

/// Port : source de données de fontes (ADR-0009).
///
/// Contrat : retourne les octets d'une fonte TTF/OTF répondant
/// à la requête, ou `None` (→ la cascade de familles continue
/// côté appelant). Aucune lecture ici — l'implémentation
/// `fontdb` vit en infra.
pub trait FontSource {
    /// Octets de la fonte pour `req`, `None` si inconnue.
    fn font_data(&self, req: &FontRequest) -> Option<Vec<u8>>;

    /// Révision du registre — `0` pour une source figée
    /// (défaut). Une source **mutable** (ex.
    /// `WebFontSource`, faces `@font-face` inscrites à la
    /// volée) la renvoie croissante à chaque mutation : les
    /// caches cross-passe (`GlyphCache` côté render, R-36)
    /// la comparent pour vider leurs entrées figées — sinon
    /// une famille absente à `t` resterait « inconnue »
    /// après l'arrivée de sa face.
    fn revision(&self) -> u64 {
        0
    }
}

/// Requête de fonte issue du style calculé.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FontRequest {
    /// Familles demandées, par ordre de préférence
    /// (`font-family` CSS, génériques en dernier recours).
    pub families: Vec<String>,
    /// Graisse 1-1000 (`font-weight`, 400 normal).
    pub weight: u16,
    /// Italique / oblique.
    pub italic: bool,
}

/// Métriques verticales de la fonte à une taille donnée (px).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FontMetrics {
    /// Montée au-dessus de la baseline.
    pub ascent: f32,
    /// Descente sous la baseline (négative).
    pub descent: f32,
    /// Interligne additionnel (`lineGap`).
    pub line_gap: f32,
    /// Unités par em de la fonte.
    pub units_per_em: u16,
}

impl FontMetrics {
    /// Hauteur de ligne naturelle (`ascent - descent + gap`).
    #[must_use]
    pub fn line_height(self) -> f32 {
        self.ascent - self.descent + self.line_gap
    }
}

/// Glyphe positionné issu du shaping.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Glyph {
    /// Index du glyphe dans la fonte.
    pub id: u16,
    /// Avance horizontale (px, signe inclus direction).
    pub advance: f32,
    /// Décalage x/y du dessin (px).
    pub offset_x: f32,
    /// Décalage vertical.
    pub offset_y: f32,
    /// Cluster source (offset octet dans le run) — sert au
    /// hit-testing et à la sélection.
    pub cluster: u32,
}

/// Run de texte shapé : glyphes + métriques agrégées.
#[derive(Debug, Clone, PartialEq)]
pub struct ShapedRun {
    /// Glyphes dans l'ordre logique du run.
    pub glyphs: Vec<Glyph>,
    /// Avance totale (px).
    pub advance: f32,
    /// Métriques de la fonte à la taille utilisée.
    pub metrics: FontMetrics,
    /// Bbox d'**encre réelle** du run (px CSS, origine =
    /// plume du premier glyphe, baseline = 0 : les `y`
    /// négatifs montent au-dessus de la ligne de base) —
    /// union des `glyph_bounding_box` décalés par position
    /// de plume et offsets. L'encre d'un glyphe peut sortir
    /// de la boîte de métriques (italique, accents,
    /// fontes plein-em type Ahem) : `None` si aucun glyphe
    /// n'a de contour (run d'espaces). Sert aux bounds de
    /// peinture du raster — R-37.
    pub ink: Option<crate::Rect>,
}

/// Cache de la pipeline fonte, borné à une passe de layout
/// ou de raster (R-33b) : auparavant `font_data`, le parse de
/// face et le shaping étaient re-facturés pour **chaque** run
/// de texte — plusieurs milliers d'appels par page lourde
/// (~28 s de layout sur ~9 000 nœuds, profil
/// `profile_resync_phases`). Une résolution par `FontRequest`,
/// un parse de métriques par (fonte, taille), un shaping par
/// (fonte, taille, texte, direction).
///
/// `Send`/`Sync` (R-67 : `Arc` — la demande NAV 2026-11-08 le
/// fait transiter dans `PaintPayload` sous `spawn_blocking` pour
/// persister entre applys) ; sert sous `RefCell` dans
/// [`crate::BlockCtx::font_cache`] — le domaine reste
/// single-writer par contrat.
#[derive(Debug, Default)]
pub struct FontCache {
    /// Révision de la source vue à la dernière requête —
    /// sync `FontSource::revision` (même contrat que le
    /// `GlyphCache` render, R-36) : indispensable maintenant
    /// que le cache peut vivre entre passes
    /// (`BlockCtx::with_font_cache`) — un `@font-face`
    /// inscrit entre deux applys doit invalider les `None`
    /// figés, sinon la famille resterait « inconnue ».
    rev: Option<u64>,
    /// `FontRequest` → octets résolus (`None` = miss figé).
    data: HashMap<FontRequest, Option<Arc<Vec<u8>>>>,
    /// (identité de fonte, taille px en bits) → métriques
    /// verticales. L'identité = l'adresse des octets cachés —
    /// stable tant que `data` vit.
    metrics: HashMap<(usize, u32), Option<FontMetrics>>,
    /// Runs shapés mémoïsés par (identité, taille, texte,
    /// rtl). `Arc` : les fragments partagent le run (R-37 —
    /// identité de contenu stable pour le build incrémental
    /// + mémoire dédupliquée). Borné à `SHAPE_CACHE_CAP`.
    shape: HashMap<(usize, u32, String, bool), Option<std::sync::Arc<ShapedRun>>>,
}

/// Borne des entrées `shape` mémorisées.
const SHAPE_CACHE_CAP: usize = 8192;

impl FontCache {
    /// Cache vide pour une passe.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Réaligne le cache sur la révision courante de la
    /// source : registre muté → tout est vidé (octets,
    /// métriques et runs meurent ensemble — les clés de
    /// `metrics`/`shape` sont des pointeurs dans `data`).
    /// Re-lu à chaque requête : source figée (`0`) = un
    /// `Option<u64>` comparé, quasi gratuit.
    fn sync(&mut self, src: &dyn FontSource) {
        let rev = src.revision();
        if self.rev != Some(rev) {
            self.data.clear();
            self.metrics.clear();
            self.shape.clear();
            self.rev = Some(rev);
        }
    }

    /// Octets de la fonte — mémoïsés par requête ; `None`
    /// figé (une famille absente ne re-coûte rien) **pour la
    /// révision courante** — voir `rev`.
    pub fn data(&mut self, src: &dyn FontSource, req: &FontRequest) -> Option<Arc<Vec<u8>>> {
        self.sync(src);
        self.data
            .entry(req.clone())
            .or_insert_with(|| src.font_data(req).map(Arc::new))
            .clone()
    }

    /// Adresse d'identité des octets cachés (clé des cartes
    /// `metrics`/`shape`) — `None` si la fonte est absente.
    fn key(&mut self, src: &dyn FontSource, req: &FontRequest) -> Option<usize> {
        self.data(src, req).map(|d| Arc::as_ptr(&d) as usize)
    }

    /// Métriques verticales à `size_px` — un parse `hhea`
    /// par (fonte, taille).
    pub fn metrics(
        &mut self,
        src: &dyn FontSource,
        req: &FontRequest,
        size_px: f32,
    ) -> Option<FontMetrics> {
        let k = self.key(src, req)?;
        if let Some(&m) = self.metrics.get(&(k, size_px.to_bits())) {
            return m;
        }
        let m = self.data(src, req).and_then(|d| metrics(&d, size_px));
        self.metrics.insert((k, size_px.to_bits()), m);
        m
    }

    /// Run shapé — mémoïsé par (fonte, taille, texte, dir) ;
    /// borne `SHAPE_CACHE_CAP` (au-delà, calcul sans insert).
    /// Retour `Arc` partagé : deux fragments du même texte
    /// pointent le **même** run (mémoire + identité R-37).
    pub fn shape(
        &mut self,
        src: &dyn FontSource,
        req: &FontRequest,
        text: &str,
        size_px: f32,
        rtl: bool,
    ) -> Option<std::sync::Arc<ShapedRun>> {
        let k = self.key(src, req)?;
        if let Some(hit) = self
            .shape
            .get(&(k, size_px.to_bits(), text.to_string(), rtl))
        {
            return hit.clone();
        }
        let run = self
            .data(src, req)
            .and_then(|d| shape(&d, text, size_px, rtl))
            .map(std::sync::Arc::new);
        if self.shape.len() < SHAPE_CACHE_CAP {
            self.shape
                .insert((k, size_px.to_bits(), text.to_string(), rtl), run.clone());
        }
        run
    }
}

/// `FontRequest` du style calculé — partagée par les points
/// d'entrée fonte (avant : triple copie inline/block/font).
#[must_use]
pub fn request_of(style: &explorer_css::ComputedStyle, ctx: &crate::BlockCtx<'_>) -> FontRequest {
    let families: Vec<String> = style.get("font-family").map_or_else(
        || vec![ctx.default_family.to_string()],
        |f| {
            f.split(',')
                .map(|s| s.trim().trim_matches(|c| c == '"' || c == '\'').to_string())
                .collect()
        },
    );
    let weight = style
        .get("font-weight")
        .and_then(|w| match w.trim() {
            "bold" | "bolder" => Some(700u16),
            "normal" | "lighter" => Some(400),
            v => v.parse().ok(),
        })
        .unwrap_or(400);
    let italic = matches!(style.get("font-style"), Some("italic" | "oblique"));
    FontRequest {
        families,
        weight,
        italic,
    }
}

/// Métriques d'une fonte à `size_px`, `None` si octets
/// invalides (dégradation : l'appelant tente la famille
/// suivante ou un carré `.notdef`).
#[must_use]
pub fn metrics(font_data: &[u8], size_px: f32) -> Option<FontMetrics> {
    metrics_of(&ttf_parser::Face::parse(font_data, 0).ok()?, size_px)
}

/// [`metrics`] sur une face déjà parsée — évite un second
/// parse quand l'appelant la possède (`shape`).
fn metrics_of(face: &ttf_parser::Face<'_>, size_px: f32) -> Option<FontMetrics> {
    let upem = f32::from(face.units_per_em());
    if upem <= 0.0 {
        return None;
    }
    let scale = size_px / upem;
    Some(FontMetrics {
        ascent: f32::from(face.ascender()) * scale,
        descent: f32::from(face.descender()) * scale,
        line_gap: f32::from(face.line_gap()) * scale,
        units_per_em: face.units_per_em(),
    })
}

/// Shape `text` avec la fonte (octets) à `size_px`.
///
/// `rtl` inverse l'avance cumulée attendue par l'appelant
/// (rustybuzz rend déjà les glyphes dans l'ordre visuel).
/// Retourne `None` si la fonte est illisible — dégradation
/// côté appelant, jamais panic sur fonte hostile.
#[must_use]
pub fn shape(font_data: &[u8], text: &str, size_px: f32, rtl: bool) -> Option<ShapedRun> {
    let tface = ttf_parser::Face::parse(font_data, 0).ok()?;
    let met = metrics_of(&tface, size_px)?;
    let face = rustybuzz::Face::from_slice(font_data, 0)?;
    // rustybuzz expose `units_per_em` en `i32` — clamp avant
    // conversion (fonte hostile → 0/négatif possible).
    #[allow(clippy::cast_precision_loss)]
    let upem = face.units_per_em().max(1) as f32;
    let scale = size_px / upem;

    let mut buf = rustybuzz::UnicodeBuffer::new();
    buf.push_str(text);
    if rtl {
        buf.set_direction(rustybuzz::Direction::RightToLeft);
    }
    let out = rustybuzz::shape(&face, &[], buf);

    let positions = out.glyph_positions();
    // i32→f32 : les positions de glyphes restent bien en deçà
    // de la précision f32 (unités fonte ≤ ±32k typique) ;
    // u32→u16 borné par try_from (id > 65535 impossible en
    // TrueType, mais `unwrap_or` préférable à panic).
    #[allow(clippy::cast_precision_loss)]
    let glyphs: Vec<Glyph> = out
        .glyph_infos()
        .iter()
        .zip(positions.iter())
        .map(|(info, pos)| Glyph {
            id: u16::try_from(info.glyph_id).unwrap_or(0),
            advance: pos.x_advance as f32 * scale,
            offset_x: pos.x_offset as f32 * scale,
            offset_y: pos.y_offset as f32 * scale,
            cluster: info.cluster,
        })
        .collect();
    let advance = glyphs.iter().map(|g| g.advance).sum();
    // Bbox d'encre réelle : union des bounding boxes de
    // contours décalées par position de plume (même
    // cumul `pen += advance` que le raster) et offsets —
    // y fonte monte → y CSS = −offset_y − y·scale. Un
    // glyphe peut sortir de la boîte de métriques.
    let mut pen = 0.0_f32;
    let mut ink: Option<crate::Rect> = None;
    for g in &glyphs {
        if let Some(b) = tface.glyph_bounding_box(ttf_parser::GlyphId(g.id)) {
            let r = crate::Rect {
                x: pen + g.offset_x + f32::from(b.x_min) * scale,
                y: -g.offset_y - f32::from(b.y_max) * scale,
                width: f32::from(b.x_max - b.x_min) * scale,
                height: f32::from(b.y_max - b.y_min) * scale,
            };
            ink = Some(match ink {
                Some(i) => crate::inline::union_rect(i, r),
                None => r,
            });
        }
        pen += g.advance;
    }
    Some(ShapedRun {
        glyphs,
        advance,
        metrics: met,
        ink,
    })
}

/// Largeur d'une chaîne — raccourci `shape().advance`.
#[must_use]
pub fn measure(font_data: &[u8], text: &str, size_px: f32) -> f32 {
    shape(font_data, text, size_px, false).map_or(0.0, |r| r.advance)
}

/// `line-height: normal` (§10.8.1) : la valeur « raisonnable
/// basée sur la fonte » = ses métriques réelles `hhea`
/// (`ascent − descent + gap` à `size_px`) — Ahem sert 1.0em,
/// les textes de conformance restent sur la grille entière.
/// `font_data_of` résout la requête style → octets ; repli
/// `size_px * 1.2` (arrêté UA) quand aucune fonte n'est
/// disponible (domaine sans `FontSource` ou fonte absente).
#[must_use]
pub fn natural_line_height(
    style: &explorer_css::ComputedStyle,
    size_px: f32,
    ctx: &crate::BlockCtx<'_>,
) -> f32 {
    let Some(src) = ctx.fonts else {
        return size_px * 1.2;
    };
    let req = request_of(style, ctx);
    ctx.font_cache
        .borrow_mut()
        .metrics(src, &req, size_px)
        .map_or(size_px * 1.2, FontMetrics::line_height)
}

/// Octets de la fonte du style via le port `FontSource` —
/// mémoïsés dans `ctx.font_cache` (R-33b) ; `None` sans
/// source injectée ni fonte trouvée.
#[must_use]
pub fn font_data_of(
    style: &explorer_css::ComputedStyle,
    ctx: &crate::BlockCtx<'_>,
) -> Option<Arc<Vec<u8>>> {
    let src = ctx.fonts?;
    let req = request_of(style, ctx);
    ctx.font_cache.borrow_mut().data(src, &req)
}
