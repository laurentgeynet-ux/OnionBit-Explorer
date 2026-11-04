// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! JS-C325 — complétude MSE côté API : machine à états spec
//! (`updating` re-entrant → `InvalidStateError` sur toute la
//! surface `SourceBuffer`/`endOfStream`/`duration` ;
//! `appendBuffer`/`abort` sur parent `ended` → `open` +
//! `sourceopen` filé — le mécanisme live-edge hls.js), `abort`
//! non destructeur (média committé conservé — spec, l'ancienne
//! sémantique « abort vide tout » vidait `buffered`), param
//! `endOfStream("network"|"decode")` → `MediaError` élément,
//! `changeType` validé quand la table `declared_codecs` est
//! renseignée, `MediaSource.duration` tronquante, `mode:
//! "sequence"` chaîné à la fin bufferisée.
//! Déviation conservée : opérations permises hors attache
//! (`closed`) — le transport `mse_pull_segments` sert
//! légitimement sans DOM (mse3).

use explorer_jscore::{Dom, Engine};
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

/// `MediaSource` créée, attachée au `<video>` (`__ms`),
/// `blob:` rendu — chemin player réel.
fn attach(e: &mut Engine) -> String {
    e.eval(
        "var ms=new MediaSource();\
         window.__ms=ms;\
         var u=URL.createObjectURL(ms);\
         document.getElementById('v').src=u;\
         u",
    )
    .map(|v| e.describe(v))
    .expect("attach")
}

#[test]
fn mse4_append_on_ended_reopens_and_pull_not_eos() {
    let (mut e, _dom) = engine();
    let u = attach(&mut e);
    assert_eq!(
        ev(
            &mut e,
            "var sb=__ms.addSourceBuffer('video/mp4');\
             var n=0;\
             __ms.addEventListener('sourceopen',function(){n++});\
             __ms.endOfStream();\
             __ms.readyState+'|'+n"
        ),
        "ended|0"
    );
    // Spec : `appendBuffer` sur `ended` → `open` + `sourceopen`
    // re-filé — sans lui un player live (hls.js live-edge :
    // `endOfStream()` à la frontière puis appends reprennent)
    // restait `ended` et le pull rendait `eos` figé.
    assert_eq!(
        ev(
            &mut e,
            "n=0;\
             sb.appendBuffer(new Uint8Array([1,2]));\
             __ms.readyState"
        ),
        "open"
    );
    assert_eq!(ev(&mut e, "n"), "1");
    let (data, _, eos, _) = e
        .mse_pull_segments(&u, 0, 16, None, false)
        .expect("pull")
        .expect("servable");
    assert_eq!(data, vec![1, 2]);
    assert!(
        !eos,
        "append post-EOS rouvre le flux — eos figé = live mort"
    );
}

#[test]
fn mse4_updating_guards_throw_invalid_state() {
    let (mut e, _dom) = engine();
    attach(&mut e);
    // Spec : toute la surface `SourceBuffer` + `endOfStream` +
    // `MediaSource.duration` jettent `InvalidStateError` pendant
    // `updating` — fenêtre re-entrante = listener `updatestart`
    // synchrone (nos events sont tirés en séquence).
    assert_eq!(
        ev(
            &mut e,
            "var sb=__ms.addSourceBuffer('video/mp4');\
             var names=[];\
             sb.addEventListener('updatestart',function(){\
               try{sb.appendBuffer(new ArrayBuffer(4))}catch(x){names.push(x.name)}\
               try{sb.remove(0,1)}catch(x){names.push(x.name)}\
               try{sb.changeType('audio/mp4')}catch(x){names.push(x.name)}\
               try{sb.timestampOffset=2}catch(x){names.push(x.name)}\
               try{sb.appendWindowStart=1}catch(x){names.push(x.name)}\
               try{sb.mode='sequence'}catch(x){names.push(x.name)}\
               try{__ms.endOfStream()}catch(x){names.push(x.name)}\
               try{__ms.duration=30}catch(x){names.push(x.name)}\
               names.push('u='+sb.updating)});\
             sb.appendBuffer(new ArrayBuffer(8));\
             names.join('|')"
        ),
        "InvalidStateError|InvalidStateError|InvalidStateError|\
         InvalidStateError|InvalidStateError|InvalidStateError|\
         InvalidStateError|InvalidStateError|u=true"
    );
}

#[test]
fn mse4_abort_preserves_buffered_and_reopens() {
    let (mut e, _dom) = engine();
    let u = attach(&mut e);
    // Spec : `abort` conserve le média committé — `buffered`
    // survit — et rouvre un parent `ended` (même mécanisme
    // live-edge qu'`appendBuffer`).
    assert_eq!(
        ev(
            &mut e,
            "var sb=__ms.addSourceBuffer('video/mp4');\
             var n=0;\
             __ms.onsourceopen=function(){n++};\
             sb.appendBuffer(new Uint8Array(8));\
             var b0=sb.buffered.end(0);\
             __ms.endOfStream();\
             sb.abort();\
             b0+'|'+sb.buffered.length+'|'+sb.buffered.end(0)+'|'+__ms.readyState"
        ),
        "6|1|6|open"
    );
    assert_eq!(ev(&mut e, "n"), "1");
    // La file transporte les octets committés malgré l'abort —
    // `updateend` avait déjà livré l'append.
    let (data, _, eos, _) = e
        .mse_pull_segments(&u, 0, 16, None, false)
        .expect("pull")
        .expect("servable");
    assert_eq!(data.len(), 8);
    assert!(!eos);
}

#[test]
fn mse4_end_of_stream_error_codes_reach_element() {
    let (mut e, dom) = engine();
    attach(&mut e);
    let node = dom.borrow().by_id("v").expect("v");
    // Claim = la surface a un état de lecture à corrompre —
    // `v.error` reflète le `MediaError` classifié (étape 182).
    e.set_media_claim(node, 7, Some(vec!["avc1".into()]));
    assert_eq!(
        ev(
            &mut e,
            "var sb=__ms.addSourceBuffer('video/mp4');\
             try{__ms.endOfStream('bogus')}catch(x){x.name}"
        ),
        "TypeError"
    );
    assert_eq!(
        ev(
            &mut e,
            "var v=document.getElementById('v');\
             var seen='';\
             v.addEventListener('error',function(){seen='E'});\
             __ms.endOfStream('network');\
             __ms.readyState+'|'+(v.error instanceof MediaError)+'|'+\
             v.error.code+'|'+(v.error.message.indexOf('network')>=0)+'|'+seen"
        ),
        "ended|true|2|true|E"
    );
    // Réouverture par append (spec) puis `"decode"` → 3.
    assert_eq!(
        ev(
            &mut e,
            "sb.appendBuffer(new Uint8Array(4));\
             __ms.endOfStream('decode');\
             document.getElementById('v').error.code"
        ),
        "3"
    );
    // Unclaimed : même `error` tiré, `el.error` reste null
    // (pas d'état de lecture — déviation documentée).
    let (mut e2, _dom2) = engine();
    attach(&mut e2);
    assert_eq!(
        ev(
            &mut e2,
            "var ms2=new MediaSource();window.__m2=ms2;\
             document.getElementById('v').src=URL.createObjectURL(ms2);\
             ms2.addSourceBuffer('video/mp4');\
             var seen2='';\
             document.getElementById('v').addEventListener('error',function(){seen2='E'});\
             ms2.endOfStream('network');\
             seen2+'|'+(document.getElementById('v').error===null)"
        ),
        "E|true"
    );
}

#[test]
fn mse4_change_type_validates_when_codecs_declared() {
    let (mut e, dom) = engine();
    let u = attach(&mut e);
    let node = dom.borrow().by_id("v").expect("v");
    // `declared_codecs = {avc1}` via le claim (MSE-4) — la
    // validation `NotSupportedError` ne s'arme que sur vérité
    // terrain (table vide = tolérance `addSourceBuffer`).
    e.set_media_claim(node, 7, Some(vec!["avc1".into()]));
    assert_eq!(
        ev(
            &mut e,
            "var sb=__ms.addSourceBuffer('video/mp4');\
             var bad='',ok='';\
             try{sb.changeType('video/mp4;codecs=\"vp09\"')}catch(x){bad=x.name}\
             try{sb.changeType('video/mp4;codecs=\"avc1.42E01E\"');ok='Y'}catch(x){ok=x.name}\
             bad+'|'+ok"
        ),
        "NotSupportedError|Y"
    );
    // Le nouveau mime est publié par le seam — le transport
    // hôte voit le switch codec au prochain claim.
    assert_eq!(
        e.mse_source_buffers(&u),
        vec!["video/mp4;codecs=\"avc1.42E01E\"".to_string()]
    );
}

#[test]
fn mse4_duration_set_truncates_buffered() {
    let (mut e, _dom) = engine();
    attach(&mut e);
    // Spec : nouvelle durée < ancienne → suppression de plage
    // sur tous les buffers (`est_ranges` clippées — `buffered`
    // ne dépasse plus la durée déclarée).
    assert_eq!(
        ev(
            &mut e,
            "var sb=__ms.addSourceBuffer('video/mp4');\
             sb.appendBuffer(new Uint8Array(8));\
             sb.appendWindowStart=40;\
             sb.appendBuffer(new Uint8Array(8));\
             var pre=sb.buffered.length+'/'+sb.buffered.end(1);\
             __ms.duration=50;\
             __ms.duration=30;\
             pre+'|'+sb.buffered.length+'/'+sb.buffered.end(0)"
        ),
        "2/46|1/6"
    );
    assert_eq!(
        ev(
            &mut e,
            "var r=[];\
             try{__ms.duration=NaN}catch(x){r.push(x.name)}\
             try{__ms.duration=-1}catch(x){r.push(x.name)}\
             r.join(',')"
        ),
        "TypeError,TypeError"
    );
}

#[test]
fn mse4_sequence_mode_chains_at_buffered_end() {
    let (mut e, _dom) = engine();
    attach(&mut e);
    // Spec `mode:"sequence"` : chaque append chaîne à la fin
    // du dernier range — un `timestampOffset` explicite pose
    // un nouveau point de chaînage puis la suite enchaîne.
    assert_eq!(
        ev(
            &mut e,
            "var sb=__ms.addSourceBuffer('video/mp4');\
             sb.mode='sequence';\
             sb.appendBuffer(new Uint8Array(8));\
             var a=sb.buffered.start(0)+'/'+sb.buffered.end(0);\
             sb.timestampOffset=50;\
             sb.appendBuffer(new Uint8Array(8));\
             var b=sb.buffered.length+'/'+sb.buffered.start(1)+'/'+sb.buffered.end(1);\
             sb.appendBuffer(new Uint8Array(8));\
             a+'|'+b+'|'+sb.buffered.length+'/'+sb.buffered.end(1)"
        ),
        "0/6|2/50/56|2/62"
    );
    assert_eq!(ev(&mut e, "sb.timestampOffset"), "62");
}

#[test]
fn mse4_add_source_buffer_empty_and_double_eos() {
    let (mut e, _dom) = engine();
    attach(&mut e);
    assert_eq!(
        ev(&mut e, "try{__ms.addSourceBuffer('')}catch(x){x.name}"),
        "TypeError"
    );
    // Double `endOfStream` = no-op propre — `sourceended`
    // n'est pas re-filé (déviation documentée vs le throw
    // spec, les players n'EOS que sur `open`).
    assert_eq!(
        ev(
            &mut e,
            "var n=0;\
             __ms.onsourceended=function(){n++};\
             __ms.endOfStream();\
             __ms.endOfStream();\
             __ms.readyState"
        ),
        "ended"
    );
    assert_eq!(ev(&mut e, "n"), "1");
}
