// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! `explorer-js` — couture JavaScript (ADR-0007) : trait
//! [`JsEngine`], moteur nul par défaut et adaptateur
//! [`JscoreEngine`] vers `explorer-jscore`.
//!
//! `explorer_jscore::Engine` est `!Send` (`Rc` dans le tas, les envs et les
//! hooks hôtes) alors que la session vit derrière un `tokio` mutex
//! exigeant `Send`. [`JscoreEngine`] est donc un **proxy canal** : un
//! thread dédié par contexte possède le moteur (single-writer — le
//! modèle navigateur : un contexte JS par document, isolé), les
//! appels (`eval`, jobs dus, budget) voyagent par message. Un script
//! hostile ne peut pas suspendre le pipeline : le budget d'ops est
//! borné côté moteur (`set_op_limit`), et la couche appelante peut
//! borner l'attente si besoin.

#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::thread::JoinHandle;

/// Erreur JavaScript côté hôte — sérialisable, sans handle de tas.
#[derive(Debug, thiserror::Error)]
pub enum JsError {
    /// Source impossible à parser.
    #[error("syntaxe js {line}:{col} : {message}")]
    Syntax {
        /// Ligne (1-base).
        line: usize,
        /// Colonne (1-base).
        col: usize,
        /// Détail du parseur.
        message: String,
    },
    /// Le script a lancé une exception non rattrapée.
    #[error("exception js : {0}")]
    Uncaught(String),
    /// Le thread moteur est mort (panic interne, poison).
    #[error("moteur js inerte")]
    Dead,
    /// Éval/drain dépassé `EVAL_CMD_TIMEOUT` — disjoncteur
    /// « moteur figé » (boucle hostile). Kind typé : la session
    /// distingue un stall d'une exception du script.
    #[error("moteur figé — éval dépassée")]
    Timeout,
}

impl From<explorer_jscore::Error> for JsError {
    fn from(e: explorer_jscore::Error) -> Self {
        match e {
            explorer_jscore::Error::Syntax { line, col, message } => {
                Self::Syntax { line, col, message }
            }
            explorer_jscore::Error::Uncaught(m) => Self::Uncaught(m),
            explorer_jscore::Error::Deadline => Self::Timeout,
        }
    }
}

/// Intention de navigation émise par un script
/// (`location.assign`/`href=`, `history.go`…) — drainée par
/// le hôte après l'appel JS (jamais de navigation re-entrante
/// pendant l'évaluation).
#[derive(Debug, Clone, PartialEq)]
pub enum NavRequest {
    /// `location.href =`/`assign` — cible brute à résoudre
    /// contre l'URL de la page.
    Navigate(String),
    /// `history.go(n)`/`back()`/`forward()` — déplacement
    /// relatif dans l'historique de session.
    HistoryGo(i32),
    /// `form.submit()`/`requestSubmit()` (étape 54) — champs
    /// déjà collectés côté miroir ; la session résout
    /// `action`, choisit le transport (`method`/`enctype`)
    /// et encode.
    FormSubmit {
        /// `method` du `<form>` (`"get"` par défaut).
        method: String,
        /// `action` brute — à résoudre à l'URL de page.
        action: String,
        /// `enctype` brut (`""` ≙ urlencoded).
        enctype: String,
        /// `(name, value)` des contrôles « successful ».
        fields: Vec<(String, String)>,
        /// `formtarget`/`target` du formulaire (CAP-159) —
        /// `""` = contexte courant ; `_blank`/nommé =
        /// contexte neuf ou nommé à réutiliser.
        target: String,
    },
    /// `history.pushState`/`replaceState` (CAP-151) —
    /// navigation **same-document** : la session résout
    /// `url` contre la base du document, exige same-origin,
    /// commit URL + historique sans fetch puis resynchronise
    /// le miroir via [`JsEngine::sync_ctx_url`]. Jamais
    /// routée vers `navigate_request` (aucun fetch).
    PushState {
        /// Argument `url` brut (peut être relatif ou vide
        /// — vide = nouvelle entrée à même adresse).
        url: String,
        /// `true` = `replaceState` (remplace l'entrée),
        /// `false` = `pushState` (nouvelle entrée).
        replace: bool,
        /// `args[0]` sérialisé JSON (CAP-158) —
        /// `history.state` de l'entrée ; `None` = absent/
        /// non sérialisable (borne structured clone).
        state: Option<String>,
    },
    /// `location.replace(url)` (CAP-152) — navigation
    /// `historyHandling: "replace"` : fetch réel puis
    /// l'entrée d'historique courante est **réécrite**
    /// (back saute la page remplacée — flows auth).
    Replace(String),
    /// `location.reload()` (CAP-152) — rejeu de la requête
    /// de l'entrée courante (méthode + corps — POST re-POSTe)
    /// sans entrée nouvelle.
    Reload,
    /// `window.open(url, target)` ou clic scripté sur `<a
    /// target=_blank|nom>` (CAP-153) — navigation dans un
    /// **nouveau** contexte top-level ; un `name` non vide
    /// désigne un contexte à réutiliser (CAP-159 — spec
    /// « selecting a browsing context »). L'intention vit
    /// dans la file de l'onglet émetteur.
    NewTab {
        /// URL brute — à résoudre côté session.
        url: String,
        /// Nom de cible (`window.open` arg 2, `target` de
        /// `<a>`/`<form>`) — `""` = toujours neuf sans nom.
        name: String,
    },
}

/// Résolveur de spécificateurs de module fourni par l'hôte :
/// `(specifier, referrer) → url absolue`. La session y injecte
/// sa politique `Url::resolve` — le moteur ne connaît pas les
/// URLs (étape 50).
pub type UrlResolver = Arc<dyn Fn(&str, &str) -> Option<String> + Send + Sync>;

/// Requête réseau émise par un script — `XMLHttpRequest` ou
/// `fetch()` (JS-C27). Le moteur ne touche jamais le réseau :
/// la session draine aux points de synchro, exécute via
/// `HttpTransport` (sa politique : origine, CORS, quotas —
/// ADR-0019) puis livre par [`JsEngine::deliver_net_response`].
#[derive(Debug, Clone, PartialEq)]
pub enum NetRequest {
    /// À exécuter — `handle` à répercuter dans [`NetResponse`].
    Fetch {
        /// Identifiant alloué par le moteur (unique par contexte).
        handle: u64,
        /// Méthode HTTP telle qu'écrite.
        method: String,
        /// URL telle qu'écrite — résolution à l'URL de page et
        /// politique = affaire de la session.
        url: String,
        /// En-têtes `setRequestHeader`/opts, dans l'ordre.
        headers: Vec<(String, String)>,
        /// Corps `send(body)`/`fetch({body})`, si présent.
        body: Option<String>,
        /// `Request.mode` de l'appel (JS-C77) — `fetch({mode})`,
        /// `Cors` pour XHR, `NoCors` pour `sendBeacon`.
        mode: explorer_jscore::FetchMode,
        /// `Request.credentials` de l'appel (JS-C77) — gouverne
        /// `Cookie`/`Set-Cookie` côté session.
        credentials: explorer_jscore::FetchCredentials,
        /// Contexte de navigation émetteur (étape 134) —
        /// [`JS_CTX_TOP`] = document top-level, sinon le
        /// contexte de l'`<iframe>` dont le script a tiré.
        /// La session résout la base URL et l'initiateur sur
        /// le document **émetteur**, jamais le parent.
        ctx: JsCtxId,
    },
    /// `xhr.abort()` — annuler la tâche `handle` si encore en vol.
    Abort {
        /// Identifiant de la requête à annuler.
        handle: u64,
    },
    /// Opération `crypto.subtle` émise par le script (JS-C79) —
    /// **pas une requête réseau** : la session l'exécute en
    /// in-process (`explorer-net::crypto`) et livre le
    /// résultat via [`JsEngine::deliver_crypto_response`].
    /// C'est la couture embedder du moteur : jscore reste
    /// zéro-dépendance, la plateforme fournit le calcul.
    Crypto {
        /// Identifiant rendu par `CryptoTransport::submit`.
        handle: u64,
        /// Opération normalisée (`Digest` + algo validé ;
        /// `ImportKey`/`Sign`/`Verify` depuis JS-C93).
        op: explorer_jscore::CryptoOp,
        /// Octets source copiés hors du heap à l'appel.
        data: Vec<u8>,
        /// Octets de la `CryptoKey` (`sign`/`verify`, JS-C93)
        /// — vide pour `digest`/`importKey`.
        key: Vec<u8>,
    },
    /// Ouverture d'un flux SSE `EventSource` (JS-C92) —
    /// requête **streamée** : la session fetch
    /// `text/event-stream` et livre `open`/`message`/`error`
    /// par [`JsEngine::deliver_es_event`] — multi-coups,
    /// contrairement au one-shot `Fetch`/`NetResponse` ;
    /// `close()` se traduit en `Abort` sur le handle.
    EventSource {
        /// Identifiant rendu par `EsTransport::open` —
        /// partage l'espace des handles `Fetch` (même
        /// registre `Abort` côté session).
        handle: u64,
        /// URL absolue — résolue par le moteur au ctor.
        url: String,
        /// `withCredentials` de l'init (`include` ↔ true).
        with_credentials: bool,
        /// Contexte émetteur (étape 134) — les livraisons
        /// visent son realm.
        ctx: JsCtxId,
    },
    /// Ouverture d'une connexion `WebSocket` réelle (JS-C94)
    /// — la session dial via `WsDial`, handshake `101`, puis
    /// livre `open`/`message`/`error`/`close` multi-coups par
    /// [`JsEngine::deliver_ws_event`] (même discipline que le
    /// flux SSE — jamais de callback ré-entrant).
    WsOpen {
        /// Identifiant rendu par `WsTransport::open` — espace
        /// `xhr` partagé (`Abort` univoque).
        handle: u64,
        /// URL absolue `ws://`/`wss://` — résolue au ctor.
        url: String,
        /// Sous-protocoles demandés (tokens validés).
        protocols: Vec<String>,
        /// Contexte émetteur (étape 134).
        ctx: JsCtxId,
    },
    /// `ws.send(data)` — trame vers un flux vivant (JS-C94).
    WsSend {
        /// Handle du flux.
        handle: u64,
        /// Charge utile (texte ou binaire — `WsOut` moteur).
        out: explorer_jscore::WsOut,
    },
    /// `ws.close(code, reason)` — handshake close RFC 6455.
    WsClose {
        /// Handle du flux.
        handle: u64,
        /// Code validé (1000 ou 3000-4999).
        code: u16,
        /// Raison ≤ 123 octets.
        reason: String,
    },
    /// Source `new Worker(url)`/`importScripts` (JS-C132) —
    /// chargement **synchrone** depuis le thread worker : la
    /// requête part sur un canal `Send` dédié (la file `Rc`
    /// des `NetRequest` ne quitte pas le thread moteur), la
    /// session fetch puis répond **par `reply`** — aucune
    /// livraison ne transite par le thread moteur bloqué.
    WorkerLoad {
        /// URL telle qu'écrite — résolution contre
        /// `referrer` (URL du module/page émetteur), repli
        /// base document côté session.
        url: String,
        /// URL de la page émettrice (`hooks.page_url`).
        referrer: String,
        /// Contexte émetteur (étape 134) — top-level pour
        /// les workers classiques.
        ctx: JsCtxId,
        /// Canal de réponse direct — le fetcher session y
        /// écrit `Ok(source)`/`Err(message)` sans repasser
        /// par le thread moteur.
        reply: WorkerReply,
    },
}

/// Canal one-shot d'un [`NetRequest::WorkerLoad`] — enrobage
/// du `Sender` pour garder les derives de `NetRequest`
/// (`PartialEq` : deux requêtes ne partagent jamais leur
/// canal de réponse → toujours distinctes).
#[derive(Clone)]
pub struct WorkerReply(
    /// Réponse livrée par la tâche fetch de la session.
    pub std::sync::mpsc::Sender<std::result::Result<String, String>>,
);

impl std::fmt::Debug for WorkerReply {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("WorkerReply(..)")
    }
}

impl PartialEq for WorkerReply {
    fn eq(&self, _other: &Self) -> bool {
        false
    }
}

/// Réponse livrée au moteur pour un `handle` drainé —
/// `status == 0` = échec réseau (event `error`, rejet de
/// promesse `fetch`). Miroir sérialisable de
/// `explorer_jscore::XhrResponse` (aucun type moteur ne fuit).
#[derive(Debug, Clone)]
pub struct NetResponse {
    /// Identifiant rendu par [`NetRequest::Fetch`].
    pub handle: u64,
    /// Statut HTTP (`0` = échec transport).
    pub status: u16,
    /// Phrase de raison.
    pub status_text: String,
    /// En-têtes de réponse.
    pub headers: Vec<(String, String)>,
    /// Corps brut — `responseText`/`text()` le décode en lossy.
    pub body: Vec<u8>,
}

/// Résultat d'une opération `crypto.subtle` livrée au moteur
/// (JS-C79) — `Ok` = octets résultat (digest → `ArrayBuffer`),
/// `Err` = message → rejet `OperationError`. Miroir de
/// `explorer_jscore::CryptoResponse`.
#[derive(Debug, Clone)]
pub struct CryptoResponse {
    /// Identifiant rendu par [`NetRequest::Crypto`].
    pub handle: u64,
    /// `Ok(octets)` ou `Err(message)`.
    pub result: Result<Vec<u8>, String>,
}

/// Chemin `(tag, index parmi les frères éléments de même
/// tag)` d'un sous-arbre depuis la racine du document —
/// format partagé miroir ↔ session (étape 55f).
pub type DomPath = Vec<(String, u32)>;

/// Charge d'une synchro DOM : HTML sérialisé du miroir +
/// chemin du sous-arbre sale (`None` = moteur sans suivi
/// granulaire → l'appelant refait un reflow global).
pub type DomSyncPayload = (String, Option<DomPath>);

/// Compteurs cumulés de la synchro miroir → session (JS-C39) —
/// l'instrument du critère d'escalade « la synchro DOM
/// domine-t-elle le frame budget ? » : si `micros`/`bytes` par
/// `syncs` dépassent ~10 % d'un frame ou qu'un churn `DoS` est
/// démontré, un `drain_dom_mutations` additif prend le relais
/// (records JS-C26 déjà mécanisés côté moteur).
#[derive(Debug, Clone, Copy, Default)]
pub struct DomSyncStats {
    /// Synchros ayant produit un payload (sérialisations
    /// réellement effectuées).
    pub syncs: u64,
    /// Octets HTML transférés (cumulés).
    pub bytes: u64,
    /// Microsecondes cumulées dans la sérialisation miroir.
    pub micros: u64,
    /// Ops de journal drainées mais **classées re-parse**
    /// (empoisonné ou contenu/parent hors pilote, cumulé —
    /// ADR-0025 : le périmètre du pilote se mesure ici ; un
    /// taux élevé signale des classes à ouvrir au rejeu).
    pub journal_classified: u64,
}

/// Une opération de mutation différentielle du miroir,
/// rejouable directement sur `explorer-dom` (ADR-0025,
/// étape 111) — `key`/`parent` sont les **`NodeId` miroir**
/// (identité stable côté JS) ; la session tient la table de
/// correspondance `miroir → explorer-dom` reconstruite à
/// chaque baseline [`DomSyncKeyed`].
#[derive(Debug, Clone, PartialEq)]
pub enum DomMutation {
    /// Insertion sous `parent` à la position `index` (parmi
    /// les enfants, éléments et texte confondus). `html` =
    /// sous-arbre sérialisé **au drain** (état courant —
    /// les mutations post-enregistrement sont comprises) ;
    /// `ids` = les `NodeId` miroir des nœuds du sous-arbre
    /// dans l'ordre de sérialisation (pré-ordre) — la
    /// session les apparie aux `NodeId` frais du parse.
    Insert {
        /// `NodeId` miroir du parent.
        parent: u64,
        /// Position d'insertion parmi les enfants.
        index: u32,
        /// Sous-arbre sérialisé (un ou plusieurs nœuds
        /// racine concaténés).
        html: String,
        /// `NodeId` miroir des nœuds insérés (pré-ordre).
        ids: Vec<u64>,
        /// Tags parallèles à `ids` (`#text\x00<hash>` pour les
        /// textes) — le rejeu apparie **par tag** : un nœud
        /// moteur synthétisé au re-parse (`<tbody>`…) ou non
        /// émis ne décale plus les clés suivantes en
        /// `UnmappedKey` (JS-C129 ; avant : zip positionnel
        /// tronqué → repli re-parse).
        tags: Vec<String>,
    },
    /// Retrait du nœud `key` (avec tout son sous-arbre).
    Remove {
        /// `NodeId` miroir de la racine retirée.
        key: u64,
    },
    /// `setAttribute`/`removeAttribute`/`toggleAttribute` —
    /// `value = None` = attribut retiré.
    SetAttr {
        /// `NodeId` miroir de l'élément.
        key: u64,
        /// Nom d'attribut (minuscules).
        name: String,
        /// Nouvelle valeur (`None` = retiré).
        value: Option<String>,
    },
    /// Texte d'un nœud `Text` fixé (`characterData`).
    SetText {
        /// `NodeId` miroir du nœud texte.
        key: u64,
        /// Nouveau contenu.
        text: String,
    },
    /// `attachShadow` programmatique — op de couture
    /// (Étape 137b) : `root` est le fragment miroir créé au
    /// `attachShadow`; le replay crée le fragment réel et
    /// l'attache à l'hôte mappé.
    AttachShadow {
        /// `NodeId` miroir de l'hôte.
        host: u64,
        /// `NodeId` miroir du fragment racine.
        root: u64,
        /// `mode: "open"`.
        open: bool,
    },
}

/// Journal de mutations drainé du miroir (ADR-0025) :
/// ops ordonnées + empreinte structurelle (tags en
/// pré-ordre, hors texte) du miroir **post-journal** — la
/// session la recalcule sur le DOM rejoué ; divergence →
/// repli re-parse. `None` de
/// [`JsEngine::drain_dom_mutations`] = « synchro par
/// sérialisation » (journal vide ou non rejouable — le
/// moteur décide, l'appelant n'a pas à distinguer).
#[derive(Debug, Clone)]
pub struct DomJournal {
    /// Ops ordonnées.
    pub ops: Vec<DomMutation>,
    /// Empreinte structurelle du miroir post-journal.
    pub fingerprint: u64,
}

/// Charge de synchro **cléée** (baseline ADR-0025) : le
/// HTML sérialisé + `ids` = `NodeId` miroir de chaque nœud
/// dans l'ordre de sérialisation (pré-ordre). La session
/// apparie les `NodeId` frais du re-parse pour (re)monter
/// la table d'identité — les [`DomMutation`] suivantes
/// adressent alors les nœuds par clé stable.
#[derive(Debug, Clone)]
pub struct DomSyncKeyed {
    /// HTML sérialisé du miroir.
    pub html: String,
    /// Chemin du sous-arbre sale (comme [`DomSyncPayload`]).
    pub dirty: Option<DomPath>,
    /// `NodeId` miroir en pré-ordre de sérialisation —
    /// vide = moteur sans identité (repli `sync_dom_path`).
    pub ids: Vec<u64>,
    /// Tag de chaque `ids[i]` (`"#text\x00{fnv64(contenu)}"`
    /// pour un nœud texte) — permet l'alignement **par tag**
    /// de la table miroir→moteur, robuste aux normalisations
    /// du parse moteur (`<tbody>` synthétisé, texte
    /// d'espaces perdu) : les `NodeId` moteur sans
    /// correspondant miroir sont ignorés plutôt que décalés.
    pub tags: Vec<String>,
}

/// Op-list `<canvas>` enregistrée par un contexte 2D du
/// miroir (JS-C28, couture R-11) — `explorer_jscore::CanvasOpRecord`
/// est le miroir sérialisable de
/// `explorer_render::canvas::CanvasOp` : la session traduit
/// champ pour champ et re-résout les chemins en `NodeId`.
pub use explorer_jscore::CanvasOpRecord;

/// Compteurs d'occupation du tas JS (JS-C37) — réexport de
/// [`explorer_jscore::HeapStats`] pour que les consommateurs du
/// trait `JsEngine` n'aient pas à dépendre du moteur.
pub use explorer_jscore::HeapStats;

/// Propriété d'événement plateforme (étape 103 —
/// `KeyboardEvent.key`/`code`/`keyCode`/`which`) — réexport de
/// [`explorer_jscore::EventProp`] pour le même motif que
/// `HeapStats` : le payload traverse le trait `JsEngine`
/// (`dispatch_event_props`), la session le calcule.
pub use explorer_jscore::EventProp;

/// Table « chemin élément → op-list **cumulée** » lue sur le
/// moteur — mêmes chemins que [`DomSyncPayload`] (ils
/// survivent au re-parse). La liste n'est pas drainée : la
/// bitmap canvas est persistante (spec), le playback R-11
/// la rejoue entière à chaque re-peinture.
pub type CanvasOpsTable = Vec<(DomPath, Vec<CanvasOpRecord>)>;

/// État de lecture rapporté par l'hôte média (étape 71c,
/// ADR-0024) — miroir sérialisable du DTO session ; aucun
/// type moteur ne fuit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JsMediaHostState {
    /// Lecture en cours.
    Playing,
    /// En pause.
    Paused,
    /// Fin de flux atteinte.
    Ended,
    /// Erreur hôte — `JsMediaReport::error` porte la cause.
    Error,
}

/// Rapport `state` hôte relu pour le moteur (étape 71c) —
/// positions en millisecondes, `None` duration = flux sans
/// durée connue.
#[derive(Debug, Clone)]
pub struct JsMediaReport {
    /// Tête de lecture (ms).
    pub position_ms: u64,
    /// Durée totale (ms) si connue.
    pub duration_ms: Option<u64>,
    /// Fin de la zone tampon (ms).
    pub buffered_ms: u64,
    /// État de lecture.
    pub state: JsMediaHostState,
    /// Cause si `state == Error`.
    pub error: Option<String>,
}

/// Ordre média page → hôte (étape 71c) — drainé par la
/// session qui émet `CoreEvent::MediaCommand`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum JsMediaCmd {
    /// `play()`.
    Play,
    /// `pause()`.
    Pause,
    /// `currentTime`/`fastSeek` (millisecondes).
    Seek {
        /// Position cible.
        ms: u64,
    },
    /// `volume = …` (0.0–1.0).
    Volume {
        /// Volume demandé.
        v: f64,
    },
    /// `muted = …`.
    Muted {
        /// Muet demandé.
        on: bool,
    },
    /// `playbackRate`/`defaultPlaybackRate = …`.
    Rate {
        /// Débit demandé.
        r: f64,
    },
}

/// Ordre média drainé — `media` = l'id de surface `u64`
/// transmis au claim (la session route sans re-résoudre de
/// chemin : l'identité survit aux re-parse miroir).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct JsMediaCommand {
    /// Surface session visée.
    pub media: u64,
    /// Ordre demandé par la page.
    pub cmd: JsMediaCmd,
}

/// Moteur JavaScript vu du navigateur — un contexte par document.
/// `Send` exigé : la session est partagée derrière l'API.
pub trait JsEngine: Send {
    /// Évalue un script dans le contexte ; retourne la valeur de
    /// complétion rendue lisible (`describe`).
    ///
    /// # Errors
    /// [`JsError::Syntax`] à l'analyse, [`JsError::Uncaught`] si le
    /// script jette, [`JsError::Dead`] si le moteur est inerte.
    fn eval(&mut self, source: &str) -> Result<String, JsError>;

    /// `eval` étiqueté : `name` identifie la source (URL du
    /// `<script src>`…) dans les frames `error.stack` —
    /// attribution par bundle pour le triage de site réel
    /// (JS-C56). Défaut : [`Self::eval`] (moteur sans notion
    /// d'origine).
    ///
    /// # Errors
    /// Identique à [`Self::eval`].
    fn eval_named(&mut self, source: &str, name: &str) -> Result<String, JsError> {
        let _ = name;
        self.eval(source)
    }

    /// Vide les jobs dus (microtasks, timers échus) — retourne le
    /// nombre exécuté. Défaut : rien à faire.
    fn run_pending(&mut self) -> usize {
        0
    }

    /// [`Self::run_pending`] borné : au plus `max_timers`
    /// callbacks de timers déclenchés (ADR-0019 — borne de jobs
    /// par tick ; le surplus reste en file pour la pompe
    /// suivante). Défaut : [`Self::run_pending`] (moteur sans
    /// notion de borne — le quota est alors non exigé).
    /// `Err(JsError::Timeout)` = deadline wall-clock interne
    /// (J1) — l'appelant arme le disjoncteur moteur figé.
    ///
    /// # Errors
    /// `JsError::Timeout` si la deadline interne tue la rafale,
    /// `JsError::Uncaught` sur un callback qui jette,
    /// `JsError::Dead` si le moteur ne répond plus.
    fn run_pending_bounded(&mut self, max_timers: usize) -> Result<usize, JsError> {
        let _ = max_timers;
        Ok(self.run_pending())
    }

    /// Délai jusqu'à la prochaine échéance de timer JS —
    /// `None` si aucun timer armé. L'hôte s'en sert pour armer
    /// son battement (étape 52 : les `setTimeout`/`setInterval`/
    /// `rAF` tirent sans attendre un input). Défaut : `None`.
    fn next_timer_deadline(&mut self) -> Option<std::time::Duration> {
        None
    }

    /// `url` est-elle un object URL `MediaSource` enregistré
    /// (`createObjectURL(ms)`) — le scan média core marque
    /// l'offre `mse` pour que l'hôte rout son `MseSource`
    /// (MSE-2, ADR-0029). Défaut : `false` (pas de
    /// registre MSE — moteur nu).
    fn is_media_source(&mut self, _url: &str) -> bool {
        false
    }

    /// Pull FIFO des segments `appendBuffer` du `SourceBuffer`
    /// `buffer` (index `addSourceBuffer`) de la `MediaSource`
    /// enregistrée sous `url` — transport MSE-3 (ADR-0029) :
    /// au plus `max` octets, retour `(octets, curseur, eos)`.
    /// `eos` vrai quand `endOfStream` est appelé **et** la
    /// file est vide. `None` : pas de source sous `url` ou
    /// index hors rang. Défaut : `None`.
    fn mse_pull(
        &mut self,
        _url: &str,
        _buffer: usize,
        _max: usize,
    ) -> Option<(Vec<u8>, u64, bool)> {
        None
    }

    /// MIME des `SourceBuffer` (index = ordre `addSourceBuffer`)
    /// de la `MediaSource` enregistrée sous `url` — l'offre
    /// `mse` les publie pour `open_mse_multi` (MSE-3/P-53).
    /// Défaut : vide.
    fn mse_buffers(&mut self, _url: &str) -> Vec<String> {
        Vec::new()
    }

    /// `timestampOffset` des `SourceBuffer` sous `url` — même
    /// ordre que [`JsEngine::mse_buffers`] ; `0.0` défaut
    /// spec. Offre `mseOffsets` → `pts_offset_secs` `MseDesc`.
    fn mse_offsets(&mut self, _url: &str) -> Vec<f64> {
        Vec::new()
    }

    /// `(promises créées, réglées)` par moteur — §B-4(c) :
    /// un gap stable ≥1 sur un boot muet = un `await` gelé
    /// (promesse jamais réglée, invisible aux erreurs).
    /// Défaut : `(0, 0)` — moteur nu.
    fn promise_stats(&mut self) -> (u64, u64) {
        (0, 0)
    }

    /// `(timers en attente, microtasks en file, readyState)`
    /// — §B-4(c) : discrimine « bootstrap garé sur un job
    /// jamais drainé » (jobs > 0) de « retour avalé
    /// synchrone » (tout à 0 + `complete`). Défaut : moteur
    /// nu.
    fn boot_diag(&mut self) -> (usize, usize, &'static str) {
        (0, 0, "loading")
    }

    /// Pousse un attribut du DOM hôte vers le nœud miroir
    /// désigné par `path` — quand la session mute le document
    /// **hors JS** (champ édité, case basculée), le miroir
    /// sinon verrait l'ancienne valeur dans les handlers
    /// d'événements (`input`, `click`…). `value: None` retire
    /// l'attribut. Cas particulier `attr == "value"` sur
    /// `<textarea>` : le contenu texte est remplacé (la
    /// propriété `value` n'y est pas un attribut). Défaut :
    /// rien (pas de miroir).
    fn sync_mirror_attr(&mut self, _path: &[(String, u32)], _attr: &str, _value: Option<&str>) {}

    /// Installe le DOM miroir du document (bindings `document`,
    /// `navigator`, `window`…). `html` = source du document tel
    /// que reçu — le miroir le re-parse côté moteur.
    /// `page_url` alimente `location`/`document.URL` ;
    /// `history_len` alimente `history.length`.
    /// Défaut : pas de bindings (moteur nu).
    fn install_dom(&mut self, _html: &str, _page_url: &str, _history_len: u32) {}

    /// Crée un contexte de navigation **enfant** (étape 134 —
    /// un `<iframe>` fetché = un realm propre, JS-C78) :
    /// intrinsics frais, `parent`/`top`/`frames[n]` câblés,
    /// `contentWindow` résolu via `frame_ord` = position de
    /// l'élément `<iframe>` dans l'ordre document du parent
    /// (`None` = cadre orphelin — `contentWindow` reste
    /// `null`). Renvoie le [`JsCtxId`] alloué, `None` sur un
    /// moteur mono-contexte. Le contexte créé n'est **pas**
    /// laissé courant : l'état d'évaluation revient au
    /// contexte de l'appelant.
    fn context_new(&mut self, _url: &str, _frame_ord: Option<u32>) -> Option<JsCtxId> {
        None
    }

    /// [`Self::context_new`] explicitement sous le contexte
    /// `parent` (CAP-171) : un `<iframe>` inséré **dans un
    /// cadre enfant** doit voir `frame_ord` résolu dans le
    /// miroir de SON document parent et son realm rattaché à
    /// ce parent (`window.parent` de l'enfant = son
    /// émetteur) — sinon l'ordre serait lu dans le document
    /// courant. Défaut : [`Self::context_new`] (moteur
    /// mono-contexte ou sans ordre par parent).
    fn context_new_in(
        &mut self,
        _parent: JsCtxId,
        url: &str,
        frame_ord: Option<u32>,
    ) -> Option<JsCtxId> {
        self.context_new(url, frame_ord)
    }

    /// Détruit un contexte enfant (étape 134) — page quittée
    /// **hors bfcache** ou cadre supprimé : `frames[]` du
    /// parent rétrécit, `postMessage` vers lui devient no-op.
    /// `JS_CTX_TOP`/inconnu → no-op (tolérance). Défaut : rien.
    fn context_destroy(&mut self, _ctx: JsCtxId) {}

    /// Installe le DOM miroir **du contexte `ctx`** (étape 134)
    /// — même séquence que [`Self::install_dom`] (miroir
    /// re-parsé, `document`/`location`, timers, XHR/fetch)
    /// avec deux écarts : les hooks de navigation ne sont pas
    /// posés (la navigation de cadre est une piste à part —
    /// `location.href=` y est inerte plutôt que de risquer le
    /// document top-level) et l'état d'origine (`local`
    /// = localStorage, `cookie_header` = vue `document.cookie`)
    /// est **celui de l'origine de l'enfant**, posé sur un
    /// jar isolé — aucun partage avec le parent. Défaut : rien.
    fn install_dom_ctx(
        &mut self,
        _ctx: JsCtxId,
        _html: &str,
        _page_url: &str,
        _local: std::collections::BTreeMap<String, String>,
        _cookie_header: &str,
    ) {
    }

    /// [`Self::eval`]/[`Self::eval_named`] adressés au contexte
    /// `ctx` (étape 134) — le script court dans le realm de
    /// l'`<iframe>` : ses globals, son `document`, son
    /// `fetch`. Contexte mort/inconnu → [`JsError::Dead`],
    /// jamais d'exécution dans le mauvais realm. Défaut :
    /// délègue à [`Self::eval_named`] (moteur mono-contexte —
    /// le tag est ignoré).
    ///
    /// # Errors
    /// Identique à [`Self::eval`].
    fn eval_ctx(
        &mut self,
        _ctx: JsCtxId,
        source: &str,
        name: Option<&str>,
    ) -> Result<String, JsError> {
        match name {
            Some(n) => self.eval_named(source, n),
            None => self.eval(source),
        }
    }

    /// Draine les `<script>` insérés dynamiquement
    /// (CAP-169) — `createElement('script')` + `appendChild`
    /// & co : la session résout `src`/`text`, applique la
    /// politique (CSP/mixed-content) et livre via
    /// [`Self::deliver_dyn_script`]. Défaut : rien (moteur
    /// sans DOM miroir dynamique).
    fn drain_dyn_scripts(&mut self) -> Vec<explorer_jscore::DynScriptReq> {
        Vec::new()
    }

    /// Descriptions des `unhandledrejection` tirés (JS-C154)
    /// — la session les verse au ring console. Défaut : rien.
    fn drain_unhandled_rejections(&mut self) -> Vec<String> {
        Vec::new()
    }

    /// Livre le résultat d'un `<script>` dynamique au moteur
    /// (CAP-169) — `Some(text)` : évalué dans le contexte
    /// `ctx` émetteur puis `load` tiré sur l'élément ;
    /// `None` : refus réseau/politique → `error`. Défaut :
    /// no-op.
    fn deliver_dyn_script(&mut self, _ctx: JsCtxId, _seq: u64, _body: Option<String>) {}

    /// Livre le résultat d'un `<script type="module">`
    /// dynamique (CAP-186) — le graphe est évalué en amont
    /// (`eval_module`) ; `true` → `load`, `false` → `error`
    /// sur l'élément. Défaut : no-op.
    fn deliver_dyn_module(&mut self, _ctx: JsCtxId, _seq: u64, _ok: bool) {}

    /// Draine les `<iframe>` insérés dynamiquement (ou
    /// re-navigués par `src=`/`srcdoc=` — CAP-171) : la
    /// session fetch le `src` (CSP `frame-src`, XFO,
    /// cookies/partition) ou parse le `srcdoc`, crée le
    /// sous-contexte (`context_new` + `install_dom_ctx`)
    /// puis livre via [`Self::deliver_dyn_frame`]. Défaut :
    /// rien (moteur sans DOM miroir dynamique).
    fn drain_dyn_frames(&mut self) -> Vec<explorer_jscore::DynFrameReq> {
        Vec::new()
    }

    /// Livre le résultat d'un `<iframe>` dynamique au moteur
    /// (CAP-171) — `true` : sous-contexte créé, `load` tiré
    /// sur l'élément ; `false` : refus réseau/politique →
    /// `error`. Défaut : no-op.
    fn deliver_dyn_frame(&mut self, _ctx: JsCtxId, _seq: u64, _ok: bool) {}

    /// Draine les `<img>` dont la source a été demandée
    /// (CAP-172 — `src`/`srcset`/`sizes` posé ou insertion ;
    /// élément détaché inclus, `new Image().src=` = prefetch
    /// spec) : la session fetch le candidat choisi
    /// (`srcset`+`sizes`+dpr, CSP `img-src`, mixed-content,
    /// `crossorigin`/`referrerpolicy`) ou décode `data:`,
    /// puis livre via [`Self::deliver_dyn_img`]. Défaut :
    /// rien (moteur sans DOM miroir dynamique).
    fn drain_dyn_imgs(&mut self) -> Vec<explorer_jscore::DynImgReq> {
        Vec::new()
    }

    /// Livre le résultat d'un `<img>` dynamique au moteur
    /// (CAP-172) — `true` : corps décodé et rangé,
    /// `load` tiré sur l'élément ; `false` : refus
    /// réseau/politique → `error`. Défaut : no-op.
    fn deliver_dyn_img(&mut self, _ctx: JsCtxId, _seq: u64, _ok: bool) {}

    /// Draine les `<link rel=stylesheet>` demandés par
    /// script (CAP-173 — insertion ou `href`/`rel` changé
    /// sur lien attaché ; détaché exclu, spec « link
    /// processing ») : la session fetch le `href` (CSP
    /// `style-src`, mixed-content, `crossorigin`/
    /// `referrerpolicy`, SRI `integrity`), applique le
    /// texte à la cascade puis livre via
    /// [`Self::deliver_dyn_link`]. Défaut : rien (moteur
    /// sans DOM miroir dynamique).
    fn drain_dyn_links(&mut self) -> Vec<explorer_jscore::DynLinkReq> {
        Vec::new()
    }

    /// Livre le résultat d'un `<link>` dynamique au moteur
    /// (CAP-173) — `true` : feuille appliquée à la cascade,
    /// `load` tiré sur l'élément ; `false` : refus
    /// réseau/politique/SRI → `error`. Défaut : no-op.
    fn deliver_dyn_link(&mut self, _ctx: JsCtxId, _seq: u64, _ok: bool) {}

    /// `document.adoptedStyleSheets` → textes CSS par feuille
    /// (CAP-195 tranche 2) — snapshot à la demande, joint par
    /// feuille, ordre du tableau (spec : après les `<link>`).
    /// La session les cascade en feuilles auteur document.
    /// Défaut : rien.
    fn adopted_sheets_css(&mut self) -> Vec<String> {
        Vec::new()
    }

    /// Noms des custom elements définis (`customElements
    /// .define`) sur tous les documents — la session les pousse
    /// dans `Dom::set_defined_names` pour `:defined`
    /// (selectors-4 §13). Défaut : rien.
    fn defined_ce_names(&mut self) -> Vec<String> {
        Vec::new()
    }

    /// Drain des requêtes `import()` dynamiques (JS-C100) —
    /// la session fetch `url` et rappelle
    /// `deliver_dyn_import`. Défaut : rien (le miss rejette
    /// comme avant).
    fn drain_dyn_imports(&mut self) -> Vec<explorer_jscore::DynImportReq> {
        Vec::new()
    }

    /// Livre la source d'un `import()` fetchée par la session
    /// (JS-C100) — `Some` → instancier+évaluer+résoudre la
    /// promesse ; `None` → rejet `fetch échoué`. `deps`
    /// (JS-C101) = les deps statiques du graphe livré, seedées
    /// dans `Engine::module_sources` avant l'entrée. Défaut :
    /// no-op.
    fn deliver_dyn_import(
        &mut self,
        _ctx: JsCtxId,
        _seq: u64,
        _src: Option<String>,
        _deps: Vec<(String, String)>,
    ) {
    }

    /// Livre le résultat d'un `<link>` **statique** (parse
    /// initial, CAP-185) — `ord` = ordinal de l'élément
    /// parmi tous les `<link>` du document ; `load`/`error`
    /// sur l'élément miroir comme `deliver_dyn_link`.
    /// Défaut : no-op.
    fn deliver_static_link(&mut self, _ctx: JsCtxId, _ord: u32, _ok: bool) {}

    /// Éval d'un élément `<script>` classique (CAP-168) :
    /// `script_ord` = ordinal de l'élément parmi les
    /// `<script>` du document (ordre document) — pendant
    /// l'évaluation `document.currentScript` le reflète
    /// (spec), puis redevient `null`. Défaut :
    /// [`Self::eval_ctx`] (moteur sans notion d'élément
    /// courant — `currentScript` reste `null`).
    ///
    /// # Errors
    /// Identique à [`Self::eval`].
    fn eval_script_el(
        &mut self,
        ctx: JsCtxId,
        source: &str,
        name: Option<&str>,
        _script_ord: usize,
    ) -> Result<String, JsError> {
        self.eval_ctx(ctx, source, name)
    }

    /// [`Self::dispatch_event`] adressé au contexte `ctx`
    /// (étape 134) — `path` se résout dans le miroir **de
    /// l'enfant** ; `path` vide = événement document de
    /// l'enfant (`DOMContentLoaded`, `load`…). Défaut :
    /// délègue à [`Self::dispatch_event`] (moteur
    /// mono-contexte).
    ///
    /// # Errors
    /// Erreur de listener JS propagée en [`JsError::Uncaught`].
    fn dispatch_event_ctx(
        &mut self,
        _ctx: JsCtxId,
        ty: &str,
        path: &[(String, u32)],
    ) -> Result<bool, JsError> {
        self.dispatch_event(ty, path)
    }

    /// Sérialise le miroir du contexte `ctx` s'il a été muté
    /// depuis la dernière synchro (étape 134) — la session
    /// re-parse le résultat en `SubFrame.dom` pour le composite.
    /// `None` = rien de neuf **ou** moteur sans miroir par
    /// contexte. Défaut : `None`.
    fn sync_dom_ctx(&mut self, _ctx: JsCtxId) -> Option<String> {
        None
    }

    /// Draine les intentions de navigation accumulées depuis
    /// le dernier appel — `location`/`history` n'agissent
    /// jamais en plein vol, la session décide après retour.
    /// Défaut : rien.
    fn drain_nav_requests(&mut self) -> Vec<NavRequest> {
        Vec::new()
    }

    /// Resynchronise `location`/`history.length` du contexte
    /// `ctx` après une navigation same-document scriptée
    /// (`pushState`/`replaceState`, CAP-151) : la session a
    /// résolu et committé — le miroir reçoit l'URL canonisée
    /// et la longueur d'historique réelle. Défaut : no-op
    /// (moteur sans sessions par contexte — harnais).
    fn sync_ctx_url(&mut self, _ctx: JsCtxId, _url: &str, _history_len: u32) {}

    /// `sync_ctx_url` + `history.state` (CAP-158) : la
    /// session remonte l'état JSON de l'entrée courante ;
    /// `popstate` = `true` à la traversée same-document
    /// (l'événement `popstate` est dispatché avec `state`
    /// désérialisé — jamais au `pushState`, spec). Défaut :
    /// délègue à `sync_ctx_url` (état/popstate ignorés).
    fn sync_ctx_state(
        &mut self,
        ctx: JsCtxId,
        url: &str,
        history_len: u32,
        _state: Option<&str>,
        _popstate: bool,
    ) {
        self.sync_ctx_url(ctx, url, history_len);
    }

    /// Nomme le contexte `ctx` pour `window.name` (CAP-159) —
    /// corrélation `target="x"`/`window.open(u,"x")`. Défaut
    /// : no-op (moteur sans contextes nommés — harnais).
    fn set_ctx_name(&mut self, _ctx: JsCtxId, _name: &str) {}

    /// Dispatch `hashchange` (`{oldURL,newURL}`) sur la
    /// fenêtre du contexte `ctx` (CAP-160 — navigation
    /// fragment et traversée entre fragments ; les routeurs
    /// hash des SPA en vivent). Défaut : no-op.
    fn fire_hashchange(&mut self, _ctx: JsCtxId, _old: &str, _new: &str) {}

    /// Pose `document.readyState` du contexte `ctx`
    /// (CAP-170) — la session appelle avec
    /// `"interactive"` avant `DOMContentLoaded` puis
    /// `"complete"` après `load` (le `readystatechange`
    /// est dispatché par `dispatch_event_ctx`, même voie
    /// que les autres events de cycle de vie). Défaut :
    /// no-op.
    fn set_ready_state(&mut self, _ctx: JsCtxId, _state: &str) {}

    /// Viewport de la session en px CSS (CAP-161) — posé
    /// **avant** `install_dom` : `innerWidth`/`screen.*` et
    /// les requêtes `matchMedia` `(min-/max-)width`/`height`/
    /// `orientation`/`aspect-ratio` lisent cette valeur.
    /// Défaut : no-op (viewport `(0,0)` — queries fausses).
    fn set_viewport(&mut self, _w: u32, _h: u32) {}

    /// Table de géométrie publiée par la session (issue #19,
    /// JS-8) — cellule partagée pré-remplie au commit paint
    /// (`Arc<Mutex<..>>` : le JS tourne sur worker, aucun appel
    /// synchrone n'est possible pendant l'exécution).
    /// `getBoundingClientRect`/`offset*`/`client*`/`scroll*`
    /// lisent l'état publié ; document top-level (borné — les
    /// cadres manquent → 0). À poser après `install_dom`.
    /// Défaut : no-op (moteur sans layout → zéros historiques).
    fn set_geometry_ctx(&mut self, _ctx: JsCtxId, _slot: explorer_jscore::GeometrySlot) {}

    /// Environnement `matchMedia` de la session (CAP-161) —
    /// snapshot schéma/pointeur poussé au spawn ;
    /// `matchMedia(q)` l'évalue alors réellement (borné —
    /// voir `explorer_jscore::mediaq`). Défaut : no-op
    /// (env `Default` : scheme clair, souris fine).
    fn set_media_env(&mut self, _env: explorer_jscore::MediaEnv) {}

    /// `document.referrer` (CAP-156) : le `Referer` envoyé
    /// par la navigation du document (`""` = aucun) — posé
    /// après `install_dom`, avant la première évaluation.
    /// Défaut : no-op (moteur sans document réel).
    fn set_page_referrer(&mut self, _referrer: &str) {}

    /// Synchronise le DOM miroir → `Some(html)` si des mutations
    /// JS ont eu lieu depuis la dernière synchro (le hôte
    /// re-parse alors le document), `None` sinon.
    fn sync_dom(&mut self) -> Option<String> {
        None
    }

    /// [`Self::sync_dom`] + le chemin `(tag, index)` du
    /// sous-arbre sale du miroir (étape 55f — même format que
    /// `element_path` côté session : survit au re-parse,
    /// sert à viser `relayout_subtree` au lieu d'un reflow
    /// global ; `None` = moteur sans suivi → reflow complet).
    /// Défaut : délègue à [`Self::sync_dom`] sans chemin.
    fn sync_dom_path(&mut self) -> Option<DomSyncPayload> {
        self.sync_dom().map(|h| (h, None))
    }

    /// Synchro **cléée** (ADR-0025, étape 111) — comme
    /// [`Self::sync_dom_path`] mais avec les `NodeId` miroir en
    /// pré-ordre : la session (re)monte la table d'identité
    /// `miroir → explorer-dom` qui rend le journal
    /// rejouable. Appeler cette méthode **clôt le journal** :
    /// la sérialisation est la nouvelle baseline, les ops
    /// antérieures sont subsumées. Défaut : délègue à
    /// [`Self::sync_dom_path`], `ids` vide (moteur sans
    /// journal — la session reste en mode re-parse).
    fn sync_dom_keyed(&mut self) -> Option<DomSyncKeyed> {
        self.sync_dom_path().map(|(html, dirty)| DomSyncKeyed {
            html,
            dirty,
            ids: Vec::new(),
            tags: Vec::new(),
        })
    }

    /// Variante **inconditionnelle** de [`Self::sync_dom_keyed`] —
    /// sérialise même si le bit sale a déjà été consommé (repli
    /// après rejet d'un journal : le drain avait pris `dirty`, la
    /// baseline doit être re-livrée quand même). Défaut :
    /// délègue à `sync_dom_keyed` (moteur sans journal — un
    /// journal rejeté n'existe pas chez eux).
    fn resync_dom_keyed(&mut self) -> Option<DomSyncKeyed> {
        self.sync_dom_keyed()
    }

    /// Draine le **journal de mutations** depuis la dernière
    /// baseline (ADR-0025, étape 111) : `Some(journal)` = ops
    /// rejouables directement sur `explorer-dom` (`NodeId`
    /// stables — plus de re-parse), `None` = « synchro par
    /// sérialisation » (rien de neuf **ou** journal non
    /// rejouable : mutations hors pilote, seuil dépassé… —
    /// l'appelant enchaîne alors [`Self::sync_dom_keyed`]
    /// sans distinction). Consomme le bit sale comme
    /// `sync_dom_path`. Défaut : `None` (moteur sans journal).
    fn drain_dom_mutations(&mut self) -> Option<DomJournal> {
        None
    }

    /// Compteurs cumulés de la synchro DOM (JS-C39) — à lire
    /// après `sync_dom_path` pour mesurer octets/temps par
    /// synchro. `None` = moteur sans miroir instrumenté.
    fn dom_sync_stats(&mut self) -> Option<DomSyncStats> {
        None
    }

    /// Compteurs de tas JS (JS-C37) — cellules/octets vivants,
    /// pics, passes GC. `None` = moteur sans comptage (ou heap
    /// non instrumenté).
    fn heap_stats(&mut self) -> Option<HeapStats> {
        None
    }

    /// Ligne de diagnostic des réservoirs internes (§B-59 —
    /// `mem_diag` session pendant la pompe) : heap, observers,
    /// timers, miroir Dom, files. `None` = moteur non
    /// instrumenté.
    fn diag_line(&mut self) -> Option<String> {
        None
    }

    /// Passe GC forcée (T3 — trim bfcache) : le gel d'une
    /// page emporte le moteur (~50 Mio/page) — la session
    /// demande une passe avant parc pour ne retenir que le
    /// vivant. Fire-and-forget (la file de commandes
    /// ordonne). Défaut : no-op (moteur sans GC).
    fn gc(&mut self) {}

    /// Snapshot « chemin élément → op-list » des `<canvas>`
    /// ayant dessiné (JS-C28, couture R-11) — liste
    /// **cumulée** relue à chaque appel (bitmap persistante,
    /// jamais drainée). La session la traduit en `CanvasOp`
    /// et re-résout les chemins dans le DOM commis. Défaut :
    /// vide (moteur sans surface canvas).
    fn canvas_ops(&mut self) -> CanvasOpsTable {
        Vec::new()
    }

    /// Enregistre une import map (JS-C44) — `json` = contenu
    /// d'un `<script type="importmap">`, à pousser **avant** le
    /// premier `eval_module` de la page. Les bare specifiers
    /// (`import 'react'`) sont alors réécrits avant d'atteindre
    /// le `resolve` de [`Self::eval_module`]. Défaut : noop
    /// (moteur sans modules).
    fn set_import_map(&mut self, _json: &str) {}

    /// Évalue un module ES (étape 50) : `entry` est le nom
    /// canonique du module d'entrée (URL résolue — ou nom
    /// synthétique aliasé pour un module inline), `sources` la
    /// map `url → source` **préfetchée** par la session (graphe
    /// statique via `explorer_jscore::module_specifiers`), `resolve` la
    /// résolution `(specifier, referrer) → url`. Le chargeur ne
    /// fait jamais de fetch : un `import` hors map est une erreur
    /// JS propre — le canal reste à sens unique, jamais de
    /// ré-entrance réseau depuis le thread moteur.
    /// Défaut : `Ok("undefined")` (moteur sans modules).
    ///
    /// # Errors
    /// Mêmes variantes que [`Self::eval`] — `Syntax` à l'analyse
    /// (y compris erreur de liaison), `Uncaught` si un corps de
    /// module jette, `Dead` si le moteur est inerte.
    fn eval_module(
        &mut self,
        _entry: &str,
        _sources: HashMap<String, String>,
        _resolve: UrlResolver,
    ) -> Result<String, JsError> {
        Ok("undefined".to_string())
    }

    /// Dispatche l'événement `ty` au nœud miroir désigné par
    /// `path` — chemin depuis la racine du document, chaque
    /// étape = `(tag, index parmi les frères éléments de même
    /// tag)`. Retourne `defaultPrevented`. Défaut : `Ok(false)`
    /// (pas de miroir → rien à dispatcher).
    ///
    /// **Chemin vide** = événement niveau document (étape 52) :
    /// la cible est `<body>` — `scroll`, `keydown`/`keyup`
    /// sans champ focalisé, `wheel` hors survol.
    ///
    /// Le chemin tolère les divergences de parse : les balises
    /// implicitement synthétisées par un tree-builder
    /// (`html`, `head`, `body`, `tbody`…) sont sautées si
    /// absentes du miroir.
    ///
    /// # Errors
    /// Erreur de listener JS propagée en [`JsError::Uncaught`].
    fn dispatch_event(&mut self, _ty: &str, _path: &[(String, u32)]) -> Result<bool, JsError> {
        Ok(false)
    }

    /// Variante à props de [`Self::dispatch_event`] (étape 103) :
    /// l'événement porte `props` (`KeyboardEvent` : `key`/`code`/
    /// `keyCode`/`which` calculés par la session). Défaut :
    /// délègue à `dispatch_event` — les props sont perdues sur
    /// un moteur qui ne les implémente pas, jamais de casse.
    ///
    /// # Errors
    /// Erreur de listener JS propagée en [`JsError::Uncaught`].
    fn dispatch_event_props(
        &mut self,
        ty: &str,
        path: &[(String, u32)],
        _props: &[(String, EventProp)],
    ) -> Result<bool, JsError> {
        self.dispatch_event(ty, path)
    }

    /// Borne le budget d'opérations de la prochaine évaluation
    /// (protection anti-script-infini — règle « budgets »).
    fn set_budget(&mut self, ops: u64);

    /// Borne la mémoire du heap JS en octets estimés
    /// (`Engine::set_mem_limit`, JS-C37/ADR-0019) — `0` =
    /// illimité explicite. Appliquée avant `install_dom` à la
    /// création du contexte (étape 123 : réglage de vol pour
    /// l'outillage diagnostic, pas une promesse produit — un
    /// `PoW` hostile-by-design reste hors de portée de
    /// l'interpréteur quel que soit le plafond). Défaut :
    /// no-op (moteur sans borne exposée).
    fn set_mem_limit(&mut self, _bytes: usize) {}

    /// Draine les requêtes réseau émises depuis le dernier
    /// appel — `xhr.send()`, `fetch()`, `abort()` (JS-C27).
    /// À appeler aux mêmes points de synchro que
    /// [`Self::drain_nav_requests`] : les handlers livrés par
    /// [`Self::deliver_net_response`] peuvent en émettre de
    /// nouvelles. Défaut : rien.
    fn drain_net_requests(&mut self) -> Vec<NetRequest> {
        Vec::new()
    }

    /// Livre la réponse d'une requête drainée : `readyState` →
    /// `done`, handlers `readystatechange`/`load`/`error` et
    /// résolution de promesse `fetch()` tirent ici, en
    /// synchrone moteur (les microtasks sont vidées au retour —
    /// [`Self::run_pending`] reste le point de pompe).
    /// Défaut : `Ok(())` (moteur sans réseau).
    ///
    /// # Errors
    /// [`JsError::Uncaught`] si un handler jette,
    /// [`JsError::Dead`] si le moteur est inerte.
    fn deliver_net_response(&mut self, _resp: NetResponse) -> Result<(), JsError> {
        Ok(())
    }

    /// Livre le résultat d'une `NetRequest::Crypto` drainée :
    /// la `Promise` `subtle.digest` se résout en `ArrayBuffer`
    /// ou rejette `OperationError`. Défaut : `Ok(())` (moteur
    /// sans backend crypto).
    ///
    /// # Errors
    /// [`JsError::Uncaught`] si un handler `then` jette,
    /// [`JsError::Dead`] si le moteur est inerte.
    fn deliver_crypto_response(&mut self, _resp: CryptoResponse) -> Result<(), JsError> {
        Ok(())
    }

    /// Drain des `NetRequest::WorkerLoad` (JS-C132) — file
    /// `Send` distincte du drain `Rc` de [`Self::drain_net_requests`] :
    /// le thread moteur peut y être bloqué en `load()` — ce
    /// drain ne transite **pas** par une commande (il doit
    /// rester instantané pour débloquer le demandeur).
    /// À appeler **avant** `drain_net_requests`. Défaut : vide.
    fn drain_worker_requests(&mut self) -> Vec<NetRequest> {
        Vec::new()
    }

    /// Livre un `EsEvent` (`open`/`message`/`error`) au flux
    /// SSE `handle` du contexte `ctx` (JS-C92). Défaut :
    /// `Ok(())` (moteur sans transport SSE).
    ///
    /// # Errors
    /// [`JsError::Uncaught`] si un handler `on*` jette,
    /// [`JsError::Dead`] si le moteur est inerte.
    fn deliver_es_event(
        &mut self,
        _ctx: JsCtxId,
        _handle: u64,
        _ev: explorer_jscore::EsEvent,
    ) -> Result<(), JsError> {
        Ok(())
    }

    /// Livraison d'un événement `WebSocket` (JS-C94) : `ctx` =
    /// realm émetteur, `handle` = flux, `ev` =
    /// `Open`/`Text`/`Binary`/`Close`/`Error`. Multi-coups —
    /// `Close` = fin de flux. Défaut : `Ok(())` (moteur sans
    /// transport WS).
    ///
    /// # Errors
    /// [`JsError::Uncaught`] si un handler `on*` jette,
    /// [`JsError::Dead`] si le moteur est inerte.
    fn deliver_ws_event(
        &mut self,
        _ctx: JsCtxId,
        _handle: u64,
        _ev: explorer_jscore::WsEventKind,
    ) -> Result<(), JsError> {
        Ok(())
    }

    /// Installe `localStorage`/`sessionStorage` : `local` est le
    /// snapshot **par origine** de la session (le moteur y écrit
    /// en direct — relire via [`Self::drain_storage`]),
    /// `sessionStorage` est frais par contexte. Défaut : pas de
    /// storage globals.
    fn install_storage(&mut self, _local: std::collections::BTreeMap<String, String>) {}

    /// Extrait le contenu `localStorage` courant — la session
    /// le persiste par origine. Défaut : map vide.
    fn drain_storage(&mut self) -> std::collections::BTreeMap<String, String> {
        std::collections::BTreeMap::new()
    }

    /// Restaure un snapshot `IndexedDB` (JSON —
    /// [`Engine::idb_restore`], PR-10a). Défaut : noop.
    fn install_idb(&mut self, _json: &str) {}

    /// Snapshot JSON du registre `IndexedDB` courant — la
    /// session le persiste par origine (`idb-<slug>.json`).
    /// Défaut : chaîne vide.
    fn drain_idb(&mut self) -> String {
        String::new()
    }

    /// Restaure un snapshot `IndexedDB` dans le realm du ctx
    /// `ctx` (frame — PR-10d isolation par contexte).
    /// Défaut : noop.
    fn install_idb_ctx(&mut self, _ctx: JsCtxId, _json: &str) {}

    /// Snapshot `IndexedDB` du realm de `ctx` — persisté par
    /// l'origine **de l'enfant** (pas du top-level).
    /// Défaut : chaîne vide.
    fn drain_idb_ctx(&mut self, _ctx: JsCtxId) -> String {
        String::new()
    }

    /// Pose le snapshot `document.cookie` : fragment
    /// `« k=v; k2=v2 »` pré-filtré par la session pour l'origine
    /// de la page (le vrai jar applique HttpOnly/Secure/SameSite
    /// — le moteur ne voit que la vue script). Défaut : noop.
    fn set_cookie_snapshot(&mut self, _header: &str) {}

    /// Draine les écritures `document.cookie =` brutes
    /// (`« k=v; attrs… »`) — la session les soumet à son jar
    /// après avoir retiré `HttpOnly` (non posable par script).
    /// Défaut : rien.
    fn drain_cookie_writes(&mut self) -> Vec<String> {
        Vec::new()
    }

    /// Draine les surfaces script **du contexte `ctx`**
    /// (étape 134) : `(écritures document.cookie brutes,
    /// contenu localStorage courant)` — la session les range
    /// sous l'origine **de l'enfant** (partition de la page
    /// top-level pour les cookies — `Partitioned` se clé sur
    /// le site de premier niveau). Contexte inconnu → défaut
    /// vide. Défaut : rien (moteur mono-contexte).
    fn drain_sinks_ctx(
        &mut self,
        _ctx: JsCtxId,
    ) -> (Vec<String>, std::collections::BTreeMap<String, String>) {
        (Vec::new(), std::collections::BTreeMap::new())
    }

    /// Marque l'élément média `path` claimé par l'hôte
    /// (`Some(codecs)`, id de surface `media`) ou releasé
    /// (`None` — retour aux stubs honnêtes ADR-0023) — étape
    /// 71c, contrat `MediaSurface`. L'id `media` est rappelé
    /// dans les commandes drainées (l'identité de surface est
    /// côté session, le chemin sert juste à résoudre le nœud
    /// miroir une fois). `url` = URL média de la surface
    /// (`src` résolue ou `blob:`) — identité de repli quand
    /// le path diverge du miroir (vol 176-ter : le JS peut
    /// réordonner le miroir plus vite que les applies ne
    /// resynchronisent le réel → path frais obsolète).
    /// Défaut : rien (pas de surface média).
    fn set_media_claim(
        &mut self,
        _path: &[(String, u32)],
        _url: &str,
        _media: u64,
        _codecs: Option<Vec<String>>,
    ) {
    }

    /// Relaie le rapport `state` de l'hôte claimé vers
    /// l'élément `path` (étape 71c) — props refletées,
    /// promesses `play()` tranchées, events DOM tirés.
    /// Rapport sur élément non claimé/mort → ignoré.
    /// `media` = id de surface session — permet de retrouver
    /// le nœud déjà résolu au claim sans re-tenter un path
    /// obsolète. Défaut : rien.
    fn deliver_media_state(
        &mut self,
        _path: &[(String, u32)],
        _media: u64,
        _report: JsMediaReport,
    ) {
    }

    /// Draine les ordres média émis par la page depuis le
    /// dernier appel (`play()`/`pause()`/seek/volume/rate
    /// sur élément claimé — étape 71c). La session les émet
    /// en `CoreEvent::MediaCommand` vers l'hôte. Défaut :
    /// rien (surface jamais claimée).
    fn drain_media_commands(&mut self) -> Vec<JsMediaCommand> {
        Vec::new()
    }
}

/// Fabrique de contextes — une instance par document chargé.
pub type JsFactory = std::sync::Arc<dyn Fn() -> Box<dyn JsEngine> + Send + Sync>;

/// Moteur nul : évalue en succès `"undefined"` — navigation sans
/// JavaScript (défaut, tests, `explorer-jscore` indisponible).
#[derive(Debug, Default)]
pub struct NullEngine;

impl JsEngine for NullEngine {
    fn eval(&mut self, _source: &str) -> Result<String, JsError> {
        Ok("undefined".to_string())
    }

    fn set_budget(&mut self, _ops: u64) {}
}

/// Fabrique de [`NullEngine`].
#[must_use]
pub fn null_factory() -> JsFactory {
    std::sync::Arc::new(|| Box::new(NullEngine))
}

pub use explorer_jscore::module_specifiers;
pub use explorer_jscore::MediaEnv;
// Issue #19 / JS-8 — table de géométrie session↔moteur
// (ré-export : le type reste unique — la session construit et
// remplit, le moteur lit).
pub use explorer_jscore::{is_bare_specifier, rewrite_bare_specifier};
pub use explorer_jscore::{ElementGeom, GeoRect, GeomPosition, GeometryMap, GeometrySlot};
// JS-C77 — `Request.mode`/`Request.credentials` transportés par
// `NetRequest::Fetch` (ré-export : le type reste unique).
/// Réémis : requête de `<iframe>` dynamique (CAP-171) —
/// drainée par [`JsEngine::drain_dyn_frames`].
pub use explorer_jscore::DynFrameReq;
/// Réémis : requête de `<img>` dynamique (CAP-172) —
/// drainée par [`JsEngine::drain_dyn_imgs`].
pub use explorer_jscore::DynImgReq;
/// `import()` dynamique à fetcher (JS-C100) — drainé par la
/// session, livré via `deliver_dyn_import`.
pub use explorer_jscore::DynImportReq;
/// Réémis : requête de `<link>` dynamique (CAP-173) —
/// drainée par [`JsEngine::drain_dyn_links`].
pub use explorer_jscore::DynLinkReq;
/// Réémis : requête de `<script>` dynamique (CAP-169) —
/// drainée par [`JsEngine::drain_dyn_scripts`].
pub use explorer_jscore::DynScriptReq;
/// Réémis : événement `EventSource` livré par la session
/// (JS-C92) — `open`/`message`/`error`.
pub use explorer_jscore::EsEvent;
pub use explorer_jscore::{FetchCredentials, FetchMode};

/// Contexte de navigation du tas moteur (étape 134 — JS-C78
/// côté `explorer-jscore`) : un realm par document chargé ;
/// `TOP` = le document top-level, les `<iframe>` fetchés en
/// reçoivent chacun un au commit. L'identifiant n'a de sens
/// que dans une session moteur — il n'est ni persisté ni
/// comparable entre moteurs.
pub type JsCtxId = u32;

/// Extension de [`JsCtxId`] : le contexte top-level.
pub const JS_CTX_TOP: JsCtxId = 0;
// JS-C79 — opération `crypto.subtle` transportée par
// `NetRequest::Crypto` (ré-export : le type reste unique).
// JS-C93 — `KeyAlgo`/`ImportFormat` pour les ops à clé.
pub use explorer_jscore::{CryptoOp, DigestAlgo, ImportFormat, KeyAlgo};
// JS-C94 — requêtes/événements `WebSocket` transportés par
// `NetRequest::WsOpen`/`WsSend`/`WsClose` (types uniques).
pub use explorer_jscore::{WsEventKind, WsOut, WsRequest};

/// Commandes vers le thread moteur — valeurs sérialisables, aucun
/// handle `Rc` ne traverse.
/// Traduit les [`explorer_jscore::JournalOp`] miroir en
/// [`DomMutation`] de couture (ADR-0025, étape 111) :
/// payloads matérialisés **au drain** (sous-arbres
/// sérialisés cléés, valeurs courantes d'attribut/texte).
/// `None` = journal classé **non rejouable** — contextes
/// dont le re-parse dépend du parent (foster parenting
/// `<table>`, contenu `<select>`/`<template>`) ou contenu à
/// re-parcours contextuel ; l'appelant retombe sur la
/// synchro par sérialisation (garde-fou auto-réparant).
fn journal_ops_to_seam(
    d: &explorer_jscore::Dom,
    ops: Vec<explorer_jscore::JournalOp>,
) -> Option<Vec<DomMutation>> {
    /// Tags parents dont le re-parse dépend du contexte
    /// (foster parenting, groupes `<select>`, contenu
    /// `<template>`, collections `head`/`html`).
    const RISKY_PARENT: &[&str] = &[
        "table", "thead", "tbody", "tfoot", "tr", "colgroup", "select", "template", "frameset",
        "head", "html",
    ];
    /// Fragments de balisage dont la re-sérialisation se
    /// re-parse différemment hors contexte.
    const RISKY_CONTENT: &[&str] = &[
        "<table",
        "<thead",
        "<tbody",
        "<tfoot",
        "<tr",
        "<td",
        "<th",
        "<col",
        "<select",
        "<option",
        "<template",
        "<script",
        "<frameset",
        "<frame",
    ];
    let tag_of = |id: u32| -> &str { d.nodes.get(id as usize).map_or("", |n| n.tag.as_str()) };
    let mut out = Vec::with_capacity(ops.len());
    for op in ops {
        match op {
            explorer_jscore::JournalOp::Insert {
                parent,
                index,
                added,
            } => {
                if RISKY_PARENT.contains(&tag_of(parent)) {
                    return None;
                }
                let mut html = String::new();
                let mut ids = Vec::new();
                let mut tags = Vec::new();
                for &a in &added {
                    let (h, i, t) = d.serialize_subtree_keyed(a);
                    html.push_str(&h);
                    ids.extend(i);
                    tags.extend(t);
                }
                if RISKY_CONTENT.iter().any(|t| html.contains(t)) {
                    return None;
                }
                out.push(DomMutation::Insert {
                    parent: u64::from(parent),
                    index,
                    html,
                    ids,
                    tags,
                });
            }
            explorer_jscore::JournalOp::Remove { key } => {
                out.push(DomMutation::Remove {
                    key: u64::from(key),
                });
            }
            explorer_jscore::JournalOp::SetAttr { key, name } => {
                if RISKY_PARENT.contains(&tag_of(key)) {
                    return None;
                }
                out.push(DomMutation::SetAttr {
                    key: u64::from(key),
                    value: d.attr(key, &name).map(str::to_string),
                    name,
                });
            }
            explorer_jscore::JournalOp::SetText { key } => {
                let n = d.nodes.get(key as usize)?;
                out.push(DomMutation::SetText {
                    key: u64::from(key),
                    text: n.text.clone(),
                });
            }
            explorer_jscore::JournalOp::AttachShadow { host, root, open } => {
                out.push(DomMutation::AttachShadow {
                    host: u64::from(host),
                    root: u64::from(root),
                    open,
                });
            }
        }
    }
    Some(out)
}

enum Cmd {
    Eval {
        src: String,
        /// Étiquette de source (`JsEngine::eval_named`) —
        /// URL du script externe, `None` = script inline.
        name: Option<String>,
        tx: Sender<Result<String, JsError>>,
    },
    RunPending {
        /// Borne de timers par pompe (ADR-0019).
        max_timers: usize,
        /// Typé : une mort deadline de la rafale remonte
        /// `JsError::Timeout` → disjoncteur moteur figé
        /// (allrecipes — auparavant `unwrap_or(0)` avalé).
        tx: Sender<Result<usize, JsError>>,
    },
    TimerDeadline {
        tx: Sender<Option<std::time::Duration>>,
    },
    /// `url` enregistrée `kind=mse` ? (MSE-2 — scan média,
    /// ADR-0029).
    IsMediaSource {
        url: String,
        tx: Sender<bool>,
    },
    /// Pull segments du `SourceBuffer` `buffer` (MSE-3
    /// transport, ADR-0029) — `(octets, curseur, eos)`.
    MsePull {
        url: String,
        buffer: usize,
        max: usize,
        tx: Sender<Option<(Vec<u8>, u64, bool)>>,
    },
    /// MIME des `SourceBuffer` sous `url` (`?buffer=N` indexe
    /// ce tableau — offre `mseBuffers`, P-53 multi-source).
    MseBuffers {
        url: String,
        tx: Sender<Vec<String>>,
    },
    /// `timestampOffset` des `SourceBuffer` sous `url` —
    /// même ordre que `MseBuffers` (`?buffer=N`).
    MseOffsets {
        url: String,
        tx: Sender<Vec<f64>>,
    },
    /// `(promises créées, réglées)` — compteur §B-4(c).
    PromiseStats {
        tx: Sender<(u64, u64)>,
    },
    /// `(timers en attente, microtasks en file, readyState)` —
    /// instantané §B-4(c).
    BootDiag {
        tx: Sender<(usize, usize, &'static str)>,
    },
    SyncField {
        path: Vec<(String, u32)>,
        attr: String,
        value: Option<String>,
    },
    InstallDom {
        html: String,
        page_url: String,
        history_len: u32,
    },
    DrainNav {
        tx: Sender<Vec<NavRequest>>,
    },
    /// Resynchro `location`/`history.length` d'un contexte
    /// après un `pushState`/`replaceState` committé par la
    /// session (CAP-151).
    SyncCtxUrl {
        ctx: JsCtxId,
        url: String,
        history_len: u32,
    },
    SyncCtxState {
        ctx: JsCtxId,
        url: String,
        history_len: u32,
        state: Option<String>,
        popstate: bool,
    },
    SetCtxName {
        ctx: JsCtxId,
        name: String,
    },
    FireHashchange {
        ctx: JsCtxId,
        old: String,
        new: String,
    },
    /// `document.readyState` du contexte `ctx` (CAP-170).
    SetReadyState {
        ctx: JsCtxId,
        state: String,
    },
    SetMediaEnv {
        env: explorer_jscore::MediaEnv,
    },
    /// Viewport CSS px poussé au moteur (CAP-161) —
    /// `innerWidth`/`screen`/`matchMedia` en dépendent.
    SetViewport {
        w: u32,
        h: u32,
    },
    /// Table de géométrie publiée (issue #19, JS-8) —
    /// posée sur le document du contexte `ctx`
    /// (`Dom::geometry` — document-local, journal interface).
    /// `getBoundingClientRect`/`offset*`/`client*`/`scroll*`
    /// la lisent ; la session la remplit au commit paint.
    SetGeometry {
        ctx: JsCtxId,
        slot: explorer_jscore::GeometrySlot,
    },
    /// `document.referrer` du document top-level (CAP-156).
    SetPageReferrer {
        referrer: String,
    },
    DomSync {
        tx: Sender<Option<DomSyncPayload>>,
    },
    DomSyncKeyed {
        /// `true` = sérialiser sans égard au bit sale (repli
        /// post-rejet de journal : le dirty a été consommé par
        /// le drain, la baseline doit être re-livrée quand même).
        force: bool,
        tx: Sender<Option<DomSyncKeyed>>,
    },
    DomJournal {
        tx: Sender<Option<DomJournal>>,
    },
    DomSyncStats {
        tx: Sender<Option<DomSyncStats>>,
    },
    HeapStats {
        tx: Sender<HeapStats>,
    },
    /// Ligne diag réservoirs internes (§B-59 — `mem_diag`
    /// session) — `Engine::b59_diag` côté worker.
    DiagLine {
        tx: Sender<String>,
    },
    /// Passe GC forcée (T3) — sans réponse, la file ordonne.
    Gc,
    CanvasOps {
        tx: Sender<CanvasOpsTable>,
    },
    EvalModule {
        entry: String,
        sources: HashMap<String, String>,
        resolve: UrlResolver,
        tx: Sender<Result<String, JsError>>,
    },
    SetImportMap {
        json: String,
    },
    Dispatch {
        ty: String,
        path: Vec<(String, u32)>,
        /// Props plateforme de l'event (`key`, `code`, `keyCode`…)
        /// — vide hors événements à payload (étape 103).
        props: Vec<(String, EventProp)>,
        tx: Sender<Result<bool, JsError>>,
    },
    SetMediaClaim {
        path: Vec<(String, u32)>,
        url: String,
        media: u64,
        codecs: Option<Vec<String>>,
    },
    DeliverMediaState {
        path: Vec<(String, u32)>,
        media: u64,
        report: JsMediaReport,
    },
    DrainMediaCommands {
        tx: Sender<Vec<JsMediaCommand>>,
    },
    SetBudget {
        ops: u64,
    },
    SetMemLimit {
        bytes: usize,
    },
    DrainNet {
        tx: Sender<Vec<NetRequest>>,
    },
    DeliverNet {
        resp: NetResponse,
        tx: Sender<Result<(), JsError>>,
    },
    DeliverCrypto {
        resp: CryptoResponse,
        tx: Sender<Result<(), JsError>>,
    },
    DeliverEs {
        ctx: JsCtxId,
        handle: u64,
        ev: explorer_jscore::EsEvent,
        tx: Sender<Result<(), JsError>>,
    },
    DeliverWs {
        ctx: JsCtxId,
        handle: u64,
        ev: explorer_jscore::WsEventKind,
        tx: Sender<Result<(), JsError>>,
    },
    InstallStorage {
        local: std::collections::BTreeMap<String, String>,
    },
    DrainStorage {
        tx: Sender<std::collections::BTreeMap<String, String>>,
    },
    InstallIdb {
        json: String,
    },
    DrainIdb {
        tx: Sender<String>,
    },
    InstallIdbCtx {
        ctx: JsCtxId,
        json: String,
    },
    DrainIdbCtx {
        ctx: JsCtxId,
        tx: Sender<String>,
    },
    CookieSnap {
        header: String,
    },
    DrainCookies {
        tx: Sender<Vec<String>>,
    },
    /// Étape 134 — contextes de navigation imbriqués (un
    /// `<iframe>` fetché = un realm). `frame_ord` = position
    /// de l'élément parmi les `<iframe>` du parent (ordre
    /// document) — le worker la résout en `NodeId` miroir.
    ContextNew {
        url: String,
        frame_ord: Option<u32>,
        tx: Sender<Option<JsCtxId>>,
    },
    /// CAP-171 — `context_new` sous un parent explicite :
    /// `frame_ord` est résolu dans le miroir du contexte
    /// `parent` (un iframe inséré dans un sous-contexte ne
    /// lit jamais le miroir du top-level).
    ContextNewIn {
        parent: JsCtxId,
        url: String,
        frame_ord: Option<u32>,
        tx: Sender<Option<JsCtxId>>,
    },
    ContextDestroy {
        ctx: JsCtxId,
    },
    InstallDomCtx {
        ctx: JsCtxId,
        html: String,
        page_url: String,
        /// localStorage de l'origine enfant (isolée).
        local: std::collections::BTreeMap<String, String>,
        /// Vue `document.cookie` de l'enfant (jar isolé).
        cookie_header: String,
    },
    /// Drain des surfaces script du contexte `ctx` (étape
    /// 134) : écritures `document.cookie` brutes + contenu
    /// `localStorage` courant — la session les range sous
    /// l'origine **de l'enfant** (jamais celle du parent).
    DrainSinksCtx {
        ctx: JsCtxId,
        tx: Sender<(Vec<String>, std::collections::BTreeMap<String, String>)>,
    },
    EvalCtx {
        ctx: JsCtxId,
        src: String,
        name: Option<String>,
        tx: Sender<Result<String, JsError>>,
    },
    EvalScriptEl {
        ctx: JsCtxId,
        src: String,
        name: Option<String>,
        ord: usize,
        tx: Sender<Result<String, JsError>>,
    },
    DrainDynImgs {
        tx: Sender<Vec<explorer_jscore::DynImgReq>>,
    },
    DrainDynLinks {
        tx: Sender<Vec<explorer_jscore::DynLinkReq>>,
    },
    AdoptedSheetsCss {
        tx: Sender<Vec<String>>,
    },
    DefinedCeNames {
        tx: Sender<Vec<String>>,
    },
    DrainDynScripts {
        tx: Sender<Vec<explorer_jscore::DynScriptReq>>,
    },
    /// `unhandledrejection` tirés (JS-C154) — descriptions
    /// vers le ring console de la session.
    DrainUnhandledRejections {
        tx: Sender<Vec<String>>,
    },
    DeliverDynScript {
        ctx: JsCtxId,
        seq: u64,
        body: Option<String>,
    },
    DrainDynFrames {
        tx: Sender<Vec<explorer_jscore::DynFrameReq>>,
    },
    DeliverDynFrame {
        ctx: JsCtxId,
        seq: u64,
        ok: bool,
    },
    DeliverDynImg {
        ctx: JsCtxId,
        seq: u64,
        ok: bool,
    },
    DeliverDynLink {
        ctx: JsCtxId,
        seq: u64,
        ok: bool,
    },
    DrainDynImports {
        tx: Sender<Vec<explorer_jscore::DynImportReq>>,
    },
    DeliverDynImport {
        ctx: JsCtxId,
        seq: u64,
        src: Option<String>,
        deps: Vec<(String, String)>,
    },
    DeliverStaticLink {
        ctx: JsCtxId,
        ord: u32,
        ok: bool,
    },
    DeliverDynModule {
        ctx: JsCtxId,
        seq: u64,
        ok: bool,
    },
    DispatchCtx {
        ctx: JsCtxId,
        ty: String,
        path: Vec<(String, u32)>,
        tx: Sender<Result<bool, JsError>>,
    },
    DomSyncCtx {
        ctx: JsCtxId,
        tx: Sender<Option<String>>,
    },
    Shutdown,
}

/// Adaptateur `explorer-jscore` : un thread par contexte (le `Rc` du tas
/// ne quitte jamais son thread), le `JsEngine` n'est qu'un canal.
pub struct JscoreEngine {
    tx: Sender<Cmd>,
    join: Option<JoinHandle<()>>,
    /// File `Send` des `WorkerLoad` (JS-C132) — alimentée
    /// depuis le thread moteur **et** les threads worker
    /// (là où `Rc<RefCell>` ne passe pas) ; drainée par
    /// `drain_net_requests` sans transiter par `Cmd` —
    /// le thread moteur peut y être bloqué en `load()`.
    worker_rx: std::sync::mpsc::Receiver<NetRequest>,
}

impl JscoreEngine {
    /// Crée un contexte neuf — spawn le thread moteur.
    #[must_use]
    pub fn new() -> Self {
        let (tx, rx) = channel::<Cmd>();
        let (wtx, wrx) = std::sync::mpsc::channel::<NetRequest>();
        let join = std::thread::Builder::new()
            .name("explorer-jscore".to_string())
            // Pile large : le parseur/évaluateur descendant coûte
            // ~8–55 Kio natif par niveau d'emboîtement/appel JS
            // (JS-C41, mesuré). 32 Mio couvre les usages réels sans
            // recourir aux segments tas de `maybe_grow` ; les
            // plafonds logiques sont relevés dans `worker`.
            .stack_size(32 * 1024 * 1024)
            .spawn(move || Self::worker(&rx, &wtx));
        match join {
            Ok(j) => Self {
                tx,
                join: Some(j),
                worker_rx: wrx,
            },
            // Thread impossible (OS saturé) : moteur inerte —
            // `eval` répondra `Dead`, jamais de panic.
            Err(_) => Self {
                tx,
                join: None,
                worker_rx: wrx,
            },
        }
    }

    /// `Eval` sur le canal — `name` étiquette la source pour
    /// Borne de réponse du thread moteur : un éval qui tourne
    /// dans le fil (spin) ne doit pas figer la session —
    ///  → erreur nommée sur les chemins d’éval,
    /// repli par défaut sur les drains (JS-C105).
    const EVAL_CMD_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
    /// Évaluations de programmes (B-23 — lemonde) : le
    /// budget boot moteur est 60 s (B-22) — un canal à 30 s
    /// tuait les compiles légitimes avant leur deadline
    /// interne (stall → disjoncteur muet). Pompes/drains
    /// restent à 30 s.
    const EVAL_CMD_TIMEOUT_EVAL: std::time::Duration = std::time::Duration::from_secs(65);

    fn recv_eval<T>(rx: &Receiver<T>) -> Result<T, JsError> {
        rx.recv_timeout(Self::EVAL_CMD_TIMEOUT)
            .map_err(|e| match e {
                RecvTimeoutError::Timeout => JsError::Timeout,
                RecvTimeoutError::Disconnected => JsError::Dead,
            })
    }

    /// les frames `error.stack` (`JsEngine::eval_named`).
    fn recv_eval_long<T>(rx: &Receiver<T>) -> Result<T, JsError> {
        rx.recv_timeout(Self::EVAL_CMD_TIMEOUT_EVAL)
            .map_err(|e| match e {
                RecvTimeoutError::Timeout => JsError::Timeout,
                RecvTimeoutError::Disconnected => JsError::Dead,
            })
    }

    fn eval_cmd(&mut self, source: &str, name: Option<&str>) -> Result<String, JsError> {
        let (tx, rx) = channel();
        if self
            .tx
            .send(Cmd::Eval {
                src: source.to_string(),
                name: name.map(str::to_string),
                tx,
            })
            .is_err()
        {
            return Err(JsError::Dead);
        }
        Self::recv_eval_long(&rx)?
    }

    /// Boucle du thread : possède le `explorer_jscore::Engine` (`!Send`)
    /// et sert les commandes jusqu'à `Shutdown` ou fermeture du
    /// canal (le `Sender` meurt avec le proxy).
    #[allow(clippy::too_many_lines)] // dispatch de commandes — grandir ne gagne rien
    fn worker(rx: &Receiver<Cmd>, wtx: &std::sync::mpsc::Sender<NetRequest>) {
        let mut eng = explorer_jscore::Engine::new();
        // JS-C100 — la session draine `import()` manquants
        // (`drain_dyn_imports` → fetch → `deliver_dyn_import`).
        eng.set_import_drain();
        // JS-C132 — `new Worker`/`importScripts` : fabrique de
        // loaders pontés vers la session (canal `Send` dédié,
        // réponse directe — jamais d'I/O dans le moteur).
        let wfac = wtx.clone();
        eng.set_worker_factory(std::sync::Arc::new(move || {
            Box::new(ChanWorker { tx: wfac.clone() }) as Box<dyn explorer_jscore::WorkerLoader>
        }));
        // Plafonds logiques relevés pour la pile de 32 Mio du
        // thread moteur (JS-C41) : la pile est aussi protégée par
        // `maybe_grow`, ces bornes ne fixent que le `RangeError`
        // — ~1024 appels/parse niveaux, proche des usages réels.
        eng.set_call_depth_limit(1024);
        eng.set_nest_limit(1024);
        // Miroir DOM installé par `install_dom` (`Rc` — vit et
        // meurt sur ce thread). Le `Dom` y reste mutable par les
        // bindings ; `DomSync` lit le dirty bit et sérialise.
        let mut jsd: Option<explorer_jscore::DomRef> = None;
        // `media id → nœud miroir` des surfaces claimées :
        // le claim résout le nœud une fois (path, repli url)
        // et les rapports `state` suivants le retrouvent sans
        // re-tenter un path déjà obsolète (vol 176-ter).
        let mut media_nodes: HashMap<u64, explorer_jscore::NodeId> = HashMap::new();
        // Intentions de navigation script (`location`,
        // `history`) — hooks `Rc` du thread moteur, drainées
        // par `DrainNav` (jamais de navigation re-entrante).
        let nav_q: std::rc::Rc<std::cell::RefCell<Vec<NavRequest>>> =
            std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        // Requêtes XHR/fetch émises par les scripts — `ChanXhr`
        // (`Rc` thread-local) alimente, `DrainNet` vide.
        let net_q: std::rc::Rc<std::cell::RefCell<Vec<NetRequest>>> =
            std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        // Jar `document.cookie` : snapshot session + miroir des
        // écritures — drainé pour le vrai jar (politiques dans
        // `explorer-net`, jamais ici).
        let jar = std::rc::Rc::new(SnapshotJar::default());
        // Backing `localStorage` posé par `InstallStorage` — le
        // moteur y écrit en direct, `DrainStorage` le relit.
        let mut local_store: Option<explorer_jscore::Storage> = None;
        // Miroirs des contextes `<iframe>` (étape 134) —
        // `ctx → DomRef` ; même durée de vie que le contexte
        // (détruits ensemble). Chaque cadre porte aussi son
        // jar `document.cookie` **isolé** — le `SnapshotJar`
        // top-level ne doit jamais absorber l'écriture d'un
        // contexte tiers (fuite cross-origin).
        let mut ctx_doms: HashMap<JsCtxId, explorer_jscore::DomRef> = HashMap::new();
        let mut ctx_jars: HashMap<JsCtxId, std::rc::Rc<SnapshotJar>> = HashMap::new();
        // Backing `localStorage` de chaque contexte — le moteur
        // y écrit en direct, `DrainSinksCtx` le relit pour la
        // session (rangé sous l'origine de l'enfant).
        let mut ctx_stores: HashMap<JsCtxId, explorer_jscore::Storage> = HashMap::new();
        // Compteur de handles XHR/fetch **partagé entre les
        // contextes** — un handle unique dans le moteur rend
        // `NetRequest::Abort { handle }` non ambigu sans tag
        // ctx (le drain session ne connaît que le handle).
        // Idem pour `crypto.subtle` (JS-C79).
        let xhr_next: std::rc::Rc<std::cell::Cell<u64>> = std::rc::Rc::new(std::cell::Cell::new(0));
        let crypto_next: std::rc::Rc<std::cell::Cell<u64>> =
            std::rc::Rc::new(std::cell::Cell::new(0));
        while let Ok(cmd) = rx.recv() {
            let mut stopped = false;
            {
                // Confinement du panic au job (JS-C251) : un
                // panic interne jscore (capacity overflow,
                // borne) tuait le thread → chaque commande
                // suivante `Dead` (« moteur js inerte »),
                // alors que le burst §B-59 rend la mémoire
                // après le pic. Désormais le job meurt seul
                // (son `tx` droppé → `Dead` borné côté client),
                // le hook panic écrit site+payload au
                // death_log, et les jobs suivants vivent.
                let eng = &mut eng;
                let stopped = &mut stopped;
                let jsd = &mut jsd;
                let ctx_doms = &mut ctx_doms;
                let ctx_jars = &mut ctx_jars;
                let ctx_stores = &mut ctx_stores;
                let local_store = &mut local_store;
                let media_nodes = &mut media_nodes;
                let net_q = &net_q;
                let nav_q = &nav_q;
                let jar = &jar;
                let xhr_next = &xhr_next;
                let crypto_next = &crypto_next;
                let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || match cmd {
                    Cmd::Eval { src, name, tx } => {
                        let res = match &name {
                            Some(n) => eng.eval_named(&src, n),
                            None => eng.eval(&src),
                        };
                        let out = match res {
                            Ok(v) => Ok(eng.describe(v)),
                            Err(e) => Err(JsError::from(e)),
                        };
                        let _ = tx.send(out);
                    }
                    Cmd::RunPending { max_timers, tx } => {
                        let r = eng
                            .run_due_timers_bounded(max_timers)
                            .and_then(|n| eng.run_microtasks().map(|m| n + m))
                            .map_err(|e| {
                                if eng.take_deadline_hit() {
                                    JsError::Timeout
                                } else {
                                    JsError::Uncaught(eng.describe(e.0))
                                }
                            });
                        let _ = tx.send(r);
                    }
                    Cmd::TimerDeadline { tx } => {
                        let d = eng
                            .next_timer_deadline()
                            .map(|at| at.saturating_duration_since(std::time::Instant::now()));
                        let _ = tx.send(d);
                    }
                    Cmd::IsMediaSource { url, tx } => {
                        let _ = tx.send(eng.is_media_source_url(&url));
                    }
                    Cmd::MsePull {
                        url,
                        buffer,
                        max,
                        tx,
                    } => {
                        let _ = tx.send(eng.mse_pull_segments(&url, buffer, max));
                    }
                    Cmd::MseBuffers { url, tx } => {
                        let _ = tx.send(eng.mse_source_buffers(&url));
                    }
                    Cmd::MseOffsets { url, tx } => {
                        let _ = tx.send(eng.mse_buffer_offsets(&url));
                    }
                    Cmd::PromiseStats { tx } => {
                        let _ = tx.send(eng.promise_stats());
                    }
                    Cmd::BootDiag { tx } => {
                        let _ = tx.send(eng.boot_diag());
                    }
                    Cmd::SyncField { path, attr, value } => {
                        sync_field(jsd.as_ref(), &path, &attr, value.as_deref());
                    }
                    Cmd::InstallDom {
                        html,
                        page_url,
                        history_len,
                    } => {
                        // Nouveau miroir → les nœuds claimés de la
                        // map sont invalides (NodeId de l'ancien Dom).
                        media_nodes.clear();
                        *jsd = Some(install_dom(
                            eng,
                            nav_q,
                            net_q,
                            jar,
                            &html,
                            &page_url,
                            history_len,
                            xhr_next,
                            crypto_next,
                        ));
                        // ADR-0025 : l'install est la baseline —
                        // les records du parse initial n'ont rien à
                        // rejouer (le DOM commis est déjà ce
                        // document) ; sans ce clear, l'`Insert` de
                        // `<html>` sous `#document` classifierait
                        // chaque futur journal « parent risqué ».
                        if let Some(d) = jsd.as_ref() {
                            d.borrow_mut().clear_journal();
                        }
                    }
                    Cmd::DrainNav { tx } => {
                        let out = std::mem::take(&mut *nav_q.borrow_mut());
                        let _ = tx.send(out);
                    }
                    Cmd::SyncCtxUrl {
                        ctx,
                        url,
                        history_len,
                    } => {
                        eng.sync_ctx_url(ctx, &url, history_len);
                    }
                    Cmd::SyncCtxState {
                        ctx,
                        url,
                        history_len,
                        state,
                        popstate,
                    } => {
                        eng.sync_ctx_state(ctx, &url, history_len, state.as_deref(), popstate);
                    }
                    Cmd::SetCtxName { ctx, name } => {
                        eng.set_ctx_name(ctx, &name);
                    }
                    Cmd::FireHashchange { ctx, old, new } => {
                        eng.fire_hashchange(ctx, &old, &new);
                    }
                    Cmd::SetReadyState { ctx, state } => {
                        eng.set_ready_state(ctx, &state);
                    }
                    Cmd::SetMediaEnv { env } => {
                        eng.set_media_env(env);
                    }
                    Cmd::SetViewport { w, h } => {
                        eng.set_viewport(w, h);
                    }
                    Cmd::SetGeometry { ctx, slot } => {
                        eng.set_geometry_ctx(ctx, slot);
                    }
                    Cmd::SetPageReferrer { referrer } => {
                        eng.set_page_referrer(&referrer);
                    }
                    Cmd::DomSync { tx } => {
                        let out = jsd.as_ref().and_then(|d| {
                            let d = d.borrow();
                            d.take_dirty()
                                .then(|| (d.serialize_document(), d.take_dirty_path()))
                        });
                        let _ = tx.send(out);
                    }
                    // ADR-0025 : baseline cléée = sérialisation +
                    // ids miroir en pré-ordre ; elle **clôt le
                    // journal** (les ops antérieures sont subsumées).
                    Cmd::DomSyncKeyed { force, tx } => {
                        let out = jsd.as_ref().and_then(|d| {
                            let mut d = d.borrow_mut();
                            (force || d.take_dirty()).then(|| {
                                let (html, ids, tags) = d.serialize_keyed();
                                let dirty = d.take_dirty_path();
                                d.clear_journal();
                                DomSyncKeyed {
                                    html,
                                    dirty,
                                    ids,
                                    tags,
                                }
                            })
                        });
                        let _ = tx.send(out);
                    }
                    // ADR-0025 : drain du journal — `None` = « voie
                    // sérialisation » (journal vide **ou** classé
                    // non rejouable ; `dirty` n'est alors PAS
                    // consommé → l'appelant enchaîne
                    // `DomSyncKeyed`, la sérialisation complète
                    // subsume toutes les mutations).
                    Cmd::DomJournal { tx } => {
                        let out = jsd.as_ref().and_then(|d| {
                            let mut d = d.borrow_mut();
                            let ops = d.take_journal()?; // poisonné → None
                            if ops.is_empty() {
                                // Mutations sur nœuds détachés
                                // seulement — invisibles au rendu ;
                                // consommer `dirty` tout de même
                                // (l'attachement journalisera).
                                d.take_dirty();
                                d.take_dirty_path();
                                return None;
                            }
                            let n = ops.len() as u64;
                            match journal_ops_to_seam(&d, ops) {
                                None => {
                                    // Classé re-parse : dirty intact
                                    // — comptabilisé (mesure du
                                    // périmètre du pilote).
                                    d.note_journal_classified(n);
                                    None
                                }
                                Some(ops) => {
                                    d.take_dirty();
                                    d.take_dirty_path();
                                    Some(DomJournal {
                                        ops,
                                        fingerprint: d.structural_fingerprint(),
                                    })
                                }
                            }
                        });
                        let _ = tx.send(out);
                    }
                    // JS-C39 : les compteurs miroir (appels/octets/
                    // micros de `serialize_document`) sont la mesure
                    // par synchro — `syncs` = sérialisations produites.
                    Cmd::DomSyncStats { tx } => {
                        let st = jsd.as_ref().map(|d| {
                            let d = d.borrow();
                            let s = d.serialize_stats();
                            DomSyncStats {
                                syncs: s.calls,
                                bytes: s.bytes,
                                micros: s.micros,
                                journal_classified: d.journal_dropped(),
                            }
                        });
                        let _ = tx.send(st);
                    }
                    Cmd::HeapStats { tx } => {
                        let _ = tx.send(eng.heap_stats());
                    }
                    Cmd::DiagLine { tx } => {
                        let _ = tx.send(eng.b59_diag());
                    }
                    Cmd::Gc => {
                        eng.collect();
                    }
                    Cmd::EvalModule {
                        entry,
                        sources,
                        resolve,
                        tx,
                    } => {
                        let out = eval_module(eng, &entry, sources, resolve);
                        let _ = tx.send(out);
                    }
                    Cmd::SetImportMap { json } => eng.set_import_map(&json),
                    Cmd::Dispatch {
                        ty,
                        path,
                        props,
                        tx,
                    } => {
                        let out = dispatch_event(eng, jsd.as_ref(), &ty, &path, &props);
                        let _ = tx.send(out);
                    }
                    // Étape 134 — realm par `<iframe>` (JS-C78 sous
                    // le capot). Chaque commande adressée à `ctx`
                    // fait `context_switch` puis **restaure** le
                    // contexte précédent : l'état courant ne fuit
                    // jamais d'une commande à l'autre.
                    Cmd::ContextNew { url, frame_ord, tx } => {
                        // `frame_node` = (pointeur du miroir parent,
                        // NodeId miroir du nième <iframe>) — sert
                        // `contentWindow`/`contentDocument`/`frameElement`.
                        let frame = jsd.as_ref().and_then(|d| {
                            frame_ord.and_then(|o| {
                                d.borrow()
                                    .nodes
                                    .iter()
                                    .enumerate()
                                    .filter(|(_, n)| n.tag == "iframe")
                                    .nth(o as usize)
                                    .map(|(i, _)| {
                                        (
                                            std::rc::Rc::as_ptr(d) as usize,
                                            u32::try_from(i).unwrap_or(u32::MAX),
                                        )
                                    })
                            })
                        });
                        let prev = eng.current_context();
                        let id = eng.context_new(&url, frame);
                        eng.context_switch(prev);
                        let _ = tx.send(Some(id));
                    }
                    Cmd::ContextNewIn {
                        parent,
                        url,
                        frame_ord,
                        tx,
                    } => {
                        // Même résolution `frame_ord → frame_node`
                        // que `ContextNew` mais dans le miroir du
                        // **parent explicite** (CAP-171 — cadre
                        // inséré dans un sous-contexte) ; le switch
                        // rattache aussi le realm au bon parent
                        // (`context_new` prend `cur_ctx`).
                        let pdom = if parent == JS_CTX_TOP {
                            jsd.as_ref()
                        } else {
                            ctx_doms.get(&parent)
                        };
                        let frame = pdom.and_then(|d| {
                            frame_ord.and_then(|o| {
                                d.borrow()
                                    .nodes
                                    .iter()
                                    .enumerate()
                                    .filter(|(_, n)| n.tag == "iframe")
                                    .nth(o as usize)
                                    .map(|(i, _)| {
                                        (
                                            std::rc::Rc::as_ptr(d) as usize,
                                            u32::try_from(i).unwrap_or(u32::MAX),
                                        )
                                    })
                            })
                        });
                        let prev = eng.current_context();
                        eng.context_switch(parent);
                        let id = eng.context_new(&url, frame);
                        eng.context_switch(prev);
                        let _ = tx.send(Some(id));
                    }
                    Cmd::ContextDestroy { ctx } => {
                        eng.context_destroy(ctx);
                        ctx_doms.remove(&ctx);
                        ctx_jars.remove(&ctx);
                        ctx_stores.remove(&ctx);
                    }
                    Cmd::InstallDomCtx {
                        ctx,
                        html,
                        page_url,
                        local,
                        cookie_header,
                    } => {
                        let prev = eng.current_context();
                        eng.context_switch(ctx);
                        if eng.current_context() == ctx {
                            // Jar et storage isolés du cadre : la
                            // vue `document.cookie`/`localStorage`
                            // est celle de SON origine ; ses
                            // écritures sont drainées vers la
                            // session sous la même origine
                            // (`DrainSinksCtx`), jamais celle du
                            // parent — pas de fuite croisée.
                            let cjar = std::rc::Rc::new(SnapshotJar::default());
                            cjar.set_header(&cookie_header);
                            let storage: explorer_jscore::Storage =
                                std::rc::Rc::new(std::cell::RefCell::new(local));
                            let d = install_dom_ctx(
                                eng,
                                net_q,
                                &cjar,
                                &storage,
                                &html,
                                &page_url,
                                xhr_next,
                                crypto_next,
                            );
                            ctx_jars.insert(ctx, cjar);
                            ctx_stores.insert(ctx, storage);
                            ctx_doms.insert(ctx, d);
                            eng.context_switch(prev);
                        } else {
                            // Contexte mort/inconnu : le switch est
                            // un no-op côté moteur — rien à installer.
                            eng.context_switch(prev);
                        }
                    }
                    Cmd::EvalCtx { ctx, src, name, tx } => {
                        let prev = eng.current_context();
                        eng.context_switch(ctx);
                        let out = if eng.current_context() == ctx {
                            let res = match &name {
                                Some(n) => eng.eval_named(&src, n),
                                None => eng.eval(&src),
                            };
                            match res {
                                Ok(v) => Ok(eng.describe(v)),
                                Err(e) => Err(JsError::from(e)),
                            }
                        } else {
                            Err(JsError::Dead)
                        };
                        eng.context_switch(prev);
                        let _ = tx.send(out);
                    }
                    Cmd::EvalScriptEl {
                        ctx,
                        src,
                        name,
                        ord,
                        tx,
                    } => {
                        let prev = eng.current_context();
                        eng.context_switch(ctx);
                        let out = if eng.current_context() == ctx {
                            match eng.eval_script_el(&src, name.as_deref(), ord) {
                                Ok(v) => Ok(eng.describe(v)),
                                Err(e) => Err(JsError::from(e)),
                            }
                        } else {
                            Err(JsError::Dead)
                        };
                        eng.context_switch(prev);
                        let _ = tx.send(out);
                    }
                    Cmd::DrainDynScripts { tx } => {
                        let _ = tx.send(eng.drain_dyn_scripts());
                    }
                    Cmd::DrainUnhandledRejections { tx } => {
                        let _ = tx.send(eng.drain_unhandled_rejections());
                    }
                    Cmd::DeliverDynScript { ctx, seq, body } => {
                        let prev = eng.current_context();
                        eng.context_switch(ctx);
                        if eng.current_context() == ctx {
                            eng.deliver_dyn_script(seq, body.as_deref());
                        }
                        eng.context_switch(prev);
                    }
                    Cmd::DrainDynFrames { tx } => {
                        let _ = tx.send(eng.drain_dyn_frames());
                    }
                    Cmd::DeliverDynFrame { ctx, seq, ok } => {
                        let prev = eng.current_context();
                        eng.context_switch(ctx);
                        if eng.current_context() == ctx {
                            eng.deliver_dyn_frame(seq, ok);
                        }
                        eng.context_switch(prev);
                    }
                    Cmd::DrainDynImgs { tx } => {
                        let _ = tx.send(eng.drain_dyn_imgs());
                    }
                    Cmd::DeliverDynImg { ctx, seq, ok } => {
                        let prev = eng.current_context();
                        eng.context_switch(ctx);
                        if eng.current_context() == ctx {
                            eng.deliver_dyn_img(seq, ok);
                        }
                        eng.context_switch(prev);
                    }
                    Cmd::AdoptedSheetsCss { tx } => {
                        let _ = tx.send(eng.adopted_sheets_css());
                    }
                    Cmd::DefinedCeNames { tx } => {
                        let _ = tx.send(eng.defined_ce_names());
                    }
                    Cmd::DrainDynLinks { tx } => {
                        let _ = tx.send(eng.drain_dyn_links());
                    }
                    Cmd::DeliverDynLink { ctx, seq, ok } => {
                        let prev = eng.current_context();
                        eng.context_switch(ctx);
                        if eng.current_context() == ctx {
                            eng.deliver_dyn_link(seq, ok);
                        }
                        eng.context_switch(prev);
                    }
                    Cmd::DrainDynImports { tx } => {
                        let _ = tx.send(eng.drain_dyn_imports());
                    }
                    Cmd::DeliverDynImport {
                        ctx,
                        seq,
                        src,
                        deps,
                    } => {
                        let prev = eng.current_context();
                        eng.context_switch(ctx);
                        if eng.current_context() == ctx {
                            // JS-C101 — deps seedées avant l'entrée :
                            // `load_source` des imports statiques du
                            // module livré les trouve en map.
                            for (u, s) in deps {
                                eng.seed_module_source(u, s);
                            }
                            eng.deliver_import(
                                seq,
                                src.map_or_else(|| Err("fetch échoué".into()), Ok),
                            );
                        }
                        eng.context_switch(prev);
                    }
                    Cmd::DeliverStaticLink { ctx, ord, ok } => {
                        let prev = eng.current_context();
                        eng.context_switch(ctx);
                        if eng.current_context() == ctx {
                            eng.deliver_static_link(ord, ok);
                        }
                        eng.context_switch(prev);
                    }
                    Cmd::DeliverDynModule { ctx, seq, ok } => {
                        let prev = eng.current_context();
                        eng.context_switch(ctx);
                        if eng.current_context() == ctx {
                            eng.deliver_dyn_module(seq, ok);
                        }
                        eng.context_switch(prev);
                    }
                    Cmd::DispatchCtx { ctx, ty, path, tx } => {
                        let prev = eng.current_context();
                        eng.context_switch(ctx);
                        let out = if eng.current_context() == ctx {
                            dispatch_event(eng, ctx_doms.get(&ctx), &ty, &path, &[])
                        } else {
                            Ok(false)
                        };
                        eng.context_switch(prev);
                        let _ = tx.send(out);
                    }
                    Cmd::DomSyncCtx { ctx, tx } => {
                        let out = ctx_doms.get(&ctx).and_then(|d| {
                            let mut d = d.borrow_mut();
                            d.take_dirty().then(|| {
                                let html = d.serialize_document();
                                d.clear_journal();
                                html
                            })
                        });
                        let _ = tx.send(out);
                    }
                    Cmd::DrainSinksCtx { ctx, tx } => {
                        let writes = ctx_jars
                            .get(&ctx)
                            .map(|j| std::mem::take(&mut *j.writes.borrow_mut()))
                            .unwrap_or_default();
                        let map = ctx_stores
                            .get(&ctx)
                            .map_or_else(std::collections::BTreeMap::new, |m| m.borrow().clone());
                        let _ = tx.send((writes, map));
                    }
                    Cmd::SetBudget { ops } => eng.set_op_limit(ops),
                    Cmd::SetMemLimit { bytes } => eng.set_mem_limit(bytes),
                    Cmd::Shutdown => *stopped = true,
                    // Surfaces script-initiées (JS-C27/C28) déléguées.
                    net => {
                        let _ = serve_side(
                            eng,
                            jsd.as_ref(),
                            net_q,
                            jar,
                            local_store,
                            media_nodes,
                            net,
                        );
                    }
                }));
            }
            if stopped {
                return;
            }
        }
    }

    /// Fabrique de [`JscoreEngine`] — pour `Session::set_js_factory`.
    #[must_use]
    pub fn factory() -> JsFactory {
        std::sync::Arc::new(|| Box::new(JscoreEngine::new()))
    }
}

impl Default for JscoreEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl JsEngine for JscoreEngine {
    fn eval(&mut self, source: &str) -> Result<String, JsError> {
        self.eval_cmd(source, None)
    }

    fn eval_named(&mut self, source: &str, name: &str) -> Result<String, JsError> {
        self.eval_cmd(source, Some(name))
    }

    fn run_pending(&mut self) -> usize {
        self.run_pending_bounded(usize::MAX).unwrap_or(0)
    }

    fn run_pending_bounded(&mut self, max_timers: usize) -> Result<usize, JsError> {
        let (tx, rx) = channel();
        if self.tx.send(Cmd::RunPending { max_timers, tx }).is_err() {
            return Err(JsError::Dead);
        }
        Self::recv_eval(&rx)?
    }

    fn next_timer_deadline(&mut self) -> Option<std::time::Duration> {
        let (tx, rx) = channel();
        if self.tx.send(Cmd::TimerDeadline { tx }).is_err() {
            return None;
        }
        rx.recv_timeout(Self::EVAL_CMD_TIMEOUT).unwrap_or(None)
    }

    fn is_media_source(&mut self, url: &str) -> bool {
        let (tx, rx) = channel();
        if self
            .tx
            .send(Cmd::IsMediaSource {
                url: url.to_string(),
                tx,
            })
            .is_err()
        {
            return false;
        }
        rx.recv_timeout(Self::EVAL_CMD_TIMEOUT).unwrap_or(false)
    }

    fn mse_pull(&mut self, url: &str, buffer: usize, max: usize) -> Option<(Vec<u8>, u64, bool)> {
        let (tx, rx) = channel();
        if self
            .tx
            .send(Cmd::MsePull {
                url: url.to_string(),
                buffer,
                max,
                tx,
            })
            .is_err()
        {
            return None;
        }
        rx.recv_timeout(Self::EVAL_CMD_TIMEOUT).ok().flatten()
    }

    fn mse_buffers(&mut self, url: &str) -> Vec<String> {
        let (tx, rx) = channel();
        if self
            .tx
            .send(Cmd::MseBuffers {
                url: url.to_string(),
                tx,
            })
            .is_err()
        {
            return Vec::new();
        }
        rx.recv_timeout(Self::EVAL_CMD_TIMEOUT).unwrap_or_default()
    }

    fn mse_offsets(&mut self, url: &str) -> Vec<f64> {
        let (tx, rx) = channel();
        if self
            .tx
            .send(Cmd::MseOffsets {
                url: url.to_string(),
                tx,
            })
            .is_err()
        {
            return Vec::new();
        }
        rx.recv_timeout(Self::EVAL_CMD_TIMEOUT).unwrap_or_default()
    }

    fn promise_stats(&mut self) -> (u64, u64) {
        let (tx, rx) = channel();
        if self.tx.send(Cmd::PromiseStats { tx }).is_err() {
            return (0, 0);
        }
        rx.recv_timeout(Self::EVAL_CMD_TIMEOUT).unwrap_or((0, 0))
    }

    fn boot_diag(&mut self) -> (usize, usize, &'static str) {
        let (tx, rx) = channel();
        if self.tx.send(Cmd::BootDiag { tx }).is_err() {
            return (0, 0, "loading");
        }
        rx.recv_timeout(Self::EVAL_CMD_TIMEOUT)
            .unwrap_or((0, 0, "loading"))
    }

    fn sync_mirror_attr(&mut self, path: &[(String, u32)], attr: &str, value: Option<&str>) {
        let _ = self.tx.send(Cmd::SyncField {
            path: path.to_vec(),
            attr: attr.to_string(),
            value: value.map(str::to_string),
        });
    }

    fn install_dom(&mut self, html: &str, page_url: &str, history_len: u32) {
        let _ = self.tx.send(Cmd::InstallDom {
            html: html.to_string(),
            page_url: page_url.to_string(),
            history_len,
        });
    }

    fn context_new(&mut self, url: &str, frame_ord: Option<u32>) -> Option<JsCtxId> {
        let (tx, rx) = channel();
        if self
            .tx
            .send(Cmd::ContextNew {
                url: url.to_string(),
                frame_ord,
                tx,
            })
            .is_err()
        {
            return None;
        }
        rx.recv_timeout(Self::EVAL_CMD_TIMEOUT).ok().flatten()
    }

    fn context_new_in(
        &mut self,
        parent: JsCtxId,
        url: &str,
        frame_ord: Option<u32>,
    ) -> Option<JsCtxId> {
        let (tx, rx) = channel();
        if self
            .tx
            .send(Cmd::ContextNewIn {
                parent,
                url: url.to_string(),
                frame_ord,
                tx,
            })
            .is_err()
        {
            return None;
        }
        rx.recv_timeout(Self::EVAL_CMD_TIMEOUT).ok().flatten()
    }

    fn context_destroy(&mut self, ctx: JsCtxId) {
        let _ = self.tx.send(Cmd::ContextDestroy { ctx });
    }

    fn install_dom_ctx(
        &mut self,
        ctx: JsCtxId,
        html: &str,
        page_url: &str,
        local: std::collections::BTreeMap<String, String>,
        cookie_header: &str,
    ) {
        let _ = self.tx.send(Cmd::InstallDomCtx {
            ctx,
            html: html.to_string(),
            page_url: page_url.to_string(),
            local,
            cookie_header: cookie_header.to_string(),
        });
    }

    fn eval_ctx(
        &mut self,
        ctx: JsCtxId,
        source: &str,
        name: Option<&str>,
    ) -> Result<String, JsError> {
        let (tx, rx) = channel();
        if self
            .tx
            .send(Cmd::EvalCtx {
                ctx,
                src: source.to_string(),
                name: name.map(str::to_string),
                tx,
            })
            .is_err()
        {
            return Err(JsError::Dead);
        }
        Self::recv_eval_long(&rx)?
    }

    fn eval_script_el(
        &mut self,
        ctx: JsCtxId,
        source: &str,
        name: Option<&str>,
        script_ord: usize,
    ) -> Result<String, JsError> {
        let (tx, rx) = channel();
        if self
            .tx
            .send(Cmd::EvalScriptEl {
                ctx,
                src: source.to_string(),
                name: name.map(str::to_string),
                ord: script_ord,
                tx,
            })
            .is_err()
        {
            return Err(JsError::Dead);
        }
        Self::recv_eval_long(&rx)?
    }

    fn drain_dyn_scripts(&mut self) -> Vec<explorer_jscore::DynScriptReq> {
        let (tx, rx) = channel();
        if self.tx.send(Cmd::DrainDynScripts { tx }).is_err() {
            return Vec::new();
        }
        rx.recv_timeout(Self::EVAL_CMD_TIMEOUT).unwrap_or_default()
    }

    fn drain_unhandled_rejections(&mut self) -> Vec<String> {
        let (tx, rx) = channel();
        if self.tx.send(Cmd::DrainUnhandledRejections { tx }).is_err() {
            return Vec::new();
        }
        rx.recv_timeout(Self::EVAL_CMD_TIMEOUT).unwrap_or_default()
    }

    fn deliver_dyn_script(&mut self, ctx: JsCtxId, seq: u64, body: Option<String>) {
        let _ = self.tx.send(Cmd::DeliverDynScript { ctx, seq, body });
    }

    fn drain_dyn_frames(&mut self) -> Vec<explorer_jscore::DynFrameReq> {
        let (tx, rx) = channel();
        if self.tx.send(Cmd::DrainDynFrames { tx }).is_err() {
            return Vec::new();
        }
        rx.recv_timeout(Self::EVAL_CMD_TIMEOUT).unwrap_or_default()
    }

    fn deliver_dyn_frame(&mut self, ctx: JsCtxId, seq: u64, ok: bool) {
        let _ = self.tx.send(Cmd::DeliverDynFrame { ctx, seq, ok });
    }

    fn drain_dyn_imgs(&mut self) -> Vec<explorer_jscore::DynImgReq> {
        let (tx, rx) = channel();
        if self.tx.send(Cmd::DrainDynImgs { tx }).is_err() {
            return Vec::new();
        }
        rx.recv_timeout(Self::EVAL_CMD_TIMEOUT).unwrap_or_default()
    }

    fn deliver_dyn_img(&mut self, ctx: JsCtxId, seq: u64, ok: bool) {
        let _ = self.tx.send(Cmd::DeliverDynImg { ctx, seq, ok });
    }

    fn adopted_sheets_css(&mut self) -> Vec<String> {
        let (tx, rx) = channel();
        if self.tx.send(Cmd::AdoptedSheetsCss { tx }).is_err() {
            return Vec::new();
        }
        rx.recv_timeout(Self::EVAL_CMD_TIMEOUT).unwrap_or_default()
    }

    fn defined_ce_names(&mut self) -> Vec<String> {
        let (tx, rx) = channel();
        if self.tx.send(Cmd::DefinedCeNames { tx }).is_err() {
            return Vec::new();
        }
        rx.recv_timeout(Self::EVAL_CMD_TIMEOUT).unwrap_or_default()
    }

    fn drain_dyn_links(&mut self) -> Vec<explorer_jscore::DynLinkReq> {
        let (tx, rx) = channel();
        if self.tx.send(Cmd::DrainDynLinks { tx }).is_err() {
            return Vec::new();
        }
        rx.recv_timeout(Self::EVAL_CMD_TIMEOUT).unwrap_or_default()
    }

    fn deliver_dyn_link(&mut self, ctx: JsCtxId, seq: u64, ok: bool) {
        let _ = self.tx.send(Cmd::DeliverDynLink { ctx, seq, ok });
    }

    fn drain_dyn_imports(&mut self) -> Vec<explorer_jscore::DynImportReq> {
        let (tx, rx) = channel();
        if self.tx.send(Cmd::DrainDynImports { tx }).is_err() {
            return Vec::new();
        }
        rx.recv_timeout(Self::EVAL_CMD_TIMEOUT).unwrap_or_default()
    }

    fn deliver_dyn_import(
        &mut self,
        ctx: JsCtxId,
        seq: u64,
        src: Option<String>,
        deps: Vec<(String, String)>,
    ) {
        let _ = self.tx.send(Cmd::DeliverDynImport {
            ctx,
            seq,
            src,
            deps,
        });
    }

    fn deliver_static_link(&mut self, ctx: JsCtxId, ord: u32, ok: bool) {
        let _ = self.tx.send(Cmd::DeliverStaticLink { ctx, ord, ok });
    }

    fn deliver_dyn_module(&mut self, ctx: JsCtxId, seq: u64, ok: bool) {
        let _ = self.tx.send(Cmd::DeliverDynModule { ctx, seq, ok });
    }

    fn dispatch_event_ctx(
        &mut self,
        ctx: JsCtxId,
        ty: &str,
        path: &[(String, u32)],
    ) -> Result<bool, JsError> {
        let (tx, rx) = channel();
        if self
            .tx
            .send(Cmd::DispatchCtx {
                ctx,
                ty: ty.to_string(),
                path: path.to_vec(),
                tx,
            })
            .is_err()
        {
            return Err(JsError::Dead);
        }
        Self::recv_eval(&rx)?
    }

    fn sync_dom_ctx(&mut self, ctx: JsCtxId) -> Option<String> {
        let (tx, rx) = channel();
        if self.tx.send(Cmd::DomSyncCtx { ctx, tx }).is_err() {
            return None;
        }
        rx.recv_timeout(Self::EVAL_CMD_TIMEOUT).ok().flatten()
    }

    fn drain_sinks_ctx(
        &mut self,
        ctx: JsCtxId,
    ) -> (Vec<String>, std::collections::BTreeMap<String, String>) {
        let (tx, rx) = channel();
        if self.tx.send(Cmd::DrainSinksCtx { ctx, tx }).is_err() {
            return (Vec::new(), std::collections::BTreeMap::new());
        }
        rx.recv_timeout(Self::EVAL_CMD_TIMEOUT).unwrap_or_default()
    }

    fn drain_nav_requests(&mut self) -> Vec<NavRequest> {
        let (tx, rx) = channel();
        if self.tx.send(Cmd::DrainNav { tx }).is_err() {
            return Vec::new();
        }
        rx.recv_timeout(Self::EVAL_CMD_TIMEOUT).unwrap_or_default()
    }

    fn sync_ctx_url(&mut self, ctx: JsCtxId, url: &str, history_len: u32) {
        let _ = self.tx.send(Cmd::SyncCtxUrl {
            ctx,
            url: url.to_string(),
            history_len,
        });
    }

    fn sync_ctx_state(
        &mut self,
        ctx: JsCtxId,
        url: &str,
        history_len: u32,
        state: Option<&str>,
        popstate: bool,
    ) {
        let _ = self.tx.send(Cmd::SyncCtxState {
            ctx,
            url: url.to_string(),
            history_len,
            state: state.map(str::to_string),
            popstate,
        });
    }

    fn set_ctx_name(&mut self, ctx: JsCtxId, name: &str) {
        let _ = self.tx.send(Cmd::SetCtxName {
            ctx,
            name: name.to_string(),
        });
    }

    fn fire_hashchange(&mut self, ctx: JsCtxId, old: &str, new: &str) {
        let _ = self.tx.send(Cmd::FireHashchange {
            ctx,
            old: old.to_string(),
            new: new.to_string(),
        });
    }

    fn set_ready_state(&mut self, ctx: JsCtxId, state: &str) {
        let _ = self.tx.send(Cmd::SetReadyState {
            ctx,
            state: state.to_string(),
        });
    }

    fn set_viewport(&mut self, w: u32, h: u32) {
        let _ = self.tx.send(Cmd::SetViewport { w, h });
    }

    fn set_geometry_ctx(&mut self, ctx: JsCtxId, slot: explorer_jscore::GeometrySlot) {
        let _ = self.tx.send(Cmd::SetGeometry { ctx, slot });
    }

    fn set_media_env(&mut self, env: explorer_jscore::MediaEnv) {
        let _ = self.tx.send(Cmd::SetMediaEnv { env });
    }

    fn set_page_referrer(&mut self, referrer: &str) {
        let _ = self.tx.send(Cmd::SetPageReferrer {
            referrer: referrer.to_string(),
        });
    }

    fn sync_dom(&mut self) -> Option<String> {
        self.sync_dom_path().map(|(html, _)| html)
    }

    fn sync_dom_path(&mut self) -> Option<DomSyncPayload> {
        let (tx, rx) = channel();
        if self.tx.send(Cmd::DomSync { tx }).is_err() {
            return None;
        }
        rx.recv_timeout(Self::EVAL_CMD_TIMEOUT).ok().flatten()
    }

    fn sync_dom_keyed(&mut self) -> Option<DomSyncKeyed> {
        let (tx, rx) = channel();
        if self
            .tx
            .send(Cmd::DomSyncKeyed { force: false, tx })
            .is_err()
        {
            return None;
        }
        rx.recv_timeout(Self::EVAL_CMD_TIMEOUT).ok().flatten()
    }

    fn resync_dom_keyed(&mut self) -> Option<DomSyncKeyed> {
        let (tx, rx) = channel();
        if self.tx.send(Cmd::DomSyncKeyed { force: true, tx }).is_err() {
            return None;
        }
        rx.recv_timeout(Self::EVAL_CMD_TIMEOUT).ok().flatten()
    }

    fn drain_dom_mutations(&mut self) -> Option<DomJournal> {
        let (tx, rx) = channel();
        if self.tx.send(Cmd::DomJournal { tx }).is_err() {
            return None;
        }
        rx.recv_timeout(Self::EVAL_CMD_TIMEOUT).ok().flatten()
    }

    fn dom_sync_stats(&mut self) -> Option<DomSyncStats> {
        let (tx, rx) = channel();
        if self.tx.send(Cmd::DomSyncStats { tx }).is_err() {
            return None;
        }
        rx.recv_timeout(Self::EVAL_CMD_TIMEOUT).ok().flatten()
    }

    fn heap_stats(&mut self) -> Option<HeapStats> {
        let (tx, rx) = channel();
        if self.tx.send(Cmd::HeapStats { tx }).is_err() {
            return None;
        }
        rx.recv_timeout(Self::EVAL_CMD_TIMEOUT).ok()
    }

    fn diag_line(&mut self) -> Option<String> {
        let (tx, rx) = channel();
        if self.tx.send(Cmd::DiagLine { tx }).is_err() {
            return None;
        }
        rx.recv_timeout(Self::EVAL_CMD_TIMEOUT).ok()
    }

    fn gc(&mut self) {
        let _ = self.tx.send(Cmd::Gc);
    }

    fn canvas_ops(&mut self) -> CanvasOpsTable {
        let (tx, rx) = channel();
        if self.tx.send(Cmd::CanvasOps { tx }).is_err() {
            return Vec::new();
        }
        rx.recv_timeout(Self::EVAL_CMD_TIMEOUT).unwrap_or_default()
    }

    fn eval_module(
        &mut self,
        entry: &str,
        sources: HashMap<String, String>,
        resolve: UrlResolver,
    ) -> Result<String, JsError> {
        let (tx, rx) = channel();
        if self
            .tx
            .send(Cmd::EvalModule {
                entry: entry.to_string(),
                sources,
                resolve,
                tx,
            })
            .is_err()
        {
            return Err(JsError::Dead);
        }
        Self::recv_eval_long(&rx)?
    }

    fn dispatch_event(&mut self, ty: &str, path: &[(String, u32)]) -> Result<bool, JsError> {
        self.dispatch_event_props(ty, path, &[])
    }

    fn dispatch_event_props(
        &mut self,
        ty: &str,
        path: &[(String, u32)],
        props: &[(String, EventProp)],
    ) -> Result<bool, JsError> {
        let (tx, rx) = channel();
        if self
            .tx
            .send(Cmd::Dispatch {
                ty: ty.to_string(),
                path: path.to_vec(),
                props: props.to_vec(),
                tx,
            })
            .is_err()
        {
            return Err(JsError::Dead);
        }
        Self::recv_eval(&rx)?
    }

    fn set_import_map(&mut self, json: &str) {
        // Feu-et-oublie : la map s'applique aux prochains modules.
        let _ = self.tx.send(Cmd::SetImportMap {
            json: json.to_string(),
        });
    }

    fn set_budget(&mut self, ops: u64) {
        // Feu-et-oublie : le budget s'applique au prochain eval.
        let _ = self.tx.send(Cmd::SetBudget { ops });
    }

    fn set_mem_limit(&mut self, bytes: usize) {
        // Feu-et-oublie : la borne s'applique dès réception —
        // émise avant `install_dom`, donc avant tout eval.
        let _ = self.tx.send(Cmd::SetMemLimit { bytes });
    }

    fn drain_net_requests(&mut self) -> Vec<NetRequest> {
        let (tx, rx) = channel();
        if self.tx.send(Cmd::DrainNet { tx }).is_err() {
            return Vec::new();
        }
        rx.recv_timeout(Self::EVAL_CMD_TIMEOUT).unwrap_or_default()
    }

    fn drain_worker_requests(&mut self) -> Vec<NetRequest> {
        self.worker_rx.try_iter().collect()
    }

    fn deliver_net_response(&mut self, resp: NetResponse) -> Result<(), JsError> {
        let (tx, rx) = channel();
        if self.tx.send(Cmd::DeliverNet { resp, tx }).is_err() {
            return Err(JsError::Dead);
        }
        Self::recv_eval(&rx)?
    }

    fn deliver_es_event(
        &mut self,
        ctx: JsCtxId,
        handle: u64,
        ev: explorer_jscore::EsEvent,
    ) -> Result<(), JsError> {
        let (tx, rx) = channel();
        if self
            .tx
            .send(Cmd::DeliverEs {
                ctx,
                handle,
                ev,
                tx,
            })
            .is_err()
        {
            return Err(JsError::Dead);
        }
        Self::recv_eval(&rx)?
    }

    fn deliver_ws_event(
        &mut self,
        ctx: JsCtxId,
        handle: u64,
        ev: explorer_jscore::WsEventKind,
    ) -> Result<(), JsError> {
        let (tx, rx) = channel();
        if self
            .tx
            .send(Cmd::DeliverWs {
                ctx,
                handle,
                ev,
                tx,
            })
            .is_err()
        {
            return Err(JsError::Dead);
        }
        Self::recv_eval(&rx)?
    }

    fn deliver_crypto_response(&mut self, resp: CryptoResponse) -> Result<(), JsError> {
        let (tx, rx) = channel();
        if self.tx.send(Cmd::DeliverCrypto { resp, tx }).is_err() {
            return Err(JsError::Dead);
        }
        Self::recv_eval(&rx)?
    }

    fn install_storage(&mut self, local: std::collections::BTreeMap<String, String>) {
        let _ = self.tx.send(Cmd::InstallStorage { local });
    }

    fn drain_storage(&mut self) -> std::collections::BTreeMap<String, String> {
        let (tx, rx) = channel();
        if self.tx.send(Cmd::DrainStorage { tx }).is_err() {
            return std::collections::BTreeMap::new();
        }
        rx.recv_timeout(Self::EVAL_CMD_TIMEOUT).unwrap_or_default()
    }

    fn install_idb(&mut self, json: &str) {
        if json.is_empty() {
            return;
        }
        let _ = self.tx.send(Cmd::InstallIdb {
            json: json.to_string(),
        });
    }

    fn drain_idb(&mut self) -> String {
        let (tx, rx) = channel();
        if self.tx.send(Cmd::DrainIdb { tx }).is_err() {
            return String::new();
        }
        rx.recv_timeout(Self::EVAL_CMD_TIMEOUT).unwrap_or_default()
    }

    fn install_idb_ctx(&mut self, ctx: JsCtxId, json: &str) {
        if json.is_empty() {
            return;
        }
        let _ = self.tx.send(Cmd::InstallIdbCtx {
            ctx,
            json: json.to_string(),
        });
    }

    fn drain_idb_ctx(&mut self, ctx: JsCtxId) -> String {
        let (tx, rx) = channel();
        if self.tx.send(Cmd::DrainIdbCtx { ctx, tx }).is_err() {
            return String::new();
        }
        rx.recv_timeout(Self::EVAL_CMD_TIMEOUT).unwrap_or_default()
    }

    fn set_cookie_snapshot(&mut self, header: &str) {
        let _ = self.tx.send(Cmd::CookieSnap {
            header: header.to_string(),
        });
    }

    fn drain_cookie_writes(&mut self) -> Vec<String> {
        let (tx, rx) = channel();
        if self.tx.send(Cmd::DrainCookies { tx }).is_err() {
            return Vec::new();
        }
        rx.recv_timeout(Self::EVAL_CMD_TIMEOUT).unwrap_or_default()
    }

    fn set_media_claim(
        &mut self,
        path: &[(String, u32)],
        url: &str,
        media: u64,
        codecs: Option<Vec<String>>,
    ) {
        let _ = self.tx.send(Cmd::SetMediaClaim {
            path: path.to_vec(),
            url: url.to_string(),
            media,
            codecs,
        });
    }

    fn deliver_media_state(&mut self, path: &[(String, u32)], media: u64, report: JsMediaReport) {
        let _ = self.tx.send(Cmd::DeliverMediaState {
            path: path.to_vec(),
            media,
            report,
        });
    }

    fn drain_media_commands(&mut self) -> Vec<JsMediaCommand> {
        let (tx, rx) = channel();
        if self.tx.send(Cmd::DrainMediaCommands { tx }).is_err() {
            return Vec::new();
        }
        rx.recv_timeout(Self::EVAL_CMD_TIMEOUT).unwrap_or_default()
    }
}

impl Drop for JscoreEngine {
    fn drop(&mut self) {
        let _ = self.tx.send(Cmd::Shutdown);
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

/// `Cmd::EvalModule` côté worker : le graphe préfetché est
/// servi au `ModuleLoader` par `MapLoader` (étape 50) —
/// jamais de fetch ré-entrant depuis le thread moteur.
fn eval_module(
    eng: &mut explorer_jscore::Engine,
    entry: &str,
    sources: std::collections::HashMap<String, String>,
    resolve: UrlResolver,
) -> Result<String, JsError> {
    let src = sources.get(entry).cloned().unwrap_or_default();
    let loader = MapLoader {
        map: sources,
        resolve,
    };
    eng.set_module_loader(std::rc::Rc::new(std::cell::RefCell::new(loader)));
    match eng.eval_module(&src, entry) {
        Ok(v) => Ok(eng.describe(v)),
        Err(e) => Err(JsError::from(e)),
    }
}

/// `Cmd::CanvasOps` côté worker : op-lists `<canvas>`
/// cumulées du miroir (JS-C28) — snapshot par chemin
/// d'élément, jamais drainé (la bitmap persiste entre deux
/// re-peintures).
fn canvas_ops_of(jsd: Option<&explorer_jscore::DomRef>) -> CanvasOpsTable {
    jsd.as_ref()
        .map_or_else(Vec::new, |d| d.borrow().canvas_ops())
}

/// `Cmd::SyncField` côté worker : l'état hôte d'un champ
/// (`value`, `checked`, `open`…) est reporté dans le miroir
/// pour que les handlers JS le lisent. `<textarea>` expose sa
/// valeur en contenu texte, pas en attribut.
fn sync_field(
    jsd: Option<&explorer_jscore::DomRef>,
    path: &[(String, u32)],
    attr: &str,
    value: Option<&str>,
) {
    let node = jsd.and_then(|d| mirror_node(&d.borrow(), path));
    if let (Some(d), Some(n)) = (jsd, node) {
        let mut d = d.borrow_mut();
        let is_area = d.nodes[n as usize].tag == "textarea";
        if attr == "value" && is_area {
            let t = d.create_text(value.unwrap_or_default());
            d.replace_children(n, &[t]);
        } else if let Some(v) = value {
            d.set_attr(n, attr, v);
        } else {
            d.remove_attr(n, attr);
        }
    }
}

/// `Cmd::Dispatch` côté worker : chemin vide = événement
/// document (`dom_event` vise `<body>` — étape 52) ; un
/// chemin non vide non résolu = cible morte → rien
/// (jamais de repli silencieux sur le document).
fn dispatch_event(
    eng: &mut explorer_jscore::Engine,
    jsd: Option<&explorer_jscore::DomRef>,
    ty: &str,
    path: &[(String, u32)],
    props: &[(String, EventProp)],
) -> Result<bool, JsError> {
    if path.is_empty() {
        return eng
            .dom_event_props(ty, props)
            .map_err(|t| JsError::Uncaught(eng.describe(t.0)));
    }
    let node = jsd.and_then(|d| mirror_node(&d.borrow(), path));
    match node {
        Some(n) => eng
            .dom_event_on_props(ty, n, props)
            .map_err(|t| JsError::Uncaught(eng.describe(t.0))),
        None => Ok(false),
    }
}

/// Bras délégué du worker : drains/livraisons des surfaces
/// script-initiées — net/storage/cookie (JS-C27) **et**
/// op-lists `<canvas>` (JS-C28, lecture de la side-table du
/// miroir). `false` = canal de réponse fermé → le worker
/// s'arrête ; tout autre `Cmd` est ignoré (invariant :
/// l'appelant ne délègue que ces commandes).
#[allow(clippy::too_many_lines)] // match de dispatch, chaque bras documenté in situ
fn serve_side(
    eng: &mut explorer_jscore::Engine,
    jsd: Option<&explorer_jscore::DomRef>,
    net_q: &std::rc::Rc<std::cell::RefCell<Vec<NetRequest>>>,
    jar: &std::rc::Rc<SnapshotJar>,
    local_store: &mut Option<explorer_jscore::Storage>,
    media_nodes: &mut HashMap<u64, explorer_jscore::NodeId>,
    cmd: Cmd,
) -> bool {
    match cmd {
        Cmd::CanvasOps { tx } => tx.send(canvas_ops_of(jsd)).is_ok(),
        Cmd::DrainNet { tx } => {
            let out = std::mem::take(&mut *net_q.borrow_mut());
            tx.send(out).is_ok()
        }
        // Seam média (étape 71c) — `path` résolu en nœud
        // miroir une fois ; les commandes drainées rappellent
        // l'id de surface session.
        Cmd::SetMediaClaim {
            path,
            url,
            media,
            codecs,
        } => {
            deliver_media_claim(eng, jsd, media_nodes, &path, &url, media, codecs);
            true
        }
        Cmd::DeliverMediaState {
            path,
            media,
            report,
        } => {
            deliver_media_state(eng, jsd, media_nodes, &path, media, report);
            true
        }
        Cmd::DrainMediaCommands { tx } => {
            let out: Vec<JsMediaCommand> = jsd
                .map(|d| {
                    d.borrow_mut()
                        .media_cmds
                        .drain(..)
                        .map(|(media, cmd)| JsMediaCommand {
                            media,
                            cmd: js_media_cmd(cmd),
                        })
                        .collect()
                })
                .unwrap_or_default();
            tx.send(out).is_ok()
        }
        Cmd::DeliverNet { resp, tx } => {
            let out = eng
                .deliver_xhr_response(explorer_jscore::XhrResponse {
                    handle: resp.handle,
                    status: resp.status,
                    status_text: resp.status_text,
                    headers: resp.headers,
                    body: resp.body,
                })
                .map_err(|t| JsError::Uncaught(eng.describe(t.0)));
            tx.send(out).is_ok()
        }
        Cmd::DeliverCrypto { resp, tx } => {
            let out = eng
                .deliver_crypto_response(explorer_jscore::CryptoResponse {
                    handle: resp.handle,
                    result: resp.result,
                })
                .map_err(|t| JsError::Uncaught(eng.describe(t.0)));
            tx.send(out).is_ok()
        }
        Cmd::DeliverEs {
            ctx,
            handle,
            ev,
            tx,
        } => {
            eng.deliver_es(ctx, handle, &ev);
            tx.send(Ok(())).is_ok()
        }
        Cmd::DeliverWs {
            ctx,
            handle,
            ev,
            tx,
        } => {
            eng.deliver_ws_event(ctx, handle, ev);
            tx.send(Ok(())).is_ok()
        }
        Cmd::InstallStorage { local } => {
            let map: explorer_jscore::Storage = std::rc::Rc::new(std::cell::RefCell::new(local));
            eng.install_storage(&map);
            *local_store = Some(map);
            true
        }
        Cmd::DrainStorage { tx } => {
            let out = local_store
                .as_ref()
                .map_or_else(std::collections::BTreeMap::new, |m| m.borrow().clone());
            tx.send(out).is_ok()
        }
        Cmd::InstallIdb { json } => {
            eng.idb_restore(&json);
            true
        }
        Cmd::DrainIdb { tx } => tx.send(eng.idb_snapshot()).is_ok(),
        Cmd::InstallIdbCtx { ctx, json } => {
            eng.idb_restore_ctx(ctx, &json);
            true
        }
        Cmd::DrainIdbCtx { ctx, tx } => tx.send(eng.idb_snapshot_ctx(ctx)).is_ok(),
        Cmd::CookieSnap { header } => {
            jar.set_header(&header);
            true
        }
        Cmd::DrainCookies { tx } => {
            let out = std::mem::take(&mut *jar.writes.borrow_mut());
            tx.send(out).is_ok()
        }
        _ => true,
    }
}

/// `Cmd::InstallDom` côté worker : miroir parsé (dirty bit
/// consommé — la construction n'est pas une mutation JS),
/// bindings `document`/`location`/`history`, puis surface
/// script-initiée (JS-C27) : XHR + `fetch` → `net_q`,
/// `document.cookie` → `jar`. Renvoie le `DomRef` installé.
#[allow(clippy::too_many_arguments)]
fn install_dom(
    eng: &mut explorer_jscore::Engine,
    nav_q: &std::rc::Rc<std::cell::RefCell<Vec<NavRequest>>>,
    net_q: &std::rc::Rc<std::cell::RefCell<Vec<NetRequest>>>,
    jar: &std::rc::Rc<SnapshotJar>,
    html: &str,
    page_url: &str,
    history_len: u32,
    xhr_next: &std::rc::Rc<std::cell::Cell<u64>>,
    crypto_next: &std::rc::Rc<std::cell::Cell<u64>>,
) -> explorer_jscore::DomRef {
    let d = std::rc::Rc::new(std::cell::RefCell::new(explorer_jscore::Dom::from_html(
        html,
    )));
    d.borrow().take_dirty();
    // La construction a aussi marqué un `dirty_node` (étape
    // 55f) — le vider avec le flag, sinon un `#document`
    // résiduel « empoisonne » l'ancêtre commun des marques
    // suivantes (le chemin sale resterait vide à vie).
    d.borrow().take_dirty_path();
    eng.install_dom(&d);
    // Timers hôtes (étape 52) : `setTimeout`/`setInterval`/
    // `clearTimeout`/`requestAnimationFrame` — les callbacks
    // tirent via `run_due_timers[_bounded]` à la pompe.
    eng.install_timers();
    eng.set_page_url(page_url);
    {
        let q = nav_q.clone();
        let nav: std::rc::Rc<dyn Fn(&str)> = std::rc::Rc::new(move |u| {
            q.borrow_mut().push(NavRequest::Navigate(u.to_string()));
        });
        let q2 = nav_q.clone();
        let hist: std::rc::Rc<dyn Fn(i32)> = std::rc::Rc::new(move |n| {
            q2.borrow_mut().push(NavRequest::HistoryGo(n));
        });
        eng.set_nav_hooks(nav, hist, history_len);
        // `form.submit()`/`requestSubmit()` — champs collectés
        // côté miroir, encodage + navigation côté session.
        let q3 = nav_q.clone();
        eng.set_form_submit_hook(std::rc::Rc::new(move |p: explorer_jscore::FormSubmit| {
            q3.borrow_mut().push(NavRequest::FormSubmit {
                method: p.method,
                action: p.action,
                enctype: p.enctype,
                fields: p.fields,
                target: p.target,
            });
        }));
        // `history.pushState`/`replaceState` (CAP-151) —
        // intention same-document : la session résout,
        // commit URL + historique, resynchronise le miroir
        // via `sync_ctx_url`. Top-level seul (comme les
        // autres hooks nav — cadre = inerte à cette tranche).
        let q4 = nav_q.clone();
        eng.set_push_state_hook(std::rc::Rc::new(
            move |u: String, replace: bool, state: Option<String>| {
                q4.borrow_mut().push(NavRequest::PushState {
                    url: u,
                    replace,
                    state,
                });
            },
        ));
        // `location.replace`/`reload` (CAP-152) — replace
        // réécrit l'entrée courante (back saute la page
        // remplacée), reload rejoue sa requête sans entrée
        // nouvelle. Top-level seul, comme les autres hooks.
        let q5 = nav_q.clone();
        let q6 = nav_q.clone();
        eng.set_nav_replace_reload_hooks(
            std::rc::Rc::new(move |u: String| {
                q5.borrow_mut().push(NavRequest::Replace(u));
            }),
            std::rc::Rc::new(move || {
                q6.borrow_mut().push(NavRequest::Reload);
            }),
        );
        // `window.open` + `<a target>` scripté (CAP-153/159)
        // — nouveau contexte top-level : même voie que le
        // clic `target=_blank` (`DrainAct::NewTab`) — le
        // nom gouverne la réutilisation (CAP-159).
        let q7 = nav_q.clone();
        eng.set_open_hook(std::rc::Rc::new(move |u: String, name: String| {
            q7.borrow_mut().push(NavRequest::NewTab { url: u, name });
        }));
    }
    eng.install_xhr(std::rc::Rc::new(ChanXhr {
        q: std::rc::Rc::clone(net_q),
        next: std::rc::Rc::clone(xhr_next),
    }));
    eng.install_crypto(std::rc::Rc::new(ChanCrypto {
        q: std::rc::Rc::clone(net_q),
        next: std::rc::Rc::clone(crypto_next),
    }));
    // `EventSource` (JS-C92) — ouvertures en `NetRequest::
    // EventSource`, `close()` en `Abort` ; les handles
    // partagent le compteur `xhr_next` (Abort univoque).
    eng.install_es_transport(std::rc::Rc::new(ChanEs {
        q: std::rc::Rc::clone(net_q),
        next: std::rc::Rc::clone(xhr_next),
    }));
    // `WebSocket` (JS-C94) — ouvertures `NetRequest::WsOpen`,
    // `send`/`close` `WsSend`/`WsClose` ; handles `xhr_next`.
    eng.install_ws_transport(std::rc::Rc::new(ChanWs {
        q: std::rc::Rc::clone(net_q),
        next: std::rc::Rc::clone(xhr_next),
    }));
    eng.set_cookie_jar(jar.clone());
    d
}

/// `Cmd::InstallDomCtx` côté worker (étape 134) — même
/// séquence que [`install_dom`] mais pour le contexte d'un
/// `<iframe>` : miroir propre, `document`/`location`/timers/
/// XHR/`crypto.subtle` actifs. **Pas** de hooks de navigation
/// ni de `form.submit` : la navigation de cadre est une piste
/// à part (`location.href=` y est inerte — mieux un no-op
/// qu'un `Navigate` émis sous l'identité du parent). Jar
/// cookie et backing `localStorage` sont **isolés** — une
/// écriture d'un contexte tiers n'atteint jamais les surfaces
/// du document top-level (drainées vers la session sous
/// l'origine enfant par `DrainSinksCtx`).
#[allow(clippy::too_many_arguments)]
fn install_dom_ctx(
    eng: &mut explorer_jscore::Engine,
    net_q: &std::rc::Rc<std::cell::RefCell<Vec<NetRequest>>>,
    jar: &std::rc::Rc<SnapshotJar>,
    storage: &explorer_jscore::Storage,
    html: &str,
    page_url: &str,
    xhr_next: &std::rc::Rc<std::cell::Cell<u64>>,
    crypto_next: &std::rc::Rc<std::cell::Cell<u64>>,
) -> explorer_jscore::DomRef {
    let d = std::rc::Rc::new(std::cell::RefCell::new(explorer_jscore::Dom::from_html(
        html,
    )));
    d.borrow().take_dirty();
    // Voir `install_dom` : le `dirty_node` de construction
    // est vidé avec le flag — un chemin sale résiduel
    // empoisonnerait la première vraie mutation de l'enfant.
    d.borrow().take_dirty_path();
    eng.install_dom(&d);
    eng.install_timers();
    eng.set_page_url(page_url);
    eng.install_xhr(std::rc::Rc::new(ChanXhr {
        q: std::rc::Rc::clone(net_q),
        next: std::rc::Rc::clone(xhr_next),
    }));
    eng.install_crypto(std::rc::Rc::new(ChanCrypto {
        q: std::rc::Rc::clone(net_q),
        next: std::rc::Rc::clone(crypto_next),
    }));
    eng.install_es_transport(std::rc::Rc::new(ChanEs {
        q: std::rc::Rc::clone(net_q),
        next: std::rc::Rc::clone(xhr_next),
    }));
    // `WebSocket` (JS-C94) — même dial que le contexte
    // parent (engine-global) ; les livraisons visent ce ctx.
    eng.install_ws_transport(std::rc::Rc::new(ChanWs {
        q: std::rc::Rc::clone(net_q),
        next: std::rc::Rc::clone(xhr_next),
    }));
    eng.set_cookie_jar(jar.clone());
    // `localStorage` pré-rempli de l'origine enfant — le
    // backing `Rc` vit aussi en `ctx_stores` : `DrainSinksCtx`
    // le relit pour la session.
    eng.install_storage(storage);
    d
}

/// Balises que le tree-builder HTML5 synthétise quand la source
/// les omet — le parseur littéral du miroir ne les produit pas.
/// À la descente de `path`, un niveau implicitement absent est
/// sauté sans consommer d'enfant.
const IMPLICIT_TAGS: &[&str] = &["html", "head", "body", "tbody", "thead", "tfoot"];

/// Rejoue `path` (depuis la racine `#document` du miroir) et
/// retourne le `NodeId` miroir — `None` si le chemin ne mène à
/// aucun élément compatible (arbres divergents → dégradation,
/// pas d'event vers le mauvais nœud).
/// Traduction rapport session → moteur (étape 71c) — aucun
/// type `explorer_jscore` ne fuit du trait.
fn media_report_of(r: JsMediaReport) -> explorer_jscore::MediaReport {
    explorer_jscore::MediaReport {
        position_ms: r.position_ms,
        duration_ms: r.duration_ms,
        buffered_ms: r.buffered_ms,
        state: match r.state {
            JsMediaHostState::Playing => explorer_jscore::MediaState::Playing,
            JsMediaHostState::Paused => explorer_jscore::MediaState::Paused,
            JsMediaHostState::Ended => explorer_jscore::MediaState::Ended,
            JsMediaHostState::Error => explorer_jscore::MediaState::Error,
        },
        error: r.error,
    }
}

/// Ordre moteur → contrat session (étape 71c).
fn js_media_cmd(c: explorer_jscore::MediaCmd) -> JsMediaCmd {
    match c {
        explorer_jscore::MediaCmd::Play => JsMediaCmd::Play,
        explorer_jscore::MediaCmd::Pause => JsMediaCmd::Pause,
        explorer_jscore::MediaCmd::Seek { ms } => JsMediaCmd::Seek { ms },
        explorer_jscore::MediaCmd::Volume { v } => JsMediaCmd::Volume { v },
        explorer_jscore::MediaCmd::Muted { on } => JsMediaCmd::Muted { on },
        explorer_jscore::MediaCmd::Rate { r } => JsMediaCmd::Rate { r },
    }
}

/// Livraison du claim média au nœud miroir résolu — path non
/// résolu (miroir divergent du réel) : repli sur l'URL média
/// (`src` de l'élément ou de son `<source>` — vol 176-ter :
/// le JS réordonne le miroir plus vite que les applies ne
/// resynchronisent le réel). Succès → `media_nodes` retient
/// la résolution pour les rapports `state` suivants.
/// Échec total : warn nommé et les codecs déclarés alimentent
/// quand même la table document-locale `isTypeSupported`
/// (vol 176-bis : le claim entier mourait silencieusement,
/// codecs inclus).
fn deliver_media_claim(
    eng: &mut explorer_jscore::Engine,
    jsd: Option<&explorer_jscore::DomRef>,
    media_nodes: &mut HashMap<u64, explorer_jscore::NodeId>,
    path: &[(String, u32)],
    url: &str,
    media: u64,
    codecs: Option<Vec<String>>,
) {
    let node = jsd.and_then(|d| {
        let d = d.borrow();
        mirror_node(&d, path).or_else(|| mirror_node_by_url(&d, url))
    });
    if let Some(n) = node {
        let is_claim = codecs.is_some();
        eng.set_media_claim(n, media, codecs);
        if is_claim {
            media_nodes.insert(media, n);
        } else {
            media_nodes.remove(&media);
        }
    } else {
        tracing::warn!(
            media,
            url,
            ?path,
            "set_media_claim: nœud miroir introuvable — claim ignoré"
        );
        if let Some(c) = codecs {
            eng.note_media_codecs(&c);
        }
    }
}

/// Livraison d'un rapport `state` hôte — le nœud est
/// d'abord retrouvé par son id de surface (résolution
/// d'identité faite au claim — le path peut être déjà
/// obsolète sous divergence miroir, vol 176-ter), repli
/// sur `mirror_node(path)` qui rafraîchit la map.
fn deliver_media_state(
    eng: &mut explorer_jscore::Engine,
    jsd: Option<&explorer_jscore::DomRef>,
    media_nodes: &mut HashMap<u64, explorer_jscore::NodeId>,
    path: &[(String, u32)],
    media: u64,
    report: JsMediaReport,
) {
    let n = media_nodes.get(&media).copied().or_else(|| {
        jsd.and_then(|d| mirror_node(&d.borrow(), path))
            .inspect(|&n| {
                media_nodes.insert(media, n);
            })
    });
    if let Some(n) = n {
        let _ = eng.deliver_media_state(n, &media_report_of(report));
    }
}

/// Repli d'identité média : élément `video`/`audio` du
/// miroir dont `src` (ou le `src` d'un enfant `<source>`)
/// égale l'URL de la surface — `blob:` compris (attache
/// `URL.createObjectURL` : la chaîne est littérale des deux
/// côtés). Miroir de `media_source` côté core.
fn mirror_node_by_url(d: &explorer_jscore::Dom, url: &str) -> Option<explorer_jscore::NodeId> {
    if url.is_empty() {
        return None;
    }
    let mut stack = vec![d.root];
    while let Some(n) = stack.pop() {
        let tag = d.nodes[n as usize].tag.as_str();
        if tag == "video" || tag == "audio" {
            if d.attr(n, "src") == Some(url) {
                return Some(n);
            }
            for c in d.element_children(n) {
                if d.nodes[c as usize].tag == "source" && d.attr(c, "src") == Some(url) {
                    return Some(n);
                }
            }
        }
        stack.extend(d.element_children(n));
    }
    None
}

fn mirror_node(
    d: &explorer_jscore::Dom,
    path: &[(String, u32)],
) -> Option<explorer_jscore::NodeId> {
    let mut cur = d.root;
    for (tag, k) in path {
        let kids = d.element_children(cur);
        let found = kids
            .iter()
            .filter(|&&c| d.nodes[c as usize].tag == *tag)
            .nth(*k as usize)
            .copied();
        match found {
            Some(n) => cur = n,
            // Wrapper synthétisé côté navigateur, absent du miroir
            // → ne pas descendre : le niveau suivant retente sur
            // les mêmes enfants.
            None if IMPLICIT_TAGS.contains(&tag.as_str()) => {}
            None => return None,
        }
    }
    Some(cur)
}

/// Chargeur de modules préfetchés (étape 50) : le graphe statique
/// est collecté par la session via `HttpTransport` **avant**
/// l'évaluation — le `load` reste synchrone et le canal
/// `JscoreEngine` à sens unique (un fetch depuis le thread moteur
/// bloquerait le hôte qui attend la fin de l'eval : deadlock).
/// Un spécificateur hors map — `import()` dynamique non préfetché,
/// borne `max_modules` atteinte — est une erreur JS propre.
struct MapLoader {
    /// `url absolue → source` — graphe collecté par la session.
    map: HashMap<String, String>,
    /// `(specifier, referrer) → url absolue` (politique hôte).
    resolve: UrlResolver,
}

impl explorer_jscore::ModuleLoader for MapLoader {
    fn load(&mut self, specifier: &str, referrer: &str) -> Result<String, String> {
        let key = (self.resolve)(specifier, referrer).ok_or_else(|| {
            format!("spécificateur irrésolvable : {specifier} (depuis {referrer})")
        })?;
        self.map
            .get(&key)
            .cloned()
            .ok_or_else(|| format!("module non chargé : {key}"))
    }

    // L'URL absolue résolue devient l'identité du module :
    // `import.meta.url` absolu — `new URL(x, import.meta.url)`
    // fonctionne (avant : spécificateur relatif `./chunk.js` →
    // « Invalid base URL ») — et sert de référent aux imports
    // qu'il émet.
    fn resolve(&mut self, specifier: &str, referrer: &str) -> Option<String> {
        (self.resolve)(specifier, referrer)
    }
}

/// `XhrTransport` du thread moteur (JS-C27) : `send`/`abort`
/// n'exécutent rien — ils empilent des [`NetRequest`] drainées
/// par la session, qui possède `HttpTransport` (modèle
/// drain-nav généralisé : jamais d'I/O depuis le moteur).
struct ChanXhr {
    /// File partagée avec `DrainNet`.
    q: std::rc::Rc<std::cell::RefCell<Vec<NetRequest>>>,
    /// Prochain handle libre — `Rc` **partagé entre tous les
    /// contextes** du moteur (étape 134 : chaque `<iframe>`
    /// a son transport ; un compteur global garantit des
    /// handles uniques, rendant `Abort { handle }` non
    /// ambigu sans tag ctx).
    next: std::rc::Rc<std::cell::Cell<u64>>,
}

impl explorer_jscore::XhrTransport for ChanXhr {
    fn send(&self, req: &explorer_jscore::XhrRequest) -> u64 {
        let handle = self.next.get();
        self.next.set(handle + 1);
        self.q.borrow_mut().push(NetRequest::Fetch {
            handle,
            method: req.method.clone(),
            url: req.url.clone(),
            headers: req.headers.clone(),
            body: req.body.clone(),
            mode: req.mode,
            credentials: req.credentials,
            ctx: req.ctx,
        });
        handle
    }

    fn abort(&self, handle: u64) {
        self.q.borrow_mut().push(NetRequest::Abort { handle });
    }
}

/// `WorkerLoader` des threads worker (JS-C132) — même modèle
/// « jamais d'I/O dans le moteur » que [`ChanXhr`], mais
/// **synchrone-bloquant** : `load` exige la source tout de
/// suite (`new Worker`/`importScripts` sont synchrones dans
/// le scope). La requête part sur un canal `Send` drainé par
/// `drain_net_requests` ; la réponse revient par le canal
/// one-shot de la requête — écrite par la tâche fetch de la
/// session, sans passer par le thread moteur (qui peut être
/// le demandeur bloqué — premier `load` du `worker_ctor`).
struct ChanWorker {
    /// File `Send` partagée avec `JscoreEngine::worker_rx`.
    tx: std::sync::mpsc::Sender<NetRequest>,
}

/// Borne d'attente d'une source worker : au-delà, `load`
/// échoue en `TypeError` plutôt que de geler le thread worker
/// indéfiniment (session morte, fetch orphelin).
const WORKER_LOAD_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

impl explorer_jscore::WorkerLoader for ChanWorker {
    /// `new Worker` — chemin non bloquant : la requête part,
    /// le receveur est emporté par le slot `pending` et la
    /// source arrive à `pump_workers` (spawn différé, JS-C132).
    fn request(
        &mut self,
        spec: &str,
        referrer: &str,
    ) -> std::sync::mpsc::Receiver<std::result::Result<String, String>> {
        let (rtx, rrx) = std::sync::mpsc::channel();
        if self
            .tx
            .send(NetRequest::WorkerLoad {
                url: spec.to_string(),
                referrer: referrer.to_string(),
                ctx: JS_CTX_TOP,
                reply: WorkerReply(rtx.clone()),
            })
            .is_err()
        {
            let _ = rtx.send(Err("file session fermée".to_string()));
        }
        rrx
    }

    /// `importScripts` dans le thread worker — synchrone par
    /// nature ; le thread est dédié, l'attente est bornée.
    fn load(&mut self, spec: &str, referrer: &str) -> std::result::Result<String, String> {
        self.request(spec, referrer)
            .recv_timeout(WORKER_LOAD_TIMEOUT)
            .unwrap_or_else(|_| Err("fetch worker : délai dépassé".to_string()))
    }
}

/// Fournisseur `crypto.subtle` vers la session (JS-C79) — même
/// couture que [`ChanXhr`] : la requête est enfilée en
/// `NetRequest::Crypto`, la réponse revient par
/// `Cmd::DeliverCrypto`. Le calcul vit côté plateforme
/// (`explorer-net`), jamais dans le moteur.
struct ChanCrypto {
    /// File partagée avec `DrainNet`.
    q: std::rc::Rc<std::cell::RefCell<Vec<NetRequest>>>,
    /// Prochain handle libre — `Rc` partagé entre contextes
    /// (même raison que [`ChanXhr::next`] : handles uniques
    /// dans tout le moteur).
    next: std::rc::Rc<std::cell::Cell<u64>>,
}

impl explorer_jscore::CryptoTransport for ChanCrypto {
    fn submit(&self, req: explorer_jscore::CryptoRequest) -> u64 {
        let handle = self.next.get();
        self.next.set(handle + 1);
        self.q.borrow_mut().push(NetRequest::Crypto {
            handle,
            op: req.op,
            data: req.data,
            key: req.key,
        });
        handle
    }
}

/// Fournisseur `EventSource` vers la session (JS-C92) — même
/// couture que [`ChanXhr`] : ouverture = `NetRequest::
/// EventSource` (streamée, multi-livraisons), `close()` =
/// `NetRequest::Abort` — le handle vit dans l'espace `xhr`
/// pour que l'abort tue la tâche streamée comme un fetch.
struct ChanEs {
    /// File partagée avec `DrainNet`.
    q: std::rc::Rc<std::cell::RefCell<Vec<NetRequest>>>,
    /// Prochain handle libre — partagé avec `ChanXhr` (les
    /// `Abort { handle }` doivent rester non ambigus).
    next: std::rc::Rc<std::cell::Cell<u64>>,
}

impl explorer_jscore::EsTransport for ChanEs {
    fn open(&self, req: explorer_jscore::EsRequest) -> u64 {
        let handle = self.next.get();
        self.next.set(handle + 1);
        self.q.borrow_mut().push(NetRequest::EventSource {
            handle,
            url: req.url,
            with_credentials: req.with_credentials,
            ctx: req.ctx,
        });
        handle
    }

    fn abort(&self, handle: u64) {
        self.q.borrow_mut().push(NetRequest::Abort { handle });
    }
}

/// Dial `WebSocket` vers la session (JS-C94) — même couture
/// que [`ChanEs`] : `open` = `NetRequest::WsOpen` (le
/// handshake réel vit côté plateforme, socket dédiée hors
/// pool), `send`/`close` = `WsSend`/`WsClose`. Les événements
/// reviennent par `Cmd::DeliverWs` — multi-coups comme SSE.
struct ChanWs {
    /// File partagée avec `DrainNet`.
    q: std::rc::Rc<std::cell::RefCell<Vec<NetRequest>>>,
    /// Prochain handle libre — partagé avec `ChanXhr`/`ChanEs`.
    next: std::rc::Rc<std::cell::Cell<u64>>,
}

impl explorer_jscore::WsTransport for ChanWs {
    fn open(&self, req: &explorer_jscore::WsRequest) -> u64 {
        let handle = self.next.get();
        self.next.set(handle + 1);
        self.q.borrow_mut().push(NetRequest::WsOpen {
            handle,
            url: req.url.clone(),
            protocols: req.protocols.clone(),
            ctx: req.ctx,
        });
        handle
    }

    fn send(&self, handle: u64, out: explorer_jscore::WsOut) {
        self.q.borrow_mut().push(NetRequest::WsSend { handle, out });
    }

    fn close(&self, handle: u64, code: u16, reason: &str) {
        self.q.borrow_mut().push(NetRequest::WsClose {
            handle,
            code,
            reason: reason.to_string(),
        });
    }
}

/// Jar `document.cookie` thread-local (JS-C27) — le vrai jar
/// (`explorer-net`, politiques Secure/SameSite/HttpOnly) vit
/// côté session : le moteur reçoit un snapshot `« k=v »` et
/// miroire les écritures pour une lecture post-écriture
/// cohérente ; la session draine les écritures brutes.
#[derive(Default)]
struct SnapshotJar {
    /// Vue `« nom → valeur »` du snapshot + écritures mises
    /// en miroir (le `set` n'interprète que le premier
    /// segment `k=v` — les attrs relèvent du vrai jar).
    map: std::cell::RefCell<std::collections::BTreeMap<String, String>>,
    /// Écritures brutes `« k=v; attrs… »` — drainées.
    writes: std::cell::RefCell<Vec<String>>,
}

impl SnapshotJar {
    /// Fragment `« k=v; k2=v2 »` pour `document.cookie`.
    fn header(&self) -> String {
        self.map
            .borrow()
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("; ")
    }

    /// Remplace la vue par le snapshot de la session.
    fn set_header(&self, header: &str) {
        let mut m = self.map.borrow_mut();
        m.clear();
        for pair in header.split(';') {
            if let Some((k, v)) = pair.trim().split_once('=') {
                m.insert(k.trim().to_string(), v.trim().to_string());
            }
        }
    }
}

impl explorer_jscore::CookieJar for SnapshotJar {
    fn get(&self, _url: &str) -> String {
        self.header()
    }

    fn set(&self, _url: &str, cookie: &str) {
        self.writes.borrow_mut().push(cookie.to_string());
        if let Some((k, v)) = cookie
            .split(';')
            .next()
            .and_then(|p| p.trim().split_once('='))
        {
            self.map
                .borrow_mut()
                .insert(k.trim().to_string(), v.trim().to_string());
        }
    }
}
