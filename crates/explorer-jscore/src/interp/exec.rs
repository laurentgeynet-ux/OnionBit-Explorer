// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

#[allow(clippy::wildcard_imports)]
use super::*;
use crate::ast::{ExportDecl, Expr, ForInit, ForTarget, Program, Stmt};
use crate::heap::{Env, EnvRef, JsResult, Thrown};
use crate::value::Value;

impl Engine {
    // -- statement execution -----------------------------------------------

    /// Parse + run `src` in the **global** scope, returning the
    /// thrown `Value` intact (unlike [`Engine::eval`] which formats
    /// it). Used by the `eval` builtin, `Function()`, and string
    /// timer callbacks — all global-scope (indirect `eval`
    /// semantics; direct-eval scoping is a documented deviation).
    /// Microtasks are *not* drained here — the enclosing job does.
    /// Plafond `eval`/`Function` — tests et harnais bornés.
    pub fn set_eval_src_cap(&mut self, cap: usize) {
        self.eval_src_cap = cap;
    }

    /// Kill doux RSS — tests et harnais bornés. `0` désarme et
    /// relâche le loquet (le moteur redevient utilisable — sinon
    /// un kill déjà tiré laisserait un moteur mort).
    pub fn set_rss_kill_mb(&mut self, mb: usize) {
        self.rss_kill_mb = mb;
        if mb == 0 {
            self.rss_kill_latched = false;
        }
    }

    /// Borne §B-59 bras 3 — chaque `eval`/`Function` retient un
    /// AST `Rc<FnLit>` (masse native hors `heap.bytes`, mesuré
    /// ×41 RSS/compté). Cumul des octets source ; au-delà du
    /// plafond → `RangeError` nommé, l'AST n'est jamais parsé.
    pub(crate) fn note_eval_src(&mut self, src: &str) -> JsResult<()> {
        self.eval_src_bytes = self.eval_src_bytes.saturating_add(src.len());
        if self.eval_src_bytes > self.eval_src_cap {
            let msg = format!(
                "eval source budget exceeded ({} B cumulative > {} B cap — AST retention bound)",
                self.eval_src_bytes, self.eval_src_cap
            );
            return Err(Thrown(self.error_value(
                self.protos.range_error,
                "RangeError",
                &msg,
            )));
        }
        Ok(())
    }

    pub(crate) fn eval_source(&mut self, src: &str) -> JsResult<Value> {
        // `eval` direct : le texte hérite du strict appelant —
        // early errors (`01`, `delete x`) et effets runtime.
        if std::env::var_os("EXPLORER_CTX_PROBE").is_some() {
            let wid = match self.get_prop(self.global, "window") {
                Some(Value::Obj(o)) => format!("{o:?}"),
                _ => "-".to_string(),
            };
            eprintln!(
                "[ctxp] kind=eval ctx={} global={:?} window={} bytes={}",
                self.cur_ctx,
                self.global,
                wid,
                src.len()
            );
        }
        if let Some(h) = &self.eval_hook {
            h(src);
        }
        Self::dump_eval_source(src);
        self.note_eval_src(src)?;
        let prog = crate::parser::parse_in(
            src,
            self.strict,
            self.nest_limit,
            Some(std::rc::Rc::from(Self::eval_src_tag(src))),
        )
        .map_err(|err| {
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

    pub(crate) fn exec_program_inner(&mut self, prog: &Program) -> JsResult<Value> {
        let env = self.global_env.clone();
        let this = Value::Obj(self.global);
        let r = self.exec_program_in(&env, this.clone(), prog);
        let Err(t) = r else {
            return r;
        };
        if !self.is_cjs_ref_error(&t) {
            return Err(t);
        }
        // §B-105 / JS-C448 — sandbox CommonJS : le script
        // référence `module`/`exports`/`require` à nu.
        // Re-exécution dans un env enfant semé des globals
        // CJS — les scripts sains (gardes `typeof`) ne
        // passent jamais ici : `typeof module` reste
        // `"undefined"` pour eux (debounce/chart.umd
        // intacts). Une seule tentative.
        self.sched_tick("cjs.sandbox");
        let (sandbox, module) = self.cjs_sandbox_env(&env);
        let r2 = self.exec_program_in(&sandbox, this, prog);
        if r2.is_ok() {
            if let Some(exports) = self.get_prop(module, "exports") {
                self.cjs_register_exports(exports);
            }
        }
        r2
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
        if let Some(h) = &self.eval_hook {
            h(src);
        }
        Self::dump_eval_source(src);
        self.note_eval_src(src)?;
        let prog = crate::parser::parse_in(
            src,
            self.strict,
            self.nest_limit,
            Some(std::rc::Rc::from(Self::eval_src_tag(src))),
        )
        .map_err(|err| {
            Thrown(self.error_value(self.protos.syntax_error, "SyntaxError", &err.to_string()))
        })?;
        let outer_strict = self.strict;
        self.strict |= prog.strict;
        let r = self.exec_program_in(env, this, &prog);
        self.strict = outer_strict;
        r
    }

    /// N1.5 — le bytecode VM BotGuard est livré par `eval()`
    /// (src="<eval>", invisible aux sondes par-src). Si la source
    /// mentionne `trayride`, on la marque du src de l'interpréteur
    /// → `bg.call`/`bg.listen`/frames la voient.
    fn eval_src_tag(src: &str) -> &'static str {
        if src.contains("trayride") {
            "<eval:google.com/js/th>"
        } else {
            "<eval>"
        }
    }

    /// Dump des sources évaluées sous `EXPLORER_EVAL_DUMP`
    /// (diagnostic §B-59/J11 : nommer le blob `eval()` en
    /// boucle — `YouTube` hydrate via un gros payload évalué).
    /// Un fichier `_evaldump_NNN.js` par source, cwd du daemon.
    fn dump_eval_source(src: &str) {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        if std::env::var_os("EXPLORER_EVAL_DUMP").is_none() {
            return;
        }
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = format!("_evaldump_{n:03}.js");
        let _ = std::fs::write(&path, src);
        eprintln!("[eval-dump] {path} bytes={}", src.len());
    }

    /// Corps de programme dans un couple env/`this` arbitraires —
    /// partagé entre `exec_program_inner` (global) et
    /// `eval_source_in` (direct eval, env appelant).
    pub(crate) fn exec_program_in(
        &mut self,
        env: &EnvRef,
        this: Value,
        prog: &Program,
    ) -> JsResult<Value> {
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
            // JS-C421 : frontière de statements — balayage GC
            // licite sous pression (env/this/racines semés).
            self.safepoint_gc(env, &this);
            match self.exec_stmt(env, this.clone(), s) {
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
            Ok(_) => Ok(self.completion.clone()),
            Err(e) => Err(e),
        }
    }

    pub(crate) fn exec_body(&mut self, env: &EnvRef, this: Value, body: &[Stmt]) -> JsResult<Flow> {
        // V-7 : temps cumulé de dispatch de corps (inclusif —
        // les appels imbriqués cumulent dedans).
        let _t = crate::j17_scope(&crate::J17_BODY_NS);
        if std::env::var("DBG_DRIVE").is_ok() {
            eprintln!(
                "[exec_body] call body={} stmts depth={} cur_mod={}",
                body.len(),
                self.call_depth,
                self.current_module
            );
        }
        // `using` (ES2026) : chaque décl pousse un disposeur sur la
        // frame courante, dépilée LIFO à TOUTE sortie du corps —
        // normal, break, return et throw (spec `DisposeResources`).
        // §B-59 : le corps est atteint — le span fatal d'ici
        // est couvert par `last_stmt` ; `last_phase` resterait
        // « call » seulement si la mort précédait ce point.
        if let Ok(mut g) = crate::LAST_PHASE.lock() {
            *g = "body";
        }
        let has_using = body.iter().any(|s| matches!(s, Stmt::Using { .. }));
        if has_using {
            self.using_stack.push(Vec::new());
        }
        let mut r = Ok(Flow::Normal);
        for s in body {
            // JS-C421 : frontière de statements — balayage GC
            // licite sous pression (env/this/racines semés).
            self.safepoint_gc(env, &this);
            if std::env::var("DBG_DRIVE").is_ok() {
                eprintln!("[exec_body] stmt={:?}", std::mem::discriminant(s));
            }
            if self.ops_total % 256 == 0 && std::env::var("DBG_DRIVE").is_ok() {
                let st = self.heap.stats();
                let txt = format!("{s:?}");
                let short: String = txt.chars().take(200).collect();
                eprintln!("[stmt] {short}");
                let frames: Vec<Rc<str>> = self
                    .call_stack
                    .iter()
                    .rev()
                    .take(6)
                    .map(|f| f.name.clone().unwrap_or_else(|| "?".into()))
                    .collect();
                eprintln!("[stack] {}", frames.join(" <- "));
                eprintln!(
                    "[ops] total={} left={} live={} bytes={}M gc={} depth={}",
                    self.ops_total,
                    self.ops_left,
                    st.live_cells,
                    st.estimated_bytes / 1_048_576,
                    st.gc_runs,
                    self.call_depth
                );
            }
            match self.exec_stmt(env, this.clone(), s) {
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
    pub(crate) fn drain_using(&mut self, mut r: JsResult<Flow>) -> JsResult<Flow> {
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
    pub(crate) fn register_using(&mut self, v: Value, is_await: bool) -> JsResult<()> {
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
                    Some(f) if self.is_callable(f.clone()) => {
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
        // Breadcrumb §B-59 : nomme le statement encadrant le span
        // fatal (lu par `rss_guard` avec `last_expr`/`last_nat`).
        if self.rss_watch_mb > 0 {
            if let Ok(mut g) = crate::LAST_STMT.lock() {
                *g = stmt_tag(s);
            }
        }
        self.tick()?;
        // JS-C421 : `this` + `env` restent vivants tant que cette
        // frame court — racines safepoint pour les corps en
        // dessous (la chaîne parente de la callee ne remonte pas
        // à l'env appelant). JS-C460 : borne d'âge **par frame** —
        // ce statement contribue son seq d'entrée ; les bras
        // dispatcheurs (boucles, blocs, try…) la masquent pendant
        // la sous-exécution et la re-stampent à la reprise, les
        // morts des enfants terminés redeviennent balayables.
        self.safepoint_roots.push(this.clone());
        self.safepoint_envs.push(env.clone());
        self.young_stack.push(self.heap.alloc_seq());
        // JS-C41 : `if`/`try`/`for` s'exécutent en récursion
        // native — segment tas de secours avant débordement.
        let r = stacker::maybe_grow(
            crate::parser::STACK_RED_ZONE,
            crate::parser::STACK_GROW,
            || self.exec_stmt_inner(env, this, s),
        );
        self.young_stack.pop();
        self.safepoint_roots.pop();
        self.safepoint_envs.pop();
        r
    }

    pub(crate) fn exec_stmt_inner(
        &mut self,
        env: &EnvRef,
        this: Value,
        s: &Stmt,
    ) -> JsResult<Flow> {
        // JS-C460 — index de la borne d'âge de CE frame (empilée
        // par `exec_stmt` juste avant) : les bras dispatcheurs la
        // masquent pendant la sous-exécution de l'enfant
        // (`u64::MAX` — tout ce que le frame tient est raciné) et
        // la re-stampent à la reprise pour leurs propres temps.
        let yslot = self.young_stack.len() - 1;
        match s {
            // `import …` — liaisons câblées à l'instanciation ;
            // l'exécution n'a rien à faire (idem `;`).
            Stmt::Empty | Stmt::Import { .. } => Ok(Flow::Normal),
            Stmt::Export(d) => match d {
                // `export <decl>` — la déclaration s'exécute
                // normalement (les noms sont déjà exportés).
                ExportDecl::Decl(inner) => {
                    self.young_stack[yslot] = u64::MAX;
                    let r = self.exec_stmt(env, this, inner);
                    self.young_stack[yslot] = self.heap.alloc_seq();
                    r
                }
                // `export default <expr|function|class>` — lie le
                // slot `default` (déclaré à l'instanciation) ; une
                // fonction/classe nommée lie aussi son propre nom.
                ExportDecl::Default(e) => {
                    let v = self.eval_expr(env, this, e)?;
                    self.env_set(env, "default", v.clone())?;
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
                // JS-C460 — un statement-expression qui EST un
                // appel ne tient rien pendant le dispatch.
                self.arm_call_only(yslot, e);
                let v = self.eval_expr(env, this, e);
                self.call_only_slot = None;
                self.completion = v?;
                Ok(Flow::Normal)
            }
            Stmt::Var(decls) => {
                for d in decls {
                    match &d.init {
                        Some(init) => {
                            self.arm_call_only(yslot, init);
                            let v = self.eval_expr(env, this.clone(), init);
                            self.call_only_slot = None;
                            let v = v?;
                            self.infer_name(&d.pat, v.clone());
                            self.bind_pat(env, this.clone(), &d.pat, v, BindMode::Var)?;
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
                        self.arm_call_only(yslot, init);
                        let v = self.eval_expr(env, this.clone(), init);
                        self.call_only_slot = None;
                        let v = v?;
                        self.infer_name(&d.pat, v.clone());
                        self.bind_pat(
                            env,
                            this.clone(),
                            &d.pat,
                            v,
                            BindMode::Lex { mutable: !constant },
                        )?;
                    }
                }
                Ok(Flow::Normal)
            }
            // Function decls were installed by hoisting; re-installing
            // keeps block-level decls working (Annex-B-ish tolerance).
            // Ne pas ré-instancier quand le binding tient déjà la
            // fonction de CETTE déclaration : le pattern babel/webpack
            // `e.registerId=…; function e(){}` (IIFE styled-components,
            // vaadin terrain) posait ses statiques sur l'objet hissé —
            // un nouvel objet les perdait (`th.registerId` undefined).
            // Décl de bloc : binding `undefined` → installe toujours ;
            // ré-exécution en boucle : lit différent → nouvelle closure.
            Stmt::FunctionDecl(f) => {
                if let Some(name) = &f.name {
                    let hoisted = matches!(self.env_get(env, name), Some(Value::Obj(id)) if {
                        matches!(
                            self.heap.obj(id).map(|o| &o.kind),
                            Some(ObjKind::Fun(Fun::Script { lit, .. }))
                                if Rc::ptr_eq(lit, f)
                        )
                    });
                    if !hoisted {
                        let fv = self.make_function(f, env, Value::Undefined);
                        self.env_set(env, name, fv)?;
                    }
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
            // `{ let x … }` — portée de bloc réelle : les `let`/
            // `const`/`class` du corps vivent dans une env enfant
            // (ils ne doivent pas écraser les liaisons du scope
            // englobant — régression réelle : catalyst `for(let
            // [t,n]of e)` clobérait le paramètre `t` de sa fonction,
            // puis `t[O]` lisait une clé symbole sur `undefined`).
            // `var`/fonctions restent fonction-scope via `env_set`
            // (résolution par la chaîne parente, `hoist_vars_only`
            // les a déjà déclarés dans l'env fonction).
            Stmt::Block(stmts) => {
                let benv = Env::block(env.clone());
                // JS-C460 — dispatch pur : le frame du bloc ne
                // tient rien pendant les sous-statements.
                self.young_stack[yslot] = u64::MAX;
                let r = self.exec_body(&benv, this, stmts);
                self.young_stack[yslot] = self.heap.alloc_seq();
                r
            }
            // `using x = e` (ES2026) — binding const + disposeur
            // enregistré sur la frame du corps courant.
            Stmt::Using { decls, is_await } => {
                for d in decls {
                    let v = match &d.init {
                        Some(i) => self.eval_expr(env, this.clone(), i)?,
                        None => Value::Undefined,
                    };
                    self.bind_pat(
                        env,
                        this.clone(),
                        &d.pat,
                        v.clone(),
                        BindMode::Lex { mutable: false },
                    )?;
                    self.register_using(v, *is_await)?;
                }
                Ok(Flow::Normal)
            }
            Stmt::If { cond, then, else_ } => {
                self.arm_call_only(yslot, cond);
                let c = self.eval_expr(env, this.clone(), cond);
                self.call_only_slot = None;
                let c = c?;
                // JS-C460 — dispatch pur des branches.
                self.young_stack[yslot] = u64::MAX;
                let r = if self.to_boolean(c) {
                    self.exec_stmt(env, this, then)
                } else if let Some(e) = else_ {
                    self.exec_stmt(env, this, e)
                } else {
                    Ok(Flow::Normal)
                };
                self.young_stack[yslot] = self.heap.alloc_seq();
                r
            }
            // `with (o) b` — les noms se résolvent d'abord dans les
            // props de `o` (sauf `@@unscopables`), puis le scope
            // englobant. Sloppy uniquement (early-error du parseur).
            Stmt::With { obj, body } => {
                let v = self.eval_expr(env, this.clone(), obj)?;
                let Some(oid) = self.with_object_of(v)? else {
                    return Err(self.type_error("cannot use `with` on `null` or `undefined`"));
                };
                let with_env = Env::with(env.clone(), oid);
                self.young_stack[yslot] = u64::MAX;
                let r = self.exec_stmt(&with_env, this, body);
                self.young_stack[yslot] = self.heap.alloc_seq();
                r
            }
            Stmt::While { cond, body } => {
                let labels = std::mem::take(&mut self.pending_labels);
                loop {
                    // JS-C460 — re-stamp à chaque tour : ce frame
                    // ne tient plus que des racines (env/this) —
                    // les morts des itérations passées redeviennent
                    // balayables au safepoint du corps.
                    self.young_stack[yslot] = self.heap.alloc_seq();
                    self.tick()?;
                    self.arm_call_only(yslot, cond);
                    let c = self.eval_expr(env, this.clone(), cond);
                    self.call_only_slot = None;
                    if !self.to_boolean(c?) {
                        return Ok(Flow::Normal);
                    }
                    self.young_stack[yslot] = u64::MAX;
                    let f = self.exec_stmt(env, this.clone(), body)?;
                    if let Some(f) = Self::loop_flow(f, &labels) {
                        return Ok(f);
                    }
                }
            }
            Stmt::DoWhile { body, cond } => {
                let labels = std::mem::take(&mut self.pending_labels);
                loop {
                    self.young_stack[yslot] = self.heap.alloc_seq();
                    self.tick()?;
                    self.young_stack[yslot] = u64::MAX;
                    let f = self.exec_stmt(env, this.clone(), body)?;
                    self.young_stack[yslot] = self.heap.alloc_seq();
                    if let Some(f) = Self::loop_flow(f, &labels) {
                        return Ok(f);
                    }
                    self.arm_call_only(yslot, cond);
                    let c = self.eval_expr(env, this.clone(), cond);
                    self.call_only_slot = None;
                    if !self.to_boolean(c?) {
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
                // `for (let i = …)` — env de tête dédiée : les noms
                // lexicaux ne fuient pas dans le scope englobant et
                // chaque itération en reçoit une copie fraîche
                // (spec `CreatePerIterationEnvironment` — la copie
                // tombe ENTRE le corps et `update` : `i++` mute la
                // cellule du tour suivant, la closure capturée au
                // tour k garde `i = k`). `ienv` = env du tour
                // courant ; `names` = liaisons à recopier.
                let mut names: Vec<(Rc<str>, bool)> = Vec::new();
                let mut ienv = env.clone();
                if let Some(init) = init {
                    match init {
                        ForInit::Var(decls) => {
                            for d in decls {
                                if let Some(i) = &d.init {
                                    let v = self.eval_expr(env, this.clone(), i)?;
                                    self.bind_pat(env, this.clone(), &d.pat, v, BindMode::Var)?;
                                }
                            }
                        }
                        ForInit::Lexical { decls, constant } => {
                            let head = Env::block(env.clone());
                            for d in decls {
                                Self::lex_pat_names(&d.pat, !constant, &mut names);
                                Self::declare_pat_lex(&head, &d.pat, !constant);
                                if let Some(i) = &d.init {
                                    let v = self.eval_expr(&head, this.clone(), i)?;
                                    self.bind_pat(
                                        &head,
                                        this.clone(),
                                        &d.pat,
                                        v,
                                        BindMode::Lex { mutable: !constant },
                                    )?;
                                }
                            }
                            ienv = head;
                        }
                        ForInit::Expr(e) => {
                            self.eval_expr(env, this.clone(), e)?;
                        }
                    }
                }
                // JS-C460 — l'env de tête (`ienv`) vit pendant
                // toute la boucle, y compris pendant les évals
                // test/update/copie où `safepoint_envs` ne la
                // couvre pas (poussée par le `exec_stmt` du corps
                // seulement) : racine indexée, re-seedée à chaque
                // copie d'itération.
                let sp_ienv = self.safepoint_envs.len();
                self.safepoint_envs.push(ienv.clone());
                let mut out: JsResult<Flow> = Ok(Flow::Normal);
                loop {
                    // JS-C460 — re-stamp à chaque tour : ce frame
                    // ne tient plus que des racines — les morts
                    // des itérations passées redeviennent
                    // balayables au safepoint du corps.
                    self.young_stack[yslot] = self.heap.alloc_seq();
                    if let Err(e) = self.tick() {
                        out = Err(e);
                        break;
                    }
                    if let Some(t) = test {
                        self.arm_call_only(yslot, t);
                        let c = self.eval_expr(&ienv, this.clone(), t);
                        self.call_only_slot = None;
                        match c {
                            Err(e) => {
                                out = Err(e);
                                break;
                            }
                            Ok(c) => {
                                if !self.to_boolean(c) {
                                    break;
                                }
                            }
                        }
                    }
                    self.young_stack[yslot] = u64::MAX;
                    match self.exec_stmt(&ienv, this.clone(), body) {
                        Err(e) => {
                            out = Err(e);
                            break;
                        }
                        Ok(f) => {
                            if let Some(f) = Self::loop_flow(f, &labels) {
                                out = Ok(f);
                                break;
                            }
                        }
                    }
                    self.young_stack[yslot] = self.heap.alloc_seq();
                    if let Some(u) = update {
                        if names.is_empty() {
                            self.arm_call_only(yslot, u);
                            let r = self.eval_expr(&ienv, this.clone(), u);
                            self.call_only_slot = None;
                            if let Err(e) = r {
                                out = Err(e);
                                break;
                            }
                        } else {
                            // Copie post-corps : le tour suivant
                            // reçoit des cellules fraîches portant
                            // les valeurs courantes, puis `update`
                            // les mute (spec). `next` racinée
                            // avant l'éval.
                            let next = Env::block(env.clone());
                            for (n, m) in &names {
                                let v = self.env_get(&ienv, n).unwrap_or(Value::Undefined);
                                Self::env_declare_lex(&next, n, *m);
                                Self::env_init_lex(&next, n, v);
                            }
                            self.safepoint_envs[sp_ienv] = next.clone();
                            self.arm_call_only(yslot, u);
                            let r = self.eval_expr(&next, this.clone(), u);
                            self.call_only_slot = None;
                            if let Err(e) = r {
                                out = Err(e);
                                break;
                            }
                            ienv = next;
                        }
                    } else if !names.is_empty() {
                        // Sans `update` : recopier quand même entre
                        // les tours (les closures restent distinctes).
                        let next = Env::block(env.clone());
                        for (n, m) in &names {
                            let v = self.env_get(&ienv, n).unwrap_or(Value::Undefined);
                            Self::env_declare_lex(&next, n, *m);
                            Self::env_init_lex(&next, n, v);
                        }
                        self.safepoint_envs[sp_ienv] = next.clone();
                        ienv = next;
                    }
                }
                self.safepoint_envs.truncate(sp_ienv);
                out
            }
            Stmt::ForIn {
                target,
                obj,
                var_init,
                body,
            } => {
                let labels = std::mem::take(&mut self.pending_labels);
                let ov = self.eval_expr(env, this.clone(), obj)?;
                if let Some(init) = var_init {
                    let v = self.eval_expr(env, this.clone(), init)?;
                    self.assign_target(env, this.clone(), target, v)?;
                }
                let Value::Obj(id) = ov else {
                    return Ok(Flow::Normal); // for-in over primitives: skip
                };
                // `for (let k in o)` — env fraîche par itération
                // (bindings block-scope + capture de closure).
                let per_iter =
                    matches!(target, ForTarget::Lexical { .. } | ForTarget::Using { .. });
                let keys = self.enum_keys(id)?;
                // JS-C421 : l'objet énuméré vit à travers tous
                // les corps d'itération — racine pour la boucle.
                self.safepoint_roots.push(Value::Obj(id));
                let mut out = Ok(Flow::Normal);
                for key in keys {
                    // JS-C460 — re-stamp par itération + env
                    // d'itération racinée (`assign_target` peut
                    // invoquer un setter → safepoint pendant le
                    // dispatch hors `exec_stmt`).
                    self.young_stack[yslot] = self.heap.alloc_seq();
                    let kv = self.str_val(key);
                    let ienv = if per_iter {
                        Env::block(env.clone())
                    } else {
                        env.clone()
                    };
                    self.safepoint_envs.push(ienv.clone());
                    self.young_stack[yslot] = u64::MAX;
                    let st = self
                        .assign_target(&ienv, this.clone(), target, kv)
                        .and_then(|()| self.exec_stmt(&ienv, this.clone(), body));
                    self.young_stack[yslot] = self.heap.alloc_seq();
                    self.safepoint_envs.pop();
                    match st {
                        Err(e) => {
                            out = Err(e);
                            break;
                        }
                        Ok(f) => {
                            if let Some(f) = Self::loop_flow(f, &labels) {
                                out = Ok(f);
                                break;
                            }
                        }
                    }
                }
                self.safepoint_roots.pop();
                out
            }
            Stmt::ForOf {
                target,
                iter,
                body,
                is_await,
            } => {
                let labels = std::mem::take(&mut self.pending_labels);
                let v = self.eval_expr(env, this.clone(), iter)?;
                // JS-C456 — boucle **paresseuse** pour les
                // itérateurs custom (`Symbol.iterator` callable)
                // : `iter.next()` par tour, pas de Vec
                // matérialisé — un `break`/`throw` ne consomme
                // pas la suite et `IteratorClose` (spec
                // §13.7.5.14) appelle `it.return()` (vol 722 —
                // l'eager avalait les effets de bord dans le
                // mauvais ordre). La voie `iter_values` reste
                // pour les valeurs sans itérateur (arrays/TA/
                // strings — observablement identiques).
                if !*is_await {
                    if let Some(it) = self.forof_iterator(v.clone())? {
                        let using = matches!(target, ForTarget::Using { .. });
                        let lex =
                            matches!(target, ForTarget::Lexical { .. } | ForTarget::Using { .. });
                        // JS-C460 — l'itérateur vit à travers tous
                        // les corps : racine pour la boucle (sous
                        // borne d'âge fine il n'est plus « jeune »
                        // après le premier tour).
                        self.safepoint_roots.push(it.clone());
                        let mut out = Ok(Flow::Normal);
                        loop {
                            self.young_stack[yslot] = self.heap.alloc_seq();
                            if let Err(e) = self.tick() {
                                out = Err(e);
                                break;
                            }
                            let Value::Obj(iid) = it else { break };
                            let Some(nf) = self.get_prop(iid, "next") else {
                                break;
                            };
                            if !self.is_callable(nf.clone()) {
                                break;
                            }
                            let r = match self.call_value(nf, it.clone(), &[]) {
                                Err(e) => {
                                    out = Err(e);
                                    break;
                                }
                                Ok(r) => r,
                            };
                            let Value::Obj(rid) = r else { break };
                            if self
                                .get_prop(rid, "done")
                                .is_some_and(|d| self.to_boolean(d))
                            {
                                break;
                            }
                            let item = self.get_prop(rid, "value").unwrap_or(Value::Undefined);
                            if using {
                                self.using_stack.push(Vec::new());
                            }
                            let ienv = if lex {
                                Env::block(env.clone())
                            } else {
                                env.clone()
                            };
                            self.safepoint_envs.push(ienv.clone());
                            self.young_stack[yslot] = u64::MAX;
                            let mut rr = self
                                .assign_target(&ienv, this.clone(), target, item)
                                .and_then(|()| self.exec_stmt(&ienv, this.clone(), body));
                            self.young_stack[yslot] = self.heap.alloc_seq();
                            self.safepoint_envs.pop();
                            if using {
                                rr = self.drain_using(rr);
                            }
                            match rr {
                                Err(e) => {
                                    if let Err(e2) = self.iterator_close(&it) {
                                        out = Err(e2);
                                        break;
                                    }
                                    out = Err(e);
                                    break;
                                }
                                Ok(f) => {
                                    if let Some(f) = Self::loop_flow(f, &labels) {
                                        // `Break` = consommé par
                                        // la boucle (Normal ici) —
                                        // `IteratorClose` sur
                                        // break aussi, pas sur
                                        // `done`/`continue`.
                                        if !matches!(f, Flow::Continue(_)) {
                                            if let Err(e) = self.iterator_close(&it) {
                                                out = Err(e);
                                                break;
                                            }
                                        }
                                        out = Ok(f);
                                        break;
                                    }
                                }
                            }
                        }
                        self.safepoint_roots.pop();
                        return out;
                    }
                }
                let items = if *is_await {
                    self.iter_values_await(v)?
                } else {
                    self.iter_values(v)?
                };
                // `for (using x of y)` : frame par itération (spec —
                // la ressource est dépilée à chaque tour). Cibles
                // `let`/`const`/`using` : env fraîche par tour —
                // la liaison n'écrase pas le scope englobant et
                // chaque closure capture sa cellule.
                let using = matches!(target, ForTarget::Using { .. });
                let lex = matches!(target, ForTarget::Lexical { .. } | ForTarget::Using { .. });
                // JS-C421 : les items non encore itérés vivent à
                // travers les corps — racines pour toute la boucle.
                let sr_mark = self.safepoint_roots.len();
                self.safepoint_roots.extend(items.iter().cloned());
                let mut out = Ok(Flow::Normal);
                for item in items {
                    // JS-C460 — re-stamp + env d'itération racinée
                    // (les `items` sont déjà racinés en bloc).
                    self.young_stack[yslot] = self.heap.alloc_seq();
                    if using {
                        self.using_stack.push(Vec::new());
                    }
                    let ienv = if lex {
                        Env::block(env.clone())
                    } else {
                        env.clone()
                    };
                    self.safepoint_envs.push(ienv.clone());
                    self.young_stack[yslot] = u64::MAX;
                    let mut r = self
                        .assign_target(&ienv, this.clone(), target, item)
                        .and_then(|()| self.exec_stmt(&ienv, this.clone(), body));
                    self.young_stack[yslot] = self.heap.alloc_seq();
                    self.safepoint_envs.pop();
                    if using {
                        r = self.drain_using(r);
                    }
                    match r {
                        Err(e) => {
                            out = Err(e);
                            break;
                        }
                        Ok(f) => {
                            if let Some(f) = Self::loop_flow(f, &labels) {
                                out = Ok(f);
                                break;
                            }
                        }
                    }
                }
                self.safepoint_roots.truncate(sr_mark);
                out
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
                self.young_stack[yslot] = u64::MAX;
                let r = self.exec_stmt(env, this, body);
                self.young_stack[yslot] = self.heap.alloc_seq();
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
                    Some(e) => {
                        self.arm_call_only(yslot, e);
                        let v = self.eval_expr(env, this, e);
                        self.call_only_slot = None;
                        v?
                    }
                    None => Value::Undefined,
                };
                Ok(Flow::Return(v))
            }
            Stmt::Throw(e) => {
                self.arm_call_only(yslot, e);
                let v = self.eval_expr(env, this, e);
                self.call_only_slot = None;
                Err(Thrown(v?))
            }
            Stmt::Try {
                block,
                catch,
                finally,
            } => {
                // `try {}`/`finally {}` sont des blocs — env enfant
                // pour leurs `let`/`const` (même règle que Block).
                let benv = Env::block(env.clone());
                // JS-C460 — dispatch pur des trois corps.
                self.young_stack[yslot] = u64::MAX;
                let mut r = self.exec_body(&benv, this.clone(), block);
                self.young_stack[yslot] = self.heap.alloc_seq();
                if let Err(Thrown(ref v)) = r {
                    if let Some((pat, cbody)) = catch {
                        let cenv = Env::block(env.clone());
                        // Param en liaison lexicale : `bind_pat Var`
                        // traverserait le bloc vers le scope fonction.
                        if let Err(e) = self.bind_pat(
                            &cenv,
                            this.clone(),
                            pat,
                            v.clone(),
                            BindMode::Lex { mutable: true },
                        ) {
                            r = Err(e);
                        } else {
                            // JS-C421 : le jeton vit dans `r`
                            // pendant le corps catch — racine.
                            self.safepoint_roots.push(v.clone());
                            self.young_stack[yslot] = u64::MAX;
                            r = self.exec_body(&cenv, this.clone(), cbody);
                            self.young_stack[yslot] = self.heap.alloc_seq();
                            self.safepoint_roots.pop();
                        }
                    }
                }
                if let Some(fbody) = finally {
                    // JS-C421 : `r` porte encore son jeton éventuel
                    // pendant le corps finally — racine temporaire.
                    // JS-C460 : la valeur d'un `return` en vol est
                    // un jeton au même titre (bornes fines).
                    let held = match &r {
                        Err(Thrown(v)) | Ok(Flow::Return(v)) => {
                            self.safepoint_roots.push(v.clone());
                            true
                        }
                        _ => false,
                    };
                    let fenv = Env::block(env.clone());
                    self.young_stack[yslot] = u64::MAX;
                    let fr = self.exec_body(&fenv, this, fbody);
                    self.young_stack[yslot] = self.heap.alloc_seq();
                    if held {
                        self.safepoint_roots.pop();
                    }
                    let fr = fr?;
                    if !matches!(fr, Flow::Normal) {
                        return Ok(fr);
                    }
                }
                r
            }
            Stmt::Switch { disc, cases } => {
                let dv = self.eval_expr(env, this.clone(), disc)?;
                let mut matched = None;
                let mut default = None;
                for (i, c) in cases.iter().enumerate() {
                    match &c.test {
                        Some(t) => {
                            let tv = self.eval_expr(env, this.clone(), t)?;
                            if strict_eq(&dv, &tv, &self.heap) {
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
                // Tous les `case` d'un `switch` partagent UN seul
                // bloc déclaratif (spec — `let` dans un case est
                // visible des suivants, jamais du scope englobant).
                let senv = Env::block(env.clone());
                // JS-C421 : `dv` vit à travers les corps de cases.
                self.safepoint_roots.push(dv);
                let mut out = Ok(Flow::Normal);
                for c in &cases[start..] {
                    // JS-C460 — dispatch pur d'un corps de case.
                    self.young_stack[yslot] = u64::MAX;
                    match self.exec_body(&senv, this.clone(), &c.body) {
                        Err(e) => {
                            out = Err(e);
                            break;
                        }
                        Ok(Flow::Normal) => {}
                        // `break` seul sort du switch ; `break lbl`
                        // remonte jusqu'au statement étiqueté.
                        Ok(Flow::Break(None)) => {
                            out = Ok(Flow::Normal);
                            break;
                        }
                        Ok(f) => {
                            out = Ok(f);
                            break;
                        }
                    }
                }
                self.safepoint_roots.pop();
                out
            }
        }
    }

    /// Consomme le `Flow` produit par un corps de boucle —
    /// `None` = itération suivante, `Some(f)` = quitter la boucle
    /// en propageant `f`. `labels` = labels de boucle captés via
    /// `pending_labels` (`continue lbl` ne les avale que si `lbl`
    /// y figure ; `break lbl` n'est jamais avalé par une boucle).
    /// Itérateur `v[Symbol.iterator]()` callable —
    /// `for..of` paresseux (JS-C456). `None` = pas
    /// d'itérateur custom → voie `iter_values` (arrays/TA/
    /// strings — observablement identique, plus rapide).
    fn forof_iterator(&mut self, v: Value) -> JsResult<Option<Value>> {
        let Value::Obj(id) = v else { return Ok(None) };
        let Some(m) = self.get_prop(id, SYM_ITERATOR) else {
            return Ok(None);
        };
        if !self.is_callable(m.clone()) {
            return Ok(None);
        }
        Ok(Some(self.call_value(m, v, &[])?))
    }

    /// `IteratorClose` (spec §13.7.5.14) — appelle
    /// `it.return()` si présent/callable ; erreurs de `next`
    /// déjà propagées ailleurs.
    fn iterator_close(&mut self, it: &Value) -> JsResult<()> {
        if let Value::Obj(iid) = it {
            if let Some(rf) = self.get_prop(*iid, "return") {
                if self.is_callable(rf.clone()) {
                    self.call_value(rf, it.clone(), &[])?;
                }
            }
        }
        Ok(())
    }

    pub(crate) fn loop_flow(f: Flow, labels: &[Rc<str>]) -> Option<Flow> {
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

    pub(crate) fn assign_target(
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
                self.bind_pat(env, this, pat, v.clone(), BindMode::Lex { mutable: false })?;
                self.register_using(v, *is_await)?;
            }
        }
        Ok(())
    }
}

impl Engine {
    /// `NamedEvaluation` spec — une fonction/classe anonyme liée à
    /// un identifiant reçoit son `name` (dw : `var C = class{}`
    /// → `C.name === "C"` ; sinon `customElements.define(C.name,…)`
    /// recevait `""` → `Illegal component name ""`).
    fn infer_name(&mut self, pat: &Pat, v: Value) {
        let Pat::Ident(name) = pat else {
            return;
        };
        let Value::Obj(id) = v else {
            return;
        };
        if !self.is_callable(v) {
            return;
        }
        // Déjà nommée (`function f(){}`, `class K{}`) — ne pas
        // écraser ; l'inférence ne vaut que pour `""`/absent.
        let cur = self
            .get_prop(id, "name")
            .as_ref()
            .and_then(Self::heap_string_of)
            .unwrap_or_default();
        if cur.is_empty() && !name.is_empty() {
            let n = self.str_val(&**name);
            self.set_own(id, "name", n, false);
        }
    }
}

/// Nom du variant `Stmt` — breadcrumb §B-59 (`LAST_STMT`),
/// zéro alloc, armé seulement sous `EXPLORER_JS_RSS_WATCH`.
fn stmt_tag(s: &Stmt) -> &'static str {
    match s {
        Stmt::Empty => "Empty",
        Stmt::Expr(_) => "Expr",
        Stmt::Var(_) => "Var",
        Stmt::Lexical { .. } => "Lexical",
        Stmt::FunctionDecl(_) => "FunctionDecl",
        Stmt::ClassDecl(_) => "ClassDecl",
        Stmt::Block(_) => "Block",
        Stmt::If { .. } => "If",
        Stmt::While { .. } => "While",
        Stmt::DoWhile { .. } => "DoWhile",
        Stmt::For { .. } => "For",
        Stmt::ForIn { .. } => "ForIn",
        Stmt::ForOf { .. } => "ForOf",
        Stmt::Using { .. } => "Using",
        Stmt::With { .. } => "With",
        Stmt::Labeled { .. } => "Labeled",
        Stmt::Break(_) => "Break",
        Stmt::Continue(_) => "Continue",
        Stmt::Return(_) => "Return",
        Stmt::Throw(_) => "Throw",
        Stmt::Try { .. } => "Try",
        Stmt::Switch { .. } => "Switch",
        Stmt::Import { .. } => "Import",
        Stmt::Export(_) => "Export",
    }
}
