// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Tree-walk interpreter: environments, property lookup, coercions,
//! calls, `new`, exceptions — the whole dynamic semantics of the subset.
//!
//! Safety rails, browser-first: `op_limit` bounds total work (a
//! `while(1);` must not freeze the UI thread), `call_depth` bounds
//! recursion, `expr_depth` bounds nested expressions.

use crate::ast::{
    ClassLit, ClassMemberKind, ExportDecl, Expr, FnLit, ForInit, ForTarget, Param, Pat, PropKey,
    Stmt,
};
use crate::error::{Error, Result};
use crate::heap::{
    Binding, Env, EnvCell, EnvKind, EnvRef, Fun, Heap, JsResult, ObjKind, Object, Prop, TaKind,
    Thrown,
};
use crate::parser;
use crate::value::{num_to_js_string, to_uint32, ObjId, Value};
use std::collections::HashSet;
use std::rc::Rc;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::SyncSender;
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

// Plafonds logiques anti-contenu hostile (JS-C41) — la pile
// elle-même est protégée par `stacker::maybe_grow` (segment tas
// quand la pile native s'épuise) : ces bornes décident du
// `RangeError` spec, pas de la survie du processus. Un appel JS
// mesure ~27–55 Kio natif ; 256 ≈ ~14 Mio de segments max.
const DEFAULT_CALL_DEPTH: u32 = 256;
const MAX_PROTO_DEPTH: u32 = 512;
const DEFAULT_OP_LIMIT: u64 = 10_000_000;
/// Seuil GC par défaut (JS-C37) — ~130 K cellules vivantes
/// déclenchent une passe à la frontière de job suivante.
const DEFAULT_GC_THRESHOLD: usize = 131_072;
/// Frames rendues dans `error.stack`/`Uncaught` (JS-C47) —
/// même borne que V8 (`Error.stackTraceLimit` = 10) : les
/// frames utiles au diagnostic sont en tête de pile.
const STACK_TRACE_LIMIT: usize = 10;

/// Hidden env bindings installed in class member scopes — the names
/// use private-use code points so no source identifier can collide
/// (`super.x` / `super()` resolve through these).
pub(crate) const HOME_BIND: &str = "\u{f120}home";
/// Parent constructor value for `super(...)`.
pub(crate) const SUPER_BIND: &str = "\u{f120}super";
/// Liaison cachée `new.target` dans l'env local d'une
/// fonction invoquée par `new`/`Reflect.construct`.
pub(crate) const NEW_TARGET: &str = "\u{f120}new.target";

/// Liaison cachée du spec résolu du module dans son env —
/// `import.meta.url` et le référent de `import()` se lisent
/// lexicalement (fonctions/callbacks imbriqués), pas dans
/// `current_module` global déjà restauré à l'évaluation
/// différée.
pub(crate) const IMPORT_META: &str = "\u{f120}import.meta";

/// Préfixe des clés privées `\u{f131}privN.name` — produit par le
/// parser (résolution lexicale des `#x`), jamais énumérable.
pub(crate) const PRIV_PREFIX: &str = "\u{f131}priv";
/// Liste des champs d'instance d'une classe de base —
/// `[[clé, thunk], …]` installée sur `this` à l'entrée du corps.
const FLD_FIELDS: &str = "\u{f131}fields";
/// Idem pour une classe dérivée : installée après le `super()`.
const FLD_FIELDS_D: &str = "\u{f131}fields.derived";
/// Marqueur posé sur `this` après installation des champs dérivés
/// — un second `super()` doit lever `TypeError` (spec).
pub(crate) const FLD_DONE: &str = "\u{f131}fields.done";
/// Marqueur « constructeur de classe » — `C()` sans `new` lève
/// `TypeError` (spec `[[IsClassConstructor]]` implicite).
const CLASS_CTOR: &str = "\u{f131}ctor";

/// `Symbol.iterator` property key — a private-use sentinel string
/// (symbols are represented as opaque string keys; `typeof sym`
/// yields `"string"`, a documented deviation).
/// Préfixe sentinelle des clés-symboles `\u{f130}Symbol.*` —
/// jamais énumérable (`for-in`, `Object.keys`, `JSON`).
pub(crate) const SYM_PREFIX: &str = "\u{f130}Symbol.";

pub(crate) const SYM_ITERATOR: &str = "\u{f130}Symbol.iterator";

/// `Symbol.asyncIterator` property key — same private-use sentinel
/// trick as `SYM_ITERATOR` (invisible to `for-in`/`Object.keys`).
pub(crate) const SYM_ASYNC_ITERATOR: &str = "\u{f130}Symbol.asyncIterator";

/// `Symbol.toPrimitive` — `@@toPrimitive(hint)` appelé avant
/// `valueOf`/`toString` dans `ToPrimitive`.
pub(crate) const SYM_TO_PRIMITIVE: &str = "\u{f130}Symbol.toPrimitive";

/// `Symbol.hasInstance` — `@@hasInstance` appelé par `instanceof`.
pub(crate) const SYM_HAS_INSTANCE: &str = "\u{f130}Symbol.hasInstance";

/// `Symbol.toStringTag` — tag personnalisé d'`Object::toString`.
pub(crate) const SYM_TO_STRING_TAG: &str = "\u{f130}Symbol.toStringTag";

/// `Symbol.unscopables` — noms exclus de la liaison `with`.
pub(crate) const SYM_UNSCOPABLES: &str = "\u{f130}Symbol.unscopables";

/// `Symbol.dispose` — protocole `using`/`DisposableStack` (ES2026).
#[allow(dead_code)] // protocole `using` en cours d'implémentation
pub(crate) const SYM_DISPOSE: &str = "\u{f130}Symbol.dispose";

/// `Symbol.asyncDispose` — variante async (`await using`).
#[allow(dead_code)] // protocole `using` en cours d'implémentation
pub(crate) const SYM_ASYNC_DISPOSE: &str = "\u{f130}Symbol.asyncDispose";

// Clés de dispatch @@symbols — consommées par le chantier
// regexp/symbol en cours (équipe JS) ; `allow` le temps que
// les dispatches soient branchés.
#[allow(dead_code)]
/// `Symbol.match` — dispatch `str.match(rx)` → `rx[@@match]`.
pub(crate) const SYM_MATCH: &str = "\u{f130}Symbol.match";
/// `Symbol.matchAll` — dispatch `str.matchAll(rx)` → `rx[@@matchAll]`.
#[allow(dead_code)]
pub(crate) const SYM_MATCH_ALL: &str = "\u{f130}Symbol.matchAll";
/// `Symbol.replace` — dispatch `str.replace(rx, r)` → `rx[@@replace]`.
#[allow(dead_code)]
pub(crate) const SYM_REPLACE: &str = "\u{f130}Symbol.replace";
/// `Symbol.search` — dispatch `str.search(rx)` → `rx[@@search]`.
#[allow(dead_code)]
pub(crate) const SYM_SEARCH: &str = "\u{f130}Symbol.search";
/// `Symbol.split` — dispatch `str.split(rx, lim)` → `rx[@@split]`.
#[allow(dead_code)]
pub(crate) const SYM_SPLIT: &str = "\u{f130}Symbol.split";
/// `Symbol.species` — ctor de résultat (`ArraySpeciesCreate`…).
#[allow(dead_code)]
pub(crate) const SYM_SPECIES: &str = "\u{f130}Symbol.species";
/// `Symbol.isConcatSpreadable` — override du spread de `concat`.
#[allow(dead_code)]
pub(crate) const SYM_IS_CONCAT_SPREADABLE: &str = "\u{f130}Symbol.isConcatSpreadable";

/// `ToPrimitive` preferred-type hint (spec `ToPrimitive`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PrimHint {
    /// `+`, `==`, `Date` conversions.
    Default,
    /// `ToNumber`, arithmetic, comparisons.
    Number,
    /// `ToString`, property keys.
    String,
}

/// How a destructuring pattern binds its targets.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum BindMode {
    /// `var` / parameter — function-scoped, declared if absent.
    Var,
    /// `let`/`const` — current-scope declarative binding.
    Lex {
        /// `false` for `const`.
        mutable: bool,
    },
    /// Destructuring *assignment* (`[a] = x` — existing bindings).
    Assign,
}

/// Référence d'affectation réifiée — produit de
/// [`Engine::resolve_lvalue`]. Capturer `base`/`key` une seule fois
/// respecte l'ordre spec (LHS avant RHS) et évite de réévaluer les
/// expressions cibles dans `op=` (`a[i()] += x` appelle `i()` une
/// seule fois).
pub(crate) enum LValueRef {
    /// Liaison de variable — l'`EnvRef` vit côté appelant.
    Ident(String),
    /// Propriété `base[key]` — `base` déjà évaluée.
    Prop(Value, String),
}

/// Hook d'observabilité des sources évaluées dynamiquement
/// (`eval`, `Function`, timer-string) — voir
/// [`Engine::set_eval_hook`] (JS-C50).
pub type EvalHook = Rc<dyn Fn(&str)>;

/// The script engine. One `Engine` per browsing context (page):
/// the heap, global object and scope chain all die together.
pub struct Engine {
    pub(crate) heap: Heap,
    pub(crate) global: ObjId,
    pub(crate) global_env: EnvRef,
    pub(crate) protos: Protos,
    pub(crate) rng: u64,
    /// Engine creation time — `performance.now()` baseline.
    pub(crate) t0: Instant,
    /// `Symbol.for`/`Symbol.keyFor` global registry (key → symbol).
    /// Compteur des clés-symboles anonymes (`\u{f130}sym{N}`).
    pub(crate) sym_seq: usize,
    /// Index clé → `ObjId` symbole (reverse-lookup pour
    /// `Object.getOwnPropertySymbols`) — tracé : les symboles
    /// restent vivants tant que l'index les référence.
    pub(crate) sym_index: std::collections::BTreeMap<String, ObjId>,
    /// Registres `FinalizationRegistry` vivants (liste faible —
    /// les entrées mortes sont purgées à chaque collecte).
    pub(crate) finregs: Vec<ObjId>,
    /// `%GeneratorFunction%`/`%AsyncGeneratorFunction%` — ctors
    /// cachés (pas de binding global, spec) posés en own
    /// `constructor` des fonctions `function*`/`async function*`
    /// pour `(f*).constructor.name` = `"GeneratorFunction"`.
    pub(crate) gen_ctor: Value,
    /// Voir `gen_ctor`.
    pub(crate) async_gen_ctor: Value,
    /// `%GeneratorFunction.prototype%` — proto des fonctions
    /// `function*` (own `.constructor` = `gen_ctor`,
    /// own `.prototype` = `gen_proto`, spec).
    pub(crate) gen_fn_proto: Value,
    /// `%Generator%` (= `%GeneratorFunction.prototype.prototype%`)
    /// — proto de `g.prototype` pour `function* g` ; son propre
    /// proto est `%IteratorPrototype%` (`protos.iterator`).
    pub(crate) gen_proto: Value,
    /// Voir `gen_fn_proto` — variante `async function*`.
    pub(crate) async_gen_fn_proto: Value,
    /// Voir `gen_proto` — variante `async function*`.
    pub(crate) async_gen_proto: Value,
    pub(crate) symbol_reg: std::collections::BTreeMap<String, Value>,
    call_depth: u32,
    call_depth_limit: u32,
    /// Pile d'appels des fonctions **script** en cours (JS-C47) —
    /// un `Rc<FnLit>` poussé à l'entrée, dépilé à la sortie ;
    /// rendue dans `error.stack` et `Error::Uncaught`. Les
    /// natives n'y figurent pas (déviation documentée).
    call_stack: Vec<Rc<crate::ast::FnLit>>,
    /// Hook d'observabilité des sources `eval`/`Function`/
    /// timer-string (JS-C50) — le code obfusqué des pages réelles
    /// vit dans ces sources, pas dans le `<script>` parent ; le
    /// hook les voit avant parse (diagnostic, jamais modifié).
    pub(crate) eval_hook: Option<EvalHook>,
    expr_depth: u32,
    /// Borne d'emboîtement du parseur et de l'évaluateur
    /// d'expressions (JS-C41) — relevable par l'embeddeur sur un
    /// thread moteur à pile large.
    pub(crate) nest_limit: u32,
    pub(crate) ops_left: u64,
    pub(crate) op_limit: u64,
    /// Seuil de GC automatique en cellules vivantes (JS-C37) —
    /// `0` = jamais. La passe est déclenchée aux frontières de
    /// jobs (`run_microtasks`), jamais pendant une évaluation
    /// (la pile d'appel n'est pas une racine).
    gc_threshold: usize,
    /// Total cumulé des `tick` — compteur d'ops exposé par
    /// [`Engine::heap_stats`]/[`Engine::ops_used`] (métriques).
    ops_total: u64,
    /// Host-side timer queue (JS-3) — pumped by the embedder's event
    /// loop via [`Engine::run_due_timers`].
    pub(crate) timers: crate::timers::TimerQueue,
    /// XHR bookkeeping (JS-4) — transport injected by the embedder.
    pub(crate) xhr: crate::xhr::XhrEngineState,
    /// Installed DOM + node cache (set by `install_dom`) — the
    /// platform-event entry point [`Engine::dom_event`] needs both
    /// to dispatch at document level (JS-5).
    pub(crate) dom_ctx: Option<(crate::dom_host::DomRef, crate::dom_host::NodeCache)>,
    /// Microtask queue (JS-7c) — `then` reactions, thenable calls,
    /// `queueMicrotask`. Drained by [`Engine::run_microtasks`] at
    /// every job boundary (end of `eval`, timer cb, event, XHR).
    /// File de microtasks — chaque job porte le contexte de son
    /// enregistrement (JS-C78) : `then`/`queueMicrotask` d'un
    /// enfant courent dans SON realm (spec « job callback realm »),
    /// switchés à l'exécution par `run_microtasks`.
    pub(crate) microtasks: std::collections::VecDeque<(crate::context::CtxId, crate::promise::Job)>,
    /// `MutationObserver` bookkeeping (JS-C26) — callbacks par
    /// id d'observer ; les records vivent dans la file du `Dom`.
    pub(crate) mutobs: crate::mutation::MutObsState,
    /// `IntersectionObserver`/`ResizeObserver` bookkeeping
    /// (JS-C69) — registrations + notifications initiales
    /// pendantes, livrées au checkpoint microtask.
    pub(crate) obs: crate::observers::ObsState,
    /// `Blob`/`File`/`FileReader` + URL objets bookkeeping
    /// (JS-C70) — protos paresseux, `blob:` urls, lectures
    /// en cours livrées au checkpoint microtask.
    pub(crate) files: crate::files::FilesState,
    /// `ReadableStream`/`WritableStream`/`TransformStream`
    /// (JS-C71) — protos + `read()` pendantes re-`pull`ées au
    /// checkpoint microtask.
    pub(crate) streams: crate::streams::StreamState,
    /// `indexedDB` + `caches` bookkeeping (JS-C72) — registres
    /// partagés injectables + file de `IDBRequest` livrées au
    /// checkpoint microtask.
    pub(crate) idb: crate::idb::IdbState,
    /// `WebSocket` bookkeeping (JS-C73) — proto + sockets en
    /// `CONNECTING` dont le refus de handshake est livré au
    /// checkpoint microtask (`error` + `close` 1006).
    pub(crate) ws: crate::websocket::WsState,
    /// `EventSource` bookkeeping (JS-C92) — proto, flux
    /// vivants `(handle, wrapper)` racines GC tant que non
    /// CLOSED, refus différés sans transport.
    pub(crate) es: crate::eventsource::EsState,
    /// `Notification`/`BroadcastChannel`/`Selection`/
    /// `reportError` bookkeeping (JS-C74) — protos, registre
    /// de canaux et messages livrés au checkpoint microtask.
    pub(crate) misc: crate::misc_apis::MiscState,
    /// `crypto.subtle` bookkeeping (JS-C79) — opérations en
    /// vol du contexte courant (handle → promesse).
    pub(crate) crypto: crate::crypto::CryptoState,
    /// Élément en cours de construction par upgrade
    /// `customElements` (JS-C68) — le `super()` d'une classe
    /// `extends HTMLElement` le récupère comme `this` (rôle du
    /// slot spec « element being upgraded »). `None` hors
    /// upgrade → `new HTMLElement()` jette `Illegal constructor`.
    pub(crate) ce_upgrade_slot: Option<Value>,
    /// Browser-surface hooks (JS-7e) — page URL, cookie jar,
    /// navigation/history callbacks injected by the embedder.
    pub(crate) hooks: crate::dom_host::BrowserHooks,
    /// Last non-empty statement completion (`eval`-style result of
    /// `if (1) 'yes'` must be `"yes"`, as real engines produce).
    completion: Value,
    /// Contexte strict courant — posé par `exec_program`
    /// (`Program::strict`) et par chaque appel `Fun::Script`
    /// (`FnLit::strict`). Pilote `TypeError`/`ReferenceError` vs
    /// échecs silencieux sloppy.
    pub(crate) strict: bool,
    /// Modules ES instanciés (JS-C5) — index = l'identité stable
    /// utilisée par les getters de namespace et le graphe.
    pub(crate) modules: Vec<crate::module::Module>,
    /// Cache `specifier → module index` — deux `import` du même
    /// specifier résolvent vers la même instance (et coupent les
    /// cycles à l'instanciation).
    pub(crate) module_specs: std::collections::HashMap<String, usize>,
    /// Registre des envs référencés par les getters de namespace —
    /// les natives bornées n'emportent que des `Value` (l'index
    /// suffit, l'env vit tant que le module vit).
    pub(crate) module_envs: Vec<EnvRef>,
    /// Chargeur de sources de modules injecté par l'hôte
    /// (fetch/cache/résolution `file:`/URL).
    pub(crate) module_loader: Option<Rc<std::cell::RefCell<dyn crate::module::ModuleLoader>>>,
    /// Specifier du module en cours d'exécution (`import.meta.url`
    /// et referrer de `import()`).
    pub(crate) current_module: String,
    /// Import map minimale (JS-C44) — paires `(specifier, target)`
    /// issues des clés `imports`, en ordre d'enregistrement (le
    /// premier gagnant garde sa clé en cas de conflit entre maps,
    /// spec « merge »). Consultée par `load_source` pour les
    /// spécificateurs bare avant d'appeler le `ModuleLoader`.
    pub(crate) import_map: Vec<(String, String)>,
    /// Slots des workers spawnés (JS-C29) — index stables, le
    /// `WorkerHost` y retrouve son slot au pump.
    pub(crate) workers: Vec<crate::worker::WorkerSlot>,
    /// Fabrique de loaders worker — chaque worker reçoit la
    /// sienne (elle vit dans son thread).
    pub(crate) worker_factory: Option<crate::worker::WorkerFactory>,
    /// Bornes du pool (`max_workers`, `queue_len`).
    pub(crate) worker_cfg: crate::worker::WorkerConfig,
    /// Côté WORKER : canal de sortie vers le main (`postMessage`).
    pub(crate) worker_out: Option<SyncSender<crate::worker::WorkerEvent>>,
    /// Côté WORKER : loader d'`importScripts`.
    pub(crate) worker_loader: Option<Box<dyn crate::worker::WorkerLoader>>,
    /// Côté WORKER : `close()` appelé dans le script.
    pub(crate) worker_close: bool,
    /// Côté WORKER : flag partagé vivant (le `WorkerHost` du
    /// parent le coupe à `terminate()`/GC).
    pub(crate) worker_alive: Arc<AtomicBool>,
    /// Pile de collecteurs `yield` des générateurs en cours
    /// (générateur eager : le corps tourne à l'appel et chaque
    /// `yield` pousse sur le collecteur du sommet — déviation
    /// documentée, pas de suspension).
    pub(crate) gen_collect: Vec<Vec<Value>>,
    /// Labels de boucle en attente — posés par `Stmt::Labeled`
    /// juste avant l'exécution d'une boucle, consommés par cette
    /// boucle (`a: b: for` empile les deux labels).
    pending_labels: Vec<String>,
    /// `using` disposal frames — une par corps contenant
    /// une décl `using` (ES2026) : `(dispose_fn, resource, is_async)`.
    pub(crate) using_stack: Vec<Vec<(Value, Value, bool)>>,
    /// Le builtin `eval` — identité pour distinguer l'appel direct
    /// `eval(…)` (env appelant) de l'indirect (`(0,eval)`, `e=eval`
    /// → global), spec `PerformEval`.
    pub(crate) eval_fn: Value,
    /// Le ctor `RegExp` — les statiques Annex B (`RegExp.$1…$9`,
    /// `input`/`$_`, `lastMatch`/`$&`, `lastParen`/`$+`, contextes
    /// gauche/droit) s'y lisent.
    pub(crate) re_ctor: Value,
    /// Dernier match d'expression régulière — statiques Annex B.
    pub(crate) re_static: Option<ReStatic>,
    /// Contexte de navigation courant (JS-C78) — `0` = top-level.
    /// Les champs document-local visibles sur `self` (`global`,
    /// `global_env`, `protos`, `dom_ctx`, `timers`, `xhr`, …)
    /// SONT ceux du contexte courant ; les contextes garés
    /// vivent dans [`Self::parked`].
    pub(crate) cur_ctx: crate::context::CtxId,
    /// Arbre des contextes — index = [`crate::context::CtxId`]
    /// (métadonnées non garées : parent/enfants/URL/nœud iframe).
    pub(crate) ctxs: Vec<crate::context::CtxMeta>,
    /// Contextes non courants — champs document-local garés,
    /// rendus à [`Self::switch_to`]. Le contexte courant n'y est
    /// jamais (ses champs sont sur `self`).
    pub(crate) parked:
        std::collections::HashMap<crate::context::CtxId, crate::context::BrowsingContext>,
    /// Messages `postMessage` clonés en attente de livraison —
    /// drainés à la frontière de job de l'émetteur
    /// (`run_microtasks`), chaque livraison switchant vers le
    /// contexte destinataire.
    pub(crate) pending_msgs: crate::context::PendingMsgs,
    /// Messages `MessageChannel` en file (JS-C81) — drainés à
    /// la même frontière de job que `pending_msgs`.
    pub(crate) port_msgs: std::collections::VecDeque<crate::channel::PortMsg>,
    /// Cache des constructeurs d'interface par nom de classe
    /// (`HTMLInputElement`…) — JS-C82 : `el.constructor` typé +
    /// `.prototype` porteur des accesseurs IDL (surface
    /// `Object.getOwnPropertyDescriptor` — `inputValueTracking`
    /// de React l'exige pour tracker `value`/`checked`).
    /// Chaque entrée est rootée à la création (`add_root`).
    pub(crate) dom_ctors: std::collections::HashMap<String, Value>,
    /// Pile des contextes appelants pour les natives cross-ctx
    /// (JS-C78) — `frame.eval()` pousse le realm appelant, le
    /// `postMessage` de l'enfant lit son `source`/`origin` ici.
    pub(crate) native_caller: Vec<crate::context::CtxId>,
    /// Fournisseur crypto injecté par l'embedder (JS-C79) —
    /// **engine-global** : la plateforme est unique pour tous
    /// les realms, chaque contexte en hérite dès `context_new`.
    pub(crate) crypto_transport: Option<Rc<dyn crate::crypto::CryptoTransport>>,
    /// Fournisseur SSE injecté par l'embedder (JS-C92) —
    /// **engine-global** comme `crypto_transport` : tous les
    /// contextes ouvrent leurs flux par la même couture.
    pub(crate) es_transport: Option<Rc<dyn crate::eventsource::EsTransport>>,
}

/// Dernier match `RegExp` — `RegExp.$1…$9`/`input`/`lastMatch`/…
/// (Annex B, spec `UpdateRegExpStatics`).
#[derive(Clone)]
pub(crate) struct ReStatic {
    /// Chaîne sujet du match.
    pub input: String,
    /// Offsets octet du match entier (`groups[0]`).
    pub start_b: usize,
    /// Fin octet du match.
    pub end_b: usize,
    /// `groups[0]` = match entier ; `None` = groupe non participé.
    pub groups: Vec<Option<String>>,
}

/// Prototype objects captured at engine init.
pub(crate) struct Protos {
    pub object: ObjId,
    pub function: ObjId,
    pub array: ObjId,
    pub string: ObjId,
    pub number: ObjId,
    pub boolean: ObjId,
    pub error: ObjId,
    pub type_error: ObjId,
    pub reference_error: ObjId,
    pub syntax_error: ObjId,
    /// `EvalError.prototype` (ES2015+ le réintroduit — dev.to
    /// l'énumère dans sa table d'intrinsèques ; JS-C56 vague 2).
    pub eval_error: ObjId,
    /// `RangeError.prototype` (`new RangeError(msg)`).
    pub range_error: ObjId,
    /// `URIError.prototype` — `error_value` l'émettait déjà pour
    /// `decodeURI` malformé sans que le global existe.
    pub uri_error: ObjId,
    pub date: ObjId,
    pub regexp: ObjId,
    pub promise: ObjId,
    /// Shared `next()`/`[Symbol.iterator]` iterator prototype.
    pub iterator: ObjId,
    /// `%ArrayIteratorPrototype%` — niveau concret entre
    /// l'itérateur retourné et `%IteratorPrototype%` (spec :
    /// `getPrototypeOf(getPrototypeOf([][Symbol.iterator]()))
    /// === Iterator.prototype`). Partagé par `Array` et les
    /// `TypedArray` (`CreateArrayIterator`).
    pub array_iter: ObjId,
    /// `%MapIteratorPrototype%`.
    pub map_iter: ObjId,
    /// `%SetIteratorPrototype%`.
    pub set_iter: ObjId,
    /// `%StringIteratorPrototype%` (`str[Symbol.iterator]()`).
    pub string_iter: ObjId,
    /// `%RegExpStringIteratorPrototype%` (`re[Symbol.matchAll]`).
    pub regexp_iter: ObjId,
    /// `%URLSearchParamsIteratorPrototype%`.
    pub usp_iter: ObjId,
    /// `%IteratorHelperPrototype%` — résultats des helpers
    /// ES2025 (`map`/`filter`/`take`/`drop`/`flatMap`,
    /// `Iterator.concat`, wrap `Iterator.from`).
    pub iter_helper: ObjId,
    pub map: ObjId,
    pub set: ObjId,
    /// `Symbol.prototype` (`toString`/`valueOf`/`description`).
    pub symbol: ObjId,
    /// `Proxy.prototype` (les instances n'ont pas de proto fixe —
    /// utilisé seulement pour détecter l'appel construct-only).
    pub proxy: ObjId,
    /// `ArrayBuffer.prototype`.
    pub array_buffer: ObjId,
    /// `DataView.prototype`.
    pub data_view: ObjId,
    /// `%TypedArray%.prototype` — proto partagé des 9 protos
    /// `XxxArray.prototype` (`set`, `subarray`, `keys`…).
    pub typed_array: ObjId,
    /// `BigInt.prototype` (`toString`/`valueOf`/`@@toPrimitive`).
    pub bigint: ObjId,
    /// `AggregateError.prototype` (proto `Error.prototype`).
    pub aggregate_error: ObjId,
    /// `URL.prototype` — `toString`/`toJSON`.
    pub url: ObjId,
    /// `URLSearchParams.prototype` — `get`/`set`/`has`/`entries`…
    pub url_params: ObjId,
    /// `WeakMap.prototype` (proto `Object.prototype` — spec).
    pub weak_map: ObjId,
    /// `WeakSet.prototype` (proto `Object.prototype` — spec).
    pub weak_set: ObjId,
}

impl Protos {
    /// Tous les prototypes — racines GC permanentes.
    pub(crate) fn all(&self) -> [ObjId; 37] {
        [
            self.object,
            self.function,
            self.array,
            self.string,
            self.number,
            self.boolean,
            self.error,
            self.type_error,
            self.reference_error,
            self.syntax_error,
            self.eval_error,
            self.range_error,
            self.uri_error,
            self.date,
            self.regexp,
            self.promise,
            self.iterator,
            self.map,
            self.set,
            self.symbol,
            self.proxy,
            self.array_buffer,
            self.data_view,
            self.typed_array,
            self.bigint,
            self.aggregate_error,
            self.url,
            self.url_params,
            self.weak_map,
            self.weak_set,
            self.array_iter,
            self.map_iter,
            self.set_iter,
            self.string_iter,
            self.regexp_iter,
            self.usp_iter,
            self.iter_helper,
        ]
    }
}

/// Statement completion — `throw` travels via `Err(Thrown)`.
pub(crate) enum Flow {
    Normal,
    /// `break [label]` — `Some` n'est avalé que par le statement
    /// étiqueté correspondant, jamais par une boucle interne.
    Break(Option<String>),
    /// `continue [label]` — `Some` n'est avalé que par la boucle
    /// portant ce label (capturé dans `my_labels`).
    Continue(Option<String>),
    Return(Value),
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

impl Engine {
    /// New engine with all builtins installed.
    #[must_use]
    pub fn new() -> Self {
        let mut heap = Heap::new();
        let object = heap.alloc_obj(Object::ordinary(Value::Null));
        let function = heap.alloc_obj(Object::ordinary(Value::Obj(object)));
        // `Array.prototype` est elle-même un tableau exotique
        // (spec : `Array.isArray(Array.prototype)` → `true` —
        // donne `Array.prototype.length` gratuitement via la
        // synthèse `find_own`/`length`).
        let array = heap.alloc_obj(Object::array(Vec::new(), Value::Obj(object)));
        let string = heap.alloc_obj(Object::ordinary(Value::Obj(object)));
        let number = heap.alloc_obj(Object::ordinary(Value::Obj(object)));
        let boolean = heap.alloc_obj(Object::ordinary(Value::Obj(object)));
        let error = heap.alloc_obj(Object::ordinary(Value::Obj(object)));
        let type_error = heap.alloc_obj(Object::ordinary(Value::Obj(error)));
        let reference_error = heap.alloc_obj(Object::ordinary(Value::Obj(error)));
        let syntax_error = heap.alloc_obj(Object::ordinary(Value::Obj(error)));
        let eval_error = heap.alloc_obj(Object::ordinary(Value::Obj(error)));
        let range_error = heap.alloc_obj(Object::ordinary(Value::Obj(error)));
        let uri_error = heap.alloc_obj(Object::ordinary(Value::Obj(error)));
        let date = heap.alloc_obj(Object::ordinary(Value::Obj(object)));
        let regexp = heap.alloc_obj(Object::ordinary(Value::Obj(object)));
        let promise = heap.alloc_obj(Object::ordinary(Value::Obj(object)));
        let iterator = heap.alloc_obj(Object::ordinary(Value::Obj(object)));
        // Prototypes itérateurs concrets (spec) — un niveau entre
        // l'objet itérateur et `%IteratorPrototype%`. Sans lui,
        // `getPrototypeOf(getPrototypeOf(iter))` retombe sur
        // `Object.prototype` : le polyfill iterator-helpers de
        // github (`ser.js` `ee()`) y installe alors `map`/`filter`…
        // via `Object.assign` — énumérables, empoisonne `for in`.
        let array_iter = heap.alloc_obj(Object::ordinary(Value::Obj(iterator)));
        let map_iter = heap.alloc_obj(Object::ordinary(Value::Obj(iterator)));
        let set_iter = heap.alloc_obj(Object::ordinary(Value::Obj(iterator)));
        let string_iter = heap.alloc_obj(Object::ordinary(Value::Obj(iterator)));
        let regexp_iter = heap.alloc_obj(Object::ordinary(Value::Obj(iterator)));
        let usp_iter = heap.alloc_obj(Object::ordinary(Value::Obj(iterator)));
        let iter_helper = heap.alloc_obj(Object::ordinary(Value::Obj(iterator)));
        let map = heap.alloc_obj(Object::ordinary(Value::Obj(object)));
        let set = heap.alloc_obj(Object::ordinary(Value::Obj(object)));
        let symbol = heap.alloc_obj(Object::ordinary(Value::Obj(object)));
        let proxy = heap.alloc_obj(Object::ordinary(Value::Obj(object)));
        let array_buffer = heap.alloc_obj(Object::ordinary(Value::Obj(object)));
        let data_view = heap.alloc_obj(Object::ordinary(Value::Obj(object)));
        let typed_array = heap.alloc_obj(Object::ordinary(Value::Obj(object)));
        let bigint = heap.alloc_obj(Object::ordinary(Value::Obj(object)));
        let aggregate_error = heap.alloc_obj(Object::ordinary(Value::Obj(error)));
        let url = heap.alloc_obj(Object::ordinary(Value::Obj(object)));
        let url_params = heap.alloc_obj(Object::ordinary(Value::Obj(object)));
        // Spec : `WeakMap.prototype`/`WeakSet.prototype` héritent de
        // `Object.prototype`, PAS de `Map`/`Set` (pas d'itérateur ni
        // d'accès aux entrées sur les weak collections).
        let weak_map = heap.alloc_obj(Object::ordinary(Value::Obj(object)));
        let weak_set = heap.alloc_obj(Object::ordinary(Value::Obj(object)));
        let global = heap.alloc_obj(Object::ordinary(Value::Obj(object)));
        let seed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0x9e37_79b9_7f4a_7c15, |d| d.as_nanos() as u64);
        let mut e = Engine {
            heap,
            global,
            global_env: Env::global(),
            protos: Protos {
                object,
                function,
                array,
                string,
                number,
                boolean,
                error,
                type_error,
                reference_error,
                syntax_error,
                eval_error,
                range_error,
                uri_error,
                date,
                regexp,
                promise,
                iterator,
                array_iter,
                map_iter,
                set_iter,
                string_iter,
                regexp_iter,
                usp_iter,
                iter_helper,
                map,
                set,
                symbol,
                proxy,
                array_buffer,
                data_view,
                typed_array,
                bigint,
                aggregate_error,
                url,
                url_params,
                weak_map,
                weak_set,
            },
            rng: seed | 1,
            t0: Instant::now(),
            symbol_reg: std::collections::BTreeMap::new(),
            sym_seq: 0,
            sym_index: std::collections::BTreeMap::new(),
            finregs: Vec::new(),
            gen_ctor: Value::Undefined,
            async_gen_ctor: Value::Undefined,
            gen_fn_proto: Value::Undefined,
            gen_proto: Value::Undefined,
            async_gen_fn_proto: Value::Undefined,
            async_gen_proto: Value::Undefined,
            call_depth: 0,
            call_depth_limit: DEFAULT_CALL_DEPTH,
            call_stack: Vec::new(),
            eval_hook: None,
            expr_depth: 0,
            nest_limit: crate::parser::DEFAULT_NEST_LIMIT,
            ops_left: DEFAULT_OP_LIMIT,
            op_limit: DEFAULT_OP_LIMIT,
            gc_threshold: DEFAULT_GC_THRESHOLD,
            ops_total: 0,
            timers: crate::timers::TimerQueue::default(),
            xhr: crate::xhr::XhrEngineState::default(),
            dom_ctx: None,
            microtasks: std::collections::VecDeque::new(),
            mutobs: crate::mutation::MutObsState::default(),
            obs: crate::observers::ObsState::default(),
            files: crate::files::FilesState::default(),
            streams: crate::streams::StreamState::default(),
            idb: crate::idb::IdbState::default(),
            ws: crate::websocket::WsState::default(),
            es: crate::eventsource::EsState::default(),
            misc: crate::misc_apis::MiscState::default(),
            crypto: crate::crypto::CryptoState::default(),
            crypto_transport: None,
            es_transport: None,
            ce_upgrade_slot: None,
            hooks: crate::dom_host::BrowserHooks::default(),
            completion: Value::Undefined,
            strict: false,
            modules: Vec::new(),
            module_specs: std::collections::HashMap::new(),
            module_envs: Vec::new(),
            module_loader: None,
            current_module: String::new(),
            import_map: Vec::new(),
            workers: Vec::new(),
            worker_factory: None,
            worker_cfg: crate::worker::WorkerConfig::default(),
            worker_out: None,
            worker_loader: None,
            worker_close: false,
            worker_alive: Arc::new(AtomicBool::new(true)),
            gen_collect: Vec::new(),
            pending_labels: Vec::new(),
            using_stack: Vec::new(),
            eval_fn: Value::Undefined,
            re_ctor: Value::Undefined,
            re_static: None,
            cur_ctx: 0,
            ctxs: vec![crate::context::CtxMeta {
                parent: None,
                children: Vec::new(),
                url: String::new(),
                frame_node: None,
                proxy: None,
                global: None,
                alive: true,
                defer_msgs: false,
            }],
            parked: std::collections::HashMap::new(),
            pending_msgs: std::collections::VecDeque::new(),
            port_msgs: std::collections::VecDeque::new(),
            dom_ctors: std::collections::HashMap::new(),
            native_caller: Vec::new(),
        };
        crate::builtins::install(&mut e);
        crate::collections::install(&mut e);
        crate::intl::install(&mut e);
        crate::temporal::install(&mut e);
        crate::json::install(&mut e);
        crate::typed_array::install(&mut e);
        e.ctxs[0].global = Some(e.global);
        e
    }

    /// Maximum number of statement/expression ticks before the script
    /// is killed. Reset by each `eval` call.
    pub fn set_op_limit(&mut self, limit: u64) {
        self.op_limit = limit;
    }

    /// Borne d'appels JS imbriqués — le défaut est calibré sur une
    /// pile de thread de 2 Mio (~27,5 Kio/appel mesuré, JS-C41).
    /// À relever quand le moteur tourne sur un thread à pile large.
    pub fn set_call_depth_limit(&mut self, limit: u32) {
        self.call_depth_limit = limit;
    }

    /// Borne d'emboîtement du parseur et de l'évaluateur
    /// d'expressions — même calcul que [`Self::set_call_depth_limit`]
    /// (jusqu'à ~19 Kio/niveau de parse mesuré, JS-C41).
    pub fn set_nest_limit(&mut self, limit: u32) {
        self.nest_limit = limit;
    }

    /// Hook d'observabilité `eval` (diagnostic) : appelé avec la
    /// source de chaque `eval()`/`Function()`/timer-string **avant
    /// parse** — les frames d'erreur de code évalué ne pointent que
    /// dans cette source (le `<script>` parent n'y est pas).
    /// `None` = aucun coût. Additif, aucune casse d'API.
    pub fn set_eval_hook(&mut self, f: Option<EvalHook>) {
        self.eval_hook = f;
    }

    /// Borne mémoire du heap JS en octets estimés (JS-C37,
    /// ADR-0019) — `0` = illimité. Le dépassement produit un kill
    /// doux : `RangeError` (« heap memory limit exceeded ») au
    /// `tick` suivant, propagé comme une exception JS ordinaire.
    /// L'estimation est approximative entre deux sweeps (les props
    /// ajoutées à un objet vivant n'entrent qu'au GC).
    pub fn set_mem_limit(&mut self, bytes: usize) {
        self.heap.set_mem_limit(bytes);
    }

    /// Seuil de GC automatique en cellules vivantes (JS-C37) —
    /// `0` = jamais. La passe ne peut partir qu'entre deux jobs.
    pub fn set_gc_threshold(&mut self, cells: usize) {
        self.gc_threshold = cells;
    }

    /// Instantané des compteurs heap (cellules vivantes, octets
    /// estimés, pics, nombre de passes GC) — instrumentation
    /// JS-C37 pour embedders et tests.
    #[must_use]
    pub fn heap_stats(&self) -> crate::heap::HeapStats {
        self.heap.stats()
    }

    /// Nombre cumulé de ticks d'exécution depuis la création —
    /// compteur d'ops brut (métrique, non remis à zéro par `eval`).
    #[must_use]
    pub fn ops_used(&self) -> u64 {
        self.ops_total
    }

    /// Roote `id` jusqu'au [`Engine::remove_root`] correspondant —
    /// handles hôtes (ex. wrapper DOM tenue côté `explorer-dom`
    /// sans ref JS). Ref-counté : chaque `add_root` appelle un
    /// `remove_root`.
    pub fn add_root(&mut self, id: ObjId) {
        self.heap.add_root(id);
    }

    /// Retire une racine posée par [`Engine::add_root`].
    pub fn remove_root(&mut self, id: ObjId) {
        self.heap.remove_root(id);
    }

    // --- Contextes de navigation imbriqués (JS-C78) -------------

    /// Échange tous les champs document-local entre `self` (le
    /// contexte courant) et un [`crate::context::BrowsingContext`]
    /// garé. **Tout champ ajouté à [`Engine`] doit décider de
    /// son camp** : document-local → swapper ici ET dans
    /// `BrowsingContext` ; engine-global (heap, budgets, hooks
    /// console, workers) → ne rien faire.
    fn swap_ctx_fields(&mut self, ctx: &mut crate::context::BrowsingContext) {
        use std::mem::swap;
        swap(&mut self.global, &mut ctx.global);
        swap(&mut self.global_env, &mut ctx.global_env);
        swap(&mut self.protos, &mut ctx.protos);
        swap(&mut self.gen_ctor, &mut ctx.gen_ctor);
        swap(&mut self.async_gen_ctor, &mut ctx.async_gen_ctor);
        swap(&mut self.gen_fn_proto, &mut ctx.gen_fn_proto);
        swap(&mut self.gen_proto, &mut ctx.gen_proto);
        swap(&mut self.async_gen_fn_proto, &mut ctx.async_gen_fn_proto);
        swap(&mut self.async_gen_proto, &mut ctx.async_gen_proto);
        swap(&mut self.eval_fn, &mut ctx.eval_fn);
        swap(&mut self.re_ctor, &mut ctx.re_ctor);
        swap(&mut self.re_static, &mut ctx.re_static);
        swap(&mut self.finregs, &mut ctx.finregs);
        swap(&mut self.dom_ctx, &mut ctx.dom_ctx);
        swap(&mut self.timers, &mut ctx.timers);
        swap(&mut self.xhr, &mut ctx.xhr);
        swap(&mut self.mutobs, &mut ctx.mutobs);
        swap(&mut self.obs, &mut ctx.obs);
        swap(&mut self.files, &mut ctx.files);
        swap(&mut self.streams, &mut ctx.streams);
        swap(&mut self.idb, &mut ctx.idb);
        swap(&mut self.ws, &mut ctx.ws);
        swap(&mut self.es, &mut ctx.es);
        swap(&mut self.misc, &mut ctx.misc);
        swap(&mut self.crypto, &mut ctx.crypto);
        swap(&mut self.ce_upgrade_slot, &mut ctx.ce_upgrade_slot);
        swap(&mut self.strict, &mut ctx.strict);
        swap(&mut self.completion, &mut ctx.completion);
        swap(&mut self.pending_labels, &mut ctx.pending_labels);
        swap(&mut self.modules, &mut ctx.modules);
        swap(&mut self.module_specs, &mut ctx.module_specs);
        swap(&mut self.module_envs, &mut ctx.module_envs);
        swap(&mut self.current_module, &mut ctx.current_module);
        swap(&mut self.import_map, &mut ctx.import_map);
        swap(&mut self.hooks.page_url, &mut ctx.page_url);
        swap(&mut self.hooks.screen_obj, &mut ctx.screen_obj);
        swap(&mut self.hooks.navigate, &mut ctx.navigate);
        swap(&mut self.hooks.history_go, &mut ctx.history_go);
        swap(&mut self.hooks.history_len, &mut ctx.history_len);
        swap(&mut self.hooks.form_submit, &mut ctx.form_submit);
        swap(&mut self.hooks.open, &mut ctx.open);
        swap(&mut self.hooks.push_state, &mut ctx.push_state);
        swap(&mut self.hooks.history_state, &mut ctx.history_state);
        swap(&mut self.hooks.win_name, &mut ctx.win_name);
        swap(
            &mut self.hooks.scroll_restore_manual,
            &mut ctx.scroll_restore_manual,
        );
        swap(&mut self.hooks.ready_state, &mut ctx.ready_state);
        swap(&mut self.hooks.dialog_modal, &mut ctx.dialog_modal);
        swap(&mut self.hooks.dialog_retval, &mut ctx.dialog_retval);
        swap(&mut self.hooks.viewport, &mut ctx.viewport);
    }

    /// Rend `target` le contexte courant : ses champs garés sont
    /// swappés sur `self`, l'ancien courant est garé sous son id.
    /// `target` inexistant/mort → no-op (l'appelant garde le
    /// contexte courant — les pompes itératives n'ont pas à
    /// vérifier).
    pub(crate) fn switch_to(&mut self, target: crate::context::CtxId) {
        if target == self.cur_ctx {
            return;
        }
        let Some(mut parked) = self.parked.remove(&target) else {
            return;
        };
        self.swap_ctx_fields(&mut parked);
        self.parked.insert(self.cur_ctx, parked);
        self.cur_ctx = target;
    }

    /// Crée un contexte enfant de `cur_ctx` et le rend courant :
    /// les champs du parent sont garés, des intrinsics frais
    /// (protos/global/env par realm — spec) sont alloués et les
    /// builtins réinstallés. `frame_node` = `(Rc::as_ptr du Dom
    /// parent, NodeId du <iframe>)` — sert `contentWindow`.
    ///
    /// La séquence `builtins → dom → xhr → files…` des installs
    /// hôte est à la charge de l'appelant **après retour** (le
    /// nouveau contexte est courant — `install_dom` & co posent
    /// dans le bon global).
    pub fn context_new(
        &mut self,
        url: &str,
        frame_node: Option<(usize, crate::dom::NodeId)>,
    ) -> u32 {
        let parent = self.cur_ctx;
        let id = self.ctxs.len() as u32;
        self.ctxs.push(crate::context::CtxMeta {
            parent: Some(parent),
            children: Vec::new(),
            url: url.to_string(),
            frame_node,
            proxy: None,
            global: None,
            alive: true,
            defer_msgs: false,
        });
        if let Some(m) = self.ctxs.get_mut(parent as usize) {
            m.children.push(id);
        }
        // Intrinsics frais par realm (spec : chaque global a ses
        // prototypes — `frame.Array !== parent.Array`).
        let mut fresh = crate::context::BrowsingContext::blank(&mut self.heap);
        self.swap_ctx_fields(&mut fresh);
        // `self` porte désormais des champs neufs — installe les
        // builtins dans le nouveau global (même séquence que
        // `Engine::new`).
        crate::builtins::install(self);
        crate::collections::install(self);
        crate::intl::install(self);
        crate::temporal::install(self);
        crate::json::install(self);
        crate::typed_array::install(self);
        self.hooks.page_url = url.to_string();
        crate::context::install_ctx_window_props(self);
        self.parked.insert(parent, fresh);
        self.cur_ctx = id;
        self.ctxs[id as usize].global = Some(self.global);
        // `window[n]` sur la fenêtre parente — accessor index
        // DYNAMIQUE (l'enfant résolu à la lecture : `destroy`
        // et réordonnancement restent honnêtes — jamais une
        // data prop figée pointant un contexte mort).
        let idx = (self.ctxs[parent as usize].children.len() - 1) as f64;
        let get = self.bound_native(
            "get [index]",
            crate::context::window_index_get,
            vec![Value::Num(idx)],
        );
        let parent_global = if parent == self.cur_ctx {
            Some(self.global)
        } else {
            self.parked.get(&parent).map(|p| p.global)
        };
        if let Some(g) = parent_global {
            if let Some(o) = self.heap.obj_mut(g) {
                o.set_accessor(&((idx as usize).to_string()), get, Value::Undefined, true);
            }
        }
        id
    }

    /// Contexte enfant porté par l'`<iframe>` `node` du miroir
    /// `dom_ptr` (`Rc::as_ptr` du Dom parent) — `contentWindow`/
    /// `contentDocument`/`frameElement` l'interrogent.
    pub(crate) fn ctx_of_frame(
        &self,
        dom_ptr: usize,
        node: crate::dom::NodeId,
    ) -> Option<crate::context::CtxId> {
        self.ctxs
            .iter()
            .position(|m| m.alive && m.frame_node == Some((dom_ptr, node)))
            .map(|i| i as u32)
    }

    /// Détruit un contexte (navigation du frame hors bfcache) :
    /// `alive=false`, retiré des `children` du parent, champs
    /// relâchés (GC les réclamera). `postMessage` vers lui devient
    /// no-op silencieux — spec.
    pub fn context_destroy(&mut self, id: crate::context::CtxId) {
        if id == 0 || id as usize >= self.ctxs.len() {
            return;
        }
        self.ctxs[id as usize].alive = false;
        self.ctxs[id as usize].children.clear();
        if let Some(p) = self.ctxs[id as usize].parent {
            self.ctxs[p as usize].children.retain(|&c| c != id);
        }
        if self.cur_ctx == id {
            let p = self.ctxs[id as usize].parent.unwrap_or(0);
            self.switch_to(p);
        }
        self.parked.remove(&id);
    }

    /// Resynchronise l'URL + `history.length` d'un contexte
    /// après une navigation same-document scriptée
    /// (`history.pushState`/`replaceState`, CAP-151) — la
    /// session a résolu et committé : le miroir reçoit
    /// l'URL **canonisée**. `hooks.page_url`/`history_len`
    /// si le contexte est courant, champs garés sinon ; la
    /// méta `ctxs[ctx]` suit toujours (origine `postMessage`,
    /// `location` d'un `WindowProxy`).
    pub fn sync_ctx_url(&mut self, ctx: crate::context::CtxId, url: &str, history_len: u32) {
        if ctx == self.cur_ctx {
            self.hooks.page_url = url.to_string();
            self.hooks.history_len = history_len;
        } else if let Some(p) = self.parked.get_mut(&ctx) {
            p.page_url = url.to_string();
            p.history_len = history_len;
        }
        if let Some(m) = self.ctxs.get_mut(ctx as usize) {
            m.url = url.to_string();
        }
    }

    /// `sync_ctx_url` + état d'historique (CAP-158) :
    /// `state` = JSON sérialisé de `history.state` pour
    /// l'entrée courante — restitué au getter. `popstate`
    /// = `true` à la traversée same-document seulement
    /// (spec : `pushState`/`replaceState` ne dispatchent
    /// **pas** `popstate`) — l'événement `{type:"popstate",
    /// state}` suit `fire_event_listeners` (cible window).
    pub fn sync_ctx_state(
        &mut self,
        ctx: crate::context::CtxId,
        url: &str,
        history_len: u32,
        state: Option<&str>,
        popstate: bool,
    ) {
        self.sync_ctx_url(ctx, url, history_len);
        let sv = state.map(str::to_string);
        if ctx == self.cur_ctx {
            self.hooks.history_state = sv;
        } else if let Some(p) = self.parked.get_mut(&ctx) {
            p.history_state = sv;
        }
        if !popstate || ctx != self.cur_ctx {
            return;
        }
        // `popstate` — state désérialisé pour les listeners
        // (`null` absent/indésérialisable).
        let st = match state {
            Some(s) => {
                let sv = self.str_val(s);
                crate::json::parse(self, sv).unwrap_or(Value::Null)
            }
            None => Value::Null,
        };
        let ev = self.new_object();
        let ty = self.str_val("popstate");
        self.set_own(ev, "type", ty, true);
        self.set_own(ev, "state", st, true);
        self.set_own(ev, "cancelable", Value::Bool(false), true);
        self.set_own(ev, "bubbles", Value::Bool(false), true);
        let _ = self.fire_event_listeners(Value::Obj(ev));
    }

    /// Environnement `matchMedia` de la session (CAP-161) —
    /// snapshot posé par l'embedder au spawn (`viewport`,
    /// `prefers-color-scheme`, `hover`/`pointer` desktop).
    pub fn set_media_env(&mut self, env: crate::mediaq::MediaEnv) {
        self.hooks.media_env = env;
    }

    /// `hashchange` (CAP-160) — événement `{type:"hashchange",
    /// oldURL, newURL}` dispatché sur la fenêtre du contexte
    /// `ctx` (routeurs hash des SPA : navigation fragment et
    /// traversée entre fragments le déclenchent — spec HTML).
    /// `ctx` non courant → `switch_to` temporaire, puis
    /// restauration (même convention que les autres coutures
    /// session).
    pub fn fire_hashchange(&mut self, ctx: crate::context::CtxId, old: &str, new: &str) {
        let prev = self.cur_ctx;
        if ctx != prev {
            self.switch_to(ctx);
        }
        let ev = self.new_object();
        let ty = self.str_val("hashchange");
        self.set_own(ev, "type", ty, true);
        let ov = self.str_val(old);
        self.set_own(ev, "oldURL", ov, true);
        let nv = self.str_val(new);
        self.set_own(ev, "newURL", nv, true);
        self.set_own(ev, "cancelable", Value::Bool(false), true);
        self.set_own(ev, "bubbles", Value::Bool(false), true);
        let _ = self.fire_event_listeners(Value::Obj(ev));
        if ctx != prev {
            self.switch_to(prev);
        }
    }

    /// `document.readyState` du contexte `ctx` (CAP-170) —
    /// posé par l'embedder au fil du cycle de vie du document
    /// (`"loading"` → `"interactive"` avant `DOMContentLoaded`
    /// → `"complete"` après `load` ; le `readystatechange` est
    /// dispatché par l'appelant via `dispatch_event_ctx`, même
    /// voie que `DOMContentLoaded`/`load`). `ctx` garé → le
    /// champ garé est écrit ; valeurs hors enum ignorées
    /// (harnais qui balancerait n'importe quoi).
    pub fn set_ready_state(&mut self, ctx: crate::context::CtxId, state: &str) {
        let v: &'static str = match state {
            "loading" => "loading",
            "interactive" => "interactive",
            "complete" => "complete",
            _ => return,
        };
        if ctx == self.cur_ctx {
            self.hooks.ready_state = v;
        } else if let Some(p) = self.parked.get_mut(&ctx) {
            p.ready_state = v;
        }
        // `defer_msgs` : `"loading"` arme la retenue des
        // `postMessage` entrants (le document n'a pas fini son
        // parse — ses scripts/listeners courent encore) ;
        // `interactive`/`complete` la désarme — la file se
        // vide alors au prochain checkpoint.
        if let Some(m) = self.ctxs.get_mut(ctx as usize) {
            m.defer_msgs = v == "loading";
        }
    }

    /// Nom du contexte `ctx` pour `window.name` (CAP-159) —
    /// posé par l'embedder quand la session nomme un
    /// contexte top-level (`target="x"`/`window.open`) ;
    /// la propre-prop `name` du global suit pour que
    /// `window.name` la lise immédiatement.
    pub fn set_ctx_name(&mut self, ctx: crate::context::CtxId, name: &str) {
        let (g, nv) = if ctx == self.cur_ctx {
            self.hooks.win_name = name.to_string();
            (self.global, self.str_val(name))
        } else {
            let Some(p) = self.parked.get_mut(&ctx) else {
                return;
            };
            p.win_name = name.to_string();
            (p.global, self.str_val(name))
        };
        self.set_own(g, "name", nv, true);
    }

    /// Draine les `<script>` devenus attachés par mutation
    /// (CAP-169) — tous les contextes (courant **et** garés :
    /// un script d'iframe injecté puis garé doit quand même
    /// être chargé — la file vit dans son `Dom`). Chaque
    /// requête porte `seq` (répercuté à la livraison pour
    /// `load`/`error`/`currentScript`) et `ctx` émetteur ;
    /// `src`/`text` selon la balise. La session résout,
    /// applique CSP/mixed-content et fetch — le moteur ne
    /// touche jamais le réseau.
    pub fn drain_dyn_scripts(&mut self) -> Vec<crate::dom_host::DynScriptReq> {
        let mut doms: Vec<(crate::context::CtxId, crate::dom_host::DomRef)> = Vec::new();
        if let Some((d, _)) = &self.dom_ctx {
            doms.push((self.cur_ctx, d.clone()));
        }
        for (id, ctx) in &self.parked {
            if let Some((d, _)) = &ctx.dom_ctx {
                doms.push((*id, d.clone()));
            }
        }
        let mut out = Vec::new();
        for (ctx, dom) in doms {
            let nodes = std::mem::take(&mut dom.borrow_mut().pending_dyn);
            let d = dom.borrow();
            for n in nodes {
                let Some(node) = d.nodes.get(n as usize) else {
                    continue;
                };
                let attr = |k: &str| {
                    node.attrs
                        .iter()
                        .find(|(name, _)| name.eq_ignore_ascii_case(k))
                        .map(|(_, v)| v.clone())
                };
                let src = attr("src").filter(|s| !s.trim().is_empty());
                let text = if src.is_none() {
                    let t = d.text_content(n);
                    (!t.trim().is_empty()).then_some(t)
                } else {
                    None
                };
                let crossorigin =
                    attr("crossorigin").map(|v| match v.trim().to_ascii_lowercase().as_str() {
                        "use-credentials" => crate::xhr::FetchCredentials::Include,
                        _ => crate::xhr::FetchCredentials::SameOrigin,
                    });
                let seq = self.hooks.dyn_script_seq;
                self.hooks.dyn_script_seq += 1;
                // Borné : 256 livraisons en vol par document —
                // la plus ancienne est abandonnée (son `load`
                // ne partira pas — l'élément reste inerte,
                // jamais de croissance hostile).
                if self.hooks.dyn_script_nodes.len() >= 256 {
                    if let Some((first, _)) = self
                        .hooks
                        .dyn_script_nodes
                        .iter()
                        .next()
                        .map(|(k, v)| (*k, *v))
                    {
                        self.hooks.dyn_script_nodes.remove(&first);
                    }
                }
                self.hooks.dyn_script_nodes.insert(seq, n);
                out.push(crate::dom_host::DynScriptReq {
                    seq,
                    src,
                    text,
                    crossorigin,
                    integrity: attr("integrity"),
                    referrerpolicy: attr("referrerpolicy"),
                    charset: attr("charset"),
                    nonce: attr("nonce"),
                    ctx,
                });
            }
        }
        out
    }

    /// Livraison d'un `<script>` dynamique (CAP-169) —
    /// appelé dans le contexte `ctx` émetteur (le `Cmd`
    /// dispatch switche avant). `Some(text)` = corps accepté
    /// → évalué avec `currentScript` posé sur l'élément,
    /// puis `load` tiré dessus ; `None` = fetch refusé
    /// (réseau, CSP, SRI…) → `error`. Les deux events sont
    /// non-bullants/non-annulables (spec — `NON_BUBBLING`/
    /// `NON_CANCELABLE` existants). Élément disparu → no-op
    /// tolérant.
    pub fn deliver_dyn_script(&mut self, seq: u64, body: Option<&str>) {
        let Some(node) = self.hooks.dyn_script_nodes.remove(&seq) else {
            return;
        };
        let Some((dom, cache)) = self.dom_ctx.clone() else {
            return;
        };
        let ty = match body {
            Some(text) => {
                let prev = self.hooks.current_script;
                self.hooks.current_script = Some(node);
                // Un script qui jette n'empêche pas `load`
                // (spec : `error` n'est que transport/CSP).
                // `eval_named` (JS-C87) : la pile d'erreur
                // attribue le chunk (`at fn (url:l:c)`) —
                // src sinon étiquette seq.
                let label = {
                    let d = dom.borrow();
                    d.attr(node, "src")
                        .map_or_else(|| format!("dyn:{seq}"), str::to_string)
                };
                let _ = self.eval_named(text, &label);
                self.hooks.current_script = prev;
                "load"
            }
            None => "error",
        };
        let (bubbles, cancelable, composed) = crate::dom_host::platform_event_flags(ty);
        let ev = crate::dom_host::make_event_state(
            ty,
            node,
            crate::dom_host::EventFlags {
                bubbles,
                cancelable,
                composed,
                trusted: true,
            },
            Vec::new(),
        );
        let _ = self.propagate_event(&dom, &cache, &ev);
    }

    /// Draine les `<iframe>` devenus attachés (ou `src`/`srcdoc`
    /// modifié sur cadre attaché — CAP-171) — tous les
    /// contextes, même discipline que [`Self::drain_dyn_scripts`].
    /// `ord` = index du nœud parmi les `<iframe>` du miroir
    /// émetteur — `context_new(frame_ord)` le traduit en
    /// `(dom_ptr, node)` pour `contentWindow`/`frameElement`.
    /// Un cadre détaché entre mutation et drain n'est pas
    /// émis (insertion-détachée = annulation, spec).
    pub fn drain_dyn_frames(&mut self) -> Vec<crate::dom_host::DynFrameReq> {
        let mut doms: Vec<(crate::context::CtxId, crate::dom_host::DomRef)> = Vec::new();
        if let Some((d, _)) = &self.dom_ctx {
            doms.push((self.cur_ctx, d.clone()));
        }
        for (id, ctx) in &self.parked {
            if let Some((d, _)) = &ctx.dom_ctx {
                doms.push((*id, d.clone()));
            }
        }
        let mut out = Vec::new();
        for (ctx, dom) in doms {
            let nodes = std::mem::take(&mut dom.borrow_mut().pending_dyn_frames);
            let d = dom.borrow();
            for n in nodes {
                let Some(node) = d.nodes.get(n as usize) else {
                    continue;
                };
                if !d.is_attached(n) {
                    continue;
                }
                let attr = |k: &str| {
                    node.attrs
                        .iter()
                        .find(|(name, _)| name.eq_ignore_ascii_case(k))
                        .map(|(_, v)| v.clone())
                };
                let ord = d
                    .nodes
                    .iter()
                    .enumerate()
                    .filter(|(_, m)| m.kind == crate::dom::NodeKind::Element && m.tag == "iframe")
                    .position(|(i, _)| i == n as usize);
                let Some(ord) = ord else { continue };
                let seq = self.hooks.dyn_script_seq;
                self.hooks.dyn_script_seq += 1;
                if self.hooks.dyn_frame_nodes.len() >= 256 {
                    if let Some((first, _)) = self
                        .hooks
                        .dyn_frame_nodes
                        .iter()
                        .next()
                        .map(|(k, v)| (*k, *v))
                    {
                        self.hooks.dyn_frame_nodes.remove(&first);
                    }
                }
                self.hooks.dyn_frame_nodes.insert(seq, n);
                out.push(crate::dom_host::DynFrameReq {
                    seq,
                    ord: u32::try_from(ord).unwrap_or(u32::MAX),
                    src: attr("src"),
                    srcdoc: attr("srcdoc"),
                    sandbox: attr("sandbox"),
                    referrerpolicy: attr("referrerpolicy"),
                    ctx,
                });
            }
        }
        out
    }

    /// Livraison d'un `<iframe>` dynamique (CAP-171) — le
    /// sous-contexte a été créé (`Ok`) ou refusé (fetch, CSP,
    /// XFO…) par la session ; l'événement `load`/`error` part
    /// sur l'élément miroir (non-bullant, spec). Élément
    /// retiré entre-temps → no-op tolérant.
    pub fn deliver_dyn_frame(&mut self, seq: u64, ok: bool) {
        let Some(node) = self.hooks.dyn_frame_nodes.remove(&seq) else {
            return;
        };
        let Some((dom, cache)) = self.dom_ctx.clone() else {
            return;
        };
        let ty = if ok { "load" } else { "error" };
        let (bubbles, cancelable, composed) = crate::dom_host::platform_event_flags(ty);
        let ev = crate::dom_host::make_event_state(
            ty,
            node,
            crate::dom_host::EventFlags {
                bubbles,
                cancelable,
                composed,
                trusted: true,
            },
            Vec::new(),
        );
        let _ = self.propagate_event(&dom, &cache, &ev);
    }

    /// Draine les `<img>` dont la source a été demandée
    /// (CAP-172 — `src`/`srcset`/`sizes` posé ou insertion)
    /// — tous les contextes, même discipline que
    /// [`Self::drain_dyn_scripts`]. Contrairement aux cadres
    /// la **détache** n'exclut pas : `new Image().src=` est
    /// un prefetch spec — seul un nœud devenu absent du
    /// miroir saute.
    pub fn drain_dyn_imgs(&mut self) -> Vec<crate::dom_host::DynImgReq> {
        let mut doms: Vec<(crate::context::CtxId, crate::dom_host::DomRef)> = Vec::new();
        if let Some((d, _)) = &self.dom_ctx {
            doms.push((self.cur_ctx, d.clone()));
        }
        for (id, ctx) in &self.parked {
            if let Some((d, _)) = &ctx.dom_ctx {
                doms.push((*id, d.clone()));
            }
        }
        let mut out = Vec::new();
        for (ctx, dom) in doms {
            let nodes = std::mem::take(&mut dom.borrow_mut().pending_dyn_imgs);
            let d = dom.borrow();
            for n in nodes {
                let Some(node) = d.nodes.get(n as usize) else {
                    continue;
                };
                if node.kind != crate::dom::NodeKind::Element || node.tag != "img" {
                    continue;
                }
                let attr = |k: &str| {
                    node.attrs
                        .iter()
                        .find(|(name, _)| name.eq_ignore_ascii_case(k))
                        .map(|(_, v)| v.clone())
                };
                let seq = self.hooks.dyn_script_seq;
                self.hooks.dyn_script_seq += 1;
                if self.hooks.dyn_img_nodes.len() >= 256 {
                    if let Some((first, _)) = self
                        .hooks
                        .dyn_img_nodes
                        .iter()
                        .next()
                        .map(|(k, v)| (*k, *v))
                    {
                        self.hooks.dyn_img_nodes.remove(&first);
                    }
                }
                self.hooks.dyn_img_nodes.insert(seq, n);
                let crossorigin =
                    attr("crossorigin").map(|v| match v.trim().to_ascii_lowercase().as_str() {
                        "use-credentials" => crate::xhr::FetchCredentials::Include,
                        _ => crate::xhr::FetchCredentials::SameOrigin,
                    });
                out.push(crate::dom_host::DynImgReq {
                    seq,
                    src: attr("src"),
                    srcset: attr("srcset"),
                    sizes: attr("sizes"),
                    crossorigin,
                    referrerpolicy: attr("referrerpolicy"),
                    ctx,
                });
            }
        }
        out
    }

    /// Livraison d'un `<img>` dynamique (CAP-172) — corps
    /// décodé (`Ok`) ou refusé (fetch, CSP `img-src`,
    /// mixed-content) par la session ; `load`/`error` part
    /// sur l'élément miroir (non-bullant, spec) — attaché
    /// **ou détaché** (prefetch `new Image()`).
    pub fn deliver_dyn_img(&mut self, seq: u64, ok: bool) {
        let Some(node) = self.hooks.dyn_img_nodes.remove(&seq) else {
            return;
        };
        let Some((dom, cache)) = self.dom_ctx.clone() else {
            return;
        };
        let ty = if ok { "load" } else { "error" };
        let (bubbles, cancelable, composed) = crate::dom_host::platform_event_flags(ty);
        let ev = crate::dom_host::make_event_state(
            ty,
            node,
            crate::dom_host::EventFlags {
                bubbles,
                cancelable,
                composed,
                trusted: true,
            },
            Vec::new(),
        );
        let _ = self.propagate_event(&dom, &cache, &ev);
    }

    /// Draine les `<link rel=stylesheet>` demandés
    /// (CAP-173 — insertion ou `href`/`rel` changé sur
    /// lien attaché) — tous les contextes, même discipline
    /// que [`Self::drain_dyn_imgs`]. **Détaché exclu** :
    /// spec « link processing » — un lien hors du
    /// document ne charge pas (la `dyn_link_seen` garde
    /// la clé : une ré-insertion à clé inchangée ne
    /// re-fetch pas — borne).
    pub fn drain_dyn_links(&mut self) -> Vec<crate::dom_host::DynLinkReq> {
        let mut doms: Vec<(crate::context::CtxId, crate::dom_host::DomRef)> = Vec::new();
        if let Some((d, _)) = &self.dom_ctx {
            doms.push((self.cur_ctx, d.clone()));
        }
        for (id, ctx) in &self.parked {
            if let Some((d, _)) = &ctx.dom_ctx {
                doms.push((*id, d.clone()));
            }
        }
        let mut out = Vec::new();
        for (ctx, dom) in doms {
            let nodes = std::mem::take(&mut dom.borrow_mut().pending_dyn_links);
            let d = dom.borrow();
            for n in nodes {
                if !d.is_attached(n) {
                    continue;
                }
                let Some(node) = d.nodes.get(n as usize) else {
                    continue;
                };
                if node.kind != crate::dom::NodeKind::Element || node.tag != "link" {
                    continue;
                }
                let attr = |k: &str| {
                    node.attrs
                        .iter()
                        .find(|(name, _)| name.eq_ignore_ascii_case(k))
                        .map(|(_, v)| v.clone())
                };
                let Some(href) = attr("href") else {
                    continue;
                };
                let seq = self.hooks.dyn_script_seq;
                self.hooks.dyn_script_seq += 1;
                if self.hooks.dyn_link_nodes.len() >= 256 {
                    if let Some((first, _)) = self
                        .hooks
                        .dyn_link_nodes
                        .iter()
                        .next()
                        .map(|(k, v)| (*k, *v))
                    {
                        self.hooks.dyn_link_nodes.remove(&first);
                    }
                }
                self.hooks.dyn_link_nodes.insert(seq, n);
                let crossorigin =
                    attr("crossorigin").map(|v| match v.trim().to_ascii_lowercase().as_str() {
                        "use-credentials" => crate::xhr::FetchCredentials::Include,
                        _ => crate::xhr::FetchCredentials::SameOrigin,
                    });
                out.push(crate::dom_host::DynLinkReq {
                    seq,
                    href,
                    integrity: attr("integrity"),
                    media: attr("media"),
                    crossorigin,
                    referrerpolicy: attr("referrerpolicy"),
                    ctx,
                });
            }
        }
        out
    }

    /// Livraison d'un `<link>` dynamique (CAP-173) —
    /// feuille décodée et rangée (`Ok`) ou refusée
    /// (fetch, CSP `style-src`, SRI, mixed-content) par
    /// la session ; `load`/`error` part sur l'élément
    /// miroir (non-bullant, spec).
    pub fn deliver_dyn_link(&mut self, seq: u64, ok: bool) {
        let Some(node) = self.hooks.dyn_link_nodes.remove(&seq) else {
            return;
        };
        let Some((dom, cache)) = self.dom_ctx.clone() else {
            return;
        };
        let ty = if ok { "load" } else { "error" };
        let (bubbles, cancelable, composed) = crate::dom_host::platform_event_flags(ty);
        let ev = crate::dom_host::make_event_state(
            ty,
            node,
            crate::dom_host::EventFlags {
                bubbles,
                cancelable,
                composed,
                trusted: true,
            },
            Vec::new(),
        );
        let _ = self.propagate_event(&dom, &cache, &ev);
    }

    /// Contexte propriétaire du miroir `dom` (`Rc::as_ptr`) —
    /// `contentDocument.readyState` vu depuis un autre realm
    /// doit lire l'état de **son** document, pas du ctx
    /// courant (CAP-171). `None` = miroir inconnu.
    pub(crate) fn dom_owner_ctx(
        &self,
        dom: &crate::dom_host::DomRef,
    ) -> Option<crate::context::CtxId> {
        if let Some((d, _)) = &self.dom_ctx {
            if std::rc::Rc::ptr_eq(d, dom) {
                return Some(self.cur_ctx);
            }
        }
        self.parked.iter().find_map(|(id, c)| {
            c.dom_ctx
                .as_ref()
                .filter(|(d, _)| std::rc::Rc::ptr_eq(d, dom))
                .map(|_| *id)
        })
    }

    /// `readyState` du document du contexte `ctx` — courant
    /// via `self.hooks`, garé via son `BrowsingContext` ;
    /// vide = `"loading"` (miroir installé en cours de parse,
    /// CAP-170).
    pub(crate) fn ready_state_ctx(&self, ctx: crate::context::CtxId) -> &'static str {
        let s = if ctx == self.cur_ctx {
            self.hooks.ready_state
        } else {
            self.parked.get(&ctx).map_or("", |c| c.ready_state)
        };
        if s.is_empty() {
            "loading"
        } else {
            s
        }
    }

    /// Contexte courant.
    #[must_use]
    pub fn current_context(&self) -> crate::context::CtxId {
        self.cur_ctx
    }

    /// Rend `ctx` le contexte courant (couture session :
    /// `install_dom`/`eval`/`dom_event` agissent alors sur lui).
    /// `0` = top-level.
    pub fn context_switch(&mut self, ctx: crate::context::CtxId) {
        self.switch_to(ctx);
    }

    /// Le `window` de `target` vu depuis `viewer` : même
    /// origine → le vrai global (accès synchrone spec) ;
    /// cross-origin → `WindowProxy` opaque stable par cible
    /// (identité préservée : `f.contentWindow === f.contentWindow`).
    /// `target` garé : son global vit dans `parked` — même heap.
    pub(crate) fn ctx_window_value_as(
        &mut self,
        viewer: crate::context::CtxId,
        target: crate::context::CtxId,
    ) -> Value {
        if self.ctx_same_origin(viewer, target) {
            let g = if target == self.cur_ctx {
                self.global
            } else {
                self.ctxs[target as usize]
                    .global
                    .or_else(|| self.parked.get(&target).map(|p| p.global))
                    .unwrap_or(self.global)
            };
            return Value::Obj(g);
        }
        self.ctx_proxy(target)
    }

    /// Raccourci `cur_ctx`-vue (props `parent`/`top`/`frames`
    /// résolvent leur ctx depuis `this` via [`Self::ctx_of_objid`]
    /// puis appellent `_as` — jamais `cur_ctx` directement, sauf
    /// pour la prop `window`/`self` qui vaut le global `this`).
    pub(crate) fn ctx_window_value(&mut self, ctx: crate::context::CtxId) -> Value {
        self.ctx_window_value_as(self.cur_ctx, ctx)
    }

    /// `WindowProxy` stable du contexte `ctx` — créé au premier
    /// accès cross-origin, gardé dans `ctxs.meta.proxy` (raciné
    /// au GC — `collect` trace les proxies de tous les ctxs).
    fn ctx_proxy(&mut self, ctx: crate::context::CtxId) -> Value {
        if let Some(p) = self.ctxs[ctx as usize].proxy {
            return Value::Obj(p);
        }
        let host = std::rc::Rc::new(crate::context::WindowProxyHost { ctx });
        let Value::Obj(id) = self.new_host_object(host) else {
            return Value::Undefined;
        };
        self.ctxs[ctx as usize].proxy = Some(id);
        Value::Obj(id)
    }

    /// `top` relatif : remonte la chaîne des parents jusqu'à la
    /// racine (`parent` absent ou cycle → `ctx` lui-même).
    pub(crate) fn ctx_top_of(&self, ctx: crate::context::CtxId) -> crate::context::CtxId {
        let mut cur = ctx;
        let mut depth = 0u32;
        while let Some(p) = self.ctxs[cur as usize].parent {
            cur = p;
            depth += 1;
            if depth > 64 {
                break;
            }
        }
        cur
    }

    /// `ctx` dont `global`/`global_env` vaut `id`/`env` —
    /// `cur_ctx` d'abord puis les garés (les propriétaires des
    /// objets résolvent TOUJOURS leur contexte depuis `this`,
    /// jamais depuis `cur_ctx`).
    pub(crate) fn ctx_of_objid(&self, id: ObjId) -> Option<crate::context::CtxId> {
        if id == self.global {
            return Some(self.cur_ctx);
        }
        // `meta.global` identifie aussi les contextes MORTS
        // (`w.closed`/`w.parent` sur un `window` gardé restent
        // résolubles — spec, le global survit comme objet heap).
        self.ctxs
            .iter()
            .position(|m| m.global == Some(id))
            .map(|i| i as crate::context::CtxId)
    }

    /// Contexte propriétaire d'un env — remonte la chaîne des
    /// parents jusqu'à la racine et la compare (`Rc::ptr_eq`)
    /// aux `global_env` des contextes vivants. Une fonction
    /// définie dans un contexte appelée depuis un autre est
    /// ainsi exécutée dans SON realm (spec).
    pub(crate) fn ctx_of_env(&self, env: &EnvRef) -> Option<crate::context::CtxId> {
        let mut cur = env.clone();
        let root = loop {
            let next = cur.borrow().parent.clone();
            match next {
                Some(p) => cur = p,
                None => break cur,
            }
        };
        if std::rc::Rc::ptr_eq(&root, &self.global_env) {
            return Some(self.cur_ctx);
        }
        self.parked
            .iter()
            .find(|(_, c)| std::rc::Rc::ptr_eq(&root, &c.global_env))
            .map(|(k, _)| *k)
    }

    /// Origine sérialisée (`scheme://host[:port]` ou `"null"`)
    /// d'un contexte — partagée par `ctx_same_origin` et
    /// `postMessage.targetOrigin`. Spec « origin inheritance »
    /// (CAP-171) : un document `about:` (`about:blank`,
    /// `about:srcdoc`) hérite l'origine de son contexte
    /// **parent** — top-level `about:` reste opaque.
    pub(crate) fn ctx_origin(&self, ctx: crate::context::CtxId) -> String {
        let meta = &self.ctxs[ctx as usize];
        let o = crate::builtins::url::url_parse_absolute(&meta.url).map_or_else(
            || "null".to_string(),
            |p| crate::builtins::url::url_origin(&p),
        );
        if o == "null" && meta.url.starts_with("about:") {
            if let Some(parent) = meta.parent {
                return self.ctx_origin(parent);
            }
        }
        o
    }

    /// Same-origin (tuple scheme/host/port sérialisé) entre deux
    /// contextes — une origine `"null"` (opaque : `about:`,
    /// `data:`…) ne matche qu'elle-même (réflexif — `parent
    /// === self` reste same-origin pour un top-level opaque).
    pub(crate) fn ctx_same_origin(
        &self,
        a: crate::context::CtxId,
        b: crate::context::CtxId,
    ) -> bool {
        if a == b {
            return true;
        }
        let (oa, ob) = (self.ctx_origin(a), self.ctx_origin(b));
        oa != "null" && oa == ob
    }

    /// `postMessage(data, targetOrigin)` vers `target` — vérifié
    /// à l'émission : `targetOrigin` exigé (`undefined`/absent/
    /// vide → `SyntaxError`) ; `'*'` (spec) = livrer sans
    /// vérification d'origine — la cible reçoit `origin`/`source`
    /// honnêtes et décide (cap « web réel » : les embeds réels
    /// font `postMessage(x, '*')`) ; origine explicite non
    /// concordante → retour silencieux (spec — le message n'est
    /// jamais livré). Contexte mort → no-op silencieux.
    /// Donnée clonée `WireValue` à l'émission.
    pub(crate) fn post_message(
        &mut self,
        target: crate::context::CtxId,
        args: &[Value],
    ) -> JsResult<Value> {
        if args.len() < 2 {
            return Err(self.syntax_error(
                "Failed to execute 'postMessage' on 'Window': 2 arguments required",
            ));
        }
        // `undefined` explicit n'est pas une origine (spec →
        // SyntaxError, même règle qu'un argument absent).
        if matches!(args[1], Value::Undefined) {
            return Err(self.syntax_error(
                "Failed to execute 'postMessage' on 'Window': 'targetOrigin' is required",
            ));
        }
        let target_origin = self.to_js_string(args[1])?;
        if target_origin.is_empty() {
            return Err(self.syntax_error(
                "Failed to execute 'postMessage' on 'Window': 'targetOrigin' is required",
            ));
        }
        if !self.ctxs[target as usize].alive {
            return Ok(Value::Undefined);
        }
        // `'*'` (spec) = livrer sans vérification d'origine —
        // la cible reçoit `origin`/`source` honnêtes et décide.
        // Une origine explicite non concordante → retour
        // silencieux (le message n'est jamais livré).
        let dest_origin = self.ctx_origin(target);
        if target_origin != "*" && dest_origin != target_origin {
            // Origine non concordante → pas de livraison (spec —
            // échec silencieux, pas d'erreur).
            return Ok(Value::Undefined);
        }
        let wire = self.wire_encode(args[0])?;
        // `source`/`origin` = realm de l'APPELANT (spec
        // « incumbent settings object ») — sous switch natif,
        // `cur_ctx` est la CIBLE ; le vrai appelant est en tête
        // de `native_caller`.
        let src = self.native_caller.last().copied().unwrap_or(self.cur_ctx);
        let origin = self.ctx_origin(src);
        self.pending_msgs.push_back(crate::context::PendingMsg {
            target,
            data: wire,
            origin,
            source: src,
        });
        Ok(Value::Undefined)
    }

    /// Vide la file `postMessage` — chaque message est décodé
    /// dans le heap partagé puis dispatché `message` sur la
    /// fenêtre destinataire (`MessageEvent.data`/`origin`/
    /// `source`/`lastEventId`/`ports` peuplés). Les listeners
    /// peuvent poster en retour — la boucle vidange jusqu'à
    /// épuisement (bornée par l'op-budget global).
    pub(crate) fn drain_pending_msgs(&mut self) -> JsResult<usize> {
        let mut n = 0usize;
        let mut deferred: crate::context::PendingMsgs = std::collections::VecDeque::new();
        while let Some(msg) = self.pending_msgs.pop_front() {
            let Some(m) = self.ctxs.get(msg.target as usize) else {
                continue;
            };
            // Contexte mort → drop silencieux (spec).
            // `defer_msgs` : document cible en cours de parse —
            // la task attend que ses scripts aient tourné
            // (spec : livraison après les jobs de parse ;
            // sinon un listener enregistré plus tard dans le
            // même parse est raté — mesuré : scripts inline de
            // l'iframe évalués avant ceux du parent).
            if !m.alive {
                continue;
            }
            if m.defer_msgs {
                deferred.push_back(msg);
                continue;
            }
            self.switch_to(msg.target);
            n += 1;
            let data = self.wire_decode(&msg.data)?;
            let source = self.ctx_window_value(msg.source);
            let ev = self.new_object();
            let ty = self.str_val("message");
            self.set_own(ev, "type", ty, true);
            self.set_own(ev, "data", data, true);
            let o = self.str_val(msg.origin.clone());
            self.set_own(ev, "origin", o, true);
            self.set_own(ev, "source", source, true);
            let s = self.str_val("");
            self.set_own(ev, "lastEventId", s, true);
            let ports = self.new_array(Vec::new());
            self.set_own(ev, "ports", ports, true);
            self.set_own(ev, "cancelable", Value::Bool(false), true);
            self.set_own(ev, "bubbles", Value::Bool(false), true);
            self.fire_event_listeners(Value::Obj(ev))?;
        }
        // Les messages différés repartent en file **devant**
        // ceux postés pendant la livraison (les listeners
        // peuvent reposter — ordre FIFO préservé).
        deferred.append(&mut self.pending_msgs);
        self.pending_msgs = deferred;
        Ok(n)
    }

    /// Dispatch d'un event littéral `message`-like sur la fenêtre
    /// du contexte **courant** : own-prop `on<ty>` du global +
    /// listeners `addEventListener` de la racine (cible window
    /// ≙ sommet — même convention que `win_add_event_listener`).
    /// `currentTarget`/`target` = le global.
    pub(crate) fn fire_event_listeners(&mut self, ev: Value) -> JsResult<()> {
        // `on<ty>` own-prop du global (spec window event handlers).
        let Value::Obj(ev_id) = ev else {
            return Ok(());
        };
        let ty = self
            .get_prop(ev_id, "type")
            .and_then(|v| self.heap_string_of(v))
            .unwrap_or_default();
        let on = format!("on{ty}");
        let g = self.global;
        if let Some(cb) = self.get_prop(g, &on) {
            if self.is_callable(cb) {
                let _ = self.call_value(cb, Value::Obj(g), &[ev])?;
            }
        }
        // Listeners `addEventListener` — sur le nœud racine du
        // dom_ctx courant (même cible que window.*).
        let Some((dom, _)) = self.dom_ctx.clone() else {
            return Ok(());
        };
        let root = dom.borrow().root;
        let cbs: Vec<Value> = dom.borrow().nodes[root as usize]
            .listeners
            .iter()
            .filter(|(t, _, _)| *t == ty)
            .map(|(_, cb, _)| *cb)
            .collect();

        for cb in cbs {
            let _ = self.call_value(cb, Value::Obj(g), &[ev])?;
        }
        Ok(())
    }

    /// `true` si `id` pointe une cellule vivante (utile pour
    /// déboguer les handles hôtes).
    #[must_use]
    pub fn is_alive(&self, id: ObjId) -> bool {
        self.heap.is_alive(id)
    }

    /// `true` si `id` est rooté explicitement.
    #[must_use]
    pub fn is_rooted(&self, id: ObjId) -> bool {
        self.heap.is_rooted(id)
    }

    /// Nombre de cellules vivantes (objets + chaînes) — métrique
    /// pour embedders et tests.
    #[must_use]
    pub fn live_count(&self) -> usize {
        self.heap.live_count()
    }

    /// Passe GC mark-sweep (ADR-0008). À invoquer **entre deux
    /// jobs** — jamais pendant une évaluation : la pile d'appel
    /// n'est pas une racine (les envs capturés par des fonctions
    /// vivantes sont tracés via leurs objets `Fun::Script`).
    ///
    /// Racines semées : objet global + prototypes, registre
    /// `Symbol.for`, env global (bindings `let`/`const`), timers
    /// en attente, file de microtasks, XHR/`fetch` en vol,
    /// callbacks `MutationObserver`, table DOM↔JS (cache +
    /// listeners de nœuds), hooks navigateur, valeur de
    /// complétion — plus les racines explicites posées par
    /// [`Engine::add_root`]. Retourne le nombre de cellules
    /// réclamées ; les host objects balayés reçoivent leur
    /// hook `Host::finalize` avant libération.
    pub fn collect(&mut self) -> usize {
        // 1. Rassemble les `ObjId` tenus par les structures
        //    annexes (RefCell/NodeCache — leurs borrows doivent
        //    être relâchés avant la passe).
        let mut ids: Vec<ObjId> = Vec::new();
        let mut vs = Vec::new();
        self.timers.roots(&mut vs);
        for v in vs {
            crate::heap::push_value(&mut ids, v);
        }
        for (_, wrapper, data) in &self.xhr.live {
            ids.push(*wrapper);
            let mut cbs = Vec::new();
            data.borrow().listener_values(&mut cbs);
            for cb in cbs {
                crate::heap::push_value(&mut ids, cb);
            }
        }
        for (_, promise, _, _) in &self.xhr.fetches {
            ids.push(*promise);
        }
        // Prototypes `Headers`/`Request`/`Response`/`FormData`
        // (JS-C66/C67) — racines tant qu'installés.
        for p in [
            self.xhr.headers_proto,
            self.xhr.req_proto,
            self.xhr.res_proto,
            self.xhr.fd_proto,
        ]
        .into_iter()
        .flatten()
        {
            ids.push(p);
        }
        // Promesses `crypto.subtle` en vol (JS-C79) — contexte
        // courant ; les garés sont tracés dans
        // `BrowsingContext::push_roots`.
        for (_, promise) in &self.crypto.pending {
            ids.push(*promise);
        }
        for (_, job) in &self.microtasks {
            match job {
                crate::promise::Job::Reaction { cb, arg, next, .. } => {
                    crate::heap::push_value(&mut ids, *cb);
                    crate::heap::push_value(&mut ids, *arg);
                    ids.push(*next);
                }
                crate::promise::Job::Thenable {
                    then_fn,
                    this,
                    resolve,
                    reject,
                } => {
                    crate::heap::push_value(&mut ids, *then_fn);
                    crate::heap::push_value(&mut ids, *this);
                    crate::heap::push_value(&mut ids, *resolve);
                    crate::heap::push_value(&mut ids, *reject);
                }
                crate::promise::Job::Plain(v) => crate::heap::push_value(&mut ids, *v),
            }
        }
        // `MutationObserver` (JS-C26) — callbacks + wrappers
        // racines tant que l'engine vit (ré-armement possible).
        self.mutobs.root_ids(&mut ids);
        // `IntersectionObserver`/`ResizeObserver` (JS-C69) —
        // même traitement : callbacks + wrappers enregistrés.
        self.obs.root_ids(&mut ids);
        // `Blob`/`File`/`FileReader` (JS-C70) — protos +
        // readers en cours de lecture (le `Host::trace`
        // couvre ensuite listeners et `result`).
        self.files.root_ids(&mut ids);
        // Streams (JS-C71) — protos + streams avec `read()`
        // pendantes (le `Host::trace` couvre queue et
        // callbacks de source/sink).
        self.streams.root_ids(&mut ids);
        self.idb.root_ids(&mut ids);
        self.ws.root_ids(&mut ids);
        self.es.root_ids(&mut ids);
        self.misc.root_ids(&mut ids);
        if let Some((dom, cache)) = &self.dom_ctx {
            for &id in cache.borrow().values() {
                ids.push(id);
            }
            for node in &dom.borrow().nodes {
                for (_, cb, _) in &node.listeners {
                    crate::heap::push_value(&mut ids, *cb);
                }
            }
            // Contextes 2D `<canvas>` (JS-C28) — détenus par la
            // side-table du miroir, racines tant qu'il vit.
            for v in dom.borrow().canvas_ctx_values() {
                crate::heap::push_value(&mut ids, v);
            }
            // `customElements` (JS-C68) — ctors des définitions,
            // promesses `whenDefined` pendantes.
            for def in dom.borrow().ce.defs.values() {
                crate::heap::push_value(&mut ids, def.ctor);
            }
            for list in dom.borrow().ce.pending.values() {
                ids.extend(list.iter().copied());
            }
        }
        // Élément sous upgrade CE traversant `construct` — il
        // n'est encore rattaché à rien de traçable côté JS.
        if let Some(v) = self.ce_upgrade_slot {
            crate::heap::push_value(&mut ids, v);
        }
        if let Some(s) = self.hooks.screen_obj {
            ids.push(s);
        }
        ids.push(self.global);
        ids.extend(self.protos.all());
        for v in self.symbol_reg.values() {
            crate::heap::push_value(&mut ids, *v);
        }
        for id in self.sym_index.values() {
            ids.push(*id);
        }
        crate::heap::push_value(&mut ids, self.gen_ctor);
        crate::heap::push_value(&mut ids, self.async_gen_ctor);
        crate::heap::push_value(&mut ids, self.gen_fn_proto);
        crate::heap::push_value(&mut ids, self.gen_proto);
        crate::heap::push_value(&mut ids, self.async_gen_fn_proto);
        crate::heap::push_value(&mut ids, self.async_gen_proto);
        crate::heap::push_value(&mut ids, self.completion);
        // 2. Env global — les bindings `let`/`const` ne vivent pas
        //    sur l'objet global — + envs de modules (leurs exports
        //    restent vivants tant que le graphe tient).
        let mut visited = HashSet::new();
        crate::heap::trace_env(&self.global_env, &mut ids, &mut visited);
        for m in &self.modules {
            crate::heap::trace_env(&m.env, &mut ids, &mut visited);
        }
        // JS-C78 : contextes garés — mêmes racines document-local
        // que le courant (`BrowsingContext::push_roots` réplique
        // le bloc ci-dessus), plus le `WindowProxy` stable de
        // chaque contexte (référençable depuis un autre ctx).
        for ctx in self.parked.values_mut() {
            ctx.push_roots(&mut ids, &mut visited);
        }
        for m in &self.ctxs {
            if let Some(p) = m.proxy {
                ids.push(p);
            }
        }
        // 3. Mark-sweep.
        let n = self.heap.collect(|mark| {
            for &id in &ids {
                mark(id);
            }
        });
        // 4. `FinalizationRegistry` — purge des registres morts,
        //    puis cleanup en microtask pour les cibles balayées
        //    (spec `HostEnqueueFinalizationRegistryCleanupJob` :
        //    jamais synchrone).
        let heap = &self.heap;
        self.finregs
            .retain(|&rid| matches!(heap.obj(rid).map(|o| &o.kind), Some(ObjKind::FinReg { .. })));
        let mut pending: Vec<(Value, Value)> = Vec::new();
        for &rid in &self.finregs {
            let Some((cb, entries)) = self.heap.obj_mut(rid).and_then(|o| match &mut o.kind {
                ObjKind::FinReg { cb, entries } => Some((*cb, std::mem::take(entries))),
                _ => None,
            }) else {
                continue;
            };
            let mut keep = Vec::new();
            for en in entries {
                if self.heap.obj(en.target).is_some() {
                    keep.push(en);
                } else {
                    pending.push((cb, en.held));
                }
            }
            if let Some(o) = self.heap.obj_mut(rid) {
                if let ObjKind::FinReg { entries: es, .. } = &mut o.kind {
                    *es = keep;
                }
            }
        }
        for (cb, held) in pending {
            let f = self.bound_native(
                "FinalizationRegistry.cleanup",
                crate::builtins::finreg_cleanup,
                vec![cb, held],
            );
            self.microtasks
                .push_back((self.cur_ctx, crate::promise::Job::Plain(f)));
        }
        n
    }

    /// Parse and run `src`; returns the completion value of the last
    /// expression statement (script-eval style). Microtasks queued by
    /// the script (promise reactions, `queueMicrotask`) are drained
    /// before returning — `eval` is one job boundary.
    ///
    /// # Errors
    /// `Error::Syntax` on parse failure, `Error::Uncaught` if the
    /// script threw and nothing caught it.
    pub fn eval(&mut self, src: &str) -> Result<Value> {
        let prog = parser::parse_in(src, false, self.nest_limit, None)?;
        self.run_program(&prog)
    }

    /// `eval` étiqueté : `name` identifie la source (URL du
    /// `<script src>`, origine…) dans les frames `error.stack`
    /// — attribution par bundle pour le triage de site réel
    /// (JS-C56). Le moteur de session l'appelle avec l'URL.
    ///
    /// # Errors
    /// Identique à [`Engine::eval`].
    pub fn eval_named(&mut self, src: &str, name: &str) -> Result<Value> {
        let prog = parser::parse_in(src, false, self.nest_limit, Some(std::rc::Rc::from(name)))?;
        self.run_program(&prog)
    }

    /// Éval d'un élément `<script>` (CAP-168) : `ord` =
    /// ordinal de l'élément parmi les `<script>` du document
    /// en ordre document — `document.currentScript` le
    /// reflète pendant l'évaluation (spec : scripts
    /// classiques seulement ; modules = `null`), puis redevient
    /// `null`. Élément introuvable (miroir divergé,
    /// `document.write`…) → `currentScript` = `null`, éval
    /// normale — tolérant, jamais de crash.
    ///
    /// # Errors
    /// Identique à [`Engine::eval`].
    pub fn eval_script_el(&mut self, src: &str, name: Option<&str>, ord: usize) -> Result<Value> {
        let el = self.dom_ctx.as_ref().and_then(|(dom, _)| {
            let d = dom.borrow();
            d.by_tag(d.root, "script").get(ord).copied()
        });
        let prev = self.hooks.current_script;
        self.hooks.current_script = el;
        let prog = parser::parse_in(src, false, self.nest_limit, name.map(std::rc::Rc::from));
        let r = match prog {
            Ok(p) => self.run_program(&p),
            Err(e) => Err(e),
        };
        self.hooks.current_script = prev;
        r
    }

    /// Corps partagé d'`eval`/`eval_named` : budget d'ops
    /// ré-armé, programme exécuté, microtasks drainées avant le
    /// retour (une frontière de job).
    fn run_program(&mut self, prog: &crate::ast::Program) -> Result<Value> {
        self.ops_left = self.op_limit;
        let r = match self.exec_program(prog) {
            Ok(v) => Ok(v),
            Err(Thrown(v)) => Err(Error::Uncaught(self.describe(v))),
        };
        if let Err(Thrown(v)) = self.run_microtasks() {
            return Err(Error::Uncaught(self.describe(v)));
        }
        r
    }

    /// Human-readable rendering of a thrown value — les objets
    /// `Error` se décrivent par leur `stack` (JS-C47 : en-tête +
    /// frames), le reste par `Name: message` ou la coercition.
    pub fn describe(&mut self, v: Value) -> String {
        if let Value::Obj(id) = v {
            if let Some(s) = self
                .get_prop(id, "stack")
                .and_then(|s| self.heap_string_of(s))
            {
                if !s.is_empty() {
                    return s;
                }
            }
            let name = self
                .get_prop(id, "name")
                .and_then(|n| self.heap_string_of(n));
            let msg = self
                .get_prop(id, "message")
                .and_then(|m| self.heap_string_of(m));
            if let (Some(n), Some(m)) = (name, msg) {
                return format!("{n}: {m}");
            }
        }
        self.to_js_string(v)
            .unwrap_or_else(|_| "<exception>".into())
    }

    pub(crate) fn heap_string_of(&self, v: Value) -> Option<String> {
        match v {
            Value::Str(id) => self.heap.string(id),
            _ => None,
        }
    }

    // -- allocation helpers -------------------------------------------

    /// Allocate a JS string value.
    pub(crate) fn str_val(&mut self, s: impl Into<String>) -> Value {
        Value::Str(self.heap.alloc_str(s))
    }

    /// Allocate a `bigint` primitive (`ObjKind::BigInt` cell,
    /// `BigInt.prototype` proto — inerte car `Value::Big` ne suit
    /// pas la chaîne de prototypes).
    pub(crate) fn big_val(&mut self, b: num_bigint::BigInt) -> Value {
        let proto = Value::Obj(self.protos.bigint);
        Value::Big(self.heap.alloc_obj(Object {
            proto,
            props: Vec::new(),
            kind: ObjKind::BigInt(b),
            extensible: true,
            elem_integrity: 0,
            args_map: None,
        }))
    }

    /// Digits d'un `Value::Big` — `None` si `v` n'est pas un bigint.
    pub(crate) fn bigint_of(&self, v: Value) -> Option<num_bigint::BigInt> {
        match v {
            Value::Big(id) => match self.heap.obj(id).map(|o| &o.kind) {
                Some(ObjKind::BigInt(b)) => Some(b.clone()),
                _ => None,
            },
            _ => None,
        }
    }

    /// `ToBigInt` — spec : `Number`/`Symbol`/objet non-primitive →
    /// `TypeError` ; chaîne non-entière → `SyntaxError`.
    pub(crate) fn to_bigint(&mut self, v: Value) -> JsResult<num_bigint::BigInt> {
        use num_traits::Zero;
        match v {
            Value::Big(_) => Ok(self.bigint_of(v).unwrap_or_default()),
            Value::Bool(b) => Ok(num_bigint::BigInt::from(u8::from(b))),
            Value::Str(id) => {
                let s = self.heap.string(id).unwrap_or_default();
                let t = s.trim();
                if t.is_empty() {
                    return Ok(num_bigint::BigInt::zero());
                }
                let (body, base) =
                    if let Some(r) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
                        (r, 16)
                    } else if let Some(r) = t.strip_prefix("0o").or_else(|| t.strip_prefix("0O")) {
                        (r, 8)
                    } else if let Some(r) = t.strip_prefix("0b").or_else(|| t.strip_prefix("0B")) {
                        (r, 2)
                    } else {
                        (t, 10)
                    };
                let (sign, digits) = match body.strip_prefix('-') {
                    Some(d) => (-1i8, d),
                    None => (1, body.strip_prefix('+').unwrap_or(body)),
                };
                match num_bigint::BigInt::parse_bytes(digits.as_bytes(), base) {
                    Some(b) => Ok(b * num_bigint::BigInt::from(sign)),
                    None => Err(self.syntax_error("cannot convert string to BigInt")),
                }
            }
            Value::Obj(_) => {
                let p = self.to_primitive(v, PrimHint::Number)?;
                self.to_bigint(p)
            }
            _ => Err(self.type_error("cannot convert to BigInt")),
        }
    }

    /// New ordinary object (`Object.prototype` proto).
    pub(crate) fn new_object(&mut self) -> ObjId {
        let proto = Value::Obj(self.protos.object);
        self.heap.alloc_obj(Object::ordinary(proto))
    }

    /// Fonction native enveloppant une méthode host
    /// (feature-detect) — `args[0]` = nom lié posé par
    /// `bound_native` au site `get_prop_value`, `this` =
    /// receveur host → re-dispatche via `Host::invoke`
    /// (même pile que `el.m()` à l'appel). `Ok(None)` →
    /// `Undefined` (la surface a pu changer entre lecture
    /// et appel).
    fn host_method(&mut self, this: Value, args: &[Value]) -> JsResult<Value> {
        let name = match args.first() {
            Some(Value::Str(s)) => self.heap.string(*s).unwrap_or_default(),
            _ => return Ok(Value::Undefined),
        };
        let Value::Obj(id) = this else {
            return Ok(Value::Undefined);
        };
        let Some(h) = self.host_of(id) else {
            return Ok(Value::Undefined);
        };
        Ok(h.invoke(self, this, &name, &args[1..])?
            .unwrap_or(Value::Undefined))
    }

    /// New array value.
    pub(crate) fn new_array(&mut self, elems: Vec<Value>) -> Value {
        let proto = Value::Obj(self.protos.array);
        Value::Obj(self.heap.alloc_obj(Object::array(elems, proto)))
    }

    /// New native function value.
    pub(crate) fn native_fn(&mut self, name: &'static str, f: crate::heap::NativeFn) -> Value {
        let proto = Value::Obj(self.protos.function);
        Value::Obj(
            self.heap
                .alloc_obj(Object::fun(Fun::Native { name, f }, proto)),
        )
    }

    /// Native function with args prepended (`bound ++ args` per call).
    pub(crate) fn bound_native(
        &mut self,
        name: &'static str,
        f: crate::heap::NativeFn,
        bound: Vec<Value>,
    ) -> Value {
        let proto = Value::Obj(self.protos.function);
        Value::Obj(
            self.heap
                .alloc_obj(Object::fun(Fun::BoundNative { name, f, bound }, proto)),
        )
    }

    /// Attach a non-enumerable native method to `obj`.
    pub(crate) fn add_method(&mut self, obj: ObjId, name: &'static str, f: crate::heap::NativeFn) {
        let fv = self.native_fn(name, f);
        self.set_own(obj, name, fv, false);
    }

    /// Attach a non-enumerable property.
    pub(crate) fn set_own(&mut self, obj: ObjId, key: &str, v: Value, enumerable: bool) {
        if let Some(o) = self.heap.obj_mut(obj) {
            o.set_own(key, v, enumerable);
        }
    }

    /// Attache une constante builtin `{w: false, e: false, c: false}`
    /// (`undefined`, `NaN`, `Infinity`, `Math.PI`, `Number.NaN`, …).
    pub(crate) fn set_const(&mut self, obj: ObjId, key: &str, v: Value) {
        if let Some(o) = self.heap.obj_mut(obj) {
            o.define_data(key, v, crate::heap::PropAttrs::FIXED);
        }
    }

    /// Attache une data prop avec des flags explicites.
    pub(crate) fn set_own_attrs(
        &mut self,
        obj: ObjId,
        key: &str,
        v: Value,
        attrs: crate::heap::PropAttrs,
    ) {
        if let Some(o) = self.heap.obj_mut(obj) {
            o.define_data(key, v, attrs);
        }
    }

    /// Rendu `stack` V8-style (JS-C47) : en-tête `Name: message`
    /// puis une ligne `    at <fn> (l:c)` par frame de la pile
    /// d'appels, appelant le plus récent d'abord — borné à
    /// [`STACK_TRACE_LIMIT`] (V8 `stackTraceLimit` = 10 : une
    /// récursion saturée ne rend pas mille lignes). La position
    /// est celle de la **définition** de la fonction (l'AST ne
    /// porte pas de span par statement — déviation documentée vs
    /// le site d'appel de V8) ; `(0,0)` = fonction synthétique,
    /// non rendue. Les natives n'empilent pas de frame.
    pub(crate) fn stack_text(&self, name: &str, msg: &str) -> String {
        let mut s = if msg.is_empty() {
            name.to_string()
        } else {
            format!("{name}: {msg}")
        };
        for lit in self.call_stack.iter().rev().take(STACK_TRACE_LIMIT) {
            let n = lit.name.as_deref().unwrap_or("<anonymous>");
            s.push_str("\n    at ");
            s.push_str(n);
            if lit.pos.0 > 0 {
                // `src` (nom de module / `<eval>`) préfixe la
                // position — `at f (bundle.js:l:c)` attribue
                // chaque frame à son bundle (JS-C56).
                match &lit.src {
                    Some(src) => s.push_str(&format!(" ({}:{}:{})", src, lit.pos.0, lit.pos.1)),
                    None => s.push_str(&format!(" ({}:{})", lit.pos.0, lit.pos.1)),
                }
            }
        }
        s
    }

    /// New `Error`-family object value (`{name, message}` on `proto`).
    pub(crate) fn error_value(&mut self, proto: ObjId, name: &str, msg: &str) -> Value {
        let id = self.heap.alloc_obj(Object::ordinary(Value::Obj(proto)));
        let n = self.str_val(name);
        let m = self.str_val(msg);
        self.set_own(id, "name", n, false);
        self.set_own(id, "message", m, false);
        // `stack` (own, non-enum) capture la pile d'appels à la
        // construction (JS-C47) — la spécification fige les frames
        // ici, pas au `throw`.
        let s = self.str_val(self.stack_text(name, msg));
        self.set_own(id, "stack", s, false);
        Value::Obj(id)
    }

    pub(crate) fn type_error(&mut self, msg: impl Into<String>) -> Thrown {
        let proto = self.protos.type_error;
        Thrown(self.error_value(proto, "TypeError", &msg.into()))
    }

    pub(crate) fn ref_error(&mut self, msg: impl Into<String>) -> Thrown {
        let proto = self.protos.reference_error;
        Thrown(self.error_value(proto, "ReferenceError", &msg.into()))
    }

    pub(crate) fn range_error(&mut self, msg: impl Into<String>) -> Thrown {
        let proto = self.protos.error;
        Thrown(self.error_value(proto, "RangeError", &msg.into()))
    }

    pub(crate) fn syntax_error(&mut self, msg: impl Into<String>) -> Thrown {
        let proto = self.protos.syntax_error;
        Thrown(self.error_value(proto, "SyntaxError", &msg.into()))
    }

    /// `URIError` — no dedicated proto, hangs off `Error`'s (deviation
    /// documented in ADR-0007; `e.name` still says `URIError`).
    pub(crate) fn uri_error(&mut self, msg: impl Into<String>) -> Thrown {
        let proto = self.protos.error;
        Thrown(self.error_value(proto, "URIError", &msg.into()))
    }

    /// Build a `RegExp` object: compiled pattern as `ObjKind::RegExp`
    /// plus the spec'd own props (`source`, flags, `lastIndex`).
    /// Pattern errors throw a catchable `SyntaxError`.
    pub(crate) fn make_regexp(&mut self, pattern: &str, flags: &str) -> JsResult<Value> {
        let re = match crate::regexp::JsRegExp::compile(pattern, flags) {
            Ok(r) => r,
            Err(msg) => return Err(self.syntax_error(msg)),
        };
        let flag_str = re.flag_string();
        let id = self.heap.alloc_obj(Object {
            proto: Value::Obj(self.protos.regexp),
            props: Vec::new(),
            kind: ObjKind::RegExp(Rc::new(re)),
            extensible: true,
            elem_integrity: 0,
            args_map: None,
        });
        let src = self.str_val(pattern.to_string());
        let fl = self.str_val(flag_str);
        self.set_own(id, "source", src, false);
        self.set_own(id, "flags", fl, false);
        let re = match self.heap.obj(id).map(|o| &o.kind) {
            Some(ObjKind::RegExp(r)) => r.clone(),
            _ => return Err(self.type_error("internal regexp error")),
        };
        self.set_own(id, "global", Value::Bool(re.global), false);
        self.set_own(id, "ignoreCase", Value::Bool(re.ignore_case), false);
        self.set_own(id, "multiline", Value::Bool(re.multiline), false);
        self.set_own(id, "dotAll", Value::Bool(re.dot_all), false);
        self.set_own(id, "sticky", Value::Bool(re.sticky), false);
        self.set_own(id, "unicode", Value::Bool(re.unicode), false);
        self.set_own(id, "hasIndices", Value::Bool(re.has_indices), false);
        self.set_own_attrs(
            id,
            "lastIndex",
            Value::Num(0.0),
            crate::heap::PropAttrs {
                writable: true,
                enumerable: false,
                configurable: false,
            },
        );
        Ok(Value::Obj(id))
    }

    /// Operation budget tick — also counts microtasks so a
    /// self-feeding reaction loop dies like an infinite `while`.
    /// La borne mémoire heap (JS-C37, ADR-0019) est relue ici :
    /// l'alloc lève le drapeau, le `tick` transforme en kill doux.
    pub(crate) fn tick(&mut self) -> JsResult<()> {
        if self.ops_left == 0 {
            return Err(self.range_error("script execution limit exceeded"));
        }
        if self.heap.over_limit() {
            return Err(self.range_error("heap memory limit exceeded"));
        }
        self.ops_left -= 1;
        self.ops_total += 1;
        Ok(())
    }

    /// Borne absolue d'une allocation matérialisée (`vec![x; n]`,
    /// `repeat`, `pad`, `ArrayBuffer`…) — JS-C55. Le `mem_limit`
    /// soft n'est relu qu'au `tick` : sans garde préalable, un
    /// `n` hostile (`new Float64Array(1e10)`) ferait avorter le
    /// processus dans l'allocateur Rust avant tout kill doux.
    /// 256 Mio couvre tout usage réaliste d'un builtin unique.
    pub(crate) const MAX_SINGLE_ALLOC: usize = 256 * 1024 * 1024;

    /// Refuse (`RangeError`) une matérialisation de `bytes` octets
    /// dépassant `min(mem_limit, MAX_SINGLE_ALLOC)` — à appeler
    /// avant tout `vec![…; n]`/`resize`/`repeat` sur un `n` tiré
    /// du script. La multiplication est saturante en amont.
    pub(crate) fn alloc_guard(&mut self, bytes: usize) -> JsResult<()> {
        let lim = match self.heap.mem_limit() {
            0 => Self::MAX_SINGLE_ALLOC,
            m => m.min(Self::MAX_SINGLE_ALLOC),
        };
        if bytes > lim {
            return Err(self.range_error("allocation exceeds engine limits"));
        }
        Ok(())
    }

    /// GC opportuniste aux frontières de jobs (JS-C37) : si le
    /// nombre de cellules vivantes dépasse `gc_threshold`, lance
    /// une passe mark-sweep. Jamais appelé pendant une évaluation
    /// — la pile d'appel n'est pas une racine.
    pub(crate) fn maybe_collect(&mut self) -> usize {
        // `over_limit` aussi : la frontière de job est la première
        // occasion sûre de recalculer `bytes` — si le sweep repasse
        // sous la borne, le drapeau retombe et le job suivant vit.
        if self.heap.over_limit()
            || (self.gc_threshold > 0 && self.heap.live_count() > self.gc_threshold)
        {
            self.collect()
        } else {
            0
        }
    }

    // -- environments --------------------------------------------------

    /// Lookup de `name` le long de la chaîne de portée — les alias
    /// d'import de module renvoient vers l'env exporteur (live
    /// binding) ; `visited` borne les chaînes cycliques
    /// (`export {a} from 'b'` ↔ `export {b} from 'a'`).
    pub(crate) fn env_get(&mut self, env: &EnvRef, name: &str) -> Option<Value> {
        self.env_get_seen(env, name, &mut HashSet::new())
    }

    pub(crate) fn env_get_seen(
        &mut self,
        env: &EnvRef,
        name: &str,
        visited: &mut HashSet<(usize, String)>,
    ) -> Option<Value> {
        let mut cur = Some(env.clone());
        let name = name.to_string();
        while let Some(e) = cur {
            enum Next {
                Up(Option<EnvRef>),
                Alias(EnvRef, String),
            }
            // `with (o)` — la résolution passe par les props de
            // l'objet (hors `@@unscopables`) puis remonte.
            let with_oid = {
                let b = e.borrow();
                match &b.kind {
                    EnvKind::With(oid) => Some(*oid),
                    _ => None,
                }
            };
            if let Some(oid) = with_oid {
                // Traps proxy en échec → nom non lié (tolérance).
                let bound = self.with_has(oid, &name).unwrap_or(false);
                cur = {
                    let b = e.borrow();
                    b.parent.clone()
                };
                if bound {
                    // `get_prop` seul est aveugle aux props
                    // host (`NodeHost` réfléchit `id`/`value`…
                    // via `get`, pas de props propres) —
                    // JS-C45 : `with(el){id}` doit la voir.
                    if let Some(h) = self.host_of(oid) {
                        if let Ok(Some(v)) = h.get(self, Value::Obj(oid), &name) {
                            return Some(v);
                        }
                    }
                    return self.get_prop(oid, &name);
                }
                continue;
            }
            let next = {
                let b = e.borrow();
                match &b.kind {
                    EnvKind::Local(m) | EnvKind::Block(m) => match m.get(&name) {
                        Some(EnvCell::Bind(v)) => return Some(v.value),
                        Some(EnvCell::Alias(t, n)) => Next::Alias(t.clone(), n.clone()),
                        None => Next::Up(b.parent.clone()),
                    },
                    EnvKind::Global(m) => match m.get(&name) {
                        Some(EnvCell::Bind(v)) => return Some(v.value),
                        Some(EnvCell::Alias(t, n)) => Next::Alias(t.clone(), n.clone()),
                        None => {
                            // Declarative record first, then `window`
                            // props — data ET accessors (JS-C78 :
                            // `parent`/`top`/`frames`/`length`/`closed`
                            // sont des getters posés sur le global —
                            // un identifiant libre doit les invoquer).
                            let g = self.global;
                            match self.find_prop(g, &name) {
                                Some(Prop::Data { value, .. }) => return Some(value),
                                Some(Prop::Acc {
                                    get: Value::Obj(gf),
                                    ..
                                }) => {
                                    if let Ok(v) =
                                        self.call_value(Value::Obj(gf), Value::Obj(g), &[])
                                    {
                                        return Some(v);
                                    }
                                    // Getter qui jette → prop non
                                    // liée (la chaîne remonte —
                                    // `ReferenceError` honnête plutôt
                                    // qu'un `undefined` silencieux).
                                }
                                _ => {}
                            }
                            Next::Up(b.parent.clone())
                        }
                    },
                    // Déjà géré au-dessus — repli défensif.
                    EnvKind::With(_) => Next::Up(b.parent.clone()),
                }
            };
            match next {
                Next::Up(p) => cur = p,
                Next::Alias(t, n) => {
                    // Boucle d'alias (ré-exports circulaires) — la
                    // liaison est indéfinie, pas un crash.
                    if !visited.insert((Rc::as_ptr(&t) as usize, n.clone())) {
                        return None;
                    }
                    return self.env_get_seen(&t, &n, visited);
                }
            }
        }
        None
    }

    /// `ToObject` pour `with` — un objet reste lui-même ; une
    /// primitive est enrobée dans un objet-wrapper jetable (les
    /// écritures `with` vont au wrapper, spec `ToObject`).
    /// `null`/`undefined` → `TypeError`.
    pub(crate) fn with_object_of(&mut self, v: Value) -> Option<ObjId> {
        let (proto, own_len, chars): (ObjId, Option<usize>, Option<Vec<String>>) = match v {
            Value::Obj(id) => return Some(id),
            Value::Str(sid) => {
                let s = self.heap.string(sid).unwrap_or_default();
                let cs: Vec<String> = s.chars().map(|c| c.to_string()).collect();
                (self.protos.string, Some(cs.len()), Some(cs))
            }
            Value::Num(_) => (self.protos.number, None, None),
            Value::Bool(_) => (self.protos.boolean, None, None),
            Value::Big(_) => (self.protos.bigint, None, None),
            Value::Undefined | Value::Null | Value::Hole => return None,
        };
        // Wrapper éphémère — jamais observable par le script hors
        // du bloc `with` (aucune référence ne s'en échappe sauf si
        // le bloc l'y place explicitement).
        let w = self.new_object();
        if let Some(o) = self.heap.obj_mut(w) {
            o.proto = Value::Obj(proto);
        }
        if let Some(n) = own_len {
            self.set_own(w, "length", Value::Num(n as f64), false);
        }
        if let Some(cs) = chars {
            for (i, c) in cs.iter().enumerate() {
                let cv = self.str_val(c.clone());
                self.set_own(w, &i.to_string(), cv, true);
            }
        }
        Some(w)
    }

    /// `with (o)` — `o` lierait-il `name` ? Spec
    /// `ObjectEnvironmentRecord.HasBinding` : `name` doit être une
    /// prop de `o` (proto chaine incluse) **et** ne pas être marqué
    /// `@@unscopables[name] === true`.
    pub(crate) fn with_has(&mut self, oid: ObjId, name: &str) -> JsResult<bool> {
        if !self.has_prop(oid, name)? {
            return Ok(false);
        }
        if let Some(Value::Obj(uid)) = self.get_prop(oid, SYM_UNSCOPABLES) {
            if self.get_prop(uid, name).is_some_and(|v| self.to_boolean(v)) {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// `true` si `name` est une liaison d'un env record (local ou
    /// global declarative) — les props de l'objet global ne
    /// comptent pas (elles passent par `[[Delete]]`).
    pub(crate) fn env_has_binding(env: &EnvRef, name: &str) -> bool {
        let mut cur = Some(env.clone());
        while let Some(e) = cur {
            let next = {
                let b = e.borrow();
                match &b.kind {
                    EnvKind::Local(m) | EnvKind::Global(m) | EnvKind::Block(m) => {
                        if m.contains_key(name) {
                            return true;
                        }
                        b.parent.clone()
                    }
                    // `with` n'est pas un record declaratif.
                    EnvKind::With(_) => b.parent.clone(),
                }
            };
            cur = next;
        }
        false
    }

    /// `SetMutableBinding` — résout la liaison, écrit, ou crée le
    /// global implicite sloppy. Erreurs : `TypeError` sur liaison
    /// `const`, `TypeError` strict sur écriture refusée,
    /// `ReferenceError` strict sur identifiant non déclaré.
    pub(crate) fn env_set(&mut self, env: &EnvRef, name: &str, v: Value) -> JsResult<()> {
        let mut cur = Some(env.clone());
        while let Some(e) = cur {
            enum Act {
                Local(EnvRef),
                Global,
                Up(Option<EnvRef>),
            }
            // `with (o)` — écriture sur une prop existante,
            // sinon remontée au scope parent.
            let with_oid = {
                let b = e.borrow();
                match &b.kind {
                    EnvKind::With(oid) => Some(*oid),
                    _ => None,
                }
            };
            if let Some(oid) = with_oid {
                if self.with_has(oid, name)? {
                    // `set_prop_value` (host-aware) plutôt que
                    // `write_prop` : une écriture `with(el)` sur
                    // une prop réfléchie (`value`, `href`…) doit
                    // passer par `h.set`, pas créer une propre
                    // prop fantôme (JS-C45 — `on*="value=…"`).
                    self.set_prop_value(Value::Obj(oid), name, v)?;
                    return Ok(());
                }
                cur = {
                    let b = e.borrow();
                    b.parent.clone()
                };
                continue;
            }
            let act = {
                let b = e.borrow();
                match &b.kind {
                    EnvKind::Local(m) | EnvKind::Block(m) if m.contains_key(name) => {
                        Act::Local(e.clone())
                    }
                    EnvKind::Global(m) if m.contains_key(name) => Act::Local(e.clone()),
                    EnvKind::Global(_) => Act::Global,
                    // `With` déjà géré au-dessus — repli défensif
                    // identique à `Local` sans la clé.
                    EnvKind::Local(_) | EnvKind::Block(_) | EnvKind::With(_) => {
                        Act::Up(b.parent.clone())
                    }
                }
            };
            match act {
                Act::Local(e) => {
                    let mut b = e.borrow_mut();
                    let m = match &mut b.kind {
                        EnvKind::Local(m) | EnvKind::Global(m) | EnvKind::Block(m) => m,
                        // `Act::Local` ne cible jamais un `with`.
                        EnvKind::With(_) => return Ok(()),
                    };
                    if let Some(cell) = m.get_mut(name) {
                        match cell {
                            // Liaison d'import — immuable côté
                            // importeur (les modules sont stricts).
                            EnvCell::Alias(..) => {
                                return Err(
                                    self.type_error(format!("cannot assign to import `{name}`"))
                                );
                            }
                            EnvCell::Bind(bnd) => {
                                if !bnd.mutable {
                                    // Nom de FunctionExpression :
                                    // ReadOnly ES3 — l'écriture
                                    // sloppy est silencieuse, le
                                    // strict jette.
                                    if bnd.sloppy_silent && !self.strict {
                                        return Ok(());
                                    }
                                    return Err(
                                        self.type_error(format!("assignment to constant `{name}`"))
                                    );
                                }
                                bnd.value = v;
                            }
                        }
                    }
                    return Ok(());
                }
                Act::Global => {
                    // L'identifiant se résout via les props de
                    // l'objet global (own + chaîne proto) —
                    // `ReferenceError` strict sinon, écriture
                    // ordinaire si résolu.
                    let g = self.global;
                    if self.find_prop(g, name).is_none() {
                        if self.strict {
                            return Err(self.ref_error(format!("{name} is not defined")));
                        }
                        self.set_own(g, name, v, true);
                        return Ok(());
                    }
                    return self.set_global_prop(name, v);
                }
                Act::Up(p) => cur = p,
            }
        }
        // Identifiant non déclaré : `ReferenceError` en strict,
        // global implicite en sloppy.
        if self.strict {
            return Err(self.ref_error(format!("{name} is not defined")));
        }
        self.set_own(self.global, name, v, true);
        Ok(())
    }

    /// Écriture d'une prop de l'objet global — erreurs strict vs
    /// silence sloppy sur refus (`undefined = 1`, prop gelée…).
    pub(crate) fn set_global_prop(&mut self, name: &str, v: Value) -> JsResult<()> {
        let g = self.global;
        match self.write_prop(g, Value::Obj(g), name, v) {
            Ok(()) => Ok(()),
            Err(t) => Err(t),
        }
    }

    pub(crate) fn env_declare(&mut self, env: &EnvRef, name: &str) {
        let parent = {
            let mut b = env.borrow_mut();
            match &mut b.kind {
                EnvKind::Local(m) => {
                    m.entry(name.to_string()).or_insert(EnvCell::Bind(Binding {
                        value: Value::Undefined,
                        mutable: true,
                        sloppy_silent: false,
                    }));
                    return;
                }
                EnvKind::Global(_) => {
                    // `var` vit sur l'objet global — prop non
                    // configurable (`delete x` → `false`, spec).
                    let g = self.global;
                    drop(b);
                    if self.heap.obj(g).is_some_and(|o| o.get_own(name).is_none()) {
                        self.set_own_attrs(
                            g,
                            name,
                            Value::Undefined,
                            crate::heap::PropAttrs {
                                writable: true,
                                enumerable: true,
                                configurable: false,
                            },
                        );
                    }
                    return;
                }
                // `var` dans un `with`/bloc se déclare dans le scope
                // fonction englobant (spec `VarDeclaredNames`).
                EnvKind::With(_) | EnvKind::Block(_) => b.parent.clone(),
            }
        };
        if let Some(p) = parent {
            self.env_declare(&p, name);
        }
    }

    /// Insert (or overwrite) a binding in `env`'s own map — used by
    /// the class scope for the hidden `super`/`home` names.
    /// `with` envs are skipped — a `with` has no declarative record.
    pub(crate) fn env_put(env: &EnvRef, name: &str, v: Value, mutable: bool) {
        let parent = {
            let mut b = env.borrow_mut();
            match &mut b.kind {
                EnvKind::Local(m) | EnvKind::Global(m) | EnvKind::Block(m) => {
                    m.insert(
                        name.to_string(),
                        EnvCell::Bind(Binding {
                            value: v,
                            mutable,
                            sloppy_silent: false,
                        }),
                    );
                    return;
                }
                EnvKind::With(_) => b.parent.clone(),
            }
        };
        if let Some(p) = parent {
            Self::env_put(&p, name, v, mutable);
        }
    }

    /// `let`/`const`: declare in the *current* scope only, no hoisting
    /// (block scoping is real — a `let` inside `{}` lives in the
    /// block env ; no TDZ: an uninitialised read yields `undefined`
    /// rather than throwing).
    ///
    /// Global `let`/`const` go to the declarative record, *not* the
    /// global object — `window.x` stays `undefined` for `let x`.
    pub(crate) fn env_declare_lex(env: &EnvRef, name: &str, mutable: bool) {
        let parent = {
            let mut b = env.borrow_mut();
            match &mut b.kind {
                EnvKind::Local(m) | EnvKind::Global(m) | EnvKind::Block(m) => {
                    m.insert(
                        name.to_string(),
                        EnvCell::Bind(Binding {
                            value: Value::Undefined,
                            mutable,
                            sloppy_silent: false,
                        }),
                    );
                    return;
                }
                // `with` n'a pas de record — `let`/`const` montent.
                EnvKind::With(_) => b.parent.clone(),
            }
        };
        if let Some(p) = parent {
            Self::env_declare_lex(&p, name, mutable);
        }
    }

    /// Initialize a binding just created by `env_declare_lex` —
    /// bypasses the `const` guard (initialisation is not mutation).
    pub(crate) fn env_init_lex(env: &EnvRef, name: &str, v: Value) {
        let parent = {
            let mut b = env.borrow_mut();
            match &mut b.kind {
                EnvKind::Local(m) | EnvKind::Global(m) | EnvKind::Block(m) => {
                    if let Some(EnvCell::Bind(bnd)) = m.get_mut(name) {
                        bnd.value = v;
                    }
                    return;
                }
                EnvKind::With(_) => b.parent.clone(),
            }
        };
        if let Some(p) = parent {
            Self::env_init_lex(&p, name, v);
        }
    }

    // -- patterns -----------------------------------------------------------

    /// `var`-declare every `Ident` inside `pat` (hoisting support —
    /// `var [a, {b}] = …` declares `a` and `b`).
    pub(crate) fn declare_pat_names(&mut self, env: &EnvRef, pat: &Pat) {
        match pat {
            Pat::Ident(n) => self.env_declare(env, n),
            Pat::Lvalue(_) => {}
            Pat::Rest(p) | Pat::Default(p, _) => self.declare_pat_names(env, p),
            Pat::Array(elems) => {
                for e in elems.iter().flatten() {
                    self.declare_pat_names(env, e);
                }
            }
            Pat::Object { props, rest } => {
                for (_, p) in props {
                    self.declare_pat_names(env, p);
                }
                if let Some(p) = rest {
                    self.declare_pat_names(env, p);
                }
            }
        }
    }

    /// Collecte les `(nom, mutable)` d'un pattern `let`/`const` —
    /// utilisé par `for (let …)` pour recréer les liaisons à chaque
    /// itération (spec `CreatePerIterationEnvironment`).
    pub(crate) fn lex_pat_names(pat: &Pat, mutable: bool, out: &mut Vec<(String, bool)>) {
        match pat {
            Pat::Ident(n) => out.push((n.clone(), mutable)),
            Pat::Lvalue(_) => {}
            Pat::Rest(p) | Pat::Default(p, _) => Self::lex_pat_names(p, mutable, out),
            Pat::Array(elems) => {
                for e in elems.iter().flatten() {
                    Self::lex_pat_names(e, mutable, out);
                }
            }
            Pat::Object { props, rest } => {
                for (_, p) in props {
                    Self::lex_pat_names(p, mutable, out);
                }
                if let Some(p) = rest {
                    Self::lex_pat_names(p, mutable, out);
                }
            }
        }
    }

    /// `let`/`const`-declare every `Ident` inside `pat`.
    pub(crate) fn declare_pat_lex(env: &EnvRef, pat: &Pat, mutable: bool) {
        match pat {
            Pat::Ident(n) => Self::env_declare_lex(env, n, mutable),
            Pat::Lvalue(_) => {}
            Pat::Rest(p) | Pat::Default(p, _) => Self::declare_pat_lex(env, p, mutable),
            Pat::Array(elems) => {
                for e in elems.iter().flatten() {
                    Self::declare_pat_lex(env, e, mutable);
                }
            }
            Pat::Object { props, rest } => {
                for (_, p) in props {
                    Self::declare_pat_lex(env, p, mutable);
                }
                if let Some(p) = rest {
                    Self::declare_pat_lex(env, p, mutable);
                }
            }
        }
    }

    /// Bind `v` to `pat` per `mode` — recursive destructuring.
    ///
    /// # Errors
    /// Propagates evaluation errors from defaults/computed keys and
    /// the not-iterable error of array patterns.
    pub(crate) fn bind_pat(
        &mut self,
        env: &EnvRef,
        this: Value,
        pat: &Pat,
        v: Value,
        mode: BindMode,
    ) -> JsResult<()> {
        match pat {
            Pat::Ident(name) => {
                if name.is_empty() {
                    return Ok(()); // `catch {}` — no binding
                }
                match mode {
                    BindMode::Var => {
                        self.env_declare(env, name);
                        self.env_set(env, name, v)?;
                    }
                    BindMode::Lex { mutable } => {
                        // `declare_pat_lex` may have pre-declared the
                        // name (decl sites); catch/for-of/params bind
                        // directly here. Init bypasses the const
                        // guard (it's not a mutation).
                        Self::env_declare_lex(env, name, mutable);
                        Self::env_init_lex(env, name, v);
                    }
                    BindMode::Assign => return self.env_set(env, name, v),
                }
                Ok(())
            }
            Pat::Lvalue(e) => self.write_lvalue(env, this, e, v),
            Pat::Rest(p) => self.bind_pat(env, this, p, v, mode),
            Pat::Default(p, d) => {
                let v = if matches!(v, Value::Undefined) {
                    self.eval_expr(env, this, d)?
                } else {
                    v
                };
                self.bind_pat(env, this, p, v, mode)
            }
            Pat::Array(elems) => {
                let items = self.iter_values(v)?;
                for (i, el) in elems.iter().enumerate() {
                    match el {
                        None => {}
                        Some(Pat::Rest(p)) => {
                            let rest = self.new_array(items[i..].to_vec());
                            self.bind_pat(env, this, p, rest, mode)?;
                            break;
                        }
                        Some(p) => {
                            let sub = items.get(i).copied().unwrap_or(Value::Undefined);
                            self.bind_pat(env, this, p, sub, mode)?;
                        }
                    }
                }
                Ok(())
            }
            Pat::Object { props, rest } => {
                if matches!(v, Value::Undefined | Value::Null) {
                    return Err(self.type_error("cannot destructure null or undefined"));
                }
                let mut consumed = Vec::with_capacity(props.len());
                for (k, p) in props {
                    let ks = self.eval_prop_key(env, this, k)?;
                    let sub = self.get_prop_value(v, &ks)?;
                    consumed.push(ks);
                    self.bind_pat(env, this, p, sub, mode)?;
                }
                if let Some(p) = rest {
                    // `{...rest}` — remaining enumerable own props.
                    let rest_obj = self.new_object();
                    if let Value::Obj(id) = v {
                        for k in self.own_enum_keys_trapped(id)? {
                            if !consumed.contains(&k) {
                                let sv = self.get_prop_value(v, &k)?;
                                self.set_own(rest_obj, &k, sv, true);
                            }
                        }
                    }
                    self.bind_pat(env, this, p, Value::Obj(rest_obj), mode)?;
                }
                Ok(())
            }
        }
    }

    /// Evaluate a `PropKey` to its string form.
    pub(crate) fn eval_prop_key(
        &mut self,
        env: &EnvRef,
        this: Value,
        k: &PropKey,
    ) -> JsResult<String> {
        match k {
            PropKey::Name(n) => Ok(n.clone()),
            PropKey::Computed(e) => {
                let v = self.eval_expr(env, this, e)?;
                self.prop_key(v)
            }
        }
    }

    /// Eager iteration — `for-of`, array destructuring, `...` spread:
    /// `Symbol.iterator` protocol first, then arrays and strings.
    ///
    /// # Errors
    /// `TypeError` when `v` is not iterable; propagates iterator
    /// callback exceptions. Op-bounded via `tick`.
    pub(crate) fn iter_values(&mut self, v: Value) -> JsResult<Vec<Value>> {
        match v {
            Value::Obj(id) => {
                if let Some(m) = self.get_prop(id, SYM_ITERATOR) {
                    if self.is_callable(m) {
                        let it = self.call_value(m, v, &[])?;
                        let mut out = Vec::new();
                        loop {
                            self.tick()?;
                            let Value::Obj(iid) = it else { break };
                            let Some(nf) = self.get_prop(iid, "next") else {
                                break;
                            };
                            if !self.is_callable(nf) {
                                break;
                            }
                            let r = self.call_value(nf, it, &[])?;
                            let Value::Obj(rid) = r else { break };
                            if self
                                .get_prop(rid, "done")
                                .is_some_and(|d| self.to_boolean(d))
                            {
                                break;
                            }
                            out.push(self.get_prop(rid, "value").unwrap_or(Value::Undefined));
                        }
                        return Ok(out);
                    }
                }
                let ta_view = match self.heap.obj(id).map(|o| &o.kind) {
                    Some(ObjKind::Array(elems)) => return Ok(elems.clone()),
                    Some(ObjKind::TypedArray {
                        ta,
                        buffer: Value::Obj(bid),
                        offset,
                        len,
                    }) => Some((*ta, *bid, *offset, *len)),
                    _ => None,
                };
                if let Some((ta, bid, off, len)) = ta_view {
                    return Ok(self.ta_elements(ta, bid, off, len));
                }
                Err(self.type_error("object is not iterable"))
            }
            Value::Str(id) => {
                let s = self.heap.string(id).unwrap_or_default();
                Ok(s.chars().map(|c| self.str_val(c.to_string())).collect())
            }
            _ => Err(self.type_error("value is not iterable")),
        }
    }

    /// `await v` — unwraps a *settled* promise; a pending one
    /// throws (tree-walk interpreter cannot suspend — documented
    /// deviation). Non-promises pass through (spec wraps, same
    /// observable result).
    ///
    /// # Errors
    /// `Rejected` promise → throws its reason; `Pending` → `TypeError`.
    pub(crate) fn await_value(&mut self, v: Value) -> JsResult<Value> {
        if let Value::Obj(id) = v {
            if let Some(ObjKind::Promise(p)) = self.heap.obj(id).map(|o| &o.kind) {
                let (state, result) = (p.state, p.result);
                return match state {
                    crate::promise::PromiseState::Fulfilled => Ok(result),
                    crate::promise::PromiseState::Rejected => Err(Thrown(result)),
                    // No continuations — `await` on pending
                    // work cannot suspend (documented).
                    crate::promise::PromiseState::Pending => {
                        Err(self.type_error("await on a pending promise is unsupported"))
                    }
                };
            }
        }
        Ok(v)
    }

    /// `for await` semantics — `Symbol.asyncIterator` first
    /// (`next()` result awaited per step), then sync-iterable
    /// fallback with each *value* awaited (spec
    /// `CreateAsyncFromSyncIterator`).
    ///
    /// # Errors
    /// Same as `iter_values` + `await_value` (pending → `TypeError`).
    pub(crate) fn iter_values_await(&mut self, v: Value) -> JsResult<Vec<Value>> {
        if let Value::Obj(id) = v {
            if let Some(m) = self.get_prop(id, SYM_ASYNC_ITERATOR) {
                if self.is_callable(m) {
                    let it = self.call_value(m, v, &[])?;
                    let mut out = Vec::new();
                    loop {
                        self.tick()?;
                        let Value::Obj(iid) = it else { break };
                        let Some(nf) = self.get_prop(iid, "next") else {
                            break;
                        };
                        if !self.is_callable(nf) {
                            break;
                        }
                        let r = self.call_value(nf, it, &[])?;
                        let r = self.await_value(r)?;
                        let Value::Obj(rid) = r else { break };
                        if self
                            .get_prop(rid, "done")
                            .is_some_and(|d| self.to_boolean(d))
                        {
                            break;
                        }
                        out.push(self.get_prop(rid, "value").unwrap_or(Value::Undefined));
                    }
                    return Ok(out);
                }
            }
        }
        let vals = self.iter_values(v)?;
        let mut out = Vec::with_capacity(vals.len());
        for x in vals {
            out.push(self.await_value(x)?);
        }
        Ok(out)
    }

    /// Argument-list evaluation with `...spread` expansion.
    pub(crate) fn eval_args(
        &mut self,
        env: &EnvRef,
        this: Value,
        arg_exprs: &[Expr],
    ) -> JsResult<Vec<Value>> {
        let mut out = Vec::with_capacity(arg_exprs.len());
        for a in arg_exprs {
            match a {
                Expr::Spread(e) => {
                    let v = self.eval_expr(env, this, e)?;
                    out.extend(self.iter_values(v)?);
                }
                _ => out.push(self.eval_expr(env, this, a)?),
            }
        }
        Ok(out)
    }

    // -- classes (JS-7d) ------------------------------------------------

    /// `super.x` base: the `home` hidden binding's prototype —
    /// parent-ctor for the ctor, parent prototype for methods,
    /// the ctor object for statics.
    pub(crate) fn super_base(&mut self, env: &EnvRef) -> JsResult<Value> {
        match self.env_get(env, HOME_BIND) {
            Some(Value::Obj(home)) => Ok(self.heap.obj(home).map_or(Value::Undefined, |o| o.proto)),
            _ => Err(self.type_error("`super` outside a class member")),
        }
    }

    /// Evaluate a `ClassLit` to its constructor value.
    ///
    /// Model: `ctor` is a `Fun::Script`; `ctor.prototype` is an
    /// ordinary object whose proto is `Super.prototype` (or
    /// `Object.prototype`). Hidden env bindings (`SUPER_BIND`,
    /// `HOME_BIND`) give the members' closures access to `super`.
    pub(crate) fn eval_class(
        &mut self,
        env: &EnvRef,
        this: Value,
        cl: &ClassLit,
    ) -> JsResult<Value> {
        // Parent ctor + prototype object for `extends`.
        let (parent, parent_proto) = match &cl.super_ {
            Some(e) => {
                let p = self.eval_expr(env, this, e)?;
                if !self.is_callable(p) {
                    return Err(self.type_error("class extends a non-constructor"));
                }
                let pp = match p {
                    Value::Obj(pid) => match self.get_prop(pid, "prototype") {
                        Some(v @ Value::Obj(_)) => v,
                        _ => Value::Obj(self.protos.object),
                    },
                    _ => Value::Obj(self.protos.object),
                };
                (Some(p), pp)
            }
            None => (None, Value::Obj(self.protos.object)),
        };
        let proto = self.heap.alloc_obj(Object::ordinary(parent_proto));

        // Class scope: hidden `super`/`home` names seen by members.
        let cenv = Env::local(env.clone());
        if let Some(p) = parent {
            Self::env_put(&cenv, SUPER_BIND, p, false);
        }

        // Constructor — explicit member or synthesized
        // (`extends`: `(...a) => super(...a)`, verbatim default).
        let ctor_lit: Rc<FnLit> = cl
            .members
            .iter()
            .find(|m| matches!(m.kind, ClassMemberKind::Ctor))
            .map_or_else(
                || {
                    const REST: &str = "\u{f120}args";
                    Rc::new(FnLit {
                        name: cl.name.clone(),
                        params: vec![Param {
                            pat: Pat::Ident(REST.into()),
                            rest: true,
                        }],
                        body: if parent.is_some() {
                            vec![Stmt::Expr(Expr::SuperCall(vec![Expr::Spread(Box::new(
                                Expr::Ident(REST.into()),
                            ))]))]
                        } else {
                            Vec::new()
                        },
                        is_arrow: false,
                        is_async: false,
                        is_gen: false,
                        strict: true, // corps de classe
                        pos: (0, 0),  // synthétisée — pas de site source
                        src: None,    // synthétisée — pas de source
                    })
                },
                |m| m.fun.clone(),
            );
        let ctor = self.make_function(&ctor_lit, &cenv, this);
        let Value::Obj(ctor_id) = ctor else {
            return Err(self.type_error("internal class error"));
        };
        // `new` path uses `ctor.prototype`; re-point it at ours.
        self.set_own(ctor_id, "prototype", Value::Obj(proto), false);
        self.set_own(proto, "constructor", ctor, false);
        Self::env_put(&cenv, HOME_BIND, ctor, false);
        // Nom propre de la classe — liaison immuable dans le
        // scope de classe (spec `ClassDefinitionEvaluation`),
        // visible des méthodes/statiques/champs : `class e {
        // static m() { return new e() } }` doit résoudre `e` =
        // la classe, pas un binding externe — indispensable pour
        // la *class expression* (trigger primer-react : `var a =
        // class e { … new e() … }` masquant le param `e` des
        // factories webpack). Pour la déclaration, l'environnement
        // externe porte aussi `e` — mais il est mutable et peut
        // être réassigné : le binding interne le masque et reste
        // la classe.
        if let Some(name) = &cl.name {
            Self::env_put(&cenv, name, ctor, false);
        }
        // Derived ctor's [[Prototype]] is the parent constructor —
        // statics inherit (`Parent.sm()` reachable via `C.sm()`,
        // `super.sm()` inside statics resolves through it).
        if let Some(p) = parent {
            if let Some(o) = self.heap.obj_mut(ctor_id) {
                o.proto = p;
            }
        }

        // Members — methods/accessors on the prototype (or ctor for
        // `static`), each closing over a scope with `home` bound.
        // Instance fields are collected (source order) into a
        // `[[key, thunk], ...]` list; `static` fields/blocks run
        // here with `this` = the constructor object.
        let mut fields: Vec<Value> = Vec::new();
        for m in &cl.members {
            if matches!(m.kind, ClassMemberKind::Ctor) {
                continue;
            }
            let target = if m.static_ { ctor_id } else { proto };
            let menv = Env::local(cenv.clone());
            Self::env_put(&menv, HOME_BIND, Value::Obj(target), false);
            let fv = self.make_function(&m.fun, &menv, this);
            match m.kind {
                ClassMemberKind::Field if m.static_ => {
                    let key = self.eval_prop_key(env, this, &m.key)?;
                    let v = self.call_value(fv, ctor, &[])?;
                    self.set_own(ctor_id, &key, v, !key.starts_with(PRIV_PREFIX));
                }
                ClassMemberKind::Field => {
                    let key = self.eval_prop_key(env, this, &m.key)?;
                    let kv = self.str_val(key);
                    let pair = self.new_array(vec![kv, fv]);
                    fields.push(pair);
                }
                ClassMemberKind::StaticBlock => {
                    self.call_value(fv, ctor, &[])?;
                }
                ClassMemberKind::Method => {
                    let key = self.eval_prop_key(env, this, &m.key)?;
                    self.set_own(target, &key, fv, false);
                }
                ClassMemberKind::Getter => {
                    let key = self.eval_prop_key(env, this, &m.key)?;
                    if let Some(o) = self.heap.obj_mut(target) {
                        o.set_accessor(&key, fv, Value::Undefined, false);
                    }
                }
                ClassMemberKind::Setter => {
                    let key = self.eval_prop_key(env, this, &m.key)?;
                    if let Some(o) = self.heap.obj_mut(target) {
                        o.set_accessor(&key, Value::Undefined, fv, false);
                    }
                }
                ClassMemberKind::Ctor => {}
            }
        }
        if !fields.is_empty() {
            let arr = self.new_array(fields);
            let marker = if parent.is_some() {
                FLD_FIELDS_D
            } else {
                FLD_FIELDS
            };
            self.set_own(ctor_id, marker, arr, false);
        }
        // `C()` without `new` -> TypeError (class constructor).
        self.set_own(ctor_id, CLASS_CTOR, Value::Bool(true), false);
        Ok(ctor)
    }

    /// Runs the `[[key, thunk], ...]` list stored under `marker`
    /// on the constructor and installs each field on `this`.
    /// Private-name keys stay non-enumerable; thunks close over
    /// the class scope.
    pub(crate) fn install_class_fields(
        &mut self,
        ctor_id: ObjId,
        marker: &str,
        this: Value,
        nt: Value,
    ) -> JsResult<()> {
        let Some(Prop::Data {
            value: Value::Obj(arr),
            ..
        }) = self.find_own(ctor_id, marker)
        else {
            return Ok(());
        };
        let pairs = self.iter_values(Value::Obj(arr))?;
        for p in pairs {
            let Value::Obj(pid) = p else {
                continue;
            };
            let k = self.get_prop(pid, "0").unwrap_or(Value::Undefined);
            let f = self.get_prop(pid, "1").unwrap_or(Value::Undefined);
            let key = self.prop_key(k)?;
            let v = self.call_value_impl(f, this, &[], nt)?;
            let Value::Obj(tid) = this else {
                return Err(self.type_error("field init without instance"));
            };
            self.set_own(tid, &key, v, !key.starts_with(PRIV_PREFIX));
        }
        Ok(())
    }

    // -- property access ------------------------------------------------

    /// Own-property lookup with array/`length` semantics — returns
    /// the raw `Prop` so accessor pairs are visible to the caller.
    pub(crate) fn find_own(&mut self, id: ObjId, key: &str) -> Option<Prop> {
        // `arguments` mappé : l'indice renvoie au slot env du
        // paramètre formel (spec `MappedArgumentsObject` [[Get]]).
        if let Some(i) = array_index(key) {
            let mapped = self.heap.obj(id).and_then(|o| {
                o.args_map.as_ref().and_then(|(env, map)| {
                    map.get(i).and_then(|n| n.clone().map(|n| (env.clone(), n)))
                })
            });
            if let Some((env, name)) = mapped {
                let v = self.env_get(&env, &name).unwrap_or(Value::Undefined);
                return Some(Prop::Data {
                    value: v,
                    attrs: crate::heap::PropAttrs::FULL,
                });
            }
        }
        let o = self.heap.obj(id)?;
        if let ObjKind::Array(elems) = &o.kind {
            if key == "length" {
                return Some(Prop::Data {
                    value: Value::Num(elems.len() as f64),
                    attrs: crate::heap::PropAttrs {
                        // `length` reste writable sous `seal`,
                        // non-writable sous `freeze` (spec).
                        writable: o.elem_integrity & 4 == 0,
                        enumerable: false,
                        // `arguments.length` est configurable (spec
                        // — `args_map` distingue l'objet arguments).
                        configurable: o.args_map.is_some(),
                    },
                });
            }
            if let Some(i) = array_index(key) {
                return elems
                    .get(i)
                    .copied()
                    .filter(|v| !v.is_hole())
                    .map(|value| Prop::Data {
                        value,
                        attrs: crate::heap::PropAttrs {
                            writable: o.elem_integrity & 2 == 0,
                            enumerable: true,
                            // `seal`/`freeze` → éléments non-config.
                            configurable: o.elem_integrity.trailing_zeros() >= 2,
                        },
                    });
            }
        }
        if let ObjKind::Fun(Fun::Script { lit, .. }) = &o.kind {
            if key == "length" {
                // `fn.length` — params before the first rest/default.
                let n = lit
                    .params
                    .iter()
                    .take_while(|p| !p.rest && !matches!(p.pat, Pat::Default(..)))
                    .count();
                return Some(Prop::Data {
                    value: Value::Num(n as f64),
                    attrs: crate::heap::PropAttrs {
                        writable: false,
                        enumerable: false,
                        configurable: true,
                    },
                });
            }
        }
        // `fn.name` — extraction pure pendant que `o` vit ;
        // matérialisation après le bloc `big` (borrow mort) :
        // `lit.name`, nom natif, ou `bound <cible>` — une vraie
        // prop own `name` (`o.find` ci-dessous) prime, spec
        // configurable:true.
        let (fname, ftgt) = match &o.kind {
            ObjKind::Fun(Fun::Script { lit, .. }) => {
                (Some(lit.name.clone().unwrap_or_default()), None)
            }
            ObjKind::Fun(Fun::Native { name, .. } | Fun::BoundNative { name, .. }) => {
                (Some((*name).to_string()), None)
            }
            ObjKind::Fun(Fun::Bound { target, .. }) => (None, Some(*target)),
            _ => (None, None),
        };
        let ta_tag = match &o.kind {
            ObjKind::TypedArray { ta, .. } => Some(ta.name()),
            _ => None,
        };
        // Statiques Annex B sur le ctor `RegExp` (`$1…$9`, `input`,
        // `lastMatch`…) — extraction pure pendant que `o` vit.
        let re_val = if matches!(self.re_ctor, Value::Obj(rc) if rc == id) {
            self.re_static_get(key)
        } else {
            None
        };
        // Vues binaires — synthèse des props dérivées
        // (`length`/`byteLength`/`byteOffset`/`buffer`) et des
        // éléments indexés, comme pour `Array`.
        if let Some(p) = self.ta_prop(&o.kind, key) {
            return Some(p);
        }
        if let Some(p) = o.find(key) {
            return Some(p);
        }
        // Kinds `BigInt` : la lecture d'un élément alloue un
        // `Value::Big` — d'où le `&mut` de cette méthode.
        let big = match &o.kind {
            ObjKind::TypedArray {
                ta,
                buffer,
                offset,
                len,
            } if ta.is_big() => Some((*ta, *buffer, *offset, *len)),
            _ => None,
        };
        if let (Some((ta, buffer, offset, len)), Some(i)) = (big, array_index(key)) {
            // Cle non-numerique (`name`, `@@toStringTag`...) → tomber
            // sur la synthese ci-dessous ; cle numerique hors-bornes
            // → `None` (proto chain).
            if i >= len {
                return None;
            }
            let Value::Obj(bid) = buffer else { return None };
            let b = {
                let Some(ObjKind::ByteBuf(bytes)) = self.heap.obj(bid).map(|b| &b.kind) else {
                    return None;
                };
                bytes
                    .get(offset..offset + len * ta.size())
                    .map(|region| ta.get_big(region, i))
            };
            return b.map(|b| Prop::Data {
                value: self.big_val(b),
                attrs: crate::heap::PropAttrs {
                    writable: true,
                    enumerable: true,
                    configurable: false,
                },
            });
        }
        // `fn.name` — `o` est mort ici (dernier use : `big`) :
        // allocs et lectures mutables permises.
        // `@@toStringTag` d'une vue typée — nom du kind (spec :
        // getter sur `%TypedArray%.prototype`, rendu ici en data
        // propre — `ta[Symbol.toStringTag]` → `"Int8Array"` etc.).
        if key == SYM_TO_STRING_TAG {
            return ta_tag.map(|n| Prop::Data {
                value: self.str_val(n),
                attrs: crate::heap::PropAttrs {
                    writable: false,
                    enumerable: false,
                    configurable: true,
                },
            });
        }
        if let Some(t) = re_val {
            return Some(Prop::Data {
                value: self.str_val(t),
                attrs: crate::heap::PropAttrs {
                    writable: true,
                    enumerable: false,
                    configurable: true,
                },
            });
        }
        if key == "name" {
            let n = match (fname, ftgt) {
                (Some(n), _) => Some(n),
                (None, Some(t)) => {
                    let tn = self
                        .get_prop_value(t, "name")
                        .ok()
                        .and_then(|v| self.to_js_string(v).ok())
                        .unwrap_or_default();
                    Some(format!("bound {tn}"))
                }
                _ => None,
            };
            return n.map(|n| Prop::Data {
                value: self.str_val(n),
                attrs: crate::heap::PropAttrs {
                    writable: false,
                    enumerable: false,
                    configurable: true,
                },
            });
        }
        None
    }

    /// Propriété synthétisée d'un `ArrayBuffer`/`DataView`/view
    /// typée — `None` si `kind` n'est pas une vue binaire ou `key`
    /// n'est pas une prop dérivée/index valide.
    pub(crate) fn ta_prop(&self, kind: &ObjKind, key: &str) -> Option<Prop> {
        use crate::heap::PropAttrs;
        let fixed = |value| Prop::Data {
            value,
            attrs: PropAttrs {
                writable: false,
                enumerable: false,
                configurable: false,
            },
        };
        match kind {
            ObjKind::ByteBuf(buf) if key == "byteLength" => {
                Some(fixed(Value::Num(buf.len() as f64)))
            }
            ObjKind::DataView {
                buffer,
                offset,
                len,
            } => match key {
                "byteLength" => Some(fixed(Value::Num(*len as f64))),
                "byteOffset" => Some(fixed(Value::Num(*offset as f64))),
                "buffer" => Some(fixed(*buffer)),
                _ => None,
            },
            ObjKind::TypedArray {
                ta,
                buffer,
                offset,
                len,
            } => match key {
                "length" => Some(fixed(Value::Num(*len as f64))),
                "byteLength" => Some(fixed(Value::Num((*len * ta.size()) as f64))),
                "byteOffset" => Some(fixed(Value::Num(*offset as f64))),
                "buffer" => Some(fixed(*buffer)),
                _ => {
                    // Kinds big : résolus dans `find_own` (&mut —
                    // matérialise un `Value::Big`).
                    if ta.is_big() {
                        return None;
                    }
                    let i = array_index(key)?;
                    if i >= *len {
                        return None;
                    }
                    let Value::Obj(bid) = buffer else { return None };
                    let Some(ObjKind::ByteBuf(bytes)) = self.heap.obj(*bid).map(|b| &b.kind) else {
                        return None;
                    };
                    let region = bytes.get(*offset..*offset + *len * ta.size())?;
                    let v = ta.get(region, i);
                    Some(Prop::Data {
                        value: Value::Num(v),
                        attrs: PropAttrs {
                            writable: true,
                            enumerable: true,
                            configurable: false,
                        },
                    })
                }
            },
            _ => None,
        }
    }

    /// Éléments d'une vue typée — `Vec<Value>` (`Num`, ou `Big`
    /// matérialisé pour les kinds big) lus dans la région de vue
    /// du buffer (partagé avec `iter_values` et les méthodes de
    /// `typed_array.rs`).
    pub(crate) fn ta_elements(
        &mut self,
        ta: TaKind,
        bid: ObjId,
        offset: usize,
        len: usize,
    ) -> Vec<Value> {
        let Some(ObjKind::ByteBuf(bytes)) = self.heap.obj(bid).map(|b| &b.kind) else {
            return Vec::new();
        };
        let end = (offset + len * ta.size()).min(bytes.len());
        let region = &bytes[offset.min(end)..end];
        if ta.is_big() {
            let vals: Vec<_> = (0..region.len() / ta.size())
                .map(|i| ta.get_big(region, i))
                .collect();
            return vals.into_iter().map(|b| self.big_val(b)).collect();
        }
        (0..region.len() / ta.size())
            .map(|i| Value::Num(ta.get(region, i)))
            .collect()
    }

    /// First `Prop` found on `id`'s prototype chain.
    pub(crate) fn find_prop(&mut self, id: ObjId, key: &str) -> Option<Prop> {
        let mut cur = Some(id);
        let mut depth = 0;
        while let Some(cid) = cur {
            if let Some(p) = self.find_own(cid, key) {
                return Some(p);
            }
            cur = match self.heap.obj(cid).map(|o| o.proto) {
                Some(Value::Obj(p)) => Some(p),
                _ => None,
            };
            depth += 1;
            if depth > MAX_PROTO_DEPTH {
                return None;
            }
        }
        None
    }

    /// Full prototype-chain lookup — *data* value only. Accessor
    /// reads must go through [`Engine::read_prop`] (the getter may
    /// run script code, needing `&mut self`); internal callers get
    /// `None` for accessor props.
    pub(crate) fn get_prop(&mut self, id: ObjId, key: &str) -> Option<Value> {
        self.find_prop(id, key).and_then(|p| p.data_value())
    }

    /// Property read with accessor invocation — `o.x` where `x` has a
    /// getter calls it with `this = recv` (differs from `base` only
    /// for `super.x` reads).
    pub(crate) fn read_prop(&mut self, base: Value, recv: Value, key: &str) -> JsResult<Value> {
        let Value::Obj(id) = base else {
            return self.get_prop_value(base, key);
        };
        if let Some((tid, target, handler)) = self.proxy_th(id)? {
            let trap = self.proxy_trap(handler, "get")?;
            if matches!(trap, Value::Obj(_)) {
                let kv = self.str_val(key);
                let v = self.call_value(trap, handler, &[target, kv, recv])?;
                // Invariants spec `[[Get]]` : data non-config
                // non-writable → valeur imposée ; accesseur
                // non-config sans getter → `undefined` imposé.
                match self.find_own(tid, key) {
                    Some(Prop::Data { value, attrs }) if !attrs.configurable && !attrs.writable => {
                        if !self.strict_eq(v, value) {
                            return Err(self.type_error(
                                "proxy get: result differs from non-configurable non-writable prop",
                            ));
                        }
                    }
                    Some(Prop::Acc { get, attrs, .. })
                        if !attrs.configurable
                            && matches!(get, Value::Undefined)
                            && !matches!(v, Value::Undefined) =>
                    {
                        return Err(self.type_error(
                            "proxy get: non-configurable accessor without getter must yield undefined",
                        ));
                    }
                    _ => {}
                }
                return Ok(v);
            }
            return self.read_prop(target, recv, key);
        }
        match self.find_prop(id, key) {
            Some(Prop::Data { value, .. }) => Ok(value),
            Some(Prop::Acc { get, .. }) => {
                if self.is_callable(get) {
                    self.call_value(get, recv, &[])
                } else {
                    Ok(Value::Undefined)
                }
            }
            None => Ok(Value::Undefined),
        }
    }

    /// Property write with setter invocation — `o.x = v` where `x`
    /// is an accessor on the chain calls the setter instead of
    /// shadowing it.
    pub(crate) fn write_prop(
        &mut self,
        id: ObjId,
        recv: Value,
        key: &str,
        v: Value,
    ) -> JsResult<()> {
        if let Some((tid, target, handler)) = self.proxy_th(id)? {
            if self
                .proxy_set_step(tid, target, handler, recv, key, v)?
                .is_some()
            {
                return Ok(());
            }
            return self.write_prop(tid, recv, key, v);
        }
        // Accessors on the chain intercept the write.
        let mut cur = Some(id);
        let mut depth = 0;
        while let Some(cid) = cur {
            // Un proxy *au milieu* de la chaîne intercepte aussi
            // — même logique qu'en tête (`Object.setPrototypeOf(
            // Symbol.prototype, spy)` suivi de `Symbol().x = v`
            // atteint la trap, spec `OrdinarySet` : chaque
            // maillon applique son propre `[[Set]]`).
            if let Some((tid, target, handler)) = self.proxy_th(cid)? {
                if self
                    .proxy_set_step(tid, target, handler, recv, key, v)?
                    .is_some()
                {
                    return Ok(());
                }
                cur = Some(tid);
                depth += 1;
                if depth > MAX_PROTO_DEPTH {
                    break;
                }
                continue;
            }
            match self.find_own(cid, key) {
                Some(Prop::Acc { set, .. }) => {
                    if self.is_callable(set) {
                        self.call_value(set, recv, &[v])?;
                        return Ok(());
                    }
                    // Accessor sans setter → écriture refusée.
                    return self.write_refused(key);
                }
                // Data prop non-writable *héritée* → l'écriture est
                // refusée (pas de shadowing possible, spec [[Set]]).
                Some(Prop::Data { attrs, .. }) if cid != id && !attrs.writable => {
                    return self.write_refused(key);
                }
                Some(_) => break, // data writable → shadow with own write
                None => {}
            }
            cur = match self.heap.obj(cid).map(|o| o.proto) {
                Some(Value::Obj(p)) => Some(p),
                _ => None,
            };
            depth += 1;
            if depth > MAX_PROTO_DEPTH {
                break;
            }
        }
        if self.set_prop(id, key, v)? {
            Ok(())
        } else {
            self.write_refused(key)
        }
    }

    /// Étape `[[Set]]` d'un maillon proxy : invoque la trap
    /// `set` (`receiver` = `recv`) puis vérifie les invariants
    /// spec — data non-config non-writable → seule la *même*
    /// valeur rapporte succès ; accesseur non-config sans
    /// setter → jamais de succès. Retourne `Some(())` quand le
    /// sort est tranché (trap appliquée ou refus enregistré
    /// via `write_refused`), `None` sans trap — l'appelant
    /// poursuit alors sur la cible `tid`.
    fn proxy_set_step(
        &mut self,
        tid: ObjId,
        target: Value,
        handler: Value,
        recv: Value,
        key: &str,
        v: Value,
    ) -> JsResult<Option<()>> {
        let trap = self.proxy_trap(handler, "set")?;
        if !matches!(trap, Value::Obj(_)) {
            return Ok(None);
        }
        let kv = self.str_val(key);
        let ok = self.call_value(trap, handler, &[target, kv, v, recv])?;
        if !self.to_boolean(ok) {
            return self.write_refused(key).map(Some);
        }
        match self.find_own(tid, key) {
            Some(Prop::Data { value, attrs })
                if !attrs.configurable && !attrs.writable && !self.strict_eq(v, value) =>
            {
                return Err(self.type_error(
                    "proxy set: cannot report success on non-configurable non-writable prop",
                ));
            }
            Some(Prop::Acc { set, attrs, .. })
                if !attrs.configurable && matches!(set, Value::Undefined) =>
            {
                return Err(self.type_error("proxy set: non-configurable accessor without setter"));
            }
            _ => {}
        }
        Ok(Some(()))
    }

    /// Écriture refusée (`[[Set]]` → `false`) — `TypeError` en
    /// strict, silence en sloppy.
    pub(crate) fn write_refused(&mut self, key: &str) -> JsResult<()> {
        if self.strict {
            Err(self.type_error(format!("cannot write `{key}`")))
        } else {
            Ok(())
        }
    }

    /// Annex B — statiques du ctor `RegExp`. `None` si `key` n'est
    /// pas une statique ; `Some("")` quand la statique est vide
    /// (aucun match enregistré — V8 rend `""`, pas `undefined`).
    pub(crate) fn re_static_get(&self, key: &str) -> Option<String> {
        let st = self.re_static.as_ref();
        let v = match key {
            "input" | "$_" => st.map(|r| r.input.clone()),
            "lastMatch" | "$&" => st.and_then(|r| r.groups.first().cloned().flatten()),
            "lastParen" | "$+" => st.and_then(|r| {
                r.groups
                    .get(1..)
                    .unwrap_or(&[])
                    .iter()
                    .rev()
                    .find_map(Clone::clone)
            }),
            "leftContext" | "$`" => st.map(|r| r.input[..r.start_b.min(r.input.len())].to_string()),
            "rightContext" | "$'" => st.map(|r| r.input[r.end_b.min(r.input.len())..].to_string()),
            _ if key.len() == 2
                && key.starts_with('$')
                && key.as_bytes()[1].is_ascii_digit()
                && key.as_bytes()[1] != b'0' =>
            {
                st.and_then(|r| {
                    r.groups
                        .get((key.as_bytes()[1] - b'0') as usize)
                        .cloned()
                        .flatten()
                })
            }
            _ => return None,
        };
        Some(v.unwrap_or_default())
    }

    /// `UpdateRegExpStatics` (Annex B) — mémorise le dernier match
    /// (`None` sur échec : les statiques se vident, comme V8).
    pub(crate) fn note_re(&mut self, input: &str, m: Option<&crate::regexp::ExecMatch>) {
        self.re_static = m.map(|m| ReStatic {
            input: input.to_string(),
            start_b: m.start_b,
            end_b: m.end_b,
            groups: m.groups.clone(),
        });
    }

    /// `key` est-elle une prop propre **non configurable** de `id` ?
    /// (`None` = prop absente — utilisé par les invariants proxy.)
    pub(crate) fn non_config_own(&mut self, id: ObjId, key: &str) -> Option<bool> {
        self.find_own(id, key).map(|p| match &p {
            Prop::Data { attrs, .. } | Prop::Acc { attrs, .. } => !attrs.configurable,
        })
    }

    /// `Object.isExtensible`-niveau interne (invariants proxy).
    pub(crate) fn target_extensible(&self, id: ObjId) -> bool {
        self.heap.obj(id).is_some_and(|o| o.extensible)
    }

    /// Extract the host delegate of an object, if any.
    pub(crate) fn host_of(&self, id: ObjId) -> Option<Rc<dyn crate::heap::Host>> {
        match self.heap.obj(id).map(|o| &o.kind) {
            Some(ObjKind::Host(h)) => Some(h.clone()),
            _ => None,
        }
    }

    /// `id` est-il un proxy ? → `(target_id, target, handler)` ;
    /// `TypeError` sur proxy révoqué (spec : toute méthode interne
    /// d'un `[[ProxyHandler]]` nul lève).
    pub(crate) fn proxy_th(&mut self, id: ObjId) -> JsResult<Option<(ObjId, Value, Value)>> {
        let (target, handler) = match self.heap.obj(id).map(|o| &o.kind) {
            Some(ObjKind::Proxy { target, handler }) => (*target, *handler),
            _ => return Ok(None),
        };
        let Some(handler) = handler else {
            return Err(self.type_error("cannot perform operation on a revoked proxy"));
        };
        let Value::Obj(tid) = target else {
            return Err(self.type_error("invalid proxy target"));
        };
        Ok(Some((tid, target, handler)))
    }

    /// Lookup d'une trap sur le handler : fonction callable,
    /// `Undefined` si absente/null, `TypeError` si non-callable.
    pub(crate) fn proxy_trap(&mut self, handler: Value, name: &str) -> JsResult<Value> {
        let f = self.get_prop_value(handler, name)?;
        match f {
            Value::Undefined | Value::Null => Ok(Value::Undefined),
            _ if self.is_callable(f) => Ok(f),
            _ => Err(self.type_error(format!("proxy trap `{name}` is not callable"))),
        }
    }

    /// Allocate a host object (used by `install_dom`, and by embedders).
    pub fn new_host_object(&mut self, host: Rc<dyn crate::heap::Host>) -> Value {
        let proto = Value::Obj(self.protos.object);
        Value::Obj(self.heap.alloc_obj(Object {
            proto,
            props: Vec::new(),
            kind: ObjKind::Host(host),
            extensible: true,
            elem_integrity: 0,
            args_map: None,
        }))
    }

    /// `new_host_object` + own-prop `Symbol.iterator` (JS-C76) : la
    /// valeur est `for-of`/spread-compatible et passe la détection
    /// `a[Symbol.iterator]` des helpers `makeIterator` des bundles
    /// closure (déclencheur : `Error: b` google.com). La native
    /// relit [`crate::heap::Host::iter_items`] à chaque appel —
    /// collection vivante ; le host doit implémenter `iter_items`.
    pub fn new_iterable_host(&mut self, host: Rc<dyn crate::heap::Host>) -> Value {
        let v = self.new_host_object(host);
        if let Value::Obj(id) = v {
            let it = self.native_fn("[Symbol.iterator]", host_sym_iterator);
            self.set_own(id, SYM_ITERATOR, it, false);
        }
        v
    }

    /// `base[key]` on any value — wraps primitives via their prototype.
    pub(crate) fn get_prop_value(&mut self, base: Value, key: &str) -> JsResult<Value> {
        match base {
            Value::Undefined | Value::Null | Value::Hole => {
                Err(self.type_error(format!("cannot read `{key}` of {base:?}")))
            }
            Value::Obj(id) => {
                let h = self.host_of(id);
                if let Some(h) = &h {
                    if let Some(v) = h.get(self, base, key)? {
                        return Ok(v);
                    }
                }
                let v = self.read_prop(base, base, key)?;
                // Feature-detect : les méthodes host sont
                // invoke-only — quand ni `get` ni la chaîne
                // proto ne produisent de valeur, exposer le
                // nom comme fonction lisible (re-dispatch via
                // `Host::invoke` à l'appel) : `typeof el.m`
                // = 'function', shims `o = el.matches || …`.
                // Une own-prop scriptée du même nom continue de
                // primer (elle aurait été trouvée par `get`/
                // `read_prop`).
                if matches!(v, Value::Undefined) {
                    if let Some(h) = &h {
                        if h.has_method(key) {
                            let name = self.str_val(key);
                            return Ok(self.bound_native(
                                "hostMethod",
                                Self::host_method,
                                vec![name],
                            ));
                        }
                    }
                }
                Ok(v)
            }
            Value::Str(id) => {
                if key == "length" {
                    // Code units UTF-16 (spec) — astraux = 2.
                    let n = self.heap.string(id).map_or(0, |s| s.encode_utf16().count());
                    return Ok(Value::Num(n as f64));
                }
                if let Some(i) = array_index(key) {
                    // Indice en code units — surrogate isolé →
                    // U+FFFD (déviation documentée : UTF-8 natif).
                    let c = self.heap.string(id).and_then(|s| {
                        s.encode_utf16()
                            .nth(i)
                            .map(|u| String::from_utf16_lossy(&[u]))
                    });
                    return Ok(match c {
                        Some(c) => self.str_val(c),
                        None => Value::Undefined,
                    });
                }
                let proto = self.protos.string;
                self.read_prop(Value::Obj(proto), base, key)
            }
            // `read_prop` plutôt que `get_prop` : les accesseurs
            // hérités (`__proto__`, getters de proto) doivent être
            // invoqués avec `this` = la primitive (spec `[[Get]]`
            // sur wrapper éphémère).
            Value::Num(_) => {
                let proto = self.protos.number;
                self.read_prop(Value::Obj(proto), base, key)
            }
            Value::Bool(_) => {
                let proto = self.protos.boolean;
                self.read_prop(Value::Obj(proto), base, key)
            }
            Value::Big(_) => {
                let proto = self.protos.bigint;
                self.read_prop(Value::Obj(proto), base, key)
            }
        }
    }

    /// `base[key] = v`.
    pub(crate) fn set_prop_value(&mut self, base: Value, key: &str, v: Value) -> JsResult<()> {
        match base {
            Value::Undefined | Value::Null => {
                Err(self.type_error(format!("cannot set `{key}` of {base:?}")))
            }
            Value::Obj(id) => {
                if let Some(h) = self.host_of(id) {
                    if h.set(self, base, key, v)? {
                        // Réactions CE synchrones (JS-C68) — un
                        // setter host a pu muter le miroir
                        // (`innerHTML`, `className`…).
                        self.drain_ce_reactions()?;
                        return Ok(());
                    }
                }
                self.write_prop(id, base, key, v)
            }
            // `[[Set]]` sur base primitive (spec `PutValue` →
            // `OrdinarySet` du wrapper éphémère) : la chaîne
            // proto de la primitive peut intercepter l'écriture
            // — setter hérité ou proxy `set` amarré via
            // `Object.setPrototypeOf(String.prototype, p)` —
            // invoquée avec `receiver` = la primitive. Sans
            // interception l'écriture est refusée (une
            // primitive ne porte jamais de prop propre) :
            // silence sloppy, `TypeError` strict.
            _ => {
                let proto = match base {
                    Value::Str(_) => Some(self.protos.string),
                    Value::Num(_) => Some(self.protos.number),
                    Value::Bool(_) => Some(self.protos.boolean),
                    Value::Big(_) => Some(self.protos.bigint),
                    _ => None,
                };
                if let Some(p) = proto {
                    if self.prim_set_intercept(p, base, key, v)? {
                        return Ok(());
                    }
                }
                self.write_refused(key)
            }
        }
    }

    /// Chaîne proto d'un receiver primitif pour `[[Set]]` :
    /// seuls les intercepteurs jouent (accesseur avec setter,
    /// proxy `set`) — jamais de shadowing propre (une
    /// primitive ne crée pas de prop). `true` = écriture
    /// prise en charge ; `false` = à refuser.
    fn prim_set_intercept(
        &mut self,
        mut cur: ObjId,
        recv: Value,
        key: &str,
        v: Value,
    ) -> JsResult<bool> {
        let mut depth = 0;
        loop {
            if let Some((tid, target, handler)) = self.proxy_th(cur)? {
                if self
                    .proxy_set_step(tid, target, handler, recv, key, v)?
                    .is_some()
                {
                    return Ok(true);
                }
                // Sans trap `set`, la cible poursuit la marche
                // (ses props propres puis sa chaîne).
                cur = tid;
            } else {
                match self.find_own(cur, key) {
                    Some(Prop::Acc { set, .. }) => {
                        if self.is_callable(set) {
                            self.call_value(set, recv, &[v])?;
                            return Ok(true);
                        }
                        return Ok(false);
                    }
                    // Data prop trouvée sur la chaîne → le receiver
                    // primitif ne peut créer d'own → refus.
                    Some(Prop::Data { .. }) => return Ok(false),
                    None => {}
                }
                cur = match self.heap.obj(cur).map(|o| o.proto) {
                    Some(Value::Obj(p)) => p,
                    _ => return Ok(false),
                };
            }
            depth += 1;
            if depth > MAX_PROTO_DEPTH {
                return Ok(false);
            }
        }
    }

    /// `[[Set]]` sur un own data prop — `false` si refusé
    /// (non-writable, non-extensible, frozen). Le caller décide
    /// silence sloppy vs `TypeError` strict.
    pub(crate) fn set_prop(&mut self, id: ObjId, key: &str, v: Value) -> JsResult<bool> {
        // Élément indexé d'une vue typée : la cible est le *buffer*
        // partagé, pas la vue ; hors-bornes canonique → ignoré
        // (spec `IntegerIndexedElementSet` retourne `true`).
        // Kinds big : `ToBigInt` — écrire un Number dans un
        // `BigInt64Array` lève `TypeError` (spec), d'où le
        // `JsResult` (les conversions échouées doivent pouvoir
        // se propager même sloppy).
        let ta_t = match self.heap.obj(id).map(|o| &o.kind) {
            Some(ObjKind::TypedArray {
                ta,
                buffer,
                offset,
                len,
            }) => array_index(key).map(|i| (*ta, *buffer, *offset, i, i < *len)),
            _ => None,
        };
        if let Some((ta, Value::Obj(bid), off, i, in_range)) = ta_t {
            if !in_range {
                return Ok(true);
            }
            if ta.is_big() {
                let b = self.to_bigint(v)?;
                if let Some(bo) = self.heap.obj_mut(bid) {
                    if let ObjKind::ByteBuf(bytes) = &mut bo.kind {
                        if off <= bytes.len() {
                            ta.set_big(&mut bytes[off..], i, &b);
                        }
                    }
                }
                return Ok(true);
            }
            // `ToNumber` propage — `u8[0] = 1n` lève `TypeError`
            // (spec `SetValueInBuffer`), comme un `valueOf` qui lève.
            let n = self.to_number(v)?;
            if let Some(b) = self.heap.obj_mut(bid) {
                if let ObjKind::ByteBuf(bytes) = &mut b.kind {
                    if off <= bytes.len() {
                        ta.set(&mut bytes[off..], i, n);
                    }
                }
            }
            return Ok(true);
        }
        // `arguments` mappé : l'écriture d'indice va au slot env du
        // paramètre (spec `MappedArgumentsObject` [[Set]]).
        if let Some(i) = array_index(key) {
            let mapped = self.heap.obj(id).and_then(|o| {
                o.args_map.as_ref().and_then(|(env, map)| {
                    map.get(i).and_then(|n| n.clone().map(|n| (env.clone(), n)))
                })
            });
            if let Some((env, name)) = mapped {
                self.env_set(&env, &name, v)?;
                return Ok(true);
            }
        }
        // Garde d'alloc AVANT le `&mut` heap : écrire `a[2^32-1]=x`
        // ou `a.length = 4e9` densifierait le Vec d'éléments (~64
        // Gio) — `alloc_guard` refuse avant l'allocateur (JS-C55).
        let grow = match self.heap.obj(id).map(|o| &o.kind) {
            Some(ObjKind::Array(elems)) => {
                let need = if key == "length" {
                    match v {
                        Value::Num(n) if n >= 0.0 => Some(to_uint32(n) as usize),
                        _ => None,
                    }
                } else {
                    array_index(key).map(|i| i + 1)
                };
                need.and_then(|n| n.checked_sub(elems.len()))
            }
            _ => None,
        };
        if let Some(d) = grow.filter(|d| *d > 0) {
            self.alloc_guard(d.saturating_mul(std::mem::size_of::<Value>()))?;
        }
        let Some(o) = self.heap.obj_mut(id) else {
            return Ok(false);
        };
        if let ObjKind::Array(elems) = &mut o.kind {
            if key == "length" {
                // `length` verrouillé (`freeze`) → écriture refusée.
                if o.elem_integrity & 4 != 0 {
                    return Ok(false);
                }
                let n = match v {
                    Value::Num(n) if n >= 0.0 => to_uint32(n) as usize,
                    _ => elems.len(),
                };
                // Réduction sous `seal`/`freeze` : la spec supprime
                // les éléments un à un → élément non-configurable
                // → `TypeError` abrupt (même en sloppy, `Set` jette).
                if o.elem_integrity & 3 != 0 && n < elems.len() {
                    return Err(self.type_error("cannot shrink a sealed array"));
                }
                elems.resize(n, Value::Hole);
                return Ok(true);
            }
            if let Some(i) = array_index(key) {
                // Élément gelé → écriture refusée (spec [[Set]]).
                if o.elem_integrity & 2 != 0 && i < elems.len() {
                    return Ok(false);
                }
                // Ajout hors bornes → met à jour `length` — refusé
                // si cible non-extensible ou `length` verrouillée.
                if i >= elems.len() && (!o.extensible || o.elem_integrity & 4 != 0) {
                    return Ok(false);
                }
                if i >= elems.len() {
                    elems.resize(i + 1, Value::Hole);
                }
                elems[i] = v;
                return Ok(true);
            }
        }
        if let Some((_, p)) = o.props.iter_mut().find(|(k, _)| k == key) {
            match p {
                crate::heap::Prop::Data { value, attrs } => {
                    if !attrs.writable {
                        return Ok(false);
                    }
                    *value = v;
                    Ok(true)
                }
                // Setter sans set → l'écriture échoue (spec).
                crate::heap::Prop::Acc { set, .. } => Ok(matches!(*set, Value::Obj(_))),
            }
        } else if o.extensible {
            o.props.push((
                key.to_string(),
                crate::heap::Prop::Data {
                    value: v,
                    attrs: crate::heap::PropAttrs::FULL,
                },
            ));
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub(crate) fn has_prop(&mut self, id: ObjId, key: &str) -> JsResult<bool> {
        if let Some((tid, target, handler)) = self.proxy_th(id)? {
            let trap = self.proxy_trap(handler, "has")?;
            if matches!(trap, Value::Obj(_)) {
                let kv = self.str_val(key);
                let r = self.call_value(trap, handler, &[target, kv])?;
                if !self.to_boolean(r) {
                    // Invariants spec `[[HasProperty]]` : prop
                    // non-config ou cible non-extensible → la prop
                    // ne peut pas être cachée.
                    match self.non_config_own(tid, key) {
                        Some(true) => {
                            return Err(
                                self.type_error("proxy has: cannot hide non-configurable prop")
                            );
                        }
                        Some(false) if !self.target_extensible(tid) => {
                            return Err(self.type_error(
                                "proxy has: cannot hide prop on non-extensible target",
                            ));
                        }
                        _ => {}
                    }
                }
                return Ok(self.to_boolean(r));
            }
            return self.has_prop(tid, key);
        }
        if let Some(h) = self.host_of(id) {
            if h.get(self, Value::Obj(id), key)?.is_some() {
                return Ok(true);
            }
        }
        Ok(self.find_prop(id, key).is_some())
    }

    pub(crate) fn delete_prop(&mut self, id: ObjId, key: &str) -> bool {
        // Élément de vue typée — non configurable dans la plage
        // (spec : `delete ta[i]` → `false`, hors-bornes → `true`).
        if let Some(ObjKind::TypedArray { len, .. }) = self.heap.obj(id).map(|o| &o.kind) {
            if let Some(i) = array_index(key) {
                return i >= *len;
            }
        }
        if let Some(o) = self.heap.obj_mut(id) {
            if let ObjKind::Array(elems) = &mut o.kind {
                if key == "length" {
                    return false;
                }
                if let Some(i) = array_index(key) {
                    // `seal`/`freeze` → éléments non-configurables,
                    // la suppression est refusée (spec).
                    if o.elem_integrity & 3 != 0 && i < elems.len() {
                        return false;
                    }
                    // `arguments` mappé : `delete` retire le
                    // mapping, puis l'élément (spec).
                    if let Some((_, map)) = o.args_map.as_mut() {
                        if let Some(m) = std::rc::Rc::get_mut(map).and_then(|m| m.get_mut(i)) {
                            *m = None;
                        }
                    }
                    if i < elems.len() {
                        elems[i] = Value::Hole;
                    }
                    return true;
                }
            }
            return o.remove_own(key);
        }
        false
    }

    /// `delete o.k` → `false` si refusé ; en strict un refus est un
    /// `TypeError` (spec `[[Delete]]` + strict Delete).
    pub(crate) fn delete_result(&mut self, id: ObjId, key: &str) -> JsResult<Value> {
        if let Some((tid, target, handler)) = self.proxy_th(id)? {
            let trap = self.proxy_trap(handler, "deleteProperty")?;
            let ok = if matches!(trap, Value::Obj(_)) {
                let kv = self.str_val(key);
                let r = self.call_value(trap, handler, &[target, kv])?;
                self.to_boolean(r)
            } else {
                return self.delete_result(tid, key);
            };
            if ok {
                // Invariant spec `[[Delete]]` : une prop non-config
                // ne peut pas être rapportée supprimée.
                if matches!(self.non_config_own(tid, key), Some(true)) {
                    return Err(self
                        .type_error("proxy deleteProperty: cannot delete non-configurable prop"));
                }
            }
            return if ok {
                Ok(Value::Bool(true))
            } else if self.strict {
                Err(self.type_error(format!("cannot delete `{key}`")))
            } else {
                Ok(Value::Bool(false))
            };
        }
        if self.delete_prop(id, key) {
            Ok(Value::Bool(true))
        } else if self.strict {
            Err(self.type_error(format!("cannot delete `{key}`")))
        } else {
            Ok(Value::Bool(false))
        }
    }

    /// `===` exposed for builtins (no user code runs).
    pub(crate) fn strict_eq(&self, l: Value, r: Value) -> bool {
        strict_eq(l, r, &self.heap)
    }

    /// *All* own keys (`Reflect.ownKeys`, `getOwnPropertyNames`):
    /// array indices + props regardless of enumerability.
    pub(crate) fn own_keys(&self, id: ObjId) -> Vec<String> {
        let mut out = Vec::new();
        let mut rest = Vec::new();
        if let Some(o) = self.heap.obj(id) {
            match &o.kind {
                ObjKind::Array(elems) => {
                    for (i, v) in elems.iter().enumerate() {
                        if !v.is_hole() {
                            out.push(i.to_string());
                        }
                    }
                }
                ObjKind::TypedArray { len, .. } => {
                    for i in 0..*len {
                        out.push(i.to_string());
                    }
                }
                _ => {}
            }
            for (k, _) in &o.props {
                rest.push(k.clone());
            }
        }
        out.extend(spec_key_order(rest));
        out
    }

    /// Enumerable *own* keys (`Object.keys`): array indices + props.
    pub(crate) fn own_enum_keys(&self, id: ObjId) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(o) = self.heap.obj(id) {
            match &o.kind {
                ObjKind::Array(elems) => {
                    for (i, v) in elems.iter().enumerate() {
                        if !v.is_hole() {
                            out.push(i.to_string());
                        }
                    }
                }
                ObjKind::TypedArray { len, .. } => {
                    for i in 0..*len {
                        out.push(i.to_string());
                    }
                }
                _ => {}
            }
            let mut rest = Vec::new();
            for (k, p) in &o.props {
                // Clés-symboles sentinelles : jamais énumérées.
                if p.enumerable() && !k.starts_with(SYM_PREFIX) {
                    rest.push(k.clone());
                }
            }
            out.extend(spec_key_order(rest));
        }
        out
    }

    /// `[[OwnPropertyKeys]]` trap-aware (`Reflect.ownKeys`,
    /// `getOwnPropertyNames`) — la trap `ownKeys` doit rendre un
    /// tableau de strings dédupliquées (spec : `TypeError` sinon).
    pub(crate) fn own_keys_trapped(&mut self, id: ObjId) -> JsResult<Vec<String>> {
        if let Some((tid, target, handler)) = self.proxy_th(id)? {
            let trap = self.proxy_trap(handler, "ownKeys")?;
            if matches!(trap, Value::Obj(_)) {
                let r = self.call_value(trap, handler, &[target])?;
                let Value::Obj(arr) = r else {
                    return Err(self.type_error("proxy ownKeys trap must return an array"));
                };
                let elems = match self.heap.obj(arr).map(|o| &o.kind) {
                    Some(ObjKind::Array(e)) => e.clone(),
                    _ => Vec::new(),
                };
                let mut out = Vec::new();
                for v in elems {
                    let k = self.prop_key(v)?;
                    if out.contains(&k) {
                        return Err(self.type_error("proxy ownKeys trap returned duplicate key"));
                    }
                    out.push(k);
                }
                // Invariants spec `[[OwnPropertyKeys]]` : toutes les
                // clés non-config de la cible doivent figurer ;
                // cible non-extensible → la liste doit être exacte.
                let mut required: Vec<String> = Vec::new();
                for k in self.own_keys(tid) {
                    match self.non_config_own(tid, &k) {
                        Some(true) => {
                            if !out.contains(&k) {
                                return Err(
                                    self.type_error("proxy ownKeys: missing non-configurable key")
                                );
                            }
                        }
                        _ => required.push(k),
                    }
                }
                if !self.target_extensible(tid)
                    && (out.len() != required.len() || out.iter().any(|k| !required.contains(k)))
                {
                    return Err(self
                        .type_error("proxy ownKeys: must list all keys of non-extensible target"));
                }
                return Ok(out);
            }
            return self.own_keys_trapped(tid);
        }
        Ok(self.own_keys(id))
    }

    /// `[[GetOwnProperty]]` trap-aware — la trap rend un objet
    /// descripteur ou `undefined` (spec : `TypeError` sinon).
    pub(crate) fn get_own_prop(&mut self, id: ObjId, key: &str) -> JsResult<Option<Prop>> {
        if let Some((tid, target, handler)) = self.proxy_th(id)? {
            let trap = self.proxy_trap(handler, "getOwnPropertyDescriptor")?;
            if matches!(trap, Value::Obj(_)) {
                let kv = self.str_val(key);
                let r = self.call_value(trap, handler, &[target, kv])?;
                return match r {
                    // Invariant : prop non-config ne peut pas être
                    // rapportée absente ; cible non-extensible idem.
                    Value::Undefined => match self.non_config_own(tid, key) {
                        Some(true) => Err(self.type_error(
                            "proxy getOwnPropertyDescriptor: cannot omit non-configurable prop",
                        )),
                        Some(false) if !self.target_extensible(tid) => Err(self.type_error(
                            "proxy getOwnPropertyDescriptor: cannot omit prop on non-extensible target",
                        )),
                        _ => Ok(None),
                    },
                    Value::Obj(d) => {
                        let p = self.prop_from_desc_obj(d)?;
                        let rconf = p.as_ref().map(|p| match p {
                            Prop::Data { attrs, .. } | Prop::Acc { attrs, .. } => attrs.configurable,
                        });
                        match (self.non_config_own(tid, key), rconf) {
                            // Rapporter non-config une prop absente.
                            (None, Some(false)) => {
                                return Err(self.type_error(
                                    "proxy getOwnPropertyDescriptor: cannot report non-configurable for absent prop",
                                ));
                            }
                            // Rapporter non-config une prop configurable.
                            (Some(false), Some(false)) => {
                                return Err(self.type_error(
                                    "proxy getOwnPropertyDescriptor: cannot report configurable prop as non-configurable",
                                ));
                            }
                            // Nouvelle prop sur cible non-extensible.
                            (None, _) if !self.target_extensible(tid) => {
                                return Err(self.type_error(
                                    "proxy getOwnPropertyDescriptor: cannot add prop on non-extensible target",
                                ));
                            }
                            _ => {}
                        }
                        Ok(p)
                    }
                    _ => Err(self.type_error(
                        "proxy getOwnPropertyDescriptor trap must return object or undefined",
                    )),
                };
            }
            return self.get_own_prop(tid, key);
        }
        Ok(self.find_own(id, key))
    }

    /// Objet descripteur `{value,writable,enumerable,configurable}`
    /// ou `{get,set,…}` → `Prop` (résultat de la trap
    /// `getOwnPropertyDescriptor`).
    pub(crate) fn prop_from_desc_obj(&mut self, d: ObjId) -> JsResult<Option<Prop>> {
        let dv = Value::Obj(d);
        let enumerable = {
            let v = self.get_prop_value(dv, "enumerable")?;
            self.to_boolean(v)
        };
        let configurable = {
            let v = self.get_prop_value(dv, "configurable")?;
            self.to_boolean(v)
        };
        let get = self.get_prop_value(dv, "get")?;
        let set = self.get_prop_value(dv, "set")?;
        let is_acc = !matches!(get, Value::Undefined) || !matches!(set, Value::Undefined);
        if is_acc {
            return Ok(Some(Prop::Acc {
                get,
                set,
                attrs: crate::heap::PropAttrs {
                    writable: false,
                    enumerable,
                    configurable,
                },
            }));
        }
        let value = self.get_prop_value(dv, "value")?;
        let writable = {
            let v = self.get_prop_value(dv, "writable")?;
            self.to_boolean(v)
        };
        Ok(Some(Prop::Data {
            value,
            attrs: crate::heap::PropAttrs {
                writable,
                enumerable,
                configurable,
            },
        }))
    }

    /// Enumerable own keys trap-aware (`Object.keys`, spreads,
    /// `for-in` head) — `ownKeys` puis filtrage par le descripteur.
    pub(crate) fn own_enum_keys_trapped(&mut self, id: ObjId) -> JsResult<Vec<String>> {
        let mut out = Vec::new();
        for k in self.own_keys_trapped(id)? {
            if self.get_own_prop(id, &k)?.is_some_and(|p| p.enumerable()) {
                out.push(k);
            }
        }
        Ok(out)
    }

    /// Enumerable keys for `for-in`: host keys, array indices, then
    /// enumerable own props, then enumerable proto-chain props
    /// (deduplicated).
    pub(crate) fn enum_keys(&mut self, id: ObjId) -> JsResult<Vec<String>> {
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        if let Some(h) = self.host_of(id) {
            for k in h.keys(self, Value::Obj(id)) {
                if seen.insert(k.clone()) {
                    out.push(k);
                }
            }
        }
        let mut cur = Some(id);
        let mut depth = 0;
        while let Some(cid) = cur {
            if matches!(
                self.heap.obj(cid).map(|o| &o.kind),
                Some(ObjKind::Proxy { .. })
            ) {
                // Proxy : ses props propres passent par les traps ;
                // la chaîne repart du proto du *target*.
                for k in self.own_enum_keys_trapped(cid)? {
                    if seen.insert(k.clone()) {
                        out.push(k);
                    }
                }
                cur = match self.proxy_th(cid)? {
                    Some((tid, _, _)) => match self.heap.obj(tid).map(|o| o.proto) {
                        Some(Value::Obj(p)) => Some(p),
                        _ => None,
                    },
                    None => None,
                };
            } else if let Some(o) = self.heap.obj(cid) {
                if let ObjKind::Array(elems) = &o.kind {
                    for (i, v) in elems.iter().enumerate() {
                        if !v.is_hole() && seen.insert(i.to_string()) {
                            out.push(i.to_string());
                        }
                    }
                }
                let pkeys: Vec<String> = o
                    .props
                    .iter()
                    .filter(|(k, p)| p.enumerable() && !k.starts_with(SYM_PREFIX))
                    .map(|(k, _)| k.clone())
                    .collect();
                for k in spec_key_order(pkeys) {
                    if seen.insert(k.clone()) {
                        out.push(k);
                    }
                }
                cur = match o.proto {
                    Value::Obj(p) => Some(p),
                    _ => None,
                };
            } else {
                cur = None;
            }
            depth += 1;
            if depth > MAX_PROTO_DEPTH {
                break;
            }
        }
        Ok(out)
    }

    // -- coercions -------------------------------------------------------

    /// `ToBoolean` (pure, no user code runs).
    pub(crate) fn to_boolean(&self, v: Value) -> bool {
        match v {
            Value::Undefined | Value::Null | Value::Hole => false,
            Value::Bool(b) => b,
            Value::Num(n) => n != 0.0 && !n.is_nan(),
            // `0n` falsy, tout le reste truthy.
            Value::Big(_) => self
                .bigint_of(v)
                .is_some_and(|b| !num_traits::Zero::is_zero(&b)),
            Value::Str(id) => self.heap.string(id).is_some_and(|s| !s.is_empty()),
            Value::Obj(_) => true,
        }
    }

    /// `ToPrimitive` — may run `@@toPrimitive`/`valueOf`/`toString`.
    /// `hint` suit la spec : `Number` pour `ToNumber`/arithmétique,
    /// `String` pour `ToString`, `Default` pour `+` et `==`.
    pub(crate) fn to_primitive(&mut self, v: Value, hint: PrimHint) -> JsResult<Value> {
        let Value::Obj(id) = v else {
            return Ok(v);
        };
        // `ToPrimitive(symbole)` → le symbole lui-même (spec) —
        // les `valueOf`/`toString` du proto ne s'appliquent pas
        // à la conversion (c'est `Symbol.prototype[@@toPrimitive]`).
        if matches!(
            self.heap.obj(id).map(|o| &o.kind),
            Some(ObjKind::Symbol { .. })
        ) {
            return Ok(v);
        }
        // `@@toPrimitive(hint)` prend le pas (spec `ToPrimitive`).
        if let Some(f) = self.get_prop(id, SYM_TO_PRIMITIVE) {
            if self.is_callable(f) {
                let h = match hint {
                    PrimHint::String => "string",
                    PrimHint::Number => "number",
                    PrimHint::Default => "default",
                };
                let hv = self.str_val(h.to_string());
                let r = self.call_value(f, v, &[hv])?;
                if r.is_object() {
                    return Err(self.type_error("Symbol.toPrimitive must return a primitive"));
                }
                return Ok(r);
            }
        }
        let order: [&str; 2] = if matches!(hint, PrimHint::String) {
            ["toString", "valueOf"]
        } else {
            ["valueOf", "toString"]
        };
        for m in order {
            if let Some(f) = self.get_prop(id, m) {
                if self.is_callable(f) {
                    let r = self.call_value(f, v, &[])?;
                    if !r.is_object() {
                        return Ok(r);
                    }
                }
            }
        }
        Err(self.type_error("cannot convert object to primitive"))
    }

    /// `InstanceofOperator(V, target)` complet : dispatch
    /// `@@hasInstance` puis `OrdinaryHasInstance` (spec). Partagé
    /// entre `instanceof` et la délégation des fonctions bound.
    pub(crate) fn instance_of(&mut self, v: Value, target: Value) -> JsResult<bool> {
        let Value::Obj(_) = target else {
            return Err(self.type_error("`instanceof` requires an object"));
        };
        // `@@hasInstance` d'abord (spec : le hook prime sur
        // l'exigence callable).
        let m = self.get_prop_value(target, SYM_HAS_INSTANCE)?;
        if self.is_callable(m) {
            let b = self.call_value(m, target, &[v])?;
            return Ok(self.to_boolean(b));
        }
        if !self.is_callable(target) {
            return Err(self.type_error("right side of `instanceof` is not callable"));
        }
        self.ordinary_has_instance(target, v)
    }

    /// `OrdinaryHasInstance(C, O)` : chaîne de prototypes de `O`
    /// contre `C.prototype`. Une fonction bound délègue à sa cible
    /// via `InstanceofOperator` (spec `BoundFunction`
    /// `[[HasInstance]]`); `prototype` non-objet → `TypeError`.
    pub(crate) fn ordinary_has_instance(&mut self, c: Value, v: Value) -> JsResult<bool> {
        if !self.is_callable(c) {
            return Ok(false);
        }
        if let Value::Obj(id) = c {
            let bound = match self.heap.obj(id).map(|o| &o.kind) {
                Some(ObjKind::Fun(Fun::Bound { target, .. })) => Some(*target),
                _ => None,
            };
            if let Some(target) = bound {
                return self.instance_of(v, target);
            }
        }
        // Lecture via `get_prop_value` : `prototype` peut passer par
        // une trap `get` de proxy (spec `OrdinaryHasInstance` fait un
        // `[[Get]]`).
        let Value::Obj(proto) = self.get_prop_value(c, "prototype")? else {
            return Err(self.type_error("constructor has no prototype"));
        };
        let mut cur = match v {
            Value::Obj(id) => match self.heap.obj(id).map(|o| o.proto) {
                Some(Value::Obj(p)) => Some(p),
                _ => None,
            },
            _ => return Ok(false),
        };
        let mut depth = 0;
        while let Some(id) = cur {
            if id == proto {
                return Ok(true);
            }
            cur = match self.heap.obj(id).map(|o| o.proto) {
                Some(Value::Obj(p)) => Some(p),
                _ => None,
            };
            depth += 1;
            if depth > MAX_PROTO_DEPTH {
                break;
            }
        }
        Ok(false)
    }

    pub(crate) fn is_callable(&self, v: Value) -> bool {
        match v {
            Value::Obj(id) => match self.heap.obj(id).map(|o| &o.kind) {
                Some(ObjKind::Fun(_)) => true,
                // Proxy : callable ssi sa cible l'est (révoqué → non).
                Some(ObjKind::Proxy { target, handler }) => {
                    handler.is_some() && self.is_callable(*target)
                }
                _ => false,
            },
            _ => false,
        }
    }

    /// `ToNumber`.
    pub(crate) fn to_number(&mut self, v: Value) -> JsResult<f64> {
        Ok(match v {
            Value::Undefined | Value::Hole => f64::NAN,
            Value::Null => 0.0,
            Value::Bool(b) => f64::from(u8::from(b)),
            Value::Num(n) => n,
            Value::Str(id) => parse_js_number(&self.heap.string(id).unwrap_or_default()),
            // `ToNumber(bigint)` → TypeError (spec) — `Number(1n)`
            // passe par `to_numeric` côté builtin, pas ici.
            Value::Big(_) => return Err(self.type_error("cannot convert a BigInt to number")),
            Value::Obj(id) => {
                // `ToNumber(Symbol)` throws — spec TypeError.
                if matches!(
                    self.heap.obj(id).map(|o| &o.kind),
                    Some(ObjKind::Symbol { .. })
                ) {
                    return Err(self.type_error("cannot convert a Symbol to number"));
                }
                let p = self.to_primitive(v, PrimHint::Number)?;
                self.to_number(p)?
            }
        })
    }

    /// `ToString`.
    pub(crate) fn to_js_string(&mut self, v: Value) -> JsResult<String> {
        Ok(match v {
            Value::Undefined | Value::Hole => "undefined".into(),
            Value::Null => "null".into(),
            Value::Bool(b) => if b { "true" } else { "false" }.into(),
            Value::Num(n) => num_to_js_string(n),
            Value::Str(id) => self.heap.string(id).unwrap_or_default(),
            Value::Big(_) => self.bigint_of(v).map(|b| b.to_string()).unwrap_or_default(),
            Value::Obj(id) => {
                // `ToString(symbol)` → TypeError (spec `ToString`).
                if matches!(
                    self.heap.obj(id).map(|o| &o.kind),
                    Some(ObjKind::Symbol { .. })
                ) {
                    return Err(self.type_error("cannot convert a Symbol to string"));
                }
                let p = self.to_primitive(v, PrimHint::String)?;
                self.to_js_string(p)?
            }
        })
    }

    /// `ToPropertyKey` — les symboles rendent leur clé unique
    /// (deux `Symbol('x')` distincts indexent deux clés
    /// différentes, spec identité).
    pub(crate) fn prop_key(&mut self, v: Value) -> JsResult<String> {
        match v {
            Value::Num(n) => Ok(num_to_js_string(n)),
            Value::Obj(id) => match self.heap.obj(id).map(|o| &o.kind) {
                Some(ObjKind::Symbol { key, .. }) => Ok(key.clone()),
                _ => self.to_js_string(v),
            },
            _ => self.to_js_string(v),
        }
    }

    /// `Symbol(desc)` anonyme — clé `\u{f130}sym{N}` unique.
    pub(crate) fn new_symbol(&mut self, desc: Option<String>) -> Value {
        let key = format!("\u{f130}sym{}", self.sym_seq);
        self.sym_seq += 1;
        self.new_symbol_key(desc, key)
    }

    /// Symbole à clé explicite (well-knowns, `Symbol.for`) —
    /// indexé dans `sym_index` pour `getOwnPropertySymbols`.
    pub(crate) fn new_symbol_key(&mut self, desc: Option<String>, key: String) -> Value {
        let proto = Value::Obj(self.protos.symbol);
        let id = self.heap.alloc_obj(crate::heap::Object {
            proto,
            props: Vec::new(),
            kind: ObjKind::Symbol {
                desc,
                key: key.clone(),
            },
            extensible: true,
            elem_integrity: 0,
            args_map: None,
        });
        self.sym_index.insert(key, id);
        Value::Obj(id)
    }

    // -- calls & construction --------------------------------------------

    /// `f.apply(this, args)` with full callable semantics.
    /// Appel ordinaire `f(args)` — vérifie la marque classe :
    /// `C()` sur un constructeur de classe → `TypeError` (spec, les
    /// classes refusent l'appel sans `new`). `new`/`super()` passent
    /// par [`call_value_ctor`].
    pub(crate) fn call_value(&mut self, f: Value, this: Value, args: &[Value]) -> JsResult<Value> {
        if let Value::Obj(fid) = f {
            if self.find_own(fid, CLASS_CTOR).is_some() {
                return Err(self.type_error("class constructor cannot be invoked without `new`"));
            }
        }
        self.call_value_impl(f, this, args, Value::Undefined)
    }

    /// Appel en contexte de construction (`new`, `super(...)`) —
    /// la marque de constructeur de classe y est attendue.
    pub(crate) fn call_value_ctor(
        &mut self,
        f: Value,
        this: Value,
        args: &[Value],
        nt: Value,
    ) -> JsResult<Value> {
        self.call_value_impl(f, this, args, nt)
    }

    /// Dispatch d'une builtin native : compte dans `call_depth`
    /// (la récursion native `join → to_primitive → join` n'était
    /// pas bornée → débordement de pile, JS-C55) et protège la
    /// pile native via `maybe_grow` comme le corps Script.
    pub(crate) fn call_native(
        &mut self,
        f: crate::heap::NativeFn,
        this: Value,
        args: &[Value],
    ) -> JsResult<Value> {
        self.call_depth += 1;
        if self.call_depth > self.call_depth_limit {
            self.call_depth -= 1;
            return Err(self.range_error("maximum call stack exceeded"));
        }
        let r = stacker::maybe_grow(
            crate::parser::STACK_RED_ZONE,
            crate::parser::STACK_GROW,
            || f(self, this, args),
        );
        self.call_depth -= 1;
        r
    }

    pub(crate) fn call_value_impl(
        &mut self,
        f: Value,
        this: Value,
        args: &[Value],
        nt: Value,
    ) -> JsResult<Value> {
        let Value::Obj(id) = f else {
            return Err(self.type_error("value is not a function"));
        };
        // Proxy callable : trap `apply` ou délégation à la cible.
        if let Some((_, target, handler)) = self.proxy_th(id)? {
            let trap = self.proxy_trap(handler, "apply")?;
            if matches!(trap, Value::Obj(_)) {
                let argv = self.new_array(args.to_vec());
                return self.call_value(trap, handler, &[target, this, argv]);
            }
            return self.call_value(target, this, args);
        }
        let fun = match self.heap.obj(id) {
            Some(o) => match &o.kind {
                ObjKind::Fun(fu) => fu.clone(),
                _ => return Err(self.type_error("object is not a function")),
            },
            None => return Err(self.type_error("value is not a function")),
        };
        match fun {
            Fun::Native { f, .. } => self.call_native_ctx(f, this, args),
            Fun::BoundNative { f, bound, .. } => {
                let mut a = Vec::with_capacity(bound.len() + args.len());
                a.extend_from_slice(&bound);
                a.extend_from_slice(args);
                self.call_native_ctx(f, this, &a)
            }
            Fun::Bound {
                target,
                this: bound_this,
                bound,
            } => {
                let mut a = Vec::with_capacity(bound.len() + args.len());
                a.extend_from_slice(&bound);
                a.extend_from_slice(args);
                self.call_value(target, bound_this, &a)
            }
            Fun::Script { lit, env, lex_this } => {
                // JS-C78 : l'appel s'exécute dans le realm de la
                // FONCTION — son env capturée racine sur le
                // `global_env` de son contexte (« job callback
                // realm » spec). Switch transactionnel restauré
                // à la sortie — les natives des host objects
                // emportent leurs `Rc` (DomRef/NodeCache), les
                // objets same-origin restent donc valides.
                let prev = self.cur_ctx;
                if let Some(t) = self.ctx_of_env(&env) {
                    self.switch_to(t);
                }
                let r = self.call_script_body(f, &lit, env, lex_this, this, args, nt);
                if self.cur_ctx != prev {
                    self.switch_to(prev);
                }
                r
            }
        }
    }

    /// Appel d'une native avec switch de contexte (JS-C78) :
    /// `frame.eval(...)`/`frame.setTimeout(...)` — `this` = le
    /// global d'un autre contexte → la native s'exécute dans SON
    /// realm (même règle que `Fun::Script`). `native_caller` garde
    /// le contexte appelant — `postMessage` en a besoin pour sa
    /// `source`/`origin` (incumbent realm spec : l'appelant, pas
    /// la cible).
    fn call_native_ctx(
        &mut self,
        f: crate::heap::NativeFn,
        this: Value,
        args: &[Value],
    ) -> JsResult<Value> {
        let target = match this {
            Value::Obj(id) => self.ctx_of_objid(id),
            _ => None,
        };
        let Some(t) = target else {
            return self.call_native(f, this, args);
        };
        if t == self.cur_ctx {
            return self.call_native(f, this, args);
        }
        let prev = self.cur_ctx;
        self.native_caller.push(prev);
        self.switch_to(t);
        let r = self.call_native(f, this, args);
        self.native_caller.pop();
        if self.cur_ctx != prev {
            self.switch_to(prev);
        }
        r
    }

    /// Corps d'un appel `Fun::Script` — extrait de
    /// [`Self::call_value_impl`] pour permettre le switch de
    /// contexte transactionnel (JS-C78) autour de l'exécution.
    #[allow(clippy::too_many_arguments)] // corps extrait de
                                         // `call_value_impl` — même paramétrage d'appel (JS-C78).
    fn call_script_body(
        &mut self,
        f: Value,
        lit: &std::rc::Rc<crate::ast::FnLit>,
        env: EnvRef,
        lex_this: Option<Value>,
        this: Value,
        args: &[Value],
        nt: Value,
    ) -> JsResult<Value> {
        self.call_depth += 1;
        // JS-C41 : borne logique d'appels — la pile elle-
        // même est protégée par `maybe_grow` au corps.
        if self.call_depth > self.call_depth_limit {
            self.call_depth -= 1;
            return Err(self.range_error("maximum call stack exceeded"));
        }
        // JS-C47 : la frame est empilée APRÈS la borne —
        // un appel refusé ne laisse pas de résidu dans
        // `error.stack`/`Uncaught`.
        self.call_stack.push(lit.clone());
        // Arrow functions ignore the caller's receiver —
        // `this` was captured at creation time. En sloppy,
        // `this` null/undefined est coercé vers l'objet
        // global (spec § OrdinaryCallBindThis).
        let this = lex_this.unwrap_or(this);
        let this = if lit.strict || matches!(this, Value::Obj(_)) {
            this
        } else {
            Value::Obj(self.global)
        };
        let outer_strict = self.strict;
        self.strict = lit.strict;
        let local = Env::local(env);
        // Param binding: patterns + `= default` + `...rest`.
        for (i, p) in lit.params.iter().enumerate() {
            let v = if p.rest {
                self.new_array(args.get(i..).unwrap_or(&[]).to_vec())
            } else {
                args.get(i).copied().unwrap_or(Value::Undefined)
            };
            // Param binding runs inside the new scope; a
            // destructuring/throwing default aborts the call.
            if let Err(t) = self.bind_pat(&local, this, &p.pat, v, BindMode::Var) {
                self.call_depth -= 1;
                self.call_stack.pop();
                return Err(t);
            }
        }
        // Arrows have no own `arguments` — lookup falls
        // through to the enclosing scope, as specced.
        // Annex B sloppy : `arguments.callee` → la fonction
        // (strict : absent — déviation vs le poisoned
        // accessor de la spec, la lecture rend `undefined`).
        let argv = (!lit.is_arrow).then(|| {
            let a = self.new_array(args.to_vec());
            if let Value::Obj(aid) = a {
                // `arguments` mappé (spec
                // `CreateMappedArgumentsObject`) : fonction
                // sloppy à liste de paramètres simple —
                // `map[i]` nomme le slot env de l'indice `i`.
                // Map vide sinon (objet `arguments` non
                // mappé, `length.configurable` quand même).
                let simple = lit
                    .params
                    .iter()
                    .all(|p| !p.rest && matches!(p.pat, crate::ast::Pat::Ident(_)));
                let map: Vec<Option<String>> = if !lit.strict && simple {
                    (0..args.len().min(lit.params.len()))
                        .map(|i| match &lit.params[i].pat {
                            crate::ast::Pat::Ident(n) => Some(n.clone()),
                            _ => None,
                        })
                        .collect()
                } else {
                    Vec::new()
                };
                if let Some(o) = self.heap.obj_mut(aid) {
                    o.args_map = Some((local.clone(), std::rc::Rc::new(map)));
                }
                if !lit.strict {
                    self.set_own(aid, "callee", f, false);
                }
            }
            a
        });
        {
            let mut b = local.borrow_mut();
            if let (EnvKind::Local(m), Some(argv)) = (&mut b.kind, argv) {
                m.insert(
                    "arguments".into(),
                    EnvCell::Bind(Binding {
                        value: argv,
                        mutable: true,
                        sloppy_silent: false,
                    }),
                );
            }
        }
        // Les flèches n'ont pas de `new.target` propre —
        // la lecture remonte lexicalement au ctor englobant
        // (spec : `new.target` est une MetaProperty du
        // contexte d'exécution englobant, pas de l'arrow).
        if !lit.is_arrow {
            Self::env_put(&local, NEW_TARGET, nt, false);
        }
        self.hoist(&lit.body, &local);
        // Générateur eager : le corps tourne à l'appel en
        // empilant un collecteur ; chaque `yield` pousse sa
        // valeur (déviation — pas de suspension).
        if lit.is_gen {
            self.gen_collect.push(Vec::new());
        }
        // JS-C41 : un appel JS coûte ~27–55 Kio de pile
        // native (mesuré) — la récursion bascule sur un
        // segment tas avant tout débordement ; la borne
        // `call_depth_limit` garde le plafond logique.
        let r = stacker::maybe_grow(
            crate::parser::STACK_RED_ZONE,
            crate::parser::STACK_GROW,
            || self.exec_body(&local, this, &lit.body),
        );
        self.call_depth -= 1;
        self.call_stack.pop();
        self.strict = outer_strict;
        if lit.is_gen {
            let items = self.gen_collect.pop().unwrap_or_default();
            let ret = match r {
                Ok(Flow::Return(ret)) => ret,
                Ok(_) => Value::Undefined,
                Err(t) => return Err(t),
            };
            let it = crate::collections::gen_iter_obj(self, items, ret);
            // `Object.getPrototypeOf(g()) === g.prototype`
            // (spec) — l'itérateur partagé rendait le proto
            // `Iterator.prototype` à la place.
            if let (Value::Obj(iid), Value::Obj(fid)) = (it, f) {
                if let Some(proto) = self.get_prop(fid, "prototype") {
                    if let Some(o) = self.heap.obj_mut(iid) {
                        o.proto = proto;
                    }
                }
            }
            return Ok(it);
        }
        let out = match r {
            Ok(Flow::Return(v)) => Ok(v),
            Ok(_) => Ok(Value::Undefined),
            Err(t) => Err(t),
        };
        // `async`: the body's result/rejection becomes a
        // promise (deviation: body runs synchronously).
        if lit.is_async {
            let pid = self.new_promise();
            match out {
                Ok(v) => self.resolve_promise(pid, v),
                Err(Thrown(t)) => self.reject_promise(pid, t),
            }
            return Ok(Value::Obj(pid));
        }
        out
    }

    /// `new F(args)` — `new.target` = `f`.
    pub(crate) fn construct(&mut self, f: Value, args: &[Value]) -> JsResult<Value> {
        self.construct_nt(f, args, f)
    }

    /// `Reflect.construct(F, args, newTarget)` — `new.target`
    /// explicite, partagé avec les sous-constructions.
    pub(crate) fn construct_nt(&mut self, f: Value, args: &[Value], nt: Value) -> JsResult<Value> {
        if !self.is_callable(f) {
            return Err(self.type_error("constructor is not a function"));
        }
        let Value::Obj(fid) = f else {
            return Err(self.type_error("constructor is not a function"));
        };
        // Proxy : trap `construct` ou délégation à la cible.
        if let Some((_, target, handler)) = self.proxy_th(fid)? {
            let trap = self.proxy_trap(handler, "construct")?;
            if matches!(trap, Value::Obj(_)) {
                let argv = self.new_array(args.to_vec());
                return self.call_value(trap, handler, &[target, argv, nt]);
            }
            return self.construct_nt(target, args, nt);
        }
        // `new boundFn(...)` delegates to the target with the bound
        // args — the *bound this is ignored* under construction
        // (spec [[Construct]] of BoundFunctionExoticObject).
        if let Some(o) = self.heap.obj(fid) {
            if let ObjKind::Fun(Fun::Bound { target, bound, .. }) = &o.kind {
                let mut a = bound.clone();
                a.extend_from_slice(args);
                let target = *target;
                return self.construct_nt(target, &a, nt);
            }
        }
        // `new (() => {})` — arrows are not constructors.
        if let Some(o) = self.heap.obj(fid) {
            if let ObjKind::Fun(Fun::Script { lit, .. }) = &o.kind {
                if lit.is_arrow {
                    return Err(self.type_error("arrow function is not a constructor"));
                }
            }
        }
        // Le [[Prototype]] de l'instance vient de
        // `newTarget.prototype` — spec `OrdinaryCreateFromConstructor`
        // (`Reflect.construct(f, args, other)` prend le proto
        // d'`other`).
        let proto_src = match nt {
            Value::Obj(nid) => nid,
            _ => fid,
        };
        let proto = match self.get_prop_value(Value::Obj(proto_src), "prototype")? {
            p @ Value::Obj(_) => p,
            _ => Value::Obj(self.protos.object),
        };
        let obj = Value::Obj(self.heap.alloc_obj(Object::ordinary(proto)));
        // Champs d'instance d'une classe de base : installés à
        // l'entrée du corps (spec `DefineField` en tête de ctor).
        self.install_class_fields(fid, FLD_FIELDS, obj, nt)?;
        let r = self.call_value_ctor(f, obj, args, nt)?;
        Ok(if r.is_object() { r } else { obj })
    }

    pub(crate) fn make_function(&mut self, lit: &Rc<FnLit>, env: &EnvRef, this: Value) -> Value {
        // Spec chaîne générateurs : `function*` → proto
        // `%GeneratorFunction.prototype%` et `.prototype` héritant
        // de `%Generator%` (repli `Function`/`Object` avant
        // l'initialisation des ctors cachés).
        let (proto, p_obj) = if lit.is_gen && self.gen_proto != Value::Undefined {
            let (fp, gp) = if lit.is_async {
                (self.async_gen_fn_proto, self.async_gen_proto)
            } else {
                (self.gen_fn_proto, self.gen_proto)
            };
            (fp, gp)
        } else {
            (
                Value::Obj(self.protos.function),
                Value::Obj(self.protos.object),
            )
        };
        let id = self.heap.alloc_obj(Object::fun(
            Fun::Script {
                lit: lit.clone(),
                env: env.clone(),
                lex_this: lit.is_arrow.then_some(this),
            },
            proto,
        ));
        let p = self.heap.alloc_obj(Object::ordinary(p_obj));
        self.set_own(p, "constructor", Value::Obj(id), false);
        self.set_own(id, "prototype", Value::Obj(p), false);
        // Spec `AddRestrictedFunctionProperties` : `caller` et
        // `arguments` sont des accesseurs poison sur les fonctions
        // strictes, les générateurs et l'async (sloppy : absents —
        // `f.caller` → `undefined`, déviation vs le `null` de V8).
        if lit.strict || lit.is_gen || lit.is_async {
            let thr = self.native_fn("restricted", crate::builtins::restricted_prop);
            if let Some(o) = self.heap.obj_mut(id) {
                o.set_accessor("caller", thr, thr, false);
                o.set_accessor("arguments", thr, thr, false);
            }
        }
        Value::Obj(id)
    }

    // -- hoisting ---------------------------------------------------------

    /// Function-level hoisting: `var` names declared, function
    /// declarations installed as values. Nested statements contribute
    /// `var` names only (Annex-B tolerance for inner function decls:
    /// declared `undefined`, installed when actually executed).
    pub(crate) fn hoist(&mut self, body: &[Stmt], env: &EnvRef) {
        for s in body {
            match s {
                Stmt::Var(decls) => {
                    for d in decls {
                        self.declare_pat_names(env, &d.pat);
                    }
                }
                Stmt::FunctionDecl(f) => {
                    if let Some(name) = &f.name {
                        self.env_declare(env, name);
                        let fv = self.make_function(f, env, Value::Undefined);
                        // Le hoisting installe dans une liaison
                        // mutable — l'erreur stricte n'a pas lieu
                        // d'être ; ignorer le résultat est sûr.
                        let _ = self.env_set(env, name, fv);
                    }
                }
                // `export <decl>` — la déclaration interne se
                // hoistée complètement (fonctions initialisées).
                Stmt::Export(ExportDecl::Decl(inner)) => {
                    self.hoist(std::slice::from_ref(&**inner), env);
                }
                other => self.hoist_vars_only(std::slice::from_ref(other), env),
            }
        }
    }

    /// `var` hoisting is function-scoped: walk nested statements too.
    pub(crate) fn hoist_vars_only(&mut self, body: &[Stmt], env: &EnvRef) {
        for s in body {
            match s {
                Stmt::Var(decls) => {
                    for d in decls {
                        self.declare_pat_names(env, &d.pat);
                    }
                }
                Stmt::FunctionDecl(f) => {
                    if let Some(name) = &f.name {
                        self.env_declare(env, name);
                    }
                }
                Stmt::Block(stmts) => self.hoist_vars_only(stmts, env),
                Stmt::If { then, else_, .. } => {
                    self.hoist_vars_only(std::slice::from_ref(&**then), env);
                    if let Some(e) = else_ {
                        self.hoist_vars_only(std::slice::from_ref(&**e), env);
                    }
                }
                Stmt::While { body, .. }
                | Stmt::DoWhile { body, .. }
                | Stmt::With { body, .. }
                | Stmt::Labeled { body, .. } => {
                    self.hoist_vars_only(std::slice::from_ref(&**body), env);
                }
                Stmt::For { init, body, .. } => {
                    if let Some(ForInit::Var(decls)) = init {
                        for d in decls {
                            self.declare_pat_names(env, &d.pat);
                        }
                    }
                    self.hoist_vars_only(std::slice::from_ref(&**body), env);
                }
                Stmt::ForIn { target, body, .. } | Stmt::ForOf { target, body, .. } => {
                    if let ForTarget::Var(pat) = target {
                        self.declare_pat_names(env, pat);
                    }
                    self.hoist_vars_only(std::slice::from_ref(&**body), env);
                }
                Stmt::Try {
                    block,
                    catch,
                    finally,
                } => {
                    self.hoist_vars_only(block, env);
                    if let Some((_, b)) = catch {
                        self.hoist_vars_only(b, env);
                    }
                    if let Some(b) = finally {
                        self.hoist_vars_only(b, env);
                    }
                }
                Stmt::Switch { cases, .. } => {
                    for c in cases {
                        self.hoist_vars_only(&c.body, env);
                    }
                }
                // `export var/function` — la déclaration interne est
                // hoistée comme au top-level.
                Stmt::Export(ExportDecl::Decl(inner)) => {
                    self.hoist_vars_only(std::slice::from_ref(&**inner), env);
                }
                _ => {}
            }
        }
    }
}
// ------------------------------------------------------------------
// JS-C40 — découpage interne (mécanique, sémantique inchangée) :
// `exec` = exécution de statements/programmes, `eval` = évaluation
// d'expressions et opérateurs, `ops` = fonctions libres partagées.
// `impl Engine` réparti sur les fichiers — méthodes `pub(crate)`.
// ------------------------------------------------------------------

mod eval;
mod exec;
mod ops;

pub(crate) use ops::*;
