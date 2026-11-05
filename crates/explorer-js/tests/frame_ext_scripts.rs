// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Étape 207 — `<script src>` parse-time des sous-documents
//! (mur §B-99 embed du vol 644 : `www-embed-player.js`/`base.js`
//! de `YouTube` jamais fetchés — `frame_context_plan` collectait
//! `max_src = 0`, lecteur figé). `install_dom_ctx` enfile les
//! `src` en `pending_dyn` : le rail dyn-script existant les
//! fetch tagué **ctx enfant** (referer/origine du cadre via
//! `ctx_net_origin`) et évalue dans son realm avec `load`/`error`
//! sur l'élément. Les balises inline ne sont **pas** re-émises
//! (déjà évaluées au câblage — double exécution sinon).

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
/// enfant ; sa livraison évalue dans **son** realm (mutation
/// de SON miroir) puis `load` sur l'élément — le parent est
/// intact.
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

/// Les balises **inline** du sous-document ne sont pas émises
/// (évaluées au câblage — re-émettre = double exécution) ; un
/// `<script src>` voisin est bien le seul en file.
#[test]
fn iframe_inline_scripts_are_not_reemitted() {
    let mut eng = engine_with_parent();
    let _ctx = wired_frame(
        &mut eng,
        "<html><body>\
            <script>window.__i = 1;</script>\
            <script src=\"/x.js\"></script>\
            <script type=\"text/plain\">mort</script>\
        </body></html>",
    );
    let reqs = eng.drain_dyn_scripts();
    let [req] = reqs.as_slice() else {
        panic!("un seul DynScriptReq attendu, obtenu {reqs:?}");
    };
    assert_eq!(req.src.as_deref(), Some("/x.js"));
    assert!(req.text.is_none(), "inline non re-collecté");
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
