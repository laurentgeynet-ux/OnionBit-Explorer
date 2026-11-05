// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! JS-C346 — résolution des **alias** de codecs dans le matching
//! `isTypeSupported`/`canPlayType` (mur §B-94 des vols `YouTube`
//! : les players sondent `codecs="vp9"`/`"h264"`/`"av1"`/`"aac"`
//! pour les familles déclarées `vp09`/`avc1`/`av01`/`mp4a` — la
//! requête rendait `""` table pleine, mesuré `vp9`→`""` au vol
//! 649). [`codec_family`] applique la résolution aux deux bords
//! (déclaration + requête). Les codecs réellement différents
//! (`eac3`/`ec-3` ≠ `ac3`, `hvc1` ≠ `hev1`) restent des
//! négatifs honnêts.

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

/// Aliases de la batterie terrain (`vp09`/`avc1`/`av01`/`mp4a`/
/// `hev1`/`ac3` déclarés) — les noms courts des players doivent
/// rejoindre la famille déclarée.
#[test]
fn c346_short_aliases_resolve_to_declared_families() {
    let mut e = engine_codecs(&[
        "avc1.4D400B",
        "hev1.1.6.L93.B0",
        "hvc1",
        "vp09.02.51.10",
        "av01.0.05M.08",
        "mp4a.40.2",
        "ac3",
        "opus",
    ]);
    // verbatim terrain : `vp9`/`h264`/`av1`/`hevc`/`aac`/`ac-3`.
    assert_eq!(
        ev(
            &mut e,
            "MediaSource.isTypeSupported('video/webm;codecs=\"vp9\"')+'|'+\
             MediaSource.isTypeSupported('video/mp4;codecs=\"h264\"')+'|'+\
             MediaSource.isTypeSupported('video/mp4;codecs=\"avc\"')+'|'+\
             MediaSource.isTypeSupported('video/mp4;codecs=\"av1\"')+'|'+\
             MediaSource.isTypeSupported('video/mp4;codecs=\"h265\"')+'|'+\
             MediaSource.isTypeSupported('video/mp4;codecs=\"hevc\"')+'|'+\
             MediaSource.isTypeSupported('audio/mp4;codecs=\"aac\"')+'|'+\
             MediaSource.isTypeSupported('audio/mp4;codecs=\"ac-3\"')+'|'+\
             document.createElement('video').canPlayType('video/webm;codecs=\"vp9\"')",
        ),
        "probably|probably|probably|probably|probably|probably|probably|probably|probably"
    );
}

/// Discriminants honnêts : `eac3`/`ec-3` (≠ `ac3`), `hvc1`
/// (param-carriage distinct — non déclaré ici), `vp10`/`avc2`
/// (inexistants) → `""`.
#[test]
fn c346_different_codecs_stay_honest_negatives() {
    let mut e = engine_codecs(&["ac3", "hev1", "avc1", "vp09"]);
    assert_eq!(
        ev(
            &mut e,
            "MediaSource.isTypeSupported('audio/mp4;codecs=\"eac3\"')+'|'+\
             MediaSource.isTypeSupported('audio/mp4;codecs=\"ec-3\"')+'|'+\
             MediaSource.isTypeSupported('video/mp4;codecs=\"hvc1\"')+'|'+\
             MediaSource.isTypeSupported('video/webm;codecs=\"vp10\"')+'|'+\
             MediaSource.isTypeSupported('video/mp4;codecs=\"avc2\"')",
        ),
        "||||"
    );
}

/// Symétrie côté déclaration : un hôte qui déclare le nom
/// court `vp9` alimente le préfixe canonique `vp09` — une
/// requête `vp09.…` le rejoint.
#[test]
fn c346_alias_resolution_applies_on_declare_side() {
    let mut e = engine_codecs(&["vp9", "h264"]);
    assert_eq!(
        ev(
            &mut e,
            "MediaSource.isTypeSupported('video/webm;codecs=\"vp09.02.51.10\"')+'|'+\
             MediaSource.isTypeSupported('video/mp4;codecs=\"avc1.4D400B\"')",
        ),
        "probably|probably"
    );
}
