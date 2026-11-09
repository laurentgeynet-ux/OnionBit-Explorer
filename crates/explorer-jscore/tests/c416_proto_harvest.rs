// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! JS-C416 / §B-97 — récolte `ShadyDOM` des méthodes natives :
//! `E(X.prototype, […])` lit `Object.getOwnPropertyDescriptor`
//! — propre **seulement**. Les méthodes spec `Element`/`Document`/
//! `DocumentFragment`/`HTMLElement` vivaient héritées de
//! `Node.prototype` ou invoke-only → `nativeMethods.setAttribute`
//! restait vide → `TypeError` à l'init webcomponents-sd →
//! upgrades CE bloqués (squelettes `ytd-*`, muts=0, vol 700).
//! Verrou : descripteurs PROPRES + `.call(this)` fonctionnel.

use explorer_jscore::{Dom, Engine};
use std::cell::RefCell;
use std::rc::Rc;

fn engine_with_dom() -> Engine {
    let mut e = Engine::new();
    let dom: explorer_jscore::DomRef = Rc::new(RefCell::new(Dom::from_html(
        "<html><body><p id=x>hi</p><template id=t></template></body></html>",
    )));
    e.install_dom(&dom);
    e
}

fn ev(e: &mut Engine, src: &str) -> String {
    e.eval(src)
        .map_or_else(|x| format!("ERR {x}"), |v| e.describe(v))
}

/// Propre + data `function` pour chaque nom de la liste.
fn own_fns(e: &mut Engine, proto: &str, names: &str) -> String {
    ev(
        e,
        &format!(
            "[{names}].map(function(k){{var d=Object.getOwnPropertyDescriptor({proto}.prototype,k);return k+':'+(d&&d.value?'v':'X')}}).filter(function(s){{return s.slice(-2)===':X'}}).join(',')||'ok'",
        ),
    )
}

#[test]
fn element_proto_owns_spec_methods() {
    let mut e = engine_with_dom();
    assert_eq!(
        own_fns(
            &mut e,
            "Element",
            "'setAttribute','getAttribute','hasAttribute','removeAttribute','toggleAttribute','focus','blur','append','prepend','replaceChildren','getAttributeNames','insertAdjacentHTML','attachInternals'"
        ),
        "ok",
        "Element.prototype doit posséder ses méthodes spec (propre)"
    );
}

#[test]
fn element_proto_method_call_dispatches() {
    let mut e = engine_with_dom();
    // `nativeMethods.setAttribute.call(el, …)` — le chemin exact
    // qui TypeErrorait dans `Kd` (webcomponents-sd.js:152:259).
    assert_eq!(
        ev(
            &mut e,
            "var el=document.getElementById('x');var f=Object.getOwnPropertyDescriptor(Element.prototype,'setAttribute').value;f.call(el,'data-v','1');el.getAttribute('data-v')"
        ),
        "1",
    );
    // `toggleAttribute`/`getAttributeNames` — idem, voie trampoline.
    assert_eq!(
        ev(
            &mut e,
            "var el=document.getElementById('x');Object.getOwnPropertyDescriptor(Element.prototype,'toggleAttribute').value.call(el,'hidden');el.hasAttribute('hidden')"
        ),
        "true",
    );
}

#[test]
fn html_element_proto_owns_harvest_accessors() {
    let mut e = engine_with_dom();
    // `children`/`innerHTML`/`className` — accesseurs propres
    // (`get` défini) que `E()` copie dans `nativeTree`.
    assert_eq!(
        ev(
            &mut e,
            "['children','innerHTML','className'].map(function(k){var d=Object.getOwnPropertyDescriptor(HTMLElement.prototype,k);return k+':'+(d&&d.get?'acc':'X')}).join(',')"
        ),
        "children:acc,innerHTML:acc,className:acc",
    );
    // Le getter className fonctionne via `.call(el)`.
    assert_eq!(
        ev(
            &mut e,
            "var el=document.getElementById('x');el.className='c1';Object.getOwnPropertyDescriptor(HTMLElement.prototype,'className').get.call(el)"
        ),
        "c1",
    );
}

#[test]
fn document_proto_owns_doc_methods() {
    let mut e = engine_with_dom();
    assert_eq!(
        own_fns(
            &mut e,
            "Document",
            "'getElementById','elementFromPoint','importNode','querySelector','querySelectorAll','append','prepend','replaceChildren','createElement'"
        ),
        "ok",
    );
    // `.call(document)` sur la méthode propre du proto.
    assert_eq!(
        ev(
            &mut e,
            "var f=Object.getOwnPropertyDescriptor(Document.prototype,'getElementById').value;f.call(document,'x').id"
        ),
        "x",
    );
    // Le trio EventTarget reste hérité (propre là-haut, pas ici).
    assert_eq!(
        ev(
            &mut e,
            "Object.getOwnPropertyDescriptor(Document.prototype,'addEventListener')===undefined"
        ),
        "true",
    );
}

#[test]
fn document_fragment_proto_surface() {
    let mut e = engine_with_dom();
    assert_eq!(
        own_fns(
            &mut e,
            "DocumentFragment",
            "'getElementById','querySelector','querySelectorAll','append','prepend','replaceChildren'"
        ),
        "ok",
    );
    // Nav getters propres (ParentNode) — `children` accesseur.
    assert_eq!(
        ev(
            &mut e,
            "var d=Object.getOwnPropertyDescriptor(DocumentFragment.prototype,'children');d&&d.get?'acc':'X'"
        ),
        "acc",
    );
    // Appel réel sur un fragment.
    assert_eq!(
        ev(
            &mut e,
            "var fr=document.createDocumentFragment();Object.getOwnPropertyDescriptor(DocumentFragment.prototype,'append').value.call(fr,document.createTextNode('t'));fr.childNodes.length"
        ),
        "1",
    );
}

#[test]
fn template_proto_owns_inner_html() {
    let mut e = engine_with_dom();
    assert_eq!(
        ev(
            &mut e,
            "var d=Object.getOwnPropertyDescriptor(HTMLTemplateElement.prototype,'innerHTML');d?(d.get?'acc':'v'):'X'"
        ),
        "acc",
    );
}

#[test]
fn node_proto_surface_unchanged() {
    let mut e = engine_with_dom();
    // `Node.prototype` garde ses méthodes propres (chaîne
    // d'héritage inchangée) — la copie Element est additive.
    assert_eq!(
        own_fns(
            &mut e,
            "Node",
            "'appendChild','setAttribute','cloneNode','contains'"
        ),
        "ok",
    );
    // Et `el instanceof`/`in` reste vrai par la chaîne.
    assert_eq!(
        ev(&mut e, "'setAttribute' in document.getElementById('x')"),
        "true",
    );
}
