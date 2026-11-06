// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! JS-C384 / §B-103 — reflets d'attributs **tag-scopés** :
//! la table plate historique rendait `'media' in div`,
//! `'srcset' in el`, `'checked' in div`… vrais sur tout
//! élément → la détection feature des bundles (kevlar)
//! prenait la mauvaise branche → return silencieux →
//! `.then` jamais enfilé — suspect mécanisme §B-97.
//! Spec IDL `reflect` : le reflet n'existe que sur les
//! classes qui le déclarent ; sur un mauvais tag, lecture
//! `undefined`, `in` → `false`, écriture → prop propre
//! (jamais d'attribut fantôme).

use explorer_jscore::{Dom, Engine};
use std::cell::RefCell;
use std::rc::Rc;

const PAGE: &str = r#"<html><body><div id="host"></div></body></html>"#;

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

/// Les reflets hors-tag rendent `false` au `in` (le poison
/// §B-103) et `undefined` en lecture — les reflets en-tag
/// restent vrais ; les globaux restent globaux.
#[test]
fn reflected_attrs_are_tag_scoped() {
    let mut e = engine();
    // Poison §B-103 — faux sur mauvais tag.
    for key in [
        "media",
        "srcset",
        "sizes",
        "srcdoc",
        "sandbox",
        "poster",
        "preload",
        "placeholder",
        "alt",
        "href",
        "src",
        "checked",
        "controls",
        "autoplay",
        "selected",
        "multiple",
        "readonly",
        "name",
        "value",
        "type",
        "rel",
        "target",
        "action",
        "method",
        "htmlFor",
        "crossOrigin",
        "playsInline",
        "referrerPolicy",
    ] {
        assert_eq!(
            ev(&mut e, &format!("'{key}' in document.createElement('div')")),
            "false",
            "'{key}' in div"
        );
    }
    // En-tag : vrais.
    for (tag, key) in [
        ("link", "media"),
        ("style", "media"),
        ("source", "media"),
        ("img", "srcset"),
        ("source", "srcset"),
        ("img", "sizes"),
        ("iframe", "srcdoc"),
        ("iframe", "sandbox"),
        ("video", "poster"),
        ("video", "preload"),
        ("audio", "preload"),
        ("video", "controls"),
        ("audio", "autoplay"),
        ("input", "placeholder"),
        ("textarea", "placeholder"),
        ("img", "alt"),
        ("input", "checked"),
        ("option", "selected"),
        ("select", "multiple"),
        ("input", "readonly"),
        ("a", "href"),
        ("link", "href"),
        ("img", "src"),
        ("script", "src"),
        ("form", "action"),
        ("form", "method"),
        ("a", "target"),
        ("a", "rel"),
        ("input", "name"),
        ("form", "name"),
        ("input", "value"),
        ("textarea", "value"),
        ("button", "type"),
        ("input", "type"),
        ("label", "htmlFor"),
        ("img", "crossOrigin"),
        ("video", "playsInline"),
        ("img", "referrerPolicy"),
        ("input", "disabled"),
        ("fieldset", "disabled"),
    ] {
        assert_eq!(
            ev(
                &mut e,
                &format!("'{key}' in document.createElement('{tag}')")
            ),
            "true",
            "'{key}' in {tag}"
        );
    }
    // Globaux — sur n'importe quel tag.
    for key in ["id", "className", "title", "lang", "dir", "slot"] {
        assert_eq!(
            ev(&mut e, &format!("'{key}' in document.createElement('div')")),
            "true",
            "'{key}' in div (global)"
        );
        assert_eq!(
            ev(
                &mut e,
                &format!("'{key}' in document.createElement('section')")
            ),
            "true",
            "'{key}' in section (global)"
        );
    }
}

/// Lecture hors-tag : `undefined`, pas `""` ; écriture
/// hors-tag : prop propre sans attribut fantôme ; écriture
/// en-tag : toujours le reflet d'attribut.
#[test]
fn tagged_reflect_read_write_semantics() {
    let mut e = engine();
    assert_eq!(
        ev(&mut e, "document.createElement('div').media"),
        "undefined"
    );
    assert_eq!(
        ev(&mut e, "document.createElement('div').poster"),
        "undefined"
    );
    // `div.poster = 'x'` → prop propre (relue), `getAttribute`
    // ne voit rien — spec : pas de reflet = pas d'attribut.
    assert_eq!(
        ev(
            &mut e,
            "var d = document.createElement('div'); d.poster = 'x'; d.getAttribute('poster')"
        ),
        "null"
    );
    assert_eq!(
        ev(
            &mut e,
            "var d2 = document.createElement('div'); d2.poster = 'x'; d2.poster"
        ),
        "x"
    );
    // En-tag : le reflet écrit l'attribut.
    assert_eq!(
        ev(
            &mut e,
            "var l = document.createElement('link'); l.media = 'print'; l.getAttribute('media')"
        ),
        "print"
    );
    assert_eq!(
        ev(
            &mut e,
            "var i = document.createElement('iframe'); i.sandbox = 'allow-scripts'; i.getAttribute('sandbox')"
        ),
        "allow-scripts"
    );
    // Booléen hors-tag : `undefined` ; en-tag : `false` (attr
    // absent) puis `true` après écriture.
    assert_eq!(
        ev(&mut e, "document.createElement('div').controls"),
        "undefined"
    );
    assert_eq!(
        ev(&mut e, "document.createElement('video').controls"),
        "false"
    );
    assert_eq!(
        ev(
            &mut e,
            "var v = document.createElement('video'); v.controls = true; v.controls"
        ),
        "true"
    );
}
