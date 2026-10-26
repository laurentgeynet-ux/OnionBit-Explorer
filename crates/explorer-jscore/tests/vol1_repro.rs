// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Repros du vol de terrain 2026-10-26 (rapports agents 1–4) :
//! chaque test est le point dur qui tuait le bootstrap d'un site
//! réel — GitLab, Excalidraw, Codeberg, ecma-international,
//! bandcamp, openstreetmap.

use explorer_jscore::{Engine, Value};
use std::cell::RefCell;
use std::rc::Rc;

/// Regex emoji du bundle GitLab — classes `\uXXXX` + ranges +
/// surrogate pairs (verbatim `vol1_gitlab_console.txt`,
/// 12,8 Kio). Rejetée « invalid regex » avant le fix.
#[test]
fn vol1_gitlab_emoji_regex_compiles() {
    let pat = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/data/vol1_emoji_regex.txt"
    ))
    .unwrap_or_else(|e| panic!("fixture: {e}"));
    let mut e = Engine::new();
    let mut lit = String::from("\"");
    for c in pat.chars() {
        match c {
            '"' => lit.push_str("\\\""),
            '\\' => lit.push_str("\\\\"),
            _ => lit.push(c),
        }
    }
    lit.push('"');
    let r = e.eval(&format!("new RegExp({lit})"));
    assert!(r.is_ok(), "gitlab emoji regex: {r:?}");
}

/// `\uD8hi\uDClo` hors classe = paire astrale (sloppy comme `u`) ;
/// `\uD8hi[\uDClo-\uDChi]` = plage astrale (idiome skin-tone).
#[test]
fn vol1_regex_surrogate_idioms() {
    let mut e = Engine::new();
    for (src, want) in [
        (r#"/\uD83D\uDCA5/.test("\u{1F4A5}")"#, true),
        (r#"/\uD83C[\uDFFB-\uDFFF]/.test("\u{1F3FD}")"#, true),
        (r#"/^[-]?[0-9]+(?:[eE][+-]?[0-9]+)?$/.test("-3")"#, true),
        (r#"/^[-]?[0-9]+(?:[eE][+-]?[0-9]+)?$/.test("12e+4")"#, true),
        (r#"/[-]/.test("-")"#, true),
    ] {
        match e.eval(src) {
            Ok(Value::Bool(b)) => assert_eq!(b, want, "{src}"),
            other => panic!("{src} -> {other:?}"),
        }
    }
}

/// `XMLHttpRequest.prototype` réel — Sentry instrumente
/// `Dr(XMLHttpRequest.prototype, "send", wrap)` ; `undefined`
/// jetait `TypeError: 'in' requires an object` (bootstrap
/// Excalidraw mort, page blanche).
#[test]
fn vol1_xhr_prototype_instrumentable() {
    struct T;
    impl explorer_jscore::XhrTransport for T {
        fn send(&self, _r: &explorer_jscore::XhrRequest) -> u64 {
            0
        }
    }
    let mut e = Engine::new();
    e.install_xhr(Rc::new(T));
    let src = r#"
        "send" in XMLHttpRequest.prototype
        && "XMLHttpRequest" in globalThis
        && new XMLHttpRequest() instanceof XMLHttpRequest
        && typeof new XMLHttpRequest().send === "function"
        && XMLHttpRequest.DONE === 4
        && XMLHttpRequest.prototype.DONE === 4"#;
    match e.eval(src) {
        Ok(Value::Bool(true)) => {}
        other => panic!("xhr proto: {other:?}"),
    }
}

/// `new XPathEvaluator().createExpression(…)` — htmx (codeberg /
/// gitea) en construit une au chargement du module ; la méthode
/// manquante tuait le bundle webpack.
#[test]
fn vol1_xpath_evaluator_surface() {
    use explorer_jscore::Dom;
    let mut e = Engine::new();
    e.install_dom(&Rc::new(RefCell::new(Dom::from_html("<p>x</p>"))));
    for (src, want) in [
        ("typeof XPathEvaluator", "function"),
        (
            "var xe = new XPathEvaluator(); typeof xe.createExpression",
            "function",
        ),
        ("typeof xe.createNSResolver", "function"),
        (
            "var ex = xe.createExpression('//p'); typeof ex.evaluate",
            "function",
        ),
    ] {
        match e.eval(src) {
            Ok(v) => assert_eq!(e.describe(v), want, "{src}"),
            Err(err) => panic!("{src} -> {err}"),
        }
    }
}

/// `Intl.supportedValuesOf` — stub honnête (sous-ensemble sans
/// CLDR) ; absent il tuait le bootstrap Vue de bandcamp.com.
#[test]
fn vol3_intl_supported_values_of() {
    let mut e = Engine::new();
    for (src, want) in [
        ("typeof Intl.supportedValuesOf", "function"),
        ("Intl.supportedValuesOf('timeZone').join(',')", "UTC"),
        ("Intl.supportedValuesOf('calendar').join(',')", "gregory"),
        ("Array.isArray(Intl.supportedValuesOf('unit'))", "true"),
    ] {
        match e.eval(src) {
            Ok(v) => assert_eq!(e.describe(v), want, "{src}"),
            Err(err) => panic!("{src} -> {err}"),
        }
    }
    // Clé inconnue → RangeError (spec).
    assert!(e.eval("Intl.supportedValuesOf('bogus')").is_err());
}

/// `Worker#addEventListener('message')` des deux côtés — Rails UJS
/// (openstreetmap) instrumente le wrapper, les workers internes
/// s'en servent aussi dans leur scope.
#[test]
fn vol2_worker_add_event_listener() {
    use explorer_jscore::{Dom, WorkerLoader};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    struct L;
    impl WorkerLoader for L {
        fn load(&mut self, spec: &str, _r: &str) -> Result<String, String> {
            if spec == "w.js" {
                Ok(
                    "self.addEventListener('message', function(ev){ postMessage(ev.data + 1); })"
                        .into(),
                )
            } else {
                Err(format!("not found: {spec}"))
            }
        }
    }
    let mut e = Engine::new();
    let dom: explorer_jscore::DomRef = Rc::new(RefCell::new(Dom::from_html("<html></html>")));
    e.install_dom(&dom);
    e.set_worker_factory(Arc::new(|| Box::new(L) as Box<dyn WorkerLoader>));
    e.eval(
        "var w = new Worker('w.js');\
         w.addEventListener('message', function(ev){ globalThis.__r = ev.data; });\
         w.postMessage(41)",
    )
    .unwrap_or_else(|err| panic!("worker script: {err}"));
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut got = String::new();
    while Instant::now() < deadline {
        e.pump_workers().unwrap_or_else(|t| panic!("pump: {t:?}"));
        got = e
            .eval("globalThis.__r")
            .map(|v| e.describe(v))
            .unwrap_or_default();
        if got == "42" {
            break;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(got, "42");
    assert_eq!(
        e.eval("typeof w.addEventListener")
            .map(|v| e.describe(v))
            .unwrap_or_default(),
        "function"
    );
}

/// `Range.createContextualFragment` — première exception de la
/// cascade `sc_*` de maps.google.com (vol9).
#[test]
fn vol9_range_contextual_fragment() {
    use explorer_jscore::Dom;
    let mut e = Engine::new();
    e.install_dom(&Rc::new(RefCell::new(Dom::from_html("<div id=x></div>"))));
    for (src, want) in [
        (
            "var r = document.createRange(); typeof r.createContextualFragment",
            "function",
        ),
        (
            "var f = r.createContextualFragment('<span>a</span><b>b</b>'); f.childNodes.length",
            "2",
        ),
        ("f.childNodes[0].tagName", "SPAN"),
        (
            "document.getElementById('x').appendChild(f); document.getElementById('x').childNodes.length",
            "2",
        ),
    ] {
        match e.eval(src) {
            Ok(v) => assert_eq!(e.describe(v), want, "{src}"),
            Err(err) => panic!("{src} -> {err}"),
        }
    }
}

/// `window` hérite `EventTarget.prototype` (`WebIDL`) — le polyfill
/// webcomponents (`YouTube`) y lit `__shady_native_addEventListener`
/// après `Object.defineProperty(EventTarget.prototype, …)`.
#[test]
fn vol10_window_inherits_event_target_prototype() {
    use explorer_jscore::Dom;
    let mut e = Engine::new();
    e.install_dom(&Rc::new(RefCell::new(Dom::from_html("<p></p>"))));
    for (src, want) in [
        (
            "Object.defineProperty(EventTarget.prototype, '__shady_native_addEventListener', {value: EventTarget.prototype.addEventListener}); typeof window.__shady_native_addEventListener",
            "function",
        ),
        ("window instanceof EventTarget", "true"),
    ] {
        match e.eval(src) {
            Ok(v) => assert_eq!(e.describe(v), want, "{src}"),
            Err(err) => panic!("{src} -> {err}"),
        }
    }
}

/// Les nœuds `Comment` n'ont pas de reflet moteur —
/// `serialize_keyed` les sautait en `<#comment>` littéral
/// (re-parsé en élément fantôme peint en texte sur des dizaines
/// de sites : MDN, JSFiddle, reddit, archive) ET décalait le zip
/// `ids` ↔ `engine_preorder` d'un cran par commentaire.
#[test]
fn vol14_comments_not_serialized() {
    use explorer_jscore::Dom;
    let d = Dom::from_html("<html><body><!-- x --><p>a<!--y--></p></body></html>");
    let (html, ids, tags) = d.serialize_keyed();
    assert!(!html.contains("#comment"), "{html}");
    assert!(!html.contains("<!--"), "{html}");
    // Sérialisables : html, body, p, et le texte "a".
    assert_eq!(ids.len(), 4, "{tags:?}");
    assert_eq!(ids.len(), tags.len());
}

/// `IDBIndex` & autres interfaces IDB déclarées — Rakuten :
/// `IDBIndex is not defined` était une `ReferenceError` fatale.
#[test]
fn vol6_idb_interfaces_declared() {
    use explorer_jscore::Dom;
    let mut e = Engine::new();
    e.install_dom(&Rc::new(RefCell::new(Dom::from_html("<p></p>"))));
    for name in [
        "IDBIndex",
        "IDBObjectStore",
        "IDBCursor",
        "IDBTransaction",
        "IDBRequest",
        "IDBDatabase",
    ] {
        match e.eval(&format!("typeof {name}")) {
            Ok(v) => assert_eq!(e.describe(v), "function", "{name}"),
            Err(err) => panic!("{name} -> {err}"),
        }
    }
    // `new IDBIndex()` → TypeError honnête (pas de ctor WebIDL).
    assert!(e.eval("new IDBIndex()").is_err());
}
