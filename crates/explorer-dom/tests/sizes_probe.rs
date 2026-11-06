// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Verrous de taille — la promesse mémoire de l'interning
//! (AR-13) est mesurable : si un champ repasse en `String`,
//! ces bornes sautent.

use std::mem::size_of;

#[test]
fn tailles_apres_interning() {
    // `Attribute` : `Atom` (16) + `String` (24) — 48 → 40 o.
    assert_eq!(size_of::<explorer_dom::Attribute>(), 40);
    // `ElementData` : `Atom` + `Vec` + `Option<NodeId>` +
    // namespace — 64 → 56 o.
    assert_eq!(size_of::<explorer_dom::ElementData>(), 56);
    // `NodeData` : `Element` est désormais le variant maximal —
    // `DoctypeData` (72 o) est boxé, un seul par document.
    assert_eq!(size_of::<explorer_dom::NodeData>(), 56);
    // `TextRange` (AR-14) : plage (u32, u32) — 24 → 8 o par
    // nœud texte, contenu dans le buffer partagé.
    assert_eq!(size_of::<explorer_dom::TextRange>(), 8);
}
