// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Tests de l'arène DOM — étape 9 + protocole ADR-0008
//! (rooting explicite, générations invalidées, collecte).

use explorer_dom::{Dom, DomError, NodeId};

fn el(dom: &mut Dom, tag: &str) -> NodeId {
    dom.create_element(tag)
        .unwrap_or_else(|_| unreachable!("budget largement suffisant"))
}

fn txt(dom: &mut Dom, data: &str) -> NodeId {
    dom.create_text(data)
        .unwrap_or_else(|_| unreachable!("budget largement suffisant"))
}

#[test]
fn arbre_basique_et_navigation() {
    let mut dom = Dom::new();
    let doc = dom.document();
    let html = el(&mut dom, "html");
    let head = el(&mut dom, "head");
    let body = el(&mut dom, "body");
    let p = el(&mut dom, "p");
    let text = txt(&mut dom, "bonjour");

    dom.append_child(doc, html).expect("append html");
    dom.append_child(html, head).expect("append head");
    dom.append_child(html, body).expect("append body");
    dom.append_child(body, p).expect("append p");
    dom.append_child(p, text).expect("append text");

    assert_eq!(dom.children(doc).expect("children doc"), vec![html]);
    assert_eq!(dom.children(html).expect("children html"), vec![head, body]);
    assert_eq!(dom.parent(p), Ok(Some(body)));
    assert_eq!(dom.prev_sibling(body), Ok(Some(head)));
    assert_eq!(dom.next_sibling(head), Ok(Some(body)));
    assert_eq!(dom.first_child(p), Ok(Some(text)));
    assert_eq!(dom.next_sibling(text), Ok(None));
}

#[test]
fn append_reparentere() {
    let mut dom = Dom::new();
    let a = el(&mut dom, "a");
    let b = el(&mut dom, "b");
    let x = el(&mut dom, "x");
    let doc = dom.document();
    dom.append_child(doc, a).expect("a");
    dom.append_child(doc, b).expect("b");
    dom.append_child(a, x).expect("x sous a");
    // appendChild sur un nœud déjà rattaché = déplacement.
    dom.append_child(b, x).expect("x sous b");
    assert_eq!(dom.children(a).expect("children a"), Vec::<NodeId>::new());
    assert_eq!(dom.children(b).expect("children b"), vec![x]);
    assert_eq!(dom.parent(x), Ok(Some(b)));
}

#[test]
fn insert_before_ordre() {
    let mut dom = Dom::new();
    let doc = dom.document();
    let div = el(&mut dom, "div");
    let (a, b, c) = (el(&mut dom, "a"), el(&mut dom, "b"), el(&mut dom, "c"));
    dom.append_child(doc, div).expect("div");
    dom.append_child(div, a).expect("a");
    dom.append_child(div, c).expect("c");
    dom.insert_before(div, b, Some(c)).expect("b avant c");
    assert_eq!(dom.children(div).expect("children"), vec![a, b, c]);
    // Référence qui n'est pas un enfant du parent → erreur typée.
    let z = el(&mut dom, "z");
    assert_eq!(
        dom.insert_before(div, z, Some(doc)),
        Err(DomError::HierarchyRequest(
            "la référence n'est pas un enfant du parent"
        ))
    );
}

#[test]
fn hierarchie_invalide() {
    let mut dom = Dom::new();
    let doc = dom.document();
    let a = el(&mut dom, "a");
    let t = txt(&mut dom, "x");
    dom.append_child(doc, a).expect("a");

    // Cycle : insérer un ancêtre sous son descendant.
    let b = el(&mut dom, "b");
    dom.append_child(a, b).expect("b sous a");
    let b = dom.children(a).expect("children a")[0];
    assert_eq!(
        dom.append_child(b, a),
        Err(DomError::HierarchyRequest("cycle interdit"))
    );
    // Soi-même.
    assert_eq!(
        dom.append_child(a, a),
        Err(DomError::HierarchyRequest("cycle interdit"))
    );
    // Parent sans capacité d'enfant.
    assert!(matches!(
        dom.append_child(t, a),
        Err(DomError::HierarchyRequest(_))
    ));
    // Le document ne peut pas être un enfant.
    assert_eq!(
        dom.append_child(a, doc),
        Err(DomError::HierarchyRequest(
            "le document ne peut pas être un enfant"
        ))
    );
    // Ni retiré.
    assert!(dom.remove(doc).is_err());
}

#[test]
fn detache_reste_valide_puis_collecte_reclame() {
    let mut dom = Dom::new();
    let doc = dom.document();
    let div = el(&mut dom, "div");
    let t = txt(&mut dom, "contenu");
    dom.append_child(doc, div).expect("div");
    dom.append_child(div, t).expect("t");

    // Détacher ne tue pas le handle.
    dom.remove(div).expect("remove");
    assert!(dom.is_alive(div));
    assert_eq!(dom.parent(div), Ok(None));
    assert_eq!(dom.tag_name(div), Ok("div"));

    // La collecte réclame le sous-arbre entier (div + t).
    let freed = dom.collect();
    assert_eq!(freed, 2);
    assert_eq!(dom.live_count(), 1); // le Document seul
    assert!(matches!(dom.data(div), Err(DomError::StaleHandle { .. })));
    assert!(matches!(dom.data(t), Err(DomError::StaleHandle { .. })));
}

#[test]
fn rooting_survit_a_la_collecte() {
    let mut dom = Dom::new();
    let doc = dom.document();
    let div = el(&mut dom, "div");
    let t = txt(&mut dom, "vivant");
    dom.append_child(doc, div).expect("div");
    dom.append_child(div, t).expect("t");

    let root = dom.add_root(div).expect("root div");
    dom.remove(div).expect("remove");

    // Rooté : le sous-arbre survit, enfants compris.
    assert_eq!(dom.collect(), 0);
    assert_eq!(dom.tag_name(div), Ok("div"));
    assert_eq!(dom.text(t), Ok("vivant"));
    assert_eq!(dom.root_target(root), Ok(div));
    assert_eq!(dom.root_count(), 1);

    // La racine levée, la collecte réclame tout.
    dom.remove_root(root).expect("unroot");
    assert_eq!(dom.collect(), 2);
    assert!(matches!(dom.data(div), Err(DomError::StaleHandle { .. })));
    assert!(matches!(
        dom.remove_root(root),
        Err(DomError::UnknownRoot(_))
    ));
}

#[test]
fn case_reutilisee_generation_bump() {
    let mut dom = Dom::new();
    let doc = dom.document();
    let a = el(&mut dom, "a");
    dom.append_child(doc, a).expect("a");
    dom.remove(a).expect("remove");
    dom.collect();
    assert!(matches!(dom.data(a), Err(DomError::StaleHandle { .. })));

    // La prochaine allocation réutilise la case avec génération +1.
    let b = el(&mut dom, "b");
    assert_eq!(b.index(), a.index());
    assert_eq!(b.generation(), a.generation() + 1);
    // L'ancien handle reste périmé malgré la case occupée.
    assert!(matches!(dom.data(a), Err(DomError::StaleHandle { .. })));
}

#[test]
fn faux_handles_echouent_types() {
    let mut dom = Dom::new();
    let doc = dom.document();
    let e = el(&mut dom, "e");
    dom.append_child(doc, e).expect("e");

    // Hors arène.
    let fake = NodeId::from_parts(9999, 1);
    assert!(matches!(dom.data(fake), Err(DomError::OutOfBounds(_))));
    // Génération fausse sur case occupée.
    let stale = NodeId::from_parts(e.index(), e.generation() + 5);
    assert!(matches!(dom.data(stale), Err(DomError::StaleHandle { .. })));
    // Case libre : un nœud collecté devient Vacant, son handle
    // reste StaleHandle même avec la génération qui suivait.
    let tmp = el(&mut dom, "tmp"); // orphelin : déjà détaché
    dom.collect();
    assert!(matches!(dom.data(tmp), Err(DomError::StaleHandle { .. })));
}

#[test]
fn attributs() {
    let mut dom = Dom::new();
    let div = el(&mut dom, "div");
    dom.set_attribute(div, "class", "a").expect("class");
    dom.set_attribute(div, "id", "x").expect("id");
    dom.set_attribute(div, "class", "b").expect("class replace");

    assert_eq!(
        dom.attribute(div, "class").expect("get"),
        Some("b".to_owned())
    );
    // Ordre d'insertion préservé.
    let names: Vec<&str> = dom
        .attributes(div)
        .expect("attrs")
        .iter()
        .map(|a| a.name.as_str())
        .collect();
    assert_eq!(names, vec!["class", "id"]);
    assert!(dom.remove_attribute(div, "class").expect("remove"));
    assert!(!dom.remove_attribute(div, "class").expect("remove absent"));
    assert_eq!(dom.attribute(div, "class").expect("get"), None);

    // Attribut sur non-élément → WrongKind.
    let t = txt(&mut dom, "t");
    assert!(matches!(
        dom.set_attribute(t, "a", "b"),
        Err(DomError::WrongKind { .. })
    ));
}

#[test]
fn texte_newlines_et_normalize() {
    let mut dom = Dom::new();
    let doc = dom.document();
    let div = el(&mut dom, "div");
    dom.append_child(doc, div).expect("div");

    // Le DOM stocke le texte **tel quel** — la normalisation
    // `\r\n`/`\r` → `\n` relève du prétraitement d'entrée du
    // tokenizer HTML (spec : les charrefs `&#xD;` doivent
    // produire un `\r` littéral dans le DOM — R-15).
    let t1 = txt(&mut dom, "a\r\nb\rc");
    assert_eq!(dom.text(t1), Ok("a\r\nb\rc"));
    dom.set_text(t1, "x\r\ny").expect("set");
    assert_eq!(dom.text(t1), Ok("x\r\ny"));

    // Fusion des textes adjacents + suppression des vides.
    let a = txt(&mut dom, "un");
    let empty = txt(&mut dom, "");
    let b = txt(&mut dom, " deux");
    let comment = dom.create_comment("stop").expect("comment");
    let c = txt(&mut dom, " trois");
    for n in [a, empty, b, comment, c] {
        dom.append_child(div, n).expect("child");
    }
    dom.normalize(div).expect("normalize");

    assert_eq!(dom.children(div).expect("children").len(), 3);
    assert_eq!(dom.text(a), Ok("un deux"));
    assert!(matches!(dom.data(empty), Err(DomError::StaleHandle { .. })));
    assert!(matches!(dom.data(b), Err(DomError::StaleHandle { .. })));
    // Le commentaire casse la série : c reste un nœud distinct.
    assert_eq!(dom.text(c), Ok(" trois"));
    assert_eq!(
        dom.text_content(div).expect("text_content"),
        "un deux trois"
    );
}

#[test]
fn normalize_preserve_texte_roote() {
    let mut dom = Dom::new();
    let doc = dom.document();
    let div = el(&mut dom, "div");
    dom.append_child(doc, div).expect("div");
    let a = txt(&mut dom, "un");
    let keep = txt(&mut dom, "deux");
    dom.append_child(div, a).expect("a");
    dom.append_child(div, keep).expect("keep");
    let root = dom.add_root(keep).expect("root");

    dom.normalize(div).expect("normalize");
    // `keep` ne peut pas être libéré (rooté) : la fusion est bloquée,
    // les deux nœuds restent vivants et distincts.
    assert_eq!(dom.text(a), Ok("un"));
    assert_eq!(dom.text(keep), Ok("deux"));
    assert_eq!(dom.children(div).expect("children").len(), 2);
    dom.remove_root(root).expect("unroot");
}

#[test]
fn budget_noeuds() {
    let mut dom = Dom::with_max_nodes(3);
    let _doc = dom.document(); // index 0 déjà consommé
    let _a = dom.create_element("a").expect("a");
    let _b = dom.create_element("b").expect("b");
    assert_eq!(
        dom.create_element("c"),
        Err(DomError::BudgetExceeded { limit: 3 })
    );
}

#[test]
fn text_content_propage_erreur_handle() {
    let dom = Dom::new();
    let fake = NodeId::from_parts(42, 7);
    assert!(matches!(
        dom.text_content(fake),
        Err(DomError::OutOfBounds(_))
    ));
}

#[test]
fn shadow_root_tables() {
    let mut dom = Dom::new();
    let host = dom.create_element("div").expect("host");
    dom.append_child(dom.document(), host).expect("append");
    let root = dom.create_fragment().expect("frag");
    let inner = dom.create_element("span").expect("inner");
    dom.append_child(root, inner).expect("append inner");

    dom.attach_shadow_root(host, root, true);
    assert_eq!(dom.shadow_root(host, false), Some(root));
    assert_eq!(dom.shadow_host(root), Some(host));
    assert!(dom.is_shadow_root(root));
    assert!(!dom.is_shadow_root(host));
    // Le fragment n'est PAS un enfant de l'hôte — arbre latéral.
    assert_eq!(dom.children(host).expect("kids"), Vec::new());
    // La remontée d'ancêtres termine à la racine shadow.
    assert_eq!(dom.containing_shadow_root(inner), Some(root));
    assert_eq!(dom.containing_shadow_root(root), Some(root));
    assert_eq!(dom.containing_shadow_root(host), None);
    // Mode closed : `shadow_root` honnête sans include_closed.
    let host2 = dom.create_element("div").expect("host2");
    let root2 = dom.create_fragment().expect("frag2");
    dom.attach_shadow_root(host2, root2, false);
    assert_eq!(dom.shadow_root(host2, false), None);
    assert_eq!(dom.shadow_root(host2, true), Some(root2));
}

/// R-97 — distribution `<slot>` : assignation nommée,
/// slot défaut (éléments sans `slot` + textes), lookup
/// `assigned_slot`.
#[test]
fn slot_distribution() {
    let mut dom = Dom::new();
    let host = dom.create_element("div").expect("host");
    dom.append_child(dom.document(), host).expect("append");
    // Enfants light : un assigné nommé, un défaut, un texte.
    let named = dom.create_element("b").expect("named");
    dom.set_attribute(named, "slot", "t").expect("slot attr");
    let plain = dom.create_element("i").expect("plain");
    let other = dom.create_element("u").expect("other");
    dom.set_attribute(other, "slot", "z").expect("slot z");
    let text = dom.create_text("hi").expect("text");
    for c in [named, plain, other, text] {
        dom.append_child(host, c).expect("append light");
    }
    // Racine : <slot name="t"> puis <slot> (défaut).
    let root = dom.create_fragment().expect("frag");
    let s_named = dom.create_element("slot").expect("slot t");
    dom.set_attribute(s_named, "name", "t").expect("name t");
    let s_def = dom.create_element("slot").expect("slot def");
    dom.append_child(root, s_named).expect("append slot t");
    dom.append_child(root, s_def).expect("append slot def");
    dom.attach_shadow_root(host, root, true);

    assert_eq!(dom.slots_in(root), vec![s_named, s_def]);
    assert_eq!(dom.slot_assigned(s_named, false), vec![named]);
    // Défaut : éléments sans slot + le texte.
    assert_eq!(dom.slot_assigned(s_def, false), vec![plain, text]);
    assert_eq!(dom.slot_assigned(s_def, true), vec![plain]);
    // `slot="z"` ne correspond à rien → non rendu.
    assert_eq!(dom.assigned_slot(named), Some(s_named));
    assert_eq!(dom.assigned_slot(plain), Some(s_def));
    assert_eq!(dom.assigned_slot(text), Some(s_def));
    assert_eq!(dom.assigned_slot(other), None);
    // Hors hôte : rien.
    let orphan = dom.create_element("span").expect("orphan");
    assert_eq!(dom.assigned_slot(orphan), None);
    assert_eq!(dom.slots_in(host), Vec::<explorer_dom::NodeId>::new());
}
