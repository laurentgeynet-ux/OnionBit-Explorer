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

// ----------------------------------------------------------------
// Instrumentation alloc natif (§B-59)
// ----------------------------------------------------------------

/// Allocateur global instrumenté — compteurs d'alloc natif par
/// processus (§B-59 : « réservoirs plats vs RSS en hausse » ne
/// nomme pas le coupable ; `stats()` donne le cumul brut et
/// `alloc_delta` le delta depuis l'appel précédent du fil —
/// attribution par phase dans `sync_phase`, activée par
/// `EXPLORER_ALLOC_TRACE=1`). L'`unsafe` vit dans `stats-alloc`
/// (dep pure Rust autorisée — AGENTS §4.1).
#[global_allocator]
static TRACKED_ALLOC: &stats_alloc::StatsAlloc<std::alloc::System> =
    &stats_alloc::INSTRUMENTED_SYSTEM;

/// Compteurs courants de l'allocateur (allocs cumulées, octets
/// demandés cumulés). Pas de delta — voir [`alloc_delta`].
pub fn alloc_stats() -> stats_alloc::Stats {
    stats_alloc::INSTRUMENTED_SYSTEM.stats()
}

/// `true` si l'attribution par phase est demandée
/// (`EXPLORER_ALLOC_TRACE` non vide/non "0").
pub fn alloc_trace() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        std::env::var("EXPLORER_ALLOC_TRACE").is_ok_and(|v| !v.is_empty() && v != "0")
    })
}

thread_local! {
    /// Dernier instantané du fil — le delta entre deux
    /// `sync_phase` du même fil attribue la phase.
    static LAST_STATS: std::cell::Cell<stats_alloc::Stats> =
        std::cell::Cell::new(stats_alloc::Stats::default());
}

/// Delta d'alloc natif depuis l'appel précédent **de ce fil** —
/// `(octets demandés, nombre d'appels alloc)`. `None` si le
/// traceur est inactif (`EXPLORER_ALLOC_TRACE` non posé).
pub fn alloc_delta() -> Option<(usize, usize)> {
    if !alloc_trace() {
        return None;
    }
    let cur = alloc_stats();
    let prev = LAST_STATS.with(|c| c.replace(cur));
    Some((
        cur.bytes_allocated.saturating_sub(prev.bytes_allocated),
        cur.allocations.saturating_sub(prev.allocations),
    ))
}
