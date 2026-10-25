// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Régressions JS-C99 — sous-ensemble sélecteur du miroir :
//! `:not()`/`:is()`/`:where()`, `:defined`, pseudo-classes
//! structurelles et d'état, listes `a, b`. Mesuré terrain :
//! `shoelace-autoloader` `querySelectorAll(":not(:defined)")`
//! rendait `[]` silencieusement → `customElements.define`
//! jamais appelé → chaîne Shadow morte (vol 2026-11-10).

use explorer_jscore::{Dom, Engine};
use std::cell::RefCell;
use std::rc::Rc;

fn engine() -> Engine {
    let mut e = Engine::new();
    e.install_dom(&Rc::new(RefCell::new(Dom::from_html(
        "<!doctype html><html><body>\
         <div id=a class=x><p class=y>1</p><p>2</p><span>3</span></div>\
         <div id=b><sl-button>ok</sl-button><sl-icon></sl-icon><em>z</em></div>\
         <input id=c type=checkbox checked required>\
         <input id=d type=text readonly placeholder='ph'>\
         <input id=e type=text disabled>\
         <details id=f open></details><details id=g></details>\
         <a id=h href='/x'></a><p id=i hidden></p>\
         <option id=j selected></option><progress id=k></progress>\
         </body></html>",
    ))));
    e
}

fn ev(src: &str) -> String {
    let mut e = engine();
    match e.eval(src) {
        Ok(v) => e.describe(v),
        Err(err) => panic!("{src} -> {err}"),
    }
}

fn qsa(sel: &str) -> String {
    ev(&format!(
        "Array.from(document.querySelectorAll('{sel}')).map(function(n){{return n.id||n.tagName;}}).join(',')"
    ))
}

fn qsa_on(el: &str, sel: &str) -> String {
    ev(&format!(
        "Array.from({el}.querySelectorAll('{sel}')).map(function(n){{return n.id||n.tagName;}}).join(',')"
    ))
}

/// `:not(compound)` — la base mesurée : `querySelectorAll(
/// ':not(:defined)')` doit trouver les custom elements non
/// définis (Shoelace autoloader).
#[test]
fn c99_not_basic() {
    // `*:not(#a)` — descendants de body sauf #a (les enfants
    // de #a — P,P,SPAN — précèdent #b en ordre document).
    assert_eq!(
        qsa_on("document.getElementById('a').parentNode", "*:not(#a)"),
        "P,P,SPAN,b,SL-BUTTON,SL-ICON,EM,c,d,e,f,g,h,i,j,k"
    );
    // `div:not(.x)` — div#b seul (#a a la classe x).
    assert_eq!(qsa("div:not(.x)"), "b");
    // `:not(:defined)` — LE cas mesuré : éléments à `-`
    // non définis.
    assert_eq!(
        qsa_on("document.body", "*:not(:defined)"),
        "SL-BUTTON,SL-ICON"
    );
    // Après `customElements.define` — sl-button devient defined.
    assert_eq!(
        ev("customElements.define('sl-button', class extends HTMLElement {}); Array.from(document.querySelectorAll('*:not(:defined)')).map(function(n){return n.tagName;}).join(',')"),
        "SL-ICON"
    );
}

/// `:defined` — builtin toujours defined, CE seulement après
/// `define`.
#[test]
fn c99_defined() {
    assert_eq!(
        qsa_on("document.body", "*:defined"),
        "a,P,P,SPAN,b,EM,c,d,e,f,g,h,i,j,k"
    );
    // `div:defined` = `div` (builtin).
    assert_eq!(qsa("div:defined"), "a,b");
    // CE défini → `:defined`.
    assert_eq!(
        ev("customElements.define('sl-button', class extends HTMLElement {}); Array.from(document.querySelectorAll('*:defined')).filter(function(n){return n.tagName.includes('-');}).map(function(n){return n.tagName;}).join(',')"),
        "SL-BUTTON"
    );
}

/// `:is()`/`:where()` — alternative de compounds.
#[test]
fn c99_is_where() {
    assert_eq!(qsa_on("document.body", "*:is(span, em)"), "SPAN,EM");
    assert_eq!(qsa_on("document.body", "*:where(#h, #i)"), "h,i");
    // `:not(:not(.y))` — double négation = .y.
    assert_eq!(
        qsa_on("document.getElementById('a')", "p:not(:not(.y))"),
        "P"
    );
}

/// Pseudo-classes structurelles — `:first-child`/`:last-child`/
/// `:only-child`/`:nth-*`/`:empty`/`:root`.
#[test]
fn c99_structural() {
    // `:first-child` des `p` dans #a.
    assert_eq!(qsa_on("document.getElementById('a')", "p:first-child"), "P");
    // `:last-child` — span est le dernier enfant de #a.
    assert_eq!(
        qsa_on("document.getElementById('a')", "span:last-child"),
        "SPAN"
    );
    // `:nth-child(2)` — 2e enfant de #a = le 2e p.
    assert_eq!(
        qsa_on("document.getElementById('a')", "*:nth-child(2)"),
        "P"
    );
    // `:nth-last-child(1)` = `:last-child`.
    assert_eq!(
        qsa_on("document.getElementById('a')", "*:nth-last-child(1)"),
        "SPAN"
    );
    // `:nth-of-type(2)` — 2e `p` de #a.
    assert_eq!(
        qsa_on("document.getElementById('a')", "p:nth-of-type(2)"),
        "P"
    );
    // `:empty` — les vides (sl-icon, inputs, details, a, p,
    // option, progress — pas de texte ni d'enfants).
    assert_eq!(
        qsa_on("document.body", "*:empty"),
        "SL-ICON,c,d,e,f,g,h,i,j,k"
    );
    // `:root` — l'élément html.
    assert_eq!(
        ev("document.querySelector(':root') === document.documentElement"),
        "true"
    );
    // `:nth-child(odd)` — positions 1,3 de #a = p.y, span.
    assert_eq!(
        qsa_on("document.getElementById('a')", "*:nth-child(odd)"),
        "P,SPAN"
    );
}

/// Pseudo-classes d'état — `:checked`, `:disabled`/`:enabled`,
/// `:required`/`:optional`, `:open`/`:closed`, `:link`,
/// `:read-only`/`:read-write`, `:placeholder-shown`,
/// `:indeterminate`, états jamais vrais (`:visited`, `:hover`).
#[test]
fn c99_state() {
    assert_eq!(qsa("input:checked"), "c");
    assert_eq!(qsa("option:checked"), "j");
    assert_eq!(qsa("input:disabled"), "e");
    assert_eq!(qsa_on("document.body", "input:enabled"), "c,d");
    assert_eq!(qsa("input:required"), "c");
    assert_eq!(qsa_on("document.body", "input:optional"), "d,e");
    assert_eq!(qsa("details:open"), "f");
    assert_eq!(qsa("details:closed"), "g");
    assert_eq!(qsa("a:link"), "h");
    assert_eq!(qsa("input:read-only"), "d,e");
    assert_eq!(qsa("input:read-write"), "c");
    assert_eq!(qsa("input:placeholder-shown"), "d");
    assert_eq!(qsa("progress:indeterminate"), "k");
    // `:visited`/`:hover` — jamais exposés (honnêteté).
    assert_eq!(qsa("a:visited"), "");
    assert_eq!(qsa("a:hover"), "");
    // `:focus` — rien de focalisé.
    assert_eq!(qsa("*:focus"), "");
    // Après `focus()` — le nœud devient `:focus`.
    assert_eq!(
        ev("document.getElementById('d').focus(); document.querySelector('*:focus').id"),
        "d"
    );
    // `:focus-within` — body contient le focus.
    assert_eq!(
        ev("document.getElementById('d').focus(); document.querySelector('body:focus-within') === document.body"),
        "true"
    );
}

/// Listes `a, b` — union en ordre document, dédoublonnée.
#[test]
fn c99_selector_list() {
    assert_eq!(qsa("span, em"), "SPAN,EM");
    // Dédoublonné : `em` matché deux fois une fois rendu.
    assert_eq!(qsa("em, em"), "EM");
    // Ordre document (pas ordre des items) : `em, span` rend
    // `span` (dans #a) avant `em` (dans #b).
    assert_eq!(qsa_on("document.body", "em, span"), "SPAN,EM");
    // Liste avec `:not` et compound.
    assert_eq!(
        qsa_on("document.body", "sl-button, :not(:defined)"),
        "SL-BUTTON,SL-ICON"
    );
}

/// `matches`/`closest` — mêmes sous-ensembles (`:not`, listes,
/// `:defined`).
#[test]
fn c99_matches_closest() {
    assert_eq!(
        ev("var s=document.querySelector('sl-icon'); [s.matches(':not(:defined)'), s.matches('sl-icon'), s.matches('sl-button, sl-icon')].join(',')"),
        "true,true,true"
    );
    assert_eq!(
        ev("var s=document.querySelector('sl-icon'); customElements.define('sl-icon', class extends HTMLElement {}); s.matches(':defined')"),
        "true"
    );
    assert_eq!(
        ev("var p=document.getElementById('a').querySelector('span'); p.closest('#b, #a').id"),
        "a"
    );
    // `closest` avec compound à descendants.
    assert_eq!(
        ev("var p=document.getElementById('a').querySelector('span'); p.closest('div#a') === document.getElementById('a')"),
        "true"
    );
}

/// Le chemin Shoelace exact : `discover(root)` →
/// `qSA(':not(:defined)')` → `define` chaque nom →
/// plus rien n'est `:not(:defined)` — la chaîne complète
/// doit vivre (la régression mesurée rendait `[]` à la
/// première étape, `define` jamais atteint).
#[test]
fn c99_shoelace_discover() {
    assert_eq!(
        ev(
            "var toRegister = Array.from(document.querySelectorAll(':not(:defined)'))\
             .map(function(el){ return el.tagName.toLowerCase(); });\
             var names = toRegister.join(',');\
             names.split(',').forEach(function(name){\
               customElements.define(name, class extends HTMLElement {});\
             });\
             Array.from(document.querySelectorAll(':not(:defined)')).length + '|' + names"
        ),
        "0|sl-button,sl-icon"
    );
}

/// JS-C102 — `el.constructor` sur un élément **upgradé CE**
/// rend le ctor enregistré, pas l'interface `HTMLElement` —
/// sinon `this.constructor.elementProperties` dans le corps
/// `LitElement` manque → `ce.failed` → `shadowRoot` jamais créé
/// (verrou Shoelace mesuré).
#[test]
fn c102_upgraded_element_constructor() {
    // `define` + createElement → l'upgrade lie le proto ;
    // `el.constructor` = le ctor CE.
    assert_eq!(
        ev("var C = class extends HTMLElement {};\
            customElements.define('x-ctor', C);\
            var el = document.createElement('x-ctor');\
            el.constructor === C"),
        "true"
    );
    // Les éléments non-CE gardent le ctor d'interface.
    assert_eq!(
        ev("document.createElement('div').constructor.name"),
        "HTMLElement"
    );
    // `this.constructor.elementProperties` dans le corps ctor
    // résout via la chaîne CE → statique héritée (verrou
    // LitElement mesuré).
    assert_eq!(
        ev("var C3 = class extends HTMLElement {\
             static elementProperties = new Map([['a',1]]);\
             constructor(){ super(); globalThis.__ep2 = this.constructor.elementProperties.get('a'); } };\
            customElements.define('x-chain', C3);\
            document.createElement('x-chain');\
            globalThis.__ep2"),
        "1"
    );
}
