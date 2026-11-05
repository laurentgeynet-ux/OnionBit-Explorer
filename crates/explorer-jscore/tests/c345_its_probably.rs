// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! JS-C345 — `MediaSource.isTypeSupported` rend `"probably"`
//! quand la liste `codecs=` est entièrement déclarée (mur §B-95
//! du corpus `YouTube` : le verdict plafonnait à `"maybe"` — les
//! players qui exigent `probably` n'étaient jamais servis,
//! alors que `canPlayType` rendait déjà `probably` sur la même
//! table honnête). Verdict désormais **partagé** :
//! conteneur connu + `codecs=` tous déclarés → `"probably"` ;
//! conteneur connu sans `codecs` → `"maybe"` ; sinon `""`.
//! Table vide → `""` partout (état C173 préservé).

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

/// Chaîne complète déclarée → `probably` (rouge avant le fix :
/// `maybe`). Conteneur seul → `maybe` inchangé. Inconnu → `""`.
#[test]
fn c345_is_type_supported_three_tier_verdict() {
    let mut e = engine_codecs(&["avc1.4D400B", "mp4a.40.2", "opus", "vp09"]);
    assert_eq!(
        ev(
            &mut e,
            "MediaSource.isTypeSupported('video/mp4;codecs=\"avc1.4D400B\"')+'|'+\
             MediaSource.isTypeSupported('video/mp4;codecs=\"avc1.4D400B,mp4a.40.2\"')+'|'+\
             MediaSource.isTypeSupported('video/webm;codecs=\"vp09.02.51.10\"')+'|'+\
             MediaSource.isTypeSupported('audio/webm;codecs=\"opus\"')+'|'+\
             MediaSource.isTypeSupported('video/mp4')+'|'+\
             MediaSource.isTypeSupported('audio/webm')+'|'+\
             MediaSource.isTypeSupported('video/mp4;codecs=\"avc1.4D400B,vp08\"')+'|'+\
             MediaSource.isTypeSupported('video/quicktime')",
        ),
        "probably|probably|probably|probably|maybe|maybe||"
    );
}

/// Table vide → `""` partout, conteneur connu compris (état
/// C173 honnête préservé — jamais de promesse sans transport).
#[test]
fn c345_empty_table_stays_honest_empty() {
    let mut e = engine_codecs(&[]);
    assert_eq!(
        ev(
            &mut e,
            "MediaSource.isTypeSupported('video/mp4')+'|'+\
             MediaSource.isTypeSupported('video/mp4;codecs=\"avc1\"')",
        ),
        "|"
    );
}
