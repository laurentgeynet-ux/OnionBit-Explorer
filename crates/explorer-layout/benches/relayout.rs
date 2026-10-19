// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Bench R-12 : `layout_tree` complet vs `relayout_subtree`
//! sur une mutation localisée — mesure le tiers `DirtyLayout`
//! d'ADR-0012.
//!
//! `cargo bench -p explorer-layout` — résultats sous
//! `target/criterion/`.

use std::collections::HashMap;

use criterion::{criterion_group, criterion_main, Criterion};
use explorer_css::{Origin, StyleEngine, StyleEnv, StyleMap};
use explorer_dom::{Dom, NodeId};
use explorer_html::TreeBuilder;
use explorer_layout::font::{FontRequest, FontSource};
use explorer_layout::{build_box_tree_sized, layout_tree, relayout_subtree, BlockCtx, LayoutBox};

struct Fixture(Vec<u8>);
impl FontSource for Fixture {
    fn font_data(&self, _req: &FontRequest) -> Option<Vec<u8>> {
        Some(self.0.clone())
    }
}

fn ctx() -> BlockCtx<'static> {
    let data = explorer_test_support::fixture_ttf();
    let src: &'static Fixture = Box::leak(Box::new(Fixture(data)));
    BlockCtx {
        fonts: Some(src),
        ..BlockCtx::default()
    }
}

/// Page ~200 blocs : le sous-arbre sale est une feuille au
/// milieu du document.
fn bench_page() -> String {
    let mut s = String::from("<html><body>");
    for i in 0..200 {
        use std::fmt::Write;
        let _ = write!(
            s,
            "<div id=d{i}><p>paragraphe {i} avec un peu de texte pour le shaping</p></div>"
        );
    }
    s.push_str("</body></html>");
    s
}

fn by_id(dom: &Dom, id: &str) -> NodeId {
    fn walk(dom: &Dom, n: NodeId, id: &str) -> Option<NodeId> {
        if dom.element(n).is_ok() && dom.attribute(n, "id").ok().flatten().as_deref() == Some(id) {
            return Some(n);
        }
        let mut c = dom.first_child(n).ok().flatten();
        while let Some(node) = c {
            if let Some(f) = walk(dom, node, id) {
                return Some(f);
            }
            c = dom.next_sibling(node).ok().flatten();
        }
        None
    }
    walk(dom, dom.document(), id).expect("id")
}

fn bench_relayout(c: &mut Criterion) {
    let html = bench_page();
    let dom = TreeBuilder::parse(&html);
    let mut eng = StyleEngine::new();
    eng.add_css("div { width: 300px; margin: 4px }", Origin::Author);
    let styles: StyleMap = eng.compute_tree(&dom, dom.document(), &StyleEnv::default());
    let root = dom.document_element().expect("el");
    let target = by_id(&dom, "d150");
    let sizes: HashMap<NodeId, (f32, f32)> = HashMap::new();

    let mut g = c.benchmark_group("relayout");
    g.bench_function("layout_tree_complet", |b| {
        b.iter(|| {
            let mut bx = build_box_tree_sized(&dom, root, &styles, &sizes).expect("arbre");
            layout_tree(&mut bx, 800.0, &ctx());
            bx.rect.height
        });
    });
    g.bench_function("relayout_subtree_sale", |b| {
        b.iter_batched(
            || {
                let mut bx = build_box_tree_sized(&dom, root, &styles, &sizes).expect("arbre");
                layout_tree(&mut bx, 800.0, &ctx());
                bx
            },
            |mut bx: LayoutBox| relayout_subtree(&mut bx, &dom, &styles, &sizes, target, &ctx()),
            criterion::BatchSize::LargeInput,
        );
    });
    g.finish();
}

criterion_group!(benches, bench_relayout);
criterion_main!(benches);
