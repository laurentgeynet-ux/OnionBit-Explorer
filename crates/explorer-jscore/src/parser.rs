// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Recursive-descent parser for the `explorer-jscore` subset.
//!
//! Implements a pragmatic ASI: a statement may end without `;` if the
//! next token is `}`, EOF, or was preceded by a line break; `return`
//! with a line break returns `undefined`; postfix `++`/`--` require
//! same-line placement. `in` expressions are suppressed inside `for`
//! init clauses (the `no_in` flag), like the spec's `NoIn` grammar.

use crate::ast::{
    AssignOp, BinOp, ChainStep, ClassLit, ClassMember, ClassMemberKind, ExportDecl, Expr, FnLit,
    ForInit, ForTarget, ImportBind, ObjProp, Param, Pat, Program, PropKey, Stmt, SwitchCase, UnOp,
    VarDecl,
};
use crate::error::{Error, Result};
use crate::lexer::{lex, Kw, Tok, Token, TplPart, P};
use std::collections::HashSet;
use std::rc::Rc;

/// Parse avec strict hérité — `eval` direct dans du code strict :
/// ses early errors s'appliquent aussi au texte évalué (spec
/// `PerformEval` — « strictCaller »). `nest_max` borne
/// l'emboîtement natif du parseur récursif (JS-C41).
pub(crate) fn parse_in(src: &str, strict: bool, nest_max: u32) -> Result<Program> {
    let toks = lex(src)?;
    let mut p = Parser {
        toks,
        pos: 0,
        nest: 0,
        nest_max,
        strict,
        module: false,
        in_gen: false,
        labels: Vec::new(),
        priv_seq: 0,
        priv_scopes: Vec::new(),
    };
    p.scan_directives();
    let mut body = Vec::new();
    while !p.at_eof() {
        body.push(p.stmt()?);
    }
    Ok(Program {
        body,
        strict: p.strict,
    })
}

/// Parse a whole module — module goal: `import`/`export` statements
/// are allowed and the code is always strict (spec : un module ne
/// quitte jamais le mode strict).
pub fn parse_module(src: &str) -> Result<Program> {
    parse_module_in(src, DEFAULT_NEST_LIMIT)
}

/// Module goal avec borne d'emboîtement explicite (JS-C41).
pub(crate) fn parse_module_in(src: &str, nest_max: u32) -> Result<Program> {
    let toks = lex(src)?;
    let mut p = Parser {
        toks,
        pos: 0,
        nest: 0,
        nest_max,
        strict: true,
        module: true,
        in_gen: false,
        labels: Vec::new(),
        priv_seq: 0,
        priv_scopes: Vec::new(),
    };
    p.scan_directives();
    let mut body = Vec::new();
    while !p.at_eof() {
        body.push(p.stmt()?);
    }
    Ok(Program {
        body,
        strict: p.strict,
    })
}

/// Borne logique d'emboîtement par défaut du parse (JS-C41) —
/// chaque transporteur récursif l'incrémente (~5 coups par niveau
/// de parenthèse → ~100 niveaux logiques). La protection anti-crash
/// est `stacker::maybe_grow` aux points de récursion, qui alloue un
/// segment de pile sur tas quand la pile native s'épuise — la borne
/// numérique ne coupe que les entrées absurdes. L'[`Engine`] la
/// relève via [`crate::interp::Engine::set_nest_limit`].
pub(crate) const DEFAULT_NEST_LIMIT: u32 = 512;

/// Marge déclenchant la croissance de pile (`maybe_grow`) — en
/// dessous, la récursion bascule sur un segment tas plutôt que de
/// risquer le débordement natif (le coût par niveau varie de ~3 à
/// ~55 Kio selon le transporteur et le profil, JS-C41).
pub(crate) const STACK_RED_ZONE: usize = 256 * 1024;

/// Taille d'un segment de pile alloué sur tas par `maybe_grow`.
pub(crate) const STACK_GROW: usize = 2 * 1024 * 1024;

struct Parser {
    toks: Vec<Token>,
    pos: usize,
    /// Expression nesting guard — `((((…` input must not blow the
    /// native stack through recursive descent.
    nest: u32,
    /// Borne de `nest` — `DEFAULT_NEST_LIMIT` par défaut,
    /// relevable par l'`Engine` (`Engine::set_nest_limit`).
    nest_max: u32,
    /// `true` dans une unité `"use strict"` — script ou corps de
    /// fonction (hérité par les corps internes). Pilote les early
    /// errors strict : octales legacy, `delete x`, liaisons
    /// réservées.
    strict: bool,
    /// `true` dans le but de grammaire *module* — autorise les
    /// statements `import`/`export` au niveau top.
    module: bool,
    /// `true` dans un corps de fonction générateur — autorise
    /// l'expression `yield` (contextuelle : identifiant ailleurs,
    /// réservée en strict).
    in_gen: bool,
    /// Pile des labels actifs `(nom, est_boucle)` — vidée à
    /// l'entrée de chaque fonction (les labels ne traversent pas
    /// les fonctions, spec `LabelledItem` scoping).
    labels: Vec<(String, bool)>,
    /// Séquence d'identité des classes — chaque corps de classe
    /// reçoit un numéro ; ses noms privés sont résolus au parse
    /// vers des clés sentinelle `\u{f131}privN.name` (invisibles à
    /// l'énumération et distinctes d'une classe à l'autre).
    priv_seq: u32,
    /// Portées de noms privés `(identité, déclarations)` — une
    /// entrée par classe imbriquée ; la résolution `#x` cherche
    /// le nom de la plus interne à la plus externe (spec : les
    /// `PrivateNameEnvironments` s'emboîtent).
    priv_scopes: Vec<(u32, HashSet<String>)>,
}

impl Parser {
    // -- token plumbing ------------------------------------------------

    fn peek(&self) -> &Tok {
        &self.toks[self.pos].tok
    }

    fn cur(&self) -> &Token {
        &self.toks[self.pos]
    }

    /// `(ligne, colonne)` 1-based du jeton courant — ouverture
    /// d'un `FnLit` (JS-C47, rendue dans `error.stack`).
    fn pos_here(&self) -> (usize, usize) {
        (self.cur().line, self.cur().col)
    }

    fn next(&mut self) -> &Token {
        let t = &self.toks[self.pos];
        if self.pos + 1 < self.toks.len() {
            self.pos += 1;
        }
        t
    }

    fn at_eof(&self) -> bool {
        matches!(self.peek(), Tok::Eof)
    }

    /// Incrémente la borne d'emboîtement — appelé en tête de
    /// chaque transporteur récursif (JS-C41). La pile elle-même
    /// est protégée par `stacker::maybe_grow` autour de l'appel
    /// récursif — ici, borne logique seule.
    fn nest_in(&mut self) -> Result<()> {
        if self.nest >= self.nest_max {
            return Err(self.err("input too deeply nested"));
        }
        self.nest += 1;
        Ok(())
    }

    /// Décrit la borne d'emboîtement — symétrique de
    /// [`Self::nest_in`], appelé à chaque sortie.
    fn nest_out(&mut self) {
        self.nest -= 1;
    }

    fn err(&self, message: impl Into<String>) -> Error {
        let t = self.cur();
        Error::Syntax {
            line: t.line,
            col: t.col,
            message: message.into(),
        }
    }

    fn eat_p(&mut self, p: P) -> bool {
        if self.peek() == &Tok::P(p) {
            self.next();
            true
        } else {
            false
        }
    }

    fn check_p(&self, p: P) -> bool {
        self.peek() == &Tok::P(p)
    }

    fn expect_p(&mut self, p: P, what: &str) -> Result<()> {
        if self.eat_p(p) {
            Ok(())
        } else {
            Err(self.err(format!("expected {what}")))
        }
    }

    fn eat_kw(&mut self, kw: Kw) -> bool {
        if self.peek() == &Tok::Kw(kw) {
            self.next();
            true
        } else {
            false
        }
    }

    fn expect_kw(&mut self, kw: Kw, what: &str) -> Result<()> {
        if self.eat_kw(kw) {
            Ok(())
        } else {
            Err(self.err(format!("expected {what}")))
        }
    }

    /// Property name after `.` — keywords are legal member names in
    /// real engines (`el.class`, `o.function`, `a.return`…).
    fn expect_prop_name(&mut self) -> Result<String> {
        match self.peek().clone() {
            Tok::Ident(s) => {
                self.next();
                Ok(s)
            }
            Tok::Kw(kw) => {
                self.next();
                Ok(kw_str(kw).to_string())
            }
            // `o.#x` — accès membre privé, résolu à la portée
            // de classe englobante.
            Tok::Private(n) => {
                self.next();
                self.resolve_priv(&n)
            }
            _ => Err(self.err("expected property name")),
        }
    }

    /// Clé sentinelle d'un nom privé — `\u{f131}privN.name`.
    /// Le préfixe d'espace privé Unicode garantit qu'aucune clé
    /// publique ne peut entrer en collision.
    fn priv_key(seq: u32, name: &str) -> String {
        format!("\u{f131}priv{seq}.{name}")
    }

    /// Résout `#name` vers sa clé sentinelle en remontant les
    /// portées de classe ouvertes — une classe imbriquée voit
    /// les privés des englobantes ; hors classe → `SyntaxError`
    /// (early error spec).
    fn resolve_priv(&self, name: &str) -> Result<String> {
        for (seq, names) in self.priv_scopes.iter().rev() {
            if names.contains(name) {
                return Ok(Self::priv_key(*seq, name));
            }
        }
        Err(self.err(format!("private name `#{name}` is not defined")))
    }

    /// Déclarations `#…` du corps de classe qui commence juste
    /// après `self.pos` (le `{` est consommé). Pré-scan à profondeur
    /// 1 — les corps de méthode sont ignorés mais ne peuvent pas
    /// déclarer de noms privés ; les `#x` qui y figurent sont des
    /// *usages* de l'englobante.
    fn prescan_privates(&self) -> HashSet<String> {
        let mut out = HashSet::new();
        let mut depth = 1u32;
        for t in &self.toks[self.pos..] {
            match &t.tok {
                Tok::P(P::LBrace) => depth += 1,
                Tok::P(P::RBrace) => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                Tok::Private(n) if depth == 1 => {
                    out.insert(n.clone());
                }
                _ => {}
            }
        }
        out
    }

    /// ASI: `;`, or `}` / `else` / EOF / line break act as terminators.
    /// (`else` can never start an expression, so it always ends the
    /// then-statement — `if (x) a else b` parses in real engines.)
    fn expect_semi(&mut self) -> Result<()> {
        if self.eat_p(P::Semi)
            || self.at_eof()
            || self.check_p(P::RBrace)
            || self.peek() == &Tok::Kw(Kw::Else)
            || self.cur().newline_before
        {
            Ok(())
        } else {
            Err(self.err("expected `;`"))
        }
    }

    // -- statements ----------------------------------------------------

    /// Label optionnel après `break`/`continue` — ASI : le nom doit
    /// être sur la même ligne.
    fn label_arg(&mut self) -> Result<Option<String>> {
        if self.cur().newline_before {
            return Ok(None);
        }
        match self.peek().clone() {
            Tok::Ident(n) => {
                self.next();
                Ok(Some(n))
            }
            _ => Ok(None),
        }
    }

    /// `ident : stmt` — statement étiqueté. `est_boucle` se
    /// détermine en regardant au-delà des labels chaînés
    /// (`a: b: for` — `continue a` vise bien la boucle, spec).
    fn labeled_stmt(&mut self) -> Result<Stmt> {
        let Tok::Ident(name) = self.peek().clone() else {
            return Err(self.err("expected label name"));
        };
        let mut i = self.pos + 2;
        let is_loop = loop {
            match self.toks.get(i).map(|t| &t.tok) {
                Some(Tok::Kw(Kw::For | Kw::While | Kw::Do)) => break true,
                Some(Tok::Ident(_))
                    if self.toks.get(i + 1).map(|t| &t.tok) == Some(&Tok::P(P::Colon)) =>
                {
                    i += 2;
                }
                _ => break false,
            }
        };
        self.next(); // `ident`
        self.next(); // `:`
        if self.labels.iter().any(|(l, _)| l == &name) {
            return Err(self.err(format!("duplicate label `{name}`")));
        }
        self.labels.push((name.clone(), is_loop));
        let body = self.stmt()?;
        self.labels.pop();
        Ok(Stmt::Labeled {
            name,
            body: Box::new(body),
        })
    }

    fn stmt(&mut self) -> Result<Stmt> {
        self.nest_in()?;
        let r = stacker::maybe_grow(STACK_RED_ZONE, STACK_GROW, || self.stmt_inner());
        self.nest_out();
        r
    }

    fn stmt_inner(&mut self) -> Result<Stmt> {
        match self.peek().clone() {
            Tok::P(P::Semi) => {
                self.next();
                Ok(Stmt::Empty)
            }
            Tok::P(P::LBrace) => {
                self.next();
                Ok(Stmt::Block(self.stmt_list()?))
            }
            Tok::Kw(Kw::Var) => {
                self.next();
                let decls = self.var_decls(false)?;
                self.expect_semi()?;
                Ok(Stmt::Var(decls))
            }
            Tok::Kw(Kw::Const) => {
                self.next();
                let decls = self.var_decls(false)?;
                self.expect_semi()?;
                Ok(Stmt::Lexical {
                    decls,
                    constant: true,
                })
            }
            Tok::Kw(Kw::Let) => {
                // `let` is contextual: a decl only before ident/`[`/`{`
                // (destructuring) — otherwise it's a sloppy identifier.
                let decl = matches!(
                    self.toks.get(self.pos + 1).map(|t| &t.tok),
                    Some(Tok::Ident(_) | Tok::P(P::LBracket | P::LBrace))
                );
                if !decl {
                    let e = self.expr(false)?;
                    self.expect_semi()?;
                    return Ok(Stmt::Expr(e));
                }
                self.next();
                let decls = self.var_decls(false)?;
                self.expect_semi()?;
                Ok(Stmt::Lexical {
                    decls,
                    constant: false,
                })
            }
            Tok::Kw(Kw::Function) => {
                let f = self.function(true, false)?;
                Ok(Stmt::FunctionDecl(f))
            }
            // `async function f(){}` — ASI guard: a newline between
            // `async` and `function` keeps `async` an identifier.
            Tok::Ident(s)
                if s == "async"
                    && matches!(
                        self.toks.get(self.pos + 1),
                        Some(t) if t.tok == Tok::Kw(Kw::Function) && !t.newline_before
                    ) =>
            {
                self.next(); // async
                let f = self.function(true, true)?;
                Ok(Stmt::FunctionDecl(f))
            }
            Tok::Kw(Kw::Class) => Ok(Stmt::ClassDecl(self.class_lit()?)),
            // `using x = e` (ES2026) — contextuel comme `let` : une
            // décl seulement devant ident/`[`/`{`.
            Tok::Ident(s)
                if s == "using"
                    && matches!(
                        self.toks.get(self.pos + 1).map(|t| &t.tok),
                        Some(Tok::Ident(_) | Tok::P(P::LBracket | P::LBrace))
                    ) =>
            {
                self.next(); // `using`
                let decls = self.var_decls(false)?;
                self.expect_semi()?;
                Ok(Stmt::Using {
                    decls,
                    is_await: false,
                })
            }
            // `await using x = e` — `@@asyncDispose`.
            Tok::Ident(s)
                if s == "await"
                    && matches!(
                        self.toks.get(self.pos + 1).map(|t| &t.tok),
                        Some(Tok::Ident(u)) if u == "using"
                    )
                    && matches!(
                        self.toks.get(self.pos + 2).map(|t| &t.tok),
                        Some(Tok::Ident(_) | Tok::P(P::LBracket | P::LBrace))
                    ) =>
            {
                self.next(); // `await`
                self.next(); // `using`
                let decls = self.var_decls(false)?;
                self.expect_semi()?;
                Ok(Stmt::Using {
                    decls,
                    is_await: true,
                })
            }
            Tok::Kw(Kw::If) => self.if_stmt(),
            Tok::Kw(Kw::With) => {
                // `with` est interdit en strict — early error spec.
                if self.strict {
                    return Err(self.err("`with` is not allowed in strict mode"));
                }
                self.next();
                self.expect_p(P::LParen, "`(`")?;
                let obj = self.expr(false)?;
                self.expect_p(P::RParen, "`)`")?;
                let body = Box::new(self.stmt()?);
                Ok(Stmt::With { obj, body })
            }
            Tok::Kw(Kw::While) => {
                self.next();
                self.expect_p(P::LParen, "`(`")?;
                let cond = self.expr(false)?;
                self.expect_p(P::RParen, "`)`")?;
                let body = Box::new(self.stmt()?);
                Ok(Stmt::While { cond, body })
            }
            Tok::Kw(Kw::Do) => {
                self.next();
                let body = Box::new(self.stmt()?);
                self.expect_kw(Kw::While, "`while`")?;
                self.expect_p(P::LParen, "`(`")?;
                let cond = self.expr(false)?;
                self.expect_p(P::RParen, "`)`")?;
                self.eat_p(P::Semi);
                Ok(Stmt::DoWhile { body, cond })
            }
            Tok::Kw(Kw::For) => self.for_stmt(),
            Tok::Kw(Kw::Return) => {
                self.next();
                let arg = if self.at_eof()
                    || self.check_p(P::Semi)
                    || self.check_p(P::RBrace)
                    || self.cur().newline_before
                {
                    None
                } else {
                    Some(self.expr(false)?)
                };
                self.expect_semi()?;
                Ok(Stmt::Return(arg))
            }
            Tok::Kw(Kw::Throw) => {
                self.next();
                if self.cur().newline_before {
                    return Err(self.err("line break not allowed after `throw`"));
                }
                let e = self.expr(false)?;
                self.expect_semi()?;
                Ok(Stmt::Throw(e))
            }
            Tok::Kw(Kw::Break) => {
                self.next();
                let label = self.label_arg()?;
                if let Some(n) = &label {
                    if !self.labels.iter().any(|(l, _)| l == n) {
                        return Err(self.err(format!("undefined label `{n}`")));
                    }
                }
                self.expect_semi()?;
                Ok(Stmt::Break(label))
            }
            Tok::Kw(Kw::Continue) => {
                self.next();
                let label = self.label_arg()?;
                if let Some(n) = &label {
                    // `continue l` exige un label de boucle
                    // (early error spec).
                    match self.labels.iter().rev().find(|(l, _)| l == n) {
                        Some((_, true)) => {}
                        _ => {
                            return Err(self.err(format!("`continue` target `{n}` is not a loop")));
                        }
                    }
                }
                self.expect_semi()?;
                Ok(Stmt::Continue(label))
            }
            Tok::Kw(Kw::Try) => self.try_stmt(),
            Tok::Kw(Kw::Switch) => self.switch_stmt(),
            // `import …` : statement de module — sauf `import(` /
            // `import.meta` qui sont des expressions (dynamic
            // import est légal aussi dans un script classique).
            Tok::Kw(Kw::Import)
                if self.module
                    && !matches!(
                        self.toks.get(self.pos + 1).map(|t| &t.tok),
                        Some(Tok::P(P::LParen | P::Dot))
                    ) =>
            {
                self.import_stmt()
            }
            Tok::Kw(Kw::Export) if self.module => self.export_stmt(),
            Tok::Kw(Kw::Export) => Err(self.err("`export` outside a module")),
            Tok::Kw(Kw::Unsupported(k)) => Err(self.err(format!("`{k}` is not supported"))),
            // `ident :` — statement étiqueté (`lbl: for …`).
            Tok::Ident(_)
                if self.toks.get(self.pos + 1).map(|t| &t.tok) == Some(&Tok::P(P::Colon)) =>
            {
                self.labeled_stmt()
            }
            // `import` en position statement hors module retombe
            // ici — seuls `import(`/`import.meta` sont des
            // expressions (dispatchées plus haut).
            _ => {
                let e = self.expr(false)?;
                self.expect_semi()?;
                Ok(Stmt::Expr(e))
            }
        }
    }

    /// `import … from 'mod'` / `import 'mod'` — curseur sur
    /// `import`, module goal garanti par le dispatch de `stmt`.
    fn import_stmt(&mut self) -> Result<Stmt> {
        self.next(); // `import`
                     // `import 'mod';` — side-effect only, no bindings.
        if let Tok::Str(src) = self.peek().clone() {
            self.next();
            self.expect_semi()?;
            return Ok(Stmt::Import {
                binds: Vec::new(),
                source: src,
            });
        }
        let mut binds = Vec::new();
        match self.peek().clone() {
            Tok::P(P::Star) => {
                self.next();
                self.expect_ident("as")?;
                binds.push(ImportBind::Namespace(self.module_export_name()?));
            }
            Tok::P(P::LBrace) => self.import_specifiers(&mut binds)?,
            // `import def` — default binding, optionally followed by
            // `, {…}` or `, * as ns`.
            Tok::Ident(_) | Tok::Kw(_) => {
                binds.push(ImportBind::Default(self.module_export_name()?));
                if self.eat_p(P::Comma) {
                    match self.peek().clone() {
                        Tok::P(P::Star) => {
                            self.next();
                            self.expect_ident("as")?;
                            binds.push(ImportBind::Namespace(self.module_export_name()?));
                        }
                        Tok::P(P::LBrace) => self.import_specifiers(&mut binds)?,
                        _ => return Err(self.err("expected `*` or `{` after `,`")),
                    }
                }
            }
            _ => return Err(self.err("expected import clause")),
        }
        self.expect_ident("from")?;
        let source = self.expect_str_tok()?;
        self.expect_semi()?;
        Ok(Stmt::Import { binds, source })
    }

    /// `{a, b as c}` — named import specifiers.
    fn import_specifiers(&mut self, binds: &mut Vec<ImportBind>) -> Result<()> {
        self.expect_p(P::LBrace, "`{`")?;
        loop {
            if self.eat_p(P::RBrace) {
                return Ok(());
            }
            let imported = self.module_export_name()?;
            let local = if self.eat_ident("as") {
                self.module_export_name()?
            } else {
                imported.clone()
            };
            binds.push(ImportBind::Named(imported, local));
            if self.eat_p(P::Comma) {
                continue;
            }
            self.expect_p(P::RBrace, "`}`")?;
            return Ok(());
        }
    }

    /// `export …` — cursor on `export`, module goal.
    fn export_stmt(&mut self) -> Result<Stmt> {
        self.next(); // `export`
        match self.peek().clone() {
            // `export * [as ns] from 'm'`
            Tok::P(P::Star) => {
                self.next();
                let ns = if self.eat_ident("as") {
                    Some(self.module_export_name()?)
                } else {
                    None
                };
                self.expect_ident("from")?;
                let from = self.expect_str_tok()?;
                self.expect_semi()?;
                Ok(Stmt::Export(ExportDecl::Star { ns, from }))
            }
            // `export {a as b}` [`from 'm'`]
            Tok::P(P::LBrace) => {
                let list = self.export_specifiers()?;
                let from = if self.eat_ident("from") {
                    Some(self.expect_str_tok()?)
                } else {
                    None
                };
                self.expect_semi()?;
                Ok(Stmt::Export(ExportDecl::Named { list, from }))
            }
            // `export default <expr|function|class>`
            Tok::Kw(Kw::Default) => {
                self.next();
                let e = match self.peek().clone() {
                    Tok::Kw(Kw::Function) => Expr::Function(self.function(false, false)?),
                    // `export default async function`
                    Tok::Ident(s)
                        if s == "async"
                            && matches!(
                                self.toks.get(self.pos + 1),
                                Some(t) if t.tok == Tok::Kw(Kw::Function) && !t.newline_before
                            ) =>
                    {
                        self.next();
                        Expr::Function(self.function(false, true)?)
                    }
                    Tok::Kw(Kw::Class) => Expr::Class(self.class_lit()?),
                    _ => self.assign(false)?,
                };
                self.expect_semi()?;
                Ok(Stmt::Export(ExportDecl::Default(Box::new(e))))
            }
            Tok::Kw(Kw::Var) => {
                self.next();
                let decls = self.var_decls(false)?;
                self.expect_semi()?;
                Ok(Stmt::Export(ExportDecl::Decl(Box::new(Stmt::Var(decls)))))
            }
            Tok::Kw(Kw::Const) => {
                self.next();
                let decls = self.var_decls(false)?;
                self.expect_semi()?;
                Ok(Stmt::Export(ExportDecl::Decl(Box::new(Stmt::Lexical {
                    decls,
                    constant: true,
                }))))
            }
            Tok::Kw(Kw::Let) => {
                self.next();
                let decls = self.var_decls(false)?;
                self.expect_semi()?;
                Ok(Stmt::Export(ExportDecl::Decl(Box::new(Stmt::Lexical {
                    decls,
                    constant: false,
                }))))
            }
            Tok::Kw(Kw::Function) => {
                let f = self.function(true, false)?;
                Ok(Stmt::Export(ExportDecl::Decl(Box::new(
                    Stmt::FunctionDecl(f),
                ))))
            }
            Tok::Ident(s)
                if s == "async"
                    && matches!(
                        self.toks.get(self.pos + 1),
                        Some(t) if t.tok == Tok::Kw(Kw::Function) && !t.newline_before
                    ) =>
            {
                self.next();
                let f = self.function(true, true)?;
                Ok(Stmt::Export(ExportDecl::Decl(Box::new(
                    Stmt::FunctionDecl(f),
                ))))
            }
            Tok::Kw(Kw::Class) => {
                let c = self.class_lit()?;
                Ok(Stmt::Export(ExportDecl::Decl(Box::new(Stmt::ClassDecl(c)))))
            }
            _ => Err(self.err("expected export declaration")),
        }
    }

    /// `{a as b, c}` — `export` specifier list, returns
    /// `(local, exported)` pairs.
    fn export_specifiers(&mut self) -> Result<Vec<(String, String)>> {
        self.expect_p(P::LBrace, "`{`")?;
        let mut list = Vec::new();
        loop {
            if self.eat_p(P::RBrace) {
                return Ok(list);
            }
            let local = self.module_export_name()?;
            let exported = if self.eat_ident("as") {
                self.module_export_name()?
            } else {
                local.clone()
            };
            list.push((local, exported));
            if self.eat_p(P::Comma) {
                continue;
            }
            self.expect_p(P::RBrace, "`}`")?;
            return Ok(list);
        }
    }

    /// `IdentifierName` or string literal — the names allowed on
    /// each side of `as` (`import {a}`, `export {a as b}`, string
    /// export names ES2022).
    fn module_export_name(&mut self) -> Result<String> {
        match self.peek().clone() {
            Tok::Ident(s) | Tok::Str(s) => {
                self.next();
                Ok(s)
            }
            Tok::Kw(kw) => {
                self.next();
                Ok(kw_str(kw).to_string())
            }
            _ => Err(self.err("expected export name")),
        }
    }

    /// Literal string argument (`'mod'` specifier).
    fn expect_str_tok(&mut self) -> Result<String> {
        match self.peek().clone() {
            Tok::Str(s) => {
                self.next();
                Ok(s)
            }
            _ => Err(self.err("expected module specifier string")),
        }
    }

    fn eat_ident(&mut self, word: &str) -> bool {
        if matches!(self.peek(), Tok::Ident(s) if s == word) {
            self.next();
            true
        } else {
            false
        }
    }

    fn expect_ident(&mut self, word: &str) -> Result<()> {
        if self.eat_ident(word) {
            Ok(())
        } else {
            Err(self.err(format!("expected `{word}`")))
        }
    }

    fn stmt_list(&mut self) -> Result<Vec<Stmt>> {
        let mut out = Vec::new();
        while !self.at_eof() && !self.check_p(P::RBrace) {
            out.push(self.stmt()?);
        }
        self.expect_p(P::RBrace, "`}`")?;
        Ok(out)
    }

    /// Directive prologue scan — lecture seule sur les tokens à
    /// venir (les directives restent parsées comme statements).
    /// `"use strict"` en tête de script/corps de fonction active le
    /// mode strict pour toute l'unité. Une directive = littéral de
    /// chaîne terminant un statement (`;`, `}`, EOF ou ASI).
    fn scan_directives(&mut self) {
        let mut i = self.pos;
        while let Some(t) = self.toks.get(i) {
            let Tok::Str(s) = &t.tok else { break };
            let ends = match self.toks.get(i + 1) {
                None => true,
                Some(n) => {
                    n.newline_before || matches!(n.tok, Tok::Eof | Tok::P(P::Semi | P::RBrace))
                }
            };
            if !ends {
                break;
            }
            if s == "use strict" {
                self.strict = true;
            }
            i += 1;
            if matches!(self.toks.get(i).map(|t| &t.tok), Some(Tok::P(P::Semi))) {
                i += 1;
            }
        }
    }

    /// Early errors strict sur un nom de liaison (`var`/`let`/
    /// `const`, param, catch, nom de fonction).
    fn check_strict_binding(&self, name: &str) -> Result<()> {
        if self.strict
            && matches!(
                name,
                "eval"
                    | "arguments"
                    | "implements"
                    | "interface"
                    | "let"
                    | "package"
                    | "private"
                    | "protected"
                    | "public"
                    | "static"
                    | "yield"
            )
        {
            return Err(self.err(format!("`{name}` is reserved in strict mode")));
        }
        Ok(())
    }

    fn if_stmt(&mut self) -> Result<Stmt> {
        self.expect_kw(Kw::If, "`if`")?;
        self.expect_p(P::LParen, "`(`")?;
        let cond = self.expr(false)?;
        self.expect_p(P::RParen, "`)`")?;
        let then = Box::new(self.stmt()?);
        let else_ = if self.eat_kw(Kw::Else) {
            Some(Box::new(self.stmt()?))
        } else {
            None
        };
        Ok(Stmt::If { cond, then, else_ })
    }

    fn var_decls(&mut self, no_in: bool) -> Result<Vec<VarDecl>> {
        let mut out = Vec::new();
        loop {
            let pat = self.pattern_decl()?;
            let init = if self.eat_p(P::Assign) {
                Some(self.assign(no_in)?)
            } else {
                None
            };
            out.push(VarDecl { pat, init });
            if !self.eat_p(P::Comma) {
                return Ok(out);
            }
        }
    }

    /// Binding pattern — `name`, `[a, , ...r]` or `{k: p, ...rest}`,
    /// each element optionally `= default`-suffixed.
    fn pattern(&mut self) -> Result<Pat> {
        self.pattern_inner(true)
    }

    /// Declarator-name pattern — the trailing `=` starts the
    /// *initializer* (`var a = 1`), not a default. Only nested
    /// sub-patterns take defaults.
    fn pattern_decl(&mut self) -> Result<Pat> {
        self.pattern_inner(false)
    }

    fn pattern_inner(&mut self, allow_default: bool) -> Result<Pat> {
        self.nest_in()?;
        let r = stacker::maybe_grow(STACK_RED_ZONE, STACK_GROW, || {
            self.pattern_nested(allow_default)
        });
        self.nest_out();
        r
    }

    fn pattern_nested(&mut self, allow_default: bool) -> Result<Pat> {
        let mut pat = match self.peek().clone() {
            Tok::P(P::Ellipsis) => {
                self.next();
                Pat::Rest(Box::new(self.pattern()?))
            }
            Tok::P(P::LBracket) => {
                self.next();
                let mut elems = Vec::new();
                loop {
                    if self.eat_p(P::RBracket) {
                        break;
                    }
                    if self.eat_p(P::Comma) {
                        elems.push(None); // hole
                        continue;
                    }
                    elems.push(Some(self.pattern()?));
                    if self.eat_p(P::Comma) {
                        continue;
                    }
                    self.expect_p(P::RBracket, "`]`")?;
                    break;
                }
                Pat::Array(elems)
            }
            Tok::P(P::LBrace) => {
                self.next();
                let mut props = Vec::new();
                let mut rest = None;
                loop {
                    if self.eat_p(P::RBrace) {
                        break;
                    }
                    if self.eat_p(P::Ellipsis) {
                        rest = Some(Box::new(self.pattern()?));
                        self.expect_p(P::RBrace, "`}`")?;
                        break;
                    }
                    let key = self.prop_key()?;
                    // `{a}` → `a: a`; `{a: p}` / `{a = d}` / `{a: p = d}`.
                    let mut sub = if self.eat_p(P::Colon) {
                        self.pattern()?
                    } else {
                        let name = match &key {
                            PropKey::Name(n) => n.clone(),
                            PropKey::Computed(_) => {
                                return Err(self.err("computed key needs `:` in pattern"))
                            }
                        };
                        Pat::Ident(name)
                    };
                    // `{a = d}` shorthand default.
                    if self.eat_p(P::Assign) {
                        sub = Pat::Default(Box::new(sub), Box::new(self.assign(false)?));
                    }
                    props.push((key, sub));
                    if self.eat_p(P::Comma) {
                        continue;
                    }
                    self.expect_p(P::RBrace, "`}`")?;
                    break;
                }
                Pat::Object { props, rest }
            }
            Tok::Ident(s) => {
                self.check_strict_binding(&s)?;
                self.next();
                Pat::Ident(s)
            }
            // Sloppy tolerance: `let` is a binding name.
            Tok::Kw(Kw::Let) => {
                self.check_strict_binding("let")?;
                self.next();
                Pat::Ident("let".into())
            }
            _ => return Err(self.err("expected binding name or pattern")),
        };
        // `= default` — params/nested/catch only (tolerant: the spec
        // rejects it in `for-in`/`for-of` targets; we allow it like
        // decls). Decl sites use `pattern_decl` — their `=` is the
        // initializer.
        if allow_default && self.eat_p(P::Assign) {
            let d = self.assign(false)?;
            pat = Pat::Default(Box::new(pat), Box::new(d));
        }
        Ok(pat)
    }

    /// Property key — `name`, `"s"`, `42`, keyword or `[computed]`.
    fn prop_key(&mut self) -> Result<PropKey> {
        if self.eat_p(P::LBracket) {
            let e = self.expr(false)?;
            self.expect_p(P::RBracket, "`]`")?;
            return Ok(PropKey::Computed(e));
        }
        let name = match self.peek().clone() {
            Tok::Ident(s) | Tok::Str(s) => {
                self.next();
                s
            }
            Tok::Num(n) => {
                self.next();
                crate::value::num_to_js_string(n)
            }
            Tok::Kw(kw) => {
                self.next();
                kw_str(kw).to_string()
            }
            Tok::Private(n) => {
                self.next();
                self.resolve_priv(&n)?
            }
            _ => return Err(self.err("expected property name")),
        };
        Ok(PropKey::Name(name))
    }

    /// `of` — contextual keyword, stays an identifier elsewhere.
    fn eat_of(&mut self) -> bool {
        if matches!(self.peek(), Tok::Ident(s) if s == "of") {
            self.next();
            true
        } else {
            false
        }
    }

    /// `in`/`of` tail shared by the three `for` header forms.
    fn for_in_of(
        &mut self,
        var_target: ForTarget,
        var_init: Option<Expr>,
        is_await: bool,
    ) -> Result<Stmt> {
        if self.eat_of() {
            let iter = self.expr(false)?;
            self.expect_p(P::RParen, "`)`")?;
            let body = Box::new(self.stmt()?);
            return Ok(Stmt::ForOf {
                target: var_target,
                iter,
                body,
                is_await,
            });
        }
        if is_await {
            return Err(self.err("for await requires `of`"));
        }
        if self.eat_kw(Kw::In) {
            let obj = self.expr(false)?;
            self.expect_p(P::RParen, "`)`")?;
            let body = Box::new(self.stmt()?);
            return Ok(Stmt::ForIn {
                target: var_target,
                obj,
                var_init,
                body,
            });
        }
        Err(self.err("expected `in` or `of`"))
    }

    fn for_stmt(&mut self) -> Result<Stmt> {
        // `var` | `let` | `const` declarator keyword.
        enum DK {
            Var,
            Let,
            Const,
        }
        self.expect_kw(Kw::For, "`for`")?;
        // `for await (…)` — `await` is contextual; the paren follows.
        let is_await = matches!(self.peek(), Tok::Ident(s) if s == "await")
            && matches!(
                self.toks.get(self.pos + 1).map(|t| &t.tok),
                Some(Tok::P(P::LParen))
            );
        if is_await {
            self.next(); // `await`
        }
        self.expect_p(P::LParen, "`(`")?;
        // init clause — `in` is not an operator here.
        if self.eat_p(P::Semi) {
            return self.for_rest(None);
        }
        let dk = if self.eat_kw(Kw::Var) {
            Some(DK::Var)
        } else if self.eat_kw(Kw::Const) {
            Some(DK::Const)
        } else if self.eat_kw(Kw::Let) {
            Some(DK::Let)
        } else {
            None
        };
        if let Some(dk) = dk {
            // First decl parsed alone so `in`/`of` can follow it.
            let pat = self.pattern_decl()?;
            let var_init = if self.eat_p(P::Assign) {
                Some(self.assign(true)?)
            } else {
                None
            };
            let is_of = matches!(self.peek(), Tok::Ident(s) if s == "of");
            let is_in = self.peek() == &Tok::Kw(Kw::In);
            if is_of || is_in {
                if var_init.is_some() && is_of {
                    return Err(self.err("for-of target cannot have an initializer"));
                }
                let target = match dk {
                    DK::Var => ForTarget::Var(pat),
                    DK::Let | DK::Const => ForTarget::Lexical {
                        pat,
                        constant: matches!(dk, DK::Const),
                    },
                };
                return self.for_in_of(target, var_init.filter(|_| is_in), is_await);
            }
            // Normal for(;;) — continue the decl list.
            let mut decls = vec![VarDecl {
                pat,
                init: var_init,
            }];
            while self.eat_p(P::Comma) {
                let p = self.pattern_decl()?;
                let i = if self.eat_p(P::Assign) {
                    Some(self.assign(true)?)
                } else {
                    None
                };
                decls.push(VarDecl { pat: p, init: i });
            }
            let init = match dk {
                DK::Var => ForInit::Var(decls),
                DK::Let | DK::Const => ForInit::Lexical {
                    decls,
                    constant: matches!(dk, DK::Const),
                },
            };
            return self.for_rest(Some(init));
        }
        // `for (using x of y)` (ES2026) — `using` est décl seulement
        // si suivi d'un binding ≠ `of` (`for (using of y)` = ident).
        let is_using = matches!(self.peek(), Tok::Ident(s) if s == "using")
            && match self.toks.get(self.pos + 1).map(|t| &t.tok) {
                Some(Tok::P(P::LBracket | P::LBrace)) => true,
                Some(Tok::Ident(u)) => u != "of",
                _ => false,
            };
        if is_using {
            self.next(); // `using`
            let pat = self.pattern_decl()?;
            if !matches!(self.peek(), Tok::Ident(s) if s == "of") {
                return Err(self.err("`using` declarations require a `for-of` loop"));
            }
            return self.for_in_of(ForTarget::Using { pat, is_await }, None, is_await);
        }
        let init = self.expr(true)?;
        if matches!(self.peek(), Tok::Ident(s) if s == "of") || self.peek() == &Tok::Kw(Kw::In) {
            if !is_lvalue(&init) && !matches!(init, Expr::Array(_) | Expr::Object(_)) {
                return Err(self.err("invalid for-in/of target"));
            }
            return self.for_in_of(ForTarget::Expr(init), None, is_await);
        }
        self.for_rest(Some(ForInit::Expr(init)))
    }

    fn for_rest(&mut self, init: Option<ForInit>) -> Result<Stmt> {
        // first `;` already consumed by callers only in the None case —
        // for Var/Expr init the `;` is still ahead.
        if init.is_some() {
            self.expect_p(P::Semi, "`;`")?;
        }
        let test = if self.check_p(P::Semi) {
            None
        } else {
            Some(self.expr(false)?)
        };
        self.expect_p(P::Semi, "`;`")?;
        let update = if self.check_p(P::RParen) {
            None
        } else {
            Some(self.expr(false)?)
        };
        self.expect_p(P::RParen, "`)`")?;
        let body = Box::new(self.stmt()?);
        Ok(Stmt::For {
            init,
            test,
            update,
            body,
        })
    }

    fn try_stmt(&mut self) -> Result<Stmt> {
        self.expect_kw(Kw::Try, "`try`")?;
        self.expect_p(P::LBrace, "`{`")?;
        let block = self.stmt_list()?;
        let catch = if self.eat_kw(Kw::Catch) {
            // `try {} catch {}` (optional binding) or `catch (pat)`.
            if self.check_p(P::LBrace) {
                self.expect_p(P::LBrace, "`{`")?;
                Some((Pat::Ident(String::new()), self.stmt_list()?))
            } else {
                self.expect_p(P::LParen, "`(`")?;
                let pat = self.pattern()?;
                self.expect_p(P::RParen, "`)`")?;
                self.expect_p(P::LBrace, "`{`")?;
                Some((pat, self.stmt_list()?))
            }
        } else {
            None
        };
        let finally = if self.eat_kw(Kw::Finally) {
            self.expect_p(P::LBrace, "`{`")?;
            Some(self.stmt_list()?)
        } else {
            None
        };
        if catch.is_none() && finally.is_none() {
            return Err(self.err("`try` requires `catch` or `finally`"));
        }
        Ok(Stmt::Try {
            block,
            catch,
            finally,
        })
    }

    fn switch_stmt(&mut self) -> Result<Stmt> {
        self.expect_kw(Kw::Switch, "`switch`")?;
        self.expect_p(P::LParen, "`(`")?;
        let disc = self.expr(false)?;
        self.expect_p(P::RParen, "`)`")?;
        self.expect_p(P::LBrace, "`{`")?;
        let mut cases = Vec::new();
        while !self.check_p(P::RBrace) {
            if self.at_eof() {
                return Err(self.err("unterminated `switch`"));
            }
            let test = if self.eat_kw(Kw::Case) {
                let e = self.expr(false)?;
                Some(e)
            } else if self.eat_kw(Kw::Default) {
                None
            } else {
                return Err(self.err("expected `case` or `default`"));
            };
            self.expect_p(P::Colon, "`:`")?;
            let mut body = Vec::new();
            while !self.check_p(P::RBrace)
                && self.peek() != &Tok::Kw(Kw::Case)
                && self.peek() != &Tok::Kw(Kw::Default)
            {
                if self.at_eof() {
                    return Err(self.err("unterminated `switch`"));
                }
                body.push(self.stmt()?);
            }
            cases.push(SwitchCase { test, body });
        }
        self.expect_p(P::RBrace, "`}`")?;
        Ok(Stmt::Switch { disc, cases })
    }

    // -- expressions ---------------------------------------------------

    fn expr(&mut self, no_in: bool) -> Result<Expr> {
        self.nest_in()?;
        let r = stacker::maybe_grow(STACK_RED_ZONE, STACK_GROW, || self.expr_inner(no_in));
        self.nest_out();
        r
    }

    fn expr_inner(&mut self, no_in: bool) -> Result<Expr> {
        let first = self.assign(no_in)?;
        if !self.check_p(P::Comma) {
            return Ok(first);
        }
        let mut parts = vec![first];
        while self.eat_p(P::Comma) {
            parts.push(self.assign(no_in)?);
        }
        Ok(Expr::Seq(parts))
    }

    fn assign(&mut self, no_in: bool) -> Result<Expr> {
        self.nest_in()?;
        let r = stacker::maybe_grow(STACK_RED_ZONE, STACK_GROW, || self.assign_inner(no_in));
        self.nest_out();
        r
    }

    fn assign_inner(&mut self, no_in: bool) -> Result<Expr> {
        // `yield [e]` / `yield* it` — corps de générateur seulement ;
        // ailleurs `yield` reste un identifiant (réservé en strict
        // via `check_strict_binding`). Pas de saut de ligne avant
        // l'opérande (spec `YieldExpression`).
        if self.in_gen && matches!(self.peek(), Tok::Ident(s) if s == "yield") {
            self.next();
            let delegate = self.eat_p(P::Star);
            let no_arg = matches!(
                self.peek(),
                Tok::P(
                    P::RParen
                        | P::RBracket
                        | P::RBrace
                        | P::Comma
                        | P::Semi
                        | P::Colon
                        | P::Question
                ) | Tok::Eof
            ) || self.toks[self.pos].newline_before;
            let arg = if no_arg {
                None
            } else {
                Some(Box::new(self.assign(no_in)?))
            };
            return Ok(Expr::Yield { arg, delegate });
        }
        if let Some(arrow) = self.try_arrow(no_in)? {
            return Ok(arrow);
        }
        let lhs = self.cond(no_in)?;
        let op = match self.peek() {
            Tok::P(P::Assign) => AssignOp::Set,
            Tok::P(P::PlusEq) => AssignOp::Add,
            Tok::P(P::MinusEq) => AssignOp::Sub,
            Tok::P(P::StarEq) => AssignOp::Mul,
            Tok::P(P::SlashEq) => AssignOp::Div,
            Tok::P(P::PercentEq) => AssignOp::Mod,
            Tok::P(P::LtLtEq) => AssignOp::Shl,
            Tok::P(P::GtGtEq) => AssignOp::Shr,
            Tok::P(P::GtGtGtEq) => AssignOp::ShrU,
            Tok::P(P::AmpEq) => AssignOp::BitAnd,
            Tok::P(P::PipeEq) => AssignOp::BitOr,
            Tok::P(P::CaretEq) => AssignOp::BitXor,
            Tok::P(P::StarStarEq) => AssignOp::Pow,
            Tok::P(P::QQEq) => AssignOp::Nullish,
            Tok::P(P::AmpAmpEq) => AssignOp::LogAnd,
            Tok::P(P::PipePipeEq) => AssignOp::LogOr,
            _ => return Ok(lhs),
        };
        self.next();
        // Destructuring assignment: `[a, b] = e` / `({x} = o)` — the
        // lhs was parsed as an array/object literal; the interpreter
        // reinterprets it as a pattern. Only `=` is legal.
        let pat_ok = op == AssignOp::Set && matches!(lhs, Expr::Array(_) | Expr::Object(_));
        if !is_lvalue(&lhs) && !pat_ok {
            return Err(self.err("invalid assignment target"));
        }
        let rhs = self.assign(no_in)?;
        Ok(Expr::Assign(Box::new(lhs), op, Box::new(rhs)))
    }

    /// Arrow function in assignment position: `x => e`,
    /// `(a, b) => {…}`, `() => e`, `async x => e`. `None` when no
    /// `=>` follows — the `(` case is a pure lookahead (cursor
    /// restored on failure).
    fn try_arrow(&mut self, no_in: bool) -> Result<Option<Expr>> {
        // `async` arrow forms — `async x =>`, `async (…) =>`. Only
        // when `=>` actually follows (speculative like `(`).
        if matches!(self.peek(), Tok::Ident(s) if s == "async")
            && !self.toks[self.pos + 1].newline_before
        {
            if let Some(e) = self.try_async_arrow(no_in)? {
                return Ok(Some(e));
            }
        }
        // `ident => body`
        if let Tok::Ident(name) = self.peek().clone() {
            if self
                .toks
                .get(self.pos + 1)
                .is_some_and(|t| t.tok == Tok::P(P::Arrow))
            {
                let pos = self.pos_here();
                self.next();
                self.next();
                let (body, strict) = self.arrow_body(no_in)?;
                return Ok(Some(Expr::Function(Rc::new(FnLit {
                    name: None,
                    params: vec![Param {
                        pat: Pat::Ident(name),
                        rest: false,
                    }],
                    body,
                    is_arrow: true,
                    is_async: false,
                    is_gen: false,
                    strict,
                    pos,
                }))));
            }
        }
        // `(params) => body` — speculative scan of a param list.
        if self.check_p(P::LParen) {
            let save = self.pos;
            let pos = self.pos_here();
            if let Some(params) = self.scan_arrow_params() {
                // cursor is on `=>`
                self.next();
                let (body, strict) = self.arrow_body(no_in)?;
                return Ok(Some(Expr::Function(Rc::new(FnLit {
                    name: None,
                    params,
                    body,
                    is_arrow: true,
                    is_async: false,
                    is_gen: false,
                    strict,
                    pos,
                }))));
            }
            self.pos = save;
        }
        Ok(None)
    }

    /// `async x => e` / `async (…) => e` — speculative; `None` when
    /// the `=>` isn't there (then `async` is just an identifier).
    fn try_async_arrow(&mut self, no_in: bool) -> Result<Option<Expr>> {
        let save = self.pos;
        let pos = self.pos_here();
        self.next(); // `async`
                     // `async x => e`
        if let Tok::Ident(name) = self.peek().clone() {
            if self
                .toks
                .get(self.pos + 1)
                .is_some_and(|t| t.tok == Tok::P(P::Arrow))
            {
                self.next();
                self.next();
                let (body, strict) = self.arrow_body(no_in)?;
                return Ok(Some(Expr::Function(Rc::new(FnLit {
                    name: None,
                    params: vec![Param {
                        pat: Pat::Ident(name),
                        rest: false,
                    }],
                    body,
                    is_arrow: true,
                    is_async: true,
                    is_gen: false,
                    strict,
                    pos,
                }))));
            }
            self.pos = save;
            return Ok(None);
        }
        // `async (…) => e`
        if self.check_p(P::LParen) {
            if let Some(params) = self.scan_arrow_params() {
                if self.check_p(P::Arrow) {
                    self.next();
                    let (body, strict) = self.arrow_body(no_in)?;
                    return Ok(Some(Expr::Function(Rc::new(FnLit {
                        name: None,
                        params,
                        body,
                        is_arrow: true,
                        is_async: true,
                        is_gen: false,
                        strict,
                        pos,
                    }))));
                }
            }
        }
        self.pos = save;
        Ok(None)
    }

    /// From `(`: parse a param list — idents, patterns, `= default`,
    /// `...rest` — returning the params iff `=>` follows; cursor
    /// lands on `=>`. `None` (pos advanced!) if the parens aren't a
    /// param list — caller must restore `save`.
    fn scan_arrow_params(&mut self) -> Option<Vec<Param>> {
        self.next(); // (
        let params = self.params().ok()?;
        if self.check_p(P::Arrow) {
            Some(params)
        } else {
            None
        }
    }

    /// `=>` body: `{ stmts }` or a single expression (desugared to
    /// `return expr`).
    /// Corps de flèche — retourne aussi le strict effectif du corps
    /// (hérité ou prologue `"use strict"`).
    fn arrow_body(&mut self, no_in: bool) -> Result<(Vec<Stmt>, bool)> {
        if self.eat_p(P::LBrace) {
            let outer_strict = self.strict;
            let outer_gen = self.in_gen;
            self.in_gen = false; // les flèches ne sont jamais génératrices
            self.scan_directives();
            let body = self.stmt_list();
            let strict = self.strict;
            self.strict = outer_strict;
            self.in_gen = outer_gen;
            return body.map(|b| (b, strict));
        }
        let e = self.assign(no_in)?;
        let strict = self.strict;
        Ok((vec![Stmt::Return(Some(e))], strict))
    }

    fn cond(&mut self, no_in: bool) -> Result<Expr> {
        let c = self.binary(1, no_in)?;
        if self.eat_p(P::Question) {
            let t = self.assign(false)?;
            self.expect_p(P::Colon, "`:`")?;
            let e = self.assign(no_in)?;
            Ok(Expr::Cond(Box::new(c), Box::new(t), Box::new(e)))
        } else {
            Ok(c)
        }
    }

    fn binary(&mut self, min_prec: u8, no_in: bool) -> Result<Expr> {
        self.nest_in()?;
        let r = stacker::maybe_grow(STACK_RED_ZONE, STACK_GROW, || {
            self.binary_inner(min_prec, no_in)
        });
        self.nest_out();
        r
    }

    fn binary_inner(&mut self, min_prec: u8, no_in: bool) -> Result<Expr> {
        let mut lhs = self.unary(no_in)?;
        loop {
            let (op, prec, right_assoc) = match self.peek() {
                Tok::P(P::PipePipe) => (BinOp::Or, 1, false),
                Tok::P(P::QQ) => (BinOp::Nullish, 1, false),
                Tok::P(P::AmpAmp) => (BinOp::And, 2, false),
                Tok::P(P::Pipe) => (BinOp::BitOr, 3, false),
                Tok::P(P::Caret) => (BinOp::BitXor, 4, false),
                Tok::P(P::Amp) => (BinOp::BitAnd, 5, false),
                Tok::P(P::EqEq) => (BinOp::Eq, 6, false),
                Tok::P(P::Ne) => (BinOp::Ne, 6, false),
                Tok::P(P::EqEqEq) => (BinOp::StrictEq, 6, false),
                Tok::P(P::NeNe) => (BinOp::StrictNe, 6, false),
                Tok::P(P::Lt) => (BinOp::Lt, 7, false),
                Tok::P(P::Le) => (BinOp::Le, 7, false),
                Tok::P(P::Gt) => (BinOp::Gt, 7, false),
                Tok::P(P::Ge) => (BinOp::Ge, 7, false),
                Tok::Kw(Kw::Instanceof) => (BinOp::Instanceof, 7, false),
                Tok::Kw(Kw::In) if !no_in => (BinOp::In, 7, false),
                Tok::P(P::LtLt) => (BinOp::Shl, 8, false),
                Tok::P(P::GtGt) => (BinOp::Shr, 8, false),
                Tok::P(P::GtGtGt) => (BinOp::ShrU, 8, false),
                Tok::P(P::Plus) => (BinOp::Add, 9, false),
                Tok::P(P::Minus) => (BinOp::Sub, 9, false),
                Tok::P(P::Star) => (BinOp::Mul, 10, false),
                Tok::P(P::Slash) => (BinOp::Div, 10, false),
                Tok::P(P::Percent) => (BinOp::Mod, 10, false),
                Tok::P(P::StarStar) => (BinOp::Pow, 11, true),
                _ => break,
            };
            if prec < min_prec {
                break;
            }
            self.next();
            // `**` is right-associative: `2**3**2` = `2**(3**2)`.
            let rhs = self.binary(if right_assoc { prec } else { prec + 1 }, no_in)?;
            lhs = Expr::Binary(op, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn unary(&mut self, no_in: bool) -> Result<Expr> {
        // `await e` — contextual: `await` stays an identifier when no
        // operand follows (`var await = 1`, `o.await`, `await(x)`…).
        // We treat `await` as unary whenever the next token can start
        // an expression — `await = x` keeps the identifier meaning.
        if matches!(self.peek(), Tok::Ident(s) if s == "await")
            && !matches!(
                self.toks.get(self.pos + 1).map(|t| &t.tok),
                None | Some(
                    Tok::P(P::Semi | P::RParen | P::RBracket | P::Comma | P::Colon) | Tok::Eof
                )
            )
            && !matches!(
                self.toks.get(self.pos + 1).map(|t| &t.tok),
                Some(
                    Tok::P(
                        P::Assign
                            | P::PlusEq
                            | P::MinusEq
                            | P::StarEq
                            | P::SlashEq
                            | P::PercentEq
                            | P::AmpEq
                            | P::PipeEq
                            | P::CaretEq
                            | P::LtLtEq
                            | P::GtGtEq
                            | P::GtGtGtEq
                            | P::StarStarEq
                            | P::QQEq
                            | P::AmpAmpEq
                            | P::PipePipeEq
                            | P::Inc
                            | P::Dec
                            | P::Dot
                            | P::QDot
                    ) | Tok::Kw(Kw::In | Kw::Instanceof)
                )
            )
        {
            // Exclude binary-operator continuations too: `await + 1`
            // would misparse `+` as Pos anyway (fine — `await` then
            // unary `+` is still `await(+1)`, intended).
            self.next();
            self.nest_in()?;
            let e = stacker::maybe_grow(STACK_RED_ZONE, STACK_GROW, || self.unary(no_in));
            self.nest_out();
            return Ok(Expr::Await(Box::new(e?)));
        }
        let op = match self.peek() {
            Tok::P(P::Bang) => Some(UnOp::Not),
            Tok::P(P::Tilde) => Some(UnOp::BitNot),
            Tok::P(P::Plus) => Some(UnOp::Pos),
            Tok::P(P::Minus) => Some(UnOp::Neg),
            Tok::P(P::Inc) => Some(UnOp::PreInc),
            Tok::P(P::Dec) => Some(UnOp::PreDec),
            Tok::Kw(Kw::Typeof) => Some(UnOp::Typeof),
            Tok::Kw(Kw::Void) => Some(UnOp::Void),
            Tok::Kw(Kw::Delete) => Some(UnOp::Delete),
            _ => None,
        };
        if let Some(op) = op {
            self.next();
            self.nest_in()?;
            let e = stacker::maybe_grow(STACK_RED_ZONE, STACK_GROW, || self.unary(no_in));
            self.nest_out();
            let e = e?;
            // Strict : `delete` d'un identifiant nu est une early
            // error (seules les propriétés sont supprimables).
            if self.strict && op == UnOp::Delete && matches!(e, Expr::Ident(_)) {
                return Err(self.err("delete of unqualified identifier in strict mode"));
            }
            return Ok(Expr::Unary(op, Box::new(e)));
        }
        self.postfix(no_in)
    }

    fn postfix(&mut self, no_in: bool) -> Result<Expr> {
        let e = self.lhs_expr(no_in)?;
        if !self.cur().newline_before && !self.at_eof() {
            let op = match self.peek() {
                Tok::P(P::Inc) => Some(UnOp::PostInc),
                Tok::P(P::Dec) => Some(UnOp::PostDec),
                _ => None,
            };
            if let Some(op) = op {
                if !is_lvalue(&e) {
                    return Err(self.err("invalid increment target"));
                }
                self.next();
                return Ok(Expr::Unary(op, Box::new(e)));
            }
        }
        Ok(e)
    }

    /// Call/member chain plus `new` and `?.` (left-hand-side
    /// expressions). A `?.` switches the tail of the chain into
    /// `Expr::Chain` steps — everything after it is skipped when the
    /// `?.` base is nullish (`a?.b.c` never evaluates `.c`).
    fn lhs_expr(&mut self, no_in: bool) -> Result<Expr> {
        let mut e = if self.eat_kw(Kw::New) {
            if self.eat_p(P::Dot) {
                // `new.target` — meta-property : la seule paire
                // `new . nom` valide de la grammaire.
                let n = self.expect_prop_name()?;
                if n != "target" {
                    return Err(self.err("expected `new.target`"));
                }
                Expr::NewTarget
            } else {
                let callee = self.new_callee(no_in)?;
                let args = if self.check_p(P::LParen) {
                    self.args(no_in)?
                } else {
                    Vec::new()
                };
                Expr::New(Box::new(callee), args)
            }
        } else {
            self.primary(no_in)?
        };
        let mut steps: Option<Vec<ChainStep>> = None;
        loop {
            if self.eat_p(P::Dot) {
                let name = self.expect_prop_name()?;
                match &mut steps {
                    Some(s) => s.push(ChainStep::Member(name)),
                    None => e = Expr::Member(Box::new(e), name),
                }
            } else if self.eat_p(P::QDot) {
                let s = steps.get_or_insert_with(Vec::new);
                if self.check_p(P::LParen) {
                    let args = self.args(no_in)?;
                    s.push(ChainStep::OptCall(args));
                } else if self.eat_p(P::LBracket) {
                    let k = self.expr(false)?;
                    self.expect_p(P::RBracket, "`]`")?;
                    s.push(ChainStep::OptIndex(k));
                } else {
                    let name = self.expect_prop_name()?;
                    s.push(ChainStep::OptMember(name));
                }
            } else if self.eat_p(P::LBracket) {
                let k = self.expr(false)?;
                self.expect_p(P::RBracket, "`]`")?;
                match &mut steps {
                    Some(s) => s.push(ChainStep::Index(k)),
                    None => e = Expr::Index(Box::new(e), Box::new(k)),
                }
            } else if self.check_p(P::LParen) {
                let args = self.args(no_in)?;
                match &mut steps {
                    Some(s) => s.push(ChainStep::Call(args)),
                    None => e = Expr::Call(Box::new(e), args),
                }
            } else if let Tok::Tpl(_) = self.peek() {
                // `` tag`…` `` — tagged template (MemberExpression,
                // spec : interdit dans une chaîne optionnelle).
                if steps.is_some() {
                    return Err(self.err("template literal in an optional chain"));
                }
                let Tok::Tpl(parts) = self.next().tok.clone() else {
                    return Err(self.err("malformed template literal"));
                };
                let (parts, raws, exprs) = self.template_parts(parts, no_in)?;
                e = Expr::TaggedTemplate {
                    tag: Box::new(e),
                    parts,
                    raws,
                    exprs,
                };
            } else {
                break;
            }
        }
        Ok(match steps {
            Some(steps) => Expr::Chain {
                head: Box::new(e),
                steps,
            },
            None => e,
        })
    }

    /// Callee of `new`: another `new`, or primary + `.`/`[]` (no calls).
    fn new_callee(&mut self, no_in: bool) -> Result<Expr> {
        self.nest_in()?;
        let r = stacker::maybe_grow(STACK_RED_ZONE, STACK_GROW, || self.new_callee_inner(no_in));
        self.nest_out();
        r
    }

    fn new_callee_inner(&mut self, no_in: bool) -> Result<Expr> {
        let mut e = if self.eat_kw(Kw::New) {
            let callee = self.new_callee(no_in)?;
            let args = if self.check_p(P::LParen) {
                self.args(no_in)?
            } else {
                Vec::new()
            };
            Expr::New(Box::new(callee), args)
        } else {
            self.primary(no_in)?
        };
        loop {
            if self.eat_p(P::Dot) {
                let name = self.expect_prop_name()?;
                e = Expr::Member(Box::new(e), name);
            } else if self.eat_p(P::LBracket) {
                let k = self.expr(false)?;
                self.expect_p(P::RBracket, "`]`")?;
                e = Expr::Index(Box::new(e), Box::new(k));
            } else {
                return Ok(e);
            }
        }
    }

    fn args(&mut self, no_in: bool) -> Result<Vec<Expr>> {
        self.expect_p(P::LParen, "`(`")?;
        let mut out = Vec::new();
        if self.eat_p(P::RParen) {
            return Ok(out);
        }
        loop {
            // `f(...a)` spread arg — `Expr::Spread` marker.
            if self.eat_p(P::Ellipsis) {
                out.push(Expr::Spread(Box::new(self.assign(no_in)?)));
            } else {
                out.push(self.assign(no_in)?);
            }
            if self.eat_p(P::RParen) {
                return Ok(out);
            }
            self.expect_p(P::Comma, "`,`")?;
        }
    }

    fn primary(&mut self, no_in: bool) -> Result<Expr> {
        match self.peek().clone() {
            Tok::Num(n) => {
                if self.strict && self.cur().legacy_octal {
                    return Err(self.err("legacy octal literal in strict mode"));
                }
                self.next();
                Ok(Expr::Num(n))
            }
            Tok::Big(s) => {
                self.next();
                Ok(Expr::BigInt(s))
            }
            Tok::Str(s) => {
                if self.strict && self.cur().legacy_octal {
                    return Err(self.err("legacy octal escape in strict mode"));
                }
                self.next();
                Ok(Expr::Str(s))
            }
            Tok::Ident(s) => {
                // `async function [f](){}` — expression form.
                if s == "async"
                    && matches!(
                        self.toks.get(self.pos + 1),
                        Some(t) if t.tok == Tok::Kw(Kw::Function) && !t.newline_before
                    )
                {
                    self.next();
                    return Ok(Expr::Function(self.function(false, true)?));
                }
                self.next();
                Ok(Expr::Ident(s))
            }
            Tok::Kw(Kw::Class) => Ok(Expr::Class(self.class_lit()?)),
            Tok::Kw(Kw::Super) => {
                self.next();
                if self.check_p(P::LParen) {
                    let args = self.args(no_in)?;
                    return Ok(Expr::SuperCall(args));
                }
                if self.eat_p(P::Dot) {
                    let name = self.expect_prop_name()?;
                    return Ok(Expr::SuperMember(name));
                }
                if self.eat_p(P::LBracket) {
                    let k = self.expr(false)?;
                    self.expect_p(P::RBracket, "`]`")?;
                    return Ok(Expr::SuperIndex(Box::new(k)));
                }
                Err(self.err("expected `super` call or member"))
            }
            Tok::Kw(Kw::True) => {
                self.next();
                Ok(Expr::Bool(true))
            }
            Tok::Kw(Kw::False) => {
                self.next();
                Ok(Expr::Bool(false))
            }
            Tok::Kw(Kw::Null) => {
                self.next();
                Ok(Expr::Null)
            }
            Tok::Kw(Kw::This) => {
                self.next();
                Ok(Expr::This)
            }
            // `#x` en position primaire : seule utilisation
            // spec = `x #x in obj` — on produit la clé sentinelle
            // comme opérande chaîne ; `in` fait le lookup.
            Tok::Private(n) => {
                self.next();
                Ok(Expr::Str(self.resolve_priv(&n)?))
            }
            Tok::Kw(Kw::Function) => Ok(Expr::Function(self.function(false, false)?)),
            // Sloppy tolerance: `let` works as an identifier.
            Tok::Kw(Kw::Let) => {
                self.next();
                Ok(Expr::Ident("let".into()))
            }
            Tok::Tpl(parts) => {
                self.next();
                self.template(parts, no_in)
            }
            Tok::Regex { body, flags } => {
                self.next();
                // Validation eager (spec `negative.phase: parse`) :
                // pattern/flags invalides → SyntaxError au parse,
                // pas au premier `.test()`.
                if let Err(msg) = crate::regexp::JsRegExp::compile(&body, &flags) {
                    return Err(self.err(msg));
                }
                Ok(Expr::Regex {
                    pattern: body,
                    flags,
                })
            }
            // `import(spec)` / `import.meta` — legal in modules and
            // classic scripts alike.
            Tok::Kw(Kw::Import) => {
                self.next();
                if self.eat_p(P::LParen) {
                    let spec = self.assign(no_in)?;
                    self.expect_p(P::RParen, "`)`")?;
                    Ok(Expr::ImportCall(Box::new(spec)))
                } else if self.eat_p(P::Dot) {
                    self.expect_ident("meta")?;
                    Ok(Expr::ImportMeta)
                } else {
                    Err(self.err("expected `(` or `.meta` after `import`"))
                }
            }
            Tok::Kw(Kw::Unsupported(k)) => Err(self.err(format!("`{k}` is not supported"))),
            Tok::P(P::LParen) => {
                self.next();
                let e = self.expr(false)?;
                self.expect_p(P::RParen, "`)`")?;
                Ok(e)
            }
            Tok::P(P::LBracket) => self.array_lit(no_in),
            Tok::P(P::LBrace) => self.object_lit(no_in),
            _ => Err(self.err("expected expression")),
        }
    }

    fn array_lit(&mut self, no_in: bool) -> Result<Expr> {
        self.expect_p(P::LBracket, "`[`")?;
        let mut elems = Vec::new();
        loop {
            if self.eat_p(P::RBracket) {
                return Ok(Expr::Array(elems));
            }
            if self.eat_p(P::Comma) {
                elems.push(None); // hole
                continue;
            }
            // `[...a]` spread element — `Expr::Spread` marker.
            if self.eat_p(P::Ellipsis) {
                elems.push(Some(Expr::Spread(Box::new(self.assign(no_in)?))));
            } else {
                elems.push(Some(self.assign(no_in)?));
            }
            if self.eat_p(P::Comma) {
                continue;
            }
            self.expect_p(P::RBracket, "`]`")?;
            return Ok(Expr::Array(elems));
        }
    }

    fn object_lit(&mut self, no_in: bool) -> Result<Expr> {
        self.expect_p(P::LBrace, "`{`")?;
        let mut props = Vec::new();
        if self.eat_p(P::RBrace) {
            return Ok(Expr::Object(props));
        }
        loop {
            // `{...o}` spread.
            if self.eat_p(P::Ellipsis) {
                props.push(ObjProp::Spread(self.assign(no_in)?));
            } else {
                let kpos = self.pos_here();
                // `*m(){}` generator method / `async *m(){}`.
                let mut gen = self.eat_p(P::Star);
                // `async m(){}` method.
                let async_ = matches!(self.peek(), Tok::Ident(s) if s == "async")
                    && !matches!(
                        self.toks.get(self.pos + 1).map(|t| &t.tok),
                        Some(Tok::P(
                            P::LParen | P::Assign | P::Colon | P::Comma | P::RBrace
                        ))
                    )
                    && if self
                        .toks
                        .get(self.pos + 1)
                        .is_some_and(|t| !t.newline_before)
                    {
                        self.next();
                        true
                    } else {
                        false
                    };
                gen |= self.eat_p(P::Star); // `async *m(){}`
                                            // `get k(){}`/`set k(v){}` vs a `get:`/`set:` prop.
                let mut accessor: Option<ClassMemberKind> = None;
                if matches!(self.peek(), Tok::Ident(s) if s == "get" || s == "set")
                    && !matches!(
                        self.toks.get(self.pos + 1).map(|t| &t.tok),
                        Some(Tok::P(
                            P::LParen | P::Assign | P::Colon | P::Comma | P::RBrace
                        ))
                    )
                {
                    accessor = Some(if matches!(self.peek(), Tok::Ident(s) if s == "get") {
                        self.next();
                        ClassMemberKind::Getter
                    } else {
                        self.next();
                        ClassMemberKind::Setter
                    });
                }
                let key = self.prop_key()?;
                match accessor {
                    Some(kind) => {
                        self.expect_p(P::LParen, "`(`")?;
                        let params = self.params()?;
                        self.expect_p(P::LBrace, "`{`")?;
                        self.in_gen = false; // get/set : jamais générateurs
                        let body = self.fn_body()?;
                        let fun = Rc::new(FnLit {
                            name: None,
                            params,
                            body: body.0,
                            is_arrow: false,
                            is_async: false,
                            is_gen: false,
                            strict: body.1,
                            pos: kpos,
                        });
                        props.push(if kind == ClassMemberKind::Getter {
                            ObjProp::Getter(key, fun)
                        } else {
                            ObjProp::Setter(key, fun)
                        });
                    }
                    None if self.check_p(P::LParen) => {
                        // Shorthand method `{ f(a) {…} }` / `{ *f() {…} }`.
                        self.expect_p(P::LParen, "`(`")?;
                        let params = self.params()?;
                        self.expect_p(P::LBrace, "`{`")?;
                        self.in_gen = gen;
                        let body = self.fn_body()?;
                        props.push(ObjProp::Prop(
                            key,
                            Expr::Function(Rc::new(FnLit {
                                name: None,
                                params,
                                body: body.0,
                                is_arrow: false,
                                is_async: async_,
                                is_gen: gen,
                                strict: body.1,
                                pos: kpos,
                            })),
                        ));
                    }
                    None => {
                        if self.eat_p(P::Colon) {
                            props.push(ObjProp::Prop(key, self.assign(no_in)?));
                        } else if self.eat_p(P::Assign) {
                            // `{a = 1}` — destructuring-default shape;
                            // as a plain literal it just assigns then
                            // stores the value (harmless tolerance).
                            let name = match &key {
                                PropKey::Name(n) => n.clone(),
                                PropKey::Computed(_) => {
                                    return Err(self.err("bad `{…=…}` shorthand"))
                                }
                            };
                            let rhs = self.assign(no_in)?;
                            props.push(ObjProp::Prop(
                                key,
                                Expr::Assign(
                                    Box::new(Expr::Ident(name)),
                                    AssignOp::Set,
                                    Box::new(rhs),
                                ),
                            ));
                        } else {
                            // Shorthand `{x}` → `x: x` (computed keys
                            // cannot be shorthand).
                            let name = match &key {
                                PropKey::Name(n) => n.clone(),
                                PropKey::Computed(_) => {
                                    return Err(self.err("computed key cannot be shorthand"))
                                }
                            };
                            props.push(ObjProp::Prop(key, Expr::Ident(name)));
                        }
                    }
                }
            }
            if self.eat_p(P::Comma) {
                if self.eat_p(P::RBrace) {
                    return Ok(Expr::Object(props));
                }
                continue;
            }
            self.expect_p(P::RBrace, "`}`")?;
            return Ok(Expr::Object(props));
        }
    }

    /// `` `t${e}t` `` — `parts.len() == exprs.len() + 1` enforced.
    fn template(&mut self, parts: Vec<TplPart>, no_in: bool) -> Result<Expr> {
        let (lit_parts, _raws, exprs) = self.template_parts(parts, no_in)?;
        Ok(Expr::Template {
            parts: lit_parts,
            exprs,
        })
    }

    /// Chunks d'un template : `(cooked, raws, exprs)` — partagé
    /// entre `` `…` `` (`Expr::Template`) et `` tag`…` ``
    /// (`Expr::TaggedTemplate`, qui consomme aussi les raws).
    fn template_parts(
        &mut self,
        parts: Vec<TplPart>,
        no_in: bool,
    ) -> Result<(Vec<String>, Vec<String>, Vec<Expr>)> {
        let mut lit_parts = Vec::new();
        let mut raws = Vec::new();
        let mut exprs = Vec::new();
        for part in parts {
            match part {
                TplPart::Str(s, raw) => {
                    lit_parts.push(s);
                    raws.push(raw);
                }
                TplPart::Expr(toks) => {
                    let mut sub = Parser {
                        toks,
                        pos: 0,
                        // Accumule l'emboîtement : `${`x${`…`}`}`
                        // récursive via des sous-parseurs (JS-C41).
                        nest: self.nest,
                        nest_max: self.nest_max,
                        strict: self.strict,
                        module: self.module,
                        in_gen: self.in_gen,
                        labels: Vec::new(),
                        // Hérite les portées privées — `#x` dans une
                        // substitution se résout dans la classe
                        // englobante ; l'identité reste cohérente.
                        priv_seq: self.priv_seq,
                        priv_scopes: self.priv_scopes.clone(),
                    };
                    let e = sub.expr(no_in)?;
                    self.priv_seq = self.priv_seq.max(sub.priv_seq);
                    if !sub.at_eof() {
                        return Err(self.err("trailing tokens in template substitution"));
                    }
                    exprs.push(e);
                }
            }
        }
        if lit_parts.len() != exprs.len() + 1 {
            return Err(self.err("malformed template literal"));
        }
        Ok((lit_parts, raws, exprs))
    }

    /// `function [name] (params) { body }` — name required iff `decl`.
    /// `async` marks `FnLit::is_async` (call wraps the result in a
    /// resolved promise — see ast.rs).
    fn function(&mut self, decl: bool, async_: bool) -> Result<Rc<FnLit>> {
        let pos = self.pos_here();
        self.expect_kw(Kw::Function, "`function`")?;
        // `function*` / `async function*`.
        let gen = self.eat_p(P::Star);
        let name = match self.peek().clone() {
            Tok::Ident(s) => {
                self.next();
                Some(s)
            }
            _ if decl => return Err(self.err("function declaration needs a name")),
            _ => None,
        };
        if let Some(n) = &name {
            self.check_strict_binding(n)?;
        }
        self.expect_p(P::LParen, "`(`")?;
        let params = self.params()?;
        self.expect_p(P::LBrace, "`{`")?;
        // Le strict du corps = hérité OU directive propre — les
        // paramètres ont été parsés dans le mode du parent (la spec
        // exige des params « simples » quand le corps est strict ;
        // tolérance : on n'a pas de paramètres non-simples).
        self.in_gen = gen;
        let (body, strict) = self.fn_body()?;
        Ok(Rc::new(FnLit {
            name,
            params,
            body,
            is_arrow: false,
            is_async: async_,
            is_gen: gen,
            strict,
            pos,
        }))
    }

    /// Corps de fonction déjà ouvert (`{` consommé) — prologue de
    /// directives scanné, retourne `(stmts, strict_effectif)`.
    /// `self.in_gen` est celui posé par l'appelant (save/restore ici).
    fn fn_body(&mut self) -> Result<(Vec<Stmt>, bool)> {
        let outer_strict = self.strict;
        let outer_gen = self.in_gen;
        // Les labels ne traversent pas la frontière fonction.
        let outer_labels = std::mem::take(&mut self.labels);
        self.scan_directives();
        let body = self.stmt_list();
        let strict = self.strict;
        self.strict = outer_strict;
        self.in_gen = outer_gen;
        self.labels = outer_labels;
        body.map(|b| (b, strict))
    }

    /// `pat (= default)? (, …)*` with a possible trailing `...rest`
    /// — cursor left after `)`.
    fn params(&mut self) -> Result<Vec<Param>> {
        let mut params = Vec::new();
        if !self.check_p(P::RParen) {
            loop {
                let rest = self.eat_p(P::Ellipsis);
                let pat = self.pattern()?;
                params.push(Param { pat, rest });
                if rest {
                    break; // `...rest` must be last
                }
                if !self.eat_p(P::Comma) {
                    break;
                }
                if self.check_p(P::RParen) {
                    break; // trailing comma
                }
            }
        }
        self.expect_p(P::RParen, "`)`")?;
        Ok(params)
    }

    /// `class [C] [extends e] { members }` — statement or
    /// expression position produce the same literal.
    fn class_lit(&mut self) -> Result<Rc<ClassLit>> {
        // JS-C41 : `class A extends class B extends …` boucle
        // `class_lit → lhs_expr → primary → class_lit` — borne
        // + segment tas comme les autres transporteurs.
        self.nest_in()?;
        let r = stacker::maybe_grow(STACK_RED_ZONE, STACK_GROW, || self.class_lit_inner());
        self.nest_out();
        r
    }

    fn class_lit_inner(&mut self) -> Result<Rc<ClassLit>> {
        self.expect_kw(Kw::Class, "`class`")?;
        let name = match self.peek().clone() {
            Tok::Ident(s) => {
                self.next();
                Some(s)
            }
            _ => None,
        };
        let super_ = if self.eat_kw(Kw::Extends) {
            Some(self.lhs_expr(false)?)
        } else {
            None
        };
        self.expect_p(P::LBrace, "`{`")?;
        // Portée des noms privés : les `#…` déclarés dans ce corps
        // sont pré-scannés ; toute référence `#x` pendant le parse
        // se résout vers `\u{f131}privN.x`.
        self.priv_seq += 1;
        let seq = self.priv_seq;
        let decls = self.prescan_privates();
        self.priv_scopes.push((seq, decls));
        let mut members = Vec::new();
        while !self.at_eof() && !self.check_p(P::RBrace) {
            // `;` — élément de classe vide (ClassElement de la
            // grammaire), fréquent dans les bundles minifiés
            // (`m(){};n(){}`). Mesuré sur github.com (JS-C56).
            if self.eat_p(P::Semi) {
                continue;
            }
            let mpos = self.pos_here();
            let mut static_ = false;
            // `static m()` vs a method literally named `static`.
            if matches!(self.peek(), Tok::Ident(s) if s == "static")
                && !matches!(
                    self.toks.get(self.pos + 1).map(|t| &t.tok),
                    Some(Tok::P(
                        P::LParen | P::Assign | P::Colon | P::Semi | P::RBrace
                    ))
                )
            {
                self.next();
                static_ = true;
            }
            // `static { … }` — bloc d'initialisation statique.
            if static_ && self.check_p(P::LBrace) {
                self.next();
                let outer_strict = self.strict;
                self.strict = true;
                let body = self.stmt_list()?;
                self.strict = outer_strict;
                members.push(ClassMember {
                    key: PropKey::Name(String::new()),
                    kind: ClassMemberKind::StaticBlock,
                    static_: true,
                    fun: Rc::new(FnLit {
                        name: None,
                        params: Vec::new(),
                        body,
                        is_arrow: false,
                        is_async: false,
                        is_gen: false,
                        strict: true,
                        pos: mpos,
                    }),
                });
                continue;
            }
            // `async m()` — same lookahead rule.
            let async_ = matches!(self.peek(), Tok::Ident(s) if s == "async")
                && !matches!(
                    self.toks.get(self.pos + 1).map(|t| &t.tok),
                    Some(Tok::P(
                        P::LParen | P::Assign | P::Colon | P::Semi | P::RBrace
                    ))
                )
                && if self
                    .toks
                    .get(self.pos + 1)
                    .is_some_and(|t| !t.newline_before)
                {
                    self.next();
                    true
                } else {
                    false
                };
            let gen = self.eat_p(P::Star); // `*m(){}` / `async *m(){}`
                                           // `get k()`/`set k(v)` vs a plain `get`/`set` member.
            let mut kind = ClassMemberKind::Method;
            if matches!(self.peek(), Tok::Ident(s) if s == "get" || s == "set")
                && !matches!(
                    self.toks.get(self.pos + 1).map(|t| &t.tok),
                    Some(Tok::P(
                        P::LParen | P::Assign | P::Colon | P::Semi | P::RBrace | P::Comma
                    ))
                )
            {
                kind = if matches!(self.peek(), Tok::Ident(s) if s == "get") {
                    self.next();
                    ClassMemberKind::Getter
                } else {
                    self.next();
                    ClassMemberKind::Setter
                };
            }
            let key = self.prop_key()?;
            if !static_ && kind == ClassMemberKind::Method {
                if let PropKey::Name(n) = &key {
                    if n == "constructor" {
                        kind = ClassMemberKind::Ctor;
                    }
                }
            }
            // `x` / `x = e` / `#x = e` — pas de `(` : champ
            // d'instance (ou statique). L'init devient un thunk
            // exécuté par le runtime à la construction.
            if !self.check_p(P::LParen) {
                let init = if self.eat_p(P::Assign) {
                    Some(self.assign(false)?)
                } else {
                    None
                };
                self.eat_p(P::Semi);
                let body = init
                    .map(|e| vec![Stmt::Return(Some(e))])
                    .unwrap_or_default();
                members.push(ClassMember {
                    key,
                    kind: ClassMemberKind::Field,
                    static_,
                    fun: Rc::new(FnLit {
                        name: None,
                        params: Vec::new(),
                        body,
                        is_arrow: false,
                        is_async: false,
                        is_gen: false,
                        strict: true,
                        pos: mpos,
                    }),
                });
                continue;
            }
            self.expect_p(P::LParen, "`(`")?;
            let params = self.params()?;
            self.expect_p(P::LBrace, "`{`")?;
            // Corps de classe : toujours strict, prologue scanné
            // pour les early errors.
            let outer_strict = self.strict;
            self.strict = true;
            let outer_gen = self.in_gen;
            // get/set ne sont jamais générateurs ; `*m` oui.
            self.in_gen = gen && kind == ClassMemberKind::Method;
            let body = self.stmt_list();
            let is_gen = self.in_gen;
            self.strict = outer_strict;
            self.in_gen = outer_gen;
            members.push(ClassMember {
                key,
                kind,
                static_,
                fun: Rc::new(FnLit {
                    name: None,
                    params,
                    body: body?,
                    is_arrow: false,
                    is_async: async_,
                    is_gen,
                    strict: true,
                    pos: mpos,
                }),
            });
        }
        self.expect_p(P::RBrace, "`}`")?;
        self.priv_scopes.pop();
        Ok(Rc::new(ClassLit {
            name,
            super_,
            members,
        }))
    }
}

fn is_lvalue(e: &Expr) -> bool {
    matches!(e, Expr::Ident(_) | Expr::Member(..) | Expr::Index(..))
}

/// Source spelling of a keyword — used for reserved-word member
/// names (`o.function`) and object keys.
fn kw_str(kw: Kw) -> &'static str {
    match kw {
        Kw::Var => "var",
        Kw::Let => "let",
        Kw::Const => "const",
        Kw::Function => "function",
        Kw::Return => "return",
        Kw::If => "if",
        Kw::Else => "else",
        Kw::While => "while",
        Kw::With => "with",
        Kw::Do => "do",
        Kw::For => "for",
        Kw::Break => "break",
        Kw::Continue => "continue",
        Kw::Switch => "switch",
        Kw::Case => "case",
        Kw::Default => "default",
        Kw::Throw => "throw",
        Kw::Try => "try",
        Kw::Catch => "catch",
        Kw::Finally => "finally",
        Kw::New => "new",
        Kw::Delete => "delete",
        Kw::Typeof => "typeof",
        Kw::Void => "void",
        Kw::In => "in",
        Kw::Instanceof => "instanceof",
        Kw::This => "this",
        Kw::True => "true",
        Kw::False => "false",
        Kw::Null => "null",
        Kw::Class => "class",
        Kw::Extends => "extends",
        Kw::Super => "super",
        Kw::Import => "import",
        Kw::Export => "export",
        Kw::Unsupported(s) => s,
    }
}
