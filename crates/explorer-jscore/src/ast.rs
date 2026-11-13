// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! AST for `explorer-jscore` — ES3 core plus pragmatic ES2015+ coverage.
//!
//! Covered beyond ES3: `let`/`const`, arrow functions, template
//! literals, optional chaining (`?.`) / nullish (`??`), `**`,
//! logical assignment (`&&=`/`||=`/`??=`). Still rejected up front:
//! `with`, labels/labelled break-continue, regex literals, classes,
//! generators, modules, destructuring, `for-of`.

use std::cell::OnceCell;
use std::collections::HashSet;
use std::rc::Rc;

use crate::lexer::Token;

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
#[derive(Debug)]
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
    /// `using x = e` / `await using x = e` (ES2026) — la ressource
    /// est dépilée en fin de corps via `@@dispose`/`@@asyncDispose`.
    Using {
        /// Déclarations (binding const — immutable post-init).
        decls: Vec<VarDecl>,
        /// `await using` — `@@asyncDispose` + await eager.
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
        name: Rc<str>,
        /// Labelled statement.
        body: Box<Stmt>,
    },
    /// `break [label]`
    Break(Option<Rc<str>>),
    /// `continue [label]` — `Some` restreint aux labels de boucle
    /// (early error du parseur).
    Continue(Option<Rc<str>>),
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

impl Clone for Stmt {
    /// Clone dont chaque arête `Box<Stmt>`/`Expr`/`Pat`
    /// rebascule sur un segment tas (JS-C400/§B-111 — le
    /// `#[derive(Clone)]` descendait nu : `lower_suspend_fn`
    /// clone le corps à chaque invocation, un AST profond
    /// débordait le segment courant, pile native indifférente).
    fn clone(&self) -> Self {
        stacker::maybe_grow(
            crate::parser::STACK_RED_ZONE,
            crate::parser::STACK_GROW,
            || self.clone_inner(),
        )
    }
}

impl Stmt {
    fn clone_inner(&self) -> Self {
        match self {
            Self::Empty => Self::Empty,
            Self::Expr(e) => Self::Expr(e.clone()),
            Self::Var(ds) => Self::Var(ds.clone()),
            Self::Lexical { decls, constant } => Self::Lexical {
                decls: decls.clone(),
                constant: *constant,
            },
            Self::FunctionDecl(l) => Self::FunctionDecl(l.clone()),
            Self::ClassDecl(l) => Self::ClassDecl(l.clone()),
            Self::Block(l) => Self::Block(l.clone()),
            Self::If { cond, then, else_ } => Self::If {
                cond: cond.clone(),
                then: then.clone(),
                else_: else_.clone(),
            },
            Self::While { cond, body } => Self::While {
                cond: cond.clone(),
                body: body.clone(),
            },
            Self::DoWhile { body, cond } => Self::DoWhile {
                body: body.clone(),
                cond: cond.clone(),
            },
            Self::For {
                init,
                test,
                update,
                body,
            } => Self::For {
                init: init.clone(),
                test: test.clone(),
                update: update.clone(),
                body: body.clone(),
            },
            Self::ForIn {
                target,
                obj,
                var_init,
                body,
            } => Self::ForIn {
                target: target.clone(),
                obj: obj.clone(),
                var_init: var_init.clone(),
                body: body.clone(),
            },
            Self::ForOf {
                target,
                iter,
                body,
                is_await,
            } => Self::ForOf {
                target: target.clone(),
                iter: iter.clone(),
                body: body.clone(),
                is_await: *is_await,
            },
            Self::Using { decls, is_await } => Self::Using {
                decls: decls.clone(),
                is_await: *is_await,
            },
            Self::With { obj, body } => Self::With {
                obj: obj.clone(),
                body: body.clone(),
            },
            Self::Labeled { name, body } => Self::Labeled {
                name: name.clone(),
                body: body.clone(),
            },
            Self::Break(l) => Self::Break(l.clone()),
            Self::Continue(l) => Self::Continue(l.clone()),
            Self::Return(e) => Self::Return(e.clone()),
            Self::Throw(e) => Self::Throw(e.clone()),
            Self::Try {
                block,
                catch,
                finally,
            } => Self::Try {
                block: block.clone(),
                catch: catch.clone(),
                finally: finally.clone(),
            },
            Self::Switch { disc, cases } => Self::Switch {
                disc: disc.clone(),
                cases: cases.clone(),
            },
            Self::Import { binds, source } => Self::Import {
                binds: binds.clone(),
                source: source.clone(),
            },
            Self::Export(d) => Self::Export(d.clone()),
        }
    }
}

/// One binding introduced by an `import` clause.
#[derive(Debug, Clone)]
pub enum ImportBind {
    /// `import def from 'm'` — liaise le `default` du module.
    Default(Rc<str>),
    /// `import {a as b}` — `(exported name, local name)`.
    Named(Rc<str>, Rc<str>),
    /// `import * as ns from 'm'` — whole module namespace object.
    Namespace(Rc<str>),
}

/// `export` forms.
#[derive(Debug, Clone)]
pub enum ExportDecl {
    /// `export {a as b, c}` — `(local, exported)`. `from` is set
    /// for `export {a} from 'm'` (re-export: no local binding).
    Named {
        /// `(local name, exported name)` pairs.
        list: Vec<(Rc<str>, Rc<str>)>,
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
        ns: Option<Rc<str>>,
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
#[derive(Debug)]
pub enum Pat {
    /// `name` — binds in scope (`var`/`let`/`const`/param) or assigns
    /// (`[x] = …` destructuring without a declarator).
    Ident(Rc<str>),
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

impl Clone for Pat {
    /// Clone dont chaque arête `Box<Pat>`/`Expr` rebascule sur
    /// un segment tas (JS-C400/§B-111 — le `#[derive(Clone)]`
    /// descendait nu : un motif profond débordait le segment
    /// courant, pile native indifférente).
    fn clone(&self) -> Self {
        stacker::maybe_grow(
            crate::parser::STACK_RED_ZONE,
            crate::parser::STACK_GROW,
            || self.clone_inner(),
        )
    }
}

impl Pat {
    fn clone_inner(&self) -> Self {
        match self {
            Self::Ident(n) => Self::Ident(n.clone()),
            Self::Lvalue(e) => Self::Lvalue(e.clone()),
            Self::Rest(p) => Self::Rest(p.clone()),
            Self::Default(p, e) => Self::Default(p.clone(), e.clone()),
            Self::Array(items) => Self::Array(items.clone()),
            Self::Object { props, rest } => Self::Object {
                props: props.clone(),
                rest: rest.clone(),
            },
        }
    }
}

/// Object/class property key.
#[derive(Debug, Clone)]
pub enum PropKey {
    /// `name`, `"name"` or `42` (already stringified).
    Name(Rc<str>),
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
    /// `for (using x of y)` (ES2026) — disposal par itération.
    Using {
        /// Binding pattern.
        pat: Pat,
        /// `for (await using x of y)` — `@@asyncDispose`.
        is_await: bool,
    },
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
    pub name: Option<Rc<str>>,
    /// Parameters (patterns + defaults + rest).
    pub params: Vec<Param>,
    /// Body statements — éager ou différé (V-5a).
    pub body: FnBody,
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
    /// Position 1-based `(ligne, colonne)` du jeton qui ouvre le
    /// littéral (JS-C47) — rendue dans les frames de `error.stack`.
    /// `(0, 0)` pour les fonctions synthétisées par le moteur
    /// (ctor de classe par défaut).
    pub pos: (usize, usize),
    /// Nom de la source qui a produit ce littéral — URL du module,
    /// `<eval>`/`Function`… rendue dans les frames `error.stack`
    /// (`at f (src:l:c)`) pour attribuer chaque frame à son bundle
    /// (triage de site réel — JS-C50/C56). `None` = source sans
    /// nom ou fonction synthétique.
    pub src: Option<Rc<str>>,
    /// Texte source verbatim de l'unité qui a produit ce littéral
    /// (JS-C264) — `Function.prototype.toString` le tranche avec
    /// `span` pour rendre le code tel qu'écrit (spec : « source
    /// text », V8 verbatim). `None` = fonction synthétisée par le
    /// moteur ou fonction issue d'une substitution de template
    /// (offsets relatifs au segment, non tranchables) → repli
    /// `function name(…) { [code] }`.
    pub src_text: Option<Rc<str>>,
    /// `(début, fin)` en **caractères** du littéral dans `src_text`
    /// — mot-clé `function`/`async`/`class`… inclus, `}` final
    /// inclus. `(0, 0)` sans `src_text`.
    pub span: (usize, usize),
}

/// Corps de fonction (V-5a) : éager ou différé.
#[derive(Debug, Clone)]
pub enum FnBody {
    /// Corps parsé à la construction — partsagé `Rc` pour que le
    /// chemin d'appel récupère une référence sans copie.
    Parsed(Rc<Vec<Stmt>>),
    /// Corps différé : la plage de tokens `{…}` est conservée et
    /// sous-parsée au **premier appel** (`close_table` a rendu le
    /// saut O(1) au parse — les bundles webpack/enregistreurs ne
    /// paient plus la descente récursive des corps jamais
    /// appelés). Les fonctions imbriquées sont différées
    /// récursivement par le sous-parse.
    Deferred(Rc<DeferredBody>),
}

/// État d'un corps différé — la matérialisation est unique
/// (`OnceCell`) et découverte par le même `fn_body` que le chemin
/// éager (prologue de directives inclus).
#[derive(Debug, Clone)]
pub struct DeferredBody {
    /// Flux de tokens du **script parent**, partagé (V-5b) —
    /// `[start..=end]` délimite le `{…}` du corps. Le defer ne
    /// copie plus la tranche : un bump de refcount par corps.
    pub toks: Rc<[Token]>,
    /// Index global du `{` d'ouverture dans `toks`.
    pub start: usize,
    /// Index global du `}` fermant (inclus) dans `toks`.
    pub end: usize,
    /// Table des délimiteurs appariés du flux parent, partagée
    /// (V-5b) — `close_of` n'est plus reconstruite par corps :
    /// ses entrées sont des index globaux, valides telles
    /// quelles puisque `toks`/`pos` restent dans le repère
    /// parent.
    pub close: Rc<Vec<usize>>,
    /// Mode strict **hérité** au point du defer — le sous-parse
    /// repart de là puis `scan_directives` tranche le strict
    /// effectif (qui a déjà été fixé sur `FnLit.strict` par le
    /// scan anticipé du parseur — coercage `this` au premier
    /// appel).
    pub strict: bool,
    /// Contexte module du parse d'origine.
    pub module: bool,
    /// Profondeur de nesting accumulée au defer — le sous-parse
    /// l'emboîte sur la même borne (pattern `template_parts`).
    pub nest: u32,
    /// Borne de nesting héritée.
    pub nest_max: u32,
    /// Compteur d'identités de noms privés au defer — la
    /// résolution `#x` du corps voit les portées de classes
    /// englobantes (clonées, identité préservée).
    pub priv_seq: u32,
    /// Portées de noms privés `(identité, déclarations)` au defer.
    pub priv_scopes: Vec<(u32, HashSet<String>)>,
    /// Nom de source pour les erreurs du sous-parse.
    pub src_name: Option<Rc<str>>,
    /// Texte source verbatim — les `FnLit` produits par le
    /// sous-parse tranchent leurs spans dedans (offsets relatifs
    /// à la source complète : les tokens gardent leurs `off`).
    pub src_text: Option<Rc<str>>,
    /// `(stmts, strict effectif)` matérialisés au premier appel.
    pub parsed: OnceCell<(Rc<Vec<Stmt>>, bool)>,
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
    pub name: Option<Rc<str>>,
    /// `extends e` clause.
    pub super_: Option<Expr>,
    /// Members in source order.
    pub members: Vec<ClassMember>,
    /// Texte source verbatim + span du littéral `class …` entier
    /// (JS-C264) — `C.toString()` rend `class C {…}` tel qu'écrit
    /// (V8 : le constructeur expose le source de la classe, pas le
    /// seul corps du ctor). Recopié sur le `FnLit` du ctor dans
    /// `eval_class`. `None`/`(0,0)` hors source directe.
    pub src_text: Option<Rc<str>>,
    /// `(début, fin)` en caractères — du mot-clé `class` au `}`
    /// final inclus.
    pub span: (usize, usize),
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
#[derive(Debug)]
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
    Ident(Rc<str>),
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
    Member(Box<Expr>, Rc<str>),
    /// `o[key]`.
    Index(Box<Expr>, Box<Expr>),
    /// `f(a, b)` — `this` is the base object for `o.m(…)` calls.
    /// Le troisième champ porte la position 1-based `(ligne,
    /// colonne)` du `(` — site d'appel rendu dans les frames
    /// `error.stack` (N1.1) ; `(0, 0)` pour les nœuds
    /// synthétisés par le moteur.
    Call(Box<Expr>, Vec<Expr>, (usize, usize)),
    /// `new F(a)` — même convention d'appel-site que `Call`
    /// (position du mot-clé `new`).
    New(Box<Expr>, Vec<Expr>, (usize, usize)),
    /// `a, b` comma sequence.
    Seq(Vec<Expr>),
    /// `` `t${e}…` `` — `parts.len() == exprs.len() + 1`.
    Template {
        /// Literal chunks (already unescaped).
        parts: Vec<String>,
        /// `${…}` substitutions.
        exprs: Vec<Expr>,
    },
    /// `` tag`t${e}…` `` — tagged template call
    /// `tag(strings, …exprs)` with `strings.raw` = raw chunks.
    /// `this` is kept when `tag` is `o.m` (like `o.m(…)`).
    TaggedTemplate {
        /// Tag expression.
        tag: Box<Expr>,
        /// Cooked chunks — `parts.len() == exprs.len() + 1`.
        parts: Vec<String>,
        /// Raw chunks (`strings.raw`), same length as `parts`.
        raws: Vec<String>,
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
    SuperMember(Rc<str>),
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

impl Clone for Expr {
    /// Clone dont chaque arête `Box<Expr>` rebascule sur un
    /// segment tas (JS-C400/§B-111 — le `#[derive(Clone)]`
    /// descendait nu : `lower_suspend_fn`/`expr_to_pat` clonent
    /// l'AST à chaque invocation, un arbre profond débordait
    /// le segment courant, pile native indifférente).
    fn clone(&self) -> Self {
        stacker::maybe_grow(
            crate::parser::STACK_RED_ZONE,
            crate::parser::STACK_GROW,
            || self.clone_inner(),
        )
    }
}

impl Expr {
    fn clone_inner(&self) -> Self {
        match self {
            Self::Num(n) => Self::Num(*n),
            Self::BigInt(s) => Self::BigInt(s.clone()),
            Self::Str(s) => Self::Str(s.clone()),
            Self::Bool(b) => Self::Bool(*b),
            Self::Null => Self::Null,
            Self::Ident(n) => Self::Ident(n.clone()),
            Self::This => Self::This,
            Self::Array(items) => Self::Array(items.clone()),
            Self::Object(props) => Self::Object(props.clone()),
            Self::Function(l) => Self::Function(l.clone()),
            Self::Class(l) => Self::Class(l.clone()),
            Self::Unary(op, e) => Self::Unary(*op, e.clone()),
            Self::Binary(op, l, r) => Self::Binary(*op, l.clone(), r.clone()),
            Self::Assign(l, op, r) => Self::Assign(l.clone(), *op, r.clone()),
            Self::Cond(c, t, e) => Self::Cond(c.clone(), t.clone(), e.clone()),
            Self::Member(o, p) => Self::Member(o.clone(), p.clone()),
            Self::Index(o, k) => Self::Index(o.clone(), k.clone()),
            Self::Call(f, args, p) => Self::Call(f.clone(), args.clone(), *p),
            Self::New(f, args, p) => Self::New(f.clone(), args.clone(), *p),
            Self::Seq(items) => Self::Seq(items.clone()),
            Self::Template { parts, exprs } => Self::Template {
                parts: parts.clone(),
                exprs: exprs.clone(),
            },
            Self::TaggedTemplate {
                tag,
                parts,
                raws,
                exprs,
            } => Self::TaggedTemplate {
                tag: tag.clone(),
                parts: parts.clone(),
                raws: raws.clone(),
                exprs: exprs.clone(),
            },
            Self::Regex { pattern, flags } => Self::Regex {
                pattern: pattern.clone(),
                flags: flags.clone(),
            },
            Self::Chain { head, steps } => Self::Chain {
                head: head.clone(),
                steps: steps.clone(),
            },
            Self::Spread(e) => Self::Spread(e.clone()),
            Self::SuperCall(args) => Self::SuperCall(args.clone()),
            Self::SuperMember(n) => Self::SuperMember(n.clone()),
            Self::SuperIndex(e) => Self::SuperIndex(e.clone()),
            Self::Await(e) => Self::Await(e.clone()),
            Self::Yield { arg, delegate } => Self::Yield {
                arg: arg.clone(),
                delegate: *delegate,
            },
            Self::ImportCall(e) => Self::ImportCall(e.clone()),
            Self::ImportMeta => Self::ImportMeta,
            Self::NewTarget => Self::NewTarget,
        }
    }
}

impl Expr {
    /// Déplace les sous-expressions possédées dans `out` en
    /// laissant des feuilles `Null` — utilisé par le `Drop`
    /// itératif (les chaînes `Binary`/`Call` de 15 K+ maillons
    /// produites par le parser itératif `1+1+…`/`a&&a&&…` ne
    /// doivent pas descendre en récursion à la destruction —
    /// résidu JS-C41).
    fn take_children(&mut self, out: &mut Vec<Expr>) {
        let take = |b: &mut Box<Expr>, out: &mut Vec<Expr>| {
            out.push(std::mem::replace(&mut **b, Expr::Null));
        };
        match self {
            Expr::Unary(_, e)
            | Expr::Member(e, _)
            | Expr::Spread(e)
            | Expr::Await(e)
            | Expr::ImportCall(e)
            | Expr::Yield { arg: Some(e), .. }
            | Expr::SuperIndex(e) => take(e, out),
            Expr::Binary(_, l, r) | Expr::Assign(l, _, r) | Expr::Index(l, r) => {
                take(l, out);
                take(r, out);
            }
            Expr::Cond(c, t, e) => {
                take(c, out);
                take(t, out);
                take(e, out);
            }
            Expr::Call(f, args, _) | Expr::New(f, args, _) => {
                take(f, out);
                out.append(args);
            }
            Expr::Array(items) => out.extend(items.drain(..).flatten()),
            Expr::Object(props) => {
                for p in props.drain(..) {
                    match p {
                        ObjProp::Prop(_, e) | ObjProp::Spread(e) => out.push(e),
                        ObjProp::Getter(..) | ObjProp::Setter(..) => {}
                    }
                }
            }
            Expr::Seq(items) | Expr::SuperCall(items) => out.append(items),
            Expr::Template { exprs, .. } => out.append(exprs),
            Expr::TaggedTemplate { tag, exprs, .. } => {
                take(tag, out);
                out.append(exprs);
            }
            Expr::Chain { head, steps } => {
                take(head, out);
                for s in steps.drain(..) {
                    match s {
                        ChainStep::Index(e) | ChainStep::OptIndex(e) => out.push(e),
                        ChainStep::Call(args) | ChainStep::OptCall(args) => out.extend(args),
                        ChainStep::Member(_) | ChainStep::OptMember(_) => {}
                    }
                }
            }
            _ => {}
        }
    }
}

impl Drop for Expr {
    /// Destruction itérative : les enfants `Box`/`Vec`
    /// transitent par une file explicite — une AST profonde
    /// (opérandes chaînées ~15 K+) ne déborde plus la pile.
    fn drop(&mut self) {
        let mut queue = Vec::new();
        self.take_children(&mut queue);
        while let Some(mut e) = queue.pop() {
            e.take_children(&mut queue);
        }
    }
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
    Member(Rc<str>),
    /// `[k]`
    Index(Expr),
    /// `(args)`
    Call(Vec<Expr>),
    /// `?.name`
    OptMember(Rc<str>),
    /// `?.[k]`
    OptIndex(Expr),
    /// `?.(args)`
    OptCall(Vec<Expr>),
}

impl Stmt {
    /// Déplace les sous-statements possédés dans `out` en
    /// laissant `Empty` — Drop itératif (JS-C121) : corps de
    /// fonctions imbriquées (IIFEs/bundles générés) sans
    /// récursion native à la destruction.
    fn take_children(&mut self, out: &mut Vec<Stmt>) {
        let take = |b: &mut Box<Stmt>, out: &mut Vec<Stmt>| {
            out.push(std::mem::replace(&mut **b, Stmt::Empty));
        };
        match self {
            Stmt::Block(items) => out.append(items),
            Stmt::If { then, else_, .. } => {
                take(then, out);
                if let Some(e) = else_ {
                    take(e, out);
                }
            }
            Stmt::While { body, .. }
            | Stmt::DoWhile { body, .. }
            | Stmt::With { body, .. }
            | Stmt::Labeled { body, .. }
            | Stmt::For { body, .. }
            | Stmt::ForIn { body, .. }
            | Stmt::ForOf { body, .. } => take(body, out),
            Stmt::Try {
                block,
                catch,
                finally,
            } => {
                out.append(block);
                if let Some((_, b)) = catch {
                    out.append(b);
                }
                if let Some(b) = finally {
                    out.append(b);
                }
            }
            Stmt::Switch { cases, .. } => {
                for c in cases.iter_mut() {
                    out.append(&mut c.body);
                }
            }
            Stmt::Export(ExportDecl::Decl(inner)) => take(inner, out),
            _ => {}
        }
    }
}

impl Drop for Stmt {
    /// Destruction itérative — même pattern que `Expr::drop` :
    /// les corps `Block`/`If`/`Try`/fonctions imbriquées des
    /// bundles ne débordent plus la pile (mur alpine.js).
    fn drop(&mut self) {
        let mut queue = Vec::new();
        self.take_children(&mut queue);
        while let Some(mut s) = queue.pop() {
            s.take_children(&mut queue);
        }
    }
}

impl Drop for FnLit {
    /// Corps de fonction vidé dans la file `Stmt` — les
    /// fonctions imbriquées (closures/webpack) droppent sans
    /// récursion (JS-C121).
    fn drop(&mut self) {
        let mut queue = Vec::new();
        match &mut self.body {
            FnBody::Parsed(b) => {
                if let Some(b) = Rc::get_mut(b) {
                    queue.append(b);
                }
            }
            FnBody::Deferred(d) => {
                // Corps matérialisé dont le `DeferredBody` et le
                // `Rc<Vec<Stmt>>` sont les derniers détenteurs —
                // même drop itératif (un dernier référent qui
                // vit ailleurs droppe récursivement, borne par
                // `STACK` — rare, accepté).
                if let Some(d) = Rc::get_mut(d) {
                    if let Some((b, _)) = d.parsed.take() {
                        if let Ok(mut v) = Rc::try_unwrap(b) {
                            queue.append(&mut v);
                        }
                    }
                }
            }
        }
        while let Some(mut s) = queue.pop() {
            s.take_children(&mut queue);
        }
    }
}

impl Pat {
    /// Sous-patterns possédés dans `out` — drop itératif
    /// (destructurings imbriqués profonds, JS-C121).
    fn take_children(&mut self, out: &mut Vec<Pat>) {
        match self {
            Pat::Rest(p) | Pat::Default(p, _) => {
                out.push(std::mem::replace(&mut **p, Pat::Ident(Rc::from(""))));
            }
            Pat::Array(elems) => {
                for e in elems.drain(..).flatten() {
                    out.push(e);
                }
            }
            Pat::Object { props, rest } => {
                for (_, p) in props.drain(..) {
                    out.push(p);
                }
                if let Some(p) = rest.take() {
                    out.push(*p);
                }
            }
            _ => {}
        }
    }
}

impl Drop for Pat {
    /// Destruction itérative des patterns imbriqués (JS-C121).
    fn drop(&mut self) {
        let mut queue = Vec::new();
        self.take_children(&mut queue);
        while let Some(mut p) = queue.pop() {
            p.take_children(&mut queue);
        }
    }
}
