// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Outil de triage (R-14/R-15) — dump got/expected pour des cas
//! choisis du corpus tree-construction.
//!
//! ```sh
//! TRIAGE=file.dat:idx,file2.dat:idx \
//!   cargo test -p explorer-html --test triage -- --ignored --nocapture
//! ```

use std::path::Path;

mod common;
use common::{dump, normalize_expected, parse_cases};

#[test]
#[ignore = "outil manuel : exige TRIAGE=file:idx,..."]
fn triage() {
    let spec = std::env::var("TRIAGE").expect("TRIAGE=file:idx,...");
    for sel in spec.split(',') {
        let (file, idx) = sel.split_once(':').expect("file:idx");
        let idx: usize = idx.parse().expect("idx numérique");
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/tree-construction");
        let content = std::fs::read_to_string(dir.join(file)).expect("read .dat");
        let case = &parse_cases(&content)[idx];
        let dom = explorer_html::TreeBuilder::parse(&case.input);
        let (got, want) = (dump(&dom), normalize_expected(&case.expected));
        println!(
            "\n##### {sel} — input {:?} (frag={} sOn={})",
            case.input, case.fragment, case.script_on
        );
        if got == want {
            println!("IDENTIQUE !");
        } else {
            let pos = got
                .bytes()
                .zip(want.bytes())
                .position(|(a, b)| a != b)
                .unwrap_or_else(|| got.len().min(want.len()));
            println!(
                "diff @ {pos} : got {:?} | want {:?}",
                &got[pos.saturating_sub(10)..(pos + 30).min(got.len())],
                &want[pos.saturating_sub(10)..(pos + 30).min(want.len())]
            );
        }
        println!("--- attendu ---\n{want}");
        println!("--- obtenu ---\n{got}");
    }
}
