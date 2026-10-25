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
use crate::selector::{
    matches_with_state_cached, parse_selector_list, AncestorFilter, ComplexSelector, MatchCache,
    PseudoClass, SimpleSelector, Specificity,
};
use crate::value::{
    expand_shorthand, parse_color, parse_length, value_is_valid_for, Color, Length,
};
use explorer_dom::{Dom, NodeId};
use std::collections::HashMap;
use std::sync::Arc;

/// Schéma de couleurs préféré de l'utilisateur (MQ5
/// `prefers-color-scheme`) — fourni par la session (réglage
/// OS/app) ; `Light` par défaut.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColorScheme {
    /// Clair (défaut navigateur).
    #[default]
    Light,
    /// Sombre — `(prefers-color-scheme: dark)` matche.
    Dark,
}

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
    /// Schéma de couleurs préféré (R-59).
    pub color_scheme: ColorScheme,
    /// « scripting flag » pour la media feature MQ5
    /// `scripting` (R-93) : `true` = un moteur JS exécute le
    /// document → `(scripting: enabled)` ; `false` =
    /// `none`. `initial-only` n'est jamais rapporté (les
    /// scripts tournent aussi après le chargement). La feuille
    /// UA s'en sert pour masquer `<noscript>` — spec HTML :
    /// son contenu ne doit s'afficher que sans scripting.
    pub scripting: bool,
}

impl Default for StyleEnv {
    fn default() -> Self {
        Self {
            media_type: "screen".into(),
            width_px: 1024,
            height_px: 768,
            font_size_base: 16.0,
            color_scheme: ColorScheme::default(),
            scripting: false,
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
/// `em`/`rem` y sont résolus contre la font-size INITIALE
/// (spec MQ4 — jamais le `:root` calculé) : `env.font_size_base`.
fn condition_matches(env: &StyleEnv, name: &str, value: &str) -> bool {
    // Facteur nié `(not (f:v))` — encodé `not-<feature>` au
    // parsing (R-64). Un corps `not` non résoluble en une
    // seule condition reste une feature inconnue → fausse →
    // niée vraie (tolérance « absente de cet env »).
    if let Some(pos) = name.strip_prefix("not-") {
        return !condition_matches(env, pos, value);
    }
    let px = || -> Option<f64> {
        let v = value.trim();
        // Ordre : `rem` avant `em`, `vmin`/`vmax` avant
        // `vw`/`vh` (suffixes partagés). R-42 : les unités
        // viewport sont légales dans les features de taille
        // (MQ4 §6.4) et se résolvent contre le viewport.
        let (num, mul) = if let Some(s) = v.strip_suffix("rem") {
            (s, f64::from(env.font_size_base))
        } else if let Some(s) = v.strip_suffix("em") {
            (s, f64::from(env.font_size_base))
        } else if let Some(s) = v.strip_suffix("vmin") {
            (s, f64::from(env.width_px.min(env.height_px)) / 100.0)
        } else if let Some(s) = v.strip_suffix("vmax") {
            (s, f64::from(env.width_px.max(env.height_px)) / 100.0)
        } else if let Some(s) = v.strip_suffix("vw") {
            (s, f64::from(env.width_px) / 100.0)
        } else if let Some(s) = v.strip_suffix("vh") {
            (s, f64::from(env.height_px) / 100.0)
        } else if let Some(s) = v.strip_suffix("px") {
            (s, 1.0)
        } else {
            (v, 1.0)
        };
        num.trim().parse::<f64>().ok().map(|n| n * mul)
    };
    match name {
        "min-width" => px().is_some_and(|v| f64::from(env.width_px) >= v),
        "max-width" => px().is_some_and(|v| f64::from(env.width_px) <= v),
        // Tolérance sous-pixel : un viewport entier ne devrait
        // jamais dépendre d'un bit de mantisse.
        "width" => px().is_some_and(|v| (f64::from(env.width_px) - v).abs() < 0.5),
        "min-height" => px().is_some_and(|v| f64::from(env.height_px) >= v),
        "max-height" => px().is_some_and(|v| f64::from(env.height_px) <= v),
        // R-56 — bornes strictes des ranges MQ4 normalisées
        // (`(width > N)` / `(width < N)` — la valeur exacte ne
        // match pas).
        "gt-width" => px().is_some_and(|v| f64::from(env.width_px) > v),
        "lt-width" => px().is_some_and(|v| f64::from(env.width_px) < v),
        "gt-height" => px().is_some_and(|v| f64::from(env.height_px) > v),
        "lt-height" => px().is_some_and(|v| f64::from(env.height_px) < v),
        "height" => px().is_some_and(|v| (f64::from(env.height_px) - v).abs() < 0.5),
        "orientation" => match value {
            "landscape" => env.width_px >= env.height_px,
            "portrait" => env.width_px < env.height_px,
            _ => false,
        },
        // R-59 — MQ5 `prefers-color-scheme` : `light`/`dark`
        // contre l'env ; feature nue (contexte booléen) →
        // vrai quel que soit le schéma.
        "prefers-color-scheme" => match value.trim() {
            "" => true,
            "light" => env.color_scheme == ColorScheme::Light,
            "dark" => env.color_scheme == ColorScheme::Dark,
            _ => false,
        },
        // R-93 — MQ5 `scripting` (Media Queries 5 §scripting) :
        // `enabled` = un moteur JS exécute le document (env
        // `scripting`) ; `none` = aucun. `initial-only`
        // n'est jamais rapporté par notre moteur — les
        // scripts tournent aussi après le chargement → toujours
        // faux, retombe sur `_`. Feature nue `(scripting)`
        // = « pas none ».
        "scripting" => match value.trim() {
            "" | "enabled" => env.scripting,
            "none" => !env.scripting,
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
    /// Arbre d'appartenance (R-96) : `None` = document ;
    /// `Some(racine shadow)` = feuille confinée à cet arbre
    /// shadow (un `<style>` en shadow n'atteint ni le
    /// document ni les autres racines — seuls `:host` sujet
    /// et `::part` franchissent, cf. `collect_candidates`).
    scope: Option<NodeId>,
    /// Chemin de rangs `@layer` (CSS Cascade 5 §6.4 — R-91) :
    /// un rang par niveau de couche imbriquée, attribué à la
    /// première déclaration du nom (comparaison
    /// lexicographique). Vide = hors-layer — qui bat TOUS les
    /// layers en déclarations normales et leur cède en
    /// `!important` (ordre renversé).
    layer: Vec<u32>,
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

/// Position d'un sélecteur dans la table des règles + son
/// pseudo-élément cible pré-extrait (le filtre `::before` de
/// la cascade devient un test de champ, R-83).
#[derive(Debug, Clone, Copy)]
struct SelEntry {
    /// Index dans la table des règles (`rules` ou
    /// `quirks_rules`).
    rule: u32,
    /// Index dans `rules[rule].selectors`.
    selector: u32,
    /// Pseudo-élément du composé final, `None` = règle
    /// d'élément.
    pseudo: Option<crate::PseudoElement>,
}

/// Index des sélecteurs par **clé du composé le plus à
/// droite** (R-83) — celui qui s'applique au nœud. Avant,
/// `collect_candidates` essayait chaque règle sur chaque
/// nœud, trois fois par élément (élément + `::before` +
/// `::after`) : O(règles × nœuds × 3) ≈ ~13 s mesurés sur
/// un article réel de 32 800 nœuds. La clé est une
/// condition **nécessaire** du match — `div.a[href]` se
/// range sous `Class(a)` (id > classe > attribut > type) :
/// tout simple présent doit matcher, n'importe lequel borne
/// donc les candidats. Le seau `universal` reçoit les
/// sélecteurs sans clé extractible (`*`, `:root`, `:nth-*`,
/// ou composé réduit à `:is()`/`:not()`/`:where()` — leurs
/// clés internes sont des alternatives : ranger sous l'une
/// manquerait les nœuds des autres).
#[derive(Debug, Default)]
struct SelectorIndex {
    /// `#id` exact → entrées (HTML = sensible à la casse).
    by_id: HashMap<String, Vec<SelEntry>>,
    /// `.classe` exacte → entrées.
    by_class: HashMap<String, Vec<SelEntry>>,
    /// Nom d'attribut minuscule → entrées. Minuscules des
    /// deux côtés : le lookup attribut du DOM est exact mais
    /// le parseur HTML stocke déjà les noms en bas-de-casse —
    /// normaliser ne change aucun verdict (`[HREF]` ratait
    /// déjà un `href` stocké, et rate encore).
    by_attr: HashMap<String, Vec<SelEntry>>,
    /// Tag minuscule → entrées (le match type est
    /// ASCII-insensitive).
    by_tag: HashMap<String, Vec<SelEntry>>,
    /// Sélecteurs sans clé — essayés sur tout élément.
    universal: Vec<SelEntry>,
    /// Au moins un sélecteur cible `::before`/`::after` —
    /// permet de sauter le calcul pseudo complet sur les
    /// feuilles qui n'en produisent aucun.
    has_before: bool,
    /// Voir `has_before`.
    has_after: bool,
}

impl SelectorIndex {
    /// Range chaque sélecteur de `rules` sous la clé de son
    /// composé final — une entrée par (règle, sélecteur).
    fn build(rules: &[CollectedRule]) -> Self {
        let mut idx = Self::default();
        for (ri, rule) in rules.iter().enumerate() {
            for (si, sel) in rule.selectors.iter().enumerate() {
                let entry = SelEntry {
                    rule: u32::try_from(ri).unwrap_or(u32::MAX),
                    selector: u32::try_from(si).unwrap_or(u32::MAX),
                    pseudo: crate::selector::selector_pseudo_element(sel),
                };
                match entry.pseudo {
                    Some(crate::PseudoElement::Before) => idx.has_before = true,
                    Some(crate::PseudoElement::After) => idx.has_after = true,
                    None => {}
                }
                match rightmost_key(sel) {
                    BucketKey::Id(k) => idx.by_id.entry(k).or_default().push(entry),
                    BucketKey::Class(k) => idx.by_class.entry(k).or_default().push(entry),
                    BucketKey::Attr(k) => idx.by_attr.entry(k).or_default().push(entry),
                    BucketKey::Tag(k) => idx.by_tag.entry(k).or_default().push(entry),
                    BucketKey::Universal => idx.universal.push(entry),
                }
            }
        }
        idx
    }

    /// Appelle `out` sur les entrées dont la clé est portée
    /// par `node` (+ le seau universel). Chaque entrée est
    /// livrée une fois — un sélecteur n'est rangé que sous
    /// une clé.
    fn candidates(&self, dom: &Dom, node: NodeId, out: &mut impl FnMut(&SelEntry)) {
        if let Ok(tag) = dom.tag_name(node) {
            if let Some(v) = self.by_tag.get(&tag.to_ascii_lowercase()) {
                for e in v {
                    out(e);
                }
            }
        }
        if let Ok(attrs) = dom.attributes(node) {
            for a in attrs {
                if a.name == "class" {
                    for c in a.value.split_ascii_whitespace() {
                        if let Some(v) = self.by_class.get(c) {
                            for e in v {
                                out(e);
                            }
                        }
                    }
                } else if a.name == "id" {
                    if let Some(v) = self.by_id.get(a.value.as_str()) {
                        for e in v {
                            out(e);
                        }
                    }
                }
                if let Some(v) = self.by_attr.get(&a.name.to_ascii_lowercase()) {
                    for e in v {
                        out(e);
                    }
                }
            }
        }
        for e in &self.universal {
            out(e);
        }
    }
}

/// Clé de rangement d'un sélecteur (cf. [`SelectorIndex`]).
#[derive(Debug)]
enum BucketKey {
    /// `#id` — valeur exacte.
    Id(String),
    /// `.classe` — valeur exacte.
    Class(String),
    /// `[attr …]` — nom minuscule.
    Attr(String),
    /// `div` — minuscule.
    Tag(String),
    /// Aucune condition nécessaire indexable.
    Universal,
}

/// Clé du composé final de `sel` — la première condition
/// nécessaire au niveau le plus contraignant (id > classe >
/// attribut > type) ; le matching exige TOUS les simples du
/// composé, n'importe lequel suffit à borner les candidats.
/// Les contenus de `:is()`/`:not()`/`:where()` ne sont pas
/// des clés (alternatives ou négations).
fn rightmost_key(sel: &ComplexSelector) -> BucketKey {
    let Some((compound, _)) = sel.parts.last() else {
        return BucketKey::Universal;
    };
    let mut class = None;
    let mut attr = None;
    let mut tag = None;
    for s in &compound.simples {
        match s {
            // `E::part(n)` : le sujet est l'élément `part` en
            // shadow — `E` et consorts qualifient l'HÔTE, pas
            // le nœud → aucune clé utilisable (R-96).
            SimpleSelector::Part(_) => return BucketKey::Universal,
            SimpleSelector::Id(i) => return BucketKey::Id(i.clone()),
            SimpleSelector::Class(c) => {
                class.get_or_insert(c);
            }
            SimpleSelector::Attribute { name, .. } => {
                attr.get_or_insert(name);
            }
            SimpleSelector::Type(t) => {
                tag.get_or_insert(t);
            }
            SimpleSelector::Universal
            | SimpleSelector::Pseudo(_)
            | SimpleSelector::PseudoElement(_) => {}
        }
    }
    if let Some(c) = class {
        BucketKey::Class(c.clone())
    } else if let Some(a) = attr {
        BucketKey::Attr(a.to_ascii_lowercase())
    } else if let Some(t) = tag {
        BucketKey::Tag(t.to_ascii_lowercase())
    } else {
        BucketKey::Universal
    }
}

/// Moteur de cascade : accumulateur de feuilles + calcul.
#[derive(Default)]
pub struct StyleEngine {
    rules: Vec<CollectedRule>,
    /// Règles UA **quirks-only** (HTML Rendering, « In quirks
    /// mode ») — parsées avec la feuille UA, ne candidatent
    /// que si `dom.document_mode() == DocumentMode::Quirks`.
    quirks_rules: Vec<CollectedRule>,
    /// Index de `rules` par clé de composé final (R-83) —
    /// reconstruit à chaque `collect_rules` (O(règles),
    /// négligeable à côté du parse de la feuille).
    index: SelectorIndex,
    /// Index de `quirks_rules` — construit dans `new` (la
    /// table quirks est figée après la feuille UA).
    quirks_index: SelectorIndex,
    next_order: u32,
    /// Rangs des layers `@layer` : chemin de noms complet
    /// (`a.b` sous `a` → `[a, b]`) → rang de première
    /// déclaration, toutes feuilles confondues (R-91). Le rang
    /// d'un niveau ne sert que contre les couches de même
    /// parent — la comparaison est lexicographique sur le
    /// chemin.
    layers: HashMap<Vec<String>, u32>,
    next_layer: u32,
    /// Compteur des layers anonymes (`@layer {…}` sans nom) —
    /// chacun est un slot unique, ordonné à sa position.
    next_anon_layer: u32,
    /// `@font-face` avec leur contexte media (étape 46).
    pub(crate) font_faces: Vec<CollectedFontFace>,
    /// `@keyframes` par nom — offsets triés, multi-sélecteurs
    /// éclatés (R-10, module `animation`).
    pub(crate) keyframes: crate::animation::Keyframes,
}

/// « Guaranteed-invalid value » (css-variables-1 §3.2) :
/// l'initiale d'une propriété custom `--*`. Stockée en sentinelle
/// dans la table `vars` (elle masque l'héritage — la spec veut
/// la valeur, pas « absente ») ; jamais substituable : un
/// `var()` qui la référence échoue et prend son fallback. Les
/// API (`get`/`iter`) la filtrent — invisible de l'extérieur.
const GUARANTEED_INVALID: &str = "\u{1}\u{fffd}giv\u{1}";

/// Computed style d'un nœud : propriété (minuscules) → valeur
/// texte résolue (mot-clés `inherit`/`initial`/`unset` déjà
/// réduits ; le *typage* est l'affaire de l'étape 16).
///
/// R-57 — les propriétés custom `--*` vivent dans une table
/// effective **partagée** (`Arc`) : un nœud sans déclaration
/// `--*` propre référence la table du parent (héritage O(1)
/// par bump de refcount au lieu d'une copie O(vars) — mesuré
/// ~3,2 M d'entrées dupliquées sur Wikipedia, ~1 Go de
/// rétention commise). Un nœud qui en déclare paie un
/// clone-on-write une fois (`Arc::make_mut`). `Arc` et non
/// `Rc` : le payload de peinture traverse `spawn_blocking`
/// (`Send` requis). Props normales : `HashMap` plat inchangé.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ComputedStyle {
    props: HashMap<String, String>,
    /// Table `--*` effective (héritées ∪ déclarées propres).
    vars: Arc<HashMap<String, String>>,
}

impl ComputedStyle {
    /// Valeur calculée d'une propriété, `None` si absente
    /// (→ valeur initiale ou héritée selon la propriété).
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&str> {
        if name.starts_with("--") {
            return self
                .vars
                .get(name)
                .map(String::as_str)
                .filter(|v| *v != GUARANTEED_INVALID);
        }
        self.props.get(name).map(String::as_str)
    }

    /// Itère les propriétés présentes (ordre indéfini).
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.props
            .iter()
            .chain(
                self.vars
                    .iter()
                    .filter(|(_, v)| v.as_str() != GUARANTEED_INVALID),
            )
            .map(|(k, v)| (k.as_str(), v.as_str()))
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
    /// R-57 : une `--*` écrit dans la table partagée par
    /// clone-on-write — la surcharge ne touche pas le parent.
    pub fn set(&mut self, name: &str, value: &str) {
        if name.starts_with("--") {
            Arc::make_mut(&mut self.vars).insert(name.to_string(), value.to_string());
        } else {
            self.props.insert(name.to_string(), value.to_string());
        }
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

    /// Fusionne une map scopée (issue de
    /// [`StyleEngine::compute_scoped`]) dans `self` : chaque
    /// nœud du sous-arbre recalculé écrase l'entrée stockée,
    /// pseudo-styles compris (`::before`/`::after` non
    /// reproduits sont purgés — la map scopée fait autorité
    /// sur son sous-arbre). Les nœuds hors du sous-arbre
    /// recalculé sont inchangés.
    pub fn apply_scoped(&mut self, scoped: StyleMap) {
        for (node, style) in scoped.map {
            self.map.insert(node, style);
            // La map scopée fait autorité par nœud couvert :
            // un `::before`/`::after` non reproduit est purgé.
            match scoped.pseudos.get(&node) {
                Some(ps) => {
                    self.pseudos.insert(node, ps.clone());
                }
                None => {
                    self.pseudos.remove(&node);
                }
            }
        }
    }

    /// Retire style et pseudo-styles de `node` — sous-arbre
    /// détaché, nœud supprimé du DOM.
    pub fn remove(&mut self, node: NodeId) {
        self.map.remove(&node);
        self.pseudos.remove(&node);
    }
}

impl StyleEngine {
    /// Moteur préchargé avec la feuille UA intégrée (règles
    /// standards + règles conditionnelles quirks).
    #[must_use]
    pub fn new() -> Self {
        let mut e = Self::default();
        e.add_css(UA_CSS, Origin::UserAgent);
        let quirks_sheet = crate::parser::parse_stylesheet(QUIRKS_UA_CSS);
        e.collect_rules_into(&quirks_sheet.rules, Origin::UserAgent, &[], &[], true, None);
        e.quirks_index = SelectorIndex::build(&e.quirks_rules);
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

    /// Feuille **confinée à un arbre shadow** (R-96) : ses
    /// règles n'atteignent que les nœuds de la racine
    /// `scope` — plus l'hôte lui-même via les sélecteurs
    /// `:host*` et les `::part` exposés. Un `<style>` trouvé
    /// en shadow se branche ici (collecte par arbre dans
    /// `explorer-core`), pas dans `add_stylesheet`.
    pub fn add_scoped_stylesheet(&mut self, sheet: &Stylesheet, origin: Origin, scope: NodeId) {
        self.collect_rules_scoped(&sheet.rules, origin, &[], scope);
    }

    /// Variante texte de [`StyleEngine::add_scoped_stylesheet`].
    pub fn add_scoped_css(&mut self, input: &str, origin: Origin, scope: NodeId) {
        let sheet = crate::parser::parse_stylesheet(input);
        self.add_scoped_stylesheet(&sheet, origin, scope);
    }

    /// Aplati récursif : règles qualifiées + corps `@media`.
    /// `media` = pile des listes englobantes (AND entre listes,
    /// OR entre les queries d'une même liste).
    fn collect_rules(&mut self, rules: &[Rule], origin: Origin, media: &[Vec<MediaQuery>]) {
        self.collect_rules_into(rules, origin, media, &[], false, None);
        // L'index reflète toujours la table complète — une
        // feuille ajoutée entre deux calculs doit être vue.
        self.index = SelectorIndex::build(&self.rules);
    }

    /// Contrepartie scopée de [`StyleEngine::collect_rules`]
    /// (R-96) : même aplatissement, règles estampillées
    /// `scope` (racine shadow propriétaire).
    fn collect_rules_scoped(
        &mut self,
        rules: &[Rule],
        origin: Origin,
        media: &[Vec<MediaQuery>],
        scope: NodeId,
    ) {
        self.collect_rules_into(rules, origin, media, &[], false, Some(scope));
        self.index = SelectorIndex::build(&self.rules);
    }

    /// Rang de la couche `path` (noms complets par niveau) :
    /// **ordre de première déclaration** — un `@layer a {}`
    /// postérieur à `@layer b {}` garde son rang initial même
    /// si son bloc est lu plus tard (§6.4).
    fn layer_slot(&mut self, path: &[String]) -> u32 {
        if let Some(s) = self.layers.get(path) {
            return *s;
        }
        let s = self.next_layer;
        self.next_layer = self.next_layer.saturating_add(1);
        self.layers.insert(path.to_vec(), s);
        s
    }

    /// Collecte vers `self.rules` — ou vers `quirks_rules` quand
    /// `quirks` est vrai (règles filtrées au document mode).
    /// `layer` = chemin de noms des couches `@layer`
    /// englobantes (R-91). `scope` = racine shadow
    /// propriétaire (R-96, `None` = document).
    #[allow(clippy::too_many_arguments)]
    fn collect_rules_into(
        &mut self,
        rules: &[Rule],
        origin: Origin,
        media: &[Vec<MediaQuery>],
        layer: &[String],
        quirks: bool,
        scope: Option<NodeId>,
    ) {
        for rule in rules {
            match rule {
                Rule::Qualified(q) => {
                    if let Some(selectors) = parse_selector_list(&q.prelude) {
                        let layer_ranks: Vec<u32> = (1..=layer.len())
                            .map(|i| self.layers[&layer[..i]])
                            .collect();
                        let dest = if quirks {
                            &mut self.quirks_rules
                        } else {
                            &mut self.rules
                        };
                        dest.push(CollectedRule {
                            selectors,
                            declarations: q.declarations.clone(),
                            order: self.next_order,
                            origin,
                            scope,
                            layer: layer_ranks,
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
                        self.collect_rules_into(rules, origin, &nested, layer, quirks, scope);
                    } else if let AtKind::Layer { name, rules } = &at.kind {
                        // `@layer nom {…}` : les règles entrent
                        // dans la couche — l'ordre du layer est
                        // celui de sa PREMIÈRE déclaration.
                        let mut path = layer.to_vec();
                        if let Some(n) = name {
                            for seg in n.split('.') {
                                let seg = seg.trim();
                                if !seg.is_empty() {
                                    path.push(seg.to_string());
                                }
                            }
                        } else {
                            // Anonyme : nom synthétique unique
                            // (non référençable par une feuille
                            // ultérieure — spec).
                            path.push(format!("\u{0}a{}", self.next_anon_layer));
                            self.next_anon_layer = self.next_anon_layer.saturating_add(1);
                        }
                        // Déclare chaque préfixe au passage.
                        for i in 1..=path.len() {
                            self.layer_slot(&path[..i]);
                        }
                        self.collect_rules_into(rules, origin, media, &path, quirks, scope);
                    } else if let AtKind::Statement { prelude } = &at.kind {
                        // `@layer a, b;` : déclare l'ordre sans
                        // règles (les corps s'y raccordent par
                        // nom — premier arrivé fixe le rang).
                        if at.name == "layer" {
                            let mut path = layer.to_vec();
                            for part in prelude.split(',') {
                                for seg in part.split('.') {
                                    let seg = seg.trim();
                                    if !seg.is_empty() {
                                        path.push(seg.to_string());
                                        self.layer_slot(&path);
                                    }
                                }
                                path.truncate(layer.len());
                            }
                        }
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
        let mut mcache = MatchCache::default();
        let root_fs = self.root_font_size(dom, env, state);
        let parent = parent_computed(
            self,
            dom,
            node,
            env,
            state,
            &mut HashMap::new(),
            &mut mcache,
            root_fs,
        );
        self.compute_with_parent(
            dom,
            node,
            env,
            parent.as_ref(),
            None,
            state,
            &mut mcache,
            &AncestorFilter::of_ancestors(dom, node),
            root_fs,
        )
    }

    /// Font-size calculée de `documentElement` — base `rem`
    /// du document (CSS Values §5.2). Repli : initiale
    /// (`env.font_size_base`) — absence de racine, ou valeur
    /// non résolue (`calc()` hostile…).
    fn root_font_size(&self, dom: &Dom, env: &StyleEnv, state: &dyn Fn(&str) -> bool) -> f32 {
        let Some(doc_el) = dom.document_element() else {
            return env.font_size_base;
        };
        // Le nœud est la racine : son propre `font-size:Nrem`
        // se résout contre l'initiale — garde d'identité dans
        // `compute_with_parent`. Appel direct : passer par
        // `compute_with_state` re-bouclerait ici sans fin.
        let mut mcache = MatchCache::default();
        self.compute_with_parent(
            dom,
            doc_el,
            env,
            None,
            None,
            state,
            &mut mcache,
            &AncestorFilter::of_ancestors(dom, doc_el),
            env.font_size_base,
        )
        .get("font-size")
        .and_then(|v| v.trim().trim_end_matches("px").trim().parse::<f32>().ok())
        .filter(|fs| *fs > 0.0)
        .unwrap_or(env.font_size_base)
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
        let mut mcache = MatchCache::default();
        // Ancêtres de `root` : un sous-arbre peut être stylé
        // hors racine document — leurs clés restent visibles
        // pour les combinators ancêtres (même contrat que
        // `compute_scoped`).
        let anc = AncestorFilter::of_ancestors(dom, root);
        // Base `rem` — fs calculée de `documentElement` ;
        // la marche la met à jour si la racine est dans le
        // sous-arbre.
        let mut root_fs = self.root_font_size(dom, env, state);
        self.compute_subtree(
            dom,
            root,
            env,
            None,
            &mut map,
            &mut pseudos,
            state,
            &mut mcache,
            &anc,
            &mut root_fs,
        );
        StyleMap { map, pseudos }
    }

    /// Re-style le sous-arbre de `root` semé sur le style
    /// `parent` déjà calculé — les sélecteurs matchent contre
    /// le DOM réel (ancêtres et frères au-dessus du sous-arbre
    /// restent visibles : `descendant`, `~`, `+`, `:nth-*`),
    /// seule l'écriture est bornée au sous-arbre.
    /// Contrepartie incrémentale de
    /// [`compute_tree`](Self::compute_tree) — demande NAV
    /// (55g-suite : coût unitaire de l'apply sur DOM muté).
    ///
    /// La map produite ne contient que les nœuds du
    /// sous-arbre ; le consommateur la fusionne dans sa map
    /// stockée via [`StyleMap::apply_scoped`]. Le `root`
    /// est au contrat de l'appelant : tout nœud dont le
    /// style peut dépendre des nœuds modifiés doit s'y
    /// trouver — un `appendChild` couvre le sous-arbre
    /// inséré, mais des frères ciblés par `~`/`+`/`:nth-*`
    /// ou des descendants d'un ancêtre dont les propres
    /// styles changent relèvent d'un `root` élargi (la
    /// cascade scopée ne devine pas la portée).
    #[must_use]
    pub fn compute_scoped(
        &self,
        dom: &Dom,
        root: NodeId,
        env: &StyleEnv,
        parent: Option<&ComputedStyle>,
    ) -> StyleMap {
        self.compute_scoped_with_state(dom, root, env, parent, &|_| false)
    }

    /// Variante étatée de [`StyleEngine::compute_scoped`].
    #[must_use]
    pub fn compute_scoped_with_state(
        &self,
        dom: &Dom,
        root: NodeId,
        env: &StyleEnv,
        parent: Option<&ComputedStyle>,
        state: &dyn Fn(&str) -> bool,
    ) -> StyleMap {
        let mut map = HashMap::new();
        let mut pseudos = HashMap::new();
        let mut mcache = MatchCache::default();
        // Les ancêtres au-dessus du sous-arbre restent
        // visibles au matching (contrat du restyle scopé).
        let anc = AncestorFilter::of_ancestors(dom, root);
        let mut root_fs = self.root_font_size(dom, env, state);
        self.compute_subtree(
            dom,
            root,
            env,
            parent,
            &mut map,
            &mut pseudos,
            state,
            &mut mcache,
            &anc,
            &mut root_fs,
        );
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
        mcache: &mut MatchCache,
        anc: &AncestorFilter,
        root_fs: &mut f32,
    ) {
        let style =
            self.compute_with_parent(dom, node, env, parent, None, state, mcache, anc, *root_fs);
        // L'élément racine vient d'être calculé : sa font-size
        // devient la base `rem` de tout le document (la marche
        // est parent-d'abord — ses descendants viennent après).
        if dom.document_element() == Some(node) {
            *root_fs = style
                .get("font-size")
                .and_then(|v| v.trim().trim_end_matches("px").trim().parse::<f32>().ok())
                .unwrap_or(env.font_size_base);
        }
        // Pseudo-éléments (R-13) : l'élément origine est le
        // parent d'héritage de ses `::before`/`::after`. Le
        // calcul est sauté quand aucune feuille n'a de règle
        // pseudo correspondante (R-83) — `content` ne peut
        // alors jamais être produit.
        if dom.element(node).is_ok() {
            let mut pseudo_style = |pe: crate::PseudoElement| -> Option<ComputedStyle> {
                if !self.has_pseudo_rules(dom, pe) {
                    return None;
                }
                let s = self.compute_with_parent(
                    dom,
                    node,
                    env,
                    Some(&style),
                    Some(pe),
                    state,
                    mcache,
                    anc,
                    *root_fs,
                );
                (s.get("content").is_some()).then_some(s)
            };
            let before = pseudo_style(crate::PseudoElement::Before);
            let after = pseudo_style(crate::PseudoElement::After);
            let ps = PseudoStyles { before, after };
            if ps.before.is_some() || ps.after.is_some() {
                pseudos.insert(node, ps);
            }
        }
        let Ok(children) = dom.children(node) else {
            map.insert(node, style);
            return;
        };
        // Le style local est prêté aux enfants (`&style`) —
        // aucun clone du style parent par nœud (R-83) ;
        // l'insertion dans `map` suit la récursion.
        // Les enfants voient `node` dans leur filtre
        // d'ancêtres (copie — le frère ne doit pas hériter
        // des clés de son aîné).
        let mut child_anc = *anc;
        child_anc.insert_node(dom, node);
        for child in children {
            if dom.element(child).is_ok() {
                // Récursion ∝ profondeur DOM → pile garantie
                // (R-24) : bascule tas en zone rouge.
                explorer_common::deep(|| {
                    self.compute_subtree(
                        dom,
                        child,
                        env,
                        Some(&style),
                        map,
                        pseudos,
                        state,
                        &mut *mcache,
                        &child_anc,
                        root_fs,
                    );
                });
            }
        }
        // R-96 — arbre shadow : le flat tree de l'hôte est
        // fait des enfants de sa racine (latérale — hors
        // `children`). L'héritage franchit la frontière :
        // le style de l'hôte est le parent des enfants
        // shadow (css-scoping §3.3 — les `--*` et propriétés
        // héritées traversent, les sélecteurs non : le
        // filtre de `collect_candidates` tient la porte).
        if let Some(root) = dom.shadow_root(node, true) {
            if let Ok(schildren) = dom.children(root) {
                for child in schildren {
                    if dom.element(child).is_ok() {
                        explorer_common::deep(|| {
                            self.compute_subtree(
                                dom,
                                child,
                                env,
                                Some(&style),
                                map,
                                pseudos,
                                state,
                                &mut *mcache,
                                &child_anc,
                                root_fs,
                            );
                        });
                    }
                }
            }
        }
        map.insert(node, style);
    }

    /// Calcul d'un nœud connaissant le style de son parent.
    /// `pseudo` = `Some(pe)` → style du pseudo-élément `pe` du
    /// nœud (R-13) ; `None` → style de l'élément lui-même.
    /// `root_fs` = font-size calculée de `documentElement` —
    /// base des `rem` (CSS Values §5.2 ; sur le `font-size` de
    /// la racine elle-même `rem` vaut l'initiale, pas la
    /// calculée — la racine n'a pas de racine au-dessus).
    #[allow(clippy::too_many_lines)]
    #[allow(clippy::too_many_arguments)]
    fn compute_with_parent(
        &self,
        dom: &Dom,
        node: NodeId,
        env: &StyleEnv,
        parent: Option<&ComputedStyle>,
        pseudo: Option<crate::PseudoElement>,
        state: &dyn Fn(&str) -> bool,
        mcache: &mut MatchCache,
        anc: &AncestorFilter,
        root_fs: f32,
    ) -> ComputedStyle {
        let mut props: HashMap<String, String> = HashMap::new();
        if dom.element(node).is_err() {
            return ComputedStyle::default();
        }
        // `rem` : initiale sur le `font-size` de `:root` — et
        // seulement lui (ses pseudos `::before`/`::after`
        // résolvent déjà contre `root_fs`, comme tout
        // descendant).
        let rem_base = if pseudo.is_none() && dom.document_element() == Some(node) {
            env.font_size_base
        } else {
            root_fs
        };
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
        let mut cand = self.collect_candidates(dom, node, env, pseudo, state, mcache, anc);
        if pseudo.is_none() {
            for d in &inline_decls {
                let rank = if d.important { 4 } else { 1 };
                // `style=` est hors-layer — la clé miroir gère
                // le renversement `!important` (R-91).
                cand.push((
                    rank,
                    layer_sort_key(&[], d.important),
                    (u32::MAX, u32::MAX, u32::MAX),
                    u32::MAX,
                    d,
                ));
            }
            // Le rang inline s'intercale dans l'ordre global.
            cand.sort_by_key(|(rank, layer, spec, order, _)| (*rank, layer.clone(), *spec, *order));
        }
        for (_, _, _, _, d) in &cand {
            // Les noms de propriétés ordinaires sont
            // ASCII-insensibles → minuscules canoniques ; les
            // propriétés custom `--*` sont SENSIBLES À LA CASSE
            // (css-variables-1 §2.1) — `--Foo` et `--foo` sont
            // deux propriétés distinctes : les lecteurs
            // (`get`, `var()`) adressent le nom tel qu'écrit,
            // le stockage doit le conserver verbatim (R-92 —
            // `--Spacer-size1012` de github s'effondrait en
            // minuscules → `var(--Spacer-size1012)` raté →
            // chaîne de fallbacks au littéral terminal).
            let lname = if d.name.starts_with("--") {
                d.name.clone()
            } else {
                d.name.to_ascii_lowercase()
            };
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
        }
        // Propriétés custom `--*` : héritées par défaut (spec)
        // — R-57 : la table effective du parent est PARTAGÉE
        // (bump `Arc` O(1)) au lieu de recopier chaque entrée ;
        // les `--*` déclarées par ce nœud vivent dans `props`
        // jusqu'à l'extraction finale (mots-clés/var() ci-dessous
        // les traitent comme avant).
        let mut vars: Arc<HashMap<String, String>> =
            parent.map_or_else(|| Arc::new(HashMap::new()), |p| Arc::clone(&p.vars));
        // 4. Mots-clés CSS-wide sur les valeurs déclarées.
        for (k, v) in &mut props {
            // Fast-path : la valeur ordinaire (quasi toutes)
            // n'est aucun mot-clé CSS-wide → pas d'alloc
            // `lowercase` par propriété (R-83).
            let t = v.trim();
            if !is_css_wide(t) {
                continue;
            }
            let kw = t.to_ascii_lowercase();
            // Propriétés custom `--*` (css-variables-1 §3.2) :
            // l'« initiale » d'une --* est la *guaranteed-invalid
            // value*, pas une valeur de la table ordinaire —
            // la sentinelle masque l'héritage et fait échouer
            // tout `var()` qui la référence (→ fallback). Une
            // --* réduite à `""` par `initial_value()` comme
            // avant rendait la substitution « réussie » avec
            // un stream tronqué (` #fff` chez MDN → texte
            // blanc sur blanc, R-80).
            if k.starts_with("--") {
                *v = match kw.as_str() {
                    "initial" => GUARANTEED_INVALID.to_string(),
                    // --* hérite : `inherit`/`unset`/`revert`
                    // lisent la valeur du parent (déjà GIV le
                    // cas échéant) ; parent sans la propriété
                    // → l'initiale, donc GIV.
                    "inherit" | "unset" | "revert" | "revert-layer" => vars
                        .get(k)
                        .cloned()
                        .unwrap_or_else(|| GUARANTEED_INVALID.to_string()),
                    _ => continue,
                };
                continue;
            }
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
        //    R-57 → R-83 : le passage est sauté quand aucune
        //    valeur ne contient `var(` ; et le snapshot
        //    `props.clone()` laisse place à une résolution en
        //    deux passes — le lookup lit `props` immuablement,
        //    la mutation suit (le clone complet par nœud
        //    disparaît ; lookup : propres d'abord, elles
        //    masquent l'héritage `vars`).
        if props
            .iter()
            .any(|(k, v)| !k.starts_with("--") && v.contains("var("))
        {
            // Une `--*` à la *guaranteed-invalid value* est « non
            // résoluble » : le `var()` qui la référence échoue et
            // prend son fallback (css-variables §3.2) — `None` ici.
            let lookup = |name: &str| -> Option<String> {
                props
                    .get(name)
                    .cloned()
                    .or_else(|| vars.get(name).cloned())
                    .filter(|v| v != GUARANTEED_INVALID)
            };
            let resolved: Vec<(String, Option<String>)> = props
                .iter()
                .filter(|(k, v)| !k.starts_with("--") && v.contains("var("))
                .map(|(k, v)| (k.clone(), substitute_vars(v, &lookup, 0)))
                .collect();
            for (k, r) in resolved {
                if let Some(res) = r {
                    props.insert(k, res);
                } else {
                    if is_inherited(&k) {
                        if let Some(pv) = parent.and_then(|p| p.get(&k)) {
                            props.insert(k, pv.to_string());
                            continue;
                        }
                    }
                    props.remove(&k);
                }
            }
        }
        // R-57 — les `--*` déclarées rejoignent la table
        // partagée : clone-on-write seulement si le nœud en
        // possède (une `--*` propre masque l'héritée du même
        // nom — insertion après héritage partagé = sémantique
        // identique à la copie plate conditionnelle).
        if props.keys().any(|k| k.starts_with("--")) {
            let own: Vec<String> = props
                .keys()
                .filter(|k| k.starts_with("--"))
                .cloned()
                .collect();
            let m = Arc::make_mut(&mut vars);
            for k in own {
                if let Some(v) = props.remove(&k) {
                    m.insert(k, v);
                }
            }
        }
        // 6. `font-size` → valeur calculée absolue (R-17 —
        //    détail dans `normalize_font_size`).
        normalize_font_size(&mut props, parent, env, rem_base);
        // 7. `color-scheme` sur la racine (R-59) : schéma
        //    `dark` utilisé → défauts UA sombres sur les
        //    propriétés restées initiales.
        if parent.is_none() && pseudo.is_none() {
            Self::apply_scheme_defaults(&mut props, env);
        }
        ComputedStyle { props, vars }
    }

    /// R-59 — défauts UA sombres (§ css-color-adjust-1) : le
    /// `color-scheme` déclaré sur la racine arbitre le schéma
    /// utilisé — `dark` s'applique quand l'env préfère le
    /// sombre **ou** qu'aucun `light` n'est déclaré (un
    /// `light` explicite est un opt-out même sous env sombre ;
    /// `only light` = opt-out par construction). Les défauts
    /// ne remplissent que les propriétés encore initiales —
    /// équivalent rang UA : l'auteur garde la main.
    fn apply_scheme_defaults(props: &mut HashMap<String, String>, env: &StyleEnv) {
        let Some(scheme) = props.get("color-scheme") else {
            return;
        };
        let dark = scheme
            .split_ascii_whitespace()
            .any(|t| t.eq_ignore_ascii_case("dark"))
            && (env.color_scheme == ColorScheme::Dark
                || !scheme
                    .split_ascii_whitespace()
                    .any(|t| t.eq_ignore_ascii_case("light")));
        if !dark {
            return;
        }
        for (name, v) in [("background-color", "#141414"), ("color", "#e6e6e6")] {
            props
                .entry(name.to_string())
                .or_insert_with(|| v.to_string());
        }
    }

    /// `true` si au moins une règle collectée cible le
    /// pseudo-élément `pe` — permet de sauter le calcul
    /// pseudo complet (`collect_candidates` + héritage) sur
    /// les feuilles qui n'en produisent aucun (R-83) : sans
    /// règle ciblée, `content` ne peut jamais être posé.
    fn has_pseudo_rules(&self, dom: &Dom, pe: crate::PseudoElement) -> bool {
        let (a, q) = match pe {
            crate::PseudoElement::Before => (self.index.has_before, self.quirks_index.has_before),
            crate::PseudoElement::After => (self.index.has_after, self.quirks_index.has_after),
        };
        a || (dom.document_mode() == explorer_dom::DocumentMode::Quirks && q)
    }

    /// R-96 — le sélecteur `sel` de la règle peut-il atteindre
    /// `node` ? Quand `rule.scope == node_scope`, toujours oui
    /// (même arbre). Sinon, deux franchissements seulement :
    ///   • sortant — `:host*` en composé sujet d'une feuille
    ///     scopée, seulement sur l'hôte de sa racine
    ///     (`.a:host` qualifie l'hôte) ;
    ///   • entrant — `::part` en composé sujet d'une feuille
    ///     externe : le sujet vit dans la racine shadow dont
    ///     l'HÔTE appartient à l'arbre de la feuille (un seul
    ///     boundary — `part` n'expose jamais au-delà).
    fn selector_reaches(
        dom: &Dom,
        rule: &CollectedRule,
        node: NodeId,
        node_scope: Option<NodeId>,
        sel_idx: usize,
    ) -> bool {
        if rule.scope == node_scope {
            return true;
        }
        let Some(sel) = rule.selectors.get(sel_idx) else {
            return false;
        };
        let Some((last, _)) = sel.parts.last() else {
            return false;
        };
        if last
            .simples
            .iter()
            .any(|s| matches!(s, SimpleSelector::Part(_)))
        {
            // Le nœud est dans `node_scope` ; sa racine a un
            // hôte — la feuille doit appartenir à l'arbre de
            // cet hôte.
            let host_scope = node_scope
                .and_then(|r| dom.shadow_host(r))
                .and_then(|h| dom.containing_shadow_root(h));
            return rule.scope == host_scope;
        }
        if rule.scope.and_then(|r| dom.shadow_host(r)) != Some(node) {
            return false;
        }
        last.simples.iter().any(|s| {
            matches!(
                s,
                SimpleSelector::Pseudo(PseudoClass::Host(_) | PseudoClass::HostContext(_))
            )
        })
    }

    /// Candidats de déclaration des **règles** triés (rang,
    /// spécificité, ordre croissants — la dernière écriture
    /// gagne) : un sélecteur doit matcher **avec le même
    /// pseudo-élément** cible. L'attribut `style` est fusionné
    /// par l'appelant (durée de vie locale).
    ///
    /// R-83 — seuls les sélecteurs dont la clé de composé
    /// final est portée par `node` sont essayés (index
    /// [`SelectorIndex`]) au lieu de balayer toute la table.
    /// La spécificité d'une règle = max de ses sélecteurs qui
    /// matchent (invariant de la cascade) ; l'ordre source
    /// départage déterministe après le tri — l'ordre de
    /// collecte par seaux n'a donc pas d'effet observable.
    #[allow(clippy::too_many_arguments)]
    fn collect_candidates<'a>(
        &'a self,
        dom: &Dom,
        node: NodeId,
        env: &StyleEnv,
        pseudo: Option<crate::PseudoElement>,
        state: &dyn Fn(&str) -> bool,
        mcache: &mut MatchCache,
        anc: &AncestorFilter,
    ) -> Vec<(u8, Vec<u32>, Specificity, u32, &'a Declaration)> {
        // Arbre d'appartenance du nœud (R-96) : `None` =
        // document. Arbitre le confinement des feuilles.
        let node_scope = dom.containing_shadow_root(node);
        // Spécificité max des sélecteurs qui matchent, par
        // règle (`true` = table quirks — mêmes rang UA et
        // ordre source que la table principale).
        let mut spec_by_rule: HashMap<(bool, u32), Specificity> = HashMap::new();
        let mut consider = |quirks: bool, e: &SelEntry| {
            if e.pseudo != pseudo {
                return;
            }
            let rules = if quirks {
                &self.quirks_rules
            } else {
                &self.rules
            };
            let Some(rule) = rules.get(e.rule as usize) else {
                return;
            };
            // R-96 — confinement : une feuille n'atteint que
            // son propre arbre (franchissements `:host*`/
            // `::part` arbitrés par `selector_reaches`).
            if !Self::selector_reaches(dom, rule, node, node_scope, e.selector as usize) {
                return;
            }
            // Chaque groupe `@media` englobant doit matcher.
            if !rule.media.is_empty() && !rule.media.iter().all(|qs| media_matches(env, qs)) {
                return;
            }
            let Some(sel) = rule.selectors.get(e.selector as usize) else {
                return;
            };
            let spec = sel.specificity();
            // Sélecteur dominé : la règle a déjà un max de
            // spécificité ≥ — matcher celui-ci ne peut rien
            // changer (R-83).
            if spec_by_rule
                .get(&(quirks, e.rule))
                .is_some_and(|m| spec <= *m)
            {
                return;
            }
            let hit = matches_with_state_cached(dom, node, sel, state, mcache, *anc, rule.scope);
            if hit {
                spec_by_rule
                    .entry((quirks, e.rule))
                    .and_modify(|s| *s = (*s).max(spec))
                    .or_insert(spec);
            }
        };
        self.index
            .candidates(dom, node, &mut |e| consider(false, e));
        // Règles UA quirks : la spec HTML Rendering ne les
        // applique qu'aux documents en mode quirks.
        if dom.document_mode() == explorer_dom::DocumentMode::Quirks {
            self.quirks_index
                .candidates(dom, node, &mut |e| consider(true, e));
        }
        let mut rules_cand: Vec<(u8, Vec<u32>, Specificity, u32, &Declaration)> = Vec::new();
        for ((quirks, ri), spec) in spec_by_rule {
            let rules = if quirks {
                &self.quirks_rules
            } else {
                &self.rules
            };
            let Some(rule) = rules.get(ri as usize) else {
                continue;
            };
            for d in &rule.declarations {
                let rank = match (rule.origin, d.important) {
                    (Origin::UserAgent, false) => 0,
                    (Origin::Author, false) => 1,
                    (Origin::Author, true) => 2,
                    (Origin::UserAgent, true) => 3,
                };
                rules_cand.push((
                    rank,
                    layer_sort_key(&rule.layer, d.important),
                    spec,
                    rule.order,
                    d,
                ));
            }
        }
        // Rang (origine/important) → layer (§6.4, inversion
        // `!important` comprise) → spécificité → ordre source.
        rules_cand
            .sort_by_key(|(rank, layer, spec, order, _)| (*rank, layer.clone(), *spec, *order));
        rules_cand
    }
}

/// Clé de tri `@layer` d'une déclaration (CSS Cascade 5 §6.4,
/// R-91) — insérée entre le rang d'origine et la spécificité :
/// une couche bat TOUTE spécificité hors de son rang.
///
/// - Normal : hors-layer (`u32::MAX`) bat tous les layers ;
///   entre layers, dernier déclaré gagne (rangs croissants,
///   lexicographique sur le chemin de sous-couches).
/// - `!important` : l'ordre se renverse — le PREMIER layer
///   déclaré gagne, hors-layer devient le plus faible (`0`).
///   Chaque segment est complémenté (`MAX - rang`) et une
///   borne `MAX` terminale fait passer les règles propres
///   d'une couche APRES ses sous-couches (miroir du tri
///   normal où un préfixe plus court trie avant).
fn layer_sort_key(layer: &[u32], important: bool) -> Vec<u32> {
    if layer.is_empty() {
        return if important { vec![0] } else { vec![u32::MAX] };
    }
    if important {
        let mut key: Vec<u32> = layer.iter().map(|s| u32::MAX - *s).collect();
        key.push(u32::MAX);
        key
    } else {
        layer.to_vec()
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
    env: &StyleEnv,
    rem_base: f32,
) {
    let Some(raw) = props.get("font-size").cloned() else {
        return;
    };
    let parent_fs = parent
        .and_then(|p| p.get("font-size"))
        .and_then(|v| v.trim().trim_end_matches("px").trim().parse::<f32>().ok())
        .unwrap_or(env.font_size_base);
    if let Some(px) = resolve_font_size(&raw, parent_fs, env, rem_base) {
        props.insert("font-size".to_string(), format!("{px}px"));
    }
}

/// Résout `font-size` en px absolus : `em`/`ex`/`ch`/`%` et
/// `larger`/`smaller` contre la taille **calculée** du parent
/// (`parent_fs`), `rem` contre la font-size **calculée de
/// l'élément racine** (`rem_base` — CSS Values §5.2 ; pour le
/// `font-size` de la racine elle-même l'appelant passe
/// `env.font_size_base`, la valeur initiale), mots-clés
/// absolus contre `env.font_size_base`. `calc()` et syntaxes
/// inconnues → `None` (la valeur brute survit pour le
/// layout — dégradation documentée, jamais de panique).
fn resolve_font_size(raw: &str, parent_fs: f32, env: &StyleEnv, rem_base: f32) -> Option<f32> {
    let v = raw.trim();
    let base = env.font_size_base;
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
        crate::value::Unit::Rem => len.value as f32 * rem_base,
        crate::value::Unit::Ex | crate::value::Unit::Ch => len.value as f32 * parent_fs * 0.5,
        // R-42 : `font-size:Nvw` légal — résolu au moment de
        // la cascade contre `env` (valeur calculée absolue).
        crate::value::Unit::Vw => (len.value * f64::from(env.width_px) / 100.0) as f32,
        crate::value::Unit::Vh => (len.value * f64::from(env.height_px) / 100.0) as f32,
        crate::value::Unit::Vmin => {
            (len.value * f64::from(env.width_px.min(env.height_px)) / 100.0) as f32
        }
        crate::value::Unit::Vmax => {
            (len.value * f64::from(env.width_px.max(env.height_px)) / 100.0) as f32
        }
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
        "textarea" => {
            // `cols`/`rows` → taille en caractères (HTML
            // Rendering) : `cols` → `width` en `ch`,
            // `rows` → `height` ≈ rows × hauteur de ligne.
            // Les défauts (10em/4em, ex-règles UA) sont
            // émis ici : une règle UA primant le hint, la
            // feuille ne déclare plus ces dimensions —
            // terrain : `<textarea rows=6 cols=60>` HN
            // rendu 10em×4em générique au lieu de
            // ~60ch×7.2em.
            hint(
                props,
                "width",
                chars(hint_attr(dom, node, "cols")).or_else(|| Some("10em".to_string())),
            );
            hint(
                props,
                "height",
                lines(hint_attr(dom, node, "rows")).or_else(|| Some("4em".to_string())),
            );
        }
        "input" => {
            // `size` → `width` en `ch` — ignoré sur les
            // états à contrôle propre (checkbox/radio/
            // submit : leur `width` UA est une vraie règle,
            // elle prime le hint sans filtrage ici).
            hint(
                props,
                "width",
                chars(hint_attr(dom, node, "size")).or_else(|| Some("10em".to_string())),
            );
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

/// `N` → "Nch" — attributs `cols`/`size` des champs de
/// saisie : largeur en caractères (HTML Rendering). Entier
/// ≥ 1 requis ; borne 1000 défensive comme `span`.
fn chars(a: Option<String>) -> Option<String> {
    a?.trim()
        .parse::<u32>()
        .ok()
        .filter(|n| *n >= 1)
        .map(|n| n.min(1000).to_string() + "ch")
}

/// `N` lignes → "N×1.2em" — `rows` de `textarea` : hauteur
/// = N × hauteur de ligne normale (approx 1.2em — le hint
/// reste rang UA, la CSS auteur prime).
fn lines(a: Option<String>) -> Option<String> {
    a?.trim()
        .parse::<u32>()
        .ok()
        .filter(|n| *n >= 1)
        .map(|n| format!("{:.1}em", f64::from(n.min(1000)) * 1.2))
}

/// `colspan`/`rowspan` (R-47) : entier ≥ 1 ; `0` et les
/// valeurs invalides → None (= 1 par défaut). Le plafond
/// 1000 (borne HTML table model) reste défensif — le vrai
/// clamp visuel s'applique dans `explorer-layout`.
fn span(a: Option<String>) -> Option<String> {
    a?.trim()
        .parse::<u32>()
        .ok()
        .filter(|n| *n >= 1)
        .map(|n| n.min(1000).to_string())
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
            // `colspan`/`rowspan` (R-47) — attributs du modèle
            // de grille HTML, pas des propriétés CSS : livrés
            // au layout table via des clés internes.
            hint(props, "-x-colspan", span(hint_attr(dom, node, "colspan")));
            hint(props, "-x-rowspan", span(hint_attr(dom, node, "rowspan")));
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
fn substitute_vars(
    text: &str,
    lookup: &dyn Fn(&str) -> Option<String>,
    depth: u32,
) -> Option<String> {
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
        let resolved = lookup(name.trim()).and_then(|v| substitute_vars(&v, lookup, depth + 1));
        // --x résolue (sa propre chaîne var() comprise) →
        // substitution ; invalide ou absente → fallback.
        if let Some(v) = resolved {
            out.push_str(&v);
        } else {
            let fb = fallback?;
            out.push_str(&substitute_vars(fb.trim(), lookup, depth + 1)?);
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
#[allow(clippy::too_many_arguments)]
fn parent_computed(
    engine: &StyleEngine,
    dom: &Dom,
    node: NodeId,
    env: &StyleEnv,
    state: &dyn Fn(&str) -> bool,
    memo: &mut HashMap<NodeId, ComputedStyle>,
    mcache: &mut MatchCache,
    root_fs: f32,
) -> Option<ComputedStyle> {
    let parent = match dom.parent(node) {
        Ok(Some(p)) if dom.element(p).is_ok() => p,
        _ => return None,
    };
    if let Some(s) = memo.get(&parent) {
        return Some(s.clone());
    }
    let grand = parent_computed(engine, dom, parent, env, state, memo, mcache, root_fs);
    let s = engine.compute_with_parent(
        dom,
        parent,
        env,
        grand.as_ref(),
        None,
        state,
        mcache,
        &AncestorFilter::of_ancestors(dom, parent),
        root_fs,
    );
    memo.insert(parent, s.clone());
    Some(s)
}

/// Propriétés héritées (spec CSS cascade / inherit list).
const INHERITED_PROPS: &[&str] = &[
    "border-collapse",
    "border-spacing",
    "caption-side",
    "color",
    "color-scheme",
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

/// La valeur (trimée) est-elle un mot-clé CSS-wide
/// (`inherit`/`initial`/`unset`/`revert`/`revert-layer`) ?
/// Comparaison insensible sans allocation — sert de
/// fast-path à la boucle mots-clés (R-83).
fn is_css_wide(t: &str) -> bool {
    t.eq_ignore_ascii_case("inherit")
        || t.eq_ignore_ascii_case("initial")
        || t.eq_ignore_ascii_case("unset")
        || t.eq_ignore_ascii_case("revert")
        || t.eq_ignore_ascii_case("revert-layer")
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
/// `noscript` est masqué **sous scripting** (R-93 — MQ5
/// `(scripting)`, HTML Rendering : son contenu ne s'affiche
/// que sans moteur de script). Sans scripting il reste un
/// vrai sous-arbre rendu — comportement historique.
/// `noframes`/`noembed` restent visibles pour la même
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
@media (scripting) { noscript { display: none } }
li { display: list-item }
ul, menu, dir { list-style-type: disc }
ul ul, ol ul, menu ul, dir ul { list-style-type: circle }
ul ul ul, ol ul ul, menu ul ul, dir ul ul { list-style-type: square }
ol { list-style-type: decimal }
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
  border: 1px solid #767676; padding: 1px
}
/* Les dimensions input/textarea vivent en hints
   (cols/rows/size — HTML Rendering) : une règle UA
   primerait les attributs de présentation. */
input { height: 1.2em }
select { border: 1px solid #767676; padding: 1px 4px }
button { border: 1px solid #767676; padding: 1px 6px; background-color: #eee }
input[type=submit], input[type=button], input[type=reset] {
  width: auto; padding: 1px 6px; background-color: #eee
}
input[type=checkbox], input[type=radio] { width: 13px; height: 13px }
input[type=hidden] { display: none }
";

/// Feuille UA **quirks mode** (HTML Standard § rendering —
/// « In quirks mode ») : la table réinitialise les propriétés
/// héritées de texte — un `<center>` ou une taille de fonte
/// autour d'une table ne déteint pas dans les cellules (web
/// d'avant-doctype type HN). Rang UA, ordre après `UA_CSS`.
const QUIRKS_UA_CSS: &str = "
table {
  font-weight: initial; font-style: initial; font-variant: initial;
  font-size: initial; line-height: initial; white-space: initial;
  text-align: initial
}
";

#[cfg(test)]
mod tests {
    //! R-57 — mécanisme interne : la table `--*` est partagée
    //! entre parent et enfants (`Arc`), pas copiée.
    use super::{StyleEngine, StyleEnv};
    use crate::Origin;
    use explorer_dom::Dom;
    use std::sync::Arc;

    /// Arbre `div > div > p` + CSS arbitraire → styles calculés.
    fn three_gen(
        css: &str,
    ) -> (
        Dom,
        explorer_dom::NodeId,
        explorer_dom::NodeId,
        explorer_dom::NodeId,
        crate::StyleMap,
    ) {
        let mut dom = Dom::new();
        let doc = dom.document();
        let a = dom.create_element("div").unwrap();
        dom.append_child(doc, a).unwrap();
        let b = dom.create_element("div").unwrap();
        dom.append_child(a, b).unwrap();
        let c = dom.create_element("p").unwrap();
        dom.append_child(b, c).unwrap();
        let mut e = StyleEngine::new();
        e.add_css(css, Origin::Author);
        let map = e.compute_tree(&dom, doc, &StyleEnv::default());
        (dom, a, b, c, map)
    }

    /// Enfant sans `--*` propre : même allocation que le parent
    /// (l'héritage ne duplique plus les ~200 entrées `:root`).
    /// NB : `div {…}` cible `a` ET `b` — chacun déclare donc ;
    /// le partage se mesure sur le `p` feuille.
    #[test]
    fn vars_partage_arc_sans_declaration() {
        let (_dom, _a, b, c, map) = three_gen("div { --a: 1px; --b: 2px }");
        let parent = map.get(b).unwrap();
        let child = map.get(c).unwrap();
        assert!(
            Arc::ptr_eq(&parent.vars, &child.vars),
            "table `--*` partagée entre générations"
        );
    }

    /// Enfant avec `--*` propre : clone-on-write — sa table
    /// diverge ; sans déclaration, le `p` partage celle de `b`.
    #[test]
    fn vars_clone_on_write_si_declaree() {
        let (_dom, a, b, c, map) = three_gen("div { --a: 1px } p { --c: 3px }");
        let sa = map.get(a).unwrap();
        let sb = map.get(b).unwrap();
        let sc = map.get(c).unwrap();
        // `b` (div) déclare `--a` → table dérivée de `a`.
        assert!(!Arc::ptr_eq(&sa.vars, &sb.vars));
        // `p` déclare aussi (`--c`) → dérivée de `b`.
        assert!(!Arc::ptr_eq(&sb.vars, &sc.vars));
        assert_eq!(sc.get("--a"), Some("1px"));
        assert_eq!(sc.get("--c"), Some("3px"));
    }

    /// Chaîne pure : deux générations sans `--*` → un seul
    /// bloc partagé de la racine à la feuille.
    #[test]
    fn vars_partage_sur_toute_la_chaine() {
        let (_dom, a, b, c, map) = three_gen(":root { --t: dark }");
        let _ = (b, c);
        // `:root` = document : tous les descendants partagent
        // la même allocation.
        let sa = map.get(a).unwrap();
        let sb = map.get(b).unwrap();
        let sc = map.get(c).unwrap();
        assert!(Arc::ptr_eq(&sa.vars, &sb.vars));
        assert!(Arc::ptr_eq(&sb.vars, &sc.vars));
    }
}
