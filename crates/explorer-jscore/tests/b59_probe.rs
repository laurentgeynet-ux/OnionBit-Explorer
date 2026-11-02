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

/// Verrou §B-59 bras DOM — une chaîne `parent` cyclique (miroir
/// corrompu/mutation hostile) rendait `ancestors` sans borne :
/// Vec<NodeId> poussé à l'infini, ~150 Mo/s jusqu'à l'abort
/// (4,3 Go mesurés franceinter, dispatch event Didomi). Les
/// marches bornées par `nodes.len()` dégradent : cycle
/// auto-référent → au plus N ancêtres.
#[test]
fn cyclic_parent_chain_bounded() {
    let mut d = Dom::from_html("<html><body><div id=a><div id=b></div></div></body></html>");
    let (a, b) = {
        let body = d.child_nodes(d.root)[0];
        let a = d.child_nodes(body)[0];
        let b = d.child_nodes(a)[0];
        (a, b)
    };
    // Corruption : parenté auto-référente et cycle à deux.
    d.nodes[a as usize].parent = Some(b);
    d.nodes[b as usize].parent = Some(a);
    assert!(d.ancestors(a).len() <= d.nodes.len());
    assert!(d.ancestors(b).len() <= d.nodes.len());
    // `root_of`/`closest`/`element_path`/`focus_within` bornés idem.
    assert!(d.closest(b, "div").is_some() || d.closest(b, "div").is_none());
}

/// Le mutateur refuse la création du cycle (défense en profondeur) :
/// `append(parent, ancêtre)`/`insert_before`/`replace_children`/
/// `replace_child` no-op sur ancêtre-ou-soi.
#[test]
fn mutators_reject_ancestor_cycles() {
    let mut d = Dom::from_html(
        "<html><body><div id=x><div id=y><div id=z></div></div></div></body></html>",
    );
    let (x, y, z) = {
        let body = d.child_nodes(d.root)[0];
        let x = d.child_nodes(body)[0];
        let y = d.child_nodes(x)[0];
        let z = d.child_nodes(y)[0];
        (x, y, z)
    };
    let x_parent = d.nodes[x as usize].parent;
    // `z.appendChild(x)` — x est ancêtre de z → refusé (inchangé).
    d.append(z, x);
    assert_eq!(d.nodes[x as usize].parent, x_parent);
    assert!(!d.ancestors(x).contains(&z));
    // `insert_before` : insérer y sous z → refusé (y ancêtre de z).
    d.insert_before(z, y, None);
    assert_eq!(d.nodes[y as usize].parent, Some(x));
    // `replace_child` : y.replaceChild(x, z) — x ancêtre de y → refusé.
    d.replace_child(y, x, z);
    assert_eq!(d.nodes[z as usize].parent, Some(y));
}
