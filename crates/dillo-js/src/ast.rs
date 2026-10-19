// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! AST for `dillo-js` — ES3 core plus pragmatic ES2015+ coverage.
//!
//! Covered beyond ES3: `let`/`const`, arrow functions, template
//! literals, optional chaining (`?.`) / nullish (`??`), `**`,
//! logical assignment (`&&=`/`||=`/`??=`). Still rejected up front:
//! `with`, labels/labelled break-continue, regex literals, classes,
//! generators, modules, destructuring, `for-of`.

use std::rc::Rc;

/// A whole script.
#[derive(Debug, Clone)]
pub struct Program {
    /// Top-level statements.
    pub body: Vec<Stmt>,
    /// `"use strict"` en prologue — le runtime bascule : écritures
    /// refusées → `TypeError`, identifiants non déclarés en
    /// écriture → `ReferenceError`, `delete` non-configurable →
    /// `TypeError`, `this` non coercé.
    pub strict: bool,
}

/// Statements.
#[derive(Debug, Clone)]
pub enum Stmt {
    /// `;`
    Empty,
    /// Expression statement.
    Expr(Expr),
    /// `var a = 1, b;`
    Var(Vec<VarDecl>),
    /// `let`/`const a = 1, b;` — declared in the *current* scope,
    /// not hoisted (block scoping is approximated: no TDZ).
    Lexical {
        /// Declarations.
        decls: Vec<VarDecl>,
        /// `const` — writes after init are ignored (sloppy tolerance,
        /// documented deviation — real engines throw `TypeError`).
        constant: bool,
    },
    /// `function f(…) { … }` — hoisted, executing it is a no-op.
    FunctionDecl(Rc<FnLit>),
    /// `class C extends B {…}` — lexical (`let`-like) binding.
    ClassDecl(Rc<ClassLit>),
    /// `{ … }`
    Block(Vec<Stmt>),
    /// `if (c) t else e`
    If {
        /// Condition.
        cond: Expr,
        /// Taken when truthy.
        then: Box<Stmt>,
        /// Taken when falsy.
        else_: Option<Box<Stmt>>,
    },
    /// `while (c) b`
    While {
        /// Condition.
        cond: Expr,
        /// Loop body.
        body: Box<Stmt>,
    },
    /// `do b while (c)`
    DoWhile {
        /// Loop body.
        body: Box<Stmt>,
        /// Condition.
        cond: Expr,
    },
    /// `for (init; test; update) b`
    For {
        /// Init clause.
        init: Option<ForInit>,
        /// Test clause.
        test: Option<Expr>,
        /// Update clause.
        update: Option<Expr>,
        /// Loop body.
        body: Box<Stmt>,
    },
    /// `for (x in o) b`
    ForIn {
        /// Loop variable target.
        target: ForTarget,
        /// Enumerated object expression.
        obj: Expr,
        /// Optional init for `for (var x = e in o)` (ES3 quirk).
        var_init: Option<Expr>,
        /// Loop body.
        body: Box<Stmt>,
    },
    /// `for (x of iterable) b` — iterator protocol.
    ForOf {
        /// Loop variable target (may destructure).
        target: ForTarget,
        /// Iterated expression.
        iter: Expr,
        /// Loop body.
        body: Box<Stmt>,
        /// `for await` — async iterator protocol.
        is_await: bool,
    },
    /// `with (obj) b` — object-as-scope (sloppy only).
    With {
        /// Scope object.
        obj: Expr,
        /// Body.
        body: Box<Stmt>,
    },
    /// `lbl : stmt` — statement étiqueté (cible de `break lbl` /
    /// `continue lbl`).
    Labeled {
        /// Label name.
        name: String,
        /// Labelled statement.
        body: Box<Stmt>,
    },
    /// `break [label]`
    Break(Option<String>),
    /// `continue [label]` — `Some` restreint aux labels de boucle
    /// (early error du parseur).
    Continue(Option<String>),
    /// `return [e]`
    Return(Option<Expr>),
    /// `throw e`
    Throw(Expr),
    /// `try {…} catch (e) {…} finally {…}` — the catch binding is a
    /// pattern (`catch ({message})` is legal ES2019).
    Try {
        /// Guarded block.
        block: Vec<Stmt>,
        /// `(pattern, body)` of the catch clause.
        catch: Option<(Pat, Vec<Stmt>)>,
        /// Finally body.
        finally: Option<Vec<Stmt>>,
    },
    /// `switch (d) { case t: … default: … }`
    Switch {
        /// Discriminant.
        disc: Expr,
        /// Clauses in source order (`test` = `None` is `default`).
        cases: Vec<SwitchCase>,
    },
    /// `import …` — module only. Liaisons installées à
    /// l'instanciation ; exécuter le statement est un no-op.
    Import {
        /// Bindings declared by the clause.
        binds: Vec<ImportBind>,
        /// Module specifier source string.
        source: String,
    },
    /// `export …` — module only.
    Export(ExportDecl),
}

/// One binding introduced by an `import` clause.
#[derive(Debug, Clone)]
pub enum ImportBind {
    /// `import def from 'm'` — liaise le `default` du module.
    Default(String),
    /// `import {a as b}` — `(exported name, local name)`.
    Named(String, String),
    /// `import * as ns from 'm'` — whole module namespace object.
    Namespace(String),
}

/// `export` forms.
#[derive(Debug, Clone)]
pub enum ExportDecl {
    /// `export {a as b, c}` — `(local, exported)`. `from` is set
    /// for `export {a} from 'm'` (re-export: no local binding).
    Named {
        /// `(local name, exported name)` pairs.
        list: Vec<(String, String)>,
        /// Re-export source specifier, if any.
        from: Option<String>,
    },
    /// `export var|let|const|function|class …` — the declaration
    /// executes normally; its declared names become exports.
    Decl(Box<Stmt>),
    /// `export default <expr|function|class>` — binds the hidden
    /// `default` export (plus the function/class own name if given).
    Default(Box<Expr>),
    /// `export * [as ns] from 'm'` — re-exports every named export
    /// of the dependency (except `default`).
    Star {
        /// `export * as ns` — the whole namespace re-exported as one name.
        ns: Option<String>,
        /// Module specifier.
        from: String,
    },
}

/// `name [= init]` inside a `var`/`let`/`const` statement — the
/// bound side is a pattern (`var {a} = o`).
#[derive(Debug, Clone)]
pub struct VarDecl {
    /// Binding pattern (a plain `Pat::Ident` in the common case).
    pub pat: Pat,
    /// Optional initialiser.
    pub init: Option<Expr>,
}

/// Binding pattern — declarations, parameters, `catch`, `for-in`/
/// `for-of` targets and destructuring assignments.
#[derive(Debug, Clone)]
pub enum Pat {
    /// `name` — binds in scope (`var`/`let`/`const`/param) or assigns
    /// (`[x] = …` destructuring without a declarator).
    Ident(String),
    /// Existing property target `o.p` / `a[i]` — only produced by
    /// destructuring *assignments*, never legal in declarations.
    Lvalue(Expr),
    /// `...pat` — collects the remaining elements/props.
    Rest(Box<Pat>),
    /// `pat = default` — default evaluated when the source is
    /// `undefined`.
    Default(Box<Pat>, Box<Expr>),
    /// `[a, , b, ...rest]` — `None` = hole (skipped).
    Array(Vec<Option<Pat>>),
    /// `{k: p, "k2": p2, [e]: p3, ...rest}`.
    Object {
        /// `key: subpattern` entries.
        props: Vec<(PropKey, Pat)>,
        /// Trailing `...rest` (collects unconsumed enumerable keys).
        rest: Option<Box<Pat>>,
    },
}

/// Object/class property key.
#[derive(Debug, Clone)]
pub enum PropKey {
    /// `name`, `"name"` or `42` (already stringified).
    Name(String),
    /// `[expr]` — evaluated at definition time.
    Computed(Expr),
}

/// `for` init clause.
#[derive(Debug, Clone)]
pub enum ForInit {
    /// `var …`
    Var(Vec<VarDecl>),
    /// `let`/`const …`
    Lexical {
        /// Declarations.
        decls: Vec<VarDecl>,
        /// `const`.
        constant: bool,
    },
    /// Plain expression.
    Expr(Expr),
}

/// Left side of `for-in`/`for-of`.
#[derive(Debug, Clone)]
pub enum ForTarget {
    /// `var pat` (possibly already declared).
    Var(Pat),
    /// `let`/`const pat`.
    Lexical {
        /// Binding pattern.
        pat: Pat,
        /// `const` target.
        constant: bool,
    },
    /// Existing lvalue or assignment pattern (`x`, `o.p`, `[a,b]`).
    Expr(Expr),
}

/// One `case`/`default` clause.
#[derive(Debug, Clone)]
pub struct SwitchCase {
    /// `None` for `default`.
    pub test: Option<Expr>,
    /// Statements until the next clause.
    pub body: Vec<Stmt>,
}

/// Function literal (declaration, expression or arrow).
#[allow(clippy::struct_excessive_bools)] // flags are flags.
#[derive(Debug, Clone)]
pub struct FnLit {
    /// Name (declarations always have one; expressions may not).
    pub name: Option<String>,
    /// Parameters (patterns + defaults + rest).
    pub params: Vec<Param>,
    /// Body statements.
    pub body: Vec<Stmt>,
    /// Arrow function — `this` is captured lexically, and a single
    /// expression body `x => e` is desugared to `{ return e }`.
    pub is_arrow: bool,
    /// `async` — the call wraps the body's return in a resolved
    /// `Promise` (and rejections into a rejected promise). `await`
    /// suspends only on already-settled promises (no continuations
    /// — a pending `await` raises a runtime error; documented
    /// deviation).
    pub is_async: bool,
    /// `function*` — appeler la fonction rend un itérateur dont le
    /// corps a tourné à l'appel en collectant les `yield`
    /// (générateur *eager* : pas de suspension — déviation
    /// documentée ; `next(v)` ignore `v`, les générateurs
    /// infinis butent sur le budget d'opérations).
    pub is_gen: bool,
    /// Corps en mode strict — hérité du contexte OU activé par un
    /// prologue `"use strict"` dans le corps. `true` pour tous les
    /// membres de classe (les corps de classe sont stricts par spec).
    /// Consommé par le runtime : `TypeError`/`ReferenceError` au lieu
    /// d'échecs silencieux (écriture non-writable, delete
    /// non-configurable, identifiant non déclaré).
    pub strict: bool,
}

/// One formal parameter.
#[derive(Debug, Clone)]
pub struct Param {
    /// Binding pattern.
    pub pat: Pat,
    /// `...rest` — collects the remaining arguments into an array.
    pub rest: bool,
}

/// Class literal/declaration.
#[derive(Debug, Clone)]
pub struct ClassLit {
    /// `class C` — `None` for anonymous class expressions.
    pub name: Option<String>,
    /// `extends e` clause.
    pub super_: Option<Expr>,
    /// Members in source order.
    pub members: Vec<ClassMember>,
}

/// One class member.
#[derive(Debug, Clone)]
pub struct ClassMember {
    /// Member name or computed key.
    pub key: PropKey,
    /// Role.
    pub kind: ClassMemberKind,
    /// `static` member (installed on the constructor object).
    pub static_: bool,
    /// Method body.
    pub fun: Rc<FnLit>,
}

/// Class member role.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClassMemberKind {
    /// `constructor(…)`.
    Ctor,
    /// Ordinary method.
    Method,
    /// `get k()`.
    Getter,
    /// `set k(v)`.
    Setter,
    /// `k = init` / `#k = init` — the `fun` body is a synthesized
    /// `return init` thunk (empty body → `undefined`).
    Field,
    /// `static { … }` block — `fun` carries the statements.
    StaticBlock,
}

/// Binary operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    /// `||`
    Or,
    /// `&&`
    And,
    /// `|`
    BitOr,
    /// `^`
    BitXor,
    /// `&`
    BitAnd,
    /// `==`
    Eq,
    /// `!=`
    Ne,
    /// `===`
    StrictEq,
    /// `!==`
    StrictNe,
    /// `<`
    Lt,
    /// `<=`
    Le,
    /// `>`
    Gt,
    /// `>=`
    Ge,
    /// `instanceof`
    Instanceof,
    /// `in`
    In,
    /// `<<`
    Shl,
    /// `>>`
    Shr,
    /// `>>>`
    ShrU,
    /// `+`
    Add,
    /// `-`
    Sub,
    /// `*`
    Mul,
    /// `/`
    Div,
    /// `%`
    Mod,
    /// `**` (right-associative).
    Pow,
    /// `??` — right side only when left is `null`/`undefined`.
    Nullish,
}

/// Unary / update operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp {
    /// `!`
    Not,
    /// `~`
    BitNot,
    /// unary `+`
    Pos,
    /// unary `-`
    Neg,
    /// `typeof`
    Typeof,
    /// `void`
    Void,
    /// `delete`
    Delete,
    /// prefix `++`
    PreInc,
    /// prefix `--`
    PreDec,
    /// postfix `++`
    PostInc,
    /// postfix `--`
    PostDec,
}

/// Compound assignment operators (`=` itself is [`Expr::Assign`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssignOp {
    /// `=`
    Set,
    /// `+=`
    Add,
    /// `-=`
    Sub,
    /// `*=`
    Mul,
    /// `/=`
    Div,
    /// `%=`
    Mod,
    /// `<<=`
    Shl,
    /// `>>=`
    Shr,
    /// `>>>=`
    ShrU,
    /// `&=`
    BitAnd,
    /// `|=`
    BitOr,
    /// `^=`
    BitXor,
    /// `**=`
    Pow,
    /// `??=`
    Nullish,
    /// `&&=`
    LogAnd,
    /// `||=`
    LogOr,
}

/// Expressions.
#[derive(Debug, Clone)]
pub enum Expr {
    /// Number literal.
    Num(f64),
    /// `123n` `BigInt` literal — digits normalisés en décimal par le
    /// lexer (`0xFn` déjà converti).
    BigInt(String),
    /// String literal.
    Str(String),
    /// `true` / `false`.
    Bool(bool),
    /// `null`.
    Null,
    /// Identifier reference.
    Ident(String),
    /// `this`.
    This,
    /// `[a, , b]` — `None` elements are holes (read as `undefined`).
    Array(Vec<Option<Expr>>),
    /// `{k: v, …}`.
    Object(Vec<ObjProp>),
    /// `function …` expression.
    Function(Rc<FnLit>),
    /// `class [C] [extends B] {…}` expression.
    Class(Rc<ClassLit>),
    /// Unary/update operation.
    Unary(UnOp, Box<Expr>),
    /// Binary operation.
    Binary(BinOp, Box<Expr>, Box<Expr>),
    /// `l = v` / `l op= v`.
    Assign(Box<Expr>, AssignOp, Box<Expr>),
    /// `c ? t : e`.
    Cond(Box<Expr>, Box<Expr>, Box<Expr>),
    /// `o.prop`.
    Member(Box<Expr>, String),
    /// `o[key]`.
    Index(Box<Expr>, Box<Expr>),
    /// `f(a, b)` — `this` is the base object for `o.m(…)` calls.
    Call(Box<Expr>, Vec<Expr>),
    /// `new F(a)`.
    New(Box<Expr>, Vec<Expr>),
    /// `a, b` comma sequence.
    Seq(Vec<Expr>),
    /// `` `t${e}…` `` — `parts.len() == exprs.len() + 1`.
    Template {
        /// Literal chunks (already unescaped).
        parts: Vec<String>,
        /// `${…}` substitutions.
        exprs: Vec<Expr>,
    },
    /// `/pat/flags` — compiled lazily by the interpreter.
    Regex {
        /// Pattern source.
        pattern: String,
        /// Flag letters (`gimy su`…).
        flags: String,
    },
    /// Member/call chain containing at least one `?.` step (a chain
    /// without `?.` stays `Member`/`Index`/`Call` — unchanged paths).
    Chain {
        /// Expression the steps apply to.
        head: Box<Expr>,
        /// Steps in source order.
        steps: Vec<ChainStep>,
    },
    /// `...e` — legal inside array literals and argument lists.
    Spread(Box<Expr>),
    /// `super(args)` — only inside a derived-class constructor.
    SuperCall(Vec<Expr>),
    /// `super.name` — resolved against the home object's prototype.
    SuperMember(String),
    /// `super[e]` — computed home-object lookup.
    SuperIndex(Box<Expr>),
    /// `await e` — unwraps an already-settled promise
    /// (fulfilled → value, rejected → throw). Await on a *pending*
    /// promise raises a runtime error: the tree-walking interpreter
    /// cannot suspend mid-function (documented deviation).
    Await(Box<Expr>),
    /// `yield [x]` / `yield* it` — pushes onto the generator's
    /// eager collector; the expression evaluates to `undefined`
    /// (the `next(v)` argument is unavailable — no suspension).
    Yield {
        /// Yielded expression (`None` → `undefined`).
        arg: Option<Box<Expr>>,
        /// `yield*` — drains the sub-iterable into the collector.
        delegate: bool,
    },
    /// `import(spec)` — dynamic import: returns a promise resolved
    /// with the module namespace object.
    ImportCall(Box<Expr>),
    /// `import.meta` — `{url: <module specifier>}`.
    ImportMeta,
    /// `new.target` — `undefined` en appel ordinaire, le
    /// constructeur cible sous `new`/`Reflect.construct`.
    NewTarget,
}

/// One member of an object literal.
#[derive(Debug, Clone)]
pub enum ObjProp {
    /// `key: v` — shorthand `{x}` and methods `{m(){}}` desugar to
    /// this (`x: x` / `key: function`).
    Prop(PropKey, Expr),
    /// `{...e}` — own enumerable props copied.
    Spread(Expr),
    /// `get k() {…}`.
    Getter(PropKey, Rc<FnLit>),
    /// `set k(v) {…}`.
    Setter(PropKey, Rc<FnLit>),
}

/// One step of an optional-chaining expression.
#[derive(Debug, Clone)]
pub enum ChainStep {
    /// `.name`
    Member(String),
    /// `[k]`
    Index(Expr),
    /// `(args)`
    Call(Vec<Expr>),
    /// `?.name`
    OptMember(String),
    /// `?.[k]`
    OptIndex(Expr),
    /// `?.(args)`
    OptCall(Vec<Expr>),
}
