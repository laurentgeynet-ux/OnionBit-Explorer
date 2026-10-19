// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! `explorer-common` — types et utilitaires partagés, erreurs racines.
//!
//! Domaine pur — aucun I/O, aucune dépendance async.

#![forbid(unsafe_code)]

/// Marge de pile exigée avant une descente récursive (R-24).
/// Les frames des traversées domaine (boxtree, layout,
/// display list, hit-test, cascade) coûtent ~1–3 Kio —
/// 128 Kio couvrent la pire frame avant bascule, en phase
/// avec la convention JS-C41 (256 Kio pour des frames JS
/// mesurées 27–55 Kio).
const STACK_RED_ZONE: usize = 128 * 1024;

/// Taille du segment tas alloué par bascule (~500–1000
/// niveaux de traversée par segment ; `maybe_grow`
/// re-bascule à la profondeur suivante — profondeur
/// effectivement non bornée côté pile).
const STACK_GROW: usize = 1024 * 1024;

/// Exécute `f` en garantissant la marge pile (R-24).
///
/// Patron `stacker::maybe_grow` éprouvé par JS-C41 : si la
/// pile native entre en zone rouge, `f` tourne sur un
/// segment tas alloué pour l'occasion puis rendu. À placer
/// sur **chaque arête récursive** dont la profondeur suit la
/// profondeur DOM (un document hostile peut nicher des
/// milliers d'éléments — `repros/host-nest*.html`) : la
/// profondeur devient un coût *tas*, jamais un débordement.
/// Ne remplace pas les bornes logiques (`with_max_nodes`,
/// budgets de parse) — seulement le crash.
#[inline]
pub fn deep<R>(f: impl FnOnce() -> R) -> R {
    stacker::maybe_grow(STACK_RED_ZONE, STACK_GROW, f)
}
