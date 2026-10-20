// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Pipeline de peinture : cascade, layout, repaint, synchro
//! DOM incrémentale (R-12). Les corps vivent dans
//! [`super::apply`] depuis l'étape 55h — les méthodes ici
//! délèguent via take/put de la [`PaintPayload`] pour
//! servir indifféremment le chemin `&mut self` et le chemin
//! détaché `spawn_blocking`.
use super::*;

use explorer_net::url::Url;

impl Session {
    /// Resynchronise le document depuis le miroir JS muté :
    /// re-parse du HTML sérialisé, relecture du titre
    /// (`TitleChanged` si changé). `focused`/`hovered` sont
    /// conservés — ce sont des chemins, pas des `NodeId`
    /// (étape 52). `relayout` contrôle
    /// l'invalidation immédiate — vrai depuis les événements,
    /// faux pendant la navigation (qui relayout juste après).
    /// `dirty_path` = chemin du sous-arbre sale rapporté par
    /// le miroir (étape 55f) — sert à viser le relayout
    /// incrémental au lieu du reflow global.
    ///
    /// Chemin synchrone `&mut self` — les hôtes partagés
    /// enfilent via [`Self::queue_apply`] puis drainent hors
    /// verrou (étape 55h).
    pub(crate) fn apply_dom_sync(
        &mut self,
        tab: TabId,
        html: &str,
        dirty_path: Option<&[(String, u32)]>,
        relayout: bool,
    ) {
        let ctx = self.apply_ctx();
        let Some(entry) = self.tabs.get_mut(&tab) else {
            return;
        };
        let canvas_ops = entry.js.as_deref_mut().map_or_else(
            explorer_js::CanvasOpsTable::new,
            explorer_js::JsEngine::canvas_ops,
        );
        let old_title = entry.title.clone();
        let mut p = entry.take_paint();
        let job = apply::PendingApply {
            tab,
            epoch: entry.nav_epoch,
            html: html.to_string(),
            dirty: dirty_path.map(<[_]>::to_vec),
            canvas_ops,
            relayout,
        };
        let out = apply::apply_payload(&mut p, &ctx, &job, old_title.as_deref());
        if let Some(entry) = self.tabs.get_mut(&tab) {
            entry.put_paint(p);
        }
        self.commit_apply(tab, &out);
    }

    /// `Navigated` + `TitleChanged` (si le titre change) +
    /// `FrameInvalidated` — séquence d'événements post-rendu.
    /// `prev` = titre avant navigation (capturé par l'appelant,
    /// `navigate_to` l'ayant déjà écrasé).
    pub(crate) fn emit_navigated(
        &mut self,
        tab: TabId,
        url: &Url,
        title: Option<String>,
        prev: Option<&str>,
    ) {
        let _ = self.events.send(CoreEvent::Navigated {
            tab,
            url: url.to_string(),
            title: title.clone(),
        });
        if let Some(t) = title.filter(|t| prev != Some(t.as_str())) {
            let _ = self.events.send(CoreEvent::TitleChanged { tab, title: t });
        }
        self.emit_invalidated(tab);
    }

    /// Log + `PipelineError` sur le bus, renvoie l'erreur.
    pub(crate) fn emit_error(&mut self, tab: TabId, e: CoreError) -> CoreError {
        tracing::debug!(tab, error = %e, "pipeline en échec");
        let _ = self.events.send(CoreEvent::PipelineError {
            tab: Some(tab),
            message: e.to_string(),
        });
        e
    }

    /// Re-pose l'arbre et re-peint depuis l'état courant
    /// (resize, style muté) — sans re-fetch.
    ///
    /// # Errors
    /// [`CoreError::UnknownTab`] ou [`CoreError::EmptyDocument`]
    /// si l'onglet n'a jamais été navigué.
    pub fn rerender(&mut self, tab: TabId) -> Result<(), CoreError> {
        if !self.tabs.contains_key(&tab) {
            return Err(CoreError::UnknownTab(tab));
        }
        self.relayout(tab)?;
        self.emit_invalidated(tab);
        Ok(())
    }

    /// Style → boîtes → layout → display list → raster → PNG.
    /// Délègue à [`apply::relayout_dom_payload`] via
    /// take/put (étape 55h) — `EmptyDocument` si la peinture
    /// est détachée pour un apply hors verrou.
    pub(crate) fn relayout(&mut self, tab: TabId) -> Result<(), CoreError> {
        let ctx = self.apply_ctx();
        let Some(entry) = self.tabs.get_mut(&tab) else {
            return Err(CoreError::UnknownTab(tab));
        };
        let canvas_ops = entry.js.as_deref_mut().map_or_else(
            explorer_js::CanvasOpsTable::new,
            explorer_js::JsEngine::canvas_ops,
        );
        // Le DOM sort de la payload le temps du pipeline —
        // `relayout_dom_payload` veut `&mut p` et un `&Dom`
        // disjoint.
        let mut p = entry.take_paint();
        let Some(dom) = p.dom.take() else {
            entry.put_paint(p);
            return Err(CoreError::EmptyDocument);
        };
        let r = apply::relayout_dom_payload(&mut p, &ctx, &dom, &canvas_ops);
        p.dom = Some(dom);
        if let Some(entry) = self.tabs.get_mut(&tab) {
            entry.put_paint(p);
        }
        r
    }

    /// Passe de styles partagée par le reflow complet et le
    /// relayout incrémental (étape 55f) : cascade `<style>` +
    /// `<link>`, calcul sur tout `dom`, surcharge des
    /// transitions en vol. Délègue à
    /// [`apply::styles_payload`] (étape 55h). Test/profil
    /// uniquement — les chemins livrés passent par
    /// `styles_payload` directement.
    #[cfg(test)]
    pub(crate) fn compute_styles(&mut self, tab: TabId, dom: &Dom) -> StyleMap {
        let Some(entry) = self.tabs.get_mut(&tab) else {
            return StyleMap::default();
        };
        let mut p = entry.take_paint();
        let styles = apply::styles_payload(&mut p, dom);
        entry.put_paint(p);
        styles
    }

    /// Pipeline style → layout → raster sur `dom` (emprunté —
    /// `relayout` pour le DOM posé, `nav_paint` pour le DOM
    /// en construction). Délègue à
    /// [`apply::relayout_dom_payload`] (étape 55h).
    pub(crate) fn relayout_dom(&mut self, tab: TabId, dom: &Dom) -> Result<(), CoreError> {
        let ctx = self.apply_ctx();
        let Some(entry) = self.tabs.get_mut(&tab) else {
            return Err(CoreError::UnknownTab(tab));
        };
        let canvas_ops = entry.js.as_deref_mut().map_or_else(
            explorer_js::CanvasOpsTable::new,
            explorer_js::JsEngine::canvas_ops,
        );
        let mut p = entry.take_paint();
        let r = apply::relayout_dom_payload(&mut p, &ctx, dom, &canvas_ops);
        entry.put_paint(p);
        r
    }

    /// Display list + raster + PNG depuis l'arbre posé —
    /// sans re-layout (scroll, invalidation partielle).
    /// Délègue à [`apply::repaint_payload`] (étape 55h).
    pub(crate) fn repaint(&mut self, tab: TabId) -> Result<(), CoreError> {
        let ctx = self.apply_ctx();
        let Some(entry) = self.tabs.get_mut(&tab) else {
            return Err(CoreError::UnknownTab(tab));
        };
        let mut p = entry.take_paint();
        let r = apply::repaint_payload(&mut p, &ctx);
        entry.put_paint(p);
        r
    }
}

#[cfg(test)]
use explorer_css::StyleMap;
use explorer_dom::Dom;
