// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Verrous JS-C500 — inline cache own-prop `site → (shape,pos)` :
//! la vérif est sémantique (`props[pos].0 == key` + `Prop::Data` +
//! éligibilité du kind) donc chaque cas ci-dessous doit rendre la
//! valeur exacte quel que soit l'état du cache. Les cas clés :
//! polymorphisme d'instances (le memo `(ObjId,pos)` initial les
//! ratait), accessor in-place, prop ajoutée/supprimée (shape sale),
//! site mégamorphe désarmé, props synthétisées (`length`, éléments
//! typés) qui devancent `o.find`, kill-switch `EXPLORER_NO_IC`.

use explorer_jscore::Engine;

/// Sérialise les tests sensibles au contexte global (env
/// `EXPLORER_NO_IC`, compteurs J17 partagés) — les autres tests
/// du binaire restent parallèles.
static IC_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn eval(src: &str) -> String {
    let mut e = Engine::new();
    match e.eval(src) {
        Ok(v) => e.describe(v),
        Err(err) => panic!("eval: {err}\n  src: {src}"),
    }
}

/// Deux instances de même layout lues au même site → chacune sa
/// valeur (le coeur du design : la shape partagée hit, jamais la
/// valeur d'un autre objet — la clé à `pos` le garantit).
#[test]
fn c500_polymorphic_instances() {
    assert_eq!(
        eval(
            "function P(v){this.x=v;this.y=v*2}\
             var a=new P(3),b=new P(7),s=0;\
             for(var i=0;i<200;i++){s+=a.x;b.x+=1}\
             s+'/'+a.x+'/'+b.x+'/'+b.y"
        ),
        "600/3/207/14"
    );
}

/// Accessor sur la chaîne → getter exécuté à chaque lecture,
/// jamais mémorisé comme data (compteur d'appels visible).
#[test]
fn c500_accessor_not_cached() {
    assert_eq!(
        eval(
            "var o={n:0};Object.defineProperty(o,'g',{get:function(){return ++this.n},enumerable:true});\
             o.g+','+o.g+','+o.g"
        ),
        "1,2,3"
    );
}

/// `Data → Acc` en place (même position) : la vérif `Prop::Data`
/// doit échouer et le getter prendre le relais dès la lecture
/// suivante — aucune valeur périmée.
#[test]
fn c500_data_to_accessor_in_place() {
    assert_eq!(
        eval(
            "var o={x:1};var r='';r+=o.x+','+o.x+',';\
             Object.defineProperty(o,'x',{get:function(){return 42}});\
             r+=o.x+','+o.x;r"
        ),
        "1,1,42,42"
    );
}

/// Prop ajoutée après apprentissage → shape change → le site
/// ré-apprend et lit la nouvelle valeur, pas l'ancienne position.
#[test]
fn c500_shape_growth() {
    assert_eq!(
        eval(
            "var o={x:5};var r='';for(var i=0;i<50;i++)r=o.x;\
             o.z=9;r+=','+o.x+','+o.z;for(var i=0;i<50;i++)r+='';r+=','+o.x;r"
        ),
        "5,5,9,5"
    );
}

/// `delete` → positions décalées + shape sale → lectures
/// suivantes exactes (la prop supprimée → undefined, ses voisines
/// à leur nouvelle place).
#[test]
fn c500_delete_shifts_positions() {
    assert_eq!(
        eval(
            "var o={a:1,b:2,c:3};var r='';for(var i=0;i<50;i++)r=o.b;\
             delete o.b;r='';for(var i=0;i<8;i++)r+=(o.a+'-'+o.b+'-'+o.c+'|');r"
        ),
        "1-undefined-3|".repeat(8)
    );
}

/// Hit proto (méthode sur le prototype, pas own-prop) : valeur
/// correcte, et l'IC n'apprend pas (owner ≠ tête) — comportement
/// identique au chemin complet.
#[test]
fn c500_proto_hit_unchanged() {
    assert_eq!(
        eval(
            "function P(){}P.prototype.m=function(){return this.v};\
             function P2(v){this.v=v}P2.prototype=Object.create(P.prototype);\
             var a=new P2(8),s=0;for(var i=0;i<200;i++)s+=a.m();s"
        ),
        "1600"
    );
}

/// Site mégamorphe (`o[k]` à clés tournantes) : thrash → désarmé
/// → valeurs toujours exactes.
#[test]
fn c500_megamorphic_disarm_correct() {
    assert_eq!(
        eval(
            "var o={};for(var i=0;i<20;i++)o['k'+i]=i;\
             var s=0;for(var i=0;i<300;i++){var k='k'+(i%20);s+=o[k]}s"
        ),
        "2850"
    );
}

/// Props synthétisées qui devancent `o.find` : `arr.length`,
/// éléments, `fn.length`/`fn.name`, typed array — jamais
/// court-circuitées par une entrée IC.
#[test]
fn c500_synthesized_props_win() {
    assert_eq!(
        eval(
            "var a=[1,2,3];var r='';for(var i=0;i<30;i++)r=a.length;\
             function f(x,y){}for(var i=0;i<30;i++)r+=f.length+'/'+f.name;\
             var t=new Uint8Array(4);t[0]=7;\
             for(var i=0;i<30;i++)r+='/'+t[0]+t.length;r"
        ),
        "3".to_owned() + &"2/f".repeat(30) + &"/74".repeat(30)
    );
}

/// Écriture in-place sur la clé cachée : `props[pos]` relu frais
/// → la nouvelle valeur est rendue (jamais l'ancienne).
#[test]
fn c500_inplace_write_visible() {
    assert_eq!(
        eval("var o={x:1};var r='';for(var i=0;i<20;i++){r+=o.x;o.x+=1}r"),
        "1234567891011121314151617181920"
    );
}

/// L'IC engage réellement : boucle chaude sur un site →
/// `ic_hit` croît au moins autant que les itérations moins le
/// premier learn.
#[test]
fn c500_counter_engages() {
    // Sous verrou : `Engine::new` lit `EXPLORER_NO_IC` — la
    // fenêtre du test kill-switch ne doit pas se superposer.
    let _g = IC_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    explorer_jscore::j17_enable();
    let before = explorer_jscore::j17_snapshot();
    let before_hit = before
        .split(' ')
        .find_map(|p| {
            p.strip_prefix("ic_hit=")
                .and_then(|v| v.parse::<u64>().ok())
        })
        .unwrap_or(0);
    let mut e = Engine::new();
    let r = e
        .eval("var o={x:7},s=0;for(var i=0;i<1000;i++)s+=o.x;s")
        .expect("eval");
    assert_eq!(e.describe(r), "7000");
    let after = explorer_jscore::j17_snapshot();
    let after_hit = after
        .split(' ')
        .find_map(|p| {
            p.strip_prefix("ic_hit=")
                .and_then(|v| v.parse::<u64>().ok())
        })
        .unwrap_or(0);
    assert!(
        after_hit >= before_hit + 500,
        "ic_hit n'a pas engagé : {before} → {after}"
    );
}

/// `EXPLORER_NO_IC=1` → valeurs inchangées (le cache est un pur
/// raccourci). Fenêtre d'env courte : les autres tests de ce
/// binaire tournent en parallèle — `ic_off` ne casse que l'opti,
/// jamais la sémantique.
#[test]
fn c500_kill_switch() {
    // Sous verrou : la fenêtre d'env ne doit pas couvrir le
    // `Engine::new` du test de compteur (même binaire, threads).
    let _g = IC_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    std::env::set_var("EXPLORER_NO_IC", "1");
    let out = eval("var o={x:4};var s=0;for(var i=0;i<50;i++)s+=o.x;s");
    std::env::remove_var("EXPLORER_NO_IC");
    assert_eq!(out, "200");
}

/// `arguments` mappé / objet `arguments` : les indices renvoient
/// au slot env — inéligible, jamais intercepté par l'IC.
#[test]
fn c500_mapped_arguments() {
    assert_eq!(
        eval(
            "function f(a,b){var r='';for(var i=0;i<30;i++)r+=arguments[0]+arguments[1];\
             a=9;return r+'/'+arguments[0]}f(1,2)"
        ),
        "3".repeat(30).as_str().to_owned() + "/9"
    );
}
