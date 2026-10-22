// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! JS-C39 — synchro DOM instrumentée + test croisé miroir ↔
//! `explorer-dom` : le HTML sérialisé du miroir JS est re-parsé
//! par le vrai tree-builder et le DOM commis doit porter les
//! mutations du script (fidélité miroir ↔ réalité).

use explorer_dom::NodeId;
use explorer_js::{JsEngine, JscoreEngine};

/// Collecte `(tag, texte)` des éléments/textes du DOM commis —
/// comparaison structurelle miroir ↔ `explorer-dom`.
fn collect(dom: &explorer_dom::Dom, id: NodeId, out: &mut Vec<(String, String)>) {
    if let Ok(el) = dom.element(id) {
        out.push((el.tag_name.clone(), String::new()));
    }
    if let Ok(t) = dom.text(id) {
        out.push(("#text".to_string(), t.to_string()));
    }
    if let Ok(kids) = dom.children(id) {
        for c in kids {
            collect(dom, c, out);
        }
    }
}

fn committed_dom(html: &str) -> explorer_dom::Dom {
    explorer_html::TreeBuilder::parse(html)
}

#[test]
fn dom_sync_stats_are_counted() {
    let mut eng = JscoreEngine::new();
    eng.install_dom("<html><body><p>hi</p></body></html>", "https://x/", 1);
    // Avant mutation : dirty bit bas → pas de synchro produite.
    assert!(eng.sync_dom_path().is_none());
    eng.eval("var d = document.createElement('div'); d.id = 'inj'; d.textContent = 'ok'; document.body.appendChild(d);")
        .unwrap();
    let payload = eng.sync_dom_path().expect("mutation → synchro");
    assert!(payload.0.contains("inj"), "le miroir porte la mutation");
    let st = eng.dom_sync_stats().expect("compteurs JS-C39");
    assert!(st.syncs >= 1, "au moins une synchro comptée : {st:?}");
    assert!(st.bytes >= payload.0.len() as u64);
}

/// Le point de cohérence exigé par JS-C39 : le HTML sérialisé du
/// miroir se re-parse dans `explorer-dom` **avec les mutations** —
/// la structure commise reflète exactement ce que le script a fait.
#[test]
fn mirror_serializes_into_explorer_dom_faithfully() {
    let mut eng = JscoreEngine::new();
    eng.install_dom(
        "<html><body><ul id='l'><li>a</li></ul></body></html>",
        "https://x/",
        1,
    );
    eng.eval(
        "var l = document.getElementById('l');\
         var li = document.createElement('li');\
         li.textContent = 'b';\
         l.appendChild(li);",
    )
    .unwrap();
    let (html, _path) = eng.sync_dom_path().expect("mutation → synchro");
    let dom = committed_dom(&html);
    let root = dom.document();
    let mut items = Vec::new();
    collect(&dom, root, &mut items);
    let texts: Vec<&str> = items
        .iter()
        .filter(|(t, _)| t == "#text")
        .map(|(_, s)| s.trim())
        .filter(|s| !s.is_empty())
        .collect();
    assert!(texts.contains(&"a") && texts.contains(&"b"), "{items:?}");
    let li_count = items.iter().filter(|(t, _)| t == "li").count();
    assert_eq!(li_count, 2, "deux <li> commis : {items:?}");
}

/// `NullEngine` (et tout moteur sans miroir) : `dom_sync_stats`
/// additif renvoie `None` — aucune casse du contrat.
#[test]
fn null_engine_reports_no_sync_stats() {
    let mut eng = explorer_js::NullEngine;
    assert!(eng.dom_sync_stats().is_none());
}

// ---------------------------------------------------------------------
// ADR-0025 / étape 111 — journal de mutations (synchro différentielle)
// ---------------------------------------------------------------------

/// Le journal est **inconditionnel** — aucune installation de
/// `MutationObserver` requise : `drain_dom_mutations` produit
/// les ops dès la première mutation.
#[test]
fn journal_drains_without_observer() {
    let mut eng = JscoreEngine::new();
    eng.install_dom("<html><body><p id='x'>a</p></body></html>", "https://x/", 1);
    // `CharacterData.data` est la voie `characterData` /
    // `set_text` (`textContent=` passe par `replaceChildren` —
    // un `childList`, spec).
    eng.eval("document.getElementById('x').firstChild.data = 'b';")
        .unwrap();
    let j = eng
        .drain_dom_mutations()
        .expect("mutation attachée → journal");
    assert!(
        j.ops.iter().any(|op| matches!(
            op,
            explorer_js::DomMutation::SetText { text, .. } if text == "b"
        )),
        "SetText attendu : {:?}",
        j.ops
    );
    // Journal drainé → le drain suivant est vide → voie keyed.
    assert!(eng.drain_dom_mutations().is_none());
}

/// Forme des ops : `appendChild` → `Insert` portant le sous-arbre
/// sérialisé + ses ids miroir ; `removeChild` → `Remove`.
#[test]
fn journal_insert_and_remove_ops() {
    let mut eng = JscoreEngine::new();
    eng.install_dom(
        "<html><body><ul id='l'><li>a</li></ul></body></html>",
        "https://x/",
        1,
    );
    eng.eval(
        "var l = document.getElementById('l');\
         var li = document.createElement('li');\
         li.textContent = 'b';\
         l.appendChild(li);\
         l.removeChild(l.firstChild);",
    )
    .unwrap();
    let j = eng.drain_dom_mutations().expect("journal");
    let ins = j.ops.iter().find_map(|op| match op {
        explorer_js::DomMutation::Insert { html, ids, .. } => Some((html, ids)),
        _ => None,
    });
    let (html, ids) = ins.expect("Insert attendu");
    assert!(html.contains("<li"), "sous-arbre inséré : {html}");
    assert!(!ids.is_empty(), "ids miroir du sous-arbre");
    assert!(
        j.ops
            .iter()
            .any(|op| matches!(op, explorer_js::DomMutation::Remove { .. })),
        "Remove attendu : {:?}",
        j.ops
    );
    // SetText de `li.textContent='b'` : le nœud texte vivait
    // détaché à la mutation → capté via l'Insert (état au
    // drain), jamais comme op autonome.
}

/// `setAttribute`/`removeAttribute` → `SetAttr` valeur relue
/// au drain (le plus récent subsume).
#[test]
fn journal_attr_ops_carry_latest_value() {
    let mut eng = JscoreEngine::new();
    eng.install_dom("<html><body><p id='x'></p></body></html>", "https://x/", 1);
    eng.eval(
        "var x = document.getElementById('x');\
         x.setAttribute('class','a');\
         x.setAttribute('class','b');\
         x.setAttribute('title','t');\
         x.removeAttribute('title');",
    )
    .unwrap();
    let j = eng.drain_dom_mutations().expect("journal");
    let set: Vec<_> = j
        .ops
        .iter()
        .filter(|op| matches!(op, explorer_js::DomMutation::SetAttr { .. }))
        .collect();
    assert!(
        set.iter().any(|op| matches!(
            op,
            explorer_js::DomMutation::SetAttr { name, value: Some(v), .. }
                if name == "class" && v == "b"
        )),
        "valeur relue au drain = 'b' : {set:?}"
    );
    assert!(
        set.iter().any(|op| matches!(
            op,
            explorer_js::DomMutation::SetAttr { name, value: None, .. } if name == "title"
        )),
        "removeAttribute → value None : {set:?}"
    );
}

/// Baseline cléée : `sync_dom_keyed` livre `ids` alignés sur
/// le pré-ordre de sérialisation, et **clôt** le journal.
#[test]
fn keyed_baseline_supersedes_journal() {
    let mut eng = JscoreEngine::new();
    eng.install_dom("<html><body><p id='x'>a</p></body></html>", "https://x/", 1);
    eng.eval("document.getElementById('x').textContent = 'b';")
        .unwrap();
    let k = eng.sync_dom_keyed().expect("keyed");
    assert!(!k.ids.is_empty(), "ids pré-ordre livrés");
    assert!(k.html.contains(">b<"), "html à jour : {}", k.html);
    // Le journal antérieur est subsumé — drain suivant vide.
    assert!(eng.drain_dom_mutations().is_none());
}

/// Hors pilote : insertion sous `<table>` (foster parenting)
/// → classée non rejouable → `drain_dom_mutations` rend
/// `None` **sans consommer dirty** — `sync_dom_keyed` livre
/// alors la baseline complète.
#[test]
fn journal_risky_parent_falls_back_to_keyed() {
    let mut eng = JscoreEngine::new();
    eng.install_dom(
        "<html><body><table id='t'><tr><td>a</td></tr></table></body></html>",
        "https://x/",
        1,
    );
    eng.eval(
        "var t = document.getElementById('t');\
         var tr = document.createElement('tr');\
         tr.innerHTML = '<td>b</td>';\
         t.appendChild(tr);",
    )
    .unwrap();
    assert!(
        eng.drain_dom_mutations().is_none(),
        "parent <table> → repli re-parse"
    );
    let k = eng.sync_dom_keyed().expect("baseline après rejet");
    assert!(
        k.html.contains("<td>b</td>"),
        "mutations livrées : {}",
        k.html
    );
}

/// `<template>.content` déplace les enfants vers un fragment
/// détaché — chemin non tracé par le journal → poison → repli.
#[test]
fn journal_template_content_poisons() {
    let mut eng = JscoreEngine::new();
    eng.install_dom(
        "<html><body><template id='t'><p>a</p></template></body></html>",
        "https://x/",
        1,
    );
    // La simple matérialisation de `.content` (déplacement des
    // enfants) empoisonne : les mutations ultérieures sur eux
    // seraient invisibles au journal.
    eng.eval(
        "var t = document.getElementById('t');\
         var c = t.content;\
         document.body.appendChild(document.createElement('div'));",
    )
    .unwrap();
    assert!(
        eng.drain_dom_mutations().is_none(),
        "template.content → poison → re-parse"
    );
    let k = eng.sync_dom_keyed().expect("baseline après poison");
    assert!(k.html.contains("template"), "{}", k.html);
}
