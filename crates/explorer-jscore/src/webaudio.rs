// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Web Audio minimal honnête (Étape 204c — ADR-0032) : le
//! `AnalyserNode` lit le ring PCM hôte (`MseState::pcm_rings`
//! alimenté par `AUDIO_MODE_TAP`, étape 204b) — analyse
//! fréquence/temps réels sur la surface claimée. Graphe
//! borné `MediaElementAudioSourceNode → [AnalyserNode] →
//! destination` ; la destination reste **muette documentée**
//! (le son audible reste le device de la DLL média).
//!
//! Déviations assumées : `frequencyBinCount` ne se réécrit pas
//! à l'assignation `fftSize` (prop donnée — les getters lisent
//! `fftSize` vivant et restent cohérents) ; `createGain`/
//! worklets/`OfflineAudioContext` absents = `undefined`
//! honnête, pas de stubs mensongers.

use crate::dom_host::{arg, audio_noop, dom_node_of};
use crate::heap::JsResult;
use crate::value::Value;
use crate::Engine;

/// `ctx.createMediaElementSource(el)` — nœud source lié à la
/// surface claimée de `el` (`dom.media[node].media`). Élément
/// non claimé ou non-DOM → `__media = -1` : l'analyse rend des
/// zéros honnêtes (jamais d'exception, jamais de données
/// fabriquées).
pub(crate) fn create_media_element_source(
    e: &mut Engine,
    _t: Value,
    a: &[Value],
) -> JsResult<Value> {
    let mut mid: i64 = -1;
    if let Some((dom, node)) = dom_node_of(e, arg(a, 0)) {
        if let Some(st) = dom.borrow().media.get(&node) {
            mid = st.media as i64;
        }
    }
    Ok(Value::Obj(audio_source_node(e, mid)))
}

/// Objet nœud source — `__media` porte l'id de surface ;
/// `connect` propage le lien (graphe minimal).
fn audio_source_node(e: &mut Engine, mid: i64) -> crate::value::ObjId {
    let o = e.new_object();
    e.set_own(o, "__media", Value::Num(mid as f64), true);
    let cn = e.native_fn("connect", audio_graph_connect);
    e.set_own(o, "connect", cn, true);
    for m in ["disconnect", "start", "stop"] {
        let f = e.native_fn(m, audio_noop);
        e.set_own(o, m, f, true);
    }
    o
}

/// `ctx.createAnalyser()` — nœud d'analyse réel : props spec
/// (`fftSize` 2048, `minDecibels` −100, `maxDecibels` −30,
/// `smoothingTimeConstant` 0.8), getters byte/float lisant la
/// fenêtre PCM la plus récente du nœud source amont.
pub(crate) fn create_analyser(e: &mut Engine, _t: Value, _a: &[Value]) -> JsResult<Value> {
    let o = e.new_object();
    e.set_own(o, "fftSize", Value::Num(2048.0), true);
    e.set_own(o, "frequencyBinCount", Value::Num(1024.0), true);
    e.set_own(o, "smoothingTimeConstant", Value::Num(0.8), true);
    e.set_own(o, "minDecibels", Value::Num(-100.0), true);
    e.set_own(o, "maxDecibels", Value::Num(-30.0), true);
    e.set_own(o, "__src_media", Value::Num(-1.0), true);
    let cn = e.native_fn("connect", audio_graph_connect);
    e.set_own(o, "connect", cn, true);
    for (name, f) in [
        (
            "getByteFrequencyData",
            analyser_byte_freq as crate::heap::NativeFn,
        ),
        (
            "getFloatFrequencyData",
            analyser_float_freq as crate::heap::NativeFn,
        ),
        (
            "getByteTimeDomainData",
            analyser_byte_td as crate::heap::NativeFn,
        ),
        ("disconnect", audio_noop as crate::heap::NativeFn),
        ("start", audio_noop as crate::heap::NativeFn),
        ("stop", audio_noop as crate::heap::NativeFn),
    ] {
        let m = e.native_fn(name, f);
        e.set_own(o, name, m, true);
    }
    Ok(Value::Obj(o))
}

/// `src.connect(dst)` — propage `__media` de la source vers
/// `__src_media` de la cible (le lien que l'analyse résout) ;
/// rend `dst` (chaînage spec).
fn audio_graph_connect(e: &mut Engine, this: Value, a: &[Value]) -> JsResult<Value> {
    let dst = arg(a, 0);
    if let (Value::Obj(sid), Value::Obj(did)) = (this, dst.clone()) {
        let mid = e.get_prop(sid, "__media").unwrap_or(Value::Num(-1.0));
        e.set_own(did, "__src_media", mid, true);
    }
    Ok(dst)
}

/// `fftSize` vivant du nœud — borné aux puissances de deux
/// spec (32…32768), repli 2048 sur valeur invalide.
fn analyser_fft_size(e: &mut Engine, this: Value) -> usize {
    let n = match this {
        Value::Obj(id) => match e.get_prop(id, "fftSize") {
            Some(Value::Num(v)) => v as usize,
            _ => 2048,
        },
        _ => 2048,
    };
    n.clamp(32, 32768).next_power_of_two()
}

/// `n` derniers échantillons **mono** f32 du ring de la
/// surface liée (`__src_media`) : f32 LE interleaved → mix
/// `Σ/ch` ; ring absent/spec non-f32 → zéros honnêtes.
fn analyser_samples(e: &mut Engine, this: Value, n: usize) -> Vec<f32> {
    let Value::Obj(id) = this else {
        return vec![0.0; n];
    };
    let mid = match e.get_prop(id, "__src_media") {
        Some(Value::Num(m)) if m >= 0.0 => m as u64,
        _ => return vec![0.0; n],
    };
    let Some(ring) = e.mse.pcm_rings.get(&mid) else {
        return vec![0.0; n];
    };
    let (_rate, ch, fmt) = ring.spec;
    if fmt != 4 || ch == 0 {
        return vec![0.0; n];
    }
    let ch = ch as usize;
    let bytes = ring.read_latest(n * ch * 4);
    let mut out = Vec::with_capacity(n);
    for frame in bytes.chunks_exact(ch * 4) {
        let mut acc = 0.0f32;
        for c in 0..ch {
            let s = &frame[c * 4..c * 4 + 4];
            acc += f32::from_le_bytes(s.try_into().unwrap_or([0u8; 4]));
        }
        out.push(acc / ch as f32);
    }
    out.resize(n, 0.0);
    out
}

/// `analyser.getByteTimeDomainData(u8)` — forme d'onde
/// **non fenêtrée** des `fftSize` derniers échantillons
/// (spec : `128 + 128·x` borné).
fn analyser_byte_td(e: &mut Engine, this: Value, a: &[Value]) -> JsResult<Value> {
    let fft = analyser_fft_size(e, this.clone());
    let samples = analyser_samples(e, this, fft);
    crate::typed_array::ta_write_region(e, arg(a, 0), move |region| {
        for (i, b) in region.iter_mut().enumerate() {
            let s = samples.get(i).copied().unwrap_or(0.0);
            *b = (128.0 + s * 128.0).clamp(0.0, 255.0) as u8;
        }
    })
}

/// `analyser.getByteFrequencyData(u8)` — magnitudes dB
/// recalées dans [`minDecibels`,`maxDecibels`] → `[0,255]`
/// (spec `AnalyserNode`).
fn analyser_byte_freq(e: &mut Engine, this: Value, a: &[Value]) -> JsResult<Value> {
    let (mags, lo, hi) = analyser_db_row(e, this);
    crate::typed_array::ta_write_region(e, arg(a, 0), move |region| {
        for (i, b) in region.iter_mut().enumerate() {
            let db = mags.get(i).copied().unwrap_or(f64::NEG_INFINITY);
            let v = if db.is_finite() {
                255.0 * (db - lo) / (hi - lo)
            } else {
                0.0
            };
            *b = v.clamp(0.0, 255.0) as u8;
        }
    })
}

/// `analyser.getFloatFrequencyData(f32)` — magnitudes dB
/// brutes (`-Infinity` hors signal — spec).
fn analyser_float_freq(e: &mut Engine, this: Value, a: &[Value]) -> JsResult<Value> {
    let (mags, _lo, _hi) = analyser_db_row(e, this);
    crate::typed_array::ta_write_region(e, arg(a, 0), move |region| {
        for (i, slot) in region.chunks_exact_mut(4).enumerate() {
            let db = mags.get(i).copied().unwrap_or(f64::NEG_INFINITY) as f32;
            slot.copy_from_slice(&db.to_le_bytes());
        }
    })
}

/// Magnitudes dB `fftSize/2` du signal courant + bornes
/// `minDecibels`/`maxDecibels` lues sur le nœud.
fn analyser_db_row(e: &mut Engine, this: Value) -> (Vec<f64>, f64, f64) {
    let fft = analyser_fft_size(e, this.clone());
    let samples = analyser_samples(e, this.clone(), fft);
    let mags = analyser_mags(&samples);
    let (mut lo, mut hi) = (-100.0, -30.0);
    if let Value::Obj(id) = this {
        if let Some(Value::Num(v)) = e.get_prop(id, "minDecibels") {
            lo = v;
        }
        if let Some(Value::Num(v)) = e.get_prop(id, "maxDecibels") {
            hi = v;
        }
    }
    (mags, lo, hi.max(lo + 1e-6))
}

/// Magnitudes dB des `n/2` bins — fenêtre Blackman (spec
/// `AnalyserNode`) + FFT radix-2 ; référence `max(1e-20)` pour
/// éviter `log(0)`.
fn analyser_mags(mono: &[f32]) -> Vec<f64> {
    let n = mono.len();
    debug_assert!(n.is_power_of_two());
    let n_f = n as f32;
    let mut re: Vec<f32> = mono
        .iter()
        .enumerate()
        .map(|(i, &s)| {
            let t = i as f32 / n_f;
            // Blackman spec : a0=0.42, a1=0.5, a2=0.08.
            s * (0.42 - 0.5 * (std::f32::consts::TAU * t).cos()
                + 0.08 * (2.0 * std::f32::consts::TAU * t).cos())
        })
        .collect();
    let mut im = vec![0.0f32; n];
    fft_radix2(&mut re, &mut im);
    re.iter()
        .zip(&im)
        .take(n / 2)
        .map(|(r, i)| 20.0 * ((r * r + i * i).sqrt() as f64).max(1e-20).log10())
        .collect()
}

/// FFT radix-2 itérative in-place — `re`/`im` de longueur
/// `n` puissance de deux (`fftSize` spec l'exige déjà ;
/// pas de crate DSP pour ~40 lignes déterministes).
fn fft_radix2(re: &mut [f32], im: &mut [f32]) {
    let n = re.len();
    let mut j = 0usize;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let ang = -std::f32::consts::TAU / len as f32;
        let (wr, wi) = (ang.cos(), ang.sin());
        for base in (0..n).step_by(len) {
            let (mut cr, mut ci) = (1.0f32, 0.0f32);
            for k in 0..len / 2 {
                let (ur, ui) = (re[base + k], im[base + k]);
                let (xr, xi) = (re[base + k + len / 2], im[base + k + len / 2]);
                let (vr, vi) = (xr * cr - xi * ci, xr * ci + xi * cr);
                re[base + k] = ur + vr;
                im[base + k] = ui + vi;
                re[base + k + len / 2] = ur - vr;
                im[base + k + len / 2] = ui - vi;
                let nr = cr * wr - ci * wi;
                ci = cr * wi + ci * wr;
                cr = nr;
            }
        }
        len <<= 1;
    }
}
