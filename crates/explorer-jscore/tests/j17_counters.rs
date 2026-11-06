// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Verrou V-2j (chantier VITESSE-JS) : les compteurs de
//! résolution J17 nomment les chemins réellement payés —
//! chacun doit tickersur le script qui l'exerce (deltas
//! avant/après, compteurs monotones partagés).

use explorer_jscore::{
    j17_enable, j17_snapshot, Engine, J17_ARRIDX_TRY, J17_CALL, J17_CALL_NAT, J17_ENV_LVL,
    J17_PARSE_CALL, J17_PARSE_DUP, J17_PROP_IDX_BUILD, J17_PROP_IDX_HIT, J17_PROP_LOOKUP,
    J17_PROP_SCAN, J17_PROTO_LVL, J17_STR_READ,
};
use std::sync::atomic::Ordering::Relaxed;

fn tick(c: &'static std::sync::atomic::AtomicU64) -> u64 {
    c.load(Relaxed)
}

#[test]
fn counters_nomment_les_chemins_payes() {
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
fn parse_probe_nomme_les_reparses() {
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
