// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Régressions JS-C96 — `await`/`yield` hors lowering : têtes
//! de statements, motifs, clés calculées, paramètres,
//! `class extends`, `for await`. Chaque cas échouait avant le
//! fix par `TypeError: await on a pending promise is
//! unsupported` (mur mesuré x.com `entry-client-logged-out`).

use explorer_jscore::Engine;

/// Évalue `setup` puis `check` dans un second job (microtasks
/// drainées entre les deux — les promesses pending se résolvent).
#[track_caller]
fn seq(setup: &str, check: &str) -> String {
    let mut e = Engine::new();
    e.eval(setup)
        .unwrap_or_else(|err| panic!("setup {setup} -> {err}"));
    match e.eval(check) {
        Ok(v) => e.describe(v),
        Err(err) => panic!("check {check} -> {err}"),
    }
}

const P: &str = "var out='?'; var p=new Promise(function(res){ Promise.resolve().then(function(){res(7);}); });";

fn await_case(body: &str) -> (String, String) {
    (
        format!(
            "{P} async function f(){{ {body} }} f().then(function(v){{out='r:'+v;}},function(e){{out='E:'+e;}});"
        ),
        "out".to_string(),
    )
}

/// Têtes de statements — `for (… = await …)`, `switch … case
/// await`, `with (await)`, `using x = await`, `for-in var_init`.
#[test]
fn c96_stmt_heads() {
    let (s, c) = await_case("var s=0; for (var i = await p; i<10; i++) s=i; return s;");
    assert_eq!(seq(&s, &c), "r:9");
    let (s, c) = await_case("var s=0; for (let i = await p; i<10; i++) s=i; return s;");
    assert_eq!(seq(&s, &c), "r:9");
    let (s, c) = await_case("var i; for (i = await p; i<10; i++) return i; return -1;");
    assert_eq!(seq(&s, &c), "r:7");
    let (s, c) = await_case("switch(7){ case await p: return 'hit'; default: return 'def'; }");
    assert_eq!(seq(&s, &c), "r:hit");
    let (s, c) = await_case("var s=''; for (var k = await p in {a:1}) { s=k; break; } return s;");
    assert_eq!(seq(&s, &c), "r:a");
    let (s, c) = await_case("with (await p) { return 1; }");
    assert_eq!(seq(&s, &c), "r:1");
    // `using` : le suspend est résolu, puis `using` rejette
    // honnêtement la primitive (spec `AddDisposableResource`).
    let (s, c) = await_case("using x = await p; return 'u';");
    assert_eq!(
        seq(&s, &c),
        "E:TypeError: using declaration requires an object or null/undefined"
    );
    let (s, c) = await_case("await using x = await p; return 'u';");
    assert_eq!(
        seq(&s, &c),
        "E:TypeError: using declaration requires an object or null/undefined"
    );
}

/// Motifs destructurants — défauts `{a = …}`/`[… = …]` sur
/// déclarations, `catch`, cible `for-of`.
#[test]
fn c96_patterns() {
    let (s, c) = await_case("var {a = await p} = {}; return a;");
    assert_eq!(seq(&s, &c), "r:7");
    let (s, c) = await_case("var {a = await p} = {a:undefined}; return a;");
    assert_eq!(seq(&s, &c), "r:7");
    let (s, c) = await_case("var [a = await p] = []; return a;");
    assert_eq!(seq(&s, &c), "r:7");
    let (s, c) = await_case("try { throw {}; } catch ({a = await p}) { return a; }");
    assert_eq!(seq(&s, &c), "r:7");
    let (s, c) = await_case("var s=0; for (var [a = await p] of [[1]]) { s=a; } return s;");
    assert_eq!(seq(&s, &c), "r:1");
}

/// Clés calculées — littéral objet, membre de classe,
/// `class extends`.
#[test]
fn c96_computed_keys() {
    let (s, c) = await_case("var o = { [await p]: 1 }; return o[7];");
    assert_eq!(seq(&s, &c), "r:1");
    let (s, c) = await_case("var C = class { [await p](){} }; return typeof C;");
    assert_eq!(seq(&s, &c), "r:function");
    // `extends` : suspend résolu, puis `extends 7` rejette
    // honnêtement (non-constructor).
    let (s, c) = await_case("var C = class extends (await p) {}; return typeof C;");
    assert_eq!(seq(&s, &c), "E:TypeError: class extends a non-constructor");
}

/// Paramètres par défaut — `a = await e` garde la paresse spec
/// (défaut évalué seulement si l'arg est `undefined`) ; motifs
/// en paramètre déplacés dans le corps.
#[test]
fn c96_param_defaults() {
    assert_eq!(
        seq(
            &format!("{P} async function f(a = await p){{ return a; }} f().then(function(v){{out='r:'+v;}},function(e){{out='E:'+e;}});"),
            "out"
        ),
        "r:7"
    );
    // Paresse : arg fourni → le défaut n'est jamais évalué
    // (sinon `side` vaudrait 1).
    assert_eq!(
        seq(
            &format!("{P} var side=0; async function f(a = (side=1,await p)){{ return a; }} f(5).then(function(v){{out='r:'+v+':'+side;}},function(e){{out='E:'+e;}});"),
            "out"
        ),
        "r:5:0"
    );
    // Motif en paramètre : `{a = await e}` déplacé dans le corps.
    assert_eq!(
        seq(
            &format!("{P} async function f({{a = await p}} = {{}}){{ return a; }} f().then(function(v){{out='r:'+v;}},function(e){{out='E:'+e;}});"),
            "out"
        ),
        "r:7"
    );
    // `fn.length` compte toujours l'original (défaut exclu).
    assert_eq!(seq("async function f(a = await 1) {}", "f.length"), "0");
}

/// `for await` — itérateur async, suspension par pas :
/// `next()` pendante → awaited ; `{value,done}` par item ;
/// `done` termine ; rejet → throw dans le corps.
#[test]
fn c96_for_await() {
    // Itérateur async maison — `next()` rend des promesses
    // pending résolues en microtâche.
    let iter = "{ [Symbol.asyncIterator](){ var i=0; return { next(){ i+=1; return Promise.resolve({value:i,done:i>3}); } }; } }";
    let (s, c) = await_case(&format!(
        "var s=0; for await (var x of {iter}) {{ s+=x; }} return s;"
    ));
    assert_eq!(seq(&s, &c), "r:6");
    // Repli `@@iterator` sync (CreateAsyncFromSyncIterator).
    let (s, c) = await_case("var s=0; for await (var x of [10,20]) { s+=x; } return s;");
    assert_eq!(seq(&s, &c), "r:30");
    // `next()` rejetée → l'exception vit dans le corps.
    let (s, c) = await_case(
        "try { for await (var x of { [Symbol.asyncIterator](){ return { next(){ return Promise.reject('boom'); } }; } }) {} return 'no'; } catch(e) { return 'caught:'+e; }",
    );
    assert_eq!(seq(&s, &c), "r:caught:boom");
    // `break` sort de la boucle en attente.
    let (s, c) = await_case(&format!(
        "var s=0; for await (var x of {iter}) {{ s+=x; break; }} return s;"
    ));
    assert_eq!(seq(&s, &c), "r:1");
}
