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
    AssignOp, BinOp, ChainStep, ClassLit, ClassMember, ClassMemberKind, DeferredBody, ExportDecl,
    Expr, FnBody, FnLit, ForInit, ForTarget, ImportBind, ObjProp, Param, Pat, Program, PropKey,
    Stmt, SwitchCase, UnOp, VarDecl,
};
use crate::error::{Error, Result};
use crate::lexer::{lex, lex_encoded, Kw, Tok, Token, TplPart, P};
use std::cell::OnceCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

/// Parse avec strict hérité — `eval` direct dans du code strict :
/// ses early errors s'appliquent aussi au texte évalué (spec
/// `PerformEval` — « strictCaller »). `nest_max` borne
/// l'emboîtement natif du parseur récursif (JS-C41).
/// Table O(n) des délimiteurs appariés : `close_of[i]` = index
/// du `)`/`]`/`}` fermant l'ouvrant à `i` (`usize::MAX` si
/// désapparié) — voir [`Parser::close_of`].
pub(crate) fn close_table(toks: &[Token]) -> Vec<usize> {
    let mut close = vec![usize::MAX; toks.len()];
    let mut stack: Vec<usize> = Vec::new();
    for (i, t) in toks.iter().enumerate() {
        match t.tok {
            Tok::P(P::LParen | P::LBracket | P::LBrace) => stack.push(i),
            Tok::P(P::RParen | P::RBracket | P::RBrace) => {
                if let Some(o) = stack.pop() {
                    close[o] = i;
                }
            }
            _ => {}
        }
    }
    close
}

/// Taille minimale (en tokens) d'un corps de fonction différé
/// (V-5a) — en deçà, le surcoût `DeferredBody` (slice + cellule +
/// scan) ne se justifie pas (`{ return x }` ≈ 5 tokens reste
/// éager). Au-delà, le saut `close_of` O(1) évite la descente
/// récursive — les corps jamais appelés ne sont jamais parsés.
/// Atomique `pub(crate)` — valeur fixe 24 en production ;
/// les tests différentiels la montent à `usize::MAX` pour
/// forcer le chemin éager et comparer les AST produits.
pub(crate) static DEFER_MIN_TOKS: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(24);

fn defer_min_toks() -> usize {
    DEFER_MIN_TOKS.load(std::sync::atomic::Ordering::Relaxed)
}

/// Scan lecture seule du prologue de directives d'un corps
/// différé — `true` si `"use strict"` en tête. Même règle de
/// terminaison que [`Parser::scan_directives`] (directive =
/// `Tok::Str` terminée par `;`, `}`, EOF ou ASI via
/// `newline_before`) : `FnLit.strict` doit être fixe dès le
/// parse — il pilote la coercition `this` au premier appel,
/// avant toute matérialisation.
fn deferred_use_strict(toks: &[Token]) -> bool {
    let mut i = 0;
    let mut strict = false;
    while let Some(t) = toks.get(i) {
        let Tok::Str(s) = &t.tok else { break };
        let ends = match toks.get(i + 1) {
            None => true,
            Some(n) => n.newline_before || matches!(n.tok, Tok::Eof | Tok::P(P::Semi | P::RBrace)),
        };
        if !ends {
            break;
        }
        if &**s == "use strict" {
            strict = true;
        }
        i += 1;
        if matches!(toks.get(i).map(|t| &t.tok), Some(Tok::P(P::Semi))) {
            i += 1;
        }
    }
    strict
}

/// Sous-parse d'un corps différé au premier appel (V-5a) — même
/// `fn_body` que le chemin éager (prologue de directives,
/// save/restore des flags). Les tokens `{…}` sont repris tels
/// quels du `Rc<[Token]>` ; `close_table` est reconstruite sur
/// le slice — O(slice) payé **une fois par corps réellement
/// appelé**. `pos = 1` : le `{` à l'indice 0 est pré-consommé.
/// Déviation documentée : les early errors que la stricte du
/// corps impose aux paramètres (doublons…) sont rapportées à
/// l'appel plutôt qu'au parse — tolérant, jamais de panic.
pub(crate) fn materialize_body(d: &DeferredBody, is_gen: bool) -> Result<(Vec<Stmt>, bool)> {
    // JS-C524 — sonde de taux d'usage des corps différés.
    crate::j17_tick(&crate::J17_DEFER_MAT);
    // V-8a+V-5b : flux `Rc<[Token]>` et `close_of` **partagés**
    // — ni clone de slice ni reconstruction de table ; `pos`
    // global pointe juste après le `{` (`d.start`), la borne
    // structurale `}` (`d.end`) arrête `fn_body` naturellement.
    debug_assert!(
        matches!(d.toks.get(d.start).map(|t| &t.tok), Some(Tok::P(P::LBrace)))
            && matches!(d.toks.get(d.end).map(|t| &t.tok), Some(Tok::P(P::RBrace)))
    );
    let mut sub = Parser {
        toks: d.toks.clone(),
        pos: d.start + 1,
        nest: d.nest,
        nest_max: d.nest_max,
        strict: d.strict,
        module: d.module,
        in_gen: is_gen,
        labels: Vec::new(),
        priv_seq: d.priv_seq,
        priv_scopes: d.priv_scopes.clone(),
        src_name: d.src_name.clone(),
        src_text: d.src_text.clone(),
        close_of: d.close.clone(),
        // Corps de fonction par construction — `return` légal,
        // `iter`/`sw` repartent à zéro comme les labels (la
        // frontière coupe les break/continue nus).
        fn_depth: 1,
        iter_depth: 0,
        sw_depth: 0,
        blk_depth: 1,
    };
    crate::j17_time(&crate::J17_STMT_NS, || sub.fn_body())
}

impl DeferredBody {
    /// Corps matérialisé — le parse ne court qu'une fois
    /// (`OnceCell`) ; `is_gen` vient du `FnLit` porteur (le flag
    /// vit sur le littéral, pas dans le corps). Une erreur de
    /// syntaxe tardive n'est pas cachée — elle re-jette à
    /// chaque appel (le throw est construit par l'appelant).
    pub(crate) fn get_or_parse(&self, is_gen: bool) -> Result<Rc<Vec<Stmt>>> {
        if let Some((b, _)) = self.parsed.get() {
            return Ok(b.clone());
        }
        crate::j17_tick(&crate::J17_FN_MAT);
        let (stmts, strict) = materialize_body(self, is_gen)?;
        let rc = Rc::new(stmts);
        let _ = self.parsed.set((rc.clone(), strict));
        Ok(rc)
    }
}

pub(crate) fn parse_in(
    src: &str,
    strict: bool,
    nest_max: u32,
    src_name: Option<Rc<str>>,
) -> Result<Program> {
    parse_in_enc(src, strict, nest_max, src_name, false)
}

/// `parse_in` sur source **pré-encodée domaine sentinelle**
/// (`re_encode` d'un `JsStr` à units — `eval` d'une chaîne
/// porteuse de surrogates isolés, test262 `literals/regexp`
/// A2) : le lexer pousse les chars raw verbatim au lieu de
/// re-échapper la zone sentinelle.
pub(crate) fn parse_in_enc(
    src: &str,
    strict: bool,
    nest_max: u32,
    src_name: Option<Rc<str>>,
    encoded: bool,
) -> Result<Program> {
    crate::j17_parse_probe(src, false);
    let toks = if encoded {
        crate::j17_time(&crate::J17_LEX_NS, || lex_encoded(src))?
    } else {
        crate::j17_time(&crate::J17_LEX_NS, || lex(src))?
    };
    crate::j17_add(
        &crate::J17_LEX_TOK,
        u64::try_from(toks.len()).unwrap_or(u64::MAX),
    );
    let close_of = crate::j17_time(&crate::J17_CLOSE_NS, || Rc::new(close_table(&toks)));
    let mut p = Parser {
        toks: Rc::from(toks),
        pos: 0,
        nest: 0,
        nest_max,
        strict,
        module: false,
        in_gen: false,
        labels: Vec::new(),
        priv_scopes: Vec::new(),
        priv_seq: 0,
        src_name,
        src_text: Some(Rc::from(src)),
        close_of,
        fn_depth: 0,
        iter_depth: 0,
        sw_depth: 0,
        blk_depth: 0,
    };
    let body = crate::j17_time(&crate::J17_STMT_NS, || -> Result<Vec<Stmt>> {
        p.scan_directives();
        let mut body = Vec::new();
        while !p.at_eof() {
            body.push(p.stmt()?);
        }
        Ok(body)
    })?;
    p.check_scope(&body, false)?;
    crate::j17_add(
        &crate::J17_AST_STMT,
        u64::try_from(body.len()).unwrap_or(u64::MAX),
    );
    Ok(Program {
        body,
        strict: p.strict,
    })
}

/// Parse a whole module — module goal: `import`/`export` statements
/// are allowed and the code is always strict (spec : un module ne
/// quitte jamais le mode strict).
pub fn parse_module(src: &str) -> Result<Program> {
    parse_module_in(src, DEFAULT_NEST_LIMIT, None)
}

/// Module goal avec borne d'emboîtement explicite (JS-C41).
/// `src_name` étiquette les `FnLit` produits — rendue dans les
/// frames `error.stack` (attribution par bundle, JS-C56).
pub(crate) fn parse_module_in(
    src: &str,
    nest_max: u32,
    src_name: Option<Rc<str>>,
) -> Result<Program> {
    crate::j17_parse_probe(src, true);
    let toks = crate::j17_time(&crate::J17_LEX_NS, || lex(src))?;
    crate::j17_add(
        &crate::J17_LEX_TOK,
        u64::try_from(toks.len()).unwrap_or(u64::MAX),
    );
    let close_of = crate::j17_time(&crate::J17_CLOSE_NS, || Rc::new(close_table(&toks)));
    let mut p = Parser {
        toks: Rc::from(toks),
        pos: 0,
        nest: 0,
        nest_max,
        strict: true,
        module: true,
        in_gen: false,
        labels: Vec::new(),
        priv_seq: 0,
        priv_scopes: Vec::new(),
        src_name,
        src_text: Some(Rc::from(src)),
        close_of,
        fn_depth: 0,
        iter_depth: 0,
        sw_depth: 0,
        blk_depth: 0,
    };
    let body = crate::j17_time(&crate::J17_STMT_NS, || -> Result<Vec<Stmt>> {
        p.scan_directives();
        let mut body = Vec::new();
        while !p.at_eof() {
            body.push(p.stmt()?);
        }
        Ok(body)
    })?;
    p.check_scope(&body, false)?;
    crate::j17_add(
        &crate::J17_AST_STMT,
        u64::try_from(body.len()).unwrap_or(u64::MAX),
    );
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
pub(crate) const DEFAULT_NEST_LIMIT: u32 = 4096;

/// Marge déclenchant la croissance de pile (`maybe_grow`) — en
/// dessous, la récursion bascule sur un segment tas plutôt que de
/// risquer le débordement natif (le coût par niveau varie de ~3 à
/// ~55 Kio selon le transporteur et le profil, JS-C41).
pub(crate) const STACK_RED_ZONE: usize = 256 * 1024;

/// Taille d'un segment de pile alloué sur tas par `maybe_grow`.
pub(crate) const STACK_GROW: usize = 2 * 1024 * 1024;

struct Parser {
    // V-8a : tranche partagée — le parseur ne mute jamais
    // `toks` ; `materialize_body` y passe son `Rc<[Token]>`
    // directement (plus de clone `to_vec` ~165 µs/corps).
    toks: Rc<[Token]>,
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
    labels: Vec<(Rc<str>, bool)>,
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
    /// Nom de la source en cours de parse — copié dans chaque
    /// `FnLit.src` pour les frames `error.stack` (JS-C56).
    src_name: Option<Rc<str>>,
    /// Texte source verbatim — copié dans chaque `FnLit.src_text` ;
    /// `span` (offsets caractères) le tranche pour `toString()`
    /// spec (JS-C264). `None` dans les sous-parseurs de template
    /// (`${…}` : offsets relatifs au segment, non tranchables).
    src_text: Option<Rc<str>>,
    /// `close_of[i]` = index du jeton qui ferme le délimiteur
    /// ouvrant à l'index `i` (`(`/`[`/`{`) — `usize::MAX` sinon.
    /// Calculée en une passe O(n) : la spéculation `(params) =>`
    /// vérifie `=>` après le `)` en O(1) au lieu de re-parser le
    /// sous-arbre — les chaînes `n=(n=(n=…` (strftime réel,
    /// base.js de dev.to) faisaient exploser le temps de parse
    /// exponentiellement, un niveau par parenthèse. `Rc` (V-5b) :
    /// les sous-parses de corps différés la partagent — repère
    /// `pos` global, aucune reconstruction ni rebase.
    close_of: Rc<Vec<usize>>,
    /// Profondeur fonction — `return` n'est légal qu'à
    /// l'intérieur (early error, JS-C611). Incrémentée dans
    /// `fn_body`/`arrow_body`/blocs statiques, restaurée ensuite
    /// (comme `labels` — les frontières fonction coupent tout).
    fn_depth: u32,
    /// Profondeur itération (`for`/`while`/`do` bodies) — le
    /// `continue` nu n'existe que dedans ; sauvegardée à chaque
    /// frontière fonction comme `labels`.
    iter_depth: u32,
    /// Profondeur `switch` — le `break` nu existe aussi dans un
    /// `case`/`default` (spec : `switch` est une cible breakable
    /// sans label).
    sw_depth: u32,
    /// Profondeur de portée lexicale — `stmt_list` (blocs, corps,
    /// blocs statiques) et les corps de `case` l'incrémentent.
    /// `using` est interdit au niveau script top-level (spec —
    /// seules les portées bloc/fonction/module l'acceptent).
    blk_depth: u32,
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

    /// Offset caractère du premier caractère du jeton courant —
    /// borne gauche de `FnLit::span` (toString verbatim, JS-C264).
    fn off_here(&self) -> usize {
        self.cur().off
    }

    /// Offset caractère juste après le dernier caractère du jeton
    /// **précédemment consommé** — borne droite de `FnLit::span`
    /// (le `}` final d'un corps, la fin d'une expression-flèche…).
    fn off_prev_end(&self) -> usize {
        self.toks[self.pos.saturating_sub(1)].end
    }

    fn next(&mut self) -> &Token {
        crate::j17_tick(&crate::J17_PAR_NEXT);
        let t = &self.toks[self.pos];
        if self.pos + 1 < self.toks.len() {
            self.pos += 1;
        }
        t
    }

    /// `peek().clone()` compté (V-6) — les variants `String`/
    /// `Vec` de `Tok` paient malloc+memcpy par clone ; la
    /// cascade de précédence multiplie ce coût par token.
    fn peek_clone(&self) -> Tok {
        crate::j17_tick(&crate::J17_TOK_CLONE);
        self.peek().clone()
    }

    /// `next().tok.clone()` compté (V-6) — même classe.
    fn next_clone(&mut self) -> Tok {
        crate::j17_tick(&crate::J17_TOK_CLONE);
        self.next().tok.clone()
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
    fn expect_prop_name(&mut self) -> Result<Rc<str>> {
        match self.peek_clone() {
            Tok::Ident(s) => {
                self.next();
                Ok(s)
            }
            Tok::Kw(kw) => {
                self.next();
                Ok(crate::intern(kw_str(kw)))
            }
            // `o.#x` — accès membre privé, résolu à la portée
            // de classe englobante.
            Tok::Private(n) => {
                self.next();
                self.resolve_priv(&n).map(|m| crate::intern(&m))
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
    ///
    /// Un `#x` n'est une *déclaration* qu'en position de membre —
    /// précédé de `{`/`;`/`}` (frontière de `ClassElement`), les
    /// modifieurs `static`/`async`/`*`/`get`/`set` étant sautés.
    /// `this.#x`, `o.#x`, `#x in o` sont des *références* : les
    /// compter comme déclarations masquait `AllPrivateNamesValid`
    /// (`class C extends class { x = this.#foo; } #foo;` passait —
    /// JS-C614).
    fn prescan_privates(&self) -> HashSet<String> {
        let mut out = HashSet::new();
        let mut depth = 1u32;
        for (i, t) in self.toks[self.pos..].iter().enumerate() {
            match &t.tok {
                Tok::P(P::LBrace) => depth += 1,
                Tok::P(P::RBrace) => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                Tok::Private(n) if depth == 1 => {
                    // `#x` n'est une *référence* (pas une
                    // déclaration) que si le token précédent
                    // continue une expression : `.`/`?.`
                    // (`o.#x`), opérateur, `(`/`[`/`,`, ou un
                    // mot-clé qui réclame un opérande
                    // (`extends`, `delete`, `return`…). Tout
                    // autre précédent — frontière `{`/`;`/`}`,
                    // fin d'expression `)`/`]`/ident/littéral/
                    // `++`, modifieur `static`/`get`/`async`… —
                    // ouvre un membre, ASI compris (`x\n#y`,
                    // `f\nstatic #y`).
                    let declares = match self.toks.get(self.pos + i - 1).map(|t| &t.tok) {
                        Some(Tok::P(p)) => matches!(
                            p,
                            // `*` n'appartient à cette liste que
                            // comme modifieur de générateur :
                            // `* #x` en profondeur de corps de
                            // classe ne peut être qu'une
                            // déclaration (`expr * #x` est fautif
                            // de toute façon).
                            P::Star
                                | P::RParen
                                | P::RBracket
                                | P::RBrace
                                | P::Semi
                                | P::LBrace
                                | P::Inc
                                | P::Dec
                        ),
                        Some(Tok::Kw(k)) => {
                            matches!(k, Kw::This | Kw::Super | Kw::True | Kw::False | Kw::Null)
                        }
                        _ => true,
                    };
                    if declares {
                        out.insert(n.to_string());
                    }
                }
                _ => {}
            }
        }
        out
    }

    /// `HasDirectSuper` sur la région `[i, end)` : `super (` est
    /// relevé sauf à l'intérieur d'un corps de `class` imbriquée
    /// (contexte `super` propre). Les corps `function` sont
    /// parcourus — un `super(` y est fautif quoi qu'il arrive,
    /// hors classes dérivées imbriquées (`skip_class_like`).
    fn scan_super_region(&self, mut i: usize, end: usize) -> bool {
        while i < end {
            match &self.toks[i].tok {
                Tok::Kw(Kw::Super)
                    if matches!(
                        self.toks.get(i + 1).map(|t| &t.tok),
                        Some(Tok::P(P::LParen))
                    ) =>
                {
                    return true;
                }
                Tok::Kw(Kw::Function) => {
                    let (next, found) = self.scan_fn_region(i, end);
                    if found {
                        return true;
                    }
                    i = next;
                    continue;
                }
                Tok::Kw(Kw::Class) => {
                    let (next, found) = self.skip_class_like(i, end);
                    if found {
                        return true;
                    }
                    i = next;
                    continue;
                }
                _ => {}
            }
            i += 1;
        }
        false
    }

    /// Parcourt `function …(params){body}` depuis `i` (jeton
    /// `function`) : tout `super(` y est fautif (une fonction
    /// n'a pas de super call propre) — rendu donc `found`.
    /// Rend `(index après `}`, super_trouvé)` — `end` borne.
    fn scan_fn_region(&self, i: usize, end: usize) -> (usize, bool) {
        let mut j = i + 1;
        while j < end && !matches!(self.toks[j].tok, Tok::P(P::LParen)) {
            j += 1;
        }
        let rp = self.close_of.get(j).copied().unwrap_or(end);
        let mut b = rp + 1;
        while b < end && !matches!(self.toks[b].tok, Tok::P(P::LBrace)) {
            b += 1;
        }
        let rb = self.close_of.get(b).copied().unwrap_or(end).min(end);
        let found = self.scan_super_region(i + 1, rb);
        ((rb + 1).min(end), found)
    }

    /// Saute `class …{…}` depuis `i` (jeton `class`) : l'héritage
    /// est parcouru — un `super(` y lie le ctor englobant ; le
    /// corps possède son propre contexte `super` → sauté via
    /// `close_of`. Rend `(index après `}`, super_en_héritage)`.
    fn skip_class_like(&self, i: usize, end: usize) -> (usize, bool) {
        let mut j = i + 1;
        let mut heritage = false;
        while j < end {
            match &self.toks[j].tok {
                // `{` de corps : après un token qui peut fermer
                // l'héritage, ou avant tout élément (`class {}`).
                Tok::P(P::LBrace) if !heritage || self.expr_end_before(j) => {
                    let close = self.close_of.get(j).copied().unwrap_or(end);
                    return ((close + 1).min(end), false);
                }
                Tok::P(P::LBrace) => {
                    // `{` littéral dans l'héritage — contenu
                    // scanné (`super(` lie le ctor englobant).
                    let close = self.close_of.get(j).copied().unwrap_or(end);
                    if self.scan_super_region(j + 1, close.min(end)) {
                        return (j, true);
                    }
                    j = close;
                }
                Tok::Kw(Kw::Super)
                    if matches!(
                        self.toks.get(j + 1).map(|t| &t.tok),
                        Some(Tok::P(P::LParen))
                    ) =>
                {
                    return (j, true);
                }
                Tok::Kw(Kw::Function) => {
                    let (next, found) = self.scan_fn_region(j, end);
                    if found {
                        return (j, true);
                    }
                    j = next;
                    continue;
                }
                Tok::Kw(Kw::Class) => {
                    let (next, found) = self.skip_class_like(j, end);
                    if found {
                        return (j, true);
                    }
                    j = next;
                    continue;
                }
                _ => {}
            }
            heritage = true;
            j += 1;
        }
        (j.min(end), false)
    }

    /// Le token à `j-1` peut-il terminer une expression — le `{`
    /// qui suit est alors un corps de classe, sinon un objet
    /// littéral de l'héritage (`extends {a:1}`).
    fn expr_end_before(&self, j: usize) -> bool {
        matches!(
            self.toks.get(j.wrapping_sub(1)).map(|t| &t.tok),
            Some(
                Tok::P(P::RBrace | P::RParen | P::RBracket)
                    | Tok::Ident(_)
                    | Tok::Num(_)
                    | Tok::Str(_)
                    | Tok::Tpl(_)
                    | Tok::Regex { .. }
                    | Tok::Private(_)
                    | Tok::Kw(Kw::This | Kw::Super | Kw::True | Kw::False | Kw::Null | Kw::Class)
            )
        )
    }

    /// Early errors déclaratifs du corps de classe complet
    /// (spec `ClassElement`/`ClassBody`) : `constructor` unique,
    /// noms privés uniques — la paire `get`/`set` seule est
    /// licite, statique et instance partagent le namespace —
    /// `#constructor`/`prototype` statique/`constructor` hors
    /// `ConstructorMethod` interdits (les clés calculées ne sont
    /// pas des `PropName` → exemptées).
    fn check_class_members(&self, members: &[ClassMember], seq: u32) -> Result<()> {
        let mut ctor = false;
        // nom privé → bits vus : 1=get, 2=set, 4=autre.
        let mut privs: HashMap<Rc<str>, u8> = HashMap::new();
        for m in members {
            let PropKey::Name(n) = &m.key else { continue };
            if n.starts_with('\u{f131}') {
                if n.as_ref() == Self::priv_key(seq, "constructor") {
                    return Err(self.err("`#constructor` is not a valid private name"));
                }
                let bit = match m.kind {
                    ClassMemberKind::Getter => 1u8,
                    ClassMemberKind::Setter => 2,
                    _ => 4,
                };
                let seen = privs.get(n).copied().unwrap_or(0);
                let both = seen | bit;
                // Doublon dès qu'un second membre sort du couple
                // {get,set} : get+set seule combinaison licite.
                if seen != 0 && both != 3 {
                    let pname = n.rsplit('.').next().unwrap_or(n);
                    return Err(self.err(format!("duplicate private name `#{pname}`")));
                }
                privs.insert(n.clone(), both);
                continue;
            }
            if m.static_ && &**n == "prototype" {
                return Err(self.err("`static prototype` is not a valid class element name"));
            }
            if &**n == "constructor" {
                // `constructor` n'est licite que comme
                // `ConstructorMethod` (simple, instance) — ou
                // n'importe quelle `MethodDefinition` *statique*
                // (spec : les early errors `ClassElement` ne
                // visent `constructor` que hors `static` ; un
                // `static get constructor(){}` est valide).
                // `FieldDefinition` le bannit même en statique.
                let legal = m.kind == ClassMemberKind::Ctor
                    || (m.static_ && m.kind != ClassMemberKind::Field);
                if !legal {
                    return Err(self.err("`constructor` is not a valid name here"));
                }
            }
            if m.kind == ClassMemberKind::Ctor {
                if ctor {
                    return Err(self.err("duplicate constructor"));
                }
                ctor = true;
            }
        }
        Ok(())
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
    fn label_arg(&mut self) -> Result<Option<Rc<str>>> {
        if self.cur().newline_before {
            return Ok(None);
        }
        match self.peek_clone() {
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
        let Tok::Ident(name) = self.peek_clone() else {
            return Err(self.err("expected label name"));
        };
        // Noms réservés comme label : strict (`yield`, `eval`…)
        // et `await` en module (early errors spec).
        self.check_strict_binding(name.as_ref())?;
        if self.module && &*name == "await" {
            return Err(self.err("`await` is reserved as a label in modules"));
        }
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
        let body = self.stmt_body(true, true)?;
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
        match self.peek_clone() {
            Tok::P(P::Semi) => {
                self.next();
                Ok(Stmt::Empty)
            }
            Tok::P(P::LBrace) => {
                self.next();
                let ss = self.stmt_list()?;
                self.check_scope(&ss, true)?;
                Ok(Stmt::Block(ss))
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
                // `const` exige un initialiser (`for-in/of` passent
                // par une autre branche — ici c'est toujours requis).
                if decls.iter().any(|d| d.init.is_none()) {
                    return Err(self.err("`const` declarations must be initialized"));
                }
                for d in &decls {
                    self.check_no_let_bound(&d.pat)?;
                }
                self.expect_semi()?;
                Ok(Stmt::Lexical {
                    decls,
                    constant: true,
                })
            }
            Tok::Kw(Kw::Let) => {
                // `let` is contextual: a decl only before ident/`[`/`{`
                // (destructuring). **Newline** : `let\n{`/`let\nx`
                // sont un identifiant suivi d'ASI — mais le
                // lookahead `let [` bannit l'ExpressionStatement
                // sans exception de ligne (`let\n[a]` reste une
                // déclaration destructurante), et les mots-clés
                // contextuels `let`/`await`/`yield` restent des
                // candidats de binding après newline
                // (`let\nawait 0` → `let await` + `0` inattendu).
                let next = self.toks.get(self.pos + 1);
                let decl = match next.map(|t| &t.tok) {
                    Some(Tok::P(P::LBracket) | Tok::Kw(Kw::Let)) => true,
                    Some(Tok::Ident(s)) if matches!(&**s, "let" | "await" | "yield") => true,
                    Some(Tok::Ident(_) | Tok::P(P::LBrace)) => {
                        !next.is_some_and(|t| t.newline_before)
                    }
                    _ => false,
                };
                if !decl {
                    let e = self.expr(false)?;
                    self.expect_semi()?;
                    return Ok(Stmt::Expr(e));
                }
                self.next();
                let decls = self.var_decls(false)?;
                for d in &decls {
                    self.check_no_let_bound(&d.pat)?;
                }
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
                if &*s == "async"
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
                if &*s == "using"
                    && matches!(
                        self.toks.get(self.pos + 1).map(|t| &t.tok),
                        Some(Tok::Ident(_) | Tok::P(P::LBracket | P::LBrace))
                    )
                    && !self
                        .toks
                        .get(self.pos + 1)
                        .is_some_and(|t| t.newline_before) =>
            {
                self.next(); // `using`
                let decls = self.var_decls(false)?;
                self.check_using_decls(&decls, false)?;
                self.expect_semi()?;
                Ok(Stmt::Using {
                    decls,
                    is_await: false,
                })
            }
            // `await using x = e` — `@@asyncDispose`.
            Tok::Ident(s)
                if &*s == "await"
                    && matches!(
                        self.toks.get(self.pos + 1).map(|t| &t.tok),
                        Some(Tok::Ident(u)) if &**u == "using"
                    )
                    && !self
                        .toks
                        .get(self.pos + 1)
                        .is_some_and(|t| t.newline_before)
                    && matches!(
                        self.toks.get(self.pos + 2).map(|t| &t.tok),
                        Some(Tok::Ident(_) | Tok::P(P::LBracket | P::LBrace))
                    ) =>
            {
                self.next(); // `await`
                self.next(); // `using`
                let decls = self.var_decls(false)?;
                self.check_using_decls(&decls, true)?;
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
                let body = Box::new(self.stmt_body(false, false)?);
                Ok(Stmt::With { obj, body })
            }
            Tok::Kw(Kw::While) => {
                self.next();
                self.expect_p(P::LParen, "`(`")?;
                let cond = self.expr(false)?;
                self.expect_p(P::RParen, "`)`")?;
                self.iter_depth += 1;
                let body = Box::new(self.stmt_body(false, false)?);
                self.iter_depth -= 1;
                Ok(Stmt::While { cond, body })
            }
            Tok::Kw(Kw::Do) => {
                self.next();
                self.iter_depth += 1;
                let body = Box::new(self.stmt_body(false, false)?);
                self.iter_depth -= 1;
                self.expect_kw(Kw::While, "`while`")?;
                self.expect_p(P::LParen, "`(`")?;
                let cond = self.expr(false)?;
                self.expect_p(P::RParen, "`)`")?;
                self.eat_p(P::Semi);
                Ok(Stmt::DoWhile { body, cond })
            }
            Tok::Kw(Kw::For) => self.for_stmt(),
            Tok::Kw(Kw::Return) => {
                // Early error spec : `return` n'existe que dans un
                // corps de fonction (blocs statiques inclus dans le
                // compte via `fn_depth` — ils n'y restent pas).
                if self.fn_depth == 0 {
                    return Err(self.err("`return` outside of function"));
                }
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
                } else if self.iter_depth == 0 && self.sw_depth == 0 {
                    // `break` nu : boucle ou `switch` seulement.
                    return Err(self.err("`break` outside of iteration or switch"));
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
                } else if self.iter_depth == 0 {
                    return Err(self.err("`continue` outside of iteration"));
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
            // `debugger` — no-op spec (J5 : un module en
            // contenant mourrait au parse — w3schools).
            Tok::Kw(Kw::Unsupported("debugger")) => {
                self.pos += 1;
                self.expect_semi()?;
                Ok(Stmt::Empty)
            }
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
        if let Tok::Str(src) = self.peek_clone() {
            self.next();
            self.expect_semi()?;
            return Ok(Stmt::Import {
                binds: Vec::new(),
                source: src.to_string(),
            });
        }
        let mut binds = Vec::new();
        match self.peek_clone() {
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
                    match self.peek_clone() {
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
        match self.peek_clone() {
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
                let e = match self.peek_clone() {
                    Tok::Kw(Kw::Function) => Expr::Function(self.function(false, false)?),
                    // `export default async function`
                    Tok::Ident(s)
                        if &*s == "async"
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
                if &*s == "async"
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
    fn export_specifiers(&mut self) -> Result<Vec<(Rc<str>, Rc<str>)>> {
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
    fn module_export_name(&mut self) -> Result<Rc<str>> {
        match self.peek_clone() {
            Tok::Ident(s) => {
                self.next();
                Ok(s)
            }
            Tok::Str(s) => {
                self.next();
                Ok(crate::intern(&s))
            }
            Tok::Kw(kw) => {
                self.next();
                Ok(crate::intern(kw_str(kw)))
            }
            _ => Err(self.err("expected export name")),
        }
    }

    /// Literal string argument (`'mod'` specifier).
    fn expect_str_tok(&mut self) -> Result<String> {
        match self.peek_clone() {
            Tok::Str(s) => {
                self.next();
                Ok(s.to_string())
            }
            _ => Err(self.err("expected module specifier string")),
        }
    }

    fn eat_ident(&mut self, word: &str) -> bool {
        if matches!(self.peek(), Tok::Ident(s) if &**s == word) {
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
        self.blk_depth += 1;
        let mut out = Vec::new();
        while !self.at_eof() && !self.check_p(P::RBrace) {
            out.push(self.stmt()?);
        }
        self.blk_depth -= 1;
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
            if &**s == "use strict" {
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

    // -- early errors de portée (JS-C611) ------------------------
    // `BoundNames` / `LexicallyDeclaredNames` / `VarDeclaredNames`
    // de la spec, matérialisés sur l'AST à la fermeture de chaque
    // portée. Seules les règles spec sont appliquées — le moteur
    // reste tolérant partout où la spec l'est.

    /// `BoundNames` d'un motif de liaison.
    fn bound_names_of(p: &Pat, out: &mut Vec<Rc<str>>) {
        match p {
            Pat::Ident(n) => out.push(n.clone()),
            Pat::Lvalue(_) => {}
            Pat::Rest(p) | Pat::Default(p, _) => Self::bound_names_of(p, out),
            Pat::Array(items) => {
                for it in items.iter().flatten() {
                    Self::bound_names_of(it, out);
                }
            }
            Pat::Object { props, rest } => {
                for (_, p) in props {
                    Self::bound_names_of(p, out);
                }
                if let Some(r) = rest {
                    Self::bound_names_of(r, out);
                }
            }
        }
    }

    /// Noms déclarés par `s` pour le scope vérifié —
    /// `at_level` = statement direct de la liste scopée (les
    /// déclarations des blocs internes ne sont pas lexicales à ce
    /// niveau, mais leurs `var`/functions annexe B remontent).
    /// `lex` = `(nom, est_plain_function)` ; `var` = noms var.
    fn decl_names(
        &self,
        s: &Stmt,
        at_level: bool,
        block_scope: bool,
        lex: &mut Vec<(Rc<str>, bool)>,
        var: &mut Vec<Rc<str>>,
    ) {
        let decls_to_lex = |decls: &[VarDecl], lex: &mut Vec<(Rc<str>, bool)>| {
            for d in decls {
                let mut n = Vec::new();
                Self::bound_names_of(&d.pat, &mut n);
                lex.extend(n.into_iter().map(|x| (x, false)));
            }
        };
        match s {
            Stmt::Var(ds) => {
                for d in ds {
                    Self::bound_names_of(&d.pat, var);
                }
            }
            Stmt::Lexical { decls, .. } | Stmt::Using { decls, .. } if at_level => {
                decls_to_lex(decls, lex);
            }
            Stmt::ClassDecl(c) if at_level => {
                if let Some(n) = &c.name {
                    lex.push((n.clone(), false));
                }
            }
            Stmt::FunctionDecl(f) => {
                let plain = !f.is_gen && !f.is_async;
                if !plain {
                    // `function*`/`async function` : toujours
                    // lexicales, jamais propagées (pas d'annexe B).
                    if at_level {
                        if let Some(n) = &f.name {
                            lex.push((n.clone(), false));
                        }
                    }
                } else if at_level {
                    if let Some(n) = &f.name {
                        // Bloc/CaseBlock/catch : `function` est
                        // lexicale ; top-level : var-scopée.
                        if block_scope {
                            lex.push((n.clone(), true));
                        } else {
                            var.push(n.clone());
                        }
                    }
                } else if !self.strict {
                    // Annexe B : les `function` nichées remontent au
                    // var-scope de la fonction englobante (sloppy).
                    if let Some(n) = &f.name {
                        var.push(n.clone());
                    }
                }
            }
            Stmt::Import { binds, .. } if at_level => {
                for b in binds {
                    let n = match b {
                        ImportBind::Default(n)
                        | ImportBind::Namespace(n)
                        | ImportBind::Named(_, n) => n,
                    };
                    lex.push((n.clone(), false));
                }
            }
            Stmt::Export(ExportDecl::Decl(inner)) => {
                self.decl_names(inner, at_level, block_scope, lex, var);
            }
            Stmt::Labeled { body, .. } => {
                self.decl_names(body, at_level, block_scope, lex, var);
            }
            Stmt::Block(ss) => {
                for s in ss {
                    self.decl_names(s, false, block_scope, lex, var);
                }
            }
            Stmt::If { then, else_, .. } => {
                self.decl_names(then, false, block_scope, lex, var);
                if let Some(e) = else_ {
                    self.decl_names(e, false, block_scope, lex, var);
                }
            }
            Stmt::While { body, .. } | Stmt::DoWhile { body, .. } | Stmt::With { body, .. } => {
                self.decl_names(body, false, block_scope, lex, var);
            }
            Stmt::For { init, body, .. } => {
                if let Some(ForInit::Var(ds)) = init {
                    for d in ds {
                        Self::bound_names_of(&d.pat, var);
                    }
                }
                self.decl_names(body, false, block_scope, lex, var);
            }
            Stmt::ForIn { target, body, .. } | Stmt::ForOf { target, body, .. } => {
                if let ForTarget::Var(p) = target {
                    Self::bound_names_of(p, var);
                }
                self.decl_names(body, false, block_scope, lex, var);
            }
            Stmt::Try {
                block,
                catch,
                finally,
            } => {
                for s in block {
                    self.decl_names(s, false, block_scope, lex, var);
                }
                if let Some((_, ss)) = catch {
                    for s in ss {
                        self.decl_names(s, false, block_scope, lex, var);
                    }
                }
                if let Some(ss) = finally {
                    for s in ss {
                        self.decl_names(s, false, block_scope, lex, var);
                    }
                }
            }
            Stmt::Switch { cases, .. } => {
                for c in cases {
                    for s in &c.body {
                        self.decl_names(s, false, block_scope, lex, var);
                    }
                }
            }
            _ => {}
        }
    }

    /// Early errors d'une portée (`StatementList`) : doublons dans
    /// `LexicallyDeclaredNames` (annexe B : la paire
    /// `function`-`function` est tolérée en sloppy) et intersection
    /// `Lexical ∩ Var` interdite. `block_scope` = Block/CaseBlock/
    /// catch (`function` y est lexicale) vs top-level/corps.
    fn check_scope(&self, stmts: &[Stmt], block_scope: bool) -> Result<()> {
        let mut lex: Vec<(Rc<str>, bool)> = Vec::new();
        let mut var: Vec<Rc<str>> = Vec::new();
        for s in stmts {
            self.decl_names(s, true, block_scope, &mut lex, &mut var);
        }
        for i in 0..lex.len() {
            for j in (i + 1)..lex.len() {
                if lex[i].0 == lex[j].0 && !(lex[i].1 && lex[j].1 && !self.strict) {
                    return Err(self.err(format!(
                        "identifier `{}` has already been declared",
                        lex[i].0
                    )));
                }
            }
        }
        for (n, _) in &lex {
            if var.iter().any(|v| v == n) {
                return Err(self.err(format!("identifier `{n}` has already been declared")));
            }
        }
        Ok(())
    }

    /// `LexicallyDeclaredNames` + `VarDeclaredNames` d'un statement
    /// unique (corps de `for`/`catch` — traité comme son propre
    /// niveau quand c'est un `Block`).
    fn names_of_stmt(&self, body: &Stmt, lex: &mut Vec<(Rc<str>, bool)>, var: &mut Vec<Rc<str>>) {
        if let Stmt::Block(ss) = body {
            for s in ss {
                self.decl_names(s, true, true, lex, var);
            }
        } else {
            self.decl_names(body, true, true, lex, var);
        }
    }

    /// `for (let/const/using …) body` : les `BoundNames` de la tête
    /// forment une portée avec le corps — doublons internes, et
    /// conflit avec les `var` du corps (spec : head ∩
    /// `VarDeclaredNames` seulement — le `let x` d'un bloc interne
    /// *ombre* la tête, `for (let n;;) { let n }` est légal).
    fn check_for_head(&self, head: &[Rc<str>], body: &Stmt) -> Result<()> {
        for i in 0..head.len() {
            for j in (i + 1)..head.len() {
                if head[i] == head[j] {
                    return Err(self.err(format!(
                        "identifier `{}` has already been declared",
                        head[i]
                    )));
                }
            }
        }
        let mut lex = Vec::new();
        let mut var = Vec::new();
        self.names_of_stmt(body, &mut lex, &mut var);
        for n in head {
            if var.iter().any(|v| v == n) {
                return Err(self.err(format!("identifier `{n}` has already been declared")));
            }
        }
        Ok(())
    }

    /// `catch (pat) body` — spec : doublons du param, param ∩
    /// `LexicallyDeclaredNames`, param ∩ `VarDeclaredNames` —
    /// sauf l'exemption annexe B (`catch(e){var e}` sloppy quand
    /// le param est un identifiant simple).
    fn check_catch(&self, pat: &Pat, body: &[Stmt]) -> Result<()> {
        let mut names = Vec::new();
        Self::bound_names_of(pat, &mut names);
        for i in 0..names.len() {
            for j in (i + 1)..names.len() {
                if names[i] == names[j] {
                    return Err(self.err(format!(
                        "identifier `{}` has already been declared",
                        names[i]
                    )));
                }
            }
        }
        let mut lex = Vec::new();
        let mut var = Vec::new();
        for s in body {
            self.decl_names(s, true, true, &mut lex, &mut var);
        }
        let var_check = self.strict || !matches!(pat, Pat::Ident(_));
        for n in &names {
            if lex.iter().any(|(l, _)| l == n) || (var_check && var.iter().any(|v| v == n)) {
                return Err(self.err(format!("identifier `{n}` has already been declared")));
            }
        }
        Ok(())
    }

    /// `using`/`await using` : bindings `BindingIdentifier` avec
    /// initialiser obligatoire ; interdit au top-level d'un
    /// *script* (portées bloc/fonction/module seulement — spec
    /// `using` exige une portée qui supporte les ressources).
    fn check_using_decls(&self, decls: &[VarDecl], _is_await: bool) -> Result<()> {
        if self.blk_depth == 0 && !self.module {
            return Err(self.err("`using` is not allowed at the top level of a script"));
        }
        for d in decls {
            if !matches!(d.pat, Pat::Ident(_)) || d.init.is_none() {
                return Err(self.err("`using` declarations must be `using x = e`"));
            }
            self.check_no_let_bound(&d.pat)?;
        }
        Ok(())
    }

    /// `let` ne peut jamais être un nom de liaison *lexicale*
    /// (`let let`, `const let`, `let [let]`, `class let`…) —
    /// les deux modes (`var let` sloppy reste légal).
    fn check_no_let_bound(&self, p: &Pat) -> Result<()> {
        let mut names = Vec::new();
        Self::bound_names_of(p, &mut names);
        if names.iter().any(|n| &**n == "let") {
            return Err(self.err("`let` is disallowed as a lexically bound name"));
        }
        Ok(())
    }

    /// `label: label2: … function f` — `IsLabelledFunction` spec :
    /// banni en position de statement unique (`if`/boucle/`with`),
    /// admis comme item d'un label (annexe B sloppy).
    fn is_labelled_fn(s: &Stmt) -> bool {
        match s {
            Stmt::Labeled { body, .. } => Self::is_labelled_fn(body),
            Stmt::FunctionDecl(_) => true,
            _ => false,
        }
    }

    /// Corps nu d'`if`/`else`/boucle/`with`/`label` — la grammaire
    /// n'y accepte qu'un `Statement`, jamais une `Declaration`.
    /// Tolérance annexe B : `function` ordinaire en sloppy et
    /// seulement sous `if`/`else`/`label` (jamais boucle/`with`).
    /// `labelled_item` = la position est l'item d'un `label:` —
    /// là, `a: function f` reste admis (B.3.2).
    fn stmt_body(&mut self, annexb_fn: bool, labelled_item: bool) -> Result<Stmt> {
        let s = self.stmt()?;
        let bad = match &s {
            Stmt::FunctionDecl(f) => f.is_gen || f.is_async || self.strict || !annexb_fn,
            Stmt::Labeled { .. } if !labelled_item && Self::is_labelled_fn(&s) => true,
            Stmt::Lexical { .. }
            | Stmt::ClassDecl(_)
            | Stmt::Using { .. }
            | Stmt::Import { .. }
            | Stmt::Export(_) => true,
            _ => false,
        };
        if bad {
            return Err(self.err("declaration is not allowed in this position"));
        }
        Ok(s)
    }

    /// Early errors des paramètres de fonction : doublons si le
    /// corps est strict ou les params non-simples, nom de
    /// function-expression ∩ params (`function f(f)`), et params
    /// ∩ `LexicallyDeclaredNames` du corps (corps éager). Pour un
    /// corps différé la vérification est différée à sa
    /// matérialisation — déviation documentée (V-5a).
    fn check_fn_params(
        &self,
        params: &[Param],
        body: Option<&[Stmt]>,
        strict_eff: bool,
        expr_name: Option<&str>,
    ) -> Result<()> {
        let mut names: Vec<Rc<str>> = Vec::new();
        for p in params {
            Self::bound_names_of(&p.pat, &mut names);
        }
        let simple = params
            .iter()
            .all(|p| !p.rest && matches!(p.pat, Pat::Ident(_)));
        if strict_eff || !simple {
            for i in 0..names.len() {
                for j in (i + 1)..names.len() {
                    if names[i] == names[j] {
                        return Err(self.err(format!("duplicate parameter name `{}`", names[i])));
                    }
                }
            }
        }
        if let Some(n) = expr_name {
            if names.iter().any(|p| &**p == n) {
                return Err(self.err(format!(
                    "function name `{n}` may not be used as a parameter name"
                )));
            }
        }
        if let Some(b) = body {
            let mut lex = Vec::new();
            let mut var = Vec::new();
            for s in b {
                self.decl_names(s, true, false, &mut lex, &mut var);
            }
            for n in &names {
                if lex.iter().any(|(l, _)| l == n) {
                    return Err(self.err(format!("identifier `{n}` has already been declared")));
                }
            }
        }
        Ok(())
    }

    fn if_stmt(&mut self) -> Result<Stmt> {
        self.expect_kw(Kw::If, "`if`")?;
        self.expect_p(P::LParen, "`(`")?;
        let cond = self.expr(false)?;
        self.expect_p(P::RParen, "`)`")?;
        let then = Box::new(self.stmt_body(true, false)?);
        let else_ = if self.eat_kw(Kw::Else) {
            Some(Box::new(self.stmt_body(true, false)?))
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
        let mut pat = match self.peek_clone() {
            Tok::P(P::Ellipsis) => {
                self.next();
                let inner = self.pattern()?;
                // `......x` : l'élément rest n'est pas lui-même
                // un rest (spec `RestElement`) ; `[...{a} = d]`
                // `[...r = d]` : l'init a été avalé comme défaut
                // du pattern interne — le corps du rest ne peut
                // pas être un `Default` non plus (early error
                // spec, JS-C619).
                if matches!(inner, Pat::Rest(_) | Pat::Default(..)) {
                    return Err(
                        self.err("rest element cannot be a rest element or have an initializer")
                    );
                }
                Pat::Rest(Box::new(inner))
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
                    let elem = self.pattern()?;
                    // Spec : le rest termine la liste — `[...r, x]`
                    // et même `[...r,]` sont des erreurs de parse.
                    if matches!(elem, Pat::Rest(_)) {
                        self.expect_p(P::RBracket, "rest element must be last")?;
                        elems.push(Some(elem));
                        break;
                    }
                    elems.push(Some(elem));
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
                        let rp = self.pattern()?;
                        // `{...{a} = d}` — même interdiction que
                        // le rest tableau (JS-C619).
                        if matches!(rp, Pat::Default(..)) {
                            return Err(self.err("rest property cannot have an initializer"));
                        }
                        rest = Some(Box::new(rp));
                        self.expect_p(P::RBrace, "`}`")?;
                        break;
                    }
                    let key = self.prop_key(false)?;
                    // `{a}` → `a: a`; `{a: p}` / `{a = d}` / `{a: p = d}`.
                    let mut sub = if self.eat_p(P::Colon) {
                        self.pattern()?
                    } else {
                        let name = match &key {
                            PropKey::Name(n) => crate::intern(n),
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
        // `[...rest = d]` / `{...rest = d}` : l'initialiseur est
        // interdit sur un rest (early error spec).
        if matches!(pat, Pat::Rest(_)) && self.check_p(P::Assign) {
            return Err(self.err("rest element cannot have an initializer"));
        }
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
    fn prop_key(&mut self, allow_private: bool) -> Result<PropKey> {
        if self.eat_p(P::LBracket) {
            let e = self.expr(false)?;
            self.expect_p(P::RBracket, "`]`")?;
            return Ok(PropKey::Computed(e));
        }
        let name = match self.peek_clone() {
            Tok::Ident(s) => {
                self.next();
                s
            }
            Tok::Str(s) => {
                self.next();
                crate::intern(&s)
            }
            Tok::Num(n) => {
                self.next();
                crate::intern(&crate::value::num_to_js_string(n))
            }
            Tok::Kw(kw) => {
                self.next();
                crate::intern(kw_str(kw))
            }
            Tok::Private(n) => {
                if !allow_private {
                    // `#x` n'est un nom de propriété légal qu'en
                    // `ClassElement` — `{#x: v}` / `({#x: p} = o)`
                    // sont des erreurs de syntaxe (JS-C614).
                    return Err(self.err("private name not allowed here"));
                }
                self.next();
                crate::intern(&self.resolve_priv(&n)?)
            }
            _ => return Err(self.err("expected property name")),
        };
        Ok(PropKey::Name(name))
    }

    /// `of` — contextual keyword, stays an identifier elsewhere.
    fn eat_of(&mut self) -> bool {
        if matches!(self.peek(), Tok::Ident(s) if &**s == "of") {
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
            self.iter_depth += 1;
            let body = Box::new(self.stmt_body(false, false)?);
            self.iter_depth -= 1;
            self.check_for_target(&var_target, &body)?;
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
            self.iter_depth += 1;
            let body = Box::new(self.stmt_body(false, false)?);
            self.iter_depth -= 1;
            self.check_for_target(&var_target, &body)?;
            return Ok(Stmt::ForIn {
                target: var_target,
                obj,
                var_init,
                body,
            });
        }
        Err(self.err("expected `in` or `of`"))
    }

    /// Tête lexicale `for (let|const|using p …)` : `let` banni
    /// comme nom lié, `using` exige un identifiant simple, et le
    /// scope tête+corps interdit les redeclarations.
    fn check_for_target(&self, target: &ForTarget, body: &Stmt) -> Result<()> {
        let pat = match target {
            ForTarget::Lexical { pat, .. } => pat,
            ForTarget::Using { pat, .. } => {
                if !matches!(pat, Pat::Ident(_)) {
                    return Err(self.err("`using` requires a simple binding identifier"));
                }
                if self.blk_depth == 0 && !self.module {
                    return Err(self.err("`using` is not allowed at the top level of a script"));
                }
                pat
            }
            ForTarget::Var(_) | ForTarget::Expr(_) => return Ok(()),
        };
        self.check_no_let_bound(pat)?;
        let mut head = Vec::new();
        Self::bound_names_of(pat, &mut head);
        self.check_for_head(&head, body)
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
        let is_await = matches!(self.peek(), Tok::Ident(s) if &**s == "await")
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
            let is_of = matches!(self.peek(), Tok::Ident(s) if &**s == "of");
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
        let is_using = matches!(self.peek(), Tok::Ident(s) if &**s == "using")
            && match self.toks.get(self.pos + 1).map(|t| &t.tok) {
                Some(Tok::P(P::LBracket | P::LBrace)) => true,
                Some(Tok::Ident(u)) => &**u != "of",
                _ => false,
            };
        if is_using {
            self.next(); // `using`
            let pat = self.pattern_decl()?;
            if !matches!(self.peek(), Tok::Ident(s) if &**s == "of") {
                return Err(self.err("`using` declarations require a `for-of` loop"));
            }
            return self.for_in_of(ForTarget::Using { pat, is_await }, None, is_await);
        }
        let init = self.expr(true)?;
        if matches!(self.peek(), Tok::Ident(s) if &**s == "of") || self.peek() == &Tok::Kw(Kw::In) {
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
        self.iter_depth += 1;
        let body = Box::new(self.stmt_body(false, false)?);
        self.iter_depth -= 1;
        if let Some(ForInit::Lexical { decls, .. }) = &init {
            for d in decls {
                self.check_no_let_bound(&d.pat)?;
            }
            let mut head = Vec::new();
            for d in decls {
                Self::bound_names_of(&d.pat, &mut head);
            }
            self.check_for_head(&head, &body)?;
        }
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
        self.check_scope(&block, true)?;
        let catch = if self.eat_kw(Kw::Catch) {
            // `try {} catch {}` (optional binding) or `catch (pat)`.
            if self.check_p(P::LBrace) {
                self.expect_p(P::LBrace, "`{`")?;
                let body = self.stmt_list()?;
                self.check_scope(&body, true)?;
                Some((Pat::Ident(Rc::from("")), body))
            } else {
                self.expect_p(P::LParen, "`(`")?;
                let pat = self.pattern()?;
                self.expect_p(P::RParen, "`)`")?;
                self.expect_p(P::LBrace, "`{`")?;
                let body = self.stmt_list()?;
                self.check_scope(&body, true)?;
                // Spec `Catch` early errors — doublons du param,
                // param ∩ lexicales/var du bloc, avec l'exemption
                // annexe B (`catch(e){var e}` sloppy).
                self.check_catch(&pat, &body)?;
                Some((pat, body))
            }
        } else {
            None
        };
        let finally = if self.eat_kw(Kw::Finally) {
            self.expect_p(P::LBrace, "`{`")?;
            let body = self.stmt_list()?;
            self.check_scope(&body, true)?;
            Some(body)
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
        // Le `CaseBlock` est une portée unique (`break` nu légal,
        // `using` admis) — les compteurs s'ouvrent une fois pour
        // tous les cases.
        self.sw_depth += 1;
        self.blk_depth += 1;
        while !self.check_p(P::RBrace) {
            if self.at_eof() {
                self.sw_depth -= 1;
                self.blk_depth -= 1;
                return Err(self.err("unterminated `switch`"));
            }
            let test = if self.eat_kw(Kw::Case) {
                let e = self.expr(false)?;
                Some(e)
            } else if self.eat_kw(Kw::Default) {
                None
            } else {
                self.sw_depth -= 1;
                self.blk_depth -= 1;
                return Err(self.err("expected `case` or `default`"));
            };
            self.expect_p(P::Colon, "`:`")?;
            let mut body = Vec::new();
            while !self.check_p(P::RBrace)
                && self.peek() != &Tok::Kw(Kw::Case)
                && self.peek() != &Tok::Kw(Kw::Default)
            {
                if self.at_eof() {
                    self.sw_depth -= 1;
                    self.blk_depth -= 1;
                    return Err(self.err("unterminated `switch`"));
                }
                body.push(self.stmt()?);
            }
            cases.push(SwitchCase { test, body });
        }
        self.sw_depth -= 1;
        self.blk_depth -= 1;
        self.expect_p(P::RBrace, "`}`")?;
        // Early errors `CaseBlock` : doublons lexicaux et
        // lex∩var à travers *tous* les cases (spec
        // `LexicallyDeclaredNames`/`VarDeclaredNames` du bloc).
        let mut lex = Vec::new();
        let mut var = Vec::new();
        for c in &cases {
            for s in &c.body {
                self.decl_names(s, true, true, &mut lex, &mut var);
            }
        }
        for i in 0..lex.len() {
            for j in (i + 1)..lex.len() {
                if lex[i].0 == lex[j].0 && !(lex[i].1 && lex[j].1 && !self.strict) {
                    return Err(self.err(format!(
                        "identifier `{}` has already been declared",
                        lex[i].0
                    )));
                }
            }
        }
        for (n, _) in &lex {
            if var.iter().any(|v| v == n) {
                return Err(self.err(format!("identifier `{n}` has already been declared")));
            }
        }
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
        if self.in_gen && matches!(self.peek(), Tok::Ident(s) if &**s == "yield") {
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
        if matches!(self.peek(), Tok::Ident(s) if &**s == "async")
            && !self.toks[self.pos + 1].newline_before
        {
            if let Some(e) = self.try_async_arrow(no_in)? {
                return Ok(Some(e));
            }
        }
        // `ident => body`
        if let Tok::Ident(name) = self.peek_clone() {
            if self
                .toks
                .get(self.pos + 1)
                .is_some_and(|t| t.tok == Tok::P(P::Arrow))
            {
                let pos = self.pos_here();
                let soff = self.off_here();
                self.next();
                self.next();
                let (body, strict) = self.arrow_body(no_in)?;
                let params = vec![Param {
                    pat: Pat::Ident(name),
                    rest: false,
                }];
                if let FnBody::Parsed(b) = &body {
                    self.check_fn_params(&params, Some(b), true, None)?;
                }
                let eoff = self.off_prev_end();
                return Ok(Some(Expr::Function(Rc::new(FnLit {
                    name: None,
                    params,
                    body,
                    is_arrow: true,
                    is_method: false,
                    is_async: false,
                    is_gen: false,
                    strict,
                    pos,
                    src: self.src_name.clone(),
                    src_text: self.src_text.clone(),
                    span: (soff, eoff),
                }))));
            }
        }
        // `(params) => body` — speculative scan of a param list.
        // Garde O(1) : `params()` n'est tenté que si `=>` suit le
        // `)` apparié — sinon `(` ouvre une expression parenthésée
        // et la spéculation re-parserait tout le sous-arbre (chaîne
        // `n=(n=(n=…)` : croissance exponentielle — base.js dev.to).
        if self.check_p(P::LParen)
            && self
                .close_of
                .get(self.pos)
                .and_then(|&c| c.checked_add(1))
                .and_then(|i| self.toks.get(i))
                .is_some_and(|t| t.tok == Tok::P(P::Arrow))
        {
            let save = self.pos;
            let pos = self.pos_here();
            let soff = self.off_here();
            if let Some(params) = self.scan_arrow_params() {
                // cursor is on `=>`
                self.next();
                let (body, strict) = self.arrow_body(no_in)?;
                if let FnBody::Parsed(b) = &body {
                    self.check_fn_params(&params, Some(b), true, None)?;
                }
                let eoff = self.off_prev_end();
                return Ok(Some(Expr::Function(Rc::new(FnLit {
                    name: None,
                    params,
                    body,
                    is_arrow: true,
                    is_method: false,
                    is_async: false,
                    is_gen: false,
                    strict,
                    pos,
                    src: self.src_name.clone(),
                    src_text: self.src_text.clone(),
                    span: (soff, eoff),
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
        let soff = self.off_here();
        self.next(); // `async`
                     // `async x => e`
        if let Tok::Ident(name) = self.peek_clone() {
            if self
                .toks
                .get(self.pos + 1)
                .is_some_and(|t| t.tok == Tok::P(P::Arrow))
            {
                self.next();
                self.next();
                let (body, strict) = self.arrow_body(no_in)?;
                let params = vec![Param {
                    pat: Pat::Ident(name),
                    rest: false,
                }];
                if let FnBody::Parsed(b) = &body {
                    self.check_fn_params(&params, Some(b), true, None)?;
                }
                let eoff = self.off_prev_end();
                return Ok(Some(Expr::Function(Rc::new(FnLit {
                    name: None,
                    params,
                    body,
                    is_arrow: true,
                    is_method: false,
                    is_async: true,
                    is_gen: false,
                    strict,
                    pos,
                    src: self.src_name.clone(),
                    src_text: self.src_text.clone(),
                    span: (soff, eoff),
                }))));
            }
            self.pos = save;
            return Ok(None);
        }
        // `async (…) => e` — même garde O(1) que `try_arrow`.
        if self.check_p(P::LParen)
            && self
                .close_of
                .get(self.pos)
                .and_then(|&c| c.checked_add(1))
                .and_then(|i| self.toks.get(i))
                .is_some_and(|t| t.tok == Tok::P(P::Arrow))
        {
            if let Some(params) = self.scan_arrow_params() {
                if self.check_p(P::Arrow) {
                    self.next();
                    let (body, strict) = self.arrow_body(no_in)?;
                    if let FnBody::Parsed(b) = &body {
                        self.check_fn_params(&params, Some(b), true, None)?;
                    }
                    let eoff = self.off_prev_end();
                    return Ok(Some(Expr::Function(Rc::new(FnLit {
                        name: None,
                        params,
                        body,
                        is_arrow: true,
                        is_method: false,
                        is_async: true,
                        is_gen: false,
                        strict,
                        pos,
                        src: self.src_name.clone(),
                        src_text: self.src_text.clone(),
                        span: (soff, eoff),
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
    fn arrow_body(&mut self, no_in: bool) -> Result<(FnBody, bool)> {
        if self.eat_p(P::LBrace) {
            // Même deferral que `fn_body` (V-5a) — une flèche
            // bloc jamais appelée ne paie pas sa descente.
            if let Some(x) = self.try_defer_body(self.pos - 1) {
                return Ok(x);
            }
            let outer_strict = self.strict;
            let outer_gen = self.in_gen;
            let outer_labels = std::mem::take(&mut self.labels);
            let outer_iter = std::mem::replace(&mut self.iter_depth, 0);
            let outer_sw = std::mem::replace(&mut self.sw_depth, 0);
            self.in_gen = false; // les flèches ne sont jamais génératrices
            self.fn_depth += 1;
            self.scan_directives();
            let body = self.stmt_list();
            self.fn_depth -= 1;
            let strict = self.strict;
            let body = body.and_then(|b| self.check_scope(&b, false).map(|()| b));
            self.strict = outer_strict;
            self.in_gen = outer_gen;
            self.labels = outer_labels;
            self.iter_depth = outer_iter;
            self.sw_depth = outer_sw;
            return body.map(|b| (FnBody::Parsed(Rc::new(b)), strict));
        }
        let e = self.assign(no_in)?;
        let strict = self.strict;
        Ok((FnBody::Parsed(Rc::new(vec![Stmt::Return(Some(e))])), strict))
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
        if matches!(self.peek(), Tok::Ident(s) if &**s == "await")
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
            // `delete o.#x` / `delete #x` — les éléments privés ne
            // sont jamais supprimables (early error, tout mode —
            // la clé sentinelle trahit la position privée).
            if op == UnOp::Delete {
                let priv_ref = match &e {
                    Expr::Member(_, k) => k.starts_with('\u{f131}'),
                    Expr::Str(s) => s.starts_with('\u{f131}'),
                    Expr::Chain { steps, .. } => steps.iter().any(|s| match s {
                        crate::ast::ChainStep::Member(k) | crate::ast::ChainStep::OptMember(k) => {
                            k.starts_with('\u{f131}')
                        }
                        _ => false,
                    }),
                    _ => false,
                };
                if priv_ref {
                    return Err(self.err("private fields can not be deleted"));
                }
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
        // Position du `new` — site d'appel porté par `Expr::New`
        // (N1.1) ; lue avant `eat_kw` qui avance le curseur.
        let new_pos = self.pos_here();
        let mut e = if self.eat_kw(Kw::New) {
            if self.eat_p(P::Dot) {
                // `new.target` — meta-property : la seule paire
                // `new . nom` valide de la grammaire.
                let n = self.expect_prop_name()?;
                if &*n != "target" {
                    return Err(self.err("expected `new.target`"));
                }
                Expr::NewTarget
            } else {
                let callee = self.new_callee(no_in)?;
                let args = if self.check_p(P::LParen) {
                    self.args()?
                } else {
                    Vec::new()
                };
                Expr::New(Box::new(callee), args, new_pos)
            }
        } else {
            self.primary()?
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
                // Premier `?.` de la chaîne : l'accès membre déjà
                // plié dans `e` (`a.b?.()`, `a.b?.c`) doit migrer
                // dans les steps — sinon le récepteur du `?.()` est
                // le global au lieu de `a.b` (spec `Reference` —
                // trigger github.com `e.matches?.()` dans le
                // polyfill popover).
                if s.is_empty() {
                    // `Expr` a un Drop itératif : pas de
                    // déstructuration — on extrait les champs
                    // par `mem::replace`.
                    let mut old = std::mem::replace(&mut e, Expr::Null);
                    match &mut old {
                        Expr::Member(inner, name) => {
                            s.push(ChainStep::Member(std::mem::take(name)));
                            e = std::mem::replace(&mut **inner, Expr::Null);
                        }
                        Expr::Index(inner, key) => {
                            s.push(ChainStep::Index(std::mem::replace(&mut **key, Expr::Null)));
                            e = std::mem::replace(&mut **inner, Expr::Null);
                        }
                        _ => e = old,
                    }
                }
                if self.check_p(P::LParen) {
                    let args = self.args()?;
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
                // Position du `(` — site d'appel porté par
                // `Expr::Call` (N1.1), lue avant `args()`.
                let call_pos = self.pos_here();
                let args = self.args()?;
                match &mut steps {
                    Some(s) => s.push(ChainStep::Call(args)),
                    None => e = Expr::Call(Box::new(e), args, call_pos),
                }
            } else if let Tok::Tpl(_) = self.peek() {
                // `` tag`…` `` — tagged template (MemberExpression,
                // spec : interdit dans une chaîne optionnelle).
                if steps.is_some() {
                    return Err(self.err("template literal in an optional chain"));
                }
                let Tok::Tpl(parts) = self.next_clone() else {
                    return Err(self.err("malformed template literal"));
                };
                let (parts, raws, exprs) = self.template_parts(&parts)?;
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
        let new_pos = self.pos_here();
        let mut e = if self.eat_kw(Kw::New) {
            let callee = self.new_callee(no_in)?;
            let args = if self.check_p(P::LParen) {
                self.args()?
            } else {
                Vec::new()
            };
            Expr::New(Box::new(callee), args, new_pos)
        } else {
            self.primary()?
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

    /// Arguments `(a, …)` — `no_in` est oublié ici : la spec
    /// remet `+In` dès le `(` ouvrant (JS-C619 — `f(a in b)`
    /// dans une tête de `for` refusait l'`in` interne).
    fn args(&mut self) -> Result<Vec<Expr>> {
        self.expect_p(P::LParen, "`(`")?;
        let mut out = Vec::new();
        if self.eat_p(P::RParen) {
            return Ok(out);
        }
        loop {
            // Virgule finale ES2017 : `f(a,)` — la parenthèse
            // ferme sans arg après la virgule (spectrum
            // ak.js:280 : `every(…,)`).
            if self.eat_p(P::RParen) {
                return Ok(out);
            }
            // `f(...a)` spread arg — `Expr::Spread` marker.
            if self.eat_p(P::Ellipsis) {
                out.push(Expr::Spread(Box::new(self.assign(false)?)));
            } else {
                out.push(self.assign(false)?);
            }
            if self.eat_p(P::RParen) {
                return Ok(out);
            }
            self.expect_p(P::Comma, "`,`")?;
        }
    }

    fn primary(&mut self) -> Result<Expr> {
        match self.peek_clone() {
            Tok::Num(n) => {
                if self.strict && self.cur().legacy_octal {
                    return Err(self.err("legacy octal literal in strict mode"));
                }
                self.next();
                Ok(Expr::Num(n))
            }
            Tok::Big(s) => {
                self.next();
                Ok(Expr::BigInt(s.to_string()))
            }
            Tok::Str(s) => {
                if self.strict && self.cur().legacy_octal {
                    return Err(self.err("legacy octal escape in strict mode"));
                }
                self.next();
                Ok(Expr::Str(s.to_string()))
            }
            Tok::Ident(s) => {
                // `async function [f](){}` — expression form.
                if &*s == "async"
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
                    let args = self.args()?;
                    return Ok(Expr::SuperCall(args));
                }
                if self.eat_p(P::Dot) {
                    // `super.#x` — les références privées ne sont
                    // jamais membres de `super` (early error spec).
                    if matches!(self.peek(), Tok::Private(_)) {
                        return Err(self.err("private name not allowed after `super.`"));
                    }
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
                self.template(&parts)
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
                    pattern: body.to_string(),
                    flags: flags.to_string(),
                })
            }
            // `import(spec)` / `import.meta` — legal in modules and
            // classic scripts alike.
            Tok::Kw(Kw::Import) => {
                self.next();
                if self.eat_p(P::LParen) {
                    let spec = self.assign(false)?;
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
            Tok::P(P::LBracket) => self.array_lit(),
            Tok::P(P::LBrace) => self.object_lit(),
            _ => Err(self.err("expected expression")),
        }
    }

    /// `[a, …]` — `+In` restauré entre crochets (JS-C619).
    fn array_lit(&mut self) -> Result<Expr> {
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
                elems.push(Some(Expr::Spread(Box::new(self.assign(false)?))));
            } else {
                elems.push(Some(self.assign(false)?));
            }
            if self.eat_p(P::Comma) {
                continue;
            }
            self.expect_p(P::RBracket, "`]`")?;
            return Ok(Expr::Array(elems));
        }
    }

    /// `{a: v, …}` — `+In` restauré entre accolades (JS-C619).
    fn object_lit(&mut self) -> Result<Expr> {
        self.expect_p(P::LBrace, "`{`")?;
        let mut props = Vec::new();
        if self.eat_p(P::RBrace) {
            return Ok(Expr::Object(props));
        }
        loop {
            // `{...o}` spread.
            if self.eat_p(P::Ellipsis) {
                props.push(ObjProp::Spread(self.assign(false)?));
            } else {
                let kpos = self.pos_here();
                // V8 : le source de la méthode part du premier
                // jeton du membre — `get`/`set`/`async`/`*`/clé.
                let koff = self.off_here();
                // `*m(){}` generator method / `async *m(){}`.
                let mut gen = self.eat_p(P::Star);
                // `async m(){}` method.
                let async_ = matches!(self.peek(), Tok::Ident(s) if &**s == "async")
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
                if matches!(self.peek(), Tok::Ident(s) if &**s == "get" || &**s == "set")
                    && !matches!(
                        self.toks.get(self.pos + 1).map(|t| &t.tok),
                        Some(Tok::P(
                            P::LParen | P::Assign | P::Colon | P::Comma | P::RBrace
                        ))
                    )
                {
                    accessor = Some(if matches!(self.peek(), Tok::Ident(s) if &**s == "get") {
                        self.next();
                        ClassMemberKind::Getter
                    } else {
                        self.next();
                        ClassMemberKind::Setter
                    });
                }
                let key = self.prop_key(false)?;
                match accessor {
                    Some(kind) => {
                        self.expect_p(P::LParen, "`(`")?;
                        let params = self.params()?;
                        self.expect_p(P::LBrace, "`{`")?;
                        let outer_gen = self.in_gen;
                        self.in_gen = false; // get/set : jamais générateurs
                        let (body, strict) = self.fn_body_or_deferred()?;
                        if let FnBody::Parsed(b) = &body {
                            self.check_fn_params(&params, Some(b), strict, None)?;
                        }
                        self.in_gen = outer_gen;
                        let fun = Rc::new(FnLit {
                            name: None,
                            params,
                            body,
                            is_arrow: false,
                            is_method: true,
                            is_async: false,
                            is_gen: false,
                            strict,
                            pos: kpos,
                            src: self.src_name.clone(),
                            src_text: self.src_text.clone(),
                            span: (koff, self.off_prev_end()),
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
                        let outer_gen = self.in_gen;
                        self.in_gen = gen;
                        let (body, strict) = self.fn_body_or_deferred()?;
                        if let FnBody::Parsed(b) = &body {
                            self.check_fn_params(&params, Some(b), strict, None)?;
                        }
                        self.in_gen = outer_gen;
                        props.push(ObjProp::Prop(
                            key,
                            Expr::Function(Rc::new(FnLit {
                                name: None,
                                params,
                                body,
                                is_arrow: false,
                                is_method: true,
                                is_async: async_,
                                is_gen: gen,
                                strict,
                                pos: kpos,
                                src: self.src_name.clone(),
                                src_text: self.src_text.clone(),
                                span: (koff, self.off_prev_end()),
                            })),
                        ));
                    }
                    None => {
                        if self.eat_p(P::Colon) {
                            props.push(ObjProp::Prop(key, self.assign(false)?));
                        } else if self.eat_p(P::Assign) {
                            // `{a = 1}` — destructuring-default shape;
                            // as a plain literal it just assigns then
                            // stores the value (harmless tolerance).
                            let name = match &key {
                                PropKey::Name(n) => crate::intern(n),
                                PropKey::Computed(_) => {
                                    return Err(self.err("bad `{…=…}` shorthand"))
                                }
                            };
                            let rhs = self.assign(false)?;
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
                                PropKey::Name(n) => crate::intern(n),
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
    fn template(&mut self, parts: &[TplPart]) -> Result<Expr> {
        let (lit_parts, _raws, exprs) = self.template_parts(parts)?;
        Ok(Expr::Template {
            parts: lit_parts,
            exprs,
        })
    }

    /// Chunks d'un template : `(cooked, raws, exprs)` — partagé
    /// entre `` `…` `` (`Expr::Template`) et `` tag`…` ``
    /// (`Expr::TaggedTemplate`, qui consomme aussi les raws).
    /// `no_in` n'entre pas dans `${…}` — `+In` restauré
    /// (JS-C619).
    fn template_parts(
        &mut self,
        parts: &[TplPart],
    ) -> Result<(Vec<String>, Vec<String>, Vec<Expr>)> {
        let mut lit_parts = Vec::new();
        let mut raws = Vec::new();
        let mut exprs = Vec::new();
        for part in parts {
            match part {
                TplPart::Str(s, raw) => {
                    lit_parts.push(s.clone());
                    raws.push(raw.clone());
                }
                TplPart::Expr(toks) => {
                    let close_of = Rc::new(close_table(toks));
                    let mut sub = Parser {
                        toks: Rc::from(toks.as_slice()),
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
                        src_name: self.src_name.clone(),
                        // Tokens relatifs au segment `${…}` —
                        // offsets non tranchables dans la source
                        // complète → toString repli `[code]`.
                        src_text: None,
                        close_of,
                        // La substitution `${…}` vit dans le même
                        // contexte — compteurs hérités.
                        fn_depth: self.fn_depth,
                        iter_depth: self.iter_depth,
                        sw_depth: self.sw_depth,
                        blk_depth: self.blk_depth,
                    };
                    let e = sub.expr(false)?;
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
        // `async function` : l'appelant a déjà consommé `async` —
        // V8 l'inclut dans le source rendu (`async function f(){}`)
        // → le span part du jeton précédent.
        let soff = if async_ {
            self.toks[self.pos.saturating_sub(1)].off
        } else {
            self.off_here()
        };
        self.expect_kw(Kw::Function, "`function`")?;
        // `function*` / `async function*`.
        let gen = self.eat_p(P::Star);
        let name = match self.peek_clone() {
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
        let outer_gen = self.in_gen;
        self.in_gen = gen;
        let (body, strict) = self.fn_body_or_deferred()?;
        // Early errors params↔corps : doublons (strict ou
        // non-simples), nom d'expression ∩ params, params ∩
        // lexicales du corps — corps différé : la part `corps`
        // attend la matérialisation (déviation V-5a).
        {
            let body_stmts = match &body {
                FnBody::Parsed(b) => Some(&b[..]),
                FnBody::Deferred(_) => None,
            };
            let expr_name = if decl { None } else { name.as_deref() };
            self.check_fn_params(&params, body_stmts, strict, expr_name)?;
        }
        // `fn_body` restaure la valeur **posée juste avant** —
        // sans ce second restore, un `function(){}` non-générateur
        // imbriqué dans un `function*(){}` masque les `yield`
        // du corps englobant (parse `expected ';'` — spotify
        // `vendor~mobile-web-player`, `__awaiter(this,…,function*(){…})`).
        self.in_gen = outer_gen;
        let eoff = self.off_prev_end();
        Ok(Rc::new(FnLit {
            name,
            params,
            body,
            is_arrow: false,
            is_method: false,
            is_async: async_,
            is_gen: gen,
            strict,
            pos,
            src: self.src_name.clone(),
            src_text: self.src_text.clone(),
            span: (soff, eoff),
        }))
    }

    /// Corps `{` ouvert à l'index `lbrace` (déjà consommé) —
    /// tentative de deferral V-5a : retient la plage de tokens
    /// et saute au `}` en O(1) via `close_of`. `None` → le
    /// caller retombe sur le parse éager (`fn_body`/`stmt_list`)
    /// qui produit les erreurs normales (délimiteur désapparié,
    /// corps trop court…).
    fn try_defer_body(&mut self, lbrace: usize) -> Option<(FnBody, bool)> {
        let close = *self.close_of.get(lbrace)?;
        if close == usize::MAX || close.saturating_sub(lbrace) < defer_min_toks() {
            return None;
        }
        // Strict effectif fixé dès le parse — le prologue de
        // directives est scanné en lecture seule pour que
        // `FnLit.strict` soit exact au premier appel.
        let strict = self.strict || deferred_use_strict(&self.toks[lbrace + 1..close]);
        let d = DeferredBody {
            // V-5b : flux parent + table `close_of` partagés —
            // ni copie de slice ni reconstruction : un bump de
            // refcount par corps différé (au lieu d'un clone de
            // `Token` par jeton du corps + O(slice) au premier
            // appel).
            toks: self.toks.clone(),
            start: lbrace,
            end: close,
            close: self.close_of.clone(),
            strict: self.strict,
            module: self.module,
            nest: self.nest,
            nest_max: self.nest_max,
            priv_seq: self.priv_seq,
            priv_scopes: self.priv_scopes.clone(),
            src_name: self.src_name.clone(),
            src_text: self.src_text.clone(),
            parsed: OnceCell::new(),
        };
        crate::j17_tick(&crate::J17_FN_DEFER);
        crate::j17_add(&crate::J17_FN_DEF_TOK, (close - lbrace + 1) as u64);
        self.pos = close + 1;
        Some((FnBody::Deferred(Rc::new(d)), strict))
    }

    /// `fn_body` avec deferral : `{` consommé à `pos-1` — corps
    /// différé si la plage le justifie, sinon parse éager
    /// enrobé. Retourne `(corps, strict effectif)`.
    fn fn_body_or_deferred(&mut self) -> Result<(FnBody, bool)> {
        if let Some(x) = self.try_defer_body(self.pos - 1) {
            return Ok(x);
        }
        self.fn_body().map(|(b, s)| (FnBody::Parsed(Rc::new(b)), s))
    }

    /// Corps de fonction déjà ouvert (`{` consommé) — prologue de
    /// directives scanné, retourne `(stmts, strict_effectif)`.
    /// `self.in_gen` est celui posé par l'appelant (save/restore ici).
    fn fn_body(&mut self) -> Result<(Vec<Stmt>, bool)> {
        let outer_strict = self.strict;
        let outer_gen = self.in_gen;
        // Les labels ne traversent pas la frontière fonction.
        let outer_labels = std::mem::take(&mut self.labels);
        let outer_iter = std::mem::replace(&mut self.iter_depth, 0);
        let outer_sw = std::mem::replace(&mut self.sw_depth, 0);
        self.fn_depth += 1;
        self.scan_directives();
        let body = self.stmt_list();
        self.fn_depth -= 1;
        let strict = self.strict;
        // `check_scope` avec le strict *effectif* du corps (avant
        // le restore) — c'est lui qui décide des tolérances sloppy.
        let body = body.and_then(|b| self.check_scope(&b, false).map(|()| b));
        self.strict = outer_strict;
        self.in_gen = outer_gen;
        self.labels = outer_labels;
        self.iter_depth = outer_iter;
        self.sw_depth = outer_sw;
        body.map(|b| (b, strict))
    }

    /// Corps `{…}` parsé comme frontière fonction — labels et
    /// compteurs `iter`/`sw` réinitialisés (les break/continue/
    /// labels ne traversent pas), `fn_depth` incrémenté si
    /// `is_fn` (un bloc statique de classe reste interdit de
    /// `return`). `check_scope` appliqué à la sortie.
    fn scoped_fn_body(&mut self, is_fn: bool) -> Result<Vec<Stmt>> {
        let outer_labels = std::mem::take(&mut self.labels);
        let outer_iter = std::mem::replace(&mut self.iter_depth, 0);
        let outer_sw = std::mem::replace(&mut self.sw_depth, 0);
        if is_fn {
            self.fn_depth += 1;
        }
        let body = self.stmt_list();
        if is_fn {
            self.fn_depth -= 1;
        }
        let body = body.and_then(|b| self.check_scope(&b, false).map(|()| b));
        self.labels = outer_labels;
        self.iter_depth = outer_iter;
        self.sw_depth = outer_sw;
        body
    }

    /// `pat (= default)? (, …)*` with a possible trailing `...rest`
    /// — cursor left after `)`.
    fn params(&mut self) -> Result<Vec<Param>> {
        let mut params = Vec::new();
        if !self.check_p(P::RParen) {
            loop {
                let rest = self.eat_p(P::Ellipsis);
                let pat = self.pattern()?;
                // `f(...r = d)` — l'init a été avalé comme défaut
                // du pattern : early error spec (JS-C619).
                if rest && matches!(pat, Pat::Default(..)) {
                    return Err(self.err("rest parameter cannot have an initializer"));
                }
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
        // `class_off` — borne gauche du span `class …` entier
        // (V8 : `C.toString()` = le texte `class` complet).
        let class_off = self.off_here();
        self.expect_kw(Kw::Class, "`class`")?;
        let name = match self.peek_clone() {
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
            if matches!(self.peek(), Tok::Ident(s) if &**s == "static")
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
            // V8 : `static` n'entre pas dans le source de la
            // méthode (`static m(){}` → `m(){}`) — le span part
            // du jeton suivant (`async`/`*`/`get`/`set`/clé/`{`).
            let fn_off = self.off_here();
            // `static { … }` — bloc d'initialisation statique.
            if static_ && self.check_p(P::LBrace) {
                self.next();
                // `super(` dans un bloc statique → early error
                // (un bloc n'est jamais un ctor dérivé).
                let blk_close = self
                    .close_of
                    .get(self.pos - 1)
                    .copied()
                    .unwrap_or(self.toks.len());
                if self.scan_super_region(self.pos - 1, (blk_close + 1).min(self.toks.len())) {
                    return Err(self.err("`super()` call is only allowed in a derived constructor"));
                }
                let outer_strict = self.strict;
                self.strict = true;
                let body = match self.try_defer_body(self.pos - 1) {
                    Some((b, _)) => b,
                    None => FnBody::Parsed(Rc::new(self.scoped_fn_body(false)?)),
                };
                self.strict = outer_strict;
                members.push(ClassMember {
                    key: PropKey::Name(Rc::from("")),
                    kind: ClassMemberKind::StaticBlock,
                    static_: true,
                    fun: Rc::new(FnLit {
                        name: None,
                        params: Vec::new(),
                        body,
                        is_arrow: false,
                        is_method: true,
                        is_async: false,
                        is_gen: false,
                        strict: true,
                        pos: mpos,
                        src: self.src_name.clone(),
                        src_text: self.src_text.clone(),
                        span: (fn_off, self.off_prev_end()),
                    }),
                });
                continue;
            }
            // `async m()` — same lookahead rule.
            let async_ = matches!(self.peek(), Tok::Ident(s) if &**s == "async")
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
            // `get`/`set` ne sont des modifieurs que devant un
            // *nom* de membre — `get\n*x(){}` est un champ `get`
            // suivi d'un générateur (ASI), pas un accesseur.
            if matches!(self.peek(), Tok::Ident(s) if &**s == "get" || &**s == "set")
                && matches!(
                    self.toks.get(self.pos + 1).map(|t| &t.tok),
                    Some(
                        Tok::Ident(_)
                            | Tok::Kw(_)
                            | Tok::Str(_)
                            | Tok::Num(_)
                            | Tok::Private(_)
                            | Tok::P(P::LBracket)
                    )
                )
            {
                kind = if matches!(self.peek(), Tok::Ident(s) if &**s == "get") {
                    self.next();
                    ClassMemberKind::Getter
                } else {
                    self.next();
                    ClassMemberKind::Setter
                };
            }
            let key = self.prop_key(true)?;
            // `constructor` ne devient `ConstructorMethod` que
            // sur une méthode *simple* non statique — `async`/
            // `*`/get/set/`static` le gardent `Method` et le
            // post-check le rejette (spec `ClassElement`).
            if !static_ && !async_ && !gen && kind == ClassMemberKind::Method {
                if let PropKey::Name(n) = &key {
                    if &**n == "constructor" {
                        kind = ClassMemberKind::Ctor;
                    }
                }
            }
            // `x` / `x = e` / `#x = e` — pas de `(` : champ
            // d'instance (ou statique). L'init devient un thunk
            // exécuté par le runtime à la construction.
            if !self.check_p(P::LParen) {
                let init = if self.eat_p(P::Assign) {
                    // `super(` dans un init de champ → early
                    // error (un init n'est pas un ctor dérivé ;
                    // `super.x` reste licite — HomeObject).
                    let init_pos = self.pos;
                    let e = self.assign(false)?;
                    if self.scan_super_region(init_pos, self.pos) {
                        return Err(
                            self.err("`super()` call is only allowed in a derived constructor")
                        );
                    }
                    Some(e)
                } else {
                    None
                };
                // `FieldDefinition ;` — pas d'ASI intra-ligne :
                // `;`, `}`, EOF ou saut de ligne exigé après le
                // champ (spec — `x y` en ligne est fautif).
                if !self.eat_p(P::Semi)
                    && !self.at_eof()
                    && !self.check_p(P::RBrace)
                    && !self.cur().newline_before
                {
                    return Err(self.err("expected `;` after class field"));
                }
                let body = FnBody::Parsed(Rc::new(
                    init.map(|e| vec![Stmt::Return(Some(e))])
                        .unwrap_or_default(),
                ));
                members.push(ClassMember {
                    key,
                    kind: ClassMemberKind::Field,
                    static_,
                    fun: Rc::new(FnLit {
                        name: None,
                        params: Vec::new(),
                        body,
                        is_arrow: false,
                        is_method: true,
                        is_async: false,
                        is_gen: false,
                        strict: true,
                        pos: mpos,
                        src: self.src_name.clone(),
                        src_text: self.src_text.clone(),
                        span: (fn_off, self.off_prev_end()),
                    }),
                });
                continue;
            }
            let params_pos = self.pos;
            self.expect_p(P::LParen, "`(`")?;
            let params = self.params()?;
            self.expect_p(P::LBrace, "`{`")?;
            // `super(` n'existe que dans le ctor d'une classe
            // *dérivée* — `HasDirectSuper` est une early error
            // partout ailleurs : méthodes et accesseurs (toute
            // classe), ctor sans `extends`. Scan tokens — couvre
            // aussi le corps différé.
            if (kind != ClassMemberKind::Ctor || super_.is_none()) && {
                let close = self
                    .close_of
                    .get(self.pos - 1)
                    .copied()
                    .unwrap_or(self.toks.len());
                self.scan_super_region(params_pos, (close + 1).min(self.toks.len()))
            } {
                return Err(self.err("`super()` call is only allowed in a derived constructor"));
            }
            // Corps de classe : toujours strict, prologue scanné
            // pour les early errors.
            let outer_strict = self.strict;
            self.strict = true;
            let outer_gen = self.in_gen;
            // get/set ne sont jamais générateurs ; `*m` oui.
            self.in_gen = gen && kind == ClassMemberKind::Method;
            let is_gen = self.in_gen;
            let body = if let Some((b, _)) = self.try_defer_body(self.pos - 1) {
                b
            } else {
                let b = self.scoped_fn_body(true)?;
                self.check_fn_params(&params, Some(&b), true, None)?;
                FnBody::Parsed(Rc::new(b))
            };
            self.strict = outer_strict;
            self.in_gen = outer_gen;
            members.push(ClassMember {
                key,
                kind,
                static_,
                fun: Rc::new(FnLit {
                    name: None,
                    params,
                    body,
                    is_arrow: false,
                    // Le ctor de classe **est** constructible —
                    // `is_method` exclut `[[Construct]]` (C609),
                    // il ne doit pas le marquer.
                    is_method: kind != ClassMemberKind::Ctor,
                    is_async: async_,
                    is_gen,
                    strict: true,
                    pos: mpos,
                    src: self.src_name.clone(),
                    src_text: self.src_text.clone(),
                    span: (fn_off, self.off_prev_end()),
                }),
            });
        }
        // Early errors déclaratifs du corps complet : ctor
        // unique, noms privés uniques (paire get/set seule),
        // noms réservés (spec `ClassElement` — JS-C614).
        self.check_class_members(&members, seq)?;
        self.expect_p(P::RBrace, "`}`")?;
        self.priv_scopes.pop();
        Ok(Rc::new(ClassLit {
            name,
            super_,
            members,
            src_text: self.src_text.clone(),
            span: (class_off, self.off_prev_end()),
        }))
    }
}

fn is_lvalue(e: &Expr) -> bool {
    // `super.x`/`super[k]` sont des lvalues spec (mur réel
    // x-web : `super.foo = …` dans les champs de classe).
    matches!(
        e,
        Expr::Ident(_)
            | Expr::Member(..)
            | Expr::Index(..)
            | Expr::SuperMember(_)
            | Expr::SuperIndex(_)
    )
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Corpus différentiel — mêmes classes de sources que les
    /// verrous historiques du deferral : regex ambiguës, `${}`
    /// imbriqués, `with`, corps imbriqués, `of` contextuel,
    /// accolades en strings/commentaires, directives.
    const CORPUS: &[&str] = &[
        "function f(){ return 1 } f()",
        "var re = /a}b{c/; function g(){ return re.test('}') }",
        "function outer(){ function inner(){ return `x${{a:1}}y` } return inner() }",
        "(function(){ var a={}; with(a||{}){ x=1; y=2; z=3; w=4; v=5; u=6; t=7 } })",
        "(function(event){ with(document||{}){ with(this.form||{}){ with(this){ hit=(hit||0)+1 } } } })",
        "for (const x of [1,2]) { let y = x } const of = 3",
        "function s(){ 'use strict'; return 1 }",
        "function c(){ /* } { */ let x = 1; // }\n return x }",
        "class K { static { let q = 1 } m(){ return `${1+{o:2}.o}` } }",
        "const f = async () => { await g(); return async function(){ return 1 } }",
        "function h(){ if (a||{}) { x=1;y=2;z=3;w=4;v=5;u=6;t=7;s=8 } return x }",
        "x = { a: function(){ return { b: () => { return 1 } } } }",
        "function tpl(){ return `a${ {k:1}.k }b${ function(){return 2}() }c` }",
    ];

    /// Hash FNV d'un `Debug` incrémental — comparaison d'AST
    /// sans chaîne géante (un bundle 1,4 Mo → des centaines de
    /// Mo de `Debug` sinon — OOM constaté au diagnostic N5.26).
    struct Fnv(u64);
    impl std::fmt::Write for Fnv {
        fn write_str(&mut self, s: &str) -> std::fmt::Result {
            for &b in s.as_bytes() {
                self.0 ^= b as u64;
                self.0 = self.0.wrapping_mul(0x0100_0000_01b3);
            }
            Ok(())
        }
    }
    fn prog_hash(p: &crate::ast::Program) -> u64 {
        use std::fmt::Write;
        let mut h = Fnv(0xcbf2_9ce4_8422_2325);
        let _ = write!(h, "{p:?}");
        h.0
    }

    /// **Différentiel defer** : le `Program` produit avec corps
    /// différés doit être **identique** à celui du chemin tout
    /// éager (`DEFER_MIN_TOKS = usize::MAX`) — le `Debug` de
    /// `FnLit` matérialise les corps, donc les sous-parses
    /// tardifs sont couverts. Corpus statique + bundles réels
    /// capturés (`tests/data/` — dont le vrai `m=base` yt de
    /// N5.26). Tout écart = borne de corps, position ou
    /// matérialisation divergente.
    #[test]
    fn deferred_ast_matches_eager() {
        let ord = std::sync::atomic::Ordering::Relaxed;
        let mut all: Vec<String> = CORPUS.iter().map(ToString::to_string).collect();
        for f in [
            "vol34_bbc_app.js",
            "vol16_dsfr_core.js",
            "bitmovin_transmuxer.js",
            "vol12_spotify_gen_slice.js",
            "vol526_yt_base.js",
        ] {
            if let Ok(s) =
                std::fs::read_to_string(format!("{}/tests/data/{f}", env!("CARGO_MANIFEST_DIR")))
            {
                all.push(s);
            }
        }
        for (n, src) in all.iter().enumerate() {
            DEFER_MIN_TOKS.store(usize::MAX, ord);
            let eager = parse_in(src, false, 4096, None);
            DEFER_MIN_TOKS.store(24, ord);
            let deferred = parse_in(src, false, 4096, None);
            DEFER_MIN_TOKS.store(24, ord);
            match (eager, deferred) {
                (Err(e1), Err(e2)) => assert_eq!(
                    format!("{e1:?}"),
                    format!("{e2:?}"),
                    "src#{n}: erreurs divergentes"
                ),
                (Ok(a), Ok(b)) => {
                    let (ha, hb) = (prog_hash(&a), prog_hash(&b));
                    if ha != hb {
                        // Localise : premier statement divergent.
                        let mut loc = String::new();
                        for i in 0..a.body.len().max(b.body.len()) {
                            let (x, y) = (
                                a.body.get(i).map(|s| format!("{s:?}")),
                                b.body.get(i).map(|s| format!("{s:?}")),
                            );
                            if x != y {
                                let trunc = |o: &Option<String>| {
                                    o.as_deref().map_or_else(
                                        || "<absent>".into(),
                                        |d| d.chars().take(300).collect::<String>(),
                                    )
                                };
                                loc = format!(
                                    "stmt#{i}\n  eager   : {}\n  deferred: {}",
                                    trunc(&x),
                                    trunc(&y)
                                );
                                break;
                            }
                        }
                        panic!("src#{n}: AST divergent\n{loc}");
                    }
                }
                (a, b) => panic!(
                    "src#{n}: eager={} vs deferred={}",
                    a.map_or("Err", |_| "Ok"),
                    b.map_or("Err", |_| "Ok")
                ),
            }
        }
    }
}
