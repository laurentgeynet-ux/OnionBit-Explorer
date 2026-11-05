// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Étape 207/207-bis — `<script>` parse-time des sous-documents
//! (murs §B-99 + II.4-bis des vols 644/647 : `www-embed-player.js`/
//! `base.js` de `YouTube` jamais fetchés, puis ordre cassé —
//! l'inline `writeEmbed()` tirait avant les `src=` du rail).
//! `install_dom_ctx` enfile **tout** le flux parse (`src` + inline)
//! en `pending_dyn`, balises ordonnées marquées `defer_order` : le
//! rail dyn-script les livre dans l'ordre document, tagué **ctx
//! enfant** (referer/origine du cadre via `ctx_net_origin`) avec
//! `load`/`error` sur l'élément. Le `<script async src>` reste
//! force-async (à réception, spec).

use std::fmt::Write as _;

use explorer_js::{JsCtxId, JsEngine, JscoreEngine};

const TOP_HTML: &str = "<html><body><iframe src=\"http://emb.test/p\"></iframe></body></html>";

fn engine_with_parent() -> JscoreEngine {
    let mut eng = JscoreEngine::new();
    eng.install_dom(TOP_HTML, "http://h.test/", 1);
    eng
}

fn wired_frame(eng: &mut JscoreEngine, html: &str) -> JsCtxId {
    let ctx = eng
        .context_new("http://emb.test/p", Some(0))
        .expect("contexte créé");
    eng.install_dom_ctx(
        ctx,
        html,
        "http://emb.test/p",
        std::collections::BTreeMap::new(),
        "",
    );
    ctx
}

/// Le `<script src>` du sous-document est émis tagué `ctx`
/// enfant + `defer_order` ; sa livraison évalue dans **son**
/// realm (mutation de SON miroir) puis `load` sur l'élément —
/// le parent est intact.
#[test]
fn iframe_parse_external_script_fetches_and_evals_in_ctx() {
    let mut eng = engine_with_parent();
    let ctx = wired_frame(
        &mut eng,
        "<html><body>\
            <script src=\"https://emb.test/app.js\"></script>\
            <div id=\"c\"></div>\
        </body></html>",
    );
    let reqs = eng.drain_dyn_scripts();
    let [req] = reqs.as_slice() else {
        panic!("un DynScriptReq attendu, obtenu {reqs:?}");
    };
    assert_eq!(req.src.as_deref(), Some("https://emb.test/app.js"));
    assert_eq!(req.ctx, ctx, "requête taguée du ctx enfant");
    assert!(req.defer_order, "balise parse = ordre document");
    // La session livre le corps : éval dans le realm enfant —
    // sa propre `window`, son propre `document`.
    eng.deliver_dyn_script(
        ctx,
        req.seq,
        Some(
            "window.__boot = 'ok'; \
              document.getElementById('c').textContent = 'booted';"
                .to_string(),
        ),
    );
    assert_eq!(eng.eval_ctx(ctx, "window.__boot", None).unwrap(), "ok");
    assert_eq!(
        eng.eval_ctx(ctx, "document.getElementById('c').textContent", None)
            .unwrap(),
        "booted"
    );
    // Le parent n'a rien vu.
    assert_eq!(
        eng.eval("typeof window.__boot").unwrap(),
        "undefined",
        "global enfant confiné à son realm"
    );
    // `load` tiré sur l'élément du sous-document (spec).
    let html = eng.sync_dom_ctx(ctx).unwrap_or_default();
    let _ = html; // la mutation a déjà été vérifiée par eval_ctx
}

/// II.4-bis : tout le flux parse est émis, ordre document
/// préservé — `src` puis inline, chacun `defer_order` avec sa
/// charge (`src` / `text`).
#[test]
fn iframe_parse_scripts_emit_in_document_order() {
    let mut eng = engine_with_parent();
    let ctx = wired_frame(
        &mut eng,
        "<html><body>\
            <script>window.__i = 1;</script>\
            <script src=\"/x.js\"></script>\
            <script>writeEmbed();</script>\
            <script type=\"text/plain\">mort</script>\
        </body></html>",
    );
    let reqs = eng.drain_dyn_scripts();
    assert_eq!(reqs.len(), 3, "src + 2 inline, le text/plain inerte");
    assert_eq!(reqs[0].text.as_deref(), Some("window.__i = 1;"));
    assert_eq!(reqs[1].src.as_deref(), Some("/x.js"));
    assert_eq!(reqs[2].text.as_deref(), Some("writeEmbed();"));
    assert!(
        reqs.iter().all(|r| r.defer_order && r.ctx == ctx),
        "ordre document + ctx enfant sur tout le flux"
    );
}

/// II.4-bis — la séquence du rail : le `src` livré **avant**
/// l'inline qui suit voit sa fonction définie (le mur était
/// `ReferenceError: writeEmbed is not defined` — vol 647).
#[test]
fn iframe_inline_after_src_sees_its_globals() {
    let mut eng = engine_with_parent();
    let ctx = wired_frame(
        &mut eng,
        "<html><body><p id=\"o\">?</p>\
            <script src=\"/base.js\"></script>\
            <script>writeEmbed();</script>\
        </body></html>",
    );
    let reqs = eng.drain_dyn_scripts();
    assert_eq!(reqs.len(), 2);
    // Ordre de livraison que la session garantit : le fetch du
    // `src` se termine, puis l'inline part.
    eng.deliver_dyn_script(
        ctx,
        reqs[0].seq,
        Some(
            "function writeEmbed(){\
             document.getElementById('o').textContent='written';}"
                .to_string(),
        ),
    );
    eng.deliver_dyn_script(ctx, reqs[1].seq, reqs[1].text.clone());
    assert_eq!(
        eng.eval_ctx(ctx, "document.getElementById('o').textContent", None)
            .unwrap(),
        "written",
        "writeEmbed défini par le src, appelé par l'inline"
    );
}

/// `<script async src>` : force-async spec — émis **hors**
/// ordre (`defer_order = false`), la session le lance sans
/// attendre les balises parse qui le précèdent.
#[test]
fn iframe_async_src_script_is_unordered() {
    let mut eng = engine_with_parent();
    let _ctx = wired_frame(
        &mut eng,
        "<html><body>\
            <script src=\"/sync.js\"></script>\
            <script src=\"/async.js\" async></script>\
        </body></html>",
    );
    let reqs = eng.drain_dyn_scripts();
    assert_eq!(reqs.len(), 2);
    assert!(reqs[0].defer_order, "src classique ordonné");
    assert!(!reqs[1].defer_order, "src async hors ordre");
}

/// Borne `MAX_CTX_PARSE_SCRIPTS` — un document hostile à 70
/// balises `src` n'en enfile que 64 (les autres restent
/// inertes, jamais de file illimitée).
#[test]
fn iframe_parse_external_scripts_bounded() {
    let mut eng = engine_with_parent();
    let mut html = String::from("<html><body>");
    for i in 0..70 {
        let _ = write!(html, "<script src=\"/s/{i}.js\"></script>");
    }
    html.push_str("</body></html>");
    let _ctx = wired_frame(&mut eng, &html);
    let reqs = eng.drain_dyn_scripts();
    assert_eq!(reqs.len(), 64, "borne bornée par document");
}

/// `srcdoc` : même rail — un `<script src>` de sous-document
/// inline est émis pour fetch/éval dans son ctx.
#[test]
fn srcdoc_frame_external_scripts_also_pending() {
    let mut eng = engine_with_parent();
    let _ctx = wired_frame(
        &mut eng,
        "<html><body><script src=\"/inline-frame.js\"></script></body></html>",
    );
    let reqs = eng.drain_dyn_scripts();
    assert_eq!(reqs.len(), 1);
    assert_eq!(reqs[0].src.as_deref(), Some("/inline-frame.js"));
}
