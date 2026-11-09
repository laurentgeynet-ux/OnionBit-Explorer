// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Verrou JS-C427 — `DOMRect` / `DOMRectReadOnly` sont
//! constructibles (spec : `new DOMRect(x,y,w,h)`), les bords
//! `right`/`bottom` sont dérivés, l'identité
//! prototype/constructor tient, et les rects produits par
//! `getBoundingClientRect` exposent le même proto.

use explorer_jscore::{Dom, Engine};
use std::cell::RefCell;
use std::rc::Rc;

fn eng() -> Engine {
    let mut e = Engine::new();
    e.install_dom(&Rc::new(RefCell::new(Dom::from_html(
        "<html><body><div id=t>x</div></body></html>",
    ))));
    e
}

fn eval_str(e: &mut Engine, src: &str) -> String {
    match e.eval(src) {
        Ok(v) => format!("{v:?}"),
        Err(err) => format!("ERR:{err:?}"),
    }
}

#[test]
fn c427_domrect_constructible() {
    let mut e = eng();
    let r = eval_str(
        &mut e,
        "var r = new DOMRect(10, 20, 100, 200);\
         [r.x, r.y, r.width, r.height, r.top, r.left, r.bottom, r.right].join(',')",
    );
    assert_eq!(r, "Str(\"10,20,100,200,20,10,220,110\")", "got {r}");
}

#[test]
fn c427_domrect_defaut_zero() {
    let mut e = eng();
    let r = eval_str(
        &mut e,
        "var r = new DOMRect();\
         [r.x, r.y, r.width, r.height].join(',')",
    );
    assert_eq!(r, "Str(\"0,0,0,0\")", "got {r}");
}

#[test]
fn c427_domrect_readonly_constructible_et_figee() {
    let mut e = eng();
    let r = eval_str(
        &mut e,
        "var r = new DOMRectReadOnly(1, 2, 30, 40);\
         var avant = [r.x, r.width, r.right, r.bottom].join(',');\
         r.x = 99; r.width = 77;\
         var apres = [r.x, r.width, r.right].join(',');\
         avant + '|' + apres",
    );
    assert_eq!(r, "Str(\"1,30,31,42|1,30,31\")", "got {r}");
}

#[test]
fn c427_identite_proto_et_ctor() {
    let mut e = eng();
    let r = eval_str(
        &mut e,
        "var r = new DOMRect(0,0,1,1);\
         var rr = new DOMRectReadOnly(0,0,1,1);\
         [\
           r instanceof DOMRect,\
           r instanceof DOMRectReadOnly,\
           rr instanceof DOMRectReadOnly,\
           !(rr instanceof DOMRect),\
           r.constructor === DOMRect,\
           rr.constructor === DOMRectReadOnly,\
           typeof DOMRect.fromRect,\
           DOMRect.fromRect({x:5, y:6, width:7, height:8}).right === 12,\
           DOMRectReadOnly.fromRect({left:1, top:2, right:11, bottom:12}).width === 10,\
           (function(){ try { DOMRect(); return 'no-throw'; } catch(x) { return 'threw'; } })()\
         ].join(',')",
    );
    assert_eq!(
        r, "Str(\"true,true,true,true,true,true,function,true,true,threw\")",
        "got {r}"
    );
}

#[test]
fn c427_gbrect_expose_le_proto() {
    let mut e = eng();
    let r = eval_str(
        &mut e,
        "var g = document.getElementById('t').getBoundingClientRect();\
         [\
           g instanceof DOMRect,\
           typeof g.x === 'number',\
           typeof g.width === 'number',\
           typeof g.right === 'number',\
           typeof g.bottom === 'number'\
         ].join(',')",
    );
    assert_eq!(r, "Str(\"true,true,true,true,true\")", "got {r}");
}
