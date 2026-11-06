// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Banc V-1 (chantier VITESSE-JS — `docs/roadmap-vitesse-js.md`) :
//! baselines avant les items V-2 → V-4.
//!
//! Chaque scénario met la boucle **dans** le script évalué —
//! le parse (re-fait à chaque `eval`, sans cache) est amorti
//! sur `N` itérations internes ; le chiffre ns/eval ÷ N ≈
//! coût par opération. `V-1f` (parse) fait l'inverse : source
//! géante à exécution triviale — `parse_in` étant `pub(crate)`,
//! le proxy honnête est un eval parse-dominé.
//!
//! `cargo bench -p explorer-jscore` — résultats sous
//! `target/criterion/`.
//!
//! Baselines (V-1 livré, release bench, 2026-11-06) :
//!   `prop_mono_30k`   33.1 ms  (~1.10 µs/lookup)
//!   `prop_poly3_30k`  52.1 ms  (~1.74 µs/lookup, +58 % vs mono)
//!   `prop_proto3_30k` 36.9 ms  (~1.23 µs/lookup, +12 % pour +3 niveaux)
//!   `env_depth_30k`   30.1 ms  (~1.00 µs/iter, 2 reads à prof. 2+1)
//!   `call_rate_30k`   148.0 ms (~4.93 µs/appel — 4.5× un lookup)
//!   `array_read_30k`  35.2 ms  (~1.17 µs/iter)
//!   `parse_50k_stmts` 53.9 ms  (~1.08 µs/stmt ; tare eval("0") = 1.8 µs)
//!   `composite_1k`    28.6 ms  (28.6 µs/iter, ~5 calls/iter → call-dominé)
//! Premier signal : `call_rate` domine — V-2 devra attribuer
//! frame/env vs résolution avant tout fix structurel.
//!
//! Post V-3a+V-3b (2026-11-06) :
//!   `composite_1k`    25.4 ms  (−11 % cumulé micro-wins)
//!   `kevlar_like_200` 15.45 ms (~77 µs/iter — mix proto/
//!                     callbacks/churn, workload attribution V-2)

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use explorer_jscore::Engine;

/// V-1a — lecture de propriété monomorphe : même objet, même
/// clé, `N` accès. Mesure la marche proto + `prop_pos` par
/// lookup — la cible de V-4.
fn bench_prop_mono(c: &mut Criterion) {
    let mut e = Engine::new();
    c.bench_function("prop_mono_30k", |b| {
        b.iter(|| {
            e.eval(black_box(
                "(function(){var o={a:1,b:2,c:3};var s=0;\
                 for(var i=0;i<10000;i++){s+=o.a+o.b+o.c}return s})()",
            ))
            .expect("prop_mono")
        });
    });
}

/// V-1b — polymorphe : 3 shapes alternées sur le même site
/// d'accès (borne de dégradation de V-4).
fn bench_prop_poly(c: &mut Criterion) {
    let mut e = Engine::new();
    c.bench_function("prop_poly3_30k", |b| {
        b.iter(|| {
            e.eval(black_box(
                "(function(){var os=[{a:1},{b:2,a:1},{c:3,b:4,a:1}];\
                 var s=0;for(var i=0;i<10000;i++){var o=os[i%3];s+=o.a+o.a+o.a}\
                 return s})()",
            ))
            .expect("prop_poly")
        });
    });
}

/// V-1c — profondeur proto : `v` résolu 3 niveaux au-dessus
/// de l'objet lu (`Object.create` chaîné).
fn bench_prop_proto(c: &mut Criterion) {
    let mut e = Engine::new();
    c.bench_function("prop_proto3_30k", |b| {
        b.iter(|| {
            e.eval(black_box(
                "(function(){var a={v:7};var b=Object.create(a);\
                 var c=Object.create(b);var d=Object.create(c);var s=0;\
                 for(var i=0;i<10000;i++){s+=d.v+d.v+d.v}return s})()",
            ))
            .expect("prop_proto")
        });
    });
}

/// V-1d — accès variable en fermeture : `inner` lit `x`
/// (2 scopes au-dessus) et `y` (1) dans la boucle.
fn bench_env_depth(c: &mut Criterion) {
    let mut e = Engine::new();
    c.bench_function("env_depth_30k", |b| {
        b.iter(|| {
            e.eval(black_box(
                "(function(){var x=1;return (function(){var y=2;\
                 return (function(){var s=0;\
                 for(var i=0;i<10000;i++){s+=x+y+1}return s})()})()})()",
            ))
            .expect("env_depth")
        });
    });
}

/// V-1e — taux d'appel : `f` petit corps, `N` invocations —
/// coût frame + env par `call_value`.
fn bench_call_rate(c: &mut Criterion) {
    let mut e = Engine::new();
    c.bench_function("call_rate_30k", |b| {
        b.iter(|| {
            e.eval(black_box(
                "(function(){function f(x){return x+1}var s=0;\
                 for(var i=0;i<30000;i++){s+=f(i)}return s})()",
            ))
            .expect("call_rate")
        });
    });
}

/// V-1h — lecture indexée + `length` : `a[i]`/`a.length` en
/// boucle (chemin `ObjKind::Array` de `find_own`).
fn bench_array_read(c: &mut Criterion) {
    let mut e = Engine::new();
    c.bench_function("array_read_30k", |b| {
        b.iter(|| {
            e.eval(black_box(
                "(function(){var a=[1,2,3,4,5,6,7,8];var s=0;\
                 for(var i=0;i<10000;i++){s+=a[i&7]+a.length}return s})()",
            ))
            .expect("array_read")
        });
    });
}

/// V-1f — parse-dominé : `n` instructions triviales `0;`
/// (parse + bind, exécution quasi nulle) — proxy de
/// `parse_in` (`pub(crate)`, inaccessible au banc) pour
/// décider V-3c. La tare est l'eval d'un programme vide.
fn bench_parse(c: &mut Criterion) {
    let mut e = Engine::new();
    let mk = |n: usize| "0;".repeat(n);
    let src = mk(50_000);
    c.bench_function("parse_50k_stmts", |b| {
        b.iter(|| e.eval(black_box(&src)).expect("parse_50k"));
    });
    c.bench_function("eval_empty_tare", |b| {
        b.iter(|| e.eval(black_box("0")).expect("tare"));
    });
}

/// V-1g — mix composite « page » : littéraux + map + accès +
/// appels — l'ordre de grandeur global, comparaison finale.
fn bench_composite(c: &mut Criterion) {
    let mut e = Engine::new();
    c.bench_function("composite_1k", |b| {
        b.iter(|| {
            e.eval(black_box(
                "(function(){var s=0;for(var i=0;i<1000;i++){\
                 var o={x:i,y:i*2,name:'n'+i};var arr=[o.x,o.y,o.x+o.y];\
                 s+=arr.map(function(v){return v*2}).reduce(function(a,b){return a+b});\
                 s+=o.name.length}return s})()",
            ))
            .expect("composite")
        });
    });
}

/// V-2 attribution — profil « kevlar-like » : config profonde
/// (`ytcfg.data_.a.b.c`), classes ES5 (`function`+`prototype`),
/// dispatch de méthodes, callbacks `map`/`filter`, closures,
/// churn objets et concat — le mix réel que la porte V-4 doit
/// chiffrer (part des lookups dans le temps total).
fn bench_kevlar_like(c: &mut Criterion) {
    let mut e = Engine::new();
    c.bench_function("kevlar_like_200", |b| {
        b.iter(|| {
            e.eval(black_box(
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
                 for(var t=0;t<200;t++){\
                   var w=ws[t%50];\
                   s+=w.tick(1)+cfg.data_.a.b.c;\
                   s+=w.collect(function(it){return it.v+it.w});\
                   var m=w.items.map(function(it){return it.v*2})\
                                .filter(function(v){return v>2});\
                   s+=m.length;\
                   var st='w'+w.id+':'+w.state;s+=st.length}\
                 return s})()",
            ))
            .expect("kevlar_like")
        });
    });
}

criterion_group!(
    benches,
    bench_prop_mono,
    bench_prop_poly,
    bench_prop_proto,
    bench_env_depth,
    bench_call_rate,
    bench_array_read,
    bench_parse,
    bench_composite,
    bench_kevlar_like,
);
criterion_main!(benches);
