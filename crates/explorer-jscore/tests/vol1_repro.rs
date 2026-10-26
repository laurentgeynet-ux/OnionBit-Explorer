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

/// `Insert.index` du journal compte les enfants **sérialisables**
/// (Element|Text) — indexer sur `children` brut décalait d'un
/// cran par `<!-- -->` antérieur : le graft moteur posait le
/// nœud un slot trop tard (les `<li>` de craigslist sortaient de
/// leur `<ul>` — vol2).
#[test]
fn vol14_journal_insert_index_skips_comments() {
    use explorer_jscore::{Dom, JournalOp};
    let mut d = Dom::from_html("<ul><!--c--><li>a</li></ul>");
    d.clear_journal();
    let ul = d.by_tag(d.root, "ul")[0];
    let li = d.create_element("li");
    let t = d.create_text("b");
    d.append(li, t);
    d.append(ul, li);
    let ops = d.take_journal().unwrap_or_default();
    let Some(JournalOp::Insert { index, .. }) =
        ops.iter().find(|op| matches!(op, JournalOp::Insert { .. }))
    else {
        panic!("pas d'Insert dans le journal: {ops:?}");
    };
    // Un seul enfant sérialisable avant le nouveau <li> —
    // le commentaire ne compte pas.
    assert_eq!(*index, 1);
}

/// `yield` à l'intérieur d'un `switch` dans un générateur —
/// le `Stmt::Switch` n'avait pas de frame `GExec` et passait
/// en éval eager → `SyntaxError: yield outside a generator`
/// (github `4u-*.js`, hydratation React morte, recherche vide).
#[test]
fn vol8_yield_inside_switch() {
    let mut e = Engine::new();
    for (src, want) in [
        (
            "var g=function*(){switch(1){case 1: yield 14}}; g().next().value",
            "14",
        ),
        // fallthrough : case atteint + corps suivants.
        (
            "var g2=function*(){switch(1){case 1: yield 1; case 2: yield 2; default: yield 3}}; [...g2()].join(',')",
            "1,2,3",
        ),
        // `break` nu sort du switch (FKind::Switch est sa cible).
        (
            "var g3=function*(){switch(1){case 1: yield 1; break; case 2: yield 2} yield 9}; [...g3()].join(',')",
            "1,9",
        ),
        // `default` seulement si aucun test ne matche.
        (
            "var g4=function*(){switch(7){case 1: yield 1; default: yield 2; case 3: yield 3}}; [...g4()].join(',')",
            "2,3",
        ),
    ] {
        match e.eval(src) {
            Ok(v) => assert_eq!(e.describe(v), want, "{src}"),
            Err(err) => panic!("{src} -> {err}"),
        }
    }
}

/// `in_gen` clobberé par une fonction NON-génératrice imbriquée
/// — `fn_body` restaurait la valeur posée par le corps, pas
/// celle de l'appelant : `function*(){var f=function(){};
/// yield x}` rejetait `expected ';'` (spotify vendor +
/// netlify `expected )`, les deux fichiers verbatim passent).
#[test]
fn vol12_nested_fn_restores_in_gen() {
    let mut e = Engine::new();
    for (src, want) in [
        (
            "var g=function*(){var f=function(){return 1}; yield 2}; [...g()][0]",
            "2",
        ),
        (
            "var g=function*(){var h={m(){return 1}, get g(){return 2}, set g(v){}}; yield 3}; [...g()][0]",
            "3",
        ),
        (
            "var g=function(){var g2=function*(){var f=function(){1}; yield 5}; return [...g2()]}; g()[0]",
            "5",
        ),
    ] {
        match e.eval(src) {
            Ok(v) => assert_eq!(e.describe(v), want, "{src}"),
            Err(err) => panic!("{src} -> {err}"),
        }
    }
    // Extrait verbatim `vendor~mobile-web-player.344bcab5.js` :
    // `__awaiter(this,void 0,void 0,function*(){…yield i(…)})`
    // — le `yield` à l'offset de l'erreur « expected ';' ».
    let slice = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/data/vol12_spotify_gen_slice.js"
    ))
    .unwrap_or_else(|e| panic!("fixture: {e}"));
    let src = format!("var g=({slice});typeof g");
    match e.eval(&src) {
        Ok(v) => assert_eq!(e.describe(v), "function"),
        Err(err) => panic!("spotify slice -> {err}"),
    }
}

/// `this` sloppy = `ToObject(thisArg)` — primitives boxées en
/// wrappers (jQuery `g.each`/`propFix`, codepen `everypage`,
/// soundcloud `1-*.js` : `toLowerCase is not a function (on
/// Object)`). Seuls `null`/`undefined` → objet global.
#[test]
fn vol14_sloppy_this_boxes_primitives() {
    let mut e = Engine::new();
    for (src, want) in [
        ("(function(){return this.toLowerCase()}).call('AB')", "ab"),
        (
            "(function(){return Object.prototype.toString.call(this)}).call('AB')",
            "[object String]",
        ),
        ("(function(){return this.length}).call('ABC')", "3"),
        ("(function(){return this.toFixed(1)}).call(3.14)", "3.1"),
        (
            "['x'].map(function(){return this.toLowerCase()},'AB')[0]",
            "ab",
        ),
        ("(function(){return typeof this}).call(null)", "object"),
        (
            "(function(){'use strict';return this.toLowerCase()}).call('AB')",
            "ab",
        ),
    ] {
        match e.eval(src) {
            Ok(v) => assert_eq!(e.describe(v), want, "{src}"),
            Err(err) => panic!("{src} -> {err}"),
        }
    }
}

/// `document.getElementsByTagName('head')[0]` — un document a
/// TOUJOURS `html`/`head`/`body` (auto-générés par le parseur) ;
/// les miroirs de cadre vide (`about:blank`) les auto-vivent —
/// le bootstrap Cloudflare `c()` de jsfiddle jetait
/// `cannot read 'appendChild' of Undefined` ×12.
#[test]
fn vol14_head_autovivified_for_tagname() {
    use explorer_jscore::Dom;
    let mut e = Engine::new();
    e.install_dom(&Rc::new(RefCell::new(Dom::from_html(""))));
    for (src, want) in [
        (
            "typeof document.getElementsByTagName('head')[0]",
            "object",
        ),
        (
            "document.getElementsByTagName('head')[0].appendChild(document.createElement('script')); document.getElementsByTagName('script').length",
            "1",
        ),
        (
            "typeof document.getElementsByTagName('body')[0]",
            "object",
        ),
        (
            "typeof document.getElementsByTagName('html')[0]",
            "object",
        ),
    ] {
        match e.eval(src) {
            Ok(v) => assert_eq!(e.describe(v), want, "{src}"),
            Err(err) => panic!("{src} -> {err}"),
        }
    }
}

/// `el.src`/`el.href` résolus contre `document.baseURI` à la
/// lecture (spec IDL — `getAttribute` reste brut) : `new URL(
/// el.src)` échouait `Invalid URL`/`Invalid scheme` (lemonde,
/// franceinfo).
#[test]
fn vol13_src_href_resolve_against_base() {
    use explorer_jscore::Dom;
    let mut e = Engine::new();
    e.sync_ctx_url(0, "https://www.lemonde.fr/x/y", 1);
    e.install_dom(&Rc::new(RefCell::new(Dom::from_html(
        "<img src='/a/b.png'><a href='/p?q=1'>x</a>",
    ))));
    for (src, want) in [
        (
            "document.querySelector('img').src",
            "https://www.lemonde.fr/a/b.png",
        ),
        (
            "document.querySelector('a').href",
            "https://www.lemonde.fr/p?q=1",
        ),
        ("document.querySelector('a').getAttribute('href')", "/p?q=1"),
        (
            "new URL(document.querySelector('img').src).href",
            "https://www.lemonde.fr/a/b.png",
        ),
    ] {
        match e.eval(src) {
            Ok(v) => assert_eq!(e.describe(v), want, "{src}"),
            Err(err) => panic!("{src} -> {err}"),
        }
    }
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
