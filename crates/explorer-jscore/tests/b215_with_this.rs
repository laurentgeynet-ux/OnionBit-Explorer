// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! §B-215 — `TypeError: cannot read 'excludedCategories' /
//! 'isCategoryExcluded' / 'date' of Undefined` — arstechnica
//! `app.c4fb404fa8e90019.js` ×68 groupés (vol 932 → 972).
//! Alpine évalue ses bindings de template via
//! `new Function("scope", "with (scope) { let r = <expr>;
//! return r }")` : un appel bare `f()` **dans un `with`**
//! reçoit `this` = l'objet `with` (spec `GetThisValue` —
//! la référence a pour base l'object-environment-record).
//! Le moteur passait `undefined` → `this.prop` jetait.
//! Fix : flag `Env::any_with` + `with_this_of` (marche la
//! chaîne dans l'ordre de `env_get_seen`, receiver =
//! l'objet du `with` porteur) au bras `Expr::Ident` de
//! `Expr::Call`.

use explorer_jscore::{Dom, Engine};
use std::cell::RefCell;
use std::rc::Rc;

const PAGE: &str = r#"<html><body><div id="m"></div></body></html>"#;

fn ev(src: &str) -> String {
    let mut e = Engine::new();
    e.install_dom(&Rc::new(RefCell::new(Dom::from_html(PAGE))));
    match e.eval(src) {
        Ok(v) => e.describe(v),
        Err(err) => panic!("{src} -> {err}"),
    }
}

#[test]
fn appel_bare_dans_with_recoit_l_objet() {
    // Forme verbatim : méthode lue via `with` invoquée en
    // bare-call — `this` = l'objet `with` (= Node).
    assert_eq!(
        ev("var s={x:7,m:function(){return this.x}}; with(s){ m() }"),
        "7"
    );
    // Chemin exact d'Alpine : `new Function('scope','with(scope){…}')`.
    assert_eq!(
        ev(
            "var s={cats:['a','b'],ex:function(t){return this.cats.includes(t)}};\
            new Function('scope','with(scope){ let r = ex(\"b\"); return r }').call({},s)"
        ),
        "true"
    );
    // `with` imbriqués : le receiver est le with PORTEUR.
    assert_eq!(
        ev(
            "var a={f:function(){return this===a}},b={g:function(){return this===b}};\
            with(a){with(b){[f(),g()].join(',')}}"
        ),
        "true,true"
    );
    // Nom résolu HORS du with → receiver `undefined` →
    // sloppy : global ; la prop n'est pas volée au scope.
    assert_eq!(
        ev("function h(){return this===globalThis} with({}){ h() }"),
        "true"
    );
    // `let` du bloc masque la prop du with (ordre de
    // résolution : bloc déclaratif avant object-env).
    assert_eq!(
        ev("var o={f:function(){return 'obj'}}; with(o){ let f=function(){return 'blk'}; f() }"),
        "blk"
    );
    // `@@unscopables` : la prop exclue n'est pas un binding
    // `with` — le nom remonte au scope englobant.
    assert_eq!(
        ev(
            "var o={f:function(){return 'obj'}}; o[Symbol.unscopables]={f:true};\
            function f(){return 'out'} with(o){ f() }"
        ),
        "out"
    );
    // `var` traverse le `with` jusqu'au scope fonction.
    assert_eq!(ev("with({}){ var q = 9 } q"), "9");
}
