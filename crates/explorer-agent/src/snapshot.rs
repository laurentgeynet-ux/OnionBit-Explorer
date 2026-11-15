// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Snapshot E3 — l'arbre **sémantique** : rôle dérivé de
//! `role=` ARIA puis du tag affiné par `type`, nom accessible
//! (`aria-label` → `alt` → `title` → texte visible borné →
//! `id` → classe → tag), états natifs (`focus`, `disabled`,
//! `checked`, `expanded`, `editable`, `link`, `selected`,
//! `required`), `depth` réelle — joints à la géométrie par
//! index arène. Sert aussi la mémoire des émissions pour le
//! diff `{since}` ([`diff`]).

use std::collections::HashMap;

use explorer_core::SnapshotEntry;
use explorer_core::{GeometryReport, TabId};

use crate::proto::{ElementRef, NodeState, Rect, Role, SnapshotNode};
use crate::state::HandleTable;

/// Plafond du nom accessible — contrat schéma (120 chars).
const NAME_CAP: usize = 120;

/// `role=` ARIA → [`Role`] — sous-ensemble mappé, le reste
/// est conservé en `Other(role)` (jamais perdu, jamais
/// inventé).
fn aria_role(role: &str) -> Role {
    match role {
        "link" => Role::Link,
        "button" => Role::Button,
        "checkbox" => Role::Checkbox,
        "radio" => Role::Radio,
        "textbox" | "searchbox" => Role::Textbox,
        "combobox" | "listbox" => Role::Combobox,
        "heading" => Role::Heading,
        "img" | "image" => Role::Img,
        "navigation" => Role::Navigation,
        "form" => Role::Form,
        "listitem" => Role::ListItem,
        "table" | "grid" => Role::Table,
        "main" => Role::Main,
        "option" => Role::Other("option".to_string()),
        "list" => Role::Other("list".to_string()),
        other => Role::Other(other.chars().take(40).collect()),
    }
}

/// Tag + `type` → rôle natif (priorité après `role=`).
fn tag_role(e: &SnapshotEntry) -> Role {
    match e.tag.as_str() {
        // `<a>` n'est un lien qu'avec `href` — une ancre nue
        // reste un conteneur.
        "a" if e.attr("href").is_some() => Role::Link,
        "button" => Role::Button,
        "input" => match e.attr("type").unwrap_or("text") {
            "checkbox" => Role::Checkbox,
            "radio" => Role::Radio,
            "button" | "submit" | "reset" | "image" => Role::Button,
            "hidden" | "range" | "color" | "file" => Role::Other("input".to_string()),
            _ => Role::Textbox,
        },
        "textarea" => Role::Textbox,
        "select" | "datalist" => Role::Combobox,
        "option" => Role::Other("option".to_string()),
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => Role::Heading,
        "img" | "picture" => Role::Img,
        "nav" => Role::Navigation,
        "form" => Role::Form,
        "li" => Role::ListItem,
        "ul" | "ol" => Role::Other("list".to_string()),
        "table" => Role::Table,
        "main" => Role::Main,
        _ => Role::Generic,
    }
}

/// Rôle d'une entrée — `role=` ARIA d'abord (le document
/// déclare), la sémantique native ensuite.
fn role_of(e: &SnapshotEntry) -> Role {
    e.attr("role").map_or_else(|| tag_role(e), aria_role)
}

/// Le rôle rend la boîte interactive — filtre
/// `interactive_only` (défaut agents LLM).
fn interactive(role: &Role) -> bool {
    matches!(
        role,
        Role::Link
            | Role::Button
            | Role::Textbox
            | Role::Combobox
            | Role::Checkbox
            | Role::Radio
            | Role::Form
    )
}

/// États natifs — attributs et focus, lus sur l'entrée
/// sémantique (jamais déduits du style : `disabled` est un
/// attribut, pas une couleur).
fn states_of(e: &SnapshotEntry, role: &Role) -> Vec<NodeState> {
    let mut v = Vec::new();
    if e.focused {
        v.push(NodeState::Focus);
    }
    if e.attr("disabled").is_some() || e.attr("aria-disabled") == Some("true") {
        v.push(NodeState::Disabled);
    }
    if e.attr("checked").is_some() || e.attr("aria-checked") == Some("true") {
        v.push(NodeState::Checked);
    }
    if e.attr("aria-expanded") == Some("true") || (e.tag == "details" && e.attr("open").is_some()) {
        v.push(NodeState::Expanded);
    }
    let editable = matches!(role, Role::Textbox | Role::Combobox)
        || matches!(e.attr("contenteditable"), Some("" | "true"));
    if editable {
        v.push(NodeState::Editable);
    }
    if matches!(role, Role::Link) {
        v.push(NodeState::Link);
    }
    if e.attr("selected").is_some() || e.attr("aria-selected") == Some("true") {
        v.push(NodeState::Selected);
    }
    if e.attr("required").is_some() || e.attr("aria-required") == Some("true") {
        v.push(NodeState::Required);
    }
    v
}

/// Nom accessible, chaîne accName-lite :
/// `aria-label` → `alt` → `title` → `placeholder`/`value`
/// (champs) → texte visible compacté → `id` → classe → tag.
/// Jamais vide : un nœud sans nom est du bruit pour un modèle.
fn name_of(e: &SnapshotEntry) -> String {
    for cand in [
        e.attr("aria-label"),
        e.attr("alt"),
        e.attr("title"),
        e.attr("placeholder"),
        e.attr("value"),
    ]
    .into_iter()
    .flatten()
    {
        let trimmed = cand.trim();
        if !trimmed.is_empty() {
            return trimmed.chars().take(NAME_CAP).collect();
        }
    }
    if !e.text.is_empty() {
        return e.text.chars().take(NAME_CAP).collect();
    }
    for cand in [e.attr("id"), e.attr("class")].into_iter().flatten() {
        let trimmed = cand.trim();
        if !trimmed.is_empty() {
            // `class` peut être une liste — la première porte
            // déjà l'essentiel pour un agent.
            return trimmed
                .split_whitespace()
                .next()
                .unwrap_or(trimmed)
                .chars()
                .take(NAME_CAP)
                .collect();
        }
    }
    e.tag.clone()
}

/// Un élément émis : nœud sérialisé + mémoire diff.
pub struct Emitted {
    /// Nœud fil.
    pub node: SnapshotNode,
    /// Identité DOM — clé de jointure du diff.
    pub index: u32,
    /// Génération à l'émission — une case réattribuée est un
    /// autre élément (added+removed, jamais changed).
    pub generation: u32,
    /// Signature de contenu — `changed` la compare.
    pub sig: u64,
}

/// Signature de contenu : rôle/nom/états/rect/depth + les
/// attributs d'état muables sans changement de nœud
/// (`value`, `checked`…) — la frappe et les cases à cocher
/// doivent apparaître dans `changed`.
fn signature(e: &SnapshotEntry, node: &SnapshotNode) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    serde_json::to_string(node).unwrap_or_default().hash(&mut h);
    for k in ["value", "checked", "open", "aria-expanded", "aria-checked"] {
        (k, e.attr(k)).hash(&mut h);
    }
    h.finish()
}

/// Construit les nœuds émis : marche sémantique jointe au
/// rapport géométrique (rect par index arène), handles
/// alloués/dédoublonnés dans `table`.
#[must_use]
pub fn emit(
    entries: &[SnapshotEntry],
    report: Option<&GeometryReport>,
    table: &mut HandleTable,
    tab: TabId,
    rev: u64,
    interactive_only: bool,
    max_nodes: usize,
) -> Vec<Emitted> {
    let rects: HashMap<u32, Rect> = report.map_or_else(HashMap::new, |r| {
        r.boxes
            .iter()
            .map(|b| {
                (
                    b.node,
                    Rect {
                        x: b.x,
                        y: b.y,
                        w: b.w,
                        h: b.h,
                    },
                )
            })
            .collect()
    });
    entries
        .iter()
        .map(|e| (e, role_of(e)))
        .filter(|(_, r)| !interactive_only || interactive(r))
        .take(max_nodes)
        .map(|(e, role)| {
            let node = SnapshotNode {
                elem_ref: table.alloc(tab, e.index, rev),
                states: states_of(e, &role),
                role,
                name: name_of(e),
                rect: rects.get(&e.index).copied(),
                depth: e.depth,
            };
            Emitted {
                sig: signature(e, &node),
                index: e.index,
                generation: e.generation,
                node,
            }
        })
        .collect()
}

/// Entrée mémorisée d'une émission passée — la base du diff.
pub struct SnapEntry {
    /// Identité DOM.
    pub index: u32,
    /// Génération à l'émission.
    pub generation: u32,
    /// Ref allouée (dédoublonnée — le même élément garde sa
    /// ref entre snapshots d'une même `rev`).
    pub elem_ref: ElementRef,
    /// Signature de contenu.
    pub sig: u64,
}

/// Diff de deux émissions — jointure sur `(index,
/// generation)` : un index réattribué est un retrait + un
/// ajout, jamais un changement (règle arène ADR-0008).
#[must_use]
pub fn diff(
    base: &[SnapEntry],
    current: &[Emitted],
) -> (Vec<SnapshotNode>, Vec<ElementRef>, Vec<SnapshotNode>) {
    let mut added = Vec::new();
    let mut changed = Vec::new();
    for c in current {
        match base
            .iter()
            .find(|b| b.index == c.index && b.generation == c.generation)
        {
            None => added.push(c.node.clone()),
            Some(b) if b.sig != c.sig => changed.push(c.node.clone()),
            Some(_) => {}
        }
    }
    let removed: Vec<ElementRef> = base
        .iter()
        .filter(|b| {
            !current
                .iter()
                .any(|c| c.index == b.index && c.generation == b.generation)
        })
        .map(|b| b.elem_ref.clone())
        .collect();
    (added, removed, changed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn emitted(index: u32, generation: u32, sig: u64) -> Emitted {
        Emitted {
            node: SnapshotNode {
                elem_ref: format!("el:{index}"),
                role: Role::Generic,
                name: String::new(),
                states: Vec::new(),
                rect: None,
                depth: 0,
            },
            index,
            generation,
            sig,
        }
    }

    fn base(index: u32, generation: u32, sig: u64) -> SnapEntry {
        SnapEntry {
            index,
            generation,
            elem_ref: format!("el:{index}"),
            sig,
        }
    }

    /// Jointure `(index, generation)` : identique → rien, sig
    /// différente → `changed`, index réattribué (autre
    /// génération) → `removed`+`added`, jamais `changed`.
    #[test]
    fn diff_classe_added_removed_changed() {
        let b = vec![base(1, 1, 10), base(2, 1, 20), base(3, 1, 30)];
        let cur = vec![
            emitted(1, 1, 10), // inchangé
            emitted(2, 1, 99), // sig changée → changed
            emitted(3, 2, 30), // case réattribuée → removed+added
        ];
        let (added, removed, changed) = diff(&b, &cur);
        assert_eq!(added.len(), 1);
        assert_eq!(added[0].elem_ref, "el:3");
        assert_eq!(removed, vec!["el:3".to_string()]);
        assert_eq!(changed.len(), 1);
        assert_eq!(changed[0].elem_ref, "el:2");
    }
}
