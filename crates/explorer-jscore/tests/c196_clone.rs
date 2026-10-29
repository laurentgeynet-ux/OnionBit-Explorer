// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! JS-C196 — `postMessage`/`structuredClone` des cloneables
//! spec (lequipe vol 124 : `cannot clone host object`) :
//! `Blob`/`File` traversent le fil en vrais `ObjKind::Blob`
//! (avant : objet vide `{}` silencieux), `AbortSignal`
//! sérialise ses slots, et l'erreur nomme la classe
//! (`HTMLVideoElement`, pas `host object`).

use explorer_jscore::{Dom, Engine};
use std::cell::RefCell;
use std::rc::Rc;

const PAGE: &str = r#"<html><body><video id="v"></video><iframe></iframe></body></html>"#;

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

#[test]
fn c196_structured_clone_blob_roundtrip() {
    let mut e = engine();
    // `Blob` cloné = vrai Blob (type + octets), pas `{}`.
    assert_eq!(
        ev(
            &mut e,
            "var c=structuredClone(new Blob(['hello'],{type:'text/plain'}));\
             (c instanceof Blob)+'|'+c.size+'|'+c.type"
        ),
        "true|5|text/plain"
    );
    // `File` cloné conserve `name`/`lastModified` (own-props).
    assert_eq!(
        ev(
            &mut e,
            "var f=new File(['x'],'a.txt',{type:'text/plain',lastModified:7});\
             var cf=structuredClone(f);\
             cf.name+'|'+cf.lastModified+'|'+cf.size"
        ),
        "a.txt|7|1"
    );
}

#[test]
fn c196_post_message_carries_blob() {
    let mut e = engine();
    // `postMessage` wire : un `Blob` embarqué traverse en vrai
    // `ObjKind::Blob` — avant le rail le sortait en `{}`
    // silencieux. `parent` = soi au contexte racine : listener
    // `message` + pompe microtask (frontière de job spec).
    e.eval(
        "var got='none';\
         window.addEventListener('message',function(ev){\
           var d=ev.data;\
           got=(d.b instanceof Blob)+'|'+d.b.size+'|'+d.b.type+'|'+d.tag;\
         });\
         window.parent.postMessage({b:new Blob(['xy'],{type:'text/x'}),tag:'pm'},'*');",
    )
    .unwrap();
    e.run_microtasks().expect("microtasks");
    assert_eq!(ev(&mut e, "got"), "true|2|text/x|pm");
    // `structuredClone` profond : Blob niché dans une Map.
    assert_eq!(
        ev(
            &mut e,
            "var m=new Map([['k',new Blob(['q'])]]);\
             (structuredClone(m).get('k') instanceof Blob)"
        ),
        "true"
    );
}

#[test]
fn c196_abort_signal_is_serializable() {
    let mut e = engine();
    // Spec : `AbortSignal` sérialisable — slots `(aborted,
    // reason)` en objet ordinaire (proto perdu, documenté).
    assert_eq!(
        ev(
            &mut e,
            "var s=AbortSignal.abort('stop');\
             var c=structuredClone(s);\
             c.aborted+'|'+c.reason"
        ),
        "true|stop"
    );
}

#[test]
fn c196_uncloneable_host_error_names_class() {
    let mut e = engine();
    // Non cloneable → erreur nommant la classe (diagnostic vol
    // : `host object` ne disait rien sur le type fautif).
    let r = e.eval("structuredClone(document.getElementById('v'))");
    match r {
        Err(err) => {
            let s = format!("{err}");
            assert!(
                s.contains("Video") || s.contains("Element") || s.contains("clone"),
                "erreur attendue nommant la classe : {s}"
            );
        }
        Ok(v) => panic!("structuredClone(video) aurait dû jeter : {}", e.describe(v)),
    }
}

#[test]
fn c196_post_message_to_frame_still_works() {
    let mut e = engine();
    // Régression : le rail `postMessage` ordinaire reste intact
    // (message livré à la pompe, pas de clone raté).
    e.eval(
        "var ok='pending';\
         window.addEventListener('message',function(ev){ok=ev.data.tag});\
         window.parent.postMessage({tag:'pm'},'*');",
    )
    .unwrap();
    e.run_microtasks().expect("microtasks");
    assert_eq!(ev(&mut e, "ok"), "pm");
}
