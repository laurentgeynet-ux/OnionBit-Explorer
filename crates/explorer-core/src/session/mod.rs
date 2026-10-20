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
//!
//! Découpage (étape 60) : `Session`/`Tab` gardent l'état et
//! les champs privés ici ; les implémentations vivent dans
//! des sous-modules par responsabilité (`navigate`, `net`,
//! `js_glue`, `input`, `paint`, `frame`, `store`, `anim`)
//! et les types/helpers dans `types`, `tab`, `popup`,
//! `anim_items`, `net_items`, `nav_types`, `dom_helpers`,
//! `paint_helpers`, `scripts`, `forms`, `decode`, `shared`.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::Instant;

use explorer_css::ComputedStyle;
use explorer_dom::Dom;
use explorer_js::{JsEngine, JsFactory, NetResponse};
use explorer_layout::font::FontSource;
use explorer_layout::{LayoutBox, Rect};
use explorer_net::cookies::CookieJar;
use explorer_net::http::{HttpTransport, Request};
use explorer_net::url::Url;
use explorer_render::{DisplayList, Frame, PaintImages, WebFontSource};
use tokio::sync::broadcast;
use tokio::task::JoinHandle;

#[allow(clippy::wildcard_imports)]
mod anim;
#[allow(clippy::wildcard_imports)]
mod anim_items;
#[allow(clippy::wildcard_imports)]
mod apply;
#[allow(clippy::wildcard_imports)]
mod decode;
#[allow(clippy::wildcard_imports)]
mod dom_helpers;

mod downloads;
#[allow(clippy::wildcard_imports)]
mod find;
#[allow(clippy::wildcard_imports)]
mod forms;
#[allow(clippy::wildcard_imports)]
mod frame;
#[allow(clippy::wildcard_imports)]
mod input;
#[allow(clippy::wildcard_imports)]
mod js_glue;
#[allow(clippy::wildcard_imports)]
mod nav_types;
#[allow(clippy::wildcard_imports)]
mod navigate;
#[allow(clippy::wildcard_imports)]
mod net;
#[allow(clippy::wildcard_imports)]
mod net_items;
#[allow(clippy::wildcard_imports)]
mod paint;
#[allow(clippy::wildcard_imports)]
mod paint_helpers;
#[allow(clippy::wildcard_imports)]
mod popup;
#[allow(clippy::wildcard_imports)]
mod scripts;
#[allow(clippy::wildcard_imports)]
mod shared;
#[allow(clippy::wildcard_imports)]
mod store;
#[allow(clippy::wildcard_imports)]
mod tab;
#[allow(clippy::wildcard_imports)]
mod types;
#[allow(clippy::wildcard_imports)]
pub(crate) use anim_items::*;
#[allow(clippy::wildcard_imports)]
pub(crate) use decode::*;
#[allow(clippy::wildcard_imports)]
pub(crate) use dom_helpers::*;

pub(crate) use downloads::*;
#[allow(clippy::wildcard_imports)]
pub(crate) use find::*;
#[allow(clippy::wildcard_imports)]
pub(crate) use forms::*;
#[allow(clippy::wildcard_imports)]
pub(crate) use nav_types::*;
#[allow(clippy::wildcard_imports)]
pub(crate) use net_items::*;
#[allow(clippy::wildcard_imports)]
pub(crate) use paint_helpers::*;
#[allow(clippy::wildcard_imports)]
pub(crate) use popup::*;
#[allow(clippy::wildcard_imports)]
pub(crate) use scripts::*;
#[allow(clippy::wildcard_imports)]
pub use shared::*;
#[allow(clippy::wildcard_imports)]
pub use types::*;

/// Entrée d'historique d'onglet (étape 76) : la requête
/// d'origine (rejouée à la traversée hors bfcache) + le
/// marqueur « échec » — une navigation en échec pousse son
/// entrée comme chez Chrome/Firefox : `back` revient à la
/// page précédente, `forward`/`go_back` **retente** la
/// cible au lieu de servir une page d'erreur gelée.
#[derive(Clone)]
pub(crate) struct HistEntry {
    /// Requête d'origine (méthode + corps — POST rejoué).
    pub(crate) req: Request,
    /// `true` = navigation en échec (page d'erreur commise).
    /// Aucune restauration bfcache — la traversée retente.
    pub(crate) failed: bool,
}

/// État d'un onglet : pipeline complet gardé — le re-render
/// (`resize`) repart du `dom` sans re-fetch.
pub(crate) struct Tab {
    /// URL courante.
    url: Option<Url>,
    /// Titre de la page (`<title>`).
    title: Option<String>,
    /// Historique local : requêtes visitées + index courant
    /// (`hist[..=hist_idx]`, le futur est tronqué à la
    /// navigation — modèle navigateur classique). La
    /// **requête** complète est stockée (étape 56) : un POST
    /// de formulaire pousse son entrée comme tout autre —
    /// pas de dédup d'URL — et back/forward la rejouent
    /// telle quelle si le bfcache l'a évincée. Une entrée
    /// `failed` (étape 76) = navigation en échec commise en
    /// page d'erreur : la traversée la **retente** (jamais de
    /// restauration bfcache de la page d'erreur).
    hist: Vec<HistEntry>,
    /// Requête qui a produit la page courante — `reload`
    /// réémet la vraie requête (un POST re-POSTe, étape 56).
    last_req: Option<Request>,
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
    /// PNG encodé — **cache paresseux** invalidé à chaque
    /// repaint, rempli par [`Session::frame_png`] (étape
    /// 55g : l'encodage n'est plus payé qu'une fois par
    /// frame servie).
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
    /// Popup `<select>` ouvert (étape 54) — overlay moteur
    /// peint en fin de display list ; refermé au clic,
    /// scroll, frappe ou navigation.
    popup: Option<SelectPopup>,
    /// Recherche dans la page (étape 57bis) — occurrences
    /// géométrisées, peintes en overlay translucide.
    find: Option<FindState>,
    /// Contexte JS du document — créé si la page a des
    /// `<script>` et qu'une fabrique est injectée ; meurt à
    /// la navigation (un contexte par document).
    js: Option<Box<dyn JsEngine>>,
    /// Bitmaps de peinture indexés par `NodeId` — `<img>`
    /// décodés (`replaced`) + `<svg>` inline rasterisés
    /// (`svgs`, étape 55b). Vidée à chaque re-parse (les ids
    /// changent), les `<img>` hors map retombent sur leur
    /// texte `alt`.
    images: PaintImages,
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
    /// Animations `@keyframes` enregistrées (`elem` ordinal
    /// et index de la liste `animation` → déclaration et
    /// `start`) — étape 55e, même borne.
    animations: Vec<ActiveAnimation>,
    /// Horloge de navigation (étape 53d) — incrémentée à
    /// chaque `nav_begin` ; les phases éclatées vérifient
    /// l'epoch avant chaque écriture, ce qui invalide
    /// automatiquement une navigation abandonnée au profit
    /// d'une plus récente (double-clic, script redirect).
    nav_epoch: u64,
    /// Intentions de navigation en attente (étape 53d) —
    /// input (`Req`) et script (`collect_script_navs` y
    /// déverse `location`/`history`/`form.submit()`) :
    /// `handle_input` ne télécharge plus sous `&mut self`,
    /// il enfile ; l'hôte conduit via `drain_script_nav`
    /// (compat) ou `drain_navs_shared` (session partagée).
    /// Bornée — un flot d'input n'accumule pas.
    pending_nav: std::collections::VecDeque<DrainAct>,
    /// Régions invalides accumulées (étape 55) — px CSS de
    /// document, scroll cuit (contrat `changed_rects`).
    /// Alimentée par `repaint` (diff avec la display list
    /// précédente) ; `dirty[emitted..]` est le delta rapporté
    /// par chaque `FrameInvalidated` (étape 55d — le client
    /// SSE reçoit ce qui a changé depuis l'événement
    /// précédent, pas l'historique à vie), `take_dirty` vide
    /// tout pour le canal pull FFI et ré-arme `emitted` ;
    /// plafonnée — au-delà elle s'effondre en « toute la
    /// frame ».
    dirty: Vec<Rect>,
    /// Tête déjà rapportée par événement dans `dirty`
    /// (étape 55d) — watermark, pas drain : les deux
    /// consommateurs (bus d'événements SSE et pull FFI
    /// `take_dirty`) sont indépendants, l'émission ne doit
    /// pas affamer le pull.
    emitted: usize,
    /// Pages gelées (étape 56, bfcache minimal) — indexées
    /// par position `hist_idx` : un `history_move` vers une
    /// position gelée restaure l'état complet sans réseau.
    /// Bornée par `SessionConfig::bfcache_pages` (FIFO).
    bfcache: VecDeque<PageSnapshot>,
    /// La frame/arbre courants proviennent d'un document
    /// **en vol** (peinture progressive 53d), pas du DOM
    /// commis (étape 56c) — à réverter au gel, à l'abandon
    /// ou à l'échec de la navigation, sinon l'état hybride
    /// (frame neuve + DOM/URL anciens) persiste ou est
    /// capturé dans un snapshot bfcache.
    spec_paint: bool,
    /// Epoch de navigation à laquelle le moteur JS détaché
    /// appartient (étape 53f) — `Some` = `js` est sorti pour
    /// un `run_pending`/`sync_dom` **hors verrou**. À la
    /// réinsertion, une epoch changée jette le moteur (son
    /// miroir parle de l'ancienne page).
    js_detached: Option<u64>,
    /// État de peinture détaché pour un apply hors verrou
    /// (étape 55h) — les champs `dom`/`tree`/`list`/`frame`
    /// sont vides le temps du `spawn_blocking` : la session
    /// répond comme pour une page jamais peinte (204 heal,
    /// hit-test inerte). Une navigation supplantant l'epoch
    /// jette la payload à la réinsertion.
    paint_detached: bool,
    /// Réponses script-net arrivées pendant le détachement
    /// JS (53f) — livrées au moteur à sa réinsertion, leurs
    /// callbacks tournent à la pompe suivante.
    js_pending_net: Vec<NetResponse>,
    /// Pushes d'attributs session→miroir survenus pendant le
    /// détachement JS (53f — `value`/`checked` des champs
    /// édités pendant le burst) : rejoués puis re-sérialisés
    /// à la réinsertion, sinon la synchro miroir→hôte
    /// écraserait la saisie.
    js_pending_attrs: Vec<PendingMirrorAttr>,
    /// Coalescence des resyncs DOM déclenchés par la pompe
    /// (55g-suite) : instant avant lequel le prochain
    /// `sync_dom` piloté par timer est **sauté** — le miroir
    /// reste sale et le prochain apply autorisé absorbe
    /// toutes les mutations accumulées en une passe. Posé
    /// après chaque `apply_dom_sync` proportionnellement à
    /// son coût : une page qui mute en continu (`setInterval`)
    /// ne tient plus le verrou en permanence — l'API respire
    /// entre deux peintures. Les synchros d'événements
    /// restent immédiates (clic = retour visuel attendu).
    sync_floor: Option<Instant>,
}

/// Push d'attribut session→miroir tamponné pendant le
/// détachement JS (53f) : `(chemin d'élément, nom, valeur)`.
type PendingMirrorAttr = (Vec<(String, u32)>, String, Option<String>);

/// Page gelée pour le bfcache (étape 56) — tout l'état qui
/// reconstruit la page **sans réseau** : DOM, arbre posé,
/// display list, frame/PNG, scroll, focus/hover, popup,
/// moteur JS figé (ses timers n'avancent pas tant qu'elle
/// est gelée), images décodées et styles calculés. Les
/// transitions CSS en vol sont abandonnées au gel
/// (reprendre au point exact serait faux — mieux rien que
/// un saut).
pub(crate) struct PageSnapshot {
    /// Position dans `Tab::hist` au moment du gel.
    hist_idx: usize,
    /// URL de la page (re-émise dans `Navigated`).
    url: Option<Url>,
    /// Titre courant.
    title: Option<String>,
    /// Décalage de scroll (restauré comme un vrai bfcache).
    scroll_y: f32,
    /// Hauteur du document.
    doc_h: f32,
    /// DOM parsé.
    dom: Option<Dom>,
    /// Arbre de boîtes posé.
    tree: Option<LayoutBox>,
    /// Display list (diff des dirty-rects au retour).
    list: Option<DisplayList>,
    /// Frame rasterisée (servie dès la restauration).
    frame: Option<Frame>,
    /// PNG encodé — cache paresseux du live au moment du
    /// gel ; `None` si jamais servi (étape 55g).
    png: Option<Vec<u8>>,
    /// Champ focalisé (chemin `(tag, index)`).
    focused: Option<Vec<(String, u32)>>,
    /// Nœud survolé (chemin).
    hovered: Option<Vec<(String, u32)>>,
    /// Popup `<select>` ouvert (retrouvé tel quel).
    popup: Option<SelectPopup>,
    /// Recherche active — le boxtree restauré garde la même
    /// géométrie, les rectangles restent valides.
    find: Option<FindState>,
    /// Moteur JS figé — le contexte survit, reprend tel
    /// quel à la restauration (modèle bfcache).
    js: Option<Box<dyn JsEngine>>,
    /// Bitmaps de peinture (`<img>` + `<svg>` inline).
    images: PaintImages,
    /// Feuilles `<link>` concaténées.
    linked_css: String,
    /// Styles calculés (transitions — vidées au gel).
    prev_styles: HashMap<usize, ComputedStyle>,
    /// Régions invalides en vol (+ tête déjà émise).
    dirty: Vec<Rect>,
    /// Watermark `emitted` gelé avec `dirty`.
    emitted: usize,
    /// Requête d'origine de la page gelée — restaurée pour
    /// que `reload` réémette la bonne méthode (étape 56).
    last_req: Option<Request>,
}

/// Session : transport + fontes injectés, onglets, viewport,
/// bus d'événements.
pub struct Session {
    transport: Arc<dyn HttpTransport>,
    /// Registre de fontes : faces `@font-face` fetchées
    /// devant la source locale injectée (ADR-0016). Purge
    /// par navigation (faces par document). Sous `Mutex`
    /// depuis l'étape 55h : un apply détaché emporte un
    /// handle `SharedFonts` pendant son `spawn_blocking`.
    webfonts: Arc<std::sync::Mutex<WebFontSource>>,
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
    /// Budget `@font-face` par document — recopié de
    /// `cfg.max_fonts` pour les phases détachées (53d, le
    /// registre `WebFontSource` borne aussi à l'inscription).
    max_fonts: usize,
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
    /// Téléchargements bufferisés en attente de décision
    /// (étape 70), `id → corps + méta`.
    downloads: HashMap<u64, PendingDownload>,
    /// Identifiants de téléchargement — un id est réservé
    /// par navigation (`nav_begin`) pour que la progression
    /// hors verrou porte déjà l'id final ; les ids non
    /// consécutifs sont volontaires (toute nav en consomme
    /// un, document ou pas).
    next_download_id: u64,
    /// Répertoire par défaut des téléchargements acceptés
    /// sans chemin explicite — `None` = chemin requis.
    download_dir: Option<std::path::PathBuf>,
    /// Borne du corps bufferisé d'un téléchargement
    /// (`SessionConfig::max_download_bytes`).
    max_download_bytes: usize,
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
    /// Plafond de la fenêtre de coalescence des resyncs
    /// pilotés par la pompe (`resync_gap_max_ms` de config,
    /// 55g-suite).
    resync_gap_max: std::time::Duration,
    /// Pages gelées par onglet dans le bfcache
    /// (`bfcache_pages` de config, étape 56).
    bfcache_pages: usize,
    /// Compteur de relayouts incrémentaux réussis (étape
    /// 55f) — diagnostic, lu par les tests de régression.
    incremental_syncs: u32,
    /// Applies de resynchro en file (étape 55h) — les points
    /// de synchro enfilent, les hôtes drainent (`drain_applies`
    /// synchrone ou `drain_applies_shared` hors verrou).
    /// Une entrée par onglet au plus (coalescence).
    pending_applies: apply::ApplyQueue,
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
            webfonts: Arc::new(std::sync::Mutex::new(WebFontSource::new(
                fonts,
                cfg.max_fonts,
            ))),
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
            max_fonts: cfg.max_fonts,
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
            resync_gap_max: std::time::Duration::from_millis(cfg.resync_gap_max_ms),
            bfcache_pages: cfg.bfcache_pages,
            incremental_syncs: 0,
            pending_applies: std::collections::VecDeque::new(),
            downloads: HashMap::new(),
            next_download_id: 1,
            download_dir: None,
            max_download_bytes: cfg.max_download_bytes,
        }
    }

    /// Nombre de synchros DOM résolues par relayout
    /// incrémental (`relayout_subtree`, étape 55f) — reste
    /// de la session. Diagnostic interne pour les tests de
    /// régression — pas un contrat d'API.
    #[doc(hidden)]
    #[must_use]
    pub fn incremental_syncs(&self) -> u32 {
        self.incremental_syncs
    }

    /// Branche la fabrique de moteurs JS (`JscoreEngine::factory()`
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
        // Profil persisté (étape 56) : le jar partagé repart
        // des cookies du disque — session cookies exclus
        // naturellement (ils n'ont jamais d'`expires_unix`).
        for sc in store.load_cookies() {
            self.cookies.insert_stored(stored_to_cookie(&sc));
        }
        self.store = Some(store);
    }

    /// Persiste le jar à cookies courant via le store
    /// (étape 56) — appelé aux points de synchro où le jar
    /// a pu muter : [`Self::after_script`], [`Self::poll_net`]
    /// et fin de [`Self::sub_commit`]. No-op sans store ; le
    /// store décide du rythme réel d'écriture (réécriture
    /// bornée `CookieConfig` côté `JsonFileStore`).
    pub(crate) fn persist_cookies(&self) {
        if let Some(s) = &self.store {
            // Cookies de session exclus : sans date
            // d'expiration ils meurent avec la session
            // (sémantique navigateur classique).
            let stored: Vec<StoredCookie> = self
                .cookies
                .export()
                .iter()
                .map(cookie_to_stored)
                .filter(|c| c.expires_unix.is_some())
                .collect();
            s.save_cookies(&stored);
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
                title: None,
                hist: Vec::new(),
                last_req: None,
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
                popup: None,
                find: None,
                js: None,
                images: PaintImages::default(),
                linked_css: String::new(),
                prev_styles: HashMap::new(),
                transitions: Vec::new(),
                animations: Vec::new(),
                nav_epoch: 0,
                pending_nav: std::collections::VecDeque::new(),
                dirty: Vec::new(),
                emitted: 0,
                bfcache: VecDeque::new(),
                spec_paint: false,
                js_detached: None,
                paint_detached: false,
                js_pending_net: Vec::new(),
                js_pending_attrs: Vec::new(),
                sync_floor: None,
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
            scroll_y: t.scroll_y,
            doc_h: t.doc_h,
            // Plafond raster en px CSS (étape 65) — la frame
            // ne peint rien au-delà, `doc_h` reste réel.
            doc_h_clamped: t.doc_h > MAX_FRAME_DIM / self.scale.max(f32::EPSILON),
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
}

#[cfg(test)]
mod tests;
