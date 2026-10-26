// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Évaluation bornée de **bundles réels capturés** — le premier
//! échec de chaque fichier nomme le gap suivant sans vol : parseur
//! (`SyntaxError` + position), runtime (`TypeError`/`ReferenceError` +
//! nom), budget (`script execution limit`). Lit `TEMP`/*.js + les
//! `_*.js` à la racine du dépôt — `cargo test --test bundle_eval --
//! --ignored --nocapture`.

use explorer_jscore::{Dom, Engine};
use std::cell::RefCell;
use std::rc::Rc;

fn try_eval(path: &std::path::Path) -> String {
    let Ok(src) = std::fs::read_to_string(path) else {
        return "SKIP unreadable".into();
    };
    let mut e = Engine::new();
    e.install_timers();
    e.install_dom(&Rc::new(RefCell::new(Dom::from_html(
        "<html><body><div id=x></div></body></html>",
    ))));
    match e.eval(&src) {
        Ok(v) => format!("OK {}", e.describe(v).chars().take(60).collect::<String>()),
        Err(t) => format!("{t}").chars().take(220).collect(),
    }
}

#[test]
#[ignore = "terrain : évalue les bundles capturés sur disque"]
fn real_bundles_first_error() {
    let mut dirs = Vec::new();
    if let Ok(t) = std::env::var("TEMP") {
        dirs.push(std::path::PathBuf::from(t));
    }
    dirs.push(std::path::PathBuf::from("."));
    if let Ok(t) = std::env::var("TEMP") {
        let bash_tmp = std::path::PathBuf::from(t.replace("AppData\\Local\\Temp", ""));
        if bash_tmp.exists() {
            dirs.push(bash_tmp);
        }
    }
    for d in &dirs {
        let Ok(rd) = std::fs::read_dir(d) else {
            continue;
        };
        for ent in rd.flatten() {
            let p = ent.path();
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            let js = std::path::Path::new(name)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("js"))
                && (p.parent() == Some(d)
                    && (d.ends_with("Temp") || name.starts_with('_') || name.starts_with('.')));
            if !js {
                continue;
            }
            let size = ent.metadata().map_or(0, |m| m.len());
            let r = try_eval(&p);
            eprintln!("[bundle] {name:40} {size:>9} o → {r}");
        }
    }
}
