// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! JS-C467 (§B-126) — « appropriate place for inserting a
//! node » sur `<template>` : `appendChild`/`insertBefore`
//! écrivent dans `content`, `template.childNodes` reste
//! vide (spec HTML — le trou mettait les enfants en light
//! children invisibles des walks → `iDo` Polymer undefined
//! → `addEventListener of Undefined` au stamp ytd-app).

use explorer_jscore::{Dom, Engine};
use std::cell::RefCell;
use std::rc::Rc;

fn engine() -> Engine {
    let mut e = Engine::new();
    e.install_dom(&Rc::new(RefCell::new(Dom::from_html(
        "<html><body></body></html>",
    ))));
    e
}

fn ev(e: &mut Engine, src: &str) -> String {
    match e.eval(src) {
        Ok(v) => e.describe(v),
        Err(err) => panic!("{src} -> {err}"),
    }
}

/// `template.appendChild` → `content`, `childNodes` vide.
#[test]
fn c467_template_appendchild_goes_to_content() {
    let mut e = engine();
    assert_eq!(
        ev(
            &mut e,
            "var t=document.createElement('template');\
             var d=document.createElement('div');d.id='x';\
             t.appendChild(d);\
             t.childNodes.length+'|'+t.content.childNodes.length+'|'+t.content.firstChild.id"
        ),
        "0|1|x"
    );
    // `insertBefore` idem (ref null ≙ append).
    assert_eq!(
        ev(
            &mut e,
            "var t=document.createElement('template');\
             var s=document.createElement('span');\
             t.insertBefore(s,null);\
             t.childNodes.length+'|'+t.content.lastChild.tagName"
        ),
        "0|SPAN"
    );
    // Position relatif : insertBefore dans content.
    assert_eq!(
        ev(
            &mut e,
            "var t=document.createElement('template');\
             t.appendChild(document.createElement('b'));\
             var s=document.createElement('i');\
             t.insertBefore(s,t.content.firstChild);\
             t.content.firstChild.tagName+'|'+t.content.childNodes.length"
        ),
        "I|2"
    );
}

/// Le chemin kevlar/dom-module : peuplement programmatique
/// puis stamp — les nœuds annotés résolvent (`iDo`).
#[test]
fn c467_template_programmatic_fill_then_stamp() {
    let mut e = engine();
    assert_eq!(
        ev(
            &mut e,
            "var t=document.createElement('template');\
             var d=document.createElement('div');\
             d.setAttribute('on-x','h');\
             var s=document.createElement('span');s.textContent='[[v]]';\
             d.appendChild(s);t.appendChild(d);\
             var F=document.importNode(t.content,true);\
             F.firstChild.tagName+'|'+F.firstChild.firstChild.textContent"
        ),
        "DIV|[[v]]"
    );
}

/// Même règle « appropriate place » sur les mutateurs de
/// remplacement et d'insertion en tête (résidu du live
/// path) : `replaceChildren`/`textContent=` écrivent dans
/// `content` — pas en light children — et `prepend` mesure
/// son repère sur `content` (sans lui : append déguisé une
/// fois `content` matérialisé).
#[test]
fn c467_template_replace_prepend_target_content() {
    let mut e = engine();
    // `replaceChildren` sur content matérialisé : l'ancien
    // enfant de `content` disparaît, `childNodes` reste vide.
    assert_eq!(
        ev(
            &mut e,
            "var t=document.createElement('template');\
             t.appendChild(document.createElement('b'));\
             var d=document.createElement('i');d.id='r';\
             t.replaceChildren(d);\
             t.childNodes.length+'|'+t.content.childNodes.length+'|'+t.content.firstChild.tagName"
        ),
        "0|1|I"
    );
    // `textContent=` — « string replace all » vise `content`.
    assert_eq!(
        ev(
            &mut e,
            "var t=document.createElement('template');\
             t.appendChild(document.createElement('b'));\
             t.textContent='x';\
             t.childNodes.length+'|'+t.content.childNodes.length+'|'+t.content.textContent"
        ),
        "0|1|x"
    );
    // `prepend` sur `content` déjà matérialisé — le repère
    // est le 1er enfant du fragment (sinon : append).
    assert_eq!(
        ev(
            &mut e,
            "var t=document.createElement('template');\
             t.appendChild(document.createElement('b'));\
             t.appendChild(document.createElement('u'));\
             t.prepend(document.createElement('i'));\
             t.content.firstChild.tagName+'|'+t.content.childNodes.length+'|'+t.content.lastChild.tagName"
        ),
        "I|3|U"
    );
}

/// Fragment appendé dans un `<template>` : aplatissement
/// dans `content` (pas en light children).
#[test]
fn c467_fragment_append_flattens_into_content() {
    let mut e = engine();
    assert_eq!(
        ev(
            &mut e,
            "var t=document.createElement('template');\
             var f=document.createDocumentFragment();\
             f.appendChild(document.createElement('i'));\
             f.appendChild(document.createElement('b'));\
             t.appendChild(f);\
             t.childNodes.length+'|'+t.content.childNodes.length+'|'+t.content.firstChild.tagName"
        ),
        "0|2|I"
    );
}
