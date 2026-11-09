// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! JS-C454 (JT-10) — surface `VTTCue`/`TextTrack` minimale
//! (vol 722 — l'interface était absente) : ctor `new
//! VTTCue(start, end, text)` + props spec + `getCueAsHTML`
//! (fragment + nœud texte, tags de cue bornés) ;
//! `media.addTextTrack` vraie `TextTrack` greffée à
//! `textTracks` (`addCue`/`removeCue`/`cues`/`activeCues`).

use explorer_jscore::{Dom, Engine};
use std::cell::RefCell;
use std::rc::Rc;

fn engine() -> Engine {
    let mut e = Engine::new();
    e.install_dom(&Rc::new(RefCell::new(Dom::from_html(
        "<html><body><video id=v></video></body></html>",
    ))));
    e
}

fn ev(e: &mut Engine, src: &str) -> String {
    match e.eval(src) {
        Ok(v) => e.describe(v),
        Err(err) => panic!("{src} -> {err}"),
    }
}

/// `new VTTCue` remplit les props spec ; appel nu = `TypeError`.
#[test]
fn c454_vttcue_ctor_and_props() {
    let mut e = engine();
    assert_eq!(
        ev(
            &mut e,
            "var c = new VTTCue(1.5, 4.0, 'hello');\
             c.startTime + '|' + c.endTime + '|' + c.text + '|' + c.align + '|' + \
             c.snapToLines + '|' + c.size + '|' + (c instanceof VTTCue)"
        ),
        "1.5|4|hello|center|true|100|true"
    );
    assert_eq!(
        ev(
            &mut e,
            "try { VTTCue(0, 1, 'x'); 'no' } catch (er) { er.name }"
        ),
        "TypeError"
    );
}

/// `getCueAsHTML` → fragment + nœud texte.
#[test]
fn c454_vttcue_getcueashtml_fragment() {
    let mut e = engine();
    assert_eq!(
        ev(
            &mut e,
            "var f = new VTTCue(0, 1, 'line').getCueAsHTML();\
             (f instanceof DocumentFragment) + '|' + f.childNodes.length + '|' + \
             f.childNodes[0].textContent"
        ),
        "true|1|line"
    );
}

/// `addTextTrack` → `TextTrack` greffée à `textTracks`,
/// `addCue`/`removeCue`/`cues`/`activeCues` vivants.
#[test]
fn c454_addtexttrack_and_cues() {
    let mut e = engine();
    assert_eq!(
        ev(
            &mut e,
            "var v = document.getElementById('v');\
             var t = v.addTextTrack('subtitles', 'Fr', 'fr');\
             t.addCue(new VTTCue(0, 1, 'a'));\
             t.addCue(new VTTCue(1, 2, 'b'));\
             t.kind + '|' + t.label + '|' + t.language + '|' + t.mode + '|' + \
             v.textTracks.length + '|' + (v.textTracks.item(0) === t) + '|' + \
             t.cues.length + '|' + t.activeCues.length + '|' + t.cues[1].text"
        ),
        "subtitles|Fr|fr|disabled|1|true|2|2|b"
    );
    // removeCue par identité.
    assert_eq!(
        ev(
            &mut e,
            "var t2 = document.getElementById('v').textTracks.item(0);\
             t2.removeCue(t2.cues[0]); t2.cues.length + '|' + t2.cues[0].text"
        ),
        "1|b"
    );
}
