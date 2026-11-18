// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Gate §B-165 — `CSSAnimation`/`CSSTransition` globales.
//!
//! Vol 874 squareup `Bnhti4GK.js:2:4767` :
//! `ReferenceError: CSSAnimation is not defined` — les
//! classes filles `WebIDL` d'`Animation` (Web Animations
//! spec) étaient absentes alors qu'`Animation`/
//! `KeyframeEffect` existent (JS-C461). Spec : les filles
//! n'ont **pas** de `[Constructor]` — surfaces globales +
//! chaîne de protos ; une animation CSS déclarative
//! (`data-explorer-anim`) EST une `CSSAnimation`.

use explorer_jscore::{Dom, Engine};
use std::cell::RefCell;
use std::rc::Rc;

fn eval(e: &mut Engine, src: &str) -> String {
    match e.eval(src) {
        Ok(v) => format!("{v:?}"),
        Err(err) => format!("ERR:{err:?}"),
    }
}

fn ok(e: &mut Engine, name: &str, src: &str) -> bool {
    let r = eval(e, src);
    if r != "Bool(true)" {
        eprintln!("[b165] FAIL {name} -> {r}");
        return false;
    }
    true
}

#[test]
fn b165_css_animation_classe_fille_d_animation() {
    let mut e = Engine::new();
    e.install_dom(&Rc::new(RefCell::new(Dom::from_html("<div id=x></div>"))));
    e.set_page_url("https://squareup.com/");

    let mut bad = false;
    // Les deux globales existent (le verbatim §B-165).
    bad |= !ok(
        &mut e,
        "globals",
        "typeof CSSAnimation === 'function' && typeof CSSTransition === 'function'",
    );
    // Chaîne de protos : `X.prototype` fille d'`Animation.prototype`.
    bad |= !ok(
        &mut e,
        "proto-chain",
        "Object.getPrototypeOf(CSSAnimation.prototype) === Animation.prototype \
         && Object.getPrototypeOf(CSSTransition.prototype) === Animation.prototype",
    );
    // Pas de `[Constructor]` WebIDL (spec) : `new` jette.
    bad |= !ok(
        &mut e,
        "illegal-ctor",
        "var t1=false,t2=false; try { new CSSAnimation(); } catch (er) { t1 = er instanceof TypeError; } \
         try { new CSSTransition(); } catch (er) { t2 = er instanceof TypeError; } t1 && t2",
    );
    // `el.animate` reste une `Animation` — PAS une `CSSAnimation`
    // (spec WAAPI : l'API crée la classe mère).
    bad |= !ok(
        &mut e,
        "animate-parent",
        "var w = document.getElementById('x').animate([{opacity:0}],{duration:1}); \
         w instanceof Animation && !(w instanceof CSSAnimation)",
    );
    // `getAnimations` remonte la même identité (wrapper stable).
    bad |= !ok(
        &mut e,
        "getAnimations-id",
        "document.getElementById('x').getAnimations()[0] === w",
    );
    // `constructor.name` spec via le proto installé.
    bad |= !ok(
        &mut e,
        "ctor-name",
        "CSSAnimation.prototype.constructor === CSSAnimation && CSSAnimation.name === 'CSSAnimation'",
    );
    // IDL `transitionProperty` : attribut du proto
    // `CSSTransition` (surface — pas de transition remontée).
    bad |= !ok(
        &mut e,
        "trans-prop",
        "'transitionProperty' in CSSTransition.prototype",
    );
    // `animationName` : `undefined` sur une anim née du script.
    bad |= !ok(&mut e, "animName-script", "w.animationName === undefined");
    // La marque CSS déclarative matérialise une `CSSAnimation`
    // : `instanceof` traversant la chaîne + `animationName`.
    bad |= !ok(
        &mut e,
        "animName-css",
        "document.getElementById('x').setAttribute('data-explorer-anim','spin'); \
         var g = document.getElementById('x').getAnimations(); \
         var ca = g.find(function(x){ return x.animationName === 'spin'; }); \
         ca instanceof CSSAnimation && ca instanceof Animation \
         && !(ca instanceof CSSTransition) && ca.animationName === 'spin' \
         && ca.constructor === CSSAnimation",
    );

    assert!(!bad, "b165 : échecs ci-dessus");
}
