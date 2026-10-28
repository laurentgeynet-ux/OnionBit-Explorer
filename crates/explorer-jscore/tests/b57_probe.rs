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
