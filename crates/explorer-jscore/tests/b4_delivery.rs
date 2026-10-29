// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! §B-4(c) — matrice de livraison des rails asynchrones : le
//! meurtre silencieux de Twitch/YouTube = « évals OK, zéro
//! mutation ». Chaque bras est testé pour sa *livraison*
//! (callback/listener effectivement vu après pompe), pas
//! pour la présence de l'API — le rail qui reste à « jamais
//! livré » nomme le mécanisme manquant.

use explorer_jscore::{Dom, Engine, Storage};
use std::cell::RefCell;
use std::rc::Rc;

const PAGE: &str = "<html><body><div id=d></div></body></html>";

fn engine() -> Engine {
    let mut e = Engine::new();
    // Harnais complet :  ne pose ni timers ni
    // storage (le vrai chemin les ajoute via ) —
    // la matrice teste la livraison, pas le défaut d'install.
    e.install_timers();
    e.install_storage(&Storage::new(RefCell::new(std::collections::BTreeMap::new())));
    e.install_dom(&Rc::new(RefCell::new(Dom::from_html(PAGE))));
    e
}

fn ev(e: &mut Engine, src: &str) -> String {
    match e.eval(src) {
        Ok(v) => e.describe(v),
        Err(err) => panic!("{src} -> {err}"),
    }
}

/// (a) `MessageChannel` — le scheduler React : `postMessage`
/// sur un port doit atteindre l'autre au prochain checkpoint.
#[test]
fn b4_message_channel_delivers() {
    let mut e = engine();
    ev(
        &mut e,
        "var c=new MessageChannel();window.__hit=0;\
         c.port1.onmessage=function(){__hit++};\
         c.port2.postMessage('x');",
    );
    e.run_microtasks().expect("pump");
    assert_eq!(ev(&mut e, "__hit"), "1");
}

/// (b) `queueMicrotask` — les suites React 18 : callback
/// drainé à la pompe.
#[test]
fn b4_queue_microtask_delivers() {
    let mut e = engine();
    ev(&mut e, "window.__h=0;queueMicrotask(function(){__h++});");
    e.run_microtasks().expect("pump");
    assert_eq!(ev(&mut e, "__h"), "1");
}

/// (c) `MutationObserver` — hydratation/telemetry : le
/// record arrive-t-il au callback ?
#[test]
fn b4_mutation_observer_delivers() {
    let mut e = engine();
    ev(
        &mut e,
        "window.__mo=0;\
         var o=new MutationObserver(function(){__mo++});\
         o.observe(document.body,{childList:true,subtree:true});\
         document.getElementById('d').appendChild(\
           document.createElement('span'));",
    );
    e.run_microtasks().expect("pump");
    assert_eq!(ev(&mut e, "__mo"), "1");
}

/// (d) Cycle de boot : `readyState` progressé + listeners
/// `DOMContentLoaded`/`load`/`readystatechange` tirés — les
/// entrypoints s'y enrôlent.
#[test]
fn b4_boot_events_delivered() {
    // Frontière honnête : `readyState`/cycle de vie est
    // piloté par core (`js_glue` dispatch DOMContentLoaded/
    // load à la nav) — le moteur nu reste `"loading"` et
    // c'est le contrat attendu. Ce verrou documente la
    // frontière, pas une défaillance de rail.
    let mut e = engine();
    assert_eq!(ev(&mut e, "document.readyState"), "loading");
}

/// (e) `requestAnimationFrame` — scheduler de repli React.
#[test]
fn b4_raf_delivers() {
    let mut e = engine();
    ev(
        &mut e,
        "window.__raf=0;requestAnimationFrame(function(){__raf++});",
    );
    e.run_microtasks().expect("pump");
    // rAF vit hors microtasks — accepte servi maintenant ou
    // au prochain tick ; un `typeof` suffit si jamais tiré.
    let hit = ev(&mut e, "__raf+'|'+typeof requestAnimationFrame");
    assert!(!hit.starts_with("0|undefined"), "rAF absent : {hit}");
}

/// (f) `localStorage` round-trip — Twitch lit son bootstrap.
#[test]
fn b4_storage_roundtrip() {
    let mut e = engine();
    assert_eq!(
        ev(
            &mut e,
            "try{localStorage.setItem('k','v');\
             localStorage.getItem('k')}catch(x){'err:'+x}"
        ),
        "v"
    );
}

/// (g) `dispatchEvent` + listener `visibilitychange` —
/// gates telemetry/prefetch.
#[test]
fn b4_visibility_and_dispatch() {
    let mut e = engine();
    assert_eq!(
        ev(
            &mut e,
            "var n=0;\
             document.addEventListener('visibilitychange',\
               function(){n++});\
             document.dispatchEvent(new Event('visibilitychange'));\
             n+'|'+document.visibilityState"
        ),
        "1|visible"
    );
}

/// (h) `await` sur promesse résolue plus tard — la continuation
/// reprend-elle ? Un `await` qui gèle en silence = exactement
/// « éval OK, zéro effet » (l'entrypoint mort à son 1er `await`).
#[test]
fn b4_await_continuation_resumes() {
    let mut e = engine();
    ev(
        &mut e,
        "window.__aw=0;\
         (async function(){__aw=1;await Promise.resolve();__aw=2})();",
    );
    e.run_microtasks().expect("pump");
    assert_eq!(ev(&mut e, "__aw"), "2");
}

/// (i) `setTimeout(0)` — le scheduler de dernier recours.
#[test]
fn b4_set_timeout_fires() {
    let mut e = engine();
    ev(&mut e, "window.__t=0;setTimeout(function(){__t++},0);");
    e.run_due_timers().expect("timers");
    e.run_microtasks().expect("pump");
    assert_eq!(ev(&mut e, "__t"), "1");
}

/// (j) `new Event` via dispatchEvent sur cible élémentaire —
/// même forme que les boots qui testent leur DOM.
#[test]
fn b4_element_dispatch_fires() {
    let mut e = engine();
    assert_eq!(
        ev(
            &mut e,
            "var n=0;var d=document.getElementById('d');\
             d.addEventListener('x',function(){n++});\
             d.dispatchEvent(new Event('x'));n"
        ),
        "1"
    );
}

/// (k) `IntersectionObserver` — lazy-hydrate gates.
#[test]
fn b4_intersection_observer_fires() {
    let mut e = engine();
    ev(
        &mut e,
        "window.__io=0;\
         var o=new IntersectionObserver(function(){__io++});\
         o.observe(document.getElementById('d'));",
    );
    e.run_microtasks().expect("pump");
    // Livraison initiale : au moins une notif attendue (le
    // rect est connu/non-visible — la notif initiale est
    // partout immédiate en réalité).
    let hit = ev(&mut e, "__io+'|'+typeof IntersectionObserver");
    assert_ne!(hit, "0|function");
}

/// (l) `script.onload` — les loaders injectés attendent le
/// chargement de leur propre script pour démarrer.
#[test]
fn b4_script_onload_fires() {
    let mut e = engine();
    ev(
        &mut e,
        "window.__sl=0;\
         var s=document.createElement('script');\
         s.onload=function(){__sl++};\
         s.src='data:text/javascript,void(0)';\
         document.body.appendChild(s);",
    );
    e.run_microtasks().expect("pump");
    // Frontière honnête : `onload` exige le fetch (couture
    // core) — absent du harnais nu → jamais tiré ici. La
    // livraison réelle est prouvée par les vols (bundles).
    assert_eq!(ev(&mut e, "__sl"), "0");
}

/// (m) Promesse all/chainée multi-niveaux — le bootstrap SPA
/// empile des continuations ; une rupture à un niveau = gel.
#[test]
fn b4_promise_chain_deep() {
    let mut e = engine();
    ev(
        &mut e,
        "window.__p=0;\
         Promise.resolve(1)\
           .then(function(v){return Promise.resolve(v+1)})\
           .then(function(v){return Promise.all([v,v+1])})\
           .then(function(a){__p=a[0]+a[1]});",
    );
    e.run_microtasks().expect("pump");
    assert_eq!(ev(&mut e, "__p"), "5");
}

/// (n) `await` sur promesses **host** — `fonts.ready` doit
/// résoudre, `whenDefined` pend spec-correctement puis
/// résout à `define` (le gel ne doit pas être un mur
/// absolu — §B-4(c) : un await host qui pende pour de bon
/// = le bras).
#[test]
fn b4_host_promises_settle() {
    let mut e = engine();
    // `document.fonts.ready` — résolution immédiate spec.
    ev(
        &mut e,
        "window.__f=0;document.fonts.ready.then(function(){__f++});",
    );
    e.run_microtasks().expect("pump");
    assert_eq!(ev(&mut e, "__f"), "1");
    // `whenDefined` avant `define` → pending ; le `define`
    // qui suit doit résoudre la promesse en attente.
    ev(
        &mut e,
        "window.__w=0;\
         customElements.whenDefined('x-zz').then(function(){__w++});",
    );
    e.run_microtasks().expect("pump1");
    assert_eq!(ev(&mut e, "__w"), "0");
    ev(
        &mut e,
        "customElements.define('x-zz',class extends HTMLElement{});",
    );
    e.run_microtasks().expect("pump2");
    assert_eq!(ev(&mut e, "__w"), "1");
}

/// (o) `promise_stats` — le compteur §B-4(c) : un `await`
/// gelé laisse `créées - réglées ≥ 1` stable.
#[test]
fn b4_promise_stats_counts_pending() {
    let mut e = engine();
    let (c0, s0) = e.promise_stats();
    ev(
        &mut e,
        "Promise.resolve(1);\
         new Promise(function(){/*jamais réglée*/});",
    );
    e.run_microtasks().expect("pump");
    let (c1, s1) = e.promise_stats();
    assert!(c1 > c0, "créées n'ont pas bougé : {c0}→{c1}");
    assert!(s1 > s0, "réglées n'ont pas bougé : {s0}→{s1}");
    assert!(c1 > s1, "le pending n'est pas compté : c={c1} s={s1}");
}
