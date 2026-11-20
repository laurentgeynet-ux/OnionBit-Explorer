// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! §B-209 — `TypeError: instanceof requires an object` (web.whatsapp.com,
//! validation §B-206) : le bootloader Facebook teste
//! `e.dataset instanceof window.DOMStringMap` pour extraire les marqueurs
//! `data-bootloader-hash` ; `DOMStringMap` n'était pas installé en global
//! → RHS `undefined` → `instanceof` jette → l'observateur
//! `_btldrLoadedSrcs` mourait.
//! `WebIDL` : `DOMStringMap` est l'interface globale (non constructible)
//! de `el.dataset` ; les objets dataset chaînent `DOMStringMap.prototype`.
//! Verrous : ctor global installé, `dataset instanceof DOMStringMap`
//! réel, comportement live `data-*` conservé.

use explorer_jscore::{Dom, Engine};
use std::cell::RefCell;
use std::rc::Rc;

const PAGE: &str = r#"<html><body>
<div id="d" data-foo-bar="x" data-bootloader-hash="H123"></div>
</body></html>"#;

fn ev(src: &str) -> String {
    let mut e = Engine::new();
    e.install_dom(&Rc::new(RefCell::new(Dom::from_html(PAGE))));
    match e.eval(src) {
        Ok(v) => e.describe(v),
        Err(err) => panic!("{src} -> {err}"),
    }
}

#[test]
fn domstringmap_est_le_ctor_global_de_dataset() {
    // Interface globale installée (non constructible, spec WebIDL).
    assert_eq!(ev("typeof DOMStringMap"), "function");
    assert_eq!(ev("typeof window.DOMStringMap"), "function");
    // Le trou : `instanceof` jetait sur un RHS `undefined`.
    assert_eq!(
        ev("document.getElementById('d').dataset instanceof DOMStringMap"),
        "true"
    );
    assert_eq!(
        ev("document.getElementById('d').dataset instanceof window.DOMStringMap"),
        "true"
    );
    // Verbatim whatsapp `_btldrLoadedSrcs` — plus de TypeError,
    // la branche `? :` rend bien l'objet dataset.
    assert_eq!(
        ev("(function(e){return e.dataset instanceof window.DOMStringMap ? e.dataset : null})(document.getElementById('d')) instanceof DOMStringMap"),
        "true"
    );
    // `new DOMStringMap()` reste « illegal constructor » (spec).
    assert_eq!(
        ev("try{new DOMStringMap()}catch(x){x.name+':'+x.message}"),
        "TypeError:illegal constructor (DOMStringMap)"
    );
}

#[test]
fn dataset_live_data_attrs_conserves() {
    assert_eq!(
        ev("var d=document.getElementById('d'); d.dataset.fooBar"),
        "x"
    );
    // camelCase ↔ kebab-case + écriture live sur l'attribut.
    assert_eq!(
        ev("var d=document.getElementById('d'); d.dataset.newKey='v'; d.getAttribute('data-new-key')"),
        "v"
    );
    assert_eq!(
        ev("var d=document.getElementById('d'); delete d.dataset.fooBar; d.hasAttribute('data-foo-bar')"),
        "false"
    );
    // L'objet dataset garde son identité interface après mutation.
    assert_eq!(
        ev("var d=document.getElementById('d'); d.dataset.x=1; d.dataset instanceof DOMStringMap"),
        "true"
    );
}
