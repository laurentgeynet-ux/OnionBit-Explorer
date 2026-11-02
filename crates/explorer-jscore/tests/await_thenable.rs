// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Régressions JS-C278 — `await` adopte les thenables étrangers
//! (mur §B-14 « promesses étrangères » dailymotion : `await`
//! liait l'objet brut au lieu d'appeler `then` →
//! `object is not iterable` en destructuration). Aussi :
//! `resolve_promise` lit `then` par le `[[Get]]` canonique
//! (accesseurs, proxies, host `get`) — un getter à effet de
//! bord n'est invoqué qu'une fois.

use explorer_jscore::{Engine, XhrRequest, XhrTransport};
use std::rc::Rc;

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

/// Pilote une `async function f` : son résultat/rejet est
/// capturé dans `out`.
fn await_case(body: &str) -> (String, String) {
    (
        format!(
            "var out='?'; async function f(){{ {body} }} f().then(function(v){{out='r:'+v;}},function(e){{out='E:'+e;}});"
        ),
        "out".to_string(),
    )
}

/// `await {then(r){r(42)}}` — suspension puis reprise sur le
/// job `then` (l'objet nu n'est plus lié brut).
#[test]
fn c278_await_thenable_sync() {
    let (s, c) = await_case("var t={then:function(res){res(42);}}; var v = await t; return v;");
    assert_eq!(seq(&s, &c), "r:42");
}

/// Thenable qui settle en différé (microtask en amont).
#[test]
fn c278_await_thenable_deferred() {
    let (s, c) = await_case(
        "var t={then:function(res){Promise.resolve().then(function(){res('late');});}};\
         var v = await t; return v;",
    );
    assert_eq!(seq(&s, &c), "r:late");
}

/// Thenable rejeté → la raison jette au site `await` et le
/// `catch` interne la rattrape.
#[test]
fn c278_await_thenable_reject() {
    let (s, c) = await_case(
        "var t={then:function(res,rej){rej('raison');}};\
         try { await t; return 'no'; } catch(x) { return 'caught:'+x; }",
    );
    assert_eq!(seq(&s, &c), "r:caught:raison");
}

/// Thenable chaîné : `then` résout avec un autre thenable —
/// l'adoption reboucle (`resolve_promise` réentrée).
#[test]
fn c278_await_thenable_chain() {
    let (s, c) = await_case(
        "var inner={then:function(r){r(5);}};\
         var outer={then:function(r){r(inner);}};\
         var v = await outer; return v;",
    );
    assert_eq!(seq(&s, &c), "r:5");
}

/// `then` en accesseur : le getter est invoqué **une fois**
/// (spec `[[Get]]`) et son résultat callable adopte.
#[test]
fn c278_await_thenable_accessor_once() {
    let (s, c) = await_case(
        "var n=0; var t={};\
         Object.defineProperty(t,'then',{get:function(){n++;return function(r){r('g');};}});\
         var v = await t; return v+':'+n;",
    );
    assert_eq!(seq(&s, &c), "r:g:1");
}

/// Getter `then` qui jette → le wrapper rejette et l'exception
/// arrive au site `await` (catchable), pas de déroulé natif.
#[test]
fn c278_await_thenable_getter_throw() {
    let (s, c) = await_case(
        "var t={};\
         Object.defineProperty(t,'then',{get:function(){throw 'boom';}});\
         try { await t; return 'no'; } catch(x) { return 'caught:'+x; }",
    );
    assert_eq!(seq(&s, &c), "r:caught:boom");
}

/// Même angle mort côté `resolve` : `Promise.resolve(t)` où
/// `then` est un accesseur — adoption (avant : fulfill brut).
#[test]
fn c278_resolve_thenable_accessor() {
    let (s, c) = await_case(
        "var t={};\
         Object.defineProperty(t,'then',{get:function(){return function(r){r(11);};}});\
         var v = await Promise.resolve(t); return v;",
    );
    assert_eq!(seq(&s, &c), "r:11");
}

/// Signature dailymotion verbatim : polyfill maison nommé
/// `Promise` + `[s] = await P.all(…)` — la destructuration
/// reçoit la valeur adoptée, pas le thenable (`object is not
/// iterable` du runtime-app).
#[test]
fn c278_await_polyfill_promise_all() {
    let (s, c) = await_case(
        "var P=function Promise(ex){var self=this;ex(function(v){self._v=v;},function(){});};\
         P.prototype.then=function(f){var v=f(this._v);return v===undefined?this:v;};\
         P.all=function(l){return new P(function(res){res([l[0]._v]);});};\
         var p1=new P(function(r){r(7);});\
         var r=await P.all([p1]); return r[0];",
    );
    assert_eq!(seq(&s, &c), "r:7");
}

/// `await` sur un objet non-thenable → lie l'objet lui-même
/// (spec : fulfill avec `v`).
#[test]
fn c278_await_plain_object() {
    let (s, c) = await_case("var v = await {a:9}; return v.a;");
    assert_eq!(seq(&s, &c), "r:9");
    // `then` présent mais non callable → pas un thenable.
    let (s, c) = await_case("var v = await {then:5}; return v.then;");
    assert_eq!(seq(&s, &c), "r:5");
}

/// `await req` IDB — le pont thenable de `IDBRequest` est
/// adopté : la request ouvre, `result` est lié (mastodon
/// vol 116 : les wrappers `await`-ent la request).
#[test]
fn c278_await_idb_request() {
    struct T;
    impl XhrTransport for T {
        fn send(&self, _req: &XhrRequest) -> u64 {
            0
        }
    }
    let mut e = Engine::new();
    e.install_xhr(Rc::new(T));
    e.eval(
        "var out='?';\
         async function main(){\
           var db = await indexedDB.open('awdb',1);\
           return db.name+'@'+db.version;\
         }\
         main().then(function(v){out='r:'+v;},function(e){out='E:'+e;});",
    )
    .expect("setup");
    e.run_microtasks().expect("microtasks");
    let out = e.eval("out").map(|v| e.describe(v)).unwrap();
    assert_eq!(out, "r:awdb@1");
}

/// Chemin eager `await_value` (`Array.fromAsync`) : thenable
/// étranger non suspendable → erreur nommée classable.
#[test]
fn c278_eager_foreign_thenable_named_error() {
    let (s, c) = await_case(
        "return Array.fromAsync([{then:function(r){r(1);}}]).then(\
           function(){return 'resolved';},function(x){return 'E:'+x;});",
    );
    let out = seq(&s, &c);
    assert!(
        out.starts_with("r:E:TypeError: await on a foreign thenable"),
        "out={out}"
    );
}
