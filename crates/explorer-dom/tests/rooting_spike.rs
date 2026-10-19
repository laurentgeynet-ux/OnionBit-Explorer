// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Spike **ADR-0008** : le protocole rooting/invalidation DOM↔JS
//! validé par un faux heap JS, avant le vrai moteur (étape 34).
//!
//! Le modèle : les objets JS référençant un nœud gardent un
//! `NodeId` + un `RootId` (rooting explicite) ; toute mutation qui
//! détache/réalloue un slot invalide la génération — un handle
//! périmé produit `DomError::StaleHandle`, jamais d'accès
//! silencieux sur un nœud réaffecté (UB logique interdite).

use std::collections::HashMap;

use explorer_dom::{Dom, DomError, NodeId, RootId};

/// Faux heap JS : table handle → `(NodeId, RootId)` mimant les
/// wrappers DOM d'un moteur (le `RootId` protège de `collect`,
/// le `NodeId` transporte la génération).
struct FakeJsHeap {
    handles: HashMap<u32, (NodeId, RootId)>,
    next: u32,
}

impl FakeJsHeap {
    fn new() -> Self {
        Self {
            handles: HashMap::new(),
            next: 0,
        }
    }

    /// Crée un wrapper JS sur `id` — le nœud devient rooté.
    fn wrap(&mut self, dom: &mut Dom, id: NodeId) -> u32 {
        let root = dom.add_root(id).expect("root");
        self.next += 1;
        self.handles.insert(self.next, (id, root));
        self.next
    }

    /// Accès via le wrapper — `StaleHandle` si le nœud a été
    /// détaché/réalloué entre-temps (invariant ADR-0008).
    fn get<'d>(&self, dom: &'d Dom, handle: u32) -> explorer_dom::Result<&'d str> {
        let (id, _root) = self.handles[&handle];
        dom.tag_name(id)
    }

    /// Collecte JS côté faux heap : dé-roote les handles morts.
    fn release(&mut self, dom: &mut Dom, handle: u32) {
        if let Some((_id, root)) = self.handles.remove(&handle) {
            dom.remove_root(root).expect("remove_root");
        }
    }
}

#[test]
fn rooted_node_survives_collect() {
    let mut dom = Dom::new();
    let doc = dom.document();
    let el = dom.create_element("div").unwrap();
    dom.append_child(doc, el).unwrap();

    let mut heap = FakeJsHeap::new();
    let h = heap.wrap(&mut dom, el);

    // Sans root, `collect` réclame le sous-arbre détaché ; avec
    // root, le nœud est protégé.
    assert_eq!(dom.collect(), 0);
    assert_eq!(heap.get(&dom, h).unwrap(), "div");
}

#[test]
fn unrooted_subtree_is_reclaimed() {
    let mut dom = Dom::new();
    let doc = dom.document();
    let parent = dom.create_element("section").unwrap();
    let child = dom.create_element("p").unwrap();
    dom.append_child(doc, parent).unwrap();
    dom.append_child(parent, child).unwrap();

    // Détache `parent` du document → ni lui ni `child` ne sont
    // rootés : `collect` les réclame tous les deux.
    dom.remove(parent).unwrap();
    let reclaimed = dom.collect();
    assert!(reclaimed >= 2, "{reclaimed} réclamés");
    assert!(!dom.is_alive(parent));
    assert!(!dom.is_alive(child));
}

#[test]
fn stale_handle_after_collect_and_reuse() {
    let mut dom = Dom::new();
    let doc = dom.document();
    let el = dom.create_element("div").unwrap();
    dom.append_child(doc, el).unwrap();

    // Détache puis réclame le slot, puis crée un nouveau nœud qui
    // réalloue le slot avec une génération incrémentée.
    dom.remove(el).unwrap();
    dom.collect();
    let fresh = dom.create_element("span").unwrap();
    dom.append_child(doc, fresh).unwrap();

    // L'ancien handle pointe un slot réaffecté (ou vacant) →
    // StaleHandle typé, jamais d'accès silencieux.
    let err = dom.tag_name(el).unwrap_err();
    assert!(matches!(err, DomError::StaleHandle { .. }), "{err}");
    // Le handle neuf voit le nouveau nœud.
    assert_eq!(dom.tag_name(fresh).unwrap(), "span");
}

#[test]
fn detach_then_collect_invalidates_handle() {
    let mut dom = Dom::new();
    let doc = dom.document();
    let p = dom.create_element("p").unwrap();
    let b = dom.create_element("b").unwrap();
    dom.append_child(doc, p).unwrap();
    dom.append_child(p, b).unwrap();

    let mut heap = FakeJsHeap::new();
    let hb = heap.wrap(&mut dom, b);

    // `remove` détache sans détruire : le handle rooté reste
    // valide — c'est le contrat (le navigateur garde des
    // références JS sur des nœuds détachés).
    dom.remove(b).unwrap();
    assert_eq!(heap.get(&dom, hb).unwrap(), "b");

    // Une fois dé-rooté + collecté, le handle devient périmé.
    heap.release(&mut dom, hb);
    dom.collect();
    let err = dom.tag_name(b).unwrap_err();
    assert!(matches!(err, DomError::StaleHandle { .. }));
}

#[test]
fn mutation_keeps_live_handle_valid() {
    let mut dom = Dom::new();
    let doc = dom.document();
    let el = dom.create_element("div").unwrap();
    dom.append_child(doc, el).unwrap();

    let mut heap = FakeJsHeap::new();
    let h = heap.wrap(&mut dom, el);

    // Mutations sans détachement : le handle reste valide.
    dom.set_attribute(el, "class", "x").unwrap();
    let child = dom.create_element("em").unwrap();
    dom.append_child(el, child).unwrap();
    assert_eq!(heap.get(&dom, h).unwrap(), "div");
    assert_eq!(dom.attribute(el, "class").unwrap().as_deref(), Some("x"));
}
