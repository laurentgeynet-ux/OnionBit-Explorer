// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Adaptateur `JscoreEngine` : proxy canal → thread `Rc` réel,
//! erreurs mappées, budget, `Send` — in-process, pas de socket.

use explorer_js::{JsEngine, JsError, JscoreEngine, NullEngine};

#[test]
fn jscore_engine_is_send() {
    fn assert_send<T: Send>() {}
    assert_send::<JscoreEngine>();
    assert_send::<NullEngine>();
}

#[test]
fn eval_returns_described_completion() {
    let mut eng = JscoreEngine::new();
    assert_eq!(eng.eval("1 + 2 * 3").unwrap(), "7");
    assert_eq!(eng.eval("'a' + 'b'").unwrap(), "ab");
}

#[test]
fn syntax_error_maps_to_variant() {
    let mut eng = JscoreEngine::new();
    match eng.eval("var = =") {
        Err(JsError::Syntax { line, .. }) => assert!(line >= 1),
        other => panic!("attendu Syntax, obtenu {other:?}"),
    }
}

#[test]
fn uncaught_maps_to_variant() {
    let mut eng = JscoreEngine::new();
    match eng.eval("throw 'boom'") {
        Err(JsError::Uncaught(m)) => assert!(m.contains("boom")),
        other => panic!("attendu Uncaught, obtenu {other:?}"),
    }
}

#[test]
fn budget_kills_infinite_loop() {
    let mut eng = JscoreEngine::new();
    eng.set_budget(10_000);
    let r = eng.eval("while (true) {}");
    assert!(r.is_err(), "la boucle infinie doit tomber sur budget");
    // Le moteur reste utilisable (contexte vivant, budget ré-armé).
    eng.set_budget(5_000_000);
    assert_eq!(eng.eval("40 + 2").unwrap(), "42");
}

#[test]
fn run_pending_drains_microtasks() {
    let mut eng = JscoreEngine::new();
    let _ = eng.eval("Promise.resolve(1)");
    let _ = eng.run_pending();
}

#[test]
fn factory_builds_contexts() {
    let f = JscoreEngine::factory();
    let mut a = (f)();
    let mut b = (f)();
    // Contextes isolés : la variable de a n'existe pas dans b.
    a.eval("var x = 42").unwrap();
    assert!(b.eval("x").is_err());
    assert_eq!(a.eval("x").unwrap(), "42");
}

#[test]
fn null_engine_noops() {
    let mut eng = NullEngine;
    assert_eq!(eng.eval("anything").unwrap(), "undefined");
    eng.set_budget(1);
    assert_eq!(eng.run_pending(), 0);
    eng.install_dom("<html></html>", "http://h.test/p", 1);
    assert_eq!(eng.sync_dom(), None);
}

#[test]
fn install_dom_exposes_document_global() {
    let mut eng = JscoreEngine::new();
    eng.install_dom(
        "<html><body><div id=\"x\">old</div></body></html>",
        "http://h.test/p",
        1,
    );
    // `document` installé → bindings visibles, pas de dirty.
    assert_eq!(eng.eval("typeof document").unwrap(), "object");
    assert_eq!(eng.sync_dom(), None);
}

#[test]
fn dom_mutation_sets_dirty_and_serializes() {
    let mut eng = JscoreEngine::new();
    eng.install_dom(
        "<html><body><div id=\"x\">old</div></body></html>",
        "http://h.test/p",
        1,
    );
    eng.eval("document.getElementById('x').textContent = 'NEUF'")
        .unwrap();
    let html = eng.sync_dom().expect("mutation attendue : dirty bit levé");
    assert!(html.contains("NEUF"), "sérialisation = {html}");
    assert!(!html.contains(">old<"), "sérialisation = {html}");
    // Dirty bit consommé : la synchro suivante ne revoit rien.
    assert_eq!(eng.sync_dom(), None);
}

#[test]
fn dom_untouched_stays_clean() {
    let mut eng = JscoreEngine::new();
    eng.install_dom(
        "<html><body><p>stable</p></body></html>",
        "http://h.test/p",
        1,
    );
    // Lecture seule (getElementById) → pas de mutation.
    eng.eval("document.getElementById('p')").unwrap();
    assert_eq!(eng.sync_dom(), None);
}

/// Chemin type produit par `element_path` (core) pour un div
/// dans body — wrappers `html`/`body` inclus.
fn path_to_div() -> Vec<(String, u32)> {
    vec![("html".into(), 0), ("body".into(), 0), ("div".into(), 0)]
}

#[test]
fn dispatch_reaches_mirror_listener() {
    let mut eng = JscoreEngine::new();
    eng.install_dom(
        "<html><body><div id=\"b\">x</div></body></html>",
        "http://h.test/p",
        1,
    );
    eng.eval(
        "document.getElementById('b').addEventListener('click',\
         function(){this.textContent='HIT'})",
    )
    .unwrap();
    let prevented = eng.dispatch_event("click", &path_to_div()).unwrap();
    assert!(!prevented);
    let html = eng.sync_dom().expect("listener a muté le miroir");
    assert!(html.contains("HIT"), "sérialisation = {html}");
}

#[test]
fn dispatch_skips_implicit_wrappers() {
    let mut eng = JscoreEngine::new();
    // Source SANS <html>/<body> : le miroir n'a que le div —
    // le chemin core (avec wrappers) doit quand même l'atteindre.
    eng.install_dom("<div id=\"b\">x</div>", "http://h.test/p", 1);
    eng.eval(
        "document.getElementById('b').addEventListener('click',\
         function(){this.textContent='HIT'})",
    )
    .unwrap();
    assert!(eng.dispatch_event("click", &path_to_div()).is_ok());
    assert!(eng.sync_dom().is_some_and(|h| h.contains("HIT")));
}

#[test]
fn dispatch_prevent_default_propagates() {
    let mut eng = JscoreEngine::new();
    eng.install_dom(
        "<html><body><div id=\"b\">x</div></body></html>",
        "http://h.test/p",
        1,
    );
    eng.eval(
        "document.getElementById('b').addEventListener('click',\
         function(e){e.preventDefault()})",
    )
    .unwrap();
    assert!(eng.dispatch_event("click", &path_to_div()).unwrap());
}

#[test]
fn dispatch_bad_path_is_noop() {
    let mut eng = JscoreEngine::new();
    eng.install_dom(
        "<html><body><div id=\"b\">x</div></body></html>",
        "http://h.test/p",
        1,
    );
    let bogus = vec![("html".into(), 0), ("body".into(), 0), ("table".into(), 0)];
    assert!(!eng.dispatch_event("click", &bogus).unwrap());
    assert_eq!(eng.sync_dom(), None);
}

#[test]
fn dispatch_without_mirror_is_noop() {
    let mut eng = JscoreEngine::new();
    assert!(!eng.dispatch_event("click", &path_to_div()).unwrap());
}

#[test]
fn nav_requests_drained() {
    let mut eng = JscoreEngine::new();
    eng.install_dom("<html></html>", "http://h.test/p", 3);
    eng.eval("location.href = '/next'; history.back(); history.go(2);")
        .unwrap();
    assert_eq!(
        eng.drain_nav_requests(),
        vec![
            explorer_js::NavRequest::Navigate("/next".to_string()),
            explorer_js::NavRequest::HistoryGo(-1),
            explorer_js::NavRequest::HistoryGo(2),
        ]
    );
    // File vidée ; un second drain ne re-rend rien.
    assert_eq!(eng.drain_nav_requests().len(), 0);
}

#[test]
fn location_reads_page_url() {
    let mut eng = JscoreEngine::new();
    eng.install_dom("<html></html>", "http://h.test/dir/p", 1);
    let out = eng.eval("location.pathname").unwrap();
    assert!(out.contains("/dir/p"), "got {out}");
    let out = eng.eval("history.length").unwrap();
    assert!(out.contains('1'), "got {out}");
}

// ---- JS-C27 : traverse proxy XHR/fetch/storage/cookie ----

/// `xhr.send()` n'exécute rien : la requête sort par
/// `drain_net_requests`, la réponse livrée remplit le wrapper
/// (`readyState` 4, `responseText`) et tire `onload`.
#[test]
fn xhr_roundtrips_through_net_drain() {
    let mut eng = JscoreEngine::new();
    eng.install_dom(
        "<html><body><div id=\"x\">avant</div></body></html>",
        "http://h.test/p",
        1,
    );
    eng.eval(
        "var x = new XMLHttpRequest();\
         x.onload = function(){document.getElementById('x').textContent=x.responseText};\
         x.open('POST', '/api');\
         x.setRequestHeader('X-A', '1');\
         x.send('corps')",
    )
    .unwrap();
    let reqs = eng.drain_net_requests();
    let explorer_js::NetRequest::Fetch {
        handle,
        method,
        url,
        headers,
        body,
        ..
    } = &reqs[0]
    else {
        panic!("attendu Fetch, obtenu {reqs:?}");
    };
    assert_eq!(method, "POST");
    assert_eq!(url, "/api");
    assert_eq!(headers, &[("X-A".to_string(), "1".to_string())]);
    assert_eq!(body.as_deref(), Some("corps"));

    eng.deliver_net_response(explorer_js::NetResponse {
        handle: *handle,
        status: 200,
        status_text: "OK".to_string(),
        headers: vec![],
        body: b"REPONSE".to_vec(),
    })
    .unwrap();
    assert_eq!(eng.eval("x.readyState").unwrap(), "4");
    assert_eq!(eng.eval("x.responseText").unwrap(), "REPONSE");
    // `onload` a tourné pendant la livraison → miroir muté.
    let html = eng.sync_dom().expect("onload a muté le miroir");
    assert!(html.contains("REPONSE"), "sérialisation = {html}");
}

/// `fetch(url)` rend une promesse : le handle sort par drain,
/// la livraison la résout (microtasks pompées au `run_pending`).
#[test]
fn fetch_promise_resolves_on_deliver() {
    let mut eng = JscoreEngine::new();
    eng.install_dom("<html></html>", "http://h.test/p", 1);
    eng.eval(
        "var res = 'rien';\
              fetch('/d').then(function(r){return r.text()})\
               .then(function(t){res=t})",
    )
    .unwrap();
    let reqs = eng.drain_net_requests();
    let explorer_js::NetRequest::Fetch { handle, .. } = reqs[0] else {
        panic!("attendu Fetch, obtenu {reqs:?}");
    };
    eng.deliver_net_response(explorer_js::NetResponse {
        handle,
        status: 200,
        status_text: "OK".to_string(),
        headers: vec![],
        body: b"donnees".to_vec(),
    })
    .unwrap();
    eng.run_pending();
    assert_eq!(eng.eval("res").unwrap(), "donnees");
}

/// `status == 0` = échec réseau : la promesse `fetch` est rejetée.
#[test]
fn fetch_network_error_rejects() {
    let mut eng = JscoreEngine::new();
    eng.install_dom("<html></html>", "http://h.test/p", 1);
    eng.eval(
        "var err = 'non';\
              fetch('/x').catch(function(){err='KO'})",
    )
    .unwrap();
    let reqs = eng.drain_net_requests();
    let explorer_js::NetRequest::Fetch { handle, .. } = reqs[0] else {
        panic!("attendu Fetch, obtenu {reqs:?}");
    };
    eng.deliver_net_response(explorer_js::NetResponse {
        handle,
        status: 0,
        status_text: String::new(),
        headers: vec![],
        body: vec![],
    })
    .unwrap();
    eng.run_pending();
    assert_eq!(eng.eval("err").unwrap(), "KO");
}

/// `xhr.abort()` sort un `Abort` pour le handle en vol.
#[test]
fn xhr_abort_is_drained() {
    let mut eng = JscoreEngine::new();
    eng.install_dom("<html></html>", "http://h.test/p", 1);
    eng.eval("var x = new XMLHttpRequest();x.open('GET','/a');x.send();x.abort()")
        .unwrap();
    let reqs = eng.drain_net_requests();
    assert_eq!(reqs.len(), 2);
    let explorer_js::NetRequest::Fetch { handle, .. } = reqs[0] else {
        panic!("attendu Fetch, obtenu {reqs:?}");
    };
    assert_eq!(reqs[1], explorer_js::NetRequest::Abort { handle });
    // Livrer quand même : handle inconnu → silencieusement ignoré.
    eng.deliver_net_response(explorer_js::NetResponse {
        handle,
        status: 200,
        status_text: "OK".to_string(),
        headers: vec![],
        body: vec![],
    })
    .unwrap();
}

/// `localStorage` : snapshot installé relu par drain, mutations
/// visibles ; `sessionStorage` est isolé (pas dans le drain).
#[test]
fn storage_snapshot_roundtrips() {
    let mut eng = JscoreEngine::new();
    eng.install_dom("<html></html>", "http://h.test/p", 1);
    eng.install_storage(std::collections::BTreeMap::from([(
        "k".to_string(),
        "v0".to_string(),
    )]));
    assert_eq!(eng.eval("localStorage.getItem('k')").unwrap(), "v0");
    eng.eval("localStorage.setItem('n', '1');sessionStorage.setItem('s', 'x')")
        .unwrap();
    let map = eng.drain_storage();
    assert_eq!(map.get("k"), Some(&"v0".to_string()));
    assert_eq!(map.get("n"), Some(&"1".to_string()));
    assert!(!map.contains_key("s"), "sessionStorage ne doit pas fuiter");
    // sessionStorage reste lisible dans le contexte.
    assert_eq!(eng.eval("sessionStorage.getItem('s')").unwrap(), "x");
}

/// `document.cookie` : snapshot posé → lecture ; écriture →
/// drainée brute ET visible en relecture (miroir).
#[test]
fn cookie_snapshot_and_writes_roundtrip() {
    let mut eng = JscoreEngine::new();
    eng.install_dom("<html></html>", "http://h.test/p", 1);
    eng.set_cookie_snapshot("a=1; b=2");
    let out = eng.eval("document.cookie").unwrap();
    assert!(out.contains("a=1") && out.contains("b=2"), "got {out}");
    eng.eval("document.cookie = 'c=3; Path=/; SameSite=Lax'")
        .unwrap();
    assert_eq!(
        eng.drain_cookie_writes(),
        vec!["c=3; Path=/; SameSite=Lax".to_string()]
    );
    // Lecture post-écriture : `c` apparaît dans la vue.
    assert!(eng.eval("document.cookie").unwrap().contains("c=3"));
    assert_eq!(eng.drain_cookie_writes(), Vec::<String>::new());
}

// ===== Étape 52 — borne timers + deadline + sync champ =====

/// `next_timer_deadline` : délai relatif jusqu'au prochain
/// timer armé ; `None` sans timer.
#[test]
fn next_timer_deadline_reports_delay() {
    let mut eng = JscoreEngine::new();
    // Les timers sont installés avec le miroir DOM.
    eng.install_dom("<html><body></body></html>", "http://h.test/p", 1);
    assert_eq!(eng.next_timer_deadline(), None);
    eng.eval("setTimeout(function(){}, 5000)").unwrap();
    let d = eng.next_timer_deadline().expect("timer armé");
    assert!(d <= std::time::Duration::from_millis(5000));
}

/// `run_pending_bounded` : borne de callbacks timer par pompe
/// — le surplus reste en file pour la pompe suivante.
#[test]
fn run_pending_bounded_caps_timer_jobs() {
    let mut eng = JscoreEngine::new();
    eng.install_dom("<html><body></body></html>", "http://h.test/p", 1);
    eng.eval(
        "var n=0; setTimeout(function(){n=n+1},0); \
         setTimeout(function(){n=n+1},0)",
    )
    .unwrap();
    assert_eq!(eng.run_pending_bounded(1).unwrap_or_default(), 1);
    assert_eq!(eng.eval("n").unwrap(), "1");
    assert!(eng.run_pending() >= 1);
    assert_eq!(eng.eval("n").unwrap(), "2");
}

/// Chemin vide = événement niveau document : un listener sur
/// `document.body` reçoit `scroll`/`keydown` (étape 52).
#[test]
fn dispatch_empty_path_targets_document() {
    let mut eng = JscoreEngine::new();
    eng.install_dom(
        "<html><body><div id=\"b\">x</div></body></html>",
        "http://h.test/p",
        1,
    );
    eng.eval(
        "document.body.addEventListener('scroll',\
         function(){document.getElementById('b').textContent='S'})",
    )
    .unwrap();
    eng.dispatch_event("scroll", &[]).unwrap();
    let html = eng.sync_dom().expect("listener a muté le miroir");
    assert!(html.contains("S</div>"), "got {html}");
}

/// `sync_mirror_attr` pousse un attribut hôte vers le miroir
/// — les handlers lisent l'état courant des champs édités
/// hors script (`value`, `checked`).
#[test]
fn sync_mirror_attr_writes_attribute() {
    let mut eng = JscoreEngine::new();
    eng.install_dom(
        "<html><body><input id=\"i\"></body></html>",
        "http://h.test/p",
        1,
    );
    let path = vec![
        ("html".to_string(), 0),
        ("body".to_string(), 0),
        ("input".to_string(), 0),
    ];
    eng.sync_mirror_attr(&path, "value", Some("abc"));
    let v = eng
        .eval("document.getElementById('i').getAttribute('value')")
        .unwrap();
    assert_eq!(v, "abc");
}

/// `<textarea>` : `value` pousse le contenu texte (pas un
/// attribut DOM).
#[test]
fn sync_mirror_attr_textarea_writes_text() {
    let mut eng = JscoreEngine::new();
    eng.install_dom(
        "<html><body><textarea id=\"t\"></textarea></body></html>",
        "http://h.test/p",
        1,
    );
    let path = vec![
        ("html".to_string(), 0),
        ("body".to_string(), 0),
        ("textarea".to_string(), 0),
    ];
    eng.sync_mirror_attr(&path, "value", Some("ligne"));
    let v = eng.eval("document.getElementById('t').value").unwrap();
    assert_eq!(v, "ligne");
}

/// Claim média : path obsolète (miroir divergent du DOM réel,
/// vol 176-ter) → repli sur l'URL de la surface. Un `<video>`
/// claimé reçoit `play()` → ordre `Play` routé sur l'id ;
/// non claimé, la promesse retombe sur le rejet stub ADR-0023
/// et rien ne se draine.
#[test]
fn media_claim_falls_back_to_url_when_path_diverges() {
    let mut eng = JscoreEngine::new();
    eng.install_dom(
        "<html><body><div><video src=\"blob:https://h.test/1\"></video></div></body></html>",
        "http://h.test/p",
        1,
    );
    let wrong = vec![
        ("html".to_string(), 0),
        ("body".to_string(), 0),
        ("div".to_string(), 7),
        ("video".to_string(), 0),
    ];
    eng.set_media_claim(
        &wrong,
        "blob:https://h.test/1",
        42,
        Some(vec!["avc1".into()]),
    );
    let _ = eng.eval("document.querySelector('video').play()");
    let cmds = eng.drain_media_commands();
    assert!(
        cmds.iter()
            .any(|c| c.media == 42 && c.cmd == explorer_js::JsMediaCmd::Play),
        "le claim par repli url doit livrer l'élément — {cmds:?}"
    );
}

/// Garde-fou : sans `src` concordante, le repli ne claim pas
/// un élément arbitraire (l'honnêteté ADR-0023 tient).
#[test]
fn media_claim_url_fallback_never_matches_wrong_element() {
    let mut eng = JscoreEngine::new();
    eng.install_dom(
        "<html><body><div><video src=\"blob:https://h.test/1\"></video></div></body></html>",
        "http://h.test/p",
        1,
    );
    let wrong = vec![
        ("html".to_string(), 0),
        ("body".to_string(), 0),
        ("div".to_string(), 7),
        ("video".to_string(), 0),
    ];
    eng.set_media_claim(
        &wrong,
        "blob:https://h.test/999",
        7,
        Some(vec!["avc1".into()]),
    );
    let _ = eng.eval("document.querySelector('video').play()");
    let cmds = eng.drain_media_commands();
    assert!(
        cmds.is_empty(),
        "aucun élément ne doit être claimé — {cmds:?}"
    );
}
