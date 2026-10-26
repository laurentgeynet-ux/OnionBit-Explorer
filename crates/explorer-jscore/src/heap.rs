// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Heap, objects, environments — the arena at the heart of `explorer-jscore`.
//!
//! Same ownership philosophy as ADR-0005's widget arena: every object
//! lives in `Heap.cells` and is referenced by `ObjId` (a `u32`), so
//! object graphs with cycles (prototype chains, closures capturing
//! scopes that capture functions) never need `Rc<RefCell>` puzzles.
//!
//! **GC mark-sweep** (ADR-0008, chantier JS-A) : les cellules mortes
//! deviennent des tombstones réutilisées par `alloc` — un `ObjId`
//! vers un slot réclamé résout en `None` (dégradation tolérante, pas
//! de dangling). Le point d'entrée [`Heap::collect`] est piloté par
//! [`crate::Engine::collect`] qui sème les racines (global,
//! prototypes, env, jobs, timers, table DOM↔JS). Les handles
//! explicites passent par [`Heap::add_root`]/[`Heap::remove_root`] —
//! le protocole est le même que celui du spike `explorer-dom`.
//!
//! Invariant : `collect()` ne doit pas être invoqué pendant une
//! évaluation — la pile d'appel n'est pas une racine (les envs
//! capturés par des fonctions vivantes sont tracés via les
//! objets `Fun::Script`, pas via la pile).

use crate::ast::FnLit;
use crate::value::{ObjId, Value};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

/// A JavaScript exception in flight — carries the thrown `Value`.
/// Distinct from [`crate::Error`] which is host-facing.
#[derive(Debug)]
pub struct Thrown(pub Value);

/// Result alias for interpreter code that can unwind via `throw`.
pub type JsResult<T> = Result<T, Thrown>;

/// Borne mémoire heap armée par défaut (ADR-0019) — 512 Mio,
/// 2× la garde d'alloc unitaire `MAX_SINGLE_ALLOC`. Assez
/// pour des graphes de page réels volumineux, assez bas pour
/// qu'un script hosti ou une preuve-de-travail ne puisse pas
/// emporter la machine : dépassée, le `tick` émet un
/// `RangeError` (« heap memory limit exceeded ») au lieu d'un
/// OOM processus. `Heap::set_mem_limit` le règle — `0`
/// restaure l'illimité pour les harnais qui en ont besoin.
pub(crate) const DEFAULT_MEM_LIMIT: usize = 512 * 1024 * 1024;

/// Native function signature. `this` is the receiver `Value`,
/// args are the call arguments.
pub type NativeFn = fn(&mut crate::Engine, Value, &[Value]) -> JsResult<Value>;

/// A heap cell: either an immutable string or an object.
pub enum CellData {
    /// String data (JS strings are immutable).
    Str(String),
    /// Object data.
    Obj(Object),
}

/// One arena slot's payload.
pub struct Cell {
    /// Payload.
    pub data: CellData,
}

/// One arena slot — `None` = tombstone réutilisable.
type Slot = Option<Cell>;

/// The object arena.
pub struct Heap {
    cells: Vec<Slot>,
    /// Indices de slots libres (sweep remplit, `alloc` consomme).
    free: Vec<u32>,
    /// Bits de marque de la passe en cours (`marks.len()` suit
    /// `cells.len()`).
    marks: Vec<bool>,
    /// Racines explicites (handles hôtes) — ref-count par id.
    roots: HashMap<ObjId, u32>,
    /// Live-object counter for tests/embedder metrics.
    pub(crate) allocated: usize,
    /// Estimation des octets vivants (JS-C37, ADR-0019) : augmentée
    /// à chaque `alloc`, recalculée exactement à chaque sweep. Les
    /// mutations post-alloc (props ajoutées) ne sont comptées
    /// qu'après une passe GC — borne approximative, documentée.
    bytes: usize,
    /// Borne mémoire par contexte en octets — `0` = illimité,
    /// sinon armée d'office à [`DEFAULT_MEM_LIMIT`] (ADR-0019 :
    /// un script hosti ne doit jamais pouvoir vider la machine —
    /// jusqu'ici la borne existait mais n'était armée nulle part,
    /// mesuré : challenge google + budget relevé = OOM station).
    /// Le dépassement lève `over_mem` ; le kill doux est émis au
    /// `tick` suivant (l'alloc elle-même ne peut pas échouer).
    mem_limit: usize,
    /// `true` tant que `bytes > mem_limit` (borne non nulle).
    over_mem: bool,
    /// Passes GC effectuées — métrique `heap_stats`.
    gc_runs: u64,
    /// Pic de cellules vivantes observé (post-sweep/alloc).
    peak_live: usize,
    /// Pic d'octets estimés.
    peak_bytes: usize,
}

impl Heap {
    /// Empty heap.
    pub fn new() -> Self {
        Heap {
            cells: Vec::new(),
            free: Vec::new(),
            marks: Vec::new(),
            roots: HashMap::new(),
            allocated: 0,
            bytes: 0,
            mem_limit: DEFAULT_MEM_LIMIT,
            over_mem: false,
            gc_runs: 0,
            peak_live: 0,
            peak_bytes: 0,
        }
    }

    fn alloc(&mut self, data: CellData) -> ObjId {
        self.bytes += cell_bytes(&data);
        let live = self.cells.len() - self.free.len() + 1;
        if live > self.peak_live {
            self.peak_live = live;
        }
        if self.bytes > self.peak_bytes {
            self.peak_bytes = self.bytes;
        }
        if self.mem_limit > 0 && self.bytes > self.mem_limit {
            self.over_mem = true;
        }
        if let Some(idx) = self.free.pop() {
            self.cells[idx as usize] = Some(Cell { data });
            return idx;
        }
        self.cells.push(Some(Cell { data }));
        self.marks.push(false);
        (self.cells.len() - 1) as ObjId
    }

    /// Allocate a string, returning its id.
    pub fn alloc_str(&mut self, s: impl Into<String>) -> ObjId {
        self.alloc(CellData::Str(s.into()))
    }

    /// Allocate an object.
    pub fn alloc_obj(&mut self, o: Object) -> ObjId {
        self.allocated += 1;
        self.alloc(CellData::Obj(o))
    }

    /// Borrow a string's contents (cloned — strings are short here).
    pub fn string(&self, id: ObjId) -> Option<String> {
        match self.cells.get(id as usize) {
            Some(Some(Cell {
                data: CellData::Str(s),
            })) => Some(s.clone()),
            _ => None,
        }
    }

    /// Borrow an object.
    pub fn obj(&self, id: ObjId) -> Option<&Object> {
        match self.cells.get(id as usize) {
            Some(Some(Cell {
                data: CellData::Obj(o),
            })) => Some(o),
            _ => None,
        }
    }

    /// Borrow an object mutably.
    pub fn obj_mut(&mut self, id: ObjId) -> Option<&mut Object> {
        match self.cells.get_mut(id as usize) {
            Some(Some(Cell {
                data: CellData::Obj(o),
            })) => Some(o),
            _ => None,
        }
    }

    /// `true` si `id` pointe une cellule vivante.
    pub fn is_alive(&self, id: ObjId) -> bool {
        self.cells.get(id as usize).is_some_and(Option::is_some)
    }

    /// Nombre de cellules vivantes (objets + chaînes).
    pub fn live_count(&self) -> usize {
        self.cells.len() - self.free.len()
    }

    // -----------------------------------------------------------------
    // Rooting explicite (ADR-0008)
    // -----------------------------------------------------------------

    /// Roote `id` — il survit aux `collect` jusqu'au `remove_root`
    /// correspondant (ref-count : plusieurs racines possibles).
    pub fn add_root(&mut self, id: ObjId) {
        *self.roots.entry(id).or_insert(0) += 1;
    }

    /// Retire une racine posée par [`Heap::add_root`].
    pub fn remove_root(&mut self, id: ObjId) {
        if let Some(n) = self.roots.get_mut(&id) {
            *n -= 1;
            if *n == 0 {
                self.roots.remove(&id);
            }
        }
    }

    /// `true` si `id` est actuellement rooté.
    pub fn is_rooted(&self, id: ObjId) -> bool {
        self.roots.contains_key(&id)
    }

    // -----------------------------------------------------------------
    // Mark-sweep
    // -----------------------------------------------------------------

    /// Extrait les `ObjId` référencés par la cellule `id` vers
    /// `out` — proto, props, éléments, charges de `ObjKind`, envs
    /// capturés (via [`trace_env`]).
    pub(crate) fn trace_cell(&self, id: ObjId, out: &mut Vec<ObjId>, envs: &mut HashSet<usize>) {
        let Some(Some(Cell {
            data: CellData::Obj(o),
        })) = self.cells.get(id as usize)
        else {
            return;
        };
        push_value(out, o.proto);
        for (_, p) in &o.props {
            match p {
                Prop::Data { value, .. } => push_value(out, *value),
                Prop::Acc { get, set, .. } => {
                    push_value(out, *get);
                    push_value(out, *set);
                }
            }
        }
        // L'env capturé par un `arguments` mappé est une racine —
        // ses slots paramètres alimentent les lectures d'indices.
        if let Some((env, _)) = &o.args_map {
            trace_env(env, out, envs);
        }
        match &o.kind {
            ObjKind::Array(elems) => {
                for &v in elems {
                    push_value(out, v);
                }
            }
            ObjKind::Fun(fun) => match fun {
                Fun::Script { env, lex_this, .. } => {
                    if let Some(t) = lex_this {
                        push_value(out, *t);
                    }
                    trace_env(env, out, envs);
                }
                Fun::BoundNative { bound, .. } => {
                    for &v in bound {
                        push_value(out, v);
                    }
                }
                Fun::Bound {
                    target,
                    this,
                    bound,
                } => {
                    push_value(out, *target);
                    push_value(out, *this);
                    for &v in bound {
                        push_value(out, v);
                    }
                }
                Fun::Native { .. } => {}
            },
            ObjKind::Host(h) => h.trace(out),
            ObjKind::Promise(p) => {
                push_value(out, p.result);
                push_value(out, p.resolve_fn);
                push_value(out, p.reject_fn);
                for r in &p.reactions {
                    push_value(out, r.on_fulfilled);
                    push_value(out, r.on_rejected);
                    out.push(r.next);
                }
            }
            ObjKind::Map(entries) => {
                for (k, v) in entries {
                    push_value(out, *k);
                    push_value(out, *v);
                }
            }
            ObjKind::Set(items) => {
                for &v in items {
                    push_value(out, v);
                }
            }
            ObjKind::Iter { items, ret, .. } => {
                for &v in items {
                    push_value(out, v);
                }
                push_value(out, *ret);
            }
            ObjKind::GenLive { exec } => {
                if let Ok(g) = exec.try_borrow() {
                    g.trace(out, envs);
                }
            }
            ObjKind::Proxy { target, handler } => {
                push_value(out, *target);
                if let Some(h) = handler {
                    push_value(out, *h);
                }
            }
            ObjKind::DataView { buffer, .. } | ObjKind::TypedArray { buffer, .. } => {
                push_value(out, *buffer);
            }
            ObjKind::FinReg { cb, entries } => {
                push_value(out, *cb);
                for en in entries {
                    push_value(out, en.held);
                    push_value(out, en.token);
                }
            }
            ObjKind::DispStack { stack, .. } => {
                for (f, t, v) in stack {
                    push_value(out, *f);
                    push_value(out, *t);
                    push_value(out, *v);
                }
            }
            // `WeakRef` : la cible n'est PAS tracée (c'est le
            // point — `deref()` observe la collecte).
            ObjKind::WeakRef(_)
            | ObjKind::Temp(_)
            | ObjKind::Ordinary
            | ObjKind::RegExp(_)
            | ObjKind::Symbol { .. }
            | ObjKind::FetchBody { .. }
            | ObjKind::ByteBuf(_)
            | ObjKind::Blob { .. }
            | ObjKind::BigInt(_) => {}
        }
    }

    /// Passe mark-sweep. `seed` sème les racines de l'Engine (la
    /// table `roots` explicite est toujours incluse). Retourne le
    /// nombre de cellules réclamées ; les host objects balayés
    /// reçoivent [`Host::finalize`] avant libération.
    pub fn collect<F>(&mut self, mut seed: F) -> usize
    where
        F: FnMut(&mut dyn FnMut(ObjId)),
    {
        // -- Mark --
        let mut worklist: Vec<ObjId> = Vec::new();
        let mut envs: HashSet<usize> = HashSet::new();
        let mut discovered: Vec<ObjId> = Vec::new();
        // Racines explicites (handles hôtes).
        for &id in self.roots.keys() {
            if (id as usize) < self.marks.len() && !self.marks[id as usize] {
                self.marks[id as usize] = true;
                worklist.push(id);
            }
        }
        // Racines Engine, semées via `seed`.
        let marks = &mut self.marks;
        seed(&mut |id| {
            if (id as usize) < marks.len() && !marks[id as usize] {
                marks[id as usize] = true;
                worklist.push(id);
            }
        });
        // Drain itératif — `trace_cell` pousse les référencements
        // dans `discovered`, on les marque en remettant.
        while let Some(id) = worklist.pop() {
            self.trace_cell(id, &mut discovered, &mut envs);
            for d in discovered.drain(..) {
                if !self.marks[d as usize] {
                    self.marks[d as usize] = true;
                    worklist.push(d);
                }
            }
        }
        // -- Sweep --
        let mut swept = 0usize;
        let mut live_bytes = 0usize;
        for (i, slot) in self.cells.iter_mut().enumerate() {
            if slot.is_some() && !self.marks[i] {
                if let Some(Cell {
                    data: CellData::Obj(o),
                }) = slot.as_ref()
                {
                    if let ObjKind::Host(h) = &o.kind {
                        h.finalize();
                    }
                }
                *slot = None;
                self.free.push(i as u32);
                swept += 1;
            } else if let Some(c) = slot.as_ref() {
                live_bytes += cell_bytes(&c.data);
            }
            self.marks[i] = false;
        }
        // Recal exact post-sweep : les props ajoutées depuis l'alloc
        // entrent dans le compte, l'estimation repart de zéro dérive.
        self.bytes = live_bytes;
        self.gc_runs += 1;
        let live = self.live_count();
        if live > self.peak_live {
            self.peak_live = live;
        }
        if self.bytes > self.peak_bytes {
            self.peak_bytes = self.bytes;
        }
        self.over_mem = self.mem_limit > 0 && self.bytes > self.mem_limit;
        swept
    }

    /// Borne mémoire par contexte (ADR-0019) — `0` = illimité.
    /// Le dépassement n'empêche pas l'alloc (signature non
    /// faillible) : il lève le drapeau `over_limit`, lu au `tick`
    /// par l'interp pour un kill doux borné.
    pub fn set_mem_limit(&mut self, bytes: usize) {
        self.mem_limit = bytes;
        self.over_mem = bytes > 0 && self.bytes > bytes;
    }

    /// `true` tant que l'estimation dépasse la borne configurée.
    pub fn over_limit(&self) -> bool {
        self.over_mem
    }

    /// Borne mémoire configurée (`0` = illimité) — relue par la
    /// garde d'allocation `Engine::alloc_guard` pour refuser les
    /// matérialisations hostiles avant l'allocateur (JS-C55).
    pub fn mem_limit(&self) -> usize {
        self.mem_limit
    }

    /// Métriques heap pour embedders/tests (JS-C37).
    pub fn stats(&self) -> HeapStats {
        HeapStats {
            live_cells: self.live_count(),
            estimated_bytes: self.bytes,
            peak_live_cells: self.peak_live,
            peak_bytes: self.peak_bytes,
            gc_runs: self.gc_runs,
        }
    }
}

/// Instantané des compteurs du heap (JS-C37).
#[derive(Debug, Clone, Copy)]
pub struct HeapStats {
    /// Cellules vivantes (objets + chaînes).
    pub live_cells: usize,
    /// Estimation des octets vivants (approximative hors sweep).
    pub estimated_bytes: usize,
    /// Pic de cellules vivantes.
    pub peak_live_cells: usize,
    /// Pic d'octets estimés.
    pub peak_bytes: usize,
    /// Nombre de passes mark-sweep exécutées.
    pub gc_runs: u64,
}

/// Coût estimé d'une cellule — approximation documentée : le
/// `String` compte sa longueur, l'`Object` son empreinte fixe plus
/// la capacité de sa table de props et la taille de ses clés (les
/// payloads `ObjKind` internes — `Vec` d'éléments, etc. — restent
/// hors compte : borne grossière, pas mesure exacte).
fn cell_bytes(data: &CellData) -> usize {
    let base = std::mem::size_of::<Cell>();
    match data {
        CellData::Str(s) => base + s.len(),
        CellData::Obj(o) => {
            base + std::mem::size_of::<Object>()
                + o.props.capacity() * std::mem::size_of::<(String, Prop)>()
                + o.props.iter().map(|(k, _)| k.len()).sum::<usize>()
                // Charges utiles octets (borne mémoire JS-C65 —
                // un upload géant ne doit pas passer sous le
                // compteur : `ByteBuf`/`Blob` comptent leur
                // contenu).
                + match &o.kind {
                    ObjKind::ByteBuf(b) => b.len(),
                    ObjKind::Blob { data, .. } => data.len(),
                    _ => 0,
                }
        }
    }
}

/// Pousse l'`ObjId` d'un `Value` (no-op pour les scalaires).
pub(crate) fn push_value(out: &mut Vec<ObjId>, v: Value) {
    match v {
        Value::Obj(i) | Value::Str(i) | Value::Big(i) => out.push(i),
        _ => {}
    }
}

/// Trace les valeurs des bindings de `env` et remonte la chaîne
/// `parent` — visited-set par adresse `Rc` (les envs peuvent boucler
/// via `env → binding → Fun::Script → env`).
pub(crate) fn trace_env(env: &EnvRef, out: &mut Vec<ObjId>, visited: &mut HashSet<usize>) {
    let mut stack = vec![env.clone()];
    while let Some(root) = stack.pop() {
        let mut cur = Some(root);
        while let Some(e) = cur.take() {
            let key = Rc::as_ptr(&e) as usize;
            if !visited.insert(key) {
                break;
            }
            let borrowed = e.borrow();
            match &borrowed.kind {
                EnvKind::Global(m) | EnvKind::Local(m) | EnvKind::Block(m) => {
                    for c in m.values() {
                        match c {
                            EnvCell::Bind(b) => push_value(out, b.value),
                            // Alias d'import : l'env du module exporteur est
                            // aussi une racine — ses liaisons restent
                            // vivantes tant qu'un importeur les référence.
                            EnvCell::Alias(target, _) => stack.push(target.clone()),
                        }
                    }
                }
                // `with (o)` — l'objet-scope vit tant que le bloc.
                EnvKind::With(oid) => push_value(out, Value::Obj(*oid)),
            }
            cur.clone_from(&borrowed.parent);
        }
    }
}

/// Property attributes — `[[Writable]]` (data only),
/// `[[Enumerable]]`, `[[Configurable]]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PropAttrs {
    /// `[[Writable]]` — meaningful on `Data` only.
    pub writable: bool,
    /// `[[Enumerable]]` — gates `for-in`/`Object.keys`.
    pub enumerable: bool,
    /// `[[Configurable]]` — gates `delete`/redefinition.
    pub configurable: bool,
}

impl PropAttrs {
    /// Ordinary assignment / literal prop : `wec = true`.
    pub const FULL: Self = Self {
        writable: true,
        enumerable: true,
        configurable: true,
    };
    /// Builtin install (methods, prototypes) : non-énumérable,
    /// réinscriptible et reconfigurable.
    pub const BUILTIN: Self = Self {
        writable: true,
        enumerable: false,
        configurable: true,
    };
    /// Constante figée (`NaN`, `undefined`, `Math.E`…) : `wec = false`.
    pub const FIXED: Self = Self {
        writable: false,
        enumerable: false,
        configurable: false,
    };
}

/// A property entry. `enumerable` gates `for-in` enumeration
/// (builtins install themselves as non-enumerable).
#[derive(Debug, Clone, Copy)]
pub enum Prop {
    /// Plain data property.
    Data {
        /// Stored value.
        value: Value,
        /// `[[Writable]]`/`[[Enumerable]]`/`[[Configurable]]`.
        attrs: PropAttrs,
    },
    /// Getter/setter pair (`{get x(){}}`, `Object.defineProperty`,
    /// class accessors). `Undefined` = absent side.
    Acc {
        /// Getter callable or `Undefined`.
        get: Value,
        /// Setter callable or `Undefined`.
        set: Value,
        /// `[[Enumerable]]`/`[[Configurable]]` (`writable` n/a).
        attrs: PropAttrs,
    },
}

impl Prop {
    /// `true` when the property shows up in `for-in`.
    pub fn enumerable(&self) -> bool {
        match self {
            Prop::Data { attrs, .. } | Prop::Acc { attrs, .. } => attrs.enumerable,
        }
    }

    /// `[[Configurable]]`.
    pub fn configurable(&self) -> bool {
        match self {
            Prop::Data { attrs, .. } | Prop::Acc { attrs, .. } => attrs.configurable,
        }
    }

    /// `[[Writable]]` — `false` for accessors.
    pub fn writable(&self) -> bool {
        match self {
            Prop::Data { attrs, .. } => attrs.writable,
            Prop::Acc { .. } => false,
        }
    }

    /// Raw value for `Data` — `None` for accessors (the engine reads
    /// those through the getter-invoking path).
    pub fn data_value(&self) -> Option<Value> {
        match self {
            Prop::Data { value, .. } => Some(*value),
            Prop::Acc { .. } => None,
        }
    }
}

/// A JS object: ordered own properties + prototype link + kind.
/// Properties are stored in insertion order — `for-in` order is
/// observable behaviour in old scripts.
pub struct Object {
    /// Prototype link (`Value::Null` or `Value::Obj`).
    pub proto: Value,
    /// Own properties, insertion-ordered.
    pub props: Vec<(String, Prop)>,
    /// Kind-specific payload.
    pub kind: ObjKind,
    /// `[[Extensible]]` — `Object.preventExtensions`/`seal`/`freeze`
    /// le passent à `false` : les nouvelles props sont alors refusées
    /// (silencieux en sloppy, `TypeError` en strict côté moteur).
    pub extensible: bool,
    /// Intégrité des *éléments* de tableau (spec
    /// `SetIntegrityLevel`), en bits : `1` = sealed (éléments
    /// non-configurables), `2` = frozen (non-writable aussi),
    /// `4` = `length` non-writable. Sans effet hors `ObjKind::Array`.
    pub elem_integrity: u8,
    /// Objet `arguments` : `Some((env, map))` où `map[i]` est le nom
    /// du paramètre formel aliasé avec l'élément `i` (spec
    /// `CreateMappedArgumentsObject` — fonctions sloppy à liste de
    /// paramètres simple). Map vide = objet `arguments` non mappé
    /// (strict ou params non simples) — sa présence distingue
    /// l'objet `arguments` d'un tableau ordinaire
    /// (`length.configurable`). `None` hors `arguments`.
    pub args_map: Option<(EnvRef, std::rc::Rc<Vec<Option<String>>>)>,
}

/// What kind of object this is.
/// Entrée `register(target, held, token)` d'une
/// [`ObjKind::FinReg`] — la `target` reste une référence faible.
pub struct FinRegEntry {
    /// Cible surveillée (non tracée).
    pub target: ObjId,
    /// Valeur passée au cleanup callback.
    pub held: Value,
    /// Jeton de `unregister` (`Undefined` si omis).
    pub token: Value,
}

/// Slots internes `Temporal.*` (JS-C22) — aucun `Value` tracé.
/// `f` pour Duration = `[years, months, weeks, days, hours,
/// minutes, seconds, milliseconds, microseconds, nanoseconds]`.
#[derive(Clone, Copy, Debug)]
pub enum TempData {
    /// `Temporal.Instant` — nanosecondes depuis l'époque UTC.
    Instant { ns: i128 },
    /// `Temporal.PlainDate` (calendrier iso8601).
    PDate { y: i64, m: u8, d: u8 },
    /// `Temporal.PlainTime` — `ms`/`us`/`ns` = milli/micro/nano
    /// du champ secondes (0..999 chacun).
    PTime {
        h: u8,
        mi: u8,
        s_: u8,
        ms: u16,
        us: u16,
        ns: u16,
    },
    /// `Temporal.PlainDateTime`.
    PDateTime {
        y: i64,
        m: u8,
        d: u8,
        h: u8,
        mi: u8,
        s_: u8,
        ms: u16,
        us: u16,
        ns: u16,
    },
    /// `Temporal.PlainYearMonth` (jour de référence iso = 1).
    PYearMonth { y: i64, m: u8 },
    /// `Temporal.PlainMonthDay` (année de référence iso = 1972,
    /// inobservable hors `monthCode`/`day`).
    PMonthDay { m: u8, d: u8 },
    /// `Temporal.Duration` — `[y, mo, w, d, h, mi, s, ms, us, ns]`,
    /// tous de même signe (spec `Duration` : mixte → `RangeError`).
    Duration { f: [i64; 10] },
}

pub enum ObjKind {
    /// `Temporal.*` — slots internes [`TempData`] (JS-C22).
    Temp(TempData),
    /// Plain `{}`.
    Ordinary,
    /// Dense array elements; `length` and numeric indices are derived.
    Array(Vec<Value>),
    /// Callable.
    Fun(Fun),
    /// Host-provided object (DOM node, `document`, style map…) —
    /// property access and method calls are delegated to a
    /// [`Host`] implementation living outside the engine.
    Host(Rc<dyn Host>),
    /// Compiled regular expression (`/re/` literal or `RegExp()`).
    RegExp(Rc<crate::regexp::JsRegExp>),
    /// `Promise` — state + reactions; see `promise.rs`.
    Promise(crate::promise::JsPromise),
    /// `Map` — insertion-ordered key/value pairs (spec order is
    /// observable in `forEach`/`keys()`).
    Map(Vec<(Value, Value)>),
    /// `Set` — insertion-ordered unique values.
    Set(Vec<Value>),
    /// Lazy iterator snapshot (`map.keys()`, `set.values()`…) —
    /// `next()` consumes it; produced by `Symbol.iterator` methods.
    Iter {
        /// Remaining items (consumed from index `idx`).
        items: Vec<Value>,
        /// Next index to yield.
        idx: usize,
        /// Terminal value — `{value, done:true}` after exhaustion
        /// (generator `return v`; `Undefined` for plain iterators).
        ret: Value,
    },
    /// Générateur lazy (`function*`) — exécuteur suspendable
    /// `GExec` (JS-C34) : pile de contrôle explicite partagée
    /// via `RefCell` (les natives `next`/`throw`/`return`
    /// reprennent le corps au dernier `yield`).
    GenLive {
        /// Exécuteur du corps (frames + env + pending).
        exec: std::rc::Rc<std::cell::RefCell<crate::interp::lazy::GExec>>,
    },
    /// `Symbol(desc)` — identité unique par alloc (`===` par
    /// `ObjId`, `typeof` → `"symbol"`). `key` est la clé-propriété
    /// unique du symbole : `\u{f130}sym{N}` pour les anonymes,
    /// sentinelle bien connue (`Symbol.iterator`…) pour les
    /// well-knowns — `o[sym]` indexe `key`, garantissant que deux
    /// symboles de même description ne collisionnent jamais.
    Symbol {
        /// Description affichée par `toString`/`description`.
        desc: Option<String>,
        /// Clé-propriété sentinelle unique.
        key: String,
    },
    /// `new WeakRef(target)` — la cible n'est **pas** tracée par
    /// le GC (vraie référence faible) ; `deref()` rend `undefined`
    /// une fois la cible collectée.
    WeakRef(ObjId),
    /// `new FinalizationRegistry(cb)` — `cb`, `held` et `token`
    /// sont tracés, les cibles non. Après chaque collecte, le
    /// moteur draine les entrées mortes et enfile `cb(held)` en
    /// microtask (spec : cleanup jamais synchrone).
    FinReg {
        /// Cleanup callback.
        cb: Value,
        /// `(target, heldValue, unregisterToken)` par `register`.
        entries: Vec<FinRegEntry>,
    },
    /// `new Proxy(target, handler)` — `[[ProxyTarget]]` +
    /// `[[ProxyHandler]]` (`None` une fois révoqué → toute
    /// opération lève `TypeError`). Toutes les méthodes internes
    /// sont interceptées par les traps côté moteur (interp.rs).
    Proxy {
        /// `[[ProxyTarget]]`.
        target: Value,
        /// `[[ProxyHandler]]` — `None` après `revoke()`.
        handler: Option<Value>,
    },
    /// `fetch` `Request`/`Response` body store (JS-C66) — the
    /// hidden payload read by the `Body` mixin; `headers`/`status`/
    /// `method`/`url`/`bodyUsed` are ordinary own props (headers
    /// lives in a `Headers`-shaped object, source of truth).
    FetchBody {
        /// Body text (lossy-decoded for network responses).
        body: String,
    },
    /// `ArrayBuffer` — fixed byte store shared by views (element
    /// writes through one view are visible through the others).
    ByteBuf(Vec<u8>),
    /// `Blob`/`File` (JS-C70) — octets immuables partagés par
    /// `Rc` (`slice`/`structuredClone` partagent le stockage,
    /// spec « snapshot state ») ; métadonnées `File` (`name`,
    /// `lastModified`…) en own-props ordinaires, distinction
    /// par proto (`file_proto`).
    Blob {
        /// Contenu (ne mute jamais — les `Blob` sont immuables).
        data: Rc<Vec<u8>>,
        /// Type MIME (minuscule — `options.type` normalisé).
        ty: String,
    },
    /// `DataView` — byte window over a `ByteBuf` object.
    DataView {
        /// Backing `ObjKind::ByteBuf` object.
        buffer: Value,
        /// Byte offset within the buffer.
        offset: usize,
        /// Viewed byte length.
        len: usize,
    },
    /// `%TypedArray%` view (`Uint8Array`, `Float64Array`…) over a
    /// `ByteBuf` — indexed reads/writes convert through [`TaKind`].
    TypedArray {
        /// Element kind (width + read/write semantics).
        ta: TaKind,
        /// Backing `ObjKind::ByteBuf` object.
        buffer: Value,
        /// Byte offset within the buffer.
        offset: usize,
        /// Element count.
        len: usize,
    },
    /// `bigint` primitive cell — digits only (immutables : chaque
    /// op arithmétique alloue une nouvelle cellule). Référencée par
    /// `Value::Big`, jamais par `Value::Obj` — pas de proto, pas de
    /// props (les accesseurs vivent sur `BigInt.prototype`).
    BigInt(num_bigint::BigInt),
    /// `DisposableStack` (ES2026) — pile LIFO de disposeurs
    /// `(fn, this, arg)` : `use` pousse `(v[@@dispose], v,
    /// undefined)` — méthode appelée sur la ressource —, `adopt`
    /// `(f, undefined, v)`, `defer` `(f, undefined, undefined)`.
    /// `dispose()` vide en ordre inverse, première erreur
    /// propagée, les suivantes enroulées en `SuppressedError`.
    DispStack {
        /// `true` après `dispose()`/`move()` — réutilisation →
        /// `ReferenceError` (spec).
        disposed: bool,
        /// Disposeurs en attente, LIFO : `(fn, this, arg)`.
        stack: Vec<(Value, Value, Value)>,
    },
}

/// Element kind of a [`ObjKind::TypedArray`] view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaKind {
    /// `Int8Array`.
    I8,
    /// `Uint8Array`.
    U8,
    /// `Uint8ClampedArray` — saturating writes.
    U8C,
    /// `Int16Array` (little-endian).
    I16,
    /// `Uint16Array`.
    U16,
    /// `Int32Array`.
    I32,
    /// `Uint32Array`.
    U32,
    /// `Float32Array`.
    F32,
    /// `Float64Array`.
    F64,
    /// `BigInt64Array` — éléments `BigInt` (JS-C7).
    I64,
    /// `BigUint64Array`.
    U64,
    /// `Float16Array` — éléments IEEE-754 binary16 (ES2025).
    F16,
}

impl TaKind {
    /// Byte width of one element.
    pub fn size(self) -> usize {
        match self {
            TaKind::I8 | TaKind::U8 | TaKind::U8C => 1,
            TaKind::I16 | TaKind::U16 | TaKind::F16 => 2,
            TaKind::I32 | TaKind::U32 | TaKind::F32 => 4,
            TaKind::F64 | TaKind::I64 | TaKind::U64 => 8,
        }
    }

    /// Constructor/global name (`"Uint8Array"`, …).
    pub fn name(self) -> &'static str {
        match self {
            TaKind::I8 => "Int8Array",
            TaKind::U8 => "Uint8Array",
            TaKind::U8C => "Uint8ClampedArray",
            TaKind::I16 => "Int16Array",
            TaKind::U16 => "Uint16Array",
            TaKind::I32 => "Int32Array",
            TaKind::U32 => "Uint32Array",
            TaKind::F32 => "Float32Array",
            TaKind::F64 => "Float64Array",
            TaKind::I64 => "BigInt64Array",
            TaKind::U64 => "BigUint64Array",
            TaKind::F16 => "Float16Array",
        }
    }

    /// All kinds in spec order (for the `install` loop).
    pub const ALL: [TaKind; 12] = [
        TaKind::I8,
        TaKind::U8,
        TaKind::U8C,
        TaKind::I16,
        TaKind::U16,
        TaKind::I32,
        TaKind::U32,
        TaKind::F32,
        TaKind::F64,
        TaKind::I64,
        TaKind::U64,
        TaKind::F16,
    ];

    /// Index used as the ctor bound-arg (`ta_ctor` dispatch).
    pub fn index(self) -> usize {
        match self {
            TaKind::I8 => 0,
            TaKind::U8 => 1,
            TaKind::U8C => 2,
            TaKind::I16 => 3,
            TaKind::U16 => 4,
            TaKind::I32 => 5,
            TaKind::U32 => 6,
            TaKind::F32 => 7,
            TaKind::F64 => 8,
            TaKind::I64 => 9,
            TaKind::U64 => 10,
            TaKind::F16 => 11,
        }
    }

    /// `true` pour les kinds dont les éléments sont des `BigInt`
    /// (`BigInt64Array`/`BigUint64Array`) — conversions `ToBigInt`
    /// au lieu de `ToNumber` à l'écriture.
    pub fn is_big(self) -> bool {
        matches!(self, TaKind::I64 | TaKind::U64)
    }

    /// Inverse of [`TaKind::index`].
    pub fn from_index(i: usize) -> Option<TaKind> {
        TaKind::ALL.get(i).copied()
    }

    /// Read element `i` from `bytes` (little-endian, as spec for
    /// platform-independent typed arrays).
    pub fn get(self, bytes: &[u8], i: usize) -> f64 {
        let o = i * self.size();
        let b = &bytes[o..o + self.size()];
        match self {
            TaKind::I8 => b[0] as i8 as f64,
            TaKind::U8 | TaKind::U8C => f64::from(b[0]),
            TaKind::I16 => i16::from_le_bytes([b[0], b[1]]) as f64,
            TaKind::U16 => u16::from_le_bytes([b[0], b[1]]) as f64,
            TaKind::I32 => i32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f64,
            TaKind::U32 => u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f64,
            TaKind::F32 => f32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f64,
            TaKind::F64 => f64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]),
            // Fallback numérique pour les kinds BigInt — tronque à
            // f64 ; le chemin nominal est `get_big` (précision pleine).
            TaKind::I64 => {
                i64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]) as f64
            }
            TaKind::U64 => {
                u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]) as f64
            }
            TaKind::F16 => f16_to_f64(u16::from_le_bytes([b[0], b[1]])),
        }
    }

    /// Read element `i` as `BigInt` — `I64` signe-étendu,
    /// `U64` non signé. Ne s'appelle que sur les kinds big
    /// (`is_big`).
    pub fn get_big(self, bytes: &[u8], i: usize) -> num_bigint::BigInt {
        let o = i * self.size();
        let a: [u8; 8] = bytes
            .get(o..o + 8)
            .and_then(|s| <[u8; 8]>::try_from(s).ok())
            .unwrap_or([0; 8]);
        match self {
            TaKind::U64 => num_bigint::BigInt::from(u64::from_le_bytes(a)),
            _ => num_bigint::BigInt::from(i64::from_le_bytes(a)),
        }
    }

    /// `SetValueInBuffer` pour les kinds big — `v mod 2^64`,
    /// complément à deux stocké little-endian (la lecture `I64`
    /// ré-interprète le signe).
    pub fn set_big(self, bytes: &mut [u8], i: usize, v: &num_bigint::BigInt) {
        use num_traits::ToPrimitive;
        let o = i * self.size();
        let m = num_bigint::BigInt::from(1) << 64u32;
        let mut r = v % &m;
        if r.sign() == num_bigint::Sign::Minus {
            r += &m;
        }
        let bits = r.to_u64().unwrap_or(0).to_le_bytes();
        bytes[o..o + 8].copy_from_slice(&bits);
    }

    /// `SetValueInBuffer` — convert `v` through the kind's
    /// conversion (modulo wrap for ints, saturating round for
    /// `Uint8Clamped`, IEEE narrowing for floats) and store it.
    pub fn set(self, bytes: &mut [u8], i: usize, v: f64) {
        let o = i * self.size();
        match self {
            TaKind::I8 => bytes[o] = (crate::value::to_int32(v) as i8) as u8,
            TaKind::U8 => bytes[o] = crate::value::to_uint32(v) as u8,
            TaKind::U8C => {
                bytes[o] = if v.is_nan() {
                    0
                } else {
                    v.round().clamp(0.0, 255.0) as u8
                };
            }
            TaKind::I16 => {
                bytes[o..o + 2].copy_from_slice(&(crate::value::to_int32(v) as i16).to_le_bytes());
            }
            TaKind::U16 => {
                bytes[o..o + 2].copy_from_slice(&(crate::value::to_uint32(v) as u16).to_le_bytes());
            }
            TaKind::I32 => {
                bytes[o..o + 4].copy_from_slice(&crate::value::to_int32(v).to_le_bytes());
            }
            TaKind::U32 => {
                bytes[o..o + 4].copy_from_slice(&crate::value::to_uint32(v).to_le_bytes());
            }
            TaKind::F32 => {
                bytes[o..o + 4].copy_from_slice(&(v as f32).to_le_bytes());
            }
            TaKind::F64 => {
                bytes[o..o + 8].copy_from_slice(&v.to_le_bytes());
            }
            // Fallback : écriture f64 dans un kind big (le chemin
            // nominal est `set_big`). Saturante `as` — ne se produit
            // que hors des méthodes refactorées.
            TaKind::I64 => {
                bytes[o..o + 8].copy_from_slice(&(v as i64).to_le_bytes());
            }
            TaKind::U64 => {
                bytes[o..o + 8].copy_from_slice(&(v as u64).to_le_bytes());
            }
            TaKind::F16 => {
                bytes[o..o + 2].copy_from_slice(&f64_to_f16(v).to_le_bytes());
            }
        }
    }
}

/// IEEE-754 binary16 → `f64` (`Float16Array`, `Math.f16round`).
pub(crate) fn f16_to_f64(h: u16) -> f64 {
    let sign = if h & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exp = i32::from((h >> 10) & 0x1f);
    let man = f64::from(h & 0x3ff);
    match exp {
        0 => sign * man * 2f64.powi(-24),
        31 => {
            if man == 0.0 {
                sign * f64::INFINITY
            } else {
                f64::NAN
            }
        }
        e => sign * (1.0 + man / 1024.0) * 2f64.powi(e - 15),
    }
}

/// `f64` → IEEE-754 binary16 — round-to-nearest approximé
/// (mantisse arrondie via `.round()` — correct sauf égalités
/// exactes, couverture navigateur suffisante).
pub(crate) fn f64_to_f16(v: f64) -> u16 {
    if v.is_nan() {
        return 0x7e00;
    }
    let sign = if v.is_sign_negative() { 0x8000u16 } else { 0 };
    let a = v.abs();
    if a == 0.0 {
        return sign;
    }
    if a >= 65504.0 {
        // Max fini 65504 ; jusqu'à 65520 exclu on arrondit encore
        // à 65504, au-delà → +Inf.
        return if a < 65520.0 {
            sign | 0x7bff
        } else {
            sign | 0x7c00
        };
    }
    if a < 0.000_061_035_156_25 {
        // Subnormal binary16 : man = a * 2^24 arrondi (0 si < 2^-25).
        return sign | (a * 16_777_216.0).round() as u16;
    }
    let mut exp = a.log2().floor() as i32;
    let mut man = ((a / 2f64.powi(exp) - 1.0) * 1024.0).round() as u32;
    if man == 1024 {
        man = 0;
        exp += 1;
    }
    if exp >= 15 {
        return sign | 0x7c00;
    }
    sign | ((exp + 15) as u16) << 10 | (man as u16)
}

impl Object {
    /// New ordinary object with the given prototype.
    pub fn ordinary(proto: Value) -> Self {
        Object {
            proto,
            props: Vec::new(),
            kind: ObjKind::Ordinary,
            extensible: true,
            elem_integrity: 0,
            args_map: None,
        }
    }

    /// New array.
    pub fn array(elems: Vec<Value>, proto: Value) -> Self {
        Object {
            proto,
            props: Vec::new(),
            kind: ObjKind::Array(elems),
            extensible: true,
            elem_integrity: 0,
            args_map: None,
        }
    }

    /// New callable.
    pub fn fun(f: Fun, proto: Value) -> Self {
        Object {
            proto,
            props: Vec::new(),
            kind: ObjKind::Fun(f),
            extensible: true,
            elem_integrity: 0,
            args_map: None,
        }
    }

    /// Raw own property lookup (no prototype walk, no array-index
    /// magic) — accessors come back as `Prop::Acc`.
    pub fn find(&self, key: &str) -> Option<Prop> {
        self.props.iter().find(|(k, _)| k == key).map(|(_, p)| *p)
    }

    /// Own property lookup — data value only (`None` for accessors).
    pub fn get_own(&self, key: &str) -> Option<Value> {
        self.find(key).and_then(|p| p.data_value())
    }

    /// `[[DefineOwnProperty]]` for a data property — respects
    /// `extensible`/existing attrs (simplified spec rules) :
    /// non-configurable props keep their attributes (`value` update
    /// honoured iff still `writable` — same-value case included).
    /// Returns `false` on rejection.
    pub fn define_data(&mut self, key: &str, value: Value, attrs: PropAttrs) -> bool {
        if let Some((_, p)) = self.props.iter_mut().find(|(k, _)| k == key) {
            match p {
                Prop::Data { value: v, attrs: a } => {
                    if a.configurable {
                        *v = value;
                        *a = attrs;
                        true
                    } else {
                        // Attrs may only shrink further
                        // (`false`→`false` transitions kept).
                        if attrs.enumerable != a.enumerable
                            || attrs.configurable
                            || (attrs.writable && !a.writable)
                        {
                            return false;
                        }
                        if !a.writable && *v != value {
                            return false;
                        }
                        *v = value;
                        a.writable &= attrs.writable;
                        true
                    }
                }
                Prop::Acc { attrs: a, .. } => {
                    if !a.configurable {
                        return false;
                    }
                    *p = Prop::Data { value, attrs };
                    true
                }
            }
        } else if self.extensible {
            self.props
                .push((key.to_string(), Prop::Data { value, attrs }));
            true
        } else {
            false
        }
    }

    /// `[[DefineOwnProperty]]` for an accessor — same rules. `get`
    /// or `set` may be `Undefined` (absent side).
    pub fn define_accessor(&mut self, key: &str, get: Value, set: Value, attrs: PropAttrs) -> bool {
        if let Some((_, p)) = self.props.iter_mut().find(|(k, _)| k == key) {
            match p {
                Prop::Acc {
                    get: g,
                    set: s,
                    attrs: a,
                } => {
                    if !a.configurable {
                        // Only flag-compatible attr writes allowed.
                        if attrs.enumerable != a.enumerable || attrs.configurable {
                            return false;
                        }
                        if !matches!(get, Value::Undefined) && *g != get {
                            return false;
                        }
                        if !matches!(set, Value::Undefined) && *s != set {
                            return false;
                        }
                    }
                    if !matches!(get, Value::Undefined) {
                        *g = get;
                    }
                    if !matches!(set, Value::Undefined) {
                        *s = set;
                    }
                    a.enumerable = attrs.enumerable;
                    true
                }
                Prop::Data { attrs: a, .. } => {
                    if !a.configurable {
                        return false;
                    }
                    *p = Prop::Acc { get, set, attrs };
                    true
                }
            }
        } else if self.extensible {
            self.props
                .push((key.to_string(), Prop::Acc { get, set, attrs }));
            true
        } else {
            false
        }
    }

    /// Insert or overwrite an own data property (keeps insertion
    /// order). A data write over an existing accessor replaces it.
    /// Convenience for install paths : writable+configurable,
    /// `enumerable` au choix.
    pub fn set_own(&mut self, key: &str, value: Value, enumerable: bool) {
        let attrs = if enumerable {
            PropAttrs::FULL
        } else {
            PropAttrs::BUILTIN
        };
        // Force-install : utilisé par les builtins avant toute
        // protection — on outrepasse configurable.
        if let Some((_, p)) = self.props.iter_mut().find(|(k, _)| k == key) {
            *p = Prop::Data { value, attrs };
            return;
        }
        self.props
            .push((key.to_string(), Prop::Data { value, attrs }));
    }

    /// Merge a getter/setter side into an own property (existing
    /// accessor sides are preserved, à la `Object.defineProperty`).
    /// Install path — attributes forced configurable.
    pub fn set_accessor(&mut self, key: &str, get: Value, set: Value, enumerable: bool) {
        let attrs = PropAttrs {
            writable: false,
            enumerable,
            configurable: true,
        };
        if let Some((_, p)) = self.props.iter_mut().find(|(k, _)| k == key) {
            match p {
                Prop::Acc { get: g, set: s, .. } => {
                    if !matches!(get, Value::Undefined) {
                        *g = get;
                    }
                    if !matches!(set, Value::Undefined) {
                        *s = set;
                    }
                    return;
                }
                Prop::Data { .. } => {
                    *p = Prop::Acc { get, set, attrs };
                    return;
                }
            }
        }
        self.props
            .push((key.to_string(), Prop::Acc { get, set, attrs }));
    }

    /// Remove an own property if `[[Configurable]]`. Returns `false`
    /// when the prop exists and is locked (the caller decides
    /// sloppy-silent vs strict-`TypeError`).
    pub fn remove_own(&mut self, key: &str) -> bool {
        match self.props.iter().position(|(k, _)| k == key) {
            Some(i) if !self.props[i].1.configurable() => false,
            Some(i) => {
                self.props.remove(i);
                true
            }
            None => true,
        }
    }
}

/// Host-side object contract — the embedding side (mini-DOM now,
/// real doctree bridge later) implements this; the engine delegates.
///
/// All hooks take `&mut Engine` because a getter may run back into
/// the engine (allocate values, call stored listeners).
///
/// Contract: `get`/`set`/`invoke` return `Ok(None)`/`Ok(false)` to
/// mean « not handled » — the engine then falls back to ordinary own
/// props / prototype / `call_value`. That keeps script-set properties
/// (`el.myFlag = 1`) working on host objects.
pub trait Host: std::any::Any {
    /// `host[key]`.
    ///
    /// # Errors
    /// Propagates script exceptions thrown while computing the
    /// property (coercion, listener calls…).
    fn get(&self, _e: &mut crate::Engine, _this: Value, _key: &str) -> JsResult<Option<Value>> {
        Ok(None)
    }

    /// `host[key] = v`. `false` → stored as ordinary own property.
    ///
    /// # Errors
    /// Propagates script exceptions thrown while coercing `v`.
    fn set(&self, _e: &mut crate::Engine, _this: Value, _key: &str, _v: Value) -> JsResult<bool> {
        Ok(false)
    }

    /// `host.m(args)` — method dispatch. `None` → property fallback.
    ///
    /// # Errors
    /// Propagates script exceptions thrown by the method body.
    fn invoke(
        &self,
        _e: &mut crate::Engine,
        _this: Value,
        _name: &str,
        _args: &[Value],
    ) -> JsResult<Option<Value>> {
        Ok(None)
    }

    /// `true` si [`Self::invoke`] dispatche `name` — lisible
    /// comme propriété fonctionnelle (feature-detect :
    /// `typeof el.m === 'function'`, shims `o = el.matches || …`).
    /// Défaut : aucune — seuls les hosts qui déclarent leur
    /// surface exposent le repli (le check peut dépendre du
    /// receveur : `<dialog>` expose `showModal`, pas `<div>`).
    fn has_method(&self, _name: &str) -> bool {
        false
    }

    /// Enumerable host keys (`for-in`); own enumerable props are
    /// appended by the engine.
    fn keys(&self, _e: &mut crate::Engine, _this: Value) -> Vec<String> {
        Vec::new()
    }

    /// `delete host[key]` — `Some(ok)` = l'hôte a tranché
    /// (valeur renvoyée par `[[Delete]]`), `None` = repli par
    /// défaut (propriété propre JS — les hôtes « vue live »
    /// comme `dataset` doivent traduire la suppression vers
    /// leur support : sans ce crochet `delete el.dataset.x`
    /// laissait l'attribut `data-x` intact — spectrum gardait
    /// `data-status` → `div[data-status]{display:none}`
    /// éternel).
    ///
    /// # Errors
    /// Propage les exceptions levées par l'hôte.
    fn del(&self, _e: &mut crate::Engine, _this: Value, _key: &str) -> JsResult<Option<bool>> {
        Ok(None)
    }

    /// `[object X]` tag for `Object.prototype.toString`.
    fn class_name(&self) -> &'static str {
        "HostObject"
    }

    /// Snapshot des valeurs itérées par `obj[Symbol.iterator]()`
    /// (`for-of`, spread, helpers closure `makeIterator` de la
    /// cible — JS-C76) : `Some(items)` expose `@@iterator`,
    /// `None` (défaut) = non itérable. Posée en own-prop par
    /// [`crate::Engine::new_iterable_host`] — la collection est
    /// **relue à chaque appel** (vivante comme les collections
    /// DOM), le snapshot gelé dans l'iterateur résultat.
    ///
    /// # Errors
    /// Propage les exceptions levées pendant la collecte.
    fn iter_items(&self, _e: &mut crate::Engine) -> JsResult<Option<Vec<Value>>> {
        Ok(None)
    }

    /// Trace hook for the mark-sweep GC (ADR-0008) : tout `Value`
    /// retenu par l'objet hôte doit être poussé dans `out`, sinon
    /// le collecteur le réclamera sous le pied du script. Défaut :
    /// aucun (la plupart des hosts retiennent des handles hôtes,
    /// pas des `Value`).
    fn trace(&self, _out: &mut Vec<ObjId>) {}

    /// Appelé juste avant libération de la cellule par le
    /// mark-sweep — point de finalisation pour l'hôte (relâcher des
    /// handles côté `explorer-dom`, fermer des ressources…).
    /// Défaut : no-op.
    fn finalize(&self) {}

    /// Downcast support for host-side operations (`appendChild`
    /// needs the *node* behind an element value). Implement as
    /// `fn as_any(&self) -> &dyn Any { self }` — the coercion needs a
    /// concrete `Self`, so no default body is possible.
    fn as_any(&self) -> &dyn std::any::Any;
}

/// Callable payload.
#[derive(Clone)]
pub enum Fun {
    /// Script-defined function: literal + defining scope (closure).
    Script {
        /// The parsed function (name, params, body).
        lit: Rc<FnLit>,
        /// Captured environment.
        env: EnvRef,
        /// `Some` for arrow functions — lexical `this` captured at
        /// creation time, overriding the caller's receiver.
        lex_this: Option<Value>,
    },
    /// Rust-implemented builtin.
    Native {
        /// Debug name.
        name: &'static str,
        /// The function pointer.
        f: NativeFn,
    },
    /// Builtin with prepended bound args (promise resolve/reject,
    /// `Promise.all` slots…): call args become `bound ++ args`.
    BoundNative {
        /// Debug name.
        name: &'static str,
        /// The function pointer.
        f: NativeFn,
        /// Args prepended to every call.
        bound: Vec<Value>,
    },
    /// `Function.prototype.bind` product — receiver + args are
    /// captured; the call forwards to `target`.
    Bound {
        /// Callable to forward to.
        target: Value,
        /// Captured `this`.
        this: Value,
        /// Args prepended to every call.
        bound: Vec<Value>,
    },
}

/// Lexical environment. The global environment delegates its bindings
/// to the global object's properties (so `foo` and `window.foo` are
/// the same slot); local scopes are plain hash maps.
///
/// `Rc<RefCell>` cycles (function ↔ its env) leak within a page —
/// acceptable: the whole heap dies with the `Engine` on navigation.
pub struct Env {
    /// Storage flavour.
    pub kind: EnvKind,
    /// Enclosing scope.
    pub parent: Option<EnvRef>,
}

/// Storage flavour of an [`Env`].
pub enum EnvKind {
    /// Global scope — `var`/implicit bindings live on the global
    /// object; the map is the declarative global record holding
    /// `let`/`const` names (they do *not* appear on `window`).
    Global(HashMap<String, EnvCell>),
    /// Function scope (paramètres, corps de fonction/programme).
    Local(HashMap<String, EnvCell>),
    /// Portée de bloc (`{ … }`, tête de `for`, `switch`, `catch`) —
    /// même stockage que `Local`, mais `var`/déclarations de
    /// fonction la TRAVERSENT jusqu'au scope fonction englobant
    /// (spec `VarDeclaredNames`) ; `let`/`const`/`class` y vivent.
    Block(HashMap<String, EnvCell>),
    /// `with (obj)` — object-as-scope : la résolution d'un nom
    /// consulte les props de l'objet (sauf `@@unscopables`)
    /// puis remonte au parent.
    With(ObjId),
}

/// A binding cell: a normal [`Binding`], or a module import alias
/// forwarding to a name in the exporting module's env — *live
/// binding* (spec : l'importeur voit les mutations ultérieures de
/// l'exporteur, et ne peut pas écrire lui-même).
#[derive(Clone)]
pub enum EnvCell {
    /// Ordinary binding.
    Bind(Binding),
    /// `import {x as y}` — forwards every read/write attempt to
    /// `name` in `env` (the exporter's module scope).
    Alias(EnvRef, String),
}

/// A local-scope binding. `var`/`let` are mutable; `const` writes
/// after initialisation throw `TypeError` (ES2015+, strict comme
/// sloppy) — sauf `sloppy_silent` (nom de `FunctionExpression`,
/// `ReadOnly` ES3 §13 : échec silencieux en sloppy, `TypeError` en
/// strict).
#[derive(Debug, Clone, Copy)]
pub struct Binding {
    /// Stored value.
    pub value: Value,
    /// `false` for `const`.
    pub mutable: bool,
    /// Écriture sloppy silencieuse (nom de `FunctionExpression`).
    pub sloppy_silent: bool,
}

/// Shared env handle.
pub type EnvRef = Rc<RefCell<Env>>;

impl Env {
    /// New local scope chained to `parent`.
    pub fn local(parent: EnvRef) -> EnvRef {
        Rc::new(RefCell::new(Env {
            kind: EnvKind::Local(HashMap::new()),
            parent: Some(parent),
        }))
    }

    /// The global scope.
    pub fn global() -> EnvRef {
        Rc::new(RefCell::new(Env {
            kind: EnvKind::Global(HashMap::new()),
            parent: None,
        }))
    }

    /// New block scope chained to `parent` — `let`/`const` y vivent,
    /// `var` la traverse (voir [`EnvKind::Block`]).
    pub fn block(parent: EnvRef) -> EnvRef {
        Rc::new(RefCell::new(Env {
            kind: EnvKind::Block(HashMap::new()),
            parent: Some(parent),
        }))
    }

    /// `with (obj)` — object-as-scope chained to `parent`.
    pub fn with(parent: EnvRef, obj: ObjId) -> EnvRef {
        Rc::new(RefCell::new(Env {
            kind: EnvKind::With(obj),
            parent: Some(parent),
        }))
    }
}

/// `true` si la chaîne proto de `sid` porte au moins un
/// accessor (`Prop::Acc`) — sonde diag : un `{...}`/
/// `Object.assign` ne copie que les own enumerable, les
/// getters proto sont perdus (spec — mur terrain `TanStack`
/// `e.id` → `undefined`).
pub(crate) fn proto_has_accessor(heap: &Heap, sid: ObjId) -> bool {
    let mut cur = heap.obj(sid).map_or(Value::Null, |o| o.proto);
    for _ in 0..32 {
        let Value::Obj(id) = cur else { return false };
        let Some(o) = heap.obj(id) else { return false };
        if o.props.iter().any(|(_, p)| matches!(p, Prop::Acc { .. })) {
            return true;
        }
        cur = o.proto;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    /// La borne heap est **armée par défaut** (ADR-0019) — le
    /// constat post-crash : `mem_limit = 0` était le défaut et
    /// `set_mem_limit` n'était appelé nulle part, donc un script
    /// assez long vidait la machine (mesuré : challenge google,
    /// budget d'ops relevé → OOM station de travail).
    #[test]
    fn mem_limit_armed_by_default() {
        let h = Heap::new();
        assert_eq!(h.mem_limit(), DEFAULT_MEM_LIMIT);
        assert!(!h.over_limit());
    }

    /// `set_mem_limit(0)` reste le désarmement explicite — les
    /// harnais/tests qui doivent dépasser le défaut le demandent.
    #[test]
    fn mem_limit_zero_disarms() {
        let mut h = Heap::new();
        h.set_mem_limit(0);
        assert_eq!(h.mem_limit(), 0);
        h.set_mem_limit(64);
        assert_eq!(h.mem_limit(), 64);
    }
}
