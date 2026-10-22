// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Cache glyphes **cross-passe** du raster (R-36) : faces
//! parsées et contours de glyphes persistés entre repaints.
//!
//! Avant, chaque `rasterize` créait un `FontCache` neuf :
//! `FontRequest → octets` était re-résolu et `Face::parse`
//! relu **par item texte, à chaque frame** — puis chaque
//! glyphe re-décodait son programme `glyf` (`outline_glyph`)
//! à chaque repaint. Sur un document dense, le raster
//! repassait la totalité des glyphes pour un scroll.
//!
//! `GlyphCache` vit dans la `PaintPayload` de l'onglet
//! (donc `Send` — `Arc`, jamais `Rc`) :
//!
//! - `faces` : `FontRequest → OwnedFace` — les octets sont
//!   possédés par la face (`ttf_parser::Face` est une vue
//!   empruntée ; `OwnedFace` la rend stockable en safe Rust) ;
//! - `paths` : `(face, glyphe) → Path` en **unités fonte**
//!   (upem, y montant) — le transform plume/dpi est appliqué
//!   par `Path::transform` à la volée, arithmétiquement
//!   identique au `GlyphSink` transformé (même `map_point`
//!   par point).
//!
//! ## Invalidation
//!
//! `FontSource::revision` est relue à chaque passe : une
//! source mutable (`WebFontSource` — faces `@font-face`
//! inscrites ou purgées) la bumpe, et tout le cache tombe
//! (faces possédées figées + contours indexés par pointeur
//! de face — un pointeur recyclé risquerait une collision).
//!
//! ## Bornes
//!
//! `FACE_CAP` / `PATH_CAP` : au-delà, le calcul se fait
//! sans insertion (même patron que `SHAPE_CACHE_CAP`) — un
//! document hostile ne gonfle pas le cache sans fin.

use std::collections::HashMap;
use std::sync::Arc;

use explorer_layout::font::{FontRequest, FontSource};
use owned_ttf_parser::OwnedFace;
use tiny_skia::{Path, PathBuilder, Transform};

/// Borne du registre de faces parsées — requêtes distinctes
/// plausibles par page (familles × graisses × italiques).
const FACE_CAP: usize = 64;

/// Borne des contours mémorisés — un glyphe ≈ dizaines de
/// points ; 32 k entrées restent de l'ordre du Mo.
const PATH_CAP: usize = 32_768;

/// Cache persistant de raster (R-36) — voir le module.
#[derive(Default)]
pub struct GlyphCache {
    /// Révision de la source vue à la passe précédente —
    /// `None` avant la première synchro.
    rev: Option<u64>,
    /// `FontRequest → face possédée` (`None` figé tant que
    /// `rev` stable — une famille absente ne re-coûte pas
    /// une résolution par item).
    faces: HashMap<FontRequest, Option<Arc<OwnedFace>>>,
    /// `(ptr d'octets partagés, index de face) → face` —
    /// faces de **repli** (R-69) : les spans d'un
    /// [`ShapedRun`] portent leurs octets en `Arc` plutôt
    /// qu'une requête — identité `Arc::as_ptr`, même
    /// borne `FACE_CAP` partagée.
    faces_fb: HashMap<(usize, u32), Option<Arc<OwnedFace>>>,
    /// `(ptr de face, index glyphe) → contour en unités
    /// fonte` (`None` figé = glyphe sans contour — espace,
    /// `.notdef` absent…).
    paths: HashMap<(usize, u16), Option<Path>>,
}

impl GlyphCache {
    /// Cache vide — identique au comportement pré-R-36.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Réaligne le cache sur la révision courante de la
    /// source : registre muté → tout est vidé (octets,
    /// faces et contours meurent ensemble — les clés de
    /// `paths` sont des pointeurs de faces).
    /// Appelée en tête de chaque passe raster.
    pub fn sync(&mut self, src: &dyn FontSource) {
        let rev = src.revision();
        match self.rev {
            Some(prev) if prev == rev => {}
            _ => {
                self.faces.clear();
                self.faces_fb.clear();
                self.paths.clear();
                self.rev = Some(rev);
            }
        }
    }

    /// Face parsée pour `req` — `Arc` clonée pour libérer
    /// l'emprunt du cache (le `&mut self` reste disponible
    /// pour les contours). `None` figé = fonte inconnue ou
    /// illisible pour la révision courante. Résolution
    /// `font_face` (index `.ttc` réel, R-69) — la même que
    /// le shaping du layout, jamais `font_data` à index 0.
    pub fn face(&mut self, src: &dyn FontSource, req: &FontRequest) -> Option<Arc<OwnedFace>> {
        if let Some(hit) = self.faces.get(req) {
            return hit.clone();
        }
        let face = src
            .font_face(req)
            .and_then(|(d, i)| OwnedFace::from_vec((*d).clone(), i).ok())
            .map(Arc::new);
        if self.faces.len() < FACE_CAP {
            self.faces.insert(req.clone(), face.clone());
        }
        face
    }

    /// Face de **repli** (R-69) — clé `(ptr des octets
    /// partagés, index de face)` : les `FallbackSpan` d'un
    /// run portent leur fonte directement, zéro requête.
    /// `None` figé = octets illisibles.
    pub fn face_at(&mut self, data: &Arc<Vec<u8>>, face_index: u32) -> Option<Arc<OwnedFace>> {
        let k = (Arc::as_ptr(data) as usize, face_index);
        if let Some(hit) = self.faces_fb.get(&k) {
            return hit.clone();
        }
        let face = OwnedFace::from_vec((**data).clone(), face_index)
            .ok()
            .map(Arc::new);
        if self.faces.len() + self.faces_fb.len() < FACE_CAP {
            self.faces_fb.insert(k, face.clone());
        }
        face
    }

    /// Contour du glyphe `id` de `face` en **unités fonte**
    /// (`face_key` = `Arc::as_ptr` de la face — identité des
    /// octets possédés), prêt pour `Path::transform` — le
    /// clone du `Vec` de points est le prix du découplage
    /// d'emprunt. `None` figé = glyphe sans contour.
    pub fn glyph_path(
        &mut self,
        face: &ttf_parser::Face<'_>,
        face_key: usize,
        id: u16,
    ) -> Option<Path> {
        if let Some(hit) = self.paths.get(&(face_key, id)) {
            return hit.clone();
        }
        let mut pb = PathBuilder::new();
        let path = face
            .outline_glyph(
                ttf_parser::GlyphId(id),
                &mut GlyphSink {
                    pb: &mut pb,
                    tf: Transform::default(),
                },
            )
            .and_then(|_| pb.finish());
        if self.paths.len() < PATH_CAP {
            self.paths.insert((face_key, id), path.clone());
        }
        path
    }

    /// Nombre de faces retenues (tests / instrumentation).
    #[must_use]
    pub fn faces_len(&self) -> usize {
        self.faces.len()
    }

    /// Nombre de contours retenus.
    #[must_use]
    pub fn paths_len(&self) -> usize {
        self.paths.len()
    }

    /// Vide tout (sans toucher `rev` — prochain sync à la
    /// même révision ne re-vide pas).
    pub fn clear(&mut self) {
        self.faces.clear();
        self.faces_fb.clear();
        self.paths.clear();
    }
}

/// Adaptateur `OutlineBuilder → PathBuilder` — mêmes
/// transferts de points que `GlyphSink` côté `raster`
/// (dupliqué ici : le sink reste privé à `raster.rs`).
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

/// Point transformé — même helper que `raster::map` (un
/// `map_point` par point, arithmétique identique).
fn map(tf: Transform, x: f32, y: f32) -> (f32, f32) {
    let mut p = tiny_skia::Point::from_xy(x, y);
    tf.map_point(&mut p);
    (p.x, p.y)
}
