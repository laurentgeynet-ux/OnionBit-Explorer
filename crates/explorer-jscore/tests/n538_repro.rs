// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Régression N5.38 — le vol a vu `uncaught: `_updatePromises`
//! is not a function (on Num(ts))` dans web-animations-next-lite.
//! Analyse du min.js (50 867 o, version yt exacte) : le seul
//! chemin vers `(on Num)` est `c._updatePromises()` dans le
//! `filter` de `animationsWithPromises` — un Num n'y entre que
//! si `push(this)` dans `get finished`/`get ready` tournait
//! avec `this`=Num (getter invoqué via `desc.get.call(t)` ou
//! proto greffé — **non producible par une cellule d'env** :
//! chaque lecture `Bindings` dérive son index du nom ou passe
//! la vérif `clé==name`). Ce test évalue le VRAI polyfill et
//! pilote sa boucle rAF (`f(c){b=v.timeline;b.currentTime=c;
//! b._discardAnimations();…}`) — le frame exact du drapeau —
//! à travers les envs canon partagés JS-C533.

use explorer_jscore::{Dom, Engine, Storage};
use std::cell::RefCell;
use std::rc::Rc;

const PAGE: &str = "<html><body><div id=d></div></body></html>";

fn engine() -> Engine {
    let mut e = Engine::new();
    e.install_timers();
    e.install_storage(&Storage::new(RefCell::new(
        std::collections::BTreeMap::new(),
    )));
    e.install_dom(&Rc::new(RefCell::new(Dom::from_html(PAGE))));
    e
}

#[test]
fn n538_polyfill_raf_loop() {
    let mut e = engine();
    let src = include_str!("data/wan.min.js");
    e.eval(src).expect("polyfill install doit passer");

    // Écritures/lectures directes sur `document.timeline`
    // (l'objet `a` du polyfill si la defProperty a posé, le
    // timeline hôte sinon — les deux doivent rester saines).
    let r = e
        .eval(
            "var tl=document.timeline;if(tl){for(var i=0;i<500;i++){tl.currentTime=i}}\
             tl?tl.currentTime:'no-tl'",
        )
        .expect("setter direct");
    assert_eq!(e.describe(r), "499");

    // Arme la boucle si possible (el.animate est natif hôte ici
    // — le polyfill garde sa propre timeline `v.timeline=a` que
    // `f` consulte ; la boucle tourne quand même si armée).
    let _ = e.eval("try{document.getElementById('d').animate&&document.getElementById('d').animate([{opacity:0},{opacity:1}],{duration:60000})}catch(x){}");

    // Pompe : chaque rAF tir appelle `f(ts)` — le frame exact
    // du drapeau — à travers des envs canon partagés (même
    // `Rc<[Rc<str>]>` d'un appel à l'autre).
    let mut fired = 0usize;
    for _ in 0..40 {
        std::thread::sleep(std::time::Duration::from_millis(20));
        fired += e.run_due_timers().expect("pump");
        let _ = e.run_microtasks();
    }
    assert!(fired > 0, "au moins un timer rAF doit avoir tiré");
}
