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
use std::time::{Duration, Instant};

use explorer_css::transition::{interpolate, transition_decls, Timing};
use explorer_css::{
    initial_value, ComputedStyle, FontSrc, Origin, StyleEngine, StyleEnv, StyleMap,
};
use explorer_dom::{Dom, DomError, NodeId};
use explorer_html::encoding::decode_sniffed;
use explorer_html::TreeBuilder;
use explorer_js::{JsEngine, JsFactory, NavRequest};
use explorer_layout::font::FontSource;
use explorer_layout::{build_box_tree_sized, layout_tree, BlockCtx, LayoutBox};
use explorer_net::http::{HttpTransport, Method, Request};
use explorer_net::url::Url;
use explorer_render::{
    build_display_list_imaged, decode_font, decode_image, rasterize, DisplayList, Frame,
    ImageTable, WebFontSource,
};
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
        /// Titre de la page (`None` si absent).
        title: Option<String>,
    },
    /// Onglet fermé.
    TabClosed {
        /// Onglet fermé.
        tab: TabId,
    },
    /// Le `<title>` de la page a changé.
    TitleChanged {
        /// Onglet concerné.
        tab: TabId,
        /// Nouveau titre.
        title: String,
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

/// Événement d'entrée côté UI — coordonnées en px CSS du
/// viewport (le scroll est compensé par le hit-test).
#[derive(Debug, Clone)]
pub enum InputEvent {
    /// Clic principal à `(x, y)`.
    Click {
        /// Position viewport.
        x: f32,
        /// Position viewport.
        y: f32,
    },
    /// Survol à `(x, y)` — coalescé côté client (ADR-0003).
    MouseMove {
        /// Position viewport.
        x: f32,
        /// Position viewport.
        y: f32,
    },
    /// Scroll vertical (delta px CSS).
    Scroll {
        /// Delta (positif = vers le bas).
        dy: f32,
    },
    /// Touche clavier — éditée si un champ est focalisé.
    Key {
        /// Identifiant de touche (`"a"`, `"Enter"`…).
        key: String,
    },
}

/// Résultat d'un événement d'entrée.
#[derive(Debug, Clone, PartialEq)]
pub enum InputOutcome {
    /// Rien à signaler.
    Ignored,
    /// Scroll appliqué → `scroll_y` résultant.
    Scrolled {
        /// Décalage courant.
        scroll_y: f32,
    },
    /// Un lien a été suivi (navigation déclenchée).
    Navigated,
    /// Survol : URL du lien sous le curseur, s'il y en a un
    /// (le client choisit le curseur à afficher).
    Hover {
        /// `href` résolu, ou `None` hors lien.
        url: Option<String>,
        /// `true` si le point est dans un champ éditable
        /// (curseur texte côté client).
        editable: bool,
    },
    /// Champ de formulaire focalisé (`input`/`textarea`).
    Focused,
    /// Le champ focalisé a été édité (frame re-peinte).
    Edited,
}

/// Vue publique d'un onglet pour l'API (DTO de sortie —
/// pas de référence interne exposée).
#[derive(Debug, Clone)]
pub struct TabInfo {
    /// Identifiant.
    pub id: TabId,
    /// URL courante (`None` si jamais navigué).
    pub url: Option<String>,
    /// Titre de la page.
    pub title: Option<String>,
    /// `back` possible.
    pub can_back: bool,
    /// `forward` possible.
    pub can_forward: bool,
}

/// Signet utilisateur.
#[derive(Debug, Clone, PartialEq)]
pub struct Bookmark {
    /// URL enregistrée.
    pub url: String,
    /// Titre au moment de l'enregistrement.
    pub title: Option<String>,
}

/// Entrée du journal de visites (historique global).
#[derive(Debug, Clone)]
pub struct Visit {
    /// URL visitée.
    pub url: String,
    /// Titre de la page.
    pub title: Option<String>,
}

/// Port de persistance du profil — implémenté en couche
/// infra (`JsonFileStore` du daemon, fichier dans le
/// répertoire de profil). Injecté, jamais d'I/O ici.
pub trait ProfileStore: Send + Sync {
    /// Enregistre une navigation aboutie (append borné).
    fn record_visit(&self, url: &str, title: Option<&str>);
    /// Charge les signets au démarrage.
    fn load_bookmarks(&self) -> Vec<Bookmark>;
    /// Persiste la liste complète des signets.
    fn save_bookmarks(&self, b: &[Bookmark]);
    /// Journal des visites (le plus récent en tête ou en
    /// queue — choix de l'impl).
    fn history(&self) -> Vec<Visit>;
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
    /// Mutation DOM refusée (nœud mort, arène pleine…).
    #[error("dom : {0}")]
    Dom(#[from] explorer_dom::DomError),
}

/// État d'un onglet : pipeline complet gardé — le re-render
/// (`resize`) repart du `dom` sans re-fetch.
struct Tab {
    /// URL courante.
    url: Option<Url>,
    /// Titre de la page (`<title>`).
    title: Option<String>,
    /// Historique local : URLs visitées + index courant
    /// (`hist[..=hist_idx]`, le futur est tronqué à la
    /// navigation — modèle navigateur classique).
    hist: Vec<Url>,
    /// Position dans `hist`.
    hist_idx: usize,
    /// Décalage de scroll (px CSS).
    scroll_y: f32,
    /// Hauteur du document posé — borne max du scroll.
    doc_h: f32,
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
    /// Champ de formulaire focalisé (`input`/`textarea`) —
    /// `NodeId` dans `dom`, invalidé à la navigation.
    focused: Option<NodeId>,
    /// Contexte JS du document — créé si la page a des
    /// `<script>` et qu'une fabrique est injectée ; meurt à
    /// la navigation (un contexte par document).
    js: Option<Box<dyn JsEngine>>,
    /// Pixels `<img>` décodés, indexés par `NodeId` — vidée à
    /// chaque re-parse (les ids changent), les `<img>` hors
    /// map retombent sur leur texte `alt`.
    images: ImageTable,
    /// Feuilles `<link rel=stylesheet>` fetchées à la
    /// navigation — concaténées, vidées à chaque re-parse
    /// (étape 46 : premières sous-ressources CSS).
    linked_css: String,
    /// Styles calculés de la dernière mise en page, indexés
    /// par **ordinal d'élément** (ordre document) — la clé
    /// survit au re-parse des synchronisations DOM, là où un
    /// `NodeId` est régénéré (étape 45).
    prev_styles: HashMap<usize, ComputedStyle>,
    /// Transitions en vol (`elem` ordinal + `prop` →
    /// horaire) — bornées par `MAX_TRANSITIONS`.
    transitions: Vec<ActiveTransition>,
}

/// Transition en vol sur `(nœud, propriété)` — étape 45 :
/// `from`/`to` sont les valeurs calculées textuelles
/// (interpolées par `explorer_css::transition`), `start`
/// l'`Instant` d'enregistrement.
#[derive(Clone)]
struct ActiveTransition {
    /// Ordinal de l'élément en ordre document — stable au
    /// re-parse (`NodeId` régénéré), seules les insertions
    /// structurelles décalent (déviation MVP documentée).
    elem: usize,
    prop: String,
    from: String,
    to: String,
    start: Instant,
    /// Durée en secondes (> 0 garanti à l'enregistrement).
    duration: f64,
    /// Délai en secondes (négatif = déjà entamée, spec).
    delay: f64,
    timing: Timing,
}

impl ActiveTransition {
    /// Instant de fin (`start + max(0, delay + duration)`).
    fn end(&self) -> Instant {
        self.start + Duration::from_secs_f64((self.delay + self.duration).max(0.0))
    }

    /// Valeur affichée à `now` : `Some` tant que la
    /// transition n'est pas terminée — la phase de délai
    /// reste sur `from`, une valeur non interpolable commute
    /// discrètement à `to` en fin (spec).
    fn value_at(&self, now: Instant) -> Option<String> {
        let elapsed = now.saturating_duration_since(self.start).as_secs_f64() - self.delay;
        if elapsed >= self.duration {
            return None;
        }
        let p = self.timing.apply(elapsed / self.duration);
        Some(interpolate(&self.from, &self.to, p).unwrap_or_else(|| {
            if p < 1.0 {
                self.from.clone()
            } else {
                self.to.clone()
            }
        }))
    }
}

/// Borne anti-hostile : transitions simultanées par onglet
/// (une page hostile peut déclencher des milliers de diffs).
const MAX_TRANSITIONS: usize = 256;

/// `NodeId` → ordinal d'élément en ordre document (étape
/// 45) — clé stable au re-parse des synchronisations DOM :
/// les attributs mutés ne changent pas l'ordre, seules les
/// insertions d'éléments décalent les suivants (leurs
/// transitions repartent alors — déviation MVP).
fn element_ordinals(dom: &Dom) -> HashMap<NodeId, usize> {
    let mut out = HashMap::new();
    let mut queue = vec![dom.document()];
    while let Some(n) = queue.pop() {
        if dom.element(n).is_ok() {
            out.insert(n, out.len());
        }
        if let Ok(ch) = dom.children(n) {
            queue.extend(ch.iter().rev());
        }
    }
    out
}

/// Diff `prev_styles` → `styles` (étape 45) : enregistre les
/// nouvelles transitions (`transition-*` déclarés, valeur
/// modifiée), retarget les transitions en vol depuis leur
/// valeur courante, applique les progressions en surcharge
/// post-cascade sur `styles`. Renvoie `true` s'il reste des
/// transitions actives. `prev` DOIT être le map brut de la
/// passe précédente (sans les surcharges d'animation),
/// indexé par ordinal d'élément.
fn update_transitions(
    prev: &HashMap<usize, ComputedStyle>,
    transitions: &mut Vec<ActiveTransition>,
    styles: &mut StyleMap,
    ord_of: &HashMap<NodeId, usize>,
    now: Instant,
) -> bool {
    let mut fresh: Vec<ActiveTransition> = Vec::new();
    for (node, cs) in styles.iter() {
        let Some(&ord) = ord_of.get(&node) else {
            continue;
        };
        // Pas de « before-change style » (premier rendu,
        // nœud inséré) → pas de transition (spec).
        let Some(old) = prev.get(&ord) else {
            continue;
        };
        for decl in transition_decls(cs)
            .iter()
            .filter(|d| d.duration > 0.0 && d.property != "none")
        {
            let targets: Vec<String> = if decl.property == "all" {
                cs.iter()
                    .map(|(k, _)| k.to_string())
                    .filter(|k| !k.starts_with("transition"))
                    .collect()
            } else {
                vec![decl.property.clone()]
            };
            for prop in targets {
                let new = cs
                    .get(&prop)
                    .or_else(|| initial_value(&prop))
                    .unwrap_or("")
                    .to_string();
                let ongoing = transitions
                    .iter()
                    .position(|t| t.elem == ord && t.prop == prop);
                if let Some(i) = ongoing {
                    if transitions[i].to == new {
                        // Même cible — la transition continue.
                        fresh.push(transitions[i].clone());
                        continue;
                    }
                }
                let oldv = old
                    .get(&prop)
                    .or_else(|| initial_value(&prop))
                    .unwrap_or("")
                    .to_string();
                if new == oldv && ongoing.is_none() {
                    continue;
                }
                // Retarget : repart de la valeur actuellement
                // affichée, sinon de l'ancienne calculée.
                let from = ongoing
                    .and_then(|i| transitions[i].value_at(now))
                    .unwrap_or(oldv);
                fresh.retain(|t| !(t.elem == ord && t.prop == prop));
                if from != new {
                    fresh.push(ActiveTransition {
                        elem: ord,
                        prop,
                        from,
                        to: new,
                        start: now,
                        duration: decl.duration,
                        delay: decl.delay,
                        timing: decl.timing,
                    });
                }
            }
        }
    }
    // Transitions non revisitées : survivent si leur
    // déclaration existe encore, la cible reste la valeur
    // calculée et elles ne sont pas finies.
    let cs_of: HashMap<usize, &ComputedStyle> = styles
        .iter()
        .filter_map(|(n, c)| ord_of.get(&n).map(|o| (*o, c)))
        .collect();
    for t in transitions.drain(..) {
        if fresh.iter().any(|f| f.elem == t.elem && f.prop == t.prop) || t.end() <= now {
            continue;
        }
        let alive = cs_of.get(&t.elem).is_some_and(|c| {
            c.get(&t.prop)
                .or_else(|| initial_value(&t.prop))
                .unwrap_or("")
                == t.to
                && transition_decls(c)
                    .iter()
                    .any(|d| d.duration > 0.0 && (d.property == "all" || d.property == t.prop))
        });
        if alive {
            fresh.push(t);
        }
    }
    transitions.clone_from(&fresh);
    transitions.truncate(MAX_TRANSITIONS);
    // Surcharges : la progression l'emporte sur toute
    // déclaration (post-cascade, spec « current transition »).
    let mut running = false;
    let node_of: HashMap<usize, NodeId> = ord_of.iter().map(|(n, o)| (*o, *n)).collect();
    for t in transitions.iter() {
        if t.end() > now {
            running = true;
            if let Some(v) = t.value_at(now) {
                if let Some(cs) = node_of.get(&t.elem).and_then(|n| styles.get_mut(*n)) {
                    cs.set(&t.prop, &v);
                }
            }
        }
    }
    transitions.retain(|t| t.end() > now);
    running
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
    /// Budget d'opérations JS par `eval` (`set_op_limit` du
    /// moteur — protection anti-script-infini, règle budgets).
    pub js_ops: u64,
    /// Nombre max d'`<img>` fetchés par document (règle
    /// budgets — une page peut déclarer des milliers de src).
    pub max_images: usize,
    /// Budget pixels par image décodée (≈ 4 octets/pixel —
    /// borne l'allocation avant décodage).
    pub image_max_px: u32,
    /// Nombre max de feuilles `<link>` fetchées par document
    /// (bornes sous-ressources — une page peut en chaîner
    /// des centaines).
    pub max_sheets: usize,
    /// Capacité du registre `@font-face` par session
    /// (règle budgets — les faces sont par document, le
    /// registre est purgé à chaque navigation).
    pub max_fonts: usize,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            event_capacity: 64,
            viewport: (1024.0, 768.0),
            scale: 1.0,
            js_ops: 5_000_000,
            max_images: 32,
            image_max_px: 16_000_000,
            max_sheets: 8,
            max_fonts: 16,
        }
    }
}

/// Session : transport + fontes injectés, onglets, viewport,
/// bus d'événements.
pub struct Session {
    transport: Arc<dyn HttpTransport>,
    /// Registre de fontes : faces `@font-face` fetchées
    /// devant la source locale injectée (ADR-0016). Purge
    /// par navigation (faces par document).
    webfonts: WebFontSource,
    tabs: HashMap<TabId, Tab>,
    events: broadcast::Sender<CoreEvent>,
    next_tab: TabId,
    /// Viewport CSS `(largeur, hauteur)` — partagé par les
    /// onglets au MVP.
    viewport: (f32, f32),
    /// Facteur DPI.
    scale: f32,
    /// Persistance du profil (historique + signets) — `None`
    /// en mémoire pure (tests, CLI).
    store: Option<Arc<dyn ProfileStore>>,
    /// Signets en mémoire (miroir du store).
    bookmarks: Vec<Bookmark>,
    /// Fabrique de contextes JS — `None` = les `<script>` sont
    /// ignorés (défaut : navigation sans JavaScript).
    js_factory: Option<JsFactory>,
    /// Budget d'ops appliqué à chaque contexte (`js_ops` de
    /// config — `set_op_limit` à la création).
    js_ops: u64,
    /// Bornes images (`SessionConfig` — nombre par document
    /// et pixels décodés).
    max_images: usize,
    image_max_px: u32,
    /// Bornes sous-ressources (`max_sheets` feuilles liées).
    max_sheets: usize,
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
            webfonts: WebFontSource::new(fonts, cfg.max_fonts),
            tabs: HashMap::new(),
            events,
            next_tab: 1,
            viewport: cfg.viewport,
            scale: cfg.scale,
            store: None,
            bookmarks: Vec::new(),
            js_factory: None,
            js_ops: cfg.js_ops,
            max_images: cfg.max_images,
            image_max_px: cfg.image_max_px,
            max_sheets: cfg.max_sheets,
        }
    }

    /// Branche la fabrique de moteurs JS (`DilloEngine::factory()`
    /// côté infra, `null_factory()` pour un no-op, `None` par
    /// défaut = scripts ignorés). Un contexte neuf est créé à
    /// chaque navigation qui rencontre un `<script>`.
    pub fn set_js_factory(&mut self, f: JsFactory) {
        self.js_factory = Some(f);
    }

    /// Branche un magasin de profil : charge les signets et
    /// enregistre chaque navigation suivante.
    pub fn set_store(&mut self, store: Arc<dyn ProfileStore>) {
        self.bookmarks = store.load_bookmarks();
        self.store = Some(store);
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
                title: None,
                hist: Vec::new(),
                hist_idx: 0,
                scroll_y: 0.0,
                doc_h: 0.0,
                dom: None,
                tree: None,
                list: None,
                frame: None,
                png: None,
                focused: None,
                js: None,
                images: ImageTable::new(),
                linked_css: String::new(),
                prev_styles: HashMap::new(),
                transitions: Vec::new(),
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
        let _ = self.events.send(CoreEvent::TabClosed { tab });
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

    /// Vue publique de l'onglet (`None` si inconnu).
    #[must_use]
    pub fn tab_info(&self, tab: TabId) -> Option<TabInfo> {
        self.tabs.get(&tab).map(|t| TabInfo {
            id: tab,
            url: t.url.as_ref().map(std::string::ToString::to_string),
            title: t.title.clone(),
            can_back: t.hist_idx > 0,
            can_forward: t.hist_idx + 1 < t.hist.len(),
        })
    }

    /// Vue publique de tous les onglets.
    #[must_use]
    pub fn tab_infos(&self) -> Vec<TabInfo> {
        self.tabs
            .keys()
            .filter_map(|&id| self.tab_info(id))
            .collect()
    }

    /// PNG courant de l'onglet (`None` si jamais navigué).
    #[must_use]
    pub fn frame_png(&self, tab: TabId) -> Option<Vec<u8>> {
        self.tabs.get(&tab).and_then(|t| t.png.clone())
    }

    /// Fait avancer les transitions CSS en vol (étape 45) :
    /// relayout + repaint si des transitions sont actives —
    /// l'animation est pilotée par le pull de frames de l'UI
    /// (le moteur n'a pas de boucle vsync propre ; `rAF` vit
    /// côté `dillo-js`). `Ok(true)` = une nouvelle frame a
    /// été produite et `FrameInvalidated` émis.
    ///
    /// # Errors
    /// [`CoreError::UnknownTab`], ou erreur de rendu.
    pub fn tick_animations(&mut self, tab: TabId) -> Result<bool, CoreError> {
        let active = self
            .tabs
            .get(&tab)
            .is_some_and(|t| !t.transitions.is_empty());
        if !active {
            return Ok(false);
        }
        self.relayout(tab)?;
        let _ = self.events.send(CoreEvent::FrameInvalidated { tab });
        Ok(true)
    }

    /// Signets (miroir du store).
    #[must_use]
    pub fn bookmarks(&self) -> &[Bookmark] {
        &self.bookmarks
    }

    /// Ajoute un signet — dédupliqué par URL. Persiste via
    /// le store si branché.
    pub fn add_bookmark(&mut self, url: String, title: Option<String>) -> bool {
        if url.is_empty() || self.bookmarks.iter().any(|b| b.url == url) {
            return false;
        }
        self.bookmarks.push(Bookmark { url, title });
        if let Some(s) = &self.store {
            s.save_bookmarks(&self.bookmarks);
        }
        true
    }

    /// Retire un signet par URL — `true` s'il existait.
    pub fn remove_bookmark(&mut self, url: &str) -> bool {
        let n = self.bookmarks.len();
        self.bookmarks.retain(|b| b.url != url);
        if self.bookmarks.len() == n {
            false
        } else {
            if let Some(s) = &self.store {
                s.save_bookmarks(&self.bookmarks);
            }
            true
        }
    }

    /// Journal des visites du store — vide en mémoire pure.
    #[must_use]
    pub fn history(&self) -> Vec<Visit> {
        self.store.as_ref().map_or_else(Vec::new, |s| s.history())
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
        let url = Url::parse(url).map_err(|e| CoreError::Url(e.to_string()))?;
        // `push` = l'entrée rejoint l'historique ; `Navigated`
        // est émis par `navigate_request` si le document final
        // est bien celui demandé (redirection script exclue).
        match self.navigate_request(tab, Request::get(url), true).await {
            Ok(_) => {
                self.drain_script_nav(tab).await;
                Ok(())
            }
            Err(e) => Err(self.emit_error(tab, e)),
        }
    }

    /// Recharge l'URL courante (re-fetch complet, historique
    /// inchangé).
    ///
    /// # Errors
    /// [`CoreError::UnknownTab`], [`CoreError::EmptyDocument`]
    /// si l'onglet n'a jamais été navigué, ou erreur de fetch.
    pub async fn reload(&mut self, tab: TabId) -> Result<(), CoreError> {
        let Some(url) = self.tabs.get(&tab).and_then(|t| t.url.clone()) else {
            return Err(if self.tabs.contains_key(&tab) {
                CoreError::EmptyDocument
            } else {
                CoreError::UnknownTab(tab)
            });
        };
        match self.navigate_to(tab, url.clone()).await {
            Ok(_) => {
                self.drain_script_nav(tab).await;
                Ok(())
            }
            Err(e) => Err(self.emit_error(tab, e)),
        }
    }

    /// Recule dans l'historique local de l'onglet (re-fetch —
    /// pas de cache de pages, les états intermédiaires ne sont
    /// pas snapshotés au MVP).
    ///
    /// # Errors
    /// [`CoreError::UnknownTab`] ou [`CoreError::EmptyDocument`]
    /// s'il n'y a pas de page précédente.
    pub async fn go_back(&mut self, tab: TabId) -> Result<(), CoreError> {
        self.history_move(tab, -1).await?;
        self.drain_script_nav(tab).await;
        Ok(())
    }

    /// Avance dans l'historique local de l'onglet.
    ///
    /// # Errors
    /// [`CoreError::UnknownTab`] ou [`CoreError::EmptyDocument`]
    /// s'il n'y a pas de page suivante.
    pub async fn go_forward(&mut self, tab: TabId) -> Result<(), CoreError> {
        self.history_move(tab, 1).await?;
        self.drain_script_nav(tab).await;
        Ok(())
    }

    /// Déplacement dans l'historique — `delta` = ±1.
    async fn history_move(&mut self, tab: TabId, delta: isize) -> Result<(), CoreError> {
        let entry = self.tabs.get_mut(&tab).ok_or(CoreError::UnknownTab(tab))?;
        let next = entry.hist_idx.checked_add_signed(delta);
        let Some(idx) = next.filter(|&i| i < entry.hist.len()) else {
            return Err(CoreError::EmptyDocument);
        };
        let url = entry.hist[idx].clone();
        // `Navigated` est émis par `navigate_request` — sauf
        // redirection script (alors `entry.url` diffère et la
        // cible a émis pour elle-même).
        match self.navigate_to(tab, url.clone()).await {
            Ok(_) => {
                if let Some(entry) = self.tabs.get_mut(&tab) {
                    entry.hist_idx = idx;
                }
                // Pas de drain ici : les appelants publics
                // (`go_back/forward`) le font après — sinon
                // `drain_script_nav` → `history_move` →
                // `drain_script_nav` bouclerait l'inférence.
                Ok(())
            }
            Err(e) => Err(self.emit_error(tab, e)),
        }
    }

    /// Fetch → parse → état de l'onglet → rendu. Ne touche
    /// pas à l'historique (reload, déplacements). Renvoie le
    /// titre de la page.
    async fn navigate_to(&mut self, tab: TabId, url: Url) -> Result<Option<String>, CoreError> {
        self.navigate_request(tab, Request::get(url), false).await
    }

    /// Variante [`navigate_to`](Self::navigate_to) prenant une
    /// requête complète — soumission de formulaire POST
    /// (corps urlencodé) ; GET passe par le wrapper.
    /// `push` = l'URL rejoint l'historique après le rendu
    /// (navigate, soumission) ; `false` = déplacement/reload.
    async fn navigate_request(
        &mut self,
        tab: TabId,
        req: Request,
        push: bool,
    ) -> Result<Option<String>, CoreError> {
        let url = req.url.clone();
        // Titre avant écrasement — pour `emit_navigated`.
        let prev_title = self.tabs.get(&tab).and_then(|t| t.title.clone());
        let resp = self
            .transport
            .fetch(&req)
            .await
            .map_err(|e| CoreError::Fetch(e.to_string()))?;
        if !(200..300).contains(&resp.status) {
            return Err(CoreError::HttpStatus(resp.status));
        }
        let (html, _enc) = decode_sniffed(&resp.body);
        let dom = TreeBuilder::parse(&html);
        let title = extract_title(&dom);
        if let Some(s) = &self.store {
            s.record_visit(&url.to_string(), title.as_deref());
        }
        {
            let entry = self.tabs.get_mut(&tab).ok_or(CoreError::UnknownTab(tab))?;
            entry.url = Some(url.clone());
            entry.title.clone_from(&title);
            entry.scroll_y = 0.0;
            entry.focused = None;
            entry.js = None;
            entry.images.clear();
            entry.linked_css.clear();
            entry.prev_styles = HashMap::new();
            entry.transitions.clear();
            entry.dom = Some(dom);
        }
        self.webfonts.clear();
        self.run_scripts(tab, &html);
        self.fetch_stylesheets(tab).await;
        self.fetch_images(tab).await;
        self.fetch_webfonts(tab).await;
        self.relayout(tab)?;
        // La page chargée prend sa place dans l'historique
        // AVANT le drain script : un `history.go` en vol la
        // voit (index réel), une redirection script pousse sa
        // propre entrée après la sienne.
        if push {
            if let Some(entry) = self.tabs.get_mut(&tab) {
                entry.hist.truncate(entry.hist_idx + 1);
                entry.hist.push(url.clone());
                entry.hist_idx = entry.hist.len() - 1;
            }
        }
        // `Navigated` seulement si le document final est bien
        // celui demandé : une redirection script a déjà émis
        // pour sa cible.
        let final_url = self.tabs.get(&tab).and_then(|t| t.url.clone());
        if final_url.as_ref() == Some(&url) {
            self.emit_navigated(tab, &url, title.clone(), prev_title.as_deref());
        }
        Ok(title)
    }

    /// Première intention de navigation script exploitable
    /// (`NavRequest` — étape 44, côté moteur) : `Navigate`
    /// résolu contre l'URL courante, `HistoryGo` en delta.
    /// Les intentions inapplicables (URL irrésolvable) sont
    /// consommées et sautées ; le reste de la file sera
    /// re-lu au tour suivant de [`drain_script_nav`].
    fn next_script_nav(&mut self, tab: TabId) -> Option<ScriptNav> {
        let reqs = self
            .tabs
            .get_mut(&tab)
            .and_then(|e| e.js.as_mut())
            .map_or_else(Vec::new, |e| e.drain_nav_requests());
        for r in reqs {
            match r {
                NavRequest::Navigate(href) => {
                    let url = self
                        .tabs
                        .get(&tab)
                        .and_then(|t| t.url.as_ref())
                        .and_then(|b| Url::resolve(b, &href).ok());
                    if let Some(u) = url {
                        return Some(ScriptNav::To(u));
                    }
                }
                NavRequest::HistoryGo(d) => {
                    // i32 → isize : lossless sur toutes les
                    // cibles réelles ; sinon MAX garantit le
                    // refus de borne dans `history_move`.
                    let delta = isize::try_from(d).unwrap_or(isize::MAX);
                    return Some(ScriptNav::Go(delta));
                }
            }
        }
        None
    }

    /// Draine les intentions de navigation script — après
    /// chargement, événement DOM ou jobs en attente. Une
    /// navigation peut en produire d'autres (scripts de la
    /// page chargée) : boucle bornée par `MAX_NAV_HOPS`,
    /// jamais de récursion — `navigate_request` reste une
    /// feuille du graphe d'appels (seul `history_move`,
    /// boxé, boucle pour la taille du future). `true` si au
    /// moins une navigation a été appliquée.
    async fn drain_script_nav(&mut self, tab: TabId) -> bool {
        const MAX_NAV_HOPS: u8 = 8;
        let mut applied = false;
        for _ in 0..MAX_NAV_HOPS {
            let Some(act) = self.next_script_nav(tab) else {
                break;
            };
            match act {
                ScriptNav::To(url) => {
                    let _ = self.navigate_request(tab, Request::get(url), true).await;
                }
                ScriptNav::Go(0) => {
                    // `history.go(0)` = rechargement de
                    // l'entrée courante (spec HTML).
                    if let Some(u) = self.tabs.get(&tab).and_then(|t| t.url.clone()) {
                        let _ = self.navigate_to(tab, u).await;
                    }
                }
                ScriptNav::Go(d) => {
                    // Box : `history_move` appelle aussi
                    // `drain_script_nav` — récursion mutuelle
                    // cassée ici (taille + inférence `Send`).
                    let mv: std::pin::Pin<Box<dyn std::future::Future<Output = _> + Send + '_>> =
                        Box::pin(self.history_move(tab, d));
                    let _ = mv.await;
                }
            }
            applied = true;
        }
        applied
    }

    /// Fetch des feuilles `<link rel=stylesheet>` — premières
    /// sous-ressources CSS (étape 46), séquentiel borné par
    /// `max_sheets`. Le CSS rejoint `linked_css` de l'onglet
    /// et est ajouté à la cascade en origine auteur par
    /// `relayout`. Un échec individuel (réseau, résolution,
    /// statut) laisse la feuille absente — la page dégrade.
    async fn fetch_stylesheets(&mut self, tab: TabId) {
        let (Some(base), Some(dom)) = (
            self.tabs.get(&tab).and_then(|t| t.url.clone()),
            self.tabs.get(&tab).and_then(|t| t.dom.as_ref()),
        ) else {
            return;
        };
        let mut css = String::new();
        for href in collect_stylesheet_links(dom)
            .into_iter()
            .take(self.max_sheets)
        {
            let Ok(url) = Url::resolve(&base, &href) else {
                continue;
            };
            let Ok(resp) = self.transport.fetch(&Request::get(url)).await else {
                continue;
            };
            if !(200..300).contains(&resp.status) {
                continue;
            }
            let (text, _enc) = decode_sniffed(&resp.body);
            css.push_str(&text);
            css.push('\n');
        }
        if let Some(entry) = self.tabs.get_mut(&tab) {
            entry.linked_css = css;
        }
    }

    /// Fetch + décode + inscription des `@font-face` du
    /// document (étape 46). Séquentiel borné : `max_fonts`
    /// faces au registre, chaque source `url()` essayée dans
    /// l'ordre (première qui décode gagne), `local()` sans
    /// fetch (la source locale repli résout par famille). La
    /// fonte est présente au `relayout` suivant — jamais de
    /// re-layout partiel (une frame = un état cohérent).
    async fn fetch_webfonts(&mut self, tab: TabId) {
        let (Some(base), Some(dom)) = (
            self.tabs.get(&tab).and_then(|t| t.url.clone()),
            self.tabs.get(&tab).and_then(|t| t.dom.as_ref()),
        ) else {
            return;
        };
        let mut eng = StyleEngine::new();
        eng.add_css(&collect_style_css(dom), Origin::Author);
        if let Some(entry) = self.tabs.get(&tab) {
            eng.add_css(&entry.linked_css.clone(), Origin::Author);
        }
        let faces = eng.font_faces(&StyleEnv::default());
        for face in faces {
            if self.webfonts.is_full() {
                break;
            }
            for src in &face.srcs {
                let FontSrc::Url(href) = src else {
                    continue;
                };
                let Ok(url) = Url::resolve(&base, href) else {
                    continue;
                };
                let Ok(resp) = self.transport.fetch(&Request::get(url)).await else {
                    continue;
                };
                if !(200..300).contains(&resp.status) {
                    continue;
                }
                let Some(data) = decode_font(&resp.body) else {
                    continue;
                };
                if self
                    .webfonts
                    .register(&face.family, face.weight, face.italic, data)
                {
                    break;
                }
            }
        }
    }

    /// Fetch + décode des `<img src>` du document — séquentiel
    /// borné (`max_images` entrées, `image_max_px` par image) ;
    /// un échec individuel (réseau, format, borne) laisse le
    /// nœud hors de la table → le boxtree peint son `alt`.
    async fn fetch_images(&mut self, tab: TabId) {
        let (Some(base), Some(dom)) = (
            self.tabs.get(&tab).and_then(|t| t.url.clone()),
            self.tabs.get(&tab).and_then(|t| t.dom.as_ref()),
        ) else {
            return;
        };
        let wanted = collect_images(dom);
        for (node, src) in wanted.into_iter().take(self.max_images) {
            let Ok(url) = Url::resolve(&base, &src) else {
                continue;
            };
            let Ok(resp) = self.transport.fetch(&Request::get(url)).await else {
                continue;
            };
            if !(200..300).contains(&resp.status) {
                continue;
            }
            let Some(img) = decode_image(&resp.body, self.image_max_px) else {
                continue;
            };
            if let Some(entry) = self.tabs.get_mut(&tab) {
                entry.images.insert(node, std::sync::Arc::new(img));
            }
        }
    }

    /// Évalue les `<script>` inline du document — contexte neuf
    /// par document (créé ici, détruit à la prochaine
    /// navigation) avec le DOM miroir installé (`document`…).
    /// Un script en échec émet `PipelineError` mais ne fait pas
    /// échouer la page (dégradation, règle 3). Les mutations
    /// du miroir sont resynchronisées : re-parse du document
    /// sérialisé → le `relayout` suivant les rend à l'écran.
    fn run_scripts(&mut self, tab: TabId, html: &str) {
        let (Some(f), Some(dom)) = (
            self.js_factory.clone(),
            self.tabs.get(&tab).and_then(|t| t.dom.as_ref()),
        ) else {
            return;
        };
        let scripts = collect_script_texts(dom);
        if scripts.is_empty() {
            return;
        }
        let (page_url, hist_len) = self.tabs.get(&tab).map_or_else(
            || (String::new(), 1),
            |t| {
                (
                    t.url.as_ref().map_or_else(String::new, Url::to_string),
                    u32::try_from(t.hist.len()).unwrap_or(u32::MAX),
                )
            },
        );
        let mut eng = (f)();
        eng.set_budget(self.js_ops);
        eng.install_dom(html, &page_url, hist_len);
        for src in &scripts {
            if let Err(e) = eng.eval(src) {
                tracing::debug!(tab, error = %e, "script en échec");
                let _ = self.events.send(CoreEvent::PipelineError {
                    tab: Some(tab),
                    message: e.to_string(),
                });
            }
        }
        let n = eng.run_pending();
        tracing::trace!(tab, jobs = n, "jobs js exécutés");
        // Miroir muté ? → re-parse : le relayout qui suit rend
        // les modifications (dirty bit consommé côté moteur).
        if let Some(mutated) = eng.sync_dom() {
            self.apply_dom_sync(tab, &mutated, false);
        }
        if let Some(entry) = self.tabs.get_mut(&tab) {
            entry.js = Some(eng);
        }
    }

    /// Resynchronise le document depuis le miroir JS muté :
    /// re-parse du HTML sérialisé, relecture du titre
    /// (`TitleChanged` si changé), reset du focus (les
    /// `NodeId` changent au re-parse). `relayout` contrôle
    /// l'invalidation immédiate — vrai depuis les événements,
    /// faux pendant la navigation (qui relayout juste après).
    fn apply_dom_sync(&mut self, tab: TabId, html: &str, relayout: bool) {
        let new_dom = TreeBuilder::parse(html);
        let new_title = extract_title(&new_dom);
        if let Some(entry) = self.tabs.get_mut(&tab) {
            entry.dom = Some(new_dom);
            entry.focused = None;
            // Les `NodeId` changent au re-parse → les pixels
            // indexés par nœud sont obsolètes (les `<img>`
            // nouveaux retombent sur `alt` jusqu'à la
            // prochaine navigation — borne documentée).
            entry.images.clear();
            if entry.title != new_title {
                entry.title.clone_from(&new_title);
                if let Some(t) = new_title {
                    let _ = self.events.send(CoreEvent::TitleChanged { tab, title: t });
                }
            }
        }
        if relayout {
            if let Err(e) = self.relayout(tab) {
                tracing::debug!(tab, error = %e, "relayout après mutation js");
                return;
            }
            let _ = self.events.send(CoreEvent::FrameInvalidated { tab });
        }
    }

    /// `Navigated` + `TitleChanged` (si le titre change) +
    /// `FrameInvalidated` — séquence d'événements post-rendu.
    /// `prev` = titre avant navigation (capturé par l'appelant,
    /// `navigate_to` l'ayant déjà écrasé).
    fn emit_navigated(&mut self, tab: TabId, url: &Url, title: Option<String>, prev: Option<&str>) {
        let _ = self.events.send(CoreEvent::Navigated {
            tab,
            url: url.to_string(),
            title: title.clone(),
        });
        if let Some(t) = title.filter(|t| prev != Some(t.as_str())) {
            let _ = self.events.send(CoreEvent::TitleChanged { tab, title: t });
        }
        let _ = self.events.send(CoreEvent::FrameInvalidated { tab });
    }

    /// Log + `PipelineError` sur le bus, renvoie l'erreur.
    fn emit_error(&mut self, tab: TabId, e: CoreError) -> CoreError {
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
        let _ = self.events.send(CoreEvent::FrameInvalidated { tab });
        Ok(())
    }

    /// Style → boîtes → layout → display list → raster → PNG.
    fn relayout(&mut self, tab: TabId) -> Result<(), CoreError> {
        let entry = self.tabs.get_mut(&tab).ok_or(CoreError::UnknownTab(tab))?;
        let dom = entry.dom.as_mut().ok_or(CoreError::EmptyDocument)?;

        // Feuilles : `<style>` inline puis `<link>` fetchées
        // (étape 46) — même origine auteur, ordre source.
        let mut eng = StyleEngine::new();
        eng.add_css(&collect_style_css(dom), Origin::Author);
        eng.add_css(&entry.linked_css.clone(), Origin::Author);
        let mut styles = eng.compute_tree(dom, dom.document(), &StyleEnv::default());
        // Transitions (étape 45) : diff sur le brut, surcharge
        // des progressions — `prev_styles` conserve le map
        // AVANT animation, indexé par ordinal d'élément (le
        // `NodeId` meurt au prochain re-parse).
        let ord_of = element_ordinals(dom);
        let raw: HashMap<usize, ComputedStyle> = styles
            .iter()
            .filter_map(|(n, c)| ord_of.get(&n).map(|o| (*o, c.clone())))
            .collect();
        update_transitions(
            &entry.prev_styles,
            &mut entry.transitions,
            &mut styles,
            &ord_of,
            Instant::now(),
        );
        entry.prev_styles = raw;
        let Some(root_el) = dom
            .children(dom.document())
            .into_iter()
            .flatten()
            .find(|&c| dom.element(c).is_ok())
        else {
            return Err(CoreError::EmptyDocument);
        };
        let sizes = image_sizes(dom, &entry.images);
        let Some(mut bx) = build_box_tree_sized(dom, root_el, &styles, &sizes) else {
            return Err(CoreError::NothingToPaint);
        };
        let (vw, _vh) = self.viewport;
        let ctx = BlockCtx {
            fonts: Some(&self.webfonts as &dyn FontSource),
            ..BlockCtx::default()
        };
        layout_tree(&mut bx, vw, &ctx);
        entry.doc_h = bx.rect.height;
        entry.tree = Some(bx);
        self.repaint(tab)
    }

    /// Display list + raster + PNG depuis l'arbre posé —
    /// sans re-layout (scroll, invalidation partielle).
    fn repaint(&mut self, tab: TabId) -> Result<(), CoreError> {
        let entry = self.tabs.get(&tab).ok_or(CoreError::UnknownTab(tab))?;
        let Some(bx) = entry.tree.as_ref() else {
            return Err(CoreError::EmptyDocument);
        };
        let (vw, vh) = self.viewport;
        let list = build_display_list_imaged(bx, vw, vh, entry.scroll_y, &entry.images);
        let frame = rasterize(&list, &self.webfonts as &dyn FontSource, self.scale)
            .ok_or(CoreError::FrameTooBig)?;
        let png = frame.encode_png();
        let entry = self.tabs.get_mut(&tab).ok_or(CoreError::UnknownTab(tab))?;
        entry.list = Some(list);
        entry.frame = Some(frame);
        entry.png = png;
        Ok(())
    }

    /// Applique un événement d'entrée à l'onglet : hit-test
    /// sur l'arbre posé, scroll borné, suivi de lien.
    ///
    /// # Errors
    /// [`CoreError::UnknownTab`], ou erreur de navigation si
    /// un clic de lien déclenche un fetch qui échoue.
    pub async fn handle_input(
        &mut self,
        tab: TabId,
        ev: InputEvent,
    ) -> Result<InputOutcome, CoreError> {
        if !self.tabs.contains_key(&tab) {
            return Err(CoreError::UnknownTab(tab));
        }
        match ev {
            InputEvent::Key { key } => {
                // Soumission implicite : `Enter` dans un champ
                // d'un `<form>` soumet comme le bouton par
                // défaut (événement `submit` d'abord —
                // `preventDefault` annule).
                if key == "Enter" {
                    if let Some(form) = self.focused_form(tab) {
                        self.blur(tab);
                        if !self.dispatch_dom_event(tab, "submit", form) {
                            self.submit_form(tab, form, None).await?;
                            return Ok(InputOutcome::Navigated);
                        }
                        return Ok(InputOutcome::Ignored);
                    }
                }
                self.handle_key(tab, &key)
            }
            InputEvent::Scroll { dy } => {
                self.scroll_by(tab, dy)?;
                Ok(InputOutcome::Scrolled {
                    scroll_y: self.tabs.get(&tab).map_or(0.0, |t| t.scroll_y),
                })
            }
            InputEvent::MouseMove { x, y } => Ok(InputOutcome::Hover {
                url: self.link_at(tab, x, y),
                editable: self.field_at(tab, x, y).is_some(),
            }),
            InputEvent::Click { x, y } => {
                // L'événement DOM précède l'action par défaut :
                // un handler peut muter la page (resynchronisée
                // et re-peinte ici) ou `preventDefault`.
                let prevented = self.dispatch_click(tab, x, y);
                // Navigation script (`location`, `history`) :
                // priorité sur l'action par défaut du clic.
                if self.drain_script_nav(tab).await {
                    return Ok(InputOutcome::Navigated);
                }
                if let Some(field) = self.field_at(tab, x, y) {
                    if let Some(entry) = self.tabs.get_mut(&tab) {
                        entry.focused = Some(field);
                    }
                    return Ok(InputOutcome::Focused);
                }
                self.blur(tab);
                // Contrôle à bascule : checkbox/radio — le
                // `checked` change → restyle + repaint.
                if let Some(ctrl) = self.toggle_at(tab, x, y) {
                    {
                        let entry = self.tabs.get_mut(&tab).ok_or(CoreError::UnknownTab(tab))?;
                        let dom = entry.dom.as_mut().ok_or(CoreError::EmptyDocument)?;
                        toggle_checked(dom, ctrl)?;
                    }
                    self.relayout(tab)?;
                    let _ = self.events.send(CoreEvent::FrameInvalidated { tab });
                    return Ok(InputOutcome::Edited);
                }
                // `<select>` : cycle d'options — le sélecteur
                // natif délégué UI est hors scope (ADR-0014).
                if let Some(sel) = self.select_at(tab, x, y) {
                    {
                        let entry = self.tabs.get_mut(&tab).ok_or(CoreError::UnknownTab(tab))?;
                        let dom = entry.dom.as_mut().ok_or(CoreError::EmptyDocument)?;
                        cycle_select(dom, sel)?;
                    }
                    self.relayout(tab)?;
                    let _ = self.events.send(CoreEvent::FrameInvalidated { tab });
                    return Ok(InputOutcome::Edited);
                }
                // Contrôle submit : l'événement `submit` du
                // formulaire part avant l'action par défaut.
                if let Some((form, sub)) = self.submit_at(tab, x, y) {
                    if !prevented && !self.dispatch_dom_event(tab, "submit", form) {
                        if self.drain_script_nav(tab).await {
                            return Ok(InputOutcome::Navigated);
                        }
                        self.submit_form(tab, form, Some(sub)).await?;
                        return Ok(InputOutcome::Navigated);
                    }
                    return Ok(InputOutcome::Ignored);
                }
                match self.link_at(tab, x, y) {
                    Some(url) if !prevented => {
                        self.navigate(tab, &url).await?;
                        Ok(InputOutcome::Navigated)
                    }
                    _ => Ok(InputOutcome::Ignored),
                }
            }
        }
    }

    /// `input`/`textarea` éditable sous `(x, y)` — hit-test +
    /// remontée d'ancêtres (le texte du champ porte le
    /// `NodeId` du texte, pas du champ).
    fn field_at(&self, tab: TabId, x: f32, y: f32) -> Option<NodeId> {
        let entry = self.tabs.get(&tab)?;
        let tree = entry.tree.as_ref()?;
        let dom = entry.dom.as_ref()?;
        let node = hit_test(tree, x, y + entry.scroll_y)?;
        editable_field(dom, node)
    }

    /// `checkbox`/`radio` sous `(x, y)` — hit-test + ancêtres.
    fn toggle_at(&self, tab: TabId, x: f32, y: f32) -> Option<NodeId> {
        let entry = self.tabs.get(&tab)?;
        let (tree, dom) = (entry.tree.as_ref()?, entry.dom.as_ref()?);
        let node = hit_test(tree, x, y + entry.scroll_y)?;
        toggle_control(dom, node)
    }

    /// `<select>` sous `(x, y)` — hit-test + ancêtres.
    fn select_at(&self, tab: TabId, x: f32, y: f32) -> Option<NodeId> {
        let entry = self.tabs.get(&tab)?;
        let (tree, dom) = (entry.tree.as_ref()?, entry.dom.as_ref()?);
        let node = hit_test(tree, x, y + entry.scroll_y)?;
        select_control(dom, node)
    }

    /// `(form, submitter)` sous `(x, y)` — un contrôle submit
    /// dans un `<form>`, `None` sinon.
    fn submit_at(&self, tab: TabId, x: f32, y: f32) -> Option<(NodeId, NodeId)> {
        let entry = self.tabs.get(&tab)?;
        let (tree, dom) = (entry.tree.as_ref()?, entry.dom.as_ref()?);
        let node = hit_test(tree, x, y + entry.scroll_y)?;
        let ctrl = submit_control(dom, node)?;
        let form = enclosing_form(dom, ctrl)?;
        Some((form, ctrl))
    }

    /// `<form>` englobant le champ focalisé — la soumission
    /// implicite sur `Enter` ne vaut que pour les `<input>`
    /// mono-ligne (`<textarea>` garde son saut de ligne).
    fn focused_form(&self, tab: TabId) -> Option<NodeId> {
        let entry = self.tabs.get(&tab)?;
        let dom = entry.dom.as_ref()?;
        let field = entry.focused?;
        if !dom.tag_name(field).is_ok_and(|t| t == "input") {
            return None;
        }
        enclosing_form(dom, field)
    }

    /// Soumission de formulaire (étape 38) : contrôles
    /// successful → `GET` (query) ou `POST` urlencodé, puis le
    /// pipeline de navigation complet sur la réponse.
    /// L'historique retient l'URL d'action — un reload/back
    /// refait un `GET` (déviation documentée ADR-0014).
    async fn submit_form(
        &mut self,
        tab: TabId,
        form: NodeId,
        submitter: Option<NodeId>,
    ) -> Result<(), CoreError> {
        let req = {
            let entry = self.tabs.get(&tab).ok_or(CoreError::UnknownTab(tab))?;
            let (Some(dom), Some(base)) = (entry.dom.as_ref(), entry.url.as_ref()) else {
                return Err(CoreError::EmptyDocument);
            };
            let pairs = collect_form_data(dom, form, submitter);
            let action = dom
                .attribute(form, "action")
                .ok()
                .flatten()
                .unwrap_or_default();
            let method = dom
                .attribute(form, "method")
                .ok()
                .flatten()
                .unwrap_or_default();
            if method.eq_ignore_ascii_case("post") {
                let url = Url::resolve(base, &action).map_err(|e| CoreError::Url(e.to_string()))?;
                Request {
                    method: Method::Post,
                    url,
                    headers: vec![(
                        "content-type".to_string(),
                        "application/x-www-form-urlencoded".to_string(),
                    )],
                    body: Some(form_url_encode(&pairs).into_bytes()),
                }
            } else {
                let qs = form_url_encode(&pairs);
                let target = if qs.is_empty() {
                    action
                } else {
                    let sep = if action.contains('?') { '&' } else { '?' };
                    format!("{action}{sep}{qs}")
                };
                let url = Url::resolve(base, &target).map_err(|e| CoreError::Url(e.to_string()))?;
                Request::get(url)
            }
        };
        match self.navigate_request(tab, req, true).await {
            Ok(_) => {
                self.drain_script_nav(tab).await;
                Ok(())
            }
            Err(e) => Err(self.emit_error(tab, e)),
        }
    }

    /// Perd le focus (clic hors champ).
    fn blur(&mut self, tab: TabId) {
        if let Some(entry) = self.tabs.get_mut(&tab) {
            entry.focused = None;
        }
    }

    /// Édition du champ focalisé : `Backspace` efface,
    /// `Enter`/`Escape` sortent du champ, un caractère
    /// imprimable s'insère — puis restyle + repaint.
    fn handle_key(&mut self, tab: TabId, key: &str) -> Result<InputOutcome, CoreError> {
        let focused = self.tabs.get(&tab).and_then(|t| t.focused).filter(|&n| {
            self.tabs
                .get(&tab)
                .and_then(|t| t.dom.as_ref())
                .is_some_and(|d| d.is_alive(n))
        });
        let Some(field) = focused else {
            tracing::trace!(tab, key, "touche sans champ focalisé");
            return Ok(InputOutcome::Ignored);
        };
        match key {
            "Enter" | "Escape" => {
                self.blur(tab);
                return Ok(InputOutcome::Ignored);
            }
            _ => {}
        }
        {
            let entry = self.tabs.get_mut(&tab).ok_or(CoreError::UnknownTab(tab))?;
            let dom = entry.dom.as_mut().ok_or(CoreError::EmptyDocument)?;
            edit_field(dom, field, key)?;
        }
        self.relayout(tab)?;
        let _ = self.events.send(CoreEvent::FrameInvalidated { tab });
        Ok(InputOutcome::Edited)
    }

    /// Dispatche `click` dans le miroir JS (propagation
    /// capture→cible→bulle côté moteur, microtasks et timers
    /// dus drainés), puis resynchronise les mutations DOM.
    /// Retourne `defaultPrevented` — l'appelant supprime alors
    /// l'action par défaut (navigation de lien).
    fn dispatch_click(&mut self, tab: TabId, x: f32, y: f32) -> bool {
        let node = self.tabs.get(&tab).and_then(|entry| {
            let (tree, _dom) = (entry.tree.as_ref()?, entry.dom.as_ref()?);
            hit_test(tree, x, y + entry.scroll_y)
        });
        node.is_some_and(|n| self.dispatch_dom_event(tab, "click", n))
    }

    /// Dispatche l'événement `ty` sur `node` dans le miroir JS
    /// puis resynchronise les mutations DOM (même traitement
    /// que `dispatch_click`). Retourne `defaultPrevented`.
    fn dispatch_dom_event(&mut self, tab: TabId, ty: &str, node: NodeId) -> bool {
        let path = self
            .tabs
            .get(&tab)
            .and_then(|entry| entry.dom.as_ref().and_then(|d| element_path(d, node)));
        let Some(path) = path else { return false };
        let (prevented, mutated) = {
            let Some(entry) = self.tabs.get_mut(&tab) else {
                return false;
            };
            let Some(eng) = entry.js.as_mut() else {
                return false;
            };
            let p = match eng.dispatch_event(ty, &path) {
                Ok(p) => p,
                Err(e) => {
                    tracing::debug!(tab, ty, error = %e, "listener dom en échec");
                    let _ = self.events.send(CoreEvent::PipelineError {
                        tab: Some(tab),
                        message: e.to_string(),
                    });
                    false
                }
            };
            let n = eng.run_pending();
            tracing::trace!(tab, jobs = n, "jobs js après click");
            (p, eng.sync_dom())
        };
        if let Some(html) = mutated {
            self.apply_dom_sync(tab, &html, true);
        }
        prevented
    }

    /// `href` résolu du lien `<a>` sous `(x, y)` viewport —
    /// hit-test profond sur la boîte posée + remontée des
    /// ancêtres DOM.
    fn link_at(&self, tab: TabId, x: f32, y: f32) -> Option<String> {
        let entry = self.tabs.get(&tab)?;
        let tree = entry.tree.as_ref()?;
        let dom = entry.dom.as_ref()?;
        let node = hit_test(tree, x, y + entry.scroll_y)?;
        let href = ancestor_href(dom, node)?;
        let base = entry.url.as_ref()?;
        Some(Url::resolve(base, &href).ok()?.to_string())
    }

    /// Scroll borné `[0, doc_h - viewport_h]` puis repaint +
    /// `frame_invalidated`.
    fn scroll_by(&mut self, tab: TabId, dy: f32) -> Result<(), CoreError> {
        {
            let entry = self.tabs.get_mut(&tab).ok_or(CoreError::UnknownTab(tab))?;
            let max = (entry.doc_h - self.viewport.1).max(0.0);
            let next = (entry.scroll_y + dy).clamp(0.0, max);
            if (next - entry.scroll_y).abs() < f32::EPSILON {
                return Ok(());
            }
            entry.scroll_y = next;
        }
        self.repaint(tab)?;
        let _ = self.events.send(CoreEvent::FrameInvalidated { tab });
        Ok(())
    }
}

/// Hit-test profond : la boîte la plus imbriquée contenant
/// `(x, y)` document gagne — renvoie son `NodeId` source
/// (`None` pour les boîtes anonymes → on remonte au parent
/// peint le plus proche via les ancêtres DOM côté appelant).
fn hit_test(bx: &LayoutBox, x: f32, y: f32) -> Option<NodeId> {
    if !(bx.rect.x <= x
        && x <= bx.rect.x + bx.rect.width
        && bx.rect.y <= y
        && y <= bx.rect.y + bx.rect.height)
    {
        return None;
    }
    for child in &bx.children {
        if let Some(n) = hit_test(child, x, y) {
            return Some(n);
        }
    }
    bx.node
}

/// Chemin `(tag, index parmi les frères éléments de même tag)`
/// de `node` (ou de son plus proche ancêtre élément) depuis la
/// racine — rejoué côté miroir JS pour localiser la cible d'un
/// événement (les deux arbres peuvent diverger sur les
/// wrappers implicites, voir `mirror_node` d'`explorer-js`).
fn element_path(dom: &Dom, node: NodeId) -> Option<Vec<(String, u32)>> {
    let mut cur = node;
    while dom.element(cur).is_err() {
        cur = dom.parent(cur).ok()??;
    }
    let mut path = Vec::new();
    while let Some(p) = dom.parent(cur).ok().flatten() {
        let tag = dom.tag_name(cur).ok()?.to_string();
        let k = dom
            .children(p)
            .ok()?
            .iter()
            .filter(|&&c| c != cur && dom.tag_name(c).is_ok_and(|t| t == tag))
            .count();
        path.push((tag, u32::try_from(k).ok()?));
        cur = p;
    }
    path.reverse();
    Some(path)
}

/// Champ éditable le plus proche du nœud (lui-même compris) :
/// `<input>` à type texte ou `<textarea>`. `None` sinon.
fn editable_field(dom: &Dom, mut node: NodeId) -> Option<NodeId> {
    /// Types `input` acceptant du texte libre (absence de
    /// `type` incluse — valeur par défaut HTML).
    const TEXT_TYPES: &[&str] = &[
        "", "text", "search", "url", "password", "email", "number", "tel",
    ];
    loop {
        match dom.tag_name(node) {
            Ok("input") => {
                let ty = dom
                    .attribute(node, "type")
                    .ok()
                    .flatten()
                    .unwrap_or_default();
                return TEXT_TYPES.contains(&ty.as_str()).then_some(node);
            }
            Ok("textarea") => return Some(node),
            _ => {}
        }
        node = match dom.parent(node) {
            Ok(Some(p)) => p,
            _ => return None,
        };
    }
}

/// Valeur textuelle du champ : attribut `value` pour
/// `<input>`, contenu texte pour `<textarea>`.
fn field_value(dom: &Dom, field: NodeId) -> String {
    if dom.tag_name(field).is_ok_and(|t| t == "textarea") {
        dom.text_content(field).unwrap_or_default()
    } else {
        dom.attribute(field, "value")
            .ok()
            .flatten()
            .unwrap_or_default()
    }
}

/// Applique la touche au contenu courant : `Backspace` retire
/// le dernier caractère (frontière UTF-8), un `key` à un seul
/// `char` s'insère à la fin (pas de curseur interne au MVP) ;
/// le reste (Shift, Ctrl…) laisse le texte tel quel.
fn apply_key(cur: &str, key: &str) -> String {
    if key == "Backspace" {
        let mut s = cur.to_string();
        s.pop();
        return s;
    }
    let mut chars = key.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) => {
            let mut s = cur.to_string();
            s.push(c);
            s
        }
        _ => cur.to_string(),
    }
}

/// Édite le champ : `input` → attribut `value`, `textarea` →
/// texte enfant unique (les enfants texte surnuméraires —
/// cas pathologique — sont vidés).
fn edit_field(dom: &mut Dom, field: NodeId, key: &str) -> Result<(), DomError> {
    let next = apply_key(&field_value(dom, field), key);
    if dom.tag_name(field).is_ok_and(|t| t == "textarea") {
        let texts: Vec<NodeId> = dom
            .children(field)?
            .into_iter()
            .filter(|&c| dom.text(c).is_ok())
            .collect();
        if let Some((&first, rest)) = texts.split_first() {
            dom.set_text(first, &next)?;
            for &n in rest {
                dom.set_text(n, "")?;
            }
        } else {
            let t = dom.create_text(&next)?;
            dom.append_child(field, t)?;
        }
    } else {
        dom.set_attribute(field, "value", &next)?;
    }
    Ok(())
}

/// `href` du plus proche ancêtre `<a>` (le nœud lui-même
/// compris) — `None` si le point n'est pas dans un lien.
fn ancestor_href(dom: &Dom, mut node: NodeId) -> Option<String> {
    loop {
        if dom.tag_name(node).is_ok_and(|t| t == "a") {
            if let Ok(Some(href)) = dom.attribute(node, "href") {
                return Some(href);
            }
        }
        node = match dom.parent(node) {
            Ok(Some(p)) => p,
            _ => return None,
        };
    }
}

/// Extrait le texte du premier `<title>` du document —
/// `None` si absent ou vide.
fn extract_title(dom: &Dom) -> Option<String> {
    let mut queue = vec![dom.document()];
    while let Some(n) = queue.pop() {
        if dom.tag_name(n).is_ok_and(|t| t == "title") {
            if let Ok(t) = dom.text_content(n) {
                let t = t.trim().to_string();
                if !t.is_empty() {
                    return Some(t);
                }
            }
        }
        if let Ok(ch) = dom.children(n) {
            queue.extend(ch.iter().rev());
        }
    }
    None
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

/// `href` des `<link rel=stylesheet>` dans l'ordre — `rel`
/// case-insensitive à tokens (la spec admet `rel="stylesheet
/// alternate"` et cie) ; `href` vide sauté.
fn collect_stylesheet_links(dom: &Dom) -> Vec<String> {
    let mut out = Vec::new();
    let mut queue = vec![dom.document()];
    while let Some(n) = queue.pop() {
        if dom.tag_name(n).is_ok_and(|t| t == "link") {
            let rel = dom
                .attribute(n, "rel")
                .ok()
                .flatten()
                .unwrap_or_default()
                .to_ascii_lowercase();
            if rel.split_whitespace().any(|w| w == "stylesheet") {
                if let Ok(Some(href)) = dom.attribute(n, "href") {
                    if !href.is_empty() {
                        out.push(href);
                    }
                }
            }
        }
        if let Ok(ch) = dom.children(n) {
            queue.extend(ch.iter().rev());
        }
    }
    out
}

/// `(NodeId, src)` des `<img>` du document dans l'ordre —
/// `src` absent ou vide est sauté.
fn collect_images(dom: &Dom) -> Vec<(NodeId, String)> {
    let mut out = Vec::new();
    let mut queue = vec![dom.document()];
    while let Some(n) = queue.pop() {
        if dom.tag_name(n).is_ok_and(|t| t == "img") {
            if let Ok(Some(src)) = dom.attribute(n, "src") {
                if !src.is_empty() {
                    out.push((n, src));
                }
            }
        }
        if let Ok(ch) = dom.children(n) {
            queue.extend(ch.iter().rev());
        }
    }
    out
}

/// Taille préférée `(w, h)` par `<img>` décodé : attribut
/// `width`/`height` sain s'il existe, sinon dimension
/// intrinsèque — chaque axe indépendamment (§10.3.2).
#[allow(clippy::cast_precision_loss)] // pixels u32 → f32 : < 2^24.
fn image_sizes(dom: &Dom, images: &ImageTable) -> HashMap<NodeId, (f32, f32)> {
    let px_attr = |n: NodeId, name: &str| {
        dom.attribute(n, name)
            .ok()
            .flatten()
            .and_then(|v| v.trim().trim_end_matches("px").parse::<f32>().ok())
            .filter(|v| *v > 0.0)
    };
    images
        .iter()
        .map(|(&n, img)| {
            let w = px_attr(n, "width").unwrap_or(img.width as f32);
            let h = px_attr(n, "height").unwrap_or(img.height as f32);
            (n, (w, h))
        })
        .collect()
}

/// Textes des `<script>` inline du document, dans l'ordre —
/// `src=` et `type` non-classique (module, json…) sautés tant
/// que les sous-ressources/modules n'existent pas.
fn collect_script_texts(dom: &Dom) -> Vec<String> {
    let mut out = Vec::new();
    let mut queue = vec![dom.document()];
    while let Some(n) = queue.pop() {
        if dom.tag_name(n).is_ok_and(|t| t == "script") {
            let classic = dom.attribute(n, "type").ok().flatten().is_none_or(|t| {
                t.is_empty()
                    || t.eq_ignore_ascii_case("text/javascript")
                    || t.eq_ignore_ascii_case("application/javascript")
            });
            let has_src = dom
                .attribute(n, "src")
                .ok()
                .flatten()
                .is_some_and(|s| !s.is_empty());
            if classic && !has_src {
                if let Ok(t) = dom.text_content(n) {
                    if !t.trim().is_empty() {
                        out.push(t);
                    }
                }
            }
        }
        if let Ok(ch) = dom.children(n) {
            queue.extend(ch.iter().rev());
        }
    }
    out
}

/// `<form>` ancêtre (le nœud lui-même compris) — `None` hors
/// formulaire. L'association `form=` hors arbre est hors
/// scope MVP (ADR-0014).
fn enclosing_form(dom: &Dom, mut node: NodeId) -> Option<NodeId> {
    loop {
        if dom.tag_name(node).is_ok_and(|t| t == "form") {
            return Some(node);
        }
        node = dom.parent(node).ok()??;
    }
}

/// Contrôle de soumission le plus proche (soi ou ancêtre) :
/// `<button>` sans `type="button|reset"` (défaut HTML =
/// submit) ou `<input type="submit|image">`.
fn submit_control(dom: &Dom, mut node: NodeId) -> Option<NodeId> {
    loop {
        match dom.tag_name(node) {
            Ok("button") => {
                let ty = dom
                    .attribute(node, "type")
                    .ok()
                    .flatten()
                    .unwrap_or_default();
                if !matches!(ty.as_str(), "button" | "reset") {
                    return Some(node);
                }
            }
            Ok("input") => {
                let ty = dom
                    .attribute(node, "type")
                    .ok()
                    .flatten()
                    .unwrap_or_default();
                if matches!(ty.as_str(), "submit" | "image") {
                    return Some(node);
                }
            }
            _ => {}
        }
        node = dom.parent(node).ok()??;
    }
}

/// `input[type=checkbox|radio]` le plus proche (soi ou
/// ancêtre). Un clic sur un `<label for=id>` résout le
/// contrôle nommé (délégation HTML standard).
fn toggle_control(dom: &Dom, mut node: NodeId) -> Option<NodeId> {
    loop {
        match dom.tag_name(node) {
            Ok("input") => {
                let ty = dom
                    .attribute(node, "type")
                    .ok()
                    .flatten()
                    .unwrap_or_default();
                if matches!(ty.as_str(), "checkbox" | "radio") {
                    return Some(node);
                }
            }
            Ok("label") => {
                if let Ok(Some(f)) = dom.attribute(node, "for") {
                    if let Some(target) = find_by_id(dom, &f) {
                        if dom.tag_name(target).is_ok_and(|t| t == "input") {
                            return Some(target);
                        }
                    }
                }
            }
            _ => {}
        }
        node = dom.parent(node).ok()??;
    }
}

/// `<select>` le plus proche (soi ou ancêtre), `label for=`
/// résolu comme `toggle_control`.
fn select_control(dom: &Dom, mut node: NodeId) -> Option<NodeId> {
    loop {
        match dom.tag_name(node) {
            Ok("select") => return Some(node),
            Ok("label") => {
                if let Ok(Some(f)) = dom.attribute(node, "for") {
                    if let Some(target) = find_by_id(dom, &f) {
                        if dom.tag_name(target).is_ok_and(|t| t == "select") {
                            return Some(target);
                        }
                    }
                }
            }
            _ => {}
        }
        node = dom.parent(node).ok()??;
    }
}

/// Premier élément portant `id` en ordre document — petit
/// parcours pour `label[for]` (pas d'index d'id exposé).
fn find_by_id(dom: &Dom, id: &str) -> Option<NodeId> {
    let mut queue = vec![dom.document()];
    while let Some(n) = queue.pop() {
        if dom.element(n).is_ok() {
            if dom.attribute(n, "id").ok().flatten().as_deref() == Some(id) {
                return Some(n);
            }
            if let Ok(ch) = dom.children(n) {
                queue.extend(ch.iter().rev());
            }
        }
    }
    None
}

/// Bascule l'état du contrôle : `checkbox` inverse son
/// `checked` ; `radio` se coche et décoche ses homonymes
/// (`name` identique) du même formulaire (ou du document
/// hors formulaire).
fn toggle_checked(dom: &mut Dom, ctrl: NodeId) -> Result<(), DomError> {
    let ty = dom.attribute(ctrl, "type")?.unwrap_or_default();
    if ty == "radio" {
        if dom.attribute(ctrl, "checked")?.is_some() {
            return Ok(()); // un radio coché ne se décoche pas au clic
        }
        dom.set_attribute(ctrl, "checked", "")?;
        let name = dom.attribute(ctrl, "name")?.unwrap_or_default();
        if !name.is_empty() {
            let scope = enclosing_form(dom, ctrl).unwrap_or_else(|| dom.document());
            let mut queue = vec![scope];
            while let Some(n) = queue.pop() {
                if n != ctrl && dom.tag_name(n).is_ok_and(|t| t == "input") {
                    let same = dom.attribute(n, "type")?.unwrap_or_default() == "radio"
                        && dom.attribute(n, "name")?.unwrap_or_default() == name;
                    if same {
                        dom.remove_attribute(n, "checked")?;
                    }
                }
                if let Ok(ch) = dom.children(n) {
                    queue.extend(ch);
                }
            }
        }
        return Ok(());
    }
    if dom.attribute(ctrl, "checked")?.is_some() {
        dom.remove_attribute(ctrl, "checked")?;
    } else {
        dom.set_attribute(ctrl, "checked", "")?;
    }
    Ok(())
}

/// Cycle `<select>` : `selected` passe à l'option suivante
/// (retour au début en fin de liste) — sélection native
/// déléguée UI hors scope MVP (ADR-0014).
fn cycle_select(dom: &mut Dom, sel: NodeId) -> Result<(), DomError> {
    let opts = descendant_elements(dom, sel, "option");
    if opts.is_empty() {
        return Ok(());
    }
    let cur = opts
        .iter()
        .position(|&o| dom.attribute(o, "selected").ok().flatten().is_some())
        .unwrap_or(0);
    let next = (cur + 1) % opts.len();
    for (i, &o) in opts.iter().enumerate() {
        if i == next {
            dom.set_attribute(o, "selected", "")?;
        } else {
            dom.remove_attribute(o, "selected")?;
        }
    }
    Ok(())
}

/// Éléments descendants portant `tag`, ordre document.
fn descendant_elements(dom: &Dom, root: NodeId, tag: &str) -> Vec<NodeId> {
    let mut out = Vec::new();
    let mut queue = vec![root];
    while let Some(n) = queue.pop() {
        if n != root && dom.tag_name(n).is_ok_and(|t| t == tag) {
            out.push(n);
        }
        if let Ok(ch) = dom.children(n) {
            queue.extend(ch.iter().rev());
        }
    }
    out
}

/// `(name, value)` des contrôles successful du formulaire
/// (HTML §4.10.21) : nommés, non `disabled`, cases cochées
/// seulement, boutons uniquement le `submitter`. Déviations
/// MVP : `form=` externe, `<fieldset disabled>` et
/// `<object>` non traités.
fn collect_form_data(dom: &Dom, form: NodeId, submitter: Option<NodeId>) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut queue = vec![form];
    while let Some(n) = queue.pop() {
        let tag = dom.tag_name(n).unwrap_or("");
        if n != form
            && dom.element(n).is_ok()
            && dom.attribute(n, "disabled").ok().flatten().is_none()
        {
            match tag {
                "input" => {
                    let ty = dom
                        .attribute(n, "type")
                        .ok()
                        .flatten()
                        .unwrap_or_default()
                        .to_lowercase();
                    let ok = match ty.as_str() {
                        "checkbox" | "radio" => {
                            dom.attribute(n, "checked").ok().flatten().is_some()
                        }
                        "submit" | "image" | "button" | "reset" => Some(n) == submitter,
                        _ => true,
                    };
                    if ok {
                        push_named(dom, n, input_value(dom, n, &ty), &mut out);
                    }
                }
                "textarea" => {
                    push_named(dom, n, dom.text_content(n).unwrap_or_default(), &mut out);
                }
                "select" => {
                    push_named(dom, n, select_value(dom, n), &mut out);
                }
                "button" => {
                    let ty = dom.attribute(n, "type").ok().flatten().unwrap_or_default();
                    if Some(n) == submitter && !matches!(ty.as_str(), "button" | "reset") {
                        push_named(
                            dom,
                            n,
                            dom.attribute(n, "value").ok().flatten().unwrap_or_default(),
                            &mut out,
                        );
                    }
                }
                _ => {}
            }
        }
        // Un `<form>` ne s'imbrigue pas : on ne descend pas
        // dans un formulaire enfant éventuel.
        if n == form || tag != "form" {
            if let Ok(ch) = dom.children(n) {
                queue.extend(ch.iter().rev());
            }
        }
    }
    out
}

/// Pousse `(name, value)` si le contrôle est nommé.
fn push_named(dom: &Dom, n: NodeId, value: String, out: &mut Vec<(String, String)>) {
    if let Ok(Some(name)) = dom.attribute(n, "name") {
        if !name.is_empty() {
            out.push((name, value));
        }
    }
}

/// Valeur courante d'un `<input>` : `value` ou `"on"` pour
/// checkbox/radio sans `value`, vide sinon.
fn input_value(dom: &Dom, n: NodeId, ty: &str) -> String {
    dom.attribute(n, "value").ok().flatten().unwrap_or_else(|| {
        if matches!(ty, "checkbox" | "radio") {
            "on".to_string()
        } else {
            String::new()
        }
    })
}

/// Valeur courante d'un `<select>` : `value` (ou texte) de
/// l'option `selected`, sinon de la première.
fn select_value(dom: &Dom, sel: NodeId) -> String {
    let opts = descendant_elements(dom, sel, "option");
    let sel_opt = opts
        .iter()
        .find(|&&o| dom.attribute(o, "selected").ok().flatten().is_some())
        .or_else(|| opts.first());
    sel_opt.map_or_else(String::new, |&o| {
        dom.attribute(o, "value")
            .ok()
            .flatten()
            .unwrap_or_else(|| dom.text_content(o).unwrap_or_default().trim().to_string())
    })
}

/// Encodage `application/x-www-form-urlencoded` : espace→`+`,
/// non-réservés conservés, le reste en `%XX`.
fn form_url_encode(pairs: &[(String, String)]) -> String {
    fn enc(s: &str) -> String {
        s.bytes()
            .map(|b| match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'*' | b'-' | b'.' | b'_' => {
                    (b as char).to_string()
                }
                b' ' => "+".to_string(),
                _ => format!("%{b:02X}"),
            })
            .collect()
    }
    pairs
        .iter()
        .map(|(k, v)| format!("{}={}", enc(k), enc(v)))
        .collect::<Vec<_>>()
        .join("&")
}

/// Intention de navigation script résolue — produite par
/// [`Session::next_script_nav`], appliquée par
/// [`Session::drain_script_nav`].
enum ScriptNav {
    /// `location.*` : URL absolue déjà résolue.
    To(Url),
    /// `history.go/back/forward` : déplacement relatif
    /// (0 = rechargement de l'entrée courante).
    Go(isize),
}
