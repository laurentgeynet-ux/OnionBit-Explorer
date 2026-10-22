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

use crate::value::Value;
use std::cell::Cell;
use std::collections::HashMap;

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

/// Kind of node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    /// Element (`<div>`…). `tag` is lowercased; `"#document"` is the
    /// synthetic root.
    Element,
    /// Text run — payload in `text`, `tag` is `"#text"`.
    Text,
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
    /// Ordres média page → hôte en file (étape 71c) —
    /// `(media, cmd)`, drainés par l'adaptateur
    /// (`drain_media_commands`) qui les relaie vers l'hôte.
    pub media_cmds: Vec<(u64, MediaCmd)>,
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
    /// « Document ouvert à l'écriture » (JS-C35) — `true` tant
    /// que la page se charge : `document.write`/`writeln` y
    /// ajoutent à `<body>`. `document.close()` ou la
    /// distribution de `DOMContentLoaded`/`load` le ferme ; un
    /// `write` sur document fermé rouvre implicitement en
    /// effaçant le contenu d'abord (spec `document open steps`).
    pub doc_open: bool,
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
            canvases: HashMap::new(),
            media: HashMap::new(),
            template_contents: HashMap::new(),
            media_cmds: Vec::new(),
            canvas_op_limit: DEFAULT_CANVAS_OP_LIMIT,
            doc_open: true,
            serialize_calls: Cell::new(0),
            serialize_bytes: Cell::new(0),
            serialize_micros: Cell::new(0),
        };
        d.root = d.alloc(NodeKind::Element, "#document");
        d.nodes[d.root as usize].attached = true;
        d
    }

    // -- MutationObserver (JS-C26) ------------------------------------

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
            self.journal.clear();
            self.journal_poisoned = false;
            return None;
        }
        Some(std::mem::take(&mut self.journal))
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
                for &b in n.tag.as_bytes() {
                    h ^= u64::from(b);
                    h = h.wrapping_mul(0x0000_0100_0000_01b3);
                }
                h ^= 0xff;
                h = h.wrapping_mul(0x0000_0100_0000_01b3);
            }
            for &c in self.serial_children(id).iter().rev() {
                stack.push(c);
            }
        }
        h
    }

    /// Sérialisation cléée du document (ADR-0025) : `(html,
    /// ids)` — `ids` = `NodeId` miroir de chaque nœud dans
    /// l'ordre d'émission (pré-ordre de `serialize`,
    /// fragments aplatis sans entrée propre). La session
    /// apparie les `NodeId` frais de son re-parse pour
    /// monter la table d'identité.
    #[must_use]
    pub fn serialize_keyed(&self) -> (String, Vec<u64>) {
        let mut out = String::new();
        let mut ids = Vec::new();
        for &c in self.serial_children(self.root) {
            self.serialize_keyed_node(c, &mut out, &mut ids);
        }
        (out, ids)
    }

    /// Variante sous-arbre de [`Self::serialize_keyed`] —
    /// utilisée par le drain de journal pour les `Insert`
    /// (`outer_html` + ids de tout le sous-arbre).
    #[must_use]
    pub fn serialize_subtree_keyed(&self, node: NodeId) -> (String, Vec<u64>) {
        let mut out = String::new();
        let mut ids = Vec::new();
        self.serialize_keyed_node(node, &mut out, &mut ids);
        (out, ids)
    }

    /// Traversée unique `serialize` + collecte d'ids — mêmes
    /// événements `Enter` que [`Self::serialize`], enregistrés.
    fn serialize_keyed_node(&self, node: NodeId, out: &mut String, ids: &mut Vec<u64>) {
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
                    ids.push(u64::from(id));
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
                        out.push_str(&escape_text(&n.text));
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
                    // valide côté moteur.
                    let index = self.nodes[rec.target as usize]
                        .children
                        .iter()
                        .position(|&c| c == first)
                        .map_or(0, |i| i as u32);
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
        d
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
            stack.extend(self.nodes[c as usize].children.iter().copied());
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
    fn element_path(&self, mut n: NodeId) -> Option<Vec<(String, u32)>> {
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
    pub fn append(&mut self, parent: NodeId, child: NodeId) {
        if self.nodes[child as usize].kind == NodeKind::Fragment {
            self.splice_fragment(parent, child, None);
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
    }

    /// Detach `node` from its parent (keeps the node alive).
    pub fn detach(&mut self, node: NodeId) {
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
    }

    /// All children — elements *and* text (`childNodes`; `children`
    /// is elements-only, see [`Dom::element_children`]).
    #[must_use]
    pub fn child_nodes(&self, node: NodeId) -> Vec<NodeId> {
        self.nodes[node as usize].children.clone()
    }

    /// `parentNode` — `None` for the root or detached nodes.
    #[must_use]
    pub fn parent(&self, node: NodeId) -> Option<NodeId> {
        self.nodes[node as usize].parent
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
        let node = &self.nodes[n as usize];
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
        let p = self.nodes[node as usize].parent?;
        let kids = &self.nodes[p as usize].children;
        let i = kids.iter().position(|&c| c == node)?;
        let j = if next { i + 1 } else { i.checked_sub(1)? };
        kids.get(j).copied()
    }

    /// `insertBefore(child, ref)` — `None` ref ≙ append. If `ref`
    /// isn't a child of `parent` the insert degrades to append
    /// (tolerant — Dillo never throws on malformed trees either).
    pub fn insert_before(&mut self, parent: NodeId, child: NodeId, ref_node: Option<NodeId>) {
        if self.nodes[child as usize].kind == NodeKind::Fragment {
            self.splice_fragment(parent, child, ref_node);
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
    }

    /// Insertion d'un `DocumentFragment` : ses enfants sont
    /// déplacés dans `parent` (devant `ref_node`, fin sinon) et
    /// le fragment reste vide — spec `insert` des fragments.
    /// Un seul record `childList` pour le groupe.
    fn splice_fragment(&mut self, parent: NodeId, frag: NodeId, ref_node: Option<NodeId>) {
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
            kids,
            Vec::new(),
            prev,
            next,
        ));
    }

    /// `replaceChild(new, old)` — `old` is detached, `new` takes
    /// its slot. Returns `old` (spec) — caller wraps it.
    pub fn replace_child(&mut self, parent: NodeId, new: NodeId, old: NodeId) {
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

    /// `el.matches(sel)` — single-node selector test (same subset
    /// as [`Dom::query`]: simple selectors + descendant combinator).
    #[must_use]
    pub fn matches(&self, node: NodeId, sel: &str) -> bool {
        let parts: Vec<&str> = sel.split_whitespace().collect();
        let Some((&last, prefix)) = parts.split_last() else {
            return false;
        };
        let Some(simple) = parse_simple_selector(last) else {
            return false;
        };
        self.node_matches(node, &simple)
            && (prefix.is_empty() || self.ancestor_matches(self.root, node, prefix))
    }

    /// `el.closest(sel)` — nearest ancestor-or-self matching `sel`.
    #[must_use]
    pub fn closest(&self, node: NodeId, sel: &str) -> Option<NodeId> {
        let mut cur = Some(node);
        while let Some(n) = cur {
            if self.matches(n, sel) {
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

    /// `getAttribute` (first occurrence, case-insensitive name).
    #[must_use]
    pub fn attr(&self, node: NodeId, name: &str) -> Option<&str> {
        self.nodes[node as usize]
            .attrs
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    /// `setAttribute`.
    pub fn set_attr(&mut self, node: NodeId, name: &str, value: &str) {
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
            old_value: old,
        });
    }

    /// `removeAttribute`. Returns whether it existed.
    pub fn remove_attr(&mut self, node: NodeId, name: &str) -> bool {
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
                old_value: old,
            });
        }
        removed
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
    /// (empty for the root itself).
    #[must_use]
    pub fn ancestors(&self, node: NodeId) -> Vec<NodeId> {
        let mut out = Vec::new();
        let mut cur = self.nodes[node as usize].parent;
        while let Some(p) = cur {
            out.push(p);
            cur = self.nodes[p as usize].parent;
        }
        out
    }

    /// Element children of `node` (text nodes filtered out).
    #[must_use]
    pub fn element_children(&self, node: NodeId) -> Vec<NodeId> {
        self.nodes[node as usize]
            .children
            .iter()
            .copied()
            .filter(|&c| self.nodes[c as usize].kind == NodeKind::Element)
            .collect()
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
    fn is_attached(&self, node: NodeId) -> bool {
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

    /// Minimal `querySelector(All)`: compound selectors —
    /// `tag`, `#id`, `.class` (répétable), `[attr]`/`[attr=v]`/
    /// `[attr~=v]`/`[attr|=v]`/`[attr^=v]`/`[attr$=v]`/`[attr*=v]`
    /// (répétables, valeur nue ou quotée, flag `i`/`s`), `*`,
    /// plus the descendant combinator (`a b`). `None`/`empty`
    /// for anything more complex (documented subset).
    #[must_use]
    pub fn query(&self, from: NodeId, sel: &str) -> Vec<NodeId> {
        let parts: Vec<&str> = sel.split_whitespace().collect();
        let Some(last) = parts.last() else {
            return Vec::new();
        };
        let Some(simple) = parse_simple_selector(last) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        self.collect_query(from, &simple, &mut out);
        // Descendant combinator: keep only nodes whose ancestors match
        // the left-to-right selector chain (bounded by `from`).
        if parts.len() > 1 {
            let prefix = &parts[..parts.len() - 1];
            out.retain(|&n| self.ancestor_matches(from, n, prefix));
        }
        out
    }

    /// Does some ancestor chain of `node` (within `from`) match the
    /// selector `parts` right-to-left? Used by the descendant
    /// combinator and by `closest` on compound selectors.
    /// Recursion bounded by `parts.len()` ≤ [`MAX_SELECTOR_PARTS`]
    /// — a longer compound selector is outside the documented
    /// subset and simply fails to match (JS-C55).
    fn ancestor_matches(&self, bound: NodeId, node: NodeId, parts: &[&str]) -> bool {
        if parts.len() > MAX_SELECTOR_PARTS {
            return false;
        }
        let Some((&last, rest)) = parts.split_last() else {
            return true;
        };
        let Some(simple) = parse_simple_selector(last) else {
            return false;
        };
        let mut cur = self.parent(node);
        while let Some(a) = cur {
            if a == bound {
                break;
            }
            if self.node_matches(a, &simple) && self.ancestor_matches(bound, a, rest) {
                return true;
            }
            cur = self.parent(a);
        }
        false
    }

    fn collect_query(&self, node: NodeId, sel: &SimpleSel, out: &mut Vec<NodeId>) {
        let mut stack: Vec<NodeId> = self.nodes[node as usize]
            .children
            .iter()
            .rev()
            .copied()
            .collect();
        while let Some(c) = stack.pop() {
            if self.node_matches(c, sel) {
                out.push(c);
            }
            stack.extend(self.nodes[c as usize].children.iter().rev());
        }
    }

    /// Single-node selector test — `matches()` and `collect_query`
    /// share the same simple-selector subset.
    fn node_matches(&self, node: NodeId, sel: &SimpleSel) -> bool {
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
        })
    }

    // -- text & serialization ---------------------------------------------

    /// `textContent` — concatenated descendant text.
    #[must_use]
    pub fn text_content(&self, node: NodeId) -> String {
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
                    // Fragment : pas de balise englobante —
                    // `innerHTML` d'un `DocumentFragment` =
                    // ses enfants (spec sérialisation).
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
            // Comments / doctype / PIs.
            if cs[i + 1..].starts_with(&['!', '-', '-']) {
                i += 4;
                while i + 2 < cs.len() && !(cs[i] == '-' && cs[i + 1] == '-' && cs[i + 2] == '>') {
                    i += 1;
                }
                i = (i + 3).min(cs.len());
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
                if let Some(pos) = stack
                    .iter()
                    .rposition(|&id| self.nodes[id as usize].tag == name)
                {
                    // Pop through the match, but never the fragment parent.
                    if pos > 0 {
                        stack.truncate(pos);
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
            // Minimal implicit-close tolerance (li/p/tr/td/option).
            if IMPLICIT_CLOSE.contains(&tag.as_str()) {
                if let Some(&top) = stack.last() {
                    if self.nodes[top as usize].tag == tag {
                        stack.pop();
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

/// Tags that implicitly close a same-tag sibling.
const IMPLICIT_CLOSE: &[&str] = &["li", "p", "tr", "td", "th", "option", "dt", "dd"];

/// `tag` / `#id` / `.class` / `tag.class` / `tag#id` / `*` → parts.
/// Sélecteur simple du sous-ensemble `Dom::query` — un
/// « compound selector » CSS : tag (ou `*`), `#id`, `.class`
/// (répétables) et filtres `[attr]` (répétables, opérateurs
/// `=`/`~=`/`|=`/`^=`/`$=`/`*=`, valeurs nues ou quotées, flag
/// `i`/`s` en fin).
#[derive(Default)]
struct SimpleSel {
    tag: Option<String>,
    id: Option<String>,
    classes: Vec<String>,
    attrs: Vec<AttrSel>,
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

fn parse_simple_selector(sel: &str) -> Option<SimpleSel> {
    if sel.is_empty() || sel.contains(' ') || sel.contains(',') || sel.contains('>') {
        return None;
    }
    let b = sel.as_bytes();
    let mut i = 0;
    let mut out = SimpleSel::default();
    let start = i;
    while i < b.len() && is_sel_ident(b[i]) {
        i += 1;
    }
    if i > start {
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
            _ => return None,
        }
    }
    if out.tag.is_none() && out.id.is_none() && out.classes.is_empty() && out.attrs.is_empty() {
        return None;
    }
    if out.tag.as_deref() == Some("*") {
        out.tag = None;
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
