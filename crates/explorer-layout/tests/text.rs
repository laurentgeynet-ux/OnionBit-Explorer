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

use explorer_layout::font::{self, FontRequest, FontSource};
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
