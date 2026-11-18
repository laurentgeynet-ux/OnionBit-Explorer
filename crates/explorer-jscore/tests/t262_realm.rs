// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Gate JS-C607 — objet hôte `$262` du harness test262 :
//! `createRealm` (realm nu à intrinsics frais via
//! `context_new_bare`), `evalScript` (évaluation **script** —
//! pas d'héritage strict — restore du contexte appelant même
//! sur throw), `detachArrayBuffer`, `global`.
//!
//! Verrous : intrinsics frais par realm (même heap, identités
//! distinctes), absence observable de `window`/`self`/`window[n]`
//! dans et autour du realm nu, jobs routés au realm
//! enregistreur, refus `TypeError` sur realm détruit.

use explorer_jscore::Engine;

fn ev(src: &str) -> String {
    let mut e = Engine::new();
    e.install_t262();
    match e.eval(src) {
        Ok(v) => format!("{v:?}"),
        Err(err) => format!("ERR {err}"),
    }
}

/// Surface de l'objet hôte : `$262` + ses membres, `global` =
/// `globalThis` du realm courant.
#[test]
fn surface_262() {
    assert_eq!(
        ev("typeof $262+'|'+typeof $262.createRealm+'|'+typeof $262.evalScript+'|'+typeof $262.detachArrayBuffer"),
        "Str(\"object|function|function|function\")"
    );
    assert_eq!(ev("$262.global===globalThis"), "Bool(true)");
    let r = ev("var r=$262.createRealm();typeof r+'|'+typeof r.global+'|'+typeof r.evalScript+'|'+typeof r.destroy");
    assert_eq!(r, "Str(\"object|object|function|function\")");
}

/// Intrinsics frais par realm : `Array` de l'un n'est pas `Array`
/// de l'autre, mais chaque realm est cohérent en interne.
#[test]
fn intrinsics_frais() {
    assert_eq!(
        ev("var r=$262.createRealm();(r.evalScript('Array')===Array)+'|'+r.evalScript('globalThis.Array===Array')+'|'+(r.evalScript('Object.getPrototypeOf([])')===Object.prototype)"),
        "Str(\"false|true|false\")"
    );
    // `realm.global` = le global du realm (identité stable).
    assert_eq!(
        ev("var r=$262.createRealm();r.global===r.evalScript('globalThis')"),
        "Bool(true)"
    );
    // Les objets traversent le même heap — utilisables cross-realm.
    assert_eq!(
        ev("var r=$262.createRealm();var o=r.evalScript('({x:42})');o.x+'|'+r.evalScript('var z={y:1};z').y"),
        "Str(\"42|1\")"
    );
}

/// Realm nu : ni `window`/`self`/`parent`/`postMessage` dedans
/// (`globalThis` reste — spec), ni `window[n]`/props ajoutées sur
/// le global parent.
#[test]
fn realm_nu() {
    assert_eq!(
        ev("var r=$262.createRealm();r.evalScript('typeof window+\\'|\\'+typeof self+\\'|\\'+typeof parent+\\'|\\'+typeof postMessage+\\'|\\'+typeof globalThis')"),
        "Str(\"undefined|undefined|undefined|undefined|object\")"
    );
    // Pas de prop énumérable « 0 » posée sur le global parent.
    assert_eq!(
        ev("var r=$262.createRealm();Object.keys(globalThis).includes('0')+'|'+('0' in globalThis)"),
        "Str(\"false|false\")"
    );
}

/// `evalScript` = évaluation **script** : le strict de l'appelant
/// ne s'hérite pas (un `delete` non qualifié — early-error
/// strict — est légal sloppy).
#[test]
fn eval_script_sloppy_meme_depuis_strict() {
    assert_eq!(
        ev("\"use strict\";var r=$262.createRealm();r.evalScript('var y=1;delete y;1')"),
        "Num(1.0)"
    );
    assert_eq!(
        ev("\"use strict\";$262.evalScript('var y=1;delete y;1')"),
        "Num(1.0)"
    );
    // Complétion rendue à l'appelant.
    assert_eq!(ev("$262.evalScript('40+2')"), "Num(42.0)");
}

/// Restore inconditionnel : un throw dans `evalScript` laisse le
/// moteur dans le realm appelant ; l'erreur porte les intrinsics
/// du realm fautif (`instanceof` cross-realm = false).
#[test]
fn restore_ctx_sur_throw() {
    assert_eq!(
        ev("var r=$262.createRealm();try{r.evalScript('throw new TypeError(\\'b\\')')}catch(e){e.constructor.name+'|'+Array.isArray([])+'|'+(this===globalThis)}"),
        "Str(\"TypeError|true|true\")"
    );
    // Erreur créée par les intrinsics DU REALM.
    assert_eq!(
        ev("var r=$262.createRealm();try{r.evalScript('throw new TypeError(\\'b\\')')}catch(e){e instanceof TypeError}"),
        "Bool(false)"
    );
    // SyntaxError de la source realm, pas de fuite de contexte.
    assert_eq!(
        ev("var r=$262.createRealm();try{r.evalScript('var var var')}catch(e){e.constructor.name+'|'+Object.keys(globalThis).includes('0')}"),
        "Str(\"SyntaxError|false\")"
    );
}

/// JS-C610 — `GetFunctionRealm` : les natives exécutent dans
/// leur realm de création (champ `realm` capturé à `install`),
/// `Bound`/`Proxy` descendent, le repli proto de
/// `OrdinaryCreateFromConstructor` lit les intrinsèques du
/// realm de `newTarget`.
#[test]
fn get_function_realm() {
    // `new otherRealm.Function()` minte la fn dans SON realm —
    // proto `otherRealm`'s `%Function.prototype%`, corps compilé
    // sous l'env `otherRealm`.
    assert_eq!(
        ev("var r=$262.createRealm();var F=r.evalScript('Function');var f=new F('return 40+2');(f instanceof F)+'|'+(f()===42)+'|'+(f instanceof Function)"),
        "Str(\"true|true|false\")"
    );
    // `%ThrowTypeError%` distinct par realm (spec intrinsèque).
    assert_eq!(
        ev("var r=$262.createRealm();var t1=Object.getOwnPropertyDescriptor(Function.prototype,'caller').get;var t2=r.evalScript('Object.getOwnPropertyDescriptor(Function.prototype,\"caller\").get');t1===t2"),
        "Bool(false)"
    );
    // Native ctor d'un autre realm : l'objet minté porte les
    // intrinsèques du realm du ctor, pas de l'appelant.
    assert_eq!(
        ev("var r=$262.createRealm();var OA=r.evalScript('Array');var a=new OA();(a instanceof OA)+'|'+(a instanceof Array)"),
        "Str(\"true|false\")"
    );
    // `Reflect.construct` avec `newTarget` cross-realm : proto
    // lu sur `newTarget` (realm distant).
    assert_eq!(
        ev("var r=$262.createRealm();var OA=r.evalScript('Array');var o=Reflect.construct(Array,[],OA);(o instanceof OA)+'|'+(o instanceof Array)"),
        "Str(\"true|false\")"
    );
    // Erreur mintée par une native realm-distante : intrinsèque
    // étranger → `instanceof` local faux.
    assert_eq!(
        ev("var r=$262.createRealm();var OT=r.evalScript('TypeError');var e;try{new OT('x')}catch(x){e=x};e instanceof TypeError"),
        "Bool(false)"
    );
}

/// `destroy()` : `evalScript` ultérieur refuse `TypeError`,
/// `global` du realm mort reste un objet (même heap, GC).
#[test]
fn destroy_realm() {
    assert_eq!(
        ev("var r=$262.createRealm();r.destroy();try{r.evalScript('1')}catch(e){e.constructor.name}"),
        "Str(\"TypeError\")"
    );
    assert_eq!(
        ev("var r=$262.createRealm();var g=r.global;r.destroy();typeof g+'|'+typeof g.Array"),
        "Str(\"object|function\")"
    );
}

/// Microtasks : un job enfilé depuis `evalScript` porte le realm
/// enregistreur — il tourne dans SON contexte au drain.
#[test]
fn microtasks_routees_au_realm() {
    let mut e = Engine::new();
    e.install_t262();
    e.eval("var r=$262.createRealm();r.evalScript('Promise.resolve(7).then(function(v){globalThis.__r=v;})');")
        .expect("eval");
    e.run_microtasks().expect("microtasks");
    let v = e.eval("r.evalScript('globalThis.__r')").expect("read");
    assert_eq!(format!("{v:?}"), "Num(7.0)");
}

/// `detachArrayBuffer` : détache in-place (`detached` + octets
/// vidés), `TypeError` sur non-ArrayBuffer, idempotent.
#[test]
fn detach_array_buffer() {
    assert_eq!(
        ev("var b=new ArrayBuffer(8);$262.detachArrayBuffer(b);b.byteLength+'|'+b.detached"),
        "Str(\"0|true\")"
    );
    assert_eq!(
        ev("var b=new ArrayBuffer(8);var t=new Uint8Array(b);$262.detachArrayBuffer(b);t.length+'|'+t.byteLength"),
        "Str(\"0|0\")"
    );
    assert_eq!(
        ev("var b=new ArrayBuffer(8);$262.detachArrayBuffer(b);$262.detachArrayBuffer(b);b.detached"),
        "Bool(true)"
    );
    assert_eq!(
        ev("try{$262.detachArrayBuffer({})}catch(e){e.constructor.name}"),
        "Str(\"TypeError\")"
    );
    // Cross-realm : buffer créé dans le realm, détaché depuis le
    // parent — les vues du realm voient le détachement.
    assert_eq!(
        ev("var r=$262.createRealm();var t=r.evalScript('var b=new ArrayBuffer(8);var t=new Uint8Array(b);$=undefined;t');$262.detachArrayBuffer(r.evalScript('b'));r.evalScript('t.length')"),
        "Num(0.0)"
    );
}
