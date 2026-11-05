// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! JS-C343 — `ResizeObserver` ré-évalué quand la `GeometryMap`
//! évolue (mur §B-97 #5 du vol 644 : `<ytd-rich-grid-renderer>`
//! observe sa taille pour calculer `elementsPerRow` et remplir
//! `shownItems` — l'`observe` initial voyait `0×0`, le layout
//! publié ensuite n'était jamais rapporté → `containerWidth`
//! figé à 0, accueil vide).

use explorer_jscore::{Dom, Engine};

fn engine() -> Engine {
    let mut e = Engine::new();
    e.install_dom(&std::rc::Rc::new(std::cell::RefCell::new(Dom::from_html(
        r#"<html><body><div id="a"></div></body></html>"#,
    ))));
    e
}

fn ev(e: &mut Engine, src: &str) -> String {
    match e.eval(src) {
        Ok(v) => e.describe(v),
        Err(err) => panic!("{src} -> {err}"),
    }
}

/// Chemin `element_path` de `#a` : `html>body>div[0]`.
fn path_a() -> Vec<(String, u32)> {
    vec![
        ("html".to_string(), 0),
        ("body".to_string(), 0),
        ("div".to_string(), 0),
    ]
}

fn geom(w: f32, h: f32) -> explorer_jscore::ElementGeom {
    explorer_jscore::ElementGeom {
        rect: explorer_jscore::GeoRect {
            x: 0.0,
            y: 0.0,
            w,
            h,
        },
        fixed: false,
        positioned: false,
        position: explorer_jscore::GeomPosition::Static,
    }
}

/// Notification initiale `0×0` → `set_geometry` publie la vraie
/// taille → le callback re-tire avec `contentRect` réel ; un
/// drain sans changement ne re-tire pas.
#[test]
fn resize_observer_fires_when_geometry_map_changes_size() {
    let mut e = engine();
    e.set_viewport(800, 600);
    let slot = explorer_jscore::GeometrySlot::default();
    e.set_geometry_ctx(0, slot.clone());
    ev(
        &mut e,
        "window.__w=[];\
         var ro=new ResizeObserver(function(es){\
           es.forEach(function(x){window.__w.push(x.contentRect.width)});\
         });\
         ro.observe(document.getElementById('a'));\
         'armed'",
    );
    e.run_microtasks().expect("pump initiale");
    // `#a` n'a pas de boîte publiée → notification initiale 0.
    assert_eq!(ev(&mut e, "window.__w.join('|')"), "0");
    // Le layout publie la géométrie de `#a` (1200×800).
    slot.lock()
        .unwrap()
        .rects
        .insert(path_a(), geom(1200.0, 800.0));
    e.run_microtasks().expect("pump ré-évaluation");
    assert_eq!(ev(&mut e, "window.__w.join('|')"), "0|1200");
    // Géométrie inchangée → silence au checkpoint suivant.
    e.run_microtasks().expect("pump stable");
    assert_eq!(ev(&mut e, "window.__w.join('|')"), "0|1200");
    // Un second changement est rapporté à nouveau.
    slot.lock()
        .unwrap()
        .rects
        .insert(path_a(), geom(600.0, 800.0));
    e.run_microtasks().expect("pump second changement");
    assert_eq!(ev(&mut e, "window.__w.join('|')"), "0|1200|600");
}

/// `unobserve`/`disconnect` purgent `last_size` — plus aucune
/// livraison sur changement postérieur.
#[test]
fn resize_observer_unobserve_and_disconnect_silence() {
    let mut e = engine();
    let slot = explorer_jscore::GeometrySlot::default();
    e.set_geometry_ctx(0, slot.clone());
    ev(
        &mut e,
        "window.__w=[];\
         var ro=new ResizeObserver(function(es){\
           es.forEach(function(x){window.__w.push(x.contentRect.width)});\
         });\
         var t=document.getElementById('a');\
         ro.observe(t); ro.unobserve(t);\
         slot=0",
    );
    slot.lock()
        .unwrap()
        .rects
        .insert(path_a(), geom(1200.0, 800.0));
    e.run_microtasks().expect("pump après unobserve");
    assert_eq!(ev(&mut e, "window.__w.length"), "0");
    // Même verdict pour `disconnect`.
    ev(
        &mut e,
        "ro.observe(document.getElementById('a'));\
         ro.disconnect(); 'd'",
    );
    e.run_microtasks().expect("pump après disconnect");
    assert_eq!(ev(&mut e, "window.__w.length"), "0");
}
