// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! R-220 — `or` top-level dans une media query (MQ4 boolean)
//! `(a) or (b)` — MDN s'en sert pour ses breakpoints ;
//! sans lui la query tombait `unknown` → évaluée vraie par
//! tolérance → règles appliquées quand Chrome les rejette.

use explorer_css::{media_matches, parse_stylesheet, StyleEnv};

fn queries_of(q: &str) -> Vec<explorer_css::MediaQuery> {
    let css = format!("@media {q}{{a{{color:red}}}}");
    let sh = parse_stylesheet(&css);
    for r in sh.rules {
        if let explorer_css::Rule::At(a) = r {
            if let explorer_css::AtKind::Media { queries, .. } = a.kind {
                return queries;
            }
        }
    }
    Vec::new()
}

#[test]
fn or_top_level_parse_sans_unknown() {
    for q in [
        "(width <= 769px) or (width < calc(1rem))",
        "(hover:none) or (pointer:coarse)",
        "(min-width: 360px) or (max-width: 480px)",
    ] {
        let qs = queries_of(q);
        assert!(qs.iter().all(|m| !m.unknown), "unknown restée pour {q}");
        assert!(!qs[0].or_groups.is_empty(), "or_groups vide pour {q}");
    }
}

#[test]
fn or_semantique_ou() {
    // `(min-width:5000px) or (max-width:2000px)` : seul le
    // second groupe est vrai à 1366 → la query doit matcher.
    let qs = queries_of("(min-width:5000px) or (max-width:2000px)");
    let env = StyleEnv { width_px: 1366, height_px: 900, ..StyleEnv::default() };
    assert!(media_matches(&env, &qs), "or : aucun groupe reconnu");
    // Aucun groupe vrai → faux.
    let qs = queries_of("(min-width:5000px) or (max-width:200px)");
    assert!(!media_matches(&env, &qs), "or devrait être faux");
}

// R-225 — calc() dans les features MQ (wikipedia :
// `min-width:calc(640px - 1px)` masque `.mw-logo-icon`).
#[test]
fn calc_dans_feature_mq() {
    let qs = queries_of("screen and (min-width:calc(640px - 1px))");
    let env = StyleEnv { width_px: 1366, height_px: 900, ..StyleEnv::default() };
    assert!(media_matches(&env, &qs), "calc(640px-1px)=639 < 1366 doit matcher");
    let env = StyleEnv { width_px: 500, height_px: 900, ..StyleEnv::default() };
    assert!(!media_matches(&env, &qs), "500 < 639 ne doit pas matcher");
    let qs = queries_of("(max-width:calc(2 * 300px + (40px / 2)))");
    let env = StyleEnv { width_px: 600, height_px: 900, ..StyleEnv::default() };
    assert!(media_matches(&env, &qs), "620 = 2*300+20 doit matcher");
}


#[test]
fn calc_max_width_faux_desktop() {
    // (max-width:calc(640px - 1px)) à 1366 → FAUX —
    // sinon `.mw-logo-icon{display:block}`+`container{
    // max-width:none}` s'appliqueraient à tort (logo
    // wikipedia 261 vs 140).
    let qs = queries_of("(max-width:calc(640px - 1px))");
    let env = StyleEnv { width_px: 1366, height_px: 900, ..StyleEnv::default() };
    assert!(!media_matches(&env, &qs), "1366 > 639 → doit être faux");
    let env = StyleEnv { width_px: 500, height_px: 900, ..StyleEnv::default() };
    assert!(media_matches(&env, &qs));
}
