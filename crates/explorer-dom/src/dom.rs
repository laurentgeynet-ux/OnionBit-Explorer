// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Arène DOM — allocation générationnelle, arbre, rooting ADR-0008.

use std::collections::HashMap;

use crate::error::{DomError, Result};
use crate::id::{NodeId, RootId};
use crate::node::{Attribute, DoctypeData, ElementData, Namespace, Node, NodeData};

/// Sentinelle de free-list (« pas de case libre »).
const NIL: u32 = u32::MAX;

/// Budget de nœuds par défaut — borne anti-contenu hostile
/// (ADR-0011). Configurable via [`Dom::with_max_nodes`].
pub const DEFAULT_MAX_NODES: usize = 1_000_000;

/// Case de l'arène : occupée par un nœud ou libre (free-list
/// chaînée). La `generation` survit à la réallocation — c'est elle
/// qui invalide les `NodeId` obsolètes.
#[derive(Debug)]
enum Slot {
    Vacant { generation: u32, next_free: u32 },
    Occupied { generation: u32, node: Node },
}

/// Mode de compatibilité du document, tranché par le DOCTYPE
/// au parsing (spec HTML « quirks mode and limited-quirks
/// mode »). La cascade le consulte pour les règles UA
/// conditionnelles — en quirks, une table réinitialise les
/// propriétés texte héritées.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DocumentMode {
    /// Doctype conforme (ou décision non prise) — mode standard.
    #[default]
    NoQuirks,
    /// Doctype legacy partiellement conforme.
    LimitedQuirks,
    /// Doctype absent ou invalide — mode quirks.
    Quirks,
}

/// Arène DOM à handles générationnels (ADR-0008).
///
/// Invariants :
/// - le nœud `Document` vit à l'index 0, jamais collecté ;
/// - toute case réattribuée a une `generation` strictement
///   incrémentée — un `NodeId` périmé échoue typé, jamais d'accès
///   à un objet réutilisé ;
/// - [`Dom::remove`] détache sans détruire (sémantique DOM : un
///   nœud détaché reste valide) ; seule [`Dom::collect`] réclame
///   les sous-arbres inaccessibles depuis le document **et** non
///   rootés ;
/// - les cycles DOM↔JS passent par la table `roots` — jamais par
///   des liens forts dans les nœuds.
pub struct Dom {
    slots: Vec<Slot>,
    free_head: u32,
    live: usize,
    max_nodes: usize,
    roots: HashMap<RootId, NodeId>,
    next_root: u64,
    document_mode: DocumentMode,
    /// Hôte → (racine shadow, mode ouvert) — R-96 (Shadow T2).
    /// La racine est un fragment [`NodeData::Document`] détaché
    /// (jamais enfant de l'hôte — l'arbre shadow vit hors de la
    /// liste `children` ; la traversée « flat tree » l'atteint
    /// via [`Dom::shadow_root`]).
    shadow_roots: HashMap<NodeId, (NodeId, bool)>,
    /// Racine shadow → hôte — inverse de `shadow_roots`, pour
    /// `host` JS et le scoping cascade (`:host`, remontée des
    /// ancêtres hors de l'arbre).
    shadow_hosts: HashMap<NodeId, NodeId>,
}

impl Dom {
    /// Arène neuve (budget [`DEFAULT_MAX_NODES`]) contenant le nœud
    /// `Document` à l'index 0.
    #[must_use]
    pub fn new() -> Self {
        Self::with_max_nodes(DEFAULT_MAX_NODES)
    }

    /// Arène neuve avec budget de nœuds explicite.
    #[must_use]
    pub fn with_max_nodes(max_nodes: usize) -> Self {
        let mut dom = Self {
            slots: Vec::new(),
            free_head: NIL,
            live: 0,
            max_nodes,
            roots: HashMap::new(),
            next_root: 0,
            document_mode: DocumentMode::NoQuirks,
            shadow_roots: HashMap::new(),
            shadow_hosts: HashMap::new(),
        };
        // Index 0 sur arène vide : ne peut pas échouer.
        let _ = dom.alloc(NodeData::Document);
        dom
    }

    /// Handle du nœud `Document` (racine implicite, index 0).
    #[must_use]
    pub fn document(&self) -> NodeId {
        NodeId::new(0, 1)
    }

    /// Mode de compatibilité du document (quirks) — fixé par
    /// le tree-builder HTML quand le DOCTYPE est tranché.
    #[must_use]
    pub fn document_mode(&self) -> DocumentMode {
        self.document_mode
    }

    /// Enregistre le mode quirks décidé au parsing.
    pub fn set_document_mode(&mut self, mode: DocumentMode) {
        self.document_mode = mode;
    }

    /// Nombre de nœuds vivants (occupés) dans l'arène.
    #[must_use]
    pub fn live_count(&self) -> usize {
        self.live
    }

    /// Budget de nœuds configuré.
    #[must_use]
    pub fn max_nodes(&self) -> usize {
        self.max_nodes
    }

    // -----------------------------------------------------------------
    // Création — nœuds orphelins, à rattacher par append/insert_before.
    // -----------------------------------------------------------------

    /// Crée un élément détaché (`create_element("div")`).
    ///
    /// # Errors
    /// [`DomError::BudgetExceeded`] si le budget de nœuds est atteint.
    pub fn create_element(&mut self, tag_name: &str) -> Result<NodeId> {
        self.alloc(NodeData::Element(ElementData::new(tag_name)))
    }

    /// Crée un élément détaché dans un namespace étranger
    /// (SVG/MathML — contenu étranger du tree-builder, R-15).
    ///
    /// # Errors
    /// [`DomError::BudgetExceeded`] si le budget de nœuds est atteint.
    pub fn create_element_ns(&mut self, tag_name: &str, ns: Namespace) -> Result<NodeId> {
        self.alloc(NodeData::Element(ElementData::with_namespace(tag_name, ns)))
    }

    /// Crée un **fragment de document** détaché — contenu propre
    /// d'un `<template>` (R-15) : pas enfant du template mais
    /// référencé par son `template_contents`.
    ///
    /// # Errors
    /// [`DomError::BudgetExceeded`] si le budget de nœuds est atteint.
    pub fn create_fragment(&mut self) -> Result<NodeId> {
        self.alloc(NodeData::Document)
    }

    /// Crée un nœud texte détaché — contenu stocké **tel quel**
    /// (la normalisation `\r\n`/`\r` → `\n` relève du prétraitement
    /// d'entrée du tokenizer HTML, pas du DOM — spec §parsing).
    ///
    /// # Errors
    /// [`DomError::BudgetExceeded`] si le budget de nœuds est atteint.
    pub fn create_text(&mut self, data: &str) -> Result<NodeId> {
        self.alloc(NodeData::Text(data.to_owned()))
    }

    /// Crée un commentaire détaché.
    ///
    /// # Errors
    /// [`DomError::BudgetExceeded`] si le budget de nœuds est atteint.
    pub fn create_comment(&mut self, data: &str) -> Result<NodeId> {
        self.alloc(NodeData::Comment(data.to_owned()))
    }

    /// Crée un `<!DOCTYPE>` détaché.
    ///
    /// # Errors
    /// [`DomError::BudgetExceeded`] si le budget de nœuds est atteint.
    pub fn create_doctype(
        &mut self,
        name: &str,
        public_id: &str,
        system_id: &str,
    ) -> Result<NodeId> {
        self.alloc(NodeData::Doctype(DoctypeData {
            name: name.to_owned(),
            public_id: public_id.to_owned(),
            system_id: system_id.to_owned(),
        }))
    }

    // -----------------------------------------------------------------
    // Accès typé
    // -----------------------------------------------------------------

    /// Contenu typé du nœud.
    ///
    /// # Errors
    /// [`DomError::StaleHandle`] si le handle est périmé (génération
    /// dépassée par une collecte), [`DomError::OutOfBounds`] si son
    /// index déborde l'arène.
    pub fn data(&self, id: NodeId) -> Result<&NodeData> {
        Ok(&self.node(id)?.data)
    }

    /// `true` si le handle désigne un nœud encore vivant.
    #[must_use]
    pub fn is_alive(&self, id: NodeId) -> bool {
        matches!(
            self.slots.get(slot_idx(id.index())),
            Some(Slot::Occupied { generation, .. }) if *generation == id.generation()
        )
    }

    /// Données d'élément.
    ///
    /// # Errors
    /// [`DomError::WrongKind`] si le nœud n'est pas un élément ;
    /// erreurs de handle comme [`Dom::data`].
    pub fn element(&self, id: NodeId) -> Result<&ElementData> {
        match self.data(id)? {
            NodeData::Element(el) => Ok(el),
            _ => Err(DomError::WrongKind {
                id,
                expected: "Element",
            }),
        }
    }

    /// Accès mutable aux données d'élément.
    ///
    /// # Errors
    /// Comme [`Dom::element`].
    pub fn element_mut(&mut self, id: NodeId) -> Result<&mut ElementData> {
        match &mut self.node_mut(id)?.data {
            NodeData::Element(el) => Ok(el),
            _ => Err(DomError::WrongKind {
                id,
                expected: "Element",
            }),
        }
    }

    /// Texte d'un nœud `Text` ou `Comment`.
    ///
    /// # Errors
    /// [`DomError::WrongKind`] sur autre type ; erreurs de handle
    /// comme [`Dom::data`].
    pub fn text(&self, id: NodeId) -> Result<&str> {
        match self.data(id)? {
            NodeData::Text(t) | NodeData::Comment(t) => Ok(t),
            _ => Err(DomError::WrongKind {
                id,
                expected: "Text | Comment",
            }),
        }
    }

    /// Remplace le texte d'un nœud `Text`/`Comment` (contenu tel
    /// quel — voir [`Dom::create_text`]).
    ///
    /// # Errors
    /// Comme [`Dom::text`].
    pub fn set_text(&mut self, id: NodeId, data: &str) -> Result<()> {
        let node = self.node_mut(id)?;
        match &mut node.data {
            NodeData::Text(t) | NodeData::Comment(t) => data.clone_into(t),
            _ => {
                return Err(DomError::WrongKind {
                    id,
                    expected: "Text | Comment",
                });
            }
        }
        Ok(())
    }

    /// `textContent` : concatène les `Text` descendants (commentaires
    /// exclus) ; pour `Text`/`Comment`, leur propre donnée.
    ///
    /// # Errors
    /// Erreurs de handle comme [`Dom::data`].
    pub fn text_content(&self, id: NodeId) -> Result<String> {
        let mut out = String::new();
        match self.data(id)? {
            NodeData::Text(t) | NodeData::Comment(t) => out.push_str(t),
            NodeData::Document | NodeData::Element(_) | NodeData::Doctype(_) => {
                self.collect_text(id, &mut out)?;
            }
        }
        Ok(out)
    }

    fn collect_text(&self, id: NodeId, out: &mut String) -> Result<()> {
        let mut cursor = self.first_child(id)?;
        while let Some(child) = cursor {
            match self.data(child)? {
                NodeData::Text(t) => out.push_str(t),
                // Descente bornée-pile : un DOM hostile peut
                // nicher des milliers d'éléments (R-24).
                NodeData::Element(_) => explorer_common::deep(|| self.collect_text(child, out))?,
                NodeData::Document | NodeData::Doctype(_) | NodeData::Comment(_) => {}
            }
            cursor = self.next_sibling(child)?;
        }
        Ok(())
    }

    // -----------------------------------------------------------------
    // Navigation — `Ok(None)` quand le lien est absent.
    // -----------------------------------------------------------------

    /// Parent du nœud, s'il est rattaché.
    ///
    /// # Errors
    /// Erreurs de handle comme [`Dom::data`].
    pub fn parent(&self, id: NodeId) -> Result<Option<NodeId>> {
        Ok(self.node(id)?.parent)
    }

    /// Premier enfant.
    ///
    /// # Errors
    /// Erreurs de handle comme [`Dom::data`].
    pub fn first_child(&self, id: NodeId) -> Result<Option<NodeId>> {
        Ok(self.node(id)?.first_child)
    }

    /// Dernier enfant.
    ///
    /// # Errors
    /// Erreurs de handle comme [`Dom::data`].
    pub fn last_child(&self, id: NodeId) -> Result<Option<NodeId>> {
        Ok(self.node(id)?.last_child)
    }

    /// Sibling précédent.
    ///
    /// # Errors
    /// Erreurs de handle comme [`Dom::data`].
    pub fn prev_sibling(&self, id: NodeId) -> Result<Option<NodeId>> {
        Ok(self.node(id)?.prev_sibling)
    }

    /// Sibling suivant.
    ///
    /// # Errors
    /// Erreurs de handle comme [`Dom::data`].
    pub fn next_sibling(&self, id: NodeId) -> Result<Option<NodeId>> {
        Ok(self.node(id)?.next_sibling)
    }

    /// Enfants dans l'ordre du document.
    ///
    /// # Errors
    /// Erreurs de handle comme [`Dom::data`].
    pub fn children(&self, id: NodeId) -> Result<Vec<NodeId>> {
        let mut out = Vec::new();
        let mut cursor = self.first_child(id)?;
        while let Some(child) = cursor {
            out.push(child);
            cursor = self.next_sibling(child)?;
        }
        Ok(out)
    }

    /// Premier enfant **élément** du `Document` — l'« élément
    /// racine » (`<html>` en pratique).
    ///
    /// `None` tant que le document **en construction** n'a produit
    /// que des nœuds non-élément (doctype, commentaires — les
    /// blancs de préambule n'atteignent jamais l'arbre) : un
    /// document partiel n'a pas de racine à peindre, ce n'est pas
    /// une erreur (peinture progressive, R-1).
    #[must_use]
    pub fn document_element(&self) -> Option<NodeId> {
        let mut cursor = self.first_child(self.document()).ok()?;
        while let Some(id) = cursor {
            if self.element(id).is_ok() {
                return Some(id);
            }
            cursor = self.next_sibling(id).ok()?;
        }
        None
    }

    /// `true` si `ancestor` est un ascendant strict de `id`.
    ///
    /// # Errors
    /// Erreurs de handle comme [`Dom::data`].
    pub fn is_ancestor(&self, ancestor: NodeId, id: NodeId) -> Result<bool> {
        let mut cursor = self.parent(id)?;
        while let Some(cur) = cursor {
            if cur == ancestor {
                return Ok(true);
            }
            cursor = self.parent(cur)?;
        }
        Ok(false)
    }

    // -----------------------------------------------------------------
    // Mutations d'arbre
    // -----------------------------------------------------------------

    /// Attache `child` en dernier enfant de `parent`. Un enfant déjà
    /// rattaché est déplacé (sémantique `appendChild`).
    ///
    /// # Errors
    /// [`DomError::HierarchyRequest`] si `parent` n'accepte pas
    /// d'enfant (`Text`, `Comment`, `Doctype`), si `child` est le
    /// `Document` ou si l'insertion crée un cycle ; erreurs de
    /// handle comme [`Dom::data`].
    pub fn append_child(&mut self, parent: NodeId, child: NodeId) -> Result<()> {
        self.insert_before(parent, child, None)
    }

    /// Attache `child` avant `reference` (enfant de `parent`) ;
    /// `None` = en fin de liste.
    ///
    /// # Errors
    /// Comme [`Dom::append_child`], plus `HierarchyRequest` si
    /// `reference` n'est pas un enfant de `parent`.
    pub fn insert_before(
        &mut self,
        parent: NodeId,
        child: NodeId,
        reference: Option<NodeId>,
    ) -> Result<()> {
        match self.data(parent)? {
            NodeData::Document | NodeData::Element(_) => {}
            _ => {
                return Err(DomError::HierarchyRequest(
                    "le parent n'accepte pas d'enfant",
                ));
            }
        }
        if matches!(self.data(child)?, NodeData::Document) {
            return Err(DomError::HierarchyRequest(
                "le document ne peut pas être un enfant",
            ));
        }
        if parent == child || self.is_ancestor(child, parent)? {
            return Err(DomError::HierarchyRequest("cycle interdit"));
        }
        if let Some(reference) = reference {
            if self.parent(reference)? != Some(parent) {
                return Err(DomError::HierarchyRequest(
                    "la référence n'est pas un enfant du parent",
                ));
            }
        }
        self.unlink(child)?;
        self.link_child(parent, child, reference)?;
        Ok(())
    }

    /// Détache `id` de son parent — le sous-arbre **reste vivant**
    /// (sémantique DOM : un nœud retiré reste manipulable). La
    /// réclamation passe par [`Dom::collect`].
    ///
    /// # Errors
    /// `HierarchyRequest` sur le nœud `Document` ; erreurs de
    /// handle comme [`Dom::data`].
    pub fn remove(&mut self, id: NodeId) -> Result<()> {
        if matches!(self.data(id)?, NodeData::Document) {
            return Err(DomError::HierarchyRequest(
                "le document ne peut pas être retiré",
            ));
        }
        self.unlink(id)
    }

    // -----------------------------------------------------------------
    // Attributs (éléments uniquement)
    // -----------------------------------------------------------------

    /// Valeur d'attribut par nom exact.
    ///
    /// # Errors
    /// Comme [`Dom::element`].
    pub fn attribute(&self, id: NodeId, name: &str) -> Result<Option<String>> {
        Ok(self
            .element(id)?
            .attributes
            .iter()
            .find(|a| a.name == name)
            .map(|a| a.value.clone()))
    }

    /// Attributs ordonnés d'un élément.
    ///
    /// # Errors
    /// Comme [`Dom::element`].
    pub fn attributes(&self, id: NodeId) -> Result<&[Attribute]> {
        Ok(&self.element(id)?.attributes)
    }

    /// Pose ou remplace un attribut (ordre d'insertion préservé).
    ///
    /// # Errors
    /// `WrongKind` si le nœud n'est pas un élément.
    pub fn set_attribute(&mut self, id: NodeId, name: &str, value: &str) -> Result<()> {
        let el = self.element_mut(id)?;
        match el.attributes.iter_mut().find(|a| a.name == name) {
            Some(attr) => value.clone_into(&mut attr.value),
            None => el.attributes.push(Attribute {
                name: name.to_owned(),
                value: value.to_owned(),
            }),
        }
        Ok(())
    }

    /// Retire un attribut ; `true` s'il existait.
    ///
    /// # Errors
    /// `WrongKind` si le nœud n'est pas un élément.
    pub fn remove_attribute(&mut self, id: NodeId, name: &str) -> Result<bool> {
        let el = self.element_mut(id)?;
        let before = el.attributes.len();
        el.attributes.retain(|a| a.name != name);
        Ok(el.attributes.len() != before)
    }

    /// Nom de balise d'un élément.
    ///
    /// # Errors
    /// Comme [`Dom::element`].
    pub fn tag_name(&self, id: NodeId) -> Result<&str> {
        Ok(&self.element(id)?.tag_name)
    }

    /// Namespace d'un élément (`Html` par défaut — R-15).
    ///
    /// # Errors
    /// Comme [`Dom::element`].
    pub fn namespace(&self, id: NodeId) -> Result<Namespace> {
        Ok(self.element(id)?.namespace)
    }

    /// Fragment `content` d'un `<template>` (R-15) — `None` si
    /// l'élément n'est pas un template ou sans fragment.
    ///
    /// # Errors
    /// Comme [`Dom::element`].
    pub fn template_contents(&self, id: NodeId) -> Result<Option<NodeId>> {
        Ok(self.element(id)?.template_contents)
    }

    /// Pose le fragment `content` d'un `<template>` (R-15).
    ///
    /// # Errors
    /// Comme [`Dom::element_mut`].
    pub fn set_template_contents(&mut self, id: NodeId, fragment: NodeId) -> Result<()> {
        self.element_mut(id)?.template_contents = Some(fragment);
        Ok(())
    }

    // -----------------------------------------------------------------
    // Racines shadow (R-96 — couture Shadow DOM T2)
    // -----------------------------------------------------------------

    /// Attache la racine shadow `root` à l'hôte `host` —
    /// `open` = `mode: "open"` de l'init. La racine est un
    /// fragment ordinaire (ses enfants sont dans `children`) ;
    /// le lien hôte↔racine vit ici, hors arbre (un `attachShadow`
    /// répété écrase — le tree-builder DSD garantit l'unicité,
    /// la surface JS borne ailleurs).
    pub fn attach_shadow_root(&mut self, host: NodeId, root: NodeId, open: bool) {
        self.shadow_roots.insert(host, (root, open));
        self.shadow_hosts.insert(root, host);
    }

    /// Racine shadow de `host` — `None` si aucune n'est attachée.
    /// `include_closed` expose aussi le mode `closed` (surface
    /// JS `ElementInternals` — la cascade et le layout lisent
    /// toujours les deux modes : le contenu se peint pareil).
    #[must_use]
    pub fn shadow_root(&self, host: NodeId, include_closed: bool) -> Option<NodeId> {
        self.shadow_roots
            .get(&host)
            .and_then(|&(r, open)| (open || include_closed).then_some(r))
    }

    /// Hôte de la racine shadow `root` — `None` si `root` n'est
    /// pas une racine attachée.
    #[must_use]
    pub fn shadow_host(&self, root: NodeId) -> Option<NodeId> {
        self.shadow_hosts.get(&root).copied()
    }

    /// `true` si `node` est une racine shadow attachée.
    #[must_use]
    pub fn is_shadow_root(&self, node: NodeId) -> bool {
        self.shadow_hosts.contains_key(&node)
    }

    /// Racine shadow englobant `node` — `None` si `node` vit dans
    /// l'arbre document (non-scopé). Marche de parent en parent :
    /// la racine shadow est le sommet de la chaîne (son `parent`
    /// est `None`, comme le `Document` — c'est `shadow_hosts`
    /// qui la distingue).
    #[must_use]
    pub fn containing_shadow_root(&self, node: NodeId) -> Option<NodeId> {
        let mut cur = node;
        loop {
            if self.is_shadow_root(cur) {
                return Some(cur);
            }
            cur = self.parent(cur).ok()??;
        }
    }

    /// Les `<slot>` de l'arbre shadow `root`, en préordre —
    /// base de la distribution (R-97, flat tree). Les racines
    /// shadow imbriquées sont latérales (hors `children`) :
    /// la descente n'y entre jamais. Pas d'aplatissement de
    /// slots imbriqués — la sémantique `flatten` d'
    /// `assignedElements({flatten:true})` reste une borne.
    #[must_use]
    pub fn slots_in(&self, root: NodeId) -> Vec<NodeId> {
        let mut out = Vec::new();
        let mut stack: Vec<NodeId> = self.children(root).unwrap_or_default();
        stack.reverse();
        while let Some(n) = stack.pop() {
            if self
                .tag_name(n)
                .is_ok_and(|t| t.eq_ignore_ascii_case("slot"))
            {
                out.push(n);
            }
            if let Ok(cs) = self.children(n) {
                for &c in cs.iter().rev() {
                    stack.push(c);
                }
            }
        }
        out
    }

    /// Nœuds distribués au `<slot>` `slot` (R-97) : enfants de
    /// l'hôte de la racine shadow contenant `slot` dont
    /// l'attribut `slot` égale `slot.name` — le slot **défaut**
    /// (`name` absent ou `""`) collecte les enfants sans
    /// `slot`/`slot=""`, plus les nœuds `Text` (slottables —
    /// `elements_only` les exclut). Vide si `slot` n'est pas
    /// dans une racine attachée ou si rien ne correspond —
    /// le fallback est alors affaire de l'appelant.
    #[must_use]
    pub fn slot_assigned(&self, slot: NodeId, elements_only: bool) -> Vec<NodeId> {
        let Some(host) = self
            .containing_shadow_root(slot)
            .and_then(|r| self.shadow_host(r))
        else {
            return Vec::new();
        };
        let name = self
            .attribute(slot, "name")
            .ok()
            .flatten()
            .unwrap_or_default();
        let mut out = Vec::new();
        if let Ok(children) = self.children(host) {
            for c in children {
                if self.element(c).is_ok() {
                    // `<template shadowrootmode>` consommé : la
                    // spec RETIRE le template de l'arbre à
                    // l'attache (notre table le conserve comme
                    // repère inerte — `template_contents` = la
                    // racine attachée) : il n'est pas un
                    // slottable, il ne couvre pas le repli.
                    let consumed = self
                        .template_contents(c)
                        .ok()
                        .flatten()
                        .is_some_and(|frag| self.is_shadow_root(frag));
                    if consumed {
                        continue;
                    }
                    let want = self.attribute(c, "slot").ok().flatten().unwrap_or_default();
                    if want == name {
                        out.push(c);
                    }
                } else if !elements_only
                    && name.is_empty()
                    && matches!(self.data(c), Ok(NodeData::Text(_)))
                {
                    out.push(c);
                }
            }
        }
        out
    }

    /// Le `<slot>` auquel `el` est distribué (R-97) : le
    /// premier slot de la racine shadow de son parent dont
    /// `name` égale l'attribut `slot` de `el` (défaut `""` —
    /// un `Text` n'a pas d'attribut, il va toujours au slot
    /// défaut). `None` si le parent n'est pas un hôte shadow
    /// ou si aucun slot ne correspond — l'enfant n'est alors
    /// pas rendu (flat tree).
    #[must_use]
    pub fn assigned_slot(&self, el: NodeId) -> Option<NodeId> {
        // `<template shadowrootmode>` consommé : pas un
        // slottable (cf. `slot_assigned`).
        if self
            .template_contents(el)
            .ok()
            .flatten()
            .is_some_and(|frag| self.is_shadow_root(frag))
        {
            return None;
        }
        let host = self.parent(el).ok().flatten()?;
        let root = self.shadow_root(host, true)?;
        let want = self
            .attribute(el, "slot")
            .ok()
            .flatten()
            .unwrap_or_default();
        self.slots_in(root).into_iter().find(|s| {
            self.attribute(*s, "name")
                .ok()
                .flatten()
                .unwrap_or_default()
                == want
        })
    }

    // -----------------------------------------------------------------
    // Normalisation (`Node.normalize()`)
    // -----------------------------------------------------------------

    /// Fusionne les nœuds `Text` adjacents et supprime les `Text`
    /// vides dans tout le sous-arbre de `id`.
    ///
    /// Les nœuds fusionnés ou vidés sont réclamés immédiatement —
    /// leurs handles deviennent `StaleHandle` (invalidation
    /// documentée, ADR-0008). Un `Text` cible d'un root est
    /// conservé pour ne pas casser un handle JS vivant ; il devient
    /// tête de la série fusionnable suivante.
    ///
    /// # Errors
    /// Erreurs de handle comme [`Dom::data`].
    pub fn normalize(&mut self, id: NodeId) -> Result<()> {
        // Post-order : les descendants d'abord.
        let mut cursor = self.first_child(id)?;
        while let Some(child) = cursor {
            self.normalize(child)?;
            cursor = self.next_sibling(child)?;
        }
        let mut prev_text: Option<NodeId> = None;
        let mut cursor = self.first_child(id)?;
        while let Some(child) = cursor {
            let next = self.next_sibling(child)?;
            if let NodeData::Text(t) = self.data(child)? {
                if self.is_root_target(child) {
                    // Vivant via un handle JS : jamais fusionné
                    // ni supprimé — il peut devenir tête de série.
                    prev_text = Some(child);
                } else if t.is_empty() {
                    self.unlink(child)?;
                    self.free(child)?;
                } else if let Some(prev) = prev_text {
                    let merged = t.clone();
                    if let NodeData::Text(pt) = &mut self.node_mut(prev)?.data {
                        pt.push_str(&merged);
                    }
                    self.unlink(child)?;
                    self.free(child)?;
                } else {
                    prev_text = Some(child);
                }
            } else {
                prev_text = None;
            }
            cursor = next;
        }
        Ok(())
    }

    // -----------------------------------------------------------------
    // Rooting et collecte (ADR-0008)
    // -----------------------------------------------------------------

    /// Enregistre `id` comme racine JS — le sous-arbre survit à
    /// [`Dom::collect`] même détaché. Retourne la clé de la table.
    ///
    /// # Errors
    /// Erreurs de handle comme [`Dom::data`].
    pub fn add_root(&mut self, id: NodeId) -> Result<RootId> {
        self.node(id)?;
        let root = RootId(self.next_root);
        self.next_root += 1;
        self.roots.insert(root, id);
        Ok(root)
    }

    /// Lève une racine.
    ///
    /// # Errors
    /// `UnknownRoot` si la clé n'existe pas.
    pub fn remove_root(&mut self, root: RootId) -> Result<()> {
        self.roots
            .remove(&root)
            .map_or_else(|| Err(DomError::UnknownRoot(root)), |_| Ok(()))
    }

    /// Nœud visé par une racine.
    ///
    /// # Errors
    /// `UnknownRoot` si la clé n'existe pas ; `StaleHandle` si le
    /// nœud visé a été collecté (la racine n'a pas empêché la
    /// réclamation — ex. ajoutée après le détachement d'un sous-arbre
    /// déjà inaccessible : le marquage part des racines **vivantes**).
    pub fn root_target(&self, root: RootId) -> Result<NodeId> {
        let id = self.roots.get(&root).ok_or(DomError::UnknownRoot(root))?;
        self.node(*id)?;
        Ok(*id)
    }

    /// `true` si le nœud est cible d'au moins une racine.
    #[must_use]
    pub fn is_root_target(&self, id: NodeId) -> bool {
        self.roots.values().any(|r| *r == id)
    }

    /// Nombre de racines enregistrées.
    #[must_use]
    pub fn root_count(&self) -> usize {
        self.roots.len()
    }

    /// Réclame tout nœud inaccessible depuis le `Document` et les
    /// racines vivantes. Les cases libérées voient leur génération
    /// incrémentée : les `NodeId` qui les désignaient deviennent
    /// `StaleHandle`.
    ///
    /// Retourne le nombre de nœuds réclamés.
    pub fn collect(&mut self) -> usize {
        // Marquage : BFS depuis le document + cibles de racines.
        let mut marked = vec![false; self.slots.len()];
        let mut stack = vec![self.document()];
        stack.extend(self.roots.values().copied());
        while let Some(id) = stack.pop() {
            let idx = slot_idx(id.index());
            if idx >= self.slots.len() || !self.is_alive(id) || marked[idx] {
                continue;
            }
            marked[idx] = true;
            if let Ok(node) = self.node(id) {
                let mut cursor = node.first_child;
                while let Some(child) = cursor {
                    stack.push(child);
                    cursor = self.node(child).ok().and_then(|n| n.next_sibling);
                }
            }
        }
        // Balayage : tout l'occupé non marqué est réclamé.
        let mut freed = 0;
        for (index, is_marked) in marked.iter().enumerate() {
            if *is_marked {
                continue;
            }
            if let Slot::Occupied { generation, .. } = self.slots[index] {
                let id = NodeId::new(index_as_u32(index), generation);
                if self.free(id).is_ok() {
                    freed += 1;
                }
            }
        }
        freed
    }

    // -----------------------------------------------------------------
    // Interne — accès validés et chaînage
    // -----------------------------------------------------------------

    /// Accès validé : index borné + génération concordante.
    fn node(&self, id: NodeId) -> Result<&Node> {
        match self.slots.get(slot_idx(id.index())) {
            None => Err(DomError::OutOfBounds(id)),
            Some(Slot::Vacant { generation, .. }) => Err(DomError::StaleHandle {
                id,
                expected: id.generation(),
                found: *generation,
            }),
            Some(Slot::Occupied { generation, node }) => {
                if *generation == id.generation() {
                    Ok(node)
                } else {
                    Err(DomError::StaleHandle {
                        id,
                        expected: id.generation(),
                        found: *generation,
                    })
                }
            }
        }
    }

    fn node_mut(&mut self, id: NodeId) -> Result<&mut Node> {
        match self.slots.get_mut(slot_idx(id.index())) {
            None => Err(DomError::OutOfBounds(id)),
            Some(Slot::Vacant { generation, .. }) => Err(DomError::StaleHandle {
                id,
                expected: id.generation(),
                found: *generation,
            }),
            Some(Slot::Occupied { generation, node }) => {
                if *generation == id.generation() {
                    Ok(node)
                } else {
                    Err(DomError::StaleHandle {
                        id,
                        expected: id.generation(),
                        found: *generation,
                    })
                }
            }
        }
    }

    /// Allocation via free-list ou croissance — bornée par
    /// `max_nodes` (ADR-0011).
    fn alloc(&mut self, data: NodeData) -> Result<NodeId> {
        let node = Node::new(data);
        if self.live >= self.max_nodes {
            return Err(DomError::BudgetExceeded {
                limit: self.max_nodes,
            });
        }
        if self.free_head != NIL {
            match &self.slots[slot_idx(self.free_head)] {
                Slot::Vacant {
                    generation,
                    next_free,
                } => {
                    let generation = *generation;
                    let index = self.free_head;
                    self.free_head = *next_free;
                    self.slots[slot_idx(index)] = Slot::Occupied { generation, node };
                    self.live += 1;
                    return Ok(NodeId::new(index, generation));
                }
                Slot::Occupied { .. } => {
                    // Invariant interne : free_head ne doit désigner
                    // que des cases libres — on dégrade au lieu de
                    // paniquer (le contenu hostile ne l'atteint pas).
                    debug_assert!(false, "free_head pointe une case occupée");
                    self.free_head = NIL;
                }
            }
        }
        let index = index_as_u32(self.slots.len());
        self.slots.push(Slot::Occupied {
            generation: 1,
            node,
        });
        self.live += 1;
        Ok(NodeId::new(index, 1))
    }

    /// Réclame une case : génération incrémentée (invalidation des
    /// handles), case poussée sur la free-list.
    fn free(&mut self, id: NodeId) -> Result<()> {
        self.node(id)?; // valide le handle
        let index = slot_idx(id.index());
        let generation = match &self.slots[index] {
            Slot::Vacant { generation, .. } | Slot::Occupied { generation, .. } => *generation,
        };
        self.slots[index] = Slot::Vacant {
            generation: generation + 1,
            next_free: self.free_head,
        };
        self.free_head = id.index();
        self.live -= 1;
        Ok(())
    }

    /// Sort `id` de sa chaîne parent/siblings sans réclamer.
    fn unlink(&mut self, id: NodeId) -> Result<()> {
        let (parent, prev, next) = {
            let node = self.node(id)?;
            (node.parent, node.prev_sibling, node.next_sibling)
        };
        if let Some(prev) = prev {
            self.node_mut(prev)?.next_sibling = next;
        }
        if let Some(next) = next {
            self.node_mut(next)?.prev_sibling = prev;
        }
        if let Some(parent) = parent {
            let p = self.node_mut(parent)?;
            if p.first_child == Some(id) {
                p.first_child = next;
            }
            if p.last_child == Some(id) {
                p.last_child = prev;
            }
        }
        let node = self.node_mut(id)?;
        node.parent = None;
        node.prev_sibling = None;
        node.next_sibling = None;
        Ok(())
    }

    /// Insère `child` (déjà délié) sous `parent`, avant `reference`.
    fn link_child(
        &mut self,
        parent: NodeId,
        child: NodeId,
        reference: Option<NodeId>,
    ) -> Result<()> {
        let (prev, next) = match reference {
            Some(reference) => (self.node(reference)?.prev_sibling, Some(reference)),
            None => (self.node(parent)?.last_child, None),
        };
        if let Some(prev) = prev {
            self.node_mut(prev)?.next_sibling = Some(child);
        }
        if let Some(next) = next {
            self.node_mut(next)?.prev_sibling = Some(child);
        }
        {
            let p = self.node_mut(parent)?;
            if let Some(reference) = reference {
                if p.first_child == Some(reference) {
                    p.first_child = Some(child);
                }
            } else {
                p.last_child = Some(child);
            }
            if p.first_child.is_none() {
                p.first_child = Some(child);
            }
        }
        let c = self.node_mut(child)?;
        c.parent = Some(parent);
        c.prev_sibling = prev;
        c.next_sibling = next;
        Ok(())
    }
}

impl Default for Dom {
    fn default() -> Self {
        Self::new()
    }
}

/// `u32` → `usize` : lossless sur nos cibles (≥ 32 bits) ; saturé en
/// dernier recours — l'index résultant déborde l'arène et l'accès
/// échoue typé (`OutOfBounds`), jamais de panic.
fn slot_idx(index: u32) -> usize {
    usize::try_from(index).unwrap_or(usize::MAX)
}

/// `usize` → `u32` saturant : le budget `max_nodes` borne l'arène
/// bien en dessous de `u32::MAX`, la saturation est inatteignable
/// mais reste non-fatale.
fn index_as_u32(index: usize) -> u32 {
    u32::try_from(index).unwrap_or(u32::MAX)
}
