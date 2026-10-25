// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Régressions JS-C98 — complétude de surface (census) : les
//! globaux web mesurés absents sont posés (stubs de détection,
//! ctors réels bornés, props window/navigator, promesses
//! honnêtes). Un `ReferenceError` sur `typeof X` ne doit plus
//! jamais arrêter un bundle à la détection.

use explorer_jscore::{Dom, Engine};
use std::cell::RefCell;
use std::rc::Rc;

fn engine() -> Engine {
    let mut e = Engine::new();
    e.install_dom(&Rc::new(RefCell::new(Dom::from_html(
        "<html><body><div></div></body></html>",
    ))));
    e
}

fn ev(src: &str) -> String {
    let mut e = engine();
    match e.eval(src) {
        Ok(v) => e.describe(v),
        Err(err) => panic!("{src} -> {err}"),
    }
}

fn ev_seq(setup: &str, check: &str) -> String {
    let mut e = engine();
    e.eval(setup)
        .unwrap_or_else(|err| panic!("setup {setup} -> {err}"));
    match e.eval(check) {
        Ok(v) => e.describe(v),
        Err(err) => panic!("check {check} -> {err}"),
    }
}

/// Globaux ajoutés par C98 — `typeof` `function`/`object`
/// attendu (jamais `undefined`, jamais `ReferenceError`).
#[test]
fn c98_globals_present() {
    let missing: Vec<String> = [
        "HTMLSummaryElement",
        "SVGSVGElement",
        "Selection",
        "FileList",
        "MediaQueryList",
        "CustomElementRegistry",
        "MessagePort",
        "IntersectionObserverEntry",
        "ResizeObserverEntry",
        "DOMStringList",
        "CSSRuleList",
        "StyleSheetList",
        "FontFace",
        "FontFaceSet",
        "Animation",
        "KeyframeEffect",
        "DocumentTimeline",
        "PerformanceEntry",
        "PerformanceObserver",
        "ReportingObserver",
        "VideoFrame",
        "ImageBitmap",
        "AudioContext",
        "MediaStream",
        "MediaRecorder",
        "MediaSource",
        "RTCPeerConnection",
        "Geolocation",
        "PushManager",
        "ServiceWorker",
        "ServiceWorkerContainer",
        "ServiceWorkerRegistration",
        "BarcodeDetector",
        "EyeDropper",
        "CookieStore",
        "FileSystemHandle",
        "XPathEvaluator",
        "XPathResult",
        "XPathExpression",
        "ProgressEvent",
        "PageTransitionEvent",
        "BeforeUnloadEvent",
        "FormDataEvent",
        "MediaQueryListEvent",
        "NotificationEvent",
        "Option",
        "DOMPoint",
        "DOMMatrix",
        "ImageData",
        "Path2D",
        "OffscreenCanvas",
        "WebAssembly",
        "trustedTypes",
        "scheduler",
        "cookieStore",
        "visualViewport",
        "isSecureContext",
        "crossOriginIsolated",
        "origin",
        "opener",
        "frameElement",
        "external",
        "reportError",
        "stop",
        "print",
        "find",
        "createImageBitmap",
        "showOpenFilePicker",
        "showSaveFilePicker",
        "showDirectoryPicker",
        "mediaDevices_check",
    ]
    .iter()
    .filter_map(|name| {
        if *name == "mediaDevices_check" {
            return None;
        }
        let src = format!("typeof {name}");
        let t = ev(&src);
        (t == "undefined").then(|| format!("{name}={t}"))
    })
    .collect();
    assert!(missing.is_empty(), "globaux absents: {missing:?}");
}

/// Props `navigator` ajoutées — `mediaDevices`, `userActivation`,
/// `speechSynthesis`, `permissions`.
#[test]
fn c98_navigator_props() {
    assert_eq!(
        ev("[typeof navigator.mediaDevices, typeof navigator.userActivation, typeof speechSynthesis, typeof navigator.permissions].join(',')"),
        "object,object,object,object"
    );
    // `permissions.query` → `{state:'denied'}` résolu.
    assert_eq!(
        ev_seq(
            "var out='?'; navigator.permissions.query({name:'geolocation'}).then(function(r){out=r.state;});",
            "out"
        ),
        "denied"
    );
    // `speechSynthesis.getVoices()` → `[]`.
    assert_eq!(ev("speechSynthesis.getVoices().length"), "0");
    // `mediaDevices.getUserMedia` → rejet `NotFoundError`.
    assert_eq!(
        ev_seq(
            "var out='?'; navigator.mediaDevices.getUserMedia({video:true}).then(function(){out='ok';},function(e){out='E:'+e;});",
            "out"
        ),
        "E:NotFoundError: no capture device"
    );
    // `enumerateDevices` → `[]` résolu.
    assert_eq!(
        ev_seq(
            "var out='?'; navigator.mediaDevices.enumerateDevices().then(function(d){out='n:'+d.length;});",
            "out"
        ),
        "n:0"
    );
}

/// `new Option(text, value)` — `<option>` miroir avec
/// `label`/`value`/`selected`.
#[test]
fn c98_option_ctor() {
    assert_eq!(
        ev("var o=new Option('Libellé','42',false,true); [o.tagName,o.getAttribute('value'),o.getAttribute('selected')].join('|')"),
        "OPTION|42|"
    );
    assert_eq!(ev("typeof new Option().tagName"), "string");
}

/// `DOMMatrix` — math 2D réelle : `translate`/`scale`/
/// `multiply`/`invert`/`transformPoint`/`matrix(...)`/
/// `toString`/`isIdentity`.
#[test]
fn c98_dommatrix_math() {
    // Identité + translate.
    assert_eq!(
        ev("var m=new DOMMatrix(); [m.isIdentity,m.a,m.e].join(',')"),
        "true,1,0"
    );
    assert_eq!(
        ev("var m=new DOMMatrix().translate(10,20); [m.e,m.f].join(',')"),
        "10,20"
    );
    // scale + transformPoint : (2,3)*scale(2) = (4,6).
    assert_eq!(
        ev("var m=new DOMMatrix().scale(2); var p=m.transformPoint({x:2,y:3}); [p.x,p.y].join(',')"),
        "4,6"
    );
    // multiply : T(10,0) * T(0,5) → translate(10,5).
    assert_eq!(
        ev("var a=new DOMMatrix().translate(10,0),b=new DOMMatrix().translate(0,5),c=a.multiply(b); [c.e,c.f].join(',')"),
        "10,5"
    );
    // invertSelf : T(10,20)⁻¹ = T(-10,-20).
    assert_eq!(
        ev("var m=new DOMMatrix().translate(10,20); m.invertSelf(); [m.e,m.f].join(',')"),
        "-10,-20"
    );
    // Parse `matrix(1, 0, 0, 2, 10, 20)`.
    assert_eq!(
        ev("var m=new DOMMatrix('matrix(1, 0, 0, 2, 10, 20)'); [m.d,m.e,m.f].join(',')"),
        "2,10,20"
    );
    // toString round-trip.
    assert_eq!(
        ev("new DOMMatrix().translate(1,2).toString()"),
        "matrix(1, 0, 0, 1, 1, 2)"
    );
    // multiplySelf mute `this`.
    assert_eq!(
        ev("var m=new DOMMatrix(); m.multiplySelf({a:1,b:0,c:0,d:1,e:5,f:5}); m.e"),
        "5"
    );
    // DOMPoint + matrixTransform.
    assert_eq!(
        ev("var p=new DOMPoint(2,3).matrixTransform({a:2,b:0,c:0,d:2,e:1,f:1}); [p.x,p.y,p.w].join(',')"),
        "5,7,1"
    );
}

/// `ImageData` — `{width,height,data:Uint8ClampedArray}`
/// (deux signatures).
#[test]
fn c98_imagedata() {
    assert_eq!(
        ev("var d=new ImageData(2,3); [d.width,d.height,d.data.length,d.data.constructor.name].join(',')"),
        "2,3,24,Uint8ClampedArray"
    );
    assert_eq!(
        ev("var d=new ImageData(new Uint8ClampedArray(16),2); [d.width,d.height].join(',')"),
        "2,2"
    );
}

/// `Path2D` — enregistreur d'ops (méthodes lisibles, stockage).
#[test]
fn c98_path2d() {
    assert_eq!(
        ev("var p=new Path2D(); [typeof p.moveTo,typeof p.lineTo,typeof p.arc,typeof p.rect].join(',')"),
        "function,function,function,function"
    );
    // Pas de throw sur les appels.
    assert_eq!(
        ev("var p=new Path2D(); p.moveTo(0,0); p.lineTo(10,10); p.arc(5,5,3,0,6.28); p.rect(0,0,10,10); p.closePath(); 'ok'"),
        "ok"
    );
    // `new Path2D('M0 0L10 10')` — string SVG acceptée.
    assert_eq!(ev("typeof new Path2D('M0 0L10 10')"), "object");
}

/// `OffscreenCanvas` — `getContext('2d')` enregistre
/// (`Ctx2dHost` détaché), `transferToImageBitmap`.
#[test]
fn c98_offscreen_canvas() {
    assert_eq!(
        ev("var c=new OffscreenCanvas(100,50); [c.width,c.height,typeof c.getContext].join(',')"),
        "100,50,function"
    );
    // Le ctx enregistre (invoke-only comme l'élément canvas —
    // `typeof ctx.fillRect` n'est pas lisible, convention
    // existante) : appeler ne jette pas, `webgl` → null.
    assert_eq!(
        ev("var c=new OffscreenCanvas(10,10); var ctx=c.getContext('2d'); ctx.fillRect(0,0,1,1); ctx.beginPath(); (c.getContext('webgl')===null)"),
        "true"
    );
    assert_eq!(
        ev("var b=new OffscreenCanvas(8,6).transferToImageBitmap(); [b.width,b.height].join(',')"),
        "8,6"
    );
}

/// `FontFace` — `{family,status:'unloaded',load()→Promise
/// self}`.
#[test]
fn c98_fontface() {
    assert_eq!(
        ev("var f=new FontFace('MyFont'); [f.family,f.status,typeof f.load].join(',')"),
        "MyFont,unloaded,function"
    );
    assert_eq!(
        ev_seq(
            "var out='?'; new FontFace('X').load().then(function(f){out=f.family;});",
            "out"
        ),
        "X"
    );
}

/// Props window/`scheduler`/`cookieStore`/`trustedTypes`/
/// `WebAssembly`/`reportError` — surfaces honnêtes.
#[test]
fn c98_window_surface() {
    // Legacy + contexte.
    assert_eq!(
        ev("[window.status===window.status,opener===null,frameElement===null,typeof external,isSecureContext===true,crossOriginIsolated===false].join(',')"),
        "true,true,true,object,true,true"
    );
    // `scheduler.yield()` → promesse résolue.
    assert_eq!(
        ev_seq(
            "var out='?'; scheduler.yield().then(function(){out='y';});",
            "out"
        ),
        "y"
    );
    // `cookieStore` — jar vide honnête.
    assert_eq!(
        ev_seq(
            "var out='?'; cookieStore.get('x').then(function(v){out='g:'+(v===null);});",
            "out"
        ),
        "g:true"
    );
    // `WebAssembly` — validate→false, compile→rejet.
    assert_eq!(ev("WebAssembly.validate(new Uint8Array(4))"), "false");
    assert_eq!(
        ev_seq(
            "var out='?'; WebAssembly.instantiate(new Uint8Array(4)).then(function(){out='ok';},function(e){out='E';});",
            "out"
        ),
        "E"
    );
    // `trustedTypes.createPolicy` — passthrough.
    assert_eq!(
        ev("var p=trustedTypes.createPolicy('d',{}); p.createHTML('<b>x</b>')"),
        "<b>x</b>"
    );
    // `reportError` — dispatch `error` sur window (listener
    // appelé).
    assert_eq!(
        ev("var got='?'; addEventListener('error',function(e){got=e.type;}); reportError(new Error('x')); got"),
        "error"
    );
    // `createImageBitmap` → promesse `{width,height,close}`.
    assert_eq!(
        ev_seq(
            "var out='?'; createImageBitmap(null,0,0,10,20).then(function(b){out=b.width+'x'+b.height;});",
            "out"
        ),
        "10x20"
    );
    // `showOpenFilePicker` → rejet `SecurityError`.
    assert_eq!(
        ev_seq(
            "var out='?'; showOpenFilePicker().then(function(){out='ok';},function(e){out='E';});",
            "out"
        ),
        "E"
    );
    // `visualViewport` — métrique présente.
    assert_eq!(
        ev("[typeof visualViewport,typeof visualViewport.width,visualViewport.scale].join(',')"),
        "object,number,1"
    );
}

/// Régression mesurée github (vol 2026-11-10) —
/// `PerformanceObserver`/`ReportingObserver` sont légalement
/// constructibles : le stub « illegal ctor » tuait
/// `environment.js` (`new PerformanceObserver(cb)` throw où
/// la garde `typeof` sautait proprement avant C98).
#[test]
fn c98_observers_constructible() {
    // `new PerformanceObserver(cb)` — ne jette pas.
    assert_eq!(
        ev("var o=new PerformanceObserver(function(){}); [typeof o,typeof o.observe,typeof o.disconnect,typeof o.takeRecords].join(',')"),
        "object,function,function,function"
    );
    // `observe`/`disconnect`/`takeRecords` — bornés honnêtes.
    assert_eq!(
        ev("var o=new PerformanceObserver(function(){}); o.observe({entryTypes:['measure']}); o.takeRecords().length"),
        "0"
    );
    // `new ReportingObserver(cb)` — même forme.
    assert_eq!(
        ev("var o=new ReportingObserver(function(){}); [typeof o,o.takeRecords().length].join(',')"),
        "object,0"
    );
    // Ctor sans callback → TypeError (spec).
    assert_eq!(ev("try{new PerformanceObserver();}catch(e){'T'}"), "T");
    // `supportedEntryTypes` — `[]` honnête.
    assert_eq!(ev("PerformanceObserver.supportedEntryTypes.length"), "0");
    // `XPathEvaluator` — `new` + `evaluate` réel (CAP-188).
    assert_eq!(
        ev("var xe=new XPathEvaluator(); [typeof xe.evaluate, typeof xe.evaluate('//div',document)].join(',')"),
        "function,object"
    );
    // `MediaEncryptedEvent` — rail Event (init copié).
    assert_eq!(ev("new MediaEncryptedEvent('encrypted').type"), "encrypted");
}
