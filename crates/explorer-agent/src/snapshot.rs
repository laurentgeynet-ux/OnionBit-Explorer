// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Snapshot E1 — l'arbre sérialisé construit sur le
//! [`GeometryReport`] : boîtes posées jointes `(tag,id,cls,rect)`
//! en pré-ordre document. Forme honnête E1 : `role` dérivé du tag
//! seul (le type de `<input>` n'est pas porté par la géométrie),
//! `name` = `id` sinon première classe sinon tag, `depth` = 0
//! (arbre plat — la hiérarchie sémantique arrive en E3).

use explorer_core::GeometryReport;

use crate::proto::{NodeState, Rect, Role, SnapshotNode};
use crate::state::HandleTable;
use explorer_core::TabId;

/// Tag → rôle (sous-ensemble v1 — le mapping ARIA complet est E3).
fn tag_role(tag: &str) -> Role {
    match tag {
        "a" => Role::Link,
        "button" => Role::Button,
        // `<input>` : sans l'attribut type, la meilleure lecture
        // interactive est textbox — approximation E1 assumée.
        "input" | "textarea" => Role::Textbox,
        "select" | "datalist" => Role::Combobox,
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => Role::Heading,
        "img" | "picture" => Role::Img,
        "nav" => Role::Navigation,
        "form" => Role::Form,
        "li" => Role::ListItem,
        "table" => Role::Table,
        "main" => Role::Main,
        _ => Role::Generic,
    }
}

/// Le rôle rend la boîte interactive — filtre
/// `interactive_only` (défaut agents LLM).
fn interactive(role: &Role) -> bool {
    matches!(
        role,
        Role::Link | Role::Button | Role::Textbox | Role::Combobox | Role::Form
    )
}

/// États déductibles d'une boîte E1 — `link` seul est certain ;
/// `editable` vient du rôle textbox/combobox.
fn states_of(role: &Role) -> Vec<NodeState> {
    let mut v = Vec::new();
    if matches!(role, Role::Link) {
        v.push(NodeState::Link);
    }
    if matches!(role, Role::Textbox | Role::Combobox) {
        v.push(NodeState::Editable);
    }
    v
}

/// Nom le plus informatif disponible : `id`, sinon première
/// classe, sinon tag (jamais vide — un nœud sans nom est du bruit
/// pour un modèle).
fn name_of(b: &explorer_core::GeomBox) -> String {
    if !b.id.is_empty() {
        b.id.clone()
    } else if !b.cls.is_empty() {
        b.cls.clone()
    } else {
        b.tag.clone()
    }
}

/// Construit la liste de nœuds depuis le rapport géométrique et
/// alloue les handles correspondants dans `table`.
#[must_use]
pub fn nodes_from_geometry(
    report: &GeometryReport,
    table: &mut HandleTable,
    tab: TabId,
    rev: u64,
    interactive_only: bool,
    max_nodes: usize,
) -> Vec<SnapshotNode> {
    report
        .boxes
        .iter()
        .map(|b| (b, tag_role(&b.tag)))
        .filter(|(_, r)| !interactive_only || interactive(r))
        .take(max_nodes)
        .map(|(b, role)| SnapshotNode {
            elem_ref: table.alloc(tab, b.node, rev),
            states: states_of(&role),
            role,
            name: name_of(b),
            rect: Some(Rect {
                x: b.x,
                y: b.y,
                w: b.w,
                h: b.h,
            }),
            depth: 0,
        })
        .collect()
}
