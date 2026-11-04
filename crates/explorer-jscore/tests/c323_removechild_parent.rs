// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Verrous JS-C323 — mur terrain youtube.com (page **vidée**
//! post-bootstrap : miroir réduit à `html,body,script`, DOM
//! vivant `<body>` nu) : `removeChild`/`insertBefore`/`replaceChild`
//! détachaient le nœud passé **sans vérifier sa paternité** —
//! un `removeChild` sur un non-enfant arrachait quand même le
//! nœud de son vrai parent. Spec DOM `pre-remove`/`pre-insert`/
//! `replace` : enfant absent → `NotFoundError`, aucune mutation.
//! Kevlar appelait `el.removeChild(documentElement)` avec `el`
//! non parent (l'exception attendue rendait l'appel inerte chez
//! un navigateur) → notre moteur arrachait `<html>` → tout
//! mourait.

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

/// `el.removeChild(documentElement)` sur un non-parent →
/// `NotFoundError` et `<html>` reste attaché — le cas kevlar
/// verbatim (trace `[dom-kill] : call removeChild
/// this=[object HTMLElement]` → `detach tag=html`).
#[test]
fn remove_child_foreign_throws_keeps_tree() {
    assert_eq!(
        eval_dom(
            "var b=document.body,d=document.createElement('div');\
             b.appendChild(d);\
             var r='';try{d.removeChild(document.documentElement)}catch(x){r=x.name};\
             r+'|'+document.documentElement.tagName+'|'+(document.documentElement!=null)"
        ),
        "NotFoundError|HTML|true"
    );
}

/// `insertBefore(n, repèreÉtranger)` → `NotFoundError`, rien
/// n'est inséré.
#[test]
fn insert_before_foreign_ref_throws() {
    assert_eq!(
        eval_dom(
            "var b=document.body,d=document.createElement('div'),f=document.createElement('p');\
             var r='';try{b.insertBefore(d,f)}catch(x){r=x.name};\
             r+'|'+d.parentNode"
        ),
        "NotFoundError|null"
    );
}

/// `replaceChild(n, vieuxÉtranger)` → `NotFoundError`, `old`
/// reste attaché à son vrai parent.
#[test]
fn replace_child_foreign_old_throws() {
    assert_eq!(
        eval_dom(
            "var b=document.body,w=document.createElement('div'),o=document.createElement('span'),n=document.createElement('i');\
             w.appendChild(o);\
             var r='';try{b.replaceChild(n,o)}catch(x){r=x.name};\
             r+'|'+(o.parentNode===w)+'|'+(n.parentNode===null)"
        ),
        "NotFoundError|true|true"
    );
}

/// Contrôle positif — `document.removeChild(documentElement)`
/// est **légal** (documentElement est un vrai enfant du
/// document) : l'opération retire la racine, elle n'est pas
/// bloquée par la garde paternité.
#[test]
fn document_remove_child_root_legal() {
    assert_eq!(
        eval_dom(
            "var h=document.documentElement;\
             document.removeChild(h)+'|'+(document.documentElement==null)+'|'+(h.parentNode===null)"
        ),
        "[object HTMLElement]|true|true"
    );
}
