// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Session navigateur (étape 24) : onglets, navigation,
//! pipeline `fetch → parse → style → layout → render → PNG`
//! et bus d'événements `frame_invalidated` (ADR-0004).
//!
//! Orchestre les couches domaine derrière des **ports
//! injectés** : `HttpTransport` pour le réseau, `FontSource`
//! pour les fontes — aucune socket ni fichier ici.
//! Single-writer : le DOM et le layout d'un onglet ne vivent
//! que dans cette structure, jamais partagés entre threads.

use std::collections::HashMap;
use std::sync::Arc;

use explorer_css::{Origin, StyleEngine, StyleEnv};
use explorer_dom::Dom;
use explorer_html::encoding::decode_sniffed;
use explorer_html::TreeBuilder;
use explorer_layout::font::FontSource;
use explorer_layout::{build_box_tree, layout_tree, BlockCtx, LayoutBox};
use explorer_net::http::{HttpTransport, Request};
use explorer_net::url::Url;
use explorer_render::{build_display_list, rasterize, DisplayList, Frame};
use tokio::sync::broadcast;

/// Identifiant d'onglet (monotone, jamais réutilisé).
pub type TabId = u64;

/// Événement de session poussé aux abonnés (SSE côté API).
#[derive(Debug, Clone)]
pub enum CoreEvent {
    /// Onglet créé.
    TabOpened {
        /// Nouvel onglet.
        tab: TabId,
    },
    /// Navigation aboutie (page posée + peinte).
    Navigated {
        /// Onglet concerné.
        tab: TabId,
        /// URL finale (après redirections).
        url: String,
    },
    /// La frame de l'onglet a changé — l'UI tire
    /// `GET /api/tabs/{id}/frame` (ADR-0004).
    FrameInvalidated {
        /// Onglet dont la frame est périmée.
        tab: TabId,
    },
    /// Erreur de pipeline (fetch, parse, alloc) — dégradation
    /// signalée, la session continue.
    PipelineError {
        /// Onglet concerné (`None` si hors onglet).
        tab: Option<TabId>,
        /// Message d'erreur.
        message: String,
    },
}

/// Erreur de session — typée, jamais de panic.
#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    /// URL malformée.
    #[error("url : {0}")]
    Url(String),
    /// Échec réseau.
    #[error("fetch : {0}")]
    Fetch(String),
    /// HTTP non-2xx.
    #[error("HTTP {0}")]
    HttpStatus(u16),
    /// Document sans élément racine.
    #[error("document sans élément")]
    EmptyDocument,
    /// Racine en `display:none` — rien à peindre.
    #[error("rien à peindre (display:none)")]
    NothingToPaint,
    /// Allocation raster refusée (page hostile).
    #[error("frame hors bornes")]
    FrameTooBig,
    /// Onglet inconnu.
    #[error("onglet inconnu : {0}")]
    UnknownTab(TabId),
}

/// État d'un onglet : pipeline complet gardé — le re-render
/// (`resize`) repart du `dom` sans re-fetch.
struct Tab {
    /// URL courante.
    url: Option<Url>,
    /// DOM parsé.
    dom: Option<Dom>,
    /// Arbre de boîtes posé.
    tree: Option<LayoutBox>,
    /// Display list courante.
    list: Option<DisplayList>,
    /// Frame rasterisée.
    frame: Option<Frame>,
    /// PNG encodé prêt à servir.
    png: Option<Vec<u8>>,
}

/// Configuration de session — seuils et viewport initiaux
/// (règle 13 : aucune valeur en dur dans le code).
#[derive(Debug, Clone, Copy)]
pub struct SessionConfig {
    /// Capacité du bus d'événements (au-delà, les abonnés
    /// lents sautent des événements — le pull PNG corrige).
    pub event_capacity: usize,
    /// Viewport CSS initial `(w, h)`.
    pub viewport: (f32, f32),
    /// Facteur DPI initial.
    pub scale: f32,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            event_capacity: 64,
            viewport: (1024.0, 768.0),
            scale: 1.0,
        }
    }
}

/// Session : transport + fontes injectés, onglets, viewport,
/// bus d'événements.
pub struct Session {
    transport: Arc<dyn HttpTransport>,
    fonts: Arc<dyn FontSource + Send + Sync>,
    tabs: HashMap<TabId, Tab>,
    events: broadcast::Sender<CoreEvent>,
    next_tab: TabId,
    /// Viewport CSS `(largeur, hauteur)` — partagé par les
    /// onglets au MVP.
    viewport: (f32, f32),
    /// Facteur DPI.
    scale: f32,
}

impl Session {
    /// Session vide : `transport` pour le fetch, `fonts`
    /// pour le texte — les deux injectés par la couche infra
    /// (`Send + Sync` exigé : la session est partagée entre
    /// tâches derrière l'API). Configuration par défaut.
    #[must_use]
    pub fn new(
        transport: Arc<dyn HttpTransport>,
        fonts: Arc<dyn FontSource + Send + Sync>,
    ) -> Self {
        Self::with_config(transport, fonts, SessionConfig::default())
    }

    /// Session avec seuils explicites.
    #[must_use]
    pub fn with_config(
        transport: Arc<dyn HttpTransport>,
        fonts: Arc<dyn FontSource + Send + Sync>,
        cfg: SessionConfig,
    ) -> Self {
        let (events, _) = broadcast::channel(cfg.event_capacity.max(1));
        Self {
            transport,
            fonts,
            tabs: HashMap::new(),
            events,
            next_tab: 1,
            viewport: cfg.viewport,
            scale: cfg.scale,
        }
    }

    /// Abonnement au bus d'événements (SSE côté API).
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<CoreEvent> {
        self.events.subscribe()
    }

    /// Viewport courant `(w, h)` en px CSS.
    #[must_use]
    pub fn viewport(&self) -> (f32, f32) {
        self.viewport
    }

    /// Change viewport + DPI ; invalide toutes les frames
    /// (le re-render des onglets chargés est différé à
    /// `rerender` — l'appelant pilote).
    pub fn set_viewport(&mut self, w: f32, h: f32, scale: f32) {
        if w > 0.0 && h > 0.0 && scale.is_finite() && scale > 0.0 {
            self.viewport = (w, h);
            self.scale = scale;
        }
    }

    /// Ouvre un onglet vide.
    pub fn open_tab(&mut self) -> TabId {
        let id = self.next_tab;
        self.next_tab += 1;
        self.tabs.insert(
            id,
            Tab {
                url: None,
                dom: None,
                tree: None,
                list: None,
                frame: None,
                png: None,
            },
        );
        let _ = self.events.send(CoreEvent::TabOpened { tab: id });
        id
    }

    /// Ferme l'onglet.
    ///
    /// # Errors
    /// [`CoreError::UnknownTab`] si l'onglet n'existe pas.
    pub fn close_tab(&mut self, tab: TabId) -> Result<(), CoreError> {
        self.tabs.remove(&tab).ok_or(CoreError::UnknownTab(tab))?;
        Ok(())
    }

    /// Onglets ouverts.
    #[must_use]
    pub fn tabs(&self) -> Vec<TabId> {
        self.tabs.keys().copied().collect()
    }

    /// URL courante de l'onglet.
    #[must_use]
    pub fn tab_url(&self, tab: TabId) -> Option<String> {
        self.tabs
            .get(&tab)
            .and_then(|t| t.url.as_ref().map(std::string::ToString::to_string))
    }

    /// PNG courant de l'onglet (`None` si jamais navigué).
    #[must_use]
    pub fn frame_png(&self, tab: TabId) -> Option<Vec<u8>> {
        self.tabs.get(&tab).and_then(|t| t.png.clone())
    }

    /// Navigue : fetch → decode → parse → style → layout →
    /// display list → raster → PNG, puis
    /// `Navigated` + `FrameInvalidated`.
    ///
    /// Toute étape qui échoue produit `PipelineError` sur le
    /// bus et une `Err` typée — la session reste saine.
    ///
    /// # Errors
    /// [`CoreError::Url`] (URL invalide), [`CoreError::Fetch`]
    /// (transport), [`CoreError::HttpStatus`] (non-2xx),
    /// [`CoreError::EmptyDocument`]/[`CoreError::NothingToPaint`]
    /// (page sans contenu peint), [`CoreError::FrameTooBig`]
    /// (allocation refusée), [`CoreError::UnknownTab`].
    pub async fn navigate(&mut self, tab: TabId, url: &str) -> Result<(), CoreError> {
        if !self.tabs.contains_key(&tab) {
            return Err(CoreError::UnknownTab(tab));
        }
        match self.navigate_inner(tab, url).await {
            Ok(final_url) => {
                let _ = self.events.send(CoreEvent::Navigated {
                    tab,
                    url: final_url.to_string(),
                });
                let _ = self.events.send(CoreEvent::FrameInvalidated { tab });
                Ok(())
            }
            Err(e) => {
                tracing::debug!(tab, error = %e, "pipeline en échec");
                let _ = self.events.send(CoreEvent::PipelineError {
                    tab: Some(tab),
                    message: e.to_string(),
                });
                Err(e)
            }
        }
    }

    /// Pipeline proprement dit — erreurs typées remontées.
    async fn navigate_inner(&mut self, tab: TabId, url: &str) -> Result<Url, CoreError> {
        let url = Url::parse(url).map_err(|e| CoreError::Url(e.to_string()))?;
        let resp = self
            .transport
            .fetch(&Request::get(url.clone()))
            .await
            .map_err(|e| CoreError::Fetch(e.to_string()))?;
        if !(200..300).contains(&resp.status) {
            return Err(CoreError::HttpStatus(resp.status));
        }
        let (html, _enc) = decode_sniffed(&resp.body);
        let dom = TreeBuilder::parse(&html);
        {
            let entry = self.tabs.get_mut(&tab).ok_or(CoreError::UnknownTab(tab))?;
            entry.url = Some(url.clone());
            entry.dom = Some(dom);
        }
        self.relayout(tab)?;
        Ok(url)
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
        let _ = self.events.send(CoreEvent::FrameInvalidated { tab });
        Ok(())
    }

    /// Style → boîtes → layout → display list → raster → PNG.
    fn relayout(&mut self, tab: TabId) -> Result<(), CoreError> {
        let entry = self.tabs.get_mut(&tab).ok_or(CoreError::UnknownTab(tab))?;
        let dom = entry.dom.as_mut().ok_or(CoreError::EmptyDocument)?;

        // Feuilles : inline `<style>` du document (les
        // feuilles liées sont l'étape « sous-ressources »).
        let mut eng = StyleEngine::new();
        eng.add_css(&collect_style_css(dom), Origin::Author);
        let styles = eng.compute_tree(dom, dom.document(), &StyleEnv::default());
        let Some(root_el) = dom
            .children(dom.document())
            .into_iter()
            .flatten()
            .find(|&c| dom.element(c).is_ok())
        else {
            return Err(CoreError::EmptyDocument);
        };
        let Some(mut bx) = build_box_tree(dom, root_el, &styles) else {
            return Err(CoreError::NothingToPaint);
        };
        let (vw, vh) = self.viewport;
        let ctx = BlockCtx {
            fonts: Some(self.fonts.as_ref() as &dyn FontSource),
            ..BlockCtx::default()
        };
        layout_tree(&mut bx, vw, &ctx);
        let list = build_display_list(&bx, vw, vh);
        let fonts = Arc::clone(&self.fonts);
        let frame = rasterize(&list, fonts.as_ref() as &dyn FontSource, self.scale)
            .ok_or(CoreError::FrameTooBig)?;
        let png = frame.encode_png();
        entry.tree = Some(bx);
        entry.list = Some(list);
        entry.frame = Some(frame);
        entry.png = png;
        Ok(())
    }
}

/// Concatène le contenu des `<style>` du document —
/// identique au helper du CLI (dupliqué tant que le pipeline
/// sous-ressources n'existe pas, étape 12).
fn collect_style_css(dom: &Dom) -> String {
    let mut css = String::new();
    let mut queue = vec![dom.document()];
    while let Some(n) = queue.pop() {
        if dom.tag_name(n).is_ok_and(|t| t == "style") {
            if let Ok(t) = dom.text_content(n) {
                css.push_str(&t);
                css.push('\n');
            }
        }
        if let Ok(ch) = dom.children(n) {
            queue.extend(ch.iter().rev());
        }
    }
    css
}
