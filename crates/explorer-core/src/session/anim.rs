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
    /// vsync propre, `rAF` vit côté `explorer-jscore`).
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
        // Pull consommateur (Étape 222) : `/frame` REST et le
        // pull FFI passent par ici — la lecture arme la fenêtre
        // `anim_pull_idle` des ticks de pompe (un headless qui
        // ne pull jamais ne paie aucun relayout anim).
        if let Some(e) = self.tabs.get_mut(&tab) {
            e.last_frame_pull = Some(Instant::now());
        }
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
    /// Progression `[0,1]` d'une lecture smooth démarrée à
    /// `start` (linéaire MVP — `smooth_scroll_ms` de config).
    pub(crate) fn smooth_progress(&self, start: std::time::Instant) -> f32 {
        let ms = self.smooth_scroll_ms;
        if ms == 0 {
            return 1.0;
        }
        #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
        // durées ms ≪ 2^24.
        let p = (start.elapsed().as_secs_f64() * 1000.0 / ms as f64) as f32;
        p
    }

    /// Tick `scroll-behavior:smooth` (R-259) : interpole la
    /// position et repeint à chaque battement — `true` tant
    /// que la lecture court (le battement reste armé).
    pub(crate) fn advance_smooth_scroll(&mut self, tab: TabId) -> Result<bool, CoreError> {
        let Some((from, to, start)) = self.tabs.get(&tab).and_then(|e| e.smooth_scroll) else {
            return Ok(false);
        };
        let p = self.smooth_progress(start).clamp(0.0, 1.0);
        let y = from + (to - from) * p;
        let done = p >= 1.0;
        {
            let entry = self.tabs.get_mut(&tab).ok_or(CoreError::UnknownTab(tab))?;
            if done {
                entry.smooth_scroll = None;
            }
            entry.set_scroll(if done { to } else { y });
        }
        if !self.tabs.get(&tab).is_some_and(|e| e.paint_detached) {
            self.repaint(tab)?;
            self.emit_invalidated(tab);
        }
        Ok(!done)
    }

    /// Tick anim **poussé par la pompe** (Étape 222 — gate
    /// consommateur) : ne relayoute que si un pull de frame a
    /// eu lieu dans `anim_pull_idle`. En vol headless (ou UI
    /// en pause) personne ne lit les frames produites — la
    /// production est sautée, l'état anim ne diverge pas :
    /// `advance_animations` interpole à `now` au prochain
    /// pull (`tick_animations`, non gaté — la frame servie
    /// est fraîche). `0` = jamais de tick pompe (pull seul).
    /// `advance_smooth_scroll` suit la même gate : sa cible
    /// est interpolée au temps courant, le prochain pull
    /// rattrape la lecture — un scroll smooth en vol n'est
    /// pas figé, seulement non peint entre-deux.
    pub(crate) fn pump_advance_animations(&mut self, tab: TabId) -> Result<bool, CoreError> {
        let watching = self.tabs.get(&tab).is_some_and(|e| {
            e.last_frame_pull
                .is_some_and(|t| t.elapsed() <= self.anim_pull_idle)
        });
        if !watching {
            return Ok(false);
        }
        self.advance_animations(tab)
    }

    pub(crate) fn advance_animations(&mut self, tab: TabId) -> Result<bool, CoreError> {
        let scrolling = self.advance_smooth_scroll(tab)?;
        let now = Instant::now();
        let (active, ords) = self.tabs.get(&tab).map_or((false, Vec::new()), |t| {
            let playing = !t.transitions.is_empty() || t.animations.iter().any(|a| a.playing(now));
            let mut o: Vec<usize> = t.transitions.iter().map(|tr| tr.elem).collect();
            o.extend(
                t.animations
                    .iter()
                    .filter(|a| a.playing(now))
                    .map(|a| a.elem),
            );
            (playing, o)
        });
        if !active {
            return Ok(scrolling);
        }
        match self.relayout_animated(tab, &ords) {
            Ok(()) => {
                self.emit_invalidated(tab);
                Ok(true)
            }
            // Peinture détachée pour un apply hors verrou
            // (étape 55h) — l'onglet est transitoirement sans
            // arbre : tick sauté, pas une erreur.
            Err(CoreError::EmptyDocument) => Ok(scrolling),
            Err(e) => Err(e),
        }
    }
}
