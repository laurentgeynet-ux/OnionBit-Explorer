// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Tests du census R-207 : les dégradations silencieuses du
//! parser/cascade produisent des clés de comptage pour le
//! tri « par lot de technologie » du corpus terrain.
//!
//! Le census est global au processus : chaque test arme → agit
//! → lit → réinitialise (Guard). Les assertions sont en `>=`
//! ou en présence de préfixe — un autre test armé en parallèle
//! ne peut qu'ajouter des clés, jamais en retirer.

use explorer_common::census;
use explorer_css::{Origin, StyleEngine, StyleEnv};
use explorer_dom::Dom;
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
fn proprietes_auteur_comptees_ua_silencieuse() {
    let _g = arm();
    let mut e = StyleEngine::new();
    e.add_css("a { color: red; --x: 1; --y: 2 }", Origin::Author);
    let s = census::snapshot();
    assert!(s.get("css.prop.color").copied().unwrap_or(0) >= 1);
    // Deux custom properties collapsées sous une seule clé.
    assert_eq!(s.get("css.prop.--custom").copied().unwrap_or(0), 2);
    // La feuille UA intégrée (origine UserAgent) ne produit
    // aucune clé css.prop — sinon le Pareto est du bruit.
    assert!(s.keys().all(|k| !k.starts_with("css.prop.display")));
    disarm();
}

#[test]
fn at_rules_sans_consommateur_signalees() {
    let _g = arm();
    let mut e = StyleEngine::new();
    e.add_css(
        "@tailwind utilities; @page { margin: 0 } @swizzle { a { opacity: 0 } } a { color: red }",
        Origin::Author,
    );
    let s = census::snapshot();
    // Statement non consommé (ni layer ni connu).
    assert!(s.get("css.at.unapplied.tailwind").copied().unwrap_or(0) >= 1);
    // Bloc de déclarations parsé mais inerte.
    assert!(s.get("css.at.unapplied.page").copied().unwrap_or(0) >= 1);
    // Bloc non structuré : parser + cascade le voient tous deux.
    assert!(s.get("css.at.unknown.swizzle").copied().unwrap_or(0) >= 1);
    assert!(s.get("css.at.unapplied.swizzle").copied().unwrap_or(0) >= 1);
    disarm();
}

#[test]
fn declaration_malformee_et_selector_bad() {
    let _g = arm();
    let mut e = StyleEngine::new();
    // `color red` sans `:` → récupération spec (decl.malformed) ;
    // `a >` = combinateur sans sujet → sélecteur non analysable.
    e.add_css("b { color red } a > { color: red }", Origin::Author);
    let s = census::snapshot();
    assert!(s.get("css.decl.malformed").copied().unwrap_or(0) >= 1);
    assert!(
        s.keys().any(|k| k.starts_with("css.selector.bad.")),
        "selector.bad attendu, clés vues: {:?}",
        s.keys().collect::<Vec<_>>()
    );
    disarm();
}

#[test]
fn valeur_invalide_dropped_et_mq_unknown() {
    let _g = arm();
    let mut e = StyleEngine::new();
    e.add_css(
        "@media screen sideways { div { color: red } } div { width: 200 }",
        Origin::Author,
    );
    let mut dom = Dom::new();
    let doc = dom.document();
    let body = dom.create_element("body").unwrap();
    dom.append_child(doc, body).unwrap();
    let div = dom.create_element("div").unwrap();
    dom.append_child(body, div).unwrap();
    let _ = e.compute(&dom, div, &StyleEnv::default());
    let s = census::snapshot();
    // `width: 200` sans unité → déclaration ignorée à la validation.
    assert!(s.get("css.value.dropped.width").copied().unwrap_or(0) >= 1);
    assert!(s.get("css.mq.unknown").copied().unwrap_or(0) >= 1);
    disarm();
}

#[test]
fn valeur_illisible_au_point_dusage() {
    let _g = arm();
    let mut dom = Dom::new();
    let doc = dom.document();
    let div = dom.create_element("div").unwrap();
    dom.set_attribute(
        div,
        "style",
        "letter-spacing: 0,5em; opacity: abc; z-index: auto",
    )
    .unwrap();
    dom.append_child(doc, div).unwrap();
    let eng = StyleEngine::new();
    let style = eng.compute(&dom, div, &StyleEnv::default());
    // Lecture typée au point d'usage — le funnel couvre tous
    // les consommateurs layout/render.
    let _ = style.length("letter-spacing");
    let _ = style.number("opacity");
    let _ = style.number("z-index");
    let s = census::snapshot();
    // `0,5em` (décimale virgule) et `abc` échouent à la lecture ;
    // `auto` est un mot-clé légal — filtré, jamais compté.
    assert!(
        s.get("css.value.unresolved.letter-spacing")
            .copied()
            .unwrap_or(0)
            >= 1,
        "clés: {s:?}"
    );
    assert!(s
        .keys()
        .all(|k| !k.starts_with("css.value.unresolved.z-index")));
    disarm();
}

#[test]
fn style_inline_compte_aussi() {
    let _g = arm();
    let e = StyleEngine::new();
    let mut dom = Dom::new();
    let doc = dom.document();
    let div = dom.create_element("div").unwrap();
    dom.set_attribute(div, "style", "letter-spacing: 2px")
        .unwrap();
    dom.append_child(doc, div).unwrap();
    let _ = e.compute(&dom, div, &StyleEnv::default());
    let s = census::snapshot();
    assert!(s.get("css.prop.letter-spacing").copied().unwrap_or(0) >= 1);
    disarm();
}
