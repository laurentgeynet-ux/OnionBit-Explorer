// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! JS-C574 — §B-137 (`select.add` absent — ebay
//! `add is not a function on HTMLSelectElement`, vol 855)
//! et §B-136 (`observe: Element requis` airbnb ×5 ;
//! `matches: receiver is not an Element` lefigaro ×8).
//! Famille « receveur élément réel non résolu » : l'arg/
//! receveur est un Element côté site mais hors du miroir
//! courant (cross-arène, époque recyclée, wrapper `.node`).
//! Dégradation spec-compatible : `observe` accepte sans
//! enregistrer, `matches`/`closest` rendent `false`/`null`,
//! les args non-nœud gardent leur `TypeError`.

use explorer_jscore::{Dom, Engine};
use std::cell::RefCell;
use std::rc::Rc;

const PAGE: &str = r#"
<html><body>
<div id="main" class="box"><p id="p1">Hello</p></div>
<select id="s"><option id="o1">A</option><option id="o2">B</option></select>
</body></html>
"#;

fn engine_with(html: &str) -> Engine {
    let mut e = Engine::new();
    e.install_dom(&Rc::new(RefCell::new(Dom::from_html(html))));
    e
}

/// `src` évalué — le résultat (ou l'erreur, préfixée `!`)
/// décrit en chaîne.
fn ev_in(e: &mut Engine, src: &str) -> String {
    match e.eval(src) {
        Ok(v) => e.describe(v),
        Err(err) => format!("!{err}"),
    }
}

/// `src` évalué dans un `try/catch` — `String` de la valeur
/// attrapée, ou du résultat.
fn ev_catch(e: &mut Engine, src: &str) -> String {
    ev_in(
        e,
        &format!("(function(){{try{{return String({src})}}catch(x){{return String(x)}}}})()"),
    )
}

// -- §B-137 : select.add ------------------------------------------------

#[test]
fn select_add_appends_by_default() {
    let mut e = engine_with(PAGE);
    assert_eq!(
        ev_in(&mut e, "typeof document.getElementById('s').add"),
        "function"
    );
    assert_eq!(
        ev_in(
            &mut e,
            "var o=document.createElement('option');o.textContent='C';\
             var s=document.getElementById('s');s.add(o);\
             s.children.length + ':' + s.children[2].textContent"
        ),
        "3:C"
    );
}

#[test]
fn select_add_before_index() {
    let mut e = engine_with(PAGE);
    // `add(o, 0)` — inséré avant `options[0]` (spec).
    assert_eq!(
        ev_in(
            &mut e,
            "var o=document.createElement('option');o.id='new';\
             var s=document.getElementById('s');s.add(o,0);\
             s.children[0].id + '|' + s.children.length"
        ),
        "new|3"
    );
}

#[test]
fn select_add_before_element() {
    let mut e = engine_with(PAGE);
    // `add(o, el)` — inséré devant `o2`.
    assert_eq!(
        ev_in(
            &mut e,
            "var o=document.createElement('option');o.id='mid';\
             var s=document.getElementById('s');s.add(o,document.getElementById('o2'));\
             s.children[1].id"
        ),
        "mid"
    );
    // `before` non descendant du select → HierarchyRequestError.
    let r = ev_catch(
        &mut e,
        "document.getElementById('s').add(document.createElement('option'),document.getElementById('main'))",
    );
    assert!(
        r.contains("HierarchyRequest"),
        "attendu Hierarchy*, reçu : {r}"
    );
}

#[test]
fn select_add_rejects_non_option() {
    let mut e = engine_with(PAGE);
    // Union IDL option|optgroup — un <div> → TypeError.
    let r = ev_catch(
        &mut e,
        "document.getElementById('s').add(document.createElement('div'))",
    );
    assert!(
        r.contains("option/optgroup"),
        "attendu union IDL, reçu : {r}"
    );
    // `add` absent des autres tags — `<div>.add` = undefined.
    assert_eq!(
        ev_in(&mut e, "typeof document.getElementById('main').add"),
        "undefined"
    );
}

/// `optgroup` accepté (union IDL), déplacé entier.
#[test]
fn select_add_accepts_optgroup() {
    let mut e = engine_with(PAGE);
    assert_eq!(
        ev_in(
            &mut e,
            "var g=document.createElement('optgroup');g.label='G';\
             var s=document.getElementById('s');s.add(g);\
             s.children[2].tagName"
        ),
        "OPTGROUP"
    );
}

// -- §B-136 : receveurs éléments hors portée ----------------------------

#[test]
fn matches_unwraps_node_wrapper() {
    let mut e = engine_with(PAGE);
    // `.call({node: el})` — wrapper ShadyDOM-like : `.node`
    // déballé (2 niveaux max).
    assert_eq!(
        ev_in(
            &mut e,
            "var el=document.getElementById('main');\
             el.matches.call({node: el}, 'div')"
        ),
        "true"
    );
    assert_eq!(
        ev_in(
            &mut e,
            "var el=document.getElementById('main');\
             el.closest.call({node: {node: el}}, 'div').id"
        ),
        "main"
    );
}

#[test]
fn matches_keeps_typeerror_for_non_element() {
    let mut e = engine_with(PAGE);
    // Parité spec : receveur franchement non-élément → TypeError.
    let r = ev_catch(
        &mut e,
        "document.getElementById('main').matches.call(42, 'div')",
    );
    assert!(
        r.contains("not an Element"),
        "attendu TypeError, reçu : {r}"
    );
    let r = ev_catch(
        &mut e,
        "document.getElementById('main').matches.call(null, 'div')",
    );
    assert!(
        r.contains("not an Element"),
        "attendu TypeError, reçu : {r}"
    );
}

/// Élément d'une **autre arène** (`DOMParser`) : le rejet
/// borné est conservé — choix délibéré (verrou
/// `cross_document_node_args_are_adopted`), les arènes ne
/// fusionnant pas.
#[test]
fn observe_still_rejects_foreign_document_element() {
    let mut e = engine_with(PAGE);
    let r = ev_catch(
        &mut e,
        "(function(){var d=new DOMParser().parseFromString('<div/>','text/html');\
         new IntersectionObserver(function(){}).observe(d.querySelector('div'))})()",
    );
    assert!(
        r.contains("Element requis"),
        "rejet borné attendu, reçu : {r}"
    );
}

/// **Verbatim §B-136 airbnb** : l'observer est construit sur
/// le miroir 1, puis `install_dom` resynchronise le document
/// (miroir 2) — `self.dom` figé rendait « étranger » tout
/// élément post-resync (`observe: Element requis`). La
/// résolution contre `dom_ctx` vivant accepte et
/// **enregistre** la cible dans le miroir courant.
#[test]
fn observe_resolves_in_live_mirror_after_resync() {
    let mut e = engine_with(PAGE);
    assert_eq!(
        ev_in(
            &mut e,
            "globalThis.io = new IntersectionObserver(function(){}); 'ok'"
        ),
        "ok"
    );
    // Resync : le 2ᵉ miroir remplace le 1er — les éléments
    // obtenus via `document` vivent dans la nouvelle arène.
    e.install_dom(&Rc::new(RefCell::new(Dom::from_html(PAGE))));
    assert_eq!(
        ev_catch(
            &mut e,
            "(function(){globalThis.io.observe(document.getElementById('main'));\
             globalThis.io.unobserve(document.getElementById('main'));\
             return 'accepted'})()"
        ),
        "accepted"
    );
    // Et le chemin inverse : élément du 1er miroir passé à un
    // observer du 2ᵉ — même rejet borné (arène étrangère au
    // ctx courant).
    assert_eq!(
        ev_in(
            &mut e,
            "globalThis.oldEl = document.getElementById('main'); 'ok'"
        ),
        "ok"
    );
    e.install_dom(&Rc::new(RefCell::new(Dom::from_html(PAGE))));
    let r = ev_catch(
        &mut e,
        "(function(){new IntersectionObserver(function(){}).observe(globalThis.oldEl)})()",
    );
    assert!(
        r.contains("Element requis"),
        "rejet borné attendu, reçu : {r}"
    );
}

/// Élément **détaché** (`createElement`, jamais inséré) :
/// reste un `Element` — `matches`/`closest` répondent
/// normalement au lieu du `TypeError` `receiver is not an
/// Element` (verbatim lefigaro §B-136).
#[test]
fn matches_closest_on_detached_element() {
    let mut e = engine_with(PAGE);
    assert_eq!(
        ev_in(
            &mut e,
            "(function(){var el=document.createElement('div');\
             return el.matches('div')+'/'+(el.closest('div')===el)})()"
        ),
        "true/true"
    );
}

#[test]
fn observe_keeps_typeerror_for_non_node() {
    let mut e = engine_with(PAGE);
    let r = ev_catch(
        &mut e,
        "new IntersectionObserver(function(){}).observe(null)",
    );
    assert!(
        r.contains("Element requis"),
        "attendu TypeError, reçu : {r}"
    );
    let r = ev_catch(&mut e, "new IntersectionObserver(function(){}).observe({})");
    assert!(
        r.contains("Element requis"),
        "attendu TypeError, reçu : {r}"
    );
}

/// Wrapper `{node: el}` nu (objet plain — forme réelle du
/// retour `ShadyDOM.wrap()`) : `shady_unwrap_in` n'était
/// atteint que pour les porteurs `Host` — l'early-return
/// `_ => None` filtrait l'objet littéral avant. §B-136 :
/// déballage borné à 2 niveaux, même-arène requis.
#[test]
fn observe_unwraps_node_wrapper() {
    let mut e = engine_with(PAGE);
    assert_eq!(
        ev_catch(
            &mut e,
            "(function(){var el=document.getElementById('main');\
             new IntersectionObserver(function(){}).observe({node: el}); return 'accepted'})()"
        ),
        "accepted"
    );
}
