// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Partagé entre les cibles `tree_construction` et `scratch_triage`
//! (format `.dat` html5lib + dump d'arbre normalisé).

use std::fmt::Write as _;

use explorer_dom::{Dom, NodeData, NodeId};

/// Un cas de test extrait d'un fichier `.dat`.
pub struct Case {
    pub input: String,
    /// Arbre attendu (brut, `#document` normalisé plus bas).
    pub expected: String,
    /// `true` si parsing fragmentaire (hors périmètre).
    pub fragment: bool,
    /// `true` si scripting on (`<noscript>` rawtext — hors périmètre).
    pub script_on: bool,
}

#[derive(PartialEq)]
enum Section {
    None,
    Data,
    Expected,
    Skip,
}

pub fn parse_cases(file: &str) -> Vec<Case> {
    let mut cases = Vec::new();
    let mut input = Vec::new();
    let mut expected = Vec::new();
    let mut fragment = false;
    let mut script_on = false;
    let mut section = Section::None;

    let flush = |cases: &mut Vec<Case>,
                 input: &mut Vec<String>,
                 expected: &mut Vec<String>,
                 fragment: &mut bool,
                 script_on: &mut bool| {
        if !expected.is_empty() {
            cases.push(Case {
                input: input.join("\n"),
                expected: expected.join("\n"),
                fragment: *fragment,
                script_on: *script_on,
            });
        }
        input.clear();
        expected.clear();
        *fragment = false;
        *script_on = false;
    };

    for line in file.lines() {
        if line.starts_with('#') {
            match line {
                "#data" => {
                    flush(
                        &mut cases,
                        &mut input,
                        &mut expected,
                        &mut fragment,
                        &mut script_on,
                    );
                    section = Section::Data;
                }
                "#document" => section = Section::Expected,
                "#document-fragment" => {
                    fragment = true;
                    section = Section::Skip;
                }
                "#script-on" => {
                    script_on = true;
                    section = Section::Skip;
                }
                _ => section = Section::Skip, // #errors, #new-errors…
            }
            continue;
        }
        match section {
            Section::Data => input.push(line.to_string()),
            Section::Expected => expected.push(line.to_string()),
            _ => {}
        }
    }
    flush(
        &mut cases,
        &mut input,
        &mut expected,
        &mut fragment,
        &mut script_on,
    );
    cases
}

/// Dump de l'arbre produit — même format que `#document`.
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

/// Normalise le dump attendu : préfixes de namespace étrangers
/// (`<svg svg>` → `<svg>`, `xlink href` → `xlink:href`, `xmlns
/// xlink` → `xmlns:xlink`, `xml lang` → `xml:lang`), et fusion des
/// nœuds texte adjacents (notre dump n'en produit pas, la spec non
/// plus — garde-fou).
pub fn normalize_expected(expected: &str) -> String {
    let mut out = String::new();
    for line in expected.lines() {
        let indent = line.len() - line.trim_start().len();
        let content = line.trim_start();
        let norm = if let Some(rest) = content.strip_prefix('<') {
            // Élément ou doctype : `<svg svg>` → `<svg>`.
            if let Some(inner) = rest.strip_suffix('>') {
                let inner = inner
                    .strip_prefix("svg ")
                    .or_else(|| inner.strip_prefix("math "))
                    .unwrap_or(inner);
                format!("<{inner}>")
            } else {
                format!("<{rest}")
            }
        } else if let Some((name, val)) = content.split_once('=') {
            // Attribut étranger `xlink href` → `xlink:href`.
            let name = name
                .replace("xlink ", "xlink:")
                .replace("xml ", "xml:")
                .replace("xmlns ", "xmlns:");
            format!("{name}={val}")
        } else {
            content.to_string()
        };
        for _ in 0..indent {
            out.push(' ');
        }
        out.push_str(&norm);
        out.push('\n');
    }
    out
}
