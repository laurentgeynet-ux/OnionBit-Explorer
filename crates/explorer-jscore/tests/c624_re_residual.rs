// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! JS-C624 — reliquat RegExp/String :
//!
//! - **`lastIndex > length` rejeté en code units** : la garde
//!   `exec` comparaît en chars du domaine encodé — `u2c(9)`
//!   clampait à la fin → match vide à la fin en boucle
//!   infinie (`/(?:)/gu` + `matchAll` au cap heap, observé
//!   dans `regexp-prototype-matchAll-v-u-flag`).
//! - **`ToLength` sur `lastIndex` relu pour `AdvanceStringIndex`**
//!   (`@@match`/`@@replace`/`@@matchAll`) : un `valueOf` qui rend
//!   2^54 est borné à 2^53−1 **puis** avancé (+1 → 2^53) —
//!   test262 `coerce-lastindex`.
//! - **Groupes nommés non-ASCII** : fancy-regex n'accepte que
//!   `[A-Za-z0-9_]` dans `?P<…>` — les `IdentifierName` plus
//!   larges (`(?<$𐒤>`, `(?<é>`) sont émis sous `__uN`
//!   synthétique et traduits à l'affichage (`groups`,
//!   `indices.groups`, `\k<…>`).

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

/// `matchAll` sur match vide + flag `u` : termine (positions
/// en units — avance `AdvanceStringIndex` unicode), ne boucle
/// plus au cap heap.
#[test]
fn matchall_empty_u_terminates() {
    assert_eq!(
        eval(r"var t='𠮷a𠮷b𠮷', r=Array.from(t.matchAll(/(?:)/gu)); r.map(m=>m.index).join(',')"),
        "0,2,3,5,6,8"
    );
    // Non-`u` : positions par unit (astral = 2 units).
    assert_eq!(
        eval(r"var t='𠮷a𠮷b𠮷', r=Array.from(t.matchAll(/(?:)/g)); r.map(m=>m.index).join(',')"),
        "0,1,2,3,4,5,6,7,8"
    );
}

/// `exec` rejette `lastIndex > length` — mesuré en **units**.
#[test]
fn exec_past_length_null() {
    assert_eq!(
        eval(r#"var r=/a/g; r.lastIndex=9; r.exec("𐒤a") === null"#),
        "true"
    );
    // `lastIndex` pile à length → match vide valide à la fin.
    assert_eq!(
        eval(r#"var r=/(?:)/g; r.lastIndex=3; var m=r.exec("𐒤a"); m.index"#),
        "3"
    );
}

/// `@@replace` : `lastIndex` relu par `ToLength` avant
/// `AdvanceStringIndex` — 2^54 → 2^53−1 → +1 → 2^53.
#[test]
fn replace_lastindex_tolength() {
    assert_eq!(
        eval(
            r#"var r=/./g,c=0; r.exec=function(){c++; if(c>1)return null; r.lastIndex={valueOf:function(){return Math.pow(2,54)}}; return {length:1,0:'',index:0}}; "abc".replace(r,"X"); r.lastIndex"#
        ),
        "9007199254740992"
    );
}

/// Groupes nommés non-ASCII : déclaration, `groups`,
/// `indices.groups`, `\k<…>` — noms originaux exposés.
#[test]
fn named_groups_non_ascii() {
    assert_eq!(
        eval(r#"var m=/(?<$𐒤>b)/.exec("ab"); [m.groups["$𐒤"], m[1]].join("|")"#),
        "b|b"
    );
    assert_eq!(
        eval(r#"var m=/(?<$𐒤>b)x\k<$𐒤>/.exec("abxb"); m ? m[0] : "null""#),
        "bxb"
    );
    assert_eq!(
        eval(
            r#"var m=/(?<$a>x)(?<é>y)/d.exec("axy"); [m.groups["$a"],m.groups["é"],JSON.stringify(m.indices.groups)].join("|")"#
        ),
        r#"x|y|{"$a":[1,2],"é":[2,3]}"#
    );
    assert_eq!(eval(r#"new RegExp("(?<$𐒤>b)").test("ab")"#), "true");
}
