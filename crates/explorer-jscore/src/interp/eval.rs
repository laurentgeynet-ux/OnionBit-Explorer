// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

#[allow(clippy::wildcard_imports)]
use super::*;
use crate::ast::{AssignOp, BinOp, ChainStep, Expr, ObjProp, UnOp};
use crate::heap::{Binding, Env, EnvCell, EnvKind, EnvRef, Fun, JsResult, ObjKind, Thrown};
use crate::value::{to_int32, to_uint32, Value};

impl Engine {
    pub(crate) fn eval_expr(&mut self, env: &EnvRef, this: Value, e: &Expr) -> JsResult<Value> {
        self.expr_depth += 1;
        if self.expr_depth > self.nest_limit {
            self.expr_depth -= 1;
            return Err(self.range_error("expression too deeply nested"));
        }
        // JS-C41 : la descente d'expressions imbriquées croît sur
        // un segment tas avant d'épuiser la pile native.
        let r = stacker::maybe_grow(
            crate::parser::STACK_RED_ZONE,
            crate::parser::STACK_GROW,
            || self.eval_inner(env, this, e),
        );
        self.expr_depth -= 1;
        r
    }

    #[allow(clippy::too_many_lines)]
    pub(crate) fn eval_inner(&mut self, env: &EnvRef, this: Value, e: &Expr) -> JsResult<Value> {
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
                        None => v.push(Value::Hole),
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
                                // diag sonde (mur TanStack `e.id`) :
                                // un clone sur objet à getters
                                // proto les perd — spéc-correct,
                                // source de bugs terrain.
                                if crate::heap::proto_has_accessor(&self.heap, sid) {
                                    self.diag("spread clone dropped proto getters");
                                }
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
                        // ES3 §13 / ES5 : le nom d'une
                        // FunctionExpression est ReadOnly dans son
                        // propre scope — écriture sloppy ignorée
                        // silencieusement (`sloppy_silent`), strict
                        // garde le `TypeError`.
                        m.insert(
                            name.clone(),
                            EnvCell::Bind(Binding {
                                value: fv,
                                mutable: false,
                                sloppy_silent: true,
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
                // Référent = le module *propriétaire* du call site
                // (liaison lexicale IMPORT_META — visible depuis
                // fonctions et callbacks) ; `current_module` ne
                // couvre que le code top-level synchrone.
                let referrer = self
                    .env_get(env, IMPORT_META)
                    .and_then(|v| self.to_js_string(v).ok())
                    .unwrap_or_else(|| self.current_module.clone());
                let pid = self.new_promise();
                match self.dynamic_import(&spec, &referrer) {
                    Ok(ns) => self.resolve_promise(pid, ns),
                    Err(Thrown(t)) => self.reject_promise(pid, t),
                }
                Ok(Value::Obj(pid))
            }
            // `import.meta` — `{url: <URL du module propriétaire>}`
            // (spec `HostGetImportMetaProperties`) — lue via la
            // liaison lexicale IMPORT_META posée à
            // l'instanciation ; un timer/callback la voit encore.
            // Repli `current_module` hors module (tolérance).
            Expr::ImportMeta => {
                let obj = self.new_object();
                let url_s = self
                    .env_get(env, IMPORT_META)
                    .and_then(|v| self.to_js_string(v).ok())
                    .unwrap_or_else(|| self.current_module.clone());
                let url = self.str_val(url_s);
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
                        // hits the host's `invoke` before prop lookup —
                        // sauf shadow own-prop : `hostObj.m = f` (patch
                        // de polyfill/shim — motif `patchDefine` de
                        // reddit, census shadow DOM) prime la méthode,
                        // spec [[Get]] avant l'appel ; le trampoline
                        // `n.call(t)` contourne le repli (référence
                        // capturée avant le patch → invoke original).
                        if let Value::Obj(bid) = recv {
                            if let Some(h) = self.host_of(bid) {
                                let shadowed = self.find_own(bid, &name).is_some();
                                if !shadowed {
                                    if let Some(v) = h.invoke(self, recv, &name, &args)? {
                                        // Réactions CE synchrones
                                        // (JS-C68) — la méthode host a
                                        // pu muter le miroir.
                                        self.drain_ce_reactions()?;
                                        return Ok(v);
                                    }
                                }
                            }
                        }
                        let f = self.get_prop_value(recv, &name)?;
                        if !self.is_callable(f) {
                            // Diagnostic : nommer la méthode + le
                            // receveur — les erreurs de pages
                            // réelles pointent l'API manquante. Pour
                            // un objet JS ordinaire, le nom de son
                            // constructeur (`D7`…) identifie la
                            // classe fautive dans du code minifié.
                            let recv_desc = match recv {
                                Value::Obj(rid) => match self.heap.obj(rid).map(|o| &o.kind) {
                                    Some(ObjKind::Host(h)) => h.class_name().to_string(),
                                    // Un callable receveur : le nom du
                                    // FnLit identifie la fonction dont
                                    // l'accesseur manque (`kW.vG`).
                                    Some(ObjKind::Fun(Fun::Script { lit, .. })) => format!(
                                        "function {}@{}:{}",
                                        lit.name.as_deref().unwrap_or("<anon>"),
                                        lit.pos.0,
                                        lit.pos.1
                                    ),
                                    Some(ObjKind::Fun(
                                        Fun::Native { name, .. } | Fun::BoundNative { name, .. },
                                    )) => {
                                        format!("native {name}")
                                    }
                                    Some(_) if self.is_callable(recv) => self
                                        .get_prop(rid, "name")
                                        .and_then(|n| self.heap_string_of(n))
                                        .filter(|n| !n.is_empty())
                                        .unwrap_or_else(|| "Function".to_string()),
                                    Some(_) => self
                                        .get_prop(rid, "constructor")
                                        .and_then(|c| self.get_prop_value(c, "name").ok())
                                        .and_then(|n| self.heap_string_of(n))
                                        .filter(|n| !n.is_empty())
                                        .unwrap_or_else(|| "Object".to_string()),
                                    None => "?".to_string(),
                                },
                                _ => format!("{recv:?}"),
                            };
                            return Err(self.type_error(format!(
                                "`{name}` is not a function (on {recv_desc})"
                            )));
                        }
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
                // `new` sur non-ctor : le callee est nommé —
                // les sondes de sites réels identifient l'API
                // hôte manquante du premier coup.
                if !self.is_callable(f) {
                    let who = match callee.as_ref() {
                        Expr::Ident(n) | Expr::Member(_, n) => n.as_str(),
                        _ => "?",
                    };
                    let d = self.describe(f);
                    return Err(
                        self.type_error(format!("new {who}: constructor is not a function: {d}"))
                    );
                }
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
                    // Spec order : la référence LHS (et les effets de
                    // bord de `base`/`key`) s'évalue AVANT le RHS —
                    // `(..., install(), K).Wn = rhs()` sur du code
                    // réel (Google Closure) en dépend.
                    if let Some(lv) = self.resolve_lvalue(env, this, lhs)? {
                        let v = self.eval_expr(env, this, rhs)?;
                        self.store_lvalue(env, lv, v)?;
                        return Ok(v);
                    }
                    let v = self.eval_expr(env, this, rhs)?;
                    self.write_lvalue(env, this, lhs, v)?;
                    return Ok(v);
                }
                // Logical assignment: rhs evaluated only when needed.
                if let Some(lv) = self.resolve_lvalue(env, this, lhs)? {
                    let cur = self.read_lvalue_ref(env, &lv)?;
                    match op {
                        AssignOp::Nullish if !matches!(cur, Value::Null | Value::Undefined) => {
                            return Ok(cur)
                        }
                        AssignOp::LogAnd if !self.to_boolean(cur) => return Ok(cur),
                        AssignOp::LogOr if self.to_boolean(cur) => return Ok(cur),
                        AssignOp::Nullish | AssignOp::LogAnd | AssignOp::LogOr => {
                            let v = self.eval_expr(env, this, rhs)?;
                            self.store_lvalue(env, lv, v)?;
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
                    self.store_lvalue(env, lv, v)?;
                    return Ok(v);
                }
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

    pub(crate) fn read_lvalue(&mut self, env: &EnvRef, this: Value, e: &Expr) -> JsResult<Value> {
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
            _ => Err(self.type_error("invalid assignment target")),
        }
    }

    pub(crate) fn write_lvalue(
        &mut self,
        env: &EnvRef,
        this: Value,
        e: &Expr,
        v: Value,
    ) -> JsResult<()> {
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
            // `super.x = v` / `super[k] = v` (mur réel x-web,
            // champs de classe) — lookup setter sur le proto
            // (`super_base`), valeur posée sur `this` (spec
            // [[Set]] receiver) via `write_prop`.
            Expr::SuperMember(name) => {
                let Value::Obj(base) = self.super_base(env)? else {
                    return Err(self.type_error("invalid `super` base"));
                };
                self.write_prop(base, this, name, v)
            }
            Expr::SuperIndex(k) => {
                let kv = self.eval_expr(env, this, k)?;
                let key = self.prop_key(kv)?;
                let Value::Obj(base) = self.super_base(env)? else {
                    return Err(self.type_error("invalid `super` base"));
                };
                self.write_prop(base, this, &key, v)
            }
            _ => Err(self.type_error("invalid assignment target")),
        }
    }

    /// Évalue une cible d'affectation en une référence réifiée —
    /// les expressions `base`/`key` courent ici (ordre spec : le LHS
    /// avant le RHS de `=`/`op=`). `None` pour les cibles non
    /// réifiables (déstructuration), qui gardent le chemin
    /// [`Self::write_lvalue`].
    pub(crate) fn resolve_lvalue(
        &mut self,
        env: &EnvRef,
        this: Value,
        e: &Expr,
    ) -> JsResult<Option<LValueRef>> {
        Ok(match e {
            Expr::Ident(name) => Some(LValueRef::Ident(name.clone())),
            Expr::Member(base, name) => {
                let b = self.eval_expr(env, this, base)?;
                Some(LValueRef::Prop(b, name.clone()))
            }
            Expr::Index(base, key) => {
                let b = self.eval_expr(env, this, base)?;
                let k = self.eval_expr(env, this, key)?;
                Some(LValueRef::Prop(b, self.prop_key(k)?))
            }
            _ => None,
        })
    }

    pub(crate) fn read_lvalue_ref(&mut self, env: &EnvRef, lv: &LValueRef) -> JsResult<Value> {
        match lv {
            LValueRef::Ident(name) => self
                .env_get(env, name)
                .ok_or_else(|| self.ref_error(format!("{name} is not defined"))),
            LValueRef::Prop(b, k) => self.get_prop_value(*b, k),
        }
    }

    pub(crate) fn store_lvalue(&mut self, env: &EnvRef, lv: LValueRef, v: Value) -> JsResult<()> {
        match lv {
            LValueRef::Ident(name) => self.env_set(env, &name, v),
            LValueRef::Prop(b, k) => self.set_prop_value(b, &k, v),
        }
    }

    pub(crate) fn unary_op(
        &mut self,
        op: UnOp,
        env: &EnvRef,
        this: Value,
        operand: &Expr,
    ) -> JsResult<Value> {
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

    pub(crate) fn binary_op(&mut self, op: BinOp, l: Value, r: Value) -> JsResult<Value> {
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
    pub(crate) fn eval_chain(
        &mut self,
        env: &EnvRef,
        this: Value,
        head: &Expr,
        steps: &[ChainStep],
    ) -> JsResult<Value> {
        let mut cur = self.eval_expr(env, this, head)?;
        // Receiver for `.m()`-style calls: the object the callee was
        // read from (plain calls get the global, per dynamic `this`),
        // plus the member key — a `hostObj?.m()` call dispatches to
        // `Host::invoke` like `hostObj.m()` does (dev.to `paginate` :
        // `localStorage?.getItem(...)` levait `value is not a
        // function`, la lecture de prop ne voyant pas `invoke`).
        let mut recv = Value::Obj(self.global);
        let mut recv_key: Option<String> = None;
        for step in steps {
            match step {
                ChainStep::Member(name) => {
                    recv = cur;
                    recv_key = Some(name.clone());
                    cur = self.get_prop_value(cur, name)?;
                }
                ChainStep::Index(k) => {
                    let kv = self.eval_expr(env, this, k)?;
                    let key = self.prop_key(kv)?;
                    recv = cur;
                    recv_key = Some(key.clone());
                    cur = self.get_prop_value(cur, &key)?;
                }
                ChainStep::OptMember(name) => {
                    if matches!(cur, Value::Null | Value::Undefined) {
                        return Ok(Value::Undefined);
                    }
                    recv = cur;
                    recv_key = Some(name.clone());
                    cur = self.get_prop_value(cur, name)?;
                }
                ChainStep::OptIndex(k) => {
                    if matches!(cur, Value::Null | Value::Undefined) {
                        return Ok(Value::Undefined);
                    }
                    let kv = self.eval_expr(env, this, k)?;
                    let key = self.prop_key(kv)?;
                    recv = cur;
                    recv_key = Some(key.clone());
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
                    // Même ordre que `Callee::Member` : `invoke` du
                    // host d'abord, la valeur lue ensuite (un host
                    // peut exposer les deux pour des noms distincts).
                    let mut done = false;
                    if let (Some(key), Value::Obj(bid)) = (&recv_key, recv) {
                        if let Some(h) = self.host_of(bid) {
                            if let Some(v) = h.invoke(self, recv, key, &args)? {
                                cur = v;
                                done = true;
                                // Réactions CE synchrones (JS-C68).
                                self.drain_ce_reactions()?;
                            }
                        }
                    }
                    if !done {
                        cur = self.call_value(cur, recv, &args)?;
                    }
                    recv = Value::Obj(self.global);
                    recv_key = None;
                }
            }
        }
        Ok(cur)
    }

    /// `==` per ES5 §11.9.3.
    pub(crate) fn loose_eq(&mut self, l: Value, r: Value) -> JsResult<bool> {
        let (l, r) = (l.unhole(), r.unhole());
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
    pub(crate) fn big_binary(&mut self, op: BinOp, l: Value, r: Value) -> JsResult<Value> {
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
                // Borne le résultat à 1 M de bits (comme `<<`
                // ci-dessous) — `10n ** 1e9n` produirait des
                // gigaoctets de limbs sans ce plafond (JS-C55).
                if e > 0 && a.bits().saturating_mul(u64::from(e)) > 1_000_000 {
                    return Err(self.range_error("BigInt result too large"));
                }
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
    pub(crate) fn big_rel(&mut self, l: Value, r: Value) -> JsResult<Option<std::cmp::Ordering>> {
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
