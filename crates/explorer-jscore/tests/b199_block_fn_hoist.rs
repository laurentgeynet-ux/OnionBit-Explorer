// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! §B-199 — `TypeError: Object.defineProperty on non-object` —
//! hulu/disney `main-…:1:82631` ×9 vagues + arstechnica
//! `cntracking…:35601` + `app…:177249` (vols 932 → 972).
//! Verbatim : helper webpack `n.dn(u)` =
//! `Object.defineProperty(u, "name", …)` où `u` est une
//! **`function` déclarée plus bas dans le même bloc** —
//! `BlockDeclarationInstantiation` (spec 14.2.3) doit lier la
//! fonction dès l'entrée du bloc. Le moteur ne pré-instanciait
//! rien : `u` résolvait vers la liaison `var` Annex-B du scope
//! fonction → `undefined` → verbatim.
//! Fix : `block_instantiate` (exec.rs — blocs, try/catch/
//! finally, switch) + noms canoniques `block_names_ord`
//! incluant `FunctionDecl` (resolve.rs — sinon le chemin
//! rapide `env_get_resolved` court-circuitait l'env bloc) +
//! sync Annex-B sloppy vers le scope `var`.

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
fn fonction_de_bloc_hissee_des_l_entree() {
    // Bloc nu : visible avant sa position textuelle.
    assert_eq!(
        ev("{ var r = typeof a + ':' + a(); function a(){return 1} r }"),
        "function:1"
    );
    // try / catch / finally — chaque clause a son env.
    assert_eq!(
        ev("var o=''; try { o = typeof t + ':' + t(); function t(){return 2} } catch(e) {} o"),
        "function:2"
    );
    assert_eq!(
        ev("var o=''; try { throw 1 } catch(e) { o += typeof c + ':' + c(); function c(){return 3} } o"),
        "function:3"
    );
    assert_eq!(
        ev("var o=''; try {} finally { o += typeof f + ':' + f(); function f(){return 4} } o"),
        "function:4"
    );
    // `if`/`switch` — l'env du switch est partagé par les cases.
    assert_eq!(
        ev("if (true) { typeof i === 'function' ? i() : 0; function i(){return 5} }"),
        "5"
    );
    assert_eq!(
        ev("switch(1){case 1: var r = typeof s + ':' + s(); function s(){return 6} r}"),
        "function:6"
    );
}

#[test]
fn verbatim_webpack_dn_recoit_la_fonction() {
    // Forme exacte cn.js:35601 — `n.dn(u)` dans un bloc `try`,
    // `function u` déclarée plus bas dans le même bloc.
    assert_eq!(
        ev(
            "function dn(f){ Object.defineProperty(f,'name',{value:'u',configurable:true}); return f.name }\
            var out; try { out = dn(u); } catch(e) { out = 'THREW' } function u(){} out"
        ),
        "u"
    );
}

#[test]
fn annex_b_sloppy_fuit_vers_le_scope_var() {
    // Sloppy : la fonction de bloc se synchronise sur la
    // liaison `var` du scope fonction à l'entrée du bloc.
    assert_eq!(
        ev("{ function ab(){return 9} } typeof ab + ':' + ab()"),
        "function:9"
    );
    // Strict : pas de fuite — la liaison reste dans le bloc.
    assert_eq!(
        ev("'use strict'; { function st(){return 1} } typeof st"),
        "undefined"
    );
    // Dans une fonction : la fuite cible l'env fonction, pas global.
    assert_eq!(
        ev("function g(){ { function inb(){return 7} } return inb() } g()"),
        "7"
    );
}

/// Évalue `setup`, draine les microtasks, puis lit `log`.
#[track_caller]
fn run(setup: &str) -> String {
    let mut e = Engine::new();
    e.install_dom(&Rc::new(RefCell::new(Dom::from_html(PAGE))));
    e.eval(setup).unwrap_or_else(|err| panic!("setup -> {err}"));
    e.run_microtasks().expect("microtasks");
    match e.eval("log.join(',')") {
        Ok(v) => e.describe(v),
        Err(err) => panic!("read log -> {err}"),
    }
}

#[test]
fn chemin_lazy_async_avec_await_verbatim() {
    // Verbatim cn.js:35601 — module async webpack : `n.dn(u)`
    // AVANT un `await`, `function u` en fin du même `try`.
    // Le `await` rend le bloc suspensif → machine lazy (GExec)
    // qui a ses propres envs de bloc — `block_instantiate` y
    // est requis autant qu'en sync.
    assert_eq!(
        run(
            "var log=[];\
            function dn(f){ Object.defineProperty(f,'name',{value:'u',configurable:true}); return f.name }\
            (async()=>{ try { log.push(dn(u)); await 0; function u(){return 1} } catch(e){ log.push('THREW:'+e) } finally {} })()"
        ),
        "u"
    );
    // `function` dans un bloc `if`/`while`/`switch` suspensif
    // (frame_env / bras lazy dédiés).
    assert_eq!(
        run(
            "var log=[];\
            (async()=>{ if(true){ log.push(typeof i + ':' + i()); await 0; function i(){return 2} } })();\
            (async()=>{ while(false){} for(let k=0;k<1;k++){ log.push(typeof w + ':' + w()); await 0; function w(){return 3} } })();\
            (async()=>{ switch(1){ case 1: log.push(typeof s + ':' + s()); await 0; function s(){return 4} } })()"
        ),
        "function:2,function:3,function:4"
    );
    // Fonction dans catch + finally suspensifs.
    assert_eq!(
        run(
            "var log=[];\
            (async()=>{ try{ throw 9 }catch(e){ log.push(typeof c + ':' + c()); await 0; function c(){return 5} } finally{ log.push(typeof f + ':' + f()); await 0; function f(){return 6} } })()"
        ),
        "function:5,function:6"
    );
}

#[test]
fn chemin_resolu_dans_fonction_sync() {
    // Le chemin rapide `env_get_resolved` (slots canoniques)
    // doit trouver la fn de bloc — sinon court-circuit vers
    // la liaison `var` Annex-B du scope fonction.
    assert_eq!(
        ev("function outer(){ { return typeof a2 + ':' + a2(); function a2(){return 3} } } outer()"),
        "function:3"
    );
}
