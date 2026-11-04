// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! JS-C327 — `SourceBuffer.audioTracks`/`videoTracks`/
//! `textTracks` peuplées depuis les `trak` de l'init `moov`
//! fMP4 : `tkhd.track_ID`, `hdlr.handler_type` (kind),
//! `mdhd.language` ; `addtrack`/`removetrack`/`change`
//! filés en tâche (même couture JS-C324) ; état mutable
//! spec (`enabled`/`selected` exclusif/`mode`).

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

// ---- fixtures fMP4 minimales (size+fourcc+payload) ----

fn bx(tag: [u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut v = u32::try_from(payload.len() + 8)
        .expect("box < 4Gio")
        .to_be_bytes()
        .to_vec();
    v.extend_from_slice(&tag);
    v.extend_from_slice(payload);
    v
}

/// `tkhd` v0 — seul `track_ID` (offset 12) est lu.
fn tkhd(id: u32) -> Vec<u8> {
    let mut p = vec![0u8; 84];
    p[0..4].copy_from_slice(&[0, 0, 0, 7]); // version+flags
    p[12..16].copy_from_slice(&id.to_be_bytes());
    p[76..80].copy_from_slice(&(640u32 << 16).to_be_bytes()); // width 16.16
    p[80..84].copy_from_slice(&(480u32 << 16).to_be_bytes()); // height
    bx(*b"tkhd", &p)
}

/// `mdhd` v0 — `timescale` (offset 12) + `language` (offset
/// 20, ISO 639-2/T packé 3×5 bits).
fn mdhd(ts: u32, lang: u16) -> Vec<u8> {
    let mut p = vec![0u8; 24];
    p[12..16].copy_from_slice(&ts.to_be_bytes());
    p[20..22].copy_from_slice(&lang.to_be_bytes());
    bx(*b"mdhd", &p)
}

/// `hdlr` — `handler_type` à l'offset 8.
fn hdlr(kind: [u8; 4]) -> Vec<u8> {
    let mut p = vec![0u8; 24];
    p[8..12].copy_from_slice(&kind);
    bx(*b"hdlr", &p)
}

fn trak(id: u32, ts: u32, lang: u16, handler: [u8; 4]) -> Vec<u8> {
    let mut mdia = mdhd(ts, lang);
    mdia.extend_from_slice(&hdlr(handler));
    let mut t = tkhd(id);
    t.extend_from_slice(&bx(*b"mdia", &mdia));
    bx(*b"trak", &t)
}

fn moov(traks: &[Vec<u8>]) -> Vec<u8> {
    let mut p = Vec::new();
    for t in traks {
        p.extend_from_slice(t);
    }
    bx(*b"moov", &p)
}

/// ISO 639-2/T 3 lettres → u16 packé (`eng` → 0x15c7).
fn lang3(s: [u8; 3]) -> u16 {
    (u16::from(s[0]) - 0x60) << 10 | (u16::from(s[1]) - 0x60) << 5 | (u16::from(s[2]) - 0x60)
}

/// `MediaSource` attachée + `SourceBuffer` — chemin player.
fn attach_sb(e: &mut Engine) {
    e.eval(
        "var ms=new MediaSource();window.__ms=ms;\
         document.getElementById('v').src=URL.createObjectURL(ms);\
         var sb=ms.addSourceBuffer('video/mp4');window.__sb=sb;",
    )
    .expect("attach");
}

/// `appendBuffer` d'un init `moov` (hex inline).
fn append_init(e: &mut Engine, init: &[u8]) {
    e.eval(&format!(
        "__sb.appendBuffer(new Uint8Array({init:?}).buffer);"
    ))
    .expect("append init");
}

#[test]
fn mse5_tracks_populated_from_init() {
    let mut e = engine();
    attach_sb(&mut e);
    let init = moov(&[
        trak(1, 90000, lang3(*b"fra"), *b"vide"),
        trak(2, 48000, lang3(*b"eng"), *b"soun"),
    ]);
    append_init(&mut e, &init);
    assert_eq!(
        ev(
            &mut e,
            "__sb.videoTracks.length+'|'+__sb.audioTracks.length+'|'+__sb.textTracks.length"
        ),
        "1|1|0"
    );
    assert_eq!(
        ev(
            &mut e,
            "var v=__sb.videoTracks[0],a=__sb.audioTracks[0];\
             v.id+'|'+v.language+'|'+a.id+'|'+a.language+'|'+\
             (v instanceof VideoTrack)+'|'+(a instanceof AudioTrack)+'|'+\
             (__sb.videoTracks instanceof VideoTrackList)+'|'+\
             (v.sourceBuffer===__sb)+'|'+(__sb.videoTracks===__sb.videoTracks)+'|'+\
             (v===__sb.videoTracks[0])"
        ),
        "1|fra|2|eng|true|true|true|true|true|true"
    );
    // Defaults spec : audio enabled, vidéo non sélectionnée.
    assert_eq!(
        ev(
            &mut e,
            "__sb.audioTracks[0].enabled+'|'+__sb.videoTracks[0].selected+'|'+__sb.videoTracks.selectedIndex"
        ),
        "true|false|-1"
    );
    // `kind` audio/vidéo indéterminé depuis `hdlr` → `""`.
    assert_eq!(
        ev(&mut e, "var a=__sb.audioTracks[0];a.kind+'|'+a.label"),
        "|"
    );
}

#[test]
fn mse5_addtrack_queued_as_task_with_track() {
    let mut e = engine();
    attach_sb(&mut e);
    // Listeners posés AVANT l'append : l'event est filé en
    // tâche — invisible dans l'expression synchrone.
    ev(
        &mut e,
        "var seen=[];\
         __sb.videoTracks.addEventListener('addtrack',function(ev){seen.push('v'+ev.track.id)});\
         __sb.audioTracks.addEventListener('addtrack',function(ev){seen.push('a'+ev.track.id)});",
    );
    let init = moov(&[
        trak(1, 90000, lang3(*b"fra"), *b"vide"),
        trak(2, 48000, lang3(*b"eng"), *b"soun"),
    ]);
    // `seen.length` lu dans LE MÊME eval que l'append →
    // synchrone `0` : l'event est filé en tâche et n'arrive
    // qu'à la frontière de job (fin du script).
    assert_eq!(
        e.eval(&format!(
            "__sb.appendBuffer(new Uint8Array({init:?}).buffer);seen.length"
        ))
        .map(|v| e.describe(v))
        .expect("append"),
        "0",
        "addtrack est une tâche, pas synchrone"
    );
    // La tâche est servie à la frontière de job suivante.
    assert_eq!(ev(&mut e, "seen.join(',')"), "v1,a2");
}

#[test]
fn mse5_track_state_changes_fire_change() {
    let mut e = engine();
    attach_sb(&mut e);
    ev(
        &mut e,
        "var ch=[];\
         __sb.audioTracks.addEventListener('change',function(){ch.push('a')});\
         __sb.videoTracks.addEventListener('change',function(){ch.push('v')});\
         __sb.textTracks.addEventListener('change',function(){ch.push('t')});",
    );
    let init = moov(&[
        trak(1, 90000, lang3(*b"fra"), *b"vide"),
        trak(7, 90000, lang3(*b"eng"), *b"vide"),
        trak(2, 48000, lang3(*b"eng"), *b"soun"),
        trak(3, 1000, lang3(*b"eng"), *b"subt"),
    ]);
    append_init(&mut e, &init);
    // `enabled` sur AudioTrack → `change` sur audioTracks.
    // `selected` vidéo exclusif : le second désélectionne le
    // premier, `selectedIndex` suit.
    // `mode` TextTrack validé (`disabled|hidden|showing`).
    assert_eq!(
        ev(
            &mut e,
            "__sb.audioTracks[0].enabled=false;\
             __sb.videoTracks[0].selected=true;\
             __sb.videoTracks[1].selected=true;\
             __sb.textTracks[0].mode='hidden';\
             __sb.audioTracks[0].enabled+'|'+\
             __sb.videoTracks[0].selected+'|'+__sb.videoTracks[1].selected+'|'+\
             __sb.videoTracks.selectedIndex+'|'+__sb.textTracks[0].mode+'|'+ch.join(',')"
        ),
        "false|false|true|1|hidden|a,v,v,t"
    );
    // `mode` invalide → erreur.
    assert_eq!(
        ev(
            &mut e,
            "try{__sb.textTracks[0].mode='bogus';'none'}catch(x){x.name||'TypeError'}"
        ),
        "TypeError"
    );
}

#[test]
fn mse5_reinit_reconciles_tracks() {
    let mut e = engine();
    attach_sb(&mut e);
    let init_ab = moov(&[
        trak(1, 90000, lang3(*b"fra"), *b"vide"),
        trak(2, 48000, lang3(*b"eng"), *b"soun"),
    ]);
    append_init(&mut e, &init_ab);
    ev(
        &mut e,
        "var rm=[];\
         __sb.audioTracks.addEventListener('removetrack',function(ev){rm.push(ev.track.id)});\
         var adds=0;\
         __sb.videoTracks.addEventListener('addtrack',function(){adds++});\
         __sb.audioTracks[0].enabled=false;\
         window.__v1=__sb.videoTracks[0];",
    );
    // Re-init MÊMES ids (switch ABR hls.js) : pistes
    // conservées — mêmes wrappers, état préservé, aucun
    // addtrack/removetrack.
    let init_same = moov(&[
        trak(1, 90000, lang3(*b"fra"), *b"vide"),
        trak(2, 48000, lang3(*b"eng"), *b"soun"),
    ]);
    append_init(&mut e, &init_same);
    assert_eq!(
        ev(
            &mut e,
            "(__sb.videoTracks[0]===window.__v1)+'|'+__sb.audioTracks[0].enabled+'|'+adds+'|'+rm.length"
        ),
        "true|false|0|0"
    );
    // Init sans piste audio → `removetrack` filé avec le
    // snapshot de la piste disparue ; la vidéo survit.
    let init_v_only = moov(&[trak(1, 90000, lang3(*b"fra"), *b"vide")]);
    append_init(&mut e, &init_v_only);
    ev(&mut e, "void 0");
    assert_eq!(
        ev(
            &mut e,
            "__sb.audioTracks.length+'|'+(__sb.videoTracks[0]===window.__v1)+'|'+rm.join(',')"
        ),
        "0|true|2"
    );
}

#[test]
fn mse5_text_track_and_get_track_by_id() {
    let mut e = engine();
    attach_sb(&mut e);
    let init = moov(&[
        trak(1, 90000, lang3(*b"fra"), *b"vide"),
        trak(3, 1000, lang3(*b"eng"), *b"subt"),
        trak(4, 1000, 0x55c4, *b"meta"),
    ]);
    append_init(&mut e, &init);
    // Deux pistes texte : `subt` → kind subtitles, `meta`
    // → metadata ; `mdhd` `und` → language `""` ; `cues`
    // honnêtement `null` (pas de démuxage des samples).
    assert_eq!(
        ev(
            &mut e,
            "var tt=__sb.textTracks;\
             tt.length+'|'+tt[0].kind+'|'+tt[0].language+'|'+tt[0].mode+'|'+tt[0].cues+'|'+\
             tt[1].kind+'|'+tt[1].language+'|'+(tt[1] instanceof TextTrack)"
        ),
        "2|subtitles|eng|disabled|null|metadata||true"
    );
    assert_eq!(
        ev(
            &mut e,
            "var t=__sb.textTracks.getTrackById('4');\
             (t===__sb.textTracks[1])+'|'+__sb.videoTracks.getTrackById('1').id+'|'+\
             __sb.textTracks.getTrackById('99')"
        ),
        "true|1|null"
    );
}
