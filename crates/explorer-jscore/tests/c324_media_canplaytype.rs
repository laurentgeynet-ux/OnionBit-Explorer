// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Verrous §B-82 — mur terrain dash.js/shaka/videojs
//! (l'attach MSE jamais tenté : `video.src` restait vide,
//! aucune surface `mse:true` offerte) : `HTMLMediaElement`
//! exposait ses méthodes uniquement en position d'appel —
//! `typeof v.canPlayType` rendait `undefined`, la feature-
//! detection universelle des players échouait avant tout
//! claim. Les méthodes média sont désormais des props
//! lisibles (même trampoline que `NODE_METHODS`) et
//! `canPlayType` consulte la table moteur `declared_codecs`
//! (même source honnête que `MediaSource.isTypeSupported` :
//! `"probably"` si tous les codecs du param sont déclarés,
//! `"maybe"` conteneur seul, `""` sinon).

use explorer_jscore::{Dom, Engine};
use std::cell::RefCell;
use std::rc::Rc;

fn eval_dom(src: &str) -> String {
    let mut e = Engine::new();
    e.install_dom(&Rc::new(RefCell::new(Dom::from_html(
        "<html><body></body></html>",
    ))));
    match e.eval(src) {
        Ok(v) => e.describe(v),
        Err(err) => panic!("eval: {err}\n  src: {src}"),
    }
}

/// `eval_dom` + codecs hôte déclarés — la table
/// `declared_codecs` que `canPlayType`/`isTypeSupported`
/// consultent (voie `EXPLORER_MEDIA_CODECS` en vol).
fn eval_dom_codecs(src: &str, codecs: &[&str]) -> String {
    let mut e = Engine::new();
    e.install_dom(&Rc::new(RefCell::new(Dom::from_html(
        "<html><body></body></html>",
    ))));
    let owned: Vec<String> = codecs.iter().map(|c| (*c).to_owned()).collect();
    e.note_media_codecs(&owned);
    match e.eval(src) {
        Ok(v) => e.describe(v),
        Err(err) => panic!("eval: {err}\n  src: {src}"),
    }
}

/// §B-82 — les méthodes média sont lisibles : `typeof`,
/// opérateur `in`, et appel réel. Avant, `typeof` rendait
/// `undefined` → les players n'attachaient jamais.
#[test]
fn media_methods_are_readable_props() {
    assert_eq!(
        eval_dom(
            "var v=document.createElement('video');\
             typeof v.canPlayType+'|'+typeof v.play+'|'+typeof v.pause+'|'+\
             ('canPlayType' in v)+'|'+('play' in v)"
        ),
        "function|function|function|true|true"
    );
}

/// §B-82 — `canPlayType` consulte `declared_codecs` même
/// sans claim : c'est la gate d'attach qui se joue AVANT
/// toute offre réseau (dash.js `capabilities`).
#[test]
fn canplaytype_reads_declared_codecs() {
    assert_eq!(
        eval_dom_codecs(
            "var v=document.createElement('video');\
             v.canPlayType('video/mp4')+'|'+\
             v.canPlayType('video/mp4;codecs=\"avc1.42E01E,mp4a.40.2\"')+'|'+\
             v.canPlayType('video/mp4;codecs=\"hvc1.1.6.L93\"')+'|'+\
             v.canPlayType('video/mp4;codecs=\"av01.0.04M.08\"')+'|'+\
             v.canPlayType('video/x-msvideo')+'|'+v.canPlayType('')",
            &["avc1", "hev1", "hvc1", "mp4a", "mp3", "opus"]
        ),
        "maybe|probably|probably|||"
    );
}

/// §B-82 — table vide → `""` partout (ADR-0023 : jamais de
/// promesse sans transport déclaré). Et `<audio>` marche
/// pareil (`is_media` couvre video+audio).
#[test]
fn canplaytype_empty_table_stays_honest() {
    assert_eq!(
        eval_dom(
            "var v=document.createElement('video'),a=document.createElement('audio');\
             v.canPlayType('video/mp4')+'|'+a.canPlayType('audio/mpeg')+'|'+\
             a.canPlayType('audio/mpeg;codecs=\"mp3\"')"
        ),
        "||"
    );
}

/// §B-82 — audio déclaré : `audio/mpeg` + `mp3` → les deux
/// formes répondent (`mp4a.40.5` → préfixe `mp4a` —
/// normalisation `codec_prefix` partagée avec MSE).
#[test]
fn canplaytype_audio_element() {
    assert_eq!(
        eval_dom_codecs(
            "var a=document.createElement('audio');\
             a.canPlayType('audio/mpeg')+'|'+a.canPlayType('audio/mpeg;codecs=\"mp3\"')+'|'+\
             a.canPlayType('audio/mp4;codecs=\"mp4a.40.5\"')",
            &["mp3", "mp4a"]
        ),
        "maybe|probably|probably"
    );
}
