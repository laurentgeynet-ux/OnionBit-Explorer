// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! §B-170 — `SyntaxError: function name 'e' may not be used
//! as a parameter name` — cluster vols 894-972 : twitch ×5
//! vagues (`e` :1:36129 — boot-mort), bilibili (`n` :457153),
//! tiktok (`t` :80509), vimeo storm ×17 (`t` :70958),
//! indeed ×6 (`e` :308392 — le script entier mourrait au
//! parse). Une
//! function-expression `function e(e){}` est LÉGALE :
//! le nom se lie dans l'environnement qui enveloppe la
//! fonction (scope « nom »), les paramètres se lient dans
//! le scope paramètre et le masquent — aucune early error
//! spec n'interdit la collision (V8 accepte en sloppy ET
//! en strict). Le check ajouté en JS-C611 était faux.

use explorer_jscore::Engine;

fn ev(src: &str) -> String {
    let mut e = Engine::new();
    match e.eval(src) {
        Ok(v) => e.describe(v),
        Err(err) => panic!("{src} -> {err}"),
    }
}

#[test]
fn function_expression_nom_egal_param_acceptee() {
    // Verbatim indeed : nom == param → le paramètre masque.
    assert_eq!(ev("(function e(e){ return e + 1 })(41)"), "42");
    assert_eq!(ev("var f = function t(t){ return t * 10 }; f(3)"), "30");
    assert_eq!(ev("((function h(h){ return h * 2 })(21))"), "42");
    // Le nom reste visible depuis le corps (self-recursion).
    assert_eq!(ev("(function rec(){ return typeof rec })()"), "function");
    assert_eq!(
        ev("(function rec(r){ return typeof r === 'number' ? r : typeof rec })(9)"),
        "9"
    );
    // Mode strict : pareil (le nom n'est ni `eval` ni `arguments`).
    assert_eq!(ev("'use strict'; (function s(s){ return s - 1 })(8)"), "7");
}

#[test]
fn early_errors_reelles_preservees() {
    let mut e = Engine::new();
    // Doublon de paramètre en strict : toujours SyntaxError.
    assert!(e.eval("'use strict'; (function(a,a){})").is_err());
    // Doublon via pattern non-simple : toujours SyntaxError.
    assert!(e.eval("(function([a,a]){})").is_err());
    // Param ∩ déclaration lexicale du corps : toujours SyntaxError.
    assert!(e.eval("(function(x){ let x = 1 })").is_err());
    // `eval`/`arguments` en param strict : toujours SyntaxError.
    assert!(e.eval("'use strict'; (function(eval){})").is_err());
}
