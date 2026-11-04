// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Verrous JS-C319 — `take_elems`/`put_elems` à l'épreuve des
//! erreurs : `take_elems` vide le backing vivant (`mem::take`)
//! et `put_elems` le repose. Historiquement, tout `?` entre les
//! deux laissait le tableau JS définitivement `[]` — erreur
//! souvent avalée par un `try/catch` applicatif (mur `YouTube`
//! §B-4(c) : `behaviors.flat(Infinity)` vidait
//! `_.ev.YtActionHandlerBehavior` → proto sans
//! `unregisterActionMap`). Chaque cas ci-dessous exige que le
//! tableau soit INTACT après l'opération, erreur ou non.

use explorer_jscore::Engine;

fn eval(src: &str) -> String {
    let mut e = Engine::new();
    match e.eval(src) {
        Ok(v) => e.describe(v),
        Err(err) => panic!("eval: {err}\n  src: {src}"),
    }
}

/// `flat` **lit** les sous-tableaux — il ne les vide pas
/// (le cas `YouTube` : `behaviors.flat(Infinity)`).
#[test]
fn flat_preserves_nested_arrays() {
    assert_eq!(
        eval("var inner=[{x:1}]; var outer=[1,inner,3];\
             var f=outer.flat(Infinity);\
             f.length+'/'+inner.length+'/'+outer.length+'/'+(inner===outer[1])+'/'+(f[1]===inner[0])"),
        "3/1/3/true/true"
    );
    // `flat` par défaut = profondeur 1 — le sous-tableau imbriqué
    // de niveau 2 survit aussi.
    assert_eq!(
        eval(
            "var deep=[7]; var mid=[deep]; var top=[mid];\
             top.flat(1); mid.length+'/'+deep.length"
        ),
        "1/1"
    );
    // `flatMap` idem, y compris quand la cb rend un tableau.
    assert_eq!(
        eval(
            "var a=[1,2]; var b=[[9]]; var r=[b].flatMap(function(x){return x});\
             r.length+'/'+b.length+'/'+(r[0]===b[0])"
        ),
        "1/1/true"
    );
}

/// `keys()`/`entries()`/`values()` ne mutent pas la source.
#[test]
fn iterators_preserve_array() {
    assert_eq!(
        eval(
            "var a=[1,2,3]; var k=a.keys(); var v=k.next().value;\
             a.length+'/'+v+'/'+a[0]"
        ),
        "3/0/1"
    );
    assert_eq!(
        eval(
            "var a=[5,6]; var en=a.entries(); en.next().value;\
             a.length+'/'+a[1]"
        ),
        "2/6"
    );
    assert_eq!(
        eval("var a=[8]; var it=a[Symbol.iterator](); it.next().value+'/'+a.length+'/'+a[0]"),
        "8/1/8"
    );
}

/// Callbacks qui jettent : le tableau survit à l'erreur.
#[test]
fn throwing_callbacks_preserve_array() {
    // `sort(cmp)` — merge sort itératif, `call_value` faillible.
    assert_eq!(
        eval(
            "var a=[3,1,2]; try{ a.sort(function(){throw 'boom'}) }catch(x){}\
             a.length+'/'+a[0]+'/'+a[1]+'/'+a[2]"
        ),
        "3/3/1/2"
    );
    // `forEach`/`map`/`filter`/`reduce`/`find`/`some`/`every`/
    // `findLast`/`flatMap` — même fenêtre `call_value`.
    assert_eq!(
        eval(
            "var a=[1,2,3]; try{ a.forEach(function(){throw 1}) }catch(x){}\
             try{ a.map(function(){throw 1}) }catch(x){}\
             try{ a.filter(function(){throw 1}) }catch(x){}\
             try{ a.reduce(function(){throw 1}) }catch(x){}\
             try{ a.find(function(){throw 1}) }catch(x){}\
             try{ a.some(function(){throw 1}) }catch(x){}\
             try{ a.every(function(){throw 1}) }catch(x){}\
             try{ a.findLast(function(){throw 1}) }catch(x){}\
             try{ a.flatMap(function(){throw 1}) }catch(x){}\
             a.length+'/'+a[0]+'/'+a[1]+'/'+a[2]"
        ),
        "3/1/2/3"
    );
    // `join` sur un élément dont `toString` jette.
    assert_eq!(
        eval(
            "var a=[{toString:function(){throw 'x'}},2];\
             try{ a.join(',') }catch(e){} a.length+'/'+a[1]"
        ),
        "2/2"
    );
}

/// Args dont la coercition jette : tableau intact.
#[test]
fn throwing_arg_coercion_preserves_array() {
    let hostile = "var bad={valueOf:function(){throw 'b'}};";
    // `indexOf(needle, fromIndex)`.
    assert_eq!(
        eval(&format!(
            "{hostile} var a=[1,2]; try{{ a.indexOf(2,bad) }}catch(x){{}} a.length+'/'+a[0]"
        )),
        "2/1"
    );
    // `splice`/`at`/`fill`/`with`/`toSpliced`/`copyWithin`.
    assert_eq!(
        eval(&format!(
            "{hostile} var a=[1,2,3];\
             try{{ a.splice(bad) }}catch(x){{}}\
             try{{ a.at(bad) }}catch(x){{}}\
             try{{ a.fill(0,bad) }}catch(x){{}}\
             try{{ a.with(bad,1) }}catch(x){{}}\
             try{{ a.toSpliced(bad) }}catch(x){{}}\
             try{{ a.copyWithin(bad) }}catch(x){{}}\
             a.length+'/'+a[0]+'/'+a[1]+'/'+a[2]"
        )),
        "3/1/2/3"
    );
}

/// Mutateurs sous `seal` : `TypeError` ET tableau intact
/// (avant : le tableau était vidé *en plus* de l'erreur).
#[test]
fn sealed_mutator_error_preserves_array() {
    assert_eq!(
        eval(
            "var a=[1,2,3]; Object.seal(a);\
             try{ a.pop() }catch(e){} a.length+'/'+a[0]+'/'+a[2]"
        ),
        "3/1/3"
    );
    assert_eq!(
        eval(
            "var a=[1,2,3]; Object.seal(a);\
             try{ a.splice(0,1) }catch(e){} a.length+'/'+a[0]+'/'+a[2]"
        ),
        "3/1/3"
    );
    assert_eq!(
        eval(
            "var a=[1,2,3]; Object.seal(a);\
             try{ a.push(4) }catch(e){} a.length+'/'+a[0]"
        ),
        "3/1"
    );
    assert_eq!(
        eval(
            "var a=[1,2,3]; Object.seal(a);\
             try{ a.unshift(4) }catch(e){} a.length+'/'+a[0]"
        ),
        "3/1"
    );
    assert_eq!(
        eval(
            "var a=[1,2,3]; Object.seal(a);\
             try{ a.shift() }catch(e){} a.length+'/'+a[0]"
        ),
        "3/1"
    );
}

/// Les chemins sains restent intacts : `flat`/`sort`/`splice`
/// produisent les bons résultats.
#[test]
fn happy_path_unchanged() {
    assert_eq!(
        eval("var a=[3,1,2]; a.sort(); a[0]+'/'+a[1]+'/'+a[2]+'/'+a.length"),
        "1/2/3/3"
    );
    assert_eq!(
        eval("var a=[1,[2,[3]]]; a.flat(Infinity).join(',')"),
        "1,2,3"
    );
    assert_eq!(
        eval("var a=[1,2,3]; var d=a.splice(1,1,9); d[0]+'/'+a.join(',')"),
        "2/1,9,3"
    );
    assert_eq!(
        eval("var a=[1,2,3]; a.map(function(x){return x*10}).join(',')"),
        "10,20,30"
    );
}
