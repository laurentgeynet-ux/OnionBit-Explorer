// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Batterie hostile par module (audit bornes/panics — piste JS-C55) :
//! chaque appel est pathologique — bornes, index, NaN/∞, surrogates,
//! profondeur — et le contrat est unique : **renvoyer `Ok` ou `Err`,
//! jamais paniquer**. Un panic fait échouer le test ; `Ok`/`Err` sont
//! tous deux recevables (le moteur dégrade ou refuse, les deux sont
//! honnêtes). Les budgets ops/heap sont serrés pour que les cas
//! « allocations astronomiques » se résolvent en `Err` rapidement.

use explorer_jscore::Engine;

/// Évalue `src` dans un moteur borné — le test échoue sur panic
/// (le runtime Rust le laisse propager).
#[track_caller]
fn ev(src: &str) -> Option<String> {
    let mut e = Engine::new();
    e.set_op_limit(200_000);
    e.set_mem_limit(16 * 1024 * 1024);
    match e.eval(src) {
        Ok(v) => Some(e.describe(v)),
        Err(_) => None,
    }
}

/// Batterie : chaque expression hostile — le seul contrat est
/// « pas de panic ».
fn battery(name: &str, srcs: &[&str]) {
    for (i, src) in srcs.iter().enumerate() {
        let _ = ev(src); // Ok/Err recevables — panic = échec.
        let _ = src; // silence
        let _ = i;
    }
    let _ = name;
}

#[test]
fn hostile_string() {
    battery(
        "string",
        &[
            "'ab'.charCodeAt(-1e20)",
            "'x'.at(1e30)",
            "'x'.repeat(-1)",
            "'x'.repeat(1e9)",
            "'abc'.padStart(2**31)",
            "'\\uD800'.charCodeAt(0)",
            "'\\u{1F600}'.codePointAt(1)",
            "'\\u{1F600}'.at(-1)",
            "'abc'.substring(NaN, Infinity)",
            "'abc'.slice(-1e30, 1e30)",
            "String.fromCharCode(1e20)",
            "String.fromCodePoint(0x110000)",
            "String.fromCodePoint(-1)",
            "'ab'.replace(/(a)/, '$$$`$&')",
            "'abc'.matchAll(/x/g).next().done",
            "'aaa'.split('').length",
            "'x'.repeat(2).padEnd(1e7, 'y')",
            "'x'.indexOf('y', -1e30)",
            "'abc'.lastIndexOf('a', 1e30)",
            "'\\uD800\\uDC00'.slice(0,1) + ' '.length",
            "'x'.localeCompare('y')",
            "''.normalize('XYZ')",
            "'abc'.includes('a', {})",
        ],
    );
}

#[test]
fn hostile_array() {
    battery(
        "array",
        &[
            "new Array(-1)",
            "new Array(1e9)",
            "new Array(2**32)",
            "[1,2].copyWithin(-1e20, 1e20)",
            "[1,2,3].fill(0, -1e20, 1e20)",
            "[1,2].splice(1e30, 1e30, 9)",
            "[1,2].splice(-1e30)",
            "[1,2].slice(-1e30)",
            "[1].lastIndexOf(1, -1e30)",
            "[3,1,2].sort(function(){return NaN})",
            "[[[1]]].flat(Infinity).length",
            "Array.from({length: 1e9}).length",
            "Array(3).fill(0).join('\u{1F600}')",
            "[1,2].indexOf(2, {})",
            "var a=[]; a[4294967295]=1; a.length",
            "var a=[]; a[-1]=5; Object.keys(a).join(',')",
            "[].concat.apply([], Array(1e5).fill([1]))",
            "[1,2,3].reduce(undefined)",
            "[].reduce(function(a,b){return a+b})",
            "Array.of.call(null)",
            "[1,2].at(-1e30)",
            "[1,,3].map(String).join('|')",
        ],
    );
}

#[test]
fn hostile_regexp() {
    battery(
        "regexp",
        &[
            "new RegExp('(?<x>)\\\\k<y>')",
            "new RegExp('[\\\\uD800-z]')",
            "new RegExp('[')",
            "new RegExp('(a')",
            "new RegExp('a{2,1}')",
            "new RegExp('*a')",
            "new RegExp('(?#x)')",
            "new RegExp('\\\\p{Bad}')",
            "new RegExp('a', 'x')",
            "var r=/x/g; r.lastIndex=1e30; 'x'.search(r)",
            "var r=/(?:)/gy; r.lastIndex=-5; r.exec('ab')",
            "'a\\u{1F600}b'.match(/\\u{1F600}/).index",
            "'abab'.replace(/(a)(b)?/g, '$<$1|$2|$9>')",
            "'ab'.split(/((x)?)/).join('|')",
            "/(?:(?:(?:(?:(?:(?:(?:(?:(?:(?:a*)*)*)*)*)*)*)*)*)*$/.test('a'.repeat(50))",
            "new RegExp('(a)' .repeat(1e4))",
            "/\\u{10FFFF}/u.test('\\u{10FFFF}')",
            "/[]/.test('x')",
            "/[^]/.test('x')",
        ],
    );
}

#[test]
fn hostile_number_typedarray() {
    battery(
        "number/ta",
        &[
            "(1e309).toFixed(100)",
            "(1).toString(37)",
            "(1).toString(1)",
            "(0.5).toPrecision(0)",
            "NaN.toString(2)",
            "new Uint8Array(-1)",
            "new Float64Array(1e10)",
            "var t=new Uint8Array(4); t.subarray(-1e30,1e30)[0]",
            "var t=new Uint8Array(4); t.set([1,2,3,4,5])",
            "var d=new DataView(new ArrayBuffer(2)); d.getInt8(5)",
            "var d=new DataView(new ArrayBuffer(2)); d.getFloat64(1e30)",
            "new ArrayBuffer(1e12)",
            "var b=new ArrayBuffer(8); b.slice(1e30)",
            "var b=new ArrayBuffer(8,{maxByteLength:16}); b.resize(1e9)",
            "new BigInt64Array(4)[0]=1.5",
            "new Uint8ClampedArray(4).fill(1.9, -1e30, 1e30).join(',')",
            "Atomics.load(new Int32Array(new SharedArrayBuffer(4)), 1e30)",
        ],
    );
}

#[test]
fn hostile_json_eval_function() {
    battery(
        "json/eval",
        &[
            "JSON.parse('[' + '['.repeat(1e4))",
            "var o={}; o.self=o; JSON.stringify(o)",
            "JSON.parse('\"\\\\u\"')",
            "JSON.parse('{\"a\"')",
            "JSON.stringify({toJSON:function(){return this}})",
            "JSON.parse('null', function(k,v){ if(k==='x') throw 7; return v})",
            "eval('')",
            "eval('(((((((((('",
            "new Function('x','y','return x*y')(3,4)",
            "new Function('`'.repeat(1e4))",
            "Function.prototype.call.call(function(){return 42})",
            "eval(\"var x=1;\" + \"x++\".repeat(0))",
            "decodeURIComponent('%')",
            "decodeURIComponent('%ZZ')",
            "encodeURI('\\uD800')",
            "parseInt('')",
            "parseFloat('0x10')",
        ],
    );
}

#[test]
fn hostile_temporal_intl() {
    battery(
        "temporal/intl",
        &[
            "Temporal.Duration.from('PT' + '9'.repeat(30) + 'S')",
            "Temporal.Duration.from({years: 1e30})",
            "Temporal.PlainDate.from('9999-99-99')",
            "Temporal.PlainDate.from('1970-01-01').add('P99999999D')",
            "Temporal.PlainTime.from('25:99:99')",
            "Temporal.Instant.fromEpochNanoseconds(9n ** 63n)",
            "Temporal.PlainDate.from('2020-01-01').toString({smallestUnit:'bogus'})",
            "Intl.DateTimeFormat('xx-BAD-tag').format(0)",
            "Intl.NumberFormat('en',{minimumFractionDigits:-1}).format(1)",
            "Intl.NumberFormat('en',{notation:'bogus'}).format(1)",
            "new Intl.Collator('en',{sensitivity:'bogus'})",
            "Intl.ListFormat('en',{type:'bogus'})",
        ],
    );
}

#[test]
fn hostile_bigint_math() {
    battery(
        "bigint/math",
        &[
            "10n ** 1e9n",
            "BigInt('0x')",
            "(1n).toString(37)",
            "(1n).toString(1)",
            "BigInt.asIntN(0, 5n)",
            "BigInt.asIntN(-1, 5n)",
            "BigInt.asUintN(1e30, 5n)",
            "1n << -1e30n",
            "Math.hypot(1e308, 1e308)",
            "Math.pow(-1, 1e20)",
            "Math.max() + Math.min()",
            "Math.clz32('x')",
            "Math.f16round(1e309)",
        ],
    );
}

#[test]
fn hostile_proxy_reflection() {
    battery(
        "proxy/reflect",
        &[
            "var p=new Proxy({},{}); p.__proto__ = p; p.x",
            "var p=new Proxy({},{}); p.__proto__ = p; p.x = 1",
            "var p=new Proxy({},{ownKeys:function(){return ['a','a']}}); Object.keys(p)",
            "var p=new Proxy({},{get:function(){throw 'boom'}}); p.x",
            "var r=Proxy.revocable({a:1},{}); r.revoke(); try{r.proxy.a}catch(e){'threw'}",
            "var p=new Proxy(function(){},{}); Reflect.construct(p,[])",
            "var p=new Proxy({},{}); Object.setPrototypeOf(p,p); 'x' in p",
            "Reflect.ownKeys(new Proxy({},{ownKeys:function(){return [Symbol()]}}))",
            "var o={}; Object.defineProperty(o,'x',{get:function(){throw 5}}); o.x",
            "var o={}; Object.defineProperty(o,'x',{value:1,writable:false}); o.x=2",
        ],
    );
}

#[test]
fn hostile_promise_async() {
    battery(
        "async",
        &[
            "Promise.resolve().then(function(){throw 9})",
            "new Promise(function(r){ r(1); r(2) })",
            "Promise.all(new Proxy([],{get:function(t,k){ if(k==='length') return 1e9; return t[k]}}))",
            "Promise.race('not iterable')",
            "async function f(){ await new Promise(function(){}) }; f()",
            "async function f(){ throw 'x' }; f().catch(String)",
            "(async function(){ await Promise.resolve() })()",
        ],
    );
}

#[test]
fn hostile_dom_mirror() {
    battery(
        "dom",
        &[
            "document.querySelector('[')",
            "document.querySelector('...')",
            "document.querySelectorAll('>>>')",
            "document.getElementById('')",
            "document.createElement('')",
            "document.createElement('x'.repeat(1e6))",
            "document.body.matches('::before')",
            "document.body.closest('[[')",
            "var d=document.createElement('div'); d.innerHTML='<p>'.repeat(1e4)",
            "document.body.setAttribute('x'.repeat(1e5),'y')",
        ],
    );
}
