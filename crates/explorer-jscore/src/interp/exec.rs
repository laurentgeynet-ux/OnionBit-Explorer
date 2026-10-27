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
    pub(crate) fn eval_source(&mut self, src: &str) -> JsResult<Value> {
        // `eval` direct : le texte hérite du strict appelant —
        // early errors (`01`, `delete x`) et effets runtime.
        if let Some(h) = &self.eval_hook {
            h(src);
        }
        let prog = crate::parser::parse_in(
            src,
            self.strict,
            self.nest_limit,
            Some(std::rc::Rc::from("<eval>")),
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
        if let Some(h) = &self.eval_hook {
            h(src);
        }
        let prog = crate::parser::parse_in(
            src,
            self.strict,
            self.nest_limit,
            Some(std::rc::Rc::from("<eval>")),
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
            if std::env::var("DBG_DRIVE").is_ok() {
                eprintln!("[exec_body] stmt={:?}", std::mem::discriminant(s));
            }
            if self.ops_total % 256 == 0 && std::env::var("DBG_DRIVE").is_ok() {
                let st = self.heap.stats();
                let txt = format!("{s:?}");
                let short: String = txt.chars().take(200).collect();
                eprintln!("[stmt] {short}");
                let frames: Vec<String> = self
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
        // JS-C41 : `if`/`try`/`for` s'exécutent en récursion
        // native — segment tas de secours avant débordement.
        stacker::maybe_grow(
            crate::parser::STACK_RED_ZONE,
            crate::parser::STACK_GROW,
            || self.exec_stmt_inner(env, this, s),
        )
    }

    pub(crate) fn exec_stmt_inner(
        &mut self,
        env: &EnvRef,
        this: Value,
        s: &Stmt,
    ) -> JsResult<Flow> {
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
                            self.infer_name(&d.pat, v);
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
                        self.infer_name(&d.pat, v);
                        self.bind_pat(env, this, &d.pat, v, BindMode::Lex { mutable: !constant })?;
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
                self.exec_body(&benv, this, stmts)
            }
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
                // `for (let i = …)` — env de tête dédiée : les noms
                // lexicaux ne fuient pas dans le scope englobant et
                // chaque itération en reçoit une copie fraîche
                // (spec `CreatePerIterationEnvironment` — la copie
                // tombe ENTRE le corps et `update` : `i++` mute la
                // cellule du tour suivant, la closure capturée au
                // tour k garde `i = k`). `ienv` = env du tour
                // courant ; `names` = liaisons à recopier.
                let mut names: Vec<(String, bool)> = Vec::new();
                let mut ienv = env.clone();
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
                            let head = Env::block(env.clone());
                            for d in decls {
                                Self::lex_pat_names(&d.pat, !constant, &mut names);
                                Self::declare_pat_lex(&head, &d.pat, !constant);
                                if let Some(i) = &d.init {
                                    let v = self.eval_expr(&head, this, i)?;
                                    self.bind_pat(
                                        &head,
                                        this,
                                        &d.pat,
                                        v,
                                        BindMode::Lex { mutable: !constant },
                                    )?;
                                }
                            }
                            ienv = head;
                        }
                        ForInit::Expr(e) => {
                            self.eval_expr(env, this, e)?;
                        }
                    }
                }
                loop {
                    self.tick()?;
                    if let Some(t) = test {
                        let c = self.eval_expr(&ienv, this, t)?;
                        if !self.to_boolean(c) {
                            return Ok(Flow::Normal);
                        }
                    }
                    let f = self.exec_stmt(&ienv, this, body)?;
                    if let Some(f) = Self::loop_flow(f, &labels) {
                        return Ok(f);
                    }
                    if let Some(u) = update {
                        if names.is_empty() {
                            self.eval_expr(&ienv, this, u)?;
                        } else {
                            // Copie post-corps : le tour suivant
                            // reçoit des cellules fraîches portant
                            // les valeurs courantes, puis `update`
                            // les mute (spec).
                            let next = Env::block(env.clone());
                            for (n, m) in &names {
                                let v = self.env_get(&ienv, n).unwrap_or(Value::Undefined);
                                Self::env_declare_lex(&next, n, *m);
                                Self::env_init_lex(&next, n, v);
                            }
                            self.eval_expr(&next, this, u)?;
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
                        ienv = next;
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
                // `for (let k in o)` — env fraîche par itération
                // (bindings block-scope + capture de closure).
                let per_iter =
                    matches!(target, ForTarget::Lexical { .. } | ForTarget::Using { .. });
                for key in self.enum_keys(id)? {
                    let kv = self.str_val(key);
                    let ienv = if per_iter {
                        Env::block(env.clone())
                    } else {
                        env.clone()
                    };
                    self.assign_target(&ienv, this, target, kv)?;
                    let f = self.exec_stmt(&ienv, this, body)?;
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
                // la ressource est dépilée à chaque tour). Cibles
                // `let`/`const`/`using` : env fraîche par tour —
                // la liaison n'écrase pas le scope englobant et
                // chaque closure capture sa cellule.
                let using = matches!(target, ForTarget::Using { .. });
                let lex = matches!(target, ForTarget::Lexical { .. } | ForTarget::Using { .. });
                for item in items {
                    if using {
                        self.using_stack.push(Vec::new());
                    }
                    let ienv = if lex {
                        Env::block(env.clone())
                    } else {
                        env.clone()
                    };
                    let mut r = self
                        .assign_target(&ienv, this, target, item)
                        .and_then(|()| self.exec_stmt(&ienv, this, body));
                    if using {
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
                // `try {}`/`finally {}` sont des blocs — env enfant
                // pour leurs `let`/`const` (même règle que Block).
                let benv = Env::block(env.clone());
                let mut r = self.exec_body(&benv, this, block);
                if let Err(Thrown(v)) = r {
                    if let Some((pat, cbody)) = catch {
                        let cenv = Env::block(env.clone());
                        // Param en liaison lexicale : `bind_pat Var`
                        // traverserait le bloc vers le scope fonction.
                        if let Err(e) =
                            self.bind_pat(&cenv, this, pat, v, BindMode::Lex { mutable: true })
                        {
                            r = Err(e);
                        } else {
                            r = self.exec_body(&cenv, this, cbody);
                        }
                    }
                }
                if let Some(fbody) = finally {
                    let fenv = Env::block(env.clone());
                    let fr = self.exec_body(&fenv, this, fbody)?;
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
                // Tous les `case` d'un `switch` partagent UN seul
                // bloc déclaratif (spec — `let` dans un case est
                // visible des suivants, jamais du scope englobant).
                let senv = Env::block(env.clone());
                for c in &cases[start..] {
                    match self.exec_body(&senv, this, &c.body)? {
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
    pub(crate) fn loop_flow(f: Flow, labels: &[String]) -> Option<Flow> {
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
                self.bind_pat(env, this, pat, v, BindMode::Lex { mutable: false })?;
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
            .and_then(|n| self.heap_string_of(n))
            .unwrap_or_default();
        if cur.is_empty() && !name.is_empty() {
            let n = self.str_val(name.clone());
            self.set_own(id, "name", n, false);
        }
    }
}
