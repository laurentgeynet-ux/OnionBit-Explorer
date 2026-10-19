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
}

impl SimpleSelector {
    /// Spécificité de ce simple (id→a, classe/attr/pseudo→b,
    /// type→c ; `:where`→0).
    #[must_use]
    pub fn specificity(&self) -> Specificity {
        match self {
            Self::Id(_) => (1, 0, 0),
            Self::Class(_) | Self::Attribute { .. } => (0, 1, 0),
            Self::Type(_) => (0, 0, 1),
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
                    simples.push(self.pseudo()?);
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

    /// `:name` ou `:fn(...)` — après le `:`.
    fn pseudo(&mut self) -> Option<SimpleSelector> {
        match self.next() {
            Some(Token::Ident(name)) => {
                let simple = match name.as_str() {
                    "root" => PseudoClass::Root,
                    "empty" => PseudoClass::Empty,
                    "first-child" => PseudoClass::FirstChild,
                    "last-child" => PseudoClass::LastChild,
                    "only-child" => PseudoClass::OnlyChild,
                    "first-of-type" => PseudoClass::FirstOfType,
                    "last-of-type" => PseudoClass::LastOfType,
                    "only-of-type" => PseudoClass::OnlyOfType,
                    // Pseudo-classes dynamiques / non supportées →
                    // sélecteur invalide (règle ignorée).
                    _ => return None,
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

/// `true` si le nœud matche l'un des sélecteurs de la liste.
#[must_use]
pub fn matches_any(dom: &Dom, node: NodeId, selectors: &[ComplexSelector]) -> bool {
    selectors.iter().any(|s| matches(dom, node, s))
}

/// `true` si le nœud matche ce sélecteur complexe.
/// Non-élément → `false` (les textes n'ont pas de style propre).
#[must_use]
pub fn matches(dom: &Dom, node: NodeId, selector: &ComplexSelector) -> bool {
    if dom.element(node).is_err() {
        return false;
    }
    match_selector_part(dom, node, selector, selector.parts.len().saturating_sub(1))
}

/// Matching récursif droite → gauche : `idx` est l'indice du
/// composé à matcher sur `node`.
fn match_selector_part(dom: &Dom, node: NodeId, selector: &ComplexSelector, idx: usize) -> bool {
    let Some((compound, _)) = selector.parts.get(idx) else {
        return true;
    };
    if !compound.iter().all(|s| simple_matches(dom, node, s)) {
        return false;
    }
    if idx == 0 {
        return true;
    }
    // Le combinateur est porté par le composé PRÉCÉDENT
    // (lien de `parts[idx-1]` vers `parts[idx]`).
    let combinator = selector.parts[idx - 1].1.unwrap_or(Combinator::Descendant);
    match combinator {
        Combinator::Child => match parent_element(dom, node) {
            Some(p) => match_selector_part(dom, p, selector, idx - 1),
            None => false,
        },
        Combinator::Descendant => {
            let mut cur = parent_element(dom, node);
            while let Some(p) = cur {
                if match_selector_part(dom, p, selector, idx - 1) {
                    return true;
                }
                cur = parent_element(dom, p);
            }
            false
        }
        Combinator::NextSibling => match prev_element_sibling(dom, node) {
            Some(s) => match_selector_part(dom, s, selector, idx - 1),
            None => false,
        },
        Combinator::SubsequentSibling => {
            let mut cur = prev_element_sibling(dom, node);
            while let Some(s) = cur {
                if match_selector_part(dom, s, selector, idx - 1) {
                    return true;
                }
                cur = prev_element_sibling(dom, s);
            }
            false
        }
    }
}

impl CompoundSelector {
    fn iter(&self) -> impl Iterator<Item = &SimpleSelector> {
        self.simples.iter()
    }
}

/// Un simple contre un nœud élément.
fn simple_matches(dom: &Dom, node: NodeId, sel: &SimpleSelector) -> bool {
    match sel {
        SimpleSelector::Universal => true,
        SimpleSelector::Type(name) => dom
            .tag_name(node)
            .is_ok_and(|t| t.eq_ignore_ascii_case(name)),
        SimpleSelector::Class(cls) => match dom.attribute(node, "class") {
            Ok(Some(v)) => v.split_ascii_whitespace().any(|c| c == cls.as_str()),
            _ => false,
        },
        SimpleSelector::Id(id) => {
            matches!(dom.attribute(node, "id"), Ok(Some(v)) if v == *id)
        }
        SimpleSelector::Attribute {
            name,
            op,
            value,
            case_insensitive,
        } => match dom.attribute(node, name) {
            Ok(Some(actual)) => attr_matches(&actual, *op, value, *case_insensitive),
            _ => false,
        },
        SimpleSelector::Pseudo(p) => pseudo_matches(dom, node, p),
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
                    && cmp(&actual[..expected.len()], expected)
                    && actual.as_bytes()[expected.len()] == b'-')
        }
        AttrOp::Prefix => {
            actual.len() >= expected.len() && cmp(&actual[..expected.len()], expected)
        }
        AttrOp::Suffix => {
            actual.len() >= expected.len()
                && cmp(&actual[actual.len() - expected.len()..], expected)
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
fn pseudo_matches(dom: &Dom, node: NodeId, p: &PseudoClass) -> bool {
    match p {
        PseudoClass::Root => {
            matches!(dom.parent(node), Ok(Some(par)) if dom.element(par).is_err())
        }
        PseudoClass::Empty => empty_element(dom, node),
        PseudoClass::FirstChild => element_index(dom, node) == 1,
        PseudoClass::LastChild => element_index_from_end(dom, node) == 1,
        PseudoClass::OnlyChild => {
            element_index(dom, node) == 1 && element_index_from_end(dom, node) == 1
        }
        PseudoClass::NthChild(a, b) => nth_hit(*a, *b, element_index(dom, node)),
        PseudoClass::NthLastChild(a, b) => nth_hit(*a, *b, element_index_from_end(dom, node)),
        PseudoClass::FirstOfType => type_index(dom, node, false) == 1,
        PseudoClass::LastOfType => type_index(dom, node, true) == 1,
        PseudoClass::OnlyOfType => {
            type_index(dom, node, false) == 1 && type_index(dom, node, true) == 1
        }
        PseudoClass::NthOfType(a, b) => nth_hit(*a, *b, type_index(dom, node, false)),
        PseudoClass::NthLastOfType(a, b) => nth_hit(*a, *b, type_index(dom, node, true)),
        PseudoClass::Not(list) => !matches_any(dom, node, list),
        PseudoClass::Is(list) | PseudoClass::Where(list) => matches_any(dom, node, list),
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
