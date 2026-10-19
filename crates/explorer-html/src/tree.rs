// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Tree-builder HTML5 WHATWG — transforme le flux de [`Token`] en
//! arbre [`Dom`].
//!
//! Implémente la machinerie de la spec : modes d'insertion
//! (initial → before-html → head → body → tables → after-body),
//! pile d'éléments ouverts + scopes, liste des éléments de format
//! actifs (AFE) avec reconstruction, foster parenting en contexte
//! de table, fermetures implicites (`<p>`, `<li>`, `<dd>`…),
//! synthèse des éléments `html`/`head`/`body` manquants, et
//! pilotage du tokenizer pour les contenus rawtext/RCDATA/
//! script/PLAINTEXT (`appropriate end tag`).
//!
//! Écarts assumés, à mesurer sur html5lib-tests (ADR-0010) :
//! l'adoption agency algorithm est simplifié (imbrications
//! croisées de formatting rares), pas de namespaces réels pour
//! SVG/MathML (les balises sont insérées telles quelles, CDATA
//! toléré à l'intérieur), `<template>`/`select` sont traités
//! sans leurs règles de contenu spéciales.

use explorer_dom::{Dom, NodeId};

use crate::token::{Token, TokenAttribute};
use crate::tokenizer::{ContentKind, Tokenizer};

/// Mode de quirks décidé par le DOCTYPE (rendu CSS ultérieur).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuirksMode {
    /// `<!DOCTYPE html>` conforme.
    NoQuirks,
    /// Doctype legacy partiellement conforme.
    LimitedQuirks,
    /// Absent ou invalide → mode quirks.
    Quirks,
}

/// Modes d'insertion de la spec (sous-ensemble couvrant le web
/// documentaire et les tables).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Initial,
    BeforeHtml,
    BeforeHead,
    InHead,
    AfterHead,
    InBody,
    Text,
    InTable,
    InCaption,
    InColumnGroup,
    InTableBody,
    InRow,
    InCell,
    InSelect,
    AfterBody,
    AfterAfterBody,
}

/// Entrée de la liste des éléments de format actifs.
#[derive(Debug, Clone, Copy)]
enum Afe {
    /// Marqueur de portée (table/cellule/applet…).
    Marker,
    /// Élément de format ouvert.
    Element(NodeId),
}

/// Terminateurs du scope par défaut (in-scope).
const SCOPE_DEFAULT: &[&str] = &[
    "applet", "caption", "html", "table", "td", "th", "marquee", "object", "template",
];

/// Terminateurs du scope bouton (`button` en plus).
const SCOPE_BUTTON: &[&str] = &[
    "applet", "button", "caption", "html", "table", "td", "th", "marquee", "object", "template",
];

/// Terminateurs du scope liste (`ul`/`ol` en plus).
const SCOPE_LIST: &[&str] = &[
    "applet", "caption", "html", "ol", "table", "td", "th", "marquee", "object", "template", "ul",
];

/// Terminateurs du scope table (limité à la table elle-même).
const SCOPE_TABLE: &[&str] = &["html", "table", "template"];

/// Balises « spéciales » pour la règle générique de fermeture.
const SPECIAL: &[&str] = &[
    "address",
    "applet",
    "area",
    "article",
    "aside",
    "base",
    "basefont",
    "bgsound",
    "blockquote",
    "body",
    "br",
    "button",
    "caption",
    "center",
    "col",
    "colgroup",
    "dd",
    "details",
    "dir",
    "div",
    "dl",
    "dt",
    "embed",
    "fieldset",
    "figcaption",
    "figure",
    "footer",
    "form",
    "frame",
    "frameset",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "head",
    "header",
    "hgroup",
    "hr",
    "html",
    "iframe",
    "img",
    "input",
    "keygen",
    "li",
    "link",
    "listing",
    "main",
    "marquee",
    "menu",
    "meta",
    "nav",
    "noembed",
    "noframes",
    "noscript",
    "object",
    "ol",
    "p",
    "param",
    "plaintext",
    "pre",
    "script",
    "section",
    "select",
    "source",
    "style",
    "summary",
    "table",
    "tbody",
    "td",
    "template",
    "textarea",
    "tfoot",
    "th",
    "thead",
    "title",
    "tr",
    "track",
    "ul",
    "wbr",
    "xmp",
];

/// Fins implicites générées par `generate_implied_end_tags`.
const IMPLIED_END: &[&str] = &[
    "dd", "dt", "li", "optgroup", "option", "p", "rb", "rp", "rt", "rtc",
];

/// Balises de format (actives formatting elements).
const FORMATTING: &[&str] = &[
    "a", "b", "big", "code", "em", "font", "i", "nobr", "s", "small", "strike", "strong", "tt", "u",
];

/// Balises de contenu rawtext (`<style>` & co — texte brut).
const RAWTEXT: &[&str] = &["iframe", "noembed", "noframes", "noscript", "style", "xmp"];

/// Conteneurs qui ferment un `<p>` ouvert. Les balises ayant une
/// règle dédiée (`table`, `form`, `hr`, `pre`) sont traitées par
/// leur propre bras — ne pas les remettre ici, elles y seraient
/// interceptées avant d'atteindre leur règle.
const P_CLOSERS: &[&str] = &[
    "address",
    "article",
    "aside",
    "blockquote",
    "center",
    "details",
    "dialog",
    "dir",
    "div",
    "dl",
    "fieldset",
    "figcaption",
    "figure",
    "footer",
    "header",
    "hgroup",
    "main",
    "menu",
    "nav",
    "ol",
    "p",
    "section",
    "summary",
    "ul",
];

/// Contexte de table : l'insertion normale vise ces éléments.
const TABLE_CONTEXT: &[&str] = &["table", "tbody", "tfoot", "thead", "tr"];

/// Tree-builder : consomme des tokens, produit un [`Dom`].
pub struct TreeBuilder {
    dom: Dom,
    /// Pile des éléments ouverts (`html` en bas).
    stack: Vec<NodeId>,
    /// Liste des éléments de format actifs.
    afe: Vec<Afe>,
    mode: Mode,
    /// Mode à restaurer à la sortie de `Text` (rawtext & co).
    original_mode: Mode,
    head: Option<NodeId>,
    form: Option<NodeId>,
    quirks: QuirksMode,
    frameset_ok: bool,
    /// Insertions redirigées avant la table (foster parenting).
    foster_parenting: bool,
    /// `<table>` ouvert le plus récent (repère foster).
    last_table: Option<NodeId>,
    /// Ignore le `\n` initial après `<pre>`/`<textarea>`/`<listing>`.
    skip_leading_lf: bool,
}

impl TreeBuilder {
    /// Tree-builder sur un [`Dom`] neuf.
    #[must_use]
    pub fn new() -> Self {
        Self::with_dom(Dom::new())
    }

    /// Tree-builder sur un DOM existant (budget configuré).
    #[must_use]
    pub fn with_dom(dom: Dom) -> Self {
        Self {
            dom,
            stack: Vec::new(),
            afe: Vec::new(),
            mode: Mode::Initial,
            original_mode: Mode::InBody,
            head: None,
            form: None,
            quirks: QuirksMode::NoQuirks,
            frameset_ok: true,
            foster_parenting: false,
            last_table: None,
            skip_leading_lf: false,
        }
    }

    /// DOM consommé une fois la construction terminée.
    #[must_use]
    pub fn into_dom(self) -> Dom {
        self.dom
    }

    /// Accès au DOM en cours de construction.
    #[must_use]
    pub fn dom(&self) -> &Dom {
        &self.dom
    }

    /// Mode de quirks décidé par le DOCTYPE.
    #[must_use]
    pub fn quirks_mode(&self) -> QuirksMode {
        self.quirks
    }

    /// Raccourci complet : tokenize + construit — pour tests et
    /// `cli dump` (l'incrémental passe par [`TreeBuilder::process`]).
    #[must_use]
    pub fn parse(input: &str) -> Dom {
        let mut tb = Self::new();
        let mut t = Tokenizer::on(input);
        tb.drive(&mut t);
        tb.into_dom()
    }

    /// Consomme le tokenizer en traitant chaque token à mesure —
    /// **ne pas** utiliser `Tokenizer::run` ici : le tree-builder
    /// pilote les états de contenu (rawtext/RCDATA/script/CDATA)
    /// au fil de l'eau, ils doivent être réglés avant que la suite
    /// ne soit lue.
    pub fn drive(&mut self, tokenizer: &mut Tokenizer) {
        while let Some(tok) = tokenizer.next_token() {
            let done = matches!(tok, Token::Eof);
            self.process(tokenizer, tok);
            if done {
                break;
            }
        }
    }

    /// Traite un token émis par `tokenizer` (le tree-builder pilote
    /// les états de contenu du tokenizer — `set_content_state`).
    pub fn process(&mut self, tokenizer: &mut Tokenizer, token: Token) {
        if self.skip_leading_lf {
            self.skip_leading_lf = false;
            if let Token::Character(s) = &token {
                let stripped = s.strip_prefix('\n');
                match stripped {
                    Some("") => return,
                    Some(rest) => {
                        self.dispatch(tokenizer, Token::Character(rest.to_owned()));
                        return;
                    }
                    None => {}
                }
            }
        }
        self.dispatch(tokenizer, token);
        self.update_cdata(tokenizer);
    }

    fn dispatch(&mut self, tokenizer: &mut Tokenizer, token: Token) {
        match self.mode {
            Mode::Initial => self.step_initial(tokenizer, token),
            Mode::BeforeHtml => self.step_before_html(tokenizer, token),
            Mode::BeforeHead => self.step_before_head(tokenizer, token),
            Mode::InHead => self.step_in_head(tokenizer, token),
            Mode::AfterHead => self.step_after_head(tokenizer, token),
            Mode::InBody => self.step_in_body(tokenizer, token),
            Mode::Text => self.step_text(tokenizer, token),
            Mode::InTable => self.step_in_table(tokenizer, token),
            Mode::InCaption => self.step_in_caption(tokenizer, token),
            Mode::InColumnGroup => self.step_in_colgroup(tokenizer, token),
            Mode::InTableBody => self.step_in_table_body(tokenizer, token),
            Mode::InRow => self.step_in_row(tokenizer, token),
            Mode::InCell => self.step_in_cell(tokenizer, token),
            Mode::InSelect => self.step_in_select(tokenizer, token),
            Mode::AfterBody => self.step_after_body(tokenizer, token),
            Mode::AfterAfterBody => self.step_after_after_body(tokenizer, token),
        }
    }

    // -----------------------------------------------------------------
    // Primitives d'insertion
    // -----------------------------------------------------------------

    /// Sommet de la pile d'éléments ouverts.
    fn current(&self) -> Option<NodeId> {
        self.stack.last().copied()
    }

    /// `true` si le sommet de pile porte ce nom de balise.
    fn current_is(&self, name: &str) -> bool {
        self.current()
            .and_then(|id| self.dom.tag_name(id).ok().map(|t| t == name))
            .unwrap_or(false)
    }

    /// Crée un élément avec ses attributs et l'insère au point
    /// approprié (foster parenting si actif et contexte de table),
    /// puis le pousse sur la pile.
    fn insert_element(&mut self, name: &str, attrs: &[TokenAttribute]) -> Option<NodeId> {
        let el = self.dom.create_element(name).ok()?;
        for a in attrs {
            let _ = self.dom.set_attribute(el, &a.name, &a.value);
        }
        self.insert_node(el);
        self.stack.push(el);
        Some(el)
    }

    /// Insère un nœud au point d'insertion approprié : foster
    /// parenting quand le contexte courant est une table et que le
    /// flag est levé, sinon enfant du courant.
    fn insert_node(&mut self, node: NodeId) {
        if self.foster_parenting && self.in_table_context() {
            self.foster_insert(node);
            return;
        }
        let parent = self.current().unwrap_or_else(|| self.dom.document());
        let _ = self.dom.append_child(parent, node);
    }

    /// `true` si la cible d'insertion courante est un contexte de
    /// table (table/tbody/tfoot/thead/tr).
    fn in_table_context(&self) -> bool {
        self.current()
            .and_then(|id| {
                self.dom
                    .tag_name(id)
                    .ok()
                    .map(|t| TABLE_CONTEXT.contains(&t))
            })
            .unwrap_or(false)
    }

    /// Parent de repli foster : l'élément juste **sous** la table
    /// dans la pile (son parent pile), sinon le Document.
    fn foster_fallback_parent(&self) -> NodeId {
        if let Some(pos) = self
            .stack
            .iter()
            .rposition(|&id| Some(id) == self.last_table)
        {
            if pos > 0 {
                return self.stack[pos - 1];
            }
        }
        self.dom.document()
    }

    /// Foster parenting : insère avant la `<table>` ouverte la plus
    /// proche (dans son parent), ou en fin de l'élément qui la
    /// précède dans la pile, sinon dans le document.
    fn foster_insert(&mut self, node: NodeId) {
        if let Some(table) = self.last_table.filter(|t| self.dom.is_alive(*t)) {
            if let Ok(Some(parent)) = self.dom.parent(table) {
                if self.dom.insert_before(parent, node, Some(table)).is_ok() {
                    return;
                }
            }
        }
        // Pas de table vivante : fin de l'élément sous la table.
        let parent = self.foster_fallback_parent();
        let _ = self.dom.append_child(parent, node);
    }

    /// Insère du texte — fusionne avec le dernier enfant s'il est
    /// déjà `Text` (les Character coalescés du tokenizer peuvent
    /// arriver en plusieurs tokens).
    fn insert_text(&mut self, data: &str) {
        if data.is_empty() {
            return;
        }
        if self.foster_parenting && self.in_table_context() {
            self.foster_text(data);
            return;
        }
        let parent = self.current().unwrap_or_else(|| self.dom.document());
        self.append_text_to(parent, data);
    }

    /// Ajoute `data` au dernier enfant `Text` de `parent`, ou crée
    /// un nœud `Text` et l'attache.
    fn append_text_to(&mut self, parent: NodeId, data: &str) {
        let last = self.dom.last_child(parent).ok().flatten();
        if let Some(t) =
            last.filter(|&t| matches!(self.dom.data(t), Ok(explorer_dom::NodeData::Text(_))))
        {
            if let Ok(prev) = self.dom.text(t) {
                let merged = format!("{prev}{data}");
                let _ = self.dom.set_text(t, &merged);
                return;
            }
        }
        if let Ok(node) = self.dom.create_text(data) {
            let _ = self.dom.append_child(parent, node);
        }
    }

    /// Texte en foster parenting : même fusion, mais repérée avant
    /// la table.
    fn foster_text(&mut self, data: &str) {
        if let Some(table) = self.last_table.filter(|t| self.dom.is_alive(*t)) {
            // Le Text se place avant la table — fusionner avec un
            // éventuel frère Text immédiatement précédent.
            if let Ok(Some(prev)) = self.dom.prev_sibling(table) {
                if matches!(self.dom.data(prev), Ok(explorer_dom::NodeData::Text(_))) {
                    if let Ok(s) = self.dom.text(prev) {
                        let merged = format!("{s}{data}");
                        let _ = self.dom.set_text(prev, &merged);
                        return;
                    }
                }
            }
            if let Ok(node) = self.dom.create_text(data) {
                if let Ok(Some(parent)) = self.dom.parent(table) {
                    if self.dom.insert_before(parent, node, Some(table)).is_ok() {
                        return;
                    }
                }
            }
        }
        // Repli : fin de l'élément sous la table.
        let parent = self.foster_fallback_parent();
        self.append_text_to(parent, data);
    }

    fn insert_comment(&mut self, data: &str) {
        if let Ok(node) = self.dom.create_comment(data) {
            self.insert_node(node);
        }
    }

    /// Commentaire accroché au Document (avant `<html>`).
    fn insert_comment_document(&mut self, data: &str) {
        if let Ok(node) = self.dom.create_comment(data) {
            let _ = self.dom.append_child(self.dom.document(), node);
        }
    }

    // -----------------------------------------------------------------
    // Pile d'éléments ouverts — scopes
    // -----------------------------------------------------------------

    /// `true` si `name` est présent dans la pile sans traverser un
    /// terminateur de `scope`.
    fn in_scope(&self, name: &str, scope: &[&str]) -> bool {
        for &id in self.stack.iter().rev() {
            let Ok(tag) = self.dom.tag_name(id) else {
                continue;
            };
            if tag == name {
                return true;
            }
            if scope.contains(&tag) {
                return false;
            }
        }
        false
    }

    /// Génère les fins implicites (`</p>`/`<li>`…) jusqu'à
    /// `except` exclu.
    fn generate_implied_end_tags(&mut self, except: &str) {
        while self.current().is_some_and(|id| {
            self.dom
                .tag_name(id)
                .is_ok_and(|t| IMPLIED_END.contains(&t) && t != except)
        }) {
            self.stack.pop();
        }
    }

    /// Ferme un `<p>` en scope bouton (fermetures implicites).
    fn close_p(&mut self) {
        if self.in_scope("p", SCOPE_BUTTON) {
            self.generate_implied_end_tags("p");
            while self
                .current()
                .is_some_and(|id| self.dom.tag_name(id).ok() != Some("p"))
            {
                self.stack.pop();
            }
            self.stack.pop();
        }
    }

    /// Ferme l'élément `name` en scope : fins implicites puis dépile
    /// jusqu'à lui inclus.
    fn close_element_in_scope(&mut self, name: &str, scope: &[&str]) -> bool {
        if !self.in_scope(name, scope) {
            return false;
        }
        self.generate_implied_end_tags(name);
        while let Some(id) = self.stack.pop() {
            if self.dom.tag_name(id).ok() == Some(name) {
                break;
            }
        }
        true
    }

    /// Dépile jusqu'à un contexte parmi `tags` (inclus).
    fn clear_stack_to(&mut self, tags: &[&str]) {
        while self
            .current()
            .is_some_and(|id| self.dom.tag_name(id).is_ok_and(|t| !tags.contains(&t)))
        {
            self.stack.pop();
        }
    }

    /// `true` si la balise courante est « spéciale » (règle
    /// générique des end tags inconnus).
    fn is_special(&self, id: NodeId) -> bool {
        self.dom.tag_name(id).is_ok_and(|t| SPECIAL.contains(&t))
    }

    /// Restaure le mode d'insertion selon la pile réelle (spec
    /// « reset the insertion mode appropriately ») — après
    /// `</table>`, `</select>`…
    fn reset_mode(&mut self) {
        for &id in self.stack.iter().rev() {
            let Ok(tag) = self.dom.tag_name(id) else {
                continue;
            };
            self.mode = match tag {
                "select" => Mode::InSelect,
                "td" | "th" => Mode::InCell,
                "tr" => Mode::InRow,
                "tbody" | "thead" | "tfoot" => Mode::InTableBody,
                "caption" => Mode::InCaption,
                "colgroup" => Mode::InColumnGroup,
                "table" => Mode::InTable,
                "body" | "frameset" => Mode::InBody,
                "html" => {
                    if self.head.is_none() {
                        Mode::BeforeHead
                    } else {
                        Mode::AfterHead
                    }
                }
                _ => continue,
            };
            return;
        }
        self.mode = Mode::InBody;
    }

    /// CDATA toléré tant qu'un contenu étranger (svg/math) est ouvert.
    fn update_cdata(&mut self, tokenizer: &mut Tokenizer) {
        let foreign = self.stack.iter().any(|&id| {
            self.dom
                .tag_name(id)
                .is_ok_and(|t| t == "svg" || t == "math")
        });
        tokenizer.set_cdata_allowed(foreign);
    }

    // -----------------------------------------------------------------
    // Liste des éléments de format actifs (AFE)
    // -----------------------------------------------------------------

    fn afe_push(&mut self, el: NodeId) {
        // Règle de Noé : max 3 entrées identiques — la plus ancienne
        // saute (spec).
        let tag = self.dom.tag_name(el).ok().map(str::to_owned);
        if let Some(tag) = tag {
            let mut same = self
                .afe
                .iter()
                .enumerate()
                .rev()
                .take_while(|(_, e)| matches!(e, Afe::Element(_)))
                .filter(|(_, e)| {
                    matches!(e, Afe::Element(id) if self.dom.tag_name(*id).ok() == Some(tag.as_str()))
                })
                .map(|(i, _)| i)
                .collect::<Vec<_>>();
            if same.len() >= 3 {
                if let Some(oldest) = same.pop() {
                    self.afe.remove(oldest);
                }
            }
        }
        self.afe.push(Afe::Element(el));
    }

    fn afe_marker(&mut self) {
        self.afe.push(Afe::Marker);
    }

    /// Supprime `el` de l'AFE s'il y figure.
    fn afe_remove(&mut self, el: NodeId) {
        self.afe
            .retain(|e| !matches!(e, Afe::Element(x) if *x == el));
    }

    /// `true` si `name` est un élément de l'AFE (après le dernier
    /// marqueur) — retourne aussi son index et son `NodeId`.
    fn afe_find(&self, name: &str) -> Option<(usize, NodeId)> {
        for (i, e) in self.afe.iter().enumerate().rev() {
            match e {
                Afe::Marker => break,
                Afe::Element(id) => {
                    if self.dom.tag_name(*id).ok() == Some(name) {
                        return Some((i, *id));
                    }
                }
            }
        }
        None
    }

    /// Reconstruit les éléments de format actifs : chaque entrée
    /// après le dernier marqueur qui n'est plus dans la pile est
    /// ré-insérée (clone d'attributs) — spec « reconstruct ».
    fn reconstruct_afe(&mut self) {
        // Collecte des entrées à recréer (de bas en haut de pile).
        let mut reopen = Vec::new();
        for e in &self.afe {
            match e {
                Afe::Marker => reopen.clear(),
                Afe::Element(id) => {
                    if !self.stack.contains(id) && self.dom.is_alive(*id) {
                        reopen.push(*id);
                    }
                }
            }
        }
        for old in reopen {
            let Ok(tag) = self.dom.tag_name(old).map(str::to_owned) else {
                continue;
            };
            let attrs = self
                .dom
                .attributes(old)
                .map(|a| {
                    a.iter()
                        .map(|at| TokenAttribute {
                            name: at.name.clone(),
                            value: at.value.clone(),
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            if let Some(new_el) = self.insert_element(&tag, &attrs) {
                // Remplace l'entrée AFE par le clone.
                for e in &mut self.afe {
                    if matches!(e, Afe::Element(id) if *id == old) {
                        *e = Afe::Element(new_el);
                    }
                }
            }
        }
    }

    /// Vide l'AFE jusqu'au dernier marqueur (fin de cellule & co).
    fn afe_clear_to_marker(&mut self) {
        while matches!(self.afe.last(), Some(Afe::Element(_))) {
            self.afe.pop();
        }
        if matches!(self.afe.last(), Some(Afe::Marker)) {
            self.afe.pop();
        }
    }

    // -----------------------------------------------------------------
    // Modes initiaux — document, html, head
    // -----------------------------------------------------------------

    fn step_initial(&mut self, tokenizer: &mut Tokenizer, token: Token) {
        match token {
            Token::Character(s) if all_ws(&s) => {}
            Token::Comment(c) => self.insert_comment_document(&c),
            Token::Doctype {
                name,
                public_id,
                system_id,
                force_quirks,
            } => {
                let name = name.unwrap_or_default();
                self.quirks = quirks_mode(
                    force_quirks,
                    &name,
                    public_id.as_deref(),
                    system_id.as_deref(),
                );
                let id = self
                    .dom
                    .create_doctype(
                        &name,
                        public_id.as_deref().unwrap_or_default(),
                        system_id.as_deref().unwrap_or_default(),
                    )
                    .ok();
                if let Some(d) = id {
                    let _ = self.dom.append_child(self.dom.document(), d);
                }
                self.mode = Mode::BeforeHtml;
            }
            _ => {
                self.quirks = QuirksMode::Quirks;
                self.mode = Mode::BeforeHtml;
                self.step_before_html(tokenizer, token);
            }
        }
    }

    fn step_before_html(&mut self, tokenizer: &mut Tokenizer, token: Token) {
        match token {
            Token::Character(s) if all_ws(&s) => {}
            Token::Comment(c) => self.insert_comment_document(&c),
            Token::StartTag {
                name, attributes, ..
            } if name == "html" => {
                self.insert_element("html", &attributes);
                self.mode = Mode::BeforeHead;
            }
            Token::EndTag { name } if matches!(name.as_str(), "head" | "body" | "html" | "br") => {
                self.insert_element("html", &[]);
                self.mode = Mode::BeforeHead;
                self.dispatch(tokenizer, Token::EndTag { name });
            }
            Token::Doctype { .. } | Token::EndTag { .. } => {}
            _ => {
                self.insert_element("html", &[]);
                self.mode = Mode::BeforeHead;
                self.dispatch(tokenizer, token);
            }
        }
    }

    fn step_before_head(&mut self, tokenizer: &mut Tokenizer, token: Token) {
        match token {
            Token::Character(s) if all_ws(&s) => {}
            Token::Comment(c) => self.insert_comment(&c),
            Token::StartTag {
                name, attributes, ..
            } if name == "html" => {
                self.step_in_body(
                    tokenizer,
                    Token::StartTag {
                        name,
                        attributes,
                        self_closing: false,
                    },
                );
            }
            Token::StartTag {
                name, attributes, ..
            } if name == "head" => {
                self.head = self.insert_element("head", &attributes);
                self.mode = Mode::InHead;
            }
            Token::EndTag { name } if matches!(name.as_str(), "head" | "body" | "html" | "br") => {
                self.head = self.insert_element("head", &[]);
                self.mode = Mode::InHead;
                self.dispatch(tokenizer, Token::EndTag { name });
            }
            Token::Doctype { .. } | Token::EndTag { .. } => {}
            _ => {
                self.head = self.insert_element("head", &[]);
                self.mode = Mode::InHead;
                self.dispatch(tokenizer, token);
            }
        }
    }

    fn step_in_head(&mut self, tokenizer: &mut Tokenizer, token: Token) {
        match token {
            Token::Doctype { .. } => {}

            Token::Character(s) => {
                let (ws, rest) = split_ws(&s);
                self.insert_text(ws);
                if rest.is_empty() {
                    return;
                }
                self.stack.pop(); // ferme head
                self.mode = Mode::AfterHead;
                self.dispatch(tokenizer, Token::Character(rest.to_owned()));
            }
            Token::Comment(c) => self.insert_comment(&c),
            Token::StartTag {
                name,
                attributes,
                self_closing,
            } => match name.as_str() {
                "html" => self.step_in_body(
                    tokenizer,
                    Token::StartTag {
                        name,
                        attributes,
                        self_closing,
                    },
                ),
                "base" | "basefont" | "bgsound" | "link" | "meta" => {
                    self.insert_element(&name, &attributes);
                    self.stack.pop();
                }
                "title" => {
                    self.insert_element(&name, &attributes);
                    tokenizer.set_content_state(ContentKind::Rcdata);
                    self.original_mode = self.mode;
                    self.mode = Mode::Text;
                }
                "noframes" | "style" => {
                    self.insert_element(&name, &attributes);
                    tokenizer.set_content_state(ContentKind::RawText);
                    self.original_mode = self.mode;
                    self.mode = Mode::Text;
                }
                "script" => {
                    self.insert_element(&name, &attributes);
                    tokenizer.set_content_state(ContentKind::ScriptData);
                    self.original_mode = self.mode;
                    self.mode = Mode::Text;
                }
                "noscript" => {
                    self.insert_element(&name, &attributes);
                }
                "template" => {
                    self.insert_element(&name, &attributes);
                    self.afe_marker();
                    self.frameset_ok = false;
                }
                "head" => {}
                _ => {
                    self.stack.pop();
                    self.mode = Mode::AfterHead;
                    self.dispatch(
                        tokenizer,
                        Token::StartTag {
                            name,
                            attributes,
                            self_closing,
                        },
                    );
                }
            },
            Token::EndTag { name } => match name.as_str() {
                "head" => {
                    self.stack.pop();
                    self.mode = Mode::AfterHead;
                }
                "body" | "html" | "br" => {
                    self.stack.pop();
                    self.mode = Mode::AfterHead;
                    self.dispatch(tokenizer, Token::EndTag { name });
                }
                "template" => {
                    self.close_element_in_scope("template", SCOPE_DEFAULT);
                    self.afe_clear_to_marker();
                }
                _ => {}
            },
            Token::Eof => {
                self.stack.pop();
                self.mode = Mode::AfterHead;
                self.dispatch(tokenizer, Token::Eof);
            }
        }
    }

    fn step_after_head(&mut self, tokenizer: &mut Tokenizer, token: Token) {
        match token {
            Token::Doctype { .. } => {}

            Token::Character(s) => {
                let (ws, rest) = split_ws(&s);
                self.insert_text(ws);
                if rest.is_empty() {
                    return;
                }
                self.insert_element("body", &[]);
                self.mode = Mode::InBody;
                self.dispatch(tokenizer, Token::Character(rest.to_owned()));
            }
            Token::Comment(c) => self.insert_comment(&c),
            Token::StartTag {
                name,
                attributes,
                self_closing,
            } => match name.as_str() {
                "html" => self.step_in_body(
                    tokenizer,
                    Token::StartTag {
                        name,
                        attributes,
                        self_closing,
                    },
                ),
                "body" => {
                    self.insert_element("body", &attributes);
                    self.frameset_ok = false;
                    self.mode = Mode::InBody;
                }
                "frameset" => {
                    self.insert_element("frameset", &attributes);
                    self.mode = Mode::InBody;
                }
                "base" | "basefont" | "bgsound" | "link" | "meta" | "noframes" | "script"
                | "style" | "template" | "title" => {
                    // Contenu de tête tardif : ré-ouvre head le
                    // temps du traitement (spec).
                    let head = self.head;
                    if let Some(h) = head {
                        self.stack.push(h);
                        self.step_in_head(
                            tokenizer,
                            Token::StartTag {
                                name,
                                attributes,
                                self_closing,
                            },
                        );
                        if let Some(pos) = self.stack.iter().rposition(|&x| x == h) {
                            self.stack.remove(pos);
                        }
                    }
                }
                "head" => {}
                _ => {
                    self.insert_element("body", &[]);
                    self.mode = Mode::InBody;
                    self.dispatch(
                        tokenizer,
                        Token::StartTag {
                            name,
                            attributes,
                            self_closing,
                        },
                    );
                }
            },
            Token::EndTag { name } => match name.as_str() {
                "body" | "html" | "br" => {
                    self.insert_element("body", &[]);
                    self.mode = Mode::InBody;
                    self.dispatch(tokenizer, Token::EndTag { name });
                }
                "template" => self.step_in_head(tokenizer, Token::EndTag { name }),
                _ => {}
            },
            Token::Eof => {
                self.insert_element("body", &[]);
                self.mode = Mode::InBody;
                self.dispatch(tokenizer, Token::Eof);
            }
        }
    }

    // -----------------------------------------------------------------
    // InBody — le gros de la spec
    // -----------------------------------------------------------------

    fn step_in_body(&mut self, tokenizer: &mut Tokenizer, token: Token) {
        match token {
            Token::Character(s) => {
                if !all_ws(&s) {
                    self.frameset_ok = false;
                }
                self.reconstruct_afe();
                self.insert_text(&s);
            }
            Token::Comment(c) => self.insert_comment(&c),
            Token::Doctype { .. } | Token::Eof => {
                // Fin : tout reste ouvert, le document est complet.
            }
            Token::StartTag {
                name,
                attributes,
                self_closing,
            } => self.body_start_tag(tokenizer, &name, &attributes, self_closing),
            Token::EndTag { name } => self.body_end_tag(tokenizer, &name),
        }
    }

    #[allow(clippy::too_many_lines)]
    fn body_start_tag(
        &mut self,
        tokenizer: &mut Tokenizer,
        name: &str,
        attributes: &[TokenAttribute],
        self_closing: bool,
    ) {
        match name {
            "html" => {
                // Fusionne les attributs sur <html> existant.
                if let Some(&html) = self.stack.first() {
                    for a in attributes {
                        if self.dom.attribute(html, &a.name).ok().flatten().is_none() {
                            let _ = self.dom.set_attribute(html, &a.name, &a.value);
                        }
                    }
                }
            }
            "body" => {
                if let Some(&body) = self.stack.get(1) {
                    if self.dom.tag_name(body).ok() == Some("body") {
                        for a in attributes {
                            if self.dom.attribute(body, &a.name).ok().flatten().is_none() {
                                let _ = self.dom.set_attribute(body, &a.name, &a.value);
                            }
                        }
                        self.frameset_ok = false;
                    }
                }
            }
            "base" | "basefont" | "bgsound" | "link" | "meta" | "noframes" | "script" | "style"
            | "template" | "title" => {
                // Spec : « process using the in-head rules » —
                // insertion au nœud courant (body), PAS dans head.
                self.step_in_head(
                    tokenizer,
                    Token::StartTag {
                        name: name.to_owned(),
                        attributes: attributes.to_vec(),
                        self_closing: false,
                    },
                );
            }
            _ if P_CLOSERS.contains(&name) => {
                self.close_p();
                self.insert_element(name, attributes);
            }
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                self.close_p();
                if self
                    .current()
                    .and_then(|id| self.dom.tag_name(id).ok().map(str::to_owned))
                    .is_some_and(|t| matches!(t.as_str(), "h1" | "h2" | "h3" | "h4" | "h5" | "h6"))
                {
                    self.stack.pop();
                }
                self.insert_element(name, attributes);
            }
            "pre" | "listing" => {
                self.close_p();
                self.insert_element(name, attributes);
                self.skip_leading_lf = true;
                self.frameset_ok = false;
            }
            "form" => {
                if self.form.is_none() {
                    self.close_p();
                    self.form = self.insert_element(name, attributes);
                }
            }
            "li" => {
                // Ferme un li ouvert dans le scope liste.
                for i in (0..self.stack.len()).rev() {
                    let id = self.stack[i];
                    let Ok(tag) = self.dom.tag_name(id).map(str::to_owned) else {
                        continue;
                    };
                    if tag == "li" {
                        self.generate_implied_end_tags("li");
                        while self
                            .stack
                            .pop()
                            .is_some_and(|x| self.dom.tag_name(x).ok() != Some("li"))
                        {
                        }
                        break;
                    }
                    if SCOPE_LIST.contains(&tag.as_str()) && tag != "li" {
                        break;
                    }
                }
                self.close_p();
                self.insert_element(name, attributes);
            }
            "dd" | "dt" => {
                for i in (0..self.stack.len()).rev() {
                    let id = self.stack[i];
                    let Ok(tag) = self.dom.tag_name(id).map(str::to_owned) else {
                        continue;
                    };
                    if tag == "dd" || tag == "dt" {
                        let target = tag.clone();
                        self.generate_implied_end_tags(&target);
                        while self
                            .stack
                            .pop()
                            .is_some_and(|x| self.dom.tag_name(x).ok() != Some(target.as_str()))
                        {
                        }
                        break;
                    }
                    if SCOPE_LIST.contains(&tag.as_str()) && tag != "dd" && tag != "dt" {
                        break;
                    }
                }
                self.close_p();
                self.insert_element(name, attributes);
            }
            "plaintext" => {
                self.close_p();
                self.insert_element(name, attributes);
                tokenizer.set_content_state(ContentKind::PlainText);
            }
            "button" => {
                if self.in_scope("button", SCOPE_DEFAULT) {
                    self.close_element_in_scope("button", SCOPE_DEFAULT);
                }
                self.reconstruct_afe();
                self.insert_element(name, attributes);
                self.frameset_ok = false;
            }
            "a" => {
                if let Some((_, old)) = self.afe_find("a") {
                    // <a> alors qu'un a est actif : ferme l'ancien.
                    self.body_end_tag(tokenizer, "a");
                    self.afe_remove(old);
                    self.stack.retain(|&x| x != old);
                }
                self.reconstruct_afe();
                if let Some(el) = self.insert_element(name, attributes) {
                    self.afe_push(el);
                }
            }
            _ if FORMATTING.contains(&name) => {
                self.reconstruct_afe();
                if let Some(el) = self.insert_element(name, attributes) {
                    self.afe_push(el);
                }
            }
            "nobr" => {
                self.reconstruct_afe();
                if self.in_scope("nobr", SCOPE_DEFAULT) {
                    self.body_end_tag(tokenizer, "nobr");
                    self.reconstruct_afe();
                }
                if let Some(el) = self.insert_element(name, attributes) {
                    self.afe_push(el);
                }
            }
            "applet" | "marquee" | "object" => {
                self.reconstruct_afe();
                self.insert_element(name, attributes);
                self.afe_marker();
                self.frameset_ok = false;
            }
            "table" => {
                if self.quirks != QuirksMode::Quirks {
                    self.close_p();
                }
                if let Some(el) = self.insert_element(name, attributes) {
                    self.last_table = Some(el);
                }
                self.frameset_ok = false;
                self.mode = Mode::InTable;
            }
            "area" | "br" | "embed" | "img" | "keygen" | "wbr" | "param" | "source" | "track"
            | "hr" => {
                if name == "hr" {
                    self.close_p();
                } else {
                    self.reconstruct_afe();
                }
                self.insert_element(name, attributes);
                self.stack.pop();
                self.frameset_ok = false;
            }
            "input" => {
                self.reconstruct_afe();
                self.insert_element(name, attributes);
                self.stack.pop();
                let hidden = attributes
                    .iter()
                    .any(|a| a.name == "type" && a.value.eq_ignore_ascii_case("hidden"));
                if !hidden {
                    self.frameset_ok = false;
                }
            }
            "image" => self.body_start_tag(tokenizer, "img", attributes, false),
            "textarea" => {
                self.insert_element(name, attributes);
                self.skip_leading_lf = true;
                tokenizer.set_content_state(ContentKind::Rcdata);
                self.original_mode = self.mode;
                self.mode = Mode::Text;
                self.frameset_ok = false;
            }
            _ if RAWTEXT.contains(&name) => {
                if name == "xmp" {
                    self.close_p();
                    self.reconstruct_afe();
                    self.frameset_ok = false;
                }
                if name == "iframe" {
                    self.frameset_ok = false;
                }
                self.insert_element(name, attributes);
                tokenizer.set_content_state(ContentKind::RawText);
                self.original_mode = self.mode;
                self.mode = Mode::Text;
            }
            "select" => {
                self.reconstruct_afe();
                self.insert_element(name, attributes);
                self.mode = Mode::InSelect;
            }
            "optgroup" | "option" => {
                if self.current_is("option") {
                    self.stack.pop();
                }
                self.reconstruct_afe();
                self.insert_element(name, attributes);
            }
            "rb" | "rtc" => {
                if self.in_scope("ruby", SCOPE_DEFAULT) {
                    self.generate_implied_end_tags("");
                }
                self.insert_element(name, attributes);
            }
            "rp" | "rt" => {
                if self.in_scope("ruby", SCOPE_DEFAULT) {
                    self.generate_implied_end_tags("rtc");
                }
                self.insert_element(name, attributes);
            }
            "math" | "svg" => {
                self.reconstruct_afe();
                self.insert_element(name, attributes);
                if self_closing {
                    self.stack.pop();
                }
            }
            "caption" | "col" | "colgroup" | "frame" | "head" | "tbody" | "td" | "tfoot" | "th"
            | "thead" | "tr" => {
                // Balises de table hors contexte → ignorées.
            }
            _ => {
                self.reconstruct_afe();
                self.insert_element(name, attributes);
            }
        }
    }

    fn body_end_tag(&mut self, _tokenizer: &mut Tokenizer, name: &str) {
        match name {
            "body" => {
                if self.in_scope("body", SCOPE_DEFAULT) {
                    self.mode = Mode::AfterBody;
                }
            }
            "html" => {
                // `</html>` = `</body>` : passage en AfterBody.
                // Le token n'est pas re-traité (évite le ping-pong
                // AfterBody ↔ InBody — équivalent spec : les
                // contenus suivants basculeront en InBody
                // d'eux-mêmes).
                if self.in_scope("body", SCOPE_DEFAULT) {
                    self.mode = Mode::AfterBody;
                }
            }
            "p" => {
                if !self.in_scope("p", SCOPE_BUTTON) {
                    // Spec : `</p>` sans p ouvert en insère un puis
                    // le ferme.
                    self.insert_element("p", &[]);
                }
                self.close_element_in_scope("p", SCOPE_BUTTON);
            }
            "li" => {
                self.close_element_in_scope("li", SCOPE_LIST);
            }
            "dd" | "dt" => {
                self.close_element_in_scope(name, SCOPE_LIST);
            }
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                for h in ["h1", "h2", "h3", "h4", "h5", "h6"] {
                    if self.close_element_in_scope(h, SCOPE_DEFAULT) {
                        break;
                    }
                }
            }
            "form" => {
                let target = self.form.take().or_else(|| {
                    self.stack
                        .iter()
                        .rev()
                        .find(|&&id| self.dom.tag_name(id).ok() == Some("form"))
                        .copied()
                });
                if let Some(f) = target {
                    self.generate_implied_end_tags("");
                    self.stack.retain(|&x| x != f);
                }
            }
            _ if P_CLOSERS.contains(&name) || name == "listing" || name == "pre" => {
                self.close_element_in_scope(name, SCOPE_DEFAULT);
            }
            _ if FORMATTING.contains(&name) => {
                self.adoption_lite(name);
            }
            "applet" | "marquee" | "object" => {
                if self.close_element_in_scope(name, SCOPE_DEFAULT) {
                    self.afe_clear_to_marker();
                }
            }
            "br" => {
                // `</br>` est une erreur → traité comme `<br>`.
                self.reconstruct_afe();
                if let Some(el) = self.insert_element("br", &[]) {
                    let _ = el;
                    self.stack.pop();
                }
                self.frameset_ok = false;
            }
            "template" => {
                self.close_element_in_scope("template", SCOPE_DEFAULT);
                self.afe_clear_to_marker();
            }
            _ => self.generic_end_tag(name),
        }
    }

    /// Règle générique des end tags (spec « any other end tag ») :
    /// remonte la pile ; si l'élément existe → fins implicites +
    /// dépile jusqu'à lui ; si un « spécial » barre la route →
    /// ignorer.
    fn generic_end_tag(&mut self, name: &str) {
        for i in (0..self.stack.len()).rev() {
            let id = self.stack[i];
            if self.dom.tag_name(id).ok() == Some(name) {
                self.generate_implied_end_tags(name);
                self.stack.truncate(i + 1);
                self.stack.pop();
                return;
            }
            if self.is_special(id) {
                return;
            }
        }
    }

    /// Adoption agency simplifiée : si l'élément de format est en
    /// scope → fins implicites + dépile jusqu'à lui + retrait AFE.
    /// (La spec complète ré-parente les contenus croisés — cas rares
    /// mesurés par corpus, ADR-0010.)
    fn adoption_lite(&mut self, name: &str) {
        if let Some((_, el)) = self.afe_find(name) {
            if self.stack.contains(&el) {
                self.generate_implied_end_tags(name);
                while let Some(top) = self.stack.pop() {
                    if top == el {
                        break;
                    }
                }
            }
            self.afe_remove(el);
        }
    }

    // -----------------------------------------------------------------
    // Text — contenu rawtext/RCDATA/script
    // -----------------------------------------------------------------

    fn step_text(&mut self, tokenizer: &mut Tokenizer, token: Token) {
        match token {
            Token::Character(s) => self.insert_text(&s),
            Token::EndTag { .. } => {
                // End tag approprié (le tokenizer n'émet que celui
                // correspondant à la balise ouvrante).
                self.stack.pop();
                self.mode = self.original_mode;
            }
            Token::Eof => {
                self.stack.pop();
                self.mode = self.original_mode;
                self.dispatch(tokenizer, Token::Eof);
            }
            _ => {}
        }
    }

    // -----------------------------------------------------------------
    // Tables
    // -----------------------------------------------------------------

    #[allow(clippy::too_many_lines)]
    fn step_in_table(&mut self, tokenizer: &mut Tokenizer, token: Token) {
        match token {
            Token::Doctype { .. } => {}

            Token::Character(s) => {
                // Spec InTableText : ws → dans la table, le reste
                // est foster-parenté.
                if all_ws(&s) {
                    self.insert_text(&s);
                } else {
                    let saved = self.foster_parenting;
                    self.foster_parenting = true;
                    self.step_in_body(tokenizer, Token::Character(s));
                    self.foster_parenting = saved;
                }
            }
            Token::Comment(c) => self.insert_comment(&c),
            Token::StartTag {
                name,
                attributes,
                self_closing,
            } => match name.as_str() {
                "caption" => {
                    self.clear_stack_to(TABLE_CONTEXT);
                    self.afe_marker();
                    self.insert_element("caption", &attributes);
                    self.mode = Mode::InCaption;
                }
                "colgroup" => {
                    self.clear_stack_to(TABLE_CONTEXT);
                    self.insert_element("colgroup", &attributes);
                    self.mode = Mode::InColumnGroup;
                }
                "col" => {
                    self.clear_stack_to(TABLE_CONTEXT);
                    self.insert_element("colgroup", &[]);
                    self.mode = Mode::InColumnGroup;
                    self.dispatch(
                        tokenizer,
                        Token::StartTag {
                            name,
                            attributes,
                            self_closing,
                        },
                    );
                }
                "tbody" | "tfoot" | "thead" => {
                    self.clear_stack_to(TABLE_CONTEXT);
                    self.insert_element(&name, &attributes);
                    self.mode = Mode::InTableBody;
                }
                "td" | "th" | "tr" => {
                    self.clear_stack_to(TABLE_CONTEXT);
                    self.insert_element("tbody", &[]);
                    self.mode = Mode::InTableBody;
                    self.dispatch(
                        tokenizer,
                        Token::StartTag {
                            name,
                            attributes,
                            self_closing,
                        },
                    );
                }
                "table" => {
                    // `<table>` dans `<table>` : erreur → agit comme
                    // `</table>` puis retraite.
                    if self.close_element_in_scope("table", SCOPE_TABLE) {
                        self.after_table_pop();
                        self.dispatch(
                            tokenizer,
                            Token::StartTag {
                                name,
                                attributes,
                                self_closing,
                            },
                        );
                    }
                }
                "style" | "script" | "template" => {
                    self.step_in_head(
                        tokenizer,
                        Token::StartTag {
                            name,
                            attributes,
                            self_closing,
                        },
                    );
                }
                "input" => {
                    let hidden = attributes
                        .iter()
                        .any(|a| a.name == "type" && a.value.eq_ignore_ascii_case("hidden"));
                    if hidden {
                        self.insert_element("input", &attributes);
                        self.stack.pop();
                    } else {
                        self.table_anything_else(
                            tokenizer,
                            Token::StartTag {
                                name,
                                attributes,
                                self_closing,
                            },
                        );
                    }
                }
                "form" => {
                    if self.form.is_none() {
                        self.form = self.insert_element("form", &attributes);
                        self.stack.pop();
                    }
                }
                _ => self.table_anything_else(
                    tokenizer,
                    Token::StartTag {
                        name,
                        attributes,
                        self_closing,
                    },
                ),
            },
            Token::EndTag { name } => match name.as_str() {
                "table" => {
                    if self.close_element_in_scope("table", SCOPE_TABLE) {
                        self.after_table_pop();
                    }
                }
                "body" | "caption" | "col" | "colgroup" | "html" | "tbody" | "td" | "tfoot"
                | "th" | "thead" | "tr" => {}
                "template" => self.step_in_head(tokenizer, Token::EndTag { name }),
                _ => self.table_anything_else(tokenizer, Token::EndTag { name }),
            },
            Token::Eof => self.step_in_body(tokenizer, Token::Eof),
        }
    }

    /// Tout ce qui n'est pas table : `InBody` avec foster parenting.
    fn table_anything_else(&mut self, tokenizer: &mut Tokenizer, token: Token) {
        let saved = self.foster_parenting;
        self.foster_parenting = true;
        self.step_in_body(tokenizer, token);
        self.foster_parenting = saved;
    }

    /// Après avoir dépilé `<table>` : met à jour le repère foster
    /// et restaure le mode.
    fn after_table_pop(&mut self) {
        self.last_table = self
            .stack
            .iter()
            .rev()
            .find(|&&id| self.dom.tag_name(id).ok() == Some("table"))
            .copied();
        self.reset_mode();
    }

    fn step_in_caption(&mut self, tokenizer: &mut Tokenizer, token: Token) {
        match token.clone() {
            Token::EndTag { name } if name == "caption" => {
                if self.close_element_in_scope("caption", SCOPE_TABLE) {
                    self.afe_clear_to_marker();
                    self.mode = Mode::InTable;
                }
            }
            Token::StartTag { name, .. }
                if matches!(
                    name.as_str(),
                    "caption"
                        | "col"
                        | "colgroup"
                        | "tbody"
                        | "td"
                        | "tfoot"
                        | "th"
                        | "thead"
                        | "tr"
                ) =>
            {
                if self.close_element_in_scope("caption", SCOPE_TABLE) {
                    self.afe_clear_to_marker();
                    self.mode = Mode::InTable;
                    self.dispatch(tokenizer, token);
                }
            }
            Token::EndTag { name } if name == "table" => {
                if self.close_element_in_scope("caption", SCOPE_TABLE) {
                    self.afe_clear_to_marker();
                    self.mode = Mode::InTable;
                    self.dispatch(tokenizer, token);
                }
            }
            _ => self.step_in_body(tokenizer, token),
        }
    }

    fn step_in_colgroup(&mut self, tokenizer: &mut Tokenizer, token: Token) {
        match token {
            Token::Doctype { .. } => {}

            Token::Character(s) => {
                let (ws, rest) = split_ws(&s);
                self.insert_text(ws);
                if rest.is_empty() {
                    return;
                }
                if self.current_is("colgroup") {
                    self.stack.pop();
                }
                self.mode = Mode::InTable;
                self.dispatch(tokenizer, Token::Character(rest.to_owned()));
            }
            Token::Comment(c) => self.insert_comment(&c),
            Token::StartTag {
                name,
                attributes,
                self_closing,
            } => match name.as_str() {
                "html" => self.step_in_body(
                    tokenizer,
                    Token::StartTag {
                        name,
                        attributes,
                        self_closing,
                    },
                ),
                "col" => {
                    self.insert_element("col", &attributes);
                    self.stack.pop();
                }
                "template" => self.step_in_head(
                    tokenizer,
                    Token::StartTag {
                        name,
                        attributes,
                        self_closing,
                    },
                ),
                _ => {
                    if self.current_is("colgroup") {
                        self.stack.pop();
                    }
                    self.mode = Mode::InTable;
                    self.dispatch(
                        tokenizer,
                        Token::StartTag {
                            name,
                            attributes,
                            self_closing,
                        },
                    );
                }
            },
            Token::EndTag { name } => match name.as_str() {
                "colgroup" => {
                    if self.current_is("colgroup") {
                        self.stack.pop();
                    }
                    self.mode = Mode::InTable;
                }
                "col" => {}
                "template" => self.step_in_head(tokenizer, Token::EndTag { name }),
                _ => {
                    if self.current_is("colgroup") {
                        self.stack.pop();
                    }
                    self.mode = Mode::InTable;
                    self.dispatch(tokenizer, Token::EndTag { name });
                }
            },
            Token::Eof => self.step_in_body(tokenizer, Token::Eof),
        }
    }

    fn step_in_table_body(&mut self, tokenizer: &mut Tokenizer, token: Token) {
        match token.clone() {
            Token::StartTag {
                name,
                attributes,
                self_closing,
            } if name == "tr" => {
                self.clear_stack_to(&["tbody", "tfoot", "thead", "html", "table"]);
                self.insert_element("tr", &attributes);
                self.mode = Mode::InRow;
                let _ = self_closing;
            }
            Token::StartTag { name, .. } if name == "td" || name == "th" => {
                // `td` direct dans tbody → tr synthétisé.
                self.clear_stack_to(&["tbody", "tfoot", "thead", "html", "table"]);
                self.insert_element("tr", &[]);
                self.mode = Mode::InRow;
                self.dispatch(tokenizer, token);
            }
            Token::EndTag { name } if matches!(name.as_str(), "tbody" | "tfoot" | "thead") => {
                if self.in_scope(&name, SCOPE_TABLE) {
                    self.clear_stack_to(&["tbody", "tfoot", "thead", "html", "table"]);
                    self.stack.pop();
                    self.mode = Mode::InTable;
                }
            }
            Token::StartTag { name, .. }
                if matches!(
                    name.as_str(),
                    "caption" | "col" | "colgroup" | "tbody" | "tfoot" | "thead"
                ) =>
            {
                self.table_body_to_table(tokenizer, token);
            }
            Token::EndTag { name } if name == "table" => {
                self.table_body_to_table(tokenizer, token);
            }
            Token::EndTag { name }
                if matches!(
                    name.as_str(),
                    "body" | "caption" | "col" | "colgroup" | "html" | "td" | "th" | "tr"
                ) => {}
            _ => self.step_in_table(tokenizer, token),
        }
    }

    /// `</table>` ou section de table dans tbody : ferme la section
    /// puis retraite en `InTable`.
    fn table_body_to_table(&mut self, tokenizer: &mut Tokenizer, token: Token) {
        for sec in ["tbody", "tfoot", "thead"] {
            if self.in_scope(sec, SCOPE_TABLE) {
                self.clear_stack_to(&["tbody", "tfoot", "thead", "html", "table"]);
                self.stack.pop();
                self.mode = Mode::InTable;
                self.dispatch(tokenizer, token);
                return;
            }
        }
        self.step_in_table(tokenizer, token);
    }

    fn step_in_row(&mut self, tokenizer: &mut Tokenizer, token: Token) {
        match token.clone() {
            Token::StartTag {
                name, attributes, ..
            } if name == "td" || name == "th" => {
                self.clear_stack_to(&["tr", "html", "table"]);
                self.insert_element(&name, &attributes);
                self.afe_marker();
                self.mode = Mode::InCell;
            }
            Token::EndTag { name } if name == "tr" => {
                if self.in_scope("tr", SCOPE_TABLE) {
                    self.clear_stack_to(&["tr", "html", "table"]);
                    self.stack.pop();
                    self.mode = Mode::InTableBody;
                }
            }
            Token::StartTag { name, .. }
                if matches!(
                    name.as_str(),
                    "caption" | "col" | "colgroup" | "tbody" | "tfoot" | "thead" | "tr"
                ) =>
            {
                self.row_to_table_body(tokenizer, token);
            }
            Token::EndTag { name }
                if matches!(name.as_str(), "table" | "tbody" | "tfoot" | "thead") =>
            {
                self.row_to_table_body(tokenizer, token);
            }
            Token::EndTag { name }
                if matches!(
                    name.as_str(),
                    "body" | "caption" | "col" | "colgroup" | "html" | "td" | "th"
                ) => {}
            _ => self.step_in_table(tokenizer, token),
        }
    }

    /// `</table>`/section/`tr` dans une ligne : ferme `tr` puis
    /// retraite en `InTableBody`.
    fn row_to_table_body(&mut self, tokenizer: &mut Tokenizer, token: Token) {
        if self.in_scope("tr", SCOPE_TABLE) {
            self.clear_stack_to(&["tr", "html", "table"]);
            self.stack.pop();
            self.mode = Mode::InTableBody;
            self.dispatch(tokenizer, token);
        }
    }

    fn step_in_cell(&mut self, tokenizer: &mut Tokenizer, token: Token) {
        match token.clone() {
            Token::EndTag { name } if name == "td" || name == "th" => {
                if self.close_element_in_scope(&name, SCOPE_TABLE) {
                    self.afe_clear_to_marker();
                    self.mode = Mode::InRow;
                }
            }
            Token::StartTag { name, .. }
                if matches!(
                    name.as_str(),
                    "caption"
                        | "col"
                        | "colgroup"
                        | "tbody"
                        | "tfoot"
                        | "thead"
                        | "tr"
                        | "td"
                        | "th"
                ) =>
            {
                self.close_cell_reprocess(tokenizer, token);
            }
            Token::EndTag { name }
                if matches!(name.as_str(), "table" | "tbody" | "tfoot" | "thead" | "tr") =>
            {
                self.close_cell_reprocess(tokenizer, token);
            }
            Token::EndTag { name }
                if matches!(
                    name.as_str(),
                    "body" | "caption" | "col" | "colgroup" | "html"
                ) => {}
            _ => self.step_in_body(tokenizer, token),
        }
    }

    /// Ferme la cellule ouverte (`td` prioritaire, sinon `th`) puis
    /// retraite le token.
    fn close_cell_reprocess(&mut self, tokenizer: &mut Tokenizer, token: Token) {
        for cell in ["td", "th"] {
            if self.in_scope(cell, SCOPE_TABLE) {
                self.close_element_in_scope(cell, SCOPE_TABLE);
                self.afe_clear_to_marker();
                self.mode = Mode::InRow;
                self.dispatch(tokenizer, token);
                return;
            }
        }
    }

    // -----------------------------------------------------------------
    // Select
    // -----------------------------------------------------------------

    fn step_in_select(&mut self, tokenizer: &mut Tokenizer, token: Token) {
        match token {
            Token::Doctype { .. } => {}

            Token::Character(s) => self.insert_text(&s),
            Token::Comment(c) => self.insert_comment(&c),
            Token::StartTag {
                name,
                attributes,
                self_closing,
            } => match name.as_str() {
                "html" => self.step_in_body(
                    tokenizer,
                    Token::StartTag {
                        name,
                        attributes,
                        self_closing,
                    },
                ),
                "option" => {
                    if self.current_is("option") {
                        self.stack.pop();
                    }
                    self.insert_element("option", &attributes);
                }
                "optgroup" => {
                    if self.current_is("option") {
                        self.stack.pop();
                    }
                    if self.current_is("optgroup") {
                        self.stack.pop();
                    }
                    self.insert_element("optgroup", &attributes);
                }
                "select" => {
                    // `<select>` dans `<select>` = `</select>`.
                    if self.in_scope("select", SCOPE_TABLE) {
                        self.close_element_in_scope("select", SCOPE_TABLE);
                        self.reset_mode();
                    }
                }
                "input" | "keygen" | "textarea" => {
                    if self.in_scope("select", SCOPE_TABLE) {
                        self.close_element_in_scope("select", SCOPE_TABLE);
                        self.reset_mode();
                        self.dispatch(
                            tokenizer,
                            Token::StartTag {
                                name,
                                attributes,
                                self_closing,
                            },
                        );
                    }
                }
                "script" | "template" => self.step_in_head(
                    tokenizer,
                    Token::StartTag {
                        name,
                        attributes,
                        self_closing,
                    },
                ),
                _ => {}
            },
            Token::EndTag { name } => match name.as_str() {
                "optgroup" => {
                    if self.current_is("option") {
                        self.stack.pop();
                    }
                    if self.current_is("optgroup") {
                        self.stack.pop();
                    }
                }
                "option" => {
                    if self.current_is("option") {
                        self.stack.pop();
                    }
                }
                "select" => {
                    if self.in_scope("select", SCOPE_TABLE) {
                        self.close_element_in_scope("select", SCOPE_TABLE);
                        self.reset_mode();
                    }
                }
                "template" => self.step_in_head(tokenizer, Token::EndTag { name }),
                _ => {}
            },
            Token::Eof => self.step_in_body(tokenizer, Token::Eof),
        }
    }

    // -----------------------------------------------------------------
    // Fins de document
    // -----------------------------------------------------------------

    fn step_after_body(&mut self, tokenizer: &mut Tokenizer, token: Token) {
        match token {
            Token::Character(s) => {
                let (ws, rest) = split_ws(&s);
                if !ws.is_empty() {
                    // ws après body → dans body (spec : InBody).
                    let saved = self.mode;
                    self.mode = Mode::InBody;
                    self.insert_text(ws);
                    self.mode = saved;
                }
                if rest.is_empty() {
                    return;
                }
                self.mode = Mode::InBody;
                self.dispatch(tokenizer, Token::Character(rest.to_owned()));
            }
            Token::Comment(c) => {
                // Commentaire enfant de <html> (premier élément).
                if let Some(&html) = self.stack.first() {
                    if let Ok(node) = self.dom.create_comment(&c) {
                        let _ = self.dom.append_child(html, node);
                    }
                }
            }
            Token::StartTag {
                name,
                attributes,
                self_closing,
            } if name == "html" => {
                self.step_in_body(
                    tokenizer,
                    Token::StartTag {
                        name,
                        attributes,
                        self_closing,
                    },
                );
            }
            Token::EndTag { name } if name == "html" => {
                self.mode = Mode::AfterAfterBody;
            }
            Token::Doctype { .. } | Token::Eof => {}
            _ => {
                // Contenu après </body> : on reste ouvert (spec —
                // le navigateur tolère).
                self.mode = Mode::InBody;
                self.dispatch(tokenizer, token);
            }
        }
    }

    fn step_after_after_body(&mut self, tokenizer: &mut Tokenizer, token: Token) {
        match token {
            Token::Comment(c) => self.insert_comment_document(&c),
            Token::Character(s) if all_ws(&s) => {
                let saved = self.mode;
                self.mode = Mode::InBody;
                self.insert_text(&s);
                self.mode = saved;
            }
            Token::StartTag {
                name,
                attributes,
                self_closing,
            } if name == "html" => {
                self.step_in_body(
                    tokenizer,
                    Token::StartTag {
                        name,
                        attributes,
                        self_closing,
                    },
                );
            }
            Token::Eof => {}
            _ => {
                self.mode = Mode::InBody;
                self.dispatch(tokenizer, token);
            }
        }
    }
}

impl Default for TreeBuilder {
    fn default() -> Self {
        Self::new()
    }
}

const QUIRKS_PREFIX: &[&str] = &[
    "+//silmaril//dtd html pro v0r11 19970101//",
    "-//advasoft ltd//dtd html 4.0 transitional//",
    "-//as//dtd html 3.0 aswedit + extensions//",
    "-//ietf//dtd html 2.0 level 1//",
    "-//ietf//dtd html 2.0 level 2//",
    "-//ietf//dtd html 2.0 strict level 1//",
    "-//ietf//dtd html 2.0 strict level 2//",
    "-//ietf//dtd html 2.0 strict//",
    "-//ietf//dtd html 2.0//",
    "-//ietf//dtd html 2.1e//",
    "-//ietf//dtd html 3.0//",
    "-//ietf//dtd html 3.2 final//",
    "-//ietf//dtd html 3.2//",
    "-//ietf//dtd html 3//",
    "-//ietf//dtd html level 0//",
    "-//ietf//dtd html level 1//",
    "-//ietf//dtd html level 2//",
    "-//ietf//dtd html level 3//",
    "-//ietf//dtd html strict level 0//",
    "-//ietf//dtd html strict level 1//",
    "-//ietf//dtd html strict level 2//",
    "-//ietf//dtd html strict level 3//",
    "-//ietf//dtd html strict//",
    "-//ietf//dtd html//",
    "-//metrius//dtd metrius presentational//",
    "-//microsoft//dtd internet explorer 2.0 html strict//",
    "-//microsoft//dtd internet explorer 2.0 html//",
    "-//microsoft//dtd internet explorer 2.0 tables//",
    "-//microsoft//dtd internet explorer 3.0 html strict//",
    "-//microsoft//dtd internet explorer 3.0 html//",
    "-//microsoft//dtd internet explorer 3.0 tables//",
    "-//netscape comm. corp.//dtd html//",
    "-//netscape comm. corp.//dtd strict html//",
    "-//o'reilly and associates//dtd html 2.0//",
    "-//o'reilly and associates//dtd html extended 1.0//",
    "-//o'reilly and associates//dtd html extended relaxed 1.0//",
    "-//softquad software//dtd hotmetal pro 6.0::19990601::extensions to html 4.0//",
    "-//softquad//dtd hotmetal pro 4.0::19971010::extensions to html 4.0//",
    "-//spyglass//dtd html 2.0 extended//",
    "-//sq//dtd html 2.0 hotmetal + extensions//",
    "-//sun microsystems corp.//dtd hotjava html//",
    "-//sun microsystems corp.//dtd hotjava strict html//",
    "-//w3c//dtd html 3 1995-03-24//",
    "-//w3c//dtd html 3.2 draft//",
    "-//w3c//dtd html 3.2 final//",
    "-//w3c//dtd html 3.2//",
    "-//w3c//dtd html 3.2s draft//",
    "-//w3c//dtd html 4.0 frameset//",
    "-//w3c//dtd html 4.0 transitional//",
    "-//w3c//dtd html experimental 19960712//",
    "-//w3c//dtd html experimental 970421//",
    "-//w3c//dtd w3 html//",
    "-//w3o//dtd w3 html 3.0//",
    "-//webtechs//dtd mozilla html 2.0//",
    "-//webtechs//dtd mozilla html//",
];

/// Décide du mode de quirks d'après le DOCTYPE (sous-ensemble de
/// la liste de la spec : les chaînes marqueurs historiques).
fn quirks_mode(
    force: bool,
    name: &str,
    public_id: Option<&str>,
    system_id: Option<&str>,
) -> QuirksMode {
    if force || !name.eq_ignore_ascii_case("html") {
        return QuirksMode::Quirks;
    }
    // Seul ce system-id force quirks à lui seul (spec).
    if system_id.is_some_and(|s| {
        s.eq_ignore_ascii_case("http://www.ibm.com/data/dtd/v11/ibmxhtml1-transitional.dtd")
    }) {
        return QuirksMode::Quirks;
    }
    let Some(public) = public_id else {
        // Public absent : aucune règle quirks ne s'applique.
        return QuirksMode::NoQuirks;
    };
    let p = public.to_ascii_lowercase();
    // Préfixes public-id → quirks inconditionnel (spec, liste
    // complète ; comparaison ASCII insensible à la casse).
    if p == "-//w3o//dtd w3 html strict 3.0//en//"
        || p == "-/w3c/dtd html 4.0 transitional/en"
        || p == "html"
        || QUIRKS_PREFIX.iter().any(|pre| p.starts_with(pre))
    {
        return QuirksMode::Quirks;
    }
    // Spec : « missing **or the empty string** » pour la règle
    // quirks 4.01 ; « neither missing nor the empty string » pour
    // la règle limited correspondante. Frameset/Transitional
    // seulement — le STRICT 4.01 sans system reste NoQuirks.
    let sys_missing = system_id.is_none_or(str::is_empty);
    if p.starts_with("-//w3c//dtd html 4.01 frameset//")
        || p.starts_with("-//w3c//dtd html 4.01 transitional//")
    {
        return if sys_missing {
            QuirksMode::Quirks
        } else {
            QuirksMode::LimitedQuirks
        };
    }
    if p.starts_with("-//w3c//dtd xhtml 1.0 frameset//")
        || p.starts_with("-//w3c//dtd xhtml 1.0 transitional//")
    {
        return QuirksMode::LimitedQuirks;
    }
    QuirksMode::NoQuirks
}

/// Sépare le préfixe whitespace (espaces/tabs/newlines HTML) du
/// reste — les modes qui ignorent le ws pur en ont besoin.
fn split_ws(s: &str) -> (&str, &str) {
    let end = s
        .find(|c: char| !matches!(c, '\t' | '\n' | '\x0C' | '\r' | ' '))
        .unwrap_or(s.len());
    (&s[..end], &s[end..])
}

/// Le token ne contient que des espaces HTML (`\t\n\f\r `).
fn all_ws(s: &str) -> bool {
    s.chars()
        .all(|c| matches!(c, '\t' | '\n' | '\x0C' | '\r' | ' '))
}
