// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! ES module machinery (JS-C5): instantiation (import/export binding
//! wiring over a dependency graph) then evaluation in DFS post-order.
//!
//! Modelled on spec `InnerModuleInstantiation`/`InnerModuleEvaluation`,
//! with documented approximations : no TDZ (an uninitialized binding
//! reads `undefined`), cycles resolve in DFS order without the spec's
//! async-completion machinery, and specifier→identity is the raw
//! specifier string (the host loader is expected to canonicalise —
//! two different spellings of the same file may instantiate twice).

use crate::ast::{ExportDecl, ImportBind, Pat, Stmt};
use crate::error::{Error, Result};
use crate::heap::{Env, EnvCell, EnvRef, JsResult, ObjKind, Object, PropAttrs, Thrown};
use crate::interp::IMPORT_META;
use crate::parser;
use crate::value::Value;
use crate::Engine;
use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

/// Host-side module source provider. `load` resolves `specifier`
/// relative to `referrer` (the importing module's specifier, `""`
/// for the entry) and returns its source text — the embedder owns
/// URL resolution, fetch and cache policy.
pub trait ModuleLoader {
    /// Fetch the source for `specifier` as seen from `referrer`.
    ///
    /// # Errors
    /// `Err` message is surfaced to the script as a module error.
    fn load(&mut self, specifier: &str, referrer: &str) -> std::result::Result<String, String>;

    /// Résolution optionnelle `specifier`+`referrer` → URL du
    /// module — devient son `import.meta.url` et le référent
    /// des imports qu'il émet (spec : `import.meta.url` = l'URL
    /// du module *propriétaire*). `None` (défaut) = le
    /// spécificateur import-map tient lieu d'identité — des
    /// bundles qui font `new URL(x, import.meta.url)` voient
    /// alors un chemin relatif, cause réelle de `Invalid base
    /// URL` mesuré en terrain.
    fn resolve(&mut self, _specifier: &str, _referrer: &str) -> Option<String> {
        None
    }
}

/// Evaluation status of a [`Module`] — `Evaluating` marks the DFS
/// stack so cyclic imports stop instead of recursing forever.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ModState {
    /// Bindings wired, body not yet executed.
    Instantiated,
    /// On the evaluation DFS stack — a dep-edge back means a cycle.
    Evaluating,
    /// Body executed to completion.
    Evaluated,
}

/// One instantiated module: its scope env, its export table and its
/// dependency edges (post-order evaluation order = dep order).
pub(crate) struct Module {
    /// Module scope — a `Local` env whose parent is the global env.
    pub env: EnvRef,
    /// `(exported name, defining env, binding name)` — live exports;
    /// the defining env is this module's for local exports, a
    /// dependency's for `export {x} from 'm'`/`export * from 'm'`.
    /// Local entries exist as soon as the module is registered —
    /// cyclic imports can link against them before the body runs.
    pub exports: Vec<(String, EnvRef, String)>,
    /// DFS status.
    pub state: ModState,
    /// Instantiated dependency indices, in source order.
    pub deps: Vec<usize>,
    /// Spec résolu du module (URL côté hôte, spécificateur
    /// sinon) — `import.meta.url` et `current_module` pendant
    /// l'évaluation.
    pub spec: String,
    /// Re-exports waiting on dep instantiation (`export {…} from`,
    /// `export * [as ns] from`).
    pub pending: Vec<PendingExport>,
    /// Parsed body — kept for evaluation.
    pub body: Vec<Stmt>,
}

/// A re-export that needs its dependency's export table — resolved
/// once that dep is instantiated.
pub(crate) enum PendingExport {
    /// `export {a as b} from 'm'` — `(local, exported)`.
    Named {
        /// `(local, exported)` pairs.
        list: Vec<(String, String)>,
        /// Specifier.
        from: String,
    },
    /// `export * [as ns] from 'm'`.
    Star {
        /// `export * as ns` target name.
        ns: Option<String>,
        /// Specifier.
        from: String,
    },
}

/// Recursion bound on the instantiation graph (deep import chains).
const MAX_MODULE_DEPTH: u32 = 200;

/// Static `import`/`export … from` specifiers of module `src`, in
/// source order — lets the host prefetch the dependency graph
/// (browser step "fetch the descendants") *before* `eval_module`,
/// which keeps the loader synchronous and deadlock-free.
/// Parse failure yields an empty list: the real parse error is
/// then reported by `eval_module` itself.
#[must_use]
pub fn module_specifiers(src: &str) -> Vec<String> {
    let Ok(prog) = parser::parse_module(src) else {
        return Vec::new();
    };
    prog.body
        .iter()
        .filter_map(|s| match s {
            Stmt::Import { source, .. } => Some(source.clone()),
            Stmt::Export(
                ExportDecl::Named { from: Some(f), .. } | ExportDecl::Star { from: f, .. },
            ) => Some(f.clone()),
            _ => None,
        })
        .collect()
}

impl Engine {
    /// Inject the host module loader used by `import`/`export …from`
    /// and dynamic `import()`.
    pub fn set_module_loader(&mut self, loader: Rc<RefCell<dyn ModuleLoader>>) {
        self.module_loader = Some(loader);
    }

    /// Enregistre une import map (JS-C44) : `json` est le contenu
    /// texte d'un `<script type="importmap">`. Minimal — seule la
    /// clé `imports` est lue (`scopes` ignoré, les valeurs non
    /// string sont sautées, une map invalide est ignorée sans
    /// erreur — tolérance miroir). Fusion additive : une clé déjà
    /// enregistrée garde sa première cible (spec « merge » des
    /// maps multiples). Les bare specifiers sont ensuite réécrits
    /// dans `load_source` avant l'appel au `ModuleLoader`.
    pub fn set_import_map(&mut self, json: &str) {
        let text = self.str_val(json);
        let Ok(v) = crate::json::json_parse(self, Value::Undefined, &[text]) else {
            return;
        };
        // Lecture directe de `props` — `JSON.parse` ne produit que
        // des props data, aucun getter à déclencher.
        let Value::Obj(root) = v else { return };
        let Some(imports) = self
            .heap
            .obj(root)
            .and_then(|o| o.props.iter().find(|(k, _)| k == "imports"))
            .and_then(|(_, p)| p.data_value())
        else {
            return;
        };
        let Value::Obj(oidx) = imports else { return };
        // Les paires sont collectées sous le borrow puis relâché —
        // `heap.string` re-emprunte.
        let pairs: Vec<(String, Value)> = self
            .heap
            .obj(oidx)
            .map(|o| {
                o.props
                    .iter()
                    .filter_map(|(k, p)| p.data_value().map(|val| (k.clone(), val)))
                    .collect()
            })
            .unwrap_or_default();
        for (key, val) in pairs {
            let Value::Str(id) = val else { continue };
            let Some(target) = self.heap.string(id) else {
                continue;
            };
            if self.import_map.iter().all(|(k, _)| *k != key) {
                self.import_map.push((key, target));
            }
        }
    }

    /// Traduction d'un spécificateur bare via l'import map —
    /// délègue à [`rewrite_bare_specifier`] (même règle
    /// consommée par le prefetch session — issue #6).
    /// `None` hors map : le `ModuleLoader` garde son message
    /// d'erreur actuel.
    fn resolve_bare(&self, spec: &str) -> Option<String> {
        rewrite_bare_specifier(&self.import_map, spec)
    }

    /// Fetch a dependency's source through the injected loader —
    /// rend `(url_résolue, source)` : l'URL est celle rapportée
    /// par `ModuleLoader::resolve` (ou le spécificateur
    /// import-map à défaut) — identité du module pour
    /// `import.meta.url` et les référents aval.
    fn load_source(&mut self, spec: &str, referrer: &str) -> JsResult<(String, String)> {
        let Some(loader) = self.module_loader.clone() else {
            return Err(self.type_error(format!("no module loader for `{spec}`")));
        };
        let resolved = self.resolve_bare(spec).unwrap_or_else(|| spec.to_string());
        let url = loader
            .borrow_mut()
            .resolve(&resolved, referrer)
            .unwrap_or_else(|| resolved.clone());
        let r = loader.borrow_mut().load(&resolved, referrer);
        match r {
            Ok(src) => Ok((url, src)),
            Err(msg) => Err(self.type_error(format!("cannot load module `{spec}`: {msg}"))),
        }
    }

    /// Instantiate and evaluate a whole module program; returns its
    /// namespace object (`{export: value}` with live getters).
    ///
    /// # Errors
    /// `Error::Syntax` on parse failure, `Error::Uncaught` if a module
    /// body threw or a specifier could not be resolved/loaded.
    pub fn eval_module(&mut self, src: &str, name: &str) -> Result<Value> {
        self.ops_left = self.op_limit;
        let prev = std::mem::replace(&mut self.current_module, name.to_string());
        let r = self.eval_module_inner(src, name);
        self.current_module = prev;
        let out = match r {
            Ok(v) => Ok(v),
            Err(Thrown(v)) => Err(Error::Uncaught(self.describe(v))),
        };
        if let Err(Thrown(v)) = self.run_microtasks() {
            return Err(Error::Uncaught(self.describe(v)));
        }
        out
    }

    fn eval_module_inner(&mut self, src: &str, name: &str) -> JsResult<Value> {
        let idx = self.instantiate(name, name, src, 0)?;
        self.evaluate_module(idx)?;
        Ok(self.module_ns_object(idx))
    }

    /// `import(spec)` — resolve, instantiate, evaluate → namespace.
    /// `referrer` = l'URL du module importeur (lue via la liaison
    /// cachée `IMPORT_META` de son env — couvre les appels depuis
    /// fonctions/callbacks, où `current_module` ne dit rien).
    /// Miss sur le chargeur → fetch à la demande si la session
    /// est branchée (JS-C100) : la promesse reste pending et
    /// une `DynImportReq` part dans `drain_dyn_imports`.
    pub(crate) fn dynamic_import(&mut self, spec: &str, referrer: &str) -> JsResult<Value> {
        match self.load_source(spec, referrer) {
            Ok((url, src)) => {
                let idx = self.instantiate(spec, &url, &src, 0)?;
                self.evaluate_module(idx)?;
                Ok(self.module_ns_object(idx))
            }
            Err(_) if self.import_drain_wired => self.defer_import(spec, referrer),
            Err(err) => Err(err),
        }
    }

    /// Miss `load_source` — l'URL résolue part en
    /// `DynImportReq`, la promesse `import()` reste pending
    /// jusqu'à `deliver_import` (JS-C100). Repli sur le rejet
    /// d'origine si le drain n'est pas branché.
    fn defer_import(&mut self, spec: &str, referrer: &str) -> JsResult<Value> {
        let resolved = self.resolve_bare(spec).unwrap_or_else(|| spec.to_string());
        let url = self
            .module_loader
            .clone()
            .and_then(|l| l.borrow_mut().resolve(&resolved, referrer))
            .unwrap_or_else(|| resolved.clone());
        let pid = self.new_promise();
        let seq = self.dyn_import_seq;
        self.dyn_import_seq += 1;
        self.pending_imports.insert(
            seq,
            PendingImport {
                spec: resolved,
                url: url.clone(),
                promise: pid,
            },
        );
        self.dyn_import_reqs.push(DynImportReq {
            seq,
            url,
            ctx: self.cur_ctx,
        });
        Ok(Value::Obj(pid))
    }

    /// Livraison d'un fetch `import()` drainé (JS-C100) :
    /// `Ok(src)` → instancier + évaluer + résoudre la promesse
    /// pending avec le namespace ; `Err(msg)` → rejet.
    pub fn deliver_import(&mut self, seq: u64, src: std::result::Result<String, String>) {
        let Some(p) = self.pending_imports.remove(&seq) else {
            return;
        };
        match src {
            Ok(s) => {
                match self
                    .instantiate(&p.spec, &p.url, &s, 0)
                    .and_then(|i| self.evaluate_module(i).map(|()| i))
                {
                    Ok(idx) => {
                        let ns = self.module_ns_object(idx);
                        self.resolve_promise(p.promise, ns);
                    }
                    Err(Thrown(t)) => self.reject_promise(p.promise, t),
                }
            }
            Err(msg) => {
                let v = self.str_val(msg);
                self.reject_promise(p.promise, v);
            }
        }
    }

    /// Drain des requêtes `import()` — la session fetch les
    /// URL et rappelle `deliver_import` (JS-C100).
    pub fn drain_dyn_imports(&mut self) -> Vec<DynImportReq> {
        std::mem::take(&mut self.dyn_import_reqs)
    }

    /// Branchement session (JS-C100) — active le defer sur
    /// miss du chargeur.
    pub fn set_import_drain(&mut self) {
        self.import_drain_wired = true;
    }

    /// `InnerModuleInstantiation` — registers the module in the
    /// specifier cache *before* recursing (cycles re-resolve to the
    /// same instance), wires import aliases and builds the export
    /// table. Idempotent by specifier.
    fn instantiate(&mut self, spec: &str, url: &str, src: &str, depth: u32) -> JsResult<usize> {
        if let Some(&idx) = self.module_specs.get(spec) {
            return Ok(idx);
        }
        // Identité canonique = l'URL résolue : deux orthographes
        // du même fichier (`./assets/x.js` depuis la racine,
        // `./x.js` depuis `assets/`) partagent l'instance — le
        // spécificateur est aliasé vers elle pour les lookups
        // `import` aval. Sans ça, le corps s'évalue une fois par
        // orthographe et le graphe d'objets top-level est
        // dupliqué (mesuré : routeTree x-web cloné, ids de
        // routes `undefined/…`, invariant TanStack).
        if let Some(&idx) = self.module_urls.get(url) {
            self.module_specs.insert(spec.to_string(), idx);
            return Ok(idx);
        }
        if depth > MAX_MODULE_DEPTH {
            return Err(self.type_error("module graph too deep"));
        }
        let prog =
            parser::parse_module_in(src, self.nest_limit, Some(Rc::from(spec))).map_err(|err| {
                // La faute porte le nom du module fautif, pas celui
                // de l'entrée — `session` préfixe déjà « module
                // « entry » », sans lui la source est introuvable
                // (debug réel : erreur attribuée à landing-pages
                // alors que le coupable était une dep `ns-*.js`).
                Thrown(self.error_value(
                    self.protos.syntax_error,
                    "SyntaxError",
                    &format!("{err} — «{spec}»"),
                ))
            })?;
        let env = Env::local(self.global_env.clone());
        // Liaison lexicale du spec résolu — `import.meta.url` et
        // le référent de `import()` se résolvent dans l'env du
        // module *propriétaire* (fonctions, closures, timers —
        // spec `HostGetImportMetaProperties`), pas dans un
        // « module courant » global déjà restauré quand une
        // microtâche s'évalue.
        let meta = self.str_val(url);
        Self::env_put(&env, IMPORT_META, meta, false);
        let idx = self.modules.len();
        self.modules.push(Module {
            env: env.clone(),
            exports: Vec::new(),
            state: ModState::Instantiated,
            deps: Vec::new(),
            spec: url.to_string(),
            pending: Vec::new(),
            body: prog.body.clone(),
        });
        self.module_specs.insert(spec.to_string(), idx);
        self.module_urls.insert(url.to_string(), idx);
        self.module_envs.push(env.clone());

        // 1. Static export pass — les entrées *locales* sont connues
        //    dès l'enregistrement : un module cyclique qui remonte
        //    vers celui-ci y trouve déjà ses liaisons (spec : les
        //    noms exportés sont statiques). `from`/`*` attendent
        //    leurs deps → `pending`.
        for s in &prog.body {
            let Stmt::Export(d) = s else { continue };
            match d {
                ExportDecl::Named { list, from } => match from {
                    None => {
                        for (local, exported) in list {
                            self.modules[idx].exports.push((
                                exported.clone(),
                                env.clone(),
                                local.clone(),
                            ));
                        }
                    }
                    Some(f) => self.modules[idx].pending.push(PendingExport::Named {
                        list: list.clone(),
                        from: f.clone(),
                    }),
                },
                ExportDecl::Decl(inner) => {
                    for name in decl_names(inner) {
                        self.modules[idx]
                            .exports
                            .push((name.clone(), env.clone(), name));
                    }
                }
                ExportDecl::Default(_) => self.modules[idx].exports.push((
                    "default".into(),
                    env.clone(),
                    "default".into(),
                )),
                ExportDecl::Star { ns, from } => {
                    self.modules[idx].pending.push(PendingExport::Star {
                        ns: ns.clone(),
                        from: from.clone(),
                    });
                }
            }
        }

        // 2. Instantiate every dependency (source order — imports,
        //    `from` re-exports, side-effect imports).
        let mut dep_specs = Vec::new();
        for s in &prog.body {
            match s {
                Stmt::Import { source, .. } => dep_specs.push(source.clone()),
                Stmt::Export(ExportDecl::Named { from: Some(f), .. }) => {
                    dep_specs.push(f.clone());
                }
                Stmt::Export(ExportDecl::Star { from, .. }) => dep_specs.push(from.clone()),
                _ => {}
            }
        }
        for dep_spec in dep_specs {
            // Référent = l'URL *résolue* du parent — la résolution
            // `./x` depuis un module relatif chaîne les chemins
            // bruts au lieu de repartir de la racine du site.
            let (dep_url, dep_src) = self.load_source(&dep_spec, url)?;
            let dep_idx = self.instantiate(&dep_spec, &dep_url, &dep_src, depth + 1)?;
            self.modules[idx].deps.push(dep_idx);
        }

        // 3. Wire the import bindings against instantiated deps —
        //    a cycle-back dep resolves through its static table.
        for s in &prog.body {
            let Stmt::Import { binds, source } = s else {
                continue;
            };
            let dep_idx = self.module_specs[source];
            for b in binds {
                match b {
                    ImportBind::Default(local) => {
                        let (denv, bname) = self.resolve_export(dep_idx, "default", source)?;
                        env_alias(&env, local, &denv, &bname);
                    }
                    ImportBind::Named(imported, local) => {
                        let (denv, bname) = self.resolve_export(dep_idx, imported, source)?;
                        env_alias(&env, local, &denv, &bname);
                    }
                    ImportBind::Namespace(local) => {
                        let ns = self.module_ns_object(dep_idx);
                        Self::env_put(&env, local, ns, false);
                    }
                }
            }
        }

        // 4. Hoist `var`/functions, then declare top-level lexical
        //    bindings (TDZ approximé : `undefined` jusqu'à l'exécution).
        self.hoist(&prog.body, &env);
        for s in &prog.body {
            for (name, mutable) in lex_names(s) {
                Self::env_declare_lex(&env, &name, mutable);
            }
        }
        // `export default` reserves its hidden slot.
        for s in &prog.body {
            if matches!(s, Stmt::Export(ExportDecl::Default(_))) {
                Self::env_put(&env, "default", Value::Undefined, true);
            }
        }

        // 5. Resolve pending re-exports — deps are fully
        //    instantiated now (a cycle-back dep answers with its
        //    static table, documented approximation).
        let pending = std::mem::take(&mut self.modules[idx].pending);
        for p in pending {
            match p {
                PendingExport::Named { list, from } => {
                    let dep_idx = self.module_specs[&from];
                    for (local, exported) in list {
                        let (denv, bname) = self.resolve_export(dep_idx, &local, &from)?;
                        self.modules[idx].exports.push((exported, denv, bname));
                    }
                }
                PendingExport::Star { ns, from } => {
                    let dep_idx = self.module_specs[&from];
                    if let Some(n) = ns {
                        // `export * as ns from 'm'` — the whole
                        // namespace under one local name.
                        let nsobj = self.module_ns_object(dep_idx);
                        Self::env_put(&env, &n, nsobj, false);
                        self.modules[idx].exports.push((n.clone(), env.clone(), n));
                    } else {
                        // `export * from 'm'` — copy every named
                        // export of the dep (not `default`, not
                        // names already exported explicitly).
                        let have: HashSet<String> = self.modules[idx]
                            .exports
                            .iter()
                            .map(|(n, _, _)| n.clone())
                            .collect();
                        let star_entries = self.modules[dep_idx].exports.clone();
                        for (name, denv, bnd) in star_entries {
                            if name == "default" || have.contains(&name) {
                                continue;
                            }
                            self.modules[idx].exports.push((name, denv, bnd));
                        }
                    }
                }
            }
        }
        Ok(idx)
    }

    /// `ResolveExport` — find `(defining env, binding name)` for
    /// `name` among `dep`'s exports; missing → `SyntaxError` (early
    /// link error, before any body runs).
    fn resolve_export(&mut self, dep: usize, name: &str, spec: &str) -> JsResult<(EnvRef, String)> {
        let hit = self.modules[dep]
            .exports
            .iter()
            .find(|(n, _, _)| n == name)
            .map(|(_, e, b)| (e.clone(), b.clone()));
        hit.ok_or_else(|| {
            let t = self.error_value(
                self.protos.syntax_error,
                "SyntaxError",
                &format!("module `{spec}` has no export `{name}`"),
            );
            Thrown(t)
        })
    }

    /// `InnerModuleEvaluation` — evaluate deps DFS post-order; a dep
    /// already `Evaluating` is a cycle edge (skipped, spec-like).
    fn evaluate_module(&mut self, idx: usize) -> JsResult<()> {
        match self.modules[idx].state {
            ModState::Evaluated | ModState::Evaluating => return Ok(()),
            ModState::Instantiated => {}
        }
        self.modules[idx].state = ModState::Evaluating;
        let deps = self.modules[idx].deps.clone();
        for dep in deps {
            self.evaluate_module(dep)?;
        }
        let env = self.modules[idx].env.clone();
        let body = std::mem::take(&mut self.modules[idx].body);
        let outer_strict = self.strict;
        self.strict = true; // modules are always strict
        let prev_mod = std::mem::replace(&mut self.current_module, self.modules[idx].spec.clone());
        // JS-C35 — TLA : un `await` au top-level → le corps
        // tourne sur `GExec` comme un `async fn` ; la promesse
        // porte le settlement (rejection → `run_microtasks`
        // la reporte au drain). Borne documentée : le graphe
        // d'importeurs n'attend pas — les exports lisent les
        // bindings lexicalement (vivants par `nsget`).
        if body.iter().any(crate::interp::lazy::stmt_has_suspend) {
            let lowered = crate::interp::lazy::lower_suspend_body(&body);
            let prom = self.new_promise();
            let gen =
                crate::interp::lazy::gen_lazy_obj(self, env.clone(), Value::Undefined, lowered);
            if let Value::Obj(gid) = gen {
                if let Some(o) = self.heap.obj(gid) {
                    if let ObjKind::GenLive { exec } = &o.kind {
                        let exec = exec.clone();
                        let mut g = exec.borrow_mut();
                        g.set_outer(prom);
                        let _ = g.start(self);
                    }
                }
            }
            self.current_module = prev_mod;
            self.strict = outer_strict;
            return Ok(());
        }
        let r = self.exec_body(&env, Value::Undefined, &body);
        self.current_module = prev_mod;
        self.strict = outer_strict;
        self.modules[idx].state = ModState::Evaluated;
        r.map(|_| ())
    }

    /// The module namespace exotic object — own props are
    /// getter-only accessors resolving the export live at each read
    /// (spec : `[[]]` namespace, `[[Prototype]]: null`).
    fn module_ns_object(&mut self, idx: usize) -> Value {
        let obj = self.heap.alloc_obj(Object::ordinary(Value::Null));
        let exports = self.modules[idx].exports.clone();
        for (exported, denv, bnd) in exports {
            let eidx = self.env_index(&denv);
            let bname = self.str_val(bnd);
            let get = self.bound_native("nsget", ns_get, vec![Value::Num(eidx as f64), bname]);
            if let Some(o) = self.heap.obj_mut(obj) {
                o.define_accessor(
                    &exported,
                    get,
                    Value::Undefined,
                    PropAttrs {
                        writable: false,
                        enumerable: true,
                        configurable: false,
                    },
                );
            }
        }
        Value::Obj(obj)
    }

    /// Registry index of `env` for bound namespace getters (they can
    /// only carry `Value`s — the index keeps the lookup stable).
    fn env_index(&mut self, env: &EnvRef) -> usize {
        let ptr = Rc::as_ptr(env) as usize;
        if let Some(i) = self
            .module_envs
            .iter()
            .position(|e| Rc::as_ptr(e) as usize == ptr)
        {
            return i;
        }
        self.module_envs.push(env.clone());
        self.module_envs.len() - 1
    }
}

/// Traduction d'un spécificateur bare via une import map —
/// correspondance exacte puis préfixe le plus long dont la clé
/// finit par `/` (spec « resolve a module specifier » : le
/// préfixe consommé est remplacé, le reste du spécificateur
/// est conservé). Fonction pure partagée : le prefetch de la
/// session applique la même règle que `Engine::resolve_bare`
/// à l'évaluation — sinon la map serait honorée à l'eval mais
/// ignorée du téléchargement (issue #6).
#[must_use]
pub fn rewrite_bare_specifier(map: &[(String, String)], spec: &str) -> Option<String> {
    if !is_bare_specifier(spec) {
        return None;
    }
    let mut best: Option<&(String, String)> = None;
    for entry in map {
        let (key, _) = &entry;
        if (key == spec || (key.ends_with('/') && spec.starts_with(key.as_str())))
            && best.is_none_or(|(bk, _)| key.len() > bk.len())
        {
            best = Some(entry);
        }
    }
    best.map(|(key, t)| format!("{t}{}", &spec[key.len()..]))
}

/// Spécificateur « bare » au sens de la spec import map : ni
/// URL absolue (`scheme:` — `https:`, `data:`, `node:`…), ni
/// chemin `/…`, `./…`, `../…`. Seuls les bare passent par la map.
/// Publique : la session refuse au prefetch/resolve un bare
/// resté hors map (issue #6 — un bare non mappé est
/// « unresolvable », jamais un chemin relatif déguisé).
#[must_use]
pub fn is_bare_specifier(spec: &str) -> bool {
    if spec.starts_with('/') || spec.starts_with("./") || spec.starts_with("../") {
        return false;
    }
    let Some(colon) = spec.find(':') else {
        return true;
    };
    // Un scheme valide → URL-like, jamais bare (`react:dom` inclus,
    // conforme à l'ordre « URL d'abord » de la spec).
    !spec[..colon].bytes().enumerate().all(|(i, b)| {
        if i == 0 {
            b.is_ascii_alphabetic()
        } else {
            b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'.')
        }
    })
}

/// Insert an `import` alias cell — `local` reads (and refuses
/// writes) through `binding` in `dep_env`, live.
fn env_alias(env: &EnvRef, local: &str, dep_env: &EnvRef, binding: &str) {
    let mut b = env.borrow_mut();
    match &mut b.kind {
        crate::heap::EnvKind::Local(m)
        | crate::heap::EnvKind::Global(m)
        | crate::heap::EnvKind::Block(m) => {
            m.insert(
                local.to_string(),
                EnvCell::Alias(dep_env.clone(), binding.to_string()),
            );
        }
        // Env de module — jamais un `with`.
        crate::heap::EnvKind::With(_) => {}
    }
}

/// Namespace getter — bound args: `[env_index, binding_name]`.
fn ns_get(e: &mut Engine, _this: Value, args: &[Value]) -> JsResult<Value> {
    let Some(Value::Num(n)) = args.first() else {
        return Ok(Value::Undefined);
    };
    let Some(name) = args.get(1).and_then(|v| match v {
        Value::Str(id) => e.heap.string(*id),
        _ => None,
    }) else {
        return Ok(Value::Undefined);
    };
    let Some(env) = e.module_envs.get(*n as usize).cloned() else {
        return Ok(Value::Undefined);
    };
    Ok(e.env_get(&env, &name).unwrap_or(Value::Undefined))
}

/// Names declared by a top-level statement for lexical
/// instantiation — `let`/`const`/`class`, plus the inner decl of
/// `export <decl>` — `(name, mutable)`.
fn lex_names(s: &Stmt) -> Vec<(String, bool)> {
    match s {
        Stmt::Lexical { decls, constant } => {
            let mut names = Vec::new();
            for d in decls {
                pat_names(&d.pat, &mut names);
            }
            let mutable = !*constant;
            names.into_iter().map(|n| (n, mutable)).collect()
        }
        Stmt::ClassDecl(c) => c.name.iter().map(|n| (n.clone(), true)).collect(),
        Stmt::Export(ExportDecl::Decl(inner)) => lex_names(inner),
        _ => Vec::new(),
    }
}

/// Bound names of a declaration for the export table — `var`/`let`/
/// `const` pats, `function`/`class` names.
fn decl_names(s: &Stmt) -> Vec<String> {
    let mut out = Vec::new();
    match s {
        Stmt::Var(decls) | Stmt::Lexical { decls, .. } => {
            for d in decls {
                pat_names(&d.pat, &mut out);
            }
        }
        Stmt::FunctionDecl(f) => {
            if let Some(n) = &f.name {
                out.push(n.clone());
            }
        }
        Stmt::ClassDecl(c) => {
            if let Some(n) = &c.name {
                out.push(n.clone());
            }
        }
        _ => {}
    }
    out
}

/// Collect `Ident` names inside a pattern (destructuring binds all).
fn pat_names(p: &Pat, out: &mut Vec<String>) {
    match p {
        Pat::Ident(n) => out.push(n.clone()),
        Pat::Rest(p) | Pat::Default(p, _) => pat_names(p, out),
        Pat::Array(elems) => {
            for e in elems.iter().flatten() {
                pat_names(e, out);
            }
        }
        Pat::Object { props, rest } => {
            for (_, p) in props {
                pat_names(p, out);
            }
            if let Some(r) = rest {
                pat_names(r, out);
            }
        }
        Pat::Lvalue(_) => {}
    }
}

/// `import()` en attente — miss du chargeur différé au fetch
/// session (JS-C100) : `resolve` a produit l'URL canonique,
/// la promesse reste pending jusqu'à `deliver_import`.
pub(crate) struct PendingImport {
    /// Spécificateur canonique (post import-map) — clé
    /// `module_specs`.
    pub spec: String,
    /// URL absolue résolue — `import.meta.url` et référent
    /// des imports du module.
    pub url: String,
    /// Promesse `import()` à résoudre/rejeter à la livraison.
    pub promise: crate::ObjId,
}

/// Requête de fetch `import()` drainée par la session
/// (JS-C100) — même motif `DynScriptReq` : l'URL est déjà
/// résolue (host policy `ModuleLoader::resolve`), la session
/// fetch et rappelle `deliver_import(seq, Ok(src)/Err(msg))`.
pub struct DynImportReq {
    /// Identifiant — répercuté à `deliver_import`.
    pub seq: u64,
    /// URL absolue à fetcher (module ou fichier JS nu).
    pub url: String,
    /// Contexte émetteur — la livraison vise son miroir.
    pub ctx: crate::context::CtxId,
}
