// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Mini-DOM autonome — la structure d'arbre derrière la façade
//! `document`/`Element` (JS-2).
//!
//! Ce n'est **pas** le doctree du navigateur : c'est un DOM miroir
//! autonome, sans dépendance sur `dillo-html`/`dillo-dw`, pensé pour
//! les scripts « old school » (lire/écrire `innerHTML`, toggles de
//! classes, formulaires). Le bridge vers le vrai `Doctree` (JS-5)
//! remplacera l'alimentation de cette structure, pas l'API.
//!
//! Le parseur de fragments est volontairement *tolérant-simple* : il
//! n'a pas vocation à reproduire la récupération d'erreurs de
//! `html.cc` — c'est un DOM de script, pas le moteur de rendu.

use crate::value::{ObjId, Value};
use std::cell::Cell;
use std::collections::{HashMap, HashSet};

/// Arena handle.
pub type NodeId = u32;

/// Options d'enregistrement d'un `MutationObserver` (JS-C26) —
/// capturées par `observe(target, options)` ; chaque drapeau
/// correspond à un membre de `MutationObserverInit`.
#[derive(Debug, Clone)]
#[allow(clippy::struct_excessive_bools)] // options spec, flags are flags.
pub struct MutObsReg {
    /// Id de l'observer (alloué par le moteur — un observer
    /// peut observer plusieurs cibles).
    pub id: u64,
    /// `childList`.
    pub child_list: bool,
    /// `attributes` — `true` implicite si `attributeFilter` ou
    /// `attributeOldValue` est fourni (spec).
    pub attributes: bool,
    /// `characterData` — `true` implicite si
    /// `characterDataOldValue` est fourni.
    pub character_data: bool,
    /// `subtree` — mutations des descendants retenues.
    pub subtree: bool,
    /// `attributeFilter` — noms d'attributs retenus (`None` =
    /// tous ; présent ⇒ `attributes` implicite).
    pub attribute_filter: Option<Vec<String>>,
    /// `attributeOldValue` — la valeur d'avant est capturée.
    pub attribute_old_value: bool,
    /// `characterDataOldValue`.
    pub character_data_old_value: bool,
}

impl MutObsReg {
    /// Ce type de record est-il retenu par les options ?
    fn wants(&self, rec: &MutationRecord) -> bool {
        match rec.ty {
            "childList" => self.child_list,
            "attributes" => {
                self.attributes
                    && self.attribute_filter.as_ref().is_none_or(|f| {
                        rec.attr_name
                            .as_ref()
                            .is_some_and(|n| f.iter().any(|x| x == n))
                    })
            }
            "characterData" => self.character_data,
            _ => false,
        }
    }

    /// `oldValue` retenu pour ce type de record ?
    fn keeps_old(&self, rec: &MutationRecord) -> bool {
        match rec.ty {
            "attributes" => self.attribute_old_value,
            "characterData" => self.character_data_old_value,
            _ => false,
        }
    }
}

/// Un `MutationRecord` en file — matérialisé en objet JS à la
/// livraison (microtask checkpoint).
#[derive(Debug, Clone)]
pub struct MutationRecord {
    /// `"childList"` | `"attributes"` | `"characterData"`.
    pub ty: &'static str,
    /// Nœud sur lequel la mutation a eu lieu.
    pub target: NodeId,
    /// `childList` : nœuds ajoutés / retirés.
    pub added: Vec<NodeId>,
    /// `childList` : nœuds retirés.
    pub removed: Vec<NodeId>,
    /// Frère précédent / suivant du bloc inséré ou retiré.
    pub prev_sibling: Option<NodeId>,
    /// Voir [`MutationRecord::prev_sibling`].
    pub next_sibling: Option<NodeId>,
    /// `attributes` : nom de l'attribut muté.
    pub attr_name: Option<String>,
    /// Valeur d'avant — déjà filtrée par observer
    /// (`*OldValue` non demandé → `None` à l'enfilage).
    pub old_value: Option<String>,
}

impl MutationRecord {
    /// Record `childList` sur `target`.
    fn child_list(
        target: NodeId,
        added: Vec<NodeId>,
        removed: Vec<NodeId>,
        prev_sibling: Option<NodeId>,
        next_sibling: Option<NodeId>,
    ) -> Self {
        Self {
            ty: "childList",
            target,
            added,
            removed,
            prev_sibling,
            next_sibling,
            attr_name: None,
            old_value: None,
        }
    }
}

/// Définition `customElements.define` (JS-C68) — figée à
/// l'enregistrement : constructeur JS + `observedAttributes`
/// lu une fois (spec).
#[derive(Debug)]
pub struct CeDef {
    /// Constructeur enregistré (`function`/`class` JS) — racine
    /// GC via `Engine::collect` (même traitement que
    /// `Node.listeners`).
    pub ctor: Value,
    /// `Ctor.observedAttributes` figé — noms minuscules.
    pub observed: Vec<String>,
}

/// Registre des custom elements du document (JS-C68) —
/// `Dom`-local : les mutateurs de `Dom` y lisent les
/// définitions pour émettre les [`CeReaction`].
#[derive(Debug, Default)]
pub struct CeRegistry {
    /// Nom (`x-foo` minuscule) → définition.
    pub defs: HashMap<String, CeDef>,
    /// Nœud → nom de sa définition (état spec « custom » —
    /// l'élément garde son proto/deps après détachement).
    pub upgraded: HashMap<NodeId, String>,
    /// Nœuds dont l'upgrade a échoué (ctor throw ou retour
    /// ≠ élément) — état spec « failed » : callbacks désarmés.
    pub failed: HashSet<NodeId>,
    /// `customElements.whenDefined` pendantes : nom → ids de
    /// promesses résolues au `define` correspondant.
    pub pending: HashMap<String, Vec<ObjId>>,
}

/// Réaction custom element produite par une mutation `Dom`
/// (JS-C68) — drainée **synchrone** par le moteur après
/// chaque dispatch host (spec « custom element reactions » :
/// les callbacks s'exécutent dans l'opération, pas en
/// microtask).
#[derive(Debug)]
pub enum CeReaction {
    /// `attached` a basculé → `connectedCallback` — et upgrade
    /// à l'insertion si le nœud matche une définition encore
    /// non appliquée (le drain décide).
    Connected(NodeId),
    /// Détachement → `disconnectedCallback` (nœud déjà
    /// customisé seulement — les non-upgradés n'ont rien à
    /// dire).
    Disconnected(NodeId),
    /// Attribut observé muté sur élément customisé →
    /// `attributeChangedCallback(name, oldValue, newValue)`.
    AttrChanged {
        /// Élément customisé.
        node: NodeId,
        /// Nom d'attribut (minuscule).
        name: String,
        /// Valeur d'avant (`null` en callback).
        old: Option<String>,
        /// Valeur d'après (`null` = retrait).
        new: Option<String>,
    },
}

/// Kind of node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    /// Element (`<div>`…). `tag` is lowercased; `"#document"` is the
    /// synthetic root.
    Element,
    /// Text run — payload in `text`, `tag` is `"#text"`.
    Text,
    /// `<!-- … -->` comment — payload in `text`, `tag` is
    /// `"#comment"`, `nodeType` 8 (Lit template markers —
    /// vaadin terrain).
    Comment,
    /// `DocumentFragment` (`nodeType` 11) — `tag` is
    /// `"#document-fragment"`. Spec insertion : appending a
    /// fragment splices its children into the parent and leaves
    /// the fragment empty (trigger réel : jQuery
    /// `createDocumentFragment`).
    Fragment,
}

/// One node in the arena.
#[derive(Debug)]
pub struct Node {
    /// Element or text.
    pub kind: NodeKind,
    /// Lowercased tag name (`"#document"`, `"#text"` for non-elements).
    pub tag: String,
    /// Attributes in source order.
    pub attrs: Vec<(String, String)>,
    /// Child node ids, in order.
    pub children: Vec<NodeId>,
    /// Parent (none for the root / detached nodes).
    pub parent: Option<NodeId>,
    /// Text payload (kind == Text).
    pub text: String,
    /// `addEventListener` handlers: `(type, function, capture)` —
    /// dispatched by the host layer / platform events (JS-3).
    pub listeners: Vec<(String, Value, bool)>,
    /// Handlers inline `on*` compilés : `(nom d'attribut,
    /// source, fonction)` — le cache est invalidé par la
    /// **source** (JS-C45) : `setAttribute`/resync qui change
    /// le texte force la recompilation au prochain dispatch.
    /// Même cycle de vie que `listeners` (ni clonés ni
    /// sérialisés — morts avec le miroir).
    pub inline_handlers: Vec<(String, String, Value)>,
    /// Rattaché au document (`root` accessible) — drapeau
    /// maintenu par [`Dom::set_attached`] aux re-parentages
    /// (append/detach/insert/replace). Lecture O(1) pour
    /// `is_attached`/`mark_dirty_at` : la remontée `parent`
    /// par mutation rendait le parsing profond quadratique
    /// (JS-C55).
    pub attached: bool,
}

/// Opération `CanvasRenderingContext2D` enregistrée sur le
/// miroir (JS-C28) — miroir sérialisable de
/// `explorer_render::canvas::CanvasOp` (couture R-11) : la
/// couche navigateur la traduit champ pour champ. `explorer-jscore`
/// n'a aucune dépendance interne (ADR-0007) — les couleurs
/// voyagent en `rgba8`.
#[derive(Debug, Clone, PartialEq)]
pub enum CanvasOpRecord {
    /// `save()` — empile l'état.
    Save,
    /// `restore()` — dépile l'état.
    Restore,
    /// `translate(x, y)`.
    Translate(f32, f32),
    /// `scale(x, y)`.
    Scale(f32, f32),
    /// `rotate(rad)`.
    Rotate(f32),
    /// `transform(a, b, c, d, e, f)`.
    Transform(f32, f32, f32, f32, f32, f32),
    /// `setTransform(a, b, c, d, e, f)`.
    SetTransform(f32, f32, f32, f32, f32, f32),
    /// `fillStyle = couleur`.
    FillColor {
        /// Rouge.
        r: u8,
        /// Vert.
        g: u8,
        /// Bleu.
        b: u8,
        /// Alpha.
        a: u8,
    },
    /// `strokeStyle = couleur`.
    StrokeColor {
        /// Rouge.
        r: u8,
        /// Vert.
        g: u8,
        /// Bleu.
        b: u8,
        /// Alpha.
        a: u8,
    },
    /// `globalAlpha = v`.
    GlobalAlpha(f32),
    /// `lineWidth = px`.
    LineWidth(f32),
    /// `beginPath()`.
    BeginPath,
    /// `moveTo(x, y)`.
    MoveTo(f32, f32),
    /// `lineTo(x, y)`.
    LineTo(f32, f32),
    /// `quadraticCurveTo(cx, cy, x, y)`.
    QuadraticTo(f32, f32, f32, f32),
    /// `bezierCurveTo(c1x, c1y, c2x, c2y, x, y)`.
    BezierTo(f32, f32, f32, f32, f32, f32),
    /// `arc(cx, cy, r, start_rad, end_rad, anticlockwise)`.
    Arc(f32, f32, f32, f32, f32, bool),
    /// `rect(x, y, w, h)`.
    Rect(f32, f32, f32, f32),
    /// `closePath()`.
    ClosePath,
    /// `fill()`.
    Fill,
    /// `stroke()`.
    Stroke,
    /// `clip()`.
    Clip,
    /// `fillRect(x, y, w, h)`.
    FillRect(f32, f32, f32, f32),
    /// `strokeRect(x, y, w, h)`.
    StrokeRect(f32, f32, f32, f32),
    /// `clearRect(x, y, w, h)`.
    ClearRect(f32, f32, f32, f32),
}

impl CanvasOpRecord {
    /// `false` si un scalaire est NaN/infini — spec canvas :
    /// l'appel est alors ignoré silencieusement.
    fn args_finite(&self) -> bool {
        use CanvasOpRecord as C;
        match *self {
            C::Translate(x, y) | C::Scale(x, y) | C::MoveTo(x, y) | C::LineTo(x, y) => {
                x.is_finite() && y.is_finite()
            }
            C::Rotate(r) | C::GlobalAlpha(r) | C::LineWidth(r) => r.is_finite(),
            C::QuadraticTo(a, b, c, d)
            | C::Rect(a, b, c, d)
            | C::FillRect(a, b, c, d)
            | C::StrokeRect(a, b, c, d)
            | C::ClearRect(a, b, c, d) => {
                a.is_finite() && b.is_finite() && c.is_finite() && d.is_finite()
            }
            C::Arc(x, y, r, sa, ea, _) => {
                x.is_finite() && y.is_finite() && r.is_finite() && sa.is_finite() && ea.is_finite()
            }
            C::BezierTo(a, b, c, d, e, f)
            | C::Transform(a, b, c, d, e, f)
            | C::SetTransform(a, b, c, d, e, f) => {
                a.is_finite()
                    && b.is_finite()
                    && c.is_finite()
                    && d.is_finite()
                    && e.is_finite()
                    && f.is_finite()
            }
            _ => true,
        }
    }
}

/// État du contexte 2D attaché à un `<canvas>` (JS-C28) —
/// l'op-list est **cumulative** (la bitmap persiste entre
/// les re-peintures) : elle n'est jamais drainée, seulement
/// relue par [`Dom::canvas_ops`].
#[derive(Debug)]
pub struct CanvasState {
    /// Op-list enregistrée, dans l'ordre d'appel JS.
    pub ops: Vec<CanvasOpRecord>,
    /// Objet contexte JS — `getContext("2d")` renvoie
    /// toujours la même valeur (spec : un contexte par mode).
    pub ctx_value: Option<Value>,
    /// `fillStyle` courant (pour le getter JS).
    pub fill: (u8, u8, u8, u8),
    /// `strokeStyle` courant.
    pub stroke: (u8, u8, u8, u8),
    /// `globalAlpha` courant.
    pub alpha: f32,
    /// `lineWidth` courant.
    pub line_w: f32,
}

impl Default for CanvasState {
    fn default() -> Self {
        Self {
            ops: Vec::new(),
            ctx_value: None,
            fill: (0, 0, 0, 255),
            stroke: (0, 0, 0, 255),
            alpha: 1.0,
            line_w: 1.0,
        }
    }
}

/// Borne par défaut d'ops enregistrées par `<canvas>` —
/// chaque record ≈ 24–48 o : quelques Mio au pire, et le
/// playback R-11 reste linéaire (règle « budgets » — boucles
/// de dessin hostiles).
const DEFAULT_CANVAS_OP_LIMIT: usize = 65_536;
/// Borne d`iframes dynamiques attachés par onglet (B-29) —
/// AdSense `sodar` en crée en continu (~90/min, heap +40 Mo/min) ;
/// au-delà l`élément monte sans realm (mort douce télémétrie).
pub(crate) const MAX_DYN_FRAMES: usize = 64;

/// Snapshot « chemin élément → op-list cumulée » exporté par
/// [`Dom::canvas_ops`] (JS-C28) — même format de chemin que
/// `take_dirty_path`.
pub type CanvasOps = Vec<(Vec<(String, u32)>, Vec<CanvasOpRecord>)>;

/// État de lecture rapporté par l'hôte média (étape 71c,
/// ADR-0024) — miroir du DTO session ; le moteur ne décode
/// rien, il reflète ce que l'hôte claimé rapporte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaState {
    /// Lecture en cours.
    Playing,
    /// En pause.
    Paused,
    /// Fin de flux atteinte.
    Ended,
    /// Erreur hôte — `MediaReport::error` porte la cause.
    Error,
}

/// Rapport `state` hôte → moteur (étape 71c) — positions en
/// millisecondes, `None` duration = flux sans durée connue.
#[derive(Debug, Clone)]
pub struct MediaReport {
    /// Tête de lecture (ms).
    pub position_ms: u64,
    /// Durée totale (ms) si connue.
    pub duration_ms: Option<u64>,
    /// Fin de la zone tampon (ms).
    pub buffered_ms: u64,
    /// État de lecture.
    pub state: MediaState,
    /// Cause si `state == Error`.
    pub error: Option<String>,
}

/// Ordre média page → hôte, drainé par l'embedder qui le
/// relaie à l'hôte claimé (étape 71c — `MediaCommand` SSE).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MediaCmd {
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

/// État `HTMLMediaElement` claimé (étape 71c) — présent dans
/// [`Dom::media`] tant que l'hôte détient la surface ; la
/// release purge l'entrée → retour aux stubs honnêtes
/// (ADR-0023 : jamais de faux succès).
#[derive(Debug)]
pub struct MediaElState {
    /// `media: u64` de la surface session — rappelé dans les
    /// commandes drainées (l'hôte identifie sa surface).
    pub media: u64,
    /// Codecs déclarés au claim — base de `canPlayType`.
    pub codecs: Vec<String>,
    /// Dernier rapport `state` hôte.
    pub report: Option<MediaReport>,
    /// `volume` géré localement (défaut spec 1.0).
    pub volume: f64,
    /// `playbackRate` géré localement (défaut 1.0).
    pub rate: f64,
    /// Seek en vol — le prochain rapport qui bouge la tête
    /// clôt par l'event `seeked`.
    pub seek_pending: bool,
    /// `loadedmetadata` déjà dispatché.
    pub metadata_fired: bool,
    /// Dernière durée rapportée (`durationchange` sur
    /// variation).
    pub last_duration: Option<u64>,
    /// Promesses `play()` en attente — résolues au premier
    /// rapport `playing`, rejetées sur `error`/release.
    pub pending_play: Vec<crate::value::ObjId>,
}

impl MediaElState {
    /// État frais d'une surface nouvellement claimée.
    #[must_use]
    pub fn new(media: u64, codecs: Vec<String>) -> Self {
        Self {
            media,
            codecs,
            report: None,
            volume: 1.0,
            rate: 1.0,
            seek_pending: false,
            metadata_fired: false,
            last_duration: None,
            pending_play: Vec::new(),
        }
    }
}

/// A standalone DOM tree: node arena + document root.
#[derive(Debug)]
pub struct Dom {
    /// Node arena (index = `NodeId`).
    pub nodes: Vec<Node>,
    /// The `#document` node.
    pub root: NodeId,
    /// Contextes 2D attachés aux `<canvas>` (JS-C28) — un état
    /// par surface ayant obtenu `getContext("2d")`. Pub comme
    /// `nodes` (lecture host objects) ; les mutations passent
    /// par `push_canvas_op`/`set_canvas_ctx`/`clear_canvas`
    /// (bornage + marquage sale). Meurt avec le document —
    /// le bitmap appartient à la page.
    pub canvases: HashMap<NodeId, CanvasState>,
    /// Surfaces média claimées par l'hôte (étape 71c,
    /// ADR-0024) — indexées par nœud miroir, meurent avec le
    /// document (une navigation re-claim tout avec des ids
    /// neufs). Vide tant que rien n'est claimé : les stubs
    /// honnêtes ADR-0023 s'appliquent.
    pub media: HashMap<NodeId, MediaElState>,
    /// `<template>` → son `DocumentFragment` de contenu (spec
    /// `template contents`), matérialisé paresseusement à la
    /// première lecture de `.content`/`innerHTML` — les enfants
    /// de l'élément y sont alors *déplacés* (mesuré github
    /// `z8-*.js` : `tpl.content.appendChild(...)`). Un template
    /// jamais touché garde ses enfants comme nœuds ordinaires —
    /// sérialisation équivalente.
    template_contents: HashMap<NodeId, NodeId>,
    /// Hôte → `(racine shadow, mode ouvert)` — stubs
    /// `attachShadow` (issue #25) : la racine est un
    /// `DocumentFragment` hors-arbre, juste assez pour que
    /// les polyfills d'instrumentation (Invoker Commands)
    /// attachent et lisent `shadowRoot`. Pas de scoping,
    /// pas de slots — l'arc Shadow DOM reste ouvert.
    shadow_roots: HashMap<NodeId, (NodeId, bool)>,
    /// Racine shadow → hôte (CAP-191 tranche 1) — inverse
    /// de `shadow_roots`, pour `shadowRoot.host` et
    /// `getRootNode({composed:true})`.
    shadow_hosts: HashMap<NodeId, NodeId>,
    /// `(media, cmd)`, drainés par l'adaptateur
    /// (`drain_media_commands`) qui les relaie vers l'hôte.
    pub media_cmds: Vec<(u64, MediaCmd)>,
    /// Registre custom elements (JS-C68) — les définitions
    /// pilotent l'émission des `CeReaction` par les
    /// mutateurs ; les `Value`/`ObjId` qu'il porte sont
    /// racines GC via `Engine::collect` (`dom_ctx`).
    pub ce: CeRegistry,
    /// Réactions CE en file — vidées par le moteur après
    /// chaque dispatch host (`drain_ce_reactions`).
    pub ce_reactions: Vec<CeReaction>,
    /// Borne d'ops par canvas (réglable `set_canvas_op_limit`).
    canvas_op_limit: usize,
    /// Marqué par toute mutation structurelle/attributaire — l'embedder
    /// lit (et efface) le flag pour savoir si le rendu doit être
    /// resynchronisé avec le miroir (JS-5b).
    dirty: Cell<bool>,
    /// Nœud couvrant toutes les mutations depuis la dernière
    /// synchro (étape 55f) — escaladé au plus proche ancêtre
    /// commun quand les mutations frappent des sous-arbres
    /// distincts. Sert à viser le relayout incrémental
    /// (`relayout_subtree`) au lieu d'un reflow global.
    dirty_node: Cell<Option<NodeId>>,
    /// Registrations `MutationObserver` : `cible → options`
    /// (un observer peut suivre plusieurs cibles — JS-C26).
    observers: HashMap<NodeId, Vec<MutObsReg>>,
    /// File des records produits, `(obs_id, record)` — la
    /// `oldValue` est déjà filtrée par observer à l'enfilage ;
    /// drainée au microtask checkpoint.
    mutations: Vec<(u64, MutationRecord)>,
    /// Journal de synchro différentielle (ADR-0025, étape 111)
    /// — capture **inconditionnelle** des mutations sur nœuds
    /// attachés, indépendante des `MutationObserver` (le journal
    /// alimente la resynchro moteur, la file `mutations` les
    /// listeners de la page).
    journal: Vec<JournalOp>,
    /// `true` = une mutation a échappé au journal (effacement
    /// par `document.write`, chemin non tracé) — le prochain
    /// drain déclare « re-parse » plutôt que rejouer à faux.
    journal_poisoned: bool,
    /// Ops drainées mais **non rejouées** — empoisonnement ou
    /// classement « re-parse » côté adaptateur (ADR-0025 :
    /// mesure du périmètre du pilote — un taux élevé signale
    /// des classes de mutations à ouvrir au rejeu).
    journal_dropped: u64,
    /// `<script>` « already started » (CAP-169, spec HTML
    /// « script processing model ») — un élément script ne
    /// s'exécute qu'une fois : le parse initial et
    /// `innerHTML` les pré-marquent ([`Self::mark_scripts_started`]) ;
    /// les mutateurs testent ce set avant d'enfiler dans
    /// `pending_dyn`.
    pub(crate) dyn_started: HashSet<NodeId>,
    /// Scripts `<script>` devenus attachés par mutation —
    /// en attente de fetch/éval par la session
    /// (`Engine::drain_dyn_scripts`). Ordre document.
    pub(crate) pending_dyn: Vec<NodeId>,
    /// `<script>` du parse initial : ordinal document →
    /// nœud (CAP-184). La session numérote les scripts du
    /// **vrai** DOM et `eval_script_el` doit retrouver le
    /// miroir — un lookup positionnel (`by_tag().get(ord)`)
    /// se décale dès qu'un script se `remove()` lui-même
    /// (stream `$_TSR` de x.com : chaque chunk s'auto-
    /// supprime → les ordinaires suivants pointent le script
    /// d'après puis hors-bornes → `currentScript` = `null`).
    /// Le tampon est figé au parse : la détache ultérieure
    /// n'invalide pas l'entrée — un script retiré avant son
    /// éval reste identifiable (spec : `currentScript` est
    /// l'élément, attaché ou non). Scripts dynamiques non
    /// tamponnés : jamais résolus par ordinal.
    pub(crate) script_seqs: HashMap<u32, NodeId>,
    /// `<link>` du parse initial : ordinal document → nœud
    /// (CAP-185) — même motif figé que `script_seqs`, pour
    /// livrer `load`/`error` de fin de feuille sur le bon
    /// élément (`deliver_static_link`). Ordinal parmi **tous**
    /// les `<link>` (même convention que `collect_scripts` —
    /// les rels non feuille sont tamponnés aussi, ils
    /// permettront leurs propres événements).
    pub(crate) link_seqs: HashMap<u32, NodeId>,
    /// `<iframe>` déjà « pris en charge » (CAP-171) — le
    /// parse initial pré-marque les siens (chargés par le
    /// pipeline sous-ressource statique, jamais via la file
    /// dynamique) ; une insertion ou un `src=` ultérieur
    /// ré-enfile. Contrairement aux scripts, un iframe
    /// `innerHTML` **charge** (spec — pas d'« already
    /// started » pour les cadres).
    pub(crate) dyn_frame_seen: HashSet<NodeId>,
    /// `<iframe>` devenus attachés (ou `src` modifié sur
    /// cadre attaché) — en attente de fetch par la session
    /// (`Engine::drain_dyn_frames`). Ordre document.
    pub(crate) pending_dyn_frames: Vec<NodeId>,
    /// `<img>` dont la source a été demandée (CAP-172) —
    /// nœud → clé `srcset|sizes|src` de la **dernière**
    /// requête : `img.src=` re-fetch à chaque changement
    /// (spec), l'insertion ne re-fetch pas une source
    /// identique déjà en vol. Contrairement aux cadres,
    /// une image se charge **détachée** (`new Image().src=`
    /// = prefetch, spec « update the image data »).
    pub(crate) dyn_img_seen: HashMap<NodeId, String>,
    /// `<img>` en attente de fetch par la session
    /// (`Engine::drain_dyn_imgs`) — ordre d'émission.
    pub(crate) pending_dyn_imgs: Vec<NodeId>,
    /// `<link rel=stylesheet>` dont la feuille a été
    /// demandée (CAP-173) — nœud → clé `rel|href` de la
    /// **dernière** requête : `href=`/`rel=` re-fetch à
    /// chaque changement réel ; l'insertion à clé inchangée
    /// ne re-fetch pas. Comme les cadres, un lien doit être
    /// **attaché** pour charger (spec « link processing »).
    pub(crate) dyn_link_seen: HashMap<NodeId, String>,
    /// `<link>` en attente de fetch par la session
    /// (`Engine::drain_dyn_links`) — ordre d'émission.
    pub(crate) pending_dyn_links: Vec<NodeId>,
    /// `document.adoptedStyleSheets` — `ObjId` des feuilles
    /// adoptées posées par `set` (CAP-195 tranche 2) : la
    /// session lit `cssRules[*].cssText` par feuille via
    /// [`Engine::adopted_sheets_css`] pour les cascader en
    /// feuilles auteur document (spec : après les `<link>`).
    pub(crate) adopted_sheets: Vec<ObjId>,
    /// « Document ouvert à l'écriture » (JS-C35) — `true` tant
    /// que la page se charge : `document.write`/`writeln` y
    /// ajoutent à `<body>`. `document.close()` ou la
    /// distribution de `DOMContentLoaded`/`load` le ferme ; un
    /// `write` sur document fermé rouvre implicitement en
    /// effaçant le contenu d'abord (spec `document open steps`).
    pub doc_open: bool,
    /// Élément détenant le focus du document (CAP-162) —
    /// `el.focus()`/`el.blur()`/`autofocus` le maintiennent ;
    /// `document.activeElement` le lit (repli `body`). Un nœud
    /// détaché perde le focus (`activeElement` retombe sur
    /// `body`, spec « unfocus »). `Cell` : lecture `&self`.
    pub focused: Cell<Option<NodeId>>,
    /// Slot de géométrie publié par la session (issue #19,
    /// JS-8) — la table `element_path → union des boîtes` du
    /// document **top-level**, lue en direct par les getters
    /// `getBoundingClientRect`/`offset*`/`client*`/`scroll*`
    /// de `dom_host`. Document-local par design (journal
    /// interface 2026-10-24) : posé par
    /// `Engine::set_geometry_ctx`, il vit et meurt avec le
    /// miroir — le swap de contexte est gratuit et le
    /// bfcache gèle le document **avec** sa table (mesures
    /// valides au retour, jamais celles d'un autre document).
    /// `None` = harnais sans session → zéros historiques.
    pub geometry: Option<crate::geom::GeometrySlot>,
    /// Compteurs `serialize_document` (JS-C37/C39) — appels,
    /// octets et micros produits, cumulés depuis la création.
    /// `Cell` : la sérialisation est `&self`.
    serialize_calls: Cell<u64>,
    /// Octets cumulés produits par `serialize_document`.
    serialize_bytes: Cell<u64>,
    /// Microsecondes cumulées dans `serialize_document` (JS-C39).
    serialize_micros: Cell<u64>,
}

/// Opération capturée pour la synchro différentielle
/// (ADR-0025, étape 111). Capturée à l'enregistrement —
/// `index` est la position de l'insert au moment de la
/// mutation (les ops sont rejouées en séquence, les index
/// restent valides) ; les payloads (`html`/`value`/`text`)
/// sont matérialisés **au drain** — l'état le plus récent
/// subsume les intermédiaires. `pub` : traduite en ops de
/// couture par `explorer-js` (adattateur `JsEngine`).
#[derive(Debug)]
pub enum JournalOp {
    /// `added` insérés sous `parent` à `index` (parmi tous
    /// les enfants, texte compris).
    Insert {
        /// Parent cible (attaché).
        parent: NodeId,
        /// Position du premier inséré.
        index: u32,
        /// Racines insérées (contiguës).
        added: Vec<NodeId>,
    },
    /// Sous-arbre `key` retiré du document.
    Remove {
        /// Racine retirée.
        key: NodeId,
    },
    /// Attribut `name` muté — valeur relue au drain.
    SetAttr {
        /// Élément cible.
        key: NodeId,
        /// Nom d'attribut (minuscules).
        name: String,
    },
    /// Texte muté — contenu relu au drain.
    SetText {
        /// Nœud texte cible.
        key: NodeId,
    },
    /// `attachShadow` programmatique — op de couture
    /// (Étape 137b) : sans elle, l'attache voyageait par
    /// re-parse (la garde `Fingerprint`/`NoBaseline` tombait
    /// sur une mutation non rejouable).
    AttachShadow {
        /// Hôte recevant la racine (élément attaché).
        host: NodeId,
        /// Fragment racine créé dans le miroir.
        root: NodeId,
        /// `mode: "open"` — `closed` garde `shadowRoot` nul
        /// côté JS mais se peint pareil.
        open: bool,
    },
}

impl Default for Dom {
    fn default() -> Self {
        Self::new()
    }
}

/// Compteurs cumulés de `serialize_document` (JS-C39) —
/// instrument du critère d'escalade « la synchro DOM domine-t-elle
/// le frame budget ? » (octets et temps par synchro).
#[derive(Debug, Clone, Copy, Default)]
pub struct SerializeStats {
    /// Sérialisations réellement produites.
    pub calls: u64,
    /// Octets HTML cumulés.
    pub bytes: u64,
    /// Microsecondes cumulées dans la sérialisation.
    pub micros: u64,
}

/// Plafond de nœuds du miroir (B-44) — insertions au-delà
/// refusées, déplacements attachés légaux (télémétrie sans fin).
const MAX_DOM_NODES: usize = 100_000;

impl Dom {
    /// Empty document (`#document` root only).
    #[must_use]
    pub fn new() -> Self {
        let mut d = Dom {
            nodes: Vec::new(),
            root: 0,
            dirty: Cell::new(false),
            dirty_node: Cell::new(None),
            observers: HashMap::new(),
            mutations: Vec::new(),
            journal: Vec::new(),
            journal_poisoned: false,
            journal_dropped: 0,
            canvases: HashMap::new(),
            media: HashMap::new(),
            template_contents: HashMap::new(),
            shadow_roots: HashMap::new(),
            shadow_hosts: HashMap::new(),
            media_cmds: Vec::new(),
            ce: CeRegistry::default(),
            ce_reactions: Vec::new(),
            canvas_op_limit: DEFAULT_CANVAS_OP_LIMIT,
            doc_open: true,
            dyn_started: HashSet::new(),
            pending_dyn: Vec::new(),
            script_seqs: HashMap::new(),
            link_seqs: HashMap::new(),
            dyn_img_seen: HashMap::new(),
            pending_dyn_imgs: Vec::new(),
            dyn_link_seen: HashMap::new(),
            pending_dyn_links: Vec::new(),
            adopted_sheets: Vec::new(),
            dyn_frame_seen: HashSet::new(),
            pending_dyn_frames: Vec::new(),
            focused: Cell::new(None),
            geometry: None,
            serialize_calls: Cell::new(0),
            serialize_bytes: Cell::new(0),
            serialize_micros: Cell::new(0),
        };
        d.root = d.alloc(NodeKind::Element, "#document");
        d.nodes[d.root as usize].attached = true;
        d
    }

    // -- MutationObserver (JS-C26) ------------------------------------

    /// `id < len` — un id obsolète (arène reconstruite sous un
    /// handle JS vivant : resync du miroir pendant un storm
    /// d'appends, spectrum `groupChildren` → `nodes[614]` sur
    /// arène 507) n'est pas un nœud valide — l'appelant dégrade
    /// en TypeError/no-op, jamais panic.
    #[must_use]
    pub fn node_valid(&self, node: NodeId) -> bool {
        (node as usize) < self.nodes.len()
    }

    /// `observer.observe(target, options)` — une registration
    /// par `(id, cible)` ; re-`observe` remplace les options.
    pub fn observe_mutations(&mut self, target: NodeId, reg: MutObsReg) {
        if target as usize >= self.nodes.len() {
            return;
        }
        let regs = self.observers.entry(target).or_default();
        regs.retain(|r| r.id != reg.id);
        regs.push(reg);
    }

    /// `observer.disconnect()` — retire toutes les
    /// registrations de `id` **et** purge ses records en file
    /// (spec : disconnect vide aussi la queue).
    pub fn disconnect_observer(&mut self, id: u64) {
        for regs in self.observers.values_mut() {
            regs.retain(|r| r.id != id);
        }
        self.mutations.retain(|(o, _)| *o != id);
    }

    /// `observer.takeRecords()` — purge et rend la file d'un
    /// observer (les autres conservent la leur).
    pub fn take_mutation_records(&mut self, id: u64) -> Vec<MutationRecord> {
        let mut out = Vec::new();
        let mut rest = Vec::new();
        for (o, r) in std::mem::take(&mut self.mutations) {
            if o == id {
                out.push(r);
            } else {
                rest.push((o, r));
            }
        }
        self.mutations = rest;
        out
    }

    /// Toute la file, drainée — le groupage par observer est
    /// à la charge du moteur (microtask checkpoint).
    pub fn drain_mutations(&mut self) -> Vec<(u64, MutationRecord)> {
        std::mem::take(&mut self.mutations)
    }

    // -- Journal de synchro (ADR-0025) ---------------------------------

    /// Draine le journal de mutations — `None` si empoisonné
    /// (un chemin non tracé a muté l'arbre : re-parse requis).
    /// L'appelant (moteur) traduit les [`JournalOp`] en ops de
    /// couture — il possède la sémantique de sérialisation.
    /// `pub` pour `explorer-js` (adaptateur `JsEngine`).
    pub fn take_journal(&mut self) -> Option<Vec<JournalOp>> {
        if self.journal_poisoned {
            self.journal_dropped += self.journal.len() as u64;
            self.journal.clear();
            self.journal_poisoned = false;
            return None;
        }
        Some(std::mem::take(&mut self.journal))
    }

    /// Comptabilise `n` ops drainées mais classées « re-parse »
    /// par l'adaptateur — même cumul que le drain empoisonné.
    /// `pub` pour `explorer-js` (adaptateur `JsEngine`).
    pub fn note_journal_classified(&mut self, n: u64) {
        self.journal_dropped += n;
    }

    /// Ops du journal tombées en « re-parse » (empoisonné +
    /// classé), cumulées — mesure d'adoption ADR-0025.
    #[must_use]
    pub fn journal_dropped(&self) -> u64 {
        self.journal_dropped
    }

    /// Vide le journal sans le consommer — appelé quand une
    /// baseline complète (sérialisation cléée) subsume les
    /// mutations antérieures.
    /// `pub` pour `explorer-js` (adaptateur `JsEngine`).
    pub fn clear_journal(&mut self) {
        self.journal.clear();
        self.journal_poisoned = false;
    }

    /// Marque le journal non rejouable — mutation par un chemin
    /// non tracé (ex. effacement `document.write` post-`load`).
    /// `pub` pour `explorer-js` (adaptateur `JsEngine`).
    pub fn poison_journal(&mut self) {
        self.journal_poisoned = true;
    }

    /// Empreinte structurelle du document : fold pré-ordre des
    /// **tags d'éléments** (texte exclu — ADR-0025 : « tag/ordre,
    /// pas de texte »). Suit `serial_children` — le contenu d'un
    /// `<template>` est compté (le DOM moteur le descend via
    /// `template_contents`, même ordre). Itératif (JS-C55).
    /// Casse normalisée : le DOM moteur garde la casse SVG
    /// (`clipPath`) là où le miroir stocke `clippath` —
    /// l'empreinte mesure la structure, pas la représentation.
    #[must_use]
    pub fn structural_fingerprint(&self) -> u64 {
        // FNV-1a sur les octets de tag + un séparateur par nœud —
        // l'ordre structure la valeur (insertion/rotation distinctes).
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        let mut stack = self.serial_children(self.root).to_vec();
        stack.reverse();
        while let Some(id) = stack.pop() {
            let n = &self.nodes[id as usize];
            if n.kind == NodeKind::Element {
                for b in n.tag.bytes().map(|b| b.to_ascii_lowercase()) {
                    h ^= u64::from(b);
                    h = h.wrapping_mul(0x0000_0100_0000_01b3);
                }
                h ^= 0xff;
                h = h.wrapping_mul(0x0000_0100_0000_01b3);
            }
            for &c in self.serial_children(id).iter().rev() {
                stack.push(c);
            }
            // R-96 — le fragment shadow compte comme le
            // `<template>` émis par `serialize_keyed` (même
            // ordre : enfants shadow avant light).
            if n.kind == NodeKind::Element {
                if let Some(&(root, _)) = self.shadow_roots.get(&id) {
                    for b in b"template" {
                        h ^= u64::from(*b);
                        h = h.wrapping_mul(0x0000_0100_0000_01b3);
                    }
                    h ^= 0xff;
                    h = h.wrapping_mul(0x0000_0100_0000_01b3);
                    for &c in self.nodes[root as usize].children.iter().rev() {
                        stack.push(c);
                    }
                }
            }
        }
        h
    }

    /// Sérialisation cléée du document (ADR-0025) : `(html,
    /// ids, tags)` — `ids` = `NodeId` miroir de chaque nœud
    /// dans l'ordre d'émission (pré-ordre de `serialize`,
    /// fragments aplatis sans entrée propre) ; `tags` est
    /// parallèle à `ids` : tag pour un élément,
    /// `"#text\x00{fnv64(contenu)}"` pour un texte (le hash
    /// rend l'alignement par contenu possible sans
    /// dupliquer le texte). La session apparie les `NodeId`
    /// frais de son re-parse **par tag** pour monter la
    /// table d'identité — robuste aux normalisations du
    /// parse moteur (`<tbody>` synthétisé…).
    #[must_use]
    pub fn serialize_keyed(&self) -> (String, Vec<u64>, Vec<String>) {
        let mut out = String::new();
        let mut ids = Vec::new();
        let mut tags = Vec::new();
        for &c in self.serial_children(self.root) {
            self.serialize_keyed_node(c, &mut out, &mut ids, &mut tags);
        }
        (out, ids, tags)
    }

    /// Variante sous-arbre de [`Self::serialize_keyed`] —
    /// utilisée par le drain de journal pour les `Insert`
    /// (`outer_html` + ids + tags de tout le sous-arbre — les
    /// tags permettent au rejeu un appariement **par tag** plutôt
    /// que positionnel : une synthèse moteur (`<tbody>`…) ou un
    /// nœud non émis ne décale plus toute la table — JS-C129).
    #[must_use]
    pub fn serialize_subtree_keyed(&self, node: NodeId) -> (String, Vec<u64>, Vec<String>) {
        let mut out = String::new();
        let mut ids = Vec::new();
        let mut tags = Vec::new();
        self.serialize_keyed_node(node, &mut out, &mut ids, &mut tags);
        (out, ids, tags)
    }

    /// Traversée unique `serialize` + collecte d'ids — mêmes
    /// événements `Enter` que [`Self::serialize`], enregistrés.
    /// `tags` est poussé en parallèle de `ids`.
    fn serialize_keyed_node(
        &self,
        node: NodeId,
        out: &mut String,
        ids: &mut Vec<u64>,
        tags: &mut Vec<String>,
    ) {
        enum Ser {
            Enter(NodeId),
            Exit(NodeId),
            Rawtext(NodeId),
            /// Balise littérale — `<template shadowrootmode>`
            /// synthétique d'une racine shadow (R-96) : le
            /// fragment n'a pas de tag propre à émettre.
            Lit(&'static str),
        }
        let mut stack = vec![Ser::Enter(node)];
        while let Some(ev) = stack.pop() {
            match ev {
                Ser::Lit(s) => out.push_str(s),
                Ser::Exit(id) => {
                    let n = &self.nodes[id as usize];
                    out.push_str("</");
                    out.push_str(&n.tag);
                    out.push('>');
                }
                Ser::Rawtext(id) => {
                    ids.push(u64::from(id));
                    let child = &self.nodes[id as usize];
                    tags.push(format!("#text\x00{:016x}", fnv(&child.text)));
                    let child = &self.nodes[id as usize];
                    let parent = &self.nodes[child.parent.unwrap_or(id) as usize];
                    let marker = format!("</{}", parent.tag);
                    if contains_ci(&child.text, &marker) {
                        out.push_str(&escape_text(&child.text));
                    } else {
                        out.push_str(&child.text);
                    }
                }
                Ser::Enter(id) => {
                    let n = &self.nodes[id as usize];
                    if n.kind == NodeKind::Text {
                        ids.push(u64::from(id));
                        tags.push(format!("#text\x00{:016x}", fnv(&n.text)));
                        out.push_str(&escape_text(&n.text));
                        continue;
                    }
                    // Commentaire : pas de reflet moteur
                    // (`serializable_kind` côté apply ne garde que
                    // Element|Text) — ni markup `<#comment>`
                    // (re-parsé en élément fantôme peint en
                    // texte), ni entrée `ids`/`tags` (le zip
                    // `engine_preorder` ↔ `ids` dériverait d'un
                    // cran par commentaire).
                    if n.kind == NodeKind::Comment {
                        continue;
                    }
                    if n.kind == NodeKind::Fragment {
                        // Aucun nœud moteur émis — id non listé
                        // (les enfants le sont).
                        for &c in n.children.iter().rev() {
                            stack.push(Ser::Enter(c));
                        }
                        continue;
                    }
                    ids.push(u64::from(id));
                    tags.push(n.tag.clone());
                    out.push('<');
                    out.push_str(&n.tag);
                    for (k, v) in &n.attrs {
                        out.push(' ');
                        out.push_str(k);
                        out.push_str("=\"");
                        out.push_str(&escape_attr(v));
                        out.push('"');
                    }
                    out.push('>');
                    if VOID_TAGS.contains(&n.tag.as_str()) {
                        continue;
                    }
                    stack.push(Ser::Exit(id));
                    let verbatim = RAW_VERBATIM.contains(&n.tag.as_str());
                    for &c in self.serial_children(id).iter().rev() {
                        let child = &self.nodes[c as usize];
                        if verbatim && child.kind == NodeKind::Text {
                            stack.push(Ser::Rawtext(c));
                        } else {
                            stack.push(Ser::Enter(c));
                        }
                    }
                    // R-96 — transport DSD : une racine shadow
                    // s'émet en `<template shadowrootmode>` EN
                    // TÊTE des enfants (le re-parse moteur la
                    // réattache ; le fragment porte l'id, aligné
                    // keymap sur l'élément template canonique).
                    // Empilé APRES les enfants light → pop avant
                    // eux : l'ordre émis est [shadow][light].
                    if let Some(&(root, open)) = self.shadow_roots.get(&id) {
                        let mode = if open {
                            "<template shadowrootmode=\"open\">"
                        } else {
                            "<template shadowrootmode=\"closed\">"
                        };
                        stack.push(Ser::Lit("</template>"));
                        // Les enfants de la racine (table
                        // latérale) — même ordre pré-ordre que
                        // le `template_contents` canonique.
                        for &c in self.nodes[root as usize].children.iter().rev() {
                            stack.push(Ser::Enter(c));
                        }
                        ids.push(u64::from(root));
                        tags.push("template".to_string());
                        stack.push(Ser::Lit(mode));
                    }
                }
            }
        }
    }

    /// Capture journal ADR-0025 — **avant** le filtre observer
    /// (le journal est inconditionnel : il alimente la
    /// resynchro moteur, pas les listeners de la page).
    /// Seules les mutations sur nœuds **attachés** sont
    /// retenues : un sous-arbre détaché n'a pas de reflet
    /// moteur — son attachement le sérialisera entier.
    fn journal_record(&mut self, rec: &MutationRecord) {
        if !self.is_attached(rec.target) {
            return;
        }
        match rec.ty {
            "childList" => {
                for &key in &rec.removed {
                    self.journal.push(JournalOp::Remove { key });
                }
                if let Some(&first) = rec.added.first() {
                    // Position au moment de la mutation — les
                    // ops rejouent en séquence, l'index reste
                    // valide côté moteur. Comptée sur les enfants
                    // **sérialisables** (Element|Text) : le moteur
                    // n'a pas de nœuds commentaires, indexer sur
                    // `children` brut décalerait l'insertion d'un
                    // cran par `<!-- -->` antérieur (graft_subtree
                    // filtre par `serializable_kind`).
                    let index = self.nodes[rec.target as usize]
                        .children
                        .iter()
                        .take_while(|&&c| c != first)
                        .filter(|&&c| {
                            matches!(
                                self.nodes[c as usize].kind,
                                NodeKind::Element | NodeKind::Text
                            )
                        })
                        .count() as u32;
                    self.journal.push(JournalOp::Insert {
                        parent: rec.target,
                        index,
                        added: rec.added.clone(),
                    });
                }
            }
            "attributes" => {
                if let Some(name) = &rec.attr_name {
                    self.journal.push(JournalOp::SetAttr {
                        key: rec.target,
                        name: name.clone(),
                    });
                }
            }
            "characterData" => {
                self.journal.push(JournalOp::SetText { key: rec.target });
            }
            _ => {}
        }
    }

    /// Dispatch d'un record aux observers correspondants :
    /// registrations propres de `target` + registrations
    /// `subtree` des ancêtres — un observer enregistré aux
    /// deux niveaux n'est notifié qu'**une** fois. La valeur
    /// d'avant est filtrée par observer (`*OldValue`).
    fn queue_mutation(&mut self, rec: &MutationRecord) {
        self.journal_record(rec);
        if self.observers.is_empty() {
            return;
        }
        let mut hits: Vec<&MutObsReg> = self
            .observers
            .get(&rec.target)
            .map(|regs| regs.iter().filter(|r| r.wants(rec)).collect())
            .unwrap_or_default();
        for a in self.ancestors(rec.target) {
            if let Some(regs) = self.observers.get(&a) {
                hits.extend(regs.iter().filter(|r| r.subtree && r.wants(rec)));
            }
        }
        for r in hits {
            let mut rec = rec.clone();
            if !r.keeps_old(&rec) {
                rec.old_value = None;
            }
            self.mutations.push((r.id, rec));
        }
    }

    /// Parse a full document fragment under the root.
    #[must_use]
    pub fn from_html(html: &str) -> Self {
        let mut d = Self::new();
        d.parse_fragment_into(d.root, html);
        // Les scripts du parse initial sont « already started »
        // — la session les collecte et évalue via son pipeline
        // (`ScriptItem`), jamais via la file dynamique ; sinon
        // chaque `<script>` statique serait évalué en double.
        d.mark_scripts_started(d.root);
        d.pending_dyn.clear();
        // CAP-184 : tampon ordinal figé des scripts du parse —
        // `eval_script_el` identifie `currentScript` par cette
        // séquence, insensible aux `remove()` (champ `script_seqs`).
        for (seq, n) in (0u32..).zip(d.by_tag(d.root, "script")) {
            d.script_seqs.insert(seq, n);
        }
        // T5 — `<template shadowrootmode>` déclaratif :
        // tout `template` portant l'attribut dont le parent
        // est un élément devient la racine shadow de ce
        // parent (spec DSD — `<template>` reste l'élément
        // hôte de l'arbre ; ses enfants = l'arbre shadow).
        // Borne : pas de `shadowrootserializable`/`delegatesfocus`
        // matérialisés (surface non mesurée).
        let tmpls = d.by_tag(d.root, "template");
        for t in tmpls {
            let Some(mode) = d.attr(t, "shadowrootmode").map(str::to_string) else {
                continue;
            };
            let open = mode != "closed";
            if let Some(p) = d.nodes[t as usize].parent {
                if d.nodes[p as usize].kind == NodeKind::Element {
                    // La racine est le fragment `content` du
                    // template (spec DSD — les enfants y
                    // vivent ; le `<template>` reste enfant
                    // light-DOM de l'hôte).
                    if let Some(frag) = d.template_content(t) {
                        d.attach_shadow_root(p, frag, open);
                    }
                }
            }
        }
        // CAP-185 : idem pour les `<link>` — la session livre
        // `load`/`error` de fin de feuille sur l'élément visé
        // par ordinal (tous `<link>`, cf. `link_seqs`).
        for (seq, n) in (0u32..).zip(d.by_tag(d.root, "link")) {
            d.link_seqs.insert(seq, n);
        }
        // Les `<iframe>` du parse initial sont chargés par
        // le pipeline sous-ressource — jamais par la file
        // dynamique (CAP-171, même exclusion que les
        // scripts ci-dessus).
        d.mark_frames_seen(d.root);
        d.pending_dyn_frames.clear();
        // Idem `<img>` (CAP-172) — les images du parse sont
        // collectées par la pile statique (`collect_images`),
        // jamais via la file dynamique.
        d.dyn_img_seen.clear();
        d.pending_dyn_imgs.clear();
        // Idem `<link>` (CAP-173) — les feuilles du parse
        // passent par `collect_stylesheet_links`.
        d.dyn_link_seen.clear();
        d.pending_dyn_links.clear();
        d
    }

    /// Marque « déjà chargés » tous les `<iframe>` du
    /// sous-arbre `node` (CAP-171) — contrairement aux
    /// scripts, la spec ne marque **pas** les cadres
    /// `innerHTML` inertes : cette fonction n'est appelée
    /// que pour le parse initial. Itératif (JS-C55).
    fn mark_frames_seen(&mut self, node: NodeId) {
        let mut stack = vec![node];
        while let Some(n) = stack.pop() {
            let el = &self.nodes[n as usize];
            if el.kind == NodeKind::Element && el.tag == "iframe" {
                self.dyn_frame_seen.insert(n);
            }
            stack.extend(el.children.iter().rev().copied());
        }
    }

    /// CAP-171 : `<iframe>` attaché → enfilé pour fetch par
    /// la session — même sans `src`/`srcdoc` (spec « process
    /// the iframe attributes » → `about:blank` : le realm
    /// existe dès l'insertion, `contentWindow` non-null et
    /// `load` part). Appelé par les mutateurs et le setter
    /// `src`/`srcdoc` sur cadre attaché (re-navigation, spec).
    fn note_attached_frame(&mut self, node: NodeId) {
        if !self.is_attached(node) || self.dyn_frame_seen.contains(&node) {
            return;
        }
        let el = &self.nodes[node as usize];
        if el.kind != NodeKind::Element || el.tag != "iframe" {
            return;
        }
        // B-29 (linuxmint) : AdSense `sodar` crée des iframes
        // de télémétrie en continu (~90/min → 419 nœuds, heap
        // +40 Mo/min). Au-delà du cap, l'élément monte mais
        // n'est jamais enfilé : pas de realm, pas de fetch,
        // `contentWindow`/`contentDocument` restent `null`
        // — mort douce pour la télémétrie, page intacte.
        if self.dyn_frame_seen.len() >= MAX_DYN_FRAMES {
            return;
        }
        self.dyn_frame_seen.insert(node);
        self.pending_dyn_frames.push(node);
    }

    /// `src`/`srcdoc` modifié sur un `<iframe>` attaché →
    /// re-navigation (spec : tout changement de `src`
    /// recharge). Une éventuelle entrée non drainée est
    /// retirée — le dernier `src` subsume l'ancien (le fetch
    /// en vol, lui, aboutira `load`/`error` normalement —
    /// borne : pas d'annulation de tâche en cours).
    /// Appelé par `set_attribute`.
    pub(crate) fn note_frame_src_changed(&mut self, node: NodeId) {
        self.pending_dyn_frames.retain(|&n| n != node);
        self.dyn_frame_seen.remove(&node);
        self.note_attached_frame(node);
    }

    /// CAP-172 — « update the image data » (spec) : appelé
    /// sur insertion attachée et sur tout changement de
    /// `src`/`srcset`/`sizes` d'un `<img>` — attaché **ou
    /// détaché** (`new Image().src=` = prefetch, spec). La
    /// clé source (`srcset|sizes|src`) dedup : ré-insérer
    /// un `<img>` à source inchangée ne re-fetch pas ;
    /// changer `src` enfile une nouvelle requête (l'ancienne
    /// en vol aboutit `load`/`error` normalement — borne :
    /// pas d'annulation de tâche).
    fn note_img(&mut self, node: NodeId) {
        let el = &self.nodes[node as usize];
        if el.kind != NodeKind::Element || el.tag != "img" {
            return;
        }
        let get = |k: &str| {
            el.attrs
                .iter()
                .find(|(name, _)| name == k)
                .map_or("", |(_, v)| v.as_str())
        };
        // B-20 (marmiton) : `data-src`-family = repli lazy-load
        // — un `<img data-src>` sans `src` n'entrait jamais
        // dans la clé → jamais enfilé (le repli session ne
        // voyait aucune requête). L'attribut repli alimente
        // le slot `src`/`srcset` de la clé.
        let src_eff = {
            let s = get("src");
            if s.is_empty() {
                ["data-src", "data-lazy-src", "data-original", "data-url"]
                    .iter()
                    .find_map(|a| {
                        let v = get(a);
                        if v.is_empty() {
                            None
                        } else {
                            Some(v)
                        }
                    })
                    .unwrap_or("")
            } else {
                s
            }
        };
        let srcset_eff = {
            let s = get("srcset");
            if s.is_empty() {
                get("data-srcset")
            } else {
                s
            }
        };
        let key = format!("{}\u{0}{}\u{0}{}", srcset_eff, get("sizes"), src_eff);
        if key == "\u{0}\u{0}" {
            return;
        }
        if self.dyn_img_seen.get(&node) == Some(&key) {
            return;
        }
        self.dyn_img_seen.insert(node, key);
        // Dernière source gagne (spec « update the image
        // data » coalesce) : une entrée non drainée pour ce
        // nœud est subsumée — `src=a; src=b` avant pompe ne
        // fetch que `b`.
        self.pending_dyn_imgs.retain(|&n| n != node);
        self.pending_dyn_imgs.push(node);
    }

    /// CAP-173 — « link processing » (spec) : un `<link>`
    /// dont `rel` contient `stylesheet` et porte un `href`
    /// charge dès qu'il est **connecté** — insertion ou
    /// changement `rel`/`href` (les liens détachés ne
    /// chargent pas, contrairement aux images). Dedup par
    /// clé `rel|href` : dernier non drainé gagne (même
    /// coalescence que les images).
    fn note_link(&mut self, node: NodeId) {
        if !self.is_attached(node) {
            return;
        }
        let el = &self.nodes[node as usize];
        if el.kind != NodeKind::Element || el.tag != "link" {
            return;
        }
        let get = |k: &str| {
            el.attrs
                .iter()
                .find(|(name, _)| name == k)
                .map_or("", |(_, v)| v.as_str())
        };
        let rel = get("rel");
        let href = get("href");
        let is_sheet = rel
            .split_whitespace()
            .any(|w| w.eq_ignore_ascii_case("stylesheet"));
        if !is_sheet || href.is_empty() {
            return;
        }
        let key = format!("{rel}\u{0}{href}");
        if self.dyn_link_seen.get(&node) == Some(&key) {
            return;
        }
        self.dyn_link_seen.insert(node, key);
        self.pending_dyn_links.retain(|&n| n != node);
        self.pending_dyn_links.push(node);
    }

    /// Marque « already started » tous les `<script>` du
    /// sous-arbre `node` (spec : le parseur de fragments pose
    /// le flag — `innerHTML` n'exécute jamais ses scripts,
    /// contrairement aux insertions directes). Itératif
    /// (JS-C55).
    pub fn mark_scripts_started(&mut self, node: NodeId) {
        let mut stack = vec![node];
        while let Some(n) = stack.pop() {
            let el = &self.nodes[n as usize];
            if el.kind == NodeKind::Element && el.tag == "script" {
                self.dyn_started.insert(n);
            }
            stack.extend(el.children.iter().rev().copied());
        }
    }

    /// CAP-169/171 : les `<script>`/`<iframe>` attachés par
    /// `node` (sous-arbre compris, ordre document) et pas
    /// encore « started »/« seen » sont enfilés pour
    /// fetch/éval — insertion dynamique (`appendChild` & co
    /// = `force-async`, spec). Appelé par chaque mutateur
    /// **après** propagation du flag `attached` — un élément
    /// dans un sous-arbre détaché ne charge rien.
    fn note_attached_scripts(&mut self, node: NodeId) {
        if !self.is_attached(node) {
            return;
        }
        let mut stack = vec![node];
        while let Some(n) = stack.pop() {
            if self.nodes[n as usize].kind != NodeKind::Element {
                stack.extend(self.nodes[n as usize].children.iter().rev().copied());
                continue;
            }
            if self.nodes[n as usize].tag == "script" && !self.dyn_started.contains(&n) {
                self.dyn_started.insert(n);
                self.pending_dyn.push(n);
            }
            // `innerHTML` n'inerte pas les cadres (spec) —
            // `note_attached_frame` filtre l'absence de
            // `src`/`srcdoc`.
            if self.nodes[n as usize].tag == "iframe" {
                self.note_attached_frame(n);
            }
            // `<img>` inséré → « update the image data »
            // (CAP-172, spec) — `note_img` dedup par clé
            // source : une image dont `src=` a déjà enfilé
            // détachée n'est pas re-fetchée à l'insertion.
            if self.nodes[n as usize].tag == "img" {
                self.note_img(n);
            }
            // `<link rel=stylesheet>` inséré → fetch de la
            // feuille (CAP-173, spec « link processing »).
            if self.nodes[n as usize].tag == "link" {
                self.note_link(n);
            }
            stack.extend(self.nodes[n as usize].children.iter().rev().copied());
        }
    }

    fn alloc(&mut self, kind: NodeKind, tag: &str) -> NodeId {
        self.nodes.push(Node {
            kind,
            tag: tag.to_lowercase(),
            attrs: Vec::new(),
            children: Vec::new(),
            parent: None,
            text: String::new(),
            listeners: Vec::new(),
            inline_handlers: Vec::new(),
            attached: false,
        });
        (self.nodes.len() - 1) as NodeId
    }

    /// Detached element (`document.createElement`).
    pub fn create_element(&mut self, tag: &str) -> NodeId {
        self.alloc(NodeKind::Element, tag)
    }

    /// Detached comment node (`document.createComment`) —
    /// `nodeType` 8, marqueurs de template Lit (vaadin terrain).
    pub fn create_comment(&mut self, text: &str) -> NodeId {
        let id = self.alloc(NodeKind::Comment, "#comment");
        self.nodes[id as usize].text = text.to_string();
        id
    }

    /// Detached text node (`document.createTextNode`).
    pub fn create_text(&mut self, text: &str) -> NodeId {
        let id = self.alloc(NodeKind::Text, "#text");
        self.nodes[id as usize].text = text.to_string();
        id
    }

    /// Detached `DocumentFragment` (`document.createDocumentFragment`).
    pub fn create_fragment(&mut self) -> NodeId {
        self.alloc(NodeKind::Fragment, "#document-fragment")
    }

    /// Attache une racine shadow à `host` (stub issue #25) —
    /// `open` = `mode: "open"` de l'init. L'hôte est marqué
    /// sale : la synchro de rendu doit re-sérialiser — le
    /// sérialiseur émet la racine en `<template
    /// shadowrootmode>` (R-96 — DSD transport).
    pub fn attach_shadow_root(&mut self, host: NodeId, root: NodeId, open: bool) {
        self.shadow_roots.insert(host, (root, open));
        self.shadow_hosts.insert(root, host);
        self.mark_dirty_at(host);
        // Étape 137b : op de couture — l'attache voyage par
        // journal comme les autres mutations (re-parse seulement
        // si la garde rejette, jamais par défaut).
        if self.is_attached(host) {
            self.journal
                .push(JournalOp::AttachShadow { host, root, open });
        }
    }

    /// Hôte de la racine shadow `root` (CAP-191) — `None`
    /// si `root` n'est pas une racine attachée.
    #[must_use]
    pub fn shadow_host(&self, root: NodeId) -> Option<NodeId> {
        self.shadow_hosts.get(&root).copied()
    }

    /// `true` si `node` est une racine shadow attachée
    /// (CAP-191) — `host`/`getRootNode({composed})`.
    #[must_use]
    pub fn is_shadow_root(&self, node: NodeId) -> bool {
        self.shadow_hosts.contains_key(&node)
    }

    /// Les `<slot>` de l'arbre shadow de `root`, DFS —
    /// l'analyse flat n'a pas de slots imbriqués à aplatir
    /// (borne tranche 3).
    #[must_use]
    pub fn slots_in(&self, root: NodeId) -> Vec<NodeId> {
        let mut out = Vec::new();
        let mut stack = vec![root];
        while let Some(n) = stack.pop() {
            let Some(node) = self.nodes.get(n as usize) else {
                continue;
            };
            if node.kind == NodeKind::Element && node.tag == "slot" {
                out.push(n);
            }
            for &c in node.children.iter().rev() {
                stack.push(c);
            }
        }
        out
    }

    /// Nœuds distribués au `<slot>` `slot` — répartition
    /// flat (tranche 3, miroir) : les enfants light-DOM de
    /// l'hôte dont l'attribut `slot` égale `slot.name`
    /// (défaut `""` → enfants sans `slot` ou `slot=""` ;
    /// les textes vont toujours au slot par défaut).
    /// `flatten` non porté — les `<slot>` imbriqués ne se
    /// re-distribuent pas.
    #[must_use]
    pub fn slot_assigned(&self, slot: NodeId, elements_only: bool) -> Vec<NodeId> {
        // Hôte = parent de la racine shadow contenant `slot`.
        let mut cur = slot;
        let host = loop {
            match self.nodes.get(cur as usize).and_then(|n| n.parent) {
                Some(p) if self.is_shadow_root(p) => match self.shadow_host(p) {
                    Some(h) => break h,
                    None => return Vec::new(),
                },
                Some(p) => cur = p,
                None => return Vec::new(),
            }
        };
        let name = self.attr(slot, "name").unwrap_or_default().to_string();
        self.child_nodes(host)
            .into_iter()
            .filter(|&c| {
                let n = &self.nodes[c as usize];
                match n.kind {
                    // Une racine shadow n'est pas distribuable.
                    NodeKind::Element => {
                        !self.is_shadow_root(c) && self.attr(c, "slot").unwrap_or_default() == name
                    }
                    NodeKind::Text => !elements_only && name.is_empty(),
                    NodeKind::Comment | NodeKind::Fragment => false,
                }
            })
            .collect()
    }

    /// `el.assignedSlot` — le `<slot>` de la racine shadow du
    /// parent de `el` dont `name` égale `el.slot` (défaut
    /// `""`) ; `None` hors distribution (pas de racine ou
    /// pas de slot correspondant — borne flat).
    #[must_use]
    pub fn assigned_slot(&self, el: NodeId) -> Option<NodeId> {
        let parent = self.nodes.get(el as usize)?.parent?;
        let (root, _) = self.shadow_roots.get(&parent).copied()?;
        let want = self.attr(el, "slot").unwrap_or_default();
        self.slots_in(root)
            .into_iter()
            .find(|&s| self.attr(s, "name").unwrap_or_default() == want)
    }

    /// Racine shadow de `host` — `include_closed` pour
    /// `ElementInternals.shadowRoot` (spec : lisible même
    /// en mode `closed`, contrairement à `el.shadowRoot`).
    pub fn shadow_root(&self, host: NodeId, include_closed: bool) -> Option<NodeId> {
        self.shadow_roots
            .get(&host)
            .and_then(|&(r, open)| (open || include_closed).then_some(r))
    }

    /// `CharacterData.data`/`nodeValue` sur un nœud texte —
    /// émet un record `characterData` (JS-C26).
    pub fn set_text(&mut self, node: NodeId, text: &str) {
        if node as usize >= self.nodes.len() {
            return;
        }
        let old = std::mem::replace(&mut self.nodes[node as usize].text, text.to_string());
        // Le nœud texte se reboîte via son élément parent.
        self.mark_dirty_at(self.parent(node).unwrap_or(node));
        self.queue_mutation(&MutationRecord {
            ty: "characterData",
            target: node,
            added: Vec::new(),
            removed: Vec::new(),
            prev_sibling: None,
            next_sibling: None,
            attr_name: None,
            old_value: Some(old),
        });
    }

    /// Dirty = mutation depuis la dernière synchro de rendu.
    /// Cible inconnue → tout le document (conservateur).
    pub fn mark_dirty(&self) {
        self.mark_dirty_at(self.root);
    }

    /// Marque le document sale **avec** la cible `node`
    /// (étape 55f) — escalade à l'ancêtre commun si un autre
    /// sous-arbre était déjà sale. Un nœud **détaché** du
    /// document ne fixe pas la cible : sa mutation devient
    /// visible à l'attachement, qui marque le parent
    /// (évite qu'un fragment hors-arbre n'escalade la cible
    /// à la racine).
    pub fn mark_dirty_at(&self, node: NodeId) {
        self.dirty.set(true);
        if !self.is_attached(node) {
            return;
        }
        let next = match self.dirty_node.get() {
            Some(d) if d == node => d,
            Some(d) => self.common_ancestor(d, node),
            None => node,
        };
        self.dirty_node.set(Some(next));
    }

    /// Plus proche ancêtre commun de `a` et `b` (racine
    /// `#document` en dernier recours). Linéaire en
    /// `profondeur(a)+profondeur(b)` — la formulation
    /// « monter `a` jusqu'à être ancêtre de `b` » payait une
    /// remontée complète de `b` par niveau (O(prof²) —
    /// JS-C55).
    fn common_ancestor(&self, a: NodeId, b: NodeId) -> NodeId {
        let mut seen = std::collections::HashSet::new();
        let mut cur = Some(a);
        while let Some(n) = cur {
            if !seen.insert(n) {
                break;
            }
            cur = self.parent(n);
        }
        let mut cur = Some(b);
        while let Some(n) = cur {
            if seen.contains(&n) {
                return n;
            }
            cur = self.parent(n);
        }
        self.root
    }

    /// Propage le drapeau « rattaché au document » au sous-arbre
    /// de `node` — DFS itératif (JS-C55 : profondeur contrôlée
    /// par le script). Les sous-arbres déjà uniformes sont
    /// sautés : un append de nœud frais coûte O(1).
    fn set_attached(&mut self, node: NodeId, flag: bool) {
        if self
            .nodes
            .get(node as usize)
            .is_none_or(|n| n.attached == flag)
        {
            return;
        }
        let mut stack = vec![node];
        while let Some(c) = stack.pop() {
            if self.nodes[c as usize].attached == flag {
                continue;
            }
            self.nodes[c as usize].attached = flag;
            // Réaction CE (JS-C68) : attachement = upgrade à
            // l'insertion ou `connectedCallback` ; détachement
            // = `disconnectedCallback` (customisés seulement).
            if flag {
                if self.ce.upgraded.contains_key(&c)
                    || self.ce.defs.contains_key(&self.nodes[c as usize].tag)
                {
                    self.ce_reactions.push(CeReaction::Connected(c));
                }
            } else if self.ce.upgraded.contains_key(&c) {
                self.ce_reactions.push(CeReaction::Disconnected(c));
            }
            // `.rev()` : la pile LIFO restitue l'ordre document
            // (les `CeReaction` se drainent en ordre d'arbre).
            stack.extend(self.nodes[c as usize].children.iter().rev().copied());
        }
    }

    /// Lit et efface le flag de mutation (≙ « dirty bit » des toolkits).
    pub fn take_dirty(&self) -> bool {
        self.dirty.replace(false)
    }

    /// Chemin `(tag, index parmi les frères éléments de même
    /// tag)` du sous-arbre sale depuis la racine — même format
    /// que `element_path` côté session (étape 55f : il survit
    /// au re-parse contrairement aux `NodeId`). `Some(vec![])`
    /// = cible non résolvable (nœud détaché) → document entier,
    /// conservateur. `None` = rien de sale.
    pub fn take_dirty_path(&self) -> Option<Vec<(String, u32)>> {
        let node = self.dirty_node.take()?;
        Some(self.element_path(node).unwrap_or_default())
    }

    /// `element_path` du nœud `n` (remonté à l'élément le plus
    /// proche) — `None` si le nœud est détaché du document.
    /// `pub(crate)` : sert aussi de clé de lookup dans la table
    /// de géométrie publiée par la session (issue #19 —
    /// `getBoundingClientRect`/`offset*` de `dom_host`).
    pub(crate) fn element_path(&self, mut n: NodeId) -> Option<Vec<(String, u32)>> {
        if !self.node_valid(n) {
            return None;
        }
        while self.nodes[n as usize].kind != NodeKind::Element
            || self.nodes[n as usize].tag == "#document"
        {
            n = self.parent(n)?;
        }
        let mut path = Vec::new();
        let mut cur = n;
        while let Some(p) = self.parent(cur) {
            let tag = self.nodes[cur as usize].tag.clone();
            let k = self.nodes[p as usize]
                .children
                .iter()
                .take_while(|&&c| c != cur)
                .filter(|&&c| self.nodes[c as usize].tag == tag)
                .count();
            path.push((tag, k as u32));
            cur = p;
        }
        path.reverse();
        Some(path)
    }

    /// Nœud au `element_path` donné — marche inverse de
    /// [`Self::element_path`] : pour chaque `(tag, k)`, le
    /// k-ième enfant élément portant ce tag. `None` si le
    /// chemin sort du miroir (divergence session/miroir —
    /// dégrade, ne plante pas ; CAP-180, hit-test inverse).
    pub(crate) fn node_at_path(&self, path: &[(String, u32)]) -> Option<NodeId> {
        let mut cur = self.root;
        for (tag, k) in path {
            let mut seen = 0u32;
            let mut next = None;
            // B-37 (caniuse vol58) : le chemin peut porter un
            // NodeId obsolète après un re-parse ADR-0025 —
            // `nodes[c]` hors borne = panic ADR-0008. `.get`
            // → `None` typé, jamais d'accès invalide.
            for &c in &self.nodes.get(cur as usize)?.children {
                if self.nodes.get(c as usize).map(|n| n.tag.as_str()) == Some(tag.as_str()) {
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

    // -- `<canvas>` 2D (JS-C28, couture R-11) ------------------------

    /// État 2D du `<canvas>` `node`, créé au premier
    /// `getContext("2d")`.
    pub fn canvas_state(&mut self, node: NodeId) -> &mut CanvasState {
        self.canvases.entry(node).or_default()
    }

    /// Enregistre `op` sur le `<canvas>` `node` et marque le
    /// nœud sale (le dessin est une mutation de **rendu** : la
    /// session re-peint le sous-arbre — le HTML sérialisé ne
    /// change pas). Ignorée si `node` n'est pas un élément
    /// connu, si un scalaire de l'op n'est pas fini (spec :
    /// arguments NaN/infini ignorés) ou si la borne
    /// `canvas_op_limit` est atteinte.
    pub fn push_canvas_op(&mut self, node: NodeId, op: CanvasOpRecord) {
        let valid = self
            .nodes
            .get(node as usize)
            .is_some_and(|n| n.kind == NodeKind::Element);
        let room = self
            .canvases
            .get(&node)
            .is_none_or(|s| s.ops.len() < self.canvas_op_limit);
        if !valid || !room || !op.args_finite() {
            return;
        }
        self.canvases.entry(node).or_default().ops.push(op);
        self.mark_dirty_at(node);
    }

    /// Objet contexte JS déjà créé pour `node` — `getContext`
    /// est idempotent par mode.
    pub fn canvas_ctx(&self, node: NodeId) -> Option<Value> {
        self.canvases.get(&node).and_then(|s| s.ctx_value)
    }

    /// Enregistre l'objet contexte de `node` — semé comme
    /// racine GC via `canvas_ctx_values`.
    pub fn set_canvas_ctx(&mut self, node: NodeId, v: Value) {
        self.canvas_state(node).ctx_value = Some(v);
    }

    /// Objets JS détenus par la side-table (contextes 2D) —
    /// semés comme racines GC par `Engine` : un `ctx` survit
    /// tant que le miroir vit, même si le script a lâché sa
    /// référence (le canvas reste ré-utilisable).
    pub fn canvas_ctx_values(&self) -> Vec<Value> {
        self.canvases.values().filter_map(|s| s.ctx_value).collect()
    }

    /// `canvas.width=`/`height=` efface le bitmap et remet
    /// l'état du contexte aux défauts (spec) — l'objet
    /// contexte survit, la op-list repart à zéro, le nœud
    /// est marqué sale.
    pub fn clear_canvas(&mut self, node: NodeId) {
        if let Some(s) = self.canvases.get_mut(&node) {
            *s = CanvasState {
                ctx_value: s.ctx_value.take(),
                ..CanvasState::default()
            };
        }
        self.mark_dirty_at(node);
    }

    /// Snapshot « chemin élément → op-list **cumulée** » de
    /// chaque `<canvas>` ayant dessiné — le playback
    /// navigateur (R-11) rejoue la liste entière à chaque
    /// re-peinture : les états ne sont **pas** drainés
    /// (bitmap persistante, pas un diff). Un `<canvas>`
    /// détaché du document n'exporte rien — son chemin ne
    /// résout plus.
    pub fn canvas_ops(&self) -> CanvasOps {
        self.canvases
            .iter()
            .filter(|(_, s)| !s.ops.is_empty())
            .filter_map(|(&n, s)| self.element_path(n).map(|p| (p, s.ops.clone())))
            .collect()
    }

    /// Règle la borne d'ops par `<canvas>` (règle « budgets »
    /// — boucles de dessin hostiles).
    pub fn set_canvas_op_limit(&mut self, limit: usize) {
        self.canvas_op_limit = limit;
    }

    /// Sérialisation du document entier (source de re-rendu, JS-5b).
    #[must_use]
    pub fn serialize_document(&self) -> String {
        let t0 = std::time::Instant::now();
        let s = self.inner_html(self.root);
        // JS-C37/C39 : coût de la resynchro DOM → host instrumenté
        // (appels + octets + micros cumulés, lus par
        // `serialize_stats`).
        self.serialize_calls.set(self.serialize_calls.get() + 1);
        self.serialize_bytes
            .set(self.serialize_bytes.get() + s.len() as u64);
        self.serialize_micros
            .set(self.serialize_micros.get() + t0.elapsed().as_micros() as u64);
        s
    }

    /// Compteurs cumulés de `serialize_document` —
    /// instrumentation du coût de resynchronisation DOM
    /// (JS-C37 appels/octets, JS-C39 micros — l'instrument du
    /// critère « la synchro domine-t-elle le frame budget ? »).
    #[must_use]
    pub fn serialize_stats(&self) -> SerializeStats {
        SerializeStats {
            calls: self.serialize_calls.get(),
            bytes: self.serialize_bytes.get(),
            micros: self.serialize_micros.get(),
        }
    }

    /// Attach `child` under `parent` (re-parents if attached).
    /// `anc` est `node` ou un ancêtre de `node` (borne
    /// `MAX_PROTO_DEPTH` — chaîne hostile non cyclique).
    #[must_use]
    pub fn is_ancestor_or_self(&self, node: NodeId, anc: NodeId) -> bool {
        let mut cur = Some(node);
        let mut d = 0;
        while let Some(n) = cur {
            if n == anc {
                return true;
            }
            cur = self.nodes.get(n as usize).and_then(|nd| nd.parent);
            d += 1;
            if d > 512 {
                return false;
            }
        }
        false
    }

    pub fn append(&mut self, parent: NodeId, child: NodeId) {
        // Id obsolète (arène reconstruite — spectrum
        // `groupChildren`, `nodes[607]` sur 507) → no-op,
        // jamais panic.
        if !self.node_valid(parent) || !self.node_valid(child) {
            return;
        }
        if self.nodes[child as usize].kind == NodeKind::Fragment {
            self.splice_fragment(parent, child, None);
            return;
        }
        // B-44 : plafond DOM — insertions refusées au-delà,
        // déplacements d'attachés restants légaux.
        if !self.nodes[child as usize].attached && self.nodes.len() >= MAX_DOM_NODES {
            return;
        }
        self.detach(child);
        self.nodes[child as usize].parent = Some(parent);
        let a = self.nodes[parent as usize].attached;
        self.set_attached(child, a);
        self.nodes[parent as usize].children.push(child);
        self.mark_dirty_at(parent);
        let prev = {
            let kids = &self.nodes[parent as usize].children;
            kids.get(kids.len().saturating_sub(2)).copied()
        };
        // Le retrait sur l'ancien parent a été émis par `detach`.
        self.queue_mutation(&MutationRecord::child_list(
            parent,
            vec![child],
            Vec::new(),
            prev,
            None,
        ));
        self.note_attached_scripts(child);
    }

    /// Detach `node` from its parent (keeps the node alive).
    pub fn detach(&mut self, node: NodeId) {
        if !self.node_valid(node) {
            return;
        }
        if let Some(p) = self.nodes[node as usize].parent {
            let (prev, next) = {
                let kids = &self.nodes[p as usize].children;
                kids.iter()
                    .position(|&c| c == node)
                    .map_or((None, None), |i| {
                        (
                            i.checked_sub(1).and_then(|j| kids.get(j).copied()),
                            kids.get(i + 1).copied(),
                        )
                    })
            };
            self.nodes[p as usize].children.retain(|&c| c != node);
            self.mark_dirty_at(p);
            self.nodes[node as usize].parent = None;
            self.set_attached(node, false);
            self.queue_mutation(&MutationRecord::child_list(
                p,
                Vec::new(),
                vec![node],
                prev,
                next,
            ));
        }
    }

    /// Replace all children of `node` with `kids` (innerHTML setter).
    /// Spec : un seul record `childList` `removed+added` par
    /// « replace all » — les `append` internes n'en émettent pas
    /// (on mute directement, sans passer par [`Dom::append`]).
    pub fn replace_children(&mut self, node: NodeId, kids: &[NodeId]) {
        if !self.node_valid(node) {
            return;
        }
        let old = std::mem::take(&mut self.nodes[node as usize].children);
        for c in &old {
            self.nodes[*c as usize].parent = None;
            self.set_attached(*c, false);
        }
        let a = self.nodes[node as usize].attached;
        for &k in kids {
            self.detach(k);
            self.nodes[k as usize].parent = Some(node);
            self.set_attached(k, a);
        }
        self.nodes[node as usize].children = kids.to_vec();
        self.mark_dirty_at(node);
        self.queue_mutation(&MutationRecord::child_list(
            node,
            kids.to_vec(),
            old,
            None,
            None,
        ));
        for &k in kids {
            self.note_attached_scripts(k);
        }
    }

    /// All children — elements *and* text (`childNodes`; `children`
    /// is elements-only, see [`Dom::element_children`]).
    #[must_use]
    pub fn child_nodes(&self, node: NodeId) -> Vec<NodeId> {
        if !self.node_valid(node) {
            return Vec::new();
        }
        self.nodes[node as usize].children.clone()
    }

    /// `parentNode` — `None` for the root or detached nodes.
    #[must_use]
    pub fn parent(&self, node: NodeId) -> Option<NodeId> {
        self.nodes.get(node as usize)?.parent
    }

    /// `<template>.content` — le `DocumentFragment` de contenu,
    /// créé à la première demande : les enfants de l'élément y
    /// sont *déplacés* (spec — ils cessent d'être des enfants
    /// de l'élément ; `template.childNodes` devient vide). Le
    /// fragment est détaché, comme chez un navigateur réel.
    /// `None` pour un non-`<template>`.
    pub fn template_content(&mut self, n: NodeId) -> Option<NodeId> {
        let node = self.nodes.get(n as usize)?;
        if node.kind != NodeKind::Element || node.tag != "template" {
            return None;
        }
        if let Some(&f) = self.template_contents.get(&n) {
            return Some(f);
        }
        let frag = self.create_fragment();
        let kids = std::mem::take(&mut self.nodes[n as usize].children);
        for &k in &kids {
            self.set_attached(k, false);
            self.nodes[k as usize].parent = Some(frag);
        }
        self.nodes[frag as usize].children = kids;
        self.template_contents.insert(n, frag);
        self.mark_dirty_at(n);
        // ADR-0025 : le déplacement vers le fragment détaché
        // n'émet pas de record — les mutations ultérieures sur
        // ces enfants seraient invisibles au journal (détachés)
        // sans changer l'empreinte structurelle (attributs,
        // texte) → repli re-parse, jamais de divergence silencieuse.
        if self.is_attached(n) {
            self.poison_journal();
        }
        Some(frag)
    }

    /// Cible d'écriture `innerHTML`/`replace all` : le fragment
    /// de contenu pour un `<template>` (le peupler revient à
    /// remplir `content`), le nœud lui-même sinon.
    pub fn content_target(&mut self, n: NodeId) -> NodeId {
        self.template_content(n).unwrap_or(n)
    }

    /// Enfants *sérialisables* de `n` : ceux du fragment de
    /// contenu pour un `<template>` matérialisé, sinon les
    /// enfants ordinaires (un template jamais touché sérialise
    /// ses enfants directs — le résultat est le même markup).
    fn serial_children(&self, n: NodeId) -> &[NodeId] {
        let Some(node) = self.nodes.get(n as usize) else {
            return &[];
        };
        if node.tag == "template" {
            if let Some(&f) = self.template_contents.get(&n) {
                return &self.nodes[f as usize].children;
            }
        }
        &node.children
    }

    /// Next (`true`) or previous (`false`) sibling, any kind —
    /// `None` when detached or at the edge.
    #[must_use]
    pub fn sibling(&self, node: NodeId, next: bool) -> Option<NodeId> {
        let p = self.nodes.get(node as usize)?.parent?;
        let kids = &self.nodes[p as usize].children;
        let i = kids.iter().position(|&c| c == node)?;
        let j = if next { i + 1 } else { i.checked_sub(1)? };
        kids.get(j).copied()
    }

    /// `insertBefore(child, ref)` — `None` ref ≙ append. If `ref`
    /// isn't a child of `parent` the insert degrades to append
    /// (tolerant — Dillo never throws on malformed trees either).
    pub fn insert_before(&mut self, parent: NodeId, child: NodeId, ref_node: Option<NodeId>) {
        if !self.node_valid(parent) || !self.node_valid(child) {
            return;
        }
        if self.nodes[child as usize].kind == NodeKind::Fragment {
            self.splice_fragment(parent, child, ref_node);
            return;
        }
        // Spec « insert » : un `ref_node` égal à `child` vise le
        // **prochain frère** de `child` — sinon le détachement
        // rend la référence introuvable et l'insert dégrade en
        // append (`prepend` du 1er enfant → queue, issue #17).
        let ref_node = if ref_node == Some(child) {
            self.sibling(child, true)
        } else {
            ref_node
        };
        // B-44 : même plafond DOM que `append`.
        if !self.nodes[child as usize].attached && self.nodes.len() >= MAX_DOM_NODES {
            return;
        }
        self.detach(child);
        let idx = ref_node.and_then(|r| {
            self.nodes[parent as usize]
                .children
                .iter()
                .position(|&c| c == r)
        });
        match idx {
            Some(i) => self.nodes[parent as usize].children.insert(i, child),
            None => self.nodes[parent as usize].children.push(child),
        }
        self.nodes[child as usize].parent = Some(parent);
        let a = self.nodes[parent as usize].attached;
        self.set_attached(child, a);
        self.mark_dirty_at(parent);
        let (prev, next) = {
            let kids = &self.nodes[parent as usize].children;
            kids.iter()
                .position(|&c| c == child)
                .map_or((None, None), |i| {
                    (
                        i.checked_sub(1).and_then(|j| kids.get(j).copied()),
                        kids.get(i + 1).copied(),
                    )
                })
        };
        self.queue_mutation(&MutationRecord::child_list(
            parent,
            vec![child],
            Vec::new(),
            prev,
            next,
        ));
        self.note_attached_scripts(child);
    }

    /// Insertion d'un `DocumentFragment` : ses enfants sont
    /// déplacés dans `parent` (devant `ref_node`, fin sinon) et
    /// le fragment reste vide — spec `insert` des fragments.
    /// Un seul record `childList` pour le groupe.
    fn splice_fragment(&mut self, parent: NodeId, frag: NodeId, ref_node: Option<NodeId>) {
        if !self.node_valid(parent) || !self.node_valid(frag) {
            return;
        }
        self.detach(frag);
        let kids = std::mem::take(&mut self.nodes[frag as usize].children);
        for &k in &kids {
            self.nodes[k as usize].parent = None;
        }
        if kids.is_empty() {
            return;
        }
        // Index d'insertion : devant `ref_node` s'il est enfant
        // de `parent`, fin sinon (dégradé tolérant, cf.
        // `insert_before`).
        let at = ref_node
            .and_then(|r| {
                self.nodes[parent as usize]
                    .children
                    .iter()
                    .position(|&c| c == r)
            })
            .unwrap_or_else(|| self.nodes[parent as usize].children.len());
        let a = self.nodes[parent as usize].attached;
        for &k in &kids {
            self.nodes[k as usize].parent = Some(parent);
            self.set_attached(k, a);
        }
        self.nodes[parent as usize]
            .children
            .splice(at..at, kids.iter().copied());
        self.mark_dirty_at(parent);
        let (prev, next) = {
            let kids_p = &self.nodes[parent as usize].children;
            (
                at.checked_sub(1).and_then(|i| kids_p.get(i).copied()),
                kids_p.get(at + kids.len()).copied(),
            )
        };
        self.queue_mutation(&MutationRecord::child_list(
            parent,
            kids.clone(),
            Vec::new(),
            prev,
            next,
        ));
        for &k in &kids {
            self.note_attached_scripts(k);
        }
    }

    /// `replaceChild(new, old)` — `old` is detached, `new` takes
    /// its slot. Returns `old` (spec) — caller wraps it.
    pub fn replace_child(&mut self, parent: NodeId, new: NodeId, old: NodeId) {
        if !self.node_valid(parent) || !self.node_valid(new) || !self.node_valid(old) {
            return;
        }
        let idx = self.nodes[parent as usize]
            .children
            .iter()
            .position(|&c| c == old);
        if let Some(i) = idx {
            self.detach(new);
            let (prev, next) = {
                let kids = &self.nodes[parent as usize].children;
                (
                    i.checked_sub(1).and_then(|j| kids.get(j).copied()),
                    kids.get(i + 1).copied(),
                )
            };
            self.nodes[parent as usize].children[i] = new;
            self.nodes[new as usize].parent = Some(parent);
            self.nodes[old as usize].parent = None;
            let a = self.nodes[parent as usize].attached;
            self.set_attached(new, a);
            self.set_attached(old, false);
            self.mark_dirty_at(parent);
            self.note_attached_scripts(new);
            self.queue_mutation(&MutationRecord::child_list(
                parent,
                vec![new],
                vec![old],
                prev,
                next,
            ));
        }
    }

    /// `cloneNode(deep)` — fresh ids, listeners are *not* cloned
    /// (spec: `addEventListener` handlers don't survive cloning).
    pub fn clone_node(&mut self, node: NodeId, deep: bool) -> NodeId {
        let (kind, tag, attrs, text, kids) = {
            let n = &self.nodes[node as usize];
            (
                n.kind,
                n.tag.clone(),
                n.attrs.clone(),
                n.text.clone(),
                n.children.clone(),
            )
        };
        let id = self.alloc(kind, &tag);
        self.nodes[id as usize].attrs = attrs;
        self.nodes[id as usize].text = text;
        if deep {
            // Itératif : la profondeur DOM est contrôlée par le
            // script — une descente récursive déborderait la pile
            // Rust (JS-C55).
            let mut work: Vec<(NodeId, NodeId)> = kids.iter().rev().map(|&c| (c, id)).collect();
            while let Some((src, dst_parent)) = work.pop() {
                let (k2, t2, a2, x2, kids2) = {
                    let n = &self.nodes[src as usize];
                    (
                        n.kind,
                        n.tag.clone(),
                        n.attrs.clone(),
                        n.text.clone(),
                        n.children.clone(),
                    )
                };
                let cid = self.alloc(k2, &t2);
                self.nodes[cid as usize].attrs = a2;
                self.nodes[cid as usize].text = x2;
                self.append(dst_parent, cid);
                work.extend(kids2.iter().rev().map(|&g| (g, cid)));
            }
        }
        id
    }

    /// `a.contains(b)` — `b` is `a` itself or a descendant.
    #[must_use]
    pub fn contains(&self, a: NodeId, b: NodeId) -> bool {
        a == b || self.ancestors(b).contains(&a)
    }

    /// `getRootNode()` — sommet de l'arbre de `node` : le
    /// `#document` si rattaché, sinon le plus haut ancêtre
    /// (racine d'un fragment/sous-arbre détaché ; un nœud
    /// orphelin est sa propre racine — spec). L'option
    /// `{composed:true}` n'ouvre rien ici : pas de shadow
    /// DOM dans le miroir (borné). Itératif + garde
    /// anti-cycle bornée par la taille d'arène.
    #[must_use]
    pub fn root_of(&self, node: NodeId) -> NodeId {
        let mut cur = node;
        let mut hops = 0usize;
        while let Some(p) = self.nodes.get(cur as usize).and_then(|n| n.parent) {
            cur = p;
            hops += 1;
            if hops > self.nodes.len() {
                return node; // corruption cyclique — repli sûr
            }
        }
        cur
    }

    /// `isEqualNode(other)` — égalité structurelle spec :
    /// kind, tag, attributs (ordre indifférent — ensemble),
    /// texte, et enfants pairwise. Itératif (JS-C55 : la
    /// profondeur est contrôlée par le script), borné par
    /// la taille de l'arène.
    #[must_use]
    pub fn is_equal_node(&self, a: NodeId, b: NodeId) -> bool {
        let mut work = vec![(a, b)];
        let mut left = self.nodes.len();
        while let Some((x, y)) = work.pop() {
            if left == 0 {
                return false;
            }
            left -= 1;
            let (nx, ny) = (&self.nodes[x as usize], &self.nodes[y as usize]);
            if nx.kind != ny.kind
                || nx.tag != ny.tag
                || nx.text != ny.text
                || nx.attrs.len() != ny.attrs.len()
                || !nx
                    .attrs
                    .iter()
                    .all(|(k, v)| ny.attrs.iter().any(|(k2, v2)| k == k2 && v == v2))
                || nx.children.len() != ny.children.len()
            {
                return false;
            }
            work.extend(nx.children.iter().copied().zip(ny.children.iter().copied()));
        }
        true
    }

    /// `compareDocumentPosition(other)` — bitmask
    /// `Node.DOCUMENT_POSITION_*` décrivant `other` relatif
    /// à `a` (spec DOM §3.4) : `a.compareDocumentPosition(b)`
    /// rend la position de **b** vue de a. Arbres disjoints
    /// → `DISCONNECTED|IMPLEMENTATION_SPECIFIC` + un ordre
    /// stable (index d'arène — ordre cohérent d'implémentation
    /// exigé par la spec, jamais aléatoire).
    #[must_use]
    pub fn compare_position(&self, a: NodeId, b: NodeId) -> u8 {
        const DISCONNECTED: u8 = 0x01;
        const PRECEDING: u8 = 0x02;
        const FOLLOWING: u8 = 0x04;
        const CONTAINS: u8 = 0x08;
        const CONTAINED_BY: u8 = 0x10;
        const IMPL_SPECIFIC: u8 = 0x20;
        if a == b {
            return 0;
        }
        if self.contains(a, b) {
            return CONTAINED_BY | FOLLOWING;
        }
        if self.contains(b, a) {
            return CONTAINS | PRECEDING;
        }
        // Rangs préordre depuis `#document` — DFS itératif.
        let mut rank = std::collections::HashMap::with_capacity(self.nodes.len());
        let mut stack = vec![self.root];
        while let Some(n) = stack.pop() {
            if rank.contains_key(&n) {
                continue;
            }
            rank.insert(n, rank.len());
            stack.extend(self.nodes[n as usize].children.iter().rev().copied());
        }
        match (rank.get(&a), rank.get(&b)) {
            (Some(&ra), Some(&rb)) => {
                if rb < ra {
                    PRECEDING
                } else {
                    FOLLOWING
                }
            }
            _ => DISCONNECTED | IMPL_SPECIFIC | if b < a { PRECEDING } else { FOLLOWING },
        }
    }

    /// `normalize()` — fusionne les `Text` adjacents et
    /// supprime les `Text` vides dans tout le sous-arbre
    /// de `node` (spec Node.normalize). Itératif ; chaque
    /// suppression passe par [`Dom::detach`] — les records
    /// `childList` et les drapeaux `attached` restent
    /// cohérents.
    pub fn normalize(&mut self, node: NodeId) {
        let mut work = vec![node];
        while let Some(n) = work.pop() {
            let kids = self.nodes[n as usize].children.clone();
            let mut i = 0;
            while i < kids.len() {
                let c = kids[i];
                if self.nodes[c as usize].kind != NodeKind::Text {
                    i += 1;
                    continue;
                }
                if self.nodes[c as usize].text.is_empty() {
                    self.detach(c);
                    i += 1;
                    continue;
                }
                // Absorbe les textes adjacents suivants.
                let mut j = i + 1;
                while j < kids.len() && self.nodes[kids[j] as usize].kind == NodeKind::Text {
                    let next = kids[j];
                    let extra = std::mem::take(&mut self.nodes[next as usize].text);
                    self.nodes[c as usize].text.push_str(&extra);
                    self.detach(next);
                    j += 1;
                }
                i = j;
            }
            work.extend(
                kids.iter()
                    .copied()
                    .filter(|&c| self.nodes[c as usize].kind != NodeKind::Text),
            );
        }
    }

    /// `el.matches(sel)` — single-node selector test (same subset
    /// as [`Dom::query`]: simple selectors + descendant combinator,
    /// selector lists `a, b`, pseudo-classes — JS-C99).
    #[must_use]
    pub fn matches(&self, node: NodeId, sel: &str) -> bool {
        let Some(list) = split_selector_list(sel) else {
            return false;
        };
        list.iter()
            .any(|item| self.matches_single(node, item, node))
    }

    /// Compound/complexe unique de `sel` sur `node` — `scope`
    /// = l'élément `:scope` (`el.matches` : le nœud lui-même).
    fn matches_single(&self, node: NodeId, sel: &str, scope: NodeId) -> bool {
        let Some(parts) = split_complex_parts(sel) else {
            return false;
        };
        let Some((last, prefix)) = parts.split_last() else {
            return false;
        };
        let Some(simple) = parse_simple_selector(last.text, 0) else {
            return false;
        };
        self.node_matches(node, &simple, scope)
            && (prefix.is_empty() || self.ancestor_matches(node, prefix, scope))
    }

    /// `el.closest(sel)` — nearest ancestor-or-self matching `sel`
    /// (`:scope` = le nœud de départ — spec).
    #[must_use]
    pub fn closest(&self, node: NodeId, sel: &str) -> Option<NodeId> {
        let list = split_selector_list(sel)?;
        let mut cur = Some(node);
        while let Some(n) = cur {
            if list.iter().any(|item| self.matches_single(n, item, node)) {
                return Some(n);
            }
            cur = self.parent(n);
        }
        None
    }

    /// `outerHTML` — serialize the node itself (inner + its own tag).
    #[must_use]
    pub fn outer_html(&self, node: NodeId) -> String {
        let mut out = String::new();
        self.serialize(node, &mut out);
        out
    }

    // -- attributes -----------------------------------------------------

    /// Le nœud peut-il recevoir le focus ? (CAP-162 — spec
    /// « focusable area », liste bornée) : contrôles non
    /// `disabled`, `a`/`area` avec `href`, `iframe`,
    /// `summary`, `[tabindex]` (toute valeur — `-1` reste
    /// focalisable *programmatiquement*), `[contenteditable]`.
    /// Un nœud détaché n'est pas focalisable.
    #[must_use]
    pub fn is_focusable(&self, node: NodeId) -> bool {
        let Some(n) = self.nodes.get(node as usize) else {
            return false;
        };
        if n.kind != NodeKind::Element || !n.attached {
            return false;
        }
        if self.attr(node, "disabled").is_some() {
            return false;
        }
        match n.tag.as_str() {
            "input" | "select" | "textarea" | "button" | "iframe" | "summary" => true,
            "a" | "area" => self.attr(node, "href").is_some(),
            _ => {
                self.attr(node, "tabindex").is_some()
                    || self.attr(node, "contenteditable").is_some()
            }
        }
    }

    /// Élément détenant le focus — `None` ou détaché → `body`
    /// (repli spec `activeElement` : « the body element or
    /// null » — `body` absent → `None`).
    #[must_use]
    pub fn active_element(&self) -> Option<NodeId> {
        if let Some(f) = self.focused.get() {
            if self.nodes.get(f as usize).is_some_and(|n| n.attached) {
                return Some(f);
            }
            // Focus perdu par détachement (spec « unfocus ») —
            // efface le pointeur mort.
            self.focused.set(None);
        }
        let b = self.body();
        (b != self.root && self.nodes[b as usize].attached).then_some(b)
    }

    /// `getAttribute` (first occurrence, case-insensitive name).
    #[must_use]
    pub fn attr(&self, node: NodeId, name: &str) -> Option<&str> {
        self.nodes
            .get(node as usize)?
            .attrs
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    /// `setAttribute`.
    pub fn set_attr(&mut self, node: NodeId, name: &str, value: &str) {
        if !self.node_valid(node) {
            return;
        }
        let old = self.attr(node, name).map(str::to_string);
        let attrs = &mut self.nodes[node as usize].attrs;
        if let Some((_, v)) = attrs.iter_mut().find(|(k, _)| k.eq_ignore_ascii_case(name)) {
            *v = value.to_string();
        } else {
            attrs.push((name.to_lowercase(), value.to_string()));
        }
        self.mark_dirty_at(node);
        self.queue_mutation(&MutationRecord {
            ty: "attributes",
            target: node,
            added: Vec::new(),
            removed: Vec::new(),
            prev_sibling: None,
            next_sibling: None,
            attr_name: Some(name.to_lowercase()),
            old_value: old.clone(),
        });
        self.ce_attr_reaction(node, name, old.clone(), Some(value.to_string()));
        // `iframe.src=`/`srcdoc=` sur cadre attaché →
        // (re-)navigation du sous-contexte (CAP-171, spec) —
        // seulement si la valeur change réellement.
        if old.as_deref() != Some(value)
            && self.nodes[node as usize].tag == "iframe"
            && (name.eq_ignore_ascii_case("src") || name.eq_ignore_ascii_case("srcdoc"))
        {
            self.note_frame_src_changed(node);
        }
        // `img.src`/`srcset`/`sizes` modifié → « update the
        // image data » (CAP-172, spec) — détaché inclus
        // (`new Image().src=` = prefetch) ; `note_img` dedup
        // par clé source.
        if old.as_deref() != Some(value)
            && self.nodes[node as usize].tag == "img"
            && matches!(
                name,
                "src" | "srcset" | "sizes" | "crossorigin" | "referrerpolicy"
            )
        {
            self.note_img(node);
        }
        // `link.rel`/`href`/… modifié → « link processing »
        // (CAP-173, spec) — `rel` qui gagne `stylesheet` ou
        // `href` changé re-fetch ; `note_link` filtre
        // l'attachement et dedup par clé `rel|href`.
        if old.as_deref() != Some(value)
            && self.nodes[node as usize].tag == "link"
            && matches!(
                name,
                "rel" | "href" | "integrity" | "crossorigin" | "referrerpolicy"
            )
        {
            self.note_link(node);
        }
    }

    /// `removeAttribute`. Returns whether it existed.
    pub fn remove_attr(&mut self, node: NodeId, name: &str) -> bool {
        if !self.node_valid(node) {
            return false;
        }
        let old = self.attr(node, name).map(str::to_string);
        let attrs = &mut self.nodes[node as usize].attrs;
        let before = attrs.len();
        attrs.retain(|(k, _)| !k.eq_ignore_ascii_case(name));
        let removed = attrs.len() != before;
        if removed {
            self.mark_dirty_at(node);
            self.queue_mutation(&MutationRecord {
                ty: "attributes",
                target: node,
                added: Vec::new(),
                removed: Vec::new(),
                prev_sibling: None,
                next_sibling: None,
                attr_name: Some(name.to_lowercase()),
                old_value: old.clone(),
            });
            self.ce_attr_reaction(node, name, old, None);
        }
        removed
    }

    /// Émet une [`CeReaction::AttrChanged`] si `node` est un
    /// élément customisé dont la définition observe `name`
    /// (JS-C68 — comparaison minuscule, noms figés à `define`).
    fn ce_attr_reaction(
        &mut self,
        node: NodeId,
        name: &str,
        old: Option<String>,
        new: Option<String>,
    ) {
        let Some(def_name) = self.ce.upgraded.get(&node) else {
            return;
        };
        let lname = name.to_lowercase();
        let watched = self
            .ce
            .defs
            .get(def_name)
            .is_some_and(|d| d.observed.iter().any(|a| a == &lname));
        if watched {
            self.ce_reactions.push(CeReaction::AttrChanged {
                node,
                name: lname,
                old,
                new,
            });
        }
    }

    /// `hasAttribute`.
    #[must_use]
    pub fn has_attr(&self, node: NodeId, name: &str) -> bool {
        self.attr(node, name).is_some()
    }

    // -- queries ----------------------------------------------------------

    /// `document.body` — first `<body>` descendant, else the root.
    #[must_use]
    pub fn body(&self) -> NodeId {
        self.by_tag(self.root, "body")
            .into_iter()
            .next()
            .unwrap_or(self.root)
    }

    /// `document.documentElement` — first element child of the root.
    #[must_use]
    pub fn document_element(&self) -> Option<NodeId> {
        self.element_children(self.root).into_iter().next()
    }

    /// Ancestor chain of `node`, parent first, `#document` root last
    /// (empty for the root itself or a stale id — §B-37 : un
    /// handle survivant à une arène reconstruite dégrade en
    /// chaîne vide, jamais panic).
    #[must_use]
    pub fn ancestors(&self, node: NodeId) -> Vec<NodeId> {
        let mut out = Vec::new();
        let mut cur = self.nodes.get(node as usize).and_then(|n| n.parent);
        while let Some(p) = cur {
            out.push(p);
            cur = self.nodes.get(p as usize).and_then(|n| n.parent);
        }
        out
    }

    /// Element children of `node` (text nodes filtered out) —
    /// `[]` pour un id obsolète (§B-37).
    #[must_use]
    pub fn element_children(&self, node: NodeId) -> Vec<NodeId> {
        self.nodes
            .get(node as usize)
            .map(|n| {
                n.children
                    .iter()
                    .copied()
                    .filter(|&c| {
                        self.nodes
                            .get(c as usize)
                            .is_some_and(|cn| cn.kind == NodeKind::Element)
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// `getElementById` — first match, document order. Seuls les
    /// nœuds **rattachés** au document comptent (JS-C35) : un
    /// élément `remove()`d ou effacé par `document.open()` n'est
    /// plus trouvable (spec — la recherche couvre l'arbre du
    /// document, pas l'arène).
    #[must_use]
    pub fn by_id(&self, id: &str) -> Option<NodeId> {
        self.nodes
            .iter()
            .enumerate()
            .find(|(i, n)| {
                n.kind == NodeKind::Element
                    && Self::attr_val(n, "id") == Some(id)
                    && self.is_attached(*i as NodeId)
            })
            .map(|(i, _)| i as NodeId)
    }

    /// Le nœud remonte à `root` — les nœuds détachés
    /// (`remove`, effacement `document.open`) sont invisibles
    /// aux requêtes au niveau document. Lecture du drapeau
    /// `attached` — O(1) ; la remontée `parent` était
    /// O(profondeur) par requête (JS-C55).
    pub(crate) fn is_attached(&self, node: NodeId) -> bool {
        self.nodes.get(node as usize).is_some_and(|n| n.attached)
    }

    fn attr_val<'a>(n: &'a Node, name: &str) -> Option<&'a str> {
        n.attrs
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    /// `getElementsByTagName` — deep, document order, `"*"` matches
    /// every element.
    #[must_use]
    pub fn by_tag(&self, from: NodeId, tag: &str) -> Vec<NodeId> {
        let mut out = Vec::new();
        self.collect_tag(from, &tag.to_lowercase(), &mut out);
        out
    }

    fn collect_tag(&self, node: NodeId, tag: &str, out: &mut Vec<NodeId>) {
        // DFS itératif (JS-C55) : la profondeur DOM est contrôlée
        // par le script, la récursion déborderait la pile Rust.
        let mut stack: Vec<NodeId> = self.nodes[node as usize]
            .children
            .iter()
            .rev()
            .copied()
            .collect();
        while let Some(c) = stack.pop() {
            let n = &self.nodes[c as usize];
            if n.kind == NodeKind::Element && (tag == "*" || n.tag == tag) {
                out.push(c);
            }
            stack.extend(n.children.iter().rev());
        }
    }

    /// `getElementsByClassName` — token-set membership.
    #[must_use]
    pub fn by_class(&self, from: NodeId, class: &str) -> Vec<NodeId> {
        let mut out = Vec::new();
        self.collect_class(from, class, &mut out);
        out
    }

    fn collect_class(&self, node: NodeId, class: &str, out: &mut Vec<NodeId>) {
        let mut stack: Vec<NodeId> = self.nodes[node as usize]
            .children
            .iter()
            .rev()
            .copied()
            .collect();
        while let Some(c) = stack.pop() {
            let n = &self.nodes[c as usize];
            if n.kind == NodeKind::Element
                && Self::attr_val(n, "class")
                    .is_some_and(|v| v.split_whitespace().any(|t| t == class))
            {
                out.push(c);
            }
            stack.extend(n.children.iter().rev());
        }
    }

    /// `querySelector(All)` — sous-ensemble documenté : compounds
    /// `tag`/`#id`/`.class`/`[attr]`/`[attr op v]` (répétables,
    /// valeur nue ou quotée, flag `i`/`s`), `*`, pseudo-classes
    /// vérifiables (`:defined`, `:not()`, `:is()`, structurelles,
    /// états — JS-C99), combinatoires descendant (`a b`) **et
    /// enfant (`a > b` — JS-C120, spectrum `main > div`)**, listes
    /// `a, b`. `[]` pour le reste.
    #[must_use]
    pub fn query(&self, from: NodeId, sel: &str) -> Vec<NodeId> {
        let Some(list) = split_selector_list(sel) else {
            return Vec::new();
        };
        // `:scope` sur `document.qS` = l'élément racine (spec —
        // `from` n'est pas un élément quand c'est le document).
        let scope = if from == self.root {
            self.root_element().unwrap_or(from)
        } else {
            from
        };
        let mut matched = std::collections::HashSet::new();
        for item in &list {
            let Some(parts) = split_complex_parts(item) else {
                continue;
            };
            let Some(last) = parts.last() else {
                continue;
            };
            let Some(simple) = parse_simple_selector(last.text, 0) else {
                continue;
            };
            let mut out = Vec::new();
            self.collect_query(from, &simple, scope, &mut out);
            // Descendant combinator : ancêtres matchent la chaîne
            // gauche→droite (bornée à `from`).
            if parts.len() > 1 {
                let prefix = &parts[..parts.len() - 1];
                out.retain(|&n| self.ancestor_matches(n, prefix, scope));
            }
            matched.extend(out);
        }
        // Ordre document — DFS unique sur `from`.
        let mut out = Vec::new();
        self.collect_ordered(from, &matched, &mut out);
        out
    }

    /// Does some ancestor chain of `node` (within `from`) match the
    /// selector `parts` right-to-left? `parts[i].child` = `>` joint
    /// au compound suivant — parent direct exigé, sinon marche
    /// d'ancêtres (descendant). Used by the combinators and by
    /// `closest`/`matches` on compound selectors. Recursion bounded
    /// by `parts.len()` ≤ [`MAX_SELECTOR_PARTS`] — a longer compound
    /// selector is outside the documented subset and simply fails
    /// to match (JS-C55).
    fn ancestor_matches(&self, node: NodeId, parts: &[SelPart<'_>], scope: NodeId) -> bool {
        if parts.len() > MAX_SELECTOR_PARTS {
            return false;
        }
        let Some((last, rest)) = parts.split_last() else {
            return true;
        };
        let Some(simple) = parse_simple_selector(last.text, 0) else {
            return false;
        };
        if last.child {
            // `a > b` — le parent seul décide. `bound` N'EST pas
            // exclu : `el.qSA(':scope > *')` exige que le parent
            // (l'élément scopé) matche — spec `qSA` évalue en
            // contexte document. Le nœud document échoue de
            // lui-même au test `kind` dans `node_matches`.
            let Some(p) = self.parent(node) else {
                return false;
            };
            return self.node_matches(p, &simple, scope) && self.ancestor_matches(p, rest, scope);
        }
        let mut cur = self.parent(node);
        let mut seen = std::collections::HashSet::new();
        while let Some(a) = cur {
            // La marche monte jusqu'à la racine : `qSA` évalue le
            // sélecteur en contexte document — un ancêtre au-dessus
            // de `bound` compte pour le match (seul le RÉSULTAT est
            // confiné au sous-arbre). `seen` borne les cycles.
            if !seen.insert(a) {
                break;
            }
            if self.node_matches(a, &simple, scope) && self.ancestor_matches(a, rest, scope) {
                return true;
            }
            cur = self.parent(a);
        }
        false
    }

    fn collect_query(&self, node: NodeId, sel: &SimpleSel, scope: NodeId, out: &mut Vec<NodeId>) {
        let mut stack: Vec<NodeId> = self.nodes[node as usize]
            .children
            .iter()
            .rev()
            .copied()
            .collect();
        // Dédup : un arbre dégénéré (cycle d'enfants — arène
        // host-content) ne peut pas faire tourner la pile
        // sans fin (wedge gov.uk : tout walk borne sa visite).
        let mut seen = std::collections::HashSet::new();
        while let Some(c) = stack.pop() {
            if !seen.insert(c) {
                continue;
            }
            if self.node_matches(c, sel, scope) {
                out.push(c);
            }
            stack.extend(self.nodes[c as usize].children.iter().rev());
        }
    }

    /// DFS document-order — les `NodeId` déjà matchés (listes
    /// `a, b` dédoublonnées).
    fn collect_ordered(
        &self,
        node: NodeId,
        set: &std::collections::HashSet<NodeId>,
        out: &mut Vec<NodeId>,
    ) {
        let mut stack: Vec<NodeId> = self.nodes[node as usize]
            .children
            .iter()
            .rev()
            .copied()
            .collect();
        let mut seen = std::collections::HashSet::new();
        while let Some(c) = stack.pop() {
            if !seen.insert(c) {
                continue;
            }
            if set.contains(&c) {
                out.push(c);
            }
            stack.extend(self.nodes[c as usize].children.iter().rev());
        }
    }

    /// Single-node selector test — `matches()` and `collect_query`
    /// share the same simple-selector subset. `scope` = nœud de
    /// contexte `:scope` (`from` pour query, l'élément pour
    /// `matches`/`closest`).
    fn node_matches(&self, node: NodeId, sel: &SimpleSel, scope: NodeId) -> bool {
        let n = &self.nodes[node as usize];
        if n.kind != NodeKind::Element {
            return false;
        }
        if let Some(t) = &sel.tag {
            if n.tag != *t {
                return false;
            }
        }
        if let Some(i) = &sel.id {
            if Self::attr_val(n, "id") != Some(i.as_str()) {
                return false;
            }
        }
        sel.classes.iter().all(|cls| {
            Self::attr_val(n, "class").is_some_and(|v| v.split_whitespace().any(|t| t == cls))
        }) && sel.attrs.iter().all(|a| {
            Self::attr_val(n, &a.name).is_some_and(|v| attr_op_matches(&a.op, v, &a.value, a.ci))
        }) && sel
            .pseudos
            .iter()
            .all(|p| self.pseudo_matches(node, n, p, scope))
    }

    /// Évalue une pseudo-class du sous-ensemble (JS-C99).
    fn pseudo_matches(&self, node: NodeId, n: &Node, p: &Pseudo, scope: NodeId) -> bool {
        match p {
            // `:defined` — builtin ou custom element défini
            // (un nom avec `-` non défini n'est PAS defined —
            // la chaîne Shoelace `:not(:defined)` mesurée).
            Pseudo::Defined => {
                !n.tag.contains('-')
                    || self.ce.defs.contains_key(&n.tag)
                    || self.ce.upgraded.contains_key(&node)
            }
            Pseudo::Not(list) => !list.iter().any(|s| self.node_matches(node, s, scope)),
            Pseudo::Any(list) => list.iter().any(|s| self.node_matches(node, s, scope)),
            Pseudo::Empty => n.children.iter().all(|&c| {
                let cn = &self.nodes[c as usize];
                cn.kind != NodeKind::Element && (cn.kind != NodeKind::Text || cn.text.is_empty())
            }),
            Pseudo::Root => self.root_element() == Some(node),
            Pseudo::Scope => node == scope,
            Pseudo::FirstChild | Pseudo::LastChild | Pseudo::OnlyChild => {
                self.elem_siblings(node).is_some_and(|(sibs, idx)| match p {
                    Pseudo::FirstChild => idx == 0,
                    Pseudo::LastChild => idx + 1 == sibs.len(),
                    _ => sibs.len() == 1,
                })
            }
            Pseudo::FirstOfType | Pseudo::LastOfType | Pseudo::OnlyOfType => {
                self.type_siblings(node).is_some_and(|(sibs, idx)| match p {
                    Pseudo::FirstOfType => idx == 0,
                    Pseudo::LastOfType => idx + 1 == sibs.len(),
                    _ => sibs.len() == 1,
                })
            }
            Pseudo::NthChild(a, b) => self
                .elem_siblings(node)
                .is_some_and(|(_, idx)| nth_ok(idx + 1, *a, *b)),
            Pseudo::NthLastChild(a, b) => self
                .elem_siblings(node)
                .is_some_and(|(sibs, idx)| nth_ok(sibs.len() - idx, *a, *b)),
            Pseudo::NthOfType(a, b) => self
                .type_siblings(node)
                .is_some_and(|(_, idx)| nth_ok(idx + 1, *a, *b)),
            Pseudo::NthLastOfType(a, b) => self
                .type_siblings(node)
                .is_some_and(|(sibs, idx)| nth_ok(sibs.len() - idx, *a, *b)),
            // `:checked` et `:default` partagent le test
            // attributaire borné (`input[checked]`/
            // `option[selected]` — la distinction « état
            // initial » vs « état courant » n'est pas suivie).
            Pseudo::Checked | Pseudo::Default => {
                (n.tag == "input" && Self::attr_val(n, "checked").is_some())
                    || (n.tag == "option" && Self::attr_val(n, "selected").is_some())
            }
            Pseudo::Disabled => {
                Self::is_form_control(&n.tag) && Self::attr_val(n, "disabled").is_some()
            }
            Pseudo::Enabled => {
                Self::is_form_control(&n.tag) && Self::attr_val(n, "disabled").is_none()
            }
            Pseudo::Link => {
                matches!(n.tag.as_str(), "a" | "area") && Self::attr_val(n, "href").is_some()
            }
            Pseudo::Never => false,
            Pseudo::Focus => self.focused.get() == Some(node),
            Pseudo::FocusWithin => {
                let mut cur = self.focused.get();
                while let Some(f) = cur {
                    if f == node {
                        return true;
                    }
                    cur = self.parent(f);
                }
                false
            }
            Pseudo::Required => {
                matches!(n.tag.as_str(), "input" | "select" | "textarea")
                    && Self::attr_val(n, "required").is_some()
            }
            Pseudo::Optional => {
                matches!(n.tag.as_str(), "input" | "select" | "textarea")
                    && Self::attr_val(n, "required").is_none()
            }
            Pseudo::ReadWrite => Self::is_read_write(n),
            Pseudo::ReadOnly => !Self::is_read_write(n),
            Pseudo::PlaceholderShown => {
                matches!(n.tag.as_str(), "input" | "textarea")
                    && Self::attr_val(n, "placeholder").is_some()
                    && Self::attr_val(n, "value").is_none_or(str::is_empty)
            }
            Pseudo::Open => {
                matches!(n.tag.as_str(), "details" | "dialog")
                    && Self::attr_val(n, "open").is_some()
            }
            Pseudo::Closed => {
                matches!(n.tag.as_str(), "details" | "dialog")
                    && Self::attr_val(n, "open").is_none()
            }
            Pseudo::Indeterminate => n.tag == "progress" && Self::attr_val(n, "value").is_none(),
            Pseudo::Dir(v) => self
                .inherited_attr(node, "dir")
                .is_some_and(|d| d.to_ascii_lowercase() == *v),
            Pseudo::Lang(v) => self.inherited_attr(node, "lang").is_some_and(|l| {
                let l = l.to_ascii_lowercase();
                l == *v || l.starts_with(&format!("{v}-"))
            }),
        }
    }

    /// Frères éléments (`idx` = position de `node`) — `None`
    /// si pas de parent.
    fn elem_siblings(&self, node: NodeId) -> Option<(Vec<NodeId>, usize)> {
        let p = self.nodes[node as usize].parent?;
        let sibs: Vec<NodeId> = self.nodes[p as usize]
            .children
            .iter()
            .copied()
            .filter(|&c| self.nodes[c as usize].kind == NodeKind::Element)
            .collect();
        let idx = sibs.iter().position(|&c| c == node)?;
        Some((sibs, idx))
    }

    /// Frères éléments de même tag (`idx` = position de `node`).
    fn type_siblings(&self, node: NodeId) -> Option<(Vec<NodeId>, usize)> {
        let p = self.nodes[node as usize].parent?;
        let tag = self.nodes[node as usize].tag.as_str();
        let sibs: Vec<NodeId> = self.nodes[p as usize]
            .children
            .iter()
            .copied()
            .filter(|&c| {
                let cn = &self.nodes[c as usize];
                cn.kind == NodeKind::Element && cn.tag == tag
            })
            .collect();
        let idx = sibs.iter().position(|&c| c == node)?;
        Some((sibs, idx))
    }

    /// Premier élément enfant du document (`documentElement`).
    fn root_element(&self) -> Option<NodeId> {
        self.nodes[self.root as usize]
            .children
            .iter()
            .copied()
            .find(|&c| self.nodes[c as usize].kind == NodeKind::Element)
    }

    /// Attr hérité — premier `dir`/`lang` rencontré en
    /// remontant (soi inclus).
    fn inherited_attr(&self, node: NodeId, name: &str) -> Option<String> {
        let mut cur = Some(node);
        while let Some(n) = cur {
            if let Some(v) = Self::attr_val(&self.nodes[n as usize], name) {
                return Some(v.to_string());
            }
            cur = self.parent(n);
        }
        None
    }

    /// Élément éditable — `contenteditable` ≠ `false` ou
    /// `input`/`textarea` sans `readonly`/`disabled` (spec :
    /// un champ verrouillé est `:read-only`).
    fn is_read_write(n: &Node) -> bool {
        if let Some(ce) = Self::attr_val(n, "contenteditable") {
            return !ce.eq_ignore_ascii_case("false");
        }
        matches!(n.tag.as_str(), "input" | "textarea")
            && Self::attr_val(n, "readonly").is_none()
            && Self::attr_val(n, "disabled").is_none()
    }

    /// Tags recevant `disabled` (borné — `fieldset` exclu de
    /// la cascade « premier fieldset »).
    fn is_form_control(tag: &str) -> bool {
        matches!(
            tag,
            "input" | "select" | "textarea" | "button" | "optgroup" | "option" | "fieldset"
        )
    }

    // -- text & serialization ---------------------------------------------

    /// `textContent` — concatenated descendant text. Pour un
    /// nœud `Text`/`Comment`, c'est la data elle-même (spec).
    #[must_use]
    pub fn text_content(&self, node: NodeId) -> String {
        let n = &self.nodes[node as usize];
        if matches!(n.kind, NodeKind::Text | NodeKind::Comment) {
            return n.text.clone();
        }
        let mut out = String::new();
        self.collect_text(node, &mut out);
        out
    }

    fn collect_text(&self, node: NodeId, out: &mut String) {
        // DFS itératif (JS-C55) — même raison que `collect_tag`.
        let mut stack = vec![node];
        while let Some(c) = stack.pop() {
            let n = &self.nodes[c as usize];
            if n.kind == NodeKind::Text {
                out.push_str(&n.text);
            }
            stack.extend(n.children.iter().rev());
        }
    }

    /// `innerHTML` getter — serialize children (`<template>` :
    /// le contenu de son fragment — spec `template contents`).
    #[must_use]
    pub fn inner_html(&self, node: NodeId) -> String {
        let mut out = String::new();
        for &c in self.serial_children(node) {
            self.serialize(c, &mut out);
        }
        out
    }

    fn serialize(&self, node: NodeId, out: &mut String) {
        // Itératif (JS-C55) : Enter ouvre l'élément, Exit le ferme,
        // Rawtext émet un nœud texte verbatim (commentaire rawtext
        // ci-dessous). Pile explicite — la profondeur DOM est
        // contrôlée par le script.
        enum Ser {
            Enter(NodeId),
            Exit(NodeId),
            Rawtext(NodeId),
        }
        let mut stack = vec![Ser::Enter(node)];
        while let Some(ev) = stack.pop() {
            match ev {
                Ser::Exit(id) => {
                    let n = &self.nodes[id as usize];
                    out.push_str("</");
                    out.push_str(&n.tag);
                    out.push('>');
                }
                Ser::Rawtext(id) => {
                    let child = &self.nodes[id as usize];
                    let parent = &self.nodes[child.parent.unwrap_or(id) as usize];
                    let marker = format!("</{}", parent.tag);
                    if contains_ci(&child.text, &marker) {
                        out.push_str(&escape_text(&child.text));
                    } else {
                        out.push_str(&child.text);
                    }
                }
                Ser::Enter(id) => {
                    let n = &self.nodes[id as usize];
                    if n.kind == NodeKind::Text {
                        out.push_str(&escape_text(&n.text));
                        continue;
                    }
                    // Comment — data verbatim entre marqueurs
                    // (spec sérialisation ; les marqueurs Lit
                    // `?lit$N$` ressortent pour le re-parse).
                    if n.kind == NodeKind::Comment {
                        out.push_str("<!--");
                        out.push_str(&n.text);
                        out.push_str("-->");
                        continue;
                    }
                    // Fragment : pas de balise englobante —
                    // `innerHTML` d'un `DocumentFragment` =
                    // ses enfants (spec sérialisation). Les
                    // racines shadow ne sont PAS émises ici
                    // (spec innerHTML/outerHTML — le transport
                    // DSD n'existe que dans `serialize_keyed`,
                    // canal synchro, R-96).
                    if n.kind == NodeKind::Fragment {
                        for &c in n.children.iter().rev() {
                            stack.push(Ser::Enter(c));
                        }
                        continue;
                    }
                    out.push('<');
                    out.push_str(&n.tag);
                    for (k, v) in &n.attrs {
                        out.push(' ');
                        out.push_str(k);
                        out.push_str("=\"");
                        out.push_str(&escape_attr(v));
                        out.push('"');
                    }
                    out.push('>');
                    if VOID_TAGS.contains(&n.tag.as_str()) {
                        continue;
                    }
                    stack.push(Ser::Exit(id));
                    // Texte brut des éléments rawtext (étape 55c) :
                    // la re-sérialisation canal miroir→Dom doit
                    // émettre le contenu **verbatim** —
                    // l'échappement `&`/`<` n'est pas décodé au
                    // re-parse rawtext et corrompt la source à
                    // chaque resynchronisation (`&lt;` →
                    // `&amp;lt;`). Repli échappé si la source
                    // contient son propre `</tag` (sans lui la
                    // frontière casse et le reste fuirait en nœud
                    // texte visible).
                    // `title`/`textarea` sont RCDATA —
                    // l'échappement normal est décodé fidèlement
                    // au re-parse.
                    let verbatim = RAW_VERBATIM.contains(&n.tag.as_str());
                    for &c in self.serial_children(id).iter().rev() {
                        let child = &self.nodes[c as usize];
                        if verbatim && child.kind == NodeKind::Text {
                            stack.push(Ser::Rawtext(c));
                        } else {
                            stack.push(Ser::Enter(c));
                        }
                    }
                }
            }
        }
    }

    // -- fragment parsing ---------------------------------------------------

    /// Parse `html` as a fragment and attach it under `parent`.
    /// Tolerant by design: unknown entities pass through, unclosed
    /// tags simply stay open, `</x>` pops to the nearest open `x`.
    pub fn parse_fragment_into(&mut self, parent: NodeId, html: &str) {
        let mut stack = vec![parent];
        // Compte des tags ouverts dans `stack` — les recherches
        // d'ancêtre (`head` ouvert, `html`/`body` implicites,
        // close-tag sans correspondant) scannaient la pile
        // entière par balise : 200 k divs nichés → O(n²),
        // ~8 min par expression hostile (`hostile_dom_depth`).
        // Incrémentée au push, décrémentée à chaque sortie de
        // pile — O(1) amorti (chaque nœud n'est compté qu'à
        // son retrait). Le parent du fragment compte aussi :
        // les scans d'origine le couvraient (index 0).
        let mut open_tags: HashMap<String, u32> = HashMap::new();
        if let Some(n) = self.nodes.get(parent as usize) {
            *open_tags.entry(n.tag.clone()).or_default() += 1;
        }
        macro_rules! unpush {
            ($slice:expr) => {
                for &n in $slice {
                    if let Some(c) = open_tags.get_mut(&self.nodes[n as usize].tag) {
                        *c = c.saturating_sub(1);
                    }
                }
            };
        }
        let cs: Vec<char> = html.chars().collect();
        let mut i = 0;
        let mut text = String::new();
        macro_rules! flush_text {
            () => {
                if !text.is_empty() {
                    // JS-C46 : les références de caractères sont
                    // décodées comme dans tout texte HTML — sinon
                    // la resérialisation ré-échappe `&` et la page
                    // affiche `&amp;nbsp;` en clair (triage M-H1).
                    let t = self.create_text(&decode_entities(&text));
                    let top = *stack.last().unwrap_or(&parent);
                    self.append(top, t);
                    text.clear();
                }
            };
        }
        while i < cs.len() {
            if cs[i] != '<' {
                text.push(cs[i]);
                i += 1;
                continue;
            }
            // Comments (`<!--…-->`) — émis en nœuds Comment
            // (nodeType 8) : Lit y pose ses marqueurs de parties
            // `?lit$N$` via innerHTML — doctype/PIs restent sautés.
            if cs[i + 1..].starts_with(&['!', '-', '-']) {
                i += 4;
                let start = i;
                while i + 2 < cs.len() && !(cs[i] == '-' && cs[i + 1] == '-' && cs[i + 2] == '>') {
                    i += 1;
                }
                let body: String = cs[start..i].iter().collect();
                i = (i + 3).min(cs.len());
                flush_text!();
                let c = self.create_comment(&body);
                let top = *stack.last().unwrap_or(&parent);
                self.append(top, c);
                continue;
            }
            if matches!(cs.get(i + 1), Some('!' | '?')) {
                while i < cs.len() && cs[i] != '>' {
                    i += 1;
                }
                i += 1;
                continue;
            }
            if cs.get(i + 1) == Some(&'/') {
                // Close tag: pop to matching element if open.
                flush_text!();
                i += 2;
                let name_start = i;
                while i < cs.len() && (cs[i].is_alphanumeric() || cs[i] == '-') {
                    i += 1;
                }
                let name: String = cs[name_start..i].iter().collect::<String>().to_lowercase();
                while i < cs.len() && cs[i] != '>' {
                    i += 1;
                }
                i += 1;
                if name.is_empty() {
                    continue;
                }
                // Pas de correspondant ouvert → le rposition
                // balayait toute la pile pour rien (O(profondeur)
                // par close-tag orphelin — O(n²) hostile).
                if open_tags.get(&name).copied().unwrap_or(0) > 0 {
                    if let Some(pos) = stack
                        .iter()
                        .rposition(|&id| self.nodes[id as usize].tag == name)
                    {
                        // Pop through the match, but never the fragment parent.
                        if pos > 0 {
                            unpush!(&stack[pos..]);
                            stack.truncate(pos);
                        }
                    }
                }
                continue;
            }
            // Start tag.
            flush_text!();
            i += 1;
            let name_start = i;
            while i < cs.len() && (cs[i].is_alphanumeric() || cs[i] == '-' || cs[i] == ':') {
                i += 1;
            }
            if i == name_start {
                text.push('<'); // stray '<' — literal text
                continue;
            }
            let tag: String = cs[name_start..i].iter().collect::<String>().to_lowercase();
            // Attributes.
            let mut attrs: Vec<(String, String)> = Vec::new();
            let mut self_close = false;
            loop {
                while i < cs.len() && cs[i].is_whitespace() {
                    i += 1;
                }
                match cs.get(i) {
                    None | Some('>') => break,
                    Some('/') => {
                        i += 1;
                        if cs.get(i) == Some(&'>') {
                            self_close = true;
                            i += 1;
                            break;
                        }
                    }
                    _ => {
                        let astart = i;
                        while i < cs.len()
                            && !matches!(cs[i], '=' | '>' | '/')
                            && !cs[i].is_whitespace()
                        {
                            i += 1;
                        }
                        let aname: String = cs[astart..i].iter().collect::<String>().to_lowercase();
                        while i < cs.len() && cs[i].is_whitespace() {
                            i += 1;
                        }
                        let mut aval = String::new();
                        if cs.get(i) == Some(&'=') {
                            i += 1;
                            while i < cs.len() && cs[i].is_whitespace() {
                                i += 1;
                            }
                            if let Some(&q @ ('"' | '\'')) = cs.get(i) {
                                i += 1;
                                let vstart = i;
                                while i < cs.len() && cs[i] != q {
                                    i += 1;
                                }
                                aval = cs[vstart..i].iter().collect();
                                i += 1;
                            } else {
                                let vstart = i;
                                while i < cs.len()
                                    && !matches!(cs[i], '>')
                                    && !cs[i].is_whitespace()
                                {
                                    i += 1;
                                }
                                aval = cs[vstart..i].iter().collect();
                            }
                        }
                        if !aname.is_empty() {
                            attrs.push((aname, decode_entities(&aval)));
                        }
                    }
                }
            }
            if i < cs.len() && cs[i] == '>' {
                i += 1;
            }
            if tag.is_empty() {
                continue;
            }
            // Implicit-close par paires — « implied end tags »
            // borné : `dd`/`dt` se ferment l'un l'autre, `tr`
            // ferme `td`/`th` **puis** `tr`, etc. Dépile jusqu'à
            // ce qu'une cible ait été poppée (traverse les
            // intermédiaires, spec) — s'arrêter au premier
            // closable laissait `tr` ouvert sous `tr>td`
            // (mesuré w3.org, `EXPLORER_GEOM_TRACE`). Ne
            // s'applique que si un ancêtre cible est présent —
            // sinon l'élément reste niché (tolérant, aucune
            // perte de contenu).
            if let Some((_, targets)) = PAIRED_CLOSE.iter().find(|(t, _)| *t == tag) {
                if targets
                    .iter()
                    .any(|t| open_tags.get(*t).copied().unwrap_or(0) > 0)
                {
                    while let Some(&top) = stack.last() {
                        let done = targets.contains(&self.nodes[top as usize].tag.as_str());
                        unpush!(std::slice::from_ref(&top));
                        stack.pop();
                        if done {
                            break;
                        }
                    }
                }
            }
            // HTML5 « in head » : un start-tag hors contenu head
            // ferme `<head>` implicitement — w3.org sert `<body>`
            // sans `</head>` littéral ; sans la règle le miroir
            // niche `body` sous `head` et **tous** les chemins
            // `element_path` divergent de l'arbre réel (gBCR 0×0
            // global — mesuré w3.org, `EXPLORER_GEOM_TRACE`).
            // `head` ouvert ? — compteur O(1), pas de scan :
            // sinon chaque `<div>` payait O(profondeur) sur la
            // pile (cause racine du O(n²) de `hostile_dom_depth`).
            if !HEAD_TAGS.contains(&tag.as_str()) && open_tags.get("head").copied().unwrap_or(0) > 0
            {
                if let Some(hp) = stack
                    .iter()
                    .rposition(|&n| self.nodes[n as usize].tag == "head")
                {
                    // Ferme `head` et ses enfants ouverts, puis
                    // spec « after head » : un entrant non
                    // body/frameset/html synthétise `<body>`
                    // implicite (parité de chemins avec le
                    // tree-builder réel, qui fait de même).
                    unpush!(&stack[hp..]);
                    stack.truncate(hp);
                    if !matches!(tag.as_str(), "body" | "frameset" | "html")
                        && open_tags.get("html").copied().unwrap_or(0) > 0
                        && open_tags.get("body").copied().unwrap_or(0) == 0
                    {
                        let b = self.alloc(NodeKind::Element, "body");
                        let t = *stack.last().unwrap_or(&parent);
                        self.append(t, b);
                        *open_tags.entry("body".to_string()).or_default() += 1;
                        stack.push(b);
                    }
                }
            }
            let id = self.alloc(NodeKind::Element, &tag);
            self.nodes[id as usize].attrs = attrs;
            let top = *stack.last().unwrap_or(&parent);
            self.append(top, id);
            if !self_close && !VOID_TAGS.contains(&tag.as_str()) {
                if RAW_TEXT.contains(&tag.as_str()) {
                    // script/style/textarea: texte brut jusqu'à
                    // `</tag` — recherche **directement dans `cs`**,
                    // insensible à la casse. Ne jamais passer par
                    // `String::to_lowercase` sur le reste : la
                    // casse d'une rune peut changer sa longueur
                    // en octets (`ﬀ` → `ff`, `İ` → `i̇`…) et
                    // l'index ramené désaligne la coupe — le
                    // `</tag` raté laissait le contenu brut fuir
                    // hors de l'élément (source `<script>` peinte
                    // en texte, wikipedia — étape 55c).
                    let close_len = 2 + tag.chars().count();
                    let mut pos = None;
                    let mut j = i;
                    while j + close_len <= cs.len() {
                        if cs[j] == '<'
                            && cs[j + 1] == '/'
                            && tag
                                .chars()
                                .enumerate()
                                .all(|(k, tc)| cs[j + 2 + k].to_ascii_lowercase() == tc)
                        {
                            pos = Some(j);
                            break;
                        }
                        j += 1;
                    }
                    let inner_end = pos.unwrap_or(cs.len());
                    if inner_end > i {
                        let inner: String = cs[i..inner_end].iter().collect();
                        // `title`/`textarea` sont RCDATA : leurs
                        // entités sont décodées comme le ferait
                        // le tokenizer réel. `script`/`style`
                        // restent verbatim.
                        let t = if RCDATA_TAGS.contains(&tag.as_str()) {
                            self.create_text(&decode_entities(&inner))
                        } else {
                            self.create_text(&inner)
                        };
                        self.append(id, t);
                    }
                    i = pos.map_or(cs.len(), |p| p + close_len);
                    // Skip to `>` past the close tag.
                    while i < cs.len() && cs[i] != '>' {
                        i += 1;
                    }
                    i = (i + 1).min(cs.len());
                    continue; // don't push — already closed
                }
                *open_tags.entry(tag.clone()).or_default() += 1;
                stack.push(id);
            }
        }
        flush_text!();
    }
}

/// HTML void elements.
const VOID_TAGS: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source",
    "track", "wbr",
];

/// Raw-text elements (content is text until matching close tag).
const RAW_TEXT: &[&str] = &["script", "style", "textarea", "title"];

/// Éléments rawtext émis **verbatim** à la sérialisation
/// (étape 55c) — le re-parseur ne décode pas d'entités en
/// rawtext : échapper corromprait la source à chaque
/// round-trip (`&lt;` → `&amp;lt;`). `title`/`textarea`
/// sont RCDATA (décodés) et restent échappés normalement ;
/// le set doit rester **en correspondance** avec les
/// entrées non-RCDATA de `RAW_TEXT`.
const RAW_VERBATIM: &[&str] = &["script", "style"];

/// Sous-ensemble RCDATA de `RAW_TEXT` — entités décodées à
/// la parse comme à la re-parse.
const RCDATA_TAGS: &[&str] = &["title", "textarea"];

/// `hay` contient-il `needle` (ASCII case-insensitive) —
/// petit scan sans allocation, pour la détection `</tag`.
fn contains_ci(hay: &str, needle: &str) -> bool {
    let (h, n) = (hay.as_bytes(), needle.as_bytes());
    n.len() <= h.len() && h.windows(n.len()).any(|w| w.eq_ignore_ascii_case(n))
}

/// Implicit-close par paires (borné, spec « implied end
/// tags ») : start-tag → **cibles** — le dépile traverse
/// les intermédiaires et s'arrête dès qu'une cible a été
/// poppée. `dd`/`dt` se ferment mutuellement ; `tr` ferme
/// `td`/`th` en chemin **puis** `tr` ; `optgroup` traverse
/// `option`. N'agit que si une cible est ancêtre ouverte.
const PAIRED_CLOSE: &[(&str, &[&str])] = &[
    ("li", &["dd", "dt", "li"]),
    ("dd", &["dd", "dt"]),
    ("dt", &["dd", "dt"]),
    ("p", &["p"]),
    ("tr", &["tr"]),
    ("td", &["td", "th"]),
    ("th", &["td", "th"]),
    ("thead", &["thead", "tbody", "tfoot"]),
    ("tbody", &["thead", "tbody", "tfoot"]),
    ("tfoot", &["thead", "tbody", "tfoot"]),
    ("option", &["option"]),
    ("optgroup", &["optgroup"]),
];

/// Contenu admis en `<head>` — mode d'insertion HTML5
/// « in head » (base/link/meta/script/style/title/template…).
/// Tout autre start-tag ferme `<head>` implicitement ;
/// `html`/`head` eux-mêmes n'ouvrent pas de fermeture.
const HEAD_TAGS: &[&str] = &[
    "base", "basefont", "bgsound", "font", "head", "html", "link", "meta", "noframes", "noscript",
    "script", "style", "template", "title",
];

/// `tag` / `#id` / `.class` / `tag.class` / `tag#id` / `*` → parts.
/// Sélecteur simple du sous-ensemble `Dom::query` — un
/// « compound selector » CSS : tag (ou `*`), `#id`, `.class`
/// (répétables), filtres `[attr]` (répétables, opérateurs
/// `=`/`~=`/`|=`/`^=`/`$=`/`*=`, valeurs nues ou quotées, flag
/// `i`/`s` en fin) et pseudo-classes vérifiables sur le miroir
/// (`:defined`, `:not()`, `:is()`, structurelles, états —
/// JS-C99).
#[derive(Default)]
struct SimpleSel {
    tag: Option<String>,
    id: Option<String>,
    classes: Vec<String>,
    attrs: Vec<AttrSel>,
    pseudos: Vec<Pseudo>,
}

/// Pseudo-classes du sous-ensemble sélecteur (JS-C99) —
/// celles évaluables honnêtement sur le miroir : structure
/// (`:empty`, `:first-child`, `nth-*`), état attributaire
/// (`:checked`, `:disabled`, `:open`), registre CE
/// (`:defined`), négation/alternatives (`:not`, `:is`,
/// `:where`), direction/langue (`:dir`, `:lang`), états de
/// focus (`:focus`, `:focus-within`). Les états inconnus du
/// miroir (`:visited`, `:hover`, `:active`, `:target`,
/// `:fullscreen`) deviennent `Never` — jamais matchés, pas
/// un échec de parse (le reste du compound reste évalué).
enum Pseudo {
    /// `:defined` — builtin ou custom element défini
    /// (`ce.defs`/`ce.upgraded`).
    Defined,
    /// `:not(list)` — négation d'une liste de compounds.
    Not(Vec<SimpleSel>),
    /// `:is(list)`/`:where(list)` — alternative de compounds.
    Any(Vec<SimpleSel>),
    /// `:empty` — ni élément ni texte non vide.
    Empty,
    /// `:root` — élément racine du document.
    Root,
    /// `:scope` — le nœud de contexte (`from`/`matches`).
    Scope,
    /// `:first-child`/`:last-child`/`:only-child`.
    FirstChild,
    LastChild,
    OnlyChild,
    /// `:*-of-type` — parmi les frères de même tag.
    FirstOfType,
    LastOfType,
    OnlyOfType,
    /// `:nth-child(an+b)` / `:nth-last-child(an+b)`.
    NthChild(i64, i64),
    NthLastChild(i64, i64),
    /// `:nth-of-type` / `:nth-last-of-type`.
    NthOfType(i64, i64),
    NthLastOfType(i64, i64),
    /// `:checked` — `input[checked]`/`option[selected]`.
    Checked,
    /// `:disabled`/`:enabled` — contrôles ± `[disabled]`.
    Disabled,
    Enabled,
    /// `:link`/`:any-link` — `a`/`area` avec `href`.
    Link,
    /// États jamais exposés par le miroir (`:visited`,
    /// `:hover`, `:active`, `:target`, `:fullscreen`) —
    /// honnêtement `false`.
    Never,
    /// `:focus`/`:focus-visible` — nœud `focused`.
    Focus,
    /// `:focus-within` — nœud ou ancêtre du `focused`.
    FocusWithin,
    /// `:required`/`:optional` — contrôles ± `[required]`.
    Required,
    Optional,
    /// `:read-only`/`:read-write` — `contenteditable` ou
    /// champ éditable.
    ReadOnly,
    ReadWrite,
    /// `:placeholder-shown` — champ avec `placeholder` et
    /// valeur vide.
    PlaceholderShown,
    /// `:default` — `checked`/`selected` attributaires.
    Default,
    /// `:open`/`:closed` — `details`/`dialog` ± `[open]`.
    Open,
    Closed,
    /// `:indeterminate` — `<progress>` sans `value`
    /// (borné : le flag JS checkbox n'est pas suivi).
    Indeterminate,
    /// `:dir(ltr|rtl|auto)` — `dir` hérité.
    Dir(String),
    /// `:lang(x)` — `lang` hérité, préfixe `x`/`x-*`.
    Lang(String),
}

/// Filtre `[name op value]` — `Exists` = présence seule.
struct AttrSel {
    name: String,
    op: AttrOp,
    value: String,
    ci: bool,
}

/// Opérateurs de sélecteur d'attribut (CSS §6.3).
enum AttrOp {
    Exists,
    Eq,
    Includes,
    DashMatch,
    Prefix,
    Suffix,
    Substring,
}

/// Caractère admis dans un identifiant de sélecteur (subset —
/// pas d'échappes `\` : un sélecteur qui en contient sort du
/// sous-ensemble et ne matche rien).
/// Borne du combinatoire descendant : un sélecteur à plus de
/// parties sort du sous-ensemble documenté (la remontée
/// `ancestor_matches` est récursive sur `parts` — JS-C55).
const MAX_SELECTOR_PARTS: usize = 64;

fn is_sel_ident(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c == b'-' || c >= 0x80
}

/// Profondeur d'imbrication des pseudo-fonctions
/// (`:not(:not(...))`) — hostile-content bound.
const MAX_PSEUDO_DEPTH: usize = 8;

/// Coupe `a, b` en items de liste — virgules hors `()`/`[]`
/// (JS-C99 — spec selector list).
fn split_selector_list(sel: &str) -> Option<Vec<&str>> {
    let b = sel.as_bytes();
    let mut depth = 0i32;
    let mut out = Vec::new();
    let mut st = 0;
    for (i, &c) in b.iter().enumerate() {
        match c {
            b'(' | b'[' => depth += 1,
            b')' | b']' => depth -= 1,
            b',' if depth == 0 => {
                out.push(&sel[st..i]);
                st = i + 1;
                if out.len() > MAX_SELECTOR_PARTS {
                    return None;
                }
            }
            _ => {}
        }
        if !(0..=16).contains(&depth) {
            return None;
        }
    }
    if depth != 0 {
        return None;
    }
    out.push(&sel[st..]);
    Some(out)
}

/// Compound d'un sélecteur complexe — `child` vrai quand le
/// compound est joint à son voisin de droite par `>` (sinon
/// descendant).
struct SelPart<'a> {
    text: &'a str,
    child: bool,
}

/// Coupe `a b`/`a > b` en compounds — espaces et `>` hors
/// `()`/`[]` (JS-C99 : un `[a="x y"]` ou `:not( .x )` ne casse
/// plus la découpe ; JS-C120 : `>` terrain spectrum
/// `main > div`).
fn split_complex_parts(sel: &str) -> Option<Vec<SelPart<'_>>> {
    let b = sel.as_bytes();
    let mut depth = 0i32;
    let mut out: Vec<SelPart<'_>> = Vec::new();
    let mut i = 0;
    let mut pending_child = false;
    while i < b.len() {
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= b.len() {
            break;
        }
        if b[i] == b'>' {
            // `a > > b` ou `> a` — hors grammaire.
            if pending_child {
                return None;
            }
            pending_child = true;
            i += 1;
            continue;
        }
        let st = i;
        while i < b.len() {
            match b[i] {
                b'(' | b'[' => depth += 1,
                b')' | b']' => depth -= 1,
                _ if depth == 0 && (b[i].is_ascii_whitespace() || b[i] == b'>') => break,
                _ => {}
            }
            if !(0..=16).contains(&depth) {
                return None;
            }
            i += 1;
        }
        if pending_child {
            // `>` en tête (`> a`) — hors grammaire.
            out.last_mut()?.child = true;
            pending_child = false;
        }
        out.push(SelPart {
            text: &sel[st..i],
            child: false,
        });
        if out.len() > MAX_SELECTOR_PARTS {
            return None;
        }
    }
    // `a >` terminal ou parenthèse/crochet non fermé.
    if depth != 0 || pending_child {
        return None;
    }
    Some(out)
}

/// `(` à `i` → `(`contenu, index après `)` correspondante`)` —
/// parenthèses imbriquées comptées (`:not(:not(x))`).
fn parse_pseudo_arg_raw(s: &str, i: usize) -> Option<(&str, usize)> {
    let b = s.as_bytes();
    if b.get(i) != Some(&b'(') {
        return None;
    }
    let mut depth = 0usize;
    let mut j = i + 1;
    while j < b.len() {
        match b[j] {
            b'(' => depth += 1,
            b')' => {
                if depth == 0 {
                    return Some((&s[i + 1..j], j + 1));
                }
                depth -= 1;
            }
            _ => {}
        }
        if depth > MAX_PSEUDO_DEPTH {
            return None;
        }
        j += 1;
    }
    None
}

/// Contenu d'un `:not()`/`:is()` — liste de compounds
/// (sous-ensemble : pas de combinators internes).
fn parse_pseudo_list(inner: &str, depth: usize) -> Option<Vec<SimpleSel>> {
    if depth >= MAX_PSEUDO_DEPTH {
        return None;
    }
    let parts = split_selector_list(inner)?;
    let mut out = Vec::new();
    for p in parts {
        out.push(parse_simple_selector(p.trim(), depth + 1)?);
    }
    if out.is_empty() {
        return None;
    }
    Some(out)
}

/// `odd`/`even`/`n`/`an+b`/entier → `(a, b)` (CSS nth).
fn parse_nth(inner: &str) -> Option<(i64, i64)> {
    let s: String = inner.chars().filter(|c| !c.is_ascii_whitespace()).collect();
    let s = s.as_str();
    if s.eq_ignore_ascii_case("odd") {
        return Some((2, 1));
    }
    if s.eq_ignore_ascii_case("even") {
        return Some((2, 0));
    }
    if let Some(pos) = s.find(['n', 'N']) {
        let (a_str, n_and_b) = s.split_at(pos);
        let a: i64 = match a_str {
            "" | "+" => 1,
            "-" => -1,
            x => x.parse().ok()?,
        };
        let b: i64 = if n_and_b.len() > 1 {
            n_and_b[1..].parse().ok()?
        } else {
            0
        };
        Some((a, b))
    } else {
        Some((0, s.parse().ok()?))
    }
}

/// `idx` (1-based) satisfait-il `an+b` ? (CSS nth-match.)
fn nth_ok(idx: usize, a: i64, b: i64) -> bool {
    let i = idx as i64;
    if a == 0 {
        return i == b;
    }
    let d = i - b;
    d % a == 0 && d / a >= 0
}

fn parse_simple_selector(sel: &str, depth: usize) -> Option<SimpleSel> {
    // Pas de rejet global ` `/`,`/`>` — les espaces sont
    // légaux dans `()`/`[` (parse attr/pseudo les consomme) ;
    // un caractère inattendu en tête de compound tombe sur
    // l'arm `_` de la boucle.
    if sel.is_empty() || depth > MAX_PSEUDO_DEPTH {
        return None;
    }
    let b = sel.as_bytes();
    let mut i = 0;
    let mut out = SimpleSel::default();
    let start = i;
    // `*` universel — en tête de compound uniquement.
    if b.get(i) == Some(&b'*') {
        i += 1;
    }
    while i < b.len() && is_sel_ident(b[i]) {
        i += 1;
    }
    // `*` seul parse en compound vide — valide (tout élément
    // matche) : le garde « sélecteur vide » plus bas ne doit
    // pas le rejeter (`qSA('*')` rendait [] — spectrum JS-C120).
    let universal = i > start && &sel[start..i] == "*";
    if i > start && !universal {
        out.tag = Some(sel[start..i].to_lowercase());
    }
    while i < b.len() {
        match b[i] {
            b'#' | b'.' => {
                let marker = b[i];
                i += 1;
                let s = i;
                while i < b.len() && is_sel_ident(b[i]) {
                    i += 1;
                }
                if i == s {
                    return None;
                }
                if marker == b'#' {
                    if out.id.is_some() {
                        return None;
                    }
                    out.id = Some(sel[s..i].to_string());
                } else {
                    out.classes.push(sel[s..i].to_string());
                }
            }
            b'[' => {
                let (a, next) = parse_attr_sel(&sel[i..])?;
                out.attrs.push(a);
                i += next;
            }
            b':' => {
                // Pseudo-élément (`::x`) → hors sous-ensemble.
                if b.get(i + 1) == Some(&b':') {
                    return None;
                }
                i += 1;
                let s = i;
                while i < b.len() && is_sel_ident(b[i]) {
                    i += 1;
                }
                if i == s {
                    return None;
                }
                let pname = &sel[s..i];
                let p = match pname {
                    "defined" => Pseudo::Defined,
                    "empty" => Pseudo::Empty,
                    "root" => Pseudo::Root,
                    "scope" => Pseudo::Scope,
                    "first-child" => Pseudo::FirstChild,
                    "last-child" => Pseudo::LastChild,
                    "only-child" => Pseudo::OnlyChild,
                    "first-of-type" => Pseudo::FirstOfType,
                    "last-of-type" => Pseudo::LastOfType,
                    "only-of-type" => Pseudo::OnlyOfType,
                    "checked" => Pseudo::Checked,
                    "disabled" => Pseudo::Disabled,
                    "enabled" => Pseudo::Enabled,
                    "link" | "any-link" => Pseudo::Link,
                    "visited" | "hover" | "active" | "target" | "fullscreen" => Pseudo::Never,
                    "focus" | "focus-visible" => Pseudo::Focus,
                    "focus-within" => Pseudo::FocusWithin,
                    "required" => Pseudo::Required,
                    "optional" => Pseudo::Optional,
                    "read-only" => Pseudo::ReadOnly,
                    "read-write" => Pseudo::ReadWrite,
                    "placeholder-shown" => Pseudo::PlaceholderShown,
                    "default" => Pseudo::Default,
                    "open" => Pseudo::Open,
                    "closed" => Pseudo::Closed,
                    "indeterminate" => Pseudo::Indeterminate,
                    "not" | "is" | "where" => {
                        let (inner, next) = parse_pseudo_arg_raw(sel, i)?;
                        let list = parse_pseudo_list(inner, depth)?;
                        i = next;
                        if pname == "not" {
                            Pseudo::Not(list)
                        } else {
                            Pseudo::Any(list)
                        }
                    }
                    "nth-child" | "nth-last-child" | "nth-of-type" | "nth-last-of-type" => {
                        let (inner, next) = parse_pseudo_arg_raw(sel, i)?;
                        let (a, bo) = parse_nth(inner)?;
                        i = next;
                        match pname {
                            "nth-child" => Pseudo::NthChild(a, bo),
                            "nth-last-child" => Pseudo::NthLastChild(a, bo),
                            "nth-of-type" => Pseudo::NthOfType(a, bo),
                            _ => Pseudo::NthLastOfType(a, bo),
                        }
                    }
                    "dir" | "lang" => {
                        let (inner, next) = parse_pseudo_arg_raw(sel, i)?;
                        let arg = inner.trim().to_ascii_lowercase();
                        if arg.is_empty() {
                            return None;
                        }
                        i = next;
                        if pname == "dir" {
                            if !matches!(arg.as_str(), "ltr" | "rtl" | "auto") {
                                return None;
                            }
                            Pseudo::Dir(arg)
                        } else {
                            Pseudo::Lang(arg)
                        }
                    }
                    // Pseudo-classes hors sous-ensemble
                    // (`:has`, `:state`, `:host*`, `:popover-open`,
                    // `:modal`, `:valid`, `::part`…) → échec de
                    // parse → `[]`, sous-ensemble documenté.
                    _ => return None,
                };
                out.pseudos.push(p);
            }
            _ => return None,
        }
    }
    if !universal
        && out.tag.is_none()
        && out.id.is_none()
        && out.classes.is_empty()
        && out.attrs.is_empty()
        && out.pseudos.is_empty()
    {
        return None;
    }
    Some(out)
}

/// `[…]` en tête de `s` — retourne le filtre + la longueur
/// consommée (jusqu'au `]` inclus). Espaces internes tolérés.
fn parse_attr_sel(s: &str) -> Option<(AttrSel, usize)> {
    let b = s.as_bytes();
    let mut i = 1;
    let skip_ws = |mut i: usize| {
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        i
    };
    i = skip_ws(i);
    let st = i;
    while i < b.len() && is_sel_ident(b[i]) {
        i += 1;
    }
    if i == st {
        return None;
    }
    let name = s[st..i].to_ascii_lowercase();
    i = skip_ws(i);
    let op = match b.get(i) {
        Some(b'~') if b.get(i + 1) == Some(&b'=') => {
            i += 2;
            AttrOp::Includes
        }
        Some(b'|') if b.get(i + 1) == Some(&b'=') => {
            i += 2;
            AttrOp::DashMatch
        }
        Some(b'^') if b.get(i + 1) == Some(&b'=') => {
            i += 2;
            AttrOp::Prefix
        }
        Some(b'$') if b.get(i + 1) == Some(&b'=') => {
            i += 2;
            AttrOp::Suffix
        }
        Some(b'*') if b.get(i + 1) == Some(&b'=') => {
            i += 2;
            AttrOp::Substring
        }
        Some(b'=') => {
            i += 1;
            AttrOp::Eq
        }
        Some(b']') => {
            return Some((
                AttrSel {
                    name,
                    op: AttrOp::Exists,
                    value: String::new(),
                    ci: false,
                },
                i + 1,
            ));
        }
        _ => return None,
    };
    i = skip_ws(i);
    let value = if let Some(&q @ (b'"' | b'\'')) = b.get(i) {
        i += 1;
        let st = i;
        while i < b.len() && b[i] != q {
            i += 1;
        }
        if i >= b.len() {
            return None;
        }
        let v = s[st..i].to_string();
        i += 1;
        v
    } else {
        let st = i;
        while i < b.len() && is_sel_ident(b[i]) {
            i += 1;
        }
        if i == st {
            return None;
        }
        s[st..i].to_string()
    };
    i = skip_ws(i);
    let mut ci = false;
    if let Some(&f) = b.get(i) {
        if f.eq_ignore_ascii_case(&b'i') {
            ci = true;
            i += 1;
        } else if f.eq_ignore_ascii_case(&b's') {
            i += 1;
        }
    }
    i = skip_ws(i);
    if b.get(i) != Some(&b']') {
        return None;
    }
    Some((
        AttrSel {
            name,
            op,
            value,
            ci,
        },
        i + 1,
    ))
}

/// `attr_val op expected` — sémantique CSS §6.3 (valeur vide
/// jamais matchée par les opérateurs de contenu).
fn attr_op_matches(op: &AttrOp, actual: &str, expected: &str, ci: bool) -> bool {
    let (a, e) = if ci {
        (actual.to_lowercase(), expected.to_lowercase())
    } else {
        (actual.to_string(), expected.to_string())
    };
    let (a, e) = (a.as_str(), e.as_str());
    match op {
        AttrOp::Exists => true,
        AttrOp::Eq => a == e,
        AttrOp::Includes => !e.is_empty() && a.split_whitespace().any(|w| w == e),
        AttrOp::DashMatch => {
            !e.is_empty()
                && (a == e || (a.starts_with(e) && a.as_bytes().get(e.len()) == Some(&b'-')))
        }
        AttrOp::Prefix => !e.is_empty() && a.starts_with(e),
        AttrOp::Suffix => !e.is_empty() && a.ends_with(e),
        AttrOp::Substring => !e.is_empty() && a.contains(e),
    }
}

/// FNV-1a 64 — hachage des contenus texte pour les marqueurs
/// `#text` des séries cléées (alignement keymap côté session :
/// borne mémoire + fidélité pleine longueur).
fn fnv(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in s.as_bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

fn escape_text(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn escape_attr(s: &str) -> String {
    escape_text(s).replace('"', "&quot;")
}

/// Decode the character references (JS-C46 — mini-table : le
/// jeu HTML 4.01 ~200 noms du web réel, pas la table WHATWG
/// complète de `explorer-html` — jscore reste autonome ; la
/// couture pour la table complète est loguée dans
/// `docs/interface-equipes.md`). Numériques décimales `&#65;`
/// et hexadécimales `&#x41;`, remap windows-1252 des
/// `0x80..=0x9F` et `U+FFFD` sur invalide (spec). Les entités
/// sans `;` (admises en legacy par la spec) ne sont pas
/// décodées ici — écart connu et assumé : l'esperluette
/// ambiguë exige la table complète.
fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let end = rest.find(';').map_or(0, |e| e + 1);
        if end == 0 || end > 32 {
            out.push('&');
            rest = &rest[1..];
            continue;
        }
        let (entity, tail) = rest.split_at(end);
        let body = &entity[1..entity.len() - 1];
        if let Some(digits) = body.strip_prefix('#') {
            let parsed = digits.strip_prefix(['x', 'X']).map_or_else(
                || digits.parse::<u32>().ok(),
                |h| u32::from_str_radix(h, 16).ok(),
            );
            if let Some(cp) = parsed {
                out.push(map_numeric(cp));
            } else {
                out.push_str(entity);
            }
            rest = tail;
            continue;
        }
        // Inconnue : passthrough verbatim — tolérance par
        // design, le texte brut est moins mauvais que rien.
        out.push_str(named_entity(body).unwrap_or(entity));
        rest = tail;
    }
    out.push_str(rest);
    out
}

/// Remap windows-1252 des références numériques `0x80..=0x9F`
/// (spec : `&#150;` = `–`, pas U+0096) — miroir de
/// `explorer_html::entities::map_numeric`, dupliqué pour
/// l'autonomie jscore.
const NUMERIC_1252: [char; 32] = [
    '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8D}', 'Ž', '\u{8F}',
    '\u{90}', '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{9D}', 'ž', 'Ÿ',
];

/// Codepoint d'une référence numérique selon la spec :
/// `0x00` → `U+FFFD`, surrogates/hors Unicode → `U+FFFD`,
/// `0x80..=0x9F` → remap windows-1252.
fn map_numeric(cp: u32) -> char {
    match cp {
        0x00 => '\u{FFFD}',
        0x80..=0x9F => NUMERIC_1252[(cp - 0x80) as usize],
        cp => char::from_u32(cp).unwrap_or('\u{FFFD}'),
    }
}

/// Jeu nommé HTML 4.01 (~200 références — le web réel) — voir
/// `decode_entities` pour le statut d'intérim.
fn named_entity(name: &str) -> Option<&'static str> {
    Some(match name {
        // Marqueur XML.
        "amp" => "&",
        "lt" => "<",
        "gt" => ">",
        "quot" => "\"",
        "apos" => "'",
        // Espaces, contrôles typo.
        "nbsp" => "\u{A0}",
        "ensp" => "\u{2002}",
        "emsp" => "\u{2003}",
        "thinsp" => "\u{2009}",
        "zwnj" => "\u{200C}",
        "zwj" => "\u{200D}",
        "lrm" => "\u{200E}",
        "rlm" => "\u{200F}",
        "shy" => "\u{AD}",
        // Latin-1 ponctuation et symboles.
        "iexcl" => "¡",
        "cent" => "¢",
        "pound" => "£",
        "curren" => "¤",
        "yen" => "¥",
        "brvbar" => "¦",
        "sect" => "§",
        "uml" => "¨",
        "copy" => "©",
        "ordf" => "ª",
        "laquo" => "«",
        "not" => "¬",
        "reg" => "®",
        "macr" => "¯",
        "deg" => "°",
        "plusmn" => "±",
        "sup2" => "²",
        "sup3" => "³",
        "acute" => "´",
        "micro" => "µ",
        "para" => "¶",
        "middot" => "·",
        "cedil" => "¸",
        "sup1" => "¹",
        "ordm" => "º",
        "raquo" => "»",
        "frac14" => "¼",
        "frac12" => "½",
        "frac34" => "¾",
        "iquest" => "¿",
        "times" => "×",
        "divide" => "÷",
        "euro" => "€",
        "trade" => "™",
        // Latin-1 lettres accentuées.
        "Agrave" => "À",
        "Aacute" => "Á",
        "Acirc" => "Â",
        "Atilde" => "Ã",
        "Auml" => "Ä",
        "Aring" => "Å",
        "AElig" => "Æ",
        "Ccedil" => "Ç",
        "Egrave" => "È",
        "Eacute" => "É",
        "Ecirc" => "Ê",
        "Euml" => "Ë",
        "Igrave" => "Ì",
        "Iacute" => "Í",
        "Icirc" => "Î",
        "Iuml" => "Ï",
        "ETH" => "Ð",
        "Ntilde" => "Ñ",
        "Ograve" => "Ò",
        "Oacute" => "Ó",
        "Ocirc" => "Ô",
        "Otilde" => "Õ",
        "Ouml" => "Ö",
        "Oslash" => "Ø",
        "Ugrave" => "Ù",
        "Uacute" => "Ú",
        "Ucirc" => "Û",
        "Uuml" => "Ü",
        "Yacute" => "Ý",
        "THORN" => "Þ",
        "szlig" => "ß",
        "agrave" => "à",
        "aacute" => "á",
        "acirc" => "â",
        "atilde" => "ã",
        "auml" => "ä",
        "aring" => "å",
        "aelig" => "æ",
        "ccedil" => "ç",
        "egrave" => "è",
        "eacute" => "é",
        "ecirc" => "ê",
        "euml" => "ë",
        "igrave" => "ì",
        "iacute" => "í",
        "icirc" => "î",
        "iuml" => "ï",
        "eth" => "ð",
        "ntilde" => "ñ",
        "ograve" => "ò",
        "oacute" => "ó",
        "ocirc" => "ô",
        "otilde" => "õ",
        "ouml" => "ö",
        "oslash" => "ø",
        "ugrave" => "ù",
        "uacute" => "ú",
        "ucirc" => "û",
        "uuml" => "ü",
        "yacute" => "ý",
        "thorn" => "þ",
        "yuml" => "ÿ",
        "OElig" => "Œ",
        "oelig" => "œ",
        "Scaron" => "Š",
        "scaron" => "š",
        "Yuml" => "Ÿ",
        "fnof" => "ƒ",
        // Typographie : guillemets, tirets, flèches, maths usuels.
        "lsquo" => "‘",
        "rsquo" => "’",
        "sbquo" => "‚",
        "ldquo" => "“",
        "rdquo" => "”",
        "bdquo" => "„",
        "dagger" => "†",
        "Dagger" => "‡",
        "bull" => "•",
        "hellip" => "…",
        "permil" => "‰",
        "prime" => "′",
        "Prime" => "″",
        "lsaquo" => "‹",
        "rsaquo" => "›",
        "oline" => "‾",
        "frasl" => "⁄",
        "ndash" => "–",
        "mdash" => "—",
        "circ" => "ˆ",
        "tilde" => "˜",
        "larr" => "←",
        "uarr" => "↑",
        "rarr" => "→",
        "darr" => "↓",
        "harr" => "↔",
        "minus" => "−",
        "lowast" => "∗",
        "radic" => "√",
        "infin" => "∞",
        "ne" => "≠",
        "equiv" => "≡",
        "le" => "≤",
        "ge" => "≥",
        "asymp" => "≈",
        "sum" => "∑",
        "prod" => "∏",
        "loz" => "◊",
        "spades" => "♠",
        "clubs" => "♣",
        "hearts" => "♥",
        "diams" => "♦",
        _ => return None,
    })
}
