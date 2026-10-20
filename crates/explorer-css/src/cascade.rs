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
use crate::value::{
    expand_shorthand, parse_color, parse_length, value_is_valid_for, Color, Length,
};
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
    /// Taille de fonte de base (px) — base des `rem`, de
    /// `medium` et de la valeur initiale de `font-size` à
    /// la racine. Convention navigateur : 16.
    pub font_size_base: f32,
}

impl Default for StyleEnv {
    fn default() -> Self {
        Self {
            media_type: "screen".into(),
            width_px: 1024,
            height_px: 768,
            font_size_base: 16.0,
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

/// Un bloc `@font-face` collecté — conservé hors cascade
/// d'éléments (ces règles ne stylent rien : elles décrivent
/// des fontes à charger, servies par
/// [`StyleEngine::font_faces`]).
pub(crate) struct CollectedFontFace {
    /// Pile des listes `@media` englobantes (AND entre piles,
    /// OR interne à une liste) — vide = inconditionnel.
    pub(crate) media: Vec<Vec<MediaQuery>>,
    pub(crate) declarations: Vec<Declaration>,
}

/// Moteur de cascade : accumulateur de feuilles + calcul.
#[derive(Default)]
pub struct StyleEngine {
    rules: Vec<CollectedRule>,
    next_order: u32,
    /// `@font-face` avec leur contexte media (étape 46).
    pub(crate) font_faces: Vec<CollectedFontFace>,
    /// `@keyframes` par nom — offsets triés, multi-sélecteurs
    /// éclatés (R-10, module `animation`).
    pub(crate) keyframes: crate::animation::Keyframes,
}

/// Computed style d'un nœud : propriété (minuscules) → valeur
/// texte résolue (mot-clés `inherit`/`initial`/`unset` déjà
/// réduits ; le *typage* est l'affaire de l'étape 16).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
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

    /// Pose `name = value` — réservé aux surcharges
    /// post-cascade (transitions, étape 45 : la valeur
    /// interpolée l'emporte sur toute déclaration).
    pub fn set(&mut self, name: &str, value: &str) {
        self.props.insert(name.to_string(), value.to_string());
    }
}

/// Styles d'un pseudo-élément (R-13) — calculés avec l'élément
/// origine pour parent d'héritage.
#[derive(Debug, Clone, Default)]
pub struct PseudoStyles {
    /// `::before` (présent si la cascade a produit un `content`).
    pub before: Option<ComputedStyle>,
    /// `::after`.
    pub after: Option<ComputedStyle>,
}

/// Table des styles calculés d'un sous-arbre (mémoïsation
/// parent → enfant de `compute_tree`).
#[derive(Debug, Clone, Default)]
pub struct StyleMap {
    map: HashMap<NodeId, ComputedStyle>,
    /// `::before`/`::after` par nœud — remplis pendant
    /// `compute_tree` (R-13). Vide pour les noeuds sans
    /// pseudo généré.
    pseudos: HashMap<NodeId, PseudoStyles>,
}

impl StyleMap {
    /// Style calculé du nœud, `None` s'il n'est pas dans
    /// le sous-arbre calculé (ou handle obsolète).
    #[must_use]
    pub fn get(&self, node: NodeId) -> Option<&ComputedStyle> {
        self.map.get(&node)
    }

    /// Style calculé du pseudo-élément `which` de `node` —
    /// `None` si aucune règle ne le produit (`content` absent
    /// ou `none`/`normal` gérés par le consommateur).
    #[must_use]
    pub fn pseudo(&self, node: NodeId, which: crate::PseudoElement) -> Option<&ComputedStyle> {
        let p = self.pseudos.get(&node)?;
        match which {
            crate::PseudoElement::Before => p.before.as_ref(),
            crate::PseudoElement::After => p.after.as_ref(),
        }
    }

    /// Style mutable du nœud — surcharge post-cascade
    /// (transitions, étape 45).
    pub fn get_mut(&mut self, node: NodeId) -> Option<&mut ComputedStyle> {
        self.map.get_mut(&node)
    }

    /// Itère `(nœud, style)` — ordre indéfini.
    pub fn iter(&self) -> impl Iterator<Item = (NodeId, &ComputedStyle)> {
        self.map.iter().map(|(n, c)| (*n, c))
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
                    } else if let AtKind::Declarations { declarations, .. } = &at.kind {
                        if at.name.eq_ignore_ascii_case("font-face") {
                            self.font_faces.push(CollectedFontFace {
                                media: media.to_vec(),
                                declarations: declarations.clone(),
                            });
                        }
                    } else if let AtKind::Keyframes { name, frames } = &at.kind {
                        // `@media` englobant appliqué ? La table
                        // des keyframes ne porte pas de media
                        // (écart documenté — la spec permet
                        // @keyframes sous @media).
                        crate::animation::collect_keyframes(
                            &mut self.keyframes,
                            name.clone(),
                            frames,
                        );
                    }
                }
            }
        }
    }

    /// Frames `@keyframes` du nom donné — triées par offset,
    /// `None` si inconnue (R-10). La session y pioche pour
    /// `animation-name` via [`crate::animation::keyframe_style`].
    #[must_use]
    pub fn keyframes(&self, name: &str) -> Option<&[(f64, Vec<Declaration>)]> {
        self.keyframes.get(name).map(Vec::as_slice)
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
        self.compute_with_parent(dom, node, env, parent.as_ref(), None, state)
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
        let mut pseudos = HashMap::new();
        self.compute_subtree(dom, root, env, None, &mut map, &mut pseudos, state);
        StyleMap { map, pseudos }
    }

    #[allow(clippy::too_many_arguments)]
    fn compute_subtree(
        &self,
        dom: &Dom,
        node: NodeId,
        env: &StyleEnv,
        parent: Option<&ComputedStyle>,
        map: &mut HashMap<NodeId, ComputedStyle>,
        pseudos: &mut HashMap<NodeId, PseudoStyles>,
        state: &dyn Fn(&str) -> bool,
    ) {
        let style = self.compute_with_parent(dom, node, env, parent, None, state);
        // Pseudo-éléments (R-13) : l'élément origine est le
        // parent d'héritage de ses `::before`/`::after`.
        if dom.element(node).is_ok() {
            let before = self.compute_with_parent(
                dom,
                node,
                env,
                Some(&style),
                Some(crate::PseudoElement::Before),
                state,
            );
            let after = self.compute_with_parent(
                dom,
                node,
                env,
                Some(&style),
                Some(crate::PseudoElement::After),
                state,
            );
            let ps = PseudoStyles {
                before: (before.get("content").is_some()).then_some(before),
                after: (after.get("content").is_some()).then_some(after),
            };
            if ps.before.is_some() || ps.after.is_some() {
                pseudos.insert(node, ps);
            }
        }
        let Ok(children) = dom.children(node) else {
            map.insert(node, style);
            return;
        };
        map.insert(node, style);
        for child in children {
            if dom.element(child).is_ok() {
                let parent_style = map.get(&node).cloned();
                // Récursion ∝ profondeur DOM → pile garantie
                // (R-24) : bascule tas en zone rouge.
                explorer_common::deep(|| {
                    self.compute_subtree(
                        dom,
                        child,
                        env,
                        parent_style.as_ref(),
                        map,
                        pseudos,
                        state,
                    );
                });
            }
        }
    }

    /// Calcul d'un nœud connaissant le style de son parent.
    /// `pseudo` = `Some(pe)` → style du pseudo-élément `pe` du
    /// nœud (R-13) ; `None` → style de l'élément lui-même.
    fn compute_with_parent(
        &self,
        dom: &Dom,
        node: NodeId,
        env: &StyleEnv,
        parent: Option<&ComputedStyle>,
        pseudo: Option<crate::PseudoElement>,
        state: &dyn Fn(&str) -> bool,
    ) -> ComputedStyle {
        let mut props: HashMap<String, String> = HashMap::new();
        if dom.element(node).is_err() {
            return ComputedStyle { props };
        }
        // 1-2. Collecte ordonnée (rang, spécificité, ordre) :
        // règles + attribut `style` (rang auteur inline,
        // `!important` au-dessus de tout — CSS2 §6.4.4 ;
        // `style=` ne s'écrit que sur l'élément : rien pour
        // un pseudo).
        // `style=` parsé avant `cand` : ses déclarations y
        // sont empruntées — il doit survivre à la boucle.
        let inline_decls = if pseudo.is_none() {
            dom.attribute(node, "style")
                .ok()
                .flatten()
                .map(|s| crate::parser::parse_declarations(&s))
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let mut cand = self.collect_candidates(dom, node, env, pseudo, state);
        if pseudo.is_none() {
            for d in &inline_decls {
                let rank = if d.important { 4 } else { 1 };
                cand.push((rank, (u32::MAX, u32::MAX, u32::MAX), u32::MAX, d));
            }
            // Le rang inline s'intercale dans l'ordre global.
            cand.sort_by_key(|(rank, spec, order, _)| (*rank, *spec, *order));
        }
        for (_, _, _, d) in &cand {
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
                    // Verbatim : la validation par propriété
                    // fait le tri (`width: 200` sans unité,
                    // raccourci invalide → déclaration ignorée).
                    if value_is_valid_for(&lname, &d.value) {
                        props.insert(lname, d.value.clone());
                    }
                }
            }
        }
        // 2b. Attributs de présentation HTML (§ rendering :
        // « presentational hints » — équivalents à des
        // déclarations auteur de spécificité nulle, donc
        // appliqués seulement si la propriété n'est pas déjà
        // fixée par une vraie règle ou l'attribut `style`).
        if pseudo.is_none() {
            apply_presentational_hints(dom, node, &mut props);
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
        // 5. `var()` : substitution depuis les `--*` résolus
        //    du nœud (hérités étape 3 ou déclarés ici). Une
        //    référence non résoluble rend la déclaration
        //    invalide (spec « invalid at computed-value time »
        //    → on retire la propriété ; pour une héritée, la
        //    valeur du parent est restaurée).
        let snap = props.clone();
        for (k, v) in &mut props {
            if !k.starts_with("--") && v.contains("var(") {
                if let Some(resolved) = substitute_vars(v, &snap, 0) {
                    *v = resolved;
                } else {
                    if is_inherited(k) {
                        if let Some(pv) = parent.and_then(|p| p.get(k)) {
                            *v = pv.to_string();
                            continue;
                        }
                    }
                    // Marqueur de retrait — éliminé juste après.
                    *v = "\u{0}".to_string();
                }
            }
        }
        props.retain(|_, v| v != "\u{0}");
        // 6. `font-size` → valeur calculée absolue (R-17 —
        //    détail dans `normalize_font_size`).
        normalize_font_size(&mut props, parent, env.font_size_base);
        ComputedStyle { props }
    }

    /// Candidats de déclaration des **règles** triés (rang,
    /// spécificité, ordre croissants — la dernière écriture
    /// gagne) : un sélecteur doit matcher **avec le même
    /// pseudo-élément** cible. L'attribut `style` est fusionné
    /// par l'appelant (durée de vie locale).
    fn collect_candidates<'a>(
        &'a self,
        dom: &Dom,
        node: NodeId,
        env: &StyleEnv,
        pseudo: Option<crate::PseudoElement>,
        state: &dyn Fn(&str) -> bool,
    ) -> Vec<(u8, Specificity, u32, &'a Declaration)> {
        let mut rules_cand: Vec<(u8, Specificity, u32, &Declaration)> = Vec::new();
        for rule in &self.rules {
            // Chaque groupe `@media` englobant doit matcher.
            if !rule.media.is_empty() && !rule.media.iter().all(|qs| media_matches(env, qs)) {
                continue;
            }
            // Spécificité = max des sélecteurs qui matchent.
            let spec = rule
                .selectors
                .iter()
                .filter(|s| crate::selector::selector_pseudo_element(s) == pseudo)
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
                rules_cand.push((rank, spec, rule.order, d));
            }
        }
        rules_cand.sort_by_key(|(rank, spec, order, _)| (*rank, *spec, *order));
        rules_cand
    }
}

/// `font-size` : la valeur calculée est une longueur
/// ABSOLUE (spec CSS — l'héritage transporte la valeur
/// calculée). Sans cette normalisation, un `em`/`%` déclaré
/// serait re-multiplié à chaque niveau de boîte en aval —
/// le style cloné des blocs anonymes et des boîtes texte
/// re-résolvait le ratio contre le font-size parent déjà
/// résolu (« texte em²/em³ », R-17). Les valeurs non
/// résolubles ici (`calc()`…) restent brutes : le layout
/// dégrade proprement.
fn normalize_font_size(
    props: &mut HashMap<String, String>,
    parent: Option<&ComputedStyle>,
    base: f32,
) {
    let Some(raw) = props.get("font-size").cloned() else {
        return;
    };
    let parent_fs = parent
        .and_then(|p| p.get("font-size"))
        .and_then(|v| v.trim().trim_end_matches("px").trim().parse::<f32>().ok())
        .unwrap_or(base);
    if let Some(px) = resolve_font_size(&raw, parent_fs, base) {
        props.insert("font-size".to_string(), format!("{px}px"));
    }
}

/// Résout `font-size` en px absolus : `em`/`ex`/`ch`/`%` et
/// `larger`/`smaller` contre la taille **calculée** du parent
/// (`parent_fs`), `rem` et mots-clés absolus contre la base
/// (`base`). `calc()` et syntaxes inconnues → `None` (la
/// valeur brute survit pour le layout — dégradation
/// documentée, jamais de panique).
fn resolve_font_size(raw: &str, parent_fs: f32, base: f32) -> Option<f32> {
    let v = raw.trim();
    // Mots-clés de la spec : échelle absolue ×1.2 autour de
    // `medium` (CSS Fonts §5.2) ; `smaller`/`larger` sont
    // relatifs au parent par convention navigateur (≈ ÷/× 1.2).
    match v {
        "xx-small" => return Some(base / 1.728),
        "x-small" => return Some(base / 1.44),
        "small" => return Some(base / 1.2),
        "medium" => return Some(base),
        "large" => return Some(base * 1.2),
        "x-large" => return Some(base * 1.44),
        "xx-large" => return Some(base * 1.728),
        "xxx-large" => return Some(base * 2.0736),
        "smaller" => return Some(parent_fs / 1.2),
        "larger" => return Some(parent_fs * 1.2),
        _ => {}
    }
    if let Some(pct) = v.strip_suffix('%') {
        return pct
            .trim()
            .parse::<f32>()
            .ok()
            .map(|f| f * parent_fs / 100.0);
    }
    let len = parse_length(v)?;
    #[allow(clippy::cast_possible_truncation)]
    Some(match len.unit {
        crate::value::Unit::Em => len.value as f32 * parent_fs,
        crate::value::Unit::Rem => len.value as f32 * base,
        crate::value::Unit::Ex | crate::value::Unit::Ch => len.value as f32 * parent_fs * 0.5,
        u => (len.value * u.px_factor()?) as f32,
    })
}

/// Attributs de présentation HTML → propriétés CSS
/// (dimension seulement — sous-ensemble documenté). Un
/// attribut `width`/`height` se lit en px CSS ou en `%`
/// (`<td width="50%">`) ; `hr@size` → `height`. Valeur
/// non analysable → ignorée (contenu hostile → dégradation).
/// Jamais au-dessus d'une vraie déclaration : `insert` si
/// la propriété est absente uniquement.
fn apply_presentational_hints(dom: &Dom, node: NodeId, props: &mut HashMap<String, String>) {
    let Ok(tag) = dom.tag_name(node) else {
        return;
    };
    match tag {
        "table" | "td" | "th" | "tr" | "col" | "colgroup" | "img" | "iframe" | "object"
        | "embed" | "canvas" | "video" | "svg" => {
            hint(props, "width", dim(hint_attr(dom, node, "width")));
            hint(props, "height", dim(hint_attr(dom, node, "height")));
        }
        "hr" => {
            hint(props, "width", dim(hint_attr(dom, node, "width")));
            // `size` = hauteur du filet — compteurs/bordures
            // exclus (approximation documentée).
            hint(props, "height", dim(hint_attr(dom, node, "size")));
        }
        _ => {}
    }
    legacy_presentational_hints(dom, node, tag, props);
}

/// Dépose `prop = val` dans `props` si la propriété n'est
/// pas déjà fixée par une vraie règle ou l'attribut `style`
/// (hints = déclarations de spécificité nulle).
fn hint(props: &mut HashMap<String, String>, prop: &str, val: Option<String>) {
    if let Some(v) = val {
        props.entry(prop.to_string()).or_insert(v);
    }
}

/// Valeur d'attribut HTML sur le nœud.
fn hint_attr(dom: &Dom, node: NodeId, name: &str) -> Option<String> {
    dom.attribute(node, name).ok().flatten()
}

/// `N` → "Npx", `N%` → "N%" ; autres syntaxes → None.
fn dim(a: Option<String>) -> Option<String> {
    let v = a?;
    let v = v.trim();
    if v.is_empty() {
        return None;
    }
    if let Some(p) = v.strip_suffix('%') {
        return p
            .trim()
            .parse::<f32>()
            .ok()
            .filter(|f| *f >= 0.0)
            .map(|f| format!("{f}%"));
    }
    v.trim_end_matches("px")
        .parse::<f32>()
        .ok()
        .filter(|f| *f >= 0.0)
        .map(|f| format!("{f}px"))
}

/// `align` → text-align (spec : presentational hint) ;
/// `middle` sur `div` vaut `center` (HTML Rendering).
fn hint_align(a: Option<String>) -> Option<String> {
    match a?.trim().to_ascii_lowercase().as_str() {
        v @ ("left" | "right" | "center" | "justify") => Some(v.to_string()),
        "middle" => Some("center".to_string()),
        _ => None,
    }
}

/// Attribut `name` du `table` ancêtre le plus proche
/// (`cellpadding`/`border` vivent sur la table mais
/// s'appliquent aux cellules — spec HTML Rendering).
/// La **première** table ancêtre est la bonne (`<table>`
/// imbriquée : une cellule relève de SA table, trouvée en
/// remontant — `tr`/`tbody` intermédiaires ne comptent pas).
fn ancestor_table_attr(dom: &Dom, node: NodeId, name: &str) -> Option<String> {
    let mut n = dom.parent(node).ok()?;
    while let Some(id) = n {
        if dom.tag_name(id).is_ok_and(|t| t == "table") {
            return dom.attribute(id, name).ok().flatten();
        }
        n = dom.parent(id).ok()?;
    }
    None
}

/// Hints « legacy » HTML Rendering : `bgcolor`, `align`,
/// `valign`, `cellspacing`/`cellpadding`/`border` des tables
/// (R-16 — les pages tabulaires type HN en dépendent).
fn legacy_presentational_hints(
    dom: &Dom,
    node: NodeId,
    tag: &str,
    props: &mut HashMap<String, String>,
) {
    match tag {
        "body" | "table" | "tr" | "td" | "th" | "colgroup" | "col" | "thead" | "tbody"
        | "tfoot" => {
            hint(
                props,
                "background-color",
                hint_attr(dom, node, "bgcolor").map(|v| v.trim().to_string()),
            );
        }
        _ => {}
    }
    match tag {
        "caption" | "col" | "colgroup" | "div" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6"
        | "legend" | "p" | "td" | "th" | "tr" => {
            hint(
                props,
                "text-align",
                hint_align(hint_attr(dom, node, "align")),
            );
        }
        _ => {}
    }
    match tag {
        "col" | "colgroup" | "tbody" | "td" | "tfoot" | "th" | "thead" | "tr" => {
            hint(
                props,
                "vertical-align",
                hint_attr(dom, node, "valign").map(|v| v.trim().to_string()),
            );
        }
        _ => {}
    }
    match tag {
        "table" => {
            // `cellspacing` → espacement des cellules ; le
            // défaut 2px est émis **en hint** (une règle UA
            // `border-spacing: 2px` écraserait l'attribut —
            // approche Blink : le défaut vit au même niveau
            // que l'attribut). `border` > 0 → bordure de la
            // table (filet 1px inset sur les cellules via
            // `ancestor_table_attr`).
            hint(
                props,
                "border-spacing",
                dim(hint_attr(dom, node, "cellspacing")).or_else(|| Some("2px".to_string())),
            );
            if let Some(bw) = dim(hint_attr(dom, node, "border")) {
                hint(props, "border-width", Some(bw));
                hint(props, "border-style", Some("outset".to_string()));
            }
        }
        "td" | "th" => {
            // `cellpadding` de la table ancêtre → padding des
            // cellules ; `border` > 0 → filet 1px inset.
            hint(
                props,
                "padding",
                ancestor_table_attr(dom, node, "cellpadding").and_then(|v| dim(Some(v))),
            );
            if let Some(b) = ancestor_table_attr(dom, node, "border") {
                if b.trim().parse::<f32>().is_ok_and(|f| f > 0.0) {
                    hint(props, "border-width", Some("1px".to_string()));
                    hint(props, "border-style", Some("inset".to_string()));
                }
            }
        }
        "body" => {
            hint(
                props,
                "color",
                hint_attr(dom, node, "text").map(|v| v.trim().to_string()),
            );
        }
        _ => {}
    }
}

/// Substitue les `var(--name[, fallback])` de `text` depuis
/// `props` — `None` si une référence est irrésoluble (pas de
/// `--name` ni de fallback). Récursion bornée (var dans var,
/// cycles → échec tolérant).
fn substitute_vars(text: &str, props: &HashMap<String, String>, depth: u32) -> Option<String> {
    if depth > 8 {
        return None;
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("var(") {
        out.push_str(&rest[..start]);
        let inner = &rest[start + 4..];
        // Parenthèses équilibrées — le fallback peut contenir
        // un `var(` imbriqué.
        let mut parens = 1usize;
        let mut end = None;
        for (i, c) in inner.char_indices() {
            match c {
                '(' => parens += 1,
                ')' => {
                    parens -= 1;
                    if parens == 0 {
                        end = Some(i);
                        break;
                    }
                }
                _ => {}
            }
        }
        let end = end?;
        let body = &inner[..end];
        rest = &inner[end + 1..];
        // `--name` + virgule de premier niveau → fallback.
        let (name, fallback) = split_top_comma(body);
        let resolved = props
            .get(name.trim())
            .and_then(|v| substitute_vars(v, props, depth + 1));
        // --x résolue (sa propre chaîne var() comprise) →
        // substitution ; invalide ou absente → fallback.
        if let Some(v) = resolved {
            out.push_str(&v);
        } else {
            let fb = fallback?;
            out.push_str(&substitute_vars(fb.trim(), props, depth + 1)?);
        }
    }
    out.push_str(rest);
    Some(out)
}

/// Coupe `a, reste` à la première virgule de niveau 0
/// (parenthèses ignorées) — `None` si pas de virgule.
fn split_top_comma(s: &str) -> (&str, Option<&str>) {
    let mut parens = 0usize;
    for (i, c) in s.char_indices() {
        match c {
            '(' => parens += 1,
            ')' => parens = parens.saturating_sub(1),
            ',' if parens == 0 => return (&s[..i], Some(&s[i + 1..])),
            _ => {}
        }
    }
    (s, None)
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
    let s = engine.compute_with_parent(dom, parent, env, grand.as_ref(), None, state);
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
///
/// `noscript` n'est **pas** masqué : scripting désactivé
/// (périmètre domaine) ⇒ son contenu est un vrai sous-arbre
/// rendu. `noframes`/`noembed` restent visibles pour la même
/// raison (pas de frames ni de plugins — leur contenu EST le
/// repli). `center` porte `-webkit-center` : centre aussi les
/// descendants **bloc** (spec « align descendants to the
/// center », comportement navigateur historique — les tables
/// à `width` en % des pages d'antan en dépendent).
const UA_CSS: &str = "
html, body, div, p, ul, ol, li, dl, dt, dd, blockquote, pre,
form, fieldset, figure, figcaption, h1, h2, h3, h4, h5, h6,
hr, address, article, aside, footer, header, nav, section,
main, legend, center, details, dialog, dir, hgroup,
listing, marquee, menu, plaintext, search, summary, xmp,
frameset { display: block }
table { display: table }
thead { display: table-header-group }
tbody { display: table-row-group }
tfoot { display: table-footer-group }
tr { display: table-row }
col { display: table-column }
colgroup { display: table-column-group }
caption { display: block }
[hidden], head, script, style, link, meta, title, template,
area, base, basefont, datalist, param, source, track,
frame { display: none }
details:not([open]) > :not(summary) { display: none }
dialog:not([open]) { display: none }
audio:not([controls]) { display: none }
li { display: list-item }
td, th { display: table-cell }
center { text-align: -webkit-center }
b, strong { font-weight: bold }
i, em, cite, dfn, var, address { font-style: italic }
u, ins { text-decoration: underline }
s, del, strike { text-decoration: line-through }
code, kbd, pre, samp, tt, listing, plaintext, xmp { font-family: monospace }
pre, textarea { white-space: pre-wrap }
listing, plaintext, xmp { white-space: pre }
h1 { font-size: 2em; font-weight: bold; margin-top: 0.67em; margin-bottom: 0.67em }
h2 { font-size: 1.5em; font-weight: bold; margin-top: 0.83em; margin-bottom: 0.83em }
h3 { font-size: 1.17em; font-weight: bold; margin-top: 1em; margin-bottom: 1em }
h4 { font-size: 1em; font-weight: bold; margin-top: 1.33em; margin-bottom: 1.33em }
h5 { font-size: 0.83em; font-weight: bold; margin-top: 1.67em; margin-bottom: 1.67em }
h6 { font-size: 0.67em; font-weight: bold; margin-top: 2.33em; margin-bottom: 2.33em }
p { margin-top: 1em; margin-bottom: 1em }
body { margin: 8px }
ul, ol, menu, dir, dl, dd { margin-top: 1em; margin-bottom: 1em }
ul, ol, menu, dir { padding-left: 40px }
dd { margin-left: 40px }
blockquote { margin: 1em 40px }
hr { border-top-width: 1px; border-style: inset }
a[href] { color: #00e; text-decoration: underline }
table { border-collapse: separate }
th { font-weight: bold; text-align: center }
input, textarea, select, button, img, svg, canvas { display: inline-block }
input, textarea {
  border: 1px solid #767676; padding: 1px; width: 10em; height: 1.2em
}
textarea { height: 4em }
select { border: 1px solid #767676; padding: 1px 4px }
button { border: 1px solid #767676; padding: 1px 6px; background-color: #eee }
input[type=submit], input[type=button], input[type=reset] {
  width: auto; padding: 1px 6px; background-color: #eee
}
input[type=checkbox], input[type=radio] { width: 13px; height: 13px }
input[type=hidden] { display: none }
";
