// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! JS-C447 (JT-2) — `DocumentFragment` constructible (`WebIDL`
//! `[Constructor]` — gitlab `illegal constructor` abortait le
//! bootstrap) : `new DocumentFragment()` = fragment détaché
//! identique à `createDocumentFragment()`, chaîne
//! `instanceof Node`/`DocumentFragment` conservée, et
//! `ShadowRoot.prototype` hérite toujours
//! `DocumentFragment.prototype`.

use explorer_jscore::{Dom, Engine};
use std::cell::RefCell;
use std::rc::Rc;

const PAGE: &str = "<html><body><div id=d></div></body></html>";

fn engine() -> Engine {
    let mut e = Engine::new();
    e.install_dom(&Rc::new(RefCell::new(Dom::from_html(PAGE))));
    e
}

fn ev(e: &mut Engine, src: &str) -> String {
    match e.eval(src) {
        Ok(v) => e.describe(v),
        Err(err) => panic!("{src} -> {err}"),
    }
}

/// `new DocumentFragment()` produit un fragment détaché :
/// `nodeType` 11, vide, chaîne `instanceof` complète.
#[test]
fn c447_docfrag_ctor_returns_detached_fragment() {
    let mut e = engine();
    assert_eq!(
        ev(
            &mut e,
            "var f=new DocumentFragment();\
             f.nodeType+'|'+f.childNodes.length+'|'+(f instanceof DocumentFragment)+'|'+(f instanceof Node)"
        ),
        "11|0|true|true"
    );
    // Deux instances = deux nœuds distincts, et égal de
    // `createDocumentFragment()` (même espèce, pas même nœud).
    assert_eq!(
        ev(
            &mut e,
            "new DocumentFragment()!==document.createDocumentFragment()\
             &&(document.createDocumentFragment() instanceof DocumentFragment)\
             &&(new DocumentFragment()!==new DocumentFragment())"
        ),
        "true"
    );
}

/// Interfaces `WebIDL` ctor-only : `DocumentFragment()` en
/// appel fonctionnel = `TypeError` (comme `Node()`/`Element()`).
#[test]
fn c447_docfrag_call_without_new_throws() {
    let mut e = engine();
    let err = e.eval("DocumentFragment()").expect_err("ctor-only");
    let msg = format!("{err}");
    assert!(
        msg.contains("llegal constructor"),
        "message inattendu: {msg}"
    );
}

/// Le fragment construit accepte des enfants et son insertion
/// dans un parent les éclate (spec — jQuery l'exige, même
/// règle que `createDocumentFragment`).
#[test]
fn c447_docfrag_insertion_splices_children() {
    let mut e = engine();
    assert_eq!(
        ev(
            &mut e,
            "var f=new DocumentFragment();\
             f.appendChild(document.createElement('span'));\
             f.appendChild(document.createElement('b'));\
             document.body.appendChild(f);\
             f.childNodes.length+'|'+document.body.children.length"
        ),
        "0|3"
    );
}

/// `ShadowRoot.prototype` continue d'hériter
/// `DocumentFragment.prototype` (la chaîne posée par la
/// boucle d'interfaces survit au vrai ctor) — le polyfill
/// `ShadyDOM` en dépend (JS-C321).
#[test]
fn c447_shadowroot_proto_chain_preserved() {
    let mut e = engine();
    assert_eq!(
        ev(
            &mut e,
            "(Object.getPrototypeOf(ShadowRoot.prototype)===DocumentFragment.prototype)\
             +'|'+(DocumentFragment.prototype.constructor===DocumentFragment)"
        ),
        "true|true"
    );
}

/// Les méthodes propres récoltées sur `DocumentFragment.
/// prototype` (JS-C416 — `getElementById`, nav accessors)
/// sont visibles depuis une instance construite.
#[test]
fn c447_docfrag_proto_methods_visible() {
    let mut e = engine();
    assert_eq!(
        ev(
            &mut e,
            "var f=new DocumentFragment();var s=document.createElement('i');\
             s.id='x';f.appendChild(s);\
             typeof f.getElementById+'|'+(f.getElementById('x')===s)"
        ),
        "function|true"
    );
}
