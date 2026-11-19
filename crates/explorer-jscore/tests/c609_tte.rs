// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Gate JS-C609 — `%ThrowTypeError%` intrinsèque unique par
//! realm + `@@dispose`/`@@asyncDispose` sur les protos
//! d'itérateurs.
//!
//! Verrous : identité partagée (get === set partout),
//! `Function.prototype.{caller,arguments}` spec `{e:false,
//! c:true}` (`AddRestrictedFunctionProperties`), TTE figé
//! (`{Extensible:false}`, `length`/`name`
//! `{c:false}`, own keys `["length","name"]`, proto =
//! `Function.prototype`), sloppy `f.caller` → `null` (§B-125
//! préservé), `arguments` non-mappé poisonné `callee` seul
//! (`caller` retiré par ES2017 — note 5 §10.4.4),
//! Forbidden Extensions (aucun own `caller`/`arguments` hors fn
//! sloppy ordinaire — le TTE hérité jette), TTE distinct par
//! realm.

use explorer_jscore::Engine;

fn ev(src: &str) -> String {
    let mut e = Engine::new();
    match e.eval(src) {
        Ok(v) => format!("{v:?}"),
        Err(err) => format!("ERR {err}"),
    }
}

/// Extrait `%ThrowTypeError%` par le canal canonique test262 :
/// `callee` d'un `arguments` strict.
const TTE: &str = "var TTE=Object.getOwnPropertyDescriptor(function(){'use strict';return arguments}(),'callee').get;";

/// Identité unique : même objet en get/set de
/// `Function.prototype.{caller,arguments}`, des own props des
/// fns strictes, de `callee` des arguments stricts.
#[test]
fn identite_unique() {
    assert_eq!(
        ev(
            "var d=Object.getOwnPropertyDescriptor.bind(Object);var fp=Function.prototype;\
            (d(fp,'caller').get===d(fp,'caller').set)+'|'+\
            (d(fp,'caller').get===d(fp,'arguments').get)+'|'+\
            (d(fp,'caller').get===d(fp,'arguments').set)"
        ),
        "Str(\"true|true|true\")"
    );
    // Forbidden Extensions : fn stricte — **pas** de props
    // propres, le TTE vient du proto hérité.
    assert_eq!(
        ev(&format!(
            "{TTE}var sf=function(){{'use strict'}};\
            (sf.hasOwnProperty('caller'))+'|'+\
            (sf.hasOwnProperty('arguments'))+'|'+\
            (TTE===Object.getOwnPropertyDescriptor(Function.prototype,'caller').get)"
        )),
        "Str(\"false|false|true\")"
    );
    // `caller` n'est PAS posé sur l'objet arguments non-mappé
    // (ES2017 a retiré le poison — note 5 §10.4.4, audit
    // V8/JSC/QuickJS unanime) ; `callee` reste le même TTE.
    assert_eq!(
        ev(&format!(
            "{TTE}var a=function(){{'use strict';return arguments}}();\
            (Object.getOwnPropertyDescriptor(a,'caller')===undefined)+'|'+\
            (TTE===Object.getOwnPropertyDescriptor(a,'callee').get)"
        )),
        "Str(\"true|true\")"
    );
    // Paramètres non-simples → arguments non-mappé → TTE même sloppy.
    assert_eq!(
        ev(&format!(
            "{TTE}var a=function(x={{}}){{return arguments}}();\
            TTE===Object.getOwnPropertyDescriptor(a,'callee').get"
        )),
        "Bool(true)"
    );
    // Bound function : pas de props propres non plus —
    // `.caller` jette via le TTE du proto.
    assert_eq!(
        ev("var b=function(){}.bind(null);\
            (b.hasOwnProperty('caller'))+'|'+\
            (b.hasOwnProperty('arguments'))+'|'+\
            (function(){try{b.caller;return 'x'}catch(e){return e instanceof TypeError}})()"),
        "Str(\"false|false|true\")"
    );
}

/// Surface spec de `%ThrowTypeError%` : fonction figée, proto
/// `Function.prototype`, `length`/`name` verrouillés, own keys
/// ordonnées, pas de `caller`/`arguments` propres.
#[test]
fn surface_tte() {
    assert_eq!(ev(&format!("{TTE}typeof TTE")), "Str(\"function\")");
    assert_eq!(
        ev(&format!(
            "{TTE}Object.getPrototypeOf(TTE)===Function.prototype"
        )),
        "Bool(true)"
    );
    assert_eq!(ev(&format!("{TTE}Object.isFrozen(TTE)")), "Bool(true)");
    assert_eq!(
        ev(&format!("{TTE}Object.getOwnPropertyNames(TTE).join(',')")),
        "Str(\"length,name\")"
    );
    assert_eq!(
        ev(&format!("{TTE}var d=Object.getOwnPropertyDescriptor(TTE,'name');d.value+'|'+d.writable+'|'+d.enumerable+'|'+d.configurable")),
        "Str(\"|false|false|false\")"
    );
    assert_eq!(
        ev(&format!(
            "{TTE}var d=Object.getOwnPropertyDescriptor(TTE,'length');d.value+'|'+d.configurable"
        )),
        "Str(\"0|false\")"
    );
    assert_eq!(
        ev(&format!(
            "{TTE}TTE.hasOwnProperty('caller')+'|'+TTE.hasOwnProperty('arguments')"
        )),
        "Str(\"false|false\")"
    );
    assert_eq!(
        ev(&format!(
            "{TTE}try{{TTE()}}catch(e){{e instanceof TypeError}}"
        )),
        "Bool(true)"
    );
}

/// `Function.prototype` : accesseurs TTE `{e:false, c:true}`
/// (`AddRestrictedFunctionProperties`) — `delete` réussit,
/// lecture sur le proto lui-même jette.
#[test]
fn function_proto_spec() {
    assert_eq!(
        ev("var d=Object.getOwnPropertyDescriptor(Function.prototype,'caller');d.enumerable+'|'+d.configurable+'|'+typeof d.get"),
        "Str(\"false|true|function\")"
    );
    assert_eq!(ev("delete Function.prototype.caller"), "Bool(true)");
    assert_eq!(
        ev("try{Function.prototype.caller}catch(e){e instanceof TypeError}"),
        "Bool(true)"
    );
    assert_eq!(
        ev("try{Function.prototype.arguments=1}catch(e){e instanceof TypeError}"),
        "Bool(true)"
    );
}

/// §B-125 préservé : fn sloppy → `caller`/`arguments` `null`
/// (accesseur own masquant le TTE du proto) ; strict/flèche →
/// `TypeError`.
#[test]
fn sloppy_vs_restreint() {
    assert_eq!(
        ev("function f(){};f.caller+'|'+f.arguments"),
        "Str(\"null|null\")"
    );
    assert_eq!(
        ev("var f=function(){'use strict'};try{f.caller}catch(e){e instanceof TypeError}"),
        "Bool(true)"
    );
    assert_eq!(
        ev("var f=()=>{};try{f.arguments}catch(e){e instanceof TypeError}"),
        "Bool(true)"
    );
    // Descripteurs propres `{e:false, c:false}` des deux côtés.
    assert_eq!(
        ev("function f(){};var d=Object.getOwnPropertyDescriptor(f,'caller');d.enumerable+'|'+d.configurable"),
        "Str(\"false|false\")"
    );
}

/// Forbidden Extensions (spec note + test262 `forbidden-ext`) :
/// strict/flèche/méthode/get-set/générateur/async/classe/bind/
/// `new Function` — **aucun** own `caller`/`arguments` ; l'accès
/// jette via le `%ThrowTypeError%` de `Function.prototype`.
#[test]
fn forbidden_ext() {
    for (label, mk) in [
        ("strict", "var x=function(){'use strict'}"),
        ("arrow", "var x=()=>{}"),
        ("method", "var o={m(){}};var x=o.m"),
        (
            "getter",
            "var x=Object.getOwnPropertyDescriptor({get k(){return 1}},'k').get",
        ),
        ("gen", "var x=function*(){}"),
        ("async", "var x=async function(){}"),
        ("class", "var x=class{}"),
        ("classmethod", "var x=(class{m(){}}).prototype.m"),
        ("bound", "var x=function(){}.bind(null)"),
        ("newFunction", "var x=new Function('return 1')"),
    ] {
        assert_eq!(
            ev(&format!(
                "{mk};x.hasOwnProperty('caller')+'|'+x.hasOwnProperty('arguments')"
            )),
            "Str(\"false|false\")",
            "{label}: own props interdites"
        );
        assert_eq!(
            ev(&format!(
                "{mk};try{{x.caller}}catch(e){{e instanceof TypeError}}"
            )),
            "Bool(true)",
            "{label}: .caller doit jeter TypeError"
        );
    }
    // `new Function` : même constat côté `arguments` + write.
    assert_eq!(
        ev("var x=new Function('\"use strict\"');try{x.arguments=1}catch(e){e instanceof TypeError}"),
        "Bool(true)"
    );
}

/// `IsConstructor` : `[[Construct]]` absent des méthodes
/// (script et natives) — les vrais ctors conservent `new`.
/// Marqueur natif = prop `prototype` own posée par `ctor()`.
#[test]
fn not_a_constructor() {
    // Méthodes script : objet littéral, classe, statique, gen.
    assert_eq!(
        ev("var out=[];var o={m(){}};var C=class{s(){}m(){}};\
            [o.m, C.prototype.m, C.s, function*(){}, new Function('')].forEach(function(f){\
            try{new f();out.push('ok')}catch(e){out.push(e instanceof TypeError)}});out.join(',')"),
        "Str(\"true,true,true,true,ok\")"
    );
    // Méthodes natives : aucune n'est constructible.
    assert_eq!(
        ev("var out=[];\
            [Array.prototype.map, Object.keys, Function.prototype.call, Promise.resolve].forEach(function(f){\
            try{new f();out.push('ok')}catch(e){out.push(e instanceof TypeError)}});out.join(',')"),
        "Str(\"true,true,true,true\")"
    );
    // `Function.prototype` elle-même n'est pas un ctor.
    assert_eq!(
        ev("try{new Function.prototype()}catch(e){e instanceof TypeError}"),
        "Bool(true)"
    );
    // Les vrais ctors restent constructibles.
    assert_eq!(
        ev("var out=[];\
            [Object, Array, Error, TypeError, Date, RegExp].forEach(function(c){\
            try{new c();out.push('ok')}catch(e){out.push('ko:'+e.message)}});\
            try{new Function('return 1');out.push('ok')}catch(e){out.push('ko:'+e.message)};\
            try{new Map();new Set();out.push('ok')}catch(e){out.push('ko:'+e.message)};\
            try{new Promise(function(){});out.push('ok')}catch(e){out.push('ko:'+e.message)};\
            out.join(',')"),
        "Str(\"ok,ok,ok,ok,ok,ok,ok,ok,ok\")"
    );
    // Ctor de classe toujours constructible ; `new` d'un objet
    // ordinaire jette bien.
    assert_eq!(ev("new (class{})() instanceof Object"), "Bool(true)");
    assert_eq!(
        ev("try{new {}}catch(e){e instanceof TypeError}"),
        "Bool(true)"
    );
}

/// `%ThrowTypeError%` est **par realm** : l'objet extrait dans
/// un realm créé diffère du nôtre mais est unique en interne.
#[test]
fn tte_par_realm() {
    let mut e = Engine::new();
    e.install_t262();
    let v = e
        .eval(
            "var r=$262.createRealm();\
             var mine=Object.getOwnPropertyDescriptor(function(){'use strict';return arguments}(),'callee').get;\
             var theirs=r.evalScript(\"Object.getOwnPropertyDescriptor(function(){'use strict';return arguments}(),'callee').get\");\
             var theirs2=r.evalScript(\"Object.getOwnPropertyDescriptor(Function.prototype,'caller').get\");\
             (mine===theirs)+'|'+(theirs===theirs2)",
        )
        .expect("eval");
    assert_eq!(format!("{v:?}"), "Str(\"false|true\")");
}

/// `Iterator.prototype[Symbol.dispose]` : méthode spec —
/// `this.return()` invoqué (sans args), `undefined` rendu.
#[test]
fn iter_symbol_dispose() {
    assert_eq!(
        ev(
            "var p=Object.getPrototypeOf(Object.getPrototypeOf([][Symbol.iterator]()));\
            typeof p[Symbol.dispose]+'|'+p[Symbol.dispose].name+'|'+p[Symbol.dispose].length"
        ),
        "Str(\"function|[Symbol.dispose]|0\")"
    );
    assert_eq!(
        ev(
            "var p=Object.getPrototypeOf(Object.getPrototypeOf([][Symbol.iterator]()));\
            var it=Object.create(p);var called=0;var n=-1;\
            it.return=function(){called++;n=arguments.length;return {done:true}};\
            var r=it[Symbol.dispose]();called+'|'+n+'|'+(r===undefined)"
        ),
        "Str(\"1|0|true\")"
    );
    // Pas de `return` → `undefined` quand même.
    assert_eq!(
        ev(
            "var p=Object.getPrototypeOf(Object.getPrototypeOf([][Symbol.iterator]()));\
            Object.create(p)[Symbol.dispose]()"
        ),
        "Undefined"
    );
    // `return` non-callable → `TypeError` (GetMethod).
    assert_eq!(
        ev("var p=Object.getPrototypeOf(Object.getPrototypeOf([][Symbol.iterator]()));\
            var it=Object.create(p);it.return=5;try{it[Symbol.dispose]()}catch(e){e.constructor.name}"),
        "Str(\"TypeError\")"
    );
}

/// `%AsyncIteratorPrototype%[Symbol.asyncDispose]` : rend une
/// promesse, `this.return()` awaité puis `undefined` résolu.
#[test]
fn aiter_symbol_async_dispose() {
    let mut e = Engine::new();
    e.eval(
        "async function* g(){};\
         var ap=Object.getPrototypeOf(Object.getPrototypeOf(g.prototype));\
         var it=Object.create(ap);var called=0;\
         it.return=function(){called++;return Promise.resolve({done:true})};\
         var out='';\
         Promise.resolve(it[Symbol.asyncDispose]()).then(function(v){out=v+'|'+called});",
    )
    .expect("eval");
    e.run_microtasks().expect("microtasks");
    let v = e.eval("out").expect("read");
    assert_eq!(format!("{v:?}"), "Str(\"undefined|1\")");
    // `return` absent → promesse résolue `undefined`.
    let mut e2 = Engine::new();
    e2.eval(
        "async function* g(){};\
         var ap=Object.getPrototypeOf(Object.getPrototypeOf(g.prototype));\
         var out2='x';\
         Object.create(ap)[Symbol.asyncDispose]().then(function(v){out2=''+v;},function(r){out2='rej:'+r});",
    )
    .expect("eval2");
    e2.run_microtasks().expect("microtasks2");
    assert_eq!(
        format!("{:?}", e2.eval("out2").expect("r2")),
        "Str(\"undefined\")"
    );
}
