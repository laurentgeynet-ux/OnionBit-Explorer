// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! §B-72 — bras fifa vol 504, repro in-process bornée.
//!
//! Fixtures hors repo (jamais commitées, ~7,6 Mo) :
//! `FIFA504_DIR` ou `%TEMP%/fifa504/` — `davinci.js`,
//! `akamai.js`, `main.js`, `smt-bridge.js`, `shell.html`.
//! Fixture absente → skip silencieux.
//!
//! État : bras 1 (davinci `AssertionError`) RÉSOLU par
//! `Object(primitive)` → `ToObject` ; bras 2 (Akamai
//! `` `W` is not a function ``) RÉSOLU par `fn.toString()`
//! verbatim (JS-C264) — la VM décode ses clés de dispatch
//! depuis son propre source ; le stub `{ [code] }` divergeait
//! → slots `W` jamais peuplés. Bras 3 (driver du reload en
//! boucle) reste un établi `#[ignore]`.

use explorer_jscore::{Dom, Engine};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

fn fixture(name: &str) -> Option<String> {
    let dir = std::env::var("FIFA504_DIR")
        .unwrap_or_else(|_| "C:/Users/Lou/AppData/Local/Temp/fifa504".into());
    std::fs::read_to_string(format!("{dir}/{name}")).ok()
}

fn engine_with_dom(html: &str) -> Engine {
    let mut e = Engine::new();
    e.install_dom(&Rc::new(RefCell::new(Dom::from_html(html))));
    e.install_fetch();
    e.install_timers();
    e.install_storage(&Rc::new(RefCell::new(BTreeMap::new())));
    e
}

/// §B-72 bras 1 RÉSOLU — le bundle davinci brut (5,96 Mo)
/// évalue à terme : avant le fix `Object(t)` ne boxait pas la
/// primitive → `Object('NA').constructor === Object` →
/// `goog.proto2.Message.checkFieldType_` assertait
/// (`AssertionError` tuait le boot `PingOne`, vol 504).
/// ~40 s d'éval — fixture-gated.
#[test]
fn davinci_bundle_raw_evals_clean() {
    let Some(src) = fixture("davinci.js") else {
        eprintln!("fixture absente — skip");
        return;
    };
    let mut e = engine_with_dom("<html><body></body></html>");
    let wrapped = format!("try {{\n{src}\n}} catch (__e) {{ throw 'EXC:' + String(__e && __e.name) + '|' + String(__e && __e.message) }}");
    match e.eval_named(&wrapped, "davinci.js") {
        Ok(v) => eprintln!("EVAL OK -> {}", e.describe(v)),
        Err(err) => panic!("EVAL ERR -> {err}"),
    }
}

/// §B-72 bras 2 RÉSOLU — Akamai `_abck` brut (609 Ko, VM
/// obfusquée) évalue à terme sans exception, sur un DOM doté
/// d'une balise `<script src>` (le sensor retrouve sa propre
/// balise pour dériver l'URL CDN).
///
/// Mécanisme racine (bissection Node parallèle) : la VM lit
/// `Function.prototype.toString()` sur ses propres fonctions —
/// le source verbatim est son codebook de décodage ; le stub
/// `{ [code] }` produisait des clés VM divergentes → le slot
/// `W` des tables de handlers n'était jamais peuplé →
/// `` `W` is not a function `` (JS-C264). ~40 s d'éval —
/// fixture-gated.
#[test]
fn akamai_bundle_evals_clean() {
    let Some(src) = fixture("akamai.js") else {
        eprintln!("fixture absente — skip");
        return;
    };
    // Le bundle cherche sa propre balise `<script src>` (URL CDN
    // dérivée) — un DOM nu sans script casse le chemin :
    // `…scripts[i].src` sur `undefined`.
    let mut e = engine_with_dom(
        "<html><head><script src=\"https://ak.fifa.com/ak.js\"></script></head><body></body></html>",
    );
    // Bundle brut + pièce `<script src>` — aucun piège : le
    // décodage VM produit les clés attendues depuis son propre
    // source (toString verbatim, JS-C264) → les slots `W` sont
    // peuplés en propre (plus de proto-trap nécessaire).
    let wrapped = format!(
        "var __w=[];\
         try {{\n{src}\n}} catch (__e) {{ \
            __w.push('EXC:' + String(__e && __e.name) + '|' + String(__e && __e.message) + '|STACK:' + String(__e && __e.stack).split('\\n').slice(0,8).join(';')) \
        }}; __w.slice(0,12).join('|') + ' :: exc=' + __w.length"
    );
    match e.eval_named(&wrapped, "akamai.js") {
        Ok(v) => eprintln!("EVAL OK -> {}", e.describe(v)),
        Err(err) => eprintln!("EVAL ERR -> {err}"),
    }
}

/// Transport XHR muet — `send()` répond un handle, jamais de
/// réponse : les bundles passent le `new XMLHttpRequest` et le
/// reste du fichier s'évalue (otSDKStub mourait à
/// `XMLHttpRequest is not defined` sans lui).
struct StubXhr;
impl explorer_jscore::XhrTransport for StubXhr {
    fn send(&self, _req: &explorer_jscore::XhrRequest) -> u64 {
        1
    }
}

/// §B-72 bras 3 — driver du reload : les hooks nav **moteur**
/// (`Navigate`/`Replace`/`Reload`/`open`/`history`/`form`)
/// capturent toute intention, quelle que soit la surface JS —
/// le marqueur `defineProperty` JS ne couvrait que
/// `location.*` directs et rendait NAV vide alors que la nav
/// live boucle.
#[test]
#[ignore = "établi de bissection §B-72 bras 3 — pas un verdict"]
fn fifa_shell_reload_driver() {
    let Some(html) = fixture("shell.html") else {
        eprintln!("fixture absente — skip");
        return;
    };
    let mut e = engine_with_dom(&html);
    e.install_xhr(std::rc::Rc::new(StubXhr));
    let navs = std::rc::Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
    {
        let n = navs.clone();
        let n2 = navs.clone();
        e.set_nav_hooks(
            std::rc::Rc::new(move |u| {
                n.borrow_mut().push(format!("navigate:{u}"));
            }),
            std::rc::Rc::new(move |d| {
                n2.borrow_mut().push(format!("history:{d}"));
            }),
            1,
        );
        let n3 = navs.clone();
        let n4 = navs.clone();
        e.set_nav_replace_reload_hooks(
            std::rc::Rc::new(move |u| {
                n3.borrow_mut().push(format!("replace:{u}"));
            }),
            std::rc::Rc::new(move || {
                n4.borrow_mut().push("reload".to_string());
            }),
        );
        let n5 = navs.clone();
        e.set_open_hook(std::rc::Rc::new(move |u, name| {
            n5.borrow_mut().push(format!("open:{u}|{name}"));
        }));
        let n6 = navs.clone();
        e.set_push_state_hook(std::rc::Rc::new(move |u, replace, _st| {
            n6.borrow_mut().push(format!("push:{u}|replace:{replace}"));
        }));
    }
    // Ordre documentaire du shell : OtAutoBlock (7,9 Mo —
    // stack OneTrust complète) → otSDKStub → davinci → inline
    // fp.env → smt-bridge → main → akamai `_abck`.
    for f in [
        "otAutoBlock.js",
        "otSDKStub.js",
        "davinci.js",
        "inline1.js",
        "smt-bridge.js",
        "main.js",
        "akamai.js",
    ] {
        if let Some(s) = fixture(f) {
            let w = format!("try {{\n{s}\n}} catch (__e) {{ 'EXC:{f}:' + String(__e && __e.name) + '|' + String(__e && __e.message) }}");
            match e.eval_named(&w, f) {
                Ok(v) => eprintln!("{f} OK -> {}", e.describe(v)),
                Err(err) => eprintln!("{f} ERR -> {err}"),
            }
            let seen = navs.borrow().join(" ; ");
            if !seen.is_empty() {
                eprintln!("  NAV après {f}: {seen}");
            }
        }
    }
    // Drain : les timers planifiés tirent au prochain eval.
    for _ in 0..8 {
        let _ = e.eval("0");
    }
    eprintln!("NAV final: {}", navs.borrow().join(" ; "));
    if let Ok(v) = e.eval(
        "[window.location.pathname, navigator.languages && navigator.languages[0], localStorage.getItem('fpLocale')].join('|')",
    ) {
        eprintln!("env nav: {}", e.describe(v));
    }

    // Contraposé : sous `page_url=/en`, la gate locale trouve
    // `en` dans le path → `changeLocale=false` → plus de nav.
    // Si NAV reste vide ici, la boucle live est l'effet « shell
    // reservi avec pathname sans locale » (corpus replay 1 URL
    // ou resync d'URL), pas un driver interne additionnel.
    navs.borrow_mut().clear();
    let mut e2 = engine_with_dom(&html);
    e2.set_page_url("https://www.fifa.com/en");
    {
        let n = navs.clone();
        e2.set_nav_hooks(
            std::rc::Rc::new(move |u| {
                n.borrow_mut().push(format!("navigate:{u}"));
            }),
            std::rc::Rc::new(|_| {}),
            1,
        );
    }
    if let Some(s) = fixture("main.js") {
        let w = format!("try {{\n{s}\n}} catch (__e) {{ 'EXC:main:' + String(__e && __e.name) + '|' + String(__e && __e.message) }}");
        match e2.eval_named(&w, "main.js") {
            Ok(v) => eprintln!("main@/en OK -> {}", e2.describe(v)),
            Err(err) => eprintln!("main@/en ERR -> {err}"),
        }
    }
    for _ in 0..8 {
        let _ = e2.eval("0");
    }
    eprintln!("NAV@/en: {}", navs.borrow().join(" ; "));
    let Ok(v) = e2.eval("window.location.pathname") else {
        return;
    };
    eprintln!("pathname@/en: {}", e2.describe(v));
}
