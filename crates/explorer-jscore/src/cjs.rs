// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Globals `CommonJS` en sandbox — résidu §B-105 (JS-C448,
//! palier JT-3).
//!
//! Un script qui référence `module`/`exports`/`require`
//! **à nu** meurt d'un `ReferenceError` (bundles CJS,
//! chunks servis hors de leur wrapper, chemins UMD non
//! gardés). Les bundles sains se gardent par `typeof` —
//! `debounce`/`chart.umd` prennent leur branche navigateur
//! et ne voient jamais ces globals : **on ne peut pas les
//! installer sur le global** (`typeof module` passerait
//! `"object"` → toutes les UMD basculeraient en branche
//! CJS et ne poseraient plus `window.X`).
//!
//! Le sandbox est donc un **repli sur échec** : la première
//! éval tourne dans `global_env` normalement ; si elle meurt
//! d'un `ReferenceError` `module`/`exports`/`require
//! is not defined`, elle est re-exécutée dans un env enfant
//! (`Env::local` — l'équivalent exact du wrapper fonction de
//! `Node` : `var` reste module-scope, les globals restent
//! visibles par la chaîne de scopes) semé de :
//!
//! - `module` — `{exports:{}, id, filename}` (`id`/`filename`
//!   = nom du script quand `eval_named`/`eval_script_el`
//!   l'a fourni) ;
//! - `exports` — alias de `module.exports` (une
//!   ré-assignation `module.exports = …` casse l'alias,
//!   comme `Node`) ;
//! - `require(spec)` — retourne le `module.exports` des
//!   scripts déjà évalués sous sandbox (clé exacte = nom
//!   du script, puis dernier segment de chemin) ; sinon
//!   `Error: Cannot find module 'spec'` — **jamais** de
//!   fetch ni d'éval réseau.
//!
//! Déviations documentées : le `var` top-level du script
//! replié vit dans le scope sandbox (pas le global —
//! sémantique `Node` du wrapper, les identifiants déclarés
//! par la première tentative en `global_env` peuvent
//! survivre comme props globales `undefined`) ; le repli
//! rejoue `hoist`+`exec` — borné à une seule tentative.

use crate::heap::{EnvRef, JsResult, Thrown};
use crate::value::{ObjId, Value};
use crate::Engine;

/// Cap du registre `nom de script → module.exports` —
/// éviction FIFO, l'historique complet n'est pas tenu.
pub(crate) const REGISTRY_CAP: usize = 64;

/// Noms qui arment le repli — toute autre `ReferenceError`
/// (ou autre classe d'erreur) remonte telle quelle.
const CJS_NAMES: [&str; 3] = ["module", "exports", "require"];

impl Engine {
    /// `true` si `t` est un `ReferenceError` « `X is not
    /// defined` » sur un global `CommonJS` — le seul signal
    /// qui arme le sandbox.
    pub(crate) fn is_cjs_ref_error(&mut self, t: &Thrown) -> bool {
        let Thrown(Value::Obj(id)) = t else {
            return false;
        };
        let id = *id;
        let is_ref = self
            .get_prop(id, "name")
            .as_ref()
            .and_then(Self::heap_string_of)
            .is_some_and(|n| n == "ReferenceError");
        if !is_ref {
            return false;
        }
        let msg = self
            .get_prop(id, "message")
            .as_ref()
            .and_then(Self::heap_string_of)
            .unwrap_or_default();
        CJS_NAMES
            .iter()
            .any(|n| msg == format!("{n} is not defined"))
    }

    /// Env sandbox semé des globals CJS. Renvoie
    /// `(env, module)` — `module` sert à récolter
    /// `module.exports` si l'éval réussit.
    pub(crate) fn cjs_sandbox_env(&mut self, parent: &EnvRef) -> (EnvRef, ObjId) {
        let env = crate::heap::Env::local(parent.clone());
        let exports = self.new_object();
        let module = self.new_object();
        self.set_own(module, "exports", Value::Obj(exports), false);
        if let Some(n) = self.cjs_eval_name.clone() {
            let id = self.str_val(n.as_ref());
            self.set_own(module, "id", id.clone(), false);
            self.set_own(module, "filename", id, false);
        }
        let req = self.native_fn("require", Self::cjs_require);
        for (name, v) in [
            ("module", Value::Obj(module)),
            ("exports", Value::Obj(exports)),
            ("require", req),
        ] {
            self.env_declare(&env, name);
            let _ = self.env_set(&env, name, v);
        }
        (env, module)
    }

    /// Enregistre le `module.exports` d'une éval sandbox
    /// réussie — clé = `cjs_eval_name` (nom/URL du script)
    /// ou `<inline>` ; borne FIFO + racine GC sur les objets.
    pub(crate) fn cjs_register_exports(&mut self, exports: Value) {
        let key = self
            .cjs_eval_name
            .clone()
            .unwrap_or_else(|| std::rc::Rc::from("<inline>"));
        self.cjs_registry.retain(|(k, _)| k != &key);
        if self.cjs_registry.len() >= REGISTRY_CAP {
            self.cjs_registry.remove(0);
        }
        if let Value::Obj(id) = exports {
            self.add_root(id);
        }
        self.cjs_registry.push((key, exports));
    }

    /// `require(spec)` du sandbox — `module.exports` des
    /// scripts déjà évalués sous sandbox (clé exacte, puis
    /// dernier segment de chemin), sinon `Error` honnête
    /// « Cannot find module ». Jamais de fetch ni d'éval.
    fn cjs_require(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
        let spec = args
            .first()
            .and_then(|v| e.to_js_string(v.clone()).ok())
            .unwrap_or_default();
        let hit = e
            .cjs_registry
            .iter()
            .find(|(k, _)| k.as_ref() == spec)
            .or_else(|| {
                let base = spec.rsplit('/').next().unwrap_or(spec.as_str());
                e.cjs_registry
                    .iter()
                    .find(|(k, _)| k.rsplit('/').next().unwrap_or(k.as_ref()) == base)
            })
            .map(|(_, v)| v.clone());
        if let Some(v) = hit {
            return Ok(v);
        }
        let p = e.protos.error;
        Err(Thrown(e.error_value(
            p,
            "Error",
            &format!("Cannot find module '{spec}'"),
        )))
    }
}
