// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! §B-213 — `TypeError: 'getPropertyPriority' is not a
//! function (on CSSStyleDeclaration)` — nintendo
//! `main-775e5101cd8a7884.js:18:79004` (vols 969/973,
//! démasqué par §B-210). Surface CSSOM de
//! `CSSStyleDeclaration` : `getPropertyPriority(k)`
//! → `"important"`/`""`, 3ᵉ arg `priority` de
//! `setProperty` (`"important"` stocke le suffixe ;
//! toute autre valeur non vide = no-op spec), les
//! accesseurs (`getPropertyValue`, getter nommé,
//! `removeProperty`) servent la valeur SANS le suffixe
//! de priorité. `removeProperty` retourne l'ancienne
//! valeur (spec — rendait `""`).

use explorer_jscore::{Dom, Engine};
use std::cell::RefCell;
use std::rc::Rc;

const PAGE: &str = r#"<html><body>
<div id="m" style="color: red !important; width: 10px"></div>
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
fn getpropertypriority_et_accesseurs_sans_suffixe() {
    let s = "var s=document.getElementById('m').style;";
    // Verbatim : la méthode existe (n'est pas masquée par
    // le named-getter CSS).
    assert_eq!(ev(&format!("{s} typeof s.getPropertyPriority")), "function");
    // Spec : `"important"` si suffixe stocké, `""` sinon.
    assert_eq!(
        ev(&format!("{s} s.getPropertyPriority('color')")),
        "important"
    );
    assert_eq!(ev(&format!("{s} s.getPropertyPriority('width')")), "");
    assert_eq!(ev(&format!("{s} s.getPropertyPriority('absent')")), "");
    // Les accesseurs excluent le suffixe de priorité.
    assert_eq!(ev(&format!("{s} s.getPropertyValue('color')")), "red");
    assert_eq!(ev(&format!("{s} s.color")), "red");
    // `cssText` conserve la déclaration brute.
    assert_eq!(
        ev(&format!("{s} s.cssText")),
        "color: red !important; width: 10px"
    );
}

#[test]
fn setproperty_troisieme_arg_priority() {
    let s = "var s=document.getElementById('m').style;";
    // `"important"` → suffixe stocké, lu par les accesseurs.
    assert_eq!(
        ev(&format!(
            "{s} s.setProperty('height','20px','important'); \
             s.getPropertyPriority('height')+'/'+s.getPropertyValue('height')"
        )),
        "important/20px"
    );
    // Case-insensitive (spec CSSOM).
    assert_eq!(
        ev(&format!(
            "{s} s.setProperty('top','1px','IMPORTANT'); s.getPropertyPriority('top')"
        )),
        "important"
    );
    // Priorité invalide → no-op spec (déclaration non écrite).
    assert_eq!(
        ev(&format!(
            "{s} s.setProperty('left','2px','bogus'); \
             s.getPropertyPriority('left')+'/'+s.getPropertyValue('left')"
        )),
        "/"
    );
    // `priority` vide/absent → déclaration ordinaire.
    assert_eq!(
        ev(&format!(
            "{s} s.setProperty('right','3px'); s.getPropertyPriority('right')"
        )),
        ""
    );
    // `removeProperty` retourne l'ancienne valeur sans
    // priorité (spec).
    assert_eq!(ev(&format!("{s} s.removeProperty('color')")), "red");
    assert_eq!(ev(&format!("{s} s.removeProperty('absent')")), "");
}
