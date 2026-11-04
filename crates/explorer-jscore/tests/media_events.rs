// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Étape 182 — boucle `state → sémantique DOM` : `deliver_media_state`
//! traduit les rapports hôte en échelle `readyState` + events média
//! (`loadedmetadata` → `loadeddata` → `canplay` → `canplaythrough`,
//! `waiting`/`stalled`, `progress`, `timeupdate`, `durationchange`,
//! `seeking`/`seeked`, `play`/`playing`/`pause`/`ended`/`error`,
//! `loadstart` au claim). Sous MSE, `buffered` lit l'union
//! `SourceBuffer` (pas le `buffered_ms` du rapport) — la même source
//! pilote l'échelle, et la croissance du buffer vit dans
//! `appendBuffer` : `progress` y est filé en tâche (discipline
//! JS-C324) vers l'élément attaché.

use explorer_jscore::{Dom, Engine, MediaReport, MediaState};
use std::cell::RefCell;
use std::rc::Rc;

const PAGE: &str = r#"<html><body><video id="v"></video></body></html>"#;

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

fn rep(pos: u64, dur: u64, buf: u64, state: MediaState) -> MediaReport {
    MediaReport {
        position_ms: pos,
        duration_ms: Some(dur),
        buffered_ms: buf,
        state,
        error: None,
    }
}

/// Claim + tous les events média logués dans `L` (sauf
/// `loadstart`, tiré par le claim lui-même — armé avant).
fn claim_and_arm(e: &mut Engine, dom: &Rc<RefCell<Dom>>) -> u32 {
    let node = dom.borrow().by_id("v").expect("v node");
    ev(
        e,
        "var L=[];var v=document.getElementById('v');\
         ['loadstart','loadedmetadata','loadeddata','canplay',\
          'canplaythrough','progress','play','playing','pause',\
          'waiting','stalled','ended','seeking','seeked',\
          'timeupdate','durationchange','error'].forEach(\
         function(t){v.addEventListener(t,function(){L.push(t)})});'armed'",
    );
    e.set_media_claim(node, 7, Some(vec!["avc1".into()]));
    node
}

#[test]
fn media_state_ladder_fires_readiness_events_in_order() {
    let (mut e, dom) = engine();
    let node = claim_and_arm(&mut e, &dom);
    // Claim = début de fetch hôte → `loadstart` (approximation
    // documentée — le vrai algo de sélection vit côté hôte).
    assert_eq!(ev(&mut e, "L.join(',')"), "loadstart");
    // Premier rapport : métadonnées puis données puis marge —
    // canplay (buffered > position), pas through (1 s ≪ durée).
    e.deliver_media_state(node, &rep(0, 60_000, 1_000, MediaState::Playing))
        .expect("report 1");
    assert_eq!(
        ev(&mut e, "L.join(',')"),
        "loadstart,durationchange,loadedmetadata,loadeddata,canplay,progress,play,playing,timeupdate"
    );
    assert_eq!(ev(&mut e, "v.readyState"), "3");
    // Source non entièrement tamponnée → fetch en cours (2).
    assert_eq!(ev(&mut e, "v.networkState"), "2");
    assert_eq!(ev(&mut e, "v.paused"), "false");
    // Le buffer couvre désormais la durée → canplaythrough ;
    // buffered a crû → progress. `timeupdate` est lié au
    // mouvement de la tête (pos 0→0 : pas de tir).
    e.deliver_media_state(node, &rep(0, 60_000, 60_000, MediaState::Playing))
        .expect("report 2");
    assert_eq!(
        ev(&mut e, "L.join(',')"),
        "loadstart,durationchange,loadedmetadata,loadeddata,canplay,progress,play,playing,\
         timeupdate,canplaythrough,progress"
    );
    assert_eq!(ev(&mut e, "v.readyState"), "4");
    // Rapport identique : strictement rien de re-tiré (les
    // events sont edge-triggered, jamais pollués).
    e.deliver_media_state(node, &rep(0, 60_000, 60_000, MediaState::Playing))
        .expect("report 3");
    assert_eq!(
        ev(&mut e, "L.join(',')"),
        "loadstart,durationchange,loadedmetadata,loadeddata,canplay,progress,play,playing,\
         timeupdate,canplaythrough,progress"
    );
}

#[test]
fn media_state_buffering_waiting_stalled_then_recovers() {
    let (mut e, dom) = engine();
    let node = claim_and_arm(&mut e, &dom);
    e.deliver_media_state(node, &rep(0, 60_000, 60_000, MediaState::Playing))
        .expect("playing");
    assert_eq!(ev(&mut e, "v.readyState"), "4");
    // Affamé : rétrogradé à HAVE_CURRENT_DATA, waiting+stalled
    // (underrun décodeur ≈ réseau lent — approximation
    // documentée, les spinners players lisent `stalled`).
    e.deliver_media_state(node, &rep(5_000, 60_000, 55_000, MediaState::Buffering))
        .expect("buffering");
    assert_eq!(
        ev(&mut e, "L.slice(-3).join(',')"),
        "stalled,waiting,timeupdate"
    );
    assert_eq!(ev(&mut e, "v.readyState"), "2");
    // `paused` reste faux pendant l'underrun (la lecture n'est
    // pas interrompue, elle attend) ; networkState = LOADING.
    assert_eq!(ev(&mut e, "v.paused"), "false");
    assert_eq!(ev(&mut e, "v.networkState"), "2");
    // Reprise : l'échelle re-monte → canplay re-tiré (spec :
    // re-climb post-rebuffer), `playing` sans `play`
    // (l'élément n'a jamais été mis en pause).
    e.deliver_media_state(node, &rep(6_000, 60_000, 60_000, MediaState::Playing))
        .expect("resumed");
    assert_eq!(
        ev(&mut e, "L.slice(-5).join(',')"),
        "canplay,canplaythrough,progress,playing,timeupdate"
    );
    assert_eq!(ev(&mut e, "v.readyState"), "4");
}

#[test]
fn media_state_pause_ended_and_error_paths() {
    let (mut e, dom) = engine();
    let node = claim_and_arm(&mut e, &dom);
    e.deliver_media_state(node, &rep(0, 60_000, 60_000, MediaState::Playing))
        .expect("playing");
    e.deliver_media_state(node, &rep(1_000, 60_000, 60_000, MediaState::Paused))
        .expect("paused");
    assert_eq!(ev(&mut e, "L.slice(-2).join(',')"), "pause,timeupdate");
    assert_eq!(ev(&mut e, "v.paused"), "true");
    // `ended` impose position=duration (spec) et tire ended.
    e.deliver_media_state(node, &rep(30_000, 60_000, 60_000, MediaState::Ended))
        .expect("ended");
    assert_eq!(ev(&mut e, "v.ended"), "true");
    assert_eq!(ev(&mut e, "v.currentTime"), "60");
    // Release → stubs honnêtes (ADR-0023) : plus rien n'arrive.
    e.set_media_claim(node, 7, None);
    assert_eq!(ev(&mut e, "v.readyState"), "0");
    let before = ev(&mut e, "L.length");
    e.deliver_media_state(node, &rep(0, 60_000, 60_000, MediaState::Playing))
        .expect("dropped report");
    assert_eq!(ev(&mut e, "L.length"), before);
}

#[test]
fn media_state_error_classified_and_event() {
    let (mut e, dom) = engine();
    let node = claim_and_arm(&mut e, &dom);
    let err = MediaReport {
        error: Some("decode: bad moov atom".into()),
        ..rep(0, 60_000, 1_000, MediaState::Error)
    };
    e.deliver_media_state(node, &err).expect("error report");
    // `MediaError.code` classifié sur la chaîne (étape 182) —
    // "decode" → MEDIA_ERR_DECODE (3), plus le 4 en dur.
    assert_eq!(
        ev(&mut e, "v.error.code+'|'+v.error.message"),
        "3|decode: bad moov atom"
    );
    assert_eq!(ev(&mut e, "v.paused"), "true");
    assert_eq!(ev(&mut e, "MediaError.MEDIA_ERR_DECODE"), "3");
    assert_eq!(ev(&mut e, "v.error instanceof MediaError"), "true");
}

#[test]
fn media_state_durationchange_only_on_change() {
    let (mut e, dom) = engine();
    let node = claim_and_arm(&mut e, &dom);
    e.deliver_media_state(node, &rep(0, 60_000, 60_000, MediaState::Playing))
        .expect("r1");
    e.deliver_media_state(node, &rep(1_000, 60_000, 60_000, MediaState::Playing))
        .expect("r2 same dur");
    let n = ev(
        &mut e,
        "L.filter(function(t){return t=='durationchange'}).length",
    );
    assert_eq!(n, "1");
    // Live → durée connue à la fin du manifest : durationchange.
    let mut live = rep(1_000, 0, 60_000, MediaState::Playing);
    live.duration_ms = None;
    e.deliver_media_state(node, &live).expect("live");
    e.deliver_media_state(node, &rep(2_000, 62_000, 62_000, MediaState::Playing))
        .expect("dur known");
    assert_eq!(ev(&mut e, "v.duration"), "62");
}

#[test]
fn media_state_seek_to_same_position_still_fires_seeked() {
    let (mut e, dom) = engine();
    let node = claim_and_arm(&mut e, &dom);
    e.deliver_media_state(node, &rep(5_000, 60_000, 60_000, MediaState::Playing))
        .expect("playing");
    // Seek vers la position courante : `position_ms` ne bouge
    // jamais — le vieux code laissait `seeking` armé à vie ;
    // désormais `seek_target_ms` ferme le cycle (étape 182).
    ev(&mut e, "v.currentTime=5;'seeked?'");
    assert_eq!(ev(&mut e, "v.seeking"), "true");
    assert_eq!(ev(&mut e, "L.slice(-1)[0]"), "seeking");
    e.deliver_media_state(node, &rep(5_000, 60_000, 60_000, MediaState::Playing))
        .expect("confirm");
    assert_eq!(ev(&mut e, "v.seeking"), "false");
    // `seeked` clôt la rafale — tête inchangée → pas de
    // `timeupdate` parasite.
    assert_eq!(ev(&mut e, "L.slice(-2).join(',')"), "seeking,seeked");
}

#[test]
fn media_state_play_promise_resolves_on_playing() {
    let (mut e, dom) = engine();
    let node = claim_and_arm(&mut e, &dom);
    ev(
        &mut e,
        "var R='pending';v.play().then(\
         function(){R='resolved'},function(x){R='rejected:'+x});'armed'",
    );
    e.deliver_media_state(node, &rep(0, 60_000, 1_000, MediaState::Playing))
        .expect("playing");
    assert_eq!(ev(&mut e, "R"), "resolved");
    // `play()` sur un élément déjà playing résout aussitôt
    // (spec) — pour éprouver le rejet, repasser par `paused`
    // puis faute hôte pendant la promesse pendante.
    e.deliver_media_state(node, &rep(0, 60_000, 1_000, MediaState::Paused))
        .expect("paused");
    ev(
        &mut e,
        "R='pending2';v.play().then(\
         function(){R='resolved'},function(x){R='rejected'});'armed'",
    );
    let err = MediaReport {
        error: Some("network: gone".into()),
        ..rep(0, 60_000, 1_000, MediaState::Error)
    };
    e.deliver_media_state(node, &err).expect("error");
    assert_eq!(ev(&mut e, "R"), "rejected");
}

#[test]
fn media_state_unclaimed_report_is_noop() {
    let (mut e, dom) = engine();
    let node = dom.borrow().by_id("v").expect("v node");
    ev(
        &mut e,
        "var L=[];var v=document.getElementById('v');\
         ['loadedmetadata','canplay','playing'].forEach(\
         function(t){v.addEventListener(t,function(){L.push(t)})});'armed'",
    );
    // Pas de claim → le rapport tombe (miroir sans état),
    // honnêteté ADR-0023 : rien n'est simulé.
    e.deliver_media_state(node, &rep(0, 60_000, 60_000, MediaState::Playing))
        .expect("dropped");
    assert_eq!(ev(&mut e, "L.length+'|'+v.readyState"), "0|0");
}

#[test]
fn media_mse_append_drives_progress_and_readiness() {
    let (mut e, dom) = engine();
    let node = claim_and_arm(&mut e, &dom);
    // Attache blob: → sourceopen filé (JS-C324), buffer,
    // append (plage estimée ~[0,6 s] — estimateur JS-C284).
    ev(
        &mut e,
        "var ms=new MediaSource();\
         v.src=URL.createObjectURL(ms);\
         var sb=ms.addSourceBuffer('video/mp4');\
         sb.appendBuffer(new ArrayBuffer(64));'appended'",
    );
    // `progress` a été filé à l'updateend vers l'élément
    // attaché — servi à la frontière de job (eval suivant).
    assert_eq!(ev(&mut e, "L.indexOf('progress')>=0"), "true");
    // Rapport hôte `buffered_ms=0` — sous MSE l'échelle lit
    // l'union SourceBuffer (~6 s de données), pas le rapport :
    // canplay monte quand même (hls.js gate — le réviseur).
    e.deliver_media_state(node, &rep(0, 6_000, 0, MediaState::Playing))
        .expect("mse report");
    assert_eq!(ev(&mut e, "v.buffered.length"), "1");
    assert_eq!(ev(&mut e, "v.buffered.end(0)>=5"), "true");
    assert_eq!(
        ev(
            &mut e,
            "L.indexOf('loadedmetadata')>=0&&L.indexOf('canplay')>=0"
        ),
        "true"
    );
    assert_eq!(ev(&mut e, "v.readyState>=3"), "true");
}
