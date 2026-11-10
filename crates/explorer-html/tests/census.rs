// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Tests du census R-207 côté tree-builder : `TreeStats`
//! compte les dégradations de tolérance (foster parenting,
//! tokens ignorés, AFE) et `finish` les verse au census
//! global quand il est armé.

use explorer_common::census;
use explorer_html::TreeBuilder;
use std::sync::{Mutex, MutexGuard, PoisonError};

/// Le census est global au processus : les tests y écrivant
/// se sérialisent — un `reset` concurrent viderait la table
/// lue par un autre test. [`arm`] rend le verrou, tenu jusqu'à
/// la fin du test ; [`disarm`] le relâche manuellement.
static LOCK: Mutex<()> = Mutex::new(());

fn arm() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(PoisonError::into_inner);
    census::reset();
    census::set_armed(true);
    g
}

fn disarm() {
    census::set_armed(false);
    census::reset();
}

#[test]
fn stats_mesurent_le_degre_de_tag_soup() {
    let mut tb = TreeBuilder::new();
    // `text` directement sous <tr> → foster parenting ;
    // `</bogus>` = end tag sans correspondant → ignoré spec.
    tb.push("<table><tr>text</tr></table></bogus>");
    tb.finish();
    let s = tb.stats();
    assert!(s.tokens_processed >= 1);
    assert!(
        s.foster_texts + s.foster_inserts >= 1,
        "foster attendu: {s:?}"
    );
    assert!(s.ignored_tokens >= 1, "token ignoré attendu: {s:?}");
}

#[test]
fn finish_verse_au_census_arme() {
    let _g = arm();
    let _dom = TreeBuilder::parse("<table><tr>text</tr></table></bogus>");
    let s = census::snapshot();
    assert!(s.get("html.tokens").copied().unwrap_or(0) >= 1);
    assert!(
        s.keys().any(|k| k.starts_with("html.quirks.")),
        "mode quirks attendu, clés: {:?}",
        s.keys().collect::<Vec<_>>()
    );
    assert!(s.get("html.err.ignored_token").copied().unwrap_or(0) >= 1);
    assert!(s.get("html.err.foster_text").copied().unwrap_or(0) >= 1);
    disarm();
}
