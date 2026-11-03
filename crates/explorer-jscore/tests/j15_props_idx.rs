// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Verrous J15 — index paresseux `nom → position` sur
//! `Object::props` (construit au premier lookup passé le seuil,
//! invalidé au `delete` qui décale les positions, jamais sur
//! réécriture en place). La sémantique externe est figée par ces
//! tests — ordre d'insertion (`for-in`/`Object.keys`), attributs,
//! accessor-vs-data, `delete` + ré-ajout en fin.
//!
//! Les bancs `j15_bench_*` (`--ignored --nocapture`) impriment le
//! `last_eval_split` d'une charge « objet-config géant » — chiffres
//! avant/après consignés dans le commit du changement (règle perf :
//! jamais d'optimisation sans le split avant → après).

use explorer_jscore::Engine;

fn t(src: &str) -> String {
    let mut e = Engine::new();
    match e.eval(src) {
        Ok(v) => e.describe(v),
        Err(err) => format!("ERR {err}"),
    }
}

/// Construit un objet de `n` propriétés nommées `k0..k{n-1}` par
/// `[[Set]]` — le chemin chaud (routé par l'index au-delà de 24).
fn giant(n: usize) -> String {
    format!("var o={{}};for(var i=0;i<{n};i++)o['k'+i]=i;o;")
}

#[test]
fn giant_object_reads_back_all_props() {
    // 60 props > seuil : chaque lecture doit retrouver sa valeur.
    let src = format!("{}var s=0;for(var j=0;j<60;j++)s+=o['k'+j];s", giant(60));
    assert_eq!(t(&src), "1770"); // 0+1+…+59
}

#[test]
fn delete_then_lookups_hit_right_slots() {
    // Après `delete k5` (décalage des positions 6..59), toutes les
    // lectures restantes doivent viser la bonne case — le verrou
    // qui attrape un index périmé.
    let src = format!(
        "{}delete o['k5'];var s='';for(var j=0;j<60;j++)if(j!==5)s+=o['k'+j]+',';s",
        giant(60)
    );
    let want: String = (0..60)
        .filter(|&j| j != 5)
        .map(|j| j.to_string() + ",")
        .collect::<Vec<_>>()
        .concat();
    assert_eq!(t(&src), want);
}

#[test]
fn delete_readd_moves_key_to_end() {
    // Spec : `delete` supprime la paire ; la ré-insertion va en
    // fin d'ordre (comportement observable `Object.keys`).
    let src = format!(
        "{}delete o['k5'];o['k5']=999;Object.keys(o).slice(-3).join(',')+'|'+o['k5']",
        giant(60)
    );
    assert_eq!(t(&src), "k58,k59,k5|999");
}

#[test]
fn for_in_order_is_insertion_order_on_giant() {
    // Clés non-entières : ordre d'insertion strict.
    let src = format!(
        "{}var r=[];for(var k in o)r.push(k);r.slice(0,3).join(',')+'|'+r.slice(-2).join(',')",
        giant(30)
    );
    assert_eq!(t(&src), "k0,k1,k2|k28,k29");
}

#[test]
fn integer_keys_sort_first_after_delete() {
    // Spec ownKeys : indices entiers d'abord (numériques), puis
    // chaînes en ordre d'insertion — même après un delete.
    let src = "var o={b:1,'10':1,a:1,'2':1,c:1};delete o.a;o.d=1;\
               Object.keys(o).join(',')";
    assert_eq!(t(src), "2,10,b,c,d");
}

#[test]
fn define_property_keeps_slot_and_attrs() {
    // `defineProperty` sur clé existante = même slot (position
    // conservée dans l'ordre) ; nouvelle clé = fin d'ordre.
    let src = format!(
        "{}Object.defineProperty(o,'k3',{{value:42,enumerable:false}});\
           Object.defineProperty(o,'zz',{{value:7,enumerable:true}});\
           Object.keys(o).slice(-1).join(',')+'|'+o['k3']+'|'+Object.keys(o).length",
        giant(40)
    );
    assert_eq!(t(&src), "zz|42|40"); // k3 non-énumérable, zz en fin
}

#[test]
fn failed_delete_keeps_index_valid() {
    // `delete` refusé (prop non-configurable) : aucune position ne
    // bouge — les lectures suivantes restent exactes. La somme
    // attendue tient compte de la redéfinition `value:1` (435-7+1).
    let src = format!(
        "{}Object.defineProperty(o,'k7',{{value:1,configurable:false}});\
           delete o['k7'];var s=0;for(var j=0;j<30;j++)s+=o['k'+j];s",
        giant(30)
    );
    assert_eq!(t(&src), "429");
}

#[test]
fn has_in_and_own_property_on_giant() {
    let src = format!(
        "{}('k25' in o)+'|'+o.hasOwnProperty('k25')+'|'+('k99' in o)+'|'+o.hasOwnProperty('k99')",
        giant(30)
    );
    assert_eq!(t(&src), "true|true|false|false");
}

#[test]
fn accessor_over_existing_data_keeps_position() {
    // Remplacer une data prop par un accessor garde le slot ;
    // la lecture passe par le getter. `enumerable:true` explicite —
    // `defineProperty` la mettrait à `false` par défaut (spec).
    let src = format!(
        "{}Object.defineProperty(o,'k10',{{get:function(){{return 77}},enumerable:true}});\
           var r=[];for(var k in o)r.push(k);r.indexOf('k10')+':'+o['k10']",
        giant(30)
    );
    assert_eq!(t(&src), "10:77");
}

#[test]
fn assign_and_spread_hit_giant_object() {
    // `Object.assign` écrit dans un objet déjà indexé — les
    // nouvelles clés sont visibles, les écrasées à jour.
    let src = format!(
        "{}Object.assign(o,{{k1:111,nouv:5}});o['k1']+'|'+o['nouv']+'|'+o['k29']",
        giant(30)
    );
    assert_eq!(t(&src), "111|5|29");
}

#[test]
fn delete_then_growth_reindexes_cleanly() {
    // Après invalidation (delete), la croissance continue : les
    // clés ajoutées et les survivantes cohabitent correctement.
    let src = format!(
        "{}delete o['k10'];delete o['k20'];for(var i=30;i<40;i++)o['n'+i]=i;\
           var s=0;for(var j=0;j<30;j++)if(j!==10&&j!==20)s+=o['k'+j];\
           var s2=0;for(var j=30;j<40;j++)s2+=o['n'+j];s+':'+s2+':'+(o['k10'])",
        giant(30)
    );
    assert_eq!(t(&src), "405:345:undefined");
}

// ─── Bancs (mesure avant/après — `--release --ignored --nocapture`)

/// Charge « objet-config géant » : construction `[[Set]]` (find +
/// push par clé) puis balayages complets de l'espace des clés —
/// le profil `OtAutoBlock` (milliers de props, lectures répétées).
#[test]
#[ignore = "banc J15 : chiffres exec_us avant/après — cargo test --release -- --ignored --nocapture"]
fn j15_bench_synthetic() {
    let mut e = Engine::new();
    let src = "var o={};var N=3000;\
               for(var i=0;i<N;i++)o['k'+i]=i;\
               var t=0;\
               for(var r=0;r<20;r++)for(var j=0;j<N;j++)t+=o['k'+j];\
               t;";
    let t0 = std::time::Instant::now();
    let r = e.eval(src);
    let wall = t0.elapsed();
    let (parse_us, exec_us) = e.last_eval_split();
    eprintln!("[j15-synth] wall={wall:?} parse_us={parse_us} exec_us={exec_us} -> {r:?}");
    assert!(r.is_ok());
}

/// Rejoue un script réel figé (dump `EXPLORER_JS_DUMP_DIR`) :
/// `J15_BENCH_JS=<chemin> cargo test --release -- --ignored
/// --nocapture`. Dom minimal installé pour survivre aux accès
/// `document`/`window` précoces.
#[test]
#[ignore = "banc J15 : fixture réelle via J15_BENCH_JS"]
fn j15_bench_real_script() {
    let Ok(path) = std::env::var("J15_BENCH_JS") else {
        eprintln!("[j15-real] SKIP — J15_BENCH_JS non posé");
        return;
    };
    let src = std::fs::read_to_string(&path).expect("fixture lisible");
    let mut e = Engine::new();
    e.install_dom(&std::rc::Rc::new(std::cell::RefCell::new(
        explorer_jscore::Dom::from_html("<html><body><div id=x></div></body></html>"),
    )));
    let t0 = std::time::Instant::now();
    let r = e.eval_named(&src, &path);
    let wall = t0.elapsed();
    let (parse_us, exec_us) = e.last_eval_split();
    eprintln!(
        "[j15-real] bytes={} wall={wall:?} parse_us={parse_us} exec_us={exec_us} ok={}",
        src.len(),
        r.is_ok()
    );
}
