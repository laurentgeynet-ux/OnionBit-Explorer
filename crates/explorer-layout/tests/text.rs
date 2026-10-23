// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Tests de la stack texte (étape 18) : itemisation
//! bidi/UAX #14, shaping `rustybuzz`, métriques réelles sur
//! la fonte fixture embarquée, port `FontSource`.

// Mesures de test en `len() as f32` — borné par la taille
// des chaînes de test.
#![allow(clippy::cast_precision_loss)]

mod common;

use explorer_layout::font::{self, FontCache, FontRequest, FontSource};
use explorer_layout::text::{self, Direction};

/// Port `FontSource` minimal dosé sur la fixture.
struct FixtureSource(Vec<u8>);
impl FontSource for FixtureSource {
    fn font_data(&self, req: &FontRequest) -> Option<Vec<u8>> {
        if req.families.iter().any(|f| f == "Fixture") {
            Some(self.0.clone())
        } else {
            None
        }
    }
}

/// Source qui compte ses appels — prouve que le cache ne
/// re-facture pas `font_data` par run (R-33b).
struct CountingSource {
    ttf: Vec<u8>,
    calls: std::cell::Cell<usize>,
}
impl FontSource for CountingSource {
    fn font_data(&self, req: &FontRequest) -> Option<Vec<u8>> {
        self.calls.set(self.calls.get() + 1);
        if req.families.iter().any(|f| f == "Fixture") {
            Some(self.ttf.clone())
        } else {
            None
        }
    }
}

fn fixture_req() -> FontRequest {
    FontRequest {
        families: vec!["Fixture".into()],
        weight: 400,
        italic: false,
    }
}

#[test]
fn fonte_fixture_est_parsable() {
    let ttf = common::fixture_ttf();
    let face = ttf_parser::Face::parse(&ttf, 0).expect("fixture parsable");
    assert_eq!(face.units_per_em(), 1000);
    assert_eq!(face.ascender(), 800);
    assert_eq!(face.descender(), -200);
    assert_eq!(face.line_gap(), 200);
}

#[test]
fn metrics_a_la_taille() {
    let ttf = common::fixture_ttf();
    let m = font::metrics(&ttf, 10.0).expect("métriques");
    assert!((m.ascent - 8.0).abs() < f32::EPSILON);
    assert!((m.descent + 2.0).abs() < f32::EPSILON);
    assert!((m.line_height() - 12.0).abs() < 0.01);
}

#[test]
fn shape_avance_reelle() {
    let ttf = common::fixture_ttf();
    let r = font::shape(&ttf, "abcd", 20.0, false).expect("shape");
    // 4 glyphes × 500/1000 × 20 px = 40 px.
    assert!((r.advance - 40.0).abs() < 0.01, "{}", r.advance);
    assert_eq!(r.glyphs.len(), 4);
    assert!(r.glyphs.iter().all(|g| g.id > 0));
    assert_eq!(r.glyphs[0].cluster, 0);
}

#[test]
fn mesure_et_clusters() {
    let ttf = common::fixture_ttf();
    assert!((font::measure(&ttf, "a b", 10.0) - 15.0).abs() < 0.01);
    // Fonte invalide → 0 (dégradation, pas de panic).
    assert_eq!(font::measure(&[0xDE, 0xAD], "x", 10.0), 0.0);
    assert!(font::metrics(&[], 10.0).is_none());
}

#[test]
fn itemize_ltr_simple() {
    let runs = text::itemize("hello world");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].direction, Direction::Ltr);
    assert_eq!(runs[0].text, "hello world");
    assert!(runs[0].breaks.iter().any(|b| b.offset == 6));
}

#[test]
fn itemize_bidi_melange() {
    // « שלום » est hébreu (RTL fort) → run de niveau impair.
    let runs = text::itemize("abc שלום def");
    assert!(runs.len() >= 3, "{runs:?}");
    assert_eq!(runs[0].direction, Direction::Ltr);
    let rtl = runs.iter().find(|r| r.direction == Direction::Rtl);
    assert!(rtl.is_some(), "un run RTL attendu : {runs:?}");
    assert_eq!(rtl.map(|r| r.text.trim()), Some("שלום"));
}

#[test]
fn itemize_vide_et_neutres() {
    assert_eq!(text::itemize(""), Vec::new());
    let runs = text::itemize("123 !!!");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].direction, Direction::Ltr);
}

#[test]
fn break_lines_wrap_greedy() {
    let ttf = common::fixture_ttf();
    let m = |s: &str| font::measure(&ttf, s, 10.0);
    // « ab cd ef » : chaque mot 10 px, espaces 5 px.
    let starts = text::break_lines("ab cd ef", 25.0, m);
    // « ab cd » tient (25 px pile), « ef » sur sa ligne.
    assert_eq!(starts, vec![0, 6]);
    let l2 = text::break_lines("ab cd ef", 15.0, m);
    assert_eq!(l2, vec![0, 3, 6]);
}

#[test]
fn break_lines_mot_geant_deborde() {
    // Mot sans opportunité interne : pas de coupure forcée
    // (overflow-wrap normal) — il reste entier et déborde.
    assert_eq!(
        text::break_lines("abcdef", 3.0, |s| s.len() as f32 * 10.0),
        vec![0]
    );
    // Mais un débordement couvre quand même l'opportunité
    // suivante : « treslongmot x » coupe avant « x ».
    assert_eq!(
        text::break_lines("treslongmot x", 3.0, |s| s.len() as f32 * 10.0),
        vec![0, 12]
    );
}

#[test]
fn break_lines_obligatoire() {
    let starts = text::break_lines("a\nb", 999.0, |s| s.len() as f32);
    assert_eq!(starts, vec![0, 2]);
}

#[test]
fn port_fontsource() {
    let ttf = common::fixture_ttf();
    let src = FixtureSource(ttf.clone());
    let ok = src.font_data(&FontRequest {
        families: vec!["Fixture".into()],
        weight: 400,
        italic: false,
    });
    assert_eq!(ok.as_deref(), Some(ttf.as_slice()));
    let ko = src.font_data(&FontRequest {
        families: vec!["Inconnue".into()],
        weight: 400,
        italic: false,
    });
    assert!(ko.is_none());
}

#[test]
fn cache_dedupe_font_data() {
    let src = CountingSource {
        ttf: common::fixture_ttf(),
        calls: std::cell::Cell::new(0),
    };
    let mut fc = FontCache::new();
    let req = fixture_req();
    // 10 résolutions de la même requête = 1 seul appel source.
    for _ in 0..10 {
        assert!(fc.data(&src, &req).is_some());
    }
    assert_eq!(src.calls.get(), 1);
    // Un miss est figé aussi : famille inconnue, 1 appel.
    let miss = FontRequest {
        families: vec!["Inconnue".into()],
        weight: 400,
        italic: false,
    };
    for _ in 0..5 {
        assert!(fc.data(&src, &miss).is_none());
    }
    assert_eq!(src.calls.get(), 2);
}

#[test]
fn cache_metrics_et_shape_coherents() {
    let src = CountingSource {
        ttf: common::fixture_ttf(),
        calls: std::cell::Cell::new(0),
    };
    let mut fc = FontCache::new();
    let req = fixture_req();
    // Métriques : un parse par (fonte, taille) ; la valeur
    // est identique au calcul direct sur les octets.
    let direct = font::metrics(&src.ttf, 10.0).expect("métriques");
    for _ in 0..3 {
        assert_eq!(fc.metrics(&src, &req, 10.0), Some(direct));
    }
    // Shape : même entrée → même run (l'`Arc` est partagé),
    // sans re-shaping.
    let direct_run = font::shape(&src.ttf, "abcd", 20.0, false).expect("shape");
    let first = fc
        .shape(&src, &req, "abcd", 20.0, false, 0.0)
        .expect("shape");
    assert_eq!(*first, direct_run);
    for _ in 0..3 {
        let hit = fc
            .shape(&src, &req, "abcd", 20.0, false, 0.0)
            .expect("shape");
        assert!(std::sync::Arc::ptr_eq(&hit, &first));
    }
    // Texte différent = entrée différente.
    let other = fc
        .shape(&src, &req, "xy", 20.0, false, 0.0)
        .expect("autre pièce");
    assert_ne!(*other, direct_run);
    // Une seule résolution de fonte pour tout.
    assert_eq!(src.calls.get(), 1);
}

/// R-78 — §16.4 : `letter-spacing` ajouté à l'avance de
/// chaque glyphe non nul (signe de l'avance) et présent
/// dans la clé de mémoïsation — deux `ls` distincts ne
/// partagent pas le run.
#[test]
fn letter_spacing_decale_les_avances_et_cle_le_cache() {
    let src = CountingSource {
        ttf: common::fixture_ttf(),
        calls: std::cell::Cell::new(0),
    };
    let mut fc = FontCache::new();
    let req = fixture_req();
    let base = fc.shape(&src, &req, "abcd", 20.0, false, 0.0).expect("run");
    let wide = fc
        .shape(&src, &req, "abcd", 20.0, false, 2.0)
        .expect("run ls");
    let n = base.glyphs.iter().filter(|g| g.advance != 0.0).count() as f32;
    assert!(
        (wide.advance - base.advance - 2.0 * n).abs() < 0.001,
        "advance ls={} base={} n={n}",
        wide.advance,
        base.advance
    );
    assert!(
        !std::sync::Arc::ptr_eq(&wide, &base),
        "ls doit faire partie de la clé"
    );
    // `ls` négatif : les avances se resserrent.
    let tight = fc
        .shape(&src, &req, "abcd", 20.0, false, -1.0)
        .expect("run -ls");
    assert!(
        tight.advance < base.advance,
        "tight={} base={} ptr_eq={}",
        tight.advance,
        base.advance,
        std::sync::Arc::ptr_eq(&tight, &base)
    );
}

/// R-65 : source à révision — un `None` figé doit être
/// invalidé quand la source mute (`@font-face` inscrit entre
/// deux passes d'un cache persistant, sinon la famille
/// resterait « inconnue » pour toujours).
struct MutableSource {
    ttf: Vec<u8>,
    rev: std::cell::Cell<u64>,
    live: std::cell::Cell<bool>,
    calls: std::cell::Cell<usize>,
}
impl FontSource for MutableSource {
    fn font_data(&self, req: &FontRequest) -> Option<Vec<u8>> {
        self.calls.set(self.calls.get() + 1);
        if self.live.get() && req.families.iter().any(|f| f == "Fixture") {
            Some(self.ttf.clone())
        } else {
            None
        }
    }
    fn revision(&self) -> u64 {
        self.rev.get()
    }
}

#[test]
fn cache_suit_la_revision_de_la_source() {
    let src = MutableSource {
        ttf: common::fixture_ttf(),
        rev: std::cell::Cell::new(0),
        live: std::cell::Cell::new(false),
        calls: std::cell::Cell::new(0),
    };
    let mut fc = FontCache::new();
    let req = fixture_req();
    // Révision 0, famille absente : miss figé, 1 appel.
    for _ in 0..3 {
        assert!(fc.data(&src, &req).is_none());
    }
    assert_eq!(src.calls.get(), 1);
    // La face arrive → revision bump : le cache se vide et
    // re-résout (plus de `None` périmé).
    src.live.set(true);
    src.rev.set(1);
    assert!(fc.data(&src, &req).is_some());
    assert_eq!(src.calls.get(), 2);
    // Révision stable : à nouveau mémoïsé, zéro appel.
    for _ in 0..3 {
        assert!(fc.data(&src, &req).is_some());
    }
    assert_eq!(src.calls.get(), 2);
    // Révocation (source vidée) : retour au miss.
    src.live.set(false);
    src.rev.set(2);
    assert!(fc.data(&src, &req).is_none());
    assert_eq!(src.calls.get(), 3);
}

#[test]
fn cache_tolere_fonte_invalide() {
    // Octets hostiles : ni métriques ni shape ne paniquent.
    struct Garbage;
    impl FontSource for Garbage {
        fn font_data(&self, _req: &FontRequest) -> Option<Vec<u8>> {
            Some(vec![0xDE, 0xAD, 0xBE, 0xEF])
        }
    }
    let src = Garbage;
    let mut fc = FontCache::new();
    let req = fixture_req();
    assert!(fc.data(&src, &req).is_some()); // octets servis
    assert!(fc.metrics(&src, &req, 10.0).is_none());
    assert!(fc.shape(&src, &req, "abc", 10.0, false, 0.0).is_none());
    // Miss re-joué : toujours None, jamais panic.
    assert!(fc.metrics(&src, &req, 12.0).is_none());
}
