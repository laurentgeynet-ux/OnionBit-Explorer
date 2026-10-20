// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Avance des animations d'un onglet (`impl Session`).
use super::*;
use std::time::Instant;

impl Session {
    /// Fait avancer les transitions CSS en vol (étape 45) et
    /// les animations `@keyframes` (étape 55e) : relayout +
    /// repaint si l'une est active — l'animation est pilotée
    /// par le pull de frames de l'UI **et** le battement
    /// d'hôte ([`Self::pump_timers_all`], sinon elle resterait
    /// figée entre deux pulls ; le moteur n'a pas de boucle
    /// vsync propre, `rAF` vit côté `dillo-js`).
    /// `Ok(true)` = une nouvelle frame a été produite et
    /// `FrameInvalidated` émis.
    ///
    /// Étape 52 : fait d'abord office de **tick JS** —
    /// [`Self::pump_timers`] vide les timers échus (borné par
    /// `max_timer_jobs`) ; leurs mutations synchronisées
    /// invalident la frame elles-mêmes. Les intentions de
    /// navigation émises restent à drainer par l'hôte
    /// ([`Self::drain_script_nav`], async — non faisable ici,
    /// le tick est synchrone).
    ///
    /// # Errors
    /// [`CoreError::UnknownTab`], ou erreur de rendu.
    pub fn tick_animations(&mut self, tab: TabId) -> Result<bool, CoreError> {
        let jobs = self.pump_timers(tab);
        if !self.advance_animations(tab)? {
            // Les timers ont pu invalider seuls (synchro DOM)
            // — pas de relayout gratuit en plus.
            return Ok(jobs > 0);
        }
        Ok(true)
    }

    /// Relayout + repaint si l'onglet a une transition ou une
    /// animation `@keyframes` **en lecture** (`phase` encore
    /// dans sa fenêtre — les terminées `forwards` ne coûtent
    /// plus un reflow). Partagé par [`Self::tick_animations`]
    /// (pull UI) et [`Self::pump_timers_all`] (battement hôte,
    /// étape 55e). `Ok(true)` = frame produite + émise.
    pub(crate) fn advance_animations(&mut self, tab: TabId) -> Result<bool, CoreError> {
        let now = Instant::now();
        let active = self.tabs.get(&tab).is_some_and(|t| {
            !t.transitions.is_empty() || t.animations.iter().any(|a| a.playing(now))
        });
        if !active {
            return Ok(false);
        }
        match self.relayout(tab) {
            Ok(()) => {
                self.emit_invalidated(tab);
                Ok(true)
            }
            // Peinture détachée pour un apply hors verrou
            // (étape 55h) — l'onglet est transitoirement sans
            // arbre : tick sauté, pas une erreur.
            Err(CoreError::EmptyDocument) => Ok(false),
            Err(e) => Err(e),
        }
    }
}
