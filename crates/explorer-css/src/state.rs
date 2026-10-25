// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! État structurel des contrôles de formulaire, évalué sur le
//! DOM (R-100 — répond à la demande journalisée 2026-10-19,
//! étape 54 NAV).
//!
//! Les pseudo-classes « état de contrôle » sont évaluables
//! **structurellement** — leur valeur se lit dans l'arbre
//! (`disabled`, `checked`, `required`, `value`, `pattern`…),
//! pas dans l'interaction : le callback `state` de
//! `compute_with_state` ne sert alors qu'aux pseudos vraiment
//! dynamiques (`:hover`, `:focus*`…). Référence : HTML
//! §4.10.19 (contraintes), §4.10.21.2 (pseudo-classes), et
//! selectors-4 §14.
//!
//! Simplifications documentées (navigateur tolérant, jamais de
//! promotion silencieuse) :
//! - `pattern` **n'est pas évalué** — il exige un moteur
//!   `RegExp` JavaScript ; la contrainte est traitée satisfaite ;
//! - `min`/`max`/`step` ne sont évalués que pour
//!   `type=number|range` (les types date passent) ;
//! - `type=email` est vérifié par la grammaire « willful
//!   violation » de la spec ; `type=url` exige un schéma
//!   absolu `alpha [alphanum + - .]* ':'` ;
//! - `minlength`/`maxlength` ne s'appliquent qu'à une valeur
//!   non vide (le drapeau « dirty » de la spec n'existe pas
//!   encore — approximation bornée) ;
//! - `select[required]` échoue si l'option sélectionnée est
//!   la *placeholder label option* (première `option` à
//!   `value=""`), approximation de la spec ;
//! - `:enabled` ne couvre que les contrôles de formulaire
//!   (les liens activables ne matchent pas — comportement
//!   navigateur réel mesuré, pas la lettre de selectors-4).

use explorer_dom::{Dom, NodeId};

/// Tag de l'élément (`""` hors élément).
fn tag(dom: &Dom, node: NodeId) -> &str {
    dom.tag_name(node).unwrap_or("")
}

/// Valeur d'attribut (`None` hors élément/absent).
fn attr(dom: &Dom, node: NodeId, name: &str) -> Option<String> {
    dom.attribute(node, name).ok().flatten()
}

fn has_attr(dom: &Dom, node: NodeId, name: &str) -> bool {
    attr(dom, node, name).is_some()
}

/// Le nœud est-il un élément du document.
fn is_element(dom: &Dom, node: NodeId) -> bool {
    dom.element(node).is_ok()
}

/// Type d'`<input>` normalisé (`"text"` absent/inconnu —
/// spec : tout type inconnu retombe sur Text).
fn input_type(dom: &Dom, node: NodeId) -> String {
    let t = attr(dom, node, "type")
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    match t.as_str() {
        // Types connus du moteur de contrôles.
        "text" | "search" | "tel" | "url" | "email" | "password" | "date" | "month" | "week"
        | "time" | "datetime-local" | "number" | "range" | "color" | "checkbox" | "radio"
        | "file" | "hidden" | "image" | "submit" | "reset" | "button" => t,
        _ => "text".to_owned(),
    }
}

/// Types d'`input` textuels (la valeur est une chaîne libre —
/// `minlength`/`maxlength`/`pattern`/`required` s'appliquent).
fn is_textual(ty: &str) -> bool {
    matches!(ty, "text" | "search" | "tel" | "url" | "email" | "password")
}

/// Un ancêtre `fieldset[disabled]` dont le nœud n'est pas dans
/// le **premier** `<legend>` (HTML §4.10.19 : les descendants
/// d'un fieldset désactivé sont désactivés, sauf ceux de sa
/// première légende).
fn inside_disabled_fieldset(dom: &Dom, node: NodeId) -> bool {
    let mut cur = dom.parent(node).ok().flatten();
    while let Some(n) = cur {
        if tag(dom, n) == "fieldset" && has_attr(dom, n, "disabled") {
            let in_first_legend = first_child_tag(dom, n, "legend")
                .is_some_and(|l| l == node || dom.is_ancestor(l, node).unwrap_or(false));
            if !in_first_legend {
                return true;
            }
        }
        cur = dom.parent(n).ok().flatten();
    }
    false
}

/// Premier enfant élément de `node` portant `name`.
fn first_child_tag(dom: &Dom, node: NodeId, name: &str) -> Option<NodeId> {
    dom.children(node)
        .ok()?
        .into_iter()
        .find(|c| tag(dom, *c) == name)
}

/// Élément réellement désactivé (`:disabled`) : contrôle
/// portant `disabled`, `fieldset[disabled]`, ou contrôle
/// descendant d'un fieldset désactivé (hors première
/// `legend`).
pub(crate) fn is_disabled(dom: &Dom, node: NodeId) -> bool {
    if !is_element(dom, node) {
        return false;
    }
    match tag(dom, node) {
        "button" | "input" | "select" | "textarea" | "optgroup" | "option" | "fieldset" => {}
        _ => return false,
    }
    if has_attr(dom, node, "disabled") {
        return true;
    }
    if tag(dom, node) == "fieldset" {
        return false; // fieldset n'hérite pas du disabled d'un ancêtre
    }
    inside_disabled_fieldset(dom, node)
}

/// `:enabled` — contrôle de formulaire non désactivé
/// (approximation navigateur : liens non inclus).
pub(crate) fn is_enabled(dom: &Dom, node: NodeId) -> bool {
    matches!(
        tag(dom, node),
        "button" | "input" | "select" | "textarea" | "optgroup" | "option"
    ) && !is_disabled(dom, node)
}

/// `:checked` — `input[checkbox|radio][checked]` ou
/// `option[selected]`.
pub(crate) fn is_checked(dom: &Dom, node: NodeId) -> bool {
    match tag(dom, node) {
        "input" => {
            matches!(input_type(dom, node).as_str(), "checkbox" | "radio")
                && has_attr(dom, node, "checked")
        }
        "option" => has_attr(dom, node, "selected"),
        _ => false,
    }
}

/// Élément pouvant porter `required` (§4.10.19.4 : input hors
/// types exclus — `checkbox`/`radio`/`file` acceptent
/// `required`, pas `hidden`/`range`/`color`/boutons — plus
/// `select` et `textarea`).
fn required_capable(dom: &Dom, node: NodeId) -> bool {
    match tag(dom, node) {
        "select" | "textarea" => true,
        "input" => !matches!(
            input_type(dom, node).as_str(),
            "hidden" | "image" | "reset" | "button" | "submit" | "range" | "color"
        ),
        _ => false,
    }
}

/// `:required` — élément capable portant `required`.
pub(crate) fn is_required(dom: &Dom, node: NodeId) -> bool {
    required_capable(dom, node) && has_attr(dom, node, "required")
}

/// `:optional` — élément qui *pourrait* être requis et ne
/// l'est pas.
pub(crate) fn is_optional(dom: &Dom, node: NodeId) -> bool {
    required_capable(dom, node) && !has_attr(dom, node, "required")
}

/// `:read-write` — `input`/`textarea` éditable (ni `readonly`
/// ni désactivé), ou élément sous `contenteditable` actif.
pub(crate) fn is_read_write(dom: &Dom, node: NodeId) -> bool {
    if !is_element(dom, node) {
        return false;
    }
    if matches!(tag(dom, node), "input" | "textarea")
        && !has_attr(dom, node, "readonly")
        && !is_disabled(dom, node)
    {
        return true;
    }
    // contenteditable : valeur autre que "false"/"plaintext-only
    // négative… la spec = true pour `""`/`true`/héritage ; on
    // traite l'attribut présent et ≠ "false" comme éditable, en
    // remontant les ancêtres (l'édition hérite).
    let mut cur = Some(node);
    while let Some(n) = cur {
        if let Some(v) = attr(dom, n, "contenteditable") {
            let v = v.trim().to_ascii_lowercase();
            return v.is_empty() || v == "true";
        }
        cur = dom.parent(n).ok().flatten();
    }
    false
}

/// `:placeholder-shown` — `input` avec `placeholder` non vide
/// et valeur vide.
pub(crate) fn placeholder_shown(dom: &Dom, node: NodeId) -> bool {
    tag(dom, node) == "input"
        && attr(dom, node, "placeholder").is_some_and(|p| !p.is_empty())
        && attr(dom, node, "value").is_none_or(|v| v.is_empty())
}

/// Candidat à la validation de contrainte (§4.10.19.2) :
/// `input` hors types bouton/caché, `select`, `textarea` — ni
/// désactivé, ni `readonly`, ni descendant de `datalist`.
fn is_validation_candidate(dom: &Dom, node: NodeId) -> bool {
    match tag(dom, node) {
        "select" | "textarea" => {}
        "input" => {
            if matches!(
                input_type(dom, node).as_str(),
                "hidden" | "reset" | "button" | "submit" | "image"
            ) {
                return false;
            }
        }
        _ => return false,
    }
    if is_disabled(dom, node)
        || (matches!(tag(dom, node), "input" | "textarea") && has_attr(dom, node, "readonly"))
    {
        return false;
    }
    // Descendant de <datalist> → barré.
    let mut cur = dom.parent(node).ok().flatten();
    while let Some(n) = cur {
        if tag(dom, n) == "datalist" {
            return false;
        }
        cur = dom.parent(n).ok().flatten();
    }
    true
}

/// Valeur de contrôle effective : attribut `value` pour
/// `input`/`textarea` (la session miroir y écrit l'état
/// courant), texte de l'option pour `option` sans `value`.
fn control_value(dom: &Dom, node: NodeId) -> String {
    match tag(dom, node) {
        "option" => attr(dom, node, "value")
            .or_else(|| dom.text_content(node).ok())
            .unwrap_or_default(),
        _ => attr(dom, node, "value").unwrap_or_default(),
    }
}

/// Longueur en unités UTF-16 (spec §4.10.19.4 — `maxlength`
/// compte des code units, pas des octets).
fn utf16_len(s: &str) -> usize {
    s.chars().map(char::len_utf16).sum()
}

/// `valueMissing` — l'élément requis n'a pas de valeur.
fn fails_required(dom: &Dom, node: NodeId) -> bool {
    if !has_attr(dom, node, "required") {
        return false;
    }
    match tag(dom, node) {
        "input" => match input_type(dom, node).as_str() {
            "checkbox" | "radio" => !has_attr(dom, node, "checked"),
            _ => control_value(dom, node).is_empty(),
        },
        "textarea" => control_value(dom, node).is_empty(),
        "select" => {
            // Placeholder label option : la première `option`
            // à valeur vide sélectionnée = non rempli (approx).
            selected_option_value(dom, node).is_none_or(|v| v.is_empty())
        }
        _ => false,
    }
}

/// Valeur de l'`option` sélectionnée d'un `select` (première
/// `option[selected]` en ordre d'arbre, sinon la première
/// `option` — sélection par défaut spec).
fn selected_option_value(dom: &Dom, node: NodeId) -> Option<String> {
    let kids = dom.children(node).ok()?;
    let mut first: Option<NodeId> = None;
    for k in kids {
        if tag(dom, k) == "optgroup" {
            if let Ok(grand) = dom.children(k) {
                for g in grand {
                    if tag(dom, g) == "option" {
                        first.get_or_insert(g);
                        if has_attr(dom, g, "selected") {
                            return Some(control_value(dom, g));
                        }
                    }
                }
            }
        } else if tag(dom, k) == "option" {
            first.get_or_insert(k);
            if has_attr(dom, k, "selected") {
                return Some(control_value(dom, k));
            }
        }
    }
    first.map(|f| control_value(dom, f))
}

/// `typeMismatch` pour `email`/`url` — valeur non vide qui ne
/// satisfait pas la grammaire du type. Valeur vide = pas de
/// mismatch (le vide n'est manquant que via `required`).
fn fails_type(dom: &Dom, node: NodeId) -> bool {
    if tag(dom, node) != "input" {
        return false;
    }
    let ty = input_type(dom, node);
    let v = control_value(dom, node);
    if v.is_empty() {
        return false;
    }
    match ty.as_str() {
        "email" => !valid_email(&v),
        "url" => !valid_absolute_url(&v),
        "number" | "range" => v.trim().parse::<f64>().is_err(),
        _ => false,
    }
}

/// Grammaire `email` de la spec (willful violation) :
/// `atext+ @ label ( "." label )*`, label =
/// alphanum puis ≤62 `[alphanum -]`.
fn valid_email(v: &str) -> bool {
    let Some((local, domain)) = v.split_once('@') else {
        return false;
    };
    if domain.contains('@') {
        return false;
    }
    let atext = |c: char| {
        c.is_ascii_alphanumeric()
            || matches!(
                c,
                '!' | '#'
                    | '$'
                    | '%'
                    | '&'
                    | '\''
                    | '*'
                    | '+'
                    | '/'
                    | '='
                    | '?'
                    | '^'
                    | '_'
                    | '`'
                    | '{'
                    | '|'
                    | '}'
                    | '~'
                    | '-'
                    | '.'
            )
    };
    if local.is_empty() || !local.chars().all(atext) {
        return false;
    }
    let label = |l: &str| {
        !l.is_empty()
            && l.len() <= 63
            && l.chars().next().is_some_and(|c| c.is_ascii_alphanumeric())
            && l.chars().last().is_some_and(|c| c.is_ascii_alphanumeric())
            && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    };
    domain.split('.').all(label)
}

/// URL absolue minimale : `scheme:` + reste non vide (spec :
/// validation syntaxique permissive — l'URL complète est
/// l'affaire de `explorer-net`, pas du validateur).
fn valid_absolute_url(v: &str) -> bool {
    let Some(colon) = v.find(':') else {
        return false;
    };
    let scheme = &v[..colon];
    !scheme.is_empty()
        && scheme
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic())
        && scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
        && !v[colon + 1..].is_empty()
}

/// `rangeUnderflow`/`rangeOverflow`/`stepMismatch` pour
/// `type=number|range` (seuls types numériques évalués).
fn fails_range(dom: &Dom, node: NodeId) -> bool {
    if tag(dom, node) != "input" || !matches!(input_type(dom, node).as_str(), "number" | "range") {
        return false;
    }
    let v = control_value(dom, node);
    if v.is_empty() {
        return false;
    }
    let Ok(val) = v.trim().parse::<f64>() else {
        return false; // typeMismatch déjà couvert par fails_type
    };
    if attr(dom, node, "min")
        .and_then(|m| m.trim().parse::<f64>().ok())
        .is_some_and(|min| val < min)
    {
        return true;
    }
    if attr(dom, node, "max")
        .and_then(|m| m.trim().parse::<f64>().ok())
        .is_some_and(|max| val > max)
    {
        return true;
    }
    // stepMismatch : (val − base) multiple de step (base = min
    // ou défaut `range`/`number` → 1 ; `step=any` saute).
    if let Some(step) = attr(dom, node, "step")
        .and_then(|s| s.trim().parse::<f64>().ok())
        .filter(|s| *s > 0.0)
    {
        let base = attr(dom, node, "min")
            .and_then(|m| m.trim().parse::<f64>().ok())
            .unwrap_or(1.0);
        let k = (val - base) / step;
        // Tolérance flottante (convention ulp du layout).
        if (k - k.round()).abs() > 1e-6 {
            return true;
        }
    }
    false
}

/// `tooShort`/`tooLong` — longueur UTF-16 bornée par
/// `minlength`/`maxlength` (types textuels + textarea ;
/// approximation : appliqué seulement à valeur non vide, faute
/// de drapeau « dirty »).
fn fails_length(dom: &Dom, node: NodeId) -> bool {
    let textual = match tag(dom, node) {
        "textarea" => true,
        "input" => is_textual(input_type(dom, node).as_str()),
        _ => false,
    };
    if !textual {
        return false;
    }
    let v = control_value(dom, node);
    if v.is_empty() {
        return false;
    }
    let len = utf16_len(&v);
    if attr(dom, node, "minlength")
        .and_then(|m| m.trim().parse::<usize>().ok())
        .is_some_and(|min| len < min)
    {
        return true;
    }
    attr(dom, node, "maxlength")
        .and_then(|m| m.trim().parse::<usize>().ok())
        .is_some_and(|max| len > max)
}

/// Contraintes satisfaites pour un candidat donné. `pattern`
/// (`RegExp` JS) documenté non évalué — voir en-tête du module.
fn satisfies_constraints(dom: &Dom, node: NodeId) -> bool {
    !fails_required(dom, node)
        && !fails_type(dom, node)
        && !fails_range(dom, node)
        && !fails_length(dom, node)
}

/// `:valid` sur un candidat. Pour `form`/`fieldset` : aucun
/// contrôle candidat descendant invalide (un form sans
/// contrôle est valide — comportement navigateur).
pub(crate) fn is_valid(dom: &Dom, node: NodeId) -> bool {
    if !is_element(dom, node) {
        return false;
    }
    match tag(dom, node) {
        "form" | "fieldset" => !any_descendant_invalid(dom, node),
        _ => is_validation_candidate(dom, node) && satisfies_constraints(dom, node),
    }
}

/// `:invalid` — candidat dont une contrainte échoue, ou
/// `form`/`fieldset` contenant un candidat invalide.
pub(crate) fn is_invalid(dom: &Dom, node: NodeId) -> bool {
    if !is_element(dom, node) {
        return false;
    }
    match tag(dom, node) {
        "form" | "fieldset" => any_descendant_invalid(dom, node),
        _ => is_validation_candidate(dom, node) && !satisfies_constraints(dom, node),
    }
}

/// Un descendant candidat de `node` échoue-t-il une contrainte.
fn any_descendant_invalid(dom: &Dom, node: NodeId) -> bool {
    let Ok(kids) = dom.children(node) else {
        return false;
    };
    kids.iter().any(|k| {
        (is_validation_candidate(dom, *k) && !satisfies_constraints(dom, *k))
            || explorer_common::deep(|| any_descendant_invalid(dom, *k))
    })
}
