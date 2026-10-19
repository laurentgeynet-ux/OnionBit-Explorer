// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Cascade CSS : origines, spécificité, ordre source,
//! `!important`, mots-clés `inherit`/`initial`/`unset`,
//! héritage et *computed style* par nœud.
//!
//! Ordre de la cascade implémenté (bas → haut) :
//! **UA normal < auteur normal < auteur `!important` < UA
//! `!important`** — deux origines seulement pour l'instant
//! (pas de feuille utilisateur), à rang égal la spécificité
//! puis l'ordre source départagent.
//!
//! Les valeurs restent du texte résolu de déclaration — le
//! typage par propriété est le travail de l'étape 16 (module
//! propriétés). La feuille UA intégrée (`UA_CSS`) est écrite
//! en CSS et parsée par notre propre parser.

use crate::parser::{AtKind, Declaration, MediaQuery, Rule, Stylesheet};
use crate::selector::{matches_with_state, parse_selector_list, ComplexSelector, Specificity};
use crate::value::{expand_shorthand, parse_color, parse_length, Color, Length};
use explorer_dom::{Dom, NodeId};
use std::collections::HashMap;

/// Environnement d'évaluation des media queries.
#[derive(Debug, Clone, PartialEq)]
pub struct StyleEnv {
    /// Type de média (`screen`, `print`…).
    pub media_type: String,
    /// Largeur du viewport en px.
    pub width_px: u32,
    /// Hauteur du viewport en px.
    pub height_px: u32,
}

impl Default for StyleEnv {
    fn default() -> Self {
        Self {
            media_type: "screen".into(),
            width_px: 1024,
            height_px: 768,
        }
    }
}

/// `true` si la liste de queries matche l'environnement —
/// liste vide (ou `all` implicite) = toujours vrai.
#[must_use]
pub fn media_matches(env: &StyleEnv, queries: &[MediaQuery]) -> bool {
    if queries.is_empty() {
        return true;
    }
    queries.iter().any(|q| query_matches(env, q))
}

fn query_matches(env: &StyleEnv, q: &MediaQuery) -> bool {
    // Forme non reconnue → vrai (tolérance documentée au parser).
    let mut ok = q.unknown;
    if !ok {
        ok = match &q.media_type {
            None => true,
            Some(t) => t == "all" || t == &env.media_type,
        };
        if ok {
            for (name, value) in &q.conditions {
                if !condition_matches(env, name, value) {
                    ok = false;
                    break;
                }
            }
        }
    }
    if q.negated {
        !ok
    } else {
        ok
    }
}

/// `(name: value)` d'une media query — px pour *-width/-height.
fn condition_matches(env: &StyleEnv, name: &str, value: &str) -> bool {
    let px = || -> Option<f64> {
        let v = value.strip_suffix("px").unwrap_or(value);
        v.trim().parse().ok()
    };
    match name {
        "min-width" => px().is_some_and(|v| f64::from(env.width_px) >= v),
        "max-width" => px().is_some_and(|v| f64::from(env.width_px) <= v),
        // Tolérance sous-pixel : un viewport entier ne devrait
        // jamais dépendre d'un bit de mantisse.
        "width" => px().is_some_and(|v| (f64::from(env.width_px) - v).abs() < 0.5),
        "min-height" => px().is_some_and(|v| f64::from(env.height_px) >= v),
        "max-height" => px().is_some_and(|v| f64::from(env.height_px) <= v),
        "height" => px().is_some_and(|v| (f64::from(env.height_px) - v).abs() < 0.5),
        "orientation" => match value {
            "landscape" => env.width_px >= env.height_px,
            "portrait" => env.width_px < env.height_px,
            _ => false,
        },
        // Feature inconnue → query fausse (spec : `not all`).
        _ => false,
    }
}

/// Origine d'une règle dans la cascade.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// Feuille agent-utilisateur (défauts navigateur).
    UserAgent,
    /// Feuille auteur (page, `<style>`, liens externes).
    Author,
}

/// Une règle de style collectée (sélecteurs pré-parsés).
struct CollectedRule {
    selectors: Vec<ComplexSelector>,
    declarations: Vec<Declaration>,
    /// Ordre source global — départage à spécificité égale.
    order: u32,
    origin: Origin,
    /// Queries `@media` englobantes — groupe de listes où
    /// CHAQUE groupe doit matcher (imbrication = AND, `,` =
    /// OR interne). Vide = inconditionnel. Évaluées à l'usage
    /// (le viewport peut changer entre deux calculs).
    media: Vec<Vec<MediaQuery>>,
}

/// Moteur de cascade : accumulateur de feuilles + calcul.
#[derive(Default)]
pub struct StyleEngine {
    rules: Vec<CollectedRule>,
    next_order: u32,
}

/// Computed style d'un nœud : propriété (minuscules) → valeur
/// texte résolue (mot-clés `inherit`/`initial`/`unset` déjà
/// réduits ; le *typage* est l'affaire de l'étape 16).
#[derive(Debug, Clone, Default)]
pub struct ComputedStyle {
    props: HashMap<String, String>,
}

impl ComputedStyle {
    /// Valeur calculée d'une propriété, `None` si absente
    /// (→ valeur initiale ou héritée selon la propriété).
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&str> {
        self.props.get(name).map(String::as_str)
    }

    /// Itère les propriétés présentes (ordre indéfini).
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.props.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }

    /// Valeur calculée ou repli explicite.
    #[must_use]
    pub fn get_or<'a>(&'a self, name: &str, fallback: &'a str) -> &'a str {
        self.get(name).unwrap_or(fallback)
    }

    /// Valeur typée `Length` (`10px`, `1.5em`, `0`) — `None`
    /// si absente ou non-longueur (`auto`, `medium`…).
    #[must_use]
    pub fn length(&self, name: &str) -> Option<Length> {
        self.get(name).and_then(parse_length)
    }

    /// Valeur typée `Color` — `currentcolor` est résolu via la
    /// couleur de l'élément (`color` elle-même).
    #[must_use]
    pub fn color(&self, name: &str) -> Option<Color> {
        let raw = self.get(name)?;
        if raw.eq_ignore_ascii_case("currentcolor") {
            return self
                .get("color")
                .and_then(parse_color)
                .or(Some(Color::BLACK));
        }
        parse_color(raw)
    }

    /// Valeur typée `f64` (`z-index`, `opacity`…).
    #[must_use]
    pub fn number(&self, name: &str) -> Option<f64> {
        self.get(name)?.trim().parse().ok()
    }
}

/// Table des styles calculés d'un sous-arbre (mémoïsation
/// parent → enfant de `compute_tree`).
#[derive(Debug, Clone, Default)]
pub struct StyleMap {
    map: HashMap<NodeId, ComputedStyle>,
}

impl StyleMap {
    /// Style calculé du nœud, `None` s'il n'est pas dans
    /// le sous-arbre calculé (ou handle obsolète).
    #[must_use]
    pub fn get(&self, node: NodeId) -> Option<&ComputedStyle> {
        self.map.get(&node)
    }
}

impl StyleEngine {
    /// Moteur préchargé avec la feuille UA intégrée.
    #[must_use]
    pub fn new() -> Self {
        let mut e = Self::default();
        e.add_css(UA_CSS, Origin::UserAgent);
        e
    }

    /// Ajoute une feuille parsée (auteur ou UA).
    pub fn add_stylesheet(&mut self, sheet: &Stylesheet, origin: Origin) {
        self.collect_rules(&sheet.rules, origin, &[]);
    }

    /// Ajoute une feuille depuis son texte CSS.
    pub fn add_css(&mut self, input: &str, origin: Origin) {
        let sheet = crate::parser::parse_stylesheet(input);
        self.add_stylesheet(&sheet, origin);
    }

    /// Aplati récursif : règles qualifiées + corps `@media`.
    /// `media` = pile des listes englobantes (AND entre listes,
    /// OR entre les queries d'une même liste).
    fn collect_rules(&mut self, rules: &[Rule], origin: Origin, media: &[Vec<MediaQuery>]) {
        for rule in rules {
            match rule {
                Rule::Qualified(q) => {
                    if let Some(selectors) = parse_selector_list(&q.prelude) {
                        self.rules.push(CollectedRule {
                            selectors,
                            declarations: q.declarations.clone(),
                            order: self.next_order,
                            origin,
                            media: media.to_vec(),
                        });
                        self.next_order = self.next_order.saturating_add(1);
                    }
                }
                Rule::At(at) => {
                    if let AtKind::Media { queries, rules } = &at.kind {
                        // `@media` imbriqué = AND : on empile la
                        // liste de queries du niveau.
                        let mut nested = media.to_vec();
                        nested.push(queries.clone());
                        self.collect_rules(rules, origin, &nested);
                    }
                }
            }
        }
    }

    /// Computed style d'un nœud (récursion parent sans mémoire —
    /// pour un seul nœud. Préférer `compute_tree` en masse).
    /// Pseudo-classes dynamiques : fausses.
    #[must_use]
    pub fn compute(&self, dom: &Dom, node: NodeId, env: &StyleEnv) -> ComputedStyle {
        self.compute_with_state(dom, node, env, &|_| false)
    }

    /// Variante étatée : `state` arbitre les pseudo-classes
    /// dynamiques (`"hover"`…) — l'appelant relance le calcul
    /// quand l'état change (restyle ADR-0012).
    #[must_use]
    pub fn compute_with_state(
        &self,
        dom: &Dom,
        node: NodeId,
        env: &StyleEnv,
        state: &dyn Fn(&str) -> bool,
    ) -> ComputedStyle {
        let parent = parent_computed(self, dom, node, env, state, &mut HashMap::new());
        self.compute_with_parent(dom, node, env, parent.as_ref(), state)
    }

    /// Styles calculés de tout le sous-arbre de `root`
    /// (chaque nœud hérite du parent déjà calculé — O(n)).
    #[must_use]
    pub fn compute_tree(&self, dom: &Dom, root: NodeId, env: &StyleEnv) -> StyleMap {
        self.compute_tree_with_state(dom, root, env, &|_| false)
    }

    /// Variante étatée de [`StyleEngine::compute_tree`].
    #[must_use]
    pub fn compute_tree_with_state(
        &self,
        dom: &Dom,
        root: NodeId,
        env: &StyleEnv,
        state: &dyn Fn(&str) -> bool,
    ) -> StyleMap {
        let mut map = HashMap::new();
        self.compute_subtree(dom, root, env, None, &mut map, state);
        StyleMap { map }
    }

    fn compute_subtree(
        &self,
        dom: &Dom,
        node: NodeId,
        env: &StyleEnv,
        parent: Option<&ComputedStyle>,
        map: &mut HashMap<NodeId, ComputedStyle>,
        state: &dyn Fn(&str) -> bool,
    ) {
        let style = self.compute_with_parent(dom, node, env, parent, state);
        let Ok(children) = dom.children(node) else {
            map.insert(node, style);
            return;
        };
        map.insert(node, style);
        for child in children {
            if dom.element(child).is_ok() {
                let parent_style = map.get(&node).cloned();
                self.compute_subtree(dom, child, env, parent_style.as_ref(), map, state);
            }
        }
    }

    /// Calcul d'un nœud connaissant le style de son parent.
    fn compute_with_parent(
        &self,
        dom: &Dom,
        node: NodeId,
        env: &StyleEnv,
        parent: Option<&ComputedStyle>,
        state: &dyn Fn(&str) -> bool,
    ) -> ComputedStyle {
        let mut props: HashMap<String, String> = HashMap::new();
        if dom.element(node).is_err() {
            return ComputedStyle { props };
        }
        // 1. Collecte (rang, spécificité, ordre, déclaration).
        let mut cand: Vec<(u8, Specificity, u32, &Declaration)> = Vec::new();
        for rule in &self.rules {
            // Chaque groupe `@media` englobant doit matcher.
            if !rule.media.is_empty() && !rule.media.iter().all(|qs| media_matches(env, qs)) {
                continue;
            }
            // Spécificité = max des sélecteurs qui matchent.
            let spec = rule
                .selectors
                .iter()
                .filter(|s| matches_with_state(dom, node, s, state))
                .map(ComplexSelector::specificity)
                .max();
            let Some(spec) = spec else { continue };
            for d in &rule.declarations {
                let rank = match (rule.origin, d.important) {
                    (Origin::UserAgent, false) => 0,
                    (Origin::Author, false) => 1,
                    (Origin::Author, true) => 2,
                    (Origin::UserAgent, true) => 3,
                };
                cand.push((rank, spec, rule.order, d));
            }
        }
        // 2. Tri stable croissant → la dernière écriture gagne.
        cand.sort_by_key(|(rank, spec, order, _)| (*rank, *spec, *order));
        for (_, _, _, d) in cand {
            let lname = d.name.to_ascii_lowercase();
            // Shorthand → longhands (spec : `margin: x` fixe les
            // quatre côtés, chacun à la spécificité de la règle).
            match expand_shorthand(&lname, &d.value) {
                Some(expanded) => {
                    for (long_name, v) in expanded {
                        props.insert(long_name, v);
                    }
                }
                None => {
                    props.insert(lname, d.value.clone());
                }
            }
        }
        // 3. Héritage : propriétés héritées non déclarées.
        if let Some(p) = parent {
            for name in INHERITED_PROPS {
                if !props.contains_key(*name) {
                    if let Some(v) = p.get(name) {
                        props.insert((*name).to_string(), v.to_string());
                    }
                }
            }
            // Propriétés custom `--*` : héritées par défaut (spec).
            for (k, v) in p.iter() {
                if k.starts_with("--") && !props.contains_key(k) {
                    props.insert(k.to_string(), v.to_string());
                }
            }
        }
        // 4. Mots-clés CSS-wide sur les valeurs déclarées.
        for (k, v) in &mut props {
            let kw = v.trim().to_ascii_lowercase();
            let initial = || initial_value(k).unwrap_or("").to_string();
            *v = match kw.as_str() {
                // `inherit` : valeur du parent (initiale à la racine).
                "inherit" => parent
                    .and_then(|p| p.get(k))
                    .map_or_else(initial, str::to_string),
                // `unset`/`revert` : hérite si la propriété hérite,
                // sinon valeur initiale (revert simplifié — pas
                // de couche utilisateur).
                "unset" | "revert" | "revert-layer" if is_inherited(k) => parent
                    .and_then(|p| p.get(k))
                    .map_or_else(initial, str::to_string),
                "unset" | "revert" | "revert-layer" | "initial" => initial(),
                _ => continue,
            };
        }
        ComputedStyle { props }
    }
}

/// Style calculé du parent élément (mémoïsation locale).
fn parent_computed(
    engine: &StyleEngine,
    dom: &Dom,
    node: NodeId,
    env: &StyleEnv,
    state: &dyn Fn(&str) -> bool,
    memo: &mut HashMap<NodeId, ComputedStyle>,
) -> Option<ComputedStyle> {
    let parent = match dom.parent(node) {
        Ok(Some(p)) if dom.element(p).is_ok() => p,
        _ => return None,
    };
    if let Some(s) = memo.get(&parent) {
        return Some(s.clone());
    }
    let grand = parent_computed(engine, dom, parent, env, state, memo);
    let s = engine.compute_with_parent(dom, parent, env, grand.as_ref(), state);
    memo.insert(parent, s.clone());
    Some(s)
}

/// Propriétés héritées (spec CSS cascade / inherit list).
const INHERITED_PROPS: &[&str] = &[
    "border-collapse",
    "border-spacing",
    "caption-side",
    "color",
    "cursor",
    "direction",
    "empty-cells",
    "font-family",
    "font-size",
    "font-style",
    "font-variant",
    "font-weight",
    "letter-spacing",
    "line-height",
    "list-style",
    "list-style-image",
    "list-style-position",
    "list-style-type",
    "orphans",
    "quotes",
    "text-align",
    "text-indent",
    "text-transform",
    "visibility",
    "white-space",
    "widows",
    "word-spacing",
];

/// `name` est-elle héritée (propriétés custom incluses) ?
#[must_use]
pub fn is_inherited(name: &str) -> bool {
    name.starts_with("--") || INHERITED_PROPS.contains(&name)
}

/// Valeur initiale des propriétés couvertes (table réduite —
/// étendue par l'étape 16). `None` = pas de valeur connue ici.
#[must_use]
pub fn initial_value(name: &str) -> Option<&'static str> {
    Some(match name {
        "display" => "inline",
        "position" => "static",
        "z-index" | "top" | "right" | "bottom" | "left" | "width" | "height" | "min-width"
        | "min-height" | "max-width" | "max-height" | "cursor" => "auto",
        "margin" | "margin-top" | "margin-right" | "margin-bottom" | "margin-left" | "padding"
        | "padding-top" | "padding-right" | "padding-bottom" | "padding-left" | "text-indent"
        | "border-spacing" => "0",
        // `medium` = initiale commune des largeurs de bordure
        // et de `font-size` (spec).
        "border"
        | "border-width"
        | "border-top-width"
        | "border-right-width"
        | "border-bottom-width"
        | "border-left-width"
        | "font-size" => "medium",
        "border-style"
        | "border-top-style"
        | "border-right-style"
        | "border-bottom-style"
        | "border-left-style"
        | "text-transform"
        | "text-decoration"
        | "list-style-image"
        | "float"
        | "clear"
        | "background-image" => "none",
        "border-color"
        | "border-top-color"
        | "border-right-color"
        | "border-bottom-color"
        | "border-left-color" => "currentcolor",
        "color" => "black",
        "background" | "background-color" => "transparent",
        "font-family" => "serif",
        "font-style" | "font-variant" | "font-weight" | "line-height" | "letter-spacing"
        | "word-spacing" | "white-space" => "normal",
        "text-align" => "start",
        "visibility" | "overflow" | "overflow-x" | "overflow-y" => "visible",
        "vertical-align" => "baseline",
        "list-style" | "list-style-type" => "disc",
        "list-style-position" => "outside",
        "border-collapse" => "separate",
        "direction" => "ltr",
        _ => return None,
    })
}

/// Feuille agent-utilisateur intégrée — écrite en CSS et
/// parsée par notre propre pipeline (dogfooding). Reprend
/// les défauts classiques de la spec HTML Rendering.
const UA_CSS: &str = "
html, body, div, p, ul, ol, li, dl, dt, dd, blockquote, pre,
form, fieldset, figure, figcaption, h1, h2, h3, h4, h5, h6,
hr, address, article, aside, footer, header, nav, section,
main, legend, video { display: block }
table { display: table }
thead, tbody, tfoot { display: table-row-group }
tr { display: table-row }
caption { display: block }
head, script, style, link, meta, title, template, noscript, area,
base, basefont, datalist, param, source, track { display: none }
li { display: list-item }
td, th { display: table-cell }
b, strong { font-weight: bold }
i, em, cite, dfn, var, address { font-style: italic }
u, ins { text-decoration: underline }
s, del, strike { text-decoration: line-through }
code, kbd, pre, samp, tt { font-family: monospace }
pre, textarea { white-space: pre-wrap }
h1 { font-size: 2em; font-weight: bold; margin-top: 0.67em; margin-bottom: 0.67em }
h2 { font-size: 1.5em; font-weight: bold; margin-top: 0.83em; margin-bottom: 0.83em }
h3 { font-size: 1.17em; font-weight: bold; margin-top: 1em; margin-bottom: 1em }
h4 { font-size: 1em; font-weight: bold; margin-top: 1.33em; margin-bottom: 1.33em }
h5 { font-size: 0.83em; font-weight: bold; margin-top: 1.67em; margin-bottom: 1.67em }
h6 { font-size: 0.67em; font-weight: bold; margin-top: 2.33em; margin-bottom: 2.33em }
p { margin-top: 1em; margin-bottom: 1em }
ul, ol, dd { margin-top: 1em; margin-bottom: 1em }
ul, ol { padding-left: 40px }
dd { margin-left: 40px }
blockquote { margin: 1em 40px }
hr { border-top-width: 1px; border-style: inset }
a[href] { color: #00e; text-decoration: underline }
table { border-collapse: separate; border-spacing: 2px }
th { font-weight: bold; text-align: center }
";
