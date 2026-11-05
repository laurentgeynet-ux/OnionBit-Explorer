// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! JS-C355 — le rail `EventTarget` (`EventTarget.prototype.
//! addEventListener` & co) route les cibles DOM vers
//! `node.listeners` / `propagate_event` au lieu de la map `__et`
//! (mur §B-97 : les patches `addEventListener` de `ShadyDOM`
//! forwardent via la native sauvegardée — `…prototype.
//! addEventListener.call(el)` — les listeners d'orchestration
//! `yt-*` tombaient dans `__et`, morts-nés pour la propagation).

use explorer_jscore::{Dom, Engine};
use std::cell::RefCell;
use std::rc::Rc;

const PAGE: &str = r#"<html><body><div id="host"><i id="kid"></i></div></body></html>"#;

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

/// Greffe verbatim `ShadyDOM` : la native sauvegardée invoquée
/// via `.call(el)` doit servir la vraie propagation.
#[test]
fn eventtarget_rail_reaches_dom_listeners() {
    let (mut e, _dom) = engine();
    assert_eq!(
        ev(
            &mut e,
            "var el = document.getElementById('host');\
             var hits = [];\
             var saved = EventTarget.prototype.addEventListener;\
             saved.call(el, 'yt-test', function(ev){ hits.push(ev.type + '@' + ev.target.tagName); });\
             el.dispatchEvent(new Event('yt-test', {bubbles:true}));\
             hits.join('|')"
        ),
        "yt-test@DIV"
    );
}

/// La bulle traverse bien depuis l'enfant vers le nœud greffé.
#[test]
fn eventtarget_rail_bubbles_from_child() {
    let (mut e, _dom) = engine();
    assert_eq!(
        ev(
            &mut e,
            "var host = document.getElementById('host');\
             var kid = document.getElementById('kid');\
             var seen = '';\
             EventTarget.prototype.addEventListener.call(host, 'yt-bub', function(){ seen = 'hit'; });\
             kid.dispatchEvent(new Event('yt-bub', {bubbles:true}));\
             seen"
        ),
        "hit"
    );
}

/// `removeEventListener` via la même greffe retire le listener.
#[test]
fn eventtarget_rail_remove() {
    let (mut e, _dom) = engine();
    assert_eq!(
        ev(
            &mut e,
            "var el = document.getElementById('host');\
             var n = 0;\
             var cb = function(){ n++; };\
             var p = EventTarget.prototype;\
             p.addEventListener.call(el, 'yt-rm', cb);\
             p.removeEventListener.call(el, 'yt-rm', cb);\
             el.dispatchEvent(new Event('yt-rm'));\
             n"
        ),
        "0"
    );
}

/// `dispatchEvent` via la greffe sert les listeners du nœud
/// (chemin direct `node.listeners`) et rend `!defaultPrevented`.
#[test]
fn eventtarget_rail_dispatch_serves_node_listeners() {
    let (mut e, _dom) = engine();
    assert_eq!(
        ev(
            &mut e,
            "var el = document.getElementById('host');\
             var n = 0;\
             el.addEventListener('yt-dsp', function(ev){ n++; ev.preventDefault(); });\
             var ok = EventTarget.prototype.dispatchEvent.call(el, new Event('yt-dsp', {cancelable:true}));\
             n + '|' + ok"
        ),
        "1|false"
    );
}

/// `document`/`window` via la greffe ≙ racine — les listeners
/// attachés ainsi reçoivent les events bullés.
#[test]
fn eventtarget_rail_document_and_window() {
    let (mut e, _dom) = engine();
    assert_eq!(
        ev(
            &mut e,
            "var host = document.getElementById('host');\
             var d = '', w = '';\
             EventTarget.prototype.addEventListener.call(document, 'yt-root', function(){ d = 'doc'; });\
             EventTarget.prototype.addEventListener.call(window, 'yt-root', function(){ w = 'win'; });\
             host.dispatchEvent(new Event('yt-root', {bubbles:true}));\
             d + '|' + w"
        ),
        "doc|win"
    );
}
