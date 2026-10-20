// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Types de contrat publics de la session : événements, entrées,
//! erreurs, DTO du store de profil, configuration.
use std::collections::BTreeMap;

use explorer_net::cookies::{Cookie, SameSite};

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
    /// Réponse en navigation top-level non affichable
    /// (étape 70) : `Content-Disposition: attachment`,
    /// `application/octet-stream` ou type inconnu — le corps
    /// est bufferisé en session, l'UI propose l'enregistrement
    /// (`accept_download`) ou le rejet (`decline_download`
    /// → rejeu comme document, dégradation historique).
    DownloadOffered {
        /// Onglet de la navigation.
        tab: TabId,
        /// Identifiant du téléchargement en attente.
        id: u64,
        /// URL servie (post-redirection).
        url: String,
        /// Nom de fichier suggéré (sanitisé).
        filename: String,
        /// Type MIME déclaré (sans paramètres).
        mime: String,
        /// Taille annoncée (`Content-Length`) si connue.
        size: Option<u64>,
    },
    /// Progression de la réception d'un téléchargement —
    /// émis par morceau reçu pendant le buffer.
    DownloadProgress {
        /// Onglet de la navigation.
        tab: TabId,
        /// Identifiant du téléchargement.
        id: u64,
        /// Octets reçus à date.
        received: u64,
        /// Taille annoncée si connue.
        total: Option<u64>,
    },
    /// Téléchargement accepté et écrit sur disque.
    DownloadDone {
        /// Onglet de la navigation.
        tab: TabId,
        /// Identifiant du téléchargement.
        id: u64,
        /// Chemin du fichier écrit.
        path: String,
    },
}

/// Fiche publique d'un téléchargement en attente (étape 70)
/// — exposée par `Session::pending_downloads` et l'API ;
/// le corps reste interne (`PendingDownload`).
#[derive(Debug, Clone)]
pub struct DownloadInfo {
    /// Identifiant du téléchargement.
    pub id: u64,
    /// Onglet de la navigation.
    pub tab: TabId,
    /// URL servie (post-redirection).
    pub url: String,
    /// Nom de fichier suggéré (sanitisé).
    pub filename: String,
    /// Type MIME déclaré.
    pub mime: String,
    /// Octets reçus.
    pub size: usize,
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
    /// Décalage de scroll (px CSS) — étape 61 : exposé pour
    /// les adaptateurs qui n'ont pas de canal d'état (FFI).
    pub scroll_y: f32,
    /// Hauteur **mesurée** du document posé (px CSS) —
    /// étape 65 : la frame raster est plafonnée
    /// (`MAX_FRAME_DIM`), le document peut dépasser ce qui
    /// est peint ; `doc_h` rapporte la hauteur réelle.
    pub doc_h: f32,
    /// `true` si la frame ne couvre pas `doc_h` en entier —
    /// le bas du document est hors raster (plafond
    /// `MAX_FRAME_DIM / scale` atteint). Sonder `doc_h`
    /// pour la hauteur réelle, jamais `frame.height`.
    pub doc_h_clamped: bool,
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
pub(crate) fn cookie_to_stored(c: &Cookie) -> StoredCookie {
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
pub(crate) fn stored_to_cookie(s: &StoredCookie) -> Cookie {
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

/// Port de persistance du profil — implémentation de
/// référence [`crate::JsonFileStore`] (fichiers dans le
/// répertoire de profil, étape 61). Injecté, jamais
/// d'I/O depuis `Session`.
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
    /// Téléchargement : identifiant inconnu, chemin
    /// refusé, écriture en échec (étape 70).
    #[error("téléchargement : {0}")]
    Download(String),
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
    /// Plafond de la fenêtre de coalescence des resyncs DOM
    /// pilotés par la pompe JS, en millisecondes (55g-suite) :
    /// après un `apply_dom_sync`, le prochain apply pompe
    /// attend `min(coût_apply, resync_gap_max)` au moins
    /// `paint_interval` — une page qui mute en continu
    /// (`setInterval`) laisse alors respirer l'API entre deux
    /// peintures au lieu de tenir le verrou en permanence.
    pub resync_gap_max_ms: u64,
    /// Borne du corps bufferisé d'un téléchargement en
    /// attente (étape 70) — un octet-stream hostile au-delà
    /// est rejeté en `Fetch` (page d'erreur, étape 69)
    /// plutôt que de gonfler la mémoire.
    pub max_download_bytes: usize,
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
            resync_gap_max_ms: 4_000,
            max_download_bytes: 64 * 1024 * 1024,
            bfcache_pages: 4,
        }
    }
}
