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
    /// Rapport pixel écran (MQ `resolution`/`dppx` et les
    /// `-webkit-*-device-pixel-ratio`, R-174) — le
    /// `devicePixelRatio` du viewport ; 1.0 par défaut.
    pub dpr: f32,
    /// Réglage `prefers-reduced-motion` (MQ5, R-174) —
    /// utilisateur/OS ; `false` par défaut.
    pub reduced_motion: bool,
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
            dpr: 1.0,
            reduced_motion: false,
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

/// `calc()` dans une valeur de feature MQ (R-225) —
/// `calc(640px - 1px)` → 639. Grammaire bornée : `+−` au
/// sommet, `*/` prioritaires, parenthèses, feuilles
/// `<nombre><unité>` (px/em/rem/vw/vh/vmin/vmax comme
/// `px()` ci-dessus ; `%` → fraction de la base pertinente
/// non portée → `None`, approximation documentée — les
/// features de taille en % sont rares).
fn mq_calc_len(v: &str, env: &StyleEnv) -> Option<f64> {
    let inner = v
        .trim()
        .strip_prefix("calc(")
        .and_then(|s| s.strip_suffix(')'))?;
    // Tokens : parenthèses, ops, atomes (`640px`, `1.5`,
    // `-` n'est jamais signe — calc exige des espaces autour
    // des `+`/`-` (spéc) ; un `-` collé invalide l'atome.
    let mut toks: Vec<String> = Vec::new();
    let mut cur = String::new();
    for c in inner.trim().chars() {
        match c {
            ' ' | '\t' => {
                if !cur.is_empty() {
                    toks.push(std::mem::take(&mut cur));
                }
            }
            '(' | ')' | '+' | '-' | '*' | '/' => {
                if !cur.is_empty() {
                    toks.push(std::mem::take(&mut cur));
                }
                toks.push(c.to_string());
            }
            _ => cur.push(c),
        }
    }
    if !cur.is_empty() {
        toks.push(cur);
    }
    if toks.is_empty() {
        return None;
    }
    let mut i = 0;
    let v = mq_calc_expr(&toks, &mut i, env)?;
    (i == toks.len()).then_some(v)
}

/// Feuille `calc` MQ : `<nombre><unité>` → px (mêmes
/// unités que `px()` : em/rem → `font_size_base`,
/// vw/vh/vmin/vmax → viewport).
fn mq_calc_leaf(t: &str, env: &StyleEnv) -> Option<f64> {
    let (num, mul) = if let Some(s) = t.strip_suffix("rem") {
        (s, f64::from(env.font_size_base))
    } else if let Some(s) = t.strip_suffix("em") {
        (s, f64::from(env.font_size_base))
    } else if let Some(s) = t.strip_suffix("vmin") {
        (s, f64::from(env.width_px.min(env.height_px)) / 100.0)
    } else if let Some(s) = t.strip_suffix("vmax") {
        (s, f64::from(env.width_px.max(env.height_px)) / 100.0)
    } else if let Some(s) = t.strip_suffix("vw") {
        (s, f64::from(env.width_px) / 100.0)
    } else if let Some(s) = t.strip_suffix("vh") {
        (s, f64::from(env.height_px) / 100.0)
    } else if let Some(s) = t.strip_suffix("px") {
        (s, 1.0)
    } else {
        (t, 1.0)
    };
    num.trim().parse::<f64>().ok().map(|n| n * mul)
}

fn mq_calc_factor(t: &[String], i: &mut usize, env: &StyleEnv) -> Option<f64> {
    match t.get(*i)?.as_str() {
        "(" => {
            *i += 1;
            let v = mq_calc_expr(t, i, env)?;
            if t.get(*i)? != ")" {
                return None;
            }
            *i += 1;
            Some(v)
        }
        t => {
            *i += 1;
            mq_calc_leaf(t, env)
        }
    }
}

fn mq_calc_term(t: &[String], i: &mut usize, env: &StyleEnv) -> Option<f64> {
    let mut v = mq_calc_factor(t, i, env)?;
    while t.get(*i).is_some_and(|s| s == "*" || s == "/") {
        let op = t[*i].clone();
        *i += 1;
        let rhs = mq_calc_factor(t, i, env)?;
        v = if op == "*" { v * rhs } else { v / rhs };
    }
    Some(v)
}

fn mq_calc_expr(t: &[String], i: &mut usize, env: &StyleEnv) -> Option<f64> {
    let mut v = mq_calc_term(t, i, env)?;
    while t.get(*i).is_some_and(|s| s == "+" || s == "-") {
        let op = t[*i].clone();
        *i += 1;
        let rhs = mq_calc_term(t, i, env)?;
        v = if op == "+" { v + rhs } else { v - rhs };
    }
    Some(v)
}

fn query_matches(env: &StyleEnv, q: &MediaQuery) -> bool {
    // Forme non reconnue → vrai (tolérance documentée au parser).
    let mut ok = q.unknown;
    if !ok {
        ok = match &q.media_type {
            None => true,
            // `@supports` (encodé `media_type:"supports"`) :
            // chaque condition = une déclaration à évaluer —
            // `(prop:val)` supportée ? `not-<prop>` l'inverse.
            Some(t) if t == "supports" => q.conditions.iter().all(|(n, v)| decl_supported(n, v)),
            Some(t) => t == "all" || t == &env.media_type,
        };
        if ok && q.media_type.as_deref() != Some("supports") {
            let g = |conds: &[(String, String)]| {
                conds.iter().all(|(n, v)| condition_matches(env, n, v))
            };
            // `or` top-level (MQ4, R-220) : un seul groupe
            // suffit — le `media_type` s'applique à tous.
            ok = g(&q.conditions) || q.or_groups.iter().any(|grp| g(grp));
        }
    }
    if q.negated {
        !ok
    } else {
        ok
    }
}

/// `@supports (prop: value)` — la déclaration est-elle
/// supportée ? Critères : le nom est une propriété connue
/// du moteur (`SUPPORTED_PROPS`), un shorthand reconnu par
/// `expand_shorthand` (retour `Some`), ou une propriété
/// custom `--x` (toujours portées). `not-<prop>` inverse
/// (même convention que les features MQ niées). C'est le
/// test le plus honnête sans validateur par valeur : la
/// valeur n'est pas type-checkée — `@supports(display:foo)`
/// passe (approximation documentée : le nom seul tranche,
/// comme les vieux `document.querySelectorAll` gates).
fn decl_supported(name: &str, value: &str) -> bool {
    let (prop, neg) = name
        .strip_prefix("not-")
        .map_or((name, false), |p| (p, true));
    let prop = vendor_alias(prop);
    let supported = prop.starts_with("--")
        || crate::value::expand_shorthand(prop, value).is_some()
        || SUPPORTED_PROPS.contains(&prop);
    if neg {
        !supported
    } else {
        supported
    }
}

/// Alias vendor-prefix → nom canonique (census terrain P2 :
/// les misses réelles des sites SPA/média sont à ~55 % des
/// props `-webkit-*`/`-moz-*`/`-ms-*`/`-o-*`). Le nom
/// canonique est rendu quand la spec le standardise — les
/// props restées préfixées-de-fait (`-webkit-line-clamp`,
/// `-webkit-box-*` legacy, `-webkit-tap-highlight-color`)
/// gardent leur nom dans `SUPPORTED_PROPS` et ne passent
/// pas ici. `-ms-filter` est délibérément **non** mappé
/// (syntaxe IE pré-CSS3 incompatible).
#[allow(clippy::too_many_lines, clippy::match_same_arms)]
fn vendor_alias(name: &str) -> &str {
    match name {
        "-webkit-animation" | "-moz-animation" | "-o-animation" => "animation",
        "-webkit-animation-delay" | "-moz-animation-delay" | "-o-animation-delay" => {
            "animation-delay"
        }
        "-webkit-animation-direction" | "-moz-animation-direction" | "-o-animation-direction" => {
            "animation-direction"
        }
        "-webkit-animation-duration" | "-moz-animation-duration" | "-o-animation-duration" => {
            "animation-duration"
        }
        "-webkit-animation-fill-mode" | "-moz-animation-fill-mode" | "-o-animation-fill-mode" => {
            "animation-fill-mode"
        }
        "-webkit-animation-iteration-count"
        | "-moz-animation-iteration-count"
        | "-o-animation-iteration-count" => "animation-iteration-count",
        "-webkit-animation-name" | "-moz-animation-name" | "-o-animation-name" => "animation-name",
        "-webkit-animation-play-state"
        | "-moz-animation-play-state"
        | "-o-animation-play-state" => "animation-play-state",
        "-webkit-animation-timing-function"
        | "-moz-animation-timing-function"
        | "-o-animation-timing-function" => "animation-timing-function",
        "-webkit-appearance" | "-moz-appearance" => "appearance",
        "-webkit-backface-visibility" | "-moz-backface-visibility" => "backface-visibility",
        "-webkit-background-clip" | "-moz-background-clip" => "background-clip",
        "-webkit-background-origin" | "-moz-background-origin" => "background-origin",
        "-webkit-background-size" | "-moz-background-size" => "background-size",
        "-webkit-backdrop-filter" => "backdrop-filter",
        "-webkit-border-image" | "-moz-border-image" | "-o-border-image" => "border-image",
        "-webkit-box-decoration-break" => "box-decoration-break",
        "-webkit-box-shadow" | "-moz-box-shadow" => "box-shadow",
        "-webkit-box-sizing" | "-moz-box-sizing" => "box-sizing",
        "-webkit-clip-path" => "clip-path",
        "-webkit-column-count" | "-moz-column-count" => "column-count",
        "-webkit-column-gap" | "-moz-column-gap" => "column-gap",
        "-webkit-column-rule" | "-moz-column-rule" => "column-rule",
        "-webkit-columns" | "-moz-columns" => "columns",
        "-webkit-filter" => "filter",
        "-webkit-flex" | "-ms-flex" => "flex",
        "-webkit-flex-align" | "-ms-flex-align" => "align-items",
        "-webkit-flex-direction" | "-ms-flex-direction" => "flex-direction",
        "-webkit-flex-pack" | "-ms-flex-pack" => "justify-content",
        "-webkit-flex-wrap" | "-ms-flex-wrap" => "flex-wrap",
        "-webkit-font-feature-settings" | "-moz-font-feature-settings" => "font-feature-settings",
        "-webkit-font-kerning" => "font-kerning",
        "-webkit-font-smoothing" | "-moz-osx-font-smoothing" => "font-smoothing",
        "-webkit-hyphens" | "-moz-hyphens" | "-ms-hyphens" => "hyphens",
        "-webkit-justify-content" => "justify-content",
        "-webkit-mask" => "mask",
        "-webkit-mask-clip" => "mask-clip",
        "-webkit-mask-composite" => "mask-composite",
        "-webkit-mask-image" => "mask-image",
        "-webkit-mask-origin" => "mask-origin",
        "-webkit-mask-position" => "mask-position",
        "-webkit-mask-repeat" => "mask-repeat",
        "-webkit-mask-size" => "mask-size",
        "-webkit-overflow-scrolling" => "overflow-scrolling",
        "-webkit-perspective" | "-moz-perspective" => "perspective",
        "-webkit-perspective-origin" | "-moz-perspective-origin" => "perspective-origin",
        "-webkit-text-decoration" | "-moz-text-decoration" => "text-decoration",
        "-webkit-text-size-adjust" | "-moz-text-size-adjust" | "-ms-text-size-adjust" => {
            "text-size-adjust"
        }
        "-webkit-text-stroke-width" => "-webkit-text-stroke-width",
        "-webkit-text-stroke-color" => "-webkit-text-stroke-color",
        "-webkit-text-fill-color" => "-webkit-text-fill-color",
        "-webkit-touch-callout" => "-webkit-touch-callout",
        "-webkit-tap-highlight-color" => "-webkit-tap-highlight-color",
        "-webkit-transform" | "-moz-transform" | "-ms-transform" | "-o-transform" => "transform",
        "-webkit-transform-origin"
        | "-moz-transform-origin"
        | "-ms-transform-origin"
        | "-o-transform-origin" => "transform-origin",
        "-webkit-transform-style" | "-moz-transform-style" => "transform-style",
        "-webkit-transition" | "-moz-transition" | "-o-transition" => "transition",
        "-webkit-transition-delay" | "-moz-transition-delay" | "-o-transition-delay" => {
            "transition-delay"
        }
        "-webkit-transition-duration" | "-moz-transition-duration" | "-o-transition-duration" => {
            "transition-duration"
        }
        "-webkit-transition-property" | "-moz-transition-property" | "-o-transition-property" => {
            "transition-property"
        }
        "-webkit-transition-timing-function"
        | "-moz-transition-timing-function"
        | "-o-transition-timing-function" => "transition-timing-function",
        "-webkit-user-select" | "-moz-user-select" | "-ms-user-select" => "user-select",
        "-webkit-user-drag" => "-webkit-user-drag",
        "-webkit-word-break" | "-ms-word-break" => "word-break",
        "-moz-float-edge" => "-moz-float-edge",
        "-moz-force-broken-image-icon" => "-moz-force-broken-image-icon",
        "-moz-orient" => "-moz-orient",
        "-moz-user-input" => "-moz-user-input",
        "-ms-flex-negative" => "flex-shrink",
        "-ms-flex-positive" => "flex-grow",
        "-ms-flex-preferred-size" => "flex-basis",
        "-ms-touch-action" => "touch-action",
        "-ms-overflow-style" => "-ms-overflow-style",
        "-o-user-select" => "user-select",
        "-o-text-overflow" => "text-overflow",
        "-o-background-size" => "background-size",
        _ => name,
    }
}

/// Propriétés reconnues comme supportées dans `@supports` —
/// noms consommés par le moteur (hors shorthands, couverts
/// par `expand_shorthand`). Liste tenue à la main —
/// approximation conservatrice : un nom absent = `not
/// supported` → les `not (…)` de feature-detection restent
/// honnêtes (les auteurs écrivent `not` contre les trucs
/// récents qui sont ici absents de fait).
/// Propriétés « quirky length » (HTML Standard §quirks) :
/// en quirks mode, SEULES celles-ci acceptent un entier nu
/// comme `Npx` — liste fermée (un `grid-row:2` ne devient
/// jamais `2px`, R-182 suite).
const QUIRKY_LENGTH_PROPS: &[&str] = &[
    "block-size",
    "border-spacing",
    "border-top-width",
    "border-right-width",
    "border-bottom-width",
    "border-left-width",
    "bottom",
    "clip",
    "column-gap",
    "row-gap",
    "font-size",
    "height",
    "inline-size",
    "left",
    "letter-spacing",
    "margin-top",
    "margin-right",
    "margin-bottom",
    "margin-left",
    "max-block-size",
    "max-height",
    "max-inline-size",
    "max-width",
    "min-block-size",
    "min-height",
    "min-inline-size",
    "min-width",
    "padding-top",
    "padding-right",
    "padding-bottom",
    "padding-left",
    "right",
    "text-indent",
    "top",
    "width",
    "word-spacing",
];

const SUPPORTED_PROPS: &[&str] = &[
    "accent-color",
    "align-content",
    "align-items",
    "align-self",
    "animation",
    "animation-name",
    "animation-duration",
    "animation-timing-function",
    "animation-delay",
    "animation-iteration-count",
    "animation-direction",
    "animation-fill-mode",
    "animation-play-state",
    "appearance",
    "aspect-ratio",
    "backdrop-filter",
    "background",
    "background-attachment",
    "background-blend-mode",
    "background-clip",
    "background-color",
    "background-image",
    "background-origin",
    "background-position",
    "background-position-x",
    "background-position-y",
    "background-repeat",
    "background-size",
    "block-ellipsis",
    "border",
    "border-bottom",
    "border-bottom-color",
    "border-bottom-left-radius",
    "border-bottom-right-radius",
    "border-bottom-style",
    "border-bottom-width",
    "border-collapse",
    "border-color",
    "border-image",
    "border-left",
    "border-left-color",
    "border-left-style",
    "border-left-width",
    "border-radius",
    "border-right",
    "border-right-color",
    "border-right-style",
    "border-right-width",
    "border-spacing",
    "border-style",
    "border-top",
    "border-top-color",
    "border-top-left-radius",
    "border-top-right-radius",
    "border-top-style",
    "border-top-width",
    "border-width",
    "border-block-start",
    "border-block-end",
    "border-inline-start",
    "border-inline-end",
    "border-block-start-color",
    "border-block-start-style",
    "border-block-start-width",
    "border-block-end-color",
    "border-block-end-style",
    "border-block-end-width",
    "border-inline-start-color",
    "border-inline-start-style",
    "border-inline-start-width",
    "border-inline-end-color",
    "border-inline-end-style",
    "border-inline-end-width",
    "border-start-start-radius",
    "border-start-end-radius",
    "border-end-start-radius",
    "border-end-end-radius",
    "bottom",
    "box-shadow",
    "box-sizing",
    "caption-side",
    "caret-color",
    "accent-color",
    "overflow-anchor",
    "overscroll-behavior",
    "overscroll-behavior-x",
    "overscroll-behavior-y",
    "print-color-adjust",
    "forced-color-adjust",
    "field-sizing",
    "contain-intrinsic-size",
    "text-wrap",
    "text-wrap-mode",
    "text-wrap-style",
    "clear",
    "clip",
    "clip-path",
    "color",
    "-webkit-text-fill-color",
    "-webkit-text-stroke",
    "-webkit-text-stroke-color",
    "-webkit-text-stroke-width",
    "column-count",
    "column-gap",
    "contain",
    "container",
    "container-name",
    "container-type",
    "content",
    "counter-increment",
    "counter-reset",
    "cursor",
    "direction",
    "display",
    "empty-cells",
    "filter",
    "flex",
    "flex-basis",
    "flex-direction",
    "flex-flow",
    "flex-grow",
    "flex-shrink",
    "flex-wrap",
    "float",
    "font",
    "font-family",
    "font-size",
    "font-style",
    "font-variant",
    "font-variant-alternates",
    "font-variant-numeric",
    "font-feature-settings",
    "font-palette",
    "font-stretch",
    "font-size-adjust",
    "font-synthesis",
    "font-kerning",
    "font-weight",
    "gap",
    "grid",
    "grid-area",
    "grid-auto-columns",
    "grid-auto-flow",
    "grid-auto-rows",
    "grid-column",
    "grid-column-end",
    "grid-column-start",
    "grid-gap",
    "grid-row",
    "grid-row-end",
    "grid-row-start",
    "grid-template",
    "grid-template-areas",
    "grid-template-columns",
    "grid-template-rows",
    "height",
    "hyphens",
    "inset",
    "inset-block",
    "inset-block-end",
    "inset-block-start",
    "inset-inline",
    "inset-inline-end",
    "inset-inline-start",
    "isolation",
    "justify-content",
    "justify-items",
    "justify-self",
    "left",
    "letter-spacing",
    "line-clamp",
    "-webkit-box-orient",
    "-webkit-box-direction",
    "-webkit-box-pack",
    "-webkit-box-align",
    "-webkit-box-flex",
    "-webkit-box-ordinal-group",
    "-webkit-line-clamp",
    "line-height",
    "list-style",
    "list-style-image",
    "list-style-position",
    "list-style-type",
    "margin",
    "margin-block",
    "margin-block-end",
    "margin-block-start",
    "margin-bottom",
    "margin-inline",
    "margin-inline-end",
    "margin-inline-start",
    "margin-left",
    "margin-right",
    "margin-top",
    "mask",
    "mask-image",
    "max-height",
    "max-width",
    "min-height",
    "min-width",
    "mix-blend-mode",
    "object-fit",
    "object-position",
    "opacity",
    "order",
    "orphans",
    "outline",
    "outline-color",
    "outline-offset",
    "outline-style",
    "outline-width",
    "overflow",
    "overflow-wrap",
    "overflow-x",
    "overflow-y",
    "padding",
    "padding-block",
    "padding-block-end",
    "padding-block-start",
    "padding-bottom",
    "padding-inline",
    "padding-inline-end",
    "padding-inline-start",
    "padding-left",
    "padding-right",
    "padding-top",
    "place-content",
    "place-items",
    "place-self",
    "pointer-events",
    "position",
    "quotes",
    "resize",
    "right",
    "rotate",
    "row-gap",
    "scale",
    "scroll-behavior",
    "tab-size",
    "text-align",
    "text-align-last",
    "text-decoration",
    "text-decoration-color",
    "text-decoration-line",
    "text-decoration-style",
    "text-decoration-thickness",
    "text-emphasis",
    "text-emphasis-color",
    "text-emphasis-position",
    "text-emphasis-style",
    "text-spacing-trim",
    "hanging-punctuation",
    "text-indent",
    "text-justify",
    "text-combine-upright",
    "text-orientation",
    "text-overflow",
    "text-shadow",
    "text-transform",
    "text-underline-offset",
    "top",
    "transform",
    "transform-box",
    "transform-origin",
    "transform-style",
    "perspective",
    "perspective-origin",
    "backface-visibility",
    "transition",
    "transition-delay",
    "transition-duration",
    "transition-property",
    "transition-timing-function",
    "translate",
    "unicode-bidi",
    "user-select",
    "vertical-align",
    "visibility",
    "white-space",
    "widows",
    "width",
    "will-change",
    "word-break",
    "word-spacing",
    "word-wrap",
    "writing-mode",
    "z-index",
    // R-243 — props reconnues au plan BCD (`prop_supported`)
    // — dormant tant qu'aucun consommateur ne les lit ; les
    // familles consommables (`*size` logiques, `border-image-*`)
    // sont déjà routées par `logical_name`/`expand`.
    "-moz-float-edge",
    "-moz-force-broken-image-icon",
    "-moz-orient",
    "-moz-user-input",
    "-webkit-app-region",
    "-webkit-border-before",
    "-webkit-border-horizontal-spacing",
    "-webkit-border-vertical-spacing",
    "-webkit-box-reflect",
    "-webkit-column-axis",
    "-webkit-column-break-after",
    "-webkit-column-break-before",
    "-webkit-column-break-inside",
    "-webkit-column-progression",
    "-webkit-cursor-visibility",
    "-webkit-hyphenate-limit-after",
    "-webkit-hyphenate-limit-before",
    "-webkit-hyphenate-limit-lines",
    "-webkit-line-align",
    "-webkit-line-box-contain",
    "-webkit-line-grid",
    "-webkit-line-snap",
    "-webkit-locale",
    "-webkit-logical-height",
    "-webkit-logical-width",
    "-webkit-mask-box-image",
    "-webkit-mask-composite",
    "-webkit-mask-position-x",
    "-webkit-mask-position-y",
    "-webkit-mask-source-type",
    "-webkit-max-logical-height",
    "-webkit-max-logical-width",
    "-webkit-min-logical-height",
    "-webkit-min-logical-width",
    "-webkit-nbsp-mode",
    "-webkit-perspective-origin-x",
    "-webkit-perspective-origin-y",
    "-webkit-rtl-ordering",
    "-webkit-tap-highlight-color",
    "-webkit-text-combine",
    "-webkit-text-decorations-in-effect",
    "-webkit-text-security",
    "-webkit-text-zoom",
    "-webkit-touch-callout",
    "-webkit-transform-origin-x",
    "-webkit-transform-origin-y",
    "-webkit-transform-origin-z",
    "-webkit-user-drag",
    "alignment-baseline",
    "all",
    "alt",
    "anchor-name",
    "anchor-scope",
    "animation-composition",
    "animation-range",
    "animation-range-end",
    "animation-range-start",
    "animation-timeline",
    "animation-trigger",
    "background-repeat-x",
    "background-repeat-y",
    "baseline-shift",
    "baseline-source",
    "block-size",
    "border-image-outset",
    "border-image-repeat",
    "border-image-slice",
    "border-image-source",
    "border-image-width",
    "border-shape",
    "box-align",
    "box-decoration-break",
    "box-direction",
    "box-flex",
    "box-flex-group",
    "box-lines",
    "box-ordinal-group",
    "box-orient",
    "box-pack",
    "break-after",
    "break-before",
    "break-inside",
    "buffered-rendering",
    "caret-animation",
    "caret-shape",
    "clip-rule",
    "color-adjust",
    "color-interpolation",
    "color-interpolation-filters",
    "color-rendering",
    "color-scheme",
    "column-fill",
    "column-height",
    "column-rule-break",
    "column-rule-color",
    "column-rule-inset",
    "column-rule-inset-cap",
    "column-rule-inset-cap-end",
    "column-rule-inset-cap-start",
    "column-rule-inset-end",
    "column-rule-inset-junction",
    "column-rule-inset-junction-end",
    "column-rule-inset-junction-start",
    "column-rule-inset-start",
    "column-rule-style",
    "column-rule-visibility-items",
    "column-rule-width",
    "column-span",
    "column-width",
    "column-wrap",
    "contain-intrinsic-block-size",
    "contain-intrinsic-height",
    "contain-intrinsic-inline-size",
    "contain-intrinsic-size",
    "contain-intrinsic-width",
    "content-visibility",
    "continue",
    "corner-block-end-shape",
    "corner-block-start-shape",
    "corner-bottom-left-shape",
    "corner-bottom-right-shape",
    "corner-bottom-shape",
    "corner-end-end-shape",
    "corner-end-start-shape",
    "corner-inline-end-shape",
    "corner-inline-start-shape",
    "corner-left-shape",
    "corner-right-shape",
    "corner-shape",
    "corner-start-end-shape",
    "corner-start-start-shape",
    "corner-top-left-shape",
    "corner-top-right-shape",
    "corner-top-shape",
    "counter-set",
    "custom-property",
    "cx",
    "cy",
    "d",
    "dominant-baseline",
    "dynamic-range-limit",
    "field-sizing",
    "fill",
    "fill-opacity",
    "fill-rule",
    "fit-tolerance",
    "flex-line-count",
    "flood-color",
    "flood-opacity",
    "font-language-override",
    "font-optical-sizing",
    "font-palette",
    "font-smooth",
    "font-synthesis-position",
    "font-synthesis-small-caps",
    "font-synthesis-style",
    "font-synthesis-weight",
    "font-variant-caps",
    "font-variant-east-asian",
    "font-variant-emoji",
    "font-variant-ligatures",
    "font-variant-numeric",
    "font-variant-position",
    "font-variation-settings",
    "font-width",
    "forced-color-adjust",
    "frame-sizing",
    "glyph-orientation-vertical",
    "hyphenate-character",
    "hyphenate-limit-chars",
    "hyphenate-limit-lines",
    "image-orientation",
    "image-rendering",
    "ime-mode",
    "initial-letter",
    "inline-size",
    "interactivity",
    "interest-delay",
    "interest-delay-end",
    "interest-delay-start",
    "interpolate-size",
    "lighting-color",
    "line-break",
    "link-parameters",
    "margin-trim",
    "marker",
    "marker-end",
    "marker-mid",
    "marker-start",
    "mask-border",
    "mask-border-outset",
    "mask-border-repeat",
    "mask-border-slice",
    "mask-border-source",
    "mask-border-width",
    "mask-clip",
    "mask-composite",
    "mask-mode",
    "mask-origin",
    "mask-position",
    "mask-repeat",
    "mask-size",
    "mask-type",
    "math-depth",
    "math-shift",
    "math-style",
    "max-block-size",
    "max-inline-size",
    "max-lines",
    "max-size",
    "min-block-size",
    "min-inline-size",
    "min-size",
    "object-view-box",
    "offset",
    "offset-anchor",
    "offset-distance",
    "offset-path",
    "offset-position",
    "offset-rotate",
    "overflow-anchor",
    "overflow-block",
    "overflow-clip-margin",
    "overflow-inline",
    "overlay",
    "overscroll-behavior",
    "overscroll-behavior-block",
    "overscroll-behavior-inline",
    "overscroll-behavior-x",
    "overscroll-behavior-y",
    "page",
    "page-break-after",
    "page-break-before",
    "page-break-inside",
    "paint-order",
    "path-length",
    "position-anchor",
    "position-area",
    // `inset-area` = nom d'origine de `position-area`
    // (css-anchor-1, renommé — Chrome a shippe l'ancien).
    "inset-area",
    "position-try",
    "position-try-fallbacks",
    "position-try-order",
    "position-visibility",
    "print-color-adjust",
    "r",
    "reading-flow",
    "reading-order",
    "row-rule",
    "row-rule-break",
    "row-rule-color",
    "row-rule-inset",
    "row-rule-inset-cap",
    "row-rule-inset-cap-end",
    "row-rule-inset-cap-start",
    "row-rule-inset-end",
    "row-rule-inset-junction",
    "row-rule-inset-junction-end",
    "row-rule-inset-junction-start",
    "row-rule-inset-start",
    "row-rule-style",
    "row-rule-visibility-items",
    "row-rule-width",
    "ruby-align",
    "ruby-overhang",
    "ruby-position",
    "rule",
    "rule-break",
    "rule-color",
    "rule-inset",
    "rule-inset-cap",
    "rule-inset-end",
    "rule-inset-junction",
    "rule-inset-start",
    "rule-overlap",
    "rule-style",
    "rule-visibility-items",
    "rule-width",
    "rx",
    "ry",
    "scroll-axis-lock",
    "scroll-initial-target",
    "scroll-margin",
    "scroll-margin-block",
    "scroll-margin-block-end",
    "scroll-margin-block-start",
    "scroll-margin-bottom",
    "scroll-margin-inline",
    "scroll-margin-inline-end",
    "scroll-margin-inline-start",
    "scroll-margin-left",
    "scroll-margin-right",
    "scroll-margin-top",
    "scroll-marker-group",
    "scroll-padding",
    "scroll-padding-block",
    "scroll-padding-block-end",
    "scroll-padding-block-start",
    "scroll-padding-bottom",
    "scroll-padding-inline",
    "scroll-padding-inline-end",
    "scroll-padding-inline-start",
    "scroll-padding-left",
    "scroll-padding-right",
    "scroll-padding-top",
    "scroll-snap-align",
    "scroll-snap-stop",
    "scroll-snap-type",
    "scroll-target-group",
    "scroll-timeline",
    "scroll-timeline-axis",
    "scroll-timeline-name",
    "scrollbar-color",
    "scrollbar-gutter",
    "scrollbar-width",
    "shape-image-threshold",
    "shape-margin",
    "shape-outside",
    "shape-rendering",
    "size",
    "speak",
    "speak-as",
    "stop-color",
    "stop-opacity",
    "stroke",
    "stroke-color",
    "stroke-dasharray",
    "stroke-dashoffset",
    "stroke-linecap",
    "stroke-linejoin",
    "stroke-miterlimit",
    "stroke-opacity",
    "stroke-width",
    "table-layout",
    "text-anchor",
    "text-autospace",
    "text-box",
    "text-box-edge",
    "text-box-trim",
    "text-decoration-inset",
    "text-decoration-skip",
    "text-decoration-skip-ink",
    "text-decoration-skip-spaces",
    "text-fit",
    "text-rendering",
    "text-size-adjust",
    "text-underline-position",
    "text-wrap",
    "text-wrap-mode",
    "text-wrap-style",
    "timeline-scope",
    "timeline-trigger",
    "timeline-trigger-activation-range",
    "timeline-trigger-activation-range-end",
    "timeline-trigger-activation-range-start",
    "timeline-trigger-active-range",
    "timeline-trigger-active-range-end",
    "timeline-trigger-active-range-start",
    "timeline-trigger-name",
    "timeline-trigger-source",
    "touch-action",
    "transition-behavior",
    "trigger-scope",
    "user-modify",
    "vector-effect",
    "view-timeline",
    "view-timeline-axis",
    "view-timeline-inset",
    "view-timeline-name",
    "view-transition-class",
    "view-transition-group",
    "view-transition-name",
    "view-transition-scope",
    "white-space-collapse",
    "white-space-trim",
    "window-drag",
    "wrap-inside",
    "x",
    "y",
    "zoom",
];

/// Propriété `name` reconnue par le moteur — même critère que
/// `decl_supported` (nom seul : `SUPPORTED_PROPS` ou shorthand
/// expansible par [`crate::value::expand_shorthand`], la valeur
/// n'est pas type-checkée ; `--custom` exclus — elles sont
/// toujours portées mais ne disent rien du moteur).
///
/// Exposée pour le rapport de couverture
/// `scripts/bcd_coverage.mjs` (croisement `@mdn/browser-compat-
/// data` — « où en est le navigateur » mesuré en x/y).
#[must_use]
pub fn prop_supported(name: &str) -> bool {
    let name = vendor_alias(name);
    SUPPORTED_PROPS.contains(&name) || crate::value::expand_shorthand(name, "0").is_some()
}

/// R-245 — **routée** : la déclaration `x{name:inherit}`
/// atteint-elle le `ComputedStyle` (nom posé, ou shorthand
/// expansé en longhands) ? Le critère `prop_supported`
/// sature une fois la table comblée — cette sonde mesure
/// si la déclaration *survit* à la cascade au lieu de
/// répondre seulement « nom reconnu ».
/// L'appelant fournit le DOM (`<p class=x>` attendu) —
/// explorer-css ne peut pas construire de DOM (pas de dep
/// vers explorer-html). Critère honnête : `x{name:inherit}`
/// doit produire un `ComputedStyle` **différent** de `x{}`
/// — un nom stocké indifféremment (`xyz-bogus`) ne compte
/// pas. `all`/shorthands routent via leurs longhands.
#[must_use]
pub fn prop_routed(dom: &explorer_dom::Dom, name: &str) -> bool {
    let baseline = {
        let mut eng = StyleEngine::new();
        eng.add_css("p {}", Origin::Author);
        eng.compute_tree(dom, dom.document(), &StyleEnv::default())
    };
    let with = {
        let mut eng = StyleEngine::new();
        eng.add_css(&format!("p {{ {name}: inherit }}"), Origin::Author);
        eng.compute_tree(dom, dom.document(), &StyleEnv::default())
    };
    let (mut b, mut w) = (None, None);
    st_of_x(dom, dom.document(), &baseline, &mut b);
    st_of_x(dom, dom.document(), &with, &mut w);
    match (b, w) {
        (Some(b), Some(w)) => b != w,
        _ => false,
    }
}

/// R-245 — descend jusqu'au `class=x` et lit son style.
fn st_of_x<'a>(
    dom: &explorer_dom::Dom,
    n: explorer_dom::NodeId,
    sm: &'a StyleMap,
    out: &mut Option<&'a ComputedStyle>,
) {
    if dom
        .attribute_str(n, "class")
        .ok()
        .flatten()
        .is_some_and(|c| c == "x")
    {
        *out = sm.get(n);
    }
    if let Ok(ch) = dom.children(n) {
        for c in ch {
            st_of_x(dom, c, sm, out);
        }
    }
}

/// `(name: value)` d'une media query — px pour *-width/-height.
/// `em`/`rem` y sont résolus contre la font-size INITIALE
/// (spec MQ4 — jamais le `:root` calculé) : `env.font_size_base`.
#[allow(clippy::too_many_lines)]
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
        // R-225 — `calc()` dans les features de taille :
        // `@media (min-width:calc(640px - 1px))` wikipedia —
        // sans lui le bloc entier (`.mw-logo-icon{
        // display:none}` + `container{max-width:120px}`)
        // ne s'appliquait jamais → icône +118 px et menus
        // abs décalés en cascade.
        if v.starts_with("calc(") {
            return mq_calc_len(v, env);
        }
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
    // `Ndppx` = ratio direct, `Ndpi` = ÷96, `Ndpcm` = ×2,54/96 ;
    // nombre nu = dppx (R-174).
    let res = || -> Option<f64> {
        let v = value.trim();
        if let Some(s) = v.strip_suffix("dppx") {
            s.trim().parse().ok()
        } else if let Some(s) = v.strip_suffix("dpi") {
            s.trim().parse::<f64>().ok().map(|n| n / 96.0)
        } else if let Some(s) = v.strip_suffix("dpcm") {
            s.trim().parse::<f64>().ok().map(|n| n * 2.54 / 96.0)
        } else {
            v.parse().ok()
        }
    };
    // `W/H` ou nombre nu (R-174 — `aspect-ratio`).
    let ratio = || -> Option<f64> {
        let v = value.trim();
        if let Some((w, h)) = v.split_once('/') {
            let w = w.trim().parse::<f64>().ok()?;
            let h = h.trim().parse::<f64>().ok()?;
            (h != 0.0).then_some(w / h)
        } else {
            v.parse().ok()
        }
    };
    let aspect = f64::from(env.width_px) / f64::from(env.height_px.max(1));
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
        // ── R-174 : features MQ4/MQ5 manquantes — cap bureau
        // plein écran : pointeur fin + hover, rafraîchissement
        // rapide, gamut sRGB, 8 bits/couleur, mode navigateur.
        "hover" | "any-hover" => matches!(value.trim(), "" | "hover"),
        "pointer" | "any-pointer" => matches!(value.trim(), "" | "fine"),
        "prefers-reduced-motion" => match value.trim() {
            "" | "reduce" => env.reduced_motion,
            "no-preference" => !env.reduced_motion,
            _ => false,
        },
        "update" => matches!(value.trim(), "" | "fast"),
        "color-gamut" => value.trim() == "srgb",
        "forced-colors" => value.trim() == "none",
        "display-mode" => value.trim() == "browser",
        // 8 bits par composante couleur, 0 monochrome.
        "color" => match value.trim() {
            "" => true,
            v => v.parse::<u32>().is_ok_and(|n| (1..=8).contains(&n)),
        },
        "min-color" => match value.trim() {
            "" => true,
            v => v.parse::<u32>().is_ok_and(|n| n <= 8),
        },
        "max-color" | "max-monochrome" => true,
        "monochrome" | "min-monochrome" => match value.trim() {
            "" => false,
            v => v.parse::<u32>().is_ok_and(|n| n == 0),
        },
        // `resolution` + préfixés historiques — `dppx`
        // canonique, `dpi`/`dpcm` convertis.
        "resolution" | "-webkit-device-pixel-ratio" | "device-pixel-ratio" => {
            res().is_some_and(|v| (f64::from(env.dpr) - v).abs() < 1e-6)
        }
        "min-resolution" | "-webkit-min-device-pixel-ratio" | "min-device-pixel-ratio" => {
            res().is_some_and(|v| f64::from(env.dpr) >= v - 1e-6)
        }
        "max-resolution" | "-webkit-max-device-pixel-ratio" | "max-device-pixel-ratio" => {
            res().is_some_and(|v| f64::from(env.dpr) <= v + 1e-6)
        }
        "aspect-ratio" => ratio().is_some_and(|v| (aspect - v).abs() < 0.01),
        "min-aspect-ratio" => ratio().is_some_and(|v| aspect >= v - 1e-6),
        "max-aspect-ratio" => ratio().is_some_and(|v| aspect <= v + 1e-6),
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
    /// R-190 — index dans `container_queries` du prédicat
    /// `@container` englobant (`None` = règle ordinaire,
    /// toujours éligible). La règle n'entre en jeu qu'en
    /// seconde passe, quand sa query matche la border-box
    /// mesurée du conteneur ancêtre du nœud.
    container: Option<u32>,
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

/// Prédicat `@container` collecté (R-122/R-190 —
/// css-contain-3) : `name` + `query` bruts. Les règles du
/// bloc sont inlinées dans la table ordinaire, taguées
/// `CollectedRule::container` — la query est évaluée en
/// seconde passe contre la border-box **mesurée** du
/// conteneur ancêtre (inconnue à la cascade : elle
/// n'existe qu'après layout).
#[derive(Debug, Clone)]
pub struct CollectedContainer {
    /// Nom de conteneur ciblé (`container-name`), si donné.
    pub name: Option<String>,
    /// Condition brute — `(min-width: 400px) and (…)` (parens).
    pub query: String,
}

/// Mesure d'un conteneur `@container` (R-190) — produite
/// après layout par `explorer_layout::measure_containers` :
/// border-box posée + noms `container-name` déclarés.
#[derive(Debug, Clone)]
pub struct ContainerMeasure {
    /// Largeur border-box mesurée (px CSS).
    pub w: f32,
    /// Hauteur border-box mesurée (px CSS).
    pub h: f32,
    /// Noms `container-name` de la boîte (vide = anonyme).
    pub names: Vec<String>,
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
#[allow(clippy::struct_excessive_bools)]
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
    /// Voir `has_before` — `::first-line` (R-177).
    has_first_line: bool,
    /// Voir `has_before` — `::first-letter`.
    has_first_letter: bool,
    /// Voir `has_before` — `::marker`.
    has_marker: bool,
    /// Voir `has_before` — `::selection`.
    has_selection: bool,
    /// Voir `has_before` — `::backdrop`.
    has_backdrop: bool,
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
                    Some(crate::PseudoElement::FirstLine) => idx.has_first_line = true,
                    Some(crate::PseudoElement::FirstLetter) => idx.has_first_letter = true,
                    Some(crate::PseudoElement::Marker) => idx.has_marker = true,
                    Some(crate::PseudoElement::Selection) => idx.has_selection = true,
                    Some(crate::PseudoElement::Backdrop) => idx.has_backdrop = true,
                    Some(crate::PseudoElement::Other) | None => {}
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
            // R-182 — quirks : l'index reste exact, le lookup
            // gagne un balayage ASCII-insensible (HTML
            // Standard §quirks — `.Foo` matche class="foo").
            let quirks = matches!(dom.document_mode(), explorer_dom::DocumentMode::Quirks);
            for a in attrs {
                if a.name == "class" {
                    for c in a.value.split_ascii_whitespace() {
                        if let Some(v) = self.by_class.get(c) {
                            for e in v {
                                out(e);
                            }
                        }
                        if quirks {
                            for (k, v) in &self.by_class {
                                if k.as_str() != c && k.eq_ignore_ascii_case(c) {
                                    for e in v {
                                        out(e);
                                    }
                                }
                            }
                        }
                    }
                } else if a.name == "id" {
                    if let Some(v) = self.by_id.get(a.value.as_str()) {
                        for e in v {
                            out(e);
                        }
                    }
                    if quirks {
                        for (k, v) in &self.by_id {
                            if k.as_str() != a.value && k.eq_ignore_ascii_case(&a.value) {
                                for e in v {
                                    out(e);
                                }
                            }
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
            // le nœud → aucune clé utilisable (R-96). Idem
            // `E::slotted(c)` : `E` qualifie le SLOT (R-97).
            SimpleSelector::Part(_) | SimpleSelector::Slotted(_) => {
                return BucketKey::Universal;
            }
            SimpleSelector::Id(i) => return BucketKey::Id(i.clone()),
            SimpleSelector::Class(c) => {
                class.get_or_insert(c);
            }
            SimpleSelector::Attribute { name, .. } => {
                attr.get_or_insert(name);
            }
            SimpleSelector::Type(t) => {
                // `ns|tag` indexé sous `tag` nu — le filtrage
                // ns reste côté `simple_matches` (R-237).
                tag.get_or_insert(t.rsplit('|').next().unwrap_or(t.as_str()));
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
/// Enregistrement `@property` (css-properties-values-api-1 —
/// R-118) : `syntax` conservée (non typée v1), `inherits`
/// et `initial-value` appliqués à la valeur calculée.
#[derive(Debug, Clone)]
pub(crate) struct RegisteredProp {
    /// `syntax` de la registration (`"*"` = tout) — stockée
    /// pour le typage futur, jamais lue en v1.
    #[allow(dead_code)]
    pub syntax: String,
    /// `inherits` (`true` par défaut spec).
    pub inherits: bool,
    /// `initial-value` (`None` → guaranteed-invalid à l'usage).
    pub initial: Option<String>,
}

/// R-239 — une palette nommée `@font-palette-values` :
/// `font-family`, `base-palette`, `override-colors`
/// (index→couleur) — consommée par `font-palette: --nom`.
#[derive(Debug, Clone, Default)]
pub struct FontPalette {
    /// Famille de police à laquelle la palette s'applique.
    pub font_family: String,
    /// `base-palette` (numéro ou mot-clé).
    pub base_palette: String,
    /// `override-colors` : index → valeur couleur.
    pub override_colors: Vec<(u32, String)>,
}

/// R-229 — une valeur nommée de `@font-feature-values` :
/// tag OpenType 4cc + indices — résolution
/// `font-variant-alternates` (annotation→`nalt`,
/// character-variant→`cvNN`, styleset→`ssNN`,
/// stylistic→`salt`, swash→`swsh`).
#[derive(Debug, Clone)]
pub struct FontFeatureValues {
    /// Réglages résolus `(tag, valeur)` — un indice de
    /// `styleset`/`character-variant` code le tag lui-même
    /// (`ssNN`/`cvNN`) alors qu'`annotation`/`stylistic`/
    /// `swash` l'emportent comme valeur (`"nalt" 1`).
    pub settings: Vec<(String, u32)>,
}

/// `@counter-style <nom>` — style de compteur nommé
/// (css-counter-styles-3). R-221 — champs minimaux :
/// `system`/`symbols`/`prefix`/`suffix`/`fallback` ;
/// `pad`/`range`/`negative`/`extends` non portés (borne
/// documentée — `fallback` vaut).
#[derive(Debug, Clone, Default)]
pub struct CounterStyle {
    /// `system` tel que déclaré (`symbolic`, `numeric`,
    /// `alphabetic`, `cyclic`, `fixed`…). Vide = `symbolic`.
    pub system: String,
    /// `symbols` — la liste de glyphes (chaînés).
    pub symbols: Vec<String>,
    /// `prefix` — texte avant le label (vide par défaut).
    pub prefix: String,
    /// `suffix` — texte après le label (spec : ". ").
    pub suffix: String,
    /// `fallback` — style de repli quand `n` sort du
    /// système (défaut `decimal`).
    pub fallback: String,
}

impl CounterStyle {
    /// Label du marqueur pour l'ordinal `n` (1-based) —
    /// `prefix + corps + suffix`. Systèmes non portés
    /// (`additive`, `extends`, `fixed` hors portée) →
    /// repli `decimal` via le corps numérique.
    #[must_use]
    pub fn label(&self, n: i32) -> String {
        // `n` est un ordinal : négatif → 1 pour le cycle
        // (arithmétique usize, aucun cast flottant).
        let ni = usize::try_from(n.max(1)).unwrap_or(1);
        let sym = |i: usize| self.symbols.get(i).cloned().unwrap_or_default();
        let body = match self.system.as_str() {
            "numeric" => {
                if self.symbols.len() >= 10 {
                    n.to_string()
                        .chars()
                        .filter_map(|c| c.to_digit(10).map(|d| sym(d as usize)))
                        .collect::<String>()
                } else {
                    n.to_string()
                }
            }
            "alphabetic" => {
                if self.symbols.is_empty() {
                    n.to_string()
                } else {
                    let base = self.symbols.len();
                    let mut v = ni;
                    let mut s = Vec::new();
                    while v > 0 {
                        v -= 1;
                        s.push(sym(v % base));
                        v /= base;
                    }
                    s.into_iter().rev().collect()
                }
            }
            "fixed" => {
                if ni <= self.symbols.len() {
                    sym(ni - 1)
                } else {
                    n.to_string()
                }
            }
            _ => {
                // `symbolic`/`cyclic` (défaut) : cycle sur
                // les symboles — sinon repli numérique pur.
                if self.symbols.is_empty() {
                    n.to_string()
                } else {
                    sym((ni - 1) % self.symbols.len())
                }
            }
        };
        let suffix = if self.suffix.is_empty() {
            "."
        } else {
            &self.suffix
        };
        format!("{}{}{}", self.prefix, body, suffix)
    }
}

#[derive(Default)]
pub struct StyleEngine {
    /// `@counter-style` enregistrés (R-221).
    pub counter_styles: HashMap<String, CounterStyle>,
    /// `@font-feature-values` enregistrés (R-229) —
    /// famille → nom → tag+indices (résolution
    /// `font-variant-alternates`).
    pub font_feature_values: HashMap<String, HashMap<String, FontFeatureValues>>,
    /// `@font-palette-values` enregistrés (R-239) —
    /// `--nom` → palette (consommée par `font-palette`).
    pub font_palettes: HashMap<String, FontPalette>,
    /// `@custom-media <--nom> (query)` enregistrés (R-231) —
    /// alias de media query : `@media (--nom)` → substitution
    /// du query enregistré à la collecte.
    pub custom_media: HashMap<String, String>,
    /// `@position-try --nom { … }` enregistrés (css-anchor-1) —
    /// appliqués par la passe de flip quand la boîte déborde.
    pub position_tries: HashMap<String, Vec<Declaration>>,
    /// `@namespace` enregistrés (R-237) — préfixe → URI,
    /// transportés au `MatchCache` à chaque `compute_*` pour
    /// résoudre les sélecteurs `ns|tag`.
    pub namespaces: HashMap<String, String>,
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
    /// Registre `@property` (css-properties-values-api-1 —
    /// R-118) : `--name` → `{syntax, inherits, initial-value}`.
    /// La `syntax` est conservée mais non typée à la
    /// vérification (v1) — `inherits`/`initial-value` agissent
    /// à la valeur calculée.
    pub(crate) property_registry: HashMap<String, RegisteredProp>,
    /// Queries `@container` collectées (R-122 — css-contain-3) :
    /// évaluées en seconde passe par l'appelant (la taille du
    /// conteneur n'existe qu'après layout).
    pub container_queries: Vec<CollectedContainer>,
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
        let raw = self.get(name)?;
        let out = parse_length(raw);
        census_unresolved(name, raw, out.is_none());
        out
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
        let out = parse_color(raw);
        if out.is_none() {
            // R-209 — une couleur qui ne s'analyse pas est
            // toujours un manque réel (pas de mot-clé légal
            // non couleur dans ce slot).
            explorer_common::census::bump(&format!("css.value.unresolved.{name}"));
        }
        out
    }

    /// Valeur typée `f64` (`z-index`, `opacity`…).
    #[must_use]
    pub fn number(&self, name: &str) -> Option<f64> {
        let raw = self.get(name)?;
        let out = raw.trim().parse().ok();
        census_unresolved(name, raw, out.is_none());
        out
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
    /// `::first-line` (R-177) — style de la première ligne
    /// formatée (sous-ensemble typographique §7.1).
    pub first_line: Option<ComputedStyle>,
    /// `::first-letter` — première lettre (drop caps).
    pub first_letter: Option<ComputedStyle>,
    /// `::marker` — marqueur de liste.
    pub marker: Option<ComputedStyle>,
    /// `::selection` — routé, consommateur peinture en attente.
    pub selection: Option<ComputedStyle>,
    /// `::backdrop` — routé, consommateur `<dialog>`/fullscreen
    /// en attente.
    pub backdrop: Option<ComputedStyle>,
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
    /// `@counter-style` enregistrés — transportés au
    /// consommateur (`build_box_tree` lit `list-style-type`
    /// nommé). R-221.
    pub counter_styles: HashMap<String, CounterStyle>,
    /// `@font-feature-values` enregistrés (R-229).
    pub font_feature_values: HashMap<String, HashMap<String, FontFeatureValues>>,
    /// `@font-palette-values` enregistrés (R-239).
    pub font_palettes: HashMap<String, FontPalette>,
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
            crate::PseudoElement::FirstLine => p.first_line.as_ref(),
            crate::PseudoElement::FirstLetter => p.first_letter.as_ref(),
            crate::PseudoElement::Marker => p.marker.as_ref(),
            crate::PseudoElement::Selection => p.selection.as_ref(),
            crate::PseudoElement::Backdrop => p.backdrop.as_ref(),
            crate::PseudoElement::Other => None,
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
    /// `@property --name { syntax; inherits; initial-value }`
    /// (R-118) — enregistre dans [`Self::property_registry`].
    /// `syntax` stockée non typée v1 ; `initial-value` absent
    /// → guaranteed-invalid à l'usage (registration invalide
    /// spec quand `syntax` ≠ `*`).
    fn register_property(&mut self, prelude: &str, declarations: &[Declaration]) {
        let name = prelude.trim();
        if !name.starts_with("--") {
            return;
        }
        let (mut syntax, mut inherits, mut initial) = ("*".to_string(), true, None);
        for d in declarations {
            let v = d.value.trim().to_string();
            match d.name.trim() {
                "syntax" => syntax = v.trim_matches('"').trim_matches('\'').into(),
                "inherits" => inherits = v == "true",
                "initial-value" => initial = Some(v),
                _ => {}
            }
        }
        self.property_registry.insert(
            name.to_string(),
            RegisteredProp {
                syntax,
                inherits,
                initial,
            },
        );
    }

    /// Moteur préchargé avec la feuille UA intégrée (règles
    /// standards + règles conditionnelles quirks).
    #[must_use]
    pub fn new() -> Self {
        let mut e = Self::default();
        e.add_css(UA_CSS, Origin::UserAgent);
        let quirks_sheet = crate::parser::parse_stylesheet(QUIRKS_UA_CSS);
        e.collect_rules_into(
            &quirks_sheet.rules,
            Origin::UserAgent,
            &[],
            &[],
            true,
            None,
            None,
        );
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
        self.collect_rules_into(rules, origin, media, &[], false, None, None);
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
        self.collect_rules_into(rules, origin, media, &[], false, Some(scope), None);
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
    /// propriétaire (R-96, `None` = document). `container` =
    /// index du prédicat `@container` englobant (R-190).
    #[allow(clippy::too_many_arguments)]
    #[allow(clippy::too_many_lines)]
    fn collect_rules_into(
        &mut self,
        rules: &[Rule],
        origin: Origin,
        media: &[Vec<MediaQuery>],
        layer: &[String],
        quirks: bool,
        scope: Option<NodeId>,
        container: Option<u32>,
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
                            container,
                        });
                        self.next_order = self.next_order.saturating_add(1);
                        // R-207 — census des propriétés
                        // déclarées par l'auteur (la feuille
                        // UA intégrée est connue : elle
                        // ferait du bruit dans le Pareto).
                        if origin == Origin::Author {
                            for d in &q.declarations {
                                census_prop(&d.name);
                            }
                        }
                    } else {
                        // R-207 — prélude non analysable : la
                        // règle entière est droppée — le prélude
                        // (tronqué par le census) nomme la
                        // construction fautive (`:has(…)`…).
                        explorer_common::census::bump(&format!(
                            "css.selector.bad.{}",
                            q.prelude.trim()
                        ));
                    }
                }
                Rule::At(at) => {
                    if let AtKind::Media { queries, rules } = &at.kind {
                        // R-231 — `@custom-media` : un groupe
                        // `(--nom)` enregistré se substitue à
                        // son query (un niveau de récursion).
                        let queries = self.expand_custom_media(queries);
                        // `@media` imbriqué = AND : on empile la
                        // liste de queries du niveau.
                        let mut nested = media.to_vec();
                        nested.push(queries);
                        self.collect_rules_into(
                            rules, origin, &nested, layer, quirks, scope, container,
                        );
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
                        self.collect_rules_into(
                            rules, origin, media, &path, quirks, scope, container,
                        );
                    } else if let AtKind::Statement { prelude } = &at.kind {
                        // `@layer a, b;` : déclare l'ordre sans
                        // règles (les corps s'y raccordent par
                        // nom — premier arrivé fixe le rang).
                        if at.name == "namespace" {
                            // R-237 — `@namespace [préfixe]
                            // "uri"` : l'absence de préfixe
                            // déclare le défaut (clé vide).
                            let toks: Vec<&str> = prelude
                                .split_whitespace()
                                .map(|t| t.trim_matches('"').trim_matches('\''))
                                .filter(|t| !t.is_empty())
                                .collect();
                            match toks.as_slice() {
                                [uri] => {
                                    self.namespaces.insert(String::new(), (*uri).to_string());
                                }
                                [pfx, uri] => {
                                    self.namespaces
                                        .insert((*pfx).to_string(), (*uri).to_string());
                                }
                                _ => {}
                            }
                        } else if at.name == "custom-media" {
                            // R-231 — `@custom-media --nom (q)` :
                            // l'alias vit dans `custom_media` —
                            // `@media (--nom)` substitué à la
                            // collecte ci-dessous.
                            if let Some((name, q)) = prelude.split_once(char::is_whitespace) {
                                self.custom_media
                                    .insert(name.trim().to_string(), q.trim().to_string());
                            }
                        } else if at.name == "layer" {
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
                        } else {
                            // R-207 — statement sans
                            // consommateur (`@namespace`,
                            // `@tailwind …;`, `@charset`
                            // hors-tête…).
                            explorer_common::census::bump(&format!("css.at.unapplied.{}", at.name));
                        }
                    } else if let AtKind::Declarations {
                        prelude,
                        declarations,
                    } = &at.kind
                    {
                        if at.name.eq_ignore_ascii_case("font-face") {
                            self.font_faces.push(CollectedFontFace {
                                media: media.to_vec(),
                                declarations: declarations.clone(),
                            });
                        } else if at.name.eq_ignore_ascii_case("property") {
                            self.register_property(prelude, declarations);
                        } else if at.name.eq_ignore_ascii_case("counter-style") {
                            // R-221 — `@counter-style <nom>`
                            // enregistré : `list-style-type:
                            // <nom>` résout ensuite dans le
                            // layout (StyleMap.counter_styles).
                            let mut cs = CounterStyle {
                                suffix: ".".to_string(),
                                ..CounterStyle::default()
                            };
                            for d in declarations {
                                match d.name.as_str() {
                                    "system" => cs.system = d.value.trim().to_string(),
                                    "symbols" => {
                                        cs.symbols = d
                                            .value
                                            .split_whitespace()
                                            .map(|t| {
                                                t.trim_matches('"').trim_matches('\'').to_string()
                                            })
                                            .collect();
                                    }
                                    "prefix" => {
                                        cs.prefix = d.value.trim().trim_matches('"').to_string();
                                    }
                                    "suffix" => {
                                        cs.suffix = d.value.trim().trim_matches('"').to_string();
                                    }
                                    "fallback" => cs.fallback = d.value.trim().to_string(),
                                    _ => {}
                                }
                            }
                            self.counter_styles
                                .insert(prelude.trim().to_ascii_lowercase(), cs);
                        } else if at.name.eq_ignore_ascii_case("position-try") {
                            // `@position-try --nom { décls }` —
                            // registre nommé : `position-try:
                            // --nom` référence la règle.
                            self.position_tries
                                .insert(prelude.trim().to_string(), declarations.clone());
                        } else if at.name.eq_ignore_ascii_case("font-palette-values") {
                            // R-239 — `@font-palette-values --nom
                            // { font-family; base-palette;
                            // override-colors }` — registre par nom.
                            let mut fp = FontPalette::default();
                            for d in declarations {
                                match d.name.as_str() {
                                    "font-family" => {
                                        fp.font_family =
                                            d.value.trim().trim_matches('"').to_string();
                                    }
                                    "base-palette" => {
                                        fp.base_palette = d.value.trim().to_string();
                                    }
                                    "override-colors" => {
                                        for pair in d.value.split(',') {
                                            let mut it = pair.trim().splitn(2, char::is_whitespace);
                                            if let (Some(i), Some(c)) = (it.next(), it.next()) {
                                                if let Ok(idx) = i.parse::<u32>() {
                                                    fp.override_colors
                                                        .push((idx, c.trim().to_string()));
                                                }
                                            }
                                        }
                                    }
                                    _ => {}
                                }
                            }
                            self.font_palettes
                                .insert(prelude.trim().to_ascii_lowercase(), fp);
                        } else {
                            // R-207 — bloc de déclarations parsé
                            // mais inerte (`@page`, `@viewport`,
                            // `@font-feature-values`…).
                            explorer_common::census::bump(&format!("css.at.unapplied.{}", at.name));
                        }
                    } else if let AtKind::FontFeatureValues { family, features } = &at.kind {
                        // R-229 — registre : la famille
                        // sert de clé à la résolution
                        // `font-variant-alternates`.
                        let fam = family.trim().trim_matches('"').to_ascii_lowercase();
                        let set = self.font_feature_values.entry(fam).or_default();
                        for (kind, name, idxs) in features {
                            // css-fonts-4 §6.2 — `ssNN`/`cvNN` :
                            // le tag code l'indice (valeur 1) ;
                            // `nalt`/`swsh`/`salt` : l'indice est
                            // la valeur.
                            let settings: Vec<(String, u32)> = match kind.as_str() {
                                "styleset" => {
                                    idxs.iter().map(|i| (format!("ss{i:02}"), 1)).collect()
                                }
                                "character-variant" => {
                                    idxs.iter().map(|i| (format!("cv{i:02}"), 1)).collect()
                                }
                                "annotation" => {
                                    idxs.iter().map(|i| ("nalt".to_string(), *i)).collect()
                                }
                                "swash" => idxs.iter().map(|i| ("swsh".to_string(), *i)).collect(),
                                _ => idxs.iter().map(|i| ("salt".to_string(), *i)).collect(),
                            };
                            set.insert(name.to_ascii_lowercase(), FontFeatureValues { settings });
                        }
                    } else if let AtKind::Container { name, query, rules } = &at.kind {
                        // `@container` (R-190) : le prédicat
                        // rejoint `container_queries` (évalué
                        // en passe 2 sur la border-box mesurée
                        // — inconnue à la cascade) ; les règles
                        // internes sont inlinées avec le tag —
                        // `order`/`media`/`layer`/`scope`
                        // composés par la collecte ordinaire.
                        let idx = u32::try_from(self.container_queries.len()).unwrap_or(u32::MAX);
                        self.container_queries.push(CollectedContainer {
                            name: name.clone(),
                            query: query.clone(),
                        });
                        self.collect_rules_into(
                            rules,
                            origin,
                            media,
                            layer,
                            quirks,
                            scope,
                            Some(idx),
                        );
                    } else if let AtKind::Scope { rules } = &at.kind {
                        // `@scope` (R-194) : préludes déjà
                        // désucrés au parse — groupe ordinaire.
                        self.collect_rules_into(
                            rules, origin, media, layer, quirks, scope, container,
                        );
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
                    } else {
                        // R-207 — at-rule structurée mais sans
                        // consommateur (`AtKind::UnknownBlock`
                        // et variantes futures) — le manque
                        // techno nommé.
                        explorer_common::census::bump(&format!("css.at.unapplied.{}", at.name));
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
        self.compute_with_ctx(dom, node, env, state, None)
    }

    /// Variante contextuelle (R-190 — passe 2 `@container`) :
    /// `containers` = mesures des boîtes `container-type` après
    /// un premier layout (`explorer_layout::measure_containers`).
    /// `None` = première passe — les règles taguées restent
    /// inertes (comportement strictement identique à
    /// [`StyleEngine::compute_with_state`]).
    #[must_use]
    pub fn compute_with_ctx(
        &self,
        dom: &Dom,
        node: NodeId,
        env: &StyleEnv,
        state: &dyn Fn(&str) -> bool,
        containers: Option<&HashMap<NodeId, ContainerMeasure>>,
    ) -> ComputedStyle {
        let mut mcache = MatchCache::default();
        mcache.namespaces.clone_from(&self.namespaces);
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
            containers,
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
            containers,
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
        mcache.namespaces.clone_from(&self.namespaces);
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
            // Pas de ctx ici : la base `rem` sert à l'évaluation
            // même des règles taguées — un `@container` sur
            // `:root` n'a de toute façon pas d'ancêtre mesuré.
            None,
        )
        .get("font-size")
        .and_then(|v| v.trim().trim_end_matches("px").trim().parse::<f32>().ok())
        .filter(|fs| *fs > 0.0)
        .unwrap_or(env.font_size_base)
    }

    /// Évalue une condition `@container` contre la taille
    /// `(w, h)` d'un conteneur (R-122/R-190 — css-contain-3
    /// §4) : `(min-?width|max-?width|min-?height|max-?height:
    /// Npx)`, forme de plage MQ5 (`(width >= 400px)`,
    /// `(400px <= width <= 800px)`), clauses `and` jointes,
    /// `or` disjointes, `not` préfixe nié par branche.
    /// Feature inconnue ou valeur non résoluble (unité
    /// relative, `orientation`, `style()`) → clause FAUSSE
    /// (fail-closed : une règle non évaluable ne s'applique
    /// jamais — dégradation vers « pas de style », jamais
    /// vers un style faux).
    /// Consommé en seconde passe par l'appelant (la taille
    /// du conteneur n'existe qu'après layout).
    #[must_use]
    pub fn matches_container_query(query: &str, w: f32, h: f32) -> bool {
        let eval_one = |q: &str| -> bool {
            // `not` préfixe : nie la branche entière.
            let (neg, q) = match q.trim_start().strip_prefix("not") {
                Some(rest) => (true, rest.trim_start()),
                None => (false, q),
            };
            let mut all = true;
            let mut seen = false;
            for clause in q.split('(').skip(1) {
                let body = clause.split(')').next().unwrap_or("").trim();
                if body.is_empty() {
                    continue;
                }
                // `and` peut chaîner plusieurs features par
                // parenthèse dans la forme longue — split sur
                // `and` à l'intérieur de la clause.
                for feat in body.split(" and ") {
                    seen = true;
                    all &= container_feature(feat.trim(), w, h);
                }
            }
            // Aucune feature reconnue → fausse (fail-closed).
            seen && all != neg
        };
        // `or` disjoint — chaque branche doit matcher seule.
        query.split(" or ").map(str::trim).any(eval_one)
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
        self.compute_tree_with_ctx(dom, root, env, state, None)
    }

    /// Variante contextuelle (R-190 — passe 2 `@container`) :
    /// `containers` = mesures des boîtes `container-type`
    /// produites par `explorer_layout::measure_containers`
    /// après un premier layout. `None` = première passe —
    /// les règles `@container` taguées restent inertes
    /// (sortie strictement identique à
    /// [`StyleEngine::compute_tree_with_state`]).
    #[must_use]
    pub fn compute_tree_with_ctx(
        &self,
        dom: &Dom,
        root: NodeId,
        env: &StyleEnv,
        state: &dyn Fn(&str) -> bool,
        containers: Option<&HashMap<NodeId, ContainerMeasure>>,
    ) -> StyleMap {
        let mut map = HashMap::new();
        let mut pseudos = HashMap::new();
        let mut mcache = MatchCache::default();
        mcache.namespaces.clone_from(&self.namespaces);
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
            containers,
        );
        self.resolve_font_alternates(&mut map);
        StyleMap {
            map,
            pseudos,
            counter_styles: self.counter_styles.clone(),
            font_feature_values: self.font_feature_values.clone(),
            font_palettes: self.font_palettes.clone(),
        }
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
        self.compute_scoped_with_ctx(dom, root, env, parent, state, None)
    }

    /// Variante contextuelle (R-190 — passe 2 `@container`)
    /// de [`StyleEngine::compute_scoped_with_state`] — même
    /// contrat, les règles taguées s'évaluent contre
    /// `containers` (mesures post-layout).
    #[must_use]
    pub fn compute_scoped_with_ctx(
        &self,
        dom: &Dom,
        root: NodeId,
        env: &StyleEnv,
        parent: Option<&ComputedStyle>,
        state: &dyn Fn(&str) -> bool,
        containers: Option<&HashMap<NodeId, ContainerMeasure>>,
    ) -> StyleMap {
        let mut map = HashMap::new();
        let mut pseudos = HashMap::new();
        let mut mcache = MatchCache::default();
        mcache.namespaces.clone_from(&self.namespaces);
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
            containers,
        );
        self.resolve_font_alternates(&mut map);
        StyleMap {
            map,
            pseudos,
            counter_styles: self.counter_styles.clone(),
            font_feature_values: self.font_feature_values.clone(),
            font_palettes: self.font_palettes.clone(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    #[allow(clippy::too_many_lines)]
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
        containers: Option<&HashMap<NodeId, ContainerMeasure>>,
    ) {
        let style = self.compute_with_parent(
            dom, node, env, parent, None, state, mcache, anc, *root_fs, containers,
        );
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
            // `need_content` = porte `::before`/`::after`
            // (R-13 — une règle sans `content` ne génère rien) ;
            // les pseudos R-177 (`first-line`, `marker`…)
            // gardent tout style produit.
            let mut pseudo_style =
                |pe: crate::PseudoElement, need_content: bool| -> Option<ComputedStyle> {
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
                        containers,
                    );
                    let keep = if need_content {
                        s.get("content").is_some()
                    } else {
                        s.iter().next().is_some()
                    };
                    keep.then_some(s)
                };
            let before = pseudo_style(crate::PseudoElement::Before, true);
            let after = pseudo_style(crate::PseudoElement::After, true);
            let ps = PseudoStyles {
                before,
                after,
                first_line: pseudo_style(crate::PseudoElement::FirstLine, false),
                first_letter: pseudo_style(crate::PseudoElement::FirstLetter, false),
                marker: pseudo_style(crate::PseudoElement::Marker, false),
                selection: pseudo_style(crate::PseudoElement::Selection, false),
                backdrop: pseudo_style(crate::PseudoElement::Backdrop, false),
            };
            if ps.before.is_some()
                || ps.after.is_some()
                || ps.first_line.is_some()
                || ps.first_letter.is_some()
                || ps.marker.is_some()
                || ps.selection.is_some()
                || ps.backdrop.is_some()
            {
                pseudos.insert(node, ps);
            }
        }
        let Ok(children) = dom.children_iter(node) else {
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
        // R-96/97 — arbre shadow calculé AVANT les enfants
        // light : le flat tree de l'hôte est fait des enfants
        // de sa racine (latérale — hors `children`), et les
        // styles des `<slot>` doivent déjà être dans `map`
        // quand les enfants light distribués héritent d'eux.
        // L'héritage franchit la frontière : le style de
        // l'hôte est le parent des enfants shadow
        // (css-scoping §3.3 — les `--*` et propriétés
        // héritées traversent, les sélecteurs non : le
        // filtre de `collect_candidates` tient la porte).
        let host_root = dom.shadow_root(node, true);
        if let Some(root) = host_root {
            if let Ok(schildren) = dom.children_iter(root) {
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
                                containers,
                            );
                        });
                    }
                }
            }
        }
        for child in children {
            if dom.element(child).is_ok() {
                // R-97 — distribution : un enfant light
                // assigné hérite du style de son `<slot>`
                // (flat tree, css-scoping §3.4) ; les non-
                // assignés héritent de l'hôte (parent DOM).
                // La sonde n'est payée que sur hôte shadow.
                let slot_style = host_root
                    .and_then(|_| dom.assigned_slot(child).and_then(|s| map.get(&s).cloned()));
                let pstyle = slot_style.as_ref().unwrap_or(&style);
                // Récursion ∝ profondeur DOM → pile garantie
                // (R-24) : bascule tas en zone rouge.
                explorer_common::deep(|| {
                    self.compute_subtree(
                        dom,
                        child,
                        env,
                        Some(pstyle),
                        map,
                        pseudos,
                        state,
                        &mut *mcache,
                        &child_anc,
                        root_fs,
                        containers,
                    );
                });
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
        containers: Option<&HashMap<NodeId, ContainerMeasure>>,
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
        let mut inline_decls = if pseudo.is_none() {
            dom.attribute_str(node, "style")
                .ok()
                .flatten()
                .map(crate::parser::parse_declarations)
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        // R-182 — quirks « hashless hex » (HTML Standard
        // §quirks colors) : `color:abc` dans `style=` sans
        // `#` — la valeur passe par `legacy_color`
        // (non-hex → `0`, complétion à 3 digits). Seules les
        // propriétés couleur, jamais en standards.
        if matches!(dom.document_mode(), explorer_dom::DocumentMode::Quirks) {
            for d in &mut inline_decls {
                if (d.name == "color" || d.name.ends_with("-color"))
                    && !d.value.trim_start().starts_with('#')
                    && crate::value::parse_color(&d.value).is_none()
                {
                    if let Some(c) = legacy_color(&d.value) {
                        d.value = c;
                    }
                }
            }
        }
        let mut cand =
            self.collect_candidates(dom, node, env, pseudo, state, mcache, anc, containers);
        if pseudo.is_none() {
            // R-207 — le style inline est aussi de la déclaration
            // auteur : même census que les règles.
            for d in &inline_decls {
                census_prop(&d.name);
            }
            // `style=` vit au contexte du nœud : interne si le
            // nœud est en shadow — le rang de frontière est
            // le même que `scope == sa racine` (§6, R-111).
            let node_outer = dom.containing_shadow_root(node).is_none();
            for d in &inline_decls {
                let rank = if d.important { 4 } else { 1 };
                // `style=` est hors-layer — la clé miroir gère
                // le renversement `!important` (R-91).
                cand.push((
                    rank,
                    u8::from(d.important == node_outer),
                    layer_sort_key(&[], d.important),
                    (u32::MAX, u32::MAX, u32::MAX),
                    u32::MAX,
                    d,
                ));
            }
            // Le rang inline s'intercale dans l'ordre global.
            cand.sort_by_key(|(rank, scope_rank, layer, spec, order, _)| {
                (*rank, *scope_rank, layer.clone(), *spec, *order)
            });
        }
        // R-215 — rang d'origine par nom inséré : la
        // résolution logique→physique doit pouvoir écraser
        // un physique de moindre priorité (UA `padding-left`
        // vs auteur `padding-inline-start` — bug mesuré
        // `dw +24` wikipedia). Absent de `ranks` = hint/
        // héritage = rang 0 (toujours le plus faible).
        let mut ranks: HashMap<String, u8> = HashMap::new();
        for (rank, _, _, _, _, d) in &cand {
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
                vendor_alias(&d.name.to_ascii_lowercase()).to_string()
            };
            // R-232 — `all: kw` : reset de TOUTES les longhands
            // connues au mot-clé (`initial`/`inherit`/`unset`/
            // `revert[-layer]` — chacune rejoint son repli
            // aval : hérité→parent, sinon initiale).
            if lname == "all"
                && matches!(
                    d.value.trim().to_ascii_lowercase().as_str(),
                    "initial" | "inherit" | "unset" | "revert" | "revert-layer"
                )
            {
                for p in SUPPORTED_PROPS {
                    ranks.insert((*p).to_string(), *rank);
                    props.insert((*p).to_string(), d.value.clone());
                }
                continue;
            }
            // Shorthand → longhands (spec : `margin: x` fixe les
            // quatre côtés, chacun à la spécificité de la règle).
            match expand_shorthand(&lname, &d.value) {
                Some(expanded) => {
                    for (long_name, v) in expanded {
                        ranks.insert(long_name.clone(), *rank);
                        props.insert(long_name, v);
                    }
                }
                None => {
                    // Verbatim : la validation par propriété
                    // fait le tri (`width: 200` sans unité,
                    // raccourci invalide → déclaration ignorée).
                    if value_is_valid_for(&lname, &d.value) {
                        ranks.insert(lname.clone(), *rank);
                        props.insert(lname, d.value.clone());
                    } else if matches!(dom.document_mode(), explorer_dom::DocumentMode::Quirks)
                        && QUIRKY_LENGTH_PROPS.contains(&lname.as_str())
                        && d.value.trim().parse::<f32>().is_ok()
                    {
                        // R-182 — quirks « unitless length » :
                        // `N` nu devient `Npx` (HTML Standard
                        // §quirks — les props numériques déjà
                        // valides ne passent jamais ici).
                        props.insert(lname, format!("{}px", d.value.trim()));
                    } else {
                        // R-207 — déclaration ignorée sur la
                        // validation par propriété (`width:
                        // 200`…) : signalée par nom ; le détail
                        // porte le début de la valeur pour que
                        // le Pareto nomme la forme fautive
                        // (github : `font-size` ×108).
                        explorer_common::census::bump(&format!("css.value.dropped.{lname}"));
                        let v = d.value.trim();
                        let v = v.strip_prefix("var(--").unwrap_or(v);
                        let key: String = v
                            .chars()
                            .take(30)
                            .map(|c| {
                                if c.is_ascii_alphanumeric()
                                    || matches!(c, '-' | '.' | '(' | ')' | '%')
                                {
                                    c
                                } else {
                                    '_'
                                }
                            })
                            .collect();
                        explorer_common::census::bump(&format!("css.value.dropped.{lname}.{key}"));
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
            apply_presentational_hints(dom, node, parent, env, &mut props);
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
        // 3b. Propriétés logiques → noms du repère de flux
        // (R-183) : après l'héritage — `writing-mode` et
        // `direction` de l'élément ET le mode du parent sont
        // alors connus. Le physique déclaré l'emporte
        // (l'ordre fin n'est pas préservable en `HashMap` —
        // approximation documentée).
        resolve_logical_props(&mut props, parent, &ranks);
        // R-235 — `-webkit-text-fill-color` remplace `color`
        // pour le remplissage du texte (spec compat : le fill
        // l'emporte quand présent, sauf `transparent`).
        if let Some(v) = props.get("-webkit-text-fill-color").cloned() {
            if v.trim() != "transparent" {
                props.insert("color".to_string(), v);
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
        // `@property` (R-118 — css-properties-values-api-1 §3)
        //    — appliqué AVANT `var()` : `props` porte les
        //    déclarations propres du nœud ; `vars` les héritées.
        //    `inherits:false` masque l'héritage ; valeur absente
        //    → `initial-value` (ou guaranteed-invalid — la
        //    registration est invalide sans initial quand
        //    `syntax` ≠ `*`).
        if !self.property_registry.is_empty() {
            for (name, reg) in &self.property_registry {
                if props.keys().any(|k| k == name) {
                    continue;
                }
                let v = reg
                    .initial
                    .clone()
                    .unwrap_or_else(|| GUARANTEED_INVALID.to_string());
                if (!reg.inherits || !vars.contains_key(name))
                    && vars.get(name).map(String::as_str) != Some(v.as_str())
                {
                    Arc::make_mut(&mut vars).insert(name.clone(), v);
                }
            }
        }
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
        // 8. `light-dark(a, b)` (R-105 — css-color-adjust-1
        //    §2.4) : résolu à la valeur calculée — le schéma
        //    usé de l'élément est sa `color-scheme` calculée
        //    (héritée ou déclarée) ∩ la préférence `env`.
        if props.values().any(|v| v.contains("light-dark(")) {
            let dark = Self::used_dark_scheme(props.get("color-scheme").map(String::as_str), env);
            for v in props.values_mut() {
                if v.contains("light-dark(") {
                    *v = resolve_light_dark(v, dark);
                }
            }
        }
        // 9. `color-mix` (R-114 — css-color-5 §3) : résolu à la
        //    valeur calculée, après `var()`/`light-dark`.
        if props
            .values()
            .any(|v| v.to_ascii_lowercase().contains("color-mix("))
        {
            for v in props.values_mut() {
                if v.to_ascii_lowercase().contains("color-mix(") {
                    *v = color_mix::resolve_color_mix(v);
                }
            }
        }
        // 10. Flexbox 2009 (R-192) : `display:-webkit-box` et
        //    la famille `-webkit-box-*` aliasée vers le modèle
        //    flex moderne — la prop moderne déclarée prime
        //    (injection seulement si absente).
        legacy_flex_aliases(&mut props);
        // 11. `subgrid` (R-196) : le grid item hérite le
        //    `grid-template-*` **déclaré** de son parent grid
        //    (MVP — approximations documentées : pas de
        //    contribution des items du sous-grid au sizing
        //    des pistes parent, pas de trim au span réel ;
        //    la chaîne subgrid→subgrid fonctionne, le
        //    template résolu cascade). Parent non-grid →
        //    `none` (spec : subgrid sans parent grid = none).
        for axis in ["grid-template-columns", "grid-template-rows"] {
            if props
                .get(axis)
                .is_some_and(|v| v.trim().eq_ignore_ascii_case("subgrid"))
            {
                let parent_is_grid = parent.is_some_and(|p| {
                    p.get("display")
                        .is_some_and(|d| matches!(d, "grid" | "inline-grid"))
                });
                // Le parent a lui-même `subgrid` ? Sa valeur
                // est déjà résolue amont (compute_tree est
                // top-down) → la copie propage la liste
                // héritée ; littéral `subgrid` résiduel →
                // `none` (pas de grid ancêtre).
                let inherited = if parent_is_grid {
                    parent
                        .and_then(|p| p.get(axis))
                        .map(|v| v.trim().to_string())
                        .filter(|v| !v.eq_ignore_ascii_case("subgrid"))
                } else {
                    None
                };
                props.insert(
                    axis.to_string(),
                    inherited.unwrap_or_else(|| "none".to_string()),
                );
            }
        }
        // 12. `@position-try --nom` (css-anchor-1) : la règle
        //     nommée référencée par `position-try`/
        //     `position-try-fallbacks` est sérialisée dans une
        //     clé porteuse `position-try--nom` — le layout ne
        //     connaît pas la feuille, seules ses déclarations
        //     servent en débordement (pas de style anticipé).
        for prop in ["position-try", "position-try-fallbacks"] {
            if let Some(names) = props.get(prop).cloned() {
                for name in names.split_whitespace().filter(|t| t.starts_with("--")) {
                    if let Some(decls) = self.position_tries.get(name) {
                        let body: String = decls
                            .iter()
                            .map(|d| format!("{}:{}", d.name, d.value.trim()))
                            .collect::<Vec<_>>()
                            .join(";");
                        props.insert(format!("position-try-{}", name.trim()), body);
                    }
                }
            }
        }
        ComputedStyle { props, vars }
    }

    /// Schéma usé sombre ? `color-scheme` : `only light` ou
    /// `light` seul → clair ; `only dark`/`dark` seul →
    /// sombre ; les deux (ou `normal`/absent) → préférence
    /// de l'env (`prefers-color-scheme`, R-59).
    fn used_dark_scheme(scheme: Option<&str>, env: &StyleEnv) -> bool {
        let Some(s) = scheme else {
            return env.color_scheme == ColorScheme::Dark;
        };
        let has = |w| {
            s.split_ascii_whitespace()
                .any(|t| t.eq_ignore_ascii_case(w))
        };
        let (l, d) = (has("light"), has("dark"));
        match (l, d) {
            (true, false) => false,
            (false, true) => true,
            _ => env.color_scheme == ColorScheme::Dark,
        }
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
            crate::PseudoElement::FirstLine => {
                (self.index.has_first_line, self.quirks_index.has_first_line)
            }
            crate::PseudoElement::FirstLetter => (
                self.index.has_first_letter,
                self.quirks_index.has_first_letter,
            ),
            crate::PseudoElement::Marker => (self.index.has_marker, self.quirks_index.has_marker),
            crate::PseudoElement::Selection => {
                (self.index.has_selection, self.quirks_index.has_selection)
            }
            crate::PseudoElement::Backdrop => {
                (self.index.has_backdrop, self.quirks_index.has_backdrop)
            }
            crate::PseudoElement::Other => (false, false),
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
        let Some(sel) = rule.selectors.get(sel_idx) else {
            return false;
        };
        let Some((last, _)) = sel.parts.last() else {
            return false;
        };
        if last
            .simples
            .iter()
            .any(|s| matches!(s, SimpleSelector::Slotted(_)))
        {
            // R-97 — `::slotted` n'a de sens qu'en feuille
            // scopée : le sujet doit être assigné à un
            // `<slot>` de la racine de la RÈGLE — arbitré
            // avant le raccourci même-arbre (un enfant light
            // assigné est dans l'arbre englobant, pas dans
            // l'arbre de la feuille ; et en feuille document
            // le scope `None` ne possède aucun slot — pas de
            // raccourci `None == None` : le slot doit exister).
            return dom
                .assigned_slot(node)
                .is_some_and(|s| dom.containing_shadow_root(s) == rule.scope);
        }
        if rule.scope == node_scope {
            return true;
        }
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
    #[allow(clippy::too_many_arguments, clippy::type_complexity)]
    fn collect_candidates<'a>(
        &'a self,
        dom: &Dom,
        node: NodeId,
        env: &StyleEnv,
        pseudo: Option<crate::PseudoElement>,
        state: &dyn Fn(&str) -> bool,
        mcache: &mut MatchCache,
        anc: &AncestorFilter,
        containers: Option<&HashMap<NodeId, ContainerMeasure>>,
    ) -> Vec<(u8, u8, Vec<u32>, Specificity, u32, &'a Declaration)> {
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
            // R-190 — règle issue d'un `@container` : inerte
            // hors seconde passe (`containers` absent) ou si
            // sa query échoue sur le conteneur ancêtre mesuré.
            if let Some(ci) = rule.container {
                let ok = containers.is_some_and(|map| {
                    self.container_queries
                        .get(ci as usize)
                        .is_some_and(|pred| container_pred_matches(dom, node, pred, map))
                });
                if !ok {
                    return;
                }
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
        let mut rules_cand: Vec<(u8, u8, Vec<u32>, Specificity, u32, &Declaration)> = Vec::new();
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
                // R-111 — frontière shadow (css-scoping §6) :
                // pour les règles **normales** la déclaration
                // interne (scopée à la racine du nœud) bat la
                // document ; pour `!important` c'est
                // l'**inverse** — la document gagne. La porte
                // `rules_reaches` garantit que `scope.is_some()`
                // = racine du nœud pour les règles retenues.
                let scope_rank = u8::from(d.important == rule.scope.is_none());
                rules_cand.push((
                    rank,
                    scope_rank,
                    layer_sort_key(&rule.layer, d.important),
                    spec,
                    rule.order,
                    d,
                ));
            }
        }
        // Rang (origine/important) → frontière shadow (§6,
        // R-111) → layer (§6.4, inversion `!important`
        // comprise) → spécificité → ordre source.
        rules_cand.sort_by_key(|(rank, scope_rank, layer, spec, order, _)| {
            (*rank, *scope_rank, layer.clone(), *spec, *order)
        });
        rules_cand
    }
}

/// R-190 — `pred` `@container` s'applique-t-il à `node` ?
/// Le conteneur de la query = l'ancêtre DOM le plus proche
/// mesuré (`containers`) et passant le filtre de nom ; sa
/// border-box mesurée est évaluée par
/// [`StyleEngine::matches_container_query`]. Aucun ancêtre
/// éligible → la règle reste inerte (spec : sans conteneur,
/// la query est fausse). Approximation MVP : l'éligibilité
/// par feature n'est pas modélisée — un conteneur
/// `inline-size` répond aussi aux queries height (spec :
/// seules les queries d'axe inline y sont admises).
fn container_pred_matches(
    dom: &Dom,
    node: NodeId,
    pred: &CollectedContainer,
    containers: &HashMap<NodeId, ContainerMeasure>,
) -> bool {
    let mut cur = node;
    while let Ok(Some(p)) = dom.parent(cur) {
        if let Some(m) = containers.get(&p) {
            // Filtre de nom : la query nommée ne se pose qu'au
            // premier conteneur ancêtre portant CE nom — les
            // conteneurs intermédiaires d'un autre nom sont
            // sautés (css-contain-3 §4 — nearest matching).
            if pred
                .name
                .as_deref()
                .is_none_or(|n| m.names.iter().any(|x| x == n))
            {
                return StyleEngine::matches_container_query(&pred.query, m.w, m.h);
            }
        }
        cur = p;
    }
    false
}

/// Une feature `(…)` de query `@container` (R-190) :
/// `(name: Npx)` ou la forme de plage MQ5
/// `(width >= 400px)` / `(400px <= width <= 800px)` /
/// `(400px < width)`. Inconnue ou valeur non résoluble →
/// **fausse** (fail-closed, voir `matches_container_query`).
fn container_feature(feat: &str, w: f32, h: f32) -> bool {
    if let Some((name, val)) = feat.split_once(':') {
        let Ok(v) = val.trim().trim_end_matches("px").trim().parse::<f32>() else {
            return false;
        };
        return match name.trim() {
            "min-width" => w >= v,
            "max-width" => w <= v,
            "width" => (w - v).abs() < f32::EPSILON,
            "min-height" => h >= v,
            "max-height" => h <= v,
            "height" => (h - v).abs() < f32::EPSILON,
            _ => false, // feature inconnue → fausse (spec)
        };
    }
    container_range_feature(feat, w, h)
}

/// Forme de plage MQ5 : `feature op val`, `val op feature`
/// ou `v1 op feature op v2` — `op` ∈ `< <= > >= =`.
/// Inconnu → `false` (fail-closed).
fn container_range_feature(feat: &str, w: f32, h: f32) -> bool {
    let toks: Vec<&str> = feat.split_whitespace().collect();
    let size_of = |name: &str| match name {
        "width" => Some(w),
        "height" => Some(h),
        _ => None,
    };
    let px = |t: &str| t.trim().trim_end_matches("px").trim().parse::<f32>().ok();
    // `a op b` : comparaison dans le sens écrit.
    let cmp = |a: f32, op: &str, b: f32| -> Option<bool> {
        Some(match op {
            "<" => a < b,
            "<=" => a <= b,
            ">" => a > b,
            ">=" => a >= b,
            "=" | "==" => (a - b).abs() < f32::EPSILON,
            _ => return None,
        })
    };
    match toks.as_slice() {
        // `width >= 400px`
        [name, op, val] if size_of(name).is_some() => size_of(name)
            .and_then(|s| px(val).and_then(|v| cmp(s, op, v)))
            .unwrap_or(false),
        // `400px <= width`
        [val, op, name] if size_of(name).is_some() => size_of(name)
            .and_then(|s| px(val).and_then(|v| cmp(v, op, s)))
            .unwrap_or(false),
        // `400px <= width <= 800px`
        [v1, op1, name, op2, v2] if size_of(name).is_some() => size_of(name)
            .and_then(|s| {
                Some(px(v1).and_then(|a| cmp(a, op1, s))? && px(v2).and_then(|b| cmp(s, op2, b))?)
            })
            .unwrap_or(false),
        _ => false,
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
        // css-fonts-4 : `math` = taille calculée du parent.
        "math" => return Some(parent_fs),
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
fn apply_presentational_hints(
    dom: &Dom,
    node: NodeId,
    parent: Option<&ComputedStyle>,
    env: &StyleEnv,
    props: &mut HashMap<String, String>,
) {
    let Ok(tag) = dom.tag_name(node) else {
        return;
    };
    // `field-sizing:content` (css-forms / R-277) : le
    // contrôle se mesure au **contenu** — les dimensions
    // fixes issues de `cols`/`rows`/`size` (et leurs
    // défauts UA) ne s'appliquent plus. La propriété n'est
    // PAS héritée : seule la déclaration propre compte,
    // déjà résolue dans `props` quand les hints passent.
    // `width`/`height` auteur priment de toute façon
    // (rang des hints) — ici on supprime le hint lui-même.
    let field_content = props
        .get("field-sizing")
        .is_some_and(|v| v.trim() == "content");
    match tag {
        "table" | "td" | "th" | "tr" | "col" | "colgroup" | "img" | "iframe" | "object"
        | "embed" | "canvas" | "video" => {
            hint(props, "width", dim(hint_attr(dom, node, "width")));
            hint(props, "height", dim(hint_attr(dom, node, "height")));
        }
        "svg" => {
            // SVG2 : `width`/`height` absent = `auto` =
            // `100%` — PAS la taille du viewBox (vol
            // 2026-10-26 : logo w3.org géant — un `<svg>`
            // à `viewBox` seul occupait ses ~1000 unités
            // en px au lieu de 100 % du conteneur, la
            // hauteur dérivant du ratio comme `height:
            // auto`). Sans `viewBox` le repli remplacé
            // 300×150 reste la référence — le hint n'est
            // posé que si la géométrie est définie.
            // `width`/`height` absent = taille d'objet remplacé
            // par défaut **300 px** (comportement Chrome — vol
            // 25 kayak : l'icône `⇄` à viewBox seul remplissait
            // ses 325 px de colonne). La hauteur dérive du
            // ratio du viewBox (§10.6.2 — un viewBox carré
            // donne 300×300, pas 300×150). `min(300px,100%)`
            // pour ne pas déborder un conteneur plus étroit.
            let vb = hint_attr(dom, node, "viewBox").is_some();
            hint(
                props,
                "width",
                dim(hint_attr(dom, node, "width"))
                    .or_else(|| vb.then(|| "min(300px, 100%)".to_string())),
            );
            hint(props, "height", dim(hint_attr(dom, node, "height")));
        }
        "hr" => {
            hint(props, "width", dim(hint_attr(dom, node, "width")));
            // `size` = hauteur du filet — compteurs/bordures
            // exclus (approximation documentée).
            hint(props, "height", dim(hint_attr(dom, node, "size")));
        }
        // `field-sizing:content` → pas de taille fixe sur
        // textarea/input (mesure au contenu) : la garde
        // laisse le tag tomber sur `_`.
        "textarea" if !field_content => {
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
        "input" if !field_content => {
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
    // R-171 — `dir` → `direction` sur tout élément (HTML
    // Rendering §bidi, presentational hint) : `ltr`/`rtl`
    // directs ; `auto` = premier caractère fort du texte
    // descendant (scan borné — spec `dir=auto`) — valeur
    // par défaut de `<bdi>`.
    hint(props, "direction", hint_dir(dom, node, tag));
    // R-176 — `unicode-bidi` UA (HTML Rendering §bidi) :
    // `<bdo>` force la surcharge (`bidi-override`), `<bdi>`
    // isole (`isolate`).
    hint(
        props,
        "unicode-bidi",
        match tag {
            "bdo" => Some("bidi-override".to_string()),
            "bdi" => Some("isolate".to_string()),
            _ => None,
        },
    );
    legacy_presentational_hints(dom, node, tag, parent, env, props);
    // R-193 — `<basefont>` : la fonte de base du document
    // est un hint de la **racine** (héritage sur tout le
    // sous-arbre — descendants `<font size=±N>` fils
    // directs inclus).
    if parent.is_none() {
        hint_basefont(dom, props);
    }
}

/// `<font>` (HTML 3.2/4.01 — la moitié du corpus legacy)
/// — mots-clés `font-size` en index de table absolue.
const FONT_SIZE_KW: [&str; 7] = [
    "xx-small", "x-small", "small", "medium", "large", "x-large", "xx-large",
];

/// Facteur de chaque mot-clé sur la base utilisateur —
/// même ordre que [`FONT_SIZE_KW`], même échelle ×1.2 que
/// [`resolve_font_size`] (sert au mapping px→index du
/// parent pour `size=±N`).
const FONT_SIZE_FACTOR: [f32; 7] = [1.0 / 1.728, 1.0 / 1.44, 1.0 / 1.2, 1.0, 1.2, 1.44, 1.728];

/// `size` de `<font>` → mot-clé `font-size` : absolu `1..7`
/// → `xx-small`…`xx-large` ; `+N`/`-N` relatif à l'index du
/// parent (HTML — `3` = medium si le parent n'est pas un
/// mot-clé, sa taille px est arrondie à l'index le plus
/// proche). Invalide → `None`.
fn font_size_hint(
    a: Option<&str>,
    parent: Option<&ComputedStyle>,
    env: &StyleEnv,
) -> Option<String> {
    let t = a?.trim();
    if let Some(rest) = t.strip_prefix(['+', '-']) {
        let n = rest.trim().parse::<i32>().ok()?;
        let n = if t.starts_with('-') { -n } else { n };
        // Index du parent : mot-clé direct, ou font-size
        // **calculée** (px absolu — `normalize_font_size`
        // l'a déjà résolue) ramenée à l'index le plus proche
        // sur la base utilisateur ; relatif/inconnu → 3
        // (medium — base HTML des tailles `<font>`).
        let pidx = i32::try_from(
            parent
                .and_then(|p| p.get("font-size"))
                .and_then(|v| {
                    let v = v.trim();
                    FONT_SIZE_KW
                        .iter()
                        .position(|kw| v.eq_ignore_ascii_case(kw))
                        .or_else(|| {
                            let len = parse_length(v)?;
                            let fs =
                                (len.value * len.unit.px_factor()?) / f64::from(env.font_size_base);
                            FONT_SIZE_FACTOR
                                .iter()
                                .enumerate()
                                .min_by(|(_, a), (_, b)| {
                                    (fs - f64::from(**a))
                                        .abs()
                                        .partial_cmp(&(fs - f64::from(**b)).abs())
                                        .unwrap_or(std::cmp::Ordering::Equal)
                                })
                                .map(|(i, _)| i)
                        })
                })
                .unwrap_or(3),
        )
        .unwrap_or(3);
        let idx = usize::try_from((pidx + n).clamp(0, 6)).unwrap_or(3);
        return Some(FONT_SIZE_KW[idx].to_string());
    }
    let n: usize = t.parse().ok()?;
    (1..=7)
        .contains(&n)
        .then(|| FONT_SIZE_KW[n - 1].to_string())
}

/// Couleur d'attribut HTML → valeur CSS (`#rrggbb` ou
/// syntaxe moderne). Règles « legacy color » du HTML
/// Standard : si la valeur n'est pas déjà une couleur CSS,
/// les caractères non-hex deviennent `0`, la chaîne est
/// complétée à un multiple de 3 puis chaque tiers tronqué
/// à 2 digits — `bgcolor="ffcc00"` / `color="f0f"` vivent
/// ainsi (R-154/155). `None` = vide/`transparent`.
fn legacy_color(v: &str) -> Option<String> {
    let t = v.trim();
    if t.is_empty() || t.eq_ignore_ascii_case("transparent") {
        return None;
    }
    if parse_color(t).is_some() {
        return Some(t.to_string());
    }
    let mut s: String = t
        .chars()
        .map(|c| if c.is_ascii_hexdigit() { c } else { '0' })
        .collect();
    while s.len() < 3 || s.len() % 3 != 0 {
        s.push('0');
    }
    let third = s.len() / 3;
    let mut out = String::from("#");
    for i in 0..3 {
        let part = &s[i * third..(i + 1) * third];
        let mut it = part.chars();
        out.push(it.next().unwrap_or('0'));
        out.push(it.next().unwrap_or('0'));
    }
    Some(out)
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
fn hint_attr<'a>(dom: &'a Dom, node: NodeId, name: &str) -> Option<&'a str> {
    dom.attribute_str(node, name).ok().flatten()
}

/// `N` → "Npx", `N%` → "N%" ; autres syntaxes → None.
fn dim(a: Option<&str>) -> Option<String> {
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
fn chars(a: Option<&str>) -> Option<String> {
    a?.trim()
        .parse::<u32>()
        .ok()
        .filter(|n| *n >= 1)
        .map(|n| n.min(1000).to_string() + "ch")
}

/// `N` lignes → "N×1.2em" — `rows` de `textarea` : hauteur
/// = N × hauteur de ligne normale (approx 1.2em — le hint
/// reste rang UA, la CSS auteur prime).
fn lines(a: Option<&str>) -> Option<String> {
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
fn span(a: Option<&str>) -> Option<String> {
    a?.trim()
        .parse::<u32>()
        .ok()
        .filter(|n| *n >= 1)
        .map(|n| n.min(1000).to_string())
}

/// `dir` → `direction` (R-171, HTML Rendering §bidi) :
/// `ltr`/`rtl` se mappent directement ; `auto` suit le
/// premier caractère à directionnalité forte du texte
/// descendant (spec `dir=auto` — **défaut de `<bdi>`**,
/// attribut absent inclus). `None` sans `dir` ou valeur
/// invalide → la propriété suit l'héritage.
fn hint_dir(dom: &Dom, node: NodeId, tag: &str) -> Option<String> {
    let value = match hint_attr(dom, node, "dir") {
        Some(v) => v.trim().to_ascii_lowercase(),
        None if tag == "bdi" => "auto".to_string(),
        None => return None,
    };
    match value.as_str() {
        v @ ("ltr" | "rtl") => Some(v.to_string()),
        "auto" => dir_auto(dom, node).map(str::to_string),
        _ => None,
    }
}

/// Direction du premier caractère fort (bidi class `L` →
/// `ltr`, `R`/`AL` → `rtl`) dans le texte descendant de
/// `node` — préordre document borné (`MAX_DIR_SCAN` nœuds,
/// contenu hostile). Les sous-arbres `bdi`/`bdo`/`script`/
/// `style`/`textarea` et tout élément portant son propre
/// `dir` sont sautés : ils portent leur directionnalité
/// (spec `dir=auto`). `None` = aucun fort trouvé → héritage.
fn dir_auto(dom: &Dom, node: NodeId) -> Option<&'static str> {
    /// Budget de nœuds visités par résolution `dir=auto`.
    const MAX_DIR_SCAN: u32 = 4_096;
    let mut stack: Vec<NodeId> = Vec::new();
    if let Ok(it) = dom.children_iter(node) {
        stack.extend(it);
        stack.reverse();
    }
    let mut visited = 0u32;
    while let Some(n) = stack.pop() {
        visited += 1;
        if visited > MAX_DIR_SCAN {
            break;
        }
        if let Ok(t) = dom.text(n) {
            for c in t.chars() {
                match unicode_bidi::bidi_class(c) {
                    unicode_bidi::BidiClass::L => return Some("ltr"),
                    unicode_bidi::BidiClass::R | unicode_bidi::BidiClass::AL => {
                        return Some("rtl");
                    }
                    _ => {}
                }
            }
            continue;
        }
        if let Ok(tag) = dom.tag_name(n) {
            if matches!(tag, "bdi" | "bdo" | "script" | "style" | "textarea")
                || hint_attr(dom, n, "dir").is_some()
            {
                continue;
            }
        }
        if let Ok(it) = dom.children_iter(n) {
            // `collect` + reverse : pop en préordre.
            let mut kids: Vec<NodeId> = it.collect();
            stack.reserve(kids.len());
            while let Some(k) = kids.pop() {
                stack.push(k);
            }
        }
    }
    None
}

/// `align` → text-align (spec : presentational hint) ;
/// `middle` sur `div` vaut `center` (HTML Rendering).
fn hint_align(a: Option<&str>) -> Option<String> {
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
fn ancestor_table_attr<'a>(dom: &'a Dom, node: NodeId, name: &str) -> Option<&'a str> {
    ancestor_elem_attr(dom, node, "table", name)
}

/// Attribut `name` de l'ancêtre `<tag>` le plus proche
/// (`cellpadding` sur `table`, `link` sur `body` —
/// R-157). La **première** occurrence en remontant fait
/// foi.
fn ancestor_elem_attr<'a>(dom: &'a Dom, node: NodeId, tag: &str, name: &str) -> Option<&'a str> {
    let mut n = dom.parent(node).ok()?;
    while let Some(id) = n {
        if dom.tag_name(id).is_ok_and(|t| t == tag) {
            return dom.attribute_str(id, name).ok().flatten();
        }
        n = dom.parent(id).ok()?;
    }
    None
}

/// Hints « legacy » HTML Rendering : `bgcolor`, `align`,
/// `valign`, `cellspacing`/`cellpadding`/`border` des tables
/// (R-16 — les pages tabulaires type HN en dépendent).
/// R-192 — flexbox 2009 (`display:-webkit-box` + famille
/// `-webkit-box-*`) aliasée vers le modèle flex moderne.
/// La prop **moderne** déclarée prime : chaque alias n'est
/// injecté que si son équivalent est absent. Approximation
/// documentée : `-webkit-box-ordinal-group` est 1-based
/// (copié tel quel dans `order` — le décalage de 1 n'a
/// d'effet que sur l'ordre relatif entre groupes > 1, cas
/// rare en terrain).
fn legacy_flex_aliases(props: &mut HashMap<String, String>) {
    match props.get("display").map(String::as_str) {
        Some("-webkit-box") => {
            props.insert("display".into(), "flex".into());
        }
        Some("-webkit-inline-box") => {
            props.insert("display".into(), "inline-flex".into());
        }
        _ => {}
    }
    let get = |p: &str| {
        props
            .get(p)
            .map(|v| v.trim().to_string())
            .unwrap_or_default()
    };
    let orient = get("-webkit-box-orient");
    let wdir = get("-webkit-box-direction");
    let pack = get("-webkit-box-pack");
    let align = get("-webkit-box-align");
    let grow = get("-webkit-box-flex");
    let ordinal = get("-webkit-box-ordinal-group");
    if !props.contains_key("flex-direction") {
        let mut dir = match orient.as_str() {
            "vertical" | "block-axis" => "column",
            _ => "row",
        };
        if wdir == "reverse" {
            dir = if dir == "column" {
                "column-reverse"
            } else {
                "row-reverse"
            };
        }
        if orient == "vertical" || wdir == "reverse" {
            props.insert("flex-direction".into(), dir.into());
        }
    }
    if !props.contains_key("justify-content") {
        if let Some(v) = match pack.as_str() {
            "start" => Some("flex-start"),
            "center" => Some("center"),
            "end" => Some("flex-end"),
            "justify" => Some("space-between"),
            _ => None,
        } {
            props.insert("justify-content".into(), v.into());
        }
    }
    if !props.contains_key("align-items") {
        if let Some(v) = match align.as_str() {
            "start" => Some("flex-start"),
            "center" => Some("center"),
            "end" => Some("flex-end"),
            "baseline" => Some("baseline"),
            "stretch" => Some("stretch"),
            _ => None,
        } {
            props.insert("align-items".into(), v.into());
        }
    }
    if !props.contains_key("flex-grow") && grow.parse::<f32>().is_ok() {
        props.insert("flex-grow".into(), grow);
    }
    if !props.contains_key("order") && ordinal.parse::<i32>().is_ok() {
        props.insert("order".into(), ordinal);
    }
}

fn legacy_presentational_hints(
    dom: &Dom,
    node: NodeId,
    tag: &str,
    parent: Option<&ComputedStyle>,
    env: &StyleEnv,
    props: &mut HashMap<String, String>,
) {
    match tag {
        "body" | "table" | "tr" | "td" | "th" | "colgroup" | "col" | "thead" | "tbody"
        | "tfoot" => {
            hint(
                props,
                "background-color",
                hint_attr(dom, node, "bgcolor").and_then(legacy_color),
            );
        }
        _ => {}
    }
    match tag {
        "body" | "table" | "thead" | "tbody" | "tfoot" | "tr" | "td" | "th" => {
            // Attribut `background` (R-156, HTML Rendering) :
            // url d'arrière-plan → `background-image`.
            hint(
                props,
                "background-image",
                hint_attr(dom, node, "background").map(|v| format!("url(\"{}\")", v.trim())),
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
            // `align` (R-156) : left/right → `float` ;
            // `center` → marges auto (HTML Rendering).
            match hint_attr(dom, node, "align").map(|v| v.trim().to_ascii_lowercase()) {
                Some(v) if v == "left" || v == "right" => hint(props, "float", Some(v)),
                Some(v) if v == "center" => {
                    hint(props, "margin-left", Some("auto".to_string()));
                    hint(props, "margin-right", Some("auto".to_string()));
                }
                _ => {}
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
            // `nowrap` (R-156) → white-space (HTML Rendering).
            if hint_attr(dom, node, "nowrap").is_some() {
                hint(props, "white-space", Some("nowrap".to_string()));
            }
        }
        "body" => hint_body(dom, node, props),
        "a" | "area" => hint_link(dom, node, props),
        "img" => hint_img(dom, node, props),
        "hr" => hint_hr(dom, node, props),
        "br" => hint_br(dom, node, props),
        "font" => hint_font(dom, node, parent, env, props),
        _ => {}
    }
}

/// `<body>` présentationnel : `text` → `color` ;
/// marges historiques (R-157, HTML Rendering) :
/// `topmargin`/`leftmargin`/`rightmargin`/`bottommargin` →
/// côté homonyme ; `marginwidth`/`marginheight` → paires.
/// Le défaut UA `8px` est émis ici en hint : une vraie
/// règle `body{margin:8px}` primerait les attributs
/// (rang 0 < règle) — l'auteur reste prioritaire.
fn hint_body(dom: &Dom, node: NodeId, props: &mut HashMap<String, String>) {
    hint(
        props,
        "color",
        hint_attr(dom, node, "text").and_then(legacy_color),
    );
    let mw = dim(hint_attr(dom, node, "marginwidth"));
    let mh = dim(hint_attr(dom, node, "marginheight"));
    for (attr, prop, pair) in [
        ("topmargin", "margin-top", &mh),
        ("leftmargin", "margin-left", &mw),
        ("rightmargin", "margin-right", &mw),
        ("bottommargin", "margin-bottom", &mh),
    ] {
        hint(
            props,
            prop,
            dim(hint_attr(dom, node, attr))
                .or_else(|| pair.clone())
                .or_else(|| Some("8px".to_string())),
        );
    }
}

/// `a[href]`/`area[href]` (R-157) : `body[link]` → couleur
/// des liens, lue sur le body ancêtre (`:link` structurel).
/// Le défaut `#00e` est émis ici en hint — une vraie règle
/// UA primerait `body[link]` sinon ; l'auteur prime toujours
/// via `props` rempli. `vlink` documenté non-fait
/// (`:visited` toujours faux, vie privée — R-19) ; `alink`
/// exige le filage `:active` (NAV, journal).
fn hint_link(dom: &Dom, node: NodeId, props: &mut HashMap<String, String>) {
    if hint_attr(dom, node, "href").is_some() {
        hint(
            props,
            "color",
            ancestor_elem_attr(dom, node, "body", "link")
                .and_then(legacy_color)
                .or_else(|| Some("#00e".to_string())),
        );
    }
}

/// `<font>` (R-154 — signature du vieux web) : `face`/
/// `color`/`size` 1-7 + `±N` relatif à l'index table du
/// parent (§ font element, HTML Rendering).
fn hint_font(
    dom: &Dom,
    node: NodeId,
    parent: Option<&ComputedStyle>,
    env: &StyleEnv,
    props: &mut HashMap<String, String>,
) {
    hint(
        props,
        "font-family",
        hint_attr(dom, node, "face").map(|v| v.trim().to_string()),
    );
    hint(
        props,
        "color",
        hint_attr(dom, node, "color").and_then(legacy_color),
    );
    hint(
        props,
        "font-size",
        font_size_hint(hint_attr(dom, node, "size"), parent, env),
    );
}

/// `<basefont>` (R-193 — HTML 3.2, quirks seulement) :
/// `size`/`color`/`face` portent la **fonte de base** du
/// document — hints sur la racine (l'héritage propage à
/// tous les descendants, y compris l'index table des
/// `<font size=±N>` fils directs, désormais base-relatif
/// pour eux). Premier `<basefont>` en ordre document
/// (spec : s'applique au contenu suivant — approximation
/// uniforme sur tout le document, la plupart des corpus
/// legacy n'en déclare qu'un).
fn hint_basefont(dom: &Dom, props: &mut HashMap<String, String>) {
    if !matches!(dom.document_mode(), explorer_dom::DocumentMode::Quirks) {
        return;
    }
    let Some(bf) = first_elem_by_tag(dom, "basefont") else {
        return;
    };
    hint(
        props,
        "font-size",
        hint_attr(dom, bf, "size")
            .and_then(|v| v.trim().parse::<i64>().ok())
            .map(|n| {
                let i = usize::try_from(n.clamp(1, 7) - 1).unwrap_or(2);
                FONT_SIZE_KW[i].to_string()
            }),
    );
    hint(
        props,
        "color",
        hint_attr(dom, bf, "color").and_then(legacy_color),
    );
    hint(
        props,
        "font-family",
        hint_attr(dom, bf, "face").map(|v| v.trim().to_string()),
    );
}

/// Premier élément portant `tag` en ordre document (DFS) —
/// `None` si absent (`<basefont>` est rare).
fn first_elem_by_tag(dom: &Dom, tag: &str) -> Option<NodeId> {
    let mut stack: Vec<NodeId> = vec![dom.document()];
    while let Some(n) = stack.pop() {
        if dom.tag_name(n).is_ok_and(|t| t.eq_ignore_ascii_case(tag)) {
            return Some(n);
        }
        stack.extend(dom.children(n).into_iter().flatten());
    }
    None
}

/// `<img>` présentationnel (R-156) : `align` left/right →
/// `float`, autres valeurs → `vertical-align` (HTML
/// Rendering) ; `hspace`/`vspace` → marges ; `border` →
/// filet solide (le bleu des images-liées vient de `color`
/// héritée de `<a>` — `border-color` currentcolor).
fn hint_img(dom: &Dom, node: NodeId, props: &mut HashMap<String, String>) {
    match hint_attr(dom, node, "align").map(|v| v.trim().to_ascii_lowercase()) {
        Some(v) if v == "left" || v == "right" => hint(props, "float", Some(v)),
        Some(v) => {
            let va = match v.as_str() {
                "top" => "top",
                "middle" | "absmiddle" => "middle",
                "baseline" => "baseline",
                "texttop" => "text-top",
                _ => "bottom",
            };
            hint(props, "vertical-align", Some(va.to_string()));
        }
        None => {}
    }
    hint(props, "margin-left", dim(hint_attr(dom, node, "hspace")));
    hint(props, "margin-right", dim(hint_attr(dom, node, "hspace")));
    hint(props, "margin-top", dim(hint_attr(dom, node, "vspace")));
    hint(props, "margin-bottom", dim(hint_attr(dom, node, "vspace")));
    if let Some(bw) = dim(hint_attr(dom, node, "border")) {
        hint(props, "border-width", Some(bw));
        hint(props, "border-style", Some("solid".to_string()));
    }
}

/// `<hr>` présentationnel (R-156) : `align` → marges auto
/// par côté ; `noshade` → filet plein (vs `inset` UA) ;
/// `color` → `color` (la bordure UA est currentcolor).
fn hint_hr(dom: &Dom, node: NodeId, props: &mut HashMap<String, String>) {
    match hint_attr(dom, node, "align").map(|v| v.trim().to_ascii_lowercase()) {
        Some(v) if v == "left" => {
            hint(props, "margin-right", Some("auto".to_string()));
        }
        Some(v) if v == "right" => {
            hint(props, "margin-left", Some("auto".to_string()));
        }
        Some(v) if v == "center" => {
            hint(props, "margin-left", Some("auto".to_string()));
            hint(props, "margin-right", Some("auto".to_string()));
        }
        _ => {}
    }
    if hint_attr(dom, node, "noshade").is_some() {
        hint(props, "border-style", Some("solid".to_string()));
    }
    hint(
        props,
        "color",
        hint_attr(dom, node, "color").and_then(legacy_color),
    );
}

/// `br[clear]` (R-156, HTML Rendering) : `all` → `both`,
/// `left`/`right`/`none` → homonyme.
fn hint_br(dom: &Dom, node: NodeId, props: &mut HashMap<String, String>) {
    let lowered = hint_attr(dom, node, "clear").map(|v| v.trim().to_ascii_lowercase());
    let c = match lowered.as_deref() {
        Some("all") => Some("both"),
        Some(v @ ("left" | "right" | "none")) => Some(v),
        _ => None,
    };
    hint(props, "clear", c.map(str::to_string));
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
    containers: Option<&HashMap<NodeId, ContainerMeasure>>,
) -> Option<ComputedStyle> {
    let parent = match dom.parent(node) {
        Ok(Some(p)) if dom.element(p).is_ok() => p,
        _ => return None,
    };
    if let Some(s) = memo.get(&parent) {
        return Some(s.clone());
    }
    let grand = explorer_common::deep(|| {
        parent_computed(
            engine, dom, parent, env, state, memo, mcache, root_fs, containers,
        )
    });
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
        containers,
    );
    memo.insert(parent, s.clone());
    Some(s)
}

/// R-183 — index d'un côté logique → arête **réelle** selon
/// (mode propre, direction) ; puis arête réelle → nom lu
/// dans le repère de flux du parent (transposé si vertical).
/// `sideways-*` suit la table du vertical de base.
fn logical_edge(mode: crate::value::WritingMode, rtl: bool, side: usize) -> &'static str {
    // 0 = inline-start, 1 = inline-end (sens `direction`),
    // 2 = block-start, 3 = block-end (sens `block_reversed`).
    match side {
        0 | 1 => {
            let start = match (mode.is_vertical(), rtl) {
                (false, false) => "left",
                (false, true) => "right",
                (true, false) => "top",
                (true, true) => "bottom",
            };
            if side == 0 {
                start
            } else {
                match start {
                    "left" => "right",
                    "right" => "left",
                    "top" => "bottom",
                    _ => "top",
                }
            }
        }
        2 => match (mode.is_vertical(), mode.block_reversed()) {
            (false, _) => "top",
            (true, false) => "left",
            (true, true) => "right",
        },
        _ => match (mode.is_vertical(), mode.block_reversed()) {
            (false, _) => "bottom",
            (true, false) => "right",
            (true, true) => "left",
        },
    }
}

/// Arête **réelle** → nom de côté dans le repère du flux
/// parent (R-183) : en mode vertical, `x'` suit l'inline
/// réel (top→`left`, bottom→`right`) et le bloc s'inverse
/// selon `vertical-rl`/`vertical-lr`.
fn stored_side(real: &str, pmode: crate::value::WritingMode) -> &'static str {
    if !pmode.is_vertical() {
        return match real {
            "top" => "top",
            "bottom" => "bottom",
            "right" => "right",
            _ => "left",
        };
    }
    match real {
        "bottom" => "right",
        "left" => {
            if pmode.block_reversed() {
                "bottom"
            } else {
                "top"
            }
        }
        "right" => {
            if pmode.block_reversed() {
                "top"
            } else {
                "bottom"
            }
        }
        _ => "left",
    }
}

/// `*-inline-start`/`*-block-end`/`-size`/`inset-*`/rayons
/// logiques → nom du repère de flux (R-183). Composé :
/// `logique → réel` (mode propre + `direction`), `réel →
/// nom stocké` (mode du parent). Le physique déjà déclaré
/// n'est pas écrasé (perdant arbitraire mais documenté).
fn resolve_logical_props(
    props: &mut HashMap<String, String>,
    parent: Option<&ComputedStyle>,
    ranks: &HashMap<String, u8>,
) {
    use crate::value::parse_writing_mode;
    let own = props
        .get("writing-mode")
        .map_or(crate::value::WritingMode::default(), |v| {
            parse_writing_mode(v)
        });
    let rtl = props.get("direction").is_some_and(|v| v.trim() == "rtl");
    let pmode = parent
        .and_then(|p| p.get("writing-mode"))
        .map_or(crate::value::WritingMode::default(), parse_writing_mode);
    let same_axes = own.is_vertical() == pmode.is_vertical();
    for k in props.keys().cloned().collect::<Vec<_>>() {
        let Some(stored) = logical_name(&k, own, rtl, pmode, same_axes) else {
            continue;
        };
        // Le logique écrase le physique présent seulement
        // si son rang d'origine dépasse (auteur > UA —
        // `padding-inline-start` auteur bat `padding-left`
        // UA). À rang égal, le physique l'emporte —
        // comportement historique documenté (l'ordre fin
        // intra-élément n'est pas préservable en `HashMap`).
        let win = ranks
            .get(&k)
            .is_some_and(|r| *r > ranks.get(&stored).copied().unwrap_or(0));
        if !props.contains_key(&stored) || win {
            if let Some(v) = props.get(&k).cloned() {
                props.insert(stored, v);
            }
        }
    }
}

/// Nom logique → nom du repère de flux (R-183) — corps du
/// mapping extrait de [`resolve_logical_props`]. `None` =
/// nom non logique.
fn logical_name(
    k: &str,
    own: crate::value::WritingMode,
    rtl: bool,
    pmode: crate::value::WritingMode,
    same_axes: bool,
) -> Option<String> {
    let side_idx = |s: &str| -> Option<usize> {
        match s {
            "inline-start" => Some(0),
            "inline-end" => Some(1),
            "block-start" => Some(2),
            "block-end" => Some(3),
            _ => None,
        }
    };
    if let Some(rest) = k
        .strip_prefix("margin-")
        .or_else(|| k.strip_prefix("padding-"))
        .or_else(|| k.strip_prefix("inset-"))
        .and_then(side_idx)
    {
        return Some(if k.starts_with("inset-") {
            stored_side(logical_edge(own, rtl, rest), pmode).to_string()
        } else {
            format!(
                "{}-{}",
                k.split('-').next().unwrap_or_default(),
                stored_side(logical_edge(own, rtl, rest), pmode)
            )
        });
    }
    if let Some(rest) = k.strip_prefix("border-") {
        let mut it = rest.split('-');
        if let (Some(a), Some(b), Some("radius")) = (it.next(), it.next(), it.next()) {
            // `border-{s|e}{s|e}-radius` : coin logique —
            // premier = bloc, second = inline (spec).
            let ve = 2 + usize::from(a != "start");
            let he = usize::from(b != "start");
            let rv = logical_edge(own, rtl, ve);
            let rh = logical_edge(own, rtl, he);
            return Some(format!(
                "border-{}-{}-radius",
                stored_side(rv, pmode),
                stored_side(rh, pmode)
            ));
        }
        for (suf, idx) in [
            ("inline-start", 0usize),
            ("inline-end", 1),
            ("block-start", 2),
            ("block-end", 3),
        ] {
            if rest == suf {
                // Nu `border-{i,b}-{s,e}` → shorthand
                // `border-{side}` (expansion aval).
                return Some(format!(
                    "border-{}",
                    stored_side(logical_edge(own, rtl, idx), pmode)
                ));
            }
            if let Some(tail) = rest.strip_prefix(suf).and_then(|t| t.strip_prefix('-')) {
                return Some(format!(
                    "border-{}-{}",
                    stored_side(logical_edge(own, rtl, idx), pmode),
                    tail
                ));
            }
        }
        return None;
    }
    if let Some(rest) = k
        .strip_prefix("corner-")
        .and_then(|t| t.strip_suffix("-shape"))
    {
        // `corner-{s|e}{s|e}-shape` : coin logique — premier
        // = bloc, second = inline (css-borders-4 §3.8.1,
        // même convention que `border-{s|e}{s|e}-radius`).
        let toks: Vec<&str> = rest.split('-').collect();
        if let [a @ ("start" | "end"), b @ ("start" | "end")] = toks.as_slice() {
            let ve = 2 + usize::from(*a != "start");
            let he = usize::from(*b != "start");
            return Some(format!(
                "corner-{}-{}-shape",
                stored_side(logical_edge(own, rtl, ve), pmode),
                stored_side(logical_edge(own, rtl, he), pmode)
            ));
        }
        return None;
    }
    match k {
        "inline-size" | "min-inline-size" | "max-inline-size" => Some(format!(
            "{}{}",
            k.strip_suffix("inline-size").unwrap_or_default(),
            if same_axes { "width" } else { "height" }
        )),
        "block-size" | "min-block-size" | "max-block-size" => Some(format!(
            "{}{}",
            k.strip_suffix("block-size").unwrap_or_default(),
            if same_axes { "height" } else { "width" }
        )),
        _ => None,
    }
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
    "unicode-bidi",
    "font-variant",
    "font-variant-alternates",
    "font-variant-numeric",
    "font-feature-settings",
    "font-palette",
    "font-stretch",
    "font-size-adjust",
    "font-synthesis",
    "font-kerning",
    "font-weight",
    "letter-spacing",
    "line-height",
    "list-style",
    "list-style-image",
    "list-style-position",
    "list-style-type",
    "orphans",
    "pointer-events",
    "quotes",
    // css-ruby-1 §3.1 : `ruby-position` est héritée — une
    // valeur posée sur `<ruby>`/`rtc` atteint le `rt`.
    "ruby-position",
    "tab-size",
    "text-align",
    "text-decoration-skip-ink",
    "text-combine-upright",
    "text-emphasis",
    "text-emphasis-color",
    "text-emphasis-position",
    "text-emphasis-style",
    "text-spacing-trim",
    "hanging-punctuation",
    // css-text-4 §9/§7.3 (R-282) : `text-autospace` et
    // `text-justify` sont héritées.
    "text-autospace",
    "text-justify",
    "text-indent",
    "text-orientation",
    "text-transform",
    "text-wrap-mode",
    "text-wrap-style",
    "visibility",
    "writing-mode",
    "white-space",
    "white-space-collapse",
    "white-space-trim",
    "widows",
    "word-break",
    "word-spacing",
    "word-wrap",
    "overflow-wrap",
    "line-break",
    "hyphens",
];

/// R-209 — une valeur **déclarée** qui échoue à la lecture
/// typée (`length`/`number`) est comptée `css.value.unresolved`
/// — le blind spot restant après la validation syntaxique
/// (`width: 200` part plus tôt ; ici : `calc` mal formé,
/// décimale virgule `0,5em`, fonction inconnue…).
///
/// Filtre anti-bruit : seules les valeurs à chiffres/unités/
/// parenthèses comptent — les ident-keyword (`auto`,
/// `normal`, `none`) échouent aussi à `parse_length` mais
/// sont des valeurs **légales** que le consommateur gère à
/// côté ; les ignorer évite un Pareto de faux positifs.
fn census_unresolved(name: &str, raw: &str, failed: bool) {
    if !failed {
        return;
    }
    let numeric = raw
        .chars()
        .any(|c| c.is_ascii_digit() || c == '(' || c == '%');
    if numeric {
        explorer_common::census::bump(&format!("css.value.unresolved.{name}"));
    }
}

/// R-207 — clé census d'une déclaration auteur : nom
/// minuscule, custom properties collapsées en `--custom`
/// (cardinalité bornée — un site à 10⁴ variables uniques ne
/// doit pas gonfler la table). Le tri « supporté ou non »
/// est fait par le rapporteur qui croise `css.prop.*` avec
/// la liste des propriétés consommées — le compteur mesure
/// l'**usage**, pas un jugement.
fn census_prop(name: &str) {
    let n = name.trim();
    if n.is_empty() {
        return;
    }
    let key = if n.starts_with("--") {
        String::from("css.prop.--custom")
    } else {
        format!("css.prop.{}", n.to_ascii_lowercase())
    };
    explorer_common::census::bump(&key);
}

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
        | "word-spacing" | "white-space" | "unicode-bidi" => "normal",
        "text-align" => "start",
        "tab-size" => "8",
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
/// que sans moteur de script). La règle est `!important`
/// comme la spec : un style auteur `noscript { display:… }`
/// (mastodon `display:flex`, terrain) ne doit pas révéler
/// le fallback — l'important UA coiffe l'auteur normal ET
/// l'auteur important. Sans scripting il reste un
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
caption { display: table-caption }
[hidden], head, script, style, link, meta, title, template,
area, base, basefont, datalist, param, source, track,
frame { display: none }
details:not([open]) > :not(summary) { display: none }
dialog:not([open]) { display: none }
/* HTML Rendering §15.3.1 : un popover ferme ne s'affiche
   pas — :popover-open est toujours faux tant que
   showPopover() n'existe pas (vol 2026-10-26 : mega-menu
   github/tool-tips popover=manual peints deplies). */
[popover]:not(:popover-open) { display: none }
/* HTML §6.4.2 : un sous-arbre `inert` est exclu du hit-test —
   spec : « as if the pointer-events property were set to
   none » — mais PEINT normalement (le masquage vient des
   styles/scripts du site). Vol 71 santemag : drawer
   `inert`+`aria-hidden` peint ouvert volait les clics de la
   page dessous (§B-17). Limite : un `pointer-events:auto`
   auteur re-armerait un enfant alors que l'inertie est
   absolue — pattern rare, accepté au titre de la règle UA. */
[inert] { pointer-events: none }
audio:not([controls]) { display: none }
@media (scripting) { noscript { display: none !important } }
/* HTML Rendering — un `<slot>` ne génère aucune boîte :
   son contenu de repli est promu (R-204 — la
   distribution proper des nœuds assignés reste la borne
   Shadow R-96/R-205). */
slot { display: contents }
li { display: list-item }
ul, menu, dir { list-style-type: disc }
ul ul, ol ul, menu ul, dir ul { list-style-type: circle }
ul ul ul, ol ul ul, menu ul ul, dir ul ul { list-style-type: square }
ol { list-style-type: decimal }
/* HTML Rendering §14.3.9 — `type` de liste = règles UA
   attribut-sélecteur (pas un hint : l'auteur les bat).
   Mesuré : `ol[type=a]` de l'IETF rendait decimal. */
ol[type='1' i], li[type='1' i] { list-style-type: decimal }
ol[type='a' s], li[type='a' s] { list-style-type: lower-alpha }
ol[type='A' s], li[type='A' s] { list-style-type: upper-alpha }
ol[type='i' s], li[type='i' s] { list-style-type: lower-roman }
ol[type='I' s], li[type='I' s] { list-style-type: upper-roman }
ul[type='disc' i], li[type='disc' i] { list-style-type: disc }
ul[type='circle' i], li[type='circle' i] { list-style-type: circle }
ul[type='square' i], li[type='square' i] { list-style-type: square }
td, th { display: table-cell }
center { text-align: -webkit-center }
b, strong { font-weight: bold }
i, em, cite, dfn, var, address { font-style: italic }
u, ins { text-decoration: underline }
s, del, strike { text-decoration: line-through }
/* HTML Rendering : `<q>` génère ses guillemets via
   `content` + les mots-clés `open-quote`/`close-quote`
   (css-content-3 §6.1, R-276 — la paire vient de `quotes`,
   `auto` = latines usuelles). */
q::before { content: open-quote }
q::after { content: close-quote }
big { font-size: larger }
small, sub, sup { font-size: smaller }
nobr { white-space: nowrap }
sub { vertical-align: sub }
sup { vertical-align: super }
/* css-ruby-1 : vrais niveaux `display:ruby*` (le boxtree les
   approxime inline ; la pose de l'annotation au-dessus de la
   base est faite côté inline.rs — `ruby-position:over|under`).
   `rt` à demi fonte ; `rp` (parenthèses pleine largeur pour
   navigateurs sans ruby) est masqué — le rt est affiché. */
ruby { display: ruby }
rbc { display: ruby-base-container }
rb { display: ruby-base }
rtc { display: ruby-text-container }
rt { display: ruby-text; font-size: 50% }
rp { display: none }
abbr[title], acronym[title] { text-decoration: underline }
mark { background-color: yellow; color: black }
figure { margin-top: 1em; margin-bottom: 1em; margin-left: 40px; margin-right: 40px }
fieldset { border-width: 2px; border-style: groove; margin-left: 2px; margin-right: 2px; padding: 0.35em 0.75em 0.625em }
legend { padding-left: 2px; padding-right: 2px }
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
ul, ol, menu, dir, dl, dd { margin-top: 1em; margin-bottom: 1em }
ul, ol, menu, dir { padding-left: 40px }
dd { margin-left: 40px }
blockquote { margin: 1em 40px }
hr { border-top-width: 1px; border-style: inset }
a[href] { text-decoration: underline }
table { border-collapse: separate }
th { font-weight: bold; text-align: center }
input, textarea, select, button, img, svg, canvas { display: inline-block }
input, textarea {
  border: 1px solid #767676; padding: 1px
}
/* Les dimensions input/textarea vivent en hints
   (cols/rows/size — HTML Rendering) : une règle UA
   primerait les attributs de présentation. `min-height`
   (pas `height`) : un `height` UA comptait comme taille
   explicite et neutralisait `align-items:stretch` en
   flex (R-109 — zone morte mesurée sur github). */
input { min-height: 1.2em }
input, textarea, select { border: 1px solid #767676; background-color: #fff }
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

/// `light-dark(a, b)` → la branche claire ou sombre
/// (css-color-adjust-1 §2.4 — R-105). Résolution récursive
/// (un `light-dark` peut nicher dans une autre valeur), la
/// virgule de tête sépare les deux branches ; un argument
/// manquant conserve le texte source (dégradation).
/// R-250 — `font-variant-east-asian` : mots-clés → tags OT
/// (css-fonts-4 §6.5).
const FVE_TAGS: &[(&str, &str)] = &[
    ("full-width", "fwid"),
    ("proportional-width", "pwid"),
    ("jis78", "jp78"),
    ("jis83", "jp83"),
    ("jis90", "jp90"),
    ("jis04", "jp04"),
    ("simplified", "smpl"),
    ("traditional", "trad"),
    ("hojo", "hojo"),
    ("nlc-kanji", "nlck"),
    ("ruby", "ruby"),
];

/// R-241 — `font-variant-numeric` : mots-clés → tags OT
/// (css-fonts-4 §6.3).
const FVN_TAGS: &[(&str, &str)] = &[
    ("lining-nums", "lnum"),
    ("oldstyle-nums", "onum"),
    ("proportional-nums", "pnum"),
    ("tabular-nums", "tnum"),
    ("diagonal-fractions", "frac"),
    ("stacked-fractions", "afrc"),
    ("ordinal", "ordn"),
    ("slashed-zero", "zero"),
];

/// `font-variant-ligatures` : mots-clés → `(tag OT, valeur)`
/// (css-fonts-4 §6.4) — `none` coupe toutes les ligatures.
const FVL_TAGS: &[(&str, &[(&str, u32)])] = &[
    ("common-ligatures", &[("liga", 1), ("clig", 1)]),
    ("no-common-ligatures", &[("liga", 0), ("clig", 0)]),
    ("discretionary-ligatures", &[("dlig", 1)]),
    ("no-discretionary-ligatures", &[("dlig", 0)]),
    ("historical-ligatures", &[("hlig", 1)]),
    ("no-historical-ligatures", &[("hlig", 0)]),
    ("contextual", &[("calt", 1)]),
    ("no-contextual", &[("calt", 0)]),
    (
        "none",
        &[("liga", 0), ("clig", 0), ("dlig", 0), ("hlig", 0)],
    ),
];

/// `font-variant` shorthand : répartition des mots-clés par
/// longhand (css-fonts-4 §6.7). Les fonctions `stylistic()`
/// /`styleset()`… vont à `font-variant-alternates` brut.
const FV_CAPS: &[&str] = &[
    "small-caps",
    "all-small-caps",
    "petite-caps",
    "all-petite-caps",
    "unicase",
    "titling-caps",
];
const FV_EMOJI: &[&str] = &["emoji", "text", "unicode"];

impl StyleEngine {
    /// `font-variant` shorthand : chaque mot-clé est routé
    /// vers son longhand (css-fonts-4 §6.7) — les résolveurs
    /// `numeric`/`east-asian`/`ligatures`/`caps`/`position`
    /// consomment alors le résultat. `normal`/`none` et les
    /// formes fonctionnelles des alternates gardent leur
    /// valeur brute sur `font-variant` (lues par font.rs
    /// pour les caps, dégradées sinon). Un longhand déjà
    /// présent n'est pas écrasé (la cascade a tranché).
    fn expand_font_variant(map: &mut HashMap<NodeId, ComputedStyle>) {
        for st in map.values_mut() {
            let Some(v) = st.get("font-variant") else {
                continue;
            };
            let v = v.trim().to_string();
            if matches!(v.as_str(), "normal" | "inherit" | "initial" | "unset") {
                continue;
            }
            let mut numeric = Vec::new();
            let mut asian = Vec::new();
            let mut ligs = Vec::new();
            let mut alts = Vec::new();
            for kw in v.split_whitespace() {
                if FV_CAPS.contains(&kw) {
                    if st.get("font-variant-caps").is_none() {
                        st.set("font-variant-caps", kw);
                    }
                } else if matches!(kw, "sub" | "super") {
                    if st.get("font-variant-position").is_none() {
                        st.set("font-variant-position", kw);
                    }
                } else if FV_EMOJI.contains(&kw) {
                    if st.get("font-variant-emoji").is_none() {
                        st.set("font-variant-emoji", kw);
                    }
                } else if FVN_TAGS.iter().any(|(k, _)| *k == kw) {
                    numeric.push(kw);
                } else if FVE_TAGS.iter().any(|(k, _)| *k == kw) {
                    asian.push(kw);
                } else if FVL_TAGS.iter().any(|(k, _)| *k == kw) {
                    ligs.push(kw);
                } else if kw == "none" {
                    // `font-variant: none` = ligatures coupées
                    // (css-fonts-4 §6.7 : none ≡
                    // `font-variant-ligatures: none` + caps
                    // normal…).
                    ligs.push("none");
                } else if kw != "normal" {
                    // `historical-forms`, fonctions
                    // `styleset(`… — vers alternates brut.
                    alts.push(kw);
                }
            }
            let merge = |st: &mut ComputedStyle, prop: &str, kws: Vec<&str>| {
                if kws.is_empty() {
                    return;
                }
                let joined = match st.get(prop) {
                    Some(prev) if !prev.trim().is_empty() && prev.trim() != "normal" => {
                        format!("{} {}", prev.trim(), kws.join(" "))
                    }
                    _ => kws.join(" "),
                };
                st.set(prop, &joined);
            };
            merge(st, "font-variant-numeric", numeric);
            merge(st, "font-variant-east-asian", asian);
            merge(st, "font-variant-ligatures", ligs);
            if !alts.is_empty() && st.get("font-variant-alternates").is_none() {
                st.set("font-variant-alternates", &alts.join(" "));
            }
        }
    }

    /// `font-variant-ligatures` : mots-clés → réglages OT
    /// concaténés (css-fonts-4 §6.4) — les valeurs `0`
    /// coupent la feature, le parsing
    /// `font-feature-settings` les porte déjà.
    fn resolve_font_ligatures(map: &mut HashMap<NodeId, ComputedStyle>) {
        for st in map.values_mut() {
            let Some(v) = st.get("font-variant-ligatures") else {
                continue;
            };
            let mut settings: Vec<String> = st
                .get("font-feature-settings")
                .map(|s| s.split(',').map(str::to_owned).collect())
                .unwrap_or_default();
            for kw in v.split_whitespace() {
                if let Some((_, pairs)) = FVL_TAGS.iter().find(|(k, _)| *k == kw) {
                    for (tag, val) in *pairs {
                        settings.push(format!("\"{tag}\" {val}"));
                    }
                }
            }
            if !settings.is_empty() {
                st.set("font-feature-settings", &settings.join(", "));
            }
        }
    }

    /// R-250 — `font-variant-east-asian` : mots-clés → tags
    /// OT concaténés (css-fonts-4 §6.5).
    fn resolve_font_east_asian(map: &mut HashMap<NodeId, ComputedStyle>) {
        for st in map.values_mut() {
            let Some(v) = st.get("font-variant-east-asian") else {
                continue;
            };
            let mut settings: Vec<String> = st
                .get("font-feature-settings")
                .map(|s| s.split(',').map(str::to_owned).collect())
                .unwrap_or_default();
            for kw in v.split_whitespace() {
                if let Some(tag) = FVE_TAGS.iter().find(|(k, _)| *k == kw).map(|(_, t)| t) {
                    settings.push(format!("\"{tag}\" 1"));
                }
            }
            if !settings.is_empty() {
                st.set("font-feature-settings", &settings.join(", "));
            }
        }
    }

    /// R-231 — `@custom-media` : chaque `MediaQuery` du lot
    /// dont l'unique condition est un `--nom` enregistré est
    /// remplacée par le parse du query alias (un niveau —
    /// l'alias peut à son tour contenir un alias, mais la
    /// chaîne est bornée à une substitution).
    fn expand_custom_media(&self, queries: &[MediaQuery]) -> Vec<MediaQuery> {
        let mut out = Vec::new();
        for q in queries {
            let custom = q.conditions.len() == 1
                && q.conditions[0].1.is_empty()
                && q.conditions[0].0.starts_with("--")
                && self.custom_media.contains_key(&q.conditions[0].0);
            if custom {
                let alias = self.custom_media[&q.conditions[0].0].clone();
                let mut expanded = crate::parser::parse_media_queries(&alias);
                for e in &mut expanded {
                    e.negated = e.negated != q.negated;
                }
                out.extend(expanded);
            } else {
                out.push(q.clone());
            }
        }
        out
    }

    /// R-229 — `font-variant-alternates` : résout les noms
    /// enregistrés `@font-feature-values` en réglages
    /// `font-feature-settings` équivalents (concatenés) —
    /// la clé de famille vient de `font-family` calculé.
    /// `normal`/absent ne touche à rien.
    fn resolve_font_alternates(&self, map: &mut HashMap<NodeId, ComputedStyle>) {
        // R-239 — `font-palette: --nom` : le registre livre
        // la palette → `base-palette` normalisée en valeur
        // (`light`/`dark`/index — `normal` si absent).
        if !self.font_palettes.is_empty() {
            for st in map.values_mut() {
                let Some(name) = st.get("font-palette") else {
                    continue;
                };
                let Some(fp) = self.font_palettes.get(name.trim()) else {
                    continue;
                };
                let base = if fp.base_palette.is_empty() {
                    "normal".to_string()
                } else {
                    fp.base_palette.clone()
                };
                st.set("font-palette", &base);
            }
        }
        Self::expand_font_variant(map);
        // R-241 — `font-variant-numeric` : mots-clés → tags
        // OT équivalents (spec css-fonts-4 §6.3) — concaténés
        // aux `font-feature-settings` existants.
        for st in map.values_mut() {
            let Some(v) = st.get("font-variant-numeric") else {
                continue;
            };
            let mut settings: Vec<String> = st
                .get("font-feature-settings")
                .map(|s| s.split(',').map(str::to_owned).collect())
                .unwrap_or_default();
            for kw in v.split_whitespace() {
                if let Some(tag) = FVN_TAGS.iter().find(|(k, _)| *k == kw).map(|(_, t)| t) {
                    settings.push(format!("\"{tag}\" 1"));
                }
            }
            if !settings.is_empty() {
                st.set("font-feature-settings", &settings.join(", "));
            }
        }
        Self::resolve_font_ligatures(map);
        Self::resolve_font_east_asian(map);
        if self.font_feature_values.is_empty() {
            return;
        }
        for st in map.values_mut() {
            let Some(alt) = st.get("font-variant-alternates") else {
                continue;
            };
            if alt.trim() == "normal" {
                continue;
            }
            let fam = st
                .get("font-family")
                .and_then(|f| f.split(',').next())
                .map(|f| {
                    f.trim()
                        .trim_matches(|c: char| c == '"' || c as u32 == 0x27)
                        .to_ascii_lowercase()
                })
                .unwrap_or_default();
            let set = self
                .font_feature_values
                .get(&fam)
                .or_else(|| self.font_feature_values.values().next());
            let Some(set) = set else {
                continue;
            };
            // `annotation(alt) styleset(a, b) swash(s)` —
            // chaque fonction → noms → réglages OT.
            let mut out = Vec::new();
            let mut rest = alt.trim();
            while let Some(open) = rest.find('(') {
                let name_part = &rest[..open];
                let end = rest[open..].find(')').map(|e| open + e + 1);
                let Some(end) = end else { break };
                let inside = &rest[open + 1..end - 1];
                for nm in inside.split(',') {
                    let nm = nm
                        .trim()
                        .trim_matches(|c: char| c == '"' || c as u32 == 0x27)
                        .to_ascii_lowercase();
                    if let Some(fv) = set.get(&nm) {
                        for (tag, v) in &fv.settings {
                            out.push(format!("\"{tag}\" {v}"));
                        }
                    }
                }
                let _ = name_part;
                rest = rest[end..].trim();
            }
            if !out.is_empty() {
                st.set("font-feature-settings", &out.join(", "));
            }
        }
    }
}

fn resolve_light_dark(v: &str, dark: bool) -> String {
    // JS-C400 — arête ∝ imbrication `light-dark(` dans la
    // valeur (contrôlée par la feuille — §B-111) : segment tas.
    explorer_common::deep(|| resolve_light_dark_inner(v, dark))
}

fn resolve_light_dark_inner(v: &str, dark: bool) -> String {
    let mut out = String::with_capacity(v.len());
    let mut rest = v;
    while let Some(i) = rest.to_ascii_lowercase().find("light-dark(") {
        out.push_str(&rest[..i]);
        let open = i + "light-dark".len();
        // Parenthèse fermante assortie.
        let mut depth = 0i32;
        let mut close = None;
        for (j, ch) in rest.char_indices().skip_while(|(j, _)| *j <= open) {
            match ch {
                '(' => depth += 1,
                ')' if depth == 0 => {
                    close = Some(j);
                    break;
                }
                ')' => depth -= 1,
                _ => {}
            }
        }
        let Some(cj) = close else {
            out.push_str(&rest[i..]);
            return out;
        };
        let inner = resolve_light_dark(&rest[open + 1..cj], dark);
        let parts = split_top_commas_css(&inner);
        let pick = usize::from(dark);
        match parts.get(pick).or_else(|| parts.first()) {
            Some(c) if !c.trim().is_empty() => out.push_str(c.trim()),
            _ => out.push_str(&rest[i..=cj]),
        }
        rest = &rest[cj + 1..];
    }
    out.push_str(rest);
    out
}

/// Éclate sur les virgules hors parenthèses (`rgb(1,2,3)` et
/// fonctions internes protégées) — miroir du split gradient.
fn split_top_commas_css(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth = 0u32;
    let mut start = 0usize;
    for (i, ch) in s.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                out.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&s[start..]);
    out
}

mod color_mix {
    #![allow(clippy::pedantic)]
    use super::*;

    /// `color-mix(in s, c1 p1, c2 p2)` → `rgba(…)` (css-color-5
    /// §3, R-114 — ×1143 dans la feuille Tailwind). Rectangulaires
    /// prémultipliés : srgb, srgb-linear, xyz, lab, oklab ;
    /// polaires : lch, oklch, hsl (hue par le plus court chemin).
    /// Somme < 100 → alpha × somme (§3.3). Inconnu → brut.
    pub(crate) fn resolve_color_mix(v: &str) -> String {
        // JS-C400 — arête ∝ imbrication `color-mix(` (réaction
        // mutuelle via `mix_eval` — §B-111) : segment tas.
        explorer_common::deep(|| resolve_color_mix_inner(v))
    }

    fn resolve_color_mix_inner(v: &str) -> String {
        let mut out = String::with_capacity(v.len());
        let mut rest = v;
        while let Some(i) = rest.to_ascii_lowercase().find("color-mix(") {
            out.push_str(&rest[..i]);
            let open = i + "color-mix".len();
            let mut depth = 0i32;
            let mut close = None;
            for (j, ch) in rest.char_indices().skip_while(|(j, _)| *j <= open) {
                match ch {
                    '(' => depth += 1,
                    ')' if depth == 0 => {
                        close = Some(j);
                        break;
                    }
                    ')' => depth -= 1,
                    _ => {}
                }
            }
            let Some(cj) = close else {
                out.push_str(&rest[i..]);
                return out;
            };
            match mix_eval(&rest[open + 1..cj]) {
                Some(c) => out.push_str(&c),
                None => out.push_str(&rest[i..=cj]),
            }
            rest = &rest[cj + 1..];
        }
        out.push_str(rest);
        out
    }

    fn mix_eval(inner: &str) -> Option<String> {
        let resolved = resolve_color_mix(inner);
        let parts = split_top_commas_css(&resolved);
        if parts.len() != 3 {
            return None;
        }
        let space = parts[0]
            .trim()
            .strip_prefix("in")?
            .trim()
            .to_ascii_lowercase();
        let (c1, p1) = mix_operand(parts[1])?;
        let (c2, p2) = mix_operand(parts[2])?;
        let (p1, p2) = match (p1, p2) {
            (Some(a), None) => (a, 100.0 - a),
            (None, Some(b)) => (100.0 - b, b),
            (None, None) => (50.0, 50.0),
            (Some(a), Some(b)) => (a, b),
        };
        if p1 <= 0.0 || p2 <= 0.0 {
            let sole = if p1 <= 0.0 { c2 } else { c1 };
            return Some(fmt_rgba(
                f32::from(sole.r),
                f32::from(sole.g),
                f32::from(sole.b),
                f32::from(sole.a) / 255.0,
            ));
        }
        let sum = p1 + p2;
        let alpha_mul = if sum < 100.0 { sum / 100.0 } else { 1.0 };
        let (w1, w2) = (p1 / sum, p2 / sum);
        let a = (f32::from(c1.a) / 255.0).mul_add(w1, (f32::from(c2.a) / 255.0) * w2) * alpha_mul;
        Some(match space.as_str() {
            "srgb" | "srgb-linear" | "xyz" | "xyz-d65" | "lab" | "oklab" => {
                let (x1, y1, z1) = mix_to(&space, c1);
                let (x2, y2, z2) = mix_to(&space, c2);
                let (a1, a2) = (f32::from(c1.a) / 255.0, f32::from(c2.a) / 255.0);
                let d = a1.mul_add(w1, a2 * w2).max(f32::EPSILON);
                mix_from(
                    &space,
                    (x1 * a1).mul_add(w1, (x2 * a2) * w2) / d,
                    (y1 * a1).mul_add(w1, (y2 * a2) * w2) / d,
                    (z1 * a1).mul_add(w1, (z2 * a2) * w2) / d,
                    a,
                )
            }
            "hsl" | "lch" | "oklch" => {
                let (x1, y1, z1) = mix_polar(&space, c1);
                let (x2, y2, z2) = mix_polar(&space, c2);
                let dh = (z2 - z1 + 540.0).rem_euclid(360.0) - 180.0;
                mix_from_polar(
                    &space,
                    x1.mul_add(w1, x2 * w2),
                    y1.mul_add(w1, y2 * w2),
                    z1 + dh * w2,
                    a,
                )
            }
            _ => return None,
        })
    }

    fn mix_operand(s: &str) -> Option<(crate::value::Color, Option<f32>)> {
        let s = s.trim();
        let (c, p) = match s.rsplit_once(' ') {
            Some((head, tail)) if tail.ends_with('%') => {
                (head, tail.trim_end_matches('%').parse::<f32>().ok())
            }
            _ => (s, None),
        };
        Some((crate::value::parse_color(c)?, p))
    }

    fn mix_to(space: &str, c: crate::value::Color) -> (f32, f32, f32) {
        let (r, g, b) = (
            f32::from(c.r) / 255.0,
            f32::from(c.g) / 255.0,
            f32::from(c.b) / 255.0,
        );
        let (rl, gl, bl) = (srgb_linear(r), srgb_linear(g), srgb_linear(b));
        match space {
            "srgb" => (r, g, b),
            "srgb-linear" => (rl, gl, bl),
            "xyz" | "xyz-d65" => srgb_xyz(rl, gl, bl),
            "lab" => xyz_lab(srgb_xyz(rl, gl, bl)),
            _ => xyz_oklab(srgb_xyz(rl, gl, bl)),
        }
    }

    fn mix_from(space: &str, x: f32, y: f32, z: f32, a: f32) -> String {
        let (r, g, b) = match space {
            "srgb" => (x, y, z),
            "srgb-linear" => (srgb_gamma(x), srgb_gamma(y), srgb_gamma(z)),
            "xyz" | "xyz-d65" => {
                let (rl, gl, bl) = xyz_srgb(x, y, z);
                (srgb_gamma(rl), srgb_gamma(gl), srgb_gamma(bl))
            }
            "lab" => {
                let (rl, gl, bl) =
                    xyz_srgb(lab_xyz(x, y, z).0, lab_xyz(x, y, z).1, lab_xyz(x, y, z).2);
                (srgb_gamma(rl), srgb_gamma(gl), srgb_gamma(bl))
            }
            _ => {
                let (rl, gl, bl) = oklab_xyz(x, y, z);
                (srgb_gamma(rl), srgb_gamma(gl), srgb_gamma(bl))
            }
        };
        fmt_rgba(r, g, b, a)
    }

    fn mix_polar(space: &str, c: crate::value::Color) -> (f32, f32, f32) {
        let (r, g, b) = (
            f32::from(c.r) / 255.0,
            f32::from(c.g) / 255.0,
            f32::from(c.b) / 255.0,
        );
        let (rl, gl, bl) = (srgb_linear(r), srgb_linear(g), srgb_linear(b));
        match space {
            "hsl" => rgb_hsl(r, g, b),
            "lch" => lab_lch(xyz_lab(srgb_xyz(rl, gl, bl))),
            _ => lab_lch(xyz_oklab(srgb_xyz(rl, gl, bl))),
        }
    }

    fn mix_from_polar(space: &str, x: f32, y: f32, z: f32, a: f32) -> String {
        let (r, g, b) = match space {
            "hsl" => hsl_rgb(x, y, z),
            "lch" => {
                let (rl, gl, bl) = {
                    let t = lab_xyz(lch_lab(x, y, z).0, lch_lab(x, y, z).1, lch_lab(x, y, z).2);
                    xyz_srgb(t.0, t.1, t.2)
                };
                (srgb_gamma(rl), srgb_gamma(gl), srgb_gamma(bl))
            }
            _ => {
                let (rl, gl, bl) = oklab_xyz(x, y * z.to_radians().cos(), y * z.to_radians().sin());
                (srgb_gamma(rl), srgb_gamma(gl), srgb_gamma(bl))
            }
        };
        fmt_rgba(r, g, b, a)
    }

    fn fmt_rgba(r: f32, g: f32, b: f32, a: f32) -> String {
        let c = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        let al = a.clamp(0.0, 1.0);
        if (al - 1.0).abs() < f32::EPSILON {
            format!("rgb({},{},{})", c(r), c(g), c(b))
        } else {
            format!("rgba({},{},{},{:.3})", c(r), c(g), c(b), al)
        }
    }

    fn srgb_linear(v: f32) -> f32 {
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    }

    fn srgb_gamma(v: f32) -> f32 {
        if v <= 0.003_130_8 {
            12.92 * v
        } else {
            1.055 * v.powf(1.0 / 2.4) - 0.055
        }
    }

    fn srgb_xyz(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
        (
            r.mul_add(0.412_390_8, g.mul_add(0.357_584_34, b * 0.180_480_8)),
            r.mul_add(0.212_639, g.mul_add(0.715_168_7, b * 0.072_192_32)),
            r.mul_add(0.019_330_82, g.mul_add(0.119_194_78, b * 0.950_532_2)),
        )
    }

    fn xyz_srgb(x: f32, y: f32, z: f32) -> (f32, f32, f32) {
        (
            x.mul_add(3.240_97, y.mul_add(-1.537_383_2, z * -0.498_610_76)),
            x.mul_add(-0.969_243_64, y.mul_add(1.875_967_5, z * 0.041_555_06)),
            x.mul_add(0.055_630_08, y.mul_add(-0.203_976_96, z * 1.056_971_5)),
        )
    }
    fn xyz_lab((x, y, z): (f32, f32, f32)) -> (f32, f32, f32) {
        let f = |t: f32| {
            if t > 0.008_856 {
                t.cbrt()
            } else {
                (841.0 / 108.0) * t + 4.0 / 29.0
            }
        };
        let (fx, fy, fz) = (f(x / 0.95047), f(y), f(z / 1.08883));
        (116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz))
    }

    fn lab_xyz(l: f32, a: f32, b: f32) -> (f32, f32, f32) {
        let fy = (l + 16.0) / 116.0;
        let f = |t: f32| {
            if t > 6.0 / 29.0 {
                t * t * t
            } else {
                (108.0 / 841.0) * (t - 4.0 / 29.0)
            }
        };
        (
            f(fy + a / 500.0) * 0.95047,
            f(fy),
            f(fy - b / 200.0) * 1.08883,
        )
    }

    fn lab_lch((l, a, b): (f32, f32, f32)) -> (f32, f32, f32) {
        (l, a.hypot(b), b.atan2(a).to_degrees().rem_euclid(360.0))
    }

    fn lch_lab(l: f32, c: f32, h: f32) -> (f32, f32, f32) {
        (l, c * h.to_radians().cos(), c * h.to_radians().sin())
    }

    fn xyz_oklab((x, y, z): (f32, f32, f32)) -> (f32, f32, f32) {
        let l = (0.818_933 * x + 0.361_866_7 * y - 0.128_859_71 * z).cbrt();
        let m = (0.032_984_54 * x + 0.929_311_9 * y + 0.036_145_64 * z).cbrt();
        let s = (0.048_200_3 * x + 0.264_366_27 * y + 0.633_851_7 * z).cbrt();
        (
            0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_05 * s,
            1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s,
            0.025_904_04 * l + 0.782_771_77 * m - 0.808_675_77 * s,
        )
    }

    fn oklab_xyz(l: f32, a: f32, b: f32) -> (f32, f32, f32) {
        let l2 = (l + 0.396_337_78 * a + 0.215_803_76 * b).powi(3);
        let m2 = (l - 0.105_561_346 * a - 0.063_854_17 * b).powi(3);
        let s2 = (l - 0.089_484_18 * a - 1.291_485_5 * b).powi(3);
        (
            1.227_014 * l2 - 0.557_799_8 * m2 + 0.281_256_15 * s2,
            -0.040_580_18 * l2 + 1.112_256_9 * m2 - 0.071_676_6 * s2,
            -0.076_381_28 * l2 - 0.421_481_98 * m2 + 1.586_163_3 * s2,
        )
    }

    fn rgb_hsl(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
        let (max, min) = (r.max(g).max(b), r.min(g).min(b));
        let l = f32::midpoint(max, min);
        if (max - min).abs() < f32::EPSILON {
            return (l, 0.0, 0.0);
        }
        let d = max - min;
        let s = d / (1.0 - (2.0 * l - 1.0).abs()).max(f32::EPSILON);
        let h = if (max - r).abs() < f32::EPSILON {
            60.0 * ((g - b) / d).rem_euclid(6.0)
        } else if (max - g).abs() < f32::EPSILON {
            60.0 * ((b - r) / d + 2.0)
        } else {
            60.0 * ((r - g) / d + 4.0)
        };
        (l, s, h)
    }

    fn hsl_rgb(l: f32, s: f32, h: f32) -> (f32, f32, f32) {
        let h = h.rem_euclid(360.0);
        let f = |n: f32| {
            let k = (n + h / 30.0).rem_euclid(12.0);
            l - s * l.min(1.0 - l) * k.min(4.0).min(9.0 - k).clamp(-1.0, 1.0)
        };
        (f(0.0), f(8.0), f(4.0))
    }
}

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
