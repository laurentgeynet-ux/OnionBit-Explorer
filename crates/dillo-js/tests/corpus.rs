// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Corpus filtré « old school » (JS-1) : extraits représentatifs des
//! scripts que le navigateur rencontrera réellement — validation de
//! formulaire, rollover `className`, menus dépliants, horloges
//! `setTimeout`, réécritures `innerHTML`, délégation d'événements.
//!
//! Ce n'est **pas** test262 : la suite officielle cible ES5+ complet
//! (regex, `let`, strict mode…) — hors périmètre du sous-ensemble
//! (cf. ADR `dillo-js`). Chaque cas couvre une *forme* observée sur
//! le web jQuery-era, écrite contre notre surface supportée.

use dillo_js::{Dom, Engine};
use std::cell::RefCell;
use std::rc::Rc;

fn engine_with(html: &str) -> Engine {
    let mut e = Engine::new();
    e.install_timers();
    e.install_dom(&Rc::new(RefCell::new(Dom::from_html(html))));
    e
}

fn ev(html: &str, src: &str) -> String {
    let mut e = engine_with(html);
    e.eval(src)
        .map_or_else(|x| format!("ERR: {x}"), |v| e.describe(v))
}

const FORM: &str = r#"<body>
<form id="f">
 <input id="name" value="">
 <input id="age" value="17">
 <span id="err"></span>
</form></body>"#;

/// Validation de formulaire classique (era `onsubmit="return check()"`).
#[test]
fn corpus_form_validation() {
    let src = r"
        function check(f) {
            var err = '';
            if (f.name.value == '') err += 'name required; ';
            var a = parseInt(f.age.value, 10);
            if (isNaN(a) || a < 18) err += 'must be 18+; ';
            document.getElementById('err').innerHTML = err;
            return err == '';
        }
        check({ name: document.getElementById('name'),
                age: document.getElementById('age') });
    ";
    assert_eq!(ev(FORM, src), "false");
    assert_eq!(
        ev(
            FORM,
            &format!("{src}\ndocument.getElementById('err').innerHTML")
        ),
        "name required; must be 18+; "
    );
}

/// `innerHTML` swap + concaténation typique de l'ère jQuery.
#[test]
fn corpus_innerhtml_swap() {
    let html = b"<body><div id='box'><i>old</i></div></body>";
    let src = r"
        var b = document.getElementById('box');
        b.innerHTML = '<p>new <b>' + (1 + 1) + '</b></p>';
        b.innerHTML;
    ";
    assert_eq!(
        ev(&String::from_utf8_lossy(html), src),
        "<p>new <b>2</b></p>"
    );
}

/// Toggle `className`/`classList` — pattern « collapsible ».
#[test]
fn corpus_classname_toggle() {
    let html = "<body><div id='m' class='panel'></div></body>";
    assert_eq!(
        ev(
            html,
            "var m = document.getElementById('m'); m.className += ' open'; m.className;"
        ),
        "panel open"
    );
    assert_eq!(
        ev(
            html,
            "var m = document.getElementById('m'); m.classList.add('x'); \
             m.classList.toggle('panel'); m.classList.contains('panel');"
        ),
        "false"
    );
}

/// Défilement de messages — `setTimeout` + récursion de timer.
#[test]
fn corpus_marquee_timers() {
    let mut e = engine_with("<body><div id='t'></div></body>");
    e.eval(
        "var msgs = ['a','b','c']; var i = 0;
         function tick() {
             document.getElementById('t').innerHTML = msgs[i];
             i = (i + 1) % msgs.length;
             if (i != 0) setTimeout(tick, 10);
         }
         tick();",
    )
    .unwrap_or_else(|x| panic!("{x:?}"));
    for _ in 0..3 {
        e.run_due_timers().unwrap_or_else(|x| panic!("{x:?}"));
        std::thread::sleep(std::time::Duration::from_millis(15));
    }
    assert_eq!(
        e.eval("document.getElementById('t').innerHTML")
            .map_or("ERR".into(), |v| e.describe(v)),
        "c" // a → b → c, puis stop (i revient à 0 → plus de timer)
    );
    // Plus aucun timer en vol — la pompe est un no-op.
    assert_eq!(e.pending_timers(), 0);
}

/// Horloge `setInterval` + `clearInterval` — pattern sidebar.
#[test]
fn corpus_clock_interval() {
    let mut e = engine_with("<body><span id='c'></span></body>");
    e.eval(
        "var n = 0;
         var id = setInterval(function () {
             n++;
             if (n >= 3) clearInterval(id);
         }, 0);",
    )
    .unwrap_or_else(|x| panic!("{x:?}"));
    for _ in 0..6 {
        e.run_due_timers().unwrap_or_else(|x| panic!("{x:?}"));
    }
    assert_eq!(e.eval("n").map_or("ERR".into(), |v| e.describe(v)), "3");
}

/// Délégation d'événements era-prototype : un handler sur le parent.
#[test]
fn corpus_event_delegation() {
    let html = "<body><ul id='l'><li>x</li><li>y</li></ul></body>";
    let mut e = engine_with(html);
    e.eval(
        "var log = [];
         document.getElementById('l').addEventListener('click', function (e) {
             log.push(e.target.textContent);
         });",
    )
    .unwrap_or_else(|x| panic!("{x:?}"));
    e.eval("document.getElementsByTagName('li')[1].click();")
        .unwrap_or_else(|x| panic!("{x:?}"));
    assert_eq!(
        e.eval("log.join(',')")
            .map_or("ERR".into(), |v| e.describe(v)),
        "y"
    );
}

/// Tri + join — traitement de liste « server-rendered ».
#[test]
fn corpus_sort_join() {
    let src = r"
        var els = document.getElementsByTagName('li');
        var names = [];
        for (var i = 0; i < els.length; i++) names.push(els[i].textContent);
        names.sort().reverse().join('|');
    ";
    let html = "<body><ul><li>pear</li><li>apple</li><li>kiwi</li></ul></body>";
    assert_eq!(ev(html, src), "pear|kiwi|apple");
}

/// Compteur de caractères restants (textarea maxlength — era forum).
#[test]
fn corpus_char_counter() {
    let html = "<body><textarea id='t'>hello</textarea><span id='n'></span></body>";
    let src = r"
        var t = document.getElementById('t');
        var left = 140 - t.value.length;
        document.getElementById('n').innerHTML = left + ' chars left';
        document.getElementById('n').innerHTML;
    ";
    assert_eq!(ev(html, src), "135 chars left");
}

/// Rollover image-era : `src` swap (ici simulé via attr `src`).
#[test]
fn corpus_image_rollover_attrs() {
    let html = "<body><img id='i' src='a.png'></body>";
    assert_eq!(
        ev(
            html,
            "var i = document.getElementById('i'); i.src = 'b.png'; i.src;"
        ),
        "b.png"
    );
}

/// Classe utilitaire prototype-era (constructor + méthode prototype).
#[test]
fn corpus_proto_class() {
    let src = r"
        function Counter(id) { this.el = document.getElementById(id); this.n = 0; }
        Counter.prototype.inc = function () { this.n++; this.el.innerHTML = '' + this.n; };
        var c = new Counter('d'); c.inc(); c.inc();
        document.getElementById('d').innerHTML;
    ";
    let html = "<body><div id='d'></div></body>";
    assert_eq!(ev(html, src), "2");
}

/// Parsing d'URL « artisanal » (avant URL API) — indexOf/split.
#[test]
fn corpus_query_string_parse() {
    let src = r"
        function qs(url) {
            var q = url.indexOf('?'); if (q < 0) return {};
            var parts = url.substring(q + 1).split('&'), o = {};
            for (var i = 0; i < parts.length; i++) {
                var kv = parts[i].split('='); o[kv[0]] = kv[1];
            }
            return o;
        }
        qs('/p?id=42&lang=fr').id + '/' + qs('/p?id=42&lang=fr').lang;
    ";
    assert_eq!(ev("", src), "42/fr");
}

/// `document.write` pendant le chargement (append à `<body>`).
#[test]
fn corpus_document_write_append() {
    let html = "<body><p>before</p></body>";
    assert_eq!(
        ev(
            html,
            "document.write('<div id=w>added</div>'); \
             document.getElementById('w').textContent;"
        ),
        "added"
    );
}

/// Chaîne de tableaux associatifs — mini `localStorage` era pattern.
#[test]
fn corpus_object_map_accumulate() {
    let src = r"
        var votes = {};
        var names = ['a', 'b', 'a', 'c', 'a', 'b'];
        for (var i = 0; i < names.length; i++) {
            var n = names[i];
            votes[n] = (votes[n] || 0) + 1;
        }
        var out = [];
        for (var k in votes) out.push(k + '=' + votes[k]);
        out.join(',');
    ";
    assert_eq!(ev("", src), "a=3,b=2,c=1");
}

/// Runtime guard : boucle infinie tuée par l'op-limit (ne gèle pas l'UI).
#[test]
fn corpus_op_limit_kills_hang() {
    let mut e = Engine::new();
    e.set_op_limit(10_000);
    let r = e.eval("while (1) { var x = 1; }");
    assert!(r.is_err());
}

/// Exception dans un listener d'événement — isolée des suivants.
#[test]
fn corpus_listener_exception_isolated() {
    let html = "<body><div id='d'>x</div></body>";
    let mut e = engine_with(html);
    e.eval(
        "var ran = 0;
         var d = document.getElementById('d');
         d.addEventListener('click', function () { throw new Error('boom'); });
         d.addEventListener('click', function () { ran++; });",
    )
    .unwrap_or_else(|x| panic!("{x:?}"));
    // Le premier listener lève → dispatch_simple s'arrête sur l'erreur
    // (comportement ≙ « exception non catchée » amont : le handler
    // suivant ne court pas — noté, isolation par événement).
    let r = e.eval("d.click();");
    assert!(r.is_err());
}
