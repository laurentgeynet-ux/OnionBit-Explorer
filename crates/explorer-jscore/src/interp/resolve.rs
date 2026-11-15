// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Résolution statique des identifiants (JS-C490, cran 80).
//!
//! Chaque site `Expr::Ident` est annoté par la **profondeur
//! d'env** où sa liaison vit — le nombre de niveaux de la chaîne
//! parente à sauter avant le `BindingMap::get`. Le runtime saute
//! ces niveaux sans sonder les maps intermédiaires, puis applique
//! `env_get` complet depuis l'env cible : si l'annotation est
//! trop courte/longue (bug du resolveur), la marche remonte
//! naturellement au-delà — le seul risque résiduel est un env
//! sauté qui porterait le même nom (écart de modèle, borné par
//! les règles conservatives ci-dessous).
//!
//! La résolution est **paresseuse par `FnLit`** : les corps
//! `FnBody::Deferred` n'existent qu'au premier appel — chaque lit
//! matérialisé est résolu alors, en ignorant les portées
//! englobantes non re-parcourues (les captures inter-fonctions
//! restent sur la marche dynamique — couverture propre au corps
//! + sous-corps déjà parsés lors du premier passage).
//!
//! Bail conservateur (aucune annotation dans le sous-arbre) :
//! - `with` : l'env objet est dans la chaîne — comptage faux ;
//! - `eval`/`delete` dans un corps de fonction : peut injecter
//!   ou retirer des noms du scope local → ce scope est marqué
//!   `poisoned` et toute résolution le traversant est sautée ;
//! - `arguments` : cellule paresseuse spéciale (`LazyArgs`) ;
//! - `import`/`export` : chaînes d'env de modules non modélisées.
//!
//! Les portées calquées sur les sites de création d'env du
//! runtime (`exec.rs`/`mod.rs`) : `Env::local` par appel de
//! fonction, `Env::block` pour `{…}`, try/catch/finally (un par
//! clause), switch (un partagé), `for(let…)` (tête + copie par
//! itération = un niveau), `for-in/of` lexical (un par itération),
//! `Env::local` par classe (binds privés `super`/`home`),
//! `Env::local` nom de function-expression.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use super::{LIT_MAY_HOIST, LIT_USES_ARGS, LIT_USES_NT, NEW_TARGET};
use crate::ast::{
    ChainStep, ClassLit, Expr, FnBody, FnLit, ForInit, ForTarget, ObjProp, Param, Pat, PropKey,
    Stmt, UnOp, VarDecl,
};

/// Portée statique — une entrée = un niveau d'env au runtime.
struct Scope {
    /// Noms liés à ce niveau.
    names: HashSet<Rc<str>>,
    /// JS-C502 — liste **canonique ordonnée** des noms quand le
    /// site de création d'env du runtime est pré-déclaré par
    /// slot : `order[slot] = nom`. `None` pour les portées sans
    /// convention de slots (scope `Local` fonction — JS-C503 —
    /// `Global`, `cenv` de classe) : leurs annotations portent
    /// `slot = u16::MAX` (depth-only).
    order: Option<Rc<[Rc<str>]>>,
    /// `eval`/`delete` dans ce corps de fonction : les résolutions
    /// qui atterrissent sur ce niveau ou le traversent sont
    /// sautées (le scope peut gagner des noms dynamiquement).
    poisoned: bool,
}

impl Scope {
    fn new(names: HashSet<Rc<str>>, order: Option<Rc<[Rc<str>]>>, poisoned: bool) -> Self {
        Self {
            names,
            order,
            poisoned,
        }
    }
}

/// `slot` sentinelle — « depth-only » (pas de convention de
/// slots sur ce niveau).
const NO_SLOT: u16 = u16::MAX;

/// Tables produites par [`resolve_lit`].
pub(crate) struct ResTables {
    /// `ptr(Expr::Ident) → (profondeur << 16) | slot`.
    pub map: HashMap<usize, u32>,
    /// `clé de site AST → liste canonique ordonnée des noms` —
    /// les envs créés à ces sites sont pré-déclarés
    /// (`EnvCell::Uninit`) dans cet ordre. Clés : `as_ptr()` du
    /// `Vec<Stmt>`/`Vec<VarDecl>` enveloppé, `ptr(ForTarget)`,
    /// `ptr(Stmt)` switch, `Rc::as_ptr(FnLit)` d'expression nommée.
    pub names: HashMap<usize, Rc<[Rc<str>]>>,
}

/// Marcheur : accumule les annotations `ptr(Expr) → profondeur`.
pub(crate) struct Resolver {
    /// Sortie — `ptr(Expr::Ident) → niveaux à sauter (+ slot)`.
    pub map: HashMap<usize, u32>,
    /// Sortie — listes canoniques par site de création d'env.
    pub names: HashMap<usize, Rc<[Rc<str>]>>,
    /// Pile de portées — `last()` = env courant (index 0 de
    /// profondeur). Parallèle : `poisoned` par entrée.
    scopes: Vec<Scope>,
    /// > 0 dans un sous-arbre `with` — aucune annotation.
    suppress: u32,
}

/// Résout les identifiants du corps d'un `FnLit` — scope propre
/// (paramètres + hoisting) puis descente dans les sous-portées et
/// les fonctions imbriquées déjà `Parsed`.
/// `lowered_params` — params **baissés** pour un lit gen/async
/// (le corps passé est déjà le corps baissé ; les défauts
/// contenant un suspend ont été réécrits `__paramN`). Le canon
/// du scope fn est alors émis sous `body.as_ptr()` — la clé que
/// le runtime demande (`lf.body.as_ptr()`). Sans params baissés,
/// gen/async reste `depth-only` (walk imbriqué sur corps source).
pub(crate) fn resolve_lit(
    lit: &FnLit,
    body: &[Stmt],
    lowered_params: Option<&[Param]>,
) -> ResTables {
    let mut r = Resolver {
        map: HashMap::new(),
        names: HashMap::new(),
        scopes: Vec::new(),
        suppress: 0,
    };
    // Modules : liaisons installées à l'instanciation, chaîne
    // d'env non modélisée — aucune annotation.
    if body
        .iter()
        .any(|s| matches!(s, Stmt::Import { .. } | Stmt::Export(_)))
    {
        return ResTables {
            map: r.map,
            names: r.names,
        };
    }
    r.walk_fn(lit, body, lowered_params);
    ResTables {
        map: r.map,
        names: r.names,
    }
}

/// `eval`/`delete <ident>` dans le corps — le scope local peut
/// gagner/perdre des noms dynamiquement. Ne descend pas dans les
/// fonctions imbriquées (leur poison est le leur).
fn body_poisons(stmts: &[Stmt]) -> bool {
    for s in stmts {
        if stmt_poisons(s) {
            return true;
        }
    }
    false
}

fn stmt_poisons(s: &Stmt) -> bool {
    // Chaque arête récursive re-bascule (même motif que
    // `walk_expr`) — un AST hostile de 20 000 niveaux devient
    // un coût tas, jamais un débordement de pile.
    stacker::maybe_grow(
        crate::parser::STACK_RED_ZONE,
        crate::parser::STACK_GROW,
        || stmt_poisons_inner(s),
    )
}

fn stmt_poisons_inner(s: &Stmt) -> bool {
    match s {
        Stmt::Expr(e) | Stmt::Return(Some(e)) | Stmt::Throw(e) => expr_poisons(e),
        Stmt::Var(ds) | Stmt::Lexical { decls: ds, .. } | Stmt::Using { decls: ds, .. } => ds
            .iter()
            .any(|d| d.init.as_ref().is_some_and(expr_poisons) || pat_poisons(&d.pat)),
        // Les corps de fonction imbriqués portent leur propre
        // poison — hors périmètre de ce scan (`_`).
        Stmt::Block(b) => body_poisons(b),
        Stmt::If { cond, then, else_ } => {
            expr_poisons(cond) || stmt_poisons(then) || else_.as_deref().is_some_and(stmt_poisons)
        }
        Stmt::While { cond, body } | Stmt::DoWhile { body, cond } => {
            expr_poisons(cond) || stmt_poisons(body)
        }
        Stmt::For {
            init,
            test,
            update,
            body,
        } => {
            init.as_ref().is_some_and(|i| match i {
                ForInit::Var(ds) | ForInit::Lexical { decls: ds, .. } => ds
                    .iter()
                    .any(|d| d.init.as_ref().is_some_and(expr_poisons) || pat_poisons(&d.pat)),
                ForInit::Expr(e) => expr_poisons(e),
            }) || test.as_ref().is_some_and(expr_poisons)
                || update.as_ref().is_some_and(expr_poisons)
                || stmt_poisons(body)
        }
        Stmt::ForIn {
            target,
            obj,
            var_init,
            body,
        } => {
            target_poisons(target)
                || expr_poisons(obj)
                || var_init.as_ref().is_some_and(expr_poisons)
                || stmt_poisons(body)
        }
        Stmt::ForOf {
            target, iter, body, ..
        } => target_poisons(target) || expr_poisons(iter) || stmt_poisons(body),
        Stmt::With { obj, body } => expr_poisons(obj) || stmt_poisons(body),
        Stmt::Labeled { body, .. } => stmt_poisons(body),

        Stmt::Try {
            block,
            catch,
            finally,
        } => {
            body_poisons(block)
                || catch
                    .as_ref()
                    .is_some_and(|(p, b)| pat_poisons(p) || body_poisons(b))
                || finally.as_ref().is_some_and(|b| body_poisons(b))
        }
        Stmt::Switch { disc, cases } => {
            expr_poisons(disc)
                || cases
                    .iter()
                    .any(|c| c.test.as_ref().is_some_and(expr_poisons) || body_poisons(&c.body))
        }
        _ => false,
    }
}

fn target_poisons(t: &ForTarget) -> bool {
    match t {
        ForTarget::Var(p) | ForTarget::Lexical { pat: p, .. } | ForTarget::Using { pat: p, .. } => {
            pat_poisons(p)
        }
        ForTarget::Expr(e) => expr_poisons(e),
    }
}

fn pat_poisons(p: &Pat) -> bool {
    stacker::maybe_grow(
        crate::parser::STACK_RED_ZONE,
        crate::parser::STACK_GROW,
        || pat_poisons_inner(p),
    )
}

fn pat_poisons_inner(p: &Pat) -> bool {
    match p {
        Pat::Ident(_) => false,
        Pat::Lvalue(e) => expr_poisons(e),
        Pat::Rest(p) => pat_poisons(p),
        Pat::Default(p, e) => pat_poisons(p) || expr_poisons(e),
        Pat::Array(elems) => elems.iter().flatten().any(pat_poisons),
        Pat::Object { props, rest } => {
            props.iter().any(|(k, p)| {
                matches!(k, PropKey::Computed(e) if expr_poisons(e)) || pat_poisons(p)
            }) || rest.as_deref().is_some_and(pat_poisons)
        }
    }
}

fn expr_poisons(e: &Expr) -> bool {
    stacker::maybe_grow(
        crate::parser::STACK_RED_ZONE,
        crate::parser::STACK_GROW,
        || expr_poisons_inner(e),
    )
}

fn expr_poisons_inner(e: &Expr) -> bool {
    match e {
        Expr::Ident(n) => &**n == "eval",
        Expr::Unary(UnOp::Delete, t) => matches!(&**t, Expr::Ident(_)) || expr_poisons(t),
        Expr::Unary(_, a)
        | Expr::Spread(a)
        | Expr::Await(a)
        | Expr::ImportCall(a)
        | Expr::SuperIndex(a) => expr_poisons(a),
        Expr::Binary(_, a, b) | Expr::Assign(a, _, b) | Expr::Index(a, b) => {
            expr_poisons(a) || expr_poisons(b)
        }
        Expr::Cond(a, b, c) => expr_poisons(a) || expr_poisons(b) || expr_poisons(c),
        Expr::Member(o, _) => expr_poisons(o),
        Expr::Call(f, args, _) | Expr::New(f, args, _) => {
            expr_poisons(f) || args.iter().any(expr_poisons)
        }
        Expr::SuperCall(args) => args.iter().any(expr_poisons),
        Expr::Seq(es) | Expr::Template { exprs: es, .. } => es.iter().any(expr_poisons),
        Expr::TaggedTemplate { tag, exprs, .. } => {
            expr_poisons(tag) || exprs.iter().any(expr_poisons)
        }
        Expr::Array(items) => items.iter().flatten().any(expr_poisons),
        Expr::Object(props) => props.iter().any(|p| match p {
            ObjProp::Prop(k, v) => {
                matches!(k, PropKey::Computed(e) if expr_poisons(e)) || expr_poisons(v)
            }
            ObjProp::Spread(e) => expr_poisons(e),
            ObjProp::Getter(k, _) | ObjProp::Setter(k, _) => {
                matches!(k, PropKey::Computed(e) if expr_poisons(e))
            }
        }),
        Expr::Chain { head, steps } => {
            expr_poisons(head)
                || steps.iter().any(|s| match s {
                    ChainStep::Index(e) | ChainStep::OptIndex(e) => expr_poisons(e),
                    ChainStep::Call(args) | ChainStep::OptCall(args) => {
                        args.iter().any(expr_poisons)
                    }
                    ChainStep::Member(_) | ChainStep::OptMember(_) => false,
                })
        }
        Expr::Yield { arg, .. } => arg.as_deref().is_some_and(expr_poisons),
        // Corps imbriqués : leur poison est le leur (`_`).
        _ => false,
    }
}

/// Noms d'un motif (`var`/`let`/param/catch…).
fn pat_names(p: &Pat, out: &mut HashSet<Rc<str>>) {
    stacker::maybe_grow(
        crate::parser::STACK_RED_ZONE,
        crate::parser::STACK_GROW,
        || {
            pat_names_inner(p, out);
        },
    );
}

/// JS-C502 — noms d'un motif en **ordre de liaison** (première
/// occurrence dedupée) — l'ordre canonique des slots.
fn pat_names_ord(p: &Pat, out: &mut Vec<Rc<str>>) {
    stacker::maybe_grow(
        crate::parser::STACK_RED_ZONE,
        crate::parser::STACK_GROW,
        || {
            pat_names_ord_inner(p, out);
        },
    );
}

fn pat_names_ord_inner(p: &Pat, out: &mut Vec<Rc<str>>) {
    match p {
        Pat::Ident(n) => {
            if !out.iter().any(|x| x == n) {
                out.push(n.clone());
            }
        }
        Pat::Lvalue(_) => {}
        Pat::Rest(p) | Pat::Default(p, _) => pat_names_ord(p, out),
        Pat::Array(elems) => {
            for e in elems.iter().flatten() {
                pat_names_ord(e, out);
            }
        }
        Pat::Object { props, rest } => {
            for (_, p) in props {
                pat_names_ord(p, out);
            }
            if let Some(r) = rest {
                pat_names_ord(r, out);
            }
        }
    }
}

/// JS-C502 — noms d'un scope **bloc** ordonnés : `let`/`const`/
/// `using`/`class` en ordre source = ordre de déclaration à
/// l'exécution (la pré-déclare fige les positions même si des
/// chemins sauteraient des déclarateurs).
fn block_names_ord(stmts: &[Stmt]) -> Vec<Rc<str>> {
    let mut out = Vec::new();
    for s in stmts {
        match s {
            Stmt::Lexical { decls, .. } | Stmt::Using { decls, .. } => {
                for d in decls {
                    pat_names_ord(&d.pat, &mut out);
                }
            }
            Stmt::ClassDecl(cl) => {
                if let Some(n) = &cl.name {
                    if !out.iter().any(|x| x == n) {
                        out.push(n.clone());
                    }
                }
            }
            _ => {}
        }
    }
    out
}

fn pat_names_inner(p: &Pat, out: &mut HashSet<Rc<str>>) {
    match p {
        Pat::Ident(n) => {
            out.insert(n.clone());
        }
        Pat::Lvalue(_) => {}
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
    }
}

/// Noms du scope **fonction** : `var` + `function` remontent de
/// toute la profondeur de blocs (commentaire `hoist_vars_only`
/// d'`exec.rs`), `let`/`const`/`class`/`using` seulement au
/// niveau supérieur du corps (ils vivent dans l'env `Local`).
fn fn_scope_names(lit: &FnLit, body: &[Stmt]) -> HashSet<Rc<str>> {
    let mut out = HashSet::new();
    for p in &lit.params {
        pat_names(&p.pat, &mut out);
    }
    collect_names(body, &mut out, true);
    out
}

/// JS-C503 — liste canonique ordonnée du scope `Local`
/// **synchrone** : réplique exacte de l'ordre d'insertion du
/// prologue `call_script_body` — `params` (ordre de motifs —
/// **baissés** pour gen/async, `lit.params` sinon),
/// `"arguments"` si `!arrow && LIT_USES_ARGS`, `new.target`
/// (nom privé [`NEW_TARGET`]) si `!arrow && LIT_USES_NT`,
/// noms hoistés dans l'ordre de `hoist`, puis `let`/`const`/
/// `class`/`using` du niveau corps (ordre source). Première
/// occurrence dedupée (`env_declare`/`insert` sautent un nom
/// déjà présent — le slot de la première liaison gagne).
fn fn_scope_ord(lit: &FnLit, params: &[Param], body: &[Stmt], lfl: u8) -> Vec<Rc<str>> {
    let mut out: Vec<Rc<str>> = Vec::new();
    for p in params {
        pat_names_ord(&p.pat, &mut out);
    }
    if !lit.is_arrow && lfl & LIT_USES_ARGS != 0 {
        out.push(Rc::from("arguments"));
    }
    if !lit.is_arrow && lfl & LIT_USES_NT != 0 {
        out.push(Rc::from(NEW_TARGET));
    }
    if lfl & LIT_MAY_HOIST != 0 {
        hoist_ord(body, &mut out);
    }
    lex_top_ord(body, &mut out);
    out
}

/// `let`/`const`/`using`/`class` du niveau corps — en ordre
/// source, après les noms hoistés.
fn lex_top_ord(body: &[Stmt], out: &mut Vec<Rc<str>>) {
    for s in body {
        match s {
            Stmt::Lexical { decls, .. } | Stmt::Using { decls, .. } => {
                for d in decls {
                    pat_names_ord(&d.pat, out);
                }
            }
            Stmt::ClassDecl(cl) => {
                if let Some(n) = &cl.name {
                    if !out.iter().any(|x| x == n) {
                        out.push(n.clone());
                    }
                }
            }
            _ => {}
        }
    }
}

/// Réplique de `Engine::hoist` : `var` + `function` en ordre de
/// parcours (les `function` sont déclarées ET initialisées au
/// hoist — même position), autres statements → marche
/// [`hoist_vars_ord_inner`].
fn hoist_ord(body: &[Stmt], out: &mut Vec<Rc<str>>) {
    stacker::maybe_grow(
        crate::parser::STACK_RED_ZONE,
        crate::parser::STACK_GROW,
        || hoist_ord_inner(body, out),
    );
}

fn hoist_ord_inner(body: &[Stmt], out: &mut Vec<Rc<str>>) {
    for s in body {
        match s {
            Stmt::Var(ds) => {
                for d in ds {
                    pat_names_ord(&d.pat, out);
                }
            }
            Stmt::FunctionDecl(l) => {
                if let Some(n) = &l.name {
                    if !out.iter().any(|x| x == n) {
                        out.push(n.clone());
                    }
                }
            }
            Stmt::Export(crate::ast::ExportDecl::Decl(inner)) => {
                hoist_ord(std::slice::from_ref(&**inner), out);
            }
            other => hoist_vars_ord_inner(std::slice::from_ref(other), out),
        }
    }
}

/// Réplique de `Engine::hoist_vars_only_inner` — `var` +
/// `function` remontés de toute la profondeur des blocs.
fn hoist_vars_ord_inner(body: &[Stmt], out: &mut Vec<Rc<str>>) {
    stacker::maybe_grow(
        crate::parser::STACK_RED_ZONE,
        crate::parser::STACK_GROW,
        || hoist_vars_ord_inner_shallow(body, out),
    );
}

fn hoist_vars_ord_inner_shallow(body: &[Stmt], out: &mut Vec<Rc<str>>) {
    for s in body {
        match s {
            Stmt::Var(ds) => {
                for d in ds {
                    pat_names_ord(&d.pat, out);
                }
            }
            Stmt::FunctionDecl(l) => {
                if let Some(n) = &l.name {
                    if !out.iter().any(|x| x == n) {
                        out.push(n.clone());
                    }
                }
            }
            Stmt::Block(b) => hoist_vars_ord_inner(b, out),
            Stmt::If { then, else_, .. } => {
                hoist_vars_ord_inner(std::slice::from_ref(then.as_ref()), out);
                if let Some(e) = else_ {
                    hoist_vars_ord_inner(std::slice::from_ref(e.as_ref()), out);
                }
            }
            Stmt::While { body, .. }
            | Stmt::DoWhile { body, .. }
            | Stmt::With { body, .. }
            | Stmt::Labeled { body, .. } => {
                hoist_vars_ord_inner(std::slice::from_ref(body.as_ref()), out);
            }
            Stmt::For { init, body, .. } => {
                if let Some(ForInit::Var(ds)) = init {
                    for d in ds {
                        pat_names_ord(&d.pat, out);
                    }
                }
                hoist_vars_ord_inner(std::slice::from_ref(body.as_ref()), out);
            }
            Stmt::ForIn { target, body, .. } | Stmt::ForOf { target, body, .. } => {
                if let ForTarget::Var(p) = target {
                    pat_names_ord(p, out);
                }
                hoist_vars_ord_inner(std::slice::from_ref(body.as_ref()), out);
            }
            Stmt::Try {
                block,
                catch,
                finally,
            } => {
                hoist_vars_ord_inner(block, out);
                if let Some((_, b)) = catch {
                    hoist_vars_ord_inner(b, out);
                }
                if let Some(b) = finally {
                    hoist_vars_ord_inner(b, out);
                }
            }
            Stmt::Switch { cases, .. } => {
                for c in cases {
                    hoist_vars_ord_inner(&c.body, out);
                }
            }
            Stmt::Export(crate::ast::ExportDecl::Decl(inner)) => {
                hoist_vars_ord_inner(std::slice::from_ref(inner.as_ref()), out);
            }
            _ => {}
        }
    }
}

/// `top = true` : tous les déclarateurs comptent (niveau supérieur
/// du corps fonction ou du script) ; `false` (blocs imbriqués) :
/// seuls `var` + `function` remontent vers le scope fonction.
fn collect_names(stmts: &[Stmt], out: &mut HashSet<Rc<str>>, top: bool) {
    stacker::maybe_grow(
        crate::parser::STACK_RED_ZONE,
        crate::parser::STACK_GROW,
        || {
            collect_names_inner(stmts, out, top);
        },
    );
}

fn collect_names_inner(stmts: &[Stmt], out: &mut HashSet<Rc<str>>, top: bool) {
    for s in stmts {
        match s {
            Stmt::Var(ds) => {
                for d in ds {
                    pat_names(&d.pat, out);
                }
            }
            Stmt::Lexical { decls: ds, .. } | Stmt::Using { decls: ds, .. } => {
                if top {
                    for d in ds {
                        pat_names(&d.pat, out);
                    }
                }
            }
            Stmt::FunctionDecl(l) => {
                if let Some(n) = &l.name {
                    out.insert(n.clone());
                }
            }
            Stmt::ClassDecl(cl) => {
                if top {
                    if let Some(n) = &cl.name {
                        out.insert(n.clone());
                    }
                }
            }
            Stmt::Block(b) => collect_names(b, out, false),
            Stmt::If { then, else_, .. } => {
                collect_names(std::slice::from_ref(then.as_ref()), out, false);
                if let Some(e) = else_ {
                    collect_names(std::slice::from_ref(e.as_ref()), out, false);
                }
            }
            Stmt::While { body, .. } | Stmt::DoWhile { body, .. } | Stmt::Labeled { body, .. } => {
                collect_names(std::slice::from_ref(body.as_ref()), out, false);
            }
            Stmt::For { init, body, .. } => {
                if let Some(ForInit::Var(ds)) = init {
                    for d in ds {
                        pat_names(&d.pat, out);
                    }
                }
                collect_names(std::slice::from_ref(body.as_ref()), out, false);
            }
            Stmt::ForIn { target, body, .. } | Stmt::ForOf { target, body, .. } => {
                if let ForTarget::Var(p) = target {
                    pat_names(p, out);
                }
                collect_names(std::slice::from_ref(body.as_ref()), out, false);
            }
            Stmt::Try {
                block,
                catch,
                finally,
            } => {
                collect_names(block, out, false);
                if let Some((_, b)) = catch {
                    collect_names(b, out, false);
                }
                if let Some(b) = finally {
                    collect_names(b, out, false);
                }
            }
            Stmt::Switch { cases, .. } => {
                for c in cases {
                    collect_names(&c.body, out, false);
                }
            }
            _ => {}
        }
    }
}

impl Resolver {
    /// Annotation d'un `Expr::Ident` lu — depth = nombre de
    /// niveaux à sauter (0 = env courant) ; `slot` = index dans
    /// la liste canonique du niveau (`NO_SLOT` si la portée
    /// n'est pas pré-déclarée). Valeur packée `(d<<16)|slot`.
    fn annotate(&mut self, e: &Expr, name: &Rc<str>) {
        if self.suppress > 0 || &**name == "arguments" {
            return;
        }
        let n = self.scopes.len();
        for i in (0..n).rev() {
            if self.scopes[i].names.contains(name) {
                // Un niveau empoisonné entre l'usage et la
                // liaison (comprise) peut intercepter le nom —
                // pas d'annotation.
                if self.scopes[i..].iter().any(|s| s.poisoned) {
                    return;
                }
                if let Ok(d) = u16::try_from(n - 1 - i) {
                    let slot = self.scopes[i]
                        .order
                        .as_deref()
                        .and_then(|o| o.iter().position(|x| x == name))
                        .and_then(|p| u16::try_from(p).ok())
                        .unwrap_or(NO_SLOT);
                    self.map.insert(
                        std::ptr::from_ref::<Expr>(e) as usize,
                        ((d as u32) << 16) | slot as u32,
                    );
                }
                return;
            }
        }
        // Non trouvé : global ou capture inter-fonction — marche
        // dynamique inchangée.
    }

    fn push(&mut self, names: HashSet<Rc<str>>, poisoned: bool) {
        self.scopes.push(Scope::new(names, None, poisoned));
    }

    /// JS-C502 — pousse un scope **pré-déclaré par slots** :
    /// `site` identifie le créateur d'env côté runtime (clé de
    /// `names`), `order` est la liste canonique ; l'ensemble est
    /// reconstruit pour la recherche de nom.
    fn push_slots(&mut self, site: usize, order: Rc<[Rc<str>]>, poisoned: bool) {
        let names: HashSet<Rc<str>> = order.iter().cloned().collect();
        self.names.insert(site, order.clone());
        self.scopes.push(Scope::new(names, Some(order), poisoned));
    }

    fn pop(&mut self) {
        self.scopes.pop();
    }

    /// Corps d'une fonction : scope `Local` (params + hoisting +
    /// lexicaux de niveau corps) puis descente des statements.
    /// `poisoned` si le corps contient `eval`/`delete`.
    /// `lowered_params` — params baissés quand `body` est le corps
    /// baissé d'un gen/async (sinon `None` : canon incertain →
    /// depth-only, émis à l'appel du lit sur son propre corps).
    fn walk_fn(&mut self, lit: &FnLit, body: &[Stmt], lowered_params: Option<&[Param]>) {
        let poisoned = body_poisons(body);
        // Filet défensif : une portée poussée et oubliée dans un
        // bras de statement ne doit jamais décaler les profondeurs
        // des fonctions sœurs (régression `for-in`/`for-of` B1 du
        // vol 811).
        let base = self.scopes.len();
        // JS-C534d — le canon est émis pour TOUT corps dont on
        // connaît les vrais params (synchrone, ou gen/async avec
        // `lowered_params`) — **y compris `poisoned`** : le flag
        // `poisoned` ne gouverne que la *suppression des
        // annotations* d'idents le traversant (eval peut injecter
        // des noms invisibles statiquement) ; côté runtime, un
        // env câblé + `extra` sert déjà les liaisons injectées
        // (le routage canon/extra ne peut pas masquer un nom
        // canonique — l'invariant est asserté). `envu:` a nommé
        // les 695 k hits `extra` du call env — les scopes
        // poisoned yt (BotGuard abuse d'`eval`) étaient le
        // réservoir.
        // Gen/async **sans** params baissés (walk imbriqué sur
        // corps source) : canon incertain → pas d'émission (la
        // clé émise serait morte — le runtime demande la clé du
        // corps baissé à l'appel du lit).
        if !(lit.is_gen || lit.is_async) || lowered_params.is_some() {
            // JS-C503 — slots du scope `Local` : liste canonique
            // alignée sur le prologue (params → arguments? →
            // new.target? → hoist → lex), clé `body.as_ptr()`.
            let lfl = crate::interp::compute_lit_flags(lit);
            self.push_slots(
                body.as_ptr() as usize,
                Rc::from(fn_scope_ord(
                    lit,
                    lowered_params.unwrap_or(&lit.params),
                    body,
                    lfl,
                )),
                poisoned,
            );
        } else {
            self.push(fn_scope_names(lit, body), poisoned);
        }
        // `= défaut` de paramètres : évalués dans l'env `Local`.
        for p in &lit.params {
            self.walk_pat(&p.pat);
        }
        self.walk_stmts(body);
        self.scopes.truncate(base);
    }

    /// Corps d'un `FnLit` quel qu'en soit la forme — `Deferred`
    /// n'est jamais appelé ici (le resolveur n'existe que si le
    /// corps est matérialisé).
    fn enter_lit(&mut self, lit: &FnLit) {
        if let FnBody::Parsed(b) = &lit.body {
            // Lit imbriqué : le walk est sur son corps **source**
            // — jamais de params baissés (son canon gen/async
            // sera émis quand le lit sera résolu à son appel).
            self.walk_fn(lit, b, None);
        }
    }

    /// Expressions dans un motif (défauts, clés calculées,
    /// lvalues de déstructuration).
    fn walk_pat(&mut self, p: &Pat) {
        match p {
            Pat::Ident(_) => {}
            Pat::Lvalue(e) => self.walk_expr(e),
            Pat::Rest(p) => self.walk_pat(p),
            Pat::Default(p, e) => {
                self.walk_pat(p);
                self.walk_expr(e);
            }
            Pat::Array(elems) => {
                for e in elems.iter().flatten() {
                    self.walk_pat(e);
                }
            }
            Pat::Object { props, rest } => {
                for (k, p) in props {
                    if let PropKey::Computed(e) = k {
                        self.walk_expr(e);
                    }
                    self.walk_pat(p);
                }
                if let Some(r) = rest {
                    self.walk_pat(r);
                }
            }
        }
    }

    fn walk_decl(&mut self, d: &VarDecl) {
        self.walk_pat(&d.pat);
        if let Some(i) = &d.init {
            self.walk_expr(i);
        }
    }

    fn walk_stmts(&mut self, stmts: &[Stmt]) {
        for s in stmts {
            self.walk_stmt(s);
        }
    }

    fn walk_stmt(&mut self, s: &Stmt) {
        stacker::maybe_grow(
            crate::parser::STACK_RED_ZONE,
            crate::parser::STACK_GROW,
            || {
                self.walk_stmt_inner(s);
            },
        );
    }

    fn walk_stmt_inner(&mut self, s: &Stmt) {
        match s {
            Stmt::Empty
            | Stmt::Break(_)
            | Stmt::Continue(_)
            | Stmt::Return(None)
            | Stmt::Import { .. }
            | Stmt::Export(_) => {}
            Stmt::Expr(e) | Stmt::Return(Some(e)) | Stmt::Throw(e) => self.walk_expr(e),
            Stmt::Var(ds) | Stmt::Lexical { decls: ds, .. } | Stmt::Using { decls: ds, .. } => {
                for d in ds {
                    self.walk_decl(d);
                }
            }
            Stmt::FunctionDecl(l) => self.enter_lit(l),
            Stmt::ClassDecl(cl) => self.walk_class(cl),
            // `{…}` → `Env::block` inconditionnel (exec.rs:534).
            // JS-C502 : noms canoniques ordonnés → pré-déclare.
            Stmt::Block(b) => {
                self.push_slots(b.as_ptr() as usize, Rc::from(block_names_ord(b)), false);
                self.walk_stmts(b);
                self.pop();
            }
            Stmt::If { cond, then, else_ } => {
                self.walk_expr(cond);
                self.walk_stmt(then);
                if let Some(e) = else_ {
                    self.walk_stmt(e);
                }
            }
            Stmt::While { cond, body } | Stmt::DoWhile { body, cond } => {
                self.walk_expr(cond);
                self.walk_stmt(body);
            }
            Stmt::For {
                init,
                test,
                update,
                body,
            } => {
                // `for(let…)` → tête `Env::block` (un niveau,
                // identique pour les copies par itération —
                // exec.rs:661/747/766, même clé `decls.as_ptr()`).
                let has_head = matches!(init, Some(ForInit::Lexical { .. }));
                if has_head {
                    let mut ord = Vec::new();
                    if let Some(ForInit::Lexical { decls, .. }) = init {
                        for d in decls {
                            pat_names_ord(&d.pat, &mut ord);
                        }
                        self.push_slots(decls.as_ptr() as usize, Rc::from(ord), false);
                    }
                }
                match init {
                    Some(ForInit::Var(ds) | ForInit::Lexical { decls: ds, .. }) => {
                        for d in ds {
                            self.walk_decl(d);
                        }
                    }
                    Some(ForInit::Expr(e)) => self.walk_expr(e),
                    None => {}
                }
                if let Some(t) = test {
                    self.walk_expr(t);
                }
                if let Some(u) = update {
                    self.walk_expr(u);
                }
                self.walk_stmt(body);
                if has_head {
                    self.pop();
                }
            }
            Stmt::ForIn {
                target,
                obj,
                var_init,
                body,
            } => {
                self.walk_expr(obj);
                if let Some(v) = var_init {
                    self.walk_expr(v);
                }
                let pushed = matches!(target, ForTarget::Lexical { .. } | ForTarget::Using { .. });
                self.walk_for_target(target);
                self.walk_stmt(body);
                if pushed {
                    self.pop();
                }
            }
            Stmt::ForOf {
                target, iter, body, ..
            } => {
                self.walk_expr(iter);
                let pushed = matches!(target, ForTarget::Lexical { .. } | ForTarget::Using { .. });
                self.walk_for_target(target);
                self.walk_stmt(body);
                if pushed {
                    self.pop();
                }
            }
            // `with` — l'env objet est dans la chaîne : aucune
            // annotation dans le sous-arbre.
            Stmt::With { obj, body } => {
                self.walk_expr(obj);
                self.suppress += 1;
                self.walk_stmt(body);
                self.suppress -= 1;
            }
            Stmt::Labeled { body, .. } => self.walk_stmt(body),

            // Un bloc d'env par clause (exec.rs:1054/1061/1095) —
            // chacun pré-déclaré : le catch lie son paramètre
            // AVANT le corps → liste canonique `[pat…, lex…]`.
            Stmt::Try {
                block,
                catch,
                finally,
            } => {
                self.push_slots(
                    block.as_ptr() as usize,
                    Rc::from(block_names_ord(block)),
                    false,
                );
                self.walk_stmts(block);
                self.pop();
                if let Some((pat, cbody)) = catch {
                    let mut ord = Vec::new();
                    pat_names_ord(pat, &mut ord);
                    for n in block_names_ord(cbody) {
                        if !ord.contains(&n) {
                            ord.push(n);
                        }
                    }
                    self.push_slots(cbody.as_ptr() as usize, Rc::from(ord), false);
                    self.walk_pat(pat);
                    self.walk_stmts(cbody);
                    self.pop();
                }
                if let Some(fbody) = finally {
                    self.push_slots(
                        fbody.as_ptr() as usize,
                        Rc::from(block_names_ord(fbody)),
                        false,
                    );
                    self.walk_stmts(fbody);
                    self.pop();
                }
            }
            // Tous les `case` partagent UN bloc déclaratif
            // (exec.rs:1131). Les tests sont évalués avant la
            // création de l'env → scope englobant.
            Stmt::Switch { disc, cases } => {
                self.walk_expr(disc);
                for c in cases {
                    if let Some(t) = &c.test {
                        self.walk_expr(t);
                    }
                }
                let mut ord = Vec::new();
                for c in cases {
                    for st in &c.body {
                        if let Stmt::Lexical { decls, .. } | Stmt::Using { decls, .. } = st {
                            for d in decls {
                                pat_names_ord(&d.pat, &mut ord);
                            }
                        } else if let Stmt::ClassDecl(cl) = st {
                            if let Some(n) = &cl.name {
                                if !ord.iter().any(|x| x == n) {
                                    ord.push(n.clone());
                                }
                            }
                        }
                    }
                }
                self.push_slots(cases.as_ptr() as usize, Rc::from(ord), false);
                for c in cases {
                    self.walk_stmts(&c.body);
                }
                self.pop();
            } // Modules : interceptés en amont (`resolve_lit`).
        }
    }

    fn walk_for_target(&mut self, t: &ForTarget) {
        match t {
            // Cible lexicale → `Env::block` par itération
            // (exec.rs:811/899/970) — clé `ptr(ForTarget)`,
            // noms = le motif en ordre de liaison.
            ForTarget::Lexical { pat, .. } | ForTarget::Using { pat, .. } => {
                let mut ord = Vec::new();
                pat_names_ord(pat, &mut ord);
                self.push_slots(
                    std::ptr::from_ref::<ForTarget>(t) as usize,
                    Rc::from(ord),
                    false,
                );
                self.walk_pat(pat);
            }
            ForTarget::Var(p) => self.walk_pat(p),
            ForTarget::Expr(e) => self.walk_expr(e),
        }
    }

    fn walk_class(&mut self, cl: &ClassLit) {
        if let Some(s) = &cl.super_ {
            self.walk_expr(s);
        }
        // `cenv` = `Env::local` (binds privés `super`/`home`) —
        // un niveau entre les membres et le scope englobant.
        self.push(HashSet::new(), false);
        for m in &cl.members {
            if let PropKey::Computed(e) = &m.key {
                self.walk_expr(e);
            }
            self.enter_lit(&m.fun);
        }
        self.pop();
    }

    fn walk_expr(&mut self, e: &Expr) {
        stacker::maybe_grow(
            crate::parser::STACK_RED_ZONE,
            crate::parser::STACK_GROW,
            || {
                self.walk_expr_inner(e);
            },
        );
    }

    fn walk_expr_inner(&mut self, e: &Expr) {
        match e {
            Expr::Ident(name) => self.annotate(e, name),
            Expr::Num(_)
            | Expr::BigInt(_)
            | Expr::Str(_)
            | Expr::Bool(_)
            | Expr::Null
            | Expr::This
            | Expr::Regex { .. }
            | Expr::SuperMember(_)
            | Expr::ImportMeta
            | Expr::NewTarget => {}
            Expr::Unary(_, a)
            | Expr::Spread(a)
            | Expr::Await(a)
            | Expr::ImportCall(a)
            | Expr::SuperIndex(a) => self.walk_expr(a),
            Expr::Binary(_, a, b) | Expr::Assign(a, _, b) | Expr::Index(a, b) => {
                self.walk_expr(a);
                self.walk_expr(b);
            }
            Expr::Cond(a, b, c) => {
                self.walk_expr(a);
                self.walk_expr(b);
                self.walk_expr(c);
            }
            Expr::Member(o, _) => self.walk_expr(o),
            Expr::Call(f, args, _) | Expr::New(f, args, _) => {
                self.walk_expr(f);
                for a in args {
                    self.walk_expr(a);
                }
            }
            Expr::SuperCall(args) => {
                for a in args {
                    self.walk_expr(a);
                }
            }
            Expr::Seq(es) | Expr::Template { exprs: es, .. } => {
                for e in es {
                    self.walk_expr(e);
                }
            }
            Expr::TaggedTemplate { tag, exprs, .. } => {
                self.walk_expr(tag);
                for e in exprs {
                    self.walk_expr(e);
                }
            }
            Expr::Array(items) => {
                for it in items.iter().flatten() {
                    self.walk_expr(it);
                }
            }
            Expr::Object(props) => {
                for p in props {
                    match p {
                        ObjProp::Prop(k, v) => {
                            if let PropKey::Computed(e) = k {
                                self.walk_expr(e);
                            }
                            self.walk_expr(v);
                        }
                        ObjProp::Spread(e) => self.walk_expr(e),
                        ObjProp::Getter(k, f) | ObjProp::Setter(k, f) => {
                            if let PropKey::Computed(e) = k {
                                self.walk_expr(e);
                            }
                            self.enter_lit(f);
                        }
                    }
                }
            }
            // Function-expression nommée : `Env::local` portant le
            // nom propre entre le corps et le scope englobant
            // (eval.rs:313) — slot 0 = le nom.
            Expr::Function(lit) => {
                if let Some(n) = &lit.name {
                    self.push_slots(Rc::as_ptr(lit) as usize, Rc::from([n.clone()]), false);
                    self.enter_lit(lit);
                    self.pop();
                } else {
                    self.enter_lit(lit);
                }
            }
            Expr::Class(cl) => self.walk_class(cl),
            Expr::Chain { head, steps } => {
                self.walk_expr(head);
                for s in steps {
                    match s {
                        ChainStep::Index(e) | ChainStep::OptIndex(e) => self.walk_expr(e),
                        ChainStep::Call(args) | ChainStep::OptCall(args) => {
                            for a in args {
                                self.walk_expr(a);
                            }
                        }
                        ChainStep::Member(_) | ChainStep::OptMember(_) => {}
                    }
                }
            }
            Expr::Yield { arg, .. } => {
                if let Some(a) = arg {
                    self.walk_expr(a);
                }
            }
        }
    }
}
