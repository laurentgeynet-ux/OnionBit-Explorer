// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Heap, objects, environments — the arena at the heart of `dillo-js`.
//!
//! Same ownership philosophy as ADR-0005's widget arena: every object
//! lives in `Heap.cells` and is referenced by `ObjId` (a `u32`), so
//! object graphs with cycles (prototype chains, closures capturing
//! scopes that capture functions) never need `Rc<RefCell>` puzzles.
//! There is no GC pass in v0.1: an [`crate::Engine`] is per-page, so
//! the arena simply dies with it — bounded, predictable, no leaks
//! across navigations.

use crate::ast::FnLit;
use crate::value::{ObjId, Value};
use std::cell::RefCell;
use std::collections::HashMap;
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

/// One arena slot.
pub struct Cell {
    /// Payload.
    pub data: CellData,
}

/// The object arena.
pub struct Heap {
    cells: Vec<Cell>,
}

impl Heap {
    /// Empty heap.
    pub fn new() -> Self {
        Heap { cells: Vec::new() }
    }

    fn alloc(&mut self, data: CellData) -> ObjId {
        self.cells.push(Cell { data });
        (self.cells.len() - 1) as ObjId
    }

    /// Allocate a string, returning its id.
    pub fn alloc_str(&mut self, s: impl Into<String>) -> ObjId {
        self.alloc(CellData::Str(s.into()))
    }

    /// Allocate an object.
    pub fn alloc_obj(&mut self, o: Object) -> ObjId {
        self.alloc(CellData::Obj(o))
    }

    /// Borrow a string's contents (cloned — strings are short here).
    pub fn string(&self, id: ObjId) -> Option<String> {
        match self.cells.get(id as usize) {
            Some(Cell {
                data: CellData::Str(s),
            }) => Some(s.clone()),
            _ => None,
        }
    }

    /// Borrow an object.
    pub fn obj(&self, id: ObjId) -> Option<&Object> {
        match self.cells.get(id as usize) {
            Some(Cell {
                data: CellData::Obj(o),
            }) => Some(o),
            _ => None,
        }
    }

    /// Borrow an object mutably.
    pub fn obj_mut(&mut self, id: ObjId) -> Option<&mut Object> {
        match self.cells.get_mut(id as usize) {
            Some(Cell {
                data: CellData::Obj(o),
            }) => Some(o),
            _ => None,
        }
    }
}

/// A property entry. `enumerable` gates `for-in` enumeration
/// (builtins install themselves as non-enumerable).
#[derive(Debug, Clone, Copy)]
pub enum Prop {
    /// Plain data property.
    Data {
        /// Stored value.
        value: Value,
        /// Shows up in `for-in`/`Object.keys`-style enumeration.
        enumerable: bool,
    },
    /// Getter/setter pair (`{get x(){}}`, `Object.defineProperty`,
    /// class accessors). `Undefined` = absent side.
    Acc {
        /// Getter callable or `Undefined`.
        get: Value,
        /// Setter callable or `Undefined`.
        set: Value,
        /// Enumeration flag.
        enumerable: bool,
    },
}

impl Prop {
    /// `true` when the property shows up in `for-in`.
    pub fn enumerable(&self) -> bool {
        match self {
            Prop::Data { enumerable, .. } | Prop::Acc { enumerable, .. } => *enumerable,
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
    /// `Object.freeze` — own writes become silent no-ops (sloppy
    /// mode semantics; no accessor/install-time distinction).
    pub frozen: bool,
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
            frozen: false,
        }
    }

    /// New array.
    pub fn array(elems: Vec<Value>, proto: Value) -> Self {
        Object {
            proto,
            props: Vec::new(),
            kind: ObjKind::Array(elems),
            frozen: false,
        }
    }

    /// New callable.
    pub fn fun(f: Fun, proto: Value) -> Self {
        Object {
            proto,
            props: Vec::new(),
            kind: ObjKind::Fun(f),
            frozen: false,
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

    /// Insert or overwrite an own data property (keeps insertion
    /// order). A data write over an existing accessor replaces it.
    /// Frozen objects silently reject the write (sloppy mode).
    pub fn set_own(&mut self, key: &str, value: Value, enumerable: bool) {
        if self.frozen {
            return;
        }
        if let Some((_, p)) = self.props.iter_mut().find(|(k, _)| k == key) {
            *p = Prop::Data { value, enumerable };
            return;
        }
        self.props
            .push((key.to_string(), Prop::Data { value, enumerable }));
    }

    /// Merge a getter/setter side into an own property (existing
    /// accessor sides are preserved, à la `Object.defineProperty`).
    pub fn set_accessor(&mut self, key: &str, get: Value, set: Value, enumerable: bool) {
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
                    *p = Prop::Acc {
                        get,
                        set,
                        enumerable,
                    };
                    return;
                }
            }
        }
        self.props.push((
            key.to_string(),
            Prop::Acc {
                get,
                set,
                enumerable,
            },
        ));
    }

    /// Remove an own property. Returns whether it existed.
    pub fn remove_own(&mut self, key: &str) -> bool {
        let before = self.props.len();
        self.props.retain(|(k, _)| k != key);
        self.props.len() != before
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
    Global(HashMap<String, Binding>),
    /// Function/catch scope.
    Local(HashMap<String, Binding>),
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
