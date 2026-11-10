// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Recensement des manques moteur (R-207) — compteurs globaux
//! des constructions rencontrées mais non honorées par le
//! pipeline, pour le pilotage « par lot de technologie » des
//! corrections terrain.
//!
//! Philosophie : le moteur **dégrade toujours** (tolérance
//! spec) — le census transforme chaque dégradation silencieuse
//! en signal mesurable. Agrégé sur un corpus de sites
//! (`site_harvest`), il produit le Pareto des technos manquantes
//! au lieu d'un triage humain site par site.
//!
//! **Désarmé par défaut** (convention J17_* : les sondes ne
//! coûtent qu'en mesure) — armer par `EXPLORER_CENSUS=1`
//! (processus, binaires/daemon) ou [`set_armed`] (bancs, tests,
//! futur endpoint API). Désarmé, chaque [`bump`] coûte un
//! `load` atomique relâché ; armé, un `Mutex<BTreeMap>` par
//! événement — acceptable en récolte, pas en usage normal.
//!
//! Espace de clés (préfixe par couche) :
//! - `css.prop.<name>` — propriété déclarée par l'auteur ;
//!   `css.prop.--*` = toutes les custom properties (cardinalité
//!   bornée). Le rapporteur croise avec la liste des propriétés
//!   consommées pour obtenir les manques.
//! - `css.value.dropped.<name>` — déclaration ignorée sur
//!   validation par propriété (`width: 200`…).
//! - `css.decl.malformed` — déclaration hors grammaire
//!   (récupération spec).
//! - `css.selector.bad.<prelude>` — prélude de règle non
//!   analysable (tronqué à `KEY_MAX`).
//! - `css.at.unknown.<name>` — at-rule à bloc non structurée ;
//!   `css.at.unapplied.<name>` — at-rule parsée sans
//!   consommateur (`@page`, `@counter-style`, `@tailwind`…).
//! - `css.mq.unknown` / `css.supports.unknown` — query non
//!   analysable (tolérée à `true`).
//! - `css.token.orphan` — token isolé avalé anti-boucle.
//! - `html.tokens` / `html.err.afe_reconstruct` /
//!   `html.err.foster_insert` / `html.err.foster_text` /
//!   `html.err.ignored_token` / `html.quirks.<mode>` —
//!   versés par `TreeBuilder::finish` depuis `TreeStats`.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(test)]
use std::sync::PoisonError;
use std::sync::{Mutex, OnceLock};

/// Longueur maximale d'une clé — les préludes/noms hostiles
/// sont tronqués, mémoire bornée par événement.
const KEY_MAX: usize = 64;

/// Cap de clés **distinctes** — au-delà, tout converge vers
/// `__overflow` : un site à 10⁵ idents uniques ne peut pas
/// gonfler la table (contenu hostile = la norme).
const KEYS_MAX: usize = 4096;

/// Armement explicite (tests, bancs, endpoint API à venir).
static ARMED: AtomicBool = AtomicBool::new(false);
/// Armement par environnement — lu une fois (`EXPLORER_CENSUS`
/// non vide et ≠ "0"), convention `EXPLORER_ALLOC_TRACE`.
static ENV: OnceLock<bool> = OnceLock::new();
/// Table globale — processus entier (un `shot` = un process ;
/// côté daemon multi-onglets, la session agrège par nav).
static MAP: Mutex<BTreeMap<String, u64>> = Mutex::new(BTreeMap::new());

/// Le census enregistre-t-il ? (`set_armed` **ou** env.)
#[must_use]
#[inline]
pub fn armed() -> bool {
    ARMED.load(Ordering::Relaxed)
        || *ENV.get_or_init(|| {
            std::env::var("EXPLORER_CENSUS").is_ok_and(|v| !v.is_empty() && v != "0")
        })
}

/// Arme/désarme explicitement — prioritaire sur l'env lu :
/// `set_armed(false)` ne coupe pas un `EXPLORER_CENSUS=1`
/// (l'env est une décision de lancement du processus).
pub fn set_armed(on: bool) {
    ARMED.store(on, Ordering::Relaxed);
}

/// Incrémente `key` de 1 — no-op si désarmé.
#[inline]
pub fn bump(key: &str) {
    bump_n(key, 1);
}

/// Incrémente `key` de `n` — no-op si désarmé ou `n == 0`.
pub fn bump_n(key: &str, n: u64) {
    if n == 0 || !armed() {
        return;
    }
    let key = normalize(key);
    let Ok(mut map) = MAP.lock() else {
        return;
    };
    let key = if map.len() >= KEYS_MAX && !map.contains_key(key.as_str()) {
        String::from("__overflow")
    } else {
        key
    };
    *map.entry(key).or_insert(0) += n;
}

/// Instantané clé → compteur (trié — rapport direct).
#[must_use]
pub fn snapshot() -> BTreeMap<String, u64> {
    MAP.lock().map(|m| m.clone()).unwrap_or_default()
}

/// Vide la table — entre deux navigations d'un même processus
/// (le shot one-shot n'en a pas besoin, le daemon si).
pub fn reset() {
    if let Ok(mut m) = MAP.lock() {
        m.clear();
    }
}

/// Clé normalisée : minuscules ASCII, `[a-z0-9._-]` gardés,
/// le reste en `_`, tronquée à `KEY_MAX`.
fn normalize(key: &str) -> String {
    let mut out = String::with_capacity(key.len().min(KEY_MAX));
    for c in key.trim().chars() {
        if out.len() >= KEY_MAX {
            break;
        }
        if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
            out.push(c.to_ascii_lowercase());
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        String::from("_")
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Le census est global au processus : les tests le
    /// traversant se sérialisent — un `reset` concurrent
    /// viderait la table lue par l'autre.
    static LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn disarmed_bump_is_noop() {
        let _l = LOCK.lock().unwrap_or_else(PoisonError::into_inner);
        // Sans env ni set_armed le bump ne remplit rien — sauf
        // si le process test tourne avec EXPLORER_CENSUS=1
        // (alors le compteur existe, assertion sautée).
        if armed() {
            return;
        }
        reset();
        bump("css.prop.width");
        assert!(snapshot().is_empty());
    }

    #[test]
    fn armed_counts_and_normalizes() {
        let _l = LOCK.lock().unwrap_or_else(PoisonError::into_inner);
        reset();
        set_armed(true);
        bump("CSS.PROP.Width");
        bump("css.prop.width");
        bump("css.at.unapplied.@tailwind ");
        bump_n("html.tokens", 42);
        let s = snapshot();
        set_armed(false);
        reset();
        assert_eq!(s.get("css.prop.width"), Some(&2));
        assert_eq!(s.get("css.at.unapplied._tailwind"), Some(&1));
        assert_eq!(s.get("html.tokens"), Some(&42));
    }

    #[test]
    fn long_keys_are_truncated() {
        let _l = LOCK.lock().unwrap_or_else(PoisonError::into_inner);
        reset();
        set_armed(true);
        bump(&format!("css.selector.bad.{}", "x".repeat(500)));
        let s = snapshot();
        set_armed(false);
        reset();
        let k = s.keys().next();
        assert!(k.is_some_and(|k| k.len() <= KEY_MAX));
    }
}
