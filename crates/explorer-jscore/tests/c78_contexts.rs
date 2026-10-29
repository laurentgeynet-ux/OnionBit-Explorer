// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! JS-C78 — contextes de navigation imbriqués (`<iframe>`) :
//! un realm par contexte (globals isolés, intrinsics frais),
//! `window.parent`/`top`/`frames[n]`/`length`, `iframe.
//! contentWindow`/`contentDocument` bornés same-origin,
//! `postMessage` structuré `WireValue` + `targetOrigin` exigé,
//! `MessageEvent` livré après la frontière de jobs de
//! l'émetteur, jamais en réentrance.

use explorer_jscore::{Dom, Engine};
use std::cell::RefCell;
use std::rc::Rc;

const TOP_HTML: &str = r#"<html><body>
<iframe id="f" src="about:blank"></iframe>
<div id="t">top</div>
</body></html>"#;

const CHILD_HTML: &str = r#"<html><head><title>child doc</title></head><body>
<p id="c">child</p>
</body></html>"#;

const TOP_URL: &str = "https://top.example/page";
const CHILD_URL: &str = "https://top.example/frame";
const CROSS_URL: &str = "https://other.example/frame";

/// Engine top-level + DOM parent posé (`ctx 0` = `TOP_URL`)..
fn top_engine() -> (Engine, Rc<RefCell<Dom>>) {
    let mut e = Engine::new();
    let d = Rc::new(RefCell::new(Dom::from_html(TOP_HTML)));
    e.install_dom(&d);
    e.set_page_url(TOP_URL);
    (e, d)
}

/// `NodeId` du premier `<iframe>` du miroir `d`.
fn iframe_node(d: &Rc<RefCell<Dom>>) -> u32 {
    u32::try_from(
        d.borrow()
            .nodes
            .iter()
            .position(|n| n.tag == "iframe")
            .expect("TOP_HTML contient un <iframe>"),
    )
    .expect("index <iframe> tient en u32")
}

/// Crée un contexte enfant (courant après `context_new`), y
/// installe son propre DOM + URL, puis repasse le courant au
/// top-level. Renvoie le `CtxId` enfant.
fn add_child(e: &mut Engine, parent: &Rc<RefCell<Dom>>, url: &str) -> u32 {
    let node = iframe_node(parent);
    let id = e.context_new(url, Some((Rc::as_ptr(parent) as usize, node)));
    let cd = Rc::new(RefCell::new(Dom::from_html(CHILD_HTML)));
    e.install_dom(&cd);
    e.set_page_url(url);
    e.install_timers();
    e.context_switch(0);
    id
}

fn ev(e: &mut Engine, src: &str) -> String {
    match e.eval(src) {
        Ok(v) => e.describe(v),
        Err(err) => panic!("{src} -> {err}"),
    }
}

/// Évalue `src` dans le contexte `ctx` (switch → eval → restore).
fn ev_ctx(e: &mut Engine, ctx: u32, src: &str) -> String {
    e.context_switch(ctx);
    let out = ev(e, src);
    e.context_switch(0);
    out
}

// -- isolation des realms ----------------------------------------------

#[test]
fn c78_globals_isolated_between_contexts() {
    let (mut e, d) = top_engine();
    let child = add_child(&mut e, &d, CHILD_URL);
    // `var`/`let` vivent dans le global/env du contexte —
    // l'enfant ne voit pas les bindings du parent.
    ev(&mut e, "var topSecret = 'top-value'");
    assert_eq!(ev_ctx(&mut e, child, "typeof topSecret"), "undefined");
    // Symétrique.
    ev_ctx(&mut e, child, "var childSecret = 'child-value'; 'ok'");
    assert_eq!(ev(&mut e, "typeof childSecret"), "undefined");
}

#[test]
fn c78_fresh_intrinsics_per_realm() {
    let (mut e, d) = top_engine();
    let _child = add_child(&mut e, &d, CHILD_URL);
    // Spec : chaque realm a SES prototypes —
    // `frame.Array.prototype !== Array.prototype` ; les objets
    // fabriqués dans un realm gardent leurs propres intrinsics.
    assert_eq!(
        ev(&mut e, "frames[0].Array.prototype === Array.prototype"),
        "false"
    );
    // Mais `instanceof` cross-realm reste faux (spec réelle).
    assert_eq!(ev(&mut e, "frames[0].eval('[]') instanceof Array"), "false");
}

#[test]
fn c78_child_eval_runs_in_child_realm() {
    let (mut e, d) = top_engine();
    let _child = add_child(&mut e, &d, CHILD_URL);
    // `frames[0].eval` appelle la fonction eval DE L'ENFANT —
    // le switch transactionnel exécute dans son realm (son
    // `global_env` est la racine de l'env capturée).
    assert_eq!(ev(&mut e, "frames[0].eval('document.title')"), "child doc");
    // L'enfant voit SON document, pas celui du parent.
    assert_eq!(
        ev(&mut e, r#"frames[0].eval('document.getElementById("t")')"#),
        "null"
    );
    assert_eq!(
        ev(
            &mut e,
            r#"frames[0].eval('document.getElementById("c").textContent')"#
        ),
        "child"
    );
}

// -- arbre de fenêtres ---------------------------------------------------

#[test]
fn c78_window_parent_top_frames() {
    let (mut e, d) = top_engine();
    let _child = add_child(&mut e, &d, CHILD_URL);
    // Top-level : `parent`/`top`/`frames` ≙ soi-même (spec).
    assert_eq!(ev(&mut e, "parent === window"), "true");
    assert_eq!(ev(&mut e, "top === window"), "true");
    assert_eq!(ev(&mut e, "frames === window"), "true");
    // Dans l'enfant : `parent`/`top` = la fenêtre top.
    assert_eq!(ev_ctx(&mut e, 1, "parent === top"), "true");
    assert_eq!(ev_ctx(&mut e, 1, "parent !== window"), "true");
    assert_eq!(ev_ctx(&mut e, 1, "top.parent === top"), "true");
    // `frames[n]`/`length` côté parent.
    assert_eq!(ev(&mut e, "window.length"), "1");
    assert_eq!(ev(&mut e, "frames.length"), "1");
    assert_eq!(ev(&mut e, "frames[0] === window[0]"), "true");
    // `frames[0]` est le `window` de l'enfant (same-origin →
    // le vrai global : on y lit `document`).
    assert_eq!(ev(&mut e, "frames[0].document.title"), "child doc");
    // `length`/`closed` côté enfant.
    assert_eq!(ev_ctx(&mut e, 1, "window.length"), "0");
    assert_eq!(ev_ctx(&mut e, 1, "window.closed"), "false");
}

#[test]
fn c78_iframe_content_window_document() {
    let (mut e, d) = top_engine();
    let _child = add_child(&mut e, &d, CHILD_URL);
    // `contentWindow` = le `window` de l'enfant, identique à
    // `frames[0]` (même objet — same-origin → vrai global).
    let w = ev(
        &mut e,
        "var f = document.getElementById('f');\n\
         f.contentWindow === frames[0]",
    );
    assert_eq!(w, "true");
    // `contentDocument` = le document de l'enfant (same-origin).
    assert_eq!(ev(&mut e, "f.contentDocument.title"), "child doc");
    assert_eq!(
        ev(&mut e, "f.contentDocument === frames[0].document"),
        "true"
    );
    // Identité stable : deux accès rendent le même objet.
    assert_eq!(ev(&mut e, "f.contentWindow === f.contentWindow"), "true");
}

// -- cross-origin --------------------------------------------------------

#[test]
fn c78_cross_origin_windowproxy_opaque() {
    let (mut e, d) = top_engine();
    let _child = add_child(&mut e, &d, CROSS_URL);
    // `contentDocument` cross-origin → `null` (spec — jamais
    // un miroir d'un autre domaine).
    let f = ev(
        &mut e,
        "var f = document.getElementById('f');\nf.contentDocument",
    );
    assert_eq!(f, "null");
    // `contentWindow` cross-origin → le `WindowProxy` opaque
    // (PAS null — `postMessage` doit rester joignable ; c'est
    // la surface spec réelle, documentée dans l'arrêt journal :
    // opaque, whitelistée, jamais d'objet natif).
    assert_eq!(ev(&mut e, "f.contentWindow === null"), "false");
    assert_eq!(ev(&mut e, "typeof f.contentWindow"), "object");
    // Surface whitelistée : `postMessage` callable, `closed`,
    // `length`, `top`/`parent`/`frames`/`self`/`window`/`opener`.
    assert_eq!(ev(&mut e, "typeof f.contentWindow.postMessage"), "function");
    assert_eq!(ev(&mut e, "f.contentWindow.closed"), "false");
    assert_eq!(ev(&mut e, "f.contentWindow.length"), "0");
    assert_eq!(ev(&mut e, "f.contentWindow.opener"), "null");
    // Identité proxy stable (même `WindowProxy` à chaque accès).
    assert_eq!(ev(&mut e, "f.contentWindow === frames[0]"), "true");
    // Toute propriété hors whitelist → `undefined` (opaque).
    assert_eq!(ev(&mut e, "f.contentWindow.document"), "undefined");
    assert_eq!(ev(&mut e, "f.contentWindow.topSecret"), "undefined");
    // Écriture ignorée silencieusement (spec).
    assert_eq!(
        ev(&mut e, "f.contentWindow.x = 1; f.contentWindow.x"),
        "undefined"
    );
}

#[test]
fn c78_cross_origin_parent_top_via_proxy() {
    let (mut e, d) = top_engine();
    let _child = add_child(&mut e, &d, CROSS_URL);
    // L'enfant cross-origin voit `parent`/`top` : le parent est
    // cross-origin pour lui → il reçoit le proxy du top, pas le
    // global (opaque des deux côtés).
    assert_eq!(ev_ctx(&mut e, 1, "typeof parent"), "object");
    assert_eq!(ev_ctx(&mut e, 1, "parent.document"), "undefined");
    assert_eq!(ev_ctx(&mut e, 1, "typeof top.postMessage"), "function");
    // `top === parent` pour un enfant direct (même proxy).
    assert_eq!(ev_ctx(&mut e, 1, "top === parent"), "true");
}

// -- postMessage ---------------------------------------------------------

#[test]
fn c78_post_message_same_origin() {
    let (mut e, d) = top_engine();
    let _child = add_child(&mut e, &d, CHILD_URL);
    // L'enfant enregistre son handler.
    ev_ctx(
        &mut e,
        1,
        "var got = null;\n\
         window.addEventListener('message', function(ev) {\n\
             got = { data: ev.data, origin: ev.origin, same: ev.source === parent };\n\
         });",
    );
    // Le parent poste — structured clone `WireValue` (objet
    // imbriqué conservé, jamais référencé). `f.contentWindow.got`
    // lu dans le MÊME eval prouve la non-réentrance : le message
    // part en file, livré à la frontière de job (fin d'eval).
    assert_eq!(
        ev(
            &mut e,
            "var f = document.getElementById('f');\n\
             f.contentWindow.postMessage({a: 1, nested: {b: [2,3]}}, 'https://top.example');\n\
             f.contentWindow.got === null"
        ),
        "true"
    );
    // Après l'eval (checkpoint microtasks), le message est livré.
    e.run_microtasks().expect("microtasks");
    assert_eq!(ev_ctx(&mut e, 1, "got.data.a"), "1");
    assert_eq!(ev_ctx(&mut e, 1, "got.data.nested.b[1]"), "3");
    assert_eq!(ev_ctx(&mut e, 1, "got.origin"), "https://top.example");
    // `event.source` = le `window` de l'émetteur (same-origin →
    // le vrai global — identité `===` vérifiable).
    assert_eq!(ev_ctx(&mut e, 1, "got.same"), "true");
}

#[test]
fn c78_post_message_target_origin_enforced() {
    let (mut e, d) = top_engine();
    let _child = add_child(&mut e, &d, CHILD_URL);
    ev_ctx(
        &mut e,
        1,
        "var got = 0; window.addEventListener('message', function() { got++; });",
    );
    // Mauvaise origine → jamais livré (échec silencieux, spec).
    ev(
        &mut e,
        "frames[0].postMessage('x', 'https://wrong.example')",
    );
    e.run_microtasks().expect("microtasks");
    assert_eq!(ev_ctx(&mut e, 1, "got"), "0");
    // Bonne origine → livré.
    ev(&mut e, "frames[0].postMessage('x', 'https://top.example')");
    e.run_microtasks().expect("microtasks");
    assert_eq!(ev_ctx(&mut e, 1, "got"), "1");
}

#[test]
fn c78_post_message_origin_required_and_star_rejected() {
    let (mut e, d) = top_engine();
    let _child = add_child(&mut e, &d, CHILD_URL);
    // Spec WHATWG moderne (JS-C189 — steam vol 115) :
    // `postMessage(msg)` sans `targetOrigin` = `"/"` (same-origin
    // only), **pas** SyntaxError — l'ancienne forme 2-args
    // obligatoire tuait `shared_global.js`. Ici enfant =
    // origine différente → drop silencieux, pas de throw.
    let r = e.eval("frames[0].postMessage('x')");
    assert!(r.is_ok(), "1-arg légal (défaut '/'), obtenu : {r:?}");
    // `'/'` = same-origin : l'enfant `top.example` le reçoit.
    ev_ctx(
        &mut e,
        1,
        "var got1 = 0; window.addEventListener('message', function() { got1++; });",
    );
    ev(&mut e, "frames[0].postMessage('x', '/')");
    e.run_microtasks().expect("microtasks");
    assert_eq!(ev_ctx(&mut e, 1, "got1"), "1");
    // Forme objet `options` (spec) : `{targetOrigin}`.
    ev(&mut e, "frames[0].postMessage('y', {targetOrigin: '/'})");
    e.run_microtasks().expect("microtasks");
    assert_eq!(ev_ctx(&mut e, 1, "got1"), "2");
    // `''` → SyntaxError (origine vide illégale).
    let r = e.eval("frames[0].postMessage('x', '')");
    assert!(r.is_err(), "'' doit jeter SyntaxError");
    let msg = format!("{:?}", r.unwrap_err());
    assert!(msg.contains("SyntaxError"), "SyntaxError attendu : {msg}");
    // `'*'` (spec) = livrer sans check d'origine — la cible
    // reçoit `origin`/`source` honnêtes (cap « web réel » :
    // les embeds font `postMessage(x, '*')`).
    ev_ctx(
        &mut e,
        1,
        "var got = null; window.addEventListener('message', function(ev) { got = { d: ev.data, o: ev.origin }; });",
    );
    ev(&mut e, "frames[0].postMessage('x', '*')");
    e.run_microtasks().expect("microtasks");
    assert_eq!(ev_ctx(&mut e, 1, "got.d"), "x");
    assert_eq!(ev_ctx(&mut e, 1, "got.o"), "https://top.example");
    // `targetOrigin` explicite non concordant → pas de livraison
    // (silencieux, spec).
    ev_ctx(&mut e, 1, "got = null;");
    ev(
        &mut e,
        "frames[0].postMessage('x', 'https://nomatch.example')",
    );
    e.run_microtasks().expect("microtasks");
    assert_eq!(ev_ctx(&mut e, 1, "got"), "null");
}

/// `scrollTo`/`scrollBy`/`scroll`/`focus`/`blur`/`close` sur le
/// global — no-op bornés honnêtes (CAP-175 : les scripts les
/// appellent librement au boot, jamais d'exception — même motif
/// que `scrollIntoView`, JS-C74 ; la géométrie/scroll réel reste
/// l'arc #19/JS-8).
#[test]
fn c78_window_scroll_focus_noop() {
    let (mut e, _d) = top_engine();
    let s = e
        .eval(
            "[ typeof scrollTo, typeof scrollBy, typeof scroll,
               typeof focus, typeof blur, typeof close ].join(',')",
        )
        .map(|v| e.describe(v))
        .unwrap_or_default();
    assert_eq!(s, "function,function,function,function,function,function");
    // Les appels ne jettent pas (no-op bornés).
    e.eval("scrollTo(0,0); scrollBy(0,0); scroll(0,0); focus(); blur()")
        .expect("no-ops appelables");
    // `window.scrollX`/`scrollY` restent 0 (pas de scroll
    // plombé — état honnête).
    assert_eq!(
        e.eval("scrollX + ',' + scrollY")
            .map(|v| e.describe(v))
            .unwrap_or_default(),
        "0,0"
    );
}

#[test]
fn c78_post_message_cross_origin() {
    let (mut e, d) = top_engine();
    let _child = add_child(&mut e, &d, CROSS_URL);
    ev_ctx(&mut e, 1, "var got = null; window.addEventListener('message', function(ev) { got = { d: ev.data, o: ev.origin, s: ev.source === parent }; });");
    // Cross-origin : `contentWindow` est le proxy — son
    // `postMessage` cible l'enfant, l'origine du parent est
    // transmise, `source` = proxy du parent (opaque, `===
    // parent` vrai côté enfant).
    ev(
        &mut e,
        "document.getElementById('f').contentWindow.postMessage('ping', 'https://other.example')",
    );
    e.run_microtasks().expect("microtasks");
    assert_eq!(ev_ctx(&mut e, 1, "got.d"), "ping");
    assert_eq!(ev_ctx(&mut e, 1, "got.o"), "https://top.example");
    assert_eq!(ev_ctx(&mut e, 1, "got.s"), "true");
}

#[test]
fn c78_post_message_delivered_after_sender_microtasks() {
    let (mut e, d) = top_engine();
    let _child = add_child(&mut e, &d, CHILD_URL);
    // L'enfant lit `event.source` (global parent, same-origin) —
    // il doit voir la microtask du parent DÉJÀ exécutée quand le
    // message arrive (spec : postMessage poste une *task*, les
    // microtasks du job émetteur la précèdent).
    ev_ctx(
        &mut e,
        1,
        "var seen = null;\n\
         window.addEventListener('message', function(ev) {\n\
             seen = ev.source.order.join(',');\n\
         });",
    );
    ev(
        &mut e,
        "var order = [];\n\
         Promise.resolve().then(function() { order.push('microtask'); });\n\
         frames[0].postMessage('x', 'https://top.example');\n\
         order.push('sync');",
    );
    e.run_microtasks().expect("microtasks");
    // `order` côté parent : 'sync' puis 'microtask'.
    assert_eq!(ev(&mut e, "order.join(',')"), "sync,microtask");
    // Le handler enfant a vu 'sync,microtask' — la microtask du
    // parent a tourné avant la livraison.
    assert_eq!(ev_ctx(&mut e, 1, "seen"), "sync,microtask");
}

#[test]
fn c78_post_message_no_reentrancy() {
    let (mut e, d) = top_engine();
    let _child = add_child(&mut e, &d, CHILD_URL);
    // Le handler de l'enfant reposte au parent — le message
    // retour part en file, livré après (jamais en réentrance
    // dans la livraison courante).
    ev_ctx(
        &mut e,
        1,
        "window.addEventListener('message', function(ev) {\n\
             parent.postMessage('pong', 'https://top.example');\n\
         });",
    );
    ev(
        &mut e,
        "var pong = 0;\n\
         window.addEventListener('message', function() { pong++; });\n\
         frames[0].postMessage('ping', 'https://top.example');\n\
         pong", // livraison pas encore faite → 0
    );
    e.run_microtasks().expect("microtasks");
    assert_eq!(ev(&mut e, "pong"), "1");
}

#[test]
fn c78_structured_clone_cycles() {
    let (mut e, d) = top_engine();
    let _child = add_child(&mut e, &d, CHILD_URL);
    ev_ctx(
        &mut e,
        1,
        "var got = null; window.addEventListener('message', function(ev) { got = ev.data; });",
    );
    // Objet cyclique — `WireValue` préserve les back-refs
    // (réutilise la mécanique Worker C29).
    ev(
        &mut e,
        "var o = {name: 'cyc'}; o.self = o;\n\
         frames[0].postMessage(o, 'https://top.example')",
    );
    e.run_microtasks().expect("microtasks");
    assert_eq!(ev_ctx(&mut e, 1, "got.self === got"), "true");
    assert_eq!(ev_ctx(&mut e, 1, "got.name"), "cyc");
    // Le clone est indépendant : muter la copie ne touche pas
    // l'original.
    ev_ctx(&mut e, 1, "got.name = 'mutated'; 'ok'");
    assert_eq!(ev(&mut e, "o.name"), "cyc");
}

// -- cycle de vie --------------------------------------------------------

#[test]
fn c78_context_destroy_closes() {
    let (mut e, d) = top_engine();
    let child = add_child(&mut e, &d, CHILD_URL);
    // Une référence gardée au `window` enfant reste valide —
    // `closed` y passera à `true` après destruction (spec).
    ev(&mut e, "var w = frames[0]; 'ok'");
    e.context_destroy(child);
    assert_eq!(ev(&mut e, "window.length"), "0");
    assert_eq!(ev(&mut e, "frames[0]"), "undefined");
    // `closed` lit le meta `alive` du contexte de `w`.
    assert_eq!(ev(&mut e, "w.closed"), "true");
    // postMessage vers un contexte mort → no-op silencieux.
    ev(&mut e, "w.postMessage('x', 'https://top.example')");
    e.run_microtasks().expect("microtasks");
}

#[test]
fn c78_gc_roots_parked_context() {
    let (mut e, d) = top_engine();
    let child = add_child(&mut e, &d, CHILD_URL);
    // L'enfant garde un gros état garé — une passe GC ne doit
    // pas balayer son global/env/document.
    ev_ctx(
        &mut e,
        child,
        "var keep = {deep: [1,2,3], fn: function() { return 42; }}; 'ok'",
    );
    e.collect();
    e.collect();
    assert_eq!(ev_ctx(&mut e, child, "keep.fn()"), "42");
    assert_eq!(ev_ctx(&mut e, child, "keep.deep.join('-')"), "1-2-3");
    // Le document enfant survit aussi (dom_ctx raciné).
    assert_eq!(ev_ctx(&mut e, child, "document.title"), "child doc");
}

#[test]
fn c78_timers_run_per_context() {
    let (mut e, d) = top_engine();
    let child = add_child(&mut e, &d, CHILD_URL);
    // Un timer posé dans l'enfant tire même quand le parent est
    // le contexte courant — la pompe visite tous les ctx vivants.
    ev_ctx(
        &mut e,
        child,
        "var fired = 0; setTimeout(function() { fired++; }, 0); 'ok'",
    );
    assert_eq!(e.pending_timers(), 1);
    // Simule l'écoulement : on ne peut pas attendre — le timer
    // `0` ms est déjà dû au prochain tick.
    std::thread::sleep(std::time::Duration::from_millis(2));
    e.run_due_timers().expect("timers");
    assert_eq!(ev_ctx(&mut e, child, "fired"), "1");
}

#[test]
fn c78_child_posts_to_parent_during_own_eval() {
    let (mut e, d) = top_engine();
    ev(&mut e, "var got=null; window.addEventListener('message',function(ev){got=ev.data.tag+'|'+ev.origin;})");
    let _c = add_child(&mut e, &d, CHILD_URL);
    // L'enfant poste PENDANT sa propre évaluation (session :
    // script inline au commit) — la livraison attend la
    // prochaine pompe de microtasks.
    ev_ctx(
        &mut e,
        1,
        "parent.postMessage({tag:'pong'},'https://top.example')",
    );
    e.run_microtasks().expect("microtasks");
    assert_eq!(ev(&mut e, "got"), "pong|https://top.example");
}
