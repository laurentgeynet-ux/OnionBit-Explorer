// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! JS-C34 — exécution **suspendable** des corps de
//! `function*`/`async fn`.
//!
//! Le tree-walker récursif ne peut pas suspendre la pile
//! Rust (`Rc`/`RefCell` `!Send`, `forbid(unsafe)`). Modèle :
//!
//! 1. `lower_suspend_body` — tout `yield`/`await` est hissé
//!    en position « init de déclaration » ou statement
//!    expression (`let __gN = yield e ; S[__gN]`) : suspendre
//!    n'arrive plus qu'à des positions de statement, jamais
//!    au milieu d'une expression. Les positions paresseuses
//!    (`?:`/`&&`/`||`/`??`/`?.`) portant un suspend sont
//!    hissées éagerment — déviation documentée (rare).
//!    JS-C96 : couverture étendue aux têtes de statements
//!    (`for` init, `switch` case, `with`, `using`, `for-in`
//!    init), aux motifs (`{a = …}`, `[… = …]`, clés
//!    calculées — déclarations, `for-of`, `catch`,
//!    paramètres), aux clés calculées d'objets/classes, à
//!    `extends`, à `export` et aux paramètres par défaut
//!    (`lower_suspend_fn`). Déviations assumées — le
//!    suspend est évalué **une fois, éagerment** là où la
//!    spec le remettrait (défauts de motifs non tirés,
//!    `case` non atteints, `catch` sans exception, `for`
//!    test/update rejoués par la spec — hissés une fois) ;
//!    les paramètres `a = await e` gardent la paresse via
//!    `if (a === undefined) a = e` dans le corps.
//! 2. `GExec` — pile de contrôle explicite (`Vec<GFrame>`) :
//!    chaque statement composé ouvre une frame ; un
//!    `yield`/`await` suspend la pile entière. Reprendre =
//!    reprendre la boucle au `pc` sauvegardé.
//! 3. `await` : promesse pendante → le frame s'enfile comme
//!    réaction `then` (fulfill → lie `into`, reject → throw
//!    dans le frame) ; `return v` résout la promesse de
//!    l'async, `throw` la rejette.

#[allow(clippy::wildcard_imports)]
use super::*;
use crate::ast::{
    AssignOp, BinOp, ClassLit, ExportDecl, Expr, FnLit, ForInit, ForTarget, Param, Pat, PropKey,
    Stmt, VarDecl,
};
use crate::heap::{Env, Fun, NativeFn, ObjKind};
use crate::value::Value;
use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

// ------------------------------------------------------------------
// 1. Lowering — suspendables hissés en positions de statement
// ------------------------------------------------------------------

/// `true` si `e` contient un `yield`/`await` (recursion sur
/// les enfants — mêmes positions que `take_children`).
fn has_suspend(e: &Expr) -> bool {
    match e {
        Expr::Yield { .. } | Expr::Await(_) => true,
        Expr::Unary(_, a)
        | Expr::Member(a, _)
        | Expr::Spread(a)
        | Expr::ImportCall(a)
        | Expr::SuperIndex(a) => has_suspend(a),
        Expr::Binary(_, l, r) | Expr::Assign(l, _, r) | Expr::Index(l, r) => {
            has_suspend(l) || has_suspend(r)
        }
        Expr::Cond(c, t, e2) => has_suspend(c) || has_suspend(t) || has_suspend(e2),
        Expr::Call(f, args) | Expr::New(f, args) => has_suspend(f) || args.iter().any(has_suspend),
        Expr::Array(items) => items.iter().flatten().any(has_suspend),
        Expr::Object(props) => props.iter().any(|p| match p {
            crate::ast::ObjProp::Prop(k, v) => key_has_suspend(k) || has_suspend(v),
            crate::ast::ObjProp::Spread(v) => has_suspend(v),
            crate::ast::ObjProp::Getter(k, _) | crate::ast::ObjProp::Setter(k, _) => {
                key_has_suspend(k)
            }
        }),
        // `class [X] extends <e>` + clés calculées — évaluées dans
        // la portée englobante à la définition (les corps de
        // membres sont leur propre frontière).
        Expr::Class(cl) => class_has_suspend(cl),
        Expr::Seq(items) | Expr::SuperCall(items) => items.iter().any(has_suspend),
        Expr::Template { exprs, .. } => exprs.iter().any(has_suspend),
        Expr::TaggedTemplate { tag, exprs, .. } => {
            has_suspend(tag) || exprs.iter().any(has_suspend)
        }
        Expr::Chain { head, steps } => {
            has_suspend(head)
                || steps.iter().any(|s| match s {
                    crate::ast::ChainStep::Index(v) | crate::ast::ChainStep::OptIndex(v) => {
                        has_suspend(v)
                    }
                    crate::ast::ChainStep::Call(a) | crate::ast::ChainStep::OptCall(a) => {
                        a.iter().any(has_suspend)
                    }
                    _ => false,
                })
        }
        _ => false,
    }
}

/// Compteur de noms `__gN`.
struct LowerCx {
    n: usize,
}

impl LowerCx {
    fn fresh(&mut self) -> String {
        self.n += 1;
        format!("__g{}", self.n)
    }
}

/// `yield e`/`await e` au sommet → `Some` (node suspendable).
fn is_suspend(e: &Expr) -> bool {
    matches!(e, Expr::Yield { .. } | Expr::Await(_))
}

/// Réécrit `e` sur place : chaque sous-expression
/// suspendable est remplacée par `__gN`, la déclaration
/// `let __gN = <node>` posée dans `pre` (ordre
/// gauche-à-droite d'évaluation). Les enfants du suspend
/// sont eux-mêmes liftés avant (suspend imbriqué).
fn lift(e: &mut Expr, pre: &mut Vec<Stmt>, cx: &mut LowerCx) {
    if is_suspend(e) {
        // Lift l'argument d'abord (`yield (await x)`…).
        match e {
            Expr::Yield { arg: Some(a), .. } | Expr::Await(a) => lift(a, pre, cx),
            _ => {}
        }
        let n = cx.fresh();
        let node = std::mem::replace(e, Expr::Ident(n.clone()));
        pre.push(Stmt::Var(vec![VarDecl {
            pat: Pat::Ident(n),
            init: Some(node),
        }]));
        return;
    }
    // Descente dans les enfants — même couverture que
    // `take_children`, en `&mut` non destructif.
    match e {
        Expr::Unary(_, a)
        | Expr::Member(a, _)
        | Expr::Spread(a)
        | Expr::ImportCall(a)
        | Expr::SuperIndex(a) => lift(a, pre, cx),
        Expr::Binary(_, l, r) | Expr::Assign(l, _, r) | Expr::Index(l, r) => {
            lift(l, pre, cx);
            lift(r, pre, cx);
        }
        Expr::Cond(c, t, e2) => {
            lift(c, pre, cx);
            lift(t, pre, cx);
            lift(e2, pre, cx);
        }
        Expr::Call(f, args) | Expr::New(f, args) => {
            lift(f, pre, cx);
            for a in args {
                lift(a, pre, cx);
            }
        }
        Expr::Array(items) => {
            for it in items.iter_mut().flatten() {
                lift(it, pre, cx);
            }
        }
        Expr::Object(props) => {
            for p in props.iter_mut() {
                match p {
                    crate::ast::ObjProp::Prop(k, v) => {
                        lift_key(k, pre, cx);
                        lift(v, pre, cx);
                    }
                    crate::ast::ObjProp::Spread(v) => lift(v, pre, cx),
                    crate::ast::ObjProp::Getter(k, _) | crate::ast::ObjProp::Setter(k, _) => {
                        lift_key(k, pre, cx);
                    }
                }
            }
        }
        Expr::Class(cl) => lift_classlit(std::rc::Rc::make_mut(cl), pre, cx),
        Expr::Seq(items) | Expr::SuperCall(items) => {
            for it in items {
                lift(it, pre, cx);
            }
        }
        Expr::Template { exprs, .. } => {
            for x in exprs {
                lift(x, pre, cx);
            }
        }
        Expr::TaggedTemplate { tag, exprs, .. } => {
            lift(tag, pre, cx);
            for x in exprs {
                lift(x, pre, cx);
            }
        }
        Expr::Chain { head, steps } => {
            lift(head, pre, cx);
            for s in steps {
                match s {
                    crate::ast::ChainStep::Index(v) | crate::ast::ChainStep::OptIndex(v) => {
                        lift(v, pre, cx);
                    }
                    crate::ast::ChainStep::Call(a) | crate::ast::ChainStep::OptCall(a) => {
                        for x in a {
                            lift(x, pre, cx);
                        }
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }
}

// ------------------------------------------------------------------
// 1b. Positions complémentaires (JS-C96) — clés calculées,
//     classes, motifs destructurants, cibles `for-*`
// ------------------------------------------------------------------

/// `true` si la clé de propriété est calculée et porte un suspend.
fn key_has_suspend(k: &PropKey) -> bool {
    matches!(k, PropKey::Computed(e) if has_suspend(e))
}

/// Hissage d'une clé calculée `PropKey::Computed(e)`.
fn lift_key(k: &mut PropKey, pre: &mut Vec<Stmt>, cx: &mut LowerCx) {
    if let PropKey::Computed(e) = k {
        lift(e, pre, cx);
    }
}

/// `class … extends <e>` ou membre `[e]` avec suspend — les
/// expressions évaluées dans la portée englobante à la
/// définition (les corps `fun` des membres sont des
/// frontières propres — jamais descendus).
fn class_has_suspend(cl: &ClassLit) -> bool {
    cl.super_.as_ref().is_some_and(has_suspend)
        || cl.members.iter().any(|m| key_has_suspend(&m.key))
}

/// Hissage `extends` + clés calculées de membres.
fn lift_classlit(cl: &mut ClassLit, pre: &mut Vec<Stmt>, cx: &mut LowerCx) {
    if let Some(s) = &mut cl.super_ {
        lift(s, pre, cx);
    }
    for m in &mut cl.members {
        lift_key(&mut m.key, pre, cx);
    }
}

/// `true` si le motif (déclaration destructurante, paramètre,
/// `catch`, cible `for-*`) contient un suspend : `= défaut`,
/// clé calculée `[e]` ou lvalue.
fn has_suspend_pat(p: &Pat) -> bool {
    match p {
        Pat::Ident(_) => false,
        Pat::Lvalue(e) => has_suspend(e),
        Pat::Rest(p) => has_suspend_pat(p),
        Pat::Default(p, e) => has_suspend_pat(p) || has_suspend(e),
        Pat::Array(items) => items.iter().flatten().any(has_suspend_pat),
        Pat::Object { props, rest } => {
            props
                .iter()
                .any(|(k, sp)| key_has_suspend(k) || has_suspend_pat(sp))
                || rest.as_deref().is_some_and(has_suspend_pat)
        }
    }
}

/// Hissage des suspends d'un motif — **évalués éagerment** là
/// où la spec ne les évalue que si le défaut tire (même
/// déviation que `?:`/`&&`, documentée en en-tête).
fn lift_pat(p: &mut Pat, pre: &mut Vec<Stmt>, cx: &mut LowerCx) {
    match p {
        Pat::Ident(_) => {}
        Pat::Lvalue(e) => lift(e, pre, cx),
        Pat::Rest(p) => lift_pat(p, pre, cx),
        Pat::Default(p, e) => {
            lift_pat(p, pre, cx);
            lift(e, pre, cx);
        }
        Pat::Array(items) => {
            for el in items.iter_mut().flatten() {
                lift_pat(el, pre, cx);
            }
        }
        Pat::Object { props, rest } => {
            for (k, sp) in props.iter_mut() {
                lift_key(k, pre, cx);
                lift_pat(sp, pre, cx);
            }
            if let Some(r) = rest {
                lift_pat(r, pre, cx);
            }
        }
    }
}

/// `true` si la cible `for-in`/`for-of` porte un suspend.
fn ftarget_has_suspend(t: &ForTarget) -> bool {
    match t {
        ForTarget::Var(p) | ForTarget::Lexical { pat: p, .. } | ForTarget::Using { pat: p, .. } => {
            has_suspend_pat(p)
        }
        ForTarget::Expr(e) => has_suspend(e),
    }
}

/// Hissage de la cible `for-in`/`for-of` — évaluée une fois
/// avant la boucle (les défauts de motif, tirés par la spec à
/// chaque itération, ne le sont qu'une fois — déviation).
fn lift_ftarget(t: &mut ForTarget, pre: &mut Vec<Stmt>, cx: &mut LowerCx) {
    match t {
        ForTarget::Var(p) | ForTarget::Lexical { pat: p, .. } | ForTarget::Using { pat: p, .. } => {
            lift_pat(p, pre, cx);
        }
        ForTarget::Expr(e) => lift(e, pre, cx),
    }
}

/// `true` si l'init `for (…)` porte un suspend.
fn finit_has_suspend(i: &ForInit) -> bool {
    match i {
        ForInit::Expr(e) => has_suspend(e),
        ForInit::Var(ds) | ForInit::Lexical { decls: ds, .. } => ds
            .iter()
            .any(|d| d.init.as_ref().is_some_and(has_suspend) || has_suspend_pat(&d.pat)),
    }
}

/// Hissage des déclarations de `for (…)` / `using …`.
fn lift_decls(ds: &mut [VarDecl], pre: &mut Vec<Stmt>, cx: &mut LowerCx) {
    for d in ds {
        lift_pat(&mut d.pat, pre, cx);
        if let Some(i) = &mut d.init {
            lift(i, pre, cx);
        }
    }
}

/// Réécrit un statement : suspendables hissés en déclarations
/// `let __gN = …` posées AVANT ; le statement utilise `__gN`.
fn lower_stmt(s: &Stmt, cx: &mut LowerCx) -> Vec<Stmt> {
    let mut pre = Vec::new();
    let mut s2 = s.clone();
    match &mut s2 {
        Stmt::Expr(e) | Stmt::Return(Some(e)) | Stmt::Throw(e) => lift(e, &mut pre, cx),
        Stmt::Var(decls) | Stmt::Lexical { decls, .. } | Stmt::Using { decls, .. } => {
            lift_decls(decls, &mut pre, cx);
        }

        Stmt::If { cond, .. } | Stmt::While { cond, .. } | Stmt::DoWhile { cond, .. } => {
            lift(cond, &mut pre, cx);
        }
        Stmt::For {
            init, test, update, ..
        } => {
            if let Some(i) = init {
                match i {
                    ForInit::Expr(e) => lift(e, &mut pre, cx),
                    ForInit::Var(ds) | ForInit::Lexical { decls: ds, .. } => {
                        lift_decls(ds, &mut pre, cx);
                    }
                }
            }
            if let Some(t) = test {
                lift(t, &mut pre, cx);
            }
            if let Some(u) = update {
                lift(u, &mut pre, cx);
            }
        }
        Stmt::ForOf { target, iter, .. } => {
            lift_ftarget(target, &mut pre, cx);
            lift(iter, &mut pre, cx);
        }
        Stmt::ForIn {
            target,
            obj,
            var_init,
            ..
        } => {
            lift_ftarget(target, &mut pre, cx);
            lift(obj, &mut pre, cx);
            if let Some(v) = var_init {
                lift(v, &mut pre, cx);
            }
        }
        Stmt::Switch { disc, cases } => {
            lift(disc, &mut pre, cx);
            // `case await e:` — hissé avant le switch : les tests
            // sont évalués même quand le case n'est pas atteint
            // (déviation « éager », même classe que `?:`).
            for c in cases {
                if let Some(t) = &mut c.test {
                    lift(t, &mut pre, cx);
                }
            }
        }
        Stmt::With { obj, .. } => lift(obj, &mut pre, cx),
        // `catch ({a = await e})` — hissé avant le `try` : le
        // défaut est évalué même sans exception (déviation).
        Stmt::Try {
            catch: Some((pat, _)),
            ..
        } => lift_pat(pat, &mut pre, cx),
        Stmt::ClassDecl(cl) => lift_classlit(std::rc::Rc::make_mut(cl), &mut pre, cx),
        Stmt::Export(d) => match d {
            ExportDecl::Default(e) => lift(e, &mut pre, cx),
            ExportDecl::Decl(inner) => match &mut **inner {
                Stmt::Var(ds) | Stmt::Lexical { decls: ds, .. } | Stmt::Using { decls: ds, .. } => {
                    lift_decls(ds, &mut pre, cx);
                }
                Stmt::ClassDecl(cl) => lift_classlit(std::rc::Rc::make_mut(cl), &mut pre, cx),
                _ => {}
            },
            _ => {}
        },
        _ => {}
    }
    pre.push(s2);
    pre
}

/// `true` si le statement contient un suspend en position
/// non liftée (corps de if/while/for/try…).
pub(crate) fn stmt_has_suspend(s: &Stmt) -> bool {
    match s {
        Stmt::Expr(e) | Stmt::Throw(e) | Stmt::Return(Some(e)) => has_suspend(e),
        Stmt::Var(ds) | Stmt::Lexical { decls: ds, .. } | Stmt::Using { decls: ds, .. } => ds
            .iter()
            .any(|d| d.init.as_ref().is_some_and(has_suspend) || has_suspend_pat(&d.pat)),

        Stmt::Block(l) => l.iter().any(stmt_has_suspend),
        Stmt::If {
            cond, then, else_, ..
        } => {
            has_suspend(cond)
                || stmt_has_suspend(then)
                || else_.as_deref().is_some_and(stmt_has_suspend)
        }
        Stmt::While { cond, body } | Stmt::DoWhile { body, cond } => {
            has_suspend(cond) || stmt_has_suspend(body)
        }
        Stmt::For {
            init,
            test,
            update,
            body,
            ..
        } => {
            init.as_ref().is_some_and(finit_has_suspend)
                || test.as_ref().is_some_and(has_suspend)
                || update.as_ref().is_some_and(has_suspend)
                || stmt_has_suspend(body)
        }
        Stmt::ForIn {
            target,
            obj,
            var_init,
            body,
        } => {
            has_suspend(obj)
                || var_init.as_ref().is_some_and(has_suspend)
                || ftarget_has_suspend(target)
                || stmt_has_suspend(body)
        }
        Stmt::ForOf {
            target,
            iter,
            body,
            is_await,
        } => {
            *is_await || has_suspend(iter) || ftarget_has_suspend(target) || stmt_has_suspend(body)
        }
        Stmt::Try {
            block,
            catch,
            finally,
        } => {
            block.iter().any(stmt_has_suspend)
                || catch
                    .as_ref()
                    .is_some_and(|(p, b)| has_suspend_pat(p) || b.iter().any(stmt_has_suspend))
                || finally
                    .as_ref()
                    .is_some_and(|f| f.iter().any(stmt_has_suspend))
        }
        Stmt::Labeled { body, .. } => stmt_has_suspend(body),
        Stmt::Switch { disc, cases } => {
            has_suspend(disc)
                || cases.iter().any(|c| {
                    c.test.as_ref().is_some_and(has_suspend) || c.body.iter().any(stmt_has_suspend)
                })
        }
        // JS-C96 — positions étendues : `with`, `class` (extends/
        // clés), `export`, `for await` (le `is_await` seul
        // suffit — l'attente par pas est une suspension).
        Stmt::With { obj, body } => has_suspend(obj) || stmt_has_suspend(body),
        Stmt::ClassDecl(cl) => class_has_suspend(cl),
        Stmt::Export(d) => match d {
            ExportDecl::Default(e) => has_suspend(e),
            ExportDecl::Decl(inner) => stmt_has_suspend(inner),
            _ => false,
        },
        _ => false,
    }
}

/// Descente récursive : chaque sous-statement composé est
/// loweré à son tour (les corps de if/while/try…).
fn lower_list(list: &[Stmt], cx: &mut LowerCx) -> Vec<Stmt> {
    let mut out = Vec::new();
    for s in list {
        let mut s2 = s.clone();
        match &mut s2 {
            Stmt::Block(l) => *l = lower_list(l, cx),
            Stmt::If { then, else_, .. } => {
                **then = Stmt::Block(lower_list(std::slice::from_ref(then), cx));
                if let Some(els) = else_ {
                    **els = Stmt::Block(lower_list(std::slice::from_ref(els), cx));
                }
            }
            Stmt::While { body, .. }
            | Stmt::DoWhile { body, .. }
            | Stmt::For { body, .. }
            | Stmt::ForIn { body, .. }
            | Stmt::ForOf { body, .. }
            | Stmt::With { body, .. }
            | Stmt::Labeled { body, .. } => {
                **body = Stmt::Block(lower_list(std::slice::from_ref(body), cx));
            }
            Stmt::Try {
                block,
                catch,
                finally,
            } => {
                *block = lower_list(block, cx);
                if let Some((_, b)) = catch {
                    *b = lower_list(b, cx);
                }
                if let Some(f) = finally {
                    *f = lower_list(f, cx);
                }
            }
            Stmt::Switch { cases, .. } => {
                for c in cases {
                    c.body = lower_list(&c.body, cx);
                }
            }
            _ => {}
        }
        out.extend(lower_stmt(&s2, cx));
    }
    out
}

/// Fonction suspendable → `(params réécrits, corps loweré)`.
///
/// Les `= défaut` de paramètres contenant un suspend sont
/// déplacés dans le corps — la liaison s'exécute hors
/// `GExec`, un `await` pendante y jetterait (`JS-C96`) :
///
/// - `a = e` → param `a` + `if (a === undefined) a = e` en
///   tête de corps — la suspension vit dans le corps et la
///   paresse spec est préservée (défaut évalué seulement si
///   l'arg est `undefined`) ;
/// - motif complexe (`{a = …}`, `[…]`) → param `__paramI` +
///   `let <motif> = __paramI` (les défauts internes sont
///   hissés éagerment — déviation documentée) ;
/// - `p = e` non-ident → `__paramI` + `if` + `let p = …`.
///
/// `lit` inchangé (`fn.length`, `args_map` lisent l'original).
pub(crate) fn lower_suspend_fn(lit: &FnLit) -> (Vec<Param>, Rc<Vec<Stmt>>) {
    let mut prelude: Vec<Stmt> = Vec::new();
    let mut params = lit.params.clone();
    for (i, p) in params.iter_mut().enumerate() {
        if !has_suspend_pat(&p.pat) {
            continue;
        }
        if let Pat::Default(inner, e) = &mut p.pat {
            let (name, need_bind) = if let Pat::Ident(n) = &**inner {
                (n.clone(), false)
            } else {
                (format!("__param{i}"), true)
            };
            let def = std::mem::replace(&mut **e, Expr::Null);
            prelude.push(Stmt::If {
                cond: Expr::Binary(
                    BinOp::StrictEq,
                    Box::new(Expr::Ident(name.clone())),
                    Box::new(Expr::Ident("undefined".into())),
                ),
                then: Box::new(Stmt::Expr(Expr::Assign(
                    Box::new(Expr::Ident(name.clone())),
                    AssignOp::Set,
                    Box::new(def),
                ))),
                else_: None,
            });
            if need_bind {
                prelude.push(Stmt::Var(vec![VarDecl {
                    pat: (**inner).clone(),
                    init: Some(Expr::Ident(name.clone())),
                }]));
            }
            p.pat = Pat::Ident(name);
        } else {
            let name = format!("__param{i}");
            let orig = std::mem::replace(&mut p.pat, Pat::Ident(name.clone()));
            prelude.push(Stmt::Var(vec![VarDecl {
                pat: orig,
                init: Some(Expr::Ident(name)),
            }]));
        }
    }
    let mut body = prelude;
    body.extend(lit.body.iter().cloned());
    (params, lower_suspend_body(&body))
}

/// Corps de générateur/async → liste lowerée partagée.
pub(crate) fn lower_suspend_body(body: &[Stmt]) -> Rc<Vec<Stmt>> {
    let mut cx = LowerCx { n: 0 };
    let out = Rc::new(lower_list(body, &mut cx));
    #[cfg(debug_assertions)]
    if std::env::var("DBG_LOWER").is_ok() {
        eprintln!("LOWERED {out:?}");
    }
    out
}

// ------------------------------------------------------------------
// 2. `GExec` — exécuteur à pile de contrôle
// ------------------------------------------------------------------

/// Ce que le frame attend pour reprendre.
enum Pending {
    /// `yield` émis — `next(v)` lie `v` au binding `into`.
    Yield { into: Option<String> },
    /// `yield*` — file d'items en vidange.
    Drain {
        items: Vec<Value>,
        idx: usize,
        into: Option<String>,
    },
    /// `await` pendante — la réaction `then` boundée reprend
    /// le frame (fulfill lie `into`, reject jette au site).
    Await { into: Option<String>, prom: ObjId },
}

/// Kind de frame — statements composés porteurs de
/// suspensions ; les autres passent par `exec_stmt`.
enum FKind {
    /// Liste plate (corps, bloc, then/else, catch, finally).
    Seq,
    /// `while`/`do` — test réévalué en sortie.
    While { cond: Expr },
    /// `for (…; test; update)`.
    For {
        test: Option<Expr>,
        update: Option<Expr>,
    },
    /// `for (t of/in it)` — items matérialisés.
    ForIter {
        items: Vec<Value>,
        idx: usize,
        target: ForTarget,
    },
    /// `for await (t of it)` — `it.next()` awaited par pas
    /// (spec `ForIn/OfBodyEvaluation` async) : `issued` marque
    /// un `next()` émis en attente de sa promesse (le résultat
    /// `{value,done}` revient lié à `__awaitit` à la reprise).
    ForAwait {
        iter: Value,
        next: Value,
        target: ForTarget,
        issued: bool,
    },
    /// `try` — throw → catch si présent ; `finally` toujours.
    Try {
        catch: Option<(Pat, Vec<Stmt>)>,
        finally: Option<Vec<Stmt>>,
    },
    /// `switch` porteur de suspensions — sa liste est le
    /// corps **aplatit** du case atteint jusqu'à la fin
    /// (fallthrough) ; un `break` non labellé s'y arrête.
    Switch,
}

/// Une frame : liste + curseur + env + labels + kind.
struct GFrame {
    list: Rc<Vec<Stmt>>,
    pc: usize,
    env: EnvRef,
    labels: Vec<String>,
    kind: FKind,
}

/// Issue d'un statement dans le frame.
enum GFlow {
    /// `pc` avance.
    Adv,
    /// Frames mutées en place (throw rattrapée → catch
    /// poussé) — `pc` du frame appelant ne s'avance PAS.
    Susp,
    /// `return v` — le corps termine.
    Ret(Value),
    /// `break [l]` / `continue [l]`.
    Brk(Option<String>),
    /// `continue [l]`.
    Cont(Option<String>),
}

/// Exécuteur suspendable — aucune récursion native entre
/// statements.
pub(crate) struct GExec {
    stack: Vec<GFrame>,
    this: Value,
    resume: Option<std::result::Result<Value, Value>>,
    pending: Option<Pending>,
    done: bool,
    ret: Value,
    emit: Value,
    /// Throw en cours pendant un `finally` sans catch.
    unwinding: Option<Value>,
    /// Promesse retournée par l'`async fn` — settlée à la
    /// fin du corps (`return` → fulfill, throw → reject).
    /// `None` pour un générateur (`function*`).
    outer: Option<ObjId>,
    /// L'objet `ObjKind::GenLive` porteur de cet exec — les
    /// réactions `await` s'y boundent (racine GC).
    gen_obj: Value,
}

/// `args[i]` ou `Undefined` — convention natives.
fn arg(args: &[Value], i: usize) -> Value {
    args.get(i).copied().unwrap_or(Value::Undefined)
}

/// `{value, done}` résultat d'itération.
fn iter_result(e: &mut Engine, value: Value, done: bool) -> Value {
    let o = e.new_object();
    e.set_own(o, "value", value, true);
    e.set_own(o, "done", Value::Bool(done), true);
    Value::Obj(o)
}

impl GExec {
    pub(crate) fn new(env: EnvRef, this: Value, body: Rc<Vec<Stmt>>) -> Self {
        Self {
            stack: vec![GFrame {
                list: body,
                pc: 0,
                env,
                labels: Vec::new(),
                kind: FKind::Seq,
            }],
            this,
            resume: None,
            pending: None,
            done: false,
            ret: Value::Undefined,
            emit: Value::Undefined,
            unwinding: None,
            outer: None,
            gen_obj: Value::Undefined,
        }
    }

    /// Attache la promesse de l'async fn (settlement au
    /// `done`).
    pub(crate) fn set_outer(&mut self, prom: ObjId) {
        self.outer = Some(prom);
    }

    /// Première course d'un corps async — jusqu'au premier
    /// `await` pendante ou la fin.
    pub(crate) fn start(&mut self, e: &mut Engine) -> JsResult<()> {
        match self.drive(e) {
            Ok(_) | Err(_) => Ok(()),
        }
    }

    /// `next(v)` — reprend ; `{value,done}`.
    pub(crate) fn next(&mut self, e: &mut Engine, v: Value) -> JsResult<Value> {
        if self.done {
            return Ok(iter_result(e, Value::Undefined, true));
        }
        self.resume = Some(Ok(v));
        self.drive(e)
    }

    /// `throw(e)` — jette au site suspendu.
    pub(crate) fn throw_at(&mut self, e: &mut Engine, v: Value) -> JsResult<Value> {
        self.resume = Some(Err(v));
        self.drive(e)
    }

    /// `return(v)` — `{value:v,done:true}` (borné : sortie
    /// forcée, `finally` simplifiée).
    pub(crate) fn ret_at(&mut self, e: &mut Engine, v: Value) -> JsResult<Value> {
        self.done = true;
        self.ret = v;
        Ok(iter_result(e, v, true))
    }

    /// Reprise `await` — fulfill `Ok`, reject `Err`.
    /// (JS-C34b — le driver promesse l'appellera.)
    #[allow(dead_code)]
    pub(crate) fn resume_await(
        &mut self,
        e: &mut Engine,
        r: std::result::Result<Value, Value>,
    ) -> JsResult<Value> {
        self.resume = Some(r);
        self.drive(e)
    }

    /// `true` quand le corps est terminé (JS-C34b).
    #[allow(dead_code)]
    pub(crate) fn is_done(&self) -> bool {
        self.done
    }

    fn drive(&mut self, e: &mut Engine) -> JsResult<Value> {
        if let Some(r) = self.resume.take() {
            match r {
                Ok(v) => self.liver(e, v)?,
                Err(v) => self.throw_into(e, v)?,
            }
        }
        loop {
            // Wedge gov.uk : une boucle dans un corps lowered
            // (`await` → `GenLive`) tournait sans jamais ticker
            // le budget — spin infini dans `eval_module`
            // (op-limit jamais atteint). Chaque pas du driver
            // compte désormais.
            e.tick()?;
            // `yield*` en vidange : les items restants
            // s'émettent AVANT tout statement — sinon le pc
            // suivant (`Expr(__gN)` lifté) tournerait pendant
            // que `into` n'est pas encore livré.
            if self.pending.is_some() && self.drain_step(e)? {
                return Ok(iter_result(e, self.emit, false));
            }
            let Some(f) = self.stack.last() else {
                self.done = true;
                self.settle_outer(e, Ok(self.ret));
                return Ok(iter_result(e, self.ret, true));
            };
            if f.pc >= f.list.len() {
                self.leave_frame(e)?;
                continue;
            }
            let (list, env, pc) = {
                let f = self.stack.last().unwrap();
                (f.list.clone(), f.env.clone(), f.pc)
            };
            let stmt = list[pc].clone();
            if std::env::var("DBG_DRIVE").is_ok() {
                eprintln!(
                    "[exec] pc={pc} env {:p} stmt={:?}",
                    Rc::as_ptr(&env),
                    std::mem::discriminant(&stmt)
                );
            }
            // Le statement peut POUSSER une frame (if/while/
            // block…) — `Adv` doit avancer le frame *courant*
            // à l'appel, pas le frame poussé (sinon sa pc=0
            // saute sa première instruction).
            let depth = self.stack.len();
            match self.exec_one(e, &stmt, &env) {
                Ok(GFlow::Adv) => {
                    if let Some(f) = self.stack.get_mut(depth - 1) {
                        f.pc += 1;
                    }
                }
                Ok(GFlow::Susp) => {}
                Ok(GFlow::Ret(v)) => {
                    self.ret = v;
                    self.stack.clear();
                    self.done = true;
                    self.settle_outer(e, Ok(v));
                    return Ok(iter_result(e, self.ret, true));
                }
                Ok(GFlow::Brk(l)) => self.do_break(e, l.as_ref())?,
                Ok(GFlow::Cont(l)) => self.do_cont(e, l.as_ref())?,
                Err(Thrown(v)) => self.throw_into(e, v)?,
            }
            if self.pending.is_some() && self.drain_step(e)? {
                return Ok(iter_result(e, self.emit, false));
            }
        }
    }

    /// Un `yield*` en file : émet l'item courant ; file vide
    /// → `pending` effacé, boucle reprend. `true` tant qu'un
    /// item est émis.
    fn drain_step(&mut self, _e: &mut Engine) -> JsResult<bool> {
        let Some(p) = self.pending.take() else {
            return Ok(false);
        };
        match p {
            Pending::Drain { items, idx, into } => {
                if idx >= items.len() {
                    if let Some(n) = into {
                        if let Some(f) = self.stack.last() {
                            let env = f.env.clone();
                            Engine::env_put(&env, &n, Value::Undefined, true);
                        }
                    }
                    return Ok(false);
                }
                self.emit = items[idx];
                self.pending = Some(Pending::Drain {
                    items,
                    idx: idx + 1,
                    into,
                });
                Ok(true)
            }
            other @ (Pending::Yield { .. } | Pending::Await { .. }) => {
                self.pending = Some(other);
                Ok(true)
            }
        }
    }

    /// `next(v)` livré au site suspendu.
    fn liver(&mut self, _e: &mut Engine, v: Value) -> JsResult<()> {
        if std::env::var("DBG_DRIVE").is_ok() {
            eprintln!(
                "[liver] pending={} stack={}",
                self.pending.is_some(),
                self.stack.len()
            );
        }
        match self.pending.take() {
            Some(Pending::Yield { into: Some(n) } | Pending::Await { into: Some(n), .. }) => {
                if let Some(env) = self.stack.last().map(|f| f.env.clone()) {
                    if std::env::var("DBG_DRIVE").is_ok() {
                        eprintln!("[bind] {n} -> env {:p}", Rc::as_ptr(&env));
                    }
                    Engine::env_put(&env, &n, v, true);
                }
            }
            Some(Pending::Drain { items, idx, into }) => {
                self.pending = Some(Pending::Drain { items, idx, into });
            }
            _ => {}
        }
        Ok(())
    }

    /// `break` — pop jusqu'à la boucle/le label porteur.
    fn do_break(&mut self, e: &mut Engine, label: Option<&String>) -> JsResult<()> {
        loop {
            let Some(f) = self.stack.last() else {
                return Ok(());
            };
            let is_loop = matches!(
                f.kind,
                FKind::While { .. }
                    | FKind::For { .. }
                    | FKind::ForIter { .. }
                    | FKind::ForAwait { .. }
            );
            // `break` nu : boucle **ou switch** englobant
            // (`switch` est la seule autre cible de `break`
            // non labellé — spec `BreakableStatement`).
            let hit = match &label {
                Some(l) => f.labels.iter().any(|x| x == *l),
                None => is_loop || matches!(f.kind, FKind::Switch),
            };
            let f = self.stack.pop().unwrap();
            self.run_finally(e, &f)?;
            if hit {
                return Ok(());
            }
        }
    }

    /// `continue` — pop jusqu'à la boucle, puis re-test.
    fn do_cont(&mut self, e: &mut Engine, label: Option<&String>) -> JsResult<()> {
        loop {
            let Some(f) = self.stack.last() else {
                return Ok(());
            };
            let is_loop = matches!(
                f.kind,
                FKind::While { .. }
                    | FKind::For { .. }
                    | FKind::ForIter { .. }
                    | FKind::ForAwait { .. }
            );
            let hit = match &label {
                Some(l) => is_loop && f.labels.iter().any(|x| x == *l),
                None => is_loop,
            };
            if hit {
                let f = self.stack.last_mut().unwrap();
                f.pc = f.list.len();
                return self.leave_frame(e);
            }
            let f = self.stack.pop().unwrap();
            self.run_finally(e, &f)?;
        }
    }

    /// Settlement de la promesse de l'async fn (`outer`)
    /// à la fin du corps — fulfill `ret` / reject la throw.
    fn settle_outer(&self, e: &mut Engine, r: std::result::Result<Value, Value>) {
        if std::env::var("DBG_DRIVE").is_ok() {
            eprintln!(
                "[settle] outer={:?} done={} r_ok={}",
                self.outer.is_some(),
                self.done,
                r.is_ok()
            );
        }
        if let Some(prom) = self.outer {
            match r {
                Ok(v) => e.resolve_promise(prom, v),
                Err(v) => e.reject_promise(prom, v),
            }
        }
    }

    /// Throw propagée : `try{…}catch` rattrape ; `finally`
    /// sans catch court puis la throw repart (`unwinding`).
    fn throw_into(&mut self, e: &mut Engine, v: Value) -> JsResult<()> {
        // La suspension courante est consommée par la throw —
        // sans ça la boucle réémettrait l'ancienne valeur.
        self.pending = None;
        loop {
            if std::env::var("DBG_DRIVE").is_ok() {
                eprintln!(
                    "[throw_into] stack={} kinds={:?}",
                    self.stack.len(),
                    self.stack
                        .iter()
                        .map(|f| std::mem::discriminant(&f.kind))
                        .collect::<Vec<_>>()
                );
            }
            let Some(f) = self.stack.pop() else {
                self.done = true;
                if std::env::var("DBG_DRIVE").is_ok() {
                    eprintln!("[uncaught] v={:?}", e.describe(v));
                }
                self.settle_outer(e, Err(v));
                return Err(Thrown(v));
            };
            match f.kind {
                FKind::Try {
                    catch: Some((pat, catch_body)),
                    finally,
                } => {
                    let cenv = Env::local(f.env.clone());
                    e.bind_pat(&cenv, self.this, &pat, v, BindMode::Lex { mutable: true })?;
                    if let Some(fin) = finally {
                        self.stack.push(GFrame {
                            list: Rc::new(fin),
                            pc: 0,
                            env: f.env.clone(),
                            labels: Vec::new(),
                            kind: FKind::Seq,
                        });
                    }
                    self.stack.push(GFrame {
                        list: Rc::new(catch_body),
                        pc: 0,
                        env: cenv,
                        labels: Vec::new(),
                        kind: FKind::Seq,
                    });
                    return Ok(());
                }
                FKind::Try {
                    catch: None,
                    finally: Some(fin),
                } => {
                    self.unwinding = Some(v);
                    self.stack.push(GFrame {
                        list: Rc::new(fin),
                        pc: 0,
                        env: f.env,
                        labels: Vec::new(),
                        kind: FKind::Seq,
                    });
                    return Ok(());
                }
                _ => {}
            }
        }
    }

    /// `finally` d'un frame sortant (break/continue).
    fn run_finally(&mut self, _e: &mut Engine, f: &GFrame) -> JsResult<()> {
        if let FKind::Try {
            finally: Some(fin), ..
        } = &f.kind
        {
            self.stack.push(GFrame {
                list: Rc::new(fin.clone()),
                pc: 0,
                env: f.env.clone(),
                labels: Vec::new(),
                kind: FKind::Seq,
            });
        }
        Ok(())
    }

    /// Fin de liste : boucles re-testent/re-bindent, `try`
    /// court `finally`, `unwinding` repart.
    fn leave_frame(&mut self, e: &mut Engine) -> JsResult<()> {
        if let Some(v) = self.unwinding.take() {
            return self.throw_into(e, v);
        }
        let Some(f) = self.stack.last() else {
            return Ok(());
        };
        // Wedge gov.uk (bissection bis) : une boucle dans un
        // corps lowered (`await` → GenLive) tournait hors budget
        // — cette continuation de frame est le point d'itération
        // (`While`/`For` réévaluent cond, `ForIter`/`ForAwait`
        // avancent) ; le tick la compte.
        e.tick()?;
        match &f.kind {
            FKind::Seq | FKind::Switch => {
                self.stack.pop();
            }
            FKind::While { cond } => {
                let (env, cond) = (f.env.clone(), cond.clone());
                let c = e.eval_expr(&env, self.this, &cond)?;
                if e.to_boolean(c) {
                    self.stack.last_mut().unwrap().pc = 0;
                } else {
                    self.stack.pop();
                }
            }
            FKind::For { test, update } => {
                let (env, test, update) = (f.env.clone(), test.clone(), update.clone());
                if let Some(u) = &update {
                    e.eval_expr(&env, self.this, u)?;
                }
                let cont = match &test {
                    Some(t) => {
                        let c = e.eval_expr(&env, self.this, t)?;
                        e.to_boolean(c)
                    }
                    None => true,
                };
                if cont {
                    self.stack.last_mut().unwrap().pc = 0;
                } else {
                    self.stack.pop();
                }
            }
            FKind::ForIter { items, idx, target } => {
                let (env, items, idx, target) =
                    (f.env.clone(), items.clone(), *idx, target.clone());
                if idx >= items.len() {
                    self.stack.pop();
                    return Ok(());
                }
                let v = items[idx];
                if let Err(Thrown(v)) = self.bind_for_target(e, &env, &target, v) {
                    return self.throw_into(e, v);
                }
                if let Some(f) = self.stack.last_mut() {
                    if let FKind::ForIter { idx, .. } = &mut f.kind {
                        *idx += 1;
                    }
                    f.pc = 0;
                }
            }
            // `for await` — un `it.next()` par itération ; le
            // `{value,done}` retourné est awaited : pendante →
            // suspend (`issued` + `__awaitit`), rejetée → throw
            // dans le corps, fulfill → bind cible ou `done`.
            FKind::ForAwait {
                iter,
                next,
                target,
                issued,
            } => {
                let (env, it, next, target, issued) =
                    (f.env.clone(), *iter, *next, target.clone(), *issued);
                if !issued {
                    let r = match e.call_value(next, it, &[]) {
                        Ok(v) => v,
                        Err(Thrown(v)) => return self.throw_into(e, v),
                    };
                    let flow = self.arm_await(e, r, &env, Some("__awaitit".into()))?;
                    // `Susp` : rejet rattrapé — les frames sont
                    // remplacées (catch), ne pas consommer.
                    if self.pending.is_some() || matches!(flow, GFlow::Susp) {
                        if self.pending.is_some() {
                            if let Some(fr) = self.stack.last_mut() {
                                if let FKind::ForAwait { issued, .. } = &mut fr.kind {
                                    *issued = true;
                                }
                            }
                        }
                        return Ok(());
                    }
                }
                // `__awaitit` = résultat `{value,done}` (lié
                // inline ou par `liver` à la reprise).
                let r = e.env_get(&env, "__awaitit").unwrap_or(Value::Undefined);
                let Value::Obj(id) = r else {
                    let Thrown(v) = e.type_error("iterator result is not an object");
                    return self.throw_into(e, v);
                };
                let done = e.get_prop(id, "done").is_some_and(|d| e.to_boolean(d));
                if done {
                    self.stack.pop();
                    return Ok(());
                }
                let val = e.get_prop(id, "value").unwrap_or(Value::Undefined);
                if let Err(Thrown(v)) = self.bind_for_target(e, &env, &target, val) {
                    return self.throw_into(e, v);
                }
                if let Some(fr) = self.stack.last_mut() {
                    if let FKind::ForAwait { issued, .. } = &mut fr.kind {
                        *issued = false;
                    }
                    fr.pc = 0;
                }
            }
            FKind::Try { finally, .. } => {
                let (env, fin) = (f.env.clone(), finally.clone());
                self.stack.pop();
                if let Some(fin) = fin {
                    self.stack.push(GFrame {
                        list: Rc::new(fin),
                        pc: 0,
                        env,
                        labels: Vec::new(),
                        kind: FKind::Seq,
                    });
                }
            }
        }
        Ok(())
    }

    /// `await` sur une valeur déjà évaluée — promesse
    /// pendante → suspend et arme les réactions de reprise ;
    /// settled → livre ou jette immédiatement ; non-promesse
    /// → livre la valeur. Partagé avec `for await` (le
    /// `next()` awaited, `into = __awaitit`).
    fn arm_await(
        &mut self,
        e: &mut Engine,
        v: Value,
        env: &EnvRef,
        into: Option<String>,
    ) -> JsResult<GFlow> {
        if let Value::Obj(pid) = v {
            let st = e.heap.obj(pid).and_then(|o| match &o.kind {
                ObjKind::Promise(p) => Some((p.state, p.result)),
                _ => None,
            });
            if let Some((state, result)) = st {
                match state {
                    crate::promise::PromiseState::Pending => {
                        // Réactions boundées `[gen_obj, outer]`
                        // — `BoundNative` prépose bound aux args
                        // → `async_resume_ok(gen, outer, v)`.
                        let gen = self.gen_obj;
                        let next = e.new_promise();
                        let ok_f = bound_resume(e, gen, self.outer, true);
                        let err_f = bound_resume(e, gen, self.outer, false);
                        if let Some(o) = e.heap.obj_mut(pid) {
                            if let ObjKind::Promise(p) = &mut o.kind {
                                p.reactions.push(crate::promise::Reaction {
                                    on_fulfilled: ok_f,
                                    on_rejected: err_f,
                                    next,
                                });
                            }
                        }
                        self.pending = Some(Pending::Await { into, prom: pid });
                        return Ok(GFlow::Adv);
                    }
                    crate::promise::PromiseState::Fulfilled => {
                        if let Some(n) = into {
                            Engine::env_put(env, &n, result, true);
                        }
                    }
                    crate::promise::PromiseState::Rejected => {
                        // Catch trouvée → frames remplacées →
                        // pas d'avance pc (le `get_mut(depth-1)`
                        // viserait la catch poussée).
                        self.throw_into(e, result)?;
                        return Ok(GFlow::Susp);
                    }
                }
                return Ok(GFlow::Adv);
            }
        }
        if let Some(n) = into {
            Engine::env_put(env, &n, v, true);
        }
        Ok(GFlow::Adv)
    }

    /// `await e` — évalue `e` puis [`Self::arm_await`].
    fn exec_await(
        &mut self,
        e: &mut Engine,
        a: &Expr,
        env: &EnvRef,
        into: Option<String>,
    ) -> JsResult<GFlow> {
        let v = e.eval_expr(env, self.this, a)?;
        self.arm_await(e, v, env, into)
    }

    /// Lie `for (t of/in it)` à `v`.
    fn bind_for_target(
        &self,
        e: &mut Engine,
        env: &EnvRef,
        t: &ForTarget,
        v: Value,
    ) -> JsResult<()> {
        match t {
            ForTarget::Var(p) | ForTarget::Lexical { pat: p, .. } => {
                e.bind_pat(env, self.this, p, v, BindMode::Var)
            }
            ForTarget::Expr(x) => {
                if let Some(lv) = e.resolve_lvalue(env, self.this, x)? {
                    e.store_lvalue(env, lv, v)?;
                }
                Ok(())
            }
            ForTarget::Using { .. } => Ok(()),
        }
    }

    /// La liste d'un corps (`Block` → ses stmts, autre →
    /// singleton).
    fn body_list(s: &Stmt) -> Vec<Stmt> {
        match s {
            Stmt::Block(l) => l.clone(),
            other => vec![other.clone()],
        }
    }

    /// Exécute un statement du frame courant : positions
    /// suspendues (`let x = yield e`, `yield e;`, `await e;`)
    /// → `pending` ; composés porteurs → frame ; le reste →
    /// `exec_stmt` (Flow mappé).
    fn exec_one(&mut self, e: &mut Engine, s: &Stmt, env: &EnvRef) -> JsResult<GFlow> {
        // Composé sans aucun suspend dans son sous-arbre →
        // `exec_stmt` direct (le chemin amont, testé) ; la
        // pile de frames ne sert qu'à porter les suspensions.
        if !stmt_has_suspend(s) && !matches!(s, Stmt::Expr(Expr::Yield { .. })) {
            return match e.exec_stmt(env, self.this, s)? {
                Flow::Normal => Ok(GFlow::Adv),
                Flow::Return(v) => Ok(GFlow::Ret(v)),
                Flow::Break(l) => Ok(GFlow::Brk(l)),
                Flow::Continue(l) => Ok(GFlow::Cont(l)),
            };
        }
        match s {
            // `await e;` (statement nu — binding ignoré).
            Stmt::Expr(Expr::Await(a)) => self.exec_await(e, a, env, None),
            // `yield e;` (statement nu — binding ignoré).
            Stmt::Expr(Expr::Yield { arg, delegate }) => {
                let v = match arg {
                    Some(a) => e.eval_expr(env, self.this, a)?,
                    None => Value::Undefined,
                };
                if *delegate {
                    let items = e.iter_values(v)?;
                    self.pending = Some(Pending::Drain {
                        items,
                        idx: 0,
                        into: None,
                    });
                } else {
                    self.emit = v;
                    self.pending = Some(Pending::Yield { into: None });
                }
                Ok(GFlow::Adv)
            }
            // `let/var/const x = yield e|await e` — suspension
            // binding-directe (post-lowering).
            Stmt::Var(decls) | Stmt::Lexical { decls, .. } => {
                for d in decls {
                    match &d.init {
                        Some(Expr::Await(a)) => {
                            let name = match &d.pat {
                                Pat::Ident(n) => Some(n.clone()),
                                _ => None,
                            };
                            return self.exec_await(e, a, env, name);
                        }
                        Some(Expr::Yield { arg, delegate }) => {
                            let v = match arg {
                                Some(a) => e.eval_expr(env, self.this, a)?,
                                None => Value::Undefined,
                            };
                            let name = match &d.pat {
                                Pat::Ident(n) => Some(n.clone()),
                                _ => None,
                            };
                            if *delegate {
                                let items = e.iter_values(v)?;
                                self.pending = Some(Pending::Drain {
                                    items,
                                    idx: 0,
                                    into: name,
                                });
                            } else {
                                self.emit = v;
                                self.pending = Some(Pending::Yield { into: name });
                            }
                            return Ok(GFlow::Adv);
                        }
                        _ => {
                            // init plain (déjà lifté) —
                            // binding par exec_stmt habituel.
                            e.exec_stmt(env, self.this, &Stmt::Var(vec![d.clone()]))?;
                        }
                    }
                }
                Ok(GFlow::Adv)
            }
            // `return e` — fin du corps.
            Stmt::Return(x) => {
                let v = match x {
                    Some(e2) => e.eval_expr(env, self.this, e2)?,
                    None => Value::Undefined,
                };
                Ok(GFlow::Ret(v))
            }
            // Composés porteurs de suspensions → frames.
            Stmt::If { cond, then, else_ } => {
                let c = e.eval_expr(env, self.this, cond)?;
                let branch = if e.to_boolean(c) {
                    Some(&**then)
                } else {
                    else_.as_deref()
                };
                if let Some(b) = branch {
                    self.stack.push(GFrame {
                        list: Rc::new(Self::body_list(b)),
                        pc: 0,
                        env: Env::local(env.clone()),
                        labels: Vec::new(),
                        kind: FKind::Seq,
                    });
                }
                Ok(GFlow::Adv)
            }
            Stmt::Block(l) => {
                self.stack.push(GFrame {
                    list: Rc::new(l.clone()),
                    pc: 0,
                    env: Env::local(env.clone()),
                    labels: Vec::new(),
                    kind: FKind::Seq,
                });
                Ok(GFlow::Adv)
            }
            Stmt::While { cond, body } => {
                let c = e.eval_expr(env, self.this, cond)?;
                if e.to_boolean(c) {
                    self.stack.push(GFrame {
                        list: Rc::new(Self::body_list(body)),
                        pc: 0,
                        env: Env::local(env.clone()),
                        labels: Vec::new(),
                        kind: FKind::While { cond: cond.clone() },
                    });
                }
                Ok(GFlow::Adv)
            }
            Stmt::DoWhile { body, cond } => {
                self.stack.push(GFrame {
                    list: Rc::new(Self::body_list(body)),
                    pc: 0,
                    env: Env::local(env.clone()),
                    labels: Vec::new(),
                    kind: FKind::While { cond: cond.clone() },
                });
                Ok(GFlow::Adv)
            }
            Stmt::For {
                init,
                test,
                update,
                body,
            } => {
                if let Some(i) = init {
                    let st = match i {
                        crate::ast::ForInit::Expr(x) => Stmt::Expr(x.clone()),
                        crate::ast::ForInit::Var(d) => Stmt::Var(d.clone()),
                        crate::ast::ForInit::Lexical { decls, constant } => Stmt::Lexical {
                            decls: decls.clone(),
                            constant: *constant,
                        },
                    };
                    e.exec_stmt(env, self.this, &st)?;
                }
                let cont = match test {
                    Some(t) => {
                        let c = e.eval_expr(env, self.this, t)?;
                        e.to_boolean(c)
                    }
                    None => true,
                };
                if cont {
                    self.stack.push(GFrame {
                        list: Rc::new(Self::body_list(body)),
                        pc: 0,
                        env: Env::local(env.clone()),
                        labels: Vec::new(),
                        kind: FKind::For {
                            test: test.clone(),
                            update: update.clone(),
                        },
                    });
                }
                Ok(GFlow::Adv)
            }
            Stmt::ForOf {
                target,
                iter,
                body,
                is_await,
            } => {
                let v = e.eval_expr(env, self.this, iter)?;
                // `for await` — itérateur async, suspension par
                // pas : le frame part « consommé » (`pc` en fin
                // de liste), `leave_frame` émet le premier
                // `it.next()`.
                if *is_await {
                    let (it, next) = async_iterator(e, v)?;
                    let list = Self::body_list(body);
                    self.stack.push(GFrame {
                        pc: list.len(),
                        list: Rc::new(list),
                        env: Env::local(env.clone()),
                        labels: Vec::new(),
                        kind: FKind::ForAwait {
                            iter: it,
                            next,
                            target: target.clone(),
                            issued: false,
                        },
                    });
                    return Ok(GFlow::Adv);
                }
                let items = e.iter_values(v)?;
                if items.is_empty() {
                    return Ok(GFlow::Adv);
                }
                self.bind_for_target(e, env, target, items[0])?;
                let benv = Env::local(env.clone());
                self.stack.push(GFrame {
                    list: Rc::new(Self::body_list(body)),
                    pc: 0,
                    env: benv,
                    labels: Vec::new(),
                    kind: FKind::ForIter {
                        items,
                        idx: 1,
                        target: target.clone(),
                    },
                });
                Ok(GFlow::Adv)
            }
            Stmt::ForIn {
                target,
                obj: iter,
                body,
                ..
            } => {
                let v = e.eval_expr(env, self.this, iter)?;
                let items = match v {
                    Value::Obj(id) => e.enum_keys(id)?.into_iter().map(|k| e.str_val(k)).collect(),
                    _ => Vec::new(),
                };
                if items.is_empty() {
                    return Ok(GFlow::Adv);
                }
                self.bind_for_target(e, env, target, items[0])?;
                let benv = Env::local(env.clone());
                self.stack.push(GFrame {
                    list: Rc::new(Self::body_list(body)),
                    pc: 0,
                    env: benv,
                    labels: Vec::new(),
                    kind: FKind::ForIter {
                        items,
                        idx: 1,
                        target: target.clone(),
                    },
                });
                Ok(GFlow::Adv)
            }
            Stmt::Try {
                block,
                catch,
                finally,
            } => {
                self.stack.push(GFrame {
                    list: Rc::new(block.clone()),
                    pc: 0,
                    env: Env::local(env.clone()),
                    labels: Vec::new(),
                    kind: FKind::Try {
                        catch: catch.clone(),
                        finally: finally.clone(),
                    },
                });
                Ok(GFlow::Adv)
            }
            Stmt::Labeled { name, body } => {
                self.stack.push(GFrame {
                    list: Rc::new(Self::body_list(body)),
                    pc: 0,
                    env: Env::local(env.clone()),
                    labels: vec![name.clone()],
                    kind: FKind::Seq,
                });
                Ok(GFlow::Adv)
            }
            // `switch` porteur de suspensions : le discriminant
            // et les tests sont purs post-lowering (les suspends
            // de `case (await e)` ont été hissés éagerment). Le
            // case atteint est aplati avec tous les suivants —
            // fallthrough = liste plate, `break` s'arrête au
            // frame `Switch` (`do_break`).
            Stmt::Switch { disc, cases } => {
                let dv = e.eval_expr(env, self.this, disc)?;
                let mut start = None;
                let mut default = None;
                for (i, c) in cases.iter().enumerate() {
                    match &c.test {
                        Some(t) => {
                            let tv = e.eval_expr(env, self.this, t)?;
                            if start.is_none() && e.strict_eq(dv, tv) {
                                start = Some(i);
                            }
                        }
                        None => {
                            if default.is_none() {
                                default = Some(i);
                            }
                        }
                    }
                }
                // Spec : `default` ne s'exécute que si aucun
                // test n'a matché (pas « case + default »).
                if let Some(from) = start.or(default) {
                    let mut flat = Vec::new();
                    for c in &cases[from..] {
                        flat.extend(c.body.iter().cloned());
                    }
                    if !flat.is_empty() {
                        self.stack.push(GFrame {
                            list: Rc::new(flat),
                            pc: 0,
                            env: Env::local(env.clone()),
                            labels: Vec::new(),
                            kind: FKind::Switch,
                        });
                    }
                }
                Ok(GFlow::Adv)
            }
            Stmt::Break(l) => Ok(GFlow::Brk(l.clone())),
            Stmt::Continue(l) => Ok(GFlow::Cont(l.clone())),
            // Le reste : non-suspendu post-lowering — `exec_stmt`.
            other => match e.exec_stmt(env, self.this, other)? {
                Flow::Normal => Ok(GFlow::Adv),
                Flow::Return(v) => Ok(GFlow::Ret(v)),
                Flow::Break(l) => Ok(GFlow::Brk(l)),
                Flow::Continue(l) => Ok(GFlow::Cont(l)),
            },
        }
    }
}

// ------------------------------------------------------------------
// 3. Objet générateur live + natives
// ------------------------------------------------------------------

impl GExec {
    /// Racines GC : envs des frames, `this`, `ret`, `emit`,
    /// items en file (`yield*`/`for-of`).
    pub(crate) fn trace(&self, out: &mut Vec<ObjId>, visited: &mut HashSet<usize>) {
        fn pv(v: &Value, out: &mut Vec<ObjId>) {
            if let Value::Obj(id) = v {
                out.push(*id);
            }
        }
        pv(&self.this, out);
        pv(&self.ret, out);
        pv(&self.emit, out);
        for f in &self.stack {
            crate::heap::trace_env(&f.env, out, visited);
            if let FKind::ForIter { items, .. } = &f.kind {
                for v in items {
                    pv(v, out);
                }
            }
            if let FKind::ForAwait { iter, next, .. } = &f.kind {
                pv(iter, out);
                pv(next, out);
            }
        }
        if let Some(Pending::Drain { items, .. }) = &self.pending {
            for v in items {
                pv(v, out);
            }
        }
        // La promesse awaited est une racine — sans elle le GC
        // libère l'objet pendant la suspension.
        if let Some(Pending::Await { prom, .. }) = &self.pending {
            out.push(*prom);
        }
        pv(&self.gen_obj, out);
        if let Some(p) = &self.outer {
            out.push(*p);
        }
        if let Some(r) = &self.resume {
            match r {
                Ok(v) | Err(v) => pv(v, out),
            }
        }
        if let Some(v) = &self.unwinding {
            pv(v, out);
        }
    }
}

/// `for await` — itérateur asynchrone : `@@asyncIterator`
/// d'abord, repli `@@iterator` (spec
/// `CreateAsyncFromSyncIterator` — chaque `next()` est
/// awaited, un itérateur sync se comporte à l'identique
/// puisque ses `{value,done}` ne sont jamais des promesses
/// pendantes). `(iter, next_method)` ou `TypeError`.
fn async_iterator(e: &mut Engine, v: Value) -> JsResult<(Value, Value)> {
    if let Value::Obj(id) = v {
        for sym in [
            crate::interp::SYM_ASYNC_ITERATOR,
            crate::interp::SYM_ITERATOR,
        ] {
            if let Some(m) = e.get_prop(id, sym) {
                if e.is_callable(m) {
                    let it = e.call_value(m, v, &[])?;
                    if let Value::Obj(iid) = it {
                        if let Some(next) = e.get_prop(iid, "next") {
                            if e.is_callable(next) {
                                return Ok((it, next));
                            }
                        }
                    }
                    return Err(e.type_error("iterator.next is not a function"));
                }
            }
        }
    }
    Err(e.type_error("object is not async iterable"))
}

/// `function*`/`async fn` appelée → porteur `GenLive` de
/// l'exécuteur (`gen_obj` posé pour que les réactions
/// `await` s'y boundent — racine GC vivante).
pub(crate) fn gen_lazy_obj(e: &mut Engine, env: EnvRef, this: Value, body: Rc<Vec<Stmt>>) -> Value {
    let exec = Rc::new(RefCell::new(GExec::new(env, this, body)));
    let id = e.heap.alloc_obj(Object {
        proto: Value::Obj(e.protos.iterator),
        props: Vec::new(),
        kind: ObjKind::GenLive { exec },
        extensible: true,
        elem_integrity: 0,
        args_map: None,
    });
    let v = Value::Obj(id);
    if let Some(o) = e.heap.obj(id) {
        if let ObjKind::GenLive { exec } = &o.kind {
            exec.borrow_mut().gen_obj = v;
        }
    }
    v
}

/// `BoundNative(async_resume_*)` boundé `[gen_obj, outer]` —
/// la réaction `then` d'un `await` pendante.
fn bound_resume(e: &mut Engine, gen: Value, outer: Option<ObjId>, ok: bool) -> Value {
    let bound = vec![gen, outer.map_or(Value::Undefined, Value::Obj)];
    let f = if ok {
        async_resume_ok as NativeFn
    } else {
        async_resume_err as NativeFn
    };
    let id = e.heap.alloc_obj(Object {
        proto: Value::Obj(e.protos.function),
        props: Vec::new(),
        kind: ObjKind::Fun(Fun::BoundNative {
            name: "async_resume",
            f,
            bound,
        }),
        extensible: true,
        elem_integrity: 0,
        args_map: None,
    });
    Value::Obj(id)
}

/// Réaction fulfill — reprend le frame avec `v`.
fn async_resume_ok(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    resume_common(e, args, true)
}

/// Réaction reject — jette `v` au site suspendu.
fn async_resume_err(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    resume_common(e, args, false)
}

/// `args` = `[gen_obj, outer, v]` (bound préposés).
fn resume_common(e: &mut Engine, args: &[Value], ok: bool) -> JsResult<Value> {
    let (gen, v) = (arg(args, 0), arg(args, 2));
    if std::env::var("DBG_DRIVE").is_ok() {
        eprintln!("[resume] ok={ok} gen={gen:?}");
    }
    if let Value::Obj(gid) = gen {
        if let Some(o) = e.heap.obj(gid) {
            if let ObjKind::GenLive { exec } = &o.kind {
                let exec = exec.clone();
                let r = if ok { Ok(v) } else { Err(v) };
                let _ = exec.borrow_mut().resume_await(e, r);
            }
        }
    }
    Ok(Value::Undefined)
}
