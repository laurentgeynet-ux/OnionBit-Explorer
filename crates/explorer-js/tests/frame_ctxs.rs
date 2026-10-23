// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Contextes de navigation imbriqués derrière le trait
//! `JsEngine` (étape 134 — JS-C78 au-dessus du canal) :
//! `context_new`/`install_dom_ctx`/`eval_ctx`/
//! `dispatch_event_ctx`/`sync_dom_ctx`/`context_destroy`
//! traversent le proxy `JscoreEngine` sans jamais laisser
//! un `Rc` sortir du thread moteur.

use explorer_js::{JsCtxId, JsEngine, JsError, JscoreEngine, NetRequest};

/// Document top-level à un `<iframe>`.
const TOP_HTML: &str = "<html><body>\
    <div id=\"p\">parent</div><iframe src=\"http://h.test/f\"></iframe>\
    </body></html>";

/// Document enfant posé dans son contexte.
const CHILD_HTML: &str = "<html><body><p id=\"c\">enfant</p></body></html>";

/// Moteur avec le document parent installé.
fn engine_with_parent() -> JscoreEngine {
    let mut eng = JscoreEngine::new();
    eng.install_dom(TOP_HTML, "http://h.test/", 1);
    eng
}

/// Cadre câblé : `context_new` + `install_dom_ctx`. Renvoie
/// `(moteur, ctx)`.
fn wired_frame(eng: &mut JscoreEngine) -> JsCtxId {
    let ctx = eng
        .context_new("http://h.test/f", Some(0))
        .expect("contexte créé");
    eng.install_dom_ctx(
        ctx,
        CHILD_HTML,
        "http://h.test/f",
        std::collections::BTreeMap::new(),
        "",
    );
    ctx
}

/// `context_new` alloue un realm — l'écriture `eval_ctx` y
/// est visible, celle du parent non ; le contexte courant
/// est restauré après l'appel (le eval suivant court en
/// top-level).
#[test]
fn context_new_isolates_globals() {
    let mut eng = engine_with_parent();
    let ctx = wired_frame(&mut eng);
    assert_ne!(ctx, 0, "un enfant n'est jamais le top-level");
    assert_eq!(
        eng.eval_ctx(ctx, "var marque = 'enfant'; marque", None)
            .unwrap(),
        "enfant"
    );
    // Le contexte courant après `context_new` est le parent —
    // `eval` nu ne touche pas le realm enfant.
    assert_eq!(
        eng.eval("typeof marque").unwrap(),
        "undefined",
        "le global de l'enfant n'a pas fuité"
    );
    // Et réciproquement.
    eng.eval("var marque = 'parent'").unwrap();
    assert_eq!(
        eng.eval_ctx(ctx, "typeof marque === 'string' ? marque : 'absent'", None)
            .unwrap(),
        "enfant",
        "le realm enfant garde son propre global"
    );
}

/// `install_dom_ctx` installe le DOM **de l'enfant** :
/// `document` du cadre voit son `<p id=c>`, le parent garde
/// le sien — le miroir `jsd` du top-level n'est pas écrasé.
#[test]
fn context_dom_is_the_childs() {
    let mut eng = engine_with_parent();
    let ctx = wired_frame(&mut eng);
    assert_eq!(
        eng.eval_ctx(ctx, "document.getElementById('c').textContent", None)
            .unwrap(),
        "enfant"
    );
    // Côté parent : le nœud enfant est invisible, le sien
    // toujours là.
    assert_eq!(
        eng.eval("document.getElementById('p').textContent")
            .unwrap(),
        "parent"
    );
    assert_eq!(
        eng.eval("document.getElementById('c')").unwrap(),
        "null",
        "pas de miroir croisé"
    );
}

/// `location` du cadre pointe son URL, pas celle du parent
/// — l'initiateur vu par les scripts est l'enfant.
#[test]
fn context_location_is_child_url() {
    let mut eng = engine_with_parent();
    let ctx = wired_frame(&mut eng);
    assert_eq!(
        eng.eval_ctx(ctx, "location.href", None).unwrap(),
        "http://h.test/f"
    );
    assert_eq!(eng.eval("location.href").unwrap(), "http://h.test/");
}

/// `fetch` émis dans le realm enfant remonte un
/// `NetRequest::Fetch` **tagué `ctx`** — la session en
/// déduit l'initiateur/base de l'enfant.
#[test]
fn context_fetch_carries_ctx_tag() {
    let mut eng = engine_with_parent();
    let ctx = wired_frame(&mut eng);
    eng.eval_ctx(ctx, "fetch('/res').then(function(){})", None)
        .unwrap();
    let reqs = eng.drain_net_requests();
    let [req] = reqs.as_slice() else {
        panic!("une requête attendue, obtenu {reqs:?}");
    };
    match req {
        NetRequest::Fetch { url, ctx: got, .. } => {
            assert_eq!(url, "/res");
            assert_eq!(*got, ctx, "tag ctx = émetteur enfant");
        }
        other => panic!("Fetch attendu, obtenu {other:?}"),
    }
    // Requête top-level ensuite : `ctx = 0`, aucune
    // confusion d'initiateur.
    eng.eval("fetch('/top')").unwrap();
    let reqs = eng.drain_net_requests();
    match reqs.as_slice() {
        [NetRequest::Fetch { ctx: 0, .. }] => {}
        other => panic!("Fetch ctx=0 attendu, obtenu {other:?}"),
    }
}

/// Les handles sont uniques **entre** contextes (compteur
/// partagé) — sinon un `Abort` top-level pourrait tuer la
/// tâche d'un cadre.
#[test]
fn context_handles_are_unique_across_ctxs() {
    let mut eng = engine_with_parent();
    let ctx = wired_frame(&mut eng);
    eng.eval("fetch('/a')").unwrap();
    eng.eval_ctx(ctx, "fetch('/b')", None).unwrap();
    let reqs = eng.drain_net_requests();
    let mut handles = Vec::new();
    for r in &reqs {
        if let NetRequest::Fetch { handle, .. } = r {
            handles.push(*handle);
        }
    }
    assert_eq!(handles.len(), 2);
    assert_ne!(handles[0], handles[1], "handles uniques cross-ctx");
}

/// `dispatch_event_ctx` vise le document de l'enfant — un
/// listener `load` posé dans son realm tire ; la mutation
/// de SON miroir remonte via `sync_dom_ctx`.
#[test]
fn context_event_and_sync() {
    let mut eng = engine_with_parent();
    let ctx = wired_frame(&mut eng);
    eng.eval_ctx(
        ctx,
        "document.addEventListener('load', function(){ \
             document.getElementById('c').textContent = 'charge'; })",
        None,
    )
    .unwrap();
    let prevented = eng.dispatch_event_ctx(ctx, "load", &[]).unwrap();
    assert!(!prevented, "pas de preventDefault");
    let html = eng
        .sync_dom_ctx(ctx)
        .expect("miroir enfant muté → sérialisation");
    assert!(html.contains("charge"), "{html}");
    // Parent intact.
    assert_eq!(eng.sync_dom(), None, "le miroir parent n'a pas bougé");
}

/// `context_destroy` tue le realm : `eval_ctx` répond
/// `Dead`, le miroir enfant disparaît de la table —
/// le parent survit intact.
#[test]
fn context_destroy_kills_realm() {
    let mut eng = engine_with_parent();
    let ctx = wired_frame(&mut eng);
    eng.eval_ctx(ctx, "var vivant = 1", None).unwrap();
    eng.context_destroy(ctx);
    match eng.eval_ctx(ctx, "vivant", None) {
        Err(JsError::Dead) => {}
        other => panic!("contexte mort → Dead attendu, obtenu {other:?}"),
    }
    assert_eq!(eng.sync_dom_ctx(ctx), None, "miroir détruit");
    // `frames[n]` du parent rétrécit : l'enfant détruit
    // n'est plus résolvable.
    assert_eq!(eng.eval("frames.length").unwrap(), "0");
    eng.eval("var toujours = 'la'").unwrap();
    assert_eq!(eng.eval("toujours").unwrap(), "la");
}

/// `contentWindow` relie l'élément `<iframe>` du parent au
/// realm enfant (même origine → vrai handle) — la
/// correspondance passe par `frame_ord` → nœud miroir.
#[test]
fn context_content_window_bridges() {
    let mut eng = engine_with_parent();
    let ctx = wired_frame(&mut eng);
    eng.eval_ctx(ctx, "var cote = 'enfant'", None).unwrap();
    assert_eq!(
        eng.eval("document.querySelector('iframe').contentWindow.cote")
            .unwrap(),
        "enfant",
        "contentWindow = global de l'enfant (same-origin)"
    );
    assert_eq!(
        eng.eval("parent === window.top ? 'top' : 'non'").unwrap(),
        "top"
    );
}

/// Un contexte inconnu (id jamais alloué ou détruit)
/// dégrade : `Dead`, pas de crash, pas d'évaluation dans
/// un autre realm.
#[test]
fn context_unknown_id_degrades() {
    let mut eng = engine_with_parent();
    match eng.eval_ctx(99, "1+1", None) {
        Err(JsError::Dead) => {}
        other => panic!("ctx inconnu → Dead, obtenu {other:?}"),
    }
    // Le top-level reste sain.
    assert_eq!(eng.eval("1+1").unwrap(), "2");
}
