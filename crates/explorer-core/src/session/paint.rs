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

use explorer_net::http::Request;
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
    /// enfilent via [`Self::queue_apply_sync`] puis drainent hors
    /// verrou (étape 55h).
    pub(crate) fn apply_dom_sync(
        &mut self,
        tab: TabId,
        html: &str,
        dirty_path: Option<&[(String, u32)]>,
        relayout: bool,
        ids: Option<Vec<u64>>,
        tags: Option<Vec<String>>,
    ) {
        self.reclaim_orphaned_paint(tab);
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
            journal: None,
            ids: ids.unwrap_or_default(),
            tags: tags.unwrap_or_default(),
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
        // Surfaces média de la page posée (étape 71b) —
        // commit nav, restauration bfcache et page d'erreur
        // passent tous par ici ; `refresh` diffe et émet
        // `MediaOffered`/`MediaMoved`/`MediaGone`.
        self.refresh_media(tab);
        self.emit_invalidated(tab);
    }

    /// Page d'erreur **peinte par le moteur** (étape 69) :
    /// une nav qui échoue commet un document d'erreur
    /// (statut, URL, cause) au lieu de laisser la page
    /// précédente ou un vide. La barre montre l'URL tentée,
    /// `can_back` est préservé (historique intact), un
    /// reload retente la cible en `GET`.
    pub(crate) fn paint_error_page(
        &mut self,
        tab: TabId,
        req: &Request,
        push: bool,
        err: &CoreError,
    ) {
        let url = &req.url;
        let html = error_document(url, err);
        let mut tb = explorer_html::TreeBuilder::new();
        tb.push(&html);
        tb.finish();
        let dom = tb.into_dom();
        let prev = self.tabs.get(&tab).and_then(|e| e.title.clone());
        let title = Some("Erreur de chargement".to_string());
        // Purge interactive AVANT le relayout — les `find`
        // de la page précédente ne doivent pas cuire dans la
        // frame d'erreur (étape 75 : bande orange stale).
        let cap = self.bfcache_pages;
        {
            let Some(e) = self.tabs.get_mut(&tab) else {
                return;
            };
            e.popup = None;
            e.find = None;
            e.focused = None;
            e.focused_key = None;
            e.hovered = None;
            e.deferred_inputs.clear();
            e.spec_paint = false;
            // Étape 76 : la page quittée gèle à son index
            // avant que l'erreur ne la remplace — `back` la
            // restaure sans réseau, comme après une nav
            // réussie (une page d'erreur quittée ne gèle
            // jamais — sa traversée retente la requête).
            // Ni une page en vol (étape 84 : pas d'entrée,
            // pas de slot — geler empoisonnerait `hist_idx`).
            if !e.hist_pending && !e.hist.get(e.hist_idx).is_some_and(|h| h.failed) {
                e.freeze(cap);
            }
        }
        if self.relayout_dom(tab, &dom).is_err() {
            return;
        }
        {
            let Some(e) = self.tabs.get_mut(&tab) else {
                return;
            };
            e.title.clone_from(&title);
            e.url = Some(url.clone());
            // `last_req = None` → reload retombe sur la
            // requête `failed` de l'entrée — retente de la
            // requête tombée, pas d'un `GET` vide (étape 93).
            e.last_req = None;
            // Idem commit réussi : la file d'intentions
            // appartient au document mort — elle n'a pas à
            // tirer après l'atterrissage de la page d'erreur.
            e.pending_nav.clear();
            // Étape 76 : la navigation en échec prend son
            // entrée d'historique (Chrome/Firefox) — `back`
            // revient à la page précédente, `forward` et la
            // traversée **retentent** la cible (entrée
            // `failed` : jamais de restauration bfcache).
            if push {
                e.hist.truncate(e.hist_idx + 1);
                e.hist.push(HistEntry {
                    req: req.clone(),
                    failed: true,
                    same_doc: false,
                    state: None,
                });
                e.hist_idx = e.hist.len() - 1;
                // L'entrée `failed` résout la position (84) —
                // les files `pushState`/`replaceState`
                // différés (CAP-151) appartiennent au
                // document tombé : vidées.
                e.hist_pending = false;
                e.pending_push.clear();
                e.pending_replace = None;
            }
        }
        self.emit_navigated(tab, url, title, prev.as_deref());
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
        self.reclaim_orphaned_paint(tab);
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

    /// Relayout ciblé sur les éléments animés (étape 139 —
    /// la mesure profil `commit_lock` 33,5 s sur github
    /// attribuait le coût à un **reflow complet par tick
    /// d'animation**) : les ordinals `elem` des transitions/
    /// animations actives sont résolus en `NodeId` par
    /// `element_ordinals`, `journal_cover` pose
    /// `relayout_subtree` sur leur ancêtre commun — repli
    /// reflow complet si la cible n'est pas trouvable.
    pub(crate) fn relayout_animated(
        &mut self,
        tab: TabId,
        ords: &[usize],
    ) -> Result<(), CoreError> {
        self.reclaim_orphaned_paint(tab);
        let ctx = self.apply_ctx();
        let Some(entry) = self.tabs.get_mut(&tab) else {
            return Err(CoreError::UnknownTab(tab));
        };
        let canvas_ops = entry.js.as_deref_mut().map_or_else(
            explorer_js::CanvasOpsTable::new,
            explorer_js::JsEngine::canvas_ops,
        );
        let mut p = entry.take_paint();
        let Some(dom) = p.dom.take() else {
            entry.put_paint(p);
            return Err(CoreError::EmptyDocument);
        };
        let ord_to_node: std::collections::HashMap<usize, explorer_dom::NodeId> =
            anim_items::element_ordinals(&dom)
                .into_iter()
                .map(|(n, o)| (o, n))
                .collect();
        let touched: Vec<explorer_dom::NodeId> =
            ords.iter().filter_map(|o| ord_to_node.get(o).copied()).collect();
        let r = match apply::journal_cover(&dom, &touched) {
            Some(t) => apply::relayout_subtree_payload(&mut p, &ctx, &dom, t, &canvas_ops),
            None => apply::relayout_dom_payload(&mut p, &ctx, &dom, &canvas_ops),
        };
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
        let styles = apply::styles_payload(
            &mut p,
            dom,
            self.viewport,
            self.color_scheme,
            entry.js.is_some(),
        );
        entry.put_paint(p);
        styles
    }

    /// Pipeline style → layout → raster sur `dom` (emprunté —
    /// `relayout` pour le DOM posé, `nav_paint` pour le DOM
    /// en construction). Délègue à
    /// [`apply::relayout_dom_payload`] (étape 55h).
    pub(crate) fn relayout_dom(&mut self, tab: TabId, dom: &Dom) -> Result<(), CoreError> {
        self.reclaim_orphaned_paint(tab);
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
        self.reclaim_orphaned_paint(tab);
        let ctx = self.apply_ctx();
        let Some(entry) = self.tabs.get_mut(&tab) else {
            return Err(CoreError::UnknownTab(tab));
        };
        let mut p = entry.take_paint();
        let r = apply::repaint_payload(&mut p, &ctx, None);
        entry.put_paint(p);
        r
    }
}

#[cfg(test)]
use explorer_css::StyleMap;
use explorer_dom::Dom;
