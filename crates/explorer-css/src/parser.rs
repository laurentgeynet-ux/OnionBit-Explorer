// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Parser CSS (spec CSS Syntax) : feuille → règles qualifiées et
//! at-rules, blocs de déclarations, récupération d'erreurs.
//!
//! Modèle : la valeur d'une déclaration reste **texte brut**
//! (trimée) — le typage par propriété est le travail de la
//! cascade/computed style (étape 15+). Le prélude d'une règle
//! qualifiée reste aussi texte : l'étape 14 y branche
//! `selector::parse`.

use crate::token::Token;
use crate::tokenizer::tokenize;

/// Une feuille de style parsée.
#[derive(Debug, Clone, Default)]
pub struct Stylesheet {
    /// Règles de niveau supérieur, dans l'ordre source.
    pub rules: Vec<Rule>,
    /// `@charset` rencontré en tête (informatif — le sniffing
    /// d'encodage est déjà fait en amont).
    pub charset: Option<String>,
    /// `@import` collectés (url + media prelude) — la résolution
    /// est l'affaire de l'appelant réseau, pas du domaine.
    pub imports: Vec<ImportRule>,
}

/// Une règle de la feuille.
#[derive(Debug, Clone, PartialEq)]
pub enum Rule {
    /// `prelude { decls }` — le prélude est le sélecteur brut
    /// (parsé par l'étape 14).
    Qualified(QualifiedRule),
    /// `@name prelude { … }` ou `@name prelude ;`.
    At(AtRule),
}

/// Règle qualifiée (style rule).
#[derive(Debug, Clone, PartialEq)]
pub struct QualifiedRule {
    /// Prélude sélecteur brut (ex : `div.a > p, span`).
    pub prelude: String,
    /// Déclarations du bloc.
    pub declarations: Vec<Declaration>,
}

/// At-rule reconnue.
#[derive(Debug, Clone, PartialEq)]
pub struct AtRule {
    /// Nom sans `@` (minuscules).
    pub name: String,
    /// Contenu : media queries pour `@media`, texte brut sinon.
    pub kind: AtKind,
}

/// Corps d'une at-rule selon son nom.
#[derive(Debug, Clone, PartialEq)]
pub enum AtKind {
    /// `@media <queries> { rules }` — queries structurées.
    Media {
        /// Liste de media queries (`,` = OU).
        queries: Vec<MediaQuery>,
        /// Règles imbriquées.
        rules: Vec<Rule>,
    },
    /// Bloc de déclarations (`@font-face`, `@page`…).
    Declarations {
        /// Prélude brut (ex : nom de police pour font-face — vide
        /// en pratique, mais conservé).
        prelude: String,
        /// Déclarations du bloc.
        declarations: Vec<Declaration>,
    },
    /// `@keyframes <nom> { <selecteurs> { decls } }` — sous-règles
    /// à prélude brut (pourcentages/from/to).
    Keyframes {
        /// Nom de l'animation.
        name: String,
        /// `(sélecteur de frame, déclarations)`.
        frames: Vec<(String, Vec<Declaration>)>,
    },
    /// Sans bloc : `@name prelude ;` (`@import`, `@charset`,
    /// `@namespace`, `@supports` sans cas nominal ici, etc.) —
    /// le prélude est conservé brut.
    Statement {
        /// Prélude brut.
        prelude: String,
    },
    /// `@name { … }` non reconnu : contenu brut en tokens perdus —
    /// la règle est conservée pour debug mais inappliquée.
    UnknownBlock {
        /// Prélude brut.
        prelude: String,
    },
}

/// `@import url media ;` capturé pour l'appelant.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportRule {
    /// Cible (contenu de `url(...)` ou chaîne).
    pub url: String,
    /// Prélude media brut après l'url.
    pub media: String,
}

/// Une déclaration `prop: value !important`.
#[derive(Debug, Clone, PartialEq)]
pub struct Declaration {
    /// Nom de propriété (tel que source — la casse est traitée à
    /// la cascade).
    pub name: String,
    /// Valeur brute trimée.
    pub value: String,
    /// `!important` présent.
    pub important: bool,
}

/// Une media query minimale : `not?/only? <type>? (and <cond>)*`
/// ou bare `(cond)` — ce qui n'est pas reconnu devient `always`
/// (tolérance : une query non comprise ne doit pas masquer une
/// feuille — spec : `not all` serait le résultat strict, on choisit
/// la forme neutre documentée).
#[derive(Debug, Clone, PartialEq)]
pub struct MediaQuery {
    /// `not` présent.
    pub negated: bool,
    /// Type de média (`screen`, `print`, `all`…) — `None` = implicite.
    pub media_type: Option<String>,
    /// Conditions `(name: value)` jointes par `and`.
    pub conditions: Vec<(String, String)>,
    /// Forme non reconnue → évaluée vraie (tolérance documentée).
    pub unknown: bool,
}

/// Parse une feuille complète — jamais d'erreur fatale : les
/// constructions invalides sont sautées (spec error handling).
#[must_use]
pub fn parse_stylesheet(input: &str) -> Stylesheet {
    let tokens = tokenize(input);
    Parser::new(&tokens).stylesheet()
}

/// Imbrication maximale des règles en bloc (`@media`/`@supports`
/// dans `@media`…) — au-delà, le bloc est avalé comme inconnu.
/// `nested_rules` est récursif : sans borne, une feuille hostile
/// (`@media{a`×10⁵) dépasserait la pile d'appels.
const MAX_NESTING: u32 = 32;

struct Parser<'t> {
    tokens: &'t [Token],
    pos: usize,
    /// Accumulateur texte pour préludes/valeurs.
    buf: String,
    /// Profondeur courante de [`Parser::nested_rules`].
    nest_depth: u32,
}

impl<'t> Parser<'t> {
    fn new(tokens: &'t [Token]) -> Self {
        Self {
            tokens,
            pos: 0,
            buf: String::new(),
            nest_depth: 0,
        }
    }

    fn peek(&self) -> Option<&'t Token> {
        self.tokens.get(self.pos)
    }

    fn next(&mut self) -> Option<&'t Token> {
        let t = self.tokens.get(self.pos);
        if t.is_some() {
            self.pos += 1;
        }
        t
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(Token::Whitespace)) {
            self.pos += 1;
        }
    }

    /// Sérialise des tokens en texte (préludes, valeurs) — espaces
    /// simples entre tokens, ponctuation collée.
    fn token_text(t: &Token) -> String {
        match t {
            Token::Ident(s) | Token::AtKeyword(s) => {
                if matches!(t, Token::AtKeyword(_)) {
                    format!("@{s}")
                } else {
                    s.clone()
                }
            }
            Token::Function(s) => format!("{s}("),
            Token::Hash { value, .. } => format!("#{value}"),
            Token::Str(s) => format!("\"{s}\""),
            Token::BadString => String::from("\"\""),
            Token::Url(s) => format!("url({s})"),
            Token::BadUrl => String::from("url()"),
            Token::Number { repr, .. } => repr.clone(),
            Token::Dimension { repr, unit, .. } => format!("{repr}{unit}"),
            Token::Percentage { repr, .. } => format!("{repr}%"),
            Token::Whitespace => String::from(" "),
            Token::Colon => String::from(":"),
            Token::Semicolon => String::from(";"),
            Token::Comma => String::from(","),
            Token::OpenBracket => String::from("["),
            Token::CloseBracket => String::from("]"),
            Token::OpenBrace => String::from("{"),
            Token::CloseBrace => String::from("}"),
            Token::OpenParen => String::from("("),
            Token::CloseParen => String::from(")"),
            Token::Cdo => String::from("<!--"),
            Token::Cdc => String::from("-->"),
            Token::Delim(c) => c.to_string(),
            Token::Eof => String::new(),
        }
    }

    fn stylesheet(&mut self) -> Stylesheet {
        let mut sheet = Stylesheet::default();
        let mut first = true;
        loop {
            self.skip_ws();
            match self.peek() {
                None => break,
                Some(Token::Cdo | Token::Cdc) => {
                    self.pos += 1;
                }
                Some(Token::AtKeyword(name)) => {
                    let name = name.clone();
                    self.pos += 1;
                    if first && name.eq_ignore_ascii_case("charset") {
                        // `@charset "x";` — lu et rangé.
                        let prelude = self.consume_prelude();
                        self.consume_semicolon();
                        sheet.charset = Some(extract_first_string(&prelude));
                    } else if name.eq_ignore_ascii_case("import") {
                        let rule = self.at_rule(&name);
                        if let Rule::At(at) = &rule {
                            if let AtKind::Statement { prelude } = &at.kind {
                                sheet.imports.push(parse_import(prelude));
                            }
                        }
                        sheet.rules.push(rule);
                    } else {
                        sheet.rules.push(self.at_rule(&name));
                    }
                }
                Some(_) => {
                    let before = self.pos;
                    if let Some(rule) = self.qualified_rule() {
                        sheet.rules.push(rule);
                    } else if self.pos == before {
                        // Aucune progression (token isolé
                        // invalide) : on l'avale pour garantir
                        // la terminaison (contenu hostile).
                        self.pos += 1;
                    }
                }
            }
            first = false;
        }
        sheet
    }

    /// At-rule : prélude jusqu'à `;` ou `{`, puis bloc selon nom.
    fn at_rule(&mut self, name: &str) -> Rule {
        let prelude = self.consume_prelude();
        match self.peek() {
            Some(Token::Semicolon) => {
                self.pos += 1;
                Rule::At(AtRule {
                    name: name.to_ascii_lowercase(),
                    kind: AtKind::Statement { prelude },
                })
            }
            Some(Token::OpenBrace) => {
                self.pos += 1;
                let lname = name.to_ascii_lowercase();
                let kind = match lname.as_str() {
                    "media" | "supports" => {
                        // supports garde prelude brut mais même
                        // structure de bloc que media (règles
                        // imbriquées). Au-delà de MAX_NESTING, le
                        // bloc est avalé comme un at-rule inconnu.
                        let rules = if self.nest_depth >= MAX_NESTING {
                            self.skip_block();
                            Vec::new()
                        } else {
                            self.nest_depth += 1;
                            let r = self.nested_rules();
                            self.nest_depth -= 1;
                            r
                        };
                        AtKind::Media {
                            queries: if lname == "media" {
                                parse_media_queries(&prelude)
                            } else {
                                // supports non évalué ici : query
                                // neutre « unknown » (toujours vrai
                                // à l'évaluation par défaut — la
                                // couche appelante tranchera).
                                vec![MediaQuery {
                                    negated: false,
                                    media_type: None,
                                    conditions: Vec::new(),
                                    unknown: true,
                                }]
                            },
                            rules,
                        }
                    }
                    "font-face" | "page" | "viewport" | "counter-style" => AtKind::Declarations {
                        prelude,
                        declarations: self.declaration_block(),
                    },
                    "keyframes" | "-webkit-keyframes" | "-moz-keyframes" => {
                        let frames = self.keyframes_block();
                        AtKind::Keyframes {
                            name: prelude.trim().to_string(),
                            frames,
                        }
                    }
                    _ => {
                        // Inconnu : on avale le bloc équilibré.
                        self.skip_block();
                        AtKind::UnknownBlock { prelude }
                    }
                };
                Rule::At(AtRule { name: lname, kind })
            }
            _ => Rule::At(AtRule {
                name: name.to_ascii_lowercase(),
                kind: AtKind::Statement { prelude },
            }),
        }
    }

    /// Prélude : texte jusqu'à `;`/`{`/EOF (imbrications `()[]`
    /// suivies — une `)`/`]` orpheline arrête).
    fn consume_prelude(&mut self) -> String {
        self.buf.clear();
        let mut parens = 0i32;
        while let Some(t) = self.peek() {
            match t {
                Token::Semicolon | Token::OpenBrace if parens == 0 => break,
                // `Function` porte son `(` : il ouvre un niveau.
                Token::Function(_) | Token::OpenParen | Token::OpenBracket => {
                    parens += 1;
                    self.buf.push_str(&Self::token_text(t));
                    self.pos += 1;
                }
                Token::CloseParen | Token::CloseBracket => {
                    if parens == 0 {
                        break;
                    }
                    parens -= 1;
                    self.buf.push_str(&Self::token_text(t));
                    self.pos += 1;
                }
                Token::Eof => break,
                _ => {
                    self.buf.push_str(&Self::token_text(t));
                    self.pos += 1;
                }
            }
        }
        std::mem::take(&mut self.buf).trim().to_string()
    }

    /// `{` déjà consommé : lit les règles imbriquées jusqu'au `}`.
    fn nested_rules(&mut self) -> Vec<Rule> {
        let mut rules = Vec::new();
        loop {
            self.skip_ws();
            match self.peek() {
                None | Some(Token::CloseBrace) => {
                    self.pos += usize::from(self.peek().is_some());
                    break;
                }
                Some(Token::AtKeyword(name)) => {
                    let name = name.clone();
                    self.pos += 1;
                    rules.push(self.at_rule(&name));
                }
                Some(_) => {
                    let before = self.pos;
                    if let Some(rule) = self.qualified_rule() {
                        rules.push(rule);
                    } else if self.pos == before {
                        // Anti-boucle : token isolé non traité.
                        self.pos += 1;
                    }
                }
            }
        }
        rules
    }

    /// `{` déjà consommé : déclarations jusqu'au `}`.
    fn declaration_block(&mut self) -> Vec<Declaration> {
        let mut decls = Vec::new();
        loop {
            self.skip_ws();
            match self.peek() {
                None | Some(Token::CloseBrace) => {
                    if self.peek().is_some() {
                        self.pos += 1;
                    }
                    break;
                }
                Some(Token::Semicolon) => {
                    self.pos += 1;
                }
                Some(_) => {
                    if let Some(d) = self.declaration() {
                        decls.push(d);
                    }
                }
            }
        }
        decls
    }

    /// Une déclaration `name : value !important` — erreurs
    /// récupérées par saut jusqu'à `;` ou `}` (spec).
    fn declaration(&mut self) -> Option<Declaration> {
        // Nom : ident attendu (sinon → erreur, saut à `;`/`}`).
        // `peek` et non `next` : le token fautif (`;`, `{`) doit
        // rester visible pour `recover_decl` (le `{` ouvre une
        // profondeur à traverser entièrement).
        self.skip_ws();
        let name = if let Some(Token::Ident(n)) = self.peek() {
            let n = n.clone();
            self.pos += 1;
            n
        } else {
            self.recover_decl();
            return None;
        };
        self.skip_ws();
        if let Some(Token::Colon) = self.peek() {
            self.pos += 1;
        } else {
            self.recover_decl();
            return None;
        }
        // Valeur : tokens jusqu'à `;`/`}` — `!important` en fin.
        let mut parts: Vec<String> = Vec::new();
        let mut parens = 0i32;
        loop {
            match self.peek() {
                None | Some(Token::Semicolon | Token::CloseBrace) if parens == 0 => break,
                // `Function` porte son `(` : il ouvre aussi un niveau.
                Some(Token::Function(_) | Token::OpenParen | Token::OpenBracket) => parens += 1,
                Some(Token::CloseParen | Token::CloseBracket) => {
                    if parens == 0 {
                        break;
                    }
                    parens -= 1;
                }
                _ => {}
            }
            if let Some(t) = self.next() {
                parts.push(Self::token_text(t));
            }
        }
        // Retire le `;` terminal s'il y est.
        if matches!(self.peek(), Some(Token::Semicolon)) {
            self.pos += 1;
        }
        let (value, important) = strip_important(&parts.join(""));
        if value.is_empty() {
            return None;
        }
        Some(Declaration {
            name,
            value,
            important,
        })
    }

    /// Saute jusqu'à `;` ou `}` (sans consommer le `}`).
    fn recover_decl(&mut self) {
        let mut depth = 0i32;
        while let Some(t) = self.peek() {
            match t {
                Token::CloseBrace if depth == 0 => return,
                Token::Semicolon if depth == 0 => {
                    self.pos += 1;
                    return;
                }
                Token::OpenBrace | Token::OpenParen | Token::OpenBracket => depth += 1,
                Token::CloseBrace | Token::CloseParen | Token::CloseBracket if depth > 0 => {
                    depth -= 1;
                }
                _ => {}
            }
            self.pos += 1;
        }
    }

    /// Règle qualifiée : prélude jusqu'à `{` puis déclarations.
    /// Invalide → `None` (le prélude a déjà été sauté).
    fn qualified_rule(&mut self) -> Option<Rule> {
        let prelude = self.consume_prelude();
        if let Some(Token::OpenBrace) = self.peek() {
            self.pos += 1;
            let declarations = self.declaration_block();
            if prelude.is_empty() {
                return None;
            }
            Some(Rule::Qualified(QualifiedRule {
                prelude,
                declarations,
            }))
        } else {
            // Pas de bloc → règle abandonnée (spec) ; le `;`
            // terminal du prélude est consommé.
            if matches!(self.peek(), Some(Token::Semicolon)) {
                self.pos += 1;
            }
            None
        }
    }

    /// Corps `@keyframes` : `<sel> { decls }` répétés.
    fn keyframes_block(&mut self) -> Vec<(String, Vec<Declaration>)> {
        let mut frames = Vec::new();
        loop {
            self.skip_ws();
            match self.peek() {
                None | Some(Token::CloseBrace) => {
                    if self.peek().is_some() {
                        self.pos += 1;
                    }
                    break;
                }
                Some(_) => {
                    let sel = self.consume_prelude();
                    if matches!(self.peek(), Some(Token::OpenBrace)) {
                        self.pos += 1;
                        let decls = self.declaration_block();
                        frames.push((sel, decls));
                    } else {
                        break;
                    }
                }
            }
        }
        frames
    }

    /// Avale un bloc `{ … }` équilibré (`{` déjà consommé).
    fn skip_block(&mut self) {
        let mut depth = 1i32;
        while let Some(t) = self.next() {
            match t {
                Token::OpenBrace => depth += 1,
                Token::CloseBrace => {
                    depth -= 1;
                    if depth == 0 {
                        return;
                    }
                }
                _ => {}
            }
        }
    }

    /// Consomme un `;` immédiat (après @charset).
    fn consume_semicolon(&mut self) {
        self.skip_ws();
        if matches!(self.peek(), Some(Token::Semicolon)) {
            self.pos += 1;
        }
    }
}

/// Extrait la première chaîne quotée d'un prélude (`@charset "x"`).
fn extract_first_string(prelude: &str) -> String {
    prelude
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .to_string()
}

/// Détecte `!important` en fin de valeur (case-insensitive, avec
/// ou sans espace avant `!` et autour de `important`).
fn strip_important(raw: &str) -> (String, bool) {
    let trimmed = raw.trim();
    let lower = trimmed.to_ascii_lowercase();
    for suffix in ["!important", "! important"] {
        if let Some(head) = lower.strip_suffix(suffix) {
            return (head.trim_end().to_string(), true);
        }
    }
    (trimmed.to_string(), false)
}

/// `@import "u" screen and (x)` → url + media brut.
fn parse_import(prelude: &str) -> ImportRule {
    let p = prelude.trim();
    let (url, rest) = if let Some(q) = p.strip_prefix("url(") {
        let end = q.find(')').unwrap_or(q.len());
        (
            q[..end]
                .trim()
                .trim_matches('"')
                .trim_matches('\'')
                .to_string(),
            q[end..].trim_start_matches(')').trim().to_string(),
        )
    } else if let Some(q) = p.strip_prefix('"').or_else(|| p.strip_prefix('\'')) {
        let quote = p.chars().next().unwrap_or('"');
        let end = q.find(quote).unwrap_or(q.len());
        (
            q[..end].to_string(),
            q[end..].trim_start_matches(quote).trim().to_string(),
        )
    } else {
        (p.to_string(), String::new())
    };
    ImportRule { url, media: rest }
}

/// Parse une liste de media queries séparées par `,`.
fn parse_media_queries(prelude: &str) -> Vec<MediaQuery> {
    prelude
        .split(',')
        .map(|q| parse_one_media(q.trim()))
        .collect()
}

/// Une query : `[not|only] [type] [and (cond)]*` ou `(cond)…`.
fn parse_one_media(q: &str) -> MediaQuery {
    let mut mq = MediaQuery {
        negated: false,
        media_type: None,
        conditions: Vec::new(),
        unknown: false,
    };
    let mut rest = q.trim();
    if rest.is_empty() {
        mq.unknown = true;
        return mq;
    }
    // Modifieur initial.
    for (word, neg) in [("not", true), ("only", false)] {
        if let Some(r) = rest
            .strip_prefix(word)
            .filter(|r| r.starts_with(' ') || r.starts_with('(') || r.is_empty())
        {
            mq.negated = neg;
            rest = r.trim_start();
            break;
        }
    }
    // Itère les morceaux séparés par `and` — tout ce qui ne
    // s'analyse pas marque la query `unknown` (tolérance).
    let mut first = true;
    loop {
        if first {
            first = false;
        } else if rest.get(..3).is_some_and(|w| w.eq_ignore_ascii_case("and"))
            && rest.get(3..4).is_some_and(|c| c == " " || c == "(")
        {
            rest = rest[3..].trim_start();
        } else {
            mq.unknown = !rest.is_empty();
            break;
        }
        if rest.is_empty() {
            break;
        }
        if let Some(inner) = rest.strip_prefix('(') {
            // Condition `(name : value)` ou `(name)`.
            let depth_end = find_matching_paren(rest);
            let inner = &inner[..depth_end.min(inner.len())];
            let inner_trim = inner.trim_end_matches(')').trim();
            match inner_trim.split_once(':') {
                Some((n, v)) => mq
                    .conditions
                    .push((n.trim().to_ascii_lowercase(), v.trim().to_string())),
                None => mq
                    .conditions
                    .push((inner_trim.to_ascii_lowercase(), String::new())),
            }
            rest = rest[depth_end + 1..].trim_start();
        } else {
            // Type de média (premier mot non-parenthèse).
            let word_end = rest
                .find(|c: char| c.is_whitespace() || c == '(')
                .unwrap_or(rest.len());
            let word = &rest[..word_end];
            if mq.media_type.is_none() && !word.is_empty() {
                mq.media_type = Some(word.to_ascii_lowercase());
            } else {
                mq.unknown = true;
            }
            rest = rest[word_end..].trim_start();
        }
    }
    mq
}

/// Index de la `)` qui ferme la `(` en tête de `s` (`s[0]` == '(').
fn find_matching_paren(s: &str) -> usize {
    let mut depth = 0i32;
    for (i, c) in s.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return i;
                }
            }
            _ => {}
        }
    }
    s.len()
}
