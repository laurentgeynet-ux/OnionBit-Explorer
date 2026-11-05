// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! JS-C344 — `drain_ce_reactions` résilient aux erreurs
//! individuelles (mur §B-97 #6 du vol 644 : le lot était pris par
//! `mem::take` puis la première `Err` propagée — une seule
//! exception de `connectedCallback`/ctor sur ~80 `ytd-*` perdait
//! les réactions sœurs restantes, gel silencieux du montage).

use explorer_jscore::{Dom, Engine};
use std::cell::RefCell;
use std::rc::Rc;

const PAGE: &str = r#"<html><body><div id="c"></div></body></html>"#;

fn engine() -> Engine {
    let mut e = Engine::new();
    e.install_dom(&Rc::new(RefCell::new(Dom::from_html(PAGE))));
    e
}

/// `<bad-el>` throw dans `connectedCallback`, `<good-el>` pose
/// `this.connected = true` — la réaction sœur survit à
/// l'exception et l'erreur est tout de même propagée (première
/// `Thrown` conservée, pas avalée).
#[test]
fn throwing_custom_element_reaction_does_not_drop_sibling_reactions() {
    let mut e = engine();
    // Définitions + insertion des deux éléments dans le même
    // drain : l'exception de `bad` doit remonter sans perdre
    // la réaction `good`.
    let res = e.eval(
        "customElements.define('bad-el', class extends HTMLElement {\
           connectedCallback(){ throw new Error('bad-connect'); }\
         });\
         customElements.define('good-el', class extends HTMLElement {\
           connectedCallback(){ this.connected = true; }\
         });\
         var c = document.getElementById('c');\
         c.innerHTML = '<bad-el></bad-el><good-el></good-el>';\
         'inserted'",
    );
    // L'exception de `bad-el` est propagée (pas avalée).
    let err = match res {
        Err(err) => err,
        Ok(v) => panic!("attendait Err, obtenu {}", e.describe(v)),
    };
    let _ = err;
    // … mais `good-el` a bien reçu son `connectedCallback`.
    match e.eval("document.querySelector('good-el').connected") {
        Ok(v) => assert_eq!(e.describe(v), "true"),
        Err(err) => panic!("relance good-el.connected -> {err}"),
    }
    // … et le drain n'est pas bloqué : une insertion ultérieure
    // d'un élément sain reçoit sa réaction normalement.
    match e.eval(
        "customElements.define('late-el', class extends HTMLElement {\
           connectedCallback(){ this.late = 'ok'; }\
         });\
         document.getElementById('c').appendChild(document.createElement('late-el')).late",
    ) {
        Ok(v) => assert_eq!(e.describe(v), "ok"),
        Err(err) => panic!("relance late-el -> {err}"),
    }
}

/// Lot entièrement sain — `Ok(())` inchangé (pas de fausse
/// erreur quand aucune réaction ne jette).
#[test]
fn healthy_reaction_batch_returns_ok() {
    let mut e = engine();
    match e.eval(
        "customElements.define('ok-el', class extends HTMLElement {\
           connectedCallback(){ this.ok = 1; }\
         });\
         var c = document.getElementById('c');\
         c.innerHTML = '<ok-el></ok-el><ok-el></ok-el>';\
         c.querySelectorAll('ok-el')[1].ok",
    ) {
        Ok(v) => assert_eq!(e.describe(v), "1"),
        Err(err) => panic!("ok-el -> {err}"),
    }
}
