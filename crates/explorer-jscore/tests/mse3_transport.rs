// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! MSE-3 — transport segments (ADR-0029) : `appendBuffer`
//! stocke en file FIFO bornée ; `Engine::mse_pull_segments`
//! draine au plus `max` octets en vrac (`(octets, curseur,
//! eos)`) pour le `MseSource` `ByteSource` hôte ; `eos`
//! quand `endOfStream` est appelé **et** la file vide ;
//! `abort` vide la file ; `remove` n'évince que les chunks
//! dont la plage est entièrement couverte (JS-C287) ;
//! `?buffer=N`
//! indexe le `addSourceBuffer`.

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

/// Hexa concaténé (fixtures fMP4 littérales multi-lignes).
fn hex(s: &str) -> Vec<u8> {
    let s: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    (0..s.len() / 2)
        .map(|i| u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).expect("hex"))
        .collect()
}

/// Crée une `MediaSource`, attache-la au `<video>` et rend
/// l'URL `blob:` (chemin player réel).
fn attached_url(e: &mut Engine) -> String {
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
fn mse3_pull_returns_appended_bytes_fifo() {
    let mut e = engine();
    let u = attached_url(&mut e);
    ev(
        &mut e,
        "var sb=__ms.addSourceBuffer('video/mp4');\
         sb.appendBuffer(new Uint8Array([1,2,3]));\
         sb.appendBuffer(new Uint8Array([4,5]));",
    );
    // Le pull draine la file dans l'ordre d'append, borné
    // par `max`.
    let (data, cursor, eos, _sec) = e.mse_pull_segments(&u, 0, 4).expect("pull");
    assert_eq!(data, vec![1, 2, 3, 4]);
    assert_eq!(cursor, 4);
    assert!(!eos);
    let (data, cursor, eos, _sec) = e.mse_pull_segments(&u, 0, 16).expect("pull2");
    assert_eq!(data, vec![5]);
    assert_eq!(cursor, 5);
    assert!(!eos);
    // File vide, readyState "open" → pas d'eos (le site peut
    // encore appender — le ByteSource attend).
    let (data, _, eos, _s) = e.mse_pull_segments(&u, 0, 16).expect("pull3");
    assert_eq!(data, Vec::<u8>::new());
    assert!(!eos);
}

#[test]
fn mse3_end_of_stream_sets_eos_when_drained() {
    let mut e = engine();
    let u = attached_url(&mut e);
    ev(
        &mut e,
        "var sb=__ms.addSourceBuffer('video/mp4');\
         sb.appendBuffer(new Uint8Array([9,9]));\
         __ms.endOfStream();",
    );
    // `ended` mais file non vide → octets d'abord, eos au
    // vidage seulement.
    let (data, _, eos, _s) = e.mse_pull_segments(&u, 0, 16).expect("pull");
    assert_eq!(data, vec![9, 9]);
    assert!(eos);
}

#[test]
fn mse3_abort_and_remove_drop_queued_bytes() {
    let mut e = engine();
    let u = attached_url(&mut e);
    ev(
        &mut e,
        "var sb=__ms.addSourceBuffer('video/mp4');\
         sb.appendBuffer(new Uint8Array([1,1]));\
         sb.remove(0,1);",
    );
    // JS-C287 — le chunk estimé couvre `[0,6)` : `[0,1)` ne
    // le couvre pas entièrement → conservé (on ne découpe
    // pas l'intérieur d'un fragment sans démuxage).
    let (data, _, _, _s) = e.mse_pull_segments(&u, 0, 16).expect("pull");
    assert_eq!(data, vec![1, 1], "remove partiel conserve le chunk");
    ev(
        &mut e,
        "sb.appendBuffer(new Uint8Array([2,2]));\
         sb.abort();",
    );
    let (data, _, _, _s) = e.mse_pull_segments(&u, 0, 16).expect("pull2");
    assert_eq!(data, Vec::<u8>::new(), "abort aurait dû vider la file");
}

/// JS-C287 — `remove` n'évince que les chunks entièrement
/// couverts : init (sans plage) + seg1 `[1,2)` conservés,
/// seg0 `[0,1)` jeté. Les octets pullés reflètent l'éviction.
#[test]
fn mse3_remove_evicts_only_covered_chunks() {
    // moov[trak[mdia[mdhd v0 ts=1000]]] + deux moof tfdt
    // 0/1000 durée 1 s (fixtures JS-C285).
    const INIT: &str = "000000386d6f6f76\
                        000000307472616b\
                        000000286d646961\
                        000000206d646864\
                        000000000000000000000000000003e80000000000000000";
    const SEG0: &str = "000000506d6f6f66\
                        0000004874726166\
                        00000014746668640000000800000001000001f4\
                        0000001474666474010000000000000000000000\
                        000000187472756e0000010000000002000001f4000001f4\
                        0000000c6d646174deadbeef";
    const SEG1: &str = "000000506d6f6f66\
                        0000004874726166\
                        00000014746668640000000800000001000001f4\
                        00000014746664740100000000000000000003e8\
                        000000187472756e0000010000000002000001f4000001f4\
                        0000000c6d646174deadbeef";
    let mut e = engine();
    let u = attached_url(&mut e);
    e.eval(&format!(
        "function H(s){{var a=new Uint8Array(s.length/2);\
         for(var i=0;i<a.length;i++)a[i]=parseInt(s.substr(i*2,2),16);\
         return a}}\
         var sb=__ms.addSourceBuffer('video/mp4');\
         sb.appendBuffer(H('{INIT}').buffer);\
         sb.appendBuffer(H('{SEG0}').buffer);\
         sb.appendBuffer(H('{SEG1}').buffer);\
         sb.remove(0,1);"
    ))
    .expect("appends+remove");
    let (data, _, _, _s) = e.mse_pull_segments(&u, 0, 4096).expect("pull");
    let init = hex(INIT);
    let seg0 = hex(SEG0);
    let seg1 = hex(SEG1);
    // Init + seg1 survivent, seg0 est évincé — ordre FIFO
    // préservé.
    let mut want = init.clone();
    want.extend_from_slice(&seg1);
    assert_eq!(data, want, "remove(0,1) doit jeter seg0 seul");
    assert!(!data.windows(seg0.len()).any(|w| w == &seg0[..]));
}

/// JS-C295 — pont C : un `moov` re-servi émet
/// `X-Mse-Section` à l'offset exact du nouvel init dans le
/// corps ; le premier init ne marque rien (pas de frontière
/// en tête de flux) ; deux re-inits dans un bulk = deux
/// offsets ordonnés.
#[test]
fn mse3_section_offsets_mark_reinit() {
    const INIT: &str = "000000386d6f6f76\
                        000000307472616b\
                        000000286d646961\
                        000000206d646864\
                        000000000000000000000000000003e80000000000000000";
    const SEG: &str = "000000506d6f6f66\
                        0000004874726166\
                        00000014746668640000000800000001000001f4\
                        0000001474666474010000000000000000000000\
                        000000187472756e0000010000000002000001f4000001f4\
                        0000000c6d646174deadbeef";
    let mut e = engine();
    let u = attached_url(&mut e);
    e.eval(&format!(
        "function H(s){{var a=new Uint8Array(s.length/2);\
         for(var i=0;i<a.length;i++)a[i]=parseInt(s.substr(i*2,2),16);\
         return a}}\
         var sb=__ms.addSourceBuffer('video/mp4');\
         sb.appendBuffer(H('{INIT}').buffer);\
         sb.appendBuffer(H('{SEG}').buffer);\
         sb.appendBuffer(H('{INIT}').buffer);\
         sb.appendBuffer(H('{SEG}').buffer);\
         sb.appendBuffer(H('{INIT}').buffer);"
    ))
    .expect("appends");
    let il = hex(INIT).len() as u64;
    let sl = hex(SEG).len() as u64;
    // Bulk unique : sections aux positions des deux re-inits.
    let (data, _, _, sections) = e.mse_pull_segments(&u, 0, 65536).expect("pull");
    assert_eq!(sections, vec![il + sl, 2 * (il + sl)]);
    assert_eq!(data.len() as u64, 3 * il + 2 * sl);
    // `abort` vide la file avant tout pull : l'init jeté
    // n'a jamais été servi → le re-init suivant est le
    // PREMIER vu de l'hôte → aucune frontière.
    ev(&mut e, "var sb2=__ms.addSourceBuffer('audio/mp4');");
    e.eval(&format!(
        "sb2.appendBuffer(H('{INIT}').buffer);\
         sb2.appendBuffer(H('{INIT}').buffer);\
         sb2.abort();\
         sb2.appendBuffer(H('{INIT}').buffer);"
    ))
    .expect("rail2");
    let (_, _, _, sections2) = e.mse_pull_segments(&u, 1, 65536).expect("pull2");
    assert_eq!(sections2, Vec::<u64>::new());
}

#[test]
fn mse3_buffer_index_selects_source_buffer() {
    let mut e = engine();
    let u = attached_url(&mut e);
    // Cas A+V : deux `SourceBuffer` = deux files
    // indépendantes adressées par `?buffer=N`.
    ev(
        &mut e,
        "var v=__ms.addSourceBuffer('video/mp4');\
         var a=__ms.addSourceBuffer('audio/mp4');\
         v.appendBuffer(new Uint8Array([1]));\
         a.appendBuffer(new Uint8Array([2]));",
    );
    let (dv, _, _, _sv) = e.mse_pull_segments(&u, 0, 16).expect("video");
    let (da, _, _, _sa) = e.mse_pull_segments(&u, 1, 16).expect("audio");
    assert_eq!(dv, vec![1]);
    assert_eq!(da, vec![2]);
    // Index hors rang → None honnête.
    assert!(e.mse_pull_segments(&u, 2, 16).is_none());
}

#[test]
fn mse3_unknown_url_pulls_none() {
    let mut e = engine();
    // URL non enregistrée → `None` (le handler rendra 410).
    assert!(e.mse_pull_segments("blob:null/999", 0, 16).is_none());
    // `blob:` d'un Blob ordinaire → None (pas une MediaSource).
    let u = e
        .eval("URL.createObjectURL(new Blob(['x']))")
        .map(|v| e.describe(v))
        .expect("blob url");
    assert!(e.mse_pull_segments(&u, 0, 16).is_none());
}

#[test]
fn mse4_is_type_supported_reflects_declared_codecs() {
    let mut e = engine();
    // Table vide → `""` (état C173 honnête préservé).
    assert_eq!(ev(&mut e, "MediaSource.isTypeSupported('video/mp4')"), "");
    // L'hôte déclare ses codecs au claim → la table MSE
    // s'alimente des préfixes normalisés (avc1/mp4a).
    e.set_media_claim(
        explorer_jscore::NodeId::from(0u32),
        1,
        Some(vec!["avc1.42E01E".to_string(), "mp4a.40.2".to_string()]),
    );
    assert_eq!(
        ev(
            &mut e,
            "MediaSource.isTypeSupported('video/mp4; codecs=\"avc1.42E01E, mp4a.40.2\"')"
        ),
        "maybe"
    );
    assert_eq!(
        ev(
            &mut e,
            "MediaSource.isTypeSupported('video/mp4; codecs=\"hvc1\"')"
        ),
        ""
    );
    assert_eq!(
        ev(&mut e, "MediaSource.isTypeSupported('video/mp4')"),
        "maybe"
    );
    assert_eq!(
        ev(&mut e, "MediaSource.isTypeSupported('video/quicktime')"),
        ""
    );
    // Claim déclaré en MIME complet (`video/mp4;codecs=…`) —
    // le cas réel du contrat `claim` : la table doit extraire
    // la liste `codecs=` (vol 177 : le MIME entier devenait un
    // « préfixe » jamais rejoint → `""` malgré le claim).
    let mut e2 = engine();
    e2.set_media_claim(
        explorer_jscore::NodeId::from(0u32),
        2,
        Some(vec![
            "video/mp4;codecs=avc1.42E01E".to_string(),
            "audio/mp4;codecs=mp4a.40.2".to_string(),
        ]),
    );
    assert_eq!(
        ev(
            &mut e2,
            "MediaSource.isTypeSupported('video/mp4; codecs=\"avc1.42E01E\"')"
        ),
        "maybe"
    );
    assert_eq!(
        ev(
            &mut e2,
            "MediaSource.isTypeSupported('video/mp4; codecs=\"avc1.42E01E,mp4a.40.2\"')"
        ),
        "maybe"
    );
    // Conteneur nu dans le claim → rien de déclaré en plus.
    let mut e3 = engine();
    e3.set_media_claim(
        explorer_jscore::NodeId::from(0u32),
        3,
        Some(vec!["video/mp4".to_string()]),
    );
    assert_eq!(
        ev(
            &mut e3,
            "MediaSource.isTypeSupported('video/mp4; codecs=\"avc1\"')"
        ),
        ""
    );
}

#[test]
fn mse4_source_buffers_enumerated_for_offer() {
    let mut e = engine();
    let u = attached_url(&mut e);
    assert_eq!(e.mse_source_buffers(&u), Vec::<String>::new());
    ev(
        &mut e,
        "__ms.addSourceBuffer('video/mp4');\
         __ms.addSourceBuffer('audio/mp4');",
    );
    assert_eq!(
        e.mse_source_buffers(&u),
        vec!["video/mp4".to_string(), "audio/mp4".to_string()]
    );
    assert_eq!(e.mse_source_buffers("blob:null/x"), Vec::<String>::new());
}
