// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! JS-C348 / mur §B-100 (vol 651) — la pompe de microtâches
//! était **ré-entrante** : une native drainant depuis un
//! callback de job (`dispatch_media`, livraison XHR,
//! `fire_dom_event`, timers, crypto) empilait un drain par
//! cycle → ~600 frames JS / ~9 frames par cycle (dumps
//! `[expr_bisect]` du vol) → `RangeError` « expression too
//! deeply nested » → bootstrap `YouTube` mort en plein chaînage
//! `.then` (SABR jamais armé, `mseOffsets=[0,0]`).
//!
//! La garde rend le drain imbriqué no-op : les jobs enfilés
//! sont dépoppés par la boucle externe au même niveau de pile
//! — sémantique « microtask checkpoint » spec (un seul drain,
//! itératif, pile remise à plat entre deux jobs).
//!
//! Trigger JS-visible : sous claim média, `v.currentTime = n`
//! dispatche `seeking` puis appelle `run_microtasks` —
//! imbriqué quand il court dans un callback de job.

use explorer_jscore::{Dom, Engine};

const PAGE: &str = r#"<html><body><video id="v"></video></body></html>"#;

fn engine() -> (Engine, std::rc::Rc<std::cell::RefCell<Dom>>) {
    let mut e = Engine::new();
    let dom = std::rc::Rc::new(std::cell::RefCell::new(Dom::from_html(PAGE)));
    e.install_dom(&dom);
    (e, dom)
}

fn ev(e: &mut Engine, src: &str) -> String {
    match e.eval(src) {
        Ok(v) => e.describe(v),
        Err(err) => panic!("{src} -> {err}"),
    }
}

/// Ordre observable du checkpoint : la réaction enfilée par le
/// listener `seeking` ne doit **pas** courir pendant le drain
/// imbriqué (avant garde : `T` avant `A`), mais au job suivant
/// de la boucle externe (`A` puis `T` puis `E`).
#[test]
fn nested_drain_inside_job_defers_to_outer_loop() {
    let (mut e, dom) = engine();
    let node = dom.borrow().by_id("v").expect("v node");
    e.set_media_claim(node, 7, Some(vec!["avc1".into()]));
    ev(
        &mut e,
        "var log=[];var v=document.getElementById('v');\
         v.addEventListener('seeking',function(){\
           log.push('L');\
           Promise.resolve().then(function(){log.push('T')});});\
         Promise.resolve().then(function(){\
           log.push('J');\
           v.currentTime=5;\
           log.push('A');}).then(function(){log.push('E')});'armed'",
    );
    assert_eq!(ev(&mut e, "log.join(',')"), "J,L,A,T,E");
}

/// Aplatissement : une chaîne `.then` dont chaque callback
/// redéclenche un drain imbriqué (`currentTime` → `seeking`)
/// empilait ~1 drain par niveau → `RangeError` vers ~80 cycles
/// sous la borne `call_depth`. Avec la garde, 400 cycles
/// s'exécutent à plat — la boucle externe dépoppe chaque job.
#[test]
fn chained_then_with_native_drain_stays_flat() {
    let (mut e, dom) = engine();
    let node = dom.borrow().by_id("v").expect("v node");
    e.set_media_claim(node, 7, Some(vec!["avc1".into()]));
    ev(
        &mut e,
        "var n=0;var v=document.getElementById('v');\
         function step(){n++;if(n<400){v.currentTime=n;\
           return Promise.resolve().then(step);}}\
         Promise.resolve().then(step);'armed'",
    );
    assert_eq!(ev(&mut e, "n"), "400");
}
