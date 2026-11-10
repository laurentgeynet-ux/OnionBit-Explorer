// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! JS-C475 — `getAnimations()` remonte les animations **CSS
//! déclaratives** : la session marque le miroir via
//! `data-explorer-anim` (`nom` ou `nom|paused`) ; la lecture
//! matérialise des `Animation` honnêtes (`animationName`
//! servi, `playState` reflet de la marque), identité stable
//! entre deux appels, mortes retirées du registre.

use explorer_jscore::{Dom, Engine};
use std::cell::RefCell;
use std::rc::Rc;

const PAGE: &str = "<html><body><div id=d></div><span id=s></span></body></html>";

fn engine() -> (Engine, Rc<RefCell<Dom>>) {
    let mut e = Engine::new();
    let dom = Rc::new(RefCell::new(Dom::from_html(PAGE)));
    e.install_dom(&dom);
    (e, dom)
}

fn ev(e: &mut Engine, src: &str) -> String {
    match e.eval(src) {
        Ok(v) => e.describe(v),
        Err(err) => panic!("{src} -> {err}"),
    }
}

/// Élément marqué → `document.getAnimations()` rend une
/// `Animation` avec `animationName`/`playState` reflets de
/// la marque ; le scope `el.getAnimations()` filtre.
#[test]
fn css_mark_materielise_animation() {
    let (mut e, dom) = engine();
    {
        let mut d = dom.borrow_mut();
        let n = d.by_id("d").unwrap();
        d.set_attr(n, "data-explorer-anim", "spin");
        let s = d.by_id("s").unwrap();
        d.set_attr(s, "data-explorer-anim", "fade|paused");
    }
    assert_eq!(
        ev(
            &mut e,
            "var a=document.getAnimations();\
             var f=a.find(function(x){return x.animationName==='fade'});\
             a.length+'|'+f.playState+'|'+(f instanceof Animation)"
        ),
        "2|paused|true",
        "animations CSS matérialisées"
    );
    // Scope élément : seule l'animation de ce nœud.
    assert_eq!(
        ev(
            &mut e,
            "var a=document.getElementById('d').getAnimations();\
             a.length+'|'+a[0].animationName+'|'+a[0].playState"
        ),
        "1|spin|running"
    );
    // Identité stable : deux appels rendent le même objet.
    assert_eq!(
        ev(
            &mut e,
            "document.getAnimations()[0]===document.getAnimations()[0]"
        ),
        "true"
    );
    // Marque retirée → l'entrée sort du registre (spec
    // « relevant animations »).
    {
        let mut d = dom.borrow_mut();
        let n = d.by_id("d").unwrap();
        d.remove_attr(n, "data-explorer-anim");
    }
    assert_eq!(
        ev(&mut e, "document.getAnimations().length"),
        "1",
        "animation morte retirée"
    );
}
