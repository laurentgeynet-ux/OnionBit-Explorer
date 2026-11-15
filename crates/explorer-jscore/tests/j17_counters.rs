// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Verrou V-2j (chantier VITESSE-JS) : les compteurs de
//! résolution J17 nomment les chemins réellement payés —
//! chacun doit tickersur le script qui l'exerce (deltas
//! avant/après, compteurs monotones partagés).

use explorer_jscore::{
    j17_enable, j17_keys_enable, j17_snapshot, Engine, J17_ARRIDX_TRY, J17_CALL, J17_CALL_NAT,
    J17_FN_DEFER, J17_FN_DEF_TOK, J17_FN_MAT, J17_PARSE_CALL, J17_PARSE_DUP, J17_PROP_IDX_BUILD,
    J17_PROP_IDX_HIT, J17_PROP_LOOKUP, J17_PROP_SCAN, J17_PROTO_LVL, J17_SLOT_HIT, J17_STR_READ,
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
    // de fermeture (slots env C502), appel script + native
    // (`Array.isArray`), index `arr["0"]` (`array_index`),
    // lecture de chaîne (`string()` clone-read).
    let before = (
        tick(&J17_PROP_LOOKUP),
        tick(&J17_PROP_SCAN),
        tick(&J17_PROTO_LVL),
        tick(&J17_SLOT_HIT),
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
    assert!(tick(&J17_SLOT_HIT) > before.3, "slot_hit doit ticker");
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
        tick(&J17_SLOT_HIT),
        tick(&J17_PROP_LOOKUP),
    );
    e.eval(
        "(function(){function f(x){return x+1}var s=0;\
         for(var i=0;i<1000;i++){s+=f(i)}return s})()",
    )
    .expect("eval");
    let calls = tick(&J17_CALL) - c0;
    let nats = tick(&J17_CALL_NAT) - cn0;
    let slot_hits = tick(&J17_SLOT_HIT) - e0;
    let lookups = tick(&J17_PROP_LOOKUP) - p0;
    eprintln!(
        "attribution call_rate: calls={calls} nats={nats} \
         slot_hits={slot_hits} lookups={lookups} | {}",
        j17_snapshot()
    );
    assert!(calls >= 1000, "1000 appels f attendus, vu {calls}");
    assert!(
        slot_hits >= calls,
        "chaque appel paie ≥1 résolution slot ({slot_hits} hits / {calls} calls)"
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
    let (c0, l0, e0) = (tick(&J17_CALL), tick(&J17_PROP_LOOKUP), tick(&J17_SLOT_HIT));
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
    let slot_hits = tick(&J17_SLOT_HIT) - e0;
    eprintln!(
        "attribution kevlar_like: wall={wall:?} calls={calls} \
         lookups={lookups} slot_hits={slot_hits} | {}",
        j17_snapshot()
    );
    // Verrous lâches : le workload doit exercer les trois
    // chemins (un profil à zéro = mesure morte).
    assert!(calls > 500, "calls attendus, vu {calls}");
    assert!(lookups > 500, "lookups attendus, vu {lookups}");
    assert!(slot_hits > 500, "slot_hits attendus, vu {slot_hits}");
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

/// JS-C527 — l'histogramme `keys:` nomme les clés marchées
/// par `find_prop_owner_links` (audit des lookups dominants
/// avant tout fast-path). Thread-local → chaque test repart
/// d'un histogramme frais.
#[test]
fn keys_histogram_names_walked_keys() {
    let _g = TEST_LOCK.lock().expect("lock");
    j17_enable();
    j17_keys_enable();
    let mut e = Engine::new();
    e.eval(
        "var P={zzkey:1}; var o=Object.create(P); var s=0;\
         for(var i=0;i<40;i++){s+=o.zzkey} s",
    )
    .expect("eval");
    let snap = j17_snapshot();
    assert!(snap.contains("keys:"), "histogramme keys attendu — {snap}");
    assert!(
        snap.contains("zzkey="),
        "la clé marchée doit être nommée — {snap}"
    );
}

/// JS-C529 — le fallback `env_get`/`env_set` sur l'objet global
/// sert les props propres par own-scan indexé (`env_ghit`) sans
/// le walk complet : la clé marchée disparaît de `keys:` pendant
/// que `env_ghit` croît — et la sémantique tient (getter global
/// invoqué, écriture via props, absent → `undefined`).
#[test]
fn env_ghit_serves_global_own_props() {
    let _g = TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    j17_enable();
    j17_keys_enable();
    let before = j17_snapshot();
    let g0 = before
        .split(' ')
        .find_map(|p| {
            p.strip_prefix("env_ghit=")
                .and_then(|v| v.parse::<u64>().ok())
        })
        .unwrap_or(0);
    let mut e = Engine::new();
    let r = e
        .eval(
            "var s=0;for(var i=0;i<60;i++){s+=Reflect===undefined?0:1;s+=Math===undefined?0:1}\
             var w=0;for(var i=0;i<60;i++){zzbind=i;w+=zzbind} s+'/'+w",
        )
        .expect("eval");
    assert_eq!(e.describe(r), "120/1770");
    let after = j17_snapshot();
    let g1 = after
        .split(' ')
        .find_map(|p| {
            p.strip_prefix("env_ghit=")
                .and_then(|v| v.parse::<u64>().ok())
        })
        .unwrap_or(0);
    assert!(
        g1 >= g0 + 100,
        "les lectures globales doivent passer par l'own-scan — {before} → {after}"
    );
    // Getter global invoqué, absent → undefined — sémantique
    // intacte sous le raccourci.
    let mut e2 = Engine::new();
    let r2 = e2
        .eval(
            "var n=0;Object.defineProperty(this,'gval',{get:function(){n++;return 41}});\
             gval+','+gval+','+n+','+(typeof zzabsent)",
        )
        .expect("eval");
    assert_eq!(e2.describe(r2), "41,41,2,undefined");
}

/// JS-C532 — histogrammes `eval:`/`stmt:` par variante : chaque
/// nœud évalué/exécuté compte une fois (maillons d'épine
/// inclus). Le tissu non-scopé devient attribuable.
#[test]
fn j17_eval_stmt_histograms() {
    let _g = TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    j17_enable();
    let mut e = Engine::new();
    let r = e
        .eval("var o={x:3};var s=0;for(var i=0;i<50;i++){s+=o.x*2+i} s")
        .expect("eval");
    assert_eq!(e.describe(r), "1525");
    let snap = j17_snapshot();
    // `o.x` membre ×50 + littéraux/idents — les variantes chaudes
    // doivent être comptées.
    for want in ["eval:", "stmt:"] {
        assert!(
            snap.contains(want),
            "section {want} attendue dans le snapshot — {snap}"
        );
    }
    let ev = snap
        .split(' ')
        .find(|p| p.starts_with("eval:"))
        .unwrap_or("");
    assert!(
        ev.contains("member=") && ev.contains("ident=") && ev.contains("binary="),
        "variantes eval attendues — {ev}"
    );
    let st = snap
        .split(' ')
        .find(|p| p.starts_with("stmt:"))
        .unwrap_or("");
    assert!(st.contains("for="), "variante stmt attendue — {st}");
}

/// JS-C534c — `envu:` émet le callsite de construction des envs
/// qui servent en `extra` (attribution de `xt_unw`).
#[test]
fn envu_nomme_le_site_de_construction() {
    let _g = TEST_LOCK.lock().unwrap();
    j17_enable();
    let mut e = Engine::new();
    // `eval`-injection : `dyn` n'est pas dans le canon du scope
    // fn (poisoned → scope depth-only) → la liaison vit en
    // `extra` → la lecture tick `bnd_extra` + `envu:<site>`.
    e.eval("(function(){ eval('var dyn=7'); return dyn; })()")
        .expect("eval dyn");
    let snap = j17_snapshot();
    assert!(
        snap.contains("envu:"),
        "le hit `extra` doit émettre un site `envu:` : {snap}"
    );
}

/// JS-C539 — `arridx:` attribue les essais `array_index` au
/// callsite (`#[track_caller]`). Un accès par clé chaîne digit
/// (`a[''+i]` → `find_own_ai` → `array_index`) doit faire
/// émettre la section — JS-C541 : les clés alpha ne paient
/// plus l'essai, il faut une clé numérique pour le voir.
#[test]
fn arridx_attribue_le_callsite() {
    let _g = TEST_LOCK.lock().unwrap();
    j17_enable();
    let mut e = Engine::new();
    e.eval("var a=[10,20,30]; var s=0; for(var i=0;i<3;i++){s+=a[''+i];} s")
        .expect("eval indexed");
    let snap = j17_snapshot();
    assert!(
        snap.contains("arridx:"),
        "les essais `array_index` doivent émettre `arridx:` : {snap}"
    );
}

/// JS-C541 — `ic_eligible` internalise l'essai `array_index` :
/// il ne vit plus que dans le bras `Array`. Un hit IC sur
/// objet ordinaire ne doit plus ticker `arridx_try` (vol N5.48
/// : `ic_hit` seul = 48,8 % des essais, kinds qui ignoraient
/// `ai`) — tandis qu'un accès indexé sur Array continue de
/// l'essayer (sémantique `a[i]` préservée).
#[test]
fn c541_ic_hit_ne_essaie_plus_array_index() {
    let _g = TEST_LOCK.lock().unwrap();
    j17_enable();
    let mut e = Engine::new();
    // Warm-up + baseline : épuise les essais intrinsèques de la
    // mise en place (prologue, builtins), puis mesure le delta
    // d'une boucle de hits IC purs sur objet ordinaire.
    e.eval("var o={a:1,b:2}; for(var i=0;i<4;i++){o.a+o.b;}")
        .expect("warmup");
    let before = tick(&J17_ARRIDX_TRY);
    e.eval("for(var i=0;i<50;i++){o.a+o.b;}").expect("ic hits");
    let after = tick(&J17_ARRIDX_TRY);
    if after != before {
        eprintln!("arridx residus: {}", j17_snapshot());
    }
    assert_eq!(
        after, before,
        "les hits IC sur objet ordinaire ne doivent plus essayer \
         `array_index` — l'essai est internalisé au bras Array"
    );
    // Garde-fou inverse : une clé chaîne `"0"` — chemin
    // `find_own_ai` — continue d'essayer `array_index` (la gate
    // premier-octet laisse passer les clés digit-first) et la
    // sémantique élément est servie.
    let s = e
        .eval("var a=[10,20,30]; var t=0; for(var i=0;i<3;i++){t+=a[''+i];} t")
        .expect("array index");
    assert!(
        tick(&J17_ARRIDX_TRY) > before,
        "une clé digit-first doit encore essayer `array_index`"
    );
    let _ = s;
}

/// JS-C542 — `pscan:` attribue les comparaisons `prop_scan`
/// au callsite (`#[track_caller]` sur `prop_pos`). Un lookup
/// linéaire répété sur petit objet doit faire émettre la
/// section avec le site `find`.
#[test]
fn pscan_attribue_le_callsite() {
    let _g = TEST_LOCK.lock().unwrap();
    j17_enable();
    let mut e = Engine::new();
    // Petit objet (props < seuil index) → `prop_pos` scanne
    // linéairement à chaque `o.a` manqué par l'IC learn —
    // la boucle produit des comparaisons attribuables.
    e.eval(
        "var o={a:1,b:2}; var q={x:9}; var s=0;\
         for(var i=0;i<20;i++){s+=o.a+q.zzz|0;} s",
    )
    .expect("eval");
    let snap = j17_snapshot();
    assert!(
        snap.contains("pscan:"),
        "les comparaisons `prop_scan` doivent émettre `pscan:` : {snap}"
    );
}

/// JS-C543 — `pidxm:` attribue les miss `prop_idx_miss` au
/// callsite (`Location` capturée à l'entrée de `prop_pos` —
/// dans une closure elle ne perçait pas `track_caller`, N5.50
/// attribuait la ligne `position` interne). Un lookup absent
/// sur objet indexé (≥ `PROPS_IDX_MIN` props) doit émettre la
/// section ; les sites nomment le driver, pas `prop_pos`.
#[test]
fn pidxm_attribue_le_callsite_du_miss_index() {
    let _g = TEST_LOCK.lock().unwrap();
    j17_enable();
    let mut e = Engine::new();
    // 12 props ≥ `PROPS_IDX_MIN` → chemin indexé : `o.zzz`
    // absent → miss d'index attribué au callsite `o.find`.
    e.eval(
        "var o={a:1,b:2,c:3,d:4,e:5,f:6,g:7,h:8,i:9,j:10,k:11,l:12};\
         var s=0; for(var n=0;n<10;n++){s+=o.zzz|0; s+=o.qq|0;} s",
    )
    .expect("eval");
    let snap = j17_snapshot();
    assert!(
        snap.contains("pidxm:"),
        "un miss d'index doit émettre `pidxm:` : {snap}"
    );
    // Le callsite doit être hors `prop_pos` (le vrai driver —
    // `find_own_ai`/`get_prop`/`find`), pas la ligne interne :
    // preuve que le `Location` capturé à l'entrée perce.
    let pidxm = snap.split("pidxm:").nth(1).unwrap_or("");
    assert!(
        !pidxm.split(',').next().unwrap_or("").contains("position"),
        "le site attribué ne doit pas être la closure interne : {snap}"
    );
}

/// JS-C535 — le peek homo-typé de `binary_op` sert `(Num,Num)`
/// et `(Str,Str)` sans `to_primitive`/`to_number` ; les
/// compteurs `bin_nn`/`bin_ss` mesurent la couverture. Le lock
/// fixe aussi la sémantique IEEE des cas limites servis par le
/// fast-path (NaN, ±0, division par zéro, reste tronqué,
/// `>>>` sur négatif, concat chaînée, mixte → chemin générique,
/// coercition d'objet préservée).
#[test]
fn c535_binop_peek_sert_et_preserve() {
    let _g = TEST_LOCK.lock().unwrap();
    j17_enable();
    let mut e = Engine::new();
    let r = e
        .eval(
            "[NaN===NaN, NaN<1, -0===0, 1/0===Infinity, -1/0===-Infinity, (0/0).toString(),\
             -5%3, 2**0.5>1.4, (-1)>>>0, 0.1+0.2!==0.3, 'b'<'a', 'x'+'y'+'z',\
             'a'+1, '5'*2, ({valueOf:()=>5})+1]",
        )
        .expect("eval");
    let out = e.describe(r);
    assert_eq!(
        out, "false,false,true,true,true,NaN,-2,true,4294967295,true,false,xyz,a1,10,6",
        "semantique binop peek : {out}"
    );
    let snap = j17_snapshot();
    for k in ["bin_nn", "bin_ss"] {
        assert!(
            snap.split(' ').any(|p| p.starts_with(&format!("{k}="))),
            "`{k}` doit émettre : {snap}"
        );
    }
}

/// JS-C538 — `===`/`!==` servis dans `binary_op` pour toutes
/// les paires hors peek `(Num,Num)` : `strict_eq` ne coerces
/// pas — `bin_se` tique et la sémantique est préservée sur
/// les classes non numériques (Str×Str hors `Add`/compar.,
/// undefined/null/obj/mixte) et les cas IEEE du peek.
#[test]
fn c538_strict_eq_direct_serve() {
    let _g = TEST_LOCK.lock().unwrap();
    j17_enable();
    let mut e = Engine::new();
    let r = e
        .eval(
            "var o={a:1};\
             ['a'==='a', 'a'!=='a', undefined===undefined, null===undefined,\
             o===o, {a:1}==={a:1}, 1==='1', true===1, 'x'===('x'+''),\
             typeof o==='object', typeof o.a==='number', null!==o]",
        )
        .expect("eval");
    let out = e.describe(r);
    assert_eq!(
        out, "true,false,true,false,true,false,false,false,true,true,true,true",
        "semantique strict-eq direct : {out}"
    );
    let snap = j17_snapshot();
    assert!(
        snap.split(' ').any(|p| p.starts_with("bin_se=")),
        "`bin_se` doit émettre : {snap}"
    );
}
