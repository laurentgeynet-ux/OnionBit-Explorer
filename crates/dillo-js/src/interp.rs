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
    AssignOp, BinOp, ChainStep, ClassLit, ClassMemberKind, Expr, FnLit, ForInit, ForTarget,
    ObjProp, Param, Pat, Program, PropKey, Stmt, UnOp,
};
use crate::error::{Error, Result};
use crate::heap::{
    Binding, Env, EnvKind, EnvRef, Fun, Heap, JsResult, ObjKind, Object, Prop, Thrown,
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

/// `Symbol.iterator` property key — a private-use sentinel string
/// (symbols are represented as opaque string keys; `typeof sym`
/// yields `"string"`, a documented deviation).
pub(crate) const SYM_ITERATOR: &str = "\u{f130}Symbol.iterator";

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
}

impl Protos {
    /// Tous les prototypes — racines GC permanentes.
    fn all(&self) -> [ObjId; 17] {
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
        ]
    }
}

/// Statement completion — `throw` travels via `Err(Thrown)`.
enum Flow {
    Normal,
    Break,
    Continue,
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
        let array = heap.alloc_obj(Object::ordinary(Value::Obj(object)));
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
            },
            rng: seed | 1,
            t0: Instant::now(),
            symbol_reg: std::collections::BTreeMap::new(),
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
        };
        crate::builtins::install(&mut e);
        crate::collections::install(&mut e);
        crate::json::install(&mut e);
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
        crate::heap::push_value(&mut ids, self.completion);
        // 2. Env global — les bindings `let`/`const` ne vivent pas
        //    sur l'objet global.
        let mut visited = HashSet::new();
        crate::heap::trace_env(&self.global_env, &mut ids, &mut visited);
        // 3. Mark-sweep.
        self.heap.collect(|mark| {
            for &id in &ids {
                mark(id);
            }
        })
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

    /// New `Error`-family object value (`{name, message}` on `proto`).
    pub(crate) fn error_value(&mut self, proto: ObjId, name: &str, msg: &str) -> Value {
        let id = self.heap.alloc_obj(Object::ordinary(Value::Obj(proto)));
        let n = self.str_val(name);
        let m = self.str_val(msg);
        self.set_own(id, "name", n, false);
        self.set_own(id, "message", m, false);
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
            frozen: false,
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
        self.set_own(id, "lastIndex", Value::Num(0.0), true);
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

    fn env_get(&self, env: &EnvRef, name: &str) -> Option<Value> {
        let mut cur = Some(env.clone());
        while let Some(e) = cur {
            let next = {
                let b = e.borrow();
                match &b.kind {
                    EnvKind::Local(m) => {
                        if let Some(v) = m.get(name) {
                            return Some(v.value);
                        }
                        b.parent.clone()
                    }
                    EnvKind::Global(m) => {
                        // Declarative record first, then `window` props.
                        if let Some(v) = m.get(name) {
                            return Some(v.value);
                        }
                        if let Some(v) = self.get_prop(self.global, name) {
                            return Some(v);
                        }
                        b.parent.clone()
                    }
                }
            };
            cur = next;
        }
        None
    }

    fn env_set(&mut self, env: &EnvRef, name: &str, v: Value) {
        let mut cur = Some(env.clone());
        while let Some(e) = cur {
            enum Act {
                Local(EnvRef),
                Global,
                Up(Option<EnvRef>),
            }
            let act = {
                let b = e.borrow();
                match &b.kind {
                    EnvKind::Local(m) if m.contains_key(name) => Act::Local(e.clone()),
                    EnvKind::Local(_) => Act::Up(b.parent.clone()),
                    EnvKind::Global(m) if m.contains_key(name) => Act::Local(e.clone()),
                    EnvKind::Global(_) => Act::Global,
                }
            };
            match act {
                Act::Local(e) => {
                    let mut b = e.borrow_mut();
                    let m = match &mut b.kind {
                        EnvKind::Local(m) | EnvKind::Global(m) => m,
                    };
                    if let Some(bnd) = m.get_mut(name) {
                        if bnd.mutable {
                            bnd.value = v;
                        }
                    }
                    return;
                }
                Act::Global => {
                    self.set_own(self.global, name, v, true);
                    return;
                }
                Act::Up(p) => cur = p,
            }
        }
        // Implicit global (sloppy mode).
        self.set_own(self.global, name, v, true);
    }

    fn env_declare(&mut self, env: &EnvRef, name: &str) {
        let mut b = env.borrow_mut();
        match &mut b.kind {
            EnvKind::Local(m) => {
                m.entry(name.to_string()).or_insert(Binding {
                    value: Value::Undefined,
                    mutable: true,
                });
            }
            EnvKind::Global(_) => {
                // `var` lives on the global object, not the record.
                let g = self.global;
                drop(b);
                if self.heap.obj(g).is_some_and(|o| o.get_own(name).is_none()) {
                    self.set_own(g, name, Value::Undefined, true);
                }
            }
        }
    }

    /// Insert (or overwrite) a binding in `env`'s own map — used by
    /// the class scope for the hidden `super`/`home` names.
    fn env_put(env: &EnvRef, name: &str, v: Value, mutable: bool) {
        let mut b = env.borrow_mut();
        match &mut b.kind {
            EnvKind::Local(m) | EnvKind::Global(m) => {
                m.insert(name.to_string(), Binding { value: v, mutable });
            }
        }
    }

    /// `let`/`const`: declare in the *current* scope only, no hoisting
    /// (block scoping is approximated — a `let` inside `{}` lives in
    /// the function scope and there is no TDZ: an uninitialised read
    /// yields `undefined` rather than throwing).
    ///
    /// Global `let`/`const` go to the declarative record, *not* the
    /// global object — `window.x` stays `undefined` for `let x`.
    fn env_declare_lex(env: &EnvRef, name: &str, mutable: bool) {
        let mut b = env.borrow_mut();
        match &mut b.kind {
            EnvKind::Local(m) | EnvKind::Global(m) => {
                m.insert(
                    name.to_string(),
                    Binding {
                        value: Value::Undefined,
                        mutable,
                    },
                );
            }
        }
    }

    /// Initialize a binding just created by `env_declare_lex` —
    /// bypasses the `const` guard (initialisation is not mutation).
    fn env_init_lex(env: &EnvRef, name: &str, v: Value) {
        let mut b = env.borrow_mut();
        match &mut b.kind {
            EnvKind::Local(m) | EnvKind::Global(m) => {
                if let Some(bnd) = m.get_mut(name) {
                    bnd.value = v;
                }
            }
        }
    }

    // -- patterns -----------------------------------------------------------

    /// `var`-declare every `Ident` inside `pat` (hoisting support —
    /// `var [a, {b}] = …` declares `a` and `b`).
    fn declare_pat_names(&mut self, env: &EnvRef, pat: &Pat) {
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
                        self.env_set(env, name, v);
                    }
                    BindMode::Lex { mutable } => {
                        // `declare_pat_lex` may have pre-declared the
                        // name (decl sites); catch/for-of/params bind
                        // directly here. Init bypasses the const
                        // guard (it's not a mutation).
                        Self::env_declare_lex(env, name, mutable);
                        Self::env_init_lex(env, name, v);
                    }
                    BindMode::Assign => self.env_set(env, name, v),
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
                        for k in self.own_enum_keys(id) {
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
                if let Some(o) = self.heap.obj(id) {
                    if let ObjKind::Array(elems) = &o.kind {
                        return Ok(elems.clone());
                    }
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
        for m in &cl.members {
            if matches!(m.kind, ClassMemberKind::Ctor) {
                continue;
            }
            let key = self.eval_prop_key(env, this, &m.key)?;
            let target = if m.static_ { ctor_id } else { proto };
            let menv = Env::local(cenv.clone());
            Self::env_put(&menv, HOME_BIND, Value::Obj(target), false);
            let fv = self.make_function(&m.fun, &menv, this);
            match m.kind {
                ClassMemberKind::Method => self.set_own(target, &key, fv, false),
                ClassMemberKind::Getter => {
                    if let Some(o) = self.heap.obj_mut(target) {
                        o.set_accessor(&key, fv, Value::Undefined, false);
                    }
                }
                ClassMemberKind::Setter => {
                    if let Some(o) = self.heap.obj_mut(target) {
                        o.set_accessor(&key, Value::Undefined, fv, false);
                    }
                }
                ClassMemberKind::Ctor => {}
            }
        }
        Ok(ctor)
    }

    // -- property access ------------------------------------------------

    /// Own-property lookup with array/`length` semantics — returns
    /// the raw `Prop` so accessor pairs are visible to the caller.
    pub(crate) fn find_own(&self, id: ObjId, key: &str) -> Option<Prop> {
        let o = self.heap.obj(id)?;
        if let ObjKind::Array(elems) = &o.kind {
            if key == "length" {
                return Some(Prop::Data {
                    value: Value::Num(elems.len() as f64),
                    enumerable: false,
                });
            }
            if let Some(i) = array_index(key) {
                return elems.get(i).copied().map(|value| Prop::Data {
                    value,
                    enumerable: true,
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
                    enumerable: false,
                });
            }
        }
        o.find(key)
    }

    /// First `Prop` found on `id`'s prototype chain.
    fn find_prop(&self, id: ObjId, key: &str) -> Option<Prop> {
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
    pub(crate) fn get_prop(&self, id: ObjId, key: &str) -> Option<Value> {
        self.find_prop(id, key).and_then(|p| p.data_value())
    }

    /// Property read with accessor invocation — `o.x` where `x` has a
    /// getter calls it with `this = recv` (differs from `base` only
    /// for `super.x` reads).
    fn read_prop(&mut self, base: Value, recv: Value, key: &str) -> JsResult<Value> {
        let Value::Obj(id) = base else {
            return self.get_prop_value(base, key);
        };
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
        // Accessors on the chain intercept the write.
        let mut cur = Some(id);
        let mut depth = 0;
        while let Some(cid) = cur {
            if let Some(Prop::Acc { set, .. }) = self.find_own(cid, key) {
                if self.is_callable(set) {
                    self.call_value(set, recv, &[v])?;
                }
                return Ok(());
            }
            if self.find_own(cid, key).is_some() {
                break; // data prop on the chain → shadow with own write
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
        self.set_prop(id, key, v);
        Ok(())
    }

    /// Extract the host delegate of an object, if any.
    fn host_of(&self, id: ObjId) -> Option<Rc<dyn crate::heap::Host>> {
        match self.heap.obj(id).map(|o| &o.kind) {
            Some(ObjKind::Host(h)) => Some(h.clone()),
            _ => None,
        }
    }

    /// Allocate a host object (used by `install_dom`, and by embedders).
    pub fn new_host_object(&mut self, host: Rc<dyn crate::heap::Host>) -> Value {
        let proto = Value::Obj(self.protos.object);
        Value::Obj(self.heap.alloc_obj(Object {
            proto,
            props: Vec::new(),
            kind: ObjKind::Host(host),
            frozen: false,
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
                    let n = self.heap.string(id).map_or(0, |s| s.chars().count());
                    return Ok(Value::Num(n as f64));
                }
                if let Some(i) = array_index(key) {
                    let c = self
                        .heap
                        .string(id)
                        .and_then(|s| s.chars().nth(i))
                        .map(|c| c.to_string());
                    return Ok(match c {
                        Some(c) => self.str_val(c),
                        None => Value::Undefined,
                    });
                }
                let proto = self.protos.string;
                Ok(self.get_prop(proto, key).unwrap_or(Value::Undefined))
            }
            Value::Num(_) => {
                let proto = self.protos.number;
                Ok(self.get_prop(proto, key).unwrap_or(Value::Undefined))
            }
            Value::Bool(_) => {
                let proto = self.protos.boolean;
                Ok(self.get_prop(proto, key).unwrap_or(Value::Undefined))
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
            // Sloppy mode: writes to primitives are silently ignored.
            _ => Ok(()),
        }
    }

    fn set_prop(&mut self, id: ObjId, key: &str, v: Value) {
        if let Some(o) = self.heap.obj_mut(id) {
            if let ObjKind::Array(elems) = &mut o.kind {
                if key == "length" {
                    let n = match v {
                        Value::Num(n) if n >= 0.0 => to_uint32(n) as usize,
                        _ => elems.len(),
                    };
                    elems.resize(n, Value::Undefined);
                    return;
                }
                if let Some(i) = array_index(key) {
                    if i >= elems.len() {
                        elems.resize(i + 1, Value::Undefined);
                    }
                    elems[i] = v;
                    return;
                }
            }
            o.set_own(key, v, true);
        }
    }

    pub(crate) fn has_prop(&mut self, id: ObjId, key: &str) -> JsResult<bool> {
        if let Some(h) = self.host_of(id) {
            if h.get(self, Value::Obj(id), key)?.is_some() {
                return Ok(true);
            }
        }
        Ok(self.find_prop(id, key).is_some())
    }

    fn delete_prop(&mut self, id: ObjId, key: &str) -> bool {
        if let Some(o) = self.heap.obj_mut(id) {
            if let ObjKind::Array(elems) = &mut o.kind {
                if key == "length" {
                    return false;
                }
                if let Some(i) = array_index(key) {
                    if i < elems.len() {
                        elems[i] = Value::Undefined;
                    }
                    return true;
                }
            }
            o.remove_own(key);
            return true;
        }
        false
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
            if let ObjKind::Array(elems) = &o.kind {
                for i in 0..elems.len() {
                    out.push(i.to_string());
                }
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
            if let ObjKind::Array(elems) = &o.kind {
                for i in 0..elems.len() {
                    out.push(i.to_string());
                }
            }
            for (k, p) in &o.props {
                if p.enumerable() {
                    out.push(k.clone());
                }
            }
        }
        out
    }

    /// Enumerable keys for `for-in`: host keys, array indices, then
    /// enumerable own props, then enumerable proto-chain props
    /// (deduplicated).
    fn enum_keys(&mut self, id: ObjId) -> Vec<String> {
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
            if let Some(o) = self.heap.obj(cid) {
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
        out
    }

    // -- coercions -------------------------------------------------------

    /// `ToBoolean` (pure, no user code runs).
    pub(crate) fn to_boolean(&self, v: Value) -> bool {
        match v {
            Value::Undefined | Value::Null => false,
            Value::Bool(b) => b,
            Value::Num(n) => n != 0.0 && !n.is_nan(),
            Value::Str(id) => self.heap.string(id).is_some_and(|s| !s.is_empty()),
            Value::Obj(_) => true,
        }
    }

    /// `ToPrimitive` — may run `valueOf`/`toString`.
    fn to_primitive(&mut self, v: Value, prefer_str: bool) -> JsResult<Value> {
        let Value::Obj(id) = v else {
            return Ok(v);
        };
        let order: [&str; 2] = if prefer_str {
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

    pub(crate) fn is_callable(&self, v: Value) -> bool {
        matches!(v, Value::Obj(id)
            if matches!(self.heap.obj(id).map(|o| &o.kind), Some(ObjKind::Fun(_))))
    }

    /// `ToNumber`.
    pub(crate) fn to_number(&mut self, v: Value) -> JsResult<f64> {
        Ok(match v {
            Value::Undefined => f64::NAN,
            Value::Null => 0.0,
            Value::Bool(b) => f64::from(u8::from(b)),
            Value::Num(n) => n,
            Value::Str(id) => parse_js_number(&self.heap.string(id).unwrap_or_default()),
            Value::Obj(id) => {
                // `ToNumber(Symbol)` throws — spec TypeError.
                if matches!(self.heap.obj(id).map(|o| &o.kind), Some(ObjKind::Symbol(_))) {
                    return Err(self.type_error("cannot convert a Symbol to number"));
                }
                let p = self.to_primitive(v, false)?;
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
            Value::Obj(_) => {
                let p = self.to_primitive(v, true)?;
                self.to_js_string(p)?
            }
        })
    }

    pub(crate) fn prop_key(&mut self, v: Value) -> JsResult<String> {
        match v {
            Value::Num(n) => Ok(num_to_js_string(n)),
            _ => self.to_js_string(v),
        }
    }

    // -- calls & construction --------------------------------------------

    /// `f.apply(this, args)` with full callable semantics.
    pub(crate) fn call_value(&mut self, f: Value, this: Value, args: &[Value]) -> JsResult<Value> {
        let Value::Obj(id) = f else {
            return Err(self.type_error("value is not a function"));
        };
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
                // `this` was captured at creation time.
                let this = lex_this.unwrap_or(this);
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
                let argv = (!lit.is_arrow).then(|| self.new_array(args.to_vec()));
                {
                    let mut b = local.borrow_mut();
                    if let (EnvKind::Local(m), Some(argv)) = (&mut b.kind, argv) {
                        m.insert(
                            "arguments".into(),
                            Binding {
                                value: argv,
                                mutable: true,
                            },
                        );
                    }
                }
                self.hoist(&lit.body, &local);
                let r = self.exec_body(&local, this, &lit.body);
                self.call_depth -= 1;
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

    /// `new F(args)`.
    pub(crate) fn construct(&mut self, f: Value, args: &[Value]) -> JsResult<Value> {
        if !self.is_callable(f) {
            return Err(self.type_error("constructor is not a function"));
        }
        let Value::Obj(fid) = f else {
            return Err(self.type_error("constructor is not a function"));
        };
        // `new boundFn(...)` delegates to the target with the bound
        // args — the *bound this is ignored* under construction
        // (spec [[Construct]] of BoundFunctionExoticObject).
        if let Some(o) = self.heap.obj(fid) {
            if let ObjKind::Fun(Fun::Bound { target, bound, .. }) = &o.kind {
                let mut a = bound.clone();
                a.extend_from_slice(args);
                let target = *target;
                return self.construct(target, &a);
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
        let proto = match self.get_prop(fid, "prototype") {
            Some(Value::Obj(p)) => Value::Obj(p),
            _ => Value::Obj(self.protos.object),
        };
        let obj = Value::Obj(self.heap.alloc_obj(Object::ordinary(proto)));
        let r = self.call_value(f, obj, args)?;
        Ok(if r.is_object() { r } else { obj })
    }

    pub(crate) fn make_function(&mut self, lit: &Rc<FnLit>, env: &EnvRef, this: Value) -> Value {
        let proto = Value::Obj(self.protos.function);
        let id = self.heap.alloc_obj(Object::fun(
            Fun::Script {
                lit: lit.clone(),
                env: env.clone(),
                lex_this: lit.is_arrow.then_some(this),
            },
            proto,
        ));
        let p_obj = Value::Obj(self.protos.object);
        let p = self.heap.alloc_obj(Object::ordinary(p_obj));
        self.set_own(p, "constructor", Value::Obj(id), false);
        self.set_own(id, "prototype", Value::Obj(p), false);
        Value::Obj(id)
    }

    // -- hoisting ---------------------------------------------------------

    /// Function-level hoisting: `var` names declared, function
    /// declarations installed as values. Nested statements contribute
    /// `var` names only (Annex-B tolerance for inner function decls:
    /// declared `undefined`, installed when actually executed).
    fn hoist(&mut self, body: &[Stmt], env: &EnvRef) {
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
                        self.env_set(env, name, fv);
                    }
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
                Stmt::While { body, .. } | Stmt::DoWhile { body, .. } => {
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
        let prog = crate::parser::parse(src).map_err(|err| {
            Thrown(self.error_value(self.protos.syntax_error, "SyntaxError", &err.to_string()))
        })?;
        self.exec_program(&prog)
    }

    pub(crate) fn exec_program(&mut self, prog: &Program) -> JsResult<Value> {
        let env = self.global_env.clone();
        self.hoist(&prog.body, &env);
        self.completion = Value::Undefined;
        let this = Value::Obj(self.global);
        for s in &prog.body {
            match self.exec_stmt(&env, this, s)? {
                Flow::Normal | Flow::Break | Flow::Continue => {}
                Flow::Return(v) => return Ok(v),
            }
        }
        Ok(self.completion)
    }

    fn exec_body(&mut self, env: &EnvRef, this: Value, body: &[Stmt]) -> JsResult<Flow> {
        for s in body {
            let f = self.exec_stmt(env, this, s)?;
            if !matches!(f, Flow::Normal) {
                return Ok(f);
            }
        }
        Ok(Flow::Normal)
    }

    fn exec_stmt(&mut self, env: &EnvRef, this: Value, s: &Stmt) -> JsResult<Flow> {
        self.tick()?;
        match s {
            Stmt::Empty => Ok(Flow::Normal),
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
                    self.env_set(env, name, fv);
                }
                Ok(Flow::Normal)
            }
            // Class decls are `let`-like (no hoist — a use before the
            // decl is a ReferenceError, closest to spec TDZ).
            Stmt::ClassDecl(cl) => {
                let v = self.eval_class(env, this, cl)?;
                if let Some(name) = &cl.name {
                    Self::env_declare_lex(env, name, true);
                    self.env_set(env, name, v);
                }
                Ok(Flow::Normal)
            }
            Stmt::Block(stmts) => self.exec_body(env, this, stmts),
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
            Stmt::While { cond, body } => loop {
                self.tick()?;
                let c = self.eval_expr(env, this, cond)?;
                if !self.to_boolean(c) {
                    return Ok(Flow::Normal);
                }
                match self.exec_stmt(env, this, body)? {
                    Flow::Normal | Flow::Continue => {}
                    Flow::Break => return Ok(Flow::Normal),
                    f @ Flow::Return(_) => return Ok(f),
                }
            },
            Stmt::DoWhile { body, cond } => loop {
                self.tick()?;
                match self.exec_stmt(env, this, body)? {
                    Flow::Normal | Flow::Continue => {}
                    Flow::Break => return Ok(Flow::Normal),
                    f @ Flow::Return(_) => return Ok(f),
                }
                let c = self.eval_expr(env, this, cond)?;
                if !self.to_boolean(c) {
                    return Ok(Flow::Normal);
                }
            },
            Stmt::For {
                init,
                test,
                update,
                body,
            } => {
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
                    match self.exec_stmt(env, this, body)? {
                        Flow::Normal | Flow::Continue => {}
                        Flow::Break => return Ok(Flow::Normal),
                        f @ Flow::Return(_) => return Ok(f),
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
                let ov = self.eval_expr(env, this, obj)?;
                if let Some(init) = var_init {
                    let v = self.eval_expr(env, this, init)?;
                    self.assign_target(env, this, target, v)?;
                }
                let Value::Obj(id) = ov else {
                    return Ok(Flow::Normal); // for-in over primitives: skip
                };
                for key in self.enum_keys(id) {
                    let kv = self.str_val(key);
                    self.assign_target(env, this, target, kv)?;
                    match self.exec_stmt(env, this, body)? {
                        Flow::Normal | Flow::Continue => {}
                        Flow::Break => return Ok(Flow::Normal),
                        f @ Flow::Return(_) => return Ok(f),
                    }
                }
                Ok(Flow::Normal)
            }
            Stmt::ForOf { target, iter, body } => {
                let v = self.eval_expr(env, this, iter)?;
                for item in self.iter_values(v)? {
                    self.assign_target(env, this, target, item)?;
                    match self.exec_stmt(env, this, body)? {
                        Flow::Normal | Flow::Continue => {}
                        Flow::Break => return Ok(Flow::Normal),
                        f @ Flow::Return(_) => return Ok(f),
                    }
                }
                Ok(Flow::Normal)
            }
            Stmt::Break => Ok(Flow::Break),
            Stmt::Continue => Ok(Flow::Continue),
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
                        Flow::Break => return Ok(Flow::Normal),
                        f => return Ok(f),
                    }
                }
                Ok(Flow::Normal)
            }
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
                                for k in self.own_enum_keys(sid) {
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
                            Binding {
                                value: fv,
                                mutable: true,
                            },
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
            Expr::Chain { head, steps } => self.eval_chain(env, this, head, steps),
            Expr::Regex { pattern, flags } => self.make_regexp(pattern, flags),
            Expr::Class(cl) => self.eval_class(env, this, cl),
            Expr::Spread(_) => Err(self.type_error("`...` outside a call or array literal")),
            Expr::Await(e) => {
                let v = self.eval_expr(env, this, e)?;
                match v {
                    Value::Obj(id)
                        if matches!(
                            self.heap.obj(id).map(|o| &o.kind),
                            Some(ObjKind::Promise(_))
                        ) =>
                    {
                        let (state, result) = {
                            let Some(o) = self.heap.obj(id) else {
                                return Err(self.type_error("await: bad promise"));
                            };
                            let ObjKind::Promise(p) = &o.kind else {
                                return Err(self.type_error("await: bad promise"));
                            };
                            (p.state, p.result)
                        };
                        match state {
                            crate::promise::PromiseState::Fulfilled => Ok(result),
                            crate::promise::PromiseState::Rejected => Err(Thrown(result)),
                            // No continuations — `await` on pending
                            // work cannot suspend (documented).
                            crate::promise::PromiseState::Pending => {
                                Err(self.type_error("await on a pending promise is unsupported"))
                            }
                        }
                    }
                    // Non-promise await returns the value (spec wraps
                    // it — same observable result).
                    _ => Ok(v),
                }
            }
            Expr::SuperCall(arg_exprs) => {
                let parent = self.env_get(env, SUPER_BIND);
                let Some(parent) = parent else {
                    return Err(self.type_error("`super()` outside a constructor"));
                };
                if !self.is_callable(parent) {
                    return Err(self.type_error("superclass is not a constructor"));
                }
                let args = self.eval_args(env, this, arg_exprs)?;
                self.call_value(parent, this, &args)?;
                Ok(this)
            }
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
                    Callee::Bare(f) => self.call_value(f, Value::Obj(self.global), &args),
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
            Expr::Ident(name) => {
                self.env_set(env, name, v);
                Ok(())
            }
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
                Ok(Value::Num(f64::from(!to_int32(self.to_number(v)?))))
            }
            UnOp::Pos => {
                let v = self.eval_expr(env, this, operand)?;
                Ok(Value::Num(self.to_number(v)?))
            }
            UnOp::Neg => {
                let v = self.eval_expr(env, this, operand)?;
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
                        Ok(Value::Bool(self.delete_prop(id, name)))
                    } else {
                        Ok(Value::Bool(true))
                    }
                }
                Expr::Index(base, key) => {
                    let b = self.eval_expr(env, this, base)?;
                    let k = self.eval_expr(env, this, key)?;
                    let k = self.prop_key(k)?;
                    if let Value::Obj(id) = b {
                        Ok(Value::Bool(self.delete_prop(id, &k)))
                    } else {
                        Ok(Value::Bool(true))
                    }
                }
                // `delete x` on a variable: vars are non-configurable.
                _ => {
                    self.eval_expr(env, this, operand).ok();
                    Ok(Value::Bool(false))
                }
            },
            UnOp::PreInc | UnOp::PreDec | UnOp::PostInc | UnOp::PostDec => {
                let cur = self.read_lvalue(env, this, operand)?;
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
                let lp = self.to_primitive(l, false)?;
                let rp = self.to_primitive(r, false)?;
                if matches!(lp, Value::Str(_)) || matches!(rp, Value::Str(_)) {
                    let ls = self.to_js_string(lp)?;
                    let rs = self.to_js_string(rp)?;
                    self.str_val(format!("{ls}{rs}"))
                } else {
                    Value::Num(self.to_number(lp)? + self.to_number(rp)?)
                }
            }
            Sub => Value::Num(self.to_number(l)? - self.to_number(r)?),
            Mul => Value::Num(self.to_number(l)? * self.to_number(r)?),
            Div => Value::Num(self.to_number(l)? / self.to_number(r)?),
            Mod => Value::Num(self.to_number(l)? % self.to_number(r)?),
            Pow => Value::Num(self.to_number(l)?.powf(self.to_number(r)?)),
            Shl => Value::Num(f64::from(
                to_int32(self.to_number(l)?) << (to_uint32(self.to_number(r)?) & 31),
            )),
            Shr => Value::Num(f64::from(
                to_int32(self.to_number(l)?) >> (to_uint32(self.to_number(r)?) & 31),
            )),
            ShrU => Value::Num(f64::from(
                to_uint32(self.to_number(l)?) >> (to_uint32(self.to_number(r)?) & 31),
            )),
            BitAnd => Value::Num(f64::from(
                to_int32(self.to_number(l)?) & to_int32(self.to_number(r)?),
            )),
            BitOr => Value::Num(f64::from(
                to_int32(self.to_number(l)?) | to_int32(self.to_number(r)?),
            )),
            BitXor => Value::Num(f64::from(
                to_int32(self.to_number(l)?) ^ to_int32(self.to_number(r)?),
            )),
            Lt | Le | Gt | Ge => {
                let lp = self.to_primitive(l, false)?;
                let rp = self.to_primitive(r, false)?;
                let ord = if let (Value::Str(a), Value::Str(b)) = (lp, rp) {
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
            Instanceof => {
                let Value::Obj(fid) = r else {
                    return Err(self.type_error("`instanceof` requires an object"));
                };
                if !self.is_callable(r) {
                    return Err(self.type_error("right side of `instanceof` is not callable"));
                }
                let Some(Value::Obj(proto)) = self.get_prop(fid, "prototype") else {
                    return Err(self.type_error("constructor has no prototype"));
                };
                let mut cur = match l {
                    Value::Obj(id) => match self.heap.obj(id).map(|o| o.proto) {
                        Some(Value::Obj(p)) => Some(p),
                        _ => None,
                    },
                    _ => return Ok(Value::Bool(false)),
                };
                let mut found = false;
                let mut depth = 0;
                while let Some(id) = cur {
                    if id == proto {
                        found = true;
                        break;
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
                Value::Bool(found)
            }
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
            (Value::Num(_), Value::Str(_))
            | (Value::Str(_), Value::Num(_))
            | (Value::Bool(_), _)
            | (_, Value::Bool(_)) => {
                let ln = self.to_number(l)?;
                let rn = self.to_number(r)?;
                Ok(ln == rn)
            }
            (Value::Obj(_), Value::Num(_) | Value::Str(_)) => {
                let p = self.to_primitive(l, false)?;
                self.loose_eq(p, r)
            }
            (Value::Num(_) | Value::Str(_), Value::Obj(_)) => {
                let p = self.to_primitive(r, false)?;
                self.loose_eq(l, p)
            }
            _ => Ok(false),
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
        Value::Obj(id) => match heap.obj(id).map(|o| &o.kind) {
            Some(ObjKind::Fun(_)) => "function",
            Some(ObjKind::Symbol(_)) => "symbol",
            _ => "object",
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
