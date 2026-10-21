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
