// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Régression JS-C525d/e : un corps différé doit produire une
//! sémantique **identique** quels que soient les lookaheads et
//! délimiteurs voisins — signature terrain N5.26 (chemin pull) :
//! handler inline `onclick="…"` (wrapper `with(document){with
//! (this.form||{}){with(this){…}}}`) muet par `expected )` avalé.
//! Conservé comme verrou du contrat de déferral (vaut pour tout
//! backend de saut de corps — `close_of` eager actuel compris).

use explorer_jscore::{set_defer_min, Dom, Engine};
use std::cell::RefCell;
use std::rc::Rc;

fn eval_with_defer(n: usize, src: &str) -> String {
    set_defer_min(n);
    let mut e = Engine::new();
    match e.eval(src) {
        Ok(v) => format!("{v:?}"),
        Err(x) => format!("ERR {x:?}"),
    }
}

/// `(fn)` dont le corps est différé alors que des lookaheads
/// avaient atteint `Eof` — parse identique eager/pull.
#[test]
fn defer_after_eof_lookahead() {
    for src in [
        "(function(){var aa=0;with(a){x=1;y=2;z=3;w=4;v=5;u=6;t=7}})",
        "(function(){var aa=0;with(a||{}){x=1;y=2;z=3;w=4}})",
        "(function(){var aa=0;if(a||{}){x=1;y=2;z=3;w=4}})",
        "(function(event){with(document||{}){with(this.form||{}){with(this){hit=(hit||0)+1\n}}}})",
    ] {
        let eager = eval_with_defer(usize::MAX, src);
        let defer = eval_with_defer(24, src);
        assert_eq!(eager, defer, "{src}");
    }
}

/// Bout à bout : le handler inline se compile et tire en pull.
#[test]
fn inline_handler_fires_with_deferred_body() {
    set_defer_min(24);
    let mut e = Engine::new();
    e.install_dom(&Rc::new(RefCell::new(Dom::from_html(
        "<html><body><button id=\"b\" onclick=\"hit=(hit||0)+1\">x</button></body></html>",
    ))));
    let v = e
        .eval(
            "var hit = 0;\
             document.getElementById('b').click();\
             document.getElementById('b').click();\
             hit",
        )
        .expect("eval");
    assert_eq!(format!("{v:?}"), "Num(2.0)");
    set_defer_min(usize::MAX);
}
