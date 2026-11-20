// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! JS-C640 — String résiduel : coercions observables et
//! conversions numériques spec.
//!
//! - `ToPrimitive` via `GetMethod` : accesseurs `@@toPrimitive`
//!   déclenchés, non-callable → `TypeError` ; fast-path objet
//!   ordinaire seulement si `Object.prototype.toString`/
//!   `valueOf` sont toujours les natives d'origine.
//! - `ToIntegerOrInfinity` positions : `NaN` → +0, troncature
//!   vers zéro (`charAt(-0.9)` → index 0), `codePointAt(-1)` →
//!   `undefined` ; `lastIndexOf` `NaN` → +∞.
//! - `ToUint16` dans `String.fromCharCode` (mod 2¹⁶, non-fini → +0).
//! - `IsRegExp(searchString)` avant `ToString(search)` pour
//!   `includes`/`startsWith`/`endsWith` (`TypeError` si `@@match`
//!   truthy, getter propagé).
//! - `repeat`/`normalize` : `RequireObjectCoercible` + `ToString`
//!   du receiver **avant** la conversion du compte.
//! - `isWellFormed`/`toWellFormed` : vraie vérification UTF-16
//!   (surrogates isolés → `false`/`U+FFFD`).
//! - `StringGetOwnProperty` : index des wrappers `new String`
//!   `{w:false, e:true, c:false}` + `Object.keys`/descripteur.
//! - `arguments` : proto `%Object.prototype%`, tag "Arguments",
//!   `Array.isArray` → `false`, `@@iterator` = `Array.prototype.values`.
//! - `new String(Symbol())` → `TypeError` ; `String(Symbol())` →
//!   `"Symbol()"`.
//! - `String.raw` : `ToLength` (Symbol → `TypeError`) et
//!   substitution absente du dernier segment ; TRV `\r\n`→`\n`.
//! - `toLowerCase` : sigma final contextuel (`Σ` → `ς`).

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

/// `charAt`/`charCodeAt`/`codePointAt` : `ToInteger` exact —
/// `NaN` → +0, `-0.9` → -0 → index 0, négatif → hors bornes.
#[test]
fn char_positions_to_integer() {
    assert_eq!(eval("'abc'.charAt(-0.9)"), "a");
    assert_eq!(eval("''+('abc'.charCodeAt(NaN))"), "97");
    assert_eq!(eval("''+('abc'.codePointAt(-1))"), "undefined");
    // `lastIndexOf` : `NaN` → +∞ → recherche depuis la fin.
    assert_eq!(eval("'ABBABABAB'.lastIndexOf('AB', NaN)"), "7");
    assert_eq!(
        eval("'ABBABABAB'.lastIndexOf('AB', {valueOf(){return NaN}})"),
        "7"
    );
}

/// `String.fromCharCode` = `ToUint16` : mod 2¹⁶, non-fini → +0.
#[test]
fn from_char_code_to_uint16() {
    assert_eq!(eval("String.fromCharCode(Infinity).charCodeAt(0)"), "0");
    assert_eq!(eval("String.fromCharCode(-1).charCodeAt(0)"), "65535");
    assert_eq!(eval("String.fromCharCode(65536+65).charCodeAt(0)"), "65");
}

/// `includes`/`startsWith`/`endsWith` : `IsRegExp` → `TypeError`,
/// erreur du getter `@@match` propagée avant `ToString(search)`.
#[test]
fn search_methods_isregexp() {
    for m in ["includes", "startsWith", "endsWith"] {
        assert_eq!(
            eval(&format!(
                "try{{'abc'.{m}(/b/);'no'}}catch(e){{e.constructor.name}}"
            )),
            "TypeError"
        );
        assert_eq!(
            eval(&format!(
                "try{{'abc'.{m}({{[Symbol.match]:true}});'no'}}catch(e){{e.constructor.name}}"
            )),
            "TypeError"
        );
    }
    // Getter `@@match` fautif propagé avant `ToString(search)`.
    assert_eq!(
        eval(
            r"var o={get [Symbol.match](){throw 'BOM'},toString(){return 'x'}};try{'abc'.includes(o);'no'}catch(e){e}"
        ),
        "BOM"
    );
}

/// `repeat` : receiver coercé **avant** le compte — erreurs du
/// receiver (null → `TypeError`, Symbol → `TypeError`) précèdent le
/// `RangeError` éventuel du compte.
#[test]
fn repeat_receiver_first() {
    assert_eq!(
        eval("try{''.repeat.call(null,{valueOf(){throw 'X'}});'no'}catch(e){e.constructor.name}"),
        "TypeError"
    );
    assert_eq!(
        eval("try{String.prototype.repeat.call(Symbol(),0);'no'}catch(e){e.constructor.name}"),
        "TypeError"
    );
    assert_eq!(eval("'ab'.repeat(2)"), "abab");
}

/// `isWellFormed`/`toWellFormed` : vraie vérification UTF-16.
#[test]
fn well_formed_utf16() {
    assert_eq!(
        eval(
            "String.fromCharCode(0xD800).isWellFormed()+'/'+String.fromCharCode(0xDC00).isWellFormed()+'/'+('a').isWellFormed()+'/'+('ab').isWellFormed()"
        ),
        "false/false/true/true"
    );
    // Paire valide conservée, orphelin → U+FFFD.
    assert_eq!(
        eval("String.fromCharCode(0xD800).toWellFormed().charCodeAt(0)"),
        "65533"
    );
    assert_eq!(
        eval("String.fromCharCode(0xD800,0xDC00).toWellFormed().length"),
        "2"
    );
}

/// `StringGetOwnProperty` : les index d'un wrapper `new String`
/// sont own `{w:false, e:true, c:false}` — lectures, clés et
/// descripteurs alignés.
#[test]
fn string_exotic_indices() {
    assert_eq!(eval("var s=new String('abc');s[0]+s[1]+s[2]"), "abc");
    assert_eq!(eval("Object.keys(new String('abc')).join()"), "0,1,2");
    assert_eq!(
        eval(
            "var d=Object.getOwnPropertyDescriptor(new String('abc'),'0');[d.value,d.writable,d.enumerable,d.configurable].join()"
        ),
        "a,false,true,false"
    );
    // `arguments` : proto Object.prototype, tag Arguments, pas un
    // Array, `@@iterator` = `Array.prototype.values`.
    assert_eq!(
        eval("Array.isArray((function(){return arguments})(1))"),
        "false"
    );
    assert_eq!(
        eval("Object.prototype.toString.call((function(){return arguments})(1))"),
        "[object Arguments]"
    );
    assert_eq!(
        eval("Object.getPrototypeOf((function(){return arguments})(1))===Object.prototype"),
        "true"
    );
    assert_eq!(
        eval(
            "var a=(function(){return arguments})(7,8);var o=[];for(var x of a)o.push(x);o.join()"
        ),
        "7,8"
    );
}

/// `StringConstructor` : `String(Symbol)` → `"Symbol()"` à
/// l'appel ; `new String(Symbol())` → `TypeError` (`ToString`).
#[test]
fn string_ctor_symbol() {
    assert_eq!(eval("String(Symbol('x'))"), "Symbol(x)");
    assert_eq!(
        eval("try{new String(Symbol());'no'}catch(e){e.name}"),
        "TypeError"
    );
    // `Object.prototype.toString` réécrit : le fast-path
    // `ToPrimitive` ne doit pas court-circuiter la méthode.
    assert_eq!(
        eval("Object.prototype.toString=function(){return 'SHIFTED'};''+new String({})"),
        "SHIFTED"
    );
    // `@@toPrimitive` accesseur déclenché, non-callable → TypeError.
    assert_eq!(eval("var o={};o[Symbol.toPrimitive]=()=>'X';''+o"), "X");
    assert_eq!(
        eval(
            "try{var o={};Object.defineProperty(o,Symbol.toPrimitive,{value:42});''+o;'no'}catch(e){e.constructor.name}"
        ),
        "TypeError"
    );
}

/// `String.raw` : `ToLength` + pas de substitution sur le
/// dernier segment ; TRV `\r\n`→`\n`.
#[test]
fn string_raw_spec() {
    assert_eq!(eval("String.raw({raw:['a','b','c']},'X','Y','Z')"), "aXbYc");
    assert_eq!(
        eval("try{String.raw({raw:{length:Symbol()}});'no'}catch(e){e.constructor.name}"),
        "TypeError"
    );
    assert_eq!(eval("String.raw({raw:{length:'x'}},1)"), "");
    // TRV : `<CR><LF>` du source → `<LF>` dans le raw.
    assert_eq!(eval("String.raw`a\r\nb` === 'a\\nb'"), "true");
}

/// `toLowerCase` : sigma final contextuel (`Σ` → `ς` en fin de
/// mot après lettre cased, `σ` sinon) — `U+180E` (Cf) ignoré.
#[test]
fn lower_final_sigma() {
    assert_eq!(eval("'Σ'.toLowerCase()"), "σ");
    assert_eq!(eval("'AΣ'.toLowerCase()"), "aς");
    assert_eq!(eval("'ΣA'.toLowerCase()"), "σa");
    // `U+180E` (Case_Ignorable) ne change pas le contexte.
    assert_eq!(eval("'AΣ\u{180e}'.toLowerCase()"), "aς\u{180e}");
    assert_eq!(eval("'Σ\u{180e}B'.toLowerCase()"), "σ\u{180e}b");
}

/// `normalize` : `RequireObjectCoercible` → `TypeError` sur
/// `null`/`undefined` (pas `ToString`).
#[test]
fn normalize_coercible() {
    assert_eq!(
        eval("try{String.prototype.normalize.call(null);'no'}catch(e){e.constructor.name}"),
        "TypeError"
    );
    assert_eq!(
        eval("try{String.prototype.normalize.call(undefined);'no'}catch(e){e.constructor.name}"),
        "TypeError"
    );
}

/// `OrdinaryToPrimitive` : `Symbol` retourné par `valueOf` est
/// primitif → `ToNumber` lève `TypeError` (indexOf position).
#[test]
fn to_primitive_symbol_result() {
    assert_eq!(
        eval("try{'x'.indexOf('x',{valueOf:function(){return Symbol()}});'no'}catch(e){e.constructor.name}"),
        "TypeError"
    );
}

/// `for-in`/`getOwnPropertyNames` sur wrapper String : index
/// énumérables, `PRIM` interne jamais exposé.
#[test]
fn string_wrapper_enum() {
    assert_eq!(
        eval("var s=new String('abc');var k=[];for(var x in s)k.push(x);k.join(',')"),
        "0,1,2"
    );
    assert_eq!(
        eval("Object.getOwnPropertyNames(new String('abc')).join(',')"),
        "0,1,2,length"
    );
}

/// `Final_Sigma` : `Case_Ignorable` inclut les `MidNumLet` —
/// `A.Σ` → `a.ς` (FULL STOP saute dans le regard arrière).
#[test]
fn sigma_midnumlet() {
    assert_eq!(eval("'A.Σ'.toLowerCase()"), "a.ς");
    assert_eq!(eval("'AΣ.B'.toLowerCase()"), "aσ.b");
    assert_eq!(eval("'AΣ'.toLocaleLowerCase()"), "aς");
}

/// `@@match` non-u : `.` matche 1 code unit — les surrogates
/// isolés restent des units exactes (pas U+FFFD).
#[test]
fn match_lone_surrogate() {
    assert_eq!(
        eval("var m='𠮷a'.match(/./g);m.map(function(s){return s.charCodeAt(0).toString(16)}).join(',')"),
        "d842,dfb7,61"
    );
    assert_eq!(eval("'𠮷a'.match(/./g).length"), "3");
}

/// `repeat(NaN|0|fraction<1)` → `""` ; arités `pad*`=1 ;
/// `localeCompare` équiv. canonique (NFC).
#[test]
fn repeat_pad_locale() {
    assert_eq!(
        eval("'x'.repeat(NaN)+'|'+''.repeat('0')+'|'+''.repeat(0.9)+'|'+''.repeat(null)"),
        "|||"
    );
    assert_eq!(
        eval("String.prototype.padStart.length+'|'+String.prototype.padEnd.length"),
        "1|1"
    );
    assert_eq!(eval("'o\u{308}'.localeCompare('ö')"), "0");
}

/// `String.raw` : substitutions épuisées → segments suivants
/// servis quand même ; getter abrupt propagé.
#[test]
fn raw_missing_subs() {
    assert_eq!(
        eval("String.raw({raw:{length:5,0:'e',1:'',2:null,3:undefined,4:'5'}})"),
        "enullundefined5"
    );
    assert_eq!(
        eval("try{String.raw({raw:{length:2,'0':function(){throw 'X'}()}});'no'}catch(e){e}"),
        "X"
    );
}
