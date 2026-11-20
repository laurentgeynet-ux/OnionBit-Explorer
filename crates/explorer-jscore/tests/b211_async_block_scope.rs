// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! §B-211 — `TypeError: cannot read 'showFallbackContent' of Null`
//! (globo.com vitrine, vols 957/967, `bs/delivery/show`) : après une
//! suspension (`await`/`yield`), la machine à états `lazy.rs` exécutait
//! les corps `catch`/`finally` dans l'env du **bloc try** (`f.env`) au
//! lieu d'un bloc frère — les `let`/`const` du try fuyaient dans le
//! finally (`u` = `utm_source:null` masquait le module `r(9850)`).
//! Même racine pour les `var` : les envs de bloc async étaient
//! `Env::local` (avale `var`) au lieu de `Env::block` (traverse au
//! scope fonction). Fix : catch/finally → `Env::block(parent de
//! l'env try)` ; envs de bloc (`frame_env`/`Stmt::Block`/`Try`/
//! `Switch`) → `Env::block`. Verrous : try-const invisible en finally
//! (normal + unwind), catch frère, post-await decl, imbriqué,
//! `var` en bloc/if/for/try async, sémantique catch param préservée.

use explorer_jscore::Engine;

/// Évalue `setup`, draine les microtasks, puis lit `out`.
#[track_caller]
fn run(setup: &str) -> String {
    let mut e = Engine::new();
    e.eval(setup).unwrap_or_else(|err| panic!("setup -> {err}"));
    e.run_microtasks().expect("microtasks");
    match e.eval("log.join(',')") {
        Ok(v) => e.describe(v),
        Err(err) => panic!("read out -> {err}"),
    }
}

#[test]
fn finally_ne_voit_pas_les_lexicals_du_try() {
    // Cas nominal globo : `u` try-bloc vs `u` externe en finally.
    assert_eq!(
        run("var log=[];var u='OUTER';\
             (async()=>{try{const{utm_source:u}={utm_source:null};await 0;}finally{log.push(u)}})();\
             var out=log.join(',')"),
        "OUTER"
    );
    // `let` fuit pareil + const déclarée APRÈS l'await invisible.
    assert_eq!(
        run("var log=[];var v='V',w='W';\
             (async()=>{try{let v='in';await 0;}finally{log.push(v)}})();\
             (async()=>{try{await 0;const w2='post';}finally{log.push(typeof w2)}})();\
             var out=log.join(',')"),
        "V,undefined"
    );
    // Catch : bloc frère aussi — les lexicals du try n'y vivent pas.
    assert_eq!(
        run("var log=[];var u='OUTER';\
             (async()=>{try{const u='in';await 0;throw 7}catch(e){log.push(u+'|'+e)}})();\
             var out=log.join(',')"),
        "OUTER|7"
    );
    // Unwind (throw traversé) : finally draine avec l'env frère.
    assert_eq!(
        run("var log=[];var u='OUTER';\
             (async()=>{try{const u='in';await 0;throw 1}catch(e){}finally{log.push(u)}})();\
             var out=log.join(',')"),
        "OUTER"
    );
    // Imbriqué : chaque finally voit son propre parent.
    assert_eq!(
        run("var log=[];var n='N';\
             (async()=>{try{const n='t1';try{const n='t2';await 0}finally{log.push(n)}}finally{log.push(n)}})();\
             var out=log.join(',')"),
        "t1,N"
    );
}

#[test]
fn var_en_bloc_async_rejoint_le_scope_fonction() {
    // `{var x}`/`if{var}`/`for{var}`/`try{var}` async : le `var`
    // traverse le bloc (spec `VarDeclaredNames`) — `Local` l'avalait.
    assert_eq!(
        run("var log=[];\
             (async()=>{{var a=1;await 0}log.push(a)})();\
             (async()=>{if(true){var b=2;await 0}log.push(b)})();\
             (async()=>{for(let i=0;i<1;i++){var c=3;await 0}log.push(c)})();\
             (async()=>{try{var d=4;await 0}finally{}log.push(d)})();\
             var out=log.join(',')"),
        "1,2,3,4"
    );
    // Catch `var` identique + le param catch reste lexical.
    assert_eq!(
        run("var log=[];\
             (async()=>{try{await 0;throw 'x'}catch(q){var e2=5;log.push(q)}log.push(e2)})();\
             var out=log.join(',')"),
        "x,5"
    );
}
