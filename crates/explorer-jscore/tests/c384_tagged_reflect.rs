// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! JS-C384 / §B-103 — reflets d'attributs **tag-scopés** :
//! la table plate historique rendait `'media' in div`,
//! `'srcset' in el`, `'checked' in div`… vrais sur tout
//! élément → la détection feature des bundles (kevlar)
//! prenait la mauvaise branche → return silencieux →
//! `.then` jamais enfilé — suspect mécanisme §B-97.
//! Spec IDL `reflect` : le reflet n'existe que sur les
//! classes qui le déclarent ; sur un mauvais tag, lecture
//! `undefined`, `in` → `false`, écriture → prop propre
//! (jamais d'attribut fantôme).

use explorer_jscore::{Dom, Engine};
use std::cell::RefCell;
use std::rc::Rc;

const PAGE: &str = r#"<html><body><div id="host"></div></body></html>"#;

fn engine() -> Engine {
    let mut e = Engine::new();
    e.install_dom(&Rc::new(RefCell::new(Dom::from_html(PAGE))));
    e
}

fn ev(e: &mut Engine, src: &str) -> String {
    match e.eval(src) {
        Ok(v) => e.describe(v),
        Err(err) => panic!("{src} -> {err}"),
    }
}

/// Les reflets hors-tag rendent `false` au `in` (le poison
/// §B-103) — les reflets en-tag restent vrais ; les globaux
/// restent globaux.
#[test]
fn reflected_attrs_false_off_tag() {
    let mut e = engine();
    // Poison §B-103 — faux sur mauvais tag.
    for key in [
        "media",
        "srcset",
        "sizes",
        "srcdoc",
        "sandbox",
        "poster",
        "preload",
        "placeholder",
        "alt",
        "href",
        "src",
        "checked",
        "controls",
        "autoplay",
        "selected",
        "multiple",
        "readonly",
        "name",
        "value",
        "type",
        "rel",
        "target",
        "action",
        "method",
        "htmlFor",
        "crossOrigin",
        "playsInline",
        "referrerPolicy",
    ] {
        assert_eq!(
            ev(&mut e, &format!("'{key}' in document.createElement('div')")),
            "false",
            "'{key}' in div"
        );
    }
}

/// Les reflets en-tag restent vrais ; les globaux restent
/// globaux sur n'importe quel élément.
#[test]
fn reflected_attrs_true_on_tag_and_globals() {
    let mut e = engine();
    // En-tag : vrais.
    for (tag, key) in [
        ("link", "media"),
        ("style", "media"),
        ("source", "media"),
        ("img", "srcset"),
        ("source", "srcset"),
        ("img", "sizes"),
        ("iframe", "srcdoc"),
        ("iframe", "sandbox"),
        ("video", "poster"),
        ("video", "preload"),
        ("audio", "preload"),
        ("video", "controls"),
        ("audio", "autoplay"),
        ("input", "placeholder"),
        ("textarea", "placeholder"),
        ("img", "alt"),
        ("input", "checked"),
        ("option", "selected"),
        ("select", "multiple"),
        ("input", "readonly"),
        ("a", "href"),
        ("link", "href"),
        ("img", "src"),
        ("script", "src"),
        ("form", "action"),
        ("form", "method"),
        ("a", "target"),
        ("a", "rel"),
        ("input", "name"),
        ("form", "name"),
        ("input", "value"),
        ("textarea", "value"),
        ("button", "type"),
        ("input", "type"),
        ("label", "htmlFor"),
        ("img", "crossOrigin"),
        ("video", "playsInline"),
        ("img", "referrerPolicy"),
        ("input", "disabled"),
        ("fieldset", "disabled"),
    ] {
        assert_eq!(
            ev(
                &mut e,
                &format!("'{key}' in document.createElement('{tag}')")
            ),
            "true",
            "'{key}' in {tag}"
        );
    }
    // Globaux — sur n'importe quel tag.
    for key in ["id", "className", "title", "lang", "dir", "slot"] {
        assert_eq!(
            ev(&mut e, &format!("'{key}' in document.createElement('div')")),
            "true",
            "'{key}' in div (global)"
        );
        assert_eq!(
            ev(
                &mut e,
                &format!("'{key}' in document.createElement('section')")
            ),
            "true",
            "'{key}' in section (global)"
        );
    }
}

/// Lecture hors-tag : `undefined`, pas `""` ; écriture
/// hors-tag : prop propre sans attribut fantôme ; écriture
/// en-tag : toujours le reflet d'attribut.
#[test]
fn tagged_reflect_read_write_semantics() {
    let mut e = engine();
    assert_eq!(
        ev(&mut e, "document.createElement('div').media"),
        "undefined"
    );
    assert_eq!(
        ev(&mut e, "document.createElement('div').poster"),
        "undefined"
    );
    // `div.poster = 'x'` → prop propre (relue), `getAttribute`
    // ne voit rien — spec : pas de reflet = pas d'attribut.
    assert_eq!(
        ev(
            &mut e,
            "var d = document.createElement('div'); d.poster = 'x'; d.getAttribute('poster')"
        ),
        "null"
    );
    assert_eq!(
        ev(
            &mut e,
            "var d2 = document.createElement('div'); d2.poster = 'x'; d2.poster"
        ),
        "x"
    );
    // En-tag : le reflet écrit l'attribut.
    assert_eq!(
        ev(
            &mut e,
            "var l = document.createElement('link'); l.media = 'print'; l.getAttribute('media')"
        ),
        "print"
    );
    assert_eq!(
        ev(
            &mut e,
            "var i = document.createElement('iframe'); i.sandbox = 'allow-scripts'; i.getAttribute('sandbox')"
        ),
        "allow-scripts"
    );
    // Booléen hors-tag : `undefined` ; en-tag : `false` (attr
    // absent) puis `true` après écriture.
    assert_eq!(
        ev(&mut e, "document.createElement('div').controls"),
        "undefined"
    );
    assert_eq!(
        ev(&mut e, "document.createElement('video').controls"),
        "false"
    );
    assert_eq!(
        ev(
            &mut e,
            "var v = document.createElement('video'); v.controls = true; v.controls"
        ),
        "true"
    );
}

/// §B-107/JS-C385 — les méthodes tag-scopées sont **lisibles**
/// (`typeof`/`in`) comme un navigateur : `canvas.getContext`,
/// `form.submit`, `dialog.showModal`, `img.decode`… étaient
/// invoke-only → `undefined` → chart.js ratant son
/// feature-detect. Hors-tag elles restent `undefined`.
#[test]
fn tagged_methods_are_readable_on_tag_only() {
    let mut e = engine();
    // En-tag : `'function'` + `in` vrai.
    for (tag, m) in [
        ("canvas", "getContext"),
        ("canvas", "toDataURL"),
        ("canvas", "toBlob"),
        ("form", "submit"),
        ("form", "requestSubmit"),
        ("form", "reset"),
        ("form", "checkValidity"),
        ("form", "reportValidity"),
        ("form", "setCustomValidity"),
        ("input", "checkValidity"),
        ("input", "select"),
        ("input", "setSelectionRange"),
        ("textarea", "setRangeText"),
        ("dialog", "showModal"),
        ("dialog", "show"),
        ("dialog", "close"),
        ("img", "decode"),
        ("slot", "assignedElements"),
        ("slot", "assignedNodes"),
        ("button", "checkValidity"),
        ("fieldset", "checkValidity"),
        ("select", "checkValidity"),
    ] {
        assert_eq!(
            ev(
                &mut e,
                &format!("typeof document.createElement('{tag}').{m}")
            ),
            "function",
            "typeof {tag}.{m}"
        );
        assert_eq!(
            ev(&mut e, &format!("'{m}' in document.createElement('{tag}')")),
            "true",
            "'{m}' in {tag}"
        );
    }
    // Hors-tag : `undefined` + `in` faux (le poison §B-107).
    for m in [
        "getContext",
        "toDataURL",
        "toBlob",
        "submit",
        "requestSubmit",
        "checkValidity",
        "showModal",
        "decode",
        "assignedElements",
    ] {
        assert_eq!(
            ev(&mut e, &format!("typeof document.createElement('div').{m}")),
            "undefined",
            "typeof div.{m}"
        );
        assert_eq!(
            ev(&mut e, &format!("'{m}' in document.createElement('div')")),
            "false",
            "'{m}' in div"
        );
    }
    // L'appel reste sain : `getContext('2d')` rend un objet
    // contexte, `checkValidity` un booléen.
    assert_eq!(
        ev(
            &mut e,
            "typeof document.createElement('canvas').getContext('2d')"
        ),
        "object"
    );
    assert_eq!(
        ev(&mut e, "document.createElement('form').checkValidity()"),
        "true"
    );
    // Garde JS-C341 : une prop auteur shadowe le trampoline.
    assert_eq!(
        ev(
            &mut e,
            "var c = document.createElement('canvas'); c.getContext = 42; c.getContext"
        ),
        "42"
    );
}

/// JS-C387 / rail `fonts.load` §B-97 — transitions spec de
/// `FontFace`/`FontFaceSet` : `status` bascule
/// `'unloaded'→'loaded'` (un gate `f.status === 'loaded'`
/// post-await lisait `unloaded` à jamais) et le set tire
/// `loading`/`loadingdone` en **microtask** — le listener posé
/// avant OU après `load()` dans le même tour voit l'événement
/// (dispatch synchrone manquerait le second motif).
#[test]
fn fontface_status_transitions_and_set_events_fire() {
    let mut e = engine();
    assert_eq!(
        ev(&mut e, "var f = new FontFace('K', 'url(x)'); f.status"),
        "unloaded"
    );
    assert_eq!(ev(&mut e, "f.load(); f.status"), "loaded");
    // `add()` charge la face posée (`status` → 'loaded') et
    // tire `loading`+`loadingdone` avec `fontfaces: [face]`.
    assert_eq!(
        ev(
            &mut e,
            "var hits = []; document.fonts.addEventListener('loadingdone', (e2) => hits.push(e2.type + ':' + e2.fontfaces.length));
             var g = new FontFace('G', 'url(y)');
             document.fonts.add(g); g.status"
        ),
        "loaded"
    );
    assert_eq!(ev(&mut e, "hits.join(',')"), "loadingdone:1");
    // `fonts.load()` : `loading` puis `loadingdone` en file —
    // une lecture synchrone verrait `seq` vide (les events sont
    // des tâches, spec) ; la suite relit dans l'eval suivant,
    // après le drain de fin d'eval.
    ev(
        &mut e,
        "var seq = []; document.fonts.addEventListener('loading', (e3) => seq.push(e3.type));
         document.fonts.addEventListener('loadingdone', (e4) => seq.push(e4.type));
         document.fonts.load('12px K'); 'queued'",
    );
    assert_eq!(ev(&mut e, "seq.join(',')"), "loading,loadingdone");
    // Listener posé APRÈS `load()` dans le même tour — la
    // microtask le voit quand même ; un dispatch synchrone
    // l'aurait manqué à jamais (le discriminant du fix).
    ev(
        &mut e,
        "var late = []; document.fonts.load('9px L');
         document.fonts.addEventListener('loadingdone', () => late.push('seen'));
         'armed'",
    );
    assert_eq!(ev(&mut e, "late.join(',')"), "seen");
}

/// §B-106 — repro synthétique du bootstrap **webpack JSONP**
/// (twitch : 1019 modules enregistrés, `prom.created=1`,
/// entrée jamais invoquée) : chunk données posé avant le
/// runtime, hook `push` + replay buffer, `checkDeferredModules`
/// qui n'invoque l'entrée que quand tous les chunks requis
/// sont installés. Verrou de la sémantique moteur : si ce
/// pattern tourne, le gate twitch est timing/données, pas
/// interpréteur.
#[test]
fn webpack_jsonp_bootstrap_entry_invokes() {
    let mut e = engine();
    assert_eq!(
        ev(
            &mut e,
            r#"
            var g = typeof self !== 'undefined' ? self : this;
            g.wpC = g.wpC || [];
            var cg = g.wpC;
            // Chunk données posé AVANT le runtime (ordre libre).
            cg.push([["data1"], {"m_d": function() { return 7; }}]);
            // --- runtime : hook push + deferred check ---
            var entryRan = [];
            var installed = {}, mods = {};
            var deferred = [["entry", ["data1", "rt", "entryC"]]];
            var require = function(id) { mods[id](); };
            cg.push = function(res) {
                res[0].forEach(function(id) { installed[id] = true; });
                for (var k in res[1]) mods[k] = res[1][k];
                // checkDeferredModules : déps posées → module
                // invoqué UNE fois, deferred consommé (splice).
                for (var i = deferred.length - 1; i >= 0; i--) {
                    var dm = deferred[i], ok = true;
                    for (var j = 0; j < dm[1].length; j++) {
                        if (!installed[dm[1][j]]) ok = false;
                    }
                    if (ok) { deferred.splice(i, 1); require(dm[0]); }
                }
            };
            installed["rt"] = true;
            // Replay des pushes bufferisés (vrai webpack le fait).
            for (var i = 0; i < cg.length; i++) cg.push.call(cg, cg[i]);
            // Chunk d'entrée posé après le runtime.
            cg.push([["entryC"], {"entry": function() { entryRan.push("entry-exec"); }}]);
            entryRan.join(',')
            "#
        ),
        "entry-exec"
    );
}
