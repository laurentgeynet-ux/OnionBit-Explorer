// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Recursive-descent parser for the `dillo-js` subset.
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
use std::rc::Rc;

/// Parse a whole script.
pub fn parse(src: &str) -> Result<Program> {
    parse_in(src, false)
}

/// Parse avec strict hérité — `eval` direct dans du code strict :
/// ses early errors s'appliquent aussi au texte évalué (spec
/// `PerformEval` — « strictCaller »).
pub(crate) fn parse_in(src: &str, strict: bool) -> Result<Program> {
    let toks = lex(src)?;
    let mut p = Parser {
        toks,
        pos: 0,
        nest: 0,
        strict,
        module: false,
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
    let toks = lex(src)?;
    let mut p = Parser {
        toks,
        pos: 0,
        nest: 0,
        strict: true,
        module: true,
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

struct Parser {
    toks: Vec<Token>,
    pos: usize,
    /// Expression nesting guard — `((((…` input must not blow the
    /// native stack through recursive descent.
    nest: u32,
    /// `true` dans une unité `"use strict"` — script ou corps de
    /// fonction (hérité par les corps internes). Pilote les early
    /// errors strict : octales legacy, `delete x`, liaisons
    /// réservées.
    strict: bool,
    /// `true` dans le but de grammaire *module* — autorise les
    /// statements `import`/`export` au niveau top.
    module: bool,
}

impl Parser {
    // -- token plumbing ------------------------------------------------

    fn peek(&self) -> &Tok {
        &self.toks[self.pos].tok
    }

    fn cur(&self) -> &Token {
        &self.toks[self.pos]
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
            _ => Err(self.err("expected property name")),
        }
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

    fn stmt(&mut self) -> Result<Stmt> {
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
            Tok::Kw(Kw::If) => self.if_stmt(),
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
                self.expect_semi()?;
                Ok(Stmt::Break)
            }
            Tok::Kw(Kw::Continue) => {
                self.next();
                self.expect_semi()?;
                Ok(Stmt::Continue)
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
    fn for_in_of(&mut self, var_target: ForTarget, var_init: Option<Expr>) -> Result<Stmt> {
        if self.eat_of() {
            let iter = self.expr(false)?;
            self.expect_p(P::RParen, "`)`")?;
            let body = Box::new(self.stmt()?);
            return Ok(Stmt::ForOf {
                target: var_target,
                iter,
                body,
            });
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
        if matches!(self.peek(), Tok::Ident(s) if s == "await")
            && matches!(
                self.toks.get(self.pos + 1).map(|t| &t.tok),
                Some(Tok::P(P::LParen))
            )
        {
            self.next(); // `await` — same loop, just tolerant
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
                return self.for_in_of(target, var_init.filter(|_| is_in));
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
        let init = self.expr(true)?;
        if matches!(self.peek(), Tok::Ident(s) if s == "of") || self.peek() == &Tok::Kw(Kw::In) {
            if !is_lvalue(&init) && !matches!(init, Expr::Array(_) | Expr::Object(_)) {
                return Err(self.err("invalid for-in/of target"));
            }
            return self.for_in_of(ForTarget::Expr(init), None);
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
        self.nest += 1;
        if self.nest > 300 {
            self.nest -= 1;
            return Err(self.err("expression too deeply nested"));
        }
        let r = self.expr_inner(no_in);
        self.nest -= 1;
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
                    strict,
                }))));
            }
        }
        // `(params) => body` — speculative scan of a param list.
        if self.check_p(P::LParen) {
            let save = self.pos;
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
                    strict,
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
                    strict,
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
                        strict,
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
            self.scan_directives();
            let body = self.stmt_list();
            let strict = self.strict;
            self.strict = outer_strict;
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
            let e = self.unary(no_in)?;
            return Ok(Expr::Await(Box::new(e)));
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
            self.nest += 1;
            if self.nest > 300 {
                self.nest -= 1;
                return Err(self.err("expression too deeply nested"));
            }
            let e = self.unary(no_in);
            self.nest -= 1;
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
                        let body = self.fn_body()?;
                        let fun = Rc::new(FnLit {
                            name: None,
                            params,
                            body: body.0,
                            is_arrow: false,
                            is_async: false,
                            strict: body.1,
                        });
                        props.push(if kind == ClassMemberKind::Getter {
                            ObjProp::Getter(key, fun)
                        } else {
                            ObjProp::Setter(key, fun)
                        });
                    }
                    None if self.check_p(P::LParen) => {
                        // Shorthand method `{ f(a) {…} }`.
                        self.expect_p(P::LParen, "`(`")?;
                        let params = self.params()?;
                        self.expect_p(P::LBrace, "`{`")?;
                        let body = self.fn_body()?;
                        props.push(ObjProp::Prop(
                            key,
                            Expr::Function(Rc::new(FnLit {
                                name: None,
                                params,
                                body: body.0,
                                is_arrow: false,
                                is_async: async_,
                                strict: body.1,
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
        let mut lit_parts = Vec::new();
        let mut exprs = Vec::new();
        for part in parts {
            match part {
                TplPart::Str(s) => lit_parts.push(s),
                TplPart::Expr(toks) => {
                    let mut sub = Parser {
                        toks,
                        pos: 0,
                        nest: 0,
                        strict: self.strict,
                        module: self.module,
                    };
                    let e = sub.expr(no_in)?;
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
        Ok(Expr::Template {
            parts: lit_parts,
            exprs,
        })
    }

    /// `function [name] (params) { body }` — name required iff `decl`.
    /// `async` marks `FnLit::is_async` (call wraps the result in a
    /// resolved promise — see ast.rs).
    fn function(&mut self, decl: bool, async_: bool) -> Result<Rc<FnLit>> {
        self.expect_kw(Kw::Function, "`function`")?;
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
        let (body, strict) = self.fn_body()?;
        Ok(Rc::new(FnLit {
            name,
            params,
            body,
            is_arrow: false,
            is_async: async_,
            strict,
        }))
    }

    /// Corps de fonction déjà ouvert (`{` consommé) — prologue de
    /// directives scanné, retourne `(stmts, strict_effectif)`.
    fn fn_body(&mut self) -> Result<(Vec<Stmt>, bool)> {
        let outer_strict = self.strict;
        self.scan_directives();
        let body = self.stmt_list();
        let strict = self.strict;
        self.strict = outer_strict;
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
        let mut members = Vec::new();
        while !self.at_eof() && !self.check_p(P::RBrace) {
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
            self.expect_p(P::LParen, "`(`")?;
            let params = self.params()?;
            self.expect_p(P::LBrace, "`{`")?;
            // Corps de classe : toujours strict, prologue scanné
            // pour les early errors.
            let outer_strict = self.strict;
            self.strict = true;
            let body = self.stmt_list();
            self.strict = outer_strict;
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
                    strict: true,
                }),
            });
        }
        self.expect_p(P::RBrace, "`}`")?;
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
