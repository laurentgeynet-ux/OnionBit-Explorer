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
    /// R-229 — `@font-feature-values <fam>` : valeurs
    /// nommées `(kind, nom, indices)` consommées par
    /// `font-variant-alternates` (css-fonts-4 §6).
    FontFeatureValues {
        /// Famille de police déclarée.
        family: String,
        /// `(kind, nom, indices)` ex : `("stylistic",
        /// "salt-style", [1, 3])`.
        features: Vec<(String, String, Vec<u32>)>,
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
    /// `@scope (<root>) [to (<borne>)] { rules }` — css-cascade-6
    /// (R-194) : les règles sont **désucrées au parse** — chaque
    /// prélude `S` devient `R S` (`:scope` → `:is(R)`), et une
    /// borne `to (B)` ajoute `:not(B *)` — la cascade itère le
    /// contenu comme un groupe ordinaire.
    Scope {
        /// Règles désucrées (préludes déjà préfixés).
        rules: Vec<Rule>,
    },
    /// `@starting-style { rules }` — styles initiaux de
    /// transition/transitions d'entrée (css-transitions-2).
    /// R-244 : parsé mais dormant — les règles NE SONT PAS
    /// appliquées (elles ne valent qu'à la première pose,
    /// consommateur futur).
    StartingStyle {
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
    /// Conditions `(name: value)` jointes par `and` —
    /// premier groupe.
    pub conditions: Vec<(String, String)>,
    /// Groupes AND supplémentaires joints par `or` (MQ4 —
    /// `(a) or (b) or (c)`). La query matche si
    /// `conditions` ou un des groupes matche entièrement.
    pub or_groups: Vec<Vec<(String, String)>>,
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
                    sheet.rules.extend(self.qualified_rule());
                    if self.pos == before {
                        // Aucune progression (token isolé
                        // invalide) : on l'avale pour garantir
                        // la terminaison (contenu hostile).
                        self.pos += 1;
                        explorer_common::census::bump("css.token.orphan");
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
                                Self::parse_supports_queries(&prelude)
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
                    "scope" => {
                        // `@scope (R) [to (B)] { … }` — R-194 :
                        // règles parsées puis préludes
                        // réécrits `R S` (+ `:not(B *)`).
                        let mut rules = if self.nest_depth >= MAX_NESTING {
                            self.skip_block();
                            Vec::new()
                        } else {
                            self.nest_depth += 1;
                            let r = self.nested_rules();
                            self.nest_depth -= 1;
                            r
                        };
                        let (root, bound) = parse_scope_prelude(&prelude);
                        scope_wrap_rules(&mut rules, &root, bound.as_deref());
                        AtKind::Scope { rules }
                    }
                    "font-face" | "page" | "viewport" | "counter-style" | "property"
                    | "font-palette-values" | "position-try" | "view-transition"
                    | "function" => {
                        AtKind::Declarations {
                            prelude,
                            declarations: self.declaration_block(),
                        }
                    }
                    // R-244 — `@starting-style` : container
                    // de règles dormant (consommateur futur —
                    // animations d'entrée).
                    "starting-style" => {
                        let rules = if self.nest_depth >= MAX_NESTING {
                            self.skip_block();
                            Vec::new()
                        } else {
                            self.nest_depth += 1;
                            let r = self.nested_rules();
                            self.nest_depth -= 1;
                            r
                        };
                        AtKind::StartingStyle { rules }
                    }
                    // R-229 — `@font-feature-values <fam> {
                    // @<kind> { <nom>: <idx>+ } }` : corps de
                    // sous-at-rules, pas de déclarations.
                    "font-feature-values" => AtKind::FontFeatureValues {
                        family: prelude.trim().to_string(),
                        features: self.font_feature_block(),
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
                        // R-207 — le bloc n'est même pas
                        // structuré (`@starting-style`,
                        // `@position-try`, at-rules futures…).
                        explorer_common::census::bump(&format!("css.at.unknown.{lname}"));
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
    /// `@supports` → `MediaQuery` encodé `media_type:
    /// Some("supports")` : le prelude « `or`-séparé »
    /// devient N requêtes (any), chaque `and`-chaîne ses
    /// conditions `(prop: value)` (all), `not` global →
    /// `negated` ; `not (prop:val)` interne → `not-<prop>`
    /// (même convention que les features niées des MQ).
    /// Évaluation dans `query_matches` — une condition
    /// supports = « la déclaration est supportée ».
    fn parse_supports_queries(prelude: &str) -> Vec<MediaQuery> {
        let mut queries = Vec::new();
        let norm = prelude
            .split_ascii_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_ascii_lowercase();
        for or_part in norm.split(" or ") {
            let or_part = or_part.trim();
            if or_part.is_empty() {
                continue;
            }
            let (negated, body) = or_part
                .strip_prefix("not ")
                .map_or((false, or_part), |b| (true, b.trim()));
            let mut conditions = Vec::new();
            for cond in body.split(" and ") {
                let c = cond.trim();
                // `(not (prop:val))` → `not-<prop>`.
                let (inner, not) = c
                    .strip_prefix("(not ")
                    .and_then(|r| r.strip_suffix(')'))
                    .map_or((c, false), |r| (r.trim(), true));
                // Dépile les parens externes jusqu'au couple
                // `prop:value` — `((display:grid))` = parens
                // redondantes autorisées par la grammaire.
                let mut inner = inner.trim();
                while inner.starts_with('(') && inner.ends_with(')') {
                    inner = inner[1..inner.len() - 1].trim();
                }
                let Some(colon) = inner.find(':') else {
                    continue;
                };
                let prop = inner[..colon].trim();
                let value = inner[colon + 1..].trim();
                if prop.is_empty() || value.is_empty() {
                    continue;
                }
                let key = if not {
                    format!("not-{prop}")
                } else {
                    prop.to_string()
                };
                conditions.push((key, value.to_string()));
            }
            if !conditions.is_empty() || negated {
                queries.push(MediaQuery {
                    negated,
                    media_type: Some("supports".to_string()),
                    conditions,
                    or_groups: Vec::new(),
                    unknown: false,
                });
            }
        }
        if queries.is_empty() {
            // Prelude inintelligible → tolérance neutre (vrai).
            // R-207 — même signal que les media queries : un
            // `@supports` incompris peut masquer une condition
            // moderne (`selector()`, `font-tech()`…).
            explorer_common::census::bump("css.supports.unknown");
            queries.push(MediaQuery {
                negated: false,
                media_type: None,
                conditions: Vec::new(),
                or_groups: Vec::new(),
                unknown: true,
            });
        }
        queries
    }

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
                    rules.extend(self.qualified_rule());
                    if self.pos == before {
                        // Anti-boucle : token isolé non traité.
                        self.pos += 1;
                        explorer_common::census::bump("css.token.orphan");
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

    /// Corps `@font-feature-values` : suite de
    /// `@<kind> { <nom>: <idx>+ ; … }` (css-fonts-4 §6).
    /// Tolérant : un bloc fautif saute à `;`/`}`.
    fn font_feature_block(&mut self) -> Vec<(String, String, Vec<u32>)> {
        let mut out = Vec::new();
        loop {
            self.skip_ws();
            match self.peek() {
                None | Some(Token::CloseBrace) => {
                    if self.peek().is_some() {
                        self.pos += 1;
                    }
                    break;
                }
                Some(Token::AtKeyword(kind)) => {
                    let kind = kind.clone();
                    self.pos += 1;
                    self.skip_ws();
                    // `@kind { ... }` — le `{` doit être
                    // consommé avant `declaration_block`
                    // (sinon il part en récupération).
                    if matches!(self.peek(), Some(Token::OpenBrace)) {
                        self.pos += 1;
                    }
                    for d in self.declaration_block() {
                        let idxs: Vec<u32> = d
                            .value
                            .split_whitespace()
                            .filter_map(|t| t.trim_end_matches(';').parse::<u32>().ok())
                            .collect();
                        if !idxs.is_empty() {
                            out.push((kind.clone(), d.name, idxs));
                        }
                    }
                }
                Some(_) => {
                    self.recover_decl();
                }
            }
        }
        out
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

    /// R-189 — l'item courant est-il une **règle** plutôt
    /// qu'une déclaration ? Lookahead non consommant : un
    /// `{` à profondeur-0 atteint avant `;`/`}` signale un
    /// bloc imbriqué (css-nesting-1 — `a:hover {}` n'est pas
    /// une déclaration, `x: y` n'atteint jamais `{`).
    fn looks_like_rule(&self) -> bool {
        let mut depth = 0i32;
        let mut i = self.pos;
        loop {
            match self.tokens.get(i) {
                None | Some(Token::Eof) => return false,
                Some(Token::OpenBrace) if depth == 0 => return true,
                Some(Token::Semicolon | Token::CloseBrace) if depth == 0 => return false,
                Some(
                    Token::OpenBrace | Token::OpenParen | Token::OpenBracket | Token::Function(_),
                ) => {
                    depth += 1;
                }
                Some(Token::CloseBrace | Token::CloseParen | Token::CloseBracket) if depth > 0 => {
                    depth -= 1;
                }
                _ => {}
            }
            i += 1;
        }
    }

    /// Bloc d'une règle **de style** (`{` déjà consommé) :
    /// dispatch déclaration / règle imbriquée / at-rule
    /// imbriquée (css-nesting-1, R-189). Chaque run de
    /// déclarations devient une [`QualifiedRule`] plate pour
    /// `sel` émise à sa position source — une déclaration
    /// après une règle imbriquée est ainsi soulevée en règle
    /// `&` implicite (spec : l'ordre source est conservé).
    /// Règles **désucrées** : l'aval ne voit que des règles
    /// plates.
    fn style_block(&mut self, sel: &str) -> Vec<Rule> {
        let mut out: Vec<Rule> = Vec::new();
        let mut decls: Vec<Declaration> = Vec::new();
        // Run de déclarations → règle plate ordonnée.
        macro_rules! flush {
            () => {
                if !decls.is_empty() {
                    out.push(Rule::Qualified(QualifiedRule {
                        prelude: sel.to_string(),
                        declarations: std::mem::take(&mut decls),
                    }));
                }
            };
        }
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
                Some(Token::AtKeyword(name)) => {
                    let name = name.clone();
                    self.pos += 1;
                    flush!();
                    out.push(self.at_rule_in_block(&name, sel));
                }
                Some(_) => {
                    if self.looks_like_rule() {
                        flush!();
                        out.extend(self.nested_qualified(sel));
                    } else if let Some(d) = self.declaration() {
                        decls.push(d);
                    }
                }
            }
        }
        flush!();
        out
    }

    /// R-189 — règle qualifiée imbriquée : prélude jusqu'à
    /// `{`, `&` substitué par `:is(<parent>)` (ou `&` implicite
    /// préfixé — descendant — pour les sélecteurs nus), puis
    /// bloc récursé avec le sélecteur **substitué** comme
    /// nouveau parent. Déjà `nest_depth`-borné à l'appel.
    fn nested_qualified(&mut self, parent_sel: &str) -> Vec<Rule> {
        let prelude = self.consume_prelude(false);
        if !matches!(self.peek(), Some(Token::OpenBrace)) {
            return Vec::new();
        }
        self.pos += 1;
        let eff = substitute_ampersand(&prelude, parent_sel);
        if self.nest_depth >= MAX_NESTING {
            self.skip_block();
            return Vec::new();
        }
        self.nest_depth += 1;
        let rules = self.style_block(&eff);
        self.nest_depth -= 1;
        rules
    }

    /// R-189 — at-rule **dans un bloc de style** : son corps
    /// contient des block-items (déclarations + règles
    /// imbriquées liées au `parent_sel`), pas des règles
    /// top-level — variante de [`Parser::at_rule`] pour ce
    /// contexte (`@media`/`@supports`/`@container`/`@layer` ;
    /// les autres corps sont avalés comme inconnus).
    fn at_rule_in_block(&mut self, name: &str, parent_sel: &str) -> Rule {
        let prelude = self.consume_prelude(true);
        let lname = name.to_ascii_lowercase();
        if !matches!(self.peek(), Some(Token::OpenBrace)) {
            self.consume_semicolon();
            return Rule::At(AtRule {
                name: lname,
                kind: AtKind::Statement { prelude },
            });
        }
        self.pos += 1;
        // Corps → règles plates liées au parent, borné.
        let rules = if self.nest_depth >= MAX_NESTING {
            self.skip_block();
            Vec::new()
        } else {
            self.nest_depth += 1;
            let r = self.style_block(parent_sel);
            self.nest_depth -= 1;
            r
        };
        let kind = match lname.as_str() {
            "media" | "supports" => AtKind::Media {
                queries: if lname == "media" {
                    parse_media_queries(&prelude)
                } else {
                    Self::parse_supports_queries(&prelude)
                },
                rules,
            },
            "container" => {
                let prelude_t = prelude.trim();
                let (cname, query) = match prelude_t.find('(') {
                    Some(i) => {
                        let n = prelude_t[..i].trim().to_string();
                        (
                            (!n.is_empty()).then_some(n),
                            prelude_t[i..].trim().to_string(),
                        )
                    }
                    None => (None, prelude_t.to_string()),
                };
                AtKind::Container {
                    name: cname,
                    query,
                    rules,
                }
            }
            "layer" => {
                let n = prelude.trim();
                AtKind::Layer {
                    name: (!n.is_empty()).then(|| n.to_string()),
                    rules,
                }
            }
            _ => AtKind::UnknownBlock { prelude },
        };
        Rule::At(AtRule { name: lname, kind })
    }

    /// Saute jusqu'à `;` ou `}` (sans consommer le `}`).
    fn recover_decl(&mut self) {
        // R-207 — déclaration hors grammaire (nom non-ident ou
        // `:` manquant) : récupération spec, signalée au census.
        // R-218 : détail par token fautif — le Pareto nomme
        // la forme (`.name.<ident>` si c'est un nom, sinon le
        // kind de token) au lieu d'un compteur muet.
        explorer_common::census::bump("css.decl.malformed");
        let detail = match self.peek() {
            Some(Token::Ident(n)) => format!("css.decl.malformed.name.{n}"),
            Some(t) => {
                let kind = format!("{t:?}");
                format!(
                    "css.decl.malformed.{}",
                    kind.split('(').next().unwrap_or("token")
                )
            }
            None => "css.decl.malformed.eof".to_string(),
        };
        explorer_common::census::bump(&detail);
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

    /// Règle qualifiée : prélude jusqu'à `{` puis bloc —
    /// R-189 : le corps peut contenir des **règles imbriquées**
    /// (css-nesting-1) désucrées en règles plates ordonnées ;
    /// retourne la liste complète (la règle propre + ses
    /// expansions). Invalide → liste vide.
    fn qualified_rule(&mut self) -> Vec<Rule> {
        let prelude = self.consume_prelude(false);
        if let Some(Token::OpenBrace) = self.peek() {
            self.pos += 1;
            if prelude.is_empty() {
                self.skip_block();
                return Vec::new();
            }
            if self.nest_depth >= MAX_NESTING {
                self.skip_block();
                return Vec::new();
            }
            self.nest_depth += 1;
            let rules = self.style_block(&prelude);
            self.nest_depth -= 1;
            rules
        } else {
            // Pas de bloc → règle abandonnée (le prélude court
            // jusqu'à `{` ou EOF — `;` ne le borne plus).
            Vec::new()
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
/// R-189 — coupe un prélude sélecteur sur les virgules de
/// profondeur-0 : `(`/`)`, `[`/`]`, chaînes quotées et
/// échappements `\` sont ignorés (`:not(.a, .b)` reste un
/// seul segment).
fn split_top_commas(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let (mut depth, mut quote, mut esc, mut start) = (0i32, '\0', false, 0usize);
    for (i, c) in s.char_indices() {
        if esc {
            esc = false;
            continue;
        }
        match c {
            '\\' => esc = true,
            '"' | '\'' if quote == '\0' => quote = c,
            c if c == quote => quote = '\0',
            '(' | '[' if quote == '\0' => depth += 1,
            ')' | ']' if quote == '\0' && depth > 0 => depth -= 1,
            ',' if quote == '\0' && depth == 0 => {
                out.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&s[start..]);
    out
}

/// R-189 — substitution `&` d'un prélude imbriqué dans son
/// parent (css-nesting-1) : chaque `&` (hors chaînes et
/// échappements) devient `:is(<partie parent>)` — wrap
/// `:is` = sémantique exacte de spécificité/matching, et
/// couvre `div&`, `:not(&)`, `&&`. Une partie sans `&`
/// reçoit le parent en descendant (`& ` implicite — le
/// combinateur initial `>`/`+`/`~` est préservé). Listes
/// parent × imbriqué → produit cartésien, borné pour les
/// préludes hostiles.
const MAX_NEST_PARTS: usize = 64;

fn substitute_ampersand(nested: &str, parent: &str) -> String {
    /// Remplace les `&` hors chaînes/échappements par `rep`.
    fn replace_amp(s: &str, rep: &str) -> String {
        let mut out = String::with_capacity(s.len() + rep.len());
        let (mut quote, mut esc) = ('\0', false);
        for c in s.chars() {
            if esc {
                out.push(c);
                esc = false;
                continue;
            }
            match c {
                '\\' => {
                    out.push(c);
                    esc = true;
                }
                '"' | '\'' if quote == '\0' => {
                    quote = c;
                    out.push(c);
                }
                c if c == quote => {
                    quote = '\0';
                    out.push(c);
                }
                '&' if quote == '\0' => out.push_str(rep),
                _ => out.push(c),
            }
        }
        out
    }
    let parents = split_top_commas(parent);
    let mut out: Vec<String> = Vec::new();
    'outer: for n in split_top_commas(nested) {
        let n = n.trim();
        if n.is_empty() {
            continue;
        }
        for p in &parents {
            let p = p.trim();
            if p.is_empty() {
                continue;
            }
            let part = if n.contains('&') {
                replace_amp(n, &format!(":is({p})"))
            } else {
                // Nu → `& ` implicite (descendant — un
                // combinateur initial `>`/`+`/`~` suit le
                // parent tel quel).
                format!("{p} {n}")
            };
            out.push(part);
            if out.len() >= MAX_NEST_PARTS {
                break 'outer;
            }
        }
    }
    out.join(", ")
}

/// R-194 — `@scope (R) [to (B)]` : extrait la racine `R` et
/// la borne `B` du prélude (parenthèses suivies ; préfixe
/// `(…)` requis — sinon tout le prélude hors `to (…)` fait
/// office de racine, tolérance malformée).
fn parse_scope_prelude(prelude: &str) -> (String, Option<String>) {
    /// Coupe un `(` en tête : (contenu interne, reste après `)`).
    fn take_paren(s: &str) -> Option<(&str, &str)> {
        let inner = s.strip_prefix('(')?;
        let mut depth = 1i32;
        for (i, c) in inner.char_indices() {
            match c {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some((&inner[..i], &inner[i + 1..]));
                    }
                }
                _ => {}
            }
        }
        None
    }
    let p = prelude.trim();
    let (root, rest) = match take_paren(p) {
        Some((inner, rest)) => (inner.trim().to_string(), rest.trim()),
        None => (p.to_string(), ""),
    };
    let bound = rest
        .strip_prefix("to")
        .map(str::trim)
        .and_then(take_paren)
        .map(|(inner, _)| inner.trim().to_string());
    (root, bound)
}

/// R-194 — réécrit les préludes des règles d'un `@scope` :
/// `R S` (sélecteur scopé descendant de la racine), `&` et
/// `:scope` → `:is(R)` (déjà ancrés — pas de préfixe alors),
/// borne `B` → `:not(B *)` sur l'élément cible. Récursif
/// dans les at-rules imbriquées.
fn scope_wrap_rules(rules: &mut [Rule], root: &str, bound: Option<&str>) {
    /// Remplace les `&` et `:scope` hors chaînes par `rep`.
    fn subst_anchor(s: &str, rep: &str) -> String {
        let mut out = String::with_capacity(s.len() + rep.len());
        let (mut quote, mut esc) = ('\0', false);
        let mut it = s.char_indices().peekable();
        while let Some((_, c)) = it.next() {
            if esc {
                out.push(c);
                esc = false;
                continue;
            }
            match c {
                '\\' => {
                    out.push(c);
                    esc = true;
                }
                '"' | '\'' if quote == '\0' => {
                    quote = c;
                    out.push(c);
                }
                c if c == quote => {
                    quote = '\0';
                    out.push(c);
                }
                '&' if quote == '\0' => out.push_str(rep),
                ':' if quote == '\0' => {
                    // `:scope` suivi d'une borne non-ident.
                    let rest: String = it.clone().take(5).map(|(_, c)| c).collect();
                    let after = it.clone().nth(5).map(|(_, c)| c);
                    if rest == "scope"
                        && !after.is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '-')
                    {
                        out.push_str(rep);
                        for _ in 0..5 {
                            it.next();
                        }
                    } else {
                        out.push(':');
                    }
                }
                _ => out.push(c),
            }
        }
        out
    }
    for rule in rules.iter_mut() {
        match rule {
            Rule::Qualified(q) => {
                let rep = format!(":is({})", root.trim());
                let mut parts: Vec<String> = Vec::new();
                'outer: for s in split_top_commas(&q.prelude) {
                    let s = s.trim();
                    if s.is_empty() {
                        continue;
                    }
                    let anchored = s.contains('&') || s.contains(":scope");
                    let s2 = subst_anchor(s, &rep);
                    for r in split_top_commas(root) {
                        let r = r.trim();
                        if r.is_empty() {
                            continue;
                        }
                        let mut p = if anchored {
                            s2.clone()
                        } else {
                            format!("{r} {s2}")
                        };
                        if let Some(b) = bound {
                            p.push_str(":not(");
                            p.push_str(b.trim());
                            p.push_str(" *)");
                        }
                        parts.push(p);
                        if parts.len() >= MAX_NEST_PARTS {
                            break 'outer;
                        }
                    }
                }
                q.prelude = parts.join(", ");
            }
            Rule::At(at) => match &mut at.kind {
                AtKind::Media { rules, .. }
                | AtKind::Layer { rules, .. }
                | AtKind::Container { rules, .. }
                | AtKind::Scope { rules } => scope_wrap_rules(rules, root, bound),
                _ => {}
            },
        }
    }
}

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
pub(crate) fn parse_media_queries(prelude: &str) -> Vec<MediaQuery> {
    let queries: Vec<MediaQuery> = prelude
        .split(',')
        .map(|q| parse_one_media(q.trim()))
        .collect();
    // R-207 — une query `unknown` est tolérée à `true` : elle
    // peut masquer une techno de media query non couverte
    // (formes range, features récentes) → signalée. R-216 :
    // clé détaillée par contenu — le Pareto nomme la feature
    // (ex. `@media (400px<=width)` → `mq.unknown.range`…).
    for (q, raw) in queries.iter().zip(prelude.split(',')) {
        if !q.unknown {
            continue;
        }
        explorer_common::census::bump("css.mq.unknown");
        let frag = raw.trim();
        let key =
            if frag.contains("<=") || frag.contains(">=") || frag.matches(['<', '>']).count() > 1 {
                "css.mq.unknown.range".to_string()
            } else {
                format!(
                    "css.mq.unknown.{}",
                    frag.chars().take(48).collect::<String>()
                )
            };
        explorer_common::census::bump(&key);
    }
    queries
}

/// Une query : `[not|only] [type] [and (cond)]*` ou `(cond)…`.
fn parse_one_media(q: &str) -> MediaQuery {
    let mut mq = MediaQuery {
        negated: false,
        media_type: None,
        conditions: Vec::new(),
        or_groups: Vec::new(),
        unknown: false,
    };
    // R-220 — `or` top-level (MQ4 boolean) : chaque `or`
    // ouvre un nouveau groupe AND. `cur` = groupe courant ;
    // `conditions` et `or_groups` sont remplis à la sortie.
    let mut groups: Vec<Vec<(String, String)>> = vec![Vec::new()];
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
        } else if rest.get(..2).is_some_and(|w| w.eq_ignore_ascii_case("or"))
            && rest.get(2..3).is_some_and(|c| c == " " || c == "(")
        {
            // R-220 — `(a) or (b)` : groupe AND suivant
            // (approximation documentée : le `media_type`
            // s'applique à tous les groupes).
            rest = rest[2..].trim_start();
            groups.push(Vec::new());
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
            // R-225 — exactement UNE paire externe :
            // `trim_end_matches(')')` mangeait la `)` d'un
            // `calc(…)` interne (`min-width:calc(640px -
            // 1px)` → valeur tronquée → parse `None` → la
            // condition ne matchait jamais).
            let inner_trim = inner.trim();
            let inner_trim = inner_trim
                .strip_suffix(')')
                .map_or(inner_trim, |s| s.trim_end());
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
                    [(n, v)] => groups
                        .last_mut()
                        .unwrap_or(&mut mq.conditions)
                        .push((format!("not-{n}"), v.clone())),
                    // Corps multi-conditions (double borne
                    // range, `and` interne) : non exprimable
                    // en conditions jointes par AND — nom
                    // inconnu → « feature absente » → nié vrai
                    // (tolérance documentée, cas rare).
                    _ => groups
                        .last_mut()
                        .unwrap_or(&mut mq.conditions)
                        .push((format!("not-{n_trim}"), String::new())),
                }
            } else {
                groups
                    .last_mut()
                    .unwrap_or(&mut mq.conditions)
                    .extend(parse_media_condition(inner_trim));
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
    mq.conditions = groups.first().cloned().unwrap_or_default();
    mq.or_groups = if groups.len() > 1 {
        groups[1..].to_vec()
    } else {
        Vec::new()
    };
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

/// At-rule `name` reconnue par le moteur — sondage par parse
/// d'un bloc `@name … {}` : tout nom qui produit un
/// [`AtKind`] structuré (≠ [`AtKind::UnknownBlock`]) est
/// reconnu ; les at-rules à statement gérées nominativement
/// (`charset`, `import`) complètent la liste.
///
/// Critère *reconnu au parse*, pas *appliqué* — `unapplied`
/// du census (R-207) mesure la seconde colonne ; le
/// dénominateur de couverture BCD se veut « le moteur sait
/// quoi faire de `@x` », pas « il l'applique »
/// (`scripts/bcd_coverage.mjs`).
#[must_use]
pub fn at_rule_supported(name: &str) -> bool {
    // Statements gérés nominativement : `charset`/`import`
    // (feuille), `namespace` (R-237 — registre préfixe→URI),
    // `custom-media` (R-231 — alias de media query). Un
    // statement `@x …;` inconnu est indistinguable d'un
    // reconnu au parse — la liste blanche est le critère.
    const KNOWN_STATEMENTS: &[&str] = &["charset", "import", "namespace", "custom-media"];
    if KNOWN_STATEMENTS.contains(&name.to_ascii_lowercase().as_str()) {
        return true;
    }
    let probe = format!("@{name} x {{}}");
    let sheet = parse_stylesheet(&probe);
    matches!(
        sheet.rules.first(),
        Some(Rule::At(at)) if !matches!(at.kind, AtKind::UnknownBlock { .. })
    )
}
