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

/// `position` usée (étape 21).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Position {
    /// Flux normal.
    #[default]
    Static,
    /// Décalée après flux normal.
    Relative,
    /// Hors flux, relative au bloc conteneur positionné.
    Absolute,
    /// Hors flux, relative au viewport.
    Fixed,
    /// En flux (relative) puis collée au seuil `top` quand
    /// le scroll l'atteint — contrainte appliquée au paint.
    Sticky,
}

/// `float` usé (étape 21).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Float {
    /// Pas de flottement.
    #[default]
    None,
    /// Flotte à gauche.
    Left,
    /// Flotte à droite.
    Right,
}

/// `overflow` usé (étape 21) — clipping au rendu.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Overflow {
    /// Déborde librement.
    #[default]
    Visible,
    /// Clippé à la padding box.
    Hidden,
    /// Clippé + défilement.
    Scroll,
    /// Défilement si déborde.
    Auto,
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
    /// Lignes produites par le layout en-ligne (étape 20) —
    /// non vide seulement pour les conteneurs en-ligne
    /// (blocs anonymes).
    pub lines: Vec<crate::inline::LineBox>,
    /// Géométrie — remplie par les étapes suivantes.
    pub rect: Rect,
    /// `position` usée (étape 21).
    pub position: Position,
    /// `float` usé (étape 21).
    pub float: Float,
    /// `overflow` usé — clipping appliqué par le rendu.
    pub overflow: Overflow,
    /// `z-index` résolu (0 = auto) — contextes d'empilement
    /// pour l'ordre de peinture.
    pub z_index: i32,
    /// Taille préférée d'un élément remplacé (`img`) en px —
    /// déjà résolue amont (attribut `width`/`height` s'il est
    /// sain, sinon dimensions intrinsèques décodées). Chaque
    /// axe : CSS `width`/`height` > cette valeur.
    pub intrinsic: Option<(f32, f32)>,
    /// État coché d'un contrôle `checkbox`/`radio` (étape 38) —
    /// le rendu peint une marque pleine centrée. La valeur des
    /// champs est portée par un enfant `Text` synthétique, pas
    /// par ce champ.
    pub control_checked: bool,
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
/// Équivaut à [`build_box_tree_sized`] sans tailles d'images.
///
/// `root` est l'élément racine du document — typiquement
/// [`Dom::document_element`] : `None` (rien à peindre) tant que
/// le document en construction n'a produit aucun élément. Un DOM
/// **partiel** est une entrée supportée (R-1) : l'arbre obtenu
/// reflète les tokens consommés à cette frontière — une frame =
/// un état cohérent.
#[must_use]
pub fn build_box_tree(dom: &Dom, root: NodeId, styles: &StyleMap) -> Option<LayoutBox> {
    build_box_tree_sized(dom, root, styles, &std::collections::HashMap::new())
}

/// Variante de [`build_box_tree`] avec `sizes` = tailles
/// préférées des éléments remplacés (`img` décodés, attributs
/// résolus) ; un `img` absent de la map produit une boîte
/// texte de son `alt` (dégradation image cassée).
#[must_use]
pub fn build_box_tree_sized<S: ::std::hash::BuildHasher>(
    dom: &Dom,
    root: NodeId,
    styles: &StyleMap,
    sizes: &std::collections::HashMap<NodeId, (f32, f32), S>,
) -> Option<LayoutBox> {
    let style = styles.get(root)?.clone();
    if style.get("display") == Some("none") {
        return None;
    }
    // `<img>` : élément remplacé si l'image est disponible,
    // texte `alt` sinon (repli du navigateur classique).
    if dom.tag_name(root).is_ok_and(|t| t == "img") {
        let alt = dom
            .attribute(root, "alt")
            .ok()
            .flatten()
            .unwrap_or_default()
            .clone();
        let (kind, text) = match sizes.get(&root) {
            Some(_) => (box_kind_of(&style), None),
            None => (BoxKind::Text, (!alt.is_empty()).then_some(alt)),
        };
        return Some(LayoutBox {
            kind,
            node: Some(root),
            style: style.clone(),
            text,
            children: Vec::new(),
            lines: Vec::new(),
            rect: Rect::default(),
            position: Position::Static,
            float: Float::None,
            overflow: Overflow::Visible,
            z_index: 0,
            intrinsic: sizes.get(&root).copied(),
            control_checked: false,
        });
    }
    // Contrôles de formulaire (étape 38) : `input`/`select` —
    // la valeur peinte est synthétisée en enfant `Text` (la
    // boîte reste atomique ; le texte se pose dedans au
    // layout), l'état `checked` devient une marque au rendu.
    if dom
        .tag_name(root)
        .is_ok_and(|t| t == "input" || t == "select")
    {
        let ty = dom
            .attribute(root, "type")
            .ok()
            .flatten()
            .unwrap_or_default()
            .to_lowercase();
        let (text, checked) = control_paint(dom, root, &ty);
        return Some(LayoutBox {
            kind: box_kind_of(&style),
            node: Some(root),
            style: style.clone(),
            text: None,
            children: text
                .map(|t| {
                    vec![LayoutBox {
                        kind: BoxKind::Text,
                        node: Some(root),
                        style: style.clone(),
                        text: Some(t),
                        children: Vec::new(),
                        lines: Vec::new(),
                        rect: Rect::default(),
                        position: Position::Static,
                        float: Float::None,
                        overflow: Overflow::Visible,
                        z_index: 0,
                        intrinsic: None,
                        control_checked: false,
                    }]
                })
                .unwrap_or_default(),
            lines: Vec::new(),
            rect: Rect::default(),
            position: Position::Static,
            float: Float::None,
            overflow: Overflow::Visible,
            z_index: 0,
            intrinsic: None,
            control_checked: checked,
        });
    }
    let kind = box_kind_of(&style);
    let mut b = LayoutBox {
        kind,
        node: Some(root),
        style: style.clone(),
        text: None,
        children: Vec::new(),
        lines: Vec::new(),
        rect: Rect::default(),
        position: Position::Static,
        float: Float::None,
        overflow: Overflow::Visible,
        z_index: 0,
        intrinsic: None,
        control_checked: false,
    };
    let children = child_boxes(dom, root, styles, &style, sizes);
    b.children = if kind.is_inline_level() {
        children // en-ligne : pas d'enveloppe anonyme ici
    } else {
        wrap_anonymous(children, &style)
    };
    Some(b)
}

/// `display` → nature de boîte. §9.7 : `float` ≠ none force
/// un niveau bloc (`display` calculé → block) — un `<span>`
/// flottant n'est plus inline et interrompt la ligne.
fn box_kind_of(style: &ComputedStyle) -> BoxKind {
    if matches!(style.get("float"), Some("left" | "right")) {
        return BoxKind::Block;
    }
    match style.get_or("display", "inline") {
        "inline" | "inline-block" | "inline-table" | "inline-flex" => BoxKind::Inline,
        "list-item" => BoxKind::ListItem,
        // block, table* et tout l'inconnu → bloc (repli).
        _ => BoxKind::Block,
    }
}

/// Boîtes des enfants de `parent` (éléments + texte utile).
fn child_boxes<S: ::std::hash::BuildHasher>(
    dom: &Dom,
    parent: NodeId,
    styles: &StyleMap,
    parent_style: &ComputedStyle,
    sizes: &std::collections::HashMap<NodeId, (f32, f32), S>,
) -> Vec<LayoutBox> {
    let Ok(children) = dom.children(parent) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for child in children {
        if dom.element(child).is_ok() {
            if let Some(mut b) = build_box_tree_sized(dom, child, styles, sizes) {
                // `<br>` : saut de ligne forcé — texte sentinelle
                // U+2028 LINE SEPARATOR, que l'itemisation UAX #14
                // coupe durement (`ends_hard_break`). Choisi plutôt
                // que "\n" parce qu'il N'EST PAS un blanc CSS : il
                // survit au collapsing §16.6.1 de `text_item`
                // (étape 49), là où "\n" deviendrait une espace.
                if dom.tag_name(child).is_ok_and(|t| t == "br") && b.text.is_none() {
                    b.text = Some("\u{2028}".to_string());
                }
                out.push(b);
            }
            // `display: none` ou absent de la map → sauté.
        } else if matches!(dom.data(child), Ok(explorer_dom::NodeData::Text(_))) {
            // Commentaires, doctype… : pas de boîte (seuls
            // les vrais `Text` en produisent une).
            let text = dom.text(child).unwrap_or_default();
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
                    lines: Vec::new(),
                    rect: Rect::default(),
                    position: Position::Static,
                    float: Float::None,
                    overflow: Overflow::Visible,
                    z_index: 0,
                    intrinsic: None,
                    control_checked: false,
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
        b.kind == BoxKind::Text
            && b.text
                .as_deref()
                .is_some_and(|t| crate::text::trim_css_ws(t).is_empty())
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
            lines: Vec::new(),
            rect: Rect::default(),
            position: Position::Static,
            float: Float::None,
            overflow: Overflow::Visible,
            z_index: 0,
            intrinsic: None,
            control_checked: false,
        });
    }
}

/// Texte à peindre + état coché d'un contrôle de formulaire.
///
/// - `input` texte/hidden-less : `value` (ou `placeholder`
///   si vide) ; boutons (`submit`/`button`/`reset`) : `value`
///   (libellé) ; `checkbox`/`radio` : pas de texte, marque si
///   `checked` ; `hidden`/`file`/`image` : rien (`hidden`
///   est déjà `display:none` en feuille UA).
/// - `select` : texte de l'`<option selected>` (ou de la
///   première) — ses enfants `<option>` ne sont pas posés.
fn control_paint(dom: &Dom, node: NodeId, ty: &str) -> (Option<String>, bool) {
    let attr = |n: NodeId, name: &str| dom.attribute(n, name).ok().flatten();
    let tag = dom.tag_name(node).unwrap_or("");
    if tag == "select" {
        // Options en ordre document ; `selected` gagne, sinon
        // la première (dégradation classique).
        let mut opts = Vec::new();
        let mut queue = vec![node];
        while let Some(n) = queue.pop() {
            if n != node && dom.tag_name(n).is_ok_and(|t| t == "option") {
                opts.push(n);
            }
            if let Ok(ch) = dom.children(n) {
                queue.extend(ch.iter().rev());
            }
        }
        let sel = opts
            .iter()
            .find(|&&o| attr(o, "selected").is_some())
            .or_else(|| opts.first());
        let text = sel.and_then(|&o| {
            attr(o, "value")
                .or_else(|| dom.text_content(o).ok().map(|t| t.trim().to_string()))
                .filter(|t| !t.is_empty())
        });
        return (text, false);
    }
    match ty {
        "checkbox" | "radio" => (None, attr(node, "checked").is_some()),
        "submit" | "button" | "reset" => (attr(node, "value"), false),
        "hidden" | "file" | "image" => (None, false),
        // Texte libre : `value`, `placeholder` en repli.
        _ => {
            let v = attr(node, "value").filter(|s| !s.is_empty());
            (v.or_else(|| attr(node, "placeholder")), false)
        }
    }
}
