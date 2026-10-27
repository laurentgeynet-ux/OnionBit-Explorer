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

use std::fmt::Write;

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
    /// `@container <name>? <condition> { rules }` — css-contain-3
    /// (R-122 — ×27 Tailwind) : query brute stockée, règles
    /// collectées ; évaluées en seconde passe par la couche
    /// appelante (taille du conteneur inconnue à la cascade).
    Container {
        /// Nom du conteneur ciblé (`container-name`), si donné.
        name: Option<String>,
        /// Condition brute (`(min-width: 400px)`, `(… and …)`).
        query: String,
        /// Règles imbriquées.
        rules: Vec<Rule>,
    },
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
    /// `@layer <nom>? { rules }` — cascade layers (CSS
    /// Cascading 5 §6.4). `name = None` pour le layer anonyme ;
    /// le prélude peut être pointé (`a.b` = sous-couche de `a`).
    /// La forme sans bloc `@layer a, b;` reste
    /// [`AtKind::Statement`] — elle déclare l'ordre, sans règles.
    Layer {
        /// Nom du layer (`None` = anonyme).
        name: Option<String>,
        /// Règles imbriquées.
        rules: Vec<Rule>,
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

/// Parse une liste de déclarations nues — attribut HTML
/// `style="…"` (spec : même grammaire qu'un bloc de
/// déclarations, sans les accolades). Récupération d'erreur
/// identique aux feuilles : une déclaration fautive est
/// sautée, jamais fatale.
#[must_use]
pub fn parse_declarations(input: &str) -> Vec<Declaration> {
    let tokens = tokenize(input);
    let mut p = Parser::new(&tokens);
    let mut out = Vec::new();
    while p.peek().is_some() {
        let before = p.pos;
        if let Some(d) = p.declaration() {
            out.push(d);
        }
        // Aucun progrès (token non-consommé par la
        // récupération) → on avance d'un cran, jamais boucle.
        if p.pos == before {
            p.pos += 1;
        }
    }
    out
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
    /// Sérialise un ident pour reconstruction de prélude
    /// (CSS Syntax « serialize an identifier ») : tout char
    /// ne tenant pas littéralement est ré-échappé. Sans ça,
    /// un ident né d'un échappement (`second\ two` →
    /// `second two`) ressortirait brut et rendrait le
    /// sélecteur stocké invalide au re-parse (Acid2
    /// `[class=second\ two]`). Forme `\c` pour les chars
    /// sûrs, `\hh ` hexadécimale pour chiffres/contrôles
    /// (`\1` serait relu comme l'escape U+0001, pas `1`).
    fn serialize_ident(s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        for (i, c) in s.chars().enumerate() {
            let first_needs_hex = i == 0 && c.is_ascii_digit();
            let after_dash_needs_hex = i == 1 && s.starts_with('-') && c.is_ascii_digit();
            let bare_ok = if i == 0 {
                (c.is_ascii_alphabetic() || c == '_' || !c.is_ascii() || c == '-')
                    && !(c == '-' && s.chars().count() == 1)
            } else {
                c.is_ascii_alphanumeric() || c == '_' || c == '-' || !c.is_ascii()
            };
            if c == '\0' {
                out.push('\u{FFFD}');
            } else if first_needs_hex || after_dash_needs_hex {
                let _ = write!(out, "\\{:x} ", c as u32);
            } else if bare_ok {
                out.push(c);
            } else if c.is_ascii_alphanumeric() || (!c.is_ascii_graphic() && c != ' ') {
                // `\c` serait relu en escape hex (a-f0-9) ou
                // serait invalide (contrôle) → forme hex.
                let _ = write!(out, "\\{:x} ", c as u32);
            } else {
                out.push('\\');
                out.push(c);
            }
        }
        out
    }

    /// Sérialise le contenu d'un `Str` — `"` et `\`
    /// internes ré-échappés, newline en `\a `.
    fn serialize_str(s: &str) -> String {
        let mut out = String::with_capacity(s.len() + 2);
        out.push('"');
        for c in s.chars() {
            match c {
                '"' | '\\' => {
                    out.push('\\');
                    out.push(c);
                }
                '\n' | '\r' | '\x0C' => {
                    let _ = write!(out, "\\{:x} ", c as u32);
                }
                _ => out.push(c),
            }
        }
        out.push('"');
        out
    }

    fn token_text(t: &Token) -> String {
        match t {
            Token::Ident(s) => Self::serialize_ident(s),
            Token::AtKeyword(s) => format!("@{}", Self::serialize_ident(s)),
            Token::Function(s) => format!("{}(", Self::serialize_ident(s)),
            Token::Hash { value, .. } => {
                // Un hash n'est pas un ident : les digits en
                // tête restent nus (`#000`, pas `#\30 00`) ;
                // échappement seulement si un caractère l'exige.
                if !value.is_empty()
                    && value
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || !c.is_ascii())
                {
                    format!("#{value}")
                } else {
                    format!("#{}", Self::serialize_ident(value))
                }
            }
            Token::Str(s) => Self::serialize_str(s),
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
                        let prelude = self.consume_prelude(true);
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
    #[allow(clippy::too_many_lines)] // match des at-rules — borné par le grammar dispatch
    fn at_rule(&mut self, name: &str) -> Rule {
        let prelude = self.consume_prelude(true);
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
                    "container" => {
                        // `@container [name] (condition) { … }` —
                        // même grammaire de bloc que `@media`.
                        let rules = if self.nest_depth >= MAX_NESTING {
                            self.skip_block();
                            Vec::new()
                        } else {
                            self.nest_depth += 1;
                            let r = self.nested_rules();
                            self.nest_depth -= 1;
                            r
                        };
                        // Prélude : `<name>? (<query…>)` — le
                        // nom est le token libre avant la parenthèse.
                        let prelude_t = prelude.trim();
                        let (name, query) = match prelude_t.find('(') {
                            Some(i) => {
                                let n = prelude_t[..i].trim().to_string();
                                (
                                    (!n.is_empty()).then_some(n),
                                    prelude_t[i..].trim().to_string(),
                                )
                            }
                            None => (None, prelude_t.to_string()),
                        };
                        AtKind::Container { name, query, rules }
                    }
                    "layer" => {
                        // Bloc de règles, même grammaire que
                        // `@media` (règles qualifiées, at-rules
                        // imbriquées dont `@layer`/`@media`).
                        let rules = if self.nest_depth >= MAX_NESTING {
                            self.skip_block();
                            Vec::new()
                        } else {
                            self.nest_depth += 1;
                            let r = self.nested_rules();
                            self.nest_depth -= 1;
                            r
                        };
                        let name = prelude.trim();
                        AtKind::Layer {
                            name: if name.is_empty() {
                                None
                            } else {
                                Some(name.to_string())
                            },
                            rules,
                        }
                    }
                    "font-face" | "page" | "viewport" | "counter-style" | "property" => {
                        AtKind::Declarations {
                            prelude,
                            declarations: self.declaration_block(),
                        }
                    }
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
    /// `semicolon_ends` : pour les at-rules (`@import x;`) le
    /// `;` borne le prélude. Pour une règle **qualifiée**, CSS
    /// Syntax §5.4.4 le consomme *dans* le prélude — un `;`
    /// orphelin devient donc un prélude invalide qui emporte
    /// la règle entière (`.a{} ; .b{}` → `.b` tombe, cas Acid2
    /// `.parser { height: 3em }` avalé).
    fn consume_prelude(&mut self, semicolon_ends: bool) -> String {
        self.buf.clear();
        let mut parens = 0i32;
        while let Some(t) = self.peek() {
            match t {
                Token::Semicolon if parens == 0 && semicolon_ends => break,
                Token::OpenBrace if parens == 0 => break,
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
                // EOF clos la déclaration même parenthèses
                // ouvertes (§B-50 — `translateX(0px;` jamais
                // fermé par un `style=` muté JS bouclait sans
                // consommer : `next()` → None, progrès nul).
                None => break,
                Some(Token::Semicolon | Token::CloseBrace) if parens == 0 => break,
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
        // Une propriété custom `--*` accepte la valeur vide
        // (css-variables-1 : `<declaration-value>` peut n'être
        // qu'un token whitespace — stream vide valide ; MDN
        // s'en sert pour basculer son toggle `light-dark` en
        // contexte sombre, R-80). Les autres propriétés à
        // valeur vide restent des déclarations mortes.
        if (value.is_empty() && !name.starts_with("--")) || has_stray_bang(&value) {
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
        let prelude = self.consume_prelude(false);
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
            // Pas de bloc → règle abandonnée (le prélude court
            // jusqu'à `{` ou EOF — `;` ne le borne plus).
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
                    let sel = self.consume_prelude(true);
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
/// Un `!` hors chaînes/parenthèses qui n'a pas été absorbé en
/// suffixe `!important` invalide la déclaration entière
/// (`border: 5em solid red ! error` → drop, spec).
fn has_stray_bang(value: &str) -> bool {
    let mut depth = 0i32;
    let mut quote = '\0';
    for c in value.chars() {
        if quote != '\0' {
            if c == quote {
                quote = '\0';
            }
            continue;
        }
        match c {
            '"' | '\'' => quote = c,
            '(' | '[' => depth += 1,
            ')' | ']' => depth = depth.saturating_sub(1),
            '!' if depth == 0 => return true,
            _ => {}
        }
    }
    false
}

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
            // Condition `(name : value)` ou `(name)` — ou
            // `(not (…))`, facteur nié MQ4 (R-64, corpus
            // lobste.rs `dark-normal`).
            let depth_end = find_matching_paren(rest);
            let inner = &inner[..depth_end.min(inner.len())];
            let inner_trim = inner.trim_end_matches(')').trim();
            let negated = inner_trim
                .get(..3)
                .is_some_and(|w| w.eq_ignore_ascii_case("not"))
                && inner_trim.get(3..4).is_some_and(|c| c == " " || c == "(");
            if negated {
                // `(not (f:v))` ou `(not f:v)` toléré — le
                // `not` interne est porté par le nom de
                // condition (`not-<feature>`, évalué nié).
                let n_inner = inner_trim[3..].trim();
                let n_trim = n_inner
                    .strip_prefix('(')
                    .map_or(n_inner, |s| s.trim_end_matches(')'))
                    .trim();
                match parse_media_condition(n_trim).as_slice() {
                    [(n, v)] => mq.conditions.push((format!("not-{n}"), v.clone())),
                    // Corps multi-conditions (double borne
                    // range, `and` interne) : non exprimable
                    // en conditions jointes par AND — nom
                    // inconnu → « feature absente » → nié vrai
                    // (tolérance documentée, cas rare).
                    _ => mq.conditions.push((format!("not-{n_trim}"), String::new())),
                }
            } else {
                mq.conditions.extend(parse_media_condition(inner_trim));
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

/// Une condition `(name : value)`, `(name)` ou forme range
/// MQ4 — normalisée en paires `(name, value)` que
/// `condition_matches` évalue (ranges → `min-/max-/gt-/lt-`).
fn parse_media_condition(inner_trim: &str) -> Vec<(String, String)> {
    match inner_trim.split_once(':') {
        Some((n, v)) => vec![(n.trim().to_ascii_lowercase(), v.trim().to_string())],
        None => match parse_range_condition(inner_trim) {
            Some(conds) => conds,
            None => vec![(inner_trim.to_ascii_lowercase(), String::new())],
        },
    }
}

/// Forme range MQ4 `(feature <op> valeur)` ou
/// `(valeur <op> feature <op> valeur)` — normalisée en
/// conditions `(min-/max-/gt-/lt-)` que `condition_matches`
/// évalue. `width >= N` ≡ `min-width`, `width <= N` ≡
/// `max-width` ; `>`/`<` stricts ont leurs noms propres
/// (`gt-*`/`lt-*` — la borne exacte ne match pas).
/// `None` = pas une range → le texte brut reste un nom de
/// condition inconnu (évalué faux — tolérance existante).
fn parse_range_condition(inner: &str) -> Option<Vec<(String, String)>> {
    // Découpe en morceaux non-espace, chacun tranché sur les
    // transitions opérateur/valeur (`width<=1044px` collé est
    // légal), puis fusionne les runs d'opérateurs adjacents
    // (`<` `=` espacés → `<=`).
    let is_op = |s: &str| s.chars().all(|c| matches!(c, '<' | '>' | '='));
    let mut toks: Vec<String> = Vec::new();
    let drain = |toks: &mut Vec<String>, cur: &mut String| {
        if cur.is_empty() {
            return;
        }
        if is_op(cur) && toks.last().is_some_and(|p| is_op(p)) {
            toks.last_mut().unwrap().push_str(cur);
        } else {
            toks.push(std::mem::take(cur));
        }
        cur.clear();
    };
    for piece in inner.split_whitespace() {
        let mut cur = String::new();
        let mut in_operator = piece
            .chars()
            .next()
            .is_some_and(|c| matches!(c, '<' | '>' | '='));
        for c in piece.chars() {
            if matches!(c, '<' | '>' | '=') != in_operator {
                drain(&mut toks, &mut cur);
                in_operator = !in_operator;
            }
            cur.push(c);
        }
        drain(&mut toks, &mut cur);
    }
    let is_name = |t: &String| t.chars().next().is_some_and(char::is_alphabetic);
    let rev = |op: &str| -> &'static str {
        match op {
            ">" => "<",
            "<" => ">",
            ">=" => "<=",
            "<=" => ">=",
            _ => "",
        }
    };
    let cond_of = |name: &str, op: &str, val: &str| -> Option<(String, String)> {
        let n = name.to_ascii_lowercase();
        let v = val.trim().to_string();
        match op {
            ">=" => Some((format!("min-{n}"), v)),
            "<=" => Some((format!("max-{n}"), v)),
            ">" => Some((format!("gt-{n}"), v)),
            "<" => Some((format!("lt-{n}"), v)),
            "=" => Some((n, v)),
            _ => None,
        }
    };
    match toks.as_slice() {
        // `(width >= 1044px)` / `(width = x)` / `(1044px <= width)`
        [a, op, b] if is_name(a) => cond_of(a, op, b).map(|c| vec![c]),
        [a, op, b] if is_name(b) => cond_of(b, rev(op), a).map(|c| vec![c]),
        // `(400px <= width <= 1044px)` — les deux bornes.
        [v1, o1, n, o2, v2] if is_name(n) => match (cond_of(n, rev(o1), v1), cond_of(n, o2, v2)) {
            (Some(a), Some(b)) => Some(vec![a, b]),
            _ => None,
        },
        _ => None,
    }
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
