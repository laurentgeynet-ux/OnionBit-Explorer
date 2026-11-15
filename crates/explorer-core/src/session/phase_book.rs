// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Livre de phases `sync_profile` requêrable (étape 250 — EAP
//! `engine.pipeline{phases}`) : `sync_phase` enregistre à
//! chaque borne `n`/`us_total`/`us_last`/`us_max` + deltas
//! d'allocation **process** (`stats_alloc` est global — le
//! delta inclut les allocations concurrentes d'autres tâches,
//! il mesure « ce que le process a alloué pendant la phase »,
//! pas une attribution stricte).
//!
//! Registre global clé par `TabId` plutôt qu'un livre porté
//! par `PaintPayload` : les ~60 bornes vivent sur deux mondes
//! (apply détaché en `spawn_blocking`, phases sous verrou
//! net/js/shared) — un livre par payload n'aurait couvert que
//! le premier. Section critique ~dizaines de ns (une entrée
//! `BTreeMap` + six compteurs), contention nulle en pratique.
//! Purge à `close_tab`, remise à zéro par
//! `engine.pipeline{reset:true}` — sémantique « take » : la
//! réponse porte l'intervalle qui vient de se clore.

use std::collections::BTreeMap;
use std::sync::Mutex;

use super::types::TabId;

/// Accumulateur d'une phase — `alloc_*` = allocations du
/// **process entier** mesurées pendant la phase (deltas de
/// l'allocateur `stats_alloc` global, attribution incluse).
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct PhaseAcc {
    /// Nombre de bornes enregistrées.
    pub n: u64,
    /// Durée cumulée (µs).
    pub us_total: u64,
    /// Durée de la dernière borne (µs).
    pub us_last: u64,
    /// Pire borne observée (µs).
    pub us_max: u64,
    /// Octets alloués process pendant les bornes (cumul).
    pub alloc_b_total: u64,
    /// Appels alloc process pendant les bornes (cumul).
    pub alloc_n_total: u64,
}

/// Livre d'un onglet — `&'static str` : toutes les bornes
/// portent un littéral de phase, zéro allocation de clé.
type PhaseBook = BTreeMap<&'static str, PhaseAcc>;

static BOOKS: Mutex<BTreeMap<TabId, PhaseBook>> = Mutex::new(BTreeMap::new());

/// Garde tolérante au poison — un panic dans une borne de
/// phase ne doit pas aveugler l'introspection.
fn books() -> std::sync::MutexGuard<'static, BTreeMap<TabId, PhaseBook>> {
    BOOKS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Enregistre une borne de phase — appelé par `sync_phase`.
pub(crate) fn record(tab: TabId, phase: &'static str, us: u64, alloc_b: u64, alloc_n: u64) {
    let mut g = books();
    let acc = g.entry(tab).or_default().entry(phase).or_default();
    acc.n += 1;
    acc.us_total = acc.us_total.saturating_add(us);
    acc.us_last = us;
    acc.us_max = acc.us_max.max(us);
    acc.alloc_b_total = acc.alloc_b_total.saturating_add(alloc_b);
    acc.alloc_n_total = acc.alloc_n_total.saturating_add(alloc_n);
}

/// Sérialisation JSON du livre (`{}` si jamais touché) —
/// noms contractuels EAP : `alloc_b`/`alloc_n` = allocs
/// **process** pendant la phase, pas une attribution stricte.
pub(crate) fn snapshot(tab: TabId) -> serde_json::Value {
    let phases: serde_json::Map<String, serde_json::Value> = books()
        .get(&tab)
        .map(|book| {
            book.iter()
                .map(|(name, a)| {
                    (
                        (*name).to_string(),
                        serde_json::json!({
                            "n": a.n,
                            "us_total": a.us_total,
                            "us_last": a.us_last,
                            "us_max": a.us_max,
                            "alloc_b": a.alloc_b_total,
                            "alloc_n": a.alloc_n_total,
                        }),
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    serde_json::Value::Object(phases)
}

/// Purge le livre d'un onglet — fermeture d'onglet et reset
/// « take » d'`engine.pipeline` (même effet : repartir à
/// zéro, `snapshot` rend `{}` ensuite).
pub(crate) fn purge(tab: TabId) {
    books().remove(&tab);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_accumule_et_snapshot_rend_json() {
        let tab = 9_000_001_u64;
        purge(tab);
        record(tab, "test.phase", 100, 10, 1);
        record(tab, "test.phase", 300, 20, 2);
        record(tab, "test.autre", 50, 5, 1);
        let s = snapshot(tab);
        let acc = &s["test.phase"];
        assert_eq!(acc["n"], 2);
        assert_eq!(acc["us_total"], 400);
        assert_eq!(acc["us_last"], 300);
        assert_eq!(acc["us_max"], 300);
        assert_eq!(acc["alloc_b"], 30);
        assert_eq!(acc["alloc_n"], 3);
        assert_eq!(s["test.autre"]["n"], 1);
        purge(tab);
        assert_eq!(snapshot(tab), serde_json::json!({}));
    }
}
