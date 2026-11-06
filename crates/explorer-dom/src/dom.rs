// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Arène DOM — allocation générationnelle, arbre, rooting ADR-0008.

use std::collections::hash_map::Entry;
use std::collections::HashMap;

use crate::atom::Atom;
use crate::error::{DomError, Result};
use crate::id::{NodeId, RootId};
use crate::node::{Attribute, DoctypeData, ElementData, Namespace, Node, NodeData, TextRange};

/// Sentinelle de free-list (« pas de case libre »).
const NIL: u32 = u32::MAX;

/// Budget de nœuds par défaut — borne anti-contenu hostile
/// (ADR-0011). Configurable via [`Dom::with_max_nodes`].
pub const DEFAULT_MAX_NODES: usize = 1_000_000;

/// Budget de racines explicites par défaut — la table `roots`
/// est bornée comme le reste de l'arène (ADR-0008 : « le nombre
/// de roots est borné »). Configurable via [`Dom::with_limits`] ;
/// valeur à réviser quand le contrat handle-JS sera câblé.
pub const DEFAULT_MAX_ROOTS: usize = 65_536;

/// Case de l'arène : occupée par un nœud ou libre (free-list
/// chaînée). La `generation` survit à la réallocation — c'est elle
/// qui invalide les `NodeId` obsolètes.
#[derive(Debug)]
enum Slot {
    Vacant { generation: u32, next_free: u32 },
    Occupied { generation: u32, node: Node },
}

/// Candidats `by_id` d'une même valeur d'attribut `id` (AR-11).
/// `One` inline couvre le cas dominant du web (identifiants
/// uniques — pas d'allocation par entrée, purge `collect`
/// séquentielle, AR-12) ; `Many` déborde sur les doublons.
/// Invariant : jamais vide — l'entrée est supprimée dès que
/// son dernier candidat part.
#[derive(Debug)]
enum IdSet {
    One(NodeId),
    Many(Vec<NodeId>),
}

impl IdSet {
    fn push(&mut self, id: NodeId) {
        match self {
            IdSet::One(existing) => {
                if *existing != id {
                    *self = IdSet::Many(vec![*existing, id]);
                }
            }
            IdSet::Many(ids) => {
                if !ids.contains(&id) {
                    ids.push(id);
                }
            }
        }
    }

    /// Retire `id` ; retourne `true` si l'ensemble est vidé
    /// (l'entrée doit être supprimée de la table).
    fn remove(&mut self, id: NodeId) -> bool {
        match self {
            IdSet::One(existing) => *existing == id,
            IdSet::Many(ids) => {
                ids.retain(|&n| n != id);
                ids.is_empty()
            }
        }
    }

    /// Ne garde que les handles vivants ; retourne `true` si
    /// l'ensemble est vidé.
    fn retain_dead(&mut self, alive: impl Fn(NodeId) -> bool) -> bool {
        match self {
            IdSet::One(existing) => !alive(*existing),
            IdSet::Many(ids) => {
                ids.retain(|&n| alive(n));
                ids.is_empty()
            }
        }
    }

    /// Vue tranche — `One` se prête `from_ref`, zéro copie.
    fn as_slice(&self) -> &[NodeId] {
        match self {
            IdSet::One(id) => std::slice::from_ref(id),
            IdSet::Many(ids) => ids,
        }
    }
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
    max_roots: usize,
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
    /// Noms de custom elements définis (registre
    /// `customElements`) — rempli par la session depuis le
    /// miroir JS ; lu par `:defined` (selectors-4 §13) à la
    /// cascade. Document-local, comme le registre spec.
    defined_names: std::collections::HashSet<String>,
    /// Index `id → NodeId` (AR-11) : tous les éléments portant
    /// un `id`, doublons compris (document hostile). Maintenu
    /// par [`Dom::set_attribute`]/[`Dom::remove_attribute`],
    /// purgé des handles morts par [`Dom::collect`] — entre
    /// deux passes, [`Dom::by_id`] filtre lui-même les handles
    /// obsolètes et les nœuds hors de l'arbre document
    /// (détachés, shadow, `<template>`).
    id_index: HashMap<String, IdSet>,
    /// Scratch `collect` conservé entre passes (AR-12) :
    /// bitmap de marquage 1 bit/case (vs `vec![bool]` =
    /// 1 o/case) et pile DFS — ~1,3 Kio pour 10 k nœuds,
    /// alloué à la première passe puis réutilisé. `take`n
    /// pendant le marquage pour contourner l'emprunt de `self`.
    mark_bits: Vec<u64>,
    mark_stack: Vec<NodeId>,
    /// Plancher de génération des cases tronquées (AR-12) :
    /// `collect` rend la queue de `slots` au système — les
    /// générations abandonnées sont conservées ici pour que
    /// toute case repoussée reparte strictement au-dessus
    /// (ADR-0008 : un handle émis avant la truncation ne doit
    /// jamais résoudre un nœud neuf).
    tail_generation: u32,
    /// High-water de `slots.len()` (AR-12) : distingue un
    /// handle vers un indice **tronqué** (`< peak_len` →
    /// `StaleHandle`, il a existé) d'un indice forgé
    /// (`≥ peak_len` → `OutOfBounds`, jamais alloué).
    peak_len: usize,
    /// Buffer texte mutualisé (AR-14) : tout le texte du
    /// document (nœuds `Text` **et** `Comment`) vit ici — une
    /// seule allocation au lieu d'une `String` par nœud.
    /// Append-only entre passes : `set_text`/fusion `normalize`
    /// ajoutent une plage, l'ancienne reste morte — [`Dom::collect`]
    /// **compacte** (réécrit les plages vivantes contiguës, rend
    /// les octets morts). Résolution via [`Dom::text_at`].
    text_buffer: String,
}

impl Dom {
    /// Arène neuve (budget [`DEFAULT_MAX_NODES`]) contenant le nœud
    /// `Document` à l'index 0.
    #[must_use]
    pub fn new() -> Self {
        Self::with_limits(DEFAULT_MAX_NODES, DEFAULT_MAX_ROOTS)
    }

    /// Arène neuve avec budget de nœuds explicite (racines à
    /// [`DEFAULT_MAX_ROOTS`]).
    #[must_use]
    pub fn with_max_nodes(max_nodes: usize) -> Self {
        Self::with_limits(max_nodes, DEFAULT_MAX_ROOTS)
    }

    /// Arène neuve avec budgets de nœuds et de racines explicites.
    #[must_use]
    pub fn with_limits(max_nodes: usize, max_roots: usize) -> Self {
        let mut dom = Self {
            slots: Vec::new(),
            free_head: NIL,
            live: 0,
            max_nodes,
            max_roots,
            roots: HashMap::new(),
            next_root: 0,
            document_mode: DocumentMode::NoQuirks,
            shadow_roots: HashMap::new(),
            shadow_hosts: HashMap::new(),
            defined_names: std::collections::HashSet::new(),
            id_index: HashMap::new(),
            mark_bits: Vec::new(),
            mark_stack: Vec::new(),
            tail_generation: 0,
            peak_len: 0,
            text_buffer: String::new(),
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

    /// Octets heap **mesurés** de l'arène — le chiffre que
    /// `mem_report` publie en `dom_bytes` (AR-7). Couvre :
    /// - la capacité du vecteur `slots` — **y compris les
    ///   cases libres** : l'arène ne rend pas la mémoire tant
    ///   que le vecteur n'est pas rétréci (le surbooking d'un
    ///   document passé persiste donc dans ce chiffre, comme
    ///   en RSS réel) ;
    /// - la capacité des tables `roots`, `shadow_roots`,
    ///   `shadow_hosts` et `defined_names` ;
    /// - les allocations internes aux nœuds vivants : textes
    ///   et commentaires (`String`), `tag_name`, vecteurs
    ///   `attributes` et les `String` de chaque attribut.
    ///
    /// La capacité d'un `HashMap`/`HashSet` est prise à sa
    /// valeur `capacity()` (case + octet de contrôle élidés
    /// — approximation de l'ordre du pour-cent, documentée,
    /// jamais un faux total exact).
    #[must_use]
    pub fn heap_bytes(&self) -> usize {
        use std::mem::size_of;
        let mut bytes = self.slots.capacity() * size_of::<Slot>()
            + self.roots.capacity() * size_of::<(RootId, NodeId)>()
            + self
                .shadow_roots
                .capacity()
                .saturating_mul(size_of::<(NodeId, (NodeId, bool))>())
            + self.shadow_hosts.capacity() * size_of::<(NodeId, NodeId)>()
            + self.defined_names.capacity() * size_of::<String>();
        bytes += self
            .defined_names
            .iter()
            .map(String::capacity)
            .sum::<usize>();
        bytes += self.id_index.capacity() * size_of::<(String, IdSet)>()
            + self
                .id_index
                .iter()
                .map(|(k, v)| {
                    k.capacity()
                        + match v {
                            IdSet::Many(ids) => ids.capacity() * size_of::<NodeId>(),
                            IdSet::One(_) => 0,
                        }
                })
                .sum::<usize>()
            + self.mark_bits.capacity() * size_of::<u64>()
            + self.mark_stack.capacity() * size_of::<NodeId>()
            + self.text_buffer.capacity();
        for slot in &self.slots {
            let Slot::Occupied { node, .. } = slot else {
                continue;
            };
            bytes += match &node.data {
                NodeData::Doctype(d) => {
                    size_of::<DoctypeData>()
                        + d.name.capacity()
                        + d.public_id.capacity()
                        + d.system_id.capacity()
                }
                NodeData::Element(el) => {
                    // `tag_name`/`attr.name` sont des `Atom`
                    // internés process-wide (AR-13) — hors
                    // empreinte propre au `Dom` (la table
                    // d'atomes est globale, cf. `atom.rs`).
                    el.attributes.capacity() * size_of::<Attribute>()
                        + el.attributes
                            .iter()
                            .map(|a| a.value.capacity())
                            .sum::<usize>()
                }
                // Plages dans le buffer partagé — compté une
                // fois via `text_buffer` (AR-14).
                NodeData::Document | NodeData::Text(_) | NodeData::Comment(_) => 0,
            };
        }
        bytes
    }

    /// Budget de nœuds configuré.
    #[must_use]
    pub fn max_nodes(&self) -> usize {
        self.max_nodes
    }

    /// Budget de racines configuré (borne de la table `roots`).
    #[must_use]
    pub fn max_roots(&self) -> usize {
        self.max_roots
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
        let range = self.push_text(data)?;
        self.alloc(NodeData::Text(range))
    }

    /// Crée un commentaire détaché.
    ///
    /// # Errors
    /// [`DomError::BudgetExceeded`] si le budget de nœuds est atteint.
    pub fn create_comment(&mut self, data: &str) -> Result<NodeId> {
        let range = self.push_text(data)?;
        self.alloc(NodeData::Comment(range))
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
        self.alloc(NodeData::Doctype(Box::new(DoctypeData {
            name: name.to_owned(),
            public_id: public_id.to_owned(),
            system_id: system_id.to_owned(),
        })))
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
        alive(&self.slots, id)
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
    /// **Contourne les index** : muter `attributes` directement
    /// ne met pas à jour `id_index` (AR-11) — préférer
    /// [`Dom::set_attribute`]/[`Dom::remove_attribute`].
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
            NodeData::Text(t) | NodeData::Comment(t) => Ok(self.text_at(*t)),
            _ => Err(DomError::WrongKind {
                id,
                expected: "Text | Comment",
            }),
        }
    }

    /// Résout une [`TextRange`] en `&str` depuis le buffer
    /// texte mutualisé (AR-14). Plage forgée hors bornes → `""`
    /// (dégradation gracieuse — le buffer ne peut pas être
    /// corrompu, seule une plage issue d'un `element_mut` ou
    /// d'une recopie manuelle peut être invalide).
    #[must_use]
    pub fn text_at(&self, range: TextRange) -> &str {
        self.text_buffer
            .get(range.start as usize..range.start as usize + range.len as usize)
            .unwrap_or("")
    }

    /// Remplace le texte d'un nœud `Text`/`Comment` (contenu tel
    /// quel — voir [`Dom::create_text`]).
    ///
    /// # Errors
    /// Comme [`Dom::text`].
    pub fn set_text(&mut self, id: NodeId, data: &str) -> Result<()> {
        if !matches!(self.data(id)?, NodeData::Text(_) | NodeData::Comment(_)) {
            return Err(DomError::WrongKind {
                id,
                expected: "Text | Comment",
            });
        }
        // Append-only : la nouvelle plage est poussée au buffer,
        // l'ancienne reste morte jusqu'à la compaction `collect`.
        let range = self.push_text(data)?;
        match &mut self.node_mut(id)?.data {
            NodeData::Text(t) | NodeData::Comment(t) => *t = range,
            _ => {}
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
            NodeData::Text(t) | NodeData::Comment(t) => out.push_str(self.text_at(*t)),
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
                NodeData::Text(t) => out.push_str(self.text_at(*t)),
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
        Ok(self.children_iter(id)?.collect())
    }

    /// Itérateur paresseux sur les enfants dans l'ordre du
    /// document — **sans `Vec` alloué** : une validation de
    /// `id` à la création, puis un check borne+génération par
    /// sibling à l'avance (le banc `arena` mesurait
    /// [`Dom::children`] à 4,2 µs/appel sur 1 000 frères —
    /// cible AR-10). L'itérateur emprunte le `Dom` :
    /// l'arène ne peut pas muter pendant la traversée ; un
    /// lien sibling mort termine l'itération (même
    /// robustesse que [`Dom::children`], sans propager
    /// d'erreur par item).
    ///
    /// # Errors
    /// Erreurs de handle comme [`Dom::data`].
    pub fn children_iter(&self, id: NodeId) -> Result<ChildrenIter<'_>> {
        Ok(ChildrenIter {
            dom: self,
            next: self.node(id)?.first_child,
        })
    }

    /// Descendants en **préordre document** (`id` exclu) —
    /// remplace l'idiome `queue`+`children().rev()` des scans
    /// d'arbre : O(1) mémoire, une validation par nœud
    /// visité (AR-10). Un lien mort termine la remontée du
    /// sous-arbre fautif sans fausser le reste du parcours.
    ///
    /// # Errors
    /// Erreurs de handle comme [`Dom::data`].
    pub fn descendants(&self, id: NodeId) -> Result<Descendants<'_>> {
        let first = self.node(id)?.first_child;
        Ok(Descendants {
            dom: self,
            root: id,
            pending: first,
        })
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

    /// Premier élément portant `id` **en ordre document** —
    /// sémantique `getElementById` (premier match préordre dans
    /// l'arbre du document). L'index `id_index` est rempli par
    /// [`Dom::set_attribute`]/[`Dom::remove_attribute`] ; les
    /// doublons d'`id` (web hostile) sont arbitrés ici par
    /// [`Dom::doc_order_lt`], pas à l'insertion — les déplacements
    /// (`insert_before`) ne réordonnent pas l'index.
    ///
    /// Ne retourne jamais un nœud hors de l'arbre document :
    /// détaché ([`Dom::remove`] = unlink, le sous-arbre reste
    /// vivant), racine/contenu shadow, fragment `<template>` —
    /// conforme à la spec (`getElementById` n'entre ni dans les
    /// shadow trees ni dans `<template>`).
    #[must_use]
    pub fn by_id(&self, id: &str) -> Option<NodeId> {
        let candidates = self.id_index.get(id)?.as_slice();
        let doc = self.document();
        let mut best: Option<NodeId> = None;
        for &n in candidates {
            if !self.is_alive(n) {
                continue; // balayé — purge différée à `collect`
            }
            if n != doc && !self.is_ancestor(doc, n).unwrap_or(false) {
                continue; // détaché, shadow ou <template>
            }
            if best.is_none_or(|b| self.doc_order_lt(n, b)) {
                best = Some(n);
            }
        }
        best
    }

    /// `a` précède-t-il `b` en **préordre document** ? Chaînes
    /// d'ancêtres comparées jusqu'à l'ancêtre commun le plus bas,
    /// puis ordre des deux branches par parcours sibling —
    /// O(profondeur + fratrie). Arbres disjoints : ordre stable
    /// arbitraire (cas filtré en amont par `by_id`).
    fn doc_order_lt(&self, a: NodeId, b: NodeId) -> bool {
        if a == b {
            return false;
        }
        if self.is_ancestor(a, b).unwrap_or(false) {
            return true; // l'ancêtre précède en préordre
        }
        if self.is_ancestor(b, a).unwrap_or(false) {
            return false;
        }
        let chain_a = self.ancestor_chain(a);
        let chain_b = self.ancestor_chain(b);
        // Les chaînes finissent à leur racine respective ; la
        // queue commune fait remonter l'ancêtre commun le plus bas.
        let mut i = chain_a.len();
        let mut j = chain_b.len();
        while i > 0 && j > 0 && chain_a[i - 1] == chain_b[j - 1] {
            i -= 1;
            j -= 1;
        }
        if i == chain_a.len() || j == chain_b.len() {
            // Racines différentes : arbres disjoints — ordre
            // stable arbitraire (filtré en amont par `by_id`).
            return a.index() < b.index();
        }
        // chain_a[i - 1] / chain_b[j - 1] : branches distinctes
        // sous l'ancêtre commun — la première par ordre sibling
        // gagne.
        let mut cursor = Some(chain_a[i - 1]);
        while let Some(n) = cursor {
            if n == chain_b[j - 1] {
                return true;
            }
            cursor = self.next_sibling(n).ok().flatten();
        }
        false
    }

    /// Chaîne `id → racine` (id inclus, racine en queue).
    fn ancestor_chain(&self, id: NodeId) -> Vec<NodeId> {
        let mut out = vec![id];
        let mut cursor = id;
        while let Ok(Some(p)) = self.parent(cursor) {
            out.push(p);
            cursor = p;
        }
        out
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
        Ok(self.attribute_str(id, name)?.map(str::to_owned))
    }

    /// Valeur d'attribut par nom exact, **sans allocation** —
    /// préférer à [`Dom::attribute`] dans les chemins chauds :
    /// le banc `arena` (AR-6) mesure le getter clonant ≈ 27×
    /// plus cher (String allouée par lookup, ×124 sites en
    /// `src`). La référence emprunte l'arène — elle expire à
    /// la prochaine mutation du `Dom`, comme [`Dom::attributes`].
    ///
    /// # Errors
    /// Comme [`Dom::element`].
    pub fn attribute_str(&self, id: NodeId, name: &str) -> Result<Option<&str>> {
        Ok(self
            .element(id)?
            .attributes
            .iter()
            .find(|a| a.name == name)
            .map(|a| a.value.as_str()))
    }

    /// Attributs ordonnés d'un élément.
    ///
    /// # Errors
    /// Comme [`Dom::element`].
    pub fn attributes(&self, id: NodeId) -> Result<&[Attribute]> {
        Ok(&self.element(id)?.attributes)
    }

    /// Pose ou remplace un attribut (ordre d'insertion préservé).
    /// `id` alimente `id_index` (AR-11) : l'ancienne valeur
    /// est retirée de l'index, la nouvelle ajoutée.
    ///
    /// # Errors
    /// `WrongKind` si le nœud n'est pas un élément.
    pub fn set_attribute(&mut self, id: NodeId, name: &str, value: &str) -> Result<()> {
        let is_id = name == "id";
        let old_id = is_id
            .then(|| {
                self.element(id).ok().and_then(|el| {
                    el.attributes
                        .iter()
                        .find(|a| a.name == "id")
                        .map(|a| a.value.clone())
                })
            })
            .flatten();
        let el = self.element_mut(id)?;
        match el.attributes.iter_mut().find(|a| a.name == name) {
            Some(attr) => value.clone_into(&mut attr.value),
            None => el.attributes.push(Attribute {
                name: Atom::from(name),
                value: value.to_owned(),
            }),
        }
        if is_id {
            if let Some(old) = old_id {
                self.unindex_id(id, &old);
            }
            self.index_id(id, value);
        }
        Ok(())
    }

    /// Retire un attribut ; `true` s'il existait.
    /// `id` déréférence le nœud d'`id_index` (AR-11).
    ///
    /// # Errors
    /// `WrongKind` si le nœud n'est pas un élément.
    pub fn remove_attribute(&mut self, id: NodeId, name: &str) -> Result<bool> {
        let el = self.element_mut(id)?;
        let old_id = (name == "id").then(|| {
            el.attributes
                .iter()
                .find(|a| a.name == "id")
                .map(|a| a.value.clone())
        });
        let old_id = old_id.flatten();
        let before = el.attributes.len();
        el.attributes.retain(|a| a.name != name);
        let removed = el.attributes.len() != before;
        if removed {
            if let Some(old) = old_id {
                self.unindex_id(id, &old);
            }
        }
        Ok(removed)
    }

    /// Nom de balise d'un élément.
    ///
    /// # Errors
    /// Comme [`Dom::element`].
    pub fn tag_name(&self, id: NodeId) -> Result<&str> {
        Ok(self.element(id)?.tag_name.as_str())
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

    /// Remplace l'ensemble des noms de custom elements définis
    /// (registre `customElements`) — la session le synchronise
    /// depuis le miroir JS à chaque drain ; `:defined` le lit.
    pub fn set_defined_names(&mut self, names: impl IntoIterator<Item = String>) {
        self.defined_names = names.into_iter().collect();
    }

    /// `true` si `name` est un nom de custom element défini —
    /// spec « defined element » : la pseudo-classe `:defined`
    /// matche les éléments dont le nom (ou l'attribut `is`) y
    /// figure, **plus tous les éléments natifs** (un élément
    /// sans tiret dans son nom local est toujours défini).
    /// Le tri par nom est délégué à l'appelant
    /// (`explorer_css::state::is_defined` côté cascade).
    #[must_use]
    pub fn is_defined_name(&self, name: &str) -> bool {
        self.defined_names.contains(name)
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
        for n in self.descendants(root).into_iter().flatten() {
            if self
                .tag_name(n)
                .is_ok_and(|t| t.eq_ignore_ascii_case("slot"))
            {
                out.push(n);
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
            .attribute_str(slot, "name")
            .ok()
            .flatten()
            .unwrap_or_default();
        let mut out = Vec::new();
        if let Ok(children) = self.children_iter(host) {
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
                    let want = self
                        .attribute_str(c, "slot")
                        .ok()
                        .flatten()
                        .unwrap_or_default();
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
            .attribute_str(el, "slot")
            .ok()
            .flatten()
            .unwrap_or_default();
        self.slots_in(root).into_iter().find(|s| {
            self.attribute_str(*s, "name")
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
    /// Itératif — borne de pile constante (la forme récursive
    /// n'avait pas la garde `deep` de `collect_text` : un
    /// DOM hostile profond débordait de pile).
    ///
    /// # Errors
    /// Erreurs de handle comme [`Dom::data`].
    pub fn normalize(&mut self, id: NodeId) -> Result<()> {
        // Pile de travail explicite (forme du miroir jscore) :
        // descendants poussés au fil de la fusion — l'ordre est
        // équivalent au post-order, la fusion ne touchant que
        // les enfants `Text` directs. Seuls les `Element` sont
        // poussés : `Text`/`Comment`/`Doctype` ne peuvent pas
        // avoir d'enfants (`insert_before` borne les parents).
        let mut work = vec![id];
        while let Some(n) = work.pop() {
            let mut prev_text: Option<NodeId> = None;
            let mut cursor = self.first_child(n)?;
            while let Some(child) = cursor {
                let next = self.next_sibling(child)?;
                match self.data(child)? {
                    NodeData::Text(t) => {
                        if self.is_root_target(child) {
                            // Vivant via un handle JS : jamais fusionné
                            // ni supprimé — il peut devenir tête de série.
                            prev_text = Some(child);
                        } else if t.len == 0 {
                            self.unlink(child)?;
                            self.free(child)?;
                        } else if let Some(prev) = prev_text {
                            // Plage append-only : la fusion
                            // produit une nouvelle plage chez
                            // `prev` ; les deux anciennes restent
                            // mortes jusqu'à `collect`.
                            let mut merged = self.text(prev)?.to_owned();
                            merged.push_str(self.text_at(*t));
                            self.set_text(prev, &merged)?;
                            self.unlink(child)?;
                            self.free(child)?;
                        } else {
                            prev_text = Some(child);
                        }
                    }
                    NodeData::Element(_) => {
                        prev_text = None;
                        work.push(child);
                    }
                    _ => prev_text = None,
                }
                cursor = next;
            }
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
    /// Erreurs de handle comme [`Dom::data`] ;
    /// [`DomError::RootBudgetExceeded`] si le budget de racines
    /// (borne ADR-0008, [`Dom::with_limits`]) est atteint.
    pub fn add_root(&mut self, id: NodeId) -> Result<RootId> {
        self.node(id)?;
        if self.roots.len() >= self.max_roots {
            return Err(DomError::RootBudgetExceeded {
                limit: self.max_roots,
            });
        }
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

    /// Réclame tout nœud inaccessible depuis le `Document`, les
    /// racines vivantes et les `seeds`. Les cases libérées voient
    /// leur génération incrémentée : les `NodeId` qui les
    /// désignaient deviennent `StaleHandle`.
    ///
    /// `seeds` = vérité de vivacité recalculée par l'appelant à
    /// chaque passe (ex. tables session retenant des `NodeId`) —
    /// un seed obsolète ou hors arène est ignoré, comme une
    /// racine morte. La passe échoue **typé** (`StaleHandle` au
    /// prochain accès), jamais muet : un seed manqué se détecte
    /// là où un `remove_root` oublié retiendrait à vie.
    ///
    /// Arêtes de marquage (R-139) : la chaîne enfants/siblings
    /// **plus** les arbres latéraux — fragment
    /// [`Dom::template_contents`] et racine shadow attachée (le
    /// même edge-set que le `Remove` du journal session,
    /// `apply.rs`) : une racine shadow ne survit que si son hôte
    /// survit, un fragment que si son `<template>` survit. Au
    /// balayage, `shadow_roots`/`shadow_hosts` sont purgées des
    /// entrées à génération morte — une table ne doit pas
    /// désigner des cases réattribuables.
    ///
    /// Retourne le nombre de nœuds réclamés.
    pub fn collect(&mut self, seeds: &[NodeId]) -> usize {
        // Marquage : DFS depuis le document + racines + seeds —
        // bitmap 1 bit/case (AR-12 ; `vec![bool]` coûtait
        // 1 o/case) sur scratch conservé entre passes.
        // `is_alive` AVANT le bit : un seed obsolète ne doit pas
        // marquer une case réattribuée à un autre nœud.
        let words = self.slots.len().div_ceil(64);
        let mut bits = std::mem::take(&mut self.mark_bits);
        bits.clear();
        bits.resize(words, 0u64);
        let mut stack = std::mem::take(&mut self.mark_stack);
        stack.clear();
        stack.reserve(1 + self.roots.len() + seeds.len());
        stack.push(self.document());
        stack.extend(self.roots.values().copied());
        stack.extend_from_slice(seeds);
        while let Some(id) = stack.pop() {
            let idx = slot_idx(id.index());
            if idx >= self.slots.len() || !self.is_alive(id) {
                continue;
            }
            let bit = 1u64 << (idx & 63);
            if bits[idx >> 6] & bit != 0 {
                continue;
            }
            bits[idx >> 6] |= bit;
            if let Ok(node) = self.node(id) {
                let mut cursor = node.first_child;
                while let Some(child) = cursor {
                    stack.push(child);
                    cursor = self.node(child).ok().and_then(|n| n.next_sibling);
                }
                // Arbre latéral : fragment `content` d'un
                // `<template>` — détaché de la chaîne enfants.
                if let NodeData::Element(el) = &node.data {
                    if let Some(frag) = el.template_contents {
                        stack.push(frag);
                    }
                }
            }
            // Arbre latéral : racine shadow attachée — vit dans
            // `shadow_roots`, hors `children`.
            if let Some(&(root, _)) = self.shadow_roots.get(&id) {
                stack.push(root);
            }
        }
        let freed = self.sweep_dead(&bits);
        // Scratch conservé pour la passe suivante.
        self.mark_bits = bits;
        self.mark_stack = stack;
        freed
    }

    /// Balayage `collect` : réclame tout occupé non marqué dans
    /// `bits`, re-chaîne la free-list au passage, tronque la
    /// queue de cases mortes puis purge les tables latérales
    /// et l'index `id`. Retourne le nombre de nœuds réclamés.
    fn sweep_dead(&mut self, bits: &[u64]) -> usize {
        // `new_len` = dernière case marquée + 1 — au-delà, tout
        // est mort et part au truncate : le balayage s'y borne
        // (le document, index 0, est toujours marqué → ≥ 1).
        let new_len = bits.iter().rposition(|&w| w != 0).map_or(1, |w| {
            w * 64 + usize::try_from(64 - bits[w].leading_zeros()).unwrap_or(64)
        });
        // Balayage : tout l'occupé non marqué sous `new_len` est
        // réclamé — les mots pleins sautent d'un coup (arbre
        // vivant = majorité marquée). La free-list est re-chaînée
        // **dans le même passage** : `next_free` émis à mesure en
        // ordre croissant → les allocs futures se tassent en tête
        // du Vec et la queue reste tronçable. Aucun second
        // parcours des cases n'est nécessaire (AR-12).
        let mut freed = 0;
        let mut new_free_head = NIL;
        let mut prev_vacant: Option<usize> = None;
        for (w, &word) in bits.iter().enumerate().take(new_len.div_ceil(64)) {
            if word == u64::MAX {
                continue;
            }
            let base = w * 64;
            for lane in 0..64 {
                if word >> lane & 1 != 0 {
                    continue;
                }
                let index = base + lane;
                if index >= new_len {
                    break;
                }
                let generation = match &self.slots[index] {
                    Slot::Occupied { generation, .. } => {
                        self.live -= 1;
                        freed += 1;
                        *generation + 1
                    }
                    Slot::Vacant { generation, .. } => *generation,
                };
                self.slots[index] = Slot::Vacant {
                    generation,
                    next_free: NIL,
                };
                if new_free_head == NIL {
                    new_free_head = index_as_u32(index);
                }
                if let Some(p) = prev_vacant {
                    if let Slot::Vacant { next_free, .. } = &mut self.slots[p] {
                        *next_free = index_as_u32(index);
                    }
                }
                prev_vacant = Some(index);
            }
        }
        self.free_head = new_free_head;
        // Queue tronçable (AR-12 — le vrai gain RAM : `slots` ne
        // rétrécissait jamais, watermark permanent post-churn ;
        // `heap_bytes` le mesure). Cases ≥ `new_len` : plancher
        // de génération conservé (toute case repoussée à ces
        // indices repart au-dessus — sinon un handle émis avant
        // la truncation résoudrait un nœud neuf, ADR-0008) ;
        // `live`/`freed` soldés pour les occupés qui meurent
        // sans passer par le chaînage.
        if new_len < self.slots.len() {
            let mut floor = self.tail_generation;
            for slot in &self.slots[new_len..] {
                match slot {
                    Slot::Occupied { generation, .. } => {
                        floor = floor.max(*generation);
                        self.live -= 1;
                        freed += 1;
                    }
                    Slot::Vacant { generation, .. } => {
                        floor = floor.max(*generation);
                    }
                }
            }
            self.tail_generation = floor;
            self.slots.truncate(new_len);
            // Rendre la RAM : `truncate` ne touche pas la
            // capacité — `shrink` amorti (seuil 2×, plancher
            // pour les petites arènes) contre le thrash ré-alloc.
            if self.slots.capacity() > 2 * new_len.max(256) {
                self.slots.shrink_to_fit();
            }
        }
        // Purge des tables latérales : une entrée dont l'hôte ou
        // la racine vient d'être balayée désignerait une case
        // réattribuable — génération vérifiée, pas la présence.
        // (Après le truncate : les cases parties rendent
        // `alive → false` par absence, pas seulement par
        // génération.)
        let slots = &self.slots;
        self.shadow_roots
            .retain(|&host, &mut (root, _)| alive(slots, host) && alive(slots, root));
        self.shadow_hosts
            .retain(|&root, &mut host| alive(slots, root) && alive(slots, host));
        // Index id : purge des handles balayés — un nœud réclamé
        // ne doit plus être candidat `by_id` (entre deux passes,
        // `by_id` filtre déjà par `is_alive`, la purge borne la
        // mémoire et le coût du filtre).
        self.id_index
            .retain(|_, set| !set.retain_dead(|n| alive(slots, n)));
        // Compaction du buffer texte (AR-14) : les plages
        // vivantes sont réécrites contiguës — les octets morts
        // (nœuds balayés, plages remplacées par `set_text` ou
        // fusions `normalize`) sont rendus. Après le truncate :
        // seuls les marqués restent `Occupied` sous `new_len`.
        // `String::new()` (pas `with_capacity`) : la croissance
        // amortie tasse la capacité au volume vivant — la RAM
        // morte est réellement rendue (`heap_bytes` la mesure).
        let mut compacted = String::new();
        for slot in self.slots.iter_mut().take(new_len) {
            let range = match slot {
                Slot::Occupied { node, .. } => match &mut node.data {
                    NodeData::Text(r) | NodeData::Comment(r) => r,
                    _ => continue,
                },
                Slot::Vacant { .. } => continue,
            };
            let start = compacted.len();
            let end = range.start as usize + range.len as usize;
            if let Some(s) = self.text_buffer.get(range.start as usize..end) {
                compacted.push_str(s);
                *range = TextRange {
                    start: u32::try_from(start).unwrap_or(u32::MAX),
                    len: range.len,
                };
            } else {
                *range = TextRange::default();
            }
        }
        self.text_buffer = compacted;
        freed
    }

    // -----------------------------------------------------------------
    // Interne — accès validés et chaînage
    // -----------------------------------------------------------------

    /// Enregistre `id` sous la valeur d'attribut `id` donnée
    /// (index multivalué — les doublons sont arbitrés à la
    /// requête par [`Dom::doc_order_lt`]).
    fn index_id(&mut self, id: NodeId, value: &str) {
        match self.id_index.entry(value.to_owned()) {
            Entry::Occupied(mut e) => e.get_mut().push(id),
            Entry::Vacant(e) => {
                e.insert(IdSet::One(id));
            }
        }
    }

    /// Retire `id` de la liste de la valeur donnée ; l'entrée
    /// disparaît quand sa liste se vide.
    fn unindex_id(&mut self, id: NodeId, value: &str) {
        if let Some(set) = self.id_index.get_mut(value) {
            if set.remove(id) {
                self.id_index.remove(value);
            }
        }
    }

    /// Accès validé : index borné + génération concordante.
    /// Ajoute `s` au buffer texte mutualisé (AR-14) et renvoie
    /// sa plage. Append-only : les plages remplacées restent
    /// mortes jusqu'à la compaction [`Dom::collect`].
    ///
    /// # Errors
    /// [`DomError::BudgetExceeded`] si le buffer dépasserait
    /// `u32::MAX` octets (borne de la plage — document hostile).
    fn push_text(&mut self, s: &str) -> Result<TextRange> {
        let start = self.text_buffer.len();
        if start
            .checked_add(s.len())
            .is_none_or(|end| end > u32::MAX as usize)
        {
            return Err(DomError::BudgetExceeded {
                limit: u32::MAX as usize,
            });
        }
        self.text_buffer.push_str(s);
        Ok(TextRange {
            start: u32::try_from(start).unwrap_or(0),
            len: u32::try_from(s.len()).unwrap_or(u32::MAX),
        })
    }

    /// Un indice sous `peak_len` mais hors `slots` (tronqué par
    /// `collect`, AR-12) rend `StaleHandle` — il a existé ;
    /// au-delà, `OutOfBounds` — forgé.
    fn node(&self, id: NodeId) -> Result<&Node> {
        match self.slots.get(slot_idx(id.index())) {
            None => {
                if slot_idx(id.index()) < self.peak_len {
                    Err(DomError::StaleHandle {
                        id,
                        expected: id.generation(),
                        found: self.tail_generation,
                    })
                } else {
                    Err(DomError::OutOfBounds(id))
                }
            }
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
            None => {
                if slot_idx(id.index()) < self.peak_len {
                    Err(DomError::StaleHandle {
                        id,
                        expected: id.generation(),
                        found: self.tail_generation,
                    })
                } else {
                    Err(DomError::OutOfBounds(id))
                }
            }
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
        // `tail_generation + 1` : une case repoussée après
        // truncation repart au-dessus de toute génération
        // abandonnée (AR-12) ; `tail_generation` vaut 0 tant
        // que rien n'a été tronqué → génération initiale 1.
        let generation = self.tail_generation + 1;
        self.slots.push(Slot::Occupied { generation, node });
        // `max` : `peak_len` ne doit jamais décroître — c'est lui
        // qui distingue « indice tronqué » (StaleHandle) d'
        // « indice jamais alloué » (OutOfBounds).
        self.peak_len = self.peak_len.max(self.slots.len());
        self.live += 1;
        Ok(NodeId::new(index, generation))
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

/// `true` si la case est occupée à la génération du handle —
/// cœur de [`Dom::is_alive`], isolé pour les fermetures qui
/// empruntent déjà un autre champ de `self` (purge des tables
/// latérales au balayage).
fn alive(slots: &[Slot], id: NodeId) -> bool {
    matches!(
        slots.get(slot_idx(id.index())),
        Some(Slot::Occupied { generation, .. }) if *generation == id.generation()
    )
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

/// Voir [`Dom::children_iter`] — curseur sibling emprunté,
/// sans `Vec` alloué.
pub struct ChildrenIter<'a> {
    dom: &'a Dom,
    next: Option<NodeId>,
}

impl Iterator for ChildrenIter<'_> {
    type Item = NodeId;

    fn next(&mut self) -> Option<NodeId> {
        let cur = self.next?;
        // Un sibling mort (case réaffectée entre deux appels
        // — impossible tant que l'emprunt vit, mais le type
        // ne l'interdit pas si le `Dom` est recréé entre
        // appels) termine l'itération plutôt que suivre un
        // handle obsolète.
        self.next = self.dom.node(cur).ok().and_then(|n| n.next_sibling);
        Some(cur)
    }
}

/// Voir [`Dom::descendants`] — marche préordre O(1) mémoire.
/// `root` borne la remontée : sans elle, le dernier
/// descendant ferait fuir le parcours vers les siblings
/// de la racine.
pub struct Descendants<'a> {
    dom: &'a Dom,
    root: NodeId,
    pending: Option<NodeId>,
}

impl Descendants<'_> {
    /// Premier nœud pas encore visité en remontant : premier
    /// `next_sibling` rencontré, borné à `root` (ses siblings
    /// restent hors du sous-arbre).
    fn next_unvisited(&self, mut n: NodeId) -> Option<NodeId> {
        while n != self.root {
            let node = self.dom.node(n).ok()?;
            if let Some(s) = node.next_sibling {
                return Some(s);
            }
            n = node.parent?;
        }
        None
    }
}

impl Iterator for Descendants<'_> {
    type Item = NodeId;

    fn next(&mut self) -> Option<NodeId> {
        let cur = self.pending?;
        // Préordre : descendre si enfant, sinon premier
        // sibling d'un ancêtre borné par la racine.
        self.pending = self
            .dom
            .node(cur)
            .ok()
            .and_then(|n| n.first_child)
            .or_else(|| self.next_unvisited(cur));
        Some(cur)
    }
}
