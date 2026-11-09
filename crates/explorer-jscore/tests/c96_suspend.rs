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

/// JS-C394 — positions paresseuses `&&`/`||`/`??`/`?:` portant
/// un suspend : hissage **sous garde** (vol 682, bundle xjs
/// Google : `a.TRa && b(await c(a.TRa(a.Xg)))` appelait
/// `a.TRa(…)` hors garde → `is not a function` — le suspend
/// était évalué éagerment, déviation documentée devenue bug
/// terrain). Chaque cas mesure aussi l'effet de bord `n` :
/// la branche non tirée ne produit rien.
#[test]
fn c394_lazy_positions_guard_suspend() {
    // `&&` falsy — forme exacte du site Google `Edc` : garde
    // `a.TRa` undefined → le membre droit ne s'évalue pas.
    let (s, c) = await_case(
        "var a={Xg:1,TRa:undefined,target:{},Aa:0,Qna:0,ka:0}; return a.TRa&&a.TRa(a.Xg);",
    );
    assert_eq!(seq(&s, &c), "r:undefined");
    // `&&` falsy : zéro effet de bord du membre droit.
    let (s, c) = await_case(
        "var n=0; function g(){n++;return Promise.resolve(9);} var r=false&&await g(); return n;",
    );
    assert_eq!(seq(&s, &c), "r:0");
    // `&&` truthy : le membre droit (await inclus) évalué.
    let (s, c) =
        await_case("var n=0; function g(){n++;return Promise.resolve(9);} return 1&&await g()+n;");
    assert_eq!(seq(&s, &c), "r:10");
    // `||` : le membre droit ne tire que si falsy.
    let (s, c) = await_case(
        "var n=0; function g(){n++;return Promise.resolve(9);} var r='L'||await g(); return r+':'+n;",
    );
    assert_eq!(seq(&s, &c), "r:L:0");
    let (s, c) = await_case(
        "var n=0; function g(){n++;return Promise.resolve(9);} var r=0||await g(); return r+':'+n;",
    );
    assert_eq!(seq(&s, &c), "r:9:1");
    // `??` : `null`/`undefined` seulement (loose `== null`).
    let (s, c) = await_case(
        "var n=0; function g(){n++;return Promise.resolve(9);} var r=5??await g(); return r+':'+n;",
    );
    assert_eq!(seq(&s, &c), "r:5:0");
    let (s, c) = await_case(
        "var n=0; function g(){n++;return Promise.resolve(9);} var r=null??await g(); return r+':'+n;",
    );
    assert_eq!(seq(&s, &c), "r:9:1");
    let (s, c) = await_case(
        "var n=0; function g(){n++;return Promise.resolve(9);} var r=undefined??await g(); return r+':'+n;",
    );
    assert_eq!(seq(&s, &c), "r:9:1");
    // `?:` : seule la branche choisie suspend.
    let (s, c) = await_case(
        "var n=0; function t(){n+=10;return Promise.resolve(1);} function f(){n+=100;return Promise.resolve(2);} var r=1?await t():await f(); return r+':'+n;",
    );
    assert_eq!(seq(&s, &c), "r:1:10");
    let (s, c) = await_case(
        "var n=0; function t(){n+=10;return Promise.resolve(1);} function f(){n+=100;return Promise.resolve(2);} var r=0?await t():await f(); return r+':'+n;",
    );
    assert_eq!(seq(&s, &c), "r:2:100");
    // Suspend paresseux imbriqué : `a && (b || await g())`.
    let (s, c) = await_case(
        "var n=0; function g(){n++;return Promise.resolve(9);} var r=1&&(0||await g()); return r+':'+n;",
    );
    assert_eq!(seq(&s, &c), "r:9:1");
    let (s, c) = await_case(
        "var n=0; function g(){n++;return Promise.resolve(9);} var r=0&&(0||await g()); return r+':'+n;",
    );
    assert_eq!(seq(&s, &c), "r:0:0");
    // La valeur awaitée circule dans l'expression englobante.
    let (s, c) = await_case(
        "var n=0; function g(){n++;return Promise.resolve(7);} var r=(1&&await g())*2; return r;",
    );
    assert_eq!(seq(&s, &c), "r:14");
    // Suspend en LHS : toujours évalué (garde comprise).
    let (s, c) = await_case(
        "var n=0; function g(){n++;return Promise.resolve(0);} var r=await g()&&9; return r+':'+n;",
    );
    assert_eq!(seq(&s, &c), "r:0:1");
}

/// JS-C400 / §B-111 — le lowering d'un corps suspendable
/// (`has_suspend`/`lift`/`lower_list`/`stmt_has_suspend`)
/// tournait **avant** le `maybe_grow` d'`exec_body`, sur le
/// segment courant : une fonction profonde débordait
/// indifféremment de `RUST_MIN_STACK` (vol 687 — `0xC00000FD`
/// à charge égale sous 2/32 Mio). Chaque arête récursive
/// re-bascule désormais — la profondeur devient un coût tas.
#[test]
fn c400_suspend_lowering_deep_expr() {
    std::thread::Builder::new()
        .stack_size(2 * 1024 * 1024)
        .spawn(|| {
            let mut e = Engine::new();
            // `!` × 20 000 sur `await p` — chaîne `Expr::Unary`
            // profonde : `has_suspend`/`lift` descendaient
            // autant de frames natives → débordement avant le
            // fix sur cette pile de 2 Mio. La borne logique de
            // nichement est relevée pour passer le parseur.
            e.set_nest_limit(200_000);
            let src = format!(
                "{P} async function f(){{ return {}await p; }} f().then(function(v){{out='r:'+v;}},function(x){{out='E:'+x;}});",
                "!".repeat(20_000)
            );
            e.eval(&src)
                .unwrap_or_else(|err| panic!("eval -> {err}"));
            assert_eq!(
                e.eval("out").map(|v| e.describe(v)).unwrap_or_default(),
                "r:true"
            );
        })
        .unwrap_or_else(|_| panic!("spawn"))
        .join()
        .unwrap_or_else(|_| panic!("thread panicked"));
}

/// Même classe côté motif : `expr_to_pat` / `has_suspend_pat` /
/// `lift_pat` recouraient nus depuis `eval_expr_inner`/
/// `exec_stmt_inner`, déjà dans un segment — `[[[[…]]]]`
/// hostile débordait le reste de segment sans pouvoir
/// re-basculer.
#[test]
fn c400_suspend_lowering_deep_pat() {
    std::thread::Builder::new()
        .stack_size(2 * 1024 * 1024)
        .spawn(|| {
            let mut e = Engine::new();
            // Affectation destructurante `([[[[…]]]] = v)` ×
            // 20 000 → `expr_to_pat` descend toute la grammaire
            // Array avant `bind_pat` (borne relevée pour le
            // parseur ; l'éval échoue ensuite en destructuration
            // — ce qui compte : plus de débordement).
            e.set_nest_limit(200_000);
            let n = 20_000;
            let src = format!(
                "{P} async function f(){{ var x; ({}x{} = await p); return 'ok'; }} f().then(function(v){{out='r:'+v;}},function(e2){{out='E:'+e2;}});",
                "[".repeat(n),
                "]".repeat(n)
            );
            e.eval(&src)
                .unwrap_or_else(|err| panic!("eval -> {err}"));
            let _ = e.eval("out");
        })
        .unwrap_or_else(|_| panic!("spawn"))
        .join()
        .unwrap_or_else(|_| panic!("thread panicked"));
}
