// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Verrous JS-C646 — résiduel `Object` : `ToObject` sur les arguments
//! (`keys`/`names`/`symbols`/`descriptor(s)`/`hasOwn`), `assign` spec,
//! `setPrototypeOf` `TypeError`s, `fromEntries` itérateur + `Define`,
//! `Object(sym)` wrapper, `newTarget`, `[[Prototype]]` `NativeError`
//! → `Error`, proxy `ownKeys`/integrity.

use explorer_jscore::Engine;

fn eval(src: &str) -> String {
    let mut e = Engine::new();
    e.install_t262();
    match e.eval(src) {
        Ok(v) => e.describe(v),
        Err(err) => format!("ERR {err}"),
    }
}

#[test]
fn c646_toobject_arguments() {
    assert_eq!(eval("try{Object.keys(null)}catch(x){x.name}"), "TypeError");
    assert_eq!(
        eval("try{Object.getOwnPropertyNames(undefined)}catch(x){x.name}"),
        "TypeError"
    );
    assert_eq!(
        eval("try{Object.getOwnPropertySymbols(null)}catch(x){x.name}"),
        "TypeError"
    );
    assert_eq!(
        eval("try{Object.getOwnPropertyDescriptor(undefined,'x')}catch(x){x.name}"),
        "TypeError"
    );
    assert_eq!(
        eval("try{Object.hasOwn(null,'a')}catch(x){x.name}"),
        "TypeError"
    );
    // Ordre spec : `ToObject(O)` d'abord — la clé n'est jamais
    // convertie si le receiver est incoercible.
    assert_eq!(
        eval("try{Object.hasOwn(null,{toString(){throw 'E'}})}catch(x){x.name||x}"),
        "TypeError"
    );
    // Wrapper string : index propres énumérables.
    assert_eq!(eval("Object.keys('ab').join()"), "0,1");
    assert_eq!(
        eval("Object.getOwnPropertyNames('ab').join()"),
        "0,1,length"
    );
    assert_eq!(eval("Object.values('abc').join()"), "a,b,c");
}

#[test]
fn c646_assign_spec() {
    // Cible primitive → `ToObject` (wrapper rendu).
    assert_eq!(
        eval("var r=Object.assign('test',{a:1}); typeof r+'|'+r.valueOf()+'|'+r.a"),
        "object|test|1"
    );
    assert_eq!(eval("typeof Object.assign(Symbol('s'),{a:1})"), "object");
    // `[[Set]]` : refus sur cible gelée → TypeError (data property).
    assert_eq!(
        eval(
            "var t={};Object.defineProperty(t,'x',{value:1,writable:false});\
             try{Object.assign(t,{x:2});'nothrow'}catch(x){x.name}"
        ),
        "TypeError"
    );
}

#[test]
fn c646_set_prototype_of_typeerrors() {
    assert_eq!(
        eval("try{Object.setPrototypeOf(null,{})}catch(x){x.name}"),
        "TypeError"
    );
    assert_eq!(
        eval("try{Object.setPrototypeOf({},undefined)}catch(x){x.name}"),
        "TypeError"
    );
    assert_eq!(
        eval("try{Object.setPrototypeOf({},5)}catch(x){x.name}"),
        "TypeError"
    );
    assert_eq!(
        eval("try{Object.setPrototypeOf({},Symbol())}catch(x){x.name}"),
        "TypeError"
    );
    // Cible primitive non-objet : rendue telle quelle, pas de throw.
    assert_eq!(eval("Object.setPrototypeOf(5,null)"), "5");
    // `Object.prototype` = immutable-proto exotic object.
    assert_eq!(
        eval("var p=Object.create(null);try{Object.setPrototypeOf(Object.prototype,p);'nothrow'}catch(x){x.name}"),
        "TypeError"
    );
}

#[test]
fn c646_from_entries() {
    assert_eq!(
        eval("try{Object.fromEntries()}catch(x){x.name}"),
        "TypeError"
    );
    assert_eq!(
        eval("try{Object.fromEntries(5)}catch(x){x.name}"),
        "TypeError"
    );
    // Entry primitive → TypeError (string non-objet).
    assert_eq!(
        eval("try{Object.fromEntries(['ab'])}catch(x){x.name}"),
        "TypeError"
    );
    // Entry non-itérable : accès `0`/`1` par `Get`.
    assert_eq!(eval("Object.fromEntries([{0:'a',1:'b'}]).a"), "b");
    // `CreateDataProperty` — pas `[[Set]]` : un setter hérité ne
    // s'observe pas et la clé reste data.
    assert_eq!(
        eval(
            "var set=0;Object.defineProperty(Object.prototype,'x',{set(){set=1},configurable:true});\
             var r=Object.fromEntries([['x',5]]);r.x+'|'+set"
        ),
        "5|0"
    );
}

#[test]
fn c646_symbol_wrapper_and_newtarget() {
    // `Object(sym)` → wrapper frais `%Symbol.prototype%`.
    assert_eq!(
        eval(
            "var s=Symbol('A');var o=Object(s);\
             typeof o+'|'+(o!==s)+'|'+(Object.getPrototypeOf(o)===Symbol.prototype)"
        ),
        "object|true|true"
    );
    // `new Sub(...)` : `NewTarget` ≠ fonction active → objet
    // ordinaire du proto de la sous-classe, argument ignoré.
    assert_eq!(
        eval(
            "class O extends Object{};var o=new O({a:1});\
             o.a===undefined&&Object.getPrototypeOf(o)===O.prototype"
        ),
        "true"
    );
}

#[test]
fn c646_native_error_proto_chain() {
    assert_eq!(eval("Object.getPrototypeOf(TypeError)===Error"), "true");
    assert_eq!(eval("Object.getPrototypeOf(RangeError)===Error"), "true");
    assert_eq!(
        eval("Object.getPrototypeOf(AggregateError)===Error"),
        "true"
    );
    assert_eq!(eval("TypeError.prototype.name"), "TypeError");
    assert_eq!(eval("new TypeError('m').message"), "m");
}

#[test]
fn c646_proxy_ownkeys_integrity() {
    // `Reflect.ownKeys` inclut `length` + symboles, strings d'abord.
    assert_eq!(
        eval("var s=Symbol();var a=[];a[s]=1;a.b=2;Reflect.ownKeys(a).map(String).join()"),
        "length,b,Symbol()"
    );
    // `isFrozen` proxy : gopd trappé pour chaque clé own (sym inclus).
    assert_eq!(
        eval(
            "var seen=[];var t={};var s=Symbol();t[s]=1;t.foo=2;t[0]=3;Object.freeze(t);\
             var p=new Proxy(t,{getOwnPropertyDescriptor(t,k){\
             seen.push(typeof k==='symbol'?'S':k);return Reflect.getOwnPropertyDescriptor(t,k)}});\
             Object.isFrozen(p);seen.join()"
        ),
        "0,foo,S"
    );
    // `freeze` proxy : def trappée `{configurable:false}` —
    // +`writable:false` pour les data props.
    assert_eq!(
        eval(
            "var seen={};var s=Symbol();\
             var p=new Proxy({[s]:1,get foo(){},set foo(v){}},\
             {defineProperty(t,k,d){seen[typeof k==='symbol'?'S':k]=d;return Reflect.defineProperty(t,k,d)}});\
             Object.freeze(p);seen.S.configurable+'|'+seen.S.writable+'|'+seen.foo.configurable"
        ),
        "false|false|false"
    );
}
