// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! JS-C449 (JT-4) — props chaîne `Document`/`type` calculé
//! des contrôles (mw wikimedia `x.toUpperCase` mourait sur
//! `Undefined`) : `compatMode`/`contentType`/`characterSet`
//!/`charset`/`inputEncoding` chaînes spec ; `textarea.type`
//! = "textarea", `select.type` = "select-one"/
//! "select-multiple" (spec calculée, pas reflet).

use explorer_jscore::{Dom, Engine};
use std::cell::RefCell;
use std::rc::Rc;

const PAGE: &str = "<html><body>
<textarea id=ta></textarea>
<select id=s1></select>
<select id=s2 multiple></select>
</body></html>";

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

/// Les props chaîne de `document` répondent des chaînes —
/// `.toUpperCase()` ne meurt plus.
#[test]
fn c449_document_string_props_chainable() {
    let mut e = engine();
    assert_eq!(
        ev(
            &mut e,
            "document.compatMode.toUpperCase()+'|'\
             +document.contentType.toUpperCase()+'|'\
             +document.characterSet+'|'\
             +document.charset+'|'\
             +document.inputEncoding"
        ),
        "CSS1COMPAT|TEXT/HTML|UTF-8|UTF-8|UTF-8"
    );
}

/// `type` calculé spec sur contrôles — `toUpperCase` direct.
#[test]
fn c449_control_type_computed() {
    let mut e = engine();
    assert_eq!(
        ev(
            &mut e,
            "document.getElementById('ta').type.toUpperCase()+'|'\
             +document.getElementById('s1').type+'|'\
             +document.getElementById('s2').type"
        ),
        "TEXTAREA|select-one|select-multiple"
    );
}

/// Le reflet `type` tag-restreint survit sur les tags
/// déclarés (`input`/`script`/`link`…) — la nouvelle branche
/// n'intercepte que `textarea`/`select`.
#[test]
fn c449_tagged_type_reflection_survives() {
    let mut e = engine();
    assert_eq!(
        ev(
            &mut e,
            "var i=document.createElement('input');i.type='checkbox';\
             var s=document.createElement('script');s.type='module';\
             i.type+'|'+s.type+'|'+(document.createElement('div').type===undefined)"
        ),
        "checkbox|module|true"
    );
}
