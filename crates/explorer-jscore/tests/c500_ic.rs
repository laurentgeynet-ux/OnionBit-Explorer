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
/// correcte — l'entrée `Chain` JS-C504 la vérifie par les shapes
/// des maillons, comportement identique au chemin complet.
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

// ── JS-C504 — hits proto + absents dans l'IC ────────────────────

/// Hit proto appris (`Chain`) : la vérif marche les shapes des
/// maillons — deux instances de même layout partagent l'entrée,
/// chacune rend sa méthode avec le bon `this`.
#[test]
fn c504_proto_hit_cached() {
    assert_eq!(
        eval(
            "function P(){}P.prototype.m=function(){return this.v+1};\
             var a=new P(),b=new P();a.v=10;b.v=20;\
             var s=0;for(var i=0;i<200;i++){s+=a.m();s+=b.m()}s"
        ),
        "6400"
    );
}

/// Absent appris (`Absent`) : feature-detect `o.noSuch` en boucle
/// → `undefined` sans scan ; ajout ultérieur de la clé sur le
/// proto (shape du maillon change) → miss → relearn → trouvée.
#[test]
fn c504_absent_then_proto_growth() {
    assert_eq!(
        eval(
            "var p={};var o=Object.create(p);var hits=0;\
             for(var i=0;i<300;i++){if(o.feat)hits++}\
             var mid=hits;p.feat=42;\
             for(var i=0;i<300;i++){if(o.feat)hits++}\
             mid+'/'+hits"
        ),
        "0/300"
    );
}

/// Ombre own-prop posée après apprentissage `Chain` : le shape de
/// la tête change → miss → l'own-prop prime dès la lecture
/// suivante (jamais la valeur du proto périmée).
#[test]
fn c504_shadow_after_chain() {
    assert_eq!(
        eval(
            "var p={x:1};var o=Object.create(p);var r='';\
             for(var i=0;i<50;i++)r+=o.x;o.x=9;\
             for(var i=0;i<50;i++)r+=o.x;\
             var e='';for(var i=0;i<50;i++)e+='1';\
             for(var i=0;i<50;i++)e+='9';r.length+'/'+(r===e)"
        ),
        "100/true"
    );
}

/// Accessor sur le proto (pas own) : le getter s'exécute à chaque
/// lecture — `Prop::Acc` n'est jamais appris.
#[test]
fn c504_proto_accessor_not_cached() {
    assert_eq!(
        eval(
            "var p={n:0};Object.defineProperty(p,'g',{get:function(){return ++this.n},enumerable:true});\
             var o=Object.create(p);o.g+','+o.g+','+o.g"
        ),
        "1,2,3"
    );
}

/// Chaîne > `IC_CHAIN_CAP` (9 maillons) : pas d'apprentissage,
/// la marche complète rend toujours la bonne valeur.
#[test]
fn c504_deep_chain_no_learn() {
    assert_eq!(
        eval(
            "var o={x:7};for(var i=0;i<10;i++)o=Object.create(o);\
             var s=0;for(var i=0;i<200;i++)s+=o.x;s"
        ),
        "1400"
    );
}

/// `o.x` absent avec proto `null` terminal : `Object.create(null)`
/// — entrée `Absent` à un maillon, `undefined` répété.
#[test]
fn c504_absent_null_proto() {
    assert_eq!(
        eval(
            "var o=Object.create(null);var c=0;\
             for(var i=0;i<100;i++){if(o.q===undefined)c++}c"
        ),
        "100"
    );
}

// ── JS-C505 — sondes N5.6 ────────────────────────────────────────

/// Les compteurs `sm_*` et `binop:*` émettent : `sm_oob` (lecture
/// `let` avant déclaration — slot résolu, cellule absente →
/// fallback correct), `badd_num`/`badd_str`, histogramme `binop:`.
#[test]
fn c505_probes_emit() {
    let _g = IC_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    explorer_jscore::j17_enable();
    let mut e = Engine::new();
    let r = e
        .eval(
            "function f(){let x=1;return x+1};var n=0,s='';\
             for(var i=0;i<50;i++){n+=f();s+='a'}\
             var z;for(var i=0;i<10;i++){z===undefined?'u':'?'}\
             var q=7;function g(){return q;let q=5}g();\
             n+'/'+s.length",
        )
        .expect("eval");
    assert_eq!(e.describe(r), "100/50");
    let snap = explorer_jscore::j17_snapshot();
    assert!(snap.contains("sm_oob="), "sm_oob attendu : {snap}");
    assert!(snap.contains("badd_num="), "badd_num attendu : {snap}");
    assert!(snap.contains("badd_str="), "badd_str attendu : {snap}");
    assert!(snap.contains("binop:"), "histogramme attendu : {snap}");
    assert!(snap.contains("add="), "op add attendu : {snap}");
}

// ── JS-C506 — IC par callsite Rust interne ──────────────────────

/// `#[track_caller]` sur `get_prop`/`get_prop_value`/`read_prop` :
/// la coercition `Number(o)` lit `@@toPrimitive` (absent → entrée
/// `Absent`) puis `valueOf` (own → entrée `Own`) aux MÊMES callsites
/// internes — `ic_hit` doit croître comme pour les sites AST, et la
/// valeur reste exacte.
#[test]
fn c506_internal_callsite_ic() {
    let _g = IC_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    explorer_jscore::j17_enable();
    let mut e = Engine::new();
    let r = e
        .eval("var o={valueOf:function(){return 5}};var s=0;for(var i=0;i<1000;i++)s+=Number(o);s")
        .expect("eval");
    assert_eq!(e.describe(r), "5000");
    let snap = explorer_jscore::j17_snapshot();
    let hit = snap
        .split(' ')
        .find_map(|p| {
            p.strip_prefix("ic_hit=")
                .and_then(|v| v.parse::<u64>().ok())
        })
        .unwrap_or(0);
    assert!(hit >= 1000, "ic_hit interne attendu : {snap}");
}

/// Un appelant interne polymorphe (clé tournante) se désarme seul
/// — les valeurs restent exactes quel que soit l'état du cache.
#[test]
fn c506_internal_polymorphic_correct() {
    assert_eq!(
        eval(
            "var o={};for(var i=0;i<20;i++)o['p'+i]=i;\
             var s=0;for(var i=0;i<200;i++){s+=JSON.stringify(o['p'+(i%20)]).length}\
             var t=0;for(var i=0;i<200;i++){t+=o['p'+(i%19)]}s+'/'+t"
        ),
        "300/1755"
    );
}

// ── JS-C507 — IC pour `'k' in o` (has_prop_at) ──────────────────

/// `in` au site AST : `Hit(Data)`→true, `Absent`→false — les deux
/// verdicts sont cachés et l'invalidation par croissance tient
/// (prop ajoutée après learn `Absent` → shape salie → miss →
/// relearn, jamais un `false` périmé).
#[test]
fn c507_in_site_ic() {
    assert_eq!(
        eval(
            "var o={a:1};var s=0;for(var i=0;i<300;i++){if('a' in o)s++;}\
             var t=0;for(var i=0;i<300;i++){if(!('z' in o))t++;}\
             o.z=9;var u=0;for(var i=0;i<300;i++){if('z' in o)u++;}s+'/'+t+'/'+u"
        ),
        "300/300/300"
    );
}

/// `in` via un proto : `Chain` prouvé → `true` ; et un objet hôte
/// n'est jamais tranché par `Absent` (l'hôte peut résoudre hors
/// `props[]`).
#[test]
fn c507_in_proto_chain() {
    assert_eq!(
        eval(
            "var p={m:4};var o=Object.create(p);o.n=2;\
             var s=0;for(var i=0;i<300;i++){if('m' in o)s++;if('n' in o)s+=10;}s"
        ),
        "3300"
    );
}

// ── JS-C508 — économie du learn : miss survit, tombales, purge ──

fn snap_counter(snap: &str, name: &str) -> u64 {
    snap.split(' ')
        .find_map(|p| p.strip_prefix(name).and_then(|v| v.parse::<u64>().ok()))
        .unwrap_or(0)
}

/// Régression N5.7 : un site alternant deux shapes thrashait
/// `miss → relearn(miss:=0)` sans jamais désarmer. Désormais le
/// relearn préserve `miss` → 8 échecs consécutifs → désarmement.
#[test]
fn c508_polymorphic_disarms() {
    let _g = IC_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    explorer_jscore::j17_enable();
    let before = snap_counter(&explorer_jscore::j17_snapshot(), "ic_disarm=");
    assert_eq!(
        eval(
            "var a={x:1},b={y:9,x:2};var s=0;\
             for(var i=0;i<200;i++){var o=i%2?a:b;s+=o.x}s"
        ),
        "300"
    );
    let after = snap_counter(&explorer_jscore::j17_snapshot(), "ic_disarm=");
    assert!(
        after > before,
        "le site polymorphe doit désarmer : {before} → {after}"
    );
}

/// Une clé tournante sur props absentes (helper `o[k]`) compte un
/// miss par rotation → désarme au lieu de relearn `Absent` à
/// l'infini. Les valeurs restent exactes.
#[test]
fn c508_rotating_key_disarms() {
    let _g = IC_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    explorer_jscore::j17_enable();
    let before = snap_counter(&explorer_jscore::j17_snapshot(), "ic_disarm=");
    assert_eq!(
        eval(
            "function f(o,k){return o[k]}var o={a:1};var s=0;\
             for(var i=0;i<200;i++){var v=f(o,'k'+i);if(v===undefined)s++;}s"
        ),
        "200"
    );
    let after = snap_counter(&explorer_jscore::j17_snapshot(), "ic_disarm=");
    assert!(
        after > before,
        "clé tournante doit désarmer : {before} → {after}"
    );
}

/// Site inapprenable (maillon `Proxy` dans la chaîne) : la
/// pierre tombale `Dead` borne les tentatives — `ic_nlearn`
/// s'incrémente puis le site cesse de réessayer (désarmé) —
/// sémantique intacte via la délégation proxy.
#[test]
fn c508_unlearnable_tombstones() {
    let _g = IC_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    explorer_jscore::j17_enable();
    let before = snap_counter(&explorer_jscore::j17_snapshot(), "ic_nlearn=");
    assert_eq!(
        eval(
            "var t={m:5};var p=new Proxy(t,{});var o=Object.create(p);\
             var s=0;for(var i=0;i<300;i++){if(o.m===undefined)s++;}s"
        ),
        "300"
    );
    let after = snap_counter(&explorer_jscore::j17_snapshot(), "ic_nlearn=");
    assert!(
        after > before,
        "learn-échecs attendus puis tombale : {before} → {after}"
    );
}

// ── JS-C509 — bail early-exit + attribution prop_scan ───────────

/// Une tête inéligible par construction (`a.length`, `a[i]`)
/// baille avant toute machinerie — `ic_bail` s'incrémente et les
/// valeurs restent exactes, sans learn ni tombale inutile.
#[test]
fn c509_bail_early_exit() {
    let _g = IC_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    explorer_jscore::j17_enable();
    let before = snap_counter(&explorer_jscore::j17_snapshot(), "ic_bail=");
    assert_eq!(
        eval(
            "var a=[1,2,3];var s=0;\
             for(var i=0;i<300;i++){s+=a.length+a[i%3]}s"
        ),
        "1500"
    );
    let after = snap_counter(&explorer_jscore::j17_snapshot(), "ic_bail=");
    assert!(
        after > before,
        "bail attendu sur tête inéligible : {before} → {after}"
    );
}

/// Les sondes d'attribution `prop_scan` émettent : lectures et
/// `in` alimentent leurs classes.
#[test]
fn c509_scan_attrib_emits() {
    let _g = IC_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    explorer_jscore::j17_enable();
    assert_eq!(
        eval(
            "var o={a:1,b:2,c:3,d:4};var s=0;\
             for(var i=0;i<50;i++){s+=o.zz||0;if('q' in o)s++;}s"
        ),
        "0"
    );
    let snap = explorer_jscore::j17_snapshot();
    assert!(snap.contains("ps_get="), "ps_get attendu : {snap}");
    assert!(snap.contains("ps_has="), "ps_has attendu : {snap}");
    assert!(snap.contains("ps_own="), "ps_own attendu : {snap}");
    assert!(snap.contains("ps_set="), "ps_set attendu : {snap}");
    assert!(snap.contains("pl_get="), "pl_get attendu : {snap}");
    assert!(snap.contains("pl_has="), "pl_has attendu : {snap}");
}

/// JS-C513 — `foai:` attribue les `find_own_ai` par callsite
/// `file:line` dans le snapshot.
#[test]
fn c513_foai_attrib_emits() {
    let _g = IC_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    explorer_jscore::j17_enable();
    assert_eq!(
        eval("var o={a:1};var s=0;for(var i=0;i<50;i++){if(o.hasOwnProperty('a'))s++;}s"),
        "50"
    );
    let snap = explorer_jscore::j17_snapshot();
    assert!(snap.contains("foai:"), "foai attendu : {snap}");
}

/// JS-C514 — write-IC : le fast-path `Hit`/`Absent` → `set_prop`
/// direct préserve la sémantique `[[Set]]` (setter hérité et
/// non-writable hérité interceptent toujours).
#[test]
fn c514_write_ic_semantics() {
    let _g = IC_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    explorer_jscore::j17_enable();
    // Own-update répété : learn `Own` au 1er, `Hit` au 2nd+ —
    // la valeur écrite reste exacte.
    assert_eq!(
        eval("var s=0;for(var i=0;i<20;i++){var o={x:0};o.x=i;s=o.x;}s"),
        "19"
    );
    // Absent→create répété : learn `Absent`, écritures servies
    // par `set_prop` direct — la prop est bien créée.
    assert_eq!(
        eval("var s=0;for(var i=0;i<20;i++){var o={a:1};o.nw=i;s+=o.nw;}s"),
        "190"
    );
    // Setter hérité : jamais appris (Acc non-IC) → l'interception
    // survit au write-IC, appelée avec receiver=derive.
    assert_eq!(
        eval("var log=0;var p={set x(v){log=v}};for(var i=0;i<20;i++){var o=Object.create(p);o.x=i;}log"),
        "19"
    );
    // Donnée héritée non-writable : refus sloppy silencieux —
    // aucune prop propre n'est créée et `o.x` reste l'héritée.
    assert_eq!(
        eval("var p={};Object.defineProperty(p,'x',{value:1,writable:false});var s=0;for(var i=0;i<10;i++){var o=Object.create(p);o.x=9;s+=o.hasOwnProperty('x')?1:0;}s"),
        "0"
    );
    assert_eq!(
        eval("var p={};Object.defineProperty(p,'x',{value:1,writable:false});var o=Object.create(p);o.x=9;o.x"),
        "1"
    );
    // Hit propre non-writable : `set_prop` refuse correctement.
    assert_eq!(
        eval("var o={};Object.defineProperty(o,'x',{value:1,writable:false});o.x=9;o.x"),
        "1"
    );
}
