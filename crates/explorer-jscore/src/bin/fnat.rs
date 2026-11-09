// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! `fnat <fichier> <l:c>` — imprime la chaîne des fonctions
//! dont le span contient `l:c` (JS-C413, forensique murs
//! terrain : lire le code au site nommé par les sondes,
//! ex. `kevlar_base:1484:39`). Les corps différés sont
//! matérialisés à la demande — les fonctions de bundle sont
//! trouvées comme les scripts directs.
//!
//! ```text
//! fnat kevlar_base.js 1484:39
//! [0] <anon> (fn) kevlar_base.js:1:1
//! [1] W (async fn) kevlar_base.js:5461:34
//! --- plus profond ---
//! function(e,t){var n=…
//! ```

use std::process::ExitCode;

/// Borne d'impression de l'extrait du plus profond
/// (les fonctions de bundle peuvent faire des centaines de
/// Ko — l'extrait sert à nommer, `fnat` n'est pas un dump).
const EXCERPT_MAX: usize = 4000;

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let (Some(file), Some(lc)) = (args.next(), args.next()) else {
        eprintln!("usage: fnat <fichier.js> <ligne:col>");
        return ExitCode::from(2);
    };
    let Some((line, col)) = lc
        .split_once(':')
        .and_then(|(l, c)| l.parse::<u32>().ok().zip(c.parse::<u32>().ok()))
    else {
        eprintln!("position invalide `{lc}` — attendu <ligne:col>");
        return ExitCode::from(2);
    };
    let Ok(src) = std::fs::read_to_string(&file) else {
        eprintln!("lecture impossible : {file}");
        return ExitCode::FAILURE;
    };
    let hits = explorer_jscore::fn_chain_at(&src, line, col);
    if hits.is_empty() {
        println!("aucune fonction ne contient {lc} (top-level/donnée ?)");
        return ExitCode::SUCCESS;
    }
    for (i, h) in hits.iter().enumerate() {
        let name = h.name.as_deref().unwrap_or("<anon>");
        let flags = [
            if h.is_async { "async " } else { "" },
            if h.is_gen { "*" } else { "" },
            if h.is_arrow { "=>" } else { "fn" },
        ]
        .concat();
        let src = h.src.as_deref().unwrap_or(&file);
        println!("[{i}] {name} ({flags}) {src}:{}:{}", h.pos.0, h.pos.1);
    }
    let last = hits.last().expect("hits non vide");
    println!(
        "--- plus profond ({} chars) ---",
        last.excerpt.chars().count()
    );
    let excerpt: String = last.excerpt.chars().take(EXCERPT_MAX).collect();
    println!("{excerpt}");
    if last.excerpt.chars().count() > EXCERPT_MAX {
        println!("… [tronqué à {EXCERPT_MAX} chars]");
    }
    ExitCode::SUCCESS
}
