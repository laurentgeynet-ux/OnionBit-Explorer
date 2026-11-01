// This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Sonde §B-59 — rejoue le script tueur capturé par
//! `EXPLORER_JS_DUMP_DIR` (`_inline25.js` sudouest, 88 Ko
//! obfusqué) sous borne mémoire : si le bras est une croissance
//! comptabilisée par le heap, `set_mem_limit` le tue en erreur
//! propre ; sinon c'est du churn natif hors compteur.
//! `cargo test --test b59_probe -- --ignored --nocapture`.

use explorer_jscore::{Dom, Engine};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

/// Verrou §B-59 — la boucle auto-défense obfuscateur
/// (`arr.push` pendant une itération bornée par `arr.length`
/// relue) retient des déchets O(n²) à vivant minuscule :
/// le ratchet `gc_cap` ×4 sans plafond laissait filer à
/// l'abort allocateur (8-12 Go mesurés sudouest). Sous cap
/// dur `lim × 4`, l'éval meurt en `RangeError` nommé et le
/// moteur survit. `mem_limit` bas → cap atteint sans Go réels.
#[test]
fn dead_cell_hard_cap_kills_eval() {
    let mut e = Engine::new();
    e.set_mem_limit(8 * 1024 * 1024); // cap dur = 32 Mio
                                      // Forme sudouest : pousser dans le tableau parcouru +
                                      // `+=` concaténant à chaque tour — infinie ET quadratique.
    let bomb = "var a=['x'],s='';for(var i=0,L=a.length;i<L;i++){\
                s+=a[i]+'x';a.push('y');L=a.length;}";
    let r = e.eval(bomb);
    match r {
        Err(err) => {
            let msg = format!("{err}");
            assert!(
                msg.contains("heap memory limit exceeded"),
                "RangeError cap dur attendu, obtenu : {msg}"
            );
        }
        Ok(_) => panic!("la bombe aurait dû être tuée par le cap dur"),
    }
    // Moteur vivant post-kill — la session dégrade, ne meurt pas.
    assert_eq!(e.eval("1+1").ok().map(|v| e.describe(v)), Some("2".into()));
}

#[test]
#[ignore = "terrain : rejoue le bundle §B-59 capturé"]
fn inline25_under_mem_limit() {
    let src = std::fs::read_to_string("_inline25.js").expect("_inline25.js à la racine");
    eprintln!("[b59] {} o — eval sous mem_limit 512 Mio", src.len());
    let mut e = Engine::new();
    e.set_mem_limit(512 * 1024 * 1024);
    e.install_timers();
    e.install_dom(&Rc::new(RefCell::new(Dom::from_html(
        "<html><body><div id=x></div></body></html>",
    ))));
    let t = Instant::now();
    match e.eval_named(&src, "inline:25") {
        Ok(v) => {
            let d = e.describe(v);
            eprintln!(
                "[b59] OK en {:?} — {}",
                t.elapsed(),
                d.chars().take(200).collect::<String>()
            );
        }
        Err(err) => eprintln!("[b59] ERR en {:?} — {err}", t.elapsed()),
    }
    let (p, x) = e.last_eval_split();
    eprintln!("[b59] split parse={p}us exec={x}us");
}
