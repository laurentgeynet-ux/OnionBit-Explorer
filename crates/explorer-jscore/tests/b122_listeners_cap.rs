// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! §B-122 — borne anti-tempête : les listeners d'un type sur un
//! nœud sont plafonnés (vol 751 explore.org — web-vitals `rB`
//! posait `addEventListener("load", ()=>rB(e), capture)` tant
//! que `readyState!=="complete"` ; chaque `load` de sous-
//! ressource traversait la capture racine et faisait doublier
//! l'armée → ~370k listeners → autant de `setTimeout` postés
//! par dispatch). Le verrou rejoue le pattern : la file refuse
//! au-delà du plafond, les listeners déjà admis tirent encore.

use explorer_jscore::{Dom, Engine};
use std::cell::RefCell;
use std::rc::Rc;

const PAGE: &str = r#"<html><body><div id="x"></div></body></html>"#;

fn engine() -> (Engine, Rc<RefCell<Dom>>) {
    let mut e = Engine::new();
    let dom = Rc::new(RefCell::new(Dom::from_html(PAGE)));
    e.install_dom(&dom);
    (e, dom)
}

fn ev(e: &mut Engine, src: &str) -> String {
    match e.eval(src) {
        Ok(v) => e.describe(v),
        Err(err) => panic!("{src} -> {err}"),
    }
}

/// La greffe au-delà du plafond est refusée : 300 demandes →
/// seuls 256 `load` capture sur `window` (≙ root) sont admis —
/// le compteur de tirs au dispatch le prouve.
#[test]
fn listeners_cap_bloque_l_armee_exponentielle() {
    let (mut e, _dom) = engine();
    // Pattern verbatim §B-122 : un helper qui re-pose un
    // listener à chaque tir tant que la condition tient —
    // ici 300 greffes directes (la croissance est linéaire
    // pour le verrou : le compte admis est la mesure).
    let hits = ev(
        &mut e,
        "var fired = 0;\
         for (var i = 0; i < 300; i++) {\
           addEventListener('load', function(){ fired++ }, true);\
         }\
         dispatchEvent(new Event('load'));\
         String(fired)",
    );
    assert_eq!(hits, "256");
}

/// La borne est **par type** : `load` plafonné n'empêche pas
/// `click` de se greffer normalement sur la même cible.
#[test]
fn listeners_cap_est_par_type() {
    let (mut e, _dom) = engine();
    let out = ev(
        &mut e,
        "for (var i = 0; i < 300; i++) addEventListener('load', function(){}, true);\
         var n = 0;\
         addEventListener('click', function(){ n++ }, true);\
         dispatchEvent(new Event('click'));\
         String(n)",
    );
    assert_eq!(out, "1");
}

/// Deux phases distinctes : `load` capture ET `load` bulle
/// partagent le même plafond par type (la file est unique —
/// capture/bulle est un attribut du listener).
#[test]
fn listeners_cap_partage_capture_et_bulle() {
    let (mut e, _dom) = engine();
    let hits = ev(
        &mut e,
        "var f = 0;\
         for (var i = 0; i < 130; i++) addEventListener('load', function(){ f++ }, true);\
         for (var j = 0; j < 130; j++) addEventListener('load', function(){ f++ }, false);\
         dispatchEvent(new Event('load'));\
         String(f)",
    );
    // 130 capture + 126 bulle = 256 admis — le dispatch cible
    // `body` et traverse root en capture puis en bulle.
    assert_eq!(hits, "256");
}
