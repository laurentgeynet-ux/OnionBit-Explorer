// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Étape 204c — Web Audio minimal : `AnalyserNode` lit le
//! ring PCM hôte (`AUDIO_MODE_TAP`, 204b) via
//! `MediaElementAudioSourceNode → AnalyserNode` ; destination
//! muette. Zéros honnêtes sur élément non claimé.

use explorer_jscore::{Dom, Engine};
use std::cell::RefCell;
use std::rc::Rc;

const PAGE: &str = r#"<html><body><video id="v"></video><p>x</p></body></html>"#;

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

/// NodeId du `<video>` de la fixture.
fn video_node(dom: &Rc<RefCell<Dom>>) -> u32 {
    let d = dom.borrow();
    (0..d.nodes.len() as u32)
        .find(|&n| d.nodes[n as usize].tag == "video")
        .expect("video présent")
}

/// `fftSize` échantillons f32 stéréo d'une sinusoïde à
/// `bin` périodes → pic garanti dans le bin `bin`.
fn sine_pcm(fft: usize, bin: usize) -> Vec<u8> {
    let mut pcm = Vec::with_capacity(fft * 2 * 4);
    for i in 0..fft {
        let s = (std::f32::consts::TAU * bin as f32 * i as f32 / fft as f32).sin();
        for _ in 0..2 {
            pcm.extend_from_slice(&s.to_le_bytes());
        }
    }
    pcm
}

#[test]
fn analyser_reads_pcm_ring() {
    let (mut e, dom) = engine();
    e.set_media_claim(video_node(&dom), 42, Some(vec!["avc1".into()]));
    e.deliver_media_audio(42, 48_000, 2, 4, &sine_pcm(2048, 16));

    assert_eq!(
        ev(
            &mut e,
            "var ctx=new AudioContext();ctx.state+','+ctx.sampleRate",
        ),
        "suspended,48000"
    );
    assert_eq!(
        ev(
            &mut e,
            "var an=ctx.createAnalyser();\
             var src=ctx.createMediaElementSource(document.getElementById('v'));\
             src.connect(an);\
             an.fftSize+','+an.frequencyBinCount",
        ),
        "2048,1024"
    );
    // Fréquence : un bin ressort > 0 (sinusoïde à bin 16 —
    // la fenêtre Blackman étale l'énergie, le plateau de
    // silence ne vaut que pour l'élément non claimé).
    assert_eq!(
        ev(
            &mut e,
            "var u=new Uint8Array(an.frequencyBinCount);\
             an.getByteFrequencyData(u);\
             var mx=0;for(var i=0;i<u.length;i++)if(u[i]>mx)mx=u[i];mx",
        ),
        "255" // pic normalisé 0 dBFS → max de l'échelle.
    );
    // Temps : la sinusoïde n'est pas un plat 128.
    assert_eq!(
        ev(
            &mut e,
            "var t=new Uint8Array(an.fftSize);an.getByteTimeDomainData(t);\
             var lo=255,hi=0;for(var i=0;i<t.length;i++){if(t[i]<lo)lo=t[i];if(t[i]>hi)hi=t[i];}\
             lo+'/'+hi",
        ),
        "0/255"
    );
    // `resume` → state running.
    assert_eq!(ev(&mut e, "ctx.resume();ctx.state"), "running");
}

#[test]
fn analyser_zeros_on_unclaimed() {
    let (mut e, _dom) = engine();
    // Aucun claim, aucun ring → zéros honnêtes partout.
    assert_eq!(
        ev(
            &mut e,
            "var ctx=new AudioContext();\
             var an=ctx.createAnalyser();\
             var src=ctx.createMediaElementSource(document.getElementById('v'));\
             src.connect(an);\
             var u=new Uint8Array(an.frequencyBinCount);\
             an.getByteFrequencyData(u);\
             var mx=0;for(var i=0;i<u.length;i++)if(u[i]>mx)mx=u[i];mx",
        ),
        "0"
    );
    assert_eq!(
        ev(
            &mut e,
            "var t=new Uint8Array(64);an.getByteTimeDomainData(t);t[10]",
        ),
        "128"
    );
    // Non-élément → même repli honnête (pas d'exception).
    assert_eq!(
        ev(
            &mut e,
            "var s2=ctx.createMediaElementSource({});typeof s2.connect==='function'",
        ),
        "true"
    );
}
