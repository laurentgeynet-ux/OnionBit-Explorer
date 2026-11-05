// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! JS-C347 — tolérance aux paramètres MIME hors `codecs=`
//! (mur §B-101 du vol `YouTube` : le player énumère ~242
//! sondes paramétrées `…;codecs="vp9";height=360;framerate=…;
//! eotf=…` — `split("codecs=")` avalait `";height=360` dans la
//! valeur → pseudo-codec jamais déclaré → `""` → table de
//! formats inexploitable). [`codecs_param`] scanne les params
//! `;name=value` à frontière stricte — seul `codecs` porte la
//! capacité, les paramètres média (height/framerate/bitrate/
//! eotf/dvh/quality) sont ignorés, pas fatals.

use explorer_jscore::{Dom, Engine};
use std::cell::RefCell;
use std::rc::Rc;

fn ev(e: &mut Engine, src: &str) -> String {
    match e.eval(src) {
        Ok(v) => e.describe(v),
        Err(err) => panic!("{src} -> {err}"),
    }
}

fn engine_codecs(codecs: &[&str]) -> Engine {
    let mut e = Engine::new();
    e.install_dom(&Rc::new(RefCell::new(Dom::from_html(
        "<html><body></body></html>",
    ))));
    let owned: Vec<String> = codecs.iter().map(|c| (*c).to_owned()).collect();
    e.note_media_codecs(&owned);
    e
}

/// La forme exacte du vol : paramètres média **après** et
/// **avant** `codecs=`, liste multi-codecs quotée, `eotf`/`dvh`.
#[test]
fn c347_media_params_do_not_break_verdict() {
    let mut e = engine_codecs(&["avc1.4D400B", "vp09.02.51.10", "mp4a.40.2", "opus"]);
    assert_eq!(
        ev(
            &mut e,
            "MediaSource.isTypeSupported('video/mp4;codecs=\"avc1.4d\";height=360')+'|'+\
             MediaSource.isTypeSupported('video/mp4;height=360;codecs=\"avc1.4d\"')+'|'+\
             MediaSource.isTypeSupported('video/mp4;codecs=\"avc1.4d,mp4a.40.2\";bitrate=1e6')+'|'+\
             MediaSource.isTypeSupported('video/webm;codecs=\"vp09.02.51.10\";framerate=30;eotf=bt709')+'|'+\
             MediaSource.isTypeSupported('audio/webm;codecs=\"opus\";dvh=1')+'|'+\
             document.createElement('video').canPlayType('video/mp4;codecs=\"avc1.4d\";width=640;quality=hd')",
        ),
        "probably|probably|probably|probably|probably|probably"
    );
}

/// Frontières honnêtes : `xcodecs=` ne matche pas (→ `maybe`
/// conteneur seul), conteneur paramétré sans `codecs` →
/// `maybe`, codec non déclaré + params → `""`, conteneur
/// inconnu + codec déclaré → `""`.
#[test]
fn c347_param_boundaries_stay_honest() {
    let mut e = engine_codecs(&["avc1", "vp09"]);
    assert_eq!(
        ev(
            &mut e,
            "MediaSource.isTypeSupported('video/mp4;xcodecs=\"avc1\"')+'|'+\
             MediaSource.isTypeSupported('video/mp4;height=360')+'|'+\
             MediaSource.isTypeSupported('video/mp4;codecs=\"av99\";height=360')+'|'+\
             MediaSource.isTypeSupported('video/mkv;codecs=\"avc1\"')",
        ),
        "maybe|maybe||"
    );
}

/// Symétrie côté déclaration : un claim MIME complet portant
/// des paramètres média alimente la table — `video/mp4;
/// codecs="vp9";height=1080` déclare `vp09`.
#[test]
fn c347_declare_side_parses_param_scoped() {
    let mut e = engine_codecs(&["video/mp4;codecs=\"vp9\";height=1080"]);
    assert_eq!(
        ev(
            &mut e,
            "MediaSource.isTypeSupported('video/webm;codecs=\"vp09.02.51.10\"')",
        ),
        "probably"
    );
}
