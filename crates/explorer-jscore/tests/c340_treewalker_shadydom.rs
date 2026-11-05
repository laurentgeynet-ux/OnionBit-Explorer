// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! JS-C340 — `TreeWalker` relatif à `currentNode` + accesseurs
//! natifs de navigation sur `Node.prototype`/`Element.prototype`
//! (mur §B-97 du vol 644 : `ShadyDOM` repositionne `currentNode`
//! puis appelle `firstChild`/`nextSibling`, et capture les
//! getters natifs `getOwnPropertyDescriptor(Node.prototype,
//! 'firstChild').get` — méthode ancrée sur `root` et getters
//! absents → montage Polymer mort à l'init).

use explorer_jscore::{Dom, Engine};
use std::cell::RefCell;
use std::rc::Rc;

const PAGE: &str = r#"<html><body>
<div id="a"><span id="s1">t1</span><span id="s2">t2</span><b id="s3">b</b></div>
<div id="empty"></div>
<div id="b"><i id="i1"></i><i id="i2"></i></div>
</body></html>"#;

fn engine() -> (Engine, Rc<RefCell<Dom>>) {
    let mut e = Engine::new();
    let dom = Rc::new(RefCell::new(Dom::from_html(PAGE)));
    e.install_dom(&dom);
    (e, dom)
}

fn ev(e: &mut Engine, src: &str) -> String {
    match e.eval(src) {
        Ok(v) => e.describe(v),
        Err(err) => panic!("{src} -> {err}"),
    }
}

/// `firstChild`/`nextSibling`/`parentNode` relatifs à
/// `currentNode` — le coeur du protocole `ShadyDOM`.
#[test]
fn treewalker_traverses_relative_to_current_node() {
    let (mut e, _dom) = engine();
    ev(
        &mut e,
        "var tw = document.createTreeWalker(document.body, NodeFilter.SHOW_ELEMENT);\
         tw.currentNode = document.getElementById('a');\
         var n1 = tw.firstChild(); var n2 = tw.nextSibling(); var n3 = tw.lastChild();\
         var p = tw.parentNode();\
         var out = [n1 && n1.id, n2 && n2.id, String(n3), p && p.id].join('|');'ok'",
    );
    // firstChild→s1 (cur devient s1) ; nextSibling→s2 ;
    // lastChild depuis s2 (sans enfant élément)→null ;
    // parentNode depuis s2→#a.
    assert_eq!(ev(&mut e, "out"), "s1|s2|null|a");
}

/// Receveur vide : `firstChild`/`lastChild` sur `#empty` →
/// `null`, **pas** `document.documentElement` ni un nœud hors
/// du sous-arbre (le bug §B-97 : borne `root` au lieu de `cur`).
#[test]
fn treewalker_empty_receiver_returns_null() {
    let (mut e, _dom) = engine();
    ev(
        &mut e,
        "var tw = document.createTreeWalker(document.documentElement, NodeFilter.SHOW_ELEMENT);\
         tw.currentNode = document.getElementById('empty');\
         var f = tw.firstChild(); var l = tw.lastChild();\
         var out = String(f) + '|' + String(l) + '|' + (f === document.documentElement);'ok'",
    );
    assert_eq!(ev(&mut e, "out"), "null|null|false");
}

/// `nextSibling`/`previousSibling` bornés au sous-arbre `root`
/// — un frère hors `root` n'est jamais rendu.
#[test]
fn treewalker_siblings_bounded_by_root() {
    let (mut e, _dom) = engine();
    ev(
        &mut e,
        "var tw = document.createTreeWalker(document.getElementById('b'), NodeFilter.SHOW_ELEMENT);\
         tw.currentNode = document.getElementById('i1');\
         var nx = tw.nextSibling(); var pv = tw.previousSibling();\
         tw.currentNode = document.getElementById('i1');\
         var nx2 = tw.previousSibling();\
         var out = [nx && nx.id, pv && pv.id, String(nx2)].join('|');'ok'",
    );
    // i1.nextSibling→i2 ; previousSibling depuis i2→i1 ;
    // previousSibling depuis i1→null (borné à root=#b).
    assert_eq!(ev(&mut e, "out"), "i2|i1|null");
}

/// `parentNode` : premier ancêtre accepté — `root` retournable.
#[test]
fn treewalker_parent_node_accepts_root() {
    let (mut e, _dom) = engine();
    ev(
        &mut e,
        "var tw = document.createTreeWalker(document.getElementById('a'), NodeFilter.SHOW_ELEMENT);\
         tw.currentNode = document.getElementById('s1');\
         var p = tw.parentNode();\
         var p2 = tw.parentNode();\
         var out = [p && p.id, String(p2)].join('|');'ok'",
    );
    // s1.parentNode→#a (== root, acceptée) ; ensuite null
    // (cur==root → plus de parent).
    assert_eq!(ev(&mut e, "out"), "a|null");
}

/// `previousNode` : ordre document inverse depuis `currentNode`,
/// `root` retournable puis borne.
#[test]
fn treewalker_previous_node_reverse_order() {
    let (mut e, _dom) = engine();
    ev(
        &mut e,
        "var tw = document.createTreeWalker(document.getElementById('a'), NodeFilter.SHOW_ELEMENT);\
         tw.currentNode = document.getElementById('s3');\
         var a = tw.previousNode(); var b = tw.previousNode(); var c = tw.previousNode(); var d = tw.previousNode();\
         var out = [a && a.id, b && b.id, c && c.id, String(d)].join('|');'ok'",
    );
    // s3 → s2 → s1 → #a (root acceptée) → null.
    assert_eq!(ev(&mut e, "out"), "s2|s1|a|null");
}

/// Filtre `whatToShow` élément : les nœuds texte sont sautés,
/// `reject` élague le sous-arbre.
#[test]
fn treewalker_filter_skip_vs_reject() {
    let (mut e, _dom) = engine();
    ev(
        &mut e,
        "var tw = document.createTreeWalker(document.getElementById('a'), NodeFilter.SHOW_ELEMENT,\
             function(n){ return n.id === 's2' ? NodeFilter.FILTER_REJECT : NodeFilter.FILTER_ACCEPT; });\
         tw.currentNode = document.getElementById('a');\
         var r=[]; var n; while(n=tw.nextNode()) r.push(n.id);\
         var out = r.join(',');'ok'",
    );
    assert_eq!(ev(&mut e, "out"), "s1,s3");
}

/// `Object.getOwnPropertyDescriptor(Node.prototype,'firstChild')
/// .get` — callable natif, appliqué via `.call(node)` (forme
/// exacte capturée par `ShadyDOM` à l'init).
#[test]
fn node_proto_nav_descriptor_getter_is_callable() {
    let (mut e, _dom) = engine();
    assert_eq!(
        ev(
            &mut e,
            "var d = Object.getOwnPropertyDescriptor(Node.prototype, 'firstChild');\
             var el = document.getElementById('a');\
             typeof d.get + '|' + (d.get.call(el) === el.firstChild) + '|' + d.get.call(el).id"
        ),
        "function|true|s1"
    );
    // `parentNode`/`nextSibling`/`childNodes` idem.
    assert_eq!(
        ev(
            &mut e,
            "var g = function(n){ return Object.getOwnPropertyDescriptor(Node.prototype, n).get; };\
             var el2 = document.getElementById('s1');\
             [g('parentNode').call(el2).id, g('nextSibling').call(el2).id, \
              g('childNodes').call(el2).length].join('|')"
        ),
        "a|s2|1"
    );
}

/// `Element.prototype` — accesseurs élément-only
/// (`firstElementChild`/`children`/`innerHTML`) natifs.
#[test]
fn element_proto_nav_descriptor_getters() {
    let (mut e, _dom) = engine();
    assert_eq!(
        ev(
            &mut e,
            "var ge = function(n){ var d = Object.getOwnPropertyDescriptor(Element.prototype, n);\
                 return d && d.get; };\
             var el = document.getElementById('a');\
             [typeof ge('firstElementChild'), ge('firstElementChild').call(el).id, \
              ge('children').call(el).length, ge('nextElementSibling').call(el).id].join('|')"
        ),
        "function|s1|3|empty"
    );
    // `innerHTML` : get + set natifs, valeur brute identique.
    assert_eq!(
        ev(
            &mut e,
            "var el = document.getElementById('empty');\
             var d = Object.getOwnPropertyDescriptor(Element.prototype, 'innerHTML');\
             d.set.call(el, '<u>x</u>');\
             [d.get.call(el), el.firstElementChild.tagName].join('|')"
        ),
        "<u>x</u>|U"
    );
}

/// Les accesseurs ne cassent pas la voie ordinaire :
/// `el.textContent = x` passe toujours (setter natif posé).
#[test]
fn nav_accessors_keep_writable_props_writable() {
    let (mut e, _dom) = engine();
    assert_eq!(
        ev(
            &mut e,
            "var el = document.getElementById('empty');\
             el.textContent = 'via prop';\
             var d = Object.getOwnPropertyDescriptor(Node.prototype, 'textContent');\
             d.set.call(el, 'via desc');\
             [el.textContent, d.get.call(el), typeof d.set].join('|')"
        ),
        "via desc|via desc|function"
    );
}
