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
    let freed = dom.collect(&[]);
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
    assert_eq!(dom.collect(&[]), 0);
    assert_eq!(dom.tag_name(div), Ok("div"));
    assert_eq!(dom.text(t), Ok("vivant"));
    assert_eq!(dom.root_target(root), Ok(div));
    assert_eq!(dom.root_count(), 1);

    // La racine levée, la collecte réclame tout.
    dom.remove_root(root).expect("unroot");
    assert_eq!(dom.collect(&[]), 2);
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
    dom.collect(&[]);
    assert!(matches!(dom.data(a), Err(DomError::StaleHandle { .. })));

    // La prochaine allocation réutilise la case avec une
    // génération strictement supérieure — AR-12 : la case
    // tronquée repart au plancher `tail_generation + 1`, pas
    // forcément à +1 exact.
    let b = el(&mut dom, "b");
    assert_eq!(b.index(), a.index());
    assert!(b.generation() > a.generation());
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
    dom.collect(&[]);
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
fn normalize_profond_iteratif() {
    // R-140 : la forme récursive descendait une frame par niveau
    // sans garde `deep` — un DOM hostile profond débordait de pile.
    // La forme itérative tient la profondeur à pile constante.
    let mut dom = Dom::new();
    let doc = dom.document();
    let depth = 10_000;
    let mut parent = doc;
    for _ in 0..depth {
        let d = el(&mut dom, "div");
        dom.append_child(parent, d).expect("chaine");
        parent = d;
    }
    let a = txt(&mut dom, "x");
    let b = txt(&mut dom, "y");
    dom.append_child(parent, a).expect("a");
    dom.append_child(parent, b).expect("b");

    dom.normalize(doc).expect("normalize profond");
    assert_eq!(dom.text(a), Ok("xy"));
    assert!(matches!(dom.data(b), Err(DomError::StaleHandle { .. })));
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
fn budget_racines() {
    // R-141 : la table `roots` est bornée (promesse ADR-0008)
    // — un contenu hostile ne peut pas la grossir sans fin.
    let mut dom = Dom::with_limits(16, 2);
    let a = el(&mut dom, "a");
    let b = el(&mut dom, "b");
    let c = el(&mut dom, "c");
    let r1 = dom.add_root(a).expect("root a");
    dom.add_root(b).expect("root b");
    assert_eq!(
        dom.add_root(c),
        Err(DomError::RootBudgetExceeded { limit: 2 })
    );
    // La borne protège la table — un handle fautif reste une
    // erreur de handle, pas de budget.
    assert!(matches!(
        dom.add_root(NodeId::from_parts(42, 7)),
        Err(DomError::OutOfBounds(_) | DomError::StaleHandle { .. })
    ));
    // Une place libérée redevient utilisable.
    dom.remove_root(r1).expect("unroot");
    assert!(dom.add_root(c).is_ok());
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

/// R-139 — `collect` suit les arêtes latérales : le fragment
/// `template_contents` et la racine shadow attachée survivent
/// tant que leur hôte survit ; hôte balayé, arbre balayé et
/// tables purgées.
#[test]
fn collect_preserve_arbres_lateraux() {
    let mut dom = Dom::new();
    let doc = dom.document();
    let host = el(&mut dom, "div");
    dom.append_child(doc, host).expect("host");
    let tpl = el(&mut dom, "template");
    dom.append_child(doc, tpl).expect("tpl");
    let frag = dom.create_fragment().expect("frag");
    dom.set_template_contents(tpl, frag).expect("tpl frag");
    let t = txt(&mut dom, "contenu");
    dom.append_child(frag, t).expect("t sous frag");
    let root = dom.create_fragment().expect("root");
    dom.attach_shadow_root(host, root, true);
    let inner = el(&mut dom, "span");
    dom.append_child(root, inner).expect("inner sous root");

    // Rien n'est réclamé : les deux arbres latéraux sont
    // atteints via les arêtes de leurs hôtes vivants.
    assert_eq!(dom.collect(&[]), 0);
    for id in [tpl, frag, t, root, inner] {
        assert!(dom.is_alive(id), "vivant {id:?}");
    }

    // Hôte retiré → racine shadow balayée avec lui, table purgée.
    dom.remove(host).expect("remove host");
    assert_eq!(dom.collect(&[]), 3); // host + racine shadow + span
    for id in [host, root, inner] {
        assert!(!dom.is_alive(id), "balayé {id:?}");
        assert!(matches!(dom.data(id), Err(DomError::StaleHandle { .. })));
    }
    assert_eq!(dom.shadow_root(host, true), None);
    assert!(!dom.is_shadow_root(root));
    // Le fragment du template survit : son élément reste rattaché.
    assert!(dom.is_alive(frag) && dom.is_alive(t));
}

/// R-139 — le fragment `content` d'un `<template>` détaché est
/// réclamé avec son élément (l'arête part d'un hôte marqué).
#[test]
fn collect_reclame_fragment_template_detache() {
    let mut dom = Dom::new();
    let doc = dom.document();
    let tpl = el(&mut dom, "template");
    dom.append_child(doc, tpl).expect("tpl");
    let frag = dom.create_fragment().expect("frag");
    dom.set_template_contents(tpl, frag).expect("tpl frag");
    let t = txt(&mut dom, "x");
    dom.append_child(frag, t).expect("t");

    dom.remove(tpl).expect("remove tpl");
    assert_eq!(dom.collect(&[]), 3); // tpl + fragment + texte
    assert!(matches!(dom.data(frag), Err(DomError::StaleHandle { .. })));
}

/// R-139 — une racine shadow survit au détachement tant que
/// son hôte est rooté : l'arête part du nœud marqué, pas de la
/// liste des tables (sinon l'arbre fuirait après purge).
#[test]
fn racine_shadow_survit_si_hote_roote() {
    let mut dom = Dom::new();
    let doc = dom.document();
    let host = el(&mut dom, "div");
    dom.append_child(doc, host).expect("host");
    let root = dom.create_fragment().expect("root");
    dom.attach_shadow_root(host, root, true);
    let inner = el(&mut dom, "b");
    dom.append_child(root, inner).expect("inner");

    let keep = dom.add_root(host).expect("root host");
    dom.remove(host).expect("remove host");
    assert_eq!(dom.collect(&[]), 0); // hôte rooté → ombre vivante
    assert!(dom.is_alive(inner));
    dom.remove_root(keep).expect("unroot");
    assert_eq!(dom.collect(&[]), 3); // host + racine + inner
    assert!(!dom.is_shadow_root(root));
}

/// R-142 — `seeds` : vérité de vivacité recalculée à chaque
/// passe — un sous-arbre détaché survit tant qu'un seed le
/// désigne ; les seeds ne persistent pas et un seed obsolète
/// est ignoré comme une racine morte.
#[test]
fn collect_seeds_retient_arbre_detache() {
    let mut dom = Dom::new();
    let doc = dom.document();
    let div = el(&mut dom, "div");
    dom.append_child(doc, div).expect("div");
    let t = txt(&mut dom, "vivant");
    dom.append_child(div, t).expect("t");
    dom.remove(div).expect("detach");

    // Seed vivant → le sous-arbre détaché survit à la passe.
    assert_eq!(dom.collect(&[div]), 0);
    assert!(dom.is_alive(div) && dom.is_alive(t));

    // Seed hors arène : ignoré — et les seeds ne persistent
    // pas : la passe sans seed sur `div` réclame l'arbre.
    let ghost = NodeId::from_parts(9_999, 1);
    assert_eq!(dom.collect(&[ghost]), 2); // div + t
    assert!(matches!(dom.data(div), Err(DomError::StaleHandle { .. })));

    // Seed profond : retient sa descendance, pas ses ancêtres.
    let host = el(&mut dom, "span");
    dom.append_child(doc, host).expect("host");
    let inner = el(&mut dom, "b");
    dom.append_child(host, inner).expect("inner");
    let leaf = txt(&mut dom, "feuille");
    dom.append_child(inner, leaf).expect("leaf");
    dom.remove(host).expect("detach host");
    assert_eq!(dom.collect(&[inner]), 1); // host seul balayé
    assert!(!dom.is_alive(host));
    assert!(dom.is_alive(inner) && dom.is_alive(leaf));
}

#[test]
fn heap_bytes_mesure_capacite_et_contenu() {
    let mut dom = Dom::new();
    let base = dom.heap_bytes();
    assert!(base > 0); // document + capacité minimale des tables

    // Allocs internes : tag_name + attributs + texte long.
    let doc = dom.document();
    let div = el(&mut dom, "div");
    dom.append_child(doc, div).expect("div");
    dom.set_attribute(div, "data-blob", &"x".repeat(4_096))
        .expect("attr");
    let big = txt(&mut dom, &"y".repeat(8_192));
    dom.append_child(div, big).expect("big");
    let grown = dom.heap_bytes();
    assert!(grown >= base + 12_288, "grown={grown} base={base}");

    // Réclamation : les String des nœuds balayés sont rendues —
    // le chiffre redescend (la capacité `slots`, elle, reste).
    dom.remove(div).expect("detach");
    assert_eq!(dom.collect(&[]), 2);
    let after = dom.heap_bytes();
    assert!(after < grown, "after={after} grown={grown}");
    assert!(after > 0);
}

#[test]
fn iterateurs_children_descendants() {
    let mut dom = Dom::new();
    let doc = dom.document();
    // doc >< premier >< milieu, fin >< texte ; autre
    let premier = el(&mut dom, "a");
    let autre = el(&mut dom, "e");
    dom.append_child(doc, premier).expect("premier");
    dom.append_child(doc, autre).expect("autre");
    let milieu = el(&mut dom, "b");
    let fin = el(&mut dom, "c");
    dom.append_child(premier, milieu).expect("milieu");
    dom.append_child(premier, fin).expect("fin");
    let texte = txt(&mut dom, "d");
    dom.append_child(fin, texte).expect("texte");

    // children_iter : ordre document, zéro Vec intermédiaire.
    let kids: Vec<_> = dom.children_iter(doc).expect("kids").collect();
    assert_eq!(kids, vec![premier, autre]);

    // descendants : préordre document.
    let order: Vec<_> = dom.descendants(doc).expect("desc").collect();
    assert_eq!(order, vec![premier, milieu, fin, texte, autre]);

    // Borné à la racine passée : pas de fuite vers `autre`.
    let sub: Vec<_> = dom.descendants(premier).expect("sub").collect();
    assert_eq!(sub, vec![milieu, fin, texte]);

    // Racine feuille / hors arène.
    assert_eq!(dom.descendants(autre).expect("autre").count(), 0);
    let ghost = NodeId::from_parts(9_999, 1);
    assert!(dom.descendants(ghost).is_err());
    assert!(dom.children_iter(ghost).is_err());
}

#[test]
fn by_id_premier_en_ordre_document() {
    let mut dom = Dom::new();
    let doc = dom.document();
    let premier = el(&mut dom, "div");
    let milieu = el(&mut dom, "span");
    let doublon = el(&mut dom, "p");
    dom.append_child(doc, premier).expect("premier");
    dom.append_child(premier, milieu).expect("milieu");
    dom.append_child(doc, doublon).expect("doublon");
    dom.set_attribute(premier, "id", "cible").expect("id");
    dom.set_attribute(doublon, "id", "cible").expect("id2");
    dom.set_attribute(milieu, "id", "autre").expect("id3");

    // Doublons : le premier en ordre document gagne.
    assert_eq!(dom.by_id("cible"), Some(premier));
    assert_eq!(dom.by_id("autre"), Some(milieu));
    assert_eq!(dom.by_id("absent"), None);

    // Renommage : l'ancienne valeur ne pointe plus le nœud.
    dom.set_attribute(milieu, "id", "cible").expect("rename");
    assert_eq!(dom.by_id("autre"), None);
    assert_eq!(dom.by_id("cible"), Some(premier));

    // Retrait d'attribut.
    dom.remove_attribute(premier, "id").expect("remove");
    assert_eq!(dom.by_id("cible"), Some(milieu));

    // Détaché (unlink seul) : plus dans l'arbre document.
    dom.remove(milieu).expect("detach");
    assert_eq!(dom.by_id("cible"), Some(doublon));

    // collect purge les handles morts — le détaché non seedé
    // est réclamé ; `doublon` reste.
    dom.remove(doublon).expect("detach2");
    dom.collect(&[]);
    assert_eq!(dom.by_id("cible"), None);
}

#[test]
fn collect_tronque_rend_la_ram() {
    let mut dom = Dom::new();
    let doc = dom.document();
    // Gardien en tête (bas indice), churn en queue : la haute
    // eau de l'arène meurt entièrement → truncate + shrink.
    let keep = el(&mut dom, "div");
    dom.append_child(doc, keep).expect("keep");
    let mut handles = Vec::new();
    for _ in 0..600 {
        let n = el(&mut dom, "div");
        dom.append_child(doc, n).expect("a");
        handles.push(n);
    }
    for &n in &handles {
        dom.remove(n).expect("rm");
    }
    let avant = dom.heap_bytes();
    let freed = dom.collect(&[]);
    assert_eq!(freed, 600);
    assert!(
        dom.heap_bytes() < avant / 4,
        "la capacité slots doit être rendue (truncate + shrink)"
    );
    // Free-list re-chaînée : les allocations restent saines.
    let n2 = el(&mut dom, "span");
    dom.append_child(doc, n2).expect("post");
    assert!(dom.is_alive(n2));
    // Le survivant garde un handle valide.
    assert_eq!(dom.tag_name(keep), Ok("div"));
    // Les handles réclamés échouent typés (StaleHandle —
    // indices sous peak_len, jamais OutOfBounds).
    assert!(matches!(
        dom.data(handles[0]),
        Err(DomError::StaleHandle { .. })
    ));
}

#[test]
fn collect_compacte_le_buffer_texte() {
    let mut dom = Dom::new();
    let doc = dom.document();
    // Plages mortes par `set_text` (append-only) sur le gardien.
    let keep = dom.create_text("v0").expect("t");
    dom.append_child(doc, keep).expect("a");
    for i in 1..50 {
        dom.set_text(keep, &format!("valeur-de-remplacement-{i}"))
            .expect("s");
    }
    // Plages mortes par balayage (churn en fin de buffer) —
    // textes longs pour que le buffer domine le signal
    // `heap_bytes` (les petites arènes sont sous le plancher
    // de shrink des slots).
    let corps = "x".repeat(64);
    let mut churn = Vec::new();
    for i in 0..600 {
        let t = dom.create_text(&format!("churn-{i}-{corps}")).expect("c");
        dom.append_child(doc, t).expect("a");
        churn.push(t);
    }
    for &t in &churn {
        dom.remove(t).expect("rm");
    }
    let avant = dom.heap_bytes();
    dom.collect(&[]);
    assert!(
        dom.heap_bytes() < avant / 4,
        "la compaction doit rendre les octets morts du buffer"
    );
    // Le texte survivant reste correct après réécriture de plage.
    assert_eq!(dom.text(keep), Ok("valeur-de-remplacement-49"));
    // Plage forgée hors bornes → "" (dégradation, jamais panic).
    assert_eq!(
        dom.text_at(explorer_dom::TextRange {
            start: u32::MAX - 3,
            len: 8,
        }),
        ""
    );
}

#[test]
fn collect_tronque_puis_realloc_garde_stale() {
    let mut dom = Dom::new();
    let doc = dom.document();
    // Même topologie : gardien bas, churn haut → truncate dur.
    let keep = el(&mut dom, "div");
    dom.append_child(doc, keep).expect("keep");
    let mut churn = Vec::new();
    for _ in 0..300 {
        let n = el(&mut dom, "div");
        dom.append_child(doc, n).expect("c");
        churn.push(n);
    }
    for &n in &churn {
        dom.remove(n).expect("rm");
    }
    dom.collect(&[]);
    // Réallocations au-delà de l'eau tronquée : les cases
    // repoussées partent au plancher de génération — un vieux
    // handle ne ressuscite jamais (ADR-0008), y compris sur un
    // indice réutilisé.
    for _ in 0..50 {
        let n = el(&mut dom, "span");
        dom.append_child(doc, n).expect("post");
    }
    // Indice 2 (churn[0]) a été réattribué, indice ~152 est
    // hors Vec mais sous peak_len : les deux rendent
    // StaleHandle, jamais OutOfBounds ni nœud vivant.
    assert!(matches!(
        dom.data(churn[0]),
        Err(DomError::StaleHandle { .. })
    ));
    assert!(matches!(
        dom.data(churn[150]),
        Err(DomError::StaleHandle { .. })
    ));
    assert!(dom.is_alive(keep));
}

#[test]
fn by_id_ordre_apres_deplacement() {
    let mut dom = Dom::new();
    let doc = dom.document();
    let avant = el(&mut dom, "div");
    let apres = el(&mut dom, "div");
    dom.append_child(doc, avant).expect("avant");
    dom.append_child(doc, apres).expect("apres");
    dom.set_attribute(avant, "id", "dup").expect("id");
    dom.set_attribute(apres, "id", "dup").expect("id2");
    assert_eq!(dom.by_id("dup"), Some(avant));

    // Déplacer `apres` avant `avant` inverse l'ordre document
    // — l'index (insertion) ne l'ordonne pas, la requête si.
    dom.insert_before(doc, apres, Some(avant)).expect("move");
    assert_eq!(dom.by_id("dup"), Some(apres));
}
