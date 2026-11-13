// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! JS-C413 — `fnat` : « fonction à `l:c` » (forensique murs
//! terrain). Les sondes nomment désormais un *site*
//! (`kevlar_base:1484:39` → consommateur `5461:34`, offsets
//! bitmovin…) ; l'étape suivante est de **lire** le code à
//! l'offset — le moteur est son propre décompilateur : spans
//! `FnLit` + `src_text` verbatim.
//!
//! `fn_chain_at` parse la source et retourne la chaîne des
//! littéraux fonction dont le span contient `l:c`, de
//! l'englobant au plus profond. Les corps différés
//! (`FnBody::Deferred` — la quasi-totalité des corps de
//! bundle, ≥24 tokens) sont matérialisés à la demande via
//! `get_or_parse` : seules les fonctions dont le span contient
//! l'offset sont sous-parsées (descente bornée à la
//! profondeur d'emboîtement, jamais le bundle entier).
//!
//! Outil associé : `src/bin/fnat.rs`.

use crate::ast::{
    ClassLit, ExportDecl, Expr, FnBody, FnLit, ForInit, ForTarget, ObjProp, Param, Pat, Program,
    PropKey, Stmt, SwitchCase, VarDecl,
};
use std::rc::Rc;

/// Un maillon de la chaîne « fonction englobante » — tout ce
/// que l'outil a besoin de rapporter (type neutre : `FnLit`
/// reste interne).
#[derive(Debug)]
pub struct FnHit {
    /// Nom (`None` = anonyme/IIFE).
    pub name: Option<String>,
    /// Position `(ligne, colonne)` 1-based du littéral.
    pub pos: (usize, usize),
    /// Nom de la source (`None` = sans nom).
    pub src: Option<String>,
    /// Drapeaux (`=>`, `async`, `function*`).
    pub is_arrow: bool,
    /// Voir [`Self::is_arrow`].
    pub is_async: bool,
    /// Voir [`Self::is_arrow`].
    pub is_gen: bool,
    /// Source verbatim du littéral, tranchée par `span`.
    pub excerpt: String,
}

/// Chaîne des `FnLit` englobants `line:col` (1-based, en
/// **caractères** comme les spans) — `[englobant, …, plus
/// profond]`. Vide si l'offset n'est dans aucune fonction
/// (top-level, donnée, source non parsable).
///
/// `#[doc(hidden)]` : API d'outillage forensique — exposée
/// pour `src/bin/fnat.rs` et les tests, pas un contrat moteur.
#[doc(hidden)]
#[must_use]
pub fn fn_chain_at(src: &str, line: u32, col: u32) -> Vec<FnHit> {
    let off = line_col_to_char(src, line, col);
    let Ok(prog) = crate::parser::parse_in(src, false, crate::parser::DEFAULT_NEST_LIMIT, None)
    else {
        return Vec::new();
    };
    let mut chain: Vec<Rc<FnLit>> = Vec::new();
    stacker::maybe_grow(
        crate::parser::STACK_RED_ZONE,
        crate::parser::STACK_GROW,
        || find_in_program(&prog, off, &mut chain),
    );
    chain
        .iter()
        .map(|lit| {
            let excerpt = lit
                .src_text
                .as_ref()
                .map(|s| {
                    let (lo, hi) = lit.span;
                    s.chars().skip(lo).take(hi.saturating_sub(lo)).collect()
                })
                .unwrap_or_default();
            FnHit {
                name: lit.name.as_deref().map(str::to_string),
                pos: lit.pos,
                src: lit.src.as_deref().map(str::to_string),
                is_arrow: lit.is_arrow,
                is_async: lit.is_async,
                is_gen: lit.is_gen,
                excerpt,
            }
        })
        .collect()
}

/// `l:c` 1-based → offset en caractères dans `src` (hors borne
/// → fin — aucun span ne le contient). Les sauts de ligne
/// comptent sur `\n` (les spans sont en caractères lexer).
fn line_col_to_char(src: &str, line: u32, col: u32) -> usize {
    let mut l = 1usize;
    let mut c = 1usize;
    for (i, ch) in src.chars().enumerate() {
        if l as u64 == u64::from(line) && c as u64 == u64::from(col) {
            return i;
        }
        if ch == '\n' {
            l += 1;
            c = 1;
        } else {
            c += 1;
        }
    }
    src.chars().count()
}

/// `off` est-il dans le span (inclusif des deux bornes —
/// `span.1` = `}` final inclus) ?
fn contains(lit: &FnLit, off: usize) -> bool {
    lit.src_text.is_some() && lit.span.0 <= off && off <= lit.span.1
}

fn find_in_program(prog: &Program, off: usize, chain: &mut Vec<Rc<FnLit>>) {
    find_in_stmts(&prog.body, off, chain);
}

fn find_in_stmts(stmts: &[Stmt], off: usize, chain: &mut Vec<Rc<FnLit>>) {
    for s in stmts {
        find_in_stmt(s, off, chain);
    }
}

/// Descend dans un littéral fonction si `off` est dans son
/// span : empile le maillon, matérialise le corps différé,
/// descend récursivement (params compris — les défauts
/// peuvent porter des fonctions).
fn descend_fn(lit: &Rc<FnLit>, off: usize, chain: &mut Vec<Rc<FnLit>>) {
    if !contains(lit, off) {
        return;
    }
    chain.push(lit.clone());
    stacker::maybe_grow(
        crate::parser::STACK_RED_ZONE,
        crate::parser::STACK_GROW,
        || {
            match &lit.body {
                FnBody::Parsed(b) => find_in_stmts(b, off, chain),
                FnBody::Deferred(d) => {
                    // Corps ≥24 tokens non sous-parsés au parse —
                    // matérialisation `OnceCell` (JS-C380/V-5a).
                    if let Ok(b) = d.get_or_parse(lit.is_gen) {
                        find_in_stmts(&b, off, chain);
                    }
                }
            }
            for p in &lit.params {
                find_in_param(p, off, chain);
            }
        },
    );
}

fn find_in_stmt(s: &Stmt, off: usize, chain: &mut Vec<Rc<FnLit>>) {
    match s {
        Stmt::Empty | Stmt::Break(_) | Stmt::Continue(_) | Stmt::Import { .. } => {}
        Stmt::Expr(e) | Stmt::Throw(e) => find_in_expr(e, off, chain),
        Stmt::Return(e) => {
            if let Some(e) = e {
                find_in_expr(e, off, chain);
            }
        }
        Stmt::Var(ds) | Stmt::Lexical { decls: ds, .. } | Stmt::Using { decls: ds, .. } => {
            for d in ds {
                find_in_var_decl(d, off, chain);
            }
        }
        Stmt::FunctionDecl(l) => descend_fn(l, off, chain),
        Stmt::ClassDecl(c) => find_in_class(c, off, chain),
        Stmt::Block(v) => find_in_stmts(v, off, chain),
        Stmt::If { cond, then, else_ } => {
            find_in_expr(cond, off, chain);
            find_in_stmt(then, off, chain);
            if let Some(e) = else_ {
                find_in_stmt(e, off, chain);
            }
        }
        Stmt::While { cond, body } => {
            find_in_expr(cond, off, chain);
            find_in_stmt(body, off, chain);
        }
        Stmt::DoWhile { body, cond } => {
            find_in_stmt(body, off, chain);
            find_in_expr(cond, off, chain);
        }
        Stmt::For {
            init,
            test,
            update,
            body,
        } => {
            if let Some(i) = init {
                find_in_for_init(i, off, chain);
            }
            for e in [test.as_ref(), update.as_ref()].into_iter().flatten() {
                find_in_expr(e, off, chain);
            }
            find_in_stmt(body, off, chain);
        }
        Stmt::ForIn {
            target,
            obj,
            var_init,
            body,
        } => {
            find_in_for_target(target, off, chain);
            find_in_expr(obj, off, chain);
            if let Some(e) = var_init {
                find_in_expr(e, off, chain);
            }
            find_in_stmt(body, off, chain);
        }
        Stmt::ForOf {
            target, iter, body, ..
        } => {
            find_in_for_target(target, off, chain);
            find_in_expr(iter, off, chain);
            find_in_stmt(body, off, chain);
        }
        Stmt::With { obj, body } => {
            find_in_expr(obj, off, chain);
            find_in_stmt(body, off, chain);
        }
        Stmt::Labeled { body, .. } => find_in_stmt(body, off, chain),
        Stmt::Try {
            block,
            catch,
            finally,
        } => {
            find_in_stmts(block, off, chain);
            if let Some((pat, v)) = catch {
                find_in_pat(pat, off, chain);
                find_in_stmts(v, off, chain);
            }
            if let Some(v) = finally {
                find_in_stmts(v, off, chain);
            }
        }
        Stmt::Switch { disc, cases } => {
            find_in_expr(disc, off, chain);
            for SwitchCase { test, body } in cases {
                if let Some(t) = test {
                    find_in_expr(t, off, chain);
                }
                find_in_stmts(body, off, chain);
            }
        }
        Stmt::Export(d) => match d {
            ExportDecl::Decl(s) => find_in_stmt(s, off, chain),
            ExportDecl::Default(e) => find_in_expr(e, off, chain),
            ExportDecl::Named { .. } | ExportDecl::Star { .. } => {}
        },
    }
}

fn find_in_expr(e: &Expr, off: usize, chain: &mut Vec<Rc<FnLit>>) {
    stacker::maybe_grow(
        crate::parser::STACK_RED_ZONE,
        crate::parser::STACK_GROW,
        || match e {
            Expr::Num(_)
            | Expr::BigInt(_)
            | Expr::Str(_)
            | Expr::Bool(_)
            | Expr::Null
            | Expr::Ident(_)
            | Expr::This
            | Expr::Regex { .. }
            | Expr::SuperMember(_)
            | Expr::ImportMeta
            | Expr::NewTarget => {}
            Expr::Function(l) => descend_fn(l, off, chain),
            Expr::Class(c) => find_in_class(c, off, chain),
            Expr::Array(items) => {
                for i in items.iter().flatten() {
                    find_in_expr(i, off, chain);
                }
            }
            Expr::Object(props) => {
                for p in props {
                    match p {
                        ObjProp::Prop(k, v) => {
                            find_in_prop_key(k, off, chain);
                            find_in_expr(v, off, chain);
                        }
                        ObjProp::Spread(v) => find_in_expr(v, off, chain),
                        ObjProp::Getter(k, l) | ObjProp::Setter(k, l) => {
                            find_in_prop_key(k, off, chain);
                            descend_fn(l, off, chain);
                        }
                    }
                }
            }
            Expr::Unary(_, a)
            | Expr::Spread(a)
            | Expr::Await(a)
            | Expr::Member(a, _)
            | Expr::SuperIndex(a)
            | Expr::ImportCall(a) => find_in_expr(a, off, chain),
            Expr::Binary(_, a, b) | Expr::Assign(a, _, b) | Expr::Index(a, b) => {
                find_in_expr(a, off, chain);
                find_in_expr(b, off, chain);
            }
            Expr::Cond(c, t, f) => {
                find_in_expr(c, off, chain);
                find_in_expr(t, off, chain);
                find_in_expr(f, off, chain);
            }
            Expr::Call(f, args, _) | Expr::New(f, args, _) => {
                find_in_expr(f, off, chain);
                for a in args {
                    find_in_expr(a, off, chain);
                }
            }
            Expr::SuperCall(args) => {
                for a in args {
                    find_in_expr(a, off, chain);
                }
            }
            Expr::Seq(items) => {
                for i in items {
                    find_in_expr(i, off, chain);
                }
            }
            Expr::Template { exprs, .. } => {
                for i in exprs {
                    find_in_expr(i, off, chain);
                }
            }
            Expr::TaggedTemplate { tag, exprs, .. } => {
                find_in_expr(tag, off, chain);
                for i in exprs {
                    find_in_expr(i, off, chain);
                }
            }
            Expr::Chain { head, steps } => {
                find_in_expr(head, off, chain);
                for st in steps {
                    match st {
                        crate::ast::ChainStep::Index(i) | crate::ast::ChainStep::OptIndex(i) => {
                            find_in_expr(i, off, chain);
                        }
                        crate::ast::ChainStep::Call(args)
                        | crate::ast::ChainStep::OptCall(args) => {
                            for a in args {
                                find_in_expr(a, off, chain);
                            }
                        }
                        crate::ast::ChainStep::Member(_) | crate::ast::ChainStep::OptMember(_) => {}
                    }
                }
            }
            Expr::Yield { arg, .. } => {
                if let Some(a) = arg {
                    find_in_expr(a, off, chain);
                }
            }
        },
    );
}

fn find_in_pat(p: &Pat, off: usize, chain: &mut Vec<Rc<FnLit>>) {
    match p {
        Pat::Ident(_) => {}
        Pat::Lvalue(e) => find_in_expr(e, off, chain),
        Pat::Rest(p) => find_in_pat(p, off, chain),
        Pat::Default(p, e) => {
            find_in_pat(p, off, chain);
            find_in_expr(e, off, chain);
        }
        Pat::Array(items) => {
            for i in items.iter().flatten() {
                find_in_pat(i, off, chain);
            }
        }
        Pat::Object { props, rest } => {
            for (k, p) in props {
                find_in_prop_key(k, off, chain);
                find_in_pat(p, off, chain);
            }
            if let Some(p) = rest {
                find_in_pat(p, off, chain);
            }
        }
    }
}

fn find_in_param(p: &Param, off: usize, chain: &mut Vec<Rc<FnLit>>) {
    find_in_pat(&p.pat, off, chain);
}

fn find_in_prop_key(k: &PropKey, off: usize, chain: &mut Vec<Rc<FnLit>>) {
    if let PropKey::Computed(e) = k {
        find_in_expr(e, off, chain);
    }
}

fn find_in_var_decl(d: &VarDecl, off: usize, chain: &mut Vec<Rc<FnLit>>) {
    find_in_pat(&d.pat, off, chain);
    if let Some(i) = &d.init {
        find_in_expr(i, off, chain);
    }
}

fn find_in_for_init(i: &ForInit, off: usize, chain: &mut Vec<Rc<FnLit>>) {
    match i {
        ForInit::Var(ds) | ForInit::Lexical { decls: ds, .. } => {
            for d in ds {
                find_in_var_decl(d, off, chain);
            }
        }
        ForInit::Expr(e) => find_in_expr(e, off, chain),
    }
}

fn find_in_for_target(t: &ForTarget, off: usize, chain: &mut Vec<Rc<FnLit>>) {
    match t {
        ForTarget::Var(p) | ForTarget::Lexical { pat: p, .. } => find_in_pat(p, off, chain),
        ForTarget::Expr(e) => find_in_expr(e, off, chain),
        ForTarget::Using { pat, .. } => find_in_pat(pat, off, chain),
    }
}

fn find_in_class(c: &ClassLit, off: usize, chain: &mut Vec<Rc<FnLit>>) {
    if let Some(s) = &c.super_ {
        find_in_expr(s, off, chain);
    }
    for m in &c.members {
        find_in_prop_key(&m.key, off, chain);
        descend_fn(&m.fun, off, chain);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRC: &str = "var outer = function() { var x = 1; \
                       var inner = function named_inner() { return x + window.__ready; };\
                       return inner(); };\ntop_level_fn();";

    /// `(ligne, colonne)` 1-based du premier `needle` dans
    /// `src` — même convention que les spans lexer.
    fn lc_of(src: &str, needle: &str) -> (u32, u32) {
        let off = src.find(needle).expect("needle présent");
        let (mut l, mut c) = (1usize, 1usize);
        for (i, ch) in src.chars().enumerate() {
            if i == off {
                break;
            }
            if ch == '\n' {
                l += 1;
                c = 1;
            } else {
                c += 1;
            }
        }
        (l as u32, c as u32)
    }

    /// JS-C413 — l'offset milieu de `named_inner` retourne la
    /// chaîne `[outer, named_inner]` : l'englobante anonyme +
    /// la plus profonde nommée, extrait verbatim borné au span.
    #[test]
    fn fnat_finds_enclosing_function() {
        // Offset dans le corps de `named_inner` (sur `window`).
        let (line, col) = lc_of(SRC, "window.__ready");
        let hits = fn_chain_at(SRC, line, col);
        assert_eq!(hits.len(), 2, "chaîne attendue: {hits:?}");
        assert_eq!(hits[0].name.as_deref(), None);
        assert_eq!(hits[1].name.as_deref(), Some("named_inner"));
        assert!(hits[1].excerpt.contains("named_inner"));
        assert!(hits[1].excerpt.contains("__ready"));
    }

    /// Offset en top-level (hors fonction) → chaîne vide.
    #[test]
    fn fnat_top_level_is_empty() {
        let (line, col) = lc_of(SRC, "top_level_fn");
        assert!(fn_chain_at(SRC, line, col).is_empty());
    }

    /// Corps différé : une fonction assez longue pour passer
    /// sous `FnBody::Deferred` doit être matérialisée et
    /// trouvée quand même (le mur réel : corps de bundle).
    #[test]
    fn fnat_materializes_deferred_bodies() {
        let filler = "var a0=0;".repeat(8); // ≥24 tokens → différé
        let src = format!(
            "var outer = function big() {{ {filler} var inner = function deep() {{ return window.__gate; }}; return inner(); }};"
        );
        let (line, col) = lc_of(&src, "__gate");
        let hits = fn_chain_at(&src, line, col);
        assert_eq!(hits.len(), 2, "chaîne attendue: {hits:?}");
        assert_eq!(hits[1].name.as_deref(), Some("deep"));
    }
}
