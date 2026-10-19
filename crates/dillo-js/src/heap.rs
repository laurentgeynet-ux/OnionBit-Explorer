// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Heap, objects, environments — the arena at the heart of `dillo-js`.
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
        }
    }

    fn alloc(&mut self, data: CellData) -> ObjId {
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
            ObjKind::Set(items) | ObjKind::Iter { items, .. } => {
                for &v in items {
                    push_value(out, v);
                }
            }
            ObjKind::Proxy { target, handler } => {
                push_value(out, *target);
                if let Some(h) = handler {
                    push_value(out, *h);
                }
            }
            ObjKind::Ordinary
            | ObjKind::RegExp(_)
            | ObjKind::Symbol(_)
            | ObjKind::FetchResponse { .. } => {}
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
            }
            self.marks[i] = false;
        }
        swept
    }
}

/// Pousse l'`ObjId` d'un `Value` (no-op pour les scalaires).
pub(crate) fn push_value(out: &mut Vec<ObjId>, v: Value) {
    match v {
        Value::Obj(i) | Value::Str(i) => out.push(i),
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
            let map = match &borrowed.kind {
                EnvKind::Global(m) | EnvKind::Local(m) => m,
            };
            for c in map.values() {
                match c {
                    EnvCell::Bind(b) => push_value(out, b.value),
                    // Alias d'import : l'env du module exporteur est
                    // aussi une racine — ses liaisons restent
                    // vivantes tant qu'un importeur les référence.
                    EnvCell::Alias(target, _) => stack.push(target.clone()),
                }
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
}

/// What kind of object this is.
pub enum ObjKind {
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
    },
    /// `Symbol(desc)` — unique identity per alloc (`===` by `ObjId`,
    /// `typeof` → `"symbol"`). Well-known symbols stay string
    /// sentinels (`SYM_ITERATOR` — prop keys are strings).
    Symbol(Option<String>),
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
    /// `fetch` Response — body + headers delivered by the embedder's
    /// transport (`status`/`statusText`/`ok` are plain own props);
    /// `text()`/`json()` read `body` as resolved promises.
    FetchResponse {
        /// Response headers (lower-cased lookup order).
        headers: Vec<(String, String)>,
        /// Body text.
        body: String,
    },
}

impl Object {
    /// New ordinary object with the given prototype.
    pub fn ordinary(proto: Value) -> Self {
        Object {
            proto,
            props: Vec::new(),
            kind: ObjKind::Ordinary,
            extensible: true,
        }
    }

    /// New array.
    pub fn array(elems: Vec<Value>, proto: Value) -> Self {
        Object {
            proto,
            props: Vec::new(),
            kind: ObjKind::Array(elems),
            extensible: true,
        }
    }

    /// New callable.
    pub fn fun(f: Fun, proto: Value) -> Self {
        Object {
            proto,
            props: Vec::new(),
            kind: ObjKind::Fun(f),
            extensible: true,
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

    /// Enumerable host keys (`for-in`); own enumerable props are
    /// appended by the engine.
    fn keys(&self, _e: &mut crate::Engine, _this: Value) -> Vec<String> {
        Vec::new()
    }

    /// `[object X]` tag for `Object.prototype.toString`.
    fn class_name(&self) -> &'static str {
        "HostObject"
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
    /// Function/catch scope.
    Local(HashMap<String, EnvCell>),
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
/// after initialisation are silently ignored (documented sloppy
/// tolerance — strict engines throw `TypeError`).
#[derive(Debug, Clone, Copy)]
pub struct Binding {
    /// Stored value.
    pub value: Value,
    /// `false` for `const`.
    pub mutable: bool,
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
}
