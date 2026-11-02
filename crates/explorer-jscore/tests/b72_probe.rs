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
//! `Object(primitive)` → `ToObject` (test actif ci-dessous) ;
//! bras 2 (Akamai `W is not a function`) et bras 3 (driver du
//! reload en boucle) restent OUVERTS — établis `#[ignore]`.

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

/// §B-72 bras 2 OUVERT — Akamai `_abck` (609 Ko, VM obfusquée).
/// Échec actuel : `` `W` is not a function `` sur une table
/// locale de ~20 handlers numériques `{3,7,13,18,30,62,…}`
/// (constructeurs + crypto définis dans des `case` vers
/// 2:437120/2:457xxx) — `'W'` n'est ni littéral, ni code de
/// `zpI()`, ni produit de `fromCharCode`/`charAt`/… : clé
/// calculée par la VM (décodage env ?). Receveur identifié
/// comme NON singleton (`Y4`/`KMI`/`BsI`/`TQ`/`ML`/`Oc`/`CO`
/// exclus par `===`). Établi d'instrumentation conservé pour
/// l'itération suivante — `cargo test -- --ignored`.
#[test]
#[ignore = "établi de bissection §B-72 bras 2 — pas un verdict"]
fn akamai_bundle_names_missing_receiver() {
    let Some(src) = fixture("akamai.js") else {
        eprintln!("fixture absente — skip");
        return;
    };
    let mut e = engine_with_dom("<html><body></body></html>");
    // UA réaliste testée (Chrome 126) — l'échec persiste :
    // l'UA seule n'est pas le signal manquant.
    let _ = e.eval(
        "try { Object.defineProperty(navigator, 'userAgent', { get: function() { return 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36'; }, configurable: true }); } catch(_e) {}",
    );
    // Trace des opcodes `Br` à l'entrée du dispatcher `GL` et
    // de chaque `case` exécuté — séquence observée : ~22× `DE`
    // puis `G` → crash. `Br=47` à l'entrée fautive.
    let needle = "var Rv=function GL(Br,vO){'use strict';var QO=GL;switch(Br){";
    let patched = "var Rv=function GL(Br,vO){'use strict';var QO=GL;var __sv=__inGL;__inGL=true;__diag.push('GL:'+String(Br));switch(Br){";
    let mut src = src.replacen(needle, patched, 1);
    let mut marked = String::with_capacity(src.len() + 4096);
    let mut rest = src.as_str();
    while let Some(i) = rest.find("case ") {
        marked.push_str(&rest[..i + 5]);
        let tail = &rest[i + 5..];
        let end = tail
            .find(|c: char| !(c.is_alphanumeric() || c == '_' || c == '$'))
            .unwrap_or(0);
        let name = &tail[..end];
        let after = &tail[end..];
        if after.starts_with(":{") && !name.is_empty() {
            marked.push_str(name);
            marked.push_str(":{if(__inGL)__diag.push('C:");
            marked.push_str(name);
            marked.push_str("');");
            rest = &after[2..];
        } else {
            marked.push_str(name);
            rest = after;
        }
    }
    marked.push_str(rest);
    src = marked;
    // Sonde proto : toute lecture `x.W` non-own passe ici —
    // log + l'erreur moteur enrichie nomme le receveur
    // (`[got undefined]` + `fn@pos` des valeurs).
    let wrapped = format!(
        "var __diag=[]; var __inGL=false;\
         Object.defineProperty(Object.prototype,'W',{{configurable:true,get:function(){{__diag.push('W-GET');return undefined}}}});\
         try {{\n{src}\n}} catch (__e) {{ \
            __diag.push('EXC:' + String(__e && __e.name) + '|' + String(__e && __e.message) + '|STACK:' + String(__e && __e.stack).split('\\n').slice(0,10).join(';')) \
        }}; __diag.slice(-80).join('\\n')"
    );
    match e.eval_named(&wrapped, "akamai.js") {
        Ok(v) => eprintln!("EVAL OK -> {}", e.describe(v)),
        Err(err) => eprintln!("EVAL ERR -> {err}"),
    }
}

/// §B-72 bras 3 — driver du reload : `location.assign`/
/// `reload`/`replace`/`href`/`window.location=` marqués avec
/// stack. Verdict vol 504 in-process : AUCUN des bundles
/// (`smt-bridge`/`main`/`akamai`) ne tire `location.*` en
/// standalone — le driver vit dans OtAutoBlock/otSDKStub
/// (fixtures non capturées) ou le code inline du shell.
#[test]
#[ignore = "établi de bissection §B-72 bras 3 — pas un verdict"]
fn fifa_shell_reload_driver() {
    let Some(html) = fixture("shell.html") else {
        eprintln!("fixture absente — skip");
        return;
    };
    let mut e = engine_with_dom(&html);
    let marker = r"
        var __nav = [];
        ['assign','reload','replace','href'].forEach(function(k){
            try {
                var d = Object.getOwnPropertyDescriptor(location, k) ||
                        Object.getOwnPropertyDescriptor(Object.getPrototypeOf(location), k) ||
                        {value: location[k], writable: true, configurable: true};
                var orig = d.value;
                Object.defineProperty(location, k, {
                    configurable: true,
                    get: function(){ return function(){
                        __nav.push(k + '|' + new Error().stack);
                        return typeof orig === 'function' ? orig.apply(location, arguments) : undefined;
                    }; },
                    set: function(v){ __nav.push(k + '=SET:' + String(v).slice(0,80) + '|' + new Error().stack); }
                });
            } catch(_e) { __nav.push(k + '|MARK-FAIL:' + _e.message); }
        });
        ['location'].forEach(function(k){
            try {
                var cur = window[k];
                Object.defineProperty(window, k, {
                    configurable: true,
                    get: function(){ return cur; },
                    set: function(v){ __nav.push('window.' + k + '=' + String(v).slice(0,80) + '|' + new Error().stack); cur = v; }
                });
            } catch(_e) { __nav.push('window.' + k + '|MARK-FAIL:' + _e.message); }
        });
        'armed:' + __nav.length
    ";
    match e.eval(marker) {
        Ok(v) => eprintln!("marker -> {}", e.describe(v)),
        Err(err) => eprintln!("marker ERR -> {err}"),
    }
    for f in ["smt-bridge.js", "main.js", "akamai.js"] {
        if let Some(s) = fixture(f) {
            let w = format!("try {{\n{s}\n}} catch (__e) {{ 'EXC:{f}:' + String(__e && __e.name) + '|' + String(__e && __e.message) }}");
            match e.eval_named(&w, f) {
                Ok(v) => eprintln!("{f} OK -> {}", e.describe(v)),
                Err(err) => eprintln!("{f} ERR -> {err}"),
            }
        }
    }
    // Drain : les timers planifiés tirent au prochain eval.
    for _ in 0..8 {
        let _ = e.eval("0");
    }
    match e.eval("__nav.join('\\n---\\n')") {
        Ok(v) => eprintln!("NAV ->\n{}", e.describe(v)),
        Err(err) => eprintln!("NAV ERR -> {err}"),
    }
}
