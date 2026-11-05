// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! JS-C349 / mur §B-100 (vols 651/652) — `updateend` filé en
//! **tâche** (spec « queue a task »), même couture que
//! `sourceopen`/`sourceended` (JS-C324). Avant : le trio
//! `updatestart`/`update`/`updateend` était tiré synchrone
//! dans `appendBuffer` — la boucle d'append d'un player
//! (`updateend` → `appendBuffer` suivant) ré-entrait un cycle
//! complet par segment (~9 frames JS/cycle ×65 = ~600 frames,
//! signature `[expr_bisect]` mesurée en vol) → `RangeError`
//! → SABR jamais armé → `mseOffsets=[0,0]`, embed noir.
//!
//! `updatestart`/`update` restent synchrones : la fenêtre
//! `updating` qu'ils encadrent porte l'`InvalidStateError`
//! re-entrant spec (JS-C325).

use explorer_jscore::{Dom, Engine};
use std::cell::RefCell;
use std::rc::Rc;

const PAGE: &str = r#"<html><body><video id="v"></video></body></html>"#;

fn engine() -> Engine {
    let mut e = Engine::new();
    e.install_dom(&Rc::new(RefCell::new(Dom::from_html(PAGE))));
    e
}

fn ev(e: &mut Engine, src: &str) -> String {
    match e.eval(src) {
        Ok(v) => e.describe(v),
        Err(err) => panic!("{src} -> {err}"),
    }
}

/// Boucle d'append réelle : un listener `updateend` qui
/// `appendBuffer` le segment suivant — le pattern scheduler
/// des players (hls.js/`SABR` `YouTube`). Tir synchrone : 400
/// cycles empilaient ~400 séquences → `RangeError` bien avant.
/// Filé en tâche : la livraison dépoppe à plat à la frontière
/// de job — les 400 appends se déroulent sans récursion.
#[test]
fn updateend_append_loop_runs_flat() {
    let mut e = engine();
    ev(
        &mut e,
        "var ms=new MediaSource();\
         var sb=ms.addSourceBuffer('video/mp4');\
         var n=0;\
         sb.addEventListener('updateend',function(){\
           if(n<400){n++;sb.appendBuffer(new ArrayBuffer(4));}});\
         sb.appendBuffer(new ArrayBuffer(4));'armed'",
    );
    assert_eq!(ev(&mut e, "n"), "400");
    assert_eq!(ev(&mut e, "sb.updating"), "false");
}

/// Le différé n'est pas perdu si le listener n'est posé
/// qu'après l'append (pattern canonique player) : `updateend`
/// reste servi à la frontière de job.
#[test]
fn updateend_listener_registered_after_append_still_fires() {
    let mut e = engine();
    ev(
        &mut e,
        "var ms=new MediaSource();\
         var sb=ms.addSourceBuffer('video/mp4');\
         var seen='';\
         sb.appendBuffer(new ArrayBuffer(4));\
         sb.addEventListener('updateend',function(){seen='hit';u=sb.updating});'armed'",
    );
    assert_eq!(ev(&mut e, "seen+'|'+u"), "hit|false");
}

/// `remove` et `abort` file `updateend` pareillement — la
/// boucle d'éviction/switch des players reste à plat aussi.
#[test]
fn remove_abort_updateend_also_tasked() {
    let mut e = engine();
    ev(
        &mut e,
        "var ms=new MediaSource();\
         var sb=ms.addSourceBuffer('video/mp4');\
         var n=0;var ab='';\
         sb.addEventListener('updateend',function(){\
           if(n<200){n++;sb.remove(0,0.001);}\
           else if(ab===''){ab='r';sb.abort();}\
           else if(ab==='r'){ab='done';}});\
         sb.appendBuffer(new ArrayBuffer(4));'armed'",
    );
    assert_eq!(ev(&mut e, "n+'|'+ab+'|'+sb.updating"), "200|done|false");
}
