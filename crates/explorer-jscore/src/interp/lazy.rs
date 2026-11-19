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
//!    `&&`/`||`/`??`/`?:` hissent leur suspend **sous garde**
//!    (`if` dans les pré-statements — paresse réelle, le
//!    suspend ne se lève que si la branche tire) ; `?.` et les
//!    positions hors-expression (cases `switch`, défauts de
//!    motifs, `catch`) restent hissées éagerment — déviation
//!    documentée (rare).
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
    // JS-C400 — arête ∝ profondeur AST : le lowering tourne
    // **avant** le `maybe_grow` d'`exec_body`, sur le segment
    // courant — un corps profond débordait indifféremment de
    // la pile native (§B-111, vol 687). Chaque arête re-bascule.
    stacker::maybe_grow(
        crate::parser::STACK_RED_ZONE,
        crate::parser::STACK_GROW,
        || has_suspend_inner(e),
    )
}

fn has_suspend_inner(e: &Expr) -> bool {
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
        Expr::Call(f, args, _) | Expr::New(f, args, _) => {
            has_suspend(f) || args.iter().any(has_suspend)
        }
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
    fn fresh(&mut self) -> std::rc::Rc<str> {
        self.n += 1;
        crate::intern(&format!("__g{}", self.n))
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
    // JS-C400 — arête ∝ profondeur AST (voir `has_suspend`).
    stacker::maybe_grow(
        crate::parser::STACK_RED_ZONE,
        crate::parser::STACK_GROW,
        || lift_inner(e, pre, cx),
    );
}

fn lift_inner(e: &mut Expr, pre: &mut Vec<Stmt>, cx: &mut LowerCx) {
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
        // Positions paresseuses `&&`/`||`/`??` portant un
        // suspend : hissage **sous garde** — `var __gN = l ;
        // if (<garde>) { <pre_r> ; __gN = r' }`, le suspend
        // ne se lève que si la branche tire (vol 682, bundle
        // xjs Google : l'évaluation éagère de
        // `a.TRa && b(await c(a.TRa(…)))` appelait `a.TRa(…)`
        // = undefined → `is not a function`).
        Expr::Binary(op @ (BinOp::And | BinOp::Or | BinOp::Nullish), l, r) => {
            lift(l, pre, cx);
            if has_suspend(r) {
                let n = cx.fresh();
                let mut pre_r = Vec::new();
                lift(r, &mut pre_r, cx);
                pre_r.push(Stmt::Expr(Expr::Assign(
                    Box::new(Expr::Ident(n.clone())),
                    AssignOp::Set,
                    Box::new(std::mem::replace(r.as_mut(), Expr::Null)),
                )));
                let cond = match op {
                    // `l && r` : la branche tire quand `l` est truthy.
                    BinOp::And => Expr::Ident(n.clone()),
                    // `l || r` : quand `l` est falsy.
                    BinOp::Or => {
                        Expr::Unary(crate::ast::UnOp::Not, Box::new(Expr::Ident(n.clone())))
                    }
                    // `l ?? r` : `l == null` (loose — null ET undefined).
                    _ => Expr::Binary(
                        BinOp::Eq,
                        Box::new(Expr::Ident(n.clone())),
                        Box::new(Expr::Null),
                    ),
                };
                pre.push(Stmt::Var(vec![VarDecl {
                    pat: Pat::Ident(n.clone()),
                    init: Some(std::mem::replace(l.as_mut(), Expr::Null)),
                }]));
                pre.push(Stmt::If {
                    cond,
                    then: Box::new(Stmt::Block(pre_r)),
                    else_: None,
                });
                *e = Expr::Ident(n);
            } else {
                lift(r, pre, cx);
            }
        }
        Expr::Binary(_, l, r) | Expr::Assign(l, _, r) | Expr::Index(l, r) => {
            lift(l, pre, cx);
            lift(r, pre, cx);
        }
        Expr::Cond(c, t, e2) => {
            lift(c, pre, cx);
            if has_suspend(t) || has_suspend(e2) {
                // `c ? t : f` avec suspend → `var __gN = c ;
                // if (__gN) { <pre_t> ; __gN = t' } else
                // { <pre_f> ; __gN = f' }` — `__gN` porte la
                // valeur du test tant que la branche choisie
                // l'écrase.
                let n = cx.fresh();
                let mut pre_t = Vec::new();
                lift(t, &mut pre_t, cx);
                pre_t.push(Stmt::Expr(Expr::Assign(
                    Box::new(Expr::Ident(n.clone())),
                    AssignOp::Set,
                    Box::new(std::mem::replace(t.as_mut(), Expr::Null)),
                )));
                let mut pre_f = Vec::new();
                lift(e2, &mut pre_f, cx);
                pre_f.push(Stmt::Expr(Expr::Assign(
                    Box::new(Expr::Ident(n.clone())),
                    AssignOp::Set,
                    Box::new(std::mem::replace(e2.as_mut(), Expr::Null)),
                )));
                pre.push(Stmt::Var(vec![VarDecl {
                    pat: Pat::Ident(n.clone()),
                    init: Some(std::mem::replace(c.as_mut(), Expr::Null)),
                }]));
                pre.push(Stmt::If {
                    cond: Expr::Ident(n.clone()),
                    then: Box::new(Stmt::Block(pre_t)),
                    else_: Some(Box::new(Stmt::Block(pre_f))),
                });
                *e = Expr::Ident(n);
            } else {
                lift(t, pre, cx);
                lift(e2, pre, cx);
            }
        }
        Expr::Call(f, args, _) | Expr::New(f, args, _) => {
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
        Expr::Seq(items) => {
            if items.iter().any(has_suspend) {
                // §B-118 — une virgule évalue **à son tour** :
                // chaque opérande est éclaté en statement ordonné
                // (`lift` de l'opérande posé juste avant lui).
                // Le hissage global évaluait l'argument d'un
                // `await` avant les affectations intermédiaires —
                // `(await a, n = f(), z = await n.p)` lisait `n.p`
                // avec `n` encore undefined (pluto.tv : `ef`
                // hydratation — `n = createRouter` sauté →
                // `next.router` jamais posé).
                let n = cx.fresh();
                pre.push(Stmt::Var(vec![VarDecl {
                    pat: Pat::Ident(n.clone()),
                    init: None,
                }]));
                for it in items.iter_mut() {
                    lift(it, pre, cx);
                    pre.push(Stmt::Expr(Expr::Assign(
                        Box::new(Expr::Ident(n.clone())),
                        AssignOp::Set,
                        Box::new(std::mem::replace(it, Expr::Null)),
                    )));
                }
                *e = Expr::Ident(n);
            } else {
                for it in items {
                    lift(it, pre, cx);
                }
            }
        }
        Expr::SuperCall(items) => {
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
    // JS-C400 — arête ∝ profondeur du motif (voir `has_suspend`).
    stacker::maybe_grow(
        crate::parser::STACK_RED_ZONE,
        crate::parser::STACK_GROW,
        || has_suspend_pat_inner(p),
    )
}

fn has_suspend_pat_inner(p: &Pat) -> bool {
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
    // JS-C400 — arête ∝ profondeur du motif (voir `has_suspend`).
    stacker::maybe_grow(
        crate::parser::STACK_RED_ZONE,
        crate::parser::STACK_GROW,
        || lift_pat_inner(p, pre, cx),
    );
}

fn lift_pat_inner(p: &mut Pat, pre: &mut Vec<Stmt>, cx: &mut LowerCx) {
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

/// Réécrit une liste de déclarateurs en séquence **ordonnée** :
/// un déclarateur dont le pattern ou l'init contient un suspend
/// voit ses lifts `let __gN` émis APRÈS les déclarateurs précédents
/// — sinon `const e = X, t = await f(e)` lirait `e` hors de sa
/// portée (init hissée avant `e` → capture silencieuse du `e`
/// externe ; observé terrain : `fetch(module)` sur dailymotion —
/// sans TDZ, la résolution retombe sur le scope englobant).
fn lower_decls_ordered(
    decls: Vec<VarDecl>,
    mk: &dyn Fn(Vec<VarDecl>) -> Stmt,
    cx: &mut LowerCx,
) -> Vec<Stmt> {
    let mut out = Vec::new();
    let mut pending: Vec<VarDecl> = Vec::new();
    for mut d in decls {
        if (d.init.as_ref().is_some_and(has_suspend) || has_suspend_pat(&d.pat))
            && !pending.is_empty()
        {
            out.push(mk(std::mem::take(&mut pending)));
        }
        lift_pat(&mut d.pat, &mut out, cx);
        if let Some(i) = &mut d.init {
            lift(i, &mut out, cx);
        }
        pending.push(d);
    }
    if !pending.is_empty() {
        out.push(mk(pending));
    }
    out
}

/// Réécrit un statement : suspendables hissés en déclarations
/// `let __gN = …` posées AVANT ; le statement utilise `__gN`.
fn lower_stmt(s: &Stmt, cx: &mut LowerCx) -> Vec<Stmt> {
    let mut pre = Vec::new();
    let mut s2 = s.clone();
    match &mut s2 {
        Stmt::Expr(e) | Stmt::Return(Some(e)) | Stmt::Throw(e) => lift(e, &mut pre, cx),
        Stmt::Var(ds) => {
            let ds = std::mem::take(ds);
            return lower_decls_ordered(ds, &Stmt::Var, cx);
        }
        Stmt::Lexical { decls, constant } => {
            let ds = std::mem::take(decls);
            let c = *constant;
            return lower_decls_ordered(
                ds,
                &move |seg| Stmt::Lexical {
                    decls: seg,
                    constant: c,
                },
                cx,
            );
        }
        Stmt::Using { decls, is_await } => {
            let ds = std::mem::take(decls);
            let a = *is_await;
            return lower_decls_ordered(
                ds,
                &move |seg| Stmt::Using {
                    decls: seg,
                    is_await: a,
                },
                cx,
            );
        }

        Stmt::If { cond, .. } | Stmt::While { cond, .. } | Stmt::DoWhile { cond, .. } => {
            lift(cond, &mut pre, cx);
        }
        Stmt::For {
            init, test, update, ..
        } => {
            if let Some(i) = init {
                // Ordre : un suspend dans les déclarateurs impose le
                // découpage ordonné — la clause init du `for` ne
                // pouvant se scinder, les déclarations migrent avant
                // la boucle (déviation : la portée for-init s'élargit
                // au bloc englobant).
                let susp = |ds: &[VarDecl]| {
                    ds.iter().any(|d| {
                        d.init.as_ref().is_some_and(has_suspend) || has_suspend_pat(&d.pat)
                    })
                };
                match i {
                    ForInit::Expr(e) => lift(e, &mut pre, cx),
                    ForInit::Var(ds) => {
                        if susp(ds) {
                            let ds = std::mem::take(ds);
                            pre.extend(lower_decls_ordered(ds, &Stmt::Var, cx));
                            *init = None;
                        } else {
                            lift_decls(ds, &mut pre, cx);
                        }
                    }
                    ForInit::Lexical { decls, constant } => {
                        if susp(decls) {
                            let ds = std::mem::take(decls);
                            let c = *constant;
                            pre.extend(lower_decls_ordered(
                                ds,
                                &move |seg| Stmt::Lexical {
                                    decls: seg,
                                    constant: c,
                                },
                                cx,
                            ));
                            *init = None;
                        } else {
                            lift_decls(decls, &mut pre, cx);
                        }
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
                // Chaque segment reste un `export` — les lifts
                // `__gN` intercalés ne le sont pas.
                Stmt::Var(ds) => {
                    let ds = std::mem::take(ds);
                    let ex = |seg| Stmt::Export(ExportDecl::Decl(Box::new(Stmt::Var(seg))));
                    return lower_decls_ordered(ds, &ex, cx);
                }
                Stmt::Lexical { decls, constant } => {
                    let ds = std::mem::take(decls);
                    let c = *constant;
                    let ex = move |seg| {
                        Stmt::Export(ExportDecl::Decl(Box::new(Stmt::Lexical {
                            decls: seg,
                            constant: c,
                        })))
                    };
                    return lower_decls_ordered(ds, &ex, cx);
                }
                Stmt::Using { decls, is_await } => {
                    let ds = std::mem::take(decls);
                    let a = *is_await;
                    let ex = move |seg| {
                        Stmt::Export(ExportDecl::Decl(Box::new(Stmt::Using {
                            decls: seg,
                            is_await: a,
                        })))
                    };
                    return lower_decls_ordered(ds, &ex, cx);
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
    // JS-C400 — arête ∝ profondeur du corps (voir `has_suspend`).
    stacker::maybe_grow(
        crate::parser::STACK_RED_ZONE,
        crate::parser::STACK_GROW,
        || stmt_has_suspend_inner(s),
    )
}

fn stmt_has_suspend_inner(s: &Stmt) -> bool {
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
    // JS-C400 — arête ∝ imbrication des corps (voir `has_suspend`).
    stacker::maybe_grow(
        crate::parser::STACK_RED_ZONE,
        crate::parser::STACK_GROW,
        || lower_list_inner(list, cx),
    )
}

fn lower_list_inner(list: &[Stmt], cx: &mut LowerCx) -> Vec<Stmt> {
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
/// `src_body` = corps matérialisé (V-5a) — passé à part car un
/// corps différé n'est pas forcément un `&[Stmt]` derrière
/// `lit.body`.
pub(crate) fn lower_suspend_fn(lit: &FnLit, src_body: &[Stmt]) -> (Vec<Param>, Rc<Vec<Stmt>>) {
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
                (crate::intern(&format!("__param{i}")), true)
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
            let name: Rc<str> = crate::intern(&format!("__param{i}"));
            let orig = std::mem::replace(&mut p.pat, Pat::Ident(name.clone()));
            prelude.push(Stmt::Var(vec![VarDecl {
                pat: orig,
                init: Some(Expr::Ident(name)),
            }]));
        }
    }
    let mut body = prelude;
    body.extend(src_body.iter().cloned());
    (params, lower_suspend_body(&body))
}

/// Corps de générateur/async → liste lowerée partagée.
pub(crate) fn lower_suspend_body(body: &[Stmt]) -> Rc<Vec<Stmt>> {
    let mut cx = LowerCx { n: 0 };
    let out = Rc::new(lower_list(body, &mut cx));
    #[cfg(debug_assertions)]
    if crate::dbg_lower() {
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
    Yield { into: Option<Rc<str>> },
    /// `yield*` — file d'items en vidange.
    Drain {
        items: Vec<Value>,
        idx: usize,
        into: Option<Rc<str>>,
    },
    /// `await` pendante — la réaction `then` boundée reprend
    /// le frame (fulfill lie `into`, reject jette au site).
    Await { into: Option<Rc<str>>, prom: ObjId },
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
        /// L'itérateur — `IteratorClose` (`iter.return()`)
        /// sur sortie abrupte (`Undefined` pour les chemins
        /// rapides tableau/TypedArray — pas de `return`).
        iter: Value,
        items: Vec<Value>,
        idx: usize,
        target: ForTarget,
    },
    /// `for await (t of it)` — `it.next()` awaited par pas
    /// (spec `ForIn/OfBodyEvaluation` async) : `issued` marque
    /// un `next()` émis en attente de sa promesse (le résultat
    /// `{value,done}` revient lié à `__awaitit` à la reprise).
    /// `sync_iter` : venu du repli `@@iterator` — c'est alors
    /// `result.value` qui est awaited (`AsyncFromSyncIterator`).
    ForAwait {
        iter: Value,
        next: Value,
        target: ForTarget,
        issued: bool,
        sync_iter: bool,
    },
    /// `try` — throw → throw → catch si présent ; `finally`
    /// toujours.
    Try {
        catch: Option<(Pat, Vec<Stmt>)>,
        finally: Option<Vec<Stmt>>,
    },
    /// `finally` poussé en plein dépilement (`break`/
    /// `continue`/`return` traversant un `try`) — marqueur :
    /// à sa sortie naturelle, l'unwind enregistré reprend.
    UnwindFin,
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
    labels: Vec<Rc<str>>,
    kind: FKind,
}

/// Complétion à reprendre après un travail différé de
/// dépilement (`finally` drainé ou `iter.return()` awaited)
/// — `Done` : l'unwind est fini, l'exécution continue.
enum Unwind {
    Done,
    Brk(Option<Rc<str>>),
    Cont(Option<Rc<str>>),
    Throw(Value),
    /// `return(v)` — la valeur est dans `self.ret`.
    Ret,
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
    Brk(Option<Rc<str>>),
    /// `continue [l]`.
    Cont(Option<Rc<str>>),
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
    /// Complétion `return(v)` en vol (injectée par `.return()`) :
    /// les frames se dépilent — catch ignorée, `finally` court,
    /// `yield` dans un `finally` suspend à nouveau (spec
    /// `GeneratorResumeAbrupt`). Valeur portée par `ret`.
    ret_unwind: bool,
    /// Promesse retournée par l'`async fn` — settlée à la
    /// fin du corps (`return` → fulfill, throw → reject).
    /// `None` pour un générateur (`function*`).
    outer: Option<ObjId>,
    /// L'objet `ObjKind::GenLive` porteur de cet exec — les
    /// réactions `await` s'y boundent (racine GC).
    gen_obj: Value,
    /// File `AsyncGeneratorEnqueue` — `Some` pour un
    /// `async function*` : `next`/`return`/`throw` rendent
    /// des promesses servies FIFO (JS-C601 — le `{value,done}`
    /// synchrone cassait `it.next().then` sur ~3 656 tests
    /// test262).
    agen: Option<AgenState>,
    /// Unwind suspendu sur un travail différé (`finally`
    /// poussé ou `IteratorClose` awaited) — JS-C616.
    /// `bool` = un `iter.return()` awaited vient d'être
    /// livré — vérifier que `__awaitit` est un objet
    /// (spec `AsyncIteratorClose`).
    unwind: Option<(Unwind, bool)>,
}

/// Espèce de requête `%AsyncGenerator%` en file.
#[derive(Clone, Copy)]
pub(crate) enum AgenKind {
    Next,
    Return,
    Throw,
}

/// État `async function*` (spec §27.5 `AsyncGeneratorEnqueue`)
/// — file FIFO + drapeau `executing` : la requête de tête
/// reste en file pendant une suspension `await` et n'est
/// dépilée que lorsque le drive produit son `{value,done}`.
pub(crate) struct AgenState {
    queue: std::collections::VecDeque<(AgenKind, Value, ObjId)>,
    executing: bool,
}

/// `args[i]` ou `Undefined` — convention natives.
fn arg(args: &[Value], i: usize) -> Value {
    args.get(i).cloned().unwrap_or(Value::Undefined)
}

/// `{value, done}` résultat d'itération.
fn iter_result(e: &mut Engine, value: Value, done: bool) -> Value {
    let o = e.new_object();
    e.set_own(o, "value", value, true);
    e.set_own(o, "done", Value::Bool(done), true);
    Value::Obj(o)
}

impl GExec {
    /// Estimation pour `cell_bytes` (JS-C411) — la pile de
    /// contrôle suspendue d'un générateur/`async` gelé retient
    /// frames + envs vivants : hors compteur, elle était
    /// invisible aux bornes (réservoir §B-114).
    pub(crate) fn mem_estimate(&self) -> usize {
        self.stack.capacity() * std::mem::size_of::<GFrame>()
    }

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
            ret_unwind: false,
            unwind: None,
            outer: None,
            gen_obj: Value::Undefined,
            agen: None,
        }
    }

    /// Marque l'exécuteur `async function*` — la file
    /// `AsyncGeneratorEnqueue` pilote `next`/`return`/`throw`
    /// (promesses FIFO au lieu du `{value,done}` synchrone).
    pub(crate) fn mark_agen(&mut self) {
        self.agen = Some(AgenState {
            queue: std::collections::VecDeque::new(),
            executing: false,
        });
    }

    /// `true` pour un `async function*` (file
    /// `AsyncGeneratorEnqueue` active — JS-C601).
    pub(crate) fn is_agen(&self) -> bool {
        self.agen.is_some()
    }

    /// `true` si le corps attend la settlement d'un `await`
    /// — la pompe agen ne sert pas de nouvelle requête tant
    /// qu'une suspension est en vol.
    fn pending_await(&self) -> bool {
        matches!(self.pending, Some(Pending::Await { .. }))
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

    /// `return(v)` — spec `GeneratorResumeAbrupt` : la complétion
    /// return dépile la pile (catch ignorée, `finally` courue,
    /// `yield` en finally → `{value, done:false}`) ; pile vide
    /// → `{value:v, done:true}`.
    pub(crate) fn ret_at(&mut self, e: &mut Engine, v: Value) -> JsResult<Value> {
        if self.done {
            return Ok(iter_result(e, v, true));
        }
        self.ret = v;
        self.pending = None; // le yield suspendu est consommé
        self.ret_unwind = true;
        self.drive(e)
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
        // JS-C421 : tant que ce `GExec` tourne, son
        // `RefCell` est emprunté — `trace_cell` le saute
        // (`try_borrow` échoue) et un safepoint GC balayerait
        // ses envs/items vivants. On publie les racines dans
        // `safepoint_roots` le temps du drive (id → `Value::Obj`).
        let sr_mark = e.safepoint_roots.len();
        let mut ids: Vec<crate::value::ObjId> = Vec::new();
        self.trace(&mut ids, &mut std::collections::HashSet::new());
        e.safepoint_roots.extend(ids.into_iter().map(Value::Obj));
        let out = self.drive_inner(e);
        e.safepoint_roots.truncate(sr_mark);
        out
    }

    /// Corps de [`GExec::drive`] — isolé pour que le publish
    /// de racines safepoint (JS-C421) entoure tout le cycle.
    fn drive_inner(&mut self, e: &mut Engine) -> JsResult<Value> {
        if let Some(r) = self.resume.take() {
            match r {
                Ok(v) => self.liver(e, v)?,
                Err(v) => match self.unwind.take() {
                    // `iter.return()` awaited rejeté : la
                    // réjection se plie dans la complétion en
                    // vol — une throw externe garde la sienne
                    // (`AsyncIteratorClose`, JS-C616).
                    Some((u, true)) => self.resume_unwind(e, fold_close(u, v))?,
                    _ => self.throw_into(e, v)?,
                },
            }
        }
        loop {
            // Wedge gov.uk : une boucle dans un corps lowered
            // (`await` → `GenLive`) tournait sans jamais ticker
            // le budget — spin infini dans `eval_module`
            // (op-limit jamais atteint). Chaque pas du driver
            // compte désormais.
            e.tick()?;
            // `return(v)` en vol : dépiler la pile — catch
            // ignorée, `finally` courus, itérateurs clos
            // (spec `GeneratorResumeAbrupt`) ; les `yield` du
            // finally suspendront via le chemin `pending`
            // ordinaire, les suspends de close/finally via
            // `UnwindFin`.
            if self.ret_unwind {
                self.ret_unwind = false;
                self.start_unwind(e, Unwind::Ret)?;
            }
            // `yield*` en vidange : les items restants
            // s'émettent AVANT tout statement — sinon le pc
            // suivant (`Expr(__gN)` lifté) tournerait pendant
            // que `into` n'est pas encore livré.
            if self.pending.is_some() && self.drain_step(e)? {
                return Ok(iter_result(e, self.emit.clone(), false));
            }
            let Some(f) = self.stack.last() else {
                self.done = true;
                self.settle_outer(e, Ok(self.ret.clone()));
                return Ok(iter_result(e, self.ret.clone(), true));
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
            if crate::dbg_drive() {
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
                    // `return` dépile comme toute complétion
                    // abrupte : `finally` courus, itérateurs
                    // clos — la boucle `drive` finalise à pile
                    // vide (`self.ret`, JS-C616).
                    self.ret_unwind = false;
                    self.ret = v.clone();
                    self.start_unwind(e, Unwind::Ret)?;
                }
                Ok(GFlow::Brk(l)) => self.start_unwind(e, Unwind::Brk(l))?,
                Ok(GFlow::Cont(l)) => self.start_unwind(e, Unwind::Cont(l))?,
                Err(Thrown(v)) => self.throw_into(e, v)?,
            }
            if self.pending.is_some() && self.drain_step(e)? {
                return Ok(iter_result(e, self.emit.clone(), false));
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
                self.emit = items[idx].clone();
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
        if crate::dbg_drive() {
            eprintln!(
                "[liver] pending={} stack={}",
                self.pending.is_some(),
                self.stack.len()
            );
        }
        match self.pending.take() {
            Some(Pending::Yield { into: Some(n) } | Pending::Await { into: Some(n), .. }) => {
                if let Some(env) = self.stack.last().map(|f| f.env.clone()) {
                    if crate::dbg_drive() {
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

    /// Nouvelle complétion abrupte — remplace toute
    /// complétion en réserve (un `break`/`throw`/`return`
    /// dans un `finally` l'emporte, spec).
    fn start_unwind(&mut self, e: &mut Engine, u: Unwind) -> JsResult<()> {
        self.unwind = None;
        self.resume_unwind(e, u)
    }

    /// Dépile les frames pour la complétion `u` — JS-C616.
    /// Chaque frame porteur de travail différé suspend
    /// l'unwind : `try{…}finally{…}` → `UnwindFin` poussé
    /// (la complétion reprend à sa sortie naturelle) ;
    /// `ForIter`/`ForAwait` → `IteratorClose` (`iter.return()`,
    /// awaited pour `for await` — spec `AsyncIteratorClose`).
    fn resume_unwind(&mut self, e: &mut Engine, u: Unwind) -> JsResult<()> {
        let mut u = u;
        loop {
            // Travail différé terminé — l'exécution continue.
            if matches!(u, Unwind::Done) {
                return Ok(());
            }
            let Some(f) = self.stack.last() else {
                return match u {
                    Unwind::Throw(v) => {
                        self.done = true;
                        if crate::dbg_drive() {
                            eprintln!("[uncaught] v={:?}", e.describe(v.clone()));
                        }
                        self.settle_outer(e, Err(v.clone()));
                        Err(Thrown(v))
                    }
                    // `Ret`/`Brk`/`Cont` en fond de pile : la
                    // boucle `drive` finalise à pile vide
                    // (`self.ret` porte la valeur pour `Ret`).
                    _ => Ok(()),
                };
            };
            let is_loop = matches!(
                f.kind,
                FKind::While { .. }
                    | FKind::For { .. }
                    | FKind::ForIter { .. }
                    | FKind::ForAwait { .. }
            );
            let hit = match &u {
                Unwind::Brk(l) => match l {
                    Some(l) => f.labels.iter().any(|x| x == l),
                    // `break` nu : boucle **ou switch**
                    // englobant (spec `BreakableStatement`).
                    None => is_loop || matches!(f.kind, FKind::Switch),
                },
                Unwind::Cont(l) => match l {
                    Some(l) => is_loop && f.labels.iter().any(|x| x == l),
                    None => is_loop,
                },
                _ => false,
            };
            if hit {
                if let Unwind::Cont(_) = u {
                    // `continue` : la boucle n'est pas quittée
                    // — elle re-teste/avance, pas de close.
                    let f = self.stack.last_mut().unwrap();
                    f.pc = f.list.len();
                    return self.leave_frame(e);
                }
                // `break` : la boucle sort — son
                // `IteratorClose` éventuel est dû avant de
                // conclure (`u = Done` ci-dessus).
                u = Unwind::Done;
            }
            let f = self.stack.pop().unwrap();
            match f.kind {
                FKind::Try {
                    catch: Some((pat, cbody)),
                    finally,
                } if matches!(u, Unwind::Throw(_)) => {
                    let Unwind::Throw(v) = &u else {
                        continue;
                    };
                    let v = v.clone();
                    // JS-C534 — canon resolver du bloc catch
                    // (`cbody.as_ptr()`, convention `exec.rs`) :
                    // le param + ses lexicals en `cells` denses.
                    let cenv = Env::local(f.env.clone());
                    e.predeclare_slots(&cenv, cbody.as_ptr() as usize);
                    e.bind_pat(
                        &cenv,
                        self.this.clone(),
                        &pat,
                        v,
                        BindMode::Lex { mutable: true },
                    )?;
                    // `finally` post-catch : marqueur
                    // `Try{None, fin}` sous le catch — le bloc
                    // court à la sortie du catch (normale via
                    // `leave_frame`, abrupte via l'unwind).
                    if finally.is_some() {
                        self.stack.push(GFrame {
                            list: Rc::new(Vec::new()),
                            pc: 0,
                            env: f.env.clone(),
                            labels: Vec::new(),
                            kind: FKind::Try {
                                catch: None,
                                finally,
                            },
                        });
                    }
                    self.stack.push(GFrame {
                        list: Rc::new(cbody),
                        pc: 0,
                        env: cenv,
                        labels: Vec::new(),
                        kind: FKind::Seq,
                    });
                    return Ok(());
                }
                FKind::Try {
                    finally: Some(fin), ..
                } => {
                    // `finally` traversé : suspendu — `UnwindFin`
                    // court le bloc, la complétion reprend à sa
                    // sortie naturelle (`leave_frame`).
                    self.unwind = Some((u, false));
                    self.stack.push(GFrame {
                        list: Rc::new(fin),
                        pc: 0,
                        env: f.env,
                        labels: Vec::new(),
                        kind: FKind::UnwindFin,
                    });
                    return Ok(());
                }
                FKind::ForIter { iter, .. } => {
                    // `for-of` : `IteratorClose` sync — les
                    // chemins rapides (arrays/TA/strings/
                    // `for-in`) portent `Undefined` (pas
                    // d'itérateur à fermer).
                    if !matches!(iter, Value::Undefined) {
                        u = Self::close_iter_sync(e, &iter, u);
                    }
                }
                // `issued` : abrupt hors corps (mécanique
                // `next()`/`await` en vol) — pas de close
                // (spec `ForIn/OfBodyEvaluation`).
                FKind::ForAwait { iter, issued, .. } if !issued => {
                    match self.close_iter_async(e, iter, f.env, u)? {
                        Some(u2) => u = u2,
                        // `return()` awaited : suspendu —
                        // `unwind`/`UnwindFin` posés par
                        // le helper.
                        None => return Ok(()),
                    }
                }
                _ => {}
            }
        }
    }

    /// `IteratorClose` sync (`for-of`) : `it.return()` —
    /// absent/non-callable → no-op ; abrupt ou résultat
    /// non-objet → plié dans `u` ([`fold_close`] : une throw
    /// externe garde sa valeur, spec §7.4.6).
    fn close_iter_sync(e: &mut Engine, iter: &Value, u: Unwind) -> Unwind {
        let ret = match e.get_prop_value(iter.clone(), "return") {
            Ok(r) => r,
            Err(Thrown(v)) => return fold_close(u, v),
        };
        if !e.is_callable(ret.clone()) {
            return u;
        }
        let r = match e.call_value(ret, iter.clone(), &[]) {
            Ok(r) => r,
            Err(Thrown(v)) => return fold_close(u, v),
        };
        if !matches!(r, Value::Obj(_)) {
            let Thrown(v) = e.type_error("iterator result is not an object");
            return fold_close(u, v);
        }
        u
    }

    /// `AsyncIteratorClose` (`for await`) : `it.return()`
    /// awaited — pendante → `UnwindFin` marqueur + `unwind`
    /// porte la complétion et le check-objet à la livraison ;
    /// rejetée ou résultat non-objet → plié dans `u`.
    /// `Some(u')` = poursuivre l'unwind, `None` = suspendu.
    fn close_iter_async(
        &mut self,
        e: &mut Engine,
        iter: Value,
        fenv: EnvRef,
        u: Unwind,
    ) -> JsResult<Option<Unwind>> {
        let ret = match e.get_prop_value(iter.clone(), "return") {
            Ok(r) => r,
            Err(Thrown(v)) => return Ok(Some(fold_close(u, v))),
        };
        if !e.is_callable(ret.clone()) {
            return Ok(Some(u));
        }
        let r = match e.call_value(ret, iter, &[]) {
            Ok(r) => r,
            Err(Thrown(v)) => return Ok(Some(fold_close(u, v))),
        };
        // `Await(innerResult)` — adoption thenable identique à
        // `arm_await`, mais la réjection se plie au lieu de
        // propager (`AsyncIteratorClose` — une throw externe
        // garde la sienne).
        let mut awaited = r.clone();
        if matches!(r, Value::Obj(_)) && !e.is_promise(r.clone()) {
            let wrapper = e.new_promise();
            if !e.adopt_thenable(wrapper, r.clone()) {
                e.settle_promise(wrapper, crate::promise::PromiseState::Fulfilled, r.clone());
            }
            awaited = Value::Obj(wrapper);
        }
        let Value::Obj(pid) = awaited else {
            // `return()` a rendu une valeur primitive —
            // `TypeError` plié.
            let Thrown(v) = e.type_error("iterator result is not an object");
            return Ok(Some(fold_close(u, v)));
        };
        let st = e.heap.obj(pid).and_then(|o| match &o.kind {
            ObjKind::Promise(p) => Some((p.state, p.result.clone())),
            _ => None,
        });
        match st {
            Some((crate::promise::PromiseState::Pending, _)) => {
                // Suspendu : `liver` liera `__awaitit` dans
                // l'env du `UnwindFin` ; `leave_frame` valide
                // l'objet (le `bool` de `unwind`).
                self.arm_promise(e, pid, Some("__awaitit".into()));
                self.stack.push(GFrame {
                    list: Rc::new(Vec::new()),
                    pc: 0,
                    env: Env::local(fenv),
                    labels: Vec::new(),
                    kind: FKind::UnwindFin,
                });
                self.unwind = Some((u, true));
                Ok(None)
            }
            Some((crate::promise::PromiseState::Fulfilled, res)) => {
                let mut u = u;
                if !matches!(res, Value::Obj(_)) {
                    let Thrown(v) = e.type_error("iterator result is not an object");
                    u = fold_close(u, v);
                }
                Ok(Some(u))
            }
            Some((crate::promise::PromiseState::Rejected, res)) => Ok(Some(fold_close(u, res))),
            None => Ok(Some(u)),
        }
    }

    /// Settlement de la promesse de l'async fn (`outer`)
    /// à la fin du corps — fulfill `ret` / reject la throw.
    fn settle_outer(&self, e: &mut Engine, r: std::result::Result<Value, Value>) {
        if crate::dbg_drive() {
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
    /// et `IteratorClose` suspendent puis la complétion
    /// repart (`resume_unwind`, JS-C616).
    fn throw_into(&mut self, e: &mut Engine, v: Value) -> JsResult<()> {
        // La suspension courante est consommée par la throw —
        // sans ça la boucle réémettrait l'ancienne valeur.
        self.pending = None;
        // Une throw remplace un `return(v)` en vol (spec — la
        // nouvelle complétion abrupte l'emporte).
        self.ret_unwind = false;
        self.start_unwind(e, Unwind::Throw(v))
    }

    /// Fin de liste : boucles re-testent/re-bindent, `try`
    /// court `finally`, `UnwindFin` reprend l'unwind
    /// enregistré.
    fn leave_frame(&mut self, e: &mut Engine) -> JsResult<()> {
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
            // `UnwindFin` — `finally` drainé ou `iter.return()`
            // awaited livré : la complétion enregistrée reprend
            // (`check` → `__awaitit` doit être un objet, spec
            // `AsyncIteratorClose`).
            FKind::UnwindFin => {
                let env = f.env.clone();
                self.stack.pop();
                if let Some((u, check)) = self.unwind.take() {
                    let mut u = u;
                    if check {
                        let r = e.env_get(&env, "__awaitit").unwrap_or(Value::Undefined);
                        if !matches!(r, Value::Obj(_)) {
                            let Thrown(v) = e.type_error("iterator result is not an object");
                            u = fold_close(u, v);
                        }
                    }
                    self.resume_unwind(e, u)?;
                }
            }
            FKind::Seq | FKind::Switch => {
                self.stack.pop();
            }
            FKind::While { cond } => {
                let (env, cond) = (f.env.clone(), cond.clone());
                let c = e.eval_expr(&env, self.this.clone(), &cond)?;
                if e.to_boolean(c) {
                    self.stack.last_mut().unwrap().pc = 0;
                } else {
                    self.stack.pop();
                }
            }
            FKind::For { test, update } => {
                let (env, test, update) = (f.env.clone(), test.clone(), update.clone());
                if let Some(u) = &update {
                    e.eval_expr(&env, self.this.clone(), u)?;
                }
                let cont = match &test {
                    Some(t) => {
                        let c = e.eval_expr(&env, self.this.clone(), t)?;
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
            FKind::ForIter {
                items, idx, target, ..
            } => {
                let (env, items, idx, target) =
                    (f.env.clone(), items.clone(), *idx, target.clone());
                if idx >= items.len() {
                    self.stack.pop();
                    return Ok(());
                }
                let v = items[idx].clone();
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
                sync_iter,
            } => {
                let (env, it, next, target, issued, sync_iter) = (
                    f.env.clone(),
                    iter.clone(),
                    next.clone(),
                    target.clone(),
                    *issued,
                    *sync_iter,
                );
                if !issued {
                    // `issued=true` pendant toute la mécanique
                    // de pas (`next()`, lecture `done`/`value`,
                    // armement) — une erreur ici est abrupte
                    // hors du corps : pas d'`IteratorClose`
                    // (spec `ForIn/OfBodyEvaluation`).
                    if let Some(fr) = self.stack.last_mut() {
                        if let FKind::ForAwait { issued, .. } = &mut fr.kind {
                            *issued = true;
                        }
                    }
                    let r = match e.call_value(next, it, &[]) {
                        Ok(v) => v,
                        Err(Thrown(v)) => return self.throw_into(e, v),
                    };
                    let awaited = if sync_iter {
                        // `AsyncFromSyncIterator` : `r` =
                        // `{value,done}` sync — `done` lu à
                        // plat, c'est `value` qui est awaited.
                        let Value::Obj(id) = r else {
                            let Thrown(v) = e.type_error("iterator result is not an object");
                            return self.throw_into(e, v);
                        };
                        if e.get_prop(id, "done").is_some_and(|d| e.to_boolean(d)) {
                            self.stack.pop();
                            return Ok(());
                        }
                        match e.get_prop_value(Value::Obj(id), "value") {
                            Ok(v) => v,
                            Err(Thrown(v)) => return self.throw_into(e, v),
                        }
                    } else {
                        r
                    };
                    let flow = self.arm_await(e, awaited, &env, Some("__awaitit".into()))?;
                    // `Susp` : rejet rattrapé — les frames sont
                    // remplacées (catch), ne pas consommer.
                    if self.pending.is_some() || matches!(flow, GFlow::Susp) {
                        return Ok(());
                    }
                }
                // `__awaitit` : résultat `{value,done}` (async)
                // ou valeur unwrapped (`sync_iter`) — lié
                // inline ou par `liver` à la reprise.
                let r = e.env_get(&env, "__awaitit").unwrap_or(Value::Undefined);
                let val = if sync_iter {
                    r
                } else {
                    let Value::Obj(id) = r else {
                        let Thrown(v) = e.type_error("iterator result is not an object");
                        return self.throw_into(e, v);
                    };
                    if e.get_prop(id, "done").is_some_and(|d| e.to_boolean(d)) {
                        self.stack.pop();
                        return Ok(());
                    }
                    e.get_prop(id, "value").unwrap_or(Value::Undefined)
                };
                // Le binding de la cible fait partie du corps
                // — ses erreurs déclenchent `IteratorClose` :
                // `issued=false` avant le bind.
                if let Some(fr) = self.stack.last_mut() {
                    if let FKind::ForAwait { issued, .. } = &mut fr.kind {
                        *issued = false;
                    }
                }
                if let Err(Thrown(v)) = self.bind_for_target(e, &env, &target, val) {
                    return self.throw_into(e, v);
                }
                if let Some(fr) = self.stack.last_mut() {
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
        into: Option<Rc<str>>,
    ) -> JsResult<GFlow> {
        let mut awaited = v.clone();
        if matches!(v, Value::Obj(_)) && !e.is_promise(v.clone()) {
            // Thenable étranger (polyfill `Promise` des bundles,
            // jQuery Deferred, `IDBRequest` pont-thenable —
            // JS-C278, mur §B-14) : spec `await v` =
            // `PromiseResolve(%Promise%, v)` — adoption par
            // `then` callable, appel différé en job. Le wrapper
            // est Pending (adoption en file) ou Rejected
            // (getter `then` abrupt → jeté au site ci-dessous) ;
            // non-thenable → fulfill direct avec `v`.
            let wrapper = e.new_promise();
            if !e.adopt_thenable(wrapper, v.clone()) {
                e.settle_promise(wrapper, crate::promise::PromiseState::Fulfilled, v.clone());
            }
            awaited = Value::Obj(wrapper);
        }
        if let Value::Obj(pid) = awaited {
            let st = e.heap.obj(pid).and_then(|o| match &o.kind {
                ObjKind::Promise(p) => Some((p.state, p.result.clone())),
                _ => None,
            });
            if let Some((state, result)) = st {
                match state {
                    crate::promise::PromiseState::Pending => {
                        return Ok(self.arm_promise(e, pid, into));
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

    /// Réactions `async_resume` boundées `[gen_obj, outer]`
    /// sur `pid` + `pending = Await{into}` armé — partagé par
    /// `arm_await` et l'`iter.return()` awaited d'
    /// `AsyncIteratorClose` (JS-C616).
    fn arm_promise(&mut self, e: &mut Engine, pid: ObjId, into: Option<Rc<str>>) -> GFlow {
        // `BoundNative` prépose bound aux args →
        // `async_resume_ok(gen, outer, v)`.
        let gen = self.gen_obj.clone();
        let next = e.new_promise();
        let ok_f = bound_resume(e, gen.clone(), self.outer, true);
        let err_f = bound_resume(e, gen, self.outer, false);
        if let Some(o) = e.heap.obj_mut(pid) {
            if let ObjKind::Promise(p) = &mut o.kind {
                p.reactions.push(crate::promise::Reaction {
                    on_fulfilled: ok_f,
                    on_rejected: err_f,
                    next: crate::promise::ThenSink::Native(next),
                });
            }
        }
        self.pending = Some(Pending::Await { into, prom: pid });
        GFlow::Adv
    }

    /// `await e` — évalue `e` puis [`Self::arm_await`].
    fn exec_await(
        &mut self,
        e: &mut Engine,
        a: &Expr,
        env: &EnvRef,
        into: Option<Rc<str>>,
    ) -> JsResult<GFlow> {
        let v = e.eval_expr(env, self.this.clone(), a)?;
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
                e.bind_pat(env, self.this.clone(), p, v, BindMode::Var)
            }
            ForTarget::Expr(x) => {
                if let Some(lv) = e.resolve_lvalue(env, self.this.clone(), x)? {
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

    /// JS-C534 — env de frame câblé quand le corps est un bloc :
    /// `names_table[l.as_ptr()]` porte le canon resolver du bloc
    /// (même convention `exec.rs`) → liaisons en `cells` denses
    /// au lieu de `extra`. Corps non-bloc : aucune déclaration
    /// lexicale possible — l'env reste un pass-through non
    /// câblé (`extra` vide, lookup remonte direct).
    fn frame_env(e: &Engine, env: &EnvRef, body: &Stmt) -> EnvRef {
        let fenv = Env::local(env.clone());
        if let Stmt::Block(l) = body {
            e.predeclare_slots(&fenv, l.as_ptr() as usize);
        }
        fenv
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
            return match e.exec_stmt(env, self.this.clone(), s)? {
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
                    Some(a) => e.eval_expr(env, self.this.clone(), a)?,
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
                                Some(a) => e.eval_expr(env, self.this.clone(), a)?,
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
                            e.exec_stmt(env, self.this.clone(), &Stmt::Var(vec![d.clone()]))?;
                        }
                    }
                }
                Ok(GFlow::Adv)
            }
            // `return e` — fin du corps.
            Stmt::Return(x) => {
                let v = match x {
                    Some(e2) => e.eval_expr(env, self.this.clone(), e2)?,
                    None => Value::Undefined,
                };
                Ok(GFlow::Ret(v))
            }
            // Composés porteurs de suspensions → frames.
            Stmt::If { cond, then, else_ } => {
                let c = e.eval_expr(env, self.this.clone(), cond)?;
                let branch = if e.to_boolean(c) {
                    Some(&**then)
                } else {
                    else_.as_deref()
                };
                if let Some(b) = branch {
                    self.stack.push(GFrame {
                        list: Rc::new(Self::body_list(b)),
                        pc: 0,
                        env: Self::frame_env(e, env, b),
                        labels: Vec::new(),
                        kind: FKind::Seq,
                    });
                }
                Ok(GFlow::Adv)
            }
            Stmt::Block(l) => {
                // JS-C502 — même clé `l.as_ptr()` que le resolver
                // (env `Local` ici vs `Block` en sync : la
                // pré-déclare accepte les deux, `Uninit` reste
                // transparente à `var`).
                let benv = Env::local(env.clone());
                e.predeclare_slots(&benv, l.as_ptr() as usize);
                self.stack.push(GFrame {
                    list: Rc::new(l.clone()),
                    pc: 0,
                    env: benv,
                    labels: Vec::new(),
                    kind: FKind::Seq,
                });
                Ok(GFlow::Adv)
            }
            Stmt::While { cond, body } => {
                let c = e.eval_expr(env, self.this.clone(), cond)?;
                if e.to_boolean(c) {
                    self.stack.push(GFrame {
                        list: Rc::new(Self::body_list(body)),
                        pc: 0,
                        env: Self::frame_env(e, env, body),
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
                    env: Self::frame_env(e, env, body),
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
                    e.exec_stmt(env, self.this.clone(), &st)?;
                }
                let cont = match test {
                    Some(t) => {
                        let c = e.eval_expr(env, self.this.clone(), t)?;
                        e.to_boolean(c)
                    }
                    None => true,
                };
                if cont {
                    self.stack.push(GFrame {
                        list: Rc::new(Self::body_list(body)),
                        pc: 0,
                        env: Self::frame_env(e, env, body),
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
                let v = e.eval_expr(env, self.this.clone(), iter)?;
                // `for await` — itérateur async, suspension par
                // pas : le frame part « consommé » (`pc` en fin
                // de liste), `leave_frame` émet le premier
                // `it.next()`.
                if *is_await {
                    let (it, next, sync_iter) = async_iterator(e, v)?;
                    let list = Self::body_list(body);
                    self.stack.push(GFrame {
                        pc: list.len(),
                        list: Rc::new(list),
                        env: Self::frame_env(e, env, body),
                        labels: Vec::new(),
                        kind: FKind::ForAwait {
                            iter: it,
                            next,
                            target: target.clone(),
                            issued: false,
                            sync_iter,
                        },
                    });
                    return Ok(GFlow::Adv);
                }
                let (it_obj, items) = e.iter_values_iter(v)?;
                if items.is_empty() {
                    return Ok(GFlow::Adv);
                }
                self.bind_for_target(e, env, target, items[0].clone())?;
                let benv = Self::frame_env(e, env, body);
                self.stack.push(GFrame {
                    list: Rc::new(Self::body_list(body)),
                    pc: 0,
                    env: benv,
                    labels: Vec::new(),
                    kind: FKind::ForIter {
                        iter: it_obj,
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
                let v = e.eval_expr(env, self.this.clone(), iter)?;
                let items = match v {
                    Value::Obj(id) => e.enum_keys(id)?.into_iter().map(|k| e.str_val(k)).collect(),
                    _ => Vec::new(),
                };
                if items.is_empty() {
                    return Ok(GFlow::Adv);
                }
                self.bind_for_target(e, env, target, items[0].clone())?;
                let benv = Self::frame_env(e, env, body);
                self.stack.push(GFrame {
                    list: Rc::new(Self::body_list(body)),
                    pc: 0,
                    env: benv,
                    labels: Vec::new(),
                    kind: FKind::ForIter {
                        // `for-in` n'a pas d'itérateur à
                        // fermer (protocole d'énumération).
                        iter: Value::Undefined,
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
                // JS-C534 — canon resolver du bloc try
                // (`block.as_ptr()`, convention `exec.rs`).
                let benv = Env::local(env.clone());
                e.predeclare_slots(&benv, block.as_ptr() as usize);
                self.stack.push(GFrame {
                    list: Rc::new(block.clone()),
                    pc: 0,
                    env: benv,
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
                    env: Self::frame_env(e, env, body),
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
                let dv = e.eval_expr(env, self.this.clone(), disc)?;
                let mut start = None;
                let mut default = None;
                for (i, c) in cases.iter().enumerate() {
                    match &c.test {
                        Some(t) => {
                            let tv = e.eval_expr(env, self.this.clone(), t)?;
                            if start.is_none() && e.strict_eq(&dv, &tv) {
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
                        // JS-C534 — bloc déclaratif unique du
                        // switch : canon `cases.as_ptr()`
                        // (convention `exec.rs` — le `flat` est
                        // synthétique, la clé reste celle des
                        // cases source).
                        let senv = Env::local(env.clone());
                        e.predeclare_slots(&senv, cases.as_ptr() as usize);
                        self.stack.push(GFrame {
                            list: Rc::new(flat),
                            pc: 0,
                            env: senv,
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
            other => match e.exec_stmt(env, self.this.clone(), other)? {
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
            if let FKind::ForIter { iter, items, .. } = &f.kind {
                pv(iter, out);
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
        // Complétion en vol (`unwind`) : la throw portée est
        // une racine tant qu'un `finally`/close est suspendu.
        if let Some((Unwind::Throw(v), _)) = &self.unwind {
            pv(v, out);
        }
        // File `AsyncGeneratorEnqueue` : promesses en attente
        // + valeurs de complétion (`next(v)`/`return(v)`/
        // `throw(v)` non servis).
        if let Some(st) = &self.agen {
            for (_, v, p) in &st.queue {
                pv(v, out);
                out.push(*p);
            }
        }
    }
}

/// `IteratorClose` : une abrupte du close (`return()` jetée/
/// rejetée, résultat non-objet) se plie dans la complétion
/// en vol — une throw externe garde sa valeur, les autres
/// complétions sont remplacées (spec §7.4.6, JS-C616).
fn fold_close(u: Unwind, v: Value) -> Unwind {
    match u {
        Unwind::Throw(_) => u,
        _ => Unwind::Throw(v),
    }
}

/// `for await` — itérateur asynchrone : `@@asyncIterator`
/// d'abord, repli `@@iterator` (spec
/// `CreateAsyncFromSyncIterator` — chaque `next()` est
/// awaited, un itérateur sync se comporte à l'identique
/// puisque ses `{value,done}` ne sont jamais des promesses
/// pendantes). `(iter, next_method)` ou `TypeError`.
fn async_iterator(e: &mut Engine, v: Value) -> JsResult<(Value, Value, bool)> {
    if let Value::Obj(id) = v {
        for (i, sym) in [
            crate::interp::SYM_ASYNC_ITERATOR,
            crate::interp::SYM_ITERATOR,
        ]
        .into_iter()
        .enumerate()
        {
            if let Some(m) = e.get_prop(id, sym) {
                if e.is_callable(m.clone()) {
                    let it = e.call_value(m, v, &[])?;
                    if let Value::Obj(iid) = it {
                        if let Some(next) = e.get_prop(iid, "next") {
                            if e.is_callable(next.clone()) {
                                // `i==1` : repli `@@iterator`
                                // — `AsyncFromSyncIterator`,
                                // `result.value` awaited.
                                return Ok((it, next, i == 1));
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
        idx: std::cell::OnceCell::new(),
        shape: std::cell::Cell::new(0),
        kind: ObjKind::GenLive { exec },
        extensible: true,
        elem_integrity: 0,
        lookups: std::cell::Cell::new(0),
        flags: std::cell::Cell::new(0),
        args_map: None,
    });
    let v = Value::Obj(id);
    if let Some(o) = e.heap.obj(id) {
        if let ObjKind::GenLive { exec } = &o.kind {
            exec.borrow_mut().gen_obj = v.clone();
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
        idx: std::cell::OnceCell::new(),
        shape: std::cell::Cell::new(0),
        kind: ObjKind::Fun(Fun::BoundNative {
            name: "async_resume",
            f,
            bound,
            realm: e.cur_ctx,
        }),
        extensible: true,
        elem_integrity: 0,
        lookups: std::cell::Cell::new(0),
        flags: std::cell::Cell::new(0),
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
    if crate::dbg_drive() {
        eprintln!("[resume] ok={ok} gen={gen:?}");
    }
    let exec = if let Value::Obj(gid) = gen {
        match e.heap.obj(gid).map(|o| &o.kind) {
            Some(ObjKind::GenLive { exec }) => Some(exec.clone()),
            _ => None,
        }
    } else {
        None
    };
    if let Some(exec) = exec {
        let r = if ok { Ok(v) } else { Err(v) };
        // Ré-entrance défensive (reprise pendant que le
        // corps tourne — drain de microtâches en vol) :
        // l'emprunt occupé rend la reprise sans effet
        // (JS-C593).
        if let Ok(mut g) = exec.try_borrow_mut() {
            // `async function*` : la reprise transite par la
            // pompe `AsyncGeneratorEnqueue` — la requête de
            // tête settle sa promesse quand le drive produit
            // un `{value,done}` ou repart en suspension
            // (JS-C601).
            if g.agen.is_some() {
                g.resume = Some(r);
                drop(g);
                agen_pump(e, &exec);
            } else {
                let _ = g.resume_await(e, r);
            }
        }
    }
    Ok(Value::Undefined)
}

/// Le `this` d'une méthode `%AsyncGenerator%` porte-t-il un
/// `GenLive` ? (La distinction sync/async se lit sous
/// emprunt à l'appel — `is_agen`.)
fn agen_exec(e: &Engine, this: &Value) -> Option<Rc<RefCell<GExec>>> {
    if let Value::Obj(id) = this {
        if let Some(o) = e.heap.obj(*id) {
            if let ObjKind::GenLive { exec } = &o.kind {
                return Some(exec.clone());
            }
        }
    }
    None
}

/// Settlement immédiat d'une requête sur générateur
/// `completed` (spec `AsyncGeneratorStart` → état
/// `completed` : `next` → `{undefined,done:true}`,
/// `return` → `{v,done:true}`, `throw` → reject).
fn agen_settle_completed(e: &mut Engine, kind: AgenKind, v: Value, prom: ObjId) {
    match kind {
        AgenKind::Next => {
            let ir = iter_result(e, Value::Undefined, true);
            e.resolve_promise(prom, ir);
        }
        AgenKind::Return => {
            let ir = iter_result(e, v, true);
            e.resolve_promise(prom, ir);
        }
        AgenKind::Throw => e.reject_promise(prom, v),
    }
}

/// `%AsyncGenerator%.next(v)` — enfile, pompe, rend la promesse.
pub(crate) fn agen_next(e: &mut Engine, this: Value, a: &[Value]) -> JsResult<Value> {
    agen_enqueue(e, this, AgenKind::Next, arg(a, 0))
}

/// `%AsyncGenerator%.return(v)` — file `AsyncGeneratorAwaitReturn`.
pub(crate) fn agen_return(e: &mut Engine, this: Value, a: &[Value]) -> JsResult<Value> {
    agen_enqueue(e, this, AgenKind::Return, arg(a, 0))
}

/// `%AsyncGenerator%.throw(e)`.
pub(crate) fn agen_throw(e: &mut Engine, this: Value, a: &[Value]) -> JsResult<Value> {
    agen_enqueue(e, this, AgenKind::Throw, arg(a, 0))
}

/// `AsyncGeneratorEnqueue` — pousse la requête, lance la
/// pompe si le générateur n'exécute pas déjà.
fn agen_enqueue(e: &mut Engine, this: Value, kind: AgenKind, v: Value) -> JsResult<Value> {
    let Some(exec) = agen_exec(e, &this) else {
        return Err(e.type_error("async generator method on incompatible receiver"));
    };
    let prom = e.new_promise();
    {
        // Ré-entrance : `it.next()` pendant que le corps
        // tourne (appel depuis le générateur lui-même ou un
        // drain en vol) — spec `GeneratorResume` → TypeError,
        // pas de panic `BorrowMutError` (même garde JS-C593).
        let Ok(mut g) = exec.try_borrow_mut() else {
            return Err(e.type_error("async generator is already running"));
        };
        if !g.is_agen() {
            return Err(e.type_error("async generator method on incompatible receiver"));
        }
        if g.done {
            drop(g);
            agen_settle_completed(e, kind, v, prom);
            return Ok(Value::Obj(prom));
        }
        if let Some(st) = &mut g.agen {
            st.queue.push_back((kind, v, prom));
        }
    }
    agen_pump(e, &exec);
    Ok(Value::Obj(prom))
}

/// Pompe `AsyncGeneratorEnqueue` — sert la tête de file tant
/// que le générateur n'est ni `completed` ni suspendu en
/// `await`. Point de convergence unique : `agen_enqueue`
/// (nouvelle requête) et `resume_common` (settlement d'un
/// `await` en vol) appellent tous deux ici — le second pose
/// `resume` puis la pompe re-drive la requête de tête.
pub(crate) fn agen_pump(e: &mut Engine, exec: &Rc<RefCell<GExec>>) {
    enum Act {
        Stop,
        Drain(Vec<(AgenKind, Value, ObjId)>),
        Run(AgenKind, Value, ObjId),
    }
    loop {
        let act = {
            let Ok(mut g) = exec.try_borrow_mut() else {
                // Emprunt occupé = un drive est en vol (le
                // corps s'appelle lui-même en ré-entrance) —
                // la pompe courante finira le service.
                return;
            };
            let done = g.done;
            let pend = g.pending_await();
            let Some(st) = &mut g.agen else { return };
            if done {
                st.executing = false;
                Act::Drain(st.queue.drain(..).collect())
            } else if pend {
                // Suspension `await` en vol : la requête de
                // tête attend sa settlement — la reprise
                // (`resume_common`) relancera la pompe.
                st.executing = true;
                Act::Stop
            } else {
                match st.queue.front() {
                    None => {
                        st.executing = false;
                        Act::Stop
                    }
                    Some((k, v, p)) => {
                        st.executing = true;
                        Act::Run(*k, v.clone(), *p)
                    }
                }
            }
        };
        match act {
            Act::Stop => return,
            Act::Drain(rest) => {
                for (k, v, p) in rest {
                    agen_settle_completed(e, k, v, p);
                }
                return;
            }
            Act::Run(kind, v, prom) => {
                let r = {
                    let Ok(mut g) = exec.try_borrow_mut() else {
                        return;
                    };
                    if g.resume.is_some() {
                        // Reprise d'une suspension `await` —
                        // la complétion livrée (`Ok`→liver,
                        // `Err`→throw_into) appartient à la
                        // requête de tête, pas à `kind`.
                        g.drive(e)
                    } else {
                        match kind {
                            AgenKind::Next => g.next(e, v),
                            AgenKind::Return => g.ret_at(e, v),
                            AgenKind::Throw => g.throw_at(e, v),
                        }
                    }
                };
                // Suspension `await` en pleine requête : la
                // tête reste en file — settle à la reprise.
                let suspended = exec.try_borrow().map_or(true, |g| g.pending_await());
                if suspended {
                    return;
                }
                {
                    let Ok(mut g) = exec.try_borrow_mut() else {
                        return;
                    };
                    if let Some(st) = &mut g.agen {
                        let _ = st.queue.pop_front();
                    }
                }
                // `{value,done}` → resolve ; complétion
                // abrupte → reject (spec
                // `AsyncGeneratorResolve`/`Reject`).
                match r {
                    Ok(ir) => e.resolve_promise(prom, ir),
                    Err(Thrown(t)) => e.reject_promise(prom, t),
                }
            }
        }
    }
}
