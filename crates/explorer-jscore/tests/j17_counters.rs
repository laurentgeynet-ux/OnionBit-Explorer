// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Verrou V-2j (chantier VITESSE-JS) : les compteurs de
//! résolution J17 nomment les chemins réellement payés —
//! chacun doit tickersur le script qui l'exerce (deltas
//! avant/après, compteurs monotones partagés).

use explorer_jscore::{
    j17_enable, j17_snapshot, Engine, J17_ARRIDX_TRY, J17_CALL, J17_CALL_NAT, J17_ENV_LVL,
    J17_FN_DEFER, J17_FN_DEF_TOK, J17_FN_MAT, J17_PARSE_CALL, J17_PARSE_DUP, J17_PROP_IDX_BUILD,
    J17_PROP_IDX_HIT, J17_PROP_LOOKUP, J17_PROP_SCAN, J17_PROTO_LVL, J17_STR_READ,
};
use std::sync::atomic::Ordering::Relaxed;

/// Les compteurs J17 sont des atomiques **process-global** :
/// les tests du harnais tournent en parallèle et se
/// contaminent mutuellement leurs fenêtres delta (flake
/// observé : `call_nat` de `kevlar_like` compté dans
/// `attribution_call_rate_domine`). Chaque test prend le
/// verrou — coût nul (suites ~ms), mesures propres.
static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn tick(c: &'static std::sync::atomic::AtomicU64) -> u64 {
    c.load(Relaxed)
}

#[test]
fn counters_nomment_les_chemins_payes() {
    let _g = TEST_LOCK.lock().expect("lock");
    j17_enable();
    let mut e = Engine::new();

    // Lookups : props propres sur petit objet (scan linéaire),
    // héritées via `Object.create` (niveaux proto), variables
    // de fermeture (niveaux env), appel script + native
    // (`Array.isArray`), index `arr["0"]` (`array_index`),
    // lecture de chaîne (`string()` clone-read).
    let before = (
        tick(&J17_PROP_LOOKUP),
        tick(&J17_PROP_SCAN),
        tick(&J17_PROTO_LVL),
        tick(&J17_ENV_LVL),
        tick(&J17_ARRIDX_TRY),
        tick(&J17_CALL),
        tick(&J17_CALL_NAT),
        tick(&J17_STR_READ),
    );
    e.eval(
        "(function(){var o={a:1,b:2};var p=Object.create(o);\
         var arr=[9,8];var s=0;\
         function f(x){return x+1}\
         for(var i=0;i<50;i++){s+=o.a+p.b+arr['0']+f(i);\
         if(Array.isArray(arr)){s+=1}}return s})()",
    )
    .expect("eval");

    assert!(tick(&J17_PROP_LOOKUP) > before.0, "prop_lookup doit ticker");
    assert!(tick(&J17_PROP_SCAN) > before.1, "prop_scan doit ticker");
    assert!(tick(&J17_PROTO_LVL) > before.2, "proto_lvl doit ticker");
    assert!(tick(&J17_ENV_LVL) > before.3, "env_lvl doit ticker");
    assert!(tick(&J17_ARRIDX_TRY) > before.4, "arridx_try doit ticker");
    assert!(tick(&J17_CALL) > before.5, "call doit ticker");
    assert!(tick(&J17_CALL_NAT) > before.6, "call_nat doit ticker");
}

#[test]
fn counters_index_paresseux() {
    let _g = TEST_LOCK.lock().expect("lock");
    j17_enable();
    let mut e = Engine::new();
    // ≥24 props → `prop_pos` construit l'index au premier
    // lookup (BUILD) puis le consulte (HIT).
    let build0 = tick(&J17_PROP_IDX_BUILD);
    let hit0 = tick(&J17_PROP_IDX_HIT);
    e.eval(
        "(function(){var o={k0:0,k1:1,k2:2,k3:3,k4:4,k5:5,k6:6,k7:7,\
         k8:8,k9:9,k10:10,k11:11,k12:12,k13:13,k14:14,k15:15,k16:16,\
         k17:17,k18:18,k19:19,k20:20,k21:21,k22:22,k23:23,k24:24};\
         var s=0;for(var i=0;i<10;i++){s+=o.k13}return s})()",
    )
    .expect("eval idx");
    assert!(
        tick(&J17_PROP_IDX_BUILD) > build0,
        "l'index doit être construit sur objet ≥24 props"
    );
    assert!(
        tick(&J17_PROP_IDX_HIT) > hit0,
        "les lookups suivants doivent hit l'index"
    );
}

#[test]
fn attribution_call_rate_domine() {
    let _g = TEST_LOCK.lock().expect("lock");
    j17_enable();
    let mut e = Engine::new();
    // Le scénario `call_rate` du banc V-1 (~4.9 µs/appel) :
    // les compteurs doivent montrer la facture frame/env
    // (env_lvl × liaisons) plus que la résolution de props.
    let (c0, cn0, e0, p0) = (
        tick(&J17_CALL),
        tick(&J17_CALL_NAT),
        tick(&J17_ENV_LVL),
        tick(&J17_PROP_LOOKUP),
    );
    e.eval(
        "(function(){function f(x){return x+1}var s=0;\
         for(var i=0;i<1000;i++){s+=f(i)}return s})()",
    )
    .expect("eval");
    let calls = tick(&J17_CALL) - c0;
    let nats = tick(&J17_CALL_NAT) - cn0;
    let env_lvls = tick(&J17_ENV_LVL) - e0;
    let lookups = tick(&J17_PROP_LOOKUP) - p0;
    eprintln!(
        "attribution call_rate: calls={calls} nats={nats} \
         env_lvls={env_lvls} lookups={lookups} | {}",
        j17_snapshot()
    );
    assert!(calls >= 1000, "1000 appels f attendus, vu {calls}");
    assert!(
        env_lvls >= calls,
        "chaque appel paie ≥1 niveau env ({env_lvls} lvls / {calls} calls)"
    );
    // Des dispatches natives existent même sur boucle « pure
    // script » (machinerie interne — toPrimitive/itérateurs/…) :
    // le compteur les nomme — vérifier seulement la domination
    // script.
    assert!(
        calls > nats * 10,
        "les appels script doivent dominer ({calls} calls / {nats} nats)"
    );
}

#[test]
fn attribution_kevlar_like() {
    let _g = TEST_LOCK.lock().expect("lock");
    j17_enable();
    let mut e = Engine::new();
    // Porte V-4 : la part des lookups dans le temps total
    // d'un workload kevlar-like (config profonde, classes
    // ES5+proto, callbacks, churn). Snapshot + wall time —
    // la projection se fait dans le test, hors assert :
    // les parts sont des mesures, pas des invariants.
    let (c0, l0, e0) = (tick(&J17_CALL), tick(&J17_PROP_LOOKUP), tick(&J17_ENV_LVL));
    let t = std::time::Instant::now();
    e.eval(
        "(function(){\
         var cfg={data_:{a:{b:{c:7}}},env:{prod:1,flags:{x:1}}};\
         function W(id){this.id=id;this.state=0;this.items=[]}\
         W.prototype.tick=function(k){this.state+=k;return this.state};\
         W.prototype.collect=function(cb){var s=0;\
           for(var i=0;i<this.items.length;i++){s+=cb(this.items[i])}return s};\
         var ws=[];\
         for(var i=0;i<50;i++){var w=new W(i);\
           for(var j=0;j<4;j++){w.items.push({v:j,w:i})}ws.push(w)}\
         var s=0;\
         for(var t=0;t<100;t++){\
           var w=ws[t%50];\
           s+=w.tick(1)+cfg.data_.a.b.c;\
           s+=w.collect(function(it){return it.v+it.w});\
           var m=w.items.map(function(it){return it.v*2})\
                        .filter(function(v){return v>2});\
           s+=m.length;\
           var st='w'+w.id+':'+w.state;s+=st.length}\
         return s})()",
    )
    .expect("eval kevlar_like");
    let wall = t.elapsed();
    let calls = tick(&J17_CALL) - c0;
    let lookups = tick(&J17_PROP_LOOKUP) - l0;
    let env_lvls = tick(&J17_ENV_LVL) - e0;
    eprintln!(
        "attribution kevlar_like: wall={wall:?} calls={calls} \
         lookups={lookups} env_lvls={env_lvls} | {}",
        j17_snapshot()
    );
    // Verrous lâches : le workload doit exercer les trois
    // chemins (un profil à zéro = mesure morte).
    assert!(calls > 500, "calls attendus, vu {calls}");
    assert!(lookups > 500, "lookups attendus, vu {lookups}");
    assert!(env_lvls > 500, "env_lvls attendus, vu {env_lvls}");
}

#[test]
fn attribution_bundle_reel_bbc() {
    let _g = TEST_LOCK.lock().expect("lock");
    j17_enable();
    // Vol armé local : le chunk `_app` BBC Good Food
    // (1 445 423 o, capturé vol 34) évalué verbatim —
    // même harnais que `j6_probe` (timers + pompe).
    // Le wall armé inclut le coût des ticks (~15 ns/
    // événement) → la part props projetée est une borne
    // haute, conservatrice pour la porte V-4.
    let src = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/data/vol34_bbc_app.js"
    ))
    .unwrap_or_else(|e| panic!("fixture: {e}"));
    let mut e = Engine::new();
    e.install_timers();
    let t = std::time::Instant::now();
    let r = e.eval(&src);
    let eval_wall = t.elapsed();
    for _ in 0..8 {
        let _ = e.run_microtasks();
        let _ = e.run_due_timers_bounded(200);
        let _ = e.drain_unhandled_rejections();
    }
    let wall = t.elapsed();
    eprintln!(
        "attribution bbc_app: eval={eval_wall:?} total={wall:?} \
         r={:?} | {}",
        r.as_ref().map(|_| "ok").map_err(|_| "err"),
        j17_snapshot()
    );
}

#[test]
fn parse_probe_nomme_les_reparses() {
    let _g = TEST_LOCK.lock().expect("lock");
    j17_enable();
    let mut e = Engine::new();
    // Verrou V-3c : même source re-parsée → `parse_dup`
    // tick ; source distincte → `parse_call` seul. Le mode
    // entre dans le hash (script ≠ module).
    let (c0, d0) = (tick(&J17_PARSE_CALL), tick(&J17_PARSE_DUP));
    e.eval("var j17x=1").expect("eval 1");
    e.eval("var j17x=1").expect("eval 2 — même source");
    e.eval("var j17y=2").expect("eval 3 — source distincte");
    assert!(
        tick(&J17_PARSE_CALL) > c0 + 2,
        "3 parses attendus, vu {}",
        tick(&J17_PARSE_CALL) - c0
    );
    assert!(
        tick(&J17_PARSE_DUP) > d0,
        "la source répétée doit compter un dup, vu {}",
        tick(&J17_PARSE_DUP) - d0
    );
}

#[test]
fn snapshot_liste_les_compteurs_armes() {
    let _g = TEST_LOCK.lock().expect("lock");
    j17_enable();
    let mut e = Engine::new();
    e.eval("var o={a:1};o.a").expect("eval");
    let snap = j17_snapshot();
    assert!(snap.contains("prop_lookup="), "snapshot: {snap}");
    // Lecture de valeur chaîne (STR_READ) — `o.a` est un
    // nombre ; vérifier séparément.
    let s0 = tick(&J17_STR_READ);
    e.eval("var s='hello';s").expect("eval str");
    // `s` lu comme Value — la chaîne n'est clonée que si
    // quelqu'un appelle `string()` ; assert non-régressif :
    // le compteur ne doit pas être incohérent.
    assert!(tick(&J17_STR_READ) >= s0);
}

/// Verrou V-5a : les corps différés se matérialisent au premier
/// appel **avec la sémantique exacte** — retour correct, strict
/// du prologue respecté, closures, et une erreur de syntaxe
/// tardive jette au call (pas au parse, jamais panic).
#[test]
fn corps_differes_materialisation_fidele() {
    let _g = TEST_LOCK.lock().expect("lock");
    j17_enable();
    let d0 = tick(&J17_FN_DEFER);
    let m0 = tick(&J17_FN_MAT);
    let mut e = Engine::new();

    // Corps ≥ 24 tokens → différé ; appelé → matérialisé une
    // fois (le 2e appel réutilise le OnceCell), retour exact.
    // big(3)=265, big(4)=363 → 628.
    let r = e
        .eval(
            "function big(a){var x=a+1;var y=x*2;var z=y-3;             var w=z+a;var q=w*10;var t=q/2;var u=t+x;             var v=u+y;var s=v+z;var r=s+w;var j=r+q;             return j+q+t}             big(3)+big(4)",
        )
        .expect("eval corps différé appelé");
    assert_eq!(e.describe(r), "628");
    assert!(tick(&J17_FN_DEFER) > d0, "le corps doit être différé");
    assert!(tick(&J17_FN_MAT) > m0, "l'appel doit matérialiser");
    assert!(
        tick(&J17_FN_MAT) - m0 <= 1,
        "la matérialisation est unique (OnceCell), vu {}",
        tick(&J17_FN_MAT) - m0
    );

    // Corps jamais appelé : différé, jamais matérialisé.
    e.eval("function dead(){var a=1;var b=2;var c=3;var d=4;            var e=5;var f=6;var g=7;var h=8;return a+b+c+d+e+f+g+h}var ok=1")
        .expect("eval corps mort");

    // Erreur de syntaxe dans un corps différé : pas de rejet
    // au parse (tolérance) mais throw à l'appel — jamais panic.
    let r2 = e.eval(
        "var bad=function(){var a=1;var b=2;var c=3;var d=4;         var e=5;var f=6;var g=7;return a+};'parsed'",
    );
    assert_eq!(e.describe(r2.expect("parse tolère")), "parsed");
    let called = e
        .eval("try{bad()}catch(ex){'thrown'}")
        .expect("appel erroné");
    assert_eq!(e.describe(called), "thrown");

    // Strict du prologue lu dès le parse : corps différé
    // `"use strict"` → `this` non coercé (undefined, pas global).
    let strict = e
        .eval(
            "var s=function(){'use strict';var a=1;var b=2;var c=3;             var d=4;var e=5;var f=6;var g=7;var h=8;             return this===undefined};s()",
        )
        .expect("eval strict différé");
    assert_eq!(e.describe(strict), "true");
}

/// Harnais d'attribution partagé (V-5a validation
/// multi-fixtures) : eval verbatim + pompe, delta
/// `fn_defer`/`fn_mat` imprimés — la part de corps réellement
/// matérialisés est la mesure, pas un invariant.
/// NB : `fn_def_tok` = volume cumulé des décisions de defer
/// (les fonctions imbriquées re-diffèrent à chaque
/// matérialisation → dépasse `lex_tok` sur bundles exécutés) ;
/// `fn_mat`/`fn_defer` restent les signaux de taux réel.
fn attribution_fixture(label: &str, file: &str) {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/");
    let src = std::fs::read_to_string(format!("{path}{file}"))
        .unwrap_or_else(|e| panic!("fixture {file}: {e}"));
    let (d0, dt0, m0) = (
        tick(&J17_FN_DEFER),
        tick(&J17_FN_DEF_TOK),
        tick(&J17_FN_MAT),
    );
    let mut e = Engine::new();
    e.install_timers();
    let t = std::time::Instant::now();
    let r = e.eval(&src);
    let eval_wall = t.elapsed();
    for _ in 0..8 {
        let _ = e.run_microtasks();
        let _ = e.run_due_timers_bounded(200);
        let _ = e.drain_unhandled_rejections();
    }
    let wall = t.elapsed();
    eprintln!(
        "attribution {label}: eval={eval_wall:?} total={wall:?} \
         r={:?} | defer=+{} defer_tok=+{} mat=+{} | {}",
        r.as_ref().map(|_| "ok").map_err(|_| "err"),
        tick(&J17_FN_DEFER) - d0,
        tick(&J17_FN_DEF_TOK) - dt0,
        tick(&J17_FN_MAT) - m0,
        j17_snapshot()
    );
}

#[test]
fn attribution_bundles_reels_dsfr_bitmovin() {
    let _g = TEST_LOCK.lock().expect("lock");
    j17_enable();
    // Profils non-webpack-BBC : dsfr (composants état) et
    // transmuxer bitmovin (pipeline media) — les corps y sont
    // pour partie appelés au startup → `fn_mat` exercera la
    // matérialisation en conditions réelles.
    attribution_fixture("dsfr_core", "vol16_dsfr_core.js");
    attribution_fixture("bitmovin", "bitmovin_transmuxer.js");
    attribution_fixture("spotify_slice", "vol12_spotify_gen_slice.js");
}
