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

/// Port : source de données de fontes (ADR-0009).
///
/// Contrat : retourne les octets d'une fonte TTF/OTF répondant
/// à la requête, ou `None` (→ la cascade de familles continue
/// côté appelant). Aucune lecture ici — l'implémentation
/// `fontdb` vit en infra.
pub trait FontSource {
    /// Octets de la fonte pour `req`, `None` si inconnue.
    fn font_data(&self, req: &FontRequest) -> Option<Vec<u8>>;
}

/// Requête de fonte issue du style calculé.
#[derive(Debug, Clone, PartialEq, Eq)]
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
}

/// Métriques d'une fonte à `size_px`, `None` si octets
/// invalides (dégradation : l'appelant tente la famille
/// suivante ou un carré `.notdef`).
#[must_use]
pub fn metrics(font_data: &[u8], size_px: f32) -> Option<FontMetrics> {
    let face = ttf_parser::Face::parse(font_data, 0).ok()?;
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
    let met = metrics(font_data, size_px)?;
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
    Some(ShapedRun {
        glyphs,
        advance,
        metrics: met,
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
    font_data(style, ctx)
        .and_then(|data| metrics(&data, size_px))
        .map_or(size_px * 1.2, FontMetrics::line_height)
}

/// Octets de la fonte du style via le port `FontSource` —
/// `None` sans source injectée ni fonte trouvée.
fn font_data(style: &explorer_css::ComputedStyle, ctx: &crate::BlockCtx<'_>) -> Option<Vec<u8>> {
    let src = ctx.fonts?;
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
            _ => w.trim().parse().ok(),
        })
        .unwrap_or(400);
    let italic = matches!(style.get("font-style"), Some("italic" | "oblique"));
    src.font_data(&FontRequest {
        families,
        weight,
        italic,
    })
}
