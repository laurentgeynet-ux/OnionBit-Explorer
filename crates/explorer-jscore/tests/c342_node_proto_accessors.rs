// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! JS-C342 — accesseurs `Prop::Acc` auteur sur la chaîne de
//! prototypes DOM + `el.data` restreint à `Text | Comment`
//! (murs §B-97 #3/#4 du vol 644 : les descripteurs `ShadyDOM`/
//! `Polymer` sur `ShadowRoot.prototype`, les protos de Custom
//! Elements et `Node.prototype` étaient court-circuités par la
//! voie hôte ; `el.data = {trackingParams:…}` sur
//! `<yt-formatted-string>` relisait `null`).

use explorer_jscore::{Dom, Engine};
use std::cell::RefCell;
use std::rc::Rc;

const PAGE: &str =
    r#"<html><body><div id="host"></div><span id="s"></span><div id="plain"></div></body></html>"#;

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

/// `el.data` sur un `Element` n'est **pas** `null` : la prop suit
/// la voie JS normale — own prop posée par le script, relue telle
/// quelle (le roundtrip `<yt-formatted-string>.data.trackingParams`).
/// `CharacterData.data` garde la spec ; `nodeValue` sur Element
/// reste `null`.
#[test]
fn element_data_property_roundtrips_on_html_and_custom_elements() {
    let mut e = engine();
    assert_eq!(
        ev(
            &mut e,
            "var el = document.createElement('yt-formatted-string');\
             el.data = { trackingParams: 'abc' };\
             el.data.trackingParams"
        ),
        "abc"
    );
    // `data` non posée sur un Element → `undefined` (absente, spec)
    // — plus le `null` fantôme.
    assert_eq!(
        ev(&mut e, "typeof document.getElementById('plain').data"),
        "undefined"
    );
    assert_eq!(ev(&mut e, "document.createTextNode('hi').data"), "hi");
    assert_eq!(ev(&mut e, "document.createComment('c').data"), "c");
    assert_eq!(
        ev(
            &mut e,
            "String(document.getElementById('plain').nodeValue) + '|' + document.createTextNode('t').nodeValue"
        ),
        "null|t"
    );
}

/// Setter/getter auteur posés sur un proto de la chaîne
/// (`Element.prototype` ici — même voie qu'un proto de Custom
/// Element ou `ShadowRoot.prototype`) : `el.title = 'x'` et
/// `el.data = {a:1}` déclenchent le descripteur, pas un expando
/// fantôme ni la voie hôte.
#[test]
fn custom_element_and_shadow_root_prototype_accessors_intercept_get_and_set() {
    let mut e = engine();
    assert_eq!(
        ev(
            &mut e,
            "var log = [];\
             Object.defineProperty(Element.prototype, 'cdata', {\
               get: function(){ return this._cd === undefined ? 'unset' : this._cd; },\
               set: function(v){ log.push('set'); this._cd = v * 2; }\
             });\
             var el = document.createElement('div');\
             var a = el.cdata;\
             el.cdata = 5;\
             [a, el.cdata, log.length].join('|')"
        ),
        "unset|10|1"
    );
    // `ShadowRoot.prototype` — le rail `ShadyDOM` verbatim.
    assert_eq!(
        ev(
            &mut e,
            "var host = document.getElementById('host');\
             var sr = host.attachShadow({mode:'open'});\
             Object.defineProperty(ShadowRoot.prototype, 'data', {\
               get: function(){ return 'shady:' + this.host.id; }\
             });\
             sr.data"
        ),
        "shady:host"
    );
    // `Node.prototype` lui-même — `insideDescriptors` de `ShadyDOM`
    // patche la base : l'accesseur auteur doit primer la voie hôte.
    assert_eq!(
        ev(
            &mut e,
            "Object.defineProperty(Node.prototype, 'firstChild', {\
               get: function(){ return 'patched-fc'; }\
             });\
             document.getElementById('plain').firstChild"
        ),
        "patched-fc"
    );
}

/// Les accesseurs **natifs** du moteur sur les protos de base
/// (JS-C340) ne shadowent pas : `el.firstChild` non patché sert
/// la valeur hôte — pas de rebouclage accesseur→hôte→accesseur.
#[test]
fn native_prototype_accessors_do_not_shadow_host_values() {
    let mut e = engine();
    assert_eq!(
        ev(
            &mut e,
            "var d = document.getElementById('plain');\
             var s = document.getElementById('s');\
             d.appendChild(s);\
             d.firstChild.id + '|' + d.firstElementChild.id"
        ),
        "s|s"
    );
}

/// Un `data` posé comme **prop donnée own** sur l'élément survit
/// à la lecture (ni getter de chaîne ni voie hôte ne le masquent).
#[test]
fn element_data_own_prop_survives_read() {
    let mut e = engine();
    assert_eq!(
        ev(
            &mut e,
            "var el = document.getElementById('plain');\
             el.data = 42;\
             el.hasOwnProperty('data') + '|' + el.data"
        ),
        "true|42"
    );
}
