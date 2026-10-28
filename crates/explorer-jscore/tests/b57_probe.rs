// This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
// SPDX-License-Identifier: MIT OR Apache-2.0

//! §B-57 (manomano polyfill + caniuse bundle, vols 96/97) —
//! le parseur produit `a+b+…`, `a.b.…`, `a[k]…`, `f()()…`
//! en boucle itérative (borne `nest` ~1) mais l'AST est
//! profonde à gauche : l'éval récursive comptait chaque
//! niveau dans `expr_depth` → `RangeError: expression too
//! deeply nested` sur les chaînes ×4 K+. Trampoline à
//! épine gauche (`eval_spine`) : la profondeur ne mesure
//! plus que la branche droite/feuille, bornée au parse.
use explorer_jscore::Engine;

/// Chaînes associatives-gauche ×5000 — compilent (parse
/// itératif) et s'évaluent désormais sans `RangeError`.
#[test]
fn left_deep_chains_eval() {
    let mut e = Engine::new();
    for (label, src, want) in [
        ("plus5000", format!("'x'{}", "+'x'".repeat(5000)), "Str"),
        ("and5000", format!("true{}", "&&true".repeat(5000)), "Bool"),
        ("or5000", format!("false{}", "||true".repeat(5000)), "Bool"),
        ("nullish5000", format!("1{}", "??2".repeat(5000)), "Num"),
        ("arith5000", format!("0{}", "+1".repeat(5000)), "Num"),
        ("member5000", format!("a{}.v", ".b".repeat(4999)), "Num"),
        (
            "index5000",
            format!("a{}[\"m\"]", "[0]".repeat(4999)),
            "Num",
        ),
        ("callchain5000", format!("g(){}", "()".repeat(4999)), "Num"),
    ] {
        // `a.b.b…`/`a[0]…`/`g()()…` : préparer le receveur.
        let setup = match label {
            // Cycle `a.b → a.b` : la chaîne retombe sur
            // `{v:42}` quel que soit le nombre de `.b`.
            "member5000" => "var a={b:{v:42}}; a.b.b=a.b;",
            "index5000" => "var a=[]; a[0]=a; a.m=9;",
            "callchain5000" => "var n=0; function g(){ return ++n>4999 ? 42 : g; }",
            _ => "",
        };
        if !setup.is_empty() {
            if let Err(err) = e.eval(setup) {
                panic!("setup {label} -> {err}");
            }
        }
        match e.eval(&src) {
            Ok(v) => {
                let d = format!("{v:?}");
                assert!(d.starts_with(want), "{label} -> {d}");
            }
            Err(err) => panic!("{label} -> {err}"),
        }
    }
}

/// Sémantique préservée par le trampoline : court-circuits
/// `&&`/`||`/`??`, ordre d'éval des opérandes et `this` des
/// appels membres (le callee member reste sur le bras
/// `Expr::Call` complet).
#[test]
fn left_deep_semantics_preserved() {
    let mut e = Engine::new();
    let cases: [(&str, &str); 8] = [
        // Court-circuit && : droite jamais évaluée.
        ("var s=0; false&&(s=1)&&true; s", "Num(0.0)"),
        // Court-circuit || : droite jamais évaluée.
        ("var s=0; true||(s=1); s", "Num(0.0)"),
        // ?? n'évalue la droite que sur null/undefined.
        ("var s=0; (null??(s=1))+(undefined??(s=2)); s", "Num(2.0)"),
        // && truthy → valeur de droite.
        ("1&&2&&3", "Num(3.0)"),
        // || falsy → valeur de droite.
        ("(0||0||'x')==='x'", "Bool(true)"),
        // Member chain → this lié au receveur complet.
        ("var o={a:{v:7,f(){return this.v}}}; o.a.f()", "Num(7.0)"),
        // f()() : callee Call dépilé, receveur undefined.
        (
            "function h(){return function(){return 9}}; h()()",
            "Num(9.0)",
        ),
        // Index chain : ordre d'éval des clés préservé.
        (
            "var log=[]; var k=()=>(log.push(1),0); var m=[ [5] ]; m[k()][k()]",
            "Num(5.0)",
        ),
    ];
    for (src, want) in cases {
        match e.eval(src) {
            Ok(v) => assert_eq!(format!("{v:?}"), want, "{src}"),
            Err(err) => panic!("{src} -> {err}"),
        }
    }
}

/// La borne `expr_depth` reste active sur la profondeur
/// *droite* : une expression hostile profondément imbriquée
/// par la droite est toujours rejetée (dégradation bornée).
#[test]
fn right_deep_still_bounded() {
    let mut e = Engine::new();
    // `1+(1+(1+(…)))` — right-deep : chaque opérande droit
    // récure → la borne doit toujours rejeter.
    let src = format!("{}{}{}", "1+(".repeat(5000), "0", ")".repeat(5000));
    match e.eval(&src) {
        Err(err) => assert!(
            format!("{err}").contains("too deeply nested"),
            "attendu RangeError, obtenu {err}"
        ),
        Ok(v) => panic!("right-deep ×5000 devrait échouer, obtenu {v:?}"),
    }
}

/// §B-57 bras 2 (manomano vol 100) — les builtins natifs
/// portaient `length` = `undefined` : le gate polyfill.io
/// `0===self.Symbol.length` échouait → faux Symbol installé →
/// le `Object.prototype.toString` polyfillé lisait le faux
/// symbole-objet en clé → `toString` → récursion `e↔Get` ∞.
/// `Fun::Native` porte désormais `length:0` (own prop,
/// défaut spec builtins).
#[test]
fn native_fns_portent_length() {
    let mut e = Engine::new();
    for (src, want) in [
        ("Symbol.length", "0"),
        ("typeof parseInt.length", "number"),
        ("typeof Object.prototype.toString.length", "number"),
        ("Symbol.hasOwnProperty('length')", "true"),
        (
            "Object.getOwnPropertyDescriptor(Symbol,'length').enumerable",
            "false",
        ),
    ] {
        match e.eval(src) {
            Ok(v) => assert_eq!(e.describe(v), want, "{src}"),
            Err(t) => panic!("{src} => ERR {t}"),
        }
    }
    // Le motif exact du rapport : toString polyfillé lisant
    // `Symbol.toStringTag` natif ne doit pas récurer.
    let src = "var t=Object.prototype.toString;Object.prototype.toString=function e(){if(void 0===this)return'[object Undefined]';if(null===this)return'[object Null]';var r=Object(this),o=r[Symbol.toStringTag];return'string'!==typeof o?t.call(r):'[object '+o+']'};Object.prototype.toString.call({})+ '|' + Object.prototype.toString.call(Symbol('x'))";
    match e.eval(src) {
        Ok(v) => assert_eq!(e.describe(v), "[object Object]|[object Symbol]"),
        Err(t) => panic!("motif manomano => ERR {t}"),
    }
}

/// §B-57 bras 3 (caniuse vol 103) — `Reflect.ownKeys` rendait
/// les clés-symboles comme strings sentinelles `\u{f130}sym{N}`
/// au lieu des `Symbol` (spec `[[OwnPropertyKeys]]`). `MobX`
/// `makeObservable` fait `Set(ownKeys).delete(z)` : le symbole
/// `z` survivait comme string → `make_("sym{N}")` posait un
/// accesseur `qr` sur le slot du symbole → `this[z]` →
/// getter → ∞ (`maximum call stack exceeded`, bundle.js:2:51736).
/// Bras sœurs colmatés : `Object.keys`/`entries`/`values` et
/// `for-in` excluent désormais toute sentinelle `\u{f120}-\u{f13f}`
/// énumérable (spec : clés string seulement).
#[test]
fn own_keys_rend_les_symboles() {
    let mut e = Engine::new();
    // `Reflect.ownKeys` : strings ET `Symbol` réels (identité).
    for (src, want) in [
        (
            "var z=Symbol('z');var o={};o[z]=1;o.a=2;Reflect.ownKeys(o).length",
            "2",
        ),
        (
            "var z=Symbol('z');var o={};o[z]=1;o.a=2;Reflect.ownKeys(o)[1]===z",
            "true",
        ),
        (
            "var z=Symbol('z');var o={};o[z]=1;typeof Reflect.ownKeys(o)[0]",
            "symbol",
        ),
        // Le mécanisme MobX exact : `delete(sym)` sur l'ensemble.
        (
            "var z=Symbol('z');var o={};o[z]=1;o.x=1;var s=new Set(Reflect.ownKeys(o));s.delete(z);s.size",
            "1",
        ),
        // Well-known : `Symbol.toStringTag` clé propre → Symbol rendu.
        (
            "var o={};o[Symbol.toStringTag]='T';Reflect.ownKeys(o)[0]===Symbol.toStringTag",
            "true",
        ),
    ] {
        match e.eval(src) {
            Ok(v) => assert_eq!(e.describe(v), want, "{src}"),
            Err(t) => panic!("{src} => ERR {t}"),
        }
    }
}

/// Bras sœurs : les sentinelle `\u{f130}sym{N}` ne doivent
/// jamais sortir comme strings par `keys`/`entries`/`values`/
/// `for-in` — même sur prop symbole `enumerable:true`.
#[test]
fn enum_ne_fuit_pas_les_symboles() {
    let mut e = Engine::new();
    for (src, want) in [
        // `Object.keys` : strings énumérables seulement.
        (
            "var z=Symbol('z');var o={};Object.defineProperty(o,z,{value:1,enumerable:true});o.a=1;Object.keys(o).join(',')",
            "a",
        ),
        // `Object.entries`/`values` : idem.
        (
            "var z=Symbol('z');var o={};Object.defineProperty(o,z,{value:7,enumerable:true});o.a=2;Object.entries(o).length+','+Object.values(o)[0]",
            "1,2",
        ),
        // `for-in` : la sentinelle anonyme n'est jamais une clé.
        (
            "var z=Symbol('z');var o={};Object.defineProperty(o,z,{value:1,enumerable:true});o.a=1;var out='';for(var k in o)out+=k;out",
            "a",
        ),
        // `getOwnPropertyNames` : filtré (comportement conservé).
        (
            "var z=Symbol('z');var o={};o[z]=1;o.a=2;Object.getOwnPropertyNames(o).join(',')",
            "a",
        ),
    ] {
        match e.eval(src) {
            Ok(v) => assert_eq!(e.describe(v), want, "{src}"),
            Err(t) => panic!("{src} => ERR {t}"),
        }
    }
    // Non-régression : `Object.assign` et le spread copient
    // toujours les props symboles énumérables (spec).
    for (src, want) in [
        (
            "var z=Symbol('z');var o={};Object.defineProperty(o,z,{value:9,enumerable:true});var t=Object.assign({},o);t[z]",
            "9",
        ),
        (
            "var z=Symbol('z');var o={};Object.defineProperty(o,z,{value:8,enumerable:true});var t={...o};t[z]",
            "8",
        ),
    ] {
        match e.eval(src) {
            Ok(v) => assert_eq!(e.describe(v), want, "{src}"),
            Err(t) => panic!("{src} => ERR {t}"),
        }
    }
}
