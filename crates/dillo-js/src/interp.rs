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
    AssignOp, BinOp, ChainStep, ClassLit, ClassMemberKind, ExportDecl, Expr, FnLit, ForInit,
    ForTarget, ObjProp, Param, Pat, Program, PropKey, Stmt, UnOp,
};
use crate::error::{Error, Result};
use crate::heap::{
    Binding, Env, EnvCell, EnvKind, EnvRef, Fun, Heap, JsResult, ObjKind, Object, Prop, TaKind,
    Thrown,
};
use crate::parser;
use crate::value::{num_to_js_string, to_int32, to_uint32, ObjId, Value};
use std::collections::HashSet;
use std::rc::Rc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

// ~5 fat debug frames per JS call; keep well under a 2 MiB thread.
const MAX_CALL_DEPTH: u32 = 100;
const MAX_EXPR_DEPTH: u32 = 300;
const MAX_PROTO_DEPTH: u32 = 512;
const DEFAULT_OP_LIMIT: u64 = 10_000_000;

/// Hidden env bindings installed in class member scopes — the names
/// use private-use code points so no source identifier can collide
/// (`super.x` / `super()` resolve through these).
pub(crate) const HOME_BIND: &str = "\u{f120}home";
/// Parent constructor value for `super(...)`.
pub(crate) const SUPER_BIND: &str = "\u{f120}super";
/// Liaison cachée `new.target` dans l'env local d'une
/// fonction invoquée par `new`/`Reflect.construct`.
pub(crate) const NEW_TARGET: &str = "\u{f120}new.target";

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
const FLD_DONE: &str = "\u{f131}fields.done";
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
enum BindMode {
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
    expr_depth: u32,
    pub(crate) ops_left: u64,
    pub(crate) op_limit: u64,
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
    pub(crate) microtasks: std::collections::VecDeque<crate::promise::Job>,
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
    pub date: ObjId,
    pub regexp: ObjId,
    pub promise: ObjId,
    /// Shared `next()`/`[Symbol.iterator]` iterator prototype.
    pub iterator: ObjId,
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
    fn all(&self) -> [ObjId; 27] {
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
        let date = heap.alloc_obj(Object::ordinary(Value::Obj(object)));
        let regexp = heap.alloc_obj(Object::ordinary(Value::Obj(object)));
        let promise = heap.alloc_obj(Object::ordinary(Value::Obj(object)));
        let iterator = heap.alloc_obj(Object::ordinary(Value::Obj(object)));
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
                date,
                regexp,
                promise,
                iterator,
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
            expr_depth: 0,
            ops_left: DEFAULT_OP_LIMIT,
            op_limit: DEFAULT_OP_LIMIT,
            timers: crate::timers::TimerQueue::default(),
            xhr: crate::xhr::XhrEngineState::default(),
            dom_ctx: None,
            microtasks: std::collections::VecDeque::new(),
            hooks: crate::dom_host::BrowserHooks::default(),
            completion: Value::Undefined,
            strict: false,
            modules: Vec::new(),
            module_specs: std::collections::HashMap::new(),
            module_envs: Vec::new(),
            module_loader: None,
            current_module: String::new(),
            gen_collect: Vec::new(),
            pending_labels: Vec::new(),
            using_stack: Vec::new(),
            eval_fn: Value::Undefined,
            re_ctor: Value::Undefined,
            re_static: None,
        };
        crate::builtins::install(&mut e);
        crate::collections::install(&mut e);
        crate::intl::install(&mut e);
        crate::temporal::install(&mut e);
        crate::json::install(&mut e);
        crate::typed_array::install(&mut e);
        e
    }

    /// Maximum number of statement/expression ticks before the script
    /// is killed. Reset by each `eval` call.
    pub fn set_op_limit(&mut self, limit: u64) {
        self.op_limit = limit;
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
    /// en attente, file de microtasks, XHR/`fetch` en vol, table
    /// DOM↔JS (cache + listeners de nœuds), hooks navigateur,
    /// valeur de complétion — plus les racines explicites posées
    /// par [`Engine::add_root`]. Retourne le nombre de cellules
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
        for (_, promise) in &self.xhr.fetches {
            ids.push(*promise);
        }
        for job in &self.microtasks {
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
        if let Some((dom, cache)) = &self.dom_ctx {
            for &id in cache.borrow().values() {
                ids.push(id);
            }
            for node in &dom.borrow().nodes {
                for (_, cb, _) in &node.listeners {
                    crate::heap::push_value(&mut ids, *cb);
                }
            }
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
            self.microtasks.push_back(crate::promise::Job::Plain(f));
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
        let prog = parser::parse(src)?;
        self.ops_left = self.op_limit;
        let r = match self.exec_program(&prog) {
            Ok(v) => Ok(v),
            Err(Thrown(v)) => Err(Error::Uncaught(self.describe(v))),
        };
        if let Err(Thrown(v)) = self.run_microtasks() {
            return Err(Error::Uncaught(self.describe(v)));
        }
        r
    }

    /// Human-readable rendering of a thrown value.
    pub fn describe(&mut self, v: Value) -> String {
        if let Value::Obj(id) = v {
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

    fn heap_string_of(&self, v: Value) -> Option<String> {
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

    /// New `Error`-family object value (`{name, message}` on `proto`).
    pub(crate) fn error_value(&mut self, proto: ObjId, name: &str, msg: &str) -> Value {
        let id = self.heap.alloc_obj(Object::ordinary(Value::Obj(proto)));
        let n = self.str_val(name);
        let m = self.str_val(msg);
        self.set_own(id, "name", n, false);
        self.set_own(id, "message", m, false);
        //  (own, non-enum). Deviation documentee : pas
        // d introspection de la pile d appel — on ne rend que la
        // premiere ligne V8-style Name: message.
        let head = if msg.is_empty() {
            name.to_string()
        } else {
            format!("{name}: {msg}")
        };
        let s = self.str_val(head);
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
    pub(crate) fn tick(&mut self) -> JsResult<()> {
        if self.ops_left == 0 {
            return Err(self.range_error("script execution limit exceeded"));
        }
        self.ops_left -= 1;
        Ok(())
    }

    // -- environments --------------------------------------------------

    /// Lookup de `name` le long de la chaîne de portée — les alias
    /// d'import de module renvoient vers l'env exporteur (live
    /// binding) ; `visited` borne les chaînes cycliques
    /// (`export {a} from 'b'` ↔ `export {b} from 'a'`).
    pub(crate) fn env_get(&mut self, env: &EnvRef, name: &str) -> Option<Value> {
        self.env_get_seen(env, name, &mut HashSet::new())
    }

    fn env_get_seen(
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
                    return self.get_prop(oid, &name);
                }
                continue;
            }
            let next = {
                let b = e.borrow();
                match &b.kind {
                    EnvKind::Local(m) => match m.get(&name) {
                        Some(EnvCell::Bind(v)) => return Some(v.value),
                        Some(EnvCell::Alias(t, n)) => Next::Alias(t.clone(), n.clone()),
                        None => Next::Up(b.parent.clone()),
                    },
                    EnvKind::Global(m) => match m.get(&name) {
                        Some(EnvCell::Bind(v)) => return Some(v.value),
                        Some(EnvCell::Alias(t, n)) => Next::Alias(t.clone(), n.clone()),
                        None => {
                            // Declarative record first, then `window` props.
                            if let Some(v) = self.get_prop(self.global, &name) {
                                return Some(v);
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
    fn with_object_of(&mut self, v: Value) -> Option<ObjId> {
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
            Value::Undefined | Value::Null => return None,
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
    fn with_has(&mut self, oid: ObjId, name: &str) -> JsResult<bool> {
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
    fn env_has_binding(env: &EnvRef, name: &str) -> bool {
        let mut cur = Some(env.clone());
        while let Some(e) = cur {
            let next = {
                let b = e.borrow();
                match &b.kind {
                    EnvKind::Local(m) | EnvKind::Global(m) => {
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
                    self.write_prop(oid, Value::Obj(oid), name, v)?;
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
                    EnvKind::Local(m) if m.contains_key(name) => Act::Local(e.clone()),
                    EnvKind::Global(m) if m.contains_key(name) => Act::Local(e.clone()),
                    EnvKind::Global(_) => Act::Global,
                    // `With` déjà géré au-dessus — repli défensif
                    // identique à `Local` sans la clé.
                    EnvKind::Local(_) | EnvKind::With(_) => Act::Up(b.parent.clone()),
                }
            };
            match act {
                Act::Local(e) => {
                    let mut b = e.borrow_mut();
                    let m = match &mut b.kind {
                        EnvKind::Local(m) | EnvKind::Global(m) => m,
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
    fn set_global_prop(&mut self, name: &str, v: Value) -> JsResult<()> {
        let g = self.global;
        match self.write_prop(g, Value::Obj(g), name, v) {
            Ok(()) => Ok(()),
            Err(t) => Err(t),
        }
    }

    fn env_declare(&mut self, env: &EnvRef, name: &str) {
        let parent = {
            let mut b = env.borrow_mut();
            match &mut b.kind {
                EnvKind::Local(m) => {
                    m.entry(name.to_string()).or_insert(EnvCell::Bind(Binding {
                        value: Value::Undefined,
                        mutable: true,
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
                // `var` dans un `with` se déclare dans le scope
                // fonction englobant (spec `VarDeclaredNames`).
                EnvKind::With(_) => b.parent.clone(),
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
                EnvKind::Local(m) | EnvKind::Global(m) => {
                    m.insert(
                        name.to_string(),
                        EnvCell::Bind(Binding { value: v, mutable }),
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
    /// (block scoping is approximated — a `let` inside `{}` lives in
    /// the function scope and there is no TDZ: an uninitialised read
    /// yields `undefined` rather than throwing).
    ///
    /// Global `let`/`const` go to the declarative record, *not* the
    /// global object — `window.x` stays `undefined` for `let x`.
    pub(crate) fn env_declare_lex(env: &EnvRef, name: &str, mutable: bool) {
        let parent = {
            let mut b = env.borrow_mut();
            match &mut b.kind {
                EnvKind::Local(m) | EnvKind::Global(m) => {
                    m.insert(
                        name.to_string(),
                        EnvCell::Bind(Binding {
                            value: Value::Undefined,
                            mutable,
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
                EnvKind::Local(m) | EnvKind::Global(m) => {
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

    /// `let`/`const`-declare every `Ident` inside `pat`.
    fn declare_pat_lex(env: &EnvRef, pat: &Pat, mutable: bool) {
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
    fn bind_pat(
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
    fn eval_prop_key(&mut self, env: &EnvRef, this: Value, k: &PropKey) -> JsResult<String> {
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
    fn eval_args(&mut self, env: &EnvRef, this: Value, arg_exprs: &[Expr]) -> JsResult<Vec<Value>> {
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
    fn super_base(&mut self, env: &EnvRef) -> JsResult<Value> {
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
    fn eval_class(&mut self, env: &EnvRef, this: Value, cl: &ClassLit) -> JsResult<Value> {
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
    fn install_class_fields(
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
                return elems.get(i).copied().map(|value| Prop::Data {
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
    fn ta_prop(&self, kind: &ObjKind, key: &str) -> Option<Prop> {
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
    fn find_prop(&mut self, id: ObjId, key: &str) -> Option<Prop> {
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
    fn read_prop(&mut self, base: Value, recv: Value, key: &str) -> JsResult<Value> {
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
    fn write_prop(&mut self, id: ObjId, recv: Value, key: &str, v: Value) -> JsResult<()> {
        if let Some((tid, target, handler)) = self.proxy_th(id)? {
            let trap = self.proxy_trap(handler, "set")?;
            if matches!(trap, Value::Obj(_)) {
                let kv = self.str_val(key);
                let ok = self.call_value(trap, handler, &[target, kv, v, recv])?;
                return if self.to_boolean(ok) {
                    // Invariants spec `[[Set]]` : data non-config
                    // non-writable → écrire la *même* valeur seulement ;
                    // accesseur non-config sans setter → jamais de
                    // succès.
                    match self.find_own(tid, key) {
                        Some(Prop::Data { value, attrs })
                            if !attrs.configurable && !attrs.writable =>
                        {
                            if !self.strict_eq(v, value) {
                                return Err(self.type_error(
                                    "proxy set: cannot report success on non-configurable non-writable prop",
                                ));
                            }
                        }
                        Some(Prop::Acc { set, attrs, .. })
                            if !attrs.configurable && matches!(set, Value::Undefined) =>
                        {
                            return Err(self.type_error(
                                "proxy set: non-configurable accessor without setter",
                            ));
                        }
                        _ => {}
                    }
                    Ok(())
                } else {
                    self.write_refused(key)
                };
            }
            return self.write_prop(tid, recv, key, v);
        }
        // Accessors on the chain intercept the write.
        let mut cur = Some(id);
        let mut depth = 0;
        while let Some(cid) = cur {
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

    /// Écriture refusée (`[[Set]]` → `false`) — `TypeError` en
    /// strict, silence en sloppy.
    fn write_refused(&mut self, key: &str) -> JsResult<()> {
        if self.strict {
            Err(self.type_error(format!("cannot write `{key}`")))
        } else {
            Ok(())
        }
    }

    /// Annex B — statiques du ctor `RegExp`. `None` si `key` n'est
    /// pas une statique ; `Some("")` quand la statique est vide
    /// (aucun match enregistré — V8 rend `""`, pas `undefined`).
    fn re_static_get(&self, key: &str) -> Option<String> {
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
    fn non_config_own(&mut self, id: ObjId, key: &str) -> Option<bool> {
        self.find_own(id, key).map(|p| match &p {
            Prop::Data { attrs, .. } | Prop::Acc { attrs, .. } => !attrs.configurable,
        })
    }

    /// `Object.isExtensible`-niveau interne (invariants proxy).
    pub(crate) fn target_extensible(&self, id: ObjId) -> bool {
        self.heap.obj(id).is_some_and(|o| o.extensible)
    }

    /// Extract the host delegate of an object, if any.
    fn host_of(&self, id: ObjId) -> Option<Rc<dyn crate::heap::Host>> {
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

    /// `base[key]` on any value — wraps primitives via their prototype.
    pub(crate) fn get_prop_value(&mut self, base: Value, key: &str) -> JsResult<Value> {
        match base {
            Value::Undefined | Value::Null => {
                Err(self.type_error(format!("cannot read `{key}` of {base:?}")))
            }
            Value::Obj(id) => {
                if let Some(h) = self.host_of(id) {
                    if let Some(v) = h.get(self, base, key)? {
                        return Ok(v);
                    }
                }
                self.read_prop(base, base, key)
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
                        return Ok(());
                    }
                }
                self.write_prop(id, base, key, v)
            }
            // Sloppy mode: writes to primitives are silently ignored;
            // strict: `TypeError` (spec `PutValue` on a primitive base).
            _ => {
                if self.strict {
                    Err(self.type_error(format!("cannot write `{key}`")))
                } else {
                    Ok(())
                }
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
                elems.resize(n, Value::Undefined);
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
                    elems.resize(i + 1, Value::Undefined);
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

    fn delete_prop(&mut self, id: ObjId, key: &str) -> bool {
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
                        elems[i] = Value::Undefined;
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
        if let Some(o) = self.heap.obj(id) {
            match &o.kind {
                ObjKind::Array(elems) => {
                    for i in 0..elems.len() {
                        out.push(i.to_string());
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
                out.push(k.clone());
            }
        }
        out
    }

    /// Enumerable *own* keys (`Object.keys`): array indices + props.
    pub(crate) fn own_enum_keys(&self, id: ObjId) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(o) = self.heap.obj(id) {
            match &o.kind {
                ObjKind::Array(elems) => {
                    for i in 0..elems.len() {
                        out.push(i.to_string());
                    }
                }
                ObjKind::TypedArray { len, .. } => {
                    for i in 0..*len {
                        out.push(i.to_string());
                    }
                }
                _ => {}
            }
            for (k, p) in &o.props {
                // Clés-symboles sentinelles : jamais énumérées.
                if p.enumerable() && !k.starts_with(SYM_PREFIX) {
                    out.push(k.clone());
                }
            }
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
    fn prop_from_desc_obj(&mut self, d: ObjId) -> JsResult<Option<Prop>> {
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
    fn enum_keys(&mut self, id: ObjId) -> JsResult<Vec<String>> {
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
                    for i in 0..elems.len() {
                        if seen.insert(i.to_string()) {
                            out.push(i.to_string());
                        }
                    }
                }
                for (k, p) in &o.props {
                    if p.enumerable() && seen.insert(k.clone()) {
                        out.push(k.clone());
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
            Value::Undefined | Value::Null => false,
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
            Value::Undefined => f64::NAN,
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
            Value::Undefined => "undefined".into(),
            Value::Null => "null".into(),
            Value::Bool(b) => if b { "true" } else { "false" }.into(),
            Value::Num(n) => num_to_js_string(n),
            Value::Str(id) => self.heap.string(id).unwrap_or_default(),
            Value::Big(_) => self.bigint_of(v).map(|b| b.to_string()).unwrap_or_default(),
            Value::Obj(_) => {
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
    fn call_value_ctor(
        &mut self,
        f: Value,
        this: Value,
        args: &[Value],
        nt: Value,
    ) -> JsResult<Value> {
        self.call_value_impl(f, this, args, nt)
    }

    fn call_value_impl(
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
            Fun::Native { f, .. } => f(self, this, args),
            Fun::BoundNative { f, bound, .. } => {
                let mut a = Vec::with_capacity(bound.len() + args.len());
                a.extend_from_slice(&bound);
                a.extend_from_slice(args);
                f(self, this, &a)
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
                self.call_depth += 1;
                if self.call_depth > MAX_CALL_DEPTH {
                    self.call_depth -= 1;
                    return Err(self.range_error("maximum call stack exceeded"));
                }
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
                let r = self.exec_body(&local, this, &lit.body);
                self.call_depth -= 1;
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
        }
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
    fn hoist_vars_only(&mut self, body: &[Stmt], env: &EnvRef) {
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

    // -- statement execution -----------------------------------------------

    /// Parse + run `src` in the **global** scope, returning the
    /// thrown `Value` intact (unlike [`Engine::eval`] which formats
    /// it). Used by the `eval` builtin, `Function()`, and string
    /// timer callbacks — all global-scope (indirect `eval`
    /// semantics; direct-eval scoping is a documented deviation).
    /// Microtasks are *not* drained here — the enclosing job does.
    pub(crate) fn eval_source(&mut self, src: &str) -> JsResult<Value> {
        // `eval` direct : le texte hérite du strict appelant —
        // early errors (`01`, `delete x`) et effets runtime.
        let prog = crate::parser::parse_in(src, self.strict).map_err(|err| {
            Thrown(self.error_value(self.protos.syntax_error, "SyntaxError", &err.to_string()))
        })?;
        self.exec_program(&prog)
    }

    pub(crate) fn exec_program(&mut self, prog: &Program) -> JsResult<Value> {
        let outer_strict = self.strict;
        self.strict |= prog.strict;
        let r = self.exec_program_inner(prog);
        self.strict = outer_strict;
        r
    }

    fn exec_program_inner(&mut self, prog: &Program) -> JsResult<Value> {
        let env = self.global_env.clone();
        self.exec_program_in(&env, Value::Obj(self.global), prog)
    }

    /// `eval(src)` **direct** (spec `PerformEval`, chemin local) :
    /// le programme tourne dans l'env et le `this` de l'appelant —
    /// les `var`/fonctions y persistent, `x`/`let` de l'appelant y
    /// sont visibles. Déviation documentée : la spec isole les
    /// `let`/`const` de l'eval dans un env déclaratif dédié ; notre
    /// approximation bloc-scope les rend déjà fonction-scope, ils
    /// vivent donc dans l'env appelant aussi.
    pub(crate) fn eval_source_in(
        &mut self,
        env: &EnvRef,
        this: Value,
        src: &str,
    ) -> JsResult<Value> {
        let prog = crate::parser::parse_in(src, self.strict).map_err(|err| {
            Thrown(self.error_value(self.protos.syntax_error, "SyntaxError", &err.to_string()))
        })?;
        let outer_strict = self.strict;
        self.strict |= prog.strict;
        let r = self.exec_program_in(env, this, &prog);
        self.strict = outer_strict;
        r
    }

    /// Corps de programme dans un couple env/`this` arbitraires —
    /// partagé entre `exec_program_inner` (global) et
    /// `eval_source_in` (direct eval, env appelant).
    fn exec_program_in(&mut self, env: &EnvRef, this: Value, prog: &Program) -> JsResult<Value> {
        self.hoist(&prog.body, env);
        self.completion = Value::Undefined;
        // `using` au niveau programme/module : frame racine dépilée
        // en fin de script (spec).
        let has_using = prog.body.iter().any(|s| matches!(s, Stmt::Using { .. }));
        if has_using {
            self.using_stack.push(Vec::new());
        }
        let mut r = Ok(Flow::Normal);
        for s in &prog.body {
            match self.exec_stmt(env, this, s) {
                Ok(Flow::Normal | Flow::Break(_) | Flow::Continue(_)) => {}
                other => {
                    r = other;
                    break;
                }
            }
        }
        if has_using {
            r = self.drain_using(r);
        }
        match r {
            Ok(Flow::Return(v)) => Ok(v),
            Ok(_) => Ok(self.completion),
            Err(e) => Err(e),
        }
    }

    pub(crate) fn exec_body(&mut self, env: &EnvRef, this: Value, body: &[Stmt]) -> JsResult<Flow> {
        // `using` (ES2026) : chaque décl pousse un disposeur sur la
        // frame courante, dépilée LIFO à TOUTE sortie du corps —
        // normal, break, return et throw (spec `DisposeResources`).
        let has_using = body.iter().any(|s| matches!(s, Stmt::Using { .. }));
        if has_using {
            self.using_stack.push(Vec::new());
        }
        let mut r = Ok(Flow::Normal);
        for s in body {
            match self.exec_stmt(env, this, s) {
                Ok(Flow::Normal) => {}
                other => {
                    r = other;
                    break;
                }
            }
        }
        if has_using {
            r = self.drain_using(r);
        }
        r
    }

    /// Dépile la frame `using` courante, appelant chaque disposeur
    /// LIFO (`v[@@dispose]()` / `v[@@asyncDispose]()` + await eager).
    /// Une erreur de disposeur se substitue au résultat courant ;
    /// une erreur déjà en cours est enroulée en `SuppressedError`
    /// (spec `DisposeResources` step `AddSuppressedError`).
    fn drain_using(&mut self, mut r: JsResult<Flow>) -> JsResult<Flow> {
        let Some(frame) = self.using_stack.pop() else {
            return r;
        };
        for (f, v, is_async) in frame.into_iter().rev() {
            let res = self.call_value(f, v, &[]).and_then(|out| {
                if is_async {
                    self.await_value(out).map(|_| ())
                } else {
                    Ok(())
                }
            });
            if let Err(Thrown(err)) = res {
                r = match r {
                    Err(Thrown(prev)) => {
                        Err(Thrown(crate::builtins::make_suppressed(self, err, prev)))
                    }
                    _ => Err(Thrown(err)),
                };
            }
        }
        r
    }

    /// `using x = v` — enregistre `v[@@dispose]` (ou
    /// `v[@@asyncDispose] ?? v[@@dispose]` pour `await using`) sur la
    /// frame courante. `null`/`undefined` passent ; un objet sans
    /// méthode ou une primitive → `TypeError` (spec
    /// `AddDisposableResource`).
    fn register_using(&mut self, v: Value, is_await: bool) -> JsResult<()> {
        match v {
            Value::Undefined | Value::Null => Ok(()),
            Value::Obj(id) => {
                let m = if is_await {
                    self.get_prop(id, crate::interp::SYM_ASYNC_DISPOSE)
                        .or_else(|| self.get_prop(id, crate::interp::SYM_DISPOSE))
                } else {
                    self.get_prop(id, crate::interp::SYM_DISPOSE)
                };
                match m {
                    Some(f) if self.is_callable(f) => {
                        if let Some(frame) = self.using_stack.last_mut() {
                            frame.push((f, v, is_await));
                        }
                        // Hors frame (décl hors corps) : la ressource
                        // n'est jamais dépilée — déviation vs spec.
                        Ok(())
                    }
                    _ => Err(self.type_error("object is not disposable")),
                }
            }
            _ => Err(self.type_error("using declaration requires an object or null/undefined")),
        }
    }

    pub(crate) fn exec_stmt(&mut self, env: &EnvRef, this: Value, s: &Stmt) -> JsResult<Flow> {
        self.tick()?;
        match s {
            // `import …` — liaisons câblées à l'instanciation ;
            // l'exécution n'a rien à faire (idem `;`).
            Stmt::Empty | Stmt::Import { .. } => Ok(Flow::Normal),
            Stmt::Export(d) => match d {
                // `export <decl>` — la déclaration s'exécute
                // normalement (les noms sont déjà exportés).
                ExportDecl::Decl(inner) => self.exec_stmt(env, this, inner),
                // `export default <expr|function|class>` — lie le
                // slot `default` (déclaré à l'instanciation) ; une
                // fonction/classe nommée lie aussi son propre nom.
                ExportDecl::Default(e) => {
                    let v = self.eval_expr(env, this, e)?;
                    self.env_set(env, "default", v)?;
                    let own = match &**e {
                        Expr::Function(lit) => lit.name.clone(),
                        Expr::Class(lit) => lit.name.clone(),
                        _ => None,
                    };
                    if let Some(n) = own {
                        Self::env_declare_lex(env, &n, true);
                        self.env_set(env, &n, v)?;
                    }
                    Ok(Flow::Normal)
                }
                // `export {…}` / `export * …` — pure liaison, déjà
                // résolue à l'instanciation.
                _ => Ok(Flow::Normal),
            },
            Stmt::Expr(e) => {
                self.completion = self.eval_expr(env, this, e)?;
                Ok(Flow::Normal)
            }
            Stmt::Var(decls) => {
                for d in decls {
                    match &d.init {
                        Some(init) => {
                            let v = self.eval_expr(env, this, init)?;
                            self.bind_pat(env, this, &d.pat, v, BindMode::Var)?;
                        }
                        // `var a;` — declare without clobbering.
                        None => self.declare_pat_names(env, &d.pat),
                    }
                }
                Ok(Flow::Normal)
            }
            Stmt::Lexical { decls, constant } => {
                for d in decls {
                    Self::declare_pat_lex(env, &d.pat, !constant);
                    if let Some(init) = &d.init {
                        let v = self.eval_expr(env, this, init)?;
                        self.bind_pat(env, this, &d.pat, v, BindMode::Lex { mutable: !constant })?;
                    }
                }
                Ok(Flow::Normal)
            }
            // Function decls were installed by hoisting; re-installing
            // keeps block-level decls working (Annex-B-ish tolerance).
            Stmt::FunctionDecl(f) => {
                if let Some(name) = &f.name {
                    let fv = self.make_function(f, env, Value::Undefined);
                    self.env_set(env, name, fv)?;
                }
                Ok(Flow::Normal)
            }
            // Class decls are `let`-like (no hoist — a use before the
            // decl is a ReferenceError, closest to spec TDZ).
            Stmt::ClassDecl(cl) => {
                let v = self.eval_class(env, this, cl)?;
                if let Some(name) = &cl.name {
                    Self::env_declare_lex(env, name, true);
                    self.env_set(env, name, v)?;
                }
                Ok(Flow::Normal)
            }
            Stmt::Block(stmts) => self.exec_body(env, this, stmts),
            // `using x = e` (ES2026) — binding const + disposeur
            // enregistré sur la frame du corps courant.
            Stmt::Using { decls, is_await } => {
                for d in decls {
                    let v = match &d.init {
                        Some(i) => self.eval_expr(env, this, i)?,
                        None => Value::Undefined,
                    };
                    self.bind_pat(env, this, &d.pat, v, BindMode::Lex { mutable: false })?;
                    self.register_using(v, *is_await)?;
                }
                Ok(Flow::Normal)
            }
            Stmt::If { cond, then, else_ } => {
                let c = self.eval_expr(env, this, cond)?;
                if self.to_boolean(c) {
                    self.exec_stmt(env, this, then)
                } else if let Some(e) = else_ {
                    self.exec_stmt(env, this, e)
                } else {
                    Ok(Flow::Normal)
                }
            }
            // `with (o) b` — les noms se résolvent d'abord dans les
            // props de `o` (sauf `@@unscopables`), puis le scope
            // englobant. Sloppy uniquement (early-error du parseur).
            Stmt::With { obj, body } => {
                let v = self.eval_expr(env, this, obj)?;
                let Some(oid) = self.with_object_of(v) else {
                    return Err(self.type_error("cannot use `with` on `null` or `undefined`"));
                };
                let with_env = Env::with(env.clone(), oid);
                self.exec_stmt(&with_env, this, body)
            }
            Stmt::While { cond, body } => {
                let labels = std::mem::take(&mut self.pending_labels);
                loop {
                    self.tick()?;
                    let c = self.eval_expr(env, this, cond)?;
                    if !self.to_boolean(c) {
                        return Ok(Flow::Normal);
                    }
                    let f = self.exec_stmt(env, this, body)?;
                    if let Some(f) = Self::loop_flow(f, &labels) {
                        return Ok(f);
                    }
                }
            }
            Stmt::DoWhile { body, cond } => {
                let labels = std::mem::take(&mut self.pending_labels);
                loop {
                    self.tick()?;
                    let f = self.exec_stmt(env, this, body)?;
                    if let Some(f) = Self::loop_flow(f, &labels) {
                        return Ok(f);
                    }
                    let c = self.eval_expr(env, this, cond)?;
                    if !self.to_boolean(c) {
                        return Ok(Flow::Normal);
                    }
                }
            }
            Stmt::For {
                init,
                test,
                update,
                body,
            } => {
                let labels = std::mem::take(&mut self.pending_labels);
                if let Some(init) = init {
                    match init {
                        ForInit::Var(decls) => {
                            for d in decls {
                                if let Some(i) = &d.init {
                                    let v = self.eval_expr(env, this, i)?;
                                    self.bind_pat(env, this, &d.pat, v, BindMode::Var)?;
                                }
                            }
                        }
                        ForInit::Lexical { decls, constant } => {
                            for d in decls {
                                Self::declare_pat_lex(env, &d.pat, !constant);
                                if let Some(i) = &d.init {
                                    let v = self.eval_expr(env, this, i)?;
                                    self.bind_pat(
                                        env,
                                        this,
                                        &d.pat,
                                        v,
                                        BindMode::Lex { mutable: !constant },
                                    )?;
                                }
                            }
                        }
                        ForInit::Expr(e) => {
                            self.eval_expr(env, this, e)?;
                        }
                    }
                }
                loop {
                    self.tick()?;
                    if let Some(t) = test {
                        let c = self.eval_expr(env, this, t)?;
                        if !self.to_boolean(c) {
                            return Ok(Flow::Normal);
                        }
                    }
                    let f = self.exec_stmt(env, this, body)?;
                    if let Some(f) = Self::loop_flow(f, &labels) {
                        return Ok(f);
                    }
                    if let Some(u) = update {
                        self.eval_expr(env, this, u)?;
                    }
                }
            }
            Stmt::ForIn {
                target,
                obj,
                var_init,
                body,
            } => {
                let labels = std::mem::take(&mut self.pending_labels);
                let ov = self.eval_expr(env, this, obj)?;
                if let Some(init) = var_init {
                    let v = self.eval_expr(env, this, init)?;
                    self.assign_target(env, this, target, v)?;
                }
                let Value::Obj(id) = ov else {
                    return Ok(Flow::Normal); // for-in over primitives: skip
                };
                for key in self.enum_keys(id)? {
                    let kv = self.str_val(key);
                    self.assign_target(env, this, target, kv)?;
                    let f = self.exec_stmt(env, this, body)?;
                    if let Some(f) = Self::loop_flow(f, &labels) {
                        return Ok(f);
                    }
                }
                Ok(Flow::Normal)
            }
            Stmt::ForOf {
                target,
                iter,
                body,
                is_await,
            } => {
                let labels = std::mem::take(&mut self.pending_labels);
                let v = self.eval_expr(env, this, iter)?;
                let items = if *is_await {
                    self.iter_values_await(v)?
                } else {
                    self.iter_values(v)?
                };
                // `for (using x of y)` : frame par itération (spec —
                // la ressource est dépilée à chaque tour).
                let per_iter = matches!(target, ForTarget::Using { .. });
                for item in items {
                    if per_iter {
                        self.using_stack.push(Vec::new());
                    }
                    let mut r = self
                        .assign_target(env, this, target, item)
                        .and_then(|()| self.exec_stmt(env, this, body));
                    if per_iter {
                        r = self.drain_using(r);
                    }
                    if let Some(f) = Self::loop_flow(r?, &labels) {
                        return Ok(f);
                    }
                }
                Ok(Flow::Normal)
            }
            // `lbl : stmt` — les labels de boucle sont consommés par
            // la boucle enfant via `pending_labels` ; `break lbl`
            // (même sur bloc) est avalé ici, tout le reste remonte.
            Stmt::Labeled { name, body } => {
                if matches!(
                    &**body,
                    Stmt::While { .. }
                        | Stmt::DoWhile { .. }
                        | Stmt::For { .. }
                        | Stmt::ForIn { .. }
                        | Stmt::ForOf { .. }
                ) {
                    self.pending_labels.push(name.clone());
                }
                let r = self.exec_stmt(env, this, body);
                // Défensif : nettoie le label si la boucle ne l'a
                // pas consommé (corps non exécuté, erreur précoce).
                if let Some(pos) = self.pending_labels.iter().rposition(|l| l == name) {
                    self.pending_labels.remove(pos);
                }
                match r? {
                    Flow::Break(Some(l)) if l == *name => Ok(Flow::Normal),
                    f => Ok(f),
                }
            }
            Stmt::Break(l) => Ok(Flow::Break(l.clone())),
            Stmt::Continue(l) => Ok(Flow::Continue(l.clone())),
            Stmt::Return(e) => {
                let v = match e {
                    Some(e) => self.eval_expr(env, this, e)?,
                    None => Value::Undefined,
                };
                Ok(Flow::Return(v))
            }
            Stmt::Throw(e) => {
                let v = self.eval_expr(env, this, e)?;
                Err(Thrown(v))
            }
            Stmt::Try {
                block,
                catch,
                finally,
            } => {
                let mut r = self.exec_body(env, this, block);
                if let Err(Thrown(v)) = r {
                    if let Some((pat, cbody)) = catch {
                        let cenv = Env::local(env.clone());
                        if let Err(e) = self.bind_pat(&cenv, this, pat, v, BindMode::Var) {
                            r = Err(e);
                        } else {
                            r = self.exec_body(&cenv, this, cbody);
                        }
                    }
                }
                if let Some(fbody) = finally {
                    let fr = self.exec_body(env, this, fbody)?;
                    if !matches!(fr, Flow::Normal) {
                        return Ok(fr);
                    }
                }
                r
            }
            Stmt::Switch { disc, cases } => {
                let dv = self.eval_expr(env, this, disc)?;
                let mut matched = None;
                let mut default = None;
                for (i, c) in cases.iter().enumerate() {
                    match &c.test {
                        Some(t) => {
                            let tv = self.eval_expr(env, this, t)?;
                            if strict_eq(dv, tv, &self.heap) {
                                matched = Some(i);
                                break;
                            }
                        }
                        None => default = Some(i),
                    }
                }
                let Some(start) = matched.or(default) else {
                    return Ok(Flow::Normal);
                };
                for c in &cases[start..] {
                    match self.exec_body(env, this, &c.body)? {
                        Flow::Normal => {}
                        // `break` seul sort du switch ; `break lbl`
                        // remonte jusqu'au statement étiqueté.
                        Flow::Break(None) => return Ok(Flow::Normal),
                        f => return Ok(f),
                    }
                }
                Ok(Flow::Normal)
            }
        }
    }

    /// Consomme le `Flow` produit par un corps de boucle —
    /// `None` = itération suivante, `Some(f)` = quitter la boucle
    /// en propageant `f`. `labels` = labels de boucle captés via
    /// `pending_labels` (`continue lbl` ne les avale que si `lbl`
    /// y figure ; `break lbl` n'est jamais avalé par une boucle).
    fn loop_flow(f: Flow, labels: &[String]) -> Option<Flow> {
        match f {
            Flow::Normal | Flow::Continue(None) => None,
            Flow::Continue(Some(l)) => {
                if labels.contains(&l) {
                    None
                } else {
                    Some(Flow::Continue(Some(l)))
                }
            }
            Flow::Break(None) => Some(Flow::Normal),
            f => Some(f),
        }
    }

    fn assign_target(
        &mut self,
        env: &EnvRef,
        this: Value,
        target: &ForTarget,
        v: Value,
    ) -> JsResult<()> {
        match target {
            ForTarget::Var(pat) => self.bind_pat(env, this, pat, v, BindMode::Var)?,
            // `for (let k in …)`: (re)declare in scope then assign —
            // fresh binding per iteration is approximated by reuse.
            ForTarget::Lexical { pat, constant } => {
                self.bind_pat(env, this, pat, v, BindMode::Lex { mutable: !constant })?;
            }
            ForTarget::Expr(e) => {
                // `[a, b] of …` / `{k} of …` — pattern assignment.
                if let Some(pat) = expr_to_pat(e) {
                    self.bind_pat(env, this, &pat, v, BindMode::Assign)?;
                } else {
                    self.write_lvalue(env, this, e, v)?;
                }
            }
            // `for (using x of y)` — binding const + disposeur.
            ForTarget::Using { pat, is_await } => {
                self.bind_pat(env, this, pat, v, BindMode::Lex { mutable: false })?;
                self.register_using(v, *is_await)?;
            }
        }
        Ok(())
    }

    // -- expression evaluation ---------------------------------------------

    fn eval_expr(&mut self, env: &EnvRef, this: Value, e: &Expr) -> JsResult<Value> {
        self.expr_depth += 1;
        if self.expr_depth > MAX_EXPR_DEPTH {
            self.expr_depth -= 1;
            return Err(self.range_error("expression too deeply nested"));
        }
        let r = self.eval_inner(env, this, e);
        self.expr_depth -= 1;
        r
    }

    #[allow(clippy::too_many_lines)]
    fn eval_inner(&mut self, env: &EnvRef, this: Value, e: &Expr) -> JsResult<Value> {
        match e {
            Expr::Num(n) => Ok(Value::Num(*n)),
            Expr::BigInt(s) => {
                Ok(self
                    .big_val(num_bigint::BigInt::parse_bytes(s.as_bytes(), 10).unwrap_or_default()))
            }
            Expr::Str(s) => Ok(self.str_val(s.clone())),
            Expr::Bool(b) => Ok(Value::Bool(*b)),
            Expr::Null => Ok(Value::Null),
            Expr::This => Ok(this),
            Expr::Ident(name) => self
                .env_get(env, name)
                .ok_or_else(|| self.ref_error(format!("{name} is not defined"))),
            Expr::Array(elems) => {
                let mut v = Vec::with_capacity(elems.len());
                for el in elems {
                    match el {
                        // `[...a]` — iterate the source.
                        Some(Expr::Spread(e)) => {
                            let sv = self.eval_expr(env, this, e)?;
                            v.extend(self.iter_values(sv)?);
                        }
                        Some(e) => v.push(self.eval_expr(env, this, e)?),
                        None => v.push(Value::Undefined),
                    }
                }
                Ok(self.new_array(v))
            }
            Expr::Object(props) => {
                let id = self.new_object();
                for p in props {
                    match p {
                        ObjProp::Prop(k, ve) => {
                            let ks = self.eval_prop_key(env, this, k)?;
                            let v = self.eval_expr(env, this, ve)?;
                            self.set_own(id, &ks, v, true);
                        }
                        ObjProp::Spread(e) => {
                            // `{...o}` — enumerable own props only.
                            let v = self.eval_expr(env, this, e)?;
                            if let Value::Obj(sid) = v {
                                for k in self.own_enum_keys_trapped(sid)? {
                                    let sv = self.get_prop_value(v, &k)?;
                                    self.set_own(id, &k, sv, true);
                                }
                            }
                        }
                        ObjProp::Getter(k, f) | ObjProp::Setter(k, f) => {
                            let ks = self.eval_prop_key(env, this, k)?;
                            let fv = self.make_function(f, env, this);
                            let (get, set) = if matches!(p, ObjProp::Getter(..)) {
                                (fv, Value::Undefined)
                            } else {
                                (Value::Undefined, fv)
                            };
                            if let Some(o) = self.heap.obj_mut(id) {
                                o.set_accessor(&ks, get, set, true);
                            }
                        }
                    }
                }
                Ok(Value::Obj(id))
            }
            Expr::Function(lit) => {
                if let Some(name) = &lit.name {
                    // Named function expression: name binds inside only.
                    let scope = Env::local(env.clone());
                    let fv = self.make_function(lit, &scope, this);
                    if let EnvKind::Local(m) = &mut scope.borrow_mut().kind {
                        m.insert(
                            name.clone(),
                            EnvCell::Bind(Binding {
                                value: fv,
                                mutable: true,
                            }),
                        );
                    }
                    Ok(fv)
                } else {
                    Ok(self.make_function(lit, env, this))
                }
            }
            Expr::Template { parts, exprs } => {
                let mut out = String::new();
                for (i, part) in parts.iter().enumerate() {
                    out.push_str(part);
                    if let Some(e) = exprs.get(i) {
                        let v = self.eval_expr(env, this, e)?;
                        out.push_str(&self.to_js_string(v)?);
                    }
                }
                Ok(self.str_val(out))
            }
            // `` tag`…` `` — même résolution de callee qu'un appel
            // (`o.m`tpl`` passe `o` en `this`). `strings` porte le
            // tableau `raw` des chunks bruts (spec : les deux sont
            // non extensibles — le cache par site n'est pas fait,
            // déviation bénigne).
            Expr::TaggedTemplate {
                tag,
                parts,
                raws,
                exprs,
            } => {
                let (f, recv) = match &**tag {
                    Expr::Member(base, name) => {
                        let b = self.eval_expr(env, this, base)?;
                        (self.get_prop_value(b, name)?, b)
                    }
                    Expr::Index(base, key) => {
                        let b = self.eval_expr(env, this, base)?;
                        let kv = self.eval_expr(env, this, key)?;
                        let k = self.prop_key(kv)?;
                        (self.get_prop_value(b, &k)?, b)
                    }
                    _ => (self.eval_expr(env, this, tag)?, Value::Undefined),
                };
                let raw_vals: Vec<Value> = raws.iter().map(|s| self.str_val(s.clone())).collect();
                let raw = self.new_array(raw_vals);
                if let Value::Obj(rid) = raw {
                    if let Some(o) = self.heap.obj_mut(rid) {
                        o.extensible = false;
                    }
                }
                let cooked: Vec<Value> = parts.iter().map(|s| self.str_val(s.clone())).collect();
                let strings = self.new_array(cooked);
                if let Value::Obj(soid) = strings {
                    self.set_own(soid, "raw", raw, false);
                    if let Some(o) = self.heap.obj_mut(soid) {
                        o.extensible = false;
                    }
                }
                let mut args = vec![strings];
                for e in exprs {
                    args.push(self.eval_expr(env, this, e)?);
                }
                self.call_value(f, recv, &args)
            }
            Expr::Chain { head, steps } => self.eval_chain(env, this, head, steps),
            Expr::Regex { pattern, flags } => self.make_regexp(pattern, flags),
            Expr::Class(cl) => self.eval_class(env, this, cl),
            Expr::Spread(_) => Err(self.type_error("`...` outside a call or array literal")),
            Expr::Await(e) => {
                let v = self.eval_expr(env, this, e)?;
                self.await_value(v)
            }
            Expr::Yield { arg, delegate } => {
                if self.gen_collect.is_empty() {
                    // `yield` hors générateur — le parseur le
                    // refuse, garde-fou pour les chemins détournés.
                    return Err(self.syntax_error("yield outside a generator"));
                }
                if *delegate {
                    let Some(ae) = arg else {
                        return Err(self.syntax_error("yield* needs an operand"));
                    };
                    let v = self.eval_expr(env, this, ae)?;
                    let items = self.iter_values_await(v)?;
                    if let Some(c) = self.gen_collect.last_mut() {
                        c.extend(items);
                    }
                } else {
                    let v = match arg {
                        Some(e) => self.eval_expr(env, this, e)?,
                        None => Value::Undefined,
                    };
                    if let Some(c) = self.gen_collect.last_mut() {
                        c.push(v);
                    }
                }
                // `next(v)` est indisponible en mode eager — le
                // résultat de l'expression `yield` est `undefined`.
                Ok(Value::Undefined)
            }
            // `import(spec)` — dynamic import: loads, instantiates
            // and evaluates the module, then resolves with its
            // namespace object. Errors reject the promise.
            Expr::ImportCall(e) => {
                let sv = self.eval_expr(env, this, e)?;
                let spec = self.to_js_string(sv)?;
                let pid = self.new_promise();
                match self.dynamic_import(&spec) {
                    Ok(ns) => self.resolve_promise(pid, ns),
                    Err(Thrown(t)) => self.reject_promise(pid, t),
                }
                Ok(Value::Obj(pid))
            }
            // `import.meta` — `{url: <specifier courant>}`.
            Expr::ImportMeta => {
                let obj = self.new_object();
                let url = self.str_val(self.current_module.clone());
                self.set_own(obj, "url", url, false);
                Ok(Value::Obj(obj))
            }
            Expr::SuperCall(arg_exprs) => {
                let parent = self.env_get(env, SUPER_BIND);
                let Some(parent) = parent else {
                    return Err(self.type_error("`super()` outside a constructor"));
                };
                if !self.is_callable(parent) {
                    return Err(self.type_error("superclass is not a constructor"));
                }
                // Double `super()` → TypeError (spec, `this` déjà lié).
                let Value::Obj(tid) = this else {
                    return Err(self.type_error("`super()` requires an object `this`"));
                };
                if self.find_own(tid, FLD_DONE).is_some() {
                    return Err(self.type_error("super() may only be called once"));
                }
                // `new.target` de l'englobant se propage (spec :
                // il reste lié au constructeur d'origine toute la
                // chaîne `super`).
                let nt = self.env_get(env, NEW_TARGET).unwrap_or(Value::Undefined);
                // Champs d'instance du parent base : installés à
                // l'entrée de son corps — `super()` l'appelle comme
                // fonction ordinaire, on pose la liste avant.
                if let Value::Obj(pid) = parent {
                    self.install_class_fields(pid, FLD_FIELDS, this, nt)?;
                }
                let args = self.eval_args(env, this, arg_exprs)?;
                self.call_value_ctor(parent, this, &args, nt)?;
                self.set_own(tid, FLD_DONE, Value::Bool(true), false);
                // Nos champs de classe dérivée : installés juste
                // après `super()` — le constructeur de *cette*
                // classe est la fermeture courante (`HOME_BIND`).
                let own_ctor = self.env_get(env, HOME_BIND);
                if let Some(Value::Obj(cid)) = own_ctor {
                    self.install_class_fields(cid, FLD_FIELDS_D, this, nt)?;
                }
                Ok(this)
            }
            // `new.target` — liaison posée par `call_value_impl` :
            // undefined en appel ordinaire, ctor cible sous `new`.
            Expr::NewTarget => Ok(self.env_get(env, NEW_TARGET).unwrap_or(Value::Undefined)),
            Expr::SuperMember(name) => {
                let base = self.super_base(env)?;
                self.read_prop(base, this, name)
            }
            Expr::SuperIndex(k) => {
                let base = self.super_base(env)?;
                let kv = self.eval_expr(env, this, k)?;
                let key = self.prop_key(kv)?;
                self.read_prop(base, this, &key)
            }
            Expr::Member(base, name) => {
                let b = self.eval_expr(env, this, base)?;
                self.get_prop_value(b, name)
            }
            Expr::Index(base, key) => {
                let b = self.eval_expr(env, this, base)?;
                let k = self.eval_expr(env, this, key)?;
                let k = self.prop_key(k)?;
                self.get_prop_value(b, &k)
            }
            Expr::Call(callee, arg_exprs) => {
                enum Callee {
                    /// `expr(…)` — f pre-evaluated, global receiver.
                    Bare(Value),
                    /// `o.m(…)`/`o[k](…)` — receiver is the base.
                    Member(Value, String),
                    /// `super.m(…)`/`super[k](…)` — home-object proto
                    /// lookup, receiver stays `this`.
                    Super(String),
                }
                // `eval(…)` écrit tel quel → appel direct (spec
                // `PerformEval`) : la callee résolue doit être *le*
                // builtin `eval` global, pas un homonyme local.
                let direct_eval = matches!(&**callee, Expr::Ident(n) if n == "eval");
                let callee = match &**callee {
                    Expr::Member(base, name) => {
                        let b = self.eval_expr(env, this, base)?;
                        Callee::Member(b, name.clone())
                    }
                    Expr::Index(base, key) => {
                        let b = self.eval_expr(env, this, base)?;
                        let k = self.eval_expr(env, this, key)?;
                        Callee::Member(b, self.prop_key(k)?)
                    }
                    Expr::SuperMember(name) => Callee::Super(name.clone()),
                    Expr::SuperIndex(key) => {
                        let k = self.eval_expr(env, this, key)?;
                        Callee::Super(self.prop_key(k)?)
                    }
                    _ => Callee::Bare(self.eval_expr(env, this, callee)?),
                };
                let args = self.eval_args(env, this, arg_exprs)?;
                match callee {
                    // Receiver `undefined` — le callee sloppy le
                    // coercera vers l'objet global, le strict le
                    // garde (spec `OrdinaryCallBindThis`).
                    Callee::Bare(f) => {
                        if direct_eval
                            && matches!((f, self.eval_fn), (Value::Obj(a), Value::Obj(b)) if a == b)
                        {
                            return match args.first() {
                                Some(Value::Str(sid)) => {
                                    let src = self.heap.string(*sid).unwrap_or_default();
                                    self.eval_source_in(env, this, &src)
                                }
                                // Non-string → rendu tel quel (spec).
                                Some(v) => Ok(*v),
                                None => Ok(Value::Undefined),
                            };
                        }
                        self.call_value(f, Value::Undefined, &args)
                    }
                    Callee::Member(recv, name) => {
                        // Host method dispatch first: `hostObj.m(…)`
                        // hits the host's `invoke` before prop lookup.
                        if let Value::Obj(bid) = recv {
                            if let Some(h) = self.host_of(bid) {
                                if let Some(v) = h.invoke(self, recv, &name, &args)? {
                                    return Ok(v);
                                }
                            }
                        }
                        let f = self.get_prop_value(recv, &name)?;
                        self.call_value(f, recv, &args)
                    }
                    Callee::Super(name) => {
                        let base = self.super_base(env)?;
                        let f = self.read_prop(base, this, &name)?;
                        self.call_value(f, this, &args)
                    }
                }
            }
            Expr::New(callee, arg_exprs) => {
                let f = self.eval_expr(env, this, callee)?;
                let args = self.eval_args(env, this, arg_exprs)?;
                self.construct(f, &args)
            }
            Expr::Seq(parts) => {
                let mut last = Value::Undefined;
                for p in parts {
                    last = self.eval_expr(env, this, p)?;
                }
                Ok(last)
            }
            Expr::Cond(c, t, f) => {
                let c = self.eval_expr(env, this, c)?;
                if self.to_boolean(c) {
                    self.eval_expr(env, this, t)
                } else {
                    self.eval_expr(env, this, f)
                }
            }
            Expr::Assign(lhs, op, rhs) => {
                if *op == AssignOp::Set {
                    let v = self.eval_expr(env, this, rhs)?;
                    self.write_lvalue(env, this, lhs, v)?;
                    return Ok(v);
                }
                // Logical assignment: rhs evaluated only when needed.
                let cur = self.read_lvalue(env, this, lhs)?;
                match op {
                    AssignOp::Nullish if !matches!(cur, Value::Null | Value::Undefined) => {
                        return Ok(cur)
                    }
                    AssignOp::LogAnd if !self.to_boolean(cur) => return Ok(cur),
                    AssignOp::LogOr if self.to_boolean(cur) => return Ok(cur),
                    AssignOp::Nullish | AssignOp::LogAnd | AssignOp::LogOr => {
                        let v = self.eval_expr(env, this, rhs)?;
                        self.write_lvalue(env, this, lhs, v)?;
                        return Ok(v);
                    }
                    _ => {}
                }
                let rv = self.eval_expr(env, this, rhs)?;
                let bin = match op {
                    AssignOp::Add => BinOp::Add,
                    AssignOp::Sub => BinOp::Sub,
                    AssignOp::Mul => BinOp::Mul,
                    AssignOp::Div => BinOp::Div,
                    AssignOp::Mod => BinOp::Mod,
                    AssignOp::Shl => BinOp::Shl,
                    AssignOp::Shr => BinOp::Shr,
                    AssignOp::ShrU => BinOp::ShrU,
                    AssignOp::BitAnd => BinOp::BitAnd,
                    AssignOp::BitOr => BinOp::BitOr,
                    AssignOp::BitXor => BinOp::BitXor,
                    AssignOp::Pow => BinOp::Pow,
                    AssignOp::Set | AssignOp::Nullish | AssignOp::LogAnd | AssignOp::LogOr => {
                        return Err(self.type_error("invalid assignment"))
                    }
                };
                let v = self.binary_op(bin, cur, rv)?;
                self.write_lvalue(env, this, lhs, v)?;
                Ok(v)
            }
            Expr::Unary(op, operand) => self.unary_op(*op, env, this, operand),
            Expr::Binary(op, l, r) => {
                match op {
                    BinOp::And => {
                        let lv = self.eval_expr(env, this, l)?;
                        if !self.to_boolean(lv) {
                            return Ok(lv);
                        }
                        return self.eval_expr(env, this, r);
                    }
                    BinOp::Or => {
                        let lv = self.eval_expr(env, this, l)?;
                        if self.to_boolean(lv) {
                            return Ok(lv);
                        }
                        return self.eval_expr(env, this, r);
                    }
                    BinOp::Nullish => {
                        let lv = self.eval_expr(env, this, l)?;
                        if matches!(lv, Value::Null | Value::Undefined) {
                            return self.eval_expr(env, this, r);
                        }
                        return Ok(lv);
                    }
                    _ => {}
                }
                let lv = self.eval_expr(env, this, l)?;
                let rv = self.eval_expr(env, this, r)?;
                self.binary_op(*op, lv, rv)
            }
        }
    }

    fn read_lvalue(&mut self, env: &EnvRef, this: Value, e: &Expr) -> JsResult<Value> {
        match e {
            Expr::Ident(name) => self
                .env_get(env, name)
                .ok_or_else(|| self.ref_error(format!("{name} is not defined"))),
            Expr::Member(base, name) => {
                let b = self.eval_expr(env, this, base)?;
                self.get_prop_value(b, name)
            }
            Expr::Index(base, key) => {
                let b = self.eval_expr(env, this, base)?;
                let k = self.eval_expr(env, this, key)?;
                let k = self.prop_key(k)?;
                self.get_prop_value(b, &k)
            }
            _ => Err(self.type_error("invalid assignment target")),
        }
    }

    fn write_lvalue(&mut self, env: &EnvRef, this: Value, e: &Expr, v: Value) -> JsResult<()> {
        match e {
            Expr::Ident(name) => self.env_set(env, name, v),
            Expr::Member(base, name) => {
                let b = self.eval_expr(env, this, base)?;
                self.set_prop_value(b, name, v)
            }
            Expr::Index(base, key) => {
                let b = self.eval_expr(env, this, base)?;
                let k = self.eval_expr(env, this, key)?;
                let k = self.prop_key(k)?;
                self.set_prop_value(b, &k, v)
            }
            // `({a, b} = o)` / `[x, y] = arr` — destructuring.
            Expr::Object(_) | Expr::Array(_) => {
                let pat = expr_to_pat(e)
                    .ok_or_else(|| self.type_error("invalid destructuring target"))?;
                self.bind_pat(env, this, &pat, v, BindMode::Assign)
            }
            _ => Err(self.type_error("invalid assignment target")),
        }
    }

    fn unary_op(&mut self, op: UnOp, env: &EnvRef, this: Value, operand: &Expr) -> JsResult<Value> {
        match op {
            UnOp::Not => {
                let v = self.eval_expr(env, this, operand)?;
                Ok(Value::Bool(!self.to_boolean(v)))
            }
            UnOp::BitNot => {
                let v = self.eval_expr(env, this, operand)?;
                if let Value::Big(_) = v {
                    let b = self.to_bigint(v)?;
                    return Ok(self.big_val(-(b + num_bigint::BigInt::from(1))));
                }
                Ok(Value::Num(f64::from(!to_int32(self.to_number(v)?))))
            }
            UnOp::Pos => {
                let v = self.eval_expr(env, this, operand)?;
                // `+1n` → TypeError (spec `ToNumeric` ≠ `ToNumber` :
                // le `+` unaire refuse explicitement les BigInt).
                if matches!(v, Value::Big(_)) {
                    return Err(self.type_error("cannot convert a BigInt to number"));
                }
                Ok(Value::Num(self.to_number(v)?))
            }
            UnOp::Neg => {
                let v = self.eval_expr(env, this, operand)?;
                if let Value::Big(_) = v {
                    let b = self.to_bigint(v)?;
                    return Ok(self.big_val(-b));
                }
                Ok(Value::Num(-self.to_number(v)?))
            }
            UnOp::Void => {
                self.eval_expr(env, this, operand)?;
                Ok(Value::Undefined)
            }
            UnOp::Typeof => {
                // `typeof x` must not throw on undeclared x.
                if let Expr::Ident(name) = operand {
                    if let Some(v) = self.env_get(env, name) {
                        return Ok(self.str_val(typeof_str(v, &self.heap)));
                    }
                    return Ok(self.str_val("undefined"));
                }
                let v = self.eval_expr(env, this, operand)?;
                Ok(self.str_val(typeof_str(v, &self.heap)))
            }
            UnOp::Delete => match operand {
                Expr::Member(base, name) => {
                    let b = self.eval_expr(env, this, base)?;
                    if let Value::Obj(id) = b {
                        self.delete_result(id, name)
                    } else {
                        Ok(Value::Bool(true))
                    }
                }
                Expr::Index(base, key) => {
                    let b = self.eval_expr(env, this, base)?;
                    let k = self.eval_expr(env, this, key)?;
                    let k = self.prop_key(k)?;
                    if let Value::Obj(id) = b {
                        self.delete_result(id, &k)
                    } else {
                        Ok(Value::Bool(true))
                    }
                }
                // `delete x` : une liaison d'env (var/let/fn) est
                // non-supprimable → `false`; sinon la prop de
                // l'objet global — configurable incluse
                // (global implicite sloppy → `true`).
                Expr::Ident(name) => {
                    if Self::env_has_binding(env, name) {
                        if self.strict {
                            return Err(self.type_error(format!("cannot delete `{name}`")));
                        }
                        Ok(Value::Bool(false))
                    } else {
                        let g = self.global;
                        self.delete_result(g, name)
                    }
                }
                _ => {
                    self.eval_expr(env, this, operand).ok();
                    Ok(Value::Bool(true))
                }
            },
            UnOp::PreInc | UnOp::PreDec | UnOp::PostInc | UnOp::PostDec => {
                let cur = self.read_lvalue(env, this, operand)?;
                // `++`/`--` sur un bigint : ±1n, sans passer par
                // `ToNumber` (spec `Increment`/`Decrement`).
                if let Value::Big(_) = cur {
                    let b = self.to_bigint(cur)?;
                    let d = if matches!(op, UnOp::PreInc | UnOp::PostInc) {
                        num_bigint::BigInt::from(1)
                    } else {
                        num_bigint::BigInt::from(-1)
                    };
                    let new = self.big_val(b + d);
                    self.write_lvalue(env, this, operand, new)?;
                    return Ok(if matches!(op, UnOp::PostInc | UnOp::PostDec) {
                        cur
                    } else {
                        new
                    });
                }
                let n = self.to_number(cur)?;
                let d = if matches!(op, UnOp::PreInc | UnOp::PostInc) {
                    1.0
                } else {
                    -1.0
                };
                let new = n + d;
                self.write_lvalue(env, this, operand, Value::Num(new))?;
                Ok(Value::Num(if matches!(op, UnOp::PostInc | UnOp::PostDec) {
                    n
                } else {
                    new
                }))
            }
        }
    }

    fn binary_op(&mut self, op: BinOp, l: Value, r: Value) -> JsResult<Value> {
        use BinOp::{
            Add, And, BitAnd, BitOr, BitXor, Div, Eq, Ge, Gt, In, Instanceof, Le, Lt, Mod, Mul, Ne,
            Or, Pow, Shl, Shr, ShrU, StrictEq, StrictNe, Sub,
        };
        Ok(match op {
            Add => {
                let lp = self.to_primitive(l, PrimHint::Default)?;
                let rp = self.to_primitive(r, PrimHint::Default)?;
                if matches!(lp, Value::Str(_)) || matches!(rp, Value::Str(_)) {
                    let ls = self.to_js_string(lp)?;
                    let rs = self.to_js_string(rp)?;
                    self.str_val(format!("{ls}{rs}"))
                } else if matches!(lp, Value::Big(_)) || matches!(rp, Value::Big(_)) {
                    // `1n + 2n` → `3n` ; mixte Num/Big → `TypeError`
                    // via `to_bigint`.
                    let a = self.to_bigint(lp)?;
                    let b = self.to_bigint(rp)?;
                    self.big_val(a + b)
                } else {
                    Value::Num(self.to_number(lp)? + self.to_number(rp)?)
                }
            }
            Sub | Mul | Div | Mod | Pow | Shl | Shr | ShrU | BitAnd | BitOr | BitXor => {
                let lp = self.to_primitive(l, PrimHint::Number)?;
                let rp = self.to_primitive(r, PrimHint::Number)?;
                if matches!(lp, Value::Big(_)) || matches!(rp, Value::Big(_)) {
                    return self.big_binary(op, lp, rp);
                }
                match op {
                    Sub => Value::Num(self.to_number(lp)? - self.to_number(rp)?),
                    Mul => Value::Num(self.to_number(lp)? * self.to_number(rp)?),
                    Div => Value::Num(self.to_number(lp)? / self.to_number(rp)?),
                    Mod => Value::Num(self.to_number(lp)? % self.to_number(rp)?),
                    Pow => Value::Num(self.to_number(lp)?.powf(self.to_number(rp)?)),
                    Shl => Value::Num(f64::from(
                        to_int32(self.to_number(lp)?) << (to_uint32(self.to_number(rp)?) & 31),
                    )),
                    Shr => Value::Num(f64::from(
                        to_int32(self.to_number(lp)?) >> (to_uint32(self.to_number(rp)?) & 31),
                    )),
                    ShrU => Value::Num(f64::from(
                        to_uint32(self.to_number(lp)?) >> (to_uint32(self.to_number(rp)?) & 31),
                    )),
                    BitAnd => Value::Num(f64::from(
                        to_int32(self.to_number(lp)?) & to_int32(self.to_number(rp)?),
                    )),
                    BitOr => Value::Num(f64::from(
                        to_int32(self.to_number(lp)?) | to_int32(self.to_number(rp)?),
                    )),
                    _ => Value::Num(f64::from(
                        to_int32(self.to_number(lp)?) ^ to_int32(self.to_number(rp)?),
                    )),
                }
            }
            Lt | Le | Gt | Ge => {
                let lp = self.to_primitive(l, PrimHint::Number)?;
                let rp = self.to_primitive(r, PrimHint::Number)?;
                let ord = if matches!(lp, Value::Big(_)) || matches!(rp, Value::Big(_)) {
                    self.big_rel(lp, rp)?
                } else if let (Value::Str(a), Value::Str(b)) = (lp, rp) {
                    let a = self.heap.string(a).unwrap_or_default();
                    let b = self.heap.string(b).unwrap_or_default();
                    Some(a.cmp(&b))
                } else {
                    let a = self.to_number(lp)?;
                    let b = self.to_number(rp)?;
                    a.partial_cmp(&b)
                };
                let b = matches!(
                    (ord, op),
                    (Some(std::cmp::Ordering::Less), Lt | Le)
                        | (Some(std::cmp::Ordering::Equal), Le | Ge)
                        | (Some(std::cmp::Ordering::Greater), Gt | Ge)
                );
                Value::Bool(b)
            }
            Eq => Value::Bool(self.loose_eq(l, r)?),
            Ne => Value::Bool(!self.loose_eq(l, r)?),
            StrictEq => Value::Bool(strict_eq(l, r, &self.heap)),
            StrictNe => Value::Bool(!strict_eq(l, r, &self.heap)),
            In => {
                let Value::Obj(id) = r else {
                    return Err(self.type_error("`in` requires an object"));
                };
                let k = self.prop_key(l)?;
                Value::Bool(self.has_prop(id, &k)?)
            }
            Instanceof => Value::Bool(self.instance_of(l, r)?),
            // `&&`/`||`/`??` short-circuit in eval — never reach here.
            And | Or | BinOp::Nullish => return Err(self.type_error("invalid operator")),
        })
    }

    /// `a?.b.c(…)` — optional-chaining evaluation. Plain steps run
    /// normally; a `?.` step short-circuits the whole rest of the
    /// chain to `undefined` on a nullish base.
    fn eval_chain(
        &mut self,
        env: &EnvRef,
        this: Value,
        head: &Expr,
        steps: &[ChainStep],
    ) -> JsResult<Value> {
        let mut cur = self.eval_expr(env, this, head)?;
        // Receiver for `.m()`-style calls: the object the callee was
        // read from (plain calls get the global, per dynamic `this`).
        let mut recv = Value::Obj(self.global);
        for step in steps {
            match step {
                ChainStep::Member(name) => {
                    recv = cur;
                    cur = self.get_prop_value(cur, name)?;
                }
                ChainStep::Index(k) => {
                    let kv = self.eval_expr(env, this, k)?;
                    let key = self.prop_key(kv)?;
                    recv = cur;
                    cur = self.get_prop_value(cur, &key)?;
                }
                ChainStep::OptMember(name) => {
                    if matches!(cur, Value::Null | Value::Undefined) {
                        return Ok(Value::Undefined);
                    }
                    recv = cur;
                    cur = self.get_prop_value(cur, name)?;
                }
                ChainStep::OptIndex(k) => {
                    if matches!(cur, Value::Null | Value::Undefined) {
                        return Ok(Value::Undefined);
                    }
                    let kv = self.eval_expr(env, this, k)?;
                    let key = self.prop_key(kv)?;
                    recv = cur;
                    cur = self.get_prop_value(cur, &key)?;
                }
                ChainStep::Call(arg_exprs) | ChainStep::OptCall(arg_exprs) => {
                    if matches!(step, ChainStep::OptCall(_))
                        && matches!(cur, Value::Null | Value::Undefined)
                    {
                        return Ok(Value::Undefined);
                    }
                    let mut args = Vec::with_capacity(arg_exprs.len());
                    for a in arg_exprs {
                        args.push(self.eval_expr(env, this, a)?);
                    }
                    // Host method dispatch isn't reachable here (the
                    // callee was already read); fall through to
                    // ordinary `call_value` with the tracked receiver.
                    cur = self.call_value(cur, recv, &args)?;
                    recv = Value::Obj(self.global);
                }
            }
        }
        Ok(cur)
    }

    /// `==` per ES5 §11.9.3.
    fn loose_eq(&mut self, l: Value, r: Value) -> JsResult<bool> {
        if strict_eq(l, r, &self.heap) {
            return Ok(true);
        }
        match (l, r) {
            (Value::Null, Value::Undefined) | (Value::Undefined, Value::Null) => Ok(true),
            // BigInt loose-eq (spec `IsLooselyEqual`) : Num →
            // égalité mathématique, Str → `StringToBigInt`,
            // Bool → 0n/1n, objet → `ToPrimitive` puis re-tour.
            // Ces bras passent avant `(Bool, _)` pour capter
            // `(BigInt, Bool)`.
            (Value::Big(_), Value::Num(n)) | (Value::Num(n), Value::Big(_)) => {
                let bv = if matches!(l, Value::Big(_)) { l } else { r };
                Ok(self.bigint_of(bv).is_some_and(|b| big_eq_num(&b, n)))
            }
            (Value::Big(_), Value::Str(_)) | (Value::Str(_), Value::Big(_)) => {
                let bv = if matches!(l, Value::Big(_)) { l } else { r };
                let sv = if matches!(l, Value::Str(_)) { l } else { r };
                match self.to_bigint(sv) {
                    Ok(sb) => Ok(self.bigint_of(bv).is_some_and(|b| b == sb)),
                    Err(_) => Ok(false), // `StringToBigInt` invalide → pas égal.
                }
            }
            (Value::Big(_), Value::Bool(b)) | (Value::Bool(b), Value::Big(_)) => {
                let bv = if matches!(l, Value::Big(_)) { l } else { r };
                Ok(self
                    .bigint_of(bv)
                    .is_some_and(|v| v == num_bigint::BigInt::from(u8::from(b))))
            }
            (Value::Num(_), Value::Str(_))
            | (Value::Str(_), Value::Num(_))
            | (Value::Bool(_), _)
            | (_, Value::Bool(_)) => {
                let ln = self.to_number(l)?;
                let rn = self.to_number(r)?;
                Ok(ln == rn)
            }
            (Value::Obj(_), Value::Num(_) | Value::Str(_) | Value::Big(_)) => {
                let p = self.to_primitive(l, PrimHint::Default)?;
                self.loose_eq(p, r)
            }
            (Value::Num(_) | Value::Str(_) | Value::Big(_), Value::Obj(_)) => {
                let p = self.to_primitive(r, PrimHint::Default)?;
                self.loose_eq(l, p)
            }
            _ => Ok(false),
        }
    }

    /// Arithmétique/bitwise sur opérandes dont au moins une est
    /// `BigInt` — `TypeError` si l'autre n'est pas `ToBigInt`-able
    /// (mixte `Number`/`BigInt` interdit par la spec).
    fn big_binary(&mut self, op: BinOp, l: Value, r: Value) -> JsResult<Value> {
        use num_traits::{Signed, Zero};
        let a = self.to_bigint(l)?;
        let b = self.to_bigint(r)?;
        let one = num_bigint::BigInt::from(1);
        let v = match op {
            BinOp::Sub => a - b,
            BinOp::Mul => a * b,
            BinOp::Div => {
                if b.is_zero() {
                    return Err(self.range_error("BigInt division by zero"));
                }
                a / b // num-bigint `/` tronque vers 0 — sémantique spec.
            }
            BinOp::Mod => {
                if b.is_zero() {
                    return Err(self.range_error("BigInt division by zero"));
                }
                a % b // reste signe-dividende — sémantique spec.
            }
            BinOp::Pow => {
                if b.is_negative() {
                    return Err(self.range_error("BigInt negative exponent"));
                }
                let Ok(e) = u32::try_from(&b) else {
                    return Err(self.range_error("BigInt exponent too large"));
                };
                a.pow(e)
            }
            BinOp::Shl | BinOp::Shr => {
                // Décalage signé (négatif = sens inverse). Borné à
                // 1 M de bits — au-delà la valeur exploserait la
                // mémoire (deviation documentée, spec n'borne pas).
                let shift = b.abs();
                if shift > num_bigint::BigInt::from(1_000_000u32) {
                    return Err(self.range_error("BigInt shift too large"));
                }
                let Ok(s) = usize::try_from(&shift) else {
                    return Err(self.range_error("BigInt shift too large"));
                };
                match (op, b.is_negative()) {
                    (BinOp::Shl, false) | (BinOp::Shr, true) => a << s,
                    _ => a >> s,
                }
            }
            BinOp::BitAnd => a & b,
            BinOp::BitOr => a | b,
            BinOp::BitXor => a ^ b,
            BinOp::ShrU => {
                return Err(self.type_error("BigInts have no unsigned right shift"));
            }
            _ => return Err(self.type_error("invalid BigInt operator")),
        };
        let _ = one;
        Ok(self.big_val(v))
    }

    /// `bigint <rel> x` — `Ordering` mathématique (`None` → `false`,
    /// ex. comparaison contre `NaN` ou chaîne non-entière).
    fn big_rel(&mut self, l: Value, r: Value) -> JsResult<Option<std::cmp::Ordering>> {
        // `StringToBigInt` d'un côté si chaîne, numérique sinon.
        let cmp_num = |e: &mut Self,
                       bv: num_bigint::BigInt,
                       o: Value|
         -> JsResult<Option<std::cmp::Ordering>> {
            if let Value::Num(n) = o {
                return Ok(big_cmp_num(&bv, n));
            }
            let n = e.to_number(o)?;
            Ok(big_cmp_num(&bv, n))
        };
        match (l, r) {
            (Value::Big(_), Value::Big(_)) => {
                let a = self.bigint_of(l).unwrap_or_default();
                let b = self.bigint_of(r).unwrap_or_default();
                Ok(Some(a.cmp(&b)))
            }
            (Value::Big(_), Value::Str(_)) => {
                let a = self.bigint_of(l).unwrap_or_default();
                match self.to_bigint(r) {
                    Ok(b) => Ok(Some(a.cmp(&b))),
                    Err(_) => Ok(None),
                }
            }
            (Value::Str(_), Value::Big(_)) => {
                let b = self.bigint_of(r).unwrap_or_default();
                match self.to_bigint(l) {
                    Ok(a) => Ok(Some(a.cmp(&b))),
                    Err(_) => Ok(None),
                }
            }
            (Value::Big(_), o) => {
                let a = self.bigint_of(l).unwrap_or_default();
                cmp_num(self, a, o)
            }
            (o, Value::Big(_)) => {
                let b = self.bigint_of(r).unwrap_or_default();
                match cmp_num(self, b, o)? {
                    Some(ord) => Ok(Some(ord.reverse())),
                    None => Ok(None),
                }
            }
            _ => Ok(None),
        }
    }
}

/// `bigint == number` — égalité mathématique (spec `IsLooselyEqual`
/// étape : le `number` doit être fini et entier).
fn big_eq_num(b: &num_bigint::BigInt, n: f64) -> bool {
    if !n.is_finite() || n.fract() != 0.0 {
        return false;
    }
    num_traits::FromPrimitive::from_f64(n).is_some_and(|nb| b == &nb)
}

/// `bigint <op> number` — comparaison mathématique exacte :
/// `None` si `n` est `NaN`. Pour `n` fractionnaire, le bigint
/// (entier) est `Less` ssi `b <= floor(n)`, `Greater` ssi
/// `b >= ceil(n)` — il ne peut jamais égaler.
fn big_cmp_num(b: &num_bigint::BigInt, n: f64) -> Option<std::cmp::Ordering> {
    if n.is_nan() {
        return None;
    }
    if n.is_infinite() {
        return Some(if n < 0.0 {
            std::cmp::Ordering::Greater
        } else {
            std::cmp::Ordering::Less
        });
    }
    let f = n.floor();
    let lo: num_bigint::BigInt = num_traits::FromPrimitive::from_f64(f)?;
    match b.cmp(&lo) {
        std::cmp::Ordering::Less => Some(std::cmp::Ordering::Less),
        std::cmp::Ordering::Greater => Some(std::cmp::Ordering::Greater),
        std::cmp::Ordering::Equal => {
            if n.fract() == 0.0 {
                Some(std::cmp::Ordering::Equal)
            } else {
                Some(std::cmp::Ordering::Greater) // b == floor(n) < n
            }
        }
    }
}

/// `===` (no user code runs).
pub(crate) fn strict_eq(l: Value, r: Value, heap: &Heap) -> bool {
    match (l, r) {
        (Value::Num(a), Value::Num(b)) => a == b, // NaN != NaN via f64 ==
        (Value::Str(a), Value::Str(b)) => {
            if a == b {
                true
            } else {
                heap.string(a) == heap.string(b)
            }
        }
        (Value::Big(a), Value::Big(b)) => {
            if a == b {
                true
            } else {
                matches!(
                    (heap.obj(a).map(|o| &o.kind), heap.obj(b).map(|o| &o.kind)),
                    (Some(ObjKind::BigInt(x)), Some(ObjKind::BigInt(y))) if x == y
                )
            }
        }
        _ => l == r,
    }
}

fn typeof_str(v: Value, heap: &Heap) -> &'static str {
    match v {
        Value::Undefined => "undefined",
        Value::Null => "object",
        Value::Bool(_) => "boolean",
        Value::Num(_) => "number",
        Value::Str(_) => "string",
        Value::Big(_) => "bigint",
        Value::Obj(mut id) => loop {
            match heap.obj(id).map(|o| &o.kind) {
                Some(ObjKind::Fun(_)) => break "function",
                Some(ObjKind::Symbol { .. }) => break "symbol",
                // `typeof proxy` = `typeof target` (révoqué → object :
                // la spec lève TypeError — approximation documentée).
                Some(ObjKind::Proxy { target, handler }) => match (handler.is_some(), target) {
                    (true, Value::Obj(tid)) => id = *tid,
                    _ => break "object",
                },
                _ => break "object",
            }
        },
    }
}

/// `ToNumber` on strings: trim, hex, `Infinity`, decimal.
fn parse_js_number(s: &str) -> f64 {
    let t = s.trim();
    if t.is_empty() {
        return 0.0;
    }
    let (sign, rest) = match t.strip_prefix('-') {
        Some(r) => (-1.0, r),
        None => (1.0, t.strip_prefix('+').unwrap_or(t)),
    };
    if rest.eq_ignore_ascii_case("infinity") {
        return sign * f64::INFINITY;
    }
    if let Some(h) = rest.strip_prefix("0x").or_else(|| rest.strip_prefix("0X")) {
        return u64::from_str_radix(h, 16).map_or(f64::NAN, |v| sign * v as f64);
    }
    rest.parse::<f64>().map_or(f64::NAN, |v| sign * v)
}

/// Is `key` a canonical array index (`"0"`, `"1"`, … no leading zeros)?
pub(crate) fn array_index(key: &str) -> Option<usize> {
    if key.is_empty() || !key.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    if key.len() > 1 && key.starts_with('0') {
        return None;
    }
    key.parse::<usize>().ok()
}

/// Convert an lhs expression to a [`Pat`] for destructuring
/// assignments (`[a] = x`, `({p: q} = o)`). `None` when the
/// expression isn't a valid pattern — the caller falls back to a
/// plain-lvalue error.
pub(crate) fn expr_to_pat(e: &Expr) -> Option<Pat> {
    match e {
        Expr::Ident(name) => Some(Pat::Ident(name.clone())),
        // `o.p`/`a[i]` destructuring targets assign through the
        // existing lvalue path.
        Expr::Member(..) | Expr::Index(..) => Some(Pat::Lvalue(e.clone())),
        Expr::Spread(inner) => expr_to_pat(inner).map(|p| Pat::Rest(Box::new(p))),
        Expr::Assign(l, AssignOp::Set, d) => {
            expr_to_pat(l).map(|p| Pat::Default(Box::new(p), d.clone()))
        }
        Expr::Array(elems) => {
            let mut out = Vec::with_capacity(elems.len());
            for el in elems {
                match el {
                    None => out.push(None),
                    Some(e) => out.push(Some(expr_to_pat(e)?)),
                }
            }
            Some(Pat::Array(out))
        }
        Expr::Object(props) => {
            let mut pats = Vec::with_capacity(props.len());
            let mut rest = None;
            for p in props {
                match p {
                    ObjProp::Prop(k, ve) => pats.push((k.clone(), expr_to_pat(ve)?)),
                    ObjProp::Spread(e) => rest = Some(Box::new(expr_to_pat(e)?)),
                    ObjProp::Getter(..) | ObjProp::Setter(..) => return None,
                }
            }
            Some(Pat::Object { props: pats, rest })
        }
        _ => None,
    }
}
