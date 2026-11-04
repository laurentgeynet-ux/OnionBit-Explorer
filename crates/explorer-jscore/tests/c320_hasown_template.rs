// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Verrous JS-C320 — mur terrain `ytd-app` youtube (§B-4(c)
//! `playlistHandlerActionMap of Undefined` @kevlar:40765) :
//! deux racines, une séquence.
//!
//! **(a) `hasOwnProperty` aveugle aux accesseurs propres** —
//! `has_own_property` lisait `o.get_own` (valeur data seule)
//! au lieu de `o.find` : `Object.defineProperty(t,k,{get,set})`
//! posait l'accesseur, `t.hasOwnProperty(k)` rendait `false` →
//! `IqL` (forwarding kevlar:3709) redéfinissait → rejet spec
//! non-configurable → `ready()` jetait → `attached()` jamais →
//! `Xcs` jamais posé → `detached` lisait `Xcs.*`.
//!
//! **(b) `cloneNode`/`importNode`/`adopt_from` perdaient le
//! contenu `<template>`** — les enfants d'un template vivent
//! dans le fragment `content`, pas dans `children` : le clone
//! (même profond) produisait un template vide. `_prepareTemplate`
//! (`P.cloneNode(!0)`) → `_stampTemplate` → `$` vide →
//! `guideInnerContent`/`guide` undefined → `ready` jetait.
//! Spec `clone a node` : les enfants de `content` sont copiés
//! **toujours** — `deep` ne gouverne que leur descendance.

use explorer_jscore::{Dom, Engine};
use std::cell::RefCell;
use std::rc::Rc;

fn eval(src: &str) -> String {
    let mut e = Engine::new();
    match e.eval(src) {
        Ok(v) => e.describe(v),
        Err(err) => panic!("eval: {err}\n  src: {src}"),
    }
}

fn eval_dom(src: &str) -> String {
    let mut e = Engine::new();
    e.install_dom(&Rc::new(RefCell::new(Dom::from_html(
        "<html><body></body></html>",
    ))));
    match e.eval(src) {
        Ok(v) => e.describe(v),
        Err(err) => panic!("eval: {err}\n  src: {src}"),
    }
}

/// (a) `hasOwnProperty` voit un accesseur propre — le cas réel
/// `IqL` : `{get,set}` posé puis re-testé → `true` attendu.
#[test]
fn has_own_sees_accessor() {
    assert_eq!(
        eval(
            "var o={};Object.defineProperty(o,'x',{get:function(){return 1},set:function(v){}});\
             o.hasOwnProperty('x')"
        ),
        "true"
    );
    // Setter seul + getter seul aussi.
    assert_eq!(
        eval(
            "var a={},b={};Object.defineProperty(a,'g',{get:function(){return 1}});\
             Object.defineProperty(b,'s',{set:function(v){}});\
             a.hasOwnProperty('g')+'|'+b.hasOwnProperty('s')"
        ),
        "true|true"
    );
}

/// (a) La séquence `IqL` verbatim : deux `defineProperty`
/// `{get,set}` gardés par `hasOwnProperty` — le second est sauté.
#[test]
fn iql_guard_sequence_no_throw() {
    assert_eq!(
        eval(
            "var el={};function IqL(n,Y){n.hasOwnProperty(Y)||Object.defineProperty(n,Y,{get:function(){return 0},set:function(v){}})}\
             try{IqL(el,'p');IqL(el,'p');'ok'}catch(x){'threw:'+x.name}"
        ),
        "ok"
    );
}

/// (a) `Object.hasOwn` couvrait déjà les accesseurs (`find`) —
/// parité avec `hasOwnProperty` désormais.
#[test]
fn object_hasown_accessor_parity() {
    assert_eq!(
        eval(
            "var o={};Object.defineProperty(o,'x',{get:function(){return 1}});\
             Object.hasOwn(o,'x')+'|'+o.hasOwnProperty('x')+'|'+('x' in o)"
        ),
        "true|true|true"
    );
}

/// (b) `cloneNode(true)` sur `<template>` : le fragment
/// `content` est copié — les enfants n'arrivent pas vides.
#[test]
fn template_clone_deep_copies_content() {
    assert_eq!(
        eval_dom(
            "var t=document.createElement('template');\
             t.innerHTML='<div id=\"a\"><span>x</span></div><b id=\"b\">y</b>';\
             var c=t.cloneNode(true),f=c.content;\
             f.childNodes.length+'|'+(f.querySelector('#a')?'1':'0')+'|'+(f.querySelector('#b')?'1':'0')+'|'+(f.querySelector('span')?'1':'0')"
        ),
        "2|1|1|1"
    );
}

/// (b) `cloneNode(false)` sur template : les enfants de
/// `content` sont copiés superficiellement (spec) — présents,
/// mais sans leur propre descendance.
#[test]
fn template_clone_shallow_keeps_content_children() {
    assert_eq!(
        eval_dom(
            "var t=document.createElement('template');\
             t.innerHTML='<div id=\"a\"><span>x</span></div>';\
             var c=t.cloneNode(false),f=c.content;\
             f.childNodes.length+'|'+(f.firstChild?f.firstChild.childNodes.length:'-')"
        ),
        "1|0"
    );
}

/// (b) `document.importNode(t, true)` — même règle (même rail).
#[test]
fn template_import_node_deep_copies_content() {
    assert_eq!(
        eval_dom(
            "var t=document.createElement('template');\
             t.innerHTML='<div id=\"a\">x</div>';\
             var c=document.importNode(t,true);\
             c.content.querySelector('#a')?'found':'miss'"
        ),
        "found"
    );
}

/// (b) `<template>` imbriqué dans `content` : son propre
/// `content` suit aussi (spec — copie inconditionnelle).
#[test]
fn nested_template_content_follows() {
    assert_eq!(
        eval_dom(
            "var t=document.createElement('template');\
             t.innerHTML='<template id=\"in\"><i>z</i></template>';\
             var c=t.cloneNode(true),inner=c.content.querySelector('template');\
             inner&&inner.content&&inner.content.childNodes.length"
        ),
        "1"
    );
}

/// (a) `var x` sur global où `x` est un accesseur propre :
/// spec `CreateGlobalVarBinding` — accesseur **non-configurable**
/// → no-op (l'accesseur survit, motif `IqL` kevlar : `{get,set}`
/// sans `configurable`) ; accesseur **configurable** → redéfini
/// en data prop (`undefined`, ex. `var top` écrase `window.top`).
/// Deux evals : dans un même script le `var` serait hoisté
/// **avant** le `defineProperty`.
#[test]
fn var_on_global_keeps_existing_accessor() {
    let mut e = Engine::new();
    e.eval("Object.defineProperty(globalThis,'__vx',{get:function(){return 42}})")
        .expect("define non-config accessor");
    let v = e.eval("var __vx;__vx").expect("var redecl non-config");
    assert_eq!(e.describe(v), "42");

    e.eval("Object.defineProperty(globalThis,'__vy',{get:function(){return 7},configurable:!0})")
        .expect("define config accessor");
    let v = e.eval("var __vy;__vy").expect("var redecl config");
    assert_eq!(e.describe(v), "undefined");
}

/// (c) Gate `ShadyDOM` — `w.Sb` = `Element.prototype.attachShadow`
/// && `Node.prototype.getRootNode` : si l'une manque à l'éval du
/// polyfill, `ShadyDOM.inUse=true` → `disconnectedCallback`
/// piloté par le polyfill → mur `playlistHandlerActionMap`.
/// `WebIDL` : `Element.prototype` hérite `Node.prototype`,
/// `HTMLElement.prototype` hérite `Element.prototype`.
#[test]
fn element_proto_chains_to_node() {
    assert_eq!(
        eval_dom(
            "[typeof Element.prototype.attachShadow,\
             typeof Node.prototype.getRootNode,\
             typeof HTMLElement.prototype.attachShadow,\
             Object.getPrototypeOf(Element.prototype)===Node.prototype,\
             Object.getPrototypeOf(HTMLElement.prototype)===Element.prototype].join('|')"
        ),
        "function|function|function|true|true"
    );
}
