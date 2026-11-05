// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! JS-C341 — les trampolines de méthodes hôte cèdent le pas aux
//! props scriptées sur l'objet ou sa chaîne de prototypes (mur
//! §B-97 du vol 644 : `ShadyDOM` pose `ShadowRoot.prototype.
//! appendChild` au chargement — `Host::get` court-circuitait la
//! chaîne proto et le natif primait le patch auteur, mort du
//! montage `ytd-*`).

use explorer_jscore::{Dom, Engine};
use std::cell::RefCell;
use std::rc::Rc;

const PAGE: &str = r#"<html><body><div id="host"></div><span id="s"></span><div id="plain"><i id="kid"></i></div></body></html>"#;

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

/// Scénario `ShadyDOM` verbatim : méthode posée sur
/// `ShadowRoot.prototype` → invoquée à la place du natif.
#[test]
fn shadow_root_proto_method_override_wins() {
    let (mut e, _dom) = engine();
    assert_eq!(
        ev(
            &mut e,
            "var host = document.getElementById('host');\
             var sr = host.attachShadow({mode:'open'});\
             var calls = 0;\
             ShadowRoot.prototype.appendChild = function(n){ calls++; return 'patched:' + n.tagName; };\
             var r = sr.appendChild(document.createElement('u'));\
             calls + '|' + r"
        ),
        "1|patched:U"
    );
    // Le `set` dérivé prime aussi via accesseur-prototype :
    // patch lu sur la chaîne, pas le trampoline hôte.
    assert_eq!(
        ev(
            &mut e,
            "typeof sr.appendChild + '|' + String(sr.appendChild(document.createElement('b')))"
        ),
        "function|patched:B"
    );
}

/// `sr.appendChild(sr)` — le patch rend sa valeur, **pas**
/// l'exception native `HierarchyRequestError` (un cycle
/// racine→racine est refusé par le natif, accepté par le shim).
#[test]
fn overridden_append_child_returns_patch_result_not_native_error() {
    let (mut e, _dom) = engine();
    assert_eq!(
        ev(
            &mut e,
            "var host = document.getElementById('host');\
             var sr = host.attachShadow({mode:'open'});\
             ShadowRoot.prototype.appendChild = function(n){ return n === this ? 'self-ok' : 'other'; };\
             var r = sr.appendChild(sr); String(r)"
        ),
        "self-ok"
    );
}

/// Le natif reste le natif hors patch : `div.appendChild(span)`
/// mute le miroir, et `Node.prototype.appendChild.call(div,
/// span)` (voie trampoline prototype) reste natif — les patchs
/// auteur ne dérivent pas la voie standard.
#[test]
fn native_append_child_stays_native_on_unpatched_protos() {
    let (mut e, _dom) = engine();
    // Patch posé sur `ShadowRoot.prototype` seulement.
    ev(
        &mut e,
        "var host = document.getElementById('host');\
         host.attachShadow({mode:'open'});\
         ShadowRoot.prototype.appendChild = function(){ return 'patched'; };'ok'",
    );
    assert_eq!(
        ev(
            &mut e,
            "var d = document.getElementById('plain');\
             var s = document.getElementById('s');\
             d.appendChild(s);\
             d.contains(s) + '|' + d.lastChild.id"
        ),
        "true|s"
    );
    // `Node.prototype.appendChild.call(div, span)` — la voie
    // native prototype (trampolines posés en props own) reste
    // fonctionnelle et dispatchée sur le receveur.
    assert_eq!(
        ev(
            &mut e,
            "var d = document.getElementById('plain');\
             var s2 = document.createElement('x');\
             Node.prototype.appendChild.call(d, s2);\
             d.lastChild.tagName"
        ),
        "X"
    );
}

/// Méthode posée en **own prop** sur l'élément — `el.m = f`
/// prime le natif (own script = shadow le plus fort).
#[test]
fn own_prop_method_shadows_native() {
    let (mut e, _dom) = engine();
    assert_eq!(
        ev(
            &mut e,
            "var d = document.getElementById('plain');\
             d.appendChild = function(){ return 'own'; };\
             d.appendChild(document.createElement('u'))"
        ),
        "own"
    );
}

/// `document` : une méthode `Document.prototype`/own scriptée
/// prime le trampoline document (même garde JS-C341).
#[test]
fn document_method_shadow_honored() {
    let (mut e, _dom) = engine();
    assert_eq!(
        ev(
            &mut e,
            "document.createElement = function(){ return 'patched-doc'; };\
             document.createElement('u')"
        ),
        "patched-doc"
    );
}

/// `has_method`/`has_prop` cohérents : patch retiré → le natif
/// resservi (pas de cache du verdict shadow).
#[test]
fn removing_patch_restores_native() {
    let (mut e, _dom) = engine();
    assert_eq!(
        ev(
            &mut e,
            "var host = document.getElementById('host');\
             var sr = host.attachShadow({mode:'open'});\
             ShadowRoot.prototype.appendChild = function(){ return 'patched'; };\
             var a = sr.appendChild(document.createElement('u'));\
             ShadowRoot.prototype.appendChild = Node.prototype.appendChild;\
             var b = sr.appendChild(document.createElement('i'));\
             [a, b && b.tagName].join('|')"
        ),
        "patched|I"
    );
}
