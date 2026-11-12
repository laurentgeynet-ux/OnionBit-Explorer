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

use crate::ast::{
    ChainStep, ClassLit, Expr, FnBody, FnLit, ForInit, ForTarget, ObjProp, Pat, PropKey, Stmt,
    UnOp, VarDecl,
};

/// Portée statique — une entrée = un niveau d'env au runtime.
struct Scope {
    /// Noms liés à ce niveau.
    names: HashSet<Rc<str>>,
    /// `eval`/`delete` dans ce corps de fonction : les résolutions
    /// qui atterrissent sur ce niveau ou le traversent sont
    /// sautées (le scope peut gagner des noms dynamiquement).
    poisoned: bool,
}

impl Scope {
    fn new(names: HashSet<Rc<str>>, poisoned: bool) -> Self {
        Self { names, poisoned }
    }
}

/// Marcheur : accumule les annotations `ptr(Expr) → profondeur`.
pub(crate) struct Resolver {
    /// Sortie — `ptr(Expr::Ident) → niveaux à sauter`.
    pub map: HashMap<usize, u16>,
    /// Pile de portées — `last()` = env courant (index 0 de
    /// profondeur). Parallèle : `poisoned` par entrée.
    scopes: Vec<Scope>,
    /// > 0 dans un sous-arbre `with` — aucune annotation.
    suppress: u32,
}

/// Résout les identifiants du corps d'un `FnLit` — scope propre
/// (paramètres + hoisting) puis descente dans les sous-portées et
/// les fonctions imbriquées déjà `Parsed`.
pub(crate) fn resolve_lit(lit: &FnLit, body: &[Stmt]) -> HashMap<usize, u16> {
    let mut r = Resolver {
        map: HashMap::new(),
        scopes: Vec::new(),
        suppress: 0,
    };
    // Modules : liaisons installées à l'instanciation, chaîne
    // d'env non modélisée — aucune annotation.
    if body
        .iter()
        .any(|s| matches!(s, Stmt::Import { .. } | Stmt::Export(_)))
    {
        return r.map;
    }
    r.walk_fn(lit, body);
    r.map
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
        Expr::Call(f, args) | Expr::New(f, args) => {
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

/// `top = true` : tous les déclarateurs comptent (niveau supérieur
/// du corps fonction ou du script) ; `false` (blocs imbriqués) :
/// seuls `var` + `function` remontent vers le scope fonction.
fn collect_names(stmts: &[Stmt], out: &mut HashSet<Rc<str>>, top: bool) {
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

/// Noms d'un scope **bloc** : `let`/`const`/`class`/`using` de ce
/// niveau seulement (les `function` restent fonction-scope chez
/// nous, `var` est déjà collecté par `collect_names`).
fn block_names(stmts: &[Stmt]) -> HashSet<Rc<str>> {
    let mut out = HashSet::new();
    for s in stmts {
        match s {
            Stmt::Lexical { decls, .. } | Stmt::Using { decls, .. } => {
                for d in decls {
                    pat_names(&d.pat, &mut out);
                }
            }
            Stmt::ClassDecl(cl) => {
                if let Some(n) = &cl.name {
                    out.insert(n.clone());
                }
            }
            _ => {}
        }
    }
    out
}

impl Resolver {
    /// Annotation d'un `Expr::Ident` lu — depth = nombre de
    /// niveaux à sauter (0 = env courant).
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
                    self.map.insert(std::ptr::from_ref::<Expr>(e) as usize, d);
                }
                return;
            }
        }
        // Non trouvé : global ou capture inter-fonction — marche
        // dynamique inchangée.
    }

    fn push(&mut self, names: HashSet<Rc<str>>, poisoned: bool) {
        self.scopes.push(Scope::new(names, poisoned));
    }

    fn pop(&mut self) {
        self.scopes.pop();
    }

    /// Corps d'une fonction : scope `Local` (params + hoisting +
    /// lexicaux de niveau corps) puis descente des statements.
    /// `poisoned` si le corps contient `eval`/`delete`.
    fn walk_fn(&mut self, lit: &FnLit, body: &[Stmt]) {
        let poisoned = body_poisons(body);
        // Filet défensif : une portée poussée et oubliée dans un
        // bras de statement ne doit jamais décaler les profondeurs
        // des fonctions sœurs (régression `for-in`/`for-of` B1 du
        // vol 811).
        let base = self.scopes.len();
        self.push(fn_scope_names(lit, body), poisoned);
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
            self.walk_fn(lit, b);
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
            // `{…}` → `Env::block` inconditionnel (exec.rs:500).
            Stmt::Block(b) => {
                self.push(block_names(b), false);
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
                // identique pour les copies par itération).
                let has_head = matches!(init, Some(ForInit::Lexical { .. }));
                if has_head {
                    let mut ns = HashSet::new();
                    if let Some(ForInit::Lexical { decls, .. }) = init {
                        for d in decls {
                            pat_names(&d.pat, &mut ns);
                        }
                    }
                    self.push(ns, false);
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

            // Un bloc d'env par clause (exec.rs:1020/1027/1061).
            Stmt::Try {
                block,
                catch,
                finally,
            } => {
                self.push(block_names(block), false);
                self.walk_stmts(block);
                self.pop();
                if let Some((pat, cbody)) = catch {
                    let mut ns = block_names(cbody);
                    pat_names(pat, &mut ns);
                    self.push(ns, false);
                    self.walk_pat(pat);
                    self.walk_stmts(cbody);
                    self.pop();
                }
                if let Some(fbody) = finally {
                    self.push(block_names(fbody), false);
                    self.walk_stmts(fbody);
                    self.pop();
                }
            }
            // Tous les `case` partagent UN bloc déclaratif
            // (exec.rs:1097). Les tests sont évalués avant la
            // création de l'env → scope englobant.
            Stmt::Switch { disc, cases } => {
                self.walk_expr(disc);
                for c in cases {
                    if let Some(t) = &c.test {
                        self.walk_expr(t);
                    }
                }
                let mut ns = HashSet::new();
                for c in cases {
                    for st in &c.body {
                        if let Stmt::Lexical { decls, .. } | Stmt::Using { decls, .. } = st {
                            for d in decls {
                                pat_names(&d.pat, &mut ns);
                            }
                        } else if let Stmt::ClassDecl(cl) = st {
                            if let Some(n) = &cl.name {
                                ns.insert(n.clone());
                            }
                        }
                    }
                }
                self.push(ns, false);
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
            // (exec.rs:777/865/936).
            ForTarget::Lexical { pat, .. } | ForTarget::Using { pat, .. } => {
                let mut ns = HashSet::new();
                pat_names(pat, &mut ns);
                self.push(ns, false);
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
            Expr::Call(f, args) | Expr::New(f, args) => {
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
            // (eval.rs:307).
            Expr::Function(lit) => {
                if let Some(n) = &lit.name {
                    let mut ns = HashSet::new();
                    ns.insert(n.clone());
                    self.push(ns, false);
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
