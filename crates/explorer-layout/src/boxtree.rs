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

use explorer_css::{ComputedStyle, PseudoElement, StyleMap};
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
    /// Paramètres d'entrée du dernier passage de `layout_block`
    /// — R-12 : permettent de re-poser la boîte en place lors
    /// d'un relayout incrémental (`relayout_subtree`). Défaut
    /// avant le premier layout.
    pub slot: BoxSlot,
    /// Hauteur utilisée > hauteur de contenu (clamp `min-height`
    /// ou `height` au-dessus de l'auto) — posé par `layout_block`.
    /// §8.3.1 : la marge basse du dernier fils in-flow ne s'échappe
    /// alors pas (elle est avalée, ni contenu ni marge externe —
    /// WPT `margin-collapse-min-height-001`).
    pub height_raised: bool,
    /// La boîte établit un contexte de formatage indépendant sans
    /// que son propre style ne le signale — posé par le layout
    /// parent pour les items flex (`layout_item`) et grid
    /// (Flexbox §4.1 / Grid §6 : un item « establishes an
    /// independent formatting context »). Conséquences §8.3.1 +
    /// §9.4.1 : les marges des descendants ne s'échappent jamais
    /// de l'item (haut ni bas) et l'item englobe ses flottants.
    /// Champ (pas paramètre) : il survit au relayout incrémental
    /// `relayout_subtree`, qui repose la boîte via `slot`.
    pub fc_root: bool,
}

/// Contexte de pose enregistré par `layout_block` (R-12) :
/// origine, largeur/hauteur du bloc conteneur, font-size du
/// parent et bloc conteneur des descendants positionnés.
#[derive(Debug, Clone, Copy, Default)]
pub struct BoxSlot {
    /// Origine de flux (x, y) au moment de la pose.
    pub x: f32,
    /// Idem — y.
    pub y: f32,
    /// Largeur du bloc conteneur.
    pub cb_w: f32,
    /// Hauteur du bloc conteneur si définie.
    pub cb_h: Option<f32>,
    /// `font-size` du parent en px.
    pub parent_fs: f32,
    /// Bloc conteneur des descendants `position:absolute/fixed`.
    pub abs_cb: Rect,
}

impl LayoutBox {
    /// `display` calculé de la boîte (`block` à défaut).
    #[must_use]
    pub fn display(&self) -> &str {
        self.style.get_or("display", "block")
    }

    /// Hors du flux normal : `position: absolute|fixed` sur une
    /// boîte d'**élément**. `position` n'est pas hérité en CSS,
    /// mais les boîtes `Text`/`AnonymousBlock` clonent le style
    /// complet du parent — leur `position` copiée ne les rend
    /// PAS positionnées (R-22 : le texte d'un `span` absolu
    /// était sauté du flux en-ligne ET re-posé comme absolu
    /// une deuxième fois). `bx.position` n'est renseigné qu'au
    /// layout — cette lecture du style calculé vaut à tout
    /// moment.
    #[must_use]
    pub fn out_of_flow(&self) -> bool {
        !matches!(self.kind, BoxKind::Text | BoxKind::AnonymousBlock)
            && matches!(self.style.get("position"), Some("absolute" | "fixed"))
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
            explorer_common::deep(|| c.dump_into(dom, out, depth + 1));
        }
    }
}

impl Drop for LayoutBox {
    /// Destruction itérative (R-24) : l'arbre peut atteindre
    /// des milliers de niveaux une fois la construction
    /// protégée par [`explorer_common::deep`] — le drop
    /// récursif par défaut déborderait la pile malgré tout.
    /// Chaque enfant est vidé de ses descendants avant sa
    /// propre destruction → aucune descente.
    fn drop(&mut self) {
        let mut stack = std::mem::take(&mut self.children);
        while let Some(mut child) = stack.pop() {
            stack.append(&mut child.children);
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
    // Jamais rendus (R-20, filet de robustesse) : un élément de
    // cette liste ne produit **aucune** boîte même si la cascade
    // a manqué le nœud (injection DOM post-compute, feuille sans
    // la règle UA, contenu étranger…) — `script`/`style` visibles
    // étaient le bug wikipedia. `noscript` n'y est PAS : scripting
    // off ⇒ contenu rendu (voir UA_CSS). `template`/`head` et ses
    // enfants non-rendus sont couverts aussi par display:none.
    if dom.tag_name(root).is_ok_and(|t| {
        matches!(
            t,
            "script" | "style" | "template" | "head" | "meta" | "link" | "base" | "title"
        )
    }) {
        return None;
    }
    // `<img>` : élément remplacé si l'image est disponible,
    // texte `alt` sinon (repli du navigateur classique).
    if dom.tag_name(root).is_ok_and(|t| t == "img") {
        return Some(img_box(dom, root, style.clone(), sizes));
    }
    // `<svg>` : élément remplacé **atomique** (R-9) — son
    // sous-arbre étranger n'est jamais boxifié en HTML ; la
    // taille vient de `sizes` (collecte [`svg_sizes`] amont) ou
    // à défaut des attributs/`viewBox` (300×150 en dernier
    // ressort — même repli que `img`).
    if dom.tag_name(root).is_ok_and(|t| t == "svg")
        && dom
            .namespace(root)
            .is_ok_and(|ns| ns == explorer_dom::Namespace::Svg)
    {
        let intrinsic = sizes
            .get(&root)
            .copied()
            .or_else(|| Some(svg_intrinsic(dom, root)));
        return Some(leaf_box(
            box_kind_of(&style),
            root,
            style.clone(),
            intrinsic,
        ));
    }
    // `<canvas>` (R-11) : élément remplacé — le contenu bitmap
    // est rejoué par explorer-render depuis l'op-list JS ;
    // le layout ne connaît que la taille intrinsèque
    // (attributs `width`/`height`, défaut spec 300×150).
    if dom.tag_name(root).is_ok_and(|t| t == "canvas") {
        return Some(leaf_box(
            box_kind_of(&style),
            root,
            style.clone(),
            Some(canvas_intrinsic(dom, root)),
        ));
    }
    // `<video>`/`<audio>` (R-34) : éléments remplacés
    // **natifs** — boîte feuille atomique ; le contenu de
    // repli n'est jamais boxifié (un moteur qui « supporte »
    // le média l'affiche toujours). La taille intrinsèque
    // vient de `sizes` — affiche `poster` décodée —
    // sinon du gabarit remplacé standard : vidéo 300×150,
    // audio 300×54 (barre de contrôle). Un `audio` sans
    // `controls` n'arrive pas ici (UA : `display:none`).
    if let Ok(tag @ ("video" | "audio")) = dom.tag_name(root) {
        let fallback = if tag == "audio" {
            (300.0, 54.0)
        } else {
            (300.0, 150.0)
        };
        return Some(leaf_box(
            box_kind_of(&style),
            root,
            style.clone(),
            Some(sizes.get(&root).copied().unwrap_or(fallback)),
        ));
    }
    // `<object>` (R-13c) : élément **remplacé** si sa ressource
    // est résolue, sinon ses **enfants se substituent**.
    if dom.tag_name(root).is_ok_and(|t| t == "object") {
        return Some(object_box(dom, root, &style, styles, sizes));
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
        return Some(control_box(
            root,
            style.clone(),
            control_paint(dom, root, &ty),
        ));
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
        slot: BoxSlot::default(),
        height_raised: false,
        fc_root: false,
    };
    let children = child_boxes(dom, root, styles, &style, sizes);
    b.children = wrap_children(children, &style, kind);
    Some(b)
}

/// `display` → nature de boîte. §9.7 : `float` ≠ none force
/// un niveau bloc (`display` calculé → block) — un `<span>`
/// flottant n'est plus inline et interrompt la ligne.
fn box_kind_of(style: &ComputedStyle) -> BoxKind {
    if matches!(style.get("float"), Some("left" | "right")) {
        return BoxKind::Block;
    }
    // §9.7 : `position: absolute|fixed` blockifie les
    // valeurs de niveau inline — `display:inline` positionné
    // hors flux devient un bloc (`.hr-line:after{
    // position:absolute}` NPR : la bande tricolore).
    // `list-item` n'est pas dans la table de blockification
    // (sa puce survit au positionnement).
    match style.get_or("display", "inline") {
        "inline" | "inline-block" | "inline-table" | "inline-flex"
            if matches!(style.get("position"), Some("absolute" | "fixed")) =>
        {
            BoxKind::Block
        }
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
    // `::before` généré en tête du contenu (R-13).
    if let Some(b) = pseudo_box(styles, parent, PseudoElement::Before) {
        out.push(b);
    }
    for child in children {
        if dom.element(child).is_ok() {
            // Récursion ∝ profondeur DOM — pile garantie
            // (R-24 : `host-nest3000` ne doit plus tuer le
            // worker).
            if let Some(mut b) =
                explorer_common::deep(|| build_box_tree_sized(dom, child, styles, sizes))
            {
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
                    slot: BoxSlot::default(),
                    height_raised: false,
                    fc_root: false,
                });
            }
        }
    }
    // `::after` en fin de contenu.
    if let Some(b) = pseudo_box(styles, parent, PseudoElement::After) {
        out.push(b);
    }
    out
}

/// Boîte générée par `::before`/`::after` (R-13, CSS2 §12) :
/// produite si la cascade a un style pseudo avec un `content`
/// générable — `none`/`normal` ou valeur non gérée (`url()`,
/// `attr()`, compteurs, guillemets) → aucune boîte. `node` =
/// `None` (pas de nœud DOM — hit-test et relayout remontent
/// à l'élément origine).
fn pseudo_box(
    styles: &StyleMap,
    node: NodeId,
    which: explorer_css::PseudoElement,
) -> Option<LayoutBox> {
    let st = styles.pseudo(node, which)?;
    let text = pseudo_content(st.get("content")?)?;
    let kind = box_kind_of(st);
    let mut bx = leaf_box_anon(kind, st.clone());
    if !text.is_empty() {
        let t = leaf_box_anon(BoxKind::Text, st.clone());
        let mut t = t;
        t.text = Some(text);
        // Texte nu dans un bloc : enveloppe anonyme (R-21) ;
        // dans un inline, le flux le collecte directement.
        bx.children = if kind == BoxKind::Block {
            wrap_anonymous(vec![t], st)
        } else {
            vec![t]
        };
    }
    Some(bx)
}

/// `content` → texte généré : concatène les chaînes
/// `'…'`/`"…"` ; `none`/`normal` ou valeurs non couvertes
/// → `None` (pas de boîte). `""` → boîte vide (ses
/// bordures/dimensions peignent quand même — Acid2 nose).
fn pseudo_content(raw: &str) -> Option<String> {
    let t = raw.trim();
    if t.eq_ignore_ascii_case("none") || t.eq_ignore_ascii_case("normal") {
        return None;
    }
    // CSS Content-3 : `content: <valeur> / <texte-alt>` — la
    // partie après le `/` de niveau 0 est l'alternative
    // d'accessibilité, jamais peinte (MDN :
    // `content:"\200b"/" (external)"` rendait « (external) »
    // en clair, R-80). `/` hors guillemets/parenthèses.
    let t = {
        let mut parens = 0i32;
        let mut quote = 0u8;
        let mut slash = t.len();
        for (i, b) in t.bytes().enumerate() {
            match b {
                b if quote != 0 => {
                    if b == quote {
                        quote = 0;
                    }
                }
                b'"' | b'\'' => quote = b,
                b'(' => parens += 1,
                b')' => parens -= 1,
                b'/' if parens == 0 => {
                    slash = i;
                    break;
                }
                _ => {}
            }
        }
        &t[..slash]
    };
    let mut out = String::new();
    let mut rest = t;
    while let Some(qpos) = rest.find(['\'', '"']) {
        let q = rest.as_bytes()[qpos];
        let after = &rest[qpos + 1..];
        let end = after.find(q as char)?;
        out.push_str(&after[..end]);
        rest = &after[end + 1..];
    }
    // Reste non-chaîne (counter, attr…) : si rien d'extrait et
    // qu'une valeur non-rien traîne → pas de boîte.
    if out.is_empty() && rest.trim().is_empty() {
        return Some(out);
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

/// Feuille sans `node` (pseudo-éléments) — même gabarit que
/// [`leaf_box`].
fn leaf_box_anon(kind: BoxKind, style: ComputedStyle) -> LayoutBox {
    LayoutBox {
        kind,
        node: None,
        style,
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
        slot: BoxSlot::default(),
        height_raised: false,
        fc_root: false,
    }
}

/// Enfants d'un conteneur **flex/grid** (flexbox §4 / grid
/// §5.1.1) : chaque enfant in-flow devient un item — on ne
/// fusionne JAMAIS des éléments dans un bloc anonyme commun.
/// Seules les suites de **texte nu** sont groupées en item
/// anonyme propre ; le blanc pur disparaît.
fn wrap_items(children: Vec<LayoutBox>, parent_style: &ComputedStyle) -> Vec<LayoutBox> {
    let mut out = Vec::new();
    let mut run: Vec<LayoutBox> = Vec::new();
    for b in children {
        if b.kind == BoxKind::Text {
            run.push(b);
        } else {
            flush_run(&mut out, &mut run, parent_style);
            out.push(b);
        }
    }
    flush_run(&mut out, &mut run, parent_style);
    out
}

/// Post-traitement des enfants selon le conteneur :
/// flex/grid → items (§9.7 blockification), autre bloc →
/// anonymisation des suites en-ligne (§9.2.1.1), inline →
/// enfants directs — **sauf** l'inline **atomique**
/// (`inline-block`/`inline-table`/remplacé), qui établit un
/// contexte de formatage bloc : ses enfants sont anonymisés
/// pareil (sinon les blancs inter-blocs compteraient chacun
/// une ligne pleine au flux bloc — R-22 : `details.caches`
/// de lobste.rs, hauteur 4× la ligne).
fn wrap_children(children: Vec<LayoutBox>, style: &ComputedStyle, kind: BoxKind) -> Vec<LayoutBox> {
    let fc = matches!(
        style.get("display"),
        Some("flex" | "inline-flex" | "grid" | "inline-grid")
    );
    let children: Vec<LayoutBox> = if fc {
        // §9.7 / CSS Display §2.7 : blockification — chaque
        // enfant en-ligne d'un conteneur flex/grid devient un
        // item de niveau bloc ; le split block-in-inline ne
        // s'applique PAS à un item (sinon un
        // `display:initial` contenant un bloc — le motif
        // styled-components de bbc.com — verrait son contenu
        // hissé comme item auto-placé à 1 piste).
        children.into_iter().map(blockify_item).collect()
    } else {
        // §9.2.1.1 (2ᵉ moitié) : un bloc en flux niché dans une
        // boîte inline éclate celle-ci — hissage récursif au niveau
        // du conteneur AVANT anonymisation, pour que le bloc
        // coupe proprement la suite en-ligne (R-22).
        children.into_iter().flat_map(hoist_blocks).collect()
    };
    let atomic_inline = kind == BoxKind::Inline
        && matches!(
            style.get("display"),
            Some("inline-block" | "inline-flex" | "inline-grid" | "inline-table")
        );
    if kind.is_inline_level() && !atomic_inline {
        return children;
    }
    if fc {
        wrap_items(children, style)
    } else {
        wrap_anonymous(children, style)
    }
}

/// §9.7 / CSS Display §2.7 : blockification d'un item
/// flex/grid — un item `display:inline` (y compris
/// `display:initial`, mot-clé CSS-wide résolu en `inline`)
/// produit une boîte de niveau bloc dont les enfants passent
/// par le chemin normal (`wrap_children` : hoist +
/// anonymisation, ou blockification récursive si l'item est
/// lui-même un conteneur `inline-flex`/`inline-grid`).
fn blockify_item(mut bx: LayoutBox) -> LayoutBox {
    if bx.kind == BoxKind::Inline {
        bx.kind = BoxKind::Block;
        let st = bx.style.clone();
        bx.children = wrap_children(std::mem::take(&mut bx.children), &st, bx.kind);
    }
    bx
}

/// §9.2.1.1 : un enfant **bloc en flux** d'une boîte de flux
/// en-ligne (`inline` pur, ou bloc anonyme après
/// anonymisation) la scinde en morceaux autour de lui — le
/// bloc est hissé au niveau du parent. La récursion fait
/// remonter un bloc profond niveau par niveau (span → a →
/// conteneur). Ne se scindent pas : les atomes
/// (`inline-block`, `inline-table`, `inline-flex`, remplacés)
/// — contexte de formatage propre — ni les boîtes hors-flux
/// (`absolute`/`fixed`). (R-22 : `span.dropdown_parent >
/// div` de lobste.rs gonflait sa ligne à ~93 px.)
fn hoist_blocks(bx: LayoutBox) -> Vec<LayoutBox> {
    let mut bx = bx;
    // `take` : un move de champ est impossible (`LayoutBox`
    // implémente `Drop` — itératif, R-24).
    let kids = std::mem::take(&mut bx.children);
    bx.children = kids
        .into_iter()
        // Arête ∝ profondeur de l'arbre → pile garantie (R-24).
        .flat_map(|c| explorer_common::deep(|| hoist_blocks(c)))
        .collect();
    let inline_container = bx.kind == BoxKind::AnonymousBlock
        || (bx.kind == BoxKind::Inline
            && bx.intrinsic.is_none()
            && !matches!(
                bx.display(),
                "inline-block" | "inline-flex" | "inline-grid" | "inline-table"
            )
            && !bx.out_of_flow());
    if !inline_container || !bx.children.iter().any(is_inflow_block) {
        return vec![bx];
    }
    let mut out = Vec::new();
    let mut seg: Vec<LayoutBox> = Vec::new();
    for c in std::mem::take(&mut bx.children) {
        if is_inflow_block(&c) {
            if !seg.is_empty() && !all_whitespace(&seg) {
                out.push(split_piece(&bx, &mut seg));
            } else {
                seg.clear();
            }
            out.push(c);
        } else {
            seg.push(c);
        }
    }
    if !seg.is_empty() && !all_whitespace(&seg) {
        out.push(split_piece(&bx, &mut seg));
    }
    out
}

/// Morceau d'une boîte inline scindée : clone de `bx` vidé
/// de sa pose (rect/lignes), portant `seg` comme enfants —
/// `..bx` est impossible (`LayoutBox` implémente `Drop`,
/// drop itératif R-24).
fn split_piece(bx: &LayoutBox, seg: &mut Vec<LayoutBox>) -> LayoutBox {
    let mut p = bx.clone();
    p.children = std::mem::take(seg);
    p.lines.clear();
    p.rect = Rect::default();
    p
}

/// `true` pour un enfant de niveau bloc **en flux** — il
/// échappe au flux en-ligne (un flottant ou un positionné
/// reste à sa place dans la boîte inline). `position`/`float`
/// se lisent sur le style calculé : les champs de `LayoutBox`
/// ne sont renseignés qu'au layout (`apply_layout_metadata`).
fn is_inflow_block(c: &LayoutBox) -> bool {
    !c.out_of_flow()
        && !matches!(c.style.get("float"), Some("left" | "right"))
        && !c.kind.is_inline_level()
}

/// Suite entièrement blanche — un morceau de scission qui ne
/// contient que des espaces ne produit pas de boîte (même
/// règle que `flush_run`).
fn all_whitespace(seg: &[LayoutBox]) -> bool {
    seg.iter().all(|b| {
        b.kind == BoxKind::Text
            && b.text
                .as_deref()
                .is_some_and(|t| crate::text::trim_css_ws(t).is_empty())
    })
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
            slot: BoxSlot::default(),
            height_raised: false,
            fc_root: false,
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

/// Taille intrinsèque de repli d'un `<svg>` (R-9) : attributs
/// `width`/`height` (px ou nombre nu — `%` ignoré, pas de
/// repère à cette étape), puis aspect du `viewBox`, enfin le
/// défaut CSS des remplacés 300×150.
/// `<object>` (R-13c) : élément **remplacé** si sa ressource
/// est résolue (`sizes` la connaît — `<img>`-like), sinon
/// ses **enfants se substituent** (repli HTML 4.01 §13.3 :
/// la cascade de `<object>` imbriqués descend jusqu'au
/// premier rendable — Acid2 en chaîne trois). Le conteneur
/// de repli est transparent (pas de fond/bordure propre).
fn object_box(
    dom: &Dom,
    root: NodeId,
    style: &ComputedStyle,
    styles: &StyleMap,
    sizes: &std::collections::HashMap<NodeId, (f32, f32), impl ::std::hash::BuildHasher>,
) -> LayoutBox {
    let kind = box_kind_of(style);
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
        intrinsic: sizes.get(&root).copied(),
        control_checked: false,
        slot: BoxSlot::default(),
        height_raised: false,
        fc_root: false,
    };
    // Ressource résolue → feuille remplacée ; sinon les
    // enfants prennent la place de l'objet.
    if b.intrinsic.is_none() {
        let children = child_boxes(dom, root, styles, style, sizes);
        b.children = wrap_children(children, style, kind);
    }
    b
}

fn svg_intrinsic(dom: &Dom, node: NodeId) -> (f32, f32) {
    let attr_px = |name: &str| {
        dom.attribute(node, name).ok().flatten().and_then(|v| {
            let t = v.trim();
            if t.ends_with('%') {
                return None;
            }
            t.trim_end_matches("px")
                .parse::<f32>()
                .ok()
                .filter(|f| f.is_finite() && *f > 0.0)
        })
    };
    let vb = dom.attribute(node, "viewBox").ok().flatten().and_then(|v| {
        let nums: Vec<f32> = v
            .split([',', ' ', '\t'])
            .filter(|s| !s.is_empty())
            .filter_map(|s| s.parse().ok())
            .collect();
        (nums.len() == 4).then(|| (nums[2], nums[3]))
    });
    let (w, h) = (attr_px("width"), attr_px("height"));
    match (w, h, vb) {
        (Some(w), Some(h), _) => (w, h),
        (Some(w), None, Some((vw, vh))) if vw > 0.0 => (w, w * vh / vw),
        (None, Some(h), Some((vw, vh))) if vh > 0.0 => (h * vw / vh, h),
        (None, None, Some((vw, vh))) if vw > 0.0 && vh > 0.0 => (vw, vh),
        (Some(w), None, _) => (w, 150.0),
        (None, Some(h), _) => (300.0, h),
        _ => (300.0, 150.0),
    }
}

/// Bras `<img>` de [`build_box_tree_sized`] : remplacé si
/// l'image est résolue (`sizes`), boîte texte `alt` sinon.
fn img_box<S: ::std::hash::BuildHasher>(
    dom: &Dom,
    node: NodeId,
    style: ComputedStyle,
    sizes: &std::collections::HashMap<NodeId, (f32, f32), S>,
) -> LayoutBox {
    let alt = dom
        .attribute(node, "alt")
        .ok()
        .flatten()
        .unwrap_or_default()
        .clone();
    let (kind, text) = match sizes.get(&node) {
        Some(_) => (box_kind_of(&style), None),
        None => (BoxKind::Text, (!alt.is_empty()).then_some(alt)),
    };
    let mut bx = leaf_box(kind, node, style, sizes.get(&node).copied());
    bx.text = text;
    bx
}

/// Boîte feuille sans enfants ni texte — éléments atomiques
/// (`<svg>` R-9, contrôles vides) : même gabarit que le bras
/// `<img>`.
fn leaf_box(
    kind: BoxKind,
    node: NodeId,
    style: ComputedStyle,
    intrinsic: Option<(f32, f32)>,
) -> LayoutBox {
    LayoutBox {
        kind,
        node: Some(node),
        style,
        text: None,
        children: Vec::new(),
        lines: Vec::new(),
        rect: Rect::default(),
        position: Position::Static,
        float: Float::None,
        overflow: Overflow::Visible,
        z_index: 0,
        intrinsic,
        control_checked: false,
        slot: BoxSlot::default(),
        height_raised: false,
        fc_root: false,
    }
}

/// Taille intrinsèque d'un `<canvas>` (R-11) : attributs
/// `width`/`height` en entiers CSS px (valeur non positive ou
/// non numérique → repli), défaut spec 300×150.
fn canvas_intrinsic(dom: &Dom, node: NodeId) -> (f32, f32) {
    let attr_px = |name: &str, def: f32| {
        dom.attribute(node, name)
            .ok()
            .flatten()
            .and_then(|v| v.trim().parse::<f32>().ok())
            .filter(|f| f.is_finite() && *f > 0.0)
            .unwrap_or(def)
    };
    (attr_px("width", 300.0), attr_px("height", 150.0))
}

/// Bras `input`/`select` : boîte atomique + valeur en enfant
/// `Text` **enveloppé d'un `AnonymousBlock`** (R-21 — un texte
/// nu ne rentrait jamais dans `lines`, la valeur tapée restait
/// invisible). `checked` = marque peinte au rendu.
fn control_box(
    node: NodeId,
    style: ComputedStyle,
    (text, checked): (Option<String>, bool),
) -> LayoutBox {
    let children = text
        .map(|t| {
            wrap_anonymous(
                vec![LayoutBox {
                    kind: BoxKind::Text,
                    node: Some(node),
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
                    slot: BoxSlot::default(),
                    height_raised: false,
                    fc_root: false,
                }],
                &style,
            )
        })
        .unwrap_or_default();
    let mut bx = leaf_box(box_kind_of(&style), node, style, None);
    bx.children = children;
    bx.control_checked = checked;
    bx
}

/// Bas de la région défilante propagée à la racine — pendant
/// de `scrollHeight`. `rect.height` de la racine ne suffit
/// pas : un contenu qui déborde un ancêtre à hauteur fixe
/// (`html,body{height:100%}` de vector-2022 — R-82) reste
/// visible et défilable ; la zone scrollable du document suit
/// le bord inférieur de toute boîte dont le débord n'est pas
/// clippé. Une boîte `overflow` ≠ visible clippe : ses
/// enfants ne contribuent pas au-delà de sa propre bordure.
/// `fixed` est ancré au viewport — exclu du total (un bandeau
/// fixe en bas de fenêtre ne doit pas allonger la page).
/// Récursion bornée par `deep` (R-24 — arbres hostiles).
#[must_use]
pub fn scrollable_overflow_bottom(root: &LayoutBox) -> f32 {
    fn visit(bx: &LayoutBox, clip_bottom: f32) -> f32 {
        let own_bottom = (bx.rect.y + bx.rect.height).min(clip_bottom);
        if bx.position == Position::Fixed {
            return own_bottom;
        }
        let child_clip = if bx.overflow == Overflow::Visible {
            clip_bottom
        } else {
            own_bottom
        };
        bx.children.iter().fold(own_bottom, |acc, c| {
            acc.max(explorer_common::deep(|| visit(c, child_clip)))
        })
    }
    visit(root, f32::INFINITY)
}
