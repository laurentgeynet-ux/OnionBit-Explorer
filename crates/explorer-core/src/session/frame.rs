// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Accès frames : PNG paresseux, pixels bruts, régions
//! dirty — la couture de pull (ADR-0004).
use super::*;

use explorer_render::Frame;

impl Session {
    /// PNG courant de l'onglet (`None` si jamais navigué) —
    /// encodé paresseusement depuis la frame raster puis
    /// caché jusqu'au prochain repaint (étape 55g : un
    /// burst de repaints entre deux lectures n'encode
    /// qu'une fois, pas par repaint).
    #[must_use]
    pub fn frame_png(&mut self, tab: TabId) -> Option<Vec<u8>> {
        self.ensure_painted(tab);
        let entry = self.tabs.get_mut(&tab)?;
        if entry.png.is_none() {
            entry.png = entry.frame.as_ref()?.encode_png();
        }
        entry.png.clone()
    }

    /// Auto-réparation (étape 56b rouvert, terrain mission
    /// 13) : un **DOM commis** sans produit peint — fenêtre
    /// `nav_apply` → `sub_commit`, `unspec` sur relayout
    /// impossible, tout chemin qui jette la frame — re-peint
    /// paresseusement à la première demande plutôt que de
    /// servir `/frame` 204 avec hit-test mort à vie. No-op
    /// si une frame existe ou si rien n'est commis ; si le
    /// DOM commis est réellement non-layable, le relayout
    /// échoue et l'appelant sert un 204 honnête.
    pub(crate) fn ensure_painted(&mut self, tab: TabId) {
        let needs = self
            .tabs
            .get(&tab)
            .is_some_and(|e| e.dom.is_some() && e.frame.is_none());
        if needs {
            let _ = self.relayout(tab);
        }
    }

    /// Frame brute courante `(largeur, hauteur, RGBA)` —
    /// `None` si jamais navigué. Canal FFI/texture (étape
    /// 47, ADR-0018) : les pixels restent empruntés, la
    /// copie est à la charge de l'appelant.
    #[must_use]
    pub fn frame_pixels(&self, tab: TabId) -> Option<(u32, u32, &[u8])> {
        self.tabs.get(&tab).and_then(|t| {
            t.frame
                .as_ref()
                .map(|f| (f.width, f.height, f.pixels.as_slice()))
        })
    }

    /// Extrait la région `(x, y, w, h)` — pixels physiques —
    /// de la frame courante (étape 55 : copie partielle des
    /// dirty-rects). Clippée aux bornes ; `None` si pas de
    /// frame ou région vide hors frame.
    #[must_use]
    pub fn frame_region(
        &self,
        tab: TabId,
        x: u32,
        y: u32,
        w: u32,
        h: u32,
    ) -> Option<(u32, u32, Vec<u8>)> {
        let frame = self.tabs.get(&tab)?.frame.as_ref()?;
        let rw = w.min(frame.width.saturating_sub(x));
        let rh = h.min(frame.height.saturating_sub(y));
        if rw == 0 || rh == 0 {
            return None;
        }
        let stride = frame.width as usize * 4;
        let row_bytes = rw as usize * 4;
        let mut out = Vec::with_capacity(row_bytes * rh as usize);
        for row in y..y + rh {
            let start = row as usize * stride + x as usize * 4;
            out.extend_from_slice(&frame.pixels[start..start + row_bytes]);
        }
        Some((rw, rh, out))
    }

    /// PNG de la région `(x, y, w, h)` — pixels physiques —
    /// (étape 55, pull partiel REST `frame?x=&y=&w=&h=`).
    /// `None` si pas de frame ou région hors bornes.
    #[must_use]
    pub fn frame_png_region(&self, tab: TabId, x: u32, y: u32, w: u32, h: u32) -> Option<Vec<u8>> {
        let (w2, h2, px) = self.frame_region(tab, x, y, w, h)?;
        Frame {
            width: w2,
            height: h2,
            pixels: px,
            scale: 1.0,
        }
        .encode_png()
    }

    /// Vide et renvoie les régions invalides accumulées —
    /// pixels physiques (étape 55). Canal de pull pour un
    /// client sans abonnement aux événements (FFI) : il
    /// copie ensuite chaque région via [`Self::frame_region`].
    /// Vide = rien à soumettre depuis le dernier appel.
    /// Indépendant des émissions (étape 55d — chaque
    /// `FrameInvalidated` n'avance que le watermark
    /// `emitted`). Limite connue : après un `scroll` les
    /// régions couvrent le document entier — le défilement
    /// est cuit dans les coordonnées raster, le pull
    /// partiel d'un scroll n'a pas de sens.
    pub fn take_dirty(&mut self, tab: TabId) -> Vec<FrameRegion> {
        let Some(entry) = self.tabs.get_mut(&tab) else {
            return Vec::new();
        };
        let s = self.scale.max(f32::EPSILON);
        entry.emitted = 0;
        entry
            .dirty
            .drain(..)
            .map(|r| FrameRegion {
                x: r.x * s,
                y: r.y * s,
                w: r.width * s,
                h: r.height * s,
            })
            .collect()
    }

    /// Émet `FrameInvalidated` avec le **delta** des régions
    /// invalides depuis l'émission précédente (étape 55d) —
    /// les rects internes sont en px CSS (`changed_rects`),
    /// le contrat sort en pixels physiques (`× scale`,
    /// comme `frame_pixels`). Watermark `emitted`, pas drain :
    /// le canal pull FFI (`take_dirty`) vide sa propre
    /// accumulation sans être affamé par les événements (le
    /// handle FFI garde un receveur inerte du bus).
    pub(crate) fn emit_invalidated(&mut self, tab: TabId) {
        let Some(entry) = self.tabs.get_mut(&tab) else {
            return;
        };
        let s = self.scale.max(f32::EPSILON);
        let rects = entry
            .dirty
            .get(entry.emitted.min(entry.dirty.len())..)
            .unwrap_or_default()
            .iter()
            .map(|r| FrameRegion {
                x: r.x * s,
                y: r.y * s,
                w: r.width * s,
                h: r.height * s,
            })
            .collect();
        entry.emitted = entry.dirty.len();
        // `scroll_y` de la frame peinte — l'hôte positionne
        // les overlays média à `rect − scroll_y` (contrat 71).
        let scroll_y = entry.scroll_y;
        let _ = self.events.send(CoreEvent::FrameInvalidated {
            tab,
            rects,
            scroll_y,
        });
    }
}
