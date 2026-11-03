// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Verrous + bancs J17 (§B-perf) : attribution par phase de
//! l'allocation native — `last_eval_alloc_split` doit clôturer
//! parse et exec aux frontières mesurées par `last_eval_split`.

use explorer_jscore::Engine;

/// Allocateur instrumenté pour le banc : sans lui `stats_alloc`
/// lit des zéros (le moteur snapshotte le même compteur global
/// que le daemon, installé ici par le binaire de test).
#[global_allocator]
static ALLOC: &stats_alloc::StatsAlloc<std::alloc::System> = &stats_alloc::INSTRUMENTED_SYSTEM;

/// Verrou : une éval quelconque produit un split alloc
/// `(parse, exec)` cohérent — l'exec d'un script qui alloue
/// (concaténation en boucle) doit être strictement positive.
#[test]
fn alloc_split_surfaces() {
    let mut e = Engine::new();
    let r = e.eval("var s=''; for(var i=0;i<2000;i++) s+='abcdef'; s.length");
    match r {
        Ok(v) => assert_eq!(e.describe(v), "12000"),
        Err(err) => panic!("eval: {err}"),
    }
    let ((pb, pn), (eb, en)) = e.last_eval_alloc_split();
    assert!(pb > 0 && pn > 0, "parse alloue (AST) : b={pb} n={pn}");
    assert!(eb > 0 && en > 0, "exec alloue (concat) : b={eb} n={en}");
}

/// Rejoue un script réel figé (dump `EXPLORER_JS_DUMP_DIR`) avec
/// le split alloc par phase + histogramme du tas :
/// `J17_BENCH_JS=<chemin> cargo test -p explorer-jscore --release
/// --test j17_alloc -- --ignored --nocapture`.
#[test]
#[ignore = "banc J17 : fixture réelle via J17_BENCH_JS"]
fn j17_bench_real_script() {
    let Ok(path) = std::env::var("J17_BENCH_JS") else {
        eprintln!("[j17-real] SKIP — J17_BENCH_JS non posé");
        return;
    };
    let src = std::fs::read_to_string(&path).expect("fixture lisible");
    let mut e = Engine::new();
    e.install_dom(&std::rc::Rc::new(std::cell::RefCell::new(
        explorer_jscore::Dom::from_html("<html><body><div id=x></div></body></html>"),
    )));
    e.install_fetch();
    e.install_timers();
    let snap = || {
        use std::sync::atomic::Ordering::Relaxed as R;
        [
            ("calls", explorer_jscore::CALL_N.load(R)),
            ("str_cell", explorer_jscore::J17_STR_CELL.load(R)),
            ("obj_cell", explorer_jscore::J17_OBJ_CELL.load(R)),
            ("prop_new", explorer_jscore::J17_PROP_NEW.load(R)),
            ("bind", explorer_jscore::J17_BIND.load(R)),
            ("env", explorer_jscore::J17_ENV.load(R)),
            ("args", explorer_jscore::J17_ARGS.load(R)),
            ("member_key", explorer_jscore::J17_MEMBER_KEY.load(R)),
            ("array", explorer_jscore::J17_ARRAY.load(R)),
            ("iter", explorer_jscore::J17_ITER.load(R)),
            ("regex", explorer_jscore::J17_REGEX.load(R)),
            ("tok_ident", explorer_jscore::J17_TOK_IDENT.load(R)),
        ]
    };
    let before = snap();
    let t0 = std::time::Instant::now();
    let r = e.eval_named(&src, &path);
    let wall = t0.elapsed();
    let after = snap();
    let (parse_us, exec_us) = e.last_eval_split();
    let ((pb, pn), (eb, en)) = e.last_eval_alloc_split();
    let hs = e.heap_stats();
    for ((name, a), (_, b)) in before.iter().zip(after.iter()) {
        eprintln!("[j17-site] {name}={}", b - a);
    }
    eprintln!(
        "[j17-real] bytes={} wall={wall:?} parse_us={parse_us} exec_us={exec_us} ok={}",
        src.len(),
        r.is_ok()
    );
    if let Err(err) = &r {
        let msg: String = format!("{err}").chars().take(300).collect();
        eprintln!("[j17-real] err={msg}");
    }
    eprintln!(
        "[j17-real] alloc parse b={pb} n={pn} | exec b={eb} n={en} | live={} peak_b={} gc_runs={} slab_o={}",
        hs.live_cells,
        hs.peak_bytes,
        hs.gc_runs,
        e.slab_overhead_bytes()
    );
    for (kind, cells, bytes) in e.heap_kind_histogram().iter().take(6) {
        eprintln!("[j17-real] kind {kind}: cells={cells} bytes={bytes}");
    }
}
