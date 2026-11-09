// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! JS-C456 (JT-12) — `for..of` paresseux sur itérateur
//! custom (vol 722 — l'eager matérialisait tout avant la
//! boucle) : `next()` par tour, `IteratorClose`
//! (`it.return()`) sur `break`/`throw`, `done`/`continue`
//! sans `return()`.

use explorer_jscore::{Dom, Engine};
use std::cell::RefCell;
use std::rc::Rc;

fn engine() -> Engine {
    let mut e = Engine::new();
    e.install_dom(&Rc::new(RefCell::new(Dom::from_html(
        "<html><body></body></html>",
    ))));
    e
}

fn ev(e: &mut Engine, src: &str) -> String {
    match e.eval(src) {
        Ok(v) => e.describe(v),
        Err(err) => panic!("{src} -> {err}"),
    }
}

const GEN: &str = "function* g(){ yield 1; yield 2; yield 3; }";

/// `break` appelle `it.return()` — `IteratorClose` spec :
/// le générateur est **fermé** (`next` → `done`) — l'eager
/// l'aurait laissé vivant (vidé, `next` rendait `3`).
#[test]
fn c456_break_does_not_consume_iterator() {
    let mut e = engine();
    assert_eq!(
        ev(
            &mut e,
            &format!(
                "{GEN}\n\
                 var it = g(), seen = [];\n\
                 for (var x of it) {{ seen.push(x); if (x === 2) break; }}\n\
                 seen.join(',') + '|' + it.next().value"
            )
        ),
        "1,2|undefined"
    );
}

/// `IteratorClose` : `it.return()` appelé sur `break`,
/// pas sur `done`/`continue`.
#[test]
fn c456_iterator_close_on_break_not_done() {
    let mut e = engine();
    assert_eq!(
        ev(
            &mut e,
            "var closed = [];\
             var it = {[Symbol.iterator](){return {i:0,\
                 next(){this.i++;return this.i<4?{value:this.i,done:false}:{done:true}},\
                 return(){closed.push('r');return {done:true}}}}};\n\
             for (var x of it) { if (x===2) break; }\
             closed.join(',')"
        ),
        "r"
    );
    // done complet → pas de return().
    assert_eq!(
        ev(
            &mut e,
            "var c2 = [];\
             var it2 = {[Symbol.iterator](){return {i:0,\
                 next(){this.i++;return this.i<2?{value:1,done:false}:{done:true}},\
                 return(){c2.push('r');return {done:true}}}}};\n\
             for (var y of it2) {} c2.join(',')"
        ),
        ""
    );
}

/// `throw` dans le corps → `IteratorClose` + propagation.
#[test]
fn c456_throw_calls_return_and_propagates() {
    let mut e = engine();
    assert_eq!(
        ev(
            &mut e,
            "var closed = '';\
             var it = {[Symbol.iterator](){return {i:0,\
                 next(){this.i++;return {value:this.i,done:false}},\
                 return(){closed='r';return {done:true}}}}};\n\
             try { for (var x of it) { throw 'boom'; } }\n\
             catch (er) { closed + '|' + er }"
        ),
        "r|boom"
    );
}

/// La voie `iter_values` des arrays reste correcte.
#[test]
fn c456_array_path_unchanged() {
    let mut e = engine();
    assert_eq!(
        ev(
            &mut e,
            "var s=[];for (var x of [1,2,3]) s.push(x); s.join(',')"
        ),
        "1,2,3"
    );
}
