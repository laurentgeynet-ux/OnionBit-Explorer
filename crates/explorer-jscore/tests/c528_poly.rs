// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Verrous JS-C528 — fallback polymorphe des sites IC désarmés :
//! un site à shapes de tête alternées bornées ressert via ses
//! membres `Poly` (vérif = même marche légère des shapes) au lieu
//! de bailer à vie, un vrai site mégamorphe meurt (`Dead`) et
//! l'invalidation par mutation de shape tient à l'intérieur du
//! poly (membre périmé → miss → relearn, jamais de valeur stale).
//! Motivation mesurée (vol N5.31) : 2 395 sites désarmés ≈ 342 k
//! `ic_bail` ≈ 27 % des lookups payaient la marche complète.

use explorer_jscore::Engine;

/// Sérialise les tests sensibles aux compteurs J17 partagés —
/// les autres tests du binaire restent parallèles.
static IC_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn eval(src: &str) -> String {
    let mut e = Engine::new();
    match e.eval(src) {
        Ok(v) => e.describe(v),
        Err(err) => panic!("eval: {err}\n  src: {src}"),
    }
}

fn snap_counter(snap: &str, name: &str) -> u64 {
    snap.split(' ')
        .find_map(|p| p.strip_prefix(name).and_then(|v| v.parse::<u64>().ok()))
        .unwrap_or(0)
}

/// Site à deux shapes alternées : 8 miss consécutifs → `Poly`
/// amorcé du dernier verdict, le second relearn peuple l'autre
/// shape — les accès suivants sont servis (`ic_poly` croît) au
/// lieu de bailer à vie. Valeurs exactes par receveur.
#[test]
fn c528_poly_serves_post_disarm() {
    let _g = IC_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    explorer_jscore::j17_enable();
    let before = snap_counter(&explorer_jscore::j17_snapshot(), "ic_poly=");
    assert_eq!(
        eval(
            "var a={x:1},b={y:9,x:2};var s=0;\
             for(var i=0;i<2000;i++){var o=i%2?a:b;s+=o.x}s"
        ),
        "3000"
    );
    let after = snap_counter(&explorer_jscore::j17_snapshot(), "ic_poly=");
    assert!(
        after > before,
        "le site désarmé doit resservir via poly : {before} → {after}"
    );
}

/// Écriture in-place à position constante (shape inchangée) sous
/// hit `Poly` : `props[pos]` est relu — la nouvelle valeur est
/// rendue, jamais l'ancienne.
#[test]
fn c528_poly_inplace_write_visible() {
    assert_eq!(
        eval(
            "var a={x:1},b={y:9,x:2};var s=0;\
             for(var i=0;i<200;i++){var o=i%2?a:b;s+=o.x}\
             a.x=5;var t=0;for(var i=0;i<200;i++){var o=i%2?a:b;t+=o.x}\
             s+'/'+t"
        ),
        "300/700"
    );
}

/// Membre `Poly` périmé par croissance de shape (`a.z` ajouté) :
/// miss → relearn de la nouvelle shape — la lecture rend la
/// valeur exacte, pas un pos stale.
#[test]
fn c528_poly_shape_growth_relearns() {
    assert_eq!(
        eval(
            "var a={x:1},b={y:9,x:2};var s=0;\
             for(var i=0;i<200;i++){var o=i%2?a:b;s+=o.x}\
             a.z=0;var t=0;for(var i=0;i<200;i++){var o=i%2?a:b;t+=o.x}\
             s+'/'+t"
        ),
        "300/300"
    );
}

/// Vrai mégamorphe (16 layouts distincts > `IC_POLY_CAP`=8) : le
/// poly thrash → `Dead` → bail à vie — les valeurs restent
/// exactes et `ic_bail` finit par compter les consultations mortes.
#[test]
fn c528_true_megamorphic_dies() {
    let _g = IC_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    explorer_jscore::j17_enable();
    let before = snap_counter(&explorer_jscore::j17_snapshot(), "ic_bail=");
    assert_eq!(
        eval(
            "var os=[];for(var i=0;i<16;i++){var o={x:i};o['p'+i]=1;os.push(o)}\
             var s=0;for(var i=0;i<2000;i++){s+=os[i%16].x}s"
        ),
        "15000"
    );
    let after = snap_counter(&explorer_jscore::j17_snapshot(), "ic_bail=");
    assert!(
        after > before,
        "le site mégamorphe doit finir mort : {before} → {after}"
    );
}
