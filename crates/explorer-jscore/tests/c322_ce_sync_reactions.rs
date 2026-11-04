// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Verrous JS-C322 — mur terrain `ytd-app` youtube
//! (`playlistHandlerActionMap of Undefined` @kevlar:40765,
//! poursuite §B-4(c)) : deux racines couplées.
//!
//! **(a) Ctor ES5 marqué `failed`** — `ce_upgrade_node` exigeait
//! le marqueur `FLD_DONE` que seul `super()` pose : les ctors
//! plain-function (`function(){return Y.apply(this,…)}` —
//! enrobage Polymer/kevlar) n'en disposent jamais → l'élément
//! était classé `failed` → `connectedCallback`/`disconnectedCallback`
//! désarmés. Spec : un ctor qui n'a pas rendu d'objet étranger
//! et n'a pas jeté a réussi (les ctors custom elements
//! enregistrés peuvent être des fonctions — `new` côté auteur
//! est interdit, l'upgrade non).
//!
//! **(b) Réactions CE drainées tard** — `h.invoke` appelé sans
//! `drain_ce_reactions` dans les rails `BoundNative hostMethod`
//! (`Engine::host_method`), trampoline lié
//! (`host_method_trampoline`) et méthodes `Node.prototype`
//! (`node_proto_method_trampoline` — le rail réel depuis
//! JS-C318 où `appendChild` vit en vraie prop proto) : la
//! réaction `Connected` restait en file après `appendChild` →
//! `connectedCallback` tirait au drain suivant → `isAttached`/
//! stamping/`Xcs` posés hors délai → ordre callbacks rompu
//! (un `detached` `ShadyDOM` lisait `Xcs` encore `undefined`).
//! Spec : les réactions custom elements sont invoquées *par*
//! l'opération qui les produit — synchrone.

use explorer_jscore::{Dom, Engine};
use std::cell::RefCell;
use std::rc::Rc;

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

/// (a)+(b) — un ctor **function** (forme ES5/Polymer) uppgrade
/// un élément créé par `createElement` puis inséré par
/// `appendChild` : `connectedCallback` doit avoir tourné
/// **avant le retour** de l'appel.
#[test]
fn es5_ctor_connects_synchronously() {
    assert_eq!(
        eval_dom(
            "var hits=[];\
             function XApp(){this.__ctor=1};\
             XApp.prototype.connectedCallback=function(){hits.push('cc')};\
             XApp.prototype.disconnectedCallback=function(){hits.push('dc')};\
             customElements.define('x-app',XApp);\
             var el=document.createElement('x-app');\
             document.body.appendChild(el);\
             hits.join(',')"
        ),
        "cc"
    );
}

/// Même rail via l'appel non lié `Node.prototype.appendChild` —
/// le trampoline proto est le chemin que prennent les shims.
#[test]
fn proto_method_drains_connected() {
    assert_eq!(
        eval_dom(
            "var hits=[];\
             function XApp2(){this.__ctor=1};\
             XApp2.prototype.connectedCallback=function(){hits.push('cc')};\
             customElements.define('x-app2',XApp2);\
             var el=document.createElement('x-app2');\
             var ap=Node.prototype.appendChild;\
             ap.call(document.body,el);\
             hits.join(',')+'|'+el.__ctor"
        ),
        "cc|1"
    );
}

/// Le retrait est symétrique : `disconnectedCallback` doit
/// avoir tourné avant le retour de `removeChild`.
#[test]
fn disconnect_is_synchronous() {
    assert_eq!(
        eval_dom(
            "var hits=[];\
             function XApp3(){this.__ctor=1};\
             XApp3.prototype.connectedCallback=function(){hits.push('cc')};\
             XApp3.prototype.disconnectedCallback=function(){hits.push('dc')};\
             customElements.define('x-app3',XApp3);\
             var el=document.createElement('x-app3');\
             document.body.appendChild(el);\
             document.body.removeChild(el);\
             hits.join(',')"
        ),
        "cc,dc"
    );
}

/// Un ctor qui rend un *autre* objet reste « failed » : ses
/// callbacks ne doivent pas tirer (rejet spec conservé).
#[test]
fn foreign_return_still_fails() {
    assert_eq!(
        eval_dom(
            "var hits=[];\
             function XApp4(){this.__ctor=1;return{alien:true}};\
             XApp4.prototype.connectedCallback=function(){hits.push('cc')};\
             customElements.define('x-app4',XApp4);\
             var el=document.createElement('x-app4');\
             document.body.appendChild(el);\
             hits.join(',')||'none'"
        ),
        "none"
    );
}
