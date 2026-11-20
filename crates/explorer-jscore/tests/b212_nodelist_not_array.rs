// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! §B-212 — `TypeError: cannot read 'appendChild' of Null`
//! (netflix/playstation/larousse/sketchfab — `OneTrust`
//! `otBannerSdk`, vols 932/939/956/958/971) : le SDK
//! construit le centre de préférences dans un
//! `DocumentFragment` puis `m(sel).append(fragment.
//! querySelectorAll(…))`. Son mini-jQuery dispatche sur
//! `Array.isArray(e)` — un **vrai** `NodeList` passe au
//! branche `this.selector.appendChild(e[i])` ; notre
//! `querySelectorAll` rendait un `Array` réel → la branche
//! `document.querySelector(this.selector)` était prise
//! avec `this.selector` = un Element → `null` → crash.
//! Spec : `querySelectorAll` rend un `NodeList` statique —
//! `Array.isArray` → `false`, `instanceof NodeList`,
//! proto `NodeList.prototype`, surface iterable
//! (`forEach`/`keys`/`values`/`entries`/`@@iterator`) +
//! `item`/`length`/props indexées.

use explorer_jscore::{Dom, Engine};
use std::cell::RefCell;
use std::rc::Rc;

const PAGE: &str = r#"<html><body>
<div id="onetrust-consent-sdk">
  <div id="onetrust-pc-sdk"><div class="ot-cat-grp"><span class="x">a</span></div></div>
</div>
<div id="a"></div><div id="b"></div>
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
fn queryselectorall_rend_un_nodelist_pas_un_array() {
    // Le trou : `Array.isArray` renvoyait `true` → OneTrust
    // prenait la branche `document.querySelector(sel)`.
    assert_eq!(
        ev("Array.isArray(document.querySelectorAll('div'))"),
        "false"
    );
    assert_eq!(ev("Array.isArray(document.body.children)"), "false");
    assert_eq!(ev("Array.isArray(document.body.childNodes)"), "false");
    // Identité spec : proto `NodeList.prototype`+ ctor + tag.
    assert_eq!(
        ev("document.querySelectorAll('div') instanceof NodeList"),
        "true"
    );
    assert_eq!(
        ev("document.body.children instanceof HTMLCollection"),
        "true"
    );
    assert_eq!(
        ev("Object.prototype.toString.call(document.querySelectorAll('div'))"),
        "[object NodeList]"
    );
    assert_eq!(
        ev("document.querySelectorAll('div').constructor.name"),
        "NodeList"
    );
}

#[test]
fn nodelist_surface_spec_et_chemin_onetrust() {
    // Snapshot + accès indexé/item/length (5 divs :
    // consent-sdk > pc-sdk > cat-grp, puis a, b).
    assert_eq!(ev("document.querySelectorAll('div').length"), "5");
    assert_eq!(
        ev("document.querySelectorAll('div')[1].id"),
        "onetrust-pc-sdk"
    );
    assert_eq!(ev("document.querySelectorAll('div').item(3).id"), "a");
    assert_eq!(ev("document.querySelectorAll('div').item(99)"), "null");
    // Iteration : forEach(v, i, coll), for-of, spread,
    // entries/keys/values.
    assert_eq!(
        ev("var s='';document.querySelectorAll('body > div').forEach(function(v,i,c){s+=v.id+'@'+i+'/'+(c instanceof NodeList)+';'});s"),
        "onetrust-consent-sdk@0/true;a@1/true;b@2/true;"
    );
    assert_eq!(
        ev("var s='';for(var n of document.querySelectorAll('body > div'))s+=n.id+' ';s.trim()"),
        "onetrust-consent-sdk a b"
    );
    assert_eq!(
        ev("[...document.querySelectorAll('body > div')].map(function(n){return n.id}).join(',')"),
        "onetrust-consent-sdk,a,b"
    );
    assert_eq!(
        ev("document.querySelectorAll('div').entries().next().value[0]"),
        "0"
    );
    assert_eq!(
        ev("document.querySelectorAll('div').keys().next().value"),
        "0"
    );
    assert_eq!(
        ev("document.querySelectorAll('div').values().next().value.id"),
        "onetrust-consent-sdk"
    );
    // namedItem sur collections nommées conservé.
    assert_eq!(
        ev("document.getElementsByTagName('div').namedItem?1:0"),
        "0"
    );
    // Le chemin OneTrust verbatim : `m(el).append(nodelist)`
    // — branche `this.selector.appendChild(e[i])`.
    assert_eq!(
        ev("var r=document.createElement('div');var nl=document.querySelectorAll('div > .ot-cat-grp');for(var i=0;i<nl.length;i++)r.appendChild(nl[i]);r.children.length"),
        "1"
    );
}
