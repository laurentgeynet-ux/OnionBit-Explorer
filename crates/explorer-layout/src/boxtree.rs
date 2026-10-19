// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Arbre de boîtes (étape 17) : DOM + `StyleMap` → boîtes
//! block/inline/anonymes/texte.
//!
//! Modèle (CSS 2.1 §9.2) :
//! - `display: none` → pas de boîte (sous-arbre sauté) ;
//! - `block`, `list-item`, `table*`… → boîte de niveau bloc ;
//! - `inline`, `inline-block` → boîte en-ligne ;
//! - texte → boîte texte en-ligne portant le style du parent ;
//! - dans un conteneur bloc, toute suite d'enfants de niveau
//!   en-ligne est enveloppée dans une boîte **anonyme**
//!   (§9.2.1.1) — les blancs purs y sont déjà sautés.
//!
//! Simplification documentée : une boîte `inline` contenant un
//! bloc garde ce bloc en descendant (le « split » d'en-ligne
//! autour des blocs, §9.2.1.1 fin, est différé — l'étape 20
//! décidera du rendu inline-block).

use std::fmt::Write as _;

use explorer_css::{ComputedStyle, StyleMap};
use explorer_dom::{Dom, NodeId};

/// Rectangle de la boîte — rempli par le dimensionnement
/// (étapes 19+), zéro à la construction.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Rect {
    /// Abscisse (px).
    pub x: f32,
    /// Ordonnée (px).
    pub y: f32,
    /// Largeur (px).
    pub width: f32,
    /// Hauteur (px).
    pub height: f32,
}

/// Nature de la boîte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoxKind {
    /// Boîte de niveau bloc (`display: block` et assimilés).
    Block,
    /// `display: list-item` — bloc + puce (marqueur : étape 20).
    ListItem,
    /// Boîte en-ligne (`display: inline`/`inline-block`).
    Inline,
    /// Texte — porte le style du parent (héritage fait).
    Text,
    /// Enveloppe anonyme autour d'une suite de contenu
    /// en-ligne dans un conteneur bloc.
    AnonymousBlock,
}

impl BoxKind {
    /// Niveau en-ligne (`inline`/`text`) — regroupé par les
    /// enveloppes anonymes.
    #[must_use]
    pub fn is_inline_level(self) -> bool {
        matches!(self, Self::Inline | Self::Text)
    }
}

/// Une boîte de l'arbre de mise en page.
#[derive(Debug, Clone)]
pub struct LayoutBox {
    /// Nature.
    pub kind: BoxKind,
    /// Nœud DOM source — `None` pour les boîtes anonymes.
    pub node: Option<NodeId>,
    /// Style calculé (du nœud, ou du parent pour le texte et
    /// les boîtes anonymes).
    pub style: ComputedStyle,
    /// Texte (`kind == Text`).
    pub text: Option<String>,
    /// Enfants.
    pub children: Vec<LayoutBox>,
    /// Géométrie — remplie par les étapes suivantes.
    pub rect: Rect,
}

impl LayoutBox {
    /// `display` calculé de la boîte (`block` à défaut).
    #[must_use]
    pub fn display(&self) -> &str {
        self.style.get_or("display", "block")
    }

    /// Dump texte indenté pour debug (format wireframe —
    /// stable pour les golden tests). `dom` sert à résoudre
    /// les noms de balise.
    #[must_use]
    pub fn dump(&self, dom: &Dom) -> String {
        let mut out = String::new();
        self.dump_into(dom, &mut out, 0);
        out
    }

    fn dump_into(&self, dom: &Dom, out: &mut String, depth: usize) {
        let indent = "  ".repeat(depth);
        let kind = match self.kind {
            BoxKind::Block => "block",
            BoxKind::ListItem => "list-item",
            BoxKind::Inline => "inline",
            BoxKind::Text => "text",
            BoxKind::AnonymousBlock => "anon",
        };
        let tag = self
            .node
            .and_then(|n| dom.tag_name(n).ok())
            .map(|t| format!("<{t}>"))
            .unwrap_or_default();
        let label = if let Some(t) = &self.text {
            let trimmed: String = t.chars().take(40).collect();
            format!(" \"{}\"", trimmed.replace('\n', "\\n"))
        } else {
            String::new()
        };
        let _ = writeln!(
            out,
            "{indent}{kind}{tag}{label} [{} {} {}x{}]",
            self.rect.x, self.rect.y, self.rect.width, self.rect.height
        );
        for c in &self.children {
            c.dump_into(dom, out, depth + 1);
        }
    }
}

/// Construit l'arbre de boîtes sous `root`.
///
/// `styles` doit contenir le style calculé de chaque nœud
/// élément du sous-arbre (`StyleEngine::compute_tree`) — les
/// boîtes texte héritent du style de leur parent. `None` si
/// `root` est `display: none` ou absent de la `StyleMap`.
#[must_use]
pub fn build_box_tree(dom: &Dom, root: NodeId, styles: &StyleMap) -> Option<LayoutBox> {
    let style = styles.get(root)?.clone();
    if style.get("display") == Some("none") {
        return None;
    }
    let kind = box_kind_of(&style);
    let mut b = LayoutBox {
        kind,
        node: Some(root),
        style: style.clone(),
        text: None,
        children: Vec::new(),
        rect: Rect::default(),
    };
    let children = child_boxes(dom, root, styles, &style);
    b.children = if kind.is_inline_level() {
        children // en-ligne : pas d'enveloppe anonyme ici
    } else {
        wrap_anonymous(children, &style)
    };
    Some(b)
}

/// `display` → nature de boîte.
fn box_kind_of(style: &ComputedStyle) -> BoxKind {
    match style.get_or("display", "inline") {
        "inline" | "inline-block" | "inline-table" | "inline-flex" => BoxKind::Inline,
        "list-item" => BoxKind::ListItem,
        // block, table* et tout l'inconnu → bloc (repli).
        _ => BoxKind::Block,
    }
}

/// Boîtes des enfants de `parent` (éléments + texte utile).
fn child_boxes(
    dom: &Dom,
    parent: NodeId,
    styles: &StyleMap,
    parent_style: &ComputedStyle,
) -> Vec<LayoutBox> {
    let Ok(children) = dom.children(parent) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for child in children {
        if dom.element(child).is_ok() {
            if let Some(b) = build_box_tree(dom, child, styles) {
                out.push(b);
            }
            // `display: none` ou absent de la map → sauté.
        } else if let Ok(text) = dom.text(child) {
            // Tout texte produit une boîte — les blancs purs
            // sont aussi gardés : ils portent l'espace entre
            // éléments en-ligne (`<b>x</b> <b>y</b>`). C'est
            // `wrap_anonymous` qui les tronque en bord de run.
            if !text.is_empty() {
                out.push(LayoutBox {
                    kind: BoxKind::Text,
                    node: Some(child),
                    style: parent_style.clone(),
                    text: Some(text.to_string()),
                    children: Vec::new(),
                    rect: Rect::default(),
                });
            }
        }
    }
    out
}

/// Enveloppe les suites de niveau en-ligne dans des blocs
/// anonymes (CSS 2.1 §9.2.1.1). Les boîtes texte blanches
/// pures en bord de suite sont tronquées ; une suite
/// entièrement blanche ne produit pas de boîte.
fn wrap_anonymous(children: Vec<LayoutBox>, parent_style: &ComputedStyle) -> Vec<LayoutBox> {
    let mut out: Vec<LayoutBox> = Vec::new();
    let mut run: Vec<LayoutBox> = Vec::new();
    for b in children {
        if b.kind.is_inline_level() {
            run.push(b);
        } else {
            flush_run(&mut out, &mut run, parent_style);
            out.push(b);
        }
    }
    flush_run(&mut out, &mut run, parent_style);
    out
}

/// Ferme une suite en-ligne : tronque les blancs purs en
/// bord, saute la suite si entièrement blanche.
fn flush_run(out: &mut Vec<LayoutBox>, run: &mut Vec<LayoutBox>, style: &ComputedStyle) {
    let is_ws = |b: &LayoutBox| {
        b.kind == BoxKind::Text && b.text.as_deref().is_some_and(|t| t.trim().is_empty())
    };
    while run.first().is_some_and(is_ws) {
        run.remove(0);
    }
    while run.last().is_some_and(is_ws) {
        run.pop();
    }
    if !run.is_empty() {
        out.push(LayoutBox {
            kind: BoxKind::AnonymousBlock,
            node: None,
            style: style.clone(),
            text: None,
            children: std::mem::take(run),
            rect: Rect::default(),
        });
    }
}
