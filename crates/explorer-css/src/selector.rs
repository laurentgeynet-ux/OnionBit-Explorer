// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Sélecteurs CSS (spec Selectors) : parsing depuis le flux de
//! tokens du tokenizer, matching sur `explorer-dom`, spécificité.
//!
//! Couverture : type, universel `*`, `.classe`, `#id`,
//! `[attr]`, `[attr op val]` (`=`, `~=`, `|=`, `^=`, `$=`, `*=`,
//! drapeau `i`/`s`), combinateurs descendant/enfant `>`/
//! adjacent `+`/subséquent `~`, listes `,`, et les
//! pseudo-classes structurelles `:first-child`, `:last-child`,
//! `:only-child`, `:nth-child()`, `:nth-last-child()`,
//! `:first-of-type`, `:last-of-type`, `:only-of-type`,
//! `:nth-of-type()`, `:nth-last-of-type()`, `:empty`, `:root`,
//! `:not()`, `:is()`, `:where()`. Les pseudo-classes
//! dynamiques (`:hover`…) et pseudo-éléments (`::before`…) sont
//! réservés à l'étape 16 : leur présence invalide le sélecteur
//! → la règle est ignorée (dégradation spec).

use crate::token::Token;
use crate::tokenizer::tokenize;
use explorer_dom::{Dom, NodeId};
use std::collections::HashMap;

/// Spécificité `(ids, classes+attributs+pseudo-classes, types+ps-élts)`.
pub type Specificity = (u32, u32, u32);

/// Un sélecteur complexe : composés reliés par combinateurs,
/// stockés **dans l'ordre source** (gauche → droite). Le matching
/// parcourt depuis le dernier composé.
#[derive(Debug, Clone, PartialEq)]
pub struct ComplexSelector {
    /// `(composé, combinateur vers le composé SUIVANT)` — le
    /// dernier a `combinator: None`.
    pub parts: Vec<(CompoundSelector, Option<Combinator>)>,
}

impl ComplexSelector {
    /// Spécificité totale du sélecteur (somme des composés).
    #[must_use]
    pub fn specificity(&self) -> Specificity {
        let mut s = (0, 0, 0);
        for (compound, _) in &self.parts {
            let (a, b, c) = compound.specificity();
            s.0 += a;
            s.1 += b;
            s.2 += c;
        }
        s
    }
}

/// Combiné de sélecteurs simples juxtaposés (`div.a[href]`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CompoundSelector {
    /// Sélecteurs simples — tous doivent matcher le même nœud.
    pub simples: Vec<SimpleSelector>,
}

impl CompoundSelector {
    /// Spécificité du composé (somme des simples).
    #[must_use]
    pub fn specificity(&self) -> Specificity {
        let mut s = (0, 0, 0);
        for simple in &self.simples {
            let (a, b, c) = simple.specificity();
            s.0 += a;
            s.1 += b;
            s.2 += c;
        }
        s
    }
}

/// Lien entre deux composés (`a > b` : combinateur Child de
/// `a` vers `b`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Combinator {
    /// Espace : ancêtre à n'importe quelle profondeur.
    Descendant,
    /// `>` : parent direct.
    Child,
    /// `+` : frère élément directement précédent.
    NextSibling,
    /// `~` : frère élément précédent (toute distance).
    SubsequentSibling,
}

/// Sélecteur simple.
#[derive(Debug, Clone, PartialEq)]
pub enum SimpleSelector {
    /// `div`, `span` — comparé ASCII-insensitive (HTML).
    Type(String),
    /// `*`.
    Universal,
    /// `.a` — classe exacte (case-sensitive HTML, sauf quirks
    /// — géré à la cascade).
    Class(String),
    /// `#x`.
    Id(String),
    /// `[name]`, `[name=v]`, `[name~=v]`…
    Attribute {
        /// Nom d'attribut (ASCII-insensitive).
        name: String,
        /// Opérateur de comparaison.
        op: AttrOp,
        /// Valeur attendue (vide pour `Exists`).
        value: String,
        /// Drapeau `i` (insensible à la casse).
        case_insensitive: bool,
    },
    /// Pseudo-classes supportées (voir doc du module).
    Pseudo(PseudoClass),
    /// `::before`/`::after` — pseudo-élément (ou syntaxe
    /// legacy à un seul `:`). Ne peut figurer que dans le
    /// composé final : le matching élément l'ignore, la
    /// cascade dirige les déclarations vers le pseudo-style.
    PseudoElement(PseudoElement),
}

/// Pseudo-éléments générant des boîtes (R-13).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PseudoElement {
    /// `::before` — contenu généré avant les enfants.
    Before,
    /// `::after` — contenu généré après les enfants.
    After,
}

impl SimpleSelector {
    /// Spécificité de ce simple (id→a, classe/attr/pseudo→b,
    /// type→c ; `:where`→0).
    #[must_use]
    pub fn specificity(&self) -> Specificity {
        match self {
            Self::Id(_) => (1, 0, 0),
            Self::Class(_) | Self::Attribute { .. } => (0, 1, 0),
            // Pseudo-élément : colonne « type » (CSS2 §6.4.3).
            Self::Type(_) | Self::PseudoElement(_) => (0, 0, 1),
            Self::Universal => (0, 0, 0),
            Self::Pseudo(p) => p.specificity(),
        }
    }
}

/// Opérateur d'un sélecteur d'attribut.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttrOp {
    /// `[a]` : présence.
    Exists,
    /// `[a=v]` : égalité.
    Equals,
    /// `[a~=v]` : mot dans la liste séparée par des espaces.
    Includes,
    /// `[a|=v]` : égal ou préfixe `v-` (langues).
    DashMatch,
    /// `[a^=v]` : préfixe.
    Prefix,
    /// `[a$=v]` : suffixe.
    Suffix,
    /// `[a*=v]` : sous-chaîne.
    Substring,
}

/// Pseudo-classes reconnues.
#[derive(Debug, Clone, PartialEq)]
pub enum PseudoClass {
    /// `:root` — l'élément racine du document.
    Root,
    /// `:empty` — aucun enfant élément ni texte non-blanc.
    Empty,
    /// `:first-child` / `:last-child` / `:only-child`.
    FirstChild,
    /// `:last-child`.
    LastChild,
    /// `:only-child`.
    OnlyChild,
    /// `:nth-child(a n + b)` — `a`, `b`.
    NthChild(i32, i32),
    /// `:nth-last-child(a n + b)`.
    NthLastChild(i32, i32),
    /// `:first-of-type` / `:last-of-type` / `:only-of-type`.
    FirstOfType,
    /// `:last-of-type`.
    LastOfType,
    /// `:only-of-type`.
    OnlyOfType,
    /// `:nth-of-type(a n + b)`.
    NthOfType(i32, i32),
    /// `:nth-last-of-type(a n + b)`.
    NthLastOfType(i32, i32),
    /// `:not(<liste>)` — aucun sélecteur de la liste ne match.
    Not(Vec<ComplexSelector>),
    /// `:is(<liste>)` — au moins un match (spécificité du max).
    Is(Vec<ComplexSelector>),
    /// `:where(<liste>)` — comme `is` mais spécificité nulle.
    Where(Vec<ComplexSelector>),
    /// Pseudo-classe dynamique (`:hover`, `:focus`…) — évaluée
    /// via l'état fourni par l'appelant (`matches_with_state`) ;
    /// `matches` seul les considère fausses (hors `:link` qui
    /// est structurel sur `a[href]`/`area[href]`).
    Dynamic(DynamicPseudo),
}

/// Pseudo-classes dynamiques reconnues (étape 16) — leur
/// déclenchement est externe : `:hover` suit le pointeur,
/// `:checked`/`disabled` l'état de contrôle, `:target` le
/// fragment d'URL, `:focus*` la navigation clavier. Le restyle
/// passe par une re-`compute` du sous-arbre (ADR-0012).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DynamicPseudo {
    /// `:hover` — pointeur sur l'élément.
    Hover,
    /// `:active` — activation en cours.
    Active,
    /// `:focus` / `:focus-within` / `:focus-visible`.
    Focus,
    /// `:focus-within`.
    FocusWithin,
    /// `:focus-visible`.
    FocusVisible,
    /// `:link` — `a[href]`/`area[href]` non visité.
    Link,
    /// `:visited` — historique de navigation (faux ici :
    /// pas de tracking, conformité vie privée).
    Visited,
    /// `:checked` — case/option cochée.
    Checked,
    /// `:disabled` / `:enabled` — état de contrôle.
    Disabled,
    /// `:enabled`.
    Enabled,
    /// `:target` — `id` = fragment de l'URL courante.
    Target,
    /// `:required` / `:optional` — contrainte de formulaire.
    Required,
    /// `:optional`.
    Optional,
    /// `:read-only` / `:read-write`.
    ReadOnly,
    /// `:read-write`.
    ReadWrite,
    /// `:placeholder-shown`.
    PlaceholderShown,
    /// `:valid` / `:invalid` — validation de formulaire.
    Valid,
    /// `:invalid`.
    Invalid,
}

impl DynamicPseudo {
    /// Nom CSS de la pseudo-classe.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Hover => "hover",
            Self::Active => "active",
            Self::Focus => "focus",
            Self::FocusWithin => "focus-within",
            Self::FocusVisible => "focus-visible",
            Self::Link => "link",
            Self::Visited => "visited",
            Self::Checked => "checked",
            Self::Disabled => "disabled",
            Self::Enabled => "enabled",
            Self::Target => "target",
            Self::Required => "required",
            Self::Optional => "optional",
            Self::ReadOnly => "read-only",
            Self::ReadWrite => "read-write",
            Self::PlaceholderShown => "placeholder-shown",
            Self::Valid => "valid",
            Self::Invalid => "invalid",
        }
    }

    /// Depuis le nom CSS.
    fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "hover" => Self::Hover,
            "active" => Self::Active,
            "focus" => Self::Focus,
            "focus-within" => Self::FocusWithin,
            "focus-visible" => Self::FocusVisible,
            "link" => Self::Link,
            "visited" => Self::Visited,
            "checked" => Self::Checked,
            "disabled" => Self::Disabled,
            "enabled" => Self::Enabled,
            "target" => Self::Target,
            "required" => Self::Required,
            "optional" => Self::Optional,
            "read-only" => Self::ReadOnly,
            "read-write" => Self::ReadWrite,
            "placeholder-shown" => Self::PlaceholderShown,
            "valid" => Self::Valid,
            "invalid" => Self::Invalid,
            _ => return None,
        })
    }
}

impl PseudoClass {
    /// Spécificité de la pseudo-classe.
    #[must_use]
    pub fn specificity(&self) -> Specificity {
        match self {
            Self::Not(list) | Self::Is(list) => list
                .iter()
                .map(ComplexSelector::specificity)
                .max()
                .unwrap_or((0, 0, 0)),
            Self::Where(_) => (0, 0, 0),
            _ => (0, 1, 0),
        }
    }
}

// ───────────────────────────── parsing ─────────────────────────────

/// Parse un prélude de règle qualifiée en liste de sélecteurs.
/// `None` si le texte n'est pas un sélecteur valide — la règle
/// doit alors être ignorée (dégradation spec).
#[must_use]
pub fn parse_selector_list(prelude: &str) -> Option<Vec<ComplexSelector>> {
    let tokens = tokenize(prelude);
    let mut p = SelectorParser {
        tokens: &tokens,
        pos: 0,
    };
    p.list().filter(|l| !l.is_empty())
}

struct SelectorParser<'t> {
    tokens: &'t [Token],
    pos: usize,
}

impl<'t> SelectorParser<'t> {
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

    /// `<sel>, <sel>, …` jusqu'à EOF ou `)` (imbriqué).
    fn list(&mut self) -> Option<Vec<ComplexSelector>> {
        let mut out = Vec::new();
        loop {
            self.skip_ws();
            if self.peek().is_none() || matches!(self.peek(), Some(Token::CloseParen)) {
                break;
            }
            out.push(self.complex()?);
            self.skip_ws();
            match self.peek() {
                Some(Token::Comma) => {
                    self.pos += 1;
                }
                None | Some(Token::CloseParen) => break,
                _ => return None,
            }
        }
        Some(out)
    }

    /// Composé (combiné composé)* — s'arrête avant `,` ou `)`.
    fn complex(&mut self) -> Option<ComplexSelector> {
        let mut parts = Vec::new();
        loop {
            self.skip_ws();
            let compound = self.compound()?;
            self.skip_ws();
            let combinator = match self.peek() {
                Some(Token::Delim('>')) => {
                    self.pos += 1;
                    Some(Combinator::Child)
                }
                Some(Token::Delim('+')) => {
                    self.pos += 1;
                    Some(Combinator::NextSibling)
                }
                Some(Token::Delim('~')) => {
                    self.pos += 1;
                    Some(Combinator::SubsequentSibling)
                }
                Some(Token::Comma | Token::CloseParen) | None => None,
                // Tout autre token = descendant implicite
                // (l'espace a déjà été consommé par skip_ws).
                Some(_) => Some(Combinator::Descendant),
            };
            let done = combinator.is_none();
            parts.push((compound, combinator));
            if done {
                break;
            }
        }
        // Un pseudo-élément ne peut figurer que dans le
        // composé final (§5.2/§12 — `a::before b` est invalide).
        if parts[..parts.len().saturating_sub(1)]
            .iter()
            .any(|(c, _)| c.simples.iter().any(is_pseudo_element))
        {
            return None;
        }
        Some(ComplexSelector { parts })
    }

    /// Séquence de simples sans espace.
    fn compound(&mut self) -> Option<CompoundSelector> {
        let mut simples = Vec::new();
        while let Some(t) = self.peek() {
            match t {
                Token::Delim('*') => {
                    self.pos += 1;
                    simples.push(SimpleSelector::Universal);
                }
                Token::Ident(name) => {
                    let name = name.clone();
                    self.pos += 1;
                    simples.push(SimpleSelector::Type(name));
                }
                Token::Hash { value, is_id } if *is_id => {
                    let value = value.clone();
                    self.pos += 1;
                    simples.push(SimpleSelector::Id(value));
                }
                Token::Delim('.') => {
                    self.pos += 1;
                    match self.next() {
                        Some(Token::Ident(name)) => {
                            simples.push(SimpleSelector::Class(name.clone()));
                        }
                        _ => return None,
                    }
                }
                Token::OpenBracket => {
                    self.pos += 1;
                    simples.push(self.attribute()?);
                }
                Token::Colon => {
                    self.pos += 1;
                    // `::x` = pseudo-élément ; `:before`/`:after`
                    // hérités CSS2 à un seul `:` valent aussi.
                    if matches!(self.peek(), Some(Token::Colon)) {
                        self.pos += 1;
                        simples.push(self.pseudo_element()?);
                    } else {
                        simples.push(self.pseudo()?);
                    }
                }
                _ => break,
            }
        }
        if simples.is_empty() {
            None
        } else {
            Some(CompoundSelector { simples })
        }
    }

    /// `[name]`, `[name op value]`, `… i` — `]` fermant requis.
    fn attribute(&mut self) -> Option<SimpleSelector> {
        self.skip_ws();
        let name = match self.next() {
            Some(Token::Ident(n)) => n.clone(),
            _ => return None,
        };
        self.skip_ws();
        // Opérateur : `=`, `~=`, `|=`, `^=`, `$=`, `*=` ou `]`.
        let op = match self.peek() {
            Some(Token::CloseBracket) => {
                self.pos += 1;
                return Some(SimpleSelector::Attribute {
                    name,
                    op: AttrOp::Exists,
                    value: String::new(),
                    case_insensitive: false,
                });
            }
            Some(Token::Delim('=')) => {
                self.pos += 1;
                AttrOp::Equals
            }
            Some(Token::Delim(c @ ('~' | '|' | '^' | '$' | '*'))) => {
                let c = *c;
                self.pos += 1;
                match self.next() {
                    Some(Token::Delim('=')) => {}
                    _ => return None,
                }
                match c {
                    '~' => AttrOp::Includes,
                    '|' => AttrOp::DashMatch,
                    '^' => AttrOp::Prefix,
                    '$' => AttrOp::Suffix,
                    _ => AttrOp::Substring,
                }
            }
            _ => return None,
        };
        self.skip_ws();
        let value = match self.next() {
            Some(Token::Ident(v) | Token::Str(v)) => v.clone(),
            Some(Token::Number { repr, .. }) => repr.clone(),
            _ => return None,
        };
        self.skip_ws();
        let mut ci = false;
        if let Some(Token::Ident(flag)) = self.peek() {
            match flag.as_str() {
                "i" | "I" => ci = true,
                "s" | "S" => {}
                _ => return None,
            }
            self.pos += 1;
            self.skip_ws();
        }
        match self.next() {
            Some(Token::CloseBracket) => Some(SimpleSelector::Attribute {
                name,
                op,
                value,
                case_insensitive: ci,
            }),
            _ => None,
        }
    }

    /// `:name` ou `:fn(...)` — après un seul `:`.
    /// `:before`/`:after` (syntaxe CSS2) → pseudo-éléments ;
    /// les autres noms d'un seul `:` sont des pseudo-classes.
    fn pseudo(&mut self) -> Option<SimpleSelector> {
        match self.next() {
            Some(Token::Ident(name)) => {
                let simple = match name.as_str() {
                    // Pseudo-éléments legacy (un seul `:` toléré,
                    // CSS 2.1) : only-child & co. restent des
                    // pseudo-classes.
                    "before" => {
                        return Some(SimpleSelector::PseudoElement(PseudoElement::Before));
                    }
                    "after" => {
                        return Some(SimpleSelector::PseudoElement(PseudoElement::After));
                    }
                    "root" => PseudoClass::Root,
                    "empty" => PseudoClass::Empty,
                    "first-child" => PseudoClass::FirstChild,
                    "last-child" => PseudoClass::LastChild,
                    "only-child" => PseudoClass::OnlyChild,
                    "first-of-type" => PseudoClass::FirstOfType,
                    "last-of-type" => PseudoClass::LastOfType,
                    "only-of-type" => PseudoClass::OnlyOfType,
                    other => {
                        // Dynamiques reconnues → gardées (l'état
                        // arbitre au matching) ; inconnues →
                        // sélecteur invalide (règle ignorée).
                        PseudoClass::Dynamic(DynamicPseudo::from_name(other)?)
                    }
                };
                Some(SimpleSelector::Pseudo(simple))
            }
            Some(Token::Function(name)) => match name.as_str() {
                "nth-child" | "nth-last-child" | "nth-of-type" | "nth-last-of-type" => {
                    let (a, b) = self.nth_args()?;
                    let simple = match name.as_str() {
                        "nth-child" => PseudoClass::NthChild(a, b),
                        "nth-last-child" => PseudoClass::NthLastChild(a, b),
                        "nth-of-type" => PseudoClass::NthOfType(a, b),
                        _ => PseudoClass::NthLastOfType(a, b),
                    };
                    Some(SimpleSelector::Pseudo(simple))
                }
                "not" | "is" | "where" => {
                    let inner = self.list()?;
                    match self.next() {
                        Some(Token::CloseParen) => {}
                        _ => return None,
                    }
                    let simple = match name.as_str() {
                        "not" => PseudoClass::Not(inner),
                        "is" => PseudoClass::Is(inner),
                        _ => PseudoClass::Where(inner),
                    };
                    Some(SimpleSelector::Pseudo(simple))
                }
                _ => None,
            },
            _ => None,
        }
    }

    /// `::name` — pseudo-élément : seuls `before`/`after`
    /// génèrent des boîtes (R-13) ; les autres noms
    /// (`first-line`, `selection`…) rendent le sélecteur
    /// invalide → la règle est ignorée, dégradation sûre.
    fn pseudo_element(&mut self) -> Option<SimpleSelector> {
        match self.next() {
            Some(Token::Ident(name)) => match name.as_str() {
                "before" => Some(SimpleSelector::PseudoElement(PseudoElement::Before)),
                "after" => Some(SimpleSelector::PseudoElement(PseudoElement::After)),
                _ => None,
            },
            _ => None,
        }
    }

    /// Arguments `an+b` de `:nth-*` jusqu'au `)` — `odd`, `even`,
    /// `n`, `-n+3`, `2n-1`, `4`.
    fn nth_args(&mut self) -> Option<(i32, i32)> {
        // Reconstitue le texte brut jusqu'à `)` (fermant non
        // consommé ici — le token Function a déjà ouvert).
        let mut text = String::new();
        let mut depth = 1usize;
        while let Some(t) = self.next() {
            match t {
                Token::OpenParen | Token::Function(_) => depth += 1,
                Token::CloseParen => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
            text.push_str(&token_text(t));
        }
        parse_an_plus_b(&text)
    }
}

/// Rendu texte d'un token pour `nth_args` (sous-ensemble).
fn token_text(t: &Token) -> String {
    match t {
        Token::Delim(c) => c.to_string(),
        Token::Ident(s) | Token::Function(s) | Token::Str(s) => {
            let mut s = s.clone();
            if matches!(t, Token::Function(_)) {
                s.push('(');
            }
            s
        }
        Token::Number { repr, .. } => repr.clone(),
        Token::Dimension { repr, unit, .. } => format!("{repr}{unit}"),
        Token::Whitespace => " ".into(),
        _ => String::new(),
    }
}

/// Parse `an+b` (`odd`=`2n+1`, `even`=`2n`, `n`=1n+0, `-n+3`…).
/// `None` si syntaxe invalide.
fn parse_an_plus_b(text: &str) -> Option<(i32, i32)> {
    let t: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    match t.as_str() {
        "odd" => return Some((2, 1)),
        "even" => return Some((2, 0)),
        _ => {}
    }
    // Trouve le `n` : `a` = préfixe signé, `b` = suffixe signé.
    if let Some(npos) = t.find(['n', 'N']) {
        let a = match &t[..npos] {
            "" | "+" => 1,
            "-" => -1,
            s => s.parse().ok()?,
        };
        let rest = &t[npos + 1..];
        let b = if rest.is_empty() {
            0
        } else {
            rest.parse().ok()?
        };
        Some((a, b))
    } else {
        // `4` = (0, 4).
        t.parse().ok().map(|b| (0, b))
    }
}

// ───────────────────────────── matching ─────────────────────────────

/// Cache de matching partagé par une passe de cascade
/// (R-83) — `(nœud, sélecteur, composé)` → résultat, plus
/// les faits fraternels par nœud (`:nth-*`/`:first-*` —
/// une marche O(frères) calculée une fois par nœud au
/// lieu d'une fois par évaluation).
/// Invariants de pureté : le DOM ne mute pas pendant le
/// calcul, `state` est uniforme, et les sélecteurs des
/// tables de règles ont des adresses stables sous `&self`
/// — la clé pointe sur l'objet sans le déréférencer.
/// Sans lui, chaque combinateur ancêtre (`A B`) ré-évalue
/// `A` sur chaque ancêtre de chaque nœud candidat.
#[derive(Default)]
pub(crate) struct MatchCache {
    sel: HashMap<(NodeId, usize, usize), bool>,
    /// Rangs fraternels par nœud (avant/après/de type) —
    /// 0 = non calculé (les rangs réels sont 1-basés).
    ranks: HashMap<NodeId, [usize; 4]>,
}

/// Borne du cache sélecteur — un document hostile au-delà
/// dégrade en vitesse seule (évaluation directe), jamais en
/// mémoire ni en rendu.
const MATCH_CACHE_CAP: usize = 524_288;

impl MatchCache {
    fn sel_get(&self, key: &(NodeId, usize, usize)) -> Option<bool> {
        self.sel.get(key).copied()
    }

    fn sel_put(&mut self, key: (NodeId, usize, usize), v: bool) {
        if self.sel.len() < MATCH_CACHE_CAP {
            self.sel.insert(key, v);
        }
    }

    /// Rang du nœud — `kind` : 0 `element_index`,
    /// 1 `element_index_from_end`, 2/3 `type_index`
    /// (avant/après). Calculé une fois, mémoïsé.
    fn rank(&mut self, dom: &Dom, node: NodeId, kind: usize) -> usize {
        if let Some(r) = self.ranks.get(&node) {
            if r[kind] != 0 {
                return r[kind];
            }
        }
        let v = match kind {
            0 => element_index(dom, node),
            1 => element_index_from_end(dom, node),
            2 => type_index(dom, node, false),
            _ => type_index(dom, node, true),
        };
        self.ranks.entry(node).or_default()[kind] = v;
        v
    }
}

/// Filtre de Bloom des ancêtres d'un nœud — tags, id,
/// classes et noms d'attributs de chaque élément ancêtre
/// (technique « ancestor filter » des moteurs de sélecteurs,
/// R-83). Propriété : **aucun faux négatif** — une clé
/// absente du filtre ne peut figurer sur aucun ancêtre, la
/// branche `A B` est alors morte sans marcher la chaîne ;
/// les faux positifs se paient en marche ordinaire (même
/// rendu, plus lent). 128 bits, 3 positions par clé.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct AncestorFilter(u128);

/// Salts de kind — une classe `.x` et un attribut `x` n'ont
/// pas la même empreinte (réduit les faux positifs croisés).
const SEED_TAG: u64 = 0x9e37_79b9_7f4a_7c15;
const SEED_CLASS: u64 = 0xc2b2_ae3d_27d4_eb4f;
const SEED_ID: u64 = 0x1656_67b1_9e37_79f9;
const SEED_ATTR: u64 = 0xd6e8_feb8_6659_fd93;

/// FNV-1a — `fold` replie la casse ASCII comme le matcher
/// (tags ASCII-insensibles ; classes/id/attributs exacts).
fn hash_key(s: &str, fold: bool, seed: u64) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64 ^ seed;
    for b in s.bytes() {
        let b = if fold { b.to_ascii_lowercase() } else { b };
        h = (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
    }
    h
}

impl AncestorFilter {
    /// Trois positions de bit dérivées de l'empreinte.
    fn bits(h: u64) -> u128 {
        (1u128 << (h & 127)) | (1u128 << ((h >> 21) & 127)) | (1u128 << ((h >> 42) & 127))
    }

    fn may(&self, h: u64) -> bool {
        self.0 & Self::bits(h) == Self::bits(h)
    }

    /// Insère les clés d'un élément ancêtre (tag, `id`, mots
    /// de `class`, noms d'attributs) — non-élément ignoré.
    pub(crate) fn insert_node(&mut self, dom: &Dom, node: NodeId) {
        if dom.element(node).is_err() {
            return;
        }
        if let Ok(t) = dom.tag_name(node) {
            self.0 |= Self::bits(hash_key(t, true, SEED_TAG));
        }
        if let Ok(attrs) = dom.attributes(node) {
            for a in attrs {
                self.0 |= Self::bits(hash_key(&a.name, false, SEED_ATTR));
                match a.name.as_str() {
                    "id" => self.0 |= Self::bits(hash_key(&a.value, false, SEED_ID)),
                    "class" => {
                        for c in a.value.split_ascii_whitespace() {
                            self.0 |= Self::bits(hash_key(c, false, SEED_CLASS));
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    /// Filtre des ancêtres de `node` (marche de parent en
    /// parent — borne `max_depth` du DOM).
    pub(crate) fn of_ancestors(dom: &Dom, node: NodeId) -> Self {
        let mut f = Self::default();
        let mut cur = dom.parent(node).ok().flatten();
        while let Some(p) = cur {
            f.insert_node(dom, p);
            cur = dom.parent(p).ok().flatten();
        }
        f
    }

    /// `true` si le composé PEUT matcher un ancêtre — `false`
    /// garanti quand l'une de ses clés est absente du filtre
    /// (un composé sans clé répond `true` : pas de prune).
    fn compound_possible(&self, c: &CompoundSelector) -> bool {
        c.simples.iter().all(|s| {
            let h = match s {
                SimpleSelector::Type(t) => hash_key(t, true, SEED_TAG),
                SimpleSelector::Class(cl) => hash_key(cl, false, SEED_CLASS),
                SimpleSelector::Id(i) => hash_key(i, false, SEED_ID),
                SimpleSelector::Attribute { name, .. } => hash_key(name, false, SEED_ATTR),
                _ => return true,
            };
            self.may(h)
        })
    }
}

/// `true` si le nœud matche l'un des sélecteurs de la liste —
/// les pseudo-classes dynamiques sont considérées fausses.
#[must_use]
pub fn matches_any(dom: &Dom, node: NodeId, selectors: &[ComplexSelector]) -> bool {
    selectors.iter().any(|s| matches(dom, node, s))
}

/// Variante étatée : `state` reçoit le nom d'une pseudo-classe
/// dynamique (`"hover"`, `"focus"`…) et arbitre son matching.
/// C'est la couture restyle : l'appelant re-`compute` quand
/// l'état du pointeur/focus/contrôle change (ADR-0012).
#[must_use]
pub fn matches_any_with_state(
    dom: &Dom,
    node: NodeId,
    selectors: &[ComplexSelector],
    state: &dyn Fn(&str) -> bool,
) -> bool {
    matches_any_with_state_in(
        dom,
        node,
        selectors,
        state,
        &mut MatchCache::default(),
        AncestorFilter::of_ancestors(dom, node),
    )
}

/// Variante mémoïsée de [`matches_any_with_state`]. L'arête
/// `:not()`/`:is()` niche la récursion (profondeur libre par
/// le texte du sélecteur) → garde-fou `deep` (R-24).
fn matches_any_with_state_in(
    dom: &Dom,
    node: NodeId,
    selectors: &[ComplexSelector],
    state: &dyn Fn(&str) -> bool,
    cache: &mut MatchCache,
    anc: AncestorFilter,
) -> bool {
    explorer_common::deep(|| {
        selectors
            .iter()
            .any(|s| matches_with_state_cached(dom, node, s, state, &mut *cache, anc))
    })
}

/// `true` si le nœud matche ce sélecteur complexe.
/// Non-élément → `false` (les textes n'ont pas de style propre).
/// Pseudo-classes dynamiques : fausses (utiliser
/// [`matches_with_state`] pour les états pointeur/focus…).
#[must_use]
pub fn matches(dom: &Dom, node: NodeId, selector: &ComplexSelector) -> bool {
    matches_with_state(dom, node, selector, &|_| false)
}

/// Variante étatée de [`crate::matches`].
#[must_use]
pub fn matches_with_state(
    dom: &Dom,
    node: NodeId,
    selector: &ComplexSelector,
    state: &dyn Fn(&str) -> bool,
) -> bool {
    matches_with_state_cached(
        dom,
        node,
        selector,
        state,
        &mut MatchCache::default(),
        AncestorFilter::of_ancestors(dom, node),
    )
}

/// Variante mémoïsée de [`matches_with_state`] — `cache`
/// est partagé par toute la passe de cascade (R-83) et
/// `anc` porte les clés des ancêtres de `node` (filtre de
/// Bloom — la cascade l'entretient au fil de la descente).
pub(crate) fn matches_with_state_cached(
    dom: &Dom,
    node: NodeId,
    selector: &ComplexSelector,
    state: &dyn Fn(&str) -> bool,
    cache: &mut MatchCache,
    anc: AncestorFilter,
) -> bool {
    if dom.element(node).is_err() {
        return false;
    }
    match_selector_part(
        dom,
        node,
        selector,
        selector.parts.len().saturating_sub(1),
        state,
        cache,
        anc,
    )
}

/// Matching récursif droite → gauche : `idx` est l'indice du
/// composé à matcher sur `node`. Mémoïsé sur `(nœud,
/// sélecteur, idx)` — les évaluations d'ancêtres sont
/// partagées entre tous les descendants candidats (R-83).
#[allow(clippy::too_many_arguments)]
fn match_selector_part(
    dom: &Dom,
    node: NodeId,
    selector: &ComplexSelector,
    idx: usize,
    state: &dyn Fn(&str) -> bool,
    cache: &mut MatchCache,
    anc: AncestorFilter,
) -> bool {
    let Some((compound, _)) = selector.parts.get(idx) else {
        return true;
    };
    let key = (node, std::ptr::from_ref(selector) as usize, idx);
    if let Some(hit) = cache.sel_get(&key) {
        return hit;
    }
    let hit = match_selector_part_eval(dom, node, selector, idx, compound, state, cache, anc);
    cache.sel_put(key, hit);
    hit
}

/// Évaluation effective d'un composé + sa suite de
/// combinateurs (corps de [`match_selector_part`] hors cache).
#[allow(clippy::too_many_arguments)]
fn match_selector_part_eval(
    dom: &Dom,
    node: NodeId,
    selector: &ComplexSelector,
    idx: usize,
    compound: &CompoundSelector,
    state: &dyn Fn(&str) -> bool,
    cache: &mut MatchCache,
    anc: AncestorFilter,
) -> bool {
    if !compound
        .iter()
        .all(|s| simple_matches(dom, node, s, state, &mut *cache, anc))
    {
        return false;
    }
    if idx == 0 {
        return true;
    }
    // Le combinateur est porté par le composé PRÉCÉDENT
    // (lien de `parts[idx-1]` vers `parts[idx]`).
    let combinator = selector.parts[idx - 1].1.unwrap_or(Combinator::Descendant);
    // Coupure filtre : si le composé à matcher porte une
    // clé (tag/classe/id/attr) qu'aucun ancêtre ne possède,
    // la marche est impossible — on n'y va pas (R-83 ;
    // ancêtres du nœud = ancêtres de ses frères pour `+`/`~`,
    // combinators frères exclus car le frère testé n'est
    // PAS un ancêtre).
    if matches!(combinator, Combinator::Child | Combinator::Descendant)
        && !anc.compound_possible(&selector.parts[idx - 1].0)
    {
        return false;
    }
    // Récursion ∝ nombre de composés du sélecteur : les
    // marches ancêtres/frères sont itératives — un seul
    // garde-fou `deep` par niveau (R-24 ; chaque niveau
    // re-bascule tas si le segment s'épuise). Jusqu'ici
    // le garde-fou courait à CHAQUE pas d'ancêtre — coût
    // dominant du matching sur pages réelles (R-83).
    explorer_common::deep(|| match combinator {
        Combinator::Child => match parent_element(dom, node) {
            Some(p) => match_selector_part(dom, p, selector, idx - 1, state, &mut *cache, anc),
            None => false,
        },
        Combinator::Descendant => {
            let mut cur = parent_element(dom, node);
            while let Some(p) = cur {
                if match_selector_part(dom, p, selector, idx - 1, state, &mut *cache, anc) {
                    return true;
                }
                cur = parent_element(dom, p);
            }
            false
        }
        Combinator::NextSibling => match prev_element_sibling(dom, node) {
            Some(s) => match_selector_part(dom, s, selector, idx - 1, state, &mut *cache, anc),
            None => false,
        },
        Combinator::SubsequentSibling => {
            let mut cur = prev_element_sibling(dom, node);
            while let Some(s) = cur {
                if match_selector_part(dom, s, selector, idx - 1, state, &mut *cache, anc) {
                    return true;
                }
                cur = prev_element_sibling(dom, s);
            }
            false
        }
    })
}

impl CompoundSelector {
    fn iter(&self) -> impl Iterator<Item = &SimpleSelector> {
        self.simples.iter()
    }
}

/// Un simple contre un nœud élément.
fn simple_matches(
    dom: &Dom,
    node: NodeId,
    sel: &SimpleSelector,
    state: &dyn Fn(&str) -> bool,
    cache: &mut MatchCache,
    anc: AncestorFilter,
) -> bool {
    match sel {
        SimpleSelector::Type(name) => dom
            .tag_name(node)
            .is_ok_and(|t| t.eq_ignore_ascii_case(name)),
        // `attributes` rend la slice empruntée : `attribute`
        // clonerait la valeur String à chaque test — chaud
        // dans la boucle candidats × nœuds (R-83).
        SimpleSelector::Class(cls) => match dom.attributes(node) {
            Ok(attrs) => attrs.iter().any(|a| {
                a.name == "class" && a.value.split_ascii_whitespace().any(|c| c == cls.as_str())
            }),
            Err(_) => false,
        },
        SimpleSelector::Id(id) => match dom.attributes(node) {
            Ok(attrs) => attrs.iter().any(|a| a.name == "id" && a.value == *id),
            Err(_) => false,
        },
        SimpleSelector::Attribute {
            name,
            op,
            value,
            case_insensitive,
        } => match dom.attributes(node) {
            Ok(attrs) => attrs
                .iter()
                .any(|a| a.name == *name && attr_matches(&a.value, *op, value, *case_insensitive)),
            Err(_) => false,
        },
        SimpleSelector::Pseudo(p) => pseudo_matches(dom, node, p, state, cache, anc),
        // Le pseudo-élément ne contraint pas l'élément — c'est
        // la cascade qui dirige les déclarations vers le
        // pseudo-style (R-13).
        SimpleSelector::Universal | SimpleSelector::PseudoElement(_) => true,
    }
}

/// Compare une valeur d'attribut selon l'opérateur.
fn attr_matches(actual: &str, op: AttrOp, expected: &str, ci: bool) -> bool {
    let cmp = |a: &str, b: &str| -> bool {
        if ci {
            a.eq_ignore_ascii_case(b)
        } else {
            a == b
        }
    };
    match op {
        AttrOp::Exists => true,
        AttrOp::Equals => cmp(actual, expected),
        AttrOp::Includes => actual.split_ascii_whitespace().any(|w| cmp(w, expected)),
        AttrOp::DashMatch => {
            cmp(actual, expected)
                || (actual.len() > expected.len()
                    // `get` : une coupe hors frontière de
                    // char (UTF-8 multi-octets) ne peut pas
                    // égaler `expected` — `None` = `false`,
                    // jamais de panic (R-72).
                    && actual
                        .get(..expected.len())
                        .is_some_and(|p| cmp(p, expected))
                    && actual.as_bytes()[expected.len()] == b'-')
        }
        AttrOp::Prefix => {
            actual.len() >= expected.len()
                && actual
                    .get(..expected.len())
                    .is_some_and(|p| cmp(p, expected))
        }
        AttrOp::Suffix => {
            actual.len() >= expected.len()
                && actual
                    .get(actual.len() - expected.len()..)
                    .is_some_and(|p| cmp(p, expected))
        }
        AttrOp::Substring => {
            if ci {
                actual
                    .to_ascii_lowercase()
                    .contains(&expected.to_ascii_lowercase())
            } else {
                actual.contains(expected)
            }
        }
    }
}

/// Pseudo-classe contre un nœud.
fn pseudo_matches(
    dom: &Dom,
    node: NodeId,
    p: &PseudoClass,
    state: &dyn Fn(&str) -> bool,
    cache: &mut MatchCache,
    anc: AncestorFilter,
) -> bool {
    match p {
        PseudoClass::Root => {
            matches!(dom.parent(node), Ok(Some(par)) if dom.element(par).is_err())
        }
        PseudoClass::Empty => empty_element(dom, node),
        PseudoClass::FirstChild => cache.rank(dom, node, 0) == 1,
        PseudoClass::LastChild => cache.rank(dom, node, 1) == 1,
        PseudoClass::OnlyChild => cache.rank(dom, node, 0) == 1 && cache.rank(dom, node, 1) == 1,
        PseudoClass::NthChild(a, b) => nth_hit(*a, *b, cache.rank(dom, node, 0)),
        PseudoClass::NthLastChild(a, b) => nth_hit(*a, *b, cache.rank(dom, node, 1)),
        PseudoClass::FirstOfType => cache.rank(dom, node, 2) == 1,
        PseudoClass::LastOfType => cache.rank(dom, node, 3) == 1,
        PseudoClass::OnlyOfType => cache.rank(dom, node, 2) == 1 && cache.rank(dom, node, 3) == 1,
        PseudoClass::NthOfType(a, b) => nth_hit(*a, *b, cache.rank(dom, node, 2)),
        PseudoClass::NthLastOfType(a, b) => nth_hit(*a, *b, cache.rank(dom, node, 3)),
        PseudoClass::Not(list) => !matches_any_with_state_in(dom, node, list, state, cache, anc),
        PseudoClass::Is(list) | PseudoClass::Where(list) => {
            matches_any_with_state_in(dom, node, list, state, cache, anc)
        }
        PseudoClass::Dynamic(d) => dynamic_matches(dom, node, *d, state),
    }
}

/// Pseudo dynamique : `:link` est structurel (`a[href]`/
/// `area[href]`), `:visited` toujours faux (pas de tracking —
/// conformité vie privée), le reste est délégué à `state`.
fn dynamic_matches(
    dom: &Dom,
    node: NodeId,
    d: DynamicPseudo,
    state: &dyn Fn(&str) -> bool,
) -> bool {
    match d {
        DynamicPseudo::Link => {
            matches!(dom.tag_name(node), Ok("a" | "area"))
                && matches!(dom.attributes(node), Ok(attrs) if attrs.iter().any(|a| a.name == "href"))
        }
        DynamicPseudo::Visited => false,
        _ => state(d.name()),
    }
}

/// `an+b` contre un indice 1-basé (0 = non-élément/absent).
fn nth_hit(a: i32, b: i32, index: usize) -> bool {
    // index est borné par le nombre de nœuds (≤ max_nodes) —
    // le cast en i64 ne peut pas déborder en pratique ; on
    // sature tout de même pour rester tolérant.
    let i = i64::try_from(index).unwrap_or(i64::MAX);
    let (a, b) = (i64::from(a), i64::from(b));
    if a == 0 {
        return i == b;
    }
    let k = (i - b) / a;
    (i - b) % a == 0 && a * k + b == i && k >= 0
}

/// Parent élément (remonte au-delà d'un éventuel parent
/// non-élément — le document).
fn parent_element(dom: &Dom, node: NodeId) -> Option<NodeId> {
    match dom.parent(node) {
        Ok(Some(p)) if dom.element(p).is_ok() => Some(p),
        _ => None,
    }
}

/// Frère élément précédent.
fn prev_element_sibling(dom: &Dom, node: NodeId) -> Option<NodeId> {
    let mut cur = dom.prev_sibling(node).ok().flatten();
    while let Some(s) = cur {
        if dom.element(s).is_ok() {
            return Some(s);
        }
        cur = dom.prev_sibling(s).ok().flatten();
    }
    None
}

/// Rang 1-basé parmi les frères éléments (0 si erreur).
fn element_index(dom: &Dom, node: NodeId) -> usize {
    let mut n = 1usize;
    let mut cur = prev_element_sibling(dom, node);
    while let Some(s) = cur {
        n += 1;
        cur = prev_element_sibling(dom, s);
    }
    n
}

/// Rang 1-basé depuis la fin.
fn element_index_from_end(dom: &Dom, node: NodeId) -> usize {
    let mut n = 1usize;
    let mut cur = dom.next_sibling(node).ok().flatten();
    while let Some(s) = cur {
        if dom.element(s).is_ok() {
            n += 1;
        }
        cur = dom.next_sibling(s).ok().flatten();
    }
    n
}

/// Rang 1-basé parmi les frères éléments de MÊME tag
/// (`from_end` = comptage depuis la fin).
fn type_index(dom: &Dom, node: NodeId, from_end: bool) -> usize {
    let Ok(tag) = dom.tag_name(node) else {
        return 0;
    };
    let tag = tag.to_string();
    let mut n = 1usize;
    let step = |s: NodeId| -> Option<NodeId> {
        if from_end {
            dom.next_sibling(s).ok().flatten()
        } else {
            dom.prev_sibling(s).ok().flatten()
        }
    };
    let mut cur = step(node);
    while let Some(s) = cur {
        if dom.element(s).is_ok() {
            if let Ok(t) = dom.tag_name(s) {
                if t == tag {
                    n += 1;
                }
            }
        }
        cur = step(s);
    }
    n
}

/// `:empty` — pas d'enfant élément ni de texte non-blanc.
fn empty_element(dom: &Dom, node: NodeId) -> bool {
    let Ok(children) = dom.children(node) else {
        return false;
    };
    for c in children {
        if dom.element(c).is_ok() {
            return false;
        }
        if let Ok(t) = dom.text(c) {
            if !t.trim().is_empty() {
                return false;
            }
        }
    }
    true
}

/// `true` si `s` est un pseudo-élément (`::before`/`::after`).
fn is_pseudo_element(s: &SimpleSelector) -> bool {
    matches!(s, SimpleSelector::PseudoElement(_))
}

/// Pseudo-élément du composé final de `sel`, le cas échéant
/// (le parseur garantit qu'il n'apparaît que là) — la cascade
/// s'en sert pour diriger les déclarations vers le pseudo-style.
#[must_use]
pub fn selector_pseudo_element(sel: &ComplexSelector) -> Option<PseudoElement> {
    sel.parts.last()?.0.simples.iter().find_map(|s| match s {
        SimpleSelector::PseudoElement(p) => Some(*p),
        _ => None,
    })
}
