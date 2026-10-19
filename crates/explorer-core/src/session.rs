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
use explorer_css::value::Color;
use explorer_css::{
    initial_value, ComputedStyle, FontSrc, Origin, StyleEngine, StyleEnv, StyleMap,
};
use explorer_dom::{Dom, DomError, NodeId};
use explorer_html::encoding::{decode, decode_windows1252, sniff_encoding, Encoding};
use explorer_html::TreeBuilder;
use explorer_js::{JsEngine, JsFactory, NavRequest, NetRequest, NetResponse, UrlResolver};
use explorer_layout::font::FontSource;
use explorer_layout::{
    build_box_tree_sized, layout_tree, relayout_subtree, BlockCtx, LayoutBox, Rect, Relayout,
};
use explorer_net::cookies::{Cookie, CookieJar, SameSite};
use explorer_net::error::NetResult;
use explorer_net::http::{HttpTransport, Method, Request, Response};
use explorer_net::policy::{initiator_headers, is_mixed_content};
use explorer_net::url::Url;
use explorer_render::{
    build_display_list_full, collect_data_images, decode_font, decode_image, rasterize,
    BorderStyle, DecodedImage, DisplayItem, DisplayList, Frame, ImageTable, PaintImages,
    WebFontSource,
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
    /// `GET /api/tabs/{id}/frame` (ADR-0004). `rects` porte
    /// les régions invalides (étape 55, contrat R-7
    /// `DisplayList::changed_rects`) en **pixels physiques**
    /// du framebuffer ; vide = repeindre entièrement
    /// (conservateur — première peinture, listes
    /// incomparables).
    FrameInvalidated {
        /// Onglet dont la frame est périmée.
        tab: TabId,
        /// Régions invalides accumulées depuis la dernière
        /// émission — l'UI peut ne re-tirer que ces zones
        /// (`frame?x=&y=&w=&h=` côté REST,
        /// `explorer_frame_copy_region` côté FFI).
        rects: Vec<FrameRegion>,
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

/// Région invalide d'une frame (étape 55) — **pixels
/// physiques** du framebuffer (le `scale` DPI est déjà
/// appliqué côté core : les rects rapportés par
/// `DisplayList::changed_rects` vivent en px CSS de
/// document, le scroll y est cuit).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameRegion {
    /// Abscisse (px physiques).
    pub x: f32,
    /// Ordonnée (px physiques).
    pub y: f32,
    /// Largeur (px physiques).
    pub w: f32,
    /// Hauteur (px physiques).
    pub h: f32,
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

/// Cookie persisté (étape 56) — miroir sérialisable neutre
/// de `explorer_net::cookies::Cookie` : expiration en
/// secondes unix (`SystemTime` n'est pas sérialisable tel
/// quel côté store).
#[derive(Debug, Clone)]
pub struct StoredCookie {
    /// Nom.
    pub name: String,
    /// Valeur.
    pub value: String,
    /// Domaine de pose.
    pub domain: String,
    /// `true` = host-only.
    pub host_only: bool,
    /// Chemin.
    pub path: String,
    /// Expiration unix secs (`None` = cookie de session).
    pub expires_unix: Option<u64>,
    /// `Secure`.
    pub secure: bool,
    /// `HttpOnly`.
    pub http_only: bool,
    /// `SameSite` — `"strict"`, `"lax"` ou `"none"`
    /// (inconnu restauré en `lax`).
    pub same_site: String,
}

/// `Cookie` filaire → forme persistée (expiration → secs
/// unix ; les expirés ne sont jamais exportés —
/// [`CookieJar::export`] purge déjà).
fn cookie_to_stored(c: &Cookie) -> StoredCookie {
    StoredCookie {
        name: c.name.clone(),
        value: c.value.clone(),
        domain: c.domain.clone(),
        host_only: c.host_only,
        path: c.path.clone(),
        expires_unix: c.expires.and_then(|t| {
            t.duration_since(std::time::UNIX_EPOCH)
                .ok()
                .map(|d| d.as_secs())
        }),
        secure: c.secure,
        http_only: c.http_only,
        same_site: match c.same_site {
            SameSite::Strict => "strict",
            SameSite::Lax => "lax",
            SameSite::None => "none",
        }
        .to_string(),
    }
}

/// Forme persistée → `Cookie` filaire (`insert_stored` du
/// jar revérifie l'expiration).
fn stored_to_cookie(s: &StoredCookie) -> Cookie {
    Cookie {
        name: s.name.clone(),
        value: s.value.clone(),
        domain: s.domain.clone(),
        host_only: s.host_only,
        path: s.path.clone(),
        expires: s
            .expires_unix
            .map(|secs| std::time::UNIX_EPOCH + std::time::Duration::from_secs(secs)),
        secure: s.secure,
        http_only: s.http_only,
        same_site: match s.same_site.as_str() {
            "strict" => SameSite::Strict,
            "none" => SameSite::None,
            _ => SameSite::Lax,
        },
    }
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
    /// Charge les cookies persistés (étape 56 — défaut :
    /// jar vide au démarrage).
    fn load_cookies(&self) -> Vec<StoredCookie> {
        Vec::new()
    }
    /// Persiste le jar complet (étape 56 — liste bornée par
    /// `CookieConfig` ; défaut : ignorer, jar volatil).
    fn save_cookies(&self, _cookies: &[StoredCookie]) {}
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
    /// Historique local : requêtes visitées + index courant
    /// (`hist[..=hist_idx]`, le futur est tronqué à la
    /// navigation — modèle navigateur classique). La
    /// **requête** complète est stockée (étape 56) : un POST
    /// de formulaire pousse son entrée comme tout autre —
    /// pas de dédup d'URL — et back/forward la rejouent
    /// telle quelle si le bfcache l'a évincée.
    hist: Vec<Request>,
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
    /// Popup `<select>` ouvert (étape 54) — overlay moteur
    /// peint en fin de display list ; refermé au clic,
    /// scroll, frappe ou navigation.
    popup: Option<SelectPopup>,
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
}

/// Page gelée pour le bfcache (étape 56) — tout l'état qui
/// reconstruit la page **sans réseau** : DOM, arbre posé,
/// display list, frame/PNG, scroll, focus/hover, popup,
/// moteur JS figé (ses timers n'avancent pas tant qu'elle
/// est gelée), images décodées et styles calculés. Les
/// transitions CSS en vol sont abandonnées au gel
/// (reprendre au point exact serait faux — mieux rien que
/// un saut).
struct PageSnapshot {
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
    /// PNG encodé prêt à servir.
    png: Option<Vec<u8>>,
    /// Champ focalisé (chemin `(tag, index)`).
    focused: Option<Vec<(String, u32)>>,
    /// Nœud survolé (chemin).
    hovered: Option<Vec<(String, u32)>>,
    /// Popup `<select>` ouvert (retrouvé tel quel).
    popup: Option<SelectPopup>,
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

impl Tab {
    /// Gèle la page courante dans le bfcache à sa position
    /// `hist_idx` (étape 56) — appelé à chaque navigation
    /// confirmée ([`Session::nav_apply`]). Sans page réelle
    /// (onglet vierge) → no-op. Borne `cap` : FIFO.
    fn freeze(&mut self, cap: usize) {
        // Rien à geler tant que la page n'a pas de produit
        // peint — un DOM commis sans frame (fenêtre
        // `nav_apply` → `sub_commit`, étape 56b) produirait
        // un snapshot empoisonné : sa restauration servirait
        // un `/frame` 204 persistant avec hit-test mort.
        // Mieux vaut re-fetcher la position plus tard.
        if self.dom.is_none() || self.png.is_none() {
            return;
        }
        // Une page ne vit qu'une fois par position —
        // une nouvelle visite remplace l'ancienne gelée.
        self.bfcache.retain(|s| s.hist_idx != self.hist_idx);
        let snap = PageSnapshot {
            hist_idx: self.hist_idx,
            url: self.url.take(),
            title: self.title.take(),
            scroll_y: self.scroll_y,
            doc_h: self.doc_h,
            dom: self.dom.take(),
            tree: self.tree.take(),
            list: self.list.take(),
            frame: self.frame.take(),
            png: self.png.take(),
            focused: self.focused.take(),
            hovered: self.hovered.take(),
            popup: self.popup.take(),
            js: self.js.take(),
            images: std::mem::take(&mut self.images),
            linked_css: std::mem::take(&mut self.linked_css),
            prev_styles: std::mem::take(&mut self.prev_styles),
            dirty: std::mem::take(&mut self.dirty),
            emitted: self.emitted,
            last_req: self.last_req.take(),
        };
        self.scroll_y = 0.0;
        self.doc_h = 0.0;
        // `dirty` vient d'être gelée → le live repart à
        // vide, watermark ré-armé.
        self.emitted = 0;
        self.bfcache.push_back(snap);
        while self.bfcache.len() > cap {
            self.bfcache.pop_front();
        }
    }

    /// Retire le snapshot gelé à `idx` s'il existe (étape
    /// 56 — le hit bfcache d'un `history_move`).
    fn take_snapshot(&mut self, idx: usize) -> Option<PageSnapshot> {
        let pos = self.bfcache.iter().position(|s| s.hist_idx == idx)?;
        self.bfcache.remove(pos)
    }

    /// Restaure un snapshot : tout l'état de page revient
    /// tel quel — epoch incrémentée (invalide toute nav en
    /// vol) et intentions de navigation purgées (un script
    /// de la page gelée ne doit pas tirer des navs périmées).
    fn restore(&mut self, snap: PageSnapshot) {
        self.url = snap.url;
        self.title = snap.title;
        self.scroll_y = snap.scroll_y;
        self.doc_h = snap.doc_h;
        self.dom = snap.dom;
        self.tree = snap.tree;
        self.list = snap.list;
        self.frame = snap.frame;
        self.png = snap.png;
        self.focused = snap.focused;
        self.hovered = snap.hovered;
        self.popup = snap.popup;
        self.js = snap.js;
        self.images = snap.images;
        self.linked_css = snap.linked_css;
        self.prev_styles = snap.prev_styles;
        self.dirty = snap.dirty;
        self.emitted = snap.emitted;
        self.spec_paint = false; // snapshot auto-cohérent (56c)
        self.last_req = snap.last_req;
        self.transitions.clear();
        self.nav_epoch += 1;
        self.pending_nav.clear();
    }
}

/// Popup `<select>` moteur (étape 54) — l'état vit hors DOM :
/// les options sont figées à l'ouverture, le `<select>` est
/// re-résolu par chemin au moment du choix (le nœud peut
/// mourir au re-parse de synchro).
struct SelectPopup {
    /// Chemin `(tag, index)` du `<select>` dans `dom`.
    path: Vec<(String, u32)>,
    /// Boîte du contrôle (coordonnées document).
    rect: Rect,
    /// `(étiquette, sélectionnée)` par `<option>` descendant.
    options: Vec<(String, bool)>,
    /// Hauteur d'une ligne du popup (px).
    row_h: f32,
    /// Ligne survolée au dernier `mousemove` — surlignage
    /// hover, sans sélection tant que non cliquée.
    hovered: Option<usize>,
}

/// Lignes visibles du popup `<select>` — le reste des
/// options est tronqué (MVP, pas de défilement interne).
const MAX_POPUP_ROWS: usize = 8;

/// Plafond de régions invalides accumulées par onglet
/// (étape 55) — au-delà, l'ensemble s'effondre en « toute
/// la frame » : un client qui ne draine jamais retombe sur
/// le comportement plein-format, et une rafale de petites
/// invalidations ne fait pas grossir la charge utile SSE
/// sans borne.
const MAX_DIRTY_RECTS: usize = 32;

/// Dimension max d'une frame rasterisée — borne anti-hostile
/// de `rasterize` (`explorer-render`), déclarée ici pour
/// plafonner la display list avant l'appel : un document
/// plus grand que le cap produit une frame tronquée au
/// sommet plutôt qu'une erreur `FrameTooBig` sèche (la
/// peinture progressive doit émettre pendant le
/// téléchargement, même sur page géante — borne documentée,
/// stratégie fenêtre/tuiles consignée `interface-equipes.md`).
const MAX_FRAME_DIM: f32 = 16_384.0;

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
    /// Pages gelées par onglet dans le bfcache (étape 56) —
    /// un `history_move` vers une position gelée restaure
    /// l'état complet sans réseau (DOM, layout, frame, JS
    /// figé, scroll). `0` = bfcache désactivé. Mémoire :
    /// chaque page gelée garde DOM + layout + frame — la
    /// borne est volontairement petite.
    pub bfcache_pages: usize,
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
            bfcache_pages: 4,
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
    /// Pages gelées par onglet dans le bfcache
    /// (`bfcache_pages` de config, étape 56).
    bfcache_pages: usize,
    /// Compteur de relayouts incrémentaux réussis (étape
    /// 55f) — diagnostic, lu par les tests de régression.
    incremental_syncs: u32,
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
            bfcache_pages: cfg.bfcache_pages,
            incremental_syncs: 0,
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
    fn persist_cookies(&self) {
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
                js: None,
                images: PaintImages::default(),
                linked_css: String::new(),
                prev_styles: HashMap::new(),
                transitions: Vec::new(),
                nav_epoch: 0,
                pending_nav: std::collections::VecDeque::new(),
                dirty: Vec::new(),
                emitted: 0,
                bfcache: VecDeque::new(),
                spec_paint: false,
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
        self.emit_invalidated(tab);
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
            (eng.run_pending_bounded(max), eng.sync_dom_path())
        };
        if let Some((html, dirty)) = mutated {
            self.apply_dom_sync(tab, &html, dirty.as_deref(), true);
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

    /// Recharge la page courante (re-fetch complet,
    /// historique inchangé) — réémet la requête d'origine
    /// `last_req` : une page servie par POST est re-POSTée
    /// (étape 56 — les navigateurs réémettent la requête,
    /// quitte à confirmer la resoumission côté UI).
    ///
    /// # Errors
    /// [`CoreError::UnknownTab`], [`CoreError::EmptyDocument`]
    /// si l'onglet n'a jamais été navigué, ou erreur de fetch.
    pub async fn reload(&mut self, tab: TabId) -> Result<(), CoreError> {
        let Some(req) = self.tabs.get(&tab).and_then(|t| {
            t.last_req
                .clone()
                .or_else(|| t.url.clone().map(Request::get))
        }) else {
            return Err(if self.tabs.contains_key(&tab) {
                CoreError::EmptyDocument
            } else {
                CoreError::UnknownTab(tab)
            });
        };
        match self.navigate_request(tab, req, false).await {
            Ok(_) => {
                self.drain_script_nav(tab).await;
                Ok(())
            }
            Err(e) => Err(self.emit_error(tab, e)),
        }
    }

    /// Recule dans l'historique local de l'onglet — restaure
    /// le snapshot bfcache s'il existe (instantané, étape 56)
    /// sinon rejoue la requête d'origine (méthode + corps —
    /// un POST historié re-POSTe).
    ///
    /// # Errors
    /// [`CoreError::UnknownTab`] ou [`CoreError::EmptyDocument`]
    /// s'il n'y a pas de page précédente.
    pub async fn go_back(&mut self, tab: TabId) -> Result<(), CoreError> {
        self.history_move(tab, -1).await?;
        self.drain_script_nav(tab).await;
        Ok(())
    }

    /// Avance dans l'historique local de l'onglet — mêmes
    /// sémantiques que [`Self::go_back`] (bfcache puis rejeu
    /// de la requête d'origine).
    ///
    /// # Errors
    /// [`CoreError::UnknownTab`] ou [`CoreError::EmptyDocument`]
    /// s'il n'y a pas de page suivante.
    pub async fn go_forward(&mut self, tab: TabId) -> Result<(), CoreError> {
        self.history_move(tab, 1).await?;
        self.drain_script_nav(tab).await;
        Ok(())
    }

    /// Tentative de restauration bfcache (étape 56) : la
    /// position `idx` est gelée → tout l'état revient sans
    /// réseau ; la page quittée est gelée à son tour
    /// (l'aller-retour ne re-télécharge jamais). `Navigated`,
    /// `TitleChanged` et `FrameInvalidated` émis comme après
    /// une navigation classique.
    ///
    /// `false` = pas de snapshot → fetch normal.
    fn try_bfcache(&mut self, tab: TabId, idx: usize) -> bool {
        let Some(entry) = self.tabs.get_mut(&tab) else {
            return false;
        };
        let prev = entry.title.clone();
        let Some(snap) = entry.take_snapshot(idx) else {
            return false;
        };
        // La page quittée gèle son état affiché — une
        // peinture spéculative en vol ne doit pas être
        // embarquée (étape 56c).
        self.unspec(tab);
        let Some(entry) = self.tabs.get_mut(&tab) else {
            return false;
        };
        entry.freeze(self.bfcache_pages);
        entry.restore(snap);
        entry.hist_idx = idx;
        // Snapshot gelé avant sa première peinture (étape
        // 56b — fenêtre commit→repaint) ou produit absent :
        // le DOM restauré suffit à re-peindre plutôt que de
        // servir un `/frame` 204 persistant avec hit-test
        // mort et scroll bloqué à 0.
        let needs_paint = entry.dom.is_some() && entry.png.is_none();
        let url = entry.url.clone();
        let title = entry.title.clone();
        if needs_paint {
            let _ = self.relayout(tab);
        }
        if let Some(u) = url {
            // Mêmes événements qu'une navigation classique :
            // `Navigated` + `TitleChanged` si le titre
            // restauré diffère + `FrameInvalidated`.
            self.emit_navigated(tab, &u, title, prev.as_deref());
        }
        true
    }

    /// Navigation **même-document** vers un `#fragment`
    /// (étape 56d — `href="#id"`, footnotes Wikipedia) :
    /// l'URL hors fragment identique à la page commise →
    /// jamais de re-fetch — on pousse l'entrée d'historique,
    /// on émet `Navigated`, puis on scrolle vers la cible
    /// (`id` d'abord, `<a name>` ensuite, `#` nu → haut de
    /// page). URL différente → navigation complète ; le
    /// fragment est appliqué post-commit dans `sub_commit`.
    fn try_fragment_nav(&mut self, tab: TabId, req: &Request) -> bool {
        if req.method != Method::Get || req.body.is_some() {
            return false;
        }
        let Some(frag) = req.url.fragment() else {
            return false;
        };
        let same = self.tabs.get(&tab).is_some_and(|e| {
            e.url
                .as_ref()
                .is_some_and(|u| u.as_str().split('#').next() == req.url.as_str().split('#').next())
        });
        if !same {
            return false;
        }
        let prev = self.tabs.get(&tab).and_then(|e| e.title.clone());
        if let Some(e) = self.tabs.get_mut(&tab) {
            e.hist.truncate(e.hist_idx + 1);
            e.hist.push(req.clone());
            e.hist_idx = e.hist.len() - 1;
            e.url = Some(req.url.clone());
        }
        let title = self.tabs.get(&tab).and_then(|e| e.title.clone());
        self.emit_navigated(tab, &req.url, title, prev.as_deref());
        if frag.is_empty() {
            // `#` nu = haut de page.
            let _ = self.scroll_to_y(tab, 0.0);
        } else {
            self.scroll_to_fragment(tab, &frag.to_string());
        }
        true
    }

    /// Scrolle vers la cible d'un `#fragment` : `id` puis
    /// `<a name>` (ordre navigateur). Position = le haut de
    /// la première boîte posée portant le nœud — bornée
    /// comme `scroll_by`. No-op si la cible est introuvable
    /// ou non rendue (dégradation, jamais d'erreur).
    fn scroll_to_fragment(&mut self, tab: TabId, frag: &str) {
        let Some(entry) = self.tabs.get(&tab) else {
            return;
        };
        let (Some(dom), Some(tree)) = (entry.dom.as_ref(), entry.tree.as_ref()) else {
            return;
        };
        let target = find_by_id(dom, frag).or_else(|| find_named_anchor(dom, frag));
        let Some(y) = target.and_then(|n| box_y_of_node(tree, n)) else {
            return;
        };
        let _ = self.scroll_to_y(tab, y);
    }

    /// Scroll absolu borné `[0, doc_h − viewport]` + repaint
    /// + `FrameInvalidated` — variante absolue de
    /// `scroll_by` pour la navigation fragment (56d).
    fn scroll_to_y(&mut self, tab: TabId, y: f32) -> Result<(), CoreError> {
        {
            let entry = self.tabs.get_mut(&tab).ok_or(CoreError::UnknownTab(tab))?;
            let max = (entry.doc_h - self.viewport.1).max(0.0);
            let next = y.clamp(0.0, max);
            if (next - entry.scroll_y).abs() < f32::EPSILON {
                return Ok(());
            }
            entry.scroll_y = next;
        }
        self.repaint(tab)?;
        self.emit_invalidated(tab);
        Ok(())
    }

    /// `navigate` vers l'URL de l'entrée **forward** (étape
    /// 56c, repro A→B→back→`navigate B`) : restaurer le
    /// snapshot bfcache — un commit complet de l'entrée —
    /// plutôt qu'un fetch neuf dont la peinture progressive
    /// afficherait B pendant que l'entrée commise reste A
    /// (état hybride : frame B + hit-test/`status` sur A).
    /// Seul un GET sans corps vers l'URL exacte de
    /// l'entrée forward est réécrit en restauration ; un
    /// POST ou une URL différente navigue normalement, et
    /// l'absence de snapshot retombe sur le chemin classique.
    fn try_forward_nav(&mut self, tab: TabId, req: &Request) -> bool {
        if req.method != Method::Get || req.body.is_some() {
            return false;
        }
        let fwd = self
            .tabs
            .get(&tab)
            .map(|e| e.hist_idx + 1)
            .unwrap_or_default();
        let matches = self
            .tabs
            .get(&tab)
            .is_some_and(|e| e.hist.get(fwd).is_some_and(|r| r.url == req.url));
        matches && self.try_bfcache(tab, fwd)
    }

    /// Déplacement dans l'historique — `delta` = ±1.
    async fn history_move(&mut self, tab: TabId, delta: isize) -> Result<(), CoreError> {
        let entry = self.tabs.get_mut(&tab).ok_or(CoreError::UnknownTab(tab))?;
        let next = entry.hist_idx.checked_add_signed(delta);
        let Some(idx) = next.filter(|&i| i < entry.hist.len()) else {
            return Err(CoreError::EmptyDocument);
        };
        let req = entry.hist[idx].clone();
        // Hit bfcache (étape 56) : restauration instantanée,
        // zéro réseau.
        if self.try_bfcache(tab, idx) {
            return Ok(());
        }
        // Miss → on rejoue la requête d'origine (méthode +
        // corps — un POST historié re-POSTe, étape 56).
        // `Navigated` est émis par `navigate_request` — sauf
        // redirection script (alors `entry.url` diffère et la
        // cible a émis pour elle-même).
        match self.navigate_request(tab, req, false).await {
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
    ///
    /// Chemin compat `&mut self` (étape 53d) : les phases
    /// [`nav_begin`](Self::nav_begin) → [`NavJob::run`] →
    /// [`nav_apply`](Self::nav_apply) → [`SubJob::run`] →
    /// [`sub_commit`](Self::sub_commit) sont exactement celles
    /// que [`navigate_shared`] conduit sur une session
    /// partagée — ici `self` reste emprunté pendant tout le
    /// parcours (CLI, FFI, tests).
    async fn navigate_request(
        &mut self,
        tab: TabId,
        req: Request,
        push: bool,
    ) -> Result<Option<String>, CoreError> {
        // `navigate` vers l'entrée forward = restauration
        // bfcache, jamais une peinture non commise (56c).
        if push && self.try_forward_nav(tab, &req) {
            return Ok(self.tabs.get(&tab).and_then(|e| e.title.clone()));
        }
        // `#fragment` même document = scroll + historique,
        // jamais de re-fetch (56d).
        if self.try_fragment_nav(tab, &req) {
            return Ok(self.tabs.get(&tab).and_then(|e| e.title.clone()));
        }
        let mut job = self.nav_begin(tab, req, push)?;
        let epoch = job.epoch;
        let (done, tail) = match job
            .run(|dom: &Dom| -> PaintFut<'_> {
                self.nav_paint(tab, epoch, dom);
                Box::pin(async {})
            })
            .await
        {
            Ok(out) => out,
            Err(e) => {
                // Le document en vol a pu être peint
                // progressivement — on réverte avant de
                // propager (étape 56c).
                self.unspec(tab);
                return Err(e);
            }
        };
        let Some(mut sub) = self.nav_apply(tab, tail, done)? else {
            return Ok(None);
        };
        sub.run().await;
        self.sub_commit(tab, sub).await
    }

    /// Phase 1 de la navigation éclatée (étape 53d) :
    /// valide l'onglet, arme l'epoch (les phases suivantes
    /// refusent tout commit d'une navigation supplantée) et
    /// capture le titre courant pour `TitleChanged`. Aucune
    /// écriture visible — URL, historique et DOM ne bougent
    /// qu'au commit.
    fn nav_begin(&mut self, tab: TabId, req: Request, push: bool) -> Result<NavJob, CoreError> {
        let entry = self.tabs.get_mut(&tab).ok_or(CoreError::UnknownTab(tab))?;
        entry.nav_epoch += 1;
        Ok(NavJob {
            epoch: entry.nav_epoch,
            req,
            push,
            prev_title: entry.title.clone(),
            transport: Arc::clone(&self.transport),
            paint_chunk_bytes: self.paint_chunk_bytes,
            paint_interval: self.paint_interval,
        })
    }

    /// Émet `FrameInvalidated` avec le **delta** des régions
    /// invalides depuis l'émission précédente (étape 55d) —
    /// les rects internes sont en px CSS (`changed_rects`),
    /// le contrat sort en pixels physiques (`× scale`,
    /// comme `frame_pixels`). Watermark `emitted`, pas drain :
    /// le canal pull FFI (`take_dirty`) vide sa propre
    /// accumulation sans être affamé par les événements (le
    /// handle FFI garde un receveur inerte du bus).
    fn emit_invalidated(&mut self, tab: TabId) {
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
        let _ = self.events.send(CoreEvent::FrameInvalidated { tab, rects });
    }

    /// Peinture progressive epoch-gardée (étape 53d) : un
    /// snapshot émis par [`NavJob::run`] n'est posé que si
    /// la navigation est encore la plus récente de l'onglet —
    /// sinon il est silencieusement jeté.
    fn nav_paint(&mut self, tab: TabId, epoch: u64, dom: &Dom) {
        let live = self.tabs.get(&tab).is_some_and(|e| e.nav_epoch == epoch);
        if live && self.relayout_dom(tab, dom).is_ok() {
            // Peinture spéculative (étape 56c) : l'arbre et la
            // frame montrent un document non commis — marqué
            // pour réversion au gel/abandon/échec.
            if let Some(e) = self.tabs.get_mut(&tab) {
                e.spec_paint = true;
            }
            self.emit_invalidated(tab);
        }
    }

    /// Réverte une peinture spéculative (étape 56c) : la
    /// frame provenait d'un document en vol — repeint depuis
    /// le DOM commis pour que frame et DOM redeviennent
    /// cohérents. No-op si rien de spéculatif.
    fn unspec(&mut self, tab: TabId) {
        let spec = self.tabs.get(&tab).is_some_and(|e| e.spec_paint);
        if !spec {
            return;
        }
        if let Some(e) = self.tabs.get_mut(&tab) {
            e.spec_paint = false;
        }
        if self.relayout(tab).is_ok() {
            self.emit_invalidated(tab);
        } else if let Some(e) = self.tabs.get_mut(&tab) {
            // Rien de commis à repeindre (onglet jamais
            // navigué) : on jette l'arbre et la frame
            // spéculatifs plutôt que de laisser un état
            // sans DOM derrière.
            e.tree = None;
            e.list = None;
            e.frame = None;
            e.png = None;
        }
    }

    /// Phase 2 — commit du DOM téléchargé : écrit l'état de
    /// l'onglet, planifie les sous-ressources, arme le moteur
    /// JS et emballe le [`SubJob`] détachable. `None` = epoch
    /// supplantée (navigation abandonnée — rien à commettre).
    fn nav_apply(
        &mut self,
        tab: TabId,
        tail: NavTail,
        done: NavDone,
    ) -> Result<Option<SubJob>, CoreError> {
        {
            let entry = self.tabs.get(&tab).ok_or(CoreError::UnknownTab(tab))?;
            if entry.nav_epoch != tail.epoch {
                self.unspec(tab);
                return Ok(None);
            }
        }
        let NavDone {
            url,
            dom,
            html,
            title,
        } = done;
        if let Some(s) = &self.store {
            s.record_visit(&url.to_string(), title.as_deref());
        }
        let scripting = self.js_factory.is_some();
        let (sheet_urls, scripts, img_urls) = self.plan_subresources(&dom, &url, scripting);
        let doc_css = collect_style_css(&dom);
        // Le gel capture l'état affiché — une peinture
        // spéculative du document en vol ne doit pas être
        // embarquée dans le snapshot (étape 56c).
        self.unspec(tab);
        {
            let entry = self.tabs.get_mut(&tab).ok_or(CoreError::UnknownTab(tab))?;
            // bfcache (étape 56) : la page quittée est gelée
            // à sa position d'historique — un retour
            // arrière la restaurera sans réseau.
            entry.freeze(self.bfcache_pages);
            entry.last_req = Some(tail.req.clone());
            entry.url = Some(url.clone());
            entry.title.clone_from(&title);
            entry.scroll_y = 0.0;
            entry.focused = None;
            entry.hovered = None;
            entry.popup = None;
            entry.js = None;
            entry.images = PaintImages::default();
            entry.linked_css.clear();
            entry.prev_styles = HashMap::new();
            entry.transitions.clear();
            entry.dom = Some(dom);
        }
        self.webfonts.clear();
        let eng = self.spawn_engine(tab, &html, !scripts.is_empty());
        Ok(Some(SubJob {
            tab,
            epoch: tail.epoch,
            url: url.clone(),
            push: tail.push,
            prev_title: tail.prev_title,
            req: tail.req,
            title,
            transport: Arc::clone(&self.transport),
            events: self.events.clone(),
            page: url,
            sheet_urls,
            scripts,
            img_urls,
            doc_css,
            eng,
            css_slots: Vec::new(),
            script_srcs: Vec::new(),
            img_bodies: HashMap::new(),
            async_mods: Vec::new(),
            fonts: Vec::new(),
            arrival_evals: Vec::new(),
            post_evals: Vec::new(),
            max_inflight: self.max_inflight,
            max_modules: self.max_modules,
            font_budget: self.max_fonts,
        }))
    }

    /// Phase finale — commit des sous-ressources et du rendu
    /// sous `&mut self` : cascade, moteur JS, files
    /// d'exécution (les graphes modules sont déjà remplis —
    /// synchrones), images, fontes, layout, historique et
    /// `Navigated`. `None` = epoch supplantée, tout est jeté.
    async fn sub_commit(&mut self, tab: TabId, job: SubJob) -> Result<Option<String>, CoreError> {
        {
            let entry = self.tabs.get(&tab).ok_or(CoreError::UnknownTab(tab))?;
            if entry.nav_epoch != job.epoch {
                self.unspec(tab);
                return Ok(None);
            }
        }
        let SubJob {
            url,
            push,
            prev_title,
            req,
            title,
            css_slots,
            scripts,
            script_srcs,
            img_bodies,
            mut eng,
            fonts,
            arrival_evals,
            post_evals,
            ..
        } = job;
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
        for (family, weight, italic, data) in fonts {
            self.webfonts.register(&family, weight, italic, data);
        }
        if let Some(eng) = eng.as_mut() {
            for item in scripts.iter().filter(|i| i.exec == ScriptExec::Parse) {
                let Some(src) = item_source(item, &script_srcs) else {
                    continue;
                };
                self.eval_or_report(tab, eng.as_mut(), src);
            }
            for me in arrival_evals {
                self.eval_module_graph(tab, eng.as_mut(), me);
            }
            let mut post_evals = post_evals.into_iter();
            for item in scripts.iter().filter(|i| i.exec == ScriptExec::Post) {
                if item.module {
                    if let Some(me) = post_evals.next() {
                        self.eval_module_graph(tab, eng.as_mut(), me);
                    }
                } else if let Some(src) = item_source(item, &script_srcs) {
                    self.eval_or_report(tab, eng.as_mut(), src);
                }
            }
            let n = eng.run_pending();
            tracing::trace!(tab, jobs = n, "jobs js exécutés");
            // Miroir muté ? → re-parse : le relayout qui suit
            // rend les modifications (dirty bit consommé côté
            // moteur).
            if let Some((mutated, _)) = eng.sync_dom_path() {
                self.apply_dom_sync(tab, &mutated, None, false);
            }
        }
        if let Some(entry) = self.tabs.get_mut(&tab) {
            entry.js = eng;
        }
        // Étape 51 : XHR/fetch émis pendant le chargement →
        // spawned ; une pompe livre les réponses déjà là,
        // l'hôte pompe périodiquement pour le reste.
        self.after_script(tab);
        self.pump_net().await;
        self.insert_images(tab, &url, &img_bodies);
        self.relayout(tab)?;
        // La page chargée prend sa place dans l'historique
        // AVANT le drain script : un `history.go` en vol la
        // voit (index réel), une redirection script pousse sa
        // propre entrée après la sienne.
        if push {
            if let Some(entry) = self.tabs.get_mut(&tab) {
                entry.hist.truncate(entry.hist_idx + 1);
                // La **requête** complète est historiée
                // (étape 56) : un POST de formulaire devient
                // une position rejouable — pas de dédup
                // d'URL, le résultat est une page à part
                // entière comme chez les navigateurs.
                entry.hist.push(req.clone());
                entry.hist_idx = entry.hist.len() - 1;
            }
        }
        // `Navigated` seulement si le document final est bien
        // celui demandé : une redirection script a déjà émis
        // pour sa cible. Le titre annoncé = `entry.title` —
        // les scripts ont pu le muter par `document.title=`
        // (`apply_dom_sync` le tient à jour) : le titre extrait
        // au parse serait alors périmé et ré-émettrait
        // l'ancien (52b).
        let final_url = self.tabs.get(&tab).and_then(|t| t.url.clone());
        if final_url.as_ref() == Some(&url) {
            let live = self.tabs.get(&tab).and_then(|t| t.title.clone());
            self.emit_navigated(tab, &url, live, prev_title.as_deref());
        }
        // Les `Set-Cookie` du document et des sous-ressources
        // ont enrichi le jar partagé (CookieTransport agit
        // hors de la vue session) — checkpoint de
        // persistance (étape 56).
        self.persist_cookies();
        // Fragment cross-document (étape 56d) : la page est
        // posée — scrolle vers `id`/`name` si l'URL en porte.
        if let Some(frag) = url.fragment() {
            let frag = frag.to_string();
            self.scroll_to_fragment(tab, &frag);
        }
        Ok(title)
    }

    /// Évalue un `<script type="module">` dont le graphe est
    /// déjà rempli (prefetch détaché dans [`SubJob::run`]) —
    /// `eval_module` + `PipelineError` sans faire échouer la
    /// page.
    fn eval_module_graph(&mut self, tab: TabId, eng: &mut dyn JsEngine, me: ModEval) {
        let ModuleGraph { sources, alias } = me.graph;
        let resolve: UrlResolver = Arc::new(move |spec, referrer| {
            let b = alias.get(referrer).map_or(referrer, String::as_str);
            Url::parse(b)
                .ok()
                .and_then(|u| Url::resolve(&u, spec).ok())
                .map(|u| u.as_str())
        });
        if let Err(e) = eng.eval_module(&me.name, sources, resolve) {
            tracing::debug!(tab, error = %e, "module en échec");
            let _ = self.events.send(CoreEvent::PipelineError {
                tab: Some(tab),
                message: e.to_string(),
            });
        }
    }

    /// Dépile la prochaine intention de navigation.
    fn pop_pending_nav(&mut self, tab: TabId) -> Option<DrainAct> {
        self.tabs
            .get_mut(&tab)
            .and_then(|e| e.pending_nav.pop_front())
    }

    /// Enfile une intention — `false` si la file est pleine
    /// (`MAX_PENDING_NAV`) : l'appelant cesse alors de
    /// vider le moteur (un flot hostile ne fait pas gonfler
    /// la mémoire, et aucune intention n'est jetée en
    /// silence — celles encore dans le moteur restent
    /// drainables au prochain tour).
    fn enqueue_nav_act(&mut self, tab: TabId, act: DrainAct) -> bool {
        const MAX_PENDING_NAV: usize = 8;
        if let Some(e) = self.tabs.get_mut(&tab) {
            if e.pending_nav.len() < MAX_PENDING_NAV {
                e.pending_nav.push_back(act);
                return true;
            }
        }
        false
    }

    /// Déverse les intentions script du moteur dans
    /// `pending_nav` (étape 53d) — les handlers consultent
    /// ensuite [`Self::nav_pending`] au lieu de conduire les
    /// navigations sous `&mut self`. Stoppe quand la file
    /// est pleine : le reste reste dans le moteur pour le
    /// prochain drain.
    fn collect_script_navs(&mut self, tab: TabId) {
        while let Some(n) = self.next_script_nav(tab) {
            let act = match n {
                ScriptNav::To(u) => DrainAct::Req(Box::new(Request::get(u)), true),
                ScriptNav::Go(d) => DrainAct::Go(d),
                ScriptNav::Submit(req) => DrainAct::Req(Box::new(req), true),
            };
            if !self.enqueue_nav_act(tab, act) {
                break;
            }
        }
    }

    /// Cible `(index, requête)` d'un déplacement
    /// d'historique — lecture pure pour les drivers partagés
    /// (53d) ; la requête d'origine est rejouée telle quelle
    /// hors bfcache (étape 56 — POST rejoué).
    fn hist_target(&self, tab: TabId, delta: isize) -> Result<(usize, Request), CoreError> {
        let entry = self.tabs.get(&tab).ok_or(CoreError::UnknownTab(tab))?;
        let Some(idx) = entry
            .hist_idx
            .checked_add_signed(delta)
            .filter(|&i| i < entry.hist.len())
        else {
            return Err(CoreError::EmptyDocument);
        };
        Ok((idx, entry.hist[idx].clone()))
    }

    /// Fixe l'index courant d'historique (commit côté driver
    /// partagé, 53d).
    fn set_hist_idx(&mut self, tab: TabId, idx: usize) {
        if let Some(e) = self.tabs.get_mut(&tab) {
            e.hist_idx = idx;
        }
    }

    /// La session a-t-elle des intentions de navigation en
    /// attente pour `tab` (input ou script) — sonde des
    /// drivers partagés (53d).
    #[must_use]
    pub fn nav_pending(&self, tab: TabId) -> bool {
        self.tabs
            .get(&tab)
            .is_some_and(|e| !e.pending_nav.is_empty())
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
                NavRequest::FormSubmit {
                    method,
                    action,
                    enctype,
                    fields,
                } => {
                    let base = self.tabs.get(&tab).and_then(|t| t.url.as_ref());
                    if let Some(req) = base
                        .and_then(|b| script_form_request(b, &method, &action, &enctype, &fields))
                    {
                        return Some(ScriptNav::Submit(req));
                    }
                }
            }
        }
        None
    }

    /// Drainage des intentions de navigation accumulées —
    /// input (`pending_nav`, étape 53d) puis script
    /// (`location`, `history`, `form.submit()`) — appelé
    /// après tout point de synchro producteur : événements,
    /// fin de chargement, tick timers (étape 52 — les hôtes
    /// la relancent après [`Self::pump_timers`]). Publique
    /// pour les pompes des hôtes `&mut self` (CLI, FFI,
    /// tests) — les hôtes à session partagée passent par
    /// [`drain_navs_shared`]. Boucle bornée par
    /// `MAX_NAV_HOPS`, jamais de récursion — chaque
    /// navigation est conduite par `navigate_request` en
    /// feuille du graphe d'appels (seul `history_move`,
    /// boxé, boucle pour la taille du future). `true` si au
    /// moins une navigation a été appliquée.
    pub async fn drain_script_nav(&mut self, tab: TabId) -> bool {
        const MAX_NAV_HOPS: u8 = 8;
        let mut applied = false;
        for _ in 0..MAX_NAV_HOPS {
            self.collect_script_navs(tab);
            let Some(act) = self.pop_pending_nav(tab) else {
                break;
            };
            match act {
                DrainAct::Req(req, push) => {
                    let _ = self.navigate_request(tab, *req, push).await;
                }
                DrainAct::Go(0) => {
                    // `history.go(0)` = rechargement de
                    // l'entrée courante (spec HTML).
                    if let Some(u) = self.tabs.get(&tab).and_then(|t| t.url.clone()) {
                        let _ = self.navigate_to(tab, u).await;
                    }
                }
                DrainAct::Go(d) => {
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
            eng.sync_dom_path()
        };
        if let Some((html, dirty)) = mutated {
            self.apply_dom_sync(tab, &html, dirty.as_deref(), true);
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
            if !writes.is_empty() {
                self.persist_cookies();
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
        // Des `Set-Cookie` XHR ont pu enrichir le jar —
        // persistance au même point de synchro.
        if n > 0 {
            self.persist_cookies();
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
            // `data:` (et autres non-http) ne se fetch pas —
            // peints inline par `collect_data_images` (56f).
            .filter(|u| matches!(u.scheme(), "http" | "https"))
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
                entry.images.replaced.insert(node, img.clone());
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
    /// `dirty_path` = chemin du sous-arbre sale rapporté par
    /// le miroir (étape 55f) — sert à viser
    /// [`Self::try_sync_incremental`] au lieu du reflow global.
    fn apply_dom_sync(
        &mut self,
        tab: TabId,
        html: &str,
        dirty_path: Option<&[(String, u32)]>,
        relayout: bool,
    ) {
        let new_dom = TreeBuilder::parse(html);
        let new_title = extract_title(&new_dom);
        // Relayout incrémental (étape 55f) avant tout remplacement
        // — il a besoin de l'ancien DOM et de l'ancien arbre.
        let incremental =
            relayout && dirty_path.is_some_and(|p| self.try_sync_incremental(tab, &new_dom, p));
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
            // En chemin incrémental `svgs`/`tree` sont déjà
            // à jour ; `replaced` seule est vidée.
            entry.images = PaintImages::default();
            // Un re-parse de synchro rend obsolètes les
            // options figées du popup `<select>` — il se
            // referme (étape 54).
            entry.popup = None;
            if entry.title != new_title {
                entry.title.clone_from(&new_title);
                if let Some(t) = new_title {
                    let _ = self.events.send(CoreEvent::TitleChanged { tab, title: t });
                }
            }
        }
        if relayout {
            if incremental {
                self.emit_invalidated(tab);
                return;
            }
            if let Err(e) = self.relayout(tab) {
                tracing::debug!(tab, error = %e, "relayout après mutation js");
                return;
            }
            self.emit_invalidated(tab);
        }
    }

    /// Relayout incrémental d'une synchro DOM (étape 55f —
    /// adoption R-12) : le sous-arbre sale rapporté par le
    /// miroir est reboîté + re-posé en place par
    /// [`relayout_subtree`] **si** la cible résout au même
    /// `NodeId` dans l'ancien et le nouveau DOM (le préfixe
    /// structurel est alors stable — une mutation interne au
    /// sous-arbre ne décale pas son propre id). Les boîtes
    /// conservées portent d'anciens ids : re-mappés par
    /// chemin d'élément — un id qui ne se re-résout pas
    /// proprement renvoie `false` (l'appelant refait un
    /// reflow global : l'arbre mixte est alors entièrement
    /// reconstruit — toujours sûr).
    fn try_sync_incremental(&mut self, tab: TabId, new_dom: &Dom, path: &[(String, u32)]) -> bool {
        // Phase 1 — résolution de la cible sous emprunt court.
        let resolve = |s: &Session, dom: &Dom| {
            let entry = s.tabs.get(&tab)?;
            let old_dom = entry.dom.as_ref()?;
            entry.tree.as_ref()?;
            let sizes = image_sizes(dom, &entry.images.replaced);
            Some((dom_path_node(old_dom, path)?, sizes))
        };
        let Some((target_old, mut sizes)) = resolve(self, new_dom) else {
            return false;
        };
        let Some(target_new) = dom_path_node(new_dom, path) else {
            return false;
        };
        let Some(root_el) = new_dom.document_element() else {
            return false;
        };
        let styles = self.compute_styles(tab, new_dom);
        sizes.extend(explorer_render::svg::svg_sizes(new_dom, root_el));
        let svgs = explorer_render::svg::collect_svg(new_dom, root_el, self.image_max_px);
        let ctx = BlockCtx {
            fonts: Some(&self.webfonts as &dyn FontSource),
            ..BlockCtx::default()
        };
        // Phase 2 — remap des ids conservés + relayout local,
        // sous emprunt mutable court. Le remap précède le
        // relayout : `bx.node == target` doit comparer des
        // ids du **nouveau** DOM (les indices d'arène ne sont
        // pas stables au re-parse — la sérialisation n'est
        // pas structure-identique).
        {
            let Some(entry) = self.tabs.get_mut(&tab) else {
                return false;
            };
            let (Some(old_dom), Some(tree)) = (entry.dom.as_ref(), entry.tree.as_mut()) else {
                return false;
            };
            if !remap_box_nodes(old_dom, new_dom, tree, target_old, target_new) {
                return false; // id irrésolvable → reflow global de l'appelant
            }
            if relayout_subtree(tree, new_dom, &styles, &sizes, target_new, &ctx) != Relayout::Done
            {
                return false; // arbre mixte jeté → fallback sûr
            }
            entry.doc_h = tree.rect.height;
            entry.images.svgs = svgs;
        }
        if self.repaint(tab).is_ok() {
            self.incremental_syncs += 1;
            return true;
        }
        false
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
        self.emit_invalidated(tab);
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
        self.emit_invalidated(tab);
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

    /// Pipeline style → layout → raster sur `dom` (emprunté —
    /// `relayout` pour le DOM posé, `nav_paint` pour le DOM
    /// en construction). Écrit `tree`/`doc_h`/frame/png.
    /// Passe de styles partagée par le reflow complet et le
    /// relayout incrémental (étape 55f) : cascade `<style>` +
    /// `<link>`, calcul sur tout `dom`, surcharge des
    /// transitions en vol. Effet de bord : `prev_styles`/
    /// `transitions` de l'onglet mis à jour.
    fn compute_styles(&mut self, tab: TabId, dom: &Dom) -> StyleMap {
        // Feuilles : `<style>` inline puis `<link>` fetchées
        // (étape 46) — même origine auteur, ordre source.
        let linked = self
            .tabs
            .get(&tab)
            .map_or_else(String::new, |e| e.linked_css.clone());
        let mut eng = StyleEngine::new();
        eng.add_css(&collect_style_css(dom), Origin::Author);
        eng.add_css(&linked, Origin::Author);
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
        if let Some(e) = self.tabs.get_mut(&tab) {
            update_transitions(
                &e.prev_styles,
                &mut e.transitions,
                &mut styles,
                &ord_of,
                Instant::now(),
            );
            e.prev_styles = raw;
        }
        styles
    }

    fn relayout_dom(&mut self, tab: TabId, dom: &Dom) -> Result<(), CoreError> {
        let styles = self.compute_styles(tab, dom);
        let entry = self.tabs.get_mut(&tab).ok_or(CoreError::UnknownTab(tab))?;
        let Some(root_el) = dom.document_element() else {
            return Err(CoreError::EmptyDocument);
        };
        // Images `data:` inline (étape 56f) : zéro réseau —
        // `<img src=data:>`/`<object data=data:>` → `replaced`
        // (sert le sizing intrinsèque juste après),
        // `background-image: url(data:)` en cascade →
        // `backgrounds`. Fusionnées avec les corps fetchés.
        let data_imgs = collect_data_images(dom, &styles, self.image_max_px);
        entry.images.replaced.extend(data_imgs.replaced);
        entry.images.backgrounds.extend(data_imgs.backgrounds);
        let mut sizes = image_sizes(dom, &entry.images.replaced);
        // `<svg>` inline (étape 55b) : tailles intrinsèques
        // `width`/`height`/`viewBox` pour la mise en boîte,
        // bitmap rasterisé une fois par reflow pour la
        // peinture (borné `image_max_px`, sous-arbres hostiles
        // refusés → la boîte retombe remplacée sans image).
        sizes.extend(explorer_render::svg::svg_sizes(dom, root_el));
        let svgs = explorer_render::svg::collect_svg(dom, root_el, self.image_max_px);
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
        entry.images.svgs = svgs;
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
        let mut list = build_display_list_full(bx, vw, vh, entry.scroll_y, &entry.images);
        // Popup `<select>` ouvert : overlay chrome peint en
        // dernier — au-dessus de tout contenu (étape 54).
        if let Some(pop) = &entry.popup {
            append_select_popup(&mut list, pop, vw, entry.doc_h);
        }
        // Document géant : plafonner la hauteur rasterisée
        // plutôt qu'échouer — le PNG couvre le sommet du
        // document (dégradation, cf. `MAX_FRAME_DIM`). La
        // borne du raster mesure des pixels : le cap CSS
        // se divise par `scale`.
        let frame_cap = MAX_FRAME_DIM / self.scale.max(f32::EPSILON);
        if list.height > frame_cap {
            list.height = frame_cap;
        }
        let frame = rasterize(&list, &self.webfonts as &dyn FontSource, self.scale)
            .ok_or(CoreError::FrameTooBig)?;
        let png = frame.encode_png();
        let entry = self.tabs.get_mut(&tab).ok_or(CoreError::UnknownTab(tab))?;
        // Dirty-rects (étape 55) : diff avec la liste
        // précédente — première peinture ou listes
        // incomparables → toute la frame (conservateur).
        let regions = entry.list.as_ref().map_or_else(
            || {
                vec![Rect {
                    x: 0.0,
                    y: 0.0,
                    width: list.width,
                    height: list.height,
                }]
            },
            |prev| list.changed_rects(prev),
        );
        entry.dirty.extend(regions);
        if entry.dirty.len() > MAX_DIRTY_RECTS {
            // Collapse « toute la frame » — le watermark est
            // ré-armé : le plein cadre sera rapporté à la
            // prochaine émission.
            entry.emitted = 0;
            entry.dirty = vec![Rect {
                x: 0.0,
                y: 0.0,
                width: list.width,
                height: list.height,
            }];
        }
        entry.list = Some(list);
        entry.frame = Some(frame);
        entry.png = png;
        // Une peinture aboutie produit un état cohérent
        // (étape 56c) — le drapeau spéculatif est ré-armé
        // par `nav_paint` **après** son propre appel ici.
        entry.spec_paint = false;
        Ok(())
    }

    /// Applique un événement d'entrée à l'onglet : hit-test
    /// sur l'arbre posé, scroll borné, suivi de lien.
    /// Corps synchrone + drain compat `&mut self` (étape
    /// 53d) — les hôtes à session partagée appellent
    /// [`Self::handle_input_now`] puis `drain_navs_shared`.
    ///
    /// # Errors
    /// [`CoreError::UnknownTab`], ou erreur de navigation si
    /// un clic de lien déclenche un fetch qui échoue.
    pub async fn handle_input(
        &mut self,
        tab: TabId,
        ev: InputEvent,
    ) -> Result<InputOutcome, CoreError> {
        let out = self.handle_input_now(tab, ev)?;
        self.drain_script_nav(tab).await;
        Ok(out)
    }

    /// Corps synchrone de [`Self::handle_input`] (étape
    /// 53d) : dispatche l'événement DOM et pose les
    /// intentions de navigation dans `pending_nav` —
    /// **jamais de fetch sous `&mut self`**. Les hôtes à
    /// session partagée enchaînent `drain_navs_shared` ;
    /// `handle_input` draine en compat.
    ///
    /// # Errors
    /// [`CoreError::UnknownTab`] ou erreur de pipeline
    /// d'édition/re-layout.
    pub fn handle_input_now(
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
                // Clavier du popup `<select>` ouvert : les
                // flèches déplacent le surlignage, `Enter`
                // choisit, `Escape` referme — consommé avant
                // tout focus d'édition (étape 54).
                if self.tabs.get(&tab).is_some_and(|e| e.popup.is_some()) {
                    return self.popup_key(tab, &key);
                }
                // Soumission implicite : `Enter` dans un champ
                // d'un `<form>` suit la séquence `try_submit`
                // complète — validation + `submit` + bouton
                // par défaut comme `submitter` (HTML).
                if key == "Enter" {
                    if let Some((form, sub)) = self.focused_form_default(tab) {
                        self.set_focus(tab, None);
                        return self.try_submit(tab, form, sub, false);
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
                // Un popup `<select>` ouvert se referme au
                // scroll (étape 54 — ancrage document).
                if let Some(entry) = self.tabs.get_mut(&tab) {
                    entry.popup = None;
                }
                if !self.dispatch_wheel(tab) {
                    self.scroll_by(tab, dy)?;
                    // `scroll` niveau document (non annulable).
                    self.dispatch_doc_event(tab, "scroll");
                }
                Ok(InputOutcome::Scrolled {
                    scroll_y: self.tabs.get(&tab).map_or(0.0, |t| t.scroll_y),
                })
            }
            InputEvent::MouseMove { x, y } => self.handle_mouse_move(tab, x, y),
            InputEvent::Click { x, y } => self.handle_click(tab, x, y),
        }
    }

    /// Survol suivi (étape 52) : `mouseout` sur l'ancienne
    /// cible, `mouseover`/`mousemove` sur la nouvelle — déjà
    /// coalescé côté client (ADR-0003), un événement par
    /// `move`. Les dispatches se font par **chemin** : chaque
    /// handler peut muter le miroir et re-parser le document —
    /// un `NodeId` capté en début de bras serait mort dès le
    /// premier dispatch.
    fn handle_mouse_move(&mut self, tab: TabId, x: f32, y: f32) -> Result<InputOutcome, CoreError> {
        // Popup `<select>` ouvert : le curseur pilote le
        // surlignage de ligne — capture chrome, pas
        // d'événements DOM vers le document (étape 54).
        let pop_hit = self.tabs.get_mut(&tab).and_then(|entry| {
            entry.popup.as_mut().map(|pop| {
                let row = popup_row(pop, x, y + entry.scroll_y, self.viewport.0, entry.doc_h);
                let changed = pop.hovered != row;
                pop.hovered = row;
                changed
            })
        });
        if let Some(changed) = pop_hit {
            if changed {
                self.repaint(tab)?;
                self.emit_invalidated(tab);
            }
            return Ok(InputOutcome::Hover {
                url: None,
                editable: false,
            });
        }
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
        self.collect_script_navs(tab);
        if self.nav_pending(tab) {
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
    fn handle_click(&mut self, tab: TabId, x: f32, y: f32) -> Result<InputOutcome, CoreError> {
        // Popup `<select>` ouvert : le clic est consommé par
        // l'overlay chrome — choix d'option ou fermeture,
        // sans `click` DOM sur le document (étape 54).
        if let Some(out) = self.popup_click(tab, x, y)? {
            return Ok(out);
        }
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
            self.emit_invalidated(tab);
            return Ok(if prevented {
                InputOutcome::Ignored
            } else {
                InputOutcome::Edited
            });
        }
        // Navigation script (`location`, `history`) : priorité
        // sur l'action par défaut du clic.
        self.collect_script_navs(tab);
        if self.nav_pending(tab) {
            return Ok(InputOutcome::Navigated);
        }
        if let Some(field) = self.field_at(tab, x, y) {
            // `blur` sur l'ancien focus puis `focus` sur le
            // champ (étape 52).
            self.set_focus(tab, Some(field));
            return Ok(InputOutcome::Focused);
        }
        self.set_focus(tab, None);
        // `<select>` : ouvre/ferme le popup options moteur
        // (étape 54 — overlay de la display list, peint
        // chrome-side, remplace le cycle MVP d'ADR-0014).
        if let Some(sel) = self.select_at(tab, x, y) {
            self.toggle_select_popup(tab, sel)?;
            self.emit_invalidated(tab);
            return Ok(InputOutcome::Edited);
        }
        // Contrôle submit : validation de contraintes puis
        // événement `submit` puis action par défaut — la
        // séquence est factorisée dans `try_submit` (partagée
        // avec la soumission implicite `Enter`).
        if let Some((form, sub)) = self.submit_at(tab, x, y) {
            return self.try_submit(tab, form, Some(sub), prevented);
        }
        match self.link_at(tab, x, y) {
            Some(url) if !prevented => {
                let u = Url::parse(&url).map_err(|e| CoreError::Url(e.to_string()))?;
                self.enqueue_nav_act(tab, DrainAct::Req(Box::new(Request::get(u)), true));
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

    /// Ouvre/ferme le popup `<select>` (étape 54) — options
    /// et boîte figées à l'ouverture ; le nœud ne survit pas
    /// forcément, on ne garde que son chemin.
    fn toggle_select_popup(&mut self, tab: TabId, sel: NodeId) -> Result<(), CoreError> {
        {
            let entry = self.tabs.get_mut(&tab).ok_or(CoreError::UnknownTab(tab))?;
            if entry.popup.is_some() {
                entry.popup = None;
                return self.repaint(tab);
            }
        }
        let entry = self.tabs.get(&tab).ok_or(CoreError::UnknownTab(tab))?;
        let (tree, dom) = (entry.tree.as_ref(), entry.dom.as_ref());
        let (Some(tree), Some(dom)) = (tree, dom) else {
            return Err(CoreError::EmptyDocument);
        };
        let Some(rect) = box_rect(tree, sel) else {
            return Ok(());
        };
        let Some(path) = element_path(dom, sel) else {
            return Ok(());
        };
        let options = descendant_elements(dom, sel, "option")
            .into_iter()
            .map(|o| {
                let label =
                    dom.attribute(o, "label").ok().flatten().unwrap_or_else(|| {
                        dom.text_content(o).unwrap_or_default().trim().to_string()
                    });
                (label, dom.attribute(o, "selected").ok().flatten().is_some())
            })
            .collect();
        let entry = self.tabs.get_mut(&tab).ok_or(CoreError::UnknownTab(tab))?;
        entry.popup = Some(SelectPopup {
            path,
            rect,
            options,
            row_h: rect.height.clamp(14.0, 32.0),
            hovered: None,
        });
        self.repaint(tab)
    }

    /// Clic pendant qu'un popup `<select>` est ouvert — la
    /// ligne choisie applique `selected`, `input`+`change`
    /// partent, le popup se ferme ; un clic hors popup le
    /// ferme simplement. `Some` = clic consommé.
    fn popup_click(
        &mut self,
        tab: TabId,
        x: f32,
        y: f32,
    ) -> Result<Option<InputOutcome>, CoreError> {
        let entry = self.tabs.get(&tab).ok_or(CoreError::UnknownTab(tab))?;
        let Some(pop) = &entry.popup else {
            return Ok(None);
        };
        let row = popup_row(pop, x, y + entry.scroll_y, self.viewport.0, entry.doc_h);
        let path = pop.path.clone();
        // Consommé dans tous les cas : un clic hors popup le
        // referme simplement (comportement natif).
        self.popup_choose(tab, &path, row).map(Some)
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

    /// `(form, bouton par défaut)` englobant le champ focalisé
    /// — la soumission implicite sur `Enter` ne vaut que pour
    /// les `<input>` mono-ligne (`<textarea>` garde son saut
    /// de ligne). Le bouton par défaut est le premier contrôle
    /// submit du formulaire (HTML §4.10.22.2).
    fn focused_form_default(&self, tab: TabId) -> Option<(NodeId, Option<NodeId>)> {
        let entry = self.tabs.get(&tab)?;
        let dom = entry.dom.as_ref()?;
        let field = entry.focused.as_ref().and_then(|p| dom_path_node(dom, p))?;
        if !dom.tag_name(field).is_ok_and(|t| t == "input") {
            return None;
        }
        let form = enclosing_form(dom, field)?;
        Some((form, default_button(dom, form)))
    }

    /// Touche pendant qu'un popup `<select>` est ouvert :
    /// `ArrowDown`/`ArrowUp` déplacent le surlignage (borné),
    /// `Enter` choisit la ligne surlignée, `Escape` referme —
    /// toutes les autres se ferment sans choix.
    fn popup_key(&mut self, tab: TabId, key: &str) -> Result<InputOutcome, CoreError> {
        let entry = self.tabs.get(&tab).ok_or(CoreError::UnknownTab(tab))?;
        let Some(pop) = &entry.popup else {
            return Ok(InputOutcome::Ignored);
        };
        let rows = pop.options.len().min(MAX_POPUP_ROWS);
        let path = pop.path.clone();
        match key {
            "ArrowDown" | "ArrowUp" => {
                let next = match pop.hovered {
                    Some(h) if key == "ArrowDown" => (h + 1).min(rows.saturating_sub(1)),
                    Some(h) => h.saturating_sub(1),
                    None if key == "ArrowDown" => 0,
                    None => rows.saturating_sub(1),
                };
                let entry = self.tabs.get_mut(&tab).ok_or(CoreError::UnknownTab(tab))?;
                if let Some(pop) = entry.popup.as_mut() {
                    pop.hovered = Some(next);
                }
                self.repaint(tab)?;
                self.emit_invalidated(tab);
                Ok(InputOutcome::Edited)
            }
            "Enter" => {
                let row = pop
                    .hovered
                    .or_else(|| pop.options.iter().position(|&(_, sel)| sel));
                self.popup_choose(tab, &path, row)
            }
            _ => {
                let entry = self.tabs.get_mut(&tab).ok_or(CoreError::UnknownTab(tab))?;
                entry.popup = None;
                self.repaint(tab)?;
                self.emit_invalidated(tab);
                Ok(InputOutcome::Ignored)
            }
        }
    }

    /// Choix `row` dans le popup `<select>` identifié par
    /// `path` — ferme le popup, applique `selected`,
    /// `input`+`change` (queue commune clic/clavier).
    fn popup_choose(
        &mut self,
        tab: TabId,
        path: &[(String, u32)],
        row: Option<usize>,
    ) -> Result<InputOutcome, CoreError> {
        {
            let entry = self.tabs.get_mut(&tab).ok_or(CoreError::UnknownTab(tab))?;
            entry.popup = None;
        }
        let sel = row.and_then(|_| self.resolve_path(tab, Some(path)));
        if let (Some(sel), Some(idx)) = (sel, row) {
            let opts = {
                let entry = self.tabs.get_mut(&tab).ok_or(CoreError::UnknownTab(tab))?;
                let dom = entry.dom.as_mut().ok_or(CoreError::EmptyDocument)?;
                choose_option(dom, sel, idx);
                descendant_elements(dom, sel, "option")
            };
            // Le miroir JS reflète le nouveau `selected` — un
            // handler `change` qui lit `option.selected` voit
            // l'état post-choix (comme `flip_checked`).
            for (i, &o) in opts.iter().enumerate() {
                self.sync_mirror_attr(tab, o, "selected", (i == idx).then_some(""));
            }
            self.relayout(tab)?;
            self.dispatch_path_event(tab, "input", path);
            self.dispatch_path_event(tab, "change", path);
        } else {
            self.repaint(tab)?;
        }
        self.emit_invalidated(tab);
        Ok(InputOutcome::Edited)
    }

    /// Séquence de soumission complète (HTML §4.10.22) —
    /// validation de contraintes, événement `submit`,
    /// action par défaut. Partagée entre le clic sur un
    /// contrôle submit (`click_prevented` = `preventDefault`
    /// du `click`) et la soumission implicite `Enter`
    /// (`submitter` = bouton par défaut du formulaire).
    ///
    /// `novalidate` sur le `<form>` ou `formnovalidate` sur
    /// le `submitter` sautent la validation (HTML).
    fn try_submit(
        &mut self,
        tab: TabId,
        form: NodeId,
        submitter: Option<NodeId>,
        click_prevented: bool,
    ) -> Result<InputOutcome, CoreError> {
        let (form_path, sub_path, novalidate) = {
            let dom = self.tabs.get(&tab).and_then(|e| e.dom.as_ref());
            let nv = dom.is_some_and(|d| {
                d.attribute(form, "novalidate").ok().flatten().is_some()
                    || submitter
                        .is_some_and(|s| d.attribute(s, "formnovalidate").ok().flatten().is_some())
            });
            (
                dom.and_then(|d| element_path(d, form)),
                dom.and_then(|d| submitter.and_then(|s| element_path(d, s))),
                nv,
            )
        };
        // Validation de contraintes (étape 54) : chaque
        // contrôle candidat invalide reçoit `invalid`
        // (non-annulable) puis la soumission est bloquée —
        // `submit` ne part que si tout est valide.
        if !click_prevented && !novalidate {
            let invalid = {
                let dom = self.tabs.get(&tab).and_then(|e| e.dom.as_ref());
                dom.map(|d| invalid_controls(d, form)).unwrap_or_default()
            };
            if !invalid.is_empty() {
                for &ctrl in &invalid {
                    self.dispatch_dom_event(tab, "invalid", ctrl);
                }
                return Ok(InputOutcome::Ignored);
            }
        }
        // `form`/`sub` sont re-résolus après le dispatch — un
        // handler peut re-parser le document (ids morts,
        // chemins survivants).
        if !click_prevented && !self.dispatch_dom_event(tab, "submit", form) {
            self.collect_script_navs(tab);
            if self.nav_pending(tab) {
                return Ok(InputOutcome::Navigated);
            }
            let f = self.resolve_path(tab, form_path.as_deref());
            let s = self.resolve_path(tab, sub_path.as_deref());
            if let Some(f) = f {
                self.submit_form(tab, f, s)?;
                return Ok(InputOutcome::Navigated);
            }
        }
        Ok(InputOutcome::Ignored)
    }

    /// Soumission de formulaire (étape 38) : contrôles
    /// successful → `GET` (query) ou `POST` urlencodé —
    /// posée en intention `pending_nav` (étape 53d : jamais
    /// de fetch sous `&mut self`, le drain de l'appelant
    /// conduit le pipeline complet sur la réponse).
    /// L'historique retient l'URL d'action — un reload/back
    /// refait un `GET` (déviation documentée ADR-0014).
    fn submit_form(
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
            let enctype = attr_lower(dom, form, "enctype");
            form_request(base, &action, &method, &enctype, &pairs)
                .ok_or_else(|| CoreError::Url("action de formulaire irrésolvable".to_string()))?
        };
        self.enqueue_nav_act(tab, DrainAct::Req(Box::new(req), true));
        Ok(())
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
        self.emit_invalidated(tab);
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
            (p, eng.sync_dom_path())
        };
        if let Some((html, dirty)) = mutated {
            self.apply_dom_sync(tab, &html, dirty.as_deref(), true);
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
        self.emit_invalidated(tab);
        Ok(())
    }
}

/// Hit-test profond : la boîte la plus imbriquée contenant
/// `(x, y)` document gagne — renvoie son `NodeId` source
/// (`None` pour les boîtes anonymes → on remonte au parent
/// peint le plus proche via les ancêtres DOM côté appelant).
fn hit_test(bx: &LayoutBox, x: f32, y: f32) -> Option<NodeId> {
    // R-19 : l'élagage sur le border-box tuait les zones dont
    // un enfant déborde le parent (lite.ddg). Le test vit en
    // amont : `explorer_layout::hit_test` élague sur l'union
    // du sous-arbre (paint bounds) — même contrat de retour.
    explorer_layout::hit_test(bx, x, y)
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

/// Future de peinture progressive (étape 53d) — bornée par
/// la durée de vie du `&Dom` emprunté au `TreeBuilder`.
type PaintFut<'a> = std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>>;

/// Phase « fetch + parse » d'une navigation, détachable de
/// la session (étape 53d) : construite sous `&mut self` par
/// [`Session::nav_begin`] (epoch armée, `Arc` du transport
/// cloné), [`NavJob::run`] ne réclame aucun `&Session` —
/// un hôte à `Mutex<Session>` garde son API réactive
/// pendant tout le téléchargement. La peinture progressive
/// est livrée à un rappel injecté : chaque snapshot du DOM
/// en construction est consommé dans sa propre future
/// (re-lock court côté appelant partagé, `nav_paint`
/// direct côté `&mut self`).
struct NavJob {
    /// Epoch capturée à `nav_begin` — comparée à
    /// `Tab::nav_epoch` à chaque reprise du verrou.
    epoch: u64,
    /// Requête de navigation (GET/POST).
    req: Request,
    /// `push` du commit (historique).
    push: bool,
    /// Titre de l'onglet avant navigation — `TitleChanged`.
    prev_title: Option<String>,
    /// Transport cloné — fetch hors session.
    transport: Arc<dyn HttpTransport>,
    /// Cadence de peinture progressive.
    paint_chunk_bytes: usize,
    paint_interval: std::time::Duration,
}

/// Queue de [`NavJob::run`] — ce que le commit consomme
/// après le `self` du job (re-publié sans l'emprunt du
/// rappel de peinture).
struct NavTail {
    /// Epoch — garde [`Session::nav_apply`].
    epoch: u64,
    /// `push` historique.
    push: bool,
    /// Titre avant navigation — `TitleChanged`.
    prev_title: Option<String>,
    /// Requête d'origine — rejouée par `reload` et stockée
    /// dans l'entrée d'historique (étape 56, POST).
    req: Request,
}

/// Sortie de [`NavJob::run`] — document parsé prêt à poser.
struct NavDone {
    /// URL effectivement naviguée.
    url: Url,
    /// DOM du document complet.
    dom: Dom,
    /// HTML décodé (miroir JS à `nav_apply`).
    html: String,
    /// Titre extrait.
    title: Option<String>,
}

impl NavJob {
    /// Fetch + parse incrémental hors session : chaque
    /// morceau décodé alimente le tree-builder —
    /// `TreeBuilder::push` garantit un arbre cohérent à
    /// frontière de token (R-1) — et à la cadence
    /// configurée (N octets ou N ms) le DOM en construction
    /// est livré à `paint`. Aucun `&mut Session` ne
    /// traverse l'`await` : le verrou hôte reste libre
    /// pendant tout le téléchargement (étape 53d).
    async fn run<F>(&mut self, mut paint: F) -> Result<(NavDone, NavTail), CoreError>
    where
        F: for<'a> FnMut(&'a Dom) -> PaintFut<'a>,
    {
        let mut resp = self
            .transport
            .fetch_stream(&self.req)
            .await
            .map_err(|e| CoreError::Fetch(e.to_string()))?;
        if !(200..300).contains(&resp.status) {
            return Err(CoreError::HttpStatus(resp.status));
        }
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
            // Fenêtre de sniffing du décodeur ou morceau sans
            // texte décodable : le DOM n'a pas bougé — peindre
            // serait un no-op (et `since_paint` reste accumulé
            // pour déclencher dès le premier texte réel, 56c).
            if !text.is_empty()
                && (since_paint >= self.paint_chunk_bytes
                    || last_paint.elapsed() >= self.paint_interval)
            {
                paint(tb.dom()).await;
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
        let done = NavDone {
            url: self.req.url.clone(),
            dom,
            html,
            title,
        };
        let tail = NavTail {
            epoch: self.epoch,
            push: self.push,
            prev_title: self.prev_title.take(),
            req: self.req.clone(),
        };
        Ok((done, tail))
    }
}

/// Évaluation d'un module différée au commit (étape 53d) —
/// le graphe statique est déjà rempli (prefetch hors
/// session dans [`SubJob::run`]), le commit ne fait plus
/// que l'`eval_module` synchrone.
struct ModEval {
    /// Clé de registre moteur (URL de l'entrée ou
    /// `inline:N`).
    name: String,
    /// Graphe rempli — `sources` + `alias`.
    graph: ModuleGraph,
}

/// Phase « sous-ressources » d'une navigation, détachable
/// de la session (étape 53d) : possède le plan de collecte,
/// le moteur JS du document et les tampons de réception —
/// [`SubJob::run`] télécharge la vague CSS/scripts/images,
/// les `@font-face` et les graphes de modules sans
/// `&Session` ; [`Session::sub_commit`] applique.
struct SubJob {
    /// Onglet + epoch de la navigation.
    tab: TabId,
    epoch: u64,
    /// URL du document — base des sous-ressources et
    /// `Referer` de la politique réseau.
    url: Url,
    /// `push` historique + titre avant navigation.
    push: bool,
    prev_title: Option<String>,
    /// Requête d'origine — `last_req` du commit et entrée
    /// d'historique poussée (étape 56, rejoue un POST).
    req: Request,
    /// Titre extrait au parse.
    title: Option<String>,
    /// Transport cloné — fetch hors session.
    transport: Arc<dyn HttpTransport>,
    /// Bus d'événements — `PipelineError` des évaluations
    /// « arrival » pendant la vague.
    events: broadcast::Sender<CoreEvent>,
    /// URL de la page — politique mixed-content/`Referer`
    /// du prefetch modules.
    page: Url,
    /// Plan de collecte — URLs résolues à `nav_apply`.
    sheet_urls: Vec<Url>,
    scripts: Vec<ScriptItem>,
    img_urls: Vec<Url>,
    /// CSS inline du document — nourrit la résolution
    /// `@font-face` (le DOM n'est pas accessible hors
    /// session).
    doc_css: String,
    /// Moteur du document — les `async` classiques sont
    /// évalués en ligne à réception pendant la vague.
    eng: Option<Box<dyn JsEngine>>,
    /// Réceptions de la vague — indexées par ordre document.
    css_slots: Vec<Option<String>>,
    script_srcs: Vec<Option<String>>,
    img_bodies: HashMap<String, Vec<u8>>,
    /// Corps des modules `async` arrivés (ordre d'arrivée).
    async_mods: Vec<(Url, String)>,
    /// Faces `@font-face` téléchargées et décodées —
    /// `family`, `weight`, `italic`, données brutes.
    fonts: Vec<(String, u16, bool, Vec<u8>)>,
    /// Modules à évaluer au commit — arrival puis post.
    arrival_evals: Vec<ModEval>,
    post_evals: Vec<ModEval>,
    /// Bornes recopiées de la config session.
    max_inflight: usize,
    max_modules: usize,
    font_budget: usize,
}

impl SubJob {
    /// Vague de sous-ressources + fontes + prefetch des
    /// graphes de modules — intégralement hors session :
    /// tous les tampons vivent dans le job, le commit les
    /// applique (étape 53d). Les scripts `async` classiques
    /// sont évalués à réception sur le moteur embarqué.
    async fn run(&mut self) {
        // Slots de réception indexés par ordre document ;
        // `slot_items[slot]` = index de l'item script.
        self.css_slots = vec![None; self.sheet_urls.len()];
        let slot_items: Vec<usize> = self
            .scripts
            .iter()
            .enumerate()
            .filter_map(|(i, s)| s.slot.map(|_| i))
            .collect();
        self.script_srcs = vec![None; slot_items.len()];

        let tab = self.tab;
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
        let jobs = sub_jobs(&self.url, &self.sheet_urls, &self.scripts, &self.img_urls);
        let mut eng = self.eng.take();
        let mut async_mods = Vec::new();
        fetch_wave(&self.transport, self.max_inflight, jobs, |tag, res| {
            let Ok(resp) = res else { return };
            if !(200..300).contains(&resp.status) {
                return;
            }
            match tag {
                SubTag::Css(i) => {
                    self.css_slots[i] = Some(decode_stylesheet(&resp.body, &resp.headers));
                }
                SubTag::Script(slot) => {
                    let item = &self.scripts[slot_items[slot]];
                    // Étape 53b : charset déclaré
                    // (classique) → BOM → UTF-8.
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
                        self.script_srcs[slot] = Some(text);
                    }
                }
                SubTag::Img(u) => {
                    self.img_bodies.insert(u.as_str(), resp.body);
                }
            }
        })
        .await;
        self.eng = eng;
        self.async_mods = async_mods;
        self.fetch_fonts().await;
        self.prefetch_modules().await;
    }

    /// Fetch + décode des `@font-face` hors session (étape
    /// 53d — ex-`fetch_webfonts` déplacé) : faces résolues
    /// sur `doc_css` + feuilles liées téléchargées, chaque
    /// source `url()` essayée dans l'ordre (première qui
    /// décode gagne) ; `register` au commit borne le
    /// registre. Séquentiel — `font_budget` vus.
    async fn fetch_fonts(&mut self) {
        let mut eng = StyleEngine::new();
        eng.add_css(&self.doc_css.clone(), Origin::Author);
        let mut linked = String::new();
        for t in self.css_slots.iter().flatten() {
            linked.push_str(t);
            linked.push('\n');
        }
        eng.add_css(&linked, Origin::Author);
        'faces: for face in eng.font_faces(&StyleEnv::default()) {
            if self.fonts.len() >= self.font_budget {
                break;
            }
            for src in &face.srcs {
                let FontSrc::Url(href) = src else {
                    continue;
                };
                let Ok(url) = Url::resolve(&self.url, href) else {
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
                self.fonts
                    .push((face.family.clone(), face.weight, face.italic, data));
                continue 'faces;
            }
        }
    }

    /// Prefetch des graphes de modules hors session (étape
    /// 53d — ex-`eval_module_src` splitté) : même ordre que
    /// la file d'exécution du commit — `async` en ordre
    /// d'arrivée puis post-parse en ordre document.
    async fn prefetch_modules(&mut self) {
        let mut entries: Vec<(String, Url, String)> = Vec::new();
        for (u, src) in std::mem::take(&mut self.async_mods) {
            entries.push((u.as_str(), u, src));
        }
        let async_n = entries.len();
        for (i, item) in self.scripts.iter().enumerate() {
            if item.exec != ScriptExec::Post || !item.module {
                continue;
            }
            let Some(src) = item_source(item, &self.script_srcs) else {
                continue;
            };
            // Entrée inline : nom synthétique unique, aliasé
            // vers la page pour la résolution des deps.
            let (name, base_u) = item.src.clone().map_or_else(
                || (format!("inline:{i}"), self.url.clone()),
                |u| (u.as_str(), u),
            );
            entries.push((name, base_u, src.to_string()));
        }
        let mut post_evals = Vec::new();
        for (n, (name, base, src)) in entries.into_iter().enumerate() {
            let mut graph = ModuleGraph {
                sources: HashMap::from([(name.clone(), src.clone())]),
                alias: HashMap::from([(name.clone(), base.as_str())]),
            };
            Session::fetch_module_sources(
                &self.transport,
                self.max_inflight,
                self.max_modules,
                &self.page,
                &base,
                &src,
                &mut graph,
            )
            .await;
            let me = ModEval { name, graph };
            if n < async_n {
                self.arrival_evals.push(me);
            } else {
                post_evals.push(me);
            }
        }
        self.post_evals = post_evals;
    }
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

/// Bouton par défaut du formulaire (HTML §4.10.22.2) : le
/// premier contrôle submit en ordre d'arbre —
/// `input[type=submit|image]` ou `<button>` non `disabled`
/// (type absent ≙ `submit`).
fn default_button(dom: &Dom, form: NodeId) -> Option<NodeId> {
    let mut queue = vec![form];
    while let Some(n) = queue.pop() {
        let tag = dom.tag_name(n).unwrap_or("");
        if n != form
            && dom.element(n).is_ok()
            && dom.attribute(n, "disabled").ok().flatten().is_none()
        {
            let is_submit = match tag {
                "input" => matches!(attr_lower(dom, n, "type").as_str(), "submit" | "image"),
                "button" => !matches!(attr_lower(dom, n, "type").as_str(), "button" | "reset"),
                _ => false,
            };
            if is_submit {
                return Some(n);
            }
        }
        if n == form || tag != "form" {
            if let Ok(ch) = dom.children(n) {
                queue.extend(ch.iter().rev());
            }
        }
    }
    None
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

/// Repli navigateur pour les fragments (étape 56d) :
/// `<a name="x">` quand aucun `id="x"` n'existe —
/// itératif, même parcours que `find_by_id`.
fn find_named_anchor(dom: &Dom, name: &str) -> Option<NodeId> {
    let mut queue = vec![dom.document()];
    while let Some(n) = queue.pop() {
        if dom.tag_name(n).ok().is_some_and(|t| t == "a")
            && dom.attribute(n, "name").ok().flatten().as_deref() == Some(name)
        {
            return Some(n);
        }
        if let Ok(ch) = dom.children(n) {
            queue.extend(ch.iter().rev());
        }
    }
    None
}

/// Hauteur `y` (haut) de la première boîte posée portant
/// `target` — `min` sur toutes les boîtes du nœud (un
/// élément peut en couvrir plusieurs — itératif, R-24).
fn box_y_of_node(root: &LayoutBox, target: NodeId) -> Option<f32> {
    let mut stack = vec![root];
    let mut best: Option<f32> = None;
    while let Some(bx) = stack.pop() {
        if bx.node == Some(target) {
            best = Some(best.map_or(bx.rect.y, |acc| acc.min(bx.rect.y)));
        }
        stack.extend(bx.children.iter());
    }
    best
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

/// Rectangle du popup ouvert (coords document) : sous le
/// contrôle, basculé au-dessus quand il déborderait la page.
fn popup_rect(pop: &SelectPopup, vw: f32, doc_h: f32) -> Rect {
    // ≤ `MAX_POPUP_ROWS` lignes — la conversion est exacte.
    #[allow(clippy::cast_precision_loss)]
    let rows = pop.options.len().min(MAX_POPUP_ROWS) as f32;
    let w = pop.rect.width.max(140.0).min(vw.max(1.0));
    let x = pop.rect.x.clamp(0.0, (vw - w).max(0.0));
    let h = pop.row_h * rows;
    let below = pop.rect.y + pop.rect.height;
    let y = if below + h <= doc_h {
        below
    } else {
        (pop.rect.y - h).max(0.0)
    };
    Rect {
        x,
        y,
        width: w,
        height: h,
    }
}

/// Ligne du popup sous `(x, doc_y)` — `None` hors du
/// rectangle du popup.
fn popup_row(pop: &SelectPopup, x: f32, doc_y: f32, vw: f32, doc_h: f32) -> Option<usize> {
    let r = popup_rect(pop, vw, doc_h);
    if !point_in(&r, x, doc_y) {
        return None;
    }
    // `point_in` garantit `doc_y - r.y ∈ [0, height]` — la
    // troncature est le numéro de ligne attendu.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let idx = ((doc_y - r.y) / pop.row_h) as usize;
    (idx < pop.options.len().min(MAX_POPUP_ROWS)).then_some(idx)
}

/// Point `(x, y)` dans `rect` (bornes incluses).
fn point_in(r: &Rect, x: f32, y: f32) -> bool {
    x >= r.x && x <= r.x + r.width && y >= r.y && y <= r.y + r.height
}

/// Boîte posée de `node` — descente récursive de l'arbre de
/// layout (chaque `LayoutBox` garde son `NodeId` source).
fn box_rect(bx: &LayoutBox, node: NodeId) -> Option<Rect> {
    if bx.node == Some(node) {
        return Some(bx.rect);
    }
    bx.children.iter().find_map(|c| box_rect(c, node))
}

/// Options du `<select>` : `selected` sur l'index choisi,
/// retiré des autres (radio-like).
fn choose_option(dom: &mut Dom, sel: NodeId, idx: usize) {
    for (i, o) in descendant_elements(dom, sel, "option")
        .into_iter()
        .enumerate()
    {
        if i == idx {
            let _ = dom.set_attribute(o, "selected", "");
        } else {
            let _ = dom.remove_attribute(o, "selected");
        }
    }
}

/// Overlay du popup `<select>` en fin de display list —
/// fond opaque + bordure, lignes `Placeholder` (étiquette
/// rendue), ligne sélectionnée ou survolée teintée.
fn append_select_popup(list: &mut DisplayList, pop: &SelectPopup, vw: f32, doc_h: f32) {
    if pop.options.is_empty() {
        return;
    }
    let rect = popup_rect(pop, vw, doc_h);
    let white = Color {
        r: 255,
        g: 255,
        b: 255,
        a: 255,
    };
    let edge = Color {
        r: 118,
        g: 118,
        b: 118,
        a: 255,
    };
    let sel_bg = Color {
        r: 30,
        g: 106,
        b: 219,
        a: 255,
    };
    let hov_bg = Color {
        r: 222,
        g: 235,
        b: 255,
        a: 255,
    };
    list.items
        .push(DisplayItem::FillRect { rect, color: white });
    list.items.push(DisplayItem::Border {
        rect,
        widths: [1.0; 4],
        colors: [edge; 4],
        styles: [BorderStyle::Solid; 4],
    });
    for (i, (label, selected)) in pop.options.iter().take(MAX_POPUP_ROWS).enumerate() {
        // `i < MAX_POPUP_ROWS` — conversion exacte.
        #[allow(clippy::cast_precision_loss)]
        let row = Rect {
            x: rect.x,
            y: rect.y + i as f32 * pop.row_h,
            width: rect.width,
            height: pop.row_h,
        };
        if *selected {
            list.items.push(DisplayItem::FillRect {
                rect: row,
                color: sel_bg,
            });
        } else if pop.hovered == Some(i) {
            list.items.push(DisplayItem::FillRect {
                rect: row,
                color: hov_bg,
            });
        }
        list.items.push(DisplayItem::Placeholder {
            rect: row,
            label: label.clone(),
        });
    }
}

/// Attribut interne transportant `setCustomValidity()` du
/// miroir JS vers le DOM réel via la synchro : le miroir le
/// pose en `data-explorer-cv` (visible dans `outerHTML` —
/// déviation documentée ADR-0014), le moteur le lit ici.
const CUSTOM_VALIDITY_ATTR: &str = "data-explorer-cv";

/// Contrôles du formulaire qui échouent la validation de
/// contraintes (étape 54 — HTML §4.10.21.2) : `required`,
/// `minlength`, `maxlength`, `pattern`, `min`/`max`/`step`
/// (types numériques + `date`-like MVP = non validés,
/// déviation) et la validité custom posée par
/// `setCustomValidity`. Les non-candidats (`disabled`,
/// `readonly` sur un champ non textuel, boutons, `type=hidden`)
/// ne valident jamais.
fn invalid_controls(dom: &Dom, form: NodeId) -> Vec<NodeId> {
    let mut out = Vec::new();
    let mut queue = vec![form];
    while let Some(n) = queue.pop() {
        let tag = dom.tag_name(n).unwrap_or("");
        if n != form
            && dom.element(n).is_ok()
            && is_control_candidate(dom, n, tag)
            && control_invalid_reason(dom, n, tag).is_some()
        {
            out.push(n);
        }
        if n == form || tag != "form" {
            if let Ok(ch) = dom.children(n) {
                queue.extend(ch.iter().rev());
            }
        }
    }
    out
}

/// `willValidate` du contrôle : `disabled`, boutons et
/// `type=hidden`/`submit`-like exclus (HTML §4.10.21.2 —
/// `barred from constraint validation`).
fn is_control_candidate(dom: &Dom, n: NodeId, tag: &str) -> bool {
    if dom.attribute(n, "disabled").ok().flatten().is_some() {
        return false;
    }
    match tag {
        "input" => {
            let ty = attr_lower(dom, n, "type");
            !matches!(
                ty.as_str(),
                "hidden" | "submit" | "image" | "reset" | "button" | "file"
            )
        }
        "textarea" | "select" => true,
        _ => false,
    }
}

/// Première raison d'invalidité du contrôle — `None` si
/// valide. L'ordre suit HTML : custom, required, longueurs,
/// pattern, bornes numériques, pas (`step`).
fn control_invalid_reason(dom: &Dom, n: NodeId, tag: &str) -> Option<&'static str> {
    if dom
        .attribute(n, CUSTOM_VALIDITY_ATTR)
        .ok()
        .flatten()
        .is_some_and(|m| !m.is_empty())
    {
        return Some("customError");
    }
    let (value, empty_required) = match tag {
        "input" => {
            let ty = attr_lower(dom, n, "type");
            match ty.as_str() {
                "checkbox" => (
                    String::new(),
                    dom.attribute(n, "checked").ok().flatten().is_none(),
                ),
                "radio" => (String::new(), !radio_group_checked(dom, n)),
                _ => (
                    input_value(dom, n, &ty),
                    input_value(dom, n, &ty).is_empty(),
                ),
            }
        }
        "textarea" => {
            let v = dom.text_content(n).unwrap_or_default();
            let e = v.is_empty();
            (v, e)
        }
        "select" => {
            // Invalide seulement si `required` et que
            // l'option choisie porte une valeur vide
            // (placeholder label option).
            let v = select_value(dom, n);
            let e = v.is_empty();
            (v, e)
        }
        _ => (String::new(), false),
    };
    if dom.attribute(n, "required").ok().flatten().is_some() && empty_required {
        return Some("valueMissing");
    }
    // Longueurs + pattern : champs textuels seulement —
    // `readonly` excepté pour required (HTML).
    let textual = tag == "textarea"
        || matches!(
            attr_lower(dom, n, "type").as_str(),
            "" | "text" | "search" | "tel" | "url" | "email" | "password"
        );
    if textual {
        let len = value.chars().count();
        if let Ok(min) = attr_lower(dom, n, "minlength").parse::<usize>() {
            if len < min && !value.is_empty() {
                return Some("tooShort");
            }
        }
        if let Ok(max) = attr_lower(dom, n, "maxlength").parse::<usize>() {
            if len > max {
                return Some("tooLong");
            }
        }
        if let Some(pat) = dom.attribute(n, "pattern").ok().flatten() {
            if !value.is_empty() && !pattern_full_match(&pat, &value) {
                return Some("patternMismatch");
            }
        }
    }
    // Bornes + pas numériques : `number`, `range` (toujours
    // borné), et les types à valeur `f64` parseable.
    if matches!(attr_lower(dom, n, "type").as_str(), "number" | "range") {
        if let Ok(v) = value.trim().parse::<f64>() {
            if let Some(min) = attr_opt(dom, n, "min").and_then(|m| m.parse::<f64>().ok()) {
                if v < min {
                    return Some("rangeUnderflow");
                }
            }
            if let Some(max) = attr_opt(dom, n, "max").and_then(|m| m.parse::<f64>().ok()) {
                if v > max {
                    return Some("rangeOverflow");
                }
            }
            // `step` : `(v - min) % step == 0`, base = `min`
            // sinon 0 (défaut `step=1`).
            let step = attr_opt(dom, n, "step").and_then(|s| s.parse::<f64>().ok());
            if let Some(step) = step.filter(|s| *s > 0.0) {
                let base = attr_opt(dom, n, "min")
                    .and_then(|m| m.parse::<f64>().ok())
                    .unwrap_or(0.0);
                let rem = ((v - base) / step).fract();
                if rem.abs() > 1e-9 && (rem.abs() - 1.0).abs() > 1e-9 {
                    return Some("stepMismatch");
                }
            }
        }
    }
    None
}

/// Un `<input type=radio>` du même groupe (même `name`,
/// même formulaire) est coché — `required` est satisfait
/// au niveau du groupe (HTML).
fn radio_group_checked(dom: &Dom, n: NodeId) -> bool {
    let Some(name) = attr_opt(dom, n, "name") else {
        return dom.attribute(n, "checked").ok().flatten().is_some();
    };
    // Ancêtre `<form>` le plus proche, sinon la racine du
    // document (un groupe hors formulaire reste groupé).
    let mut scope = n;
    while let Ok(Some(p)) = dom.parent(scope) {
        if dom.tag_name(p) == Ok("form") {
            scope = p;
            break;
        }
        scope = p;
    }
    descendant_elements(dom, scope, "input").iter().any(|&r| {
        attr_lower(dom, r, "type") == "radio"
            && attr_opt(dom, r, "name").as_deref() == Some(name.as_str())
            && dom.attribute(r, "checked").ok().flatten().is_some()
    })
}

/// `pattern` HTML : correspondance **entière** (la spec
/// enveloppe en `^(?:…)$`). Un pattern qui ne compile pas
/// ne contraint pas (HTML) — `regex` ne couvre pas tout le
/// dialecte JS (lookahead…) : déviation documentée.
fn pattern_full_match(pat: &str, value: &str) -> bool {
    regex::Regex::new(&format!("^(?:{pat})$")).is_ok_and(|re| re.is_match(value))
}

/// Attribut en minuscules, `""` si absent.
fn attr_lower(dom: &Dom, n: NodeId, attr: &str) -> String {
    attr_opt(dom, n, attr).unwrap_or_default().to_lowercase()
}

/// Attribut ou `None`.
fn attr_opt(dom: &Dom, n: NodeId, attr: &str) -> Option<String> {
    dom.attribute(n, attr).ok().flatten()
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
/// Champ sérialisable d'un formulaire (étape 54 —
/// `multipart/form-data` exige de distinguer les fichiers
/// des valeurs texte).
enum FormEntry {
    /// `(name, value)` — contrôle textuel.
    Text(String, String),
    /// `(name, filename, bytes)` — `input[type=file]` : le
    /// nom de fichier vient de l'attribut `value` (aucun
    /// sélecteur de fichier réel — déviation ADR-0014), un
    /// champ non choisi émet un fichier vide (HTML).
    File(String, String, Vec<u8>),
}

/// Champs « successful » du formulaire (HTML §4.10.21) :
/// nommés, non `disabled`, cases cochées seulement, boutons
/// uniquement le `submitter`. Déviations MVP : `form=`
/// externe, `<fieldset disabled>` et `<object>` non traités.
fn collect_form_data(dom: &Dom, form: NodeId, submitter: Option<NodeId>) -> Vec<FormEntry> {
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
                    let ty = attr_lower(dom, n, "type");
                    match ty.as_str() {
                        "checkbox" | "radio" => {
                            if dom.attribute(n, "checked").ok().flatten().is_some() {
                                push_named(dom, n, input_value(dom, n, &ty), &mut out);
                            }
                        }
                        "submit" | "image" | "button" | "reset" => {
                            if Some(n) == submitter {
                                push_named(dom, n, input_value(dom, n, &ty), &mut out);
                            }
                        }
                        "file" => {
                            if let Some(name) = attr_opt(dom, n, "name").filter(|s| !s.is_empty()) {
                                let filename = attr_opt(dom, n, "value").unwrap_or_default();
                                out.push(FormEntry::File(name, filename, Vec::new()));
                            }
                        }
                        _ => push_named(dom, n, input_value(dom, n, &ty), &mut out),
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

/// Pousse `Text(name, value)` si le contrôle est nommé.
fn push_named(dom: &Dom, n: NodeId, value: String, out: &mut Vec<FormEntry>) {
    if let Some(name) = attr_opt(dom, n, "name").filter(|s| !s.is_empty()) {
        out.push(FormEntry::Text(name, value));
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
/// Encodage `application/x-www-form-urlencoded` : espace→`+`,
/// non-réservés conservés, le reste en `%XX`. Un `File` y
/// porte son nom de fichier (HTML).
fn form_url_encode(entries: &[FormEntry]) -> String {
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
    entries
        .iter()
        .map(|e| {
            let (k, v) = match e {
                FormEntry::Text(k, v) => (k, v),
                FormEntry::File(k, f, _) => (k, f),
            };
            format!("{}={}", enc(k), enc(v))
        })
        .collect::<Vec<_>>()
        .join("&")
}

/// Corps `multipart/form-data` (RFC 7578 / HTML §4.10.21.6) :
/// chaque entrée est une partie `Content-Disposition:
/// form-data`, les fichiers portent `filename=` +
/// `Content-Type: application/octet-stream` (MIME réelle
/// indisponible sans picker — déviation documentée).
fn form_multipart_encode(entries: &[FormEntry], boundary: &str) -> Vec<u8> {
    fn esc(s: &str) -> String {
        s.replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace(['\r', '\n'], "")
    }
    let mut body = Vec::new();
    for e in entries {
        body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        match e {
            FormEntry::Text(name, value) => {
                body.extend_from_slice(
                    format!(
                        "Content-Disposition: form-data; name=\"{}\"\r\n\r\n",
                        esc(name)
                    )
                    .as_bytes(),
                );
                body.extend_from_slice(value.as_bytes());
            }
            FormEntry::File(name, filename, bytes) => {
                body.extend_from_slice(
                    format!(
                        "Content-Disposition: form-data; name=\"{}\"; filename=\"{}\"\r\n\
                         Content-Type: application/octet-stream\r\n\r\n",
                        esc(name),
                        esc(filename)
                    )
                    .as_bytes(),
                );
                body.extend_from_slice(bytes);
            }
        }
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    body
}

/// Requête de soumission (HTML §4.10.21.7) partagée entre
/// `submit_form` (DOM réel) et `NavRequest::FormSubmit`
/// (miroir) : `POST` selon `enctype` (`multipart/form-data`
/// ou `urlencoded` — `text/plain` retombe sur urlencoded,
/// déviation ADR-0014), sinon `GET` en query.
fn form_request(
    base: &Url,
    action: &str,
    method: &str,
    enctype: &str,
    entries: &[FormEntry],
) -> Option<Request> {
    if method.eq_ignore_ascii_case("post") {
        let url = Url::resolve(base, action).ok()?;
        let (content_type, body) = if enctype.eq_ignore_ascii_case("multipart/form-data") {
            let boundary = multipart_boundary();
            (
                format!("multipart/form-data; boundary={boundary}"),
                form_multipart_encode(entries, &boundary),
            )
        } else {
            (
                "application/x-www-form-urlencoded".to_string(),
                form_url_encode(entries).into_bytes(),
            )
        };
        return Some(Request {
            method: Method::Post,
            url,
            headers: vec![("content-type".to_string(), content_type)],
            body: Some(body),
        });
    }
    let qs = form_url_encode(entries);
    let target = if qs.is_empty() {
        action.to_string()
    } else {
        let sep = if action.contains('?') { '&' } else { '?' };
        format!("{action}{sep}{qs}")
    };
    Some(Request::get(Url::resolve(base, &target).ok()?))
}

/// Requête d'un `NavRequest::FormSubmit` — les champs sont
/// déjà collectés côté miroir en `(name, value)` plats
/// (les fichiers y portent leur nom de fichier).
fn script_form_request(
    base: &Url,
    method: &str,
    action: &str,
    enctype: &str,
    fields: &[(String, String)],
) -> Option<Request> {
    let entries: Vec<FormEntry> = fields
        .iter()
        .map(|(n, v)| FormEntry::Text(n.clone(), v.clone()))
        .collect();
    form_request(base, action, method, enctype, &entries)
}

/// Frontière multipart pseudo-unique : 96 bits d'horloge +
/// compteur de process — suffisant sans dépendance `rand`,
/// le contenu qui la contiendrait la verrait invalide (RFC
/// recommande de vérifier, MVP documenté).
fn multipart_boundary() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    format!("----ExplorerFormBoundary{nanos:032x}")
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
    /// `form.submit()`/`requestSubmit()` : requête déjà
    /// construite (méthode + corps + en-têtes — étape 54).
    Submit(Request),
}

/// Action dépilée par un drain de navigation (étape 53d) —
/// requête prête (input `pending_nav`, `location`, submit
/// de formulaire) ou déplacement relatif d'historique.
enum DrainAct {
    /// `(requête, push historique)` — conduite par
    /// `navigate_request`/`navigate_shared`. `Box` :
    /// `Request` (233 o) domine `Go` (8 o) — la file
    /// n'embarque pas le pire cas par cellule.
    Req(Box<Request>, bool),
    /// `history.go(d)` — résolu via `history_move` (ou son
    /// équivalent partagé) ; `0` = reload.
    Go(isize),
}

// ═══════════════ Étape 53d — drivers partagés ═══════════════

/// Conduit une navigation sur une session partagée
/// (`Mutex<Session>`) **sans tenir le verrou pendant
/// l'I/O** (étape 53d — le défaut produit était un gel de
/// toute l'API pendant `navigate`) : `nav_begin` arme
/// l'epoch sous verrou court, `NavJob::run` télécharge et
/// parse hors verrou (chaque snapshot de peinture
/// progressive est appliqué dans un re-lock minimal),
/// `nav_apply` puis `sub_commit` n'écrivent qu'en verrou
/// court. Pendant tout le parcours, `status`, `tabs`,
/// `frame`, l'input et les autres onglets restent
/// serviables.
///
/// Une navigation supplantée (epoch périmée — double-clic,
/// redirection script plus récente) est abandonnée
/// silencieusement : ses fetches ont tourné mais rien
/// d'obsolète n'est commis.
///
/// Ne draine pas les intentions produites : l'appelant
/// enchaîne [`drain_navs_shared`] — même contrat que
/// [`Session::navigate`] + [`Session::drain_script_nav`].
///
/// # Errors
/// Mêmes erreurs que [`Session::navigate`]
/// (`Url`, `Fetch`, `HttpStatus`, `EmptyDocument`,
/// `NothingToPaint`, `FrameTooBig`, `UnknownTab`) —
/// `PipelineError` est émis sur le bus pour les erreurs de
/// pipeline, comme le chemin `&mut self`.
pub async fn navigate_shared(
    sess: &tokio::sync::Mutex<Session>,
    tab: TabId,
    req: Request,
    push: bool,
) -> Result<(), CoreError> {
    let mut job = {
        let mut s = sess.lock().await;
        // `navigate` vers l'entrée forward = restauration
        // bfcache (56c) — jamais une peinture non commise.
        if push && s.try_forward_nav(tab, &req) {
            return Ok(());
        }
        // `#fragment` même document = scroll, pas re-fetch
        // (56d).
        if s.try_fragment_nav(tab, &req) {
            return Ok(());
        }
        s.nav_begin(tab, req, push)?
    };
    let epoch = job.epoch;
    let (done, tail) = match job
        .run(|dom: &Dom| -> PaintFut<'_> {
            // `try_lock` : la peinture progressive ne bloque
            // jamais le téléchargement — un tick manqué (lock
            // occupé par un `/frame` ou une autre phase) est
            // repeint au suivant, le commit final repeint de
            // toute façon.
            if let Ok(mut s) = sess.try_lock() {
                s.nav_paint(tab, epoch, dom);
            }
            Box::pin(async {})
        })
        .await
    {
        Ok(out) => out,
        Err(e) => {
            let mut s = sess.lock().await;
            s.unspec(tab); // peinture spéculative → réverte (56c)
            return Err(s.emit_error(tab, e));
        }
    };
    let sub = {
        let mut s = sess.lock().await;
        match s.nav_apply(tab, tail, done) {
            Ok(sub) => sub,
            Err(e) => return Err(s.emit_error(tab, e)),
        }
    };
    // Epoch supplantée : la navigation la plus récente a
    // gagné — celle-ci est jetée proprement (rien à
    // commettre, aucune erreur n'est observable).
    let Some(mut sub) = sub else {
        return Ok(());
    };
    sub.run().await;
    let mut s = sess.lock().await;
    match s.sub_commit(tab, sub).await {
        Ok(_) => Ok(()),
        Err(e) => Err(s.emit_error(tab, e)),
    }
}

/// `history_move` partagé (étape 53d) : la cible est lue
/// sous verrou court, la navigation conduite hors verrou,
/// l'index commis seulement si la navigation a abouti.
///
/// # Errors
/// [`CoreError::UnknownTab`], [`CoreError::EmptyDocument`]
/// (pas d'entrée dans cette direction) ou erreur de
/// navigation.
pub async fn history_move_shared(
    sess: &tokio::sync::Mutex<Session>,
    tab: TabId,
    delta: isize,
) -> Result<(), CoreError> {
    let (idx, req) = {
        let mut s = sess.lock().await;
        let t = s.hist_target(tab, delta)?;
        // Hit bfcache (étape 56) : restauration instantanée
        // sous verrou court — aucune navigation n'est lancée.
        if s.try_bfcache(tab, t.0) {
            return Ok(());
        }
        t
    };
    // Miss → la requête d'origine est rejouée (POST rejoué
    // tel quel — étape 56).
    match navigate_shared(sess, tab, req, false).await {
        Ok(()) => {
            let mut s = sess.lock().await;
            s.set_hist_idx(tab, idx);
            Ok(())
        }
        Err(e) => {
            let mut s = sess.lock().await;
            Err(s.emit_error(tab, e))
        }
    }
}

/// Drain partagé des intentions de navigation (étape 53d) —
/// même contrat que [`Session::drain_script_nav`] mais
/// chaque navigation est conduite par [`navigate_shared`]
/// **hors verrou** : la boucle bornée `MAX_NAV_HOPS`
/// alterne collecte sous verrou court et conduite libre —
/// `form.submit()`, `location=` et les clics de liens n'en
/// gèlent plus l'API non plus. `true` si au moins une
/// navigation a été appliquée.
pub async fn drain_navs_shared(sess: &tokio::sync::Mutex<Session>, tab: TabId) -> bool {
    const MAX_NAV_HOPS: u8 = 8;
    let mut applied = false;
    for _ in 0..MAX_NAV_HOPS {
        let act = {
            let mut s = sess.lock().await;
            s.collect_script_navs(tab);
            s.pop_pending_nav(tab)
        };
        match act {
            None => break,
            Some(DrainAct::Req(req, push)) => {
                let _ = navigate_shared(sess, tab, *req, push).await;
            }
            Some(DrainAct::Go(0)) => {
                // `history.go(0)` = rechargement de l'entrée
                // courante (spec HTML).
                let url = {
                    let s = sess.lock().await;
                    s.tabs.get(&tab).and_then(|t| t.url.clone())
                };
                if let Some(u) = url {
                    let _ = navigate_shared(sess, tab, Request::get(u), false).await;
                }
            }
            Some(DrainAct::Go(d)) => {
                let _ = history_move_shared(sess, tab, d).await;
            }
        }
        applied = true;
    }
    applied
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

/// Re-mappe les `NodeId` des boîtes conservées **avant** le
/// relayout incrémental (étape 55f) : l'arbre porte des ids
/// de l'**ancien** DOM — chacun est re-résolu dans le
/// nouveau par son chemin d'élément (les non-éléments via
/// l'index dans les enfants du parent, inchangé hors
/// sous-arbre). La racine du sous-arbre sale (`target_old`)
/// est traduite en `target_new` et sa descendance n'est pas
/// visitée — elle sera reconstruite par `relayout_subtree`.
/// `false` = un id irrésolvable → l'appelant fait un reflow
/// global (l'arbre mixte est alors jeté, jamais servi).
fn remap_box_nodes(
    old: &Dom,
    new: &Dom,
    root: &mut LayoutBox,
    target_old: NodeId,
    target_new: NodeId,
) -> bool {
    // Itératif — le parcours suit la profondeur du document
    // (R-24 : ~300 <div> imbriqués réels, pas de récursion).
    let mut stack = vec![root];
    while let Some(bx) = stack.pop() {
        if bx.node == Some(target_old) {
            bx.node = Some(target_new);
            continue;
        }
        if let Some(id) = bx.node {
            let Some(fresh) = remap_node(old, new, id) else {
                return false;
            };
            bx.node = Some(fresh);
        }
        stack.extend(bx.children.iter_mut());
    }
    true
}

/// `id` (ancien DOM) → `NodeId` équivalent dans `new` :
/// éléments par `element_path`, non-éléments par index dans
/// les enfants du parent (inchangé hors sous-arbre sale).
fn remap_node(old: &Dom, new: &Dom, id: NodeId) -> Option<NodeId> {
    if old.element(id).is_ok() {
        return dom_path_node(new, &element_path(old, id)?);
    }
    let parent = old.parent(id).ok()??;
    let new_parent = if old.element(parent).is_ok() {
        dom_path_node(new, &element_path(old, parent)?)?
    } else {
        new.document()
    };
    let sibs = old.children(parent).ok()?;
    let idx = sibs.iter().position(|&c| c == id)?;
    let cand = new.children(new_parent).ok()?.get(idx).copied()?;
    // Garde structurelle : élément ↔ élément (même tag),
    // texte/commentaire ↔ non-élément.
    let ok = match (old.element(id).is_ok(), new.element(cand).is_ok()) {
        (true, true) => old.tag_name(id).ok() == new.tag_name(cand).ok(),
        (false, false) => true,
        _ => false,
    };
    ok.then_some(cand)
}
#[cfg(test)]
mod tests {
    use super::*;

    /// Fonte fixture partagée (TTF synthétique, métriques
    /// réelles) — les tests internes peignent pour de vrai.
    struct TestFont;

    impl FontSource for TestFont {
        fn font_data(&self, _req: &explorer_layout::font::FontRequest) -> Option<Vec<u8>> {
            Some(explorer_test_support::fixture_ttf())
        }
    }

    /// Transport stub — jamais appelé par ces tests
    /// (restauration bfcache = zéro réseau).
    struct FailTransport;

    #[async_trait::async_trait]
    impl HttpTransport for FailTransport {
        async fn fetch(&self, _req: &Request) -> NetResult<Response> {
            Err(explorer_net::error::NetError::InvalidResponse(
                "stub : pas de réseau".to_string(),
            ))
        }
    }

    /// DOM minimal parsé par le vrai tree-builder.
    fn test_dom(body: &str) -> Dom {
        let mut tb = TreeBuilder::new();
        tb.push(&format!("<html><body>{body}</body></html>"));
        tb.finish();
        tb.into_dom()
    }

    /// Étape 56b — un snapshot gelé pendant la fenêtre
    /// `nav_apply` → `sub_commit` (DOM commis, produit
    /// jamais peint — `/frame` 204 persistant, hit-test
    /// mort, scroll bloqué, terrain mission 11) :
    /// (a) `freeze` refuse désormais une page sans produit
    /// peint — plus de snapshots empoisonnés ;
    /// (b) la restauration d'un snapshot sans produit
    /// **repeint** depuis son DOM plutôt que de servir un
    /// cadre vide à vie.
    #[test]
    fn paintless_snapshot_repairs_on_restore() {
        let mut s = Session::new(Arc::new(FailTransport), Arc::new(TestFont));
        let tab = s.open_tab();
        let url_a = Url::parse("http://h.test/a").unwrap();
        let url_b = Url::parse("http://h.test/b").unwrap();

        // (a) Une page commise mais jamais peinte ne gèle pas.
        {
            let e = s.tabs.get_mut(&tab).unwrap();
            e.dom = Some(test_dom("<p>en vol</p>"));
        }
        s.tabs.get_mut(&tab).unwrap().freeze(4);
        assert!(
            s.tabs.get(&tab).unwrap().bfcache.is_empty(),
            "une page sans produit peint ne doit pas être gelée (56b)"
        );

        // Live = A « peinte » (png factice — l'accesseur ne
        // valide pas le contenu) ; snapshot B gelé sans
        // produit : le poison du bug terrain.
        {
            let e = s.tabs.get_mut(&tab).unwrap();
            e.url = Some(url_a.clone());
            e.dom = Some(test_dom("<p>page A</p>"));
            e.png = Some(b"png-a".to_vec());
            e.hist = vec![Request::get(url_a), Request::get(url_b.clone())];
            e.hist_idx = 0;
            e.bfcache.push_back(PageSnapshot {
                hist_idx: 1,
                url: Some(url_b),
                title: None,
                scroll_y: 0.0,
                doc_h: 0.0,
                dom: Some(test_dom("<p>page B gelée tôt</p>")),
                tree: None,
                list: None,
                frame: None,
                png: None,
                focused: None,
                hovered: None,
                popup: None,
                js: None,
                images: PaintImages::default(),
                linked_css: String::new(),
                prev_styles: HashMap::new(),
                dirty: Vec::new(),
                emitted: 0,
                last_req: None,
            });
        }

        // (b) Restauration : le snapshot sans produit est
        // repeint depuis son DOM — fin du 204 persistant.
        assert!(s.try_bfcache(tab, 1));
        assert_eq!(s.tab_url(tab).as_deref(), Some("http://h.test/b"));
        assert!(
            s.frame_png(tab).is_some(),
            "un snapshot sans produit doit être repeint à la restauration (56b)"
        );
        assert!(
            s.tabs.get(&tab).unwrap().tree.is_some(),
            "l'arbre repeint restaure aussi le hit-test et le scroll"
        );
    }

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
