// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

use explorer_css::{Origin, StyleEngine, StyleEnv};
use explorer_dom::Dom;

/// Le shorthand `font` sans `/lh` réinitialise `line-height`
/// à `normal` (CSS Fonts §3.6) — sinon un `line-height`
/// hérité survivrait à travers le raccourci
/// (WPT `margin-collapse-020`).
#[test]
fn font_shorthand_resets_line_height() {
    let mut dom = Dom::new();
    let doc = dom.document();
    let html = dom.create_element("html").unwrap();
    let p = dom.create_element("p").unwrap();
    dom.append_child(doc, html).unwrap();
    dom.append_child(html, p).unwrap();
    let mut e = StyleEngine::new();
    e.add_css("html { line-height: 1em }", Origin::Author);
    e.add_css("p { font: 12pt serif }", Origin::Author);
    let s = e.compute(&dom, p, &StyleEnv::default());
    assert_eq!(s.get("line-height"), Some("normal"));
    assert_eq!(s.get("font-size"), Some("16px"));
    assert_eq!(s.get("font-family"), Some("serif"));
    assert_eq!(s.get("font-weight"), Some("normal"));
}

/// Avec `/lh` explicite, le shorthand le conserve.
#[test]
fn font_shorthand_keeps_slash_line_height() {
    let mut dom = Dom::new();
    let doc = dom.document();
    let div = dom.create_element("div").unwrap();
    dom.append_child(doc, div).unwrap();
    let mut e = StyleEngine::new();
    e.add_css("div { font: 1em/1.25 serif }", Origin::Author);
    let s = e.compute(&dom, div, &StyleEnv::default());
    assert_eq!(s.get("line-height"), Some("1.25"));
}

/// R-209 — `font_faces` relègue les faces dont `unicode-range`
/// exclut l'ASCII en fin de liste : sous budget de fetch borné,
/// les feuilles éclatées par sous-ensemble d'écriture (Google
/// Fonts : latin en dernier) faisaient enregistrer des faces
/// sans couverture latine — le texte tombait en fallback par
/// glyphe (vol 771 plyr.io, tout en serif/italique de repli).
#[test]
fn r209_font_faces_unicode_range_latin_prioritaire() {
    let mut e = StyleEngine::new();
    // Ordre « Google » : sous-ensembles non-latins d'abord,
    // latin en dernier.
    e.add_css(
        "@font-face{font-family:Inter;font-weight:400;font-style:italic;\
         src:url(i-it-cyr.ttf);unicode-range:U+0400-045F}\
         @font-face{font-family:Inter;font-weight:700;font-style:normal;\
         src:url(i-n-gr.ttf);unicode-range:U+0370-03FF}\
         @font-face{font-family:Inter;font-weight:400;font-style:normal;\
         src:url(i-n-lat.ttf);unicode-range:U+0000-00FF}\
         @font-face{font-family:Inter;font-weight:700;font-style:normal;\
         src:url(i-n-lat7.ttf);unicode-range:U+0000-00FF}\
         @font-face{font-family:Solo;src:url(s.ttf)}",
        Origin::Author,
    );
    let faces = e.font_faces(&StyleEnv::default());
    assert_eq!(faces.len(), 5);
    // Unicode-range éclatée + joker.
    assert_ne!(faces[0].unicode_ranges.len(), 0);
    // Les faces couvrant 'A' (latin + universelle) passent
    // devant, ordre relatif conservé.
    assert_eq!(faces[0].unicode_ranges, vec![(0x0000, 0x00FF)]);
    assert_eq!(faces[1].unicode_ranges, vec![(0x0000, 0x00FF)]);
    assert!(faces[2].unicode_ranges.is_empty(), "Solo universelle");
    assert_eq!(faces[3].unicode_ranges, vec![(0x0400, 0x045F)]);
    assert_eq!(faces[4].unicode_ranges, vec![(0x0370, 0x03FF)]);
}
