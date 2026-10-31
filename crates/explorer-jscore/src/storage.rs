// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! `localStorage`/`sessionStorage` (JS-7e) — key/value stores.
//!
//! The store itself is a shared `Rc<RefCell<BTreeMap>>` held by the
//! engine; the embedder can share the `localStorage` map across
//! pages of the same origin (or persist it) while `sessionStorage`
//! is per-page by construction.

use crate::heap::{Host, JsResult};
use crate::value::Value;
use crate::Engine;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

/// Shared store backing a `localStorage`/`sessionStorage` object.
pub type Storage = Rc<RefCell<BTreeMap<String, String>>>;

/// One `Storage` host object.
struct StorageHost {
    map: Storage,
}

fn arg(args: &[Value], i: usize) -> Value {
    args.get(i).copied().unwrap_or(Value::Undefined)
}

impl Engine {
    /// Install `localStorage`/`sessionStorage` globals — `local` is
    /// shared with the embedder (per-origin persistence is its
    /// business), `session` is a fresh page-local map.
    pub fn install_storage(&mut self, local: &Storage) {
        let lv = self.new_host_object(Rc::new(StorageHost {
            map: Rc::clone(local),
        }));
        let sv = self.new_host_object(Rc::new(StorageHost {
            map: Rc::new(RefCell::new(BTreeMap::new())),
        }));
        let g = self.global;
        // Interface `Storage` (§B-14 vol 403 vite.dev —
        // `x instanceof Storage`/`typeof Storage` : les globals
        // `localStorage`/`sessionStorage` orphelins rendaient
        // `Storage is not defined`). Ctor illégal spec +
        // `Storage.prototype` chaîné aux deux instances.
        let f = self.illegal_ctor_bound("Storage");
        let p = self.new_object();
        let tag = self.str_val("Storage");
        self.set_own(p, crate::interp::SYM_TO_STRING_TAG, tag, false);
        if let Value::Obj(cid) = f {
            self.set_own(cid, "prototype", Value::Obj(p), false);
            self.set_own(p, "constructor", f, false);
        }
        for v in [lv, sv] {
            if let Value::Obj(id) = v {
                if let Some(o) = self.heap.obj_mut(id) {
                    o.proto = Value::Obj(p);
                }
            }
        }
        self.set_own(g, "Storage", f, false);
        self.set_own(g, "localStorage", lv, false);
        self.set_own(g, "sessionStorage", sv, false);
    }
}

impl Host for StorageHost {
    fn class_name(&self) -> &'static str {
        "Storage"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn get(&self, e: &mut Engine, _t: Value, key: &str) -> JsResult<Option<Value>> {
        if key == "length" {
            return Ok(Some(Value::Num(self.map.borrow().len() as f64)));
        }
        // Named property access `storage["k"]` — methods win per
        // WebIDL legacy-platform-object order (proto lookup happens
        // first for `getItem` & co anyway since we return None).
        match self.map.borrow().get(key).cloned() {
            Some(v) => Ok(Some(e.str_val(v))),
            None => Ok(None),
        }
    }

    fn set(&self, e: &mut Engine, _t: Value, key: &str, v: Value) -> JsResult<bool> {
        let s = e.to_js_string(v)?;
        self.map.borrow_mut().insert(key.to_string(), s);
        Ok(true)
    }

    fn invoke(
        &self,
        e: &mut Engine,
        _t: Value,
        name: &str,
        args: &[Value],
    ) -> JsResult<Option<Value>> {
        match name {
            "getItem" => {
                let k = e.to_js_string(arg(args, 0))?;
                Ok(Some(match self.map.borrow().get(&k).cloned() {
                    Some(v) => e.str_val(v),
                    None => Value::Null,
                }))
            }
            "setItem" => {
                let k = e.to_js_string(arg(args, 0))?;
                let v = e.to_js_string(arg(args, 1))?;
                self.map.borrow_mut().insert(k, v);
                Ok(Some(Value::Undefined))
            }
            "removeItem" => {
                let k = e.to_js_string(arg(args, 0))?;
                self.map.borrow_mut().remove(&k);
                Ok(Some(Value::Undefined))
            }
            "clear" => {
                self.map.borrow_mut().clear();
                Ok(Some(Value::Undefined))
            }
            "key" => {
                let i = e.to_number(arg(args, 0))? as usize;
                let k = self.map.borrow().keys().nth(i).cloned();
                Ok(Some(match k {
                    Some(k) => e.str_val(k),
                    None => Value::Null,
                }))
            }
            _ => Ok(None),
        }
    }
}
