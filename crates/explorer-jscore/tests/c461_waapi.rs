// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! JS-C461 — surface WAAPI constructible (vol 733 §B-124 —
//! `uncaught TypeError: illegal constructor (KeyframeEffect)`
//! à `late,app:5743:23` coupait la cascade d'hydratation
//! YouTube accueil) : `new KeyframeEffect(el, frames, opts)`
//! et `new Animation(effect?, timeline?)` WebIDL
//! `[Constructor]`, `effect`/`timeline` servis, machine à
//! états `idle → running`, `el.animate` pose un KFE
//! implicite + `document.timeline`, identité `instanceof`
//! unifiée entre wrappers et ctor.

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

/// `new KeyframeEffect(...)` ne jette plus : cible/frames/
/// options lisibles, proto marqué, `instanceof` unifié.
#[test]
fn c461_keyframe_effect_constructible() {
    let mut e = engine();
    assert_eq!(
        ev(
            &mut e,
            "var el=document.getElementById('d');\
             var kf=new KeyframeEffect(el,[{opacity:0},{opacity:1}],{duration:300});\
             (kf.target===el)+'|'+(kf instanceof KeyframeEffect)+'|'\
             +kf.pseudoElement+'|'+(typeof kf.getKeyframes)+'|'\
             +(kf.getKeyframes().length===2)"
        ),
        "true|true|null|function|true"
    );
}

/// `new Animation(kfe, tl)` : `idle` au ctor (spec), `play()`
/// démarre et résout `ready`, `effect`/`timeline` rendent
/// les args, `instanceof Animation` sur le wrapper.
#[test]
fn c461_animation_ctor_machine_etat() {
    let mut e = engine();
    assert_eq!(
        ev(
            &mut e,
            "var el=document.getElementById('d');\
             var kf=new KeyframeEffect(el,[],1000);\
             var a=new Animation(kf,document.timeline);\
             a.playState+'|'+(a.effect===kf)+'|'+(a instanceof Animation)"
        ),
        "idle|true|true"
    );
    assert_eq!(
        ev(&mut e, "a.play();a.playState+'|'+(a.pending===false)"),
        "running|true"
    );
    // `finish()`/`cancel()` hérités de la machine CAP-187.
    assert_eq!(
        ev(
            &mut e,
            "a.finish();a.playState+'|'+(a instanceof Animation)"
        ),
        "finished|true"
    );
}

/// `el.animate` pose un `effect` KeyframeEffect et une
/// `timeline` non nulles — identité `instanceof` croisée
/// entre wrapper implicite et ctor.
#[test]
fn c461_animate_effect_timeline_exposes() {
    let mut e = engine();
    assert_eq!(
        ev(
            &mut e,
            "var a=document.getElementById('d').animate([{opacity:0}],500);\
             (a instanceof Animation)+'|'+(a.effect instanceof KeyframeEffect)\
             +'|'+(a.timeline!==null&&a.timeline!==undefined)\
             +'|'+(a.playState)"
        ),
        "true|true|true|running"
    );
}

/// `document.timeline` est un singleton stable (deux
/// lectures = même objet) aux horloges honnêtes à 0.
#[test]
fn c461_document_timeline_singleton() {
    let mut e = engine();
    assert_eq!(
        ev(
            &mut e,
            "document.timeline===document.timeline&&document.timeline!==undefined"
        ),
        "true"
    );
}
