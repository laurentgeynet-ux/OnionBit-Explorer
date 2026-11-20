// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! JS-C634 — `RegExp(pattern, flags)` spec : `IsRegExp`
//! (`@@match` observable), early-return sur
//! `SameValue(newTarget, pattern.constructor)`, regexp-like
//! via `Get(source)`/`Get(flags)`, `get source` sur
//! `%RegExp.prototype%` = `"(?:)"`, `flags` sur Symbol =
//! `TypeError`, pas de `@@toStringTag` sur le proto.

use explorer_jscore::Engine;

#[track_caller]
fn eval(src: &str) -> String {
    let mut e = Engine::new();
    e.install_t262();
    match e.eval(src) {
        Ok(v) => e.describe(v),
        Err(err) => format!("ERR {err}"),
    }
}

/// `RegExp(rx)` avec `rx.constructor === RegExp` et flags
/// absents rend l'entrée telle quelle (`SameValue`) — y compris
/// un objet non-RegExp portant `@@match` truthy.
#[test]
fn ctor_same_ctor_returns_input() {
    assert_eq!(eval(r"var r=/a/g; RegExp(r)===r"), "true");
    assert_eq!(
        eval(r"var o={constructor:RegExp,[Symbol.match]:true}; RegExp(o)===o"),
        "true"
    );
    // `constructor` lu par Get — même pour un objet non-RegExp
    // marqué `@@match`, et son accesseur peut jeter.
    assert_eq!(
        eval(r"try{RegExp({get constructor(){throw 7},[Symbol.match]:1})}catch(x){x}"),
        "7"
    );
    // ctor différent → nouvel objet depuis les props.
    assert_eq!(
        eval(
            r"var o={[Symbol.match]:true,source:'s',flags:'m',constructor:1};Object.getPrototypeOf(RegExp(o))===RegExp.prototype&&o!==RegExp(o)"
        ),
        "true"
    );
}

/// RegExp-like : `@@match` truthy de tout type → `Get(source)`
/// et `Get(flags)` ; `flags` explicite prime sans lire la
/// prop. Erreurs propagées.
#[test]
fn ctor_regexp_like() {
    assert_eq!(
        eval(r"RegExp({[Symbol.match]:86,source:'a+',flags:'i'}).source"),
        "a+"
    );
    assert_eq!(
        eval(r"RegExp({[Symbol.match]:true,source:'x',flags:'g'}).flags"),
        "g"
    );
    // Override : la prop `flags` n'est pas lue quand l'arg est
    // fourni.
    assert_eq!(
        eval(
            r"var n=0;var o={[Symbol.match]:true,source:'x',get flags(){n++;return 'u'}};RegExp(o,'g');n"
        ),
        "0"
    );
    assert_eq!(
        eval(r"try{RegExp({[Symbol.match]:1,get source(){throw 'SE'}})}catch(x){x}"),
        "SE"
    );
    // `@@match` falsy → objet quelconque → ToString.
    assert_eq!(
        eval(r"RegExp({[Symbol.match]:false,toString(){return 'zz'}}).test('zz')"),
        "true"
    );
}

/// Accesseurs `%RegExp.prototype%` : `source` = `"(?:)"`
/// (pas de slot), `flags` = `""` (`RegExpHasFlag` → undefined),
/// `flags` sur Symbol → `TypeError`. Pas de `@@toStringTag`.
#[test]
fn proto_this_val() {
    assert_eq!(eval(r"RegExp.prototype.source"), "(?:)");
    assert_eq!(eval(r"RegExp.prototype.flags"), "");
    assert_eq!(
        eval(r"try{RegExp.prototype.flags.call(Symbol())}catch(x){x.name}"),
        "TypeError"
    );
    assert_eq!(
        eval(r"Object.prototype.toString.call(RegExp.prototype)"),
        "[object Object]"
    );
    // Instances : comportement conservé.
    assert_eq!(
        eval(r"Object.prototype.toString.call(/x/)"),
        "[object RegExp]"
    );
    assert_eq!(eval(r"/abc/g.source"), "abc");
}
