// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Bench AR-6 (chantier ARÈNE — `docs/roadmap-arene.md`) :
//! baselines avant les items perf AR-9 → AR-15.
//!
//! Les scénarios calent les points chauds de l'audit :
//! `children` (Vec alloué par appel — 68 sites en `src`),
//! `attribute` (String clonée par lookup — 124 sites),
//! `collect` sur arène fragmentée (le déclencheur
//! journal, étape 208) et `set_attribute` (scan linéaire
//! du Vec d'attributs).
//!
//! `cargo bench -p explorer-dom` — résultats sous
//! `target/criterion/`.

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use explorer_dom::{Dom, NodeId};

/// Arbre plat : document → `n` éléments, chacun porteur de
/// deux attributs et d'un nœud texte.
fn build_flat(dom: &mut Dom, n: usize) -> Vec<NodeId> {
    let doc = dom.document();
    let mut els = Vec::with_capacity(n);
    for i in 0..n {
        let el = dom.create_element("div").expect("el");
        dom.set_attribute(el, "class", &format!("c{i}"))
            .expect("class");
        dom.set_attribute(el, "id", &format!("n{i}")).expect("id");
        dom.append_child(doc, el).expect("append el");
        let t = dom.create_text("contenu").expect("text");
        dom.append_child(el, t).expect("append text");
        els.push(el);
    }
    els
}

/// Chaîne profonde de `n` éléments (le marquage `collect`
/// est itératif — ce scénario borne sa linéarité).
fn build_deep(dom: &mut Dom, n: usize) -> NodeId {
    let doc = dom.document();
    let mut parent = doc;
    let mut last = doc;
    for _ in 0..n {
        let el = dom.create_element("section").expect("section");
        dom.append_child(parent, el).expect("append");
        parent = el;
        last = el;
    }
    last
}

fn bench_build(c: &mut Criterion) {
    let mut g = c.benchmark_group("build");
    for n in [1_000usize, 10_000] {
        g.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, &n| {
            b.iter(|| {
                let mut dom = Dom::new();
                build_flat(&mut dom, n);
                black_box(&dom);
            });
        });
    }
    g.finish();
}

fn bench_traverse(c: &mut Criterion) {
    let mut dom = Dom::new();
    let els = build_flat(&mut dom, 1_000);
    let doc = dom.document();

    // Vec de 1 000 `NodeId` alloué à chaque appel — la
    // baseline avant AR-9.
    c.bench_function("children_doc_1000", |b| {
        b.iter(|| black_box(dom.children(doc).expect("children")));
    });

    // Un lookup `class` par élément — String clonée par
    // appel : la baseline avant AR-10.
    c.bench_function("attribute_1000", |b| {
        b.iter(|| {
            for el in &els {
                black_box(dom.attribute(*el, "class").expect("attr"));
            }
        });
    });

    // Lecture directe du slice (comparateur sans alloc).
    c.bench_function("attributes_slice_1000", |b| {
        b.iter(|| {
            for el in &els {
                black_box(dom.attributes(*el).expect("attrs"));
            }
        });
    });
}

fn bench_set_attribute(c: &mut Criterion) {
    let mut dom = Dom::new();
    let els = build_flat(&mut dom, 1_000);
    // Réécriture d'un attribut existant — scan du Vec +
    // remplacement de la String en place.
    c.bench_function("set_attribute_overwrite_1000", |b| {
        b.iter(|| {
            for el in &els {
                dom.set_attribute(*el, "class", "x").expect("set");
            }
        });
    });
}

fn bench_collect(c: &mut Criterion) {
    let mut g = c.benchmark_group("collect");
    g.bench_function("flat_10000", |b| {
        b.iter_batched(
            || {
                let mut dom = Dom::new();
                let els = build_flat(&mut dom, 10_000);
                // Fragmente l'arène : un sous-arbre sur
                // deux détaché → sweep ~10 k nœuds balayés.
                for el in els.iter().step_by(2) {
                    dom.remove(*el).expect("detach");
                }
                dom
            },
            |mut dom| black_box(dom.collect(&[])),
            criterion::BatchSize::LargeInput,
        );
    });

    // Chaîne profonde : le marquage itératif ne doit pas
    // dériver de la linéarité.
    g.bench_function("deep_10000", |b| {
        b.iter_batched(
            || {
                let mut dom = Dom::new();
                build_deep(&mut dom, 10_000);
                dom
            },
            |mut dom| black_box(dom.collect(&[])),
            criterion::BatchSize::LargeInput,
        );
    });
    g.finish();
}

criterion_group!(
    benches,
    bench_build,
    bench_traverse,
    bench_set_attribute,
    bench_collect
);
criterion_main!(benches);
