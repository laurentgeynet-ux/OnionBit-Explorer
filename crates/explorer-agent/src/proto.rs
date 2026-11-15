// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Messages EAP v1 — miroir de `schema/eap-1.schema.json` (source de
//! vérité). Enveloppe JSON-RPC 2.0 : requêtes et notifications sont
//! discriminées par la présence de `id`.
//!
//! Conventions : `TabId`/`ElementRef` sont opaques côté client ; les
//! `*_ms` sont des millisecondes u32 ; `rev` est un compteur monotone
//! de document commis par onglet (base du diff `{since}`).

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Identifiant d'onglet — même espace que `explorer_core::TabId`,
/// opacité maintenue (jamais interprété côté client).
pub type TabId = u64;

/// Handle d'élément — chaîne opaque `"el:<n>"` allouée par la table
/// de handles de la session agent. Une ref expirée produit
/// [`ErrorCode::StaleRef`].
pub type ElementRef = String;

/// `jsonrpc: "2.0"` — sérialisé tel quel.
pub const JSONRPC: &str = "2.0";

// ------------------------------------------------------------------
// Enveloppes
// ------------------------------------------------------------------

/// Tout message entrant : présence de `id` = requête (réponse
/// attendue), absence = notification (feu et oubli).
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum Incoming {
    /// Appel corrélé — `id` recopié dans la réponse.
    Request {
        /// Corrélateur JSON-RPC (nombre ou chaîne).
        id: Value,
        /// Méthode + paramètres (tag interne `method`/`params`).
        #[serde(flatten)]
        method: Method,
    },
    /// Notification — jamais de réponse.
    Notification {
        /// Méthode + paramètres.
        #[serde(flatten)]
        method: Method,
    },
}

/// Champ `jsonrpc` sérialisé `"2.0"` — marker type (le client
/// reste tolérant : la lecture ignore la valeur).
#[derive(Debug, Clone, Copy, Default)]
pub struct Jsonrpc;

impl Serialize for Jsonrpc {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(JSONRPC)
    }
}
impl<'de> Deserialize<'de> for Jsonrpc {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = String::deserialize(d)?;
        (v == JSONRPC)
            .then_some(Self)
            .ok_or_else(|| serde::de::Error::custom(format!("jsonrpc attendu {JSONRPC}, reçu {v}")))
    }
}

/// Réponse à une requête — `result` xor `error`.
#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum Response {
    /// Succès.
    Ok {
        /// `"2.0"`.
        jsonrpc: Jsonrpc,
        /// Corrélateur recopié.
        id: Value,
        /// Charge — dépend de la méthode.
        result: Value,
    },
    /// Échec typé.
    Err {
        /// `"2.0"`.
        jsonrpc: Jsonrpc,
        /// Corrélateur recopié.
        id: Value,
        /// Erreur JSON-RPC (+ codes EAP `-32001..-32008`).
        error: RpcError,
    },
}

impl Response {
    /// Réponse de succès.
    #[must_use]
    pub fn ok(id: Value, result: Value) -> Self {
        Self::Ok {
            jsonrpc: Jsonrpc,
            id,
            result,
        }
    }
    /// Réponse d'erreur typée.
    #[must_use]
    pub fn err(id: Value, code: ErrorCode, message: String, data: Option<Value>) -> Self {
        Self::Err {
            jsonrpc: Jsonrpc,
            id,
            error: RpcError {
                code: code.into(),
                message,
                data,
            },
        }
    }
}

/// Bloc `error` JSON-RPC.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcError {
    /// Code — [`ErrorCode`] ou code JSON-RPC standard.
    pub code: i64,
    /// Message humain.
    pub message: String,
    /// Détail structuré optionnel (ex. checks d'actionabilité
    /// échoués sous [`ErrorCode::NotActionable`]).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

/// Codes d'erreur EAP (plage `-32001..-32008`, réservée application
/// par JSON-RPC) + codes standard réutilisés.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(into = "i64", try_from = "i64")]
#[repr(i64)]
pub enum ErrorCode {
    /// JSON invalide.
    Parse = -32700,
    /// Méthode inconnue du serveur.
    MethodNotFound = -32601,
    /// Paramètres invalides (schéma).
    InvalidParams = -32602,
    /// Capacité connue mais non livrée par cette implémentation —
    /// réponse légale à une feature non négociée (ADR-0035 §3).
    Unsupported = -32001,
    /// `ref` expirée : nœud mort, re-parse ou navigation depuis
    /// l'émission — l'agent re-snapshote.
    StaleRef = -32002,
    /// Élément résolu mais non actionnable dans le délai —
    /// `data` porte l'[`Actionability`] constatée.
    NotActionable = -32003,
    /// `wait.for` ou auto-wait échu dans `timeout_ms`.
    Timeout = -32004,
    /// Onglet inconnu ou fermé.
    NoSuchTab = -32005,
    /// Sélecteur malformé / syntaxe hors sous-ensemble v1 (CSS).
    BadSelector = -32006,
    /// Moteur momentanément indisponible (pompe en vol, apply
    /// détaché) — réessayer.
    EngineUnavailable = -32007,
    /// Profil de session sans le droit requis (`input`, `eval`,
    /// `tabs` hors scope).
    Unauthorized = -32008,
}

impl From<ErrorCode> for i64 {
    fn from(c: ErrorCode) -> Self {
        c as i64
    }
}
impl TryFrom<i64> for ErrorCode {
    // `i64` en erreur : `Display` requis par serde(try_from), et la
    // valeur fautive est elle-même le message.
    type Error = i64;
    fn try_from(v: i64) -> Result<Self, i64> {
        Ok(match v {
            -32700 => Self::Parse,
            -32601 => Self::MethodNotFound,
            -32602 => Self::InvalidParams,
            -32001 => Self::Unsupported,
            -32002 => Self::StaleRef,
            -32003 => Self::NotActionable,
            -32004 => Self::Timeout,
            -32005 => Self::NoSuchTab,
            -32006 => Self::BadSelector,
            -32007 => Self::EngineUnavailable,
            -32008 => Self::Unauthorized,
            _ => return Err(v),
        })
    }
}

// ------------------------------------------------------------------
// Types partagés
// ------------------------------------------------------------------

/// Capacités négociables à `session.open` — chaîne de feature. Une
/// capacité non livrée répond [`ErrorCode::Unsupported`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    /// `snapshot.get` (arbre sérialisé — DOM v1, sémantique E3).
    Snapshot,
    /// Diff d'arbre `snapshot.get{since}` (E3).
    SnapshotDiff,
    /// Refs d'éléments + actions par handle (E2 — sans elle, E1
    /// n'expose que `input.*_at` en coordonnées).
    Handles,
    /// Vérification d'actionabilité native (E2).
    Actionable,
    /// `wait.for` sur commits internes.
    WaitCommit,
    /// `input.*` (sous le droit `profile.input`).
    Input,
    /// `eval.run` (sous le droit `profile.eval`).
    Eval,
    /// `frame.get` (PNG binaire).
    Frames,
    /// `console.get`.
    ConsoleLog,
    /// `dom.get` — échappatoire debug, coûteux en tokens.
    Dom,
    /// Cycle de vie onglets.
    Tabs,
    /// Notifications `events.*`.
    Events,
}

/// Profil de sécurité de session — annoncé à `session.open`,
/// appliqué par le serveur à chaque appel (ADR-0035 §3).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Profile {
    /// Droit aux actions `input.*` — défaut `false` (lecture seule).
    #[serde(default)]
    pub input: bool,
    /// Droit à `eval.run` — défaut `false` (exécution arbitraire).
    #[serde(default)]
    pub eval: bool,
    /// Portée onglets : `all` ou liste bornée.
    #[serde(default)]
    pub tabs: TabScope,
}

/// Portée d'onglets autorisée par le profil — `"all"` ou `[ids]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TabScope {
    /// Tous les onglets de la session.
    All(AllMarker),
    /// Sous-ensemble explicite.
    Only(Vec<TabId>),
}

impl TabScope {
    /// `All` court.
    #[must_use]
    pub fn all() -> Self {
        Self::All(AllMarker::All)
    }
}

/// Marqueur `"all"` — newtype : un enum unitaire untagged se
/// sérialiserait `null`, pas la chaîne.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AllMarker {
    /// `"all"`.
    All,
}

impl Default for TabScope {
    fn default() -> Self {
        Self::all()
    }
}

/// Rectangle en px CSS de viewport (sauf `frame.get{full}` : px
/// document).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    /// Abscisse.
    pub x: f32,
    /// Ordonnée.
    pub y: f32,
    /// Largeur.
    pub w: f32,
    /// Hauteur.
    pub h: f32,
}

/// Modifieurs clavier — même forme que `explorer_core::KeyMods`.
#[allow(clippy::struct_excessive_bools)] // forme fil `KeyboardEventInit`
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyMods {
    /// Ctrl.
    #[serde(default)]
    pub ctrl: bool,
    /// Shift.
    #[serde(default)]
    pub shift: bool,
    /// Alt.
    #[serde(default)]
    pub alt: bool,
    /// Meta.
    #[serde(default)]
    pub meta: bool,
}

/// Checks d'actionabilité exigibles sur une action par `ref`
/// (ADR-0035 §6). `stable` = rect identique sur deux commits
/// layout consécutifs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Check {
    /// Style calculé + boîte posée non vide, dans le clip de
    /// son ancêtre scrollable.
    Visible,
    /// Deux commits layout consécutifs rendent le même rect.
    Stable,
    /// Ni `disabled`, ni ancêtre `inert`/fieldset bloquant.
    Enabled,
    /// Hit-test au centre de la boîte retombe sur l'élément.
    Topmost,
}

/// Options d'auto-wait communes aux actions par `ref`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ActionOpts {
    /// Checks exigés — défaut `[visible, stable, enabled, topmost]`
    /// côté serveur.
    #[serde(default)]
    pub require: Option<Vec<Check>>,
    /// Budget d'attente avant [`ErrorCode::NotActionable`] —
    /// défaut côté config serveur.
    #[serde(default)]
    pub timeout_ms: Option<u32>,
    /// `true` = agir au dernier rect connu si `stable` jamais
    /// atteint — résultat marqué `degraded` (risque page à JS
    /// continu, ADR-0035 « risque nommé »).
    #[serde(default)]
    pub force: bool,
}

/// Rapport d'actionabilité retourné par une action — constat
/// **au moment de l'action** (diagnostic, pas engagement).
#[allow(clippy::struct_excessive_bools)] // un booléen par check, forme du contrat
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct Actionability {
    /// Cf. [`Check`].
    pub visible: bool,
    /// Cf. [`Check`].
    pub stable: bool,
    /// Cf. [`Check`].
    pub enabled: bool,
    /// Cf. [`Check`].
    pub topmost: bool,
}

/// Rôle d'un nœud de snapshot — sous-ensemble ARIA/tag v1.
/// `Other` conserve la chaîne (rôles moins fréquents : `tab`,
/// `option`, `row`…).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// `<a href>`.
    Link,
    /// `<button>` / `input[type=button|submit|reset]`.
    Button,
    /// `input[type=text|search|…]` / `textarea` / `[contenteditable]`.
    Textbox,
    /// `input[type=checkbox]`.
    Checkbox,
    /// `input[type=radio]`.
    Radio,
    /// `h1`–`h6` (niveau dans `name` préfixé `h<n> ` si absent
    /// d'`aria-level` — choix de forme, voir schéma).
    Heading,
    /// `<img>` / `<picture>`.
    Img,
    /// `<nav>`.
    Navigation,
    /// `<form>`.
    Form,
    /// `<select>` / `datalist`.
    Combobox,
    /// `<li>`.
    ListItem,
    /// `<table>`.
    Table,
    /// `main`/landmark principal.
    Main,
    /// Conteneur sans rôle propre (`div`, `span`…).
    Generic,
    /// Rôle hors sous-ensemble — chaîne conservée
    /// (`aria role` ou dérivation non mappée).
    #[serde(untagged)]
    Other(String),
}

/// États exposés par nœud de snapshot (E3) — v1 E1 : `focus`,
/// `editable`, `link` seulement fiables ; le reste arrive avec le
/// mapping sémantique.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeState {
    /// `document.activeElement` === ce nœud.
    Focus,
    /// `disabled` présent ou hérité.
    Disabled,
    /// `checked` (checkbox/radio).
    Checked,
    /// `aria-expanded=true` / `<details open>`.
    Expanded,
    /// Champ saisissable (`input`/`textarea`/`contenteditable`).
    Editable,
    /// Émet une navigation au clic (`<a href>`).
    Link,
    /// `selected` (option/tab).
    Selected,
    /// `required` (contrôle de form).
    Required,
}

/// Nœud de snapshot — liste aplatie, `depth` porte l'arborécence
/// (pré-ordre document). La forme plate évite la récursion dans le
/// JSON et compresse mieux côté tokens LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotNode {
    /// Handle d'action.
    #[serde(rename = "ref")]
    pub elem_ref: ElementRef,
    /// Rôle.
    pub role: Role,
    /// Nom accessible (borné — 120 chars côté serveur).
    pub name: String,
    /// États constatés.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub states: Vec<NodeState>,
    /// Rect viewport — absent si non posé (display:none…).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rect: Option<Rect>,
    /// Profondeur dans l'arbre (0 = document).
    pub depth: u16,
}

// ------------------------------------------------------------------
// Méthodes — `#[serde(tag="method", content="params")]`
// ------------------------------------------------------------------

/// Verbes EAP v1. Chaque variante porte son `x-phase` de
/// livraison en docstring (E1 honnête, cf. capacités).
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "method", content = "params")]
pub enum Method {
    /// Handshake — **première requête obligatoire** de la
    /// connexion : négocie protocole, capacités et profil.
    #[serde(rename = "session.open")]
    SessionOpen(SessionOpenParams),
    /// Ferme la session agent (les onglets survivent — ils
    /// appartiennent à la session moteur).
    #[serde(rename = "session.close")]
    SessionClose(EmptyParams),
    /// Liveness.
    #[serde(rename = "session.ping")]
    SessionPing(EmptyParams),

    /// Ouvre un onglet → `{"tab": n}`.
    #[serde(rename = "tab.open")]
    TabOpen(EmptyParams),
    /// Ferme l'onglet (refs de la table purgées).
    #[serde(rename = "tab.close")]
    TabClose(TabParams),
    /// Liste les onglets vivants.
    #[serde(rename = "tab.list")]
    TabList(EmptyParams),

    /// Navigation — répond au commit `nav.committed` (ou
    /// [`ErrorCode::Timeout`] / `wait` plus lâche demandé).
    #[serde(rename = "nav.goto")]
    NavGoto(NavParams),
    /// Re-fetch.
    #[serde(rename = "nav.reload")]
    NavReload(WaitParams),
    /// Historique −1.
    #[serde(rename = "nav.back")]
    NavBack(WaitParams),
    /// Historique +1.
    #[serde(rename = "nav.forward")]
    NavForward(WaitParams),
    /// Règle le viewport (px CSS).
    #[serde(rename = "nav.viewport")]
    NavViewport(ViewportParams),

    /// Snapshot courant ou diff `{since}` (cap `snapshot[_diff]`).
    #[serde(rename = "snapshot.get")]
    SnapshotGet(SnapshotParams),
    /// Sélecteur CSS → refs (sous-ensemble v1 — `BadSelector`
    /// hors grammaire).
    #[serde(rename = "query.select")]
    QuerySelect(QueryParams),
    /// PNG — la réponse est une **trame binaire** corrélée à
    /// `id`, pas un `result` JSON (cap `frames`).
    #[serde(rename = "frame.get")]
    FrameGet(FrameParams),
    /// Ring console de l'onglet (cap `console`).
    #[serde(rename = "console.get")]
    ConsoleGet(ConsoleParams),
    /// DOM sérialisé — échappatoire debug (cap `dom`).
    #[serde(rename = "dom.get")]
    DomGet(TabParams),

    /// Clic par handle (cap `handles`+`input`, droit `input`).
    #[serde(rename = "input.click")]
    InputClick(ClickParams),
    /// Frappe de texte dans un champ (id.).
    #[serde(rename = "input.type")]
    InputType(TypeParams),
    /// Survol par handle.
    #[serde(rename = "input.hover")]
    InputHover(RefParams),
    /// Frappe clavier au focus courant.
    #[serde(rename = "input.key")]
    InputKey(KeyParams),
    /// Scroll — `ref` (`scroll_into_view`) ou delta.
    #[serde(rename = "input.scroll")]
    InputScroll(ScrollParams),
    /// Clic en coordonnées viewport — échappatoire E1, réponse
    /// toujours `degraded` (pas de handle, pas de check).
    #[serde(rename = "input.click_at")]
    InputClickAt(ClickAtParams),

    /// Évaluation JS dans le contexte top-level (cap+droit `eval`).
    #[serde(rename = "eval.run")]
    EvalRun(EvalParams),

    /// Attente sur commits internes (cap `wait_commit`).
    #[serde(rename = "wait.for")]
    WaitFor(WaitForParams),

    /// Souscription aux notifications (cap `events`).
    #[serde(rename = "events.subscribe")]
    EventsSubscribe(SubscribeParams),
    /// Fin de souscription — `tab` absent : toute la session.
    #[serde(rename = "events.unsubscribe")]
    EventsUnsubscribe(UnsubscribeParams),
}

/// Params `{}`.
pub type EmptyParams = Option<Value>;

/// `{tab}` commun.
#[derive(Debug, Clone, Deserialize)]
pub struct TabParams {
    /// Onglet cible.
    pub tab: TabId,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SessionOpenParams {
    /// Version demandée — `"eap/1"` (la négociation de socket
    /// couvre déjà `Sec-WebSocket-Protocol`, ce champ verrouille
    /// le contrat de messages).
    pub protocol: Option<String>,
    /// Capacités voulues — absent = tout ce que le serveur livre.
    #[serde(default)]
    pub capabilities: Option<Vec<Capability>>,
    /// Profil de droits demandé — le serveur accorde au plus ce
    /// qui est demandé, jamais plus.
    #[serde(default)]
    pub profile: Option<Profile>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NavParams {
    /// Onglet.
    pub tab: TabId,
    /// URL demandée (schémas `http`/`https`/`about:` v1).
    pub url: String,
    /// Condition de retour — défaut `commit`.
    #[serde(default)]
    pub wait: Option<WaitKind>,
    /// Borne d'attente du commit demandé.
    #[serde(default)]
    pub timeout_ms: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WaitParams {
    /// Onglet.
    pub tab: TabId,
    /// Condition de retour.
    #[serde(default)]
    pub wait: Option<WaitKind>,
    /// Borne.
    #[serde(default)]
    pub timeout_ms: Option<u32>,
}

/// Conditions de retour de navigation / attente.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WaitKind {
    /// Navigation commise (titre connu).
    Commit,
    /// Commit + sous-ressources critiques drainées.
    Idle,
    /// Première frame peinte post-commit.
    Frame,
    /// Élément `selector` présent ET visible (`wait.for` seul).
    Selector,
    /// Deux commits layout sans delta de rects (`wait.for` seul).
    Stable,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ViewportParams {
    /// Onglet.
    pub tab: TabId,
    /// Largeur px CSS.
    pub w: u32,
    /// Hauteur px CSS.
    pub h: u32,
    /// Scale DPI optionnel.
    #[serde(default)]
    pub scale: Option<f32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SnapshotParams {
    /// Onglet.
    pub tab: TabId,
    /// Révision de base — présent : réponse diff, absent :
    /// snapshot complet.
    #[serde(default)]
    pub since: Option<u64>,
    /// `true` = nœuds interactifs seulement (liens, contrôles,
    /// éditables) — le mode par défaut des agents LLM.
    #[serde(default)]
    pub interactive_only: Option<bool>,
    /// Borne de nœuds — défaut config serveur.
    #[serde(default)]
    pub max_nodes: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct QueryParams {
    /// Onglet.
    pub tab: TabId,
    /// Sélecteur CSS (sous-ensemble v1 : type, `.cls`, `#id`,
    /// `[attr]`, descendant, enfant — voir schéma).
    pub selector: String,
    /// Borne de résultats.
    #[serde(default)]
    pub max: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FrameParams {
    /// Onglet.
    pub tab: TabId,
    /// Recadrage viewport — absent : frame entière.
    #[serde(default)]
    pub clip: Option<Rect>,
    /// `true` = document entier (au cap `FrameTooBig` du moteur).
    #[serde(default)]
    pub full: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ConsoleParams {
    /// Onglet.
    pub tab: TabId,
    /// Dernières entrées — défaut : tout le ring (512).
    #[serde(default)]
    pub limit: Option<u32>,
}

/// Action par handle avec auto-wait.
#[derive(Debug, Clone, Deserialize)]
pub struct ClickParams {
    /// Onglet.
    pub tab: TabId,
    /// Handle cible.
    #[serde(rename = "ref")]
    pub elem_ref: ElementRef,
    /// Bouton — v1 : `left` uniquement (`right`/`middle` →
    /// `unsupported`).
    #[serde(default)]
    pub button: Option<String>,
    /// Modifieurs tenus.
    #[serde(default)]
    pub mods: Option<KeyMods>,
    /// Auto-wait d'actionabilité.
    #[serde(default)]
    pub opts: Option<ActionOpts>,
}

/// `ref` seul + opts.
#[derive(Debug, Clone, Deserialize)]
pub struct RefParams {
    /// Onglet.
    pub tab: TabId,
    /// Handle cible.
    #[serde(rename = "ref")]
    pub elem_ref: ElementRef,
    /// Auto-wait.
    #[serde(default)]
    pub opts: Option<ActionOpts>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TypeParams {
    /// Onglet.
    pub tab: TabId,
    /// Handle du champ — le serveur vérifie `Editable`.
    #[serde(rename = "ref")]
    pub elem_ref: ElementRef,
    /// Texte à frapper (dispatch complet keydown/input/keyup
    /// par caractère — pas d'injection de valeur).
    pub text: String,
    /// `true` = vider le champ avant frappe.
    #[serde(default)]
    pub clear: bool,
    /// Auto-wait.
    #[serde(default)]
    pub opts: Option<ActionOpts>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct KeyParams {
    /// Onglet.
    pub tab: TabId,
    /// Identifiant de touche (`"Enter"`, `"a"`, `"Control+a"`…).
    pub key: String,
    /// Modifieurs.
    #[serde(default)]
    pub mods: Option<KeyMods>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ScrollParams {
    /// Onglet.
    pub tab: TabId,
    /// Amener la ref dans le viewport (prioritaire sur `dy`).
    #[serde(rename = "ref", default)]
    pub elem_ref: Option<ElementRef>,
    /// Delta vertical px CSS (défaut 0).
    #[serde(default)]
    pub dy: Option<f32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ClickAtParams {
    /// Onglet.
    pub tab: TabId,
    /// Position viewport (px CSS).
    pub x: f32,
    /// Position viewport.
    pub y: f32,
    /// Modifieurs.
    #[serde(default)]
    pub mods: Option<KeyMods>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct EvalParams {
    /// Onglet.
    pub tab: TabId,
    /// Source JS — évaluée dans le contexte top-level ; la valeur
    /// retourne sérialisée JSON (bornée 64 Ko, tronquée au-delà).
    pub expr: String,
    /// Borne d'exécution — défaut config.
    #[serde(default)]
    pub timeout_ms: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WaitForParams {
    /// Onglet.
    pub tab: TabId,
    /// Condition.
    pub kind: WaitKind,
    /// Sélecteur — requis pour `kind=selector`.
    #[serde(default)]
    pub selector: Option<String>,
    /// Borne — [`ErrorCode::Timeout`] au-delà.
    pub timeout_ms: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SubscribeParams {
    /// Onglet — absent : tous les onglets de la session.
    #[serde(default)]
    pub tab: Option<TabId>,
    /// Familles d'événements.
    pub kinds: Vec<EventKind>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UnsubscribeParams {
    /// Onglet — absent : désabonnement global.
    #[serde(default)]
    pub tab: Option<TabId>,
}

/// Familles de notifications souscriptibles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    /// `nav.started` / `nav.committed`.
    Nav,
    /// `frame.invalidated` (rects + scroll — sans pixels).
    Frame,
    /// `console.entry`.
    ConsoleLog,
    /// `snapshot.changed` — indice d'invalidation bon marché :
    /// la prochaine `snapshot.get{since}` livrera un diff.
    Snapshot,
    /// `tab.opened` / `tab.closed`.
    Tabs,
    /// `download.*` (journal téléchargements).
    Download,
}

// ------------------------------------------------------------------
// Notifications serveur — `{"method":"event.<kind>", "params":{…}}`
// ------------------------------------------------------------------

/// Notification poussée — sérialisée `{"method":"event.<name>",
/// "params":{…}}` : le tag interne produit exactement la forme
/// golden (`method` + `params` plats).
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "method", content = "params")]
pub enum Event {
    /// Fetch engagé.
    #[serde(rename = "event.nav.started")]
    NavStarted {
        /// Onglet.
        tab: TabId,
        /// URL demandée.
        url: String,
    },
    /// Navigation commise — le document est posé.
    #[serde(rename = "event.nav.committed")]
    NavCommitted {
        /// Onglet.
        tab: TabId,
        /// URL finale.
        url: String,
        /// Titre.
        title: Option<String>,
        /// Nouvelle révision de document (base des diffs).
        rev: u64,
    },
    /// Frame périmée — l'agent retirera `frame.get` s'il en a
    /// besoin (pull, jamais de pixels poussés).
    #[serde(rename = "event.frame.invalidated")]
    FrameInvalidated {
        /// Onglet.
        tab: TabId,
        /// Régions invalides (px physiques).
        rects: Vec<Rect>,
        /// Scroll de la frame peinte.
        scroll_y: f32,
    },
    /// Entrée de console (ring).
    #[serde(rename = "event.console.entry")]
    ConsoleEntry {
        /// Onglet.
        tab: TabId,
        /// Message.
        message: String,
    },
    /// Révision de snapshot incrémentée.
    #[serde(rename = "event.snapshot.changed")]
    SnapshotChanged {
        /// Onglet.
        tab: TabId,
        /// Nouvelle révision.
        rev: u64,
    },
    /// Onglet ouvert.
    #[serde(rename = "event.tab.opened")]
    TabOpened {
        /// Onglet.
        tab: TabId,
    },
    /// Onglet fermé — refs purgées, `StaleRef` dorénavant.
    #[serde(rename = "event.tab.closed")]
    TabClosed {
        /// Onglet.
        tab: TabId,
    },
}

/// Forme de la notification sur le fil :
/// `{"jsonrpc":"2.0","method":"event.…","params":{…}}`.
#[derive(Debug, Serialize)]
pub struct Notification {
    /// `"2.0"`.
    pub jsonrpc: Jsonrpc,
    /// Tag `method` + `params` de l'événement (plat).
    #[serde(flatten)]
    pub event: Event,
}

impl Notification {
    /// Construit la notification sérialisable d'un événement.
    #[must_use]
    pub fn new(event: Event) -> Self {
        Self {
            jsonrpc: Jsonrpc,
            event,
        }
    }
}

// ------------------------------------------------------------------
// Trame binaire — `[u32 LE id][u32 LE kind][payload]`
// ------------------------------------------------------------------

/// Trames binaires du socket : réponses à charge lourde corrélées
/// à la requête par `id` (pas de `result` JSON).
pub mod binary {
    /// Taille de l'en-tête.
    pub const HEADER: usize = 8;

    /// Type de charge binaire.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    #[repr(u32)]
    pub enum Kind {
        /// PNG de `frame.get`.
        PngFrame = 1,
    }

    impl Kind {
        /// Décode le code trame — `None` si inconnu (laisser
        /// tomber une trame inconnue est la dégradation correcte,
        /// jamais d'`unreachable!` sur contenu distant).
        #[must_use]
        pub fn from_u32(v: u32) -> Option<Self> {
            match v {
                1 => Some(Self::PngFrame),
                _ => None,
            }
        }
    }

    /// Encode `id|kind|payload` — l'appelant fournit le buffer.
    pub fn encode(id: u32, kind: Kind, payload: &[u8], out: &mut Vec<u8>) {
        out.reserve(HEADER + payload.len());
        out.extend_from_slice(&id.to_le_bytes());
        out.extend_from_slice(&(kind as u32).to_le_bytes());
        out.extend_from_slice(payload);
    }

    /// Découpe l'en-tête — `None` si trame courte ou kind inconnu.
    #[must_use]
    pub fn decode(frame: &[u8]) -> Option<(u32, Kind, &[u8])> {
        let (hdr, payload) = frame.split_at_checked(HEADER)?;
        let id = u32::from_le_bytes(hdr[..4].try_into().ok()?);
        let kind = Kind::from_u32(u32::from_le_bytes(hdr[4..].try_into().ok()?))?;
        Some((id, kind, payload))
    }
}
