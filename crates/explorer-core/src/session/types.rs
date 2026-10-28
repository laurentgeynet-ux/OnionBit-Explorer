// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Types de contrat publics de la session : événements, entrées,
//! erreurs, DTO du store de profil, configuration.
use std::collections::BTreeMap;

use explorer_dom::Dom;
use explorer_net::cookies::{Cookie, SameSite};
use explorer_net::url::Url;

/// Identifiant d'onglet (monotone, jamais réutilisé).
pub type TabId = u64;

/// Signature textuelle de la mort du tas JS (§B-12) — les
/// erreurs la portant sont plafonnées à une par document,
/// quel que soit le chemin d'émission (évals session,
/// scripts `async` évalués sur le moteur détaché de la
/// vague de sous-ressources, livraisons réseau/WS/SSE).
pub(crate) const HEAP_DEAD_SIG: &str = "heap memory limit exceeded";

/// Événement de session poussé aux abonnés (SSE côté API).
#[derive(Debug, Clone)]
pub enum CoreEvent {
    /// Onglet créé.
    TabOpened {
        /// Nouvel onglet.
        tab: TabId,
    },
    /// Navigation engagée (début du fetch) — précède les
    /// évals de boot : le ring console purge ici (B-51),
    /// pas au commit `Navigated` qui effaçait les erreurs
    /// de la navigation en cours.
    Navigating {
        /// Onglet concerné.
        tab: TabId,
        /// URL demandée.
        url: String,
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
        /// Scroll vertical de la frame peinte (px CSS) —
        /// transport `MediaSurface` : l'hôte positionne ses
        /// overlays à `rect.y − scroll_y` (contrat 71, les
        /// rects média sont en espace document). Pas de
        /// `scroll_x` : le moteur ne scrolle que verticalement.
        scroll_y: f32,
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
    /// Un élément `<video>`/`<audio>` lisible est présent
    /// sur la page (étape 71b, ADR-0024) — l'hôte peut
    /// `claim` la surface pour y composite sa lecture ;
    /// sans claim elle reste un stub ADR-0023. `rect` est
    /// en **px CSS de document** (l'hôte soustrait
    /// `scroll_y` pour positionner son overlay) ; `audio`
    /// sans `controls` arrive à `rect` 0×0 — pipeline de
    /// lecture seul, rien à positionner.
    MediaOffered {
        /// Onglet de la page.
        tab: TabId,
        /// Identifiant de la surface — unique par onglet et
        /// par instance d'élément ; toute ré-offre (bfcache
        /// restauré, élément ré-inséré) produit un nouvel id.
        media: u64,
        /// `video` ou `audio`.
        kind: MediaKind,
        /// Rectangle de la boîte remplacée (px CSS document).
        rect: MediaRect,
        /// Source résolue (URL absolue).
        url: String,
        /// Type MIME — attribut `type` ou suffixe d'URL,
        /// `None` si inconnu.
        mime: Option<String>,
        /// L'hôte doit afficher son UI native.
        controls: bool,
        /// Lecture automatique demandée par la page.
        autoplay: bool,
    },
    /// Le `rect` d'une surface offerte a changé (relayout,
    /// resize). Coalescence à la cadence de peinture : au
    /// plus un `MediaMoved` par frame produite, le dernier
    /// rect connu gagne.
    MediaMoved {
        /// Onglet de la page.
        tab: TabId,
        /// Identifiant de la surface.
        media: u64,
        /// Nouveau rectangle (px CSS document).
        rect: MediaRect,
    },
    /// Fin de vie d'une surface : retrait DOM de l'élément,
    /// navigation, fermeture d'onglet. Un claim/state sur
    /// cet id renvoie ensuite `410`.
    MediaGone {
        /// Onglet de la page.
        tab: TabId,
        /// Identifiant de la surface morte.
        media: u64,
    },
    /// Ordre initié par la page JS (étape 71c) — relayé à
    /// l'hôte claimé ; jamais émis sur une surface non
    /// claimée.
    MediaCommand {
        /// Onglet de la page.
        tab: TabId,
        /// Identifiant de la surface.
        media: u64,
        /// Commande.
        cmd: MediaCmd,
    },
}

/// Genre d'une surface média offerte (étape 71b, ADR-0024).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKind {
    /// `<video>`.
    Video,
    /// `<audio>`.
    Audio,
}

/// Rectangle d'une offre média — **px CSS de document**
/// (l'hôte soustrait `scroll_y` pour son overlay ; contrat
/// `docs/interface-equipes.md` § `MediaSurface`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MediaRect {
    /// Abscisse (px).
    pub x: f32,
    /// Ordonnée (px).
    pub y: f32,
    /// Largeur (px).
    pub w: f32,
    /// Hauteur (px).
    pub h: f32,
}

/// Commande média émise par la page JS, relayée à l'hôte
/// claimé (contrat `MediaSurface`, étape 71b/c).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MediaCmd {
    /// `play()`.
    Play,
    /// `pause()`.
    Pause,
    /// `currentTime = …` (millisecondes).
    Seek {
        /// Position cible.
        ms: u64,
    },
    /// `volume = …` (0.0–1.0).
    Volume {
        /// Volume demandé.
        v: f64,
    },
    /// `muted = …` (étape 71c — distinct de `volume = 0` :
    /// le dé-mute restaure le volume courant).
    Muted {
        /// Muet demandé.
        on: bool,
    },
    /// `playbackRate = …`.
    Rate {
        /// Débit demandé.
        r: f64,
    },
}

/// État de lecture rapporté par l'hôte claimé (sens
/// hôte → moteur, contrat `MediaSurface`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaHostState {
    /// Lecture en cours — premier rapport = première frame
    /// (résolution de `play()`).
    Playing,
    /// En pause.
    Paused,
    /// Fin de flux atteinte.
    Ended,
    /// Erreur hôte — `MediaReport::error` porte la cause.
    Error,
}

/// Rapport `state` hôte → moteur : positions en
/// millisecondes, `None` duration = flux sans durée connue.
/// Battement borné ≤ 4 Hz (contrat `MediaSurface`).
#[derive(Debug, Clone, PartialEq)]
pub struct MediaReport {
    /// Tête de lecture (ms).
    pub position_ms: u64,
    /// Durée totale (ms) si connue.
    pub duration_ms: Option<u64>,
    /// Fin de la zone tampon (ms).
    pub buffered_ms: u64,
    /// État de lecture.
    pub state: MediaHostState,
    /// Cause si `state == Error`.
    pub error: Option<String>,
}

/// Fiche publique d'une surface média vivante (étape 71b)
/// — exposée par `Session::media_list` et l'API (debug,
/// parité FFI future).
#[derive(Debug, Clone)]
pub struct MediaOfferInfo {
    /// Identifiant de la surface.
    pub id: u64,
    /// Onglet de la page.
    pub tab: TabId,
    /// `video` ou `audio`.
    pub kind: MediaKind,
    /// Rectangle (px CSS document).
    pub rect: MediaRect,
    /// Source résolue (URL absolue).
    pub url: String,
    /// Type MIME si connu.
    pub mime: Option<String>,
    /// UI native attendue.
    pub controls: bool,
    /// Lecture automatique demandée.
    pub autoplay: bool,
    /// Codecs déclarés par l'hôte claimé, `None` = non
    /// claimée (la surface reste un stub ADR-0023).
    pub claim: Option<Vec<String>>,
    /// Dernier état rapporté par l'hôte.
    pub state: Option<MediaReport>,
}

/// Refus sur une surface média (étape 71b) — mappé `409`/
/// `410` par la couche REST.
#[derive(Debug, thiserror::Error, Clone, Copy, PartialEq, Eq)]
pub enum MediaError {
    /// Identifiant mort (élément retiré, navigation,
    /// onglet fermé) → `410 Gone`.
    #[error("surface média inexistante ou morte")]
    Dead,
    /// Déjà claimée — premier arrivé gagne → `409 Conflict`.
    #[error("surface média déjà claimée")]
    Taken,
    /// `state`/`release` exige un claim actif → `409`.
    #[error("surface média non claimée")]
    NotClaimed,
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
        /// Modifieurs tenus à la frappe (étape 105).
        mods: KeyMods,
    },
    /// Relâchement de touche — `keyup` dispatché (étape 52).
    KeyUp {
        /// Identifiant de touche.
        key: String,
        /// Modifieurs tenus au relâchement (étape 105).
        mods: KeyMods,
    },
}

/// État des modifieurs clavier au moment d'une frappe
/// (étape 105) — alimente `ctrlKey`/`shiftKey`/`altKey`/
/// `metaKey` sur le `KeyboardEvent` dispatché.
#[allow(clippy::struct_excessive_bools)] // forme DOM `KeyboardEventInit`
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct KeyMods {
    /// `Ctrl`.
    pub ctrl: bool,
    /// `Shift`.
    pub shift: bool,
    /// `Alt` (`AltGr` exclu — c'est du texte).
    pub alt: bool,
    /// `Meta` (⊞/⌘).
    pub meta: bool,
}

impl KeyMods {
    /// Aucun modifieur.
    pub const NONE: Self = Self {
        ctrl: false,
        shift: false,
        alt: false,
        meta: false,
    };

    /// `true` si un modifieur de raccourci (Ctrl/Alt/Meta) est
    /// tenu — l'édition par défaut est alors sautée : un accord
    /// appartient à l'app ou à la page, pas à la frappe (le
    /// `key` livré porte déjà la casse Shift).
    pub(crate) fn chord(self) -> bool {
        self.ctrl || self.alt || self.meta
    }

    /// Décode le bitmask FFI/REST (`1`=ctrl, `2`=shift,
    /// `4`=alt, `8`=meta).
    #[must_use]
    pub fn from_bits(bits: u8) -> Self {
        Self {
            ctrl: bits & 1 != 0,
            shift: bits & 2 != 0,
            alt: bits & 4 != 0,
            meta: bits & 8 != 0,
        }
    }
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
    /// Aucun champ focalisé à l'arrivée ou à l'issue de la
    /// frappe (perdu par `Enter`/`Escape`, nœud supprimé par
    /// un handler, jamais posé) — le client peut
    /// resynchroniser son drapeau « champ focalisé ».
    /// Distinct d'`Ignored` (un `keydown` consommé par JS
    /// laisse le champ focalisé).
    Unfocused,
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
    /// Clé de partition CHIPS (étape 127) — `"{scheme}://
    /// {registrable}"` du site top-level ; `None` = cookie
    /// non partitionné.
    pub partition: Option<String>,
}

/// Drapeaux `sandbox` d'un `<iframe>` (CAP-149) — `None` sur
/// [`SubFrame::sandbox`] = pas d'attribut = contexte plein
/// droit. Tout jeton absent de l'attribut = capacité **refusée**
/// (spec HTML : le défaut est tout restreint).
#[allow(clippy::struct_excessive_bools)] // chaque jeton `allow-*` EST un booléen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SandboxFlags {
    /// `allow-scripts` présent — sinon aucun script du cadre
    /// n'est évalué (le realm existe encore pour le miroir/
    /// `contentWindow`, jamais de `eval` ni d'événement utile).
    pub(crate) scripts: bool,
    /// `allow-same-origin` présent — sinon le contexte vit en
    /// **origine opaque** : `localStorage` isolé et jamais
    /// persisté, `document.cookie` vide et muet, `Origin: null`,
    /// `Sec-Fetch-Site: cross-site`, cookies `Lax`/`Strict`
    /// retenus sur ses requêtes script.
    pub(crate) same_origin: bool,
    /// `allow-popups` (borne : les cadres n'ouvrent pas de
    /// popup à cette tranche — jeton retenu pour la suite).
    pub(crate) popups: bool,
    /// `allow-forms` (borne : pas de soumission depuis un
    /// cadre à cette tranche).
    pub(crate) forms: bool,
    /// `allow-top-navigation` (borne : les cadres ne
    /// naviguent pas la page top-level à cette tranche).
    pub(crate) top_navigation: bool,
}

/// Contexte de navigation imbriqué d'un `<iframe>`
/// (étape 128 — socle) : le document enfant est fetché
/// sous l'initiateur de la page parente (`Subresource` —
/// `SameSite`/`CHIPS` corrects, `sec-fetch-dest: iframe`,
/// `sec-fetch-mode: navigate`) puis **parsé en son propre
/// `Dom`**. Le rendu composite dans la boîte remplacée et
/// les ponts JS (`window.parent`/`top`, `postMessage`)
/// sont les tranches suivantes — l'étape livre le
/// contexte réseau + DOM.
pub(crate) struct SubFrame {
    /// URL effectivement chargée (résolue sur la page) —
    /// `about:srcdoc` pour un document inline (CAP-142).
    pub(crate) url: Url,
    /// Drapeaux `sandbox` de l'élément (CAP-149) — `None` =
    /// contexte plein droit.
    pub(crate) sandbox: Option<SandboxFlags>,
    /// HTML du `srcdoc` quand le sous-contexte est inline
    /// (CAP-142) — clé d'association boîte ↔ contexte côté
    /// composite (`None` = contexte fetché, clé = `url`).
    pub(crate) srcdoc: Option<String>,
    /// DOM du document enfant — propre arbre, isolé du
    /// parent (aucun accès DOM croisé n'existe).
    pub(crate) dom: Dom,
    /// `<title>` extrait, s'il existe.
    pub(crate) title: Option<String>,
    /// Source HTML décodée du document enfant (étape 134) —
    /// le miroir JS du contexte enfant est re-parsé depuis
    /// elle (`install_dom_ctx`), comme `install_dom` pour
    /// le document top-level.
    pub(crate) html: String,
}

/// Lien `SubFrame` ↔ contexte JS (étape 134) : `ctx` est
/// l'identifiant moteur ([`explorer_js::JsCtxId`]) du realm
/// de l'`<iframe>` ; `sub` indexe `Tab::frames` ; `url`
/// duplique celle du sous-document pour la résolution de
/// base/initiateur des `NetRequest` tagués `ctx`.
#[derive(Debug, Clone)]
pub(crate) struct FrameCtx {
    /// Contexte JS de l'enfant (`JS_CTX_TOP` jamais ici).
    pub(crate) ctx: u32,
    /// Index dans `Tab::frames` du `SubFrame` affiché.
    pub(crate) sub: usize,
    /// URL du document enfant — base de résolution et
    /// initiateur de ses requêtes réseau.
    pub(crate) url: Url,
    /// `NodeId` de l'élément `<iframe>` porteur dans le DOM
    /// parent — sert à émettre `load` sur l'élément ; mort
    /// au re-parse (nettoyé, jamais déréférencé aveuglément).
    pub(crate) node: explorer_dom::NodeId,
    /// Le contexte vit en origine **opaque** (CAP-149 :
    /// `sandbox` sans `allow-same-origin`) — storage isolé
    /// non persisté, cookies muets, `Origin: null` et
    /// cookies `SameSite` jamais same-site sur ses
    /// `NetRequest`.
    pub(crate) opaque: bool,
    /// `readystatechange`/`load` déjà tirés dans le realm
    /// (CAP-171 : un `<iframe>` dynamique installe son
    /// contexte et tire son cycle complet à la réponse — un
    /// script parseur peut le faire **avant**
    /// `frame_ctx_load_events` du même commit, qui ne doit
    /// pas rejouer le cycle).
    pub(crate) load_fired: bool,
}

/// Étape du cycle de vie document tamponnée pendant le
/// détachement du moteur JS (étape 53f) — rejouée à la
/// réinsertion, dans l'ordre d'émission. Sans ce tampon,
/// `set_ready_state`/`readystatechange`/`DOMContentLoaded`/
/// `load` du tail de commit tombaient dans le vide quand il
/// courait pendant une pompe détachée (`entry.js = None` →
/// drop muet) : `readyState` restait `"loading"` à jamais
/// et les portes `readystatechange` des pages réelles
/// (ex. `element-registry` de github, dont les
/// custom-elements lazy attendent `ready`) pendaient
/// indéfiniment — sans la moindre erreur surfacée.
#[derive(Debug)]
pub(crate) enum DocLifecycle {
    /// `document.readyState` ← l'état, pour le contexte.
    ReadyState(explorer_js::JsCtxId, String),
    /// Événement cible document dans le realm
    /// (`readystatechange`, `DOMContentLoaded`, `load`,
    /// `scroll`, `wheel`… — vide `path`/`props`).
    Event(explorer_js::JsCtxId, String),
}

/// Borne de [`super::Tab::pending_doc_lifecycle`] — le tail
/// de commit n'émet que 6 étapes ; la garde borne un flot
/// d'événements document (`scroll`/`wheel`) pendant une
/// longue pompe détachée.
pub(crate) const MAX_PENDING_LIFECYCLE: usize = 32;

/// Borne de [`super::Tab::pending_anim_births`] — naissances
/// d'animations collectées par passe de style en attente de
/// dispatch `animationstart` (étape 141b). Une page hostile
/// qui recalcule en boucle ne peut saturer la file.
pub(crate) const MAX_ANIM_BIRTHS: usize = 256;

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
        partition: c.partition.clone(),
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
        partition: s.partition.clone(),
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
    /// Attente bornée expirée (B-33 — `nav_paint` en
    /// attente d'un détenteur mort en vol).
    #[error("attente expirée : {0}")]
    Timeout(String),
}

/// Configuration de session — seuils et viewport initiaux
/// (règle 13 : aucune valeur en dur dans le code).
#[derive(Debug, Clone, Copy)]
pub struct SessionConfig {
    /// Capacité du bus d'événements (au-delà, les abonnés
    /// lents sautent des événements — le pull PNG corrige
    /// les frames, mais les `PipelineError` enregistrés
    /// dans le ring console sont **perdus** pour de bon :
    /// dimensionné pour les bursts d'une navigation
    /// complète — vol 18 x.com).
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
    /// Défaut 256 : corpus de vol §B-20 — marmiton ~216
    /// `data-src` par document, lemonde ~195 ; à 32 la
    /// première vague (icônes de menus cachés) épuisait
    /// tout le budget et les images éditoriales n'étaient
    /// jamais fetchées. La borne borne toujours les pages
    /// hostiles et couvre `fetch_backgrounds` (solde du
    /// même budget).
    pub max_images: usize,
    /// Budget pixels par image décodée (≈ 4 octets/pixel —
    /// borne l'allocation avant décodage).
    pub image_max_px: u32,
    /// Nombre max de feuilles `<link>` fetchées par document
    /// (bornes sous-ressources — une page peut en chaîner
    /// des centaines). La borne s'applique **après** le
    /// filtre `media=` (étape 96) : une variante
    /// `print`/`(prefers-color-scheme: dark)` hors viewport
    /// ne dépense pas le quota. Défaut 32 — corpus
    /// `site_smoke` : GitHub ~22 feuilles effectives, MDN ~20.
    pub max_sheets: usize,
    /// Nombre max de feuilles `@import` fetchées par document
    /// (chaîne bornée — dédup par URL et profondeur
    /// `MAX_IMPORT_DEPTH` neutralisent les boucles).
    pub max_imports: usize,
    /// Capacité du registre `@font-face` par session
    /// (règle budgets — les faces sont par document, le
    /// registre est purgé à chaque navigation).
    pub max_fonts: usize,
    /// Nombre max de `<script src>` fetchés par document
    /// (étape 50 — une page peut en déclarer des centaines).
    pub max_scripts: usize,
    /// Nombre max de `<iframe src>` fetchés par document
    /// (étape 128 — un sous-contexte = un document complet ;
    /// `0` = iframes désactivés). Profondeur 1 : les
    /// `<iframe>` **dans** un document enfant ne sont pas
    /// descendus (borne la cascade de sous-contextes).
    pub max_frames: usize,
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
    /// Navigations **automatiques** par onglet (meta refresh +
    /// intentions script `location.*`/reload/submit) — borne
    /// session, distincte de `MAX_NAV_HOPS` (chaîne) et de
    /// `MAX_PENDING_NAV` (file). Cdiscount mesuré : 3 812 navs
    /// auto en boucle = process à tuer ; le web légitime en
    /// fait une poignée. `0` = aucune nav auto.
    pub max_auto_navs: u32,
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
    /// Schéma de couleurs préféré propagé à `StyleEnv`
    /// (étape 97) : `prefers-color-scheme` dans les media
    /// queries, filtre `media=` des `<link>` (étape 96) et
    /// défauts UA sombres de la cascade (R-59). Défaut
    /// `Light`.
    pub color_scheme: explorer_css::ColorScheme,
    /// Borne d'ops d'un journal de mutations rejoué tel quel
    /// (ADR-0025, étape 111) — au-delà, le re-parse complet
    /// coûte moins cher que le rejeu + la sérialisation des
    /// sous-arbres insérés. Garde-fou « mutations massives »
    /// de l'ADR, pas un seuil de fonctionnalité.
    pub dom_journal_max_ops: usize,
    /// Borne mémoire du heap JS par contexte, en Mio
    /// (étape 123 — `Engine::set_mem_limit`, JS-C37/ADR-0019 ;
    /// `0` = illimité explicite). Réglage d'**outillage
    /// diagnostic** : il permet d'atteindre un espace
    /// d'exécution masqué par la protection (ex. classer une
    /// exception derrière le challenge `PoW` `SG_SS` de Google)
    /// — il ne rend **pas** un tel site fonctionnel (`PoW`
    /// hostile-by-design ~100× trop lent sous interpréteur,
    /// mesuré hors de portée à 2 Gio d'ops). Défaut 512 Mio,
    /// aligné sur `DEFAULT_MEM_LIMIT` du moteur : la
    /// protection reste le comportement nominal.
    pub js_mem_limit_mib: usize,
    /// Disjoncteur « moteur figé » (vol 25 : worker jscore
    /// saturé — chaque `recv_eval` patiente 30 s sous verrou
    /// → API muette ~90 s). Après un timeout/`Dead`, les
    /// appels moteur bloquants de l'onglet sont court-circuités
    /// pendant cette durée — le verrou session est relâché
    /// vite, l'onglet perd son JS le temps du refroidissement.
    /// `Duration::ZERO` = disjoncteur désactivé. Défaut 10 s.
    pub js_stall_cooldown: std::time::Duration,
    /// Budget wall-clock d'une passe de livraisons `poll_net`
    /// (Étape 151 / §B-54 — flixbus vol 93 : des livraisons
    /// `NetDone` chaînées multi-secondes tenaient le verrou
    /// session → API morte). Au-delà, le reste de la passe est
    /// différé en `pending_net_done` et relivré **en tête** à
    /// la passe suivante — la pompe bat en continu, rien n'est
    /// perdu. `Duration::ZERO` = borne désactivée. Défaut 2 s.
    pub poll_net_budget: std::time::Duration,
    /// Budget du chemin `event.*` sous verrou (Étape 151 /
    /// §B-54) : dispatch + synchro miroir mesurés depuis
    /// l'entrée de l'événement — au-delà, la queue
    /// `after_script`/`poll_net` est sautéee (récurrente, la
    /// prochaine pompe la livre — l'apply enfilé est
    /// conservé). `Duration::ZERO` = borne désactivée.
    /// Défaut 15 s.
    pub event_tail_budget: std::time::Duration,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            event_capacity: 512,
            viewport: (1024.0, 768.0),
            scale: 1.0,
            js_ops: 5_000_000,
            max_images: 256,
            image_max_px: 16_000_000,
            max_sheets: 32,
            max_imports: 16,
            max_fonts: 16,
            max_scripts: 32,
            max_frames: 8,
            // Graphe statique réel : mastodon `application-*.js`
            // en compte >64 (axios = n°65+ → « module non chargé »,
            // mesuré en vol). 256 borne encore les chaînes hostiles.
            max_modules: 256,
            max_inflight: 6,
            max_script_net: 16,
            max_timer_jobs: 128,
            max_auto_navs: 64,
            paint_chunk_bytes: 32 * 1024,
            paint_interval_ms: 100,
            resync_gap_max_ms: 4_000,
            max_download_bytes: 64 * 1024 * 1024,
            bfcache_pages: 4,
            color_scheme: explorer_css::ColorScheme::default(),
            dom_journal_max_ops: 512,
            js_mem_limit_mib: 512,
            js_stall_cooldown: std::time::Duration::from_secs(10),
            poll_net_budget: std::time::Duration::from_secs(2),
            event_tail_budget: std::time::Duration::from_secs(15),
        }
    }
}
