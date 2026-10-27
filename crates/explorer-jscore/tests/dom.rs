// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Tests for the standalone mini-DOM façade (JS-2): `document`,
//! elements, `innerHTML`, style/classList, events, `document.write`.

use explorer_jscore::{Dom, Engine};
use std::cell::RefCell;
use std::rc::Rc;

const PAGE: &str = r#"
<html><head><title>Test page</title></head><body>
<div id="main" class="box"><p>Hello <b>world</b></p></div>
<input id="inp" type="text" value="init">
<input id="cb" type="checkbox" checked>
<ul id="list"><li>a</li><li>b</li></ul>
</body></html>
"#;

fn engine_with(html: &str) -> Engine {
    let mut e = Engine::new();
    e.install_dom(&Rc::new(RefCell::new(Dom::from_html(html))));
    e
}

/// Eval `src` in a fresh engine seeded with `html`, return the
/// completion value described as a string.
fn ev(src: &str) -> String {
    ev_in(PAGE, src)
}

fn ev_in(html: &str, src: &str) -> String {
    let mut e = engine_with(html);
    match e.eval(src) {
        Ok(v) => e.describe(v),
        Err(err) => panic!("{src} -> {err}"),
    }
}

// -- document ---------------------------------------------------------

#[test]
fn get_element_by_id() {
    assert_eq!(ev("document.getElementById('main').tagName"), "DIV");
    assert_eq!(ev("document.getElementById('main').id"), "main");
    assert_eq!(ev("document.getElementById('nope')"), "null");
}

#[test]
fn node_identity_is_stable() {
    assert_eq!(
        ev("document.getElementById('main') === document.getElementById('main')"),
        "true"
    );
}

#[test]
fn document_structure_accessors() {
    assert_eq!(ev("document.title"), "Test page");
    assert_eq!(ev("document.body.tagName"), "BODY");
    assert_eq!(ev("document.documentElement.tagName"), "HTML");
    assert_eq!(ev("document.getElementsByTagName('li').length"), "2");
    assert_eq!(ev("document.getElementsByClassName('box').length"), "1");
}

#[test]
fn document_write_appends_to_body() {
    assert_eq!(
        ev("document.write('<div id=\"w\">added</div>'); document.getElementById('w').innerHTML"),
        "added"
    );
}

// -- JS-C35 : document.write complet ---------------------------------

#[test]
fn document_write_is_variadic() {
    // Spec : tous les arguments sont concaténés — auparavant seul
    // le premier était écrit.
    assert_eq!(
        ev("document.write('<i>a</i>', '<i>b</i>'); document.getElementsByTagName('i').length"),
        "2"
    );
}

#[test]
fn document_writeln_appends_newline() {
    assert_eq!(
        ev("document.writeln('<p>w</p>'); document.body.innerHTML.indexOf('\\n') >= 0"),
        "true"
    );
}

#[test]
fn document_open_erases_then_writes() {
    // open() efface le document (spec `document open steps`) —
    // les `write` suivants reconstruisent.
    assert_eq!(
        ev("document.open(); document.write('<p>n</p>'); \
            String(document.getElementById('main')) + ',' + \
            document.querySelector('p').textContent"),
        "null,n"
    );
}

#[test]
fn document_write_on_closed_replaces_page() {
    // `write` sur document fermé = open() implicite : le contenu
    // existant est effacé d'abord (spec — le piège classique
    // post-load).
    assert_eq!(
        ev("document.close(); document.write('<div id=n>x</div>'); \
            String(document.getElementById('main')) + ',' + \
            document.getElementById('n').textContent"),
        "null,x"
    );
}

#[test]
fn document_write_inside_load_listener_replaces() {
    // `load` ferme le flux avant que les listeners ne courent :
    // un `write` dedans efface la page (spec).
    let mut e = engine_with(PAGE);
    e.eval("document.addEventListener('load', function() { document.write('<i>post</i>') })")
        .expect("listener install");
    e.dom_event("load").expect("dispatch");
    let v = e
        .eval("String(document.getElementById('main')) + ',' + document.getElementsByTagName('i').length")
        .expect("probe");
    assert_eq!(e.describe(v), "null,1");
}

#[test]
fn document_written_script_stays_inert() {
    // Un `<script>` écrit n'exécute pas dans le miroir —
    // l'ordonnancement revient à l'amont (étape 50), qui le
    // découvre à la re-sérialisation.
    assert_eq!(
        ev("document.write('<script>window.__t = 1</' + 'script>'); typeof window.__t"),
        "undefined"
    );
}

// -- Étape 103 : props KeyboardEvent ------------------------------------

#[test]
fn dom_event_props_reach_listeners() {
    // Props plateforme sur l'event dispatché : `key`/`code`/
    // `keyCode`/`which` calculés par l'embedder arrivent au
    // listener.
    let dom = Rc::new(RefCell::new(Dom::from_html(PAGE)));
    let mut e = Engine::new();
    e.install_dom(&dom);
    let body = dom.borrow().body();
    e.eval("var __log = ''; document.body.addEventListener('keydown', function(ev) { __log = ev.key+'/'+ev.code+'/'+ev.keyCode+'/'+ev.which })")
        .expect("listener");
    e.dom_event_on_props(
        "keydown",
        body,
        &[
            ("key".into(), explorer_jscore::EventProp::Str("a".into())),
            (
                "code".into(),
                explorer_jscore::EventProp::Str("KeyA".into()),
            ),
            ("keyCode".into(), explorer_jscore::EventProp::Num(65.0)),
            ("which".into(), explorer_jscore::EventProp::Num(65.0)),
        ],
    )
    .expect("dispatch");
    let v = e.eval("__log").expect("probe");
    assert_eq!(e.describe(v), "a/KeyA/65/65");
}

#[test]
fn dom_event_props_on_document_level() {
    // Chemin document (pas de nœud cible) : `dom_event_props`
    // vise `<body>`.
    let mut e = engine_with(PAGE);
    e.eval(
        "var __log = ''; document.body.addEventListener('keyup', function(ev) { __log = ev.key })",
    )
    .expect("listener");
    e.dom_event_props(
        "keyup",
        &[(
            "key".into(),
            explorer_jscore::EventProp::Str("Enter".into()),
        )],
    )
    .expect("dispatch");
    let v = e.eval("__log").expect("probe");
    assert_eq!(e.describe(v), "Enter");
}

#[test]
fn keyboard_event_ctor_copies_init_keys() {
    // `new KeyboardEvent('k', {key:'a', code:'KeyA', keyCode:65})`
    // — les clés d'init survivent à la lecture.
    assert_eq!(
        ev("var k = new KeyboardEvent('keydown', {key:'a', code:'KeyA', keyCode:65, which:65}); k.key+'/'+k.code+'/'+k.keyCode+'/'+k.which"),
        "a/KeyA/65/65"
    );
}

#[test]
fn dispatched_literal_event_exposes_key() {
    // `dispatchEvent({type:'keydown', key:'a'})` — le literal
    // porte ses props à l'EventHost.
    assert_eq!(
        ev("var __k = ''; document.body.addEventListener('keydown', function(ev) { __k = ev.key }); \
            document.body.dispatchEvent({type:'keydown', key:'z'}); __k"),
        "z"
    );
}

// -- JS-C57 : `Image`/`HTMLImageElement` --------------------------------

#[test]
fn image_constructor_creates_detached_img() {
    // wikipedia.org (vol 22/10) : `new Image()` dans `doWhenReady`
    // mourait en ReferenceError — alias `createElement('img')`
    // détaché + attrs width/height optionnels.
    assert_eq!(
        ev("var i = new Image(); i.tagName + '|' + String(i.parentNode)"),
        "IMG|null"
    );
    assert_eq!(
        ev("var i = new Image(100, 50); i.getAttribute('width') + 'x' + i.getAttribute('height')"),
        "100x50"
    );
    // Insertable comme tout élément — le pattern preload
    // (`i.src = …`, listener `onload`) n'a plus d'obstacle au ctor.
    assert_eq!(ev("document.body.appendChild(new Image()).tagName"), "IMG");
    // `HTMLImageElement` = stub de détection, inconstructible.
    assert_eq!(ev("typeof HTMLImageElement"), "function");
    assert_eq!(
        ev("var r=''; try{new HTMLImageElement()}catch(e){r=e.name} r"),
        "TypeError"
    );
}

#[test]
fn html_element_interfaces_detectable() {
    // github.com vol 22/10 : `ReferenceError: HTMLButtonElement`
    // dans `t2-*.js` — la famille `HTML*Element` complète est
    // installée en stubs de détection (inconstructibles).
    assert_eq!(
        ev("['HTMLButtonElement','HTMLInputElement','HTMLAnchorElement',\
            'HTMLDivElement','HTMLFormElement','HTMLCanvasElement',\
            'HTMLVideoElement','HTMLTemplateElement','HTMLSelectElement',\
            'HTMLTableElement','HTMLSpanElement','HTMLMediaElement']\
            .map(function(n){return typeof globalThis[n]}).join(',')"),
        "function,function,function,function,function,function,function,function,function,function,function,function"
    );
    // Inconstructibles comme le reste du registre.
    assert_eq!(
        ev("var r=''; try{new HTMLButtonElement()}catch(e){r=e.name} r"),
        "TypeError"
    );
}

#[test]
fn navigator_is_fixed_and_poor() {
    assert_eq!(ev("navigator.userAgent"), "Explorer/0.1 (explorer-jscore)");
    // Deliberately absent: plugins, geolocation, hardware APIs…
    assert_eq!(ev("navigator.plugins === undefined"), "true");
    assert_eq!(ev("navigator.geolocation === undefined"), "true");
}

#[test]
fn navigator_languages_frozen_array() {
    // JS-C51 — le `<svg>` d'example.com mourait sur
    // `navigator.languages.some` (TypeError → script stoppé).
    assert_eq!(ev("navigator.languages.join(',')"), "en-US,en");
    assert_eq!(
        ev("navigator.languages.some(function(l){return l.indexOf('en') === 0})"),
        "true"
    );
    // Spec : gelé, même instance à chaque lecture.
    assert_eq!(ev("Object.isFrozen(navigator.languages)"), "true");
    assert_eq!(ev("navigator.languages === navigator.languages"), "true");
    assert_eq!(
        ev("try { navigator.languages.push('fr') } catch (e) {} navigator.languages.length"),
        "2"
    );
}

// -- elements ---------------------------------------------------------

#[test]
fn inner_html_get_set() {
    assert_eq!(
        ev("document.getElementById('main').innerHTML"),
        "<p>Hello <b>world</b></p>"
    );
    assert_eq!(
        ev("var el = document.getElementById('main');\
            el.innerHTML = '<span>x</span>';\
            el.innerHTML"),
        "<span>x</span>"
    );
}

#[test]
fn text_content() {
    assert_eq!(
        ev("document.getElementById('main').textContent"),
        "Hello world"
    );
    assert_eq!(
        ev("var el = document.getElementById('main');\
            el.textContent = 'plain';\
            el.innerHTML"),
        "plain"
    );
}

#[test]
fn class_name_and_class_list() {
    assert_eq!(
        ev("var el = document.getElementById('main');\
            el.className = 'a b';\
            el.className"),
        "a b"
    );
    assert_eq!(
        ev("var el = document.getElementById('main');\
            el.classList.add('on');\
            el.classList.contains('on') + ':' + el.getAttribute('class')"),
        "true:box on"
    );
    assert_eq!(
        ev("var el = document.getElementById('main');\
            el.classList.toggle('box');\
            el.className"),
        ""
    );
}

#[test]
fn style_property() {
    assert_eq!(
        ev("var el = document.getElementById('main');\
            el.style.backgroundColor = 'red';\
            el.getAttribute('style')"),
        "background-color: red"
    );
    assert_eq!(
        ev("var el = document.getElementById('main');\
            el.style.backgroundColor = 'red';\
            el.style.backgroundColor"),
        "red"
    );
}

#[test]
fn form_like_properties() {
    assert_eq!(ev("document.getElementById('inp').value"), "init");
    assert_eq!(
        ev("var i = document.getElementById('inp');\
            i.value = 'typed';\
            i.getAttribute('value')"),
        "typed"
    );
    // Boolean attribute → bool prop, both ways.
    assert_eq!(ev("document.getElementById('cb').checked"), "true");
    assert_eq!(
        ev("var c = document.getElementById('cb');\
            c.checked = false;\
            c.checked + ':' + c.hasAttribute('checked')"),
        "false:false"
    );
}

#[test]
fn tree_navigation() {
    assert_eq!(
        ev("var p = document.getElementsByTagName('p')[0];\
            p.parentNode.id"),
        "main"
    );
    assert_eq!(ev("document.getElementById('main').children.length"), "1");
    assert_eq!(
        ev("document.getElementById('list').children[1].textContent"),
        "b"
    );
    assert_eq!(
        ev("document.getElementById('main').firstChild.tagName"),
        "P"
    );
}

#[test]
fn query_selector_subset() {
    assert_eq!(ev("document.querySelector('#main').id"), "main");
    assert_eq!(ev("document.querySelector('.box').id"), "main");
    assert_eq!(ev("document.querySelector('p').tagName"), "P");
    assert_eq!(ev("document.querySelectorAll('li').length"), "2");
    // Complex selectors out of subset → null, not an error.
    assert_eq!(ev("document.querySelector('div ~ p')"), "null");
}

/// Combinateur enfant `a > b` — motif terrain : spectrum.adobe.com
/// décore ses sections via `querySelectorAll('main > div')`
/// (JS-C120 ; `doc:25` quand le sélecteur rendait vide).
#[test]
fn query_selector_child_combinator() {
    assert_eq!(ev("document.querySelectorAll('div > p').length"), "1");
    assert_eq!(ev("document.querySelectorAll('#main > p').length"), "1");
    assert_eq!(ev("document.querySelectorAll('body > div').length"), "1");
    assert_eq!(ev("document.querySelectorAll('ul > li').length"), "2");
    // Sans espaces autour de `>`.
    assert_eq!(ev("document.querySelectorAll('#list>li').length"), "2");
    // `b` est descendant de div mais pas enfant direct.
    assert_eq!(ev("document.querySelectorAll('div > b').length"), "0");
    // Chaîne mixte : `>` puis descendant.
    assert_eq!(ev("document.querySelectorAll('#main > p b').length"), "1");
    // `querySelector`/`matches`/`closest` partagent le rail.
    assert_eq!(ev("document.querySelector('div > p').tagName"), "P");
    assert_eq!(ev("document.querySelector('p').matches('div > p')"), "true");
    assert_eq!(
        ev("document.querySelector('b').closest('p > b').tagName"),
        "B"
    );
    assert_eq!(
        ev("document.querySelector('b').closest('div > b') === null"),
        "true"
    );
}

#[test]
fn query_selector_universal() {
    // `*` — tout élément sous le nœud requêté.
    assert_eq!(
        ev("document.querySelectorAll('*').length === document.getElementsByTagName('*').length"),
        "true"
    );
    assert_eq!(ev("document.querySelectorAll('ul > *').length"), "2");
    assert_eq!(ev("document.querySelectorAll('#main > *').length"), "1");
}

/// `el.qSA(':scope > *')` — le parent-scopé DOIT matcher (spec :
/// les sélecteurs s'évaluent en contexte document, le `bound`
/// n'exclut pas l'élément requêté). Motif : ak.js `groupChildren`
/// — spectrum.adobe.com.
#[test]
fn query_selector_scope_child() {
    assert_eq!(
        ev("document.getElementById('list').querySelectorAll(':scope > *').length"),
        "2"
    );
    assert_eq!(
        ev("document.getElementById('main').querySelectorAll(':scope > p').length"),
        "1"
    );
    // `el.qSA('div > p')` — `el` lui-même peut servir de parent.
    assert_eq!(
        ev("document.getElementById('main').querySelectorAll('#main > p').length"),
        "1"
    );
    // Descendant ne franchit pas davantage : p reste trouvé.
    assert_eq!(
        ev("document.getElementById('main').querySelectorAll('div b').length"),
        "1"
    );
}

/// Sélecteurs d'attribut `[a]`/`[a=v]`/`~=`/`|=`/`^=`/`$=`/`*=` —
/// le motif terrain qui les a motivés : lobste.rs itère
/// `qSA('time[data-at-unix]')` au `DOMContentLoaded`.
#[test]
fn query_selector_attr_operators() {
    // Présence + valeur exacte (nue ou quotée).
    assert_eq!(
        ev("document.querySelector('input[type=checkbox]').id"),
        "cb"
    );
    assert_eq!(
        ev("document.querySelector('input[type=\"text\"]').id"),
        "inp"
    );
    assert_eq!(ev("document.querySelector('[checked]').id"), "cb");
    assert_eq!(ev("document.querySelectorAll('input[type]').length"), "2");
    // `~=` mot dans une liste ; `|=` préfixe de tiret.
    assert_eq!(
        ev_in(
            "<div id=a class='x y z'></div><div id=b class='xy'></div><i id=c lang='fr-FR'></i>",
            "document.querySelectorAll('[class~=y]').length + '/' + \
             document.querySelector('[lang|=fr]').id"
        ),
        "1/c"
    );
    // `^=`/`$=`/`*=` préfixe/suffixe/sous-chaîne.
    assert_eq!(
        ev("document.querySelector('input[id^=\"in\"]').id + '/' + \
            document.querySelector('input[id$=\"b\"]').id + '/' + \
            document.querySelectorAll('input[id*=n]').length"),
        "inp/cb/1"
    );
    // Compound : tag + classe + attribut.
    assert_eq!(
        ev("document.querySelector('input[type=checkbox][checked]').id"),
        "cb"
    );
    // Absent ou valeur différente → null.
    assert_eq!(ev("document.querySelector('[data-nope]')"), "null");
    assert_eq!(ev("document.querySelector('input[type=radio]')"), "null");
    // Listes `a, b` — sous-ensemble JS-C99 : premier match en
    // ordre document.
    assert_eq!(
        ev("document.querySelector('[type=text],[checked]').id"),
        "inp"
    );
    // Hors sous-ensemble (`>`, pseudo-élément) → null, jamais
    // d'erreur ; `:hover` = état `Never` (JS-C99) → non matché.
    assert_eq!(ev("document.querySelector('div + p')"), "null");
    assert_eq!(ev("document.querySelector('input::x')"), "null");
    assert_eq!(ev("document.querySelector('input:hover')"), "null");
    // `matches`/`closest` partagent le même sous-ensemble.
    assert_eq!(
        ev("document.getElementById('cb').matches('input[checked]')"),
        "true"
    );
    assert_eq!(
        ev("document.getElementById('inp').matches('[checked]')"),
        "false"
    );
}

#[test]
fn create_and_attach() {
    assert_eq!(
        ev("var d = document.createElement('div');\
            d.appendChild(document.createTextNode('hi'));\
            document.getElementById('main').appendChild(d);\
            document.getElementById('main').children.length"),
        "2"
    );
    assert_eq!(
        ev("var d = document.createElement('div');\
            document.getElementById('main').appendChild(d);\
            document.getElementById('main').removeChild(d);\
            document.getElementById('main').children.length"),
        "1"
    );
}

#[test]
fn create_comment_node() {
    // `document.createComment` — nodeType 8 ; Lit y dépose ses
    // marqueurs de parties (vaadin terrain — le manque faisait
    // `createComment is not a function` et aucun CE ne montait).
    assert_eq!(
        ev("var c = document.createComment('?lit$0$');\
            [c.nodeType, c.nodeName, c.data, c.textContent].join('|')"),
        "8|#comment|?lit$0$|?lit$0$"
    );
    // `data`/`nodeValue` écrivables sur le comment.
    assert_eq!(
        ev("var c = document.createComment('a');\
            c.data = 'b'; c.nodeValue"),
        "b"
    );
    // Les `<!--…-->` d'`innerHTML` deviennent des nœuds Comment
    // (marqueurs Lit injectés via innerHTML) et ressortent à la
    // sérialisation — round-trip fidèle.
    assert_eq!(
        ev("var d = document.createElement('div');\
            d.innerHTML = 'x<!--?lit$1$-->y';\
            [d.childNodes.length, d.childNodes[1].nodeType, d.childNodes[1].data, d.innerHTML].join('|')"),
        "3|8|?lit$1$|x<!--?lit$1$-->y"
    );
    // `textContent` du parent saute les comments (spec) ;
    // `children` (éléments seuls) n'en compte pas.
    assert_eq!(
        ev("var d = document.createElement('div');\
            d.innerHTML = '<b>a</b><!--m--><b>c</b>';\
            [d.children.length, d.textContent].join('|')"),
        "2|ac"
    );
}

#[test]
fn event_target_and_domexception_constructible() {
    // css-tricks terrain : wp-polyfill/core-js meurt au feature-
    // detect si `new EventTarget()`/`new DOMException()` jettent
    // « illegal constructor » — les deux sont constructibles spec.
    assert_eq!(
        ev("var et = new EventTarget();\
            [typeof et.addEventListener, et instanceof EventTarget].join('|')"),
        "function|true"
    );
    // Listener + dispatch : cb appelée avec l'event, `target`
    // posé, `dispatchEvent` → true (non preventDefault).
    assert_eq!(
        ev("var et = new EventTarget(); var log='';\
            et.addEventListener('ping', function(ev){\
              log += ev.type + '/' + (ev.target === et); });\
            var r = et.dispatchEvent({type:'ping'});\
            [log, r].join('|')"),
        "ping/true|true"
    );
    // removeEventListener retire la cb.
    assert_eq!(
        ev("var et = new EventTarget(); var n=0;\
            var f = function(){ n++ };\
            et.addEventListener('x', f);\
            et.removeEventListener('x', f);\
            et.dispatchEvent({type:'x'}); n"),
        "0"
    );
    // `new DOMException(msg, name)` — `{name, message}` + instanceof.
    assert_eq!(
        ev("var d = new DOMException('nope', 'NotSupportedError');\
            [d.name, d.message, d instanceof DOMException].join('|')"),
        "NotSupportedError|nope|true"
    );
    assert_eq!(ev("new DOMException('x').name"), "Error");
}

#[test]
fn append_child_ancestor_cycle_rejected() {
    // Spec `HierarchyRequestError` : `parent.appendChild(ancêtre)`
    // créerait un cycle → toutes les walks DFS postérieures
    // boucleraient (qSA, innerHTML, textContent…).
    assert_eq!(
        ev("var g = document.createElement('div');\
            var p = document.createElement('section');\
            g.appendChild(p);\
            try { p.appendChild(g); 'no' } catch (e) { 'err' }"),
        "err"
    );
    // Soi-même — `p.appendChild(p)` est aussi un cycle.
    assert_eq!(
        ev("var p2 = document.createElement('div');\
            try { p2.appendChild(p2); 'no' } catch (e) { 'err' }"),
        "err"
    );
    // insertBefore même garde.
    assert_eq!(
        ev("var g3 = document.createElement('div');\
            var p3 = document.createElement('section');\
            g3.appendChild(p3);\
            try { p3.insertBefore(g3, null); 'no' } catch (e) { 'err' }"),
        "err"
    );
    // Append normal toujours OK.
    assert_eq!(
        ev("var a = document.createElement('div');\
            var b = document.createElement('div');\
            a.appendChild(b); a.childNodes.length"),
        "1"
    );
}

#[test]
fn adopted_sheets_css_extracts_rule_texts() {
    // CAP-195 tranche 2 : les feuilles adoptées livrent leurs
    // `cssText` à la session — cascade en feuilles auteur
    // document (ordre spec : après les `<link>`).
    let mut e = engine_full(PAGE);
    e.0.eval(
        "var s=new CSSStyleSheet();s.insertRule('a{color:red}');\
         s.insertRule('b{margin:0}');document.adoptedStyleSheets=[s];",
    )
    .unwrap();
    assert_eq!(e.0.adopted_sheets_css(), vec!["a{color:red}\nb{margin:0}"]);
}

#[test]
fn define_duplicate_throws_domexception_not_supported() {
    // Spec + catalyst github : re-define doit jeter
    // `DOMException`/`NotSupportedError` — le catch de catalyst
    // tolère précisément cette forme (`!(e instanceof
    // DOMException && e.name==="NotSupportedError") → rethrow`) ;
    // un TypeError cassait la chaîne lazy-CE.
    assert_eq!(
        ev("class CE1 extends HTMLElement{};\
            customElements.define('x-dup', CE1);\
            try { customElements.define('x-dup', CE1); 'no' }\
            catch (e) { [e instanceof DOMException, e.name].join('|') }"),
        "true|NotSupportedError"
    );
    // Nom invalide → DOMException `SyntaxError` (spec).
    assert_eq!(
        ev(
            "try { customElements.define('nohyphen', class extends HTMLElement{}); 'no' }\
            catch (e) { [e instanceof DOMException, e.name].join('|') }"
        ),
        "true|SyntaxError"
    );
}

#[test]
fn template_content_is_fragment() {
    // github.com `z8-*.js` : `tpl.content.appendChild(row)` —
    // `.content` matérialise le fragment de contenu et y
    // *déplace* les enfants (spec `template contents`).
    assert_eq!(
        ev("var t = document.createElement('template');\
            t.innerHTML = '<b>x</b><i>y</i>';\
            t.content.nodeType"),
        "11"
    );
    // `innerHTML` du template sérialise le contenu du fragment.
    assert_eq!(
        ev("var t = document.createElement('template');\
            t.innerHTML = '<b>x</b><i>y</i>';\
            t.content.appendChild(document.createElement('u'));\
            t.innerHTML"),
        "<b>x</b><i>y</i><u></u>"
    );
    // Les enfants déplacés : `childNodes` de l'élément est vide.
    assert_eq!(
        ev("var t = document.createElement('template');\
            t.innerHTML = '<b>x</b>';\
            t.content;\
            t.childNodes.length"),
        "0"
    );
    // Stabilité d'identité : deux lectures, même objet.
    assert_eq!(
        ev("var t = document.createElement('template');\
            t.content === t.content"),
        "true"
    );
    // `<template>` du markup : la lecture `.content` avale les
    // enfants parsés.
    assert_eq!(
        ev_in(
            "<html><body><template id='tpl'><b>m</b></template></body></html>",
            "document.getElementById('tpl').content.children.length",
        ),
        "1"
    );
    // …et son `childNodes` propre passe à 0 après la matérialisation.
    assert_eq!(
        ev_in(
            "<html><body><template id='tpl'><b>m</b></template></body></html>",
            "document.getElementById('tpl').content;\
             document.getElementById('tpl').childNodes.length",
        ),
        "0"
    );
}

#[test]
fn create_element_ns_creates_local_name() {
    // github.com `z8-*.js` : svg inline via
    // `document.createElementNS('http://www.w3.org/2000/svg',
    // 'svg')` — l'URI est acceptée/ignorée, le nœud est créé
    // par son nom local (préfixe `ns:tag` dépouillé).
    assert_eq!(
        ev("document.createElementNS('http://www.w3.org/2000/svg','svg').tagName"),
        "SVG"
    );
    assert_eq!(
        ev("document.createElementNS('http://www.w3.org/2000/svg','svg:circle').tagName"),
        "CIRCLE"
    );
}

// -- host semantics -----------------------------------------------------

#[test]
fn in_operator_sees_host_props() {
    assert_eq!(ev("'id' in document.getElementById('main')"), "true");
    assert_eq!(ev("'innerHTML' in document.body"), "true");
    assert_eq!(ev("'tortexplode' in document.body"), "false");
}

#[test]
fn script_expandos_work_on_host_objects() {
    // Host.set returning false → ordinary own property fallback.
    assert_eq!(
        ev("var el = document.getElementById('main');\
            el.myFlag = 42;\
            el.myFlag"),
        "42"
    );
}

#[test]
fn for_in_over_expandos() {
    assert_eq!(
        ev("var el = document.getElementById('main');\
            el.a = 1; el.b = 2;\
            var seen = '';\
            for (var k in el) { if (k === 'a' || k === 'b') seen += k; }\
            seen"),
        "ab"
    );
}

#[test]
fn typeof_and_class_tag() {
    assert_eq!(ev("typeof document"), "object");
    assert_eq!(
        ev("Object.prototype.toString.call(document.getElementById('main'))"),
        "[object HTMLElement]"
    );
    assert_eq!(
        ev("Object.prototype.toString.call(document)"),
        "[object HTMLDocument]"
    );
}

// -- events -------------------------------------------------------------

#[test]
fn add_event_listener_and_click() {
    assert_eq!(
        ev("var b = document.getElementById('main');\
            var hit = '';\
            b.addEventListener('click', function (ev) { hit = ev.type; });\
            b.click();\
            hit"),
        "click"
    );
}

#[test]
fn onclick_property_and_event_target() {
    assert_eq!(
        ev("var b = document.getElementById('main');\
            var same = false;\
            b.onclick = function (ev) { same = (ev.target === b); };\
            b.click();\
            same"),
        "true"
    );
}

#[test]
fn inline_onclick_attribute_compiles_and_fires() {
    // JS-C45 : `onclick="src"` est compilé paresseusement au
    // premier dispatch — wrapper spec `with(document){
    // with(this.form||{}){with(this){ src }}}`.
    assert_eq!(
        ev_in(
            "<html><body><button id=\"b\" onclick=\"hit=(hit||0)+1\">x</button></body></html>",
            "var hit = 0;\
             document.getElementById('b').click();\
             document.getElementById('b').click();\
             hit"
        ),
        "2"
    );
}

#[test]
fn inline_handler_sees_this_event_and_bare_props() {
    // `this` = élément, `event` porté, props nues via `with(this)`.
    assert_eq!(
        ev_in(
            "<html><body><button id=\"b9\" onclick=\"mark=id+':'+event.type\">x</button></body></html>",
            "var mark = '';\
             document.getElementById('b9').click();\
             mark"
        ),
        "b9:click"
    );
}

#[test]
fn inline_handler_recompiles_on_setattribute() {
    // Le cache est invalidé par la source : `setAttribute`
    // impose la recompilation ; `removeAttribute` éteint.
    assert_eq!(
        ev_in(
            "<html><body><button id=\"b\" onclick=\"hit='v1'\">x</button></body></html>",
            "var hit = '';\
             var b = document.getElementById('b');\
             b.click();\
             var v1 = hit;\
             b.setAttribute('onclick', \"hit='v2'\");\
             b.click();\
             var v2 = hit;\
             b.removeAttribute('onclick');\
             b.click();\
             [v1, v2, hit].join('/')",
        ),
        "v1/v2/v2"
    );
}

#[test]
fn inline_handler_loses_to_script_prop() {
    // `el.onclick = f` écrase l'attribut (dernier écrivain
    // gagne — l'attribut est écrit au parse).
    assert_eq!(
        ev_in(
            "<html><body><button id=\"b\" onclick=\"hit='attr'\">x</button></body></html>",
            "var hit = '';\
             var b = document.getElementById('b');\
             b.onclick = function () { hit = 'prop'; };\
             b.click();\
             hit",
        ),
        "prop"
    );
}

#[test]
fn remove_event_listener() {
    assert_eq!(
        ev("var b = document.getElementById('main');\
            var n = 0;\
            var f = function () { n += 1; };\
            b.addEventListener('click', f);\
            b.click();\
            b.removeEventListener('click', f);\
            b.click();\
            n"),
        "1"
    );
}

// -- events: propagation (JS-3) ----------------------------------------------

#[test]
fn event_bubbles_to_parent() {
    // li (inside ul#list) → #list → body: bubbling walks ancestors.
    assert_eq!(
        ev("var li = document.getElementsByTagName('li')[0];\
            var order = '';\
            document.getElementById('list').addEventListener('click', function () { order += 'L'; });\
            document.body.addEventListener('click', function () { order += 'B'; });\
            li.click();\
            order"),
        "LB"
    );
}

#[test]
fn event_capture_runs_before_bubble() {
    assert_eq!(
        ev("var li = document.getElementsByTagName('li')[0];\
            var list = document.getElementById('list');\
            var order = '';\
            li.addEventListener('click', function () { order += 'b'; });\
            list.addEventListener('click', function () { order += 'c'; }, true);\
            list.addEventListener('click', function () { order += 'B'; });\
            li.click();\
            order"),
        "cbB"
    );
}

#[test]
fn current_target_and_target() {
    assert_eq!(
        ev("var li = document.getElementsByTagName('li')[0];\
            var seen = '';\
            document.getElementById('list').addEventListener('click', function (ev) {\
                seen = ev.target.tagName + '/' + ev.currentTarget.id; });\
            li.click();\
            seen"),
        "LI/list"
    );
}

#[test]
fn stop_propagation_halts_bubble() {
    assert_eq!(
        ev("var li = document.getElementsByTagName('li')[0];\
            var order = '';\
            li.addEventListener('click', function (ev) {\
                order += 'L'; ev.stopPropagation(); });\
            document.getElementById('list').addEventListener('click', function () { order += 'P'; });\
            li.click();\
            order"),
        "L"
    );
}

#[test]
fn dispatch_event_with_literal() {
    assert_eq!(
        ev("var b = document.getElementById('main');\
            var got = '';\
            b.addEventListener('ping', function (ev) { got = ev.type; });\
            b.dispatchEvent({ type: 'ping', bubbles: false });\
            got"),
        "ping"
    );
}

#[test]
fn dispatch_event_with_create_event() {
    assert_eq!(
        ev("var b = document.getElementById('main');\
            var got = '';\
            b.addEventListener('custom', function (ev) { got = ev.type + ':' + ev.bubbles; });\
            var ev = document.createEvent('HTMLEvents');\
            ev.initEvent('custom', true, true);\
            b.dispatchEvent(ev);\
            got"),
        "custom:true"
    );
}

#[test]
fn prevent_default_returned_by_dispatch() {
    assert_eq!(
        ev("var b = document.getElementById('main');\
            b.addEventListener('x', function (ev) { ev.preventDefault(); });\
            var ev = document.createEvent('HTMLEvents');\
            ev.initEvent('x', false, true);\
            b.dispatchEvent(ev) + ':' + ev.defaultPrevented"),
        "false:true"
    );
}

/// JS-C133 — chemin polymer/shady : `createEvent('CustomEvent')`
/// suivi de `initCustomEvent(type, bubbles, cancelable, detail)`
/// doit porter `detail` au listener (strates ytd-app) — le
/// payload arrive post-`createEvent` (posé en props d'event).
#[test]
fn init_custom_event_carries_detail() {
    assert_eq!(
        ev("var b = document.getElementById('main');\
            var got = '';\
            b.addEventListener('cev', function (ev) {\
                got = ev.type + ':' + ev.bubbles + ':' + (ev.detail && ev.detail.n);\
            });\
            var ev = document.createEvent('CustomEvent');\
            ev.initCustomEvent('cev', true, false, {n: 7});\
            b.dispatchEvent(ev); got"),
        "cev:true:7"
    );
}

// -- timers (JS-3) -----------------------------------------------------------

#[test]
fn set_timeout_runs_on_pump() {
    let mut e = engine_with("");
    e.install_timers();
    e.eval("var done = ''; setTimeout(function () { done = 'fired'; }, 0);")
        .expect("eval");
    // Not yet: timers don't run synchronously.
    assert_eq!(
        e.eval("done").map(|v| e.describe(v)).unwrap_or_default(),
        ""
    );
    assert_eq!(e.run_due_timers().unwrap_or(0), 1);
    assert_eq!(
        e.eval("done").map(|v| e.describe(v)).unwrap_or_default(),
        "fired"
    );
}

#[test]
fn set_interval_reschedules_and_clears() {
    let mut e = engine_with("");
    e.install_timers();
    e.eval(
        "var n = 0;\
            var id = setInterval(function () { n += 1; }, 0);",
    )
    .expect("eval");
    e.run_due_timers().unwrap_or(0);
    e.run_due_timers().unwrap_or(0);
    e.eval("clearInterval(id);").expect("eval");
    e.run_due_timers().unwrap_or(0);
    assert_eq!(e.eval("n").map(|v| e.describe(v)).unwrap_or_default(), "2");
}

#[test]
fn clear_timeout_cancels() {
    let mut e = engine_with("");
    e.install_timers();
    e.eval(
        "var ran = false;\
            var id = setTimeout(function () { ran = true; }, 0);\
            clearTimeout(id);",
    )
    .expect("eval");
    e.run_due_timers().unwrap_or(0);
    assert_eq!(
        e.eval("ran").map(|v| e.describe(v)).unwrap_or_default(),
        "false"
    );
}

#[test]
fn string_callback_runs_in_global_scope() {
    // `setTimeout('code')` — legacy string form evaluated at fire
    // time in global scope (indirect-eval semantics).
    let mut e = engine_with("");
    e.install_timers();
    e.eval("var hit = 0; setTimeout('hit = 42', 0);")
        .expect("eval");
    assert_eq!(e.run_due_timers().unwrap_or(0), 1);
    assert_eq!(
        e.eval("hit").map(|v| e.describe(v)).unwrap_or_default(),
        "42"
    );
}

/// `requestIdleCallback`/`cancelIdleCallback` (CAP-165) —
/// React scheduler et les libs riches en dépendent
/// (`window.requestIdleCallback || shim` sinon).
#[test]
fn request_idle_callback_deadline() {
    let mut e = engine_with("");
    e.install_timers();
    e.eval(
        "var got = '';\
            requestIdleCallback(function (d) {\
                got = d.didTimeout + '|' + (typeof d.timeRemaining) + '|' +\
                    (d.timeRemaining() >= 0 && d.timeRemaining() <= 50);\
            });",
    )
    .expect("eval");
    assert_eq!(e.run_due_timers().unwrap_or(0), 1);
    assert_eq!(
        e.eval("got").map(|v| e.describe(v)).unwrap_or_default(),
        "false|function|true"
    );
}

/// `{timeout: ms}` arme l'échéance — un timeout lointain ne
/// tire pas à la pompe immédiate (devient `didTimeout:true`
/// à son échéance).
#[test]
fn idle_callback_timeout_arms() {
    let mut e = engine_with("");
    e.install_timers();
    e.eval(
        "var fired = false;\
            requestIdleCallback(function (d) { fired = d.didTimeout; }, { timeout: 60000 });",
    )
    .expect("eval");
    assert_eq!(e.run_due_timers().unwrap_or(0), 0);
    assert_eq!(
        e.eval("fired").map(|v| e.describe(v)).unwrap_or_default(),
        "false"
    );
}

/// `cancelIdleCallback` ≙ `clearTimeout` ; callback non
/// fonction → `TypeError` (spec `WebIDL`).
#[test]
fn idle_callback_cancel_and_typecheck() {
    let mut e = engine_with("");
    e.install_timers();
    e.eval(
        "var ran = false;\
            var id = requestIdleCallback(function () { ran = true; });\
            cancelIdleCallback(id);",
    )
    .expect("eval");
    e.run_due_timers().unwrap_or(0);
    assert_eq!(
        e.eval("ran").map(|v| e.describe(v)).unwrap_or_default(),
        "false"
    );
    assert!(e.eval("requestIdleCallback(42)").is_err());
}

// -- tolerance ------------------------------------------------------------

#[test]
fn malformed_markup_degrades() {
    // Unclosed tags, stray '<', stray '</x>' — must not panic.
    assert_eq!(
        ev_in(
            "<div id=a><b>bold<p>para",
            "document.getElementById('a').innerHTML"
        ),
        "<b>bold<p>para</p></b>"
    );
}

#[test]
fn attribute_entities_and_case() {
    assert_eq!(
        ev_in(
            "<DIV ID=X data-k='a&amp;b'>t</DIV>",
            "document.getElementById('X').getAttribute('data-k')"
        ),
        "a&b"
    );
}

#[test]
fn text_entities_decoded_and_roundtrip() {
    // JS-C46 : les références de caractères du texte injecté
    // sont décodées à l'entrée — `textContent` lit le caractère
    // réel et la resérialisation ne l'échappe qu'une fois
    // (`&nbsp;` injecté → `\u{A0}` → `&nbsp;`, jamais
    // `&amp;nbsp;` affiché en clair après re-parse).
    assert_eq!(
        ev_in(
            "<div id=a>t</div>",
            "var el = document.getElementById('a');\
             el.innerHTML = 'caf&eacute;&nbsp;x&#8217;y&#x2019;&copy;';\
             el.textContent"
        ),
        "café\u{A0}x’y’©"
    );
    // Round-trip : le texte décodé `<`/`&` re-échappe une
    // seule fois à la lecture d'innerHTML.
    assert_eq!(
        ev_in(
            "<div id=a>t</div>",
            "var el = document.getElementById('a');\
             el.innerHTML = 'a &lt; b &amp; c';\
             el.innerHTML"
        ),
        "a &lt; b &amp; c"
    );
    // Inconnue : passthrough verbatim tolérant.
    assert_eq!(
        ev_in(
            "<div id=a>t</div>",
            "var el = document.getElementById('a');\
             el.innerHTML = 'x&notanentity;y';\
             el.textContent"
        ),
        "x&notanentity;y"
    );
}

// -- browser surface (JS-7e): cookie, storage, location, history -------

/// In-memory `CookieJar` — `set` keeps raw `k=v` pairs (attrs ignored).
struct TestJar {
    inner: RefCell<Vec<String>>,
}

impl explorer_jscore::CookieJar for TestJar {
    fn get(&self, _url: &str) -> String {
        self.inner.borrow().join("; ")
    }
    fn set(&self, _url: &str, cookie: &str) {
        // Keep only `k=v` before the first ';' like a real jar's
        // `document.cookie` read-back.
        let kv = cookie.split(';').next().unwrap_or("").trim().to_string();
        self.inner.borrow_mut().push(kv);
    }
}

fn engine_full(html: &str) -> (Engine, Rc<TestJar>, Rc<RefCell<Vec<String>>>) {
    let mut e = Engine::new();
    e.install_dom(&Rc::new(RefCell::new(Dom::from_html(html))));
    // Per-page localStorage map — an embedder shares it by origin.
    e.install_storage(&Rc::new(RefCell::new(std::collections::BTreeMap::new())));
    let jar = Rc::new(TestJar {
        inner: RefCell::new(Vec::new()),
    });
    let navs: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let navs2 = navs.clone();
    let navs3 = navs.clone();
    e.set_page_url("http://ex.org:8080/a/b?q=1#frag");
    e.set_cookie_jar(jar.clone());
    e.set_nav_hooks(
        Rc::new(move |u: &str| navs2.borrow_mut().push(u.to_string())),
        Rc::new(move |n: i32| navs3.borrow_mut().push(format!("go:{n}"))),
        3,
    );
    (e, jar, navs)
}

fn evf(e: &mut Engine, src: &str) -> String {
    match e.eval(src) {
        Ok(v) => e.describe(v),
        Err(err) => panic!("{src} -> {err}"),
    }
}

#[test]
fn document_cookie_roundtrip() {
    let (mut e, _jar, _n) = engine_full(PAGE);
    evf(
        &mut e,
        "document.cookie = 'sess=abc'; document.cookie = 'pref=dark; path=/'",
    );
    assert_eq!(evf(&mut e, "document.cookie"), "sess=abc; pref=dark");
}

#[test]
fn storage_set_get_remove() {
    let (mut e, _jar, _n) = engine_full(PAGE);
    evf(
        &mut e,
        "localStorage.setItem('k', 'v'); localStorage.alt = 'x';",
    );
    assert_eq!(
        evf(
            &mut e,
            "localStorage.getItem('k') + '|' + localStorage['alt'] + '|' + localStorage.length"
        ),
        "v|x|2"
    );
    // Named-property assignment also stores.
    assert_eq!(
        evf(&mut e, "localStorage.key(0) + ',' + localStorage.key(1)"),
        "alt,k"
    );
    evf(&mut e, "localStorage.removeItem('k');");
    assert_eq!(evf(&mut e, "localStorage.getItem('k')"), "null");
    assert_eq!(evf(&mut e, "localStorage.length"), "1");
    evf(&mut e, "localStorage.clear();");
    assert_eq!(evf(&mut e, "localStorage.length"), "0");
    // sessionStorage is a separate, page-local map.
    evf(&mut e, "sessionStorage.setItem('s', '1');");
    assert_eq!(evf(&mut e, "sessionStorage.getItem('s')"), "1");
    assert_eq!(evf(&mut e, "localStorage.getItem('s')"), "null");
}

#[test]
fn document_inherits_document_prototype() {
    // dev.to : `Document.prototype.ready = new Promise(…)` puis
    // `document.ready.then(…)` — les patches du proto DOM doivent
    // être visibles via `document` (mesuré : TypeError `then` of
    // Undefined en fin de bootstrap application.js).
    let (mut e, _jar, _n) = engine_full(PAGE);
    evf(&mut e, "Document.prototype.ready = Promise.resolve('ok');");
    assert_eq!(evf(&mut e, "document.ready instanceof Promise"), "true");
    // Propriétés propres du host prioritaires sur le proto.
    assert_eq!(evf(&mut e, "typeof document.readyState"), "string");
    assert_eq!(evf(&mut e, "document instanceof Document"), "true");
}

#[test]
fn storage_opt_chain_method_dispatch() {
    // dev.to `paginate` : `localStorage?.getItem(...)` levait
    // `TypeError: value is not a function` — le chemin `?.` lisait
    // la prop sans consulter `Host::invoke`.
    let (mut e, _jar, _n) = engine_full(PAGE);
    evf(&mut e, "localStorage.setItem('f', 'feed');");
    assert_eq!(evf(&mut e, "localStorage?.getItem('f')"), "feed");
    assert_eq!(evf(&mut e, "localStorage?.getItem('nope')"), "null");
    assert_eq!(evf(&mut e, "localStorage?.key(0)"), "f");
    // Chaîne mixte : méthode host après un `?.` amont.
    assert_eq!(evf(&mut e, "globalThis?.localStorage.getItem('f')"), "feed");
    assert_eq!(
        evf(&mut e, "globalThis?.localStorage?.getItem('f')"),
        "feed"
    );
    // Méthode absente du host → `value is not a function` honnête.
    match e.eval("localStorage?.nope()") {
        Err(err) => {
            let s = format!("{err}");
            assert!(s.contains("not a function"), "{s}");
        }
        Ok(v) => panic!("localStorage?.nope() -> {}", e.describe(v)),
    }
    // Base nullish → court-circuit `undefined` comme avant.
    assert_eq!(evf(&mut e, "var u; u?.getItem('f')"), "undefined");
}

#[test]
fn window_location_getters() {
    let (mut e, _jar, _n) = engine_full(PAGE);
    assert_eq!(
        evf(&mut e, "location.href"),
        "http://ex.org:8080/a/b?q=1#frag"
    );
    assert_eq!(evf(&mut e, "location.protocol"), "http:");
    assert_eq!(evf(&mut e, "location.host"), "ex.org:8080");
    assert_eq!(evf(&mut e, "location.hostname"), "ex.org");
    assert_eq!(evf(&mut e, "location.port"), "8080");
    assert_eq!(evf(&mut e, "location.pathname"), "/a/b");
    assert_eq!(evf(&mut e, "location.search"), "?q=1");
    assert_eq!(evf(&mut e, "location.hash"), "#frag");
    assert_eq!(evf(&mut e, "location.origin"), "http://ex.org:8080");
    // window aliases + same object.
    assert_eq!(evf(&mut e, "window.location === location"), "true");
    assert_eq!(
        evf(&mut e, "window === self && self === globalThis"),
        "true"
    );
}

#[test]
fn window_location_navigation() {
    let (mut e, _jar, navs) = engine_full(PAGE);
    evf(
        &mut e,
        "location.href = '/next'; location.assign('/a2'); location.replace('/r');",
    );
    // reload navigates to the current URL.
    evf(&mut e, "location.reload();");
    assert_eq!(
        navs.borrow().as_slice(),
        ["/next", "/a2", "/r", "http://ex.org:8080/a/b?q=1#frag"]
    );
}

#[test]
fn window_history() {
    let (mut e, _jar, navs) = engine_full(PAGE);
    assert_eq!(evf(&mut e, "history.length"), "3");
    evf(
        &mut e,
        "history.back(); history.forward(); history.go(-2); history.go(0);",
    );
    // go(0) = reload — not a relative move, ignored by the hook.
    assert_eq!(navs.borrow().as_slice(), ["go:-1", "go:1", "go:-2"]);
    // Références de méthode lisibles : les wraps SPA lisent
    // `history.pushState` en valeur puis `e.call(history,…)`
    // (turbo — github.com).
    assert_eq!(evf(&mut e, "typeof history.pushState"), "function");
    assert_eq!(evf(&mut e, "typeof history.go"), "function");
    evf(
        &mut e,
        "var p = history.pushState; p.call(history, {t:1}, '', '/via-call');",
    );
    assert_eq!(
        evf(&mut e, "location.pathname || location.href"),
        "/via-call"
    );
    evf(&mut e, "var g = history.go; g.call(history, -1);");
    assert_eq!(
        navs.borrow().as_slice(),
        ["go:-1", "go:1", "go:-2", "go:-1"]
    );
}

// -- JS-7f window surface ------------------------------------------------

#[test]
fn console_hook_receives_lines() {
    let lines = Rc::new(RefCell::new(Vec::<String>::new()));
    let sink = lines.clone();
    let mut e = Engine::new();
    e.set_console_hook(Rc::new(move |line: String| sink.borrow_mut().push(line)));
    e.eval("console.log('a', 1, 'b'); console.warn('w');")
        .expect("eval");
    assert_eq!(lines.borrow().as_slice(), ["a 1 b", "w"]);
}

#[test]
fn console_absent_hook_is_noop() {
    // No hook installed — `console.log` still parses and runs.
    let mut e = engine_with("");
    e.eval("console.log('x'); 'ok'").expect("eval");
}

#[test]
fn dialogs_route_to_hooks() {
    use std::cell::Cell;
    let mut e = engine_with("");
    let seen = Rc::new(RefCell::new(Vec::<String>::new()));
    let seen2 = seen.clone();
    let answered = Rc::new(Cell::new(true));
    let answered2 = answered.clone();
    e.set_dialog_hooks(
        Rc::new(move |m: String| seen2.borrow_mut().push(m)),
        Rc::new(move |_m: String| answered2.get()),
        Rc::new(move |m: String, d: String| Some(format!("{m}:{d}"))),
    );
    e.eval("alert('hi');").expect("alert");
    assert_eq!(seen.borrow().as_slice(), ["hi"]);
    assert_eq!(
        e.eval("confirm('q?')")
            .map(|v| e.describe(v))
            .unwrap_or_default(),
        "true"
    );
    assert_eq!(
        e.eval("prompt('name', 'def')")
            .map(|v| e.describe(v))
            .unwrap_or_default(),
        "name:def"
    );
}

#[test]
fn dialogs_absent_degrade() {
    // No hooks → alert no-op, confirm false, prompt null.
    assert_eq!(ev("confirm('x')"), "false");
    assert_eq!(ev("prompt('x')"), "null");
    assert_eq!(ev("alert('x'); 'ran'"), "ran");
}

#[test]
fn window_open_and_screen() {
    use std::cell::RefCell as RC;
    let opened = Rc::new(RC::new(Vec::<(String, String)>::new()));
    let o = opened.clone();
    let mut e = Engine::new();
    e.set_viewport(1280, 800);
    e.install_dom(&Rc::new(RC::new(Dom::from_html(""))));
    e.set_open_hook(Rc::new(move |u: String, n: String| {
        o.borrow_mut().push((u, n));
    }));
    e.eval("window.open('http://x/');window.open('http://y/','auth');")
        .expect("open");
    assert_eq!(
        opened.borrow().as_slice(),
        [
            ("http://x/".to_string(), String::new()),
            ("http://y/".to_string(), "auth".to_string())
        ]
    );
    assert_eq!(
        e.eval("innerWidth")
            .map(|v| e.describe(v))
            .unwrap_or_default(),
        "1280"
    );
    assert_eq!(
        e.eval("screen.height")
            .map(|v| e.describe(v))
            .unwrap_or_default(),
        "800"
    );
    assert_eq!(
        e.eval("screen.colorDepth")
            .map(|v| e.describe(v))
            .unwrap_or_default(),
        "24"
    );
    assert_eq!(
        e.eval("devicePixelRatio")
            .map(|v| e.describe(v))
            .unwrap_or_default(),
        "1"
    );
}

// ---------------------------------------------------------------
// JS-7g — DOM traversal, selectors, mutation, collections
// ---------------------------------------------------------------

#[test]
fn traversal_props() {
    assert_eq!(
        ev("document.getElementById('list').firstChild.tagName"),
        "LI"
    );
    assert_eq!(
        ev("document.getElementById('list').lastChild.textContent"),
        "b"
    );
    assert_eq!(ev("document.getElementById('list').children.length"), "2");
    assert_eq!(ev("document.getElementById('list').childNodes.length"), "2");
    assert_eq!(
        ev("document.getElementById('list').children[0].nextSibling.textContent"),
        "b"
    );
    assert_eq!(
        ev("document.getElementById('list').children[1].previousSibling.textContent"),
        "a"
    );
    assert_eq!(
        ev("document.getElementById('main').children[0].parentNode.id"),
        "main"
    );
    // Text nodes are skipped by `children` but counted by childNodes.
    assert_eq!(
        ev_in(
            "<div id='x'>t<b>y</b></div>",
            "document.getElementById('x').childNodes.length"
        ),
        "2"
    );
    assert_eq!(
        ev_in(
            "<div id='x'>t<b>y</b></div>",
            "document.getElementById('x').children.length"
        ),
        "1"
    );
}

#[test]
fn matches_closest_query() {
    assert_eq!(ev("document.getElementById('main').matches('div')"), "true");
    assert_eq!(
        ev("document.getElementById('main').matches('.box')"),
        "true"
    );
    assert_eq!(
        ev("document.getElementById('main').matches('#main')"),
        "true"
    );
    assert_eq!(
        ev("document.getElementById('main').matches('span')"),
        "false"
    );
    assert_eq!(ev("document.querySelector('#list li').tagName"), "LI");
    assert_eq!(ev("document.querySelectorAll('li').length"), "2");
    assert_eq!(
        ev("document.querySelectorAll('#list li')[1].textContent"),
        "b"
    );
    // closest walks ancestors.
    assert_eq!(
        ev("document.getElementById('list').children[0].closest('ul').id"),
        "list"
    );
    assert_eq!(
        ev("document.getElementById('list').children[0].closest('div.nope')"),
        "null"
    );
    // Element-scoped query.
    assert_eq!(
        ev("document.getElementById('main').querySelectorAll('b').length"),
        "1"
    );
}

#[test]
fn mutation_methods() {
    assert_eq!(
        ev("var l = document.getElementById('list'); var li = document.createElement('li'); li.textContent = 'c'; l.appendChild(li); l.children.length"),
        "3"
    );
    assert_eq!(
        ev("var l = document.getElementById('list'); var li = document.createElement('li'); l.prepend(li); l.children[0] === li"),
        "true"
    );
    // Issue #17 — move d'un nœud déjà parenté : `prepend`
    // détache puis insère en tête (pas un append).
    assert_eq!(
        ev("var b=document.body; var l=document.getElementById('list'); b.prepend(l); b.children[0]===l"),
        "true"
    );
    // …et le cas du bug réel : `prepend` du PREMIER enfant —
    // `ref == child`, l'insert doit viser son prochain frère
    // (sans fix : détaché → ref introuvable → append en queue).
    assert_eq!(
        ev("var b=document.body; var f=b.children[0]; b.prepend(f); b.children[0]===f"),
        "true"
    );
    // Ordre des args multiples préservé.
    assert_eq!(
        ev("var l=document.getElementById('list'); var a=document.createElement('li'); var c=document.createElement('li'); l.prepend(a,c); l.children[0]===a && l.children[1]===c"),
        "true"
    );
    assert_eq!(
        ev("var l = document.getElementById('list'); var n = document.createElement('li'); l.children[0].before(n); l.children[0] === n"),
        "true"
    );
    assert_eq!(
        ev("var l = document.getElementById('list'); var n = document.createElement('li'); l.children[0].after(n); l.children[1] === n"),
        "true"
    );
    assert_eq!(
        ev("var l = document.getElementById('list'); var n = document.createElement('li'); l.children[0].replaceWith(n); l.children[0] === n"),
        "true"
    );
    assert_eq!(
        ev("var l = document.getElementById('list'); var c = l.children[0]; l.removeChild(c); l.children.length"),
        "1"
    );
    assert_eq!(
        ev("var l = document.getElementById('list'); var n = document.createElement('li'); l.replaceChild(n, l.children[1]); l.children[1] === n"),
        "true"
    );
    assert_eq!(
        ev("var l = document.getElementById('list'); l.children[0].remove(); l.children.length"),
        "1"
    );
    // insertBefore with ref + at end.
    assert_eq!(
        ev("var l = document.getElementById('list'); var n = document.createElement('li'); l.insertBefore(n, l.children[1]); l.children[1] === n"),
        "true"
    );
}

/// Issue #16 — `Element.toggleAttribute(name[, force])` :
/// bascule sans force, impose avec, retourne l'état final.
#[test]
fn toggle_attribute() {
    assert_eq!(
        ev("var m=document.getElementById('main'); m.toggleAttribute('hidden')"),
        "true"
    );
    assert_eq!(
        ev("var m=document.getElementById('main'); m.setAttribute('hidden',''); m.toggleAttribute('hidden')"),
        "false"
    );
    // `force` impose l'état (idempotent).
    assert_eq!(
        ev("var m=document.getElementById('main'); m.toggleAttribute('hidden',true) && m.hasAttribute('hidden') && m.toggleAttribute('hidden',true)"),
        "true"
    );
    assert_eq!(
        ev("var m=document.getElementById('main'); m.setAttribute('hidden','x'); m.toggleAttribute('hidden',false)===false && !m.hasAttribute('hidden')"),
        "true"
    );
    // `undefined` explicite = force absent → bascule.
    assert_eq!(
        ev("var m=document.getElementById('main'); m.toggleAttribute('hidden',undefined)"),
        "true"
    );
}

/// Issue #20 — `style.setProperty` : les propriétés custom
/// `--*` gardent leur casse (css-variables-1 §2.1 — miroir
/// du fix cascade R-92), les standard sont minuscules.
#[test]
fn style_set_property_custom_case() {
    assert_eq!(
        ev("var m=document.getElementById('main'); m.style.setProperty('--Spacer-size1012','660px'); m.style.getPropertyValue('--Spacer-size1012')"),
        "660px"
    );
    // La lecture sous une autre casse NE retrouve PAS la var
    // (custom properties = case-sensitive).
    assert_eq!(
        ev("var m=document.getElementById('main'); m.style.setProperty('--Spacer-size1012','660px'); m.style.getPropertyValue('--spacer-size1012')"),
        ""
    );
    // Standard : minuscules toujours.
    assert_eq!(
        ev("var m=document.getElementById('main'); m.style.setProperty('MARGIN-TOP','3px'); m.style.getPropertyValue('margin-top')"),
        "3px"
    );
    // removeProperty tient le même nommage.
    assert_eq!(
        ev("var m=document.getElementById('main'); m.style.setProperty('--A','1'); m.style.removeProperty('--A'); m.style.getPropertyValue('--A')"),
        ""
    );
}

#[test]
fn clone_and_adjacent_mutations() {
    // cloneNode shallow vs deep.
    assert_eq!(
        ev("var c = document.getElementById('list').cloneNode(false); c.children.length"),
        "0"
    );
    assert_eq!(
        ev("var c = document.getElementById('list').cloneNode(true); c.children.length"),
        "2"
    );
    // insertAdjacentHTML positions.
    assert_eq!(
        ev("var l = document.getElementById('list'); l.insertAdjacentHTML('beforeend', '<li>z</li>'); l.children[2].textContent"),
        "z"
    );
    assert_eq!(
        ev("var l = document.getElementById('list'); l.insertAdjacentHTML('afterbegin', '<li>z</li>'); l.children[0].textContent"),
        "z"
    );
    assert_eq!(
        ev("var m = document.getElementById('main'); m.insertAdjacentText('beforeend', 'tail'); m.textContent.indexOf('tail') >= 0"),
        "true"
    );
}

#[test]
fn collections_and_document_props() {
    assert_eq!(
        ev_in(
            "<a href='x'>1</a><a href='y'>2</a><img src='i'><form id='f'></form>",
            "document.links.length + '/' + document.images.length + '/' + document.forms.length"
        ),
        "2/1/1"
    );
    assert_eq!(
        ev_in(
            "<a href='x'>1</a><a href='y'>2</a>",
            "document.links[0].tagName"
        ),
        "A"
    );
    // CAP-170 : `readyState` reflète le cycle de vie —
    // sans embedder qui pousse les transitions, le document
    // reste honnêtement `loading` (plus de `"complete"`
    // codé en dur).
    assert_eq!(ev("document.readyState"), "loading");
    assert_eq!(ev("document.head.tagName"), "HEAD");
    assert_eq!(ev("document.body.tagName"), "BODY");
}

#[test]
fn outer_html_and_dataset() {
    assert_eq!(
        ev("document.getElementById('list').outerHTML.indexOf('<ul')"),
        "0"
    );
    assert_eq!(
        ev_in(
            "<div id='d' data-user-id='42' data-foo='bar'></div>",
            "var d = document.getElementById('d'); d.dataset.userId + '/' + d.dataset.foo"
        ),
        "42/bar"
    );
    assert_eq!(
        ev_in(
            "<div id='d'></div>",
            "var d = document.getElementById('d'); d.dataset.newKey = 'v'; d.getAttribute('data-new-key')"
        ),
        "v"
    );
    assert_eq!(
        ev("document.body.contains(document.getElementById('list'))"),
        "true"
    );
    assert_eq!(
        ev("document.getElementById('list').contains(document.body)"),
        "false"
    );
}

// -- MutationObserver (JS-C26) -----------------------------------------
//
// Les callbacks sont livrés au checkpoint microtask (fin
// d'`eval`) — le test lit leurs effets via une sonde `eval`
// séparée.

/// `setup` puis `probe` sur le même moteur — le probe lit les
/// effets des callbacks `MutationObserver` livrés au checkpoint
/// du premier eval.
fn ev2(html: &str, setup: &str, probe: &str) -> String {
    let mut e = engine_with(html);
    match e.eval(setup) {
        Ok(_) => {}
        Err(err) => panic!("{setup} -> {err}"),
    }
    match e.eval(probe) {
        Ok(v) => e.describe(v),
        Err(err) => panic!("{probe} -> {err}"),
    }
}

#[test]
fn mutation_observer_child_list() {
    assert_eq!(
        ev2(
            PAGE,
            "var log=[];\
             var mo=new MutationObserver(function(rs,o){\
               log.push(rs[0].type+'|'+rs[0].addedNodes.length+'|'+(o===mo));\
             });\
             mo.observe(document.body,{childList:true});\
             document.body.appendChild(document.createElement('i'));",
            "log.join(',')"
        ),
        "childList|1|true"
    );
}

#[test]
fn mutation_observer_attributes_and_old_value() {
    assert_eq!(
        ev2(
            PAGE,
            "var log=[];\
             var mo=new MutationObserver(function(rs){\
               log.push(rs[0].type+'|'+rs[0].attributeName+'|'+rs[0].oldValue);\
             });\
             var el=document.getElementById('main');\
             mo.observe(el,{attributes:true,attributeOldValue:true});\
             el.setAttribute('class','new');",
            "log.join(',')"
        ),
        "attributes|class|box"
    );
}

#[test]
fn mutation_observer_attribute_filter() {
    assert_eq!(
        ev2(
            PAGE,
            "var n=0;\
             var mo=new MutationObserver(function(){n++});\
             var el=document.getElementById('main');\
             mo.observe(el,{attributes:true,attributeFilter:['class']});\
             el.setAttribute('id','z');\
             el.setAttribute('class','q');\
             el.removeAttribute('title');",
            "n"
        ),
        "1"
    );
}

#[test]
fn mutation_observer_subtree() {
    assert_eq!(
        ev2(
            PAGE,
            "var n=0;\
             var mo=new MutationObserver(function(){n++});\
             mo.observe(document.body,{childList:true,subtree:true});\
             document.getElementById('list').appendChild(document.createElement('li'));\
             document.getElementById('inp').setAttribute('value','x');",
            "n"
        ),
        "1"
    );
}

#[test]
fn mutation_observer_batches_once_per_checkpoint() {
    assert_eq!(
        ev2(
            PAGE,
            "var calls=0,total=0;\
             var mo=new MutationObserver(function(rs){calls++;total+=rs.length});\
             mo.observe(document.body,{childList:true});\
             document.body.appendChild(document.createElement('i'));\
             document.body.appendChild(document.createElement('b'));\
             document.body.appendChild(document.createElement('u'));",
            "calls+'/'+total"
        ),
        "1/3"
    );
}

#[test]
fn mutation_observer_disconnect_stops_delivery() {
    assert_eq!(
        ev2(
            PAGE,
            "var n=0;\
             var mo=new MutationObserver(function(){n++});\
             mo.observe(document.body,{childList:true});\
             document.body.appendChild(document.createElement('i'));\
             mo.disconnect();\
             document.body.appendChild(document.createElement('b'));",
            "n"
        ),
        // disconnect purge la file : le record déjà produit
        // n'est pas livré non plus (spec).
        "0"
    );
}

#[test]
fn mutation_observer_take_records() {
    assert_eq!(
        ev2(
            PAGE,
            "var called=0,len=0;\
             var mo=new MutationObserver(function(){called++});\
             mo.observe(document.body,{childList:true});\
             document.body.appendChild(document.createElement('i'));\
             len=mo.takeRecords().length;\
             document.body.appendChild(document.createElement('b'));",
            "len+'/'+called"
        ),
        // takeRecords vide la file — le record suivant est
        // livré normalement au checkpoint.
        "1/1"
    );
}

#[test]
fn mutation_observer_character_data() {
    assert_eq!(
        ev2(
            PAGE,
            "var log='';\
             var tn=document.createTextNode('abc');\
             var mo=new MutationObserver(function(rs){\
               log=rs[0].type+'|'+rs[0].oldValue;\
             });\
             mo.observe(tn,{characterData:true,characterDataOldValue:true});\
             tn.data='xyz';",
            "log"
        ),
        "characterData|abc"
    );
}

#[test]
fn mutation_observer_callback_can_mutate_again() {
    assert_eq!(
        ev2(
            PAGE,
            "var n=0;\
             var mo=new MutationObserver(function(){\
               if(n++<2){document.body.appendChild(document.createElement('i'))}\
             });\
             mo.observe(document.body,{childList:true});\
             document.body.appendChild(document.createElement('b'));",
            "n"
        ),
        // 1re livraison → le callback remute → nouvelle file →
        // 2e livraison au même checkpoint (boucle bornée).
        "3"
    );
}

#[test]
fn mutation_observer_survives_gc() {
    // Le callback n'est référencé que par `mutobs.cbs` — un
    // GC entre observe et mutation ne doit pas le balayer.
    let mut e = engine_with(PAGE);
    e.eval(
        "var log=[];\
         new MutationObserver(function(){log.push(1)})\
           .observe(document.body,{childList:true});",
    )
    .unwrap_or_else(|err| panic!("setup -> {err}"));
    e.collect();
    e.eval("document.body.appendChild(document.createElement('i'));")
        .unwrap_or_else(|err| panic!("mutate -> {err}"));
    match e.eval("log.join(',')") {
        Ok(v) => assert_eq!(e.describe(v), "1"),
        Err(err) => panic!("probe -> {err}"),
    }
}

#[test]
fn document_fragment_splice_et_serialise() {
    // Trigger réel : jQuery (`document.createDocumentFragment`
    // manquait — arstechnica.com, census terrain). Spec : un
    // fragment inséré éclate ses enfants dans le parent et se
    // vide ; `innerHTML` d'un fragment = ses enfants.
    assert_eq!(
        ev2(
            PAGE,
            "var f=document.createDocumentFragment();\
             f.appendChild(document.createElement('b'));\
             f.appendChild(document.createTextNode('x'));\
             document.getElementById('main').appendChild(f);",
            "f.childNodes.length+'|'+f.nodeType+'|'+document.getElementById('main').innerHTML"
        ),
        "0|11|<p>Hello <b>world</b></p><b></b>x"
    );
    // `insertBefore` au milieu : les enfants prennent le slot.
    assert_eq!(
        ev2(
            PAGE,
            "var f=document.createDocumentFragment();\
             var a=document.createElement('a'),b=document.createElement('b');\
             f.appendChild(a);f.appendChild(b);\
             var m=document.getElementById('main');\
             m.insertBefore(f,m.firstChild);",
            "document.getElementById('main').innerHTML"
        ),
        "<a></a><b></b><p>Hello <b>world</b></p>"
    );
    // `innerHTML` d'un fragment rempli = ses enfants, sans
    // balise englobante (sérialisation spec).
    assert_eq!(
        ev2(
            PAGE,
            "var f=document.createDocumentFragment();\
             f.appendChild(document.createElement('i'));\
             window.__h=f.innerHTML;",
            "window.__h"
        ),
        "<i></i>"
    );
}

#[test]
fn document_location_egale_window() {
    // `document.location === window.location` — trigger réel :
    // scripts lisant `document.location.search` (arstechnica).
    assert_eq!(
        ev2(PAGE, "", "document.location === window.location"),
        "true"
    );
    assert_eq!(ev2(PAGE, "", "typeof document.location.search"), "string");
}

#[test]
fn mutation_observer_document_target() {
    // `observe(document)` : `document` est un DocumentHost, pas un
    // NodeHost — il résout sur la racine du miroir. Mesuré sur
    // github.com (JS-C56).
    assert_eq!(
        ev2(
            PAGE,
            "var log=[];\
             var mo=new MutationObserver(function(rs){\
               log.push(rs[0].type+'|'+(rs[0].target===document.body));\
             });\
             mo.observe(document,{childList:true,subtree:true});\
             document.body.appendChild(document.createElement('i'));",
            "log.join(',')"
        ),
        "childList|true"
    );
    // Options inférées : attributeFilter implique attributes.
    assert_eq!(
        ev2(
            PAGE,
            "var n=0;\
             var mo=new MutationObserver(function(){n++});\
             mo.observe(document,{attributeFilter:['class'],subtree:true});\
             document.getElementById('main').setAttribute('class','z');",
            "n"
        ),
        "1"
    );
    // Config sans type d'observation → TypeError (spec).
    assert_eq!(
        ev2(
            PAGE,
            "var r='';try{new MutationObserver(function(){}).observe(document,{})}\
             catch(e){r=e.name}",
            "r"
        ),
        "TypeError"
    );
}

#[test]
fn dom_interface_constructors() {
    // Interfaces DOM standard : présentes pour la détection
    // (`typeof`, `in prototype`, `instanceof`) — stubs
    // inconstructibles sauf `Event` et sa famille. Mesuré sur
    // github.com : ReferenceError `HTMLElement`/`Event` (JS-C56).
    assert_eq!(
        ev2(
            PAGE,
            "var r=['Element','HTMLElement','SVGElement','Document',\
              'DocumentFragment','ShadowRoot','Text','Comment',\
              'NodeList','HTMLCollection','DOMTokenList','Range',\
              'DOMParser','IntersectionObserver','ResizeObserver',\
              'DOMException','AbortSignal','EventTarget'].map(function(n){\
               return typeof globalThis[n]}).join(',')",
            "r"
        ),
        "function,function,function,function,function,function,function,function,function,function,function,function,function,function,function,function,function,function"
    );
    // Inconstructibles → TypeError.
    assert_eq!(
        ev2(
            PAGE,
            "var r='';try{new HTMLElement()}catch(e){r=e.name}\
             r+='|';try{new Document()}catch(e){r+=e.name}",
            "r"
        ),
        "TypeError|TypeError"
    );
    // `Event` constructible, dispatchable, `instanceof` honnête.
    assert_eq!(
        ev2(
            PAGE,
            "var e=new Event('click',{bubbles:true});\
             var r=e.type+'|'+e.bubbles+'|'+e.cancelable+'|'+\
             (e instanceof Event)+'|'+e.defaultPrevented+'|'+e.isTrusted",
            "r"
        ),
        "click|true|false|true|false|false"
    );
    // Sous-classes : proto chaîné sur Event.prototype, payload
    // d'init copié (CustomEvent.detail).
    assert_eq!(
        ev2(
            PAGE,
            "var ce=new CustomEvent('load',{detail:{a:7}});\
             var r=(ce instanceof Event)+'|'+(ce instanceof CustomEvent)+'|'+ce.detail.a+'|'+\
             (new MouseEvent('m') instanceof Event)",
            "r"
        ),
        "true|true|7|true"
    );
    // Méthodes du prototype Event.
    assert_eq!(
        ev2(
            PAGE,
            "var e=new Event('x');e.preventDefault();e.stopPropagation();\
             var r=e.defaultPrevented+'|'+e.cancelBubble+'|'+e.returnValue+'|'+\
             e.composedPath().length+'|'+Event.BUBBLING_PHASE",
            "r"
        ),
        "true|true|false|0|3"
    );
    // dispatchEvent accepte un event script-créé.
    assert_eq!(
        ev2(
            PAGE,
            "var seen='';\
             document.body.addEventListener('ping',function(ev){seen=ev.type+'|'+ev.bubbles});\
             document.body.dispatchEvent(new Event('ping',{bubbles:true}))",
            "seen"
        ),
        "ping|true"
    );
    // AbortController minimal : signal.aborted + abort().
    assert_eq!(
        ev2(
            PAGE,
            "var c=new AbortController();var s=c.signal;\
             var r=s.aborted+'|'+typeof c.abort",
            "r"
        ),
        "false|function"
    );
    assert_eq!(
        ev2(
            PAGE,
            "var c=new AbortController();c.abort();var r=c.signal.aborted",
            "r"
        ),
        "true"
    );
    // `IntersectionObserver`/`ResizeObserver` — constructibles,
    // callback requis, méthodes no-op honnêtes (le miroir ne
    // génère ni intersection ni resize).
    assert_eq!(
        ev2(
            PAGE,
            "var io=new IntersectionObserver(function(){});\
             var ro=new ResizeObserver(function(){});\
             io.observe(document.body);ro.observe(document.body);\
             io.unobserve(document.body);ro.disconnect();\
             var r=io.takeRecords().length+'|'+typeof io",
            "r"
        ),
        "0|object"
    );
    // Sans callback → TypeError.
    assert_eq!(
        ev2(
            PAGE,
            "var r='';try{new IntersectionObserver()}catch(e){r=e.name}\
             r+='|';try{new ResizeObserver(42)}catch(e){r+=e.name}",
            "r"
        ),
        "TypeError|TypeError"
    );
}

/// Surface des events *dispatchés* (host object, JS-C59 — trou
/// remonté par github.com : `composedPath is not a function`) :
/// chemin réel, phases, trusted, flags — le chemin script-créé
/// (`new Event` → `Event.prototype`) était déjà couvert par
/// `dom_interface_constructors`.
#[test]
fn event_dispatche_surface_honnete() {
    assert_eq!(
        ev2(
            PAGE,
            "var r='';var el=document.getElementById('main');\
             el.addEventListener('x',function(e){\
             var p=e.composedPath();\
             r=p.length+'|'+p[0].id+'|'+e.eventPhase+'|'+e.isTrusted+'|'+\
             (typeof e.timeStamp)+'|'+(e.timeStamp>=0)});\
             el.dispatchEvent(new Event('x',{bubbles:true}));\
             r=r+'|'+(new Event('x').composedPath().length)",
            "r"
        ),
        "4|main|2|false|number|true|0"
    );
    // `stopPropagation` épargne les listeners frères du nœud
    // (spec) ; `stopImmediatePropagation` les tue. L'ancêtre
    // meurt dans les deux cas.
    assert_eq!(
        ev2(
            PAGE,
            "var r=0;var el=document.getElementById('main');\
             el.addEventListener('x',function(e){e.stopPropagation();r+=1});\
             el.addEventListener('x',function(e){r+=10});\
             document.body.addEventListener('x',function(e){r+=100});\
             el.dispatchEvent(new Event('x',{bubbles:true}));r",
            "r"
        ),
        "11"
    );
    assert_eq!(
        ev2(
            PAGE,
            "var r=0;var el=document.getElementById('main');\
             el.addEventListener('x',function(e){e.stopImmediatePropagation();r+=1});\
             el.addEventListener('x',function(e){r+=10});\
             document.body.addEventListener('x',function(e){r+=100});\
             el.dispatchEvent(new Event('x',{bubbles:true}));r",
            "r"
        ),
        "1"
    );
    // `preventDefault` est no-op sans `cancelable` (spec) ;
    // `dispatchEvent` rend `!defaultPrevented`.
    assert_eq!(
        ev2(
            PAGE,
            "var el=document.getElementById('main');\
             el.addEventListener('x',function(e){e.preventDefault()});\
             var r=(!el.dispatchEvent(new Event('x')))+'|'+\
             (!el.dispatchEvent(new Event('x',{cancelable:true})))",
            "r"
        ),
        "false|true"
    );
    // `Element.prototype.matches` en *valeur* — les shims
    // lisent la référence puis `.call(el,…)` (selector-observer
    // github, JS-C59) ; receveur non-Element → TypeError.
    assert_eq!(
        ev2(
            PAGE,
            "var o=Element.prototype.matches||Element.prototype.msMatchesSelector;\
             var r=typeof o+'|'+o.call(document.getElementById('main'),'#main')+'|'+\
             o.call(document.getElementById('main'),'#nope')+'|'+\
             Element.prototype.closest.call(document.getElementById('main'),'body').tagName;\
             try{Element.prototype.matches.call(document,'x')}catch(e){r+='|'+e.name}",
            "r"
        ),
        "function|true|false|BODY|TypeError"
    );
    // `e.matches?.(sel)` — appel optionnel sur membre : le
    // récepteur `e` est conservé (trigger réel github.com :
    // `t.find(e=>e.matches?.("…"))` dans le polyfill popover,
    // `t` = composedPath → éléments, #document et window).
    assert_eq!(
        ev2(
            PAGE,
            "var p=document.getElementById('main');             var r=p.matches?.('#main')+'|'+p.matches?.('span')+'|'+             ([p,document.body,document,window]             .find(e=>e.matches?.('body'))===document.body)+'|'+             ([window,document].find(e=>e.matches?.('x')))",
            "r"
        ),
        "true|false|true|undefined"
    );
}

#[test]
fn custom_elements_registry() {
    // `customElements` — registre minimal (JS-C56, github.com).
    assert_eq!(
        ev2(
            PAGE,
            "class MyEl extends HTMLElement {};\
             var r=''+typeof customElements;\
             r+='|'+customElements.get('my-el');\
             customElements.define('my-el',MyEl);\
             r+='|'+(customElements.get('my-el')===MyEl)",
            "r"
        ),
        "object|undefined|true"
    );
    // ctor non-fonction → TypeError ; nom déjà pris →
    // `NotSupportedError` DOMException (spec + catalyst).
    assert_eq!(
        ev2(
            PAGE,
            "customElements.define('x-el',class{});\
             var r='';try{customElements.define('x-el',class{})}catch(e){r=e.name}\
             r+='|';try{customElements.define('y-el',42)}catch(e){r+=e.name}",
            "r"
        ),
        "NotSupportedError|TypeError"
    );
    // `whenDefined` — promesse tenue `undefined` pour un nom
    // enregistré (spec — l'ancien stub résolvait le ctor).
    assert_eq!(
        ev2(
            PAGE,
            "customElements.define('w-el',class{});\
             var r='pending';\
             customElements.whenDefined('w-el').then(function(c){r=c===undefined})",
            "r"
        ),
        "true"
    );
    // `upgrade` — upgrade réel du sous-arbre (JS-C68), pas de throw.
    assert_eq!(
        ev2(
            PAGE,
            "customElements.upgrade(document.body);var r='ok'",
            "r"
        ),
        "ok"
    );
}

/// Motif `patchDefine` de reddit (census shadow DOM — chantier
/// tranche 1) : le shim lit `customElements.define` pour le
/// wrapper — invoke-only le rendait `undefined` et
/// `'_patched' in undefined` tuait le patch d'un `TypeError`.
/// Attendus : méthode lisible (`typeof` function), `in` sans
/// throw, shadow propre `t.define = f` qui s'installe, et
/// l'originale restée joignable via `n.call(t, …)` (le
/// trampoline re-dispatche l'`invoke`).
#[test]
fn custom_elements_methods_readable_and_patchable() {
    assert_eq!(
        ev2(
            PAGE,
            "var t=window.customElements;\
             var n=t.define;\
             var r=''+typeof n;\
             r+='|'+('_patched' in n);\
             t.define=function(a,o,s){window.__ALL_ELEMENTS=window.__ALL_ELEMENTS||[];\
                 window.__ALL_ELEMENTS.push(a);return n.call(t,a,o,s)};\
             class MyP extends HTMLElement {}\
             t.define('p-el',MyP);\
             r+='|'+(customElements.get('p-el')===MyP);\
             r+='|'+window.__ALL_ELEMENTS.join(',');\
             r+='|'+(t.get('p-el')===MyP)",
            "r"
        ),
        "function|false|true|p-el|true"
    );
}

// -- constraint validation (étape 54) -----------------------------------

const FORM_PAGE: &str = r#"
<html><body>
<form id="f" action="/go">
<input id="q" name="q" required>
<input id="n" type="number" name="n" min="1" max="10" value="42">
<input id="p" name="p" pattern="[0-9]+" value="abc">
<input id="ok" name="ok" value="fine">
</form>
</body></html>
"#;

#[test]
fn validity_flags_on_invalid_controls() {
    assert_eq!(
        ev_in(
            FORM_PAGE,
            "document.getElementById('q').validity.valueMissing"
        ),
        "true"
    );
    assert_eq!(
        ev_in(
            FORM_PAGE,
            "document.getElementById('n').validity.rangeOverflow"
        ),
        "true"
    );
    assert_eq!(
        ev_in(
            FORM_PAGE,
            "document.getElementById('p').validity.patternMismatch"
        ),
        "true"
    );
    assert_eq!(
        ev_in(FORM_PAGE, "document.getElementById('ok').validity.valid"),
        "true"
    );
}

#[test]
fn check_validity_and_will_validate() {
    assert_eq!(
        ev_in(FORM_PAGE, "document.getElementById('f').checkValidity()"),
        "false"
    );
    assert_eq!(
        ev_in(FORM_PAGE, "document.getElementById('ok').checkValidity()"),
        "true"
    );
    assert_eq!(
        ev_in(FORM_PAGE, "document.getElementById('q').willValidate"),
        "true"
    );
    // `disabled` ne valide jamais.
    assert_eq!(
        ev_in(
            FORM_PAGE,
            "document.getElementById('q').disabled = true;\
             document.getElementById('q').willValidate"
        ),
        "false"
    );
}

#[test]
fn custom_validity_and_message() {
    assert_eq!(
        ev_in(
            FORM_PAGE,
            "var i = document.getElementById('ok');\
             i.setCustomValidity('nope');\
             i.validationMessage"
        ),
        "nope"
    );
    assert_eq!(
        ev_in(
            FORM_PAGE,
            "var i = document.getElementById('ok');\
             i.setCustomValidity('nope');\
             i.validity.customError"
        ),
        "true"
    );
    assert_eq!(
        ev_in(
            FORM_PAGE,
            "var i = document.getElementById('ok');\
             i.setCustomValidity('nope'); i.setCustomValidity('');\
             i.validity.valid"
        ),
        "true"
    );
    // L'état transite par l'attribut interne (canal miroir→moteur).
    assert_eq!(
        ev_in(
            FORM_PAGE,
            "var i = document.getElementById('ok');\
             i.setCustomValidity('nope');\
             i.getAttribute('data-explorer-cv')"
        ),
        "nope"
    );
}

#[test]
fn invalid_event_fires_on_failed_check() {
    assert_eq!(
        ev_in(
            FORM_PAGE,
            "var hits = 0;\
             document.getElementById('q').addEventListener('invalid', function(){hits++});\
             document.getElementById('f').checkValidity();\
             hits"
        ),
        "1"
    );
}

#[test]
fn report_validity_returns_bool() {
    assert_eq!(
        ev_in(FORM_PAGE, "document.getElementById('f').reportValidity()"),
        "false"
    );
    assert_eq!(
        ev_in(FORM_PAGE, "document.getElementById('ok').reportValidity()"),
        "true"
    );
}

#[test]
fn minlength_maxlength_step_and_radio_group() {
    const P: &str = r#"<html><body><form id="f">
    <input id="s" minlength="3" value="xy">
    <input id="l" maxlength="2" value="abcd">
    <input id="st" type="number" min="0" step="2" value="3">
    <input id="r1" type="radio" name="g" required>
    <input id="r2" type="radio" name="g">
    </form></body></html>"#;
    assert_eq!(
        ev_in(P, "document.getElementById('s').validity.tooShort"),
        "true"
    );
    assert_eq!(
        ev_in(P, "document.getElementById('l').validity.tooLong"),
        "true"
    );
    assert_eq!(
        ev_in(P, "document.getElementById('st').validity.stepMismatch"),
        "true"
    );
    // `required` radio : le groupe entier est en échec tant
    // qu'aucun n'est coché, valide dès que l'un l'est.
    assert_eq!(
        ev_in(P, "document.getElementById('r1').validity.valueMissing"),
        "true"
    );
    assert_eq!(
        ev_in(
            P,
            "document.getElementById('r2').checked = true;\
             document.getElementById('r1').validity.valid"
        ),
        "true"
    );
}

#[test]
fn form_submit_calls_hook_without_event() {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let mut e = engine_with(FORM_PAGE);
    {
        let c = calls.clone();
        e.set_form_submit_hook(Rc::new(move |p: explorer_jscore::FormSubmit| {
            c.borrow_mut().push(p);
        }));
    }
    // `submit()` : pas d'événement `submit` (spec), appel
    // direct du hook avec les champs.
    e.eval(
        "var hits = 0;\
         document.getElementById('f').addEventListener('submit', function(){hits++});\
         document.getElementById('f').submit();",
    )
    .unwrap_or_else(|err| panic!("submit -> {err}"));
    assert_eq!(calls.borrow().len(), 1);
    assert_eq!(e.eval("hits").map(|v| e.describe(v)).unwrap(), "0");
    let p = calls.borrow()[0].clone();
    assert_eq!(p.action, "/go");
    assert!(p.fields.iter().any(|(n, _)| n == "q"));
}

#[test]
fn request_submit_fires_event_then_hook() {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let mut e = engine_with(FORM_PAGE);
    {
        let c = calls.clone();
        e.set_form_submit_hook(Rc::new(move |p: explorer_jscore::FormSubmit| {
            c.borrow_mut().push(p);
        }));
    }
    // Formulaire invalide (required vide) → `invalid` part,
    // aucune soumission.
    e.eval(
        "var inv = 0;\
         document.getElementById('q').addEventListener('invalid', function(){inv++});\
         document.getElementById('f').requestSubmit();",
    )
    .unwrap_or_else(|err| panic!("requestSubmit -> {err}"));
    assert!(calls.borrow().is_empty());
    assert_eq!(e.eval("inv").map(|v| e.describe(v)).unwrap(), "1");
    // Formulaire rendu valide + `submit` non annulé → hook.
    e.eval(
        "var sub = 0;\
         document.getElementById('f').addEventListener('submit', function(){sub++});\
         document.getElementById('q').value = 'x';\
         document.getElementById('n').value = '5';\
         document.getElementById('p').value = '42';\
         document.getElementById('f').requestSubmit();",
    )
    .unwrap_or_else(|err| panic!("requestSubmit2 -> {err}"));
    assert_eq!(e.eval("sub").map(|v| e.describe(v)).unwrap(), "1");
    assert_eq!(calls.borrow().len(), 1);
    // `preventDefault` bloque la soumission.
    e.eval(
        "document.getElementById('f').addEventListener('submit', function(ev){ev.preventDefault()});\
         document.getElementById('f').requestSubmit();",
    )
    .unwrap_or_else(|err| panic!("requestSubmit3 -> {err}"));
    assert_eq!(calls.borrow().len(), 1);
}

/// Étape 55c — le `</tag` de fermeture rawtext est cherché
/// par scan insensible à la casse **directement** dans le
/// flux de caractères : un `to_lowercase()` sur le reste du
/// document changeait la longueur en octets des runes
/// (`ﬀ`→`ff`, `İ`→`i̇`…) et l'index ramené désalignait la
/// coupe — `</script>` raté, le contenu suivant parsé comme
/// texte hors de l'élément (JSON `<script>` peints en page,
/// wikipedia — terrain mission 6/7).
#[test]
fn rawtext_close_survives_length_shifting_runes() {
    // 'ﬀ' (U+FB00) : lowercased → "ff" — 3 octets → 2.
    let html = "<div><script>var a='ﬀx';</script><p>after</p></div>";
    let d = Dom::from_html(html);
    let s = d.serialize_document();
    assert!(
        s.contains("<p>after</p>"),
        "l'élément suivant survit intact : {s}"
    );
    // Le texte du script reste **dans** l'élément : rien de
    // brut ne fuit comme nœud texte sibling.
    assert!(s.contains("<script>var a='ﬀx';</script>"));

    // Round-trip : la re-parse ne produit aucun texte hors
    // élément (la source du script ne devient jamais un
    // `Text` visible dans le DOM resynchronisé).
    let d2 = Dom::from_html(&s);
    let s2 = d2.serialize_document();
    assert!(s2.contains("<p>after</p>"), "structure stable : {s2}");

    // Casse mélangée de la balise fermante — tolérée.
    let d3 = Dom::from_html("<div><script>var a=1;</ScRiPt><b>z</b></div>");
    assert!(d3.serialize_document().contains("<b>z</b>"));
}

/// gBCR w3.org — un document **sans `</head>` littéral**
/// (HTML5 légal : `<body>` ferme `head` implicitement) doit
/// donner `head` et `body` en frères sous `html`, jamais
/// `body` niché dans `head`. Sans la règle, chaque chemin
/// `element_path` porte un maillon `head` en trop et
/// **aucun** rect géométrie ne matche — `gBCR` 0×0 global
/// (mesuré `EXPLORER_GEOM_TRACE`, 300/300 zéro).
#[test]
fn implicit_head_close_keeps_body_sibling() {
    let d = Dom::from_html(
        "<html><head><title>t</title><meta charset=utf-8><body class=h><p>hi</p></body></html>",
    );
    let html = d.document_element().unwrap();
    let head = d.by_tag(html, "head")[0];
    let body = d.body();
    assert_eq!(d.parent(body), Some(html), "body enfant de html");
    assert_eq!(d.parent(head), Some(html), "head enfant de html");
    assert_ne!(d.parent(body), Some(head), "body n'est pas dans head");
    // Le paragraphe vit dans body — pas sous head.
    let p = d.by_tag(body, "p")[0];
    assert_eq!(d.parent(p), Some(body));

    // Contenu flux sans `<body>` littéral : la spec « after
    // head » synthétise un `<body>` — parité de chemins avec
    // le tree-builder réel.
    let d2 = Dom::from_html("<html><head><title>t</title><div>x</div></html>");
    let b2 = d2.body();
    assert_ne!(b2, d2.root, "un body implicite a été posé");
    assert_eq!(d2.nodes[b2 as usize].tag, "body");
    let div = d2.by_tag(d2.root, "div")[0];
    assert_eq!(d2.parent(div), Some(b2), "le div rejoint le body implicite");

    // `<body>` explicitement fermé dans head (cas normal) :
    // rien ne change — head et body restent frères.
    let d3 = Dom::from_html("<html><head><title>t</title></head><body><p>x</p></body></html>");
    let h3 = d3.by_tag(d3.root, "head")[0];
    assert_eq!(d3.parent(d3.body()), Some(d3.document_element().unwrap()));
    assert_eq!(d3.parent(h3), Some(d3.document_element().unwrap()));
}

/// Paires `dd`/`dt` — la spec « implied end tags » les ferme
/// mutuellement : `<dt>` puis `<dd>` sont **frères** sous `dl`,
/// jamais nichés l'un dans l'autre (w3.org : listes de
/// définitions → chemins divergents → gBCR miss classés).
#[test]
fn implicit_close_dt_dd_pairs() {
    let d = Dom::from_html("<body><dl><dt>t1<dd>d1<dt>t2<dd>d2</dl></body>");
    let dl = d.by_tag(d.root, "dl")[0];
    let dts = d.by_tag(dl, "dt");
    let dds = d.by_tag(dl, "dd");
    assert_eq!(dts.len(), 2);
    assert_eq!(dds.len(), 2);
    for n in dts.iter().chain(&dds) {
        assert_eq!(d.parent(*n), Some(dl), "dt/dd frères sous dl");
    }
    // `td` ferme `td` (same-tag, comportement conservé) et
    // `tr` ferme `td` ouvert puis `tr` — table correcte.
    let d2 = Dom::from_html("<body><table><tr><td>a<td>b<tr><td>c</table></body>");
    let trs = d2.by_tag(d2.root, "tr");
    assert_eq!(trs.len(), 2);
    let tds = d2.by_tag(d2.root, "td");
    assert_eq!(tds.len(), 3);
    assert_eq!(d2.parent(tds[0]), Some(trs[0]));
    assert_eq!(d2.parent(tds[1]), Some(trs[0]), "td b frère de td a");
    assert_eq!(d2.parent(tds[2]), Some(trs[1]), "td c sous la 2e tr");
    // Sections de table : `tbody` ferme `thead` ouvert (à
    // travers `tr`/`th`) — jamais niché sous un en-tête
    // (w3.org : tbody sous th → sous-arbre entier divergent).
    let dt =
        Dom::from_html("<body><table><thead><tr><th>h<tbody><tr><td>x<tr><td>y</table></body>");
    let table = dt.by_tag(dt.root, "table")[0];
    let thead = dt.by_tag(table, "thead")[0];
    let tbody = dt.by_tag(table, "tbody")[0];
    assert_eq!(dt.parent(thead), Some(table));
    assert_eq!(dt.parent(tbody), Some(table), "tbody frère de thead");
    let btrs = dt.by_tag(tbody, "tr");
    assert_eq!(btrs.len(), 2, "deux tr frères dans tbody");
    assert_eq!(dt.parent(btrs[1]), Some(tbody));
    // Hors contexte closable : tolérance — un `<dd>` sans
    // ancêtre closable reste niché plutôt que perdu.
    let d3 = Dom::from_html("<body><div><dd>x</dd></div></body>");
    assert_eq!(d3.by_tag(d3.root, "dd").len(), 1);
}

/// vaadin terrain — `document.styleSheets` doit être
/// itérable : le fixup lumo fait `new Set(e.styleSheets)`
/// à l'upgrade CE (`object is not iterable` sinon) —
/// `<style>` + `<link rel=stylesheet>` en ordre document,
/// `ownerNode`/`href` posés.
#[test]
fn document_style_sheets_iterable() {
    assert_eq!(
        ev_in(
            "<html><head><link rel=stylesheet href=a.css>\
             <style>.x{}</style><link rel=icon href=i.ico></head>\
             <body><style>.y{}</style></body></html>",
            "(function(){var s=document.styleSheets;var o=[];\
             for(const x of s){o.push(x.ownerNode.tagName+(x.href?'!':''))}\
             return o.join(',')+'|'+s.length})()"
        ),
        "LINK!,STYLE,STYLE|3"
    );
}

/// JS-C37/C39 — le coût de resynchronisation `serialize_document`
/// est instrumenté : appels + octets + micros produits cumulés.
#[test]
fn serialize_stats_counts_calls_and_bytes() {
    let d = Dom::from_html("<div><p>hello</p></div>");
    let st0 = d.serialize_stats();
    assert_eq!(st0.calls, 0);
    assert_eq!(st0.bytes, 0);
    let s1 = d.serialize_document();
    let s2 = d.serialize_document();
    let st = d.serialize_stats();
    assert_eq!(st.calls, 2);
    assert_eq!(st.bytes, (s1.len() + s2.len()) as u64);
    // micros mesuré (peut valoir 0 sur une horloge grossière —
    // seul le cumul monotone est contractualisé).
    assert!(st.micros < 60_000_000);
}

/// 55c — fidélité round-trip : le contenu rawtext (`script`,
/// `style`) est émis **verbatim** (le re-parseur ne décode
/// pas d'entités en rawtext — échapper double à chaque
/// resynchronisation). `title`/`textarea` sont RCDATA et
/// gardent l'échappement décodé au re-parse.
#[test]
fn rawtext_serialization_is_verbatim() {
    let html = "<div><script>var y=1&lt;2&amp;&amp;3</script><title>A&amp;B</title></div>";
    let d = Dom::from_html(html);
    let s = d.serialize_document();
    // La source du script reste brute — pas de `&amp;lt;`.
    assert!(
        s.contains("<script>var y=1&lt;2&amp;&amp;3</script>"),
        "script verbatim : {s}"
    );
    // Le titre reste échappé (RCDATA décodé au re-parse).
    assert!(s.contains("<title>A&amp;B</title>"), "title RCDATA : {s}");

    // Round-trip stable : re-serialiser ne dégrade rien.
    let s2 = Dom::from_html(&s).serialize_document();
    assert_eq!(s, s2, "round-trip stable");

    // Source contenant son propre `</script` : repli échappé
    // — la frontière tient au re-parse, rien ne fuit.
    let tricky = "<div><script>document.write('</scri'+'pt>')</script><p>x</p></div>";
    let d3 = Dom::from_html(tricky);
    let s3 = d3.serialize_document();
    let d4 = Dom::from_html(&s3);
    assert!(
        d4.serialize_document().contains("<p>x</p>"),
        "aucune fuite hors script : {s3}"
    );
}

/// Étape 55f — le chemin du sous-arbre sale : mutations
/// rapportées par `take_dirty_path` au format
/// `(tag, index)` depuis la racine ; un nœud détaché ne fixe
/// pas la cible (couvert à l'attachement par la marque du
/// parent) ; deux sous-arbres distincts escaladent à
/// l'ancêtre commun.
#[test]
fn dirty_path_tracks_mutated_subtree() {
    let mut dom = Dom::from_html(
        "<html><body><div id=\"a\"><p>x</p></div><div id=\"b\">y</div></body></html>",
    );
    dom.take_dirty();
    dom.take_dirty_path(); // purge construction (cf. install_dom)
    assert_eq!(dom.take_dirty_path(), None, "rien de sale après purge");

    let body = dom.parent(dom.by_id("b").unwrap()).unwrap();
    // Mutation d'un nœud détaché puis attachement : la cible
    // est le parent d'attachement, pas le fragment.
    let frag = dom.create_element("span");
    let text = dom.create_text("z");
    dom.append(frag, text);
    dom.append(body, frag);
    assert_eq!(
        dom.take_dirty_path(),
        Some(vec![("html".to_string(), 0), ("body".to_string(), 0)]),
        "l'attachement vise le parent, pas le fragment détaché"
    );

    // Deux sous-arbres distincts → ancêtre commun.
    let first = dom.by_id("a").unwrap();
    let second = dom.by_id("b").unwrap();
    dom.set_attr(first, "class", "m");
    dom.set_attr(second, "class", "n");
    assert_eq!(
        dom.take_dirty_path(),
        Some(vec![("html".to_string(), 0), ("body".to_string(), 0)]),
        "mutations dispersées → ancêtre commun (body)"
    );

    // Une seule cible → son propre chemin.
    let para = dom.element_children(first)[0];
    dom.set_text(dom.child_nodes(para)[0], "q");
    assert_eq!(
        dom.take_dirty_path(),
        Some(vec![
            ("html".to_string(), 0),
            ("body".to_string(), 0),
            ("div".to_string(), 0),
            ("p".to_string(), 0)
        ]),
        "mutation localisée → sous-arbre précis"
    );
}

// -- `<canvas>` 2D (JS-C28, couture R-11) ------------------------------

const PAGE_CANVAS: &str = r#"
<html><body><canvas id="c" width="100" height="80"></canvas></body></html>
"#;

#[test]
fn canvas_context_records_ops() {
    use explorer_jscore::CanvasOpRecord as R;
    let dom = Rc::new(RefCell::new(Dom::from_html(PAGE_CANVAS)));
    let mut e = Engine::new();
    e.install_dom(&dom);
    e.eval(
        r"const ctx = document.getElementById('c').getContext('2d');
          ctx.fillStyle = '#ff0000';
          ctx.fillRect(1, 2, 30, 40);
          ctx.globalAlpha = 0.5;
          ctx.beginPath();
          ctx.moveTo(0, 0); ctx.lineTo(10, 10); ctx.stroke();",
    )
    .expect("eval canvas");
    let ops = dom.borrow().canvas_ops();
    assert_eq!(ops.len(), 1);
    let (path, recs) = &ops[0];
    assert_eq!(path.last().map(|(t, _)| t.as_str()), Some("canvas"));
    assert_eq!(
        recs.as_slice(),
        &[
            R::FillColor {
                r: 255,
                g: 0,
                b: 0,
                a: 255
            },
            R::FillRect(1.0, 2.0, 30.0, 40.0),
            R::GlobalAlpha(0.5),
            R::BeginPath,
            R::MoveTo(0.0, 0.0),
            R::LineTo(10.0, 10.0),
            R::Stroke,
        ]
    );
}

#[test]
fn canvas_getctx_kinds_and_identity() {
    let dom = Rc::new(RefCell::new(Dom::from_html(PAGE_CANVAS)));
    let mut e = Engine::new();
    e.install_dom(&dom);
    // Spec : un seul contexte par mode ; mode inconnu → null ;
    // non-canvas → null.
    let webgl = e
        .eval("document.getElementById('c').getContext('webgl')")
        .expect("webgl");
    assert_eq!(e.describe(webgl), "null");
    let same = e
        .eval(
            "document.getElementById('c').getContext('2d') === \
             document.getElementById('c').getContext('2d')",
        )
        .expect("identity");
    assert_eq!(e.describe(same), "true");
    let not_canvas = e
        .eval("document.body.getContext('2d')")
        .expect("non-canvas");
    assert_eq!(e.describe(not_canvas), "null");
}

#[test]
fn canvas_op_marks_dirty_path() {
    // Un dessin est une mutation de rendu — le `<canvas>` est
    // le chemin sale rapporté à la synchro (relayout ciblé).
    let dom = Rc::new(RefCell::new(Dom::from_html(PAGE_CANVAS)));
    dom.borrow().take_dirty();
    dom.borrow().take_dirty_path();
    let mut e = Engine::new();
    e.install_dom(&dom);
    e.eval("document.getElementById('c').getContext('2d').fillRect(0,0,5,5)")
        .expect("fillRect");
    let path = dom
        .borrow()
        .take_dirty_path()
        .expect("une op canvas doit salir");
    assert_eq!(path.last().map(|(t, _)| t.as_str()), Some("canvas"));
}

#[test]
fn canvas_to_data_url_and_blob() {
    // JS-C151 — `toDataURL`/`toBlob` stubs honnêtes (pas de
    // raster miroir, JS-C28) : data-URL PNG vide, suffisant
    // aux feature-detects anti-bot (aliexpress sufei-punish).
    let dom = Rc::new(RefCell::new(Dom::from_html(PAGE_CANVAS)));
    let mut e = Engine::new();
    e.install_dom(&dom);
    let d = e
        .eval("document.getElementById('c').toDataURL()")
        .expect("toDataURL");
    assert_eq!(e.describe(d), "data:image/png;base64,");
    // `toBlob` invoque son callback avec la data-URL.
    e.eval("document.getElementById('c').toBlob(b => { window.__b = b; })")
        .expect("toBlob");
    let b = e.eval("window.__b").expect("blob cb");
    assert_eq!(e.describe(b), "data:image/png;base64,");
    // Non-canvas : repli générique (pas de méthode) — pas de
    // data-URL fantôme.
    assert!(e.eval("document.body.toDataURL()").is_err());
}

#[test]
fn canvas_width_reset_clears_ops() {
    // `canvas.width =` efface le bitmap enregistré (spec).
    let dom = Rc::new(RefCell::new(Dom::from_html(PAGE_CANVAS)));
    let mut e = Engine::new();
    e.install_dom(&dom);
    e.eval(
        r"const c = document.getElementById('c');
          const ctx = c.getContext('2d');
          ctx.fillRect(0, 0, 9, 9);
          c.width = 200;",
    )
    .expect("resize");
    assert!(
        dom.borrow().canvas_ops().is_empty() || dom.borrow().canvas_ops()[0].1.is_empty(),
        "width= efface la op-list"
    );
    // La valeur numérique est relue depuis l'attribut.
    let w = e.eval("c.width").expect("width prop");
    assert_eq!(e.describe(w), "200");
}

// -- HTMLMediaElement (étape 58, ADR-0023) ---------------------------

const PAGE_MEDIA: &str = r#"<html><body>
<video id="v" src="/v.mp4" poster="/p.png" controls></video>
<audio id="a"></audio></body></html>"#;

#[test]
fn media_props_are_honest_stubs() {
    assert_eq!(
        ev_in(PAGE_MEDIA, "document.getElementById('v').paused"),
        "true"
    );
    assert_eq!(
        ev_in(PAGE_MEDIA, "document.getElementById('v').ended"),
        "false"
    );
    assert_eq!(
        ev_in(PAGE_MEDIA, "document.getElementById('v').volume"),
        "1"
    );
    assert_eq!(
        ev_in(PAGE_MEDIA, "document.getElementById('v').currentTime"),
        "0"
    );
    assert_eq!(
        ev_in(PAGE_MEDIA, "document.getElementById('v').readyState"),
        "0"
    );
    // `src` présent mais indécodable → NETWORK_NO_SOURCE.
    assert_eq!(
        ev_in(PAGE_MEDIA, "document.getElementById('v').networkState"),
        "3"
    );
    // Sans `src` → NETWORK_EMPTY.
    assert_eq!(
        ev_in(PAGE_MEDIA, "document.getElementById('a').networkState"),
        "0"
    );
    assert_eq!(
        ev_in(PAGE_MEDIA, "document.getElementById('v').currentSrc"),
        ""
    );
    assert_eq!(
        ev_in(PAGE_MEDIA, "document.getElementById('v').error"),
        "null"
    );
    assert_eq!(
        ev_in(PAGE_MEDIA, "document.getElementById('v').buffered.length"),
        "0"
    );
    // Attributs réfléchis.
    assert_eq!(
        ev_in(PAGE_MEDIA, "document.getElementById('v').controls"),
        "true"
    );
    assert_eq!(
        ev_in(PAGE_MEDIA, "document.getElementById('v').poster"),
        "/p.png"
    );
    assert_eq!(
        ev_in(PAGE_MEDIA, "document.getElementById('v').src"),
        "/v.mp4"
    );
    // Aucun codec → canPlayType avoue "".
    assert_eq!(
        ev_in(
            PAGE_MEDIA,
            "document.getElementById('v').canPlayType('video/mp4')"
        ),
        ""
    );
    // `pause()`/`load()` = no-op silencieux.
    assert_eq!(
        ev_in(PAGE_MEDIA, "document.getElementById('v').pause(); 'ok'"),
        "ok"
    );
    // Écritures IDL sans attribut : acceptées-ignorées.
    assert_eq!(
        ev_in(
            PAGE_MEDIA,
            "document.getElementById('v').volume = 0.5; 'ok'"
        ),
        "ok"
    );
    // Non-média : les stubs ne fuient pas.
    assert_eq!(
        ev_in(PAGE_MEDIA, "document.body.paused === undefined"),
        "true"
    );
}

#[test]
fn media_play_rejects_like_no_source() {
    let mut e = engine_with(PAGE_MEDIA);
    e.eval(
        "var mediaResult='';\
         document.getElementById('v').play().then(\
             function(){ mediaResult='resolved'; },\
             function(r){ mediaResult=String(r); });",
    )
    .expect("play() call");
    e.run_microtasks().expect("microtasks");
    let got = e
        .eval("mediaResult")
        .map(|v| e.describe(v))
        .unwrap_or_default();
    assert!(
        got.contains("NotSupportedError"),
        "play() doit rejeter NotSupportedError, got {got}"
    );
    // `pause()`/`load()` = no-op silencieux.
    assert_eq!(
        ev_in(PAGE_MEDIA, "document.getElementById('v').pause(); 'ok'"),
        "ok"
    );
}

/// Étape 74 — `constructor.name` reflète le nom de classe
/// hôte (`HTMLVideoElement`…), pas « Object ».
#[test]
fn media_constructor_name_is_class_name() {
    let mut e = engine_with(PAGE_MEDIA);
    assert_eq!(
        e.eval("document.getElementById('v').constructor.name")
            .map(|v| e.describe(v))
            .unwrap_or_default(),
        "HTMLVideoElement"
    );
    assert_eq!(
        e.eval("document.getElementById('a').constructor.name")
            .map(|v| e.describe(v))
            .unwrap_or_default(),
        "HTMLAudioElement"
    );
}

// ------------------------------------------------------------------
// JS-C76 — collections DOM itérables (déclencheur `Error: b`
// google.com : les helpers `makeIterator` des bundles closure
// exigent `Symbol.iterator` **ou** `length` numérique + indexés).
// ------------------------------------------------------------------

const PAGE_COLL: &str = r#"<html><body>
<form id="f" name="login"><input id="u" name="user" value="v1">
<input name="kind" value="a"><input name="kind" value="b">
<select id="s"><option value="x">X</option><option value="y" selected>Y</option></select>
<button name="go">ok</button></form>
<div id="c" class="aa bb" data-x="1" lang="fr"></div>
<input id="fi" type="file"><input id="tf" type="text">
</body></html>"#;

/// `for-of`/`Symbol.iterator` sur les collections host —
/// `makeIterator` de Google s'appuie dessus (`Error: b` sinon).
#[test]
fn c76_collections_iterables() {
    // `classList` : itérable + indexée (DOMTokenList spec).
    assert_eq!(
        ev_in(
            PAGE_COLL,
            "var r=[];for(var t of document.getElementById('c').classList)r.push(t);r.join(',')"
        ),
        "aa,bb"
    );
    assert_eq!(
        ev_in(
            PAGE_COLL,
            "var c=document.getElementById('c').classList;c[0]+'|'+c[1]+'|'+c[9]+'|'+c.length+'|'+c.item(1)+'|'+c.item(9)+'|'+c.value"
        ),
        "aa|bb|undefined|2|bb|null|aa bb"
    );
    // `attributes` : NamedNodeMap itérable, indexée, nommée.
    assert_eq!(
        ev_in(
            PAGE_COLL,
            "var a=document.getElementById('c').attributes;var r=[];for(var x of a)r.push(x.name+'='+x.value);a.length+'|'+r.join(',')+'|'+a[1].name+'|'+a.lang.value+'|'+a.item(0).name"
        ),
        "4|id=c,class=aa bb,data-x=1,lang=fr|class|fr|id"
    );
    // `form.elements` : itérable, length, accès nommé + namedItem.
    assert_eq!(
        ev_in(
            PAGE_COLL,
            "var el=document.getElementById('f').elements;var tags=[];for(var x of el)tags.push(x.tagName);el.length+'|'+tags.join(',')+'|'+el.user.id+'|'+el.namedItem('u').value+'|'+el.namedItem('zz')"
        ),
        "5|INPUT,INPUT,INPUT,SELECT,BUTTON|u|v1|null"
    );
    // `children`/`childNodes`/`qSA`/`getElementsBy*` : `.item(i)`.
    assert_eq!(
        ev_in(
            PAGE_COLL,
            "var l=document.querySelectorAll('input');l.item(1).name+'|'+l.item(99)+'|'+document.getElementById('f').children.item(0).tagName"
        ),
        "kind|null|INPUT"
    );
    // NamedNodeMap mutateurs.
    assert_eq!(
        ev_in(
            PAGE_COLL,
            "var el=document.getElementById('c'),a=el.attributes;a.setNamedItem({name:'title',value:'hi'});a.getNamedItem('title').value+'|'+el.getAttribute('title')+'|'+a.removeNamedItem('title').value+'|'+el.getAttribute('title')"
        ),
        "hi|hi|hi|null"
    );
}

/// Le `z()` closure au sens littéral : `Symbol.iterator` ou
/// `length` numérique — chaque surface passait `undefined` avant.
#[test]
fn c76_makeiterator_gate() {
    let gate = "function z(a){var b=typeof Symbol!='undefined'&&Symbol.iterator&&a[Symbol.iterator];if(b)return b.call(a);if(typeof a.length=='number')return{arraylike:true};throw Error('b`'+String(a));}";
    for (label, src, want) in [
        (
            "attributes",
            "z(document.getElementById('c').attributes);'ok'",
            "ok",
        ),
        (
            "elements",
            "z(document.getElementById('f').elements);'ok'",
            "ok",
        ),
        (
            "classList",
            "z(document.getElementById('c').classList);'ok'",
            "ok",
        ),
        ("style", "z(document.getElementById('c').style);'ok'", "ok"),
        ("files", "z(document.getElementById('fi').files);'ok'", "ok"),
        (
            "options",
            "z(document.getElementById('s').options);'ok'",
            "ok",
        ),
    ] {
        assert_eq!(ev_in(PAGE_COLL, &format!("{gate}{src}")), want, "{label}");
    }
}

/// Détails de surface : `style.length` numérique, `form.username`
/// named getter, multi-match → collection, `files` null hors
/// file, `selectedOptions`/`selectedIndex`, `getElementsByName`,
/// `classList.forEach`/`toggle(force)`/`replace`.
#[test]
fn c76_surfaces_detail() {
    assert_eq!(
        ev_in(
            PAGE_COLL,
            "var el=document.getElementById('c');el.setAttribute('style','color:red; margin: 0');el.style.length+'|'+el.style[0]+'|'+el.style.item(1)"
        ),
        "2|color|margin"
    );
    assert_eq!(
        ev_in(
            PAGE_COLL,
            "var f=document.getElementById('f');f.user.tagName+'|'+f.kind.length+'|'+f.kind[0].value+'|'+(document.forms.login===f)+'|'+document.forms.namedItem('login').id+'|'+document.getElementsByName('kind').length"
        ),
        "INPUT|2|a|true|f|2"
    );
    assert_eq!(
        ev_in(
            PAGE_COLL,
            "var s=document.getElementById('s');s.options.length+'|'+s.options[1].value+'|'+s.selectedOptions.length+'|'+s.selectedIndex"
        ),
        "2|y|1|1"
    );
    assert_eq!(
        ev_in(
            PAGE_COLL,
            "var c=document.getElementById('c').classList,r=[];c.forEach(function(t,i){r.push(i+':'+t)});c.toggle('zz',true);c.replace('aa','cc');r.join(',')+'|'+c.contains('zz')+'|'+c.contains('cc')+'|'+c.contains('aa')"
        ),
        "0:aa,1:bb|true|true|false"
    );
    // `input.files` : FileList vide pour file, null sinon.
    assert_eq!(
        ev_in(
            PAGE_COLL,
            "document.getElementById('fi').files.length+'|'+document.getElementById('tf').files"
        ),
        "0|null"
    );
}

// -- JS-C67 : FormData + URLSearchParams bidirectionnel --------------

const PAGE_FD: &str = r#"
<html><body>
<form id="f" action="/go" method="post">
  <input name="u" value="lou">
  <input name="pw" type="password" value="s3cr">
  <input name="cb" type="checkbox" value="on" checked>
  <input name="off" type="checkbox" value="no">
  <input name="dis" value="x" disabled>
  <input type="submit" name="go" value="send">
  <button type="submit" name="btn" value="b1">Go</button>
  <textarea name="bio">hello world</textarea>
  <select name="sel"><option value="a">A</option><option value="b" selected>B</option></select>
  <input type="file" name="fi">
</form>
</body></html>
"#;

#[test]
fn c67_formdata_from_form() {
    // Champs « successful » : nommés, non-disabled, cases cochées —
    // submitter seul pour les boutons (spec FormData(form, submitter)).
    assert_eq!(
        ev_in(
            PAGE_FD,
            "var fd=new FormData(document.getElementById('f'));\
             fd.getAll('pw').length+'|'+fd.get('u')+'|'+fd.get('cb')+'|'\
             +fd.has('off')+'|'+fd.has('dis')+'|'+fd.has('go')+'|'\
             +fd.get('bio')+'|'+fd.get('sel')+'|'+fd.get('fi')+'|'\
             +(fd instanceof FormData)"
        ),
        "1|lou|on|false|false|false|hello world|b||true"
    );
    // `submitter` inclus quand nommé et passé en 2e argument.
    assert_eq!(
        ev_in(
            PAGE_FD,
            "var f=document.getElementById('f');\
             new FormData(f, f.elements.go).get('go')+'|'\
             +new FormData(f, f.elements.btn).get('btn')"
        ),
        "send|b1"
    );
    // Non-`<form>` → TypeError.
    assert_eq!(
        {
            let mut e = engine_with(PAGE_FD);
            match e.eval("new FormData(document.getElementById('f').elements.u)") {
                Ok(v) => e.describe(v),
                Err(_) => "TypeError".to_string(),
            }
        },
        "TypeError"
    );
}

#[test]
fn c67_formdata_methods() {
    // append/set/delete/get/getAll/has/forEach + itérable.
    assert_eq!(
        ev_in(
            PAGE_FD,
            "var fd=new FormData();fd.append('a','1');fd.append('a','2');fd.append('b','3');\
             fd.getAll('a').join(',')+'|'+fd.get('b')"
        ),
        "1,2|3"
    );
    assert_eq!(
        ev_in(
            PAGE_FD,
            "var fd=new FormData();fd.append('a','1');fd.append('a','2');fd.append('b','3');\
             fd.set('a','9');fd.delete('b');\
             fd.getAll('a').join(',')+'|'+fd.has('b')"
        ),
        "9|false"
    );
    assert_eq!(
        ev_in(
            PAGE_FD,
            "var fd=new FormData(),s='';fd.append('x','1');fd.append('y','2');\
             fd.forEach(function(v,k){s+=k+'='+v+';'});\
             for(var kv of fd)s+='['+kv[0]+kv[1]+']';\
             s+'|'+Object.keys(fd).length+'|'+fd.keys().next().value+'|'\
             +fd.values().next().value"
        ),
        "x=1;y=2;[x1][y2]|0|x|1"
    );
    // `entries()`/`keys()`/`values()` = itérateurs.
    assert_eq!(
        ev_in(
            PAGE_FD,
            "var fd=new FormData();fd.append('m','v');\
             var e=fd.entries();e.next().value.join(',')+'|'\
             +fd.keys().next().value+'|'+fd.values().next().value"
        ),
        "m,v|m|v"
    );
    // `new URLSearchParams(fd)` — bidirectionnel (iterable spec).
    assert_eq!(
        ev_in(
            PAGE_FD,
            "var fd=new FormData();fd.append('a','x y');fd.append('a','2');\
             var usp=new URLSearchParams(fd);\
             usp.getAll('a').join(',')+'|'+usp.toString()"
        ),
        "x y,2|a=x+y&a=2"
    );
    // `new URLSearchParams(string)` ↔ `String(fd)` n'existe pas —
    // mais l'encodage fetch est testé côté xhr.rs.
    assert_eq!(
        ev_in(PAGE_FD, "new URLSearchParams('?a=1&b=2').get('b')"),
        "2"
    );
}

// -- JS-C68 : customElements machinery --------------------------------

const PAGE_CE: &str = r#"
<html><body>
<x-foo id="a" observed="v0" other="z"></x-foo>
<div id="host"></div>
</body></html>
"#;

#[test]
fn c68_define_upgrades_existing() {
    // Élément présent au `define` → proto `Ctor.prototype`,
    // `instanceof`, ctor exécuté avec `this` = l'élément
    // (super() → HTMLElement rend l'élément via le slot),
    // `connectedCallback` tiré (attaché).
    assert_eq!(
        ev_in(
            PAGE_CE,
            "var log=[];\
             class XFoo extends HTMLElement {\
               constructor(){ super(); this.made=true; log.push('ctor:'+(this===document.getElementById('a'))); }\
               connectedCallback(){ log.push('connected'); }\
             }\
             customElements.define('x-foo', XFoo);\
             var a=document.getElementById('a');\
             log.join(',')+'|'+(a instanceof XFoo)+'|'+(a instanceof HTMLElement)+'|'+a.made+'|'+(a.tagName==='X-FOO')"
        ),
        "ctor:true,connected|true|true|true|true"
    );
}

#[test]
fn c68_create_insert_remove_lifecycle() {
    // `createElement` post-define → ctor (détaché : pas de
    // connected) ; `appendChild` → connected ; `remove` →
    // disconnected ; re-append → connected à nouveau.
    assert_eq!(
        ev_in(
            PAGE_CE,
            "var log=[];\
             class XBar extends HTMLElement {\
               constructor(){ super(); log.push('ctor'); }\
               connectedCallback(){ log.push('con'); }\
               disconnectedCallback(){ log.push('dis'); }\
             }\
             customElements.define('x-bar', XBar);\
             var el=document.createElement('x-bar');\
             log.push('created');\
             document.getElementById('host').appendChild(el);\
             el.remove();\
             document.body.appendChild(el);\
             log.join(',')"
        ),
        "ctor,created,con,dis,con"
    );
}

#[test]
fn c68_attribute_changed_observed_only() {
    // `attributeChangedCallback` : observés seulement
    // (name, old, new — `null` à l'ajout / au retrait).
    assert_eq!(
        ev_in(
            PAGE_CE,
            "var log=[];\
             class XObs extends HTMLElement {\
               static get observedAttributes(){ return ['observed']; }\
               attributeChangedCallback(n,o,v){ log.push(n+'='+o+'>'+v); }\
             }\
             customElements.define('x-obs', XObs);\
             var el=document.createElement('x-obs');\
             el.setAttribute('observed','1');\
             el.setAttribute('other','x');\
             el.setAttribute('observed','2');\
             el.removeAttribute('observed');\
             log.join(',')"
        ),
        "observed=null>1,observed=1>2,observed=2>null"
    );
    // Attributs observés présents à l'upgrade → rappel initial.
    assert_eq!(
        ev_in(
            PAGE_CE,
            "var log=[];\
             class XInit extends HTMLElement {\
               static get observedAttributes(){ return ['observed']; }\
               attributeChangedCallback(n,o,v){ log.push(n+':'+o+'>'+v); }\
             }\
             document.body.innerHTML='<x-init observed=\"v9\" other=\"q\"></x-init>';\
             customElements.define('x-init', XInit);\
             log.join(',')"
        ),
        "observed:null>v9"
    );
}

#[test]
fn c68_when_defined_and_get() {
    // `whenDefined` : pendante avant define → résolue
    // `undefined` au define ; définie → résolue immédiat.
    // `get`/`getName` interrogent le registre.
    assert_eq!(
        ev_in(
            PAGE_CE,
            "var out='';\
             var p1=customElements.whenDefined('x-later');\
             p1.then(function(v){ out+='p1:'+v+';'; });\
             class XLater extends HTMLElement {}\
             customElements.define('x-later', XLater);\
             customElements.whenDefined('x-later').then(function(v){ out+='p2:'+v+';'; });\
             out+'|'+(customElements.get('x-later')===XLater)+'|'+customElements.get('nope')+'|'+customElements.getName(XLater)"
        ),
        "|true|undefined|x-later"
    );
    // Les promesses résolues ont livré après le checkpoint.
    let mut e = engine_with(PAGE_CE);
    describe_ev(
        &mut e,
        "var out='';\
         customElements.whenDefined('x-w').then(function(v){ out+='A:'+v; });\
         class XW extends HTMLElement {} customElements.define('x-w', XW);",
    );
    let _ = e.run_microtasks();
    assert_eq!(describe_ev(&mut e, "out"), "A:undefined");
}

#[test]
fn c68_define_validation() {
    // Nom invalide / doublon / non-appelable / ctor déjà
    // utilisé → TypeError (approche SyntaxError/NotSupportedError).
    let try_def = |html: &str, src: &str| -> String {
        let mut e = engine_with(html);
        match e.eval(src) {
            Ok(_) => "no throw".to_string(),
            Err(err) => err.to_string(),
        }
    };
    assert!(try_def(
        PAGE_CE,
        "customElements.define('nodash', class extends HTMLElement {})"
    )
    .contains("invalid name"));
    assert!(try_def(
        PAGE_CE,
        "customElements.define('font-face', class extends HTMLElement {})"
    )
    .contains("invalid name"));
    assert!(try_def(PAGE_CE, "customElements.define('x-a', 42)").contains("not a constructor"));
    assert!(try_def(
        PAGE_CE,
        "class C extends HTMLElement {}\
         customElements.define('x-b', C);\
         customElements.define('x-b', C)"
    )
    .contains("already defined"));
    assert!(try_def(
        PAGE_CE,
        "class C2 extends HTMLElement {}\
         customElements.define('x-c', C2);\
         customElements.define('x-c2', C2)"
    )
    .contains("already registered"));
}

#[test]
fn c68_innerhtml_and_upgrade_method() {
    // innerHTML parse un CE connu → upgrade + connected via le
    // drain synchrone.
    assert_eq!(
        ev_in(
            PAGE_CE,
            "var log=[];\
             class XIn extends HTMLElement {\
               connectedCallback(){ log.push('con:'+this.id); }\
             }\
             customElements.define('x-in', XIn);\
             document.getElementById('host').innerHTML='<x-in id=\"k1\"></x-in>';\
             log.join(',')"
        ),
        "con:k1"
    );
    // `customElements.upgrade(root)` — sous-arbre détaché upgradé
    // à la demande (ctor tiré, pas de connected).
    assert_eq!(
        ev_in(
            PAGE_CE,
            "var log=[];\
             class XUp extends HTMLElement {\
               constructor(){ super(); log.push('ctor:'+this.id); }\
             }\
             var d=document.createElement('div');\
             d.innerHTML='<x-up id=\"u1\"></x-up>';\
             customElements.define('x-up', XUp);\
             customElements.upgrade(d);\
             log.join(',')"
        ),
        "ctor:u1"
    );
}

#[test]
fn c68_failed_ctor_marks_failed() {
    // Ctor dont `super()` n'atteint pas HTMLElement (classe
    // sans extends) → l'élément garde son proto mais l'état
    // « failed » désarme les callbacks (honnête spec).
    assert_eq!(
        ev_in(
            PAGE_CE,
            "var log=[];\
             class Plain { constructor(){ this.x=1; } connectedCallback(){ log.push('con'); } }\
             customElements.define('x-plain', Plain);\
             var el=document.createElement('x-plain');\
             document.getElementById('host').appendChild(el);\
             log.length+'|'+(el instanceof Plain)"
        ),
        "0|true"
    );
}

/// Eval via `describe` — multi-appels sur le même moteur.
fn describe_ev(e: &mut Engine, src: &str) -> String {
    match e.eval(src) {
        Ok(v) => e.describe(v),
        Err(err) => panic!("{src} -> {err}"),
    }
}

// -- JS-C69 : IntersectionObserver/ResizeObserver fonctionnels -------
//
// Même modèle de livraison que MutationObserver : notifications
// au checkpoint microtask — `ev2` lit leurs effets au second eval.

const PAGE_OBS: &str = r#"
<html><body>
<div id="a"></div>
<div id="b"><i>x</i></div>
</body></html>
"#;

#[test]
fn c69_intersection_initial_entry() {
    // `observe` enfile la notification initiale (spec — les
    // navigateurs rapportent toujours l'état courant à la
    // première passe) : entries[1], target, `isIntersecting`
    // (attaché), ratio 1, `observer` en second argument.
    assert_eq!(
        ev2(
            PAGE_OBS,
            "var log='';\
             var io=new IntersectionObserver(function(es,o){\
               var e=es[0];\
               log=es.length+'/'+(o===io)+'/'+(e.target===document.getElementById('a'))\
                 +'/'+e.isIntersecting+'/'+e.intersectionRatio\
                 +'/'+typeof e.time+'/'+typeof e.boundingClientRect.width\
                 +'/'+typeof e.intersectionRect.width+'/'+typeof e.rootBounds.width;\
             });\
             io.observe(document.getElementById('a'));",
            "log"
        ),
        "1/true/true/true/1/number/number/number/number"
    );
}

#[test]
fn c69_detached_not_intersecting() {
    // Élément détaché → `isIntersecting:false`, ratio 0 — un
    // nœud hors document ne peut intersecter (spec).
    assert_eq!(
        ev2(
            PAGE_OBS,
            "var log='';\
             var el=document.createElement('div');\
             new IntersectionObserver(function(es){\
               log=es[0].isIntersecting+'/'+es[0].intersectionRatio+'/'+es[0].intersectionRect.width;\
             }).observe(el);",
            "log"
        ),
        "false/0/0"
    );
}

#[test]
fn c69_unobserve_disconnect_takerecords() {
    // `unobserve` avant le checkpoint purge la notification ;
    // `disconnect` purge cibles + pendantes ; `takeRecords`
    // consomme la file immédiatement (le callback ne la voit
    // plus — spec).
    assert_eq!(
        ev2(
            PAGE_OBS,
            "var calls=0;\
             var io1=new IntersectionObserver(function(){calls++});\
             io1.observe(document.getElementById('a'));\
             io1.unobserve(document.getElementById('a'));\
             var io2=new IntersectionObserver(function(){calls++});\
             io2.observe(document.getElementById('b'));\
             io2.disconnect();\
             var io3=new IntersectionObserver(function(){calls++});\
             io3.observe(document.getElementById('a'));\
             var taken=io3.takeRecords().length;",
            "taken+'/'+calls"
        ),
        // `takeRecords` a vidé la pendante d'io3 → 0 appel ;
        // io1/io2 purgés → 0 appel non plus.
        "1/0"
    );
}

#[test]
fn c69_observe_dedup_and_rearm() {
    // Deux `observe` sur la même cible = une seule notification
    // (spec : ensemble de cibles) ; re-observe après
    // `disconnect` ré-arme proprement.
    assert_eq!(
        ev2(
            PAGE_OBS,
            "var n=0;\
             var io=new IntersectionObserver(function(es){n+=es.length});\
             io.observe(document.getElementById('a'));\
             io.observe(document.getElementById('a'));\
             io.observe(document.getElementById('b'));\
             io.disconnect();\
             io.observe(document.getElementById('b'));",
            "n+''"
        ),
        "1"
    );
}

#[test]
fn c69_resize_initial_entry() {
    // `ResizeObserver` — notification initiale avec
    // `contentRect`/`contentBoxSize`/`borderBoxSize`/
    // `devicePixelContentBoxSize` (spec).
    assert_eq!(
        ev2(
            PAGE_OBS,
            "var log='';\
             var ro=new ResizeObserver(function(es,o){\
               var e=es[0];\
               log=(o===ro)+'/'+(e.target===document.getElementById('b'))\
                 +'/'+typeof e.contentRect.width+'/'+e.contentBoxSize[0].inlineSize\
                 +'/'+e.borderBoxSize.length+'/'+e.devicePixelContentBoxSize.length;\
             });\
             ro.observe(document.getElementById('b'));",
            "log"
        ),
        "true/true/number/0/1/1"
    );
}

#[test]
fn c69_viewport_geometry_root() {
    // `documentElement`/`body` ≙ viewport (règle `clientWidth`)
    // — `rootBounds` sans `root` = viewport ; avec `root: el`
    // = rect honnête de l'élément (0 pour un non-racine).
    let mut e = engine_with(PAGE_OBS);
    e.set_viewport(800, 600);
    let _ = describe_ev(
        &mut e,
        "var log='';\
         new IntersectionObserver(function(es){\
           log=es[0].rootBounds.width+'/'+es[0].rootBounds.height+'/'+es[0].boundingClientRect.width+'/'+es[0].intersectionRect.height;\
         }).observe(document.body);",
    );
    assert_eq!(describe_ev(&mut e, "log"), "800/600/800/600");
    // `io.root` reflète l'option ; `thresholds`/`rootMargin`
    // figés et exposés (spec).
    assert_eq!(
        describe_ev(
            &mut e,
            "var a=document.getElementById('a');\
             var io=new IntersectionObserver(function(){},{root:a,threshold:[0.25,0.5],rootMargin:'10px 5%'});\
             var io2=new IntersectionObserver(function(){});\
             (io.root===a)+'/'+io.thresholds.join(',')+'/'+io.rootMargin\
             +'|'+(io2.root===null)+'/'+io2.thresholds.join(',')+'/'+io2.rootMargin"
        ),
        "true/0.25,0.5/10px 5%|true/0/0px"
    );
}

#[test]
fn c69_ctor_validation() {
    // Callback requis ; `threshold` hors [0,1] rejeté ;
    // `rootMargin` malformé rejeté ; `box` inconnu rejeté.
    assert_eq!(
        ev_in(
            PAGE_OBS,
            "var r='';\
             try{new IntersectionObserver(42)}catch(e){r+=e.name+'|'}\
             try{new IntersectionObserver(function(){},{threshold:2})}catch(e){r+=e.name+'|'}\
             try{new IntersectionObserver(function(){},{rootMargin:'10em'})}catch(e){r+=e.name+'|'}\
             try{new IntersectionObserver(function(){},{root:42})}catch(e){r+=e.name+'|'}\
             try{new ResizeObserver(function(){},{box:'weird'})}catch(e){r+=e.name}\
             var t=typeof new IntersectionObserver(function(){});\
             r+'|'+t+'|'+Object.prototype.toString.call(new ResizeObserver(function(){}))"
        ),
        "TypeError|TypeError|TypeError|TypeError|TypeError|object|[object ResizeObserver]"
    );
}

// -- JS-C117 : visibilité viewport réelle (vol github lazy-CE) -------

#[test]
fn c117_scroll_getters_read_geometry() {
    // `scrollY`/`pageYOffset`/`scrollTop`/`scrollingElement`
    // lisent la `GeometryMap` publiée (figés 0/undefined au
    // vol github → loaders lazy jamais déclenchés).
    let (mut e, _jar, _navs) = engine_full(PAGE_OBS);
    e.set_viewport(800, 600);
    let geom = explorer_jscore::GeometrySlot::default();
    geom.lock().unwrap().scroll_y = 1234.0;
    e.set_geometry_ctx(0, geom);
    assert_eq!(
        evf(
            &mut e,
            "[window.scrollY,window.pageYOffset,window.scrollX,\
              document.documentElement.scrollTop,\
              document.scrollingElement===document.documentElement,\
              document.scrollingElement.scrollTop].join(',')"
        ),
        "1234,1234,0,1234,true,1234"
    );
    // Méthodes IO visibles à `typeof` — les feature-detects
    // des loaders voient l'API fonctionnelle.
    assert_eq!(
        evf(
            &mut e,
            "var io=new IntersectionObserver(function(){});\
             [typeof io.observe,typeof io.unobserve,\
              typeof io.disconnect,typeof io.takeRecords].join(',')"
        ),
        "function,function,function,function"
    );
}

#[test]
fn c117_io_reevaluates_on_scroll() {
    // Élément sous le fold → initiale `false` ; scroll publié
    // → ré-évaluation au checkpoint → livraison `true`
    // (vol github : IO silencieux après l'init → lazy CE
    // jamais importée).
    let (mut e, _jar, _navs) = engine_full(PAGE_OBS);
    e.set_viewport(800, 600);
    let geom = explorer_jscore::GeometrySlot::default();
    {
        let mut g = geom.lock().unwrap();
        g.scroll_y = 0.0;
        g.doc_h = 4000.0;
        // #a → html>body>div[0] — sous le fold (doc y=3000).
        g.rects.insert(
            vec![
                ("html".to_string(), 0),
                ("body".to_string(), 0),
                ("div".to_string(), 0),
            ],
            explorer_jscore::ElementGeom {
                rect: explorer_jscore::GeoRect {
                    x: 0.0,
                    y: 3000.0,
                    w: 100.0,
                    h: 100.0,
                },
                fixed: false,
                positioned: false,
            },
        );
    }
    e.set_geometry_ctx(0, geom.clone());
    evf(
        &mut e,
        "window.__log=[];\
         var io=new IntersectionObserver(function(es){\
           es.forEach(function(x){window.__log.push(x.isIntersecting)});\
         });\
         io.observe(document.getElementById('a'));\
         'armed'",
    );
    e.run_microtasks().expect("pump initiale");
    assert_eq!(evf(&mut e, "window.__log.join('|')"), "false");
    // Scroll session publié — l'entrée entre au viewport.
    geom.lock().unwrap().scroll_y = 2500.0;
    e.run_microtasks().expect("pump ré-évaluation");
    assert_eq!(evf(&mut e, "window.__log.join('|')"), "false|true");
    // `scrollY`/`scrollTop` suivent le scroll publié.
    assert_eq!(
        evf(
            &mut e,
            "[window.scrollY,document.documentElement.scrollTop].join(',')"
        ),
        "2500,2500"
    );
}

// ------------------------------------------------------------------
// JS-C74 — Notification / Selection / reportError / scrollIntoView
// ------------------------------------------------------------------

fn dev(e: &mut Engine, src: &str) -> String {
    match e.eval(src) {
        Ok(v) => e.describe(v),
        Err(err) => panic!("{src} -> {err}"),
    }
}

#[test]
fn c74_notification_stub() {
    let mut e = engine_with(PAGE);
    // Stub honnête : permission 'default', requestPermission
    // → 'denied', instance inerte avec props spec.
    assert_eq!(
        dev(
            &mut e,
            "var s = [];             s.push(typeof Notification);             s.push(Notification.permission);             s.push(Notification.maxActions);             var n = new Notification('Hi', {body: 'B', tag: 't1', silent: true});             s.push(n instanceof Notification);             s.push(n.title + '|' + n.body + '|' + n.tag + '|' + n.silent);             s.push(n.onclick === null);             n.close();             s.join(',')"
        ),
        "function,default,0,true,Hi|B|t1|true,true"
    );
    // requestPermission → promesse 'denied' + callback legacy.
    dev(
        &mut e,
        "var p = '';         Notification.requestPermission(function (r) { p = 'cb:' + r; }).then(function (r) { p += '|th:' + r; });",
    );
    assert_eq!(dev(&mut e, "p"), "cb:denied|th:denied");
}

#[test]
fn c74_get_selection() {
    let mut e = engine_with(PAGE);
    // Selection réelle : singleton (`s1 === s2`),
    // selectAllChildren, toString = textContent, ranges.
    assert_eq!(
        dev(
            &mut e,
            "var s1 = window.getSelection();             var s2 = document.getSelection();             var out = [];             out.push(s1 === s2);             out.push(s1.rangeCount + '/' + s1.isCollapsed + '/' + s1.type);             s1.selectAllChildren(document.getElementById('list'));             out.push(s1.rangeCount + '/' + s1.isCollapsed + '/' + s1.type);             out.push(s1.toString());             var r = s1.getRangeAt(0);             out.push(r.startContainer.id + '/' + r.startOffset + '/' + r.endOffset + '/' + r.collapsed);             out.push(s1.anchorNode === r.startContainer);             out.join(',')"
        ),
        "true,0/true/None,1/false/Range,ab,list/0/2/false,true"
    );
    // collapse → Caret ; removeAllRanges → None.
    assert_eq!(
        dev(
            &mut e,
            "s1.collapse(document.getElementById('main'), 0);             s1.type + '/' + s1.isCollapsed"
        ),
        "Caret/true"
    );
    assert_eq!(
        dev(
            &mut e,
            "s1.removeAllRanges(); s1.rangeCount + '/' + s1.type + '/' + s1.toString()"
        ),
        "0/None/"
    );
    // containsNode + getRangeAt hors borne → TypeError.
    assert_eq!(
        dev(
            &mut e,
            "var s3 = window.getSelection();             s3.selectAllChildren(document.getElementById('main'));             var hits = [];             hits.push(s3.containsNode(document.querySelector('#main p'), false));             hits.push(s3.containsNode(document.getElementById('inp'), false));             try { s3.getRangeAt(5); } catch (err) { hits.push('rng:' + (err instanceof TypeError)); }             hits.join(',')"
        ),
        "true,false,rng:true"
    );
}

#[test]
fn c74_report_error() {
    let mut e = engine_with(PAGE);
    // reportError → window.onerror (signature legacy) ; un
    // retour truthy = géré (pas de rapport console).
    dev(
        &mut e,
        "var log = [];         window.onerror = function (msg, src, line, col, err) {           log.push('on:' + msg + '|' + (err && err.message));           return true;         };         reportError(new Error('boom'));         window.onerror = null;",
    );
    assert_eq!(dev(&mut e, "log.join(',')"), "on:Error: boom|boom");
    // Non-annulé par onerror → listener error reçoit
    // l'ErrorEvent ; preventDefault = géré.
    dev(
        &mut e,
        "var log2 = [];         window.addEventListener('error', function (ev) {           log2.push('l:' + ev.type + '|' + ev.message);           ev.preventDefault();         });         reportError('plain string');",
    );
    assert_eq!(dev(&mut e, "log2.join(',')"), "l:error|plain string");
}

#[test]
fn c74_scroll_into_view() {
    let mut e = engine_with(PAGE);
    // Méthode no-op honnête — lisible (feature-detect sur le
    // prototype), appelable sans exception, TypeError sur
    // receveur non-Element.
    assert_eq!(
        dev(
            &mut e,
            "var out = [];             out.push(typeof Element.prototype.scrollIntoView);             var el = document.getElementById('main');             out.push(el.scrollIntoView() === undefined);             out.push(el.scrollIntoViewIfNeeded() === undefined);             el.scrollIntoView({behavior: 'smooth', block: 'center'});             try { Element.prototype.scrollIntoView.call({}); } catch (err) { out.push('bad:' + (err instanceof TypeError)); }             out.join(',')"
        ),
        "function,true,true,bad:true"
    );
}

/// `location.hash=`/`search=`/`pathname=` naviguent vers
/// l'URL recomposée (CAP-160 — spec : chaque setter de
/// composant est une navigation).
#[test]
fn location_part_setters_navigate() {
    let (mut e, _jar, navs) = engine_full(PAGE);
    evf(&mut e, "location.hash='x'");
    evf(&mut e, "location.search='?y=2'");
    evf(&mut e, "location.pathname='/c'");
    let got = navs.borrow();
    assert_eq!(
        got.as_slice(),
        [
            "http://ex.org:8080/a/b?q=1#x",
            "http://ex.org:8080/a/b?y=2#frag",
            "http://ex.org:8080/c?q=1#frag"
        ]
    );
}

/// `matchMedia` évalue réellement contre l'env posé par
/// `set_media_env` (CAP-161) : viewport, dark, hover,
/// `not`, jointures `and`, `or` dans les parens, inconnu →
/// false (dégradation spec).
#[test]
fn matchmedia_evaluates_against_env() {
    let mut e = engine_with("");
    e.set_viewport(800, 600);
    e.set_media_env(explorer_jscore::MediaEnv {
        dark: true,
        reduced_motion: false,
        hover: true,
        pointer_fine: true,
    });
    let cases = [
        ("(prefers-color-scheme: dark)", true),
        ("(prefers-color-scheme: light)", false),
        ("(min-width: 700px)", true),
        ("(max-width: 700px)", false),
        ("(min-width: 50em)", true),
        ("(orientation: landscape)", true),
        ("(hover: hover)", true),
        ("(pointer: fine)", true),
        ("(pointer: coarse)", false),
        ("not (prefers-color-scheme: light)", true),
        (
            "screen and (min-width: 600px) and (max-width: 1000px)",
            true,
        ),
        ("print and (min-width: 600px)", false),
        ("(min-width: 1000px) or (orientation: landscape)", true),
        ("(unknown-feature: x)", false),
        ("(scripting: enabled)", true),
        ("(prefers-reduced-motion: reduce)", false),
        ("", true),
    ];
    for (q, want) in cases {
        let got = evf(&mut e, &format!("matchMedia('{q}').matches"));
        assert_eq!(got, want.to_string(), "matchMedia('{q}')");
    }
}

/// `el.focus()`/`blur()` + `document.activeElement` +
/// `autofocus` (CAP-162) : état de focus réel du miroir —
/// déplacement blur/focusout→focus/focusin, `focus` ne
/// bulle pas (`focusin` oui), non-focalisable = no-op,
/// `activeElement` retombe sur `body`.
#[test]
fn focus_state_and_events() {
    let mut e = engine_with(
        "<form><input id=a><input id=b disabled>         <div id=d tabindex=0></div></form><div id=log></div>",
    );
    let got = evf(
        &mut e,
        "var L=[];         var a=document.getElementById('a'),b=document.getElementById('b'),         d=document.getElementById('d');         var f=document.querySelector('form');         f.addEventListener('focus',function(){L.push('P:focus')});         f.addEventListener('focusin',function(ev){L.push('P:focusin:'+ev.target.id)});         a.addEventListener('blur',function(){L.push('a:blur')});         a.addEventListener('focusout',function(){L.push('a:focusout')});         b.addEventListener('focus',function(){L.push('b:focus')});         a.focus();L.push('ae:'+document.activeElement.id);         b.focus();L.push('ae2:'+document.activeElement.id);         d.focus();L.push('ae3:'+document.activeElement.id);         d.blur();L.push('ae4:'+(document.activeElement?document.activeElement.tagName:'null'));         L.join(',')",
    );
    assert_eq!(
        got,
        "P:focusin:a,ae:a,ae2:a,a:blur,a:focusout,P:focusin:d,ae3:d,ae4:null"
    );
}

/// `autofocus` (CAP-162) : le premier élément focalisable
/// `[autofocus]` tient `activeElement` dès l'install —
/// le `disabled` avant lui est sauté.
#[test]
fn autofocus_sets_active_element() {
    let mut e = engine_with("<input id=nope disabled autofocus><input id=yes autofocus>");
    assert_eq!(evf(&mut e, "document.activeElement.id"), "yes");
}

/// CAP-164 — surface Node de complétude : `getRootNode`,
/// `isConnected`, `hasChildNodes`, `isSameNode`,
/// `compareDocumentPosition`, `isEqualNode`, `normalize`.
#[test]
fn node_completeness_surface() {
    let mut e = engine_with("<div id=a><p id=p>x<b id=b>y</b></p></div><div id=z></div>");
    // getRootNode : attaché → le document (identité `===`).
    assert_eq!(
        evf(
            &mut e,
            "document.getElementById('b').getRootNode() === document"
        ),
        "true"
    );
    // Détaché → le sommet du sous-arbre (pas document).
    assert_eq!(
        evf(
            &mut e,
            "var f=document.createElement('div');f.appendChild(document.createTextNode('t'));\
             var r=f.getRootNode(); (r===f)+'|'+(r===document)"
        ),
        "true|false"
    );
    // isConnected / hasChildNodes / isSameNode.
    assert_eq!(
        evf(
            &mut e,
            "var a=document.getElementById('a'),b=document.getElementById('b');\
             a.isConnected+'|'+f.isConnected+'|'+a.hasChildNodes()+'|'+\
             document.getElementById('z').hasChildNodes()+'|'+a.isSameNode(a)+'|'+a.isSameNode(b)"
        ),
        "true|false|true|false|true|false"
    );
}

/// `compareDocumentPosition` — bitmask spec : l'autre vu de
/// `this` (contains → 8|4, contained → 16|4, ordre doc → 2/4,
/// détaché → 1|32+ordre).
#[test]
fn compare_document_position_bits() {
    let mut e = engine_with("<div id=a><p id=p></p></div><div id=z></div>");
    assert_eq!(
        evf(
            &mut e,
            "var a=document.getElementById('a'),p=document.getElementById('p'),\
             z=document.getElementById('z');\
             a.compareDocumentPosition(p)+'/'+p.compareDocumentPosition(a)+'/'+\
             a.compareDocumentPosition(z)+'/'+z.compareDocumentPosition(a)+'/'+\
             a.compareDocumentPosition(a)"
        ),
        // p est contenu par a et suit a : 16|4=20 ; a contient
        // p et précède : 8|2=10 ; z suit a : 4 ; a précède z : 2 ;
        // même nœud : 0.
        "20/10/4/2/0"
    );
    // Nœuds d'arbres disjoints : DISCONNECTED|IMPL + ordre.
    let d = evf(
        &mut e,
        "var f=document.createElement('span');\
         a.compareDocumentPosition(f)&(1|32)",
    );
    assert_eq!(d, "33");
}

/// `isEqualNode` / `normalize` — égalité structurelle et
/// fusion des textes adjacents (spec Node).
#[test]
fn equal_node_and_normalize() {
    let mut e = engine_with("<div id=a></div>");
    assert_eq!(
        evf(
            &mut e,
            "var a=document.getElementById('a');\
             var x=document.createElement('div');x.setAttribute('c','1');x.setAttribute('d','2');\
             var y=document.createElement('div');y.setAttribute('d','2');y.setAttribute('c','1');\
             x.isEqualNode(y)+'|'+x.isEqualNode(a)+'|'+x.isEqualNode(null)"
        ),
        "true|false|false"
    );
    // normalize : "ab"+"cd" adjacents + texte vide → 1 enfant.
    assert_eq!(
        evf(
            &mut e,
            "a.appendChild(document.createTextNode('ab'));\
             a.appendChild(document.createTextNode('cd'));\
             a.appendChild(document.createTextNode(''));\
             a.appendChild(document.createElement('i'));\
             a.appendChild(document.createTextNode('x'));\
             a.normalize();a.childNodes.length+'|'+a.childNodes[0].textContent+'|'+a.childNodes[2].textContent"
        ),
        "3|abcd|x"
    );
}

/// `history.scrollRestoration` (CAP-166) — enum reflété
/// `"auto"`/`"manual"` document-local ; les routeurs SPA le
/// posent `'manual'` avant de restaurer le scroll eux-mêmes.
/// Valeur invalide → `TypeError` (enum `WebIDL`).
#[test]
fn history_scroll_restoration() {
    let mut e = engine_with("");
    assert_eq!(evf(&mut e, "history.scrollRestoration"), "auto");
    evf(&mut e, "history.scrollRestoration = 'manual';");
    assert_eq!(evf(&mut e, "history.scrollRestoration"), "manual");
    evf(&mut e, "history.scrollRestoration = 'auto';");
    assert_eq!(evf(&mut e, "history.scrollRestoration"), "auto");
    // Valeur hors enum → TypeError, état inchangé.
    assert!(e.eval("history.scrollRestoration = 'bogus'").is_err());
    assert_eq!(evf(&mut e, "history.scrollRestoration"), "auto");
}

/// `document.currentScript` (CAP-168) — l'élément `<script>`
/// en cours d'évaluation, `null` hors éval ; identifie le
/// bundle en exécution (loaders UMD, polyfills).
#[test]
fn document_current_script() {
    let mut e = engine_with(
        "<html><body>\
         <script src=a.js></script>\
         <script src=b.js></script>\
         <script>var z = 1;</script>\
         </body></html>",
    );
    // Hors éval : `null`.
    assert_eq!(evf(&mut e, "document.currentScript"), "null");
    // Pendant l'éval de l'élément ordinal 1 : `b.js`.
    let v = e
        .eval_script_el(
            "var seen = document.currentScript;\
             (seen ? seen.tagName + ':' + seen.getAttribute('src') : 'null')",
            None,
            1,
        )
        .unwrap();
    assert_eq!(e.describe(v), "SCRIPT:b.js");
    // Inline (ordinal 2) : pas de `src`, c'est le bon élément
    // quand même (`textContent` relu).
    let v = e
        .eval_script_el("document.currentScript.textContent.trim()", None, 2)
        .unwrap();
    assert_eq!(e.describe(v), "var z = 1;");
    // Après éval : `null` restauré (spec).
    assert_eq!(evf(&mut e, "document.currentScript"), "null");
    // Éval ordinaire (pas d'élément porteur) : `null`.
    assert_eq!(evf(&mut e, "document.currentScript === null"), "true");
}

/// CAP-184 — `document.currentScript` sous auto-suppression :
/// un `<script>` qui se `remove()` ne décale pas le lookup
/// des suivants (pattern stream `$_TSR` de x.com : chaque
/// chunk s'auto-supprime — le lookup positionnel renvoyait
/// le script d'après, puis `null` hors-bornes).
#[test]
fn document_current_script_survives_self_remove() {
    let mut e = engine_with(
        "<html><body>\
         <script src=a.js></script>\
         <script src=b.js></script>\
         <script src=c.js></script>\
         </body></html>",
    );
    // Le script 0 se supprime du miroir.
    e.eval_script_el("document.currentScript.remove()", None, 0)
        .unwrap();
    // Le script 1 se résout toujours sur lui-même.
    let v = e
        .eval_script_el(
            "var s=document.currentScript; s ? s.getAttribute('src') : 'null'",
            None,
            1,
        )
        .unwrap();
    assert_eq!(e.describe(v), "b.js");
    // L'ordinal 0 identifie encore l'élément — détaché mais
    // vivant dans l'arène (spec : `currentScript` est
    // l'élément, attaché ou non).
    let v = e
        .eval_script_el(
            "var s=document.currentScript; s ? s.getAttribute('src') : 'null'",
            None,
            0,
        )
        .unwrap();
    assert_eq!(e.describe(v), "a.js");
}

// ===== CAP-185 — `load`/`error` des `<link>` statiques :
// ordinal figé au parse (`link_seqs`) ; la capture sur
// `document` voit la cible (pattern bootstrap `x-web` :
// `addEventListener('load', f, true)` attend la feuille
// pour injecter le script d'entrée).

#[test]
fn cap185_static_link_load_error() {
    let mut e = engine_with(
        "<html><head>\
         <link rel=stylesheet href=/a.css>\
         <link rel=stylesheet href=/b.css>\
         <link rel=icon href=/i.png>\
         </head><body><p id=p>?</p></body></html>",
    );
    e.eval(
        "var out=[];\
        document.addEventListener('load',function(ev){\
          out.push('load:'+ev.target.getAttribute('href'))},true);\
        document.addEventListener('error',function(ev){\
          out.push('err:'+ev.target.getAttribute('href'))},true)",
    )
    .expect("listeners");
    // ord = ordinal parmi TOUS les `<link>` (icon compte).
    e.deliver_static_link(0, true);
    e.deliver_static_link(1, false);
    // Ordinal absent → no-op tolérant.
    e.deliver_static_link(42, true);
    assert_eq!(evf(&mut e, "out.join('|')"), "load:/a.css|err:/b.css");
}

/// CAP-170 — `document.readyState` : `loading` par défaut
/// (sans embedder), transitions pilotées par
/// `Engine::set_ready_state`, valeurs hors enum ignorées.
#[test]
fn document_ready_state_transitions() {
    let mut e = engine_with("<p>x</p>");
    let read = |e: &mut Engine| match e.eval("document.readyState") {
        Ok(v) => e.describe(v),
        Err(err) => panic!("readyState -> {err}"),
    };
    assert_eq!(read(&mut e), "loading");
    e.set_ready_state(0, "interactive");
    assert_eq!(read(&mut e), "interactive");
    e.set_ready_state(0, "complete");
    assert_eq!(read(&mut e), "complete");
    e.set_ready_state(0, "bogus");
    assert_eq!(read(&mut e), "complete");
}

/// `document.domain` (issue #24) — getter = hostname de
/// l'URL de page ; setter à suffixe de label accepté
/// (raccourcit le domaine effectif), non-suffixe ignoré
/// (tolérant — spec : `SecurityError`).
#[test]
fn document_domain_getter_setter() {
    let (mut e, _jar, _navs) = engine_full(PAGE);
    assert_eq!(evf(&mut e, "document.domain"), "ex.org");
    // Réécriture au même host : acceptée.
    evf(&mut e, "document.domain = 'ex.org'");
    assert_eq!(evf(&mut e, "document.domain"), "ex.org");
    // Non-suffixe : ignoré, la valeur courante survit.
    evf(&mut e, "document.domain = 'other.com'");
    assert_eq!(evf(&mut e, "document.domain"), "ex.org");
}

/// `el.attachShadow` stubs (issue #25) — le polyfill
/// Invoker Commands lit la **référence** sur
/// `HTMLElement.prototype` puis l'appelle via `.call`
/// (mesuré github-elements) : la méthode doit être une
/// valeur callable, pas seulement un dispatch `invoke`.
/// `mode:"open"` → `el.shadowRoot` non-null ; `closed` →
/// `null` publiquement, visible via `attachInternals`.
#[test]
fn attach_shadow_stub_polyfill_pattern() {
    let (mut e, _jar, _navs) = engine_full(PAGE);
    assert_eq!(
        evf(
            &mut e,
            "var out = [];             var el = document.getElementById('main');             var el2 = document.getElementById('list');             var n = HTMLElement.prototype.attachShadow;             out.push(typeof n);             out.push(typeof Element.prototype.attachShadow);             var r = n.call(el, {mode: 'open'});             out.push(el.shadowRoot === r);             out.push(el.shadowRoot.nodeType);             var r2 = n.call(el2, {mode: 'closed'});             out.push(el2.shadowRoot === null);             out.push(el2.attachInternals().shadowRoot === r2);             out.push(n.call(el, {mode: 'closed'}) === r);             out.join(',')"
        ),
        "function,function,true,11,true,true,true"
    );
}

// ===== CAP-174 — feature-detect : méthodes host lisibles
// comme propriétés fonctionnelles (typeof el.m = 'function',
// shims `o = el.matches || …`). Mesuré par sonde terrain :
// les feature-detects tombaient en faux sur toute la surface
// invoke-only alors que les appels marchaient.

#[test]
fn host_methods_read_as_functions() {
    let mut e = engine_with("<!doctype html><html><body><div id=x></div></body></html>");
    // `typeof` sur les méthodes dispatchées invoke-only.
    let s = e
        .eval(
            "var el=document.getElementById('x');
             [ typeof el.appendChild,
               typeof el.querySelector,
               typeof el.matches,
               typeof el.getBoundingClientRect,
               typeof el.setAttribute,
               typeof el.attachShadow,
               typeof el.cloneNode,
               typeof el.addEventListener,
               typeof document.getElementById,
               typeof document.querySelector,
               typeof document.createElement,
               typeof document.hasFocus ].join(',')",
        )
        .map(|v| e.describe(v))
        .unwrap_or_default();
    assert_eq!(
        s,
        "function,function,function,function,function,function,function,function,function,function,function,function",
        "toutes les méthodes lisibles en fonction : {s}"
    );
    // Shim feature-detect réel : `o = el.matches || el.msMatchesSelector`.
    let v2 = e
        .eval(
            "var el=document.getElementById('x');
             var o=el.matches || el.msMatchesSelector; typeof o",
        )
        .map(|v| e.describe(v))
        .unwrap_or_default();
    assert_eq!(v2, "function");
    // La fonction lue appelle bien la méthode (re-dispatch invoke).
    let v3 = e
        .eval(
            "var el=document.getElementById('x');
             var m=el.matches; m.call(el,'div') ? 'ok' : 'bad'",
        )
        .map(|v| e.describe(v))
        .unwrap_or_default();
    assert_eq!(v3, "ok");
    // Une own-prop scriptée du même nom prime (shadowing préservé).
    let v4 = e
        .eval(
            "var el=document.getElementById('x');
             el.foo=function(){return 7;}; el.foo()",
        )
        .map(|v| e.describe(v))
        .unwrap_or_default();
    assert_eq!(v4, "7");
}

// ===== CAP-176 — `<dialog>` : showModal/show/close bornés
// (trou mesuré par sonde empirique `_census_calls` —
// `dialog.showModal` TypeError sur le terrain).

#[test]
fn dialog_show_modal_close_bounded() {
    let mut e = engine_with(
        "<!doctype html><html><body><dialog id=d></dialog><div id=v></div></body></html>",
    );
    e.eval("var d=document.getElementById('d'), v=document.getElementById('v');")
        .expect("setup");
    // `open` reflété bool, fermé par défaut.
    assert_eq!(evf(&mut e, "d.open"), "false");
    // `showModal()` → `open`, flag modal interne.
    assert_eq!(evf(&mut e, "d.showModal(); d.open"), "true");
    // Déjà ouvert → `InvalidStateError` (spec).
    let r = e.eval("d.showModal()");
    assert!(r.is_err(), "showModal sur ouvert doit jeter");
    let msg = format!("{:?}", r.unwrap_err());
    assert!(msg.contains("InvalidStateError"), "attendu : {msg}");
    // `close()` → `open` retiré + événement `close` tiré.
    assert_eq!(
        evf(
            &mut e,
            "var c=0; d.addEventListener('close', function(){c++;}); \
             d.close('ret42'); [d.open, d.returnValue, c].join(',')"
        ),
        "false,ret42,1"
    );
    // `close()` sur fermé → no-op silencieux (pas d'événement).
    assert_eq!(evf(&mut e, "d.close(); c"), "1");
    // `show()` → `open` non-modal.
    assert_eq!(evf(&mut e, "d.show(); d.open"), "true");
    // `returnValue` settable en propriété (slot spec).
    assert_eq!(evf(&mut e, "d.returnValue='x'; d.returnValue"), "x");
    // Feature-detect : `typeof dlg.showModal` = 'function',
    // `typeof div.showModal` = 'undefined' (gate per-tag).
    assert_eq!(evf(&mut e, "typeof d.showModal"), "function");
    assert_eq!(evf(&mut e, "typeof v.showModal"), "undefined");
    // `showModal` sur non-dialog → TypeError (pas de méthode).
    let r = e.eval("v.showModal()");
    assert!(r.is_err(), "showModal sur <div> doit jeter");
}

// ===== CAP-177 — primitives mesurées absentes (sonde
// `_census_calls` : TypeError terrain) :
// insertAdjacentElement, replaceChildren, form.reset,
// select/setSelectionRange/setRangeText + selection*,
// img.decode, document.importNode/adoptNode.

#[test]
fn cap177_dom_primitives_batch() {
    let mut e = engine_with(
        "<!doctype html><html><body>\
         <div id=a><span id=s>x</span></div>\
         <form id=f><input id=i value=hello><textarea id=t>world</textarea></form>\
         <img id=g src=x.png></body></html>",
    );
    e.eval("var a=document.getElementById('a'), s=document.getElementById('s'), f=document.getElementById('f'), i=document.getElementById('i'), t=document.getElementById('t'), g=document.getElementById('g'), v=document.createElement('div');")
        .expect("setup");
    // `insertAdjacentElement` — retourne l'élément inséré.
    assert_eq!(
        evf(
            &mut e,
            "v.id='z'; var r=a.insertAdjacentElement('beforeend',v); r===v&&a.lastElementChild===v"
        ),
        "true"
    );
    // `replaceChildren` — vide puis insère.
    assert_eq!(
        evf(&mut e,
            "a.replaceChildren(s, v); [a.childElementCount, a.firstElementChild===s, a.lastElementChild===v].join(',')"),
        "2,true,true"
    );
    // `form.reset()` — événement `reset` annulable tiré.
    assert_eq!(
        evf(
            &mut e,
            "var rc=0; f.addEventListener('reset', function(){rc++;}); f.reset(); rc"
        ),
        "1"
    );
    // `input.select()` → selectionStart=0, end=len.
    assert_eq!(
        evf(
            &mut e,
            "i.select(); [i.selectionStart, i.selectionEnd, i.selectionDirection].join(',')"
        ),
        "0,5,none"
    );
    // `setSelectionRange(1,3)` → bornes posées.
    assert_eq!(
        evf(&mut e, "i.setSelectionRange(1,3,'forward'); [i.selectionStart, i.selectionEnd, i.selectionDirection].join(',')"),
        "1,3,forward"
    );
    // `selectionStart` settable en prop.
    assert_eq!(evf(&mut e, "i.selectionStart=2; i.selectionStart"), "2");
    // `textarea.setRangeText` — édite `value` + caret.
    assert_eq!(
        evf(
            &mut e,
            "t.setRangeText('XY', 0, 2); [t.value, t.selectionStart].join('|')"
        ),
        "XYrld|2"
    );
    // `img.decode()` → promesse résolue.
    assert_eq!(
        evf(
            &mut e,
            "var ok=0; g.decode().then(function(){ok=1;}); typeof g.decode"
        ),
        "function"
    );
    // `document.importNode` — clone profond.
    assert_eq!(
        evf(
            &mut e,
            "var im=document.importNode(a, true); [im.childElementCount, im!==a].join(',')"
        ),
        "2,true"
    );
    // `document.adoptNode` — détache et retourne le nœud.
    assert_eq!(
        evf(
            &mut e,
            "var ad=document.adoptNode(v); [ad===v, v.parentNode===null].join(',')"
        ),
        "true,true"
    );
    // Feature-detect per-tag (has_method) : les méthodes
    // gatées ne se lisent que sur leur élément.
    assert_eq!(
        evf(&mut e,
            "[typeof i.select, typeof a.select, typeof f.reset, typeof a.reset, typeof g.decode, typeof a.decode].join(',')"),
        "function,undefined,function,undefined,function,undefined"
    );
    // Non-dispatch : `div.insertAdjacentElement`/`replaceChildren`
    // lisibles partout (méthodes Node universelles).
    assert_eq!(
        evf(
            &mut e,
            "[typeof a.insertAdjacentElement, typeof a.replaceChildren].join(',')"
        ),
        "function,function"
    );
}

/// Pont de géométrie (issue #19, JS-8) — la session publie
/// `element_path → union des boîtes` dans une map partagée
/// posée sur le document par `Engine::set_geometry_ctx` :
/// `getBoundingClientRect` répond en coords **viewport**
/// (scroll soustrait — sauf sous `position:fixed` où les
/// coords sont déjà viewport, i#18) ; `offset*`/`client*`/
/// `scroll*` non racine = border-box ; `scroll*` racine =
/// dimensions du document publiées ; chemin absent → 0
/// (comportement historique, jamais de panic) ; la méthode
/// est une **référence lisible** (`.call(el)` — motif #25).
#[test]
fn geometry_bridge_gbrect_and_dims() {
    let (mut e, _jar, _navs) = engine_full(PAGE);
    e.set_viewport(800, 600);
    let geom = explorer_jscore::GeometrySlot::default();
    {
        let mut g = geom.lock().unwrap();
        g.scroll_y = 5.0;
        g.doc_w = 800.0;
        g.doc_h = 3000.0;
        // #main → html>body>div[0]
        g.rects.insert(
            vec![
                ("html".to_string(), 0),
                ("body".to_string(), 0),
                ("div".to_string(), 0),
            ],
            explorer_jscore::ElementGeom {
                rect: explorer_jscore::GeoRect {
                    x: 10.0,
                    y: 20.0,
                    w: 100.0,
                    h: 50.0,
                },
                fixed: false,
                positioned: false,
            },
        );
        // #list → html>body>ul[0] — sous un ancêtre fixed :
        // coords viewport, le scroll ne se soustrait pas.
        g.rects.insert(
            vec![
                ("html".to_string(), 0),
                ("body".to_string(), 0),
                ("ul".to_string(), 0),
            ],
            explorer_jscore::ElementGeom {
                rect: explorer_jscore::GeoRect {
                    x: 0.0,
                    y: 8.0,
                    w: 200.0,
                    h: 40.0,
                },
                fixed: true,
                positioned: true,
            },
        );
    }
    e.set_geometry_ctx(0, geom);
    assert_eq!(
        evf(
            &mut e,
            "var out=[];\
             var el=document.getElementById('main');\
             var r=el.getBoundingClientRect();\
             out.push(r.x,r.y,r.width,r.height,r.bottom,r.right);\
             out.push(el.offsetWidth,el.offsetHeight,el.clientWidth,el.scrollHeight);\
             out.push(document.getElementById('list').getBoundingClientRect().y);\
             out.push(document.getElementById('inp').offsetWidth);\
             out.push(document.documentElement.scrollHeight,document.documentElement.clientHeight);\
             out.push(Element.prototype.getBoundingClientRect.call(el).x);\
             out.join(',')"
        ),
        "10,15,100,50,65,110,100,50,100,50,8,0,3000,600,10"
    );
}

/// Sans table injectée (harnais, `set_geometry_ctx` jamais
/// appelé) : les zéros historiques — le pont est additif.
#[test]
fn geometry_absent_keeps_zero_stub() {
    let (mut e, _jar, _navs) = engine_full(PAGE);
    assert_eq!(
        evf(
            &mut e,
            "var el=document.getElementById('main');\
             var r=el.getBoundingClientRect();\
             [r.width,el.offsetWidth].join(',')"
        ),
        "0,0"
    );
}

// -- attributs énumérés à défaut spec (JS-C80) ------------------------------

const ENUM_PAGE: &str = r#"
<html><body>
<input id="i1">
<input id="i2" type="checkbox">
<input id="i3" type="BOGUS">
<button id="b1"></button>
<button id="b2" type="reset">
<button id="b3" type="BOGUS">
<select id="s1"></select>
<select id="s2" multiple></select>
<form id="f1"></form>
<form id="f2" method="POST" enctype="multipart/form-data"></form>
<textarea id="ta" wrap="HARD"></textarea>
<table><tr><th id="th1"></th></tr></table>
<img id="im">
<iframe id="fr"></iframe>
</body></html>
"#;

/// React pose `input.type` dans `isTextInputElement` : absent
/// ou invalide, la lecture IDL doit rendre `"text"` (HTML
/// §2.5.5 — « limited to only known values »).
#[test]
fn enum_attr_input_type_defaults() {
    assert_eq!(
        ev_in(ENUM_PAGE, "document.getElementById('i1').type"),
        "text"
    );
    assert_eq!(
        ev_in(ENUM_PAGE, "document.getElementById('i2').type"),
        "checkbox"
    );
    // Invalide → défaut, casse normalisée.
    assert_eq!(
        ev_in(ENUM_PAGE, "document.getElementById('i3').type"),
        "text"
    );
}

#[test]
fn enum_attr_button_and_select() {
    assert_eq!(
        ev_in(ENUM_PAGE, "document.getElementById('b1').type"),
        "submit"
    );
    assert_eq!(
        ev_in(ENUM_PAGE, "document.getElementById('b2').type"),
        "reset"
    );
    assert_eq!(
        ev_in(ENUM_PAGE, "document.getElementById('b3').type"),
        "submit"
    );
    assert_eq!(
        ev_in(ENUM_PAGE, "document.getElementById('s1').type"),
        "select-one"
    );
    assert_eq!(
        ev_in(ENUM_PAGE, "document.getElementById('s2').type"),
        "select-multiple"
    );
}

#[test]
fn enum_attr_form_media_misc() {
    assert_eq!(
        ev_in(ENUM_PAGE, "document.getElementById('f1').method"),
        "get"
    );
    // POST → mot-clé canonique minuscule.
    assert_eq!(
        ev_in(ENUM_PAGE, "document.getElementById('f2').method"),
        "post"
    );
    assert_eq!(
        ev_in(ENUM_PAGE, "document.getElementById('f1').enctype"),
        "application/x-www-form-urlencoded"
    );
    assert_eq!(
        ev_in(ENUM_PAGE, "document.getElementById('f2').encoding"),
        "multipart/form-data"
    );
    assert_eq!(
        ev_in(ENUM_PAGE, "document.getElementById('ta').wrap"),
        "hard"
    );
    assert_eq!(
        ev_in(ENUM_PAGE, "document.getElementById('th1').scope"),
        "auto"
    );
    assert_eq!(
        ev_in(ENUM_PAGE, "document.getElementById('im').decoding"),
        "auto"
    );
    assert_eq!(
        ev_in(ENUM_PAGE, "document.getElementById('im').loading"),
        "eager"
    );
    assert_eq!(
        ev_in(ENUM_PAGE, "document.getElementById('fr').loading"),
        "eager"
    );
}

/// Les éléments hors table gardent le reflet brut (`""`).
#[test]
fn enum_attr_unlisted_tags_stay_raw() {
    assert_eq!(ev_in(ENUM_PAGE, "document.createElement('div').type"), "");
    // `track.kind` : absent → `subtitles`, invalide → `metadata`.
    assert_eq!(
        ev_in(ENUM_PAGE, "document.createElement('track').kind"),
        "subtitles"
    );
    assert_eq!(
        ev_in(
            ENUM_PAGE,
            "var t=document.createElement('track');t.setAttribute('kind','bogus');t.kind"
        ),
        "metadata"
    );
}

/// L'écriture reflète l'attribut brut ; la lecture clampe —
/// aller-retour fidèle pour les valeurs du vocabulaire.
#[test]
fn enum_attr_set_reflects_get_clamps() {
    assert_eq!(
        ev_in(
            ENUM_PAGE,
            "var i=document.getElementById('i1');i.type='radio';i.type"
        ),
        "radio"
    );
    assert_eq!(
        ev_in(
            ENUM_PAGE,
            "var i=document.getElementById('i1');i.type='BOGUS';[i.type,i.getAttribute('type')].join('|')"
        ),
        "text|BOGUS"
    );
}

// -- méthodes lisibles + handlers `on*` (JS-C80) -----------------------------

/// Les méthodes host étaient invoke-only : invisibles à
/// `typeof`/`in`. `canUseDOM` de React exige
/// `typeof document.createElement === 'function'` — sinon
/// `isInputEventSupported` tombe faux et `onChange` passe au
/// polyfill IE8 (`propertychange`) : champs contrôlés morts.
#[test]
fn methods_are_readable_functions() {
    assert_eq!(ev("typeof document.createElement"), "function");
    assert_eq!(ev("typeof document.getElementById"), "function");
    assert_eq!(ev("typeof document.querySelectorAll"), "function");
    assert_eq!(ev("typeof document.write"), "function");
    assert_eq!(ev("'createElement' in document"), "true");
    // Éléments : les méthodes universelles aussi.
    assert_eq!(
        ev("typeof document.getElementById('main').appendChild"),
        "function"
    );
    assert_eq!(
        ev("typeof document.getElementById('main').setAttribute"),
        "function"
    );
    assert_eq!(
        ev("'querySelector' in document.getElementById('main')"),
        "true"
    );
    // Hors table → toujours `undefined` (pas de promesse vide).
    assert_eq!(ev("typeof document.pasUneMethode"), "undefined");
    // Hors gate de tag : `div.submit` n'existe pas côté spec.
    assert_eq!(
        ev("typeof document.getElementById('main').submit"),
        "undefined"
    );
}

/// La fonction lue est appelable — elle redispatche `invoke`.
#[test]
fn readable_method_is_callable() {
    assert_eq!(ev("var f=document.createElement;f('span').tagName"), "SPAN");
    assert_eq!(
        ev("var d=document.getElementById('main'),g=d.querySelector;g.call(d,'b').textContent"),
        "world"
    );
    assert_eq!(ev("document.createElement.call(document,'i').tagName"), "I");
}

/// `el.on*`/`document.on*` : propre scriptée prioritaire, puis
/// attribut compilé (JS-C45), sinon `null` — la surface est
/// **détectable** (`"oninput" in document`), ce que le sniff
/// `Lj` de React exige pour armer `ChangeEventPlugin`.
#[test]
fn on_handlers_readable_and_detectable() {
    assert_eq!(ev("document.getElementById('main').onclick"), "null");
    assert_eq!(ev("document.oninput"), "null");
    assert_eq!(ev("'oninput' in document"), "true");
    assert_eq!(ev("'onchange' in document"), "true");
    // Nom inconnu → pas un handler (`undefined`, fidèle spec).
    assert_eq!(ev("'onpasuntruc' in document"), "false");
    // Attribut compilé lisible en prop.
    assert_eq!(
        ev_in(
            "<html><body><div id=\"x\" onclick=\"void 0\"></div></body></html>",
            "typeof document.getElementById('x').onclick"
        ),
        "function"
    );
    // Propre scriptée gagne sur l'attribut.
    assert_eq!(
        ev_in(
            "<html><body><div id=\"x\" onclick=\"void 0\"></div></body></html>",
            "var d=document.getElementById('x');var f=function(){};d.onclick=f;d.onclick===f"
        ),
        "true"
    );
}

// -- `MessageChannel`/`MessagePort` (JS-C81) --------------------------------

/// La voie rapide des schedulers : `port2.postMessage` →
/// `message` sur `port1` au checkpoint de pompe (React
/// préfère `MessageChannel` à `setTimeout`).
#[test]
fn message_channel_roundtrip() {
    let mut e = engine_with(PAGE);
    e.eval(
        "var ch=new MessageChannel();\
         var got='none';\
         ch.port1.onmessage=function(ev){got='M:'+ev.data};\
         ch.port2.postMessage('hi')",
    )
    .expect("eval");
    e.run_microtasks().expect("pump");
    let v = e.eval("got").expect("read");
    assert_eq!(e.describe(v), "M:hi");
}

/// `typeof` détectable, structured clone des données,
/// `addEventListener` et `close` (défausse à la livraison).
#[test]
fn message_channel_surface() {
    let mut e = engine_with(PAGE);
    e.eval(
        "var out=[];         var ch=new MessageChannel();         out.push(typeof MessageChannel);         out.push(typeof ch.port1.postMessage);         out.push(ch.port1.onmessage===null);         out.push(String(ch));         ch.port1.addEventListener('message',function(ev){out.push('L:'+ev.data.v)});         ch.port2.postMessage({v:42});         window.__out=out;",
    )
    .expect("eval");
    e.run_microtasks().expect("pump");
    // `close` défausse les messages en file du port (spec :
    // désentrelacement).
    e.eval(
        "var c2=new MessageChannel();         c2.port1.onmessage=function(){window.__out.push('BAD')};         c2.port2.postMessage('x');         c2.port1.close();",
    )
    .expect("eval2");
    e.run_microtasks().expect("pump2");
    let v = e.eval("window.__out.join('|')").expect("read");
    assert_eq!(
        e.describe(v),
        "function|function|true|[object MessageChannel]|L:42"
    );
}

// -- accesseurs IDL + `constructor` typé (JS-C82) ----------------------------

/// `inputValueTracking` de React exige un vrai descripteur
/// `{get,set}` sur `ctor.prototype` + `defineProperty`
/// accesseur own sur l'élément — la chaîne complète.
#[test]
fn idl_accessors_descriptor_surface() {
    assert_eq!(
        ev("var i=document.getElementById('inp');i.constructor.name"),
        "HTMLInputElement"
    );
    assert_eq!(
        ev("var i=document.getElementById('inp');\
            var d=Object.getOwnPropertyDescriptor(i.constructor.prototype,'value');\
            d && typeof d.get + '/' + typeof d.set + '/' + d.configurable"),
        "function/function/true"
    );
    // defineProperty accesseur own : interception réelle.
    assert_eq!(
        ev("var i=document.getElementById('inp');var seen='';\
            var d=Object.getOwnPropertyDescriptor(i.constructor.prototype,'value');\
            Object.defineProperty(i,'value',{configurable:true,\
              get:function(){return d.get.call(this)},\
              set:function(v){seen='S:'+v;d.set.call(this,v)}});\
            i.value='zz';seen+'|'+i.value+'|'+i.getAttribute('value')"),
        "S:zz|zz|zz"
    );
    // `new HTMLInputElement()` — illegal constructor (spec).
    assert_eq!(
        ev("try{new document.getElementById('inp').constructor()}catch(e){e.message}"),
        "illegal constructor"
    );
}

// -- `MouseEventInit` + dispatch littéral (JS-C83) ----------------------------

/// React Router `<Link>` garde `event.button === 0` — sans les
/// défauts `MouseEventInit` au ctor et leur recopie au dispatch
/// d'un événement littéral, le clic est ignoré.
#[test]
fn mouse_event_defaults_reach_dispatch() {
    // Constructeur : défauts numériques.
    assert_eq!(
        ev("var m=new MouseEvent('click');m.button+'/'+m.buttons+'/'+m.clientX+'/'+m.ctrlKey"),
        "0/0/0/false"
    );
    // Init dict lu (`button:2`) + le reste par défaut.
    assert_eq!(
        ev("var m=new MouseEvent('click',{button:2,shiftKey:true});m.button+'/'+m.shiftKey+'/'+m.pageY"),
        "2/true/0"
    );
    // Dispatch : les props atteignent les listeners.
    assert_eq!(
        ev(
            "var b=document.createElement('b');document.body.appendChild(b);var seen='';\
            document.addEventListener('click',function(e){seen='d:'+e.button+'/'+e.clientX});\
            b.dispatchEvent(new MouseEvent('click',{bubbles:true,clientX:7}));seen"
        ),
        "d:0/7"
    );
}

// -- init d'événements spécialisés (JS-C84) ---------------------------------

/// `WheelEventInit`/`PointerEventInit`/`FocusEvent` : les
/// payloads d'init sont copiés au ctor **et** rejoignent les
/// listeners au dispatch littéral — `e.deltaY` était `0`
/// pour `onWheel` React avant ce lot.
#[test]
fn specialized_event_init_reaches_dispatch() {
    // Ctor : `deltaY`/`pointerType`/`relatedTarget` lus.
    assert_eq!(
        ev("var w=new WheelEvent('wheel',{deltaY:120,pointerType:'mouse'});w.deltaY+'/'+w.pointerType"),
        "120/mouse"
    );
    // Dispatch : les valeurs atteignent les handlers.
    assert_eq!(
        ev(
            "var b=document.createElement('b');document.body.appendChild(b);var seen='';\
            b.addEventListener('wheel',function(e){seen='w:'+e.deltaY+'/'+e.deltaMode});\
            b.dispatchEvent(new WheelEvent('wheel',{deltaY:120,deltaMode:1}));seen"
        ),
        "w:120/1"
    );
    assert_eq!(
        ev(
            "var b=document.createElement('b');document.body.appendChild(b);var seen='';\
            b.addEventListener('focusin',function(e){seen='f'});\
            b.dispatchEvent(new FocusEvent('focusin',{bubbles:true,relatedTarget:null}));seen"
        ),
        "f"
    );
}

// ===== CAP-178 — batch mesuré (sonde `_census_calls` :
// TypeError/ReferenceError terrain) : `Audio`/`Video` ctors,
// `getHTML`/`setHTMLUnsafe`, `checkVisibility`, stubs fenêtre
// legacy, `XMLSerializer`, ctors événements.

#[test]
fn cap178_surface_batch() {
    let mut e = engine_with("<!doctype html><html><body><div id=x><b>hi</b></div></body></html>");
    e.eval("var x=document.getElementById('x');")
        .expect("setup");
    // `new Audio(url)`/`new Video()` — alias createElement.
    assert_eq!(
        evf(
            &mut e,
            "var a=new Audio('/m.mp3'),v=new Video();\
            [a.tagName,v.tagName,a.getAttribute('src'),typeof a.play].join('|')"
        ),
        "AUDIO|VIDEO|/m.mp3|undefined"
    );
    // `getHTML`/`setHTMLUnsafe` — outerHTML/innerHTML.
    assert_eq!(
        evf(
            &mut e,
            "var t=document.createElement('div');t.setHTMLUnsafe('<p>w</p>');\
            [t.innerHTML, t.getHTML()].join('|')"
        ),
        "<p>w</p>|<div><p>w</p></div>"
    );
    // `checkVisibility` — attaché+visible / détaché / hidden.
    assert_eq!(
        evf(&mut e,
            "var d=document.createElement('div');d.style.display='none';var h=document.createElement('div');\
            h.setAttribute('hidden','');document.body.appendChild(h);\
            [x.checkVisibility(), d.checkVisibility(), h.checkVisibility()].join(',')"),
        "true,false,false"
    );
    // Stubs fenêtre honnêtes : `find`→false, no-ops undefined
    // (join rend `undefined` vide — on vérifie `typeof` à
    // la place pour la lisibilité).
    assert_eq!(
        evf(
            &mut e,
            "[window.find('x'), typeof window.find, window.moveTo(0,0)===undefined,\
             window.resizeTo(1,1)===undefined, window.openDialog('u')===undefined,\
             window.showModalDialog('u')===undefined].join('|')"
        ),
        "false|function|true|true|true|true"
    );
    // `XMLSerializer` — élément et document.
    assert_eq!(
        evf(&mut e,
            "var xs=new XMLSerializer();\
            [xs.serializeToString(x), xs.serializeToString(document).indexOf('<body>')>-1].join('|')"),
        "<div id=\"x\"><b>hi</b></div>|true"
    );
    // Ctors événements — init copié.
    assert_eq!(
        evf(&mut e,
            "var m=new MessageEvent('message',{data:'d'});var s=new StorageEvent('storage',{key:'k',oldValue:'1'});\
            var c=new CloseEvent('close',{wasClean:true,code:1000});\
            [m.data,s.key+'/'+s.oldValue,c.wasClean+'/'+c.code].join('|')"),
        "d|k/1|true/1000"
    );
    // `Worker` — refus honnête sans chargeur (l'embedder
    // installe `worker_factory` ; jamais de faux succès).
    assert_eq!(
        evf(
            &mut e,
            "try{new Worker('/w.js')}catch(err){err.name+'|'+err.message.slice(0,24)}"
        ),
        "TypeError|Worker: no loader instal"
    );
}
// -- décomposition URL `<a>`/`<area>` (JS-C87) --------------------------------

/// Le patron « parse via ancre » (`c.href=u; c.pathname`) des
/// libs de télémétrie : `pathname` doit rendre une **chaîne**
/// (`""` sans href) — `undefined` tuait gtag (`d[0]`).
#[test]
fn anchor_url_decomposition() {
    assert_eq!(
        ev(
            "var a=document.createElement('a');a.href='https://ex.com:8080/p/q?x=1#f';\
            [a.protocol,a.hostname,a.port,a.pathname,a.search,a.hash,a.origin].join('|')"
        ),
        "https:|ex.com|8080|/p/q|?x=1|#f|https://ex.com:8080"
    );
    // Sans `href` : chaîne vide — `d[0]` lit un char absent, pas TypeError.
    assert_eq!(
        ev("var a=document.createElement('a');var d=a.pathname;typeof d+'|'+d"),
        "string|"
    );
    // `<area>` pareil, `<div>` inerte.
    assert_eq!(
        ev("var a=document.createElement('area');a.href='https://x/y';\
            var d=document.createElement('div');\
            [a.pathname, typeof d.pathname].join('|')"),
        "/y|undefined"
    );
}

// -- `document.head` auto-créé (JS-C89) ----------------------------------------

/// Spec : `head` existe toujours (parseur l'auto-génère) —
/// React 19 `mountHoistable` fait `document.head.insertBefore`
/// pour `<title>`/metadata. Un doc sans `<head>` explicit doit
/// quand même le servir (créé + inséré avant `body`).
#[test]
fn document_head_always_exists() {
    let mut e = engine_with("<!doctype html><html><body></body></html>");
    let v = e
        .eval("document.head ? document.head.tagName : 'null'")
        .expect("head");
    assert_eq!(e.describe(v), "HEAD");
    // head créé = premier enfant de html, avant body.
    let v = e.eval(
        "var h=document.documentElement;[h.firstElementChild.tagName,h.children[1].tagName].join('/')",
    ).expect("order");
    assert_eq!(e.describe(v), "HEAD/BODY");
}

// -- méthodes `Array.prototype` génériques (JS-C90) ---------------------------

/// Spec : les méthodes `Array.prototype` n'exigent pas un vrai
/// tableau — `length` + `[[Get]]`/`[[Set]]`/`[[Delete]]` indices.
/// jQuery attache `push`/`sort`/`splice` à ses instances : ses
/// helpers mouraient « array method called on non-array ».
#[test]
fn array_methods_generic_on_array_like() {
    // push/pop/sort/splice/slice/concat sur `{length,indices}`.
    assert_eq!(
        ev("var o={length:2,'0':'b','1':'a'};\
            [].push.call(o,'c');o.length+'/'+o['2']"),
        "3/c"
    );
    assert_eq!(
        ev("var o={length:3,'0':'x','1':'a','2':'m'};\
            [].sort.call(o);[o['0'],o['1'],o['2']].join('')"),
        "amx"
    );
    assert_eq!(
        ev("var o={length:4,'0':1,'1':2,'2':3,'3':4};\
            var d=[].splice.call(o,1,2,'z');o.length+'/'+o['1']+'/'+d.join(',')"),
        "3/z/2,3"
    );
    assert_eq!(
        ev("var o={length:3,'0':'a','1':'b','2':'c'};\
            [].slice.call(o,1).join(',')"),
        "b,c"
    );
    // La queue est [[Delete]]'ée : pop raccourcit et efface le dernier slot.
    assert_eq!(
        ev("var o={length:3,'0':'a','1':'b','2':'c'};\
            [].pop.call(o);o.length+'|'+('2' in o)"),
        "2|false"
    );
}

// -- `DOMParser` (JS-C91) -----------------------------------------------------

/// `new DOMParser().parseFromString(html,'text/html')` — htmx
/// parse ses réponses `hx-swap` ainsi ; le ctor était classé
/// `illegal_ctor`.
#[test]
fn dom_parser_constructs_and_parses() {
    assert_eq!(
        ev(
            "var d=new DOMParser().parseFromString('<p id=x>hi</p>','text/html');\
            d.body.childNodes.length+'/'+d.getElementById('x').textContent"
        ),
        "1/hi"
    );
    // MIME non-HTML : document vide (déviation — pas de parseur XML).
    assert_eq!(
        ev("new DOMParser().parseFromString('<a/>','text/xml').body.childNodes.length"),
        "0"
    );
}

/// Cross-document (vol spectrum) : un nœud d'une **autre arène**
/// (`DOMParser`, `iframe.contentDocument`, `xhr.responseXML`)
/// passé aux mutateurs du document courant validait l'id contre
/// SA propre arène puis l'indexait dans celle du receveur —
/// hors-limites = panic (`is_ancestor_or_self`), in-bounds =
/// mutation du mauvais nœud (le `<main>` de la page détaché
/// puis propagé au DOM session par le journal). `node_for_in`
/// borne à l'arène du receveur → `TypeError` amont, jamais de
/// corruption. La spec adopterait cross-document ; les arènes
/// ne fusionnant pas, le dégradé honnête est le rejet.
#[test]
fn cross_document_node_args_are_rejected() {
    // Mutateurs structurels : TypeError sur nœud étranger.
    assert_eq!(
        ev(
            "var d=new DOMParser().parseFromString('<div id=x>y</div>','text/html');\
            var fx=d.getElementById('x');\
            var out=[];\
            try{document.body.appendChild(fx)}catch(e){out.push('a')}\
            try{document.body.insertBefore(fx,null)}catch(e){out.push('i')}\
            try{document.body.replaceChild(fx,document.body.firstChild)}catch(e){out.push('r')}\
            try{document.body.removeChild(fx)}catch(e){out.push('d')}\
            try{document.body.insertAdjacentElement('beforeend',fx)}catch(e){out.push('j')}\
            try{document.body.prepend(fx)}catch(e){out.push('p')}\
            out.join('')"
        ),
        "airdjp"
    );
    // Prédicats : pas de throw, mais jamais d'égalité/
    // contenance factice cross-document.
    assert_eq!(
        ev(
            "var d=new DOMParser().parseFromString('<div id=x>y</div>','text/html');\
            var fx=d.getElementById('x');\
            document.body.contains(fx)+'/'+document.body.isSameNode(fx)+'/'+\
            document.body.isEqualNode(fx)"
        ),
        "false/false/false"
    );
    // Régression directe du symptôme : l'append d'un nœud
    // étranger ne doit **pas** détacher/rattacher le nœud
    // local de même indice (le `<main>` spectrum).
    assert_eq!(
        ev_in(
            "<html><body><main><div>s1</div></main><footer>f</footer></body></html>",
            "var d=new DOMParser().parseFromString('<a/><b/><c/>','text/html');\
                var f=d.querySelector('b');\
                try{document.body.appendChild(f)}catch(e){}\
                document.getElementsByTagName('main').length+'/'+\
                document.querySelectorAll('main > div').length"
        ),
        "1/1"
    );
    // Observateurs/Range/Selection : mêmes rejets bornés.
    assert_eq!(
        ev("var d=new DOMParser().parseFromString('<div id=x>y</div>','text/html');\
            var fx=d.getElementById('x');\
            var out=[];\
            try{new MutationObserver(function(){}).observe(fx,{childList:true})}catch(e){out.push('m')}\
            try{new IntersectionObserver(function(){}).observe(fx)}catch(e){out.push('o')}\
            try{document.createRange().selectNode(fx)}catch(e){out.push('r')}\
            try{window.getSelection().selectAllChildren(fx)}catch(e){out.push('s')}\
            out.join('')"),
        "mors"
    );
}

/// `delete el.dataset.x` doit retirer `data-x` (vol spectrum :
/// `delete section.dataset.status` tombait sur la prop JS
/// propre — l'attribut restait, `div[data-status]` restait
/// `display:none` et la page ne se révélait jamais). Sans le
/// crochet `Host::del`, la suppression visait une prop
/// fantôme ; `delete` en strict sur un host non-supprimable
/// jetait même `TypeError`.
#[test]
fn dataset_delete_removes_data_attr() {
    assert_eq!(
        ev("var el=document.createElement('div');\
            el.dataset.status='decorated';\
            var before=el.getAttribute('data-status');\
            delete el.dataset.status;\
            before+'/'+el.getAttribute('data-status')+'/'+('status' in el.dataset)+'/'+el.dataset.status"),
        "decorated/null/false/undefined"
    );
    // camelCase ↔ kebab-case sur le chemin delete.
    assert_eq!(
        ev("var el=document.createElement('div');\
            el.setAttribute('data-block-name','x');\
            delete el.dataset.blockName;\
            el.getAttribute('data-block-name')"),
        "null"
    );
}

// -- `classList` méthodes lisibles (JS-C91) -----------------------------------

/// `classList.add` doit se lire comme fonction — htmx fait
/// `e.classList["add"].call(e.classList,…)` : `.call` sur
/// `undefined` tuait `hx-indicator`.
#[test]
fn classlist_methods_readable() {
    assert_eq!(
        ev("var el=document.createElement('div');\
            typeof el.classList['add']+'/'+\
            (el.classList['add'].call(el.classList,'x'),el.className)"),
        "function/x"
    );
}

// (le test `xhr.upload` vit dans `tests/xhr.rs` — son
// harnais installe le transport stub)

// ===== CAP-179 — fullscreen + popover (sonde `_census_calls` :
// `requestFullscreen`/`exitFullscreen`/`fullscreenElement`,
// `showPopover`/`hidePopover`/`togglePopover` absents).

#[test]
fn cap179_fullscreen_popover() {
    let mut e = engine_with(
        "<!doctype html><html><body><div id=x></div>\
        <div id=p popover></div><div id=q></div></body></html>",
    );
    e.eval("var x=document.getElementById('x'),p=document.getElementById('p'),q=document.getElementById('q'),d=document;")
        .expect("setup");
    // `requestFullscreen` : promesse résolue, `fullscreenElement`,
    // `fullscreenchange` ; détaché → rejet.
    assert_eq!(
        evf(
            &mut e,
            "var fs='';d.addEventListener('fullscreenchange',function(){fs+='!'});\
            x.requestFullscreen().then(function(){fs+='ok'});\
            [d.fullscreenElement===x, d.fullscreen, d.fullscreenEnabled].join('|')"
        ),
        "true|true|true"
    );
    e.eval("var rj='';document.createElement('div').requestFullscreen().catch(function(){rj='catch'});")
        .expect("detached");
    e.run_microtasks().expect("microtasks");
    assert_eq!(evf(&mut e, "rj"), "catch");
    // `exitFullscreen` : efface + événement ; double-exit = no-op.
    assert_eq!(
        evf(
            &mut e,
            "d.exitFullscreen();d.exitFullscreen();\
            [d.fullscreenElement===null, d.fullscreen].join('|')"
        ),
        "true|false"
    );
    // Popover : show/hide/toggle + `InvalidStateError` rules.
    assert_eq!(
        evf(&mut e,
            "var tg='';p.addEventListener('toggle',function(){tg+='T'});p.addEventListener('beforetoggle',function(){tg+='b'});\
            p.showPopover();p.showPopover===undefined;\
            [tg, typeof q.showPopover].join('|')"),
        "bT|undefined"
    );
    assert_eq!(
        evf(
            &mut e,
            "var errs='';try{p.showPopover()}catch(err){errs+=err.name};\
            try{q.showPopover()}catch(err){errs+='/'+err.name};\
            [errs, p.togglePopover(), p.togglePopover()].join('|')"
        ),
        "InvalidStateError/InvalidStateError|false|true"
    );
    assert_eq!(
        evf(&mut e, "try{p.hidePopover();'noerr'}catch(err){err.name}"),
        "noerr"
    );
}

// -- `WebSocket` réel (JS-C94) -------------------------------------------------

/// Transport simulé : `open` rend un handle fixe, `send`/
/// `close` notés — les événements sont injectés par le test
/// via `deliver_ws_event` (comme le harnais `deliver_xhr`).
struct FakeWsTr(std::cell::RefCell<Vec<String>>);
impl explorer_jscore::WsTransport for FakeWsTr {
    fn open(&self, _r: &explorer_jscore::WsRequest) -> u64 {
        7
    }
    fn send(&self, _h: u64, out: explorer_jscore::WsOut) {
        self.0.borrow_mut().push(format!("send:{out:?}"));
    }
    fn close(&self, h: u64, c: u16, _r: &str) {
        self.0.borrow_mut().push(format!("close:{h}:{c}"));
    }
}

#[test]
fn ws_lifecycle_echo() {
    let mut e = engine_with("<!doctype html><html><body><p id=p>?</p></body></html>");
    let tr: std::rc::Rc<dyn explorer_jscore::WsTransport> =
        std::rc::Rc::new(FakeWsTr(std::cell::RefCell::new(Vec::new())));
    e.install_ws_transport(tr);
    e.eval(
        "var ws=new WebSocket('ws://h.test/x','p1');var out=[];\
        ws.onopen=function(){out.push('open:'+ws.readyState);ws.send('hi')};\
        ws.onmessage=function(ev){out.push('msg:'+ev.data);ws.close(1000,'bye')};\
        ws.onclose=function(ev){out.push('close:'+ev.code+'/'+ev.wasClean);\
        document.getElementById('p').textContent=out.join('|')}",
    )
    .expect("script");
    // `open` → onopen → send('hi') via transport.
    e.deliver_ws_event(0, 7, explorer_jscore::WsEventKind::Open("p1".into()));
    // echo texte → onmessage → close(1000).
    e.deliver_ws_event(0, 7, explorer_jscore::WsEventKind::Text("hi".into()));
    // close serveur → onclose → DOM.
    e.deliver_ws_event(
        0,
        7,
        explorer_jscore::WsEventKind::Close {
            code: 1000,
            reason: "bye".into(),
            clean: true,
        },
    );
    let v = e
        .eval("document.getElementById('p').textContent")
        .expect("dom");
    assert_eq!(e.describe(v), "open:1|msg:hi|close:1000/true");
}
// ===== CAP-180 — hit-test inverse + getClientRects
// (sonde `_census_calls` : `elementFromPoint`/`elementsFromPoint`/
// `caretPositionFromPoint`/`getClientRects` absents) — même
// `GeometryMap` publiée que `gBCR` (i#19).

#[test]
fn cap180_inverse_geometry() {
    let (mut e, _jar, _navs) = engine_full(PAGE);
    let geom = explorer_jscore::GeometrySlot::default();
    {
        let mut g = geom.lock().unwrap();
        g.scroll_y = 5.0;
        // #main div — document coords (10,20)-(110,70),
        // viewport (10,15)-(110,65) après scroll.
        g.rects.insert(
            vec![
                ("html".to_string(), 0),
                ("body".to_string(), 0),
                ("div".to_string(), 0),
            ],
            explorer_jscore::ElementGeom {
                rect: explorer_jscore::GeoRect {
                    x: 10.0,
                    y: 20.0,
                    w: 100.0,
                    h: 50.0,
                },
                fixed: false,
                positioned: false,
            },
        );
        // #list ul — fixed, déjà viewport (10,15)-(110,65)
        // — chevauche le div : le frère tardif gagne (DFS desc).
        g.rects.insert(
            vec![
                ("html".to_string(), 0),
                ("body".to_string(), 0),
                ("ul".to_string(), 0),
            ],
            explorer_jscore::ElementGeom {
                rect: explorer_jscore::GeoRect {
                    x: 10.0,
                    y: 15.0,
                    w: 100.0,
                    h: 50.0,
                },
                fixed: true,
                positioned: true,
            },
        );
    }
    e.set_geometry_ctx(0, geom);
    assert_eq!(
        evf(
            &mut e,
            "var m=document.getElementById('main'),l=document.getElementById('list');\
            var top=document.elementFromPoint(50,40),all=document.elementsFromPoint(50,40);\
            var cp=document.caretPositionFromPoint(50,40);\
            [top===l, all.length, all[0]===l, all[1]===m, cp.offsetNode===l, cp.offset].join('|')"
        ),
        "true|2|true|true|true|0"
    );
    // Hors rect → null / liste vide.
    assert_eq!(
        evf(
            &mut e,
            "[document.elementFromPoint(500,500)===null,\
             document.elementsFromPoint(500,500).length,\
             document.caretPositionFromPoint(500,500)===null].join('|')"
        ),
        "true|0|true"
    );
    // `getClientRects` — union bornée : `[rect]` si géom,
    // `[]` sinon (détaché). `typeof` honnête partout.
    assert_eq!(
        evf(
            &mut e,
            "var r=m.getClientRects();\
            [r.length, r[0].x, r[0].y, r[0].width,\
             document.createElement('div').getClientRects().length,\
             typeof document.elementFromPoint].join('|')"
        ),
        "1|10|15|100|0|function"
    );
}

// ===== CAP-181 — `navigator.clipboard` par hook embedder
// (sonde `_census_calls` : `writeText` TypeError/absent) —
// sans hook → `NotAllowedError` honnête.

#[test]
fn cap181_clipboard_hook() {
    let (mut e, _jar, _navs) = engine_full(PAGE);
    // Surface : objet présent, méthodes lisibles.
    assert_eq!(
        evf(
            &mut e,
            "[typeof navigator.clipboard, typeof navigator.clipboard.writeText,\
             typeof navigator.clipboard.readText, navigator.clipboard.toString()].join('|')"
        ),
        "object|function|function|[object Clipboard]"
    );
    // Sans hook → `NotAllowedError` rejetée (honnête).
    e.eval(
        "var wr='',rd='';navigator.clipboard.writeText('x').catch(function(v){wr=v});\
            navigator.clipboard.readText().catch(function(v){rd=v});",
    )
    .expect("deny");
    e.run_microtasks().expect("microtasks");
    assert_eq!(
        evf(&mut e, "wr.slice(0,15)+'|'+rd.slice(0,15)"),
        "NotAllowedError|NotAllowedError"
    );
    // Hook écriture/livraison — l'embedder décide.
    e.set_clipboard_hooks(
        Some(std::rc::Rc::new(|t| {
            if t == "ok" {
                Ok(())
            } else {
                Err("denied".into())
            }
        })),
        Some(std::rc::Rc::new(|| Some("clip!".into()))),
    );
    e.eval(
        "var w='',r='';navigator.clipboard.writeText('ok').then(function(){w='resolved'});\
            navigator.clipboard.writeText('no').catch(function(v){w+='/reject'});\
            navigator.clipboard.readText().then(function(v){r=v});",
    )
    .expect("hooks");
    e.run_microtasks().expect("microtasks");
    assert_eq!(evf(&mut e, "w+'|'+r"), "resolved/reject|clip!");
}

// ===== JS-C157 — props `navigator` triviales (CMP/feature-detects)

#[test]
fn navigator_spec_props() {
    let mut e = engine_with("<p>x</p>");
    // Valeurs honnêtes low-fingerprint — `undefined` cassait
    // la lecture (`navigator.onLine`, `hardwareConcurrency`).
    assert_eq!(
        evf(
            &mut e,
            "[typeof navigator.onLine, navigator.onLine,\
             typeof navigator.cookieEnabled, navigator.cookieEnabled,\
             navigator.doNotTrack === null,\
             navigator.globalPrivacyControl, navigator.webdriver,\
             navigator.maxTouchPoints, navigator.hardwareConcurrency,\
             navigator.hardwareConcurrency >= 1].join('|')"
        ),
        "boolean|true|boolean|true|true|false|false|0|4|true"
    );
}

// ===== CAP-182 — arc édition : `new Range`/`createRange`,
// `createTreeWalker`, `execCommand` (sonde `_census_calls`).

#[test]
fn cap182_editing_arc() {
    let mut e = engine_with(
        "<!doctype html><html><body><div id=x><p id=a>hello</p><p id=b>world</p></div></body></html>",
    );
    e.eval(
        "var x=document.getElementById('x'),a=document.getElementById('a'),\
            b=document.getElementById('b'),d=document,s=d.getSelection();",
    )
    .expect("setup");
    // `new Range()` — frontières document + selectNode +
    // addRange → `getRangeAt`/`toString`.
    assert_eq!(
        evf(&mut e,
            "var r=new Range();r.selectNode(a);\
            [r.startContainer===x, r.startOffset, r.endOffset, r.collapsed, r.toString()].join('|')"),
        "true|0|1|false|hello"
    );
    // selectNodeContents + collapse + cloneContents fragment.
    assert_eq!(
        evf(&mut e,
            "r.selectNodeContents(x);\
            var f=r.cloneContents();\
            [f.childNodes.length, f.childNodes[0].textContent, r.startOffset, r.endOffset].join('|')"),
        "2|hello|0|2"
    );
    // TreeWalker — DFS filtré (`SHOW_ELEMENT`), `nextNode`
    // itère ; `firstChild` = premier descendant (avant
    // `extractContents`, qui détache les `<p>`).
    assert_eq!(
        evf(&mut e,
            "var tw=d.createTreeWalker(document.body, NodeFilter.SHOW_ELEMENT);\
            var seen='';for(var i=0;i<3;i++){var n=tw.nextNode();seen+=(n?n.id||n.tagName:'' )+'/'};\
            seen+'|'+typeof d.createTreeWalker"),
        "x/a/b/|function"
    );
    // extractContents = clone + delete ; deleteContents sur
    // sous-range texte.
    assert_eq!(
        evf(
            &mut e,
            "var r2=d.createRange();r2.selectNodeContents(x);\
            var f2=r2.extractContents();\
            [x.childNodes.length, f2.childNodes.length, x.textContent].join('|')"
        ),
        "0|2|"
    );
    // execCommand — `selectAll`→true range body ; `copy`→false
    // sans hook ; commande inconnue→false (honnête).
    assert_eq!(
        evf(
            &mut e,
            "d.execCommand('selectAll');\
            [s.rangeCount, d.execCommand('copy'), d.execCommand('bold'),\
             typeof d.createRange, typeof Range].join('|')"
        ),
        "1|false|false|function|function"
    );
    // addRange d'un vrai Range → `toString` de la sélection.
    assert_eq!(
        evf(
            &mut e,
            "s.removeAllRanges();var r3=new Range();r3.selectNodeContents(document.body);\
            s.addRange(r3);\
            [s.rangeCount, s.anchorNode===document.body, s.anchorOffset, s.focusOffset,\
             typeof r3.setStart].join('|')"
        ),
        "1|true|0|1|function"
    );
}

// ===== CAP-183 — surfaces honnêtes : `createNodeIterator`,
// `SharedWorker` gate, `CSSStyleSheet` constructible borné,
// ctors illegal-ctor spec (`TreeWalker`/`MutationRecord`/
// `IdleDeadline`/`NodeIterator` → TypeError, pas ReferenceError).

#[test]
fn cap183_honest_surfaces() {
    let mut e = engine_with(
        "<!doctype html><html><body><div id=x><p id=a>hello</p><p id=b>world</p></div></body></html>",
    );
    e.eval(
        "var x=document.getElementById('x'),a=document.getElementById('a'),\
            b=document.getElementById('b'),d=document;",
    )
    .expect("setup");
    // NodeIterator — document-order : premier nextNode = root
    // si accepté ; previousNode recule ; detach no-op.
    assert_eq!(
        evf(
            &mut e,
            "var ni=d.createNodeIterator(document.body, NodeFilter.SHOW_ELEMENT);\
            [ni.nextNode().tagName, ni.nextNode().id, ni.nextNode().id,\
             ni.nextNode()===null, ni.previousNode().id].join('|')"
        ),
        "BODY|x|a|false|a"
    );
    // Illegal-ctor spec : interfaces définies → TypeError au
    // new (fidelity — pas ReferenceError).
    assert_eq!(
        evf(
            &mut e,
            "var s='';try{new TreeWalker()}catch(v){s+='T'}try{new NodeIterator()}catch(v){s+='N'}\
            try{new MutationRecord()}catch(v){s+='M'}try{new IdleDeadline()}catch(v){s+='I'}\
            [s, typeof TreeWalker, typeof NodeIterator].join('|')"
        ),
        "TNMI|function|function"
    );
    // `new CSSStyleSheet()` — constructible borné : surface
    // `{cssRules, insertRule, deleteRule, replace, replaceSync}`
    // (règles stockées, cascade = chantier session).
    assert_eq!(
        evf(
            &mut e,
            "var sh=new CSSStyleSheet();\
            var i=sh.insertRule('div{color:red}');sh.insertRule('p{}');sh.deleteRule(1);\
            sh.replaceSync('a{}');\
            [typeof sh.insertRule, i, sh.cssRules.length, sh.type, sh.disabled,\
             sh.toString(), typeof sh.replace].join('|')"
        ),
        "function|0|0|text/css|false|[object CSSStyleSheet]|function"
    );
    // `new SharedWorker` — ctor défini → TypeError gate
    // honnête sans loader (comme `Worker`) ; avec factory →
    // `{port}` (testé côté harness worker).
    assert_eq!(
        evf(
            &mut e,
            "var t='';try{new SharedWorker('w.js')}catch(v){t=v.name}\
            [t, typeof SharedWorker].join('|')"
        ),
        "TypeError|function"
    );
}

// ===== CAP-187 — Web Animations borné : `animate`,
// `getAnimations`, `startViewTransition` (sonde
// `_census_calls` — machine à états honnête, pas de
// compositeur).

#[test]
fn cap187_web_animations_bounded() {
    let mut e =
        engine_with("<!doctype html><html><body><div id=x></div><p id=y></p></body></html>");
    e.eval("var x=document.getElementById('x'),y=document.getElementById('y'),d=document;")
        .expect("setup");
    // `animate` — Animation vivante : playState, finished
    // pendante, ready résolue ; `finish` → 'finished' +
    // promesse + onfinish.
    assert_eq!(
        evf(
            &mut e,
            "var an=x.animate({opacity:[0,1]},1000);var f='';an.onfinish=function(){f='F'};\
            [typeof an.play, an.playState, an.playbackRate, x.getAnimations().length,\
             an===x.getAnimations()[0]].join('|')"
        ),
        "function|running|1|1|true"
    );
    // `finish` → état + onfinish ; `pause`/`cancel` →
    // playState + sortie du registre (`getAnimations` vide).
    assert_eq!(
        evf(
            &mut e,
            "an.finish();\
            var a2=y.animate({},500);a2.cancel();\
            var a3=x.animate({},0);a3.pause();a3.play();\
            [an.playState, f, a2.playState, x.getAnimations().length,\
             d.getAnimations().length, a3.playState].join('|')"
        ),
        "finished|F|idle|2|2|running"
    );
    // `startViewTransition` — objet {updateCallbackDone,
    // ready, finished, skipTransition} ; callback exécuté,
    // promesses résolues.
    assert_eq!(
        evf(
            &mut e,
            "var c='';var vt=d.startViewTransition(function(){c='C'});\
            [c, typeof vt.skipTransition, typeof vt.finished.then,\
             vt.toString()].join('|')"
        ),
        "C|function|function|[object ViewTransition]"
    );
    e.run_microtasks().expect("microtasks");
    // `finished` de la première animation a résolu.
    assert_eq!(
        evf(&mut e, "var r='';an.finished.then(function(){r='ok'});r"),
        ""
    );
    e.run_microtasks().expect("microtasks");
    assert_eq!(evf(&mut e, "r"), "ok");
}

// ===== CAP-188 — `document.evaluate` : sous-ensemble XPath —

#[test]
fn cap188_xpath_subset() {
    let mut e = engine_with(
        "<!doctype html><html><body><div id=x><p class=a>hello</p><p class=a>world</p>\
         <span id=s>t</span></div><p class=b>out</p></body></html>",
    );
    e.eval("var d=document;").expect("setup");
    // `//tag` descendant : 3 `<p>` dans le document.
    assert_eq!(
        evf(
            &mut e,
            "var r=d.evaluate('//p',d,null,7,null);\
            [r.resultType, r.snapshotLength, r.snapshotItem(0).textContent,\
             r.snapshotItem(2).className].join('|')"
        ),
        "7|3|hello|b"
    );
    // `[@a='v']` filtre + `[n]` position + `.//` relatif.
    assert_eq!(
        evf(
            &mut e,
            "var x=d.getElementById('x');\
            var r2=d.evaluate('.//p[@class=\"a\"]',x,null,9,null);\
            var r3=d.evaluate('//p[2]',d,null,9,null);\
            [r2.singleNodeValue.textContent, r3.singleNodeValue.textContent].join('|')"
        ),
        "hello|world"
    );
    // `text()` + `id('…')` + `count(…)` + `booleanValue`.
    assert_eq!(
        evf(
            &mut e,
            "var r4=d.evaluate('//p[@class=\"a\"]/text()',d,null,7,null);\
            var r5=d.evaluate(\"id('s')\",d,null,9,null);\
            var r6=d.evaluate(\"count(//p)\",d,null,1,null);\
            var r7=d.evaluate('//nonexistent',d,null,3,null);\
            [r4.snapshotLength, r4.snapshotItem(0).nodeValue,\
             r5.singleNodeValue.id, r6.numberValue, r7.booleanValue].join('|')"
        ),
        "2|hello|s|3|false"
    );
    // Itérateur (type 4) — `iterateNext` ordre document.
    assert_eq!(
        evf(
            &mut e,
            "var it=d.evaluate('//p',d,null,4,null);var s='';var n;\
            while((n=it.iterateNext())!==null){s+=n.textContent[0]}s"
        ),
        "hwo"
    );
    // Syntaxe non portée → TypeError honnête.
    assert_eq!(
        evf(
            &mut e,
            "var t='';try{d.evaluate('//p[position()=1]',d,null,7,null)}catch(v){t=v.name}t"
        ),
        "TypeError"
    );
}

// ===== CAP-191 — Shadow DOM tranche 1 : surface API sans
// composition — `attachShadow`, `shadowRoot` open/closed,
// `host`, `getRootNode({composed})`, `assignedSlot`/
// `assignedElements` honnêtes (pas de scoping ni slots).

#[test]
fn cap191_shadow_surface() {
    let mut e =
        engine_with("<!doctype html><html><body><div id=h><p id=light>x</p></div></body></html>");
    e.eval("var h=document.getElementById('h'),d=document;")
        .expect("setup");
    // attachShadow open → objet fragment, nodeType 11,
    // `el.shadowRoot === r`, `r.host === h`, ownerDocument.
    assert_eq!(
        evf(
            &mut e,
            "var r=h.attachShadow({mode:'open'});\
            [r.nodeType, r.nodeName, h.shadowRoot===r, r.host===h,\
             r.getRootNode()===r, r instanceof DocumentFragment,\
             typeof r.querySelector].join('|')"
        ),
        "11|#document-fragment|true|true|true|true|function"
    );
    // Enfants de la racine : getRootNode() = racine ;
    // {composed:true} → document (traverse l'hôte).
    assert_eq!(
        evf(
            &mut e,
            "var s=d.createElement('b');r.appendChild(s);\
            [s.getRootNode()===r, s.getRootNode({composed:true})===document,\
             s.getRootNode()===d.getElementById('light').getRootNode()].join('|')"
        ),
        "true|true|false"
    );
    // mode closed → `el.shadowRoot` null, racine via
    // attachInternals (spec `ElementInternals.shadowRoot`).
    assert_eq!(
        evf(
            &mut e,
            "var h2=d.createElement('div');d.body.appendChild(h2);\
            var r2=h2.attachShadow({mode:'closed'});\
            [h2.shadowRoot===null, r2.host===h2, r2.getRootNode()===r2].join('|')"
        ),
        "true|true|true"
    );
    // Re-attach → NotSupportedError (spec).
    assert_eq!(
        evf(&mut e, "var t='';try{h.attachShadow()}catch(v){t=v.name}t"),
        "NotSupportedError"
    );
    // Répartition flat (CAP-195 tranche 3) : `light` (enfant
    // light-DOM sans `slot`) est distribué au slot par défaut
    // — `assignedSlot` résout, `assigned*` comptent réel.
    assert_eq!(
        evf(
            &mut e,
            "var sl=d.createElement('slot');r.appendChild(sl);\
            [d.getElementById('light').assignedSlot===sl,\
             sl.assignedElements().length, sl.assignedNodes().length].join('|')"
        ),
        "true|1|1"
    );
    // innerHTML sur la racine (fragment parsing).
    assert_eq!(
        evf(
            &mut e,
            "r.innerHTML='<i>in</i>';r.querySelector('i')===null?'null':r.childNodes.length"
        ),
        "1"
    );
}

// ===== R-96 — transport DSD : la synchro sérialise les racines
// shadow en `<template shadowrootmode>` (re-parse moteur →
// racine attachée), le fragment porte l'id aligné keymap ;
// `innerHTML`/`outerHTML` JS restent muets (spec).

#[test]
fn r96_serialize_keyed_emet_shadowrootmode() {
    let dom = Rc::new(RefCell::new(Dom::from_html(
        "<!doctype html><html><body><div id=h><em id=light>L</em></div></body></html>",
    )));
    let mut e = Engine::new();
    e.install_dom(&dom);
    evf(
        &mut e,
        "var h=document.getElementById('h'),r=h.attachShadow({mode:'open'});\
         var s=document.createElement('b');s.id='sh';r.appendChild(s);'ok'",
    );
    let (html, ids, tags) = dom.borrow().serialize_keyed();
    // Ordre émis : host, <template>(id=fragment), enfants
    // shadow, enfants light.
    let pos_tpl = html
        .find("<template shadowrootmode=\"open\">")
        .expect("dsd");
    let pos_b = html.find("<b id=\"sh\"").expect("enfant shadow");
    let pos_em = html.find("<em id=\"light\"").expect("light");
    let pos_close = html.find("</template>").expect("fermeture");
    assert!(pos_tpl < pos_b && pos_b < pos_close && pos_close < pos_em);
    // La série `tags` aligne le fragment sur « template ».
    let i = tags.iter().position(|t| t == "template").expect("tag tpl");
    let host_i = tags.iter().position(|t| t == "div").expect("host");
    let b_i = tags.iter().position(|t| t == "b").expect("b");
    let em_i = tags.iter().position(|t| t == "em").expect("em");
    assert!(host_i < i && i < b_i && b_i < em_i);
    assert_eq!(ids.len(), tags.len());
    // `innerHTML`/`outerHTML` JS : pas de racine shadow (spec).
    assert_eq!(
        evf(
            &mut e,
            "[h.innerHTML.includes('shadowroot'),h.outerHTML.includes('shadowroot')].join('|')"
        ),
        "false|false"
    );
}

#[test]
fn r96_fingerprint_compte_le_template_shadow() {
    let dom = Rc::new(RefCell::new(Dom::from_html(
        "<!doctype html><html><body><div id=h></div></body></html>",
    )));
    let fp_before = dom.borrow().structural_fingerprint();
    let mut e = Engine::new();
    e.install_dom(&dom);
    evf(
        &mut e,
        "var r=document.getElementById('h').attachShadow({mode:'open'});\
         r.appendChild(document.createElement('i'));'ok'",
    );
    let fp_after = dom.borrow().structural_fingerprint();
    // L'attach + l'enfant shadow changent l'empreinte (le
    // template synthétique et `i` sont comptés).
    assert_ne!(fp_before, fp_after);
}

// ===== CAP-198 — Shadow DOM tranche 4 : event retargeting —
// `target`/`relatedTarget` ré-étiquetés à la frontière shadow
// (racine du `currentTarget`), `composed` traverse (racine →
// hôte → ancêtres, imbriqué compris), non-composé tronqué à la
// racine, `composedPath()` réel pendant le dispatch.

#[test]
fn cap198_shadow_event_retarget() {
    let mut e = engine_with("<!doctype html><html><body><div id=h></div></body></html>");
    e.eval(
        "var h=document.getElementById('h'),d=document;\
        var r=h.attachShadow({mode:'open'});\
        var b=d.createElement('button');r.appendChild(b);",
    )
    .expect("setup");

    // `composed:false` — la propagation s'arrête à la racine
    // shadow : le listener document ne reçoit rien ; le
    // listener interne (racine) voit la cible réelle `b`.
    assert_eq!(
        evf(
            &mut e,
            "var seen='none',tg=false;\
            r.addEventListener('x',function(ev){seen='in';tg=ev.target===b;});\
            d.addEventListener('x',function(ev){seen='doc';});\
            b.dispatchEvent({type:'x',bubbles:true,composed:false});seen+'/'+tg"
        ),
        "in/true"
    );

    // `composed:true` — la propagation traverse : le listener
    // interne voit `b`, les listeners hôte/document voient
    // `target` retargeté = hôte (spec « retarget »).
    assert_eq!(
        evf(
            &mut e,
            "var it=false,ht=false,dt=false,htgt=false;\
            r.addEventListener('y',function(ev){it=ev.target===b;});\
            h.addEventListener('y',function(ev){ht=ev.target===h;});\
            d.addEventListener('y',function(ev){dt=true;htgt=ev.target===h;});\
            b.dispatchEvent({type:'y',bubbles:true,composed:true});\
            [it,ht,dt,htgt].join('|')"
        ),
        "true|true|true|true"
    );

    // `composedPath()` pendant le dispatch — interne et
    // externe voient la chaîne complète (racines shadow
    // incluses) pour `composed`, tronquée à la racine sinon ;
    // `[]` après dispatch (spec).
    assert_eq!(
        evf(
            &mut e,
            "var plen=0,p0=false,p1=false;\
            h.addEventListener('z',function(ev){var p=ev.composedPath();plen=p.length;p0=p[0]===b;p1=p[1]===r;});\
            var ev3=new Event('z',{bubbles:true,composed:true});\
            b.dispatchEvent(ev3);\
            var post=ev3.composedPath().length;\
            var nclen=0;\
            b.addEventListener('w',function(ev2){nclen=ev2.composedPath().length});\
            b.dispatchEvent(new Event('w',{bubbles:true,composed:false}));\
            [plen,p0,p1,post,nclen].join('|')"
        ),
        "6|true|true|0|2"
    );

    // `relatedTarget` retargeté comme `target` (spec) : le
    // listener interne voit le nœud shadow réel, l'externe
    // l'hôte ; restauré après dispatch.
    assert_eq!(
        evf(
            &mut e,
            "var b2=d.createElement('i');r.appendChild(b2);\
            var rin=false,rout=false;\
            r.addEventListener('f',function(ev){rin=ev.relatedTarget===b2;});\
            h.addEventListener('f',function(ev){rout=ev.relatedTarget===h;});\
            var fev=new FocusEvent('f',{bubbles:true,composed:true,relatedTarget:b2});\
            b.dispatchEvent(fev);\
            [rin,rout,fev.relatedTarget===b2].join('|')"
        ),
        "true|true|true"
    );

    // Non-composé sur un arbre shadow **imbriqué** : la
    // frontière la plus proche seule compte — un listener sur
    // la racine externe ne voit rien non plus.
    assert_eq!(
        evf(
            &mut e,
            "var h3=d.createElement('span');r.appendChild(h3);\
            var r3=h3.attachShadow({mode:'open'});\
            var deep=d.createElement('b');r3.appendChild(deep);\
            var got='';\
            r3.addEventListener('g',function(){got+='r3'});\
            h3.addEventListener('g',function(){got+='h3'});\
            r.addEventListener('g',function(){got+='r'});\
            deep.dispatchEvent({type:'g',bubbles:true,composed:false});got||'none'"
        ),
        "r3"
    );

    // Composé imbriqué : traversée double frontière —
    // `target` retargeté par PALIERS (deep → h3 pour la racine
    // externe, h pour le document).
    assert_eq!(
        evf(
            &mut e,
            "var t1='',t2='',t3='';\
            r3.addEventListener('k',function(ev){t1=ev.target===deep?'deep':'?'});\
            r.addEventListener('k',function(ev){t2=ev.target===h3?'h3':'?'});\
            d.addEventListener('k',function(ev){t3=ev.target===h?'h':'?'});\
            deep.dispatchEvent({type:'k',bubbles:true,composed:true});[t1,t2,t3].join('|')"
        ),
        "deep|h3|h"
    );

    // Régression light-DOM : `target` nu hors shadow, et
    // `composedPath` = [cible, body, html, document].
    assert_eq!(
        evf(
            &mut e,
            "var lt=false,lp=0;\
            h.addEventListener('m',function(ev){lt=ev.target===h;});\
            d.addEventListener('m',function(ev){lp=ev.composedPath().length});\
            h.dispatchEvent({type:'m',bubbles:true});[lt,lp].join('|')"
        ),
        "true|4"
    );
}

// ===== CAP-194 — `AbortSignal` réel : statics
// `abort`/`timeout`/`any`, proto (`aborted`, `reason`,
// `throwIfAborted`, `onabort`, `addEventListener`), ctor
// Illegal spec (mur mesuré : `AbortSignal is not defined`,
// bootstrap x-web).

#[test]
fn cap194_abort_signal_surface() {
    let mut e = engine_with("<p>x</p>");
    // Statics + proto existent ; ctor Illegal (spec).
    assert_eq!(
        evf(
            &mut e,
            "typeof AbortSignal+'/'+typeof AbortSignal.timeout+'/'\
            +typeof AbortSignal.abort+'/'+typeof AbortSignal.any+'/'\
            +(function(){try{new AbortSignal();return 'ctor'}catch(x){return 'illegal'}})()"
        ),
        "function/function/function/function/illegal"
    );
    // `abort(r)` → aborted+reason ; `throwIfAborted` jette reason.
    assert_eq!(
        evf(
            &mut e,
            "var s=AbortSignal.abort('R');\
            s.aborted+'/'+s.reason+'/'\
            +(function(){try{s.throwIfAborted();return 'x'}catch(v){return v}})()"
        ),
        "true/R/R"
    );
    // `AbortController` → signal réel ; `abort` tire
    // onabort + listeners `abort` dans l'ordre.
    assert_eq!(
        evf(
            &mut e,
            "var c=new AbortController(),log='';\
            c.signal.addEventListener('abort',function(){log+='L'});\
            c.signal.onabort=function(){log+='O'};c.abort('Z');\
            c.signal.aborted+'/'+c.signal.reason+'/'+log"
        ),
        "true/Z/OL"
    );
    // `any` : une source déjà abortée propage sa reason ;
    // `timeout` arme un abort différé.
    assert_eq!(
        evf(
            &mut e,
            "var a=AbortSignal.any([AbortSignal.abort('A')]);\
            a.aborted+'/'+a.reason+'/'\
            +typeof AbortSignal.timeout(0).then"
        ),
        "true/A/undefined"
    );
    // JS-C153 — statique `AbortSignal.throwIfAborted(sig)`
    // (youtube kevlar vol22 : lue sur le ctor →
    // `of Undefined`). Jette `reason` si aborted, passe sinon.
    assert_eq!(
        evf(
            &mut e,
            "typeof AbortSignal.throwIfAborted+'/'+\
            (function(){try{AbortSignal.throwIfAborted(AbortSignal.abort('X'));return 'n'}\
             catch(v){return v.name||v}})()+'/'+\
            (function(){var c=new AbortController();\
             AbortSignal.throwIfAborted(c.signal);return 'ok'})()"
        ),
        "function/X/ok"
    );
}

// ===== CAP-195 — `document.adoptedStyleSheets` settable `[]`
// (mur mesuré : le polyfill Popover github fait `[C, ...e.
// adoptedStyleSheets]` → `value is not iterable` quand le
// nom manquait).

#[test]
fn cap195_adopted_style_sheets_surface() {
    let mut e = engine_with("<p>x</p>");
    // `[]` réel + identité stable `===` + spread sain.
    assert_eq!(
        evf(
            &mut e,
            "Array.isArray(document.adoptedStyleSheets)+'/'+\
            (document.adoptedStyleSheets===document.adoptedStyleSheets)+'/'+\
            [0,...document.adoptedStyleSheets].length"
        ),
        "true/true/1"
    );
    // Assignation remplace — motif `adoptedStyleSheets=[…]`.
    assert_eq!(
        evf(
            &mut e,
            "document.adoptedStyleSheets=[{a:1},{b:2}];\
            document.adoptedStyleSheets.length+'/'+document.adoptedStyleSheets[1].b"
        ),
        "2/2"
    );
}

// ===== CAP-196 — `getComputedStyle(el).position` ← `fixed`
// de la géométrie publiée (mur mesuré : garde `"fixed"===
// position` du script `--marketing-header-fixed-top` github).

#[test]
fn cap196_computed_position_from_geometry() {
    let (mut e, _jar, _navs) = engine_full(PAGE);
    let geom = explorer_jscore::GeometrySlot::default();
    {
        let mut g = geom.lock().unwrap();
        // #list → html>body>ul[0] — boîte `fixed` publiée.
        g.rects.insert(
            vec![
                ("html".to_string(), 0),
                ("body".to_string(), 0),
                ("ul".to_string(), 0),
            ],
            explorer_jscore::ElementGeom {
                rect: explorer_jscore::GeoRect {
                    x: 0.0,
                    y: 8.0,
                    w: 200.0,
                    h: 40.0,
                },
                fixed: true,
                positioned: true,
            },
        );
    }
    e.set_geometry_ctx(0, geom);
    // `position` feuille-only : la cascade l'a vue au layout
    // (fixed publié) → `"fixed"` ; le non-publié et `el.style`
    // (inline pur, spec) restent honnêtes `""` ; idem via
    // `getPropertyValue`.
    assert_eq!(
        evf(
            &mut e,
            "var s=getComputedStyle(document.getElementById('list'));\
            s.position+'/'+\
            getComputedStyle(document.getElementById('main')).position+'/'+\
            document.getElementById('list').style.position+'/'+\
            s.getPropertyValue('position')"
        ),
        "fixed///fixed"
    );
}

/// `offsetParent`/`offsetTop`/`offsetLeft` (JS-C97, palier 2
/// i#19) : la session publie `positioned` par élément ; le
/// miroir marche sa chaîne — plus proche ancêtre positionné
/// ou `body`, `null` pour `position:fixed` ; la distance est
/// `rect − parent.rect` en coords document (insensible au
/// scroll) ; `fixed` → `rect.y` viewport.
#[test]
fn c97_offset_parent_top_left() {
    let (mut e, _jar, _navs) = engine_full(PAGE);
    e.set_viewport(800, 600);
    let geom = explorer_jscore::GeometrySlot::default();
    {
        let mut g = geom.lock().unwrap();
        g.scroll_y = 100.0;
        // body → html>body[0] — positionné (relative).
        g.rects.insert(
            vec![("html".to_string(), 0), ("body".to_string(), 0)],
            explorer_jscore::ElementGeom {
                rect: explorer_jscore::GeoRect {
                    x: 0.0,
                    y: 8.0,
                    w: 800.0,
                    h: 2000.0,
                },
                fixed: false,
                positioned: true,
            },
        );
        // #main div (statique) → html>body>div[0]
        g.rects.insert(
            vec![
                ("html".to_string(), 0),
                ("body".to_string(), 0),
                ("div".to_string(), 0),
            ],
            explorer_jscore::ElementGeom {
                rect: explorer_jscore::GeoRect {
                    x: 40.0,
                    y: 120.0,
                    w: 100.0,
                    h: 50.0,
                },
                fixed: false,
                positioned: false,
            },
        );
        // #list ul (fixed) → html>body>ul[0]
        g.rects.insert(
            vec![
                ("html".to_string(), 0),
                ("body".to_string(), 0),
                ("ul".to_string(), 0),
            ],
            explorer_jscore::ElementGeom {
                rect: explorer_jscore::GeoRect {
                    x: 0.0,
                    y: 8.0,
                    w: 200.0,
                    h: 40.0,
                },
                fixed: true,
                positioned: true,
            },
        );
    }
    e.set_geometry_ctx(0, geom);
    assert_eq!(
        evf(
            &mut e,
            "var out=[],m=document.getElementById('main'),l=document.getElementById('list');\
             out.push(m.offsetParent===document.body,l.offsetParent===null);\
             out.push(m.offsetTop,m.offsetLeft,l.offsetTop,l.offsetLeft);\
             out.join(',')"
        ),
        "true,true,112,40,8,0"
    );
}

/// Ecart form github (vol 11/10) : focus/blur dispatches
/// par l hote (trusted, chemin session) doivent synchroniser
/// dom.focused — document.activeElement suit le focus reel,
/// pas le stale precedent.
#[test]
fn host_focus_dispatch_updates_active_element() {
    let dom = Rc::new(RefCell::new(Dom::from_html(
        "<body><input id=a><input id=b><button id=x>ok</button></body>",
    )));
    let mut e = Engine::new();
    e.install_dom(&dom);
    let a = dom.borrow().by_id("a").expect("a present");
    e.dom_event_on("focus", a).expect("focus dispatch");
    assert_eq!(evf(&mut e, "document.activeElement.id"), "a");
    e.dom_event_on("blur", a).expect("blur dispatch");
    assert_eq!(evf(&mut e, "document.activeElement.tagName"), "BODY");
}

/// JS-C106 — `RangeError: expression too deeply nested` sur du
/// minifié réel (caniuse bundle.js) : les bundles nichent des
/// chaînes `&&`/`?:`/parenthèses bien au-delà de 512 — la borne
/// passe à 4096 (la pile reste protégée par `maybe_grow`).
#[test]
fn deeply_nested_expression_parses() {
    let deep = "x".to_string();
    let open = "(".repeat(600);
    let close = ")".repeat(open.len());
    let src = format!("var x = {open}{deep}{close};");
    let mut e = explorer_jscore::Engine::new();
    let r = e.eval(&src);
    assert!(r.is_ok(), "600 parenthèses imbriquées rejetées : {r:?}");
}

/// JS-C127 — `new URL(".", location)` → « Invalid base URL »
/// (svelte/fastapi/mkdocs — motif de résolution d'assets ultra-
/// commun) : `location` n'avait pas de `toString` surfacé →
/// `to_primitive` tombait sur `Object.prototype.toString` →
/// "[object Location]" rejeté comme base. Spec : `Location`
/// stringifie à `href` ; `assign`/`replace`/`reload` lisibles.
#[test]
fn new_url_against_location_resolves() {
    let (mut e, _jar, _n) = engine_full(PAGE);
    // `page_url` = http://ex.org:8080/a/b?q=1#frag.
    assert_eq!(
        evf(&mut e, "new URL('.', location).href"),
        "http://ex.org:8080/a/"
    );
    assert_eq!(
        evf(&mut e, "String(location)"),
        "http://ex.org:8080/a/b?q=1#frag"
    );
    assert_eq!(
        evf(&mut e, "location + ''"),
        "http://ex.org:8080/a/b?q=1#frag"
    );
    assert_eq!(
        evf(&mut e, "location.toString()"),
        "http://ex.org:8080/a/b?q=1#frag"
    );
    assert_eq!(evf(&mut e, "typeof location.assign"), "function");
    assert_eq!(evf(&mut e, "typeof location.replace"), "function");
}

#[test]
fn url_ctor_with_location_base() {
    // `new URL(".", location)` / `new URL(href, document.baseURI)`
    // — Location/baseURI doivent être stringifiés en base
    // (svelte.dev, fastapi, mkdocs : « Invalid base URL »).
    let (mut e, _jar, _navs) = engine_full(PAGE);
    for src in [
        "new URL('.', location).href",
        "new URL('x', location).href",
        "new URL('.', location.href).href",
        "new URL('.', document.baseURI).href",
    ] {
        match e.eval(src) {
            Ok(v) => println!("OK   {src} → {}", e.describe(v)),
            Err(x) => println!("FAIL {src} → {x}"),
        }
    }
}

#[test]
fn location_global_exists_and_chains() {
    // `Location` global (news.google) : interface exposée,
    // `location instanceof Location`, `new` illégal.
    let (mut e, _jar, _navs) = engine_full(PAGE);
    for (src, want) in [
        ("typeof Location", "function"),
        ("location instanceof Location", "true"),
        ("typeof Location.prototype", "object"),
        ("try{new Location();'no'}catch(x){x.name}", "TypeError"),
    ] {
        assert_eq!(evf(&mut e, src), want, "{src}");
    }
}

#[test]
fn document_xpath_create_expression() {
    // DOM L3 XPath (mur htmx/hyperscript — `createExpression is
    // not a function`) : expression compilée + evaluate + resolver.
    let (mut e, _jar, _navs) = engine_full(PAGE);
    e.eval("var _xp=document.createExpression('.//a');var _nr=document.createNSResolver(document)")
        .unwrap_or_else(|x| panic!("xpath setup: {x}"));
    for (src, want) in [
        ("typeof document.createExpression", "function"),
        ("typeof document.createNSResolver", "function"),
        ("typeof _xp", "object"),
        ("typeof _xp.evaluate", "function"),
        ("typeof _xp.evaluate(document, 9)", "object"),
    ] {
        assert_eq!(evf(&mut e, src), want, "{src}");
    }
}

// ===== JS-C159 — surface observable shadow/Ws/BC/EventSource

#[test]
fn c159_shadow_ws_bc_es_surface() {
    let mut e = engine_with("<div id=h></div>");
    // `sr.getElementById` scopé au sous-arbre + `instanceof`.
    assert_eq!(
        evf(
            &mut e,
            "var h=document.getElementById('h'),r=h.attachShadow({mode:'open'});\
            r.innerHTML='<p id=x>hi</p>';\
            [r instanceof ShadowRoot, r.getElementById('x').textContent,\
             r.getElementById('absent')===null, document.getElementById('x')===null].join('|')"
        ),
        "true|hi|true|true"
    );
    // `getElementById` non exposé sur élément ordinaire (spec —
    // invoke-only → `undefined` à la lecture).
    assert_eq!(evf(&mut e, "typeof h.getElementById"), "undefined");
    // Méthodes WebSocket/BroadcastChannel lisibles (invoke-only
    // les rendait `undefined` — feature-detects cassés).
    assert_eq!(
        evf(
            &mut e,
            "[typeof new WebSocket('ws://x').send,\
             typeof new WebSocket('ws://x').close,\
             typeof new WebSocket('ws://x').addEventListener,\
             typeof new BroadcastChannel('c').postMessage].join('|')"
        ),
        "function|function|function|function"
    );
    // `EventSource` global posé à install_dom (refus différé
    // sans transport, jamais un global absent).
    assert_eq!(evf(&mut e, "typeof EventSource"), "function");
}

#[test]
fn instanceof_dom_hierarchy() {
    // J5 (impots/ameli) : `x instanceof Element`/`Node` —
    // protos chaînés par kind + héritage d'interfaces.
    let (mut e, _jar, _navs) = engine_full("<html><body><div id=d>hi</div><!--c--></body></html>");
    e.eval("var el=document.getElementById('d');var tx=el.firstChild")
        .unwrap_or_else(|x| panic!("setup: {x}"));
    for (src, want) in [
        ("el instanceof Element", "true"),
        ("el instanceof Node", "true"),
        ("el instanceof EventTarget", "true"),
        ("el instanceof HTMLElement", "true"),
        ("document instanceof Node", "true"),
        ("document instanceof Element", "false"),
        ("tx instanceof Text", "true"),
        ("tx instanceof Node", "true"),
        ("tx instanceof Element", "false"),
        ("typeof el.matches", "function"),
    ] {
        assert_eq!(evf(&mut e, src), want, "{src}");
    }
}

#[test]
fn component_name_and_function_name_probe() {
    // J5 suite (dw) : `Illegal component name ""` —
    // customElements.define(name vide ou cls sans nom) et
    // Function.name sur classes minifiées.
    let (mut e, _jar, _navs) = engine_full(PAGE);
    for src in [
        "class extends Object {}",
        "var C=class{}; C.name",
        "var C=class extends Object{}; C.name",
        "function f(){} f.name",
        "class {}; (class{}).name",
        "customElements.define('', class extends HTMLElement{});'d'",
        "customElements.define('x-1', class extends HTMLElement{});'d2'",
    ] {
        match e.eval(src) {
            Ok(v) => println!("OK   {src} → {}", e.describe(v)),
            Err(x) => println!("FAIL {src} → {x}"),
        }
    }
}

#[test]
fn document_contains_nodes() {
    // ratp (vol 45) : `document.contains(el)` — Node.contains
    // au niveau document.
    let (mut e, _jar, _navs) = engine_full(PAGE);
    assert_eq!(
        evf(&mut e, "document.contains(document.getElementById('main'))"),
        "true"
    );
    assert_eq!(evf(&mut e, "document.contains(document.body)"), "true");
    assert_eq!(evf(&mut e, "document.contains(null)"), "false");
}

#[test]
fn geolocation_names_defined() {
    // marmiton (V4) : `GeolocationPosition`/`GeolocationCoordinates`
    // nommés (détection, jamais construits).
    let (mut e, _jar, _navs) = engine_full(PAGE);
    assert_eq!(evf(&mut e, "typeof GeolocationPosition"), "function");
    assert_eq!(evf(&mut e, "typeof GeolocationCoordinates"), "function");
}

#[test]
fn user_agent_data_desktop() {
    // B-24 qwant : `userAgentData.mobile` répond `false` —
    // l'absence laissait les détects en fallback mobile.
    let (mut e, _jar, _navs) = engine_full(PAGE);
    assert_eq!(evf(&mut e, "typeof navigator.userAgentData"), "object");
    assert_eq!(evf(&mut e, "navigator.userAgentData.mobile"), "false");
    assert_eq!(evf(&mut e, "navigator.userAgentData.platform"), "Windows");
}

#[test]
fn frame_document_probe() {
    // dw : `cannot read 'document' of Null` — un hôte rend
    // null où le site lit `.document` (iframe legacy
    // `frames[i].document` / `contentDocument` absent).
    let (mut e, _jar, _navs) = engine_full(
        "<html><body><iframe id=f src='http://h/f'></iframe></body></html>",
    );
    for src in [
        "document.getElementById('f').contentDocument",
        "document.getElementById('f').document",
        "document.getElementById('f').contentWindow",
        "typeof frames",
    ] {
        match e.eval(src) {
            Ok(v) => println!("OK   {src} → {}", e.describe(v)),
            Err(x) => println!("FAIL {src} → {x}"),
        }
    }
}
