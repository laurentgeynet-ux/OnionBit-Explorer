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

    /// Octets **+ index de face** dans une collection
    /// (`.ttc`, R-69) — `font_data` seul ne peut pas porter
    /// l'index. Défaut : `font_data` à l'index 0 — les
    /// sources mono-fichier n'ont rien à faire.
    fn font_face(&self, req: &FontRequest) -> Option<(Arc<Vec<u8>>, u32)> {
        self.font_data(req).map(|d| (Arc::new(d), 0))
    }

    /// Fonte de repli couvrant le caractère `c` (fallback
    /// par glyphe, R-69) — octets + index de face, `Arc`
    /// pour éviter un clone de plusieurs Mo par caractère.
    /// `req` = requête d'origine : la source préfère les
    /// familles demandées quand elles couvrent, puis son
    /// registre. Défaut : `None` (pas de repli — les
    /// glyphes manquants restent `.notdef`).
    fn fallback_for(&self, c: char, req: &FontRequest) -> Option<(Arc<Vec<u8>>, u32)> {
        let _ = (c, req);
        None
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

/// Plage de glyphes shapés dans une fonte de **repli**
/// (R-69 — fallback par glyphe) : `[start, end)` en index
/// de glyphe du run, plages triées croissant non
/// chevauchantes. `data@face` = octets partagés + index
/// dans une collection `.ttc` (`0` pour un fichier simple).
#[derive(Debug, Clone)]
pub struct FallbackSpan {
    /// Premier glyphe de la plage (inclus).
    pub start: u32,
    /// Dernier glyphe de la plage (exclu).
    pub end: u32,
    /// Octets de la fonte de repli (partagés `FontCache` —
    /// comparés par pointeur, jamais par contenu).
    pub data: Arc<Vec<u8>>,
    /// Index de face dans une collection `.ttc`, `0` sinon.
    pub face: u32,
}

impl PartialEq for FallbackSpan {
    /// Identité par pointeur : comparer les octets de fonte
    /// (plusieurs Mo) à chaque `eq` de fragment coûterait
    /// trop — deux spans égaux partagent le même `Arc`.
    fn eq(&self, other: &Self) -> bool {
        self.start == other.start
            && self.end == other.end
            && self.face == other.face
            && Arc::ptr_eq(&self.data, &other.data)
    }
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
    /// Plages de glyphes issues du fallback par glyphe —
    /// vides dans le cas dominant (run mono-fonte, aucune
    /// alloc). Le raster peint ces glyphes avec
    /// `span.data@span.face` au lieu de la fonte de la
    /// requête.
    pub fallback_spans: Vec<FallbackSpan>,
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
    /// `FontRequest` → (octets, index de face `.ttc`)
    /// résolus (`None` = miss figé).
    faces: HashMap<FontRequest, Option<FaceRef>>,
    /// (identité de fonte, index de face, taille px en
    /// bits) → métriques verticales. L'identité = l'adresse
    /// des octets cachés — stable tant que `faces` vit.
    metrics: HashMap<(usize, u32, u32), Option<FontMetrics>>,
    /// Runs shapés mémoïsés par (identité, index de face,
    /// taille, texte, rtl). `Arc` : les fragments partagent
    /// le run (R-37 — identité de contenu stable pour le
    /// build incrémental + mémoire dédupliquée). Borné à
    /// `SHAPE_CACHE_CAP`.
    shape: HashMap<ShapeKey, Option<std::sync::Arc<ShapedRun>>>,
    /// (caractère, requête) → face de repli (`None` figé =
    /// aucune face du registre ne le couvre). La requête est
    /// dans la clé : `fallback_for` préfère les familles
    /// demandées — un cache à caractère seul rendrait le
    /// choix dépendant de l'ordre des runs. Borné à
    /// `FB_CACHE_CAP` — un texte CJK dense ne re-parcourt
    /// le registre qu'une fois par caractère distinct.
    fb: HashMap<(char, FontRequest), Option<FaceRef>>,
}

/// Octets de fonte partagés + index de face `.ttc` — la
/// référence que les cartes du cache retournent (R-69).
type FaceRef = (Arc<Vec<u8>>, u32);

/// Clé des runs mémoïsés : (identité octets, index face,
/// taille px en bits, texte, rtl).
type ShapeKey = (usize, u32, u32, String, bool);

/// Choix de face par segment de texte (R-69) : la fonte
/// primaire, ou une face de repli couvrant le segment.
#[derive(Clone)]
enum Choice {
    Primary,
    Fallback(Arc<Vec<u8>>, u32),
}

/// Borne des entrées `shape` mémorisées.
const SHAPE_CACHE_CAP: usize = 8192;

/// Borne des entrées `fb` mémorisées (caractères distincts
/// plausibles par document).
const FB_CACHE_CAP: usize = 4096;

impl FontCache {
    /// Cache vide pour une passe.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Réaligne le cache sur la révision courante de la
    /// source : registre muté → tout est vidé (octets,
    /// métriques et runs meurent ensemble — les clés de
    /// `metrics`/`shape` sont des pointeurs dans `faces`).
    /// Re-lu à chaque requête : source figée (`0`) = un
    /// `Option<u64>` comparé, quasi gratuit.
    fn sync(&mut self, src: &dyn FontSource) {
        let rev = src.revision();
        if self.rev != Some(rev) {
            self.faces.clear();
            self.metrics.clear();
            self.shape.clear();
            self.fb.clear();
            self.rev = Some(rev);
        }
    }

    /// Octets + index de face — mémoïsés par requête ;
    /// `None` figé (une famille absente ne re-coûte rien)
    /// **pour la révision courante** — voir `rev`.
    fn face(&mut self, src: &dyn FontSource, req: &FontRequest) -> Option<(Arc<Vec<u8>>, u32)> {
        self.sync(src);
        self.faces
            .entry(req.clone())
            .or_insert_with(|| src.font_face(req))
            .clone()
    }

    /// Octets de la fonte (index de face perdu — compat
    /// `font_data_of` ; les consommateurs qui parsent à
    /// l'index 0 sur une collection prennent la face 0 —
    /// limite connue, documentée).
    pub fn data(&mut self, src: &dyn FontSource, req: &FontRequest) -> Option<Arc<Vec<u8>>> {
        self.face(src, req).map(|(d, _)| d)
    }

    /// Face de repli pour `c` sous `req` — mémoïsée par
    /// (caractère, requête) ; `None` figé = le registre ne
    /// couvre pas ce glyphe pour ces familles.
    fn fallback(
        &mut self,
        src: &dyn FontSource,
        c: char,
        req: &FontRequest,
    ) -> Option<(Arc<Vec<u8>>, u32)> {
        self.sync(src);
        if let Some(hit) = self.fb.get(&(c, req.clone())) {
            return hit.clone();
        }
        let f = src.fallback_for(c, req);
        if self.fb.len() < FB_CACHE_CAP {
            self.fb.insert((c, req.clone()), f.clone());
        }
        f
    }

    /// Adresse d'identité des octets cachés (clé des cartes
    /// `metrics`/`shape`) — `None` si la fonte est absente.
    fn key(&mut self, src: &dyn FontSource, req: &FontRequest) -> Option<(usize, u32)> {
        self.face(src, req)
            .map(|(d, i)| (Arc::as_ptr(&d) as usize, i))
    }

    /// Métriques verticales à `size_px` — un parse `hhea`
    /// par (fonte, face, taille).
    pub fn metrics(
        &mut self,
        src: &dyn FontSource,
        req: &FontRequest,
        size_px: f32,
    ) -> Option<FontMetrics> {
        let (k, i) = self.key(src, req)?;
        if let Some(&m) = self.metrics.get(&(k, i, size_px.to_bits())) {
            return m;
        }
        let m = self
            .face(src, req)
            .and_then(|(d, i)| metrics_face(&d, i, size_px));
        self.metrics.insert((k, i, size_px.to_bits()), m);
        m
    }

    /// Run shapé — mémoïsé par (fonte, taille, texte, dir) ;
    /// borne `SHAPE_CACHE_CAP` (au-delà, calcul sans insert).
    /// Retour `Arc` partagé : deux fragments du même texte
    /// pointent le **même** run (mémoire + identité R-37).
    /// Glyphes manquants → segmentation par couverture et
    /// shaping en fonte de repli (R-69) — voir
    /// `shape_with_fallback`.
    pub fn shape(
        &mut self,
        src: &dyn FontSource,
        req: &FontRequest,
        text: &str,
        size_px: f32,
        rtl: bool,
    ) -> Option<std::sync::Arc<ShapedRun>> {
        let (k, i) = self.key(src, req)?;
        if let Some(hit) = self
            .shape
            .get(&(k, i, size_px.to_bits(), text.to_string(), rtl))
        {
            return hit.clone();
        }
        let run = self
            .face(src, req)
            .and_then(|(d, i)| shape_face(&d, i, text, size_px, rtl).map(|r| (d, i, r)))
            .map(|(d, i, r)| {
                let r = if r.glyphs.iter().any(|g| g.id == 0) {
                    self.shape_with_fallback(src, req, &d, i, text, size_px, rtl, r)
                } else {
                    r
                };
                std::sync::Arc::new(r)
            });
        if self.shape.len() < SHAPE_CACHE_CAP {
            self.shape.insert(
                (k, i, size_px.to_bits(), text.to_string(), rtl),
                run.clone(),
            );
        }
        run
    }

    /// Re-shape `text` en **segments de couverture** (R-69) :
    /// les caractères absents du cmap de la fonte principale
    /// sont groupés en plages contiguës, chaque plage est
    /// shapée avec la première fonte de repli qui la couvre
    /// (`FontSource::fallback_for`) — sans repli trouvé, la
    /// plage reste dans la fonte principale (carrés
    /// `.notdef` honnêtes). Les glyphes fusionnés portent
    /// leurs clusters réajustés (offsets d'octet du segment)
    /// et `fallback_spans` balise les plages pour le raster.
    /// `primary` = run mono-fonte déjà produit (métriques et
    /// glyphes couverts sont réutilisés par segment quand
    /// c'est possible — les segments couverts reprennent le
    /// shaping indépendant, approximant le contexte
    /// inter-segments : limite documentée).
    #[allow(clippy::too_many_arguments)]
    fn shape_with_fallback(
        &mut self,
        src: &dyn FontSource,
        req: &FontRequest,
        data: &Arc<Vec<u8>>,
        index: u32,
        text: &str,
        size_px: f32,
        rtl: bool,
        primary: ShapedRun,
    ) -> ShapedRun {
        let Some(tface) = ttf_parser::Face::parse(data, index).ok() else {
            return primary;
        };
        // Segments logiques : couverture de `tface` caractère
        // par caractère → plages contiguës ; chaque plage non
        // couverte cherche UNE face de repli commune (groupe
        // par (pointeur, index) — alternance japonais/coréen
        // sur deux fontes découpe correctement).
        let mut segs: Vec<(usize, usize, Choice)> = Vec::new();
        for (off, c) in text.char_indices() {
            let covered = tface.glyph_index(c).is_some_and(|g| g.0 != 0);
            let choice = if covered {
                Choice::Primary
            } else {
                self.fallback(src, c, req)
                    .map_or(Choice::Primary, |(d, i)| Choice::Fallback(d, i))
            };
            let same =
                segs.last_mut()
                    .is_some_and(|(_, end, prev): &mut (usize, usize, Choice)| {
                        let eq = match (prev, &choice) {
                            (Choice::Primary, Choice::Primary) => true,
                            (Choice::Fallback(a, ia), Choice::Fallback(b, ib)) => {
                                *ia == *ib && Arc::ptr_eq(a, b)
                            }
                            _ => false,
                        };
                        if eq {
                            *end = off + c.len_utf8();
                        }
                        eq
                    });
            if !same {
                segs.push((off, off + c.len_utf8(), choice));
            }
        }
        // Sortie anticipée : run entièrement primaire =
        // shaping déjà produit. Un texte **entier** en repli
        // (un seul segment `Fallback`) doit continuer.
        if segs.iter().all(|s| matches!(s.2, Choice::Primary)) {
            return primary;
        }
        // Shaping par segment + fusion : clusters décalés de
        // l'offset du segment, `rtl` inverse l'ordre des
        // segments pour l'affichage (approximation bidi).
        let metrics = primary.metrics;
        let order: Vec<usize> = if rtl {
            (0..segs.len()).rev().collect()
        } else {
            (0..segs.len()).collect()
        };
        let mut glyphs = Vec::new();
        let mut advance = 0.0;
        let mut ink: Option<crate::Rect> = None;
        let mut spans: Vec<FallbackSpan> = Vec::new();
        for si in order {
            let (start, end, choice) = &segs[si];
            let (bytes, idx) = match choice {
                Choice::Primary => (data.clone(), index),
                Choice::Fallback(d, i) => (d.clone(), *i),
            };
            let Some(run) = shape_face(&bytes, idx, &text[*start..*end], size_px, rtl) else {
                continue;
            };
            let start_glyph = u32::try_from(glyphs.len()).unwrap_or(u32::MAX);
            // Face re-parsée pour la bbox d'encre (une par
            // segment — peu de segments par run).
            let tf = ttf_parser::Face::parse(&bytes, idx).ok();
            let scale =
                size_px / f32::from(tf.as_ref().map_or(1, ttf_parser::Face::units_per_em)).max(1.0);
            let mut pen = advance;
            for mut g in run.glyphs {
                g.cluster = g
                    .cluster
                    .saturating_add(u32::try_from(*start).unwrap_or(u32::MAX));
                if let Some(f) = tf.as_ref() {
                    if let Some(b) = f.glyph_bounding_box(ttf_parser::GlyphId(g.id)) {
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
                }
                pen += g.advance;
                glyphs.push(g);
            }
            if let Choice::Fallback(d, i) = choice {
                spans.push(FallbackSpan {
                    start: start_glyph,
                    end: u32::try_from(glyphs.len()).unwrap_or(u32::MAX),
                    data: d.clone(),
                    face: *i,
                });
            }
            advance += run.advance;
        }
        ShapedRun {
            glyphs,
            advance,
            metrics,
            fallback_spans: spans,
            ink,
        }
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
    metrics_face(font_data, 0, size_px)
}

/// [`metrics`] à l'index `face_index` d'une collection
/// `.ttc` (`0` = fichier simple — index `u32` pour coller
/// à `ttf_parser`).
#[must_use]
pub fn metrics_face(font_data: &[u8], face_index: u32, size_px: f32) -> Option<FontMetrics> {
    metrics_of(
        &ttf_parser::Face::parse(font_data, face_index).ok()?,
        size_px,
    )
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
    shape_face(font_data, 0, text, size_px, rtl)
}

/// [`shape`] à l'index `face_index` d'une collection
/// `.ttc` (`0` = fichier simple).
#[must_use]
pub fn shape_face(
    font_data: &[u8],
    face_index: u32,
    text: &str,
    size_px: f32,
    rtl: bool,
) -> Option<ShapedRun> {
    let tface = ttf_parser::Face::parse(font_data, face_index).ok()?;
    let met = metrics_of(&tface, size_px)?;
    let face = rustybuzz::Face::from_slice(font_data, face_index)?;
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
        fallback_spans: Vec::new(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use explorer_test_support::{fixture_ttf, fixture_ttf_custom};

    /// Source à deux fontes : `latin` (fixture U+0020-00FF)
    /// servie à la requête, `cjk` (fixture U+6500-67FF)
    /// servie par `fallback_for` — mécanique R-69 mesurée
    /// sans dépendre du système de fichiers.
    struct Src {
        latin: Arc<Vec<u8>>,
        cjk: Arc<Vec<u8>>,
        with_fb: bool,
    }

    impl Src {
        fn new(with_fb: bool) -> Self {
            Self {
                latin: Arc::new(fixture_ttf()),
                // Plage bornée aux 255 glyphes de la fixture —
                // couvre 日 (0x65E5 → id 6) et 明 (0x660E →
                // id 0x4F).
                cjk: Arc::new(fixture_ttf_custom("Cjk", 0x65E0, 0x65FF)),
                with_fb,
            }
        }

        fn req() -> FontRequest {
            FontRequest {
                families: vec!["Latin".into()],
                weight: 400,
                italic: false,
            }
        }
    }

    impl FontSource for Src {
        fn font_data(&self, _req: &FontRequest) -> Option<Vec<u8>> {
            Some((*self.latin).clone())
        }

        fn font_face(&self, _req: &FontRequest) -> Option<(Arc<Vec<u8>>, u32)> {
            Some((self.latin.clone(), 0))
        }

        fn fallback_for(&self, c: char, _req: &FontRequest) -> Option<(Arc<Vec<u8>>, u32)> {
            (self.with_fb && (0x65E0..=0x65FF).contains(&(c as u32))).then(|| (self.cjk.clone(), 0))
        }
    }

    /// 'A' couvert, '日' non couvert → run segmenté : le
    /// glyphe manquant est shapé dans la fonte de repli et
    /// `fallback_spans` balise la plage pour le raster.
    #[test]
    fn fallback_segmente_le_run() {
        let src = Src::new(true);
        let mut cache = FontCache::new();
        let run = cache
            .shape(&src, &Src::req(), "A日C", 10.0, false)
            .expect("run");
        assert_eq!(run.glyphs.len(), 3);
        assert_eq!(run.fallback_spans.len(), 1);
        let span = &run.fallback_spans[0];
        assert_eq!((span.start, span.end, span.face), (1, 2, 0));
        assert!(Arc::ptr_eq(&span.data, &src.cjk));
        // '日' a un vrai glyphe dans la fixture CJK
        // (0x65E5 → id 6), pas `.notdef`.
        assert_eq!(run.glyphs[1].id, 6);
        // Cluster de 'C' = son offset octet réel (日 = 3
        // octets UTF-8) — réajusté sur le texte complet.
        assert_eq!(run.glyphs[2].cluster, 4);
        assert_eq!(run.advance, 15.0); // 3 × 0.5em à 10 px
    }

    /// Texte entièrement couvert : aucun span — le chemin
    /// reste identique à l'avant-R-69.
    #[test]
    fn couverture_complete_sans_span() {
        let src = Src::new(true);
        let mut cache = FontCache::new();
        let run = cache
            .shape(&src, &Src::req(), "ABC", 10.0, false)
            .expect("run");
        assert_eq!(run.glyphs.len(), 3);
        assert_eq!(run.fallback_spans.len(), 0);
    }

    /// Deux manquants contigus dans la même fonte de repli
    /// = **un** span (regroupement par (ptr, index)).
    #[test]
    fn manquants_contigus_regroupes() {
        let src = Src::new(true);
        let mut cache = FontCache::new();
        let run = cache
            .shape(&src, &Src::req(), "日旦", 10.0, false)
            .expect("run");
        assert_eq!(run.glyphs.len(), 2);
        assert_eq!(run.fallback_spans.len(), 1);
        assert_eq!(
            (run.fallback_spans[0].start, run.fallback_spans[0].end),
            (0, 2)
        );
        // '旦' (0x65E6) → id 7 dans la fixture.
        assert_eq!(run.glyphs[1].id, 7);
    }

    /// Sans source de repli : glyphe `.notdef` honnête,
    /// aucun span — dégradation inchangée.
    #[test]
    fn sans_repli_notdef() {
        let src = Src::new(false);
        let mut cache = FontCache::new();
        let run = cache
            .shape(&src, &Src::req(), "A日", 10.0, false)
            .expect("run");
        assert_eq!(run.glyphs.len(), 2);
        assert_eq!(run.glyphs[1].id, 0);
        assert_eq!(run.fallback_spans.len(), 0);
    }

    /// `fallback_for` sans `fallback_for` : la méthode par
    /// défaut du port renvoie `None` — aucune panique sur
    /// une source minimale.
    #[test]
    fn source_minimale_defaut() {
        struct Min;
        impl FontSource for Min {
            fn font_data(&self, _req: &FontRequest) -> Option<Vec<u8>> {
                Some(fixture_ttf())
            }
        }
        let src = Min;
        let mut cache = FontCache::new();
        let run = cache
            .shape(&src, &Src::req(), "日", 10.0, false)
            .expect("run");
        assert_eq!(run.glyphs[0].id, 0);
    }

    /// `face` (`font_face`) propage l'index `.ttc` — la clé
    /// `metrics`/`shape` le distingue du même octet à un
    /// autre index.
    #[test]
    fn face_index_propage() {
        struct Ttc;
        impl FontSource for Ttc {
            fn font_data(&self, _req: &FontRequest) -> Option<Vec<u8>> {
                Some(fixture_ttf())
            }
            fn font_face(&self, _req: &FontRequest) -> Option<(Arc<Vec<u8>>, u32)> {
                Some((Arc::new(fixture_ttf()), 2))
            }
        }
        let src = Ttc;
        let mut cache = FontCache::new();
        assert_eq!(cache.face(&src, &Src::req()).map(|(_, i)| i), Some(2));
    }
}
