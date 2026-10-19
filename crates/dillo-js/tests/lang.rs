// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Conformance tests for the `dillo-js` ES3-subset engine.

use dillo_js::{Engine, Value};

#[track_caller]
fn ev(src: &str) -> Value {
    let mut e = Engine::new();
    e.eval(src).unwrap_or_else(|err| panic!("{src} -> {err}"))
}

#[track_caller]
fn ev_str(src: &str) -> String {
    let mut e = Engine::new();
    match e.eval(src) {
        Ok(v) => e.describe(v),
        Err(err) => panic!("{src} -> {err}"),
    }
}

#[track_caller]
fn num(v: Value) -> f64 {
    match v {
        Value::Num(n) => n,
        other => panic!("expected Num, got {other:?}"),
    }
}

// -- basics ---------------------------------------------------------

#[test]
fn arithmetic_and_precedence() {
    assert_eq!(num(ev("1 + 2 * 3")), 7.0);
    assert_eq!(num(ev("(1 + 2) * 3")), 9.0);
    assert_eq!(num(ev("10 % 3")), 1.0);
    assert_eq!(num(ev("2 * 3 - 4 / 2")), 4.0);
    assert_eq!(num(ev("-5 + +3")), -2.0);
}

#[test]
fn string_concat_and_coercion() {
    assert_eq!(ev_str("'a' + 'b'"), "ab");
    assert_eq!(ev_str("'x' + 1 + 2"), "x12");
    assert_eq!(ev_str("1 + 2 + 'x'"), "3x");
    assert_eq!(ev_str("'5' * 2"), "10");
    assert_eq!(ev_str("'5' - 2"), "3");
}

#[test]
fn equality_loose_and_strict() {
    assert_eq!(ev_str("1 == '1'"), "true");
    assert_eq!(ev_str("1 === '1'"), "false");
    assert_eq!(ev_str("null == undefined"), "true");
    assert_eq!(ev_str("null === undefined"), "false");
    assert_eq!(ev_str("NaN == NaN"), "false");
    assert_eq!(ev_str("'abc' == 'abc'"), "true");
}

#[test]
fn comparisons() {
    assert_eq!(ev_str("1 < 2"), "true");
    assert_eq!(ev_str("'b' > 'a'"), "true");
    assert_eq!(ev_str("'10' < '9'"), "true"); // lexicographic
    assert_eq!(ev_str("10 < 9"), "false");
    assert_eq!(ev_str("1 <= 1 && 2 >= 2"), "true");
}

#[test]
fn logical_short_circuit() {
    assert_eq!(ev_str("0 || 'fallback'"), "fallback");
    assert_eq!(ev_str("1 && 'yes'"), "yes");
    // short-circuit must not evaluate rhs
    assert_eq!(ev_str("var x = 0; false && (x = 1); x"), "0");
    assert_eq!(ev_str("var y = 0; true || (y = 1); y"), "0");
}

// -- variables, scoping ----------------------------------------------

#[test]
fn var_and_assign() {
    assert_eq!(ev_str("var x = 3; x = x + 1; x"), "4");
    assert_eq!(ev_str("var a = 1, b = 2; a + b"), "3");
    assert_eq!(ev_str("x = 5; x"), "5"); // implicit global
}

#[test]
fn hoisting() {
    assert_eq!(ev_str("x = 1; var x; x"), "1");
    assert_eq!(ev_str("typeof x"), "undefined");
    assert_eq!(ev_str("f(); function f() { return 7 }"), "7");
    assert_eq!(ev_str("var y = f(); function f() { return 9 } y"), "9");
}

#[test]
fn functions_and_closures() {
    assert_eq!(ev_str("function add(a, b) { return a + b } add(2, 3)"), "5");
    assert_eq!(
        ev_str("var add = function(a, b) { return a + b }; add(4, 5)"),
        "9"
    );
    assert_eq!(
        ev_str(
            "function counter() { var n = 0; return function() { n = n + 1; return n } } \
             var c = counter(); c(); c(); c()"
        ),
        "3"
    );
    assert_eq!(ev_str("(function() { return 42 })()"), "42");
    assert_eq!(
        ev_str("var f = function fib(n) { return n < 2 ? n : fib(n-1) + fib(n-2) }; f(10)"),
        "55"
    );
}

#[test]
fn recursion() {
    assert_eq!(
        ev_str("function fact(n) { return n <= 1 ? 1 : n * fact(n - 1) } fact(10)"),
        "3628800"
    );
}

// -- control flow ------------------------------------------------------

#[test]
fn loops() {
    assert_eq!(
        ev_str("var s = 0; for (var i = 0; i < 10; i++) { s += i } s"),
        "45"
    );
    assert_eq!(
        ev_str("var s = 0, i = 0; while (i < 5) { s += i; i++ } s"),
        "10"
    );
    assert_eq!(
        ev_str("var s = 0, i = 0; do { s += i; i++ } while (i < 5); s"),
        "10"
    );
    assert_eq!(
        ev_str("var s = 0; for (var i = 0; i < 10; i++) { if (i == 5) break; if (i % 2) continue; s += i } s"),
        "6" // 0+2+4
    );
}

#[test]
fn if_else_and_ternary() {
    assert_eq!(ev_str("if (1) 'yes'"), "yes");
    assert_eq!(ev_str("if (0) 'a' else 'b'"), "b");
    assert_eq!(ev_str("1 ? 't' : 'f'"), "t");
}

#[test]
fn switch() {
    assert_eq!(
        ev_str("var r=''; switch(2){case 1:r='a';break;case 2:r='b';break;default:r='d'} r"),
        "b"
    );
    // fallthrough
    assert_eq!(
        ev_str("var r=''; switch(1){case 1:r+='a';case 2:r+='b';break;default:r+='d'} r"),
        "ab"
    );
    // default
    assert_eq!(
        ev_str("var r=''; switch(9){case 1:r='a';break;default:r='d'} r"),
        "d"
    );
}

#[test]
fn asi() {
    assert_eq!(ev_str("var x = 1\nx + 1"), "2");
    assert_eq!(ev_str("var x = 1\nvar y = 2\nx + y"), "3");
    // return\n → undefined
    assert_eq!(ev_str("function f() { return\n5 } f()"), "undefined");
    // postfix must be same-line
    assert_eq!(ev_str("var i = 1; i++\ni"), "2");
}

// -- objects & arrays ---------------------------------------------------

#[test]
fn objects() {
    assert_eq!(ev_str("var o = {a: 1, b: 'x'}; o.a"), "1");
    assert_eq!(ev_str("var o = {}; o.x = 5; o.x"), "5");
    assert_eq!(ev_str("var o = {a: 1}; o['a']"), "1");
    assert_eq!(ev_str("var o = {a: 1}; 'a' in o"), "true");
    assert_eq!(ev_str("var o = {a: 1}; delete o.a; 'a' in o"), "false");
}

#[test]
fn arrays() {
    assert_eq!(ev_str("var a = [1, 2, 3]; a.length"), "3");
    assert_eq!(ev_str("var a = [1, 2, 3]; a[1]"), "2");
    assert_eq!(ev_str("var a = []; a.push(1); a.push(2); a.length"), "2");
    assert_eq!(ev_str("var a = [1,2]; a.pop(); a.length"), "1");
    assert_eq!(ev_str("var a = [1,2,3]; a.join('-')"), "1-2-3");
    assert_eq!(ev_str("var a = [3,1,2]; a.sort(); a.join(',')"), "1,2,3");
    assert_eq!(
        ev_str("var a = [3,1,2]; a.sort(function(x,y){return y-x}); a.join(',')"),
        "3,2,1"
    );
    assert_eq!(ev_str("var a = [1,2,3]; a.indexOf(2)"), "1");
    assert_eq!(ev_str("var a = [1,2,3]; a.slice(1).join(',')"), "2,3");
    assert_eq!(
        ev_str("var a = [1,2]; a.concat([3,4]).join(',')"),
        "1,2,3,4"
    );
}

#[test]
fn for_in() {
    assert_eq!(
        ev_str("var o = {a:1, b:2}; var s=''; for (var k in o) { s += k } s"),
        "ab"
    );
    assert_eq!(
        ev_str("var a = [10,20]; var s=0; for (var i in a) { s += a[i] } s"),
        "30"
    );
    // proto props must not appear
    assert_eq!(ev_str("var n = 0; for (var k in {}) { n++ } n"), "0");
}

#[test]
fn prototypes_and_new() {
    assert_eq!(
        ev_str("function P(x){ this.x = x } var p = new P(5); p.x"),
        "5"
    );
    assert_eq!(
        ev_str("function P(){} P.prototype.greet = function(){ return 'hi' }; var p = new P(); p.greet()"),
        "hi"
    );
    assert_eq!(
        ev_str("function P(){} var p = new P(); p instanceof P"),
        "true"
    );
    assert_eq!(ev_str("var a = []; a instanceof Array"), "true");
    assert_eq!(ev_str("var a = []; a instanceof Object"), "true");
}

#[test]
fn this_binding() {
    assert_eq!(
        ev_str("var o = {x: 3, get: function() { return this.x }}; o.get()"),
        "3"
    );
    // method extracted loses `this` → global (sloppy)
    assert_eq!(
        ev_str("var o = {x: 3, get: function() { return this.x }}; var g = o.get; g()"),
        "undefined"
    );
}

// -- exceptions -----------------------------------------------------------

#[test]
fn exceptions() {
    assert_eq!(
        ev_str("try { throw 'boom' } catch (e) { 'caught ' + e }"),
        "caught boom"
    );
    assert_eq!(ev_str("try { throw {code: 7} } catch (e) { e.code }"), "7");
    assert_eq!(
        ev_str("var r=''; try { throw 1 } catch (e) { r += 'c' } finally { r += 'f' } r"),
        "cf"
    );
    assert_eq!(
        ev_str("try { null.x } catch (e) { e instanceof TypeError }"),
        "true"
    );
    assert_eq!(
        ev_str("try { nosuchvar } catch (e) { e instanceof ReferenceError }"),
        "true"
    );
    assert_eq!(
        ev_str("try { throw new Error('oops') } catch (e) { e.message }"),
        "oops"
    );
}

#[test]
fn uncaught_error_is_host_error() {
    let mut e = Engine::new();
    match e.eval("throw 'x'") {
        Err(dillo_js::Error::Uncaught(m)) => assert_eq!(m, "x"),
        other => panic!("expected Uncaught, got {other:?}"),
    }
}

#[test]
fn syntax_error() {
    let mut e = Engine::new();
    assert!(matches!(
        e.eval("var = 3"),
        Err(dillo_js::Error::Syntax { .. })
    ));
    // Still rejected: `with` is out of the subset by design.
    assert!(matches!(
        e.eval("with (o) { x }"),
        Err(dillo_js::Error::Syntax { .. })
    ));
    assert!(matches!(
        e.eval("`tpl ${"),
        Err(dillo_js::Error::Syntax { .. })
    ));
}

// -- JS-7a: let/const, arrows, templates, ?./?? -------------------------------

#[test]
fn let_const_bindings() {
    assert_eq!(num(ev("let x = 5; x")), 5.0);
    assert_eq!(num(ev("const c = 7; c")), 7.0);
    assert_eq!(num(ev("let a = 1, b = 2; a + b")), 3.0);
    // Declared in the current scope — `{ x = 2 }` writes through.
    assert_eq!(num(ev("let x = 1; { x = 2; } x")), 2.0);
    // `const` write → `TypeError` (spec `SetMutableBinding`) —
    // l'ancienne tolérance silencieuse est corrigée en JS-C2.
    assert!(matches!(
        Engine::new().eval("const c = 1; c = 9"),
        Err(dillo_js::Error::Uncaught(_))
    ));
    assert_eq!(num(ev("const c = 1; try { c = 9; } catch(e) {} c")), 1.0);
    // `let` still works as a sloppy-mode identifier.
    assert_eq!(num(ev("var let = 3; let + 1")), 4.0);
    assert_eq!(ev_str("for (let i = 0; i < 3; i++) {} 'done'"), "done");
    assert_eq!(
        ev_str("var s = ''; for (const k in {a:1, b:2}) s += k; s"),
        "ab"
    );
}

#[test]
fn arrow_functions() {
    assert_eq!(num(ev("var f = x => x * 2; f(21)")), 42.0);
    assert_eq!(num(ev("var g = (a, b) => a + b; g(3, 4)")), 7.0);
    assert_eq!(num(ev("var h = () => 9; h()")), 9.0);
    assert_eq!(num(ev("var k = () => { return 8; }; k()")), 8.0);
    // Trailing comma in params.
    assert_eq!(num(ev("var t = (a,) => a; t(4)")), 4.0);
    // Arrow used as callback.
    assert_eq!(ev_str("[1,2,3].map(x => x * 10).join(',')"), "10,20,30");
    // Lexical this: arrow inside a method sees the method's receiver.
    assert_eq!(
        num(ev(
            "var o = { v: 5, get: function() { var f = () => this.v; return f(); } }; o.get()"
        )),
        5.0
    );
    // Ordinary functions keep dynamic this.
    assert_eq!(
        ev_str("var o2 = { v: 5, get: function() { var f = function() { return this.v; }; return f(); } }; typeof o2.get()"),
        "undefined"
    );
    // Arrows can't be constructed.
    assert!(Engine::new().eval("new (() => {})").is_err());
}

#[test]
fn template_literals() {
    assert_eq!(ev_str("`hello`"), "hello");
    assert_eq!(ev_str("var n = 3; `n=${n}`"), "n=3");
    assert_eq!(ev_str("`a${1+1}b${`c${2}`}d`"), "a2bc2d");
    assert_eq!(ev_str("`line1\nline2`"), "line1\nline2");
    assert_eq!(ev_str("`esc \\` tick \\${no}`"), "esc ` tick ${no}");
}

#[test]
fn optional_chaining_and_nullish() {
    assert_eq!(num(ev("var o = {a: {b: 7}}; o?.a?.b")), 7.0);
    assert_eq!(ev_str("var n = null; n?.x"), "undefined");
    assert_eq!(ev_str("var n = null; n?.x?.y"), "undefined");
    assert_eq!(num(ev("var a = [1]; a?.[0]")), 1.0);
    assert_eq!(ev_str("var f = null; f?.(1,2)"), "undefined");
    // `??` only fires on null/undefined, not other falsy values.
    assert_eq!(num(ev("0 ?? 5")), 0.0);
    assert_eq!(ev_str("'' ?? 'x'"), "");
    assert_eq!(ev_str("null ?? 'd'"), "d");
    assert_eq!(ev_str("undefined ?? 'd'"), "d");
    assert_eq!(num(ev("var u; u ?? 1 ?? 2")), 1.0);
}

#[test]
fn modern_operators() {
    assert_eq!(num(ev("2 ** 10")), 1024.0);
    assert_eq!(num(ev("2 ** 3 ** 2")), 512.0); // right-assoc
    assert_eq!(num(ev("var x = 3; x **= 2; x")), 9.0);
    assert_eq!(num(ev("var a; a ??= 4; a")), 4.0);
    assert_eq!(num(ev("var b = 1; b ??= 9; b")), 1.0);
    assert_eq!(num(ev("var c = 0; c ||= 6; c")), 6.0);
    assert_eq!(num(ev("var d = 5; d ||= 6; d")), 5.0);
    assert_eq!(num(ev("var e2 = 1; e2 &&= 7; e2")), 7.0);
    assert_eq!(num(ev("var e3 = 0; e3 &&= 7; e3")), 0.0);
    // Reserved words as member names / object keys.
    assert_eq!(
        num(ev(
            "var o = {function: 1, return: 2}; o.function + o.return"
        )),
        3.0
    );
    assert_eq!(ev_str("var o = {}; o.class = 'x'; o.class"), "x");
    // Shorthand + method syntax.
    assert_eq!(
        ev_str("var v = 3; var o = {v, m(x){ return x + v; }}; o.v + ':' + o.m(1)"),
        "3:4"
    );
}

// -- JS-7b: regex literals + RegExp + String methods ---------------------------

#[test]
fn regex_literals_and_test() {
    assert!(matches!(ev("/\\d+/.test('abc123')"), Value::Bool(true)));
    assert!(matches!(ev("/^a+$/.test('bbb')"), Value::Bool(false)));
    // Flags.
    assert!(matches!(ev("/ABC/i.test('abc')"), Value::Bool(true)));
    // `/` still divides after an operand.
    assert_eq!(num(ev("var x = 10; x / 2")), 5.0);
    assert_eq!(num(ev("var x = 10 / 2 / 5; x")), 1.0);
    // Division then regex in the same expression position.
    assert!(matches!(
        ev("var re = /b/; 'ab'.match(re).index"),
        Value::Num(1.0)
    ));
}

#[test]
fn regexp_exec_and_match() {
    // exec result: array + .index/.input.
    assert_eq!(
        ev_str("var m = /b(\\d)/.exec('a1b2'); m[0] + '|' + m[1] + '|' + m.index"),
        "b2|2|2"
    );
    // Global exec iterates via lastIndex.
    assert_eq!(
        ev_str("var r = /\\d/g; var out = ''; var m; while ((m = r.exec('a1b22c'))) { out += m[0] + '@' + m.index + ';' } out + r.lastIndex"),
        "1@1;2@3;2@4;0"
    );
    // match without /g → exec-shaped result.
    assert_eq!(ev_str("'foo123'.match(/\\d+/).index"), "3");
    // match with /g → all matched texts.
    assert_eq!(ev_str("'a1b22c3'.match(/\\d/g).join('-')"), "1-2-2-3");
    // No match → null.
    assert_eq!(ev_str("'abc'.match(/\\d/)"), "null");
    // new RegExp + clone flags.
    assert_eq!(
        ev_str("var r = new RegExp('a+', 'i'); r.test('AAA')"),
        "true"
    );
    assert_eq!(ev_str("/x/gi.source + '/' + /x/gi.flags"), "x/gi");
    assert_eq!(ev_str("(/a/i).toString()"), "/a/i");
    // search.
    assert_eq!(num(ev("'hello'.search(/l+/)")), 2.0);
    assert_eq!(num(ev("'hello'.search(/z/)")), -1.0);
}

#[test]
fn regex_replace_split() {
    // Literal string replace still first-only.
    assert_eq!(ev_str("'aaa'.replace('a', 'b')"), "baa");
    assert_eq!(ev_str("'aaa'.replaceAll('a', 'b')"), "bbb");
    // Regex replace: non-global first only, global all.
    assert_eq!(ev_str("'a1b2'.replace(/\\d/, 'X')"), "aXb2");
    assert_eq!(ev_str("'a1b2'.replace(/\\d/g, 'X')"), "aXbX");
    // $-substitutions.
    assert_eq!(ev_str("'abc'.replace(/b/, \"[$`|$&|$']\")"), "a[a|b|c]c");
    assert_eq!(
        ev_str("'john smith'.replace(/(\\w+) (\\w+)/, '$2 $1')"),
        "smith john"
    );
    assert_eq!(ev_str("'abc'.replace(/(?<m>b)/, '[$<m>]')"), "a[b]c");
    // Function replacement.
    assert_eq!(
        ev_str("'a1b22'.replace(/\\d+/g, function(m) { return '<' + m + '>'; })"),
        "a<1>b<22>"
    );
    // Split by regex, captures inserted between pieces.
    assert_eq!(ev_str("'a1b22c'.split(/\\d+/).join('|')"), "a|b|c");
    assert_eq!(ev_str("'a1b2'.split(/(\\d)/).join('|')"), "a|1|b|2|");
    // Split with limit.
    assert_eq!(ev_str("'a,b,c'.split(',', 2).join('|')"), "a|b");
    // Invalid regex → SyntaxError (catchable).
    assert_eq!(
        ev_str("try { new RegExp('['); 'no' } catch (e) { e.name }"),
        "SyntaxError"
    );
    // Lookahead/backrefs : compilés depuis fancy-regex (JS-C3).
    assert_eq!(ev_str("/x(?=y)/.test('xy')"), "true");
    assert_eq!(ev_str("/x(?!y)/.test('xz')"), "true");
    assert_eq!(ev_str("/(?<=a)b/.test('ab')"), "true");
    assert_eq!(ev_str("/(\\d)\\1/.test('44')"), "true");
    assert_eq!(ev_str("/(?<n>ab)cd\\k<n>/.test('abcdab')"), "true");
}

#[test]
fn regexp_js_c3_semantics() {
    // Backref nommée avant sa déclaration → match vide (21.2.2.9).
    assert_eq!(ev_str("/\\k<a>(?<a>x)/.test('x')"), "true");
    // `\k` identity escape seulement en sloppy sans groupe nommé.
    assert_eq!(ev_str("/\\k/.test('k')"), "true");
    assert_eq!(
        ev_str("try { eval('/(?<a>.)\\\\k/'); 'no' } catch (e) { e.name }"),
        "SyntaxError"
    );
    assert_eq!(
        ev_str("try { eval('/\\\\k<>/u'); 'no' } catch (e) { e.name }"),
        "SyntaxError"
    );
    // Folding `i` non-`u` = ToUppercase simple : KELVIN U+212A ne
    // matche ni 'k' ni 'K', mais 'k' et 'K' se matchent entre eux.
    assert_eq!(ev_str("/\\u212a/i.test('k')"), "false");
    assert_eq!(ev_str("/\\u212a/i.test('K')"), "false");
    assert_eq!(ev_str("/\\u212a/i.test('\\u212a')"), "true");
    assert_eq!(ev_str("/k/i.test('K')"), "true");
    assert_eq!(ev_str("/K/i.test('k')"), "true");
    assert_eq!(ev_str("/[a-z]+/i.test('ABC')"), "true");
    // Quantificateur sans atome → SyntaxError.
    assert_eq!(
        ev_str("try { eval('/{2}/'); 'no' } catch (e) { e.name }"),
        "SyntaxError"
    );
    // Groupes nommés : doublon et nom invalide → SyntaxError.
    assert_eq!(
        ev_str("try { eval('/(?<a>x)(?<a>y)/'); 'no' } catch (e) { e.name }"),
        "SyntaxError"
    );
    assert_eq!(
        ev_str("try { eval('/(?<1a>x)/'); 'no' } catch (e) { e.name }"),
        "SyntaxError"
    );
    // Modificateurs inline ES2025 : forme valide acceptée, `(?u:` rejeté.
    assert_eq!(ev_str("/(?i:a)b/.test('Ab')"), "true");
    assert_eq!(
        ev_str("try { eval('/(?u:a)/'); 'no' } catch (e) { e.name }"),
        "SyntaxError"
    );
    // Backref numérique hors bornes → Annex B octale/identité.
    assert_eq!(ev_str("/\\8/.test('8')"), "true");
    assert_eq!(
        ev_str("try { eval('/\\\\8/u'); 'no' } catch (e) { e.name }"),
        "SyntaxError"
    );
}

// -- builtins ---------------------------------------------------------------

#[test]
fn string_methods() {
    assert_eq!(ev_str("'hello'.toUpperCase()"), "HELLO");
    assert_eq!(ev_str("'HELLO'.toLowerCase()"), "hello");
    assert_eq!(ev_str("'hello'.charAt(1)"), "e");
    assert_eq!(ev_str("'hello'.charCodeAt(0)"), "104");
    assert_eq!(ev_str("'hello'.indexOf('l')"), "2");
    assert_eq!(ev_str("'hello'.lastIndexOf('l')"), "3");
    assert_eq!(ev_str("'hello'.slice(1, 3)"), "el");
    assert_eq!(ev_str("'hello'.substring(1, 3)"), "el");
    assert_eq!(ev_str("'hello'.substr(1, 3)"), "ell");
    assert_eq!(ev_str("' hello '.trim()"), "hello");
    assert_eq!(ev_str("'a,b,c'.split(',').join('|')"), "a|b|c");
    assert_eq!(ev_str("'hello'.replace('l', 'L')"), "heLlo");
    assert_eq!(ev_str("'abc'.length"), "3");
    assert_eq!(ev_str("String.fromCharCode(72, 105)"), "Hi");
    assert_eq!(ev_str("'abc'[1]"), "b");
}

#[test]
fn number_builtins() {
    assert_eq!(ev_str("parseInt('42px')"), "42");
    assert_eq!(ev_str("parseInt('0x1f')"), "31");
    assert_eq!(ev_str("parseInt('10', 2)"), "2");
    assert_eq!(ev_str("parseFloat('1.5x')"), "1.5");
    assert_eq!(ev_str("isNaN(NaN)"), "true");
    assert_eq!(ev_str("Number('3.5')"), "3.5");
    assert_eq!(ev_str("(255).toString(16)"), "ff");
    assert_eq!(ev_str("(1.005).toFixed(2)"), "1.00");
    assert_eq!(ev_str("String(42)"), "42");
}

#[test]
fn math() {
    assert_eq!(ev_str("Math.floor(1.9)"), "1");
    assert_eq!(ev_str("Math.ceil(1.1)"), "2");
    assert_eq!(ev_str("Math.round(2.5)"), "3");
    assert_eq!(ev_str("Math.abs(-3)"), "3");
    assert_eq!(ev_str("Math.max(1, 5, 3)"), "5");
    assert_eq!(ev_str("Math.min(1, 5, 3)"), "1");
    assert_eq!(ev_str("Math.pow(2, 10)"), "1024");
    assert_eq!(ev_str("Math.sqrt(16)"), "4");
    let r = num(ev("Math.random()"));
    assert!((0.0..1.0).contains(&r));
}

#[test]
fn array_higher_order() {
    assert_eq!(
        ev_str("var s=0; [1,2,3].forEach(function(x){ s += x }); s"),
        "6"
    );
    assert_eq!(
        ev_str("[1,2,3].map(function(x){ return x*2 }).join(',')"),
        "2,4,6"
    );
    assert_eq!(
        ev_str("[1,2,3,4].filter(function(x){ return x%2==0 }).join(',')"),
        "2,4"
    );
    assert_eq!(
        ev_str("[1,2,3].reduce(function(a,b){ return a+b }, 0)"),
        "6"
    );
}

#[test]
fn typeof_and_misc() {
    assert_eq!(ev_str("typeof 1"), "number");
    assert_eq!(ev_str("typeof 's'"), "string");
    assert_eq!(ev_str("typeof true"), "boolean");
    assert_eq!(ev_str("typeof undefined"), "undefined");
    assert_eq!(ev_str("typeof null"), "object");
    assert_eq!(ev_str("typeof {}"), "object");
    assert_eq!(ev_str("typeof function(){}"), "function");
    assert_eq!(ev_str("typeof nosuch"), "undefined");
    assert_eq!(ev_str("void 0 === undefined"), "true");
    assert_eq!(ev_str("var a=[5]; a instanceof Object"), "true");
}

#[test]
fn globals() {
    assert_eq!(ev_str("window === this"), "true");
    assert_eq!(ev_str("window.Math === Math"), "true");
    assert_eq!(ev_str("escape('a b')"), "a%20b");
    assert_eq!(ev_str("unescape('a%20b')"), "a b");
    assert_eq!(ev_str("encodeURIComponent('a b')"), "a%20b");
    assert_eq!(ev_str("decodeURIComponent('a%20b')"), "a b");
    assert_eq!(ev_str("btoa('abc')"), "YWJj");
    assert_eq!(ev_str("atob('YWJj')"), "abc");
    assert_eq!(ev_str("typeof Date.now()"), "number");
}

// -- safety rails ---------------------------------------------------------

#[test]
fn op_limit_kills_infinite_loop() {
    let mut e = Engine::new();
    e.set_op_limit(100_000);
    match e.eval("while (true) {}") {
        Err(dillo_js::Error::Uncaught(m)) => assert!(m.contains("execution limit")),
        other => panic!("expected Uncaught, got {other:?}"),
    }
}

#[test]
fn deep_recursion_throws() {
    let mut e = Engine::new();
    match e.eval("function f() { f() } f()") {
        Err(dillo_js::Error::Uncaught(m)) => assert!(m.contains("stack")),
        other => panic!("expected Uncaught, got {other:?}"),
    }
}

// -- real-world-ish script shapes ----------------------------------------

#[test]
fn jquery_era_shapes() {
    // Typical validation/DOM-toggle era script.
    assert_eq!(
        ev_str(
            "var form = {email: '', ok: false}; \
             function validate(f) { \
               if (f.email.indexOf('@') < 0) return false; \
               f.ok = true; return true; \
             } \
             form.email = 'a@b.c'; \
             validate(form) ? form.ok : 'bad'"
        ),
        "true"
    );
    assert_eq!(
        ev_str(
            "var lis = ['a','b','c']; var out = ''; \
             for (var i = 0; i < lis.length; i++) { \
               out += '<li>' + lis[i] + '</li>'; \
             } out"
        ),
        "<li>a</li><li>b</li><li>c</li>"
    );
    // sprintf-ish util
    assert_eq!(
        ev_str(
            "function pad(n, w) { var s = '' + n; while (s.length < w) s = '0' + s; return s } \
             pad(7, 3) + '-' + pad(42, 3)"
        ),
        "007-042"
    );
}

// -- Promise / microtasks (JS-7c) --------------------------------------

/// `setup` runs (microtasks drain at its end — `eval` is a job
/// boundary), then `check` observes the post-microtask world.
#[track_caller]
fn ev_seq(setup: &str, check: &str) -> Value {
    let mut e = Engine::new();
    e.eval(setup)
        .unwrap_or_else(|err| panic!("setup {setup} -> {err}"));
    e.eval(check)
        .unwrap_or_else(|err| panic!("check {check} -> {err}"))
}

#[track_caller]
fn seq_str(setup: &str, check: &str) -> String {
    let mut e = Engine::new();
    e.eval(setup)
        .unwrap_or_else(|err| panic!("setup {setup} -> {err}"));
    match e.eval(check) {
        Ok(v) => e.describe(v),
        Err(err) => panic!("check {check} -> {err}"),
    }
}

#[test]
fn promise_resolve_then() {
    // resolve → then handler runs as a drained microtask.
    assert_eq!(
        num(ev_seq(
            "var r = 0; new Promise(function(res) { res(42) }).then(function(v) { r = v * 2 })",
            "r"
        )),
        84.0
    );
    // then chains: the handler's return becomes the next input.
    assert_eq!(
        num(ev_seq(
            "var r = 0; Promise.resolve(1)\
               .then(function(v) { return v + 1 })\
               .then(function(v) { return v * 10 })\
               .then(function(v) { r = v })",
            "r"
        )),
        20.0
    );
    // Pass-through: a then without onFulfilled forwards the value.
    assert_eq!(
        num(ev_seq(
            "var r = 0; Promise.resolve(7).then().then(function(v) { r = v })",
            "r"
        )),
        7.0
    );
}

#[test]
fn promise_reject_catch() {
    assert_eq!(
        seq_str(
            "var r = ''; new Promise(function(res, rej) { rej('boom') })\
               .catch(function(e) { r = 'caught:' + e })",
            "r"
        ),
        "caught:boom"
    );
    // Executor throwing == rejecting.
    assert_eq!(
        seq_str(
            "var r = ''; new Promise(function() { throw 'kaboom' })\
               .catch(function(e) { r = e })",
            "r"
        ),
        "kaboom"
    );
    // A throwing onFulfilled rejects the chained promise.
    assert_eq!(
        seq_str(
            "var r = ''; Promise.resolve(1)\
               .then(function() { throw 'mid' })\
               .catch(function(e) { r = 'c:' + e })",
            "r"
        ),
        "c:mid"
    );
    // Promise.reject static.
    assert_eq!(
        seq_str(
            "var r = ''; Promise.reject('no').catch(function(e) { r = e })",
            "r"
        ),
        "no"
    );
}

#[test]
fn promise_finally() {
    // finally runs on fulfill and forwards the value.
    assert_eq!(
        seq_str(
            "var log = ''; Promise.resolve('v')\
               .finally(function() { log += 'F' })\
               .then(function(v) { log += ':' + v })",
            "log"
        ),
        "F:v"
    );
    // finally runs on reject; the reason still propagates.
    assert_eq!(
        seq_str(
            "var log = ''; Promise.reject('r')\
               .finally(function() { log += 'F' })\
               .catch(function(e) { log += ':' + e })",
            "log"
        ),
        "F:r"
    );
}

#[test]
fn promise_adoption() {
    // Resolving with a promise adopts its state.
    assert_eq!(
        num(ev_seq(
            "var r = 0; new Promise(function(res) {\
               res(new Promise(function(r2) { r2(9) }))\
             }).then(function(v) { r = v })",
            "r"
        )),
        9.0
    );
    // Resolving with a thenable calls its `then` as a microtask.
    assert_eq!(
        num(ev_seq(
            "var r = 0; Promise.resolve({ then: function(res) { res(5) } })\
               .then(function(v) { r = v })",
            "r"
        )),
        5.0
    );
}

#[test]
fn promise_combinators() {
    // all: ordered results.
    assert_eq!(
        seq_str(
            "var r = ''; Promise.all([Promise.resolve('a'), 'b', Promise.resolve('c')])\
               .then(function(vs) { r = vs.join('-') })",
            "r"
        ),
        "a-b-c"
    );
    // all: first rejection wins.
    assert_eq!(
        seq_str(
            "var r = ''; Promise.all([Promise.resolve(1), Promise.reject('x')])\
               .catch(function(e) { r = 'E:' + e })",
            "r"
        ),
        "E:x"
    );
    // race: first *registered* settle wins (both already settled →
    // iteration order decides, spec-conformant).
    assert_eq!(
        num(ev_seq(
            "var r = 0; Promise.race([Promise.resolve(3), Promise.reject('l')])\
               .then(function(v) { r = v }, function() { r = -1 })",
            "r"
        )),
        3.0
    );
    // allSettled: never rejects, status objects.
    assert_eq!(
        seq_str(
            "var r = ''; Promise.allSettled([Promise.resolve(1), Promise.reject('x')])\
               .then(function(vs) {\
                 r = vs[0].status + ':' + vs[0].value + ',' + vs[1].status\
               })",
            "r"
        ),
        "fulfilled:1,rejected"
    );
    // any: first fulfillment wins over rejections.
    assert_eq!(
        num(ev_seq(
            "var r = 0; Promise.any([Promise.reject('a'), Promise.resolve(2)])\
               .then(function(v) { r = v })",
            "r"
        )),
        2.0
    );
}

#[test]
fn microtask_ordering() {
    // Microtasks run after synchronous code of the same job.
    assert_eq!(
        seq_str(
            "var s = ''; Promise.resolve().then(function() { s += 'micro' }); s += 'sync'",
            "s"
        ),
        "syncmicro"
    );
    // queueMicrotask queues behind the current job too.
    assert_eq!(
        seq_str(
            "var s = ''; queueMicrotask(function() { s += 'q' }); s += 'a'",
            "s"
        ),
        "aq"
    );
    // Microtasks queued *by* microtasks drain in the same checkpoint.
    assert_eq!(
        seq_str(
            "var s = ''; Promise.resolve().then(function() {\
               s += '1'; Promise.resolve().then(function() { s += '2' })\
             })",
            "s"
        ),
        "12"
    );
}

#[test]
fn promise_unhandled_rejection_is_silent() {
    // No crash, no uncaught — the page keeps running.
    assert_eq!(
        ev_str("Promise.reject('nobody listens'); 'still alive'"),
        "still alive"
    );
    // Non-function executor → TypeError at construction.
    assert!(Engine::new().eval("new Promise(42)").is_err());
}

// -- JS-7d : destructuring, for-of, spread/rest, classes --------------

#[test]
fn destructuring_declarations() {
    // Array patterns: holes, defaults, rest, nested.
    assert_eq!(num(ev("var [a, , b] = [1, 2, 3]; a * 10 + b")), 13.0);
    assert_eq!(num(ev("var [x = 9] = []; x")), 9.0);
    assert_eq!(
        num(ev("var [h, ...t] = [1, 2, 3]; t.length * 10 + t[0]")),
        22.0
    );
    assert_eq!(num(ev("var [[a], {b}] = [[7], {b: 5}]; a + b")), 12.0);
    // Object patterns: shorthand, rename, default, rest, computed.
    assert_eq!(num(ev("var {p, q} = {p: 3, q: 4}; p * q")), 12.0);
    assert_eq!(num(ev("var {p: r} = {p: 8}; r")), 8.0);
    assert_eq!(num(ev("var {p = 6} = {}; p")), 6.0);
    assert_eq!(
        ev_str("var {keep, ...rest} = {keep: 1, a: 2, b: 3}; rest.a + '' + rest.b + (rest.keep === undefined ? '!' : '?')"),
        "23!"
    );
    // let/const destructure too; const keeps its guard.
    assert_eq!(num(ev("const [c1, c2] = [4, 5]; c1 + c2")), 9.0);
    assert_eq!(num(ev("let {z} = {z: 11}; z")), 11.0);
    // `let` remains a plain identifier (contextual keyword).
    assert_eq!(num(ev("var let = 5; let")), 5.0);
}

#[test]
fn destructuring_assignment_and_params() {
    // `({a, b} = o)` — needs parens, assigns existing bindings.
    assert_eq!(
        num(ev("var a, b; ({a, b} = {a: 2, b: 3}); a * 10 + b")),
        23.0
    );
    // `[x, y] = arr` — expression statement.
    assert_eq!(num(ev("var x, y; [x, y] = [5, 6]; x + y")), 11.0);
    // Property targets through destructuring.
    assert_eq!(num(ev("var o = {}; ({v: o.v} = {v: 42}); o.v")), 42.0);
    // Parameter destructuring + defaults + rest.
    assert_eq!(
        num(ev("function f({a, b = 10}) { return a + b } f({a: 1})")),
        11.0
    );
    assert_eq!(num(ev("var f = ([x, y = 2]) => x + y; f([9])")), 11.0);
    assert_eq!(
        num(ev(
            "function g(a, ...rest) { return a * 100 + rest.length * 10 + rest[0] } g(1, 2, 3)"
        )),
        122.0
    );
    assert_eq!(num(ev("function h(a, b = a * 2) { return b } h(5)")), 10.0);
    // catch with a pattern (ES2019 optional binding too).
    assert_eq!(
        ev_str("try { throw {code: 7} } catch ({code}) { 'code=' + code }"),
        "code=7"
    );
}

#[test]
fn for_of_and_spread() {
    // for-of on arrays and strings.
    assert_eq!(
        num(ev("var s = 0; for (var v of [1, 2, 3]) s += v; s")),
        6.0
    );
    assert_eq!(
        ev_str("var s = ''; for (const c of 'abc') s += c; s"),
        "abc"
    );
    // for-of with destructuring target.
    assert_eq!(
        num(ev(
            "var s = 0; for (var [a, b] of [[1, 2], [3, 4]]) s += a * 10 + b; s"
        )),
        46.0
    );
    // Custom iterable via Symbol.iterator-ish hook (sentinel key).
    assert_eq!(
        num(ev("var it = { vals: [5, 6], '\u{f130}Symbol.iterator': function() { var i = 0, vs = this.vals; return { next: function() { return i < vs.length ? {value: vs[i++], done: false} : {done: true} } } } }; var s = 0; for (var v of it) s += v; s")),
        11.0
    );
    // Spread in array literals and calls.
    assert_eq!(
        num(ev("var a = [1, ...[2, 3], 4]; a.length * 10 + a[2]")),
        43.0
    );
    assert_eq!(
        num(ev(
            "function m(a, b, c) { return a * 100 + b * 10 + c } m(...[1, 2, 3])"
        )),
        123.0
    );
    assert_eq!(
        ev_str("var o = {a: 1}; var o2 = {...o, b: 2}; o2.a + ':' + o2.b"),
        "1:2"
    );
    // Not iterable → TypeError, catchable.
    assert_eq!(
        ev_str("try { for (var x of 42) {} 'bad' } catch (e) { 'caught' }"),
        "caught"
    );
}

#[test]
fn classes_basics() {
    // Declaration + constructor + methods + instanceof.
    assert_eq!(
        num(ev(
            "class Point { constructor(x, y) { this.x = x; this.y = y } sum() { return this.x + this.y } }\
             var p = new Point(3, 4); p.sum()"
        )),
        7.0
    );
    assert_eq!(
        ev_str("class A {} var a = new A(); (a instanceof A) + ':' + typeof A"),
        "true:function"
    );
    // Class expression + accessors.
    assert_eq!(
        num(ev(
            "var T = class { constructor() { this._v = 5 } get v() { return this._v * 2 } set v(x) { this._v = x } };\
             var t = new T(); t.v + 0"
        )),
        10.0
    );
    assert_eq!(
        num(ev(
            "var T = class { constructor() { this._v = 1 } get v() { return this._v * 2 } set v(x) { this._v = x } };\
             var t = new T(); t.v = 9; t._v"
        )),
        9.0
    );
    // `new` on a class-less arrow still refuses.
    assert!(Engine::new()
        .eval("new (class { constructor() {} })()")
        .is_ok());
}

#[test]
fn classes_inheritance_and_super() {
    // `extends` + `super()` in the derived ctor + `super.m()`.
    assert_eq!(
        ev_str(
            "class Base { constructor(v) { this.v = v } describe() { return 'B' + this.v } }\
             class Kid extends Base { constructor(v) { super(v * 10) } describe() { return 'K' + super.describe() } }\
             var k = new Kid(4); k.describe()"
        ),
        "KB40"
    );
    // Derived instanceof both links.
    assert_eq!(
        ev_str(
            "class B {} class K extends B {} var k = new K();\
             (k instanceof K) + ':' + (k instanceof B)"
        ),
        "true:true"
    );
    // Getter through super + static members.
    assert_eq!(
        ev_str(
            "class S { static make() { return 'made' } }\
             class S2 extends S {} S2.make()"
        ),
        "made"
    );
    // Default derived ctor forwards args to super.
    assert_eq!(
        num(ev("class P { constructor(a, b) { this.n = a + b } }\
             class C extends P {} new C(2, 3).n")),
        5.0
    );
}

#[test]
fn async_fn_and_await() {
    // `async` wraps the return value in a settled promise.
    assert_eq!(
        seq_str(
            "var r; async function f() { return 7 } f().then(function(v) { r = v })",
            "r"
        ),
        "7"
    );
    // `await` on a fulfilled promise unwraps it; on a plain value
    // returns it.
    assert_eq!(
        seq_str(
            "var r; async function g() { var v = await Promise.resolve(21); return v * 2 } g().then(function(x) { r = x })",
            "r"
        ),
        "42"
    );
    // Rejection inside async → rejected promise → catch.
    assert_eq!(
        seq_str(
            "var r; async function h() { throw 'boom' } h().catch(function(e) { r = 'c:' + e })",
            "r"
        ),
        "c:boom"
    );
    // `await` on a pending promise → runtime TypeError (no
    // continuations — documented deviation).
    assert_eq!(
        ev_str(
            "var p = new Promise(function(res, rej) {});\
             async function w() { return await p }\
             var out; w().then(function(v) { out = 'ok' }, function(e) { out = 'rejected' });\
             out === undefined ? 'pending-handled' : out"
        ),
        "pending-handled"
    );
}

#[test]
fn object_statics() {
    // defineProperty — data descriptor.
    assert_eq!(
        num(ev(
            "var o = {}; Object.defineProperty(o, 'x', {value: 7, enumerable: true}); o.x"
        )),
        7.0
    );
    // Non-enumerable hides from keys().
    assert_eq!(
        ev_str("var o = {}; Object.defineProperty(o, 'h', {value: 1}); o.visible = 2; Object.keys(o).join(',')"),
        "visible"
    );
    // Accessor descriptor fires the getter.
    assert_eq!(
        num(ev(
            "var o = {n: 5}; Object.defineProperty(o, 'dbl', {get: function() { return this.n * 2 }}); o.dbl"
        )),
        10.0
    );
    // getOwnPropertyDescriptor round-trips both kinds.
    assert_eq!(
        ev_str(
            "var o = {}; Object.defineProperty(o, 'x', {value: 3});\
             var d = Object.getOwnPropertyDescriptor(o, 'x');\
             d.value + ':' + d.enumerable"
        ),
        "3:false"
    );
    assert_eq!(
        ev_str(
            "var o = {}; Object.defineProperty(o, 'g', {get: function() { return 1 }});\
             typeof Object.getOwnPropertyDescriptor(o, 'g').get"
        ),
        "function"
    );
    // create + proto chain.
    assert_eq!(
        num(ev(
            "var p = {n: 4}; var c = Object.create(p); c.n + (c instanceof Object ? 0 : 100)"
        )),
        4.0
    );
    // assign copies enumerable own props, accessors read values.
    assert_eq!(
        ev_str(
            "var t = {}; Object.assign(t, {a: 1, b: 2}, {c: 3});\
             t.a + ':' + t.b + ':' + t.c"
        ),
        "1:2:3"
    );
    // values mirrors keys.
    assert_eq!(ev_str("Object.values({x: 9, y: 8}).join('|')"), "9|8");
}

// -- Map / Set / JSON (JS-7e) -----------------------------------------

#[test]
fn map_basics() {
    assert_eq!(
        ev_str(
            "var m = new Map();\
             m.set('a', 1); m.set('b', 2);\
             m.size + '|' + m.get('a') + '|' + m.has('b') + '|' + m.has('z')"
        ),
        "2|1|true|false"
    );
    assert_eq!(
        ev_str("var m = new Map(); m.set('a', 1); m.delete('a'); m.size + '|' + m.get('a')"),
        "0|undefined"
    );
    // `set` returns the map (chainable); `clear` empties.
    assert_eq!(
        ev_str("var m = new Map(); m.set('x', 1).set('y', 2).size"),
        "2"
    );
    assert_eq!(
        ev_str("var m = new Map(); m.set('x', 1); m.clear(); m.size"),
        "0"
    );
    // Overwriting an existing key keeps insertion order.
    assert_eq!(
        ev_str("var m = new Map(); m.set('a',1); m.set('a',9); m.get('a')"),
        "9"
    );
}

#[test]
fn map_ctor_and_identity_keys() {
    // Iterable-of-pairs ctor.
    assert_eq!(
        ev_str("var m = new Map([['x', 1], ['y', 2]]); m.get('x') + '/' + m.get('y')"),
        "1/2"
    );
    // Object keys are by identity — a fresh {} is not found.
    assert_eq!(
        ev_str(
            "var k = {}; var m = new Map(); m.set(k, 7);\
                m.get(k) + '|' + m.get({})"
        ),
        "7|undefined"
    );
}

#[test]
fn map_same_value_zero() {
    // NaN is a findable key (SameValueZero, unlike ===).
    assert_eq!(
        ev_str("var m = new Map(); m.set(NaN, 'nan'); m.get(NaN)"),
        "nan"
    );
    // -0 and +0 are the same key.
    assert_eq!(ev_str("var m = new Map(); m.set(0, 'z'); m.get(-0)"), "z");
}

#[test]
fn map_iteration() {
    // `for of` yields [k, v] pairs in insertion order.
    assert_eq!(
        ev_str(
            "var m = new Map(); m.set('a', 1); m.set('b', 2);\
             var s = ''; for (var kv of m) s += kv[0] + kv[1]; s"
        ),
        "a1b2"
    );
    // entries()/keys()/values() are iterators too.
    assert_eq!(
        ev_str(
            "var m = new Map(); m.set('a', 1);\
             var s = ''; for (var k of m.keys()) s += k; s"
        ),
        "a"
    );
    // forEach(v, k, m).
    assert_eq!(
        ev_str("var m = new Map(); m.set('a', 1); var s = ''; m.forEach(function (v, k) { s += k + v; }); s"),
        "a1"
    );
    // Spread consumes the iterator → array of pairs.
    assert_eq!(
        ev_str("var m = new Map(); m.set('a', 1); var a = [...m]; a[0][0] + ':' + a.length"),
        "a:1"
    );
}

#[test]
fn set_basics() {
    assert_eq!(
        ev_str(
            "var s = new Set(); s.add(1); s.add(1); s.add(2);\
             s.size + '|' + s.has(1) + '|' + s.has(9)"
        ),
        "2|true|false"
    );
    // Iterable ctor dedupes.
    assert_eq!(ev_str("var s = new Set([1, 2, 2, 3]); s.size"), "3");
    assert_eq!(
        ev_str("var s = new Set(); s.add('x'); s.delete('x'); s.has('x')"),
        "false"
    );
}

#[test]
fn set_iteration() {
    assert_eq!(
        ev_str(
            "var s = new Set(); s.add('a'); s.add('b');\
             var o = ''; for (var v of s) o += v; o"
        ),
        "ab"
    );
    // keys() === values() for sets; entries() yields [v, v].
    assert_eq!(
        ev_str("var s = new Set(['q']); var i = s.entries().next(); i.value[0] + i.value[1]"),
        "qq"
    );
    assert_eq!(
        ev_str("var s = new Set(); s.add('p'); var o = ''; s.forEach(function (v) { o += v; }); o"),
        "p"
    );
}

#[test]
fn json_parse_roundtrip() {
    assert_eq!(ev_str("JSON.parse('{\"a\":1}').a"), "1");
    assert_eq!(ev_str("JSON.parse('[1,2,3]')[1]"), "2");
    assert_eq!(ev_str("JSON.parse('\"x\"')"), "x");
    assert_eq!(ev_str("JSON.parse('null')"), "null");
    assert_eq!(ev_str("JSON.parse('true')"), "true");
    assert_eq!(ev_str("JSON.parse('1.5e2')"), "150");
    // Nested structure.
    assert_eq!(
        ev_str("var o = JSON.parse('{\"l\":[{\"k\":2}]}'); o.l[0].k"),
        "2"
    );
    // Invalid JSON throws SyntaxError.
    assert_eq!(
        ev_str("try { JSON.parse('{bad'); 'no' } catch (e) { e instanceof SyntaxError }"),
        "true"
    );
}

#[test]
fn json_stringify() {
    assert_eq!(
        ev_str("JSON.stringify({a: 1, b: 'x'})"),
        "{\"a\":1,\"b\":\"x\"}"
    );
    assert_eq!(
        ev_str("JSON.stringify([1, 'x', null, true])"),
        "[1,\"x\",null,true]"
    );
    assert_eq!(ev_str("JSON.stringify('he\"y')"), "\"he\\\"y\"");
    assert_eq!(ev_str("JSON.stringify(1.5)"), "1.5");
    assert_eq!(ev_str("JSON.stringify(undefined)"), "undefined");
    assert_eq!(ev_str("JSON.stringify(function () {})"), "undefined");
    // undefined/function props are dropped from objects, null in arrays.
    assert_eq!(
        ev_str("JSON.stringify({a: undefined, b: function () {}, c: 1})"),
        "{\"c\":1}"
    );
    assert_eq!(
        ev_str("JSON.stringify([undefined, function () {}])"),
        "[null,null]"
    );
    // Round-trip.
    assert_eq!(
        ev_str("JSON.stringify(JSON.parse('{\"x\":[1,2]}'))"),
        "{\"x\":[1,2]}"
    );
    // Cyclic object throws TypeError.
    assert_eq!(
        ev_str("var o = {}; o.self = o; try { JSON.stringify(o); 'no' } catch (e) { e instanceof TypeError }"),
        "true"
    );
}

// -- JS-7f ------------------------------------------------------------

#[test]
fn eval_global_scope() {
    // `eval(src)` — runs in global scope (indirect-eval semantics);
    // returns the completion value.
    assert_eq!(ev_str("eval('1 + 2')"), "3");
    assert_eq!(ev_str("var q = 0; eval('q = 7'); q"), "7");
    // Non-string args pass through.
    assert_eq!(ev_str("eval(42)"), "42");
    assert_eq!(ev_str("eval(undefined)"), "undefined");
    // Syntax error surfaces as a catchable SyntaxError.
    assert_eq!(
        ev_str("try { eval('var ='); 'no' } catch (e) { e instanceof SyntaxError }"),
        "true"
    );
}

#[test]
fn function_ctor_creates_global_function() {
    assert_eq!(
        ev_str("var f = new Function('a', 'b', 'return a + b'); f(2, 3)"),
        "5"
    );
    // No-arg Function() — empty body, callable.
    assert_eq!(ev_str("typeof new Function()"), "function");
    // Function() is *not* a closure — it binds the global env.
    assert_eq!(
        ev_str(
            "var mk = function () { var x = 9; return new Function('return x'); }; \
             var x = 4; mk()()"
        ),
        "4"
    );
    // A malformed body throws a catchable SyntaxError.
    assert_eq!(
        ev_str("try { new Function('@@@'); 'no' } catch (e) { e instanceof SyntaxError }"),
        "true"
    );
}

#[test]
fn function_prototype_bind() {
    assert_eq!(
        ev_str(
            "var o = {x: 5}; function g() { return this.x; } o.g = g; \
                var b = g.bind({x: 9}); b()"
        ),
        "9"
    );
    // Bound args prepend.
    assert_eq!(
        ev_str("function add(a, b) { return a + b; } var add2 = add.bind(null, 2); add2(3)"),
        "5"
    );
    // Bound fn stays a function for typeof.
    assert_eq!(ev_str("typeof (function () {}).bind(null)"), "function");
    // bind on non-callable throws TypeError.
    assert_eq!(
        ev_str(
            "try { Function.prototype.bind.call(42); 'no' } catch (e) { e instanceof TypeError }"
        ),
        "true"
    );
    // Curried chains.
    assert_eq!(
        ev_str("function f(a, b, c) { return a + b + c; } f.bind(null, 1).bind(null, 2)(3)"),
        "6"
    );
}

#[test]
fn symbol_basics() {
    assert_eq!(ev_str("typeof Symbol()"), "symbol");
    assert_eq!(ev_str("Symbol('x').toString()"), "Symbol(x)");
    assert_eq!(ev_str("Symbol('x').description"), "x");
    assert_eq!(ev_str("Symbol().description"), "undefined");
    // Identity: two Symbol() with same desc differ.
    assert_eq!(ev_str("Symbol('a') === Symbol('a')"), "false");
    // Registry: for() is stable, keyFor reverses.
    assert_eq!(ev_str("Symbol.for('k') === Symbol.for('k')"), "true");
    assert_eq!(ev_str("Symbol.keyFor(Symbol.for('k'))"), "k");
    assert_eq!(ev_str("Symbol.keyFor(Symbol())"), "undefined");
    // Symbol.for sets description = key.
    assert_eq!(ev_str("Symbol.for('k').description"), "k");
}

#[test]
fn reflect_basics() {
    assert_eq!(ev_str("var o = {a: 1}; Reflect.get(o, 'a')"), "1");
    assert_eq!(ev_str("var o = {}; Reflect.set(o, 'a', 3); o.a"), "3");
    assert_eq!(ev_str("Reflect.has({x: 1}, 'x')"), "true");
    assert_eq!(ev_str("Reflect.has({x: 1}, 'y')"), "false");
    assert_eq!(ev_str("Reflect.ownKeys({a: 1, b: 2}).join(',')"), "a,b");
    assert_eq!(
        ev_str("Reflect.apply(function (a, b) { return a + b; }, null, [2, 3])"),
        "5"
    );
    assert_eq!(
        ev_str("function C(a) { this.v = a; } Reflect.construct(C, [7]).v"),
        "7"
    );
    assert_eq!(
        ev_str("try { Reflect.get(42, 'x'); 'no' } catch (e) { e instanceof TypeError }"),
        "true"
    );
}

#[test]
fn array_extras() {
    assert_eq!(ev_str("Array.isArray([1])"), "true");
    assert_eq!(ev_str("Array.isArray({length: 0})"), "false");
    assert_eq!(ev_str("[1, 2, 3].includes(2)"), "true");
    assert_eq!(ev_str("[1, 2].includes(5)"), "false");
    // NaN matches itself in includes (SameValueZero), unlike indexOf.
    assert_eq!(ev_str("[NaN].includes(NaN)"), "true");
    assert_eq!(ev_str("[NaN].indexOf(NaN)"), "-1");
    assert_eq!(
        ev_str("[1, 2, 3].find(function (x) { return x > 1; })"),
        "2"
    );
    assert_eq!(
        ev_str("[1, 2, 3].findIndex(function (x) { return x > 1; })"),
        "1"
    );
    assert_eq!(
        ev_str("[1, 2].some(function (x) { return x > 1; })"),
        "true"
    );
    assert_eq!(
        ev_str("[1, 2].every(function (x) { return x > 0; })"),
        "true"
    );
    assert_eq!(ev_str("Array.from('ab').length"), "2");
}

#[test]
fn string_extras() {
    assert_eq!(ev_str("'hello'.includes('ell')"), "true");
    assert_eq!(ev_str("'hello'.startsWith('he')"), "true");
    assert_eq!(ev_str("'hello'.startsWith('lo', 3)"), "true");
    assert_eq!(ev_str("'hello'.endsWith('lo')"), "true");
    assert_eq!(ev_str("'ab'.repeat(3)"), "ababab");
    assert_eq!(ev_str("'5'.padStart(3, '0')"), "005");
    assert_eq!(ev_str("'5'.padEnd(3, '0')"), "500");
    assert_eq!(ev_str("'  x '.trimStart()"), "x ");
    assert_eq!(ev_str("'  x '.trimEnd()"), "  x");
}

#[test]
fn number_object_extras() {
    assert_eq!(ev_str("Number.isNaN(NaN)"), "true");
    assert_eq!(ev_str("Number.isNaN('NaN')"), "false"); // no coercion
    assert_eq!(ev_str("Number.isFinite(1)"), "true");
    assert_eq!(ev_str("Number.isFinite(Infinity)"), "false");
    assert_eq!(ev_str("Number.isInteger(3)"), "true");
    assert_eq!(ev_str("Number.isInteger(3.5)"), "false");
    assert_eq!(ev_str("Number.parseInt('42x')"), "42");
    assert_eq!(ev_str("Number.MAX_SAFE_INTEGER > 0"), "true");
    // Object statics.
    assert_eq!(
        ev_str("Object.getOwnPropertyNames({a: 1, b: 2}).join(',')"),
        "a,b"
    );
    assert_eq!(ev_str("Object.is(NaN, NaN)"), "true");
    assert_eq!(ev_str("Object.is(0, -0)"), "false");
    assert_eq!(ev_str("Object.is('a', 'a')"), "true");
    // freeze: silent no-op writes.
    assert_eq!(ev_str("var o = Object.freeze({a: 1}); o.a = 9; o.a"), "1");
    assert_eq!(ev_str("Object.isFrozen(Object.freeze({}))"), "true");
    assert_eq!(ev_str("Object.isFrozen({})"), "false");
}

#[test]
fn uri_extras_and_performance() {
    assert_eq!(
        ev_str("encodeURI('http://x/a b?q=1&k=v')"),
        "http://x/a%20b?q=1&k=v"
    );
    assert_eq!(
        ev_str("decodeURI('http://x/a%20b?q=1')"),
        "http://x/a b?q=1"
    );
    assert_eq!(
        ev_str("try { decodeURI('%zz'); 'no' } catch (e) { e.name }"),
        "URIError"
    );
    // Monotonic ms since engine start.
    assert_eq!(ev_str("typeof performance.now()"), "number");
    assert_eq!(ev_str("performance.now() >= 0"), "true");
}

// ---------------------------------------------------------------
// JS-7g — coverage extras
// ---------------------------------------------------------------

#[test]
fn object_entries_from_entries_has_own() {
    assert_eq!(
        ev_str("var o = {a: 1, b: 'x'}; Object.entries(o).map(function(p) { return p[0] + '=' + p[1]; }).join(',')"),
        "a=1,b=x"
    );
    assert_eq!(
        ev_str("var o = Object.fromEntries([['k', 42], ['s', 'v']]); o.k + '|' + o.s"),
        "42|v"
    );
    assert_eq!(
        ev_str("var m = new Map([['a', 1]]); Object.fromEntries(m).a"),
        "1"
    );
    assert_eq!(ev_str("Object.hasOwn({a: 1}, 'a')"), "true");
    assert_eq!(ev_str("Object.hasOwn({a: 1}, 'b')"), "false");
    assert_eq!(ev_str("Object.hasOwn([1, 2], '0')"), "true");
    // hasOwn vs proto
    assert_eq!(
        ev_str("var o = Object.create({p: 1}); Object.hasOwn(o, 'p')"),
        "false"
    );
}

#[test]
fn array_7g_extras() {
    assert_eq!(ev_str("[1, 2, 3].at(-1)"), "3");
    assert_eq!(ev_str("[1, 2, 3].at(5)"), "undefined");
    assert_eq!(ev_str("[1, 2, 3].fill(9, 1).join(',')"), "1,9,9");
    assert_eq!(ev_str("[1, [2, [3, [4]]]].flat(2).join(',')"), "1,2,3,4");
    assert_eq!(
        ev_str("[1, 2].flatMap(function(x) { return [x, x * 10]; }).join(',')"),
        "1,10,2,20"
    );
    assert_eq!(
        ev_str("[1, 2, 3, 4].findLast(function(x) { return x % 2 === 0; })"),
        "4"
    );
    assert_eq!(
        ev_str("[1, 2, 3, 4].findLastIndex(function(x) { return x % 2 === 0; })"),
        "3"
    );
    // Iterators interop with for-of / spread.
    assert_eq!(
        ev_str("var s = ''; for (var v of [7, 8].values()) s += v; s"),
        "78"
    );
    assert_eq!(
        ev_str("var s = ''; for (var k of ['a', 'b'].keys()) s += k; s"),
        "01"
    );
    assert_eq!(
        ev_str("var s = ''; for (var p of ['a', 'b'].entries()) s += p[0] + p[1]; s"),
        "0a1b"
    );
    assert_eq!(ev_str("[...[1, 2].values()].join('')"), "12");
    // ES2023 non-mutating copies.
    assert_eq!(
        ev_str("var a = [1, 2, 3]; var b = a.with(1, 9); a[1] + '/' + b[1]"),
        "2/9"
    );
    assert_eq!(
        ev_str("var a = [1, 2, 3]; var b = a.toReversed(); a.join('') + '/' + b.join('')"),
        "123/321"
    );
    assert_eq!(
        ev_str("var a = [3, 1, 2]; var b = a.toSorted(); a.join('') + '/' + b.join('')"),
        "312/123"
    );
    assert_eq!(
        ev_str("[1, 2, 3, 4].toSpliced(1, 2, 'x', 'y').join(',')"),
        "1,x,y,4"
    );
}

#[test]
fn string_7g_extras() {
    assert_eq!(ev_str("'abc'.at(-1)"), "c");
    assert_eq!(ev_str("'abc'.codePointAt(1)"), "98");
    assert_eq!(ev_str("String.fromCodePoint(97, 98)"), "ab");
    assert_eq!(
        ev_str("var m = 'a1b2'.matchAll(/[0-9]/g); var s = ''; for (var x of m) s += x[0]; s"),
        "12"
    );
    assert_eq!(
        ev_str("try { 'x'.matchAll(/./); 'no' } catch (e) { e.name }"),
        "TypeError"
    );
}

#[test]
fn text_encoder_decoder_utf8() {
    assert_eq!(
        ev_str("var b = new TextEncoder().encode('aé'); b.length + ':' + b[0] + ',' + b[1] + ',' + b[2]"),
        "3:97,195,169"
    );
    assert_eq!(
        ev_str("new TextDecoder().decode(new TextEncoder().encode('héllo'))"),
        "héllo"
    );
    assert_eq!(
        ev_str("typeof TextDecoder().decode"), // missing `new` still constructs? ctor requires new
        "function"
    );
}

#[test]
fn weakmap_weakset() {
    assert_eq!(
        ev_str("var w = new WeakMap(); var k = {}; w.set(k, 7); w.get(k) + ':' + w.has(k)"),
        "7:true"
    );
    assert_eq!(
        ev_str("var w = new WeakMap(); w.set('k', 1); w.get('k')"),
        "1"
    );
    assert_eq!(
        ev_str("var w = new WeakSet(); var k = {}; w.add(k); w.has(k)"),
        "true"
    );
    assert_eq!(ev_str("var w = new WeakSet(); w.has({})"), "false");
}

#[test]
fn date_utc_rich() {
    // Epoch + 1s → getters UTC.
    assert_eq!(
        ev_str("var d = new Date(1000); d.getUTCFullYear() + '/' + d.getUTCMonth() + '/' + d.getUTCDate() + ' ' + d.getUTCHours() + ':' + d.getUTCMinutes() + ':' + d.getUTCSeconds()"),
        "1970/0/1 0:0:1"
    );
    // Local == UTC (engine has no tz).
    assert_eq!(ev_str("new Date(0).getTimezoneOffset()"), "0");
    // Multi-arg ctor is UTC y,mo-0-based,d.
    assert_eq!(
        ev_str("var d = new Date(2000, 1, 29); d.getUTCFullYear() + '-' + d.getUTCMonth() + '-' + d.getUTCDate()"),
        "2000-1-29"
    );
    // toISOString.
    assert_eq!(
        ev_str("new Date(Date.UTC(2020, 11, 25, 12, 30, 45, 123)).toISOString()"),
        "2020-12-25T12:30:45.123Z"
    );
    // Date.parse ISO + round-trip.
    assert_eq!(
        ev_str("new Date(Date.parse('2020-12-25T12:30:45.123Z')).toISOString()"),
        "2020-12-25T12:30:45.123Z"
    );
    assert_eq!(ev_str("isNaN(Date.parse('not a date'))"), "true");
    // toUTCString shape.
    assert_eq!(
        ev_str("new Date(0).toUTCString()"),
        "Thu, 01 Jan 1970 00:00:00 GMT"
    );
    // y < 100 → 1900+y quirk.
    assert_eq!(ev_str("new Date(99, 0).getUTCFullYear()"), "1999");
}

#[test]
fn math_number_7g() {
    assert_eq!(ev_str("Math.hypot(3, 4)"), "5");
    assert_eq!(ev_str("Math.imul(3, 4)"), "12");
    assert_eq!(ev_str("Math.imul(-1, 1)"), "-1");
    assert_eq!(ev_str("Math.clz32(1)"), "31");
    assert_eq!(ev_str("(123.456).toPrecision(4)"), "123.5");
    assert_eq!(ev_str("(12345).toExponential(2)"), "1.23e+4");
}

// -- JS-C1 : lexique conforme test262 (identifiants Unicode, échappements) ----

#[test]
fn unicode_identifiers() {
    // Lettres et marques Unicode valides.
    assert_eq!(num(ev("var καλημέρα = 3; καλημέρα")), 3.0);
    assert_eq!(num(ev("var \\u0061bc = 4; abc")), 4.0);
    assert_eq!(num(ev("var \\u{62}c = 5; bc")), 5.0);
    // ZWNJ/ZWJ autorisés en continuation.
    assert_eq!(num(ev("var a\\u200cb = 6; a\u{200c}b")), 6.0);
    // U+2E2F VERTICAL TILDE : Lm mais Pattern_Syntax → rejeté.
    assert!(matches!(
        Engine::new().eval("var \u{2e2f} = 1"),
        Err(dillo_js::Error::Syntax { .. })
    ));
    // Mot réservé échappé → rejeté.
    assert!(matches!(
        Engine::new().eval("var \\u{69}f = 1"),
        Err(dillo_js::Error::Syntax { .. })
    ));
}

#[test]
fn whitespace_and_line_terminators() {
    // U+FEFF est un WhiteSpace ES.
    assert_eq!(num(ev("var\u{feff}x = 7; x")), 7.0);
    assert_eq!(num(ev("var x = /a/.source.length\u{feff}+ 6; x")), 7.0);
    // LS/PS sont des terminateurs (ASI) — deux instructions.
    assert_eq!(num(ev("var a = 1\u{2028}a + 1")), 2.0);
    assert_eq!(num(ev("var a = 1\u{2029}a + 1")), 2.0);
    // Terminateur dans une regex → erreur de syntaxe.
    for src in ["var r = /a\nb/", "var r = /a\rb/", "var r = /a\u{2028}b/"] {
        assert!(matches!(
            Engine::new().eval(src),
            Err(dillo_js::Error::Syntax { .. })
        ));
    }
}

#[test]
fn html_style_comments() {
    assert_eq!(num(ev("var x = 1; <!-- note\nx + 1")), 2.0);
    // `-->` en début de ligne est un commentaire (Annex B).
    assert_eq!(num(ev("var x = 2;\n--> fin de commentaire\nx")), 2.0);
    // `-->` hors début de ligne : décrément + comparaison.
    assert_eq!(ev_str("var x = 1; x --> 0"), "true");
}

#[test]
fn string_escapes() {
    // LegacyOctalEscapeSequence (sloppy).
    assert_eq!(ev_str(r"'\077'"), "?");
    assert_eq!(ev_str(r"'\1'"), "\u{1}");
    assert_eq!(ev_str(r"'\18'"), "\u{1}8");
    assert_eq!(ev_str(r"'\477'"), "'7");
    assert_eq!(ev_str(r"'\0'"), "\0");
    // `\0` suivi d'un chiffre non-octal : NUL + chiffre littéral.
    assert_eq!(ev_str(r"'\08'"), "\08");
    // `\u{…}` dans les chaînes.
    assert_eq!(ev_str(r"'\u{1f600}'"), "\u{1f600}");
    // Paire de surrogates fusionnée ; surrogate seul → U+FFFD.
    assert_eq!(ev_str(r"'\ud83c\udf06'"), "\u{1f306}");
    assert_eq!(ev_str(r"'\ud83c'"), "\u{fffd}");
    // Continuation de ligne sur tout terminateur.
    assert_eq!(ev_str("'a\\\u{2028}b'"), "ab");
    // CR/LF bruts restent interdits ; LS/PS bruts permis (ES2019).
    assert!(Engine::new().eval("'a\nb'").is_err());
    assert!(Engine::new().eval("'a\rb'").is_err());
    assert_eq!(ev_str("'a\u{2028}b'"), "a\u{2028}b");
}

#[test]
fn unterminated_inputs_rejected() {
    for src in [
        "var x = /* jamais fermé",
        "'chaîne",
        "var r = /regex",
        r"var s = '\",
    ] {
        assert!(matches!(
            Engine::new().eval(src),
            Err(dillo_js::Error::Syntax { .. })
        ));
    }
}

#[test]
fn malformed_numbers_rejected() {
    for src in ["var n = 3in", "var n = 1e", "var n = 1e+", "var n = 0x"] {
        assert!(matches!(
            Engine::new().eval(src),
            Err(dillo_js::Error::Syntax { .. })
        ));
    }
    // Octales legacy sloppy restent acceptées.
    assert_eq!(num(ev("var n = 010; n")), 8.0);
}

#[test]
fn regex_literal_eager_validation() {
    // Pattern invalide → SyntaxError au parse, pas d'objet RegExp.
    for src in ["var r = /[/", r"var r = /x\y/u", "var r = /a/zz"] {
        assert!(matches!(
            Engine::new().eval(src),
            Err(dillo_js::Error::Syntax { .. })
        ));
    }
    // Identity escape Annex B (sloppy) : `\y` = 'y' littéral.
    assert_eq!(ev_str(r"/x\y/.test('xy')"), "true");
    // `\0` NUL compilé.
    assert_eq!(ev_str(r"/a\0b/.test('a\0b')"), "true");
    // `\u{…}` et paires de surrogates en mode `u`.
    assert_eq!(ev_str(r"/\u{1f600}/u.test('\u{1f600}')"), "true");
    assert_eq!(ev_str(r"/^\ud83c\udf06$/u.test('\u{1f306}')"), "true");
}

#[test]
fn strict_mode() {
    let syn = |src: &str| matches!(Engine::new().eval(src), Err(dillo_js::Error::Syntax { .. }));
    // Octales legacy interdites en strict, tolérées en sloppy.
    assert!(syn(r#""use strict"; var n = 010"#));
    assert!(syn(r#""use strict"; var n = 08"#));
    assert!(syn(r#""use strict"; var s = "\1""#));
    assert_eq!(num(ev("var n = 010; n")), 8.0);
    // `\0` NUL reste légal en strict.
    assert_eq!(ev_str(r#""use strict"; "\0""#), "\0");
    // `delete` d'identifiant nu → early error.
    assert!(syn(r#""use strict"; var x = 1; delete x"#));
    assert!(!syn(r#""use strict"; var o = {a:1}; delete o.a"#));
    // Liaisons réservées strict.
    for src in [
        r#""use strict"; var eval = 1"#,
        r#""use strict"; var arguments = 1"#,
        r#""use strict"; function package() {}"#,
        r#""use strict"; var let = 1"#,
    ] {
        assert!(syn(src), "{src}");
    }
    // Sloppy : mêmes formes OK.
    assert_eq!(num(ev("var eval_ = 1; eval_")), 1.0);
    // Strict au niveau fonction seulement.
    assert_eq!(
        num(ev(
            "function f() { 'use strict'; return 1; } var n = 010; f() + n"
        )),
        9.0
    );
    // Hérité par les fonctions internes d'un script strict.
    assert!(syn(
        r#""use strict"; function g() { return function() { var n = 010; }; }"#
    ));
}

// -- JS-C2 : descripteurs de propriétés + strict runtime -------------

#[test]
fn property_descriptors() {
    // Descripteur partiel → flags par défaut `false` à la création.
    assert_eq!(
        ev_str(
            "var o = {}; Object.defineProperty(o, 'x', {value: 7}); \
             [o.x, Object.keys(o).length, delete o.x].join(',')"
        ),
        "7,0,false"
    );
    // Descripteur complet : flags lus et écrits.
    assert_eq!(
        ev_str(
            "var o = {}; Object.defineProperty(o, 'x', \
             {value: 1, writable: true, enumerable: true, configurable: true}); \
             o.x = 9; delete o.x; typeof o.x"
        ),
        "undefined"
    );
    // `getOwnPropertyDescriptor` reflète les flags.
    assert_eq!(
        ev_str(
            "var o = {a: 5}; var d = Object.getOwnPropertyDescriptor(o, 'a'); \
             [d.value, d.writable, d.enumerable, d.configurable].join(',')"
        ),
        "5,true,true,true"
    );
    // Redéfinition d'une non-configurable → TypeError.
    assert!(matches!(
        Engine::new().eval(
            "var o = {}; Object.defineProperty(o, 'x', {value: 1}); \
             Object.defineProperty(o, 'x', {value: 2})"
        ),
        Err(dillo_js::Error::Uncaught(_))
    ));
    // `defineProperties` + `getOwnPropertyDescriptors`.
    assert_eq!(
        ev_str(
            "var o = {}; Object.defineProperties(o, {a: {value: 1, enumerable: true}, \
             b: {value: 2}}); var ds = Object.getOwnPropertyDescriptors(o); \
             [o.a, o.b, ds.a.enumerable, ds.b.enumerable].join(',')"
        ),
        "1,2,true,false"
    );
    // `Object.create` avec map de descripteurs + héritage proto.
    assert_eq!(
        ev_str(
            "var p = {g: function() { return 1; }}; \
             var o = Object.create(p, {x: {value: 41, enumerable: true, writable: true}}); \
             o.x + o.g()"
        ),
        "42"
    );
    // `setPrototypeOf` + rejet de cycle.
    assert_eq!(
        ev_str("var o = {}, p = {k: 3}; Object.setPrototypeOf(o, p); o.k"),
        "3"
    );
    assert!(matches!(
        Engine::new().eval("var o = {}; Object.setPrototypeOf(o, o)"),
        Err(dillo_js::Error::Uncaught(_))
    ));
    // Constantes globales : `undefined`/`NaN`/`Math.E` verrouillées.
    assert_eq!(
        ev_str(
            "undefined = 5; NaN = 0; Math.E = 9; \
             [typeof undefined, isNaN(NaN), Math.E > 2.7 && Math.E < 2.72].join(',')"
        ),
        "undefined,true,true"
    );
    // `var` global non-configurable.
    assert_eq!(ev_str("var q = 1; delete q; q"), "1");
}

#[test]
fn freeze_seal_semantics() {
    // freeze : écriture silencieuse sloppy, TypeError strict.
    assert_eq!(
        ev_str("var o = Object.freeze({a: 1}); o.a = 9; o.b = 1; o.a + ',' + ('b' in o)"),
        "1,false"
    );
    assert!(matches!(
        Engine::new().eval("'use strict'; var o = Object.freeze({a: 1}); o.a = 9"),
        Err(dillo_js::Error::Uncaught(_))
    ));
    // seal : écriture OK, delete refusé.
    assert_eq!(
        ev_str(
            "var o = Object.seal({a: 1}); o.a = 9; \
             [o.a, delete o.a, Object.isSealed(o)].join(',')"
        ),
        "9,false,true"
    );
    // preventExtensions.
    assert_eq!(
        ev_str(
            "var o = Object.preventExtensions({}); o.x = 1; \
             [Object.isExtensible(o), 'x' in o].join(',')"
        ),
        "false,false"
    );
}

#[test]
fn strict_runtime() {
    let uncaught = |src: &str| matches!(Engine::new().eval(src), Err(dillo_js::Error::Uncaught(_)));
    // Écriture sur identifiant non déclaré : ReferenceError strict,
    // global implicite sloppy.
    assert!(uncaught("'use strict'; nouvelleVar = 1"));
    assert_eq!(num(ev("nouvelleVar2 = 3; nouvelleVar2")), 3.0);
    // Écriture sur prop non-writable : TypeError strict, silence sloppy.
    assert!(uncaught(
        "'use strict'; var o = {}; \
         Object.defineProperty(o, 'x', {value: 1, writable: false, configurable: true}); \
         o.x = 2"
    ));
    assert_eq!(
        ev_str("var o = {}; Object.defineProperty(o, 'x', {value: 1}); o.x = 2; o.x"),
        "1"
    );
    // delete non-configurable : TypeError strict.
    assert!(uncaught(
        "'use strict'; var o = {}; \
         Object.defineProperty(o, 'x', {value: 1}); delete o.x"
    ));
    // Écriture sur primitive : TypeError strict.
    assert!(uncaught("'use strict'; var s = 'ab'; s.x = 1"));
    // `this` : sloppy → objet global, strict → undefined.
    assert_eq!(
        ev_str("function f() { return this === window; } f()"),
        "true"
    );
    assert_eq!(
        ev_str("function f() { 'use strict'; return this === undefined; } f()"),
        "true"
    );
    // Strict hérité via fonction interne en script strict.
    assert!(uncaught(
        "'use strict'; function outer() { return function inner() { u = 1; }; } \
         outer()()"
    ));
    // Fonction strict dans script sloppy.
    assert!(uncaught("function s() { 'use strict'; y = 1; } s()"));
    // Accessor sans setter : TypeError strict, silence sloppy.
    assert!(uncaught(
        "'use strict'; var o = {}; \
         Object.defineProperty(o, 'x', {get: function() { return 1; }, configurable: true}); \
         o.x = 2"
    ));
    // const assign → TypeError dans les deux modes.
    assert!(uncaught("const c = 1; c = 2"));
    assert!(uncaught("'use strict'; const c = 1; c = 2"));
}
