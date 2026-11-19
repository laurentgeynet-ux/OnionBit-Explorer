// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Gate JS-C626 — sémantique `%IteratorPrototype%` /
//! `Iterator.from` (`collections.rs`, `builtins/c17.rs`,
//! `heap.rs`) :
//! 1. `GetIteratorDirect` — `next` lu brut : l'appel sur
//!    `{next: 0}` ne jette qu'au `next()` (paresseux) ;
//!    receveur non-objet → `TypeError` à l'appel.
//! 2. `GetIteratorFlattenable` — `@@iterator` lu **avant**
//!    `next` (ordre observable) ; `flatMap` rejette les
//!    primitives (`"ab"` non aplatie).
//! 3. Validation fermée : arg invalide → `IteratorClose`
//!    du sous-jacent (`map(null)`, `take(-1)`,
//!    `includes(0, 1.5)`, `windows(1, null)`).
//! 4. `return` retiré de `%IteratorPrototype%` (un
//!    itérateur de tableau n'en a pas) ; helpers/
//!    `%WrapForValidIteratorPrototype%` le gardent
//!    (`Iterator.from(o).return()` → `o.return`, verbatim).
//! 5. `includes(v, skipped)` non coercé, `join` nullish →
//!    `""` + close sur coercition, `windows(n, undersized)`
//!    `"allow-partial"`, `flatMap` : `return` forwardé à
//!    l'interne ; step interne abrupt → seul le sous-jacent
//!    fermé.

use explorer_jscore::Engine;

/// Évalue `src` et rend `out` décrit.
#[track_caller]
fn run(src: &str) -> String {
    let mut e = Engine::new();
    e.eval(src).unwrap_or_else(|err| panic!("setup -> {err}"));
    match e.eval("out") {
        Ok(v) => e.describe(v),
        Err(err) => panic!("read out -> {err}"),
    }
}

/// Helpers paresseux + terminaux de base toujours sains.
#[test]
fn c626_basics() {
    let out = run(
        "var out=Iterator.from([1,2,3,4]).map(x=>x*2).filter(x=>x>2).take(2).toArray().join('|')",
    );
    assert_eq!(out, "4|6");
}

/// `map.call({next:0})` ne jette pas ; le `TypeError` vient
/// au `next()` (spec `GetIteratorDirect` — `next` non validé).
#[test]
fn c626_lazy_noncallable_next() {
    let out = run("var h=Iterator.prototype.map.call({next:0},x=>x);\
         var out;try{h.next();out='noerr'}catch(e){out=e.constructor.name}");
    assert_eq!(out, "TypeError");
}

/// Receveur non-objet → `TypeError` à l'appel (eager).
#[test]
fn c626_this_non_object() {
    let out = run(
        "var out;try{Iterator.prototype.map.call(7,x=>x);out='noerr'}catch(e){out=e.constructor.name}",
    );
    assert_eq!(out, "TypeError");
}

/// Arg invalide → `IteratorClose` du sous-jacent (`map`,
/// `take`, `includes`, `windows`/`chunks`, `join`).
#[test]
fn c626_validation_closes() {
    let out = run(
        "var c=0;var mk=function(){return {__proto__:Iterator.prototype,\
             next:function(){return {done:true,value:undefined}},\
             return:function(){c++;return {}}}};\
         try{mk().map(null)}catch(e){}\
         try{mk().take(-1)}catch(e){}\
         try{mk().take()}catch(e){}\
         try{mk().includes(0,1.5)}catch(e){}\
         try{mk().includes(0,-1)}catch(e){}\
         try{mk().includes(0,{valueOf:function(){return 0}})}catch(e){}\
         try{mk().windows(1,null)}catch(e){}\
         try{mk().chunks(0)}catch(e){}\
         try{mk().join({toString:function(){throw 9}})}catch(e){}\
         var out='c'+c",
    );
    assert_eq!(out, "c9");
}

/// Types d'erreur spec : NaN → `RangeError` (`take`), non-
/// entier `skipped` → `TypeError`, négatif → `RangeError`,
/// `windows(0)` → `RangeError`, `windows(1,'x')` → `TypeError`.
#[test]
fn c626_error_kinds() {
    let out = run(
        "var it={__proto__:Iterator.prototype,next:function(){return {done:true}}};\
         var k=function(f){try{f()}catch(e){return e.constructor.name}return 'noerr'};\
         var out=[k(function(){it.take()}),k(function(){it.take(-1)}),\
                  k(function(){it.includes(0,1.5)}),k(function(){it.includes(0,-1)}),\
                  k(function(){it.includes(0,NaN)}),k(function(){it.windows(0)}),\
                  k(function(){it.windows(1,'x')}),k(function(){it.chunks(1.5)})].join(',')",
    );
    assert_eq!(
        out,
        "RangeError,RangeError,TypeError,RangeError,TypeError,RangeError,TypeError,TypeError"
    );
}

/// `take` accepte `-0.5` (→ 0), `null` (→ 0), `Infinity` ;
/// `includes` `+Infinity` = skip-all (faux, `return` jamais
/// appelé, source drainée).
#[test]
fn c626_limit_edges() {
    let out = run(
        "var g=function(){return {__proto__:Iterator.prototype,\
             n:0,rc:0,next:function(){this.n++;return this.n<4?{done:false,value:this.n}:{done:true}},\
             return:function(){this.rc++;return {}}}};\
         var a=g(),b=g(),c=g();\
         var out=[a.take(-0.5).toArray().length,\
                  b.take(Infinity).toArray().length+','+b.rc,\
                  c.includes(9,Infinity)+','+c.n+','+c.rc].join('|')",
    );
    assert_eq!(out, "0|3,0|false,4,0");
}

/// Un itérateur de tableau n'a PAS de `return` (spec) ;
/// `every` early-exit sans `return` = pas de close, le
/// `next()` suivant rend l'élément restant.
#[test]
fn c626_iterator_has_no_return() {
    let out = run("var it=[1,2,3,4,5][Symbol.iterator]();\
         var r=it.every(function(v){return v<4});\
         var n=it.next();\
         var out=(it.return===undefined)+'|'+r+'|'+n.value+'|'+it.next().done");
    assert_eq!(out, "true|false|5|true");
}

/// `Iterator.from(o).return()` → `o.return` appelé, résultat
/// **verbatim** forwardé.
#[test]
fn c626_wrap_return_verbatim() {
    let out = run(
        "var calls=[];var o={next:function(){calls.push('n');return {done:true}},\
             return:function(){calls.push('r');return {value:5,done:true}}};\
         var w=Iterator.from(o);\
         var r1=w.return();var r2=w.return();\
         var out=calls.join('')+'|'+r1.value+'/'+r1.done+'|'+r2.value+'/'+r2.done",
    );
    assert_eq!(out, "r|5/true|undefined/true");
}

/// `Iterator.from` : `@@iterator` non-callable → `TypeError` ;
/// null/undefined → repli itérateur `{next}` ; chaîne
/// primitive admise (`iterate-string-primitives`).
#[test]
fn c626_from_flattenable() {
    let out = run(
        "var n={next:function(){return {done:true,value:0}}};\
         var t='';try{Iterator.from({[Symbol.iterator]:0,next:function(){}})}catch(e){t=e.constructor.name}\
         var a=Array.from(Iterator.from({[Symbol.iterator]:null,next:function(){return {done:true}}})).length;\
         var s=Iterator.from('ab').next().value;\
         var out=t+'|'+a+'|'+s",
    );
    assert_eq!(out, "TypeError|0|a");
}

/// `flatMap` : `return` forwardé à l'itérateur interne ouvert ;
/// chaîne mapper-result → `TypeError` (reject-primitives) ;
/// step interne abrupt → sous-jacent fermé, interne PAS fermé.
#[test]
fn c626_flatmap_semantics() {
    let out = run(
        "var ic=0,oc=0;\
         var inner=function(){return {n:0,next:function(){this.n++;\
             return this.n<3?{done:false,value:this.n}:{done:true}},\
             return:function(){ic++;return {}}}};\
         var f=Iterator.from([0]).flatMap(function(){return inner()});\
         f.next();f.next();f.return();\
         var a=ic;\
         var t='';try{Iterator.from([1]).flatMap(function(){return 'ab'}).next()}catch(e){t=e.constructor.name}\
         var inn={next:function(){throw 'X'},return:function(){ic++;return{}},\
             [Symbol.iterator]:function(){return this}};\
         var ou={__proto__:Iterator.prototype,next:function(){return {done:false,value:1}},\
             return:function(){oc++;return{}}};\
         try{ou.flatMap(function(){return inn}).next()}catch(e){}\
         var out=a+'|'+t+'|'+oc+'/'+ic",
    );
    assert_eq!(out, "1|TypeError|1/1");
}

/// `windows(n,"allow-partial")` rend les suffixes courts ;
/// `join` met les nullish vides.
#[test]
fn c626_windows_join() {
    let out = run(
        "var a=Iterator.from([1,2]).windows(3,'allow-partial').toArray().map(function(w){return w.join('.')}).join('|');\
         var b=Iterator.from([1,2]).windows(3).toArray().length;\
         var j=Iterator.from([1,null,2,undefined]).join('-');\
         var out=a+'|'+b+'|'+j",
    );
    assert_eq!(out, "1.2|0|1--2-");
}
