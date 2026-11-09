// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! JS-C446 (JT-1) — §B-104 côté messages : la livraison se
//! fait sous le realm **destinataire**, pas celui du
//! posteur. Un `MessagePort` atteint via un global
//! same-origin d'un autre realm doit décoder avec les
//! intrinsics de son handler (`instanceof` honnête) —
//! `PortMsg.ctx` = realm de naissance du port, et
//! `drain_pending_msgs` refuse de livrer quand le switch
//! a échoué (garde `msg.drop_ctx`, parité `port.drop_ctx`).

use explorer_jscore::{Dom, Engine};
use std::cell::RefCell;
use std::rc::Rc;

const TOP_HTML: &str = r#"<html><body>
<iframe id="f" src="about:blank"></iframe>
<div id="t">top</div>
</body></html>"#;

const CHILD_HTML: &str = r#"<html><head><title>child doc</title></head><body>
<p id="c">child</p>
</body></html>"#;

const TOP_URL: &str = "https://top.example/page";
const CHILD_URL: &str = "https://top.example/frame";

fn top_engine() -> (Engine, Rc<RefCell<Dom>>) {
    let mut e = Engine::new();
    let d = Rc::new(RefCell::new(Dom::from_html(TOP_HTML)));
    e.install_dom(&d);
    e.set_page_url(TOP_URL);
    (e, d)
}

fn iframe_node(d: &Rc<RefCell<Dom>>) -> u32 {
    u32::try_from(
        d.borrow()
            .nodes
            .iter()
            .position(|n| n.tag == "iframe")
            .expect("TOP_HTML contient un <iframe>"),
    )
    .expect("index <iframe> tient en u32")
}

/// Crée un contexte enfant (courant après `context_new`), y
/// installe son propre DOM + URL, puis repasse le courant au
/// top-level. Renvoie le `CtxId` enfant.
fn add_child(e: &mut Engine, parent: &Rc<RefCell<Dom>>, url: &str) -> u32 {
    let node = iframe_node(parent);
    let id = e.context_new(url, Some((Rc::as_ptr(parent) as usize, node)));
    let cd = Rc::new(RefCell::new(Dom::from_html(CHILD_HTML)));
    e.install_dom(&cd);
    e.set_page_url(url);
    e.install_timers();
    e.context_switch(0);
    id
}

fn ev(e: &mut Engine, src: &str) -> String {
    match e.eval(src) {
        Ok(v) => e.describe(v),
        Err(err) => panic!("{src} -> {err}"),
    }
}

/// Évalue `src` dans le contexte `ctx` (switch → eval → restore).
fn ev_ctx(e: &mut Engine, ctx: u32, src: &str) -> String {
    e.context_switch(ctx);
    let out = ev(e, src);
    e.context_switch(0);
    out
}

/// Canal créé dans l'enfant (ctx1), posté depuis le parent
/// (ctx0) : les données doivent être décodées avec les
/// intrinsics de ctx1 — `ev.data instanceof Array` dans le
/// handler est le discriminant exact de la classe §B-104
/// (sans le fix : `ctx` = posteur → decode sous ctx0 →
/// `instanceof` faux dans un handler ctx1).
#[test]
fn c446_port_posted_from_foreign_realm_decodes_under_port_home() {
    let (mut e, d) = top_engine();
    let _child = add_child(&mut e, &d, CHILD_URL);
    // L'enfant crée le canal et son handler.
    ev_ctx(
        &mut e,
        1,
        "var ch = new MessageChannel();\n\
         window.__ok = '';\n\
         ch.port2.onmessage = function(ev) {\n\
             __ok = (ev.data instanceof Array) + '|' + ev.data[1];\n\
         };",
    );
    // Le parent poste sur port1 (atteint via le global
    // same-origin de l'enfant) — le message est routé vers
    // port2, realm de naissance ctx1.
    ev(
        &mut e,
        "var w = document.getElementById('f').contentWindow;\n\
         w.ch.port1.postMessage([10, 20]);",
    );
    e.run_microtasks().expect("pump");
    assert_eq!(ev_ctx(&mut e, 1, "__ok"), "true|20");
}

/// Même canal, message posté depuis le realm de naissance —
/// le comportement nominal reste inchangé (parité b4).
#[test]
fn c446_port_same_realm_delivery_unchanged() {
    let (mut e, d) = top_engine();
    let _child = add_child(&mut e, &d, CHILD_URL);
    ev_ctx(
        &mut e,
        1,
        "var ch = new MessageChannel();\n\
         window.__hit = 0;\n\
         ch.port1.onmessage = function(ev) { __hit += ev.data; };\n\
         ch.port2.postMessage(7);",
    );
    e.run_microtasks().expect("pump");
    assert_eq!(ev_ctx(&mut e, 1, "__hit"), "7");
}

/// `postMessage` fenêtre : livré sous le realm cible — le
/// handler lit les intrinsics de SON realm (`instanceof`
/// honnête sur la donnée clonée), parité avec le verrou
/// port ci-dessus.
#[test]
fn c446_window_post_delivers_under_target_realm() {
    let (mut e, d) = top_engine();
    let _child = add_child(&mut e, &d, CHILD_URL);
    ev_ctx(
        &mut e,
        1,
        "window.__ok = '';\n\
         window.addEventListener('message', function(ev) {\n\
             __ok = (ev.data instanceof Array) + '|' + ev.origin;\n\
         });",
    );
    ev(
        &mut e,
        "document.getElementById('f').contentWindow\n\
             .postMessage([1, 2], 'https://top.example');",
    );
    e.run_microtasks().expect("pump");
    assert_eq!(ev_ctx(&mut e, 1, "__ok"), "true|https://top.example");
}
