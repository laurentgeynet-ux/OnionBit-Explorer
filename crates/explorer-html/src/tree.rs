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

use explorer_dom::{DocumentMode, Dom, Namespace, NodeId};

use crate::foreign;
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
    InHeadNoscript,
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
    InSelectInTable,
    InTemplate,
    InFrameset,
    AfterFrameset,
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

/// Terminateurs du scope par défaut (in-scope) — inclut les
/// points d'intégration MathML/SVG de la spec (R-15).
/// `select` est borne de scope depuis la suppression du mode
/// « in select » (WHATWG PR 10557) : un `</font>` sous `<select>`
/// ne peut pas atteindre un `font` ancêtre du select → ignoré,
/// alors qu'un `</i>` ouvert **à l'intérieur** du select reste
/// en scope et suit l'AAA normalement.
const SCOPE_DEFAULT: &[&str] = &[
    "applet",
    "caption",
    "html",
    "table",
    "td",
    "th",
    "marquee",
    "object",
    "select",
    "template",
    "mi",
    "mo",
    "mn",
    "ms",
    "mtext",
    "annotation-xml",
    "foreignObject",
    "desc",
    "title",
];

/// Terminateurs du scope bouton (`button` en plus).
const SCOPE_BUTTON: &[&str] = &[
    "applet",
    "button",
    "caption",
    "html",
    "table",
    "td",
    "th",
    "marquee",
    "object",
    "select",
    "template",
    "mi",
    "mo",
    "mn",
    "ms",
    "mtext",
    "annotation-xml",
    "foreignObject",
    "desc",
    "title",
];

/// Terminateurs du scope liste (`ul`/`ol` en plus).
const SCOPE_LIST: &[&str] = &[
    "applet",
    "caption",
    "html",
    "ol",
    "table",
    "td",
    "th",
    "marquee",
    "object",
    "select",
    "template",
    "ul",
    "mi",
    "mo",
    "mn",
    "ms",
    "mtext",
    "annotation-xml",
    "foreignObject",
    "desc",
    "title",
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
    // `noscript` absent : spec — catégorie « special » seulement
    // si scripting activé ; hors de ce cas c'est un élément
    // ordinaire (contenu = sous-arbre DOM).
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
    // Éléments étrangers « spéciaux » (spec — points d'intégration,
    // R-15).
    "foreignObject",
    "desc",
    "mi",
    "mo",
    "mn",
    "ms",
    "mtext",
    "annotation-xml",
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
/// `noscript` n'y figure **pas** : sans scripting (notre cas), son
/// contenu est un vrai sous-arbre DOM (spec §13.2.6.4.1).
const RAWTEXT: &[&str] = &["iframe", "noembed", "noframes", "style", "xmp"];

/// Profondeur maximale de la pile d'éléments ouverts — au-delà,
/// le tag ouvrant est ignoré (dégradation tolérante). Sans cette
/// borne, un document hostile (`<div>`×10⁶) produirait un arbre
/// dont les traversées récursives aval (boxtree, display list)
/// dépasseraient la pile d'appels. 512 ≈ limites observées dans
/// les navigateurs réels.
const MAX_DEPTH: usize = 512;

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
    "search",
    "section",
    "summary",
    "ul",
];

/// Contexte de table pour le foster parenting (spec : ces cinq
/// éléments uniquement).
const TABLE_CONTEXT: &[&str] = &["table", "tbody", "tfoot", "thead", "tr"];

/// Cibles du « clear back to a table context » de la spec
/// (`html`, `table`, `template` — distinct de `TABLE_CONTEXT`).
const TABLE_CONTEXT_CLEAR: &[&str] = &["html", "table", "template"];

/// Tree-builder : consomme des tokens, produit un [`Dom`].
// Les drapeaux de mode (`frameset_ok`, `foster_parenting`,
// `skip_leading_lf`, `scripting`) sont des bools atomiques de la
// machine d'insertion — la spec les énonce comme tels ; les
// fusionner en enum d'état briserait leur orthogonalité.
#[allow(clippy::struct_excessive_bools)]
pub struct TreeBuilder {
    dom: Dom,
    /// Tokenizer interne de la conduite [`TreeBuilder::push`] /
    /// [`TreeBuilder::finish`] — sorti par `mem::take` le temps
    /// de chaque boucle (`process` exige `&mut self` ET
    /// `&mut Tokenizer`, les deux ne peuvent pas se prêter
    /// simultanément).
    tokenizer: Tokenizer,
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
    /// Pile des modes d'insertion « template » (spec §13.2.4.1).
    template_modes: Vec<Mode>,
    /// « scripting flag » de la spec (§13.2.3.5) : quand il est
    /// levé, `<noscript>` est du **rawtext** — son contenu reste
    /// texte et ne devient jamais des éléments (le fallback ne
    /// s'affiche que sans scripting). `false` par défaut =
    /// comportement historique (scripting off, parse complet) ;
    /// l'appelant qui exécute du JS le lève via
    /// [`TreeBuilder::set_scripting`].
    scripting: bool,
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
            tokenizer: Tokenizer::new(),
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
            template_modes: Vec::new(),
            scripting: false,
        }
    }

    /// Lève le « scripting flag » (CAP-155 — spec §13.2.3.5) :
    /// `<noscript>` bascule en contenu rawtext — ses enfants ne
    /// sont plus jamais des éléments (un moteur JS actif n'affiche
    /// pas le fallback). À appeler **avant** le premier
    /// [`TreeBuilder::push`]/[`TreeBuilder::process`] — un
    /// basculement en cours de parse ne revisite pas le passé.
    pub fn set_scripting(&mut self, scripting: bool) {
        self.scripting = scripting;
    }

    /// DOM consommé une fois la construction terminée.
    #[must_use]
    pub fn into_dom(mut self) -> Dom {
        self.dom.set_document_mode(document_mode_of(self.quirks));
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

    /// Raccourci complet : `push` + `finish` — pour tests et
    /// `cli dump` (la conduite bas niveau externe passe par
    /// [`TreeBuilder::process`]).
    #[must_use]
    pub fn parse(input: &str) -> Dom {
        let mut tb = Self::new();
        tb.push(input);
        tb.finish();
        tb.into_dom()
    }

    /// [`TreeBuilder::parse`] avec « scripting flag » levé
    /// (CAP-155) — raccourci pour les documents dont le JS
    /// tournera (`<noscript>` = rawtext, spec §13.2.3.5).
    #[must_use]
    pub fn parse_scripting(input: &str, scripting: bool) -> Dom {
        let mut tb = Self::new();
        tb.set_scripting(scripting);
        tb.push(input);
        tb.finish();
        tb.into_dom()
    }

    /// Alimente le builder par chunk — conduite **incrémentale**
    /// (ADR-0012) : le tokenizer interne conserve les balises et
    /// entités coupées entre deux appels, tous les tokens prêts
    /// sont consommés. Entre deux `push`, [`TreeBuilder::dom`]
    /// expose un arbre **cohérent à frontière de token** — un
    /// état peignable, jamais transitoire (peinture progressive,
    /// R-1 ; la cadence de peinture relève de l'appelant).
    ///
    /// Invariant utile à l'aval : tant que le builder vit, aucun
    /// nœud inséré n'est collecté — les `NodeId` vus dans une
    /// frame intermédiaire restent résolubles aux frames
    /// suivantes.
    ///
    /// `push` après [`TreeBuilder::finish`] est ignoré : le flux
    /// est clos. Ne pas mélanger avec [`TreeBuilder::drive`] /
    /// [`TreeBuilder::process`] — conduite externe à tokenizer
    /// distinct, les deux états divergeraient.
    pub fn push(&mut self, chunk: &str) {
        if self.tokenizer.is_finished() {
            return;
        }
        let mut t = std::mem::take(&mut self.tokenizer);
        t.feed(chunk);
        self.drive(&mut t);
        self.tokenizer = t;
    }

    /// Clôt le flux (fin de téléchargement) : résout les états
    /// suspendus du tokenizer (commentaire ouvert, `<` orphelin…)
    /// puis joue l'`Eof` de la spec — les éléments encore ouverts
    /// sont réputés clos, le document est complet. Idempotent.
    ///
    /// Le DOM final se récupère via [`TreeBuilder::into_dom`] (ou
    /// reste observable via [`TreeBuilder::dom`]).
    pub fn finish(&mut self) {
        let mut t = std::mem::take(&mut self.tokenizer);
        t.finish();
        self.drive(&mut t);
        self.tokenizer = t;
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
        // `<selectedcontent>` : à fin de document, tout `<select>`
        // encore ouvert voit son option sélectionnée miroirée
        // (spec customizable-select — corpus webkit02).
        self.mirror_selectedcontent_open();
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
        // Contenu étranger (§13.2.6.5) : hors points d'intégration,
        // les tokens suivent leurs propres règles.
        if self.uses_foreign_rules(&token) {
            self.step_foreign(tokenizer, token);
            return;
        }
        self.dispatch_mode(tokenizer, token);
    }

    /// Routage par mode d'insertion — sans le gate étranger
    /// (utilisé par la délégation interne de `step_foreign`).
    fn dispatch_mode(&mut self, tokenizer: &mut Tokenizer, token: Token) {
        match self.mode {
            Mode::Initial => self.step_initial(tokenizer, token),
            Mode::BeforeHtml => self.step_before_html(tokenizer, token),
            Mode::BeforeHead => self.step_before_head(tokenizer, token),
            Mode::InHead => self.step_in_head(tokenizer, token),
            Mode::InHeadNoscript => self.step_in_head_noscript(tokenizer, token),
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
            Mode::InSelectInTable => self.step_in_select_in_table(tokenizer, token),
            Mode::InTemplate => self.step_in_template(tokenizer, token),
            Mode::InFrameset => self.step_in_frameset(tokenizer, token),
            Mode::AfterFrameset => self.step_after_frameset(tokenizer, token),
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

    /// Un élément **HTML** de ce nom est-il sur la pile des
    /// éléments ouverts (un `svg template` n'est pas un
    /// `<template>` — corpus template.dat).
    fn stack_has(&self, name: &str) -> bool {
        self.stack.iter().any(|&id| self.is_html_named(id, name))
    }

    /// Crée un élément avec ses attributs et l'insère au point
    /// approprié (foster parenting si actif et contexte de table),
    /// puis le pousse sur la pile.
    fn insert_element(&mut self, name: &str, attrs: &[TokenAttribute]) -> Option<NodeId> {
        // Borne de profondeur : le token est ignoré — son contenu
        // reste inséré dans le parent courant (dégradation spec-
        // like « ignore the token »).
        if self.stack.len() >= MAX_DEPTH {
            return None;
        }
        let el = self.dom.create_element(name).ok()?;
        if name == "template" {
            // Fragment `content` propre au template (spec/R-15) —
            // les descendants y seront redirigés.
            if let Ok(frag) = self.dom.create_fragment() {
                let _ = self.dom.set_template_contents(el, frag);
            }
        }
        for a in attrs {
            let _ = self.dom.set_attribute(el, &a.name, &a.value);
        }
        self.insert_node(el);
        self.stack.push(el);
        Some(el)
    }

    /// Cible d'insertion effective : un `<template>` ouvert
    /// redirige vers son fragment `content` (spec « appropriate
    /// place for inserting a node », R-15).
    fn insertion_parent(&self, node: NodeId) -> NodeId {
        if self.dom.namespace(node).ok() == Some(Namespace::Html)
            && self.dom.tag_name(node).ok() == Some("template")
        {
            if let Ok(Some(frag)) = self.dom.template_contents(node) {
                return frag;
            }
        }
        node
    }

    /// Insère un nœud au point d'insertion approprié : foster
    /// parenting quand le contexte courant est une table et que le
    /// flag est levé, sinon enfant du courant (redirigé vers
    /// `content` si c'est un template).
    fn insert_node(&mut self, node: NodeId) {
        if self.foster_parenting && self.in_table_context() {
            self.foster_insert(node);
            return;
        }
        let parent = self
            .current()
            .map_or_else(|| self.dom.document(), |c| self.insertion_parent(c));
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

    /// `<template>` ouvert le plus récent dans la pile.
    fn recent_template(&self) -> Option<NodeId> {
        self.stack
            .iter()
            .rev()
            .find(|&&id| {
                self.dom.tag_name(id).ok() == Some("template")
                    && self.dom.namespace(id).ok() == Some(Namespace::Html)
            })
            .copied()
    }

    /// Foster parenting (spec « appropriate place ») : le
    /// `content` du dernier `<template>` gagne s'il est plus
    /// récent que la dernière `<table>` ; sinon insertion avant
    /// la table, ou en fin de l'élément qui la précède dans la
    /// pile, sinon dans le document.
    fn foster_insert(&mut self, node: NodeId) {
        let tmpl_pos = self
            .recent_template()
            .and_then(|t| self.stack.iter().rposition(|&x| x == t));
        let table_pos = self
            .last_table
            .and_then(|t| self.stack.iter().rposition(|&x| x == t));
        if let Some(tp) = tmpl_pos {
            if table_pos.is_none_or(|tpb| tp > tpb) {
                if let Some(tmpl) = self.recent_template() {
                    if let Ok(Some(frag)) = self.dom.template_contents(tmpl) {
                        let _ = self.dom.append_child(frag, node);
                        return;
                    }
                }
            }
        }
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
        let parent = self
            .current()
            .map_or_else(|| self.dom.document(), |c| self.insertion_parent(c));
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

    /// Texte en foster parenting : `content` du dernier template
    /// si plus récent que la table, sinon repérage avant la table.
    fn foster_text(&mut self, data: &str) {
        let tmpl_pos = self
            .recent_template()
            .and_then(|t| self.stack.iter().rposition(|&x| x == t));
        let table_pos = self
            .last_table
            .and_then(|t| self.stack.iter().rposition(|&x| x == t));
        if let Some(tp) = tmpl_pos {
            if table_pos.is_none_or(|tpb| tp > tpb) {
                if let Some(tmpl) = self.recent_template() {
                    if let Ok(Some(frag)) = self.dom.template_contents(tmpl) {
                        self.append_text_to(frag, data);
                        return;
                    }
                }
            }
        }
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
    /// terminateur de `scope`. Sensibilité namespace (spec : les
    /// types sont des **tuples** ns+nom) : seul un élément **HTML**
    /// du nom compte (un `svg td` n'est pas un `td` — corpus
    /// `namespace-sensitivity.dat`) ; les terminateurs sont les
    /// noms listés en ns HTML + les IPs étrangers nommés.
    fn in_scope(&self, name: &str, scope: &[&str]) -> bool {
        for &id in self.stack.iter().rev() {
            let Ok(el) = self.dom.element(id) else {
                continue;
            };
            if el.namespace == Namespace::Html {
                if el.tag_name == name {
                    return true;
                }
                if scope.contains(&el.tag_name.as_str()) {
                    return false;
                }
            } else if foreign::is_foreign_scope_boundary(el) && scope.contains(&"mi") {
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
                .is_some_and(|id| !self.is_html_named(id, "p"))
            {
                self.stack.pop();
            }
            self.stack.pop();
        }
    }

    /// `id` désigne-t-il un élément **HTML** de nom `name`.
    fn is_html_named(&self, id: NodeId, name: &str) -> bool {
        self.dom
            .element(id)
            .is_ok_and(|e| e.namespace == Namespace::Html && e.tag_name == name)
    }

    /// Ferme l'élément `name` en scope : fins implicites puis dépile
    /// jusqu'à lui inclus.
    fn close_element_in_scope(&mut self, name: &str, scope: &[&str]) -> bool {
        if !self.in_scope(name, scope) {
            return false;
        }
        self.generate_implied_end_tags(name);
        while let Some(id) = self.stack.pop() {
            if self.is_html_named(id, name) {
                break;
            }
        }
        true
    }

    /// Dépile jusqu'à un contexte parmi `tags` (inclus).
    fn clear_stack_to(&mut self, tags: &[&str]) {
        while self.current().is_some_and(|id| {
            self.dom.element(id).is_ok_and(|e| {
                e.namespace != Namespace::Html || !tags.contains(&e.tag_name.as_str())
            })
        }) {
            self.stack.pop();
        }
    }

    /// `true` si la balise courante est « spéciale » (règle
    /// générique des end tags inconnus). Spec : la catégorie
    /// « special » couvre les éléments HTML listés **plus** les
    /// points d'intégration étrangers nommés (mi…mtext,
    /// annotation-xml, foreignObject, desc, title) — un `svg g`
    /// ou `svg tr` ordinaire n'est PAS special.
    fn is_special(&self, id: NodeId) -> bool {
        self.dom.element(id).is_ok_and(|e| {
            if e.namespace == Namespace::Html {
                SPECIAL.contains(&e.tag_name.as_str())
            } else {
                foreign::is_foreign_scope_boundary(e)
            }
        })
    }

    /// Restaure le mode d'insertion selon la pile réelle (spec
    /// « reset the insertion mode appropriately ») — après
    /// `</table>`, `</select>`…
    fn reset_mode(&mut self) {
        for (i, &id) in self.stack.iter().enumerate().rev() {
            let Ok(tag) = self.dom.tag_name(id) else {
                continue;
            };
            self.mode = match tag {
                // Spec « reset » : un `select` ancstralement sous
                // une `<table>` relève du mode « in select in
                // table » (les balises de table y ferment le
                // select) — corpus template.dat:101.
                "select" => {
                    if self.stack[..i]
                        .iter()
                        .any(|&low| self.is_html_named(low, "table"))
                    {
                        Mode::InSelectInTable
                    } else {
                        Mode::InSelect
                    }
                }
                "template" => self.template_modes.last().copied().unwrap_or(Mode::InBody),
                "head" => Mode::InHead,
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

    /// CDATA permis quand le nœud courant ajusté est étranger
    /// (spec — pas « un élément svg/math quelque part dans la
    /// pile »).
    fn update_cdata(&mut self, tokenizer: &mut Tokenizer) {
        let foreign = self
            .current()
            .and_then(|id| self.dom.namespace(id).ok())
            .is_some_and(|ns| ns != Namespace::Html);
        tokenizer.set_cdata_allowed(foreign);
    }

    // -----------------------------------------------------------------
    // Contenu étranger (§13.2.6.5, R-15)
    // -----------------------------------------------------------------

    /// `true` si le token doit être traité par les règles « in
    /// foreign content » plutôt que par le mode courant (spec :
    /// nœud courant ajusté hors namespace HTML, sauf points
    /// d'intégration pour les start tags / le texte, sauf Eof).
    fn uses_foreign_rules(&self, token: &Token) -> bool {
        if matches!(token, Token::Eof) {
            return false;
        }
        let Some(cur) = self.current() else {
            return false;
        };
        let Ok(el) = self.dom.element(cur) else {
            return false;
        };
        if el.namespace == Namespace::Html {
            return false;
        }
        match token {
            Token::StartTag { name, .. } => {
                if foreign::is_mathml_text_ip(el) {
                    return matches!(name.as_str(), "mglyph" | "malignmark");
                }
                if foreign::is_html_ip(el) {
                    return false;
                }
                if el.namespace == Namespace::MathMl && el.tag_name == "annotation-xml" {
                    return name != "svg";
                }
                true
            }
            Token::Character(_) => {
                if foreign::is_mathml_text_ip(el) || foreign::is_html_ip(el) {
                    return false;
                }
                true
            }
            // End tags, commentaires, doctype : règles étrangères.
            _ => true,
        }
    }

    /// « In foreign content » — texte/commentaires insérés tels
    /// quels, breakout list, balises et attributs ajustés.
    fn step_foreign(&mut self, tokenizer: &mut Tokenizer, token: Token) {
        match token {
            Token::Character(s) => {
                // Spec « in foreign content » : `\0` → U+FFFD. Le
                // `\0` ne coupe pas frameset_ok (html5lib : token
                // `\0` pur → `elif` court-circuité — corpus
                // plain-text-unsafe veut `<frameset>` fonctionnel
                // après `<svg>\0`).
                if s.chars()
                    .any(|c| c != '\0' && !matches!(c, '\t' | '\n' | '\x0C' | '\r' | ' '))
                {
                    self.frameset_ok = false;
                }
                let s: String = s
                    .chars()
                    .map(|c| if c == '\0' { '\u{FFFD}' } else { c })
                    .collect();
                self.insert_text(&s);
            }
            Token::Comment(c) => self.insert_comment(&c),
            Token::Doctype { .. } | Token::Eof => {}
            Token::StartTag {
                name,
                attributes,
                self_closing,
            } => {
                if foreign::is_breakout(&name, &attributes) {
                    self.pop_out_of_foreign();
                    self.dispatch_mode(
                        tokenizer,
                        Token::StartTag {
                            name,
                            attributes,
                            self_closing,
                        },
                    );
                    return;
                }
                // Enfant étranger : namespace = celui du nœud
                // courant ajusté (spec §13.2.6.5).
                let ns = self
                    .current()
                    .and_then(|id| self.dom.namespace(id).ok())
                    .filter(|ns| *ns != Namespace::Html)
                    .unwrap_or(Namespace::Svg);
                self.insert_foreign_element(&name, &attributes, ns, self_closing);
            }
            Token::EndTag { name } => {
                // Spec « in foreign content » : `</br>` et `</p>`
                // cassent la zone étrangère puis repassent au mode
                // courant en tant qu'end tag (in-body sait que
                // `</br>` vaut `<br>` et que `</p>` ferme le p).
                if name == "br" || name == "p" {
                    self.pop_out_of_foreign();
                    self.dispatch_mode(tokenizer, Token::EndTag { name });
                    return;
                }
                self.foreign_end_tag(tokenizer, &name);
            }
        }
    }

    /// Insère un élément étranger dans `ns` : balise/attributs
    /// ajustés en casse. `self_closing` acquitté → pas de push
    /// (spec).
    fn insert_foreign_element(
        &mut self,
        name: &str,
        attrs: &[TokenAttribute],
        ns: Namespace,
        self_closing: bool,
    ) {
        if self.stack.len() >= MAX_DEPTH {
            return;
        }
        let tag = foreign::adjust_tag(name, ns);
        let Ok(el) = self.dom.create_element_ns(tag, ns) else {
            return;
        };
        for a in attrs {
            let _ = self
                .dom
                .set_attribute(el, foreign::adjust_attr(&a.name, ns), &a.value);
        }
        self.insert_node(el);
        if !self_closing {
            self.stack.push(el);
        }
    }

    /// Dépile jusqu'à ce que le sommet soit un élément HTML ou un
    /// point d'intégration (spec breakout : la frontière **reste**
    /// en pile — ses futurs enfants relèvent des règles HTML).
    fn pop_out_of_foreign(&mut self) {
        while let Some(id) = self.current() {
            let Ok(el) = self.dom.element(id) else {
                break;
            };
            if el.namespace == Namespace::Html
                || foreign::is_mathml_text_ip(el)
                || foreign::is_html_ip(el)
            {
                break;
            }
            self.stack.pop();
        }
    }

    /// End tag en contenu étranger : si un élément correspondant
    /// existe au-dessus du premier nœud HTML, dépile jusqu'à lui ;
    /// sinon le token est retraité par le mode courant.
    fn foreign_end_tag(&mut self, tokenizer: &mut Tokenizer, name: &str) {
        // Spec « any other end tag » (ordre html5lib) : en
        // descendant la pile — le **premier nœud HTML** redirige le
        // token vers le mode courant sans tester son nom (un
        // `<div>` HTML sous `foreignObject` n'est jamais fermé par
        // `</div>` : il retombe en in-body où le scope l'ignore) ;
        // un nœud étranger de nom correspondant tronque la pile
        // jusqu'à lui inclus.
        let mut idx = self.stack.len();
        while idx > 0 {
            idx -= 1;
            let id = self.stack[idx];
            let Ok(el) = self.dom.element(id) else {
                continue;
            };
            if el.namespace == Namespace::Html {
                self.dispatch_mode(
                    tokenizer,
                    Token::EndTag {
                        name: name.to_owned(),
                    },
                );
                return;
            }
            if el.tag_name.eq_ignore_ascii_case(name) {
                self.stack.truncate(idx);
                return;
            }
        }
        // Aucun correspondant : token ignoré (parse error spec).
    }

    /// Miroir `selectedcontent` (select personnalisable, corpus
    /// webkit02) : pour chaque `<select>` HTML **ouvert**, le
    /// contenu de l'option sélectionnée — dernière option portant
    /// `selected`, sinon la première — est cloné dans le
    /// descendant `<selectedcontent>` (sous `<button>`).
    fn mirror_selectedcontent_open(&mut self) {
        let selects: Vec<NodeId> = self
            .stack
            .iter()
            .filter(|&&id| self.is_html_named(id, "select"))
            .copied()
            .collect();
        for select in selects {
            // Descendants en ordre du document (DFS).
            let mut order = Vec::new();
            self.collect_descendants(select, &mut order);
            let target = order
                .iter()
                .find(|&&id| self.is_html_named(id, "selectedcontent"))
                .copied();
            let Some(target) = target else {
                continue;
            };
            let options: Vec<NodeId> = order
                .iter()
                .filter(|&&id| self.is_html_named(id, "option"))
                .copied()
                .collect();
            let selected = options
                .iter()
                .rev()
                .find(|&&id| self.dom.attribute(id, "selected").ok().flatten().is_some())
                .copied()
                .or_else(|| options.first().copied());
            let Some(selected) = selected else {
                continue;
            };
            // Purge puis clone profond des enfants de l'option.
            for child in self.dom.children(target).unwrap_or_default() {
                let _ = self.dom.remove(child);
            }
            for child in self.dom.children(selected).unwrap_or_default() {
                self.clone_subtree_into(child, target);
            }
        }
    }

    /// DFS des descendants d'un nœud (ordre du document).
    fn collect_descendants(&self, root: NodeId, out: &mut Vec<NodeId>) {
        for child in self.dom.children(root).unwrap_or_default() {
            out.push(child);
            self.collect_descendants(child, out);
        }
    }

    /// Clone profond : `src` (élément/texte/commentaire) copié
    /// sous `parent` avec toute sa descendance.
    fn clone_subtree_into(&mut self, src: NodeId, parent: NodeId) {
        let clone = match self.dom.data(src).cloned() {
            Ok(explorer_dom::NodeData::Element(e)) => {
                let Ok(el) = self.dom.create_element_ns(&e.tag_name, e.namespace) else {
                    return;
                };
                for a in &e.attributes {
                    let _ = self.dom.set_attribute(el, &a.name, &a.value);
                }
                el
            }
            Ok(explorer_dom::NodeData::Text(t)) => {
                let Ok(t) = self.dom.create_text(&t) else {
                    return;
                };
                t
            }
            Ok(explorer_dom::NodeData::Comment(c)) => {
                let Ok(c) = self.dom.create_comment(&c) else {
                    return;
                };
                c
            }
            _ => return,
        };
        if self.dom.append_child(parent, clone).is_err() {
            return;
        }
        for child in self.dom.children(src).unwrap_or_default() {
            self.clone_subtree_into(child, clone);
        }
    }

    // -----------------------------------------------------------------
    // Liste des éléments de format actifs (AFE)
    // -----------------------------------------------------------------

    fn afe_push(&mut self, el: NodeId) {
        // Règle de Noé : max 3 entrées **identiques** — tag +
        // attributs égaux, la plus ancienne saute (spec).
        if self.dom.tag_name(el).is_ok() {
            let mut same = self
                .afe
                .iter()
                .enumerate()
                .rev()
                .take_while(|(_, e)| matches!(e, Afe::Element(_)))
                .filter(|(_, e)| {
                    matches!(e, Afe::Element(id) if {
                        let Ok(other) = self.dom.element(*id) else {
                            return false;
                        };
                        let Ok(this) = self.dom.element(el) else {
                            return false;
                        };
                        other.namespace == this.namespace
                            && other.tag_name == this.tag_name
                            && attrs_equal(&other.attributes, &this.attributes)
                    })
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

    /// `true` si `el` est présent (élément) dans la liste des
    /// éléments de formatage actifs.
    fn afe_contains(&self, el: NodeId) -> bool {
        self.afe
            .iter()
            .any(|e| matches!(e, Afe::Element(id) if *id == el))
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
                // Mode synchronisé sur le DOM dès la décision :
                // la cascade le lit aussi sur un arbre partiel
                // (peinture progressive).
                self.dom.set_document_mode(document_mode_of(self.quirks));
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
            Token::Character(s) => {
                // Chaque caractère est un token : le ws de tête est
                // ignoré, le premier non-ws force le quirks.
                let (_, rest) = split_ws(&s);
                if rest.is_empty() {
                    return;
                }
                self.quirks = QuirksMode::Quirks;
                self.dom.set_document_mode(DocumentMode::Quirks);
                self.mode = Mode::BeforeHtml;
                self.dispatch(tokenizer, Token::Character(rest.to_owned()));
            }
            _ => {
                self.quirks = QuirksMode::Quirks;
                self.dom.set_document_mode(DocumentMode::Quirks);
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
            Token::Character(s) => {
                let (_, rest) = split_ws(&s);
                if rest.is_empty() {
                    return;
                }
                self.insert_element("html", &[]);
                self.mode = Mode::BeforeHead;
                self.dispatch(tokenizer, Token::Character(rest.to_owned()));
            }
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
            Token::Character(s) => {
                let (_, rest) = split_ws(&s);
                if rest.is_empty() {
                    return;
                }
                self.head = self.insert_element("head", &[]);
                self.mode = Mode::InHead;
                self.dispatch(tokenizer, Token::Character(rest.to_owned()));
            }
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
                    if self.scripting {
                        // Scripting on (CAP-155 — spec
                        // §13.2.6.4.1) : rawtext, comme
                        // `noframes` — le contenu reste texte.
                        self.insert_element(&name, &attributes);
                        tokenizer.set_content_state(ContentKind::RawText);
                        self.original_mode = self.mode;
                        self.mode = Mode::Text;
                    } else {
                        // Scripting off → « in head noscript ».
                        self.insert_element(&name, &attributes);
                        self.mode = Mode::InHeadNoscript;
                    }
                }
                "template" => {
                    self.insert_element(&name, &attributes);
                    self.afe_marker();
                    self.frameset_ok = false;
                    self.template_modes.push(Mode::InTemplate);
                    self.mode = Mode::InTemplate;
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
                "template" => self.close_template(),
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
                    self.mode = Mode::InFrameset;
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

    /// Mode « in head noscript » (spec §13.2.6.4.2) — scripting off.
    /// Contenu admis : whitespace, commentaires, `link/meta/style/
    /// basefont/bgsound/noframes` ; tout le reste referme noscript
    /// et repasse en « in head ».
    fn step_in_head_noscript(&mut self, tokenizer: &mut Tokenizer, token: Token) {
        let close_and_reprocess = matches!(
            token,
            Token::Eof | Token::EndTag { .. } | Token::StartTag { .. } | Token::Character(_)
        ) && !matches!(&token, Token::Character(s) if all_ws(s));
        match token {
            Token::Comment(c) => self.insert_comment(&c),
            Token::Character(s) if all_ws(&s) => {
                self.step_in_head(tokenizer, Token::Character(s));
            }
            Token::StartTag {
                name,
                attributes,
                self_closing,
            } if matches!(
                name.as_str(),
                "basefont" | "bgsound" | "link" | "meta" | "noframes" | "style"
            ) =>
            {
                self.step_in_head(
                    tokenizer,
                    Token::StartTag {
                        name,
                        attributes,
                        self_closing,
                    },
                );
            }
            // Spec : `<html>` relève des règles « in body » et ne
            // referme **pas** le noscript.
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
            Token::EndTag { ref name } if name == "noscript" => {
                if self.current_is("noscript") {
                    self.stack.pop();
                }
                self.mode = Mode::InHead;
            }
            // html5lib : seul `</br>` (parmi les end tags) referme
            // le noscript ; tout autre end tag est un parse error
            // **ignoré** (`</p>` laisse noscript ouvert — le
            // commentaire qui suit reste dedans).
            Token::EndTag { ref name } if name == "br" => {
                if self.current_is("noscript") {
                    self.stack.pop();
                }
                self.mode = Mode::InHead;
                self.dispatch(tokenizer, token);
            }
            _ if close_and_reprocess => {
                // `head`/`noscript` ouvrants → ignorés (parse error).
                if let Token::StartTag { ref name, .. } = token {
                    if matches!(name.as_str(), "head" | "noscript") {
                        return;
                    }
                }
                // End tags inconnus → parse error ignoré, noscript
                // reste ouvert.
                if matches!(token, Token::EndTag { .. }) {
                    return;
                }
                // Le reste (texte, starts, EOF) : fin implicite de
                // noscript + rejet du token en « in head ».
                if self.current_is("noscript") {
                    self.stack.pop();
                }
                self.mode = Mode::InHead;
                self.dispatch(tokenizer, token);
            }
            _ => {}
        }
    }

    // -----------------------------------------------------------------
    // InBody — le gros de la spec
    // -----------------------------------------------------------------

    fn step_in_body(&mut self, tokenizer: &mut Tokenizer, token: Token) {
        match token {
            Token::Character(s) => {
                // Spec : `\0` est ignoré en « in body » — sans
                // invalider `frameset_ok`.
                let s: String = s.chars().filter(|&c| c != '\0').collect();
                if s.is_empty() {
                    return;
                }
                if !all_ws(&s) {
                    self.frameset_ok = false;
                }
                self.reconstruct_afe();
                self.insert_text(&s);
            }
            Token::Comment(c) => self.insert_comment(&c),
            Token::Eof => {
                // Spec : un template encore ouvert → règles
                // « in template » (le template se ferme et le
                // document reprend son cours : `</head>`, body…).
                if self.stack_has("template") {
                    self.step_in_template(tokenizer, Token::Eof);
                }
            }
            Token::Doctype { .. } => {}
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
                // Spec : ignoré si un `<template>` est ouvert,
                // sinon fusion des attributs sur <html> existant.
                if self.recent_template().is_none() {
                    if let Some(&html) = self.stack.first() {
                        for a in attributes {
                            if self.dom.attribute(html, &a.name).ok().flatten().is_none() {
                                let _ = self.dom.set_attribute(html, &a.name, &a.value);
                            }
                        }
                    }
                }
            }
            "body" => {
                if self.recent_template().is_none() {
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
            "frameset" => {
                // Spec : si `frameset_ok` et que le second élément
                // est `body` → le body sort de l'arbre et le
                // frameset prend sa place sous `html`.
                if self.frameset_ok
                    && self
                        .stack
                        .get(1)
                        .is_some_and(|&id| self.dom.tag_name(id).ok() == Some("body"))
                    && self.recent_template().is_none()
                {
                    let body = self.stack[1];
                    let _ = self.dom.remove(body);
                    self.stack.truncate(1); // garde seulement <html>
                    self.insert_element(name, attributes);
                    self.mode = Mode::InFrameset;
                }
                // sinon : token ignoré (parse error spec).
            }
            "form" => {
                if self.form.is_none() {
                    self.close_p();
                    self.form = self.insert_element(name, attributes);
                }
            }
            "li" => {
                // Spec : li/dd/dt verrouillent le document contre
                // le frameset.
                self.frameset_ok = false;
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
                self.frameset_ok = false;
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
            _ if FORMATTING.contains(&name) => {
                self.reconstruct_afe();
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
            "area" | "br" | "embed" | "img" | "keygen" | "wbr" | "hr" => {
                if name == "hr" {
                    self.close_p();
                } else {
                    self.reconstruct_afe();
                }
                self.insert_element(name, attributes);
                self.stack.pop();
                self.frameset_ok = false;
            }
            "param" | "source" | "track" => {
                // Spec : ces éléments vides ne verrouillent **pas**
                // le frameset (différence avec la liste ci-dessus).
                self.reconstruct_afe();
                self.insert_element(name, attributes);
                self.stack.pop();
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
            "noscript" if self.scripting => {
                // Scripting on (CAP-155 — spec §13.2.6.4.4,
                // même clause que les rawtext) : le contenu
                // reste texte — le fallback ne s'affiche que
                // sans scripting.
                self.insert_element(name, attributes);
                tokenizer.set_content_state(ContentKind::RawText);
                self.original_mode = self.mode;
                self.mode = Mode::Text;
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
                self.frameset_ok = false;
                // Spec : un contexte de table ouvert → « in select
                // in table » (les tokens de table repasseront en
                // InTable après fermeture du select).
                self.mode = if self.stack.iter().any(|&id| {
                    self.dom
                        .tag_name(id)
                        .is_ok_and(|t| matches!(t, "table" | "tbody" | "tfoot" | "thead" | "tr"))
                }) {
                    Mode::InSelectInTable
                } else {
                    Mode::InSelect
                };
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
                // Entrée en contenu étranger : élément namespacé
                // (R-15) — les descendants suivront les règles
                // « in foreign content » via `uses_foreign_rules`.
                self.reconstruct_afe();
                let ns = if name == "math" {
                    Namespace::MathMl
                } else {
                    Namespace::Svg
                };
                self.insert_foreign_element(name, attributes, ns, self_closing);
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

    fn body_end_tag(&mut self, tokenizer: &mut Tokenizer, name: &str) {
        match name {
            "body" => {
                if self.in_scope("body", SCOPE_DEFAULT) {
                    self.mode = Mode::AfterBody;
                }
            }
            "html" => {
                // Spec : `</html>` = `</body>` — le token est ensuite
                // **retraité** : en AfterBody il mène à AfterAfterBody
                // (les commentaires après `</html>` sont enfants du
                // document, pas de `<html>`).
                if self.in_scope("body", SCOPE_DEFAULT) {
                    self.mode = Mode::AfterBody;
                    self.dispatch_mode(
                        tokenizer,
                        Token::EndTag {
                            name: name.to_string(),
                        },
                    );
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
            "button" => {
                // Règle dédiée spec : `</button>` traverse les
                // implicites (un `p` interne « spécial » bloquerait
                // generic_end_tag).
                self.close_element_in_scope("button", SCOPE_DEFAULT);
            }
            "dd" | "dt" => {
                self.close_element_in_scope(name, SCOPE_LIST);
            }
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                // Spec : fins implicites puis dépile jusqu'au
                // **premier** heading rencontré — `</h1>` peut
                // fermer un `h3` interne (h1 reste alors ouvert).
                const HEADINGS: &[&str] = &["h1", "h2", "h3", "h4", "h5", "h6"];
                if HEADINGS.iter().any(|h| self.in_scope(h, SCOPE_DEFAULT)) {
                    self.generate_implied_end_tags("");
                    while let Some(top) = self.stack.pop() {
                        if self.dom.tag_name(top).is_ok_and(|t| HEADINGS.contains(&t)) {
                            break;
                        }
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
                self.adoption_agency(tokenizer, name);
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
            "template" => self.close_template(),
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

    /// `true` si `id` est en scope (aucun terminateur `scope`
    /// entre lui et le sommet de pile).
    fn id_in_scope(&self, id: NodeId) -> bool {
        for &other in self.stack.iter().rev() {
            if other == id {
                return true;
            }
            if let Ok(el) = self.dom.element(other) {
                let boundary = if el.namespace == Namespace::Html {
                    SCOPE_DEFAULT.contains(&el.tag_name.as_str())
                } else {
                    foreign::is_foreign_scope_boundary(el)
                };
                if boundary {
                    return false;
                }
            }
        }
        false
    }

    /// Clone un élément (tag, namespace, attributs) — spec
    /// « create an element for the token » des étapes AAA.
    fn clone_element(&mut self, src: NodeId) -> Option<NodeId> {
        let Ok(e) = self.dom.element(src) else {
            return None;
        };
        let (tag, ns, attrs) = (e.tag_name.clone(), e.namespace, e.attributes.clone());
        let node = self.dom.create_element_ns(&tag, ns).ok()?;
        for a in attrs {
            let _ = self.dom.set_attribute(node, &a.name, &a.value);
        }
        Some(node)
    }

    /// Adoption agency algorithm complet (spec §13.2.6.4.7, R-15) :
    /// ferme un élément de format en re-parentant les contenus
    /// croisés — `<b>x<i>y</b>z</i>` → `<b>x<i>y</i></b><i>z</i>`.
    #[allow(clippy::too_many_lines)]
    fn adoption_agency(&mut self, tokenizer: &mut Tokenizer, name: &str) {
        // 1. Sommet = sujet mais absent de l'AFE → simple dépile.
        if let Some(&top) = self.stack.last() {
            let in_afe = self
                .afe
                .iter()
                .any(|e| matches!(e, Afe::Element(id) if *id == top));
            if !in_afe && self.dom.tag_name(top).ok() == Some(name) {
                self.stack.pop();
                return;
            }
        }
        // 3. Boucle externe bornée à 8 itérations (spec).
        for _ in 0..8 {
            // 4. Dernier élément de format du sujet dans l'AFE.
            let Some((afe_idx, fe)) = self.afe_find(name) else {
                // Aucun → règle générique « any other end tag ».
                self.generic_end_tag(name);
                return;
            };
            // 5. Hors de la pile → retrait AFE, token ignoré.
            let Some(stack_idx) = self.stack.iter().rposition(|&id| id == fe) else {
                self.afe_remove(fe);
                return;
            };
            // 6. Pas en scope → token ignoré.
            if !self.id_in_scope(fe) {
                return;
            }
            // 7. « Furthest block » : le « special » le plus proche
            // sous `fe` dans la pile ; absent → dépile jusqu'à `fe`.
            let Some(rel) = self.stack[stack_idx + 1..]
                .iter()
                .position(|&id| self.is_special(id))
            else {
                self.stack.truncate(stack_idx + 1);
                self.stack.pop();
                self.afe_remove(fe);
                return;
            };
            let furthest_block = self.stack[stack_idx + 1 + rel];
            // 8. Ancêtre commun = élément sous `fe` dans la pile.
            let common_ancestor = self.stack[stack_idx - 1];
            // 9. Position de `fe` dans l'AFE (bookmark).
            let bookmark = afe_idx;
            // 10-16. Boucle interne : on remonte de `furthest_block`
            // vers `fe`. Au-delà de 3 itérations, un nœud encore
            // présent dans l'AFE en est **retiré** (puis sorti de
            // la pile sans clonage — spec §13.2.6.4.7 étape 16.4).
            let mut node_stack_idx = stack_idx + 1 + rel;
            let mut last_node = furthest_block;
            let mut inner = 0usize;
            loop {
                inner += 1;
                // `node` = élément « au-dessus » dans la pile (vers
                // `fe`) ; la position tient compte des retraits.
                if node_stack_idx == 0 {
                    break;
                }
                node_stack_idx -= 1;
                let node = self.stack[node_stack_idx];
                // 16.3. `node` == fe → fin de boucle interne.
                if node == fe {
                    break;
                }
                let in_afe = self.afe_contains(node);
                // 16.4. Au-delà de 3 passages : un nœud de l'AFE
                // en sort (puis quitte la pile sans clone).
                if inner > 3 && in_afe {
                    self.afe_remove(node);
                }
                // 16.5. Absent de l'AFE → sortie de pile, reprise.
                if !self.afe_contains(node) {
                    self.stack.remove(node_stack_idx);
                    continue;
                }
                // 16.6-8. Remplacement par clone dans AFE et pile,
                // puis `last_node` devient enfant du clone.
                let Some(clone) = self.clone_element(node) else {
                    break;
                };
                for e in &mut self.afe {
                    if matches!(e, Afe::Element(id) if *id == node) {
                        *e = Afe::Element(clone);
                    }
                }
                self.stack[node_stack_idx] = clone;
                let _ = self.dom.remove(last_node);
                let _ = self.dom.append_child(clone, last_node);
                last_node = clone;
            }
            // 12. `last_node` à sa place : foster si l'ancêtre est
            // un contexte de table (avant lui), sinon enfant.
            let _ = self.dom.remove(last_node);
            let ca_tag = self.dom.tag_name(common_ancestor).unwrap_or("").to_owned();
            if TABLE_CONTEXT.contains(&ca_tag.as_str()) {
                if let Ok(Some(parent)) = self.dom.parent(common_ancestor) {
                    let _ = self
                        .dom
                        .insert_before(parent, last_node, Some(common_ancestor));
                } else {
                    let _ = self.dom.append_child(common_ancestor, last_node);
                }
            } else {
                let _ = self
                    .dom
                    .append_child(self.insertion_parent(common_ancestor), last_node);
            }
            // 13. Clone de `fe` : récupère les enfants du bloc lointain.
            let Some(fe_clone) = self.clone_element(fe) else {
                return;
            };
            for child in self.dom.children(furthest_block).unwrap_or_default() {
                let _ = self.dom.remove(child);
                let _ = self.dom.append_child(fe_clone, child);
            }
            // 14. Le clone devient enfant du bloc lointain.
            let _ = self.dom.append_child(furthest_block, fe_clone);
            // 15. AFE : `fe` sorti, clone inséré au bookmark.
            self.afe_remove(fe);
            self.afe
                .insert(bookmark.min(self.afe.len()), Afe::Element(fe_clone));
            // 16. Pile : `fe` sorti, clone immédiatement **sous**
            // le bloc lointain (au-dessus dans la pile — la boucle
            // externe le dépilera au passage suivant s'il y a lieu).
            if let Some(i) = self.stack.iter().rposition(|&id| id == fe) {
                self.stack.remove(i);
            }
            if let Some(i) = self.stack.iter().rposition(|&id| id == furthest_block) {
                self.stack.insert(i + 1, fe_clone);
            }
            // Boucle externe : recommence (les clones créés peuvent
            // déclencher un autre passage).
            let _ = tokenizer;
        }
    }

    /// `</template>` complet (spec « in head ») : si un template
    /// est en scope → fins implicites, dépile jusqu'à lui, purge
    /// l'AFE au marqueur, dépile le mode template et réinitialise
    /// le mode d'insertion. Sans template en scope → ignoré.
    fn close_template(&mut self) {
        if !self.stack_has("template") {
            return;
        }
        self.generate_implied_end_tags("");
        while let Some(top) = self.stack.pop() {
            if self.is_html_named(top, "template") {
                break;
            }
        }
        self.afe_clear_to_marker();
        self.template_modes.pop();
        self.reset_mode();
    }

    // -----------------------------------------------------------------
    // « In template » (spec §13.2.6.4.18, R-15)
    // -----------------------------------------------------------------

    fn step_in_template(&mut self, tokenizer: &mut Tokenizer, token: Token) {
        match token {
            Token::Character(_) | Token::Comment(_) | Token::Doctype { .. } => {
                self.step_in_body(tokenizer, token);
            }
            Token::StartTag {
                name,
                attributes,
                self_closing,
            } => {
                let target = match name.as_str() {
                    "base" | "basefont" | "bgsound" | "link" | "meta" | "noframes" | "script"
                    | "style" | "template" | "title" => {
                        self.step_in_head(
                            tokenizer,
                            Token::StartTag {
                                name,
                                attributes,
                                self_closing,
                            },
                        );
                        return;
                    }
                    "caption" | "colgroup" | "tbody" | "tfoot" | "thead" => Mode::InTable,
                    "col" => Mode::InColumnGroup,
                    "tr" => Mode::InTableBody,
                    "td" | "th" => Mode::InRow,
                    _ => Mode::InBody,
                };
                // Spec : le mode template courant saute, le mode
                // cible est empilé puis le token retraité.
                self.template_modes.pop();
                self.template_modes.push(target);
                self.mode = target;
                self.dispatch(
                    tokenizer,
                    Token::StartTag {
                        name,
                        attributes,
                        self_closing,
                    },
                );
            }
            Token::EndTag { name } => {
                if name == "template" {
                    self.step_in_head(tokenizer, Token::EndTag { name });
                }
                // Autres end tags : ignorés (parse error spec).
            }
            Token::Eof => {
                if !self.stack_has("template") {
                    // Fin de parsing — template absent.
                    return;
                }
                // Ferme le template puis retraite l'Eof.
                while let Some(top) = self.stack.pop() {
                    if self.dom.tag_name(top).ok() == Some("template") {
                        break;
                    }
                }
                self.afe_clear_to_marker();
                self.template_modes.pop();
                self.reset_mode();
                self.dispatch(tokenizer, Token::Eof);
            }
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
                    self.clear_stack_to(TABLE_CONTEXT_CLEAR);
                    self.afe_marker();
                    self.insert_element("caption", &attributes);
                    self.mode = Mode::InCaption;
                }
                "colgroup" => {
                    self.clear_stack_to(TABLE_CONTEXT_CLEAR);
                    self.insert_element("colgroup", &attributes);
                    self.mode = Mode::InColumnGroup;
                }
                "col" => {
                    self.clear_stack_to(TABLE_CONTEXT_CLEAR);
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
                    self.clear_stack_to(TABLE_CONTEXT_CLEAR);
                    self.insert_element(&name, &attributes);
                    self.mode = Mode::InTableBody;
                }
                "td" | "th" | "tr" => {
                    self.clear_stack_to(TABLE_CONTEXT_CLEAR);
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
                // `</colgroup>` virtuel : ignoré si le sommet n'est
                // pas colgroup → le texte non-ws l'est aussi.
                if !self.current_is("colgroup") {
                    return;
                }
                self.stack.pop();
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
                    // `</colgroup>` virtuel ; ignoré sans colgroup
                    // courant → le token l'est aussi.
                    if !self.current_is("colgroup") {
                        return;
                    }
                    self.stack.pop();
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
                        self.mode = Mode::InTable;
                    }
                }
                "col" => {}
                "template" => self.step_in_head(tokenizer, Token::EndTag { name }),
                _ => {
                    if !self.current_is("colgroup") {
                        return;
                    }
                    self.stack.pop();
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
                self.clear_stack_to(&["tbody", "tfoot", "thead", "html", "table", "template"]);
                self.insert_element("tr", &attributes);
                self.mode = Mode::InRow;
                let _ = self_closing;
            }
            Token::StartTag { name, .. } if name == "td" || name == "th" => {
                // `td` direct dans tbody → tr synthétisé.
                self.clear_stack_to(&["tbody", "tfoot", "thead", "html", "table", "template"]);
                self.insert_element("tr", &[]);
                self.mode = Mode::InRow;
                self.dispatch(tokenizer, token);
            }
            Token::EndTag { name } if matches!(name.as_str(), "tbody" | "tfoot" | "thead") => {
                if self.in_scope(&name, SCOPE_TABLE) {
                    self.clear_stack_to(&["tbody", "tfoot", "thead", "html", "table", "template"]);
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
    /// puis retraite en `InTable`. Spec : sans section ouverte en
    /// scope de table, le token est **ignoré** (pas de repli).
    fn table_body_to_table(&mut self, tokenizer: &mut Tokenizer, token: Token) {
        for sec in ["tbody", "tfoot", "thead"] {
            if self.in_scope(sec, SCOPE_TABLE) {
                self.clear_stack_to(&["tbody", "tfoot", "thead", "html", "table", "template"]);
                self.stack.pop();
                self.mode = Mode::InTable;
                self.dispatch(tokenizer, token);
                return;
            }
        }
    }

    fn step_in_row(&mut self, tokenizer: &mut Tokenizer, token: Token) {
        match token.clone() {
            Token::StartTag {
                name, attributes, ..
            } if name == "td" || name == "th" => {
                self.clear_stack_to(&["tr", "html", "table", "template"]);
                self.insert_element(&name, &attributes);
                self.afe_marker();
                self.mode = Mode::InCell;
            }
            Token::EndTag { name } if name == "tr" => {
                if self.in_scope("tr", SCOPE_TABLE) {
                    self.clear_stack_to(&["tr", "html", "table", "template"]);
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
            Token::EndTag { name } if name == "table" => {
                // `</table>` : ferme `tr` (si en scope) + retraite
                // en `InTableBody`.
                self.row_to_table_body(tokenizer, token);
            }
            Token::EndTag { name } if matches!(name.as_str(), "tbody" | "tfoot" | "thead") => {
                // Spec : seulement si l'élément de même nom est en
                // scope — sinon ignoré (`<tr>` reste ouvert).
                if self.in_scope(&name, SCOPE_TABLE) {
                    self.row_to_table_body(tokenizer, token);
                }
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
            self.clear_stack_to(&["tr", "html", "table", "template"]);
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
                // Spec : la cellule ne se ferme que si l'élément
                // **de même nom** est en scope de table — sinon
                // le end tag est ignoré (la cellule reste ouverte).
                if self.in_scope(&name, SCOPE_TABLE) {
                    self.close_cell_reprocess(tokenizer, token);
                }
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

            Token::Character(s) => {
                // Spec : `\0` ignoré dans un select. Le texte
                // reconstruit l'AFE (un formatage orphelin
                // enveloppe le texte — corpus webkit02:45).
                let s: String = s.chars().filter(|c| *c != '\0').collect();
                if !s.is_empty() {
                    self.reconstruct_afe();
                    self.insert_text(&s);
                }
            }
            Token::Comment(c) => self.insert_comment(&c),
            Token::StartTag {
                name,
                attributes,
                self_closing,
            } => self.in_select_start_tag(tokenizer, name, attributes, self_closing),
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
                        self.mirror_selectedcontent_open();
                        self.close_element_in_scope("select", SCOPE_TABLE);
                        self.reset_mode();
                    }
                }
                "template" => self.step_in_head(tokenizer, Token::EndTag { name }),
                // Spec « in select » permissive : tout autre
                // end tag relève des règles « in body » —
                // `</i>`/`</b>` y passent l'AAA (purge AFE), les
                // `</div>` la règle générique — corpus webkit02.
                _ => self.step_in_body(tokenizer, Token::EndTag { name }),
            },
            Token::Eof => self.step_in_body(tokenizer, Token::Eof),
        }
    }

    /// Bras « start tag » de [`TreeBuilder::step_in_select`].
    fn in_select_start_tag(
        &mut self,
        tokenizer: &mut Tokenizer,
        name: String,
        attributes: Vec<TokenAttribute>,
        self_closing: bool,
    ) {
        match name.as_str() {
            "option" => {
                if self.current_is("option") {
                    self.stack.pop();
                }
                // Spec « in select » : reconstruction de l'AFE
                // (un `<i>` refermé par `</div>` enveloppe
                // l'option suivante — corpus webkit02:35).
                self.reconstruct_afe();
                self.insert_element("option", &attributes);
            }
            "optgroup" => {
                if self.current_is("option") {
                    self.stack.pop();
                }
                if self.current_is("optgroup") {
                    self.stack.pop();
                }
                self.reconstruct_afe();
                self.insert_element("optgroup", &attributes);
            }
            "select" => {
                // `<select>` dans `<select>` = `</select>`.
                if self.in_scope("select", SCOPE_TABLE) {
                    self.mirror_selectedcontent_open();
                    self.close_element_in_scope("select", SCOPE_TABLE);
                    self.reset_mode();
                }
            }
            "hr" => {
                // Spec : `hr` permis dans `select` — referme un
                // éventuel option/optgroup courant puis s'insère
                // et se dépile aussitôt.
                if self.current_is("option") {
                    self.stack.pop();
                }
                if self.current_is("optgroup") {
                    self.stack.pop();
                }
                self.insert_element("hr", &attributes);
                self.stack.pop();
            }
            "input" | "textarea" => {
                if self.in_scope("select", SCOPE_TABLE) {
                    self.mirror_selectedcontent_open();
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
            // Corpus : `svg`/`math` s'insèrent dans le select
            // (contenu étranger autorisé sous select).
            "svg" => {
                self.insert_foreign_element("svg", &attributes, Namespace::Svg, self_closing);
            }
            "math" => {
                self.insert_foreign_element("math", &attributes, Namespace::MathMl, self_closing);
            }
            // Balises de table hors contexte table : ignorées
            // (le mode « in select in table » les traite à part).
            "caption" | "table" | "tbody" | "tfoot" | "thead" | "tr" | "td" | "th" => {}
            // Spec « in select » moderne : tout autre élément
            // est admis (select personnalisable — `html` compris) —
            // délégué aux règles « in body » pour que le formatage
            // (`b`…) suive la voie AFE/reconstruction.
            _ => self.step_in_body(
                tokenizer,
                Token::StartTag {
                    name,
                    attributes,
                    self_closing,
                },
            ),
        }
    }

    /// Mode « in select in table » (spec §13.2.6.4.17) : les tokens
    /// de table ferment le select et repartent en `InTable`.
    fn step_in_select_in_table(&mut self, tokenizer: &mut Tokenizer, token: Token) {
        let table_name = |n: &str| {
            matches!(
                n,
                "caption" | "table" | "tbody" | "tfoot" | "thead" | "tr" | "td" | "th"
            )
        };
        let should_close = match &token {
            Token::StartTag { name, .. } => table_name(name),
            // Spec : end tag — seulement si l'élément de même nom
            // est en scope de table, sinon ignoré.
            Token::EndTag { name } => table_name(name) && self.in_scope(name, SCOPE_TABLE),
            _ => false,
        };
        if should_close {
            // `</select>` virtuel : dépile jusqu'au select, puis
            // le mode suit la pile réelle (un tbody ouvert reprend
            // ses tokens en « in table body »).
            self.mirror_selectedcontent_open();
            while let Some(top) = self.stack.pop() {
                if self.dom.tag_name(top).ok() == Some("select") {
                    break;
                }
            }
            self.reset_mode();
            self.dispatch(tokenizer, token);
        } else {
            self.step_in_select(tokenizer, token);
        }
    }

    // -----------------------------------------------------------------
    // Frameset (spec §13.2.6.4.19-20, R-15)
    // -----------------------------------------------------------------

    fn step_in_frameset(&mut self, tokenizer: &mut Tokenizer, token: Token) {
        match token {
            Token::Character(s) => {
                // Seul le whitespace survit dans un frameset.
                let ws: String = s
                    .chars()
                    .filter(|c| matches!(c, '\t' | '\n' | '\x0C' | '\r' | ' '))
                    .collect();
                self.insert_text(&ws);
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
                "frameset" => {
                    self.insert_element("frameset", &attributes);
                }
                "frame" => {
                    self.insert_element("frame", &attributes);
                    self.stack.pop();
                }
                "noframes" => self.step_in_head(
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
                "frameset" => {
                    if self.current_is("frameset") {
                        self.stack.pop();
                        if !self.current_is("frameset") {
                            self.mode = Mode::AfterFrameset;
                        }
                    }
                }
                "html" => {
                    // `</html>` en frameset = `</frameset>`.
                    self.step_in_frameset(
                        tokenizer,
                        Token::EndTag {
                            name: "frameset".into(),
                        },
                    );
                }
                _ => {}
            },
            Token::Doctype { .. } | Token::Eof => {}
        }
    }

    fn step_after_frameset(&mut self, tokenizer: &mut Tokenizer, token: Token) {
        match token {
            // Spec : seul le whitespace survit après `</frameset>`
            // (les caractères non-ws sont des erreurs, ignorés).
            Token::Character(s) => {
                let ws: String = s
                    .chars()
                    .filter(|c| matches!(c, '\t' | '\n' | '\x0C' | '\r' | ' '))
                    .collect();
                self.insert_text(&ws);
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
                "noframes" => self.step_in_head(
                    tokenizer,
                    Token::StartTag {
                        name,
                        attributes,
                        self_closing,
                    },
                ),
                _ => {}
            },
            Token::EndTag { name } => {
                if name == "html" {
                    self.mode = Mode::AfterAfterBody;
                }
            }
            Token::Doctype { .. } | Token::Eof => {}
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
            Token::Character(s) => {
                let (ws, rest) = split_ws(&s);
                if !ws.is_empty() {
                    // ws après `</html>` : insertion « in body »
                    // (le nœud courant, souvent `<html>`).
                    let saved = self.mode;
                    self.mode = Mode::InBody;
                    self.insert_text(ws);
                    self.mode = saved;
                }
                if rest.is_empty() {
                    return;
                }
                // Spec : contenu tardif → « in body ». Le corpus
                // ne l'admet que si un `body` est encore ouvert —
                // sans body (document frameset), tout est ignoré.
                if self.stack_has("body") {
                    self.mode = Mode::InBody;
                    self.dispatch(tokenizer, Token::Character(rest.to_owned()));
                }
            }
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
                _ => {
                    if self.stack_has("body") {
                        self.mode = Mode::InBody;
                        self.dispatch(
                            tokenizer,
                            Token::StartTag {
                                name,
                                attributes,
                                self_closing,
                            },
                        );
                    } else if name == "noframes" {
                        // `noframes` survit en document frameset.
                        self.step_in_head(
                            tokenizer,
                            Token::StartTag {
                                name,
                                attributes,
                                self_closing,
                            },
                        );
                    }
                }
            },
            Token::Eof | Token::EndTag { .. } | Token::Doctype { .. } => {}
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

/// Traduit le mode interne du builder en [`DocumentMode`]
/// porté par le DOM (le crate dom ne connaît pas
/// `QuirksMode` — sens de dépendance).
fn document_mode_of(mode: QuirksMode) -> DocumentMode {
    match mode {
        QuirksMode::NoQuirks => DocumentMode::NoQuirks,
        QuirksMode::LimitedQuirks => DocumentMode::LimitedQuirks,
        QuirksMode::Quirks => DocumentMode::Quirks,
    }
}

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

/// Égalité ensembliste des attributs (ordre indifférent) — règle
/// de Noé de l'AFE : `<font size=4>` n'égale pas
/// `<font color=red>` (spec « equal attributes »).
fn attrs_equal(a: &[explorer_dom::Attribute], b: &[explorer_dom::Attribute]) -> bool {
    a.len() == b.len()
        && a.iter()
            .all(|x| b.iter().any(|y| x.name == y.name && x.value == y.value))
}
