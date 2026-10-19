// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Dump texte d'un [`Dom`] — format indenté inspiré de
//! html5lib-tests (`| <tag>`, `|   "texte"`, `|   <!-- c -->`).
//! Utilisé par les tests de conformance et par
//! `explorer-cli dump` (jalon J2 : retour visuel immédiat).

use std::fmt::Write as _;

use explorer_dom::{Dom, NodeData, NodeId};

/// Sérialise `dom` en dump indenté (sans la ligne `#document`).
#[must_use]
pub fn dump(dom: &Dom) -> String {
    let mut out = String::new();
    dump_children(dom, dom.document(), 1, &mut out);
    out
}

fn dump_children(dom: &Dom, parent: NodeId, depth: usize, out: &mut String) {
    for child in dom.children(parent).unwrap_or_default() {
        out.push_str("| ");
        for _ in 1..depth {
            out.push_str("  ");
        }
        match dom.data(child).unwrap_or(&NodeData::Document) {
            NodeData::Document => out.push_str("#document-nested"),
            NodeData::Doctype(d) => {
                let _ = write!(out, "<!DOCTYPE {}>", d.name);
            }
            NodeData::Element(e) => {
                let _ = write!(out, "<{}>", e.tag_name);
                let mut attrs: Vec<_> = e
                    .attributes
                    .iter()
                    .map(|a| format!("{}=\"{}\"", a.name, a.value))
                    .collect();
                attrs.sort();
                for a in attrs {
                    out.push('\n');
                    out.push_str("| ");
                    for _ in 0..depth {
                        out.push_str("  ");
                    }
                    out.push_str(&a);
                }
            }
            NodeData::Text(t) => {
                let _ = write!(out, "\"{t}\"");
            }
            NodeData::Comment(c) => {
                let _ = write!(out, "<!-- {c} -->");
            }
        }
        out.push('\n');
        dump_children(dom, child, depth + 1, out);
    }
}
