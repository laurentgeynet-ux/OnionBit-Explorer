// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! §B-122(f) — une `<iframe>` insérée dans une racine shadow
//! d'un hôte connecté est « connected » (spec « shadow-
//! including root ») → `note_attached_frame` l'enfile pour
//! navigation. Vol 757 explore.org : le lecteur
//! `youtube-video` encapsule son iframe en shadow — l'arbre
//! gardait `attached=false`, jamais enfilé, jamais de realm,
//! `contentWindow` nul → handshake `YT.Player` mort.
//! Le verrou couvre : iframe en shadow → drainée ; hôte
//! détaché/réattaché avec shadow peuplée → le drapeau suit.

use explorer_jscore::{Dom, Engine};
use std::cell::RefCell;
use std::rc::Rc;

const PAGE: &str = r#"<html><body><div id="host"></div></body></html>"#;

fn engine() -> Engine {
    let mut e = Engine::new();
    let dom = Rc::new(RefCell::new(Dom::from_html(PAGE)));
    e.install_dom(&dom);
    e
}

#[test]
fn shadow_iframe_enqueued_for_navigation() {
    let mut e = engine();
    e.eval(
        "var h = document.getElementById('host');\
         var sr = h.attachShadow({mode:'open'});\
         var f = document.createElement('iframe');\
         f.setAttribute('src', 'https://example.com/embed/xyz');\
         sr.appendChild(f);",
    )
    .expect("insert shadow iframe");
    let reqs = e.drain_dyn_frames();
    assert_eq!(reqs.len(), 1, "l'iframe shadow doit être enfilée");
    assert_eq!(
        reqs[0].src.as_deref(),
        Some("https://example.com/embed/xyz")
    );
}

#[test]
fn shadow_tree_follows_host_attachment() {
    let mut e = engine();
    // Shadow peuplée AVANT insertion de l'hôte → tout
    // l'arbre shadow devient connecté à l'attachement.
    e.eval(
        "var h = document.createElement('div');\
         var sr = h.attachShadow({mode:'open'});\
         var f = document.createElement('iframe');\
         f.setAttribute('src', 'https://example.com/late');\
         sr.appendChild(f);\
         document.body.appendChild(h);",
    )
    .expect("attach host after shadow populated");
    let reqs = e.drain_dyn_frames();
    assert_eq!(
        reqs.len(),
        1,
        "l'iframe shadow doit être enfilée à l'attachement de l'hôte"
    );
    assert_eq!(reqs[0].src.as_deref(), Some("https://example.com/late"));
}

#[test]
fn shadow_iframe_is_connected() {
    let mut e = engine();
    let got = e
        .eval(
            "var h = document.getElementById('host');\
             var sr = h.attachShadow({mode:'open'});\
             var f = document.createElement('iframe');\
             sr.appendChild(f);\
             f.isConnected",
        )
        .map(|v| e.describe(v))
        .expect("eval isConnected");
    assert_eq!(got, "true");
}
