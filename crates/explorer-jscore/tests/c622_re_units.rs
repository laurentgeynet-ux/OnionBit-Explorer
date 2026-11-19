// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! JS-C622 : positions RegExp/String en **code units UTF-16**
//! (et non en scalaires Rust) — astral = 2 units, mid-paire
//! matchable en non-`u`, `lastIndex`/`index`/`indices`/positions
//! de callback en units. `UnitMap` traduit le domaine interne ;
//! les regex non-`u` matchent en domaine « units éclatés »
//! (chaque unit = une sentinelle `U+F0000+u-0xD800`).

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

/// `index`/`lastIndex` comptent les code units (astral = 2).
#[test]
fn astral_index_and_lastindex() {
    // 'a' est au code unit 2 (𐒤 occupe 0-2).
    assert_eq!(eval(r#"/a/.exec("𐒤a").index"#), "2");
    // non-`u` : `lastIndex` en units — repart à 3, matche `a` à 3.
    assert_eq!(
        eval(r#"var r=/a/g; r.exec("𐒤a"); var x=[r.lastIndex, r.exec("𐒤aa").index]; x.join(",")"#),
        "3,3"
    );
    // `u` : positions toujours en units.
    assert_eq!(eval(r#"/a/u.exec("𐒤a").index"#), "2");
}

/// Non-`u` : les units isolés d'une paire sont matchables
/// (lead et trail séparément — sémantique code-unit de la spec).
#[test]
fn non_u_matches_inside_pair() {
    // 𐒤 = units D801/DCA4 — `\ud83d` ne matche pas, `\ud801` si.
    assert_eq!(eval(r#"/\ud83d/.test("𐒤")"#), "false");
    assert_eq!(eval(r#"/\ud801/.test("𐒤")"#), "true");
    assert_eq!(eval(r#"/\ud801/.exec("x𐒤").index"#), "1");
    assert_eq!(eval(r#"/\udca4/.exec("x𐒤").index"#), "2");
    assert_eq!(eval(r#"/\udca4/g.exec("x𐒤𐒤").index"#), "2");
    // Point `.` non-`u` consomme 1 unit (mid-paire possible) ;
    // `u` consomme le point entier (2 units).
    assert_eq!(
        eval(r#"var r=/./g,u=/𐒤/ug; r.exec("𐒤x"); u.exec("𐒤x"); r.lastIndex+","+u.lastIndex"#),
        "1,2"
    );
}

/// Flag `d` : `indices` rapportés en code units.
#[test]
fn d_flag_indices_in_units() {
    assert_eq!(
        eval(r#"var m=/(a)/d.exec("𐒤a"); m.indices[0].join(",")+";"+m.indices[1].join(",")"#),
        "2,3;2,3"
    );
    assert_eq!(
        eval(r#"var m=/(𐒤)(b)/du.exec("𐒤b"); m.indices.map(p=>p.join("-")).join("|")"#),
        "0-3|0-2|2-3"
    );
}

/// `@@replace` : positions de callback et de remplacement en
/// units, `lastIndex` lu/écrit en units.
#[test]
fn replace_positions_in_units() {
    assert_eq!(eval(r#""a𐒤z".replace(/z/, (m,o)=>o)"#), "a𐒤3");
    assert_eq!(eval(r#""𐒤a𐒤".replace(/a/g, m=>m+"!")"#), "𐒤a!𐒤");
    // Insertion après mid-paire — match d'un unit trail.
    assert_eq!(eval(r#""𐒤x".replace(/\udca4/, "Y").length"#), "3");
}

/// `@@split` : bornes et pièces en units autour d'astrals.
#[test]
fn split_positions_in_units() {
    assert_eq!(eval(r#""𐒤,a,𐒤".split(/,/).join("|")"#), "𐒤|a|𐒤");
    // Split sur un unit isolé mid-paire : `"a\uD802"` + `"b"`.
    assert_eq!(
        eval(r#"var p="a𐒤b".split(/\udca4/); p.length+":"+p[0].length+":"+p[1]"#),
        "2:2:b"
    );
}

/// `matchAll` paresseux : `index` en units à chaque `next`.
#[test]
fn match_all_indices_in_units() {
    assert_eq!(
        eval(
            r#"var it="𐒤a𐒤a".matchAll(/a/g), x=[]; var m; while(!(m=it.next()).done) x.push(m.value.index); x.join(",")"#
        ),
        "2,5"
    );
}

/// `@@search`/`String.match` positions en units.
#[test]
fn search_and_match_units() {
    assert_eq!(eval(r#""𐒤ab".search(/b/)"#), "3");
    assert_eq!(eval(r#"var m="𐒤ab".match(/b/); m.index+":"+m[0]"#), "3:b");
}
