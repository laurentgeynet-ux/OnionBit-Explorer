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

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use explorer_css::transition::{interpolate, transition_decls, Timing};
use explorer_css::{
    initial_value, ComputedStyle, FontSrc, Origin, StyleEngine, StyleEnv, StyleMap,
};
use explorer_dom::{Dom, DomError, NodeId};
use explorer_html::encoding::{decode, decode_windows1252, sniff_encoding, Encoding};
use explorer_html::TreeBuilder;
use explorer_js::{JsEngine, JsFactory, NavRequest, NetRequest, NetResponse, UrlResolver};
use explorer_layout::font::FontSource;
use explorer_layout::{build_box_tree_sized, layout_tree, BlockCtx, LayoutBox};
use explorer_net::cookies::CookieJar;
use explorer_net::error::NetResult;
use explorer_net::http::{HttpTransport, Method, Request, Response};
use explorer_net::policy::{initiator_headers, is_mixed_content};
use explorer_net::url::Url;
use explorer_render::{
    build_display_list_imaged, decode_font, decode_image, rasterize, DecodedImage, DisplayList,
    Frame, ImageTable, WebFontSource,
};
use futures_util::stream::{FuturesUnordered, StreamExt};
use tokio::sync::broadcast;
use tokio::task::JoinHandle;

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
    /// Touche clavier — `keydown` dispatché puis éditée si un
    /// champ est focalisé.
    Key {
        /// Identifiant de touche (`"a"`, `"Enter"`…).
        key: String,
    },
    /// Relâchement de touche — `keyup` dispatché (étape 52).
    KeyUp {
        /// Identifiant de touche.
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
    /// Charge le `localStorage` persisté d'une origine
    /// (étape 51 — défaut : rien, l'origine repart vide).
    fn load_storage(&self, _origin: &str) -> BTreeMap<String, String> {
        BTreeMap::new()
    }
    /// Persiste le `localStorage` d'une origine (map complète
    /// — défaut : ignorer, stockage volatil).
    fn save_storage(&self, _origin: &str, _map: &BTreeMap<String, String>) {}
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
    /// **chemin** `(tag, index)` dans `dom`, pas `NodeId` :
    /// le re-parse de synchro DOM tue les ids, le chemin se
    /// re-résout à chaque usage (`dom_path_node`, étape 52).
    /// Invalidé à la navigation.
    focused: Option<Vec<(String, u32)>>,
    /// Nœud sous le curseur au dernier `mousemove` (étape 52)
    /// — sert de cible à `mouseout`/`mouseover`/`wheel` ;
    /// chemin `(tag, index)`, même raisonnement que `focused`.
    hovered: Option<Vec<(String, u32)>>,
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
    /// Nombre max de `<script src>` fetchés par document
    /// (étape 50 — une page peut en déclarer des centaines).
    pub max_scripts: usize,
    /// Nombre max de modules dans le graphe préfetché d'un
    /// `<script type="module">` (entrée + dépendances
    /// transitives — borne les chaînes d'import hostiles).
    pub max_modules: usize,
    /// Fetches de sous-ressources simultanés par onglet
    /// (`FuturesUnordered` borné — scheduler étape 50 ; les
    /// navigateurs plafonnent classiquement à ~6 par hôte).
    pub max_inflight: usize,
    /// Requêtes script-initiées (XHR/`fetch`) en vol par
    /// onglet — quota ADR-0019 : au-delà, la requête échoue
    /// en `status: 0` (événement `error`, page vivante).
    pub max_script_net: usize,
    /// Callbacks de timers JS déclenchés par tick (étape 52,
    /// borne ADR-0019 — une page enregistrant des milliers
    /// de timers échus ne monopolise pas la pompe ; le reste
    /// attend le tick suivant).
    pub max_timer_jobs: usize,
    /// Octets de corps reçus entre deux peintures progressives
    /// (étape 53 — cadence « N octets » du relayout pendant le
    /// téléchargement ; `0` = peinture à chaque morceau reçu).
    pub paint_chunk_bytes: usize,
    /// Intervalle minimal entre deux peintures progressives, en
    /// millisecondes (étape 53 — cadence « N ms » ; `0` = pas de
    /// seuil temporel, seuls les octets cadencent).
    pub paint_interval_ms: u64,
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
            max_scripts: 32,
            max_modules: 64,
            max_inflight: 6,
            max_script_net: 16,
            max_timer_jobs: 128,
            paint_chunk_bytes: 32 * 1024,
            paint_interval_ms: 100,
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
    /// Bornes scheduler (étape 50) : `<script src>` par
    /// document, modules transitifs par graphe, fetches
    /// simultanés par onglet.
    max_scripts: usize,
    max_modules: usize,
    max_inflight: usize,
    /// Jar à cookies partagé — le daemon le branche aussi
    /// sur le `CookieTransport` pour que `document.cookie`
    /// et les navigations voient le même état (étape 51).
    cookies: Arc<CookieJar>,
    /// `localStorage` par origine sérialisée — snapshot
    /// installé au spawn du contexte, map drainée aux points
    /// de synchro, persistée via le profil.
    local_storage: HashMap<String, BTreeMap<String, String>>,
    /// Canal des réponses de requêtes script-initiées
    /// (tâches tokio → points de synchro — ADR-0019 exige
    /// des intentions drainées, jamais de callback
    /// ré-entrant dans le pipeline).
    net_tx: Sender<NetDone>,
    net_rx: Receiver<NetDone>,
    /// Fetches script-initiés en vol, `(tab, handle)` →
    /// tâche tokio — `Abort` moteur ou fermeture d'onglet
    /// tuent la jointure.
    net_tasks: HashMap<(TabId, u64), JoinHandle<()>>,
    /// Quota de requêtes script-initiées simultanées par
    /// onglet (`max_script_net` de config, ADR-0019).
    max_script_net: usize,
    /// Quota de callbacks de timers déclenchés par tick
    /// (`max_timer_jobs` de config, étape 52 — borne
    /// ADR-0019).
    max_timer_jobs: usize,
    /// Octets reçus entre peintures progressives
    /// (`paint_chunk_bytes` de config, étape 53).
    paint_chunk_bytes: usize,
    /// Intervalle minimal entre peintures progressives
    /// (`paint_interval_ms` de config, étape 53).
    paint_interval: std::time::Duration,
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
        let (net_tx, net_rx) = channel();
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
            max_scripts: cfg.max_scripts,
            max_modules: cfg.max_modules,
            max_inflight: cfg.max_inflight,
            cookies: Arc::new(CookieJar::new()),
            local_storage: HashMap::new(),
            net_tx,
            net_rx,
            net_tasks: HashMap::new(),
            max_script_net: cfg.max_script_net,
            max_timer_jobs: cfg.max_timer_jobs,
            paint_chunk_bytes: cfg.paint_chunk_bytes,
            paint_interval: std::time::Duration::from_millis(cfg.paint_interval_ms),
        }
    }

    /// Branche la fabrique de moteurs JS (`DilloEngine::factory()`
    /// côté infra, `null_factory()` pour un no-op, `None` par
    /// défaut = scripts ignorés). Un contexte neuf est créé à
    /// chaque navigation qui rencontre un `<script>`.
    pub fn set_js_factory(&mut self, f: JsFactory) {
        self.js_factory = Some(f);
    }

    /// Jar à cookies de la session — l'hôte le partage avec
    /// son `CookieTransport` pour que `document.cookie`,
    /// les XHR et les navigations voient le même état.
    #[must_use]
    pub fn cookie_jar(&self) -> Arc<CookieJar> {
        self.cookies.clone()
    }

    /// Remplace le jar à cookies (restauration de profil
    /// ou jar pré-peuplé côté hôte).
    pub fn set_cookie_jar(&mut self, jar: Arc<CookieJar>) {
        self.cookies = jar;
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
                hovered: None,
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
        // Requêtes script-initiées en vol : annulées — leurs
        // réponses éventuelles arriveraient à un onglet mort.
        self.net_tasks.retain(|(t, _), j| {
            if *t == tab {
                j.abort();
                false
            } else {
                true
            }
        });
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

    /// Fait avancer les transitions CSS en vol (étape 45) :
    /// relayout + repaint si des transitions sont actives —
    /// l'animation est pilotée par le pull de frames de l'UI
    /// (le moteur n'a pas de boucle vsync propre ; `rAF` vit
    /// côté `dillo-js`). `Ok(true)` = une nouvelle frame a
    /// été produite et `FrameInvalidated` émis.
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
        let active = self
            .tabs
            .get(&tab)
            .is_some_and(|t| !t.transitions.is_empty());
        if !active {
            // Les timers ont pu invalider seuls (synchro DOM)
            // — pas de relayout gratuit en plus.
            return Ok(jobs > 0);
        }
        self.relayout(tab)?;
        let _ = self.events.send(CoreEvent::FrameInvalidated { tab });
        Ok(true)
    }

    /// Pompe les jobs JS dus d'un onglet — timers bornés à
    /// `max_timer_jobs` (ADR-0019) + microtasks +
    /// `MutationObserver` (JS-C26, livré au même checkpoint) —
    /// puis synchronise DOM, storage, cookies et net aux
    /// mêmes points de synchro que les événements. Renvoie le
    /// nombre de jobs exécutés.
    ///
    /// Synchrone : les intentions de navigation émises par un
    /// timer sont à drainer ensuite via
    /// [`Self::drain_script_nav`] (async).
    pub fn pump_timers(&mut self, tab: TabId) -> usize {
        let max = self.max_timer_jobs;
        let (jobs, mutated) = {
            let Some(entry) = self.tabs.get_mut(&tab) else {
                return 0;
            };
            let Some(eng) = entry.js.as_mut() else {
                return 0;
            };
            (eng.run_pending_bounded(max), eng.sync_dom())
        };
        if let Some(html) = mutated {
            self.apply_dom_sync(tab, &html, true);
        }
        if jobs > 0 {
            // Les callbacks ont pu émettre XHR/cookies/storage.
            self.after_script(tab);
            self.poll_net();
        }
        jobs
    }

    /// [`Self::pump_timers`] sur tous les onglets — le
    /// battement de l'hôte (daemon, CLI) itère ici.
    pub fn pump_timers_all(&mut self) -> usize {
        let tabs: Vec<TabId> = self.tabs.keys().copied().collect();
        tabs.into_iter().map(|t| self.pump_timers(t)).sum()
    }

    /// Plus proche échéance de timer JS tous onglets — délai
    /// relatif recalculé par chaque moteur (`None` = aucun
    /// timer armé). L'hôte dort jusqu'à `min(délai,
    /// battement)`.
    #[must_use]
    pub fn next_timer_deadline(&mut self) -> Option<std::time::Duration> {
        self.tabs
            .values_mut()
            .filter_map(|e| e.js.as_mut())
            .filter_map(|eng| eng.next_timer_deadline())
            .min()
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
        let mut resp = self
            .transport
            .fetch_stream(&req)
            .await
            .map_err(|e| CoreError::Fetch(e.to_string()))?;
        if !(200..300).contains(&resp.status) {
            return Err(CoreError::HttpStatus(resp.status));
        }
        // Parse incrémental (étape 53) : chaque morceau de corps
        // décodé alimente le tree-builder — `TreeBuilder::push`
        // garantit un arbre cohérent à frontière de token (R-1) —
        // et la peinture est cadencée (N octets ou N ms) : la
        // page se peint pendant le téléchargement.
        let mut tb = TreeBuilder::new();
        let mut dec = StreamDecoder::new();
        let mut html = String::new();
        let mut since_paint = 0usize;
        let mut last_paint = Instant::now();
        while let Some(chunk) = resp.body.recv().await {
            let bytes = chunk.map_err(|e| CoreError::Fetch(e.to_string()))?;
            let text = dec.feed(&bytes);
            html.push_str(&text);
            tb.push(&text);
            since_paint += bytes.len();
            if since_paint >= self.paint_chunk_bytes || last_paint.elapsed() >= self.paint_interval
            {
                self.paint_progress(tab, &tb);
                since_paint = 0;
                last_paint = Instant::now();
            }
        }
        let text = dec.finish();
        html.push_str(&text);
        tb.push(&text);
        tb.finish();
        let dom = tb.into_dom();
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
            entry.hovered = None;
            entry.js = None;
            entry.images.clear();
            entry.linked_css.clear();
            entry.prev_styles = HashMap::new();
            entry.transitions.clear();
            entry.dom = Some(dom);
        }
        self.webfonts.clear();
        self.load_subresources(tab, &html).await;
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
    /// Drainage des intentions de navigation script
    /// (`location`, `history`) accumulées — appelé après tout
    /// point de synchro producteur : événements, fin de
    /// chargement, tick timers (étape 52 — les hôtes la
    /// relancent après [`Self::pump_timers`]). Publique pour
    /// les pompes des hôtes (daemon, CLI) ; bornée à 8 sauts.
    pub async fn drain_script_nav(&mut self, tab: TabId) -> bool {
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

    /// `NetRequest::Fetch` → `Request` émettable (étape 51),
    /// `None` = bloquée : URL irrésoluble, mixed-content,
    /// verbe inconnu, cross-origin non-safelistée (ADR-0019).
    /// `Referer`/`Origin` posés à l'émission — une requête
    /// CORS porte `Origin` même en verbe safelisté (spec
    /// Fetch) ; `Cookie` injecté same-origin uniquement
    /// (`credentials` cross-origin hors MVP).
    fn script_request(
        &self,
        page: &Url,
        method: &str,
        raw_url: &str,
        headers: &[(String, String)],
        body: Option<&str>,
    ) -> Option<(Request, bool)> {
        let url = Url::resolve(page, raw_url.trim()).ok()?;
        if is_mixed_content(page, &url) {
            return None;
        }
        let m = parse_method(method)?;
        let cross = page.origin() != url.origin();
        // CORS minimal : cross-origin limité aux requêtes
        // simples — en-tête non safelisté = preflight requis,
        // bloqué au MVP (déviation documentée).
        if cross
            && !(cors_simple_method(m) && headers.iter().all(|(n, _)| cors_safelisted_header(n)))
        {
            return None;
        }
        let mut req = Request {
            method: m,
            url: url.clone(),
            headers: initiator_headers(page, &url, m),
            body: body.map(|b| b.as_bytes().to_vec()),
        };
        if cross
            && !req
                .headers
                .iter()
                .any(|(n, _)| n.eq_ignore_ascii_case("origin"))
        {
            req.headers
                .push(("origin".to_string(), origin_header_value(page)));
        }
        req.headers.extend(
            headers
                .iter()
                .filter(|(n, _)| !forbidden_request_header(n))
                .cloned(),
        );
        if !cross {
            let c = self.cookies.header_for(&url, true);
            if !c.is_empty() {
                req.headers.push(("Cookie".to_string(), c));
            }
        }
        Some((req, cross))
    }

    /// Draine les `NetRequest` du moteur (étape 51) : chaque
    /// `Fetch` admis devient une tâche tokio qui livrera sa
    /// réponse sur le canal (`pump_net` aux points de
    /// synchro) ; `Abort` tue la tâche corrélée. Quota
    /// `max_script_net` et requêtes bloquées → livraison
    /// immédiate d'un échec `status: 0` (`error` côté
    /// script — la page reste vivante, jamais de panic).
    fn drain_script_net(&mut self, tab: TabId) {
        let Some((page, reqs)) = self.tabs.get_mut(&tab).and_then(|e| {
            e.js.as_mut()
                .map(|eng| (e.url.clone(), eng.drain_net_requests()))
        }) else {
            return;
        };
        let Some(page) = page else { return };
        let mut failed = Vec::new();
        for r in reqs {
            match r {
                NetRequest::Fetch {
                    handle,
                    method,
                    url,
                    headers,
                    body,
                } => {
                    let inflight = self.net_tasks.keys().filter(|(t, _)| *t == tab).count();
                    let built = if inflight >= self.max_script_net {
                        tracing::trace!(tab, handle, "quota requêtes script atteint");
                        None
                    } else {
                        self.script_request(&page, &method, &url, &headers, body.as_deref())
                    };
                    let Some((req, cross)) = built else {
                        failed.push(handle);
                        continue;
                    };
                    let tr = self.transport.clone();
                    let tx = self.net_tx.clone();
                    let page_origin = origin_header_value(&page);
                    let req_url = req.url.clone();
                    let j = tokio::spawn(async move {
                        let out = tr.fetch(&req).await;
                        let _ = tx.send(NetDone {
                            tab,
                            handle,
                            cross,
                            page_origin,
                            req_url,
                            out,
                        });
                    });
                    self.net_tasks.insert((tab, handle), j);
                }
                NetRequest::Abort { handle } => {
                    if let Some(j) = self.net_tasks.remove(&(tab, handle)) {
                        j.abort();
                    }
                }
            }
        }
        // Échecs immédiats (bloquée/quota) : livrés comme un
        // échec réseau — déclenche `error`, jamais de panic.
        for handle in failed {
            self.deliver_net_response(
                tab,
                NetResponse {
                    handle,
                    status: 0,
                    status_text: String::new(),
                    headers: Vec::new(),
                    body: Vec::new(),
                },
            );
        }
    }

    /// Livre une `NetResponse` au moteur puis applique le
    /// tour post-script : microtasks, resynchro DOM +
    /// relayout, drains (réseau chaîné, cookies, storage).
    fn deliver_net_response(&mut self, tab: TabId, resp: NetResponse) {
        let mutated = {
            let Some(entry) = self.tabs.get_mut(&tab) else {
                return;
            };
            let Some(eng) = entry.js.as_mut() else {
                return;
            };
            if let Err(e) = eng.deliver_net_response(resp) {
                tracing::debug!(tab, error = %e, "livraison réseau js");
                let _ = self.events.send(CoreEvent::PipelineError {
                    tab: Some(tab),
                    message: e.to_string(),
                });
            }
            let n = eng.run_pending();
            tracing::trace!(tab, jobs = n, "jobs js après réponse réseau");
            eng.sync_dom()
        };
        if let Some(html) = mutated {
            self.apply_dom_sync(tab, &html, true);
        }
        self.after_script(tab);
    }

    /// Points de synchro post-script (étape 51) : draine les
    /// écritures `document.cookie` vers le jar partagé
    /// (`HttpOnly` filtré — non posable par un script), le
    /// `localStorage` vers la map d'origine + le profil, et
    /// les nouvelles requêtes réseau vers le scheduler.
    fn after_script(&mut self, tab: TabId) {
        let Some((page, writes, map)) = self.tabs.get_mut(&tab).and_then(|e| {
            e.js.as_mut().map(|eng| {
                (
                    e.url.clone(),
                    eng.drain_cookie_writes(),
                    eng.drain_storage(),
                )
            })
        }) else {
            return;
        };
        if let Some(page) = page {
            for w in &writes {
                self.cookies.set_from_header(&page, &strip_http_only(w));
            }
            let key = page.origin().to_string();
            // Persisté seulement sur changement réel : une
            // map vidée par script est sauvée (effacement),
            // un contexte qui n'a jamais touché storage ne
            // produit aucune écriture.
            let changed = match self.local_storage.get(&key) {
                Some(prev) => *prev != map,
                None => !map.is_empty(),
            };
            if changed {
                self.local_storage.insert(key.clone(), map.clone());
                if let Some(s) = &self.store {
                    s.save_storage(&key, &map);
                }
            }
        }
        self.drain_script_net(tab);
    }

    /// Livre les réponses arrivées — synchrone, sans yield :
    /// appelée aux points de synchro internes (événements).
    fn poll_net(&mut self) -> usize {
        let mut done = Vec::new();
        while let Ok(d) = self.net_rx.try_recv() {
            done.push(d);
        }
        let n = done.len();
        for d in done {
            self.net_tasks.remove(&(d.tab, d.handle));
            if !d.cross {
                if let Ok(resp) = &d.out {
                    for v in resp.headers_all("set-cookie") {
                        self.cookies.set_from_header(&d.req_url, v);
                    }
                }
            }
            let resp = match &d.out {
                Ok(resp) if !d.cross || cors_readable(&d.page_origin, resp) => NetResponse {
                    handle: d.handle,
                    status: resp.status,
                    status_text: resp.reason.clone(),
                    headers: resp.headers.clone(),
                    body: resp.body.clone(),
                },
                _ => NetResponse {
                    handle: d.handle,
                    status: 0,
                    status_text: String::new(),
                    headers: Vec::new(),
                    body: Vec::new(),
                },
            };
            self.deliver_net_response(d.tab, resp);
        }
        n
    }

    /// Pompe les réponses des requêtes script-initiées
    /// (étape 51) — cède d'abord le tour pour laisser les
    /// tâches produire, puis livre ce qui est arrivé.
    /// Renvoie le nombre livré. La session n'a pas
    /// d'horloge propre : l'hôte appelle périodiquement
    /// (daemon : battement dédié ; CLI/tests : après
    /// navigation) — jamais de callback ré-entrant.
    pub async fn pump_net(&mut self) -> usize {
        tokio::task::yield_now().await;
        self.poll_net()
    }

    /// Requêtes script-initiées encore en vol (tous onglets)
    /// — condition d'arrêt des pompes bornées de l'hôte
    /// (CLI, tests).
    #[must_use]
    pub fn net_pending(&self) -> usize {
        self.net_tasks.len()
    }

    /// Fetch des sous-ressources + exécution des `<script>`
    /// (étape 50, ADR-0012 côté session). **Une seule vague
    /// parallèle bornée** — `max_inflight` requêtes en vol via
    /// `FuturesUnordered`, émises par priorité
    /// `CSS > script > img` (le document est déjà servi) ;
    /// `Referer`/`Origin` sont calculés à l'émission
    /// (`initiator_headers`), le mixed-content filtré à la
    /// collecte.
    ///
    /// `<script>` — collecte en ordre document, trois files :
    /// - classiques (inline ou `src` sans `async`/`defer`) :
    ///   file « parse », évalués en ordre document — le parse
    ///   one-shot ne bloque pas, chaque externe attend sa
    ///   source fetchée (approximation « parser-blocking »
    ///   documentée : un classique voit le document complet) ;
    /// - `defer` et modules non-`async` : file post-parse,
    ///   ordre document ;
    /// - `async` : classiques évalués **en ligne** à réception
    ///   dans la vague, modules bufferisés puis évalués avant
    ///   la file post-parse.
    /// - `type="module"` : graphe statique **préfetché** borné
    ///   (`max_modules`) via `module_specifiers` puis
    ///   `eval_module` — le `ModuleLoader` reçoit une map
    ///   `url → source` (jamais de fetch ré-entrant depuis le
    ///   thread moteur : deadlock). Modules CORS : le fetch
    ///   cross-origin n'est pas vérifié au MVP (réévaluation
    ///   ADR-0019, étape 51).
    ///
    /// Tout échec individuel (réseau, statut, borne) saute la
    /// ressource — la page dégrade, jamais de panic.
    async fn load_subresources(&mut self, tab: TabId, html: &str) {
        let (Some(base), Some(dom)) = (
            self.tabs.get(&tab).and_then(|t| t.url.clone()),
            self.tabs.get(&tab).and_then(|t| t.dom.as_ref()),
        ) else {
            return;
        };
        let scripting = self.js_factory.is_some();
        let (sheet_urls, scripts, img_urls) = self.plan_subresources(dom, &base, scripting);

        // Slots de réception indexés par ordre document ;
        // `slot_items[slot]` = index de l'item script.
        let mut css_slots: Vec<Option<String>> = vec![None; sheet_urls.len()];
        let slot_items: Vec<usize> = scripts
            .iter()
            .enumerate()
            .filter_map(|(i, s)| s.slot.map(|_| i))
            .collect();
        let mut script_srcs: Vec<Option<String>> = vec![None; slot_items.len()];
        let mut img_bodies: HashMap<String, Vec<u8>> = HashMap::new();
        // Modules `async` arrivés pendant la vague : `(url de
        // l'entrée, corps)` en ordre d'arrivée.
        let mut async_mods: Vec<(Url, String)> = Vec::new();

        // Contexte JS avant la vague : un `async` classique est
        // évalué dès sa complétion — le miroir doit être posé.
        let mut eng = self.spawn_engine(tab, html, !scripts.is_empty());

        let transport = self.transport.clone();
        let events = self.events.clone();
        let eval_arrival = |eng: &mut Option<Box<dyn JsEngine>>, src: &str| {
            if let Some(e) = eng.as_mut() {
                if let Err(err) = e.eval(src) {
                    tracing::debug!(tab, error = %err, "script async en échec");
                    let _ = events.send(CoreEvent::PipelineError {
                        tab: Some(tab),
                        message: err.to_string(),
                    });
                }
            }
        };
        // Émission par priorité : CSS → scripts → images.
        let jobs = sub_jobs(&base, &sheet_urls, &scripts, &img_urls);
        fetch_wave(&transport, self.max_inflight, jobs, |tag, res| {
            let Ok(resp) = res else { return };
            if !(200..300).contains(&resp.status) {
                return;
            }
            match tag {
                SubTag::Css(i) => {
                    css_slots[i] = Some(decode_stylesheet(&resp.body, &resp.headers));
                }
                SubTag::Script(slot) => {
                    let item = &scripts[slot_items[slot]];
                    // Étape 53b : charset déclaré (classique) →
                    // BOM → UTF-8 — pas le sniffer HTML.
                    let text = decode_subresource(&resp.body, item.charset.as_deref());
                    if item.exec == ScriptExec::Arrival {
                        if item.module {
                            if let Some(u) = &item.src {
                                async_mods.push((u.clone(), text));
                            }
                        } else {
                            eval_arrival(&mut eng, &text);
                        }
                    } else {
                        script_srcs[slot] = Some(text);
                    }
                }
                SubTag::Img(u) => {
                    img_bodies.insert(u.as_str(), resp.body);
                }
            }
        })
        .await;

        // Feuilles liées dans l'ordre document (slot vide =
        // feuille en échec → absente de la cascade).
        let mut css = String::new();
        for t in css_slots.iter().flatten() {
            css.push_str(t);
            css.push('\n');
        }
        if let Some(entry) = self.tabs.get_mut(&tab) {
            entry.linked_css = css;
        }

        let Some(mut eng) = eng else {
            self.insert_images(tab, &base, &img_bodies);
            return;
        };
        self.run_script_queues(tab, eng.as_mut(), &scripts, &script_srcs, async_mods, &base)
            .await;
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
        // Étape 51 : XHR/fetch émis pendant le chargement →
        // spawned ; une pompe livre les réponses déjà là,
        // l'hôte pompe périodiquement pour le reste.
        self.after_script(tab);
        self.pump_net().await;
        self.insert_images(tab, &base, &img_bodies);
    }

    /// Collecte des sous-ressources du document (étape 50) :
    /// URLs résolues ici (émission propre), mixed-content
    /// filtré, images dédupliquées — `(feuilles, scripts,
    /// images)` bornés par la config.
    fn plan_subresources(
        &self,
        dom: &Dom,
        base: &Url,
        scripting: bool,
    ) -> (Vec<Url>, Vec<ScriptItem>, Vec<Url>) {
        let sheets: Vec<Url> = collect_stylesheet_links(dom)
            .into_iter()
            .take(self.max_sheets)
            .filter_map(|h| Url::resolve(base, &h).ok())
            .filter(|u| !is_mixed_content(base, u))
            .collect();
        let scripts = collect_scripts(dom, base, scripting, self.max_scripts);
        let mut seen_img = std::collections::HashSet::new();
        let imgs: Vec<Url> = collect_images(dom)
            .into_iter()
            .filter_map(|(_, src)| Url::resolve(base, &src).ok())
            .filter(|u| !is_mixed_content(base, u) && seen_img.insert(u.as_str()))
            .take(self.max_images)
            .collect();
        (sheets, scripts, imgs)
    }

    /// Contexte JS de chargement : moteur neuf + miroir posé —
    /// `want = false` (aucun script collecté) → `None`, pas de
    /// moteur créé (règle budgets).
    fn spawn_engine(&self, tab: TabId, html: &str, want: bool) -> Option<Box<dyn JsEngine>> {
        if !want {
            return None;
        }
        self.js_factory.clone().map(|f| {
            let (page_url, hist_len) = self.tabs.get(&tab).map_or_else(
                || (String::new(), 1),
                |t| {
                    (
                        t.url.as_ref().map_or_else(String::new, Url::to_string),
                        u32::try_from(t.hist.len()).unwrap_or(u32::MAX),
                    )
                },
            );
            let mut e = f();
            e.set_budget(self.js_ops);
            e.install_dom(html, &page_url, hist_len);
            // Étape 51 : état d'origine injecté au contexte —
            // `localStorage` (map session > profil persisté)
            // et snapshot cookie non-HttpOnly
            // (`document.cookie` en lecture).
            if let Ok(u) = Url::parse(&page_url) {
                let key = u.origin().to_string();
                let local = self
                    .local_storage
                    .get(&key)
                    .cloned()
                    .or_else(|| self.store.as_ref().map(|s| s.load_storage(&key)))
                    .unwrap_or_default();
                e.install_storage(local);
                e.set_cookie_snapshot(&self.cookies.header_for(&u, false));
            }
            e
        })
    }

    /// Exécute les trois files de scripts dans l'ordre spec :
    /// classiques « parse » (ordre document) → modules `async`
    /// arrivés pendant la vague (ordre d'arrivée — le graphe
    /// est préfetché à la suite, approximation « à
    /// réception ») → file post-parse (`defer` et modules,
    /// ordre document).
    async fn run_script_queues(
        &mut self,
        tab: TabId,
        eng: &mut dyn JsEngine,
        scripts: &[ScriptItem],
        script_srcs: &[Option<String>],
        async_mods: Vec<(Url, String)>,
        base: &Url,
    ) {
        for item in scripts.iter().filter(|i| i.exec == ScriptExec::Parse) {
            let Some(src) = item_source(item, script_srcs) else {
                continue;
            };
            self.eval_or_report(tab, eng, src);
        }
        for (u, src) in async_mods {
            let name = u.as_str();
            self.eval_module_src(tab, eng, name, &u, src).await;
        }
        for (i, item) in scripts.iter().enumerate() {
            if item.exec != ScriptExec::Post {
                continue;
            }
            let Some(src) = item_source(item, script_srcs) else {
                continue;
            };
            if item.module {
                // Entrée inline : nom synthétique unique, aliasé
                // vers la page pour la résolution des deps.
                let (name, base_u) = item.src.clone().map_or_else(
                    || (format!("inline:{i}"), base.clone()),
                    |u| (u.as_str(), u),
                );
                self.eval_module_src(tab, eng, name, &base_u, src.to_string())
                    .await;
            } else {
                self.eval_or_report(tab, eng, src);
            }
        }
    }

    /// `eval` avec rapport : un échec émet `PipelineError`
    /// sans faire échouer la page (dégradation, règle 3).
    fn eval_or_report(&mut self, tab: TabId, eng: &mut dyn JsEngine, src: &str) {
        if let Err(e) = eng.eval(src) {
            tracing::debug!(tab, error = %e, "script en échec");
            let _ = self.events.send(CoreEvent::PipelineError {
                tab: Some(tab),
                message: e.to_string(),
            });
        }
    }

    /// Évalue un `<script type="module">` : prefetch BFS borné
    /// du graphe statique puis `eval_module`. `name` = clé de
    /// registre du moteur (URL de l'entrée ou nom synthétique
    /// inline), `base` = URL de résolution des spécificateurs.
    async fn eval_module_src(
        &mut self,
        tab: TabId,
        eng: &mut dyn JsEngine,
        name: String,
        base: &Url,
        src: String,
    ) {
        let mut graph = ModuleGraph {
            sources: HashMap::from([(name.clone(), src.clone())]),
            alias: HashMap::from([(name.clone(), base.as_str())]),
        };
        let page = self
            .tabs
            .get(&tab)
            .and_then(|t| t.url.clone())
            .unwrap_or_else(|| base.clone());
        // `Arc` cloné : la future de fetch n'a pas besoin de
        // `&Session` (`dyn JsEngine` n'est pas `Sync`).
        let transport = Arc::clone(&self.transport);
        Self::fetch_module_sources(
            &transport,
            self.max_inflight,
            self.max_modules,
            &page,
            base,
            &src,
            &mut graph,
        )
        .await;
        let ModuleGraph { sources, alias } = graph;
        let resolve: UrlResolver = Arc::new(move |spec, referrer| {
            let b = alias.get(referrer).map_or(referrer, String::as_str);
            Url::parse(b)
                .ok()
                .and_then(|u| Url::resolve(&u, spec).ok())
                .map(|u| u.as_str())
        });
        if let Err(e) = eng.eval_module(&name, sources, resolve) {
            tracing::debug!(tab, error = %e, "module en échec");
            let _ = self.events.send(CoreEvent::PipelineError {
                tab: Some(tab),
                message: e.to_string(),
            });
        }
    }

    /// Remplit `graph` en BFS du graphe statique
    /// d'`entry_src` — vagues parallèles `max_inflight`,
    /// `sources` plafonné à `max_modules` (l'entrée incluse).
    /// `page` porte la politique (`Referer`, mixed-content) ;
    /// `entry_base` sert à résoudre les spécificateurs.
    /// Associée sans `&self` : `dyn JsEngine` n'est pas
    /// `Sync`, `&Session` ne peut donc pas traverser un
    /// `.await` dans une future `Send`.
    async fn fetch_module_sources(
        transport: &Arc<dyn HttpTransport>,
        max_inflight: usize,
        max_modules: usize,
        page: &Url,
        entry_base: &Url,
        entry_src: &str,
        graph: &mut ModuleGraph,
    ) {
        let mut queued: std::collections::HashSet<String> =
            std::iter::once(entry_base.as_str()).collect();
        let mut frontier: Vec<(String, Url)> = Vec::new();
        for (spec, u) in module_deps(entry_src, entry_base) {
            graph
                .alias
                .entry(spec.clone())
                .or_insert_with(|| u.as_str());
            if !is_mixed_content(page, &u) && queued.insert(u.as_str()) {
                frontier.push((spec, u));
            }
        }
        while !frontier.is_empty() && graph.sources.len() < max_modules {
            let room = (max_modules - graph.sources.len()).min(frontier.len());
            let mut jobs = VecDeque::new();
            for (_, u) in frontier.drain(..room) {
                jobs.push_back(((u.as_str(), u.clone()), sub_request(page, &u)));
            }
            let mut fetched = Vec::new();
            fetch_wave(transport, max_inflight, jobs, |tag, res| {
                fetched.push((tag, res));
            })
            .await;
            let mut next = Vec::new();
            for ((key, u), res) in fetched {
                let Ok(resp) = res else { continue };
                if !(200..300).contains(&resp.status) {
                    continue;
                }
                let text = decode_subresource(&resp.body, None);
                graph.sources.insert(key, text.clone());
                for (spec2, u2) in module_deps(&text, &u) {
                    graph
                        .alias
                        .entry(spec2.clone())
                        .or_insert_with(|| u2.as_str());
                    if !is_mixed_content(page, &u2) && queued.insert(u2.as_str()) {
                        next.push((spec2, u2));
                    }
                }
            }
            frontier = next;
        }
    }

    /// Décode + insère les corps `<img>` dans la table de
    /// l'onglet — **après** la resynchro DOM : les `NodeId`
    /// changent au re-parse, les corps sont re-matchés par
    /// `src` résolue en ordre document (un fetch = tous les
    /// `<img>` homonymes, décodage dédupliqué).
    fn insert_images(&mut self, tab: TabId, base: &Url, bodies: &HashMap<String, Vec<u8>>) {
        if bodies.is_empty() {
            return;
        }
        let max_px = self.image_max_px;
        let Some(entry) = self.tabs.get_mut(&tab) else {
            return;
        };
        let Some(dom) = entry.dom.as_ref() else {
            return;
        };
        let items = collect_images(dom);
        let mut decoded: HashMap<String, Option<Arc<DecodedImage>>> = HashMap::new();
        for (node, src) in items {
            let Ok(u) = Url::resolve(base, &src) else {
                continue;
            };
            let Some(body) = bodies.get(&u.as_str()) else {
                continue;
            };
            let img = decoded
                .entry(u.as_str())
                .or_insert_with(|| decode_image(body, max_px).map(Arc::new));
            if let Some(img) = img {
                entry.images.insert(node, img.clone());
            }
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

    /// Resynchronise le document depuis le miroir JS muté :
    /// re-parse du HTML sérialisé, relecture du titre
    /// (`TitleChanged` si changé). `focused`/`hovered` sont
    /// conservés — ce sont des chemins, pas des `NodeId`
    /// (étape 52). `relayout` contrôle
    /// l'invalidation immédiate — vrai depuis les événements,
    /// faux pendant la navigation (qui relayout juste après).
    fn apply_dom_sync(&mut self, tab: TabId, html: &str, relayout: bool) {
        let new_dom = TreeBuilder::parse(html);
        let new_title = extract_title(&new_dom);
        if let Some(entry) = self.tabs.get_mut(&tab) {
            entry.dom = Some(new_dom);
            // `focused`/`hovered` sont des **chemins** — ils
            // survivent au re-parse et se re-résolvent à
            // l'usage (étape 52) ; un nœud supprimé
            // simplement ne résout plus.
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
        // Le DOM sort de l'onglet le temps du pipeline —
        // `relayout_dom` veut `&mut self` et un `&Dom` disjoint.
        let dom = self
            .tabs
            .get_mut(&tab)
            .and_then(|e| e.dom.take())
            .ok_or(CoreError::EmptyDocument)?;
        let r = self.relayout_dom(tab, &dom);
        if let Some(e) = self.tabs.get_mut(&tab) {
            e.dom = Some(dom);
        }
        r
    }

    /// Peinture progressive (étape 53) : relayout + raster sur
    /// le DOM **en construction** du `TreeBuilder` — cohérent à
    /// frontière de token (R-1). L'échec est ignoré : un arbre
    /// trop tôt (pas encore de `<html>`) ne peint rien.
    fn paint_progress(&mut self, tab: TabId, tb: &TreeBuilder) {
        if self.relayout_dom(tab, tb.dom()).is_ok() {
            let _ = self.events.send(CoreEvent::FrameInvalidated { tab });
        }
    }

    /// Pipeline style → layout → raster sur `dom` (emprunté —
    /// `relayout` pour le DOM posé, `paint_progress` pour le
    /// DOM en construction). Écrit `tree`/`doc_h`/frame/png.
    fn relayout_dom(&mut self, tab: TabId, dom: &Dom) -> Result<(), CoreError> {
        let entry = self.tabs.get_mut(&tab).ok_or(CoreError::UnknownTab(tab))?;

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
        let Some(root_el) = dom.document_element() else {
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
                // `keydown` sur le champ focalisé, sinon niveau
                // document (étape 52) — `preventDefault`
                // supprime l'édition et la soumission `Enter`.
                if self.dispatch_key(tab, "keydown") {
                    return Ok(InputOutcome::Ignored);
                }
                // Soumission implicite : `Enter` dans un champ
                // d'un `<form>` soumet comme le bouton par
                // défaut (événement `submit` d'abord —
                // `preventDefault` annule).
                if key == "Enter" {
                    if let Some(form) = self.focused_form(tab) {
                        self.set_focus(tab, None);
                        if !self.dispatch_dom_event(tab, "submit", form) {
                            self.submit_form(tab, form, None).await?;
                            return Ok(InputOutcome::Navigated);
                        }
                        return Ok(InputOutcome::Ignored);
                    }
                }
                self.handle_key(tab, &key)
            }
            InputEvent::KeyUp { key: _ } => {
                self.dispatch_key(tab, "keyup");
                Ok(InputOutcome::Ignored)
            }
            InputEvent::Scroll { dy } => {
                // `wheel` (annulable) sur le nœud survolé ou le
                // document — `preventDefault` bloque le scroll.
                if !self.dispatch_wheel(tab) {
                    self.scroll_by(tab, dy)?;
                    // `scroll` niveau document (non annulable).
                    self.dispatch_doc_event(tab, "scroll");
                }
                Ok(InputOutcome::Scrolled {
                    scroll_y: self.tabs.get(&tab).map_or(0.0, |t| t.scroll_y),
                })
            }
            InputEvent::MouseMove { x, y } => self.handle_mouse_move(tab, x, y).await,
            InputEvent::Click { x, y } => self.handle_click(tab, x, y).await,
        }
    }

    /// Survol suivi (étape 52) : `mouseout` sur l'ancienne
    /// cible, `mouseover`/`mousemove` sur la nouvelle — déjà
    /// coalescé côté client (ADR-0003), un événement par
    /// `move`. Les dispatches se font par **chemin** : chaque
    /// handler peut muter le miroir et re-parser le document —
    /// un `NodeId` capté en début de bras serait mort dès le
    /// premier dispatch.
    async fn handle_mouse_move(
        &mut self,
        tab: TabId,
        x: f32,
        y: f32,
    ) -> Result<InputOutcome, CoreError> {
        let node = self.node_at(tab, x, y);
        let path = node.and_then(|n| {
            self.tabs
                .get(&tab)
                .and_then(|e| e.dom.as_ref())
                .and_then(|d| element_path(d, n))
        });
        let prev = self.tabs.get(&tab).and_then(|t| t.hovered.clone());
        if prev != path {
            if let Some(o) = prev {
                self.dispatch_path_event(tab, "mouseout", &o);
            }
            if let Some(p) = &path {
                self.dispatch_path_event(tab, "mouseover", p);
            }
            if let Some(entry) = self.tabs.get_mut(&tab) {
                entry.hovered.clone_from(&path);
            }
        }
        if let Some(p) = &path {
            self.dispatch_path_event(tab, "mousemove", p);
        }
        if self.drain_script_nav(tab).await {
            return Ok(InputOutcome::Navigated);
        }
        Ok(InputOutcome::Hover {
            url: self.link_at(tab, x, y),
            editable: self.field_at(tab, x, y).is_some(),
        })
    }

    /// Clic : `click` DOM d'abord (`preventDefault` annule
    /// l'action par défaut), bascule des contrôles puis
    /// `input`/`change`, focus sur les champs, `submit` des
    /// formulaires, navigation des liens.
    async fn handle_click(
        &mut self,
        tab: TabId,
        x: f32,
        y: f32,
    ) -> Result<InputOutcome, CoreError> {
        let node = self.node_at(tab, x, y);
        // Contrôle à bascule : activation AVANT `click`
        // (spec DOM — le handler lit `checked` post-bascule
        // via le miroir synchronisé) ; `preventDefault`
        // annule et restaure l'état.
        let toggled = self.toggle_at(tab, x, y);
        // Chemin du contrôle capté AVANT le `click` : le
        // handler peut re-parser le document — le `NodeId`
        // meurt, le chemin survit.
        let ctrl_path = toggled.and_then(|c| {
            self.tabs
                .get(&tab)
                .and_then(|e| e.dom.as_ref())
                .and_then(|d| element_path(d, c))
        });
        if let Some(ctrl) = toggled {
            self.flip_checked(tab, ctrl)?;
        }
        // L'événement DOM précède l'action par défaut : un
        // handler peut muter la page (resynchronisée et
        // re-peinte ici) ou `preventDefault`.
        let prevented = node.is_some_and(|n| self.dispatch_dom_event(tab, "click", n));
        if toggled.is_some() {
            // Re-résolution sur l'arbre courant pour le revert
            // ou la suite `input`/`change`.
            let ctrl = self.resolve_path(tab, ctrl_path.as_deref());
            if prevented {
                if let Some(c) = ctrl {
                    self.flip_checked(tab, c)?;
                }
            } else if let Some(p) = &ctrl_path {
                // `input` puis `change` — ordre DOM.
                self.dispatch_path_event(tab, "input", p);
                self.dispatch_path_event(tab, "change", p);
            }
            self.relayout(tab)?;
            let _ = self.events.send(CoreEvent::FrameInvalidated { tab });
            return Ok(if prevented {
                InputOutcome::Ignored
            } else {
                InputOutcome::Edited
            });
        }
        // Navigation script (`location`, `history`) : priorité
        // sur l'action par défaut du clic.
        if self.drain_script_nav(tab).await {
            return Ok(InputOutcome::Navigated);
        }
        if let Some(field) = self.field_at(tab, x, y) {
            // `blur` sur l'ancien focus puis `focus` sur le
            // champ (étape 52).
            self.set_focus(tab, Some(field));
            return Ok(InputOutcome::Focused);
        }
        self.set_focus(tab, None);
        // `<select>` : cycle d'options — le sélecteur natif
        // délégué UI est hors scope (ADR-0014).
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
        // Contrôle submit : l'événement `submit` du formulaire
        // part avant l'action par défaut. `form`/`sub` sont
        // re-résolus après le dispatch — un handler peut
        // re-parser le document (ids morts, chemins
        // survivants).
        if let Some((form, sub)) = self.submit_at(tab, x, y) {
            let paths = {
                let dom = self.tabs.get(&tab).and_then(|e| e.dom.as_ref());
                (
                    dom.and_then(|d| element_path(d, form)),
                    dom.and_then(|d| element_path(d, sub)),
                )
            };
            if !prevented && !self.dispatch_dom_event(tab, "submit", form) {
                if self.drain_script_nav(tab).await {
                    return Ok(InputOutcome::Navigated);
                }
                let f = self.resolve_path(tab, paths.0.as_deref());
                let s = self.resolve_path(tab, paths.1.as_deref());
                if let (Some(f), Some(s)) = (f, s) {
                    self.submit_form(tab, f, Some(s)).await?;
                    return Ok(InputOutcome::Navigated);
                }
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
        let field = entry.focused.as_ref().and_then(|p| dom_path_node(dom, p))?;
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

    /// Édition du champ focalisé : `Backspace` efface,
    /// `Enter`/`Escape` sortent du champ, un caractère
    /// imprimable s'insère — puis `input` dispatché (valeur
    /// synchronisée au miroir d'abord), restyle + repaint.
    fn handle_key(&mut self, tab: TabId, key: &str) -> Result<InputOutcome, CoreError> {
        let Some(field) = self.focused_node(tab) else {
            tracing::trace!(tab, key, "touche sans champ focalisé");
            return Ok(InputOutcome::Ignored);
        };
        match key {
            "Enter" | "Escape" => {
                self.set_focus(tab, None);
                return Ok(InputOutcome::Ignored);
            }
            _ => {}
        }
        let value = {
            let entry = self.tabs.get_mut(&tab).ok_or(CoreError::UnknownTab(tab))?;
            let dom = entry.dom.as_mut().ok_or(CoreError::EmptyDocument)?;
            edit_field(dom, field, key)?;
            field_value(dom, field)
        };
        // Le handler `input` lit `.value` — le miroir est
        // synchronisé avant le dispatch (édition hors script).
        self.sync_mirror_attr(tab, field, "value", Some(&value));
        self.dispatch_dom_event(tab, "input", field);
        self.relayout(tab)?;
        let _ = self.events.send(CoreEvent::FrameInvalidated { tab });
        Ok(InputOutcome::Edited)
    }

    /// Dispatche l'événement `ty` sur `node` dans le miroir JS
    /// puis resynchronise les mutations DOM. Retourne
    /// `defaultPrevented`.
    fn dispatch_dom_event(&mut self, tab: TabId, ty: &str, node: NodeId) -> bool {
        let path = self
            .tabs
            .get(&tab)
            .and_then(|entry| entry.dom.as_ref().and_then(|d| element_path(d, node)));
        let Some(path) = path else { return false };
        self.dispatch_path_event(tab, ty, &path)
    }

    /// Événement niveau document (étape 52) — cible `<body>`
    /// côté miroir : `scroll`, `keydown`/`keyup`/`wheel` sans
    /// nœud porteur.
    fn dispatch_doc_event(&mut self, tab: TabId, ty: &str) -> bool {
        self.dispatch_path_event(tab, ty, &[])
    }

    /// Cœur du dispatch : `path` vers le miroir (vide =
    /// document), resynchronisation + drains des surfaces
    /// script (étapes 50-51) — point de synchro partagé par
    /// les événements utilisateur et le tick timers.
    fn dispatch_path_event(&mut self, tab: TabId, ty: &str, path: &[(String, u32)]) -> bool {
        let (prevented, mutated) = {
            let Some(entry) = self.tabs.get_mut(&tab) else {
                return false;
            };
            let Some(eng) = entry.js.as_mut() else {
                return false;
            };
            let p = match eng.dispatch_event(ty, path) {
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
            tracing::trace!(tab, jobs = n, "jobs js après événement");
            (p, eng.sync_dom())
        };
        if let Some(html) = mutated {
            self.apply_dom_sync(tab, &html, true);
        }
        // Les listeners ont pu émettre XHR/cookies/storage
        // (étape 51) — drains + livraison de ce qui est déjà
        // arrivé, sans yield (point de synchro interne).
        self.after_script(tab);
        self.poll_net();
        prevented
    }

    /// `keydown`/`keyup` : cible = champ focalisé s'il vit
    /// encore dans `dom`, document sinon. Retourne
    /// `defaultPrevented`.
    fn dispatch_key(&mut self, tab: TabId, ty: &str) -> bool {
        match self.focused_node(tab) {
            Some(n) => self.dispatch_dom_event(tab, ty, n),
            None => self.dispatch_doc_event(tab, ty),
        }
    }

    /// `wheel` : cible = nœud survolé s'il vit encore,
    /// document sinon. Retourne `defaultPrevented` — le
    /// scroll est alors supprimé.
    fn dispatch_wheel(&mut self, tab: TabId) -> bool {
        let path = self
            .tabs
            .get(&tab)
            .and_then(|t| t.hovered.clone())
            .filter(|p| {
                self.tabs
                    .get(&tab)
                    .and_then(|t| t.dom.as_ref())
                    .is_some_and(|d| dom_path_node(d, p).is_some())
            });
        match path {
            Some(p) => self.dispatch_path_event(tab, "wheel", &p),
            None => self.dispatch_doc_event(tab, "wheel"),
        }
    }

    /// Résout `path` vers le `NodeId` courant de `dom` —
    /// re-résolution après re-parse de synchro (étape 52).
    fn resolve_path(&self, tab: TabId, path: Option<&[(String, u32)]>) -> Option<NodeId> {
        path.and_then(|p| {
            self.tabs
                .get(&tab)
                .and_then(|e| e.dom.as_ref())
                .and_then(|d| dom_path_node(d, p))
        })
    }

    /// Champ focalisé résolu dans `dom` courant — `None` si
    /// le chemin stocké ne mène plus à un élément (le nœud a
    /// été supprimé par un handler ou un re-parse l'a
    /// déplacé).
    fn focused_node(&self, tab: TabId) -> Option<NodeId> {
        self.resolve_path(tab, self.tabs.get(&tab)?.focused.as_deref())
    }

    /// Focus champ (étape 52) : `blur` sur l'ancien nœud puis
    /// `focus` sur le nouveau — ordre navigateur. `None` =
    /// perte de focus seule. Les événements sont dispatchés
    /// par **chemin** : un handler `blur` qui mute le DOM
    /// re-parse le document — la cible `focus` est re-résolue
    /// ensuite sur l'arbre courant.
    fn set_focus(&mut self, tab: TabId, field: Option<NodeId>) {
        let entry = self.tabs.get(&tab);
        let new_path = field.and_then(|f| {
            entry
                .and_then(|e| e.dom.as_ref())
                .and_then(|d| element_path(d, f))
        });
        let old_path = entry.and_then(|e| e.focused.clone());
        if old_path == new_path {
            return;
        }
        if let Some(o) = old_path {
            self.dispatch_path_event(tab, "blur", &o);
        }
        if let Some(entry) = self.tabs.get_mut(&tab) {
            entry.focused.clone_from(&new_path);
        }
        if let Some(p) = new_path {
            self.dispatch_path_event(tab, "focus", &p);
        }
    }

    /// Pousse l'état `attr` d'un nœud hôte vers le miroir JS —
    /// les handlers d'événements lisent l'état courant quand
    /// la session a muté le document hors script (champ
    /// édité → `value`, case basculée → `checked`).
    fn sync_mirror_attr(&mut self, tab: TabId, node: NodeId, attr: &str, value: Option<&str>) {
        let path = self
            .tabs
            .get(&tab)
            .and_then(|e| e.dom.as_ref())
            .and_then(|d| element_path(d, node));
        let Some(path) = path else { return };
        if let Some(eng) = self.tabs.get_mut(&tab).and_then(|e| e.js.as_mut()) {
            eng.sync_mirror_attr(&path, attr, value);
        }
    }

    /// Bascule `checked` du contrôle dans le DOM hôte puis
    /// pousse l'attribut au miroir (étape 52 — l'activation
    /// précède le `click`, le revert suit `preventDefault`).
    fn flip_checked(&mut self, tab: TabId, ctrl: NodeId) -> Result<(), CoreError> {
        let checked = {
            let entry = self.tabs.get_mut(&tab).ok_or(CoreError::UnknownTab(tab))?;
            let dom = entry.dom.as_mut().ok_or(CoreError::EmptyDocument)?;
            toggle_checked(dom, ctrl)?;
            dom.attribute(ctrl, "checked").ok().flatten().is_some()
        };
        self.sync_mirror_attr(tab, ctrl, "checked", checked.then_some(""));
        Ok(())
    }

    /// Nœud DOM sous `(x, y)` viewport — hit-test sur l'arbre
    /// posé, scroll compensé.
    fn node_at(&self, tab: TabId, x: f32, y: f32) -> Option<NodeId> {
        let entry = self.tabs.get(&tab)?;
        let tree = entry.tree.as_ref()?;
        hit_test(tree, x, y + entry.scroll_y)
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
            .take_while(|&&c| c != cur)
            .filter(|&&c| dom.tag_name(c).is_ok_and(|t| t == tag))
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

/// Types `type=` exécutés comme script **classique** (HTML —
/// sous-ensemble des « JavaScript MIME types » usuels ; le
/// type absent/vaut `""` par défaut).
const CLASSIC_SCRIPT_TYPES: &[&str] = &[
    "",
    "text/javascript",
    "application/javascript",
    "text/ecmascript",
    "application/ecmascript",
    "text/jscript",
];

/// Phase d'exécution d'un `<script>` (étape 50) — l'ordre
/// document est conservé à l'intérieur de chaque file.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ScriptExec {
    /// Classique (inline ou `src`) : file « parse » — ordre
    /// document, évalué sitôt sa source disponible (le parse
    /// one-shot a déjà fini : approximation « parser-blocking »
    /// documentée — le script voit le document complet).
    Parse,
    /// `defer` ou module non-`async` : file post-parse, ordre
    /// document.
    Post,
    /// `async` (classique ou module) : à réception — le
    /// classique est évalué en ligne dans la vague, le module
    /// est bufferisé et évalué avant la file post-parse.
    Arrival,
}

/// Balise `<script>` exécutable, collectée en ordre document
/// (étape 50).
struct ScriptItem {
    /// `src` résolue — `Some` = script externe.
    src: Option<Url>,
    /// Corps inline (classique ou module).
    inline: Option<String>,
    /// `type="module"`.
    module: bool,
    /// File d'exécution.
    exec: ScriptExec,
    /// `charset` déclaré sur la balise (script classique
    /// externe seulement — ignoré par spec pour les modules,
    /// toujours UTF-8 ; étape 53b).
    charset: Option<String>,
    /// Slot de réception du corps fetché (index dans
    /// `script_srcs`) — `None` pour les inline.
    slot: Option<usize>,
}

/// `<script>` exécutables du document, ordre document —
/// `src` résolues, bornés `max_src`, mixed-content filtré.
/// `scripting = false` (pas de fabrique JS) → vide : scripting
/// désactivé = aucun fetch de script (spec HTML).
fn collect_scripts(dom: &Dom, base: &Url, scripting: bool, max_src: usize) -> Vec<ScriptItem> {
    let mut out = Vec::new();
    if !scripting {
        return out;
    }
    let mut slots = 0usize;
    let mut queue = vec![dom.document()];
    while let Some(n) = queue.pop() {
        if dom.tag_name(n).is_ok_and(|t| t == "script") {
            let ty = dom
                .attribute(n, "type")
                .ok()
                .flatten()
                .map_or_else(String::new, |t| t.trim().to_ascii_lowercase());
            let module = ty == "module";
            if module || CLASSIC_SCRIPT_TYPES.contains(&ty.as_str()) {
                // `nomodule` n'a d'effet que sur les classiques.
                let nomodule = dom.attribute(n, "nomodule").ok().flatten().is_some();
                if module || !nomodule {
                    let src_attr = dom
                        .attribute(n, "src")
                        .ok()
                        .flatten()
                        .filter(|s| !s.trim().is_empty());
                    if let Some(raw) = src_attr {
                        // Externe : `async` l'emporte sur `defer`
                        // (spec — les deux présents = async).
                        let async_ = dom.attribute(n, "async").ok().flatten().is_some();
                        let defer = !async_ && dom.attribute(n, "defer").ok().flatten().is_some();
                        let resolved = Url::resolve(base, raw.trim())
                            .ok()
                            .filter(|u| !is_mixed_content(base, u));
                        if let (Some(u), true) = (resolved, slots < max_src) {
                            out.push(ScriptItem {
                                src: Some(u),
                                inline: None,
                                charset: if module {
                                    None
                                } else {
                                    dom.attribute(n, "charset").ok().flatten()
                                },
                                module,
                                exec: if async_ {
                                    ScriptExec::Arrival
                                } else if module || defer {
                                    ScriptExec::Post
                                } else {
                                    ScriptExec::Parse
                                },
                                slot: {
                                    let s = slots;
                                    slots += 1;
                                    Some(s)
                                },
                            });
                        }
                        // Hors borne / irrésoluble / bloqué :
                        // balise inerte, non collectée.
                    } else {
                        let text = dom.text_content(n).unwrap_or_default();
                        if !text.trim().is_empty() {
                            // Inline : `async`/`defer` sans effet —
                            // classique = file parse, module = post.
                            out.push(ScriptItem {
                                src: None,
                                inline: Some(text),
                                charset: None,
                                module,
                                exec: if module {
                                    ScriptExec::Post
                                } else {
                                    ScriptExec::Parse
                                },
                                slot: None,
                            });
                        }
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

/// Corps d'un `ScriptItem` : inline direct, externe depuis son
/// slot (`None` = fetch en échec → le script est sauté).
fn item_source<'a>(item: &'a ScriptItem, srcs: &'a [Option<String>]) -> Option<&'a str> {
    if let Some(t) = &item.inline {
        return Some(t);
    }
    item.slot
        .and_then(|s| srcs.get(s).and_then(|o| o.as_deref()))
}

/// Routage d'une complétion de sous-ressource (étape 50).
enum SubTag {
    /// Slot feuille liée (ordre document).
    Css(usize),
    /// Slot script externe (ordre document).
    Script(usize),
    /// `<img>` — clé `Url::as_str()` : la table images ne
    /// prend des `NodeId` qu'après la resynchro DOM.
    Img(Url),
}

/// Graphe de modules préfetché (étape 50) — `sources` :
/// `url → corps` ; `alias` : spécificateur brut ou nom
/// d'entrée (`inline:N`) → URL de résolution des
/// dépendances (la page pour les entrées inline).
struct ModuleGraph {
    sources: HashMap<String, String>,
    alias: HashMap<String, String>,
}

/// Complétion d'une requête script-initiée (étape 51) —
/// produite par la tâche tokio spawned, consommée par
/// [`Session::pump_net`] aux points de synchro.
struct NetDone {
    /// Onglet émetteur.
    tab: TabId,
    /// Corrélation côté moteur (`NetResponse::handle`).
    handle: u64,
    /// Requête cross-origin — la lisibilité de la réponse
    /// est conditionnée à `Access-Control-Allow-Origin`.
    cross: bool,
    /// Origine canonique de la page émettrice (check ACAO).
    page_origin: String,
    /// URL de la requête — capture `Set-Cookie` (same-origin
    /// uniquement, `credentials` cross-origin hors MVP).
    req_url: Url,
    /// Sortie brute du transport.
    out: NetResult<Response>,
}

/// Verbe d'un `NetRequest` → `Method` — un verbe hors
/// sous-ensemble usuel n'est pas émis (dégradation status 0).
fn parse_method(s: &str) -> Option<Method> {
    Some(match s.trim().to_ascii_uppercase().as_str() {
        "GET" => Method::Get,
        "HEAD" => Method::Head,
        "POST" => Method::Post,
        "PUT" => Method::Put,
        "DELETE" => Method::Delete,
        "OPTIONS" => Method::Options,
        _ => return None,
    })
}

/// Verbe « safelisté » CORS — une requête cross-origin avec
/// un autre verbe exigerait un preflight `OPTIONS` (hors MVP).
const fn cors_simple_method(m: Method) -> bool {
    matches!(m, Method::Get | Method::Head | Method::Post)
}

/// En-tête safelisté CORS — une requête cross-origin portant
/// un autre en-tête exigerait un preflight : bloquée au MVP.
fn cors_safelisted_header(name: &str) -> bool {
    matches!(
        name.trim().to_ascii_lowercase().as_str(),
        "accept" | "accept-language" | "content-language" | "content-type" | "range"
    )
}

/// En-têtes interdits au script (Fetch « forbidden header
/// names », sous-ensemble) — le navigateur garde le contrôle
/// de `Referer`/`Origin`/`Cookie`/`Host`…, jamais posés par
/// la page (filtrés silencieusement à l'émission).
fn forbidden_request_header(name: &str) -> bool {
    let n = name.trim().to_ascii_lowercase();
    n.starts_with("sec-")
        || n.starts_with("proxy-")
        || matches!(
            n.as_str(),
            "accept-charset"
                | "accept-encoding"
                | "access-control-request-headers"
                | "access-control-request-method"
                | "connection"
                | "content-length"
                | "cookie"
                | "date"
                | "expect"
                | "host"
                | "keep-alive"
                | "origin"
                | "referer"
                | "te"
                | "trailer"
                | "transfer-encoding"
                | "upgrade"
                | "via"
        )
}

/// Réponse cross-origin lisible par le script ? CORS minimal :
/// `Access-Control-Allow-Origin: *` ou égal à l'origine
/// canonique de la page — sinon la réponse est opaque
/// (`status: 0` → événement `error`, comme un échec réseau).
fn cors_readable(page_origin: &str, resp: &Response) -> bool {
    resp.headers_all("access-control-allow-origin").any(|v| {
        let v = v.trim();
        v == "*" || v == page_origin
    })
}

/// `Origin` canonique d'une page — `scheme://host[:port]`,
/// port omis s'il est le défaut du schéma ; origine opaque
/// sérialisée `null` (spec Fetch, valeur `Origin` en-tête).
fn origin_header_value(u: &Url) -> String {
    let default = match u.scheme() {
        "http" => Some(80),
        "https" => Some(443),
        _ => None,
    };
    match u.effective_port() {
        Some(p) if Some(p) != default => {
            format!("{}://{}:{}", u.scheme(), u.host(), p)
        }
        Some(_) => format!("{}://{}", u.scheme(), u.host()),
        None => "null".to_string(),
    }
}

/// `document.cookie` ne peut pas poser `HttpOnly` — l'attribut
/// est ignoré (spec : le cookie est posé, visible côté script).
fn strip_http_only(raw: &str) -> String {
    raw.split(';')
        .enumerate()
        .filter(|(i, attr)| *i == 0 || !attr.trim().eq_ignore_ascii_case("httponly"))
        .map(|(_, s)| s.trim())
        .collect::<Vec<_>>()
        .join("; ")
}

/// File de jobs de sous-ressources, émise par priorité
/// `CSS > script > img` — le scheduler `fetch_wave` les tire
/// dans cet ordre (étape 50 ; le document est déjà servi).
fn sub_jobs(
    base: &Url,
    sheet_urls: &[Url],
    scripts: &[ScriptItem],
    img_urls: &[Url],
) -> VecDeque<(SubTag, Request)> {
    let mut jobs = VecDeque::new();
    for (i, u) in sheet_urls.iter().enumerate() {
        jobs.push_back((SubTag::Css(i), sub_request(base, u)));
    }
    for item in scripts {
        if let (Some(u), Some(slot)) = (&item.src, item.slot) {
            jobs.push_back((SubTag::Script(slot), sub_request(base, u)));
        }
    }
    for u in img_urls {
        jobs.push_back((SubTag::Img(u.clone()), sub_request(base, u)));
    }
    jobs
}

/// Requête GET de sous-ressource : `Referer`/`Origin` calculés
/// à l'émission (`initiator_headers`, strict-origin-when-
/// cross-origin) — le mixed-content est filtré en amont.
fn sub_request(base: &Url, url: &Url) -> Request {
    let mut req = Request::get(url.clone());
    for (n, v) in initiator_headers(base, url, Method::Get) {
        req.headers.push((n, v));
    }
    req
}

/// Vague de fetches parallèle bornée (étape 50, scheduler
/// ADR-0012) : `jobs` émis dans l'ordre fourni (priorité =
/// position), `max_inflight` requêtes en vol via
/// `FuturesUnordered` ; `on_done(tag, résultat)` est appelé
/// dans l'ordre d'arrivée, en ligne, entre deux émissions.
async fn fetch_wave<T>(
    transport: &Arc<dyn HttpTransport>,
    max_inflight: usize,
    mut jobs: VecDeque<(T, Request)>,
    mut on_done: impl FnMut(T, NetResult<Response>),
) {
    let cap = max_inflight.max(1);
    let mut inflight = FuturesUnordered::new();
    loop {
        while inflight.len() < cap {
            let Some((tag, req)) = jobs.pop_front() else {
                break;
            };
            let tr = Arc::clone(transport);
            inflight.push(async move {
                let res = tr.fetch(&req).await;
                (tag, res)
            });
        }
        if inflight.is_empty() {
            return;
        }
        let Some((tag, res)) = inflight.next().await else {
            return;
        };
        on_done(tag, res);
    }
}

/// Spécificateurs statiques `(spec brut, url résolue)` d'un
/// corps de module — `module_specifiers` (dillo-js, JS-C5)
/// + résolution contre l'URL du module parent.
fn module_deps(src: &str, referrer: &Url) -> Vec<(String, Url)> {
    explorer_js::module_specifiers(src)
        .into_iter()
        .filter_map(|spec| Url::resolve(referrer, &spec).ok().map(|u| (spec, u)))
        .collect()
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
/// Inverse d'[`element_path`] : nœud désigné par `path`
/// dans `dom` courant — utilisé pour re-résoudre les cibles
/// stockées après un re-parse de synchro (les `NodeId`
/// meurent, les chemins survivent).
fn dom_path_node(dom: &Dom, path: &[(String, u32)]) -> Option<NodeId> {
    let mut cur = dom.document();
    for (tag, k) in path {
        let mut seen = 0u32;
        let mut next = None;
        for c in dom.children(cur).ok()? {
            if dom.tag_name(c).is_ok_and(|t| t == *tag) {
                if seen == *k {
                    next = Some(c);
                    break;
                }
                seen += 1;
            }
        }
        cur = next?;
    }
    Some(cur)
}

fn find_by_id(dom: &Dom, id: &str) -> Option<NodeId> {
    let mut queue = vec![dom.document()];
    while let Some(n) = queue.pop() {
        if dom.element(n).is_ok() && dom.attribute(n, "id").ok().flatten().as_deref() == Some(id) {
            return Some(n);
        }
        if let Ok(ch) = dom.children(n) {
            queue.extend(ch.iter().rev());
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

/// Fenêtre de sniffing `<meta charset>` — même borne que
/// `explorer-html` (`META_SCAN_LEN`, privée côté domaine) : la
/// spec WHATWG limite la recherche au premier kilo-octet.
const SNIFF_WINDOW: usize = 1024;

/// Décodeur de document progressif (étape 53) : sniffing
/// WHATWG sur les `SNIFF_WINDOW` premiers octets (ou à EOF),
/// puis décodage incrémental — une séquence multi-octets
/// coupée par une frontière de morceau est conservée en queue
/// plutôt que perdue en `U+FFFD` (le décodage one-shot de
/// `decode_sniffed` est préservé à l'assemblage final).
struct StreamDecoder {
    /// Octets non encore décodés (pré-sniffing ou queue
    /// d'une séquence incomplète).
    buf: Vec<u8>,
    /// Encodage décidé — `None` tant que la fenêtre de
    /// sniffing n'est pas pleine (et pas EOF).
    enc: Option<Encoding>,
}

impl StreamDecoder {
    fn new() -> Self {
        Self {
            buf: Vec::new(),
            enc: None,
        }
    }

    /// Morceau de corps → texte décodé (potentiellement vide
    /// pendant la fenêtre de sniffing).
    fn feed(&mut self, chunk: &[u8]) -> String {
        self.buf.extend_from_slice(chunk);
        if self.enc.is_none() {
            if self.buf.len() < SNIFF_WINDOW {
                return String::new();
            }
            self.decide();
        }
        self.decode_tail(false)
    }

    /// Fin de corps — force la décision si la fenêtre n'était
    /// pas pleine, puis vide la queue.
    fn finish(&mut self) -> String {
        if self.enc.is_none() {
            self.decide();
        }
        self.decode_tail(true)
    }

    /// Sniffing + consommation du BOM (même règle que
    /// `decode_sniffed` : BOM réel seulement).
    fn decide(&mut self) {
        let enc = sniff_encoding(&self.buf);
        let bom = match enc {
            Encoding::Utf8 if self.buf.starts_with(&[0xEF, 0xBB, 0xBF]) => 3,
            Encoding::Utf16Le | Encoding::Utf16Be
                if self.buf.starts_with(&[0xFF, 0xFE]) || self.buf.starts_with(&[0xFE, 0xFF]) =>
            {
                2
            }
            _ => 0,
        };
        self.buf.drain(..bom);
        self.enc = Some(enc);
    }

    /// Décode le maximum décodable ; `last` = fin de flux (la
    /// queue incomplète est alors décodée en perte tolérante).
    fn decode_tail(&mut self, last: bool) -> String {
        let Some(enc) = self.enc else {
            return String::new();
        };
        let buf = std::mem::take(&mut self.buf);
        let (text, rest) = decode_prefix(&buf, enc, last);
        self.buf = rest;
        text
    }
}

/// Préfixe décodable de `buf` selon `enc` — renvoie le texte et
/// la queue d'octets non consommés (séquence incomplète en fin,
/// conservée pour le morceau suivant ; vide quand `last`).
fn decode_prefix(buf: &[u8], enc: Encoding, last: bool) -> (String, Vec<u8>) {
    match enc {
        // Un octet = un codepoint — tout est décodable.
        Encoding::Windows1252 => (decode_windows1252(buf), Vec::new()),
        // Multi-octets : seule la queue d'une séquence coupée en
        // frontière de morceau est conservée.
        Encoding::Utf8 => {
            // Fin de flux : décodage lossy complet — identique au
            // chemin one-shot `decode` (U+FFFD par séquence
            // invalide).
            if last {
                return (String::from_utf8_lossy(buf).into_owned(), Vec::new());
            }
            let (text, rest) = match std::str::from_utf8(buf) {
                Ok(_) => (String::from_utf8_lossy(buf).into_owned(), Vec::new()),
                Err(e) => {
                    let valid = e.valid_up_to();
                    let mut text = String::from_utf8_lossy(&buf[..valid]).into_owned();
                    match e.error_len() {
                        // Séquence valide tronquée en queue —
                        // conservée pour le morceau suivant.
                        None => (text, buf[valid..].to_vec()),
                        // Séquence invalide : `U+FFFD` une fois, le
                        // suffixe repart en queue (une séquence
                        // valide coupée y survivra).
                        Some(bad) => {
                            text.push('\u{FFFD}');
                            (text, buf[valid + bad..].to_vec())
                        }
                    }
                }
            };
            (text, rest)
        }
        Encoding::Utf16Le | Encoding::Utf16Be => {
            // Par paires d'octets ; une paire de surrogates ne
            // doit pas être coupée en frontière de morceau.
            let mut n = buf.len() & !1;
            if !last && n >= 2 {
                let unit = match enc {
                    Encoding::Utf16Le => u16::from_le_bytes([buf[n - 2], buf[n - 1]]),
                    _ => u16::from_be_bytes([buf[n - 2], buf[n - 1]]),
                };
                if (0xD800..=0xDBFF).contains(&unit) {
                    n -= 2; // lead surrogate sans trail — attendu.
                }
            }
            let units: Vec<u16> = buf[..n]
                .chunks_exact(2)
                .map(|c| match enc {
                    Encoding::Utf16Le => u16::from_le_bytes([c[0], c[1]]),
                    _ => u16::from_be_bytes([c[0], c[1]]),
                })
                .collect();
            (String::from_utf16_lossy(&units), buf[n..].to_vec())
        }
    }
}

/// BOM d'une sous-ressource (étape 53b) — `(encodage, octets
/// à sauter)`. Contrairement au sniffer HTML, aucun `<meta>` :
/// un script ou une feuille n'est pas un document.
fn sniff_bom(bytes: &[u8]) -> (Encoding, usize) {
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        (Encoding::Utf8, 3)
    } else if bytes.starts_with(&[0xFE, 0xFF]) {
        (Encoding::Utf16Be, 2)
    } else if bytes.starts_with(&[0xFF, 0xFE]) {
        (Encoding::Utf16Le, 2)
    } else {
        (Encoding::Utf8, 0)
    }
}

/// Label de charset WHATWG → encodage supporté — mêmes
/// mappings que `explorer-html::encoding` (famille latin-1 →
/// `Windows1252`). `None` = label absent/inconnu : la
/// priorité retombe sur l'échelon suivant (BOM, UTF-8).
fn label_encoding(label: &str) -> Option<Encoding> {
    match label
        .trim()
        .trim_matches(['"', '\''])
        .to_ascii_lowercase()
        .as_str()
    {
        "utf-8" | "utf8" | "unicode-1-1-utf-8" => Some(Encoding::Utf8),
        "utf-16" | "utf-16le" | "utf-16l" => Some(Encoding::Utf16Le),
        "utf-16be" => Some(Encoding::Utf16Be),
        "windows-1252" | "iso-8859-1" | "latin1" | "latin-1" | "ascii" | "us-ascii"
        | "iso8859-1" | "iso_8859-1" | "cp1252" => Some(Encoding::Windows1252),
        _ => None,
    }
}

/// Corps de sous-ressource texte (étape 53b) : `charset`
/// explicite → BOM → **UTF-8** — jamais le sniffer HTML ni
/// son repli `windows-1252`, qui corrompt les littéraux UTF-8
/// des scripts réels (mojibake « Lâ€™usage » sur
/// example.com). `charset` = attribut `<script charset>`
/// (classiques uniquement — les modules sont toujours UTF-8)
/// ou label de transport/déclaration CSS. Les octets de BOM
/// sont toujours sautés — ils n'appartiennent pas au texte.
fn decode_subresource(bytes: &[u8], charset: Option<&str>) -> String {
    let (bom_enc, skip) = sniff_bom(bytes);
    let enc = charset.and_then(label_encoding).unwrap_or(bom_enc);
    decode(&bytes[skip..], enc)
}

/// `charset=` du `Content-Type` transport (priorité maximale
/// pour les feuilles, WHATWG §4.4 — étape 53b).
fn transport_charset(headers: &[(String, String)]) -> Option<String> {
    let v = headers
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case("content-type"))?
        .1
        .as_str();
    let pos = v.to_ascii_lowercase().find("charset=")? + 8;
    let raw = v[pos..]
        .trim_start_matches(['"', '\''])
        .split([';', '"', '\'', ' '])
        .next()?
        .trim();
    (!raw.is_empty()).then(|| raw.to_string())
}

/// `@charset "…"` en tête de feuille (étape 53b) — la règle
/// ne compte que tout au début (après BOM éventuel) ; ASCII
/// tolérant sur la casse et les espaces.
fn css_charset(bytes: &[u8]) -> Option<String> {
    let (_, skip) = sniff_bom(bytes);
    let head = &bytes[skip..bytes.len().min(skip + 128)];
    let lower: Vec<u8> = head.iter().map(u8::to_ascii_lowercase).collect();
    let mut i = 0;
    while i < lower.len() && lower[i].is_ascii_whitespace() {
        i += 1;
    }
    let rest = lower.get(i..)?;
    rest.strip_prefix(b"@charset")?;
    let mut j = i + "@charset".len();
    while j < lower.len() && lower[j].is_ascii_whitespace() {
        j += 1;
    }
    let quote = *lower.get(j)?;
    if quote != b'"' && quote != b'\'' {
        return None;
    }
    let start = j + 1;
    let end = lower[start..]
        .iter()
        .position(|&b| b == quote)
        .map(|p| start + p)?;
    let label = String::from_utf8_lossy(&head[start..end]).trim().to_owned();
    (!label.is_empty()).then_some(label)
}

/// Corps de feuille liée (étape 53b) : `charset` transport →
/// BOM → `@charset` → UTF-8 (ordre WHATWG §4.4 — le document
/// référent n'entre pas en jeu : nos feuilles externes sont
/// décodées sur leur propre étiquette).
fn decode_stylesheet(body: &[u8], headers: &[(String, String)]) -> String {
    if let Some(c) = transport_charset(headers) {
        return decode_subresource(body, Some(&c));
    }
    let (enc, skip) = sniff_bom(body);
    if skip > 0 {
        return decode(&body[skip..], enc);
    }
    decode_subresource(body, css_charset(body).as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// UTF-8 par défaut — la régression 53b : « ’ » (E2 80 99)
    /// reste « ’ », pas « â€™ » (repli 1252 du sniffer HTML).
    #[test]
    fn subresource_utf8_default() {
        assert_eq!(
            decode_subresource("L\u{2019}usage".as_bytes(), None),
            "L\u{2019}usage"
        );
    }

    /// `charset` déclaré → label appliqué : 0xE9 = « é ».
    #[test]
    fn subresource_charset_label() {
        assert_eq!(
            decode_subresource(b"caf\xE9", Some("iso-8859-1")),
            "caf\u{E9}"
        );
    }

    /// Label inconnu → échelon suivant (UTF-8) — 0xE9 nu
    /// devient U+FFFD, pas « é » (le label ne gagne pas par
    /// défaut).
    #[test]
    fn subresource_unknown_label_falls_through() {
        assert_eq!(
            decode_subresource(b"caf\xE9", Some("x-bogus")),
            "caf\u{FFFD}"
        );
    }

    /// BOM UTF-8 + charset Latin-1 : le label déclaré décide
    /// de l'encodage, le BOM est sauté du texte.
    #[test]
    fn subresource_bom_stripped() {
        assert_eq!(
            decode_subresource(b"\xEF\xBB\xBFcaf\xE9", Some("iso-8859-1")),
            "caf\u{E9}"
        );
    }

    /// `Content-Type: …; charset=` l'emporte sur tout.
    #[test]
    fn stylesheet_transport_charset() {
        let h = [(
            "content-type".to_string(),
            "text/css; charset=ISO-8859-1".to_string(),
        )];
        assert!(decode_stylesheet(b".x{content:'caf\xE9'}", &h).contains("caf\u{E9}"));
    }

    /// `@charset` en tête, sans transport → label appliqué.
    #[test]
    fn stylesheet_at_charset() {
        assert!(
            decode_stylesheet(b"@charset \"iso-8859-1\";.x{content:'caf\xE9'}", &[])
                .contains("caf\u{E9}")
        );
    }

    /// Rien déclaré → UTF-8 (jamais 1252).
    #[test]
    fn stylesheet_utf8_default() {
        assert_eq!(
            decode_stylesheet(".x{content:'café'}".as_bytes(), &[]),
            ".x{content:'café'}"
        );
    }

    /// `@charset` pas en tête → ignoré (spec CSS : la règle
    /// doit être absolument première).
    #[test]
    fn stylesheet_at_charset_late_ignored() {
        let body = b".a{color:red}@charset 'iso-8859-1';.x{content:'caf\xE9'}".as_slice();
        assert_eq!(css_charset(body), None);
    }
}
