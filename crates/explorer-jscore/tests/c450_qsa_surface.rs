// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! JS-C450 (JT-5) — verrou de surface `querySelector(All)` :
//! le gap nommé en vol (jwplayer archive.org) n'est PAS
//! l'API — Document/Element/DocumentFragment/ShadowRoot
//! répondent tous, sélecteurs attribut/complexe/liste
//! compris. `categories of Null` résiduel = lecture de
//! config (`item.categories` sur playlist item null —
//! interne jwplayer, pas un trou DOM) : à nommer par sonde
//! au prochain vol, jamais fixé à l'aveugle.

use explorer_jscore::{Dom, Engine};
use std::cell::RefCell;
use std::rc::Rc;

fn engine() -> Engine {
    let mut e = Engine::new();
    e.install_dom(&Rc::new(RefCell::new(Dom::from_html(
        "<html><body><div class=jw><video src=x></video><p id=p></p></div></body></html>",
    ))));
    e
}

fn ev(e: &mut Engine, src: &str) -> String {
    match e.eval(src) {
        Ok(v) => e.describe(v),
        Err(err) => panic!("{src} -> {err}"),
    }
}

/// `querySelectorAll` répond sur tous les receveurs DOM —
/// document, élément, fragment, shadow root.
#[test]
fn c450_qsa_all_dom_receivers() {
    let mut e = engine();
    assert_eq!(ev(&mut e, "document.querySelectorAll('video').length"), "1");
    assert_eq!(
        ev(&mut e, "document.querySelectorAll('.jw video').length"),
        "1"
    );
    assert_eq!(
        ev(&mut e, "document.querySelectorAll('p, video').length"),
        "2"
    );
    // DocumentFragment — ParentNode complet.
    assert_eq!(
        ev(
            &mut e,
            "var f=document.createDocumentFragment();f.innerHTML='<i></i><b></b>';\
             f.querySelectorAll('*').length"
        ),
        "2"
    );
    // ShadowRoot — hérite DocumentFragment.prototype.
    assert_eq!(
        ev(
            &mut e,
            "var d=document.createElement('div');var s=d.attachShadow({mode:'open'});\
             s.innerHTML='<q></q>';\
             typeof s.querySelectorAll+'|'+s.querySelectorAll('q').length"
        ),
        "function|1"
    );
}

/// Sélecteurs de la classe jwplayer : attribut, descendant
/// classe+tag, enfant direct, liste — `tagName` majuscule.
#[test]
fn c450_qsa_selector_classes_and_tagname() {
    let mut e = engine();
    assert_eq!(
        ev(&mut e, "document.querySelectorAll('video[src]').length"),
        "1"
    );
    assert_eq!(
        ev(&mut e, "document.querySelectorAll('div > p').length"),
        "1"
    );
    assert_eq!(
        ev(&mut e, "document.querySelectorAll('video')[0].tagName"),
        "VIDEO"
    );
}
